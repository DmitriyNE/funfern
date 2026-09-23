//! The editor resource itself: every piece of state one Playground carries,
//! and the scene it starts on.

use crate::field_paint::FieldPaintTopology;
use crate::files::FileEvent;
use crate::material_overlay::{MaterialOverlayJob, MaterialOverlaySnapshot};
use crate::recording::VideoRecorder;
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy_egui::egui::{self, Rect};
use funfern_app::document::{ProbeId, VectorOverlay};
use funfern_app::topology_editor::{TopologyDocument, TopologyEditor};
use funfern_app::topology_persistence::TopologyLoadCandidate;
use funfern_app::topology_runtime::{TopologyRuntime, TopologyToken};
use funfern_app::topology_viewport::{
    SampledTopologyGeometry, TopologySelection, TopologySpanTarget,
};
use funfern_core::*;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::AtomicUsize;
use std::sync::{
    Arc, Mutex,
    mpsc::{self, Receiver, Sender},
};

use super::events::EventEntry;
use super::gesture::{DragGesture, DrawGesture, PendingMerge};
use super::probe_view::{AreaTrace, CurveTrace, FarFieldTrace, ProbeTrace, ProbeViewState};
use super::workers::{BackgroundAmrWorker, BackgroundPreparationWorker};
use super::*;

#[derive(Resource)]
pub struct Playground {
    pub(super) editor: TopologyEditor,
    pub(super) runtime: TopologyRuntime,
    pub(super) background_preparation: Option<BackgroundPreparationWorker>,
    pub(super) background_amr: Option<BackgroundAmrWorker>,
    pub(super) selection: TopologySelection,
    pub(super) inspector: Option<InspectorPanel>,
    pub(super) draw_open: bool,
    pub(super) closed_purpose: ClosedPurpose,
    pub(super) open_purpose: OpenPurpose,
    pub(super) draw: Option<DrawGesture>,
    pub(super) drag: Option<DragGesture>,
    pub(super) touch_navigation: bool,
    pub(super) touch_active: bool,
    pub(super) suppress_touch_click: bool,
    pub(super) center: Point2,
    pub(super) scale: f64,
    pub(super) fit: bool,
    pub(super) sampled: Option<SampledTopologyGeometry>,
    pub(super) sampled_revision: u64,
    pub(super) sampled_scale: f64,
    pub(super) selected_side: CurveTraceSide,
    pub(super) transform_translation: Point2,
    pub(super) transform_rotation_degrees: f64,
    pub(super) transform_scale: f64,
    pub(super) gizmo_pivot: Option<(BTreeSet<TopologySpanTarget>, Point2)>,
    pub(super) pending_merge: Option<PendingMerge>,
    pub(super) material_selection: MaterialId,
    pub(super) region_selection: RegionId,
    /// Whether a widget held keyboard focus when the previous frame ended. egui
    /// clears focus on Escape before any app code runs, so the live predicate is
    /// already false on the one frame where it matters.
    pub(super) keyboard_focus_previous: bool,
    /// Whether the Materials panel lists compiled faces or material regions, and
    /// which of the two a viewport click picks.
    pub(super) subdomain_listing: SubdomainListing,
    /// Index into `face_assignments` while the panel lists faces.
    pub(super) face_selection: usize,
    pub(super) material_edit: Option<Material>,
    pub(super) material_formula_edits: BTreeMap<(u64, u8), String>,
    pub(super) material_formula_errors: BTreeMap<(u64, u8), String>,
    /// Whether the formula reference is showing. It is a window rather than a
    /// menu so it stays readable while a formula is being typed.
    pub(super) formula_help_open: bool,
    /// Whether the example gallery is showing. It stays open across a pick, so
    /// the catalog can be clicked through.
    pub(super) examples_open: bool,
    /// One thumbnail per catalog entry, built lazily and at most one per frame.
    pub(super) example_previews: Vec<Option<ExamplePreview>>,
    /// The catalog entry the document came from, for the gallery's own marker.
    /// Any other load clears it.
    pub(super) example_opened: Option<usize>,
    pub(super) material_color_edit: Option<(MaterialId, [u8; 3])>,
    pub(super) new_separator_material: MaterialId,
    /// The slider produces a value per frame; the rebuild waits for release.
    pub(super) mesh_edge_dragging: bool,
    /// The Remesh button: rebuild at the current resolution even though
    /// nothing changed, which also leaves an adapted mesh.
    pub(super) remesh_requested: bool,
    pub(super) requested_edge: f64,
    pub(super) requested_revision: Option<u64>,
    pub(super) uploading: Option<Uploading>,
    pub(super) source_commit: Option<PendingSourceCommit>,
    pub(super) gpu_upload_preparation: Option<GpuUploadPreparation>,
    pub(super) wave_running: bool,
    pub(super) wave_step: bool,
    pub(super) reset_requested: bool,
    /// Set when a whole document is replaced: the next preparation must start
    /// the field from zero rather than transfer the outgoing scene's into it.
    /// Separate from `reset_requested` because that one is spent by the GPU
    /// reset below, which runs earlier in the frame and against the topology
    /// still active — the scene being replaced. Sharing one flag let the load
    /// reset the outgoing scene, which then ran on for the seconds its
    /// replacement took to prepare and handed over a full-amplitude field.
    pub(super) fresh_requested: bool,
    pub(super) accumulator: f64,
    /// Largest solver batch a frame may ask for, so the display is never held
    /// behind one. Starts at the floor rather than the ceiling so the first
    /// frames are readings of the display rather than of the solver.
    /// See [`super::pacing::frame_step_budget`].
    pub(super) frame_budget: f64,
    /// Frame interval the display is actually reaching, measured rather than
    /// assumed. See [`DisplayCadence`].
    pub(super) display_cadence: DisplayCadence,
    pub(super) sim_time_offset: f64,
    pub(super) completed_steps: u64,
    pub(super) steps_per_second: f64,
    /// The best simulated-seconds-per-wall-second seen lately, which is what the
    /// shortfall note reads. The raw measurement dips whenever a handoff
    /// withholds stepping inside its window.
    pub(super) speed_reached: f64,
    /// The step the GPU was last uploaded with. Not the active operator's
    /// recommendation: the speed ceiling can ask for a smaller one, and between
    /// a speed change and the republish that carries it the two differ.
    pub(super) uploaded_time_step: f64,
    /// The accepted-step total at the previous observation. Ordinary handovers
    /// preserve it; a fresh install may reset it, so every new generation first
    /// establishes a baseline before its increments are counted.
    pub(super) rate_steps: u64,
    /// The generation `rate_steps` was read from.
    pub(super) rate_generation: u64,
    /// Steps banked since the window opened, across however many generations.
    pub(super) rate_window_steps: u64,
    pub(super) rate_started: Instant,
    pub(super) pulse_mode: bool,
    pub(super) pulse_amplitude: f32,
    pub(super) pulse_width: f32,
    pub(super) pending_pulse: Option<(Point2, RegionId)>,
    pub(super) canonical_event_serial: u32,
    pub(super) canonical_event_observed: u32,
    pub(super) probe_mode: Option<ProbePlacement>,
    pub(super) selected_probe: Option<ProbeId>,
    pub(super) probe_windows: BTreeSet<ProbeId>,
    pub(super) far_field_window: bool,
    pub(super) probe_traces: BTreeMap<ProbeId, ProbeTrace>,
    pub(super) probe_views: BTreeMap<ProbeId, ProbeViewState>,
    pub(super) probe_status: BTreeMap<ProbeId, String>,
    pub(super) probe_metrics: BTreeMap<ProbeId, (f64, bool)>,
    pub(super) probe_anchors: BTreeMap<ProbeId, Point2>,
    pub(super) probe_metadata_token: Option<(Option<TopologyToken>, u64)>,
    pub(super) probe_name_edit: Option<(ProbeId, String)>,
    pub(super) probe_history_seconds: f64,
    pub(super) far_field_view: ProbeViewState,
    pub(super) probe_readback: u64,
    pub(super) curve_probe_readback: u64,
    pub(super) area_probe_readback: u64,
    pub(super) far_field_readback: u64,
    pub(super) curve_probe_traces: BTreeMap<ProbeId, CurveTrace>,
    pub(super) area_probe_traces: BTreeMap<ProbeId, AreaTrace>,
    pub(super) far_field_trace: FarFieldTrace,
    pub(super) logo_texture: Option<egui::TextureHandle>,
    pub(super) message: String,
    pub(super) file_busy: bool,
    pub(super) load: Option<TopologyLoadCandidate>,
    pub(super) sender: Sender<FileEvent>,
    pub(super) receiver: Mutex<Receiver<FileEvent>>,
    pub(super) snapshot_state: SnapshotState,
    pub(super) video_recorder: VideoRecorder,
    pub(super) recording_state: RecordingState,
    pub(super) recording_started: Option<Instant>,
    pub(super) recording_description: String,
    pub(super) recording_dropped_frames: u64,
    pub(super) recording_last_requested_slot: Option<u64>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) recording_readback_in_flight: Arc<AtomicUsize>,
    pub(super) startup_done: bool,
    pub(super) autosave_observed: TopologyDocument,
    pub(super) autosave_due: Option<Instant>,
    pub(super) probe_upload: Option<ProbeUpload>,
    /// The upload before it, kept because the readback it issued is still in
    /// flight when the next one is made, and the samples in it are the last of
    /// the old mesh rather than anything the new one will record again.
    pub(super) probe_upload_previous: Option<ProbeUpload>,
    pub(super) probe_clock_restarted: bool,
    /// Skin whose physical labels and observable meanings own the current
    /// traces. A skin change starts a new history segment rather than joining
    /// differently named fields into one plot.
    pub(super) probe_history_physics: Option<PhysicsModel>,
    /// Simulated time the far-field ring started recording from, or `None` when
    /// no recorder is running.
    pub(super) far_field_recording_from: Option<f64>,
    pub(super) frame_ms: f32,
    pub(super) wave_energy: Option<f64>,
    /// Full-state energy is a diagnostic, not a render input. Recomputing it
    /// over every canonical node and sample at display rate made large meshes
    /// consume a main-thread core even when the diagnostics window was closed.
    pub(super) energy_readback: u64,
    pub(super) energy_updated: Instant,
    pub(super) full_snapshot_requested: Instant,
    pub(super) viewport_rect: Rect,
    /// Latest sampling lattice submitted to the GPU. Camera changes are
    /// coalesced while this revision is in flight instead of continually
    /// replacing the readback before it can complete.
    pub(super) vector_overlay_layout: Option<VectorOverlayLayout>,
    /// World-space lattice corresponding to the last completed readback. It
    /// remains drawable while a camera/remesh replacement is in flight, so
    /// arrows reproject with the view instead of blinking out.
    pub(super) vector_overlay_previous_layout: Option<VectorOverlayLayout>,
    /// Presentation-only DC-blocker state for complementary-field arrows. It
    /// never feeds the canonical solver or physical consumers.
    pub(super) vector_overlay_ac_state: BTreeMap<u32, VectorAcState>,
    pub(super) vector_overlay_ac_owner: Option<VectorOverlayAcOwner>,
    pub(super) vector_overlay_dc_step: u64,
    pub(super) vector_overlay_dc_active: bool,
    pub(super) vector_overlay_mode: VectorOverlay,
    pub(super) vector_overlay_exposure: AutoExposure,
    pub(super) field_exposure: AutoExposure,
    /// Static field-surface geometry. Egui's ordinary mesh path recopies every
    /// index every frame; the paint callback keeps this topology on the GPU.
    pub(super) field_paint_topology: Option<Arc<FieldPaintTopology>>,
    /// Reused by the field's quantile so a frame's sample costs no allocation.
    pub(super) exposure_scratch: Vec<f64>,
    /// Wall-clock seconds since the previous frame, which is what the exposures
    /// release against so they behave the same at any frame rate.
    pub(super) frame_delta: f32,
    pub(super) material_overlay_job: Option<MaterialOverlayJob>,
    pub(super) material_overlay_snapshot: Option<MaterialOverlaySnapshot>,
    pub(super) material_overlay_error: Option<String>,
    pub(super) amr_status: String,
    pub(super) amr_error: Option<String>,
    pub(super) amr_last_started: Option<Instant>,
    pub(super) amr_last_analyzed_step: Option<u64>,
    pub(super) amr_coarsen_streak: u8,
    pub(super) amr_indicator_job: Option<AmrIndicatorJob>,
    pub(super) amr_indicator_completed: Option<Result<SolutionIndicatorResult, AmrIndicatorError>>,
    pub(super) amr_indicator_source: Option<AmrIndicatorSource>,
    pub(super) amr_indicator_result: Option<SolutionIndicatorResult>,
    pub(super) amr_energy_peak: f64,
    pub(super) amr_adaptation_job: Option<MeshAdaptationJob>,
    pub(super) amr_adaptation_completed: Option<Result<MeshAdaptationResult, MeshAdaptationError>>,
    /// Revision of the active mesh the running adaptation started from. The
    /// job is dropped as soon as that mesh is no longer the active one.
    pub(super) amr_adaptation_source: Option<u64>,
    pub(super) amr_adaptation_state: Option<MeshAdaptationState>,
    pub(super) amr_pending_state: Option<MeshAdaptationState>,
    pub(super) amr_report: Option<MeshAdaptationReport>,
    pub(super) gpu_status: &'static str,
    pub(super) gpu_dispatches: u64,
    pub(super) canonical_gpu_bytes: Option<usize>,
    pub(super) step_backlog: u64,
    pub(super) diagnostics_open: bool,
    /// What the transient channels said before they were overwritten, newest
    /// last. Entries are appended when a channel's value changes, which is why
    /// the last value logged from each is kept beside them.
    pub(super) events: VecDeque<EventEntry>,
    pub(super) logged_status: Option<String>,
    pub(super) logged_preparation: Option<String>,
    pub(super) logged_adaptation: Option<String>,
    /// Repair fallbacks a committed transaction reported, waiting for the next
    /// frame to stamp them. A fallback is an event rather than a state, so two
    /// transactions that fall back the same way are two of them.
    pub(super) pending_repairs: Vec<String>,
    /// An error was logged since the diagnostics were last open. Keeps the
    /// status marker lit for an error that clears itself a frame later.
    pub(super) unseen_error: bool,
    pub(super) frame_history: VecDeque<f32>,
    pub(super) handoff_requested: Option<Instant>,
    pub(super) handoff_ready: Option<Instant>,
    pub(super) handoff_packed: Option<Instant>,
    pub(super) handoff_upload: Option<Instant>,
    pub(super) last_handoff: Option<HandoffRecord>,
    pub(super) ready: bool,
}

impl Default for Playground {
    fn default() -> Self {
        let document = funfern_app::topology_examples::catalog()[0]
            .document
            .clone();
        let editor = TopologyEditor::from_document(document.clone()).unwrap_or_default();
        let (sender, receiver) = mpsc::channel();
        Self {
            editor,
            runtime: TopologyRuntime::default(),
            background_preparation: BackgroundPreparationWorker::spawn(),
            background_amr: BackgroundAmrWorker::spawn(),
            selection: TopologySelection::None,
            inspector: Some(InspectorPanel::Edit),
            draw_open: false,
            closed_purpose: ClosedPurpose::Subdomain,
            open_purpose: OpenPurpose::Baffle,
            draw: None,
            drag: None,
            touch_navigation: false,
            touch_active: false,
            suppress_touch_click: false,
            center: Point2::default(),
            scale: 300.0,
            fit: true,
            sampled: None,
            sampled_revision: u64::MAX,
            sampled_scale: 0.0,
            selected_side: CurveTraceSide::Left,
            transform_translation: Point2::default(),
            transform_rotation_degrees: 0.0,
            transform_scale: 1.0,
            gizmo_pivot: None,
            pending_merge: None,
            material_selection: DEFAULT_MATERIAL,
            region_selection: BACKGROUND_REGION,
            keyboard_focus_previous: false,
            subdomain_listing: SubdomainListing::Regions,
            face_selection: 0,
            material_edit: None,
            material_formula_edits: BTreeMap::new(),
            material_formula_errors: BTreeMap::new(),
            formula_help_open: false,
            examples_open: false,
            example_previews: funfern_app::topology_examples::catalog()
                .iter()
                .map(|_| None)
                .collect(),
            example_opened: Some(0),
            material_color_edit: None,
            new_separator_material: DEFAULT_MATERIAL,
            mesh_edge_dragging: false,
            remesh_requested: false,
            requested_edge: f64::NAN,
            requested_revision: None,
            uploading: None,
            source_commit: None,
            gpu_upload_preparation: None,
            wave_running: true,
            wave_step: false,
            reset_requested: false,
            fresh_requested: false,
            accumulator: 0.0,
            frame_budget: 1.0,
            display_cadence: DisplayCadence::new(),
            sim_time_offset: 0.0,
            completed_steps: 0,
            steps_per_second: 0.0,
            speed_reached: 0.0,
            uploaded_time_step: 0.0,
            rate_steps: 0,
            rate_generation: 0,
            rate_window_steps: 0,
            rate_started: Instant::now(),
            pulse_mode: false,
            pulse_amplitude: 1.0,
            pulse_width: 0.06,
            pending_pulse: None,
            canonical_event_serial: 0,
            canonical_event_observed: 0,
            probe_mode: None,
            selected_probe: None,
            probe_windows: BTreeSet::new(),
            far_field_window: false,
            probe_traces: BTreeMap::new(),
            probe_views: BTreeMap::new(),
            probe_status: BTreeMap::new(),
            probe_metrics: BTreeMap::new(),
            probe_anchors: BTreeMap::new(),
            probe_metadata_token: None,
            probe_name_edit: None,
            probe_history_seconds: 10.0,
            far_field_view: ProbeViewState::new(10.0),
            probe_readback: 0,
            curve_probe_readback: 0,
            area_probe_readback: 0,
            far_field_readback: 0,
            curve_probe_traces: BTreeMap::new(),
            area_probe_traces: BTreeMap::new(),
            far_field_trace: FarFieldTrace::default(),
            logo_texture: None,
            message: String::new(),
            file_busy: false,
            load: None,
            sender,
            receiver: Mutex::new(receiver),
            snapshot_state: SnapshotState::Idle,
            video_recorder: VideoRecorder::default(),
            recording_state: RecordingState::Idle,
            recording_started: None,
            recording_description: String::new(),
            recording_dropped_frames: 0,
            recording_last_requested_slot: None,
            #[cfg(not(target_arch = "wasm32"))]
            recording_readback_in_flight: Arc::new(AtomicUsize::new(0)),
            startup_done: false,
            autosave_observed: document,
            autosave_due: None,
            probe_upload: None,
            probe_upload_previous: None,
            probe_clock_restarted: true,
            probe_history_physics: None,
            far_field_recording_from: None,
            frame_ms: 16.0,
            wave_energy: None,
            energy_readback: 0,
            energy_updated: Instant::now(),
            full_snapshot_requested: Instant::now(),
            viewport_rect: Rect::NOTHING,
            vector_overlay_layout: None,
            vector_overlay_previous_layout: None,
            vector_overlay_ac_state: BTreeMap::new(),
            vector_overlay_ac_owner: None,
            vector_overlay_dc_step: u64::MAX,
            vector_overlay_dc_active: false,
            vector_overlay_mode: VectorOverlay::Off,
            vector_overlay_exposure: AutoExposure::default(),
            field_exposure: AutoExposure::default(),
            field_paint_topology: None,
            exposure_scratch: Vec::new(),
            frame_delta: 0.0,
            material_overlay_job: None,
            material_overlay_snapshot: None,
            material_overlay_error: None,
            amr_status: "waiting for solution".into(),
            amr_error: None,
            amr_last_started: None,
            amr_last_analyzed_step: None,
            amr_coarsen_streak: 0,
            amr_indicator_job: None,
            amr_indicator_completed: None,
            amr_indicator_source: None,
            amr_indicator_result: None,
            amr_energy_peak: 0.0,
            amr_adaptation_job: None,
            amr_adaptation_completed: None,
            amr_adaptation_source: None,
            amr_adaptation_state: None,
            amr_pending_state: None,
            amr_report: None,
            gpu_status: "loading",
            gpu_dispatches: 0,
            canonical_gpu_bytes: None,
            step_backlog: 0,
            diagnostics_open: false,
            events: VecDeque::with_capacity(EVENT_LOG_ENTRIES),
            logged_status: None,
            logged_preparation: None,
            logged_adaptation: None,
            pending_repairs: vec![],
            unseen_error: false,
            frame_history: VecDeque::with_capacity(FRAME_HISTORY),
            handoff_requested: None,
            handoff_ready: None,
            handoff_packed: None,
            handoff_upload: None,
            last_handoff: None,
            ready: false,
        }
    }
}
