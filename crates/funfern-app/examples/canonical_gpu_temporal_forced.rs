//! Real-device validation of a driven medium with its forcing composed in.
//!
//! The CPU reference now composes prescribed data, volume sources, loss, both
//! absorbing wall orders and thin gaps with a time-driven medium. The GPU
//! pipeline has carried all of those stages since the fixed path, and the
//! shader already knows how to read an instantaneous coefficient - but until
//! the plan compiler's admission widened, the two could not be asked for
//! together, so no stage had ever been exercised with a moving nodal mass.
//!
//! That is what this gate is for. A stage that quietly divides by the authored
//! mass instead of the instantaneous one is correct on every fixture that
//! exists today and wrong the moment a source or a wall meets a pump. The
//! f64 reference is the oracle; disagreement localizes to whichever stage the
//! fixture turns on.

use std::time::{Duration, Instant};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuPlan, CanonicalGpuRequest,
    CanonicalWaveGpuPlugin,
};
use funfern_app::wave_gpu::WaveGpuPlugin;
use funfern_core::{
    CanonicalForcing, CanonicalSource, CanonicalTemporalWaveOperator, CanonicalTemporalWaveState,
    CoefficientLaw, MeshingOptions, OuterBoundaryCondition, Point2, QuadraticWaveOperator,
    ScalarField, Scene, TimeDrive, TimeSignal, mesh_scene,
};

const TOTAL_STEPS: u64 = 48;

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
    // A pumped medium, a volume source driving it, and an absorbing wall for
    // the radiation to leave through: three stages at once, each of which
    // reads the nodal mass somewhere.
    let mut scene = Scene::initial();
    scene.materials[0].mass_law.drive = TimeDrive::ParametricPump {
        depth: ScalarField::constant(0.24),
        frequency_hz: ScalarField::constant(0.85),
        phase_radians: ScalarField::constant(0.2),
    };

    let mut fixed_scene = scene.clone();
    for material in &mut fixed_scene.materials {
        material.mass_law = CoefficientLaw::linear();
        material.stiffness_law = CoefficientLaw::linear();
    }
    // `FORCED_BARE=1` strips the fixture back to a pumped medium inside
    // reflecting walls with no source, which is the configuration the
    // end-to-end driven-document run uses. It exists to tell a configuration
    // difference apart from an assembly-path one.
    let bare = std::env::var("FORCED_BARE").is_ok_and(|value| value == "1");
    let mesh = mesh_scene(
        &fixed_scene,
        1,
        MeshingOptions {
            target_edge_length: 0.2,
            ..MeshingOptions::default()
        },
    )
    .expect("forced temporal mesh");
    let scalar = QuadraticWaveOperator::assemble_scene(
        &mesh,
        &fixed_scene,
        if bare {
            OuterBoundaryCondition::Reflecting
        } else {
            OuterBoundaryCondition::FirstOrderOutgoing
        },
    )
    .expect("forced temporal scalar operator");
    let operator = CanonicalTemporalWaveOperator::compile_scene(&mesh, &scalar, &scene, 1)
        .expect("forced temporal operator");
    let base = operator.base();
    assert!(
        operator.forced_composition_supported() && !operator.conservative_bulk_supported(),
        "the fixture must exercise the widened admission"
    );
    assert!(
        bare || base
            .first_order_boundary_damping()
            .iter()
            .any(|value| *value != 0.0),
        "the fixture needs an absorbing wall"
    );

    let mut forcing = CanonicalForcing::none(base);
    if !bare {
        forcing
            .push_source(
                CanonicalSource::direct(
                    base,
                    base.primary_mass().to_vec(),
                    TimeSignal::harmonic(0.0, 0.6, 1.4, 0.35),
                )
                .expect("volume source"),
            )
            .expect("push volume source");
    }

    let time_step = 0.4 * operator.maximum_time_step();
    let primary = base
        .primary_mass()
        .iter()
        .zip(base.node_points())
        .map(|(mass, point)| mass * (0.04 + 0.03 * (1.3 * point.x - 0.7 * point.y).sin()))
        .collect::<Vec<_>>();
    let potential = base
        .node_points()
        .iter()
        .map(|point| 0.03 * (0.9 * point.x + 1.1 * point.y).cos())
        .collect::<Vec<_>>();
    let complementary = base
        .compatible_flux(&potential)
        .expect("compatible forced flux");
    let state = CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary)
        .expect("forced temporal state");

    let mut oracle = state.clone();
    let mut escaped = 0.0;
    let mut injected = 0.0;
    for _ in 0..TOTAL_STEPS {
        let accounting = oracle
            .step_with_forcing(&operator, &forcing)
            .expect("f64 forced step");
        escaped += accounting.boundary_loss;
        injected += accounting.source_work;
    }
    assert!(
        bare || escaped > 1.0e-9,
        "the wall must radiate in the fixture"
    );
    assert!(
        bare || injected.abs() > 1.0e-9,
        "the source must drive the fixture"
    );

    let clock = CanonicalGpuClock::initial(time_step).expect("forced temporal clock");
    let plan = CanonicalGpuPlan::compile_temporal(&operator, &state, &forcing, clock)
        .expect("forced temporal GPU plan");
    println!(
        "forced temporal gate: {} Q, {} b, wall carried {:.3e}, source supplied {:.3e}",
        plan.node_count, plan.sample_count, escaped, injected
    );

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern forced temporal validation".into(),
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
    canonical.request_steps(TOTAL_STEPS);
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
        eprintln!("forced temporal validation timed out or failed");
        expected.failed = true;
        expected.finished = true;
        return;
    }
    if request.stats().completed_steps() < TOTAL_STEPS {
        return;
    }
    if !request.request_full_state_readback(&mut commands) && display.full_readbacks == 0 {
        return;
    }
    if display.primary_flux.len() != expected.primary.len() {
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
        "forced temporal errors after {:.2} ms: Q {primary:.3e}, b {complementary:.3e}",
        expected.started.elapsed().as_secs_f64() * 1_000.0
    );
    expected.finished = true;
    // f32 against an f64 oracle over 48 steps of a driven, forced, radiating
    // fixture. A stage reading the authored mass instead of the instantaneous
    // one misses by the modulation depth, which is percent-level, not this.
    if primary > 2.0e-4 || complementary > 2.0e-4 {
        expected.failed = true;
    }
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
