//! Real-device regression: a probe sample that lands on a resident grid-filter
//! commit must still report a rate.
//!
//! The filter flips the accepted state lane without advancing physical time,
//! so at that boundary the other lane holds the pre-filter value at the same
//! instant. A recorder that differences the lanes there reports a filter
//! correction divided by `dt`. Measured before the fix: between 0.03% and 18%
//! of the true rate, always a collapse toward zero.
//!
//! The fixture is deliberately the worst case rather than a rare one. The
//! sample rate and timestep give a stride of exactly the filter cadence, so
//! before the fix every single sample was corrupted. That is the `0.0625x`
//! speed setting, which `paced_time_step` reaches by making the step the
//! speed over 120.

use std::time::{Duration, Instant};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::{
    canonical_gpu::{
        CanonicalGpuClock, CanonicalGpuPlan, CanonicalGpuRequest, CanonicalWaveGpuPlugin,
    },
    wave_gpu::{ProbeDisplay, RecorderContext, RecorderHistory, WaveGpuPlugin, WaveGpuRequest},
};
use funfern_core::{
    CanonicalForcing, CanonicalWaveOperator, CanonicalWaveState, GRID_SCALE_FILTER_CADENCE,
    MeshingOptions, OuterBoundaryCondition, PhysicsModel, Point2, QuadraticPointStencil,
    QuadraticWaveOperator, Scene, mesh_scene,
};

const PROBE_ID: u64 = 91;
const SAMPLE_RATE: f64 = 120.0;
/// Four commits, so the comparison is not the first one out of the gate.
const BOUNDARIES: u64 = 4;

#[derive(Resource)]
struct Pending {
    plan: Option<CanonicalGpuPlan>,
    operator: CanonicalWaveOperator,
    stencil: QuadraticPointStencil,
    time_step: f64,
}

#[derive(Resource)]
struct Expected {
    steps: u64,
    time: f64,
    primary: f64,
    primary_rate: f64,
    collapsed_rate: f64,
    started: Instant,
    deadline: Instant,
    finished: bool,
    failed: bool,
}

fn main() {
    let scene = Scene::initial();
    let mesh = mesh_scene(
        &scene,
        1,
        MeshingOptions {
            target_edge_length: 0.18,
            ..MeshingOptions::default()
        },
    )
    .expect("filter-boundary mesh");
    let scalar =
        QuadraticWaveOperator::assemble_scene(&mesh, &scene, OuterBoundaryCondition::Reflecting)
            .expect("filter-boundary scalar operator");
    let operator = CanonicalWaveOperator::compile_scene(&mesh, &scalar, &scene, 1)
        .expect("canonical operator");

    // Stride exactly the cadence: every sample would land on a commit.
    let time_step = 1.0 / (SAMPLE_RATE * GRID_SCALE_FILTER_CADENCE as f64);
    assert!(
        time_step <= operator.maximum_time_step(),
        "the fixture step {time_step:.3e} must be stable, bound is {:.3e}",
        operator.maximum_time_step()
    );
    let stride = (1.0 / (SAMPLE_RATE * time_step)).round().max(1.0) as u64;
    assert_eq!(
        stride, GRID_SCALE_FILTER_CADENCE,
        "the fixture must collide"
    );

    let triangle = mesh.triangles[0].vertices;
    let point = triangle
        .into_iter()
        .map(|vertex| mesh.vertices[vertex].point)
        .fold(Point2::default(), |sum, point| sum + point)
        / 3.0;
    let stencil =
        QuadraticPointStencil::build(&mesh, &scalar, &scene, point).expect("interior stencil");
    let consumer = funfern_core::CanonicalPointStencil::from_quadratic(stencil, &operator)
        .expect("point consumer");

    let primary = operator
        .node_points()
        .iter()
        .map(|point| 0.05 + 0.03 * (4.0 * point.x - 2.5 * point.y).sin())
        .collect::<Vec<_>>();
    let potential = operator
        .node_points()
        .iter()
        .map(|point| 0.04 * (3.0 * point.x + 2.0 * point.y).cos())
        .collect::<Vec<_>>();
    let mut state =
        CanonicalWaveState::from_primary_and_potential(&operator, time_step, &primary, &potential)
            .expect("filter-boundary state");
    let forcing = CanonicalForcing::none(&operator);
    let gpu_state = state.clone();

    // The deferred sample lands one step past the last commit.
    let steps = BOUNDARIES * GRID_SCALE_FILTER_CADENCE + 1;
    for step in 1..=BOUNDARIES * GRID_SCALE_FILTER_CADENCE {
        state.step(&operator).expect("f64 step");
        if step.is_multiple_of(GRID_SCALE_FILTER_CADENCE) {
            state
                .apply_grid_filter(&operator, &forcing, 1.0)
                .expect("f64 resident filter");
        }
    }
    let previous_endpoint = state.primary_flux().to_vec();
    state.step(&operator).expect("final f64 step");

    let field = |flux: &[f64]| {
        let mut value = 0.0;
        for local in 0..consumer.nodes.len() {
            value += consumer.primary_weights[local]
                * flux[consumer.nodes[local] as usize]
                * consumer.primary_inverse_mass[local];
        }
        value
    };
    let current = field(state.primary_flux());
    let primary_rate = (current - field(&previous_endpoint)) / time_step;

    // For the record: differencing at the commit instead, which is what the
    // recorder did before it learned to step past one.
    let mut at_commit = gpu_state.clone();
    for step in 1..=BOUNDARIES * GRID_SCALE_FILTER_CADENCE - 1 {
        at_commit.step(&operator).expect("f64 step");
        if step.is_multiple_of(GRID_SCALE_FILTER_CADENCE) {
            at_commit
                .apply_grid_filter(&operator, &forcing, 1.0)
                .expect("f64 resident filter");
        }
    }
    at_commit.step(&operator).expect("f64 step");
    let before_filter = field(at_commit.primary_flux());
    at_commit
        .apply_grid_filter(&operator, &forcing, 1.0)
        .expect("f64 resident filter");
    let collapsed_rate = (field(at_commit.primary_flux()) - before_filter) / time_step;

    let clock = CanonicalGpuClock::initial(time_step).expect("clock");
    let plan = CanonicalGpuPlan::compile(&operator, &gpu_state, &forcing, clock).expect("GPU plan");
    let node_count = plan.node_count;
    println!(
        "filter-boundary gate: {node_count} Q, stride {stride} against cadence {GRID_SCALE_FILTER_CADENCE}, {steps} steps"
    );
    println!(
        "true rate {primary_rate:.6e}, what differencing at the commit gives {collapsed_rate:.6e}"
    );

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern filter-boundary validation".into(),
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
        operator,
        stencil,
        time_step,
    })
    .insert_resource(Expected {
        steps,
        time: state.time(),
        primary: current,
        primary_rate,
        collapsed_rate,
        started: Instant::now(),
        deadline: Instant::now() + Duration::from_secs(60),
        finished: false,
        failed: false,
    })
    .add_systems(Startup, install)
    .add_systems(Update, finish_when_ready);
    app.run();
}

fn install(
    mut commands: Commands,
    mut pending: ResMut<Pending>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut canonical: ResMut<CanonicalGpuRequest>,
    mut recorders: ResMut<WaveGpuRequest>,
    expected: Res<Expected>,
) {
    canonical.install(
        &mut assets,
        &mut commands,
        pending.plan.take().expect("one plan"),
    );
    canonical.set_grid_scale_filter(true);
    recorders.adopt_canonical_generation(canonical.generation());
    recorders
        .update_canonical_point_probes(
            &mut assets,
            &mut commands,
            &pending.operator,
            &[(PROBE_ID, Some(pending.stencil))],
            SAMPLE_RATE,
            RecorderContext {
                time_step: pending.time_step,
                physics: PhysicsModel::Mechanical,
                history: RecorderHistory::Restart,
            },
        )
        .expect("install point recorder");
    canonical.request_steps(expected.steps);
    commands.spawn(Camera2d);
}

fn finish_when_ready(
    canonical: Res<CanonicalGpuRequest>,
    display: Res<ProbeDisplay>,
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
    if Instant::now() >= expected.deadline || canonical.stats().failure() != 0 {
        eprintln!("filter-boundary validation timed out or failed");
        expected.finished = true;
        expected.failed = true;
        exit.write(AppExit::error());
        return;
    }
    if canonical.stats().completed_steps() < expected.steps {
        return;
    }
    let Some(sample) = display
        .records
        .iter()
        .filter(|sample| sample.probe_id == PROBE_ID)
        .max_by(|left, right| left.time.total_cmp(&right.time))
        .copied()
    else {
        return;
    };
    // The deferred sample carries the later time, which is the point. A
    // sample still sitting on the commit is the defect itself, so say so
    // rather than spin until the deadline.
    if (sample.time - expected.time).abs() > 2.0e-4 {
        eprintln!(
            "the last sample is at {:.6e}, not the deferred {:.6e}: it landed on the filter commit",
            sample.time, expected.time
        );
        expected.finished = true;
        expected.failed = true;
        exit.write(AppExit::error());
        return;
    }
    let field = relative_error(sample.displacement, expected.primary);
    let rate = relative_error(sample.velocity, expected.primary_rate);
    println!(
        "filter-boundary errors after {:.2} ms: u {field:.3e}, rate {rate:.3e} (a collapsed rate would read {:.3e})",
        expected.started.elapsed().as_secs_f64() * 1_000.0,
        expected.collapsed_rate
    );
    expected.finished = true;
    if field > 2.0e-4 || rate > 2.0e-4 {
        expected.failed = true;
        exit.write(AppExit::error());
    } else {
        exit.write(AppExit::Success);
    }
}

fn relative_error(actual: f64, expected: f64) -> f64 {
    (actual - expected).abs() / expected.abs().max(1.0e-8)
}
