//! The application's GPU readbacks: a buffer, or a range of one, copied to a
//! staging buffer after each frame's work, mapped once the frame is
//! submitted, and delivered to the main world as a [`PacedReadbackComplete`]
//! event on the entity that asked, with the bytes.
//!
//! Bevy's own `Readback` did this, under the backpressure this module added:
//! at most [`IN_FLIGHT_DEPTH`] copies per entity in flight, where Bevy
//! schedules a new copy every render frame and an expensive solver or a
//! backgrounded native window could leave thousands of copies and Metal
//! command buffers outstanding. Bevy's mapping, though, `expect`s the map to
//! succeed, and on a lost device every copy in flight fails to map, so the
//! loss was a panic inside the render schedule before Bevy's own error
//! handler could see it: natively an error exit with a trace, in the browser
//! a trap the page took for an exhausted memory. Here a map that fails is a
//! copy that never arrives - its slot freed, its staging buffer returned -
//! and the loss reaches the handler that stops rendering and says so
//! (`stopped`). Bevy's plugin stays registered, with nothing to serve.
//!
//! The pipeline, a frame: [`request_copies`] claims a slot and a staging
//! buffer for each readback whose source is prepared; [`encode_copies`]
//! records the copies after the frame's rendering; [`map_copies`] asks for
//! the maps once the frame is submitted; and [`deliver_copies`], at the next
//! extraction, triggers the events for the maps that completed and returns
//! their staging buffers. A copy's round trip is two frames or more, as it
//! was with Bevy's.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, Ordering},
};

use bevy::{
    core_pipeline::schedule::camera_driver,
    platform::collections::HashMap,
    prelude::*,
    render::{
        ExtractSchedule, MainWorld, Render, RenderApp, RenderSystems,
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        render_asset::RenderAssets,
        render_resource::{
            Buffer, BufferDescriptor, BufferUsages, MapMode, ShaderType,
            encase::internal::{ReadFrom, Reader},
        },
        renderer::{RenderContext, RenderDevice, RenderGraph, RenderGraphSystems},
        storage::{GpuShaderBuffer, ShaderBuffer},
        sync_world::MainEntity,
    },
};

/// Copies of one readback that may be in flight at once.
///
/// A copy's round trip is two frames or more: encoded after the frame's
/// drawing, mapped at a later submission, delivered to the main world at the
/// extraction after that. The picture is painted from the state readback, so
/// the round trip divided by the depth is how often the picture changes.
/// Measured on an M1 Max at 60 Hz (`docs/spikes/funfern-drawn-pacing.md`):
/// with one in flight the picture changed every other frame on every scene,
/// a 30 Hz picture; with two, on 92 % of frames; with three, on 97 %, and on
/// 92 % of a scene whose solver takes the whole frame, where the round trip
/// is three to four frames. The cost is a staging buffer a copy. At 120 Hz
/// four and five did no better than three: there the copies all arrive, but
/// unevenly, which `picture_playout` smooths.
const IN_FLIGHT_DEPTH: u8 = 3;

/// Frames a staging buffer stays in the pool unused before it is dropped.
const UNUSED_FRAMES_KEPT: usize = 10;

/// A buffer, or a range of one, read back every frame the depth allows. The
/// count in flight is shared between the main and render worlds: the render
/// world claims a slot as it requests a copy; the copy's delivery, or its
/// failure, releases it.
#[derive(Component, Clone, ExtractComponent)]
pub(crate) struct PacedReadback {
    buffer: Handle<ShaderBuffer>,
    /// The range read, start and size, or the whole buffer.
    range: Option<(u64, u64)>,
    in_flight: Arc<AtomicU8>,
}

impl PacedReadback {
    /// The whole buffer.
    pub(crate) fn buffer(buffer: Handle<ShaderBuffer>) -> Self {
        Self {
            buffer,
            range: None,
            in_flight: Arc::new(AtomicU8::new(0)),
        }
    }

    /// `size` bytes of the buffer from `start`.
    pub(crate) fn buffer_range(buffer: Handle<ShaderBuffer>, start: u64, size: u64) -> Self {
        Self {
            buffer,
            range: Some((start, size)),
            in_flight: Arc::new(AtomicU8::new(0)),
        }
    }

    #[cfg(test)]
    fn new() -> Self {
        Self::buffer(Handle::default())
    }

    fn try_submit(&self) -> bool {
        self.in_flight
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < IN_FLIGHT_DEPTH).then_some(count + 1)
            })
            .is_ok()
    }

    #[cfg(test)]
    fn complete(&self) {
        complete(&self.in_flight);
    }
}

/// Releases one slot. Saturating: a completion is never counted below none
/// in flight.
fn complete(in_flight: &AtomicU8) {
    let _ = in_flight.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
        Some(count.saturating_sub(1))
    });
}

/// A readback's bytes, on the entity that asked for them. Triggered in the
/// main world at the extraction after the map completed.
#[derive(Event, Debug)]
pub(crate) struct PacedReadbackComplete {
    pub(crate) entity: Entity,
    pub(crate) data: Vec<u8>,
}

impl PacedReadbackComplete {
    /// The bytes as a shader type, as Bevy's event read them.
    pub(crate) fn to_shader_type<T: ShaderType + ReadFrom + Default>(&self) -> T {
        let mut value = T::default();
        let mut reader = Reader::new::<T>(&self.data, 0).expect("a reader over the readback");
        T::read_from(&mut value, &mut reader);
        value
    }
}

/// A copy under way: requested, encoded, then mapped, until delivered.
struct Copy {
    entity: Entity,
    source: Buffer,
    start: u64,
    size: u64,
    staging: Buffer,
    in_flight: Arc<AtomicU8>,
    /// Set by the map's callback: the bytes, or `None` for a map that failed.
    outcome: Arc<Mutex<Option<Option<Vec<u8>>>>>,
}

#[derive(Resource, Default)]
struct Copies {
    requested: Vec<Copy>,
    mapped: Vec<Copy>,
}

struct StagingBuffer {
    buffer: Buffer,
    taken: bool,
    frames_unused: usize,
}

/// Staging buffers by size, reused across frames.
#[derive(Resource, Default)]
struct Staging {
    buffers: HashMap<u64, Vec<StagingBuffer>>,
}

impl Staging {
    fn take(&mut self, device: &RenderDevice, size: u64) -> Buffer {
        let buffers = self.buffers.entry(size).or_default();
        if let Some(staging) = buffers.iter_mut().find(|staging| !staging.taken) {
            staging.taken = true;
            staging.frames_unused = 0;
            return staging.buffer.clone();
        }
        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some("paced readback staging"),
            size,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        buffers.push(StagingBuffer {
            buffer: buffer.clone(),
            taken: true,
            frames_unused: 0,
        });
        buffer
    }

    /// Drops a buffer whose map failed. wgpu keeps a failed map's state on
    /// the buffer, so mapping it again would assert; and the device it
    /// belongs to is most likely lost.
    fn discard(&mut self, buffer: &Buffer) {
        if let Some(buffers) = self.buffers.get_mut(&buffer.size()) {
            buffers.retain(|staging| staging.buffer.id() != buffer.id());
        }
    }

    fn give_back(&mut self, buffer: &Buffer) {
        if let Some(staging) = self.buffers.get_mut(&buffer.size()).and_then(|buffers| {
            buffers
                .iter_mut()
                .find(|staging| staging.buffer.id() == buffer.id())
        }) {
            staging.taken = false;
        }
    }

    fn retire_unused(&mut self) {
        for buffers in self.buffers.values_mut() {
            for staging in buffers.iter_mut() {
                if !staging.taken {
                    staging.frames_unused += 1;
                }
            }
            buffers.retain(|staging| staging.frames_unused < UNUSED_FRAMES_KEPT);
        }
        self.buffers.retain(|_, buffers| !buffers.is_empty());
    }
}

#[derive(Default)]
pub(crate) struct PacedReadbackPlugin;

impl Plugin for PacedReadbackPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<PacedReadback>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<Copies>()
            .init_resource::<Staging>()
            .add_systems(ExtractSchedule, deliver_copies.ambiguous_with_all())
            .add_systems(
                Render,
                (
                    request_copies.in_set(RenderSystems::PrepareResources),
                    map_copies.in_set(RenderSystems::Cleanup),
                ),
            )
            // After the frame's rendering, where the solver's own copies have
            // run, and before the graph submits.
            .add_systems(
                RenderGraph,
                encode_copies
                    .in_set(RenderGraphSystems::Render)
                    .after(camera_driver),
            );
    }
}

/// Claims a slot and a staging buffer for each readback whose source buffer
/// is prepared, within its depth.
fn request_copies(
    device: Res<RenderDevice>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    readbacks: Query<(&MainEntity, &PacedReadback)>,
    mut copies: ResMut<Copies>,
    mut staging: ResMut<Staging>,
) {
    for (entity, readback) in &readbacks {
        let Some(source) = buffers.get(&readback.buffer) else {
            continue;
        };
        let (start, size) = readback.range.unwrap_or((0, source.buffer.size()));
        if size == 0 || start + size > source.buffer.size() {
            // A range past the buffer reads nothing rather than faulting the
            // device: the readback's author gets no bytes and notices.
            continue;
        }
        if !readback.try_submit() {
            continue;
        }
        let staging = staging.take(&device, size);
        copies.requested.push(Copy {
            entity: entity.id(),
            source: source.buffer.clone(),
            start,
            size,
            staging,
            in_flight: readback.in_flight.clone(),
            outcome: Arc::new(Mutex::new(None)),
        });
    }
}

/// Records this frame's copies into the graph's command encoder.
fn encode_copies(mut render_context: RenderContext, copies: Res<Copies>) {
    if copies.requested.is_empty() {
        return;
    }
    let encoder = render_context.command_encoder();
    for copy in &copies.requested {
        encoder.copy_buffer_to_buffer(&copy.source, copy.start, &copy.staging, 0, copy.size);
    }
}

/// Asks for each encoded copy's map, once the frame is submitted. A map that
/// fails - the device lost - is recorded as such and delivers nothing.
fn map_copies(mut copies: ResMut<Copies>) {
    let requested = std::mem::take(&mut copies.requested);
    for copy in requested {
        let staging = copy.staging.clone();
        let outcome = copy.outcome.clone();
        copy.staging
            .slice(..)
            .map_async(MapMode::Read, move |result| {
                let bytes = result.ok().map(|()| {
                    let bytes = staging.slice(..).get_mapped_range().to_vec();
                    staging.unmap();
                    bytes
                });
                *outcome.lock().unwrap() = Some(bytes);
            });
        copies.mapped.push(copy);
    }
}

/// Triggers the events for the maps that completed, on the main world, and
/// frees their slots and staging buffers; a map that failed frees its slot
/// and loses its buffer.
fn deliver_copies(
    mut main_world: ResMut<MainWorld>,
    mut copies: ResMut<Copies>,
    mut staging: ResMut<Staging>,
) {
    copies.mapped.retain(|copy| {
        let Some(outcome) = copy.outcome.lock().unwrap().take() else {
            return true;
        };
        match outcome {
            Some(data) => {
                main_world.trigger(PacedReadbackComplete {
                    entity: copy.entity,
                    data,
                });
                staging.give_back(&copy.staging);
            }
            None => staging.discard(&copy.staging),
        }
        complete(&copy.in_flight);
        false
    });
    staging.retire_unused();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Claims fill the depth and a completion frees exactly one, so a
    /// completion landing in the frame of a claim cannot leave more in flight
    /// than the depth allows.
    #[test]
    fn continuous_readback_rearms_one_claim_per_completion() {
        let paced = PacedReadback::new();
        for _ in 0..IN_FLIGHT_DEPTH {
            assert!(paced.try_submit());
        }
        assert!(!paced.try_submit());
        paced.complete();
        assert!(paced.try_submit());
        assert!(!paced.try_submit());
        for _ in 0..IN_FLIGHT_DEPTH {
            paced.complete();
        }
        paced.complete();
        for _ in 0..IN_FLIGHT_DEPTH {
            assert!(paced.try_submit());
        }
        assert!(!paced.try_submit());
    }

    /// The bytes read as the shader type they were written as.
    #[test]
    fn the_event_reads_its_bytes_as_a_shader_type() {
        let data = [1.5f32, -2.0, 0.25, 8.0]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let event = PacedReadbackComplete {
            entity: Entity::PLACEHOLDER,
            data,
        };
        let values: Vec<f32> = event.to_shader_type();
        assert_eq!(values, [1.5, -2.0, 0.25, 8.0]);
    }
}
