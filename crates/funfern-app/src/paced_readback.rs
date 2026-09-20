//! Backpressure for Bevy's continuous GPU readbacks.
//!
//! `Readback` deliberately schedules a new staging copy every render frame. That
//! is convenient while the GPU keeps up, but an expensive solver or a
//! backgrounded native window can leave thousands of copies and Metal command
//! buffers outstanding. `PacedReadback` keeps the public completion-event model
//! while allowing at most one copy per entity to be in flight.

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

const IDLE: u8 = 0;
const IN_FLIGHT: u8 = 1;
const DONE: u8 = 2;

/// Shares the submission state between the main and render worlds. The render
/// world claims an idle readback immediately before Bevy allocates its staging
/// buffer; the completion observer releases continuous readbacks for the same
/// render frame's next submission.
#[derive(Component, Clone, ExtractComponent)]
pub(crate) struct PacedReadback {
    state: Arc<AtomicU8>,
    continuous: bool,
}

impl PacedReadback {
    pub(crate) fn continuous(readback: Readback) -> (Readback, Self) {
        (readback, Self::new(true))
    }

    pub(crate) fn once(readback: Readback) -> (Readback, Self) {
        (readback, Self::new(false))
    }

    fn new(continuous: bool) -> Self {
        Self {
            state: Arc::new(AtomicU8::new(IDLE)),
            continuous,
        }
    }

    fn try_submit(&self) -> bool {
        self.state
            .compare_exchange(IDLE, IN_FLIGHT, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn complete(&self) {
        self.state
            .store(if self.continuous { IDLE } else { DONE }, Ordering::Release);
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
        // command boundary.
        render_app.add_systems(
            Render,
            gate_paced_readbacks.before(RenderSystems::PrepareResources),
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

    #[test]
    fn continuous_readback_rearms_only_after_completion() {
        let paced = PacedReadback::new(true);
        assert!(paced.try_submit());
        assert!(!paced.try_submit());
        paced.complete();
        assert!(paced.try_submit());
    }

    #[test]
    fn one_shot_readback_never_rearms() {
        let paced = PacedReadback::new(false);
        assert!(paced.try_submit());
        paced.complete();
        assert!(!paced.try_submit());
    }
}
