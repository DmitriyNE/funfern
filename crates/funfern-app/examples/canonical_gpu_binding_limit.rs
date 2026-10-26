//! A plan past the device's storage-binding limit, refused before upload.
//!
//! The device is made to bind half the largest buffer of the cavity's plan
//! at edge 0.08, as a smaller device or a browser's default would. That plan
//! must be refused, its buffer named, before any of its buffers is made; a
//! coarser plan that fits must install and step. Installed, the larger plan's
//! bind group failed validation and rendering stopped for good (GPU-7 of the
//! 2026-10-20 review). The limit the request admits against must be the
//! device's own, copied in as the app starts.

#[path = "support/check.rs"]
mod check;
use std::time::Instant;

use bevy::{
    app::AppExit,
    prelude::*,
    render::{
        RenderPlugin,
        settings::{WgpuLimits, WgpuSettings},
        storage::ShaderBuffer,
    },
};
use funfern_app::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuPlan, CanonicalGpuRequest,
    CanonicalWaveGpuPlugin,
};
use funfern_core::TopologyWaveModel;
use funfern_core::test_scenes::*;
use funfern_core::{
    CanonicalForcing, CanonicalWaveOperator, CanonicalWaveState, MeshingOptions,
    OuterBoundaryCondition,
};

const STEPS: u64 = 16;

#[derive(Resource)]
struct Pending {
    refused: CanonicalGpuPlan,
    admitted: Option<CanonicalGpuPlan>,
    limit: u64,
    deadline: Instant,
    failed: bool,
}

/// The cavity's plan at `edge`, and its largest buffer's size.
fn cavity_plan(edge: f64) -> (CanonicalGpuPlan, u64) {
    let scene = initial();
    let mesh = meshed(
        &scene,
        1,
        MeshingOptions {
            target_edge_length: edge,
            ..MeshingOptions::default()
        },
    )
    .expect("cavity mesh");
    let scalar = assembled(&mesh, &scene, OuterBoundaryCondition::Reflecting)
        .expect("cavity scalar operator");
    let operator = CanonicalWaveOperator::compile(
        &mesh,
        &scalar,
        TopologyWaveModel::from_topology_scene(&scene),
        1,
    )
    .expect("cavity canonical operator");
    let time_step = 0.5 * operator.maximum_time_step();
    let plan = CanonicalGpuPlan::compile_with_quadratic(
        &operator,
        &scalar,
        &CanonicalWaveState::zero(&operator, time_step).expect("cavity state"),
        &CanonicalForcing::none(&operator),
        CanonicalGpuClock::initial(time_step).expect("cavity clock"),
    )
    .expect("cavity GPU plan");
    let largest = plan
        .buffer_bytes()
        .into_iter()
        .map(|(_, bytes)| bytes as u64)
        .max()
        .expect("a plan has buffers");
    (plan, largest)
}

fn main() -> AppExit {
    let (refused, refused_largest) = cavity_plan(0.08);
    let (admitted, admitted_largest) = cavity_plan(0.24);
    let limit = refused_largest / 2;
    assert!(admitted_largest <= limit, "the coarse plan must fit");
    println!(
        "the device binds {limit} B; the plans' largest buffers hold {refused_largest} and \
         {admitted_largest} B"
    );
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "funfern canonical GPU binding-limit validation".into(),
                    visible: false,
                    resolution: (64, 64).into(),
                    ..default()
                }),
                ..default()
            })
            .set(RenderPlugin {
                render_creation: WgpuSettings {
                    constrained_limits: Some(WgpuLimits {
                        max_storage_buffer_binding_size: limit,
                        ..WgpuLimits::default()
                    }),
                    ..default()
                }
                .into(),
                ..default()
            }),
    )
    .add_plugins(CanonicalWaveGpuPlugin)
    .insert_resource(Pending {
        refused,
        admitted: Some(admitted),
        limit,
        deadline: check::deadline(60),
        failed: false,
    })
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
    commands.spawn(Camera2d);
    let limits = request.limits();
    if limits.map(|limits| limits.max_storage_buffer_binding_size) != Some(pending.limit) {
        eprintln!("the request admits against {limits:?}, not the device's own limit");
        pending.failed = true;
        return;
    }
    match request.admit(&pending.refused, None) {
        Err(refusal) => println!("refused: {refusal}"),
        Ok(()) => {
            eprintln!("a plan past the device's limit was admitted");
            pending.failed = true;
            return;
        }
    }
    let admitted = pending.admitted.take().expect("the plan that fits");
    if let Err(refusal) = request.admit(&admitted, None) {
        eprintln!("a plan within the device's limit was refused: {refusal}");
        pending.failed = true;
        return;
    }
    request.install(&mut assets, &mut commands, admitted);
    request.request_steps(STEPS);
}

fn validate(
    request: Res<CanonicalGpuRequest>,
    display: Res<CanonicalGpuDisplay>,
    pending: Res<Pending>,
    mut exit: MessageWriter<AppExit>,
) {
    if pending.failed {
        exit.write(AppExit::error());
        return;
    }
    if request.stats().failure() != 0 || Instant::now() >= pending.deadline {
        eprintln!("the plan that fits did not step");
        exit.write(AppExit::error());
        return;
    }
    if display
        .clock
        .is_some_and(|clock| clock.accepted_steps >= STEPS)
    {
        println!("the plan that fits stepped {STEPS} steps under the device's limit");
        exit.write(AppExit::Success);
    }
}
