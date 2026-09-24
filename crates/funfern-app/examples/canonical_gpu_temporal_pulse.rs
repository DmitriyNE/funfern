//! Real-device validation that a pulse lands at the mass the medium has when
//! it lands, not the mass it was authored with.
//!
//! A pulse is authored as a field increment. The canonical state is the
//! integrated nodal flux, so the increment has to be scaled by the nodal mass
//! before it can be added - and on a time-driven medium that mass is moving.
//! The host cannot do the scaling, because the event is applied at a
//! complete-step boundary it cannot see ahead to: by the time the pulse lands
//! the trajectory has moved on. So the device scales it, the same way the
//! Switch event is stamped at the boundary it takes effect on.
//!
//! The fixture is deliberately empty apart from the pulse: a zero state, no
//! source, reflecting walls. Everything the readback holds came from the pulse,
//! so an amplitude scaled by the wrong mass is a relative error of the whole
//! state rather than a correction buried in a forced solution. The gate checks
//! that the two candidate masses are far enough apart to tell apart first, so
//! it cannot pass by measuring nothing.

use std::time::{Duration, Instant};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuPlan, CanonicalGpuRequest,
    CanonicalWaveGpuPlugin,
};
use funfern_app::wave_gpu::WaveGpuPlugin;
use funfern_core::{
    CanonicalForcing, CanonicalTemporalWaveOperator, CanonicalTemporalWaveState, CoefficientLaw,
    MeshingOptions, OuterBoundaryCondition, Point2, QuadraticWaveOperator, ScalarField, Scene,
    TimeDrive, mesh_scene,
};

const TOTAL_STEPS: u64 = 32;

/// Device f32 against an f64 oracle over the steps below. The discrimination
/// check keeps this far under the error a wrongly scaled pulse would make.
const TOLERANCE: f64 = 2.0e-4;

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

fn main() -> AppExit {
    // A deep pump, so the instantaneous mass is a long way from the authored
    // one at the instant the pulse lands.
    let mut scene = Scene::initial();
    scene.materials[0].mass_law.drive = TimeDrive::ParametricPump {
        depth: ScalarField::constant(0.35),
        frequency_hz: ScalarField::constant(0.85),
        phase_radians: ScalarField::constant(1.1),
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
            target_edge_length: 0.2,
            ..MeshingOptions::default()
        },
    )
    .expect("temporal pulse mesh");
    let scalar = QuadraticWaveOperator::assemble_scene(
        &mesh,
        &fixed_scene,
        OuterBoundaryCondition::Reflecting,
    )
    .expect("temporal pulse scalar operator");
    let operator = CanonicalTemporalWaveOperator::compile_scene(&mesh, &scalar, &scene, 1)
        .expect("temporal pulse operator");
    let base = operator.base();

    let time_step = 0.4 * operator.maximum_time_step();
    let forcing = CanonicalForcing::none(base);
    let state = CanonicalTemporalWaveState::zero(&operator, time_step).expect("zero driven state");

    let pulse = base
        .node_points()
        .iter()
        .map(|point| 0.05 * (1.3 * point.x - 0.7 * point.y).cos())
        .collect::<Vec<_>>();

    // The gate is only meaningful if the two masses differ by much more than
    // the tolerance. Check that before trusting a pass.
    let instantaneous = operator
        .primary_mass_at(state.time(), state.runtime())
        .expect("instantaneous nodal mass");
    let authored = base.primary_mass();
    let separation = relative_l2(instantaneous.iter().copied(), authored.iter().copied());
    println!(
        "temporal pulse gate: the instantaneous nodal mass stands {separation:.3e} from the \
         authored one at the boundary the pulse lands on"
    );
    assert!(
        separation > 50.0 * TOLERANCE,
        "the fixture cannot tell the two masses apart: {separation:.3e}"
    );

    let mut oracle = state.clone();
    oracle
        .apply_primary_pulse(&operator, &forcing, &pulse)
        .expect("f64 pulse oracle");
    for _ in 0..TOTAL_STEPS {
        oracle
            .step_with_forcing(&operator, &forcing)
            .expect("f64 driven step");
    }
    assert!(
        oracle
            .primary_flux()
            .iter()
            .any(|value| value.abs() > 1.0e-6),
        "the pulse left nothing to compare"
    );
    // Both lanes have to carry something, or the comparison below passes by
    // measuring nothing.
    assert!(
        oracle
            .complementary_flux()
            .iter()
            .any(|value| value.x.abs() > 1.0e-6 || value.y.abs() > 1.0e-6),
        "the pulse never reached the complementary lane"
    );

    let clock = CanonicalGpuClock::initial(time_step).expect("temporal pulse clock");
    let mut plan = CanonicalGpuPlan::compile_temporal(&operator, &state, &forcing, clock)
        .expect("temporal pulse GPU plan");
    plan.stage_primary_pulse(&pulse, 1)
        .expect("staged device pulse");

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern temporal pulse validation".into(),
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
    .run()
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
        eprintln!("temporal pulse validation timed out or failed");
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
        "temporal pulse errors after {:.2} ms: Q {primary:.3e}, b {complementary:.3e}",
        expected.started.elapsed().as_secs_f64() * 1_000.0
    );
    expected.finished = true;
    if primary > TOLERANCE || complementary > TOLERANCE {
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
