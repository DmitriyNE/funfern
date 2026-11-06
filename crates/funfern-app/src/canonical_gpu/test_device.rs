//! A device for host tests, which have no render world: what the GPU writes
//! into the buffers the readbacks map, the copies of them a frame takes, and
//! their arrival, through the production observers and settle systems, in
//! whatever order the test picks.
//!
//! The device keeps what the shaders keep of a run: the control word's clock
//! and event result, the accepted lane, a failure latched in a status buffer,
//! and a handoff's receipt in its transfer buffer. Its field is the one the
//! host packed, unchanged: what is under test is which copy may change what,
//! not the physics. A copy holds the bytes as they stood when its frame took
//! it, at most [`DEPTH`] in flight a readback as `paced_readback` allows, and
//! arrives on the entity it was taken for, despawned since or not, as
//! `deliver_copies` triggers it. The copies of one frame map together and
//! arrive together, in whatever order, and frames' copies in the order the
//! frames took them; a test chooses when.

use std::collections::{HashMap, VecDeque};

use bevy::{
    asset::AssetId,
    ecs::system::RunSystemOnce,
    render::render_resource::encase::internal::{ReadFrom, WriteInto},
};

use super::*;

/// Copies of one readback in flight at once, `paced_readback`'s depth.
const DEPTH: usize = 3;

/// What a readback copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    Control,
    State,
    Status,
    Integrated,
    Receipt,
}

/// A copy taken and not yet delivered.
struct Copy {
    entity: Entity,
    kind: Kind,
    /// The generation its tag names; a status or receipt copy names none.
    generation: Option<u64>,
    data: Vec<u8>,
}

/// One copy's arrival, as the test may want to describe it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Arrival {
    pub(crate) kind: Kind,
    /// Whether it was taken of the generation the host's request held when
    /// it arrived, or for a status or a receipt, of a readback that still
    /// stands.
    pub(crate) current: bool,
}

/// The run of the generation the host's request holds, as the shaders keep
/// it.
struct Run {
    generation: u64,
    control: GpuCanonicalControl,
    /// Accepted steps, which the control word carries in a u32.
    completed: u64,
    /// The accepted lane, which every commit flips.
    slot: u32,
}

/// A pending handoff's validation, as its receipt prefix holds it.
#[derive(Clone, Copy)]
struct Receipt {
    failure: u32,
    snapshot: u64,
    local: u32,
    slot: u32,
}

#[derive(Default)]
pub(crate) struct TestDevice {
    run: Option<Run>,
    /// Failures latched, by the status buffer that latched them: clearing a
    /// failure replaces that buffer.
    failures: HashMap<AssetId<ShaderBuffer>, u32>,
    /// Handoffs validated, by their receipt readback.
    receipts: HashMap<Entity, Receipt>,
    /// The lane the device left its run in when it admitted a handoff, for
    /// the target that starts from it.
    admitted_slot: Option<u32>,
    in_flight: VecDeque<Vec<Copy>>,
}

/// The readbacks' observers and the request's settle systems need these
/// resources in `world`, and the observers added once, as
/// `CanonicalWaveGpuPlugin` adds them.
pub(crate) fn connect(world: &mut World) {
    world.add_observer(receive_canonical_state);
    world.add_observer(receive_canonical_control);
    world.add_observer(receive_canonical_status);
    world.add_observer(receive_canonical_handoff_receipt);
    world.add_observer(receive_canonical_integrated);
}

/// Runs `f` on `world` with `request` and `display` in it, where the
/// observers and settle systems look for them, for a test that keeps them
/// apart to hand to `refresh_runtime`.
pub(crate) fn lend<R>(
    world: &mut World,
    request: &mut CanonicalGpuRequest,
    display: &mut CanonicalGpuDisplay,
    f: impl FnOnce(&mut World) -> R,
) -> R {
    world.insert_resource(std::mem::take(request));
    world.insert_resource(std::mem::take(display));
    let result = f(world);
    *request = world.remove_resource().unwrap();
    *display = world.remove_resource().unwrap();
    result
}

fn read<T: ShaderType + ReadFrom + Default>(
    assets: &Assets<ShaderBuffer>,
    handle: &Handle<ShaderBuffer>,
) -> Option<T> {
    let data = assets.get(handle)?.data.clone()?;
    Some(
        PacedReadbackComplete {
            entity: Entity::PLACEHOLDER,
            data,
        }
        .to_shader_type(),
    )
}

fn bytes(value: impl ShaderType + WriteInto) -> Vec<u8> {
    let mut buffer = ShaderBuffer::default();
    buffer.set_data(value);
    buffer.data.unwrap()
}

/// The metadata word every commit writes after the state: the accepted lane
/// with twice the epoch's low 16 bits, and the accepted steps in two halves.
fn metadata(run: &Run) -> GpuCanonicalStateWord {
    let epoch = snapshot_epoch(run.control.clock_u32.x as u64);
    let step = run.control.clock_u32.w;
    GpuCanonicalStateWord {
        values: Vec4::new(
            SNAPSHOT_METADATA_MAGIC,
            (run.slot + 2 * epoch) as f32,
            (step & 0xffff) as f32,
            (step >> 16) as f32,
        ),
    }
}

impl TestDevice {
    /// The run of the generation the host's request holds, set up the first
    /// time the device meets it from what the host packed: an install's
    /// clock, or a handoff's target at the source's snapshot.
    fn sync(&mut self, world: &World) -> Option<&mut Run> {
        let request = world.resource::<CanonicalGpuRequest>();
        let Some(handles) = request.buffers.as_ref() else {
            self.run = None;
            return None;
        };
        if self
            .run
            .as_ref()
            .is_none_or(|run| run.generation != request.generation)
        {
            let assets = world.resource::<Assets<ShaderBuffer>>();
            let mut control: GpuCanonicalControl = read(assets, &handles.control)?;
            let completed = request.stats.completed_steps();
            control.clock_u32.z = request.stats.local_step();
            control.clock_u32.w = completed as u32;
            control.clock_f32.y = control.clock_u32.z as f32 * control.clock_f32.x;
            self.run = Some(Run {
                generation: request.generation,
                control,
                completed,
                slot: self.admitted_slot.take().unwrap_or(0),
            });
        }
        self.run.as_mut()
    }

    fn failure(&self, request: &CanonicalGpuRequest) -> u32 {
        request
            .buffers
            .as_ref()
            .and_then(|handles| self.failures.get(&handles.status.id()))
            .copied()
            .unwrap_or(0)
    }

    /// Whether the running generation has latched a failure the host may not
    /// have read yet.
    pub(crate) fn failed(&self, request: &CanonicalGpuRequest) -> bool {
        self.failure(request) != 0
    }

    /// The steps the running generation has accepted and the last event it
    /// processed, once the device has met it.
    pub(crate) fn done(&self, request: &CanonicalGpuRequest) -> Option<(u64, u32)> {
        self.run
            .as_ref()
            .filter(|run| run.generation == request.generation && request.buffers.is_some())
            .map(|run| (run.completed, run.control.event_result.z))
    }

    /// The device completes up to `steps` more of the steps asked of it,
    /// unless it has failed; its queue retires them at once.
    pub(crate) fn advance(&mut self, world: &World, steps: u64) {
        let request = world.resource::<CanonicalGpuRequest>();
        if self.failure(request) != 0 {
            return;
        }
        let desired = request.desired_steps;
        let staged = request
            .buffers
            .as_ref()
            .filter(|handles| handles.event_kind != EVENT_NONE)
            .map(|handles| (handles.event_kind, handles.event_serial));
        let Some(run) = self.sync(world) else {
            return;
        };
        let reached = run.completed.saturating_add(steps).min(desired);
        if reached <= run.completed {
            return;
        }
        let taken = reached - run.completed;
        // A generation installed with an event staged commits it with its
        // first step.
        if let Some((kind, serial)) = staged
            && run.control.event_result.z != serial
        {
            accept_event(&mut run.control, kind, serial);
        }
        let control = &mut run.control;
        control.clock_u32.z = control.clock_u32.z.wrapping_add(taken as u32);
        control.clock_u32.w = reached as u32;
        control.clock_f32.y = control.clock_u32.z as f32 * control.clock_f32.x;
        control.clock_f32.z = control.clock_f32.y;
        control.event.x = control.event.y;
        run.slot ^= (taken & 1) as u32;
        run.completed = reached;
        request.stats.retire(reached);
    }

    /// The device validates the pending handoff's transfer, admitting the
    /// target or refusing it, once; the receipt waits in the transfer buffer
    /// for a copy.
    pub(crate) fn admit(&mut self, world: &World, admit: bool) {
        let request = world.resource::<CanonicalGpuRequest>();
        let Some(entity) = request
            .handoff
            .as_ref()
            .map(|handoff| handoff.receipt_entity)
        else {
            return;
        };
        if self.receipts.contains_key(&entity) {
            return;
        }
        let Some(run) = self.sync(world) else {
            return;
        };
        let receipt = Receipt {
            failure: if admit {
                0
            } else {
                CANONICAL_FAILURE_NON_FINITE
            },
            snapshot: run.completed,
            local: run.control.clock_u32.z,
            slot: run.slot,
        };
        self.receipts.insert(entity, receipt);
        if admit {
            self.admitted_slot = Some(receipt.slot);
        }
    }

    /// The device processes the live event the host queued, taking it or
    /// refusing it, unless it has failed: a stopped device processes none.
    pub(crate) fn process(&mut self, world: &World, take: bool) {
        let request = world.resource::<CanonicalGpuRequest>();
        if self.failure(request) != 0 {
            return;
        }
        let Some((kind, serial)) = request
            .live_event
            .as_ref()
            .map(|event| (event.kind, event.serial))
        else {
            return;
        };
        let Some(run) = self.sync(world) else {
            return;
        };
        if run.control.event_result.z == serial {
            return;
        }
        if take {
            accept_event(&mut run.control, kind, serial);
        } else {
            run.control.event_result =
                UVec4::new(serial, kind, serial, CANONICAL_FAILURE_NON_FINITE);
        }
    }

    /// The running generation fails at its last accepted step and latches
    /// the failure in its status buffer.
    pub(crate) fn latch(&mut self, world: &World) {
        let request = world.resource::<CanonicalGpuRequest>();
        if let Some(handles) = request.buffers.as_ref() {
            self.failures
                .entry(handles.status.id())
                .or_insert(CANONICAL_FAILURE_NON_FINITE);
        }
    }

    /// A frame's copies: one of each readback with room in flight.
    pub(crate) fn take(&mut self, world: &mut World) {
        self.sync(world);
        let mut readbacks = Vec::new();
        let mut of = |kind: Kind, entity: Entity| readbacks.push((kind, entity));
        for entity in world
            .query_filtered::<Entity, (With<PacedReadback>, With<CanonicalControlReadback>)>()
            .iter(world)
        {
            of(Kind::Control, entity);
        }
        for entity in world
            .query_filtered::<Entity, (With<PacedReadback>, With<CanonicalStateReadback>)>()
            .iter(world)
        {
            of(Kind::State, entity);
        }
        for entity in world
            .query_filtered::<Entity, (With<PacedReadback>, With<CanonicalStatusReadback>)>()
            .iter(world)
        {
            of(Kind::Status, entity);
        }
        for entity in world
            .query_filtered::<Entity, (With<PacedReadback>, With<CanonicalIntegratedReadback>)>()
            .iter(world)
        {
            of(Kind::Integrated, entity);
        }
        for entity in world
            .query_filtered::<Entity, (With<PacedReadback>, With<CanonicalHandoffReceiptReadback>)>(
            )
            .iter(world)
        {
            of(Kind::Receipt, entity);
        }
        readbacks.sort();
        let mut batch = Vec::new();
        for (kind, entity) in readbacks {
            let in_flight = self
                .in_flight
                .iter()
                .flatten()
                .filter(|copy| copy.entity == entity)
                .count();
            if in_flight >= DEPTH {
                continue;
            }
            if let Some((generation, data)) = self.copy(world, kind, entity) {
                batch.push(Copy {
                    entity,
                    kind,
                    generation,
                    data,
                });
            }
        }
        if !batch.is_empty() {
            self.in_flight.push_back(batch);
        }
    }

    /// What a copy of `entity`'s source holds now. A copy is taken only of a
    /// buffer that still exists, so a readback of another generation than
    /// the running one copies nothing.
    fn copy(&self, world: &World, kind: Kind, entity: Entity) -> Option<(Option<u64>, Vec<u8>)> {
        let request = world.resource::<CanonicalGpuRequest>();
        let assets = world.resource::<Assets<ShaderBuffer>>();
        let run = self.run.as_ref();
        let handles = request.buffers.as_ref();
        let current = |generation: u64| run.filter(|run| run.generation == generation).zip(handles);
        match kind {
            Kind::Control => {
                let tag = world.get::<CanonicalControlReadback>(entity)?;
                let (run, _) = current(tag.generation)?;
                Some((Some(tag.generation), bytes(run.control)))
            }
            Kind::State => {
                let tag = world.get::<CanonicalStateReadback>(entity)?;
                let (run, handles) = current(tag.generation)?;
                let mut words: Vec<GpuCanonicalStateWord> = read(assets, &handles.state)?;
                if tag.full {
                    let at = handles.state_count as usize;
                    *words.get_mut(at)? = metadata(run);
                } else {
                    words.truncate(handles.node_count as usize);
                    words.push(metadata(run));
                }
                Some((Some(tag.generation), bytes(words)))
            }
            Kind::Integrated => {
                let tag = world.get::<CanonicalIntegratedReadback>(entity)?;
                let (run, handles) = current(tag.generation)?;
                let words: Vec<GpuCanonicalStateWord> = read(assets, &handles.state)?;
                let end = handles.state_count as usize;
                let mut integrated = words[end - handles.integrated_count as usize..end].to_vec();
                integrated.push(metadata(run));
                Some((Some(tag.generation), bytes(integrated)))
            }
            Kind::Status => {
                let readback = world.get::<PacedReadback>(entity)?;
                let mut status: GpuCanonicalStatus = read(assets, readback.source())?;
                if let Some(failure) = self.failures.get(&readback.source().id()) {
                    status.words.y = status.words.y.max(*failure);
                }
                Some((None, bytes(status)))
            }
            Kind::Receipt => {
                let tag = world.get::<CanonicalHandoffReceiptReadback>(entity)?;
                let nodes = tag.node_count as usize;
                let integrated = tag.integrated_count as usize;
                let Some(receipt) = self.receipts.get(&entity) else {
                    // The transfer as the host wrote it: no receipt yet.
                    let words = vec![GpuCanonicalTransferWord::default(); nodes + 1 + integrated];
                    return Some((None, bytes(words)));
                };
                let target = &request
                    .handoff
                    .as_ref()
                    .filter(|handoff| handoff.receipt_entity == entity)?
                    .target;
                let state: Vec<GpuCanonicalStateWord> = read(assets, &target.state)?;
                let end = target.state_count as usize;
                let word = |word: &GpuCanonicalStateWord| GpuCanonicalTransferWord {
                    data: UVec4::from_array(word.values.to_array().map(f32::to_bits)),
                };
                let mut words = vec![GpuCanonicalTransferWord {
                    data: UVec4::new(
                        HANDOFF_RECEIPT_MAGIC,
                        receipt.failure,
                        receipt.snapshot as u32,
                        receipt.local << 1 | receipt.slot,
                    ),
                }];
                words.extend(state[..nodes].iter().map(word));
                words.extend(state[end - integrated..end].iter().map(word));
                Some((None, bytes(words)))
            }
        }
    }

    /// The oldest frame's copies arrive, `rotation` of them moved to the
    /// back: the copies of one frame map together but may land in any order.
    pub(crate) fn deliver_oldest(&mut self, world: &mut World, rotation: usize) -> Vec<Arrival> {
        let Some(mut batch) = self.in_flight.pop_front() else {
            return Vec::new();
        };
        let length = batch.len();
        batch.rotate_left(rotation % length.max(1));
        self.arrive(world, batch)
    }

    /// Every copy in flight arrives, oldest frame first.
    pub(crate) fn deliver(&mut self, world: &mut World) -> Vec<Arrival> {
        let arriving = self.in_flight.drain(..).flatten().collect();
        self.arrive(world, arriving)
    }

    fn arrive(&mut self, world: &mut World, copies: Vec<Copy>) -> Vec<Arrival> {
        let mut arrivals = Vec::new();
        for copy in copies {
            let generation = world.resource::<CanonicalGpuRequest>().generation;
            arrivals.push(Arrival {
                kind: copy.kind,
                current: match copy.generation {
                    Some(taken) => taken == generation,
                    None => world.get_entity(copy.entity).is_ok(),
                },
            });
            world.trigger(PacedReadbackComplete {
                entity: copy.entity,
                data: copy.data,
            });
            world.flush();
        }
        arrivals
    }

    /// The device does what `does` does, and then every copy in flight
    /// arrives, and a frame's fresh copies after them.
    pub(crate) fn answer(
        &mut self,
        world: &mut World,
        does: impl FnOnce(&mut Self, &World),
    ) -> Vec<Arrival> {
        does(self, world);
        let mut arrivals = self.deliver(world);
        self.take(world);
        arrivals.extend(self.deliver(world));
        arrivals
    }

    /// The request's settle systems, as each frame's `Update` runs them.
    pub(crate) fn settle(world: &mut World) {
        world
            .run_system_once(settle_canonical_handoff)
            .expect("the handoff settles");
        world
            .run_system_once(settle_canonical_live_event)
            .expect("the live event settles");
        world
            .run_system_once(receive_canonical_snapshot)
            .expect("the snapshot is received");
    }
}

/// What `commit_event` writes of an event it takes.
fn accept_event(control: &mut GpuCanonicalControl, kind: u32, serial: u32) {
    control.event.x = serial;
    control.event.y = serial;
    match kind {
        EVENT_LINEAR_LAW_PATCH => control.runtime_serials.y = serial,
        EVENT_PRIMARY_PULSE => control.runtime_serials.z = serial,
        EVENT_MAINTENANCE | EVENT_FLUX_MAINTENANCE => control.runtime_serials.w = serial,
        EVENT_SOURCE_PATCH | EVENT_SOURCE_WEIGHT_PATCH => {
            control.runtime_serials.x = serial;
            control.runtime_slots.x ^= 1;
        }
        EVENT_TEMPORAL_SWITCH | EVENT_TEMPORAL_LAW_PATCH => {
            control.runtime_serials.y = serial;
            control.runtime_slots.y ^= 1;
        }
        _ => {}
    }
    control.event_result = UVec4::new(serial, kind, serial, 0);
}
