//! Real-device validation of oscillator media (Stage 11, Gate O).
//!
//! The CPU reference restores the integrated field `r = ∫u dt`: a restoring
//! force `Σ m₀V′(r)` in both kicks and `ṙ = u` in the drift. The device now
//! carries `r` as one auxiliary lane per node and does the same in f32. This
//! gate steps a medium on both and compares `Q`, `b` and `r`.
//!
//! `OSCILLATOR_MEDIUM` picks the medium:
//! - `klein-gordon` (default) and `sine-gordon`: a smooth field with `r`
//!   seeded at 1.2 in amplitude, so sine-Gordon is well away from its tangent;
//! - `kink`: a sine-Gordon kink launched at half the wave speed;
//! - `phi4`: a φ⁴ domain wall, kicked;
//! - `pumped`: sine-Gordon beside a parametric pump on the mass row;
//! - `kerr`: sine-Gordon beside a Kerr mass row;
//! - `van-der-pol`: Klein-Gordon with a van der Pol primary loss, whose
//!   stage map is the exact Bernoulli map and whose energy is active gain,
//!   compared lane for lane with the reference's.
//!
//! `OSCILLATOR_COMPOSE` adds one composition: `wall1`, `wall2`, `gap`,
//! `pins`, `source`, `loss`, or `junction`: a second material inside the
//! scene's obstacle, with its own Klein-Gordon cutoff and a constant primary
//! loss, so interface nodes sum two restoring laws and weigh an active rate
//! against a passive one. `OSCILLATOR_FILTER=1` turns the resident grid
//! filter on, against the reference's `apply_grid_filter_with_forcing` every
//! sixteenth step. `OSCILLATOR_STEPS` sets the run (default 200); as for
//! `canonical_gpu_long_run`, the Stage 0 bound is asserted up to 1000 steps
//! and past that the figures are printed, not judged. And
//! `OSCILLATOR_AMPLITUDE` scales the smooth initial field (default 1): at 0.05
//! van der Pol sits below its threshold and grows.

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
    InternalBoundary, InternalBoundaryCoupling, InternalBoundaryId, InternalBoundaryLaw, LoopRole,
    LossChannel, Material, MaterialFrame, MaterialId, MeshingOptions, OpenCubicSpline,
    OuterBoundaryCondition, Point2, QuadraticWaveOperator, RateLaw, Region, RegionId, RestoringLaw,
    ScalarField, Scene, TimeDrive, TimeSignal, mesh_scene,
};

#[derive(Resource)]
struct Pending {
    plan: Option<CanonicalGpuPlan>,
    filter: bool,
    steps: u64,
}

#[derive(Resource)]
struct Expected {
    primary: Vec<f64>,
    complementary: Vec<Point2>,
    integrated: Vec<f64>,
    /// The reference's summed active gain and primary loss lanes.
    gained: f64,
    lost: f64,
    steps: u64,
    started: Instant,
    deadline: Instant,
    finished: bool,
    failed: bool,
}

fn main() -> AppExit {
    let medium = std::env::var("OSCILLATOR_MEDIUM").unwrap_or_else(|_| "klein-gordon".into());
    let compose = std::env::var("OSCILLATOR_COMPOSE").unwrap_or_default();
    let filter = std::env::var("OSCILLATOR_FILTER").is_ok_and(|value| value == "1");
    let steps = std::env::var("OSCILLATOR_STEPS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(200);
    let amplitude = std::env::var("OSCILLATOR_AMPLITUDE")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(1.0);

    let sine_gordon = |omega0: f64| RestoringLaw::SineGordon {
        omega0: ScalarField::constant(omega0),
    };
    let mut scene = if compose == "junction" {
        let mut scene = Scene::initial();
        let interior = RegionId(2);
        scene.obstacles[0].role = LoopRole::MaterialInterface {
            exterior: BACKGROUND_REGION,
            interior,
        };
        scene.materials.push(Material {
            id: MaterialId(2),
            name: "Oscillator interior".into(),
            stiffness: ScalarField::constant(2.5),
            restoring: RestoringLaw::KleinGordon {
                omega0: ScalarField::constant(2.0),
            },
            magnetic_loss: Some(LossChannel {
                base_rate: ScalarField::constant(0.3),
                law: DampingLaw {
                    rate: RateLaw::Constant,
                    drive: TimeDrive::None,
                },
            }),
            ..Material::default_medium()
        });
        scene.regions.push(Region {
            id: interior,
            material: MaterialId(2),
            frame: MaterialFrame::world(),
        });
        scene
    } else {
        Scene::default()
    };
    let material = &mut scene.materials[0];
    material.restoring = match medium.as_str() {
        "klein-gordon" | "van-der-pol" => RestoringLaw::KleinGordon {
            omega0: ScalarField::constant(3.0),
        },
        "kink" => sine_gordon(4.0),
        "phi4" => RestoringLaw::Phi4 {
            lambda: ScalarField::constant(16.0),
            amplitude_bound: ScalarField::constant(1.6),
        },
        "sine-gordon" | "pumped" | "kerr" => sine_gordon(3.0),
        other => panic!("unknown OSCILLATOR_MEDIUM {other}"),
    };
    if medium == "pumped" {
        material.mass_law.drive = TimeDrive::ParametricPump {
            depth: ScalarField::constant(0.2),
            frequency_hz: ScalarField::constant(1.1),
            phase_radians: ScalarField::constant(0.3),
        };
    }
    if medium == "kerr" {
        material.mass_law.field = FieldLaw::Polynomial {
            chi1: ScalarField::constant(0.0),
            chi2: ScalarField::constant(0.8),
            amplitude_bound: None,
        };
    }
    let channel = |rate: f64| LossChannel {
        base_rate: ScalarField::constant(rate),
        law: DampingLaw {
            rate: RateLaw::Constant,
            drive: TimeDrive::None,
        },
    };
    if compose == "loss" {
        material.electric_loss = Some(channel(0.4));
        material.magnetic_loss = Some(channel(0.25));
    }
    if medium == "van-der-pol" {
        // Gain below `a = 0.4`, loss above it, on the primary row.
        material.magnetic_loss = Some(LossChannel {
            base_rate: ScalarField::constant(0.8),
            law: DampingLaw {
                rate: RateLaw::VanDerPol {
                    threshold: ScalarField::constant(0.4),
                    amplitude_bound: ScalarField::constant(10.0),
                },
                drive: TimeDrive::None,
            },
        });
    }
    if compose == "gap" {
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
    let wall = match compose.as_str() {
        "wall1" => OuterBoundaryCondition::FirstOrderOutgoing,
        "wall2" => OuterBoundaryCondition::SecondOrderOutgoing,
        _ => OuterBoundaryCondition::Reflecting,
    };
    let mut fixed_scene = scene.clone();
    for material in &mut fixed_scene.materials {
        material.mass_law = CoefficientLaw::linear();
        material.stiffness_law = CoefficientLaw::linear();
        material.restoring = RestoringLaw::None;
    }
    let mesh = mesh_scene(
        &fixed_scene,
        1,
        MeshingOptions {
            target_edge_length: 0.1,
            ..MeshingOptions::default()
        },
    )
    .expect("oscillator mesh");
    let scalar = QuadraticWaveOperator::assemble_scene(&mesh, &fixed_scene, wall)
        .expect("oscillator scalar operator");
    let operator = CanonicalTemporalWaveOperator::compile_scene(&mesh, &scalar, &scene, 1)
        .expect("oscillator operator");
    assert!(operator.has_restoring());
    let base = operator.base();
    let mut forcing = CanonicalForcing::none(base);
    if compose == "pins" {
        let mut prescribed = vec![None; base.degrees_of_freedom()];
        for (node, point) in base.node_points().iter().enumerate() {
            if point.x < -0.999 {
                prescribed[node] = Some(TimeSignal::harmonic(0.2, 0.15, 1.3, 0.2));
            }
        }
        forcing = CanonicalForcing::from_prescribed(base, prescribed).expect("prescribed wall");
    }
    if compose == "source" {
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
    let (primary, integrated) = match medium.as_str() {
        "kink" => {
            // `4·atan(e^{γ(x − x₀)/ℓ})` at v = 0.5, and its velocity field.
            let (length, speed, start) = (0.25, 0.5, -0.3);
            let gamma = 1.0 / (1.0_f64 - speed * speed).sqrt();
            let r = base
                .node_points()
                .iter()
                .map(|point| 4.0 * (gamma * (point.x - start) / length).exp().atan())
                .collect::<Vec<_>>();
            let q = base
                .node_points()
                .iter()
                .zip(base.primary_mass())
                .map(|(point, mass)| {
                    let s = gamma * (point.x - start) / length;
                    mass * (-speed * 2.0 * gamma / length / s.cosh())
                })
                .collect::<Vec<_>>();
            (q, r)
        }
        "phi4" => {
            let width = 2.0_f64.sqrt() / 4.0;
            let r = base
                .node_points()
                .iter()
                .map(|point| (point.x / width).tanh())
                .collect::<Vec<_>>();
            let q = base
                .node_points()
                .iter()
                .zip(base.primary_mass())
                .map(|(point, mass)| mass * 0.3 * (1.4 * point.x - 0.9 * point.y).sin())
                .collect::<Vec<_>>();
            (q, r)
        }
        _ => {
            let q = base
                .node_points()
                .iter()
                .zip(base.primary_mass())
                .map(|(point, mass)| {
                    let u = amplitude * 0.5 * (1.4 * point.x - 0.9 * point.y).sin();
                    let multiplier = if medium == "kerr" {
                        1.0 + 0.8 * u * u
                    } else {
                        1.0
                    };
                    mass * multiplier * u
                })
                .collect::<Vec<_>>();
            let r = base
                .node_points()
                .iter()
                .map(|point| amplitude * 1.2 * (0.7 * point.x + 0.5 * point.y).cos())
                .collect::<Vec<_>>();
            (q, r)
        }
    };
    // `b = ηC r`, the state the step keeps without complementary loss.
    let complementary = base
        .compatible_flux(&integrated)
        .expect("compatible oscillator flux");
    let state = CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary)
        .expect("oscillator state")
        .with_integrated_field(&operator, integrated)
        .expect("oscillator integrated field")
        .pinned(&operator, &forcing)
        .expect("pinned oscillator state");

    let mut oracle = state.clone();
    let initial = oracle.energy(&operator).expect("initial energy");
    let norms = |state: &CanonicalTemporalWaveState| {
        let norm =
            |values: &mut dyn Iterator<Item = f64>| values.map(|v| v * v).sum::<f64>().sqrt();
        (
            norm(&mut state.primary_flux().iter().copied()),
            norm(&mut state.complementary_flux().iter().flat_map(|v| [v.x, v.y])),
            norm(&mut state.integrated_field().iter().copied()),
        )
    };
    let initial_norms = norms(&oracle);
    let (mut filters, mut skipped, mut removed) = (0, 0, 0.0);
    let (mut gained, mut lost) = (0.0, 0.0);
    for step in 1..=steps {
        let accounting = oracle
            .step_with_forcing(&operator, &forcing)
            .expect("f64 oscillator step");
        gained += accounting.active_gain;
        lost += accounting.primary_loss;
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
            "oscillator gate: {filters} resident filters, {skipped} not taken, removing \
             {removed:.4e} of {initial:.4e}"
        );
    }
    // Each lane's norm against its start, so a relative error read against a
    // lane that has shrunk is not mistaken for one that has grown.
    let (q, b, r) = norms(&oracle);
    println!(
        "oscillator lane scales against the start: |Q| {:.3e}, |b| {:.3e}, |r| {:.3e}",
        q / initial_norms.0.max(1e-300),
        b / initial_norms.1.max(1e-300),
        r / initial_norms.2.max(1e-300)
    );

    let clock = CanonicalGpuClock::initial(time_step).expect("oscillator clock");
    let plan = CanonicalGpuPlan::compile_temporal(&operator, &state, &forcing, clock)
        .expect("oscillator GPU plan");
    println!(
        "oscillator gate ({medium}{}{}): {} Q, {} b, {steps} steps; energy {initial:.4e}",
        if compose.is_empty() { "" } else { " + " },
        compose,
        plan.node_count,
        plan.sample_count,
    );

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern oscillator validation".into(),
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
        steps,
    })
    .insert_resource(Expected {
        primary: oracle.primary_flux().to_vec(),
        complementary: oracle.complementary_flux().to_vec(),
        integrated: oracle.integrated_field().to_vec(),
        gained,
        lost,
        steps,
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
    canonical.request_steps(pending.steps);
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
            "oscillator validation timed out or failed: status {}",
            request.stats().failure()
        );
        expected.failed = true;
        expected.finished = true;
        return;
    }
    if request.stats().completed_steps() < expected.steps {
        return;
    }
    if !request.request_full_state_readback(&mut commands) && display.full_readbacks == 0 {
        return;
    }
    // Every lane, or a comparison runs before its readback has populated it
    // and passes by measuring nothing.
    if display.primary_flux.len() != expected.primary.len()
        || display.complementary_flux.len() != expected.complementary.len()
        || display.integrated_field().len() != expected.integrated.len()
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
    let integrated = relative_l2(
        display
            .integrated_field()
            .iter()
            .map(|value| f64::from(*value)),
        expected.integrated.iter().copied(),
    );
    println!(
        "oscillator errors after {:.2} ms: Q {primary:.3e}, b {complementary:.3e}, \
         r {integrated:.3e}",
        expected.started.elapsed().as_secs_f64() * 1_000.0
    );
    expected.finished = true;
    // The Stage 0 f32 gate, as for the field-dependent media, up to 1000
    // steps. Past that a medium's own sensitivity, and a lane that shrinks
    // while the others grow, make the relative figure a characterization.
    let judged = expected.steps <= 1000;
    if judged && (primary > 3.0e-5 || complementary > 3.0e-5 || integrated > 3.0e-5) {
        expected.failed = true;
    }
    // The gain and primary-loss lanes, each against the reference's sum. An
    // f32 lane accumulates every step's rounding, so it is held to 1e-3 of
    // the larger of the two.
    let scale = expected.gained.abs().max(expected.lost.abs()).max(1.0e-12);
    let gain = (f64::from(display.active_gain) - expected.gained).abs() / scale;
    let loss = (f64::from(display.accounting[2]) - expected.lost).abs() / scale;
    println!(
        "oscillator lanes: gain {:.6e} against {:.6e} ({gain:.2e}), primary loss {:.6e} \
         against {:.6e} ({loss:.2e})",
        display.active_gain, expected.gained, display.accounting[2], expected.lost
    );
    if judged && (gain > 1.0e-3 || loss > 1.0e-3) {
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
