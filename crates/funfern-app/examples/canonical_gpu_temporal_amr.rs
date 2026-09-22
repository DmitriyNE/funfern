//! Real-device validation of the error estimate on a modulated medium.
//!
//! The estimate's gradient terms now read the solver's own complementary flux
//! rather than differentiating the reconstructed scalar field. That change was
//! measured in f64 on the CPU, and it moved which quantity the estimator
//! differentiates - so it has to be re-measured against the f32 state the
//! device actually produces. `canonical_primary_rate` already records why:
//! the nodal force carries f32 cancellation and must not be spatially
//! differentiated for AMR recovery. The new terms come from stored
//! complementary state instead, which should be better behaved, but that is an
//! argument until a device says so.
//!
//! The instantaneous coefficients the estimate needs come from the accepted
//! material runtime decoded out of the same buffer copy as the state, not from
//! a runtime reconstructed from the host clock. That is the whole reason the
//! runtime bank travels in the state snapshot.
//!
//! The fixture drives a travelling modulation on the mass row, which is the
//! medium that used to take the efficiency index from 1.4 to 17.4 before the
//! estimate moved onto the flux.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuHandoffOutcome, CanonicalGpuPlan,
    CanonicalGpuRequest, CanonicalGpuRuntimeTransfer, CanonicalGpuTransferPlan,
    CanonicalWaveGpuPlugin,
};
use funfern_app::wave_gpu::WaveGpuPlugin;
use funfern_core::{
    CanonicalForcing, CanonicalIndicatorSnapshot, CanonicalMaterialRuntimeState,
    CanonicalOutgoingHistoryTransferMap, CanonicalPrimaryTransferMap,
    CanonicalTemporalWaveOperator, CanonicalTemporalWaveState, CanonicalThinGapHistoryTransferMap,
    CanonicalVectorTransferMap, CoefficientLaw, MeshAdaptationJob, MeshAdaptationOptions,
    MeshAdaptationState, MeshSizeField, MeshingOptions, OuterBoundaryCondition, Point2,
    QuadraticSolutionSnapshot, QuadraticTransferMap, QuadraticWaveOperator, ScalarField, Scene,
    SolutionIndicatorJob, SolutionIndicatorOptions, SolutionIndicatorResult, TimeDrive, TriMesh,
    canonical_temporal_indicator_supplement, mesh_scene,
};

const TOTAL_STEPS: u64 = 48;
const TARGET_EDGE: f64 = 0.2;
/// The estimator and the adapter have to agree on the size bounds. A target
/// the adapter would refuse is a defect rather than a tuning choice, so both
/// read these.
const MINIMUM_EDGE: f64 = 0.04;
const MAXIMUM_EDGE: f64 = 0.3;
/// Steps taken on the refined generation before it is estimated.
///
/// An estimate wants an accepted state, and at the handoff boundary itself the
/// second state lane still belongs to the mesh that was left behind. That is
/// not a defect in the transfer: the rate is only meaningful once the new
/// generation has taken its own steps, which is also when production would
/// estimate.
const SETTLE_STEPS: u64 = 8;

/// Everything the gate compares, from one estimate.
#[derive(Clone)]
struct Estimate {
    total_energy: f64,
    recovery: f64,
    interior_jump: f64,
    cell_residual: f64,
    boundary_residual: f64,
    complementary_jump: f64,
    global_indicator: f64,
    element_indicators: Vec<f64>,
}

/// One mesh generation and everything needed to estimate on it.
struct Generation {
    mesh: Arc<TriMesh>,
    quadratic: Arc<QuadraticWaveOperator>,
    operator: CanonicalTemporalWaveOperator,
    oracle: Estimate,
    /// The host's own copy of the accepted runtime, so a disagreement can be
    /// attributed to the coefficients as well as to the state.
    runtime: CanonicalMaterialRuntimeState,
    /// The host's own copy of the accepted state, so a disagreement can be
    /// attributed to the state or to the estimate built on it.
    primary: Vec<f64>,
    previous_primary: Vec<f64>,
    complementary: Vec<Point2>,
    previous_complementary: Vec<Point2>,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Phase {
    Evolve,
    Handoff,
    Transferred,
    Settled,
    Done,
}

#[derive(Resource)]
struct Pending {
    plan: Option<CanonicalGpuPlan>,
    target: Option<CanonicalGpuPlan>,
    transfer: Option<CanonicalGpuTransferPlan>,
}

#[derive(Resource)]
struct Expected {
    source: Generation,
    target: Generation,
    /// The maps the GPU transfer plan was compiled from, kept so the host can
    /// apply the same transfer to the same input and attribute any difference
    /// to the transfer itself rather than to the state it started from.
    primary_map: CanonicalPrimaryTransferMap,
    vector_map: CanonicalVectorTransferMap,
    /// The device's own accepted state just before the handoff.
    departing: Option<(Vec<f64>, Vec<Point2>)>,
    readbacks_at_handoff: u64,
    scene: Scene,
    time: f64,
    target_time: f64,
    time_step: f64,
    phase: Phase,
    started: Instant,
    deadline: Instant,
    failed: bool,
}

fn main() {
    let mut scene = Scene::initial();
    scene.materials[0].mass_law.drive = TimeDrive::TravellingModulation {
        depth: ScalarField::constant(0.22),
        frequency_hz: ScalarField::constant(0.9),
        phase_radians: ScalarField::constant(0.15),
        wavenumber: ScalarField::constant(3.0),
        angle_radians: ScalarField::constant(0.3),
    };

    // The scalar operator and the mesh cannot carry a law, so they come from a
    // stripped copy. The estimator gets the authored scene, which is what
    // production holds.
    let mut fixed_scene = scene.clone();
    for material in &mut fixed_scene.materials {
        material.mass_law = CoefficientLaw::linear();
        material.stiffness_law = CoefficientLaw::linear();
    }
    let mesh = Arc::new(
        mesh_scene(
            &fixed_scene,
            1,
            MeshingOptions {
                target_edge_length: TARGET_EDGE,
                ..MeshingOptions::default()
            },
        )
        .expect("temporal AMR mesh"),
    );
    let quadratic = Arc::new(
        QuadraticWaveOperator::assemble_scene(
            &mesh,
            &fixed_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .expect("temporal AMR scalar operator"),
    );
    let operator = CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1)
        .expect("temporal AMR operator");
    let time_step = 0.4 * operator.maximum_time_step();

    let primary = operator
        .base()
        .primary_mass()
        .iter()
        .zip(operator.base().node_points())
        .map(|(mass, point)| mass * (0.05 + 0.03 * (1.4 * point.x - 0.8 * point.y).sin()))
        .collect::<Vec<_>>();
    let potential = operator
        .base()
        .node_points()
        .iter()
        .map(|point| 0.035 * (0.7 * point.x + 1.1 * point.y).cos())
        .collect::<Vec<_>>();
    let complementary = operator
        .base()
        .compatible_flux(&potential)
        .expect("compatible temporal AMR flux");
    let state = CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary)
        .expect("temporal AMR state");
    let gpu_state = state.clone();

    // The f64 oracle runs the same trajectory and is estimated the same way.
    let mut oracle_state = state;
    let mut previous_primary = oracle_state.primary_flux().to_vec();
    let mut previous_complementary = oracle_state.complementary_flux().to_vec();
    for _ in 0..TOTAL_STEPS {
        previous_primary = oracle_state.primary_flux().to_vec();
        previous_complementary = oracle_state.complementary_flux().to_vec();
        oracle_state.step(&operator).expect("f64 oracle step");
    }
    let oracle_result = estimate(
        &mesh,
        &quadratic,
        &scene,
        &operator,
        oracle_state.primary_flux(),
        &previous_primary,
        oracle_state.complementary_flux(),
        &previous_complementary,
        oracle_state.time(),
        time_step,
        oracle_state.runtime(),
    )
    .expect("f64 oracle estimate");
    let oracle = summarize(&oracle_result);
    assert!(
        oracle.complementary_jump > 0.0,
        "the estimate must actually be on the flux, or the gate proves nothing"
    );
    assert!(
        oracle.global_indicator > 0.0,
        "the fixture must carry a nonzero estimate"
    );

    // The refined generation the estimate itself asks for. Building it from
    // the estimator's own size field is what makes this an adaptive run
    // rather than a transfer to an arbitrary mesh.
    let size_field: Arc<dyn MeshSizeField> = oracle_result.field.clone();
    let mut adaptation = MeshAdaptationJob::new(
        mesh.clone(),
        fixed_scene.clone(),
        MeshAdaptationState::from_mesh(&mesh),
        mesh.mesh_revision + 1,
        size_field,
        MeshAdaptationOptions {
            meshing: MeshingOptions {
                target_edge_length: TARGET_EDGE,
                ..MeshingOptions::default()
            },
            minimum_target_edge_length: MINIMUM_EDGE,
            maximum_target_edge_length: MAXIMUM_EDGE,
            ..MeshAdaptationOptions::default()
        },
    );
    let adapted = loop {
        if let Some(result) = adaptation.advance(8_192) {
            break result.expect("temporal AMR adaptation");
        }
    };
    let target_mesh = Arc::new(adapted.mesh);
    assert!(
        target_mesh.triangles.len() != mesh.triangles.len(),
        "the estimate has to actually move the mesh, or the transfer proves nothing"
    );
    let target_quadratic = Arc::new(
        QuadraticWaveOperator::assemble_scene(
            &target_mesh,
            &fixed_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .expect("target scalar operator"),
    );
    let target_operator =
        CanonicalTemporalWaveOperator::compile_scene(&target_mesh, &target_quadratic, &scene, 1)
            .expect("target temporal operator");

    // The host applies the same maps the device will, so the new generation
    // gets an oracle of its own rather than being compared against the old
    // mesh's answer.
    let interpolation =
        QuadraticTransferMap::build(&mesh, &quadratic, &target_mesh, &target_quadratic)
            .expect("temporal AMR interpolation");
    let primary_map = CanonicalPrimaryTransferMap::prepare(
        &interpolation,
        operator.base(),
        target_operator.base(),
    )
    .expect("temporal AMR primary map");
    let vector_map = CanonicalVectorTransferMap::prepare(
        &mesh,
        operator.base(),
        &target_mesh,
        target_operator.base(),
    )
    .expect("temporal AMR vector map");
    let gap_map = CanonicalThinGapHistoryTransferMap::prepare(
        operator.base().thin_gap_samples(),
        target_operator.base().thin_gap_samples(),
    )
    .expect("temporal AMR gap map");
    let outgoing_map = CanonicalOutgoingHistoryTransferMap::prepare(
        &interpolation,
        operator.base(),
        target_operator.base(),
    )
    .expect("temporal AMR outgoing map");
    let prescribed = vec![false; primary_map.target_node_count()];
    let components = primary_map.target_component_count();
    let labels = operator.base().component_labels().to_vec();
    // Interpolating a field onto another mesh must not create or destroy any
    // of it, so each isolated component's total is carried across. This is the
    // transfer the device performs; asking for the free one instead moves the
    // state by 2e-4 and was the first version of this oracle's mistake.
    let move_primary = |flux: &[f64]| {
        let mut totals = vec![0.0; components];
        for (value, label) in flux.iter().zip(&labels) {
            totals[*label as usize] += value;
        }
        let totals = totals.into_iter().map(Some).collect::<Vec<_>>();
        primary_map
            .transfer(flux, &totals, &prescribed)
            .expect("temporal AMR primary transfer")
            .0
    };
    let move_complementary = |flux: &[Point2]| {
        vector_map
            .transfer(flux)
            .expect("temporal AMR vector transfer")
            .0
    };
    let target_primary = move_primary(oracle_state.primary_flux());
    let target_complementary = move_complementary(oracle_state.complementary_flux());
    // The runtime is keyed by material, not by mesh, and both generations
    // compile the same materials, so the accepted one carries over as is.
    let mut target_state = CanonicalTemporalWaveState::new_at(
        &target_operator,
        time_step,
        target_primary.clone(),
        target_complementary.clone(),
        oracle_state.time(),
    )
    .expect("target temporal oracle state");
    *target_state.runtime_mut() = oracle_state.runtime().clone();
    let mut target_previous_primary = target_state.primary_flux().to_vec();
    let mut target_previous_complementary = target_state.complementary_flux().to_vec();
    for _ in 0..SETTLE_STEPS {
        target_previous_primary = target_state.primary_flux().to_vec();
        target_previous_complementary = target_state.complementary_flux().to_vec();
        target_state
            .step(&target_operator)
            .expect("target f64 step");
    }
    let target_time = target_state.time();
    let target_oracle = summarize(
        &estimate(
            &target_mesh,
            &target_quadratic,
            &scene,
            &target_operator,
            target_state.primary_flux(),
            &target_previous_primary,
            target_state.complementary_flux(),
            &target_previous_complementary,
            target_time,
            time_step,
            target_state.runtime(),
        )
        .expect("f64 target estimate"),
    );

    let clock = CanonicalGpuClock::initial(time_step).expect("temporal AMR clock");
    let plan = CanonicalGpuPlan::compile_temporal_bulk(&operator, &gpu_state, clock)
        .expect("temporal AMR GPU plan");
    // Only a layout template for the target plan; the device fills it from
    // the transfer, and the host's own copy of that result is the oracle.
    let target_placeholder = CanonicalTemporalWaveState::zero(&target_operator, time_step)
        .expect("target placeholder state");
    let target_plan =
        CanonicalGpuPlan::compile_temporal_bulk(&target_operator, &target_placeholder, clock)
            .expect("target temporal GPU plan");
    let source_forcing = CanonicalForcing::none(operator.base());
    let target_forcing = CanonicalForcing::none(target_operator.base());
    // A genuine refinement changes the node layout, so runtime ownership has
    // to be rebuilt from the primary transfer rather than carried across.
    let runtime_transfer = CanonicalGpuRuntimeTransfer::from_primary_transfer(
        operator.base(),
        target_operator.base(),
        &source_forcing,
        &target_forcing,
        &primary_map,
        [0; 4],
    )
    .expect("temporal AMR runtime ownership");
    let transfer = CanonicalGpuTransferPlan::compile(
        operator.base(),
        target_operator.base(),
        &source_forcing,
        &target_forcing,
        &primary_map,
        &vector_map,
        &gap_map,
        &outgoing_map,
        &runtime_transfer,
    )
    .expect("temporal AMR geometry transfer")
    .with_temporal_material_runtime(&plan, &target_plan)
    .expect("temporal AMR material runtime transfer");

    println!(
        "temporal AMR gate: {} Q, {} b, {} elements refined to {}",
        plan.node_count,
        plan.sample_count,
        mesh.triangles.len(),
        target_mesh.triangles.len()
    );
    println!(
        "  oracle indicator {:.6e} before refinement, {:.6e} after",
        oracle.global_indicator, target_oracle.global_indicator
    );

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern temporal AMR validation".into(),
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
        target: Some(target_plan),
        transfer: Some(transfer),
    })
    .insert_resource(Expected {
        primary_map,
        vector_map,
        departing: None,
        readbacks_at_handoff: 0,
        source: Generation {
            mesh,
            quadratic,
            operator,
            oracle,
            runtime: oracle_state.runtime().clone(),
            primary: oracle_state.primary_flux().to_vec(),
            previous_primary,
            complementary: oracle_state.complementary_flux().to_vec(),
            previous_complementary,
        },
        target: Generation {
            mesh: target_mesh,
            quadratic: target_quadratic,
            operator: target_operator,
            oracle: target_oracle,
            runtime: target_state.runtime().clone(),
            primary: target_state.primary_flux().to_vec(),
            previous_primary: target_previous_primary,
            complementary: target_state.complementary_flux().to_vec(),
            previous_complementary: target_previous_complementary,
        },
        scene,
        time: oracle_state.time(),
        target_time,
        time_step,
        phase: Phase::Evolve,
        started: Instant::now(),
        deadline: Instant::now() + Duration::from_secs(180),
        failed: false,
    })
    .add_systems(Startup, install)
    .add_systems(Update, drive)
    .run();
}

/// One estimate, built exactly the way the application will build it: the
/// supplement and the scalar job over the same accepted state, with the
/// instantaneous coefficients coming from the supplied runtime.
#[allow(clippy::too_many_arguments)]
fn estimate(
    mesh: &Arc<TriMesh>,
    quadratic: &Arc<QuadraticWaveOperator>,
    scene: &Scene,
    operator: &CanonicalTemporalWaveOperator,
    primary_flux: &[f64],
    previous_primary_flux: &[f64],
    complementary_flux: &[Point2],
    previous_complementary_flux: &[Point2],
    time: f64,
    time_step: f64,
    runtime: &CanonicalMaterialRuntimeState,
) -> Option<SolutionIndicatorResult> {
    let snapshot = CanonicalIndicatorSnapshot {
        mesh_revision: mesh.mesh_revision,
        primary_flux: primary_flux.to_vec(),
        previous_primary_flux: previous_primary_flux.to_vec(),
        complementary_flux: complementary_flux.to_vec(),
        previous_complementary_flux: previous_complementary_flux.to_vec(),
        auxiliary: vec![],
        previous_auxiliary: vec![],
        time,
        time_step,
    };
    let supplement =
        canonical_temporal_indicator_supplement(mesh, operator, &snapshot, runtime, 0.0).ok()?;
    let current = operator
        .primary_field_at(primary_flux, time, runtime)
        .ok()?;
    let earlier = operator
        .primary_field_at(previous_primary_flux, time - time_step, runtime)
        .ok()?;
    let velocity = current
        .iter()
        .zip(&earlier)
        .map(|(now, before)| (now - before) / time_step)
        .collect::<Vec<_>>();
    let count = quadratic.degrees_of_freedom();
    let scalar_snapshot = QuadraticSolutionSnapshot {
        mesh_revision: mesh.mesh_revision,
        displacement: current,
        velocity,
        acceleration: vec![0.0; count],
        auxiliary: vec![0.0; count],
        volume_acceleration: vec![0.0; count],
        time,
        time_step,
    };
    let demand = operator.resolution_demand(0.0);
    let mut job = SolutionIndicatorJob::new(
        mesh.clone(),
        quadratic.clone(),
        scene.clone(),
        scalar_snapshot,
        SolutionIndicatorOptions {
            minimum_edge_length: MINIMUM_EDGE,
            maximum_edge_length: MAXIMUM_EDGE,
            resolved_frequency_hz: demand.frequency_hz,
            coefficient_wavelength: demand.coefficient_wavelength,
            ..SolutionIndicatorOptions::default()
        },
    )
    .with_canonical_supplement(supplement)
    .with_instantaneous_materials(runtime.clone());
    loop {
        if let Some(result) = job.advance(8_192) {
            return result.ok();
        }
    }
}

fn summarize(result: &SolutionIndicatorResult) -> Estimate {
    Estimate {
        total_energy: result.report.total_energy,
        recovery: result.report.recovery_contribution,
        interior_jump: result.report.interior_jump_contribution,
        cell_residual: result.report.cell_residual_contribution,
        boundary_residual: result.report.boundary_residual_contribution,
        complementary_jump: result.report.complementary_jump_contribution,
        global_indicator: result.report.global_indicator,
        element_indicators: result.element_indicators.clone(),
    }
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
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut pending: ResMut<Pending>,
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
    if Instant::now() >= expected.deadline || request.stats().failure() != 0 {
        eprintln!("temporal AMR validation timed out or failed");
        expected.failed = true;
        expected.phase = Phase::Done;
        return;
    }

    match expected.phase {
        Phase::Evolve => {
            if request.stats().completed_steps() < TOTAL_STEPS {
                return;
            }
            if !request.request_full_state_readback(&mut commands) && display.full_readbacks == 0 {
                return;
            }
            let Some((device, _)) = read_estimate(&display, &expected, false) else {
                return;
            };
            let oracle = expected.source.oracle.clone();
            if !compare("before refinement", &device, &oracle, &expected, false) {
                expected.failed = true;
                expected.phase = Phase::Done;
                return;
            }
            // Keep what the device is about to hand over, so the transfer
            // can be measured against its own input.
            expected.departing = Some((
                display
                    .primary_flux
                    .iter()
                    .map(|value| f64::from(*value))
                    .collect(),
                display
                    .complementary_flux
                    .iter()
                    .map(|value| Point2::new(f64::from(value[0]), f64::from(value[1])))
                    .collect(),
            ));
            expected.readbacks_at_handoff = display.full_readbacks;
            request
                .begin_handoff(
                    &mut assets,
                    &mut commands,
                    pending.target.take().expect("target plan"),
                    pending.transfer.take().expect("transfer plan"),
                )
                .expect("begin temporal AMR handoff");
            expected.phase = Phase::Handoff;
        }
        Phase::Handoff => match request.handoff_outcome() {
            CanonicalGpuHandoffOutcome::Accepted => {
                expected.phase = Phase::Transferred;
            }
            CanonicalGpuHandoffOutcome::Rejected(reason) => {
                eprintln!("temporal AMR handoff rejected with {reason}");
                expected.failed = true;
                expected.phase = Phase::Done;
            }
            _ => {}
        },
        // The transfer on its own, with no stepping either side of it: the
        // host applies the same maps to the same input the device just
        // consumed, so whatever is left is the transfer's own arithmetic.
        Phase::Transferred => {
            if !request.request_full_state_readback(&mut commands)
                && display.full_readbacks <= expected.readbacks_at_handoff
            {
                return;
            }
            if display.full_readbacks <= expected.readbacks_at_handoff {
                return;
            }
            let (departing_primary, departing_complementary) =
                expected.departing.take().expect("a departing state");
            // The transfer can be asked to preserve each isolated component's
            // total, which is the physical statement that interpolating a
            // field onto another mesh must not create or destroy any of it.
            // Asking and not asking are different transfers, and which one the
            // device performs is the question.
            let mut departing_totals = vec![0.0; expected.primary_map.target_component_count()];
            for (value, label) in departing_primary
                .iter()
                .zip(expected.source.operator.base().component_labels())
            {
                departing_totals[*label as usize] += value;
            }
            let prescribed = vec![false; expected.primary_map.target_node_count()];
            let host_primary = |conserving: bool| {
                let totals = departing_totals
                    .iter()
                    .map(|total| conserving.then_some(*total))
                    .collect::<Vec<_>>();
                expected
                    .primary_map
                    .transfer(&departing_primary, &totals, &prescribed)
                    .expect("host primary transfer")
                    .0
            };
            let host_complementary = expected
                .vector_map
                .transfer(&departing_complementary)
                .expect("host vector transfer")
                .0;
            let device_primary = display
                .primary_flux
                .iter()
                .map(|value| f64::from(*value))
                .collect::<Vec<_>>();
            let device_complementary = display
                .complementary_flux
                .iter()
                .map(|value| Point2::new(f64::from(value[0]), f64::from(value[1])))
                .collect::<Vec<_>>();
            if device_primary.len() != expected.primary_map.target_node_count() {
                return;
            }
            let free = relative_l2(&device_primary, &host_primary(false));
            let conserving = relative_l2(&device_primary, &host_primary(true));
            let complementary = relative_l2_flux(&device_complementary, &host_complementary);
            println!(
                "  transfer alone, same input both sides: Q {free:.3e} free, {conserving:.3e} conserving, b {complementary:.3e}"
            );
            // Which of the two transfers the device performs, stated as a
            // check rather than left to a comment. The free one misses by
            // `2e-4`; the conserving one lands at f32 level, and the
            // complementary transfer is exact.
            if conserving > 1.0e-6 || complementary > 1.0e-6 || free <= conserving {
                eprintln!("the device is not performing the conserving primary transfer");
                expected.failed = true;
                expected.phase = Phase::Done;
                return;
            }
            let component_total = |flux: &[f64], labels: &[u32]| {
                let mut totals = vec![0.0; expected.primary_map.target_component_count()];
                for (value, label) in flux.iter().zip(labels) {
                    totals[*label as usize] += value;
                }
                totals
            };
            println!(
                "  component totals: departing {:?}, device {:?}",
                departing_totals
                    .iter()
                    .map(|total| format!("{total:.6e}"))
                    .collect::<Vec<_>>(),
                component_total(
                    &device_primary,
                    expected.target.operator.base().component_labels()
                )
                .iter()
                .map(|total| format!("{total:.6e}"))
                .collect::<Vec<_>>(),
            );
            request.request_steps(SETTLE_STEPS);
            expected.phase = Phase::Settled;
        }
        Phase::Settled => {
            if display
                .clock
                .is_none_or(|clock| (clock.accepted_steps as u64) < SETTLE_STEPS)
            {
                return;
            }
            if !request.request_full_state_readback(&mut commands) {
                return;
            }
            let Some((device, lanes)) = read_estimate(&display, &expected, true) else {
                return;
            };
            let oracle = expected.target.oracle.clone();
            let ok = compare("after refinement", &device, &oracle, &expected, true);
            // The transferred state itself, which is what the rate-sensitive
            // terms are actually reporting on.
            let carried = lanes <= 1.0e-3;
            if !carried {
                eprintln!("the transfer moved the state by {lanes:.3e}, beyond its measured 2e-4");
            }
            // Refining where the estimate asked has to lower the estimate, or
            // the size field and the error term disagree about where the error
            // is and an adaptive run would not converge.
            let fell = oracle.global_indicator < expected.source.oracle.global_indicator;
            if !fell {
                eprintln!(
                    "the refined generation did not lower the estimate: {:.6e} against {:.6e}",
                    oracle.global_indicator, expected.source.oracle.global_indicator
                );
            }
            expected.failed = !ok || !fell || !carried;
            expected.phase = Phase::Done;
        }
        Phase::Done => {}
    }
}

/// Estimates on whichever generation is currently accepted, from the state the
/// device just handed back.
fn read_estimate(
    display: &CanonicalGpuDisplay,
    expected: &Expected,
    refined: bool,
) -> Option<(Estimate, f64)> {
    let generation = if refined {
        &expected.target
    } else {
        &expected.source
    };
    // The runtime the device actually used, decoded from the same copy as the
    // fields it explains, against the epoch origin in force now.
    //
    // A handoff rebases the clock, so an origin captured before one is stale
    // by the handoff time. Decoding with a stale origin displaces every
    // carrier phase by that much: here it moved the instantaneous mass by four
    // percent while the state itself agreed to `2e-7`, which reads exactly
    // like a transfer defect and is not one.
    let runtime = display.material_runtime(
        &generation.operator.initial_runtime(),
        display.clock?.epoch_origin_seconds,
    )?;
    let nodes = generation.operator.base().degrees_of_freedom();
    let samples = generation
        .operator
        .base()
        .complementary_degrees_of_freedom();
    if display.primary_flux.len() != nodes
        || display.previous_primary_flux.len() != nodes
        || display.complementary_flux.len() != samples
        || display.previous_complementary_flux.len() != samples
    {
        return None;
    }
    let widen = |values: &[f32]| {
        values
            .iter()
            .map(|value| f64::from(*value))
            .collect::<Vec<_>>()
    };
    let widen_flux = |values: &[[f32; 2]]| {
        values
            .iter()
            .map(|value| Point2::new(f64::from(value[0]), f64::from(value[1])))
            .collect::<Vec<_>>()
    };
    let primary = widen(&display.primary_flux);
    let previous_primary = widen(&display.previous_primary_flux);
    let complementary = widen_flux(&display.complementary_flux);
    let previous_complementary = widen_flux(&display.previous_complementary_flux);
    // Which of the two the device and the host disagree about: the state, or
    // the estimate built on it.
    let assumed = if refined {
        expected.target_time
    } else {
        expected.time
    };
    println!(
        "  clock: device {:.9e} against assumed {:.9e}, steps {}, epoch {} origin {:.9e}",
        display
            .clock
            .map_or(f64::NAN, |clock| clock.absolute_seconds),
        assumed,
        display.clock.map_or(0, |clock| clock.accepted_steps),
        display.clock.map_or(0, |clock| clock.epoch),
        display
            .clock
            .map_or(f64::NAN, |clock| clock.epoch_origin_seconds),
    );
    let lanes = [
        relative_l2(&primary, &generation.primary),
        relative_l2(&previous_primary, &generation.previous_primary),
        relative_l2_flux(&complementary, &generation.complementary),
        relative_l2_flux(&previous_complementary, &generation.previous_complementary),
    ];
    println!(
        "  state lanes: Q {:.3e}, previous Q {:.3e}, b {:.3e}, previous b {:.3e}",
        lanes[0], lanes[1], lanes[2], lanes[3]
    );
    let lanes = lanes.into_iter().fold(0.0_f64, f64::max);
    // The coefficients the two sides are actually using. A mass-row drive puts
    // its whole effect here, and nowhere in the flux terms, so this separates
    // a state disagreement from a coefficient one.
    let assumed_time = if refined {
        expected.target_time
    } else {
        expected.time
    };
    if let (Ok(device_mass), Ok(host_mass)) = (
        generation.operator.primary_mass_at(assumed_time, &runtime),
        generation
            .operator
            .primary_mass_at(assumed_time, &generation.runtime),
    ) {
        println!(
            "  instantaneous mass: {:.3e}",
            relative_l2(&device_mass, &host_mass)
        );
    }
    estimate(
        &generation.mesh,
        &generation.quadratic,
        &expected.scene,
        &generation.operator,
        &primary,
        &previous_primary,
        &complementary,
        &previous_complementary,
        if refined {
            expected.target_time
        } else {
            expected.time
        },
        expected.time_step,
        &runtime,
    )
    .as_ref()
    .map(|result| (summarize(result), lanes))
}

fn compare(
    label: &str,
    device: &Estimate,
    oracle: &Estimate,
    expected: &Expected,
    refined: bool,
) -> bool {
    // An element whose indicator is near zero has no meaningful relative
    // error, and letting one dominate would measure the fixture rather than
    // the device. The floor is a thousandth of the largest indicator in the
    // field, which is far below any refinement decision.
    let largest = oracle
        .element_indicators
        .iter()
        .copied()
        .fold(0.0_f64, f64::max);
    let floor = (1.0e-3 * largest).max(1.0e-12);
    let worst_element = device
        .element_indicators
        .iter()
        .zip(&oracle.element_indicators)
        .map(|(device, oracle)| (device - oracle).abs() / oracle.abs().max(floor))
        .fold(0.0_f64, f64::max);
    let errors = [
        (
            "total energy",
            relative_error(device.total_energy, oracle.total_energy),
        ),
        ("recovery", relative_error(device.recovery, oracle.recovery)),
        (
            "interior jump",
            relative_error(device.interior_jump, oracle.interior_jump),
        ),
        (
            "cell residual",
            relative_error(device.cell_residual, oracle.cell_residual),
        ),
        (
            "boundary residual",
            relative_error(device.boundary_residual, oracle.boundary_residual),
        ),
        (
            "global indicator",
            relative_error(device.global_indicator, oracle.global_indicator),
        ),
        ("worst element", worst_element),
    ];
    println!(
        "temporal AMR estimate {label} after {:.2} ms, indicator {:.6e} against oracle {:.6e}:",
        expected.started.elapsed().as_secs_f64() * 1_000.0,
        device.global_indicator,
        oracle.global_indicator
    );
    for (name, error) in errors {
        println!("  {name}: {error:.3e}");
    }
    // The same bounds on both generations, because the estimate is as good
    // after a refinement transfer as before one. They come from measured runs
    // on an M1 Max, which reproduce exactly, with room for a device difference
    // but not for a regression, which would be orders out. The cell residual
    // is the weakest term at `1.7e-5`: it is the one term that still
    // differentiates the nodal primary quotient, twice, through a Hessian.
    // The worst single element indicator is looser because one element's
    // relative error is a noisier quantity than any field-wide sum.
    let _ = refined;
    !errors.into_iter().any(|(name, error)| {
        let bound = if name == "worst element" {
            1.0e-2
        } else {
            1.0e-4
        };
        !error.is_finite() || error > bound
    })
}

fn relative_l2(actual: &[f64], expected: &[f64]) -> f64 {
    let difference: f64 = actual
        .iter()
        .zip(expected)
        .map(|(actual, expected)| (actual - expected).powi(2))
        .sum();
    let scale: f64 = expected.iter().map(|value| value * value).sum();
    (difference / scale.max(1.0e-30)).sqrt()
}

fn relative_l2_flux(actual: &[Point2], expected: &[Point2]) -> f64 {
    let difference: f64 = actual
        .iter()
        .zip(expected)
        .map(|(actual, expected)| (*actual - *expected).dot(*actual - *expected))
        .sum();
    let scale: f64 = expected.iter().map(|value| value.dot(*value)).sum();
    (difference / scale.max(1.0e-30)).sqrt()
}

fn relative_error(actual: f64, expected: f64) -> f64 {
    (actual - expected).abs() / expected.abs().max(1.0e-12)
}
