//! Real-device validation of temporal-work accounting.
//!
//! A driven material pumps power into the field, and reporting that honestly
//! needs the accepted runtime the solver is actually using. A Switch origin
//! is stamped at a GPU commit boundary, so the host cannot reconstruct it
//! from the clock. The bank therefore travels in the state buffer itself,
//! published by the same commit that wrote the fields around it; a
//! separately arriving copy would be status rather than snapshot identity.
//!
//! The fixture stamps a Switch, evolves past it, then compares the decoded
//! runtime and the energy breakdown against the f64 oracle.

use std::time::{Duration, Instant};

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuLiveEvent, CanonicalGpuPlan,
    CanonicalGpuRequest, CanonicalWaveGpuPlugin,
};
use funfern_app::wave_gpu::WaveGpuPlugin;
use funfern_core::{
    CanonicalTemporalWaveOperator, CanonicalTemporalWaveState, CoefficientLaw, MaterialId,
    MeshingOptions, OuterBoundaryCondition, QuadraticWaveOperator, ScalarField, Scene, TimeDrive,
    canonical_temporal_energy_breakdown, mesh_scene,
};

const WARMUP_STEPS: u64 = 24;
const TOTAL_STEPS: u64 = 60;
const SWITCH_DURATION: f64 = 0.9;

#[derive(Resource)]
struct Pending {
    plan: Option<CanonicalGpuPlan>,
    switch_event: Option<CanonicalGpuLiveEvent>,
}

#[derive(Resource)]
struct Expected {
    operator: CanonicalTemporalWaveOperator,
    material: MaterialId,
    epoch_origin: f64,
    time: f64,
    switch_start_time: f64,
    primary_energy: f64,
    complementary_energy: f64,
    temporal_power: f64,
    queued: bool,
    started: Instant,
    deadline: Instant,
    finished: bool,
    failed: bool,
}

fn main() -> AppExit {
    let mut scene = Scene::initial();
    scene.materials[0].mass_law.drive = TimeDrive::ParametricPump {
        depth: ScalarField::constant(0.26),
        frequency_hz: ScalarField::constant(0.73),
        phase_radians: ScalarField::constant(0.18),
    };
    scene.materials[0].mass_law.alternate = Some(ScalarField::constant(1.35));
    scene.materials[0].stiffness_law.drive = TimeDrive::TimeCrystal {
        depth: ScalarField::constant(0.14),
        frequency_hz: ScalarField::constant(0.58),
        phase_radians: ScalarField::constant(-0.24),
        sharpness: ScalarField::constant(2.8),
    };
    scene.materials[0].switch_ramp = SWITCH_DURATION;

    let mut fixed_scene = scene.clone();
    for material in &mut fixed_scene.materials {
        material.mass_law = CoefficientLaw::linear();
        material.stiffness_law = CoefficientLaw::linear();
    }
    let mesh = mesh_scene(
        &fixed_scene,
        1,
        MeshingOptions {
            target_edge_length: 0.2,
            ..MeshingOptions::default()
        },
    )
    .expect("temporal-work mesh");
    let scalar = QuadraticWaveOperator::assemble_scene(
        &mesh,
        &fixed_scene,
        OuterBoundaryCondition::Reflecting,
    )
    .expect("temporal-work scalar operator");
    let operator = CanonicalTemporalWaveOperator::compile_scene(&mesh, &scalar, &scene, 1)
        .expect("temporal-work operator");
    let material = operator.initial_runtime().records()[0].material();
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
        .expect("compatible temporal-work flux");
    let state = CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary)
        .expect("temporal-work state");
    let gpu_state = state.clone();
    let switch_event =
        CanonicalGpuLiveEvent::temporal_switch(&state, material, true, SWITCH_DURATION, 1)
            .expect("Switch event");

    // The oracle stamps the Switch at the same accepted boundary the GPU will.
    let mut oracle = state;
    for _ in 0..WARMUP_STEPS {
        oracle.step(&operator).expect("f64 warmup step");
    }
    let switch_start_time = oracle.time();
    oracle
        .runtime_mut()
        .begin_switch(material, true, switch_start_time, SWITCH_DURATION)
        .expect("f64 Switch");
    for _ in WARMUP_STEPS..TOTAL_STEPS {
        oracle.step(&operator).expect("f64 step past the Switch");
    }
    let breakdown = canonical_temporal_energy_breakdown(
        &operator,
        oracle.primary_flux(),
        oracle.complementary_flux(),
        oracle.time(),
        oracle.runtime(),
    )
    .expect("f64 temporal energy breakdown");
    assert!(
        breakdown.temporal_power.abs() > 1.0e-6,
        "the fixture must actually pump"
    );

    let clock = CanonicalGpuClock::initial(time_step).expect("temporal-work clock");
    let epoch_origin = clock.epoch_origin_seconds;
    let plan = CanonicalGpuPlan::compile_temporal_bulk(&operator, &gpu_state, clock)
        .expect("temporal-work GPU plan");
    println!(
        "temporal-work gate: {} Q, {} b, Switch at {switch_start_time:.6e} over {SWITCH_DURATION}s",
        plan.node_count, plan.sample_count
    );

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern temporal-work validation".into(),
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
        switch_event: Some(switch_event),
    })
    .insert_resource(Expected {
        operator,
        material,
        epoch_origin,
        time: oracle.time(),
        switch_start_time,
        primary_energy: breakdown.primary,
        complementary_energy: breakdown.complementary,
        temporal_power: breakdown.temporal_power,
        queued: false,
        started: Instant::now(),
        deadline: Instant::now() + Duration::from_secs(60),
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
    canonical.install(
        &mut assets,
        &mut commands,
        pending.plan.take().expect("one plan"),
    );
    canonical.request_steps(WARMUP_STEPS);
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
    if expected.finished {
        exit.write(if expected.failed {
            AppExit::error()
        } else {
            AppExit::Success
        });
        return;
    }
    if Instant::now() >= expected.deadline || request.stats().failure() != 0 {
        eprintln!("temporal-work validation timed out or failed");
        expected.finished = true;
        expected.failed = true;
        exit.write(AppExit::error());
        return;
    }

    if !expected.queued {
        let Some(clock) = display.clock else { return };
        if (clock.accepted_steps as u64) < WARMUP_STEPS || request.live_event_pending() {
            return;
        }
        let event = pending.switch_event.take().expect("one Switch event");
        request
            .queue_live_event(&mut assets, event)
            .expect("queue Switch");
        request.request_steps(TOTAL_STEPS - WARMUP_STEPS);
        expected.queued = true;
        return;
    }

    if request.stats().completed_steps() < TOTAL_STEPS {
        return;
    }
    if !request.request_full_state_readback(&mut commands) && display.full_readbacks == 0 {
        return;
    }
    let Some(runtime) =
        display.material_runtime(&expected.operator.initial_runtime(), expected.epoch_origin)
    else {
        return;
    };
    let record = runtime
        .records()
        .iter()
        .find(|record| record.material() == expected.material)
        .expect("the switched material");
    let switch = record.switch();

    let primary = display
        .primary_flux
        .iter()
        .map(|value| f64::from(*value))
        .collect::<Vec<_>>();
    let complementary = display
        .complementary_flux
        .iter()
        .map(|value| funfern_core::Point2::new(f64::from(value[0]), f64::from(value[1])))
        .collect::<Vec<_>>();
    if primary.len() != expected.operator.base().degrees_of_freedom() {
        return;
    }
    let Ok(breakdown) = canonical_temporal_energy_breakdown(
        &expected.operator,
        &primary,
        &complementary,
        expected.time,
        &runtime,
    ) else {
        return;
    };

    let errors = [
        relative_error(switch.start_time(), expected.switch_start_time),
        relative_error(switch.duration(), SWITCH_DURATION),
        relative_error(breakdown.primary, expected.primary_energy),
        relative_error(breakdown.complementary, expected.complementary_energy),
        relative_error(breakdown.temporal_power, expected.temporal_power),
    ];
    println!(
        "temporal-work errors after {:.2} ms: switch start {:.3e}, duration {:.3e}, primary energy {:.3e}, complementary energy {:.3e}, pump power {:.3e}",
        expected.started.elapsed().as_secs_f64() * 1_000.0,
        errors[0],
        errors[1],
        errors[2],
        errors[3],
        errors[4]
    );
    println!(
        "  switch blend {:.4} -> {:.4}, pump power {:.6e}",
        switch.start_blend(),
        switch.target_blend(),
        breakdown.temporal_power
    );
    expected.finished = true;
    if errors.into_iter().any(|error| error > 1.0e-3) {
        expected.failed = true;
        exit.write(AppExit::error());
    } else {
        exit.write(AppExit::Success);
    }
}

fn relative_error(actual: f64, expected: f64) -> f64 {
    (actual - expected).abs() / expected.abs().max(1.0e-8)
}
