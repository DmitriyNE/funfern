#![allow(clippy::collapsible_if)]

use crate::files::{self, FileEvent, SaveKind};
use crate::material_overlay::{
    MaterialOverlay, MaterialOverlayJob, MaterialOverlaySnapshot, MaterialProperty, OverlayKey,
    OverlayRange,
};
use crate::recording::{self, DestinationRequest, RecordingEvent, RecordingSpec, VideoRecorder};
use crate::wave_gpu::{
    AreaProbeDisplay, AreaProbeInput, AreaProbeRecord, CurveProbeDisplay, CurveProbeInput,
    CurveProbeRecord, FAR_FIELD_DIRECTIONS, FarFieldClock, FarFieldDisplay, FarFieldHandoff,
    FarFieldInput, FarFieldRecord, MAX_STEPS_PER_FRAME, PointProbeRecord, ProbeDisplay,
    PulseSettings, WaveDisplay, WaveGpuRequest, WaveTransfer,
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
    ClosedCurvePurpose, JoinRecord, OpenCurvePurpose, TopologyAcceptance, TopologyAttachment,
    TopologyBoundaryProbeTarget, TopologyCurveRemoval, TopologyDocument, TopologyEditor,
    TopologyProbeDefinition, TopologyProbeTarget, TopologySpanRemoval,
};
use funfern_app::topology_persistence::{self as persistence, TopologyLoadCandidate};
use funfern_app::topology_runtime::{
    PreparedTopology, TopologyPreparationTiming, TopologyProbeCompilation, TopologyProbeStencil,
    TopologyRuntime, TopologyToken,
};
use funfern_app::topology_viewport::{
    AttachmentHit, RigidTransform, SampledTopologyGeometry, ScreenPoint, TopologyHandle,
    TopologyHit, TopologySelection, TopologySpanTarget, ViewportTransform, plan_axis_scale,
    plan_handle_drag, plan_rigid_transform, selected_span_controls, span_context, weld_hit,
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
const FRAME_HISTORY: usize = 120;
const GIZMO_PADDING: f32 = 18.0;

/// What the Materials panel lists, and what a viewport click selects with it.
/// Faces include holes, which own no region and so cannot appear in the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SubdomainListing {
    Faces,
    Regions,
}

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

/// Which part of the outer rectangle a domain resize has hold of.
#[derive(Clone, Copy, Debug, PartialEq)]
enum DomainDrag {
    Side { side: OuterSide, start: DomainRect },
    Corner { index: usize, start: DomainRect },
}

#[derive(Clone, Debug)]
enum DragGesture {
    Handle {
        handle: TopologyHandle,
    },
    /// The outer rectangle being resized by one of its sides or corners.
    Domain {
        drag: DomainDrag,
    },
    /// A loose end of an open curve on the move. `snap` is the target it would
    /// weld onto if released now, for the preview ring only.
    Endpoint {
        curve: CurveId,
        node: usize,
        snap: Option<AttachmentHit>,
    },
    Spans {
        start: Point2,
        pivot: Point2,
        custom_pivot: bool,
        gizmo_before: Option<(BTreeSet<TopologySpanTarget>, Point2)>,
        geometry: TopologyGeometry,
    },
    Rotate {
        pivot: Point2,
        start_angle: f64,
        geometry: TopologyGeometry,
    },
    Scale {
        axis: GizmoScaleAxis,
        pivot: Point2,
        start_distance: f64,
        geometry: TopologyGeometry,
    },
    Pivot {
        previous: Option<(BTreeSet<TopologySpanTarget>, Point2)>,
        offset: Point2,
    },
    Marquee {
        start: Pos2,
        current: Pos2,
        base: BTreeSet<TopologySpanTarget>,
        operation: MarqueeOperation,
    },
    Source,
    MaterialFrame {
        region: RegionId,
        start: MaterialFrame,
        hit: MaterialFrameGizmoHit,
        grab: f64,
    },
    Probe {
        hit: ProbeHit,
        grab: Point2,
        original: TopologyProbeTarget,
    },
}

/// A deletion waiting on the survivor choice, because it merges two subdomains
/// carrying different materials.
#[derive(Clone, Debug)]
enum PendingRemoval {
    Curve(CurveId, Vec<RegionId>),
    Spans(BTreeSet<CurveSpanId>, Vec<RegionId>),
}

impl PendingRemoval {
    fn choices(&self) -> &[RegionId] {
        match self {
            Self::Curve(_, choices) | Self::Spans(_, choices) => choices,
        }
    }
}

/// The two grips of a region's material/source frame: its origin and the ring
/// that turns its local axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MaterialFrameGizmoHit {
    Origin,
    Rotate,
}

const MATERIAL_FRAME_RADIUS: f32 = 42.0;

/// What a pointer landed on within a probe. Endpoint and radius grips take
/// priority over the body so a small probe stays reshapeable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProbeHit {
    Point(ProbeId),
    SegmentEndpoint(ProbeId, bool),
    SegmentBody(ProbeId),
    Boundary(ProbeId),
    AreaDiskBody(ProbeId),
    AreaDiskRadius(ProbeId),
    AreaRegion(ProbeId),
}

impl ProbeHit {
    const fn id(self) -> ProbeId {
        match self {
            Self::Point(id)
            | Self::SegmentEndpoint(id, _)
            | Self::SegmentBody(id)
            | Self::Boundary(id)
            | Self::AreaDiskBody(id)
            | Self::AreaDiskRadius(id)
            | Self::AreaRegion(id) => id,
        }
    }

    const fn draggable(self) -> bool {
        !matches!(self, Self::Boundary(_) | Self::AreaRegion(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransformGizmoHit {
    Pivot,
    Rotate,
    Scale(GizmoScaleAxis),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GizmoScaleAxis {
    Uniform,
    X,
    Y,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MarqueeOperation {
    Replace,
    Add,
    Subtract,
}

impl MarqueeOperation {
    const fn label(self) -> &'static str {
        match self {
            Self::Replace => "Replace",
            Self::Add => "Add",
            Self::Subtract => "Subtract",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MarqueeContainment {
    Enclosed,
    Crossing,
}

impl MarqueeContainment {
    const fn from_drag(start: Pos2, current: Pos2) -> Self {
        if current.x >= start.x {
            Self::Enclosed
        } else {
            Self::Crossing
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Enclosed => "Enclosed",
            Self::Crossing => "Crossing",
        }
    }
}

#[derive(Clone, Debug)]
struct ProbeTrace {
    samples: VecDeque<PointProbeRecord>,
    last_time: f64,
}

/// A quantity a line or boundary probe samples along its path. The GPU records
/// all four every frame; the readout chooses which to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineProbeQuantity {
    Field,
    Transverse,
    Flux,
    Energy,
}

impl LineProbeQuantity {
    const ALL: [Self; 4] = [Self::Field, Self::Transverse, Self::Flux, Self::Energy];

    const fn label_for(self, physics: PhysicsModel) -> &'static str {
        match self {
            Self::Field => primary_field_label(physics),
            Self::Transverse => transverse_field_magnitude_label(physics),
            Self::Flux => match physics {
                PhysicsModel::Mechanical => "Normal energy flux",
                PhysicsModel::Electromagnetic { .. } => "Normal Poynting flux",
            },
            Self::Energy => "Energy density",
        }
    }

    const fn color(self) -> Color32 {
        match self {
            Self::Field => SELECT,
            Self::Transverse => Color32::from_rgb(188, 139, 255),
            Self::Flux => TEAL,
            Self::Energy => GOLD,
        }
    }

    const fn offset(self) -> usize {
        match self {
            Self::Field => 0,
            Self::Transverse => 3,
            Self::Flux => 6,
            Self::Energy => 9,
        }
    }

    const fn applies(self, physics: PhysicsModel) -> bool {
        !matches!(self, Self::Transverse) || matches!(physics, PhysicsModel::Electromagnetic { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineProbeRepresentation {
    Arclength,
    Waterfall,
    Integral,
}

impl LineProbeRepresentation {
    const ALL: [Self; 3] = [Self::Arclength, Self::Waterfall, Self::Integral];

    const fn label(self) -> &'static str {
        match self {
            Self::Arclength => "vs s",
            Self::Waterfall => "Waterfall",
            Self::Integral => "∫ vs t",
        }
    }

    const fn offset(self) -> usize {
        match self {
            Self::Arclength => 0,
            Self::Waterfall => 1,
            Self::Integral => 2,
        }
    }
}

/// Per-readout presentation: which traces are drawn and the shared time window
/// every trace in that window pans and zooms together.
#[derive(Clone, Debug)]
struct ProbeViewState {
    live: bool,
    end_time: f64,
    span: f64,
    field: bool,
    secondary_field: bool,
    poynting: bool,
    energy: bool,
    area_mean_field: bool,
    area_rms_field: bool,
    area_rms_transverse: bool,
    area_mean_energy: bool,
    area_total_energy: bool,
    line_plots: [bool; 12],
    far_waterfall: bool,
    far_polar: bool,
    far_power: bool,
    waterfall_gain: f32,
}

impl ProbeViewState {
    fn new(span: f64) -> Self {
        Self {
            live: true,
            end_time: 0.0,
            span: span.min(2.0),
            field: true,
            secondary_field: false,
            poynting: false,
            energy: true,
            area_mean_field: false,
            area_rms_field: true,
            area_rms_transverse: false,
            area_mean_energy: false,
            area_total_energy: true,
            // Field versus arclength and its waterfall, plus the two integrals.
            line_plots: [
                true, true, false, // primary component
                false, false, false, // transverse magnitude
                false, false, true, // normal flux
                false, false, true, // energy density
            ],
            far_waterfall: true,
            far_polar: true,
            far_power: true,
            waterfall_gain: 1.0,
        }
    }
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

/// One completed geometry-to-GPU transaction, split into the three waits the
/// user can actually act on: CPU preparation, draining the solver's requested
/// steps, and the GPU upload itself.
#[derive(Clone, Debug)]
struct HandoffRecord {
    prepare_ms: f64,
    drain_ms: f64,
    upload_ms: f64,
    timing: TopologyPreparationTiming,
    action: TopologyMeshUpdateAction,
    operator_reused: bool,
    adapted: bool,
    transferred: bool,
    /// Target nodes the transfer copied exactly, when the field crossed over.
    exact_nodes: usize,
    fresh: bool,
    degrees_of_freedom: usize,
    triangles: usize,
    carve: Option<CarveReport>,
    repair_fallback: Option<String>,
}

struct Uploading {
    token: TopologyToken,
    generation: u64,
    fresh: bool,
    time_offset: f64,
    degrees_of_freedom: usize,
}

/// What the probe buffers on the GPU were last built for. The topology names
/// the stencils, but the wave buffers own the probes: every path that replaces
/// them drops each probe's buffers and readback, and a reset or a rolled-back
/// transfer takes that path without a commit. Keying the upload to the GPU
/// generation as well is what brings the probes back afterwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProbeUpload {
    token: TopologyToken,
    generation: u64,
    revision: u64,
    curve_revision: u64,
    area_revision: u64,
    far_field_revision: u64,
}

fn probes_need_upload(upload: Option<ProbeUpload>, token: TopologyToken, generation: u64) -> bool {
    upload.is_none_or(|upload| upload.token != token || upload.generation != generation)
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
    touch_active: bool,
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
    gizmo_pivot: Option<(BTreeSet<TopologySpanTarget>, Point2)>,
    pending_removal: Option<PendingRemoval>,
    material_selection: MaterialId,
    region_selection: RegionId,
    /// Whether a widget held keyboard focus when the previous frame ended. egui
    /// clears focus on Escape before any app code runs, so the live predicate is
    /// already false on the one frame where it matters.
    keyboard_focus_previous: bool,
    /// Whether the Materials panel lists compiled faces or material regions, and
    /// which of the two a viewport click picks.
    subdomain_listing: SubdomainListing,
    /// Index into `face_assignments` while the panel lists faces.
    face_selection: usize,
    material_edit: Option<Material>,
    material_formula_edits: BTreeMap<(u64, u8), String>,
    material_formula_errors: BTreeMap<(u64, u8), String>,
    /// Whether the formula reference is showing. It is a window rather than a
    /// menu so it stays readable while a formula is being typed.
    formula_help_open: bool,
    material_color_edit: Option<(MaterialId, [u8; 3])>,
    new_separator_material: MaterialId,
    mesh_edge: f64,
    /// The slider produces a value per frame; the rebuild waits for release.
    mesh_edge_dragging: bool,
    /// The Remesh button: rebuild at the current resolution even though
    /// nothing changed, which also leaves an adapted mesh.
    remesh_requested: bool,
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
    probe_views: BTreeMap<ProbeId, ProbeViewState>,
    probe_status: BTreeMap<ProbeId, String>,
    probe_metrics: BTreeMap<ProbeId, (f64, bool)>,
    probe_anchors: BTreeMap<ProbeId, Point2>,
    probe_metadata_token: Option<(TopologyToken, u64)>,
    probe_name_edit: Option<(ProbeId, String)>,
    probe_history_seconds: f64,
    far_field_view: ProbeViewState,
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
    probe_upload: Option<ProbeUpload>,
    probe_clock_restarted: bool,
    /// Simulated time the far-field ring started recording from, or `None` when
    /// no recorder is running.
    far_field_recording_from: Option<f64>,
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
    /// Revision of the active mesh the running adaptation started from. The
    /// job is dropped as soon as that mesh is no longer the active one.
    amr_adaptation_source: Option<u64>,
    amr_adaptation_state: Option<MeshAdaptationState>,
    amr_pending_state: Option<MeshAdaptationState>,
    amr_report: Option<MeshAdaptationReport>,
    gpu_status: &'static str,
    gpu_dispatches: u64,
    step_backlog: u64,
    diagnostics_open: bool,
    diagnostics_warned: bool,
    frame_history: VecDeque<f32>,
    handoff_requested: Option<Instant>,
    handoff_ready: Option<Instant>,
    handoff_upload: Option<Instant>,
    last_handoff: Option<HandoffRecord>,
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
            pending_removal: None,
            material_selection: DEFAULT_MATERIAL,
            region_selection: BACKGROUND_REGION,
            keyboard_focus_previous: false,
            subdomain_listing: SubdomainListing::Regions,
            face_selection: 0,
            material_edit: None,
            material_formula_edits: BTreeMap::new(),
            material_formula_errors: BTreeMap::new(),
            formula_help_open: false,
            material_color_edit: None,
            new_separator_material: DEFAULT_MATERIAL,
            mesh_edge: 0.08,
            mesh_edge_dragging: false,
            remesh_requested: false,
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
            recording_readback_in_flight: Arc::new(AtomicBool::new(false)),
            startup_done: false,
            autosave_observed: document,
            autosave_due: None,
            probe_upload: None,
            probe_clock_restarted: true,
            far_field_recording_from: None,
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
            amr_adaptation_source: None,
            amr_adaptation_state: None,
            amr_pending_state: None,
            amr_report: None,
            gpu_status: "loading",
            gpu_dispatches: 0,
            step_backlog: 0,
            diagnostics_open: false,
            diagnostics_warned: false,
            frame_history: VecDeque::with_capacity(FRAME_HISTORY),
            handoff_requested: None,
            handoff_ready: None,
            handoff_upload: None,
            last_handoff: None,
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
        match &self.drag {
            Some(DragGesture::Pivot { previous, .. }) => {
                self.gizmo_pivot = previous.clone();
            }
            Some(DragGesture::Spans { gizmo_before, .. }) => {
                self.gizmo_pivot = gizmo_before.clone();
            }
            Some(DragGesture::Marquee { base, .. }) => {
                self.selection = Self::selection_from_spans(base.clone());
            }
            _ => {}
        }
        if self.editor.editing() {
            self.editor.cancel();
        }
        self.drag = None;
        self.draw = None;
        self.pending_removal = None;
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
        self.pending_removal = None;
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
        if self.draw_open && !self.capturing() {
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
        // On a narrow layout the inspector floats over the viewport instead of
        // docking beside it, which would put a panel inside the capture crop.
        if self.capturing() && root.available_width() < 700.0 {
            return;
        }
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
        if let TopologyHandle::Control { curve, control } = handle {
            // Ask the command itself whether it would succeed, so the button is
            // live exactly when the deletion is.
            let refusal = self.editor.control_removal_error(curve, control);
            let response = ui
                .add_enabled(refusal.is_none(), egui::Button::new("Delete control"))
                .on_disabled_hover_text(refusal.unwrap_or_default());
            if response.clicked() {
                match self.editor.remove_control(curve, control) {
                    Ok(()) => {
                        self.selection = TopologySelection::None;
                        self.invalidate_samples();
                    }
                    Err(error) => self.notify(error),
                }
            }
        }
        if let TopologyHandle::Junction(vertex) = handle {
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
    /// Drops a staged survivor question when undo, a reload, or another edit
    /// moved the geometry out from under it, rather than act on something else.
    fn prune_stale_pending_removal(&mut self) {
        if self.pending_removal.as_ref().is_some_and(|pending| {
            let geometry = &self.editor.document.model.draft.geometry;
            match pending {
                PendingRemoval::Curve(curve, _) => geometry.curve(*curve).is_none(),
                PendingRemoval::Spans(spans, _) => !spans.iter().all(|span| {
                    geometry
                        .curves
                        .iter()
                        .any(|curve| curve.spans.iter().any(|candidate| candidate.id == *span))
                }),
            }
        }) {
            self.pending_removal = None;
        }
    }
    /// The region owning the draft face under a world point, from the editor's
    /// own compile so the answer does not wait on the GPU runtime.
    fn draft_region_at(&self, point: Point2) -> Option<RegionId> {
        let compiled = self
            .editor
            .compiled_draft
            .as_ref()
            .unwrap_or(&self.editor.compiled_accepted);
        let face = compiled.topology.face_at(point)?;
        compiled
            .assignments
            .iter()
            .find(|assignment| assignment.face == face)
            .and_then(|assignment| assignment.region)
    }
    fn material_name(&self, region: RegionId) -> String {
        let draft = &self.editor.document.model.draft;
        draft
            .region(region)
            .and_then(|region| draft.material(region.material))
            .map_or_else(|| format!("Region {}", region.0), |m| m.name.clone())
    }
    /// Highlights every subdomain a staged deletion may keep: the mesh of each
    /// candidate filled gold, its boundary stroked, and its material named at
    /// the face centre. The one under the cursor reads stronger.
    fn draw_removal_candidates(&self, painter: &egui::Painter, r: Rect) {
        if self.capturing() {
            return;
        }
        let Some(pending) = &self.pending_removal else {
            return;
        };
        let candidates = pending.choices().iter().copied().collect::<BTreeSet<_>>();
        let hovered = painter
            .ctx()
            .pointer_hover_pos()
            .filter(|pos| r.contains(*pos))
            .and_then(|pos| self.draft_region_at(self.world(pos, r)))
            .filter(|region| candidates.contains(region));
        let fill = |region: RegionId| {
            Color32::from_rgba_unmultiplied(
                248,
                196,
                112,
                if hovered == Some(region) { 150 } else { 85 },
            )
        };
        if let Some(active) = self.runtime.active() {
            // One mesh, so the highlight does not print the triangulation on
            // the subdomain the user is being asked to look at.
            let mesh = &active.mesh;
            let mut highlight = egui::Mesh::default();
            for triangle in &mesh.triangles {
                if !candidates.contains(&triangle.region) {
                    continue;
                }
                let color = fill(triangle.region);
                let first = highlight.vertices.len() as u32;
                for index in triangle.vertices {
                    highlight.colored_vertex(self.screen(mesh.vertices[index].point, r), color);
                }
                highlight.add_triangle(first, first + 1, first + 2);
            }
            if !highlight.is_empty() {
                painter.add(egui::Shape::mesh(highlight));
            }
        }
        let compiled = self
            .editor
            .compiled_draft
            .as_ref()
            .unwrap_or(&self.editor.compiled_accepted);
        let region_of = |face: FaceId| {
            compiled
                .assignments
                .iter()
                .find(|assignment| assignment.face == face)
                .and_then(|assignment| assignment.region)
                .filter(|region| candidates.contains(region))
        };
        for edge in &compiled.topology.edges {
            if region_of(edge.left).is_some() || region_of(edge.right).is_some() {
                painter.line_segment(
                    [
                        self.screen(edge.points[0], r),
                        self.screen(edge.points[1], r),
                    ],
                    Stroke::new(2.4, GOLD),
                );
            }
        }
        for face in &compiled.topology.faces {
            let Some(region) = region_of(face.id) else {
                continue;
            };
            let Some(centroid) = face.centroid() else {
                continue;
            };
            let point = self.screen(centroid, r);
            let label = format!("Keep {}", self.material_name(region));
            let galley =
                painter.layout_no_wrap(label, egui::FontId::proportional(13.0), Color32::WHITE);
            let rect = egui::Rect::from_center_size(point, galley.size() + egui::vec2(14.0, 8.0));
            painter.rect_filled(rect, 4.0, Color32::from_rgba_unmultiplied(8, 13, 18, 210));
            painter.rect_stroke(rect, 4.0, Stroke::new(1.0, GOLD), egui::StrokeKind::Outside);
            painter.galley(rect.min + egui::vec2(7.0, 4.0), galley, Color32::WHITE);
        }
    }
    /// The question a staged deletion asks, anchored over the viewport so it is
    /// visible whatever panels are open.
    fn removal_prompt(&mut self, ctx: &egui::Context, viewport: Rect) {
        if self.pending_removal.is_none() || self.capturing() {
            return;
        }
        let mut cancel = false;
        egui::Window::new("Merge subdomains")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(Pos2::new(viewport.center().x, viewport.top() + 14.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Click the subdomain that keeps its material").strong(),
                    );
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel {
            self.pending_removal = None;
        }
    }
    /// Answers the staged deletion with the candidate under a click; a click
    /// anywhere else leaves the question open.
    fn pick_removal_survivor(&mut self, point: Point2) {
        let Some(pending) = self.pending_removal.clone() else {
            return;
        };
        let Some(region) = self
            .draft_region_at(point)
            .filter(|region| pending.choices().contains(region))
        else {
            return;
        };
        let outcome = match &pending {
            PendingRemoval::Curve(curve, _) => self
                .editor
                .remove_curve(*curve, Some(region))
                .map(|removal| self.report_curve_removal(&removal)),
            PendingRemoval::Spans(spans, _) => self
                .editor
                .remove_spans(spans, Some(region))
                .map(|removal| self.report_span_removal(&removal)),
        };
        match outcome {
            Ok(()) => {
                self.pending_removal = None;
                self.selection = TopologySelection::None;
                self.invalidate_samples();
            }
            Err(error) => self.message = error,
        }
    }
    /// The closed-curve Subdomain/Hole switch; acts on the complete curve
    /// selection.
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
        // A span with an excluded face on both sides, two holes say, bounds
        // nothing the simulation solves, so its boundary settings do nothing.
        if let Some(compiled) = self.editor.compiled_draft.as_ref() {
            let inactive = curve_spans
                .iter()
                .filter(|span| {
                    span_context(compiled, **span)
                        .is_some_and(|context| !context.left.active && !context.right.active)
                })
                .count();
            if inactive > 0 {
                ui.colored_label(
                    GOLD,
                    if inactive == curve_spans.len() && inactive == 1 {
                        "Inactive".to_owned()
                    } else if inactive == curve_spans.len() {
                        "All inactive".to_owned()
                    } else {
                        format!("{inactive} of {} inactive", curve_spans.len())
                    },
                )
                .on_hover_text("Excluded on both sides, so nothing here reaches the simulation");
            }
        }
        if !curve_spans.is_empty() {
            // The boxes report which of the two states the selection is in
            // rather than offering an action. A mixed selection ticks neither,
            // and unticking the state a span is already in would leave it in no
            // state at all, so only a tick applies anything.
            let state =
                span_behavior_state(&self.editor.document.model.draft.geometry, &curve_spans);
            ui.horizontal(|ui| {
                for (value, label, hint, behavior) in [
                    (
                        SpanBehaviorState::Transmit,
                        "Transmit",
                        "The field crosses these spans",
                        SpanBehavior::Transmitting,
                    ),
                    (
                        SpanBehaviorState::Boundary,
                        "Boundary",
                        "Each side of these spans carries its own condition",
                        SpanBehavior::REFLECTING,
                    ),
                ] {
                    let mut checked = state == Some(value);
                    if ui
                        .checkbox(&mut checked, label)
                        .on_hover_text(hint)
                        .changed()
                        && checked
                        && let Err(error) = self.editor.set_span_behavior(&curve_spans, behavior)
                    {
                        self.notify(error);
                    }
                }
                if state.is_none() {
                    ui.weak("Mixed").on_hover_text(
                        "These spans are not all in the same state; tick one to put them there",
                    );
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
                if let Some((curve, breakpoint, continuity, attached)) =
                    self.selected_end_continuity(span)
                {
                    ui.label(format!("End knot · C{continuity}"));
                    ui.horizontal_wrapped(|ui| {
                        for (target, label) in
                            [(2, "C2 smooth"), (1, "C1 tangent"), (0, "C0 corner")]
                        {
                            let response = ui.add_enabled(
                                target == 0 || !attached,
                                egui::Button::selectable(continuity == target, label),
                            );
                            if response.clicked() && continuity != target {
                                match self.editor.set_curve_continuity(curve, breakpoint, target) {
                                    Ok(displacement) => {
                                        self.invalidate_samples();
                                        if displacement > 0.0 {
                                            self.notify(format!(
                                                "Curve smoothed; maximum displacement {displacement:.3e}"
                                            ));
                                        }
                                    }
                                    Err(error) => self.notify(error),
                                }
                            }
                        }
                    });
                }
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button("Straighten spans")
                    .on_hover_text("Make each selected span straight between its own endpoints")
                    .clicked()
                {
                    match self.editor.straighten_spans(&curve_spans) {
                        Ok(()) => self.invalidate_samples(),
                        Err(error) => self.notify(error),
                    }
                }
                let contiguous = self.editor.selection_is_contiguous(&curve_spans);
                if ui
                    .add_enabled(contiguous, egui::Button::new("Straighten selection"))
                    .on_hover_text(if contiguous {
                        "Lay the whole selected run on one straight chord"
                    } else {
                        "Select one contiguous run of spans on each curve"
                    })
                    .clicked()
                {
                    match self.editor.straighten_span_sections(&curve_spans) {
                        Ok(()) => self.invalidate_samples(),
                        Err(error) => self.notify(error),
                    }
                }
                if ui
                    .button("Isolate at C0")
                    .on_hover_text("Add exact corners at the selected section boundaries")
                    .clicked()
                {
                    match self.editor.isolate_span_boundaries(&curve_spans) {
                        Ok(()) => self.invalidate_samples(),
                        Err(error) => self.notify(error),
                    }
                }
            });
            let pivot = self
                .gizmo_pivot_for(&spans, &curve_spans)
                .unwrap_or_default();
            let transform = RigidTransform {
                pivot,
                translation: Point2::default(),
                rotation_radians: 0.0,
                scale: 1.0,
            };
            // A selection that cannot move rigidly simply gets no gizmo. What is
            // holding it is visible in the scene, and a marquee fixes it.
            let transform_allowed = plan_rigid_transform(
                &self.editor.document.model.draft.geometry,
                &spans,
                transform,
            )
            .is_ok();
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
                        pivot,
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
                            if self
                                .gizmo_pivot
                                .as_ref()
                                .is_some_and(|(selection, _)| selection == &spans)
                            {
                                self.gizmo_pivot =
                                    Some((spans.clone(), pivot + self.transform_translation));
                            }
                            self.transform_translation = Point2::default();
                            self.transform_rotation_degrees = 0.0;
                            self.transform_scale = 1.0;
                            self.invalidate_samples();
                        }
                        Err(error) => self.notify(error),
                    }
                }
            }
            // One button for every selection, running the same path as the
            // Delete key: whole curves, partial runs, and the survivor picker
            // when a deletion merges two subdomains.
            let whole = self.selected_complete_curves(&curve_spans).len();
            if ui
                .button("Delete")
                .on_hover_text(match whole {
                    0 => "Delete the selected spans and leave the rest as baffles",
                    1 => "Delete the selected curve",
                    _ => "Delete the selected curves",
                })
                .clicked()
            {
                self.delete_selection();
                self.invalidate_samples();
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
    fn gizmo_pivot_for(
        &self,
        selected: &BTreeSet<TopologySpanTarget>,
        spans: &BTreeSet<CurveSpanId>,
    ) -> Option<Point2> {
        self.gizmo_pivot
            .as_ref()
            .filter(|(selection, _)| selection == selected)
            .map(|(_, point)| *point)
            .or_else(|| self.selection_pivot(spans))
    }
    fn snap_point(point: Point2) -> Point2 {
        const STEP: f64 = 0.05;
        Point2::new(
            (point.x / STEP).round() * STEP,
            (point.y / STEP).round() * STEP,
        )
    }
    fn scale_drag_distance(axis: GizmoScaleAxis, relative: Point2) -> f64 {
        match axis {
            GizmoScaleAxis::Uniform => relative.norm(),
            GizmoScaleAxis::X => relative.x.abs(),
            GizmoScaleAxis::Y => relative.y.abs(),
        }
    }
    fn marquee_operation(modifiers: egui::Modifiers) -> MarqueeOperation {
        if modifiers.alt {
            MarqueeOperation::Subtract
        } else if modifiers.shift {
            MarqueeOperation::Add
        } else {
            MarqueeOperation::Replace
        }
    }
    fn marquee_result(
        base: &BTreeSet<TopologySpanTarget>,
        hits: BTreeSet<TopologySpanTarget>,
        operation: MarqueeOperation,
    ) -> BTreeSet<TopologySpanTarget> {
        match operation {
            MarqueeOperation::Replace => hits,
            MarqueeOperation::Add => base.union(&hits).copied().collect(),
            MarqueeOperation::Subtract => base.difference(&hits).copied().collect(),
        }
    }
    fn selection_from_spans(spans: BTreeSet<TopologySpanTarget>) -> TopologySelection {
        if spans.is_empty() {
            TopologySelection::None
        } else {
            TopologySelection::Spans(spans)
        }
    }
    fn drag_starts_inside_span_selection(selection: &TopologySelection, hit: TopologyHit) -> bool {
        matches!(
            hit,
            TopologyHit::Span { target, .. }
                if selection
                    .spans()
                    .is_some_and(|spans| spans.contains(&target))
        )
    }
    fn selected_end_continuity(&self, span: CurveSpanId) -> Option<(CurveId, usize, u8, bool)> {
        let curve = self
            .editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.spans.iter().any(|candidate| candidate.id == span))?;
        let index = curve
            .spans
            .iter()
            .position(|candidate| candidate.id == span)?;
        let breakpoint = if curve.spline.is_open() {
            index + 1
        } else {
            (index + 1) % curve.spans.len()
        };
        let continuity = match &curve.spline {
            CurveSpline::Closed(spline) => spline.continuity(breakpoint),
            CurveSpline::Open(spline) => spline.continuity(breakpoint),
        }?;
        let attached = curve
            .nodes
            .get(breakpoint)
            .is_some_and(|node| node.vertex.is_some());
        Some((curve.id, breakpoint, continuity, attached))
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
        ui.checkbox(&mut p.field, "Field");
        ui.add(egui::Slider::new(&mut p.field_gain, 0.25..=12.0).text("Field intensity"));
        let physics = self.editor.document.model.draft.physics;
        p.vector_overlay = p.vector_overlay.resolved(physics);
        egui::ComboBox::from_id_salt("vector-overlay")
            .selected_text(p.vector_overlay.label(physics))
            .show_ui(ui, |ui| {
                for mode in VectorOverlay::choices(physics) {
                    ui.selectable_value(&mut p.vector_overlay, *mode, mode.label(physics));
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
                for overlay in [
                    MaterialOverlay::Regions,
                    MaterialOverlay::Subdomains,
                    MaterialOverlay::AdaptationTarget,
                ] {
                    ui.selectable_value(&mut p.material_overlay, overlay, overlay.label());
                }
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
        if p.material_overlay == MaterialOverlay::AdaptationTarget {
            ui.small("Blue where the estimate wants the finest elements, orange the coarsest");
            // The overlay has three ways of being empty and none of them used to
            // say anything, which is how it came to look broken.
            if !self.amr_enabled {
                ui.colored_label(GOLD, "Adaptation is off in Simulation");
            } else {
                match &self.amr_indicator_result {
                    None => {
                        ui.colored_label(GOLD, format!("No estimate yet · {}", self.amr_status))
                    }
                    Some(result) => {
                        let triangles = self
                            .runtime
                            .active()
                            .map(|active| active.mesh.triangles.len());
                        if triangles.is_some_and(|count| count != result.element_targets.len()) {
                            ui.colored_label(GOLD, "The estimate is behind the current mesh")
                        } else {
                            ui.small(format!(
                                "Targets {:.3}–{:.3}",
                                result.report.minimum_target, result.report.maximum_target
                            ))
                        }
                    }
                };
            }
        }
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
        ui.checkbox(&mut p.line_probes, "Lines");
        ui.checkbox(&mut p.boundary_probes, "Boundaries");
        ui.checkbox(&mut p.area_probes, "Areas");
        ui.checkbox(&mut p.far_field_contour, "Far field");
        ui.checkbox(&mut p.probe_labels, "Probe names");
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
        ui.separator();
        ui.label("Mesh resolution");
        const PRESETS: [(f64, &str); 3] = [(0.16, "Coarse"), (0.08, "Medium"), (0.04, "Fine")];
        let preset_name = |edge: f64| {
            PRESETS
                .iter()
                .find(|(value, _)| (edge - value).abs() < 1.0e-9)
                .map_or("Custom", |(_, name)| name)
        };
        egui::ComboBox::from_id_salt("mesh_resolution")
            .width(ui.available_width())
            .selected_text(format!(
                "{} · target edge {:.3}",
                preset_name(self.mesh_edge),
                self.mesh_edge
            ))
            .show_ui(ui, |ui| {
                for (value, name) in PRESETS {
                    ui.selectable_value(
                        &mut self.mesh_edge,
                        value,
                        format!("{name} · h ≤ {value:.2}"),
                    );
                }
            });
        let slider = ui.add(
            egui::Slider::new(&mut self.mesh_edge, 0.02..=0.25)
                .logarithmic(true)
                .text("Target edge"),
        );
        self.mesh_edge_dragging = slider.dragged();
        ui.horizontal(|ui| {
            if ui
                .button("Remesh")
                .on_hover_text(
                    "Rebuild the mesh at this resolution, leaving any adapted mesh behind",
                )
                .clicked()
            {
                self.remesh_requested = true;
            }
            if let Some(active) = self.runtime.active()
                && (active.meshing.target_edge_length - self.mesh_edge).abs() > 1.0e-9
            {
                ui.small(format!(
                    "Active {:.3} · requested {:.3}",
                    active.meshing.target_edge_length, self.mesh_edge
                ));
            }
        });
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
        // Framed, because arming a mode is an action. A bare selectable label
        // reads as a caption until it is switched on.
        if ui
            .add(egui::Button::new("Place pulse").selected(self.pulse_mode))
            .on_hover_text("Click in the scene to drop a pulse; click here again to stop")
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
        ui.horizontal(|ui| {
            ui.label("Subdomain assignment");
            for (mode, label, hint) in [
                (
                    SubdomainListing::Faces,
                    "Faces",
                    "Every compiled subdomain, holes included; a click in the scene picks one",
                ),
                (
                    SubdomainListing::Regions,
                    "Regions",
                    "Only the material regions; a click in the scene picks one",
                ),
            ] {
                if ui
                    .selectable_label(self.subdomain_listing == mode, label)
                    .on_hover_text(hint)
                    .clicked()
                {
                    self.subdomain_listing = mode;
                }
            }
        });
        let materials = self.editor.document.model.draft.materials.clone();
        if self.subdomain_listing == SubdomainListing::Faces {
            self.face_listing(ui, &materials);
        } else {
            self.region_listing(ui, &materials);
        }
        self.region_detail(ui);
    }

    /// One row per assigned face, so a hole is editable in the same place a
    /// subdomain is: it is simply the row whose material is Hole.
    fn face_listing(&mut self, ui: &mut egui::Ui, materials: &[Material]) {
        let assignments = self.editor.document.model.draft.face_assignments.clone();
        if self.face_selection >= assignments.len() {
            self.face_selection = 0;
        }
        for (index, assignment) in assignments.iter().enumerate() {
            ui.horizontal(|ui| {
                let (swatch, _) =
                    ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                ui.painter().rect_filled(
                    swatch,
                    2.0,
                    match assignment.region {
                        Some(region) => {
                            subdomain_color(&self.editor.document.model.draft, region, 1.0)
                        }
                        None => Color32::from_gray(70),
                    },
                );
                let name = match assignment.region {
                    Some(region) if region == BACKGROUND_REGION => "Background".to_owned(),
                    Some(region) => self.material_name(region),
                    None => "Hole".to_owned(),
                };
                if ui
                    .selectable_label(self.face_selection == index, name)
                    .on_hover_text("Select this subdomain and outline it in the scene")
                    .clicked()
                {
                    self.face_selection = index;
                    if let Some(region) = assignment.region {
                        self.region_selection = region;
                    }
                }
                let mut chosen = assignment.region.and_then(|region| {
                    self.editor
                        .document
                        .model
                        .draft
                        .region(region)
                        .map(|region| region.material)
                });
                egui::ComboBox::from_id_salt(("face", index))
                    .selected_text(match chosen {
                        Some(material) => materials
                            .iter()
                            .find(|item| item.id == material)
                            .map_or("Missing", |item| item.name.as_str()),
                        None => "Hole",
                    })
                    .show_ui(ui, |ui| {
                        for item in materials.iter() {
                            ui.selectable_value(&mut chosen, Some(item.id), &item.name);
                        }
                        ui.selectable_value(&mut chosen, None, "Hole")
                            .on_hover_text("Remove this subdomain and wall its boundary");
                    });
                let current = assignment.region.and_then(|region| {
                    self.editor
                        .document
                        .model
                        .draft
                        .region(region)
                        .map(|region| region.material)
                });
                if chosen != current
                    && let Err(error) = self.editor.set_face_disposition(index, chosen)
                {
                    self.notify(error);
                }
            });
        }
    }

    fn region_listing(&mut self, ui: &mut egui::Ui, materials: &[Material]) {
        let regions = self.editor.document.model.draft.regions.clone();
        for region in regions {
            ui.horizontal(|ui| {
                let (swatch, _) =
                    ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                ui.painter().rect_filled(
                    swatch,
                    2.0,
                    subdomain_color(&self.editor.document.model.draft, region.id, 1.0),
                );

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
                        for item in materials {
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
    }

    /// The selected region's source and frame, plus the material library.
    fn region_detail(&mut self, ui: &mut egui::Ui) {
        let materials = self.editor.document.model.draft.materials.clone();
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
            let mut source_changed = false;
            ui.horizontal(|ui| {
                source_changed = ui.checkbox(&mut source.enabled, "Volume source").changed();
                // Only beside a visible profile editor: with the source off
                // there is no formula on screen to explain.
                if source.enabled {
                    self.formula_help_toggle(ui);
                }
            });
            // The editors follow the checkbox exactly. Turning the source off
            // keeps its profile and signal in the document, so turning it back
            // on restores what was there.
            if source.enabled {
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
            }
            // Committed outside the block so unchecking is recorded rather than
            // springing back on the next frame.
            if source_changed
                && let Err(error) = self.editor.set_volume_source(region.id, Some(source))
            {
                self.notify(error);
            }

            let uses_frame = self
                .editor
                .document
                .model
                .draft
                .material(region.material)
                .is_some_and(Material::uses_frame)
                || existing
                    .as_ref()
                    .is_some_and(|source| source.enabled && source.varying());
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
            self.formula_help_toggle(ui);
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
            ui.horizontal(|ui| {
                // `color_edit_button_srgb` reports a change on every frame of a
                // drag inside its popup, so stage the value and commit it once
                // the pointer is released.
                let mut color = self
                    .material_color_edit
                    .filter(|(id, _)| *id == material.id)
                    .map_or(material.color, |(_, color)| color);
                let response = ui
                    .color_edit_button_srgb(&mut color)
                    .on_hover_text("Colour used by the Materials overlay");
                if response.changed() {
                    self.material_color_edit = Some((material.id, color));
                }
                if let Some((id, staged)) = self.material_color_edit
                    && id == material.id
                    && !ui.ctx().egui_is_using_pointer()
                {
                    self.material_color_edit = None;
                    if staged != material.color {
                        let mut updated = material.clone();
                        updated.color = staged;
                        if let Err(error) = self.editor.update_material(updated) {
                            self.notify(error);
                        }
                    }
                }
                if ui
                    .selectable_label(self.material_selection == material.id, &material.name)
                    .clicked()
                {
                    self.material_selection = material.id;
                    self.material_edit = None;
                    self.material_formula_edits.clear();
                    self.material_formula_errors.clear();
                }
                if material.id == DEFAULT_MATERIAL {
                    ui.small("ambient");
                }
            });
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
            for (mode, label, hint) in [
                (
                    ProbePlacement::Point,
                    "Point",
                    "Click in the scene to place a point probe",
                ),
                (
                    ProbePlacement::Segment { start: None },
                    "Line",
                    "Click the line's start, then its end",
                ),
                (
                    ProbePlacement::Disk { center: None },
                    "Disk",
                    "Click the disk's centre, then a point on its rim",
                ),
                (
                    ProbePlacement::Region,
                    "Region",
                    "Click a subdomain to probe the whole of it",
                ),
            ] {
                let selected = self.probe_mode.is_some_and(|active| {
                    std::mem::discriminant(&active) == std::mem::discriminant(&mode)
                });
                if ui
                    .add(egui::Button::new(format!("+ {label}")).selected(selected))
                    .on_hover_text(hint)
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
            .on_hover_text("Create a boundary probe from one span selection on one curve")
            .clicked()
            && let Some(target) = boundary_target
        {
            match self.editor.create_probe(
                format!("Boundary {}", self.editor.document.model.probes.len() + 1),
                [248, 196, 112],
                TopologyProbeTarget::Boundary(target),
            ) {
                Ok(id) => {
                    self.selected_probe = Some(id);
                    self.probe_windows.insert(id);
                    self.notify("Boundary probe added");
                }
                Err(error) => self.notify(error),
            }
        }
        ui.add(
            egui::Slider::new(&mut self.probe_history_seconds, 2.0..=60.0)
                .logarithmic(true)
                .text("History (sim s)"),
        )
        .on_hover_text("Longest time window a readout can show");
        ui.separator();
        let probes = self.editor.document.model.probes.clone();
        for mut probe in probes {
            ui.horizontal(|ui| {
                let open = self.probe_windows.contains(&probe.id);
                if ui
                    .selectable_label(self.selected_probe == Some(probe.id), &probe.name)
                    .clicked()
                {
                    self.selected_probe = Some(probe.id);
                    self.selection = TopologySelection::None;
                }
                if ui
                    .add(egui::Button::new("Plot").selected(open))
                    .on_hover_text("Show or hide this probe's readout window")
                    .clicked()
                {
                    if open {
                        self.probe_windows.remove(&probe.id);
                    } else {
                        self.probe_windows.insert(probe.id);
                    }
                }
                if ui
                    .checkbox(&mut probe.enabled, "Live")
                    .on_hover_text("Record samples from the solver")
                    .changed()
                    && let Err(error) = self.editor.update_probe(probe.clone())
                {
                    self.notify(error);
                }
                if ui.small_button("×").on_hover_text("Delete").clicked() {
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
            if let Some(status) = self.probe_status.get(&probe.id) {
                ui.small(egui::RichText::new(status).color(GOLD));
            }
        }
        if let Some(id) = self.selected_probe {
            self.selected_probe_editor(ui, id);
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
        if let Some(recorded) = self.far_field_recording() {
            ui.small(
                egui::RichText::new(format!(
                    "Recording the delay window · {:.0}%",
                    recorded * 100.0
                ))
                .color(GOLD),
            )
            .on_hover_text(
                "Every observation angle reads the contour at its own retarded time, \
                 so the recorder reports nothing until it holds a whole window. \
                 A remesh keeps what it has; moving the contour starts it again.",
            );
        }
    }
    /// Name, color, and target settings for the selected probe. The name is
    /// committed when the field loses focus so every keystroke is not a
    /// separate document revision.
    fn selected_probe_editor(&mut self, ui: &mut egui::Ui, id: ProbeId) {
        let Some(mut probe) = self
            .editor
            .document
            .model
            .probes
            .iter()
            .find(|probe| probe.id == id)
            .cloned()
        else {
            self.selected_probe = None;
            self.probe_name_edit = None;
            return;
        };
        ui.separator();
        ui.label(match probe.target {
            TopologyProbeTarget::Point(_) => "Selected point probe",
            TopologyProbeTarget::Segment { .. } => "Selected line probe",
            TopologyProbeTarget::Boundary(_) => "Selected boundary probe",
            TopologyProbeTarget::AreaDisk { .. } => "Selected disk probe",
            TopologyProbeTarget::AreaRegion(_) => "Selected region probe",
        });
        if !matches!(self.probe_name_edit.as_ref(), Some((candidate, _)) if *candidate == id) {
            self.probe_name_edit = Some((id, probe.name.clone()));
        }
        let mut commit_name = None;
        if let Some((_, name)) = self.probe_name_edit.as_mut() {
            let response = ui.add(egui::TextEdit::singleline(name).hint_text("Probe name"));
            if (response.lost_focus()
                || response
                    .ctx
                    .input(|input| input.key_pressed(egui::Key::Enter)))
                && !name.trim().is_empty()
                && name.len() <= 64
            {
                commit_name = Some(name.trim().to_owned());
            }
        }
        if let Some(name) = commit_name
            && name != probe.name
        {
            probe.name = name;
            if let Err(error) = self.editor.update_probe(probe.clone()) {
                self.notify(error);
            }
        }
        ui.horizontal(|ui| {
            ui.label("Color");
            if ui.color_edit_button_srgb(&mut probe.color).changed()
                && let Err(error) = self.editor.update_probe(probe.clone())
            {
                self.notify(error);
            }
        });
        let mut changed = false;
        match &mut probe.target {
            TopologyProbeTarget::Segment { start, end, preset } => {
                changed |= sampling_preset_picker(ui, id, preset);
                if ui
                    .button("Swap ends")
                    .on_hover_text("Reverse the arclength axis and the normal flux sign")
                    .clicked()
                {
                    std::mem::swap(start, end);
                    changed = true;
                }
            }
            TopologyProbeTarget::Boundary(target) => {
                changed |= sampling_preset_picker(ui, id, &mut target.preset);
                let mut side = target.side;
                ui.horizontal(|ui| {
                    ui.label("Trace side");
                    for (value, label) in [
                        (CurveTraceSide::Left, "Left"),
                        (CurveTraceSide::Right, "Right"),
                    ] {
                        if ui.selectable_label(side == value, label).clicked() {
                            side = value;
                        }
                    }
                });
                if side != target.side {
                    target.side = side;
                    changed = true;
                }
                let mut reversed = target.reversed;
                if ui
                    .checkbox(&mut reversed, "Reverse direction")
                    .on_hover_text("Sample the path against increasing curve parameter")
                    .changed()
                {
                    target.reversed = reversed;
                    changed = true;
                }
                ui.small(format!("{} spans", target.spans.len()));
            }
            TopologyProbeTarget::AreaDisk { radius, .. } => {
                changed |= ui
                    .add(
                        egui::DragValue::new(radius)
                            .speed(0.005)
                            .range(0.001..=10.0)
                            .prefix("Radius "),
                    )
                    .changed();
            }
            TopologyProbeTarget::Point(_) | TopologyProbeTarget::AreaRegion(_) => {}
        }
        if changed && let Err(error) = self.editor.update_probe(probe) {
            self.notify(error);
        }
        if let Some((length, closed)) = self.probe_metrics.get(&id).copied()
            && length > 0.0
        {
            ui.small(format!(
                "Path length {length:.4}{}",
                if closed { " · closed" } else { "" }
            ));
        }
        if ui.button("Clear recorded samples").clicked() {
            self.clear_probe_trace(id);
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
        self.draw_weld_targets(&painter, viewport);
        self.draw_domain_handles(&painter, viewport);
        self.draw_markers(&painter, viewport);
        self.draw_material_frame(&painter, viewport);
        self.draw_transform_gizmo(&painter, viewport);
        self.prune_stale_pending_removal();
        self.draw_removal_candidates(&painter, viewport);
        let ctx = ui.ctx().clone();
        self.removal_prompt(&ctx, viewport);
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
        // One mesh rather than a polygon per triangle, for the reason the
        // categorical overlay gives below: per-polygon outlines would imprint
        // the mesh on the wash whether or not the user asked to see it.
        if presentation.material_overlay == MaterialOverlay::AdaptationTarget
            && let Some(result) = &self.amr_indicator_result
            && result.element_targets.len() == mesh.triangles.len()
        {
            let span = (self.amr_maximum_edge - self.amr_minimum_edge).max(f64::MIN_POSITIVE);
            let alpha = (presentation.material_overlay_opacity * 210.0).round() as u8;
            let mut targets = egui::Mesh::default();
            targets.reserve_vertices(mesh.triangles.len() * 3);
            targets.reserve_triangles(mesh.triangles.len());
            for (triangle, target) in mesh.triangles.iter().zip(&result.element_targets) {
                let fraction = ((*target - self.amr_minimum_edge) / span).clamp(0.0, 1.0) as f32;
                let color = amr_target_color(fraction, alpha);
                let first = targets.vertices.len() as u32;
                for index in triangle.vertices {
                    targets.colored_vertex(self.screen(mesh.vertices[index].point, r), color);
                }
                targets.add_triangle(first, first + 1, first + 2);
            }
            if !targets.is_empty() {
                painter.add(egui::Shape::mesh(targets));
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
        // One mesh rather than one polygon per triangle. A polygon carries its
        // own antialiased outline, and the outlines of neighbours leave a seam
        // along every shared edge, which imprints the mesh on a categorical
        // overlay whether or not the user asked to see it. Vertices are
        // duplicated per triangle so each keeps its own flat colour and the
        // boundary between two regions stays a step rather than a gradient.
        if matches!(
            presentation.material_overlay,
            MaterialOverlay::Regions | MaterialOverlay::Subdomains
        ) {
            let mut fills = egui::Mesh::default();
            fills.reserve_vertices(mesh.triangles.len() * 3);
            fills.reserve_triangles(mesh.triangles.len());
            for triangle in &mesh.triangles {
                let base = match presentation.material_overlay {
                    MaterialOverlay::Regions => active
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
                        .unwrap_or(Color32::TRANSPARENT),
                    _ => subdomain_color(
                        &self.editor.document.model.draft,
                        triangle.region,
                        presentation.material_overlay_opacity,
                    ),
                };
                if base == Color32::TRANSPARENT {
                    continue;
                }
                let first = fills.vertices.len() as u32;
                for index in triangle.vertices {
                    fills.colored_vertex(self.screen(mesh.vertices[index].point, r), base);
                }
                fills.add_triangle(first, first + 1, first + 2);
            }
            if !fills.is_empty() {
                painter.add(egui::Shape::mesh(fills));
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
        // The field covers the whole domain and is opaque with no material
        // overlay under it, so the mesh has to be drawn over the field rather
        // than under it, and brightly enough to read against one.
        if presentation.mesh {
            for triangle in &mesh.triangles {
                let points = triangle
                    .vertices
                    .map(|index| self.screen(mesh.vertices[index].point, r));
                painter.add(egui::Shape::closed_line(
                    points.to_vec(),
                    Stroke::new(0.7, Color32::from_rgba_unmultiplied(160, 180, 195, 110)),
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
        let mode = presentation
            .vector_overlay
            .resolved(active.bundle.authored.physics);
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
        let mode = settings
            .vector_overlay
            .resolved(self.editor.document.model.draft.physics);
        if self.vector_overlay_mode != mode {
            self.vector_overlay_average.clear();
            self.vector_overlay_step = u64::MAX;
            self.vector_overlay_peak_reference = 0.0;
            self.vector_overlay_mode = mode;
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
        // The boundary-law strokes are a diagnostic layer: draw them first and
        // push them clear of a selected span so the selection always reads above.
        if interactive && self.editor.document.presentation.boundary_conditions {
            for span in &sampled.spans {
                let Some((left, right)) = self.span_condition_colors(span.target) else {
                    continue;
                };
                let offset = if self.span_selected(span.target) {
                    width * 0.5 + 4.0
                } else {
                    2.5
                };
                for segment in span.samples.windows(2) {
                    let a = self.screen(segment[0].point, r);
                    let b = self.screen(segment[1].point, r);
                    let tangent = b - a;
                    if tangent.length_sq() <= f32::EPSILON {
                        continue;
                    }
                    let normal = side_offset(tangent, CurveTraceSide::Left) * offset;
                    painter.line_segment([a + normal, b + normal], Stroke::new(1.4, left));
                    painter.line_segment([a - normal, b - normal], Stroke::new(1.4, right));
                }
            }
        }
        for span in &sampled.spans {
            let selected = interactive && self.span_selected(span.target);
            let points = span
                .samples
                .iter()
                .map(|sample| self.screen(sample.point, r))
                .collect::<Vec<_>>();
            if points.len() < 2 {
                continue;
            }
            if selected {
                // A dark halo keeps the blue readable over the law strokes and
                // over a bright field.
                painter.add(egui::Shape::line(
                    points.clone(),
                    Stroke::new(width + 4.5, Color32::from_rgba_unmultiplied(8, 13, 18, 190)),
                ));
            }
            painter.add(egui::Shape::line(
                points,
                Stroke::new(
                    if selected { width + 2.5 } else { width },
                    if selected { SELECT } else { color },
                ),
            ));
        }
        // The side the Boundary inspector is editing. A span's two traces are
        // geometrically coincident, so without a band in the scene the Left and
        // Right buttons name something the scene never shows. The arrow gives
        // the start-to-end direction those names are measured from.
        if interactive {
            // Clear of the condition strokes when they are on, and tight against
            // the span when they are not.
            let offset = width * 0.5
                + if self.editor.document.presentation.boundary_conditions {
                    7.5
                } else {
                    4.0
                };
            for span in &sampled.spans {
                if !matches!(span.target, TopologySpanTarget::Curve(_))
                    || !self.span_selected(span.target)
                {
                    continue;
                }
                let Some(middle) = span
                    .samples
                    .first()
                    .zip(span.samples.last())
                    .map(|(first, last)| 0.5 * (first.t + last.t))
                else {
                    continue;
                };
                let mut arrow: Option<(f64, Pos2, Pos2)> = None;
                for segment in span.samples.windows(2) {
                    let a = self.screen(segment[0].point, r);
                    let b = self.screen(segment[1].point, r);
                    let normal = side_offset(b - a, self.selected_side);
                    if normal == egui::Vec2::ZERO {
                        continue;
                    }
                    let normal = normal * offset;
                    painter
                        .line_segment([a + normal, b + normal], Stroke::new(3.0, Color32::WHITE));
                    let score = (0.5 * (segment[0].t + segment[1].t) - middle).abs();
                    if arrow.is_none_or(|(best, _, _)| score < best) {
                        arrow = Some((score, a, b));
                    }
                }
                if let Some((_, start, end)) = arrow {
                    let tangent = end - start;
                    let direction = tangent / tangent.length();
                    let normal = egui::vec2(-direction.y, direction.x);
                    let center = start + 0.5 * tangent;
                    painter.line_segment(
                        [center - direction * 7.0, center + direction * 7.0],
                        Stroke::new(1.5, GOLD),
                    );
                    let tip = center + direction * 7.0;
                    for barb in [normal, -normal] {
                        painter.line_segment(
                            [tip, tip - direction * 5.0 + barb * 3.0],
                            Stroke::new(1.5, GOLD),
                        );
                    }
                }
            }
        }
        if interactive && self.editor.document.presentation.handles {
            // A control belongs to a selection only through a selected span it
            // shapes; the rest of a long curve stays quiet.
            let owned_controls = self
                .selection
                .spans()
                .filter(|_| !self.capturing())
                .map(|spans| {
                    selected_span_controls(&self.editor.document.model.draft.geometry, spans)
                })
                .unwrap_or_default();
            for handle in &sampled.handles {
                let active = !self.capturing()
                    && matches!(self.selection, TopologySelection::Handle(value) if value == handle.handle);
                let owned = matches!(
                    handle.handle,
                    TopologyHandle::Control { curve, control } if owned_controls.contains(&(curve, control))
                );
                let point = self.screen(handle.point, r);
                let junction = matches!(handle.handle, TopologyHandle::Junction(_));
                let radius = match (junction, active) {
                    (true, true) => 7.0,
                    (true, false) => 5.5,
                    (false, true) => 6.0,
                    (false, false) => 4.0,
                };
                let fill = if active || junction {
                    GOLD
                } else {
                    Color32::from_rgb(23, 34, 44)
                };
                let ring = if active || owned {
                    SELECT
                } else if junction {
                    Color32::WHITE
                } else {
                    Color32::from_rgb(106, 133, 150)
                };
                painter.circle_filled(point, radius, fill);
                painter.circle_stroke(point, radius, Stroke::new(1.5, ring));
            }
        }
    }
    /// True while a PNG snapshot or a video frame is being taken. The capture
    /// crops to the viewport, so panels and the status bar are outside it by
    /// construction; what has to go is the chrome drawn inside the crop and the
    /// windows that float over it. `browser-checks.md` sets the rule: the field,
    /// the active View overlays, probes and the logo stay, while readouts,
    /// selection emphasis, gizmos, marquees and tool prompts do not.
    fn capturing(&self) -> bool {
        matches!(
            self.snapshot_state,
            SnapshotState::Armed | SnapshotState::Capturing
        ) || matches!(
            self.recording_state,
            RecordingState::Preparing | RecordingState::Starting | RecordingState::Recording
        )
    }

    fn span_selected(&self, target: TopologySpanTarget) -> bool {
        !self.capturing()
            && matches!(&self.selection, TopologySelection::Spans(spans) if spans.contains(&target))
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
    /// Probe markers and their names. Sizes and strokes follow the pre-topology
    /// viewport so every probe keeps a grabbable badge and a readable label.
    fn draw_probes(&self, painter: &egui::Painter, r: Rect) {
        for probe in &self.editor.document.model.probes {
            if !self.probe_visible(&probe.target) {
                continue;
            }
            let selected = !self.capturing() && self.selected_probe == Some(probe.id);
            let color = self.probe_color(probe);
            match &probe.target {
                TopologyProbeTarget::Point(position) => {
                    let center = self.screen(*position, r);
                    painter.circle_filled(center, if selected { 6.0 } else { 4.5 }, color);
                    painter.circle_stroke(
                        center,
                        if selected { 9.0 } else { 7.0 },
                        Stroke::new(if selected { 2.0 } else { 1.3 }, Color32::WHITE),
                    );
                }
                TopologyProbeTarget::Segment { start, end, .. } => {
                    let a = self.screen(*start, r);
                    let b = self.screen(*end, r);
                    painter
                        .line_segment([a, b], Stroke::new(if selected { 3.0 } else { 2.0 }, color));
                    let midpoint = a + (b - a) * 0.5;
                    let direction = b - a;
                    let length = direction.length().max(1.0);
                    let normal = egui::vec2(direction.y, -direction.x) / length;
                    painter.arrow(midpoint, normal * 18.0, Stroke::new(1.5, color));
                    for endpoint in [a, b] {
                        painter.circle_filled(endpoint, if selected { 5.0 } else { 4.0 }, color);
                        painter.circle_stroke(
                            endpoint,
                            if selected { 7.0 } else { 6.0 },
                            Stroke::new(1.5, Color32::WHITE),
                        );
                    }
                }
                TopologyProbeTarget::Boundary(target) => {
                    let path = self.boundary_probe_polyline(target);
                    for pair in path.windows(2) {
                        painter.line_segment(
                            [self.screen(pair[0], r), self.screen(pair[1], r)],
                            Stroke::new(if selected { 4.0 } else { 2.5 }, color),
                        );
                    }
                    if let Some(badge) = Self::polyline_midpoint(&path) {
                        let badge = self.screen(badge, r);
                        painter.circle_filled(badge, if selected { 7.0 } else { 5.5 }, color);
                        painter.circle_stroke(
                            badge,
                            if selected { 9.0 } else { 7.5 },
                            Stroke::new(1.5, Color32::WHITE),
                        );
                    }
                }
                TopologyProbeTarget::AreaDisk { center, radius } => {
                    let center = self.screen(*center, r);
                    let radius = (radius * self.scale) as f32;
                    painter.circle_filled(
                        center,
                        radius,
                        Color32::from_rgba_unmultiplied(
                            color.r(),
                            color.g(),
                            color.b(),
                            if selected { 32 } else { 18 },
                        ),
                    );
                    painter.circle_stroke(
                        center,
                        radius,
                        Stroke::new(if selected { 2.5 } else { 1.5 }, color),
                    );
                    painter.circle_filled(center, if selected { 5.0 } else { 3.5 }, color);
                    let handle = center + egui::vec2(radius, 0.0);
                    painter.circle_filled(handle, if selected { 5.0 } else { 4.0 }, color);
                    painter.circle_stroke(
                        handle,
                        if selected { 7.0 } else { 6.0 },
                        Stroke::new(1.5, Color32::WHITE),
                    );
                }
                TopologyProbeTarget::AreaRegion(region) => {
                    if let Some(sampled) = &self.sampled {
                        for span in &sampled.spans {
                            if !self.span_bounds_region(span.target, *region) {
                                continue;
                            }
                            for pair in span.samples.windows(2) {
                                painter.line_segment(
                                    [self.screen(pair[0].point, r), self.screen(pair[1].point, r)],
                                    Stroke::new(if selected { 4.0 } else { 2.5 }, color),
                                );
                            }
                        }
                    }
                    if let Some(anchor) = self.probe_anchors.get(&probe.id).copied() {
                        let center = self.screen(anchor, r);
                        painter.circle_filled(center, if selected { 9.0 } else { 7.0 }, color);
                        painter.circle_stroke(
                            center,
                            if selected { 11.0 } else { 9.0 },
                            Stroke::new(1.5, Color32::WHITE),
                        );
                        painter.text(
                            center,
                            egui::Align2::CENTER_CENTER,
                            "A",
                            egui::FontId::monospace(9.0),
                            Color32::WHITE,
                        );
                    }
                }
            }
            if !self.editor.document.presentation.probe_labels {
                continue;
            }
            if let Some(badge) = self.probe_badge(probe) {
                let origin = self.screen(badge, r) + egui::vec2(10.0, -10.0);
                let label = painter.layout_no_wrap(
                    probe.name.clone(),
                    egui::FontId::monospace(10.0),
                    color,
                );
                let rect = Rect::from_min_size(
                    Pos2::new(origin.x, origin.y - label.size().y),
                    label.size(),
                );
                painter.rect_filled(
                    rect.expand(2.0),
                    2.0,
                    Color32::from_rgba_unmultiplied(8, 13, 18, 170),
                );
                painter.galley(rect.min, label, color);
            }
        }
    }
    /// Whether a compiled span has the given region on either side, used to
    /// outline the face a region probe integrates over.
    /// The compiled face the Materials panel has selected, with its region, or
    /// `None` when that assignment no longer resolves.
    fn selected_face(&self) -> Option<(FaceId, Option<RegionId>)> {
        let compiled = self
            .editor
            .compiled_draft
            .as_ref()
            .unwrap_or(&self.editor.compiled_accepted);
        let assignment = self
            .editor
            .document
            .model
            .draft
            .face_assignments
            .get(self.face_selection)?;
        let face = assignment.anchor.resolve(&compiled.topology).ok()?;
        Some((face, assignment.region))
    }

    /// Which assignment owns the face under a point, by document index, so a
    /// viewport click can select a hole as readily as a subdomain.
    fn draft_face_assignment_at(&self, point: Point2) -> Option<usize> {
        let compiled = self
            .editor
            .compiled_draft
            .as_ref()
            .unwrap_or(&self.editor.compiled_accepted);
        let face = compiled.topology.face_at(point)?;
        self.editor
            .document
            .model
            .draft
            .face_assignments
            .iter()
            .position(|assignment| assignment.anchor.resolve(&compiled.topology) == Ok(face))
    }

    fn span_bounds_face(&self, target: TopologySpanTarget, face: FaceId) -> bool {
        let compiled = self
            .editor
            .compiled_draft
            .as_ref()
            .unwrap_or(&self.editor.compiled_accepted);
        compiled.topology.edges.iter().any(|edge| {
            let matches_target = match target {
                TopologySpanTarget::Outer(side) => edge.source == CompiledEdgeSource::Outer(side),
                TopologySpanTarget::Curve(span) => edge.source == CompiledEdgeSource::Curve(span),
            };
            matches_target && (edge.left == face || edge.right == face)
        })
    }

    fn span_bounds_region(&self, target: TopologySpanTarget, region: RegionId) -> bool {
        let Some(active) = self.runtime.active() else {
            return false;
        };
        let Some(face) = active
            .bundle
            .plan
            .domains
            .iter()
            .find(|domain| domain.region == region)
            .map(|domain| domain.face)
        else {
            return false;
        };
        active.bundle.snapshot.edges.iter().any(|edge| {
            let matches_target = match target {
                TopologySpanTarget::Outer(side) => edge.source == CompiledEdgeSource::Outer(side),
                TopologySpanTarget::Curve(span) => edge.source == CompiledEdgeSource::Curve(span),
            };
            matches_target && (edge.left == face || edge.right == face)
        })
    }
    fn draw_markers(&self, painter: &egui::Painter, r: Rect) {
        let p = self.editor.document.presentation;
        let source = self.editor.document.model.source;
        if source.enabled {
            let center = self.screen(source.position, r);
            painter.circle_stroke(center, 7.0, Stroke::new(2.0, GOLD));
            for offset in [egui::vec2(10.0, 0.0), egui::vec2(0.0, 10.0)] {
                painter.line_segment([center - offset, center + offset], Stroke::new(1.0, GOLD));
            }
        }
        let show_region = !self.capturing()
            && (self.inspector == Some(InspectorPanel::Materials)
                || matches!(
                    p.material_overlay,
                    MaterialOverlay::Regions | MaterialOverlay::Subdomains
                ));
        // While the Materials panel lists faces, outline the selected face
        // instead of the selected region, so a hole can be picked out too.
        let selected_face = (self.inspector == Some(InspectorPanel::Materials)
            && self.subdomain_listing == SubdomainListing::Faces)
            .then(|| self.selected_face())
            .flatten();
        if show_region && let Some(sampled) = &self.sampled {
            let color = match selected_face {
                Some((_, Some(region))) => {
                    subdomain_color(&self.editor.document.model.draft, region, 1.0)
                }
                Some((_, None)) => Color32::from_gray(150),
                None => subdomain_color(
                    &self.editor.document.model.draft,
                    self.region_selection,
                    1.0,
                ),
            };
            for span in &sampled.spans {
                let bounds = match selected_face {
                    Some((face, _)) => self.span_bounds_face(span.target, face),
                    None => self.span_bounds_region(span.target, self.region_selection),
                };
                if !bounds {
                    continue;
                }
                for pair in span.samples.windows(2) {
                    painter.line_segment(
                        [self.screen(pair[0].point, r), self.screen(pair[1].point, r)],
                        Stroke::new(4.0, color),
                    );
                }
            }
        }
        self.draw_probes(painter, r);
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
        if !self.capturing()
            && let Some(DragGesture::Marquee {
                start,
                current,
                operation,
                ..
            }) = &self.drag
        {
            let marquee = Rect::from_two_pos(*start, *current).intersect(r);
            let operation_color = match operation {
                MarqueeOperation::Replace => SELECT,
                MarqueeOperation::Add => GOLD,
                MarqueeOperation::Subtract => RED,
            };
            let containment = MarqueeContainment::from_drag(*start, *current);
            let border_color = match containment {
                MarqueeContainment::Enclosed => SELECT,
                MarqueeContainment::Crossing => TEAL,
            };
            painter.rect_filled(
                marquee,
                0.0,
                Color32::from_rgba_unmultiplied(
                    operation_color.r(),
                    operation_color.g(),
                    operation_color.b(),
                    24,
                ),
            );
            if containment == MarqueeContainment::Enclosed {
                painter.rect_stroke(
                    marquee,
                    0.0,
                    Stroke::new(1.0, border_color),
                    egui::StrokeKind::Inside,
                );
            } else {
                let corners = [
                    marquee.left_top(),
                    marquee.right_top(),
                    marquee.right_bottom(),
                    marquee.left_bottom(),
                    marquee.left_top(),
                ];
                painter.extend(egui::Shape::dashed_line(
                    &corners,
                    Stroke::new(1.0, border_color),
                    5.0,
                    3.0,
                ));
            }
            painter.text(
                marquee.left_top() + egui::vec2(4.0, 4.0),
                egui::Align2::LEFT_TOP,
                format!("{} · {}", operation.label(), containment.label()),
                egui::FontId::monospace(9.0),
                border_color,
            );
        }
        if let Some(draw) = &self.draw.as_ref().filter(|_| !self.capturing()) {
            self.draw_attachment_targets(painter, r, draw);
            let points = draw
                .points
                .iter()
                .map(|p| self.screen(*p, r))
                .collect::<Vec<_>>();
            for p in &points {
                painter.circle_filled(*p, 4.0, GOLD);
            }
            if points.len() > 1 {
                painter.add(egui::Shape::line(points.clone(), Stroke::new(1.5, GOLD)));
            }
            if let (Some(last), Some(pointer)) = (points.last(), painter.ctx().pointer_hover_pos())
            {
                let target = self
                    .draw_attachment_hit(ScreenPoint::new(pointer.x as f64, pointer.y as f64), r)
                    .map_or(pointer, |hit| self.screen(hit.point, r));
                painter.line_segment(
                    [*last, target],
                    Stroke::new(1.2, Color32::from_rgba_unmultiplied(248, 196, 112, 180)),
                );
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
    fn draw_attachment_hit(&self, pointer: ScreenPoint, r: Rect) -> Option<AttachmentHit> {
        let draw = self.draw.as_ref()?;
        if !matches!(draw.tool, DrawTool::Polyline | DrawTool::OpenSpline) {
            return None;
        }
        let compiled = self.editor.compiled_draft.as_ref()?;
        let face = (self.open_purpose == OpenPurpose::Separator)
            .then_some(draw.face)
            .flatten();
        let hit = weld_hit(
            compiled,
            &self.editor.document.model.draft.geometry,
            self.transform(r),
            pointer,
            self.hit_tolerance(14.0) as f64,
            None,
            face,
        )?;
        if self.open_purpose == OpenPurpose::Separator
            && draw.face.is_none()
            && !compiled.assignments.iter().any(|assignment| {
                assignment.face == attachment_face(hit.attachment, compiled)
                    && assignment.region.is_some()
            })
        {
            return None;
        }
        Some(hit)
    }
    fn draw_attachment_targets(&self, painter: &egui::Painter, r: Rect, draw: &DrawGesture) {
        if !matches!(draw.tool, DrawTool::Polyline | DrawTool::OpenSpline) {
            return;
        }
        let Some(compiled) = &self.editor.compiled_draft else {
            return;
        };
        let active_faces = compiled
            .assignments
            .iter()
            .filter_map(|assignment| assignment.region.map(|_| assignment.face))
            .collect::<BTreeSet<_>>();
        let required_face = (self.open_purpose == OpenPurpose::Separator)
            .then_some(draw.face)
            .flatten();
        let eligible = |faces: &[FaceId]| {
            required_face.map_or_else(
                || {
                    self.open_purpose == OpenPurpose::Baffle
                        || faces.iter().any(|face| active_faces.contains(face))
                },
                |required| faces.contains(&required),
            )
        };
        let stroke = Stroke::new(2.2, Color32::from_rgba_unmultiplied(248, 196, 112, 105));
        for edge in &compiled.topology.edges {
            let faces = match edge.source {
                CompiledEdgeSource::Outer(_) => [edge.left, edge.left],
                CompiledEdgeSource::Curve(_) => [edge.left, edge.right],
            };
            if eligible(&faces) {
                painter.line_segment(
                    [
                        self.screen(edge.points[0], r),
                        self.screen(edge.points[1], r),
                    ],
                    stroke,
                );
            }
        }
        for vertex in &compiled.topology.vertices {
            if vertex.authored.is_some()
                && eligible(
                    &vertex
                        .traces
                        .iter()
                        .map(|trace| trace.face)
                        .collect::<Vec<_>>(),
                )
            {
                painter.circle_filled(self.screen(vertex.point, r), 5.0, GOLD);
            }
        }
        self.draw_vertexless_nodes(painter, r, None);
        if let Some(pointer) = painter.ctx().pointer_hover_pos()
            && r.contains(pointer)
            && let Some(hit) =
                self.draw_attachment_hit(ScreenPoint::new(pointer.x as f64, pointer.y as f64), r)
        {
            self.draw_snap_ring(painter, r, &hit, None);
        }
    }
    /// Gold dots on every loose end and vertex-less breakpoint a curve may be
    /// welded onto. `dragged` is the end on the move: only its own other end
    /// stays eligible on that curve.
    fn draw_vertexless_nodes(
        &self,
        painter: &egui::Painter,
        r: Rect,
        dragged: Option<(CurveId, usize)>,
    ) {
        for candidate in &self.editor.document.model.draft.geometry.curves {
            let open = candidate.spline.is_open();
            let count = candidate.nodes.len();
            for (index, node) in candidate.nodes.iter().enumerate() {
                let is_end = open && (index == 0 || index + 1 == count);
                let eligible = match dragged {
                    Some((curve, dragged_node)) if curve == candidate.id => {
                        is_end && index != dragged_node
                    }
                    _ => true,
                };
                if node.vertex.is_some() || !eligible {
                    continue;
                }
                let Some(point) = candidate.spline.node_point(index) else {
                    continue;
                };
                painter.circle_filled(self.screen(point, r), if is_end { 5.0 } else { 3.5 }, GOLD);
            }
        }
    }
    fn draw_snap_ring(
        &self,
        painter: &egui::Painter,
        r: Rect,
        hit: &AttachmentHit,
        dragged: Option<CurveId>,
    ) {
        let point = self.screen(hit.point, r);
        painter.circle_filled(point, 4.0, GOLD);
        painter.circle_stroke(point, 9.0, Stroke::new(2.0, GOLD));
        let label = match hit.attachment {
            TopologyAttachment::LooseEnd { curve, .. } if Some(curve) == dragged => "Close",
            TopologyAttachment::LooseEnd { .. } => "Weld",
            _ => "Attach",
        };
        painter.text(
            point + egui::vec2(11.0, -11.0),
            egui::Align2::LEFT_BOTTOM,
            label,
            egui::FontId::proportional(11.0),
            GOLD,
        );
    }
    /// Targets and the live snap while a loose end is being dragged.
    fn draw_weld_targets(&self, painter: &egui::Painter, r: Rect) {
        if self.capturing() {
            return;
        }
        let Some(DragGesture::Endpoint { curve, node, snap }) = &self.drag else {
            return;
        };
        self.draw_vertexless_nodes(painter, r, Some((*curve, *node)));
        if let Some(hit) = snap {
            self.draw_snap_ring(painter, r, hit, Some(*curve));
        }
    }
    /// The node of a loose end when `control` is that end's on-curve control.
    fn loose_end_of_control(&self, curve: CurveId, control: usize) -> Option<usize> {
        let curve = self.editor.document.model.draft.geometry.curve(curve)?;
        let CurveSpline::Open(spline) = &curve.spline else {
            return None;
        };
        let node = if control == 0 {
            0
        } else if control + 1 == spline.controls().len() {
            curve.nodes.len() - 1
        } else {
            return None;
        };
        curve.nodes[node].vertex.is_none().then_some(node)
    }
    fn endpoint_control(&self, curve: CurveId, node: usize) -> Option<usize> {
        let curve = self.editor.document.model.draft.geometry.curve(curve)?;
        let CurveSpline::Open(spline) = &curve.spline else {
            return None;
        };
        Some(if node == 0 {
            0
        } else {
            spline.controls().len() - 1
        })
    }
    /// What a dragged loose end would weld onto at `pos`. Point targets come
    /// from the draft; junctions and edges from the last valid compile, as the
    /// editor's weld command resolves them.
    fn weld_hit_for(
        &self,
        pos: Pos2,
        r: Rect,
        exclude: Option<(CurveId, usize)>,
    ) -> Option<AttachmentHit> {
        let compiled = self
            .editor
            .compiled_draft
            .as_ref()
            .unwrap_or(&self.editor.compiled_accepted);
        weld_hit(
            compiled,
            &self.editor.document.model.draft.geometry,
            self.transform(r),
            ScreenPoint::new(pos.x as f64, pos.y as f64),
            self.hit_tolerance(14.0) as f64,
            exclude,
            None,
        )
    }
    /// Welds the released end onto whatever it landed on. The snap is recomputed
    /// here rather than taken from the gesture: one captured mid-drag may name a
    /// face of a snapshot that validation has since replaced.
    fn finish_endpoint_drag(
        &mut self,
        curve: CurveId,
        node: usize,
        pointer: Option<Pos2>,
        r: Rect,
    ) {
        let Some(pos) = pointer else {
            return;
        };
        let Some(hit) = self.weld_hit_for(pos, r, Some((curve, node))) else {
            return;
        };
        let endpoint = if node == 0 { 0 } else { 1 };
        match self.editor.weld_endpoint(curve, endpoint, hit.attachment) {
            Ok(weld) => {
                self.selection = match weld.seam_control {
                    Some(control) => TopologySelection::Handle(TopologyHandle::Control {
                        curve: weld.curve,
                        control,
                    }),
                    None => self.endpoint_control(curve, node).map_or(
                        TopologySelection::None,
                        |control| {
                            TopologySelection::Handle(TopologyHandle::Control { curve, control })
                        },
                    ),
                };
                self.invalidate_samples();
                let what = match hit.attachment {
                    TopologyAttachment::LooseEnd { curve: other, .. } if other == curve => {
                        "Closed into a loop"
                    }
                    TopologyAttachment::LooseEnd { .. } => "Welded into one curve",
                    TopologyAttachment::Junction { .. } | TopologyAttachment::Breakpoint { .. } => {
                        "Attached to junction"
                    }
                    TopologyAttachment::Boundary(FaceAnchor::Outer { .. }) => {
                        "Attached to the outer boundary"
                    }
                    TopologyAttachment::Boundary(FaceAnchor::Curve { .. }) => {
                        "Attached to the curve"
                    }
                };
                let mut parts = vec![what.to_owned()];
                if !weld.span_splits.is_empty() {
                    parts.push("junction inserted".to_owned());
                }
                if weld.promoted {
                    parts.push("curve promoted to a baffle".to_owned());
                }
                self.notify(parts.join(" · "));
            }
            Err(error) => self.message = error,
        }
    }
    fn transform_gizmo(&self, r: Rect) -> Option<(Point2, Pos2, f32, f32, f32)> {
        if self.draw.is_some() || self.pulse_mode || self.probe_mode.is_some() {
            return None;
        }
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
        let pivot = self.gizmo_pivot_for(selected, &spans)?;
        let updates = plan_rigid_transform(
            &self.editor.document.model.draft.geometry,
            selected,
            RigidTransform {
                pivot,
                translation: Point2::default(),
                rotation_radians: 0.0,
                scale: 1.0,
            },
        )
        .ok()?;
        let center = self.screen(pivot, r);
        let radius = updates
            .iter()
            .map(|update| match update {
                funfern_app::topology_viewport::TopologyTransformUpdate::Control {
                    point, ..
                }
                | funfern_app::topology_viewport::TopologyTransformUpdate::Vertex {
                    point, ..
                } => center.distance(self.screen(*point, r)),
            })
            .fold(22.0_f32, f32::max)
            + GIZMO_PADDING;
        let x_radius = updates
            .iter()
            .map(|update| match update {
                funfern_app::topology_viewport::TopologyTransformUpdate::Control {
                    point, ..
                }
                | funfern_app::topology_viewport::TopologyTransformUpdate::Vertex {
                    point, ..
                } => (self.screen(*point, r).x - center.x).abs(),
            })
            .fold(22.0_f32, f32::max)
            + GIZMO_PADDING;
        let y_radius = updates
            .iter()
            .map(|update| match update {
                funfern_app::topology_viewport::TopologyTransformUpdate::Control {
                    point, ..
                }
                | funfern_app::topology_viewport::TopologyTransformUpdate::Vertex {
                    point, ..
                } => (self.screen(*point, r).y - center.y).abs(),
            })
            .fold(22.0_f32, f32::max)
            + GIZMO_PADDING;
        Some((pivot, center, radius, x_radius, y_radius))
    }
    fn draw_transform_gizmo(&self, painter: &egui::Painter, r: Rect) {
        if self.capturing() {
            return;
        }
        let Some((_, center, radius, x_radius, y_radius)) = self.transform_gizmo(r) else {
            return;
        };
        painter.circle_stroke(
            center,
            radius,
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(72, 166, 255, 150)),
        );
        painter.line_segment(
            [center, center + egui::vec2(x_radius, 0.0)],
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(72, 166, 255, 110)),
        );
        painter.line_segment(
            [center, center + egui::vec2(0.0, -y_radius)],
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(72, 166, 255, 110)),
        );
        painter.circle_filled(center, 6.0, Color32::from_rgb(16, 23, 31));
        painter.circle_stroke(center, 6.0, Stroke::new(2.0, SELECT));
        painter.line_segment(
            [
                center - egui::vec2(11.0, 0.0),
                center + egui::vec2(11.0, 0.0),
            ],
            Stroke::new(1.5, Color32::WHITE),
        );
        painter.line_segment(
            [
                center - egui::vec2(0.0, 11.0),
                center + egui::vec2(0.0, 11.0),
            ],
            Stroke::new(1.5, Color32::WHITE),
        );
        let diagonal = radius * std::f32::consts::FRAC_1_SQRT_2;
        painter.rect_filled(
            Rect::from_center_size(
                center + egui::vec2(diagonal, diagonal),
                egui::vec2(9.0, 9.0),
            ),
            1.0,
            SELECT,
        );
        painter.rect_filled(
            Rect::from_center_size(center + egui::vec2(x_radius, 0.0), egui::vec2(7.0, 11.0)),
            1.0,
            SELECT,
        );
        painter.rect_filled(
            Rect::from_center_size(center + egui::vec2(0.0, -y_radius), egui::vec2(11.0, 7.0)),
            1.0,
            SELECT,
        );
    }
    fn scale_axis_cursor(axis: GizmoScaleAxis) -> egui::CursorIcon {
        match axis {
            GizmoScaleAxis::Uniform => egui::CursorIcon::ResizeNwSe,
            GizmoScaleAxis::X => egui::CursorIcon::ResizeHorizontal,
            GizmoScaleAxis::Y => egui::CursorIcon::ResizeVertical,
        }
    }

    fn outer_side_cursor(side: OuterSide) -> egui::CursorIcon {
        match side {
            OuterSide::Left | OuterSide::Right => egui::CursorIcon::ResizeHorizontal,
            OuterSide::Bottom | OuterSide::Top => egui::CursorIcon::ResizeVertical,
        }
    }

    /// A mode that owns the whole viewport says so with the cursor, since there
    /// is no handle anywhere to carry the meaning.
    fn modal_cursor(&self, pos: Pos2, r: Rect) -> Option<egui::CursorIcon> {
        if let Some(pending) = &self.pending_removal {
            // Only the candidates are clickable; everywhere else the click does
            // nothing and the cursor should not promise otherwise.
            return self
                .draft_region_at(self.world(pos, r))
                .filter(|region| pending.choices().contains(region))
                .map(|_| egui::CursorIcon::PointingHand);
        }
        (self.draw.is_some() || self.pulse_mode || self.probe_mode.is_some())
            .then_some(egui::CursorIcon::Crosshair)
    }

    /// The cursor a live gesture holds on to, so what appeared under the pointer
    /// does not vanish the moment the drag begins.
    fn drag_cursor(&self) -> Option<egui::CursorIcon> {
        Some(match self.drag.as_ref()? {
            DragGesture::Pivot { .. } | DragGesture::Spans { .. } => egui::CursorIcon::Move,
            DragGesture::Rotate { .. } => egui::CursorIcon::Grabbing,
            DragGesture::Scale { axis, .. } => Self::scale_axis_cursor(*axis),
            DragGesture::Domain {
                drag: DomainDrag::Corner { index, .. },
            } => Self::domain_corner_cursor(*index),
            DragGesture::Domain {
                drag: DomainDrag::Side { side, .. },
            } => Self::outer_side_cursor(*side),
            DragGesture::MaterialFrame { hit, .. } => match hit {
                MaterialFrameGizmoHit::Origin => egui::CursorIcon::Move,
                MaterialFrameGizmoHit::Rotate => egui::CursorIcon::Grabbing,
            },
            DragGesture::Probe { hit, .. } => match hit {
                ProbeHit::AreaDiskRadius(_) => egui::CursorIcon::ResizeHorizontal,
                ProbeHit::AreaDiskBody(_) | ProbeHit::SegmentBody(_) => egui::CursorIcon::Move,
                _ => return None,
            },
            _ => return None,
        })
    }

    /// What the pointer is over while editing. Only an affordance the drawing
    /// does not already announce, or one whose direction matters, earns a
    /// cursor: a drawn handle that moves itself is its own announcement. The
    /// order matches the one the press handler resolves grabs in.
    fn hover_cursor(&self, pos: Pos2, r: Rect) -> Option<egui::CursorIcon> {
        if self.selected_material_frame().is_some()
            && let Some(hit) = self.hit_material_frame_gizmo(pos, r)
        {
            return Some(match hit {
                MaterialFrameGizmoHit::Origin => egui::CursorIcon::Move,
                MaterialFrameGizmoHit::Rotate => egui::CursorIcon::Grab,
            });
        }
        if let Some((hit, _)) = self.hit_transform_gizmo(pos, r) {
            return Some(match hit {
                TransformGizmoHit::Pivot => egui::CursorIcon::Move,
                TransformGizmoHit::Rotate => egui::CursorIcon::Grab,
                TransformGizmoHit::Scale(axis) => Self::scale_axis_cursor(axis),
            });
        }
        if let Some(hit) = self.hit_probe(pos, r) {
            return match hit {
                ProbeHit::AreaDiskRadius(_) => Some(egui::CursorIcon::ResizeHorizontal),
                ProbeHit::AreaDiskBody(_) | ProbeHit::SegmentBody(_) => {
                    Some(egui::CursorIcon::Move)
                }
                _ => None,
            };
        }
        if let Some(index) = self.hit_domain_corner(pos, r) {
            return Some(Self::domain_corner_cursor(index));
        }
        let hit = self.sampled.as_ref().and_then(|sampled| {
            sampled.hit_test(
                self.transform(r),
                ScreenPoint::new(pos.x as f64, pos.y as f64),
                self.hit_tolerance(13.0) as f64,
                self.hit_tolerance(9.0) as f64,
            )
        })?;
        let TopologyHit::Span { target, .. } = hit else {
            return None;
        };
        if let TopologySpanTarget::Outer(side) = target {
            return Some(Self::outer_side_cursor(side));
        }
        // Pressing a span of the selection drags the whole of it, but only when
        // that selection can move rigidly. The gizmo answers exactly that, so
        // its absence is what tells the user to widen the selection.
        let selected = self
            .selection
            .spans()
            .is_some_and(|spans| spans.contains(&target));
        (selected && self.transform_gizmo(r).is_some()).then_some(egui::CursorIcon::Move)
    }

    fn hit_transform_gizmo(&self, point: Pos2, r: Rect) -> Option<(TransformGizmoHit, Point2)> {
        let (pivot, center, radius, x_radius, y_radius) = self.transform_gizmo(r)?;
        if center.distance(point) <= 12.0 {
            return Some((TransformGizmoHit::Pivot, pivot));
        }
        let diagonal = radius * std::f32::consts::FRAC_1_SQRT_2;
        for (axis, offset) in [
            (GizmoScaleAxis::Uniform, egui::vec2(diagonal, diagonal)),
            (GizmoScaleAxis::X, egui::vec2(x_radius, 0.0)),
            (GizmoScaleAxis::Y, egui::vec2(0.0, -y_radius)),
        ] {
            if (center + offset).distance(point) <= 10.0 {
                return Some((TransformGizmoHit::Scale(axis), pivot));
            }
        }
        ((center.distance(point) - radius).abs() <= 7.0)
            .then_some((TransformGizmoHit::Rotate, pivot))
    }
    fn handle_viewport_input(&mut self, ui: &egui::Ui, response: &egui::Response, r: Rect) {
        let pointer = response.interact_pointer_pos();
        // egui reports `drag_started` only once the pointer has travelled past
        // `max_click_dist`, so by then the live position has already left
        // whatever the user aimed at. Every grab test uses the press origin.
        let press = ui.input(|input| input.pointer.press_origin()).or(pointer);
        let typing = ui.ctx().egui_wants_keyboard_input();
        let touch_active = ui.input(|input| input.any_touches());
        self.touch_active = touch_active;
        let multi_touch = ui.input(|input| input.multi_touch());
        if !touch_active {
            // Hover, not interaction: `interact_pointer_pos` is `None` until a
            // gesture is already under way, which is precisely when a cursor has
            // nothing left to announce.
            let hovering = response.hover_pos();
            let cursor = hovering
                .and_then(|pos| self.modal_cursor(pos, r))
                .or_else(|| self.drag_cursor())
                .or_else(|| hovering.and_then(|pos| self.hover_cursor(pos, r)));
            if let Some(cursor) = cursor {
                ui.ctx().set_cursor_icon(cursor);
            }
        }
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
        // The first Escape leaves the text field, which egui has already done by
        // now; only a second one reaches the viewport.
        if !typing
            && !self.keyboard_focus_previous
            && ui.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.cancel_interaction();
            return;
        }
        if self.pending_removal.is_some() {
            // The survivor question owns the viewport until it is answered or
            // cancelled: a click picks, everything else waits.
            if response.clicked_by(egui::PointerButton::Primary)
                && let Some(pos) = pointer
            {
                self.pick_removal_survivor(self.world(pos, r));
            }
            return;
        }
        if response.double_clicked()
            && let Some(pos) = pointer
        {
            // The same lookup a single click uses, so a double click honours the
            // View visibility toggles and opens the probe on top rather than the
            // one underneath.
            if let Some(hit) = self.hit_probe(pos, r) {
                self.probe_windows.insert(hit.id());
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
            if !typing && ui.input(|i| i.key_pressed(egui::Key::Backspace)) {
                if let Some(draw) = &mut self.draw {
                    draw.points.pop();
                    draw.attachments.pop();
                }
            }
            if !typing && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
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
            if let (Some(pos), Some(grab)) = (pointer, press) {
                if let Some(hit) = self.hit_material_frame_gizmo(grab, r)
                    && let Some((region, start)) = self.selected_material_frame()
                {
                    let world = self.world(grab, r);
                    self.editor.begin();
                    self.drag = Some(DragGesture::MaterialFrame {
                        region,
                        start,
                        hit,
                        grab: match hit {
                            MaterialFrameGizmoHit::Origin => 0.0,
                            MaterialFrameGizmoHit::Rotate => {
                                let relative = world - start.origin;
                                relative.y.atan2(relative.x)
                            }
                        },
                    });
                    return;
                }
                if let Some((hit, pivot)) = self.hit_transform_gizmo(grab, r) {
                    let relative = self.world(pos, r) - pivot;
                    self.drag = Some(match hit {
                        TransformGizmoHit::Pivot => DragGesture::Pivot {
                            previous: self.gizmo_pivot.clone(),
                            offset: pivot - self.world(pos, r),
                        },
                        TransformGizmoHit::Rotate => {
                            self.editor.begin();
                            DragGesture::Rotate {
                                pivot,
                                start_angle: relative.y.atan2(relative.x),
                                geometry: self.editor.document.model.draft.geometry.clone(),
                            }
                        }
                        TransformGizmoHit::Scale(axis) => {
                            self.editor.begin();
                            DragGesture::Scale {
                                axis,
                                pivot,
                                start_distance: (Self::scale_drag_distance(axis, relative)
                                    - GIZMO_PADDING as f64 / self.scale)
                                    .max(1.0e-12),
                                geometry: self.editor.document.model.draft.geometry.clone(),
                            }
                        }
                    });
                    return;
                }
                let screen = ScreenPoint::new(grab.x as f64, grab.y as f64);
                let source = self.editor.document.model.source;
                if source.enabled
                    && self.screen(source.position, r).distance(grab) <= self.hit_tolerance(13.0)
                {
                    self.editor.begin();
                    self.drag = Some(DragGesture::Source);
                    return;
                }
                if let Some(hit) = self.hit_probe(grab, r) {
                    self.selected_probe = Some(hit.id());
                    self.selection = TopologySelection::None;
                    if hit.draggable()
                        && let Some(original) = self
                            .editor
                            .document
                            .model
                            .probes
                            .iter()
                            .find(|probe| probe.id == hit.id())
                            .map(|probe| probe.target.clone())
                    {
                        self.editor.begin();
                        self.drag = Some(DragGesture::Probe {
                            hit,
                            grab: self.world(pos, r),
                            original,
                        });
                    }
                    return;
                }
                // A corner of the outer rectangle outranks the two sides that
                // meet there, so both a corner and a side drag stay reachable.
                if let Some(index) = self.hit_domain_corner(grab, r) {
                    self.selected_probe = None;
                    self.selection = TopologySelection::Spans(
                        [
                            TopologySpanTarget::Outer(OuterSide::ALL[index]),
                            TopologySpanTarget::Outer(
                                OuterSide::ALL
                                    [(index + OuterSide::ALL.len() - 1) % OuterSide::ALL.len()],
                            ),
                        ]
                        .into_iter()
                        .collect(),
                    );
                    self.editor.begin();
                    self.drag = Some(DragGesture::Domain {
                        drag: DomainDrag::Corner {
                            index,
                            start: self.editor.document.model.draft.geometry.domain,
                        },
                    });
                    return;
                }
                let hit = self.sampled.as_ref().and_then(|sampled| {
                    sampled.hit_test(
                        self.transform(r),
                        screen,
                        self.hit_tolerance(13.0) as f64,
                        self.hit_tolerance(9.0) as f64,
                    )
                });
                if let Some(hit) = hit {
                    self.selected_probe = None;
                    let shift = ui.input(|i| i.modifiers.shift);
                    if !Self::drag_starts_inside_span_selection(&self.selection, hit) {
                        self.selection.apply_hit(
                            &self.editor.document.model.draft.geometry,
                            hit,
                            shift,
                            ui.input(|i| i.modifiers.command),
                        );
                    }
                    self.editor.begin();
                    let loose_end = match hit {
                        TopologyHit::Handle {
                            handle: TopologyHandle::Control { curve, control },
                            ..
                        } => self
                            .loose_end_of_control(curve, control)
                            .map(|node| (curve, node)),
                        _ => None,
                    };
                    let outer_side = match hit {
                        TopologyHit::Span {
                            target: TopologySpanTarget::Outer(side),
                            ..
                        } if !shift && !ui.input(|i| i.modifiers.command) => Some(side),
                        _ => None,
                    };
                    self.drag = Some(if let Some(side) = outer_side {
                        DragGesture::Domain {
                            drag: DomainDrag::Side {
                                side,
                                start: self.editor.document.model.draft.geometry.domain,
                            },
                        }
                    } else if let Some((curve, node)) = loose_end {
                        DragGesture::Endpoint {
                            curve,
                            node,
                            snap: None,
                        }
                    } else {
                        match hit {
                            TopologyHit::Handle { handle, .. } => DragGesture::Handle { handle },
                            TopologyHit::Span { .. } => {
                                let selected = self.selection.spans().cloned().unwrap_or_default();
                                let curve_spans = selected
                                    .iter()
                                    .filter_map(|target| match target {
                                        TopologySpanTarget::Curve(span) => Some(*span),
                                        TopologySpanTarget::Outer(_) => None,
                                    })
                                    .collect::<BTreeSet<_>>();
                                let custom_pivot = self
                                    .gizmo_pivot
                                    .as_ref()
                                    .is_some_and(|(selection, _)| selection == &selected);
                                DragGesture::Spans {
                                    start: self.world(pos, r),
                                    pivot: self
                                        .gizmo_pivot_for(&selected, &curve_spans)
                                        .unwrap_or_default(),
                                    custom_pivot,
                                    gizmo_before: self.gizmo_pivot.clone(),
                                    geometry: self.editor.document.model.draft.geometry.clone(),
                                }
                            }
                        }
                    });
                } else {
                    let base = self.selection.spans().cloned().unwrap_or_default();
                    self.drag = Some(DragGesture::Marquee {
                        start: pos,
                        current: pos,
                        base,
                        operation: Self::marquee_operation(ui.input(|input| input.modifiers)),
                    });
                }
            }
        }
        if response.dragged_by(egui::PointerButton::Primary) {
            if let (Some(pos), Some(drag)) = (pointer, self.drag.clone()) {
                let point = self.world(pos, r);
                let shift = ui.input(|input| input.modifiers.shift);
                let invalidates_geometry = !matches!(
                    &drag,
                    DragGesture::Marquee { .. } | DragGesture::Pivot { .. }
                );
                let result = match drag {
                    DragGesture::Endpoint { curve, node, .. } => {
                        let point = if shift {
                            Self::snap_point(point)
                        } else {
                            point
                        };
                        let moved = self
                            .endpoint_control(curve, node)
                            .ok_or_else(|| "Curve end no longer exists".to_owned())
                            .and_then(|control| {
                                plan_handle_drag(
                                    &self.editor.document.model.draft.geometry,
                                    TopologyHandle::Control { curve, control },
                                    point,
                                )
                                .map_err(|e| e.to_string())
                            })
                            .and_then(|update| {
                                self.editor.apply_transform_updates_during_edit(&[update])
                            });
                        let snap = self.weld_hit_for(pos, r, Some((curve, node)));
                        self.drag = Some(DragGesture::Endpoint { curve, node, snap });
                        moved
                    }
                    DragGesture::Domain { drag } => {
                        let point = if shift {
                            Self::snap_point(point)
                        } else {
                            point
                        };
                        let start = match drag {
                            DomainDrag::Side { start, .. } | DomainDrag::Corner { start, .. } => {
                                start
                            }
                        };
                        self.editor
                            .set_domain_during_edit(Self::resize_domain(start, drag, point))
                    }
                    DragGesture::Handle { handle } => {
                        let point = if shift {
                            Self::snap_point(point)
                        } else {
                            point
                        };
                        plan_handle_drag(&self.editor.document.model.draft.geometry, handle, point)
                            .map_err(|e| e.to_string())
                            .and_then(|update| {
                                self.editor.apply_transform_updates_during_edit(&[update])
                            })
                    }
                    DragGesture::Spans {
                        start,
                        pivot,
                        custom_pivot,
                        gizmo_before: _,
                        geometry,
                    } => {
                        let selected = self.selection.spans().cloned().unwrap_or_default();
                        let target = if shift {
                            Self::snap_point(pivot + point - start)
                        } else {
                            pivot + point - start
                        };
                        let translation = target - pivot;
                        plan_rigid_transform(
                            &geometry,
                            &selected,
                            RigidTransform {
                                pivot: Point2::default(),
                                translation,
                                rotation_radians: 0.0,
                                scale: 1.0,
                            },
                        )
                        .map_err(|e| e.to_string())
                        .and_then(|updates| {
                            self.editor.document.model.draft.geometry = geometry;
                            self.editor.apply_transform_updates_during_edit(&updates)?;
                            if custom_pivot {
                                self.gizmo_pivot = Some((selected, pivot + translation));
                            }
                            Ok(())
                        })
                    }
                    DragGesture::Rotate {
                        pivot,
                        start_angle,
                        geometry,
                    } => {
                        let selected = self.selection.spans().cloned().unwrap_or_default();
                        let relative = point - pivot;
                        let mut angle = relative.y.atan2(relative.x) - start_angle;
                        if shift {
                            let step = 15.0_f64.to_radians();
                            angle = (angle / step).round() * step;
                        }
                        plan_rigid_transform(
                            &geometry,
                            &selected,
                            RigidTransform {
                                pivot,
                                translation: Point2::default(),
                                rotation_radians: angle,
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
                        axis,
                        pivot,
                        start_distance,
                        geometry,
                    } => {
                        let selected = self.selection.spans().cloned().unwrap_or_default();
                        let distance = (Self::scale_drag_distance(axis, point - pivot)
                            - GIZMO_PADDING as f64 / self.scale)
                            .max(0.0);
                        let mut factor = (distance / start_distance).max(0.01);
                        if shift {
                            factor = ((factor * 10.0).round() / 10.0).max(0.1);
                        }
                        plan_axis_scale(
                            &geometry,
                            &selected,
                            pivot,
                            if axis == GizmoScaleAxis::Y {
                                1.0
                            } else {
                                factor
                            },
                            if axis == GizmoScaleAxis::X {
                                1.0
                            } else {
                                factor
                            },
                        )
                        .map_err(|e| e.to_string())
                        .and_then(|updates| {
                            self.editor.document.model.draft.geometry = geometry;
                            self.editor.apply_transform_updates_during_edit(&updates)
                        })
                    }
                    DragGesture::Pivot { offset, .. } => {
                        let selected = self.selection.spans().cloned().unwrap_or_default();
                        let target = if shift {
                            Self::snap_point(point + offset)
                        } else {
                            point + offset
                        };
                        self.gizmo_pivot = Some((selected, target));
                        Ok(())
                    }
                    DragGesture::Marquee { start, base, .. } => {
                        let operation = Self::marquee_operation(ui.input(|input| input.modifiers));
                        let hits = self
                            .sampled
                            .as_ref()
                            .map(|sampled| {
                                sampled.marquee_hits(
                                    self.transform(r),
                                    ScreenPoint::new(start.x as f64, start.y as f64),
                                    ScreenPoint::new(pos.x as f64, pos.y as f64),
                                )
                            })
                            .unwrap_or_default();
                        self.selection = Self::selection_from_spans(Self::marquee_result(
                            &base, hits, operation,
                        ));
                        self.drag = Some(DragGesture::Marquee {
                            start,
                            current: pos,
                            base,
                            operation,
                        });
                        Ok(())
                    }
                    DragGesture::MaterialFrame {
                        region,
                        start,
                        hit,
                        grab,
                    } => {
                        let mut frame = start;
                        match hit {
                            MaterialFrameGizmoHit::Origin => {
                                frame.origin = if shift {
                                    Self::snap_point(point)
                                } else {
                                    point
                                };
                            }
                            MaterialFrameGizmoHit::Rotate => {
                                let relative = point - start.origin;
                                let mut angle =
                                    start.angle_radians + relative.y.atan2(relative.x) - grab;
                                if shift {
                                    let step = 15.0_f64.to_radians();
                                    angle = (angle / step).round() * step;
                                }
                                frame.angle_radians = angle;
                            }
                        }
                        self.editor.set_region_frame_during_edit(region, frame)
                    }
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
                    DragGesture::Probe {
                        hit,
                        grab,
                        ref original,
                    } => {
                        // Snap what the drag moves, not the pointer: the grab
                        // offset would otherwise leave the probe off the grid by
                        // however far inside itself it was picked up.
                        self.drag_probe(hit, original, point - grab, shift);
                        Ok(())
                    }
                };
                if let Err(error) = result {
                    self.message = error;
                }
                if invalidates_geometry {
                    self.invalidate_samples();
                }
            }
        }
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            if let Some(drag) = self.drag.take() {
                match drag {
                    DragGesture::Marquee {
                        start,
                        current,
                        base,
                        ..
                    } => {
                        let release_modifiers = ui.input(|input| {
                            input
                                .raw
                                .events
                                .iter()
                                .find_map(|event| match event {
                                    egui::Event::PointerButton {
                                        button: egui::PointerButton::Primary,
                                        pressed: false,
                                        modifiers,
                                        ..
                                    } => Some(*modifiers),
                                    _ => None,
                                })
                                .unwrap_or(input.modifiers)
                        });
                        let operation = Self::marquee_operation(release_modifiers);
                        let end = pointer.unwrap_or(current);
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
                        self.selection = Self::selection_from_spans(Self::marquee_result(
                            &base, hits, operation,
                        ));
                    }
                    DragGesture::Pivot { .. } => {}
                    DragGesture::Endpoint { curve, node, .. } => {
                        self.finish_endpoint_drag(curve, node, pointer, r);
                        // A no-op after a successful weld, which committed the
                        // drag and the weld together; the drag alone otherwise.
                        self.editor.commit();
                    }
                    _ => self.editor.commit(),
                }
            }
        }
        if response.clicked() && self.drag.is_none() {
            if let Some(pos) = pointer {
                if let Some(hit) = self.hit_probe(pos, r) {
                    self.selected_probe = Some(hit.id());
                    self.selection = TopologySelection::None;
                    return;
                }
                let hit = self.sampled.as_ref().and_then(|sampled| {
                    sampled.hit_test(
                        self.transform(r),
                        ScreenPoint::new(pos.x as f64, pos.y as f64),
                        self.hit_tolerance(13.0) as f64,
                        self.hit_tolerance(9.0) as f64,
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
                    let point = self.world(pos, r);
                    if self.subdomain_listing == SubdomainListing::Faces {
                        if let Some(index) = self.draft_face_assignment_at(point) {
                            self.face_selection = index;
                            if let Some(region) =
                                self.editor.document.model.draft.face_assignments[index].region
                            {
                                self.region_selection = region;
                            }
                        }
                    } else if let Some(region) = self.region_at(point) {
                        self.region_selection = region;
                    }
                }
            }
        }
        if !typing
            && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
        {
            self.delete_selection();
        }
    }
    fn draw_click(&mut self, mut point: Point2, screen: ScreenPoint, r: Rect) {
        let snap = self.draw_attachment_hit(screen, r);
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
            if let Some(hit) = snap {
                point = hit.point;
                attachment = Some(hit.attachment);
                if gesture.points.is_empty()
                    && let Some(compiled) = &self.editor.compiled_draft
                {
                    gesture.face = Some(attachment_face(hit.attachment, compiled));
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
            self.delete_span_selection();
            return;
        }
        // One gesture, one history entry: the removals share a bracket, so undo
        // takes the whole selection back rather than one curve at a time.
        self.editor.begin();
        for curve in curves {
            // Every removal invalidates the compiled draft, and both the choice
            // query and the command need it, so the next curve in a multi-curve
            // selection has to wait for revalidation rather than fail.
            self.settle_editor();
            // Merging two assigned subdomains needs an explicit survivor, so
            // hand the choice to the Edit panel instead of failing the gesture.
            match self.editor.curve_removal_choices(curve) {
                Ok(choices) if choices.len() > 1 => {
                    // Close what is already done and let the question stand on
                    // its own; answering it is a separate decision.
                    self.editor.commit();
                    self.pending_removal = Some(PendingRemoval::Curve(curve, choices));
                    return;
                }
                Ok(_) => {}
                Err(error) => {
                    self.editor.cancel();
                    self.message = error;
                    return;
                }
            }
            match self.editor.remove_curve_during_edit(curve, None) {
                Ok(removal) => self.report_curve_removal(&removal),
                Err(error) => {
                    self.editor.cancel();
                    self.message = error;
                    return;
                }
            }
        }
        self.editor.commit();
        self.selection = TopologySelection::None;
        self.material_edit = None;
        self.material_formula_edits.clear();
        self.material_formula_errors.clear();
        self.invalidate_samples();
    }
    /// Drives the editor's cooperative validation to a decision. Only a gesture
    /// that must issue several dependent commands in one frame needs this; the
    /// ordinary path validates across frames in `frame`.
    fn settle_editor(&mut self) {
        for _ in 0..4096 {
            if self.editor.acceptance != TopologyAcceptance::Pending {
                return;
            }
            self.editor.validate_frame(4096);
        }
    }
    /// Deletes a partial span selection, splitting the curve and leaving baffles.
    fn delete_span_selection(&mut self) {
        let TopologySelection::Spans(targets) = &self.selection else {
            return;
        };
        let spans = targets
            .iter()
            .filter_map(|target| match target {
                TopologySpanTarget::Curve(span) => Some(*span),
                TopologySpanTarget::Outer(_) => None,
            })
            .collect::<BTreeSet<_>>();
        if spans.is_empty() {
            return;
        }
        let choices = match self.editor.span_removal_choices(&spans) {
            Ok(choices) => choices,
            Err(error) => {
                self.message = error;
                return;
            }
        };
        if choices.len() > 1 {
            self.pending_removal = Some(PendingRemoval::Spans(spans, choices));
            return;
        }
        match self.editor.remove_spans(&spans, choices.first().copied()) {
            Ok(removal) => {
                self.selection = TopologySelection::None;
                self.invalidate_samples();
                self.report_span_removal(&removal);
            }
            Err(error) => self.message = error,
        }
    }
    /// Says what the deletion did beyond the selection: a curve promoted to a
    /// baffle or a probe dropped is not something to discover later.
    fn report_span_removal(&mut self, removal: &TopologySpanRemoval) {
        let lead = match removal.pieces.len() {
            0 => "Curve deleted".to_owned(),
            1 => "Deleted spans; the rest is a baffle".to_owned(),
            count => format!("Deleted spans; split into {count} baffles"),
        };
        self.report_removal(
            lead,
            &removal.promoted,
            &removal.joined,
            &removal.removed_probes,
            &removal.removed_regions,
        );
    }
    fn report_curve_removal(&mut self, removal: &TopologyCurveRemoval) {
        self.report_removal(
            "Curve deleted".to_owned(),
            &removal.promoted,
            &removal.joined,
            &removal.removed_probes,
            &removal.removed_regions,
        );
    }
    fn report_removal(
        &mut self,
        lead: String,
        promoted: &[CurveId],
        joined: &[JoinRecord],
        removed_probes: &[ProbeId],
        removed_regions: &[RegionId],
    ) {
        let mut parts = vec![lead];
        if !promoted.is_empty() {
            parts.push(format!(
                "{} attached curve{} promoted to baffles",
                promoted.len(),
                if promoted.len() == 1 { "" } else { "s" }
            ));
        }
        let closed = joined
            .iter()
            .filter(|record| record.survivor == record.absorbed)
            .count();
        let fused = joined.len() - closed;
        if fused > 0 {
            parts.push(format!(
                "{} pair{} of loose ends welded into one curve",
                fused,
                if fused == 1 { "" } else { "s" }
            ));
        }
        if closed > 0 {
            parts.push(format!(
                "{} curve{} closed into a loop",
                closed,
                if closed == 1 { "" } else { "s" }
            ));
        }
        if !removed_probes.is_empty() {
            parts.push(format!("{} probe(s) removed", removed_probes.len()));
        }
        if !removed_regions.is_empty() {
            parts.push(format!("{} subdomain(s) merged", removed_regions.len()));
        }
        self.notify(parts.join(" · "));
    }
    /// Stable region owning the committed face under a world point.
    fn region_at(&self, point: Point2) -> Option<RegionId> {
        let active = self.runtime.active()?;
        let face = active.bundle.snapshot.face_at(point)?;
        active
            .bundle
            .plan
            .domains
            .iter()
            .find(|domain| domain.face == face)
            .map(|domain| domain.region)
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
    /// Wall time a frame lends to topology preparation. The cooperative jobs
    /// step at very fine granularity, so the earlier fixed 256 steps per frame
    /// stretched a 60 ms rebuild across hundreds of frames.
    const PREPARATION_FRAME_BUDGET: std::time::Duration = std::time::Duration::from_millis(6);

    fn request_runtime(&mut self) {
        if self.editor.acceptance != TopologyAcceptance::Valid
            || self.editor.editing()
            || self.mesh_edge_dragging
        {
            return;
        }
        let options = MeshingOptions {
            target_edge_length: self.mesh_edge,
            curve_tolerance: (self.mesh_edge * 0.02).min(5e-4),
            ..MeshingOptions::default()
        };
        if !self.remesh_requested
            && self.requested_revision == Some(self.editor.revision)
            && self.requested_edge == self.mesh_edge
            && (self.runtime.phase().is_some()
                || self.runtime.active().is_some_and(|active| {
                    active.bundle.token.document_revision == self.editor.revision
                }))
        {
            return;
        }
        let fresh = self.runtime.active().is_none() || self.reset_requested;
        if std::mem::take(&mut self.remesh_requested) {
            self.runtime.request_full_rebuild();
        }
        self.runtime.set_preserve_adaptation(self.amr_enabled);
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
                self.begin_handoff_timeline();
            }
            Err(error) => self.message = error,
        }
    }
    fn begin_handoff_timeline(&mut self) {
        self.handoff_requested = Some(Instant::now());
        self.handoff_ready = None;
        self.handoff_upload = None;
    }
    fn record_handoff(&mut self, active: &Arc<PreparedTopology>) {
        let now = Instant::now();
        let millis = |from: Option<Instant>, to: Instant| {
            from.map_or(0.0, |from| (to - from).as_secs_f64() * 1000.0)
        };
        let ready = self.handoff_ready.unwrap_or(now);
        let upload = self.handoff_upload.unwrap_or(ready);
        self.last_handoff = Some(HandoffRecord {
            prepare_ms: millis(self.handoff_requested, ready),
            drain_ms: millis(Some(ready), upload),
            upload_ms: millis(Some(upload), now),
            timing: active.timing,
            action: active.mesh_action,
            operator_reused: active.operator_reused,
            adapted: active.adapted,
            transferred: active.transfer.is_some(),
            exact_nodes: active
                .transfer
                .as_ref()
                .map_or(0, |transfer| transfer.exact_nodes()),
            fresh: active.fresh,
            degrees_of_freedom: active.operator.degrees_of_freedom(),
            triangles: active.mesh.triangles.len(),
            carve: active.carve,
            repair_fallback: active.repair_fallback.clone(),
        });
        self.handoff_requested = None;
        self.handoff_ready = None;
        self.handoff_upload = None;
    }
    fn refresh_runtime(
        &mut self,
        request: &mut WaveGpuRequest,
        display: &WaveDisplay,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        delta: f64,
    ) {
        // Starting another preparation mid-upload clears `runtime.ready`, and the
        // in-flight commit then fails after the GPU has already been finalised.
        // The edit is picked up on a later frame; `request_runtime` compares the
        // document revision every frame.
        if self.uploading.is_none() {
            self.request_runtime();
        }
        if let Some(Ok(_)) = self
            .runtime
            .advance_for(Self::PREPARATION_FRAME_BUDGET, 256)
        {
            self.handoff_ready = Some(Instant::now());
        }
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
                self.handoff_upload = Some(Instant::now());
                match request.update_source(
                    assets,
                    &candidate.mesh,
                    &candidate.operator,
                    candidate.point_source,
                ) {
                    Ok(()) => match self.runtime.commit_ready(token) {
                        Ok(active) => {
                            self.message = "Simulation settings committed".into();
                            self.record_handoff(&active);
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
                        self.handoff_upload = Some(Instant::now());
                        self.uploading = Some(Uploading {
                            token: candidate.bundle.token,
                            generation: request.generation(),
                            fresh: candidate.fresh,
                            time_offset,
                            degrees_of_freedom: candidate.operator.degrees_of_freedom(),
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
                && display.current.len() == upload.degrees_of_freedom
            {
                let upload = self.uploading.take().unwrap();
                // `finish_transfer` frees the old buffers, so it must not run
                // until the candidate is actually committed. Finalising first
                // would leave the GPU on a discretization `runtime.active()`
                // does not name.
                match self.runtime.commit_ready(upload.token) {
                    Ok(active) => {
                        request.finish_transfer(assets);
                        self.sim_time_offset = upload.time_offset;
                        if upload.fresh {
                            self.accumulator = 0.0;
                            self.restart_probe_traces();
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
                        self.message = "Simulation topology committed".into();
                        self.record_handoff(&active);
                    }
                    Err(error) => {
                        if request.transfer_pending() {
                            let _ = request.rollback_transfer(assets, commands);
                        }
                        self.message = error;
                    }
                }
            }
        }
        // Reset rebuilds the GPU buffers against the active topology, which bumps
        // the generation the pending commit is waiting for. That wedges the
        // handoff, and because stepping is withheld while one is pending, it
        // stops the solver for good. It stays queued instead.
        if self.reset_requested && self.uploading.is_none() {
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
                    self.restart_probe_traces();
                }
            }
        }
        // Every probe buffer belongs to the wave buffers and dies with them, so
        // the upload is repeated whenever the topology or the GPU generation
        // moves. A commit moves the token; a reset or a rolled-back transfer
        // moves the generation on its own, and used to leave the probes with
        // nothing to sample and no way back.
        // Mid-upload the buffers already belong to the candidate while the
        // active topology still names the old mesh, so the wait is the same one
        // the pulse takes below: the commit a few frames later carries both.
        if self.uploading.is_none()
            && let Some(active) = self.runtime.active().cloned()
            && probes_need_upload(self.probe_upload, active.bundle.token, request.generation())
        {
            self.configure_probes(request, assets, commands, &active);
        }
        if let Some(active) = self.runtime.active() {
            let dt = active.operator.recommended_time_step();
            // A pulse written during an upload would land in the new buffers
            // through the old operator's stencil, so it waits too.
            if let Some((position, region)) = self
                .uploading
                .is_none()
                .then(|| self.pending_pulse.take())
                .flatten()
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
            // A pending handoff can only replace the GPU buffers once the render
            // world has encoded every step already requested. Withhold both
            // continuous and manual scheduling until then, leaving the user's
            // Run/Pause preference and a pressed Step untouched.
            let handoff_pending = self.runtime.ready().is_some() || self.uploading.is_some();
            if !handoff_pending {
                if self.wave_running {
                    self.accumulator += delta.clamp(0.0, 0.1);
                    let steps = (self.accumulator / dt)
                        .floor()
                        .min(MAX_STEPS_PER_FRAME as f64) as u64;
                    if steps > 0 {
                        request.request_steps(steps);
                        self.accumulator -= steps as f64 * dt;
                    }
                    // The solver cannot encode more than one frame's worth of
                    // steps, so unspent wall-clock time is dropped instead of
                    // queued into a backlog that never drains.
                    self.accumulator = self.accumulator.min(MAX_STEPS_PER_FRAME as f64 * dt);
                } else if self.wave_step {
                    request.request_steps(1);
                    self.wave_step = false;
                }
            }
        }
        self.completed_steps = request.stats().completed_steps();
        self.gpu_status = request.stats().status();
        self.gpu_dispatches = request.stats().dispatches();
        self.step_backlog = request
            .requested_steps()
            .saturating_sub(self.completed_steps);
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
        // The recorder's history is a time series at fixed world points, so a
        // new mesh over the same contour takes it over. Only a clock that
        // restarted, or a contour that moved, starts the delay window again.
        let clock = FarFieldClock {
            origin: self.sim_time_offset,
            keep_history: !std::mem::take(&mut self.probe_clock_restarted),
        };
        match request.update_far_field(assets, commands, far.as_ref(), dt, clock) {
            Ok(FarFieldHandoff::Restarted) => {
                self.far_field_trace = FarFieldTrace::default();
                self.far_field_recording_from = Some(self.sim_time_offset);
            }
            Ok(FarFieldHandoff::Off) => self.far_field_recording_from = None,
            Ok(FarFieldHandoff::Kept) => {}
            Err(error) => {
                self.far_field_recording_from = None;
                self.message = error;
            }
        }
        // Recorded whatever happened: a rejected recorder setting is rejected
        // the same way every frame, and the readback filter below keys on these
        // revisions, so what the GPU actually holds is what is written down.
        self.probe_upload = Some(ProbeUpload {
            token: active.bundle.token,
            generation: request.generation(),
            revision: request.probe_revision(),
            curve_revision: request.curve_probe_revision(),
            area_revision: request.area_probe_revision(),
            far_field_revision: request.far_field_revision(),
        });
    }
    /// The simulation clock is starting over at zero. Ingestion only takes a
    /// record newer than the trace's last one, so a trace carried across the
    /// restart would refuse the entire new run — which is what made a reset
    /// look like it had frozen the probes until the traces were cleared by hand.
    fn restart_probe_traces(&mut self) {
        self.probe_traces.clear();
        self.curve_probe_traces.clear();
        self.area_probe_traces.clear();
        self.far_field_trace = FarFieldTrace::default();
        self.probe_clock_restarted = true;
    }
    fn simulated_time(&self) -> f64 {
        let dt = self
            .runtime
            .active()
            .map_or(0.0, |active| active.operator.recommended_time_step());
        self.sim_time_offset + self.completed_steps as f64 * dt
    }
    /// How much of the delay window the far-field recorder holds, once it is
    /// running and has not filled it yet. Nothing can be projected before it is
    /// whole, and the empty plot says nothing on its own.
    fn far_field_recording(&self) -> Option<f64> {
        let history = self
            .runtime
            .active()?
            .far_field
            .as_ref()?
            .as_ref()
            .ok()?
            .history_seconds();
        let from = self.far_field_recording_from?;
        let recorded = (self.simulated_time() - from) / history;
        (history > 0.0 && recorded < 1.0).then(|| recorded.clamp(0.0, 1.0))
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
        // An adaptation spans many frames while the user may remesh underneath
        // it. Its result is only meaningful against the mesh it started from,
        // so once another mesh is active the job is dropped here instead of
        // finishing and being rejected at the handoff as an error.
        if self.amr_adaptation_job.is_some()
            && self.amr_adaptation_source.is_some_and(|source| {
                self.runtime
                    .active()
                    .is_none_or(|active| active.mesh.mesh_revision != source)
            })
        {
            self.amr_adaptation_job = None;
            self.amr_adaptation_source = None;
            self.amr_pending_state = None;
            self.amr_status = "adaptation discarded: the mesh changed underneath it".into();
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
            self.amr_adaptation_source = None;
            match result {
                Ok(result) => {
                    self.amr_pending_state = Some(result.state);
                    self.amr_report = Some(result.report);
                    match self.runtime.request_adapted(
                        self.editor.revision,
                        &self.editor.document,
                        result.mesh,
                    ) {
                        Ok(_) => {
                            self.amr_status = "preparing adaptive handoff".into();
                            self.amr_error = None;
                            self.begin_handoff_timeline();
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
            self.amr_adaptation_source = Some(active.mesh.mesh_revision);
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
    /// A readback carries the generation and the revision it was recorded
    /// against. Anything else is a ring the app no longer owns — the tail of the
    /// run before a reset, or of the probe set before an edit — and its times
    /// belong to a clock the traces have left behind.
    fn readback_is_current(
        &self,
        generation: u64,
        revision: u64,
        recorded: impl Fn(&ProbeUpload) -> u64,
    ) -> bool {
        self.probe_upload
            .as_ref()
            .is_some_and(|upload| upload.generation == generation && recorded(upload) == revision)
    }
    fn ingest_probes(&mut self, display: &ProbeDisplay) {
        if display.readbacks == self.probe_readback
            || !self.readback_is_current(display.generation, display.revision, |upload| {
                upload.revision
            })
        {
            return;
        }
        self.probe_readback = display.readbacks;
        for record in &display.records {
            let trace = self
                .probe_traces
                .entry(ProbeId(record.probe_id))
                .or_default();
            let mut record = *record;
            record.time += self.sim_time_offset;
            if record.time > trace.last_time {
                trace.last_time = record.time;
                trace.samples.push_back(record);
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
        if curves.readbacks != self.curve_probe_readback
            && self.readback_is_current(curves.generation, curves.revision, |upload| {
                upload.curve_revision
            })
        {
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
        if areas.readbacks != self.area_probe_readback
            && self.readback_is_current(areas.generation, areas.revision, |upload| {
                upload.area_revision
            })
        {
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
        if far.readbacks != self.far_field_readback
            && self.readback_is_current(far.generation, far.revision, |upload| {
                upload.far_field_revision
            })
        {
            self.far_field_readback = far.readbacks;
            for record in &far.records {
                // Stamped on the app's clock by the recorder, which has to hold
                // one across the mesh swaps its ring survives.
                let record = record.clone();
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
    /// The frame a region-local material profile or volume source is written in,
    /// shown only while Materials is open and something actually uses it.
    fn selected_material_frame(&self) -> Option<(RegionId, MaterialFrame)> {
        if self.inspector != Some(InspectorPanel::Materials) {
            return None;
        }
        let scene = &self.editor.document.model.draft;
        let region = scene.region(self.region_selection)?;
        let material_uses = scene
            .material(region.material)
            .is_some_and(Material::uses_frame);
        let source_uses = scene
            .volume_sources
            .iter()
            .any(|source| source.region == region.id && source.enabled && source.varying());
        (material_uses || source_uses).then_some((region.id, region.frame))
    }
    fn hit_material_frame_gizmo(&self, point: Pos2, r: Rect) -> Option<MaterialFrameGizmoHit> {
        let (_, frame) = self.selected_material_frame()?;
        let distance = self.screen(frame.origin, r).distance(point);
        if distance <= self.hit_tolerance(11.0) {
            Some(MaterialFrameGizmoHit::Origin)
        } else if (distance - MATERIAL_FRAME_RADIUS).abs() <= self.hit_tolerance(9.0) {
            Some(MaterialFrameGizmoHit::Rotate)
        } else {
            None
        }
    }
    fn draw_material_frame(&self, painter: &egui::Painter, r: Rect) {
        if self.capturing() {
            return;
        }
        let Some((_, frame)) = self.selected_material_frame() else {
            return;
        };
        let center = self.screen(frame.origin, r);
        let (sin, cos) = frame.angle_radians.sin_cos();
        let x_end = center + egui::vec2((cos * 33.0) as f32, (-sin * 33.0) as f32);
        let y_end = center + egui::vec2((-sin * 27.0) as f32, (-cos * 27.0) as f32);
        painter.circle_stroke(center, MATERIAL_FRAME_RADIUS, Stroke::new(1.5, TEAL));
        painter.circle_filled(
            center
                + egui::vec2(
                    cos as f32 * MATERIAL_FRAME_RADIUS,
                    -sin as f32 * MATERIAL_FRAME_RADIUS,
                ),
            4.5,
            TEAL,
        );
        painter.line_segment([center, x_end], Stroke::new(2.0, RED));
        painter.line_segment([center, y_end], Stroke::new(2.0, TEAL));
        painter.circle_filled(center, 6.0, Color32::from_rgb(16, 23, 31));
        painter.circle_stroke(center, 6.0, Stroke::new(2.0, GOLD));
        for (end, label, color) in [(x_end, "x", RED), (y_end, "y", TEAL)] {
            painter.text(
                end,
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::monospace(11.0),
                color,
            );
        }
    }
    /// Screen-space hit radius. Touch input keeps the drawn controls small but
    /// widens what counts as a hit.
    /// Small grips on the outer rectangle's corners, so the resize is something
    /// the user can see rather than have to know about. Hidden while a gesture
    /// that has nothing to do with the domain is running.
    fn draw_domain_handles(&self, painter: &egui::Painter, r: Rect) {
        if self.capturing()
            || self.draw.is_some()
            || self.pending_removal.is_some()
            || matches!(
                self.drag,
                Some(
                    DragGesture::Marquee { .. }
                        | DragGesture::Spans { .. }
                        | DragGesture::Rotate { .. }
                        | DragGesture::Scale { .. }
                )
            )
        {
            return;
        }
        let dragging = matches!(self.drag, Some(DragGesture::Domain { .. }));
        for (index, corner) in self
            .editor
            .document
            .model
            .draft
            .geometry
            .domain
            .corners()
            .into_iter()
            .enumerate()
        {
            let center = self.screen(corner, r);
            if !r.contains(center) {
                continue;
            }
            let held = dragging
                && matches!(
                    self.drag,
                    Some(DragGesture::Domain {
                        drag: DomainDrag::Corner { index: held, .. },
                    }) if held == index
                );
            let half = if held { 5.0 } else { 4.0 };
            let rect = egui::Rect::from_center_size(center, egui::vec2(half * 2.0, half * 2.0));
            painter.rect_filled(rect, 1.0, Color32::from_rgba_unmultiplied(8, 13, 18, 220));
            painter.rect_stroke(
                rect,
                1.0,
                Stroke::new(1.0, if held { GOLD } else { TEAL }),
                egui::StrokeKind::Outside,
            );
        }
    }

    /// The outer rectangle's corner under the pointer, if any. Corners win over
    /// the sides they meet, so a corner drag is always reachable.
    fn hit_domain_corner(&self, point: Pos2, viewport: Rect) -> Option<usize> {
        self.editor
            .document
            .model
            .draft
            .geometry
            .domain
            .corners()
            .into_iter()
            .enumerate()
            .map(|(index, corner)| (index, self.screen(corner, viewport).distance(point)))
            .filter(|(_, distance)| *distance <= self.hit_tolerance(10.0))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    }

    fn domain_corner_cursor(index: usize) -> egui::CursorIcon {
        match index {
            0 | 2 => egui::CursorIcon::ResizeNeSw,
            _ => egui::CursorIcon::ResizeNwSe,
        }
    }

    /// Where the dragged side or corner lands, leaving the rest of the rectangle
    /// where it was.
    fn resize_domain(start: DomainRect, drag: DomainDrag, point: Point2) -> DomainRect {
        let mut domain = start;
        match drag {
            DomainDrag::Side { side, .. } => match side {
                OuterSide::Bottom => domain.min_y = point.y,
                OuterSide::Right => domain.max_x = point.x,
                OuterSide::Top => domain.max_y = point.y,
                OuterSide::Left => domain.min_x = point.x,
            },
            DomainDrag::Corner { index, .. } => match index {
                0 => {
                    domain.min_x = point.x;
                    domain.min_y = point.y;
                }
                1 => {
                    domain.max_x = point.x;
                    domain.min_y = point.y;
                }
                2 => {
                    domain.max_x = point.x;
                    domain.max_y = point.y;
                }
                _ => {
                    domain.min_x = point.x;
                    domain.max_y = point.y;
                }
            },
        }
        domain
    }

    fn hit_tolerance(&self, mouse: f32) -> f32 {
        if self.touch_active {
            mouse.max(18.0)
        } else {
            mouse
        }
    }
    fn probe_visible(&self, target: &TopologyProbeTarget) -> bool {
        let presentation = self.editor.document.presentation;
        match target {
            TopologyProbeTarget::Point(_) => presentation.point_probes,
            TopologyProbeTarget::Segment { .. } => presentation.line_probes,
            TopologyProbeTarget::Boundary(_) => presentation.boundary_probes,
            TopologyProbeTarget::AreaDisk { .. } | TopologyProbeTarget::AreaRegion(_) => {
                presentation.area_probes
            }
        }
    }
    /// Colour rule shared by markers and labels: a probe that failed to compile
    /// reads red, one that is not recording reads grey.
    fn probe_color(&self, probe: &TopologyProbeDefinition) -> Color32 {
        if self.probe_status.contains_key(&probe.id) {
            RED
        } else if !probe.enabled {
            Color32::from_rgb(112, 130, 143)
        } else {
            Color32::from_rgb(probe.color[0], probe.color[1], probe.color[2])
        }
    }
    /// The drawn path of a boundary probe, in the target's own span order.
    fn boundary_probe_polyline(&self, target: &TopologyBoundaryProbeTarget) -> Vec<Point2> {
        let Some(sampled) = &self.sampled else {
            return vec![];
        };
        let mut path = Vec::new();
        for span in &target.spans {
            let Some(samples) = sampled
                .spans
                .iter()
                .find(|candidate| candidate.target == TopologySpanTarget::Curve(*span))
            else {
                continue;
            };
            for sample in &samples.samples {
                if path.last() != Some(&sample.point) {
                    path.push(sample.point);
                }
            }
        }
        path
    }
    /// Point half way along a polyline by arclength, used to badge and hit the
    /// probe where the eye expects it rather than at an arbitrary end.
    fn polyline_midpoint(path: &[Point2]) -> Option<Point2> {
        let total: f64 = path.windows(2).map(|pair| (pair[1] - pair[0]).norm()).sum();
        let mut remaining = total * 0.5;
        for pair in path.windows(2) {
            let length = (pair[1] - pair[0]).norm();
            if remaining <= length {
                let fraction = if length > 0.0 {
                    remaining / length
                } else {
                    0.0
                };
                return Some(pair[0].lerp(pair[1], fraction));
            }
            remaining -= length;
        }
        path.first().copied()
    }
    fn probe_badge(&self, probe: &TopologyProbeDefinition) -> Option<Point2> {
        match &probe.target {
            TopologyProbeTarget::Point(point) => Some(*point),
            TopologyProbeTarget::Segment { start, end, .. } => Some(start.lerp(*end, 0.5)),
            TopologyProbeTarget::Boundary(target) => {
                Self::polyline_midpoint(&self.boundary_probe_polyline(target))
            }
            TopologyProbeTarget::AreaDisk { center, .. } => Some(*center),
            TopologyProbeTarget::AreaRegion(_) => self.probe_anchors.get(&probe.id).copied(),
        }
    }
    /// Topmost probe under the pointer. Later probes win, matching the draw order.
    fn hit_probe(&self, point: Pos2, viewport: Rect) -> Option<ProbeHit> {
        self.editor
            .document
            .model
            .probes
            .iter()
            .rev()
            .filter(|probe| self.probe_visible(&probe.target))
            .find_map(|probe| match &probe.target {
                TopologyProbeTarget::Point(position) => {
                    (self.screen(*position, viewport).distance(point) <= self.hit_tolerance(13.0))
                        .then_some(ProbeHit::Point(probe.id))
                }
                TopologyProbeTarget::Segment { start, end, .. } => {
                    let a = self.screen(*start, viewport);
                    let b = self.screen(*end, viewport);
                    if a.distance(point) <= self.hit_tolerance(13.0) {
                        Some(ProbeHit::SegmentEndpoint(probe.id, true))
                    } else if b.distance(point) <= self.hit_tolerance(13.0) {
                        Some(ProbeHit::SegmentEndpoint(probe.id, false))
                    } else if screen_segment_distance(point, a, b) <= self.hit_tolerance(9.0) {
                        Some(ProbeHit::SegmentBody(probe.id))
                    } else {
                        None
                    }
                }
                TopologyProbeTarget::Boundary(target) => {
                    let badge = Self::polyline_midpoint(&self.boundary_probe_polyline(target))?;
                    (self.screen(badge, viewport).distance(point) <= self.hit_tolerance(13.0))
                        .then_some(ProbeHit::Boundary(probe.id))
                }
                TopologyProbeTarget::AreaDisk { center, radius } => {
                    let center = self.screen(*center, viewport);
                    let radius = (radius * self.scale) as f32;
                    let handle = center + egui::vec2(radius, 0.0);
                    if handle.distance(point) <= self.hit_tolerance(13.0) {
                        Some(ProbeHit::AreaDiskRadius(probe.id))
                    } else if center.distance(point) <= radius + self.hit_tolerance(9.0) {
                        Some(ProbeHit::AreaDiskBody(probe.id))
                    } else {
                        None
                    }
                }
                TopologyProbeTarget::AreaRegion(_) => {
                    let anchor = self.probe_anchors.get(&probe.id).copied()?;
                    (self.screen(anchor, viewport).distance(point) <= self.hit_tolerance(15.0))
                        .then_some(ProbeHit::AreaRegion(probe.id))
                }
            })
    }
    /// Applies a probe drag to the target captured when the gesture started, so
    /// repeated updates stay exact instead of accumulating rounding.
    fn snap_scalar(value: f64) -> f64 {
        const STEP: f64 = 0.05;
        (value / STEP).round() * STEP
    }

    fn drag_probe(
        &mut self,
        hit: ProbeHit,
        original: &TopologyProbeTarget,
        delta: Point2,
        snap: bool,
    ) {
        let place = |point: Point2| {
            if snap {
                Self::snap_point(point + delta)
            } else {
                point + delta
            }
        };
        let Some(mut probe) = self
            .editor
            .document
            .model
            .probes
            .iter()
            .find(|probe| probe.id == hit.id())
            .cloned()
        else {
            return;
        };
        probe.target = original.clone();
        match (hit, &mut probe.target) {
            (ProbeHit::Point(_), TopologyProbeTarget::Point(position)) => {
                *position = place(*position);
            }
            (
                ProbeHit::SegmentEndpoint(_, first),
                TopologyProbeTarget::Segment { start, end, .. },
            ) => {
                let endpoint = if first { start } else { end };
                *endpoint = place(*endpoint);
            }
            (ProbeHit::SegmentBody(_), TopologyProbeTarget::Segment { start, end, .. }) => {
                // Translate rigidly, putting the start on the grid.
                let shift = place(*start) - *start;
                *start = *start + shift;
                *end = *end + shift;
            }
            (ProbeHit::AreaDiskBody(_), TopologyProbeTarget::AreaDisk { center, .. }) => {
                *center = place(*center);
            }
            (ProbeHit::AreaDiskRadius(_), TopologyProbeTarget::AreaDisk { center, radius }) => {
                let grown = *radius + delta.x;
                *radius = if snap {
                    Self::snap_scalar(grown)
                } else {
                    grown
                }
                .max(1.0e-3);
                let _ = center;
            }
            _ => return,
        }
        if let Err(error) = self.editor.update_probe_during_edit(probe) {
            self.message = error;
        }
    }
    /// Mirrors the committed compilation status and path metrics of every probe
    /// so the readouts can report a precise reason and a real arclength axis.
    fn refresh_probe_metadata(&mut self) {
        let ids = self
            .editor
            .document
            .model
            .probes
            .iter()
            .map(|probe| probe.id)
            .collect::<BTreeSet<_>>();
        self.probe_status.retain(|id, _| ids.contains(id));
        self.probe_metrics.retain(|id, _| ids.contains(id));
        self.probe_traces.retain(|id, _| ids.contains(id));
        self.curve_probe_traces.retain(|id, _| ids.contains(id));
        self.area_probe_traces.retain(|id, _| ids.contains(id));
        self.probe_views.retain(|id, _| ids.contains(id));
        self.probe_windows.retain(|id| ids.contains(id));
        self.probe_anchors.retain(|id, _| ids.contains(id));
        let Some(active) = self.runtime.active().cloned() else {
            return;
        };
        // Path metrics and region anchors are derived from the committed mesh,
        // so they only change when a new candidate or probe set is published.
        let token = (active.bundle.token, self.editor.revision);
        if self.probe_metadata_token == Some(token) {
            return;
        }
        self.probe_metadata_token = Some(token);
        self.probe_anchors.clear();
        for compiled in active.probes.iter() {
            match &compiled.result {
                TopologyProbeCompilation::Ready(stencil) => {
                    self.probe_status.remove(&compiled.id);
                    if let TopologyProbeStencil::Boundary(samples) = stencil.as_ref() {
                        let closed = self.boundary_probe_is_closed(compiled.id);
                        let mut length = 0.0;
                        for pair in samples.windows(2) {
                            length += (pair[1].point - pair[0].point).norm();
                        }
                        if closed
                            && let (Some(first), Some(last)) = (samples.first(), samples.last())
                        {
                            length += (first.point - last.point).norm();
                        }
                        self.probe_metrics.insert(compiled.id, (length, closed));
                    }
                }
                TopologyProbeCompilation::Failed(reason) => {
                    self.probe_status.insert(compiled.id, reason.clone());
                }
                TopologyProbeCompilation::Disabled => {
                    self.probe_status
                        .insert(compiled.id, "Recording is off".into());
                }
            }
        }
        for probe in &self.editor.document.model.probes {
            match probe.target {
                TopologyProbeTarget::Segment { start, end, .. } => {
                    self.probe_metrics
                        .insert(probe.id, ((end - start).norm(), false));
                }
                TopologyProbeTarget::AreaRegion(region) => {
                    if let Some(point) = region_anchor(&active.mesh, region) {
                        self.probe_anchors.insert(probe.id, point);
                    }
                }
                _ => {}
            }
        }
    }
    /// A boundary probe covering every span of a periodic curve samples a closed
    /// loop, so its last sample joins its first when integrating.
    fn boundary_probe_is_closed(&self, id: ProbeId) -> bool {
        let Some(TopologyProbeTarget::Boundary(target)) = self
            .editor
            .document
            .model
            .probes
            .iter()
            .find(|probe| probe.id == id)
            .map(|probe| &probe.target)
        else {
            return false;
        };
        self.editor
            .document
            .model
            .accepted
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == target.curve)
            .is_some_and(|curve| {
                !curve.spline.is_open()
                    && curve
                        .spans
                        .iter()
                        .all(|span| target.spans.contains(&span.id))
            })
    }
    fn clear_probe_trace(&mut self, id: ProbeId) {
        self.probe_traces.remove(&id);
        self.curve_probe_traces.remove(&id);
        self.area_probe_traces.remove(&id);
    }
    fn probe_time_window(
        samples: &[PointProbeRecord],
        view: &mut ProbeViewState,
    ) -> Option<(f64, f64)> {
        let first = samples.first()?.time;
        let last = samples.last()?.time;
        let available = last - first;
        if !available.is_finite() || available <= f64::EPSILON {
            return None;
        }
        let span = view.span.min(available);
        if view.live {
            view.end_time = last;
        }
        view.end_time = view.end_time.clamp(first + span, last);
        Some((view.end_time - span, view.end_time))
    }
    /// One pannable, zoomable time trace. Every trace in a readout shares the
    /// window held by `view`, so they stay aligned while the user navigates.
    fn probe_plot(
        ui: &mut egui::Ui,
        label: &str,
        samples: &[PointProbeRecord],
        value: impl Fn(&PointProbeRecord) -> f64,
        color: Color32,
        view: &mut ProbeViewState,
        maximum_span: f64,
    ) {
        ui.small(label);
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().max(120.0), 92.0),
            egui::Sense::drag(),
        );
        ui.painter()
            .rect_filled(rect, 2.0, Color32::from_rgb(12, 18, 24));
        ui.painter().rect_stroke(
            rect,
            2.0,
            Stroke::new(1.0, Color32::from_rgb(55, 69, 80)),
            egui::StrokeKind::Inside,
        );
        let waiting = |painter: &egui::Painter| {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Waiting for samples",
                egui::FontId::monospace(11.0),
                Color32::from_rgb(112, 130, 143),
            );
        };
        let Some((mut minimum_time, mut maximum_time)) = Self::probe_time_window(samples, view)
        else {
            waiting(ui.painter());
            return;
        };
        let visible_span = maximum_time - minimum_time;
        if response.drag_started() {
            view.live = false;
        }
        if response.dragged() {
            let delta = response.drag_delta().x as f64;
            view.end_time -= delta / rect.width() as f64 * visible_span;
        }
        if response.hovered() {
            let wheel = ui.ctx().input(|input| input.smooth_scroll_delta.y);
            if wheel != 0.0 {
                let fraction = response.hover_pos().map_or(0.5, |position| {
                    ((position.x - rect.left()) / rect.width()).clamp(0.0, 1.0)
                }) as f64;
                let anchored_time = minimum_time + fraction * visible_span;
                view.live = false;
                view.span = (visible_span * (-wheel as f64 * 0.01).exp()).clamp(0.02, maximum_span);
                let first = samples.first().unwrap().time;
                let last = samples.last().unwrap().time;
                let zoomed_span = view.span.min(last - first);
                let fullest_span = maximum_span.min(last - first);
                if view.span >= fullest_span * (1.0 - 1.0e-9) {
                    view.live = true;
                    view.end_time = last;
                } else {
                    view.end_time = anchored_time + (1.0 - fraction) * zoomed_span;
                }
            }
        }
        (minimum_time, maximum_time) = Self::probe_time_window(samples, view).unwrap();
        let visible = samples
            .iter()
            .filter(|sample| sample.time >= minimum_time && sample.time <= maximum_time)
            .collect::<Vec<_>>();
        if visible.len() < 2 {
            waiting(ui.painter());
            return;
        }
        let mut minimum = f64::INFINITY;
        let mut maximum = f64::NEG_INFINITY;
        for sample in &visible {
            let sample = value(sample);
            minimum = minimum.min(sample);
            maximum = maximum.max(sample);
        }
        if !minimum.is_finite() || !maximum.is_finite() {
            return;
        }
        if (maximum - minimum).abs() < 1.0e-15 {
            let padding = maximum.abs().max(1.0) * 0.05;
            minimum -= padding;
            maximum += padding;
        }
        let time_span = (maximum_time - minimum_time).max(f64::MIN_POSITIVE);
        let value_span = maximum - minimum;
        let points = visible
            .iter()
            .map(|sample| {
                egui::pos2(
                    egui::lerp(
                        rect.left()..=rect.right(),
                        ((sample.time - minimum_time) / time_span) as f32,
                    ),
                    egui::lerp(
                        rect.bottom()..=rect.top(),
                        ((value(sample) - minimum) / value_span) as f32,
                    ),
                )
            })
            .collect();
        ui.painter()
            .add(egui::Shape::line(points, Stroke::new(1.4, color)));
        ui.painter().text(
            rect.left_top() + egui::vec2(4.0, 3.0),
            egui::Align2::LEFT_TOP,
            format!("{maximum:+.3e}"),
            egui::FontId::monospace(9.0),
            Color32::from_rgb(142, 161, 175),
        );
        ui.painter().text(
            rect.left_bottom() + egui::vec2(4.0, -3.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{minimum:+.3e}"),
            egui::FontId::monospace(9.0),
            Color32::from_rgb(142, 161, 175),
        );
        response.on_hover_text(format!(
            "Simulation time {minimum_time:.4}–{maximum_time:.4}"
        ));
    }
    fn curve_probe_values(frame: &CurveProbeRecord, quantity: LineProbeQuantity) -> &[f32] {
        match quantity {
            LineProbeQuantity::Field => &frame.displacement,
            LineProbeQuantity::Transverse => &frame.transverse_magnitude,
            LineProbeQuantity::Flux => &frame.normal_flux,
            LineProbeQuantity::Energy => &frame.energy_density,
        }
    }
    /// Trapezoidal integral along the sampled path, plus the fraction of the
    /// intervals that carried finite values.
    fn curve_probe_integral(
        frame: &CurveProbeRecord,
        length: f64,
        quantity: LineProbeQuantity,
        closed: bool,
    ) -> (f64, f64) {
        let samples = Self::curve_probe_values(frame, quantity);
        let intervals = if closed {
            samples.len()
        } else {
            samples.len().saturating_sub(1)
        };
        if intervals == 0 || !length.is_finite() {
            return (f64::NAN, 0.0);
        }
        let mut integral = 0.0;
        let mut valid = 0usize;
        for index in 0..intervals {
            let a = samples[index] as f64;
            let b = samples[(index + 1) % samples.len()] as f64;
            if a.is_finite() && b.is_finite() {
                integral += 0.5 * (a + b);
                valid += 1;
            }
        }
        if valid == 0 {
            return (f64::NAN, 0.0);
        }
        (
            integral * length / intervals as f64,
            valid as f64 / intervals as f64,
        )
    }
    fn curve_probe_profile(
        ui: &mut egui::Ui,
        frames: &[CurveProbeRecord],
        view: &ProbeViewState,
        quantity: LineProbeQuantity,
        length: f64,
        physics: PhysicsModel,
    ) {
        ui.small(format!("{} vs arclength", quantity.label_for(physics)));
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().max(120.0), 110.0),
            egui::Sense::hover(),
        );
        ui.painter()
            .rect_filled(rect, 2.0, Color32::from_rgb(12, 18, 24));
        let Some(frame) = frames.iter().min_by(|a, b| {
            (a.time - view.end_time)
                .abs()
                .total_cmp(&(b.time - view.end_time).abs())
        }) else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Waiting for samples",
                egui::FontId::monospace(11.0),
                Color32::from_rgb(112, 130, 143),
            );
            return;
        };
        let samples = Self::curve_probe_values(frame, quantity);
        let mut minimum = samples
            .iter()
            .copied()
            .filter(|value| value.is_finite())
            .fold(f32::INFINITY, f32::min);
        let mut maximum = samples
            .iter()
            .copied()
            .filter(|value| value.is_finite())
            .fold(f32::NEG_INFINITY, f32::max);
        if !minimum.is_finite() || !maximum.is_finite() {
            return;
        }
        if (maximum - minimum).abs() < 1.0e-12 {
            let padding = maximum.abs().max(1.0) * 0.05;
            minimum -= padding;
            maximum += padding;
        }
        if minimum <= 0.0 && maximum >= 0.0 {
            let zero = egui::lerp(
                rect.bottom()..=rect.top(),
                (0.0 - minimum) / (maximum - minimum),
            );
            ui.painter().line_segment(
                [
                    egui::pos2(rect.left(), zero),
                    egui::pos2(rect.right(), zero),
                ],
                Stroke::new(1.0, Color32::from_rgb(45, 57, 67)),
            );
        }
        let count = samples.len().max(2);
        let mut run = Vec::new();
        for (index, value) in samples.iter().copied().enumerate() {
            if value.is_finite() {
                run.push(egui::pos2(
                    egui::lerp(
                        rect.left()..=rect.right(),
                        index as f32 / (count - 1) as f32,
                    ),
                    egui::lerp(
                        rect.bottom()..=rect.top(),
                        (value - minimum) / (maximum - minimum),
                    ),
                ));
            } else if run.len() >= 2 {
                ui.painter().add(egui::Shape::line(
                    std::mem::take(&mut run),
                    Stroke::new(1.5, quantity.color()),
                ));
            } else {
                run.clear();
            }
        }
        if run.len() >= 2 {
            ui.painter()
                .add(egui::Shape::line(run, Stroke::new(1.5, quantity.color())));
        }
        ui.painter().text(
            rect.left_top() + egui::vec2(4.0, 3.0),
            egui::Align2::LEFT_TOP,
            format!("{maximum:+.3e}"),
            egui::FontId::monospace(9.0),
            Color32::from_rgb(142, 161, 175),
        );
        ui.painter().text(
            rect.left_bottom() + egui::vec2(4.0, -3.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{minimum:+.3e} · s=0"),
            egui::FontId::monospace(9.0),
            Color32::from_rgb(142, 161, 175),
        );
        ui.painter().text(
            rect.right_bottom() + egui::vec2(-4.0, -3.0),
            egui::Align2::RIGHT_BOTTOM,
            format!("s={length:.3}"),
            egui::FontId::monospace(9.0),
            Color32::from_rgb(142, 161, 175),
        );
    }
    fn curve_probe_waterfall(
        ui: &mut egui::Ui,
        frames: &[CurveProbeRecord],
        times: &[PointProbeRecord],
        view: &mut ProbeViewState,
        maximum_span: f64,
        quantity: LineProbeQuantity,
        physics: PhysicsModel,
    ) {
        ui.small(format!("{} waterfall", quantity.label_for(physics)));
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().max(120.0), 170.0),
            egui::Sense::drag(),
        );
        ui.painter()
            .rect_filled(rect, 2.0, Color32::from_rgb(12, 18, 24));
        let Some((minimum_time, maximum_time)) = Self::probe_time_window(times, view) else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Waiting for samples",
                egui::FontId::monospace(11.0),
                Color32::from_rgb(112, 130, 143),
            );
            return;
        };
        let span = maximum_time - minimum_time;
        if response.drag_started() {
            view.live = false;
        }
        if response.dragged() {
            view.end_time += response.drag_delta().y as f64 / rect.height() as f64 * span;
        }
        if response.hovered() {
            let wheel = ui.ctx().input(|input| input.smooth_scroll_delta.y);
            if wheel != 0.0 {
                view.live = false;
                view.span = (span * (-wheel as f64 * 0.01).exp()).clamp(0.02, maximum_span);
            }
        }
        let Some((minimum_time, maximum_time)) = Self::probe_time_window(times, view) else {
            return;
        };
        let visible = frames
            .iter()
            .filter(|frame| frame.time >= minimum_time && frame.time <= maximum_time)
            .collect::<Vec<_>>();
        let maximum = visible
            .iter()
            .flat_map(|frame| Self::curve_probe_values(frame, quantity).iter())
            .copied()
            .filter(|value| value.is_finite())
            .map(f32::abs)
            .fold(0.0, f32::max)
            .max(1.0e-12)
            / view.waterfall_gain;
        for (row, frame) in visible.iter().enumerate() {
            let samples = Self::curve_probe_values(frame, quantity);
            let count = samples.len();
            if count == 0 {
                continue;
            }
            let top = egui::lerp(
                rect.bottom()..=rect.top(),
                (row + 1) as f32 / visible.len() as f32,
            );
            let bottom = egui::lerp(
                rect.bottom()..=rect.top(),
                row as f32 / visible.len() as f32,
            );
            for (column, value) in samples.iter().copied().enumerate() {
                if !value.is_finite() {
                    continue;
                }
                let normalized = (value / maximum).clamp(-1.0, 1.0);
                let color = waterfall_color(normalized, quantity == LineProbeQuantity::Energy);
                let left = egui::lerp(rect.left()..=rect.right(), column as f32 / count as f32);
                let right = egui::lerp(
                    rect.left()..=rect.right(),
                    (column + 1) as f32 / count as f32,
                );
                ui.painter().rect_filled(
                    Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, bottom)),
                    0.0,
                    color,
                );
            }
        }
        ui.painter().text(
            rect.left_top() + egui::vec2(4.0, 3.0),
            egui::Align2::LEFT_TOP,
            format!("t={maximum_time:.3}"),
            egui::FontId::monospace(9.0),
            Color32::WHITE,
        );
        ui.painter().text(
            rect.left_bottom() + egui::vec2(4.0, -3.0),
            egui::Align2::LEFT_BOTTOM,
            format!("t={minimum_time:.3} · s=0"),
            egui::FontId::monospace(9.0),
            Color32::WHITE,
        );
        ui.painter().text(
            rect.right_bottom() + egui::vec2(-4.0, -3.0),
            egui::Align2::RIGHT_BOTTOM,
            "s=L",
            egui::FontId::monospace(9.0),
            Color32::WHITE,
        );
    }
    fn far_field_waterfall(
        ui: &mut egui::Ui,
        frames: &[FarFieldRecord],
        times: &[PointProbeRecord],
        view: &mut ProbeViewState,
        maximum_span: f64,
    ) {
        ui.small("Direction × time");
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().max(120.0), 170.0),
            egui::Sense::drag(),
        );
        ui.painter()
            .rect_filled(rect, 2.0, Color32::from_rgb(12, 18, 24));
        let Some((minimum_time, maximum_time)) = Self::probe_time_window(times, view) else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Warming up the propagation delay",
                egui::FontId::monospace(11.0),
                Color32::from_rgb(112, 130, 143),
            );
            return;
        };
        let span = maximum_time - minimum_time;
        if response.drag_started() {
            view.live = false;
        }
        if response.dragged() {
            view.end_time += response.drag_delta().y as f64 / rect.height() as f64 * span;
        }
        if response.hovered() {
            let wheel = ui.ctx().input(|input| input.smooth_scroll_delta.y);
            if wheel != 0.0 {
                view.live = false;
                view.span = (span * (-wheel as f64 * 0.01).exp()).clamp(0.02, maximum_span);
                if view.span >= maximum_span * (1.0 - 1.0e-9) {
                    view.live = true;
                }
            }
        }
        let Some((minimum_time, maximum_time)) = Self::probe_time_window(times, view) else {
            return;
        };
        let visible = frames
            .iter()
            .filter(|frame| frame.time >= minimum_time && frame.time <= maximum_time)
            .collect::<Vec<_>>();
        let maximum = visible
            .iter()
            .flat_map(|frame| frame.amplitude.iter())
            .copied()
            .filter(|value| value.is_finite())
            .map(f32::abs)
            .fold(0.0, f32::max)
            .max(1.0e-12)
            / view.waterfall_gain;
        for (row, frame) in visible.iter().enumerate() {
            let count = frame.amplitude.len();
            if count == 0 {
                continue;
            }
            let top = egui::lerp(
                rect.bottom()..=rect.top(),
                (row + 1) as f32 / visible.len() as f32,
            );
            let bottom = egui::lerp(
                rect.bottom()..=rect.top(),
                row as f32 / visible.len() as f32,
            );
            for (column, value) in frame.amplitude.iter().copied().enumerate() {
                if !value.is_finite() {
                    continue;
                }
                let color = waterfall_color((value / maximum).clamp(-1.0, 1.0), false);
                let left = egui::lerp(rect.left()..=rect.right(), column as f32 / count as f32);
                let right = egui::lerp(
                    rect.left()..=rect.right(),
                    (column + 1) as f32 / count as f32,
                );
                ui.painter().rect_filled(
                    Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, bottom)),
                    0.0,
                    color,
                );
            }
        }
        ui.painter().text(
            rect.left_top() + egui::vec2(4.0, 3.0),
            egui::Align2::LEFT_TOP,
            format!("t={maximum_time:.3}"),
            egui::FontId::monospace(9.0),
            Color32::WHITE,
        );
        ui.painter().text(
            rect.left_bottom() + egui::vec2(4.0, -3.0),
            egui::Align2::LEFT_BOTTOM,
            format!("t={minimum_time:.3} · 0°"),
            egui::FontId::monospace(9.0),
            Color32::WHITE,
        );
        ui.painter().text(
            rect.right_bottom() + egui::vec2(-4.0, -3.0),
            egui::Align2::RIGHT_BOTTOM,
            "360°",
            egui::FontId::monospace(9.0),
            Color32::WHITE,
        );
    }
    /// Mean intensity over exactly the window the other far-field traces show.
    fn far_field_average(
        frames: &[FarFieldRecord],
        times: &[PointProbeRecord],
        view: &ProbeViewState,
    ) -> Vec<f32> {
        let mut window = view.clone();
        let Some((minimum_time, maximum_time)) = Self::probe_time_window(times, &mut window) else {
            return vec![];
        };
        let mut average = vec![0.0_f32; FAR_FIELD_DIRECTIONS];
        let mut count = 0_u32;
        for frame in frames
            .iter()
            .filter(|frame| frame.time >= minimum_time && frame.time <= maximum_time)
        {
            if frame.intensity.len() != FAR_FIELD_DIRECTIONS {
                continue;
            }
            for (sum, value) in average.iter_mut().zip(&frame.intensity) {
                *sum += *value;
            }
            count += 1;
        }
        if count > 0 {
            for value in &mut average {
                *value /= count as f32;
            }
            average
        } else {
            vec![]
        }
    }
    fn far_field_polar(ui: &mut egui::Ui, label: &str, intensity: &[f32]) {
        ui.small(label);
        let size = ui.available_width().max(100.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
        let center = rect.center();
        let radius = 0.44 * size;
        ui.painter()
            .rect_filled(rect, 2.0, Color32::from_rgb(12, 18, 24));
        for fraction in [0.25, 0.5, 0.75, 1.0] {
            ui.painter().circle_stroke(
                center,
                radius * fraction,
                Stroke::new(0.7, Color32::from_rgb(55, 69, 80)),
            );
        }
        ui.painter().line_segment(
            [
                egui::pos2(center.x - radius, center.y),
                egui::pos2(center.x + radius, center.y),
            ],
            Stroke::new(0.7, Color32::from_rgb(55, 69, 80)),
        );
        ui.painter().line_segment(
            [
                egui::pos2(center.x, center.y - radius),
                egui::pos2(center.x, center.y + radius),
            ],
            Stroke::new(0.7, Color32::from_rgb(55, 69, 80)),
        );
        let maximum = intensity
            .iter()
            .copied()
            .filter(|value| value.is_finite())
            .fold(0.0_f32, f32::max);
        if maximum <= 1.0e-30 {
            ui.painter().text(
                center,
                egui::Align2::CENTER_CENTER,
                "Waiting for signal",
                egui::FontId::monospace(10.0),
                Color32::from_rgb(112, 130, 143),
            );
            return;
        }
        let mut points = intensity
            .iter()
            .copied()
            .enumerate()
            .map(|(index, value)| {
                let db = 10.0 * (value.max(1.0e-30) / maximum).log10();
                let radial = (1.0 + db / 40.0).clamp(0.0, 1.0);
                let angle = std::f32::consts::TAU * index as f32 / FAR_FIELD_DIRECTIONS as f32;
                center + egui::vec2(angle.cos(), -angle.sin()) * radius * radial
            })
            .collect::<Vec<_>>();
        if let Some(first) = points.first().copied() {
            points.push(first);
        }
        if points.len() > 2 {
            ui.painter()
                .add(egui::Shape::line(points, Stroke::new(2.0, TEAL)));
        }
        for (offset, align, label) in [
            (egui::vec2(radius, 0.0), egui::Align2::RIGHT_BOTTOM, "0°"),
            (egui::vec2(0.0, -radius), egui::Align2::LEFT_TOP, "90°"),
            (egui::vec2(-radius, 0.0), egui::Align2::LEFT_BOTTOM, "180°"),
            (egui::vec2(0.0, radius), egui::Align2::LEFT_BOTTOM, "270°"),
        ] {
            ui.painter().text(
                center + offset,
                align,
                label,
                egui::FontId::monospace(9.0),
                Color32::from_rgb(142, 161, 175),
            );
        }
    }
    fn probe_windows(&mut self, ctx: &egui::Context) {
        let physics = self.editor.document.model.accepted.physics;
        let history = self.probe_history_seconds;
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
            let point_samples = self
                .probe_traces
                .get(&id)
                .map(|trace| trace.samples.iter().copied().collect::<Vec<_>>())
                .unwrap_or_default();
            let curve_frames = self
                .curve_probe_traces
                .get(&id)
                .map(|trace| trace.records.iter().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            let area_samples = self
                .area_probe_traces
                .get(&id)
                .map(|trace| trace.records.iter().copied().collect::<Vec<_>>())
                .unwrap_or_default();
            let is_curve = matches!(
                probe.target,
                TopologyProbeTarget::Segment { .. } | TopologyProbeTarget::Boundary(_)
            );
            let is_area = matches!(
                probe.target,
                TopologyProbeTarget::AreaDisk { .. } | TopologyProbeTarget::AreaRegion(_)
            );
            let (length, closed) = self.probe_metrics.get(&id).copied().unwrap_or((0.0, false));
            let curve_times = curve_frames
                .iter()
                .map(|frame| PointProbeRecord {
                    probe_id: frame.probe_id,
                    time: frame.time,
                    ..Default::default()
                })
                .collect::<Vec<_>>();
            let status = self.probe_status.get(&id).cloned();
            let mut view = self
                .probe_views
                .remove(&id)
                .unwrap_or_else(|| ProbeViewState::new(history));
            let newest_time = point_samples
                .last()
                .map(|sample| sample.time)
                .or_else(|| curve_frames.last().map(|frame| frame.time))
                .or_else(|| area_samples.last().map(|sample| sample.time));
            if view.live
                && let Some(time) = newest_time
            {
                view.end_time = time;
            }
            view.span = view.span.clamp(0.02, history);
            let kind = match probe.target {
                TopologyProbeTarget::Point(_) => "point probe",
                TopologyProbeTarget::Segment { .. } => "line probe",
                TopologyProbeTarget::Boundary(_) => "boundary probe",
                TopologyProbeTarget::AreaDisk { .. } | TopologyProbeTarget::AreaRegion(_) => {
                    "area probe"
                }
            };
            let mut open = true;
            let mut clear = false;
            egui::Window::new(format!("{} · {kind}", probe.name))
                .id(egui::Id::new(("probe_readout", id.0)))
                .open(&mut open)
                .default_width(430.0)
                .resizable(true)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .add(egui::Button::new("Live").selected(view.live))
                            .on_hover_text("Follow the newest sample")
                            .clicked()
                        {
                            view.live = true;
                            if let Some(time) = newest_time {
                                view.end_time = time;
                            }
                        }
                        if is_curve {
                            let active = LineProbeQuantity::ALL
                                .into_iter()
                                .filter(|quantity| quantity.applies(physics))
                                .flat_map(|quantity| {
                                    LineProbeRepresentation::ALL.into_iter().map(
                                        move |representation| {
                                            quantity.offset() + representation.offset()
                                        },
                                    )
                                })
                                .filter(|index| view.line_plots[*index])
                                .count();
                            egui::containers::menu::MenuButton::new(format!("Plots ({active})"))
                                .config(
                                    egui::containers::menu::MenuConfig::new().close_behavior(
                                        egui::PopupCloseBehavior::CloseOnClickOutside,
                                    ),
                                )
                                .ui(ui, |ui| {
                                    egui::Grid::new(("line_probe_plots", id.0))
                                        .num_columns(4)
                                        .spacing(egui::vec2(12.0, 4.0))
                                        .show(ui, |ui| {
                                            ui.label("");
                                            for representation in LineProbeRepresentation::ALL {
                                                ui.small(representation.label());
                                            }
                                            ui.end_row();
                                            for quantity in LineProbeQuantity::ALL {
                                                if !quantity.applies(physics) {
                                                    continue;
                                                }
                                                ui.label(quantity.label_for(physics));
                                                for representation in LineProbeRepresentation::ALL {
                                                    let index =
                                                        quantity.offset() + representation.offset();
                                                    ui.checkbox(&mut view.line_plots[index], "")
                                                        .on_hover_text(format!(
                                                            "{} {}",
                                                            quantity.label_for(physics),
                                                            representation.label()
                                                        ));
                                                }
                                                ui.end_row();
                                            }
                                        });
                                    ui.separator();
                                    ui.add(
                                        egui::Slider::new(&mut view.waterfall_gain, 0.1..=10.0)
                                            .logarithmic(true)
                                            .text("Waterfall gain"),
                                    );
                                });
                        } else if is_area {
                            let electromagnetic =
                                matches!(physics, PhysicsModel::Electromagnetic { .. });
                            let active = [
                                view.area_mean_field,
                                view.area_rms_field,
                                view.area_rms_transverse && electromagnetic,
                                view.area_mean_energy,
                                view.area_total_energy,
                            ]
                            .into_iter()
                            .filter(|enabled| *enabled)
                            .count();
                            egui::containers::menu::MenuButton::new(format!("Plots ({active})"))
                                .config(
                                    egui::containers::menu::MenuConfig::new().close_behavior(
                                        egui::PopupCloseBehavior::CloseOnClickOutside,
                                    ),
                                )
                                .ui(ui, |ui| {
                                    ui.checkbox(
                                        &mut view.area_mean_field,
                                        format!("Mean {}", primary_field_label(physics)),
                                    );
                                    ui.checkbox(
                                        &mut view.area_rms_field,
                                        format!("RMS {}", primary_field_label(physics)),
                                    );
                                    if electromagnetic {
                                        ui.checkbox(
                                            &mut view.area_rms_transverse,
                                            format!(
                                                "RMS {}",
                                                transverse_field_magnitude_label(physics)
                                            ),
                                        );
                                    }
                                    ui.checkbox(&mut view.area_mean_energy, "Mean energy density");
                                    ui.checkbox(
                                        &mut view.area_total_energy,
                                        total_energy_label(physics),
                                    );
                                });
                        } else {
                            let electromagnetic =
                                matches!(physics, PhysicsModel::Electromagnetic { .. });
                            let active = [
                                view.field,
                                view.secondary_field,
                                view.poynting && electromagnetic,
                                view.energy,
                            ]
                            .into_iter()
                            .filter(|enabled| *enabled)
                            .count();
                            egui::containers::menu::MenuButton::new(format!("Plots ({active})"))
                                .config(
                                    egui::containers::menu::MenuConfig::new().close_behavior(
                                        egui::PopupCloseBehavior::CloseOnClickOutside,
                                    ),
                                )
                                .ui(ui, |ui| {
                                    ui.checkbox(&mut view.field, primary_field_label(physics));
                                    ui.checkbox(
                                        &mut view.secondary_field,
                                        match physics {
                                            PhysicsModel::Mechanical => {
                                                primary_field_rate_label(physics)
                                            }
                                            PhysicsModel::Electromagnetic { .. } => {
                                                transverse_field_magnitude_label(physics)
                                            }
                                        },
                                    );
                                    if electromagnetic {
                                        ui.checkbox(&mut view.poynting, "Poynting magnitude |S|");
                                    }
                                    ui.checkbox(&mut view.energy, "Energy density");
                                });
                        }
                        if ui.small_button("Clear").clicked() {
                            clear = true;
                        }
                    });
                    if let Some(status) = &status {
                        ui.colored_label(GOLD, status);
                    }
                    if is_curve {
                        if let Some(frame) = curve_frames.last() {
                            let (_, coverage) = Self::curve_probe_integral(
                                frame,
                                length,
                                LineProbeQuantity::Field,
                                closed,
                            );
                            if coverage < 0.999 {
                                ui.small(format!("Valid coverage {:.0}%", coverage * 100.0));
                            }
                        }
                        for quantity in LineProbeQuantity::ALL {
                            if !quantity.applies(physics) {
                                continue;
                            }
                            if view.line_plots
                                [quantity.offset() + LineProbeRepresentation::Arclength.offset()]
                            {
                                Self::curve_probe_profile(
                                    ui,
                                    &curve_frames,
                                    &view,
                                    quantity,
                                    length,
                                    physics,
                                );
                            }
                            if view.line_plots
                                [quantity.offset() + LineProbeRepresentation::Waterfall.offset()]
                            {
                                Self::curve_probe_waterfall(
                                    ui,
                                    &curve_frames,
                                    &curve_times,
                                    &mut view,
                                    history,
                                    quantity,
                                    physics,
                                );
                            }
                            if view.line_plots
                                [quantity.offset() + LineProbeRepresentation::Integral.offset()]
                            {
                                let integral = curve_frames
                                    .iter()
                                    .map(|frame| PointProbeRecord {
                                        probe_id: frame.probe_id,
                                        time: frame.time,
                                        displacement: Self::curve_probe_integral(
                                            frame, length, quantity, closed,
                                        )
                                        .0,
                                        ..Default::default()
                                    })
                                    .collect::<Vec<_>>();
                                Self::probe_plot(
                                    ui,
                                    &format!("{} ∫ ds", quantity.label_for(physics)),
                                    &integral,
                                    |sample| sample.displacement,
                                    quantity.color(),
                                    &mut view,
                                    history,
                                );
                            }
                        }
                    } else if is_area {
                        if let Some(sample) = area_samples.last() {
                            ui.small(format!(
                                "area {:.4} · covered {:.0}%",
                                sample.covered_area,
                                sample.coverage * 100.0
                            ));
                        }
                        let history_of = |value: fn(&AreaProbeRecord) -> f64| {
                            area_samples
                                .iter()
                                .map(|sample| PointProbeRecord {
                                    probe_id: sample.probe_id,
                                    time: sample.time,
                                    displacement: value(sample),
                                    ..Default::default()
                                })
                                .collect::<Vec<_>>()
                        };
                        for (enabled, label, values, color) in [
                            (
                                view.area_mean_field,
                                format!("Mean {}", primary_field_label(physics)),
                                history_of(|s| s.mean_displacement),
                                SELECT,
                            ),
                            (
                                view.area_rms_field,
                                format!("RMS {}", primary_field_label(physics)),
                                history_of(|s| s.rms_displacement),
                                TEAL,
                            ),
                            (
                                view.area_rms_transverse
                                    && matches!(physics, PhysicsModel::Electromagnetic { .. }),
                                format!("RMS {}", transverse_field_magnitude_label(physics)),
                                history_of(|s| s.rms_transverse_magnitude),
                                Color32::from_rgb(188, 139, 255),
                            ),
                            (
                                view.area_mean_energy,
                                "Mean energy density".to_owned(),
                                history_of(|s| s.mean_energy_density),
                                GOLD,
                            ),
                            (
                                view.area_total_energy,
                                total_energy_label(physics).to_owned(),
                                history_of(|s| s.total_energy),
                                RED,
                            ),
                        ] {
                            if enabled {
                                Self::probe_plot(
                                    ui,
                                    &label,
                                    &values,
                                    |sample| sample.displacement,
                                    color,
                                    &mut view,
                                    history,
                                );
                            }
                        }
                    } else {
                        let electromagnetic =
                            matches!(physics, PhysicsModel::Electromagnetic { .. });
                        if view.field {
                            Self::probe_plot(
                                ui,
                                primary_field_label(physics),
                                &point_samples,
                                |sample| sample.displacement,
                                SELECT,
                                &mut view,
                                history,
                            );
                        }
                        if view.secondary_field {
                            Self::probe_plot(
                                ui,
                                match physics {
                                    PhysicsModel::Mechanical => primary_field_rate_label(physics),
                                    PhysicsModel::Electromagnetic { .. } => {
                                        transverse_field_magnitude_label(physics)
                                    }
                                },
                                &point_samples,
                                |sample| match physics {
                                    PhysicsModel::Mechanical => sample.velocity,
                                    PhysicsModel::Electromagnetic { .. } => {
                                        sample.transverse_magnitude
                                    }
                                },
                                TEAL,
                                &mut view,
                                history,
                            );
                        }
                        if view.poynting && electromagnetic {
                            Self::probe_plot(
                                ui,
                                "Poynting magnitude |S|",
                                &point_samples,
                                |sample| sample.poynting_magnitude,
                                RED,
                                &mut view,
                                history,
                            );
                        }
                        if view.energy {
                            Self::probe_plot(
                                ui,
                                "Local energy density",
                                &point_samples,
                                |sample| sample.energy_density,
                                GOLD,
                                &mut view,
                                history,
                            );
                        }
                    }
                    ui.small(if is_curve {
                        "Drag traces horizontally or waterfalls vertically · wheel to zoom"
                    } else {
                        "Drag right for earlier time · wheel to zoom"
                    });
                });
            if clear {
                self.clear_probe_trace(id);
            }
            if !open {
                self.probe_windows.remove(&id);
            }
            self.probe_views.insert(id, view);
        }
        self.far_field_readout_window(ctx);
    }
    fn far_field_readout_window(&mut self, ctx: &egui::Context) {
        if !self.far_field_window {
            return;
        }
        let history = self.probe_history_seconds;
        let frames = self
            .far_field_trace
            .records
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let times = frames
            .iter()
            .map(|frame| PointProbeRecord {
                time: frame.time,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let power = frames
            .iter()
            .map(|frame| PointProbeRecord {
                time: frame.time,
                displacement: frame
                    .intensity
                    .iter()
                    .map(|value| *value as f64)
                    .sum::<f64>()
                    * std::f64::consts::TAU
                    / FAR_FIELD_DIRECTIONS as f64,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let newest_time = frames.last().map(|frame| frame.time);
        if self.far_field_view.live
            && let Some(time) = newest_time
        {
            self.far_field_view.end_time = time;
        }
        self.far_field_view.span = self.far_field_view.span.clamp(0.02, history);
        let status = self
            .runtime
            .active()
            .and_then(|active| active.far_field.as_ref())
            .and_then(|result| result.as_ref().err().cloned());
        let recording = self.far_field_recording();
        let mut open = true;
        let mut clear = false;
        egui::Window::new("Outer-domain far field")
            .id(egui::Id::new("far_field_readout"))
            .open(&mut open)
            .default_width(470.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Button::new("Live").selected(self.far_field_view.live))
                        .clicked()
                    {
                        self.far_field_view.live = true;
                        if let Some(time) = newest_time {
                            self.far_field_view.end_time = time;
                        }
                    }
                    let active = [
                        self.far_field_view.far_waterfall,
                        self.far_field_view.far_polar,
                        self.far_field_view.far_power,
                    ]
                    .into_iter()
                    .filter(|enabled| *enabled)
                    .count();
                    egui::containers::menu::MenuButton::new(format!("Plots ({active})"))
                        .config(
                            egui::containers::menu::MenuConfig::new()
                                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
                        )
                        .ui(ui, |ui| {
                            ui.checkbox(&mut self.far_field_view.far_waterfall, "Waterfall");
                            ui.checkbox(&mut self.far_field_view.far_polar, "Polar patterns");
                            ui.checkbox(&mut self.far_field_view.far_power, "Radiated power")
                                .on_hover_text(
                                    "Directional intensity integrated over observation angle",
                                );
                        });
                    if self.far_field_view.far_waterfall {
                        ui.add(
                            egui::Slider::new(&mut self.far_field_view.waterfall_gain, 0.2..=5.0)
                                .logarithmic(true)
                                .text("gain"),
                        );
                    }
                    if ui.small_button("Clear").clicked() {
                        clear = true;
                    }
                });
                if let Some(status) = &status {
                    ui.colored_label(RED, status);
                    return;
                }
                // A projection needs the whole delay window before it can report
                // anything, so an empty plot here is a recorder still filling
                // rather than a silence in the field.
                if let Some(recorded) = recording {
                    ui.colored_label(
                        GOLD,
                        format!("Recording the delay window · {:.0}%", recorded * 100.0),
                    );
                }
                if self.far_field_view.far_waterfall {
                    Self::far_field_waterfall(
                        ui,
                        &frames,
                        &times,
                        &mut self.far_field_view,
                        history,
                    );
                }
                if self.far_field_view.far_polar {
                    let instantaneous = frames
                        .iter()
                        .min_by(|a, b| {
                            (a.time - self.far_field_view.end_time)
                                .abs()
                                .total_cmp(&(b.time - self.far_field_view.end_time).abs())
                        })
                        .map(|frame| frame.intensity.clone())
                        .unwrap_or_default();
                    let averaged = Self::far_field_average(&frames, &times, &self.far_field_view);
                    ui.small("Relative radiation pattern · 40 dB");
                    ui.columns(2, |columns| {
                        Self::far_field_polar(&mut columns[0], "Instantaneous", &instantaneous);
                        Self::far_field_polar(&mut columns[1], "Time-averaged", &averaged);
                    });
                }
                if self.far_field_view.far_power {
                    Self::probe_plot(
                        ui,
                        "Radiated power",
                        &power,
                        |sample| sample.displacement,
                        GOLD,
                        &mut self.far_field_view,
                        history,
                    );
                }
                ui.small(
                    "Drag through time · wheel to zoom · the average follows the visible window",
                );
            });
        if clear {
            self.far_field_trace = FarFieldTrace::default();
        }
        self.far_field_window = open;
    }
    fn diagnostics_warning(&self) -> bool {
        self.runtime.last_error().is_some() || self.amr_error.is_some()
    }
    fn summary_line(&self) -> String {
        let active = self.runtime.active();
        format!(
            "{:.0} fps · {:.0} steps/s · {} dofs · {} elements · dt {}",
            1000.0 / self.frame_ms.max(0.01),
            self.steps_per_second,
            active.map_or(0, |v| v.operator.degrees_of_freedom()),
            active.map_or(0, |v| v.mesh.triangles.len()),
            active.map_or("—".into(), |v| format!(
                "{:.2e}",
                v.operator.recommended_time_step()
            ))
        )
    }
    /// One draggable window over the whole transaction: the frame it costs, the
    /// topology and mesh it produced, the handoff waits, and the running solver.
    /// A preparation or adaptation error opens it once and lights the status
    /// marker; an ordinary rebuild does neither.
    fn diagnostics_window(&mut self, ctx: &egui::Context) {
        let warning = self.diagnostics_warning();
        if warning && !self.diagnostics_warned {
            self.diagnostics_open = true;
        }
        self.diagnostics_warned = warning;
        if self.frame_ms.is_finite() && self.frame_ms > 0.0 {
            if self.frame_history.len() == FRAME_HISTORY {
                self.frame_history.pop_front();
            }
            self.frame_history.push_back(self.frame_ms);
        }
        if !self.diagnostics_open {
            return;
        }
        let mut open = true;
        egui::Window::new("Performance diagnostics")
            .open(&mut open)
            .default_width(420.0)
            .resizable(true)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.monospace(self.summary_line());
                    ui.separator();
                    self.frame_section(ui);
                    self.topology_section(ui);
                    self.mesh_section(ui);
                    self.handoff_section(ui);
                    self.solver_section(ui);
                });
            });
        self.diagnostics_open = open;
    }
    /// The `?` beside a place where formulas are typed.
    fn formula_help_toggle(&mut self, ui: &mut egui::Ui) {
        if ui
            .small_button("?")
            .on_hover_text("Formula syntax reference")
            .clicked()
        {
            self.formula_help_open = !self.formula_help_open;
        }
    }

    fn formula_help_window(&mut self, ctx: &egui::Context) {
        if !self.formula_help_open {
            return;
        }
        let mut open = true;
        egui::Window::new("Formula syntax")
            .id(egui::Id::new("formula_help"))
            .open(&mut open)
            .default_width(340.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.small("Every material coefficient and every source profile takes either a constant or a formula in these terms.");
                ui.separator();
                ui.strong("Coordinates and constants");
                egui::Grid::new("formula_help_symbols")
                    .num_columns(2)
                    .spacing([12.0, 2.0])
                    .show(ui, |ui| {
                        for (names, meaning) in FORMULA_SYMBOLS {
                            ui.monospace(names);
                            ui.small(meaning);
                            ui.end_row();
                        }
                    });
                ui.small("A material's own named parameters can be used directly.");
                ui.separator();
                ui.strong("Operators");
                ui.monospace("+  -  *  /  ^  ( )");
                ui.small("^ raises to a power and groups to the right.");
                ui.separator();
                ui.strong("Functions");
                egui::Grid::new("formula_help_functions")
                    .num_columns(2)
                    .spacing([12.0, 2.0])
                    .show(ui, |ui| {
                        for (signature, meaning, _) in FORMULA_FUNCTIONS {
                            ui.monospace(signature);
                            ui.small(meaning);
                            ui.end_row();
                        }
                    });
                ui.separator();
                ui.small("A radial profile, for example: parameter R = 0.35 with stiffness 2 - clamp(0, 1, r / R)^2.");
            });
        self.formula_help_open = open;
    }

    fn frame_section(&self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Frame")
            .default_open(true)
            .show(ui, |ui| {
                ui.label(format!(
                    "{:.2} ms · {:.0} px/unit",
                    self.frame_ms, self.scale
                ));
                let samples = self.frame_history.len();
                let peak = self.frame_history.iter().copied().fold(0.0_f32, f32::max);
                let average =
                    self.frame_history.iter().sum::<f32>() / self.frame_history.len().max(1) as f32;
                let mut ordered = self.frame_history.iter().copied().collect::<Vec<_>>();
                ordered.sort_by(f32::total_cmp);
                let p95 = ordered
                    .get(((samples as f32 * 0.95).ceil() as usize).saturating_sub(1))
                    .copied()
                    .unwrap_or(0.0);
                ui.small(format!(
                    "Average {average:.2} ms · p95 {p95:.2} ms · peak {peak:.2} ms · {samples} samples"
                ));
                let (response, painter) =
                    ui.allocate_painter(egui::vec2(ui.available_width(), 46.0), Sense::hover());
                let rect = response.rect;
                painter.rect_filled(rect, 3.0, Color32::from_rgb(10, 16, 22));
                if samples > 1 && peak > 0.0 {
                    let points = self
                        .frame_history
                        .iter()
                        .enumerate()
                        .map(|(index, value)| {
                            Pos2::new(
                                egui::lerp(
                                    rect.left()..=rect.right(),
                                    index as f32 / (samples - 1) as f32,
                                ),
                                rect.bottom() - rect.height() * (value / peak),
                            )
                        })
                        .collect();
                    painter.add(egui::Shape::line(points, Stroke::new(1.5, TEAL)));
                }
            });
    }
    fn topology_section(&self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Topology")
            .default_open(true)
            .show(ui, |ui| {
                match self.editor.acceptance {
                    TopologyAcceptance::Valid => ui.label("Draft accepted"),
                    TopologyAcceptance::Pending => ui.label("Draft compiling"),
                    // The status bar says this in words; the diagnostics window
                    // is where the structure behind it belongs.
                    TopologyAcceptance::Invalid(issue) => {
                        ui.colored_label(RED, format!("Draft invalid: {issue:?}"))
                    }
                };
                ui.small(format!(
                    "Document revision {} · {} curves · {} authored regions",
                    self.editor.revision,
                    self.editor.document.model.draft.geometry.curves.len(),
                    self.editor.document.model.draft.regions.len(),
                ));
                match self.runtime.active() {
                    Some(active) => ui.small(format!(
                        "Committed token: document {} · topology {} · mesh generation {}",
                        active.bundle.token.document_revision,
                        active.bundle.token.topology_revision,
                        active.bundle.token.mesh_generation,
                    )),
                    None => ui.small("No committed topology"),
                };
                match (self.runtime.phase(), self.runtime.preparing_timing()) {
                    (Some(phase), Some(timing)) => {
                        ui.small(format!(
                            "Preparing: {}",
                            self.runtime.detail().unwrap_or(phase.label())
                        ));
                        ui.small(timing_line(timing));
                    }
                    (Some(phase), None) => {
                        ui.small(format!("Candidate: {}", phase.label()));
                    }
                    _ => {
                        ui.small("No candidate in preparation");
                    }
                }
                if let Some(error) = self.runtime.last_error() {
                    ui.colored_label(RED, format!("{error}"));
                }
            });
    }
    fn mesh_section(&self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Mesh")
            .default_open(true)
            .show(ui, |ui| {
                match self.runtime.active() {
                    Some(active) => {
                        ui.label(format!(
                            "{} vertices · {} triangles · {} active regions",
                            active.mesh.vertices.len(),
                            active.mesh.triangles.len(),
                            active.bundle.plan.domains.len(),
                        ));
                        ui.small(format!(
                            "Minimum angle {:.1}° · maximum edge {:.3} · target {:.3}",
                            active.mesh.quality.minimum_angle_degrees,
                            active.mesh.quality.maximum_edge_length,
                            active.meshing.target_edge_length,
                        ));
                    }
                    None => {
                        ui.label("No committed mesh");
                    }
                }
                ui.small(format!("Adaptation: {}", self.amr_status));
                if let Some(result) = &self.amr_indicator_result {
                    let report = &result.report;
                    ui.small(format!(
                        "Indicator {:.2e}–{:.2e} · target {:.3}–{:.3}",
                        report.minimum_indicator,
                        report.maximum_indicator,
                        report.minimum_target,
                        report.maximum_target,
                    ));
                    ui.small(format!(
                        "Refine candidates {} · coarsen candidates {} · {} work units",
                        report.refine_candidates, report.coarsen_candidates, report.work_units,
                    ));
                }
                if let Some(report) = &self.amr_report {
                    ui.small(format!(
                        "Last adaptation: {} refinements · {} coarsenings · {} rejected collapses",
                        report
                            .topology_changes
                            .saturating_sub(report.coarsening_changes),
                        report.coarsening_changes,
                        report.skipped_collapses,
                    ));
                    ui.small(format!(
                        "{} refine passes · {} coarsen passes · {} work units",
                        report.refine_passes, report.collapse_passes, report.work_units,
                    ));
                    ui.small(format!(
                        "{:.0}% of the source triangles survived · {} still oversized",
                        100.0 * report.preserved_triangles as f64
                            / report.original_triangles.max(1) as f64,
                        report.remaining_oversized_triangles,
                    ));
                }
                if let Some(error) = &self.amr_error {
                    ui.colored_label(RED, error);
                }
            });
    }
    fn handoff_section(&self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Handoff")
            .default_open(true)
            .show(ui, |ui| {
                if self.uploading.is_some() {
                    ui.label("Uploading the candidate to the GPU");
                } else if self.runtime.ready().is_some() {
                    ui.label(format!(
                        "Candidate ready · draining {} requested steps",
                        self.step_backlog
                    ));
                } else if self.runtime.phase().is_some() {
                    ui.label("Preparing a candidate on the CPU");
                } else {
                    ui.label("Idle");
                }
                ui.small("The committed field keeps running until a candidate is acknowledged.");
                match &self.last_handoff {
                    Some(record) => {
                        ui.small(format!(
                            "Last handoff: prepare {:.1} ms · drain {:.1} ms · upload {:.1} ms",
                            record.prepare_ms, record.drain_ms, record.upload_ms,
                        ));
                        ui.small(timing_line(record.timing));
                        ui.small(match record.action {
                            TopologyMeshUpdateAction::Reuse => {
                                if record.adapted {
                                    "Adapted mesh · operator reassembled".to_owned()
                                } else {
                                    format!(
                                        "Reused the committed mesh · operator {}",
                                        if record.operator_reused {
                                            "reused"
                                        } else {
                                            "reassembled"
                                        }
                                    )
                                }
                            }
                            TopologyMeshUpdateAction::Repair(reason) => match record.carve {
                                Some(carve) => format!(
                                    "Mesh repaired: {} · kept {} · removed {} · inserted {}",
                                    reason.label(),
                                    carve.kept_triangles,
                                    carve.removed_triangles,
                                    carve.inserted_triangles,
                                ),
                                None => format!("Mesh repaired: {}", reason.label()),
                            },
                            TopologyMeshUpdateAction::FullRebuild(reason) => {
                                match &record.repair_fallback {
                                    Some(message) => {
                                        format!("Full rebuild: {} ({message})", reason.label())
                                    }
                                    None => format!("Full rebuild: {}", reason.label()),
                                }
                            }
                        });
                        ui.small(format!(
                            "{} · {} dofs · {} triangles",
                            if record.fresh {
                                "Fresh field".to_owned()
                            } else if record.transferred {
                                format!(
                                    "Field transferred · {} of {} nodes copied exactly",
                                    record.exact_nodes, record.degrees_of_freedom
                                )
                            } else {
                                "Field preserved in place".to_owned()
                            },
                            record.degrees_of_freedom,
                            record.triangles,
                        ));
                    }
                    None => {
                        ui.small("No handoff has completed yet.");
                    }
                }
            });
    }
    fn solver_section(&self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Solver")
            .default_open(true)
            .show(ui, |ui| {
                ui.label(format!(
                    "GPU {} · {} dispatches",
                    self.gpu_status, self.gpu_dispatches
                ));
                match self.runtime.active() {
                    Some(active) => {
                        let dt = active.operator.recommended_time_step();
                        ui.small(format!(
                            "{} dofs · {:.2} MiB",
                            active.operator.degrees_of_freedom(),
                            active.operator.estimated_gpu_bytes() as f64 / (1024.0 * 1024.0),
                        ));
                        ui.small(format!(
                            "dt {dt:.3e} · {:.0} steps/s · {:.2} simulated s per wall s",
                            self.steps_per_second,
                            self.steps_per_second * dt,
                        ));
                        ui.small(format!(
                            "Simulated time {:.4} s · {} completed steps",
                            self.simulated_time(),
                            self.completed_steps,
                        ));
                    }
                    None => {
                        ui.small("Waiting for an accepted mesh and wave operator.");
                    }
                }
                let withheld = self.runtime.ready().is_some() || self.uploading.is_some();
                ui.small(format!(
                    "Backlog {} steps · {} per frame ceiling · scheduling {}",
                    self.step_backlog,
                    MAX_STEPS_PER_FRAME,
                    if withheld {
                        "withheld for a pending handoff"
                    } else if self.wave_running {
                        "running"
                    } else {
                        "paused"
                    },
                ));
                if let Some(energy) = self.wave_energy {
                    ui.small(format!("Discrete energy {energy:.6e}"));
                }
            });
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
                    let warning = self.diagnostics_warning();
                    let summary = self.summary_line();
                    let toggled = ui
                        .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let clicked = ui
                                .add(egui::Button::new(summary).frame(false))
                                .on_hover_text("Open performance diagnostics")
                                .clicked();
                            if warning {
                                ui.colored_label(GOLD, "⚠")
                                    .on_hover_text("Preparation or adaptation reported an error");
                            }
                            clicked
                        })
                        .inner;
                    if toggled {
                        self.diagnostics_open = !self.diagnostics_open;
                    }
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
        if !self.capturing() {
            self.probe_windows(root.ctx());
            self.diagnostics_window(root.ctx());
            self.formula_help_window(root.ctx());
        }
        self.keyboard_focus_previous = root.ctx().egui_wants_keyboard_input();
        viewport
    }
}

fn timing_line(timing: TopologyPreparationTiming) -> String {
    format!(
        "mesh {:.1} · assembly {:.1} · transfer {:.1} · sources {:.1} · probes {:.1} ms · {} slices · longest {:.1} ms",
        timing.meshing_ms,
        timing.assembly_ms,
        timing.transfer_ms,
        timing.sources_ms,
        timing.measurements_ms,
        timing.slices,
        timing.longest_slice_ms,
    )
}

/// Which of the two states a span selection is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpanBehaviorState {
    Transmit,
    Boundary,
}

/// The state every selected span shares, or `None` when they disagree. A
/// separated span counts as a boundary whatever its two faces carry, so a
/// Dirichlet or thin-gap baffle reports the state it is actually in rather than
/// reading as neither.
fn span_behavior_state(
    geometry: &TopologyGeometry,
    spans: &BTreeSet<CurveSpanId>,
) -> Option<SpanBehaviorState> {
    let mut state = None;
    for span in geometry
        .curves
        .iter()
        .flat_map(|curve| &curve.spans)
        .filter(|span| spans.contains(&span.id))
    {
        let current = match span.behavior {
            SpanBehavior::Transmitting => SpanBehaviorState::Transmit,
            SpanBehavior::Separated { .. } => SpanBehaviorState::Boundary,
        };
        if state.is_some_and(|previous| previous != current) {
            return None;
        }
        state = Some(current);
    }
    state
}

/// The screen-space unit normal pointing to one side of a span running along
/// `tangent`, or zero for a degenerate segment. Left of increasing parameter in
/// the world is `(t.y, -t.x)` on screen, because the viewport flips the vertical
/// axis; `screen_side` classifies a point by the same rule.
fn side_offset(tangent: egui::Vec2, side: CurveTraceSide) -> egui::Vec2 {
    let length = tangent.length();
    if !length.is_finite() || length <= f32::EPSILON {
        return egui::Vec2::ZERO;
    }
    let left = egui::vec2(tangent.y, -tangent.x) / length;
    match side {
        CurveTraceSide::Left => left,
        CurveTraceSide::Right => -left,
    }
}

/// The coordinates and constants a formula can name, as the reference lists
/// them. Names within an entry are comma separated so the test can evaluate
/// each one on its own.
const FORMULA_SYMBOLS: [(&str, &str); 4] = [
    ("x, y", "position in the region's frame, world units"),
    ("r", "distance from the frame's origin"),
    ("theta", "angle from the frame's x axis, radians"),
    ("pi, e", "3.14159..., 2.71828..."),
];

/// The functions a formula can call, each with an expression the parser has to
/// accept. The reference drifted from the parser once already - it advertised
/// `ln` and `pow`, which the language has never had - so these examples are
/// what keeps the two in step.
const FORMULA_FUNCTIONS: [(&str, &str, &str); 11] = [
    ("sqrt(v)", "square root", "sqrt(r)"),
    ("abs(v)", "magnitude", "abs(x)"),
    ("sin(v)", "sine of an angle in radians", "sin(theta)"),
    ("cos(v)", "cosine of an angle in radians", "cos(theta)"),
    ("tan(v)", "tangent of an angle in radians", "tan(theta)"),
    ("exp(v)", "e raised to v", "exp(-r)"),
    ("log(v)", "natural logarithm", "log(1 + r)"),
    ("min(a, b)", "the smaller of the two", "min(x, y)"),
    ("max(a, b)", "the larger of the two", "max(x, y)"),
    (
        "clamp(min, max, v)",
        "v held inside [min, max]",
        "clamp(0, 1, r)",
    ),
    (
        "smoothstep(edge0, edge1, v)",
        "0 below edge0, 1 above edge1, smooth between",
        "smoothstep(0, 1, r)",
    ),
];

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

/// Average of the triangle centroids carrying one region, used to anchor a
/// region probe's viewport label where the region actually is.
fn region_anchor(mesh: &TriMesh, region: RegionId) -> Option<Point2> {
    let mut sum = Point2::default();
    let mut count = 0.0;
    for triangle in &mesh.triangles {
        if triangle.region != region {
            continue;
        }
        let centroid = triangle
            .vertices
            .iter()
            .fold(Point2::default(), |total, index| {
                total + mesh.vertices[*index].point
            })
            / 3.0;
        sum = sum + centroid;
        count += 1.0;
    }
    (count > 0.0).then(|| sum / count)
}

fn sampling_preset_picker(
    ui: &mut egui::Ui,
    id: ProbeId,
    preset: &mut ProbeSamplingPreset,
) -> bool {
    let mut chosen = *preset;
    egui::ComboBox::from_id_salt(("probe_sampling", id.0))
        .selected_text(match chosen {
            ProbeSamplingPreset::Low => "Low · 32 pts / 30 Hz",
            ProbeSamplingPreset::Medium => "Medium · 64 pts / 60 Hz",
            ProbeSamplingPreset::High => "High · 128 pts / 120 Hz",
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut chosen, ProbeSamplingPreset::Low, "Low");
            ui.selectable_value(&mut chosen, ProbeSamplingPreset::Medium, "Medium");
            ui.selectable_value(&mut chosen, ProbeSamplingPreset::High, "High");
        });
    if chosen != *preset {
        *preset = chosen;
        return true;
    }
    false
}

const fn primary_field_label(physics: PhysicsModel) -> &'static str {
    match physics {
        PhysicsModel::Mechanical => "Displacement u",
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        } => "Electric field E_z",
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        } => "Magnetic field H_z",
    }
}

const fn primary_field_rate_label(physics: PhysicsModel) -> &'static str {
    match physics {
        PhysicsModel::Mechanical => "Velocity ∂u/∂t",
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        } => "Electric-field rate ∂E_z/∂t",
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        } => "Magnetic-field rate ∂H_z/∂t",
    }
}

const fn total_energy_label(physics: PhysicsModel) -> &'static str {
    match physics {
        PhysicsModel::Mechanical => "Mechanical energy",
        PhysicsModel::Electromagnetic { .. } => "Electromagnetic energy",
    }
}

const fn transverse_field_magnitude_label(physics: PhysicsModel) -> &'static str {
    match physics {
        PhysicsModel::Mechanical => "In-plane magnitude",
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        } => "Magnetic magnitude |H|",
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        } => "Electric magnitude |E|",
    }
}

/// Diverging blue/red ramp for a signed waterfall cell, or a single-sided ramp
/// for a non-negative quantity such as energy density.
fn waterfall_color(normalized: f32, single_sided: bool) -> Color32 {
    if single_sided {
        let amount = normalized.max(0.0);
        Color32::from_rgb(
            (22.0 + amount * 233.0) as u8,
            (35.0 + amount * 155.0) as u8,
            (55.0 + amount * 55.0) as u8,
        )
    } else if normalized >= 0.0 {
        Color32::from_rgb(
            (30.0 + normalized * 225.0) as u8,
            (45.0 + normalized * 90.0) as u8,
            (60.0 + normalized * 45.0) as u8,
        )
    } else {
        let amount = -normalized;
        Color32::from_rgb(
            (30.0 + amount * 35.0) as u8,
            (45.0 + amount * 65.0) as u8,
            (60.0 + amount * 195.0) as u8,
        )
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
        TopologyAttachment::LooseEnd { curve, endpoint } => compiled
            .geometry
            .curve(curve)
            .and_then(|curve| {
                let node = if endpoint == 0 {
                    0
                } else {
                    curve.nodes.len() - 1
                };
                curve.spline.node_point(node)
            })
            .and_then(|point| compiled.topology.face_at(point))
            .unwrap_or(EXTERIOR_FACE),
        TopologyAttachment::Breakpoint { curve, node, side } => compiled
            .geometry
            .curve(curve)
            .and_then(|target| {
                let [a, b] = target.spline.span_bounds(node)?;
                FaceAnchor::Curve {
                    curve,
                    span: target.spans.get(node)?.id,
                    side,
                    parameter: (a + b) * 0.5,
                }
                .resolve(&compiled.topology)
                .ok()
            })
            .unwrap_or(EXTERIOR_FACE),
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

/// Categorical colour for one subdomain, taken by the region's position in the
/// authored draft list so neighbouring faces sharing a material still read
/// apart. Every caller must pass the same scene — the draft — or the panel and
/// the viewport disagree about which colour belongs to which region.
fn subdomain_color(scene: &TopologyScene, region: RegionId, opacity: f32) -> Color32 {
    const PALETTE: [[u8; 3]; 10] = [
        [91, 220, 194],
        [248, 196, 112],
        [174, 126, 241],
        [255, 106, 123],
        [72, 166, 255],
        [126, 217, 87],
        [255, 154, 70],
        [236, 130, 200],
        [110, 198, 233],
        [204, 194, 108],
    ];
    let Some(index) = scene
        .regions
        .iter()
        .position(|candidate| candidate.id == region)
    else {
        return Color32::TRANSPARENT;
    };
    let [red, green, blue] = PALETTE[index % PALETTE.len()];
    Color32::from_rgba_unmultiplied(red, green, blue, (opacity * 210.0) as u8)
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

fn amr_target_color(fraction: f32, alpha: u8) -> Color32 {
    let fraction = fraction.clamp(0.0, 1.0);
    let fine = [71.0, 144.0, 232.0];
    let coarse = [246.0, 183.0, 92.0];
    Color32::from_rgba_unmultiplied(
        egui::lerp(fine[0]..=coarse[0], fraction) as u8,
        egui::lerp(fine[1]..=coarse[1], fraction) as u8,
        egui::lerp(fine[2]..=coarse[2], fraction) as u8,
        alpha,
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
    state.refresh_probe_metadata();
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

#[cfg(test)]
mod tests {
    use super::*;
    use funfern_app::topology_viewport::screen_side;

    #[test]
    fn a_span_selection_reports_one_state_only_when_every_span_agrees() {
        let mut editor = TopologyEditor::default();
        editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(0.0, 0.0), 0.3),
                ClosedCurvePurpose::Subdomain {
                    material: DEFAULT_MATERIAL,
                },
            )
            .unwrap();
        let spans = editor.document.model.draft.geometry.curves[0]
            .spans
            .iter()
            .map(|span| span.id)
            .collect::<BTreeSet<_>>();
        assert!(spans.len() > 1);
        let first = BTreeSet::from([*spans.first().unwrap()]);
        let state = |editor: &TopologyEditor, spans: &BTreeSet<CurveSpanId>| {
            span_behavior_state(&editor.document.model.draft.geometry, spans)
        };

        // A closed subdomain's spans start out transmitting.
        assert_eq!(state(&editor, &spans), Some(SpanBehaviorState::Transmit));

        editor
            .set_span_behavior(&first, SpanBehavior::REFLECTING)
            .unwrap();
        assert_eq!(state(&editor, &spans), None);
        assert_eq!(state(&editor, &first), Some(SpanBehaviorState::Boundary));

        // A boundary stays a boundary whatever its faces carry, which is what
        // the old pair of buttons could not say.
        editor
            .set_span_face_condition(
                &first,
                CurveTraceSide::Left,
                FaceBoundaryCondition::Impedance { ratio: 2.0 },
            )
            .unwrap();
        assert_eq!(state(&editor, &first), Some(SpanBehaviorState::Boundary));

        editor
            .set_span_behavior(&spans, SpanBehavior::REFLECTING)
            .unwrap();
        assert_eq!(state(&editor, &spans), Some(SpanBehaviorState::Boundary));
    }

    #[test]
    fn the_side_band_falls_where_a_click_reads_the_same_side() {
        // A span drawn left to right on screen runs along +x in the world, so
        // its left side is up the screen.
        assert_eq!(
            side_offset(egui::vec2(2.0, 0.0), CurveTraceSide::Left),
            egui::vec2(0.0, -1.0)
        );
        for (x, y) in [
            (1.0, 0.0),
            (0.0, 1.0),
            (-1.0, 0.0),
            (0.0, -1.0),
            (3.0, -2.0),
            (-1.5, -4.0),
        ] {
            let a = ScreenPoint::new(0.0, 0.0);
            let b = ScreenPoint::new(f64::from(x), f64::from(y));
            for side in [CurveTraceSide::Left, CurveTraceSide::Right] {
                let offset = side_offset(egui::vec2(x, y), side) * 3.0;
                let point = ScreenPoint::new(f64::from(offset.x), f64::from(offset.y));
                assert_eq!(
                    screen_side(a, b, point),
                    side,
                    "a band on the {side:?} of ({x}, {y}) reads back as the other side"
                );
            }
        }
        assert_eq!(
            side_offset(egui::Vec2::ZERO, CurveTraceSide::Left),
            egui::Vec2::ZERO
        );
    }

    #[test]
    fn the_formula_reference_names_what_the_parser_accepts() {
        let origin = MaterialCoordinates {
            x: 0.0,
            y: 0.0,
            r: 0.0,
            theta: 0.0,
        };
        for (names, _) in FORMULA_SYMBOLS {
            for name in names.split(", ") {
                // An unknown name parses as a material parameter and only fails
                // once it is evaluated without one, so the evaluation is what
                // proves the reference still names something built in.
                let field = ScalarField::formula(name).expect("a listed symbol parses");
                assert!(
                    field.evaluate(origin, &[]).is_ok(),
                    "the reference lists `{name}`, which the parser does not know"
                );
            }
        }
        for (signature, _, example) in FORMULA_FUNCTIONS {
            let field = ScalarField::formula(example)
                .unwrap_or_else(|error| panic!("{signature}: `{example}` is rejected: {error}"));
            assert!(
                field.evaluate(origin, &[]).is_ok(),
                "{signature}: `{example}` does not evaluate"
            );
        }
        for retired in ["ln(1 + r)", "pow(r, 2)"] {
            assert!(
                ScalarField::formula(retired).is_err(),
                "`{retired}` parses, so the reference should be listing it"
            );
        }
    }

    #[test]
    fn marquee_operation_and_direction_are_live_conventions() {
        assert_eq!(
            Playground::marquee_operation(egui::Modifiers::NONE),
            MarqueeOperation::Replace
        );
        assert_eq!(
            Playground::marquee_operation(egui::Modifiers::SHIFT),
            MarqueeOperation::Add
        );
        assert_eq!(
            Playground::marquee_operation(egui::Modifiers::ALT),
            MarqueeOperation::Subtract
        );
        assert_eq!(
            MarqueeContainment::from_drag(Pos2::ZERO, Pos2::new(10.0, 4.0)),
            MarqueeContainment::Enclosed
        );
        assert_eq!(
            MarqueeContainment::from_drag(Pos2::ZERO, Pos2::new(-10.0, 4.0)),
            MarqueeContainment::Crossing
        );
    }

    #[test]
    fn marquee_add_and_subtract_apply_against_the_drag_baseline() {
        let a = TopologySpanTarget::Curve(CurveSpanId(1));
        let b = TopologySpanTarget::Curve(CurveSpanId(2));
        let base = BTreeSet::from([a]);
        let hits = BTreeSet::from([b]);
        assert_eq!(
            Playground::marquee_result(&base, hits.clone(), MarqueeOperation::Replace),
            BTreeSet::from([b])
        );
        assert_eq!(
            Playground::marquee_result(&base, hits.clone(), MarqueeOperation::Add),
            BTreeSet::from([a, b])
        );
        assert_eq!(
            Playground::marquee_result(&BTreeSet::from([a, b]), hits, MarqueeOperation::Subtract,),
            BTreeSet::from([a])
        );
    }

    #[test]
    fn dragging_a_selected_span_preserves_the_complete_selection() {
        let a = TopologySpanTarget::Curve(CurveSpanId(1));
        let b = TopologySpanTarget::Curve(CurveSpanId(2));
        let selection = TopologySelection::Spans(BTreeSet::from([a, b]));

        assert!(Playground::drag_starts_inside_span_selection(
            &selection,
            TopologyHit::Span {
                target: a,
                distance: 0.0,
            },
        ));
        assert!(!Playground::drag_starts_inside_span_selection(
            &selection,
            TopologyHit::Span {
                target: TopologySpanTarget::Curve(CurveSpanId(3)),
                distance: 0.0,
            },
        ));
        assert!(!Playground::drag_starts_inside_span_selection(
            &selection,
            TopologyHit::Handle {
                handle: TopologyHandle::Control {
                    curve: CurveId(1),
                    control: 0,
                },
                distance: 0.0,
            },
        ));
    }
}

#[cfg(test)]
mod probe_interaction_tests {
    use super::*;

    fn viewport() -> Rect {
        Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))
    }

    #[test]
    fn polyline_midpoint_splits_the_path_by_arclength() {
        let path = [
            Point2::new(0.0, 0.0),
            Point2::new(2.0, 0.0),
            Point2::new(2.0, 2.0),
        ];
        let midpoint = Playground::polyline_midpoint(&path).unwrap();
        assert!((midpoint - Point2::new(2.0, 0.0)).norm() < 1.0e-12);
        assert_eq!(
            Playground::polyline_midpoint(&[Point2::new(1.0, 3.0)]),
            Some(Point2::new(1.0, 3.0))
        );
        assert_eq!(Playground::polyline_midpoint(&[]), None);
    }

    /// Every probe kind must resolve a badge point, otherwise it draws no label.
    #[test]
    fn every_probe_kind_resolves_a_label_anchor() {
        let mut state = Playground::default();
        let curve = state
            .editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(0.0, 0.0), 0.4),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        for _ in 0..100_000 {
            state.editor.validate_frame(256);
            if state.editor.acceptance != TopologyAcceptance::Pending {
                break;
            }
        }
        assert_eq!(state.editor.acceptance, TopologyAcceptance::Valid);
        let spans = state
            .editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|candidate| candidate.id == curve)
            .unwrap()
            .spans
            .iter()
            .map(|span| span.id)
            .collect::<Vec<_>>();
        state
            .editor
            .create_probe(
                "Edge".into(),
                [248, 196, 112],
                TopologyProbeTarget::Boundary(TopologyBoundaryProbeTarget {
                    curve,
                    spans,
                    side: CurveTraceSide::Left,
                    reversed: false,
                    preset: ProbeSamplingPreset::Medium,
                }),
            )
            .unwrap();
        state
            .editor
            .create_probe(
                "Line".into(),
                [91, 220, 194],
                TopologyProbeTarget::Segment {
                    start: Point2::new(-0.8, -0.2),
                    end: Point2::new(0.8, 0.2),
                    preset: ProbeSamplingPreset::Medium,
                },
            )
            .unwrap();
        state
            .editor
            .create_probe(
                "Spot".into(),
                [91, 220, 194],
                TopologyProbeTarget::Point(Point2::new(0.6, 0.6)),
            )
            .unwrap();
        state.refresh_samples(viewport());
        assert!(state.sampled.is_some());

        for probe in state.editor.document.model.probes.clone() {
            let badge = state
                .probe_badge(&probe)
                .unwrap_or_else(|| panic!("{} has no label anchor", probe.name));
            assert!(
                badge.finite(),
                "{} anchored at a non-finite point",
                probe.name
            );
            if matches!(probe.target, TopologyProbeTarget::Boundary(_)) {
                assert!(
                    (badge.norm() - 0.4).abs() < 0.05,
                    "boundary badge left its curve: {badge:?}"
                );
            }
        }
    }

    /// A click on a probe must select it without starting a drag, and the badge
    /// of a boundary probe must be grabbable at its drawn position.
    #[test]
    fn probe_hit_testing_covers_every_kind() {
        let mut state = Playground::default();
        state
            .editor
            .create_probe(
                "Spot".into(),
                [91, 220, 194],
                TopologyProbeTarget::Point(Point2::new(0.2, 0.1)),
            )
            .unwrap();
        state
            .editor
            .create_probe(
                "Line".into(),
                [91, 220, 194],
                TopologyProbeTarget::Segment {
                    start: Point2::new(-0.6, -0.3),
                    end: Point2::new(0.6, -0.3),
                    preset: ProbeSamplingPreset::Medium,
                },
            )
            .unwrap();
        state
            .editor
            .create_probe(
                "Disk".into(),
                [91, 220, 194],
                TopologyProbeTarget::AreaDisk {
                    center: Point2::new(-0.5, 0.5),
                    radius: 0.2,
                },
            )
            .unwrap();
        state.refresh_samples(viewport());
        let id_of = |state: &Playground, name: &str| {
            state
                .editor
                .document
                .model
                .probes
                .iter()
                .find(|probe| probe.name == name)
                .unwrap()
                .id
        };
        let point = id_of(&state, "Spot");
        let line = id_of(&state, "Line");
        let disk = id_of(&state, "Disk");

        let at = |state: &Playground, world: Point2| state.screen(world, viewport());
        assert_eq!(
            state.hit_probe(at(&state, Point2::new(0.2, 0.1)), viewport()),
            Some(ProbeHit::Point(point))
        );
        assert_eq!(
            state.hit_probe(at(&state, Point2::new(-0.6, -0.3)), viewport()),
            Some(ProbeHit::SegmentEndpoint(line, true))
        );
        assert_eq!(
            state.hit_probe(at(&state, Point2::new(0.6, -0.3)), viewport()),
            Some(ProbeHit::SegmentEndpoint(line, false))
        );
        assert_eq!(
            state.hit_probe(at(&state, Point2::new(0.0, -0.3)), viewport()),
            Some(ProbeHit::SegmentBody(line))
        );
        assert_eq!(
            state.hit_probe(at(&state, Point2::new(-0.3, 0.5)), viewport()),
            Some(ProbeHit::AreaDiskRadius(disk))
        );
        assert_eq!(
            state.hit_probe(at(&state, Point2::new(-0.5, 0.5)), viewport()),
            Some(ProbeHit::AreaDiskBody(disk))
        );
        assert_eq!(
            state.hit_probe(at(&state, Point2::new(0.9, 0.9)), viewport()),
            None
        );
    }

    /// The frame gizmo must be reachable wherever the numeric placement controls
    /// are, otherwise a region-local profile can only be aligned by typing.
    /// Deleting a selection is one gesture, so it is one undo step however many
    /// curves it covers.
    #[test]
    fn deleting_several_curves_is_one_history_entry() {
        // The default playground opens an example, which this test is not about.
        let mut state = Playground {
            editor: funfern_app::topology_editor::TopologyEditor::default(),
            ..Playground::default()
        };
        let settle = |state: &mut Playground| {
            for _ in 0..100_000 {
                state.editor.validate_frame(4096);
                if state.editor.acceptance != TopologyAcceptance::Pending {
                    return;
                }
            }
            panic!("validation did not terminate")
        };
        settle(&mut state);
        let mut curves = vec![];
        for (index, y) in [0.3f64, 0.0, -0.3].into_iter().enumerate() {
            let x = -0.4 + 0.1 * index as f64;
            curves.push(
                state
                    .editor
                    .create_boundary_baffle(
                        OpenCubicSpline::polyline(vec![
                            Point2::new(x, y),
                            Point2::new(x + 0.2, y + 0.08),
                            Point2::new(x + 0.4, y),
                        ])
                        .unwrap(),
                    )
                    .unwrap(),
            );
            settle(&mut state);
        }
        assert_eq!(state.editor.acceptance, TopologyAcceptance::Valid);
        let before = state.editor.history_len().0;
        state.selection = TopologySelection::Spans(
            state
                .editor
                .document
                .model
                .draft
                .geometry
                .curves
                .iter()
                .flat_map(|curve| curve.spans.iter())
                .map(|span| TopologySpanTarget::Curve(span.id))
                .collect(),
        );

        state.delete_selection();
        settle(&mut state);
        assert_eq!(state.editor.acceptance, TopologyAcceptance::Valid);
        assert!(
            state.editor.document.model.draft.geometry.curves.is_empty(),
            "every selected curve went"
        );
        assert_eq!(
            state.editor.history_len().0,
            before + 1,
            "three curves, one entry"
        );

        assert!(state.editor.undo());
        settle(&mut state);
        assert_eq!(
            state.editor.document.model.draft.geometry.curves.len(),
            curves.len(),
            "one undo brings all three back"
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

    #[test]
    fn material_frame_gizmo_appears_with_its_numeric_controls() {
        let mut state = Playground::default();
        assert_eq!(state.selected_material_frame(), None, "hidden by default");

        let material = state.editor.add_material().unwrap();
        let mut updated = state
            .editor
            .document
            .model
            .draft
            .material(material)
            .unwrap()
            .clone();
        updated.stiffness = ScalarField::formula("1 + 0.2 * x").unwrap();
        state.editor.update_material(updated).unwrap();
        state
            .editor
            .set_region_material(BACKGROUND_REGION, material)
            .unwrap();

        state.inspector = Some(InspectorPanel::Edit);
        assert_eq!(
            state.selected_material_frame(),
            None,
            "the gizmo belongs to the Materials panel"
        );
        state.inspector = Some(InspectorPanel::Materials);
        state.region_selection = BACKGROUND_REGION;
        let (region, frame) = state
            .selected_material_frame()
            .expect("a frame-using material must expose its gizmo");
        assert_eq!(region, BACKGROUND_REGION);

        let centre = state.screen(frame.origin, viewport());
        assert_eq!(
            state.hit_material_frame_gizmo(centre, viewport()),
            Some(MaterialFrameGizmoHit::Origin)
        );
        assert_eq!(
            state.hit_material_frame_gizmo(
                centre + egui::vec2(MATERIAL_FRAME_RADIUS, 0.0),
                viewport()
            ),
            Some(MaterialFrameGizmoHit::Rotate)
        );
        assert_eq!(
            state.hit_material_frame_gizmo(centre + egui::vec2(25.0, 0.0), viewport()),
            None
        );
    }

    /// The combo label and the drawn colour must name the same thing, and two
    /// subdomains sharing a material must still be told apart.
    /// Shift snaps what the drag moves, not the pointer, so where inside a probe
    /// it was picked up cannot leave it off the grid.
    /// The combo label and the drawn colour must name the same thing, and two
    /// subdomains sharing a material must still be told apart.
    #[test]
    fn subdomain_overlay_is_categorical_and_named_consistently() {
        assert_eq!(MaterialOverlay::Subdomains.label(), "Subdomains");
        assert_eq!(
            MaterialOverlay::Subdomains.label_for(PhysicsModel::Mechanical),
            MaterialOverlay::Subdomains.label()
        );
        assert_eq!(MaterialOverlay::Regions.label(), "Materials");

        let mut scene = TopologyScene::default();
        scene.regions.push(Region {
            id: RegionId(7),
            material: DEFAULT_MATERIAL,
            frame: MaterialFrame::world(),
        });
        scene.regions.push(Region {
            id: RegionId(8),
            material: DEFAULT_MATERIAL,
            frame: MaterialFrame::world(),
        });
        let background = subdomain_color(&scene, BACKGROUND_REGION, 1.0);
        let first = subdomain_color(&scene, RegionId(7), 1.0);
        let second = subdomain_color(&scene, RegionId(8), 1.0);
        assert_ne!(first, second, "equal materials must still read apart");
        assert_ne!(background, first);
        assert_eq!(
            subdomain_color(&scene, RegionId(99), 1.0),
            Color32::TRANSPARENT
        );
    }

    #[test]
    fn shift_snaps_the_probe_rather_than_the_cursor() {
        let mut state = Playground::default();
        state
            .editor
            .create_probe(
                "Spot".into(),
                [91, 220, 194],
                TopologyProbeTarget::Point(Point2::new(0.0, 0.0)),
            )
            .unwrap();
        let probe = state.editor.document.model.probes.last().unwrap().clone();
        // Grabbed well off centre, then dragged to an arbitrary place.
        for grab_offset in [0.0, 0.017, -0.023] {
            let original = TopologyProbeTarget::Point(Point2::new(0.0, 0.0));
            let pointer = Point2::new(0.31 + grab_offset, -0.22 + grab_offset);
            let grab = Point2::new(grab_offset, grab_offset);
            state.drag_probe(ProbeHit::Point(probe.id), &original, pointer - grab, true);
            let TopologyProbeTarget::Point(position) =
                state.editor.document.model.probes.last().unwrap().target
            else {
                panic!("a point probe")
            };
            for value in [position.x, position.y] {
                let steps = value / 0.05;
                assert!(
                    (steps - steps.round()).abs() < 1.0e-9,
                    "grabbed at {grab_offset}, landed off the grid at {value}"
                );
            }
        }
        // A disk radius snaps too, rather than jumping by the grab offset.
        state
            .editor
            .create_probe(
                "Disk".into(),
                [91, 220, 194],
                TopologyProbeTarget::AreaDisk {
                    center: Point2::new(0.0, 0.0),
                    radius: 0.2,
                },
            )
            .unwrap();
        let disk = state.editor.document.model.probes.last().unwrap().clone();
        state.drag_probe(
            ProbeHit::AreaDiskRadius(disk.id),
            &disk.target,
            Point2::new(0.113, 0.0),
            true,
        );
        let TopologyProbeTarget::AreaDisk { radius, .. } =
            state.editor.document.model.probes.last().unwrap().target
        else {
            panic!("a disk probe")
        };
        let steps = radius / 0.05;
        assert!((steps - steps.round()).abs() < 1.0e-9, "radius {radius}");
    }

    /// A cursor appears for an affordance the drawing does not announce, or one
    /// whose direction matters, and stays away from drawn handles that move
    /// themselves.
    #[test]
    fn cursors_mark_the_affordances_the_drawing_does_not() {
        let mut state = Playground::default();
        let r = viewport();
        state.refresh_samples(r);
        let domain = state.editor.document.model.draft.geometry.domain;

        // Direction matters: the outer rectangle's corners and sides.
        let corner = state.screen(domain.corners()[0], r);
        assert_eq!(
            state.hover_cursor(corner, r),
            Some(egui::CursorIcon::ResizeNeSw)
        );
        let left_middle = state.screen(
            Point2::new(domain.min_x, (domain.min_y + domain.max_y) * 0.5),
            r,
        );
        assert_eq!(
            state.hover_cursor(left_middle, r),
            Some(egui::CursorIcon::ResizeHorizontal),
            "a bare outer side announces nothing on its own"
        );
        let bottom_middle = state.screen(
            Point2::new((domain.min_x + domain.max_x) * 0.5, domain.min_y),
            r,
        );
        assert_eq!(
            state.hover_cursor(bottom_middle, r),
            Some(egui::CursorIcon::ResizeVertical)
        );

        // Open space keeps the plain arrow.
        assert_eq!(
            state.hover_cursor(state.screen(Point2::default(), r), r),
            None
        );

        // A drawn handle that moves itself says enough by itself.
        let settle = |state: &mut Playground| {
            for _ in 0..100000 {
                state.editor.validate_frame(1000);
                if state.editor.acceptance != TopologyAcceptance::Pending {
                    return;
                }
            }
            panic!("validation did not terminate")
        };
        settle(&mut state);
        let curve = state
            .editor
            .create_boundary_baffle(
                OpenCubicSpline::polyline(vec![
                    Point2::new(-0.3, 0.25),
                    Point2::new(0.0, 0.45),
                    Point2::new(0.3, 0.25),
                ])
                .unwrap(),
            )
            .unwrap();
        settle(&mut state);
        state.invalidate_samples();
        state.refresh_samples(r);
        let control = match &state
            .editor
            .document
            .model
            .draft
            .geometry
            .curve(curve)
            .unwrap()
            .spline
        {
            CurveSpline::Open(spline) => spline.controls()[0],
            CurveSpline::Closed(spline) => spline.controls()[0],
        };
        assert_eq!(
            state.hover_cursor(state.screen(control, r), r),
            None,
            "a control point is its own announcement"
        );

        // A selected span moves the whole selection, which nothing draws.
        let spans = state
            .editor
            .document
            .model
            .draft
            .geometry
            .curve(curve)
            .unwrap()
            .spans
            .iter()
            .map(|span| TopologySpanTarget::Curve(span.id))
            .collect::<BTreeSet<_>>();
        let midpoint = Point2::new(0.0, 0.3833);
        assert_eq!(
            state.hover_cursor(state.screen(midpoint, r), r),
            None,
            "an unselected span offers no move"
        );
        state.selection = TopologySelection::Spans(spans.clone());
        assert_eq!(
            state.hover_cursor(state.screen(midpoint, r), r),
            Some(egui::CursorIcon::Move)
        );
        // Widen the selection to include an outer side, which cannot move.
        let mut blocked = spans;
        blocked.insert(TopologySpanTarget::Outer(OuterSide::Bottom));
        state.selection = TopologySelection::Spans(blocked);
        assert_eq!(
            state.hover_cursor(state.screen(midpoint, r), r),
            None,
            "the cursor is absent exactly when the drag would do nothing"
        );
    }

    /// A mode that owns the viewport says so, since no handle can.
    #[test]
    fn modal_cursors_say_what_a_click_would_do() {
        let mut state = Playground::default();
        let r = viewport();
        let centre = state.screen(Point2::default(), r);
        assert_eq!(state.modal_cursor(centre, r), None);
        state.pulse_mode = true;
        assert_eq!(
            state.modal_cursor(centre, r),
            Some(egui::CursorIcon::Crosshair)
        );
        state.pulse_mode = false;
        state.pending_removal = Some(PendingRemoval::Curve(CurveId(99), vec![]));
        assert_eq!(
            state.modal_cursor(centre, r),
            None,
            "nowhere is clickable while no candidate is offered"
        );
    }

    /// Whatever appeared under the pointer stays for the gesture it started.
    #[test]
    fn a_gesture_keeps_the_cursor_it_began_with() {
        let mut state = Playground::default();
        assert_eq!(state.drag_cursor(), None);
        state.drag = Some(DragGesture::Domain {
            drag: DomainDrag::Side {
                side: OuterSide::Left,
                start: state.editor.document.model.draft.geometry.domain,
            },
        });
        assert_eq!(
            state.drag_cursor(),
            Some(egui::CursorIcon::ResizeHorizontal)
        );
        state.drag = Some(DragGesture::Marquee {
            start: Pos2::ZERO,
            current: Pos2::ZERO,
            base: BTreeSet::new(),
            operation: MarqueeOperation::Replace,
        });
        assert_eq!(
            state.drag_cursor(),
            None,
            "a marquee needs no cursor of its own"
        );
    }

    #[test]
    fn vector_overlay_offers_only_modes_that_draw() {
        assert_eq!(
            VectorOverlay::choices(PhysicsModel::Mechanical),
            &[VectorOverlay::Off, VectorOverlay::RelativeEnergyFlow]
        );
        assert_eq!(
            VectorOverlay::choices(PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            })
            .len(),
            3
        );
        assert_eq!(
            VectorOverlay::ComplementaryField.resolved(PhysicsModel::Mechanical),
            VectorOverlay::RelativeEnergyFlow
        );
        assert_eq!(
            VectorOverlay::ComplementaryField.resolved(PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            }),
            VectorOverlay::ComplementaryField
        );
        assert_eq!(
            VectorOverlay::ComplementaryField.label(PhysicsModel::Mechanical),
            VectorOverlay::RelativeEnergyFlow.label(PhysicsModel::Mechanical)
        );
    }

    /// egui only reports a drag once the pointer has passed `max_click_dist`,
    /// so a grab radius must still cover the control from that far away.
    /// Dragging the outer rectangle resizes it: a side moves only its own edge,
    /// a corner moves the two that meet there, and the rest stays put.
    #[test]
    fn domain_drags_move_one_side_or_one_corner() {
        let start = DomainRect::new(-1.0, 1.0, -1.0, 1.0);
        for (side, expected) in [
            (OuterSide::Left, DomainRect::new(-0.5, 1.0, -1.0, 1.0)),
            (OuterSide::Right, DomainRect::new(-1.0, -0.5, -1.0, 1.0)),
            (OuterSide::Bottom, DomainRect::new(-1.0, 1.0, -0.5, 1.0)),
            (OuterSide::Top, DomainRect::new(-1.0, 1.0, -1.0, -0.5)),
        ] {
            assert_eq!(
                Playground::resize_domain(
                    start,
                    DomainDrag::Side { side, start },
                    Point2::new(-0.5, -0.5),
                ),
                expected,
                "{side:?} moved the wrong edge"
            );
        }
        for (index, expected) in [
            (0usize, DomainRect::new(0.5, 1.0, 0.25, 1.0)),
            (1, DomainRect::new(-1.0, 0.5, 0.25, 1.0)),
            (2, DomainRect::new(-1.0, 0.5, -1.0, 0.25)),
            (3, DomainRect::new(0.5, 1.0, -1.0, 0.25)),
        ] {
            assert_eq!(
                Playground::resize_domain(
                    start,
                    DomainDrag::Corner { index, start },
                    Point2::new(0.5, 0.25),
                ),
                expected,
                "corner {index} moved the wrong pair"
            );
        }
    }

    /// The corners are grabbable at the same radius as any other handle, and
    /// each offers the diagonal cursor that matches it.
    #[test]
    fn domain_corners_are_grabbable_and_cursored() {
        let state = Playground::default();
        let domain = state.editor.document.model.draft.geometry.domain;
        for (index, corner) in domain.corners().into_iter().enumerate() {
            let centre = state.screen(corner, viewport());
            assert_eq!(
                state.hit_domain_corner(centre, viewport()),
                Some(index),
                "corner {index} is not grabbable at its own centre"
            );
            assert_eq!(
                state.hit_domain_corner(centre + egui::vec2(60.0, 60.0), viewport()),
                None,
                "corner {index} grabs far too wide"
            );
        }
        assert_eq!(
            Playground::domain_corner_cursor(0),
            egui::CursorIcon::ResizeNeSw
        );
        assert_eq!(
            Playground::domain_corner_cursor(1),
            egui::CursorIcon::ResizeNwSe
        );
    }

    #[test]
    fn grab_radii_absorb_the_drag_threshold() {
        let mut state = Playground::default();
        state
            .editor
            .create_probe(
                "Spot".into(),
                [91, 220, 194],
                TopologyProbeTarget::Point(Point2::new(0.0, 0.0)),
            )
            .unwrap();
        let id = state
            .editor
            .document
            .model
            .probes
            .iter()
            .find(|probe| probe.name == "Spot")
            .unwrap()
            .id;
        let centre = state.screen(Point2::new(0.0, 0.0), viewport());
        let threshold = egui::InputOptions::default().max_click_dist;
        // The drawn marker reaches 9 px with its selected ring.
        let visual = 9.0;
        assert!(
            state.hit_tolerance(13.0) >= visual + threshold * 0.5,
            "grab radius must exceed the drawn control plus half the drag threshold"
        );
        for offset in [0.0, 6.0, 12.0] {
            assert_eq!(
                state.hit_probe(centre + egui::vec2(offset, 0.0), viewport()),
                Some(ProbeHit::Point(id)),
                "probe lost {offset} px from its centre"
            );
        }
        assert_eq!(
            state.hit_probe(centre + egui::vec2(20.0, 0.0), viewport()),
            None
        );
    }

    /// Dragging an endpoint must move only that endpoint, and a body drag must
    /// translate the whole probe.
    #[test]
    fn probe_drags_reshape_and_translate() {
        let mut state = Playground::default();
        state
            .editor
            .create_probe(
                "Line".into(),
                [91, 220, 194],
                TopologyProbeTarget::Segment {
                    start: Point2::new(-0.5, 0.0),
                    end: Point2::new(0.5, 0.0),
                    preset: ProbeSamplingPreset::Medium,
                },
            )
            .unwrap();
        let index = state
            .editor
            .document
            .model
            .probes
            .iter()
            .position(|probe| probe.name == "Line")
            .unwrap();
        let id = state.editor.document.model.probes[index].id;
        let original = state.editor.document.model.probes[index].target.clone();

        state.drag_probe(
            ProbeHit::SegmentEndpoint(id, true),
            &original,
            Point2::new(0.0, 0.25),
            false,
        );
        let TopologyProbeTarget::Segment { start, end, .. } =
            state.editor.document.model.probes[index].target.clone()
        else {
            panic!("expected a segment")
        };
        assert!((start - Point2::new(-0.5, 0.25)).norm() < 1.0e-12);
        assert!((end - Point2::new(0.5, 0.0)).norm() < 1.0e-12);

        state.drag_probe(
            ProbeHit::SegmentBody(id),
            &original,
            Point2::new(0.1, -0.1),
            false,
        );
        let TopologyProbeTarget::Segment { start, end, .. } =
            state.editor.document.model.probes[index].target.clone()
        else {
            panic!("expected a segment")
        };
        assert!((start - Point2::new(-0.4, -0.1)).norm() < 1.0e-12);
        assert!((end - Point2::new(0.6, -0.1)).norm() < 1.0e-12);
    }

    fn settle(editor: &mut TopologyEditor) {
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
    fn activate(state: &mut Playground) -> Arc<PreparedTopology> {
        let options = MeshingOptions {
            target_edge_length: 0.18,
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

    /// An adaptation spans many frames while the user may remesh underneath
    /// it. Once the active mesh is no longer the one the job started from, the
    /// job is dropped quietly instead of finishing and being rejected at the
    /// handoff as "Adapted mesh does not match the active topology".
    #[test]
    fn an_adaptation_of_a_replaced_mesh_is_discarded_without_an_error() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        let first = activate(&mut state);
        state.amr_enabled = true;
        state.amr_adaptation_job = Some(MeshAdaptationJob::new_topology(
            first.mesh.clone(),
            &first.bundle.plan,
            MeshAdaptationState::from_mesh(&first.mesh),
            state.runtime.reserve_mesh_revision(),
            Arc::new(|_, _| 0.09),
            MeshAdaptationOptions::default(),
        ));
        state.amr_adaptation_source = Some(first.mesh.mesh_revision);

        state
            .editor
            .set_domain(DomainRect {
                max_x: 1.4,
                ..DomainRect::UNIT
            })
            .unwrap();
        settle(&mut state.editor);
        let second = activate(&mut state);
        assert_ne!(second.mesh.mesh_revision, first.mesh.mesh_revision);

        state.refresh_amr(&WaveGpuRequest::default(), &WaveDisplay::default());
        assert!(state.amr_adaptation_job.is_none());
        assert!(state.amr_adaptation_source.is_none());
        assert_eq!(state.amr_error, None);
        assert!(
            state.amr_status.contains("discarded"),
            "status was {:?}",
            state.amr_status
        );
    }

    fn probe_upload(token: TopologyToken, generation: u64, revision: u64) -> ProbeUpload {
        ProbeUpload {
            token,
            generation,
            revision,
            curve_revision: revision,
            area_revision: revision,
            far_field_revision: revision,
        }
    }

    #[test]
    fn replacing_the_wave_buffers_asks_for_the_probe_buffers_again() {
        let token = TopologyToken {
            document_revision: 4,
            topology_revision: 3,
            mesh_generation: 2,
        };
        let upload = probe_upload(token, 7, 1);
        assert!(probes_need_upload(None, token, 7));
        assert!(!probes_need_upload(Some(upload), token, 7));

        // A reset keeps the topology and replaces the wave buffers, and every
        // probe buffer goes with them. Nothing else says so.
        assert!(probes_need_upload(Some(upload), token, 8));

        // A commit that reuses the buffers still moves the stencils.
        assert!(probes_need_upload(
            Some(upload),
            TopologyToken {
                mesh_generation: 3,
                ..token
            },
            7
        ));
    }

    #[test]
    fn a_restarted_run_records_from_its_own_clock() {
        let sample = |time: f64| PointProbeRecord {
            probe_id: 1,
            time,
            ..PointProbeRecord::default()
        };
        let mut state = Playground::default();
        let token = TopologyToken {
            document_revision: 1,
            topology_revision: 1,
            mesh_generation: 1,
        };
        state.probe_upload = Some(probe_upload(token, 1, 1));
        state.sim_time_offset = 4.0;
        state.ingest_probes(&ProbeDisplay {
            generation: 1,
            revision: 1,
            records: vec![sample(0.5), sample(1.0)],
            readbacks: 1,
        });
        let samples = |state: &Playground| -> Vec<f64> {
            state
                .probe_traces
                .get(&ProbeId(1))
                .map(|trace| trace.samples.iter().map(|sample| sample.time).collect())
                .unwrap_or_default()
        };
        assert_eq!(samples(&state), vec![4.5, 5.0]);

        // The run restarts: new buffers, new probes, and a clock back at zero.
        // With the previous run's samples still in the trace every record of the
        // new one sits below its high-water mark and is dropped — the freeze
        // that only Clear could undo.
        state.sim_time_offset = 0.0;
        state.probe_upload = Some(probe_upload(token, 2, 2));
        state.ingest_probes(&ProbeDisplay {
            generation: 2,
            revision: 2,
            records: vec![sample(0.25)],
            readbacks: 2,
        });
        assert_eq!(samples(&state), vec![4.5, 5.0]);

        state.restart_probe_traces();

        // A readback still in flight from the run that ended carries times from
        // a clock the trace has left behind, and would land ahead of everything
        // the new run is about to record.
        state.ingest_probes(&ProbeDisplay {
            generation: 1,
            revision: 1,
            records: vec![sample(1.5)],
            readbacks: 3,
        });
        assert_eq!(samples(&state), Vec::<f64>::new());

        state.ingest_probes(&ProbeDisplay {
            generation: 2,
            revision: 2,
            records: vec![sample(0.25)],
            readbacks: 4,
        });
        assert_eq!(samples(&state), vec![0.25]);
    }
}
