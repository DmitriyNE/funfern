//! Real-device validation for synchronized temporal point, line, area and
//! arrow diagnostics.
//!
//! The fixture uses the production canonical render graph with the point and
//! line recorder shaders, which share one reconstruction block. It compares
//! the final endpoint field, rate, energy and flow against the f64 temporal
//! consumer contract, and the line samples against the same contract dotted
//! with each sample's normal.

use std::time::{Duration, Instant};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::{
    canonical_gpu::{
        CanonicalGpuClock, CanonicalGpuPlan, CanonicalGpuRequest, CanonicalWaveGpuPlugin,
    },
    wave_gpu::{
        AreaProbeDisplay, AreaProbeInput, CurveProbeDisplay, CurveProbeInput, ProbeDisplay,
        RecorderContext, RecorderHistory, VectorOverlayDisplay, WaveGpuPlugin, WaveGpuRequest,
    },
};
use funfern_core::{
    AreaProbeShape, BACKGROUND_REGION, CanonicalTemporalPointStencil,
    CanonicalTemporalWaveOperator, CanonicalTemporalWaveState, CoefficientLaw, MeshingOptions,
    OuterBoundaryCondition, PhysicsModel, Point2, QuadraticAreaStencil, QuadraticPointStencil,
    QuadraticWaveOperator, ScalarField, Scene, TimeDrive, mesh_scene,
    sample_temporal_canonical_area,
};

const PROBE_ID: u64 = 71;
const LINE_PROBE_ID: u64 = 72;
const AREA_PROBE_ID: u64 = 73;
const SAMPLE_RATE: f64 = 480.0;
const LINE_SAMPLE_RATE: f64 = 120.0;
const AREA_SAMPLE_RATE: f64 = 60.0;

#[derive(Resource)]
struct Pending {
    plan: Option<CanonicalGpuPlan>,
    operator: CanonicalTemporalWaveOperator,
    stencil: QuadraticPointStencil,
    line: Vec<(QuadraticPointStencil, Point2)>,
    area: QuadraticAreaStencil,
}

/// One line sample's expected values, from the same f64 contract the point
/// probe uses. Normal flow is the contract's flow dotted with the sample's
/// normal.
struct ExpectedLineSample {
    primary: f64,
    complementary_magnitude: f64,
    energy: f64,
    normal_flux: f64,
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
    line: Vec<ExpectedLineSample>,
    line_time: f64,
    area_total_energy: f64,
    area_rms_complementary: f64,
    arrows: Vec<(f64, f64)>,
    started: Instant,
    deadline: Instant,
    finished: bool,
    failed: bool,
}

fn main() -> AppExit {
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
    // A straight interior line, slanted so the travelling drive varies along
    // it, and clear of the scene's central hole.
    let ends = (Point2::new(-0.52, 0.38), Point2::new(0.54, 0.61));
    let span = ends.1 - ends.0;
    let normal = Point2::new(-span.y, span.x) / span.norm();
    let line = (0..5)
        .map(|index| {
            let position = ends.0 + span * (index as f64 / 4.0);
            let stencil = QuadraticPointStencil::build(&mesh, &scalar, &fixed_scene, position)
                .expect("interior line stencil");
            (stencil, normal)
        })
        .collect::<Vec<_>>();

    // The whole active face, so the reported energy must equal the solver's.
    let area = QuadraticAreaStencil::build(
        &mesh,
        &scalar,
        &fixed_scene,
        AreaProbeShape::Region(BACKGROUND_REGION),
    )
    .expect("region area stencil");

    let operator = CanonicalTemporalWaveOperator::compile_scene(&mesh, &scalar, &scene, 1)
        .expect("temporal consumer operator");
    let time_step = 0.38 * operator.maximum_time_step();
    let sample_stride = (1.0 / (SAMPLE_RATE * time_step)).round().max(1.0) as u64;
    let line_stride = (1.0 / (LINE_SAMPLE_RATE * time_step)).round().max(1.0) as u64;
    // Both recorders must land their last sample on the compared state.
    let area_stride = (1.0 / (AREA_SAMPLE_RATE * time_step)).round().max(1.0) as u64;
    let cadence = [line_stride, area_stride]
        .into_iter()
        .fold(sample_stride, |cadence, stride| {
            cadence / gcd(cadence, stride) * stride
        });
    let steps = (8 * sample_stride).div_ceil(cadence) * cadence;

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
    let line_expected = line
        .iter()
        .map(|(stencil, normal)| {
            let consumer = CanonicalTemporalPointStencil::from_quadratic(*stencil, &operator)
                .expect("temporal line consumer");
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
                .expect("f64 temporal line sample");
            ExpectedLineSample {
                primary: sample.primary,
                complementary_magnitude: sample.complementary.norm(),
                energy: sample.energy_density,
                normal_flux: sample.energy_flow.dot(*normal),
            }
        })
        .collect::<Vec<_>>();
    // The arrow lattice reuses the line's stencils, so the same f64 contract
    // supplies both and any disagreement is the overlay's own.
    let arrows = line
        .iter()
        .map(|(stencil, _)| {
            let consumer = CanonicalTemporalPointStencil::from_quadratic(*stencil, &operator)
                .expect("temporal arrow consumer");
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
                .expect("f64 temporal arrow sample");
            (sample.complementary.norm(), sample.energy_flow.norm())
        })
        .collect::<Vec<_>>();
    let area_sample = sample_temporal_canonical_area(
        &area,
        &operator,
        state.primary_flux(),
        state.complementary_flux(),
        state.time(),
        state.runtime(),
    )
    .expect("f64 temporal area sample");
    let expected = Expected {
        steps,
        time: state.time(),
        primary: sample.primary,
        primary_rate: sample.primary_rate,
        complementary_magnitude: sample.complementary.norm(),
        flow_magnitude: sample.energy_flow.norm(),
        energy: sample.energy_density,
        line: line_expected,
        line_time: state.time(),
        area_total_energy: area_sample.total_energy,
        area_rms_complementary: area_sample.rms_complementary,
        arrows,
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
        line,
        area,
    })
    .insert_resource(expected)
    .add_systems(Startup, install)
    .add_systems(Update, finish_when_ready);
    app.run()
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
    recorders
        .update_temporal_canonical_curve_probes(
            &mut assets,
            &mut commands,
            &pending.operator,
            temporal_manifest,
            &[CurveProbeInput {
                id: LINE_PROBE_ID,
                sample_rate: LINE_SAMPLE_RATE,
                samples: pending.line.iter().copied().map(Some).collect(),
            }],
            RecorderContext {
                time_step: 0.38 * pending.operator.maximum_time_step(),
                physics: PhysicsModel::Mechanical,
                history: RecorderHistory::Restart,
            },
        )
        .expect("install temporal line recorder");
    recorders
        .update_temporal_canonical_area_probes(
            &mut assets,
            &mut commands,
            &pending.operator,
            temporal_manifest,
            &[AreaProbeInput {
                id: AREA_PROBE_ID,
                stencil: Some(pending.area.clone()),
            }],
            AREA_SAMPLE_RATE,
            RecorderContext {
                time_step: 0.38 * pending.operator.maximum_time_step(),
                physics: PhysicsModel::Mechanical,
                history: RecorderHistory::Restart,
            },
        )
        .expect("install temporal area recorder");
    let lattice = pending
        .line
        .iter()
        .map(|(stencil, _)| *stencil)
        .collect::<Vec<_>>();
    recorders
        .update_temporal_canonical_vector_overlay(
            &mut assets,
            &mut commands,
            &pending.operator,
            temporal_manifest,
            &lattice,
        )
        .expect("install temporal vector overlay");
    canonical.request_steps(expected.steps);
    commands.spawn(Camera2d);
}

fn finish_when_ready(
    canonical: Res<CanonicalGpuRequest>,
    display: Res<ProbeDisplay>,
    curves: Res<CurveProbeDisplay>,
    areas: Res<AreaProbeDisplay>,
    arrows: Res<VectorOverlayDisplay>,
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
    let Some(line) = curves
        .records
        .iter()
        .filter(|record| record.probe_id == LINE_PROBE_ID)
        .max_by(|left, right| left.time.total_cmp(&right.time))
    else {
        return;
    };
    if (line.time - expected.line_time).abs() > 2.0e-4
        || line.displacement.len() != expected.line.len()
    {
        return;
    }
    let mut line_errors = [0.0_f64; 4];
    for (index, want) in expected.line.iter().enumerate() {
        let got = [
            f64::from(line.displacement[index]),
            f64::from(line.transverse_magnitude[index]),
            f64::from(line.energy_density[index]),
            f64::from(line.normal_flux[index]),
        ];
        let reference = [
            want.primary,
            want.complementary_magnitude,
            want.energy,
            want.normal_flux,
        ];
        for lane in 0..4 {
            line_errors[lane] = line_errors[lane].max(relative_error(got[lane], reference[lane]));
        }
    }
    let Some(area) = areas
        .records
        .iter()
        .filter(|record| record.probe_id == AREA_PROBE_ID)
        .max_by(|left, right| left.time.total_cmp(&right.time))
    else {
        return;
    };
    if (area.time - expected.line_time).abs() > 2.0e-4 {
        return;
    }
    let area_errors = [
        relative_error(area.total_energy, expected.area_total_energy),
        relative_error(
            area.rms_transverse_magnitude,
            expected.area_rms_complementary,
        ),
    ];
    println!(
        "temporal line consumer worst errors over {} samples: u {:.3e}, complement {:.3e}, energy {:.3e}, normal flow {:.3e}",
        expected.line.len(),
        line_errors[0],
        line_errors[1],
        line_errors[2],
        line_errors[3]
    );
    if arrows.samples.len() != expected.arrows.len()
        || (arrows.absolute_time - expected.line_time).abs() > 2.0e-4
    {
        return;
    }
    let mut arrow_errors = [0.0_f64; 2];
    for (sample, (complementary, flow)) in arrows.samples.iter().zip(&expected.arrows) {
        arrow_errors[0] =
            arrow_errors[0].max(relative_error(sample.complementary.norm(), *complementary));
        arrow_errors[1] = arrow_errors[1].max(relative_error(sample.energy_flow.norm(), *flow));
    }
    println!(
        "temporal area consumer over {:.0}% coverage: total energy {:.3e}, complement rms {:.3e}",
        area.coverage * 100.0,
        area_errors[0],
        area_errors[1]
    );
    println!(
        "temporal arrow consumer worst errors over {} samples: complement {:.3e}, flow {:.3e}",
        expected.arrows.len(),
        arrow_errors[0],
        arrow_errors[1]
    );
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
    if errors
        .into_iter()
        .chain(line_errors)
        .chain(area_errors)
        .chain(arrow_errors)
        .any(|error| error > 2.0e-4)
    {
        expected.failed = true;
        exit.write(AppExit::error());
    } else {
        exit.write(AppExit::Success);
    }
}

fn relative_error(actual: f64, expected: f64) -> f64 {
    (actual - expected).abs() / expected.abs().max(1.0e-8)
}

fn gcd(left: u64, right: u64) -> u64 {
    if right == 0 {
        left
    } else {
        gcd(right, left % right)
    }
}
