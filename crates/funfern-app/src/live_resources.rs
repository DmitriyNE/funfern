//! What the app holds on the device and in its workers, counted for the
//! `lifecycle` fixture (RT-07, and section 5.4 of the 11 October testing
//! plan): the shader buffers in the main world and on the device, the
//! readbacks and their staging, the snapshot's staging, the field paint's
//! buffers, the bind groups each consumer keeps, the pipelines, the entities
//! of both worlds, and the worker threads alive. These are the app's own
//! records, not wgpu's counters, which take a feature of their own and read
//! nothing in the browser.
//!
//! The fixtures binary adds the render world's count ([`add_render_count`]);
//! the shipped app never runs it. The thread count is kept in both, an atomic
//! add as a worker starts and as it ends.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use bevy::{
    ecs::{archetype::Archetypes, resource::IS_RESOURCE},
    prelude::*,
    render::{
        Render, RenderApp, RenderSystems,
        render_asset::RenderAssets,
        render_resource::{Buffer, PipelineCache},
        storage::GpuShaderBuffer,
    },
};

/// How many of a kind of buffer, and their bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Tally {
    pub(crate) count: usize,
    pub(crate) bytes: u64,
}

impl Tally {
    pub(crate) fn add(&mut self, buffer: &Buffer) {
        self.count += 1;
        self.bytes += buffer.size();
    }
}

impl std::fmt::Display for Tally {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({} B)", self.count, self.bytes)
    }
}

/// What the render world holds, as one frame's cleanup found it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RenderCounts {
    /// The render frame it was counted at, which only grows.
    pub(crate) frame: u64,
    /// The shader buffers on the device.
    pub(crate) device_buffers: Tally,
    /// The readbacks' staging pool, and the copies in flight through it,
    /// which vary with the frame and are reported, not compared.
    pub(crate) readback_staging: Tally,
    pub(crate) copies_in_flight: usize,
    /// The full-state snapshot's own staging buffer.
    pub(crate) snapshot_staging: Tally,
    /// The field paint's vertex, index and uniform buffers.
    pub(crate) paint_buffers: Tally,
    /// Each bind group kept, by its holder, and how many it holds.
    pub(crate) bind_groups: Vec<(&'static str, usize)>,
    pub(crate) pipelines: usize,
    /// The entities, resources' aside ([`entities`]).
    pub(crate) entities: u32,
}

impl RenderCounts {
    /// Records a holder's bind groups, if it holds any.
    pub(crate) fn bind_groups(&mut self, holder: &'static str, count: usize) {
        if count > 0 {
            self.bind_groups.push((holder, count));
        }
    }

    pub(crate) fn bind_group_count(&self) -> usize {
        self.bind_groups.iter().map(|(_, count)| count).sum()
    }
}

/// The render world's latest count, shared with the main world's fixture.
#[derive(Resource, Clone, Default)]
pub(crate) struct RenderCount(pub(crate) Arc<Mutex<RenderCounts>>);

/// Counts the render world each frame from here on, into what this returns.
pub(crate) fn add_render_count(app: &mut App) -> RenderCount {
    let count = RenderCount::default();
    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app
            .insert_resource(count.clone())
            .add_systems(Render, count_render_world.in_set(RenderSystems::Cleanup));
    }
    count
}

fn count_render_world(world: &World, count: Res<RenderCount>, mut frame: Local<u64>) {
    *frame += 1;
    let mut counts = RenderCounts {
        frame: *frame,
        ..RenderCounts::default()
    };
    if let Some(buffers) = world.get_resource::<RenderAssets<GpuShaderBuffer>>() {
        for (_, buffer) in buffers.iter() {
            counts.device_buffers.add(&buffer.buffer);
        }
    }
    crate::paced_readback::count_live(world, &mut counts);
    crate::canonical_gpu::count_live(world, &mut counts);
    crate::wave_gpu::count_live(world, &mut counts);
    crate::field_paint::count_live(world, &mut counts);
    counts.pipelines = world
        .get_resource::<PipelineCache>()
        .map_or(0, |cache| cache.pipelines().count());
    counts.entities = entities(world.archetypes());
    *count.0.lock().unwrap() = counts;
}

/// The entities a world holds, besides its resources'. Bevy keeps each
/// resource in an entity of its own, and keeps the entity once the resource
/// is removed ("Resource entities are not supposed to be despawned"), so a
/// world holds one for every kind of resource it has ever held: the render
/// world held six of them where only the step's and the field paint's bind
/// groups had been made, and ten once every holder had held one.
pub(crate) fn entities(archetypes: &Archetypes) -> u32 {
    archetypes
        .iter()
        .filter(|archetype| !archetype.contains(IS_RESOURCE))
        .map(|archetype| archetype.len())
        .sum()
}

/// The app's worker threads alive: the preparation and adaptation workers
/// and the GPU packs, natively and in the threaded browser build.
static LIVE_THREADS: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn live_threads() -> usize {
    LIVE_THREADS.load(Ordering::Acquire)
}

/// Held by a worker thread for its life: counted alive from here until it
/// drops, as the thread returns or unwinds. A browser build without threads
/// has none, and its count stays at zero.
#[cfg(any(
    not(target_arch = "wasm32"),
    all(target_arch = "wasm32", feature = "browser-threads")
))]
pub(crate) struct LiveThread(());

#[cfg(any(
    not(target_arch = "wasm32"),
    all(target_arch = "wasm32", feature = "browser-threads")
))]
pub(crate) fn live_thread() -> LiveThread {
    LIVE_THREADS.fetch_add(1, Ordering::AcqRel);
    LiveThread(())
}

#[cfg(any(
    not(target_arch = "wasm32"),
    all(target_arch = "wasm32", feature = "browser-threads")
))]
impl Drop for LiveThread {
    fn drop(&mut self) {
        LIVE_THREADS.fetch_sub(1, Ordering::AcqRel);
    }
}
