#[allow(dead_code)]
mod canonical_gpu;
mod capture;
mod field_paint;
#[allow(dead_code)]
mod files;
mod material_overlay;
mod paced_readback;
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
    .add_plugins(field_paint::FieldPaintPlugin)
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
fn set_browser_worker_status(attribute: &str, status: &str) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(root) = document.document_element() else {
        return;
    };
    let _ = root.set_attribute(attribute, status);
}

#[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
pub(crate) fn set_browser_preparation_worker_status(status: &str) {
    set_browser_worker_status("data-funfern-preparation-worker", status);
}

#[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
pub(crate) fn set_browser_amr_worker_status(status: &str) {
    set_browser_worker_status("data-funfern-amr-worker", status);
}

#[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
pub(crate) fn set_browser_amr_job_status(status: &str) {
    set_browser_worker_status("data-funfern-amr-job", status);
}

#[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
pub(crate) fn set_browser_gpu_pack_status(status: &str) {
    set_browser_worker_status("data-funfern-gpu-pack", status);
}

#[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
fn main() {
    set_browser_preparation_worker_status("initializing");
    set_browser_amr_worker_status("initializing");
    set_browser_amr_job_status("waiting");
    set_browser_gpu_pack_status("waiting");
    wasm_bindgen_futures::spawn_local(async {
        // Topology preparation and AMR each own a permanent receiver loop. A
        // third execution slot gives dependent GPU-plan packing the same
        // off-UI-thread placement it has natively; the two receiver loops do
        // not return to Rayon between jobs and therefore cannot lend it a slot.
        const BACKGROUND_WORKERS: usize = 3;
        let worker_ready = wasm_bindgen_futures::JsFuture::from(
            wasm_bindgen_rayon::init_thread_pool(BACKGROUND_WORKERS),
        )
        .await
        .is_ok();
        ui::set_browser_background_pool_ready(worker_ready);
        set_browser_preparation_worker_status(if worker_ready { "ready" } else { "unavailable" });
        set_browser_amr_worker_status(if worker_ready { "ready" } else { "unavailable" });
        run_app();
    });
}

#[cfg(all(target_arch = "wasm32", not(feature = "browser-threads")))]
fn main() {
    run_app();
}
