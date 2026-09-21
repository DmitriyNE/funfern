//! Helpers shared by the tests of several ui submodules.

use bevy_egui::egui::{self, Pos2, Rect};
use funfern_app::topology_editor::{TopologyAcceptance, TopologyEditor};

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
