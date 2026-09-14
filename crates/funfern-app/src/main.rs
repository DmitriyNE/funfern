mod capture;
#[allow(dead_code)]
mod files;
mod material_overlay;
mod recording;
mod recovery;
#[allow(dead_code)]
mod sharing;
mod ui;
#[allow(dead_code)]
mod wave_gpu;
use bevy::prelude::*;
use bevy_egui::{EguiPlugin, EguiPrimaryContextPass};
fn main() {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "funfern · wave playground".into(),
            canvas: Some("#funfern".into()),
            fit_canvas_to_parent: true,
            resolution: (1280, 800).into(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(EguiPlugin::default())
    .add_plugins(wave_gpu::WaveGpuPlugin)
    .init_resource::<ui::Playground>()
    .add_systems(Startup, |mut commands: Commands| {
        commands.spawn(Camera2d);
    })
    .add_systems(EguiPrimaryContextPass, ui::frame);
    app.run();
}
