//! Full render-graph validation/timing harness for the production canonical GPU
//! solver. This runs the same plugin, buffers, shader and atomic commit path as
//! the application; it is not an isolated kernel benchmark.
//!
//! Stepping is unfenced by default, so the timing is what the steps cost the
//! device. `--fenced` restores the interactive lead fence, under which the
//! figure is the readback round trip instead: 64 steps per one or two frames.
//! The clock stops when a readback reports the last step, one to three frames
//! after the device finished it, so the default 128 steps validate but do not
//! time: read throughput from `--steps=3000` or more.

use std::time::{Duration, Instant};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuPlan, CanonicalGpuRequest,
    CanonicalWaveGpuPlugin,
};
use funfern_app::wave_gpu::{VectorOverlayDisplay, WaveGpuPlugin, WaveGpuRequest};
use funfern_core::{
    BACKGROUND_REGION, CanonicalAuxiliaryState, CanonicalForcing, CanonicalPointStencil,
    CanonicalSource, CanonicalWaveOperator, CanonicalWaveState, DampingLaw,
    GRID_SCALE_FILTER_CADENCE, InternalBoundary, InternalBoundaryCoupling, InternalBoundaryId,
    InternalBoundaryLaw, LossChannel, MeshingOptions, Obstacle, ObstacleId, OpenCubicSpline,
    OuterBoundaryCondition, PeriodicCubicSpline, Point2, QuadraticPointStencil,
    QuadraticWaveOperator, RateLaw, ScalarField, Scene, TimeDrive, TimeSignal, mesh_scene,
};

const DEFAULT_STEPS: u64 = 128;
const DEFAULT_WARMUP_STEPS: u64 = 16;

#[derive(Resource)]
struct PendingPlan(Option<CanonicalGpuPlan>);

#[derive(Resource)]
struct Expected {
    primary: Vec<f64>,
    complementary: Vec<[f64; 2]>,
    auxiliary: Vec<f64>,
    primary_inverse_mass: Vec<f64>,
    complementary_energy: Vec<[f64; 4]>,
    auxiliary_energy_weight: Vec<f64>,
    initial_energy: f64,
    time_step: f64,
    steps: u64,
    warmup_steps: u64,
    event_serial: u32,
    initial_step: u64,
    failure_test: bool,
    primary_readback: bool,
    fenced: bool,
    periodic_filter: bool,
    overlay_operator: CanonicalWaveOperator,
    overlay_stencils: Vec<QuadraticPointStencil>,
    overlay_expected: Vec<(Point2, Point2)>,
    failure_snapshot: Option<FailureSnapshot>,
    recovering: bool,
    started: Option<Instant>,
    settle_after: Option<u64>,
    completed_elapsed: Option<Duration>,
    finished: bool,
    failed: bool,
    deadline: Instant,
}

struct FailureSnapshot {
    state: Vec<u32>,
    accounting: [u32; 8],
    readbacks: u64,
}

fn main() -> AppExit {
    let second_order = std::env::args().any(|argument| argument == "--second-order");
    let first_order = std::env::args().any(|argument| argument == "--first-order");
    let loss = std::env::args().any(|argument| argument == "--loss");
    let thin_gap = std::env::args().any(|argument| argument == "--thin-gap");
    let obstacles = std::env::args().any(|argument| argument == "--obstacles");
    let driven = std::env::args().any(|argument| argument == "--forcing");
    let steps = std::env::args()
        .find_map(|argument| argument.strip_prefix("--steps=")?.parse::<u64>().ok())
        .unwrap_or(DEFAULT_STEPS);
    let warmup_steps = std::env::args()
        .find_map(|argument| argument.strip_prefix("--warmup=")?.parse::<u64>().ok())
        .unwrap_or(DEFAULT_WARMUP_STEPS);
    let edge = std::env::args()
        .find_map(|argument| argument.strip_prefix("--edge=")?.parse::<f64>().ok())
        .unwrap_or(0.08);
    let event = if std::env::args().any(|argument| argument == "--pulse") {
        "pulse"
    } else if std::env::args().any(|argument| argument == "--filter") {
        "filter"
    } else if std::env::args().any(|argument| argument == "--maintenance") {
        "maintenance"
    } else if std::env::args().any(|argument| argument == "--law-patch") {
        "law-patch"
    } else {
        "none"
    };
    let failure_test = std::env::args().any(|argument| argument == "--failure");
    let primary_readback = std::env::args().any(|argument| argument == "--primary-readback");
    let fenced = std::env::args().any(|argument| argument == "--fenced");
    let periodic_filter = std::env::args().any(|argument| argument == "--periodic-filter");
    let vector_overlay_samples = std::env::args()
        .find_map(|argument| {
            argument
                .strip_prefix("--vector-overlay-samples=")?
                .parse::<usize>()
                .ok()
        })
        .or_else(|| {
            std::env::args()
                .any(|argument| argument == "--vector-overlay")
                .then_some(3)
        })
        .unwrap_or(0);
    assert!(vector_overlay_samples <= 16_384);
    let clock_rebase = std::env::args().any(|argument| argument == "--clock-rebase");
    let preparation = Instant::now();
    let mut scene = if thin_gap || obstacles {
        Scene::default()
    } else {
        Scene::initial()
    };
    if obstacles {
        scene.obstacles = [
            [-0.55, -0.48],
            [-0.15, -0.55],
            [0.30, -0.45],
            [0.58, -0.10],
            [0.25, 0.05],
            [-0.25, -0.02],
            [-0.52, 0.38],
            [0.12, 0.48],
        ]
        .into_iter()
        .enumerate()
        .map(|(index, center)| {
            Obstacle::hole(
                ObstacleId(index as u64 + 1),
                PeriodicCubicSpline::rounded(Point2::new(center[0], center[1]), 0.105),
            )
        })
        .collect();
    }
    if thin_gap {
        scene.internal_boundaries.push(InternalBoundary {
            id: InternalBoundaryId(1),
            spline: OpenCubicSpline::uniform(vec![
                Point2::new(-0.65, 0.0),
                Point2::new(-0.2, 0.0),
                Point2::new(0.2, 0.0),
                Point2::new(0.65, 0.0),
            ])
            .expect("thin-gap spline"),
            region: BACKGROUND_REGION,
            span_laws: vec![InternalBoundaryLaw {
                coupling: InternalBoundaryCoupling::ThinGap {
                    stiffness_ratio: 25.0,
                },
                ..InternalBoundaryLaw::REFLECTING
            }],
        });
    }
    if loss {
        scene.materials[0].electric_loss = Some(LossChannel {
            base_rate: ScalarField::constant(0.08),
            law: DampingLaw {
                rate: RateLaw::Constant,
                drive: TimeDrive::None,
            },
        });
        scene.materials[0].magnetic_loss = Some(LossChannel {
            base_rate: ScalarField::constant(0.05),
            law: DampingLaw {
                rate: RateLaw::Constant,
                drive: TimeDrive::None,
            },
        });
    }
    let mesh_started = Instant::now();
    let mesh = mesh_scene(
        &scene,
        1,
        MeshingOptions {
            target_edge_length: edge,
            max_vertices: 100_000,
            max_triangles: 200_000,
            max_refinement_steps: 100_000,
            ..MeshingOptions::default()
        },
    )
    .expect("standard timing mesh");
    let mesh_elapsed = mesh_started.elapsed();
    let boundary = if second_order {
        OuterBoundaryCondition::SecondOrderOutgoing
    } else if first_order {
        OuterBoundaryCondition::FirstOrderOutgoing
    } else {
        OuterBoundaryCondition::Reflecting
    };
    let mut scalar_scene = scene.clone();
    scalar_scene.materials[0].electric_loss = None;
    scalar_scene.materials[0].magnetic_loss = None;
    let scalar_started = Instant::now();
    let scalar = QuadraticWaveOperator::assemble_scene(&mesh, &scalar_scene, boundary)
        .expect("quadratic comparison operator");
    let scalar_elapsed = scalar_started.elapsed();
    let canonical_started = Instant::now();
    let operator = CanonicalWaveOperator::compile_scene(&mesh, &scalar, &scene, 1)
        .expect("canonical operator");
    let canonical_elapsed = canonical_started.elapsed();
    let overlay_stencils = if vector_overlay_samples > 0 {
        (0..vector_overlay_samples)
            .map(|sample| {
                let element = (sample * mesh.triangles.len() / vector_overlay_samples)
                    .min(mesh.triangles.len() - 1);
                let triangle = &mesh.triangles[element];
                let [a, b, c] = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
                let point = (a + b + c) / 3.0;
                QuadraticPointStencil::build(&mesh, &scalar, &scalar_scene, point)
                    .expect("vector-overlay stencil")
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let time_step = 0.9 * operator.maximum_time_step();
    let primary = operator
        .node_points()
        .iter()
        .map(|point| (1.1 * point.x).sin() * (0.7 * point.y).cos())
        .collect::<Vec<_>>();
    let potential = operator
        .node_points()
        .iter()
        .map(|point| 0.08 * (0.8 * point.x - 0.3 * point.y).cos())
        .collect::<Vec<_>>();
    let initial =
        CanonicalWaveState::from_primary_and_potential(&operator, time_step, &primary, &potential)
            .expect("compatible initial state");
    let initial_energy = initial.energy(&operator).expect("initial energy");
    let mut forcing = if driven {
        let mut prescribed = vec![None; operator.degrees_of_freedom()];
        prescribed[0] = Some(TimeSignal::harmonic(0.01, 0.02, 0.7, 0.3));
        CanonicalForcing::from_prescribed(&operator, prescribed)
            .expect("prescribed-primary forcing")
    } else {
        CanonicalForcing::none(&operator)
    };
    if driven {
        let weights = operator
            .primary_mass()
            .iter()
            .zip(operator.node_points())
            .map(|(mass, point)| 0.015 * mass * (0.6 * point.x - 0.2 * point.y).cos())
            .collect::<Vec<_>>();
        forcing
            .push_source(
                CanonicalSource::direct(
                    &operator,
                    weights.clone(),
                    TimeSignal::harmonic(0.03, 0.05, 1.1, -0.2),
                )
                .expect("direct source"),
            )
            .expect("direct source layout");
        forcing
            .push_source(
                CanonicalSource::legacy(
                    &operator,
                    weights,
                    TimeSignal::harmonic(0.02, 0.04, 0.9, 0.4),
                    0.0,
                )
                .expect("legacy source"),
            )
            .expect("legacy source layout");
    }
    let clock = if clock_rebase {
        CanonicalGpuClock {
            epoch: 7,
            epoch_origin_seconds: 1_000.0,
            step_in_epoch: (1 << 16) - 1,
            time_step,
        }
    } else {
        CanonicalGpuClock::initial(time_step).unwrap()
    };
    let initial_step = clock.step_in_epoch as u64;
    let gpu_plan_started = Instant::now();
    let mut plan =
        CanonicalGpuPlan::compile_with_quadratic(&operator, &scalar, &initial, &forcing, clock)
            .expect("canonical GPU plan");
    let gpu_plan_elapsed = gpu_plan_started.elapsed();
    let mut oracle = initial;
    let event_serial = u32::from(event != "none");
    match event {
        "pulse" => {
            let increment = operator
                .node_points()
                .iter()
                .map(|point| 0.002 * (0.9 * point.x + 0.4 * point.y).sin())
                .collect::<Vec<_>>();
            plan.stage_primary_pulse(&increment, event_serial)
                .expect("GPU pulse event");
            oracle
                .apply_primary_pulse(&operator, &forcing, &increment)
                .expect("CPU pulse event");
        }
        "filter" => {
            plan.stage_grid_filter(0.6, event_serial)
                .expect("GPU filter event");
            oracle
                .apply_grid_filter(&operator, &forcing, 0.6)
                .expect("CPU filter event");
        }
        "maintenance" => {
            let mut correction = vec![0.0; operator.degrees_of_freedom()];
            correction[0] = 2.0e-7;
            correction[1] = -2.0e-7;
            plan.stage_maintenance(&correction, event_serial)
                .expect("GPU maintenance event");
            let field_increment = correction
                .iter()
                .zip(operator.primary_mass())
                .map(|(value, mass)| value / mass)
                .collect::<Vec<_>>();
            oracle
                .apply_primary_pulse(&operator, &forcing, &field_increment)
                .expect("CPU maintenance reference");
        }
        "law-patch" => {
            plan.stage_linear_loss_patch(
                operator.primary_loss_rate(),
                operator.complementary_loss_rate(),
                event_serial,
            )
            .expect("GPU linear-law patch");
        }
        _ => {}
    }
    let manifest = plan.manifest.clone();
    let preparation_time = preparation.elapsed();
    let measured_steps = if failure_test { 1 } else { steps };
    for _ in 0..measured_steps + warmup_steps {
        oracle
            .step_with_forcing(&operator, &forcing)
            .expect("CPU oracle step");
        if periodic_filter
            && (initial_step + oracle.steps()).is_multiple_of(GRID_SCALE_FILTER_CADENCE)
        {
            oracle
                .apply_grid_filter(&operator, &forcing, 1.0)
                .expect("CPU periodic grid filter");
        }
    }
    let zero_rate = vec![0.0; operator.degrees_of_freedom()];
    let overlay_expected = overlay_stencils
        .iter()
        .map(|stencil| {
            let sample = CanonicalPointStencil::from_quadratic(*stencil, &operator)
                .and_then(|stencil| {
                    stencil.sample(
                        oracle.primary_flux(),
                        &zero_rate,
                        oracle.complementary_flux(),
                    )
                })
                .expect("CPU vector-overlay sample");
            (sample.complementary, sample.energy_flow)
        })
        .collect();
    let expected = Expected {
        primary: oracle.primary_flux().to_vec(),
        complementary: oracle
            .complementary_flux()
            .iter()
            .map(|value| [value.x, value.y])
            .collect(),
        auxiliary: match oracle.auxiliaries() {
            CanonicalAuxiliaryState::None => Vec::new(),
            CanonicalAuxiliaryState::Linear(value) => value
                .thin_gap_jump()
                .iter()
                .chain(value.outgoing_z())
                .copied()
                .collect(),
            _ => panic!("unsupported canonical auxiliary variant"),
        },
        primary_inverse_mass: operator
            .primary_mass()
            .iter()
            .map(|mass| mass.recip())
            .collect(),
        auxiliary_energy_weight: operator
            .thin_gap_samples()
            .iter()
            .map(|sample| sample.stiffness)
            .chain(std::iter::repeat_n(
                1.0,
                operator
                    .outgoing_boundary()
                    .map_or(0, |boundary| boundary.auxiliary_count()),
            ))
            .collect(),
        complementary_energy: operator
            .constitutive_samples()
            .iter()
            .map(|sample| {
                [
                    sample.complementary_inverse.xx,
                    sample.complementary_inverse.xy,
                    sample.complementary_inverse.yy,
                    sample.integration_weight,
                ]
            })
            .collect(),
        initial_energy,
        time_step,
        steps: measured_steps,
        warmup_steps,
        event_serial,
        initial_step,
        failure_test,
        primary_readback,
        fenced,
        periodic_filter,
        overlay_operator: operator,
        overlay_stencils,
        overlay_expected,
        failure_snapshot: None,
        recovering: false,
        started: None,
        settle_after: None,
        completed_elapsed: None,
        finished: false,
        failed: false,
        deadline: Instant::now() + Duration::from_secs(90),
    };
    println!(
        "prepared {} Q, {} b, {} auxiliary values in {:.2} ms; {:.2} MiB steady (state {:.2}, nodes {:.2}, samples {:.2}, tables {:.2}, scratch {:.2}, boundary {:.2}), {:.2} MiB accepted physical state, {} dispatches/step, {} event dispatches ({event})",
        plan.node_count,
        plan.sample_count,
        plan.auxiliary_count,
        preparation_time.as_secs_f64() * 1_000.0,
        manifest.bytes.steady_bytes() as f64 / (1024.0 * 1024.0),
        manifest.bytes.state as f64 / (1024.0 * 1024.0),
        manifest.bytes.nodes as f64 / (1024.0 * 1024.0),
        manifest.bytes.samples as f64 / (1024.0 * 1024.0),
        manifest.bytes.tables as f64 / (1024.0 * 1024.0),
        manifest.bytes.scratch as f64 / (1024.0 * 1024.0),
        manifest.bytes.boundary as f64 / (1024.0 * 1024.0),
        manifest.bytes.accepted_state_bytes(&plan) as f64 / (1024.0 * 1024.0),
        manifest.dispatches_per_step,
        manifest.event_dispatches,
    );
    println!(
        "CPU preparation: mesh {:.2} ms, scalar assembly {:.2}, canonical assembly {:.2}, GPU plan {:.2}",
        mesh_elapsed.as_secs_f64() * 1_000.0,
        scalar_elapsed.as_secs_f64() * 1_000.0,
        canonical_elapsed.as_secs_f64() * 1_000.0,
        gpu_plan_elapsed.as_secs_f64() * 1_000.0,
    );

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern canonical GPU validation".into(),
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
    mut recorders: ResMut<WaveGpuRequest>,
    expected: Res<Expected>,
) {
    if expected.finished {
        return;
    }
    if !expected.primary_readback {
        request
            .set_continuous_full_state_readback(true)
            .expect("select validation readback mode");
    }
    request.set_grid_scale_filter(expected.periodic_filter);
    request.set_unfenced_stepping(!expected.fenced);
    request.install(
        &mut assets,
        &mut commands,
        plans.0.take().expect("one pending plan"),
    );
    recorders.adopt_canonical_generation(request.generation());
    recorders
        .update_canonical_vector_overlay(
            &mut assets,
            &mut commands,
            &expected.overlay_operator,
            &expected.overlay_stencils,
        )
        .expect("install vector-overlay sampler");
    request.request_steps(expected.warmup_steps);
    commands.spawn(Camera2d);
}

fn finish_when_ready(
    mut commands: Commands,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut request: ResMut<CanonicalGpuRequest>,
    display: Res<CanonicalGpuDisplay>,
    vector_display: Res<VectorOverlayDisplay>,
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
    if Instant::now() >= expected.deadline {
        eprintln!(
            "canonical GPU validation timed out ({})",
            request.stats().status()
        );
        expected.finished = true;
        expected.failed = true;
        exit.write(AppExit::error());
        return;
    }
    if request.stats().failure() != 0 && !expected.failure_test {
        eprintln!("canonical GPU failure {}", request.stats().failure());
        expected.finished = true;
        expected.failed = true;
        exit.write(AppExit::error());
        return;
    }
    let Some(clock) = display.clock else { return };
    if expected.failure_test && request.stats().failure() != 0 && !expected.recovering {
        let snapshot = expected
            .failure_snapshot
            .as_ref()
            .expect("failure injection follows a snapshot");
        if display.readbacks < snapshot.readbacks.saturating_add(2) {
            return;
        }
        let accounting = display.accounting.map(f32::to_bits);
        if display.accepted_storage_bits() != snapshot.state
            || accounting != snapshot.accounting
            || clock.accepted_steps != (expected.initial_step + expected.warmup_steps) as u32
        {
            eprintln!("failed GPU transaction changed accepted state, accounting, or clock");
            exit.write(AppExit::error());
            expected.finished = true;
            expected.failed = true;
            return;
        }
        request
            .clear_failure(&mut assets, &mut commands)
            .expect("clear injected GPU failure");
        request.request_steps(1);
        expected.started = Some(Instant::now());
        expected.recovering = true;
        return;
    }
    if expected.failure_test && expected.failure_snapshot.is_some() && !expected.recovering {
        return;
    }
    if expected.started.is_none() {
        if clock.accepted_steps >= (expected.initial_step + expected.warmup_steps) as u32
            && clock.event_serial == expected.event_serial
        {
            if expected.failure_test {
                expected.failure_snapshot = Some(FailureSnapshot {
                    state: display.accepted_storage_bits(),
                    accounting: display.accounting.map(f32::to_bits),
                    readbacks: display.readbacks,
                });
                request
                    .inject_failure(
                        &mut assets,
                        &mut commands,
                        funfern_app::canonical_gpu::CANONICAL_FAILURE_NON_FINITE,
                        0,
                    )
                    .expect("inject GPU failure");
                request.request_steps(1);
            } else {
                request.request_steps(expected.steps);
                // The request is visible to extraction at the next complete frame.
                expected.started = Some(Instant::now());
            }
        }
        return;
    }
    if clock.accepted_steps
        < (expected.initial_step + expected.steps + expected.warmup_steps) as u32
        || display.primary_flux.len() != expected.primary.len()
    {
        return;
    }
    if expected.completed_elapsed.is_none() {
        expected.completed_elapsed = Some(expected.started.unwrap().elapsed());
    }
    if display.complementary_flux.len() != expected.complementary.len() {
        request.request_full_state_readback(&mut commands);
        return;
    }
    if expected.settle_after.is_none() {
        expected.settle_after = Some(display.readbacks.saturating_add(2));
    }
    let settle_after = expected.settle_after.unwrap();
    if display.readbacks < settle_after {
        return;
    }
    if !expected.overlay_expected.is_empty()
        && (vector_display.generation != request.generation()
            || vector_display.samples.len() != expected.overlay_expected.len()
            || vector_display.completed_steps
                < expected.initial_step + expected.steps + expected.warmup_steps)
    {
        return;
    }
    expected.finished = true;
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
    let auxiliary_error = relative_l2(
        display.auxiliary.iter().map(|value| *value as f64),
        expected.auxiliary.iter().copied(),
    );
    let auxiliary_absolute = rms_difference(
        display.auxiliary.iter().map(|value| *value as f64),
        expected.auxiliary.iter().copied(),
    );
    let elapsed = expected.completed_elapsed.unwrap().as_secs_f64();
    let final_energy = gpu_energy(&display, &expected);
    let accounting = display.accounting.map(|value| value as f64);
    let external = accounting[0] + accounting[1] + accounting[6] + accounting[7];
    let dissipated = accounting[2] + accounting[3] + accounting[4] + accounting[5];
    let energy_residual = (final_energy - expected.initial_energy - external + dissipated).abs()
        / expected.initial_energy.abs().max(1.0);
    let overlay_error = relative_l2(
        vector_display.samples.iter().flat_map(|sample| {
            [
                sample.complementary.x,
                sample.complementary.y,
                sample.energy_flow.x,
                sample.energy_flow.y,
            ]
        }),
        expected
            .overlay_expected
            .iter()
            .flat_map(|(complementary, flow)| [complementary.x, complementary.y, flow.x, flow.y]),
    );
    println!(
        "{} accepted steps in {:.2} ms ({:.3} ms/step, {}): {:.2} simulated seconds/wall second; Q error {:.3e}, b error {:.3e}, auxiliary error {:.3e} relative/{:.3e} RMS, energy residual {:.3e}, vector-overlay error {:.3e}; {} dispatches",
        expected.steps,
        elapsed * 1_000.0,
        elapsed * 1_000.0 / expected.steps as f64,
        if expected.fenced {
            "fenced"
        } else {
            "unfenced"
        },
        expected.time_step * expected.steps as f64 / elapsed,
        q_error,
        b_error,
        auxiliary_error,
        auxiliary_absolute,
        energy_residual,
        overlay_error,
        request.stats().dispatches(),
    );
    if q_error > 3.0e-5
        || b_error > 3.0e-5
        || auxiliary_absolute > 2.0e-5
        || energy_residual > 2.0e-4
        || overlay_error > 3.0e-5
    {
        eprintln!("canonical GPU accuracy or energy gate was exceeded");
        expected.failed = true;
        exit.write(AppExit::error());
    } else {
        exit.write(AppExit::Success);
    }
}

fn gpu_energy(display: &CanonicalGpuDisplay, expected: &Expected) -> f64 {
    let primary = display
        .primary_flux
        .iter()
        .zip(&expected.primary_inverse_mass)
        .map(|(value, inverse_mass)| 0.5 * (*value as f64).powi(2) * inverse_mass)
        .sum::<f64>();
    let complementary = display
        .complementary_flux
        .iter()
        .zip(&expected.complementary_energy)
        .map(|(value, tensor)| {
            let x = value[0] as f64;
            let y = value[1] as f64;
            0.5 * tensor[3] * (tensor[0] * x * x + 2.0 * tensor[1] * x * y + tensor[2] * y * y)
        })
        .sum::<f64>();
    let auxiliary = 0.5
        * display
            .auxiliary
            .iter()
            .zip(&expected.auxiliary_energy_weight)
            .map(|(value, weight)| weight * (*value as f64).powi(2))
            .sum::<f64>();
    primary + complementary + auxiliary
}

fn relative_l2(actual: impl Iterator<Item = f64>, expected: impl Iterator<Item = f64>) -> f64 {
    let (difference, scale) = actual.zip(expected).fold((0.0, 0.0), |sum, pair| {
        (sum.0 + (pair.0 - pair.1).powi(2), sum.1 + pair.1.powi(2))
    });
    (difference / scale.max(1.0e-30)).sqrt()
}

fn rms_difference(actual: impl Iterator<Item = f64>, expected: impl Iterator<Item = f64>) -> f64 {
    let (sum, count) = actual
        .zip(expected)
        .fold((0.0, 0_usize), |(sum, count), (actual, expected)| {
            (sum + (actual - expected).powi(2), count + 1)
        });
    (sum / count.max(1) as f64).sqrt()
}
