//! Steady-stepping throughput of the f32 GPU core, driven against fixed.
//!
//! The CPU oracle's driven path costs about thirteen times its fixed path, but
//! that oracle is not what anyone runs. The production core is this one, whose
//! shader reads a stage's coefficients once where the reference recomputes them
//! in every helper that wants them. Whether the drive is affordable is decided
//! here and nowhere else.
//!
//! Pass `--fixed` for the comparison run; everything else about the fixture is
//! identical, including the mesh, the operator and the timestep.
//!
//! `--outgoing` swaps the first-order wall for a second-order one, which is the
//! only composition whose stage does non-local work. Its trace system carries
//! no nodal mass, so a driven generation sweeps it with the stage's own mass
//! rather than refactorizing, and this is where that sweep's cost is read.
//!
//! `--nonlinear` adds Kerr on the mass row and saturation on the stiffness row
//! to the driven medium, at an amplitude where both maps depart from linear,
//! so the step pays every inverse and, with `--outgoing`, the wall's Newton.
//!
//! `--filter` turns the resident grid filter on, which runs every sixteenth
//! step, so the figure includes its amortized cost.
//!
//! Stepping is unfenced, so the figure is what a step costs the device rather
//! than the readback round trip the interactive lead fence waits on.

use std::time::{Duration, Instant};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuPlan, CanonicalGpuRequest,
    CanonicalWaveGpuPlugin,
};
use funfern_app::wave_gpu::WaveGpuPlugin;
use funfern_core::{
    CanonicalForcing, CanonicalTemporalWaveOperator, CanonicalTemporalWaveState, CoefficientLaw,
    MeshingOptions, OuterBoundaryCondition, QuadraticWaveOperator, ScalarField, Scene, TimeDrive,
    mesh_scene,
};

const STEPS: u64 = 2_000;
const EDGE: f64 = 0.06;

#[derive(Resource)]
struct Pending {
    plan: Option<CanonicalGpuPlan>,
    filter: bool,
}

#[derive(Resource)]
struct Timing {
    label: &'static str,
    nodes: usize,
    time_step: f64,
    started: Option<Instant>,
    deadline: Instant,
    finished: bool,
}

fn main() -> AppExit {
    let driven = !std::env::args().any(|argument| argument == "--fixed");
    let outgoing = std::env::args().any(|argument| argument == "--outgoing");
    let nonlinear = std::env::args().any(|argument| argument == "--nonlinear");
    let filter = std::env::args().any(|argument| argument == "--filter");
    let amplitude = if nonlinear { 12.0 } else { 1.0 };
    let mut scene = Scene::initial();
    if driven {
        scene.materials[0].mass_law.drive = TimeDrive::ParametricPump {
            depth: ScalarField::constant(0.22),
            frequency_hz: ScalarField::constant(0.9),
            phase_radians: ScalarField::constant(0.2),
        };
        scene.materials[0].stiffness_law.drive = TimeDrive::TravellingModulation {
            depth: ScalarField::constant(0.15),
            frequency_hz: ScalarField::constant(0.7),
            phase_radians: ScalarField::constant(-0.1),
            wavenumber: ScalarField::constant(2.0),
            angle_radians: ScalarField::constant(0.4),
        };
    }

    if nonlinear {
        scene.materials[0].mass_law.field = funfern_core::FieldLaw::Polynomial {
            chi1: ScalarField::constant(0.0),
            chi2: ScalarField::constant(0.8),
            amplitude_bound: None,
        };
        scene.materials[0].stiffness_law.field = funfern_core::FieldLaw::Saturable {
            chi: ScalarField::constant(6.0),
            saturation: ScalarField::constant(0.3),
        };
    }
    let mut fixed_scene = scene.clone();
    for material in &mut fixed_scene.materials {
        material.mass_law = CoefficientLaw::linear();
        material.stiffness_law = CoefficientLaw::linear();
    }
    let mesh = mesh_scene(
        &fixed_scene,
        1,
        MeshingOptions {
            target_edge_length: EDGE,
            ..MeshingOptions::default()
        },
    )
    .expect("timing mesh");
    let scalar = QuadraticWaveOperator::assemble_scene(
        &mesh,
        &fixed_scene,
        if outgoing {
            OuterBoundaryCondition::SecondOrderOutgoing
        } else {
            OuterBoundaryCondition::FirstOrderOutgoing
        },
    )
    .expect("timing scalar operator");
    let operator = CanonicalTemporalWaveOperator::compile_scene(&mesh, &scalar, &scene, 1)
        .expect("timing temporal operator");
    let base = operator.base();

    // The driven ceiling is the lower of the two, so both runs use it and the
    // comparison is per step at a matched timestep.
    let time_step = 0.4 * operator.maximum_time_step();
    let primary = base
        .primary_mass()
        .iter()
        .zip(base.node_points())
        .map(|(mass, point)| mass * (amplitude * 0.03 * (1.3 * point.x - 0.7 * point.y).sin()))
        .collect::<Vec<_>>();
    let potential = base
        .node_points()
        .iter()
        .map(|point| amplitude * 0.02 * (0.9 * point.x + 1.1 * point.y).cos())
        .collect::<Vec<_>>();
    let complementary = base.compatible_flux(&potential).expect("timing flux");
    let state = CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary)
        .expect("timing state");
    let clock = CanonicalGpuClock::initial(time_step).expect("timing clock");
    let forcing = CanonicalForcing::none(base);
    let plan = CanonicalGpuPlan::compile_temporal(&operator, &state, &forcing, clock)
        .expect("timing GPU plan");

    if let Some(boundary) = base.outgoing_boundary() {
        println!(
            "gpu timing: {} trace nodes solved in {} sweeps",
            boundary.trace_nodes().len(),
            plan.trace_sweeps,
        );
    }
    let label = match (driven, nonlinear) {
        (_, true) => "nonlinear",
        (true, false) => "driven",
        (false, false) => "fixed",
    };
    println!(
        "gpu {label} timing: {} Q, {} b, dt {time_step:.4e}, {STEPS} steps",
        plan.node_count, plan.sample_count
    );

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern gpu temporal timing".into(),
            visible: false,
            resolution: (64, 64).into(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(WaveGpuPlugin)
    .add_plugins(CanonicalWaveGpuPlugin)
    .insert_resource(Pending {
        plan: Some(plan),
        filter,
    })
    .insert_resource(Timing {
        label,
        nodes: base.degrees_of_freedom(),
        time_step,
        started: None,
        deadline: Instant::now() + Duration::from_secs(300),
        finished: false,
    })
    .add_systems(Startup, install)
    .add_systems(Update, drive)
    .run()
}

fn install(
    mut commands: Commands,
    mut pending: ResMut<Pending>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut canonical: ResMut<CanonicalGpuRequest>,
) {
    canonical.set_unfenced_stepping(true);
    canonical.set_grid_scale_filter(pending.filter);
    canonical.install(
        &mut assets,
        &mut commands,
        pending.plan.take().expect("one plan"),
    );
    commands.spawn(Camera2d);
}

fn drive(
    mut request: ResMut<CanonicalGpuRequest>,
    display: Res<CanonicalGpuDisplay>,
    mut timing: ResMut<Timing>,
    mut exit: MessageWriter<AppExit>,
) {
    if timing.finished {
        exit.write(AppExit::Success);
        return;
    }
    if Instant::now() >= timing.deadline || request.stats().failure() != 0 {
        eprintln!("gpu timing timed out or failed");
        timing.finished = true;
        return;
    }
    if timing.started.is_none() {
        // Let the first frames settle before the clock starts, so pipeline
        // creation and the first buffer upload are not charged to stepping.
        if display.clock.is_none_or(|clock| clock.accepted_steps < 32) {
            request.request_steps(64);
            return;
        }
        timing.started = Some(Instant::now());
        request.request_steps(STEPS);
        return;
    }
    if request.stats().completed_steps() < STEPS + 32 {
        return;
    }
    let elapsed = timing.started.expect("started").elapsed().as_secs_f64();
    let per_step = elapsed * 1.0e6 / STEPS as f64;
    let simulated = timing.time_step * STEPS as f64;
    println!(
        "gpu {}: {} DOFs, {per_step:.1} us/step, {:.1} simulated s per wall s",
        timing.label,
        timing.nodes,
        simulated / elapsed
    );
    timing.finished = true;
}
