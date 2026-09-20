#![allow(clippy::collapsible_if)]

use crate::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuHandoffOutcome, CanonicalGpuLiveEvent,
    CanonicalGpuPlan, CanonicalGpuRequest, CanonicalGpuRuntimeTransfer, CanonicalGpuTransferPlan,
};
use crate::files::{self, FileEvent, SaveKind};
use crate::material_overlay::{
    MaterialOverlay, MaterialOverlayJob, MaterialOverlaySnapshot, MaterialProperty, OverlayKey,
    OverlayRange,
};
use crate::recording::{self, DestinationRequest, RecordingEvent, RecordingSpec, VideoRecorder};
use crate::wave_gpu::{
    AreaProbeDisplay, AreaProbeInput, AreaProbeRecord, CurveProbeDisplay, CurveProbeInput,
    CurveProbeRecord, FAR_FIELD_DIRECTIONS, FarFieldDisplay, FarFieldHandoff, FarFieldInput,
    FarFieldRecord, MAX_STEPS_PER_FRAME, PointProbeRecord, ProbeDisplay, RecorderContext,
    RecorderHistory, VectorOverlayDisplay, WaveDisplay, WaveGpuRequest,
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
    TopologyBoundaryProbeTarget, TopologyDocument, TopologyEditor, TopologyProbeDefinition,
    TopologyProbeTarget, TopologyRemoval, TopologyRemovalTarget, TopologyWeldOutcome,
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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{
    Arc, Mutex,
    mpsc::{self, Receiver, Sender},
};

const TEAL: Color32 = Color32::from_rgb(91, 220, 194);
const SELECT: Color32 = Color32::from_rgb(72, 166, 255);
const RED: Color32 = Color32::from_rgb(255, 106, 123);
const GOLD: Color32 = Color32::from_rgb(248, 196, 112);
const FRAME_HISTORY: usize = 120;
const EVENT_LOG_ENTRIES: usize = 200;
/// Seconds of progress the steps-per-second readout averages over.
const STEP_RATE_WINDOW: f64 = 0.5;
/// Below one percent of the run's meaningful amplitude, automatic exposure is
/// more likely to reveal the slowly decaying numerical tail than useful wave
/// content. The saved-scene drain fixture plateaus at roughly 0.2--0.3% even
/// with resident damping, so the former 0.1% floor still normalized that tail.
const PRESENTATION_QUIET_AMPLITUDE_RATIO: f64 = 1.0e-2;
/// Energy is quadratic in amplitude. AMR's relative residual becomes dormant
/// below the energy counterpart of the presentation floor.
const DORMANT_ENERGY_RATIO: f64 =
    PRESENTATION_QUIET_AMPLITUDE_RATIO * PRESENTATION_QUIET_AMPLITUDE_RATIO;
/// Frames one line or boundary probe keeps. With the sampling presets' rates
/// this is 17, 8.5, or 4.3 seconds of path history, and it bounds how far back
/// the averaged flux row can look.
const CURVE_TRACE_FRAMES: usize = 512;
const GIZMO_PADDING: f32 = 18.0;
/// Estimated error of the whole field the adaptation aims for, in percent. A
/// factor of two apart like the mesh resolution presets, and named the same
/// way, because they answer the same question at either end of the loop: how
/// much detail is this worth.
const AMR_ACCURACY_PRESETS: [(f64, &str); 3] = [(24.0, "Coarse"), (12.0, "Medium"), (6.0, "Fine")];

/// What to call the accuracy the adaptation is set to. The slider reaches
/// everything between, so most values have no name.
fn amr_accuracy_preset_name(percent: f64) -> &'static str {
    AMR_ACCURACY_PRESETS
        .iter()
        .find(|(value, _)| (percent - value).abs() < 1.0e-9)
        .map_or("Custom", |(_, name)| *name)
}

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

impl BoundaryKind {
    /// What a condition of this kind is called, wherever it is named. The
    /// picker lists these and both condition enums answer with the same words,
    /// which `boundary_names_agree_across_every_source` holds them to.
    const fn label(self) -> &'static str {
        match self {
            Self::Reflecting => "Reflecting",
            Self::FirstOrder => "First-order outgoing",
            Self::SecondOrder => "Second-order outgoing",
            Self::ElectricWall => "Electric wall",
            Self::MagneticWall => "Magnetic wall",
            Self::Neumann => "Prescribed Neumann",
            Self::Dirichlet => "Prescribed Dirichlet",
        }
    }

    const ALL: [Self; 7] = [
        Self::Reflecting,
        Self::FirstOrder,
        Self::SecondOrder,
        Self::ElectricWall,
        Self::MagneticWall,
        Self::Neumann,
        Self::Dirichlet,
    ];
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

/// A change worked out as far as the survivor question, waiting for the click
/// that answers it.
#[derive(Clone, Debug)]
struct PendingMerge {
    action: MergeAction,
    choices: Vec<RegionId>,
}

/// What will be done once the question is answered. Both kinds fold two
/// subdomains into one face - a deletion by removing the edge between them, a
/// weld by moving an end until the circuit that separated them no longer
/// closes - and neither says by itself which material survives.
#[derive(Clone, Debug)]
enum MergeAction {
    /// The span selection the deletion named, rather than the target planned
    /// from it, so the staleness guard has one thing to check and the target is
    /// rebuilt against whatever the document says when the answer arrives.
    Delete(BTreeSet<CurveSpanId>),
    Weld {
        curve: CurveId,
        node: usize,
        endpoint: usize,
        target: TopologyAttachment,
    },
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
/// four of these every frame; `MeanFlux` is derived here from `Flux`. The
/// readout chooses which to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineProbeQuantity {
    Field,
    Transverse,
    Flux,
    MeanFlux,
    Energy,
}

impl LineProbeQuantity {
    const ALL: [Self; 5] = [
        Self::Field,
        Self::Transverse,
        Self::Flux,
        Self::MeanFlux,
        Self::Energy,
    ];

    const fn label_for(self, physics: PhysicsModel) -> &'static str {
        match self {
            Self::Field => primary_field_label(physics),
            Self::Transverse => transverse_field_magnitude_label(physics),
            Self::Flux => match physics {
                PhysicsModel::Mechanical => "Normal energy flux",
                PhysicsModel::Electromagnetic { .. } => "Normal Poynting flux",
            },
            // Named plainly rather than with angle brackets: egui's default
            // font has no glyph for those and drew them as tofu.
            Self::MeanFlux => "Average flux",
            Self::Energy => "Energy density",
        }
    }

    const fn color(self) -> Color32 {
        match self {
            Self::Field => SELECT,
            Self::Transverse => Color32::from_rgb(188, 139, 255),
            Self::Flux => TEAL,
            Self::MeanFlux => RED,
            Self::Energy => GOLD,
        }
    }

    const fn offset(self) -> usize {
        match self {
            Self::Field => 0,
            Self::Transverse => 3,
            Self::Flux => 6,
            Self::MeanFlux => 9,
            Self::Energy => 12,
        }
    }

    const fn applies(self, _physics: PhysicsModel) -> bool {
        true
    }

    /// Whether the row is drawn from the trailing mean of the recorded flux
    /// rather than from the record the GPU wrote.
    const fn averaged(self) -> bool {
        matches!(self, Self::MeanFlux)
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
    transverse_field: bool,
    poynting: bool,
    energy: bool,
    area_mean_field: bool,
    area_rms_field: bool,
    area_rms_transverse: bool,
    area_mean_energy: bool,
    area_total_energy: bool,
    line_plots: [bool; 15],
    /// Seconds of flux the `MeanFlux` row averages over. Fixed rather than tied
    /// to the visible window, so panning and zooming move the view over the
    /// same data instead of rewriting it.
    mean_window: f64,
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
            transverse_field: false,
            poynting: false,
            energy: true,
            area_mean_field: false,
            area_rms_field: true,
            area_rms_transverse: false,
            area_mean_energy: false,
            area_total_energy: true,
            // Field versus arclength and its waterfall, the mean flux profile,
            // and the two integrals.
            line_plots: [
                true, true, false, // primary component
                false, false, false, // transverse magnitude
                false, false, true, // normal flux
                true, false, false, // trailing mean of the normal flux
                false, false, true, // energy density
            ],
            // Two and a half periods of the default source, five of the flux,
            // which oscillates at twice the driven frequency.
            mean_window: 1.0,
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

/// Which transient channel a log entry was caught from, which is also how much
/// it matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EventSource {
    Status,
    Repair,
    Preparation,
    Adaptation,
}

impl EventSource {
    const fn tag(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Repair => "repair",
            Self::Preparation => "preparation",
            Self::Adaptation => "adaptation",
        }
    }
    /// Whether an entry from here lights the status marker until the
    /// diagnostics are opened. A repair fallback explains a rebuild that
    /// succeeded, so it is worth keeping but not worth interrupting for.
    const fn error(self) -> bool {
        matches!(self, Self::Preparation | Self::Adaptation)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct EventEntry {
    /// Seconds since the window system started, as egui counts them.
    time: f64,
    source: EventSource,
    text: String,
    /// How many times in a row this same line arrived, so a channel that
    /// clears and returns every frame cannot flood the ring.
    repeats: usize,
}

fn event_line(entry: &EventEntry) -> String {
    let minutes = (entry.time / 60.0).floor().max(0.0);
    let seconds = entry.time - minutes * 60.0;
    format!(
        "{minutes:.0}:{seconds:04.1} · {} · {}{}",
        entry.source.tag(),
        entry.text,
        if entry.repeats > 1 {
            format!(" ×{}", entry.repeats)
        } else {
            String::new()
        }
    )
}

struct Uploading {
    token: TopologyToken,
    generation: u64,
    fresh: bool,
    degrees_of_freedom: usize,
}

struct PreparedGpuUpload {
    plan: CanonicalGpuPlan,
    transfer: Option<CanonicalGpuTransferPlan>,
}

#[derive(Clone, Debug, PartialEq)]
struct VectorOverlayLayoutKey {
    topology: TopologyToken,
    generation: u64,
    center: Point2,
    scale: f64,
    viewport: Rect,
    spacing: f32,
}

#[derive(Clone, Copy, Debug)]
struct VectorOverlayLayoutPoint {
    element: u32,
    point: Point2,
    stencil: QuadraticPointStencil,
}

struct VectorOverlayLayout {
    key: VectorOverlayLayoutKey,
    revision: u64,
    points: Vec<VectorOverlayLayoutPoint>,
}

/// CPU packing for a candidate generation. The immutable topology owns every
/// input, so native builds can prepare the 80+ MiB GPU layout off the UI thread
/// while the accepted generation keeps running.
struct GpuUploadPreparation {
    token: TopologyToken,
    time_step: f64,
    receiver: Mutex<Receiver<Result<PreparedGpuUpload, String>>>,
    result: Option<Result<PreparedGpuUpload, String>>,
}

fn compile_gpu_upload(
    candidate: PreparedTopology,
    active: Option<Arc<PreparedTopology>>,
    time_step: f64,
    runtime_serials: [u32; 4],
) -> Result<PreparedGpuUpload, String> {
    let state = CanonicalWaveState::zero(&candidate.canonical_operator, time_step)
        .map_err(|error| error.to_string())?;
    let plan = CanonicalGpuPlan::compile_with_quadratic(
        &candidate.canonical_operator,
        &candidate.operator,
        &state,
        &candidate.canonical_forcing,
        CanonicalGpuClock::initial(time_step).map_err(|error| format!("{error:?}"))?,
    )
    .map_err(|error| format!("{error:?}"))?;
    if candidate.fresh || active.is_none() {
        return Ok(PreparedGpuUpload {
            plan,
            transfer: None,
        });
    }
    let active = active.unwrap();
    let transfer = candidate
        .canonical_transfer
        .as_ref()
        .ok_or_else(|| "Canonical handoff maps are not prepared".to_owned())?;
    let runtime = CanonicalGpuRuntimeTransfer::from_primary_transfer(
        &active.canonical_operator,
        &candidate.canonical_operator,
        &active.canonical_forcing,
        &candidate.canonical_forcing,
        &transfer.primary,
        runtime_serials,
    )
    .map_err(|error| format!("{error:?}"))?;
    let gpu_transfer = CanonicalGpuTransferPlan::compile_prepared(
        &active.canonical_operator,
        &candidate.canonical_operator,
        &active.canonical_forcing,
        &candidate.canonical_forcing,
        &transfer.primary,
        &transfer.complementary,
        &transfer.thin_gap,
        &transfer.outgoing,
        &runtime,
    )
    .map_err(|error| format!("{error:?}"))?;
    Ok(PreparedGpuUpload {
        plan,
        transfer: Some(gpu_transfer),
    })
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

/// Ownership of an AMR estimate's immutable snapshot. Live GPU events replace
/// command buffers and advance the request revision, but do not change the
/// topology or invalidate a snapshot already copied to the CPU. A generation
/// handoff does both and therefore is part of the ownership key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AmrIndicatorSource {
    topology: TopologyToken,
    gpu_generation: u64,
    accepted_step: u64,
}

impl AmrIndicatorSource {
    fn is_current(self, topology: TopologyToken, gpu_generation: u64) -> bool {
        self.topology == topology && self.gpu_generation == gpu_generation
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct VectorAcState {
    input: Point2,
    output: Point2,
    step: u64,
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
    pending_merge: Option<PendingMerge>,
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
    /// Whether the example gallery is showing. It stays open across a pick, so
    /// the catalog can be clicked through.
    examples_open: bool,
    /// One thumbnail per catalog entry, built lazily and at most one per frame.
    example_previews: Vec<Option<ExamplePreview>>,
    /// The catalog entry the document came from, for the gallery's own marker.
    /// Any other load clears it.
    example_opened: Option<usize>,
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
    gpu_upload_preparation: Option<GpuUploadPreparation>,
    wave_running: bool,
    wave_step: bool,
    reset_requested: bool,
    /// Set when a whole document is replaced: the next preparation must start
    /// the field from zero rather than transfer the outgoing scene's into it.
    /// Separate from `reset_requested` because that one is spent by the GPU
    /// reset below, which runs earlier in the frame and against the topology
    /// still active — the scene being replaced. Sharing one flag let the load
    /// reset the outgoing scene, which then ran on for the seconds its
    /// replacement took to prepare and handed over a full-amplitude field.
    fresh_requested: bool,
    accumulator: f64,
    sim_time_offset: f64,
    completed_steps: u64,
    steps_per_second: f64,
    /// The best simulated-seconds-per-wall-second seen lately, which is what the
    /// shortfall note reads. The raw measurement dips whenever a handoff
    /// withholds stepping inside its window.
    speed_reached: f64,
    /// The step the GPU was last uploaded with. Not the active operator's
    /// recommendation: the speed ceiling can ask for a smaller one, and between
    /// a speed change and the republish that carries it the two differ.
    uploaded_time_step: f64,
    /// The accepted-step total at the previous observation. Ordinary handovers
    /// preserve it; a fresh install may reset it, so every new generation first
    /// establishes a baseline before its increments are counted.
    rate_steps: u64,
    /// The generation `rate_steps` was read from.
    rate_generation: u64,
    /// Steps banked since the window opened, across however many generations.
    rate_window_steps: u64,
    rate_started: Instant,
    pulse_mode: bool,
    pulse_amplitude: f32,
    pulse_width: f32,
    pending_pulse: Option<(Point2, RegionId)>,
    canonical_event_serial: u32,
    canonical_event_observed: u32,
    probe_mode: Option<ProbePlacement>,
    selected_probe: Option<ProbeId>,
    probe_windows: BTreeSet<ProbeId>,
    far_field_window: bool,
    probe_traces: BTreeMap<ProbeId, ProbeTrace>,
    probe_views: BTreeMap<ProbeId, ProbeViewState>,
    probe_status: BTreeMap<ProbeId, String>,
    probe_metrics: BTreeMap<ProbeId, (f64, bool)>,
    probe_anchors: BTreeMap<ProbeId, Point2>,
    probe_metadata_token: Option<(Option<TopologyToken>, u64)>,
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
    recording_readback_in_flight: Arc<AtomicUsize>,
    startup_done: bool,
    autosave_observed: TopologyDocument,
    autosave_due: Option<Instant>,
    probe_upload: Option<ProbeUpload>,
    /// The upload before it, kept because the readback it issued is still in
    /// flight when the next one is made, and the samples in it are the last of
    /// the old mesh rather than anything the new one will record again.
    probe_upload_previous: Option<ProbeUpload>,
    probe_clock_restarted: bool,
    /// Skin whose physical labels and observable meanings own the current
    /// traces. A skin change starts a new history segment rather than joining
    /// differently named fields into one plot.
    probe_history_physics: Option<PhysicsModel>,
    /// Simulated time the far-field ring started recording from, or `None` when
    /// no recorder is running.
    far_field_recording_from: Option<f64>,
    frame_ms: f32,
    wave_energy: Option<f64>,
    /// Full-state energy is a diagnostic, not a render input. Recomputing it
    /// over every canonical node and sample at display rate made large meshes
    /// consume a main-thread core even when the diagnostics window was closed.
    energy_readback: u64,
    energy_updated: Instant,
    full_snapshot_requested: Instant,
    viewport_rect: Rect,
    vector_overlay_layout: Option<VectorOverlayLayout>,
    /// Presentation-only DC-blocker state for complementary-field arrows. It
    /// never feeds the canonical solver or physical consumers.
    vector_overlay_ac_state: BTreeMap<u32, VectorAcState>,
    vector_overlay_ac_generation: u64,
    vector_overlay_dc_step: u64,
    vector_overlay_dc_active: bool,
    vector_overlay_mode: VectorOverlay,
    vector_overlay_exposure: AutoExposure,
    field_exposure: AutoExposure,
    /// Canonical primary field copied for exposure and paint traversal.
    field_render: Vec<f32>,
    /// Reused by the field's quantile so a frame's sample costs no allocation.
    exposure_scratch: Vec<f64>,
    /// Wall-clock seconds since the previous frame, which is what the exposures
    /// release against so they behave the same at any frame rate.
    frame_delta: f32,
    material_overlay_job: Option<MaterialOverlayJob>,
    material_overlay_snapshot: Option<MaterialOverlaySnapshot>,
    material_overlay_error: Option<String>,
    amr_enabled: bool,
    /// Estimated error of the whole field the adaptation aims for, as a
    /// percentage. Held in the units the control shows so the presets are the
    /// round numbers they read as.
    amr_accuracy_percent: f64,
    amr_elements_per_wavelength: f64,
    amr_minimum_edge: f64,
    amr_maximum_edge: f64,
    grid_scale_filter: bool,
    amr_status: String,
    amr_error: Option<String>,
    amr_last_started: Option<Instant>,
    amr_last_analyzed_step: Option<u64>,
    amr_coarsen_streak: u8,
    amr_indicator_job: Option<SolutionIndicatorJob>,
    amr_indicator_source: Option<AmrIndicatorSource>,
    amr_indicator_result: Option<SolutionIndicatorResult>,
    amr_energy_peak: f64,
    amr_adaptation_job: Option<MeshAdaptationJob>,
    /// Revision of the active mesh the running adaptation started from. The
    /// job is dropped as soon as that mesh is no longer the active one.
    amr_adaptation_source: Option<u64>,
    amr_adaptation_state: Option<MeshAdaptationState>,
    amr_pending_state: Option<MeshAdaptationState>,
    amr_report: Option<MeshAdaptationReport>,
    gpu_status: &'static str,
    gpu_dispatches: u64,
    canonical_gpu_bytes: Option<usize>,
    step_backlog: u64,
    diagnostics_open: bool,
    /// What the transient channels said before they were overwritten, newest
    /// last. Entries are appended when a channel's value changes, which is why
    /// the last value logged from each is kept beside them.
    events: VecDeque<EventEntry>,
    logged_status: Option<String>,
    logged_preparation: Option<String>,
    logged_adaptation: Option<String>,
    /// Repair fallbacks a committed transaction reported, waiting for the next
    /// frame to stamp them. A fallback is an event rather than a state, so two
    /// transactions that fall back the same way are two of them.
    pending_repairs: Vec<String>,
    /// An error was logged since the diagnostics were last open. Keeps the
    /// status marker lit for an error that clears itself a frame later.
    unseen_error: bool,
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
            mesh_edge: 0.08,
            mesh_edge_dragging: false,
            remesh_requested: false,
            requested_edge: f64::NAN,
            requested_revision: None,
            uploading: None,
            gpu_upload_preparation: None,
            wave_running: true,
            wave_step: false,
            reset_requested: false,
            fresh_requested: false,
            accumulator: 0.0,
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
            vector_overlay_ac_state: BTreeMap::new(),
            vector_overlay_ac_generation: u64::MAX,
            vector_overlay_dc_step: u64::MAX,
            vector_overlay_dc_active: false,
            vector_overlay_mode: VectorOverlay::Off,
            vector_overlay_exposure: AutoExposure::default(),
            field_exposure: AutoExposure::default(),
            field_render: Vec::new(),
            exposure_scratch: Vec::new(),
            frame_delta: 0.0,
            material_overlay_job: None,
            material_overlay_snapshot: None,
            material_overlay_error: None,
            amr_enabled: true,
            amr_accuracy_percent: AMR_ACCURACY_PRESETS[1].0,
            amr_elements_per_wavelength: 6.0,
            amr_minimum_edge: 0.02,
            amr_maximum_edge: 0.16,
            grid_scale_filter: true,
            amr_status: "waiting for solution".into(),
            amr_error: None,
            amr_last_started: None,
            amr_last_analyzed_step: None,
            amr_coarsen_streak: 0,
            amr_indicator_job: None,
            amr_indicator_source: None,
            amr_indicator_result: None,
            amr_energy_peak: 0.0,
            amr_adaptation_job: None,
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
        });
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
        self.pending_merge = None;
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
        self.example_opened = None;
        self.pending_merge = None;
        self.requested_revision = None;
        self.fresh_requested = fresh;
        // The scale is left alone here and started again when the new field
        // actually arrives, for the reason the reset path gives: the outgoing
        // scene is still on display until its replacement is prepared, and a
        // scale cleared now would measure that — magnifying a residue for as
        // long as the new mesh takes.
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
                    if ui.button("New").clicked() {
                        self.new_scene();
                        ui.close();
                    }
                    if ui.button("Examples…").clicked() {
                        self.examples_open = true;
                        ui.close();
                    }
                    ui.separator();
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
        // The palette stays up across draws - one primitive after another is the
        // usual way it is used - so it closes only from its own button or the
        // toolbar toggle, and it floats where it was last dragged.
        if self.draw_open && !self.capturing() {
            let ctx = root.ctx().clone();
            let mut open = true;
            egui::Window::new("Draw")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .default_pos([300.0, 42.0])
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
            self.draw_open = open;
        }
    }
    fn side_panel(&mut self, root: &mut egui::Ui) {
        let Some(panel) = self.inspector else { return };
        let title = match panel {
            InspectorPanel::Edit => "Edit",
            InspectorPanel::View => "View",
            InspectorPanel::Simulation => "Simulation",
            InspectorPanel::Materials => "Materials",
            InspectorPanel::Probes => "Probes",
        };
        // On a narrow layout the inspector floats over the viewport instead of
        // docking beside it, which would put a panel inside the capture crop.
        if self.capturing() && root.available_width() < 700.0 {
            return;
        }
        if root.available_width() < 700.0 {
            let mut open = true;
            let maximum_height = (root.ctx().viewport_rect().height() - 54.0).max(96.0);
            egui::Window::new(title)
                .id(egui::Id::new("mobile-inspector"))
                .open(&mut open)
                .default_width(280.0)
                .max_height(maximum_height)
                .anchor(egui::Align2::RIGHT_TOP, [-6.0, 48.0])
                .show(root.ctx(), |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt(("mobile-inspector-scroll", title))
                        .show(ui, |ui| self.inspector_contents(ui, panel));
                });
            if !open {
                self.inspector = None;
            }
            return;
        }
        egui::Panel::right("inspector")
            .default_size(292.0)
            .show(root, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("inspector-scroll", title))
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.inspector_contents(ui, panel));
            });
    }
    fn inspector_contents(&mut self, ui: &mut egui::Ui, panel: InspectorPanel) {
        match panel {
            InspectorPanel::Edit => self.edit_panel(ui),
            InspectorPanel::View => self.view_panel(ui),
            InspectorPanel::Simulation => self.simulation_panel(ui),
            InspectorPanel::Materials => self.materials_panel(ui),
            InspectorPanel::Probes => self.probes_panel(ui),
        }
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
    fn prune_stale_pending_merge(&mut self) {
        if self.pending_merge.as_ref().is_some_and(|pending| {
            let geometry = &self.editor.document.model.draft.geometry;
            match &pending.action {
                MergeAction::Delete(spans) => !spans.iter().all(|span| {
                    geometry
                        .curves
                        .iter()
                        .any(|curve| curve.spans.iter().any(|candidate| candidate.id == *span))
                }),
                MergeAction::Weld { curve, .. } => geometry.curve(*curve).is_none(),
            }
        }) {
            self.pending_merge = None;
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
        let Some(pending) = &self.pending_merge else {
            return;
        };
        let candidates = pending.choices.iter().copied().collect::<BTreeSet<_>>();
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
        if self.pending_merge.is_none() || self.capturing() {
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
            self.pending_merge = None;
        }
    }
    /// Answers the staged deletion with the candidate under a click; a click
    /// anywhere else leaves the question open.
    fn pick_merge_survivor(&mut self, point: Point2) {
        let Some(pending) = self.pending_merge.clone() else {
            return;
        };
        let Some(region) = self
            .draft_region_at(point)
            .filter(|region| pending.choices.contains(region))
        else {
            return;
        };
        match &pending.action {
            MergeAction::Delete(spans) => {
                let outcome = self.editor.removal_target(spans).and_then(|target| {
                    self.editor
                        .remove(&target, Some(region))
                        .map(|removal| (target, removal))
                });
                match outcome {
                    Ok((target, removal)) => {
                        self.pending_merge = None;
                        self.selection = TopologySelection::None;
                        self.invalidate_samples();
                        self.report_removal(&target, &removal);
                    }
                    Err(error) => self.message = error,
                }
            }
            MergeAction::Weld {
                curve,
                node,
                endpoint,
                target,
            } => {
                let (curve, node, endpoint, target) = (*curve, *node, *endpoint, *target);
                if self.weld(curve, node, endpoint, target, Some(region)) {
                    self.pending_merge = None;
                }
            }
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
        // Two states the boundary settings below do not describe. A span with
        // an excluded face on both sides, two holes say, bounds nothing the
        // simulation solves. A transmitting span with an excluded face on one
        // side has nothing to transmit into, and the plan walls it.
        if let Some(compiled) = self.editor.compiled_draft.as_ref() {
            let contexts = curve_spans
                .iter()
                .filter_map(|span| span_context(compiled, *span))
                .collect::<Vec<_>>();
            let inactive = contexts
                .iter()
                .filter(|context| !context.left.active && !context.right.active)
                .count();
            let walled = contexts
                .iter()
                .filter(|context| {
                    context.behavior == SpanBehavior::Transmitting
                        && context.left.active != context.right.active
                })
                .count();
            let badge = |count: usize, one: &str, many: &str| {
                if count == curve_spans.len() && count == 1 {
                    one.to_owned()
                } else if count == curve_spans.len() {
                    format!("All {many}")
                } else {
                    format!("{count} of {} {many}", curve_spans.len())
                }
            };
            if inactive > 0 {
                ui.colored_label(GOLD, badge(inactive, "Inactive", "inactive"))
                    .on_hover_text(
                        "Excluded on both sides, so nothing here reaches the simulation",
                    );
            }
            if walled > 0 {
                ui.colored_label(GOLD, badge(walled, "Walled", "walled"))
                    .on_hover_text(
                        "Transmit meets an excluded face here, so the span reflects until the far side carries a material",
                    );
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
    /// One click while a probe is being placed. `raw` is where the pointer is;
    /// `snap` puts it on the grid Shift snaps everything else to, with the same
    /// conventions a probe already follows when it is dragged - a position goes
    /// onto the grid, a radius is itself a multiple of it.
    fn probe_placement_click(&mut self, raw: Point2, snap: bool) {
        let Some(mode) = self.probe_mode else {
            return;
        };
        let step = self.snap_step();
        let point = if snap {
            Self::snap_point(raw, step)
        } else {
            raw
        };
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
                    radius: Self::placed_disk_radius(center, raw, snap, step),
                })
            }
            // A region is picked by the face under the pointer, which
            // the grid has nothing to say about.
            ProbePlacement::Region => self.runtime.active().and_then(|active| {
                let face = active.bundle.snapshot.face_at(raw)?;
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
    fn snap_point(point: Point2, step: f64) -> Point2 {
        Point2::new(
            (point.x / step).round() * step,
            (point.y / step).round() * step,
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
        let field_reference = self.field_exposure.reference();
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
        if p.field {
            ui.checkbox(&mut p.field_auto_exposure, "Auto exposure")
                .on_hover_text(
                    "Scale the field's colours from what is on screen, so a quiet scene reads \
                     like a loud one. Off, the intensity slider is the whole scale.",
                );
            // The colours are relative, so the level they are relative to has to
            // be readable somewhere or a decaying field looks like a steady one.
            if p.field_auto_exposure
                && let Some(reference) = field_reference
            {
                ui.small(format!("Auto scale {reference:.2e}"));
            }
        }
        let physics = self.editor.document.model.draft.physics;
        p.vector_overlay = p.vector_overlay.resolved(physics);
        ui.separator();
        ui.label("Vector overlay");
        egui::ComboBox::from_id_salt("vector-overlay")
            .selected_text(p.vector_overlay.label(physics))
            .show_ui(ui, |ui| {
                for mode in VectorOverlay::choices(physics) {
                    ui.selectable_value(&mut p.vector_overlay, *mode, mode.label(physics));
                }
            });
        if p.vector_overlay != VectorOverlay::Off {
            if p.vector_overlay == VectorOverlay::ComplementaryField {
                ui.checkbox(&mut p.vector_overlay_ac_coupled, "AC-couple arrows")
                    .on_hover_text(
                        "Subtract a slowly varying presentation baseline from the arrows. \
                         The canonical field, probes and energy remain unchanged.",
                    );
            }
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
    /// The rate the solver is actually reaching, when it is short of the one
    /// asked for and is genuinely trying to reach it.
    fn simulation_speed_shortfall(&self) -> Option<f64> {
        // Not suppressed while a handoff is pending. Adaptation keeps one
        // pending much of the time on exactly the scenes heavy enough to fall
        // short, which silenced the note when it mattered most; the held rate
        // already rides over the few frames a handoff withholds.
        let stepping = self.runtime.active().is_some() && self.wave_running;
        speed_shortfall(
            self.speed_reached,
            self.editor.document.presentation.simulation_speed,
            stepping,
        )
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
        ui.label("Speed");
        ui.add(
            egui::Slider::new(
                &mut self.editor.document.presentation.simulation_speed,
                0.02..=2.0,
            )
            .logarithmic(true)
            .suffix("×")
            .text("Simulated s per wall s"),
        )
        .on_hover_text(
            "A ceiling on how fast simulated time runs against the clock. The solver falls \
             short of it when a step costs more than a frame can afford.",
        );
        if let Some(reached) = self.simulation_speed_shortfall() {
            ui.colored_label(GOLD, format!("Reaching {reached:.2}×"))
                .on_hover_text(
                    "The scene costs more per step than the frame budget allows, so simulated \
                     time runs slower than asked. A coarser mesh buys it back.",
                );
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
        let before_amr = self.amr_settings();
        ui.checkbox(&mut self.amr_enabled, "Adapt mesh to the wave");
        ui.add_enabled_ui(self.amr_enabled, |ui| {
            let preset = amr_accuracy_preset_name(self.amr_accuracy_percent);
            egui::ComboBox::from_id_salt("amr_accuracy")
                .width(ui.available_width())
                .selected_text(format!(
                    "{preset} · target accuracy {:.0}%",
                    self.amr_accuracy_percent
                ))
                .show_ui(ui, |ui| {
                    for (value, name) in AMR_ACCURACY_PRESETS {
                        ui.selectable_value(
                            &mut self.amr_accuracy_percent,
                            value,
                            format!("{name} · {value:.0}%"),
                        );
                    }
                });
            ui.add(
                egui::Slider::new(&mut self.amr_accuracy_percent, 2.0..=50.0)
                    .logarithmic(true)
                    .suffix("%")
                    .text("Target accuracy"),
            )
            .on_hover_text(
                "How much estimated error the whole field is allowed to carry. \
                 Refinement the error estimate asks for stops once the field is inside \
                 this; carrying a forced wavelength and staying under the largest \
                 element allowed are floors, and go on regardless.",
            );
            ui.small(&self.amr_status);
            ui.small(self.amr_estimate_line());
            // Nothing else in the panel explains a mesh pinned at its floor
            // while the accuracy target reads satisfied. It is a standing
            // condition rather than a passing one, so it may take its own line.
            if let Some(result) = &self.amr_indicator_result
                && result.report.smallest_wavelength_target < self.amr_minimum_edge
            {
                ui.colored_label(
                    GOLD,
                    format!(
                        "The forcing wants elements of {:.3}, under the smallest allowed \
                         of {:.3}, so the mesh sits at its floor whatever the accuracy asks",
                        result.report.smallest_wavelength_target, self.amr_minimum_edge,
                    ),
                );
            }
            if let Some(error) = &self.amr_error {
                ui.colored_label(RED, error);
            }
        });
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
        ui.small(
            "Version-22 acceleration control: the canonical solver integrates this signal \
             analytically into a primary-field-rate drive and applies the accepted \
             generation's immutable reference mass.",
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
                "Canonical stored energy: {}",
                self.wave_energy.map_or("—".into(), |v| format!("{v:.4e}"))
            ));
        }
        ui.separator();
        self.advanced_settings(ui);
        if self.amr_settings() != before_amr {
            self.amr_indicator_job = None;
            self.amr_adaptation_job = None;
            self.amr_last_analyzed_step = None;
            self.amr_last_started = None;
            self.amr_coarsen_streak = 0;
            self.amr_error = None;
        }
    }

    /// What the panel says about the estimate, whether or not one is in hand.
    /// Committing a mesh drops the estimate it was measured against, so a line
    /// that exists only while there is one comes and goes with every adaptation
    /// and shifts the panel out from under the pointer. It holds its place and
    /// says it has nothing instead.
    fn amr_estimate_line(&self) -> String {
        match &self.amr_indicator_result {
            Some(result) if result.report.dormant => format!(
                "Estimated error dormant · target {:.0}%",
                self.amr_accuracy_percent
            ),
            Some(result) => format!(
                "Estimated error {:.1}% · target {:.0}%",
                100.0 * result.report.global_indicator,
                self.amr_accuracy_percent,
            ),
            None => format!(
                "Estimated error — · target {:.0}%",
                self.amr_accuracy_percent
            ),
        }
    }

    /// The accuracy target as the estimate states it, rather than as the
    /// control shows it.
    fn amr_target_accuracy(&self) -> f64 {
        self.amr_accuracy_percent / 100.0
    }

    /// Everything an estimate is built from. A change to any of it drops the
    /// work in flight rather than letting it finish against settings nobody
    /// asked for - which is why the settings folded away below are read here
    /// too, and why this is compared after the whole panel has been drawn.
    fn amr_settings(&self) -> (bool, f64, f64, f64, f64) {
        (
            self.amr_enabled,
            self.amr_accuracy_percent,
            self.amr_elements_per_wavelength,
            self.amr_minimum_edge,
            self.amr_maximum_edge,
        )
    }

    /// Numerical hygiene rather than physics, so it is folded away by default:
    /// the limits adaptation works between, and what the scheme does with detail
    /// no mesh can carry. Anything here changes what the solver does, not what
    /// it shows.
    fn advanced_settings(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Advanced settings")
            .default_open(false)
            .show(ui, |ui| {
                ui.label("Adaptation");
                ui.add_enabled_ui(self.amr_enabled, |ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.amr_elements_per_wavelength)
                            .speed(0.1)
                            .range(2.0..=16.0)
                            .prefix("Elements per wavelength "),
                    )
                    .on_hover_text(
                        "How finely a forced wave is carried, wherever a source or a \
                         boundary signal forces one. Quadratic elements put two nodes on \
                         every edge, so six elements is twelve nodes a wavelength. This is \
                         a floor the accuracy target does not lift.",
                    );
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut self.amr_minimum_edge)
                                .speed(0.002)
                                .range(0.005..=1.0)
                                .prefix("Min "),
                        )
                        .on_hover_text("The smallest element adaptation may build");
                        ui.add(
                            egui::DragValue::new(&mut self.amr_maximum_edge)
                                .speed(0.005)
                                .range(0.005..=1.0)
                                .prefix("Max "),
                        )
                        .on_hover_text("The largest element adaptation may leave standing");
                    });
                    self.amr_minimum_edge =
                        self.amr_minimum_edge.min(self.amr_maximum_edge).max(0.005);
                    self.amr_maximum_edge = self.amr_maximum_edge.max(self.amr_minimum_edge);
                });
                ui.separator();
                ui.label("Solver");
                ui.checkbox(&mut self.grid_scale_filter, "Damp unresolvable detail")
                    .on_hover_text(
                        "The scheme does not dissipate at any wavelength, and the fastest \
                         modes a mesh can hold barely travel, so a sharp event - deleting a \
                         wall the field had a step across, or a source narrower than a few \
                         nodes - leaves a speckle that stays put for the rest of the run. \
                         This removes it, at a cost of well under a percent per half minute \
                         to a wave resolved as finely as the adaptation above aims for. \
                         It preserves constants and stationary force-free flux; it is not a \
                         terminal-silence or DC-removal control. \
                         Turn it off to see the untouched scheme.",
                    );
            });
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
                ui.small(
                    "Version-22 acceleration control: this signal is analytically integrated \
                     into a canonical primary-field-rate drive using immutable \
                     accepted-generation normalization.",
                );
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
                let referenced_names = material
                    .parameter_names()
                    .map(str::to_owned)
                    .collect::<BTreeSet<_>>();
                for (index, parameter) in material.parameters.iter_mut().enumerate() {
                    let referenced = referenced_names.contains(&parameter.name);
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
                    debug_assert!(material.remove_parameter(index).is_ok());
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
                    ui.label("Trace side").on_hover_text(
                        "Which of the span's two traces is read. In the scene the arrow \
                         arrives at the marker from that side, along the way positive flux \
                         points",
                    );
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
                    .on_hover_text(
                        "Sample the path against increasing curve parameter. The second \
                         arrow in the scene follows it",
                    )
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
    fn viewport(
        &mut self,
        ui: &mut egui::Ui,
        display: &WaveDisplay,
        vector_display: &VectorOverlayDisplay,
    ) -> Rect {
        let available = ui.available_size();
        let (response, painter) = ui.allocate_painter(available, Sense::click_and_drag());
        let viewport = response.rect;
        self.viewport_rect = viewport;
        if self.fit {
            self.fit_view(viewport);
        }
        self.refresh_samples(viewport);
        let transform = self.transform(viewport);
        self.draw_solution(&painter, viewport, display, vector_display);
        if self.editor.document.presentation.grid {
            self.draw_grid(&painter, viewport);
        }
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
        self.prune_stale_pending_merge();
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
    /// Drawn over the field rather than under it. The field wash is opaque, so
    /// a grid beneath it is only ever visible where nothing is meshed; these
    /// lines tint instead, faint enough not to compete with the wave and light
    /// enough to read on the dark base beside it.
    /// The spacing and the world coordinates the grid draws at. Both axes walk
    /// upwards from the lower corner of the view: `world` flips y, so the
    /// bottom of the screen is the smaller world coordinate.
    fn grid_axes(&self, r: Rect, step: f64) -> (Vec<f64>, Vec<f64>) {
        let minimum = self.world(r.left_bottom(), r);
        let maximum = self.world(r.right_top(), r);
        (
            grid_lines(minimum.x, maximum.x, step),
            grid_lines(minimum.y, maximum.y, step),
        )
    }

    /// The lattice Shift lands on: the grid's own fine step, so what is drawn
    /// is what a snapped position can reach.
    fn snap_step(&self) -> f64 {
        grid_steps(self.scale).1
    }
    fn draw_grid(&self, painter: &egui::Painter, r: Rect) {
        let (step, fine) = grid_steps(self.scale);
        let (columns, rows) = self.grid_axes(r, fine);
        // The fine lattice is what Shift lands on, so it is drawn - faintly,
        // because it is there to be aimed at rather than read.
        let stroke = |value: f64| {
            if value.abs() < fine * 0.1 {
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(148, 163, 184, 90))
            } else if (value / step - (value / step).round()).abs() < 1.0e-6 {
                Stroke::new(0.5, Color32::from_rgba_unmultiplied(148, 163, 184, 45))
            } else {
                Stroke::new(0.5, Color32::from_rgba_unmultiplied(148, 163, 184, 20))
            }
        };
        for x in columns {
            let sx = self.screen(Point2::new(x, 0.0), r).x;
            painter.line_segment(
                [Pos2::new(sx, r.top()), Pos2::new(sx, r.bottom())],
                stroke(x),
            );
        }
        for y in rows {
            let sy = self.screen(Point2::new(0.0, y), r).y;
            painter.line_segment(
                [Pos2::new(r.left(), sy), Pos2::new(r.right(), sy)],
                stroke(y),
            );
        }
    }
    /// A reset zeroes the field and a mesh handoff renumbers it; both bump the
    /// solver's generation, and either can move the field's scale outright.
    /// Measuring a new field against the old reference would paint its first
    /// seconds wrong, which is what loading one example over another used to do
    /// to the arrows. How loud the run has been is kept, so a field decaying
    /// through an adaptation handoff is not renormalized back into view.
    /// A handoff carries the field onto a new mesh — the same field, renumbered
    /// — so its scale carries across untouched. Only a field replaced with zeros
    /// starts a new run.
    ///
    /// Keying this on the solver's generation instead was wrong twice over:
    /// adaptation bumps the generation every second or two, and each bump
    /// dropped the scale onto the instantaneous level. A decaying field then
    /// fell in visible steps — 8.09e-3 to 1.21e-3 across one handoff — instead
    /// of easing down at the release rate.
    fn restart_exposures_after_handoff(&mut self, fresh: bool) {
        if fresh {
            self.field_exposure.restart();
            self.vector_overlay_exposure.restart();
        }
    }

    fn refresh_vector_overlay(
        &mut self,
        recorders: &mut WaveGpuRequest,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        active: Option<&Arc<PreparedTopology>>,
        generation: u64,
    ) {
        let mode = self
            .editor
            .document
            .presentation
            .vector_overlay
            .resolved(self.editor.document.model.draft.physics);
        let Some(active) = active.filter(|_| mode != VectorOverlay::Off) else {
            if self.vector_overlay_layout.take().is_some() || recorders.vector_overlay.is_some() {
                recorders.clear_vector_overlay(assets, commands);
            }
            return;
        };
        if generation == 0 || !self.viewport_rect.is_positive() {
            return;
        }
        let key = VectorOverlayLayoutKey {
            topology: active.bundle.token,
            generation,
            center: self.center,
            scale: self.scale,
            viewport: self.viewport_rect,
            spacing: self.editor.document.presentation.vector_overlay_density,
        };
        if self
            .vector_overlay_layout
            .as_ref()
            .is_some_and(|layout| layout.key == key)
        {
            return;
        }
        let points = vector_overlay_layout(
            &active.bundle.authored,
            &active.mesh,
            &active.operator,
            key.spacing,
            (key.center, key.scale, key.viewport),
        );
        let stencils = points.iter().map(|point| point.stencil).collect::<Vec<_>>();
        match recorders.update_canonical_vector_overlay(
            assets,
            commands,
            &active.canonical_operator,
            &stencils,
        ) {
            Ok(()) => {
                self.vector_overlay_layout = Some(VectorOverlayLayout {
                    key,
                    revision: recorders.vector_overlay_revision(),
                    points,
                });
            }
            Err(error) => {
                recorders.clear_vector_overlay(assets, commands);
                self.vector_overlay_layout = None;
                self.message = error;
            }
        }
    }

    fn draw_solution(
        &mut self,
        painter: &egui::Painter,
        r: Rect,
        display: &WaveDisplay,
        vector_display: &VectorOverlayDisplay,
    ) {
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
            // The canonical primary field is authoritative. Display no longer
            // removes a component mean or reconstructs a gauge-dependent
            // scalar before exposure.
            self.field_render.clone_from(&display.current);
            let level = exposure_level(
                &self.field_render,
                FIELD_EXPOSURE_QUANTILE,
                &mut self.exposure_scratch,
            );
            let reference = self.field_exposure.update(level, self.frame_delta);
            let visibility = if presentation.field_auto_exposure {
                self.field_exposure.visibility(level)
            } else {
                1.0
            };
            let scale = visibility
                * field_scale(
                    presentation.field_gain,
                    reference,
                    presentation.field_auto_exposure,
                );
            let mut field = egui::Mesh::default();
            field.reserve_vertices(active.operator.degrees_of_freedom());
            field.reserve_triangles(active.operator.element_nodes().len() * 6);
            for (point, value) in active.operator.node_points().iter().zip(&self.field_render) {
                let scaled = (f64::from(*value) * scale) as f32;
                let color = if presentation.material_overlay == MaterialOverlay::Off {
                    field_color(scaled, Color32::TRANSPARENT)
                } else {
                    field_color_over_overlay(scaled)
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
            let samples = self
                .vector_overlay_layout
                .as_ref()
                .filter(|layout| {
                    layout.key.topology == active.bundle.token
                        && layout.key.generation == vector_display.generation
                        && layout.revision == vector_display.revision
                        && layout.points.len() == vector_display.samples.len()
                })
                .map(|layout| {
                    layout
                        .points
                        .iter()
                        .zip(&vector_display.samples)
                        .map(|(point, sample)| {
                            let value = match mode {
                                VectorOverlay::ComplementaryField => sample.complementary,
                                VectorOverlay::RelativeEnergyFlow => sample.energy_flow,
                                VectorOverlay::Off => Point2::default(),
                            };
                            (point.element, self.screen(point.point, r), value)
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            self.draw_vector_overlay(
                painter,
                samples,
                vector_display.generation,
                vector_display.completed_steps,
            );
        } else {
            self.vector_overlay_ac_state.clear();
            self.vector_overlay_ac_generation = u64::MAX;
            self.vector_overlay_dc_step = u64::MAX;
            self.vector_overlay_dc_active = false;
        }
    }

    fn draw_vector_overlay(
        &mut self,
        painter: &egui::Painter,
        mut samples: Vec<(u32, Pos2, Point2)>,
        generation: u64,
        completed_steps: u64,
    ) {
        let settings = self.editor.document.presentation;
        let mode = settings
            .vector_overlay
            .resolved(self.editor.document.model.draft.physics);
        if self.vector_overlay_mode != mode {
            self.vector_overlay_ac_state.clear();
            self.vector_overlay_ac_generation = u64::MAX;
            self.vector_overlay_dc_step = u64::MAX;
            self.vector_overlay_dc_active = false;
            self.vector_overlay_exposure.clear();
            self.vector_overlay_mode = mode;
        }
        let dc_active =
            mode == VectorOverlay::ComplementaryField && settings.vector_overlay_ac_coupled;
        if self.vector_overlay_dc_active != dc_active {
            self.vector_overlay_ac_state.clear();
            self.vector_overlay_ac_generation = u64::MAX;
            self.vector_overlay_dc_step = u64::MAX;
            self.vector_overlay_exposure.clear();
            self.vector_overlay_dc_active = dc_active;
        }
        if dc_active {
            if self.vector_overlay_ac_generation != generation {
                self.vector_overlay_ac_state.clear();
                self.vector_overlay_ac_generation = generation;
                self.vector_overlay_dc_step = u64::MAX;
            }
            self.ac_couple_vector_samples(
                &mut samples,
                completed_steps,
                resident_filter_boundary(self.grid_scale_filter, completed_steps),
            );
        } else {
            self.vector_overlay_ac_state.clear();
            self.vector_overlay_ac_generation = generation;
            self.vector_overlay_dc_step = completed_steps;
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
        let Some(reference) = self
            .vector_overlay_exposure
            .update(instantaneous, self.frame_delta)
        else {
            return;
        };
        let visibility = self.vector_overlay_exposure.visibility(instantaneous);
        // Below this point even the longest possible arrow is sub-pixel. Do
        // not normalize its direction: f32 residue has no stable direction,
        // so drawing it only turns numerical noise into visible twitching.
        if visibility <= VECTOR_OVERLAY_VISIBILITY_CUTOFF {
            return;
        }
        let maximum_length = settings.vector_overlay_density * 0.46;
        for (_, origin, value) in samples {
            let magnitude = value.norm();
            if magnitude < reference * 0.015 || !magnitude.is_finite() {
                continue;
            }
            // Clamp the exposed arrow first and fade the result. Clamping
            // after multiplication by `visibility` let a sparse outlier undo
            // the quiet-tail fade and remain at full length.
            let length = vector_arrow_length(
                magnitude,
                reference,
                settings.vector_overlay_gain,
                maximum_length,
                visibility,
            );
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

    /// Removes only the slowly varying presentation baseline from the sampled
    /// complementary field. The exact pole and trapezoidal input difference
    /// keep the corner stable across solver steps and readback batching without
    /// attenuating ordinary source frequencies.
    fn ac_couple_vector_samples(
        &mut self,
        samples: &mut [(u32, Pos2, Point2)],
        completed_steps: u64,
        maintenance_discontinuity: bool,
    ) {
        let restarted = self.vector_overlay_dc_step == u64::MAX
            || completed_steps < self.vector_overlay_dc_step;
        if restarted {
            self.vector_overlay_ac_state.clear();
        }
        for (key, _, value) in samples {
            match self.vector_overlay_ac_state.entry(*key) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(VectorAcState {
                        input: *value,
                        output: Point2::default(),
                        step: completed_steps,
                    });
                    // A newly visible physical sample has no temporal history.
                    // Passing its first value through would interpret an
                    // unknown DC baseline as AC and flash whenever the view
                    // moves. Start silent; subsequent accepted samples provide
                    // the temporal difference the high-pass actually knows.
                    *value = Point2::default();
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    let state = entry.get_mut();
                    let elapsed_steps = completed_steps.saturating_sub(state.step);
                    if elapsed_steps > 0 {
                        let elapsed = elapsed_steps as f64 * self.uploaded_time_step.max(0.0);
                        let pole = (-VECTOR_DC_REJECTION_RATE * elapsed).exp();
                        if maintenance_discontinuity {
                            // The paired grid filter is a zero-duration accepted
                            // maintenance event. Its jump is not temporal field
                            // content, so rebase the input without feeding that
                            // correction through the arrow high-pass.
                            state.output = state.output * pole;
                        } else {
                            let input_gain = 0.5 * (1.0 + pole);
                            state.output =
                                state.output * pole + (*value - state.input) * input_gain;
                        }
                        state.input = *value;
                        state.step = completed_steps;
                    }
                    *value = state.output;
                }
            }
        }
        if restarted || completed_steps > self.vector_overlay_dc_step {
            self.vector_overlay_dc_step = completed_steps;
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
                    let width = if selected { 4.0 } else { 2.5 };
                    for pair in path.windows(2) {
                        painter.line_segment(
                            [self.screen(pair[0], r), self.screen(pair[1], r)],
                            Stroke::new(width, color),
                        );
                    }
                    let ring = if selected { 9.0 } else { 7.5 };
                    if let Some(badge) = Self::polyline_midpoint(&path) {
                        let badge = self.screen(badge, r);
                        painter.circle_filled(badge, if selected { 7.0 } else { 5.5 }, color);
                        painter.circle_stroke(badge, ring, Stroke::new(1.5, Color32::WHITE));
                    }
                    // A span's two traces lie on top of each other, so the
                    // scene has to say which one is read. A stem stands on that
                    // side and runs into the badge, so the reading arrives from
                    // the side the stem sits on, travelling the way positive
                    // flux points; the arclength axis leaves the middle of that
                    // stem, which keeps one mark rather than two that happen to
                    // touch.
                    if let Some((point, outward, along)) =
                        boundary_probe_orientation(&path, target.side, target.reversed)
                    {
                        let reach = 18.0;
                        let normal = screen_direction(outward);
                        // Landing the head on the ring ties the stem to the
                        // marker rather than leaving it beside the path.
                        let tail = self.screen(point, r) - normal * (ring + reach);
                        let stroke = Stroke::new(if selected { 2.0 } else { 1.5 }, color);
                        painter.arrow(tail, normal * reach, stroke);
                        painter.arrow(
                            tail + normal * (reach * 0.5),
                            screen_direction(along) * reach,
                            stroke,
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
                // Where the click will land, not where the cursor is.
                let target = self
                    .draw_attachment_hit(ScreenPoint::new(pointer.x as f64, pointer.y as f64), r)
                    .map(|hit| self.screen(hit.point, r))
                    .unwrap_or_else(|| {
                        if painter.ctx().input(|input| input.modifiers.shift) {
                            self.screen(
                                Self::snap_point(self.world(pointer, r), self.snap_step()),
                                r,
                            )
                        } else {
                            pointer
                        }
                    });
                painter.line_segment(
                    [*last, target],
                    Stroke::new(1.2, Color32::from_rgba_unmultiplied(248, 196, 112, 180)),
                );
            }
        }
        if let Some(pointer) = painter.ctx().pointer_hover_pos() {
            let current = self.world(pointer, r);
            // Where the click will land, not where the cursor is.
            let snap = painter.ctx().input(|input| input.modifiers.shift);
            match self.probe_mode {
                Some(ProbePlacement::Segment { start: Some(start) }) => {
                    let end = if snap {
                        self.screen(Self::snap_point(current, self.snap_step()), r)
                    } else {
                        pointer
                    };
                    painter.line_segment([self.screen(start, r), end], Stroke::new(1.5, TEAL));
                }
                Some(ProbePlacement::Disk {
                    center: Some(center),
                }) => {
                    painter.circle_stroke(
                        self.screen(center, r),
                        (Self::placed_disk_radius(center, current, snap, self.snap_step())
                            * self.scale) as f32,
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
        // No face restriction, for either purpose. Which side of a boundary the
        // pointer is on is not what the user is choosing by clicking it, and
        // the editor settles the face from the drawn path instead.
        let hit = weld_hit(
            compiled,
            &self.editor.document.model.draft.geometry,
            self.transform(r),
            pointer,
            self.hit_tolerance(14.0) as f64,
            None,
            None,
        )?;
        Some(hit)
    }
    fn draw_attachment_targets(&self, painter: &egui::Painter, r: Rect, draw: &DrawGesture) {
        if !matches!(draw.tool, DrawTool::Polyline | DrawTool::OpenSpline) {
            return;
        }
        let Some(compiled) = &self.editor.compiled_draft else {
            return;
        };
        // Every boundary is a target for either purpose. A separator no longer
        // has to divide something the moment it is drawn, and which side of a
        // boundary a click lands on is not a choice the user is making.
        let stroke = Stroke::new(2.2, Color32::from_rgba_unmultiplied(248, 196, 112, 105));
        for edge in &compiled.topology.edges {
            painter.line_segment(
                [
                    self.screen(edge.points[0], r),
                    self.screen(edge.points[1], r),
                ],
                stroke,
            );
        }
        for vertex in &compiled.topology.vertices {
            if vertex.authored.is_some() {
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
        self.weld(curve, node, endpoint, hit.attachment, None);
    }

    /// Welds, or stages the question first. A weld that would fold two
    /// subdomains into one face reports the regions instead of performing it,
    /// and the scene asks the same way a deletion does. Answers whether the
    /// weld is settled, so a question that still stands is not cleared.
    fn weld(
        &mut self,
        curve: CurveId,
        node: usize,
        endpoint: usize,
        attachment: TopologyAttachment,
        keep_region: Option<RegionId>,
    ) -> bool {
        let welded = match self
            .editor
            .weld_endpoint(curve, endpoint, attachment, keep_region)
        {
            Ok(TopologyWeldOutcome::Welded(weld)) => weld,
            Ok(TopologyWeldOutcome::NeedsSurvivor(choices)) => {
                self.pending_merge = Some(PendingMerge {
                    action: MergeAction::Weld {
                        curve,
                        node,
                        endpoint,
                        target: attachment,
                    },
                    choices,
                });
                return false;
            }
            Err(error) => {
                self.message = error;
                return false;
            }
        };
        self.selection = match welded.seam_control {
            Some(control) => TopologySelection::Handle(TopologyHandle::Control {
                curve: welded.curve,
                control,
            }),
            None => self
                .endpoint_control(curve, node)
                .map_or(TopologySelection::None, |control| {
                    TopologySelection::Handle(TopologyHandle::Control { curve, control })
                }),
        };
        self.invalidate_samples();
        let what = match attachment {
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
            TopologyAttachment::Boundary(FaceAnchor::Curve { .. }) => "Attached to the curve",
        };
        let mut parts = vec![what.to_owned()];
        if !welded.span_splits.is_empty() {
            parts.push("junction inserted".to_owned());
        }
        if welded.promoted {
            parts.push("curve promoted to a baffle".to_owned());
        }
        if !welded.removed_regions.is_empty() {
            parts.push(format!(
                "{} subdomain{} merged away",
                welded.removed_regions.len(),
                if welded.removed_regions.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ));
        }
        if !welded.removed_probes.is_empty() {
            parts.push(format!(
                "{} probe{} dropped",
                welded.removed_probes.len(),
                if welded.removed_probes.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ));
        }
        self.notify(parts.join(" · "));
        true
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
        if let Some(pending) = &self.pending_merge {
            // Only the candidates are clickable; everywhere else the click does
            // nothing and the cursor should not promise otherwise.
            return self
                .draft_region_at(self.world(pos, r))
                .filter(|region| pending.choices.contains(region))
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
        if self.pending_merge.is_some() {
            // The survivor question owns the viewport until it is answered or
            // cancelled: a click picks, everything else waits.
            if response.clicked_by(egui::PointerButton::Primary)
                && let Some(pos) = pointer
            {
                self.pick_merge_survivor(self.world(pos, r));
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
                        ui.input(|input| input.modifiers.shift),
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
        if self.probe_mode.is_some() && response.clicked() {
            if let Some(pos) = pointer {
                self.probe_placement_click(
                    self.world(pos, r),
                    ui.input(|input| input.modifiers.shift),
                );
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
                let step = self.snap_step();
                let result = match drag {
                    DragGesture::Endpoint { curve, node, .. } => {
                        let point = if shift {
                            Self::snap_point(point, step)
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
                            Self::snap_point(point, step)
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
                            Self::snap_point(point, step)
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
                            Self::snap_point(pivot + point - start, step)
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
                            Self::snap_point(point + offset, step)
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
                                    Self::snap_point(point, step)
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
    fn draw_click(&mut self, mut point: Point2, screen: ScreenPoint, r: Rect, snap_to_grid: bool) {
        let hit = self.draw_attachment_hit(screen, r);
        let Some(mut gesture) = self.draw.take() else {
            return;
        };
        let open = matches!(gesture.tool, DrawTool::Polyline | DrawTool::OpenSpline);
        let mut attachment = None;
        // An attachment is a snap of its own and outranks the grid: the point
        // being welded to is where the curve has to land.
        if open && let Some(hit) = hit {
            point = hit.point;
            attachment = Some(hit.attachment);
        } else if snap_to_grid {
            point = Self::snap_point(point, self.snap_step());
        }
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
        // The whole selection is one removal, planned once. Which subdomains a
        // deletion merges is a property of all of it together, so asking curve
        // by curve asked the wrong question, closed the history entry to ask it,
        // and left everything after the first question undeleted.
        let target = match self.editor.removal_target(&spans) {
            Ok(target) => target,
            Err(error) => {
                self.message = error;
                return;
            }
        };
        let choices = match self.editor.removal_choices(&target) {
            Ok(choices) => choices,
            Err(error) => {
                self.message = error;
                return;
            }
        };
        // Merging two assigned subdomains needs an explicit survivor, so hand
        // the choice to the scene instead of failing the gesture.
        if choices.len() > 1 {
            self.pending_merge = Some(PendingMerge {
                action: MergeAction::Delete(spans),
                choices,
            });
            return;
        }
        match self.editor.remove(&target, choices.first().copied()) {
            Ok(removal) => {
                self.selection = TopologySelection::None;
                self.material_edit = None;
                self.material_formula_edits.clear();
                self.material_formula_errors.clear();
                self.invalidate_samples();
                self.report_removal(&target, &removal);
            }
            Err(error) => self.message = error,
        }
    }
    /// Says what the deletion did beyond the selection: a curve promoted to a
    /// baffle or a probe dropped is not something to discover later.
    fn report_removal(&mut self, target: &TopologyRemovalTarget, removal: &TopologyRemoval) {
        let curves = target.whole_curves();
        let pieces = removal.pieces.len();
        let lead = match (curves, pieces) {
            (0 | 1, 0) => "Curve deleted".to_owned(),
            (0, 1) => "Deleted spans; the rest is a baffle".to_owned(),
            (0, pieces) => format!("Deleted spans; split into {pieces} baffles"),
            (curves, 0) => format!("{curves} curves deleted"),
            (curves, pieces) => format!(
                "{curves} curve{} deleted and one cut, leaving {pieces} baffle{}",
                if curves == 1 { "" } else { "s" },
                if pieces == 1 { "" } else { "s" },
            ),
        };
        let TopologyRemoval {
            promoted,
            joined,
            removed_probes,
            removed_regions,
            ..
        } = removal;
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
    /// Wall time a frame lends to topology preparation. The runtime checks the
    /// deadline between individual work units, including each formula-heavy
    /// canonical element, so this remains a latency bound rather than only an
    /// average throughput target.
    const PREPARATION_FRAME_BUDGET: std::time::Duration = std::time::Duration::from_millis(4);

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
        let fresh = starts_from_zero(
            self.runtime.active().is_some(),
            self.reset_requested,
            self.fresh_requested,
        );
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
                self.fresh_requested = false;
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
        // A fallback is only in the record until the next transaction replaces
        // it, and the record says nothing about how often one has happened.
        if let Some(fallback) = &active.repair_fallback {
            self.pending_repairs.push(fallback.clone());
        }
        self.handoff_requested = None;
        self.handoff_ready = None;
        self.handoff_upload = None;
    }
    fn refresh_runtime(
        &mut self,
        request: &mut CanonicalGpuRequest,
        display: &CanonicalGpuDisplay,
        recorders: &mut WaveGpuRequest,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        delta: f64,
    ) {
        request.set_grid_scale_filter(self.grid_scale_filter);
        // Starting another preparation mid-upload clears `runtime.ready`, and
        // would make the accepted GPU generation impossible to publish under
        // its immutable topology token. A later frame picks the edit up.
        if self.uploading.is_none() && self.gpu_upload_preparation.is_none() {
            self.retime_for_speed();
            self.request_runtime();
        }
        if let Some(Ok(_)) = self.runtime.advance_for(Self::PREPARATION_FRAME_BUDGET) {
            self.handoff_ready = Some(Instant::now());
        }
        if self.gpu_upload_preparation.as_ref().is_some_and(|job| {
            self.runtime
                .ready()
                .is_none_or(|candidate| candidate.bundle.token != job.token)
        }) {
            self.gpu_upload_preparation = None;
        }
        if self.uploading.is_none()
            && self.gpu_upload_preparation.is_none()
            && let Some(candidate) = self.runtime.ready().cloned()
        {
            let dt = paced_time_step(
                candidate.canonical_operator.recommended_time_step(),
                self.editor.document.presentation.simulation_speed,
            );
            let token = candidate.bundle.token;
            let active = self.runtime.active().cloned();
            let runtime_serials = display.runtime_serials;
            let (sender, receiver) = mpsc::channel();
            #[cfg(not(target_arch = "wasm32"))]
            let _ = std::thread::Builder::new()
                .name("funfern-gpu-pack".into())
                .spawn(move || {
                    let _ = sender.send(compile_gpu_upload(candidate, active, dt, runtime_serials));
                });
            #[cfg(target_arch = "wasm32")]
            let _ = sender.send(compile_gpu_upload(candidate, active, dt, runtime_serials));
            self.gpu_upload_preparation = Some(GpuUploadPreparation {
                token,
                time_step: dt,
                receiver: Mutex::new(receiver),
                result: None,
            });
        }
        if let Some(job) = &mut self.gpu_upload_preparation
            && job.result.is_none()
        {
            let received = job.receiver.lock().unwrap().try_recv();
            match received {
                Ok(result) => job.result = Some(result),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    job.result = Some(Err("Canonical GPU packing worker stopped".into()));
                }
            }
        }
        if self.uploading.is_none()
            && request.caught_up()
            && self
                .gpu_upload_preparation
                .as_ref()
                .is_some_and(|job| job.result.is_some())
        {
            let mut job = self.gpu_upload_preparation.take().unwrap();
            let token = job.token;
            let dt = job.time_step;
            let Some(candidate) = self
                .runtime
                .ready()
                .filter(|candidate| candidate.bundle.token == token)
                .cloned()
            else {
                return;
            };
            let upload = job.result.take().unwrap().and_then(|prepared| {
                if let Some(transfer) = prepared.transfer {
                    request
                        .begin_handoff(assets, commands, prepared.plan, transfer)
                        .map_err(str::to_owned)
                } else {
                    request.install(assets, commands, prepared.plan);
                    Ok(())
                }
            });
            match upload {
                Ok(()) => {
                    self.uploaded_time_step = dt;
                    self.handoff_upload = Some(Instant::now());
                    self.uploading = Some(Uploading {
                        token,
                        generation: if candidate.fresh {
                            request.generation()
                        } else {
                            request.generation().wrapping_add(1).max(1)
                        },
                        fresh: candidate.fresh,
                        degrees_of_freedom: candidate.canonical_operator.degrees_of_freedom(),
                    });
                }
                Err(error) => {
                    self.runtime.reject_ready(token, error.clone());
                    self.message = error;
                }
            }
        }
        if let Some(upload) = &self.uploading {
            if request.failed()
                || matches!(
                    request.handoff_outcome(),
                    CanonicalGpuHandoffOutcome::Rejected(_)
                )
            {
                let token = upload.token;
                self.runtime
                    .reject_ready(token, "Canonical GPU upload failed");
                self.uploading = None;
            } else if request.ready()
                && display.generation == upload.generation
                && display.primary_flux.len() == upload.degrees_of_freedom
                && !matches!(
                    request.handoff_outcome(),
                    CanonicalGpuHandoffOutcome::Pending
                )
            {
                let upload = self.uploading.take().unwrap();
                match self.runtime.commit_ready(upload.token) {
                    Ok(active) => {
                        if upload.fresh {
                            self.accumulator = 0.0;
                            self.restart_probe_traces();
                            self.canonical_event_serial = 0;
                            self.canonical_event_observed = 0;
                            self.wave_energy = None;
                            self.amr_energy_peak = 0.0;
                        }
                        self.restart_exposures_after_handoff(upload.fresh);
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
                    Err(error) => self.message = error,
                }
            }
        }
        if self.reset_requested && self.uploading.is_none() {
            if let Some(active) = self.runtime.active() {
                let dt = paced_time_step(
                    active.canonical_operator.recommended_time_step(),
                    self.editor.document.presentation.simulation_speed,
                );
                let reset = CanonicalWaveState::zero(&active.canonical_operator, dt)
                    .map_err(|error| error.to_string())
                    .and_then(|state| {
                        CanonicalGpuPlan::compile_with_quadratic(
                            &active.canonical_operator,
                            &active.operator,
                            &state,
                            &active.canonical_forcing,
                            CanonicalGpuClock::initial(dt).map_err(|error| format!("{error:?}"))?,
                        )
                        .map_err(|error| format!("{error:?}"))
                    });
                if let Ok(plan) = reset {
                    request.install(assets, commands, plan);
                    self.reset_requested = false;
                    self.uploaded_time_step = dt;
                    self.sim_time_offset = 0.0;
                    self.canonical_event_serial = 0;
                    self.canonical_event_observed = 0;
                    self.restart_probe_traces();
                }
            }
        }
        recorders.adopt_canonical_generation(request.generation());
        let overlay_active = self.runtime.active().cloned().filter(|active| {
            display.generation == request.generation()
                && display.primary_flux.len() == active.canonical_operator.degrees_of_freedom()
        });
        self.refresh_vector_overlay(
            recorders,
            assets,
            commands,
            overlay_active.as_ref(),
            request.generation(),
        );
        if self.uploading.is_none()
            && let Some(active) = self.runtime.active().cloned()
            && probes_need_upload(self.probe_upload, active.bundle.token, request.generation())
        {
            self.configure_probes(recorders, assets, commands, &active);
        }
        if let Some(active) = self.runtime.active() {
            let dt = self.solver_time_step();
            if let Some((position, region)) = self
                .uploading
                .is_none()
                .then(|| self.pending_pulse.take())
                .flatten()
            {
                let mut increment = vec![0.0; active.canonical_operator.degrees_of_freedom()];
                for (triangle, nodes) in active
                    .mesh
                    .triangles
                    .iter()
                    .zip(active.canonical_operator.element_nodes())
                {
                    if triangle.region != region {
                        continue;
                    }
                    for node in nodes {
                        let delta =
                            active.canonical_operator.node_points()[*node as usize] - position;
                        increment[*node as usize] = f64::from(self.pulse_amplitude)
                            * (-0.5 * delta.dot(delta) / f64::from(self.pulse_width).powi(2)).exp();
                    }
                }
                self.canonical_event_serial = self
                    .canonical_event_serial
                    .max(request.stats().processed_event())
                    .saturating_add(1)
                    .max(1);
                match CanonicalGpuLiveEvent::primary_pulse(
                    &active.canonical_operator,
                    &increment,
                    self.canonical_event_serial,
                )
                .map_err(|error| format!("{error:?}"))
                .and_then(|event| {
                    request
                        .queue_live_event(assets, event)
                        .map_err(str::to_owned)
                }) {
                    Ok(()) => {}
                    Err(error) => self.message = error,
                }
            }
            // Drain once the packed candidate is ready so begin_handoff gets a
            // complete requested-step boundary. After that, keep advancing the
            // accepted generation while the target assets upload; the render
            // graph snapshots one complete boundary while later requests keep
            // the source display live; the target consumes that short backlog
            // after admission. Fresh installs have no outgoing generation.
            let packed_candidate_waiting = self.uploading.is_none()
                && self
                    .gpu_upload_preparation
                    .as_ref()
                    .is_some_and(|job| job.result.is_some());
            let fresh_upload = self.uploading.as_ref().is_some_and(|upload| upload.fresh);
            if !canonical_steps_withheld(packed_candidate_waiting, fresh_upload) {
                if self.wave_running {
                    let steps = steps_for_frame(
                        &mut self.accumulator,
                        delta,
                        self.editor.document.presentation.simulation_speed,
                        dt,
                    );
                    if steps > 0 {
                        request.request_steps(steps);
                    }
                } else if self.wave_step {
                    request.request_steps(1);
                    self.wave_step = false;
                }
                self.speed_reached =
                    hold_rate(self.speed_reached, self.steps_per_second * dt, delta);
            }
        }
        self.completed_steps = request.stats().completed_steps();
        self.gpu_status = request.stats().status();
        self.gpu_dispatches = request.stats().dispatches();
        let processed_event = request.stats().processed_event();
        if processed_event != self.canonical_event_observed {
            self.canonical_event_observed = processed_event;
            let rejection = request.stats().event_rejection();
            if rejection != 0 {
                self.unseen_error = true;
                self.notify(format!(
                    "Canonical event {processed_event} was rejected without changing the accepted state (failure code {rejection})"
                ));
            }
        }
        self.canonical_gpu_bytes = request
            .manifest()
            .map(|manifest| manifest.bytes.steady_bytes());
        self.step_backlog = request
            .requested_steps()
            .saturating_sub(self.completed_steps);
        // Full physical snapshots are for AMR and energy diagnostics. The
        // vector overlay has its own compact display-rate GPU sampler.
        let full_snapshot_interval = 0.25;
        if self.full_snapshot_requested.elapsed().as_secs_f64() >= full_snapshot_interval
            && request.request_full_state_readback(commands)
        {
            self.full_snapshot_requested = Instant::now();
        }
        if let Some(active) = self.runtime.active()
            && display.generation == request.generation()
            && display.primary_flux.len() == active.canonical_operator.degrees_of_freedom()
        {
            if display.full_readbacks != self.energy_readback
                && display.full_readback_at == display.readbacks
                && self.energy_updated.elapsed().as_secs_f64() >= 0.25
            {
                let primary = display
                    .primary_flux
                    .iter()
                    .map(|value| f64::from(*value))
                    .collect::<Vec<_>>();
                let complementary = display
                    .complementary_flux
                    .iter()
                    .map(|value| Point2::new(f64::from(value[0]), f64::from(value[1])))
                    .collect::<Vec<_>>();
                let auxiliary = display
                    .auxiliary
                    .iter()
                    .map(|value| f64::from(*value))
                    .collect::<Vec<_>>();
                let energy = canonical_energy_breakdown(
                    &active.canonical_operator,
                    &primary,
                    &complementary,
                    &auxiliary,
                )
                .ok()
                .map(CanonicalEnergyBreakdown::total);
                if let Some(energy) = energy.filter(|energy| energy.is_finite() && *energy > 0.0) {
                    // Full snapshots run throughout the simulation, whether
                    // AMR is currently enabled or not. Preserve that history
                    // so AMR enabled after a pulse does not establish its
                    // "run" peak from the numerical tail it is meant to ignore.
                    self.amr_energy_peak = self.amr_energy_peak.max(energy);
                }
                self.wave_energy = energy;
                self.energy_readback = display.full_readbacks;
                self.energy_updated = Instant::now();
            }
            if let Some(clock) = display.clock {
                self.sim_time_offset = clock.absolute_seconds
                    - self.completed_steps as f64 * f64::from(clock.time_step);
            }
        }
        self.accumulate_step_rate(
            request.generation(),
            self.completed_steps,
            self.rate_started.elapsed().as_secs_f64(),
        );
    }
    /// Banks the progress the solver made since the previous frame and closes
    /// the averaging window when it is full.
    ///
    /// Accepted-step totals survive a handover, while a fresh install/reset may
    /// start a new generation at zero. The first observation of any generation
    /// is therefore a baseline, not progress: counting its absolute total
    /// credited the whole run again after every adaptive handover and could
    /// leave the reported real-time rate falsely high for many seconds.
    fn accumulate_step_rate(&mut self, generation: u64, completed: u64, elapsed: f64) {
        if self.rate_generation != generation {
            self.rate_generation = generation;
            self.rate_steps = completed;
        } else {
            self.rate_window_steps += completed.saturating_sub(self.rate_steps);
            self.rate_steps = completed;
        }
        if elapsed >= STEP_RATE_WINDOW {
            self.steps_per_second = self.rate_window_steps as f64 / elapsed;
            self.rate_window_steps = 0;
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
        let dt = self.solver_time_step();
        let physics = active.bundle.authored.physics;
        if self
            .probe_history_physics
            .is_some_and(|previous| previous != physics)
        {
            self.restart_probe_traces();
        }
        self.probe_history_physics = Some(physics);
        // Every recorder writes a time series on the solver's own clock, and a
        // transfer carries that clock across. So a new mesh over the same
        // recorders takes over the rings they were filling, and only a clock
        // that restarted starts them again. Without that the samples the GPU
        // wrote since the last readback went with the buffers - two to four of
        // them at 120 Hz, on every adaptation.
        let restarted = std::mem::take(&mut self.probe_clock_restarted);
        let history = if restarted {
            RecorderHistory::Restart
        } else {
            RecorderHistory::Keep
        };
        let context = RecorderContext {
            time_step: dt,
            physics,
            history,
        };
        let result = request
            .update_canonical_point_probes(
                assets,
                commands,
                &active.canonical_operator,
                &points,
                120.0,
                context,
            )
            .and_then(|()| {
                request.update_canonical_curve_probes(
                    assets,
                    commands,
                    &active.canonical_operator,
                    &curves,
                    context,
                )
            })
            .and_then(|()| {
                request.update_canonical_area_probes(
                    assets,
                    commands,
                    &active.canonical_operator,
                    &areas,
                    60.0,
                    context,
                )
            });
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
        // The far field records at fixed world points rather than at a probe,
        // so a contour that moved starts its delay window again even when the
        // clock did not.
        match request.update_canonical_far_field(assets, commands, far.as_ref(), dt, history) {
            Ok(FarFieldHandoff::Restarted) => {
                self.far_field_trace = FarFieldTrace::default();
                self.far_field_recording_from = Some(self.simulated_time());
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
        // A restarted clock disowns the run before it; anything else leaves the
        // last upload addressable, for the readback still on its way here.
        self.probe_upload_previous = (!restarted).then_some(self.probe_upload).flatten();
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
        self.probe_upload_previous = None;
    }
    /// The step the GPU is actually running at.
    fn solver_time_step(&self) -> f64 {
        if self.uploaded_time_step > 0.0 {
            return self.uploaded_time_step;
        }
        self.runtime
            .active()
            .map_or(0.0, |active| active.operator.recommended_time_step())
    }

    /// Republishes when the speed ceiling wants a different step from the one
    /// the solver is running.
    ///
    /// The scene is unchanged, so the preparation reuses the plan, the mesh and
    /// the operator and the field crosses on the identity transfer — the same
    /// path an adaptation handoff takes, which already changes the step every
    /// time it runs. Clearing the requested revision is the lever a document
    /// load pulls.
    fn retime_for_speed(&mut self) {
        if self.uploaded_time_step <= 0.0
            || self.uploading.is_some()
            || self.runtime.phase().is_some()
            || self.runtime.ready().is_some()
        {
            return;
        }
        let Some(active) = self.runtime.active() else {
            return;
        };
        let wanted = paced_time_step(
            active.operator.recommended_time_step(),
            self.editor.document.presentation.simulation_speed,
        );
        if (wanted / self.uploaded_time_step - 1.0).abs() > TIME_STEP_HYSTERESIS {
            self.requested_revision = None;
        }
    }

    fn simulated_time(&self) -> f64 {
        self.sim_time_offset + self.completed_steps as f64 * self.solver_time_step()
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
    fn refresh_amr(
        &mut self,
        request: &CanonicalGpuRequest,
        canonical: &CanonicalGpuDisplay,
        display: &WaveDisplay,
    ) {
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
            let Some(source) = source else {
                self.amr_status = "discarded stale estimate".into();
                return;
            };
            let Some(active) = self.runtime.active().cloned() else {
                return;
            };
            if !source.is_current(active.bundle.token, request.generation()) {
                self.amr_status = "discarded stale estimate".into();
                return;
            }
            self.amr_last_analyzed_step = Some(source.accepted_step);
            let result = match result {
                Ok(result) => result,
                Err(error) => {
                    self.amr_status = "estimate failed".into();
                    self.amr_error = Some(error.to_string());
                    return;
                }
            };
            if result.report.total_energy.is_finite() && result.report.total_energy > 0.0 {
                self.amr_energy_peak = self.amr_energy_peak.max(result.report.total_energy);
            }
            let refine = adaptation_refines(&result.report, self.amr_target_accuracy());
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
            || display.snapshot_current.len() != dofs
            || display.snapshot_previous.len() != dofs
            || display.auxiliary.len() != dofs
            || display.snapshot_velocity.len() != dofs
            || canonical.previous_primary_flux.len() != dofs
            || canonical.previous_complementary_flux.len()
                != active.canonical_operator.complementary_degrees_of_freedom()
            || canonical.previous_auxiliary.len() != canonical.auxiliary.len()
            || canonical.full_readback_at != canonical.readbacks
        {
            self.amr_status = "waiting for aligned readback".into();
            return;
        }
        let step = display.snapshot_completed_steps;
        if resident_filter_boundary(self.grid_scale_filter, step) {
            // The resident filter is accepted at this same solver step and
            // flips the state lanes once more. At that instant the other lane
            // is the pre-filter state, not the endpoint one `dt` earlier.
            // Let the next ordinary step restore the endpoint contract instead
            // of reporting the deliberate damping correction as wave error.
            self.amr_status = "waiting for post-filter endpoint".into();
            return;
        }
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
        let dt = self.solver_time_step();
        let time = canonical.clock.map_or(self.simulated_time(), |clock| {
            clock.absolute_seconds + (step as f64 - f64::from(clock.accepted_steps)) * dt
        });
        let displacement = display
            .snapshot_current
            .iter()
            .map(|value| f64::from(*value))
            .collect::<Vec<_>>();
        let velocity = display
            .snapshot_velocity
            .iter()
            .map(|value| f64::from(*value))
            .collect::<Vec<_>>();
        let stiffness = match active.operator.apply_stiffness(&displacement) {
            Ok(stiffness) => stiffness,
            Err(error) => {
                self.amr_status = "canonical estimate failed".into();
                self.amr_error = Some(error.to_string());
                return;
            }
        };
        let volume_acceleration = active.volume_sources.acceleration(time);
        let acceleration = stiffness
            .iter()
            .zip(active.operator.lumped_mass())
            .zip(active.canonical_operator.primary_loss_rate())
            .zip(&velocity)
            .zip(&volume_acceleration)
            .map(|((((force, mass), loss), velocity), source)| {
                source - force / mass - loss * velocity
            })
            .collect::<Vec<_>>();
        let Some(auxiliary) = aligned_indicator_auxiliary(display, &active.operator, dt, step)
        else {
            self.amr_status = "waiting for aligned readback".into();
            return;
        };
        let snapshot = QuadraticSolutionSnapshot {
            mesh_revision: active.mesh.mesh_revision,
            displacement,
            velocity,
            acceleration,
            auxiliary,
            volume_acceleration,
            time,
            time_step: dt,
        };
        let canonical_snapshot = CanonicalIndicatorSnapshot {
            mesh_revision: active.mesh.mesh_revision,
            primary_flux: canonical
                .primary_flux
                .iter()
                .map(|value| f64::from(*value))
                .collect(),
            previous_primary_flux: canonical
                .previous_primary_flux
                .iter()
                .map(|value| f64::from(*value))
                .collect(),
            complementary_flux: canonical
                .complementary_flux
                .iter()
                .map(|value| Point2::new(f64::from(value[0]), f64::from(value[1])))
                .collect(),
            previous_complementary_flux: canonical
                .previous_complementary_flux
                .iter()
                .map(|value| Point2::new(f64::from(value[0]), f64::from(value[1])))
                .collect(),
            auxiliary: canonical
                .auxiliary
                .iter()
                .map(|value| f64::from(*value))
                .collect(),
            previous_auxiliary: canonical
                .previous_auxiliary
                .iter()
                .map(|value| f64::from(*value))
                .collect(),
            time,
            time_step: dt,
        };
        let supplement = match canonical_indicator_supplement(
            &active.mesh,
            &active.canonical_operator,
            &active.canonical_forcing,
            &canonical_snapshot,
        ) {
            Ok(supplement) => supplement,
            Err(error) => {
                self.amr_status = "canonical estimate failed".into();
                self.amr_error = Some(error.to_string());
                return;
            }
        };
        self.amr_indicator_job = Some(
            SolutionIndicatorJob::new_topology(
                active.mesh.clone(),
                active.operator.clone(),
                &active.bundle.plan,
                active.bundle.model(),
                snapshot,
                SolutionIndicatorOptions {
                    minimum_edge_length: self.amr_minimum_edge,
                    maximum_edge_length: self.amr_maximum_edge,
                    relative_tolerance: self.amr_target_accuracy(),
                    elements_per_wavelength: self.amr_elements_per_wavelength,
                    forcing_frequency_hz: highest_forcing_frequency(
                        &active.bundle.authored,
                        active.point_source,
                    ),
                    dormant_below_energy: self.amr_energy_peak * DORMANT_ENERGY_RATIO,
                    ..Default::default()
                },
            )
            .with_canonical_supplement(supplement),
        );
        self.amr_indicator_source = Some(AmrIndicatorSource {
            topology: active.bundle.token,
            gpu_generation: request.generation(),
            accepted_step: step,
        });
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
        [&self.probe_upload, &self.probe_upload_previous]
            .into_iter()
            .flatten()
            .any(|upload| upload.generation == generation && recorded(upload) == revision)
    }
    /// Every recorder stamps its samples with the solver's own clock, which the
    /// transfer carries into the buffers that replace it. That clock is already
    /// the app's, and adding `sim_time_offset` to it counted the run so far a
    /// second time — a jump the width of the previous mesh's whole lifetime at
    /// every handoff, which is what put the holes in these traces.
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
            let record = *record;
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
                let record = record.clone();
                if record.time > trace.last_time {
                    trace.last_time = record.time;
                    trace.records.push_back(record);
                    while trace.records.len() > CURVE_TRACE_FRAMES {
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
                let record = *record;
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
            || self.pending_merge.is_some()
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
    /// Point a fraction of the way along a polyline by arclength, with the
    /// segment carrying it - which is what orients anything drawn there.
    fn polyline_anchor(path: &[Point2], fraction: f64) -> Option<(Point2, [Point2; 2])> {
        let total: f64 = path.windows(2).map(|pair| (pair[1] - pair[0]).norm()).sum();
        let mut remaining = total * fraction;
        for pair in path.windows(2) {
            let length = (pair[1] - pair[0]).norm();
            if remaining <= length {
                let fraction = if length > 0.0 {
                    remaining / length
                } else {
                    0.0
                };
                return Some((pair[0].lerp(pair[1], fraction), [pair[0], pair[1]]));
            }
            remaining -= length;
        }
        None
    }
    /// Point half way along a polyline by arclength, used to badge and hit the
    /// probe where the eye expects it rather than at an arbitrary end.
    fn polyline_midpoint(path: &[Point2]) -> Option<Point2> {
        Self::polyline_anchor(path, 0.5)
            .map(|(point, _)| point)
            .or_else(|| path.first().copied())
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
    /// The radius a second placement click asks for. Snapping puts the radius
    /// itself on the grid rather than the point it was measured to, which is
    /// what dragging a disk's rim already does - snapping the rim point would
    /// leave a radius that is no multiple of anything.
    fn placed_disk_radius(center: Point2, point: Point2, snap: bool, step: f64) -> f64 {
        let radius = (point - center).norm();
        if snap {
            // A click inside the first grid step would round the disk away.
            Self::snap_scalar(radius, step).max(step)
        } else {
            radius
        }
    }
    fn snap_scalar(value: f64, step: f64) -> f64 {
        (value / step).round() * step
    }

    fn drag_probe(
        &mut self,
        hit: ProbeHit,
        original: &TopologyProbeTarget,
        delta: Point2,
        snap: bool,
    ) {
        let step = self.snap_step();
        let place = |point: Point2| {
            if snap {
                Self::snap_point(point + delta, step)
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
                    Self::snap_scalar(grown, step)
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
    /// so the readouts can report a precise reason and a real arclength axis,
    /// and places each subdomain probe's marker from the compiled geometry.
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
        // Compilation status and path metrics mirror a committed candidate, so
        // they change only when one is published or the probe set moves. Region
        // anchors come from the compiled geometry instead, which is why the
        // token survives having no candidate at all.
        let token = (
            self.runtime.active().map(|active| active.bundle.token),
            self.editor.revision,
        );
        if self.probe_metadata_token == Some(token) {
            return;
        }
        self.probe_metadata_token = Some(token);
        let scene = self
            .editor
            .compiled_draft
            .as_ref()
            .unwrap_or(&self.editor.compiled_accepted);
        self.probe_anchors = self
            .editor
            .document
            .model
            .probes
            .iter()
            .filter_map(|probe| match probe.target {
                TopologyProbeTarget::AreaRegion(region) => {
                    Some((probe.id, region_anchor(scene, region)?))
                }
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        let Some(active) = self.runtime.active().cloned() else {
            return;
        };
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
            if let TopologyProbeTarget::Segment { start, end, .. } = probe.target {
                self.probe_metrics
                    .insert(probe.id, ((end - start).norm(), false));
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
            LineProbeQuantity::Flux | LineProbeQuantity::MeanFlux => &frame.normal_flux,
            LineProbeQuantity::Energy => &frame.energy_density,
        }
    }
    /// A causal trailing mean of the recorded normal flux: every frame carries
    /// the average of the `window` seconds ending at its own time, sample point
    /// by sample point. The instantaneous flux of a standing wave swings
    /// symmetrically about zero at twice the driven frequency, so only this
    /// average says how much power a path actually carries.
    ///
    /// The window is fixed rather than taken from the visible one, so panning
    /// and zooming move over the same numbers instead of rewriting them. A
    /// frame whose window the record does not cover in full yields a row of
    /// NaN, which keeps the series one row per recorded frame: every
    /// representation then holds the raw series' time alignment, and the
    /// renderers already skip what is not finite. The second return is how much
    /// of the window the newest frame holds, for the readout to report while it
    /// is still filling.
    /// The longest trailing window the trace can ever cover.
    ///
    /// The ring holds a fixed number of frames, so how much time it spans
    /// depends on the interval between them - and that is not the preset's
    /// nominal rate. The recorder strides the solver's own steps,
    /// `round(1 / (rate * dt))` of them, so a coarse enough time step rounds
    /// the stride down and oversamples: at the default mesh a 120 Hz preset
    /// records every step, about 149 Hz, and 512 frames reach back 3.4 seconds
    /// where the nominal rate promises 4.3.
    ///
    /// The interval is measured as the smallest gap in the trace. A dropped
    /// readback inflates an average and would put the limit back out of reach,
    /// while the smallest gap is still the true stride; if the time step
    /// changed inside the ring it takes the shorter of the two, which errs
    /// short. One interval is held back so the newest frame's window begins at
    /// or before a frame the ring holds rather than exactly on one.
    fn curve_mean_window_limit(frames: &[CurveProbeRecord], nominal: f64) -> f64 {
        let interval = frames
            .windows(2)
            .map(|pair| pair[1].time - pair[0].time)
            .filter(|gap| gap.is_finite() && *gap > 0.0)
            .fold(f64::INFINITY, f64::min);
        let interval = if interval.is_finite() {
            interval
        } else {
            nominal
        };
        interval * (CURVE_TRACE_FRAMES - 2) as f64
    }
    fn curve_probe_running_mean(
        frames: &[CurveProbeRecord],
        window: f64,
    ) -> (Vec<CurveProbeRecord>, f64) {
        if frames.is_empty() || !window.is_finite() || window <= 0.0 {
            return (Vec::new(), 0.0);
        }
        let mut means = Vec::with_capacity(frames.len());
        let mut sums: Vec<f64> = Vec::new();
        let mut counts: Vec<u32> = Vec::new();
        // The first frame of the run of equal-width records the accumulator was
        // built for, and the oldest frame still inside the window.
        let mut run = 0;
        let mut oldest = 0;
        let mut filled = 0.0;
        for (index, frame) in frames.iter().enumerate() {
            if frame.normal_flux.len() != sums.len() {
                // A sampling-preset change or a boundary remesh leaves rows of
                // another length in the same trace, and two layouts have no
                // common average. The window starts again here.
                sums = vec![0.0; frame.normal_flux.len()];
                counts = vec![0; frame.normal_flux.len()];
                run = index;
                oldest = index;
            }
            for (point, value) in frame.normal_flux.iter().enumerate() {
                if value.is_finite() {
                    sums[point] += *value as f64;
                    counts[point] += 1;
                }
            }
            let begin = frame.time - window;
            while oldest < index && frames[oldest].time < begin {
                for (point, value) in frames[oldest].normal_flux.iter().enumerate() {
                    if value.is_finite() {
                        sums[point] -= *value as f64;
                        counts[point] -= 1;
                    }
                }
                oldest += 1;
            }
            // Either a frame has already left the window, or the run itself
            // reaches back past its start. Anything else is a partial average
            // of whatever happens to be recorded, which is not what the row
            // claims to show.
            let covered = oldest > run || frames[run].time <= begin;
            filled = if covered {
                1.0
            } else {
                ((frame.time - frames[run].time) / window).clamp(0.0, 1.0)
            };
            means.push(CurveProbeRecord {
                probe_id: frame.probe_id,
                time: frame.time,
                normal_flux: counts
                    .iter()
                    .zip(&sums)
                    .map(|(count, sum)| {
                        if covered && *count > 0 {
                            (sum / *count as f64) as f32
                        } else {
                            f32::NAN
                        }
                    })
                    .collect(),
                ..Default::default()
            });
        }
        (means, filled)
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
        let waiting = |painter: &egui::Painter, message: &str| {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                message,
                egui::FontId::monospace(11.0),
                Color32::from_rgb(112, 130, 143),
            );
        };
        let Some(frame) = frames.iter().min_by(|a, b| {
            (a.time - view.end_time)
                .abs()
                .total_cmp(&(b.time - view.end_time).abs())
        }) else {
            waiting(ui.painter(), "Waiting for samples");
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
            // Every sample at this time is invalid: outside the domain, on a
            // two-trace boundary, or - for the averaged row - earlier than the
            // first full window.
            waiting(ui.painter(), "No valid samples at this time");
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
            // The averaged flux row can only look back as far as the trace
            // reaches, which at the faster presets is well short of the history
            // slider. Offering more than that would be a setting that shows
            // nothing however long the run continues.
            let mean_limit = match &probe.target {
                TopologyProbeTarget::Segment { preset, .. } => Some(*preset),
                TopologyProbeTarget::Boundary(target) => Some(target.preset),
                _ => None,
            }
            .map_or(history, |preset| {
                history.min(Self::curve_mean_window_limit(
                    &curve_frames,
                    1.0 / preset.sample_rate(),
                ))
            })
            .max(0.2);
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
            view.mean_window = view.mean_window.clamp(0.05, mean_limit);
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
                                    ui.add(
                                        egui::Slider::new(&mut view.mean_window, 0.05..=mean_limit)
                                            .logarithmic(true)
                                            .text("Mean window"),
                                    )
                                    .on_hover_text(
                                        "Seconds the averaged flux row looks back over. \
                                         Cover several periods of the flux, which swings at \
                                         twice the driven frequency. It stops at what the \
                                         recorded trace can reach back over.",
                                    );
                                });
                        } else if is_area {
                            let active = [
                                view.area_mean_field,
                                view.area_rms_field,
                                view.area_rms_transverse,
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
                                    ui.checkbox(
                                        &mut view.area_rms_transverse,
                                        format!(
                                            "RMS {}",
                                            transverse_field_magnitude_label(physics)
                                        ),
                                    );
                                    ui.checkbox(&mut view.area_mean_energy, "Mean energy density");
                                    ui.checkbox(
                                        &mut view.area_total_energy,
                                        total_energy_label(physics),
                                    );
                                });
                        } else {
                            let active = [
                                view.field,
                                view.secondary_field,
                                view.transverse_field,
                                view.poynting,
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
                                        primary_field_rate_label(physics),
                                    );
                                    ui.checkbox(
                                        &mut view.transverse_field,
                                        transverse_field_magnitude_label(physics),
                                    );
                                    ui.checkbox(
                                        &mut view.poynting,
                                        energy_flow_magnitude_label(physics),
                                    );
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
                        // One pass over the trace serves all three averaged
                        // views, and none of them asks for it unless drawn.
                        let averaged =
                            LineProbeRepresentation::ALL
                                .into_iter()
                                .any(|representation| {
                                    view.line_plots[LineProbeQuantity::MeanFlux.offset()
                                        + representation.offset()]
                                });
                        let (mean_frames, mean_filled) = if averaged {
                            Self::curve_probe_running_mean(&curve_frames, view.mean_window)
                        } else {
                            (Vec::new(), 1.0)
                        };
                        if averaged && !curve_frames.is_empty() && mean_filled < 0.999 {
                            ui.colored_label(
                                GOLD,
                                format!(
                                    "Filling the {:.2} s mean window · {:.0}%",
                                    view.mean_window,
                                    mean_filled * 100.0
                                ),
                            );
                        }
                        for quantity in LineProbeQuantity::ALL {
                            if !quantity.applies(physics) {
                                continue;
                            }
                            let frames = if quantity.averaged() {
                                &mean_frames
                            } else {
                                &curve_frames
                            };
                            if view.line_plots
                                [quantity.offset() + LineProbeRepresentation::Arclength.offset()]
                            {
                                Self::curve_probe_profile(
                                    ui, frames, &view, quantity, length, physics,
                                );
                            }
                            if view.line_plots
                                [quantity.offset() + LineProbeRepresentation::Waterfall.offset()]
                            {
                                Self::curve_probe_waterfall(
                                    ui,
                                    frames,
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
                                // A frame the averaging window does not cover
                                // integrates to NaN, and so does one whose
                                // samples are all invalid. Leaving those out
                                // shortens the trace rather than breaking it.
                                let integral = frames
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
                                    .filter(|sample| sample.displacement.is_finite())
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
                                view.area_rms_transverse,
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
                                primary_field_rate_label(physics),
                                &point_samples,
                                |sample| sample.velocity,
                                TEAL,
                                &mut view,
                                history,
                            );
                        }
                        if view.transverse_field {
                            Self::probe_plot(
                                ui,
                                transverse_field_magnitude_label(physics),
                                &point_samples,
                                |sample| sample.transverse_magnitude,
                                Color32::from_rgb(188, 139, 255),
                                &mut view,
                                history,
                            );
                        }
                        if view.poynting {
                            Self::probe_plot(
                                ui,
                                energy_flow_magnitude_label(physics),
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
        self.runtime.last_error().is_some() || self.amr_error.is_some() || self.unseen_error
    }
    /// Appends what each transient channel says, the moment it changes. Every
    /// one of them is overwritten by whatever happens next - a status line by
    /// the next message, a preparation or adaptation error by the next
    /// success, a repair fallback by the next transaction - so a change is the
    /// only moment the value can be caught. Watching the values rather than
    /// the two dozen places that set them is what makes the log complete.
    fn record_events(&mut self, now: f64) {
        let status = (!self.message.is_empty()).then(|| self.message.clone());
        if status != self.logged_status {
            self.logged_status = status.clone();
            if let Some(text) = status {
                self.push_event(now, EventSource::Status, text);
            }
        }
        let preparation = self.runtime.last_error().map(|error| error.to_string());
        if preparation != self.logged_preparation {
            self.logged_preparation = preparation.clone();
            if let Some(text) = preparation {
                self.push_event(now, EventSource::Preparation, text);
            }
        }
        let adaptation = self.amr_error.clone();
        if adaptation != self.logged_adaptation {
            self.logged_adaptation = adaptation.clone();
            if let Some(text) = adaptation {
                self.push_event(now, EventSource::Adaptation, text);
            }
        }
        for text in std::mem::take(&mut self.pending_repairs) {
            self.push_event(now, EventSource::Repair, text);
        }
    }
    fn push_event(&mut self, now: f64, source: EventSource, text: String) {
        if let Some(last) = self.events.back_mut()
            && last.source == source
            && last.text == text
        {
            last.repeats += 1;
            last.time = now;
            return;
        }
        if self.events.len() == EVENT_LOG_ENTRIES {
            self.events.pop_front();
        }
        self.events.push_back(EventEntry {
            time: now,
            source,
            text,
            repeats: 1,
        });
        if source.error() {
            self.unseen_error = true;
        }
    }
    fn summary_line(&self) -> String {
        let active = self.runtime.active();
        format!(
            "{:.0} fps · {:.0} steps/s · {} dofs · {} elements · dt {}",
            1000.0 / self.frame_ms.max(0.01),
            self.steps_per_second,
            active.map_or(0, |v| v.operator.degrees_of_freedom()),
            active.map_or(0, |v| v.mesh.triangles.len()),
            if active.is_some() {
                format!("{:.2e}", self.solver_time_step())
            } else {
                "—".into()
            }
        )
    }
    /// One draggable window over the whole transaction: the frame it costs, the
    /// log of what the transient channels said, and then the topology and mesh
    /// it produced, the handoff waits and the running solver. A preparation or
    /// adaptation error lights the status marker and keeps it lit until this
    /// is opened, rather than opening it; an ordinary rebuild does neither.
    fn diagnostics_window(&mut self, ctx: &egui::Context) {
        self.record_events(ctx.input(|input| input.time));
        if self.frame_ms.is_finite() && self.frame_ms > 0.0 {
            if self.frame_history.len() == FRAME_HISTORY {
                self.frame_history.pop_front();
            }
            self.frame_history.push_back(self.frame_ms);
        }
        if !self.diagnostics_open {
            return;
        }
        // Open is seen: the log below holds whatever lit the marker.
        self.unseen_error = false;
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
                    // Above the sections whose height follows whatever the
                    // last transaction did, so reading the log does not mean
                    // chasing it down the window.
                    self.log_section(ui);
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

    /// The example gallery. It is a window rather than a menu so the thumbnails
    /// and descriptions have room, and it stays open across a pick so several
    /// scenes can be tried one after another.
    fn examples_window(&mut self, ctx: &egui::Context) {
        if !self.examples_open {
            return;
        }
        let catalog = funfern_app::topology_examples::catalog();
        // At most one preview is built per frame. The largest example takes
        // about twenty milliseconds to compile, so building all of them at once
        // would drop a frame outright; this way a row shows its name and
        // description immediately and its thumbnail a few frames later.
        if let Some(index) = self.example_previews.iter().position(Option::is_none) {
            self.example_previews[index] = Some(build_example_preview(&catalog[index]));
        }
        let previews = &self.example_previews;
        let opened = self.example_opened;
        let mut open = true;
        let mut selected = None;
        egui::Window::new("Examples")
            .id(egui::Id::new("examples"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.label("Choose a ready-to-run scene. Picking one leaves this open.");
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(540.0)
                    .show(ui, |ui| {
                        for (index, example) in catalog.iter().enumerate() {
                            ui.horizontal(|ui| {
                                let thumbnail = paint_example_thumbnail(
                                    ui,
                                    example,
                                    previews[index].as_ref(),
                                    egui::vec2(144.0, 144.0),
                                );
                                ui.vertical(|ui| {
                                    ui.heading(example.name);
                                    ui.set_max_width(320.0);
                                    ui.label(example.description);
                                    ui.add_space(8.0);
                                    ui.horizontal(|ui| {
                                        if ui.button("Open").clicked() || thumbnail.clicked() {
                                            selected = Some(index);
                                        }
                                        if opened == Some(index) {
                                            ui.weak("Opened");
                                        }
                                    });
                                });
                            });
                            if index + 1 < catalog.len() {
                                ui.add_space(8.0);
                                ui.separator();
                                ui.add_space(8.0);
                            }
                        }
                    });
            });
        self.examples_open = open;
        if let Some(index) = selected {
            self.open_example(index);
        }
    }

    fn open_example(&mut self, index: usize) {
        let catalog = funfern_app::topology_examples::catalog();
        match self.set_document(catalog[index].document.clone(), true, true) {
            Ok(()) => {
                self.example_opened = Some(index);
                self.notify(format!("Opened {}", catalog[index].name));
            }
            Err(error) => self.notify(error),
        }
    }

    /// A scene to start from: nothing drawn, one background material, every wall
    /// second-order outgoing, and the point source switched on, because an empty
    /// scene that makes no wave is a still picture rather than a starting point.
    fn new_scene(&mut self) {
        let mut document = TopologyDocument::default();
        document.model.source.enabled = true;
        match self.set_document(document, true, true) {
            Ok(()) => self.notify("New scene"),
            Err(error) => self.notify(error),
        }
    }

    /// Opens a catalog entry at random, for a launch with nothing to restore.
    fn open_random_example(&mut self) {
        let catalog = funfern_app::topology_examples::catalog();
        let index = random_example_index(random_fraction(), catalog.len());
        if let Err(error) = self.set_document(catalog[index].document.clone(), false, true) {
            self.notify(error);
        } else {
            self.example_opened = Some(index);
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
                    if report.dormant {
                        ui.small("Whole field dormant · relative error suppressed");
                    } else {
                        ui.small(format!(
                            "Whole field {:.2}% · target {:.0}% · {}",
                            100.0 * report.global_indicator,
                            self.amr_accuracy_percent,
                            if adaptation_refines(report, self.amr_target_accuracy()) {
                                "refining"
                            } else {
                                "settled"
                            },
                        ));
                    }
                    ui.small(format!(
                        "Refine candidates {} ({} error, {} limit) · coarsen candidates {} · \
                         {} work units",
                        report.refine_candidates,
                        report.error_refine_candidates,
                        report.limit_refine_candidates,
                        report.coarsen_candidates,
                        report.work_units,
                    ));
                    if report.smallest_wavelength_target.is_finite() {
                        ui.small(format!(
                            "Forced wavelength wants {:.4}",
                            report.smallest_wavelength_target
                        ));
                    }
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
                } else if self
                    .gpu_upload_preparation
                    .as_ref()
                    .is_some_and(|job| job.result.is_none())
                {
                    ui.label("Packing candidate GPU buffers in the background");
                } else if self.gpu_upload_preparation.is_some() {
                    ui.label(format!(
                        "GPU buffers ready · draining {} requested steps",
                        self.step_backlog
                    ));
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
                        let dt = self.solver_time_step();
                        let solver_bytes = self
                            .canonical_gpu_bytes
                            .unwrap_or_else(|| active.operator.estimated_gpu_bytes());
                        ui.small(format!(
                            "{} dofs · {:.2} MiB canonical solver storage",
                            active.operator.degrees_of_freedom(),
                            solver_bytes as f64 / (1024.0 * 1024.0),
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
                    ui.small(format!(
                        "Canonical discrete energy {energy:.6e} · bulk + gaps + outgoing memory"
                    ));
                }
            });
    }
    /// What the transient channels said, kept after they were overwritten.
    fn log_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Log")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .small_button("Clear")
                        .on_hover_text("Forget every entry below")
                        .clicked()
                    {
                        self.events.clear();
                    }
                    if ui
                        .small_button("Copy")
                        .on_hover_text("Copy the whole log, oldest first")
                        .clicked()
                    {
                        let text = self
                            .events
                            .iter()
                            .map(event_line)
                            .collect::<Vec<_>>()
                            .join("\n");
                        ui.ctx().copy_text(text);
                    }
                    ui.weak(format!("{} of {EVENT_LOG_ENTRIES}", self.events.len()));
                });
                if self.events.is_empty() {
                    ui.weak("Nothing recorded yet");
                    return;
                }
                egui::ScrollArea::vertical()
                    .max_height(160.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for entry in self.events.iter().rev() {
                            let line = event_line(entry);
                            match entry.source {
                                EventSource::Preparation | EventSource::Adaptation => {
                                    ui.colored_label(RED, line)
                                }
                                EventSource::Repair => ui.colored_label(GOLD, line),
                                EventSource::Status => ui.small(line),
                            };
                        }
                    });
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
                                ui.colored_label(GOLD, "⚠").on_hover_text(
                                    "Preparation or adaptation reported an error; the diagnostics log keeps it",
                                );
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
    fn show(
        &mut self,
        root: &mut egui::Ui,
        display: &WaveDisplay,
        vector_display: &VectorOverlayDisplay,
    ) -> Rect {
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
                viewport = self.viewport(ui, display, vector_display);
            });
        if !self.capturing() {
            self.probe_windows(root.ctx());
            self.diagnostics_window(root.ctx());
            self.formula_help_window(root.ctx());
            self.examples_window(root.ctx());
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

/// A world direction as a screen one. The viewport scales uniformly and flips
/// y, so a unit vector stays a unit vector.
fn screen_direction(direction: Point2) -> egui::Vec2 {
    egui::vec2(direction.x as f32, -direction.y as f32)
}

/// Where a boundary probe reads, and which way, at the midpoint its badge sits
/// on. The path arrives in increasing curve parameter, and `Left` names the
/// face on the left of that direction, so the normal that leaves a left trace
/// is the right one. Crossing that way is what a positive flux reading means,
/// and coming that way is what the probe reads, so one vector carries both:
/// drawn from the sampled side into the badge it says where the data is from
/// and which way positive points. `along` runs the way the arclength axis does,
/// which `reversed` turns without moving the side.
fn boundary_probe_orientation(
    path: &[Point2],
    side: CurveTraceSide,
    reversed: bool,
) -> Option<(Point2, Point2, Point2)> {
    let (point, [start, end]) = Playground::polyline_anchor(path, 0.5)?;
    let tangent = end - start;
    let length = tangent.norm();
    if !length.is_finite() || length <= f64::EPSILON {
        return None;
    }
    let tangent = tangent / length;
    let left = Point2::new(-tangent.y, tangent.x);
    let outward = match side {
        CurveTraceSide::Left => left * -1.0,
        CurveTraceSide::Right => left,
    };
    let along = if reversed { tangent * -1.0 } else { tangent };
    Some((point, outward, along))
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
    let mut kind = outer_kind(*condition);
    egui::ComboBox::from_id_salt("outer-condition")
        .selected_text(condition.label())
        .show_ui(ui, |ui| boundary_kind_choices(ui, &mut kind));
    if kind != outer_kind(before) {
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

fn outer_kind(condition: OuterBoundaryCondition) -> BoundaryKind {
    match condition {
        OuterBoundaryCondition::Reflecting => BoundaryKind::Reflecting,
        OuterBoundaryCondition::FirstOrderOutgoing => BoundaryKind::FirstOrder,
        OuterBoundaryCondition::SecondOrderOutgoing => BoundaryKind::SecondOrder,
        OuterBoundaryCondition::ElectricWall => BoundaryKind::ElectricWall,
        OuterBoundaryCondition::MagneticWall => BoundaryKind::MagneticWall,
        OuterBoundaryCondition::Neumann { .. } => BoundaryKind::Neumann,
        OuterBoundaryCondition::Dirichlet { .. } => BoundaryKind::Dirichlet,
    }
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
    for value in BoundaryKind::ALL {
        ui.selectable_value(kind, value, value.label());
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
/// Where a subdomain probe's marker sits: the area-weighted centroid of the
/// faces its region covers, taken from the compiled geometry. Averaging the
/// mesh's triangle centroids instead puts the marker wherever triangles are
/// dense, which under adaptation is wherever the wave currently is, so it
/// wandered on every remesh. For a region shaped like a ring the point lands in
/// the hole, as it does for one such face.
fn region_anchor(scene: &CompiledTopologyScene, region: RegionId) -> Option<Point2> {
    let mut moment = Point2::default();
    let mut area = 0.0;
    for assignment in &scene.assignments {
        if assignment.region != Some(region) {
            continue;
        }
        let Some(face) = scene.topology.face(assignment.face) else {
            continue;
        };
        let Some(centroid) = face.centroid() else {
            continue;
        };
        moment = moment + centroid * face.area;
        area += face.area;
    }
    (area > 0.0).then(|| moment / area)
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

const fn energy_flow_magnitude_label(physics: PhysicsModel) -> &'static str {
    match physics {
        PhysicsModel::Mechanical => "Energy-flow magnitude |F|",
        PhysicsModel::Electromagnetic { .. } => "Poynting magnitude |S|",
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
/// The 1/2/5-per-decade spacing that keeps grid lines about 70 pixels apart.
/// The step the grid draws at, and the finer one it subdivides to.
///
/// Snapping lands on the fine step, so every position Shift can reach has a
/// line under it. The grid used to step in 1/2/5 per decade from the zoom while
/// snapping went to a fixed 0.05, so at most zooms it drew one lattice and
/// landed on another. Each decade divides into round numbers: a 2 into four
/// parts and a 1 or a 5 into five, which at the default zoom puts the fine step
/// back at the 0.05 it used to be fixed at.
fn grid_steps(scale: f64) -> (f64, f64) {
    let raw = 70.0 / scale;
    let power = 10f64.powf(raw.log10().floor());
    let (digit, divisions) = [(1.0, 5.0), (2.0, 4.0), (5.0, 5.0), (10.0, 5.0)]
        .into_iter()
        .find(|(digit, _)| digit * power >= raw)
        .unwrap_or((10.0, 5.0));
    let major = digit * power;
    (major, major / divisions)
}

/// Every multiple of `step` inside `[minimum, maximum]`, lowest first. Both
/// axes walk upwards from the lower bound; the vertical one used to start at
/// the top of the view and test against the bottom, so it never drew a line.
fn grid_lines(minimum: f64, maximum: f64, step: f64) -> Vec<f64> {
    let mut lines = vec![];
    if !minimum.is_finite() || !maximum.is_finite() || !step.is_finite() || step <= 0.0 {
        return lines;
    }
    let mut value = (minimum / step).floor() * step;
    while value <= maximum && lines.len() < 4096 {
        lines.push(value);
        value += step;
    }
    lines
}

/// Whether the next preparation starts the field at zero instead of carrying
/// the running one into it.
///
/// `reset_requested` is spent earlier in the frame by the GPU reset, so a
/// document load cannot rely on it and raises `fresh_requested` instead; this
/// has to honour that even when a topology is already active and the reset flag
/// has been cleared.
const fn starts_from_zero(active: bool, reset_requested: bool, fresh_requested: bool) -> bool {
    !active || reset_requested || fresh_requested
}

/// A display reference level for one measured quantity.
///
/// Across the shipped examples the field's own amplitude spans a hundredfold,
/// which is wider than the intensity slider's whole range, so no fixed gain can
/// serve them: at the default, seven of the eight painted under a tenth of full
/// colour, and at the slider's maximum three of them still did. The level is
/// measured from the field each frame instead.
///
/// It rises the instant the field does, so a real transient is never clipped,
/// and falls back over about a second, so a placed pulse fades out of the scale
/// rather than darkening everything after it for the rest of the run — which is
/// what the monotone run peak this replaces used to do. It never falls below a
/// small fraction of the loudest level seen, and that is what keeps a field
/// which has decayed into numerical noise from being magnified back into view.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct AutoExposure {
    reference: f64,
    peak: f64,
}

impl AutoExposure {
    /// The most the reference may fall in a second, as a factor. A scale that
    /// moves by a factor rather than by a difference takes the same time to
    /// clear a spike whatever its size, which is the only behaviour that reads
    /// the same on a field of 6e-3 and one of 7e-1.
    ///
    /// This has to be *slower* than the field's own decay or the scale simply
    /// follows it down and a domain that has emptied still paints at full
    /// brightness. Measured on a recorded level series from a scene whose walls
    /// all radiate: after the sources stop the field drains 2000-fold in eight
    /// seconds, and at the 8.0 this started at that still painted 48 % — the
    /// wave looked like it never left. The rates trade against each other in one
    /// direction, the tail brightness a field settles at against how long a
    /// placed pulse holds the scale:
    ///
    /// | per second | drain tail | 20x spike clears |
    /// | --- | --- | --- |
    /// | 1.15 | 0.07-0.37 % | 21 s |
    /// | 1.4 | up to 3.8 % | 9 s |
    /// | 1.7 | up to 8.2 % | 6 s |
    /// | 8.0 | 100 % then 48 % | 2 s |
    ///
    /// Above about 1.25 the scale catches up with the slow late decay and the
    /// picture creeps back up — which is also what made the high-frequency modes
    /// the grid-scale filter is busy killing swim back into view. 1.15 never
    /// does; the cost is that a pulse holds the scale for some twenty seconds,
    /// which is honest, since the pulse really was that much brighter.
    const RELEASE_PER_SECOND: f64 = 1.15;
    /// How far under the loudest level seen the reference may go.
    const QUIET_FLOOR: f64 = PRESENTATION_QUIET_AMPLITUDE_RATIO;
    /// The longest step the release is allowed to take at once, so a stalled
    /// frame cannot drop the scale by an unbounded factor in one go.
    const MAX_STEP_SECONDS: f32 = 1.0;

    /// Starts the scale again for a field that has been replaced with zeros.
    ///
    /// How loud the session has been is kept. The first frames of a new field
    /// are numerical dust — measured at 3.5e-10 — and an instant attack onto a
    /// scale with nothing behind it latches straight onto that and paints it at
    /// full colour. The remembered peak holds the quiet floor above the dust
    /// until the field is really there.
    fn restart(&mut self) {
        self.reference = 0.0;
    }

    /// Starts a genuinely different displayed quantity. Unlike a fresh field
    /// in the same run, it must not inherit a peak measured in different units.
    fn clear(&mut self) {
        *self = Self::default();
    }

    fn reference(self) -> Option<f64> {
        (self.reference > 0.0).then_some(self.reference)
    }

    /// Takes this frame's measured level and the wall-clock seconds since the
    /// previous one, and answers with the level to divide by.
    fn update(&mut self, level: f64, elapsed: f32) -> Option<f64> {
        if level.is_finite() && level > 0.0 {
            self.peak = self.peak.max(level);
            let elapsed = f64::from(elapsed.clamp(0.0, Self::MAX_STEP_SECONDS));
            // The maximum rises to meet a louder field at once, so nothing is
            // ever clipped, and the release only ever slows the way back down.
            self.reference = level.max(self.reference * Self::RELEASE_PER_SECOND.powf(-elapsed));
            self.reference = self.reference.max(self.peak * Self::QUIET_FLOOR);
        }
        self.reference()
    }

    /// Smoothly turns off structure below the run-relative quiet floor. Merely
    /// flooring the denominator still paints late f32 residue at a few percent,
    /// which reads as a full-domain static pattern. Squaring the smoothstep
    /// makes that residue disappear without a visible threshold crossing.
    fn visibility(self, level: f64) -> f64 {
        if !level.is_finite() || level <= 0.0 || self.peak <= 0.0 {
            return 0.0;
        }
        let fraction = (level / (self.peak * Self::QUIET_FLOOR)).clamp(0.0, 1.0);
        let smooth = fraction * fraction * (3.0 - 2.0 * fraction);
        smooth * smooth
    }
}

/// Where the field's reference level sits in its own distribution: above the
/// quiet bulk of the domain, below the few nodes right against a source.
const FIELD_EXPOSURE_QUANTILE: f64 = 0.98;

/// The intensity slider is a trim on the automatic scale rather than the scale
/// itself. At its default of 2.0 the reference level lands on `tanh(1.0)`,
/// about three quarters of full colour, which leaves the brightest nodes
/// brighter still instead of clipping them flat.
const FIELD_EXPOSURE_GAIN: f32 = 0.5;

/// Presentation-only complementary-field DC rejection. This is the old
/// reconstruction corner (0.5 rad/s, about 0.08 Hz), now applied only to arrow
/// samples and measured in simulated time. Ordinary 2.5--4 Hz waves therefore
/// retain more than 99.9% of their amplitude.
const VECTOR_DC_REJECTION_RATE: f64 = 0.5;
/// The longest arrow below this global visibility is less than about a tenth
/// of a pixel even at the coarsest supported density. Avoiding the draw also
/// avoids assigning a visible direction to near-zero floating-point residue.
const VECTOR_OVERLAY_VISIBILITY_CUTOFF: f64 = 1.0 / 512.0;

/// Scales one arrow under a shared exposure. Saturation belongs before the
/// quiet-tail visibility: otherwise an arbitrarily large sparse outlier can
/// cancel an arbitrarily small global fade by hitting the length clamp.
fn vector_arrow_length(
    magnitude: f64,
    reference: f64,
    gain: f32,
    maximum_length: f32,
    visibility: f64,
) -> f32 {
    let exposed = (magnitude * f64::from(gain) / reference).clamp(0.0, 1.0);
    (f64::from(maximum_length) * exposed * visibility.clamp(0.0, 1.0)) as f32
}

/// The `quantile` of `values` by magnitude, sampled rather than sorted.
///
/// Sorting every node each frame would spend milliseconds placing a number the
/// field itself moves by more than the estimate's error. Nodes are numbered in
/// meshing order, which bears no relation to the field, so a strided sample is
/// a fair one.
fn exposure_level(values: &[f32], quantile: f64, scratch: &mut Vec<f64>) -> f64 {
    const SAMPLES: usize = 4096;
    scratch.clear();
    let stride = values.len().div_ceil(SAMPLES).max(1);
    scratch.extend(
        values
            .iter()
            .step_by(stride)
            .map(|value| f64::from(value.abs()))
            .filter(|value| value.is_finite()),
    );
    if scratch.is_empty() {
        return 0.0;
    }
    let index = ((scratch.len() - 1) as f64 * quantile).round() as usize;
    *scratch
        .select_nth_unstable_by(index, |a, b| a.total_cmp(b))
        .1
}

/// Wall-clock seconds a frame is budgeted when deciding how small the solver's
/// step has to be.
///
/// Fixed rather than the measured frame time: a step that moved with the frame
/// rate would jitter, and each jitter costs a republish. A hundred and twentieth
/// keeps every frame of a fast display fed.
const PACING_FRAME_SECONDS: f64 = 1.0 / 120.0;

/// How far the wanted step may drift from the one the solver is running before
/// it is worth republishing to change it.
const TIME_STEP_HYSTERESIS: f64 = 0.1;

/// The step the solver runs at: the mesh's stability limit, or smaller when the
/// speed ceiling is low enough that pacing by step count alone would leave whole
/// frames without one.
///
/// Above a speed of `recommended / PACING_FRAME_SECONDS` this is the
/// recommendation unchanged and the rate is paced purely by how many steps a
/// frame asks for. Below it that count floors to zero on most frames and the
/// picture judders — measured at 0.53 steps a frame with 52 % of frames
/// advancing at 0.05x, and a coarse mesh crosses the threshold at 0.8x — so the
/// step shrinks instead and every frame gets one. Shrinking is always safe: the
/// stability limit is an upper bound, and this never goes above it.
fn paced_time_step(recommended: f64, speed: f64) -> f64 {
    if !recommended.is_finite() || recommended <= 0.0 || !speed.is_finite() || speed <= 0.0 {
        return recommended;
    }
    recommended.min(PACING_FRAME_SECONDS * speed)
}

/// The two short boundaries at which it is unsafe to publish more work.
/// Preparing and uploading a replacement generation are deliberately absent:
/// the accepted generation can keep advancing through both. We drain just
/// before `begin_handoff`. Once the transfer is encoded, later source steps
/// remain visible while validation is in flight and are replayed by the target
/// from its exact transferred clock before its first visible readback.
fn canonical_steps_withheld(packed_candidate_waiting: bool, fresh_upload: bool) -> bool {
    packed_candidate_waiting || fresh_upload
}

/// Steps to ask the solver for this frame, spending `accumulator` at
/// `time_step` a step.
///
/// `speed` is the ceiling on simulated seconds per wall second: the wall-clock
/// time a frame took is scaled by it before being spent, so half asks for half
/// the steps. The frame's own delta is clamped first, so a stall cannot spend
/// more than a tenth of a second at once, and the leftover is capped at one
/// frame's worth — the solver cannot encode more than `MAX_STEPS_PER_FRAME` in
/// a frame, so unspent time is dropped rather than queued into a backlog that
/// never drains. That cap is also why a high speed on a heavy scene simply
/// falls short instead of running away.
fn steps_for_frame(accumulator: &mut f64, delta: f64, speed: f64, time_step: f64) -> u64 {
    if !time_step.is_finite() || time_step <= 0.0 || !speed.is_finite() || speed <= 0.0 {
        return 0;
    }
    *accumulator += delta.clamp(0.0, 0.1) * speed;
    let steps = (*accumulator / time_step)
        .floor()
        .clamp(0.0, MAX_STEPS_PER_FRAME as f64) as u64;
    *accumulator -= steps as f64 * time_step;
    *accumulator = accumulator.min(MAX_STEPS_PER_FRAME as f64 * time_step);
    steps
}

/// How close the reached rate has to come to the one asked for before the
/// shortfall is worth mentioning.
const SPEED_SHORTFALL_MARGIN: f64 = 0.8;

/// How fast the held rate gives up a better reading, as a factor per second.
///
/// The windowed measurement dips to about three quarters of the rate asked for
/// whenever a handoff withholds stepping inside its window — measured at every
/// speed, including ones the solver reaches comfortably — so comparing it
/// directly would flash the note at random. Holding the best reading rides over
/// a dip of a second while still letting a real slowdown through in under two.
const SPEED_HOLD_PER_SECOND: f64 = 1.15;

/// The best rate seen lately: instant to a better reading, slow to give one up.
fn hold_rate(held: f64, measured: f64, elapsed: f64) -> f64 {
    if !measured.is_finite() || measured < 0.0 {
        return held;
    }
    measured.max(held * SPEED_HOLD_PER_SECOND.powf(-elapsed.clamp(0.0, 1.0)))
}

/// The rate actually being reached, when it falls meaningfully short of `target`
/// and the solver is genuinely trying.
///
/// Both are simulated seconds per wall second. Nothing is said while the solver
/// is paused or before any steps have been measured — neither is the solver
/// failing to keep up.
fn speed_shortfall(measured: f64, target: f64, stepping: bool) -> Option<f64> {
    if !stepping || !measured.is_finite() || measured <= 0.0 {
        return None;
    }
    (measured < target * SPEED_SHORTFALL_MARGIN).then_some(measured)
}

/// The factor a node's value is multiplied by before it becomes colour.
///
/// Automatic, the reference level lands on `tanh(gain * FIELD_EXPOSURE_GAIN)`,
/// which at the default gain is about three quarters of full colour. Manual, the
/// slider is the whole scale — `tanh(value * gain)`, exactly what the field was
/// painted with before it measured its own. With nothing measured yet there is
/// no scale, and every node is zero anyway.
fn field_scale(gain: f32, reference: Option<f64>, automatic: bool) -> f64 {
    if !automatic {
        return f64::from(gain);
    }
    reference.map_or(0.0, |reference| {
        f64::from(gain) * f64::from(FIELD_EXPOSURE_GAIN) / reference
    })
}

/// `scaled` is the node's value already divided by whatever scale is in force —
/// the exposure's reference when it is on, the intensity slider alone when it is
/// not — so this only has to decide a colour.
fn field_color(scaled: f32, under: Color32) -> Color32 {
    let value = scaled.tanh();
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

fn field_color_over_overlay(scaled: f32) -> Color32 {
    let value = if scaled.is_finite() {
        scaled.tanh()
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

/// A fraction in `[0, 1)` without a random-number dependency: the wall clock's
/// sub-second bits natively, and the platform's own generator in the browser,
/// where `SystemTime::now` is not available.
#[cfg(not(target_arch = "wasm32"))]
fn random_fraction() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |since| f64::from(since.subsec_nanos()) / 1e9)
}

#[cfg(target_arch = "wasm32")]
fn random_fraction() -> f64 {
    js_sys::Math::random()
}

/// The catalog entry a fraction picks. A fraction of exactly one is the reason
/// for the clamp: it is outside the half-open range the callers promise, and a
/// browser's generator is only documented to stay below it.
fn random_example_index(fraction: f64, total: usize) -> usize {
    ((fraction.clamp(0.0, 1.0) * total as f64) as usize).min(total.saturating_sub(1))
}

/// One example's thumbnail, built once from its compiled scene and then only
/// painted. Faces are rasterized into scanline spans rather than triangulated:
/// a face is an arbitrary polygon with holes, and a span is one quad when the
/// face carries a flat colour and a row of small quads when the example asks
/// for a material property, which is the only way a radial profile shows up at
/// all.
#[derive(Default)]
struct ExamplePreview {
    quads: Vec<PreviewQuad>,
    /// Face outlines and the zero-area traces of baffles, stroked over the fill.
    strokes: Vec<Vec<Point2>>,
}

struct PreviewQuad {
    low: Point2,
    high: Point2,
    color: Color32,
}

/// Rows down the domain. A row is under three pixels of a thumbnail, and the
/// stroked outlines cover the stair-stepping left at a face's edge. Going finer
/// doubles the quads a shaded example costs and changes nothing that shows.
const PREVIEW_ROWS: usize = 48;

fn build_example_preview(
    example: &funfern_app::topology_examples::TopologyExample,
) -> ExamplePreview {
    let scene = &example.document.model.accepted;
    let Ok(compiled) = scene.compile(0) else {
        return ExamplePreview::default();
    };
    let domain = scene.geometry.domain;
    let property = match example.document.presentation.material_overlay {
        MaterialOverlay::Property(property) => Some(property),
        _ => None,
    };
    let mut faces = compiled.topology.faces.iter().collect::<Vec<_>>();
    faces.sort_by(|a, b| b.area.total_cmp(&a.area));

    let mut cells = Vec::new();
    let (mut minimum, mut maximum) = (f64::INFINITY, f64::NEG_INFINITY);
    for face in &faces {
        let region = compiled
            .assignments
            .iter()
            .find(|assignment| assignment.face == face.id)
            .and_then(|assignment| assignment.region);
        let flat = region
            .and_then(|region| scene.region(region))
            .and_then(|region| scene.material(region.material))
            .map(|material| {
                Color32::from_rgb(material.color[0], material.color[1], material.color[2])
            })
            // A face outside the simulation is a hole, and reads as one.
            .unwrap_or(Color32::from_rgb(16, 23, 31));
        let shaded = property.zip(region).filter(|_| face.area > 0.0);
        let height = domain.height() / PREVIEW_ROWS as f64;
        for row in 0..PREVIEW_ROWS {
            let low_y = domain.min_y + height * row as f64;
            for (start, end) in face_spans(&face.cycles, low_y + height * 0.5) {
                match shaded {
                    None => cells.push((
                        Point2::new(start, low_y),
                        Point2::new(end, low_y + height),
                        None,
                        flat,
                    )),
                    Some((property, region)) => {
                        let steps = (((end - start) / height).ceil() as usize).max(1);
                        let step = (end - start) / steps as f64;
                        for index in 0..steps {
                            let low_x = start + step * index as f64;
                            let center = Point2::new(low_x + step * 0.5, low_y + height * 0.5);
                            let value = crate::material_overlay::sample(scene, region, center)
                                .value(property)
                                .ok();
                            if let Some(value) = value {
                                minimum = minimum.min(value);
                                maximum = maximum.max(value);
                            }
                            cells.push((
                                Point2::new(low_x, low_y),
                                Point2::new(low_x + step, low_y + height),
                                value,
                                flat,
                            ));
                        }
                    }
                }
            }
        }
    }

    let span = maximum - minimum;
    let quads = cells
        .into_iter()
        .map(|(low, high, value, flat)| PreviewQuad {
            low,
            high,
            color: match (value, property) {
                (Some(value), Some(property)) => {
                    let fraction = if span > 0.0 {
                        ((value - minimum) / span).clamp(0.0, 1.0) as f32
                    } else {
                        0.5
                    };
                    overlay_property_color(property, fraction, 255)
                }
                _ => flat,
            },
        })
        .collect();
    let strokes = faces
        .iter()
        .flat_map(|face| &face.cycles)
        .filter(|cycle| cycle.len() >= 2)
        .cloned()
        .collect();
    ExamplePreview { quads, strokes }
}

/// Where one horizontal line enters and leaves a face. Every cycle is closed
/// and thrown in together, so the even-odd pairing that falls out excludes the
/// face's own holes without any extra bookkeeping.
fn face_spans(cycles: &[Vec<Point2>], y: f64) -> Vec<(f64, f64)> {
    let mut crossings = Vec::new();
    for cycle in cycles {
        for index in 0..cycle.len() {
            let a = cycle[index];
            let b = cycle[(index + 1) % cycle.len()];
            if (a.y > y) != (b.y > y) {
                crossings.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
            }
        }
    }
    crossings.sort_by(f64::total_cmp);
    crossings
        .chunks_exact(2)
        .filter(|pair| pair[1] > pair[0])
        .map(|pair| (pair[0], pair[1]))
        .collect()
}

fn paint_example_thumbnail(
    ui: &mut egui::Ui,
    example: &funfern_app::topology_examples::TopologyExample,
    preview: Option<&ExamplePreview>,
    size: egui::Vec2,
) -> egui::Response {
    let scene = &example.document.model.accepted;
    let domain = scene.geometry.domain;
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 5.0, Color32::from_rgb(24, 32, 39));
    let inner = rect.shrink(7.0);
    let project = |point: Point2| {
        egui::pos2(
            egui::lerp(
                inner.left()..=inner.right(),
                ((point.x - domain.min_x) / domain.width()) as f32,
            ),
            egui::lerp(
                inner.bottom()..=inner.top(),
                ((point.y - domain.min_y) / domain.height()) as f32,
            ),
        )
    };
    if let Some(preview) = preview {
        let mut mesh = egui::Mesh::default();
        mesh.reserve_vertices(preview.quads.len() * 4);
        mesh.reserve_triangles(preview.quads.len() * 2);
        for quad in &preview.quads {
            let base = mesh.vertices.len() as u32;
            let low = project(quad.low);
            let high = project(quad.high);
            for corner in [
                egui::pos2(low.x, low.y),
                egui::pos2(high.x, low.y),
                egui::pos2(high.x, high.y),
                egui::pos2(low.x, high.y),
            ] {
                mesh.colored_vertex(corner, quad.color);
            }
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
        }
        painter.add(egui::Shape::mesh(mesh));
        for stroke in &preview.strokes {
            let mut points = stroke
                .iter()
                .map(|point| project(*point))
                .collect::<Vec<_>>();
            points.push(points[0]);
            painter.add(egui::Shape::line(points, Stroke::new(1.0, TEAL)));
        }
    }
    let corners = [
        (inner.left_bottom(), inner.right_bottom()),
        (inner.right_bottom(), inner.right_top()),
        (inner.right_top(), inner.left_top()),
        (inner.left_top(), inner.left_bottom()),
    ];
    for side in OuterSide::ALL {
        let (start, end) = corners[side.index()];
        let condition = scene.outer_boundaries.sides[side.index()];
        painter.line_segment(
            [start, end],
            Stroke::new(2.5, outer_condition_color(condition)),
        );
        if condition
            .signal()
            .is_some_and(|signal| signal.characteristic_amplitude() != 0.0)
        {
            painter.circle_filled(start.lerp(end, 0.5), 3.5, GOLD);
        }
    }
    if example.document.model.source.enabled {
        let center = project(example.document.model.source.position);
        painter.circle_stroke(center, 4.5, Stroke::new(1.6, GOLD));
        painter.line_segment(
            [center - egui::vec2(6.0, 0.0), center + egui::vec2(6.0, 0.0)],
            Stroke::new(1.0, GOLD),
        );
        painter.line_segment(
            [center - egui::vec2(0.0, 6.0), center + egui::vec2(0.0, 6.0)],
            Stroke::new(1.0, GOLD),
        );
    }
    painter.rect_stroke(
        rect,
        5.0,
        Stroke::new(1.0, Color32::from_rgb(88, 104, 116)),
        egui::StrokeKind::Inside,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
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

/// Whether an estimate is asking for a finer mesh.
///
/// A limit is a floor rather than a judgement: too few elements across a forced
/// wavelength, or an element larger than the largest allowed, is wrong however
/// small the error reads, so those refine on their own account. What the error
/// estimate asks for answers to the accuracy target instead, and it answers to
/// it for the field as a whole rather than element by element. Without that
/// second clause anything the estimate cannot satisfy - a boundary the field
/// disagrees with, the grid-scale leftovers of a wave that has passed - shrinks
/// by the estimator's step every cycle until it reaches the smallest element
/// allowed, and no accuracy setting reaches far enough to stop it: the step is
/// clamped, so it is the same step whatever the target says.
///
/// The cost is that error concentrated in a small part of a domain the estimate
/// is otherwise happy with stops being chased once the whole field is inside
/// the target. That is the trade a single number for the whole field makes.
fn adaptation_refines(report: &SolutionIndicatorReport, target_accuracy: f64) -> bool {
    report.limit_refine_candidates >= 4
        || (report.error_refine_candidates >= 4 && report.global_indicator > target_accuracy)
}

/// A resident filter flips the accepted state lane without advancing physical
/// time. The other lane at this boundary is the pre-filter state, not the
/// previous solver endpoint; derivative and residual consumers must not treat
/// the pair as one ordinary `dt` step.
fn resident_filter_boundary(enabled: bool, completed_steps: u64) -> bool {
    enabled && completed_steps > 0 && completed_steps.is_multiple_of(GRID_SCALE_FILTER_CADENCE)
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

#[allow(clippy::too_many_arguments)]
fn vector_overlay_layout(
    scene: &TopologyScene,
    mesh: &TriMesh,
    operator: &QuadraticWaveOperator,
    spacing: f32,
    view: (Point2, f64, Rect),
) -> Vec<VectorOverlayLayoutPoint> {
    let (view_center, view_scale, viewport) = view;
    if mesh.triangles.len() != operator.element_nodes().len() || spacing <= 0.0 {
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
        .filter_map(|(_key, (element, _screen, centroid, _))| {
            let triangle = &mesh.triangles[element];
            let region = scene.region(triangle.region)?;
            let material = scene.material(region.material)?;
            let raw = material.evaluate(region.frame, centroid).ok()?;
            let coefficients = scene
                .physics
                .directional_wave_coefficients(raw, region.frame);
            let stencil = QuadraticPointStencil {
                element: element as u32,
                barycentric: [1.0 / 3.0; 3],
                nodes: operator.element_nodes()[element],
                value_weights: enriched_quadratic_basis([1.0 / 3.0; 3]),
                gradient_weights: [Point2::default(); 7],
                region: triangle.region,
                mass_density: coefficients.mass_density,
                stiffness: coefficients.stiffness,
            };
            Some(VectorOverlayLayoutPoint {
                element: element as u32,
                point: centroid,
                stencil,
            })
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
        || display.snapshot_current.len() != count
        || display.auxiliary.len() != count
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
                            * (display.snapshot_current[node] as f64
                                + display.snapshot_current[node] as f64)
                }
            })
            .collect(),
    )
}

fn refresh_canonical_wave_display(
    active: Option<&Arc<PreparedTopology>>,
    canonical: &CanonicalGpuDisplay,
    request: &CanonicalGpuRequest,
    display: &mut WaveDisplay,
) {
    let Some(active) = active else { return };
    let operator = &active.canonical_operator;
    if canonical.generation != request.generation()
        || canonical.primary_flux.len() != operator.degrees_of_freedom()
        || canonical.previous_primary_flux.len() != operator.degrees_of_freedom()
        || canonical.constitutive_force.len() != operator.degrees_of_freedom()
    {
        return;
    }
    if display.generation == canonical.generation && display.readbacks == canonical.readbacks {
        return;
    }
    if display.generation != canonical.generation {
        display.complementary_flux.clear();
        display.snapshot_current.clear();
        display.snapshot_previous.clear();
        display.snapshot_velocity.clear();
        display.snapshot_completed_steps = 0;
    }
    let dt = canonical
        .clock
        .map_or(operator.recommended_time_step(), |clock| {
            f64::from(clock.time_step)
        });
    display.generation = canonical.generation;
    display.completed_steps = request.stats().completed_steps();
    display.current.clear();
    display.current.extend(
        canonical
            .primary_flux
            .iter()
            .zip(operator.primary_mass())
            .map(|(flux, mass)| (f64::from(*flux) / mass) as f32),
    );
    display.previous.clear();
    display.previous.extend(
        canonical
            .previous_primary_flux
            .iter()
            .zip(operator.primary_mass())
            .map(|(flux, mass)| (f64::from(*flux) / mass) as f32),
    );
    display.indicator_velocity.clear();
    display.indicator_velocity.extend(
        display
            .current
            .iter()
            .zip(&display.previous)
            .map(|(current, previous)| (*current - *previous) / dt as f32),
    );
    display.indicator_displacement.clear();
    display
        .indicator_displacement
        .extend(display.current.iter().copied());
    // Acceleration needs a sparse stiffness application and is consumed only
    // by the AMR snapshot. Build it at the 0.75 s AMR cadence instead of on
    // every visual readback.
    display.indicator_acceleration.clear();
    display.auxiliary.clear();
    display.auxiliary.resize(operator.degrees_of_freedom(), 0.0);
    display.indicator_potential.clear();
    if canonical.full_readback_at == canonical.readbacks {
        display.snapshot_current.clear();
        display
            .snapshot_current
            .extend(display.current.iter().copied());
        display.snapshot_previous.clear();
        display
            .snapshot_previous
            .extend(display.previous.iter().copied());
        display.snapshot_velocity.clear();
        display
            .snapshot_velocity
            .extend(display.indicator_velocity.iter().copied());
        display.snapshot_completed_steps = canonical.full_snapshot_completed_steps();
        display.complementary_flux.clear();
        display
            .complementary_flux
            .extend(canonical.complementary_flux.iter().copied());
    }
    display.readbacks = canonical.readbacks;
}

#[allow(clippy::too_many_arguments)]
pub fn frame(
    mut contexts: EguiContexts,
    mut state: ResMut<Playground>,
    time: Res<Time>,
    mut recorders: ResMut<WaveGpuRequest>,
    mut request: ResMut<CanonicalGpuRequest>,
    canonical_display: Res<CanonicalGpuDisplay>,
    mut display: ResMut<WaveDisplay>,
    probe_display: Res<ProbeDisplay>,
    curve_display: Res<CurveProbeDisplay>,
    area_display: Res<AreaProbeDisplay>,
    far_display: Res<FarFieldDisplay>,
    vector_display: Res<VectorOverlayDisplay>,
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
    state.frame_delta = time.delta_secs();
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
        } else {
            match crate::recovery::load()
                .ok()
                .flatten()
                .and_then(|bytes| persistence::parse(&bytes).ok())
            {
                Some(candidate) => {
                    state.load = Some(candidate);
                    state.file_busy = true;
                }
                // A first run, or an autosave that went away or stopped
                // parsing. Open a random example rather than the same one
                // every time, so the app opens on something worth looking at.
                None => state.open_random_example(),
            }
        }
    }
    state.editor.validate_frame(12000);
    state.refresh_runtime(
        &mut request,
        &canonical_display,
        &mut recorders,
        &mut assets,
        &mut commands,
        time.delta_secs_f64(),
    );
    refresh_canonical_wave_display(
        state.runtime.active(),
        &canonical_display,
        &request,
        &mut display,
    );
    state.refresh_amr(&request, &canonical_display, &display);
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
    let logical_viewport = state.show(&mut root, &display, &vector_display);
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
            && state.recording_readback_in_flight.load(Ordering::Acquire)
                < recording::MAX_READBACKS_IN_FLIGHT
            && let Some(target) = state.video_recorder.native_frame_target()
        {
            state.recording_last_requested_slot = Some(slot);
            // The schedule is the only producer, so the load above and this
            // increment cannot race each other; the capture observers only
            // ever decrement.
            state
                .recording_readback_in_flight
                .fetch_add(1, Ordering::AcqRel);
            let in_flight = state.recording_readback_in_flight.clone();
            commands.spawn(Screenshot::primary_window()).observe(
                move |captured: On<ScreenshotCaptured>| {
                    target.submit(
                        captured.image.clone(),
                        logical_canvas,
                        logical_viewport,
                        slot,
                    );
                    in_flight.fetch_sub(1, Ordering::AcqRel);
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
    fn handoff_withholds_steps_only_when_the_source_cannot_advance() {
        assert!(!canonical_steps_withheld(false, false));
        // A packed candidate drains the requests already published before the
        // UI calls begin_handoff.
        assert!(canonical_steps_withheld(true, false));
        // A fresh install has no accepted source generation to advance.
        assert!(canonical_steps_withheld(false, true));
        // Ordinary target upload and validation are not solver pauses. Later
        // requests become a target catch-up backlog after admission.
        assert!(!canonical_steps_withheld(false, false));
    }

    /// The point of the whole mechanism: the catalog's quietest example and its
    /// loudest sit a hundredfold apart, and both must paint the same picture.
    #[test]
    fn an_exposure_paints_the_same_picture_at_any_field_scale() {
        let scale = 118.0;
        let mut levels = vec![0.0, 1.0e-3, 6.0e-3, 4.0e-3, 6.2e-3, 5.9e-3, 2.0e-2];
        // Then all the way down past the exposure's own floor, so a scale that
        // is relative to the field is told apart from one pinned to a constant.
        let mut decaying = 2.0e-2;
        for _ in 0..40 {
            decaying *= 0.5;
            levels.push(decaying);
        }
        let mut quiet = AutoExposure::default();
        let mut loud = AutoExposure::default();
        let mut floored = false;
        for level in levels {
            let (Some(a), Some(b)) = (quiet.update(level, 0.1), loud.update(level * scale, 0.1))
            else {
                assert_eq!(level, 0.0, "a measured level produced no reference");
                continue;
            };
            floored |= a > level * 2.0;
            let probe = level * 0.7;
            assert!(
                (probe / a - probe * scale / b).abs() < 1.0e-9 * (probe / a).max(1.0e-9),
                "{level}: {a} against {b}"
            );
            assert!(
                (quiet.visibility(level) - loud.visibility(level * scale)).abs() < 1.0e-12,
                "the quiet-tail fade changed with absolute field scale"
            );
        }
        assert!(floored, "the run never reached the quiet floor");
    }

    /// The failure the monotone run peak had: one placed pulse set the scale for
    /// the rest of the run.
    #[test]
    fn an_exposure_recovers_after_a_transient_spike() {
        let mut exposure = AutoExposure::default();
        for _ in 0..120 {
            exposure.update(1.0, 1.0 / 60.0);
        }
        assert_eq!(exposure.update(20.0, 1.0 / 60.0), Some(20.0), "clipped");
        // The release gives up a factor of 1.15 a second, so a twentyfold spike
        // takes some twenty-one seconds to walk off. Bounded is the property
        // that matters — the run peak it replaced never gave it up at all.
        for _ in 0..1_500 {
            exposure.update(1.0, 1.0 / 60.0);
        }
        assert_eq!(exposure.reference(), Some(1.0));
    }

    /// The failure this release rate was chosen for. A scale that falls faster
    /// than the field does simply follows it down, so a domain that has emptied
    /// still paints at full brightness and the wave looks like it never left.
    /// The shape here is the recorded one: full amplitude, then three decades
    /// over eight seconds once the sources stop.
    #[test]
    fn a_field_that_drains_away_stops_being_painted() {
        let mut exposure = AutoExposure::default();
        for _ in 0..600 {
            exposure.update(3.0e-2, 1.0 / 60.0);
        }
        let mut level = 3.0e-2;
        for _ in 0..480 {
            level *= 0.985_7;
            exposure.update(level, 1.0 / 60.0);
        }
        let reference = exposure.reference().unwrap();
        let painted = level / reference;
        assert!(
            level < 3.0e-5,
            "the fixture did not actually drain: {level:e}"
        );
        assert!(
            painted < 0.05,
            "a drained domain still paints at {:.1}%",
            painted * 100.0
        );
    }

    /// And the failure the other way: once a field has decayed into rounding
    /// noise, renormalizing it would fill the view with structure that is not
    /// there.
    #[test]
    fn an_exposure_refuses_to_magnify_decayed_noise() {
        let mut exposure = AutoExposure::default();
        exposure.update(1.0, 1.0 / 60.0);
        // Two decades down to the floor at 1.15 a second is about thirty-three.
        for _ in 0..3_600 {
            exposure.update(1.0e-9, 1.0 / 60.0);
        }
        let floor = exposure.reference().unwrap();
        assert!(
            (floor - AutoExposure::QUIET_FLOOR).abs() < 1.0e-12,
            "{floor}"
        );
        assert!(1.0e-9 / floor < 1.0e-5, "noise would still be drawn");
        assert!(
            exposure.visibility(1.0e-9) < 1.0e-10,
            "late residue was not faded out"
        );
    }

    #[test]
    fn exposure_fades_smoothly_below_its_run_relative_floor() {
        let mut exposure = AutoExposure::default();
        exposure.update(1.0, 0.0);
        assert_eq!(exposure.visibility(AutoExposure::QUIET_FLOOR), 1.0);
        let tenth = exposure.visibility(AutoExposure::QUIET_FLOOR * 0.1);
        assert!((0.0..1.0e-3).contains(&tenth), "weak fade {tenth}");
        assert_eq!(exposure.visibility(0.0), 0.0);
        assert_eq!(exposure.visibility(f64::NAN), 0.0);
    }

    #[test]
    fn measured_damped_tail_is_not_renormalized_into_a_field() {
        let mut exposure = AutoExposure::default();
        exposure.update(1.0, 0.0);
        // The saved source-free TM drain fixture settles near 0.2--0.3% of
        // its propagated peak. Let the release reach its run-relative floor,
        // then verify that tail still paints below two percent of full scale.
        let tail = 3.0e-3;
        for _ in 0..6_000 {
            exposure.update(tail, 1.0 / 60.0);
        }
        let painted = tail / exposure.reference().unwrap() * exposure.visibility(tail);
        assert!(painted < 0.02, "tail still paints at {painted:.3}");
        assert_eq!(
            DORMANT_ENERGY_RATIO,
            AutoExposure::QUIET_FLOOR * AutoExposure::QUIET_FLOOR
        );
    }

    #[test]
    fn arrow_ac_coupling_rejects_static_state_in_simulation_time() {
        let mut state = Playground {
            uploaded_time_step: 0.01,
            ..Playground::default()
        };
        let sample = |value| vec![(0, Pos2::ZERO, Point2::new(value, 0.0))];

        let mut first = sample(1.0);
        state.ac_couple_vector_samples(&mut first, 0, false);
        assert_eq!(first[0].2.x, 0.0);

        let mut after_one_second = sample(1.0);
        state.ac_couple_vector_samples(&mut after_one_second, 100, false);
        assert_eq!(after_one_second[0].2.x, 0.0);

        let mut after_two_seconds = sample(1.0);
        state.ac_couple_vector_samples(&mut after_two_seconds, 200, false);
        assert_eq!(after_two_seconds[0].2.x, 0.0);
    }

    #[test]
    fn resident_filter_boundaries_are_not_ordinary_time_endpoints() {
        assert!(!resident_filter_boundary(true, 0));
        assert!(!resident_filter_boundary(true, 15));
        assert!(resident_filter_boundary(true, 16));
        assert!(resident_filter_boundary(true, 32));
        assert!(!resident_filter_boundary(false, 16));
    }

    #[test]
    fn arrow_ac_coupling_does_not_turn_filter_maintenance_into_a_wave() {
        let mut state = Playground {
            uploaded_time_step: 0.01,
            ..Playground::default()
        };
        let mut first = vec![(0, Pos2::ZERO, Point2::default())];
        state.ac_couple_vector_samples(&mut first, 14, false);

        let mut ordinary = vec![(0, Pos2::ZERO, Point2::new(0.2, 0.0))];
        state.ac_couple_vector_samples(&mut ordinary, 15, false);
        assert!((0.19..0.21).contains(&ordinary[0].2.x));

        // Deliberately exaggerated maintenance correction. Feeding the raw
        // input jump to the high-pass would produce an arrow near 9.0.
        let mut filtered = vec![(0, Pos2::ZERO, Point2::new(9.0, 0.0))];
        state.ac_couple_vector_samples(&mut filtered, 16, true);
        assert!((0.19..0.21).contains(&filtered[0].2.x));

        let mut after = vec![(0, Pos2::ZERO, Point2::new(9.1, 0.0))];
        state.ac_couple_vector_samples(&mut after, 17, false);
        assert!((0.28..0.31).contains(&after[0].2.x));
    }

    #[test]
    fn arrow_ac_coupling_tracks_physical_samples_and_cold_starts_new_ones() {
        let mut state = Playground {
            uploaded_time_step: 0.01,
            ..Playground::default()
        };
        let mut first = vec![(7, Pos2::ZERO, Point2::new(1.0, 0.0))];
        state.ac_couple_vector_samples(&mut first, 0, false);
        assert_eq!(first[0].2, Point2::default());

        // The same mesh element carries temporal history even if its screen
        // cell changes. A genuinely new element does not inherit that history
        // or flash its unknown baseline into the AC view.
        let mut moved = vec![
            (7, Pos2::new(80.0, 40.0), Point2::new(1.5, 0.0)),
            (11, Pos2::ZERO, Point2::new(9.0, 0.0)),
        ];
        state.ac_couple_vector_samples(&mut moved, 1, false);
        assert!(moved[0].2.x > 0.49, "lost physical-sample history");
        assert_eq!(moved[1].2, Point2::default(), "new sample flashed DC");

        // A lazily retained element uses its own last accepted step when it
        // returns to view; time spent off-screen still decays its baseline.
        let mut elsewhere = vec![(11, Pos2::ZERO, Point2::new(9.0, 0.0))];
        state.ac_couple_vector_samples(&mut elsewhere, 100, false);
        let mut returned = vec![(7, Pos2::ZERO, Point2::new(1.5, 0.0))];
        state.ac_couple_vector_samples(&mut returned, 101, false);
        assert!(
            (0.29..0.31).contains(&returned[0].2.x),
            "off-screen time was lost: {}",
            returned[0].2.x
        );
    }

    #[test]
    fn sparse_arrow_outliers_cannot_defeat_the_global_quiet_fade() {
        let maximum = 55.2;
        let visibility = VECTOR_OVERLAY_VISIBILITY_CUTOFF;
        let ordinary = vector_arrow_length(1.0, 1.0, 1.0, maximum, visibility);
        let outlier = vector_arrow_length(1.0e12, 1.0, 5.0, maximum, visibility);
        assert!((ordinary - outlier).abs() < f32::EPSILON);
        assert!(outlier < 0.11, "quiet outlier still spans {outlier} px");
    }

    #[test]
    fn arrow_ac_coupling_preserves_an_ordinary_source_frequency() {
        let mut state = Playground {
            uploaded_time_step: 1.0 / 600.0,
            ..Playground::default()
        };
        let frequency = 3.0;
        let mut input_square = 0.0;
        let mut output_square = 0.0;
        // The solver advances ten small steps between display-rate samples.
        for frame in 0..600_u64 {
            let step = frame * 10;
            let time = step as f64 * state.uploaded_time_step;
            let value = (std::f64::consts::TAU * frequency * time).sin();
            let mut samples = vec![(0, Pos2::ZERO, Point2::new(value, 0.0))];
            state.ac_couple_vector_samples(&mut samples, step, false);
            if frame >= 300 {
                input_square += value * value;
                output_square += samples[0].2.x * samples[0].2.x;
            }
        }
        let retained = (output_square / input_square).sqrt();
        assert!(retained > 0.999, "3 Hz amplitude retention {retained}");
    }

    /// Release is a rate in seconds, so the same second of wall clock has to
    /// land in the same place whether it took two frames or two hundred.
    #[test]
    fn an_exposure_releases_by_wall_clock_not_by_frame_count() {
        let mut coarse = AutoExposure::default();
        let mut fine = AutoExposure::default();
        coarse.update(10.0, 0.016);
        fine.update(10.0, 0.016);
        coarse.update(1.0, 0.5);
        coarse.update(1.0, 0.5);
        for _ in 0..100 {
            fine.update(1.0, 0.01);
        }
        // A relative tolerance: the two differ only in how the same decay was
        // recomposed in floating point.
        let (a, b) = (coarse.reference().unwrap(), fine.reference().unwrap());
        assert!((a - b).abs() < a * 1.0e-6, "{a} against {b}");
    }

    #[test]
    fn a_sampled_quantile_matches_the_sorted_one() {
        let mut scratch = Vec::new();
        let values = (0..50_000)
            .map(|index| index as f32 / 50_000.0)
            .collect::<Vec<_>>();
        let level = exposure_level(&values, 0.98, &mut scratch);
        assert!((level - 0.98).abs() < 0.01, "{level}");
        assert_eq!(exposure_level(&[0.0; 32], 0.98, &mut scratch), 0.0);
        assert_eq!(exposure_level(&[], 0.98, &mut scratch), 0.0);
        let broken = [f32::NAN, f32::INFINITY, -3.0, 1.0];
        assert_eq!(exposure_level(&broken, 0.5, &mut scratch), 3.0);
    }

    /// The default gain has to land the reference level somewhere legible, and
    /// leave the nodes above it room to read brighter still.
    #[test]
    fn the_default_gain_paints_the_reference_level_in_the_readable_band() {
        let default = funfern_app::document::PresentationSettings::default().field_gain;
        let scale = default * FIELD_EXPOSURE_GAIN;
        let base = field_color(0.0, Color32::TRANSPARENT);
        let at_reference = field_color(scale, Color32::TRANSPARENT);
        let above = field_color(2.0 * scale, Color32::TRANSPARENT);
        let reach = |color: Color32| f32::from(color.r() - base.r()) / f32::from(244 - base.r());
        assert!(
            (0.7..0.85).contains(&reach(at_reference)),
            "{}",
            reach(at_reference)
        );
        assert!(reach(above) > reach(at_reference) + 0.1, "no room above");
        assert_eq!(
            field_color_over_overlay(scale).a(),
            (0.761_594_f32 * 220.0).round() as u8
        );
    }

    /// Below a ceiling of `recommended / PACING_FRAME_SECONDS`, pacing by step
    /// count alone leaves whole frames without one. The step shrinks there
    /// instead — always downward, since the mesh's figure is a stability limit.
    #[test]
    fn a_low_ceiling_shrinks_the_step_rather_than_skipping_frames() {
        // The GRIN rod's step at the default mesh, whose threshold is most of
        // the slider.
        let recommended = 6.6e-3;
        let threshold = recommended / PACING_FRAME_SECONDS;
        assert!(
            (0.7..0.85).contains(&threshold),
            "threshold moved: {threshold}"
        );

        // At and above it the mesh keeps its own step and the count does the work.
        assert_eq!(paced_time_step(recommended, 1.0), recommended);
        assert_eq!(paced_time_step(recommended, 2.0), recommended);
        assert!(
            paced_time_step(recommended, 1.0e6) <= recommended,
            "went above the limit"
        );

        // Below it the step follows the ceiling down.
        assert_eq!(
            paced_time_step(recommended, 0.1),
            PACING_FRAME_SECONDS * 0.1
        );
        assert_eq!(
            paced_time_step(recommended, 0.02),
            PACING_FRAME_SECONDS * 0.02
        );

        // Nonsense leaves the mesh's own step alone.
        assert_eq!(paced_time_step(recommended, 0.0), recommended);
        assert_eq!(paced_time_step(recommended, f64::NAN), recommended);
        assert_eq!(paced_time_step(recommended, -1.0), recommended);
    }

    /// What the cap is for: a frame's budget buys at least one step at every
    /// ceiling, on coarse meshes and fine. Without it a coarse mesh leaves half
    /// the frames unadvanced below 0.8x and the picture judders.
    #[test]
    fn a_frame_advances_at_every_ceiling() {
        for recommended in [6.6e-3, 8.9e-4, 6.2e-4] {
            for speed in [2.0, 1.0, 0.5, 0.2, 0.05, 0.02] {
                let step = paced_time_step(recommended, speed);
                assert!(step <= recommended, "{recommended:e} {speed}");
                let mut accumulator = 0.0;
                let idle = (0..600)
                    .filter(|_| {
                        steps_for_frame(&mut accumulator, PACING_FRAME_SECONDS, speed, step) == 0
                    })
                    .count();
                assert_eq!(
                    idle, 0,
                    "recommended {recommended:e} at {speed}x left {idle} frames unadvanced"
                );
            }
        }
    }

    /// The ceiling is on simulated seconds per wall second, so half the speed
    /// asks for half the steps out of the same frame.
    #[test]
    fn speed_scales_the_steps_a_frame_asks_for() {
        let step = 1.0e-3;
        let frame = 16.0e-3;
        let mut full = 0.0;
        let mut half = 0.0;
        let mut quiet = 0.0;
        let (mut full_total, mut half_total, mut quiet_total) = (0, 0, 0);
        for _ in 0..60 {
            full_total += steps_for_frame(&mut full, frame, 1.0, step);
            half_total += steps_for_frame(&mut half, frame, 0.5, step);
            quiet_total += steps_for_frame(&mut quiet, frame, 0.02, step);
        }
        // A second of frames at one millisecond a step.
        assert_eq!(full_total, 960);
        assert_eq!(half_total, 480);
        assert_eq!(quiet_total, 19);
    }

    /// The per-frame ceiling is what turns a speed the scene cannot afford into
    /// a shortfall rather than a runaway backlog.
    #[test]
    fn the_frame_ceiling_caps_the_steps_and_the_leftover() {
        let step = 1.0e-3;
        let mut accumulator = 0.0;
        // A whole clamped frame at double speed wants 200 steps.
        let steps = steps_for_frame(&mut accumulator, 1.0, 2.0, step);
        assert_eq!(steps, MAX_STEPS_PER_FRAME);
        assert!(
            accumulator <= MAX_STEPS_PER_FRAME as f64 * step + 1.0e-12,
            "the leftover built a backlog: {accumulator}"
        );
        // And it stays capped however long the solver is behind.
        for _ in 0..100 {
            steps_for_frame(&mut accumulator, 1.0, 2.0, step);
        }
        assert!(accumulator <= MAX_STEPS_PER_FRAME as f64 * step + 1.0e-12);

        // Nonsense asks for nothing rather than panicking or racing.
        let mut idle = 0.0;
        assert_eq!(steps_for_frame(&mut idle, 0.016, 1.0, 0.0), 0);
        assert_eq!(steps_for_frame(&mut idle, 0.016, 0.0, 1.0e-3), 0);
        assert_eq!(steps_for_frame(&mut idle, -1.0, 1.0, 1.0e-3), 0);
    }

    /// The windowed rate dips whenever a handoff withholds stepping inside its
    /// window, at every speed and including ones the solver reaches easily, so
    /// the note reads a held best rather than the raw measurement.
    #[test]
    fn a_held_rate_rides_over_a_dip_but_not_a_slowdown() {
        let frame = 1.0 / 60.0;
        let mut held = 0.0;
        for _ in 0..120 {
            held = hold_rate(held, 1.0, frame);
        }
        assert_eq!(held, 1.0);

        // A second of the worst dip measured still reads as keeping up.
        let mut dipped = held;
        for _ in 0..60 {
            dipped = hold_rate(dipped, 0.74, frame);
        }
        assert!(
            speed_shortfall(dipped, 1.0, true).is_none(),
            "a dip was reported as a shortfall: {dipped}"
        );

        // A real slowdown gets through inside two seconds.
        let mut slow = held;
        for _ in 0..120 {
            slow = hold_rate(slow, 0.5, frame);
        }
        assert_eq!(speed_shortfall(slow, 1.0, true), Some(slow));

        // A better reading is taken at once, and nonsense is ignored.
        assert_eq!(hold_rate(0.5, 2.0, frame), 2.0);
        assert_eq!(hold_rate(0.5, f64::NAN, frame), 0.5);
        assert_eq!(hold_rate(0.5, -1.0, frame), 0.5);
    }

    /// A rate below the one asked for is worth saying, but only when the solver
    /// is actually trying to reach it.
    #[test]
    fn a_shortfall_is_only_reported_while_the_solver_is_trying() {
        assert_eq!(speed_shortfall(0.34, 1.0, true), Some(0.34));
        assert_eq!(
            speed_shortfall(0.98, 1.0, true),
            None,
            "jitter is not a shortfall"
        );
        assert_eq!(speed_shortfall(0.19, 0.2, true), None);
        assert_eq!(speed_shortfall(0.09, 0.2, true), Some(0.09));
        // Paused, mid-handoff, or before anything has been measured.
        assert_eq!(speed_shortfall(0.34, 1.0, false), None);
        assert_eq!(speed_shortfall(0.0, 1.0, true), None);
        assert_eq!(speed_shortfall(f64::NAN, 1.0, true), None);
    }

    /// Adaptation hands the field to a new mesh every second or two. It is the
    /// same field, so its scale has to carry across: restarting it there dropped
    /// the reference onto the instantaneous level, and a decaying field fell in
    /// visible steps instead of easing down at the release rate.
    #[test]
    fn a_mesh_handoff_leaves_the_scale_alone() {
        let mut state = Playground::default();
        state.field_exposure.update(0.71, 0.016);
        state.vector_overlay_exposure.update(0.71, 0.016);
        state.restart_exposures_after_handoff(false);
        assert_eq!(state.field_exposure.reference(), Some(0.71));
        assert_eq!(state.vector_overlay_exposure.reference(), Some(0.71));

        // A field replaced with zeros starts the scale again.
        state.restart_exposures_after_handoff(true);
        assert_eq!(state.field_exposure.reference(), None);
        assert_eq!(state.vector_overlay_exposure.reference(), None);
    }

    /// Loading a document and pressing Reset both leave the scale alone. Each
    /// zeroes the field on the GPU, but the outgoing scene stays on display
    /// until the replacement arrives — a reset for a few frames, a load for as
    /// long as the new mesh takes. A scale cleared at the request measures that
    /// residue and paints it at full brightness: measured at 0.09 % before, 100 %
    /// after, for a tenth of a second on a reset and four tenths on a load.
    #[test]
    fn asking_for_a_new_field_does_not_magnify_the_outgoing_one() {
        let mut state = Playground::default();
        state.field_exposure.update(3.0e-2, 0.016);
        let residue = 3.8e-6;
        let held = state.field_exposure.update(residue, 0.016).unwrap();
        assert!(residue / held < 0.01, "the residue was not already dark");

        state.reset_requested = true;
        let document = state.editor.document.clone();
        state.set_document(document, false, true).unwrap();
        let after = state.field_exposure.update(residue, 0.016).unwrap();
        assert!(
            residue / after < 0.01,
            "the outgoing field was magnified to {:.0}%",
            100.0 * residue / after
        );
    }

    /// The first frames of a replaced field are numerical dust, and an instant
    /// attack onto a scale with nothing behind it paints that dust at full
    /// colour. Measured at 3.5e-10 arriving one frame before the real field.
    #[test]
    fn a_restarted_scale_does_not_latch_onto_the_first_dust() {
        let mut exposure = AutoExposure::default();
        exposure.update(3.0e-2, 0.016);
        exposure.restart();
        let dust = 3.5e-10;
        let reference = exposure.update(dust, 0.016).unwrap();
        assert!(
            dust / reference < 1.0e-4,
            "dust painted at {:.0}%",
            100.0 * dust / reference
        );
        // The real field, when it arrives, takes the scale straight over.
        assert_eq!(exposure.update(2.0e-2, 0.016), Some(2.0e-2));
    }

    /// The symptom the handoff bug showed as: a scale that falls faster than the
    /// release allows. Nothing `update` does may outrun that rate.
    #[test]
    fn the_scale_never_falls_faster_than_the_release_rate() {
        let mut exposure = AutoExposure::default();
        let mut level = 1.0_f64;
        let step = 1.0_f32 / 60.0;
        let mut previous = exposure.update(level, step).unwrap();
        for _ in 0..1_200 {
            level *= 0.98;
            let reference = exposure.update(level, step).unwrap();
            // The same widening `update` does, so the two agree to the bit.
            let allowed = previous * AutoExposure::RELEASE_PER_SECOND.powf(-f64::from(step));
            assert!(
                reference >= allowed * (1.0 - 1.0e-12),
                "the scale fell to {reference:e} when {allowed:e} was the floor"
            );
            previous = reference;
        }
    }

    /// Turning the automatic scale off has to put the field back exactly where it
    /// was before there was one: the slider as the whole scale.
    #[test]
    fn turning_auto_exposure_off_restores_the_plain_gain() {
        let default = funfern_app::document::PresentationSettings::default();
        assert!(default.field_auto_exposure, "it should start on");
        assert_eq!(field_scale(2.0, Some(0.02), false), 2.0);
        assert_eq!(field_scale(0.25, None, false), 0.25);
        // Automatic, the same field paints the same whatever its size.
        let quiet = field_scale(2.0, Some(6.0e-3), true) * 6.0e-3;
        let loud = field_scale(2.0, Some(7.1e-1), true) * 7.1e-1;
        assert!((quiet - loud).abs() < 1.0e-12);
        assert_eq!(field_scale(2.0, None, true), 0.0);
    }

    #[test]
    fn canonical_render_keeps_authoritative_component_offsets() {
        let values = vec![0.4_f32, 0.41, -0.9, -0.89];
        let mut state = Playground::default();
        state.field_render.clone_from(&values);
        assert_eq!(state.field_render, values);
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

    /// A thumbnail is rasterized, not traced, so a face covers area rather than
    /// only an outline, and every quad lands inside the scene it came from.
    #[test]
    fn a_thumbnail_fills_the_faces_of_the_scene_it_previews() {
        let example = &funfern_app::topology_examples::catalog()[0];
        let preview = build_example_preview(example);
        let domain = example.document.model.accepted.geometry.domain;
        assert!(preview.quads.len() > PREVIEW_ROWS, "a face was not filled");
        assert!(!preview.strokes.is_empty(), "nothing was outlined");
        for quad in &preview.quads {
            assert!(quad.low.x < quad.high.x && quad.low.y < quad.high.y);
            assert!(
                quad.low.x >= domain.min_x - 1e-9
                    && quad.high.x <= domain.max_x + 1e-9
                    && quad.low.y >= domain.min_y - 1e-9
                    && quad.high.y <= domain.max_y + 1e-9,
                "a quad left the domain"
            );
        }
        let colors = preview
            .quads
            .iter()
            .map(|quad| quad.color.to_array())
            .collect::<BTreeSet<_>>();
        assert!(
            colors.len() > 1,
            "the obstacle is the same colour as the background it sits in"
        );
    }

    /// The reason a thumbnail samples cell centres rather than polygon corners:
    /// every corner of a Luneburg lens sits on the same circle, so a profile
    /// read at the corners alone is one flat colour.
    #[test]
    fn a_radial_material_profile_reaches_the_thumbnail() {
        let example = funfern_app::topology_examples::catalog()
            .iter()
            .find(|example| example.name == "Luneburg lens")
            .expect("the catalog still carries the Luneburg lens");
        assert!(matches!(
            example.document.presentation.material_overlay,
            MaterialOverlay::Property(MaterialProperty::WaveSpeed)
        ));
        let preview = build_example_preview(example);
        let shades = preview
            .quads
            .iter()
            .map(|quad| quad.color.to_array())
            .collect::<BTreeSet<_>>();
        assert!(
            shades.len() > 8,
            "the lens reads as {} colour(s), not a profile",
            shades.len()
        );
    }

    /// Even-odd pairing across every cycle at once is what keeps a face out of
    /// its own holes, and a slit traced out and back contributes nothing.
    #[test]
    fn scanline_spans_skip_the_holes_in_a_face() {
        let outer = vec![
            Point2::new(-1.0, -1.0),
            Point2::new(1.0, -1.0),
            Point2::new(1.0, 1.0),
            Point2::new(-1.0, 1.0),
        ];
        let hole = vec![
            Point2::new(-0.5, -0.5),
            Point2::new(-0.5, 0.5),
            Point2::new(0.5, 0.5),
            Point2::new(0.5, -0.5),
        ];
        assert_eq!(
            face_spans(std::slice::from_ref(&outer), 0.0),
            vec![(-1.0, 1.0)]
        );
        assert_eq!(
            face_spans(&[outer.clone(), hole], 0.0),
            vec![(-1.0, -0.5), (0.5, 1.0)]
        );
        // A line above the face crosses nothing.
        assert!(face_spans(&[outer], 2.0).is_empty());
    }

    /// New is a scene that can be run, not just an empty one.
    #[test]
    fn a_new_scene_is_empty_outgoing_and_driven() {
        let mut state = Playground::default();
        state.new_scene();
        let scene = &state.editor.document.model.accepted;
        assert!(scene.geometry.curves.is_empty());
        assert_eq!(scene.regions.len(), 1);
        for side in OuterSide::ALL {
            assert_eq!(
                scene.outer_boundaries.sides[side.index()],
                OuterBoundaryCondition::SecondOrderOutgoing
            );
        }
        assert!(state.editor.document.model.source.enabled);
        assert_eq!(state.example_opened, None, "a new scene is not an example");
        assert!(state.editor.undo(), "New is one undoable action");
    }

    /// The gallery survives a pick, and the pick is what changes the document.
    #[test]
    fn opening_an_example_leaves_the_gallery_open_and_marks_the_row() {
        let mut state = Playground {
            examples_open: true,
            ..Playground::default()
        };
        let index = funfern_app::topology_examples::catalog()
            .iter()
            .position(|example| example.name == "Double slit")
            .unwrap();
        state.open_example(index);
        assert!(state.examples_open, "the gallery closed on a pick");
        assert_eq!(state.example_opened, Some(index));
        assert_eq!(
            state.editor.document.model,
            funfern_app::topology_examples::catalog()[index]
                .document
                .model
        );
        state.new_scene();
        assert_eq!(state.example_opened, None, "the marker outlived its scene");
    }

    /// Every catalog entry has to reach the gallery, and building them all is
    /// spread over frames because the largest one costs a frame by itself.
    #[test]
    fn every_example_previews_and_the_cache_fills_one_per_frame() {
        let mut state = Playground::default();
        let total = funfern_app::topology_examples::catalog().len();
        assert_eq!(state.example_previews.len(), total);
        assert!(state.example_previews.iter().all(Option::is_none));
        for filled in 1..=total {
            let index = state
                .example_previews
                .iter()
                .position(Option::is_none)
                .expect("an unbuilt preview");
            state.example_previews[index] = Some(build_example_preview(
                &funfern_app::topology_examples::catalog()[index],
            ));
            assert_eq!(
                state
                    .example_previews
                    .iter()
                    .filter(|p| p.is_some())
                    .count(),
                filled
            );
        }
        for (index, preview) in state.example_previews.iter().enumerate() {
            let preview = preview.as_ref().unwrap();
            assert!(
                !preview.quads.is_empty(),
                "{} previews as nothing",
                funfern_app::topology_examples::catalog()[index].name
            );
        }
    }

    /// The random start has to land on every example and never off the end.
    #[test]
    fn the_random_start_stays_inside_the_catalog() {
        let total = funfern_app::topology_examples::catalog().len();
        let mut seen = BTreeSet::new();
        for step in 0..=1000 {
            let index = random_example_index(f64::from(step) / 1000.0, total);
            assert!(index < total, "{step} lands past the catalog");
            seen.insert(index);
        }
        assert_eq!(seen.len(), total, "some example can never open at startup");
        assert!((0.0..1.0).contains(&random_fraction()));
    }

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

    /// Every channel the log watches overwrites itself, so a change is the only
    /// moment its value can be caught. Holding a value logs it once, clearing
    /// and returning without anything in between counts a repeat rather than
    /// filling the ring, and an error keeps the status marker lit until the
    /// window is opened on it.
    #[test]
    fn the_log_keeps_one_entry_per_change_of_a_transient_channel() {
        let mut state = Playground::default();
        state.record_events(1.0);
        assert!(state.events.is_empty());
        assert!(!state.diagnostics_warning());

        state.message = "Pulse queued in region 1".into();
        state.record_events(2.0);
        state.record_events(3.0);
        assert_eq!(state.events.len(), 1, "a held value is logged once");
        assert_eq!(state.events[0].source, EventSource::Status);
        assert_eq!(state.events[0].repeats, 1);
        assert!(
            !state.diagnostics_warning(),
            "a status line is not an error"
        );

        state.amr_error = Some("Invalid adaptation source".into());
        state.record_events(4.0);
        assert_eq!(state.events.len(), 2);
        assert_eq!(state.events[1].source, EventSource::Adaptation);
        assert!(state.unseen_error);

        // The adaptation clears itself and fails again with nothing logged in
        // between, which is the shape that would otherwise flood the ring.
        for time in [5.0, 6.0, 7.0, 8.0] {
            state.amr_error = (time as u64)
                .is_multiple_of(2)
                .then(|| "Invalid adaptation source".to_owned());
            state.record_events(time);
        }
        assert_eq!(state.events.len(), 2, "{:?}", state.events);
        assert_eq!(state.events[1].repeats, 3);

        // A different message is its own entry, and the marker survives the
        // error clearing.
        state.amr_error = None;
        state.message = "Simulation topology committed".into();
        state.record_events(9.0);
        assert_eq!(state.events.len(), 3);
        assert_eq!(state.events[2].source, EventSource::Status);
        assert!(
            state.diagnostics_warning(),
            "the marker stays lit for an error the window has not been opened on"
        );

        state.unseen_error = false;
        assert!(!state.diagnostics_warning());
        assert_eq!(
            event_line(&state.events[1]),
            "0:08.0 · adaptation · Invalid adaptation source ×3"
        );

        // A repair fallback is queued by the transaction that reported it and
        // stamped on the next frame, so two transactions that fall back the
        // same way are counted rather than reading as one.
        state.pending_repairs = vec!["mesh repair failed".into(), "mesh repair failed".into()];
        state.record_events(10.0);
        assert!(state.pending_repairs.is_empty());
        assert_eq!(state.events.len(), 4);
        assert_eq!(state.events[3].source, EventSource::Repair);
        assert_eq!(state.events[3].repeats, 2);
        assert!(
            !state.diagnostics_warning(),
            "a rebuild that carried the edit through is not an error"
        );
    }

    /// The picker lists one set of names and the two condition enums answer
    /// with their own, so a condition used to rename itself the moment it was
    /// chosen. They are held to the same words here, in both directions: every
    /// kind is reachable and every condition reads back as the kind that lists
    /// it.
    #[test]
    fn boundary_names_agree_across_every_source() {
        let signal = TimeSignal::harmonic(0.0, 1.0, 1.0, 0.0);
        let faces = [
            FaceBoundaryCondition::Reflecting,
            FaceBoundaryCondition::Impedance { ratio: 1.0 },
            FaceBoundaryCondition::SecondOrderOutgoing,
            FaceBoundaryCondition::ElectricWall,
            FaceBoundaryCondition::MagneticWall,
            FaceBoundaryCondition::Neumann { signal },
            FaceBoundaryCondition::Dirichlet { signal },
        ];
        for condition in faces {
            assert_eq!(
                face_kind(condition).label(),
                condition.label(),
                "{condition:?} is listed under another name"
            );
        }
        let outers = [
            OuterBoundaryCondition::Reflecting,
            OuterBoundaryCondition::FirstOrderOutgoing,
            OuterBoundaryCondition::SecondOrderOutgoing,
            OuterBoundaryCondition::ElectricWall,
            OuterBoundaryCondition::MagneticWall,
            OuterBoundaryCondition::Neumann { signal },
            OuterBoundaryCondition::Dirichlet { signal },
        ];
        for condition in outers {
            assert_eq!(
                outer_kind(condition).label(),
                condition.label(),
                "{condition:?} is listed under another name"
            );
        }
        // Each list covers every kind the picker offers, so nothing is
        // unreachable and no two kinds share a name.
        for kinds in [
            faces.map(face_kind).to_vec(),
            outers.map(outer_kind).to_vec(),
        ] {
            assert_eq!(kinds, BoundaryKind::ALL.to_vec());
        }
        assert_eq!(
            BoundaryKind::ALL
                .iter()
                .map(|kind| kind.label())
                .collect::<BTreeSet<_>>()
                .len(),
            BoundaryKind::ALL.len()
        );
    }

    /// Both axes step upwards from the lower bound. The vertical one used to
    /// start at the top of the view and test against the bottom, so the grid
    /// had only ever been columns.
    #[test]
    fn the_grid_covers_both_axes_of_the_view() {
        // An 800x600 view at 300 pixels per world unit, centred on the origin.
        let state = Playground {
            scale: 300.0,
            center: Point2::default(),
            ..Playground::default()
        };
        let view = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let (step, fine) = grid_steps(state.scale);
        assert_eq!(step, 0.5, "70 px at this zoom lands on the half-unit");
        assert_eq!(fine, 0.1, "which divides into five");
        let (columns, rows) = state.grid_axes(view, step);
        assert_eq!(columns, vec![-1.5, -1.0, -0.5, 0.0, 0.5, 1.0]);
        assert_eq!(rows, vec![-1.0, -0.5, 0.0, 0.5, 1.0]);
        assert!(
            rows.iter().any(|y| y.abs() < step * 0.1),
            "the horizontal axis is among them, and is the emphasised line"
        );
        // The fine lattice nests inside the drawn one, so every line that is
        // drawn is one Shift can land on.
        assert!(
            (step / fine - 5.0).abs() < 1.0e-9,
            "a half-unit divides in five"
        );
        let (fine_columns, fine_rows) = state.grid_axes(view, fine);
        assert!(fine_columns.len() >= columns.len() * 4);
        assert!(fine_rows.len() >= rows.len() * 4);

        // A bound the wrong way round draws nothing rather than looping.
        assert!(grid_lines(1.0, -1.0, step).is_empty());
        assert!(grid_lines(-1.0, 1.0, 0.0).is_empty());
        assert!(grid_lines(f64::NAN, 1.0, step).is_empty());
        // A degenerate zoom cannot hang the painter.
        assert!(grid_lines(-1.0, 1.0, grid_steps(0.0).0).is_empty());
        assert!(grid_lines(-1.0e9, 1.0e9, 1.0e-9).len() <= 4096);

        // The spacing holds its decade: about 70 pixels apart at any zoom, and
        // the step Shift lands on is always a round division of it.
        for scale in [12.0, 37.0, 300.0, 1_500.0, 9_000.0] {
            let (step, fine) = grid_steps(scale);
            let pixels = step * scale;
            assert!(
                (35.0..=180.0).contains(&pixels),
                "{scale} pixels per unit put lines {pixels} apart"
            );
            let divisions = step / fine;
            assert!(
                (divisions - divisions.round()).abs() < 1.0e-9 && (4.0..=5.0).contains(&divisions),
                "{scale} divides {step} into {divisions}"
            );
        }
    }

    /// Shift places a point on the same 0.05 grid every drag snaps to. A
    /// default editor has compiled nothing, so no attachment can outrank it and
    /// this is the grid path.
    #[test]
    fn shift_places_a_drawn_point_on_the_grid() {
        let viewport = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        // The grid belongs to the zoom, so the zoom is named: 400 pixels per
        // unit divides fifths of a unit in four, landing on 0.05.
        let mut state = Playground {
            scale: 400.0,
            ..Playground::default()
        };
        state.begin_draw(DrawTool::Polyline);
        state.draw_click(
            Point2::new(0.117, -0.233),
            ScreenPoint::new(0.0, 0.0),
            viewport,
            true,
        );
        state.draw_click(
            Point2::new(-0.481, 0.062),
            ScreenPoint::new(0.0, 0.0),
            viewport,
            false,
        );
        let points = &state.draw.as_ref().unwrap().points;
        assert_eq!(points[0], Point2::new(0.10, -0.25));
        assert_eq!(
            points[1],
            Point2::new(-0.481, 0.062),
            "without shift the point stays where it was put"
        );

        // Every tool goes through the same place, the two-click rectangle
        // included. Clear of the default scene's loop, so it compiles.
        let before = state.editor.document.model.draft.geometry.curves.len();
        state.begin_draw(DrawTool::Rectangle);
        for point in [Point2::new(0.537, 0.562), Point2::new(0.873, 0.818)] {
            state.draw_click(point, ScreenPoint::new(0.0, 0.0), viewport, true);
        }
        assert_eq!(
            state.editor.document.model.draft.geometry.curves.len(),
            before + 1,
            "the rectangle was refused: {}",
            state.message
        );
        let rectangle = state
            .editor
            .document
            .model
            .draft
            .geometry
            .curves
            .last()
            .expect("the rectangle was created")
            .spline
            .clone();
        assert_eq!(rectangle.node_count(), 4);
        for index in 0..rectangle.node_count() {
            let corner = rectangle.node_point(index).unwrap();
            for value in [corner.x, corner.y] {
                let steps = value / 0.05;
                assert!(
                    (steps - steps.round()).abs() < 1.0e-9,
                    "corner off the grid at {value}"
                );
            }
        }
    }

    /// Picking a tool starts the gesture and leaves the palette up, so one
    /// primitive can follow another without a trip back to the toolbar.
    #[test]
    fn the_draw_palette_outlives_the_gesture_it_starts() {
        let mut state = Playground {
            draw_open: true,
            ..Playground::default()
        };
        state.begin_draw(DrawTool::Circle);
        assert!(state.draw.is_some());
        assert!(state.draw_open, "the palette closes only when it is closed");
        state.begin_draw(DrawTool::Polyline);
        assert!(state.draw_open);
    }

    /// Handover generations preserve the accepted-step total. Their first
    /// observation establishes a baseline instead of re-crediting the run;
    /// genuinely new steps on either side remain in the same rate window.
    #[test]
    fn the_step_rate_survives_a_handover() {
        let mut state = Playground::default();
        // Four frames of a settled generation, then the window closes.
        state.accumulate_step_rate(7, 0, 0.0);
        for (frame, completed) in [(1, 300_u64), (2, 600), (3, 900), (4, 1200)] {
            state.accumulate_step_rate(7, completed, if frame == 4 { 0.5 } else { 0.1 });
        }
        assert_eq!(state.steps_per_second, 2400.0);
        assert_eq!(state.rate_window_steps, 0, "a closed window starts empty");

        // A handover mid-window preserves its total, and the steps already
        // banked stay without the preserved total being counted a second time.
        state.accumulate_step_rate(7, 1500, 0.1);
        assert_eq!(state.rate_window_steps, 300);
        state.accumulate_step_rate(8, 1500, 0.1);
        assert_eq!(
            state.rate_window_steps, 300,
            "the preserved total is a baseline and the old progress is kept"
        );
        state.accumulate_step_rate(8, 1700, 0.5);
        assert_eq!(state.steps_per_second, 1000.0);

        // A window holding only the frames either side of another handover.
        state.accumulate_step_rate(8, 5000, 0.1);
        state.accumulate_step_rate(9, 5000, 0.5);
        assert_eq!(state.steps_per_second, 3300.0 / 0.5);
        assert!(
            state.steps_per_second > 0.0,
            "a handover is not a stall in the solver"
        );

        // A fresh install may reset the counter; its first observation is also
        // only a baseline, and subsequent progress is measured normally.
        state.accumulate_step_rate(10, 0, 0.1);
        state.accumulate_step_rate(10, 250, 0.5);
        assert_eq!(state.steps_per_second, 500.0);
    }

    /// The ring drops its oldest rather than growing without bound.
    #[test]
    fn the_log_is_bounded() {
        let mut state = Playground::default();
        for index in 0..EVENT_LOG_ENTRIES + 20 {
            state.message = format!("message {index}");
            state.record_events(index as f64);
        }
        assert_eq!(state.events.len(), EVENT_LOG_ENTRIES);
        assert_eq!(state.events[0].text, format!("message {}", 20));
        assert_eq!(
            state.events[EVENT_LOG_ENTRIES - 1].text,
            format!("message {}", EVENT_LOG_ENTRIES + 19)
        );
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
    fn vector_overlay_layout_selects_at_most_one_sample_per_screen_bin() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        let active = activate_at(&mut state, 0.08);
        let viewport = viewport();
        let spacing = 28.0;
        let points = vector_overlay_layout(
            &active.bundle.authored,
            &active.mesh,
            &active.operator,
            spacing,
            (state.center, state.scale, viewport),
        );
        assert!(!points.is_empty());
        let keys = points
            .iter()
            .map(|point| {
                let screen = state.screen(point.point, viewport);
                (
                    ((screen.x - viewport.left()) / spacing).floor() as i32,
                    ((screen.y - viewport.top()) / spacing).floor() as i32,
                )
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(keys.len(), points.len());
        let maximum_bins = (viewport.width() / spacing).ceil() as usize
            * (viewport.height() / spacing).ceil() as usize;
        assert!(points.len() <= maximum_bins);
        assert!(points.iter().all(|point| {
            point.point.x.is_finite()
                && point.point.y.is_finite()
                && point.stencil.element < active.mesh.triangles.len() as u32
        }));
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

    #[test]
    fn polyline_anchor_carries_the_segment_it_landed_on() {
        let path = [
            Point2::new(0.0, 0.0),
            Point2::new(2.0, 0.0),
            Point2::new(2.0, 2.0),
        ];
        let (point, segment) = Playground::polyline_anchor(&path, 0.25).unwrap();
        assert!((point - Point2::new(1.0, 0.0)).norm() < 1.0e-12);
        assert_eq!(segment, [path[0], path[1]]);
        let (point, segment) = Playground::polyline_anchor(&path, 0.75).unwrap();
        assert!((point - Point2::new(2.0, 1.0)).norm() < 1.0e-12);
        assert_eq!(segment, [path[1], path[2]]);
        assert_eq!(
            Playground::polyline_anchor(&[Point2::new(1.0, 3.0)], 0.5),
            None
        );
    }

    /// The arrow is drawn from the trace the probe reads into its badge, so it
    /// has to run out of that trace. `Left` names the face on the left of
    /// increasing parameter, so on a path running east the left trace is the
    /// northern one and an arrow arriving from it points south.
    #[test]
    fn a_boundary_probes_arrow_leaves_the_trace_it_reads() {
        let path = [Point2::new(-1.0, 0.0), Point2::new(1.0, 0.0)];
        let outward = |side, reversed| boundary_probe_orientation(&path, side, reversed).unwrap().1;
        assert!((outward(CurveTraceSide::Left, false) - Point2::new(0.0, -1.0)).norm() < 1.0e-12);
        assert!((outward(CurveTraceSide::Right, false) - Point2::new(0.0, 1.0)).norm() < 1.0e-12);
        // Reversing turns the arclength axis, never the side that is read.
        assert!((outward(CurveTraceSide::Left, true) - Point2::new(0.0, -1.0)).norm() < 1.0e-12);
        assert!((outward(CurveTraceSide::Right, true) - Point2::new(0.0, 1.0)).norm() < 1.0e-12);
    }

    /// The two arrows share a corner, so reversing has to turn one of them and
    /// leave the other where it is: the arclength axis runs the other way, the
    /// side that is read does not change, and the corner stays put.
    #[test]
    fn reversing_a_boundary_probe_turns_its_arclength_arrow_alone() {
        let path = [
            Point2::new(-1.0, 0.0),
            Point2::new(0.0, 0.0),
            Point2::new(0.0, 2.0),
        ];
        let orientation =
            |reversed| boundary_probe_orientation(&path, CurveTraceSide::Left, reversed).unwrap();
        let (forward_point, forward_outward, forward_along) = orientation(false);
        let (back_point, back_outward, back_along) = orientation(true);
        assert!((forward_point - Point2::new(0.0, 0.5)).norm() < 1.0e-12);
        assert!((back_point - forward_point).norm() < 1.0e-12);
        assert!((forward_along - Point2::new(0.0, 1.0)).norm() < 1.0e-12);
        assert!((back_along - Point2::new(0.0, -1.0)).norm() < 1.0e-12);
        assert!((forward_outward - Point2::new(1.0, 0.0)).norm() < 1.0e-12);
        assert!((back_outward - forward_outward).norm() < 1.0e-12);
    }

    #[test]
    fn a_boundary_path_too_short_to_orient_draws_no_marks() {
        for path in [vec![], vec![Point2::new(0.0, 0.0)]] {
            assert!(boundary_probe_orientation(&path, CurveTraceSide::Left, false).is_none());
        }
        let stationary = [Point2::new(1.0, 1.0), Point2::new(1.0, 1.0)];
        assert!(boundary_probe_orientation(&stationary, CurveTraceSide::Left, false).is_none());
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

    /// Placing a probe reads the same modifier dragging one already does, with
    /// the same conventions: a position lands on the grid, and a disk's radius
    /// is itself a multiple of it rather than the distance to a snapped rim.
    ///
    /// The grid is the zoom's, so the zoom is named. At 400 pixels per unit it
    /// draws fifths of a unit and divides them in four, which is the 0.05 this
    /// used to be fixed at.
    #[test]
    fn shift_places_a_probe_on_the_grid() {
        let step = grid_steps(400.0).1;
        assert!((step - 0.05).abs() < 1.0e-9, "the zoom moved: {step}");
        let on_grid = |value: f64| {
            let steps = value / step;
            (steps - steps.round()).abs() < 1.0e-9
        };
        let at = |point: Point2, x: f64, y: f64| {
            assert!(
                on_grid(point.x) && on_grid(point.y),
                "{point:?} is off the grid"
            );
            assert!(
                (point.x - x).abs() < 1.0e-9 && (point.y - y).abs() < 1.0e-9,
                "{point:?} is not the nearest node to ({x}, {y})"
            );
        };
        let mut state = Playground {
            probe_mode: Some(ProbePlacement::Point),
            scale: 400.0,
            ..Playground::default()
        };
        state.probe_placement_click(Point2::new(0.117, -0.233), true);
        let TopologyProbeTarget::Point(placed) =
            state.editor.document.model.probes.last().unwrap().target
        else {
            panic!("a point probe")
        };
        at(placed, 0.10, -0.25);

        state.probe_mode = Some(ProbePlacement::Segment { start: None });
        state.probe_placement_click(Point2::new(-0.312, 0.081), true);
        state.probe_placement_click(Point2::new(0.446, -0.377), true);
        let TopologyProbeTarget::Segment { start, end, .. } = state
            .editor
            .document
            .model
            .probes
            .last()
            .unwrap()
            .target
            .clone()
        else {
            panic!("a line probe")
        };
        at(start, -0.30, 0.10);
        at(end, 0.45, -0.40);

        // The rim click lands at a distance of 0.3111…, which is no multiple of
        // the grid; snapping the point rather than the radius would keep it.
        state.probe_mode = Some(ProbePlacement::Disk { center: None });
        state.probe_placement_click(Point2::new(0.019, -0.022), true);
        state.probe_placement_click(Point2::new(0.244, 0.193), true);
        let TopologyProbeTarget::AreaDisk { center, radius } =
            state.editor.document.model.probes.last().unwrap().target
        else {
            panic!("a disk probe")
        };
        at(center, 0.0, 0.0);
        assert!(on_grid(radius), "radius {radius} is off the grid");
        assert!(radius >= step);

        // Without the modifier nothing moves.
        state.probe_mode = Some(ProbePlacement::Point);
        state.probe_placement_click(Point2::new(0.117, -0.233), false);
        let TopologyProbeTarget::Point(loose) =
            state.editor.document.model.probes.last().unwrap().target
        else {
            panic!("a point probe")
        };
        assert_eq!(loose, Point2::new(0.117, -0.233));
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
        state.pending_merge = Some(PendingMerge {
            action: MergeAction::Delete(BTreeSet::from([CurveSpanId(99)])),
            choices: vec![],
        });
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
    fn vector_overlay_exposes_the_shared_canonical_vectors_in_every_skin() {
        assert_eq!(
            VectorOverlay::choices(PhysicsModel::Mechanical),
            &[
                VectorOverlay::Off,
                VectorOverlay::ComplementaryField,
                VectorOverlay::RelativeEnergyFlow,
            ]
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
            VectorOverlay::ComplementaryField
        );
        assert_eq!(
            VectorOverlay::ComplementaryField.resolved(PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            }),
            VectorOverlay::ComplementaryField
        );
        assert_eq!(
            VectorOverlay::ComplementaryField.label(PhysicsModel::Mechanical),
            "In-plane field"
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

    /// Two subdomains selected and deleted in one gesture. Asking curve by
    /// curve asked about the first one only, closed the history entry to ask,
    /// and then returned - so the rest of the selection was never deleted and
    /// nothing said so. One question over the whole deletion, one entry.
    #[test]
    fn a_multi_curve_deletion_asks_once_and_takes_everything() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        for centre in [Point2::new(-0.4, 0.0), Point2::new(0.4, 0.0)] {
            state
                .editor
                .create_closed_curve(
                    PeriodicCubicSpline::rounded(centre, 0.25),
                    ClosedCurvePurpose::Subdomain {
                        material: DEFAULT_MATERIAL,
                    },
                )
                .unwrap();
            settle(&mut state.editor);
        }
        state.selection = TopologySelection::Spans(every_span(&state));
        let history = state.editor.history_len();

        state.delete_selection();
        let pending = state.pending_merge.clone().expect("a survivor question");
        assert_eq!(
            pending.choices.len(),
            3,
            "both subdomains and the background meet in one face: {:?}",
            pending.choices
        );
        assert_eq!(
            state.editor.history_len(),
            history,
            "nothing is deleted until the question is answered"
        );

        state.pick_merge_survivor(Point2::new(0.0, 0.95));
        settle(&mut state.editor);
        assert!(state.pending_merge.is_none());
        assert!(
            state.editor.document.model.draft.geometry.curves.is_empty(),
            "the whole selection goes, not the curve the question was about"
        );
        assert_eq!(
            state.editor.history_len(),
            (history.0 + 1, history.1),
            "one gesture, one entry"
        );
        assert!(state.editor.undo());
        settle(&mut state.editor);
        assert_eq!(
            state.editor.document.model.draft.geometry.curves.len(),
            2,
            "one undo brings the whole gesture back"
        );
    }

    /// A selection covering one curve whole and part of another used to delete
    /// the whole one and ignore the spans on the other without a word.
    #[test]
    fn a_deletion_of_one_whole_curve_and_part_of_another_takes_both() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        let mut holes = vec![];
        for centre in [Point2::new(-0.4, 0.0), Point2::new(0.4, 0.0)] {
            holes.push(
                state
                    .editor
                    .create_closed_curve(
                        PeriodicCubicSpline::rounded(centre, 0.25),
                        ClosedCurvePurpose::Hole,
                    )
                    .unwrap(),
            );
            settle(&mut state.editor);
        }
        let geometry = &state.editor.document.model.draft.geometry;
        let whole = geometry.curve(holes[0]).unwrap();
        let cut = geometry.curve(holes[1]).unwrap();
        let spans = whole
            .spans
            .iter()
            .map(|span| TopologySpanTarget::Curve(span.id))
            .chain(
                cut.spans
                    .iter()
                    .take(2)
                    .map(|span| TopologySpanTarget::Curve(span.id)),
            )
            .collect::<BTreeSet<_>>();
        let survivors = cut.spans.len() - 2;
        state.selection = TopologySelection::Spans(spans);
        let history = state.editor.history_len();

        state.delete_selection();
        settle(&mut state.editor);
        assert!(state.pending_merge.is_none(), "{}", state.message);
        let curves = &state.editor.document.model.draft.geometry.curves;
        assert_eq!(curves.len(), 1, "one baffle is left, and only that");
        assert_eq!(
            curves[0].spans.len(),
            survivors,
            "the run went, the rest stayed"
        );
        assert_eq!(
            state.editor.history_len(),
            (history.0 + 1, history.1),
            "one gesture, one entry"
        );
    }

    fn every_span(state: &Playground) -> BTreeSet<TopologySpanTarget> {
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

    /// The scene asks for a weld the same way it asks for a deletion: the
    /// question is staged, nothing is welded, and the click that names a
    /// subdomain finishes the weld that raised it.
    #[test]
    fn a_weld_that_merges_subdomains_is_staged_for_the_picker() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        let free = state
            .editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    Point2::new(0.4, -0.3),
                    Point2::new(0.4, 0.0),
                    Point2::new(0.4, 0.3),
                ])
                .unwrap(),
                OpenCurvePurpose::BoundaryBaffle,
                None,
                None,
            )
            .unwrap()
            .curve;
        settle(&mut state.editor);
        let divider = state
            .editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    Point2::new(0.0, -0.8),
                    Point2::new(0.0, 0.0),
                    Point2::new(0.0, 0.8),
                ])
                .unwrap(),
                OpenCurvePurpose::BoundaryBaffle,
                Some(TopologyAttachment::Boundary(FaceAnchor::Outer {
                    side: OuterSide::Bottom,
                    fraction: 0.5,
                })),
                Some(TopologyAttachment::Boundary(FaceAnchor::Outer {
                    side: OuterSide::Top,
                    fraction: 0.5,
                })),
            )
            .unwrap()
            .curve;
        settle(&mut state.editor);
        assert_eq!(state.editor.document.model.draft.regions.len(), 2);

        // Anchor on the middle of the free baffle, whichever side resolves.
        let compiled = &state.editor.compiled_accepted;
        let owner = compiled.geometry.curve(free).unwrap();
        let span = owner.spans[1].id;
        let [a, b] = owner.spline.span_bounds(1).unwrap();
        let parameter = (a + b) * 0.5;
        let side = [CurveTraceSide::Left, CurveTraceSide::Right]
            .into_iter()
            .find(|side| {
                FaceAnchor::Curve {
                    curve: free,
                    span,
                    side: *side,
                    parameter,
                }
                .resolve(&compiled.topology)
                .is_ok()
            })
            .expect("a resolvable side");
        let onto = TopologyAttachment::Boundary(FaceAnchor::Curve {
            curve: free,
            span,
            side,
            parameter,
        });
        state.editor.detach_endpoint(divider, 1).unwrap();
        settle(&mut state.editor);
        let node = state
            .editor
            .document
            .model
            .draft
            .geometry
            .curve(divider)
            .unwrap()
            .nodes
            .len()
            - 1;

        state.weld(divider, node, 1, onto, None);
        let pending = state.pending_merge.clone().expect("a survivor question");
        assert!(matches!(pending.action, MergeAction::Weld { .. }));
        assert_eq!(pending.choices.len(), 2);
        assert_eq!(state.editor.document.model.draft.regions.len(), 2);

        state.pick_merge_survivor(Point2::new(-0.5, 0.0));
        settle(&mut state.editor);
        assert!(state.pending_merge.is_none(), "{}", state.message);
        assert_eq!(state.editor.document.model.draft.regions.len(), 1);
        assert_eq!(state.editor.acceptance, TopologyAcceptance::Valid);
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

    /// A subdomain marker is placed from the compiled geometry, so it exists
    /// before any mesh does and sits where the region's area is.
    #[test]
    fn a_subdomain_marker_is_placed_from_the_geometry() {
        let mut state = Playground::default();
        let probe = state
            .editor
            .create_probe(
                "Field".into(),
                [91, 220, 194],
                TopologyProbeTarget::AreaRegion(RegionId(1)),
            )
            .unwrap();
        assert!(
            state.runtime.active().is_none(),
            "the scene has not been meshed"
        );
        state.refresh_probe_metadata();
        let anchor = state
            .probe_anchors
            .get(&probe)
            .copied()
            .expect("the marker is placed without a mesh");

        let scene = &state.editor.compiled_accepted;
        let hole = scene
            .assignments
            .iter()
            .find(|assignment| assignment.region.is_none())
            .and_then(|assignment| scene.topology.face(assignment.face))
            .and_then(|face| face.centroid())
            .expect("the default scene holds one excluded face");
        let center = scene.geometry.domain.center();
        // Taking area out of a shape moves its centroid away from where that
        // area was, and no further than the area which left could carry it.
        // Averaging triangle centroids moved it the other way, towards the
        // dense mesh the hole's boundary asks for.
        let moved = anchor - center;
        let away = center - hole;
        assert!(moved.norm() > 0.0, "the hole moved the marker");
        assert!(
            moved.dot(away) / (moved.norm() * away.norm()) > 0.999,
            "the marker moved {moved:?}, away from the hole is {away:?}"
        );
        assert!(moved.norm() < away.norm(), "the marker left the region");
    }

    /// The marker is the region's, not the mesh's, so meshing the same scene
    /// twice at different densities has to leave it exactly where it was.
    #[test]
    fn a_subdomain_marker_never_moves_with_the_mesh() {
        let mut state = Playground::default();
        let probe = state
            .editor
            .create_probe(
                "Field".into(),
                [91, 220, 194],
                TopologyProbeTarget::AreaRegion(RegionId(1)),
            )
            .unwrap();
        let mut placed = Vec::new();
        for target in [0.30, 0.09] {
            let active = activate_at(&mut state, target);
            let triangles = active.mesh.triangles.len();
            state.refresh_probe_metadata();
            let anchor = state
                .probe_anchors
                .get(&probe)
                .copied()
                .expect("the marker is placed");
            placed.push((triangles, anchor));
        }
        let [(coarse, first), (fine, second)] = placed[..] else {
            unreachable!()
        };
        assert!(fine > coarse * 4, "the two meshes differ: {coarse} {fine}");
        assert_eq!(
            first, second,
            "the marker moved between a {coarse} and a {fine} triangle mesh"
        );
    }

    /// Prepares the editor's accepted scene and makes it the active topology,
    /// the way a frame does once the GPU has acknowledged the upload.
    fn activate(state: &mut Playground) -> Arc<PreparedTopology> {
        activate_at(state, 0.18)
    }

    /// `activate` at a chosen mesh density, for anything that has to hold
    /// across two different meshes of one scene.
    fn activate_at(state: &mut Playground, target_edge_length: f64) -> Arc<PreparedTopology> {
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

        state.refresh_amr(
            &CanonicalGpuRequest::default(),
            &CanonicalGpuDisplay::default(),
            &WaveDisplay::default(),
        );
        assert!(state.amr_adaptation_job.is_none());
        assert!(state.amr_adaptation_source.is_none());
        assert_eq!(state.amr_error, None);
        assert!(
            state.amr_status.contains("discarded"),
            "status was {:?}",
            state.amr_status
        );
    }

    /// The estimate has no notion of enough on its own. Its step down is
    /// clamped, so an element it cannot satisfy - a boundary the field
    /// disagrees with, the grid-scale leftovers of a wave that has passed -
    /// asks for the same refinement however loose the target is, and walks to
    /// the smallest element allowed. The accuracy target is what answers that,
    /// and it answers for the whole field at once; the floors it does not
    /// answer for at all.
    #[test]
    fn the_accuracy_target_settles_error_driven_refinement_alone() {
        let report = |error, limit, global| SolutionIndicatorReport {
            refine_candidates: error + limit,
            error_refine_candidates: error,
            limit_refine_candidates: limit,
            global_indicator: global,
            ..Default::default()
        };
        assert!(adaptation_refines(&report(2000, 0, 0.2), 0.12));
        assert!(!adaptation_refines(&report(2000, 0, 0.05), 0.12));
        // The same estimate, asked for more: still running.
        assert!(adaptation_refines(&report(2000, 0, 0.05), 0.04));
        // A forced wavelength is carried whatever the error reads.
        assert!(adaptation_refines(&report(0, 2000, 0.0), 0.12));
        // A handful of elements is noise, as it always was.
        assert!(!adaptation_refines(&report(3, 0, 0.9), 0.12));
    }

    /// An estimate owns a copied solution snapshot. Accepted-state maintenance
    /// may continue while the CPU walks that snapshot; only a
    /// topology/generation handoff makes it stale.
    #[test]
    fn a_live_gpu_event_does_not_disown_an_amr_snapshot() {
        let token = TopologyToken {
            document_revision: 4,
            topology_revision: 3,
            mesh_generation: 2,
        };
        let source = AmrIndicatorSource {
            topology: token,
            gpu_generation: 7,
            accepted_step: 120,
        };
        assert!(source.is_current(token, 7));
        assert!(!source.is_current(token, 8));
        assert!(!source.is_current(
            TopologyToken {
                mesh_generation: 3,
                ..token
            },
            7
        ));
    }

    #[test]
    fn the_accuracy_control_starts_on_the_medium_preset() {
        let state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        assert_eq!(
            amr_accuracy_preset_name(state.amr_accuracy_percent),
            "Medium"
        );
        assert!((state.amr_target_accuracy() - 0.12).abs() < 1.0e-12);
        for (percent, name) in AMR_ACCURACY_PRESETS {
            assert_eq!(amr_accuracy_preset_name(percent), name);
        }
        assert_eq!(amr_accuracy_preset_name(9.0), "Custom");
    }

    /// Committing a mesh drops the estimate, and every adaptation commits one,
    /// so a reading that only exists while an estimate is in hand blinks out of
    /// the panel on every cycle and takes everything below it down a line.
    #[test]
    fn the_estimate_reading_holds_its_place_between_estimates() {
        let state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        assert!(state.amr_indicator_result.is_none());
        let line = state.amr_estimate_line();
        assert!(line.contains("target 12%"), "the line read {line:?}");
    }

    /// The size limits and the wavelength live in the fold at the bottom of the
    /// panel now, which is drawn after the adaptation section rather than
    /// inside it. Every one of them still has to drop an estimate in flight, or
    /// a change there is not felt until the next estimate happens to start.
    #[test]
    fn every_adaptation_setting_drops_an_estimate_in_flight() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        const WATCHED: [&str; 5] = [
            "adaptation itself",
            "the accuracy target",
            "elements per wavelength",
            "the smallest element",
            "the largest element",
        ];
        for (step, name) in WATCHED.into_iter().enumerate() {
            let before = state.amr_settings();
            match step {
                0 => state.amr_enabled = !state.amr_enabled,
                1 => state.amr_accuracy_percent = 24.0,
                2 => state.amr_elements_per_wavelength = 9.0,
                3 => state.amr_minimum_edge = 0.01,
                _ => state.amr_maximum_edge = 0.2,
            }
            assert_ne!(state.amr_settings(), before, "{name} is not watched");
        }
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
        // A recorder stamps the solver's clock, which the transfer carries from
        // one generation into the next. `sim_time_offset` turns this
        // generation's step count into that clock; adding it to a time already
        // on it counts the run so far a second time.
        state.sim_time_offset = 4.0;
        state.ingest_probes(&ProbeDisplay {
            generation: 1,
            revision: 1,
            records: vec![sample(4.5), sample(5.0)],
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

    #[test]
    fn the_plot_matrix_addresses_every_cell_exactly_once() {
        let view = ProbeViewState::new(10.0);
        let mut seen = BTreeSet::new();
        for quantity in LineProbeQuantity::ALL {
            for representation in LineProbeRepresentation::ALL {
                let index = quantity.offset() + representation.offset();
                assert!(
                    index < view.line_plots.len(),
                    "{quantity:?} {representation:?}"
                );
                assert!(
                    seen.insert(index),
                    "{quantity:?} {representation:?} collides"
                );
            }
        }
        assert_eq!(seen.len(), view.line_plots.len());

        // A new readout opens on the field's profile and waterfall, the mean
        // flux profile, and the two integrals.
        let enabled = |quantity: LineProbeQuantity, representation: LineProbeRepresentation| {
            view.line_plots[quantity.offset() + representation.offset()]
        };
        assert!(enabled(
            LineProbeQuantity::Field,
            LineProbeRepresentation::Arclength
        ));
        assert!(enabled(
            LineProbeQuantity::Field,
            LineProbeRepresentation::Waterfall
        ));
        assert!(enabled(
            LineProbeQuantity::MeanFlux,
            LineProbeRepresentation::Arclength
        ));
        assert!(enabled(
            LineProbeQuantity::Flux,
            LineProbeRepresentation::Integral
        ));
        assert!(enabled(
            LineProbeQuantity::Energy,
            LineProbeRepresentation::Integral
        ));
        assert_eq!(view.line_plots.iter().filter(|plot| **plot).count(), 5);
    }

    /// The window slider stopped at the preset's nominal rate, which the
    /// recorder does not sample at: it strides the solver's steps, so a coarse
    /// enough step rounds the stride down and the ring reaches back a quarter
    /// less than the slider offered. The top of the slider filled nothing,
    /// however long the run went on.
    #[test]
    fn the_mean_window_stops_at_what_the_trace_can_reach() {
        // The default mesh's time step, which rounds a 120 Hz preset to every
        // step: 149 Hz recorded, not 120.
        let interval = 6.692_306e-3;
        let nominal = 1.0 / 120.0;
        let frames = flux_trace(CURVE_TRACE_FRAMES, interval, 4, |time, _| time as f32);

        let old = CURVE_TRACE_FRAMES as f64 * nominal;
        let (_, old_filled) = Playground::curve_probe_running_mean(&frames, old);
        assert!(
            old_filled < 1.0,
            "the nominal ceiling used to fill: {old_filled}"
        );

        let limit = Playground::curve_mean_window_limit(&frames, nominal);
        assert!(limit < old, "{limit} is not below the nominal {old}");
        let (means, filled) = Playground::curve_probe_running_mean(&frames, limit);
        assert_eq!(filled, 1.0, "the top of the slider never filled");
        assert!(
            means
                .last()
                .unwrap()
                .normal_flux
                .iter()
                .all(|v| v.is_finite()),
            "the newest frame is still waiting at the limit"
        );

        // A dropped readback leaves a gap. The smallest interval is still the
        // stride, so the limit stays reachable rather than following the gap up.
        let mut gapped = frames.clone();
        gapped.remove(CURVE_TRACE_FRAMES / 2);
        let gapped_limit = Playground::curve_mean_window_limit(&gapped, nominal);
        assert!((gapped_limit - limit).abs() < 1.0e-9);
        let (_, gapped_filled) = Playground::curve_probe_running_mean(&gapped, gapped_limit);
        assert_eq!(gapped_filled, 1.0);

        // Nothing recorded yet: the nominal rate is all there is to go on, and
        // the slider still has a range.
        assert_eq!(
            Playground::curve_mean_window_limit(&[], nominal),
            nominal * (CURVE_TRACE_FRAMES - 2) as f64
        );
    }

    /// Frames of `points` samples at `dt`, each value from the frame's time and
    /// the point's index.
    fn flux_trace(
        count: usize,
        dt: f64,
        points: usize,
        value: impl Fn(f64, usize) -> f32,
    ) -> Vec<CurveProbeRecord> {
        (0..count)
            .map(|frame| {
                let time = frame as f64 * dt;
                CurveProbeRecord {
                    probe_id: 1,
                    time,
                    normal_flux: (0..points).map(|point| value(time, point)).collect(),
                    ..Default::default()
                }
            })
            .collect()
    }

    #[test]
    fn the_averaged_flux_row_withholds_a_window_it_cannot_fill() {
        let frames = flux_trace(40, 0.1, 3, |_, point| 2.0 + point as f32);
        let (means, filled) = Playground::curve_probe_running_mean(&frames, 1.0);

        // One row per recorded frame, so every representation keeps the raw
        // series' time alignment.
        assert_eq!(means.len(), frames.len());
        assert!(
            means
                .iter()
                .zip(&frames)
                .all(|(mean, frame)| mean.time == frame.time)
        );

        // Nothing before the record reaches a full second back: a mean over
        // whatever happens to be recorded is not what the row claims to show.
        assert!(
            means[..10]
                .iter()
                .all(|mean| mean.normal_flux.iter().all(|value| value.is_nan()))
        );
        for mean in &means[10..] {
            assert_eq!(mean.normal_flux, vec![2.0, 3.0, 4.0]);
        }
        assert_eq!(filled, 1.0);

        // Reported while it fills, so an empty plot reads as a recorder still
        // filling rather than a silence in the field.
        let (_, partial) = Playground::curve_probe_running_mean(&frames[..5], 1.0);
        assert!((partial - 0.4).abs() < 1.0e-9, "{partial}");
    }

    #[test]
    fn the_averaged_flux_row_cancels_a_standing_wave_and_keeps_a_net_flow() {
        // A standing wave's flux swings symmetrically about zero at twice the
        // driven frequency; a travelling one carries a constant across it. The
        // instantaneous row cannot tell them apart at an arbitrary instant.
        let dt = 1.0 / 120.0;
        let flux = |time: f64, point: usize| {
            let offset = if point == 0 { 0.0 } else { 0.3 };
            (offset + (std::f64::consts::TAU * 5.0 * time).sin()) as f32
        };
        let frames = flux_trace(600, dt, 2, flux);
        let (means, _) = Playground::curve_probe_running_mean(&frames, 1.0);
        let newest = means.last().unwrap();

        // Five whole periods of the oscillation land in the window, to within
        // the one sample the window's edge is ambiguous by.
        assert!(
            newest.normal_flux[0].abs() < 0.02,
            "{}",
            newest.normal_flux[0]
        );
        assert!(
            (newest.normal_flux[1] - 0.3).abs() < 0.02,
            "{}",
            newest.normal_flux[1]
        );
    }

    #[test]
    fn the_averaged_flux_row_skips_gaps_point_by_point() {
        // Portions outside the domain or on a two-trace boundary arrive as NaN,
        // and one bad point must not discard the rest of the path.
        let frames = flux_trace(40, 0.1, 3, |time, point| match point {
            0 => f32::NAN,
            1 if time < 1.5 => f32::NAN,
            _ => 4.0,
        });
        let (means, _) = Playground::curve_probe_running_mean(&frames, 1.0);
        let newest = means.last().unwrap();

        assert!(newest.normal_flux[0].is_nan());
        assert_eq!(newest.normal_flux[1], 4.0);
        assert_eq!(newest.normal_flux[2], 4.0);

        // Halfway through its gap the point averages only what it has, not a
        // zero for every frame it was missing.
        let straddling = means
            .iter()
            .find(|mean| (mean.time - 2.0).abs() < 1.0e-9)
            .unwrap();
        assert_eq!(straddling.normal_flux[1], 4.0);
    }

    #[test]
    fn the_averaged_flux_row_restarts_when_the_sample_layout_changes() {
        // A sampling preset change or a boundary remesh leaves rows of another
        // length in the same trace. Two layouts have no common average.
        let mut frames = flux_trace(20, 0.1, 2, |_, _| 1.0);
        frames.extend(flux_trace(20, 0.1, 4, |_, _| 5.0).into_iter().map(|frame| {
            CurveProbeRecord {
                time: frame.time + 2.0,
                ..frame
            }
        }));
        let (means, filled) = Playground::curve_probe_running_mean(&frames, 1.0);

        assert_eq!(means[19].normal_flux, vec![1.0, 1.0]);
        // The first second after the change is withheld, then the mean is the
        // new layout's alone.
        assert!(means[25].normal_flux.iter().all(|value| value.is_nan()));
        assert_eq!(means[35].normal_flux, vec![5.0; 4]);
        assert_eq!(filled, 1.0);
    }
}
