//! Real-device validation of temporal material runtime generation handoff.

use std::time::{Duration, Instant};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::canonical_gpu::{
    CANONICAL_FAILURE_INVERSE_DOMAIN, CanonicalGpuClock, CanonicalGpuDisplay,
    CanonicalGpuHandoffOutcome, CanonicalGpuPlan, CanonicalGpuRequest, CanonicalGpuRuntimeTransfer,
    CanonicalGpuTransferPlan, CanonicalWaveGpuPlugin,
};
use funfern_app::wave_gpu::WaveGpuPlugin;
use funfern_core::{
    CanonicalForcing, CanonicalMaterialDrive, CanonicalOutgoingHistoryTransferMap,
    CanonicalPrimaryTransferMap, CanonicalTemporalWaveOperator, CanonicalTemporalWaveState,
    CanonicalThinGapHistoryTransferMap, CanonicalVectorTransferMap, CoefficientLaw, MeshingOptions,
    OuterBoundaryCondition, QuadraticTransferMap, QuadraticWaveOperator, ScalarField, Scene,
    TimeDrive, mesh_scene,
};

const WARMUP_STEPS: u64 = 24;
const TARGET_STEPS: u64 = 40;

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
    absolute_time: f64,
    time_step: f64,
    phase: Phase,
    settle_after: Option<u64>,
    started: Instant,
    deadline: Instant,
    failed: bool,
    reject: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Warmup,
    Handoff,
    Evolution,
    /// After a rejected handoff: the source must keep stepping.
    Continue(u32),
    Done,
}

fn main() -> AppExit {
    let mut source_scene = Scene::initial();
    let material = &mut source_scene.materials[0];
    material.mass_law.drive = TimeDrive::TravellingModulation {
        depth: ScalarField::constant(0.21),
        frequency_hz: ScalarField::constant(0.75),
        phase_radians: ScalarField::constant(0.27),
        wavenumber: ScalarField::constant(2.4),
        angle_radians: ScalarField::constant(-0.35),
    };
    material.mass_law.alternate = Some(ScalarField::constant(1.6));
    material.stiffness_law.drive = TimeDrive::TimeCrystal {
        depth: ScalarField::constant(0.14),
        frequency_hz: ScalarField::constant(0.55),
        phase_radians: ScalarField::constant(-0.19),
        sharpness: ScalarField::constant(2.8),
    };
    material.stiffness_law.alternate = Some(ScalarField::constant(0.78));
    let material_id = material.id;

    let mut target_scene = source_scene.clone();
    let target_material = &mut target_scene.materials[0];
    target_material.mass_law.drive = TimeDrive::TravellingModulation {
        depth: ScalarField::constant(0.16),
        frequency_hz: ScalarField::constant(1.05),
        phase_radians: ScalarField::constant(0.27),
        wavenumber: ScalarField::constant(2.1),
        angle_radians: ScalarField::constant(-0.2),
    };
    target_material.stiffness_law.drive = TimeDrive::TimeCrystal {
        depth: ScalarField::constant(0.11),
        frequency_hz: ScalarField::constant(0.8),
        phase_radians: ScalarField::constant(0.63),
        sharpness: ScalarField::constant(2.2),
    };

    // `HANDOFF_NONLINEAR=1` gives the target generation Kerr on its mass row
    // and saturation on its stiffness row, so the handoff admits the driven
    // field into a field-dependent map. `HANDOFF_REJECT=1` gives it instead a
    // defocusing Kerr law whose bound the running field already exceeds: the
    // device must reject the handoff and keep stepping the source.
    let flag = |name: &str| std::env::var(name).is_ok_and(|value| value == "1");
    let reject = flag("HANDOFF_REJECT");
    if flag("HANDOFF_NONLINEAR") {
        target_material.mass_law.field = funfern_core::FieldLaw::Polynomial {
            chi1: ScalarField::constant(0.0),
            chi2: ScalarField::constant(40.0),
            amplitude_bound: None,
        };
        target_material.stiffness_law.field = funfern_core::FieldLaw::Saturable {
            chi: ScalarField::constant(300.0),
            saturation: ScalarField::constant(0.03),
        };
    }
    if reject {
        target_material.mass_law.field = funfern_core::FieldLaw::Polynomial {
            chi1: ScalarField::constant(0.0),
            chi2: ScalarField::constant(-0.2),
            amplitude_bound: Some(ScalarField::constant(0.04)),
        };
    }

    let mut fixed_scene = source_scene.clone();
    for material in &mut fixed_scene.materials {
        material.mass_law = CoefficientLaw::linear();
        material.stiffness_law = CoefficientLaw::linear();
    }
    let mesh = mesh_scene(
        &fixed_scene,
        71,
        MeshingOptions {
            target_edge_length: 0.2,
            ..MeshingOptions::default()
        },
    )
    .expect("temporal handoff mesh");
    let scalar = QuadraticWaveOperator::assemble_scene(
        &mesh,
        &fixed_scene,
        OuterBoundaryCondition::Reflecting,
    )
    .expect("temporal handoff scalar operator");
    let source_operator =
        CanonicalTemporalWaveOperator::compile_scene(&mesh, &scalar, &source_scene, 71)
            .expect("source temporal operator");
    let target_operator =
        CanonicalTemporalWaveOperator::compile_scene(&mesh, &scalar, &target_scene, 72)
            .expect("target temporal operator");
    let time_step = 0.38
        * source_operator
            .maximum_time_step()
            .min(target_operator.maximum_time_step());

    let primary = source_operator
        .base()
        .primary_mass()
        .iter()
        .zip(source_operator.base().node_points())
        .map(|(mass, point)| mass * (0.045 + 0.026 * (1.2 * point.x).sin()))
        .collect::<Vec<_>>();
    let potential = source_operator
        .base()
        .node_points()
        .iter()
        .map(|point| 0.021 * (0.8 * point.x - 0.3 * point.y).cos())
        .collect::<Vec<_>>();
    let complementary = source_operator
        .base()
        .compatible_flux(&potential)
        .expect("compatible temporal handoff flux");
    let mut source_state =
        CanonicalTemporalWaveState::new(&source_operator, time_step, primary, complementary)
            .expect("source temporal state");
    source_state
        .runtime_mut()
        .begin_switch(material_id, true, 0.0, 1.4)
        .expect("source active Switch");
    let source_plan = CanonicalGpuPlan::compile_temporal_bulk(
        &source_operator,
        &source_state,
        CanonicalGpuClock::initial(time_step).unwrap(),
    )
    .expect("source temporal GPU plan");

    for _ in 0..WARMUP_STEPS {
        source_state
            .step(&source_operator)
            .expect("source temporal oracle step");
    }
    let handoff_time = source_state.time();
    let old_mass_drive = source_scene.materials[0]
        .mass_law
        .drive
        .evaluate(&source_scene.materials[0].parameters)
        .expect("source mass drive");
    let target_state = CanonicalTemporalWaveState::new_at(
        &target_operator,
        time_step,
        source_state.primary_flux().to_vec(),
        source_state.complementary_flux().to_vec(),
        handoff_time,
    );
    if reject {
        // The reference refuses the same state, which is what the device
        // is held to.
        assert!(
            target_state.is_err(),
            "the fixture must leave the target's domain"
        );
    }
    let mut target_state = if reject {
        CanonicalTemporalWaveState::zero(&target_operator, time_step)
            .expect("placeholder for the rejected target")
    } else {
        target_state.expect("target temporal oracle state")
    };
    target_state
        .runtime_mut()
        .begin_switch(material_id, true, 0.0, 1.4)
        .expect("preserved target Switch");
    target_state
        .runtime_mut()
        .preserve_carrier(
            material_id,
            CanonicalMaterialDrive::MassCoefficient,
            old_mass_drive,
            handoff_time,
        )
        .expect("preserved target mass carrier");
    for _ in 0..TARGET_STEPS {
        if reject {
            break;
        }
        target_state
            .step(&target_operator)
            .expect("target temporal oracle step");
    }

    let target_placeholder = CanonicalTemporalWaveState::zero(&target_operator, time_step)
        .expect("target temporal placeholder");
    let target_plan = CanonicalGpuPlan::compile_temporal_bulk(
        &target_operator,
        &target_placeholder,
        CanonicalGpuClock::initial(time_step).unwrap(),
    )
    .expect("target temporal GPU plan");

    let interpolation = QuadraticTransferMap::identity_on_mesh(&mesh, &scalar, &scalar)
        .expect("temporal handoff identity map");
    let primary_map = CanonicalPrimaryTransferMap::prepare(
        &interpolation,
        source_operator.base(),
        target_operator.base(),
    )
    .expect("temporal primary handoff map");
    let vector_map = CanonicalVectorTransferMap::prepare(
        &mesh,
        source_operator.base(),
        &mesh,
        target_operator.base(),
    )
    .expect("temporal vector handoff map");
    let gap_map = CanonicalThinGapHistoryTransferMap::prepare(
        source_operator.base().thin_gap_samples(),
        target_operator.base().thin_gap_samples(),
    )
    .expect("temporal gap handoff map");
    let outgoing_map = CanonicalOutgoingHistoryTransferMap::prepare(
        &interpolation,
        source_operator.base(),
        target_operator.base(),
    )
    .expect("temporal outgoing handoff map");
    let source_forcing = CanonicalForcing::none(source_operator.base());
    let target_forcing = CanonicalForcing::none(target_operator.base());
    let runtime = CanonicalGpuRuntimeTransfer::identity(
        source_operator.base(),
        target_operator.base(),
        &source_forcing,
        &target_forcing,
    )
    .expect("temporal stable runtime ownership");
    let transfer = CanonicalGpuTransferPlan::compile(
        source_operator.base(),
        target_operator.base(),
        &source_forcing,
        &target_forcing,
        &primary_map,
        &vector_map,
        &gap_map,
        &outgoing_map,
        &runtime,
    )
    .expect("temporal geometry transfer")
    .with_temporal_material_runtime(&source_plan, &target_plan)
    .expect("temporal material runtime transfer");

    let expected = Expected {
        primary: target_state.primary_flux().to_vec(),
        complementary: target_state
            .complementary_flux()
            .iter()
            .map(|value| [value.x, value.y])
            .collect(),
        absolute_time: target_state.time(),
        time_step,
        reject,
        phase: Phase::Warmup,
        settle_after: None,
        started: Instant::now(),
        deadline: Instant::now() + Duration::from_secs(90),
        failed: false,
    };
    println!(
        "temporal handoff GPU gate: {} Q, {} b, {}+{} steps",
        source_plan.node_count, source_plan.sample_count, WARMUP_STEPS, TARGET_STEPS,
    );

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern temporal handoff GPU validation".into(),
            visible: false,
            resolution: (64, 64).into(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(WaveGpuPlugin)
    .add_plugins(CanonicalWaveGpuPlugin)
    .insert_resource(Pending {
        source: Some(source_plan),
        target: Some(target_plan),
        transfer: Some(transfer),
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
        .expect("select temporal handoff readback mode");
    request.install(
        &mut assets,
        &mut commands,
        pending.source.take().expect("source temporal plan"),
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
    let rejected_as_expected = expected.reject
        && matches!(
            request.handoff_outcome(),
            CanonicalGpuHandoffOutcome::Rejected(_)
        );
    if Instant::now() >= expected.deadline
        || (request.stats().failure() != 0 && !rejected_as_expected)
    {
        eprintln!("temporal handoff GPU gate timed out or failed");
        expected.failed = true;
        expected.phase = Phase::Done;
        return;
    }

    match expected.phase {
        Phase::Warmup
            if display
                .clock
                .is_some_and(|clock| clock.accepted_steps >= WARMUP_STEPS as u32) =>
        {
            request
                .begin_handoff(
                    &mut assets,
                    &mut commands,
                    pending.target.take().expect("target temporal plan"),
                    pending.transfer.take().expect("temporal transfer plan"),
                )
                .expect("begin temporal GPU handoff");
            expected.phase = Phase::Handoff;
        }
        Phase::Handoff => match request.handoff_outcome() {
            CanonicalGpuHandoffOutcome::Accepted if expected.reject => {
                eprintln!("the device admitted a handoff past the target's amplitude bound");
                expected.failed = true;
                expected.phase = Phase::Done;
            }
            CanonicalGpuHandoffOutcome::Accepted => {
                request.request_steps(TARGET_STEPS);
                expected.phase = Phase::Evolution;
            }
            CanonicalGpuHandoffOutcome::Rejected(reason) if expected.reject => {
                let accepted = display.clock.map_or(0, |clock| clock.accepted_steps);
                println!(
                    "temporal handoff into a bounded field law rejected with status {reason} \
                     ({}); the source kept {accepted} accepted steps",
                    funfern_app::canonical_gpu::canonical_failure_description(reason)
                );
                if reason != CANONICAL_FAILURE_INVERSE_DOMAIN {
                    expected.failed = true;
                    expected.phase = Phase::Done;
                    return;
                }
                request.request_steps(4);
                expected.phase = Phase::Continue(accepted);
            }
            CanonicalGpuHandoffOutcome::Rejected(reason) => {
                eprintln!("temporal GPU handoff rejected with {reason}");
                expected.failed = true;
                expected.phase = Phase::Done;
            }
            _ => {}
        },
        Phase::Evolution
            if display.clock.is_some_and(|clock| {
                clock.accepted_steps >= (WARMUP_STEPS + TARGET_STEPS) as u32
            }) && display.full_snapshot_completed_steps() >= WARMUP_STEPS + TARGET_STEPS =>
        {
            if expected.settle_after.is_none() {
                expected.settle_after = Some(display.readbacks + 2);
                return;
            }
            if display.readbacks < expected.settle_after.unwrap() {
                return;
            }
            let clock = display.clock.expect("evolution guard checked clock");
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
            let clock_error = (clock.absolute_seconds - expected.absolute_time).abs();
            println!(
                "temporal handoff completed in {:.2} ms: Q error {:.3e}, b error {:.3e}, clock error {:.3e}",
                expected.started.elapsed().as_secs_f64() * 1_000.0,
                q_error,
                b_error,
                clock_error,
            );
            if q_error > 8.0e-5
                || b_error > 8.0e-5
                || clock_error > 2.0e-4 * expected.time_step.max(1.0)
            {
                expected.failed = true;
                eprintln!("temporal handoff accuracy gate was exceeded");
            }
            expected.phase = Phase::Done;
        }
        Phase::Continue(from) => {
            let accepted = display.clock.map_or(0, |clock| clock.accepted_steps);
            if request.stats().failure() != 0 && !rejected_as_expected {
                eprintln!("the source faulted after the rejected handoff");
                expected.failed = true;
                expected.phase = Phase::Done;
            } else if accepted >= from + 4 {
                println!("the source stepped on: {from} -> {accepted} accepted steps");
                expected.phase = Phase::Done;
            }
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
