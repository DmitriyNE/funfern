//! Real-device validation of a field-dependent medium (Stage 9).
//!
//! The Stage 8 CPU reference inverts the assembled Kerr and saturable maps at
//! every node and quadrature sample; the device now does the same in f32,
//! with a safeguarded Newton under the Stage 8 device tolerance. This gate
//! steps a strong field - both maps tens of percent from linear - on both and
//! compares the canonical state.
//!
//! `NONLINEAR_PUMPED=1` adds a parametric pump on the Kerr row, so the
//! coefficient the inverse divides by also moves in time.

use std::time::{Duration, Instant};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuPlan, CanonicalGpuRequest,
    CanonicalWaveGpuPlugin,
};
use funfern_app::wave_gpu::WaveGpuPlugin;
use funfern_core::{
    CanonicalForcing, CanonicalTemporalWaveOperator, CanonicalTemporalWaveState, CoefficientLaw,
    FieldLaw, MeshingOptions, OuterBoundaryCondition, Point2, QuadraticWaveOperator, ScalarField,
    Scene, TimeDrive, mesh_scene,
};

const TOTAL_STEPS: u64 = 200;

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
    let pumped = std::env::var("NONLINEAR_PUMPED").is_ok_and(|value| value == "1");
    let mut scene = Scene::default();
    scene.materials[0].mass_law.field = FieldLaw::Polynomial {
        chi1: ScalarField::constant(0.0),
        chi2: ScalarField::constant(0.8),
        amplitude_bound: None,
    };
    scene.materials[0].stiffness_law.field = FieldLaw::Saturable {
        chi: ScalarField::constant(6.0),
        saturation: ScalarField::constant(0.3),
    };
    if pumped {
        scene.materials[0].mass_law.drive = TimeDrive::ParametricPump {
            depth: ScalarField::constant(0.2),
            frequency_hz: ScalarField::constant(1.1),
            phase_radians: ScalarField::constant(0.3),
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
            target_edge_length: 0.1,
            ..MeshingOptions::default()
        },
    )
    .expect("nonlinear mesh");
    let scalar = QuadraticWaveOperator::assemble_scene(
        &mesh,
        &fixed_scene,
        OuterBoundaryCondition::Reflecting,
    )
    .expect("nonlinear scalar operator");
    let operator = CanonicalTemporalWaveOperator::compile_scene(&mesh, &scalar, &scene, 1)
        .expect("nonlinear operator");
    assert!(operator.has_field_laws());
    let base = operator.base();
    let forcing = CanonicalForcing::none(base);

    let time_step = 0.4 * operator.maximum_time_step();
    // A field of about 0.5 and a complementary field of about 0.3: Kerr is
    // 20% above linear at the peak, the saturable row well into its knee.
    let primary = base
        .primary_mass()
        .iter()
        .zip(base.node_points())
        .map(|(mass, point)| {
            let u = 0.5 * (1.4 * point.x - 0.9 * point.y).sin();
            mass * (1.0 + 0.8 * u * u) * u
        })
        .collect::<Vec<_>>();
    let potential = base
        .node_points()
        .iter()
        .map(|point| 0.3 * (0.8 * point.x + 1.2 * point.y).cos())
        .collect::<Vec<_>>();
    let complementary = base
        .compatible_flux(&potential)
        .expect("compatible nonlinear flux");
    let state = CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary)
        .expect("nonlinear state");

    let mut oracle = state.clone();
    let initial = oracle.energy(&operator).expect("initial energy");
    for _ in 0..TOTAL_STEPS {
        oracle
            .step_with_forcing(&operator, &forcing)
            .expect("f64 nonlinear step");
    }
    let linear = operator
        .base()
        .primary_field(oracle.primary_flux())
        .expect("linear read");
    let nonlinear_field = operator
        .primary_field_at(oracle.primary_flux(), oracle.time(), oracle.runtime())
        .expect("nonlinear read");
    let departure = linear
        .iter()
        .zip(&nonlinear_field)
        .map(|(a, b)| (a - b).abs() / a.abs().max(1e-9))
        .fold(0.0_f64, f64::max);

    let clock = CanonicalGpuClock::initial(time_step).expect("nonlinear clock");
    let plan = CanonicalGpuPlan::compile_temporal(&operator, &state, &forcing, clock)
        .expect("nonlinear GPU plan");
    println!(
        "nonlinear gate{}: {} Q, {} b, {TOTAL_STEPS} steps; energy {initial:.4e}; the field sits \
         up to {:.0}% from its linear read",
        if pumped { " (pumped)" } else { "" },
        plan.node_count,
        plan.sample_count,
        100.0 * departure
    );

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern nonlinear validation".into(),
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
        eprintln!(
            "nonlinear validation timed out or failed: status {}",
            request.stats().failure()
        );
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
    // Both lanes, or the complementary comparison below runs before the
    // readback has populated that lane and passes by measuring nothing.
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
        "nonlinear errors after {:.2} ms: Q {primary:.3e}, b {complementary:.3e}",
        expected.started.elapsed().as_secs_f64() * 1_000.0
    );
    expected.finished = true;
    // The Stage 0 f32 gate. A stage that read a linear map would miss by the
    // departure printed above, which is tens of percent.
    if primary > 3.0e-5 || complementary > 3.0e-5 {
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
