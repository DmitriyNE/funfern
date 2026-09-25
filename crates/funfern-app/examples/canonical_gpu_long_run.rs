//! A gallery scene stepped from rest on the device against the f64 reference,
//! for many more steps than the other gates take.
//!
//! The other gates run 48-200 steps, and every one passed while the device's
//! epoch-local time was an f32 running sum: each step rounded at the spacing
//! of `t` and kept the error, 8e-6 s behind by step 400 and 0.12 s across a
//! full epoch. Sources, drives, pins and Switches all read that time, so a
//! sourced scene drifted from the reference as the error compounded - 6e-5
//! at 400 steps against the Stage 0 3e-5. The local time is now formed from
//! the step count each step, and this gate keeps it that way.
//!
//! `LONG_RUN_SCENE` names a catalogue scene (default "Parametric pump") and
//! `LONG_RUN_STEPS` the step count (default 400). `LONG_RUN_LOSS` puts an
//! electric loss of 0.6 on the scene's second material: `none` a constant
//! one, a number a pump of that depth on the rate. On the pumped slab both
//! failed before the device read its loss records: the constant one because a
//! node on the slab's edge weighs the two materials' rates by masses the pump
//! moves (3.2e-4 at 200 steps), the driven one because its drive was dropped
//! (2.3e-2). The Stage 0 bound is
//! asserted at up to 1000 steps. Past that, a driven or field-dependent
//! scene's own sensitivity lets the f32 trajectory part from the f64 one
//! faster than roundoff alone, and the figure is printed, not judged.
//!
//! A scene with a loss or gain rate past 0.02 per half step is held to 1e-4
//! instead. The device forms `e^z − 1` there as `exp(z) − 1`, which is off by
//! up to half an f32 unit of one with the same sign every stage, so its
//! multipliers drift from the reference's by about 1.2e-7 a step. That is a
//! rate error of a few 1e-5/s, which nothing on screen shows, but it
//! compounds against the reference until the dynamics forget it: the
//! self-sustained emitter's gain of 15 reads 5.2e-5 at 500 steps while it
//! grows, and 1.5e-5 by 1000 once it has saturated. Not fully accurate; it
//! is under "Worth checking sometime" in `docs/plan.md`.

use bevy::{app::AppExit, prelude::*, render::storage::ShaderBuffer};
use funfern_app::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuPlan, CanonicalGpuRequest,
    CanonicalWaveGpuPlugin,
};
use funfern_app::topology_editor::TopologyEditor;
use funfern_app::topology_runtime::TopologyRuntime;
use funfern_app::wave_gpu::WaveGpuPlugin;
use funfern_core::{CanonicalTemporalWaveState, CanonicalWaveState, MeshingOptions, Point2};
use std::time::{Duration, Instant};
fn steps() -> u64 {
    std::env::var("LONG_RUN_STEPS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(400)
}
#[derive(Resource)]
struct Pending {
    plan: Option<CanonicalGpuPlan>,
}
#[derive(Resource)]
struct Expected {
    primary: Vec<f64>,
    complementary: Vec<Point2>,
    bound: f64,
    started: Instant,
    deadline: Instant,
    finished: bool,
    failed: bool,
}
fn main() -> AppExit {
    let name = std::env::var("LONG_RUN_SCENE").unwrap_or_else(|_| "Parametric pump".into());
    let document = funfern_app::topology_examples::catalog()
        .iter()
        .find(|example| example.name == name)
        .unwrap_or_else(|| panic!("no catalogue scene named {name:?}"))
        .document
        .clone();
    let mut document = document;
    if let Ok(loss) = std::env::var("LONG_RUN_LOSS") {
        let drive = match loss.as_str() {
            "none" => funfern_core::TimeDrive::None,
            depth => funfern_core::TimeDrive::ParametricPump {
                depth: funfern_core::ScalarField::constant(depth.parse().expect("a loss depth")),
                frequency_hz: funfern_core::ScalarField::constant(1.3),
                phase_radians: funfern_core::ScalarField::constant(0.0),
            },
        };
        for scene in [&mut document.model.draft, &mut document.model.accepted] {
            scene.materials[1].electric_loss = Some(funfern_core::LossChannel {
                base_rate: funfern_core::ScalarField::constant(0.6),
                law: funfern_core::DampingLaw {
                    rate: funfern_core::RateLaw::Constant,
                    drive: drive.clone(),
                },
            });
        }
    }
    let edge = document.presentation.mesh_edge;
    let editor = TopologyEditor::from_document(document).unwrap();
    let mut runtime = TopologyRuntime::default();
    let token = runtime
        .request(
            editor.revision,
            &editor.document,
            editor.compiled_accepted.clone(),
            MeshingOptions {
                target_edge_length: edge,
                curve_tolerance: (edge * 0.02).min(5e-4),
                ..MeshingOptions::default()
            },
            true,
        )
        .unwrap();
    let prepared = loop {
        if let Some(r) = runtime.advance(1 << 16) {
            r.unwrap();
            break runtime.commit_ready(token).unwrap();
        }
    };
    let forcing = prepared.canonical_forcing.clone();
    let mut fastest_rate = 0.0_f64;
    let (plan, primary, complementary) =
        if let Some(op) = prepared.canonical_temporal_operator.clone() {
            let dt = prepared.recommended_time_step();
            fastest_rate = op
                .primary_loss_samples()
                .chain(op.complementary_loss_samples())
                .map(|sample| sample.base_rate.abs())
                .fold(0.0, f64::max)
                * 0.5
                * dt;
            let seeded = CanonicalTemporalWaveState::zero(&op, dt)
                .unwrap()
                .pinned(&op, &forcing)
                .unwrap();
            let plan = CanonicalGpuPlan::compile_temporal(
                &op,
                &seeded,
                &forcing,
                CanonicalGpuClock::initial(dt).unwrap(),
            )
            .unwrap();
            let mut oracle = seeded;
            for _ in 0..steps() {
                oracle.step_with_forcing(&op, &forcing).unwrap();
            }
            println!(
                "long run {name:?}: temporal, {} dofs, dt {dt:.4e}, {} steps, fastest rate \
                 {fastest_rate:.3} per half step",
                op.base().degrees_of_freedom(),
                steps()
            );
            (
                plan,
                oracle.primary_flux().to_vec(),
                oracle.complementary_flux().to_vec(),
            )
        } else {
            let base = prepared.canonical_operator.clone();
            let dt = prepared.recommended_time_step();
            let seeded = CanonicalWaveState::zero(&base, dt).unwrap();
            let plan = CanonicalGpuPlan::compile(
                &base,
                &seeded,
                &forcing,
                CanonicalGpuClock::initial(dt).unwrap(),
            )
            .unwrap();
            let mut oracle = seeded;
            for _ in 0..steps() {
                oracle.step_with_forcing(&base, &forcing).unwrap();
            }
            println!(
                "long run {name:?}: fixed, {} dofs, dt {dt:.4e}, {} steps",
                base.degrees_of_freedom(),
                steps()
            );
            (
                plan,
                oracle.primary_flux().to_vec(),
                oracle.complementary_flux().to_vec(),
            )
        };
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "p".into(),
            visible: false,
            resolution: (64, 64).into(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(WaveGpuPlugin)
    .add_plugins(CanonicalWaveGpuPlugin)
    .insert_resource(Pending { plan: Some(plan) })
    .insert_resource(Expected {
        primary,
        complementary,
        bound: if fastest_rate > 0.02 { 1.0e-4 } else { 3.0e-5 },
        started: Instant::now(),
        deadline: Instant::now() + Duration::from_secs(300),
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
    canonical.request_steps(steps());
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
        eprintln!("long run timed out or failed");
        expected.failed = true;
        expected.finished = true;
        return;
    }
    if request.stats().completed_steps() < steps() {
        return;
    }
    if !request.request_full_state_readback(&mut commands) && display.full_readbacks == 0 {
        return;
    }
    // Both lanes, or the complementary comparison below runs before the
    // readback has populated that lane and passes by measuring nothing.
    if display.primary_flux.len() != expected.primary.len()
        || display.complementary_flux.len() != expected.complementary.len()
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
    println!(
        "long run after {:.2} ms: Q {primary:.3e}, b {complementary:.3e}",
        expected.started.elapsed().as_secs_f64() * 1_000.0
    );
    expected.finished = true;
    // The Stage 0 f32 gate, where it is a claim.
    if steps() <= 1000 && (primary > expected.bound || complementary > expected.bound) {
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
