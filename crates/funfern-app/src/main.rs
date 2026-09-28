// The threaded web build is nightly, for `-Zbuild-std`; an allocation failure
// reaches no panic hook, so it takes its own (`record_worker_aborts`).
#![cfg_attr(
    all(target_arch = "wasm32", feature = "browser-threads"),
    feature(alloc_error_hook)
)]
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
    }));
    #[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
    record_worker_aborts();
    app.add_plugins(EguiPlugin::default())
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

/// Records a background worker's abort where the main thread reads it
/// (`ui::BROWSER_BACKGROUND_FAILURE`). With `panic = "abort"` a pool worker
/// that panics or fails an allocation traps, and nothing outside it hears:
/// the pool's message handler is async, so the trap is only an unhandled
/// rejection inside the worker. Both hooks record before they allocate.
///
/// The panic hook is chained, so it goes in once `DefaultPlugins` have set
/// theirs: on the web Bevy's replaces whatever was there.
#[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
fn record_worker_aborts() {
    use std::sync::atomic::Ordering;
    std::alloc::set_alloc_error_hook(|layout| {
        ui::BROWSER_BACKGROUND_FAILURE.store(2, Ordering::Release);
        web_sys::console::error_2(
            &"funfern: an allocation failed, bytes:".into(),
            &(layout.size() as f64).into(),
        );
    });
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = ui::BROWSER_BACKGROUND_FAILURE.compare_exchange(
            0,
            1,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        web_sys::console::error_1(&format!("funfern: {info}").into());
        previous(info);
    }));
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
        let worker_ready = if shared_memory_growth_visible().await {
            wasm_bindgen_futures::JsFuture::from(wasm_bindgen_rayon::init_thread_pool(
                BACKGROUND_WORKERS,
            ))
            .await
            .is_ok()
        } else {
            web_sys::console::warn_1(
                &"funfern: this browser's main thread does not see a worker's growth of shared \
                  memory, so the background pool stays off and its work runs on the main thread"
                    .into(),
            );
            false
        };
        ui::set_browser_background_pool_ready(worker_ready);
        set_browser_preparation_worker_status(if worker_ready { "ready" } else { "unavailable" });
        set_browser_amr_worker_status(if worker_ready { "ready" } else { "unavailable" });
        run_app();
    });
}

/// The page's check of whether this browser's main thread sees a worker grow a
/// shared memory (see `index.html`). Where it does not, the pool's workers
/// growing the memory as they start make the main thread's next bulk copy trap
/// inside the allocator. A page without the check starts the pool as before.
#[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
async fn shared_memory_growth_visible() -> bool {
    let Some(window) = web_sys::window() else {
        return true;
    };
    let check = js_sys::Reflect::get(&window, &"funfernSharedMemoryGrowthVisible".into())
        .unwrap_or(wasm_bindgen::JsValue::UNDEFINED);
    if check.is_undefined() {
        return true;
    }
    wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&check))
        .await
        .is_ok_and(|visible| visible.as_bool() == Some(true))
}

#[cfg(all(target_arch = "wasm32", not(feature = "browser-threads")))]
fn main() {
    run_app();
}
