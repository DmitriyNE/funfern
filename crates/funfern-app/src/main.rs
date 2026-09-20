#[allow(dead_code)]
mod canonical_gpu;
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

fn run_app() {
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
    .add_plugins(canonical_gpu::CanonicalWaveGpuPlugin)
    .init_resource::<ui::Playground>()
    .add_systems(Startup, |mut commands: Commands| {
        commands.spawn(Camera2d);
    })
    .add_systems(EguiPrimaryContextPass, ui::frame);
    app.run();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    run_app();
}

#[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
pub(crate) fn set_browser_preparation_worker_status(status: &str) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(root) = document.document_element() else {
        return;
    };
    let _ = root.set_attribute("data-funfern-preparation-worker", status);
}

#[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
fn main() {
    set_browser_preparation_worker_status("initializing");
    wasm_bindgen_futures::spawn_local(async {
        let worker_ready =
            wasm_bindgen_futures::JsFuture::from(wasm_bindgen_rayon::init_thread_pool(1))
                .await
                .is_ok();
        ui::set_browser_preparation_worker_ready(worker_ready);
        set_browser_preparation_worker_status(if worker_ready { "ready" } else { "unavailable" });
        run_app();
    });
}

#[cfg(all(target_arch = "wasm32", not(feature = "browser-threads")))]
fn main() {
    run_app();
}
