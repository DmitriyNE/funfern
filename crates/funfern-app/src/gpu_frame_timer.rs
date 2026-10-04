//! The GPU's own clock on each frame's solver pass: when it began and ended.
//!
//! Pacing needs to know whether a late frame was the solver's: a frame whose
//! solver pass fits well inside the display's interval was late for some
//! other reason, and cutting the batch cannot bring it back on time. The pass
//! carries timestamp writes; readings come back through a small ring of
//! mappable buffers a few frames later and are shared with the main world
//! through [`GpuFrameReadings`]. Without timestamp queries there is no timer
//! and no readings.
//!
//! The pass is all that can be timed from here. The drawing does not depend
//! on it: the painter uploads the read-back field from the host, so the
//! cameras' passes run beside the solver on the GPU, and no marker ordered
//! after the pass can see when they ended. A marker reading the drawn texture
//! was tried and began 34 µs after the pass on every scene, Metal's compute
//! encoders serialising, not the drawing finishing.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use bevy::{
    platform::time::Instant,
    prelude::*,
    render::{
        render_resource::{
            Buffer, BufferDescriptor, BufferUsages, CommandEncoder, MapMode, WgpuFeatures,
        },
        renderer::{RenderDevice, RenderQueue, WgpuWrapper},
    },
};

/// One frame on the GPU's clock, in seconds since the timer's first reading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuFrameReading {
    /// The render world's frame count when the frame was encoded.
    pub frame: u64,
    pub pass_begin: f64,
    pub pass_end: f64,
    /// The host's clock when the reading was taken in; pacing ignores stale
    /// ones.
    pub taken_at: Instant,
}

impl GpuFrameReading {
    /// The solver pass's own time.
    pub fn pass_seconds(&self) -> f64 {
        self.pass_end - self.pass_begin
    }
}

/// How many readings the main world can look back over.
const READINGS_KEPT: usize = 32;

/// The latest readings, shared between the render world, which takes them in,
/// and the main world's pacing, which reads them.
#[derive(Clone, Default)]
pub struct GpuFrameReadings(Arc<Mutex<VecDeque<GpuFrameReading>>>);

impl GpuFrameReadings {
    pub fn push(&self, reading: GpuFrameReading) {
        let mut readings = self.0.lock().unwrap();
        if readings.len() == READINGS_KEPT {
            readings.pop_front();
        }
        readings.push_back(reading);
    }

    /// The readings kept, oldest first.
    pub fn recent(&self) -> Vec<GpuFrameReading> {
        self.0.lock().unwrap().iter().copied().collect()
    }
}

/// Frames in flight. A slot is free again once its readback maps, a few
/// frames after it was written; a frame that finds none free goes untimed.
const SLOTS: u32 = 16;

/// Timestamps a slot holds: the pass's start and end.
const QUERIES: u32 = 2;

const READING_BYTES: u64 = 8 * QUERIES as u64;

enum SlotState {
    Free,
    /// This frame's solver pass writes it.
    Timing,
    /// Resolved and submitted with its frame.
    Submitted(u64),
    /// Mapping asked for.
    Mapping(u64),
}

struct Slot {
    staging: Buffer,
    state: SlotState,
    mapped: Arc<Mutex<Option<Option<[u64; 2]>>>>,
}

/// The render world's timer, present where the device has timestamp queries.
#[derive(Resource)]
pub struct GpuFrameTimer {
    // Bevy's wrappers: on the threaded web build a raw wgpu handle is not
    // `Send`, and a resource has to be.
    query_set: WgpuWrapper<wgpu::QuerySet>,
    resolve: Buffer,
    slots: Vec<Slot>,
    nanoseconds_per_tick: f64,
    origin: Option<u64>,
    frame: u64,
    /// The slot this frame's solver pass writes, until it is resolved.
    timing: Option<u32>,
    /// Whether this frame's solver pass was timed at all.
    timed: bool,
}

impl GpuFrameTimer {
    pub fn new(device: &RenderDevice, queue: &RenderQueue) -> Option<Self> {
        if !device.features().contains(WgpuFeatures::TIMESTAMP_QUERY) {
            return None;
        }
        Some(Self {
            query_set: WgpuWrapper::new(device.wgpu_device().create_query_set(
                &wgpu::QuerySetDescriptor {
                    label: Some("GPU frame timestamps"),
                    ty: wgpu::QueryType::Timestamp,
                    count: QUERIES * SLOTS,
                },
            )),
            resolve: device.create_buffer(&BufferDescriptor {
                label: Some("GPU frame timestamp resolve"),
                size: wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT * u64::from(SLOTS),
                usage: BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            slots: (0..SLOTS)
                .map(|_| Slot {
                    staging: device.create_buffer(&BufferDescriptor {
                        label: Some("GPU frame timestamp staging"),
                        size: READING_BYTES,
                        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    state: SlotState::Free,
                    mapped: Arc::new(Mutex::new(None)),
                })
                .collect(),
            nanoseconds_per_tick: f64::from(queue.get_timestamp_period()),
            origin: None,
            frame: 0,
            timing: None,
            timed: false,
        })
    }

    /// The render world's frame count, which readings are keyed by.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Whether this frame's solver pass was timed.
    pub fn timed(&self) -> bool {
        self.timed
    }

    /// Timestamp writes for this frame's solver pass, if a slot is free.
    pub fn pass_timestamps(&mut self) -> Option<wgpu::ComputePassTimestampWrites<'_>> {
        if self.timing.is_some() {
            return None;
        }
        let slot = self
            .slots
            .iter()
            .position(|slot| matches!(slot.state, SlotState::Free))? as u32;
        self.slots[slot as usize].state = SlotState::Timing;
        self.timing = Some(slot);
        self.timed = true;
        Some(wgpu::ComputePassTimestampWrites {
            query_set: &self.query_set,
            beginning_of_pass_write_index: Some(QUERIES * slot),
            end_of_pass_write_index: Some(QUERIES * slot + 1),
        })
    }

    /// Copies this frame's timestamps where they can be mapped. Called once
    /// the pass that wrote them has ended, on the same encoder or a later one.
    pub fn resolve_pass(&mut self, encoder: &mut CommandEncoder) {
        let Some(slot) = self.timing.take() else {
            return;
        };
        let first = QUERIES * slot;
        // Each slot resolves at its own aligned offset.
        let offset = wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT * u64::from(slot);
        encoder.resolve_query_set(
            &self.query_set,
            first..first + QUERIES,
            &self.resolve,
            offset,
        );
        encoder.copy_buffer_to_buffer(
            &self.resolve,
            offset,
            &self.slots[slot as usize].staging,
            0,
            READING_BYTES,
        );
        self.slots[slot as usize].state = SlotState::Submitted(self.frame);
    }

    /// Maps what was submitted and takes in what has mapped, into `readings`,
    /// and ends the frame. Called once a frame, after its submission.
    pub fn collect(&mut self, readings: &GpuFrameReadings) {
        // A pass timed but never resolved would hold its slot; every pass is
        // resolved on the encoder that ran it, so none does.
        if let Some(slot) = self.timing.take() {
            self.slots[slot as usize].state = SlotState::Free;
        }
        self.timed = false;
        self.frame += 1;
        for slot in &mut self.slots {
            match slot.state {
                SlotState::Submitted(frame) => {
                    slot.state = SlotState::Mapping(frame);
                    let mapped = slot.mapped.clone();
                    let buffer = slot.staging.clone();
                    slot.staging
                        .slice(..)
                        .map_async(MapMode::Read, move |outcome| {
                            let ticks = outcome.ok().map(|()| {
                                let ticks: [u64; 2] = bytemuck::pod_read_unaligned(
                                    &buffer.slice(..).get_mapped_range(),
                                );
                                ticks
                            });
                            buffer.unmap();
                            *mapped.lock().unwrap() = Some(ticks);
                        });
                }
                SlotState::Mapping(frame) => {
                    let Some(ticks) = slot.mapped.lock().unwrap().take() else {
                        continue;
                    };
                    slot.state = SlotState::Free;
                    let Some([begin, end]) = ticks else { continue };
                    let origin = *self.origin.get_or_insert(begin);
                    let seconds = |tick: u64| {
                        (tick as f64 - origin as f64) * self.nanoseconds_per_tick * 1.0e-9
                    };
                    readings.push(GpuFrameReading {
                        frame,
                        pass_begin: seconds(begin),
                        pass_end: seconds(end),
                        taken_at: Instant::now(),
                    });
                }
                SlotState::Free | SlotState::Timing => {}
            }
        }
    }
}
