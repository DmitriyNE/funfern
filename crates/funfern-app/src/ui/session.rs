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

    /// The Undo button. Stepping back over a New, an opened example or a
    /// load swaps the whole scene, and is treated as one.
    pub(super) fn undo(&mut self) {
        let replaces = self.editor.undo_replaces_scene();
        if self.editor.undo() {
            self.history_moved(replaces);
        }
    }

    /// The Redo button, the same way round.
    pub(super) fn redo(&mut self) {
        let replaces = self.editor.redo_replaces_scene();
        if self.editor.redo() {
            self.history_moved(replaces);
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
        self.requested_revision = None;
        self.fresh_requested = fresh;
        self.drop_requested = true;
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
                        self.notify(error);
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
                FileEvent::Error(error) => {
                    self.file_busy = false;
                    self.snapshot_state = SnapshotState::Idle;
                    self.notify(error);
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
                Err(error) => self.notify(error),
            }
        }
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
            let _ = crate::recovery::save(&self.editor.document);
        }
    }
    pub(super) fn save_scene(&mut self) {
        match persistence::save(&self.editor.document) {
            Ok(json) => {
                self.file_busy = true;
                files::save(self.sender.clone(), json.into_bytes(), SaveKind::Scene);
            }
            Err(error) => self.notify(error),
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
                    self.notify(error);
                }
            }
        }
    }
    pub(super) fn copy_link(&mut self, context: &egui::Context) {
        match crate::sharing::encode(&self.editor.document)
            .and_then(|fragment| crate::sharing::link(&fragment))
        {
            Ok(link) => {
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
            (state.uploaded_time_step, state.sim_time_offset),
            (0.0, 0.0)
        );
        assert_eq!(state.amr_status, "waiting for solution");
        settle(&mut state.editor);
        let next = activate(&mut state);
        assert!(matches!(
            next.mesh_action,
            TopologyMeshUpdateAction::FullRebuild(_)
        ));
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
}
