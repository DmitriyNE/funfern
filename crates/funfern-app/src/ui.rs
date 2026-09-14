#![allow(clippy::collapsible_if)]

use crate::files::{self, FileEvent, SaveKind};
use crate::material_overlay::{
    MaterialOverlay, MaterialOverlayJob, MaterialOverlaySnapshot, MaterialProperty, OverlayKey,
    OverlayRange,
};
use crate::recording::{self, DestinationRequest, RecordingEvent, RecordingSpec, VideoRecorder};
use crate::wave_gpu::{
    AreaProbeDisplay, AreaProbeInput, AreaProbeRecord, CurveProbeDisplay, CurveProbeInput,
    CurveProbeRecord, FarFieldDisplay, FarFieldInput, FarFieldRecord, ProbeDisplay, PulseSettings,
    WaveDisplay, WaveGpuRequest, WaveTransfer,
};
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy_egui::{
    EguiContexts,
    egui::{self, Color32, Pos2, Rect, Sense, Stroke},
};
use funfern_app::document::{ProbeId, ProbeSamplingPreset, VectorOverlay};
use funfern_app::topology_editor::{
    ClosedCurvePurpose, OpenCurvePurpose, TopologyAcceptance, TopologyAttachment,
    TopologyBoundaryProbeTarget, TopologyDocument, TopologyEditor, TopologyProbeTarget,
};
use funfern_app::topology_persistence::{self as persistence, TopologyLoadCandidate};
use funfern_app::topology_runtime::{
    PreparedTopology, TopologyProbeCompilation, TopologyProbeStencil, TopologyRuntime,
    TopologyToken,
};
use funfern_app::topology_viewport::{
    RigidTransform, SampledTopologyGeometry, ScreenPoint, TopologyHandle, TopologyHit,
    TopologySelection, TopologySpanTarget, ViewportTransform, hit_attachment, plan_handle_drag,
    plan_rigid_transform, span_context,
};
use funfern_core::*;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{
    Arc, Mutex,
    mpsc::{self, Receiver, Sender},
};

const TEAL: Color32 = Color32::from_rgb(91, 220, 194);
const SELECT: Color32 = Color32::from_rgb(72, 166, 255);
const RED: Color32 = Color32::from_rgb(255, 106, 123);
const GOLD: Color32 = Color32::from_rgb(248, 196, 112);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum InspectorPanel {
    #[default]
    Edit,
    View,
    Simulation,
    Materials,
    Probes,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ClosedPurpose {
    #[default]
    Subdomain,
    Hole,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OpenPurpose {
    #[default]
    Separator,
    Baffle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BoundaryKind {
    Reflecting,
    FirstOrder,
    SecondOrder,
    ElectricWall,
    MagneticWall,
    Neumann,
    Dirichlet,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DrawTool {
    Circle,
    Rectangle,
    Polygon,
    ClosedSpline,
    Polyline,
    OpenSpline,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ProbePlacement {
    Point,
    Segment { start: Option<Point2> },
    Disk { center: Option<Point2> },
    Region,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SnapshotState {
    #[default]
    Idle,
    Requested,
    Armed,
    Capturing,
    Saving,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum RecordingState {
    #[default]
    Idle,
    SelectingDestination,
    Requested,
    Preparing,
    Starting,
    Recording,
    Finalizing,
}

#[derive(Clone, Debug)]
struct DrawGesture {
    tool: DrawTool,
    points: Vec<Point2>,
    attachments: Vec<Option<TopologyAttachment>>,
    face: Option<FaceId>,
}

#[derive(Clone, Debug)]
enum DragGesture {
    Handle {
        handle: TopologyHandle,
    },
    Spans {
        start: Point2,
        geometry: TopologyGeometry,
    },
    Rotate {
        pivot: Point2,
        start_angle: f64,
        geometry: TopologyGeometry,
    },
    Scale {
        pivot: Point2,
        start_distance: f64,
        geometry: TopologyGeometry,
    },
    Marquee {
        start: Pos2,
        base: TopologySelection,
    },
    Source,
    Probe(ProbeId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransformGizmoHit {
    Rotate,
    Scale,
}

#[derive(Clone, Debug)]
struct ProbeTrace {
    samples: VecDeque<(f64, f64)>,
    last_time: f64,
}

struct CurveTrace {
    records: VecDeque<CurveProbeRecord>,
    last_time: f64,
}
impl Default for CurveTrace {
    fn default() -> Self {
        Self {
            records: VecDeque::new(),
            last_time: -1.0,
        }
    }
}

struct AreaTrace {
    records: VecDeque<AreaProbeRecord>,
    last_time: f64,
}
impl Default for AreaTrace {
    fn default() -> Self {
        Self {
            records: VecDeque::new(),
            last_time: -1.0,
        }
    }
}

struct FarFieldTrace {
    records: VecDeque<FarFieldRecord>,
    last_time: f64,
}
impl Default for FarFieldTrace {
    fn default() -> Self {
        Self {
            records: VecDeque::new(),
            last_time: -1.0,
        }
    }
}
impl Default for ProbeTrace {
    fn default() -> Self {
        Self {
            samples: VecDeque::new(),
            last_time: -1.0,
        }
    }
}

struct Uploading {
    token: TopologyToken,
    generation: u64,
    fresh: bool,
    time_offset: f64,
}

#[derive(Resource)]
pub struct Playground {
    editor: TopologyEditor,
    runtime: TopologyRuntime,
    selection: TopologySelection,
    inspector: Option<InspectorPanel>,
    draw_open: bool,
    closed_purpose: ClosedPurpose,
    open_purpose: OpenPurpose,
    draw: Option<DrawGesture>,
    drag: Option<DragGesture>,
    touch_navigation: bool,
    suppress_touch_click: bool,
    center: Point2,
    scale: f64,
    fit: bool,
    sampled: Option<SampledTopologyGeometry>,
    sampled_revision: u64,
    sampled_scale: f64,
    selected_side: CurveTraceSide,
    transform_translation: Point2,
    transform_rotation_degrees: f64,
    transform_scale: f64,
    material_selection: MaterialId,
    region_selection: RegionId,
    material_edit: Option<Material>,
    material_formula_edits: BTreeMap<(u64, u8), String>,
    material_formula_errors: BTreeMap<(u64, u8), String>,
    new_separator_material: MaterialId,
    mesh_edge: f64,
    requested_edge: f64,
    requested_revision: Option<u64>,
    uploading: Option<Uploading>,
    wave_running: bool,
    wave_step: bool,
    reset_requested: bool,
    accumulator: f64,
    sim_time_offset: f64,
    completed_steps: u64,
    steps_per_second: f64,
    rate_steps: u64,
    rate_started: Instant,
    pulse_mode: bool,
    pulse_amplitude: f32,
    pulse_width: f32,
    pending_pulse: Option<(Point2, RegionId)>,
    probe_mode: Option<ProbePlacement>,
    selected_probe: Option<ProbeId>,
    probe_windows: BTreeSet<ProbeId>,
    far_field_window: bool,
    probe_traces: BTreeMap<ProbeId, ProbeTrace>,
    probe_readback: u64,
    curve_probe_readback: u64,
    area_probe_readback: u64,
    far_field_readback: u64,
    curve_probe_traces: BTreeMap<ProbeId, CurveTrace>,
    area_probe_traces: BTreeMap<ProbeId, AreaTrace>,
    far_field_trace: FarFieldTrace,
    logo_texture: Option<egui::TextureHandle>,
    message: String,
    file_busy: bool,
    load: Option<TopologyLoadCandidate>,
    sender: Sender<FileEvent>,
    receiver: Mutex<Receiver<FileEvent>>,
    snapshot_state: SnapshotState,
    video_recorder: VideoRecorder,
    recording_state: RecordingState,
    recording_started: Option<Instant>,
    recording_description: String,
    recording_dropped_frames: u64,
    recording_last_requested_slot: Option<u64>,
    #[cfg(not(target_arch = "wasm32"))]
    recording_readback_in_flight: Arc<AtomicBool>,
    startup_done: bool,
    autosave_observed: TopologyDocument,
    autosave_due: Option<Instant>,
    probe_gpu_token: Option<TopologyToken>,
    frame_ms: f32,
    wave_energy: Option<f64>,
    vector_overlay_average: BTreeMap<(i32, i32), Point2>,
    vector_overlay_step: u64,
    vector_overlay_mode: VectorOverlay,
    vector_overlay_peak_reference: f64,
    material_overlay_job: Option<MaterialOverlayJob>,
    material_overlay_snapshot: Option<MaterialOverlaySnapshot>,
    material_overlay_error: Option<String>,
    amr_enabled: bool,
    amr_minimum_edge: f64,
    amr_maximum_edge: f64,
    amr_status: String,
    amr_error: Option<String>,
    amr_last_started: Option<Instant>,
    amr_last_analyzed_step: Option<u64>,
    amr_coarsen_streak: u8,
    amr_indicator_job: Option<SolutionIndicatorJob>,
    amr_indicator_source: Option<(TopologyToken, u64, u64, u64)>,
    amr_indicator_result: Option<SolutionIndicatorResult>,
    amr_adaptation_job: Option<MeshAdaptationJob>,
    amr_adaptation_state: Option<MeshAdaptationState>,
    amr_pending_state: Option<MeshAdaptationState>,
    ready: bool,
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
            selection: TopologySelection::None,
            inspector: Some(InspectorPanel::Edit),
            draw_open: false,
            closed_purpose: ClosedPurpose::Subdomain,
            open_purpose: OpenPurpose::Baffle,
            draw: None,
            drag: None,
            touch_navigation: false,
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
            material_selection: DEFAULT_MATERIAL,
            region_selection: BACKGROUND_REGION,
            material_edit: None,
            material_formula_edits: BTreeMap::new(),
            material_formula_errors: BTreeMap::new(),
            new_separator_material: DEFAULT_MATERIAL,
            mesh_edge: 0.08,
            requested_edge: f64::NAN,
            requested_revision: None,
            uploading: None,
            wave_running: true,
            wave_step: false,
            reset_requested: false,
            accumulator: 0.0,
            sim_time_offset: 0.0,
            completed_steps: 0,
            steps_per_second: 0.0,
            rate_steps: 0,
            rate_started: Instant::now(),
            pulse_mode: false,
            pulse_amplitude: 1.0,
            pulse_width: 0.06,
            pending_pulse: None,
            probe_mode: None,
            selected_probe: None,
            probe_windows: BTreeSet::new(),
            far_field_window: false,
            probe_traces: BTreeMap::new(),
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
            recording_readback_in_flight: Arc::new(AtomicBool::new(false)),
            startup_done: false,
            autosave_observed: document,
            autosave_due: None,
            probe_gpu_token: None,
            frame_ms: 16.0,
            wave_energy: None,
            vector_overlay_average: BTreeMap::new(),
            vector_overlay_step: u64::MAX,
            vector_overlay_mode: VectorOverlay::Off,
            vector_overlay_peak_reference: 0.0,
            material_overlay_job: None,
            material_overlay_snapshot: None,
            material_overlay_error: None,
            amr_enabled: true,
            amr_minimum_edge: 0.02,
            amr_maximum_edge: 0.16,
            amr_status: "waiting for solution".into(),
            amr_error: None,
            amr_last_started: None,
            amr_last_analyzed_step: None,
            amr_coarsen_streak: 0,
            amr_indicator_job: None,
            amr_indicator_source: None,
            amr_indicator_result: None,
            amr_adaptation_job: None,
            amr_adaptation_state: None,
            amr_pending_state: None,
            ready: false,
        }
    }
}

impl Playground {
    fn transform(&self, viewport: Rect) -> ViewportTransform {
        ViewportTransform {
            screen_center: ScreenPoint::new(viewport.center().x as f64, viewport.center().y as f64),
            world_center: self.center,
            pixels_per_world: self.scale,
        }
    }
    fn screen(&self, point: Point2, viewport: Rect) -> Pos2 {
        let p = self.transform(viewport).world_to_screen(point);
        Pos2::new(p.x as f32, p.y as f32)
    }
    fn world(&self, point: Pos2, viewport: Rect) -> Point2 {
        self.transform(viewport)
            .screen_to_world(ScreenPoint::new(point.x as f64, point.y as f64))
    }
    fn fit_view(&mut self, viewport: Rect) {
        let domain = self.editor.document.model.draft.geometry.domain;
        self.center = domain.center();
        self.scale = (viewport.width() as f64 / domain.width())
            .min(viewport.height() as f64 / domain.height())
            * 0.88;
        self.fit = false;
    }
    fn invalidate_samples(&mut self) {
        self.sampled_revision = u64::MAX;
    }
    fn refresh_samples(&mut self, viewport: Rect) {
        if self.sampled_revision == self.editor.revision && self.sampled_scale == self.scale {
            return;
        }
        match SampledTopologyGeometry::new(
            &self.editor.document.model.draft.geometry,
            self.transform(viewport),
            0.6,
        ) {
            Ok(sampled) => {
                self.sampled = Some(sampled);
                self.sampled_revision = self.editor.revision;
                self.sampled_scale = self.scale;
            }
            Err(_) => self.sampled = None,
        }
    }
    fn notify(&mut self, message: impl Into<String>) {
        self.message = message.into();
    }
    fn begin_draw(&mut self, tool: DrawTool) {
        self.draw = Some(DrawGesture {
            tool,
            points: vec![],
            attachments: vec![],
            face: None,
        });
        self.draw_open = false;
        self.selection = TopologySelection::None;
    }
    fn cancel_interaction(&mut self) {
        if self.editor.editing() {
            self.editor.cancel();
        }
        self.drag = None;
        self.draw = None;
        self.pulse_mode = false;
        self.probe_mode = None;
        self.invalidate_samples();
    }
    fn set_document(
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
        self.selection = TopologySelection::None;
        self.selected_probe = None;
        self.draw = None;
        self.requested_revision = None;
        self.reset_requested = fresh;
        self.invalidate_samples();
        Ok(())
    }
    fn update_files(&mut self) {
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
    fn autosave(&mut self) {
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
    fn save_scene(&mut self) {
        match persistence::save(&self.editor.document) {
            Ok(json) => {
                self.file_busy = true;
                files::save(self.sender.clone(), json.into_bytes(), SaveKind::Scene);
            }
            Err(error) => self.notify(error),
        }
    }
    fn export_viewport_png(&mut self) {
        if self.snapshot_state == SnapshotState::Idle
            && self.recording_state == RecordingState::Idle
        {
            // The request is armed at the start of the next frame. This gives
            // egui one complete frame to close the File menu before readback.
            self.snapshot_state = SnapshotState::Requested;
            self.file_busy = true;
        }
    }
    fn request_video_recording(&mut self) {
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
    fn stop_video_recording(&mut self) {
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
    fn begin_capture_frame(&mut self) {
        if self.snapshot_state == SnapshotState::Requested {
            self.snapshot_state = SnapshotState::Armed;
        }
        if self.recording_state == RecordingState::Requested {
            self.recording_state = RecordingState::Preparing;
        }
    }
    fn update_recording(&mut self) {
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
                            self.recording_readback_in_flight = Arc::new(AtomicBool::new(false));
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
                        .store(false, Ordering::Release);
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
                        .store(false, Ordering::Release);
                    self.notify(error);
                }
            }
        }
    }
    fn copy_link(&mut self, context: &egui::Context) {
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
    fn top_bar(&mut self, root: &mut egui::Ui) {
        let fold_panels = root.available_width() < 1080.0;
        egui::Panel::top("top").exact_size(42.0).show(root, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Undo").clicked() && self.editor.undo() {
                    self.material_edit = None;
                    self.material_formula_edits.clear();
                    self.material_formula_errors.clear();
                    self.invalidate_samples();
                }
                if ui.button("Redo").clicked() && self.editor.redo() {
                    self.material_edit = None;
                    self.material_formula_edits.clear();
                    self.material_formula_errors.clear();
                    self.invalidate_samples();
                }
                ui.menu_button("File", |ui| {
                    if ui.button("Open…").clicked() {
                        self.file_busy = true;
                        files::load(self.sender.clone());
                        ui.close();
                    }
                    if ui.button("Save…").clicked() {
                        self.save_scene();
                        ui.close();
                    }
                    if ui.button("Copy scene link").clicked() {
                        self.copy_link(ui.ctx());
                        ui.close();
                    }
                    let capture_ready = self.snapshot_state == SnapshotState::Idle
                        && self.recording_state == RecordingState::Idle;
                    if ui
                        .add_enabled(capture_ready, egui::Button::new("Export viewport PNG"))
                        .clicked()
                    {
                        self.export_viewport_png();
                        ui.close();
                    }
                    let recording_label = if matches!(
                        self.recording_state,
                        RecordingState::Starting | RecordingState::Recording
                    ) {
                        "Stop recording"
                    } else if self.recording_state != RecordingState::Idle {
                        "Preparing recording…"
                    } else {
                        "Record viewport"
                    };
                    if ui
                        .add_enabled(
                            capture_ready
                                || matches!(
                                    self.recording_state,
                                    RecordingState::Starting | RecordingState::Recording
                                ),
                            egui::Button::new(recording_label),
                        )
                        .clicked()
                    {
                        if self.recording_state == RecordingState::Idle {
                            self.request_video_recording();
                        } else {
                            self.stop_video_recording();
                        }
                        ui.close();
                    }
                    ui.separator();
                    ui.menu_button("Examples", |ui| {
                        for example in funfern_app::topology_examples::catalog() {
                            if ui
                                .button(example.name)
                                .on_hover_text(example.description)
                                .clicked()
                            {
                                if let Err(error) =
                                    self.set_document(example.document.clone(), true, true)
                                {
                                    self.notify(error);
                                }
                                ui.close();
                            }
                        }
                    });
                });
                if ui.button("Fit view").clicked() {
                    self.fit = true;
                }
                let panels = [
                    (InspectorPanel::Edit, "Edit"),
                    (InspectorPanel::View, "View"),
                    (InspectorPanel::Simulation, "Simulation"),
                    (InspectorPanel::Materials, "Materials"),
                    (InspectorPanel::Probes, "Probes"),
                ];
                if fold_panels {
                    ui.menu_button("Panels", |ui| {
                        for (panel, label) in panels {
                            let selected = self.inspector == Some(panel);
                            if ui.selectable_label(selected, label).clicked() {
                                self.inspector = (!selected).then_some(panel);
                            }
                        }
                    });
                } else {
                    for (panel, label) in panels {
                        let selected = self.inspector == Some(panel);
                        if ui.selectable_label(selected, label).clicked() {
                            self.inspector = (!selected).then_some(panel);
                        }
                    }
                }
                if ui.button("+ Draw").clicked() {
                    self.draw_open = !self.draw_open;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Reset").clicked() {
                        self.reset_requested = true;
                    }
                    if ui.button("Step").clicked() {
                        self.wave_step = true;
                    }
                    if ui
                        .button(if self.wave_running { "Pause" } else { "Run" })
                        .clicked()
                    {
                        self.wave_running = !self.wave_running;
                    }
                });
            });
        });
        if self.draw_open {
            let ctx = root.ctx().clone();
            egui::Window::new("Draw")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::LEFT_TOP, [300.0, 42.0])
                .show(&ctx, |ui| {
                    ui.label("Closed curve");
                    ui.horizontal(|ui| {
                        ui.radio_value(
                            &mut self.closed_purpose,
                            ClosedPurpose::Subdomain,
                            "Subdomain",
                        );
                        ui.radio_value(&mut self.closed_purpose, ClosedPurpose::Hole, "Hole");
                    });
                    ui.horizontal(|ui| {
                        for (tool, label) in [
                            (DrawTool::Circle, "Circle"),
                            (DrawTool::Rectangle, "Rectangle"),
                            (DrawTool::Polygon, "Polygon"),
                            (DrawTool::ClosedSpline, "Spline"),
                        ] {
                            if ui.button(label).clicked() {
                                self.begin_draw(tool);
                            }
                        }
                    });
                    ui.separator();
                    ui.label("Open curve");
                    ui.horizontal(|ui| {
                        ui.radio_value(
                            &mut self.open_purpose,
                            OpenPurpose::Separator,
                            "Subdomain separator",
                        );
                        ui.radio_value(&mut self.open_purpose, OpenPurpose::Baffle, "BC baffle");
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Polyline").clicked() {
                            self.begin_draw(DrawTool::Polyline);
                        }
                        if ui.button("Spline").clicked() {
                            self.begin_draw(DrawTool::OpenSpline);
                        }
                    });
                });
        }
    }
    fn side_panel(&mut self, root: &mut egui::Ui) {
        let Some(panel) = self.inspector else { return };
        if root.available_width() < 700.0 {
            let mut open = true;
            egui::Window::new(match panel {
                InspectorPanel::Edit => "Edit",
                InspectorPanel::View => "View",
                InspectorPanel::Simulation => "Simulation",
                InspectorPanel::Materials => "Materials",
                InspectorPanel::Probes => "Probes",
            })
            .id(egui::Id::new("mobile-inspector"))
            .open(&mut open)
            .default_width(280.0)
            .anchor(egui::Align2::RIGHT_TOP, [-6.0, 48.0])
            .show(root.ctx(), |ui| match panel {
                InspectorPanel::Edit => self.edit_panel(ui),
                InspectorPanel::View => self.view_panel(ui),
                InspectorPanel::Simulation => self.simulation_panel(ui),
                InspectorPanel::Materials => self.materials_panel(ui),
                InspectorPanel::Probes => self.probes_panel(ui),
            });
            if !open {
                self.inspector = None;
            }
            return;
        }
        egui::Panel::right("inspector")
            .default_size(292.0)
            .show(root, |ui| match panel {
                InspectorPanel::Edit => self.edit_panel(ui),
                InspectorPanel::View => self.view_panel(ui),
                InspectorPanel::Simulation => self.simulation_panel(ui),
                InspectorPanel::Materials => self.materials_panel(ui),
                InspectorPanel::Probes => self.probes_panel(ui),
            });
    }
    fn edit_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Edit");
        ui.collapsing("Features", |ui| {
            if ui.selectable_label(matches!(self.selection, TopologySelection::Spans(ref s) if s.iter().all(|v| matches!(v, TopologySpanTarget::Outer(_)))), "Outer boundary").clicked() {
                self.selection = TopologySelection::Spans(OuterSide::ALL.into_iter().map(TopologySpanTarget::Outer).collect());
            }
            let curves = self.editor.document.model.draft.geometry.curves.iter().map(|curve| (curve.id, curve.spline.is_open(), curve.spans.iter().map(|span| span.id).collect::<Vec<_>>())).collect::<Vec<_>>();
            for (curve, open, spans) in curves {
                let selected = matches!(&self.selection, TopologySelection::Spans(selection) if spans.iter().all(|span| selection.contains(&TopologySpanTarget::Curve(*span))));
                if ui.selectable_label(selected, format!("{} {}", if open { "Open curve" } else { "Closed curve" }, curve.0)).clicked() {
                    self.selection = TopologySelection::Spans(spans.into_iter().map(TopologySpanTarget::Curve).collect());
                }
            }
        });
        ui.separator();
        match self.selection.clone() {
            TopologySelection::None => {
                ui.weak("Select a control, junction, span, or face");
            }
            TopologySelection::Handle(handle) => self.handle_inspector(ui, handle),
            TopologySelection::Spans(spans) => self.span_inspector(ui, spans),
        }
    }
    fn handle_inspector(&mut self, ui: &mut egui::Ui, handle: TopologyHandle) {
        let geometry = &self.editor.document.model.draft.geometry;
        let point = match handle {
            TopologyHandle::Control { curve, control } => geometry
                .curves
                .iter()
                .find(|item| item.id == curve)
                .and_then(|curve| match &curve.spline {
                    CurveSpline::Closed(s) => s.controls().get(control),
                    CurveSpline::Open(s) => s.controls().get(control),
                })
                .copied(),
            TopologyHandle::Junction(vertex) => geometry
                .vertices
                .iter()
                .find(|item| item.id == vertex)
                .and_then(|vertex| vertex.point(geometry.domain)),
        };
        let Some(mut point) = point else { return };
        ui.label(match handle {
            TopologyHandle::Control { curve, control } => {
                format!("Control {} · Curve {}", control + 1, curve.0)
            }
            TopologyHandle::Junction(vertex) => format!("Junction {}", vertex.0),
        });
        let before = point;
        ui.horizontal(|ui| {
            ui.label("x");
            ui.add(egui::DragValue::new(&mut point.x).speed(0.005));
            ui.label("y");
            ui.add(egui::DragValue::new(&mut point.y).speed(0.005));
        });
        if point != before {
            let result =
                plan_handle_drag(&self.editor.document.model.draft.geometry, handle, point)
                    .map_err(|e| e.to_string())
                    .and_then(|update| self.editor.apply_transform_updates(&[update]));
            match result {
                Ok(()) => self.invalidate_samples(),
                Err(error) => self.notify(error),
            }
        }
        if let TopologyHandle::Control { curve, control } = handle
            && ui.button("Delete control").clicked()
        {
            match self.editor.remove_control(curve, control) {
                Ok(()) => {
                    self.selection = TopologySelection::None;
                    self.invalidate_samples();
                }
                Err(error) => self.notify(error),
            }
        }
        if let TopologyHandle::Junction(vertex) = handle {
            let incident = funfern_app::topology_viewport::incident_spans(
                &self.editor.document.model.draft.geometry,
                vertex,
            );
            if ui.button("Select incident spans").clicked() {
                self.selection = TopologySelection::Spans(
                    incident
                        .into_iter()
                        .map(TopologySpanTarget::Curve)
                        .collect(),
                );
            }
            let endpoints = self
                .editor
                .document
                .model
                .draft
                .geometry
                .curves
                .iter()
                .filter_map(|curve| {
                    if !curve.spline.is_open() {
                        return None;
                    }
                    if curve
                        .nodes
                        .first()
                        .is_some_and(|node| node.vertex == Some(vertex))
                    {
                        Some((curve.id, 0))
                    } else if curve
                        .nodes
                        .last()
                        .is_some_and(|node| node.vertex == Some(vertex))
                    {
                        Some((curve.id, 1))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            for (curve, endpoint) in endpoints {
                if ui
                    .button(format!(
                        "Detach curve {} {}",
                        curve.0,
                        if endpoint == 0 { "start" } else { "end" }
                    ))
                    .clicked()
                {
                    match self.editor.detach_endpoint(curve, endpoint) {
                        Ok(()) => {
                            self.selection = TopologySelection::None;
                            self.invalidate_samples();
                        }
                        Err(error) => self.notify(error),
                    }
                }
            }
        }
    }
    fn span_inspector(&mut self, ui: &mut egui::Ui, spans: BTreeSet<TopologySpanTarget>) {
        ui.label(format!(
            "{} span{}",
            spans.len(),
            if spans.len() == 1 { "" } else { "s" }
        ));
        let curve_spans = spans
            .iter()
            .filter_map(|target| match target {
                TopologySpanTarget::Curve(span) => Some(*span),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        if !curve_spans.is_empty() {
            ui.horizontal(|ui| {
                if ui.button("Transmit").clicked() {
                    if let Err(error) = self
                        .editor
                        .set_span_behavior(&curve_spans, SpanBehavior::Transmitting)
                    {
                        self.notify(error);
                    }
                }
                if ui.button("Boundary").clicked() {
                    if let Err(error) = self
                        .editor
                        .set_span_behavior(&curve_spans, SpanBehavior::REFLECTING)
                    {
                        self.notify(error);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.selected_side, CurveTraceSide::Left, "Left");
                ui.selectable_value(&mut self.selected_side, CurveTraceSide::Right, "Right");
            });
            let first_behavior = self
                .editor
                .document
                .model
                .draft
                .geometry
                .curves
                .iter()
                .flat_map(|curve| &curve.spans)
                .find(|span| curve_spans.contains(&span.id))
                .map(|span| span.behavior);
            if let Some(behavior) = first_behavior {
                let mut condition = match behavior {
                    SpanBehavior::Transmitting => FaceBoundaryCondition::Reflecting,
                    SpanBehavior::Separated { left, right, .. } => match self.selected_side {
                        CurveTraceSide::Left => left,
                        CurveTraceSide::Right => right,
                    },
                };
                if edit_face_condition(ui, &mut condition)
                    && let Err(error) = self.editor.set_span_face_condition(
                        &curve_spans,
                        self.selected_side,
                        condition,
                    )
                {
                    self.notify(error);
                }
                let (mut thin_gap, mut stiffness) = match behavior {
                    SpanBehavior::Separated {
                        coupling: InternalBoundaryCoupling::ThinGap { stiffness_ratio },
                        ..
                    } => (true, stiffness_ratio),
                    _ => (false, 1.0),
                };
                let gap_changed = ui.checkbox(&mut thin_gap, "Thin gap coupling").changed();
                let stiffness_changed = thin_gap
                    && ui
                        .add(
                            egui::DragValue::new(&mut stiffness)
                                .speed(0.02)
                                .range(1.0e-6..=1.0e6)
                                .prefix("Stiffness "),
                        )
                        .changed();
                if (gap_changed || stiffness_changed)
                    && let Err(error) = self.editor.set_span_coupling(
                        &curve_spans,
                        if thin_gap {
                            InternalBoundaryCoupling::ThinGap {
                                stiffness_ratio: stiffness,
                            }
                        } else {
                            InternalBoundaryCoupling::Independent
                        },
                    )
                {
                    self.notify(error);
                }
            }
            if curve_spans.len() == 1 {
                let span = *curve_spans.first().unwrap();
                if let Some(context) = self
                    .editor
                    .compiled_draft
                    .as_ref()
                    .and_then(|scene| span_context(scene, span))
                {
                    ui.weak(format!(
                        "{} side · {}",
                        if self.selected_side == CurveTraceSide::Left {
                            "Left"
                        } else {
                            "Right"
                        },
                        if match self.selected_side {
                            CurveTraceSide::Left => context.left.active,
                            CurveTraceSide::Right => context.right.active,
                        } {
                            "active"
                        } else {
                            "excluded"
                        }
                    ));
                }
            }
            let transform = RigidTransform {
                pivot: self.selection_pivot(&curve_spans).unwrap_or_default(),
                translation: Point2::default(),
                rotation_radians: 0.0,
                scale: 1.0,
            };
            let transform_allowed = match plan_rigid_transform(
                &self.editor.document.model.draft.geometry,
                &spans,
                transform,
            ) {
                Ok(_) => true,
                Err(issue) => {
                    ui.colored_label(GOLD, issue.to_string());
                    if let funfern_app::topology_viewport::TopologyTransformIssue::PartialJunction {
                        vertex,
                        ..
                    } = issue
                    {
                        if ui.button("Select incident spans").clicked() {
                            self.selection.select_incident_spans(
                                &self.editor.document.model.draft.geometry,
                                vertex,
                            );
                        }
                    }
                    false
                }
            };
            if transform_allowed {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.transform_translation.x)
                            .speed(0.01)
                            .prefix("Δx "),
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.transform_translation.y)
                            .speed(0.01)
                            .prefix("Δy "),
                    );
                });
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.transform_rotation_degrees)
                            .speed(0.5)
                            .suffix("°"),
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.transform_scale)
                            .speed(0.01)
                            .range(0.01..=100.0)
                            .prefix("Scale "),
                    );
                });
                if ui.button("Apply transform").clicked() {
                    let transform = RigidTransform {
                        pivot: self.selection_pivot(&curve_spans).unwrap_or_default(),
                        translation: self.transform_translation,
                        rotation_radians: self.transform_rotation_degrees.to_radians(),
                        scale: self.transform_scale,
                    };
                    match plan_rigid_transform(
                        &self.editor.document.model.draft.geometry,
                        &spans,
                        transform,
                    )
                    .map_err(|error| error.to_string())
                    .and_then(|updates| self.editor.apply_transform_updates(&updates))
                    {
                        Ok(()) => {
                            self.transform_translation = Point2::default();
                            self.transform_rotation_degrees = 0.0;
                            self.transform_scale = 1.0;
                            self.invalidate_samples();
                        }
                        Err(error) => self.notify(error),
                    }
                }
            }
            let curve_ids = self.selected_complete_curves(&curve_spans);
            for curve in curve_ids {
                if ui.button(format!("Delete curve {}", curve.0)).clicked() {
                    match self.editor.remove_curve(curve, None) {
                        Ok(_) => {
                            self.selection = TopologySelection::None;
                            self.invalidate_samples();
                        }
                        Err(error) => self.notify(error),
                    }
                }
            }
        }
        let outer = spans
            .iter()
            .filter_map(|target| match target {
                TopologySpanTarget::Outer(side) => Some(*side),
                _ => None,
            })
            .collect::<Vec<_>>();
        if !outer.is_empty() {
            let sides = outer.iter().copied().collect::<BTreeSet<_>>();
            let mut condition =
                self.editor.document.model.draft.outer_boundaries.sides[outer[0].index()];
            if edit_outer_condition(ui, &mut condition)
                && let Err(error) = self.editor.set_outer_condition(&sides, condition)
            {
                self.notify(error);
            }
            let mut domain = self.editor.document.model.draft.geometry.domain;
            let before = domain;
            ui.label("Domain extents");
            ui.horizontal(|ui| {
                ui.label("Left");
                ui.add(egui::DragValue::new(&mut domain.min_x).speed(0.01));
                ui.label("Right");
                ui.add(egui::DragValue::new(&mut domain.max_x).speed(0.01));
            });
            ui.horizontal(|ui| {
                ui.label("Bottom");
                ui.add(egui::DragValue::new(&mut domain.min_y).speed(0.01));
                ui.label("Top");
                ui.add(egui::DragValue::new(&mut domain.max_y).speed(0.01));
            });
            if domain != before {
                match self.editor.set_domain(domain) {
                    Ok(()) => self.invalidate_samples(),
                    Err(error) => self.notify(error),
                }
            }
        }
    }
    fn selection_pivot(&self, spans: &BTreeSet<CurveSpanId>) -> Option<Point2> {
        let mut sum = Point2::default();
        let mut count = 0usize;
        for curve in &self.editor.document.model.draft.geometry.curves {
            for (index, span) in curve.spans.iter().enumerate() {
                if spans.contains(&span.id) {
                    let [a, b] = curve.spline.span_bounds(index)?;
                    sum = sum
                        + match &curve.spline {
                            CurveSpline::Closed(s) => s.evaluate((a + b) * 0.5),
                            CurveSpline::Open(s) => s.evaluate((a + b) * 0.5),
                        };
                    count += 1;
                }
            }
        }
        (count > 0).then(|| sum / count as f64)
    }
    fn selected_complete_curves(&self, spans: &BTreeSet<CurveSpanId>) -> Vec<CurveId> {
        self.editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .filter(|curve| curve.spans.iter().all(|span| spans.contains(&span.id)))
            .map(|curve| curve.id)
            .collect()
    }
    fn view_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("View");
        let overlay_error = self.material_overlay_error.clone();
        let overlay_progress = self.material_overlay_job.as_ref().map(|job| job.progress());
        let overlay_invalid = match self.editor.document.presentation.material_overlay {
            MaterialOverlay::Property(property) => self
                .material_overlay_snapshot
                .as_ref()
                .map(|snapshot| snapshot.invalid_count(property)),
            _ => None,
        };
        let p = &mut self.editor.document.presentation;
        ui.checkbox(&mut p.grid, "Grid");
        ui.checkbox(&mut p.control_polygons, "Control polygons");
        ui.checkbox(&mut p.handles, "Handles");
        ui.checkbox(&mut p.boundary_conditions, "Boundary conditions");
        ui.checkbox(&mut p.mesh, "Mesh");
        ui.checkbox(&mut p.mesh_boundaries, "Mesh boundaries");
        ui.checkbox(&mut p.adaptation_target, "Adaptation target");
        ui.checkbox(&mut p.field, "Field");
        ui.add(egui::Slider::new(&mut p.field_gain, 0.25..=12.0).text("Field intensity"));
        egui::ComboBox::from_id_salt("vector-overlay")
            .selected_text(
                p.vector_overlay
                    .label(self.editor.document.model.draft.physics),
            )
            .show_ui(ui, |ui| {
                for mode in [
                    VectorOverlay::Off,
                    VectorOverlay::ComplementaryField,
                    VectorOverlay::RelativeEnergyFlow,
                ] {
                    ui.selectable_value(
                        &mut p.vector_overlay,
                        mode,
                        mode.label(self.editor.document.model.draft.physics),
                    );
                }
            });
        if p.vector_overlay != VectorOverlay::Off {
            ui.checkbox(&mut p.vector_overlay_smoothed, "Smooth arrows");
            ui.add(
                egui::Slider::new(&mut p.vector_overlay_density, 28.0..=120.0)
                    .text("Arrow spacing"),
            );
            ui.add(egui::Slider::new(&mut p.vector_overlay_gain, 0.1..=5.0).text("Arrow gain"));
        }
        ui.separator();
        ui.label("Overlay");
        egui::ComboBox::from_id_salt("overlay")
            .selected_text(
                p.material_overlay
                    .label_for(self.editor.document.model.draft.physics),
            )
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut p.material_overlay, MaterialOverlay::Off, "Off");
                ui.selectable_value(
                    &mut p.material_overlay,
                    MaterialOverlay::Regions,
                    "Subdomains",
                );
                for property in [
                    MaterialProperty::Density,
                    MaterialProperty::Stiffness,
                    MaterialProperty::Damping,
                    MaterialProperty::WaveSpeed,
                    MaterialProperty::Impedance,
                    MaterialProperty::Anisotropy,
                    MaterialProperty::VolumeSource,
                ] {
                    ui.selectable_value(
                        &mut p.material_overlay,
                        MaterialOverlay::Property(property),
                        property.label_for(self.editor.document.model.draft.physics),
                    );
                }
            });
        ui.add(
            egui::Slider::new(&mut p.material_overlay_opacity, 0.05..=1.0)
                .text("Overlay intensity"),
        );
        if matches!(p.material_overlay, MaterialOverlay::Property(_)) {
            ui.checkbox(&mut p.material_overlay_auto_range, "Automatic range");
            ui.checkbox(&mut p.material_overlay_logarithmic, "Logarithmic scale");
            if !p.material_overlay_auto_range {
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut p.material_overlay_manual_min).prefix("Min "));
                    ui.add(egui::DragValue::new(&mut p.material_overlay_manual_max).prefix("Max "));
                });
            }
            if let Some(error) = &overlay_error {
                ui.colored_label(RED, error);
            } else if let Some((done, total)) = overlay_progress {
                ui.label(format!("Sampling property… {done}/{total}"));
            } else if let Some(invalid) = overlay_invalid.filter(|count| *count > 0) {
                ui.colored_label(RED, format!("{invalid} invalid samples"));
            }
        }
        ui.separator();
        ui.label("Probes");
        ui.checkbox(&mut p.point_probes, "Points");
        ui.checkbox(&mut p.line_probes, "Curves");
        ui.checkbox(&mut p.area_probes, "Areas");
        ui.checkbox(&mut p.far_field_contour, "Far field");
    }
    fn simulation_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Simulation");
        let mut physics = self.editor.document.model.draft.physics;
        egui::ComboBox::from_id_salt("physics")
            .selected_text(match physics {
                PhysicsModel::Mechanical => "Mechanical",
                PhysicsModel::Electromagnetic {
                    polarization: ElectromagneticPolarization::Tm,
                } => "EM · TM",
                PhysicsModel::Electromagnetic {
                    polarization: ElectromagneticPolarization::Te,
                } => "EM · TE",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut physics, PhysicsModel::Mechanical, "Mechanical");
                ui.selectable_value(
                    &mut physics,
                    PhysicsModel::Electromagnetic {
                        polarization: ElectromagneticPolarization::Tm,
                    },
                    "EM · TM",
                );
                ui.selectable_value(
                    &mut physics,
                    PhysicsModel::Electromagnetic {
                        polarization: ElectromagneticPolarization::Te,
                    },
                    "EM · TE",
                );
            });
        if physics != self.editor.document.model.draft.physics {
            if let Err(error) = self.editor.set_physics(physics) {
                self.notify(error)
            }
        }
        ui.add(
            egui::Slider::new(&mut self.mesh_edge, 0.02..=0.25)
                .logarithmic(true)
                .text("Target edge"),
        );
        ui.separator();
        let before_amr = (
            self.amr_enabled,
            self.amr_minimum_edge,
            self.amr_maximum_edge,
        );
        ui.checkbox(&mut self.amr_enabled, "Adapt mesh to the wave");
        ui.add_enabled_ui(self.amr_enabled, |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut self.amr_minimum_edge)
                        .speed(0.002)
                        .range(0.005..=1.0)
                        .prefix("Min "),
                );
                ui.add(
                    egui::DragValue::new(&mut self.amr_maximum_edge)
                        .speed(0.005)
                        .range(0.005..=1.0)
                        .prefix("Max "),
                );
            });
            self.amr_minimum_edge = self.amr_minimum_edge.min(self.amr_maximum_edge).max(0.005);
            self.amr_maximum_edge = self.amr_maximum_edge.max(self.amr_minimum_edge);
            ui.small(&self.amr_status);
            if let Some(error) = &self.amr_error {
                ui.colored_label(RED, error);
            }
        });
        if before_amr
            != (
                self.amr_enabled,
                self.amr_minimum_edge,
                self.amr_maximum_edge,
            )
        {
            self.amr_indicator_job = None;
            self.amr_adaptation_job = None;
            self.amr_last_analyzed_step = None;
            self.amr_last_started = None;
            self.amr_coarsen_streak = 0;
            self.amr_error = None;
        }
        ui.separator();
        ui.label("Continuous source");
        let mut source = self.editor.document.model.source;
        let before = source;
        ui.checkbox(&mut source.enabled, "Enabled");
        let (_, amplitude, frequency, phase) = source.signal.harmonic_parameters_mut();
        ui.add(
            egui::DragValue::new(amplitude)
                .speed(0.05)
                .prefix("Amplitude "),
        );
        ui.add(
            egui::DragValue::new(frequency)
                .speed(0.1)
                .range(0.0..=1.0e5)
                .prefix("Frequency "),
        );
        ui.add(egui::DragValue::new(phase).speed(0.05).prefix("Phase "));
        ui.add(
            egui::DragValue::new(&mut source.width)
                .speed(0.002)
                .range(0.001..=1.0)
                .prefix("Width "),
        );
        if source != before {
            if let Err(error) = self.editor.set_point_source(source) {
                self.notify(error)
            }
        }
        ui.separator();
        if ui
            .selectable_label(self.pulse_mode, "Place pulse")
            .clicked()
        {
            self.pulse_mode = !self.pulse_mode;
            self.probe_mode = None;
        }
        ui.add(egui::Slider::new(&mut self.pulse_amplitude, -5.0..=5.0).text("Pulse amplitude"));
        ui.add(egui::Slider::new(&mut self.pulse_width, 0.01..=0.25).text("Pulse width"));
        if self.runtime.active().is_some() {
            ui.separator();
            ui.label(format!(
                "Energy: {}",
                self.wave_energy.map_or("—".into(), |v| format!("{v:.4e}"))
            ));
        }
    }
    fn materials_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Materials");
        ui.label("Subdomain assignment");
        let regions = self.editor.document.model.draft.regions.clone();
        let materials = self.editor.document.model.draft.materials.clone();
        for region in regions {
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(
                        self.region_selection == region.id,
                        if region.id == BACKGROUND_REGION {
                            "Background".into()
                        } else {
                            format!("Region {}", region.id.0)
                        },
                    )
                    .clicked()
                {
                    self.region_selection = region.id;
                }
                let mut material = region.material;
                egui::ComboBox::from_id_salt(("region", region.id.0))
                    .selected_text(
                        materials
                            .iter()
                            .find(|item| item.id == material)
                            .map_or("Missing", |item| item.name.as_str()),
                    )
                    .show_ui(ui, |ui| {
                        for item in &materials {
                            ui.selectable_value(&mut material, item.id, &item.name);
                        }
                    });
                if material != region.material {
                    if let Err(error) = self.editor.set_region_material(region.id, material) {
                        self.notify(error)
                    }
                }
            });
        }
        if let Some(region) = self
            .editor
            .document
            .model
            .draft
            .region(self.region_selection)
            .copied()
        {
            ui.separator();
            let existing = self
                .editor
                .document
                .model
                .draft
                .volume_sources
                .iter()
                .find(|source| source.region == region.id)
                .cloned();
            let mut source = existing.clone().unwrap_or(VolumeSource {
                region: region.id,
                enabled: false,
                profile: ScalarField::constant(1.0),
                parameters: vec![],
                signal: TimeSignal::harmonic(0.0, 12.0, 3.0, 0.0),
            });
            let mut source_changed = ui.checkbox(&mut source.enabled, "Volume source").changed();
            if existing.is_some() || source.enabled {
                ui.add_enabled_ui(source.enabled, |ui| {
                    let before = source.profile.clone();
                    material_scalar_editor(
                        ui,
                        (u64::MAX - region.id.0, 4),
                        "Profile",
                        &mut source.profile,
                        &source.parameters,
                        0.0,
                        &mut self.material_formula_edits,
                        &mut self.material_formula_errors,
                    );
                    source_changed |= source.profile != before;
                    ui.label("Signal");
                    let before = source.signal;
                    edit_time_signal(ui, &mut source.signal);
                    source_changed |= source.signal != before;
                });
                if source_changed
                    && let Err(error) = self.editor.set_volume_source(region.id, Some(source))
                {
                    self.notify(error);
                }
            }

            let uses_frame = self
                .editor
                .document
                .model
                .draft
                .material(region.material)
                .is_some_and(Material::uses_frame)
                || existing.as_ref().is_some_and(VolumeSource::varying);
            if uses_frame {
                ui.separator();
                ui.label("Profile placement");
                let mut frame = region.frame;
                let before = frame;
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut frame.origin.x)
                            .speed(0.01)
                            .prefix("x ")
                            .update_while_editing(false),
                    );
                    ui.add(
                        egui::DragValue::new(&mut frame.origin.y)
                            .speed(0.01)
                            .prefix("y ")
                            .update_while_editing(false),
                    );
                });
                let mut angle = frame.angle_radians.to_degrees();
                ui.add(
                    egui::DragValue::new(&mut angle)
                        .speed(0.5)
                        .suffix("°")
                        .prefix("Angle ")
                        .update_while_editing(false),
                );
                frame.angle_radians = angle.to_radians();
                if region.id != BACKGROUND_REGION {
                    egui::ComboBox::from_label("Attachment")
                        .selected_text(match frame.attachment {
                            MaterialFrameAttachment::World => "World",
                            MaterialFrameAttachment::FollowRegion => "Follow region",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut frame.attachment,
                                MaterialFrameAttachment::World,
                                "World",
                            );
                            ui.selectable_value(
                                &mut frame.attachment,
                                MaterialFrameAttachment::FollowRegion,
                                "Follow region",
                            );
                        });
                }
                if frame != before
                    && let Err(error) = self.editor.set_region_frame(region.id, frame)
                {
                    self.notify(error);
                }
            }
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Library");
            ui.small_button("?").on_hover_text(
                "Formulas use x, y, r, theta and material parameters. Functions: abs, sqrt, exp, ln, sin, cos, tan, min, max, clamp, pow.",
            );
            if ui.button("+").clicked() {
                match self.editor.add_material() {
                    Ok(id) => {
                        self.material_selection = id;
                        self.material_edit = None;
                        self.material_formula_edits.clear();
                        self.material_formula_errors.clear();
                    }
                    Err(error) => self.notify(error),
                }
            }
        });
        for material in &materials {
            if ui
                .selectable_label(self.material_selection == material.id, &material.name)
                .clicked()
            {
                self.material_selection = material.id;
                self.material_edit = None;
                self.material_formula_edits.clear();
                self.material_formula_errors.clear();
            }
        }
        if self
            .material_edit
            .as_ref()
            .is_none_or(|material| material.id != self.material_selection)
        {
            self.material_edit = self
                .editor
                .document
                .model
                .draft
                .materials
                .iter()
                .find(|item| item.id == self.material_selection)
                .cloned();
        }
        if let Some(mut material) = self.material_edit.take() {
            ui.separator();
            ui.text_edit_singleline(&mut material.name);
            material_scalar_editor(
                ui,
                (material.id.0, 0),
                "Density / ε",
                &mut material.mass_density,
                &material.parameters,
                0.000001,
                &mut self.material_formula_edits,
                &mut self.material_formula_errors,
            );
            material_scalar_editor(
                ui,
                (material.id.0, 1),
                "Stiffness / μ",
                &mut material.stiffness,
                &material.parameters,
                0.000001,
                &mut self.material_formula_edits,
                &mut self.material_formula_errors,
            );
            material_scalar_editor(
                ui,
                (material.id.0, 2),
                "Damping / α",
                &mut material.damping,
                &material.parameters,
                0.0,
                &mut self.material_formula_edits,
                &mut self.material_formula_errors,
            );
            material_scalar_editor(
                ui,
                (material.id.0, 3),
                "Axis ratio",
                &mut material.axis_ratio,
                &material.parameters,
                1.0,
                &mut self.material_formula_edits,
                &mut self.material_formula_errors,
            );
            ui.collapsing("Parameters", |ui| {
                let mut remove = None;
                for (index, parameter) in material.parameters.iter_mut().enumerate() {
                    let referenced = [
                        &material.mass_density,
                        &material.stiffness,
                        &material.damping,
                        &material.axis_ratio,
                    ]
                    .into_iter()
                    .flat_map(ScalarField::parameter_names)
                    .any(|name| name == parameter.name);
                    ui.horizontal(|ui| {
                        ui.label(&parameter.name);
                        ui.add(
                            egui::DragValue::new(&mut parameter.value)
                                .speed(0.01)
                                .update_while_editing(false),
                        );
                        if ui
                            .add_enabled(!referenced, egui::Button::new("−"))
                            .on_hover_text(if referenced {
                                "Parameter is used by a formula"
                            } else {
                                "Delete parameter"
                            })
                            .clicked()
                        {
                            remove = Some(index);
                        }
                    });
                }
                if let Some(index) = remove {
                    material.parameters.remove(index);
                }
                if material.parameters.len() < MAX_MATERIAL_PARAMETERS
                    && ui.button("+ Parameter").clicked()
                {
                    let name = (1..)
                        .map(|index| format!("p{index}"))
                        .find(|name| {
                            material
                                .parameters
                                .iter()
                                .all(|parameter| parameter.name != *name)
                        })
                        .unwrap();
                    material
                        .parameters
                        .push(MaterialParameter { name, value: 1.0 });
                }
            });
            let stored = self
                .editor
                .document
                .model
                .draft
                .material(material.id)
                .cloned();
            let dirty = stored.as_ref() != Some(&material);
            if ui.add_enabled(dirty, egui::Button::new("Apply")).clicked() {
                if let Err(error) = self.editor.update_material(material.clone()) {
                    self.notify(error)
                }
            }
            if self.material_selection != DEFAULT_MATERIAL
                && !self
                    .editor
                    .document
                    .model
                    .draft
                    .regions
                    .iter()
                    .any(|region| region.material == self.material_selection)
                && ui.button("Delete material").clicked()
            {
                match self.editor.delete_material(self.material_selection) {
                    Ok(()) => self.material_selection = DEFAULT_MATERIAL,
                    Err(error) => self.notify(error),
                }
            }
            self.material_edit = Some(material);
        }
    }
    fn selected_boundary_probe_target(&self) -> Option<TopologyBoundaryProbeTarget> {
        let selected = self.selection.spans()?;
        let span_ids = selected
            .iter()
            .filter_map(|target| match target {
                TopologySpanTarget::Curve(span) => Some(*span),
                TopologySpanTarget::Outer(_) => None,
            })
            .collect::<BTreeSet<_>>();
        if span_ids.is_empty() || span_ids.len() != selected.len() {
            return None;
        }
        let mut matching = self
            .editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .filter(|curve| curve.spans.iter().any(|span| span_ids.contains(&span.id)));
        let curve = matching.next()?;
        if matching.next().is_some() {
            return None;
        }
        let spans = curve
            .spans
            .iter()
            .filter(|span| span_ids.contains(&span.id))
            .map(|span| span.id)
            .collect::<Vec<_>>();
        (spans.len() == span_ids.len()).then_some(TopologyBoundaryProbeTarget {
            curve: curve.id,
            spans,
            side: self.selected_side,
            reversed: false,
            preset: ProbeSamplingPreset::Medium,
        })
    }
    fn probes_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Probes");
        ui.horizontal_wrapped(|ui| {
            for (mode, label) in [
                (ProbePlacement::Point, "Point"),
                (ProbePlacement::Segment { start: None }, "Line"),
                (ProbePlacement::Disk { center: None }, "Disk"),
                (ProbePlacement::Region, "Region"),
            ] {
                let selected = self.probe_mode.is_some_and(|active| {
                    std::mem::discriminant(&active) == std::mem::discriminant(&mode)
                });
                if ui
                    .selectable_label(selected, format!("+ {label}"))
                    .clicked()
                {
                    self.probe_mode = (!selected).then_some(mode);
                    self.pulse_mode = false;
                }
            }
        });
        let boundary_target = self.selected_boundary_probe_target();
        if ui
            .add_enabled(
                boundary_target.is_some(),
                egui::Button::new("+ Selected boundary"),
            )
            .clicked()
            && let Some(target) = boundary_target
        {
            match self.editor.create_probe(
                format!("Boundary {}", self.editor.document.model.probes.len() + 1),
                [248, 196, 112],
                TopologyProbeTarget::Boundary(target),
            ) {
                Ok(id) => {
                    self.probe_windows.insert(id);
                    self.notify("Boundary probe added");
                }
                Err(error) => self.notify(error),
            }
        }
        let probes = self.editor.document.model.probes.clone();
        for mut probe in probes {
            ui.horizontal(|ui| {
                let response =
                    ui.selectable_label(self.selected_probe == Some(probe.id), &probe.name);
                if response.clicked() {
                    self.selected_probe = Some(probe.id);
                    self.selection = TopologySelection::None;
                }
                if response.double_clicked() {
                    self.probe_windows.insert(probe.id);
                }
                if ui.checkbox(&mut probe.enabled, "Live").changed() {
                    if let Err(error) = self.editor.update_probe(probe.clone()) {
                        self.notify(error)
                    }
                }
                if ui.small_button("×").clicked() {
                    if let Err(error) = self.editor.delete_probe(probe.id) {
                        self.notify(error)
                    } else {
                        self.probe_windows.remove(&probe.id);
                        if self.selected_probe == Some(probe.id) {
                            self.selected_probe = None;
                        }
                    }
                }
            });
        }
        ui.separator();
        let mut far = self.editor.document.model.far_field;
        if ui.checkbox(&mut far.enabled, "Far field").changed() {
            if let Err(error) = self.editor.set_far_field(far) {
                self.notify(error)
            }
        }
        if ui
            .add_enabled(far.enabled, egui::Button::new("Open far-field readout"))
            .clicked()
        {
            self.far_field_window = true;
        }
        if ui
            .add(
                egui::DragValue::new(&mut far.inset)
                    .speed(0.005)
                    .range(0.001..=10.0)
                    .prefix("Inset "),
            )
            .changed()
        {
            if let Err(error) = self.editor.set_far_field(far) {
                self.notify(error)
            }
        }
    }
    fn viewport(&mut self, ui: &mut egui::Ui, display: &WaveDisplay) -> Rect {
        let available = ui.available_size();
        let (response, painter) = ui.allocate_painter(available, Sense::click_and_drag());
        let viewport = response.rect;
        if self.fit {
            self.fit_view(viewport);
        }
        self.refresh_samples(viewport);
        let transform = self.transform(viewport);
        if self.editor.document.presentation.grid {
            self.draw_grid(&painter, viewport);
        }
        self.draw_solution(&painter, viewport, display);
        if matches!(self.editor.acceptance, TopologyAcceptance::Invalid(_))
            && self.editor.document.presentation.accepted_reference
        {
            if let Ok(reference) = SampledTopologyGeometry::new(
                &self.editor.document.model.accepted.geometry,
                transform,
                0.8,
            ) {
                self.draw_sampled(
                    &painter,
                    viewport,
                    &reference,
                    Color32::from_gray(65),
                    1.0,
                    false,
                );
            }
        }
        if let Some(sampled) = &self.sampled {
            let color = if matches!(self.editor.acceptance, TopologyAcceptance::Invalid(_)) {
                RED
            } else {
                TEAL
            };
            self.draw_sampled(&painter, viewport, sampled, color, 2.0, true);
        }
        self.draw_markers(&painter, viewport);
        self.draw_transform_gizmo(&painter, viewport);
        if let Some(logo) = &self.logo_texture {
            let size = egui::vec2(140.0, 57.0);
            let logo_rect = Rect::from_min_size(
                Pos2::new(viewport.right() - size.x - 14.0, viewport.top() + 12.0),
                size,
            );
            painter.image(
                logo.id(),
                logo_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if let MaterialOverlay::Property(property) =
            self.editor.document.presentation.material_overlay
            && let Some(pointer) = response.hover_pos()
            && let Some(sample) = self
                .material_overlay_snapshot
                .as_ref()
                .and_then(|snapshot| {
                    snapshot.samples.iter().min_by(|left, right| {
                        self.screen(left.point, viewport)
                            .distance(pointer)
                            .total_cmp(&self.screen(right.point, viewport).distance(pointer))
                    })
                })
            && self.screen(sample.point, viewport).distance(pointer) <= 14.0
        {
            let value = sample
                .value(property)
                .map_or_else(|error| error.into(), |value| format!("{value:.5}"));
            response.clone().on_hover_text(format!(
                "{} · region {} · {value}\nlocal x {:.3}, y {:.3}, r {:.3}, θ {:.1}°",
                sample.material_name,
                sample.region.0,
                sample.coordinates.x,
                sample.coordinates.y,
                sample.coordinates.r,
                sample.coordinates.theta.to_degrees(),
            ));
        }
        self.handle_viewport_input(ui, &response, viewport);
        viewport
    }
    fn draw_grid(&self, painter: &egui::Painter, r: Rect) {
        let world_min = self.world(r.left_bottom(), r);
        let world_max = self.world(r.right_top(), r);
        let raw = 70.0 / self.scale;
        let power = 10f64.powf(raw.log10().floor());
        let step = [1.0, 2.0, 5.0, 10.0]
            .into_iter()
            .map(|v| v * power)
            .find(|v| *v >= raw)
            .unwrap();
        let mut x = (world_min.x / step).floor() * step;
        while x <= world_max.x {
            let sx = self.screen(Point2::new(x, 0.0), r).x;
            painter.line_segment(
                [Pos2::new(sx, r.top()), Pos2::new(sx, r.bottom())],
                Stroke::new(
                    if x.abs() < step * 0.1 { 1.0 } else { 0.5 },
                    Color32::from_gray(42),
                ),
            );
            x += step;
        }
        let mut y = (world_max.y / step).floor() * step;
        while y <= world_min.y {
            let sy = self.screen(Point2::new(0.0, y), r).y;
            painter.line_segment(
                [Pos2::new(r.left(), sy), Pos2::new(r.right(), sy)],
                Stroke::new(
                    if y.abs() < step * 0.1 { 1.0 } else { 0.5 },
                    Color32::from_gray(42),
                ),
            );
            y += step;
        }
    }
    fn draw_solution(&mut self, painter: &egui::Painter, r: Rect, display: &WaveDisplay) {
        let Some(active) = self.runtime.active().cloned() else {
            return;
        };
        let mesh = &active.mesh;
        let presentation = self.editor.document.presentation;
        if presentation.adaptation_target
            && let Some(result) = &self.amr_indicator_result
            && result.element_targets.len() == mesh.triangles.len()
        {
            let span = (self.amr_maximum_edge - self.amr_minimum_edge).max(f64::MIN_POSITIVE);
            for (triangle, target) in mesh.triangles.iter().zip(&result.element_targets) {
                let fraction = ((*target - self.amr_minimum_edge) / span).clamp(0.0, 1.0) as f32;
                let color = amr_target_color(fraction);
                let points = triangle
                    .vertices
                    .map(|index| self.screen(mesh.vertices[index].point, r));
                painter.add(egui::Shape::convex_polygon(
                    points.to_vec(),
                    color,
                    Stroke::NONE,
                ));
            }
        }
        if let MaterialOverlay::Property(property) = presentation.material_overlay
            && let Some(snapshot) = &self.material_overlay_snapshot
            && let Some(range) = self.material_overlay_range(property)
        {
            let alpha = (presentation.material_overlay_opacity * 255.0).round() as u8;
            let mut values = egui::Mesh::default();
            values.reserve_vertices(snapshot.samples.len());
            values.reserve_triangles(snapshot.triangles.len());
            for sample in &snapshot.samples {
                let color = sample
                    .value(property)
                    .ok()
                    .and_then(|value| {
                        range
                            .normalized(value, presentation.material_overlay_logarithmic)
                            .map(|fraction| overlay_property_color(property, fraction, alpha))
                    })
                    .unwrap_or(Color32::from_rgba_unmultiplied(255, 73, 91, alpha.max(150)));
                values.colored_vertex(self.screen(sample.point, r), color);
            }
            for triangle in &snapshot.triangles {
                values.add_triangle(triangle[0], triangle[1], triangle[2]);
            }
            painter.add(egui::Shape::mesh(values));
            if property == MaterialProperty::Anisotropy {
                let stride = (snapshot.samples.len() / 90).max(1);
                for sample in snapshot.samples.iter().step_by(stride) {
                    let Ok(ratio) = sample.value(property) else {
                        continue;
                    };
                    if ratio <= 1.0 + 1.0e-6 {
                        continue;
                    }
                    let angle = snapshot
                        .key
                        .scene
                        .region(sample.region)
                        .map_or(0.0, |region| region.frame.angle_radians);
                    let center = self.screen(sample.point, r);
                    let direction = egui::vec2(angle.cos() as f32, -angle.sin() as f32) * 7.0;
                    painter.line_segment(
                        [center - direction, center + direction],
                        Stroke::new(1.2, Color32::from_rgba_unmultiplied(232, 247, 242, 190)),
                    );
                }
            }
        }
        for triangle in &mesh.triangles {
            let points = triangle
                .vertices
                .map(|index| self.screen(mesh.vertices[index].point, r));
            let region_color = active
                .bundle
                .authored
                .region(triangle.region)
                .and_then(|region| active.bundle.authored.material(region.material))
                .map(|m| {
                    Color32::from_rgba_unmultiplied(
                        m.color[0],
                        m.color[1],
                        m.color[2],
                        (presentation.material_overlay_opacity * 210.0) as u8,
                    )
                })
                .unwrap_or(Color32::TRANSPARENT);
            let base = match presentation.material_overlay {
                MaterialOverlay::Off => Color32::TRANSPARENT,
                MaterialOverlay::Regions => region_color,
                MaterialOverlay::Property(_) => Color32::TRANSPARENT,
            };
            if base != Color32::TRANSPARENT {
                painter.add(egui::Shape::convex_polygon(
                    points.to_vec(),
                    base,
                    Stroke::NONE,
                ));
            }
            if presentation.mesh {
                painter.add(egui::Shape::closed_line(
                    points.to_vec(),
                    Stroke::new(0.5, Color32::from_gray(75)),
                ));
            }
        }
        if presentation.mesh_boundaries {
            for edge in &mesh.boundary_edges {
                let [a, b] = edge.vertices.map(|index| mesh.vertices[index].point);
                painter.line_segment(
                    [self.screen(a, r), self.screen(b, r)],
                    Stroke::new(1.15, Color32::from_rgba_unmultiplied(184, 201, 211, 180)),
                );
            }
        }
        if presentation.field
            && display.generation > 0
            && display.current.len() == active.operator.degrees_of_freedom()
        {
            let mut field = egui::Mesh::default();
            field.reserve_vertices(active.operator.degrees_of_freedom());
            field.reserve_triangles(active.operator.element_nodes().len() * 6);
            for (point, value) in active.operator.node_points().iter().zip(&display.current) {
                let color = if presentation.material_overlay == MaterialOverlay::Off {
                    field_color(*value, presentation.field_gain, Color32::TRANSPARENT)
                } else {
                    field_color_over_overlay(*value, presentation.field_gain)
                };
                field.colored_vertex(self.screen(*point, r), color);
            }
            for nodes in active.operator.element_nodes() {
                for [a, b] in [[0, 3], [3, 1], [1, 4], [4, 2], [2, 5], [5, 0]] {
                    field.add_triangle(nodes[a], nodes[b], nodes[6]);
                }
            }
            painter.add(egui::Shape::mesh(field));
        }
        let mode = presentation.vector_overlay;
        if mode != VectorOverlay::Off {
            let samples = vector_overlay_samples(
                &active.bundle.authored,
                mesh,
                &active.operator,
                display,
                mode,
                presentation.vector_overlay_density,
                (self.center, self.scale, r),
            );
            self.draw_vector_overlay(painter, samples, display.completed_steps);
        } else {
            self.vector_overlay_average.clear();
            self.vector_overlay_step = u64::MAX;
        }
    }

    fn draw_vector_overlay(
        &mut self,
        painter: &egui::Painter,
        mut samples: Vec<((i32, i32), Pos2, Point2)>,
        completed_steps: u64,
    ) {
        let settings = self.editor.document.presentation;
        if self.vector_overlay_mode != settings.vector_overlay {
            self.vector_overlay_average.clear();
            self.vector_overlay_step = u64::MAX;
            self.vector_overlay_peak_reference = 0.0;
            self.vector_overlay_mode = settings.vector_overlay;
        }
        if settings.vector_overlay_smoothed {
            if self.vector_overlay_step != completed_steps {
                let mut next = BTreeMap::new();
                for (key, _, value) in &samples {
                    let filtered = self
                        .vector_overlay_average
                        .get(key)
                        .map_or(*value, |previous| *previous * 0.82 + *value * 0.18);
                    next.insert(*key, filtered);
                }
                self.vector_overlay_average = next;
                self.vector_overlay_step = completed_steps;
            }
            for (key, _, value) in &mut samples {
                if let Some(filtered) = self.vector_overlay_average.get(key) {
                    *value = *filtered;
                }
            }
        } else {
            self.vector_overlay_average.clear();
            self.vector_overlay_step = completed_steps;
        }
        let mut magnitudes = samples
            .iter()
            .map(|(_, _, value)| value.norm())
            .filter(|magnitude| magnitude.is_finite() && *magnitude > 0.0)
            .collect::<Vec<_>>();
        if magnitudes.is_empty() {
            return;
        }
        magnitudes.sort_by(f64::total_cmp);
        let instantaneous = magnitudes[(magnitudes.len() - 1) * 9 / 10];
        if !instantaneous.is_finite() || instantaneous < 1.0e-6 {
            return;
        }
        self.vector_overlay_peak_reference = self.vector_overlay_peak_reference.max(instantaneous);
        if instantaneous < self.vector_overlay_peak_reference * 1.0e-4 {
            return;
        }
        let reference = self.vector_overlay_peak_reference;
        let maximum_length = settings.vector_overlay_density * 0.46;
        let scale = maximum_length as f64 * settings.vector_overlay_gain as f64 / reference;
        for (_, origin, value) in samples {
            let magnitude = value.norm();
            if magnitude < reference * 0.015 || !magnitude.is_finite() {
                continue;
            }
            let length = (magnitude * scale).min(maximum_length as f64) as f32;
            let direction = egui::vec2(value.x as f32, -value.y as f32).normalized();
            let tip = origin + direction * length;
            let normal = egui::vec2(-direction.y, direction.x);
            let head = 5.0_f32.min(length * 0.35);
            let stroke = Stroke::new(1.45, Color32::from_rgba_unmultiplied(116, 232, 210, 220));
            painter.line_segment([origin, tip], stroke);
            painter.line_segment([tip, tip - direction * head + normal * head * 0.55], stroke);
            painter.line_segment([tip, tip - direction * head - normal * head * 0.55], stroke);
        }
    }
    fn draw_sampled(
        &self,
        painter: &egui::Painter,
        r: Rect,
        sampled: &SampledTopologyGeometry,
        color: Color32,
        width: f32,
        interactive: bool,
    ) {
        if interactive && self.editor.document.presentation.control_polygons {
            for curve in &self.editor.document.model.draft.geometry.curves {
                let (controls, closed) = match &curve.spline {
                    CurveSpline::Closed(spline) => (spline.controls(), true),
                    CurveSpline::Open(spline) => (spline.controls(), false),
                };
                for pair in controls.windows(2) {
                    painter.line_segment(
                        [self.screen(pair[0], r), self.screen(pair[1], r)],
                        Stroke::new(0.8, Color32::from_rgba_unmultiplied(156, 171, 180, 100)),
                    );
                }
                if closed && controls.len() > 2 {
                    painter.line_segment(
                        [
                            self.screen(*controls.last().unwrap(), r),
                            self.screen(controls[0], r),
                        ],
                        Stroke::new(0.8, Color32::from_rgba_unmultiplied(156, 171, 180, 100)),
                    );
                }
            }
        }
        for span in &sampled.spans {
            let selected = matches!(&self.selection,TopologySelection::Spans(spans) if spans.contains(&span.target));
            let stroke = Stroke::new(
                if selected { width + 2.0 } else { width },
                if selected { SELECT } else { color },
            );
            for segment in span.samples.windows(2) {
                painter.line_segment(
                    [
                        self.screen(segment[0].point, r),
                        self.screen(segment[1].point, r),
                    ],
                    stroke,
                );
            }
            if interactive
                && self.editor.document.presentation.boundary_conditions
                && let Some((left, right)) = self.span_condition_colors(span.target)
            {
                for segment in span.samples.windows(2) {
                    let a = self.screen(segment[0].point, r);
                    let b = self.screen(segment[1].point, r);
                    let tangent = b - a;
                    if tangent.length_sq() <= f32::EPSILON {
                        continue;
                    }
                    let normal = egui::vec2(-tangent.y, tangent.x).normalized() * 2.5;
                    painter.line_segment([a + normal, b + normal], Stroke::new(1.4, left));
                    painter.line_segment([a - normal, b - normal], Stroke::new(1.4, right));
                }
            }
        }
        if interactive && self.editor.document.presentation.handles {
            for handle in &sampled.handles {
                let selected = matches!(self.selection,TopologySelection::Handle(value) if value==handle.handle);
                let p = self.screen(handle.point, r);
                let (radius, fill) = match handle.handle {
                    TopologyHandle::Junction(_) => (5.0, GOLD),
                    _ => (
                        3.5,
                        if selected {
                            SELECT
                        } else {
                            Color32::from_gray(205)
                        },
                    ),
                };
                painter.circle_filled(p, radius, fill);
            }
        }
    }
    fn span_condition_colors(&self, target: TopologySpanTarget) -> Option<(Color32, Color32)> {
        match target {
            TopologySpanTarget::Outer(side) => {
                let color = outer_condition_color(
                    self.editor.document.model.draft.outer_boundaries.sides[side.index()],
                );
                Some((color, color))
            }
            TopologySpanTarget::Curve(id) => self
                .editor
                .document
                .model
                .draft
                .geometry
                .curves
                .iter()
                .flat_map(|curve| &curve.spans)
                .find(|span| span.id == id)
                .map(|span| match span.behavior {
                    SpanBehavior::Transmitting => (TEAL, TEAL),
                    SpanBehavior::Separated { left, right, .. } => {
                        (face_condition_color(left), face_condition_color(right))
                    }
                }),
        }
    }
    fn draw_markers(&self, painter: &egui::Painter, r: Rect) {
        let p = self.editor.document.presentation;
        let source = self.editor.document.model.source;
        if source.enabled {
            let s = self.screen(source.position, r);
            painter.circle(
                s,
                6.0,
                Color32::from_rgb(246, 154, 70),
                Stroke::new(1.5, Color32::WHITE),
            );
        }
        for probe in &self.editor.document.model.probes {
            let point = match probe.target {
                TopologyProbeTarget::Point(point) => Some(point),
                TopologyProbeTarget::AreaDisk { center, radius } => {
                    if p.area_probes {
                        painter.circle_stroke(
                            self.screen(center, r),
                            (radius * self.scale) as f32,
                            Stroke::new(
                                1.2,
                                Color32::from_rgb(probe.color[0], probe.color[1], probe.color[2]),
                            ),
                        );
                    }
                    Some(center)
                }
                TopologyProbeTarget::Segment { start, end, .. } => {
                    if p.line_probes {
                        painter.line_segment(
                            [self.screen(start, r), self.screen(end, r)],
                            Stroke::new(
                                2.0,
                                Color32::from_rgb(probe.color[0], probe.color[1], probe.color[2]),
                            ),
                        );
                    }
                    None
                }
                TopologyProbeTarget::Boundary(ref target) => {
                    if p.boundary_probes
                        && let Some(sampled) = &self.sampled
                    {
                        for span in sampled.spans.iter().filter(|span| {
                            matches!(span.target, TopologySpanTarget::Curve(id) if target.spans.contains(&id))
                        }) {
                            for segment in span.samples.windows(2) {
                                painter.line_segment(
                                    [
                                        self.screen(segment[0].point, r),
                                        self.screen(segment[1].point, r),
                                    ],
                                    Stroke::new(
                                        3.5,
                                        Color32::from_rgb(
                                            probe.color[0],
                                            probe.color[1],
                                            probe.color[2],
                                        ),
                                    ),
                                );
                            }
                        }
                    }
                    None
                }
                TopologyProbeTarget::AreaRegion(_) => None,
            };
            if let Some(point) = point {
                let visible = match probe.target {
                    TopologyProbeTarget::Point(_) => p.point_probes,
                    TopologyProbeTarget::AreaDisk { .. } | TopologyProbeTarget::AreaRegion(_) => {
                        p.area_probes
                    }
                    _ => true,
                };
                if visible {
                    let pos = self.screen(point, r);
                    if self.selected_probe == Some(probe.id) {
                        painter.circle_stroke(pos, 8.0, Stroke::new(1.5, SELECT));
                    }
                    painter.circle_filled(
                        pos,
                        4.5,
                        Color32::from_rgb(probe.color[0], probe.color[1], probe.color[2]),
                    );
                }
            }
        }
        if p.far_field_contour && self.editor.document.model.far_field.enabled {
            let d = self.editor.document.model.draft.geometry.domain;
            let inset = self.editor.document.model.far_field.inset;
            let min = self.screen(Point2::new(d.min_x + inset, d.max_y - inset), r);
            let max = self.screen(Point2::new(d.max_x - inset, d.min_y + inset), r);
            painter.rect_stroke(
                Rect::from_min_max(min, max),
                0.0,
                Stroke::new(1.0, GOLD),
                egui::StrokeKind::Middle,
            );
            painter.text(
                Pos2::new(min.x + 5.0, min.y + 4.0),
                egui::Align2::LEFT_TOP,
                "FF",
                egui::FontId::proportional(12.0),
                GOLD,
            );
        }
        if let Some(DragGesture::Marquee { start, .. }) = &self.drag {
            if let Some(current) = painter.ctx().pointer_interact_pos() {
                painter.rect_stroke(
                    Rect::from_two_pos(*start, current),
                    0.0,
                    Stroke::new(1.0, SELECT),
                    egui::StrokeKind::Middle,
                );
            }
        }
        if let Some(draw) = &self.draw {
            let points = draw
                .points
                .iter()
                .map(|p| self.screen(*p, r))
                .collect::<Vec<_>>();
            for p in &points {
                painter.circle_filled(*p, 4.0, GOLD);
            }
            if points.len() > 1 {
                painter.add(egui::Shape::line(points, Stroke::new(1.5, GOLD)));
            }
        }
        if let Some(pointer) = painter.ctx().pointer_hover_pos() {
            let current = self.world(pointer, r);
            match self.probe_mode {
                Some(ProbePlacement::Segment { start: Some(start) }) => {
                    painter.line_segment([self.screen(start, r), pointer], Stroke::new(1.5, TEAL));
                }
                Some(ProbePlacement::Disk {
                    center: Some(center),
                }) => {
                    painter.circle_stroke(
                        self.screen(center, r),
                        ((current - center).norm() * self.scale) as f32,
                        Stroke::new(1.5, TEAL),
                    );
                }
                _ => {}
            }
        }
    }
    fn transform_gizmo(&self, r: Rect) -> Option<(Point2, Pos2)> {
        let selected = self.selection.spans()?;
        if selected.is_empty()
            || selected
                .iter()
                .any(|target| matches!(target, TopologySpanTarget::Outer(_)))
        {
            return None;
        }
        let spans = selected
            .iter()
            .filter_map(|target| match target {
                TopologySpanTarget::Curve(span) => Some(*span),
                TopologySpanTarget::Outer(_) => None,
            })
            .collect::<BTreeSet<_>>();
        let pivot = self.selection_pivot(&spans)?;
        Some((pivot, self.screen(pivot, r)))
    }
    fn draw_transform_gizmo(&self, painter: &egui::Painter, r: Rect) {
        let Some((_, center)) = self.transform_gizmo(r) else {
            return;
        };
        painter.circle_stroke(
            center,
            34.0,
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(72, 166, 255, 150)),
        );
        painter.circle_filled(center, 3.0, SELECT);
        let grip = center + egui::vec2(24.0, -24.0);
        painter.rect_filled(
            Rect::from_center_size(grip, egui::vec2(8.0, 8.0)),
            1.0,
            SELECT,
        );
    }
    fn hit_transform_gizmo(&self, point: Pos2, r: Rect) -> Option<(TransformGizmoHit, Point2)> {
        let (pivot, center) = self.transform_gizmo(r)?;
        let grip = center + egui::vec2(24.0, -24.0);
        if grip.distance(point) <= 10.0 {
            return Some((TransformGizmoHit::Scale, pivot));
        }
        ((center.distance(point) - 34.0).abs() <= 7.0).then_some((TransformGizmoHit::Rotate, pivot))
    }
    fn handle_viewport_input(&mut self, ui: &egui::Ui, response: &egui::Response, r: Rect) {
        let pointer = response.interact_pointer_pos();
        let touch_active = ui.input(|input| input.any_touches());
        let multi_touch = ui.input(|input| input.multi_touch());
        if let Some(gesture) = multi_touch
            && (self.touch_navigation || r.contains(gesture.center_pos))
        {
            if !self.touch_navigation {
                self.cancel_interaction();
                self.touch_navigation = true;
                self.suppress_touch_click = true;
            }
            self.center = self.center
                + Point2::new(
                    -gesture.translation_delta.x as f64 / self.scale,
                    gesture.translation_delta.y as f64 / self.scale,
                );
            let before = self.world(gesture.center_pos, r);
            self.scale = (self.scale * gesture.zoom_delta as f64).clamp(20.0, 5000.0);
            let after = self.world(gesture.center_pos, r);
            self.center = self.center + before - after;
            self.invalidate_samples();
            return;
        }
        if self.touch_navigation {
            if !touch_active {
                self.touch_navigation = false;
            }
            return;
        }
        if self.suppress_touch_click {
            if !touch_active {
                self.suppress_touch_click = false;
            }
            return;
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                    let before = self.world(pos, r);
                    self.scale = (self.scale * (scroll as f64 * 0.0015).exp()).clamp(20.0, 5000.0);
                    let after = self.world(pos, r);
                    self.center = self.center + (before - after);
                    self.invalidate_samples();
                }
            }
        }
        if response.dragged_by(egui::PointerButton::Secondary) {
            let delta = response.drag_delta();
            self.center = self.center - Point2::new(delta.x as f64, -delta.y as f64) / self.scale;
            self.invalidate_samples();
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.cancel_interaction();
            return;
        }
        if response.double_clicked()
            && let Some(pos) = pointer
        {
            if let Some(probe) = self
                .editor
                .document
                .model
                .probes
                .iter()
                .find(|probe| match probe.target {
                    TopologyProbeTarget::Point(point)
                    | TopologyProbeTarget::AreaDisk { center: point, .. } => {
                        self.screen(point, r).distance(pos) <= 12.0
                    }
                    TopologyProbeTarget::Segment { start, end, .. } => {
                        screen_segment_distance(pos, self.screen(start, r), self.screen(end, r))
                            <= 8.0
                    }
                    _ => false,
                })
            {
                self.probe_windows.insert(probe.id);
                return;
            }
            if self.editor.document.model.far_field.enabled {
                let domain = self.editor.document.model.draft.geometry.domain;
                let inset = self.editor.document.model.far_field.inset;
                let min = self.screen(Point2::new(domain.min_x + inset, domain.max_y - inset), r);
                let max = self.screen(Point2::new(domain.max_x - inset, domain.min_y + inset), r);
                let contour = Rect::from_min_max(min, max);
                let distance = [
                    (pos.x - contour.left()).abs(),
                    (pos.x - contour.right()).abs(),
                    (pos.y - contour.top()).abs(),
                    (pos.y - contour.bottom()).abs(),
                ]
                .into_iter()
                .fold(f32::INFINITY, f32::min);
                if contour.expand(9.0).contains(pos) && distance <= 9.0 {
                    self.far_field_window = true;
                    return;
                }
            }
            if let Some((curve, parameter)) = self
                .sampled
                .as_ref()
                .and_then(|sampled| closest_curve_parameter(sampled, self.transform(r), pos, 8.0))
            {
                match self.editor.insert_control(curve, parameter) {
                    Ok((control, _)) => {
                        self.selection =
                            TopologySelection::Handle(TopologyHandle::Control { curve, control });
                        self.invalidate_samples();
                        self.notify("Control inserted without changing the curve");
                    }
                    Err(error) => self.notify(error),
                }
                return;
            }
        }
        if let Some(draw) = self.draw.as_ref().map(|d| d.tool) {
            if response.clicked_by(egui::PointerButton::Primary) {
                if let Some(pos) = pointer {
                    self.draw_click(
                        self.world(pos, r),
                        ScreenPoint::new(pos.x as f64, pos.y as f64),
                        r,
                    );
                }
            }
            if ui.input(|i| i.key_pressed(egui::Key::Backspace)) {
                if let Some(draw) = &mut self.draw {
                    draw.points.pop();
                    draw.attachments.pop();
                }
            }
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.finish_draw();
            }
            let _ = draw;
            return;
        }
        if self.pulse_mode && response.clicked() {
            if let Some(pos) = pointer {
                self.place_pulse(self.world(pos, r));
            }
            return;
        }
        if let Some(mode) = self.probe_mode
            && response.clicked()
        {
            if let Some(pos) = pointer {
                let point = self.world(pos, r);
                let target = match mode {
                    ProbePlacement::Point => Some(TopologyProbeTarget::Point(point)),
                    ProbePlacement::Segment { start: None } => {
                        self.probe_mode = Some(ProbePlacement::Segment { start: Some(point) });
                        self.notify("Choose the line end");
                        None
                    }
                    ProbePlacement::Segment { start: Some(start) } => {
                        self.probe_mode = Some(ProbePlacement::Segment { start: None });
                        Some(TopologyProbeTarget::Segment {
                            start,
                            end: point,
                            preset: ProbeSamplingPreset::Medium,
                        })
                    }
                    ProbePlacement::Disk { center: None } => {
                        self.probe_mode = Some(ProbePlacement::Disk {
                            center: Some(point),
                        });
                        self.notify("Choose the disk radius");
                        None
                    }
                    ProbePlacement::Disk {
                        center: Some(center),
                    } => {
                        self.probe_mode = Some(ProbePlacement::Disk { center: None });
                        Some(TopologyProbeTarget::AreaDisk {
                            center,
                            radius: (point - center).norm(),
                        })
                    }
                    ProbePlacement::Region => self.runtime.active().and_then(|active| {
                        let face = active.bundle.snapshot.face_at(point)?;
                        active
                            .bundle
                            .plan
                            .domains
                            .iter()
                            .find(|domain| domain.face == face)
                            .map(|domain| TopologyProbeTarget::AreaRegion(domain.region))
                    }),
                };
                if let Some(target) = target {
                    match self.editor.create_probe(
                        format!("Probe {}", self.editor.document.model.probes.len() + 1),
                        [91, 220, 194],
                        target,
                    ) {
                        Ok(id) => {
                            self.probe_windows.insert(id);
                            self.notify("Probe added");
                        }
                        Err(error) => self.notify(error),
                    }
                }
            }
            return;
        }
        if response.drag_started_by(egui::PointerButton::Primary) {
            if let Some(pos) = pointer {
                if let Some((hit, pivot)) = self.hit_transform_gizmo(pos, r) {
                    let relative = self.world(pos, r) - pivot;
                    self.editor.begin();
                    self.drag = Some(match hit {
                        TransformGizmoHit::Rotate => DragGesture::Rotate {
                            pivot,
                            start_angle: relative.y.atan2(relative.x),
                            geometry: self.editor.document.model.draft.geometry.clone(),
                        },
                        TransformGizmoHit::Scale => DragGesture::Scale {
                            pivot,
                            start_distance: relative.norm().max(1.0e-12),
                            geometry: self.editor.document.model.draft.geometry.clone(),
                        },
                    });
                    return;
                }
                let screen = ScreenPoint::new(pos.x as f64, pos.y as f64);
                let source = self.editor.document.model.source;
                if source.enabled && self.screen(source.position, r).distance(pos) <= 11.0 {
                    self.editor.begin();
                    self.drag = Some(DragGesture::Source);
                    return;
                }
                if let Some(probe) = self
                    .editor
                    .document
                    .model
                    .probes
                    .iter()
                    .find(|probe| match probe.target {
                        TopologyProbeTarget::Point(point)
                        | TopologyProbeTarget::AreaDisk { center: point, .. } => {
                            self.screen(point, r).distance(pos) <= 11.0
                        }
                        _ => false,
                    })
                    .map(|probe| probe.id)
                {
                    self.selected_probe = Some(probe);
                    self.selection = TopologySelection::None;
                    self.editor.begin();
                    self.drag = Some(DragGesture::Probe(probe));
                    return;
                }
                let hit = self
                    .sampled
                    .as_ref()
                    .and_then(|sampled| sampled.hit_test(self.transform(r), screen, 8.0, 7.0));
                if let Some(hit) = hit {
                    self.selected_probe = None;
                    self.selection.apply_hit(
                        &self.editor.document.model.draft.geometry,
                        hit,
                        ui.input(|i| i.modifiers.shift),
                        ui.input(|i| i.modifiers.command),
                    );
                    self.editor.begin();
                    self.drag = Some(match hit {
                        TopologyHit::Handle { handle, .. } => DragGesture::Handle { handle },
                        TopologyHit::Span { .. } => DragGesture::Spans {
                            start: self.world(pos, r),
                            geometry: self.editor.document.model.draft.geometry.clone(),
                        },
                    });
                } else {
                    self.drag = Some(DragGesture::Marquee {
                        start: pos,
                        base: self.selection.clone(),
                    });
                }
            }
        }
        if response.dragged_by(egui::PointerButton::Primary) {
            if let (Some(pos), Some(drag)) = (pointer, self.drag.clone()) {
                let point = self.world(pos, r);
                let result = match drag {
                    DragGesture::Handle { handle } => {
                        plan_handle_drag(&self.editor.document.model.draft.geometry, handle, point)
                            .map_err(|e| e.to_string())
                            .and_then(|update| {
                                self.editor.apply_transform_updates_during_edit(&[update])
                            })
                    }
                    DragGesture::Spans { start, geometry } => {
                        let selected = self.selection.spans().cloned().unwrap_or_default();
                        plan_rigid_transform(
                            &geometry,
                            &selected,
                            RigidTransform {
                                pivot: Point2::default(),
                                translation: point - start,
                                rotation_radians: 0.0,
                                scale: 1.0,
                            },
                        )
                        .map_err(|e| e.to_string())
                        .and_then(|updates| {
                            self.editor.document.model.draft.geometry = geometry;
                            self.editor.apply_transform_updates_during_edit(&updates)
                        })
                    }
                    DragGesture::Rotate {
                        pivot,
                        start_angle,
                        geometry,
                    } => {
                        let selected = self.selection.spans().cloned().unwrap_or_default();
                        let relative = point - pivot;
                        plan_rigid_transform(
                            &geometry,
                            &selected,
                            RigidTransform {
                                pivot,
                                translation: Point2::default(),
                                rotation_radians: relative.y.atan2(relative.x) - start_angle,
                                scale: 1.0,
                            },
                        )
                        .map_err(|e| e.to_string())
                        .and_then(|updates| {
                            self.editor.document.model.draft.geometry = geometry;
                            self.editor.apply_transform_updates_during_edit(&updates)
                        })
                    }
                    DragGesture::Scale {
                        pivot,
                        start_distance,
                        geometry,
                    } => {
                        let selected = self.selection.spans().cloned().unwrap_or_default();
                        plan_rigid_transform(
                            &geometry,
                            &selected,
                            RigidTransform {
                                pivot,
                                translation: Point2::default(),
                                rotation_radians: 0.0,
                                scale: ((point - pivot).norm() / start_distance).max(0.01),
                            },
                        )
                        .map_err(|e| e.to_string())
                        .and_then(|updates| {
                            self.editor.document.model.draft.geometry = geometry;
                            self.editor.apply_transform_updates_during_edit(&updates)
                        })
                    }
                    DragGesture::Marquee { .. } => Ok(()),
                    DragGesture::Source => {
                        let mut source = self.editor.document.model.source;
                        source.position = point;
                        if let Some(active) = self.runtime.active()
                            && let Some(face) = active.bundle.snapshot.face_at(point)
                            && let Some(region) = active
                                .bundle
                                .plan
                                .domains
                                .iter()
                                .find(|domain| domain.face == face)
                        {
                            source.region = region.region;
                        }
                        self.editor.set_point_source_during_edit(source)
                    }
                    DragGesture::Probe(id) => {
                        let probe = self
                            .editor
                            .document
                            .model
                            .probes
                            .iter()
                            .find(|probe| probe.id == id)
                            .cloned();
                        if let Some(mut probe) = probe {
                            match &mut probe.target {
                                TopologyProbeTarget::Point(position) => *position = point,
                                TopologyProbeTarget::AreaDisk { center, .. } => *center = point,
                                _ => return,
                            }
                            self.editor.update_probe_during_edit(probe)
                        } else {
                            Err("Probe no longer exists".into())
                        }
                    }
                };
                if let Err(error) = result {
                    self.message = error;
                }
                self.invalidate_samples();
            }
        }
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            if let Some(drag) = self.drag.take() {
                match drag {
                    DragGesture::Marquee { start, base } => {
                        if let Some(end) = pointer {
                            let hits = self
                                .sampled
                                .as_ref()
                                .map(|sampled| {
                                    sampled.marquee_hits(
                                        self.transform(r),
                                        ScreenPoint::new(start.x as f64, start.y as f64),
                                        ScreenPoint::new(end.x as f64, end.y as f64),
                                    )
                                })
                                .unwrap_or_default();
                            let add = ui.input(|i| i.modifiers.shift);
                            let mut selection = if add {
                                base.spans().cloned().unwrap_or_default()
                            } else {
                                BTreeSet::new()
                            };
                            selection.extend(hits);
                            self.selection = if selection.is_empty() {
                                TopologySelection::None
                            } else {
                                TopologySelection::Spans(selection)
                            };
                        }
                    }
                    _ => self.editor.commit(),
                }
            }
        }
        if response.clicked() && self.drag.is_none() {
            if let Some(pos) = pointer {
                let hit = self.sampled.as_ref().and_then(|sampled| {
                    sampled.hit_test(
                        self.transform(r),
                        ScreenPoint::new(pos.x as f64, pos.y as f64),
                        8.0,
                        7.0,
                    )
                });
                if let Some(hit) = hit {
                    self.selected_probe = None;
                    self.selection.apply_hit(
                        &self.editor.document.model.draft.geometry,
                        hit,
                        ui.input(|i| i.modifiers.shift),
                        ui.input(|i| i.modifiers.command),
                    );
                } else {
                    self.selection = TopologySelection::None;
                    self.selected_probe = None;
                }
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) {
            self.delete_selection();
        }
    }
    fn draw_click(&mut self, mut point: Point2, screen: ScreenPoint, r: Rect) {
        let Some(mut gesture) = self.draw.take() else {
            return;
        };
        if gesture.tool == DrawTool::Circle {
            let spline = PeriodicCubicSpline::rounded(point, 0.15);
            let purpose = match self.closed_purpose {
                ClosedPurpose::Subdomain => ClosedCurvePurpose::Subdomain {
                    material: self.material_selection,
                },
                ClosedPurpose::Hole => ClosedCurvePurpose::Hole,
            };
            match self.editor.create_closed_curve(spline, purpose) {
                Ok(curve) => {
                    self.select_curve(curve);
                    self.notify("Closed curve added");
                }
                Err(error) => self.notify(error),
            }
            self.invalidate_samples();
            return;
        }
        let open = matches!(gesture.tool, DrawTool::Polyline | DrawTool::OpenSpline);
        let mut attachment = None;
        if open {
            let face = if self.open_purpose == OpenPurpose::Separator {
                gesture.face
            } else {
                None
            };
            if let Some(compiled) = &self.editor.compiled_draft {
                if let Some(hit) = hit_attachment(compiled, self.transform(r), screen, 11.0, face) {
                    point = hit.point;
                    attachment = Some(hit.attachment);
                    if gesture.points.is_empty() {
                        gesture.face = Some(attachment_face(hit.attachment, compiled));
                    }
                }
            }
            if self.open_purpose == OpenPurpose::Separator
                && gesture.points.is_empty()
                && attachment.is_none()
            {
                self.message = "Start a separator on an active boundary".into();
                self.draw = Some(gesture);
                return;
            }
        }
        if gesture
            .points
            .first()
            .is_some_and(|first| (point - *first).norm() < 8.0 / self.scale)
            && gesture.points.len() >= 3
            && !open
        {
            self.draw = Some(gesture);
            self.finish_draw();
            return;
        }
        gesture.points.push(point);
        gesture.attachments.push(attachment);
        let finish = gesture.tool == DrawTool::Rectangle && gesture.points.len() == 2;
        self.draw = Some(gesture);
        if finish {
            self.finish_draw();
        }
    }
    fn finish_draw(&mut self) {
        let Some(gesture) = self.draw.take() else {
            return;
        };
        let result: Result<CurveId, String> = match gesture.tool {
            DrawTool::Rectangle if gesture.points.len() == 2 => {
                let a = gesture.points[0];
                let b = gesture.points[1];
                let points = vec![
                    Point2::new(a.x, a.y),
                    Point2::new(b.x, a.y),
                    Point2::new(b.x, b.y),
                    Point2::new(a.x, b.y),
                ];
                PeriodicCubicSpline::polygon(points)
                    .map_err(|e| e.to_string())
                    .and_then(|s| self.create_closed(s))
            }
            DrawTool::Polygon if gesture.points.len() >= 3 => {
                PeriodicCubicSpline::polygon(gesture.points.clone())
                    .map_err(|e| e.to_string())
                    .and_then(|s| self.create_closed(s))
            }
            DrawTool::ClosedSpline if gesture.points.len() >= 4 => {
                PeriodicCubicSpline::uniform(gesture.points.clone())
                    .map_err(|e| e.to_string())
                    .and_then(|s| self.create_closed(s))
            }
            DrawTool::Polyline if gesture.points.len() >= 2 => {
                OpenCubicSpline::polyline(gesture.points.clone())
                    .map_err(|e| e.to_string())
                    .and_then(|s| self.create_open(s, &gesture))
            }
            DrawTool::OpenSpline if gesture.points.len() == 2 => {
                OpenCubicSpline::polyline(gesture.points.clone())
                    .map_err(|e| e.to_string())
                    .and_then(|s| self.create_open(s, &gesture))
            }
            DrawTool::OpenSpline if gesture.points.len() >= 4 => {
                OpenCubicSpline::uniform(gesture.points.clone())
                    .map_err(|e| e.to_string())
                    .and_then(|s| self.create_open(s, &gesture))
            }
            _ => Err("Add enough points to finish this curve".into()),
        };
        match result {
            Ok(curve) => {
                self.select_curve(curve);
                self.notify("Curve added");
                self.invalidate_samples();
            }
            Err(error) => {
                self.message = error;
                self.draw = Some(gesture);
            }
        }
    }
    fn create_closed(&mut self, spline: PeriodicCubicSpline) -> Result<CurveId, String> {
        let purpose = match self.closed_purpose {
            ClosedPurpose::Subdomain => ClosedCurvePurpose::Subdomain {
                material: self.material_selection,
            },
            ClosedPurpose::Hole => ClosedCurvePurpose::Hole,
        };
        self.editor.create_closed_curve(spline, purpose)
    }
    fn create_open(
        &mut self,
        spline: OpenCubicSpline,
        gesture: &DrawGesture,
    ) -> Result<CurveId, String> {
        let purpose = match self.open_purpose {
            OpenPurpose::Separator => OpenCurvePurpose::SubdomainSeparator {
                material: self.new_separator_material,
            },
            OpenPurpose::Baffle => OpenCurvePurpose::BoundaryBaffle,
        };
        let start = gesture.attachments.first().copied().flatten();
        let end = gesture.attachments.last().copied().flatten();
        self.editor
            .create_open_curve(spline, purpose, start, end)
            .map(|edit| edit.curve)
    }
    fn select_curve(&mut self, id: CurveId) {
        if let Some(curve) = self
            .editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == id)
        {
            self.selection = TopologySelection::Spans(
                curve
                    .spans
                    .iter()
                    .map(|span| TopologySpanTarget::Curve(span.id))
                    .collect(),
            );
        }
    }
    fn delete_selection(&mut self) {
        if let Some(probe) = self.selected_probe {
            match self.editor.delete_probe(probe) {
                Ok(()) => {
                    self.selected_probe = None;
                    self.probe_windows.remove(&probe);
                }
                Err(error) => self.message = error,
            }
            return;
        }
        if let TopologySelection::Handle(TopologyHandle::Control { curve, control }) =
            &self.selection
        {
            let (curve, control) = (*curve, *control);
            match self.editor.remove_control(curve, control) {
                Ok(()) => {
                    self.selection = TopologySelection::None;
                    self.invalidate_samples();
                }
                Err(error) => self.message = error,
            }
            return;
        }
        let curves = match &self.selection {
            TopologySelection::Spans(spans) => self
                .editor
                .document
                .model
                .draft
                .geometry
                .curves
                .iter()
                .filter(|curve| {
                    curve
                        .spans
                        .iter()
                        .all(|span| spans.contains(&TopologySpanTarget::Curve(span.id)))
                })
                .map(|curve| curve.id)
                .collect::<Vec<_>>(),
            _ => vec![],
        };
        if curves.is_empty() {
            return;
        }
        for curve in curves {
            if let Err(error) = self.editor.remove_curve(curve, None) {
                self.message = error;
                return;
            }
        }
        self.selection = TopologySelection::None;
        self.material_edit = None;
        self.material_formula_edits.clear();
        self.material_formula_errors.clear();
        self.invalidate_samples();
    }
    fn place_pulse(&mut self, point: Point2) {
        let Some(active) = self.runtime.active() else {
            return;
        };
        let region = active.bundle.snapshot.face_at(point).and_then(|face| {
            active
                .bundle
                .plan
                .domains
                .iter()
                .find(|domain| domain.face == face)
                .map(|domain| domain.region)
        });
        let Some(region) = region else {
            self.message = "Pulse must be inside an active subdomain".into();
            return;
        };
        self.pending_pulse = Some((point, region));
        self.message = format!("Pulse queued in region {}", region.0);
    }
    fn request_runtime(&mut self) {
        if self.editor.acceptance != TopologyAcceptance::Valid || self.editor.editing() {
            return;
        }
        let options = MeshingOptions {
            target_edge_length: self.mesh_edge,
            curve_tolerance: (self.mesh_edge * 0.02).min(5e-4),
            ..MeshingOptions::default()
        };
        if self.requested_revision == Some(self.editor.revision)
            && self.requested_edge == self.mesh_edge
            && (self.runtime.phase().is_some()
                || self.runtime.active().is_some_and(|active| {
                    active.bundle.token.document_revision == self.editor.revision
                }))
        {
            return;
        }
        let fresh = self.runtime.active().is_none() || self.reset_requested;
        match self.runtime.request(
            self.editor.revision,
            &self.editor.document,
            self.editor.compiled_accepted.clone(),
            options,
            fresh,
        ) {
            Ok(_) => {
                self.requested_revision = Some(self.editor.revision);
                self.requested_edge = self.mesh_edge;
                self.reset_requested = false;
            }
            Err(error) => self.message = error,
        }
    }
    fn refresh_runtime(
        &mut self,
        request: &mut WaveGpuRequest,
        display: &WaveDisplay,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        delta: f64,
    ) {
        self.request_runtime();
        let _ = self.runtime.advance(256);
        if self.uploading.is_none() && self.runtime.ready().is_some() && request.caught_up() {
            let candidate = self.runtime.ready().unwrap().clone();
            let in_place = candidate.operator_reused
                && candidate.transfer.is_none()
                && self.runtime.active().is_some_and(|active| {
                    Arc::ptr_eq(&active.mesh, &candidate.mesh)
                        && Arc::ptr_eq(&active.operator, &candidate.operator)
                });
            if in_place {
                let token = candidate.bundle.token;
                match request.update_source(
                    assets,
                    &candidate.mesh,
                    &candidate.operator,
                    candidate.point_source,
                ) {
                    Ok(()) => match self.runtime.commit_ready(token) {
                        Ok(active) => {
                            self.message = "Simulation settings committed".into();
                            self.configure_probes(request, assets, commands, &active);
                        }
                        Err(error) => self.message = error,
                    },
                    Err(error) => {
                        self.runtime.reject_ready(token, error.clone());
                        self.message = error;
                    }
                }
            } else {
                let dt = candidate.operator.recommended_time_step();
                let upload = if candidate.fresh || self.runtime.active().is_none() {
                    request.replace_with_volume_sources(
                        assets,
                        commands,
                        &candidate.mesh,
                        &candidate.operator,
                        dt,
                        candidate.point_source,
                        &candidate.volume_sources,
                    )
                } else if let (Some(active), Some(map)) =
                    (self.runtime.active(), candidate.transfer.as_ref())
                {
                    request.replace_transferred_with_volume_sources(
                        assets,
                        commands,
                        WaveTransfer {
                            source_mesh: &active.mesh,
                            source_operator: &active.operator,
                            target_mesh: &candidate.mesh,
                            target_operator: &candidate.operator,
                            target_time_step: dt,
                            source: candidate.point_source,
                            map,
                        },
                        &candidate.volume_sources,
                    )
                } else {
                    request.replace_with_volume_sources(
                        assets,
                        commands,
                        &candidate.mesh,
                        &candidate.operator,
                        dt,
                        candidate.point_source,
                        &candidate.volume_sources,
                    )
                };
                match upload {
                    Ok(()) => {
                        let time_offset = if candidate.fresh {
                            0.0
                        } else {
                            self.runtime
                                .active()
                                .map_or(self.sim_time_offset, |active| {
                                    self.sim_time_offset
                                        + display.completed_steps as f64
                                            * active.operator.recommended_time_step()
                                })
                        };
                        self.uploading = Some(Uploading {
                            token: candidate.bundle.token,
                            generation: request.generation(),
                            fresh: candidate.fresh,
                            time_offset,
                        })
                    }
                    Err(error) => {
                        let token = candidate.bundle.token;
                        self.runtime.reject_ready(token, error.clone());
                        self.message = error;
                    }
                }
            }
        }
        if let Some(upload) = &self.uploading {
            if request.failed() {
                let token = upload.token;
                if request.transfer_pending() {
                    let _ = request.rollback_transfer(assets, commands);
                }
                self.runtime.reject_ready(token, "GPU upload failed");
                self.uploading = None;
            } else if request.ready()
                && display.generation == upload.generation
                && display.current.len()
                    == self
                        .runtime
                        .ready()
                        .map_or(0, |candidate| candidate.operator.degrees_of_freedom())
            {
                let upload = self.uploading.take().unwrap();
                request.finish_transfer(assets);
                match self.runtime.commit_ready(upload.token) {
                    Ok(active) => {
                        self.sim_time_offset = upload.time_offset;
                        if upload.fresh {
                            self.accumulator = 0.0;
                        }
                        self.amr_adaptation_state = if active.adapted {
                            self.amr_pending_state
                                .take()
                                .filter(|state| state.mesh_revision == active.mesh.mesh_revision)
                                .or_else(|| Some(MeshAdaptationState::from_mesh(&active.mesh)))
                        } else {
                            self.amr_pending_state = None;
                            Some(MeshAdaptationState::from_mesh(&active.mesh))
                        };
                        self.amr_indicator_job = None;
                        self.amr_indicator_result = None;
                        self.amr_last_analyzed_step = None;
                        self.probe_gpu_token = None;
                        self.message = "Simulation topology committed".into();
                        self.configure_probes(request, assets, commands, &active);
                    }
                    Err(error) => self.message = error,
                }
            }
        }
        if self.reset_requested {
            if let Some(active) = self.runtime.active() {
                let dt = active.operator.recommended_time_step();
                if request
                    .reset(
                        assets,
                        commands,
                        &active.mesh,
                        &active.operator,
                        dt,
                        active.point_source,
                    )
                    .is_ok()
                {
                    self.reset_requested = false;
                    self.sim_time_offset = 0.0;
                }
            }
        }
        if let Some(active) = self.runtime.active() {
            let dt = active.operator.recommended_time_step();
            if let Some((position, region)) = self.pending_pulse.take()
                && let Err(error) = request.inject_pulse(
                    assets,
                    &active.mesh,
                    &active.operator,
                    PulseSettings {
                        position,
                        width: self.pulse_width,
                        amplitude: self.pulse_amplitude,
                        region,
                    },
                )
            {
                self.message = error;
            }
            if self.wave_running {
                self.accumulator += delta;
                let steps = (self.accumulator / dt).floor().min(4096.0) as u64;
                if steps > 0 {
                    request.request_steps(steps);
                    self.accumulator -= steps as f64 * dt;
                }
            } else if self.wave_step {
                request.request_steps(1);
                self.wave_step = false;
            }
        }
        self.completed_steps = request.stats().completed_steps();
        if let Some(active) = self.runtime.active()
            && display.generation == request.generation()
            && display.current.len() == active.operator.degrees_of_freedom()
        {
            let current = display
                .current
                .iter()
                .map(|value| *value as f64)
                .collect::<Vec<_>>();
            let previous = display
                .previous
                .iter()
                .map(|value| *value as f64)
                .collect::<Vec<_>>();
            let auxiliary = display
                .auxiliary
                .iter()
                .map(|value| *value as f64)
                .collect::<Vec<_>>();
            self.wave_energy = active
                .operator
                .discrete_energy_with_auxiliary(
                    &current,
                    &previous,
                    &auxiliary,
                    active.operator.recommended_time_step(),
                )
                .ok();
        }
        let elapsed = self.rate_started.elapsed().as_secs_f64();
        if elapsed >= 0.5 {
            self.steps_per_second =
                (self.completed_steps.saturating_sub(self.rate_steps)) as f64 / elapsed;
            self.rate_steps = self.completed_steps;
            self.rate_started = Instant::now();
        }
    }
    fn configure_probes(
        &mut self,
        request: &mut WaveGpuRequest,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        active: &Arc<PreparedTopology>,
    ) {
        let mut points = vec![];
        let mut curves = vec![];
        let mut areas = vec![];
        for compiled in active.probes.iter() {
            let definition = self
                .editor
                .document
                .model
                .probes
                .iter()
                .find(|probe| probe.id == compiled.id);
            let Some(definition) = definition else {
                continue;
            };
            match &compiled.result {
                TopologyProbeCompilation::Ready(stencil) => match stencil.as_ref() {
                    TopologyProbeStencil::Point(stencil) => {
                        points.push((compiled.id.0, Some(*stencil)))
                    }
                    TopologyProbeStencil::Segment(stencils) => {
                        if let TopologyProbeTarget::Segment { start, end, preset } =
                            definition.target
                        {
                            let count = stencils.len();
                            curves.push(CurveProbeInput {
                                id: compiled.id.0,
                                sample_rate: preset.sample_rate(),
                                samples: stencils
                                    .iter()
                                    .enumerate()
                                    .map(|(index, stencil)| {
                                        Some((
                                            *stencil,
                                            start.lerp(
                                                end,
                                                index as f64 / (count - 1).max(1) as f64,
                                            ),
                                        ))
                                    })
                                    .collect(),
                            });
                        }
                    }
                    TopologyProbeStencil::Boundary(stencils) => {
                        let rate = match &definition.target {
                            TopologyProbeTarget::Boundary(target) => target.preset.sample_rate(),
                            _ => 60.0,
                        };
                        curves.push(CurveProbeInput {
                            id: compiled.id.0,
                            sample_rate: rate,
                            samples: stencils
                                .iter()
                                .map(|sample| Some((sample.stencil, sample.point)))
                                .collect(),
                        });
                    }
                    TopologyProbeStencil::Area(stencil) => areas.push(AreaProbeInput {
                        id: compiled.id.0,
                        stencil: Some(stencil.clone()),
                    }),
                },
                TopologyProbeCompilation::Disabled => {}
                TopologyProbeCompilation::Failed(error) => {
                    self.message = format!("Probe {}: {error}", definition.name)
                }
            }
        }
        let dt = active.operator.recommended_time_step();
        let physics = active.bundle.authored.physics;
        let result = request
            .update_point_probes(assets, commands, &points, 120.0, dt, physics)
            .and_then(|()| request.update_curve_probes(assets, commands, &curves, dt, physics))
            .and_then(|()| request.update_area_probes(assets, commands, &areas, 60.0, dt, physics));
        if let Err(error) = result {
            self.message = error;
            return;
        }
        let far = active
            .far_field
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .map(|stencil| FarFieldInput {
                samples: stencil.samples.clone(),
                wave_speed: stencil.wave_speed,
                sample_spacing: stencil.sample_spacing,
                delay_margin: stencil.delay_margin,
            });
        if let Err(error) = request.update_far_field(assets, commands, far.as_ref(), dt) {
            self.message = error;
        } else {
            self.probe_gpu_token = Some(active.bundle.token);
        }
    }
    fn refresh_amr(&mut self, request: &WaveGpuRequest, display: &WaveDisplay) {
        if !self.amr_enabled {
            self.amr_indicator_job = None;
            self.amr_adaptation_job = None;
            self.amr_indicator_result = None;
            self.amr_coarsen_streak = 0;
            self.amr_status = "off".into();
            return;
        }
        if self.uploading.is_some() || self.runtime.phase().is_some() || self.editor.editing() {
            self.amr_status = if self.amr_adaptation_job.is_some() {
                "adapting mesh"
            } else {
                "geometry has priority"
            }
            .into();
            return;
        }

        if let Some(job) = &mut self.amr_adaptation_job {
            self.amr_status = job.phase().into();
            let started = Instant::now();
            let mut result = None;
            while result.is_none() && started.elapsed().as_secs_f64() < 0.002 {
                result = job.advance(64);
            }
            let Some(result) = result else { return };
            self.amr_adaptation_job = None;
            match result {
                Ok(result) => {
                    self.amr_pending_state = Some(result.state);
                    match self.runtime.request_adapted(
                        self.editor.revision,
                        &self.editor.document,
                        result.mesh,
                    ) {
                        Ok(_) => {
                            self.amr_status = "preparing adaptive handoff".into();
                            self.amr_error = None;
                        }
                        Err(error) => {
                            self.amr_pending_state = None;
                            self.amr_status = "adaptation discarded".into();
                            self.amr_error = Some(error);
                        }
                    }
                }
                Err(error) => {
                    self.amr_status = "adaptation failed".into();
                    self.amr_error = Some(error.to_string());
                }
            }
            return;
        }

        if let Some(job) = &mut self.amr_indicator_job {
            self.amr_status = job.phase().into();
            let started = Instant::now();
            let mut result = None;
            while result.is_none() && started.elapsed().as_secs_f64() < 0.002 {
                result = job.advance(64);
            }
            let Some(result) = result else { return };
            let source = self.amr_indicator_source.take();
            self.amr_indicator_job = None;
            let Some((token, generation, buffer_revision, step)) = source else {
                self.amr_status = "discarded stale estimate".into();
                return;
            };
            let Some(active) = self.runtime.active().cloned() else {
                return;
            };
            if active.bundle.token != token
                || request.generation() != generation
                || request.buffer_revision() != buffer_revision
            {
                self.amr_status = "discarded stale estimate".into();
                return;
            }
            self.amr_last_analyzed_step = Some(step);
            let result = match result {
                Ok(result) => result,
                Err(error) => {
                    self.amr_status = "estimate failed".into();
                    self.amr_error = Some(error.to_string());
                    return;
                }
            };
            let refine = result.report.refine_candidates >= 4;
            let coarsen = result.report.coarsen_candidates >= 4;
            self.amr_coarsen_streak = if coarsen {
                self.amr_coarsen_streak.saturating_add(1)
            } else {
                0
            };
            self.amr_indicator_result = Some(result.clone());
            if !refine && !coarsen {
                self.amr_status = "mesh matches solution".into();
                self.amr_error = None;
                return;
            }
            if !refine && self.amr_coarsen_streak < 2 {
                self.amr_status = "confirming coarsening".into();
                return;
            }
            let Some(state) = self
                .amr_adaptation_state
                .clone()
                .filter(|state| state.mesh_revision == active.mesh.mesh_revision)
            else {
                self.amr_adaptation_state = Some(MeshAdaptationState::from_mesh(&active.mesh));
                self.amr_status = "initializing adaptation".into();
                return;
            };
            let target_revision = self.runtime.reserve_mesh_revision();
            let options = MeshAdaptationOptions {
                meshing: MeshingOptions {
                    curve_tolerance: (self.amr_minimum_edge * 0.02).min(1.5e-3),
                    target_edge_length: self.amr_maximum_edge / 1.05,
                    minimum_angle_degrees: 12.0,
                    max_vertices: 50_000,
                    max_triangles: 100_000,
                    max_refinement_steps: 50_000,
                },
                minimum_target_edge_length: self.amr_minimum_edge,
                maximum_target_edge_length: self.amr_maximum_edge,
                collapse_ratio: 0.65,
                max_topology_changes: 512,
                max_coarsening_changes: if self.amr_coarsen_streak >= 2 { 256 } else { 0 },
                max_work_units: 5_000_000,
                ..Default::default()
            };
            let field: Arc<dyn MeshSizeField> = result.field;
            self.amr_adaptation_job = Some(MeshAdaptationJob::new_topology(
                active.mesh.clone(),
                &active.bundle.plan,
                state,
                target_revision,
                field,
                options,
            ));
            self.amr_status = "adapting mesh".into();
            return;
        }

        let Some(active) = self.runtime.active() else {
            self.amr_status = "waiting for solution".into();
            return;
        };
        let dofs = active.operator.degrees_of_freedom();
        if !request.ready()
            || display.generation != request.generation()
            || display.current.len() != dofs
            || display.auxiliary.len() != dofs
            || display.indicator_displacement.len() != dofs
            || display.indicator_velocity.len() != dofs
            || display.indicator_acceleration.len() != dofs
        {
            self.amr_status = "waiting for aligned readback".into();
            return;
        }
        let step = display.completed_steps;
        if self
            .amr_last_analyzed_step
            .is_some_and(|previous| step < previous.saturating_add(8))
            || self
                .amr_last_started
                .is_some_and(|started| started.elapsed().as_secs_f64() < 0.75)
        {
            self.amr_status = "monitoring solution".into();
            return;
        }
        let dt = active.operator.recommended_time_step();
        let time = self.sim_time_offset + step.saturating_sub(1) as f64 * dt;
        let Some(volume_acceleration) = request.volume_acceleration(time) else {
            self.amr_status = "waiting for source state".into();
            return;
        };
        let Some(auxiliary) = aligned_indicator_auxiliary(display, &active.operator, dt, step)
        else {
            self.amr_status = "waiting for aligned readback".into();
            return;
        };
        let snapshot = QuadraticSolutionSnapshot {
            mesh_revision: active.mesh.mesh_revision,
            displacement: display
                .indicator_displacement
                .iter()
                .map(|value| *value as f64)
                .collect(),
            velocity: display
                .indicator_velocity
                .iter()
                .map(|value| *value as f64)
                .collect(),
            acceleration: display
                .indicator_acceleration
                .iter()
                .map(|value| *value as f64)
                .collect(),
            auxiliary,
            volume_acceleration,
            time,
            time_step: dt,
        };
        self.amr_indicator_job = Some(SolutionIndicatorJob::new_topology(
            active.mesh.clone(),
            active.operator.clone(),
            &active.bundle.plan,
            active.bundle.model(),
            snapshot,
            SolutionIndicatorOptions {
                minimum_edge_length: self.amr_minimum_edge,
                maximum_edge_length: self.amr_maximum_edge,
                forcing_frequency_hz: highest_forcing_frequency(
                    &active.bundle.authored,
                    active.point_source,
                ),
                ..Default::default()
            },
        ));
        self.amr_indicator_source = Some((
            active.bundle.token,
            request.generation(),
            request.buffer_revision(),
            step,
        ));
        self.amr_last_started = Some(Instant::now());
        self.amr_status = "preparing estimate".into();
    }
    fn ingest_probes(&mut self, display: &ProbeDisplay) {
        if display.readbacks == self.probe_readback {
            return;
        }
        self.probe_readback = display.readbacks;
        for record in &display.records {
            let trace = self
                .probe_traces
                .entry(ProbeId(record.probe_id))
                .or_default();
            let time = self.sim_time_offset + record.time;
            if time > trace.last_time {
                trace.last_time = time;
                trace.samples.push_back((time, record.displacement));
                while trace.samples.len() > 4096 {
                    trace.samples.pop_front();
                }
            }
        }
    }
    fn ingest_spatial_probes(
        &mut self,
        curves: &CurveProbeDisplay,
        areas: &AreaProbeDisplay,
        far: &FarFieldDisplay,
    ) {
        if curves.readbacks != self.curve_probe_readback {
            self.curve_probe_readback = curves.readbacks;
            for record in &curves.records {
                let trace = self
                    .curve_probe_traces
                    .entry(ProbeId(record.probe_id))
                    .or_default();
                let mut record = record.clone();
                record.time += self.sim_time_offset;
                if record.time > trace.last_time {
                    trace.last_time = record.time;
                    trace.records.push_back(record);
                    while trace.records.len() > 512 {
                        trace.records.pop_front();
                    }
                }
            }
        }
        if areas.readbacks != self.area_probe_readback {
            self.area_probe_readback = areas.readbacks;
            for record in &areas.records {
                let trace = self
                    .area_probe_traces
                    .entry(ProbeId(record.probe_id))
                    .or_default();
                let mut record = *record;
                record.time += self.sim_time_offset;
                if record.time > trace.last_time {
                    trace.last_time = record.time;
                    trace.records.push_back(record);
                    while trace.records.len() > 4096 {
                        trace.records.pop_front();
                    }
                }
            }
        }
        if far.readbacks != self.far_field_readback {
            self.far_field_readback = far.readbacks;
            for record in &far.records {
                let mut record = record.clone();
                record.time += self.sim_time_offset;
                if record.time > self.far_field_trace.last_time {
                    self.far_field_trace.last_time = record.time;
                    self.far_field_trace.records.push_back(record);
                    while self.far_field_trace.records.len() > 512 {
                        self.far_field_trace.records.pop_front();
                    }
                }
            }
        }
    }
    fn refresh_material_overlay(&mut self) {
        if !matches!(
            self.editor.document.presentation.material_overlay,
            MaterialOverlay::Property(_)
        ) {
            self.material_overlay_job = None;
            return;
        }
        let Some(active) = self.runtime.active() else {
            return;
        };
        let key = OverlayKey {
            mesh_revision: active.mesh.mesh_revision,
            scene: active.bundle.authored.as_ref().clone(),
        };
        if self
            .material_overlay_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.key == key)
        {
            self.material_overlay_job = None;
            return;
        }
        if !self
            .material_overlay_job
            .as_ref()
            .is_some_and(|job| job.key() == &key)
        {
            match MaterialOverlayJob::new(
                active.mesh.clone(),
                active.operator.clone(),
                key.scene.clone(),
            ) {
                Ok(job) => {
                    self.material_overlay_job = Some(job);
                    self.material_overlay_error = None;
                }
                Err(error) => {
                    self.material_overlay_job = None;
                    self.material_overlay_error = Some(error);
                    return;
                }
            }
        }
        if let Some(snapshot) = self
            .material_overlay_job
            .as_mut()
            .and_then(|job| job.advance(2_048))
        {
            if snapshot.key == key {
                self.material_overlay_snapshot = Some(snapshot);
                self.material_overlay_error = None;
            }
            self.material_overlay_job = None;
        }
    }
    fn material_overlay_range(&self, property: MaterialProperty) -> Option<OverlayRange> {
        let presentation = self.editor.document.presentation;
        if presentation.material_overlay_auto_range {
            self.material_overlay_snapshot
                .as_ref()?
                .range(property, presentation.material_overlay_logarithmic)
        } else if presentation.material_overlay_manual_min.is_finite()
            && presentation.material_overlay_manual_max.is_finite()
            && presentation.material_overlay_manual_min < presentation.material_overlay_manual_max
        {
            Some(OverlayRange {
                minimum: presentation.material_overlay_manual_min,
                maximum: presentation.material_overlay_manual_max,
            })
        } else {
            None
        }
    }
    fn probe_windows(&mut self, ctx: &egui::Context) {
        let ids = self.probe_windows.iter().copied().collect::<Vec<_>>();
        for id in ids {
            let Some(probe) = self
                .editor
                .document
                .model
                .probes
                .iter()
                .find(|probe| probe.id == id)
                .cloned()
            else {
                self.probe_windows.remove(&id);
                continue;
            };
            let mut open = true;
            egui::Window::new(probe.name)
                .id(egui::Id::new(("probe", id.0)))
                .open(&mut open)
                .default_size([420.0, 220.0])
                .show(ctx, |ui| {
                    if ui.button("Clear").clicked() {
                        self.probe_traces.remove(&id);
                        self.curve_probe_traces.remove(&id);
                        self.area_probe_traces.remove(&id);
                    }
                    match probe.target {
                        TopologyProbeTarget::Point(_) => {
                            ui.label("Field × time");
                            let values = self
                                .probe_traces
                                .get(&id)
                                .map(|trace| trace.samples.iter().copied().collect::<Vec<_>>())
                                .unwrap_or_default();
                            draw_time_series(ui, &values, TEAL, 180.0);
                        }
                        TopologyProbeTarget::Segment { .. } | TopologyProbeTarget::Boundary(_) => {
                            let latest = self
                                .curve_probe_traces
                                .get(&id)
                                .and_then(|trace| trace.records.back());
                            for (label, values, color) in latest.map_or_else(Vec::new, |record| {
                                vec![
                                    ("Field × arclength", record.displacement.as_slice(), TEAL),
                                    ("Flux × arclength", record.normal_flux.as_slice(), GOLD),
                                    (
                                        "Energy × arclength",
                                        record.energy_density.as_slice(),
                                        Color32::from_rgb(174, 126, 241),
                                    ),
                                ]
                            }) {
                                ui.label(label);
                                draw_profile(ui, values, color, 105.0);
                            }
                        }
                        TopologyProbeTarget::AreaDisk { .. }
                        | TopologyProbeTarget::AreaRegion(_) => {
                            ui.label("Integrated energy × time");
                            let values = self
                                .area_probe_traces
                                .get(&id)
                                .map(|trace| {
                                    trace
                                        .records
                                        .iter()
                                        .map(|record| (record.time, record.total_energy))
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default();
                            draw_time_series(ui, &values, GOLD, 180.0);
                            if let Some(record) = self
                                .area_probe_traces
                                .get(&id)
                                .and_then(|trace| trace.records.back())
                            {
                                ui.small(format!(
                                    "mean field {:.3e} · area {:.3} · coverage {:.0}%",
                                    record.mean_displacement,
                                    record.covered_area,
                                    record.coverage * 100.0
                                ));
                            }
                        }
                    }
                });
            if !open {
                self.probe_windows.remove(&id);
            }
        }
        if self.far_field_window {
            let mut open = true;
            egui::Window::new("Far field")
                .id(egui::Id::new("far-field-readout"))
                .open(&mut open)
                .default_size([560.0, 620.0])
                .show(ctx, |ui| {
                    if ui.button("Clear").clicked() {
                        self.far_field_trace = FarFieldTrace::default();
                    }
                    if let Some(record) = self.far_field_trace.records.back() {
                        ui.label("Direction × time");
                        draw_far_waterfall(ui, &self.far_field_trace.records, 190.0);
                        ui.label("Relative radiation pattern");
                        draw_polar(ui, &record.intensity, 250.0);
                    }
                    ui.label("Radiated power × time");
                    let values = self
                        .far_field_trace
                        .records
                        .iter()
                        .map(|record| {
                            (
                                record.time,
                                record
                                    .intensity
                                    .iter()
                                    .map(|value| *value as f64)
                                    .sum::<f64>(),
                            )
                        })
                        .collect::<Vec<_>>();
                    draw_time_series(ui, &values, GOLD, 120.0);
                });
            self.far_field_window = open;
        }
    }
    fn status_bar(&mut self, root: &mut egui::Ui) {
        egui::Panel::bottom("status")
            .exact_size(29.0)
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    let status = match self.editor.acceptance {
                        TopologyAcceptance::Invalid(issue) => format!("Geometry invalid: {issue}"),
                        TopologyAcceptance::Pending => "Topology rebuilding: tracing faces".into(),
                        TopologyAcceptance::Valid => self.runtime.phase().map_or_else(
                            || "Simulation ready".into(),
                            |phase| self.runtime.detail().unwrap_or(phase.label()).into(),
                        ),
                    };
                    ui.label(status);
                    if self.recording_state == RecordingState::SelectingDestination {
                        ui.colored_label(GOLD, "Choosing recording destination…");
                    } else if matches!(
                        self.recording_state,
                        RecordingState::Requested
                            | RecordingState::Preparing
                            | RecordingState::Starting
                    ) {
                        ui.colored_label(GOLD, "Preparing recording…");
                    } else if self.recording_state == RecordingState::Recording {
                        let elapsed = self
                            .recording_started
                            .map(recording::elapsed_label)
                            .unwrap_or_else(|| "00:00".into());
                        ui.colored_label(RED, format!("● REC {elapsed}"));
                        if self.recording_dropped_frames > 0 {
                            ui.colored_label(
                                GOLD,
                                format!("{} dropped", self.recording_dropped_frames),
                            );
                        }
                        if ui.small_button("Stop").clicked() {
                            self.stop_video_recording();
                        }
                    } else if self.recording_state == RecordingState::Finalizing {
                        ui.colored_label(GOLD, "Finalizing recording…");
                    } else if self.snapshot_state != SnapshotState::Idle {
                        ui.colored_label(GOLD, "Capturing snapshot…");
                    }
                    if !self.message.is_empty() {
                        ui.separator();
                        ui.label(&self.message);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let active = self.runtime.active();
                        ui.label(format!(
                            "{:.0} fps · {:.0} steps/s · {} dofs · {} elements · dt {}",
                            1000.0 / self.frame_ms.max(0.01),
                            self.steps_per_second,
                            active.map_or(0, |v| v.operator.degrees_of_freedom()),
                            active.map_or(0, |v| v.mesh.triangles.len()),
                            active.map_or("—".into(), |v| format!(
                                "{:.2e}",
                                v.operator.recommended_time_step()
                            ))
                        ));
                    });
                });
            });
    }
    fn show(&mut self, root: &mut egui::Ui, display: &WaveDisplay) -> Rect {
        if self.logo_texture.is_none() {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [216, 88],
                include_bytes!("../../../assets/logo-216x88.rgba"),
            );
            self.logo_texture = Some(root.ctx().load_texture(
                "funfern-logo",
                image,
                egui::TextureOptions::LINEAR,
            ));
        }
        self.top_bar(root);
        self.status_bar(root);
        self.side_panel(root);
        let mut viewport = Rect::NOTHING;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(root, |ui| {
                viewport = self.viewport(ui, display);
            });
        self.probe_windows(root.ctx());
        viewport
    }
}

#[allow(clippy::too_many_arguments)]
fn material_scalar_editor(
    ui: &mut egui::Ui,
    key: (u64, u8),
    label: &str,
    field: &mut ScalarField,
    parameters: &[MaterialParameter],
    minimum: f64,
    edits: &mut BTreeMap<(u64, u8), String>,
    errors: &mut BTreeMap<(u64, u8), String>,
) {
    let mut formula_mode = matches!(field, ScalarField::Formula(_));
    let was_formula = formula_mode;
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt(("field-mode", key))
            .selected_text(if formula_mode { "Formula" } else { "Constant" })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut formula_mode, false, "Constant");
                ui.selectable_value(&mut formula_mode, true, "Formula");
            });
    });
    if formula_mode != was_formula {
        if formula_mode {
            let source = field.constant_value().unwrap_or(1.0).to_string();
            *field = ScalarField::formula(source.clone()).unwrap();
            edits.insert(key, source);
            errors.remove(&key);
        } else {
            let coordinates = MaterialCoordinates {
                x: 0.0,
                y: 0.0,
                r: 0.0,
                theta: 0.0,
            };
            match field.evaluate(coordinates, parameters) {
                Ok(value) if value >= minimum => {
                    *field = ScalarField::constant(value);
                    edits.remove(&key);
                    errors.remove(&key);
                }
                Ok(_) => {
                    errors.insert(key, format!("Value must be at least {minimum}"));
                }
                Err(error) => {
                    errors.insert(key, error.to_string());
                }
            }
        }
    }
    match field {
        ScalarField::Constant(value) => {
            ui.add(
                egui::DragValue::new(value)
                    .speed(if minimum == 0.0 { 0.005 } else { 0.01 })
                    .range(minimum..=1.0e6)
                    .update_while_editing(false),
            );
        }
        ScalarField::Formula(formula) => {
            let source = edits
                .entry(key)
                .or_insert_with(|| formula.source().to_owned());
            let response = ui.add(
                egui::TextEdit::singleline(source)
                    .desired_width(ui.available_width())
                    .hint_text("Expression"),
            );
            if response.lost_focus() {
                match ScalarField::formula(source.clone()).and_then(|candidate| {
                    let value = candidate.evaluate(
                        MaterialCoordinates {
                            x: 0.0,
                            y: 0.0,
                            r: 0.0,
                            theta: 0.0,
                        },
                        parameters,
                    )?;
                    if value < minimum {
                        return Err(MaterialError::InvalidValue);
                    }
                    Ok(candidate)
                }) {
                    Ok(candidate) => {
                        *field = candidate;
                        errors.remove(&key);
                    }
                    Err(error) => {
                        errors.insert(key, error.to_string());
                    }
                }
            }
        }
    }
    if let Some(error) = errors.get(&key) {
        ui.colored_label(RED, error);
    }
}

fn edit_face_condition(ui: &mut egui::Ui, condition: &mut FaceBoundaryCondition) -> bool {
    let before = *condition;
    let mut kind = match condition {
        FaceBoundaryCondition::Reflecting => BoundaryKind::Reflecting,
        FaceBoundaryCondition::Impedance { .. } => BoundaryKind::FirstOrder,
        FaceBoundaryCondition::SecondOrderOutgoing => BoundaryKind::SecondOrder,
        FaceBoundaryCondition::ElectricWall => BoundaryKind::ElectricWall,
        FaceBoundaryCondition::MagneticWall => BoundaryKind::MagneticWall,
        FaceBoundaryCondition::Neumann { .. } => BoundaryKind::Neumann,
        FaceBoundaryCondition::Dirichlet { .. } => BoundaryKind::Dirichlet,
    };
    egui::ComboBox::from_id_salt("span-condition")
        .selected_text(condition.label())
        .show_ui(ui, |ui| boundary_kind_choices(ui, &mut kind));
    if kind != face_kind(before) {
        *condition = match kind {
            BoundaryKind::Reflecting => FaceBoundaryCondition::Reflecting,
            BoundaryKind::FirstOrder => FaceBoundaryCondition::Impedance { ratio: 1.0 },
            BoundaryKind::SecondOrder => FaceBoundaryCondition::SecondOrderOutgoing,
            BoundaryKind::ElectricWall => FaceBoundaryCondition::ElectricWall,
            BoundaryKind::MagneticWall => FaceBoundaryCondition::MagneticWall,
            BoundaryKind::Neumann => FaceBoundaryCondition::Neumann {
                signal: TimeSignal::harmonic(0.0, 1.0, 1.0, 0.0),
            },
            BoundaryKind::Dirichlet => FaceBoundaryCondition::Dirichlet {
                signal: TimeSignal::harmonic(0.0, 1.0, 1.0, 0.0),
            },
        };
    }
    match condition {
        FaceBoundaryCondition::Impedance { ratio } => {
            ui.add(
                egui::DragValue::new(ratio)
                    .speed(0.02)
                    .range(1.0e-6..=1.0e6)
                    .prefix("Ratio "),
            );
        }
        FaceBoundaryCondition::Neumann { signal } | FaceBoundaryCondition::Dirichlet { signal } => {
            edit_time_signal(ui, signal)
        }
        _ => {}
    }
    *condition != before
}

fn edit_outer_condition(ui: &mut egui::Ui, condition: &mut OuterBoundaryCondition) -> bool {
    let before = *condition;
    let mut kind = match condition {
        OuterBoundaryCondition::Reflecting => BoundaryKind::Reflecting,
        OuterBoundaryCondition::FirstOrderOutgoing => BoundaryKind::FirstOrder,
        OuterBoundaryCondition::SecondOrderOutgoing => BoundaryKind::SecondOrder,
        OuterBoundaryCondition::ElectricWall => BoundaryKind::ElectricWall,
        OuterBoundaryCondition::MagneticWall => BoundaryKind::MagneticWall,
        OuterBoundaryCondition::Neumann { .. } => BoundaryKind::Neumann,
        OuterBoundaryCondition::Dirichlet { .. } => BoundaryKind::Dirichlet,
    };
    egui::ComboBox::from_id_salt("outer-condition")
        .selected_text(condition.label())
        .show_ui(ui, |ui| boundary_kind_choices(ui, &mut kind));
    let old_kind = match before {
        OuterBoundaryCondition::Reflecting => BoundaryKind::Reflecting,
        OuterBoundaryCondition::FirstOrderOutgoing => BoundaryKind::FirstOrder,
        OuterBoundaryCondition::SecondOrderOutgoing => BoundaryKind::SecondOrder,
        OuterBoundaryCondition::ElectricWall => BoundaryKind::ElectricWall,
        OuterBoundaryCondition::MagneticWall => BoundaryKind::MagneticWall,
        OuterBoundaryCondition::Neumann { .. } => BoundaryKind::Neumann,
        OuterBoundaryCondition::Dirichlet { .. } => BoundaryKind::Dirichlet,
    };
    if kind != old_kind {
        *condition = match kind {
            BoundaryKind::Reflecting => OuterBoundaryCondition::Reflecting,
            BoundaryKind::FirstOrder => OuterBoundaryCondition::FirstOrderOutgoing,
            BoundaryKind::SecondOrder => OuterBoundaryCondition::SecondOrderOutgoing,
            BoundaryKind::ElectricWall => OuterBoundaryCondition::ElectricWall,
            BoundaryKind::MagneticWall => OuterBoundaryCondition::MagneticWall,
            BoundaryKind::Neumann => OuterBoundaryCondition::Neumann {
                signal: TimeSignal::harmonic(0.0, 1.0, 1.0, 0.0),
            },
            BoundaryKind::Dirichlet => OuterBoundaryCondition::Dirichlet {
                signal: TimeSignal::harmonic(0.0, 1.0, 1.0, 0.0),
            },
        };
    }
    match condition {
        OuterBoundaryCondition::Neumann { signal }
        | OuterBoundaryCondition::Dirichlet { signal } => edit_time_signal(ui, signal),
        _ => {}
    }
    *condition != before
}

fn face_kind(condition: FaceBoundaryCondition) -> BoundaryKind {
    match condition {
        FaceBoundaryCondition::Reflecting => BoundaryKind::Reflecting,
        FaceBoundaryCondition::Impedance { .. } => BoundaryKind::FirstOrder,
        FaceBoundaryCondition::SecondOrderOutgoing => BoundaryKind::SecondOrder,
        FaceBoundaryCondition::ElectricWall => BoundaryKind::ElectricWall,
        FaceBoundaryCondition::MagneticWall => BoundaryKind::MagneticWall,
        FaceBoundaryCondition::Neumann { .. } => BoundaryKind::Neumann,
        FaceBoundaryCondition::Dirichlet { .. } => BoundaryKind::Dirichlet,
    }
}

fn face_condition_color(condition: FaceBoundaryCondition) -> Color32 {
    match condition {
        FaceBoundaryCondition::Reflecting => Color32::from_rgb(184, 201, 211),
        FaceBoundaryCondition::Impedance { .. } => Color32::from_rgb(91, 220, 194),
        FaceBoundaryCondition::SecondOrderOutgoing => Color32::from_rgb(87, 174, 255),
        FaceBoundaryCondition::ElectricWall => Color32::from_rgb(255, 126, 141),
        FaceBoundaryCondition::MagneticWall => Color32::from_rgb(188, 143, 255),
        FaceBoundaryCondition::Neumann { .. } => Color32::from_rgb(248, 196, 112),
        FaceBoundaryCondition::Dirichlet { .. } => Color32::from_rgb(255, 139, 84),
    }
}

fn outer_condition_color(condition: OuterBoundaryCondition) -> Color32 {
    match condition {
        OuterBoundaryCondition::Reflecting => {
            face_condition_color(FaceBoundaryCondition::Reflecting)
        }
        OuterBoundaryCondition::FirstOrderOutgoing => {
            face_condition_color(FaceBoundaryCondition::Impedance { ratio: 1.0 })
        }
        OuterBoundaryCondition::SecondOrderOutgoing => {
            face_condition_color(FaceBoundaryCondition::SecondOrderOutgoing)
        }
        OuterBoundaryCondition::ElectricWall => {
            face_condition_color(FaceBoundaryCondition::ElectricWall)
        }
        OuterBoundaryCondition::MagneticWall => {
            face_condition_color(FaceBoundaryCondition::MagneticWall)
        }
        OuterBoundaryCondition::Neumann { signal } => {
            face_condition_color(FaceBoundaryCondition::Neumann { signal })
        }
        OuterBoundaryCondition::Dirichlet { signal } => {
            face_condition_color(FaceBoundaryCondition::Dirichlet { signal })
        }
    }
}

fn boundary_kind_choices(ui: &mut egui::Ui, kind: &mut BoundaryKind) {
    for (value, label) in [
        (BoundaryKind::Reflecting, "Reflecting"),
        (BoundaryKind::FirstOrder, "First-order outgoing"),
        (BoundaryKind::SecondOrder, "Second-order outgoing"),
        (BoundaryKind::ElectricWall, "Electric wall"),
        (BoundaryKind::MagneticWall, "Magnetic wall"),
        (BoundaryKind::Neumann, "Driven Neumann"),
        (BoundaryKind::Dirichlet, "Driven Dirichlet"),
    ] {
        ui.selectable_value(kind, value, label);
    }
}

fn edit_time_signal(ui: &mut egui::Ui, signal: &mut TimeSignal) {
    let (offset, amplitude, frequency, phase) = signal.harmonic_parameters_mut();
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(offset).speed(0.02).prefix("Offset "));
        ui.add(
            egui::DragValue::new(amplitude)
                .speed(0.02)
                .prefix("Amplitude "),
        );
    });
    ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(frequency)
                .speed(0.05)
                .range(0.0..=1.0e6)
                .prefix("Hz "),
        );
        ui.add(egui::DragValue::new(phase).speed(0.05).prefix("Phase "));
    });
}

fn draw_time_series(ui: &mut egui::Ui, values: &[(f64, f64)], color: Color32, height: f32) {
    let (response, painter) =
        ui.allocate_painter(egui::vec2(ui.available_width(), height), Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 4.0, Color32::from_rgb(10, 16, 22));
    if values.len() < 2 {
        return;
    }
    let t1 = values.last().unwrap().0;
    let t0 = (t1 - 10.0).max(values.first().unwrap().0);
    let visible = values.iter().filter(|(time, _)| *time >= t0);
    let max = visible
        .clone()
        .map(|(_, value)| value.abs())
        .fold(1.0e-12, f64::max);
    let points = visible
        .map(|(time, value)| {
            Pos2::new(
                egui::remap_clamp(
                    *time as f32,
                    t0 as f32..=t1.max(t0 + 1.0e-9) as f32,
                    rect.left()..=rect.right(),
                ),
                egui::remap_clamp(
                    *value as f32,
                    -max as f32..=max as f32,
                    rect.bottom()..=rect.top(),
                ),
            )
        })
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(1.5, color)));
}

fn draw_profile(ui: &mut egui::Ui, values: &[f32], color: Color32, height: f32) {
    let (response, painter) =
        ui.allocate_painter(egui::vec2(ui.available_width(), height), Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 4.0, Color32::from_rgb(10, 16, 22));
    if values.len() < 2 {
        return;
    }
    let max = values
        .iter()
        .map(|value| value.abs())
        .fold(1.0e-9, f32::max);
    let points = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            Pos2::new(
                egui::lerp(
                    rect.left()..=rect.right(),
                    index as f32 / (values.len() - 1) as f32,
                ),
                egui::remap_clamp(*value, -max..=max, rect.bottom()..=rect.top()),
            )
        })
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(1.5, color)));
}

fn draw_polar(ui: &mut egui::Ui, values: &[f32], size: f32) {
    let width = ui.available_width();
    let side = size.min(width);
    let left = ui.cursor().left() + (width - side) * 0.5;
    let rect = Rect::from_min_size(Pos2::new(left, ui.cursor().top()), egui::vec2(side, side));
    ui.allocate_rect(rect, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_rgb(10, 16, 22));
    let center = rect.center();
    let radius = side * 0.43;
    for fraction in [0.25, 0.5, 0.75, 1.0] {
        painter.circle_stroke(
            center,
            radius * fraction,
            Stroke::new(0.7, Color32::from_gray(60)),
        );
    }
    if values.len() < 3 {
        return;
    }
    let maximum = values.iter().copied().fold(1.0e-12, f32::max);
    let mut points = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let angle = std::f32::consts::TAU * index as f32 / values.len() as f32;
            let magnitude = (value.max(0.0) / maximum).sqrt();
            center + egui::vec2(angle.cos(), -angle.sin()) * radius * magnitude
        })
        .collect::<Vec<_>>();
    points.push(points[0]);
    painter.add(egui::Shape::line(points, Stroke::new(1.6, TEAL)));
}

fn draw_far_waterfall(ui: &mut egui::Ui, records: &VecDeque<FarFieldRecord>, height: f32) {
    let (response, painter) =
        ui.allocate_painter(egui::vec2(ui.available_width(), height), Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 4.0, Color32::from_rgb(10, 16, 22));
    let shown = records.len().min(96);
    if shown == 0 {
        return;
    }
    for (row, record) in records.iter().rev().take(shown).rev().enumerate() {
        let count = record.amplitude.len();
        if count == 0 {
            continue;
        }
        let top = egui::lerp(rect.top()..=rect.bottom(), row as f32 / shown as f32);
        let bottom = egui::lerp(rect.top()..=rect.bottom(), (row + 1) as f32 / shown as f32);
        let scale = record
            .amplitude
            .iter()
            .map(|value| value.abs())
            .fold(1.0e-9, f32::max);
        for (index, value) in record.amplitude.iter().enumerate() {
            let left = egui::lerp(rect.left()..=rect.right(), index as f32 / count as f32);
            let right = egui::lerp(
                rect.left()..=rect.right(),
                (index + 1) as f32 / count as f32,
            );
            painter.rect_filled(
                Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom)),
                0.0,
                field_color(*value / scale, 1.0, Color32::from_rgb(10, 16, 22)),
            );
        }
    }
}

fn screen_segment_distance(point: Pos2, start: Pos2, end: Pos2) -> f32 {
    let segment = end - start;
    let length_squared = segment.length_sq();
    if length_squared <= f32::EPSILON {
        return point.distance(start);
    }
    let fraction = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
    point.distance(start + fraction * segment)
}

fn closest_curve_parameter(
    sampled: &SampledTopologyGeometry,
    transform: ViewportTransform,
    point: Pos2,
    tolerance: f32,
) -> Option<(CurveId, f64)> {
    let p = ScreenPoint::new(point.x as f64, point.y as f64);
    let mut best: Option<(f64, CurveId, f64)> = None;
    for span in &sampled.spans {
        let Some(curve) = span.curve else { continue };
        for segment in span.samples.windows(2) {
            let a = transform.world_to_screen(segment[0].point);
            let b = transform.world_to_screen(segment[1].point);
            let ab = ScreenPoint::new(b.x - a.x, b.y - a.y);
            let ap = ScreenPoint::new(p.x - a.x, p.y - a.y);
            let length_squared = ab.x * ab.x + ab.y * ab.y;
            let fraction = if length_squared > 0.0 {
                ((ap.x * ab.x + ap.y * ab.y) / length_squared).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let closest = ScreenPoint::new(a.x + ab.x * fraction, a.y + ab.y * fraction);
            let distance = ((p.x - closest.x).powi(2) + (p.y - closest.y).powi(2)).sqrt();
            if distance <= tolerance as f64
                && best
                    .as_ref()
                    .is_none_or(|(best_distance, _, _)| distance < *best_distance)
            {
                best = Some((
                    distance,
                    curve,
                    segment[0].t + (segment[1].t - segment[0].t) * fraction,
                ));
            }
        }
    }
    best.map(|(_, curve, parameter)| (curve, parameter))
}
fn attachment_face(attachment: TopologyAttachment, compiled: &CompiledTopologyScene) -> FaceId {
    match attachment {
        TopologyAttachment::Junction { face, .. } => face,
        TopologyAttachment::Boundary(anchor) => {
            anchor.resolve(&compiled.topology).unwrap_or(EXTERIOR_FACE)
        }
    }
}
fn field_color(value: f32, gain: f32, under: Color32) -> Color32 {
    let value = (value * gain).tanh();
    let target = if value >= 0.0 {
        Color32::from_rgb(244, 105, 122)
    } else {
        Color32::from_rgb(63, 144, 239)
    };
    let amount = value.abs();
    let base = if under == Color32::TRANSPARENT {
        Color32::from_rgb(16, 23, 31)
    } else {
        under
    };
    Color32::from_rgb(
        egui::lerp(base.r() as f32..=target.r() as f32, amount) as u8,
        egui::lerp(base.g() as f32..=target.g() as f32, amount) as u8,
        egui::lerp(base.b() as f32..=target.b() as f32, amount) as u8,
    )
}

fn field_color_over_overlay(value: f32, gain: f32) -> Color32 {
    let value = if value.is_finite() {
        (value * gain).tanh()
    } else {
        0.0
    };
    let target = if value >= 0.0 {
        [244, 105, 122]
    } else {
        [63, 144, 239]
    };
    Color32::from_rgba_unmultiplied(
        target[0],
        target[1],
        target[2],
        (value.abs() * 220.0).round() as u8,
    )
}

fn material_property_color(fraction: f32, alpha: u8) -> Color32 {
    const STOPS: [(f32, [u8; 3]); 5] = [
        (0.0, [20, 34, 69]),
        (0.25, [42, 91, 132]),
        (0.5, [55, 168, 154]),
        (0.75, [184, 205, 104]),
        (1.0, [255, 211, 103]),
    ];
    let value = fraction.clamp(0.0, 1.0);
    let (left, right) = STOPS
        .windows(2)
        .find_map(|pair| (value <= pair[1].0).then_some((pair[0], pair[1])))
        .unwrap_or((STOPS[3], STOPS[4]));
    let t = ((value - left.0) / (right.0 - left.0)).clamp(0.0, 1.0);
    let channel = |index: usize| {
        (left.1[index] as f32 + t * (right.1[index] as f32 - left.1[index] as f32)).round() as u8
    };
    Color32::from_rgba_unmultiplied(channel(0), channel(1), channel(2), alpha)
}

fn overlay_property_color(property: MaterialProperty, fraction: f32, alpha: u8) -> Color32 {
    if property != MaterialProperty::VolumeSource {
        return material_property_color(fraction, alpha);
    }
    const STOPS: [(f32, [u8; 3]); 3] = [
        (0.0, [63, 144, 239]),
        (0.5, [24, 32, 39]),
        (1.0, [244, 105, 122]),
    ];
    let value = fraction.clamp(0.0, 1.0);
    let (left, right) = if value <= 0.5 {
        (STOPS[0], STOPS[1])
    } else {
        (STOPS[1], STOPS[2])
    };
    let t = ((value - left.0) / (right.0 - left.0)).clamp(0.0, 1.0);
    let channel = |index: usize| {
        (left.1[index] as f32 + t * (right.1[index] as f32 - left.1[index] as f32)).round() as u8
    };
    Color32::from_rgba_unmultiplied(channel(0), channel(1), channel(2), alpha)
}

fn amr_target_color(fraction: f32) -> Color32 {
    let fraction = fraction.clamp(0.0, 1.0);
    let fine = [71.0, 144.0, 232.0];
    let coarse = [246.0, 183.0, 92.0];
    Color32::from_rgba_unmultiplied(
        egui::lerp(fine[0]..=coarse[0], fraction) as u8,
        egui::lerp(fine[1]..=coarse[1], fraction) as u8,
        egui::lerp(fine[2]..=coarse[2], fraction) as u8,
        75,
    )
}

fn highest_forcing_frequency(scene: &TopologyScene, source: PointSource) -> f64 {
    let mut frequency = if source.enabled {
        source.signal.frequency_ceiling_hz()
    } else {
        0.0
    };
    for condition in scene.outer_boundaries.sides {
        if let Some(signal) = condition.signal() {
            frequency = frequency.max(signal.frequency_ceiling_hz());
        }
    }
    for curve in &scene.geometry.curves {
        for span in &curve.spans {
            if let SpanBehavior::Separated { left, right, .. } = span.behavior {
                for condition in [left, right] {
                    if let Some(signal) = condition.signal() {
                        frequency = frequency.max(signal.frequency_ceiling_hz());
                    }
                }
            }
        }
    }
    for source in &scene.volume_sources {
        if source.enabled {
            frequency = frequency.max(source.signal.frequency_ceiling_hz());
        }
    }
    frequency
}

fn vector_overlay_samples(
    scene: &TopologyScene,
    mesh: &TriMesh,
    operator: &QuadraticWaveOperator,
    display: &WaveDisplay,
    mode: VectorOverlay,
    spacing: f32,
    view: (Point2, f64, Rect),
) -> Vec<((i32, i32), Pos2, Point2)> {
    let (view_center, view_scale, viewport) = view;
    if mesh.triangles.len() != operator.element_nodes().len()
        || display.indicator_displacement.len() != operator.degrees_of_freedom()
        || display.indicator_velocity.len() != operator.degrees_of_freedom()
        || display.indicator_potential.len() != operator.degrees_of_freedom()
        || spacing <= 0.0
    {
        return vec![];
    }
    let mut bins = BTreeMap::<(i32, i32), (usize, Pos2, Point2, f32)>::new();
    for (element, triangle) in mesh.triangles.iter().enumerate() {
        let points = triangle.vertices.map(|index| mesh.vertices[index].point);
        let centroid = (points[0] + points[1] + points[2]) / 3.0;
        let screen = Pos2::new(
            viewport.center().x
                + ((centroid.x - view_center.x) * view_scale).clamp(-1.0e7, 1.0e7) as f32,
            viewport.center().y
                - ((centroid.y - view_center.y) * view_scale).clamp(-1.0e7, 1.0e7) as f32,
        );
        if !viewport.contains(screen) {
            continue;
        }
        let key = (
            ((screen.x - viewport.left()) / spacing).floor() as i32,
            ((screen.y - viewport.top()) / spacing).floor() as i32,
        );
        let cell_center = Pos2::new(
            viewport.left() + (key.0 as f32 + 0.5) * spacing,
            viewport.top() + (key.1 as f32 + 0.5) * spacing,
        );
        let distance = screen.distance_sq(cell_center);
        if bins.get(&key).is_none_or(|entry| distance < entry.3) {
            bins.insert(key, (element, screen, centroid, distance));
        }
    }
    bins.into_iter()
        .filter_map(|(key, (element, screen, centroid, _))| {
            let triangle = &mesh.triangles[element];
            let (primary, gradient) = operator.element_value_and_gradient(
                element,
                &display.indicator_displacement,
                [1.0 / 3.0; 3],
            )?;
            let (velocity, _) = operator.element_value_and_gradient(
                element,
                &display.indicator_velocity,
                [1.0 / 3.0; 3],
            )?;
            let (_, potential_gradient) = operator.element_value_and_gradient(
                element,
                &display.indicator_potential,
                [1.0 / 3.0; 3],
            )?;
            let region = scene.region(triangle.region)?;
            let material = scene.material(region.material)?;
            let raw = material.evaluate(region.frame, centroid).ok()?;
            let coefficients = scene
                .physics
                .directional_wave_coefficients(raw, region.frame);
            let potential_flux = coefficients.stiffness.apply(potential_gradient);
            let flux = coefficients.stiffness.apply(gradient);
            let vector = match (mode, scene.physics) {
                (
                    VectorOverlay::ComplementaryField,
                    PhysicsModel::Electromagnetic {
                        polarization: ElectromagneticPolarization::Tm,
                    },
                ) => Point2::new(-potential_flux.y, potential_flux.x),
                (
                    VectorOverlay::ComplementaryField,
                    PhysicsModel::Electromagnetic {
                        polarization: ElectromagneticPolarization::Te,
                    },
                ) => Point2::new(potential_flux.y, -potential_flux.x),
                (VectorOverlay::RelativeEnergyFlow, PhysicsModel::Electromagnetic { .. }) => {
                    potential_flux * -primary
                }
                (VectorOverlay::RelativeEnergyFlow, PhysicsModel::Mechanical) => flux * -velocity,
                _ => return None,
            };
            vector.finite().then_some((key, screen, vector))
        })
        .collect()
}

fn aligned_indicator_auxiliary(
    display: &WaveDisplay,
    operator: &QuadraticWaveOperator,
    time_step: f64,
    completed_steps: u64,
) -> Option<Vec<f64>> {
    let count = operator.degrees_of_freedom();
    if !time_step.is_finite()
        || time_step <= 0.0
        || display.current.len() != count
        || display.auxiliary.len() != count
        || display.indicator_displacement.len() != count
    {
        return None;
    }
    Some(
        (0..count)
            .map(|node| {
                if !operator.auxiliary_active()[node]
                    || operator.dirichlet_signals()[node].is_some()
                    || completed_steps == 0
                {
                    display.auxiliary[node] as f64
                } else {
                    display.auxiliary[node] as f64
                        - 0.5
                            * time_step
                            * (display.indicator_displacement[node] as f64
                                + display.current[node] as f64)
                }
            })
            .collect(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn frame(
    mut contexts: EguiContexts,
    mut state: ResMut<Playground>,
    time: Res<Time>,
    mut request: ResMut<WaveGpuRequest>,
    display: Res<WaveDisplay>,
    probe_display: Res<ProbeDisplay>,
    curve_display: Res<CurveProbeDisplay>,
    area_display: Res<AreaProbeDisplay>,
    far_display: Res<FarFieldDisplay>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut commands: Commands,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    if !state.ready {
        ctx.set_visuals(egui::Visuals::dark());
        ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.button_padding = egui::vec2(8.0, 4.0);
            style.visuals.selection.bg_fill = Color32::from_rgb(38, 94, 135);
        });
        state.ready = true;
        #[cfg(target_arch = "wasm32")]
        if let Some(element) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("startup"))
        {
            let _ = element.set_attribute("hidden", "");
        }
    }
    state.frame_ms = state.frame_ms * 0.95 + time.delta_secs() * 1000.0 * 0.05;
    state.update_files();
    state.update_recording();
    state.begin_capture_frame();
    if !state.startup_done {
        state.startup_done = true;
        if let Some(bytes) = crate::sharing::initial_fragment() {
            match bytes.and_then(|bytes| persistence::parse(&bytes)) {
                Ok(candidate) => {
                    state.load = Some(candidate);
                    state.file_busy = true;
                }
                Err(error) => state.message = error,
            }
        } else if let Ok(Some(bytes)) = crate::recovery::load() {
            if let Ok(candidate) = persistence::parse(&bytes) {
                state.load = Some(candidate);
                state.file_busy = true;
            }
        }
    }
    state.editor.validate_frame(12000);
    state.refresh_runtime(
        &mut request,
        &display,
        &mut assets,
        &mut commands,
        time.delta_secs_f64(),
    );
    state.refresh_amr(&request, &display);
    state.ingest_probes(&probe_display);
    state.ingest_spatial_probes(&curve_display, &area_display, &far_display);
    state.refresh_material_overlay();
    let mut root = egui::Ui::new(
        ctx.clone(),
        "root".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.viewport_rect()),
    );
    let logical_canvas = ctx.viewport_rect();
    let logical_viewport = state.show(&mut root, &display);
    if state.snapshot_state == SnapshotState::Armed {
        state.snapshot_state = SnapshotState::Capturing;
        let sender = state.sender.clone();
        commands.spawn(Screenshot::primary_window()).observe(
            move |captured: On<ScreenshotCaptured>| {
                let event = match crate::capture::encode_viewport_png(
                    captured.image.clone(),
                    logical_canvas,
                    logical_viewport,
                ) {
                    Ok(bytes) => FileEvent::SnapshotCaptured(bytes),
                    Err(error) => FileEvent::Error(error),
                };
                let _ = sender.send(event);
            },
        );
    }
    if state.recording_state == RecordingState::Preparing {
        let pixels_per_point = ctx.pixels_per_point() as f64;
        let physical_width = (logical_canvas.width() as f64 * pixels_per_point).round() as u32;
        let physical_height = (logical_canvas.height() as f64 * pixels_per_point).round() as u32;
        let result = crate::capture::pixel_crop(
            logical_canvas,
            logical_viewport,
            physical_width,
            physical_height,
        )
        .and_then(crate::capture::video_dimensions)
        .and_then(|(width, height)| {
            state.video_recorder.start(
                RecordingSpec {
                    width,
                    height,
                    fps: recording::VIDEO_FPS,
                },
                logical_canvas,
                logical_viewport,
            )
        });
        match result {
            Ok(()) => state.recording_state = RecordingState::Starting,
            Err(error) => {
                state.recording_state = RecordingState::Idle;
                state.notify(error);
            }
        }
    }
    if matches!(
        state.recording_state,
        RecordingState::Starting | RecordingState::Recording
    ) {
        state
            .video_recorder
            .update_viewport(logical_canvas, logical_viewport);
    }
    #[cfg(not(target_arch = "wasm32"))]
    if state.recording_state == RecordingState::Recording
        && let Some(started) = state.recording_started
    {
        let slot = (started.elapsed().as_secs_f64() * recording::VIDEO_FPS as f64).floor() as u64;
        if state.recording_last_requested_slot != Some(slot)
            && state
                .recording_readback_in_flight
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            && let Some(target) = state.video_recorder.native_frame_target()
        {
            state.recording_last_requested_slot = Some(slot);
            let in_flight = state.recording_readback_in_flight.clone();
            commands.spawn(Screenshot::primary_window()).observe(
                move |captured: On<ScreenshotCaptured>| {
                    target.submit(
                        captured.image.clone(),
                        logical_canvas,
                        logical_viewport,
                        slot,
                    );
                    in_flight.store(false, Ordering::Release);
                },
            );
        }
    }
    state.autosave();
    Ok(())
}
