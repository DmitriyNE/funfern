//! An integrated source's mean rate across the clock's rebase.
//!
//! Every authored harmonic source compiles to an integrated drive, its rate
//! `∫ a` held as an anchor and a carrier phase that a rebase carries to the
//! new origin. Here one drives a closed reflecting box in proportion to the
//! nodal mass, from rest, its carrier at the phase where that integral has no
//! mean: the field stays uniform, the mean mode has no restoring force, and
//! the box's total flux integrates the rate. A mean-rate error grows the total
//! linearly where the reference's only oscillates.
//!
//! The clock starts 64 steps before an epoch's rebase, 1,000 s into the run,
//! and runs 8 s past it. A rebase that formed the anchor apart from the
//! carrier's reduced phase moved the mean rate for good: the total read
//! 4.1e-4 of its swing off on the M1 Max, against 2.4e-5 with the anchor
//! formed from that phase (GPU-6 of the 2026-10-20 review). What is left is
//! the epoch's own: late in an epoch the carrier's phase rounds at the local
//! time's magnitude, which biases the mean rate until the next rebase, and a
//! run inside one epoch reads 2.8e-3 after 127 s. Soon after a rebase the
//! local time is small, which is why the run stops there.

#[path = "support/check.rs"]
mod check;
use check::within;
use std::time::Instant;

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuPlan, CanonicalGpuRequest,
    CanonicalWaveGpuPlugin,
};
use funfern_core::{
    CanonicalForcing, CanonicalIntegrator, CanonicalSource, CanonicalWaveOperator,
    CanonicalWaveState, MeshingOptions, OuterBoundaryCondition, QuadraticWaveOperator, Scene,
    TimeSignal, mesh_scene,
};

const ORIGIN: f64 = 1_000.0;
const BEFORE_REBASE: u64 = 64;
const AFTER_REBASE: u64 = 4_096;
const FREQUENCY_HZ: f64 = 2.0;

#[derive(Resource)]
struct PendingPlan(Option<CanonicalGpuPlan>);

#[derive(Resource)]
struct Expected {
    /// The reference's total flux at the end, and the largest it reached.
    total: f64,
    swing: f64,
    primary: Vec<f64>,
    initial_epoch: u64,
    initial_step: u64,
    steps: u64,
    settle_after: Option<u64>,
    deadline: Instant,
    failed: bool,
    finished: bool,
}

fn main() -> AppExit {
    let scene = Scene::initial();
    let mesh = mesh_scene(
        &scene,
        41,
        MeshingOptions {
            target_edge_length: 0.2,
            ..MeshingOptions::default()
        },
    )
    .expect("rebase mesh");
    let scalar =
        QuadraticWaveOperator::assemble_scene(&mesh, &scene, OuterBoundaryCondition::Reflecting)
            .expect("rebase scalar operator");
    let operator = CanonicalWaveOperator::compile_scene(&mesh, &scalar, &scene, 41)
        .expect("rebase canonical operator");
    let integrator = CanonicalIntegrator::FourthOrder;
    // A power of two, so the device's f32 step and the reference's count the
    // same seconds, and an epoch ends at 256 s on a whole step.
    let time_step = 2.0_f64.powi(
        (integrator.recommended() * operator.maximum_time_step())
            .log2()
            .floor() as i32,
    );
    let epoch_steps = ((256.0 / time_step).ceil() as u64).min(1 << 16);
    let initial_step = epoch_steps - BEFORE_REBASE;
    let start = ORIGIN + initial_step as f64 * time_step;
    let steps = BEFORE_REBASE + AFTER_REBASE;

    // At the phase π/2 the rate, (A/ω)(cos φ − cos(φ + ωt)), has no mean.
    let mut forcing = CanonicalForcing::none(&operator);
    forcing
        .push_source(
            CanonicalSource::legacy(
                &operator,
                operator.primary_mass().to_vec(),
                TimeSignal::harmonic(0.0, 1.0, FREQUENCY_HZ, std::f64::consts::FRAC_PI_2),
                0.0,
            )
            .expect("integrated source"),
        )
        .expect("integrated source layout");
    let initial = CanonicalWaveState::zero(&operator, time_step)
        .and_then(|state| state.with_time_origin(start))
        .expect("state at rest")
        .with_integrator(integrator);
    let clock = CanonicalGpuClock {
        epoch: 7,
        epoch_origin_seconds: ORIGIN,
        step_in_epoch: initial_step as u32,
        time_step,
    };
    let plan =
        CanonicalGpuPlan::compile_with_quadratic(&operator, &scalar, &initial, &forcing, clock)
            .expect("integrated-source GPU plan");

    let mut oracle = initial;
    let mut swing = 0.0_f64;
    for _ in 0..steps {
        oracle
            .step_with_forcing(&operator, &forcing)
            .expect("reference step");
        swing = swing.max(oracle.primary_flux().iter().sum::<f64>().abs());
    }
    let expected = Expected {
        total: oracle.primary_flux().iter().sum(),
        swing,
        primary: oracle.primary_flux().to_vec(),
        initial_epoch: clock.epoch,
        initial_step,
        steps,
        settle_after: None,
        deadline: check::deadline(60),
        failed: false,
        finished: false,
    };
    println!(
        "{} nodes, step {time_step} s, {epoch_steps} steps an epoch; {steps} steps from \
         {start:.3} s across a rebase",
        operator.degrees_of_freedom()
    );

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern integrated-source rebase validation".into(),
            visible: false,
            resolution: (64, 64).into(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(CanonicalWaveGpuPlugin)
    .insert_resource(PendingPlan(Some(plan)))
    .insert_resource(expected)
    .add_systems(Startup, install)
    .add_systems(Update, validate);
    app.run()
}

fn install(
    mut commands: Commands,
    mut pending: ResMut<PendingPlan>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut request: ResMut<CanonicalGpuRequest>,
    expected: Res<Expected>,
) {
    request
        .set_continuous_full_state_readback(true)
        .expect("select validation readback mode");
    request.set_unfenced_stepping(true);
    request.install(
        &mut assets,
        &mut commands,
        pending.0.take().expect("integrated-source plan"),
    );
    request.request_steps(expected.steps);
    commands.spawn(Camera2d);
}

fn validate(
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
    if Instant::now() >= expected.deadline || request.stats().failure() != 0 {
        eprintln!("integrated-source rebase validation timed out or failed");
        expected.failed = true;
        expected.finished = true;
        return;
    }
    let Some(clock) = display.clock else { return };
    if clock.accepted_steps < expected.initial_step + expected.steps {
        return;
    }
    let Some(settle_after) = expected.settle_after else {
        expected.settle_after = Some(display.readbacks + 2);
        return;
    };
    if display.readbacks < settle_after {
        return;
    }
    let total = display
        .primary_flux
        .iter()
        .map(|value| f64::from(*value))
        .sum::<f64>();
    let drift = (total - expected.total).abs() / expected.swing.max(1.0e-30);
    let (difference, scale) = display.primary_flux.iter().zip(&expected.primary).fold(
        (0.0, 0.0),
        |sum, (actual, expected)| {
            (
                sum.0 + (f64::from(*actual) - expected).powi(2),
                sum.1 + expected.powi(2),
            )
        },
    );
    let q_error = (difference / scale.max(1.0e-30)).sqrt();
    println!(
        "epoch {} -> {}: total flux {total:.6e} against {:.6e}, {drift:.2e} of its swing \
         {:.3e}; Q {q_error:.3e}",
        expected.initial_epoch, clock.epoch, expected.total, expected.swing
    );
    // The M1 Max reads 2.4e-5, and 4.1e-4 where the rebase moved the mean
    // rate: a quarter of that, and four times this.
    expected.failed = clock.epoch != expected.initial_epoch + 1 || !within(drift, 1.0e-4);
    expected.finished = true;
}
