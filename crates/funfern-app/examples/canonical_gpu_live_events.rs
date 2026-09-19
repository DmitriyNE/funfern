//! Paused zero-duration edits on the production canonical GPU core.

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
    CanonicalForcing, CanonicalSource, CanonicalWaveOperator, CanonicalWaveState, MeshingOptions,
    OuterBoundaryCondition, QuadraticWaveOperator, Scene, TimeSignal, mesh_scene,
};

const WARMUP_STEPS: u64 = 20;
const MEASURED_STEPS: u64 = 48;
const FILTER_STRENGTH: f64 = 0.35;
const EVENT_COUNT: u32 = 5;

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
    event_started: Option<Instant>,
    batch_started: Option<Instant>,
    maximum_event_elapsed: Duration,
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

fn main() {
    let scene = Scene::initial();
    let mesh = mesh_scene(
        &scene,
        31,
        MeshingOptions {
            target_edge_length: 0.12,
            ..MeshingOptions::default()
        },
    )
    .expect("event mesh");
    let scalar = QuadraticWaveOperator::assemble_scene(
        &mesh,
        &scene,
        OuterBoundaryCondition::FirstOrderOutgoing,
    )
    .expect("event scalar operator");
    let operator = CanonicalWaveOperator::compile_scene(&mesh, &scalar, &scene, 31)
        .expect("event canonical operator");
    let time_step = 0.8 * operator.maximum_time_step();
    let primary = operator
        .node_points()
        .iter()
        .map(|point| 0.08 * (0.7 * point.x + 0.3 * point.y).cos())
        .collect::<Vec<_>>();
    let potential = operator
        .node_points()
        .iter()
        .map(|point| 0.03 * (0.5 * point.x - 0.2 * point.y).sin())
        .collect::<Vec<_>>();
    let initial =
        CanonicalWaveState::from_primary_and_potential(&operator, time_step, &primary, &potential)
            .expect("event initial state");
    let weights = operator
        .primary_mass()
        .iter()
        .zip(operator.node_points())
        .map(|(mass, point)| 0.02 * mass * (0.6 * point.x - 0.1 * point.y).cos())
        .collect::<Vec<_>>();
    let old_frequency = 0.9;
    let old_phase = 0.35;
    let mut old_forcing = CanonicalForcing::none(&operator);
    old_forcing
        .push_source(
            CanonicalSource::direct(
                &operator,
                weights.clone(),
                TimeSignal::harmonic(0.01, 0.04, old_frequency, old_phase),
            )
            .unwrap(),
        )
        .unwrap();
    let plan = CanonicalGpuPlan::compile(
        &operator,
        &initial,
        &old_forcing,
        CanonicalGpuClock::initial(time_step).unwrap(),
    )
    .expect("event GPU plan");

    let new_frequency = 1.45;
    let mut authored_new_forcing = CanonicalForcing::none(&operator);
    authored_new_forcing
        .push_source(
            CanonicalSource::direct(
                &operator,
                weights.clone(),
                // This authored phase is intentionally different: the live
                // event preserves the current carrier unless phase is edited
                // through a future explicit phase-edit operation.
                TimeSignal::harmonic(0.015, 0.03, new_frequency, -1.7),
            )
            .unwrap(),
        )
        .unwrap();
    let pulse = operator
        .node_points()
        .iter()
        .map(|point| 0.002 * (0.8 * point.x - 0.4 * point.y).sin())
        .collect::<Vec<_>>();
    let events = VecDeque::from([
        CanonicalGpuLiveEvent::linear_loss_patch(
            time_step,
            &vec![0.0; operator.degrees_of_freedom()],
            &vec![0.0; operator.complementary_degrees_of_freedom()],
            1,
        )
        .expect("live law event"),
        CanonicalGpuLiveEvent::source_patch(&authored_new_forcing, time_step, 2)
            .expect("live source event"),
        CanonicalGpuLiveEvent::primary_pulse(&operator, &pulse, 3).expect("live pulse event"),
        CanonicalGpuLiveEvent::grid_filter(FILTER_STRENGTH, 4).expect("live filter event"),
        CanonicalGpuLiveEvent::maintenance(&vec![0.0; operator.degrees_of_freedom()], 5)
            .expect("live maintenance event"),
    ]);

    let mut oracle = initial;
    for _ in 0..WARMUP_STEPS {
        oracle
            .step_with_forcing(&operator, &old_forcing)
            .expect("old-source oracle step");
    }
    let edit_time = WARMUP_STEPS as f64 * time_step;
    let preserved_anchor =
        old_phase + std::f64::consts::TAU * (old_frequency - new_frequency) * edit_time;
    let mut accepted_new_forcing = CanonicalForcing::none(&operator);
    accepted_new_forcing
        .push_source(
            CanonicalSource::direct(
                &operator,
                weights,
                TimeSignal::harmonic(0.015, 0.03, new_frequency, preserved_anchor),
            )
            .unwrap(),
        )
        .unwrap();
    oracle
        .apply_primary_pulse(&operator, &accepted_new_forcing, &pulse)
        .expect("pulse oracle event");
    oracle
        .apply_grid_filter(&operator, &accepted_new_forcing, FILTER_STRENGTH)
        .expect("filter oracle event");
    for _ in 0..MEASURED_STEPS {
        oracle
            .step_with_forcing(&operator, &accepted_new_forcing)
            .expect("new-source oracle step");
    }
    let expected = Expected {
        primary: oracle.primary_flux().to_vec(),
        complementary: oracle
            .complementary_flux()
            .iter()
            .map(|value| [value.x, value.y])
            .collect(),
        phase: Phase::Warmup,
        paused_time: edit_time,
        event_started: None,
        batch_started: None,
        maximum_event_elapsed: Duration::ZERO,
        last_observed_serial: 0,
        settle_after: None,
        failed: false,
        deadline: Instant::now() + Duration::from_secs(90),
    };

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern canonical GPU live-event validation".into(),
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
    app.run();
}

fn install(
    mut commands: Commands,
    mut pending: ResMut<Pending>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut request: ResMut<CanonicalGpuRequest>,
) {
    request.install(
        &mut assets,
        &mut commands,
        pending.plan.take().expect("event plan"),
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
        eprintln!("live-event validation timed out or failed");
        expected.failed = true;
        expected.phase = Phase::Done;
        return;
    }
    let Some(clock) = display.clock else { return };
    match expected.phase {
        Phase::Warmup if clock.accepted_steps >= WARMUP_STEPS as u32 => {
            expected.batch_started = Some(Instant::now());
            expected.phase = Phase::Event;
        }
        Phase::Event => {
            let processed = request.stats().processed_event();
            if processed > expected.last_observed_serial {
                if processed != expected.last_observed_serial + 1
                    || request.stats().event_rejection() != 0
                    || clock.accepted_steps != WARMUP_STEPS as u32
                    || (clock.absolute_seconds - expected.paused_time).abs() > 2.0e-6
                {
                    eprintln!("paused event changed time, skipped a serial, or was rejected");
                    expected.failed = true;
                    expected.phase = Phase::Done;
                    return;
                }
                expected.maximum_event_elapsed = expected
                    .maximum_event_elapsed
                    .max(expected.event_started.take().unwrap().elapsed());
                expected.last_observed_serial = processed;
            }
            if expected.event_started.is_none()
                && let Some(event) = pending.events.front().cloned()
            {
                if request.queue_live_event(&mut assets, event).is_ok() {
                    pending.events.pop_front();
                    expected.event_started = Some(Instant::now());
                }
                return;
            }
            if processed != EVENT_COUNT {
                return;
            }
            if display.runtime_serials != [2, 1, 3, 5]
                || clock.accepted_steps != WARMUP_STEPS as u32
                || (clock.absolute_seconds - expected.paused_time).abs() > 2.0e-6
            {
                eprintln!("paused events failed runtime ownership or changed time");
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
                "five paused events committed in {:.2} ms total ({:.2} ms maximum); Q {:.3e}, b {:.3e}, clock unchanged",
                expected.batch_started.unwrap().elapsed().as_secs_f64() * 1_000.0,
                expected.maximum_event_elapsed.as_secs_f64() * 1_000.0,
                q_error,
                b_error,
            );
            expected.failed = q_error > 3.0e-5 || b_error > 3.0e-5;
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
