//! Real-device validation that a live source edit composes with a driven
//! medium.
//!
//! A source edit is a paused zero-duration event that rewrites the drive table
//! and, for a weight patch, the nodal weights. Neither touches the material
//! runtime bank, and the weights are normalized by the generation's immutable
//! reference mass rather than the moving one, so the edit should compose with
//! a time-driven medium exactly as it does with a fixed one. This gate checks
//! that on the device.
//!
//! The generation starts at a nonzero epoch origin, which is where a clock
//! rebase leaves it, so the preserved carrier phase is anchored to an origin
//! that is not zero. Two edits land at the same boundary - weights and drive
//! first, then the drive alone - followed by a pulse, and the medium keeps
//! moving through all of it. The oracle carries each edit's phase forward from
//! the one before, as the device does. The gate first checks that an unedited
//! run and one that restarts the carrier at its authored phase both land far
//! from the oracle, so it cannot pass by measuring nothing.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuLiveEvent, CanonicalGpuPlan,
    CanonicalGpuRequest, CanonicalWaveGpuPlugin,
};
use funfern_core::{
    CanonicalForcing, CanonicalSource, CanonicalTemporalWaveOperator, CanonicalTemporalWaveState,
    CanonicalWaveOperator, CoefficientLaw, MeshingOptions, OuterBoundaryCondition,
    QuadraticWaveOperator, ScalarField, Scene, TimeDrive, TimeSignal, mesh_scene,
};

const WARMUP_STEPS: u64 = 24;
const MEASURED_STEPS: u64 = 320;
const EVENT_COUNT: u32 = 3;
/// Where a clock rebase would have left the epoch.
const EPOCH_ORIGIN: f64 = 37.25;
/// Device f32 against the f64 oracle. The discrimination checks keep this
/// far below what a wrong edit would cost.
const TOLERANCE: f64 = 3.0e-4;

#[derive(Resource)]
struct Pending {
    plan: Option<CanonicalGpuPlan>,
    events: VecDeque<CanonicalGpuLiveEvent>,
}

#[derive(Resource)]
struct Expected {
    primary: Vec<f64>,
    complementary: Vec<[f64; 2]>,
    phase: Phase,
    paused_time: f64,
    event_started: bool,
    last_observed_serial: u32,
    settle_after: Option<u64>,
    failed: bool,
    deadline: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Warmup,
    Event,
    Evolution,
    Done,
}

fn source(
    operator: &CanonicalWaveOperator,
    weights: &[f64],
    signal: TimeSignal,
) -> CanonicalForcing {
    let mut forcing = CanonicalForcing::none(operator);
    forcing
        .push_source(CanonicalSource::direct(operator, weights.to_vec(), signal).unwrap())
        .unwrap();
    forcing
}

fn main() -> AppExit {
    let mut scene = Scene::initial();
    let material = &mut scene.materials[0];
    material.mass_law.drive = TimeDrive::ParametricPump {
        depth: ScalarField::constant(0.3),
        frequency_hz: ScalarField::constant(0.85),
        phase_radians: ScalarField::constant(1.1),
    };
    material.stiffness_law.drive = TimeDrive::TimeCrystal {
        depth: ScalarField::constant(0.15),
        frequency_hz: ScalarField::constant(0.61),
        phase_radians: ScalarField::constant(-0.19),
        sharpness: ScalarField::constant(3.0),
    };
    let mut fixed_scene = scene.clone();
    for material in &mut fixed_scene.materials {
        material.mass_law = CoefficientLaw::linear();
        material.stiffness_law = CoefficientLaw::linear();
    }
    let mesh = mesh_scene(
        &fixed_scene,
        1,
        MeshingOptions {
            target_edge_length: 0.16,
            ..MeshingOptions::default()
        },
    )
    .expect("live source mesh");
    let scalar = QuadraticWaveOperator::assemble_scene(
        &mesh,
        &fixed_scene,
        OuterBoundaryCondition::Reflecting,
    )
    .expect("live source scalar operator");
    let operator = CanonicalTemporalWaveOperator::compile_scene(&mesh, &scalar, &scene, 1)
        .expect("live source temporal operator");
    let base = operator.base();
    let time_step = 0.4 * operator.maximum_time_step();

    // Weights over the immutable reference mass, which is the contract for a
    // point or volume source.
    let weights = base
        .primary_mass()
        .iter()
        .zip(base.node_points())
        .map(|(mass, point)| 0.02 * mass * (0.6 * point.x - 0.1 * point.y).cos())
        .collect::<Vec<_>>();
    let new_weights = weights
        .iter()
        .zip(base.node_points())
        .map(|(weight, point)| weight * (1.0 + 0.15 * point.x))
        .collect::<Vec<_>>();
    let (old_frequency, old_phase) = (0.9, 0.35);
    let (middle_frequency, final_frequency) = (1.45, 0.62);
    let old_forcing = source(
        base,
        &weights,
        TimeSignal::harmonic(0.01, 0.04, old_frequency, old_phase),
    );
    // The authored phases are deliberately different: a live edit keeps the
    // carrier running rather than restarting it.
    let middle_authored = source(
        base,
        &new_weights,
        TimeSignal::harmonic(0.015, 0.03, middle_frequency, -1.7),
    );
    let final_authored = source(
        base,
        &new_weights,
        TimeSignal::harmonic(0.012, 0.05, final_frequency, 2.4),
    );

    // A zero state, so everything the readback holds came from the source and
    // an edit that went wrong is an error of the whole state.
    let initial = CanonicalTemporalWaveState::new_at(
        &operator,
        time_step,
        vec![0.0; base.degrees_of_freedom()],
        vec![Default::default(); base.complementary_degrees_of_freedom()],
        EPOCH_ORIGIN,
    )
    .expect("live source initial state");
    let clock = CanonicalGpuClock {
        epoch: 1,
        epoch_origin_seconds: EPOCH_ORIGIN,
        step_in_epoch: 0,
        time_step,
    };
    let plan = CanonicalGpuPlan::compile_temporal(&operator, &initial, &old_forcing, clock)
        .expect("live source temporal GPU plan");

    let pulse = base
        .node_points()
        .iter()
        .map(|point| 2.0e-4 * (0.8 * point.x - 0.4 * point.y).sin())
        .collect::<Vec<_>>();
    let events = VecDeque::from([
        CanonicalGpuLiveEvent::source_weight_patch(&old_forcing, &middle_authored, time_step, 1)
            .expect("live source-weight event"),
        CanonicalGpuLiveEvent::source_patch(&final_authored, time_step, 2)
            .expect("live source-drive event"),
        CanonicalGpuLiveEvent::primary_pulse(base, &pulse, 3).expect("live pulse event"),
    ]);

    let mut warm = initial;
    for _ in 0..WARMUP_STEPS {
        warm.step_with_forcing(&operator, &old_forcing)
            .expect("old-source oracle step");
    }
    let edit_time = warm.time();
    // Each edit keeps the carrier's phase at the instant it lands.
    let tau = std::f64::consts::TAU;
    let middle_anchor = old_phase + tau * (old_frequency - middle_frequency) * edit_time;
    let final_anchor = middle_anchor + tau * (middle_frequency - final_frequency) * edit_time;
    let accepted = source(
        base,
        &new_weights,
        TimeSignal::harmonic(0.012, 0.05, final_frequency, final_anchor),
    );
    let run = |forcing: &CanonicalForcing| {
        let mut state = warm.clone();
        state
            .apply_primary_pulse(&operator, forcing, &pulse)
            .expect("pulse oracle event");
        for _ in 0..MEASURED_STEPS {
            state
                .step_with_forcing(&operator, forcing)
                .expect("new-source oracle step");
        }
        state
    };
    let oracle = run(&accepted);
    let unedited = run(&old_forcing);
    let restarted = run(&final_authored);
    let separation = |other: &CanonicalTemporalWaveState| {
        relative_l2(
            other.primary_flux().iter().copied(),
            oracle.primary_flux().iter().copied(),
        )
    };
    let (unedited, restarted) = (separation(&unedited), separation(&restarted));
    println!(
        "temporal live source gate: an unedited run stands {unedited:.3e} from the oracle and a \
         restarted carrier {restarted:.3e}"
    );
    assert!(
        unedited > 50.0 * TOLERANCE && restarted > 50.0 * TOLERANCE,
        "the fixture cannot tell a correct edit from a wrong one"
    );

    let expected = Expected {
        primary: oracle.primary_flux().to_vec(),
        complementary: oracle
            .complementary_flux()
            .iter()
            .map(|value| [value.x, value.y])
            .collect(),
        phase: Phase::Warmup,
        paused_time: edit_time,
        event_started: false,
        last_observed_serial: 0,
        settle_after: None,
        failed: false,
        deadline: Instant::now() + Duration::from_secs(90),
    };

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern temporal live source validation".into(),
            visible: false,
            resolution: (64, 64).into(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(CanonicalWaveGpuPlugin)
    .insert_resource(Pending {
        plan: Some(plan),
        events,
    })
    .insert_resource(expected)
    .add_systems(Startup, install)
    .add_systems(Update, validate);
    app.run()
}

fn install(
    mut commands: Commands,
    mut pending: ResMut<Pending>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut request: ResMut<CanonicalGpuRequest>,
) {
    request
        .set_continuous_full_state_readback(true)
        .expect("select validation readback mode");
    request.install(
        &mut assets,
        &mut commands,
        pending.plan.take().expect("live source plan"),
    );
    request.request_steps(WARMUP_STEPS);
    commands.spawn(Camera2d);
}

fn validate(
    mut pending: ResMut<Pending>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut request: ResMut<CanonicalGpuRequest>,
    display: Res<CanonicalGpuDisplay>,
    mut expected: ResMut<Expected>,
    mut exit: MessageWriter<AppExit>,
) {
    if expected.phase == Phase::Done {
        exit.write(if expected.failed {
            AppExit::error()
        } else {
            AppExit::Success
        });
        return;
    }
    if Instant::now() >= expected.deadline || request.stats().failure() != 0 {
        eprintln!("temporal live source validation timed out or failed");
        expected.failed = true;
        expected.phase = Phase::Done;
        return;
    }
    let Some(clock) = display.clock else { return };
    match expected.phase {
        Phase::Warmup if clock.accepted_steps >= WARMUP_STEPS as u32 => {
            expected.phase = Phase::Event;
        }
        Phase::Event => {
            let processed = request.stats().processed_event();
            if processed > expected.last_observed_serial {
                if processed != expected.last_observed_serial + 1
                    || request.stats().event_rejection() != 0
                    || clock.accepted_steps != WARMUP_STEPS as u32
                    || (clock.absolute_seconds - expected.paused_time).abs() > 2.0e-5
                {
                    eprintln!("a paused edit changed time, skipped a serial, or was rejected");
                    expected.failed = true;
                    expected.phase = Phase::Done;
                    return;
                }
                expected.last_observed_serial = processed;
                expected.event_started = false;
            }
            if !expected.event_started
                && let Some(event) = pending.events.front().cloned()
            {
                match request.queue_live_event(&mut assets, event) {
                    Ok(()) => {
                        pending.events.pop_front();
                        expected.event_started = true;
                    }
                    Err("another canonical transaction is pending") => {}
                    Err(error) => {
                        eprintln!("the driven generation refused a live edit: {error}");
                        expected.failed = true;
                        expected.phase = Phase::Done;
                    }
                }
                return;
            }
            if processed != EVENT_COUNT {
                return;
            }
            if display.runtime_serials[0] != 2 || display.runtime_serials[2] != 3 {
                eprintln!(
                    "the edits did not take ownership of their slots: {:?}",
                    display.runtime_serials
                );
                expected.failed = true;
                expected.phase = Phase::Done;
                return;
            }
            request.request_steps(MEASURED_STEPS);
            expected.phase = Phase::Evolution;
        }
        Phase::Evolution if clock.accepted_steps >= (WARMUP_STEPS + MEASURED_STEPS) as u32 => {
            if expected.settle_after.is_none() {
                expected.settle_after = Some(display.readbacks + 2);
                return;
            }
            if display.readbacks < expected.settle_after.unwrap() {
                return;
            }
            if display.primary_flux.len() != expected.primary.len()
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
            println!(
                "three paused edits on a driven medium from epoch origin {EPOCH_ORIGIN}: \
                 Q {q_error:.3e}, b {b_error:.3e}"
            );
            expected.failed = q_error > TOLERANCE || b_error > TOLERANCE;
            expected.phase = Phase::Done;
        }
        _ => {}
    }
}

fn relative_l2(actual: impl Iterator<Item = f64>, expected: impl Iterator<Item = f64>) -> f64 {
    let (difference, scale) = actual.zip(expected).fold((0.0, 0.0), |sum, pair| {
        (sum.0 + (pair.0 - pair.1).powi(2), sum.1 + pair.1.powi(2))
    });
    (difference / scale.max(1.0e-30)).sqrt()
}
