//! Real-device rejection/rollback gate for temporal material live events.

use std::time::{Duration, Instant};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::canonical_gpu::{
    CANONICAL_FAILURE_NON_FINITE, CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuLiveEvent,
    CanonicalGpuPlan, CanonicalGpuRequest, CanonicalWaveGpuPlugin,
};
use funfern_app::wave_gpu::WaveGpuPlugin;
use funfern_core::{
    CanonicalMaterialDrive, CanonicalTemporalWaveOperator, CanonicalTemporalWaveState,
    CoefficientLaw, MeshingOptions, OuterBoundaryCondition, QuadraticWaveOperator, ScalarField,
    Scene, TimeDrive, mesh_scene,
};

const WARMUP_STEPS: u64 = 20;
const TARGET_STEPS: u64 = 36;

#[derive(Resource)]
struct Pending {
    plan: Option<CanonicalGpuPlan>,
    rejected_event: Option<CanonicalGpuLiveEvent>,
    accepted_event: Option<CanonicalGpuLiveEvent>,
}

#[derive(Resource)]
struct Expected {
    primary: Vec<f64>,
    complementary: Vec<[f64; 2]>,
    absolute_time: f64,
    time_step: f64,
    material_serial_before: u32,
    rollback_state: Option<Vec<u32>>,
    rollback_readbacks: u64,
    phase: Phase,
    settle_after: Option<u64>,
    deadline: Instant,
    failed: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Warmup,
    Rejection,
    OldLawStep,
    Acceptance,
    Evolution,
    Done,
}

fn main() -> AppExit {
    let mut source_scene = Scene::initial();
    let material = &mut source_scene.materials[0];
    material.mass_law.drive = TimeDrive::TravellingModulation {
        depth: ScalarField::constant(0.2),
        frequency_hz: ScalarField::constant(0.7),
        phase_radians: ScalarField::constant(0.24),
        wavenumber: ScalarField::constant(2.2),
        angle_radians: ScalarField::constant(-0.3),
    };
    material.mass_law.alternate = Some(ScalarField::constant(1.55));
    material.stiffness_law.drive = TimeDrive::TimeCrystal {
        depth: ScalarField::constant(0.13),
        frequency_hz: ScalarField::constant(0.5),
        phase_radians: ScalarField::constant(-0.17),
        sharpness: ScalarField::constant(2.5),
    };
    material.stiffness_law.alternate = Some(ScalarField::constant(0.8));
    let material_id = material.id;

    let mut target_scene = source_scene.clone();
    let target_material = &mut target_scene.materials[0];
    target_material.mass_law.drive = TimeDrive::TravellingModulation {
        depth: ScalarField::constant(0.15),
        frequency_hz: ScalarField::constant(1.0),
        phase_radians: ScalarField::constant(0.24),
        wavenumber: ScalarField::constant(2.0),
        angle_radians: ScalarField::constant(-0.18),
    };
    target_material.mass_law.alternate = Some(ScalarField::constant(1.42));
    target_material.stiffness_law.drive = TimeDrive::TimeCrystal {
        depth: ScalarField::constant(0.1),
        frequency_hz: ScalarField::constant(0.76),
        phase_radians: ScalarField::constant(-0.17),
        sharpness: ScalarField::constant(2.1),
    };
    target_material.stiffness_law.alternate = Some(ScalarField::constant(0.86));

    let mut fixed_scene = source_scene.clone();
    for material in &mut fixed_scene.materials {
        material.mass_law = CoefficientLaw::linear();
        material.stiffness_law = CoefficientLaw::linear();
    }
    let mesh = mesh_scene(
        &fixed_scene,
        81,
        MeshingOptions {
            target_edge_length: 0.2,
            ..MeshingOptions::default()
        },
    )
    .expect("temporal rollback mesh");
    let scalar = QuadraticWaveOperator::assemble_scene(
        &mesh,
        &fixed_scene,
        OuterBoundaryCondition::Reflecting,
    )
    .expect("temporal rollback scalar operator");
    let source_operator =
        CanonicalTemporalWaveOperator::compile_scene(&mesh, &scalar, &source_scene, 81)
            .expect("source temporal rollback operator");
    let target_operator =
        CanonicalTemporalWaveOperator::compile_scene(&mesh, &scalar, &target_scene, 82)
            .expect("target temporal rollback operator");
    let time_step = 0.38
        * source_operator
            .maximum_time_step()
            .min(target_operator.maximum_time_step());
    let primary = source_operator
        .base()
        .primary_mass()
        .iter()
        .zip(source_operator.base().node_points())
        .map(|(mass, point)| mass * (0.043 + 0.025 * (1.15 * point.x).sin()))
        .collect::<Vec<_>>();
    let potential = source_operator
        .base()
        .node_points()
        .iter()
        .map(|point| 0.019 * (0.75 * point.x - 0.28 * point.y).cos())
        .collect::<Vec<_>>();
    let complementary = source_operator
        .base()
        .compatible_flux(&potential)
        .expect("compatible temporal rollback flux");
    let mut oracle =
        CanonicalTemporalWaveState::new(&source_operator, time_step, primary, complementary)
            .expect("temporal rollback state");
    oracle
        .runtime_mut()
        .begin_switch(material_id, true, 0.0, 1.25)
        .expect("active temporal rollback Switch");
    let source_plan = CanonicalGpuPlan::compile_temporal_bulk(
        &source_operator,
        &oracle,
        CanonicalGpuClock::initial(time_step).unwrap(),
    )
    .expect("source temporal rollback GPU plan");
    let target_placeholder = CanonicalTemporalWaveState::zero(&target_operator, time_step)
        .expect("target temporal rollback placeholder");
    let target_plan = CanonicalGpuPlan::compile_temporal_bulk(
        &target_operator,
        &target_placeholder,
        CanonicalGpuClock::initial(time_step).unwrap(),
    )
    .expect("target temporal rollback GPU plan");
    let rejected_event = CanonicalGpuLiveEvent::temporal_law_patch(&source_plan, &target_plan, 1)
        .expect("rejected temporal law event");
    let accepted_event = CanonicalGpuLiveEvent::temporal_law_patch(&source_plan, &target_plan, 2)
        .expect("accepted temporal law event");

    for _ in 0..=WARMUP_STEPS {
        oracle
            .step(&source_operator)
            .expect("old-law temporal rollback oracle step");
    }
    let edit_time = oracle.time();
    for (drive, old_drive) in [
        (
            CanonicalMaterialDrive::MassCoefficient,
            source_scene.materials[0]
                .mass_law
                .drive
                .evaluate(&source_scene.materials[0].parameters)
                .expect("old mass drive"),
        ),
        (
            CanonicalMaterialDrive::StiffnessCoefficient,
            source_scene.materials[0]
                .stiffness_law
                .drive
                .evaluate(&source_scene.materials[0].parameters)
                .expect("old stiffness drive"),
        ),
    ] {
        oracle
            .runtime_mut()
            .preserve_carrier(material_id, drive, old_drive, edit_time)
            .expect("preserved retry carrier");
    }
    for _ in 0..TARGET_STEPS {
        oracle
            .step(&target_operator)
            .expect("target temporal rollback oracle step");
    }

    let expected = Expected {
        primary: oracle.primary_flux().to_vec(),
        complementary: oracle
            .complementary_flux()
            .iter()
            .map(|value| [value.x, value.y])
            .collect(),
        absolute_time: oracle.time(),
        time_step,
        material_serial_before: 0,
        rollback_state: None,
        rollback_readbacks: 0,
        phase: Phase::Warmup,
        settle_after: None,
        deadline: Instant::now() + Duration::from_secs(90),
        failed: false,
    };

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern temporal event rollback validation".into(),
            visible: false,
            resolution: (64, 64).into(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(WaveGpuPlugin)
    .add_plugins(CanonicalWaveGpuPlugin)
    .insert_resource(Pending {
        plan: Some(source_plan),
        rejected_event: Some(rejected_event),
        accepted_event: Some(accepted_event),
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
        .expect("select temporal rollback readback mode");
    request.install(
        &mut assets,
        &mut commands,
        pending.plan.take().expect("temporal rollback plan"),
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
        eprintln!("temporal event rollback gate timed out or latched a solver failure");
        expected.failed = true;
        expected.phase = Phase::Done;
        return;
    }
    let Some(clock) = display.clock else { return };

    match expected.phase {
        Phase::Warmup
            if clock.accepted_steps >= WARMUP_STEPS as u32
                && display.full_snapshot_completed_steps() >= WARMUP_STEPS =>
        {
            expected.rollback_state = Some(display.accepted_storage_bits());
            expected.rollback_readbacks = display.readbacks;
            expected.material_serial_before = display.runtime_serials[1];
            request
                .inject_failure(&mut assets, &mut commands, CANONICAL_FAILURE_NON_FINITE, 0)
                .expect("inject temporal event rejection");
            request
                .queue_live_event(
                    &mut assets,
                    pending
                        .rejected_event
                        .take()
                        .expect("one rejected temporal event"),
                )
                .expect("queue rejected temporal event");
            expected.phase = Phase::Rejection;
        }
        Phase::Rejection
            if request.stats().processed_event() >= 1
                && !request.live_event_pending()
                && display.readbacks >= expected.rollback_readbacks + 2 =>
        {
            if request.stats().event_rejection() != CANONICAL_FAILURE_NON_FINITE
                || clock.accepted_steps != WARMUP_STEPS as u32
                || clock.event_serial != 0
                || display.runtime_serials[1] != expected.material_serial_before
                || display.accepted_storage_bits()
                    != *expected.rollback_state.as_ref().expect("rollback snapshot")
            {
                eprintln!("rejected temporal event changed accepted state or ownership");
                expected.failed = true;
                expected.phase = Phase::Done;
                return;
            }
            request
                .clear_failure(&mut assets, &mut commands)
                .expect("clear temporal event failure injection");
            request.request_steps(1);
            expected.phase = Phase::OldLawStep;
        }
        Phase::OldLawStep
            if clock.accepted_steps >= (WARMUP_STEPS + 1) as u32
                && display.full_snapshot_completed_steps() > WARMUP_STEPS =>
        {
            request
                .queue_live_event(
                    &mut assets,
                    pending
                        .accepted_event
                        .take()
                        .expect("one accepted temporal event"),
                )
                .expect("queue accepted temporal event");
            expected.phase = Phase::Acceptance;
        }
        Phase::Acceptance
            if request.stats().processed_event() >= 2 && !request.live_event_pending() =>
        {
            if request.stats().event_rejection() != 0
                || clock.accepted_steps != (WARMUP_STEPS + 1) as u32
                || clock.event_serial != 2
                || display.runtime_serials[1] != 2
            {
                eprintln!("valid temporal retry did not commit at the paused boundary");
                expected.failed = true;
                expected.phase = Phase::Done;
                return;
            }
            request.request_steps(TARGET_STEPS);
            expected.phase = Phase::Evolution;
        }
        Phase::Evolution
            if clock.accepted_steps >= (WARMUP_STEPS + 1 + TARGET_STEPS) as u32
                && display.full_snapshot_completed_steps() >= WARMUP_STEPS + 1 + TARGET_STEPS =>
        {
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
            let clock_error = (clock.absolute_seconds - expected.absolute_time).abs();
            println!(
                "temporal event rejection rolled back byte-exactly, resumed one old-law step, then committed retry: Q {:.3e}, b {:.3e}, clock {:.3e}",
                q_error, b_error, clock_error,
            );
            if q_error > 8.0e-5
                || b_error > 8.0e-5
                || clock_error > 2.0e-4 * expected.time_step.max(1.0)
            {
                eprintln!("temporal event rollback/resumption accuracy gate was exceeded");
                expected.failed = true;
            }
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
