//! Real-device validation for the Stage 7 temporal conservative-bulk path.
//!
//! This deliberately uses the production render graph, shader, storage layout,
//! atomic commit path, state readback, and clock rebase. It is not an isolated
//! shader benchmark.

use std::time::{Duration, Instant};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuPlan, CanonicalGpuRequest,
    CanonicalWaveGpuPlugin,
};
use funfern_app::wave_gpu::WaveGpuPlugin;
use funfern_core::{
    CanonicalTemporalWaveOperator, CanonicalTemporalWaveState, CoefficientLaw, MeshingOptions,
    OuterBoundaryCondition, QuadraticWaveOperator, ScalarField, Scene, TimeDrive, mesh_scene,
};

const DEFAULT_STEPS: u64 = 96;

#[derive(Resource)]
struct PendingPlan(Option<CanonicalGpuPlan>);

#[derive(Resource)]
struct Expected {
    primary: Vec<f64>,
    complementary: Vec<[f64; 2]>,
    absolute_time: f64,
    initial_epoch: u64,
    initial_step: u64,
    steps: u64,
    time_step: f64,
    started: Instant,
    deadline: Instant,
    finished: bool,
    failed: bool,
}

fn main() {
    let steps = std::env::args()
        .find_map(|argument| argument.strip_prefix("--steps=")?.parse::<u64>().ok())
        .unwrap_or(DEFAULT_STEPS);
    assert!(steps >= 2, "the temporal gate must cross a clock rebase");

    let mut scene = Scene::initial();
    let material = &mut scene.materials[0];
    material.mass_law.drive = TimeDrive::TravellingModulation {
        depth: ScalarField::constant(0.24),
        frequency_hz: ScalarField::constant(0.8),
        phase_radians: ScalarField::constant(0.31),
        wavenumber: ScalarField::constant(2.7),
        angle_radians: ScalarField::constant(-0.4),
    };
    material.mass_law.alternate = Some(ScalarField::constant(1.8));
    material.mass_law.inverted = true;
    material.stiffness_law.drive = TimeDrive::TimeCrystal {
        depth: ScalarField::constant(0.17),
        frequency_hz: ScalarField::constant(0.6),
        phase_radians: ScalarField::constant(-0.23),
        sharpness: ScalarField::constant(3.2),
    };
    material.stiffness_law.alternate = Some(ScalarField::constant(0.75));
    let material_id = material.id;

    let mut fixed_scene = scene.clone();
    for material in &mut fixed_scene.materials {
        material.mass_law = CoefficientLaw::linear();
        material.stiffness_law = CoefficientLaw::linear();
    }
    let mesh = mesh_scene(
        &fixed_scene,
        1,
        MeshingOptions {
            target_edge_length: 0.18,
            ..MeshingOptions::default()
        },
    )
    .expect("temporal validation mesh");
    let scalar = QuadraticWaveOperator::assemble_scene(
        &mesh,
        &fixed_scene,
        OuterBoundaryCondition::Reflecting,
    )
    .expect("temporal validation scalar operator");
    let operator = CanonicalTemporalWaveOperator::compile_scene(&mesh, &scalar, &scene, 1)
        .expect("temporal validation operator");
    let time_step = 0.4 * operator.maximum_time_step();

    // Install immediately before the same threshold used by the production
    // render node. The second accepted step must therefore rebase both clock
    // and temporal runtime records.
    let rebase_limit = ((256.0 / time_step).ceil() as u64).min(1 << 16) as u32;
    assert!(rebase_limit > 2);
    let clock = CanonicalGpuClock {
        epoch: 7,
        epoch_origin_seconds: 1_000.0,
        step_in_epoch: rebase_limit - 2,
        time_step,
    };
    assert!(!clock.requires_rebase());

    let primary = operator
        .base()
        .primary_mass()
        .iter()
        .zip(operator.base().node_points())
        .map(|(mass, point)| mass * (0.04 + 0.03 * (1.3 * point.x).sin() * (0.8 * point.y).cos()))
        .collect::<Vec<_>>();
    let potential = operator
        .base()
        .node_points()
        .iter()
        .map(|point| 0.025 * (0.7 * point.x - 0.4 * point.y).cos())
        .collect::<Vec<_>>();
    let complementary = operator
        .base()
        .compatible_flux(&potential)
        .expect("compatible initial complementary flux");
    let mut state = CanonicalTemporalWaveState::new_at(
        &operator,
        time_step,
        primary,
        complementary,
        clock.time(),
    )
    .expect("clock-aligned temporal state");
    let switch_start = state.time() - 0.2;
    state
        .runtime_mut()
        .begin_switch(material_id, true, switch_start, 1.3)
        .expect("in-progress material Switch");

    let plan = CanonicalGpuPlan::compile_temporal_bulk(&operator, &state, clock)
        .expect("temporal GPU plan");
    let temporal = plan.manifest.temporal.expect("temporal layout manifest");
    let mut oracle = state;
    for _ in 0..steps {
        oracle.step(&operator).expect("f64 temporal oracle step");
    }
    let expected = Expected {
        primary: oracle.primary_flux().to_vec(),
        complementary: oracle
            .complementary_flux()
            .iter()
            .map(|value| [value.x, value.y])
            .collect(),
        absolute_time: oracle.time(),
        initial_epoch: clock.epoch,
        initial_step: clock.step_in_epoch as u64,
        steps,
        time_step,
        started: Instant::now(),
        deadline: Instant::now() + Duration::from_secs(90),
        finished: false,
        failed: false,
    };
    println!(
        "temporal GPU gate: {} Q, {} b, {} runtime records, {} steps across clock rebase",
        plan.node_count, plan.sample_count, temporal.runtime_record_count, steps,
    );

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern temporal GPU validation".into(),
            visible: false,
            resolution: (64, 64).into(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(WaveGpuPlugin)
    .add_plugins(CanonicalWaveGpuPlugin)
    .insert_resource(PendingPlan(Some(plan)))
    .insert_resource(expected)
    .add_systems(Startup, install)
    .add_systems(Update, finish_when_ready);
    app.run();
}

fn install(
    mut commands: Commands,
    mut plans: ResMut<PendingPlan>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut request: ResMut<CanonicalGpuRequest>,
    expected: Res<Expected>,
) {
    request
        .set_continuous_full_state_readback(true)
        .expect("select temporal validation readback mode");
    request.install(
        &mut assets,
        &mut commands,
        plans.0.take().expect("one pending temporal plan"),
    );
    request.request_steps(expected.steps);
    commands.spawn(Camera2d);
}

fn finish_when_ready(
    request: Res<CanonicalGpuRequest>,
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
    if Instant::now() >= expected.deadline {
        eprintln!(
            "temporal GPU validation timed out ({})",
            request.stats().status()
        );
        expected.finished = true;
        expected.failed = true;
        exit.write(AppExit::error());
        return;
    }
    if request.stats().failure() != 0 {
        eprintln!("temporal GPU failure {}", request.stats().failure());
        expected.finished = true;
        expected.failed = true;
        exit.write(AppExit::error());
        return;
    }

    let target = expected.initial_step + expected.steps;
    let Some(clock) = display.clock else { return };
    if (clock.accepted_steps as u64) < target
        || display.full_snapshot_completed_steps() < target
        || display.primary_flux.len() != expected.primary.len()
        || display.complementary_flux.len() != expected.complementary.len()
    {
        return;
    }

    let q_error = relative_l2(
        display.primary_flux.iter().map(|value| *value as f64),
        expected.primary.iter().copied(),
    );
    let b_error = relative_l2(
        display
            .complementary_flux
            .iter()
            .flat_map(|value| value.iter().map(|lane| *lane as f64)),
        expected
            .complementary
            .iter()
            .flat_map(|value| value.iter().copied()),
    );
    let clock_error = (clock.absolute_seconds - expected.absolute_time).abs();
    let elapsed = expected.started.elapsed().as_secs_f64();
    let crossed_rebase = clock.epoch > expected.initial_epoch;
    println!(
        "{} accepted temporal steps in {:.2} ms: Q error {:.3e}, b error {:.3e}, clock error {:.3e}; epoch {} -> {}, local step {}, {} dispatches",
        expected.steps,
        elapsed * 1_000.0,
        q_error,
        b_error,
        clock_error,
        expected.initial_epoch,
        clock.epoch,
        clock.step_in_epoch,
        request.stats().dispatches(),
    );
    expected.finished = true;
    if q_error > 6.0e-5
        || b_error > 6.0e-5
        || clock_error > 2.0e-4 * expected.time_step.max(1.0)
        || !crossed_rebase
    {
        eprintln!("temporal GPU accuracy or clock-rebase gate was exceeded");
        expected.failed = true;
        exit.write(AppExit::error());
    } else {
        exit.write(AppExit::Success);
    }
}

fn relative_l2(actual: impl Iterator<Item = f64>, expected: impl Iterator<Item = f64>) -> f64 {
    let (difference, scale) = actual.zip(expected).fold((0.0, 0.0), |sum, pair| {
        (sum.0 + (pair.0 - pair.1).powi(2), sum.1 + pair.1.powi(2))
    });
    (difference / scale.max(1.0e-30)).sqrt()
}
