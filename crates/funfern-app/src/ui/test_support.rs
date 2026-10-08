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

/// One frame of the viewport alone, at `time` seconds: `events` go to egui
/// and the viewport's response to them to the playground, as a frame does.
pub(super) fn viewport_frame(
    state: &mut Playground,
    context: &egui::Context,
    time: f64,
    modifiers: egui::Modifiers,
    events: Vec<egui::Event>,
) {
    let input = egui::RawInput {
        screen_rect: Some(viewport()),
        time: Some(time),
        modifiers,
        events,
        ..egui::RawInput::default()
    };
    let _ = context.run_ui(input, |ui| {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let (response, _) =
                    ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
                state.refresh_samples(response.rect);
                state.handle_viewport_input(ui, &response, response.rect);
            });
    });
}

/// A context that has shown the viewport once. egui hit-tests against the
/// widgets of the frame before, so the first frame's events land nowhere.
pub(super) fn viewport_context(state: &mut Playground) -> egui::Context {
    let context = egui::Context::default();
    viewport_frame(state, &context, 0.0, egui::Modifiers::NONE, vec![]);
    context
}

/// A mouse click at `at` over two frames from `time`, with `modifiers` held.
/// Answers the time after it.
pub(super) fn viewport_click(
    state: &mut Playground,
    context: &egui::Context,
    time: f64,
    modifiers: egui::Modifiers,
    at: Pos2,
) -> f64 {
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers,
    };
    viewport_frame(
        state,
        context,
        time,
        modifiers,
        vec![egui::Event::PointerMoved(at), button(true)],
    );
    viewport_frame(state, context, time + 0.02, modifiers, vec![button(false)]);
    time + 0.04
}

/// A scene with one baffle standing across the middle and `extra` besides,
/// framed so a world point can be clicked where it is drawn.
pub(super) fn with_baffles(extra: &[[Point2; 2]]) -> Playground {
    let mut state = Playground {
        editor: TopologyEditor::default(),
        scale: 250.0,
        center: Point2::new(0.0, 0.0),
        ..Playground::default()
    };
    for [start, end] in [[Point2::new(0.0, -0.5), Point2::new(0.0, 0.5)]]
        .iter()
        .chain(extra)
    {
        state
            .editor
            .create_boundary_baffle(
                OpenCubicSpline::polyline(vec![*start, start.lerp(*end, 0.5), *end]).unwrap(),
            )
            .unwrap();
        settle(&mut state.editor);
    }
    state
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

/// A widget of a pass as AccessKit was told of it: its label, or a label's
/// own text, where it was laid out, whether it was enabled, and what kind of
/// control it is.
#[derive(Clone, Debug)]
pub(super) struct LaidOut {
    pub(super) label: String,
    pub(super) rect: Rect,
    pub(super) enabled: bool,
    pub(super) role: egui::accesskit::Role,
}

/// Every labelled widget of `output`'s pass, which a context with AccessKit
/// enabled reports.
pub(super) fn laid_out(output: &egui::FullOutput) -> Vec<LaidOut> {
    let Some(update) = &output.platform_output.accesskit_update else {
        return Vec::new();
    };
    update
        .nodes
        .iter()
        .filter_map(|(_, node)| {
            let label = node.label().or_else(|| node.value())?;
            let bounds = node.bounds()?;
            Some(LaidOut {
                label: label.to_owned(),
                rect: Rect::from_min_max(
                    Pos2::new(bounds.x0 as f32, bounds.y0 as f32),
                    Pos2::new(bounds.x1 as f32, bounds.y1 as f32),
                ),
                enabled: !node.is_disabled(),
                role: node.role(),
            })
        })
        .collect()
}

/// A panel shown pass by pass in a context of its own, with the events of one
/// frame each, as the panel tests do by hand: themed, so the fonts are the
/// app's, and with AccessKit on, so the widgets of each pass can be found.
pub(super) struct PanelDriver {
    context: egui::Context,
}

impl PanelDriver {
    pub(super) fn new() -> Self {
        let context = egui::Context::default();
        theme::apply(&context);
        context.enable_accesskit();
        // A header opens over a dozen passes otherwise, its body laid out
        // where the next pass's click no longer finds it.
        context.all_styles_mut(|style| style.animation_time = 0.0);
        Self { context }
    }

    /// Whether a widget holds the keyboard focus, as a text field being
    /// typed in does.
    pub(super) fn focused(&self) -> bool {
        self.context.memory(|memory| memory.focused().is_some())
    }

    /// One pass of `show` with `events`, on a screen tall enough that nothing
    /// scrolls out of reach. Answers the widgets laid out, top to bottom and
    /// left to right, so the nth of a kind is the same widget pass to pass.
    pub(super) fn pass(
        &self,
        events: Vec<egui::Event>,
        show: impl FnMut(&mut egui::Ui),
    ) -> Vec<LaidOut> {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(480.0, 6000.0))),
            events,
            ..egui::RawInput::default()
        };
        let output = self.context.run_ui(input, show);
        let mut widgets = laid_out(&output);
        widgets.sort_by(|a, b| {
            a.rect
                .min
                .y
                .total_cmp(&b.rect.min.y)
                .then(a.rect.min.x.total_cmp(&b.rect.min.x))
        });
        widgets
    }
}

/// A key pressed with `modifiers` held, as the events of one pass.
pub(super) fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

/// Replacing a focused text field's text with `text` and pressing Enter, as
/// the events of one pass: select all, type, commit.
pub(super) fn typed(text: &str) -> Vec<egui::Event> {
    vec![
        key(egui::Key::A, egui::Modifiers::COMMAND),
        egui::Event::Text(text.to_owned()),
        key(egui::Key::Enter, egui::Modifiers::NONE),
    ]
}

/// A primary click at the centre of `widget`, as the events of one pass.
pub(super) fn click(widget: &LaidOut) -> Vec<egui::Event> {
    click_at(widget.rect.center())
}

/// A primary click at `at`, pressed and released in one pass.
pub(super) fn click_at(at: Pos2) -> Vec<egui::Event> {
    [true, false]
        .map(|pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        })
        .into_iter()
        .chain([egui::Event::PointerMoved(at)])
        .collect()
}

/// The primary button pressed at `at`, or released there, as the events of
/// one pass: a click whose press and release come in frames of their own.
pub(super) fn pointer_at(at: Pos2, pressed: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(at),
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}
