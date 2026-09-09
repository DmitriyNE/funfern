mod files;
mod ui;
use bevy::prelude::*;
use bevy_egui::{EguiPlugin, EguiPrimaryContextPass};
fn main() {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "femfun · geometry playground".into(),
            canvas: Some("#femfun".into()),
            fit_canvas_to_parent: true,
            resolution: (1280, 800).into(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(EguiPlugin::default())
    .init_resource::<ui::Playground>()
    .add_systems(Startup, |mut commands: Commands| {
        commands.spawn(Camera2d);
    })
    .add_systems(EguiPrimaryContextPass, ui::frame);
    #[cfg(not(target_arch = "wasm32"))]
    if std::env::args().any(|arg| arg == "--mesh-edit-benchmark") {
        app.insert_resource(ui::mesh_benchmark_scene())
            .init_resource::<ui::MeshBenchmark>()
            .add_systems(EguiPrimaryContextPass, ui::mesh_benchmark.after(ui::frame));
    }
    app.run();
}
