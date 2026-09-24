//! A field-dependent medium driven past its declared amplitude bound on the
//! device (Stage 9).
//!
//! Defocusing Kerr needs an authored bound, and a field that reaches it has no
//! constitutive inverse there. The f64 reference refuses the step that would
//! cross it; the device must refuse the same step, at the stage that saw it,
//! with the inverse-domain status - and keep the last accepted state exactly.
//! A retry from that state (the app's Run after a failure) must fail again
//! without moving a single stored bit, because the domain is a property of
//! the state and not a transient.
//!
//! `FAILURE_LAW=phi4` runs the same gate on a φ⁴ oscillator medium (Gate O):
//! a kicked domain wall whose integrated field passes the law's declared
//! bound, refused with the restoring-domain status. The retry's bit check
//! covers `r`, which is part of the accepted auxiliary lanes.

use std::time::{Duration, Instant};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::canonical_gpu::{
    CANONICAL_FAILURE_INVERSE_DOMAIN, CANONICAL_FAILURE_RESTORING_DOMAIN, CanonicalGpuClock,
    CanonicalGpuDisplay, CanonicalGpuPlan, CanonicalGpuRequest, CanonicalWaveGpuPlugin,
};
use funfern_app::wave_gpu::WaveGpuPlugin;
use funfern_core::{
    CanonicalForcing, CanonicalTemporalWaveOperator, CanonicalTemporalWaveState, CoefficientLaw,
    FieldLaw, MeshingOptions, OuterBoundaryCondition, Point2, QuadraticWaveOperator, RestoringLaw,
    ScalarField, Scene, mesh_scene,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Running,
    Retry,
    Done,
}

#[derive(Resource)]
struct Pending {
    plan: Option<CanonicalGpuPlan>,
    requested: u64,
}

#[derive(Resource)]
struct Expected {
    /// The oracle's accepted state after every step it took.
    history: Vec<(Vec<f64>, Vec<Point2>)>,
    /// The step the oracle refused.
    refused_step: u64,
    /// The status the device must refuse it with.
    status: u32,
    phase: Phase,
    failed_bits: Option<Vec<u32>>,
    failed_readbacks: u64,
    started: Instant,
    deadline: Instant,
    failed: bool,
}

fn main() -> AppExit {
    let phi4 = std::env::var("FAILURE_LAW").is_ok_and(|value| value == "phi4");
    let mut scene = Scene::default();
    if phi4 {
        scene.materials[0].restoring = RestoringLaw::Phi4 {
            lambda: ScalarField::constant(16.0),
            amplitude_bound: ScalarField::constant(1.3),
        };
    } else {
        scene.materials[0].mass_law.field = FieldLaw::Polynomial {
            chi1: ScalarField::constant(0.0),
            chi2: ScalarField::constant(-0.2),
            amplitude_bound: Some(ScalarField::constant(1.0)),
        };
    }
    let mut fixed_scene = scene.clone();
    for material in &mut fixed_scene.materials {
        material.mass_law = CoefficientLaw::linear();
        material.restoring = RestoringLaw::None;
    }
    let mesh = mesh_scene(
        &fixed_scene,
        1,
        MeshingOptions {
            target_edge_length: 0.15,
            ..MeshingOptions::default()
        },
    )
    .expect("failure mesh");
    let scalar = QuadraticWaveOperator::assemble_scene(
        &mesh,
        &fixed_scene,
        OuterBoundaryCondition::Reflecting,
    )
    .expect("failure scalar operator");
    let operator = CanonicalTemporalWaveOperator::compile_scene(&mesh, &scalar, &scene, 1)
        .expect("failure operator");
    let base = operator.base();
    let forcing = CanonicalForcing::none(base);
    let time_step = 0.4 * operator.maximum_time_step();

    // Released from rest with the field at 0.6 of its bound, the flux sheds
    // its energy into the field of a neighbouring region until one node
    // crosses 1. Search the amplitude for a crossing well into the run.
    let mut chosen = None;
    for amplitude in [2.0_f64, 3.0, 4.0, 6.0, 8.0, 12.0] {
        let potential = base
            .node_points()
            .iter()
            .map(|point| amplitude * 0.1 * (1.6 * point.x).sin() * (1.1 * point.y).cos())
            .collect::<Vec<_>>();
        let complementary = base.compatible_flux(&potential).expect("failure flux");
        let primary = vec![0.0; base.degrees_of_freedom()];
        let mut state =
            CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary)
                .expect("failure state");
        if phi4 {
            // A domain wall `tanh(x/(√2ℓ))`, kicked: the field that runs
            // into the wells overshoots them past the bound of 1.3.
            let width = 2.0_f64.sqrt() / 4.0;
            let integrated = base
                .node_points()
                .iter()
                .map(|point| (point.x / width).tanh())
                .collect::<Vec<_>>();
            let primary = base
                .node_points()
                .iter()
                .zip(base.primary_mass())
                .map(|(point, mass)| mass * amplitude * 0.5 * (1.6 * point.x).cos())
                .collect::<Vec<_>>();
            state = CanonicalTemporalWaveState::new(
                &operator,
                time_step,
                primary,
                base.compatible_flux(&integrated).expect("wall flux"),
            )
            .expect("failure state")
            .with_integrated_field(&operator, integrated)
            .expect("failure wall");
        }
        let mut oracle = state.clone();
        let mut history = Vec::new();
        for step in 0..400_u64 {
            match oracle.step_with_forcing(&operator, &forcing) {
                Ok(_) => history.push((
                    oracle.primary_flux().to_vec(),
                    oracle.complementary_flux().to_vec(),
                )),
                Err(error) => {
                    if step >= 20 {
                        chosen = Some((state.clone(), history, step, error.to_string()));
                    }
                    break;
                }
            }
        }
        if chosen.is_some() {
            break;
        }
    }
    let (state, history, refused_step, reason) =
        chosen.expect("an amplitude that crosses the bound mid-run");
    println!(
        "{} failure gate: {} Q; the oracle refuses step {} ({reason})",
        if phi4 { "φ⁴" } else { "nonlinear" },
        base.degrees_of_freedom(),
        refused_step + 1
    );

    let clock = CanonicalGpuClock::initial(time_step).expect("failure clock");
    let plan = CanonicalGpuPlan::compile_temporal(&operator, &state, &forcing, clock)
        .expect("failure GPU plan");

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern nonlinear failure validation".into(),
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
        requested: refused_step + 20,
    })
    .insert_resource(Expected {
        history,
        refused_step,
        status: if phi4 {
            CANONICAL_FAILURE_RESTORING_DOMAIN
        } else {
            CANONICAL_FAILURE_INVERSE_DOMAIN
        },
        phase: Phase::Running,
        failed_bits: None,
        failed_readbacks: 0,
        started: Instant::now(),
        deadline: Instant::now() + Duration::from_secs(120),
        failed: false,
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
    let requested = pending.requested;
    canonical.install(
        &mut assets,
        &mut commands,
        pending.plan.take().expect("one plan"),
    );
    canonical.request_steps(requested);
    commands.spawn(Camera2d);
}

fn drive(
    mut commands: Commands,
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
    if Instant::now() >= expected.deadline {
        eprintln!("nonlinear failure validation timed out");
        expected.failed = true;
        expected.phase = Phase::Done;
        return;
    }
    let failure = request.stats().failure();
    if failure == 0 {
        if request.stats().completed_steps() > expected.refused_step + 1 {
            eprintln!("the device stepped past the bound the oracle refused");
            expected.failed = true;
            expected.phase = Phase::Done;
        }
        return;
    }
    // A readback taken after the failure latched: the accepted lane.
    if display.full_readbacks <= expected.failed_readbacks
        || display.full_snapshot_completed_steps() != request.stats().completed_steps()
    {
        request.request_full_state_readback(&mut commands);
        return;
    }
    let completed = request.stats().completed_steps();
    match expected.phase {
        Phase::Running => {
            // f32 may meet the bound a step either side of f64.
            if failure != expected.status
                || completed + 1 < expected.refused_step
                || completed > expected.refused_step + 1
            {
                eprintln!(
                    "device failed with status {failure} after {completed} steps; the oracle \
                     refused step {}",
                    expected.refused_step + 1
                );
                expected.failed = true;
                expected.phase = Phase::Done;
                return;
            }
            let (primary, complementary) = &expected.history[completed as usize - 1];
            let error = relative_l2(
                display.primary_flux.iter().map(|value| f64::from(*value)),
                primary.iter().copied(),
            )
            .max(relative_l2(
                display
                    .complementary_flux
                    .iter()
                    .flat_map(|value| value.iter().map(|lane| f64::from(*lane))),
                complementary.iter().flat_map(|value| [value.x, value.y]),
            ));
            println!(
                "device refused step {} with status {failure} ({}); accepted state against the \
                 oracle's step {completed}: {error:.3e}",
                completed + 1,
                funfern_app::canonical_gpu::canonical_failure_description(failure)
            );
            if error > 3.0e-5 {
                expected.failed = true;
                expected.phase = Phase::Done;
                return;
            }
            expected.failed_bits = Some(display.accepted_storage_bits());
            expected.failed_readbacks = display.full_readbacks;
            request
                .clear_failure(&mut assets, &mut commands)
                .expect("clear the domain failure");
            request.request_steps(1);
            expected.phase = Phase::Retry;
        }
        Phase::Retry => {
            let unchanged = expected.failed_bits.as_ref() == Some(&display.accepted_storage_bits());
            println!(
                "retry from the accepted step failed again with status {failure}; stored state \
                 {} after {:.2} ms",
                if unchanged {
                    "bit-identical"
                } else {
                    "CHANGED"
                },
                expected.started.elapsed().as_secs_f64() * 1e3
            );
            expected.failed = !unchanged || failure != expected.status;
            expected.phase = Phase::Done;
        }
        Phase::Done => {}
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
