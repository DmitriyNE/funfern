//! Real-device validation of gated material drives.
//!
//! Every runtime lane of one material carries a gated drive: a travelling
//! mass modulation under a flat-top train that started long before the run,
//! a time-crystal stiffness under one Gaussian, the primary loss pumped under
//! a sinc and the complementary loss under a flat top. The generation is
//! installed two steps short of the clock rebase, at an epoch origin far from
//! zero, so every gate start is packed against one origin and then moved onto
//! the next by the device, the train's folded back within one repeat.
//!
//! The oracle is the f64 temporal reference on the same absolute clock. The
//! gate first checks that the gates matter: the same drives ungated, and the
//! medium undriven, both land far from the oracle, so the run cannot pass by
//! measuring nothing.

#[path = "support/check.rs"]
mod check;
use check::within;
use std::time::{Duration, Instant};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuPlan, CanonicalGpuRequest,
    CanonicalWaveGpuPlugin,
};
use funfern_app::wave_gpu::WaveGpuPlugin;
use funfern_core::{
    CanonicalTemporalWaveOperator, CanonicalTemporalWaveState, CoefficientLaw, DampingLaw,
    LossChannel, MeshingOptions, OuterBoundaryCondition, PulseEnvelope, PulseTrain,
    QuadraticWaveOperator, RateLaw, ScalarField, Scene, TimeDrive, TriMesh, mesh_scene,
};

const EPOCH_ORIGIN: f64 = 1_000.0;
/// Seconds of evolution after the install.
const RUN_SECONDS: f64 = 0.9;
/// Device f32 against the f64 oracle. The discrimination check keeps this far
/// below what a gate that fired at the wrong time would cost.
const TOLERANCE: f64 = 5.0e-5;

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
    deadline: Instant,
    finished: bool,
    failed: bool,
}

/// The pump the gates switch on and off, at `depth`.
fn pump(depth: f64, frequency_hz: f64) -> TimeDrive {
    TimeDrive::ParametricPump {
        depth: ScalarField::constant(depth),
        frequency_hz: ScalarField::constant(frequency_hz),
        phase_radians: ScalarField::constant(0.2),
    }
}

/// The medium, its gates timed from `start`, the first absolute instant the
/// run steps from. `gated: false` leaves the same drives running throughout.
fn scene(start: f64, gated: bool, driven: bool) -> Scene {
    let gate = |train: PulseTrain| gated.then_some(train);
    let mut scene = Scene::initial();
    let material = &mut scene.materials[0];
    material.mass_law.drive = TimeDrive::TravellingModulation {
        depth: ScalarField::constant(0.24),
        frequency_hz: ScalarField::constant(1.6),
        phase_radians: ScalarField::constant(0.31),
        wavenumber: ScalarField::constant(2.7),
        angle_radians: ScalarField::constant(-0.4),
    };
    material.mass_law.gate = gate(PulseTrain {
        envelope: PulseEnvelope::FlatTop {
            duration: 0.3,
            edge: 0.1,
        },
        start: start - 5.3,
        repeat: 0.45,
    });
    material.stiffness_law.drive = TimeDrive::TimeCrystal {
        depth: ScalarField::constant(0.17),
        frequency_hz: ScalarField::constant(1.2),
        phase_radians: ScalarField::constant(-0.23),
        sharpness: ScalarField::constant(3.2),
    };
    material.stiffness_law.gate = gate(PulseTrain {
        envelope: PulseEnvelope::Gaussian { width: 0.06 },
        start: start + 0.05,
        repeat: 0.0,
    });
    material.magnetic_loss = Some(LossChannel {
        base_rate: ScalarField::constant(0.4),
        law: DampingLaw {
            rate: RateLaw::Constant,
            drive: pump(0.6, 0.0),
            gate: gate(PulseTrain {
                envelope: PulseEnvelope::Sinc {
                    bandwidth_hz: 3.0,
                    lobes: 3,
                },
                start: start + 0.02,
                repeat: 0.0,
            }),
        },
    });
    material.electric_loss = Some(LossChannel {
        base_rate: ScalarField::constant(0.3),
        law: DampingLaw {
            rate: RateLaw::Constant,
            drive: pump(0.5, 2.0),
            gate: gate(PulseTrain {
                envelope: PulseEnvelope::FlatTop {
                    duration: 0.4,
                    edge: 0.1,
                },
                start: start + 0.25,
                repeat: 0.0,
            }),
        },
    });
    if !driven {
        material.mass_law = CoefficientLaw::linear();
        material.stiffness_law = CoefficientLaw::linear();
        for loss in [&mut material.magnetic_loss, &mut material.electric_loss]
            .into_iter()
            .flatten()
        {
            loss.law = DampingLaw::constant();
        }
    }
    scene
}

fn operator(
    mesh: &TriMesh,
    scalar: &QuadraticWaveOperator,
    scene: &Scene,
) -> CanonicalTemporalWaveOperator {
    CanonicalTemporalWaveOperator::compile_scene(mesh, scalar, scene, 1)
        .expect("gated validation operator")
}

fn main() -> AppExit {
    let probe = scene(0.0, true, true);
    let mut fixed_scene = probe.clone();
    for material in &mut fixed_scene.materials {
        material.mass_law = CoefficientLaw::linear();
        material.stiffness_law = CoefficientLaw::linear();
        material.magnetic_loss = None;
        material.electric_loss = None;
    }
    let mesh = mesh_scene(
        &fixed_scene,
        1,
        MeshingOptions {
            target_edge_length: 0.18,
            ..MeshingOptions::default()
        },
    )
    .expect("gated validation mesh");
    let scalar = QuadraticWaveOperator::assemble_scene(
        &mesh,
        &fixed_scene,
        OuterBoundaryCondition::Reflecting,
    )
    .expect("gated validation scalar operator");
    // The gates do not move the step bound, so the probe's is the step.
    let time_step = 0.4 * operator(&mesh, &scalar, &probe).maximum_time_step();

    // Two steps short of the production rebase threshold.
    let rebase_limit = ((256.0 / time_step).ceil() as u64).min(1 << 16) as u32;
    let clock = CanonicalGpuClock {
        epoch: 7,
        epoch_origin_seconds: EPOCH_ORIGIN,
        step_in_epoch: rebase_limit - 2,
        time_step,
    };
    assert!(!clock.requires_rebase());
    let start = clock.time();
    let steps = (RUN_SECONDS / time_step).round() as u64;

    let run = |scene: &Scene| {
        let operator = operator(&mesh, &scalar, scene);
        let primary = operator
            .base()
            .primary_mass()
            .iter()
            .zip(operator.base().node_points())
            .map(|(mass, point)| {
                mass * (0.04 + 0.03 * (1.3 * point.x).sin() * (0.8 * point.y).cos())
            })
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
        let initial =
            CanonicalTemporalWaveState::new_at(&operator, time_step, primary, complementary, start)
                .expect("clock-aligned gated state");
        let mut state = initial.clone();
        for _ in 0..steps {
            state.step(&operator).expect("f64 gated oracle step");
        }
        (operator, initial, state)
    };
    let (operator, initial, oracle) = run(&scene(start, true, true));
    let (_, _, ungated) = run(&scene(start, false, true));
    let (_, _, undriven) = run(&scene(start, true, false));
    let separation = |other: &CanonicalTemporalWaveState| {
        relative_l2(
            other.primary_flux().iter().copied(),
            oracle.primary_flux().iter().copied(),
        )
    };
    let (ungated, undriven) = (separation(&ungated), separation(&undriven));
    println!(
        "gated drive gate: {steps} steps of {time_step:.3e} s from {start:.3} s; the same drives \
         ungated stand {ungated:.3e} from the oracle, the medium undriven {undriven:.3e}"
    );
    assert!(
        ungated.min(undriven) > 50.0 * TOLERANCE,
        "the fixture cannot tell a gate from no gate"
    );

    let plan = CanonicalGpuPlan::compile_temporal_bulk(&operator, &initial, clock)
        .expect("gated GPU plan");
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
        deadline: Instant::now() + Duration::from_secs(90),
        finished: false,
        failed: false,
    };

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern gated drive validation".into(),
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
    app.run()
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
        .expect("select gated validation readback mode");
    request.install(
        &mut assets,
        &mut commands,
        plans.0.take().expect("one pending gated plan"),
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
    if Instant::now() >= expected.deadline || request.stats().failure() != 0 {
        eprintln!(
            "gated GPU validation timed out or failed ({}, {})",
            request.stats().status(),
            request.stats().failure()
        );
        expected.finished = true;
        expected.failed = true;
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
    println!(
        "four gated lanes across the clock rebase, epoch {} -> {}: Q {q_error:.3e}, \
         b {b_error:.3e}, clock {clock_error:.3e}",
        expected.initial_epoch, clock.epoch
    );
    expected.finished = true;
    expected.failed = !within(q_error, TOLERANCE)
        || !within(b_error, TOLERANCE)
        || !within(clock_error, 2.0e-4 * expected.time_step.max(1.0))
        || clock.epoch == expected.initial_epoch;
}

fn relative_l2(actual: impl Iterator<Item = f64>, expected: impl Iterator<Item = f64>) -> f64 {
    let (difference, scale) = actual.zip(expected).fold((0.0, 0.0), |sum, pair| {
        (sum.0 + (pair.0 - pair.1).powi(2), sum.1 + pair.1.powi(2))
    });
    (difference / scale.max(1.0e-30)).sqrt()
}
