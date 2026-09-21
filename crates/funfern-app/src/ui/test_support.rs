//! Helpers shared by the tests of several ui submodules.

use super::*;
use bevy_egui::egui::{self, Pos2, Rect};
use funfern_app::topology_editor::{TopologyAcceptance, TopologyEditor};
use funfern_app::topology_runtime::PreparedTopology;
use funfern_app::topology_viewport::TopologySpanTarget;
use std::collections::BTreeSet;
use std::sync::Arc;

pub(super) fn viewport() -> Rect {
    Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))
}

pub(super) fn settle(editor: &mut TopologyEditor) {
    for _ in 0..100_000 {
        editor.validate_frame(64);
        if editor.acceptance != TopologyAcceptance::Pending {
            return;
        }
    }
    panic!("topology validation did not finish");
}

/// Prepares the editor's accepted scene and makes it the active topology,
/// the way a frame does once the GPU has acknowledged the upload.
pub(super) fn activate(state: &mut Playground) -> Arc<PreparedTopology> {
    activate_at(state, 0.18)
}

/// `activate` at a chosen mesh density, for anything that has to hold
/// across two different meshes of one scene.
pub(super) fn activate_at(
    state: &mut Playground,
    target_edge_length: f64,
) -> Arc<PreparedTopology> {
    let options = MeshingOptions {
        target_edge_length,
        ..MeshingOptions::default()
    };
    let token = state
        .runtime
        .request(
            state.editor.revision,
            &state.editor.document,
            state.editor.compiled_accepted.clone(),
            options,
            true,
        )
        .unwrap();
    for _ in 0..1_000_000 {
        if let Some(result) = state.runtime.advance(4096) {
            result.unwrap();
            return state.runtime.commit_ready(token).unwrap();
        }
    }
    panic!("topology preparation did not finish");
}

pub(super) fn every_span(state: &Playground) -> BTreeSet<TopologySpanTarget> {
    state
        .editor
        .document
        .model
        .draft
        .geometry
        .curves
        .iter()
        .flat_map(|curve| {
            curve
                .spans
                .iter()
                .map(|span| TopologySpanTarget::Curve(span.id))
        })
        .collect()
}
