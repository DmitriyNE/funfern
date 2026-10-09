//! Real-device validation of pulsed signals.
//!
//! Four sources carry one pulse each: a flat top, a Gaussian flash with no
//! carrier, a sinc, and a Hann train that started long before the run. A pin
//! carries a Gaussian flash. The generation starts at a nonzero epoch origin,
//! where a clock rebase would leave it, so every start is relative to an
//! origin that is not zero, and the train's has been reduced on the host.
//!
//! Partway through, while the pulses are under way, a live source edit
//! replaces every pulse: new amplitudes, a later start, a different repeat.
//! A patch's records are relative to t = 0, so the device moves each start
//! onto its running epoch itself. The oracle is the CPU reference with its
//! signals shifted by the epoch origin, since its own clock starts at zero.
//!
//! The gate first checks that the pulses put something in and that an
//! unedited run lands far from the oracle, so it cannot pass by measuring
//! nothing.
//!
//! `--long` runs on past the first clock rebase, which moves every start by
//! the epoch it closes, some 72 s in at this step. Over that many steps the
//! f32 trajectory parts from the f64 one by more than roundoff in one step,
//! so the bound there is looser; a start the rebase lost or a carrier phase
//! it moved is a wrong train, far outside it.

#[path = "support/check.rs"]
mod check;
use check::within;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuLiveEvent, CanonicalGpuPlan,
    CanonicalGpuRequest, CanonicalWaveGpuPlugin, LiveEventRefusal,
};
use funfern_core::{
    CanonicalForcing, CanonicalSource, CanonicalWaveOperator, CanonicalWaveState, MeshingOptions,
    OuterBoundaryCondition, PulseEnvelope, QuadraticWaveOperator, Scene, TimeSignal, mesh_scene,
};

/// Where a clock rebase would have left the epoch.
const EPOCH_ORIGIN: f64 = 37.25;
/// Local seconds at the edit, and at the end of the run.
const EDIT_TIME: f64 = 0.45;
const END_TIME: f64 = 1.9;
/// Steps after which the device rebases its clock.
const REBASE_STEPS: u64 = 1 << 16;
/// Device f32 against the f64 oracle. The discrimination check keeps this far
/// below what a wrong edit would cost.
const TOLERANCE: f64 = 5.0e-5;
const LONG_TOLERANCE: f64 = 2.0e-4;

#[derive(Resource)]
struct Pending {
    plan: Option<CanonicalGpuPlan>,
    events: VecDeque<CanonicalGpuLiveEvent>,
}

#[derive(Resource)]
struct Expected {
    primary: Vec<f64>,
    complementary: Vec<[f64; 2]>,
    warmup_steps: u64,
    measured_steps: u64,
    tolerance: f64,
    long: bool,
    phase: Phase,
    event_started: bool,
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

/// The authored pulses, at absolute time; `edited` is what the live edit
/// replaces them with.
fn pulses(edited: bool) -> [TimeSignal; 4] {
    let later = if edited { 0.2 } else { 0.0 };
    let gain = if edited { 1.4 } else { 1.0 };
    [
        TimeSignal::pulsed(
            [0.0, 0.05 * gain, 2.0, 0.0],
            PulseEnvelope::FlatTop {
                duration: 0.6,
                edge: 0.2,
            },
            EPOCH_ORIGIN + 0.1 + later,
            0.0,
        ),
        TimeSignal::pulsed(
            [0.03 * gain, 0.0, 1.0, 0.0],
            PulseEnvelope::Gaussian { width: 0.08 },
            EPOCH_ORIGIN + 0.25 + later,
            0.0,
        ),
        TimeSignal::pulsed(
            [0.0, 0.04 * gain, 1.5, 0.3],
            PulseEnvelope::Sinc {
                bandwidth_hz: 2.0,
                lobes: 3,
            },
            EPOCH_ORIGIN + 0.05 + later,
            0.0,
        ),
        TimeSignal::pulsed(
            [0.0, 0.03 * gain, 3.0, 0.0],
            PulseEnvelope::FlatTop {
                duration: 0.3,
                edge: 0.15,
            },
            EPOCH_ORIGIN - 5.3,
            if edited { 0.55 } else { 0.45 },
        ),
    ]
}

fn pin() -> TimeSignal {
    TimeSignal::pulsed(
        [0.02, 0.0, 1.0, 0.0],
        PulseEnvelope::Gaussian { width: 0.1 },
        EPOCH_ORIGIN + 0.2,
        0.0,
    )
}

/// The same pulse on a clock that starts `origin` seconds later.
fn shifted(signal: TimeSignal, origin: f64) -> TimeSignal {
    let TimeSignal::Pulsed {
        envelope,
        start,
        repeat,
        ..
    } = signal
    else {
        unreachable!("every signal here is a pulse")
    };
    TimeSignal::pulsed(signal.carrier(), envelope, start - origin, repeat)
}

fn forcing(
    operator: &CanonicalWaveOperator,
    weights: &[Vec<f64>],
    signals: [TimeSignal; 4],
    pin: TimeSignal,
    origin: f64,
) -> CanonicalForcing {
    let mut prescribed = vec![None; operator.degrees_of_freedom()];
    prescribed[0] = Some(shifted(pin, origin));
    let mut forcing = CanonicalForcing::from_prescribed(operator, prescribed).unwrap();
    for (weights, signal) in weights.iter().zip(signals) {
        forcing
            .push_source(
                CanonicalSource::direct(operator, weights.clone(), shifted(signal, origin))
                    .unwrap(),
            )
            .unwrap();
    }
    forcing
}

fn main() -> AppExit {
    let long = std::env::args().any(|argument| argument == "--long");
    let tolerance = if long { LONG_TOLERANCE } else { TOLERANCE };
    let scene = Scene::initial();
    let mesh = mesh_scene(
        &scene,
        1,
        MeshingOptions {
            target_edge_length: 0.14,
            ..MeshingOptions::default()
        },
    )
    .expect("pulse mesh");
    // Radiating walls on the long run, so the train's energy does not pile
    // up for a minute and more.
    let walls = if long {
        OuterBoundaryCondition::FirstOrderOutgoing
    } else {
        OuterBoundaryCondition::Reflecting
    };
    let scalar =
        QuadraticWaveOperator::assemble_scene(&mesh, &scene, walls).expect("pulse scalar operator");
    let operator =
        CanonicalWaveOperator::compile_scene(&mesh, &scalar, &scene, 1).expect("pulse operator");
    let time_step = 0.4 * operator.maximum_time_step();
    let warmup_steps = (EDIT_TIME / time_step).round() as u64;
    let measured_steps = if long {
        REBASE_STEPS + 3000 - warmup_steps
    } else {
        ((END_TIME - EDIT_TIME) / time_step).round() as u64
    };

    // Weights over the reference mass, each source on its own part of the
    // domain.
    let weights = [(0.5, 0.2), (-0.4, 0.3), (0.1, -0.5), (-0.2, -0.1)]
        .map(|(x, y)| {
            operator
                .primary_mass()
                .iter()
                .zip(operator.node_points())
                .map(|(mass, point)| {
                    let (dx, dy) = (point.x - x, point.y - y);
                    mass * (-(dx * dx + dy * dy) / 0.05).exp()
                })
                .collect::<Vec<_>>()
        })
        .to_vec();

    // The device runs on absolute time, the oracle from zero.
    let device = forcing(&operator, &weights, pulses(false), pin(), 0.0);
    let edit = forcing(&operator, &weights, pulses(true), pin(), 0.0);
    let before = forcing(&operator, &weights, pulses(false), pin(), EPOCH_ORIGIN);
    let after = forcing(&operator, &weights, pulses(true), pin(), EPOCH_ORIGIN);

    let initial = CanonicalWaveState::zero(&operator, time_step).expect("pulse initial state");
    let clock = CanonicalGpuClock {
        epoch: 1,
        epoch_origin_seconds: EPOCH_ORIGIN,
        step_in_epoch: 0,
        time_step,
    };
    let plan =
        CanonicalGpuPlan::compile_with_quadratic(&operator, &scalar, &initial, &device, clock)
            .expect("pulse GPU plan");
    let events =
        VecDeque::from([
            CanonicalGpuLiveEvent::source_patch(&device, &edit, time_step, 1)
                .expect("live pulse edit"),
        ]);

    let mut warm = initial;
    for _ in 0..warmup_steps {
        warm.step_with_forcing(&operator, &before)
            .expect("oracle step before the edit");
    }
    let run = |forcing: &CanonicalForcing| {
        let mut state = warm.clone();
        for _ in 0..measured_steps {
            state
                .step_with_forcing(&operator, forcing)
                .expect("oracle step after the edit");
        }
        state
    };
    let oracle = run(&after);
    let unedited = run(&before);
    let norm = oracle
        .primary_flux()
        .iter()
        .map(|value| value * value)
        .sum::<f64>()
        .sqrt();
    let separation = relative_l2(
        unedited.primary_flux().iter().copied(),
        oracle.primary_flux().iter().copied(),
    );
    println!(
        "pulse gate: {warmup_steps} + {measured_steps} steps of {time_step:.3e} s; |Q| {norm:.3e}, \
         an unedited run stands {separation:.3e} from the oracle"
    );
    assert!(norm > 1.0e-6, "the pulses put almost nothing in");
    assert!(
        separation > 50.0 * tolerance,
        "the fixture cannot tell a correct edit from a wrong one"
    );

    let expected = Expected {
        primary: oracle.primary_flux().to_vec(),
        complementary: oracle
            .complementary_flux()
            .iter()
            .map(|value| [value.x, value.y])
            .collect(),
        warmup_steps,
        measured_steps,
        tolerance,
        long,
        phase: Phase::Warmup,
        event_started: false,
        settle_after: None,
        failed: false,
        deadline: Instant::now() + Duration::from_secs(if long { 600 } else { 90 }),
    };

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern pulse validation".into(),
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
    expected: Res<Expected>,
) {
    request
        .set_continuous_full_state_readback(true)
        .expect("select validation readback mode");
    request.install(
        &mut assets,
        &mut commands,
        pending.plan.take().expect("pulse plan"),
    );
    request.request_steps(expected.warmup_steps);
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
        eprintln!("pulse validation timed out or failed");
        expected.failed = true;
        expected.phase = Phase::Done;
        return;
    }
    let Some(clock) = display.clock else { return };
    match expected.phase {
        Phase::Warmup if clock.accepted_steps >= expected.warmup_steps as u32 => {
            expected.phase = Phase::Event;
        }
        Phase::Event => {
            if !expected.event_started {
                if let Some(event) = pending.events.front().cloned() {
                    match request.queue_live_event(&mut assets, event) {
                        Ok(()) => {
                            pending.events.pop_front();
                            expected.event_started = true;
                        }
                        Err(LiveEventRefusal::Busy) => {}
                        Err(error) => {
                            eprintln!("the generation refused the live pulse edit: {error}");
                            expected.failed = true;
                            expected.phase = Phase::Done;
                        }
                    }
                }
                return;
            }
            if request.stats().processed_event() != 1 {
                return;
            }
            if request.stats().event_rejection() != 0
                || clock.accepted_steps != expected.warmup_steps as u32
            {
                eprintln!(
                    "the live pulse edit was rejected ({}) or moved the clock",
                    request.stats().event_rejection()
                );
                expected.failed = true;
                expected.phase = Phase::Done;
                return;
            }
            request.request_steps(expected.measured_steps);
            expected.phase = Phase::Evolution;
        }
        Phase::Evolution
            if clock.accepted_steps >= (expected.warmup_steps + expected.measured_steps) as u32 =>
        {
            if expected.settle_after.is_none() {
                expected.settle_after = Some(display.readbacks + 2);
                return;
            }
            if display.readbacks < expected.settle_after.unwrap()
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
            println!(
                "four pulsed sources and a pulsed pin from epoch origin {EPOCH_ORIGIN}, edited \
                 live mid-pulse, {} steps in epoch {}: Q {q_error:.3e}, b {b_error:.3e}",
                clock.accepted_steps, clock.epoch
            );
            if expected.long && clock.epoch < 2 {
                eprintln!("the long run ended without a clock rebase");
                expected.failed = true;
            }
            expected.failed |=
                !within(q_error, expected.tolerance) || !within(b_error, expected.tolerance);
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
