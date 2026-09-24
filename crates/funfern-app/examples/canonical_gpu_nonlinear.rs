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
//!
//! `NONLINEAR_FORCED=1` composes a volume source, a harmonic prescribed wall
//! and both loss channels with the pumped medium; `NONLINEAR_GAP=1` a stiff
//! thin gap across it. Each is a stage the device writes through the maps.
//!
//! `NONLINEAR_WALL=1` or `=2` replaces the reflecting walls with a first- or
//! second-order outgoing wall, so the Kerr trace kicks through its discrete
//! gradient: a scalar Newton at an absorbing node, or a budget of Newton
//! linearizations around the linear trace solve.
//!
//! `NONLINEAR_FILTER=1` turns the resident grid filter on, so the device
//! freezes every site's tangent and filters every sixteenth step beside
//! whatever else is composed, against the reference's
//! `apply_grid_filter_with_forcing`. `NONLINEAR_LINEAR=1` drops the field
//! laws and keeps the pump, which is the driven linear filter beside the same
//! compositions. `NONLINEAR_AMPLITUDE` scales the initial field (default 1).

use std::time::{Duration, Instant};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuPlan, CanonicalGpuRequest,
    CanonicalWaveGpuPlugin,
};
use funfern_app::wave_gpu::WaveGpuPlugin;
use funfern_core::{
    BACKGROUND_REGION, CanonicalForcing, CanonicalSource, CanonicalTemporalWaveOperator,
    CanonicalTemporalWaveState, CoefficientLaw, DampingLaw, FieldLaw, GRID_SCALE_FILTER_CADENCE,
    InternalBoundary, InternalBoundaryCoupling, InternalBoundaryId, InternalBoundaryLaw,
    LossChannel, MeshingOptions, OpenCubicSpline, OuterBoundaryCondition, Point2,
    QuadraticWaveOperator, RateLaw, ScalarField, Scene, TimeDrive, TimeSignal, mesh_scene,
};

const TOTAL_STEPS: u64 = 200;

#[derive(Resource)]
struct Pending {
    plan: Option<CanonicalGpuPlan>,
    filter: bool,
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
    let flag = |name: &str| std::env::var(name).is_ok_and(|value| value == "1");
    let forced = flag("NONLINEAR_FORCED");
    let gap = flag("NONLINEAR_GAP");
    let filter = flag("NONLINEAR_FILTER");
    let linear_medium = flag("NONLINEAR_LINEAR");
    let pumped = flag("NONLINEAR_PUMPED") || forced || gap || linear_medium;
    let wall = match std::env::var("NONLINEAR_WALL").as_deref() {
        Ok("1") => OuterBoundaryCondition::FirstOrderOutgoing,
        Ok("2") => OuterBoundaryCondition::SecondOrderOutgoing,
        _ => OuterBoundaryCondition::Reflecting,
    };
    let mut scene = Scene::default();
    if forced {
        let channel = |rate: f64| LossChannel {
            base_rate: ScalarField::constant(rate),
            law: DampingLaw {
                rate: RateLaw::Constant,
                drive: TimeDrive::None,
            },
        };
        scene.materials[0].electric_loss = Some(channel(0.4));
        scene.materials[0].magnetic_loss = Some(channel(0.25));
    }
    if gap {
        scene.internal_boundaries.push(InternalBoundary {
            id: InternalBoundaryId(1),
            spline: OpenCubicSpline::uniform(vec![
                Point2::new(-0.65, 0.0),
                Point2::new(-0.2, 0.0),
                Point2::new(0.2, 0.0),
                Point2::new(0.65, 0.0),
            ])
            .expect("gap spline"),
            region: BACKGROUND_REGION,
            span_laws: vec![InternalBoundaryLaw {
                coupling: InternalBoundaryCoupling::ThinGap {
                    stiffness_ratio: 120.0,
                },
                ..InternalBoundaryLaw::REFLECTING
            }],
        });
    }
    if !linear_medium {
        scene.materials[0].mass_law.field = FieldLaw::Polynomial {
            chi1: ScalarField::constant(0.0),
            chi2: ScalarField::constant(0.8),
            amplitude_bound: None,
        };
        scene.materials[0].stiffness_law.field = FieldLaw::Saturable {
            chi: ScalarField::constant(6.0),
            saturation: ScalarField::constant(0.3),
        };
    }
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
    let scalar = QuadraticWaveOperator::assemble_scene(&mesh, &fixed_scene, wall)
        .expect("nonlinear scalar operator");
    let operator = CanonicalTemporalWaveOperator::compile_scene(&mesh, &scalar, &scene, 1)
        .expect("nonlinear operator");
    assert_eq!(operator.has_field_laws(), !linear_medium);
    let base = operator.base();
    let mut forcing = CanonicalForcing::none(base);
    if forced {
        let mut prescribed = vec![None; base.degrees_of_freedom()];
        for (node, point) in base.node_points().iter().enumerate() {
            if point.x < -0.999 {
                prescribed[node] = Some(TimeSignal::harmonic(0.2, 0.15, 1.3, 0.2));
            }
        }
        forcing = CanonicalForcing::from_prescribed(base, prescribed).expect("prescribed wall");
        forcing
            .push_source(
                CanonicalSource::direct(
                    base,
                    base.primary_mass().to_vec(),
                    TimeSignal::harmonic(0.0, 0.6, 1.4, 0.35),
                )
                .expect("volume source"),
            )
            .expect("push volume source");
    }

    let time_step = 0.4 * operator.maximum_time_step();
    let scale = std::env::var("NONLINEAR_AMPLITUDE")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(1.0);
    // A field of about 0.5 and a complementary field of about 0.3: Kerr is
    // 20% above linear at the peak, the saturable row well into its knee.
    let primary = base
        .primary_mass()
        .iter()
        .zip(base.node_points())
        .map(|(mass, point)| {
            let u = scale * 0.5 * (1.4 * point.x - 0.9 * point.y).sin();
            mass * (1.0 + 0.8 * u * u) * u
        })
        .collect::<Vec<_>>();
    let potential = base
        .node_points()
        .iter()
        .map(|point| scale * 0.3 * (0.8 * point.x + 1.2 * point.y).cos())
        .collect::<Vec<_>>();
    let complementary = base
        .compatible_flux(&potential)
        .expect("compatible nonlinear flux");
    let state = CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary)
        .expect("nonlinear state")
        .pinned(&operator, &forcing)
        .expect("pinned nonlinear state");

    let mut oracle = state.clone();
    let initial = oracle.energy(&operator).expect("initial energy");
    let (mut filters, mut skipped, mut removed) = (0, 0, 0.0);
    for step in 1..=TOTAL_STEPS {
        oracle
            .step_with_forcing(&operator, &forcing)
            .expect("f64 nonlinear step");
        if filter && step.is_multiple_of(GRID_SCALE_FILTER_CADENCE) {
            filters += 1;
            // A candidate that gains energy is not taken, on either side.
            match oracle.apply_grid_filter_with_forcing(&operator, &forcing, 1.0) {
                Ok(energy) => removed += energy,
                Err(_) => skipped += 1,
            }
        }
    }
    if filter {
        println!(
            "nonlinear gate: {filters} resident filters, {skipped} not taken, removing \
             {removed:.4e} of {initial:.4e}"
        );
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
        "{} gate{}{}{}: {} Q, {} b, {TOTAL_STEPS} steps; energy {initial:.4e}; the field sits \
         up to {:.0}% from its linear read",
        if linear_medium {
            "driven linear"
        } else {
            "nonlinear"
        },
        if pumped { " (pumped)" } else { "" },
        if forced { " + source, pins, loss" } else { "" },
        match wall {
            OuterBoundaryCondition::FirstOrderOutgoing => " + first-order wall",
            OuterBoundaryCondition::SecondOrderOutgoing => " + second-order wall",
            _ =>
                if gap {
                    " + thin gap"
                } else {
                    ""
                },
        },
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
    .insert_resource(Pending {
        plan: Some(plan),
        filter,
    })
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
    canonical.set_grid_scale_filter(pending.filter);
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
