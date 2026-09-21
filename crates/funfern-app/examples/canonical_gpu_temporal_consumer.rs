//! Real-device validation for synchronized temporal point diagnostics.
//!
//! The fixture uses the production canonical render graph and point-recorder
//! shader. It compares the final endpoint field, rate, energy and flow
//! magnitudes against the f64 temporal consumer contract.

use std::time::{Duration, Instant};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::{
    canonical_gpu::{
        CanonicalGpuClock, CanonicalGpuPlan, CanonicalGpuRequest, CanonicalWaveGpuPlugin,
    },
    wave_gpu::{ProbeDisplay, RecorderContext, RecorderHistory, WaveGpuPlugin, WaveGpuRequest},
};
use funfern_core::{
    CanonicalTemporalPointStencil, CanonicalTemporalWaveOperator, CanonicalTemporalWaveState,
    CoefficientLaw, MeshingOptions, OuterBoundaryCondition, PhysicsModel, Point2,
    QuadraticPointStencil, QuadraticWaveOperator, ScalarField, Scene, TimeDrive, mesh_scene,
};

const PROBE_ID: u64 = 71;
const SAMPLE_RATE: f64 = 480.0;

#[derive(Resource)]
struct Pending {
    plan: Option<CanonicalGpuPlan>,
    operator: CanonicalTemporalWaveOperator,
    stencil: QuadraticPointStencil,
}

#[derive(Resource)]
struct Expected {
    steps: u64,
    time: f64,
    primary: f64,
    primary_rate: f64,
    complementary_magnitude: f64,
    flow_magnitude: f64,
    energy: f64,
    started: Instant,
    deadline: Instant,
    finished: bool,
    failed: bool,
}

fn main() {
    let mut scene = Scene::initial();
    let material = &mut scene.materials[0];
    material.mass_law.drive = TimeDrive::TravellingModulation {
        depth: ScalarField::constant(0.23),
        frequency_hz: ScalarField::constant(0.78),
        phase_radians: ScalarField::constant(0.27),
        wavenumber: ScalarField::constant(2.5),
        angle_radians: ScalarField::constant(-0.36),
    };
    material.mass_law.alternate = Some(ScalarField::constant(1.42));
    material.stiffness_law.drive = TimeDrive::TimeCrystal {
        depth: ScalarField::constant(0.15),
        frequency_hz: ScalarField::constant(0.61),
        phase_radians: ScalarField::constant(-0.19),
        sharpness: ScalarField::constant(3.0),
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
            target_edge_length: 0.18,
            ..MeshingOptions::default()
        },
    )
    .expect("temporal consumer mesh");
    let scalar = QuadraticWaveOperator::assemble_scene(
        &mesh,
        &fixed_scene,
        OuterBoundaryCondition::Reflecting,
    )
    .expect("temporal consumer scalar operator");
    let triangle = mesh.triangles[0].vertices;
    let point = triangle
        .into_iter()
        .map(|vertex| mesh.vertices[vertex].point)
        .fold(Point2::default(), |sum, point| sum + point)
        / 3.0;
    let stencil = QuadraticPointStencil::build(&mesh, &scalar, &fixed_scene, point)
        .expect("interior point stencil");
    let operator = CanonicalTemporalWaveOperator::compile_scene(&mesh, &scalar, &scene, 1)
        .expect("temporal consumer operator");
    let time_step = 0.38 * operator.maximum_time_step();
    let sample_stride = (1.0 / (SAMPLE_RATE * time_step)).round().max(1.0) as u64;
    let steps = 8 * sample_stride;

    let primary = operator
        .base()
        .primary_mass()
        .iter()
        .zip(operator.base().node_points())
        .map(|(mass, point)| mass * (0.045 + 0.027 * (1.2 * point.x - 0.7 * point.y).sin()))
        .collect::<Vec<_>>();
    let potential = operator
        .base()
        .node_points()
        .iter()
        .map(|point| 0.031 * (0.6 * point.x + 0.9 * point.y).cos())
        .collect::<Vec<_>>();
    let complementary = operator
        .base()
        .compatible_flux(&potential)
        .expect("compatible temporal consumer flux");
    let mut state = CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary)
        .expect("temporal consumer state");
    let gpu_state = state.clone();
    for _ in 0..steps - 1 {
        state.step(&operator).expect("f64 temporal step");
    }
    let previous_primary = state.primary_flux().to_vec();
    state.step(&operator).expect("final f64 temporal step");
    let consumer = CanonicalTemporalPointStencil::from_quadratic(stencil, &operator)
        .expect("temporal point consumer");
    let sample = consumer
        .sample(
            &operator,
            state.primary_flux(),
            &previous_primary,
            state.complementary_flux(),
            state.time(),
            time_step,
            state.runtime(),
        )
        .expect("f64 temporal point sample");
    let expected = Expected {
        steps,
        time: state.time(),
        primary: sample.primary,
        primary_rate: sample.primary_rate,
        complementary_magnitude: sample.complementary.norm(),
        flow_magnitude: sample.energy_flow.norm(),
        energy: sample.energy_density,
        started: Instant::now(),
        deadline: Instant::now() + Duration::from_secs(60),
        finished: false,
        failed: false,
    };
    let clock = CanonicalGpuClock::initial(time_step).expect("temporal consumer clock");
    let plan = CanonicalGpuPlan::compile_temporal_bulk(&operator, &gpu_state, clock)
        .expect("temporal consumer GPU plan");

    println!(
        "temporal point-consumer GPU gate: {} Q, {} b, {} steps at stride {}",
        plan.node_count, plan.sample_count, steps, sample_stride
    );
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern temporal point-consumer validation".into(),
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
    })
    .insert_resource(expected)
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
        pending.plan.take().expect("one temporal consumer plan"),
    );
    recorders.adopt_canonical_generation(canonical.generation());
    let temporal_manifest = canonical
        .manifest()
        .and_then(|manifest| manifest.temporal)
        .expect("temporal GPU manifest");
    recorders
        .update_temporal_canonical_point_probes(
            &mut assets,
            &mut commands,
            &pending.operator,
            temporal_manifest,
            &[(PROBE_ID, Some(pending.stencil))],
            SAMPLE_RATE,
            RecorderContext {
                time_step: 0.38 * pending.operator.maximum_time_step(),
                physics: PhysicsModel::Mechanical,
                history: RecorderHistory::Restart,
            },
        )
        .expect("install temporal point recorder");
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
        eprintln!("temporal point-consumer GPU validation timed out or failed");
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
    if (sample.time - expected.time).abs() > 2.0e-4 {
        return;
    }
    let errors = [
        relative_error(sample.displacement, expected.primary),
        relative_error(sample.velocity, expected.primary_rate),
        relative_error(
            sample.transverse_magnitude,
            expected.complementary_magnitude,
        ),
        relative_error(sample.poynting_magnitude, expected.flow_magnitude),
        relative_error(sample.energy_density, expected.energy),
    ];
    println!(
        "temporal point consumer errors after {:.2} ms: u {:.3e}, rate {:.3e}, complement {:.3e}, flow {:.3e}, energy {:.3e}",
        expected.started.elapsed().as_secs_f64() * 1_000.0,
        errors[0],
        errors[1],
        errors[2],
        errors[3],
        errors[4]
    );
    expected.finished = true;
    if errors.into_iter().any(|error| error > 2.0e-4) {
        expected.failed = true;
        exit.write(AppExit::error());
    } else {
        exit.write(AppExit::Success);
    }
}

fn relative_error(actual: f64, expected: f64) -> f64 {
    (actual - expected).abs() / expected.abs().max(1.0e-8)
}
