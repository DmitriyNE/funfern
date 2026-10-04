//! Backpressure for Bevy's continuous GPU readbacks.
//!
//! `Readback` deliberately schedules a new staging copy every render frame. That
//! is convenient while the GPU keeps up, but an expensive solver or a
//! backgrounded native window can leave thousands of copies and Metal command
//! buffers outstanding. `PacedReadback` keeps the public completion-event model
//! while allowing at most [`IN_FLIGHT_DEPTH`] copies per entity to be in flight.

use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

use bevy::{
    prelude::*,
    render::{
        Render, RenderApp, RenderSystems,
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        gpu_readback::{Readback, ReadbackComplete},
        render_asset::RenderAssets,
        storage::GpuShaderBuffer,
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
/// is three to four frames. The cost is a staging buffer a copy.
const IN_FLIGHT_DEPTH: u8 = 3;

/// Shares the count in flight between the main and render worlds. The render
/// world claims a slot immediately before Bevy allocates its staging buffer;
/// the completion observer releases it for the same render frame's next
/// submission.
#[derive(Component, Clone, ExtractComponent)]
pub(crate) struct PacedReadback {
    in_flight: Arc<AtomicU8>,
}

impl PacedReadback {
    pub(crate) fn continuous(readback: Readback) -> (Readback, Self) {
        (readback, Self::new())
    }

    fn new() -> Self {
        Self {
            in_flight: Arc::new(AtomicU8::new(0)),
        }
    }

    fn try_submit(&self) -> bool {
        self.in_flight
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < IN_FLIGHT_DEPTH).then_some(count + 1)
            })
            .is_ok()
    }

    fn complete(&self) {
        // Saturating: a completion is never counted below none in flight.
        let _ = self
            .in_flight
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                Some(count.saturating_sub(1))
            });
    }
}

#[derive(Default)]
pub(crate) struct PacedReadbackPlugin;

impl Plugin for PacedReadbackPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(complete_paced_readback)
            .add_plugins(ExtractComponentPlugin::<PacedReadback>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        // This is exclusive so removal is visible to Bevy's readback-buffer
        // allocator in the following PrepareResources set without a deferred
        // command boundary. It has to run after the extracted components have
        // landed: `ExtractCommands` is what inserts this frame's `Readback`,
        // and a gate ordered only before `PrepareResources` was free to run
        // before it, removing nothing and letting a copy through on every
        // frame it refused. Which side it fell on changed with unrelated
        // systems being added to the schedule.
        render_app.add_systems(
            Render,
            gate_paced_readbacks
                .after(RenderSystems::ExtractCommands)
                .before(RenderSystems::PrepareResources),
        );
    }
}

fn complete_paced_readback(event: On<ReadbackComplete>, paced: Query<&PacedReadback>) {
    if let Ok(paced) = paced.get(event.entity) {
        paced.complete();
    }
}

fn gate_paced_readbacks(world: &mut World) {
    let candidates = {
        let mut query = world.query::<(Entity, &Readback, &PacedReadback)>();
        query
            .iter(world)
            .map(|(entity, readback, paced)| (entity, readback.clone(), paced.clone()))
            .collect::<Vec<_>>()
    };
    for (entity, readback, paced) in candidates {
        let ready = match &readback {
            Readback::Buffer { buffer, .. } => world
                .resource::<RenderAssets<GpuShaderBuffer>>()
                .get(buffer)
                .is_some(),
            // Funfern currently paces buffers only. Keeping texture support
            // permissive makes the component safe to reuse without coupling
            // this gate to Bevy's image asset preparation internals.
            Readback::Texture(_) => true,
        };
        if !ready || !paced.try_submit() {
            world.entity_mut(entity).remove::<Readback>();
        }
    }
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
}
