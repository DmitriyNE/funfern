//! The document and what leaves it: loading a scene, autosave, saving,
//! snapshots, viewport recording and the shareable link.

use crate::files::{self, FileEvent, SaveKind};
use crate::recording::{DestinationRequest, RecordingEvent};
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy_egui::egui::{self};
use funfern_app::topology_editor::{TopologyDocument, TopologyEditor};
use funfern_app::topology_persistence::{self as persistence};
use funfern_app::topology_viewport::TopologySelection;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::AtomicUsize;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::Ordering;

use super::*;

pub(super) const UNDO_SHORTCUT: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z);
pub(super) const REDO_SHORTCUT: egui::KeyboardShortcut = egui::KeyboardShortcut::new(
    egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
    egui::Key::Z,
);

/// A notice held in its window until closed.
pub(super) struct Notice {
    pub(super) title: &'static str,
    pub(super) text: String,
}

impl Playground {
    pub(super) fn set_document(
        &mut self,
        document: TopologyDocument,
        history: bool,
        fresh: bool,
    ) -> Result<(), String> {
        if history {
            self.editor.replace_validated_with_history(document)?;
        } else {
            self.editor.replace_validated(document)?;
        }
        self.example_opened = None;
        self.scene_replaced(fresh);
        Ok(())
    }

    /// The Undo button and shortcut. Stepping back over a New, an opened
    /// example or a load swaps the whole scene, and is treated as one. A
    /// gesture still under way is only cancelled, as Escape cancels it: its
    /// edit is not in the history yet, and stepping the history beneath it
    /// would strand it.
    pub(super) fn undo(&mut self) {
        if self.interaction_in_progress() {
            self.cancel_interaction();
            return;
        }
        let replaces = self.editor.undo_replaces_scene();
        if self.editor.undo() {
            self.history_moved(replaces);
        }
    }

    /// The Redo button and shortcut, the same way round.
    pub(super) fn redo(&mut self) {
        if self.interaction_in_progress() {
            self.cancel_interaction();
            return;
        }
        let replaces = self.editor.redo_replaces_scene();
        if self.editor.redo() {
            self.history_moved(replaces);
        }
    }

    /// Ctrl/Cmd+Z and Ctrl/Cmd+Shift+Z, the two buttons' own steps. A focused
    /// text field keeps them for its own text.
    pub(super) fn history_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        // The shifted one first, since the plain one would match it as well.
        let (redo, undo) = ctx.input_mut(|input| {
            let redo = input.consume_shortcut(&REDO_SHORTCUT);
            (redo, input.consume_shortcut(&UNDO_SHORTCUT))
        });
        if undo {
            self.undo();
        } else if redo {
            self.redo();
        }
    }

    fn history_moved(&mut self, replaced_scene: bool) {
        self.material_edit = None;
        self.material_formula_edits.clear();
        self.material_formula_errors.clear();
        if replaced_scene {
            self.example_opened = None;
            self.scene_replaced(true);
        } else {
            self.invalidate_samples();
        }
    }

    /// What follows a whole scene coming in, whether opened, loaded or
    /// reached by undo or redo across one: nothing of the outgoing scene's
    /// selection or tools carries over, and its generation is dropped at the
    /// next runtime update so none of it runs on under the new geometry.
    pub(super) fn scene_replaced(&mut self, fresh: bool) {
        self.selection = TopologySelection::None;
        self.selected_probe = None;
        self.draw = None;
        self.pending_merge = None;
        // The incoming scene numbers its materials and subdomains from the
        // same small ids, so none of the outgoing ones carries over: a copy
        // left open in the Materials panel would pass for unapplied edits to
        // the material sharing its id, show it as the old one, and hold a
        // click on a subdomain back from selecting its material.
        self.material_selection = DEFAULT_MATERIAL;
        self.region_selection = BACKGROUND_REGION;
        self.face_selection = 0;
        self.material_edit = None;
        self.material_formula_edits.clear();
        self.material_formula_errors.clear();
        self.parameter_name_edits.clear();
        self.material_color_edit = None;
        self.requested_revision = None;
        self.fresh_requested = fresh;
        self.drop_requested = true;
        // Framed as at launch: the new domain need not be where, or as
        // large as, the old one.
        self.fit = true;
        // The scale is started again when the new field arrives: with the
        // outgoing generation dropped there is no field to measure until then.
        self.invalidate_samples();
    }
    pub(super) fn update_files(&mut self) {
        let events = self.receiver.lock().unwrap().try_iter().collect::<Vec<_>>();
        for event in events {
            match event {
                FileEvent::Loaded(bytes) => match persistence::parse(&bytes) {
                    Ok(candidate) => {
                        self.load = Some(candidate);
                        self.file_busy = true;
                    }
                    Err(error) => {
                        self.file_busy = false;
                        self.raise_notice("Scene not opened", error);
                    }
                },
                FileEvent::SnapshotCaptured(bytes) => {
                    self.snapshot_state = SnapshotState::Saving;
                    self.file_busy = true;
                    files::save(self.sender.clone(), bytes, SaveKind::SnapshotPng);
                }
                FileEvent::Saved(message) => {
                    self.file_busy = false;
                    self.snapshot_state = SnapshotState::Idle;
                    self.notify(message);
                }
                FileEvent::Cancelled => {
                    self.file_busy = false;
                    self.snapshot_state = SnapshotState::Idle;
                }
                FileEvent::Error(title, error) => {
                    self.file_busy = false;
                    self.snapshot_state = SnapshotState::Idle;
                    self.raise_notice(title, error);
                }
            }
        }
        if let Some(result) = self.load.as_mut().and_then(|load| load.advance(1)) {
            self.load = None;
            self.file_busy = false;
            match result.and_then(|document| {
                TopologyEditor::from_document(document.clone())?;
                self.set_document(document, false, true)
            }) {
                Ok(()) => self.notify("Scene loaded; history cleared"),
                Err(error) => self.raise_notice("Scene not opened", error),
            }
        }
    }
    /// What launch opens: a shared link first, then the last session's
    /// autosave, then a random example. A link or an autosave that does not
    /// open, an older version's among them, raises a notice and leaves the
    /// next in line to open: a broken link does not cost the last session,
    /// and an autosave this version cannot read is replaced at the next
    /// autosave, but not quietly.
    pub(super) fn open_startup_scene(
        &mut self,
        link: Option<Result<Vec<u8>, String>>,
        autosave: impl FnOnce() -> Result<Option<Vec<u8>>, String>,
    ) {
        // A link that does not open is still in the address, and is replaced
        // by the scene that opens in its place.
        self.link_in_address = link.is_some();
        let link_error = match link.map(|link| link.and_then(|bytes| self.open_scene_bytes(&bytes)))
        {
            Some(Ok(())) => return,
            Some(Err(error)) => Some(error),
            None => None,
        };
        let restored = match autosave()
            .and_then(|bytes| bytes.map(|bytes| self.open_scene_bytes(&bytes)).transpose())
        {
            Ok(Some(())) => true,
            // A first run. A random example rather than the same one every
            // time, so the app opens on something worth looking at.
            Ok(None) => {
                self.open_random_example();
                false
            }
            Err(error) => {
                self.open_random_example();
                self.raise_notice(
                    "Previous session not restored",
                    format!(
                        "{error}.\n\nAn example opened instead, and the next autosave replaces the old one."
                    ),
                );
                false
            }
        };
        if let Some(error) = link_error {
            let instead = if restored {
                "Your last session was restored instead."
            } else {
                "An example opened instead."
            };
            self.notices.insert(
                0,
                Notice {
                    title: "Shared link not opened",
                    text: format!("{error}.\n\n{instead}"),
                },
            );
        }
    }

    /// What the user should know and the status line would lose to the next
    /// message: anything given up on their behalf. It is held in a window
    /// until they close it.
    pub(super) fn raise_notice(&mut self, title: &'static str, text: String) {
        self.notices.push(Notice { title, text });
    }

    pub(super) fn notice_window(&mut self, ctx: &egui::Context) {
        let Some(first) = self.notices.first() else {
            return;
        };
        let mut close = false;
        egui::Window::new(first.title)
            .id(egui::Id::new("notice"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(360.0)
            .show(ctx, |ui| {
                for (index, notice) in self.notices.iter().enumerate() {
                    if index > 0 {
                        ui.separator();
                        ui.strong(notice.title);
                    }
                    ui.label(&notice.text);
                }
                ui.add_space(4.0);
                // In a row, so the right-to-left layout takes one line's
                // height rather than the rest of the screen's.
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = ui.button("OK").clicked();
                    });
                });
            });
        if close {
            self.notices.clear();
        }
    }

    /// Opens a scene file's bytes in place of the document, or leaves the
    /// document as it was.
    fn open_scene_bytes(&mut self, bytes: &[u8]) -> Result<(), String> {
        let document = persistence::parse_document(bytes)?;
        TopologyEditor::from_document(document.clone())?;
        self.set_document(document, false, true)
    }
    pub(super) fn autosave(&mut self) {
        if self.editor.document != self.autosave_observed {
            self.autosave_observed = self.editor.document.clone();
            self.autosave_due = Some(Instant::now());
        }
        if self
            .autosave_due
            .is_some_and(|at| at.elapsed().as_secs_f32() > 0.8)
        {
            self.autosave_due = None;
            let written = crate::recovery::save(&self.editor.document);
            self.autosave_written(written);
            self.refresh_link();
        }
    }

    /// An autosave that fails says so once, until one succeeds: the session
    /// it should have kept would otherwise be lost to a reload unannounced.
    fn autosave_written(&mut self, written: Result<(), String>) {
        match written {
            Ok(()) => self.autosave_failing = false,
            Err(_) if self.autosave_failing => {}
            Err(error) => {
                self.autosave_failing = true;
                self.raise_notice(
                    "Autosave failed",
                    format!(
                        "{error}.\n\nUntil an autosave succeeds, a reload or relaunch will not bring this session back. Save it as a file to keep it."
                    ),
                );
            }
        }
    }

    /// While the address carries a scene link, opened or copied, it follows
    /// the document: a reload opens the link ahead of the autosave, and a
    /// link left behind would drop every edit since.
    fn refresh_link(&mut self) {
        if self.link_in_address {
            self.follow_link(
                crate::sharing::encode(&self.editor.document)
                    .and_then(|fragment| crate::sharing::replace_fragment(&fragment)),
            );
        }
    }

    /// A scene the link can no longer carry, grown past its size, takes the
    /// link out of the address, so a reload restores the autosave instead.
    fn follow_link(&mut self, written: Result<(), String>) {
        if let Err(error) = written {
            self.link_in_address = false;
            let _ = crate::sharing::clear_fragment();
            self.raise_notice(
                "Scene link taken out of the address",
                format!(
                    "{error}.\n\nThe address no longer follows the scene, and a reload restores the autosave."
                ),
            );
        }
    }
    pub(super) fn save_scene(&mut self) {
        match persistence::save(&self.editor.document) {
            Ok(json) => {
                self.file_busy = true;
                files::save(self.sender.clone(), json.into_bytes(), SaveKind::Scene);
            }
            Err(error) => self.raise_notice("Scene not saved", error),
        }
    }
    pub(super) fn export_viewport_png(&mut self) {
        if self.snapshot_state == SnapshotState::Idle
            && self.recording_state == RecordingState::Idle
        {
            // The request is armed at the start of the next frame. This gives
            // egui one complete frame to close the File menu before readback.
            self.snapshot_state = SnapshotState::Requested;
            self.file_busy = true;
        }
    }
    pub(super) fn request_video_recording(&mut self) {
        if self.snapshot_state != SnapshotState::Idle
            || self.recording_state != RecordingState::Idle
        {
            return;
        }
        self.recording_started = None;
        self.recording_description.clear();
        self.recording_dropped_frames = 0;
        self.recording_last_requested_slot = None;
        match self.video_recorder.request_destination() {
            Ok(DestinationRequest::Ready) => self.recording_state = RecordingState::Requested,
            Ok(DestinationRequest::Pending) => {
                self.recording_state = RecordingState::SelectingDestination
            }
            Err(error) => self.notify(error),
        }
    }
    pub(super) fn stop_video_recording(&mut self) {
        match self.recording_state {
            RecordingState::Requested | RecordingState::Preparing => {
                self.recording_state = RecordingState::Idle;
            }
            RecordingState::Starting | RecordingState::Recording => {
                self.video_recorder.stop();
                self.recording_state = RecordingState::Finalizing;
            }
            _ => {}
        }
    }
    pub(super) fn begin_capture_frame(&mut self) {
        if self.snapshot_state == SnapshotState::Requested {
            self.snapshot_state = SnapshotState::Armed;
        }
        if self.recording_state == RecordingState::Requested {
            self.recording_state = RecordingState::Preparing;
        }
    }
    pub(super) fn update_recording(&mut self) {
        for event in self.video_recorder.poll() {
            match event {
                RecordingEvent::DestinationReady => {
                    if self.recording_state == RecordingState::SelectingDestination {
                        self.recording_state = RecordingState::Requested;
                    }
                }
                RecordingEvent::Started(description) => {
                    if self.recording_state == RecordingState::Starting {
                        self.recording_description = description;
                        self.recording_started = Some(Instant::now());
                        self.recording_last_requested_slot = None;
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            self.recording_readback_in_flight = Arc::new(AtomicUsize::new(0));
                        }
                        self.recording_state = RecordingState::Recording;
                    }
                }
                RecordingEvent::Finished(message) => {
                    self.video_recorder.stop();
                    self.recording_state = RecordingState::Idle;
                    self.recording_started = None;
                    self.recording_last_requested_slot = None;
                    #[cfg(not(target_arch = "wasm32"))]
                    self.recording_readback_in_flight
                        .store(0, Ordering::Release);
                    self.notify(message);
                }
                RecordingEvent::Cancelled => self.recording_state = RecordingState::Idle,
                RecordingEvent::DroppedFrame => self.recording_dropped_frames += 1,
                RecordingEvent::Error(error) => {
                    self.video_recorder.stop();
                    self.recording_state = RecordingState::Idle;
                    self.recording_started = None;
                    self.recording_last_requested_slot = None;
                    #[cfg(not(target_arch = "wasm32"))]
                    self.recording_readback_in_flight
                        .store(0, Ordering::Release);
                    self.raise_notice("Recording stopped", error);
                }
            }
        }
    }
    pub(super) fn copy_link(&mut self, context: &egui::Context) {
        match crate::sharing::encode(&self.editor.document)
            .and_then(|fragment| crate::sharing::link(&fragment))
        {
            Ok(link) => {
                self.link_in_address = true;
                context.copy_text(link.clone());
                let _ = &link;
                #[cfg(target_arch = "wasm32")]
                if let Some(clipboard) = web_sys::window().map(|w| w.navigator().clipboard()) {
                    let _ = clipboard.write_text(&link);
                }
                self.notify("Scene link copied");
            }
            Err(error) => self.notify(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_support::*;
    use funfern_app::topology_editor::TopologyEditor;
    use funfern_app::topology_viewport::TopologySpanTarget;

    /// A scene that replaces the whole document opens framed, as launch
    /// does: its domain need not be where, or as large as, the last one's.
    /// A history step within a scene leaves the view where it is.
    #[test]
    fn a_replaced_scene_opens_fitted_to_the_view() {
        let mut state = Playground {
            fit: false,
            ..Playground::default()
        };
        state.open_example(0);
        assert!(state.fit, "an opened example");
        state.fit = false;
        state.new_scene();
        assert!(state.fit, "a new scene");
        state.fit = false;
        state.undo();
        assert!(state.fit, "undo across a new scene");
        state.fit = false;
        state.history_moved(false);
        assert!(!state.fit, "a step within the scene");
    }

    /// Opening another scene drops the outgoing generation, as at launch:
    /// the host forgets the active topology and its clock, and the new
    /// scene's first preparation is a fresh full build rather than a repair
    /// of the old mesh running on underneath it.
    #[test]
    fn a_replaced_scene_drops_the_outgoing_generation() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        settle(&mut state.editor);
        activate(&mut state);
        state.uploaded_time_step = 0.01;
        state.sim_time_offset = 3.0;
        state.sim_time_step = 0.01;
        state.amr_status = "monitoring solution".into();
        let example = &funfern_app::topology_examples::catalog()[0];
        state
            .set_document(example.document.clone(), true, true)
            .unwrap();
        assert!(state.drop_requested && state.fresh_requested);
        state.drop_requested = false;
        state.drop_generation();
        assert!(state.runtime.active().is_none());
        assert_eq!(
            (
                state.uploaded_time_step,
                state.sim_time_offset,
                state.sim_time_step
            ),
            (0.0, 0.0, 0.0)
        );
        assert_eq!(state.amr_status, "waiting for solution");
        settle(&mut state.editor);
        let next = activate(&mut state);
        assert!(matches!(
            next.mesh_action,
            TopologyMeshUpdateAction::FullRebuild(_)
        ));
    }

    /// A replaced scene is measured against its own loudest level, not the
    /// outgoing scene's. Carried over, the Josephson line's integrated field,
    /// 34.8, floored every quieter scene opened after it at a hundredth of that
    /// and left it painting black.
    #[test]
    fn a_replaced_scene_starts_its_scales_from_nothing() {
        let mut state = Playground::default();
        state.field_exposure.update(34.8, 0.016);
        state.vector_overlay_exposure.update(2.0, 0.016);
        let example = &funfern_app::topology_examples::catalog()[0];
        state
            .set_document(example.document.clone(), true, true)
            .unwrap();
        state.drop_requested = false;
        state.drop_generation();
        let quiet = 5.0e-3;
        assert_eq!(state.field_exposure.update(quiet, 0.016), Some(quiet));
        assert_eq!(state.field_exposure.visibility(quiet), 1.0);
        assert_eq!(
            state.vector_overlay_exposure.update(quiet, 0.016),
            Some(quiet)
        );
    }

    /// One frame that presses Z with `modifiers` and runs the shortcuts first,
    /// as the app's frame does, then shows a text field when one is given.
    fn press_z(
        state: &mut Playground,
        context: &egui::Context,
        modifiers: egui::Modifiers,
        mut field: Option<(egui::Id, &mut String)>,
    ) {
        let input = egui::RawInput {
            modifiers,
            events: vec![egui::Event::Key {
                key: egui::Key::Z,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..egui::RawInput::default()
        };
        let _ = context.run_ui(input, |ui| {
            state.history_shortcuts(ui.ctx());
            if let Some((id, text)) = field.as_mut() {
                ui.add(egui::TextEdit::singleline(*text).id(*id));
            }
        });
    }

    /// A scene one edit into its history, the edit being a wider domain.
    fn one_edit_in() -> Playground {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        settle(&mut state.editor);
        state
            .editor
            .set_domain(DomainRect {
                max_x: 1.4,
                ..DomainRect::UNIT
            })
            .unwrap();
        settle(&mut state.editor);
        state
    }

    #[test]
    fn the_keyboard_steps_the_history_as_the_buttons_do() {
        let mut state = one_edit_in();
        let edited = state.editor.document.model.clone();
        let context = egui::Context::default();
        press_z(&mut state, &context, egui::Modifiers::COMMAND, None);
        assert_eq!(state.editor.history_len(), (0, 1));
        assert_ne!(state.editor.document.model, edited);
        press_z(
            &mut state,
            &context,
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            None,
        );
        assert_eq!(state.editor.history_len(), (1, 0));
        assert_eq!(state.editor.document.model, edited);
        press_z(&mut state, &context, egui::Modifiers::NONE, None);
        assert_eq!(state.editor.history_len(), (1, 0), "a bare Z stepped");
    }

    #[test]
    fn a_focused_text_field_keeps_the_undo_shortcut() {
        let mut state = one_edit_in();
        let context = egui::Context::default();
        let field = egui::Id::new("typing");
        let mut text = String::new();
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            ui.add(egui::TextEdit::singleline(&mut text).id(field));
            ui.memory_mut(|memory| memory.request_focus(field));
        });
        assert!(context.egui_wants_keyboard_input());
        press_z(
            &mut state,
            &context,
            egui::Modifiers::COMMAND,
            Some((field, &mut text)),
        );
        assert_eq!(state.editor.history_len(), (1, 0));
    }

    /// Mid-gesture, the shortcut does what Escape does and nothing more, and
    /// so do the toolbar's buttons: they used to step the history under a
    /// drawing and leave it open.
    #[test]
    fn the_undo_shortcut_cancels_a_gesture_rather_than_stepping_under_it() {
        let mut state = one_edit_in();
        let context = egui::Context::default();
        state.begin_draw(DrawTool::Polygon);
        press_z(&mut state, &context, egui::Modifiers::COMMAND, None);
        assert!(state.draw.is_none(), "the draw survived");
        assert_eq!(state.editor.history_len(), (1, 0));
        for button in [Playground::undo, Playground::redo] {
            state.begin_draw(DrawTool::Polygon);
            button(&mut state);
            assert!(state.draw.is_none(), "the draw survived the button");
            assert_eq!(state.editor.history_len(), (1, 0));
        }

        let edited = state.editor.document.model.clone();
        state.editor.begin();
        state.editor.document.model.source.width *= 2.0;
        press_z(&mut state, &context, egui::Modifiers::COMMAND, None);
        assert!(!state.editor.editing(), "the edit session survived");
        assert_eq!(state.editor.document.model, edited);
        assert_eq!(state.editor.history_len(), (1, 0));
    }

    /// Undo and redo across an opened scene are scene replacements too;
    /// across an edit they are not, and a live edit never drops anything.
    #[test]
    fn undoing_an_opened_scene_drops_its_generation_and_undoing_an_edit_does_not() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        settle(&mut state.editor);
        let example = &funfern_app::topology_examples::catalog()[0];
        state
            .set_document(example.document.clone(), true, true)
            .unwrap();
        settle(&mut state.editor);
        state.drop_requested = false;
        state.fresh_requested = false;

        state
            .editor
            .set_domain(DomainRect {
                max_x: 1.4,
                ..DomainRect::UNIT
            })
            .unwrap();
        settle(&mut state.editor);
        assert!(!state.drop_requested, "a live edit dropped the generation");
        state.undo();
        assert!(
            !state.drop_requested,
            "undoing an edit dropped the generation"
        );

        state.undo();
        assert!(
            state.drop_requested && state.fresh_requested,
            "undoing the opened scene kept its generation"
        );
        state.drop_requested = false;
        state.fresh_requested = false;
        state.redo();
        assert!(
            state.drop_requested && state.fresh_requested,
            "redoing the opened scene kept the other one's generation"
        );
    }

    /// A capture crops to the viewport, so what has to be suppressed is the
    /// chrome inside the crop, not the panels outside it.
    #[test]
    fn a_capture_hides_the_chrome_inside_its_crop() {
        let mut state = Playground::default();
        assert!(!state.capturing(), "idle is not a capture");
        for snapshot in [SnapshotState::Armed, SnapshotState::Capturing] {
            state.snapshot_state = snapshot;
            assert!(state.capturing(), "{snapshot:?}");
        }
        state.snapshot_state = SnapshotState::Saving;
        assert!(
            !state.capturing(),
            "saving happens after the pixels are taken"
        );
        state.snapshot_state = SnapshotState::Idle;
        for recording in [
            RecordingState::Preparing,
            RecordingState::Starting,
            RecordingState::Recording,
        ] {
            state.recording_state = recording;
            assert!(state.capturing(), "{recording:?}");
        }
        state.recording_state = RecordingState::SelectingDestination;
        assert!(
            !state.capturing(),
            "choosing a destination is not yet a capture"
        );

        // Selection emphasis is the one thing that has to read through the
        // predicate rather than be gated at a call site.
        state.recording_state = RecordingState::Recording;
        state.selection = TopologySelection::Spans(
            [TopologySpanTarget::Outer(OuterSide::Bottom)]
                .into_iter()
                .collect(),
        );
        assert!(
            !state.span_selected(TopologySpanTarget::Outer(OuterSide::Bottom)),
            "a selected span must not read as selected in a capture"
        );
        state.recording_state = RecordingState::Idle;
        assert!(state.span_selected(TopologySpanTarget::Outer(OuterSide::Bottom)));
    }
    /// Loading a document used to raise `reset_requested`, which the GPU reset
    /// spends earlier in the frame and against the topology still active — the
    /// scene on its way out. That scene was zeroed and then ran on for the
    /// seconds its replacement took to prepare, so by the time the new mesh was
    /// ready the transfer carried a full-amplitude field into it. Only the
    /// oscillation radiated away; the constant it left behind is in the
    /// stiffness operator's null space and no outgoing wall can remove it, so
    /// opening the Luneburg lens and then anything else washed the new scene
    /// flat.
    #[test]
    fn loading_a_document_asks_for_a_field_that_starts_at_zero() {
        let mut state = Playground::default();
        let example = &funfern_app::topology_examples::catalog()[0];
        state
            .set_document(example.document.clone(), false, true)
            .unwrap();
        assert!(
            state.fresh_requested,
            "the load did not ask to start at zero"
        );
        assert!(
            !state.reset_requested,
            "the load armed the flag the GPU reset spends against the outgoing scene"
        );
        // The shape of the bug: a topology is already active and the GPU reset
        // has taken its flag, and the load must still start the field at zero.
        assert!(starts_from_zero(true, false, true));
        assert!(starts_from_zero(true, true, false));
        assert!(starts_from_zero(false, false, false));
        assert!(!starts_from_zero(true, false, false));
    }

    /// A scene file as version 21 wrote it, as far as this version can tell:
    /// the header is what turns it away.
    fn version_21_bytes() -> Vec<u8> {
        let saved = persistence::save(&funfern_app::topology_examples::catalog()[1].document);
        let mut value: serde_json::Value = serde_json::from_str(&saved.unwrap()).unwrap();
        value["version"] = 21.into();
        serde_json::to_vec(&value).unwrap()
    }

    fn example_bytes(index: usize) -> Vec<u8> {
        let document = &funfern_app::topology_examples::catalog()[index].document;
        persistence::save(document).unwrap().into_bytes()
    }

    /// Each held notice as its title and text.
    fn notices(state: &Playground) -> Vec<(&'static str, String)> {
        state
            .notices
            .iter()
            .map(|notice| (notice.title, notice.text.clone()))
            .collect()
    }

    #[test]
    fn an_autosave_that_does_not_open_raises_a_notice_and_opens_an_example() {
        for (autosave, reason) in [
            (Ok(Some(version_21_bytes())), "version 21"),
            (Ok(Some(b"{\"version\": 22, \"model\"".to_vec())), "EOF"),
            (Err("Autosave exceeds 16 MiB".to_string()), "16 MiB"),
        ] {
            let mut state = Playground::default();
            state.open_startup_scene(None, || autosave);
            assert!(
                state.example_opened.is_some(),
                "{reason}: no example opened"
            );
            let notices = notices(&state);
            assert!(
                matches!(&notices[..], [("Previous session not restored", text)]
                    if text.contains(reason) && text.contains("An example opened instead")),
                "{reason}: {notices:?}"
            );
        }
    }

    #[test]
    fn a_first_run_opens_an_example_quietly_and_an_autosave_restores() {
        let mut state = Playground::default();
        state.open_startup_scene(None, || Ok(None));
        assert!(state.example_opened.is_some());
        assert!(state.notices.is_empty());

        let mut state = Playground::default();
        state.open_startup_scene(None, || Ok(Some(example_bytes(2))));
        let expected = &funfern_app::topology_examples::catalog()[2].document;
        assert_eq!(&state.editor.document, expected);
        assert_eq!(state.example_opened, None);
        assert!(state.notices.is_empty());
        assert_eq!(state.editor.history_len(), (0, 0));
    }

    /// A link that does not open, an older version's among them, still
    /// restores the last session rather than leaving it to be autosaved over.
    #[test]
    fn a_link_that_does_not_open_leaves_the_last_session_to_restore() {
        let expected = &funfern_app::topology_examples::catalog()[2].document;
        for (link, reason) in [
            (Ok(version_21_bytes()), "version 21"),
            (Err("Shared scene data is damaged".to_string()), "damaged"),
        ] {
            let mut state = Playground::default();
            state.open_startup_scene(Some(link), || Ok(Some(example_bytes(2))));
            assert_eq!(&state.editor.document, expected, "{reason}");
            let notices = notices(&state);
            assert!(
                matches!(&notices[..], [("Shared link not opened", text)]
                    if text.contains(reason) && text.contains("last session was restored")),
                "{reason}: {notices:?}"
            );
        }

        let mut state = Playground::default();
        state.open_startup_scene(Some(Ok(version_21_bytes())), || Ok(Some(b"[]".to_vec())));
        assert!(state.example_opened.is_some());
        let notices = notices(&state);
        assert!(
            matches!(&notices[..], [("Shared link not opened", link), ("Previous session not restored", _)]
                if link.contains("An example opened instead")),
            "{notices:?}"
        );
    }

    /// The status line is one slot, and the mesh the opened scene sends to
    /// the device writes "Simulation topology committed" over it within the
    /// first seconds: a startup notice there was gone before it could be
    /// read. The window holds it, past any message, until OK.
    #[test]
    fn a_notice_is_held_past_the_status_line_until_closed() {
        let mut state = Playground::default();
        state.open_startup_scene(Some(Err("Shared scene data is damaged".into())), || {
            Ok(None)
        });
        state.notify("Simulation topology committed");
        let context = egui::Context::default();
        theme::apply(&context);
        context.enable_accesskit();
        let pass = |state: &mut Playground, events| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                events,
                ..egui::RawInput::default()
            };
            laid_out(&context.run_ui(input, |ui| state.notice_window(ui.ctx())))
        };
        for _ in 0..2 {
            pass(&mut state, vec![]);
        }
        let widgets = pass(&mut state, vec![]);
        assert!(
            widgets
                .iter()
                .any(|widget| widget.label.contains("Shared scene data is damaged")),
            "{:?}",
            widgets
                .iter()
                .map(|widget| &widget.label)
                .collect::<Vec<_>>()
        );
        let ok = widgets.iter().find(|widget| widget.label == "OK").unwrap();
        pass(&mut state, click(ok));
        assert!(state.notices.is_empty());
        assert!(pass(&mut state, vec![]).is_empty());
    }

    #[test]
    fn a_link_that_opens_leaves_the_autosave_unread() {
        let mut state = Playground::default();
        state.open_startup_scene(Some(Ok(example_bytes(3))), || {
            panic!("the autosave was read under a link that opened")
        });
        let expected = &funfern_app::topology_examples::catalog()[3].document;
        assert_eq!(&state.editor.document, expected);
    }

    /// An older or damaged file leaves the document, its history and its run
    /// as they were, and says why.
    #[test]
    fn a_file_that_does_not_open_leaves_the_document_untouched() {
        for (bytes, reason) in [
            (version_21_bytes(), "requires version 22"),
            (b"not a scene".to_vec(), "expected"),
        ] {
            let mut state = Playground::default();
            state.open_example(2);
            state.fresh_requested = false;
            let document = state.editor.document.clone();
            let history = state.editor.history_len();
            state.sender.send(FileEvent::Loaded(bytes)).unwrap();
            state.update_files();
            assert_eq!(state.editor.document, document, "{reason}");
            assert_eq!(state.editor.history_len(), history, "{reason}");
            assert_eq!(state.example_opened, Some(2), "{reason}");
            assert!(!state.fresh_requested, "{reason}: the run was restarted");
            assert!(!state.file_busy, "{reason}");
            let notices = notices(&state);
            assert!(
                matches!(&notices[..], [("Scene not opened", text)] if text.contains(reason)),
                "{reason}: {notices:?}"
            );
        }
    }

    /// A file that was not written, or not read, is said in a notice titled
    /// for what did not happen.
    #[test]
    fn a_file_that_fails_raises_a_notice_for_what_did_not_happen() {
        let mut state = Playground {
            file_busy: true,
            ..Playground::default()
        };
        state
            .sender
            .send(FileEvent::Error(
                "Scene not saved",
                "Permission denied (os error 13)".into(),
            ))
            .unwrap();
        state.update_files();
        assert!(!state.file_busy);
        assert_eq!(
            notices(&state),
            [(
                "Scene not saved",
                "Permission denied (os error 13)".to_string()
            )]
        );
    }

    /// One notice for a run of failed autosaves, and another for a failure
    /// after one succeeded.
    #[test]
    fn a_failing_autosave_says_so_once_until_one_succeeds() {
        let mut state = Playground::default();
        let full = || Err("Browser storage for this site is full".to_string());
        state.autosave_written(full());
        state.autosave_written(full());
        let notices_now = notices(&state);
        assert!(
            matches!(&notices_now[..], [("Autosave failed", text)]
                if text.contains("storage for this site is full")),
            "{notices_now:?}"
        );
        state.autosave_written(Ok(()));
        state.autosave_written(full());
        assert_eq!(state.notices.len(), 2);
    }

    fn address() -> Option<String> {
        crate::sharing::ADDRESS.with(|address| address.borrow().clone())
    }

    fn scene_in(fragment: &str) -> TopologyDocument {
        persistence::parse_document(&crate::sharing::decode(fragment).unwrap()).unwrap()
    }

    /// The live site, before: open a link, choose New, reload, and the link's
    /// scene came back over the new one the autosave held. Since the switch
    /// to version 22 nothing rewrote the link after it opened.
    #[test]
    fn an_opened_link_follows_the_document() {
        let mut state = Playground::default();
        state.open_startup_scene(Some(Ok(example_bytes(3))), || Ok(None));
        state.open_example(2);
        state.refresh_link();
        let expected = &funfern_app::topology_examples::catalog()[2].document;
        assert_eq!(&scene_in(&address().unwrap()), expected);
    }

    /// A link that did not open gives way to the scene that opened instead,
    /// so the address stops failing on every reload.
    #[test]
    fn a_link_that_did_not_open_is_replaced_by_the_scene_that_did() {
        let mut state = Playground::default();
        state.open_startup_scene(Some(Ok(version_21_bytes())), || Ok(Some(example_bytes(2))));
        state.refresh_link();
        let expected = &funfern_app::topology_examples::catalog()[2].document;
        assert_eq!(&scene_in(&address().unwrap()), expected);
    }

    #[test]
    fn without_a_link_the_address_is_left_alone() {
        let mut state = Playground::default();
        state.open_startup_scene(None, || Ok(None));
        state.open_example(2);
        state.refresh_link();
        assert_eq!(address(), None);
    }

    #[test]
    fn a_copied_link_follows_the_document() {
        let mut state = Playground::default();
        state.open_startup_scene(None, || Ok(None));
        state.copy_link(&egui::Context::default());
        assert_eq!(state.message, "Scene link copied");
        state.open_example(4);
        state.refresh_link();
        let expected = &funfern_app::topology_examples::catalog()[4].document;
        assert_eq!(&scene_in(&address().unwrap()), expected);
    }

    #[test]
    fn a_scene_past_the_links_size_takes_the_link_out_of_the_address() {
        let mut state = Playground::default();
        state.open_startup_scene(Some(Ok(example_bytes(3))), || Ok(None));
        state.follow_link(Err(
            "Scene is too large for a shareable link; save it as a file instead".into(),
        ));
        assert_eq!(address(), Some(String::new()));
        assert!(
            matches!(&notices(&state)[..], [("Scene link taken out of the address", text)]
                if text.contains("too large")),
            "{:?}",
            notices(&state)
        );
        // Said once: the address no longer carries a link to keep up.
        state.notices.clear();
        state.refresh_link();
        assert_eq!(address(), Some(String::new()));
        assert!(state.notices.is_empty());
    }
}
