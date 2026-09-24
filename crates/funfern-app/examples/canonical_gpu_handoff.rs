//! Full render-graph validation of Stage 5 latest-state generation handoff.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::canonical_gpu::{
    CanonicalComponentTransfer, CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuHandoffOutcome,
    CanonicalGpuPlan, CanonicalGpuRequest, CanonicalGpuRuntimeTransfer, CanonicalGpuTransferPlan,
    CanonicalWaveGpuPlugin,
};
use funfern_core::{
    BACKGROUND_REGION, CanonicalAuxiliaryState, CanonicalForcing,
    CanonicalOutgoingHistoryTransferMap, CanonicalOutgoingNormalizedTransferJob,
    CanonicalPrimaryTransferMap, CanonicalThinGapHistoryTransferMap, CanonicalVectorTransferJob,
    CanonicalWaveOperator, CanonicalWaveState, InternalBoundary, InternalBoundaryCoupling,
    InternalBoundaryId, InternalBoundaryLaw, MeshingOptions, OpenCubicSpline,
    OuterBoundaryCondition, Point2, QuadraticTransferMap, QuadraticWaveOperator, Scene, TimeSignal,
    mesh_scene,
};

const WARMUP_STEPS: u64 = 12;
const MEASURED_STEPS: u64 = 24;
const HANDOFF_STEPS: u64 = 8;
const SETTLEMENT_STEPS: u64 = 8;

#[derive(Resource)]
struct Pending {
    source: Option<CanonicalGpuPlan>,
    target: Option<CanonicalGpuPlan>,
    transfer: Option<CanonicalGpuTransferPlan>,
}

#[derive(Resource)]
struct Expected {
    primary: Vec<f64>,
    complementary: Vec<[f64; 2]>,
    auxiliary: Vec<f64>,
    time_step: f64,
    handoff_steps: u64,
    settlement_steps: u64,
    measured_steps: u64,
    source_bytes: usize,
    target_bytes: usize,
    transfer_bytes: usize,
    phase: Phase,
    started: Option<Instant>,
    submitted: Option<Instant>,
    accepted_elapsed: Option<Duration>,
    boundary_elapsed: Option<Duration>,
    settle_after: Option<u64>,
    failed: bool,
    inject_failure: bool,
    rollback_snapshot: Option<Vec<u32>>,
    source_generation: u64,
    deadline: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Warmup,
    Handoff,
    Evolution,
    Done,
}

fn main() -> AppExit {
    let second_order = std::env::args().any(|argument| argument == "--second-order");
    let same_mesh = std::env::args().any(|argument| argument == "--same-mesh");
    let thin_gap = std::env::args().any(|argument| argument == "--thin-gap");
    let inject_failure = std::env::args().any(|argument| argument == "--failure");
    let advance_during_upload =
        std::env::args().any(|argument| argument == "--advance-during-upload");
    let handoff_steps = if advance_during_upload {
        HANDOFF_STEPS
    } else {
        0
    };
    let settlement_steps =
        if std::env::args().any(|argument| argument == "--continue-through-settlement") {
            SETTLEMENT_STEPS
        } else {
            0
        };
    assert!(
        !(inject_failure && (advance_during_upload || settlement_steps != 0)),
        "--failure and continued evolution exercise different handoff contracts"
    );
    let source_drive = std::env::args().any(|argument| argument == "--source");
    let measured_steps = std::env::args()
        .find_map(|argument| argument.strip_prefix("--steps=")?.parse::<u64>().ok())
        .unwrap_or(MEASURED_STEPS);
    let prescribed = std::env::args().any(|argument| argument == "--prescribed");
    let standard = std::env::args().any(|argument| argument == "--standard");
    let requested_edge =
        std::env::args().find_map(|argument| argument.strip_prefix("--edge=")?.parse::<f64>().ok());
    let source_edge = requested_edge.unwrap_or(if standard { 0.08 } else { 0.16 });
    let target_edge = requested_edge.unwrap_or(if standard { 0.08 } else { 0.12 });
    let mut scene = if thin_gap {
        Scene::default()
    } else {
        Scene::initial()
    };
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
    let source_mesh = Arc::new(
        mesh_scene(
            &scene,
            11,
            MeshingOptions {
                target_edge_length: source_edge,
                max_vertices: 100_000,
                max_triangles: 200_000,
                max_refinement_steps: 100_000,
                ..MeshingOptions::default()
            },
        )
        .expect("source mesh"),
    );
    let target_mesh = if same_mesh {
        source_mesh.clone()
    } else {
        Arc::new(
            mesh_scene(
                &scene,
                12,
                MeshingOptions {
                    target_edge_length: target_edge,
                    max_vertices: 100_000,
                    max_triangles: 200_000,
                    max_refinement_steps: 100_000,
                    ..MeshingOptions::default()
                },
            )
            .expect("target mesh"),
        )
    };
    let boundary = if second_order {
        OuterBoundaryCondition::SecondOrderOutgoing
    } else {
        OuterBoundaryCondition::FirstOrderOutgoing
    };
    let source_scalar = QuadraticWaveOperator::assemble_scene(&source_mesh, &scene, boundary)
        .expect("source scalar");
    let target_scalar = QuadraticWaveOperator::assemble_scene(&target_mesh, &scene, boundary)
        .expect("target scalar");
    let source_operator = Arc::new(
        CanonicalWaveOperator::compile_scene(&source_mesh, &source_scalar, &scene, 11)
            .expect("source canonical operator"),
    );
    let target_operator = Arc::new(
        CanonicalWaveOperator::compile_scene(&target_mesh, &target_scalar, &scene, 12)
            .expect("target canonical operator"),
    );
    let time_step = 0.72
        * source_operator
            .maximum_time_step()
            .min(target_operator.maximum_time_step());
    let primary = source_operator
        .node_points()
        .iter()
        .map(|point| 0.35 + 0.08 * (1.1 * point.x).sin() * (0.7 * point.y).cos())
        .collect::<Vec<_>>();
    let potential = source_operator
        .node_points()
        .iter()
        .map(|point| 0.05 * (0.6 * point.x - 0.4 * point.y).cos())
        .collect::<Vec<_>>();
    let initial = CanonicalWaveState::from_primary_and_potential(
        &source_operator,
        time_step,
        &primary,
        &potential,
    )
    .expect("source initial state");
    let prescribed_signal = TimeSignal::harmonic(0.04, 0.03, 0.8, 0.25);
    let mut source_prescribed = vec![None; source_operator.degrees_of_freedom()];
    let mut target_prescribed = vec![None; target_operator.degrees_of_freedom()];
    if prescribed {
        source_prescribed[0] = Some(prescribed_signal);
        target_prescribed[0] = Some(prescribed_signal);
    }
    let mut source_forcing = CanonicalForcing::from_prescribed(&source_operator, source_prescribed)
        .expect("source forcing");
    let mut target_forcing = CanonicalForcing::from_prescribed(&target_operator, target_prescribed)
        .expect("target forcing");
    let source_signal = TimeSignal::harmonic(0.012, 0.035, 1.1, -0.37);
    let target_source_weights = target_operator
        .primary_mass()
        .iter()
        .zip(target_operator.node_points())
        .map(|(mass, point)| 0.03 * mass * (0.4 * point.x + 0.2 * point.y).cos())
        .collect::<Vec<_>>();
    if source_drive {
        let source_weights = source_operator
            .primary_mass()
            .iter()
            .zip(source_operator.node_points())
            .map(|(mass, point)| 0.03 * mass * (0.4 * point.x + 0.2 * point.y).cos())
            .collect::<Vec<_>>();
        source_forcing
            .push_source(
                funfern_core::CanonicalSource::direct(
                    &source_operator,
                    source_weights,
                    source_signal,
                )
                .expect("source generation drive"),
            )
            .expect("source generation forcing");
        target_forcing
            .push_source(
                funfern_core::CanonicalSource::direct(
                    &target_operator,
                    target_source_weights.clone(),
                    source_signal,
                )
                .expect("target generation drive"),
            )
            .expect("target generation forcing");
    }
    let source_plan = CanonicalGpuPlan::compile_with_quadratic(
        &source_operator,
        &source_scalar,
        &initial,
        &source_forcing,
        CanonicalGpuClock::initial(time_step).unwrap(),
    )
    .expect("source GPU plan");

    let transfer_preparation_started = Instant::now();
    let interpolation_started = Instant::now();
    let interpolation = if same_mesh {
        QuadraticTransferMap::identity_on_mesh(&source_mesh, &source_scalar, &target_scalar)
            .expect("same-mesh quadratic identity")
    } else {
        QuadraticTransferMap::build(&source_mesh, &source_scalar, &target_mesh, &target_scalar)
            .expect("quadratic interpolation")
    };
    let interpolation_elapsed = interpolation_started.elapsed();
    let primary_started = Instant::now();
    let primary_map = CanonicalPrimaryTransferMap::prepare_with_meshes(
        &interpolation,
        &source_mesh,
        &source_operator,
        &target_mesh,
        &target_operator,
    )
    .expect("primary transfer map");
    let primary_elapsed = primary_started.elapsed();
    let vector_started = Instant::now();
    let mut vector_job = CanonicalVectorTransferJob::new(
        source_mesh.clone(),
        source_operator.clone(),
        target_mesh.clone(),
        target_operator.clone(),
    );
    let mut maximum_vector_slice = Duration::ZERO;
    let vector_map = loop {
        let slice_started = Instant::now();
        let result = vector_job.advance(256);
        maximum_vector_slice = maximum_vector_slice.max(slice_started.elapsed());
        if let Some(result) = result {
            break result.expect("vector transfer map");
        }
    };
    let vector_elapsed = vector_started.elapsed();
    let history_started = Instant::now();
    let gap_map = CanonicalThinGapHistoryTransferMap::prepare(
        source_operator.thin_gap_samples(),
        target_operator.thin_gap_samples(),
    )
    .expect("gap history map");
    let outgoing_map = Arc::new(
        CanonicalOutgoingHistoryTransferMap::prepare(
            &interpolation,
            &source_operator,
            &target_operator,
        )
        .expect("outgoing history map"),
    );
    let history_elapsed = history_started.elapsed();
    let outgoing_started = Instant::now();
    let mut outgoing_job = CanonicalOutgoingNormalizedTransferJob::new(
        outgoing_map.clone(),
        source_operator.clone(),
        target_operator.clone(),
    );
    let mut maximum_outgoing_slice = Duration::ZERO;
    let outgoing_transfer = loop {
        let slice_started = Instant::now();
        let result = outgoing_job.advance(128);
        maximum_outgoing_slice = maximum_outgoing_slice.max(slice_started.elapsed());
        if let Some(result) = result {
            break result.expect("normalized outgoing transfer");
        }
    };
    let outgoing_elapsed = outgoing_started.elapsed();
    let runtime = CanonicalGpuRuntimeTransfer {
        components: (0..target_operator.component_count())
            .map(|component| CanonicalComponentTransfer {
                sources: (component < source_operator.component_count())
                    .then_some(vec![(component as u32, 1.0)])
                    .unwrap_or_default(),
            })
            .collect(),
        prescribed_sources: (0..target_operator.degrees_of_freedom())
            .map(|node| (prescribed && node == 0).then_some(0))
            .collect(),
        drive_sources: (0..target_forcing.sources().len())
            .map(|drive| source_drive.then_some(drive as u32))
            .collect(),
        runtime_serials: [0; 4],
    };
    let packing_started = Instant::now();
    let transfer_plan = CanonicalGpuTransferPlan::compile_prepared(
        &source_operator,
        &target_operator,
        &source_forcing,
        &target_forcing,
        &primary_map,
        &vector_map,
        &gap_map,
        &outgoing_transfer,
        &runtime,
    )
    .expect("GPU transfer plan");
    let packing_elapsed = packing_started.elapsed();
    let transfer_preparation_elapsed = transfer_preparation_started.elapsed();

    let mut oracle = initial;
    for _ in 0..WARMUP_STEPS {
        oracle
            .step_with_forcing(&source_operator, &source_forcing)
            .expect("source oracle step");
    }
    for _ in 0..handoff_steps {
        oracle
            .step_with_forcing(&source_operator, &source_forcing)
            .expect("source oracle step during target upload");
    }
    let source_steps = WARMUP_STEPS + handoff_steps;
    let desired_totals = (0..target_operator.component_count())
        .map(|component| {
            (component < source_operator.component_count()).then(|| {
                oracle
                    .primary_flux()
                    .iter()
                    .zip(source_operator.component_labels())
                    .filter(|(_, label)| **label as usize == component)
                    .map(|(value, _)| value)
                    .sum::<f64>()
            })
        })
        .collect::<Vec<_>>();
    let (target_q, _) = primary_map
        .transfer(
            oracle.primary_flux(),
            &desired_totals,
            &(0..target_operator.degrees_of_freedom())
                .map(|node| prescribed && node == 0)
                .collect::<Vec<_>>(),
        )
        .expect("CPU primary transfer");
    let (target_b, _) = vector_map
        .transfer(oracle.complementary_flux())
        .expect("CPU vector transfer");
    let mut target_state = CanonicalWaveState::new(&target_operator, time_step, target_q, target_b)
        .expect("target state");
    let (gap_memory, _) = gap_map
        .transfer(&oracle.thin_gap_memory(&source_operator).unwrap())
        .expect("CPU gap transfer");
    target_state
        .set_thin_gap_memory(&target_operator, &gap_memory, 1.0)
        .expect("target gap history");
    let source_outgoing = oracle
        .outgoing_physical_memory(&source_operator)
        .expect("source outgoing memory");
    let (target_outgoing, _) = outgoing_map
        .transfer(&source_operator, &target_operator, source_outgoing.as_ref())
        .expect("CPU outgoing transfer");
    if let Some(memory) = target_outgoing.as_ref() {
        target_state
            .set_outgoing_physical_memory(&target_operator, memory)
            .expect("target outgoing history");
    }
    let mut target_oracle_forcing = if prescribed {
        let [offset, amplitude, frequency, phase] = prescribed_signal.harmonic_parameters();
        let shifted = TimeSignal::harmonic(
            offset,
            amplitude,
            frequency,
            phase + std::f64::consts::TAU * frequency * source_steps as f64 * time_step,
        );
        let mut signals = vec![None; target_operator.degrees_of_freedom()];
        signals[0] = Some(shifted);
        CanonicalForcing::from_prescribed(&target_operator, signals).expect("target oracle forcing")
    } else {
        CanonicalForcing::none(&target_operator)
    };
    if source_drive {
        let [offset, amplitude, frequency, phase] = source_signal.harmonic_parameters();
        let shifted = TimeSignal::harmonic(
            offset,
            amplitude,
            frequency,
            phase + std::f64::consts::TAU * frequency * source_steps as f64 * time_step,
        );
        target_oracle_forcing
            .push_source(
                funfern_core::CanonicalSource::direct(
                    &target_operator,
                    target_source_weights,
                    shifted,
                )
                .expect("shifted target drive"),
            )
            .expect("shifted target forcing");
    }
    for _ in 0..settlement_steps + measured_steps {
        target_state
            .step_with_forcing(&target_operator, &target_oracle_forcing)
            .expect("target oracle step");
    }
    let target_placeholder = CanonicalWaveState::zero(&target_operator, time_step).unwrap();
    let target_plan_started = Instant::now();
    let mut target_plan = CanonicalGpuPlan::compile_with_quadratic(
        &target_operator,
        &target_scalar,
        &target_placeholder,
        &target_forcing,
        CanonicalGpuClock::initial(time_step).unwrap(),
    )
    .expect("target GPU plan");
    let target_plan_elapsed = target_plan_started.elapsed();
    if inject_failure {
        target_plan
            .stage_failure_injection(funfern_app::canonical_gpu::CANONICAL_FAILURE_NON_FINITE, 0)
            .expect("handoff failure injection");
    }
    let expected = Expected {
        primary: target_state.primary_flux().to_vec(),
        complementary: target_state
            .complementary_flux()
            .iter()
            .map(|value| [value.x, value.y])
            .collect(),
        auxiliary: match target_state.auxiliaries() {
            CanonicalAuxiliaryState::None => Vec::new(),
            CanonicalAuxiliaryState::Linear(value) => value
                .thin_gap_jump()
                .iter()
                .chain(value.outgoing_z())
                .copied()
                .collect(),
            _ => panic!("unsupported auxiliary variant"),
        },
        time_step,
        handoff_steps,
        settlement_steps,
        measured_steps,
        source_bytes: source_plan.manifest.bytes.steady_bytes(),
        target_bytes: target_plan.manifest.bytes.steady_bytes(),
        transfer_bytes: transfer_plan.manifest.bytes,
        phase: Phase::Warmup,
        started: None,
        submitted: None,
        accepted_elapsed: None,
        boundary_elapsed: None,
        settle_after: None,
        failed: false,
        inject_failure,
        rollback_snapshot: None,
        source_generation: 0,
        deadline: Instant::now() + Duration::from_secs(90),
    };
    println!(
        "prepared handoff {}→{} Q, {}→{} b, {} components, {}→{} gap, {}→{} outgoing, {} transfer words ({:.2} MiB source + {:.2} MiB target + {:.2} MiB map = {:.2} MiB peak)",
        source_operator.degrees_of_freedom(),
        target_operator.degrees_of_freedom(),
        source_operator.complementary_degrees_of_freedom(),
        target_operator.complementary_degrees_of_freedom(),
        source_operator.component_count(),
        source_operator.thin_gap_samples().len(),
        target_operator.thin_gap_samples().len(),
        source_operator
            .outgoing_boundary()
            .map_or(0, |boundary| boundary.auxiliary_count()),
        target_operator
            .outgoing_boundary()
            .map_or(0, |boundary| boundary.auxiliary_count()),
        transfer_plan.manifest.word_count,
        expected.source_bytes as f64 / (1024.0 * 1024.0),
        expected.target_bytes as f64 / (1024.0 * 1024.0),
        expected.transfer_bytes as f64 / (1024.0 * 1024.0),
        (expected.source_bytes + expected.target_bytes + expected.transfer_bytes) as f64
            / (1024.0 * 1024.0),
    );
    println!(
        "target GPU plan packed in {:.2} ms",
        target_plan_elapsed.as_secs_f64() * 1_000.0,
    );
    println!(
        "transfer preparation {:.2} ms: interpolation {:.2}, primary {:.2}, vector {:.2} (max slice {:.2}), histories {:.2}, outgoing bases {:.2} (max slice {:.2}), GPU packing {:.2}",
        transfer_preparation_elapsed.as_secs_f64() * 1_000.0,
        interpolation_elapsed.as_secs_f64() * 1_000.0,
        primary_elapsed.as_secs_f64() * 1_000.0,
        vector_elapsed.as_secs_f64() * 1_000.0,
        maximum_vector_slice.as_secs_f64() * 1_000.0,
        history_elapsed.as_secs_f64() * 1_000.0,
        outgoing_elapsed.as_secs_f64() * 1_000.0,
        maximum_outgoing_slice.as_secs_f64() * 1_000.0,
        packing_elapsed.as_secs_f64() * 1_000.0,
    );

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern canonical GPU handoff validation".into(),
            visible: false,
            resolution: (64, 64).into(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(CanonicalWaveGpuPlugin)
    .insert_resource(Pending {
        source: Some(source_plan),
        target: Some(target_plan),
        transfer: Some(transfer_plan),
    })
    .insert_resource(expected)
    .add_systems(Startup, install)
    .add_systems(Update, validate);
    app.run()
}

fn install(
    mut commands: Commands,
    mut pending: ResMut<Pending>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut request: ResMut<CanonicalGpuRequest>,
) {
    request
        .set_continuous_full_state_readback(true)
        .expect("select validation readback mode");
    request.install(
        &mut assets,
        &mut commands,
        pending.source.take().expect("source plan"),
    );
    request.request_steps(WARMUP_STEPS);
    commands.spawn(Camera2d);
}

fn validate(
    mut commands: Commands,
    mut pending: ResMut<Pending>,
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
    if Instant::now() >= expected.deadline || request.stats().failure() != 0 {
        eprintln!("canonical handoff timed out or failed");
        expected.phase = Phase::Done;
        expected.failed = true;
        exit.write(AppExit::error());
        return;
    }
    let clock = display.clock;
    if expected.phase == Phase::Handoff
        && expected.submitted.is_none()
        && request.handoff_submitted()
    {
        expected.submitted = Some(Instant::now());
        request.request_steps(expected.settlement_steps);
    }
    match expected.phase {
        Phase::Warmup if clock.is_some_and(|clock| clock.accepted_steps >= WARMUP_STEPS as u32) => {
            expected.rollback_snapshot = Some(display.accepted_storage_bits());
            expected.source_generation = display.generation;
            let upload_pack_started = Instant::now();
            request
                .begin_handoff(
                    &mut assets,
                    &mut commands,
                    pending.target.take().expect("target plan"),
                    pending.transfer.take().expect("transfer plan"),
                )
                .expect("begin GPU handoff");
            println!(
                "main-thread handoff buffer serialization took {:.2} ms",
                upload_pack_started.elapsed().as_secs_f64() * 1_000.0,
            );
            expected.started = Some(Instant::now());
            request.request_steps(expected.handoff_steps);
            expected.phase = Phase::Handoff;
        }
        Phase::Handoff => match request.handoff_outcome() {
            CanonicalGpuHandoffOutcome::Accepted => {
                if expected.inject_failure {
                    eprintln!("injected handoff failure was incorrectly accepted");
                    expected.failed = true;
                    expected.phase = Phase::Done;
                    return;
                }
                if display.generation != request.generation()
                    || display.primary_flux.len() != expected.primary.len()
                {
                    eprintln!("handoff admission became visible before its target display receipt");
                    expected.failed = true;
                    expected.phase = Phase::Done;
                    return;
                }
                let accepted = Instant::now();
                expected.accepted_elapsed =
                    Some(accepted.duration_since(expected.started.unwrap()));
                expected.boundary_elapsed = expected
                    .submitted
                    .map(|submitted| accepted.duration_since(submitted));
                request.request_steps(expected.measured_steps);
                expected.phase = Phase::Evolution;
            }
            CanonicalGpuHandoffOutcome::Rejected(reason) => {
                if expected.inject_failure
                    && reason == funfern_app::canonical_gpu::CANONICAL_FAILURE_NON_FINITE
                    && display.generation == expected.source_generation
                    && display.accepted_storage_bits()
                        == *expected.rollback_snapshot.as_ref().unwrap()
                    && clock.is_some_and(|clock| clock.accepted_steps == WARMUP_STEPS as u32)
                {
                    println!(
                        "injected handoff rejected in {:.2} ms with byte-exact source rollback",
                        expected.started.unwrap().elapsed().as_secs_f64() * 1_000.0,
                    );
                } else {
                    eprintln!("canonical handoff rejected with {reason}");
                    expected.failed = true;
                }
                expected.phase = Phase::Done;
            }
            _ => {}
        },
        Phase::Evolution
            if clock.is_some_and(|clock| {
                clock.accepted_steps
                    >= (WARMUP_STEPS
                        + expected.handoff_steps
                        + expected.settlement_steps
                        + expected.measured_steps) as u32
            }) && display.primary_flux.len() == expected.primary.len() =>
        {
            let clock = clock.expect("evolution guard checked the display clock");
            if expected.settle_after.is_none() {
                expected.settle_after = Some(display.readbacks + 2);
                return;
            }
            if display.readbacks < expected.settle_after.unwrap() {
                return;
            }
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
            let auxiliary_error = rms_difference(
                display.auxiliary.iter().map(|value| *value as f64),
                expected.auxiliary.iter().copied(),
            );
            let absolute_error = (clock.absolute_seconds
                - (WARMUP_STEPS
                    + expected.handoff_steps
                    + expected.settlement_steps
                    + expected.measured_steps) as f64
                    * expected.time_step)
                .abs();
            println!(
                "handoff committed in {:.2} ms (admission + first display receipt {:.2} ms, source live); Q {:.3e}, b {:.3e}, auxiliary RMS {:.3e}, clock {:.3e} s",
                expected.accepted_elapsed.unwrap().as_secs_f64() * 1_000.0,
                expected
                    .boundary_elapsed
                    .map_or(0.0, |elapsed| elapsed.as_secs_f64() * 1_000.0),
                q_error,
                b_error,
                auxiliary_error,
                absolute_error,
            );
            expected.failed = q_error > 3.0e-5
                || b_error > 3.0e-5
                || auxiliary_error > 2.0e-5
                || absolute_error > 2.0e-5;
            expected.phase = Phase::Done;
        }
        _ => {}
    }
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
