//! End to end: a document with a material drive, through the application's own
//! preparation pipeline, onto the device.
//!
//! Every other gate exercises a piece. The unit tests cover preparation, the
//! solver gates cover the GPU core against an f64 oracle, and the forced gate
//! covers a driven medium with its forcing. Nothing has run the assembled
//! thing: an authored document, meshed and assembled by the application's
//! resumable jobs, compiled into a temporal plan and stepped on the device.
//!
//! That assembly is where a surprise would hide, because it is the only place
//! the stripped model, the shared base, the trajectory timestep and the
//! temporal plan all meet.

use std::time::{Duration, Instant};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuPlan, CanonicalGpuRequest,
    CanonicalWaveGpuPlugin,
};
use funfern_app::topology_editor::TopologyEditor;
use funfern_app::topology_runtime::TopologyRuntime;
use funfern_app::wave_gpu::WaveGpuPlugin;
use funfern_core::{CanonicalTemporalWaveState, MeshingOptions, Point2, ScalarField, TimeDrive};

fn steps() -> u64 {
    std::env::var("DRIVEN_STEPS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(48)
}

#[derive(Resource)]
struct Pending {
    plan: Option<CanonicalGpuPlan>,
}

#[derive(Resource)]
struct Expected {
    primary: Vec<f64>,
    complementary: Vec<Point2>,
    started: Instant,
    deadline: Instant,
    finished: bool,
    failed: bool,
}

fn main() {
    // An authored document, exactly as the editor would hold one.
    let mut document = TopologyEditor::default().document;
    // A zero depth keeps every piece of the temporal path switched on - the
    // operator, the tables, the plan - while the mass stays where the authored
    // coefficients put it. It separates "this path diverges" from "a moving
    // mass is mishandled", which no amount of staring at the shader would.
    let depth = std::env::var("DRIVEN_DEPTH")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0.24);
    let pump = TimeDrive::ParametricPump {
        depth: ScalarField::constant(depth),
        frequency_hz: ScalarField::constant(0.9),
        phase_radians: ScalarField::constant(0.2),
    };
    document.model.draft.materials[0].mass_law.drive = pump.clone();
    document.model.accepted.materials[0].mass_law.drive = pump;
    // The document's default walls are second-order outgoing, which a driven
    // medium cannot yet use: that boundary's trace factorization is built from
    // the nodal mass and the device has no way to refresh it. The plan
    // compiler refuses the combination, so this fixture runs reflecting walls
    // and `DRIVEN_WALLS=outgoing` asks for the refusal instead.
    if !std::env::var("DRIVEN_WALLS").is_ok_and(|value| value == "outgoing") {
        let walls = funfern_core::OuterBoundaryConditions::uniform(
            funfern_core::OuterBoundaryCondition::Reflecting,
        );
        document.model.draft.outer_boundaries = walls;
        document.model.accepted.outer_boundaries = walls;
    }
    let editor = TopologyEditor::from_document(document).expect("authored document");

    // The application's own preparation: meshing, both assemblies from the
    // stripped model, and the temporal operator over the base they produced.
    let mut runtime = TopologyRuntime::default();
    let token = runtime
        .request(
            editor.revision,
            &editor.document,
            editor.compiled_accepted.clone(),
            MeshingOptions {
                target_edge_length: 0.12,
                ..MeshingOptions::default()
            },
            true,
        )
        .expect("preparation request");
    let prepared = loop {
        if let Some(result) = runtime.advance(4_096) {
            result.expect("preparation");
            break runtime.commit_ready(token).expect("prepared topology");
        }
    };
    let temporal = prepared
        .canonical_temporal_operator
        .clone()
        .expect("an authored drive must produce a temporal operator");
    let time_step = prepared.recommended_time_step();
    assert!(
        time_step <= prepared.canonical_operator.recommended_time_step(),
        "a driven generation cannot take a looser step than the fixed one"
    );
    println!(
        "driven document: outer {:?}, bulk {}, gaps {}, damped {}, prescribed {}",
        editor.document.model.accepted.outer_boundaries,
        temporal.conservative_bulk_supported(),
        !base_of(&temporal).thin_gap_samples().is_empty(),
        base_of(&temporal)
            .first_order_boundary_damping()
            .iter()
            .any(|value| *value != 0.0),
        prepared
            .operator
            .dirichlet_signals()
            .iter()
            .any(Option::is_some),
    );
    println!(
        "driven document: {} DOFs, {} elements, dt {time_step:.4e} against a fixed {:.4e}",
        prepared.canonical_operator.degrees_of_freedom(),
        prepared.mesh.triangles.len(),
        prepared.canonical_operator.recommended_time_step()
    );

    let clock = CanonicalGpuClock::initial(time_step).expect("clock");

    // A zero field stays zero, which proves nothing. Give it something to
    // carry and step the oracle the same way.
    let base = temporal.base();
    let primary = base
        .primary_mass()
        .iter()
        .zip(base.node_points())
        .map(|(mass, point)| mass * (0.04 * (1.4 * point.x - 0.8 * point.y).sin()))
        .collect::<Vec<_>>();
    let potential = base
        .node_points()
        .iter()
        .map(|point| 0.03 * (0.9 * point.x + 1.2 * point.y).cos())
        .collect::<Vec<_>>();
    let complementary = base.compatible_flux(&potential).expect("compatible flux");
    let seeded = CanonicalTemporalWaveState::new(&temporal, time_step, primary, complementary)
        .expect("seeded state");
    let plan =
        CanonicalGpuPlan::compile_temporal(&temporal, &seeded, &prepared.canonical_forcing, clock)
            .expect("temporal plan from the application's own generation");
    if let Some(outgoing) = base.outgoing_boundary() {
        println!(
            "driven document: {} trace nodes solved in {} sweeps",
            outgoing.trace_nodes().len(),
            plan.trace_sweeps,
        );
    }

    let mut oracle = seeded;
    for _ in 0..steps() {
        oracle
            .step_with_forcing(&temporal, &prepared.canonical_forcing)
            .expect("f64 oracle step");
    }

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern driven document".into(),
            visible: false,
            resolution: (64, 64).into(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(WaveGpuPlugin)
    .add_plugins(CanonicalWaveGpuPlugin)
    .insert_resource(Pending { plan: Some(plan) })
    .insert_resource(Expected {
        primary: oracle.primary_flux().to_vec(),
        complementary: oracle.complementary_flux().to_vec(),
        started: Instant::now(),
        deadline: Instant::now() + Duration::from_secs(120),
        finished: false,
        failed: false,
    })
    .add_systems(Startup, install)
    .add_systems(Update, drive)
    .run();
}

fn install(
    mut commands: Commands,
    mut pending: ResMut<Pending>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut canonical: ResMut<CanonicalGpuRequest>,
) {
    canonical.install(
        &mut assets,
        &mut commands,
        pending.plan.take().expect("one plan"),
    );
    canonical.request_steps(steps());
    commands.spawn(Camera2d);
}

fn drive(
    mut commands: Commands,
    mut request: ResMut<CanonicalGpuRequest>,
    display: Res<CanonicalGpuDisplay>,
    mut expected: ResMut<Expected>,
    mut exit: MessageWriter<AppExit>,
) {
    if expected.finished {
        exit.write(if expected.failed {
            AppExit::error()
        } else {
            AppExit::Success
        });
        return;
    }
    if Instant::now() >= expected.deadline || request.stats().failure() != 0 {
        eprintln!("driven document run timed out or failed");
        expected.failed = true;
        expected.finished = true;
        return;
    }
    if request.stats().completed_steps() < steps() {
        return;
    }
    if !request.request_full_state_readback(&mut commands) && display.full_readbacks == 0 {
        return;
    }
    // Both lanes, or the complementary comparison below runs before the
    // readback has populated that lane and passes by measuring nothing.
    if display.primary_flux.len() != expected.primary.len()
        || display.complementary_flux.len() != expected.complementary.len()
    {
        return;
    }
    let primary = relative_l2(
        display.primary_flux.iter().map(|value| f64::from(*value)),
        expected.primary.iter().copied(),
    );
    let complementary = relative_l2(
        display
            .complementary_flux
            .iter()
            .flat_map(|value| value.iter().map(|lane| f64::from(*lane))),
        expected
            .complementary
            .iter()
            .flat_map(|value| [value.x, value.y]),
    );
    println!(
        "driven document after {:.2} ms: Q {primary:.3e}, b {complementary:.3e}",
        expected.started.elapsed().as_secs_f64() * 1_000.0
    );
    expected.finished = true;
    if primary > 2.0e-4 || complementary > 2.0e-4 {
        expected.failed = true;
    }
}

fn base_of(
    temporal: &funfern_core::CanonicalTemporalWaveOperator,
) -> &funfern_core::CanonicalWaveOperator {
    temporal.base()
}

fn relative_l2(
    actual: impl Iterator<Item = f64>,
    expected: impl Iterator<Item = f64> + Clone,
) -> f64 {
    let scale = expected
        .clone()
        .map(|value| value * value)
        .sum::<f64>()
        .max(1.0e-30);
    let difference = actual
        .zip(expected)
        .map(|(actual, expected)| (actual - expected).powi(2))
        .sum::<f64>();
    (difference / scale).sqrt()
}
