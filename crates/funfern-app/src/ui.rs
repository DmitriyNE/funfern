#[cfg(not(target_arch = "wasm32"))]
use crate::wave_gpu::forcing_weights;
use crate::wave_gpu::{
    AreaProbeDisplay, AreaProbeInput, AreaProbeRecord, CurveProbeDisplay, CurveProbeInput,
    CurveProbeRecord, FAR_FIELD_CONTOUR_POINTS, FAR_FIELD_DIRECTIONS, FarFieldDisplay,
    FarFieldInput, FarFieldRecord, PointProbeRecord, ProbeDisplay, PulseSettings, WaveDisplay,
    WaveGpuRequest, WaveTransfer,
};
use crate::{
    examples,
    files::{self, FileEvent, SaveKind},
    recovery, sharing,
};
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use bevy_egui::{
    EguiContexts,
    egui::{self, Color32, Pos2, Rect, Stroke},
};
use funfern_app::{
    editor::{
        Acceptance, BoundaryFaceTarget, BoundaryProbeFeature, BoundaryProbeSide,
        BoundaryProbeTarget, Editor, FarFieldSettings, GeometryControl, LoopKind, ProbeDefinition,
        ProbeId, ProbeSamplingPreset, ProbeTarget, SourceSettings,
    },
    persistence::{self, LoadCandidate},
};
use funfern_core::*;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{
    Arc, Mutex,
    mpsc::{self, Receiver, Sender},
};
const TEAL: Color32 = Color32::from_rgb(91, 220, 194);
const SELECT: Color32 = Color32::from_rgb(72, 166, 255);
const RED: Color32 = Color32::from_rgb(255, 106, 123);
const GOLD: Color32 = Color32::from_rgb(248, 196, 112);
const GIZMO_PADDING: f64 = 18.0;
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum CreationRole {
    #[default]
    Hole,
    MaterialInterface,
    InternalBoundary,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum InteractionMode {
    #[default]
    Select,
    DrawPreset {
        role: CreationRole,
    },
    DrawCustom {
        role: CreationRole,
    },
    PlacePulse,
    PlaceProbe,
    PlaceSegmentProbe,
    PlaceAreaDisk,
    PlaceAreaRegion,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusedFeature {
    Loop(ObstacleId),
    Baffle(InternalBoundaryId),
}
const EDITABLE_LOOP_KINDS: [(LoopKind, &str); 2] = [
    (LoopKind::Hole, "Hole"),
    (LoopKind::MaterialInterface, "Material interface"),
];
struct UiNotice {
    text: String,
    created: Instant,
}
#[derive(Clone, Copy)]
enum LoadMode {
    Replace,
    Undoable,
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
impl InspectorPanel {
    const ALL: [Self; 5] = [
        Self::Edit,
        Self::View,
        Self::Simulation,
        Self::Materials,
        Self::Probes,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Edit => "Edit",
            Self::View => "View",
            Self::Simulation => "Simulation",
            Self::Materials => "Materials",
            Self::Probes => "Probes",
        }
    }
}

#[derive(Clone, Default)]
struct ProbeTrace {
    samples: VecDeque<PointProbeRecord>,
    accept_after: f64,
}

#[derive(Clone, Default)]
struct CurveProbeTrace {
    frames: VecDeque<CurveProbeRecord>,
    accept_after: f64,
}

#[derive(Clone, Default)]
struct AreaProbeTrace {
    samples: VecDeque<AreaProbeRecord>,
    accept_after: f64,
}

#[derive(Clone, Default)]
struct FarFieldTrace {
    frames: VecDeque<FarFieldRecord>,
    accept_after: f64,
}

#[derive(Clone, Copy)]
struct BoundaryPathSegment {
    label: BoundaryLabel,
    parameter: [f64; 2],
    period: f64,
    region: RegionId,
    points: [Point2; 2],
}

struct CompiledBoundaryPath {
    segments: Vec<BoundaryPathSegment>,
    length: f64,
    closed: bool,
}
type CurveStencilSamples = Vec<Option<(QuadraticPointStencil, Point2)>>;

#[derive(Clone)]
struct CompiledProbeState {
    generation: u64,
    revision: u64,
    curve_revision: u64,
    area_revision: u64,
    far_field_revision: u64,
    mesh_revision: u64,
    probes: Vec<ProbeDefinition>,
    sample_rate: f64,
    time_step: f64,
    far_field: FarFieldSettings,
    far_field_wave_speed: Option<f64>,
}

enum ProbeDrag {
    Point {
        id: ProbeId,
    },
    SegmentEndpoint {
        id: ProbeId,
        start_endpoint: bool,
    },
    SegmentBody {
        id: ProbeId,
        anchor: Point2,
        start: Point2,
        end: Point2,
    },
    AreaDiskBody {
        id: ProbeId,
        anchor: Point2,
        center: Point2,
        radius: f64,
    },
    AreaDiskRadius {
        id: ProbeId,
        center: Point2,
    },
}

#[derive(Clone, Copy)]
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineProbeQuantity {
    Field,
    Flux,
    Energy,
}

impl LineProbeQuantity {
    const ALL: [Self; 3] = [Self::Field, Self::Flux, Self::Energy];

    const fn label(self) -> &'static str {
        match self {
            Self::Field => "Field",
            Self::Flux => "Normal flux",
            Self::Energy => "Energy",
        }
    }

    const fn color(self) -> Color32 {
        match self {
            Self::Field => SELECT,
            Self::Flux => TEAL,
            Self::Energy => GOLD,
        }
    }

    const fn offset(self) -> usize {
        match self {
            Self::Field => 0,
            Self::Flux => 3,
            Self::Energy => 6,
        }
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

#[derive(Clone)]
struct ProbeViewState {
    live: bool,
    end_time: f64,
    span: f64,
    field: bool,
    velocity: bool,
    energy: bool,
    area_mean_field: bool,
    area_rms_field: bool,
    area_mean_energy: bool,
    area_total_energy: bool,
    line_plots: [bool; 9],
    waterfall_gain: f32,
}

impl ProbeViewState {
    fn new(span: f64) -> Self {
        Self {
            live: true,
            end_time: 0.0,
            span: span.min(2.0),
            field: true,
            velocity: true,
            energy: true,
            area_mean_field: true,
            area_rms_field: false,
            area_mean_energy: false,
            area_total_energy: true,
            // Field vs arclength + waterfall, and the two useful integral traces.
            line_plots: [true, true, false, false, false, true, false, false, true],
            waterfall_gain: 1.0,
        }
    }
}
#[derive(Clone, Copy, Default, PartialEq)]
enum SpanSelectionFilter {
    #[default]
    All,
    Outer,
    Loops,
    Baffles,
}
impl SpanSelectionFilter {
    const fn label(self) -> &'static str {
        match self {
            Self::All => "All spans",
            Self::Outer => "Outer edges",
            Self::Loops => "Loops",
            Self::Baffles => "Baffles",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum AmrQuality {
    Fast,
    #[default]
    Balanced,
    Detailed,
}
impl AmrQuality {
    const ALL: [Self; 3] = [Self::Fast, Self::Balanced, Self::Detailed];

    const fn label(self) -> &'static str {
        match self {
            Self::Fast => "Fast",
            Self::Balanced => "Balanced",
            Self::Detailed => "Detailed",
        }
    }

    const fn relative_tolerance(self) -> f64 {
        match self {
            Self::Fast => 0.10,
            Self::Balanced => 0.06,
            Self::Detailed => 0.035,
        }
    }

    const fn elements_per_wavelength(self) -> f64 {
        match self {
            Self::Fast => 4.0,
            Self::Balanced => 6.0,
            Self::Detailed => 8.0,
        }
    }

    const fn topology_budget(self) -> usize {
        match self {
            Self::Fast => 250,
            Self::Balanced => 600,
            Self::Detailed => 1_200,
        }
    }

    const fn maximum_coarsening_scale(self) -> f64 {
        match self {
            Self::Fast => 2.5,
            Self::Balanced => 2.2,
            Self::Detailed => 1.9,
        }
    }

    const fn collapse_ratio(self) -> f64 {
        0.65
    }
}
#[derive(Clone, Copy)]
enum MarqueeOperation {
    Replace,
    Add,
    Subtract,
}
#[derive(Clone, Copy)]
enum PendingSpanClick {
    Collapse(GeometrySpan),
    Toggle(GeometrySpan),
}
enum Drag {
    Translate {
        anchor: Point2,
        pivot: Point2,
        gizmo_before: Option<Point2>,
        controls: Vec<(GeometryControl, Point2)>,
        moved: bool,
    },
    Rotate {
        pivot: Point2,
        start_angle: f64,
        controls: Vec<(GeometryControl, Point2)>,
        moved: bool,
    },
    Scale {
        pivot: Point2,
        start_control_distance: f64,
        controls: Vec<(GeometryControl, Point2)>,
        moved: bool,
    },
    Pivot {
        start: Option<Point2>,
        offset: Point2,
    },
    Marquee {
        anchor: Pos2,
        current: Pos2,
        base: Vec<GeometrySpan>,
        operation: MarqueeOperation,
    },
}
#[derive(Clone, Copy)]
enum GizmoHit {
    Pivot,
    Rotate,
    Scale,
}
struct Curve {
    id: ObstacleId,
    samples: Vec<Sample>,
}
struct InternalCurve {
    id: InternalBoundaryId,
    samples: Vec<Sample>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GeometrySpan {
    Outer(OuterSide),
    Loop(ObstacleId, usize),
    Baffle(InternalBoundaryId, usize),
}
type ExposedBreakpoints = (Vec<(ObstacleId, usize)>, Vec<(InternalBoundaryId, usize)>);
struct SimulationCandidate {
    mesh: Arc<TriMesh>,
    scene: Scene,
    max_edge: f64,
    low_quality: Vec<bool>,
    operator: Arc<QuadraticWaveOperator>,
    boundary: OuterBoundaryConditions,
    time_step: f64,
    transfer: Option<QuadraticTransferMap>,
    generation: Option<u64>,
    simulation_time: f64,
    exposed_nodes: usize,
    source_region: RegionId,
    adaptation_state: MeshAdaptationState,
    fresh: bool,
}
#[derive(Resource)]
pub struct Playground {
    automated_benchmark: bool,
    editor: Editor,
    interaction_mode: InteractionMode,
    inspector_panel: Option<InspectorPanel>,
    add_geometry_open: bool,
    add_geometry_anchor: Pos2,
    performance_open: bool,
    performance_history: VecDeque<f32>,
    performance_warning_active: bool,
    selected_probe: Option<ProbeId>,
    probe_windows: BTreeSet<ProbeId>,
    probe_views: BTreeMap<ProbeId, ProbeViewState>,
    probe_traces: BTreeMap<ProbeId, ProbeTrace>,
    curve_probe_traces: BTreeMap<ProbeId, CurveProbeTrace>,
    area_probe_traces: BTreeMap<ProbeId, AreaProbeTrace>,
    far_field_trace: FarFieldTrace,
    curve_probe_metrics: BTreeMap<ProbeId, (f64, bool)>,
    probe_status: BTreeMap<ProbeId, String>,
    probe_observed: Vec<ProbeDefinition>,
    probe_compiled: Option<CompiledProbeState>,
    probe_display_readback: u64,
    curve_probe_display_readback: u64,
    area_probe_display_readback: u64,
    far_field_display_readback: u64,
    far_field_status: Option<String>,
    far_field_open: bool,
    far_field_view: ProbeViewState,
    probe_sample_rate: f64,
    probe_history_seconds: f64,
    show_point_probes: bool,
    show_line_probes: bool,
    show_boundary_probes: bool,
    show_area_probes: bool,
    show_far_field_contour: bool,
    probe_drag: Option<ProbeDrag>,
    source_dragging: bool,
    segment_probe_start: Option<Point2>,
    area_probe_center: Option<Point2>,
    probe_name_edit: Option<(ProbeId, String)>,
    creation_role: CreationRole,
    material_selection: MaterialId,
    material_name_edit: Option<(MaterialId, String)>,
    loop_role_edit: Option<(ObstacleId, LoopKind)>,
    region_selection: RegionId,
    selection: Option<(ObstacleId, Option<usize>)>,
    internal_selection: Option<(InternalBoundaryId, Option<usize>)>,
    focused_feature: Option<FocusedFeature>,
    selected_spans: Vec<GeometrySpan>,
    span_selection_filter: SpanSelectionFilter,
    baffle_face: InternalBoundarySide,
    gizmo_pivot: Option<Point2>,
    pending_span_click: Option<PendingSpanClick>,
    custom: Vec<Point2>,
    drag: Option<Drag>,
    transform_translation: Point2,
    transform_rotation_degrees: f64,
    transform_scale: f64,
    snap_to_grid: bool,
    snap_step: f64,
    panning: bool,
    center: Point2,
    scale: f64,
    fit: bool,
    grid: bool,
    polygon: bool,
    handles: bool,
    reference: bool,
    show_boundary_conditions: bool,
    cache_revision: u64,
    cache_scale: f64,
    cache_accepted: Scene,
    draft_curves: Vec<Curve>,
    accepted_curves: Vec<Curve>,
    draft_internal_curves: Vec<InternalCurve>,
    accepted_internal_curves: Vec<InternalCurve>,
    sampling_warning: bool,
    message: String,
    notice: Option<UiNotice>,
    sender: Sender<FileEvent>,
    receiver: Mutex<Receiver<FileEvent>>,
    file_busy: bool,
    load: Option<LoadCandidate>,
    load_mode: LoadMode,
    load_notice: &'static str,
    load_example_simulation: Option<examples::ExampleSimulation>,
    startup_load_checked: bool,
    autosave_observed: funfern_app::editor::Document,
    autosave_due: Option<Instant>,
    share_fragment_active: bool,
    examples_open: bool,
    frame_ms: f32,
    ready: bool,
    logo_texture: Option<egui::TextureHandle>,
    keyboard_captured: bool,
    mesh: Option<Arc<TriMesh>>,
    simulation_candidate: Option<SimulationCandidate>,
    mesh_job: Option<MeshUpdateJob>,
    mesh_adaptation_job: Option<MeshAdaptationJob>,
    mesh_adaptation_automatic: bool,
    mesh_adaptation_state: Option<MeshAdaptationState>,
    mesh_adaptation_report: Option<MeshAdaptationReport>,
    solution_indicator_job: Option<SolutionIndicatorJob>,
    solution_indicator_result: Option<SolutionIndicatorResult>,
    solution_indicator_report: Option<SolutionIndicatorReport>,
    solution_indicator_source: Option<(u64, u64, u64, u64, u64)>,
    amr_enabled: bool,
    amr_quality: AmrQuality,
    amr_minimum_edge: f64,
    amr_maximum_edge: f64,
    amr_settings_revision: u64,
    amr_last_analyzed_step: Option<u64>,
    amr_last_started: Option<Instant>,
    amr_coarsen_streak: u8,
    amr_status: &'static str,
    amr_error: Option<String>,
    amr_work_ms: f64,
    amr_max_slice_ms: f64,
    show_amr_target: bool,
    next_mesh_revision: u64,
    mesh_source: Scene,
    mesh_committed_scene: Scene,
    mesh_committed_max_edge: f64,
    mesh_report: Option<MeshUpdateReport>,
    mesh_attempts: usize,
    mesh_fallbacks: usize,
    mesh_fallback_causes: BTreeMap<MeshUpdateFailureKind, usize>,
    mesh_max_edge: f64,
    mesh_source_max_edge: f64,
    mesh_low_quality: Vec<bool>,
    mesh_error: Option<String>,
    mesh_started: Option<Instant>,
    mesh_build_ms: f64,
    mesh_work_ms: f64,
    mesh_max_slice_ms: f64,
    show_mesh: bool,
    show_mesh_boundary: bool,
    show_materials: bool,
    show_field: bool,
    field_gain: f32,
    wave_mesh: Option<Arc<TriMesh>>,
    wave_operator: Option<Arc<QuadraticWaveOperator>>,
    wave_boundary_committed: OuterBoundaryConditions,
    wave_time_step: f64,
    wave_time_offset: f64,
    wave_running: bool,
    wave_speed: f64,
    wave_accumulator: f64,
    wave_reset_requested: bool,
    fresh_simulation_requested: bool,
    wave_step_requested: bool,
    wave_pending_pulse: Option<Point2>,
    pulse_amplitude: f32,
    pulse_width: f32,
    wave_source: SourceSettings,
    wave_source_dirty: bool,
    wave_prepare_ms: f64,
    wave_active_wall_seconds: f64,
    wave_completed_steps: u64,
    wave_steps_per_second: f64,
    wave_rate_previous_completed: u64,
    wave_dispatches: u64,
    wave_substeps_last: u64,
    wave_energy: Option<f64>,
    wave_energy_step: u64,
    wave_gpu_status: &'static str,
    wave_error: Option<String>,
}
impl Default for Playground {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            automated_benchmark: false,
            editor: Editor::default(),
            interaction_mode: InteractionMode::Select,
            inspector_panel: Some(InspectorPanel::Edit),
            add_geometry_open: false,
            add_geometry_anchor: Pos2::new(520.0, 40.0),
            performance_open: false,
            performance_history: VecDeque::with_capacity(90),
            performance_warning_active: false,
            selected_probe: None,
            probe_windows: BTreeSet::new(),
            probe_views: BTreeMap::new(),
            probe_traces: BTreeMap::new(),
            curve_probe_traces: BTreeMap::new(),
            area_probe_traces: BTreeMap::new(),
            far_field_trace: FarFieldTrace::default(),
            curve_probe_metrics: BTreeMap::new(),
            probe_status: BTreeMap::new(),
            probe_observed: vec![],
            probe_compiled: None,
            probe_display_readback: 0,
            curve_probe_display_readback: 0,
            area_probe_display_readback: 0,
            far_field_display_readback: 0,
            far_field_status: None,
            far_field_open: false,
            far_field_view: ProbeViewState::new(10.0),
            probe_sample_rate: 120.0,
            probe_history_seconds: 10.0,
            show_point_probes: true,
            show_line_probes: true,
            show_boundary_probes: true,
            show_area_probes: true,
            show_far_field_contour: true,
            probe_drag: None,
            source_dragging: false,
            segment_probe_start: None,
            area_probe_center: None,
            probe_name_edit: None,
            creation_role: CreationRole::Hole,
            material_selection: DEFAULT_MATERIAL,
            material_name_edit: None,
            loop_role_edit: None,
            region_selection: BACKGROUND_REGION,
            selection: None,
            internal_selection: None,
            focused_feature: Some(FocusedFeature::Loop(ObstacleId(1))),
            selected_spans: (0..8)
                .map(|span| GeometrySpan::Loop(ObstacleId(1), span))
                .collect(),
            span_selection_filter: SpanSelectionFilter::All,
            baffle_face: InternalBoundarySide::Left,
            gizmo_pivot: None,
            pending_span_click: None,
            custom: vec![],
            drag: None,
            transform_translation: Point2::default(),
            transform_rotation_degrees: 0.0,
            transform_scale: 1.0,
            snap_to_grid: false,
            snap_step: 0.05,
            panning: false,
            center: Point2::default(),
            scale: 300.0,
            fit: true,
            grid: true,
            polygon: true,
            handles: true,
            reference: true,
            show_boundary_conditions: false,
            cache_revision: u64::MAX,
            cache_scale: 0.0,
            cache_accepted: Scene::default(),
            draft_curves: vec![],
            accepted_curves: vec![],
            draft_internal_curves: vec![],
            accepted_internal_curves: vec![],
            sampling_warning: false,
            message: String::new(),
            notice: None,
            sender,
            receiver: Mutex::new(receiver),
            file_busy: false,
            load: None,
            load_mode: LoadMode::Replace,
            load_notice: "Scene loaded; history cleared",
            load_example_simulation: None,
            startup_load_checked: false,
            autosave_observed: funfern_app::editor::Document::default(),
            autosave_due: None,
            share_fragment_active: false,
            examples_open: false,
            frame_ms: 0.0,
            ready: false,
            logo_texture: None,
            keyboard_captured: false,
            mesh: None,
            simulation_candidate: None,
            mesh_job: None,
            mesh_adaptation_job: None,
            mesh_adaptation_automatic: false,
            mesh_adaptation_state: None,
            mesh_adaptation_report: None,
            solution_indicator_job: None,
            solution_indicator_result: None,
            solution_indicator_report: None,
            solution_indicator_source: None,
            amr_enabled: true,
            amr_quality: AmrQuality::Balanced,
            amr_minimum_edge: 0.02,
            amr_maximum_edge: 0.16,
            amr_settings_revision: 1,
            amr_last_analyzed_step: None,
            amr_last_started: None,
            amr_coarsen_streak: 0,
            amr_status: "waiting for solution",
            amr_error: None,
            amr_work_ms: 0.0,
            amr_max_slice_ms: 0.0,
            show_amr_target: false,
            next_mesh_revision: 1,
            mesh_source: Scene::default(),
            mesh_committed_scene: Scene::default(),
            mesh_committed_max_edge: 0.0,
            mesh_report: None,
            mesh_attempts: 0,
            mesh_fallbacks: 0,
            mesh_fallback_causes: BTreeMap::new(),
            mesh_max_edge: 0.08,
            mesh_source_max_edge: 0.0,
            mesh_low_quality: vec![],
            mesh_error: None,
            mesh_started: None,
            mesh_build_ms: 0.0,
            mesh_work_ms: 0.0,
            mesh_max_slice_ms: 0.0,
            show_mesh: false,
            show_mesh_boundary: true,
            show_materials: true,
            show_field: true,
            field_gain: 2.0,
            wave_mesh: None,
            wave_operator: None,
            wave_boundary_committed: OuterBoundaryConditions::default(),
            wave_time_step: 0.0,
            wave_time_offset: 0.0,
            wave_running: true,
            wave_speed: 1.0,
            wave_accumulator: 0.0,
            wave_reset_requested: false,
            fresh_simulation_requested: false,
            wave_step_requested: false,
            wave_pending_pulse: None,
            pulse_amplitude: 0.65,
            pulse_width: 0.06,
            wave_source: SourceSettings::default(),
            wave_source_dirty: false,
            wave_prepare_ms: 0.0,
            wave_active_wall_seconds: 0.0,
            wave_completed_steps: 0,
            wave_steps_per_second: 0.0,
            wave_rate_previous_completed: 0,
            wave_dispatches: 0,
            wave_substeps_last: 0,
            wave_energy: None,
            wave_energy_step: 0,
            wave_gpu_status: "loading",
            wave_error: None,
        }
    }
}
impl Playground {
    fn screen(&self, p: Point2, r: Rect) -> Pos2 {
        Pos2::new(
            r.center().x + ((p.x - self.center.x) * self.scale).clamp(-1e7, 1e7) as f32,
            r.center().y - ((p.y - self.center.y) * self.scale).clamp(-1e7, 1e7) as f32,
        )
    }

    fn wave_display_operator<'a>(
        &'a self,
        display: &WaveDisplay,
    ) -> Option<&'a QuadraticWaveOperator> {
        if let Some(candidate) = &self.simulation_candidate
            && candidate.generation == Some(display.generation)
            && display.current.len() == candidate.operator.degrees_of_freedom()
        {
            return Some(&candidate.operator);
        }
        self.wave_operator
            .as_deref()
            .filter(|operator| display.current.len() == operator.degrees_of_freedom())
    }

    fn reconcile_probe_definitions(&mut self) {
        let current = self.editor.document.probes.clone();
        for probe in &current {
            if let Some(previous) = self
                .probe_observed
                .iter()
                .find(|candidate| candidate.id == probe.id)
                && previous.target != probe.target
                && !matches!(
                    (previous.target, probe.target),
                    (ProbeTarget::Boundary(_), ProbeTarget::Boundary(_))
                )
            {
                self.clear_probe_trace(probe.id);
            }
        }
        let ids = current
            .iter()
            .map(|probe| probe.id)
            .collect::<BTreeSet<_>>();
        self.probe_traces.retain(|id, _| ids.contains(id));
        self.curve_probe_traces.retain(|id, _| ids.contains(id));
        self.area_probe_traces.retain(|id, _| ids.contains(id));
        self.curve_probe_metrics.retain(|id, _| ids.contains(id));
        self.probe_status.retain(|id, _| ids.contains(id));
        self.probe_windows.retain(|id| ids.contains(id));
        self.probe_views.retain(|id, _| ids.contains(id));
        if self.selected_probe.is_some_and(|id| !ids.contains(&id)) {
            self.selected_probe = None;
        }
        self.probe_observed = current;
    }

    fn ingest_probe_samples(&mut self, display: &ProbeDisplay) {
        self.reconcile_probe_definitions();
        let Some(compiled) = &self.probe_compiled else {
            return;
        };
        if display.generation != compiled.generation
            || display.revision != compiled.revision
            || display.readbacks == self.probe_display_readback
        {
            return;
        }
        self.probe_display_readback = display.readbacks;
        for probe in &self.editor.document.probes {
            if !probe.enabled || self.probe_status.contains_key(&probe.id) {
                continue;
            }
            let incoming = display
                .records
                .iter()
                .filter(|record| record.probe_id == probe.id.0)
                .copied()
                .collect::<Vec<_>>();
            if incoming.is_empty() {
                continue;
            }
            let trace = self.probe_traces.entry(probe.id).or_default();
            let last = trace
                .samples
                .back()
                .map(|sample| sample.time)
                .unwrap_or(trace.accept_after);
            let fresh = incoming
                .into_iter()
                .filter(|sample| sample.time > last + 1.0e-7)
                .collect::<Vec<_>>();
            trace.samples.extend(fresh);
            if let Some(newest) = trace.samples.back().map(|sample| sample.time) {
                let oldest = newest - self.probe_history_seconds;
                while trace
                    .samples
                    .front()
                    .is_some_and(|sample| sample.time < oldest)
                {
                    trace.samples.pop_front();
                }
            }
        }
    }

    fn ingest_curve_probe_samples(&mut self, display: &CurveProbeDisplay) {
        self.reconcile_probe_definitions();
        let Some(compiled) = &self.probe_compiled else {
            return;
        };
        if display.generation != compiled.generation
            || display.revision != compiled.curve_revision
            || display.readbacks == self.curve_probe_display_readback
        {
            return;
        }
        self.curve_probe_display_readback = display.readbacks;
        for probe in &self.editor.document.probes {
            if !probe.enabled
                || !matches!(
                    probe.target,
                    ProbeTarget::Segment { .. } | ProbeTarget::Boundary(_)
                )
            {
                continue;
            }
            let incoming = display
                .records
                .iter()
                .filter(|record| record.probe_id == probe.id.0)
                .cloned()
                .collect::<Vec<_>>();
            if incoming.is_empty() {
                continue;
            }
            let trace = self.curve_probe_traces.entry(probe.id).or_default();
            let last = trace
                .frames
                .back()
                .map(|sample| sample.time)
                .unwrap_or(trace.accept_after);
            trace.frames.extend(
                incoming
                    .into_iter()
                    .filter(|sample| sample.time > last + 1.0e-7),
            );
            if let Some(newest) = trace.frames.back().map(|sample| sample.time) {
                let oldest = newest - self.probe_history_seconds;
                while trace
                    .frames
                    .front()
                    .is_some_and(|sample| sample.time < oldest)
                {
                    trace.frames.pop_front();
                }
            }
        }
    }

    fn ingest_area_probe_samples(&mut self, display: &AreaProbeDisplay) {
        self.reconcile_probe_definitions();
        let Some(compiled) = &self.probe_compiled else {
            return;
        };
        if display.generation != compiled.generation
            || display.revision != compiled.area_revision
            || display.readbacks == self.area_probe_display_readback
        {
            return;
        }
        self.area_probe_display_readback = display.readbacks;
        for probe in &self.editor.document.probes {
            if !probe.enabled
                || !matches!(
                    probe.target,
                    ProbeTarget::AreaDisk { .. } | ProbeTarget::AreaRegion { .. }
                )
                || self.probe_status.contains_key(&probe.id)
            {
                continue;
            }
            let trace = self.area_probe_traces.entry(probe.id).or_default();
            let last = trace
                .samples
                .back()
                .map_or(trace.accept_after, |sample| sample.time);
            trace.samples.extend(
                display
                    .records
                    .iter()
                    .filter(|sample| sample.probe_id == probe.id.0 && sample.time > last + 1.0e-7)
                    .copied(),
            );
            if let Some(newest) = trace.samples.back().map(|sample| sample.time) {
                let oldest = newest - self.probe_history_seconds;
                while trace
                    .samples
                    .front()
                    .is_some_and(|sample| sample.time < oldest)
                {
                    trace.samples.pop_front();
                }
            }
        }
    }

    fn boundary_probe_path(
        scene: &Scene,
        target: BoundaryProbeTarget,
    ) -> Option<CompiledBoundaryPath> {
        const PIECES_PER_SPAN: usize = 16;
        let mut segments = Vec::new();
        let (total, spans) = match target.feature {
            BoundaryProbeFeature::Outer => (4, target.spans(4)),
            BoundaryProbeFeature::Loop(id) => {
                let loop_ = scene.obstacles.iter().find(|loop_| loop_.id == id)?;
                let total = loop_.spline.intervals().len();
                (total, target.spans(total))
            }
            BoundaryProbeFeature::Baffle(id) => {
                let boundary = scene
                    .internal_boundaries
                    .iter()
                    .find(|boundary| boundary.id == id)?;
                let total = boundary.spline.intervals().len();
                (total, target.spans(total))
            }
        };
        if spans.is_empty() || spans.iter().any(|span| *span >= total) {
            return None;
        }
        match target.feature {
            BoundaryProbeFeature::Outer => {
                let corners = [
                    Point2::new(-1.0, -1.0),
                    Point2::new(1.0, -1.0),
                    Point2::new(1.0, 1.0),
                    Point2::new(-1.0, 1.0),
                ];
                for span in spans {
                    segments.push(BoundaryPathSegment {
                        label: BoundaryLabel::Outer(OuterSide::ALL[span]),
                        parameter: [0.0, 1.0],
                        period: 1.0,
                        region: BACKGROUND_REGION,
                        points: [corners[span], corners[(span + 1) % 4]],
                    });
                }
            }
            BoundaryProbeFeature::Loop(id) => {
                let loop_ = scene.obstacles.iter().find(|loop_| loop_.id == id)?;
                let (label, region) = match (loop_.role, target.side) {
                    (LoopRole::Hole { exterior }, BoundaryProbeSide::Domain) => {
                        (BoundaryLabel::Obstacle(id), exterior)
                    }
                    (LoopRole::MaterialInterface { exterior, .. }, BoundaryProbeSide::Exterior) => {
                        (BoundaryLabel::MaterialInterface(id), exterior)
                    }
                    (LoopRole::MaterialInterface { interior, .. }, BoundaryProbeSide::Interior) => {
                        (BoundaryLabel::MaterialInterface(id), interior)
                    }
                    (LoopRole::Wall { exterior, .. }, BoundaryProbeSide::Exterior) => (
                        BoundaryLabel::Wall {
                            loop_id: id,
                            side: BoundarySide::Exterior,
                        },
                        exterior,
                    ),
                    (LoopRole::Wall { interior, .. }, BoundaryProbeSide::Interior) => (
                        BoundaryLabel::Wall {
                            loop_id: id,
                            side: BoundarySide::Interior,
                        },
                        interior,
                    ),
                    _ => return None,
                };
                for span in spans {
                    let bounds = loop_.spline.span_bounds(span)?;
                    for piece in 0..PIECES_PER_SPAN {
                        let a = piece as f64 / PIECES_PER_SPAN as f64;
                        let b = (piece + 1) as f64 / PIECES_PER_SPAN as f64;
                        let parameters = [
                            bounds[0] + (bounds[1] - bounds[0]) * a,
                            bounds[0] + (bounds[1] - bounds[0]) * b,
                        ];
                        segments.push(BoundaryPathSegment {
                            label,
                            parameter: parameters,
                            period: loop_.spline.period(),
                            region,
                            points: parameters.map(|parameter| loop_.spline.evaluate(parameter)),
                        });
                    }
                }
            }
            BoundaryProbeFeature::Baffle(id) => {
                let boundary = scene
                    .internal_boundaries
                    .iter()
                    .find(|boundary| boundary.id == id)?;
                let side = match target.side {
                    BoundaryProbeSide::Left => InternalBoundarySide::Left,
                    BoundaryProbeSide::Right => InternalBoundarySide::Right,
                    _ => return None,
                };
                for span in spans {
                    let start = boundary.spline.breakpoint(span)?;
                    let end = boundary.spline.breakpoint(span + 1)?;
                    for piece in 0..PIECES_PER_SPAN {
                        let a = piece as f64 / PIECES_PER_SPAN as f64;
                        let b = (piece + 1) as f64 / PIECES_PER_SPAN as f64;
                        let parameters = [start + (end - start) * a, start + (end - start) * b];
                        segments.push(BoundaryPathSegment {
                            label: BoundaryLabel::InternalBoundary { id, side },
                            parameter: parameters,
                            period: boundary.spline.period(),
                            region: boundary.region,
                            points: parameters.map(|parameter| boundary.spline.evaluate(parameter)),
                        });
                    }
                }
            }
        }
        if target.reversed {
            segments.reverse();
            for segment in &mut segments {
                segment.parameter.swap(0, 1);
                segment.points.swap(0, 1);
            }
        }
        let length = segments
            .iter()
            .map(|segment| (segment.points[1] - segment.points[0]).norm())
            .sum::<f64>();
        (length.is_finite() && length > 0.0).then_some(CompiledBoundaryPath {
            segments,
            length,
            closed: target.whole
                && matches!(
                    target.feature,
                    BoundaryProbeFeature::Outer | BoundaryProbeFeature::Loop(_)
                ),
        })
    }

    fn compile_boundary_probe(
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        scene: &Scene,
        target: BoundaryProbeTarget,
    ) -> Option<(CurveStencilSamples, f64, bool)> {
        let path = Self::boundary_probe_path(scene, target)?;
        let count = target.preset.spatial_points();
        let mut cumulative = Vec::with_capacity(path.segments.len() + 1);
        cumulative.push(0.0);
        for segment in &path.segments {
            let next = cumulative.last().copied().unwrap()
                + (segment.points[1] - segment.points[0]).norm();
            cumulative.push(next);
        }
        let samples = (0..count)
            .map(|index| {
                let fraction = if path.closed {
                    index as f64 / count as f64
                } else {
                    index as f64 / (count - 1) as f64
                };
                let distance = fraction * path.length;
                let segment_index = cumulative
                    .partition_point(|value| *value <= distance)
                    .saturating_sub(1)
                    .min(path.segments.len() - 1);
                let segment = path.segments[segment_index];
                let segment_length = cumulative[segment_index + 1] - cumulative[segment_index];
                let local = if segment_length > 0.0 {
                    (distance - cumulative[segment_index]) / segment_length
                } else {
                    0.0
                };
                let parameter = segment.parameter[0]
                    + (segment.parameter[1] - segment.parameter[0]) * local.clamp(0.0, 1.0);
                QuadraticBoundaryStencil::build(
                    mesh,
                    operator,
                    scene,
                    segment.label,
                    parameter,
                    segment.period,
                    segment.region,
                )
                .ok()
                .map(|sample| (sample.stencil, sample.outward_normal))
            })
            .collect();
        Some((samples, path.length, path.closed))
    }

    fn far_field_contour(inset: f64) -> Vec<(Point2, Point2)> {
        let half_extent = 1.0 - inset;
        let points_per_side = FAR_FIELD_CONTOUR_POINTS / 4;
        let mut samples = Vec::with_capacity(FAR_FIELD_CONTOUR_POINTS);
        for side in 0..4 {
            for index in 0..points_per_side {
                let fraction = (index as f64 + 0.5) / points_per_side as f64;
                let along = -half_extent + 2.0 * half_extent * fraction;
                let sample = match side {
                    0 => (Point2::new(along, -half_extent), Point2::new(0.0, -1.0)),
                    1 => (Point2::new(half_extent, along), Point2::new(1.0, 0.0)),
                    2 => (Point2::new(-along, half_extent), Point2::new(0.0, 1.0)),
                    _ => (Point2::new(-half_extent, -along), Point2::new(-1.0, 0.0)),
                };
                samples.push(sample);
            }
        }
        samples
    }

    fn compile_far_field(
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        scene: &Scene,
        settings: FarFieldSettings,
    ) -> Result<FarFieldInput, String> {
        if !settings.valid() {
            return Err("Far-field inset must lie between 0 and 1".into());
        }
        let half_extent = 1.0 - settings.inset;
        let enclosed = |point: Point2| {
            point.x.abs() < half_extent - 1.0e-6 && point.y.abs() < half_extent - 1.0e-6
        };
        if scene
            .obstacles
            .iter()
            .flat_map(|loop_| loop_.spline.controls())
            .chain(
                scene
                    .internal_boundaries
                    .iter()
                    .flat_map(|boundary| boundary.spline.controls()),
            )
            .any(|point| !enclosed(*point))
        {
            return Err("Decrease the inset: the contour must enclose all geometry".into());
        }
        let material = scene
            .region_material(BACKGROUND_REGION)
            .ok_or("The background material is missing")?;
        if material.damping.abs() > 1.0e-12 {
            return Err("Far-field projection requires a lossless background material".into());
        }
        let wave_speed = (material.stiffness / material.mass_density).sqrt();
        if !wave_speed.is_finite() || wave_speed <= 0.0 {
            return Err("The background wave speed is invalid".into());
        }
        let samples = Self::far_field_contour(settings.inset)
            .into_iter()
            .map(|(position, normal)| {
                let stencil = QuadraticPointStencil::build(mesh, operator, scene, position)
                    .map_err(|error| format!("Far-field contour is unavailable: {error}"))?;
                if stencil.region != BACKGROUND_REGION {
                    return Err("Far-field contour must stay in the background material".into());
                }
                Ok((stencil, position, normal))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(FarFieldInput {
            samples,
            wave_speed,
            sample_spacing: 8.0 * half_extent / FAR_FIELD_CONTOUR_POINTS as f64,
            delay_margin: std::f64::consts::SQRT_2 * half_extent / wave_speed,
        })
    }

    fn ingest_far_field_samples(&mut self, display: &FarFieldDisplay) {
        let Some(compiled) = &self.probe_compiled else {
            return;
        };
        if display.generation != compiled.generation
            || display.revision != compiled.far_field_revision
            || display.readbacks == self.far_field_display_readback
        {
            return;
        }
        self.far_field_display_readback = display.readbacks;
        let last = self
            .far_field_trace
            .frames
            .back()
            .map_or(self.far_field_trace.accept_after, |frame| frame.time);
        self.far_field_trace.frames.extend(
            display
                .records
                .iter()
                .filter(|frame| frame.time > last + 1.0e-7)
                .cloned(),
        );
        if let Some(newest) = self.far_field_trace.frames.back().map(|frame| frame.time) {
            let oldest = newest - self.probe_history_seconds;
            while self
                .far_field_trace
                .frames
                .front()
                .is_some_and(|frame| frame.time < oldest)
            {
                self.far_field_trace.frames.pop_front();
            }
        }
    }

    fn refresh_probe_gpu(
        &mut self,
        request: &mut WaveGpuRequest,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
    ) {
        if self.simulation_candidate.is_some() || !request.ready() {
            return;
        }
        if self.mesh_committed_scene != self.editor.document.accepted {
            return;
        }
        let (Some(mesh), Some(operator)) = (&self.wave_mesh, &self.wave_operator) else {
            return;
        };
        let probes = self.editor.document.probes.clone();
        let far_field_settings = self.editor.document.far_field;
        let unchanged = self.probe_compiled.as_ref().is_some_and(|compiled| {
            compiled.generation == request.generation()
                && compiled.mesh_revision == mesh.mesh_revision
                && compiled.probes == probes
                && compiled.sample_rate == self.probe_sample_rate
                && compiled.time_step == self.wave_time_step
                && compiled.far_field == far_field_settings
        });
        if unchanged {
            return;
        }
        self.probe_status.clear();
        let point_probes = probes
            .iter()
            .filter_map(|probe| match probe.target {
                ProbeTarget::Point(position) => {
                    let stencil = if probe.enabled {
                        QuadraticPointStencil::build(
                            mesh,
                            operator,
                            &self.mesh_committed_scene,
                            position,
                        )
                        .map_err(|error| error.to_string())
                        .ok()
                    } else {
                        None
                    };
                    if probe.enabled && stencil.is_none() {
                        let reason = QuadraticPointStencil::build(
                            mesh,
                            operator,
                            &self.mesh_committed_scene,
                            position,
                        )
                        .err()
                        .map_or_else(|| "Probe is inactive".into(), |error| error.to_string());
                        self.probe_status.insert(probe.id, reason);
                    }
                    Some((probe.id.0, stencil))
                }
                ProbeTarget::Segment { .. }
                | ProbeTarget::Boundary(_)
                | ProbeTarget::AreaDisk { .. }
                | ProbeTarget::AreaRegion { .. } => None,
            })
            .collect::<Vec<_>>();
        let mut curve_metrics = BTreeMap::new();
        let curve_probes = probes
            .iter()
            .filter_map(|probe| match probe.target {
                ProbeTarget::Point(_)
                | ProbeTarget::AreaDisk { .. }
                | ProbeTarget::AreaRegion { .. } => None,
                ProbeTarget::Segment { start, end, preset } => {
                    let count = preset.spatial_points();
                    let delta = end - start;
                    let length = delta.norm();
                    let normal = Point2::new(-delta.y / length, delta.x / length);
                    let samples = (0..count)
                        .map(|index| {
                            let fraction = index as f64 / (count - 1) as f64;
                            let position = start + delta * fraction;
                            probe
                                .enabled
                                .then(|| {
                                    QuadraticPointStencil::build(
                                        mesh,
                                        operator,
                                        &self.mesh_committed_scene,
                                        position,
                                    )
                                    .ok()
                                })
                                .flatten()
                                .map(|stencil| (stencil, normal))
                        })
                        .collect::<Vec<_>>();
                    if probe.enabled && samples.iter().all(Option::is_none) {
                        self.probe_status
                            .insert(probe.id, "Line probe does not intersect the mesh".into());
                    }
                    curve_metrics.insert(probe.id, (length, false));
                    Some(CurveProbeInput {
                        id: probe.id.0,
                        sample_rate: preset.sample_rate(),
                        samples,
                    })
                }
                ProbeTarget::Boundary(target) => {
                    let compiled = Self::compile_boundary_probe(
                        mesh,
                        operator,
                        &self.mesh_committed_scene,
                        target,
                    );
                    let (samples, length, closed) = compiled.unwrap_or_else(|| {
                        (vec![None; target.preset.spatial_points()], 0.0, false)
                    });
                    if probe.enabled && samples.iter().all(Option::is_none) {
                        self.probe_status.insert(
                            probe.id,
                            "Boundary probe is unavailable on the committed mesh".into(),
                        );
                    }
                    curve_metrics.insert(probe.id, (length, closed));
                    Some(CurveProbeInput {
                        id: probe.id.0,
                        sample_rate: target.preset.sample_rate(),
                        samples: if probe.enabled {
                            samples
                        } else {
                            vec![None; target.preset.spatial_points()]
                        },
                    })
                }
            })
            .collect::<Vec<_>>();
        self.curve_probe_metrics = curve_metrics;
        let area_probes = probes
            .iter()
            .filter_map(|probe| {
                let shape = match probe.target {
                    ProbeTarget::AreaDisk { center, radius } => {
                        AreaProbeShape::Disk { center, radius }
                    }
                    ProbeTarget::AreaRegion { region } => AreaProbeShape::Region(region),
                    ProbeTarget::Point(_)
                    | ProbeTarget::Segment { .. }
                    | ProbeTarget::Boundary(_) => return None,
                };
                let stencil = probe.enabled.then(|| {
                    QuadraticAreaStencil::build(mesh, operator, &self.mesh_committed_scene, shape)
                });
                let stencil = match stencil {
                    Some(Ok(stencil)) => Some(stencil),
                    Some(Err(error)) => {
                        self.probe_status.insert(probe.id, error.to_string());
                        None
                    }
                    None => None,
                };
                Some(AreaProbeInput {
                    id: probe.id.0,
                    stencil,
                })
            })
            .collect::<Vec<_>>();
        self.far_field_status = None;
        let far_field = if far_field_settings.enabled {
            match Self::compile_far_field(
                mesh,
                operator,
                &self.mesh_committed_scene,
                far_field_settings,
            ) {
                Ok(input) => Some(input),
                Err(error) => {
                    self.far_field_status = Some(error);
                    None
                }
            }
        } else {
            None
        };
        let far_field_wave_speed = far_field.as_ref().map(|input| input.wave_speed);
        let preserve_far_field = self.probe_compiled.as_ref().is_some_and(|compiled| {
            compiled.far_field == far_field_settings
                && compiled.far_field_wave_speed == far_field_wave_speed
        });
        if !preserve_far_field {
            self.far_field_trace = FarFieldTrace::default();
            self.far_field_view = ProbeViewState::new(self.probe_history_seconds);
        }
        let probe_result = request
            .update_point_probes(
                assets,
                commands,
                &point_probes,
                self.probe_sample_rate,
                self.wave_time_step,
            )
            .and_then(|()| {
                request.update_curve_probes(assets, commands, &curve_probes, self.wave_time_step)
            })
            .and_then(|()| {
                request.update_area_probes(
                    assets,
                    commands,
                    &area_probes,
                    self.probe_sample_rate.min(120.0),
                    self.wave_time_step,
                )
            });
        if let Err(error) = probe_result {
            for probe in &probes {
                self.probe_status.insert(probe.id, error.clone());
            }
        }
        if let Err(error) =
            request.update_far_field(assets, commands, far_field.as_ref(), self.wave_time_step)
        {
            self.far_field_status = Some(error);
        }
        self.probe_display_readback = 0;
        self.curve_probe_display_readback = 0;
        self.area_probe_display_readback = 0;
        self.far_field_display_readback = 0;
        self.probe_compiled = Some(CompiledProbeState {
            generation: request.generation(),
            revision: request.probe_revision(),
            curve_revision: request.curve_probe_revision(),
            area_revision: request.area_probe_revision(),
            far_field_revision: request.far_field_revision(),
            mesh_revision: mesh.mesh_revision,
            probes,
            sample_rate: self.probe_sample_rate,
            time_step: self.wave_time_step,
            far_field: far_field_settings,
            far_field_wave_speed,
        });
    }

    fn clear_all_probe_traces(&mut self) {
        self.probe_traces.clear();
        self.curve_probe_traces.clear();
        self.area_probe_traces.clear();
        self.far_field_trace = FarFieldTrace::default();
        self.probe_display_readback = 0;
        self.curve_probe_display_readback = 0;
        self.area_probe_display_readback = 0;
        self.far_field_display_readback = 0;
    }
    fn world(&self, p: Pos2, r: Rect) -> Point2 {
        Point2::new(
            self.center.x + (p.x - r.center().x) as f64 / self.scale,
            self.center.y - (p.y - r.center().y) as f64 / self.scale,
        )
    }
    fn clear_transient(&mut self) {
        self.selection = None;
        self.internal_selection = None;
        self.focused_feature = None;
        self.selected_spans.clear();
        self.selected_probe = None;
        self.probe_drag = None;
        self.source_dragging = false;
        self.segment_probe_start = None;
        self.area_probe_center = None;
        self.probe_name_edit = None;
        self.gizmo_pivot = None;
        self.pending_span_click = None;
        self.region_selection = BACKGROUND_REGION;
        self.drag = None;
        self.panning = false;
        self.custom.clear();
        self.interaction_mode = InteractionMode::Select;
    }
    fn error<T>(&mut self, result: Result<T, String>) -> Option<T> {
        match result {
            Ok(v) => {
                self.message.clear();
                Some(v)
            }
            Err(e) => {
                self.message = e;
                None
            }
        }
    }

    fn notify(&mut self, text: impl Into<String>) {
        self.notice = Some(UiNotice {
            text: text.into(),
            created: Instant::now(),
        });
    }

    fn expire_notice(&mut self) {
        if self
            .notice
            .as_ref()
            .is_some_and(|notice| notice.created.elapsed().as_secs_f32() >= 4.0)
        {
            self.notice = None;
        }
    }

    fn select_control(&mut self, control: GeometryControl) {
        self.selected_probe = None;
        self.selected_spans.clear();
        self.gizmo_pivot = None;
        self.pending_span_click = None;
        self.loop_role_edit = None;
        self.focus_control(Some(control));
    }

    fn focus_control(&mut self, control: Option<GeometryControl>) {
        match control {
            None => {
                self.selection = None;
                self.internal_selection = None;
                self.focused_feature = None;
            }
            Some(GeometryControl::Loop(id, index)) => {
                self.selection = Some((id, Some(index)));
                self.internal_selection = None;
                self.focused_feature = Some(FocusedFeature::Loop(id));
            }
            Some(GeometryControl::Baffle(id, index)) => {
                self.selection = None;
                self.internal_selection = Some((id, Some(index)));
                self.focused_feature = Some(FocusedFeature::Baffle(id));
            }
        }
    }

    fn select_loop(&mut self, id: ObstacleId) {
        let spans = self
            .editor
            .obstacle(id)
            .map(|obstacle| {
                (0..obstacle.spline.intervals().len())
                    .map(|span| GeometrySpan::Loop(id, span))
                    .collect()
            })
            .unwrap_or_default();
        self.set_span_selection(spans);
    }

    fn select_baffle(&mut self, id: InternalBoundaryId) {
        let spans = self
            .editor
            .internal_boundary(id)
            .map(|boundary| {
                (0..boundary.spline.intervals().len())
                    .map(|span| GeometrySpan::Baffle(id, span))
                    .collect()
            })
            .unwrap_or_default();
        self.set_span_selection(spans);
    }

    fn set_span_selection(&mut self, spans: Vec<GeometrySpan>) {
        self.selected_probe = None;
        self.selected_spans.clear();
        let mut seen = BTreeSet::new();
        for span in spans {
            if self.span_valid(span) && seen.insert(geometry_span_key(span)) {
                self.selected_spans.push(span);
            }
        }
        self.gizmo_pivot = None;
        self.pending_span_click = None;
        self.loop_role_edit = None;
        self.selection = None;
        self.internal_selection = None;
        self.focused_feature = self.complete_selected_feature();
    }

    fn complete_selected_feature(&self) -> Option<FocusedFeature> {
        let first = *self.selected_spans.first()?;
        match first {
            GeometrySpan::Loop(id, _) => {
                let obstacle = self.editor.obstacle(id)?;
                (self.selected_spans.len() == obstacle.spline.intervals().len()
                    && self.selected_spans.iter().all(
                        |span| matches!(span, GeometrySpan::Loop(candidate, _) if *candidate == id),
                    ))
                .then_some(FocusedFeature::Loop(id))
            }
            GeometrySpan::Baffle(id, _) => {
                let boundary = self.editor.internal_boundary(id)?;
                (self.selected_spans.len() == boundary.spline.intervals().len()
                    && self
                        .selected_spans
                        .iter()
                        .all(|span| matches!(span, GeometrySpan::Baffle(candidate, _) if *candidate == id)))
                .then_some(FocusedFeature::Baffle(id))
            }
            GeometrySpan::Outer(_) => None,
        }
    }

    fn selection_summary(&self) -> Option<String> {
        if let Some((id, Some(index))) = self.selection {
            return Some(format!(
                "Control {} on {} {}",
                index + 1,
                self.editor.obstacle(id)?.role.label(),
                id.0
            ));
        }
        if let Some((id, Some(index))) = self.internal_selection {
            return Some(format!("Control {} on Baffle {}", index + 1, id.0));
        }
        if self.selected_spans.is_empty() {
            return None;
        }
        if let Some(feature) = self.complete_selected_feature() {
            return Some(match feature {
                FocusedFeature::Loop(id) => format!(
                    "{} spans on {} {}",
                    self.selected_spans.len(),
                    self.editor.obstacle(id)?.role.label(),
                    id.0
                ),
                FocusedFeature::Baffle(id) => {
                    format!("{} spans on Baffle {}", self.selected_spans.len(), id.0)
                }
            });
        }
        let mut boundaries = BTreeSet::new();
        for span in &self.selected_spans {
            boundaries.insert(match span {
                GeometrySpan::Outer(_) => (0_u8, 0_u64),
                GeometrySpan::Loop(id, _) => (1, id.0),
                GeometrySpan::Baffle(id, _) => (2, id.0),
            });
        }
        Some(format!(
            "{} spans across {} boundaries",
            self.selected_spans.len(),
            boundaries.len()
        ))
    }

    fn span_valid(&self, span: GeometrySpan) -> bool {
        match span {
            GeometrySpan::Outer(_) => true,
            GeometrySpan::Loop(id, span) => self
                .editor
                .obstacle(id)
                .is_some_and(|obstacle| span < obstacle.spline.intervals().len()),
            GeometrySpan::Baffle(id, span) => self
                .editor
                .internal_boundary(id)
                .is_some_and(|boundary| span < boundary.spline.intervals().len()),
        }
    }

    fn span_matches_filter(&self, span: GeometrySpan) -> bool {
        matches!(self.span_selection_filter, SpanSelectionFilter::All)
            || matches!(
                (self.span_selection_filter, span),
                (SpanSelectionFilter::Outer, GeometrySpan::Outer(_))
                    | (SpanSelectionFilter::Loops, GeometrySpan::Loop(_, _))
                    | (SpanSelectionFilter::Baffles, GeometrySpan::Baffle(_, _))
            )
    }

    fn filtered_spans(&self) -> Vec<GeometrySpan> {
        let mut spans = Vec::new();
        for side in OuterSide::ALL {
            let span = GeometrySpan::Outer(side);
            if self.span_matches_filter(span) {
                spans.push(span);
            }
        }
        for obstacle in &self.editor.document.draft.obstacles {
            for index in 0..obstacle.spline.intervals().len() {
                let span = GeometrySpan::Loop(obstacle.id, index);
                if self.span_matches_filter(span) {
                    spans.push(span);
                }
            }
        }
        for boundary in &self.editor.document.draft.internal_boundaries {
            for index in 0..boundary.spline.intervals().len() {
                let span = GeometrySpan::Baffle(boundary.id, index);
                if self.span_matches_filter(span) {
                    spans.push(span);
                }
            }
        }
        spans
    }

    fn select_filtered(&mut self) {
        let spans = self.filtered_spans();
        self.set_span_selection(spans);
    }

    fn invert_filtered_selection(&mut self) {
        let filtered = self.filtered_spans();
        let selected = self
            .selected_spans
            .iter()
            .copied()
            .map(geometry_span_key)
            .collect::<BTreeSet<_>>();
        let mut spans = self
            .selected_spans
            .iter()
            .copied()
            .filter(|span| !self.span_matches_filter(*span))
            .collect::<Vec<_>>();
        spans.extend(
            filtered
                .into_iter()
                .filter(|span| !selected.contains(&geometry_span_key(*span))),
        );
        self.set_span_selection(spans);
    }

    fn apply_marquee_selection(
        &mut self,
        base: &[GeometrySpan],
        hits: Vec<GeometrySpan>,
        operation: MarqueeOperation,
    ) {
        let mut spans = match operation {
            MarqueeOperation::Replace => Vec::new(),
            MarqueeOperation::Add | MarqueeOperation::Subtract => base.to_vec(),
        };
        match operation {
            MarqueeOperation::Replace | MarqueeOperation::Add => {
                let mut selected = spans
                    .iter()
                    .copied()
                    .map(geometry_span_key)
                    .collect::<BTreeSet<_>>();
                for hit in hits {
                    if selected.insert(geometry_span_key(hit)) {
                        spans.push(hit);
                    }
                }
            }
            MarqueeOperation::Subtract => {
                let hits = hits
                    .into_iter()
                    .map(geometry_span_key)
                    .collect::<BTreeSet<_>>();
                spans.retain(|span| !hits.contains(&geometry_span_key(*span)));
            }
        }
        self.set_span_selection(spans);
    }

    fn spans_in_marquee(&self, marquee: Rect, viewport: Rect) -> Vec<GeometrySpan> {
        let mut spans = Vec::new();
        let mut seen = BTreeSet::new();
        let mut add = |span| {
            if self.span_matches_filter(span) && seen.insert(geometry_span_key(span)) {
                spans.push(span);
            }
        };
        for side in OuterSide::ALL {
            let [a, b] = outer_side_points(side);
            if segment_intersects_rect(self.screen(a, viewport), self.screen(b, viewport), marquee)
            {
                add(GeometrySpan::Outer(side));
            }
        }
        for curve in &self.draft_curves {
            let Some(obstacle) = self.editor.obstacle(curve.id) else {
                continue;
            };
            for segment in curve.samples.windows(2) {
                if segment_intersects_rect(
                    self.screen(segment[0].point, viewport),
                    self.screen(segment[1].point, viewport),
                    marquee,
                ) && let Some(span) = obstacle
                    .spline
                    .span_index(0.5 * (segment[0].t + segment[1].t))
                {
                    add(GeometrySpan::Loop(curve.id, span));
                }
            }
        }
        for curve in &self.draft_internal_curves {
            let Some(boundary) = self.editor.internal_boundary(curve.id) else {
                continue;
            };
            for segment in curve.samples.windows(2) {
                if segment_intersects_rect(
                    self.screen(segment[0].point, viewport),
                    self.screen(segment[1].point, viewport),
                    marquee,
                ) && let Some(span) = boundary
                    .spline
                    .span_index(0.5 * (segment[0].t + segment[1].t))
                {
                    add(GeometrySpan::Baffle(curve.id, span));
                }
            }
        }
        spans
    }

    fn toggle_span(&mut self, span: GeometrySpan) {
        let mut spans = self.selected_spans.clone();
        if let Some(index) = spans.iter().position(|candidate| *candidate == span) {
            spans.remove(index);
        } else {
            spans.push(span);
        }
        self.set_span_selection(spans);
    }

    fn toggle_curve(&mut self, spans: Vec<GeometrySpan>) {
        let all_selected = spans.iter().all(|span| self.selected_spans.contains(span));
        let mut selection = self.selected_spans.clone();
        if all_selected {
            selection.retain(|span| !spans.contains(span));
        } else {
            for span in spans {
                if !selection.contains(&span) {
                    selection.push(span);
                }
            }
        }
        self.set_span_selection(selection);
    }

    fn reconcile_selection(&mut self) {
        let spans = self
            .selected_spans
            .iter()
            .copied()
            .filter(|span| self.span_valid(*span))
            .collect::<Vec<_>>();
        if spans != self.selected_spans {
            self.set_span_selection(spans);
        }
        let control_valid = self
            .selection
            .and_then(|(id, index)| index.map(|index| GeometryControl::Loop(id, index)))
            .or_else(|| {
                self.internal_selection
                    .and_then(|(id, index)| index.map(|index| GeometryControl::Baffle(id, index)))
            })
            .is_none_or(|control| self.editor.control_point(control).is_some());
        if !control_valid {
            self.focus_control(None);
        }
        let focus_valid = match self.focused_feature {
            Some(FocusedFeature::Loop(id)) => self.editor.obstacle(id).is_some(),
            Some(FocusedFeature::Baffle(id)) => self.editor.internal_boundary(id).is_some(),
            None => true,
        };
        if !focus_valid {
            self.focused_feature = None;
            self.loop_role_edit = None;
        }
    }

    fn transformable_curve_controls(&self) -> Option<Vec<Vec<GeometryControl>>> {
        if self.selected_spans.is_empty()
            || self
                .selected_spans
                .iter()
                .any(|span| matches!(span, GeometrySpan::Outer(_)))
        {
            return None;
        }
        let mut groups = Vec::<Vec<GeometryControl>>::new();
        let mut loop_ids = Vec::new();
        let mut baffle_ids = Vec::new();
        for selected in &self.selected_spans {
            match *selected {
                GeometrySpan::Loop(id, _) if !loop_ids.contains(&id) => loop_ids.push(id),
                GeometrySpan::Baffle(id, _) if !baffle_ids.contains(&id) => baffle_ids.push(id),
                GeometrySpan::Outer(_) => return None,
                _ => {}
            }
        }
        for id in loop_ids {
            let spline = &self.editor.obstacle(id)?.spline;
            let count = spline.intervals().len();
            let selected = (0..count)
                .map(|span| self.selected_spans.contains(&GeometrySpan::Loop(id, span)))
                .collect::<Vec<_>>();
            if selected.iter().all(|value| *value) {
                groups.push(
                    (0..spline.controls().len())
                        .map(|index| GeometryControl::Loop(id, index))
                        .collect(),
                );
                continue;
            }
            let anchor = selected.iter().position(|value| !*value)?;
            let mut run = Vec::new();
            let mut runs = Vec::new();
            for offset in 1..=count {
                let span = (anchor + offset) % count;
                if selected[span] {
                    run.push(span);
                } else if !run.is_empty() {
                    runs.push(std::mem::take(&mut run));
                }
            }
            if !run.is_empty() {
                runs.push(run);
            }
            for run in runs {
                let first = run[0];
                let after = (run[run.len() - 1] + 1) % count;
                if spline.multiplicities()[first] != 3 || spline.multiplicities()[after] != 3 {
                    return None;
                }
                let mut controls = Vec::new();
                for span in run {
                    for index in spline.span_control_indices(span)? {
                        let control = GeometryControl::Loop(id, index);
                        if !controls.contains(&control) {
                            controls.push(control);
                        }
                    }
                }
                groups.push(controls);
            }
        }
        for id in baffle_ids {
            let spline = &self.editor.internal_boundary(id)?.spline;
            let count = spline.intervals().len();
            let selected = (0..count)
                .map(|span| {
                    self.selected_spans
                        .contains(&GeometrySpan::Baffle(id, span))
                })
                .collect::<Vec<_>>();
            if selected.iter().all(|value| *value) {
                groups.push(
                    (0..spline.controls().len())
                        .map(|index| GeometryControl::Baffle(id, index))
                        .collect(),
                );
                continue;
            }
            let mut start = 0;
            while start < count {
                if !selected[start] {
                    start += 1;
                    continue;
                }
                let first = start;
                while start + 1 < count && selected[start + 1] {
                    start += 1;
                }
                let last = start;
                let left_isolated = first == 0 || spline.multiplicities()[first - 1] == 3;
                let right_isolated = last + 1 == count || spline.multiplicities()[last] == 3;
                if !left_isolated || !right_isolated {
                    return None;
                }
                let mut controls = Vec::new();
                for span in first..=last {
                    for index in spline.span_control_indices(span)? {
                        let control = GeometryControl::Baffle(id, index);
                        if !controls.contains(&control) {
                            controls.push(control);
                        }
                    }
                }
                groups.push(controls);
                start += 1;
            }
        }
        Some(groups)
    }

    fn selected_control_points(&self) -> Vec<(GeometryControl, Point2)> {
        if let Some(control) = self
            .selection
            .and_then(|(id, index)| index.map(|index| GeometryControl::Loop(id, index)))
            .or_else(|| {
                self.internal_selection
                    .and_then(|(id, index)| index.map(|index| GeometryControl::Baffle(id, index)))
            })
        {
            return self
                .editor
                .control_point(control)
                .map(|point| vec![(control, point)])
                .unwrap_or_default();
        }
        let mut points = Vec::new();
        for control in self
            .transformable_curve_controls()
            .unwrap_or_default()
            .into_iter()
            .flatten()
        {
            if points
                .iter()
                .any(|(candidate, _): &(GeometryControl, Point2)| *candidate == control)
            {
                continue;
            }
            if let Some(point) = self.editor.control_point(control) {
                points.push((control, point));
            }
        }
        points
    }

    fn selection_pivot(&self) -> Option<Point2> {
        if let Some(pivot) = self.gizmo_pivot {
            return Some(pivot);
        }
        if self.selected_spans.is_empty() {
            return self
                .selected_control_points()
                .first()
                .map(|(_, point)| *point);
        }
        self.transformable_curve_controls()?;
        self.selected_arc_measure()
            .map(|(weighted, length)| weighted / length)
    }

    fn selected_arc_measure(&self) -> Option<(Point2, f64)> {
        let options = SamplingOptions {
            tolerance: 1.0e-3,
            max_depth: 12,
            max_points: 2048,
        };
        let mut weighted = Point2::default();
        let mut total = 0.0;
        let mut loop_ids = Vec::new();
        let mut baffle_ids = Vec::new();
        for selected in &self.selected_spans {
            match *selected {
                GeometrySpan::Loop(id, _) if !loop_ids.contains(&id) => loop_ids.push(id),
                GeometrySpan::Baffle(id, _) if !baffle_ids.contains(&id) => baffle_ids.push(id),
                _ => {}
            }
        }
        for id in loop_ids {
            let spline = &self.editor.obstacle(id)?.spline;
            let samples = sample(spline, options).ok()?;
            for segment in samples.windows(2) {
                let parameter = (segment[0].t + segment[1].t) * 0.5;
                let span = spline.span_index(parameter)?;
                if self.selected_spans.contains(&GeometrySpan::Loop(id, span)) {
                    let length = (segment[1].point - segment[0].point).norm();
                    weighted = weighted + (segment[0].point + segment[1].point) * (0.5 * length);
                    total += length;
                }
            }
        }
        for id in baffle_ids {
            let spline = &self.editor.internal_boundary(id)?.spline;
            let samples = sample_open(spline, options).ok()?;
            for segment in samples.windows(2) {
                let parameter = (segment[0].t + segment[1].t) * 0.5;
                let span = spline.span_index(parameter)?;
                if self
                    .selected_spans
                    .contains(&GeometrySpan::Baffle(id, span))
                {
                    let length = (segment[1].point - segment[0].point).norm();
                    weighted = weighted + (segment[0].point + segment[1].point) * (0.5 * length);
                    total += length;
                }
            }
        }
        (total > 0.0).then_some((weighted, total))
    }

    fn snap_point(&self, point: Point2) -> Point2 {
        if !self.snap_to_grid || !self.snap_step.is_finite() || self.snap_step <= 0.0 {
            return point;
        }
        Point2::new(
            (point.x / self.snap_step).round() * self.snap_step,
            (point.y / self.snap_step).round() * self.snap_step,
        )
    }

    fn apply_selection_transform(&mut self) {
        let Some(pivot) = self.selection_pivot() else {
            return;
        };
        if !self.transform_scale.is_finite()
            || self.transform_scale <= 0.0
            || !self.transform_rotation_degrees.is_finite()
            || !self.transform_translation.finite()
        {
            self.message = "Transform values must be finite and scale must be positive".into();
            return;
        }
        let angle = self.transform_rotation_degrees.to_radians();
        let (sin, cos) = angle.sin_cos();
        let translated_pivot = self.snap_point(pivot + self.transform_translation);
        let translation = translated_pivot - pivot;
        let updates = self
            .selected_control_points()
            .into_iter()
            .map(|(control, point)| {
                let relative = (point - pivot) * self.transform_scale;
                let rotated = Point2::new(
                    cos * relative.x - sin * relative.y,
                    sin * relative.x + cos * relative.y,
                );
                (control, pivot + rotated + translation)
            })
            .collect::<Vec<_>>();
        self.editor.begin();
        let result = self.editor.set_control_points(&updates);
        if self.error(result).is_some() {
            self.editor.commit();
            self.gizmo_pivot = Some(translated_pivot);
            self.transform_translation = Point2::default();
            self.transform_rotation_degrees = 0.0;
            self.transform_scale = 1.0;
        } else {
            self.editor.cancel();
        }
    }

    fn apply_control_updates(&mut self, updates: Vec<(GeometryControl, Point2)>) -> bool {
        self.editor.begin();
        let result = self.editor.set_control_points(&updates);
        if self.error(result).is_some() {
            self.editor.commit();
            true
        } else {
            self.editor.cancel();
            false
        }
    }

    fn snap_selection_now(&mut self) {
        let Some(pivot) = self.selection_pivot() else {
            return;
        };
        let target = self.snap_point(pivot);
        let delta = target - pivot;
        let updates = self
            .selected_control_points()
            .into_iter()
            .map(|(control, point)| (control, point + delta))
            .collect();
        if self.apply_control_updates(updates) {
            self.gizmo_pivot = (!self.selected_spans.is_empty()).then_some(target);
        }
    }

    fn align_selection(&mut self, horizontal: bool) {
        let Some(pivot) = self.selection_pivot() else {
            return;
        };
        let Some(groups) = self.transformable_curve_controls() else {
            return;
        };
        let mut updates = Vec::new();
        for group in groups {
            let Some(center) = group
                .iter()
                .filter_map(|control| self.editor.control_point(*control))
                .reduce(|sum, point| sum + point)
                .map(|sum| sum / group.len() as f64)
            else {
                return;
            };
            let delta = if horizontal {
                Point2::new(0.0, pivot.y - center.y)
            } else {
                Point2::new(pivot.x - center.x, 0.0)
            };
            updates.extend(group.into_iter().filter_map(|control| {
                self.editor
                    .control_point(control)
                    .map(|point| (control, point + delta))
            }));
        }
        self.apply_control_updates(updates);
    }
    fn finish_custom(&mut self) {
        if self.custom.len() < 4 {
            self.message = "A cubic curve needs at least four control points".into();
            return;
        }
        if self.creation_role == CreationRole::InternalBoundary {
            let spline = OpenCubicSpline::uniform(self.custom.clone()).unwrap();
            let anchor = spline.evaluate(spline.period() * 0.5);
            let region = self.region_at(anchor);
            let result = self.editor.create_internal_boundary(spline, region);
            if let Some(id) = self.error(result) {
                self.select_baffle(id);
                self.custom.clear();
                self.interaction_mode = InteractionMode::Select;
            }
            return;
        }
        let spline = PeriodicCubicSpline::uniform(self.custom.clone()).unwrap();
        let center = self
            .custom
            .iter()
            .copied()
            .fold(Point2::default(), |sum, point| sum + point)
            / self.custom.len() as f64;
        let result = self.create_spline(spline, center);
        if let Some(id) = self.error(result) {
            self.select_loop(id);
            self.custom.clear();
            self.interaction_mode = InteractionMode::Select;
        }
    }

    fn create_spline(
        &mut self,
        spline: PeriodicCubicSpline,
        anchor: Point2,
    ) -> Result<ObstacleId, String> {
        let exterior = self.region_at(anchor);
        match self.creation_role {
            CreationRole::Hole => self.editor.create_loop(spline, LoopRole::Hole { exterior }),
            CreationRole::MaterialInterface => {
                self.editor
                    .create_region_loop(spline, exterior, self.material_selection, false)
            }
            CreationRole::InternalBoundary => Err("Open boundaries use an open spline".into()),
        }
    }

    fn region_at(&self, point: Point2) -> RegionId {
        self.mesh
            .as_ref()
            .and_then(|mesh| mesh_region_at(mesh, point))
            .unwrap_or(BACKGROUND_REGION)
    }
    fn update_files(&mut self) {
        if !self.startup_load_checked {
            self.startup_load_checked = true;
            if !self.automated_benchmark {
                // Establish the first catalog entry as the startup baseline. A
                // valid shared scene or recovery document replaces this pending
                // load before validation advances below.
                self.start_initial_example_load();
                if let Some(result) = sharing::initial_fragment() {
                    self.share_fragment_active = true;
                    match result.and_then(|bytes| persistence::parse(&bytes)) {
                        Ok(load) => self.start_load(
                            load,
                            LoadMode::Replace,
                            "Shared scene loaded; history cleared",
                        ),
                        Err(error) => {
                            self.message = format!("Could not load shared scene: {error}")
                        }
                    }
                } else {
                    match recovery::load() {
                        Ok(Some(bytes)) => match persistence::parse(&bytes) {
                            Ok(load) => {
                                self.start_load(load, LoadMode::Replace, "Autosaved scene restored")
                            }
                            Err(error) => {
                                self.message = format!("Could not restore autosave: {error}")
                            }
                        },
                        Ok(None) => {}
                        Err(error) => self.message = error,
                    }
                }
            }
        }
        let events: Vec<_> = self.receiver.lock().unwrap().try_iter().collect();
        for event in events {
            self.file_busy = false;
            match event {
                FileEvent::Loaded(bytes) => match persistence::parse(&bytes) {
                    Ok(load) => {
                        self.start_load(load, LoadMode::Replace, "Scene loaded; history cleared")
                    }
                    Err(e) => self.message = e,
                },
                FileEvent::Saved(message) => self.notify(message),
                FileEvent::Cancelled => {}
                FileEvent::Error(e) => self.message = e,
            }
        }
        if let Some(load) = &mut self.load
            && let Some(result) = load.advance(12_000)
        {
            self.load = None;
            let example_simulation = self.load_example_simulation.take();
            match result {
                Ok(document) => {
                    match self.load_mode {
                        LoadMode::Replace => self.editor.replace_validated(document),
                        LoadMode::Undoable => self.editor.replace_validated_with_history(document),
                    }
                    self.clear_transient();
                    self.clear_all_probe_traces();
                    self.probe_compiled = None;
                    self.wave_source = self.editor.document.source;
                    self.wave_source_dirty = true;
                    if let Some(simulation) = example_simulation {
                        self.wave_source = simulation.source;
                        self.wave_pending_pulse = None;
                        self.fresh_simulation_requested = true;
                        self.show_boundary_conditions = true;
                    }
                    if !self.load_notice.is_empty() {
                        self.notify(self.load_notice);
                    }
                }
                Err(e) => self.message = e,
            }
        }
        self.refresh_autosave();
    }

    fn start_load(&mut self, load: LoadCandidate, mode: LoadMode, notice: &'static str) {
        self.load = Some(load);
        self.load_mode = mode;
        self.load_notice = notice;
        self.load_example_simulation = None;
        self.message.clear();
    }

    fn start_example_load(&mut self, example: &examples::ExampleScene) {
        self.start_load(
            persistence::candidate(example.document.clone()),
            LoadMode::Undoable,
            "Example opened; Undo restores the previous scene",
        );
        self.load_example_simulation = Some(example.simulation);
    }

    fn start_initial_example_load(&mut self) {
        let example = &examples::catalog()[0];
        self.start_load(
            persistence::candidate(example.document.clone()),
            LoadMode::Replace,
            "",
        );
        self.load_example_simulation = Some(example.simulation);
    }

    fn refresh_autosave(&mut self) {
        if self.automated_benchmark || self.load.is_some() {
            return;
        }
        if self.editor.document != self.autosave_observed {
            self.autosave_observed = self.editor.document.clone();
            self.autosave_due = Some(Instant::now());
            return;
        }
        if self
            .autosave_due
            .is_some_and(|started| started.elapsed().as_secs_f32() >= 0.8)
        {
            self.autosave_due = None;
            let result = recovery::save(&self.autosave_observed).and_then(|()| {
                if self.share_fragment_active {
                    let fragment = sharing::encode(&self.autosave_observed)?;
                    sharing::replace_fragment(&fragment)
                } else {
                    Ok(())
                }
            });
            if let Err(error) = result {
                self.message = error;
            }
        }
    }
    fn refresh_curves(&mut self) {
        if self.cache_revision == self.editor.revision
            && self.cache_scale == self.scale
            && self.cache_accepted == self.editor.document.accepted
        {
            return;
        }
        self.cache_revision = self.editor.revision;
        self.cache_scale = self.scale;
        self.cache_accepted = self.editor.document.accepted.clone();
        self.sampling_warning = false;
        let options = SamplingOptions {
            tolerance: 0.6 / self.scale,
            max_depth: 14,
            max_points: 2048,
        };
        {
            let mut curves = |scene: &Scene| {
                scene
                    .obstacles
                    .iter()
                    .map(|o| {
                        let samples = match sample(&o.spline, options) {
                            Ok(s) => s,
                            Err(_) => {
                                self.sampling_warning = true;
                                vec![]
                            }
                        };
                        Curve { id: o.id, samples }
                    })
                    .collect()
            };
            self.draft_curves = curves(&self.editor.document.draft);
            self.accepted_curves = curves(&self.editor.document.accepted);
        }
        let mut internal_curves = |scene: &Scene| {
            scene
                .internal_boundaries
                .iter()
                .map(|boundary| {
                    let samples = match sample_open(&boundary.spline, options) {
                        Ok(samples) => samples,
                        Err(_) => {
                            self.sampling_warning = true;
                            vec![]
                        }
                    };
                    InternalCurve {
                        id: boundary.id,
                        samples,
                    }
                })
                .collect()
        };
        self.draft_internal_curves = internal_curves(&self.editor.document.draft);
        self.accepted_internal_curves = internal_curves(&self.editor.document.accepted);
    }

    fn refresh_mesh(&mut self) {
        // Prepare from the displayed mesh's own scene, never an obsolete
        // in-flight request. Geometry edits are coalesced until the drag ends.
        let geometry_pending = !self.mesh_source.geometry_eq(&self.editor.document.accepted)
            || self.mesh_source_max_edge != self.mesh_max_edge;
        if self.editor.editing() || geometry_pending {
            self.solution_indicator_job = None;
            self.solution_indicator_result = None;
            self.solution_indicator_source = None;
            self.mesh_adaptation_job = None;
            self.mesh_adaptation_automatic = false;
            self.amr_coarsen_streak = 0;
            self.amr_status = "geometry has priority";
        }
        if self.editor.editing() || self.simulation_candidate.is_some() {
            return;
        }
        let start = Instant::now();
        let geometry_changed = geometry_pending;
        if geometry_changed {
            self.mesh_started = Some(start);
            self.mesh_source = self.editor.document.accepted.clone();
            self.mesh_source_max_edge = self.mesh_max_edge;
            let previous = self
                .mesh
                .as_ref()
                .filter(|_| self.mesh_committed_max_edge == self.mesh_max_edge)
                .map(|mesh| (mesh.clone(), self.mesh_committed_scene.clone()));
            let mesh_revision = self.next_mesh_revision;
            self.next_mesh_revision = self.next_mesh_revision.wrapping_add(1).max(1);
            self.mesh_job = Some(MeshUpdateJob::new_versioned(
                previous,
                self.mesh_source.clone(),
                self.editor.revision,
                mesh_revision,
                MeshingOptions {
                    curve_tolerance: (self.mesh_max_edge * 0.02).min(1.5e-3),
                    target_edge_length: self.mesh_max_edge / 1.05,
                    minimum_angle_degrees: 12.0,
                    max_vertices: 50_000,
                    max_triangles: 100_000,
                    max_refinement_steps: 50_000,
                },
            ));
            self.mesh_error = None;
            self.mesh_build_ms = 0.0;
            self.mesh_work_ms = 0.0;
            self.mesh_max_slice_ms = 0.0;
        }
        // A soft 2 ms deadline plus an operation ceiling. Check between units,
        // including topology preparation and individual edge flips.
        let mut result = None;
        let active = self.mesh_job.is_some();
        if let Some(job) = &mut self.mesh_job {
            for _ in 0..100_000 {
                result = job.advance(1);
                if result.is_some() || start.elapsed().as_secs_f64() >= 0.002 {
                    break;
                }
            }
        }
        if let Some(result) = result {
            let report = match &result {
                Ok(result) => result.report.clone(),
                Err(_) => self.mesh_job.as_ref().unwrap().report().clone(),
            };
            if report.local_attempted {
                self.mesh_attempts += 1;
                if !report.used_local {
                    self.mesh_fallbacks += 1;
                    if let Some(failure) = &report.fallback_failure {
                        *self.mesh_fallback_causes.entry(failure.kind).or_default() += 1;
                    }
                }
            }
            self.mesh_report = Some(report);
            self.mesh_job = None;
            match result {
                Ok(result) => {
                    let mesh = Arc::new(result.mesh);
                    let low_quality = (0..mesh.triangles.len())
                        .map(|i| mesh.triangle_quality(i).unwrap().minimum_angle_degrees < 15.0)
                        .collect();
                    let prepare = Instant::now();
                    match QuadraticWaveOperator::assemble_scene_with_boundaries(
                        &mesh,
                        &self.mesh_source,
                        self.mesh_source.outer_boundaries,
                    ) {
                        Ok(operator) => {
                            let fresh = self.fresh_simulation_requested;
                            let transfer = if fresh {
                                Ok(None)
                            } else {
                                self.wave_mesh
                                    .as_ref()
                                    .zip(self.wave_operator.as_ref())
                                    .map(|(source_mesh, source_operator)| {
                                        QuadraticTransferMap::build(
                                            source_mesh,
                                            source_operator,
                                            &mesh,
                                            &operator,
                                        )
                                    })
                                    .transpose()
                            };
                            match transfer {
                                Ok(transfer) => {
                                    let exposed_nodes = transfer
                                        .as_ref()
                                        .map_or(operator.degrees_of_freedom(), |map| {
                                            map.exposed_nodes()
                                        });
                                    let adaptation_state = MeshAdaptationState::from_mesh(&mesh);
                                    let time_step = operator.recommended_time_step();
                                    self.simulation_candidate = Some(SimulationCandidate {
                                        source_region: mesh_region_at(
                                            &mesh,
                                            self.wave_source.position,
                                        )
                                        .unwrap_or(RegionId(0)),
                                        mesh,
                                        scene: self.mesh_source.clone(),
                                        max_edge: self.mesh_source_max_edge,
                                        low_quality,
                                        operator: Arc::new(operator),
                                        boundary: self.mesh_source.outer_boundaries,
                                        time_step,
                                        transfer,
                                        generation: None,
                                        simulation_time: 0.0,
                                        exposed_nodes,
                                        adaptation_state,
                                        fresh,
                                    });
                                    self.wave_prepare_ms = prepare.elapsed().as_secs_f64() * 1000.0;
                                    self.mesh_error = None;
                                }
                                Err(error) => self.mesh_error = Some(error.to_string()),
                            }
                        }
                        Err(error) => self.mesh_error = Some(error.to_string()),
                    }
                }
                Err(error) => {
                    self.mesh_error = Some(error.to_string());
                }
            }
        }
        if !geometry_changed
            && self.mesh_job.is_none()
            && self.simulation_candidate.is_none()
            && (self.editor.document.accepted != self.mesh_committed_scene
                || self.fresh_simulation_requested)
            && let (Some(mesh), Some(source_operator)) =
                (self.wave_mesh.as_ref(), self.wave_operator.as_ref())
        {
            let prepare = Instant::now();
            match QuadraticWaveOperator::assemble_scene_with_boundaries(
                mesh,
                &self.editor.document.accepted,
                self.editor.document.accepted.outer_boundaries,
            ) {
                Ok(operator) => {
                    let fresh = self.fresh_simulation_requested;
                    let transfer = if fresh {
                        Ok(None)
                    } else {
                        QuadraticTransferMap::identity_on_mesh(mesh, source_operator, &operator)
                            .map(Some)
                    };
                    match transfer {
                        Ok(transfer) => {
                            let time_step = operator.recommended_time_step();
                            let exposed_nodes = transfer.as_ref().map_or(
                                operator.degrees_of_freedom(),
                                QuadraticTransferMap::exposed_nodes,
                            );
                            self.simulation_candidate = Some(SimulationCandidate {
                                source_region: mesh_region_at(mesh, self.wave_source.position)
                                    .unwrap_or(RegionId(0)),
                                mesh: mesh.clone(),
                                scene: self.editor.document.accepted.clone(),
                                max_edge: self.mesh_committed_max_edge,
                                low_quality: self.mesh_low_quality.clone(),
                                operator: Arc::new(operator),
                                boundary: self.editor.document.accepted.outer_boundaries,
                                time_step,
                                exposed_nodes,
                                transfer,
                                adaptation_state: self
                                    .mesh_adaptation_state
                                    .clone()
                                    .unwrap_or_else(|| MeshAdaptationState::from_mesh(mesh)),
                                generation: None,
                                simulation_time: 0.0,
                                fresh,
                            });
                            self.wave_prepare_ms = prepare.elapsed().as_secs_f64() * 1000.0;
                            self.wave_error = None;
                            self.message.clear();
                        }
                        Err(error) => self.wave_error = Some(error.to_string()),
                    }
                }
                Err(error) => self.wave_error = Some(error.to_string()),
            }
        }
        if active {
            // Include completed-job cleanup and mesh replacement in the slice.
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            self.mesh_work_ms += elapsed;
            self.mesh_max_slice_ms = self.mesh_max_slice_ms.max(elapsed);
            self.mesh_build_ms = self
                .mesh_started
                .map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0);
        }
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn start_mesh_adaptation(
        &mut self,
        field: Arc<dyn MeshSizeField>,
        options: MeshAdaptationOptions,
    ) -> Result<(), String> {
        if self.editor.editing()
            || self.mesh_job.is_some()
            || self.mesh_adaptation_job.is_some()
            || self.simulation_candidate.is_some()
        {
            return Err("Another mesh transaction is active".into());
        }
        let mesh = self
            .mesh
            .as_ref()
            .ok_or("The committed mesh is not ready")?
            .clone();
        let state = self
            .mesh_adaptation_state
            .clone()
            .filter(|state| state.mesh_revision == mesh.mesh_revision)
            .unwrap_or_else(|| MeshAdaptationState::from_mesh(&mesh));
        let revision = self.next_mesh_revision;
        self.next_mesh_revision = self.next_mesh_revision.wrapping_add(1).max(1);
        self.mesh_adaptation_job = Some(MeshAdaptationJob::new(
            mesh,
            self.mesh_committed_scene.clone(),
            state,
            revision,
            field,
            options,
        ));
        self.mesh_adaptation_automatic = false;
        self.mesh_adaptation_report = None;
        Ok(())
    }

    fn refresh_mesh_adaptation(&mut self) {
        if self.mesh_adaptation_job.is_none() || self.simulation_candidate.is_some() {
            return;
        }
        let started = Instant::now();
        let mut result = None;
        if let Some(job) = &mut self.mesh_adaptation_job {
            for _ in 0..100_000 {
                result = job.advance(1);
                if result.is_some() || started.elapsed().as_secs_f64() >= 0.002 {
                    break;
                }
            }
        }
        let Some(result) = result else {
            return;
        };
        let report = match &result {
            Ok(result) => result.report.clone(),
            Err(_) => self.mesh_adaptation_job.as_ref().unwrap().report().clone(),
        };
        self.mesh_adaptation_report = Some(report);
        self.mesh_adaptation_job = None;
        let automatic = std::mem::take(&mut self.mesh_adaptation_automatic);
        let result = match result {
            Ok(result) => result,
            Err(MeshAdaptationError::WorkLimit) if automatic => {
                self.amr_status = "adaptation budget reached; mesh retained";
                self.amr_error = None;
                return;
            }
            Err(error) => {
                self.mesh_error = Some(error.to_string());
                return;
            }
        };
        let mesh = Arc::new(result.mesh);
        let low_quality = (0..mesh.triangles.len())
            .map(|index| mesh.triangle_quality(index).unwrap().minimum_angle_degrees < 15.0)
            .collect();
        let prepare = Instant::now();
        let operator = match QuadraticWaveOperator::assemble_scene_with_boundaries(
            &mesh,
            &self.mesh_committed_scene,
            self.mesh_committed_scene.outer_boundaries,
        ) {
            Ok(operator) => operator,
            Err(error) => {
                self.mesh_error = Some(error.to_string());
                return;
            }
        };
        let transfer = match self
            .wave_mesh
            .as_ref()
            .zip(self.wave_operator.as_ref())
            .map(|(source_mesh, source_operator)| {
                QuadraticTransferMap::build(source_mesh, source_operator, &mesh, &operator)
            })
            .transpose()
        {
            Ok(transfer) => transfer,
            Err(error) => {
                self.mesh_error = Some(error.to_string());
                return;
            }
        };
        let exposed_nodes = transfer.as_ref().map_or(
            operator.degrees_of_freedom(),
            QuadraticTransferMap::exposed_nodes,
        );
        self.simulation_candidate = Some(SimulationCandidate {
            source_region: mesh_region_at(&mesh, self.wave_source.position)
                .unwrap_or(BACKGROUND_REGION),
            mesh,
            scene: self.mesh_committed_scene.clone(),
            max_edge: self.mesh_committed_max_edge,
            low_quality,
            time_step: operator.recommended_time_step(),
            operator: Arc::new(operator),
            boundary: self.mesh_committed_scene.outer_boundaries,
            transfer,
            generation: None,
            simulation_time: 0.0,
            exposed_nodes,
            adaptation_state: result.state,
            fresh: false,
        });
        self.wave_prepare_ms = prepare.elapsed().as_secs_f64() * 1000.0;
        self.mesh_error = None;
    }

    fn solution_indicator_options(&self) -> SolutionIndicatorOptions {
        SolutionIndicatorOptions {
            minimum_edge_length: self.amr_minimum_edge,
            maximum_edge_length: self.amr_maximum_edge,
            relative_tolerance: self.amr_quality.relative_tolerance(),
            elements_per_wavelength: self.amr_quality.elements_per_wavelength(),
            forcing_frequency_hz: highest_forcing_frequency(
                &self.mesh_committed_scene,
                self.wave_source,
            ),
            maximum_scale: self.amr_quality.maximum_coarsening_scale(),
            coarsen_ratio: self.amr_quality.collapse_ratio(),
            ..Default::default()
        }
    }

    fn refresh_solution_amr(&mut self, request: &WaveGpuRequest, display: &WaveDisplay) {
        if !self.amr_enabled {
            self.solution_indicator_job = None;
            self.solution_indicator_result = None;
            self.solution_indicator_source = None;
            self.amr_coarsen_streak = 0;
            self.amr_status = "off";
            return;
        }
        if self.editor.editing()
            || self.mesh_job.is_some()
            || self.mesh_adaptation_job.is_some()
            || self.simulation_candidate.is_some()
        {
            if self.editor.editing() || self.mesh_job.is_some() {
                self.solution_indicator_job = None;
                self.solution_indicator_source = None;
                self.amr_status = "geometry has priority";
            } else if self.mesh_adaptation_job.is_some() {
                self.amr_status = "adapting mesh";
            } else {
                self.amr_status = "handing off mesh";
            }
            return;
        }

        if self.solution_indicator_job.is_some() {
            let started = Instant::now();
            let mut result = None;
            if let Some(job) = &mut self.solution_indicator_job {
                self.amr_status = job.phase();
                for _ in 0..100_000 {
                    result = job.advance(1);
                    if result.is_some() || started.elapsed().as_secs_f64() >= 0.002 {
                        break;
                    }
                }
            }
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            self.amr_work_ms += elapsed;
            self.amr_max_slice_ms = self.amr_max_slice_ms.max(elapsed);
            let Some(result) = result else {
                return;
            };
            let source = self.solution_indicator_source.take();
            let report = match &result {
                Ok(result) => result.report.clone(),
                Err(_) => self
                    .solution_indicator_job
                    .as_ref()
                    .unwrap()
                    .report()
                    .clone(),
            };
            self.solution_indicator_job = None;
            self.solution_indicator_report = Some(report);
            let Some((mesh_revision, generation, buffer_revision, step, settings_revision)) =
                source
            else {
                self.amr_status = "discarded stale estimate";
                return;
            };
            let Some(mesh) = self.wave_mesh.as_ref() else {
                self.amr_status = "discarded stale estimate";
                return;
            };
            if mesh.mesh_revision != mesh_revision
                || request.generation() != generation
                || request.buffer_revision() != buffer_revision
                || self.amr_settings_revision != settings_revision
            {
                self.amr_status = "discarded stale estimate";
                return;
            }
            self.amr_last_analyzed_step = Some(step);
            let result = match result {
                Ok(result) => result,
                Err(error) => {
                    self.amr_error = Some(error.to_string());
                    self.amr_status = "estimate failed";
                    return;
                }
            };
            let decision = amr_transaction_decision(mesh, &result);
            let coarsening_confirmed =
                update_coarsening_confirmation(&mut self.amr_coarsen_streak, decision.coarsen);
            self.solution_indicator_result = Some(result.clone());
            if !decision.refine && !decision.coarsen {
                self.amr_status = "mesh matches solution";
                self.amr_error = None;
                return;
            }
            if !decision.refine && !coarsening_confirmed {
                self.amr_status = "confirming coarsening";
                self.amr_error = None;
                return;
            }
            let topology_budget = self.amr_quality.topology_budget();
            let coarsening_budget = if coarsening_confirmed {
                topology_budget / 2
            } else {
                0
            };
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
                collapse_ratio: self.amr_quality.collapse_ratio(),
                max_topology_changes: topology_budget,
                max_coarsening_changes: coarsening_budget,
                max_work_units: automatic_adaptation_work_limit(mesh, topology_budget),
                ..Default::default()
            };
            let field: Arc<dyn MeshSizeField> = result.field;
            match self.start_mesh_adaptation(field, options) {
                Ok(()) => {
                    self.mesh_adaptation_automatic = true;
                    self.amr_status = "adapting mesh";
                    self.amr_error = None;
                }
                Err(error) => {
                    self.amr_error = Some(error);
                    self.amr_status = "adaptation deferred";
                }
            }
            return;
        }

        let (Some(mesh), Some(operator)) = (self.wave_mesh.as_ref(), self.wave_operator.as_ref())
        else {
            self.amr_status = "waiting for solution";
            return;
        };
        let dofs = operator.degrees_of_freedom();
        if !request.ready()
            || display.generation != request.generation()
            || display.current.len() != dofs
            || display.auxiliary.len() != dofs
            || display.indicator_displacement.len() != dofs
            || display.indicator_velocity.len() != dofs
            || display.indicator_acceleration.len() != dofs
        {
            self.amr_status = "waiting for aligned readback";
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
            self.amr_status = "monitoring solution";
            return;
        }
        let time = self.wave_time_offset + step.saturating_sub(1) as f64 * self.wave_time_step;
        let Some(volume_acceleration) = request.volume_acceleration(time) else {
            self.amr_status = "waiting for source state";
            return;
        };
        let Some(auxiliary) =
            aligned_indicator_auxiliary(display, operator, self.wave_time_step, step)
        else {
            self.amr_status = "waiting for aligned readback";
            return;
        };
        let snapshot = QuadraticSolutionSnapshot {
            mesh_revision: mesh.mesh_revision,
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
            time_step: self.wave_time_step,
        };
        self.solution_indicator_job = Some(SolutionIndicatorJob::new(
            mesh.clone(),
            operator.clone(),
            self.mesh_committed_scene.clone(),
            snapshot,
            self.solution_indicator_options(),
        ));
        self.solution_indicator_source = Some((
            mesh.mesh_revision,
            request.generation(),
            request.buffer_revision(),
            step,
            self.amr_settings_revision,
        ));
        self.amr_last_started = Some(Instant::now());
        self.amr_status = "preparing estimate";
    }

    fn refresh_wave(
        &mut self,
        request: &mut WaveGpuRequest,
        display: &WaveDisplay,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        delta_seconds: f64,
    ) {
        if self.wave_reset_requested && self.simulation_candidate.is_none() {
            self.wave_reset_requested = false;
            self.wave_running = false;
            self.wave_accumulator = 0.0;
            self.wave_time_offset = 0.0;
            self.wave_active_wall_seconds = 0.0;
            self.wave_completed_steps = 0;
            self.wave_steps_per_second = 0.0;
            self.wave_rate_previous_completed = 0;
            self.wave_dispatches = 0;
            self.wave_substeps_last = 0;
            self.wave_energy = None;
            self.wave_energy_step = 0;
            self.wave_error = None;
            self.clear_all_probe_traces();
            self.probe_compiled = None;
            self.solution_indicator_job = None;
            self.solution_indicator_result = None;
            self.solution_indicator_source = None;
            self.amr_last_analyzed_step = None;
            self.amr_status = "waiting for reset readback";
            if let (Some(mesh), Some(operator)) = (&self.wave_mesh, &self.wave_operator) {
                if let Err(error) = request.reset(
                    assets,
                    commands,
                    mesh,
                    operator,
                    self.wave_time_step,
                    self.wave_source,
                ) {
                    self.wave_error = Some(error);
                } else {
                    self.wave_source_dirty = false;
                }
            }
        }
        if let Some(candidate) = &mut self.simulation_candidate
            && candidate.generation.is_none()
        {
            // Stop issuing new steps and let the render world encode every
            // previously requested step before changing buffer generations.
            if request.caught_up() {
                let mut target_source = self.wave_source;
                target_source.region = candidate.source_region;
                candidate.simulation_time = if candidate.fresh {
                    0.0
                } else {
                    self.wave_time_offset
                        + request.stats().completed_steps() as f64 * self.wave_time_step
                };
                let replacement = if candidate.fresh {
                    request.replace(
                        assets,
                        commands,
                        &candidate.mesh,
                        &candidate.operator,
                        candidate.time_step,
                        target_source,
                    )
                } else if let (Some(source_mesh), Some(source_operator), Some(map)) =
                    (&self.wave_mesh, &self.wave_operator, &candidate.transfer)
                {
                    request.replace_transferred(
                        assets,
                        commands,
                        WaveTransfer {
                            source_mesh,
                            source_operator,
                            target_mesh: &candidate.mesh,
                            target_operator: &candidate.operator,
                            target_time_step: candidate.time_step,
                            source: target_source,
                            map,
                        },
                    )
                } else {
                    request.replace(
                        assets,
                        commands,
                        &candidate.mesh,
                        &candidate.operator,
                        candidate.time_step,
                        target_source,
                    )
                };
                match replacement {
                    Ok(()) => candidate.generation = Some(request.generation()),
                    Err(error) => {
                        self.wave_error = Some(format!("Candidate upload failed: {error}"));
                        self.simulation_candidate = None;
                    }
                }
            }
        }
        let candidate_ready = self.simulation_candidate.as_ref().is_some_and(|candidate| {
            candidate.generation == Some(request.generation())
                && request.ready()
                && display.generation == request.generation()
                && display.current.len() == candidate.operator.degrees_of_freedom()
        });
        if candidate_ready {
            let candidate = self.simulation_candidate.take().unwrap();
            let same_mesh = self
                .mesh
                .as_ref()
                .is_some_and(|mesh| Arc::ptr_eq(mesh, &candidate.mesh))
                && candidate.max_edge == self.mesh_committed_max_edge;
            let scene_settings_changed = candidate.scene != self.mesh_committed_scene;
            let boundary_changed = candidate.boundary != self.wave_boundary_committed;
            let commit_message = if candidate.fresh {
                "Example simulation initialized with a fresh field".into()
            } else if same_mesh && scene_settings_changed && boundary_changed {
                "Scene settings and outer boundary committed; live field preserved".into()
            } else if same_mesh && scene_settings_changed {
                "Scene settings committed; live field preserved".into()
            } else if same_mesh && boundary_changed {
                format!(
                    "{} outer boundary committed; live field preserved",
                    candidate.boundary.label()
                )
            } else if candidate.transfer.is_some() {
                format!(
                    "Simulation mesh committed · {} newly exposed solution nodes initialized to zero",
                    candidate.exposed_nodes
                )
            } else {
                "Initial simulation mesh committed".into()
            };
            request.finish_transfer(assets);
            self.mesh = Some(candidate.mesh.clone());
            self.mesh_low_quality = candidate.low_quality;
            self.mesh_committed_scene = candidate.scene;
            self.mesh_committed_max_edge = candidate.max_edge;
            self.wave_mesh = Some(candidate.mesh);
            self.wave_operator = Some(candidate.operator);
            self.mesh_adaptation_state = Some(candidate.adaptation_state);
            self.wave_boundary_committed = candidate.boundary;
            self.wave_time_step = candidate.time_step;
            self.wave_time_offset = candidate.simulation_time;
            if candidate.fresh {
                self.wave_accumulator = 0.0;
                self.wave_active_wall_seconds = 0.0;
                self.wave_dispatches = 0;
                self.wave_step_requested = false;
            }
            self.wave_completed_steps = 0;
            self.wave_steps_per_second = 0.0;
            self.wave_rate_previous_completed = 0;
            self.wave_energy = None;
            self.wave_energy_step = u64::MAX;
            self.wave_source_dirty = false;
            self.wave_source.region = candidate.source_region;
            self.editor.document.source.region = candidate.source_region;
            if candidate.fresh {
                self.fresh_simulation_requested = false;
            }
            self.solution_indicator_job = None;
            self.solution_indicator_result = None;
            self.solution_indicator_source = None;
            self.amr_last_analyzed_step = None;
            self.amr_coarsen_streak = 0;
            self.amr_status = "waiting for solution";
            self.mesh_build_ms = self
                .mesh_started
                .map_or(0.0, |started| started.elapsed().as_secs_f64() * 1000.0);
            self.notify(commit_message);
        } else if self
            .simulation_candidate
            .as_ref()
            .is_some_and(|candidate| candidate.generation.is_some())
            && request.failed()
        {
            if request.transfer_pending() {
                let rollback = request.rollback_transfer(assets, commands);
                if let Err(error) = rollback {
                    self.wave_error = Some(format!("Transfer failed and rollback failed: {error}"));
                } else {
                    self.wave_error =
                        Some("Candidate GPU transfer failed; active simulation retained".into());
                }
            } else {
                self.wave_error = Some("Candidate GPU initialization failed".into());
            }
            self.simulation_candidate = None;
        }
        if self.wave_source_dirty && self.wave_operator.is_some() {
            let Some(mesh) = self.wave_mesh.as_deref() else {
                return;
            };
            let Some(operator) = self.wave_operator.as_deref() else {
                return;
            };
            match request.update_source(assets, mesh, operator, self.wave_source) {
                Ok(()) => self.wave_source_dirty = false,
                Err(error) => self.wave_error = Some(error),
            }
        }
        if self.simulation_candidate.is_none()
            && let Some(position) = self.wave_pending_pulse.take()
        {
            let region = self
                .wave_mesh
                .as_ref()
                .and_then(|mesh| mesh_region_at(mesh, position))
                .unwrap_or(RegionId(0));
            let Some(mesh) = self.wave_mesh.as_deref() else {
                return;
            };
            let Some(operator) = self.wave_operator.as_deref() else {
                return;
            };
            match request.inject_pulse(
                assets,
                mesh,
                operator,
                PulseSettings {
                    position,
                    amplitude: self.pulse_amplitude,
                    width: self.pulse_width,
                    region,
                },
            ) {
                Ok(()) => {
                    // Commit one level after injection so the readback marker
                    // and energy identify the pulse even while paused.
                    request.request_steps(1);
                }
                Err(error) => self.wave_error = Some(error),
            }
        }

        self.wave_gpu_status = request.stats().status();
        let completed_steps = request.stats().completed_steps();
        let completed_delta = completed_steps.saturating_sub(self.wave_rate_previous_completed);
        let completed_rate = if delta_seconds > 1.0e-6 {
            completed_delta as f64 / delta_seconds
        } else {
            0.0
        };
        self.wave_rate_previous_completed = completed_steps;
        self.wave_completed_steps = completed_steps;
        self.wave_dispatches = request.stats().dispatches();
        self.wave_substeps_last = 0;
        // A pending handoff must drain the already requested work before it can
        // replace the GPU buffers. Keep the user's Run/Pause preference intact
        // while temporarily withholding both continuous and manual scheduling.
        if self.wave_operator.is_some() && request.ready() && self.simulation_candidate.is_none() {
            if self.wave_step_requested {
                request.request_steps(1);
                self.wave_substeps_last += 1;
                self.wave_step_requested = false;
            }
            if self.wave_running {
                let elapsed = delta_seconds.clamp(0.0, 0.1);
                self.wave_active_wall_seconds += elapsed;
                self.wave_accumulator += elapsed * self.wave_speed;
                let requested = (self.wave_accumulator / self.wave_time_step).floor() as u64;
                let requested = requested.min(16);
                if requested > 0 {
                    request.request_steps(requested);
                    self.wave_accumulator -= requested as f64 * self.wave_time_step;
                    self.wave_substeps_last += requested;
                }
                self.wave_accumulator = self.wave_accumulator.min(16.0 * self.wave_time_step);
            }
        }
        let scheduled_rate = if delta_seconds > 1.0e-6 {
            self.wave_substeps_last as f64 / delta_seconds
        } else {
            0.0
        };
        let observed_rate = if completed_delta > 0 {
            completed_rate
        } else {
            scheduled_rate
        };
        // Measure solver work itself rather than the UI's Run flag.  A GPU
        // request can finish after a handoff pauses scheduling, and a single
        // manually requested step should still be visible in diagnostics.
        // With no work the exponential smoothing decays the value to zero.
        self.wave_steps_per_second = 0.85 * self.wave_steps_per_second + 0.15 * observed_rate;
        if display.generation == request.generation()
            && display.current.len()
                == self
                    .wave_operator
                    .as_ref()
                    .map_or(0, |operator| operator.degrees_of_freedom())
            && display.completed_steps != self.wave_energy_step
            && let Some(operator) = &self.wave_operator
        {
            let current: Vec<_> = display.current.iter().map(|value| *value as f64).collect();
            let previous: Vec<_> = display.previous.iter().map(|value| *value as f64).collect();
            let auxiliary: Vec<_> = display
                .auxiliary
                .iter()
                .map(|value| *value as f64)
                .collect();
            match operator.discrete_energy_with_auxiliary(
                &current,
                &previous,
                &auxiliary,
                self.wave_time_step,
            ) {
                Ok(energy) => {
                    self.wave_energy = Some(energy);
                    self.wave_energy_step = display.completed_steps;
                }
                Err(error) => {
                    self.wave_running = false;
                    self.wave_error = Some(format!("GPU field validation failed: {error}"));
                }
            }
        }
    }
    fn select_inspector_panel(&mut self, panel: InspectorPanel) {
        if self.inspector_panel == Some(panel) {
            self.inspector_panel = None;
            return;
        }
        self.inspector_panel = Some(panel);
    }

    fn save_scene(&mut self) {
        self.editor.commit();
        match persistence::save(&self.editor.document) {
            Ok(json) => {
                files::save(self.sender.clone(), json.into_bytes(), SaveKind::Scene);
                self.file_busy = true;
            }
            Err(error) => self.message = error,
        }
    }

    fn export_scene_svg(&mut self) {
        let svg = examples::scene_svg(&self.editor.document.accepted);
        files::save(self.sender.clone(), svg.into_bytes(), SaveKind::SceneSvg);
        self.file_busy = true;
    }

    fn copy_scene_link(&mut self, context: &egui::Context) {
        match sharing::encode(&self.editor.document).and_then(|fragment| sharing::link(&fragment)) {
            Ok(url) => {
                self.share_fragment_active = true;
                context.copy_text(url);
                self.notify("Scene link copied");
            }
            Err(error) => self.message = error,
        }
    }

    fn load_scene(&mut self) {
        self.editor.commit();
        files::load(self.sender.clone());
        self.file_busy = true;
    }

    fn top_file_controls(&mut self, ui: &mut egui::Ui, compact: bool) {
        let (undo, redo) = self.editor.history_len();
        if ui
            .add_enabled(undo > 0, egui::Button::new("Undo"))
            .clicked()
        {
            self.editor.undo();
            self.clear_transient();
        }
        if ui
            .add_enabled(redo > 0, egui::Button::new("Redo"))
            .clicked()
        {
            self.editor.redo();
            self.clear_transient();
        }
        ui.separator();
        let file_enabled = !self.file_busy && self.load.is_none() && !self.automated_benchmark;
        if compact {
            ui.add_enabled_ui(file_enabled, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Save").clicked() {
                        self.save_scene();
                        ui.close();
                    }
                    if ui.button("Load").clicked() {
                        self.load_scene();
                        ui.close();
                    }
                    if ui.button("Examples").clicked() {
                        self.examples_open = true;
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Copy scene link").clicked() {
                        self.copy_scene_link(ui.ctx());
                        ui.close();
                    }
                    if ui.button("Export scene SVG").clicked() {
                        self.export_scene_svg();
                        ui.close();
                    }
                });
            });
        } else {
            if ui
                .add_enabled(file_enabled, egui::Button::new("Save"))
                .clicked()
            {
                self.save_scene();
            }
            if ui
                .add_enabled(file_enabled, egui::Button::new("Load"))
                .clicked()
            {
                self.load_scene();
            }
            if ui
                .add_enabled(file_enabled, egui::Button::new("Examples"))
                .clicked()
            {
                self.examples_open = true;
            }
            ui.add_enabled_ui(file_enabled, |ui| {
                ui.menu_button("Export", |ui| {
                    if ui.button("Copy scene link").clicked() {
                        self.copy_scene_link(ui.ctx());
                        ui.close();
                    }
                    if ui.button("Scene SVG").clicked() {
                        self.export_scene_svg();
                        ui.close();
                    }
                });
            });
        }
        if ui.button("Fit view").clicked() {
            self.fit = true;
        }
    }

    fn top_editor_controls(&mut self, ui: &mut egui::Ui, compact: bool) {
        if compact {
            ui.menu_button("Panels", |ui| {
                for panel in InspectorPanel::ALL {
                    if ui
                        .selectable_label(self.inspector_panel == Some(panel), panel.label())
                        .clicked()
                    {
                        self.select_inspector_panel(panel);
                        ui.close();
                    }
                }
            });
        } else {
            for panel in InspectorPanel::ALL {
                if ui
                    .selectable_label(self.inspector_panel == Some(panel), panel.label())
                    .clicked()
                {
                    self.select_inspector_panel(panel);
                }
            }
        }
        let drawing = matches!(
            self.interaction_mode,
            InteractionMode::DrawPreset { .. } | InteractionMode::DrawCustom { .. }
        );
        let draw_response =
            ui.add(egui::Button::new("+ Draw").selected(drawing || self.add_geometry_open));
        self.add_geometry_anchor = draw_response.rect.left_bottom() + egui::vec2(0.0, 4.0);
        if draw_response.clicked() {
            if drawing || self.add_geometry_open {
                self.interaction_mode = InteractionMode::Select;
                self.custom.clear();
                self.add_geometry_open = false;
            } else {
                self.inspector_panel = Some(InspectorPanel::Edit);
                self.add_geometry_open = true;
            }
        }
    }

    fn top_playback_controls(&mut self, ui: &mut egui::Ui) {
        let wave_available = self.wave_operator.is_some();
        ui.add_enabled_ui(wave_available, |ui| {
            if ui.button("Reset").clicked() {
                self.wave_reset_requested = true;
            }
            if ui
                .add_enabled(!self.wave_running, egui::Button::new("Step"))
                .on_hover_text("Pause the simulation to advance one solver step")
                .clicked()
            {
                self.wave_step_requested = true;
            }
            if ui
                .button(if self.wave_running { "Pause" } else { "Run" })
                .clicked()
            {
                self.wave_running = !self.wave_running;
            }
        });
        ui.separator();
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let width = ui.available_width();
        let compact_files = width < 1100.0;
        let compact_panels = width < 950.0;
        ui.horizontal_centered(|ui| {
            self.top_file_controls(ui, compact_files);
            ui.separator();
            self.top_editor_controls(ui, compact_panels);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.top_playback_controls(ui);
            });
        });
    }

    fn add_geometry_popover(&mut self, ctx: &egui::Context) {
        if !self.add_geometry_open {
            return;
        }
        let mut open = true;
        let mut close = false;
        egui::Window::new("Draw geometry")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .fixed_pos(self.add_geometry_anchor)
            .default_width(250.0)
            .show(ctx, |ui| {
                ui.small("Choose a role, then place a primitive in the viewport.");
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    for (role, label) in [
                        (CreationRole::Hole, "Hole"),
                        (CreationRole::MaterialInterface, "Interface"),
                        (CreationRole::InternalBoundary, "Baffle"),
                    ] {
                        ui.selectable_value(&mut self.creation_role, role, label);
                    }
                });
                if self.creation_role == CreationRole::MaterialInterface {
                    let materials = self.editor.document.draft.materials.clone();
                    egui::ComboBox::from_label("Interior material")
                        .selected_text(
                            self.editor
                                .document
                                .draft
                                .material(self.material_selection)
                                .map_or("Missing", |material| material.name.as_str()),
                        )
                        .show_ui(ui, |ui| {
                            for material in materials {
                                ui.selectable_value(
                                    &mut self.material_selection,
                                    material.id,
                                    material.name,
                                );
                            }
                        });
                }
                ui.separator();
                ui.label("Primitive");
                ui.horizontal(|ui| {
                    let primitive_label = if self.creation_role == CreationRole::InternalBoundary {
                        "Straight"
                    } else {
                        "Circle"
                    };
                    if ui.button(primitive_label).clicked() {
                        self.interaction_mode = InteractionMode::DrawPreset {
                            role: self.creation_role,
                        };
                        close = true;
                    }
                    if ui.button("Custom").clicked() {
                        self.interaction_mode = InteractionMode::DrawCustom {
                            role: self.creation_role,
                        };
                        self.custom.clear();
                        close = true;
                    }
                });
            });
        if close {
            open = false;
        }
        self.add_geometry_open = open;
    }

    fn example_gallery(&mut self, context: &egui::Context) {
        if !self.examples_open {
            return;
        }
        let mut open = true;
        let mut selected = None;
        egui::Window::new("Examples")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .show(context, |ui| {
                ui.label("Choose a ready-to-run scene.");
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(540.0)
                    .show(ui, |ui| {
                        for (index, example) in examples::catalog().iter().enumerate() {
                            ui.horizontal(|ui| {
                                let preview =
                                    paint_example_thumbnail(ui, example, egui::vec2(144.0, 144.0));
                                ui.vertical(|ui| {
                                    ui.heading(example.name);
                                    ui.set_max_width(320.0);
                                    ui.label(example.description);
                                    ui.add_space(8.0);
                                    if ui.button("Open example").clicked() || preview.clicked() {
                                        selected = Some(index);
                                    }
                                });
                            });
                            if index + 1 < examples::catalog().len() {
                                ui.add_space(8.0);
                                ui.separator();
                                ui.add_space(8.0);
                            }
                        }
                    });
            });
        self.examples_open = open;
        if let Some(index) = selected {
            let example = &examples::catalog()[index];
            self.start_example_load(example);
            self.examples_open = false;
        }
    }

    fn performance_warning(&self) -> bool {
        self.mesh_error.is_some() || self.wave_error.is_some()
    }

    fn performance_summary(&self) -> String {
        let fps = if self.frame_ms > 0.0 {
            1000.0 / self.frame_ms
        } else {
            0.0
        };
        let steps_per_second = self.wave_steps_per_second;
        let dofs = self.wave_operator.as_ref().map_or_else(
            || "—".into(),
            |operator| operator.degrees_of_freedom().to_string(),
        );
        let mesh = self.mesh.as_ref().map_or_else(
            || "—".into(),
            |mesh| format!("{}v/{}t", mesh.vertices.len(), mesh.triangles.len()),
        );
        let dt = if self.wave_time_step > 0.0 {
            format!("{:.2e}", self.wave_time_step)
        } else {
            "—".into()
        };
        format!(
            "FPS {fps:.0} · steps/s {steps_per_second:.1} · DOFs {dofs} · mesh {mesh} · dt {dt}"
        )
    }

    fn compact_performance_summary(&self) -> String {
        let fps = if self.frame_ms > 0.0 {
            1000.0 / self.frame_ms
        } else {
            0.0
        };
        let dofs = self.wave_operator.as_ref().map_or_else(
            || "—".into(),
            |operator| operator.degrees_of_freedom().to_string(),
        );
        format!(
            "FPS {fps:.0} · steps/s {:.1} · DOFs {dofs}",
            self.wave_steps_per_second
        )
    }

    fn performance_window(&mut self, ctx: &egui::Context) {
        let warning = self.performance_warning();
        if warning && !self.performance_warning_active {
            self.performance_open = true;
        }
        self.performance_warning_active = warning;
        if self.frame_ms.is_finite() && self.frame_ms > 0.0 {
            if self.performance_history.len() == 90 {
                self.performance_history.pop_front();
            }
            self.performance_history.push_back(self.frame_ms);
        }
        if !self.performance_open {
            return;
        }
        let mut open = true;
        egui::Window::new("Performance diagnostics")
            .open(&mut open)
            .default_width(390.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.monospace(self.performance_summary());
                ui.separator();
                egui::CollapsingHeader::new("Frame")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.label(format!(
                            "Frame {:.2} ms · {:.0} px/unit",
                            self.frame_ms, self.scale
                        ));
                        let recent_max = self
                            .performance_history
                            .iter()
                            .copied()
                            .fold(0.0_f32, f32::max);
                        let average = self.performance_history.iter().sum::<f32>()
                            / self.performance_history.len().max(1) as f32;
                        let mut ordered = self.performance_history.iter().copied().collect::<Vec<_>>();
                        ordered.sort_by(f32::total_cmp);
                        let p95 = ordered
                            .get(((ordered.len() as f32 * 0.95).ceil() as usize).saturating_sub(1))
                            .copied()
                            .unwrap_or(0.0);
                        ui.small(format!(
                            "Average {:.2} ms · p95 {:.2} ms · peak {:.2} ms · {} samples",
                            average,
                            p95,
                            recent_max,
                            self.performance_history.len()
                        ));
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 46.0),
                            egui::Sense::hover(),
                        );
                        if self.performance_history.len() > 1 && recent_max > 0.0 {
                            let points = self
                                .performance_history
                                .iter()
                                .enumerate()
                                .map(|(index, value)| {
                                    egui::pos2(
                                        egui::lerp(
                                            rect.left()..=rect.right(),
                                            index as f32
                                                / (self.performance_history.len() - 1) as f32,
                                        ),
                                        rect.bottom() - rect.height() * (*value / recent_max),
                                    )
                                })
                                .collect();
                            ui.painter().rect_stroke(
                                rect,
                                2.0,
                                Stroke::new(1.0, Color32::from_rgb(55, 69, 80)),
                                egui::StrokeKind::Inside,
                            );
                            ui.painter().add(egui::Shape::line(
                                points,
                                Stroke::new(1.5, TEAL),
                            ));
                        }
                    });
                egui::CollapsingHeader::new("Mesh")
                    .default_open(true)
                    .show(ui, |ui| {
                        if let Some(mesh) = &self.mesh {
                            ui.label(format!(
                                "{} vertices · {} triangles",
                                mesh.vertices.len(),
                                mesh.triangles.len()
                            ));
                            ui.small(format!(
                                "Minimum angle {:.1}° · maximum edge {:.3}",
                                mesh.quality.minimum_angle_degrees,
                                mesh.quality.maximum_edge_length
                            ));
                            let low_quality =
                                self.mesh_low_quality.iter().filter(|poor| **poor).count();
                            ui.small(format!("{low_quality} elements below 15°"));
                        } else {
                            ui.label("No committed mesh");
                        }
                        ui.small(format!(
                            "Build {:.1} ms · work {:.1} ms · longest slice {:.2} ms",
                            self.mesh_build_ms, self.mesh_work_ms, self.mesh_max_slice_ms
                        ));
                        ui.small(format!(
                            "AMR {} · indicator work {:.1} ms · longest slice {:.2} ms",
                            self.amr_status, self.amr_work_ms, self.amr_max_slice_ms
                        ));
                        if let Some(report) = &self.solution_indicator_report {
                            ui.small(format!(
                                "Indicator {:.3e}–{:.3e} · target {:.3}–{:.3}",
                                report.minimum_indicator,
                                report.maximum_indicator,
                                report.minimum_target,
                                report.maximum_target
                            ));
                            ui.small(format!(
                                "Refine candidates {} · coarsen candidates {} · {} work units",
                                report.refine_candidates,
                                report.coarsen_candidates,
                                report.work_units
                            ));
                            let total = report.recovery_contribution
                                + report.cell_residual_contribution
                                + report.interior_jump_contribution
                                + report.boundary_residual_contribution;
                            let percent = |value: f64| {
                                if total > f64::MIN_POSITIVE {
                                    100.0 * value / total
                                } else {
                                    0.0
                                }
                            };
                            ui.small(format!(
                                "Error share: recovery {:.0}% · cell {:.0}% · interior {:.0}% · boundary {:.0}%",
                                percent(report.recovery_contribution),
                                percent(report.cell_residual_contribution),
                                percent(report.interior_jump_contribution),
                                percent(report.boundary_residual_contribution),
                            ));
                            ui.small(format!(
                                "Boundary edges {} · Dirichlet mismatch {:.2e}",
                                report.boundary_edges_evaluated,
                                report.maximum_dirichlet_mismatch
                            ));
                        }
                        if let Some(report) = &self.mesh_adaptation_report {
                            let refinement_changes = report
                                .topology_changes
                                .saturating_sub(report.coarsening_changes);
                            ui.small(format!(
                                "Last adaptation: {} refinement changes · {} coarsening changes",
                                refinement_changes, report.coarsening_changes
                            ));
                            ui.small(format!(
                                "Collapse results: {} accepted · {} rejected",
                                report.coarsening_changes, report.skipped_collapses
                            ));
                        }
                        ui.small(format!(
                            "Completed edits {} · full rebuild fallbacks {}",
                            self.mesh_attempts, self.mesh_fallbacks
                        ));
                        if let Some(report) = self
                            .mesh_job
                            .as_ref()
                            .map(|job| job.report())
                            .or(self.mesh_report.as_ref())
                        {
                            if report.local_attempted && report.repair_attempts > 0 {
                                ui.small(format!(
                                    "Local repair attempts {} · final patch {} vertices / {} triangles",
                                    report.repair_attempts,
                                    report.repair_vertices,
                                    report.repair_triangles
                                ));
                                ui.small(format!(
                                    "Moved {} · inserted {} · collapsed {}",
                                    report.moved_vertices,
                                    report.inserted_vertices,
                                    report.collapsed_vertices
                                ));
                                if report.repaired_baffles > 0 {
                                    ui.small(format!(
                                        "Baffles {} · paired trace segments {}",
                                        report.repaired_baffles,
                                        report.paired_trace_segments
                                    ));
                                }
                            }
                            if report.used_local {
                                ui.small(format!(
                                    "Reuse {:.1}% unchanged · {:.1}% connectivity retained",
                                    100.0 * report.preserved_triangles as f64
                                        / report.original_triangles.max(1) as f64,
                                    100.0 * report.preserved_connectivity as f64
                                        / report.original_triangles.max(1) as f64,
                                ));
                            }
                            for (index, failure) in report.retry_failures.iter().enumerate() {
                                ui.small(format!(
                                    "Retry {}: {}",
                                    index + 1,
                                    failure.kind.label()
                                ));
                            }
                            if let Some(failure) = &report.fallback_failure {
                                ui.colored_label(
                                    GOLD,
                                    format!("Full rebuild: {}", failure.kind.label()),
                                )
                                .on_hover_text(&failure.detail);
                            }
                        }
                        if !self.mesh_fallback_causes.is_empty() {
                            ui.small("Fallback causes this session:");
                            for (kind, count) in &self.mesh_fallback_causes {
                                ui.small(format!("{} · {count}", kind.label()));
                            }
                        }
                        if let Some(error) = &self.mesh_error {
                            ui.colored_label(RED, error);
                        }
                    });
                egui::CollapsingHeader::new("Handoff")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.label(format!(
                            "Operator/transfer preparation {:.1} ms",
                            self.wave_prepare_ms
                        ));
                        ui.small(
                            "The previous committed field remains active until the candidate is ready.",
                        );
                        ui.small(if self.simulation_candidate.is_some() {
                            "Candidate handoff in progress…"
                        } else {
                            "No candidate handoff pending."
                        });
                    });
                egui::CollapsingHeader::new("Solver")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.label(format!("GPU {}", self.wave_gpu_status));
                        if let Some(operator) = &self.wave_operator {
                            ui.small(format!(
                                "{} DOFs · {:.2} MiB",
                                operator.degrees_of_freedom(),
                                operator.estimated_gpu_bytes() as f64 / (1024.0 * 1024.0)
                            ));
                            ui.small(format!(
                                "dt {:.6} · {:.1} completed steps/s · {} substeps/frame",
                                self.wave_time_step,
                                self.wave_steps_per_second,
                                self.wave_substeps_last
                            ));
                            let simulated_time = self.wave_time_offset
                                + self.wave_completed_steps as f64 * self.wave_time_step;
                            let throughput = if self.wave_active_wall_seconds > 0.0 {
                                simulated_time / self.wave_active_wall_seconds
                            } else {
                                0.0
                            };
                            ui.small(format!("{throughput:.2} simulated s / wall s"));
                            if let Some(energy) = self.wave_energy {
                                ui.small(format!("Discrete energy {energy:.6e}"));
                            }
                        } else {
                            ui.small("Waiting for an accepted mesh and wave operator.");
                        }
                        if let Some(error) = &self.wave_error {
                            ui.colored_label(RED, error);
                        }
                    });
            });
        self.performance_open = open;
    }

    fn probe_readout_windows(&mut self, ctx: &egui::Context) {
        let open_ids = self.probe_windows.iter().copied().collect::<Vec<_>>();
        for id in open_ids {
            let Some(probe) = self
                .editor
                .document
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
                .map(|trace| trace.frames.iter().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            let area_samples = self
                .area_probe_traces
                .get(&id)
                .map(|trace| trace.samples.iter().copied().collect::<Vec<_>>())
                .unwrap_or_default();
            let is_area = matches!(
                probe.target,
                ProbeTarget::AreaDisk { .. } | ProbeTarget::AreaRegion { .. }
            );
            let curve_metric = match probe.target {
                ProbeTarget::Point(_) => None,
                ProbeTarget::Segment { start, end, .. } => Some(((end - start).norm(), false)),
                ProbeTarget::Boundary(target) => {
                    self.curve_probe_metrics.get(&id).copied().or_else(|| {
                        Self::boundary_probe_path(&self.editor.document.draft, target)
                            .map(|path| (path.length, path.closed))
                    })
                }
                ProbeTarget::AreaDisk { .. } | ProbeTarget::AreaRegion { .. } => None,
            };
            let segment_length = curve_metric.map(|metric| metric.0);
            let curve_closed = curve_metric.is_some_and(|metric| metric.1);
            let curve_times = curve_metric.map_or_else(Vec::new, |_| {
                curve_frames
                    .iter()
                    .map(|frame| PointProbeRecord {
                        probe_id: frame.probe_id,
                        time: frame.time,
                        displacement: 0.0,
                        velocity: 0.0,
                        energy_density: 0.0,
                    })
                    .collect::<Vec<_>>()
            });
            let status = self.probe_status.get(&id).cloned();
            let mut view = self
                .probe_views
                .remove(&id)
                .unwrap_or_else(|| ProbeViewState::new(self.probe_history_seconds));
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
            view.span = view.span.clamp(0.02, self.probe_history_seconds);
            let mut open = true;
            let mut clear = false;
            let kind = match probe.target {
                ProbeTarget::Point(_) => "point probe",
                ProbeTarget::Segment { .. } => "line probe",
                ProbeTarget::Boundary(_) => "boundary probe",
                ProbeTarget::AreaDisk { .. } | ProbeTarget::AreaRegion { .. } => "area probe",
            };
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
                        if segment_length.is_some() {
                            let active = view.line_plots.iter().filter(|enabled| **enabled).count();
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
                                                ui.label(quantity.label());
                                                for representation in LineProbeRepresentation::ALL {
                                                    let index =
                                                        quantity.offset() + representation.offset();
                                                    ui.checkbox(&mut view.line_plots[index], "")
                                                        .on_hover_text(format!(
                                                            "{} {}",
                                                            quantity.label(),
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
                            let active = [
                                view.area_mean_field,
                                view.area_rms_field,
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
                                    ui.checkbox(&mut view.area_mean_field, "Mean field");
                                    ui.checkbox(&mut view.area_rms_field, "RMS field");
                                    ui.checkbox(&mut view.area_mean_energy, "Mean energy density");
                                    ui.checkbox(&mut view.area_total_energy, "Total energy");
                                });
                        } else {
                            ui.checkbox(&mut view.field, "Field");
                            ui.checkbox(&mut view.velocity, "Velocity");
                            ui.checkbox(&mut view.energy, "Energy");
                        }
                        if ui.small_button("Clear").clicked() {
                            clear = true;
                        }
                    });
                    if let Some(status) = &status {
                        ui.colored_label(GOLD, status);
                    }
                    if segment_length.is_some() {
                        if let Some(frame) = curve_frames.last() {
                            let (_, coverage) = Self::curve_probe_integral(
                                frame,
                                segment_length.unwrap_or_default(),
                                LineProbeQuantity::Field,
                                curve_closed,
                            );
                            if coverage < 0.999 {
                                ui.small(format!("Valid coverage {:.0}%", coverage * 100.0));
                            }
                        }
                        let length = segment_length.unwrap_or_default();
                        for quantity in LineProbeQuantity::ALL {
                            if view.line_plots
                                [quantity.offset() + LineProbeRepresentation::Arclength.offset()]
                            {
                                Self::curve_probe_profile(
                                    ui,
                                    &curve_frames,
                                    &view,
                                    quantity,
                                    length,
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
                                    self.probe_history_seconds,
                                    quantity,
                                );
                            }
                            if view.line_plots
                                [quantity.offset() + LineProbeRepresentation::Integral.offset()]
                            {
                                let history = curve_frames
                                    .iter()
                                    .map(|frame| PointProbeRecord {
                                        probe_id: frame.probe_id,
                                        time: frame.time,
                                        displacement: Self::curve_probe_integral(
                                            frame,
                                            length,
                                            quantity,
                                            curve_closed,
                                        )
                                        .0,
                                        velocity: 0.0,
                                        energy_density: 0.0,
                                    })
                                    .collect::<Vec<_>>();
                                Self::probe_plot(
                                    ui,
                                    &format!("{} ∫ ds", quantity.label()),
                                    &history,
                                    |sample| sample.displacement,
                                    quantity.color(),
                                    &mut view,
                                    self.probe_history_seconds,
                                );
                            }
                        }
                    } else if !is_area && view.field {
                        Self::probe_plot(
                            ui,
                            "Field",
                            &point_samples,
                            |sample| sample.displacement,
                            Color32::from_rgb(72, 166, 255),
                            &mut view,
                            self.probe_history_seconds,
                        );
                    }
                    if segment_length.is_none() && !is_area && view.velocity {
                        Self::probe_plot(
                            ui,
                            "Velocity",
                            &point_samples,
                            |sample| sample.velocity,
                            Color32::from_rgb(91, 220, 194),
                            &mut view,
                            self.probe_history_seconds,
                        );
                    }
                    if segment_length.is_none() && !is_area && view.energy {
                        Self::probe_plot(
                            ui,
                            "Local energy density",
                            &point_samples,
                            |sample| sample.energy_density,
                            GOLD,
                            &mut view,
                            self.probe_history_seconds,
                        );
                    }
                    if is_area {
                        if let Some(sample) = area_samples.last()
                            && sample.coverage < 0.999
                        {
                            ui.small(format!(
                                "Covered {:.0}% of the target",
                                sample.coverage * 100.0
                            ));
                        }
                        let history = |value: fn(&AreaProbeRecord) -> f64| {
                            area_samples
                                .iter()
                                .map(|sample| PointProbeRecord {
                                    probe_id: sample.probe_id,
                                    time: sample.time,
                                    displacement: value(sample),
                                    velocity: 0.0,
                                    energy_density: 0.0,
                                })
                                .collect::<Vec<_>>()
                        };
                        if view.area_mean_field {
                            Self::probe_plot(
                                ui,
                                "Mean field",
                                &history(|s| s.mean_displacement),
                                |s| s.displacement,
                                SELECT,
                                &mut view,
                                self.probe_history_seconds,
                            );
                        }
                        if view.area_rms_field {
                            Self::probe_plot(
                                ui,
                                "RMS field",
                                &history(|s| s.rms_displacement),
                                |s| s.displacement,
                                TEAL,
                                &mut view,
                                self.probe_history_seconds,
                            );
                        }
                        if view.area_mean_energy {
                            Self::probe_plot(
                                ui,
                                "Mean energy density",
                                &history(|s| s.mean_energy_density),
                                |s| s.displacement,
                                GOLD,
                                &mut view,
                                self.probe_history_seconds,
                            );
                        }
                        if view.area_total_energy {
                            Self::probe_plot(
                                ui,
                                "Total energy",
                                &history(|s| s.total_energy),
                                |s| s.displacement,
                                RED,
                                &mut view,
                                self.probe_history_seconds,
                            );
                        }
                    }
                    ui.small(if segment_length.is_some() {
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
    }

    fn far_field_readout_window(&mut self, ctx: &egui::Context) {
        if !self.far_field_open {
            return;
        }
        let frames = self
            .far_field_trace
            .frames
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let times = frames
            .iter()
            .map(|frame| PointProbeRecord {
                probe_id: 0,
                time: frame.time,
                displacement: 0.0,
                velocity: 0.0,
                energy_density: 0.0,
            })
            .collect::<Vec<_>>();
        let energy = frames
            .iter()
            .map(|frame| PointProbeRecord {
                probe_id: 0,
                time: frame.time,
                displacement: frame
                    .intensity
                    .iter()
                    .map(|value| *value as f64)
                    .sum::<f64>()
                    * std::f64::consts::TAU
                    / FAR_FIELD_DIRECTIONS as f64,
                velocity: 0.0,
                energy_density: 0.0,
            })
            .collect::<Vec<_>>();
        let newest_time = frames.last().map(|frame| frame.time);
        if self.far_field_view.live
            && let Some(time) = newest_time
        {
            self.far_field_view.end_time = time;
        }
        self.far_field_view.span = self
            .far_field_view
            .span
            .clamp(0.02, self.probe_history_seconds);
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
                    if ui.button("Clear").clicked() {
                        clear = true;
                    }
                    ui.add(
                        egui::Slider::new(&mut self.far_field_view.waterfall_gain, 0.2..=5.0)
                            .logarithmic(true)
                            .text("gain"),
                    );
                });
                if let Some(status) = &self.far_field_status {
                    ui.colored_label(RED, status);
                    return;
                }
                Self::far_field_waterfall(
                    ui,
                    &frames,
                    &times,
                    &mut self.far_field_view,
                    self.probe_history_seconds,
                );
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
                    Self::far_field_polar(
                        &mut columns[0],
                        "Instantaneous",
                        &instantaneous,
                    );
                    Self::far_field_polar(
                        &mut columns[1],
                        "Time-averaged",
                        &averaged,
                    );
                });
                Self::probe_plot(
                    ui,
                    "Angular energy",
                    &energy,
                    |sample| sample.displacement,
                    GOLD,
                    &mut self.far_field_view,
                    self.probe_history_seconds,
                );
                ui.small(
                    "Drag through time · wheel to zoom · average follows the visible window · polar scale spans 40 dB",
                );
            });
        if clear {
            let time = self
                .far_field_trace
                .frames
                .back()
                .map_or(0.0, |frame| frame.time);
            self.far_field_trace.frames.clear();
            self.far_field_trace.accept_after = time;
        }
        self.far_field_open = open;
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
            let top = egui::lerp(
                rect.bottom()..=rect.top(),
                (row + 1) as f32 / visible.len() as f32,
            );
            let bottom = egui::lerp(
                rect.bottom()..=rect.top(),
                row as f32 / visible.len() as f32,
            );
            for (column, value) in frame.amplitude.iter().copied().enumerate() {
                let normalized = (value / maximum).clamp(-1.0, 1.0);
                let color = if normalized >= 0.0 {
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
                };
                let left = egui::lerp(
                    rect.left()..=rect.right(),
                    column as f32 / FAR_FIELD_DIRECTIONS as f32,
                );
                let right = egui::lerp(
                    rect.left()..=rect.right(),
                    (column + 1) as f32 / FAR_FIELD_DIRECTIONS as f32,
                );
                ui.painter().rect_filled(
                    Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, bottom)),
                    0.0,
                    color,
                );
            }
        }
        for (x, label) in [(0.0, "0°"), (0.25, "90°"), (0.5, "180°"), (0.75, "270°")] {
            ui.painter().text(
                egui::pos2(
                    egui::lerp(rect.left()..=rect.right(), x),
                    rect.bottom() - 3.0,
                ),
                egui::Align2::LEFT_BOTTOM,
                label,
                egui::FontId::monospace(9.0),
                Color32::WHITE,
            );
        }
        ui.painter().text(
            rect.left_top() + egui::vec2(4.0, 3.0),
            egui::Align2::LEFT_TOP,
            format!("t={maximum_time:.3}"),
            egui::FontId::monospace(9.0),
            Color32::WHITE,
        );
        ui.painter().text(
            rect.left_bottom() + egui::vec2(4.0, -15.0),
            egui::Align2::LEFT_BOTTOM,
            format!("t={minimum_time:.3}"),
            egui::FontId::monospace(9.0),
            Color32::WHITE,
        );
    }

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

    fn curve_probe_values(frame: &CurveProbeRecord, quantity: LineProbeQuantity) -> &[f32] {
        match quantity {
            LineProbeQuantity::Field => &frame.displacement,
            LineProbeQuantity::Flux => &frame.normal_flux,
            LineProbeQuantity::Energy => &frame.energy_density,
        }
    }

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
    ) {
        ui.small(format!("{} vs arclength", quantity.label()));
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
    ) {
        ui.small(format!("{} waterfall", quantity.label()));
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().max(120.0), 170.0),
            egui::Sense::drag(),
        );
        ui.painter()
            .rect_filled(rect, 2.0, Color32::from_rgb(12, 18, 24));
        let Some((minimum_time, maximum_time)) = Self::probe_time_window(times, view) else {
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
                let color = if quantity == LineProbeQuantity::Energy {
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
                };
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
        let Some((mut minimum_time, mut maximum_time)) = Self::probe_time_window(samples, view)
        else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Waiting for samples",
                egui::FontId::monospace(11.0),
                Color32::from_rgb(112, 130, 143),
            );
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
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Waiting for samples",
                egui::FontId::monospace(11.0),
                Color32::from_rgb(112, 130, 143),
            );
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
            "Simulation time {:.4}–{:.4}",
            minimum_time, maximum_time
        ));
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

    fn panel(&mut self, ui: &mut egui::Ui) {
        self.reconcile_selection();
        if self.automated_benchmark {
            ui.label("Automated mesh benchmark");
            ui.disable();
        }
        match self.inspector_panel {
            Some(InspectorPanel::View) => self.view_panel(ui),
            Some(InspectorPanel::Simulation) => self.simulation_panel(ui),
            Some(InspectorPanel::Materials) => self.materials_panel(ui),
            Some(InspectorPanel::Probes) => self.probes_panel(ui),
            Some(InspectorPanel::Edit) | None => self.edit_panel(ui),
        }
    }

    fn panel_header(&mut self, ui: &mut egui::Ui, title: &str) {
        ui.horizontal(|ui| {
            ui.heading(title);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("×")
                    .on_hover_text("Close inspector")
                    .clicked()
                {
                    self.inspector_panel = None;
                }
            });
        });
    }

    fn view_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        self.panel_header(ui, "View");
        ui.separator();
        ui.checkbox(&mut self.grid, "Grid");
        ui.checkbox(&mut self.polygon, "Control polygons");
        ui.checkbox(&mut self.handles, "Handles");
        ui.checkbox(&mut self.reference, "Accepted reference");
        ui.checkbox(&mut self.show_materials, "Material regions");
        ui.checkbox(&mut self.show_boundary_conditions, "Boundary conditions");
        if self.show_boundary_conditions {
            for (label, color) in [
                (
                    "Reflecting",
                    boundary_condition_color(FaceBoundaryCondition::Reflecting),
                ),
                (
                    "First-order / impedance",
                    boundary_condition_color(FaceBoundaryCondition::Impedance { ratio: 1.0 }),
                ),
                (
                    "Second-order",
                    boundary_condition_color(FaceBoundaryCondition::SecondOrderOutgoing),
                ),
                (
                    "Driven Neumann",
                    boundary_condition_color(FaceBoundaryCondition::Neumann {
                        signal: BoundarySignal::ZERO,
                    }),
                ),
                (
                    "Driven Dirichlet",
                    boundary_condition_color(FaceBoundaryCondition::Dirichlet {
                        signal: BoundarySignal::ZERO,
                    }),
                ),
                ("Thin gap", Color32::from_rgb(215, 123, 244)),
            ] {
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(18.0, 8.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, color);
                    ui.small(label);
                });
            }
        }
        ui.checkbox(&mut self.show_mesh, "Accepted triangle mesh");
        ui.add_enabled_ui(self.show_mesh, |ui| {
            ui.checkbox(&mut self.show_mesh_boundary, "Mesh boundary labels");
        });
        ui.checkbox(&mut self.show_amr_target, "Adaptation target");
        ui.checkbox(&mut self.show_point_probes, "Point probes");
        ui.checkbox(&mut self.show_line_probes, "Line probes");
        ui.checkbox(&mut self.show_boundary_probes, "Boundary probes");
        ui.checkbox(&mut self.show_area_probes, "Area probes");
        ui.checkbox(&mut self.show_far_field_contour, "Far-field contour");
        ui.checkbox(&mut self.show_field, "Field colors");
        ui.add_enabled_ui(self.show_field, |ui| {
            ui.add(
                egui::Slider::new(&mut self.field_gain, 0.25..=12.0)
                    .logarithmic(true)
                    .text("field intensity"),
            );
            let width = ui.available_width().max(60.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 10.0), egui::Sense::hover());
            for index in 0..64 {
                let fraction = index as f32 / 63.0;
                let value = 2.0 * fraction - 1.0;
                let strip = Rect::from_min_max(
                    egui::pos2(rect.left() + rect.width() * index as f32 / 64.0, rect.top()),
                    egui::pos2(
                        rect.left() + rect.width() * (index + 1) as f32 / 64.0,
                        rect.bottom(),
                    ),
                );
                ui.painter()
                    .rect_filled(strip, 0.0, field_color(value, 1.0));
            }
            ui.horizontal(|ui| {
                ui.small("negative");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.small("positive");
                });
            });
        });
        ui.add_space(8.0);
        if ui.button("Restore view defaults").clicked() {
            self.grid = true;
            self.polygon = true;
            self.handles = true;
            self.reference = true;
            self.show_materials = true;
            self.show_boundary_conditions = false;
            self.show_mesh = false;
            self.show_mesh_boundary = true;
            self.show_amr_target = false;
            self.show_point_probes = true;
            self.show_line_probes = true;
            self.show_boundary_probes = true;
            self.show_area_probes = true;
            self.show_far_field_contour = true;
            self.show_field = true;
            self.field_gain = 2.0;
        }
    }

    fn materials_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        self.panel_header(ui, "Materials");
        ui.label("Subdomain assignment");
        let materials = self.editor.document.draft.materials.clone();
        let regions = self.editor.document.draft.regions.clone();
        for region in regions {
            let label = if region.id == BACKGROUND_REGION {
                "Background".into()
            } else {
                self.editor
                    .document
                    .draft
                    .obstacles
                    .iter()
                    .find(|loop_| loop_.role.interior() == Some(region.id))
                    .map_or_else(
                        || format!("Region {}", region.id.0),
                        |loop_| format!("{} {} interior", loop_.role.label(), loop_.id.0),
                    )
            };
            let mut selected = region.material;
            ui.horizontal(|ui| {
                if ui
                    .add_sized(
                        [128.0, 22.0],
                        egui::Button::new(label).selected(self.region_selection == region.id),
                    )
                    .clicked()
                {
                    self.region_selection = region.id;
                }
                if let Some(material) = materials.iter().find(|material| material.id == selected) {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                    ui.painter().rect_filled(
                        rect,
                        2.0,
                        Color32::from_rgb(material.color[0], material.color[1], material.color[2]),
                    );
                }
                egui::ComboBox::from_id_salt(("region_material", region.id.0))
                    .width(ui.available_width())
                    .selected_text(
                        materials
                            .iter()
                            .find(|material| material.id == selected)
                            .map_or("Missing", |material| material.name.as_str()),
                    )
                    .show_ui(ui, |ui| {
                        for material in &materials {
                            ui.selectable_value(&mut selected, material.id, &material.name);
                        }
                    });
            });
            if selected != region.material {
                let result = self.editor.set_region_material(region.id, selected);
                self.error(result);
            }
        }
        ui.separator();
        ui.label("Library");
        ui.horizontal(|ui| {
            if ui.button("+ Material").clicked() {
                let result = self.editor.add_material();
                if let Some(id) = self.error(result) {
                    self.material_selection = id;
                }
            }
        });
        ui.horizontal(|ui| {
            if let Some(material) = materials
                .iter()
                .find(|material| material.id == self.material_selection)
            {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                ui.painter().rect_filled(
                    rect,
                    2.0,
                    Color32::from_rgb(material.color[0], material.color[1], material.color[2]),
                );
            }
            egui::ComboBox::from_id_salt("material_editor")
                .width(ui.available_width())
                .selected_text(
                    materials
                        .iter()
                        .find(|material| material.id == self.material_selection)
                        .map_or("Missing", |material| material.name.as_str()),
                )
                .show_ui(ui, |ui| {
                    for material in &materials {
                        ui.selectable_value(
                            &mut self.material_selection,
                            material.id,
                            &material.name,
                        );
                    }
                });
        });
        if let Some(mut material) = self
            .editor
            .document
            .draft
            .material(self.material_selection)
            .cloned()
        {
            if !matches!(
                self.material_name_edit.as_ref(),
                Some((id, _)) if *id == material.id
            ) {
                self.material_name_edit = Some((material.id, material.name.clone()));
            }
            let mut edited_name = None;
            let mut name_lost_focus = false;
            ui.horizontal(|ui| {
                ui.label("Name");
                if let Some((_, name)) = self.material_name_edit.as_mut() {
                    let response = ui.add(
                        egui::TextEdit::singleline(name)
                            .desired_width(ui.available_width())
                            .hint_text("Material name"),
                    );
                    if response.changed() {
                        edited_name = Some(name.clone());
                    }
                    name_lost_focus = response.lost_focus();
                }
            });
            if edited_name.is_some() {
                self.editor.begin();
            }
            if let Some(name) = edited_name
                && !name.trim().is_empty()
                && name.len() <= 64
            {
                material.name = name;
                let result = self.editor.update_material(material.clone());
                self.error(result);
            }
            if name_lost_focus {
                if let Some((_, name)) = self.material_name_edit.as_ref()
                    && (name.trim().is_empty() || name.len() > 64)
                {
                    self.material_name_edit = Some((material.id, material.name.clone()));
                }
                self.editor.commit();
                self.material_name_edit = None;
            }
            let responses = [
                ui.add(
                    egui::DragValue::new(&mut material.mass_density)
                        .speed(0.01)
                        .range(1.0e-6..=1.0e6)
                        .prefix("density ")
                        .update_while_editing(false),
                ),
                ui.add(
                    egui::DragValue::new(&mut material.stiffness)
                        .speed(0.01)
                        .range(1.0e-6..=1.0e6)
                        .prefix("stiffness ")
                        .update_while_editing(false),
                ),
                ui.add(
                    egui::DragValue::new(&mut material.damping)
                        .speed(0.005)
                        .range(0.0..=1.0e6)
                        .prefix("damping ")
                        .update_while_editing(false),
                ),
            ];
            ui.small("Nondimensional coefficients");
            if responses.iter().any(egui::Response::changed) {
                let result = self.editor.update_material(material.clone());
                self.error(result);
            }
            ui.small(format!(
                "wave speed {:.3}",
                (material.stiffness / material.mass_density).sqrt()
            ));
            if self.material_selection != DEFAULT_MATERIAL {
                let material_in_use = self
                    .editor
                    .document
                    .draft
                    .regions
                    .iter()
                    .any(|region| region.material == self.material_selection);
                let response =
                    ui.add_enabled(!material_in_use, egui::Button::new("Delete material"));
                if response.clicked() {
                    let result = self.editor.delete_material(self.material_selection);
                    if self.error(result).is_some() {
                        self.material_selection = DEFAULT_MATERIAL;
                        self.material_name_edit = None;
                    }
                }
                if material_in_use {
                    response.on_hover_text("Material is assigned to a subdomain");
                }
            }
        }
    }

    fn probes_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        self.panel_header(ui, "Probes");
        ui.horizontal_wrapped(|ui| {
            let placing = self.interaction_mode == InteractionMode::PlaceProbe;
            if ui
                .add(egui::Button::new("+ Point probe").selected(placing))
                .on_hover_text(
                    "Click repeatedly in the viewport; click again or press Esc to finish",
                )
                .clicked()
            {
                self.interaction_mode = if placing {
                    InteractionMode::Select
                } else {
                    InteractionMode::PlaceProbe
                };
            }
            let placing_line = self.interaction_mode == InteractionMode::PlaceSegmentProbe;
            if ui
                .add(egui::Button::new("+ Line probe").selected(placing_line))
                .on_hover_text("Click a start point and an end point; repeat to add more")
                .clicked()
            {
                self.segment_probe_start = None;
                self.interaction_mode = if placing_line {
                    InteractionMode::Select
                } else {
                    InteractionMode::PlaceSegmentProbe
                };
            }
            egui::containers::menu::MenuButton::new("+ Area probe").ui(ui, |ui| {
                let placing_disk = self.interaction_mode == InteractionMode::PlaceAreaDisk;
                if ui
                    .add(egui::Button::new("Disk").selected(placing_disk))
                    .clicked()
                {
                    self.area_probe_center = None;
                    self.interaction_mode = if placing_disk {
                        InteractionMode::Select
                    } else {
                        InteractionMode::PlaceAreaDisk
                    };
                    ui.close();
                }
                let placing_region = self.interaction_mode == InteractionMode::PlaceAreaRegion;
                if ui
                    .add(egui::Button::new("Subdomain").selected(placing_region))
                    .clicked()
                {
                    self.area_probe_center = None;
                    self.interaction_mode = if placing_region {
                        InteractionMode::Select
                    } else {
                        InteractionMode::PlaceAreaRegion
                    };
                    ui.close();
                }
            });
            if ui.button("Clear all").clicked() {
                for probe in self.editor.document.probes.clone() {
                    self.clear_probe_trace(probe.id);
                }
                self.far_field_trace = FarFieldTrace::default();
            }
        });
        let boundary_target = self.boundary_probe_target_from_selection();
        let response = ui.add_enabled(
            boundary_target.is_some(),
            egui::Button::new("+ From selected spans"),
        );
        if response.clicked()
            && let Some(target) = boundary_target
        {
            match self.editor.create_boundary_probe(target) {
                Ok(id) => {
                    self.select_probe(id);
                    self.probe_windows.insert(id);
                }
                Err(error) => {
                    self.error::<()>(Err(error));
                }
            }
        }
        response.on_hover_text(
            "Create a boundary probe from one contiguous span selection on one curve",
        );
        ui.add(
            egui::Slider::new(&mut self.probe_sample_rate, 30.0..=480.0)
                .logarithmic(true)
                .integer()
                .text("samples / sim s"),
        )
        .on_hover_text("Point probes use this rate; area probes are capped at 120");
        ui.add(
            egui::Slider::new(&mut self.probe_history_seconds, 2.0..=60.0)
                .logarithmic(true)
                .text("history (sim s)"),
        );
        ui.separator();

        ui.label("Far field");
        let mut far_field = self.editor.document.far_field;
        let enabled_changed = ui
            .checkbox(&mut far_field.enabled, "Outer-domain far field")
            .changed();
        let inset_changed = ui
            .add_enabled(
                far_field.enabled,
                egui::DragValue::new(&mut far_field.inset)
                    .speed(0.005)
                    .range(0.01..=0.9)
                    .prefix("inset ")
                    .update_while_editing(false),
            )
            .changed();
        if enabled_changed || inset_changed {
            let result = self.editor.set_far_field(far_field);
            self.error(result);
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(far_field.enabled, egui::Button::new("Open readout"))
                .clicked()
            {
                self.far_field_open = true;
            }
            if ui
                .add_enabled(
                    far_field.enabled && !self.far_field_trace.frames.is_empty(),
                    egui::Button::new("Clear"),
                )
                .clicked()
            {
                let time = self
                    .far_field_trace
                    .frames
                    .back()
                    .map_or(0.0, |frame| frame.time);
                self.far_field_trace.frames.clear();
                self.far_field_trace.accept_after = time;
            }
        });
        if far_field.enabled {
            if let Some(status) = &self.far_field_status {
                ui.colored_label(RED, status);
            } else if let Some(frame) = self.far_field_trace.frames.back() {
                ui.small(format!(
                    "{} directions · t {:.3}",
                    FAR_FIELD_DIRECTIONS, frame.time
                ));
            } else {
                ui.small("Warming up the propagation delay…");
            }
        }
        ui.separator();

        let probes = self.editor.document.probes.clone();
        if probes.is_empty() {
            ui.label("No probes");
        }
        for probe in &probes {
            let selected = self.selected_probe == Some(probe.id);
            ui.horizontal(|ui| {
                let color = Color32::from_rgb(probe.color[0], probe.color[1], probe.color[2]);
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 5.0, color);
                if ui
                    .selectable_label(selected, &probe.name)
                    .on_hover_text(match probe.target {
                        ProbeTarget::Point(_) => "Point probe",
                        ProbeTarget::Segment { .. } => "Line probe",
                        ProbeTarget::Boundary(_) => "Boundary probe",
                        ProbeTarget::AreaDisk { .. } | ProbeTarget::AreaRegion { .. } => {
                            "Area probe"
                        }
                    })
                    .clicked()
                {
                    self.select_probe(probe.id);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let open = self.probe_windows.contains(&probe.id);
                    if ui
                        .small_button(if open { "Close" } else { "Plot" })
                        .clicked()
                    {
                        if open {
                            self.probe_windows.remove(&probe.id);
                        } else {
                            self.probe_windows.insert(probe.id);
                        }
                    }
                });
            });
            if let Some(status) = self.probe_status.get(&probe.id) {
                ui.small(status);
            } else {
                match probe.target {
                    ProbeTarget::Point(_) => {
                        if let Some(sample) = self
                            .probe_traces
                            .get(&probe.id)
                            .and_then(|trace| trace.samples.back())
                        {
                            ui.small(format!(
                                "u {:+.3e} · energy {:.3e}",
                                sample.displacement, sample.energy_density
                            ));
                        } else {
                            ui.small(if probe.enabled {
                                "Waiting for samples"
                            } else {
                                "Disabled"
                            });
                        }
                    }
                    ProbeTarget::Segment { start, end, .. } => {
                        if let Some(frame) = self
                            .curve_probe_traces
                            .get(&probe.id)
                            .and_then(|trace| trace.frames.back())
                        {
                            let (power, coverage) = Self::curve_probe_integral(
                                frame,
                                (end - start).norm(),
                                LineProbeQuantity::Flux,
                                false,
                            );
                            ui.small(format!(
                                "power {power:+.3e} · {:.0}% coverage",
                                coverage * 100.0
                            ));
                        } else {
                            ui.small(if probe.enabled {
                                "Waiting for samples"
                            } else {
                                "Disabled"
                            });
                        }
                    }
                    ProbeTarget::Boundary(_) => {
                        if let (Some(frame), Some((length, closed))) = (
                            self.curve_probe_traces
                                .get(&probe.id)
                                .and_then(|trace| trace.frames.back()),
                            self.curve_probe_metrics.get(&probe.id).copied(),
                        ) {
                            let (power, coverage) = Self::curve_probe_integral(
                                frame,
                                length,
                                LineProbeQuantity::Flux,
                                closed,
                            );
                            ui.small(format!(
                                "power {power:+.3e} · {:.0}% coverage",
                                coverage * 100.0
                            ));
                        } else {
                            ui.small(if probe.enabled {
                                "Waiting for samples"
                            } else {
                                "Disabled"
                            });
                        }
                    }
                    ProbeTarget::AreaDisk { .. } | ProbeTarget::AreaRegion { .. } => {
                        if let Some(sample) = self
                            .area_probe_traces
                            .get(&probe.id)
                            .and_then(|trace| trace.samples.back())
                        {
                            ui.small(format!(
                                "mean u {:+.3e} · energy {:.3e} · {:.0}% coverage",
                                sample.mean_displacement,
                                sample.total_energy,
                                sample.coverage * 100.0
                            ));
                        } else {
                            ui.small(if probe.enabled {
                                "Waiting for samples"
                            } else {
                                "Disabled"
                            });
                        }
                    }
                }
            }
        }

        let Some(id) = self.selected_probe else {
            return;
        };
        let Some(mut probe) = self
            .editor
            .document
            .probes
            .iter()
            .find(|probe| probe.id == id)
            .cloned()
        else {
            self.selected_probe = None;
            return;
        };
        ui.separator();
        ui.label(match probe.target {
            ProbeTarget::Point(_) => "Selected point probe",
            ProbeTarget::Segment { .. } => "Selected line probe",
            ProbeTarget::Boundary(_) => "Selected boundary probe",
            ProbeTarget::AreaDisk { .. } | ProbeTarget::AreaRegion { .. } => "Selected area probe",
        });
        if !matches!(self.probe_name_edit.as_ref(), Some((candidate, _)) if *candidate == id) {
            self.probe_name_edit = Some((id, probe.name.clone()));
        }
        let mut commit_name = None;
        if let Some((_, name)) = self.probe_name_edit.as_mut() {
            let response = ui.add(egui::TextEdit::singleline(name).hint_text("Probe name"));
            if response.lost_focus() && !name.trim().is_empty() && name.len() <= 64 {
                commit_name = Some(name.clone());
            }
        }
        if let Some(name) = commit_name {
            probe.name = name;
            let result = self.editor.update_probe(probe.clone());
            self.error(result);
            self.probe_name_edit = None;
        }
        let mut enabled = probe.enabled;
        if ui.checkbox(&mut enabled, "Recording").changed() {
            probe.enabled = enabled;
            let result = self.editor.update_probe(probe.clone());
            self.error(result);
        }
        ui.horizontal(|ui| {
            ui.label("Color");
            if ui.color_edit_button_srgb(&mut probe.color).changed() {
                let result = self.editor.update_probe(probe.clone());
                self.error(result);
            }
        });
        if let ProbeTarget::Segment { start, end, preset } = probe.target {
            let mut next_preset = preset;
            egui::ComboBox::from_label("Sampling")
                .selected_text(match preset {
                    ProbeSamplingPreset::Low => "Low · 32 × 30 Hz",
                    ProbeSamplingPreset::Medium => "Medium · 64 × 60 Hz",
                    ProbeSamplingPreset::High => "High · 128 × 120 Hz",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut next_preset,
                        ProbeSamplingPreset::Low,
                        "Low · 32 × 30 Hz",
                    );
                    ui.selectable_value(
                        &mut next_preset,
                        ProbeSamplingPreset::Medium,
                        "Medium · 64 × 60 Hz",
                    );
                    ui.selectable_value(
                        &mut next_preset,
                        ProbeSamplingPreset::High,
                        "High · 128 × 120 Hz",
                    );
                });
            if next_preset != preset {
                probe.target = ProbeTarget::Segment {
                    start,
                    end,
                    preset: next_preset,
                };
                let result = self.editor.update_probe(probe.clone());
                self.error(result);
                self.clear_probe_trace(id);
            }
            if ui
                .button("Flip direction")
                .on_hover_text("Reverse the line and its positive normal")
                .clicked()
            {
                probe.target = ProbeTarget::Segment {
                    start: end,
                    end: start,
                    preset: next_preset,
                };
                let result = self.editor.update_probe(probe.clone());
                self.error(result);
                self.clear_probe_trace(id);
            }
        }
        if let ProbeTarget::Boundary(mut target) = probe.target {
            let mut changed = false;
            let mut next_preset = target.preset;
            egui::ComboBox::from_label("Sampling")
                .selected_text(match target.preset {
                    ProbeSamplingPreset::Low => "Low · 32 × 30 Hz",
                    ProbeSamplingPreset::Medium => "Medium · 64 × 60 Hz",
                    ProbeSamplingPreset::High => "High · 128 × 120 Hz",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut next_preset,
                        ProbeSamplingPreset::Low,
                        "Low · 32 × 30 Hz",
                    );
                    ui.selectable_value(
                        &mut next_preset,
                        ProbeSamplingPreset::Medium,
                        "Medium · 64 × 60 Hz",
                    );
                    ui.selectable_value(
                        &mut next_preset,
                        ProbeSamplingPreset::High,
                        "High · 128 × 120 Hz",
                    );
                });
            if next_preset != target.preset {
                target.preset = next_preset;
                changed = true;
            }
            match target.feature {
                BoundaryProbeFeature::Loop(id) => {
                    if self.editor.obstacle(id).is_some_and(|loop_| {
                        matches!(
                            loop_.role,
                            LoopRole::MaterialInterface { .. } | LoopRole::Wall { .. }
                        )
                    }) {
                        let mut side = target.side;
                        egui::ComboBox::from_label("Trace")
                            .selected_text(match side {
                                BoundaryProbeSide::Exterior => "Outside",
                                BoundaryProbeSide::Interior => "Inside",
                                _ => "Unavailable",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut side,
                                    BoundaryProbeSide::Exterior,
                                    "Outside",
                                );
                                ui.selectable_value(
                                    &mut side,
                                    BoundaryProbeSide::Interior,
                                    "Inside",
                                );
                            });
                        if side != target.side {
                            target.side = side;
                            changed = true;
                        }
                    }
                }
                BoundaryProbeFeature::Baffle(_) => {
                    let mut side = target.side;
                    egui::ComboBox::from_label("Trace")
                        .selected_text(match side {
                            BoundaryProbeSide::Left => "Left",
                            BoundaryProbeSide::Right => "Right",
                            _ => "Unavailable",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut side, BoundaryProbeSide::Left, "Left");
                            ui.selectable_value(&mut side, BoundaryProbeSide::Right, "Right");
                        });
                    if side != target.side {
                        target.side = side;
                        changed = true;
                    }
                }
                BoundaryProbeFeature::Outer => {}
            }
            if ui
                .button("Flip direction")
                .on_hover_text("Reverse the arclength axis; outward flux keeps its sign")
                .clicked()
            {
                target.reversed = !target.reversed;
                changed = true;
            }
            if changed {
                probe.target = ProbeTarget::Boundary(target);
                let result = self.editor.update_probe(probe.clone());
                self.error(result);
                self.clear_probe_trace(id);
            }
        }
        if let ProbeTarget::AreaDisk { center, radius } = probe.target {
            let mut next_radius = radius;
            if ui
                .add(
                    egui::DragValue::new(&mut next_radius)
                        .speed(0.01)
                        .range(1.0e-6..=4.0)
                        .prefix("radius ")
                        .update_while_editing(false),
                )
                .changed()
            {
                probe.target = ProbeTarget::AreaDisk {
                    center,
                    radius: next_radius,
                };
                let result = self.editor.update_probe(probe.clone());
                self.error(result);
                self.clear_probe_trace(id);
            }
        } else if let ProbeTarget::AreaRegion { region } = probe.target {
            let material = self
                .editor
                .document
                .draft
                .region(region)
                .and_then(|region| self.editor.document.draft.material(region.material))
                .map_or("Missing material", |material| material.name.as_str());
            ui.small(format!("{material} subdomain"));
        }
        ui.horizontal(|ui| {
            if ui.button("Open readout").clicked() {
                self.probe_windows.insert(id);
            }
            if ui.button("Delete probe").clicked() {
                let result = self.editor.delete_probe(id);
                self.error(result);
                self.probe_windows.remove(&id);
                self.probe_traces.remove(&id);
                self.curve_probe_traces.remove(&id);
                self.area_probe_traces.remove(&id);
                self.probe_status.remove(&id);
                self.selected_probe = None;
                self.interaction_mode = InteractionMode::Select;
            }
        });
    }

    fn select_probe(&mut self, id: ProbeId) {
        self.selected_probe = Some(id);
        self.selection = None;
        self.internal_selection = None;
        self.selected_spans.clear();
        self.focused_feature = None;
    }

    fn clear_probe_trace(&mut self, id: ProbeId) {
        let current_time =
            self.wave_time_offset + self.wave_completed_steps as f64 * self.wave_time_step;
        let trace = self.probe_traces.entry(id).or_default();
        let newest = trace
            .samples
            .back()
            .map_or(trace.accept_after, |sample| sample.time);
        trace.samples.clear();
        trace.accept_after = current_time.max(newest);
        let curve = self.curve_probe_traces.entry(id).or_default();
        let newest = curve
            .frames
            .back()
            .map_or(curve.accept_after, |sample| sample.time);
        curve.frames.clear();
        curve.accept_after = current_time.max(newest);
        let area = self.area_probe_traces.entry(id).or_default();
        let newest = area
            .samples
            .back()
            .map_or(area.accept_after, |sample| sample.time);
        area.samples.clear();
        area.accept_after = current_time.max(newest);
    }

    fn simulation_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        self.panel_header(ui, "Simulation");
        ui.add_space(8.0);
        ui.separator();
        ui.label("Mesh resolution");
        let resolution_name = |edge: f64| {
            if edge >= 0.12 {
                "Coarse"
            } else if edge >= 0.06 {
                "Medium"
            } else {
                "Fine"
            }
        };
        egui::ComboBox::from_id_salt("mesh_resolution")
            .width(ui.available_width())
            .selected_text(format!(
                "{} · max edge {:.2}",
                resolution_name(self.mesh_max_edge),
                self.mesh_max_edge
            ))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.mesh_max_edge, 0.16, "Coarse · h ≤ 0.16 · P2e");
                ui.selectable_value(&mut self.mesh_max_edge, 0.08, "Medium · h ≤ 0.08 · P2e");
                ui.selectable_value(&mut self.mesh_max_edge, 0.04, "Fine · h ≤ 0.04 · P2e");
            });
        if self.mesh.is_some() && self.mesh_committed_max_edge != self.mesh_max_edge {
            ui.small(format!(
                "Active {:.2} · requested {:.2}",
                self.mesh_committed_max_edge, self.mesh_max_edge
            ));
        }
        if self.editor.editing() && self.mesh_source != self.editor.document.accepted {
            ui.small("Waiting for edit to finish…");
        } else if let Some(job) = &self.mesh_job {
            ui.small(format!("Mesh rebuilding: {}…", job.phase()));
        } else if let Some(error) = &self.mesh_error {
            ui.colored_label(RED, error);
            ui.small("Previous mesh retained.");
        } else if self.mesh.is_some() {
            ui.small("Mesh ready");
        } else {
            ui.small("Preparing mesh…");
        }

        ui.add_space(8.0);
        ui.label("Automatic adaptation");
        let previous_amr = (
            self.amr_enabled,
            self.amr_quality,
            self.amr_minimum_edge,
            self.amr_maximum_edge,
        );
        ui.checkbox(&mut self.amr_enabled, "Adapt mesh to the wave");
        ui.add_enabled_ui(self.amr_enabled, |ui| {
            egui::ComboBox::from_id_salt("amr_quality")
                .width(ui.available_width())
                .selected_text(self.amr_quality.label())
                .show_ui(ui, |ui| {
                    for quality in AmrQuality::ALL {
                        ui.selectable_value(&mut self.amr_quality, quality, quality.label());
                    }
                });
            egui::CollapsingHeader::new("Advanced size limits").show(ui, |ui| {
                ui.add(
                    egui::DragValue::new(&mut self.amr_minimum_edge)
                        .range(0.005..=0.15)
                        .speed(0.002)
                        .prefix("min "),
                );
                ui.add(
                    egui::DragValue::new(&mut self.amr_maximum_edge)
                        .range(0.02..=0.30)
                        .speed(0.005)
                        .prefix("max "),
                );
                self.amr_minimum_edge = self.amr_minimum_edge.min(self.amr_maximum_edge).max(0.005);
                self.amr_maximum_edge = self.amr_maximum_edge.max(self.amr_minimum_edge);
            });
            ui.small(self.amr_status);
            if let Some(error) = &self.amr_error {
                ui.colored_label(RED, error);
            }
        });
        let current_amr = (
            self.amr_enabled,
            self.amr_quality,
            self.amr_minimum_edge,
            self.amr_maximum_edge,
        );
        if current_amr != previous_amr {
            self.amr_settings_revision = self.amr_settings_revision.wrapping_add(1).max(1);
            self.solution_indicator_job = None;
            self.solution_indicator_result = None;
            self.solution_indicator_source = None;
            self.mesh_adaptation_job = None;
            self.mesh_adaptation_automatic = false;
            self.amr_last_analyzed_step = None;
            self.amr_last_started = None;
            self.amr_coarsen_streak = 0;
            self.amr_error = None;
        }

        ui.add_space(10.0);
        ui.separator();
        ui.label("Excitation");
        let wave_available = self.wave_operator.is_some();
        ui.add_enabled_ui(wave_available, |ui| {
            let placing_pulse = self.interaction_mode == InteractionMode::PlacePulse;
            if ui
                .add(egui::Button::new("Place pulse").selected(placing_pulse))
                .on_hover_text(
                    "Click repeatedly in the viewport; click again or press Esc to finish",
                )
                .clicked()
            {
                self.interaction_mode = if placing_pulse {
                    InteractionMode::Select
                } else {
                    InteractionMode::PlacePulse
                };
            }
            ui.add(
                egui::Slider::new(&mut self.pulse_amplitude, 0.01..=5.0)
                    .logarithmic(true)
                    .text("pulse strength"),
            );
            ui.add(
                egui::Slider::new(&mut self.pulse_width, 0.005..=0.3)
                    .logarithmic(true)
                    .text("pulse width"),
            );
            ui.add(
                egui::Slider::new(&mut self.wave_speed, 0.1..=4.0)
                    .logarithmic(true)
                    .text("simulation speed"),
            );
            let source_before = self.wave_source;
            let enabled_response = ui.checkbox(&mut self.wave_source.enabled, "Continuous source");
            let source_responses = ui.add_enabled_ui(self.wave_source.enabled, |ui| {
                let mut responses = Vec::new();
                let mut position = self.wave_source.position;
                ui.horizontal(|ui| {
                    ui.label("Position");
                    responses.push(
                        ui.add(
                            egui::DragValue::new(&mut position.x)
                                .speed(0.005)
                                .prefix("x "),
                        ),
                    );
                    responses.push(
                        ui.add(
                            egui::DragValue::new(&mut position.y)
                                .speed(0.005)
                                .prefix("y "),
                        ),
                    );
                });
                if position != self.wave_source.position {
                    self.wave_source.position = position;
                    self.wave_source.region = self
                        .wave_mesh
                        .as_ref()
                        .and_then(|mesh| mesh_region_at(mesh, position))
                        .unwrap_or(BACKGROUND_REGION);
                }
                responses.extend([
                    ui.add(
                        egui::Slider::new(&mut self.wave_source.frequency_hz, 0.25..=8.0)
                            .logarithmic(true)
                            .text("source frequency"),
                    ),
                    ui.add(
                        egui::Slider::new(&mut self.wave_source.amplitude, 1.0..=50.0)
                            .logarithmic(true)
                            .text("source strength"),
                    ),
                    ui.add(
                        egui::Slider::new(&mut self.wave_source.width, 0.005..=0.3)
                            .logarithmic(true)
                            .text("source width"),
                    ),
                ]);
                ui.small(format!("Region {}", self.wave_source.region.0));
                responses
            });
            if self.wave_source != source_before {
                if self.wave_source.valid() {
                    self.editor.begin();
                    self.editor.document.source = self.wave_source;
                    self.wave_source_dirty = true;
                } else {
                    self.wave_source = source_before;
                    self.message = "Continuous-source values must be finite".into();
                }
            }
            if enabled_response.changed()
                || source_responses
                    .inner
                    .iter()
                    .any(|response| response.lost_focus() || response.drag_stopped())
            {
                self.editor.commit();
            }
        });
        if let Some(operator) = &self.wave_operator {
            ui.small(format!(
                "GPU {} · {} DOFs · dt {:.6}",
                self.wave_gpu_status,
                operator.degrees_of_freedom(),
                self.wave_time_step,
            ));
            if let Some(energy) = self.wave_energy {
                ui.small(format!("Discrete energy {energy:.6e}"));
            }
        } else if self.mesh.is_some() {
            ui.small("Preparing wave operator…");
        } else if self.simulation_candidate.is_some() {
            ui.small("Preparing initial wave state…");
        } else {
            ui.small("Waiting for an accepted mesh…");
        }
        if let Some(error) = &self.wave_error {
            ui.colored_label(RED, error);
        }
    }

    fn edit_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        self.panel_header(ui, "Edit");
        if self
            .editor
            .document
            .draft
            .material(self.material_selection)
            .is_none()
        {
            self.material_selection = self
                .editor
                .document
                .draft
                .materials
                .first()
                .map_or(DEFAULT_MATERIAL, |material| material.id);
        }
        ui.horizontal(|ui| {
            ui.label("Span filter");
            egui::ComboBox::from_id_salt("span_selection_filter")
                .selected_text(self.span_selection_filter.label())
                .show_ui(ui, |ui| {
                    for filter in [
                        SpanSelectionFilter::All,
                        SpanSelectionFilter::Outer,
                        SpanSelectionFilter::Loops,
                        SpanSelectionFilter::Baffles,
                    ] {
                        ui.selectable_value(
                            &mut self.span_selection_filter,
                            filter,
                            filter.label(),
                        );
                    }
                });
        });
        ui.horizontal(|ui| {
            if ui.small_button("Select filtered").clicked() {
                self.select_filtered();
            }
            if ui.small_button("Invert").clicked() {
                self.invert_filtered_selection();
            }
            if ui.small_button("Clear").clicked() {
                self.set_span_selection(vec![]);
            }
        });
        if let InteractionMode::DrawCustom { .. } = self.interaction_mode {
            ui.horizontal(|ui| {
                ui.label(format!("{} / 128 points", self.custom.len()));
                if ui
                    .add_enabled(self.custom.len() >= 4, egui::Button::new("Finish"))
                    .clicked()
                {
                    self.finish_custom();
                }
                if ui.button("Cancel").clicked() {
                    self.custom.clear();
                    self.interaction_mode = InteractionMode::Select;
                }
            });
            ui.small("Enter finishes · Backspace removes the last point");
        }
        if let Some(summary) = self.selection_summary() {
            ui.add_space(8.0);
            ui.strong(summary);
        }
        ui.add_space(10.0);
        ui.separator();
        ui.label(format!(
            "Features  {} / 32",
            self.editor.document.draft.obstacles.len()
                + self.editor.document.draft.internal_boundaries.len()
        ));
        egui::ScrollArea::vertical()
            .id_salt("obstacles")
            .max_height(135.0)
            .show(ui, |ui| {
                let obstacles = self.editor.document.draft.obstacles.clone();
                let boundaries = self.editor.document.draft.internal_boundaries.clone();
                for o in &obstacles {
                    let assignment = if matches!(o.role, LoopRole::Hole { .. }) {
                        if o.span_conditions
                            .iter()
                            .all(|condition| *condition == FaceBoundaryCondition::Reflecting)
                        {
                            " · Reflecting"
                        } else {
                            " · Assigned BCs"
                        }
                    } else {
                        ""
                    };
                    if ui
                        .selectable_label(
                            self.selected_spans.iter().any(
                                |span| matches!(span, GeometrySpan::Loop(id, _) if *id == o.id),
                            ) || self.focused_feature == Some(FocusedFeature::Loop(o.id)),
                            format!(
                                "{} {}{} · {} controls",
                                o.role.label(),
                                o.id.0,
                                assignment,
                                o.spline.controls().len()
                            ),
                        )
                        .clicked()
                    {
                        self.select_loop(o.id);
                    }
                }
                for boundary in &boundaries {
                    if ui
                        .selectable_label(
                            self.selected_spans.iter().any(|span| {
                                matches!(span, GeometrySpan::Baffle(id, _) if *id == boundary.id)
                            }) || self.focused_feature
                                == Some(FocusedFeature::Baffle(boundary.id)),
                            format!(
                                "Baffle {:02} · {} · {} controls",
                                boundary.id.0,
                                if boundary
                                    .span_laws
                                    .iter()
                                    .all(|law| *law == InternalBoundaryLaw::REFLECTING)
                                {
                                    "Reflecting"
                                } else {
                                    "Assigned laws"
                                },
                                boundary.spline.controls().len()
                            ),
                        )
                        .clicked()
                    {
                        self.select_baffle(boundary.id);
                    }
                }
            });

        if let Some(FocusedFeature::Loop(id)) = self.focused_feature
            && self.complete_selected_feature() == Some(FocusedFeature::Loop(id))
        {
            if ui
                .button(egui::RichText::new("Delete loop").color(RED))
                .clicked()
            {
                self.editor.delete_obstacle(id);
                self.clear_transient();
            }
            if ui.button("Duplicate loop").clicked() {
                let result = self.editor.duplicate_obstacle(id, Point2::new(0.05, -0.05));
                if let Some(id) = self.error(result) {
                    self.select_loop(id);
                }
            }
            if let Some(current_kind) = self.editor.loop_kind(id) {
                ui.add_space(6.0);
                ui.label("Loop role");
                let mut kind = self
                    .loop_role_edit
                    .filter(|(candidate, _)| *candidate == id)
                    .map_or(current_kind, |(_, kind)| kind);
                egui::ComboBox::from_id_salt(("loop_kind", id.0))
                    .width(ui.available_width())
                    .selected_text(match kind {
                        LoopKind::Hole => "Hole",
                        LoopKind::MaterialInterface => "Material interface",
                        LoopKind::Wall => "Legacy closed wall",
                    })
                    .show_ui(ui, |ui| {
                        for (editable_kind, label) in EDITABLE_LOOP_KINDS {
                            ui.selectable_value(&mut kind, editable_kind, label);
                        }
                    });
                self.loop_role_edit = Some((id, kind));
                if kind == LoopKind::MaterialInterface && kind != current_kind {
                    let materials = self.editor.document.draft.materials.clone();
                    ui.label("Interior material");
                    egui::ComboBox::from_id_salt(("new_interior_material", id.0))
                        .width(ui.available_width())
                        .selected_text(
                            self.editor
                                .document
                                .draft
                                .material(self.material_selection)
                                .map_or("Missing", |material| material.name.as_str()),
                        )
                        .show_ui(ui, |ui| {
                            for material in materials {
                                ui.selectable_value(
                                    &mut self.material_selection,
                                    material.id,
                                    material.name,
                                );
                            }
                        });
                }
                if kind != current_kind && ui.button("Apply role change").clicked() {
                    let result = self.editor.set_loop_kind(id, kind, self.material_selection);
                    if self.error(result).is_some() {
                        self.select_loop(id);
                        self.loop_role_edit = None;
                    }
                }
            }
        }
        if let Some((id, Some(index))) = self.selection
            && let Some(o) = self.editor.obstacle(id)
            && let Some(p) = o.spline.controls().get(index).copied()
        {
            ui.add_space(8.0);
            ui.label(format!("Control {}", index + 1));
            let mut p = p;
            let responses = ui
                .push_id((id.0, index, "coordinates"), |ui| {
                    ui.horizontal(|ui| {
                        [
                            ui.add(
                                egui::DragValue::new(&mut p.x)
                                    .speed(0.002)
                                    .range(-1e6..=1e6)
                                    .prefix("x ")
                                    .update_while_editing(false),
                            ),
                            ui.add(
                                egui::DragValue::new(&mut p.y)
                                    .speed(0.002)
                                    .range(-1e6..=1e6)
                                    .prefix("y ")
                                    .update_while_editing(false),
                            ),
                        ]
                    })
                    .inner
                })
                .inner;
            if responses
                .iter()
                .any(|r| r.gained_focus() || r.drag_started() || r.changed())
            {
                self.editor.begin();
            }
            if responses.iter().any(|r| r.changed()) {
                let result = self.editor.set_point(id, index, p);
                self.error(result);
            }
            if responses.iter().any(|r| r.lost_focus() || r.drag_stopped()) {
                self.editor.commit();
            }
            let can_remove = self.editor.obstacle(id).is_some_and(|o| {
                o.spline.controls().len() > 4
                    && o.spline.multiplicities().iter().all(|value| *value == 1)
            });
            if ui
                .add_enabled(
                    can_remove,
                    egui::Button::new("Remove control · reshapes curve"),
                )
                .clicked()
            {
                let result = self.editor.remove_point(id, index);
                if self.error(result).is_some() {
                    self.select_loop(id);
                }
            }
        }

        if let Some(FocusedFeature::Baffle(id)) = self.focused_feature
            && self.complete_selected_feature() == Some(FocusedFeature::Baffle(id))
        {
            if ui
                .button(egui::RichText::new("Delete baffle").color(RED))
                .clicked()
            {
                self.editor.delete_internal_boundary(id);
                self.clear_transient();
            }
            if ui.button("Duplicate baffle").clicked() {
                let result = self
                    .editor
                    .duplicate_internal_boundary(id, Point2::new(0.05, -0.05));
                if let Some(id) = self.error(result) {
                    self.select_baffle(id);
                }
            }
            if ui.button("Straighten baffle").clicked() {
                let result = self.editor.straighten_internal_boundary(id);
                if self.error(result).is_some() {
                    self.gizmo_pivot = None;
                }
            }
        }
        if let Some((id, Some(index))) = self.internal_selection
            && let Some(boundary) = self.editor.internal_boundary(id)
            && let Some(point) = boundary.spline.controls().get(index).copied()
        {
            ui.add_space(8.0);
            ui.label(format!("Baffle control {}", index + 1));
            let mut point = point;
            let responses = ui
                .push_id((id.0, index, "baffle_coordinates"), |ui| {
                    ui.horizontal(|ui| {
                        [
                            ui.add(
                                egui::DragValue::new(&mut point.x)
                                    .speed(0.002)
                                    .range(-1e6..=1e6)
                                    .prefix("x ")
                                    .update_while_editing(false),
                            ),
                            ui.add(
                                egui::DragValue::new(&mut point.y)
                                    .speed(0.002)
                                    .range(-1e6..=1e6)
                                    .prefix("y ")
                                    .update_while_editing(false),
                            ),
                        ]
                    })
                    .inner
                })
                .inner;
            if responses.iter().any(|response| {
                response.gained_focus() || response.drag_started() || response.changed()
            }) {
                self.editor.begin();
            }
            if responses.iter().any(|response| response.changed()) {
                let result = self.editor.set_internal_boundary_point(id, index, point);
                self.error(result);
            }
            if responses
                .iter()
                .any(|response| response.lost_focus() || response.drag_stopped())
            {
                self.editor.commit();
            }
            let can_remove = self.editor.internal_boundary(id).is_some_and(|boundary| {
                boundary.spline.controls().len() > 4
                    && boundary
                        .spline
                        .multiplicities()
                        .iter()
                        .all(|value| *value == 1)
            });
            if ui
                .add_enabled(
                    can_remove,
                    egui::Button::new("Remove control · reshapes curve"),
                )
                .clicked()
            {
                let result = self.editor.remove_internal_boundary_point(id, index);
                if self.error(result).is_some() {
                    self.select_baffle(id);
                }
            }
        }
        let has_single_control = self.selected_spans.is_empty()
            && (matches!(self.selection, Some((_, Some(_))))
                || matches!(self.internal_selection, Some((_, Some(_)))));
        if has_single_control {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.snap_to_grid, "Snap");
                ui.add_enabled(
                    self.snap_to_grid,
                    egui::DragValue::new(&mut self.snap_step)
                        .speed(0.005)
                        .range(1.0e-6..=2.0)
                        .prefix("step "),
                );
                if ui
                    .add_enabled(self.snap_to_grid, egui::Button::new("Snap point"))
                    .clicked()
                {
                    self.snap_selection_now();
                }
            });
        }
        let transformable = self.transformable_curve_controls();
        if !self.selected_spans.is_empty() {
            ui.add_space(8.0);
            if let Some(groups) = transformable {
                let piece_count = groups.len();
                let noun = if piece_count == 1 { "piece" } else { "pieces" };
                ui.label(format!("Transform · {piece_count} {noun}"));
                if let Some(pivot) = self.selection_pivot() {
                    ui.small(format!("Pivot  x {:.4}  y {:.4}", pivot.x, pivot.y));
                }
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.transform_translation.x)
                            .speed(0.005)
                            .prefix("dx "),
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.transform_translation.y)
                            .speed(0.005)
                            .prefix("dy "),
                    );
                });
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.transform_rotation_degrees)
                            .speed(0.5)
                            .prefix("rotate ")
                            .suffix("°"),
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.transform_scale)
                            .speed(0.01)
                            .range(1.0e-4..=1.0e4)
                            .prefix("scale "),
                    );
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.snap_to_grid, "Snap");
                    ui.add_enabled(
                        self.snap_to_grid,
                        egui::DragValue::new(&mut self.snap_step)
                            .speed(0.005)
                            .range(1.0e-6..=2.0)
                            .prefix("step "),
                    );
                });
                ui.horizontal(|ui| {
                    if ui.button("Apply transform").clicked() {
                        self.apply_selection_transform();
                    }
                    if ui
                        .add_enabled(self.snap_to_grid, egui::Button::new("Snap now"))
                        .clicked()
                    {
                        self.snap_selection_now();
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(piece_count > 1, egui::Button::new("Align horizontal"))
                        .on_hover_text(
                            "Align the centers of two or more independently movable pieces",
                        )
                        .clicked()
                    {
                        self.align_selection(true);
                    }
                    if ui
                        .add_enabled(piece_count > 1, egui::Button::new("Align vertical"))
                        .on_hover_text(
                            "Align the centers of two or more independently movable pieces",
                        )
                        .clicked()
                    {
                        self.align_selection(false);
                    }
                });
            }
        }
        self.topology_inspector(ui);
        self.boundary_inspector(ui);
        ui.add_space(12.0);
        ui.add_space(6.0);
        ui.small("Pan: right drag / Space + drag\nZoom: wheel over viewport\nUndo: Ctrl/Cmd + Z · Shift for redo");
    }

    fn selected_boundary_targets(&self) -> Option<Vec<BoundaryFaceTarget>> {
        if self.selected_spans.is_empty() {
            return None;
        }
        self.selected_spans
            .iter()
            .map(|span| match *span {
                GeometrySpan::Outer(side) => Some(BoundaryFaceTarget::Outer(side)),
                GeometrySpan::Loop(id, span) => self
                    .editor
                    .obstacle(id)
                    .filter(|obstacle| matches!(obstacle.role, LoopRole::Hole { .. }))
                    .map(|_| BoundaryFaceTarget::Hole(id, span)),
                GeometrySpan::Baffle(id, span) => {
                    Some(BoundaryFaceTarget::Baffle(id, span, self.baffle_face))
                }
            })
            .collect()
    }

    fn boundary_probe_target_from_selection(&self) -> Option<BoundaryProbeTarget> {
        let first = *self.selected_spans.first()?;
        let (feature, total, closed, side) = match first {
            GeometrySpan::Outer(_) => (
                BoundaryProbeFeature::Outer,
                4,
                true,
                BoundaryProbeSide::Domain,
            ),
            GeometrySpan::Loop(id, _) => {
                let loop_ = self.editor.obstacle(id)?;
                let side = match loop_.role {
                    LoopRole::Hole { .. } => BoundaryProbeSide::Domain,
                    LoopRole::MaterialInterface { .. } | LoopRole::Wall { .. } => {
                        BoundaryProbeSide::Exterior
                    }
                };
                (
                    BoundaryProbeFeature::Loop(id),
                    loop_.spline.intervals().len(),
                    true,
                    side,
                )
            }
            GeometrySpan::Baffle(id, _) => (
                BoundaryProbeFeature::Baffle(id),
                self.editor.internal_boundary(id)?.spline.intervals().len(),
                false,
                match self.baffle_face {
                    InternalBoundarySide::Left => BoundaryProbeSide::Left,
                    InternalBoundarySide::Right => BoundaryProbeSide::Right,
                },
            ),
        };
        let mut indices = BTreeSet::new();
        for span in &self.selected_spans {
            let index = match (*span, feature) {
                (GeometrySpan::Outer(side), BoundaryProbeFeature::Outer) => side.index(),
                (GeometrySpan::Loop(id, index), BoundaryProbeFeature::Loop(expected))
                    if id == expected =>
                {
                    index
                }
                (GeometrySpan::Baffle(id, index), BoundaryProbeFeature::Baffle(expected))
                    if id == expected =>
                {
                    index
                }
                _ => return None,
            };
            if index >= total {
                return None;
            }
            indices.insert(index);
        }
        let whole = indices.len() == total;
        let start_span = if whole {
            0
        } else if closed {
            (0..total).find(|index| {
                indices.contains(index) && !indices.contains(&((*index + total - 1) % total))
            })?
        } else {
            *indices.first()?
        };
        let contiguous = (0..indices.len()).all(|offset| {
            let index = if closed {
                (start_span + offset) % total
            } else {
                start_span + offset
            };
            indices.contains(&index)
        });
        contiguous.then_some(BoundaryProbeTarget {
            feature,
            start_span,
            span_count: indices.len(),
            whole,
            side,
            reversed: false,
            preset: ProbeSamplingPreset::Medium,
        })
    }

    fn topology_inspector(&mut self, ui: &mut egui::Ui) {
        if self.selected_spans.is_empty() {
            return;
        }
        let has_endpoint = self.selected_topology_endpoint().is_some();
        let exposed = self.exposed_selection_breakpoints();
        let needs_isolation = exposed.as_ref().is_some_and(|(loops, baffles)| {
            loops.iter().any(|(id, breakpoint)| {
                self.editor
                    .obstacle(*id)
                    .and_then(|obstacle| obstacle.spline.continuity(*breakpoint))
                    != Some(0)
            }) || baffles.iter().any(|(id, breakpoint)| {
                self.editor
                    .internal_boundary(*id)
                    .and_then(|boundary| boundary.spline.continuity(*breakpoint))
                    != Some(0)
            })
        });
        let mut selected_baffles = Vec::new();
        for span in &self.selected_spans {
            if let GeometrySpan::Baffle(id, _) = span
                && !selected_baffles.contains(id)
            {
                selected_baffles.push(*id);
            }
        }
        let has_merge_pair = selected_baffles.len() == 2
            && self
                .selected_spans
                .iter()
                .all(|span| matches!(span, GeometrySpan::Baffle(_, _)))
            && selected_baffles.iter().all(|id| {
                self.editor.internal_boundary(*id).is_some_and(|boundary| {
                    (0..boundary.spline.intervals().len()).all(|span| {
                        self.selected_spans
                            .contains(&GeometrySpan::Baffle(*id, span))
                    })
                })
            });
        if !has_endpoint && !needs_isolation && !has_merge_pair {
            return;
        }
        ui.add_space(8.0);
        ui.separator();
        ui.label("Spline topology");

        if self.selected_spans.len() == 1 {
            match self.selected_spans[0] {
                GeometrySpan::Loop(id, span) => {
                    if let Some(obstacle) = self.editor.obstacle(id) {
                        let breakpoint = (span + 1) % obstacle.spline.intervals().len();
                        let continuity = obstacle.spline.continuity(breakpoint).unwrap();
                        ui.label(format!("End knot · C{continuity}"));
                        ui.horizontal(|ui| {
                            for (target, label) in
                                [(2, "C2 smooth"), (1, "C1 tangent"), (0, "C0 corner")]
                            {
                                if ui.selectable_label(continuity == target, label).clicked()
                                    && continuity != target
                                {
                                    let result =
                                        self.editor.set_obstacle_continuity(id, breakpoint, target);
                                    if let Some(displacement) = self.error(result)
                                        && displacement > 0.0
                                    {
                                        self.notify(format!(
                                            "Continuity upgraded; curve reshaped by at most {displacement:.3e}"
                                        ));
                                    }
                                }
                            }
                        });
                    }
                }
                GeometrySpan::Baffle(id, span) => {
                    if let Some(boundary) = self.editor.internal_boundary(id) {
                        let breakpoint = span + 1;
                        if breakpoint < boundary.spline.intervals().len() {
                            let continuity = boundary.spline.continuity(breakpoint).unwrap();
                            ui.label(format!("End knot · C{continuity}"));
                            ui.horizontal(|ui| {
                                for (target, label) in
                                    [(2, "C2 smooth"), (1, "C1 tangent"), (0, "C0 corner")]
                                {
                                    if ui.selectable_label(continuity == target, label).clicked()
                                        && continuity != target
                                    {
                                        let result = self.editor.set_internal_boundary_continuity(
                                            id, breakpoint, target,
                                        );
                                        if let Some(displacement) = self.error(result)
                                            && displacement > 0.0
                                        {
                                            self.notify(format!(
                                                "Continuity upgraded; curve reshaped by at most {displacement:.3e}"
                                            ));
                                        }
                                    }
                                }
                            });
                            if ui.button("Split baffle at end knot").clicked() {
                                let result = self.editor.split_internal_boundary(id, breakpoint);
                                if let Some(new_id) = self.error(result) {
                                    let mut spans = self.curve_spans(GeometrySpan::Baffle(id, 0));
                                    spans.extend(self.curve_spans(GeometrySpan::Baffle(new_id, 0)));
                                    self.set_span_selection(spans);
                                }
                            }
                        }
                    }
                }
                GeometrySpan::Outer(_) => {}
            }
        }

        if let Some((loop_breakpoints, baffle_breakpoints)) = exposed
            && needs_isolation
            && ui.button("Isolate selection at C0").clicked()
        {
            let result = self
                .editor
                .isolate_span_boundaries(&loop_breakpoints, &baffle_breakpoints);
            if self.error(result).is_some() {
                self.gizmo_pivot = None;
            }
        }

        let mut baffles = Vec::new();
        for selected in &self.selected_spans {
            if let GeometrySpan::Baffle(id, _) = *selected
                && !baffles.contains(&id)
            {
                baffles.push(id);
            }
        }
        let two_complete_baffles = baffles.len() == 2
            && self
                .selected_spans
                .iter()
                .all(|span| matches!(span, GeometrySpan::Baffle(_, _)))
            && baffles.iter().all(|id| {
                self.editor.internal_boundary(*id).is_some_and(|boundary| {
                    (0..boundary.spline.intervals().len()).all(|span| {
                        self.selected_spans
                            .contains(&GeometrySpan::Baffle(*id, span))
                    })
                })
            });
        let merge_tolerance = if self.snap_to_grid {
            self.snap_step.max(2.0e-4)
        } else {
            0.02
        };
        let nearest_tip_distance = if two_complete_baffles {
            let endpoints = baffles.iter().filter_map(|id| {
                let boundary = self.editor.internal_boundary(*id)?;
                Some((
                    boundary.spline.evaluate(0.0),
                    boundary.spline.evaluate(boundary.spline.period()),
                ))
            });
            let endpoints = endpoints.collect::<Vec<_>>();
            (endpoints.len() == 2).then(|| {
                let (a_start, a_end) = endpoints[0];
                let (b_start, b_end) = endpoints[1];
                [
                    (a_end - b_start).norm(),
                    (a_end - b_end).norm(),
                    (a_start - b_start).norm(),
                    (a_start - b_end).norm(),
                ]
                .into_iter()
                .fold(f64::INFINITY, f64::min)
            })
        } else {
            None
        };
        let can_merge = nearest_tip_distance.is_some_and(|distance| distance <= merge_tolerance);
        if two_complete_baffles {
            if ui
                .add_enabled(can_merge, egui::Button::new("Merge nearest baffle tips"))
                .clicked()
            {
                let result =
                    self.editor
                        .merge_internal_boundaries(baffles[0], baffles[1], merge_tolerance);
                if let Some(id) = self.error(result) {
                    self.select_baffle(id);
                }
            }
            if let Some(distance) = nearest_tip_distance {
                if can_merge {
                    ui.small(format!(
                        "Nearest tips {:.4} apart · tolerance {:.4}",
                        distance, merge_tolerance
                    ));
                } else {
                    ui.colored_label(
                        GOLD,
                        format!(
                            "Move tips within {:.4} to merge · currently {:.4}",
                            merge_tolerance, distance
                        ),
                    );
                }
            }
        }
    }

    fn selected_topology_endpoint(&self) -> Option<Point2> {
        if self.selected_spans.len() != 1 {
            return None;
        }
        match self.selected_spans[0] {
            GeometrySpan::Loop(id, span) => {
                let spline = &self.editor.obstacle(id)?.spline;
                let breakpoint = (span + 1) % spline.intervals().len();
                Some(spline.evaluate(spline.knots()[breakpoint]))
            }
            GeometrySpan::Baffle(id, span) => {
                let spline = &self.editor.internal_boundary(id)?.spline;
                let breakpoint = span + 1;
                (breakpoint < spline.intervals().len())
                    .then(|| spline.evaluate(spline.breakpoint(breakpoint).unwrap()))
            }
            GeometrySpan::Outer(_) => None,
        }
    }

    fn exposed_selection_breakpoints(&self) -> Option<ExposedBreakpoints> {
        if self
            .selected_spans
            .iter()
            .any(|span| matches!(span, GeometrySpan::Outer(_)))
        {
            return None;
        }
        let mut loops = Vec::new();
        let mut baffles = Vec::new();
        let mut loop_ids = Vec::new();
        let mut baffle_ids = Vec::new();
        for selected in &self.selected_spans {
            match *selected {
                GeometrySpan::Loop(id, _) if !loop_ids.contains(&id) => loop_ids.push(id),
                GeometrySpan::Baffle(id, _) if !baffle_ids.contains(&id) => baffle_ids.push(id),
                _ => {}
            }
        }
        for id in loop_ids {
            let count = self.editor.obstacle(id)?.spline.intervals().len();
            let selected = (0..count)
                .map(|span| self.selected_spans.contains(&GeometrySpan::Loop(id, span)))
                .collect::<Vec<_>>();
            if selected.iter().all(|value| *value) {
                continue;
            }
            for breakpoint in 0..count {
                if selected[breakpoint] != selected[(breakpoint + count - 1) % count] {
                    loops.push((id, breakpoint));
                }
            }
        }
        for id in baffle_ids {
            let count = self.editor.internal_boundary(id)?.spline.intervals().len();
            let selected = (0..count)
                .map(|span| {
                    self.selected_spans
                        .contains(&GeometrySpan::Baffle(id, span))
                })
                .collect::<Vec<_>>();
            for breakpoint in 1..count {
                if selected[breakpoint] != selected[breakpoint - 1] {
                    baffles.push((id, breakpoint));
                }
            }
        }
        Some((loops, baffles))
    }

    fn selected_baffle_spans(&self) -> Option<Vec<(InternalBoundaryId, usize)>> {
        (!self.selected_spans.is_empty()).then_some(())?;
        self.selected_spans
            .iter()
            .map(|span| match *span {
                GeometrySpan::Baffle(id, span) => Some((id, span)),
                _ => None,
            })
            .collect()
    }

    fn boundary_inspector(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.separator();
        ui.label("Boundary");
        if self.selected_spans.is_empty() {
            ui.small("Click a span; Shift-click adds spans.");
            return;
        }
        let has_baffles = self
            .selected_spans
            .iter()
            .any(|span| matches!(span, GeometrySpan::Baffle(_, _)));
        let mut coupled_gap = false;
        let mut mixed_gap = false;
        if let Some(spans) = self.selected_baffle_spans() {
            let couplings = spans
                .iter()
                .map(|(id, span)| {
                    self.editor.internal_boundary(*id).unwrap().span_laws[*span].coupling
                })
                .collect::<Vec<_>>();
            let common = couplings
                .first()
                .copied()
                .filter(|first| couplings.iter().all(|coupling| coupling == first));
            mixed_gap = common.is_none()
                && couplings
                    .iter()
                    .any(|coupling| matches!(coupling, InternalBoundaryCoupling::ThinGap { .. }));
            let mut selected = None;
            ui.label("Span law");
            egui::ComboBox::from_id_salt("selected_baffle_span_law")
                .width(ui.available_width())
                .selected_text(match common {
                    Some(InternalBoundaryCoupling::Independent) => "Independent faces",
                    Some(InternalBoundaryCoupling::ThinGap { .. }) => "Coupled thin gap",
                    None => "Mixed",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut selected,
                        Some(InternalBoundaryCoupling::Independent),
                        "Independent faces",
                    );
                    ui.selectable_value(
                        &mut selected,
                        Some(InternalBoundaryCoupling::ThinGap {
                            stiffness_ratio: 1.0,
                        }),
                        "Coupled thin gap",
                    );
                });
            let mut coupling = selected.or(common);
            let mut changed = selected.is_some();
            if let Some(InternalBoundaryCoupling::ThinGap { stiffness_ratio }) = &mut coupling {
                coupled_gap = true;
                changed |= ui
                    .add(
                        egui::DragValue::new(stiffness_ratio)
                            .speed(0.02)
                            .range(0.01..=100.0)
                            .prefix("gap stiffness ")
                            .update_while_editing(false),
                    )
                    .changed();
                ui.small("The gap law controls both faces.");
            }
            if changed && let Some(coupling) = coupling {
                let result = self
                    .editor
                    .set_internal_boundary_couplings(&spans, coupling);
                self.error(result);
            }
        }
        if coupled_gap || mixed_gap {
            if mixed_gap {
                ui.small("Select one span law before editing face conditions.");
            }
            return;
        }
        if has_baffles {
            ui.horizontal(|ui| {
                ui.label("Baffle face");
                ui.selectable_value(&mut self.baffle_face, InternalBoundarySide::Left, "Left");
                ui.selectable_value(&mut self.baffle_face, InternalBoundarySide::Right, "Right");
            });
        }

        let Some(targets) = self.selected_boundary_targets() else {
            ui.small("This selection has no common boundary-face condition.");
            return;
        };
        let conditions = targets
            .iter()
            .filter_map(|target| self.editor.boundary_face_condition(*target).ok())
            .collect::<Vec<_>>();
        if conditions.len() != targets.len() {
            ui.small("This selection has no common boundary-face condition.");
            return;
        }
        let common = conditions
            .first()
            .copied()
            .filter(|first| conditions.iter().all(|condition| condition == first));
        if let Some(condition) = Self::bulk_face_condition_editor(ui, common) {
            let result = self
                .editor
                .set_boundary_face_conditions(&targets, condition);
            self.error(result);
        }
    }

    fn bulk_face_condition_editor(
        ui: &mut egui::Ui,
        common: Option<FaceBoundaryCondition>,
    ) -> Option<FaceBoundaryCondition> {
        let ratio = match common {
            Some(FaceBoundaryCondition::Impedance { ratio }) => ratio,
            _ => 1.0,
        };
        let signal = common
            .and_then(FaceBoundaryCondition::signal)
            .unwrap_or(BoundarySignal::ZERO);
        let mut selected = None;
        ui.label("Condition");
        egui::ComboBox::from_id_salt("selected_boundary_condition")
            .width(ui.available_width())
            .selected_text(common.map_or("Mixed", FaceBoundaryCondition::label))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut selected,
                    Some(FaceBoundaryCondition::Reflecting),
                    "Neumann · zero / reflecting",
                );
                ui.selectable_value(
                    &mut selected,
                    Some(FaceBoundaryCondition::Impedance { ratio }),
                    "First-order outgoing / impedance",
                );
                ui.selectable_value(
                    &mut selected,
                    Some(FaceBoundaryCondition::SecondOrderOutgoing),
                    "Second-order outgoing",
                );
                ui.selectable_value(
                    &mut selected,
                    Some(FaceBoundaryCondition::Neumann { signal }),
                    "Neumann · prescribed flux",
                );
                ui.selectable_value(
                    &mut selected,
                    Some(FaceBoundaryCondition::Dirichlet { signal }),
                    "Dirichlet · prescribed value",
                );
            });
        let mut condition = selected.or(common);
        let mut changed = selected.is_some();
        if let Some(FaceBoundaryCondition::Impedance { ratio }) = &mut condition {
            changed |= ui
                .add(
                    egui::DragValue::new(ratio)
                        .speed(0.02)
                        .range(0.01..=100.0)
                        .prefix("impedance ratio ")
                        .update_while_editing(false),
                )
                .changed();
            ui.small("Outer edges use the matched ratio 1.0.");
        } else if let Some(condition) = &mut condition
            && let Some(mut signal) = condition.signal()
            && Self::boundary_signal_editor(ui, &mut signal)
        {
            *condition = match condition {
                FaceBoundaryCondition::Neumann { .. } => FaceBoundaryCondition::Neumann { signal },
                FaceBoundaryCondition::Dirichlet { .. } => {
                    FaceBoundaryCondition::Dirichlet { signal }
                }
                _ => unreachable!(),
            };
            changed = true;
        }
        if changed { condition } else { None }
    }

    fn boundary_signal_editor(ui: &mut egui::Ui, signal: &mut BoundarySignal) -> bool {
        ui.small("value(t) = offset + amplitude · sin(2π f t + phase)");
        [
            ui.add(
                egui::DragValue::new(&mut signal.offset)
                    .speed(0.01)
                    .prefix("offset ")
                    .update_while_editing(false),
            ),
            ui.add(
                egui::DragValue::new(&mut signal.amplitude)
                    .speed(0.01)
                    .prefix("amplitude ")
                    .update_while_editing(false),
            ),
            ui.add(
                egui::DragValue::new(&mut signal.frequency_hz)
                    .speed(0.05)
                    .range(0.0..=1.0e6)
                    .suffix(" Hz")
                    .update_while_editing(false),
            ),
            ui.add(
                egui::DragValue::new(&mut signal.phase_radians)
                    .speed(0.05)
                    .prefix("phase ")
                    .suffix(" rad")
                    .update_while_editing(false),
            ),
        ]
        .iter()
        .any(egui::Response::changed)
    }

    fn viewport(&mut self, ui: &mut egui::Ui, wave_display: Option<&WaveDisplay>) -> Rect {
        let (response, painter) =
            ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let r = response.rect;
        if self.fit {
            self.center = Point2::default();
            self.scale = (r.width().min(r.height()) as f64 / 2.5).max(20.0);
            self.fit = false;
        }
        let ctx = ui.ctx();
        let pointer = ctx.input(|i| i.pointer.hover_pos());
        let over = response.contains_pointer() && pointer.is_some_and(|p| r.contains(p));
        let typing = self.keyboard_captured || ctx.text_edit_focused();
        let enabled = !self.automated_benchmark && !self.file_busy && self.load.is_none();
        if over && !typing {
            let cursor = if self.panning {
                egui::CursorIcon::Grabbing
            } else if self.interaction_mode != InteractionMode::Select {
                egui::CursorIcon::Crosshair
            } else if let Some(point) = pointer {
                if self.hit_far_field(point, r) {
                    egui::CursorIcon::PointingHand
                } else if self.hit_source(point, r) || self.hit_probe(point, r).is_some() {
                    egui::CursorIcon::Grab
                } else {
                    match self.hit_gizmo(point, r) {
                        Some(GizmoHit::Scale) => egui::CursorIcon::ResizeNwSe,
                        Some(_) => egui::CursorIcon::Grab,
                        None if self.hit_handle(point, r).is_some()
                            || self.hit_internal_handle(point, r).is_some() =>
                        {
                            egui::CursorIcon::Grab
                        }
                        None => egui::CursorIcon::Default,
                    }
                }
            } else {
                egui::CursorIcon::Default
            };
            ctx.set_cursor_icon(cursor);
        }
        if enabled {
            if !typing && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                let cancelled_segment_start = self.interaction_mode
                    == InteractionMode::PlaceSegmentProbe
                    && self.segment_probe_start.take().is_some();
                let cancelled_area_start = self.interaction_mode == InteractionMode::PlaceAreaDisk
                    && self.area_probe_center.take().is_some();
                let source_dragging = std::mem::take(&mut self.source_dragging);
                let probe_drag = self.probe_drag.take();
                let drag = self.drag.take();
                match &drag {
                    Some(Drag::Translate { gizmo_before, .. }) => {
                        self.gizmo_pivot = *gizmo_before;
                    }
                    Some(Drag::Pivot { start, .. }) => self.gizmo_pivot = *start,
                    Some(Drag::Marquee { base, .. }) => {
                        self.set_span_selection(base.clone());
                    }
                    _ => {}
                }
                self.pending_span_click = None;
                if source_dragging
                    || probe_drag.is_some()
                    || drag.is_some()
                    || self.editor.editing()
                {
                    self.editor.cancel();
                    self.wave_source = self.editor.document.source;
                    self.wave_source_dirty = true;
                } else if !cancelled_segment_start && !cancelled_area_start {
                    self.custom.clear();
                    self.interaction_mode = InteractionMode::Select;
                }
                self.panning = false;
            }
            if !typing && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z)) {
                if ctx.input(|i| i.modifiers.shift) {
                    self.editor.redo()
                } else {
                    self.editor.undo()
                }
                self.clear_transient();
            }
            if over && !typing {
                if self.interaction_mode == InteractionMode::Select
                    && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::A))
                {
                    self.select_filtered();
                }
                if matches!(self.interaction_mode, InteractionMode::DrawCustom { .. }) {
                    if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.finish_custom();
                    }
                    if ctx.input(|i| i.key_pressed(egui::Key::Backspace)) {
                        self.custom.pop();
                    }
                } else if ctx.input(|i| i.key_pressed(egui::Key::Delete))
                    && let Some(id) = self.selected_probe
                {
                    let result = self.editor.delete_probe(id);
                    self.error(result);
                    self.probe_windows.remove(&id);
                    self.probe_traces.remove(&id);
                    self.curve_probe_traces.remove(&id);
                    self.area_probe_traces.remove(&id);
                    self.selected_probe = None;
                } else if ctx.input(|i| i.key_pressed(egui::Key::Delete))
                    && let Some((id, Some(index))) = self.selection
                {
                    let result = self.editor.remove_point(id, index);
                    if self.error(result).is_some() {
                        self.select_loop(id);
                    }
                } else if ctx.input(|i| i.key_pressed(egui::Key::Delete))
                    && let Some((id, Some(index))) = self.internal_selection
                {
                    let result = self.editor.remove_internal_boundary_point(id, index);
                    if self.error(result).is_some() {
                        self.select_baffle(id);
                    }
                }
                let p = pointer.unwrap();
                // Use only this frame's wheel events, so a panel's smoothed
                // scroll tail cannot zoom when the cursor enters the viewport.
                let wheel = ctx.input(|i| {
                    i.raw
                        .events
                        .iter()
                        .filter_map(|event| {
                            if let egui::Event::MouseWheel {
                                unit,
                                delta,
                                modifiers,
                                ..
                            } = event
                                && !modifiers.command
                                && !modifiers.ctrl
                            {
                                let multiplier = match unit {
                                    egui::MouseWheelUnit::Point => 1.0,
                                    egui::MouseWheelUnit::Line => 40.0,
                                    egui::MouseWheelUnit::Page => r.height(),
                                };
                                Some(delta.y * multiplier)
                            } else {
                                None
                            }
                        })
                        .sum::<f32>()
                });
                if wheel != 0.0 && self.drag.is_none() && !self.panning {
                    let before = self.world(p, r);
                    self.scale = (self.scale * (wheel as f64 * 0.002).exp()).clamp(20.0, 20_000.0);
                    let after = self.world(p, r);
                    self.center = self.center + before - after;
                }
                let primary = ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
                let secondary =
                    ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Secondary));
                let space = ctx.input(|i| i.key_down(egui::Key::Space));
                if secondary || primary && space {
                    self.panning = true;
                } else if primary && self.interaction_mode == InteractionMode::Select {
                    self.refresh_curves();
                    let modifiers = ctx.input(|input| input.modifiers);
                    if self.hit_source(p, r) {
                        self.editor.begin();
                        self.source_dragging = true;
                    } else if let Some(hit) = self.hit_probe(p, r) {
                        let id = hit.id();
                        self.select_probe(id);
                        if matches!(hit, ProbeHit::Boundary(_) | ProbeHit::AreaRegion(_)) {
                            self.probe_drag = None;
                        } else {
                            self.editor.begin();
                            self.probe_drag = Some(match hit {
                                ProbeHit::Point(id) => ProbeDrag::Point { id },
                                ProbeHit::SegmentEndpoint(id, start_endpoint) => {
                                    ProbeDrag::SegmentEndpoint { id, start_endpoint }
                                }
                                ProbeHit::SegmentBody(id) => {
                                    let probe = self
                                        .editor
                                        .document
                                        .probes
                                        .iter()
                                        .find(|probe| probe.id == id)
                                        .expect("hit probe exists");
                                    let ProbeTarget::Segment { start, end, .. } = probe.target
                                    else {
                                        unreachable!()
                                    };
                                    ProbeDrag::SegmentBody {
                                        id,
                                        anchor: self.world(p, r),
                                        start,
                                        end,
                                    }
                                }
                                ProbeHit::AreaDiskBody(id) => {
                                    let (center, radius) = self
                                        .editor
                                        .document
                                        .probes
                                        .iter()
                                        .find_map(|probe| match probe.target {
                                            ProbeTarget::AreaDisk { center, radius }
                                                if probe.id == id =>
                                            {
                                                Some((center, radius))
                                            }
                                            _ => None,
                                        })
                                        .expect("hit disk probe exists");
                                    ProbeDrag::AreaDiskBody {
                                        id,
                                        anchor: self.world(p, r),
                                        center,
                                        radius,
                                    }
                                }
                                ProbeHit::AreaDiskRadius(id) => {
                                    let center = self
                                        .editor
                                        .document
                                        .probes
                                        .iter()
                                        .find_map(|probe| match probe.target {
                                            ProbeTarget::AreaDisk { center, .. }
                                                if probe.id == id =>
                                            {
                                                Some(center)
                                            }
                                            _ => None,
                                        })
                                        .expect("hit disk probe exists");
                                    ProbeDrag::AreaDiskRadius { id, center }
                                }
                                ProbeHit::Boundary(_) | ProbeHit::AreaRegion(_) => unreachable!(),
                            });
                        }
                    } else if let Some((id, index)) = self.hit_handle(p, r) {
                        let control = GeometryControl::Loop(id, index);
                        self.select_control(control);
                        if let Some(point) = self.editor.control_point(control) {
                            self.editor.begin();
                            self.drag = Some(Drag::Translate {
                                anchor: self.world(p, r),
                                pivot: point,
                                gizmo_before: self.gizmo_pivot,
                                controls: vec![(control, point)],
                                moved: false,
                            });
                        }
                    } else if let Some((id, index)) = self.hit_internal_handle(p, r) {
                        let control = GeometryControl::Baffle(id, index);
                        self.select_control(control);
                        if let Some(point) = self.editor.control_point(control) {
                            self.editor.begin();
                            self.drag = Some(Drag::Translate {
                                anchor: self.world(p, r),
                                pivot: point,
                                gizmo_before: self.gizmo_pivot,
                                controls: vec![(control, point)],
                                moved: false,
                            });
                        }
                    } else if let Some(hit) = self.hit_gizmo(p, r) {
                        let pivot = self.selection_pivot().unwrap();
                        match hit {
                            GizmoHit::Pivot => {
                                self.drag = Some(Drag::Pivot {
                                    start: self.gizmo_pivot,
                                    offset: pivot - self.world(p, r),
                                });
                            }
                            GizmoHit::Rotate => {
                                let relative = self.world(p, r) - pivot;
                                self.editor.begin();
                                self.drag = Some(Drag::Rotate {
                                    pivot,
                                    start_angle: relative.y.atan2(relative.x),
                                    controls: self.selected_control_points(),
                                    moved: false,
                                });
                            }
                            GizmoHit::Scale => {
                                let relative = self.world(p, r) - pivot;
                                let padding = GIZMO_PADDING / self.scale;
                                self.editor.begin();
                                self.drag = Some(Drag::Scale {
                                    pivot,
                                    start_control_distance: (relative.norm() - padding)
                                        .max(f64::EPSILON),
                                    controls: self.selected_control_points(),
                                    moved: false,
                                });
                            }
                        }
                    } else {
                        let obstacle_hit = self.hit_curve(p, r);
                        let internal_hit = obstacle_hit
                            .is_none()
                            .then(|| self.hit_internal_curve(p, r))
                            .flatten();
                        let hit = obstacle_hit
                            .and_then(|(id, parameter)| {
                                self.editor
                                    .obstacle(id)
                                    .and_then(|obstacle| obstacle.spline.span_index(parameter))
                                    .map(|span| GeometrySpan::Loop(id, span))
                            })
                            .or_else(|| {
                                internal_hit.and_then(|(id, parameter)| {
                                    self.editor
                                        .internal_boundary(id)
                                        .and_then(|boundary| boundary.spline.span_index(parameter))
                                        .map(|span| GeometrySpan::Baffle(id, span))
                                })
                            })
                            .or_else(|| self.hit_outer_boundary(p, r).map(GeometrySpan::Outer))
                            .filter(|span| self.span_matches_filter(*span));
                        if let Some(span) = hit {
                            let curve_spans = self.curve_spans(span);
                            if modifiers.command && modifiers.shift {
                                self.toggle_curve(curve_spans);
                            } else if modifiers.command {
                                self.set_span_selection(curve_spans);
                            } else if modifiers.shift {
                                if self.selected_spans.contains(&span) {
                                    self.pending_span_click = Some(PendingSpanClick::Toggle(span));
                                } else {
                                    self.toggle_span(span);
                                }
                            } else if self.selected_spans.contains(&span) {
                                self.pending_span_click = Some(PendingSpanClick::Collapse(span));
                            } else {
                                self.set_span_selection(vec![span]);
                            }
                            if !(modifiers.command && modifiers.shift)
                                && self.selected_spans.contains(&span)
                                && let Some(pivot) = self.selection_pivot()
                                && self.transformable_curve_controls().is_some()
                            {
                                self.editor.begin();
                                self.drag = Some(Drag::Translate {
                                    anchor: self.world(p, r),
                                    pivot,
                                    gizmo_before: self.gizmo_pivot,
                                    controls: self.selected_control_points(),
                                    moved: false,
                                });
                            }
                        } else {
                            self.pending_span_click = None;
                            self.drag = Some(Drag::Marquee {
                                anchor: p,
                                current: p,
                                base: self.selected_spans.clone(),
                                operation: if modifiers.alt {
                                    MarqueeOperation::Subtract
                                } else if modifiers.shift {
                                    MarqueeOperation::Add
                                } else {
                                    MarqueeOperation::Replace
                                },
                            });
                        }
                    }
                }
                if self.panning {
                    let delta = ctx.input(|i| i.pointer.delta());
                    self.center = self.center
                        + Point2::new(-delta.x as f64 / self.scale, delta.y as f64 / self.scale);
                } else if response.dragged_by(egui::PointerButton::Primary) {
                    let world = self.world(p, r);
                    if self.source_dragging {
                        let position =
                            if self.snap_to_grid || ctx.input(|input| input.modifiers.shift) {
                                Point2::new(
                                    (world.x / self.snap_step).round() * self.snap_step,
                                    (world.y / self.snap_step).round() * self.snap_step,
                                )
                            } else {
                                world
                            };
                        self.wave_source.position = position;
                        self.wave_source.region = self
                            .wave_mesh
                            .as_ref()
                            .and_then(|mesh| mesh_region_at(mesh, position))
                            .unwrap_or(BACKGROUND_REGION);
                        self.editor.document.source = self.wave_source;
                        self.wave_source_dirty = true;
                    } else if let Some(probe_drag) = self.probe_drag.as_ref() {
                        let snap = |point: Point2| {
                            if self.snap_to_grid || ctx.input(|input| input.modifiers.shift) {
                                Point2::new(
                                    (point.x / self.snap_step).round() * self.snap_step,
                                    (point.y / self.snap_step).round() * self.snap_step,
                                )
                            } else {
                                point
                            }
                        };
                        let (id, target) = match *probe_drag {
                            ProbeDrag::Point { id } => (id, Some(ProbeTarget::Point(snap(world)))),
                            ProbeDrag::SegmentEndpoint { id, start_endpoint } => {
                                let target = self.editor.document.probes.iter().find_map(|probe| {
                                    if probe.id != id {
                                        return None;
                                    }
                                    let ProbeTarget::Segment { start, end, preset } = probe.target
                                    else {
                                        return None;
                                    };
                                    let (start, end) = if start_endpoint {
                                        (snap(world), end)
                                    } else {
                                        (start, snap(world))
                                    };
                                    ((end - start).norm() >= 1.0e-6)
                                        .then_some(ProbeTarget::Segment { start, end, preset })
                                });
                                (id, target)
                            }
                            ProbeDrag::SegmentBody {
                                id,
                                anchor,
                                start,
                                end,
                            } => {
                                let initial_center = (start + end) * 0.5;
                                let center = snap(initial_center + world - anchor);
                                let delta = center - initial_center;
                                (
                                    id,
                                    Some(ProbeTarget::Segment {
                                        start: start + delta,
                                        end: end + delta,
                                        preset: self
                                            .editor
                                            .document
                                            .probes
                                            .iter()
                                            .find_map(|probe| {
                                                (probe.id == id).then_some(probe.target)
                                            })
                                            .and_then(|target| match target {
                                                ProbeTarget::Segment { preset, .. } => Some(preset),
                                                _ => None,
                                            })
                                            .unwrap_or_default(),
                                    }),
                                )
                            }
                            ProbeDrag::AreaDiskBody {
                                id,
                                anchor,
                                center,
                                radius,
                            } => (
                                id,
                                Some(ProbeTarget::AreaDisk {
                                    center: snap(center + world - anchor),
                                    radius,
                                }),
                            ),
                            ProbeDrag::AreaDiskRadius { id, center } => {
                                let mut radius = (world - center).norm().max(1.0e-6);
                                if self.snap_to_grid || ctx.input(|input| input.modifiers.shift) {
                                    radius =
                                        (radius / self.snap_step).round().max(1.0) * self.snap_step;
                                }
                                (id, Some(ProbeTarget::AreaDisk { center, radius }))
                            }
                        };
                        if let Some(target) = target
                            && let Some(probe) = self
                                .editor
                                .document
                                .probes
                                .iter_mut()
                                .find(|probe| probe.id == id)
                        {
                            probe.target = target;
                            self.clear_probe_trace(id);
                        }
                    }
                    let snap_to_grid =
                        self.snap_to_grid || ctx.input(|input| input.modifiers.shift);
                    let snap_step = self.snap_step;
                    let snap = |point: Point2| {
                        if !snap_to_grid || !snap_step.is_finite() || snap_step <= 0.0 {
                            point
                        } else {
                            Point2::new(
                                (point.x / snap_step).round() * snap_step,
                                (point.y / snap_step).round() * snap_step,
                            )
                        }
                    };
                    let mut pivot_after = None;
                    let mut marquee_update = None;
                    let updates = match self.drag.as_mut() {
                        Some(Drag::Translate {
                            anchor,
                            pivot,
                            gizmo_before: _,
                            controls,
                            moved,
                        }) => {
                            *moved = true;
                            let target = snap(*pivot + world - *anchor);
                            let delta = target - *pivot;
                            pivot_after = Some(target);
                            Some(
                                controls
                                    .iter()
                                    .map(|(control, point)| (*control, *point + delta))
                                    .collect::<Vec<_>>(),
                            )
                        }
                        Some(Drag::Rotate {
                            pivot,
                            start_angle,
                            controls,
                            moved,
                        }) => {
                            *moved = true;
                            let relative = world - *pivot;
                            let mut angle = relative.y.atan2(relative.x) - *start_angle;
                            if ctx.input(|input| input.modifiers.shift) {
                                let step = 15.0_f64.to_radians();
                                angle = (angle / step).round() * step;
                            }
                            let (sin, cos) = angle.sin_cos();
                            Some(
                                controls
                                    .iter()
                                    .map(|(control, point)| {
                                        let relative = *point - *pivot;
                                        (
                                            *control,
                                            *pivot
                                                + Point2::new(
                                                    cos * relative.x - sin * relative.y,
                                                    sin * relative.x + cos * relative.y,
                                                ),
                                        )
                                    })
                                    .collect::<Vec<_>>(),
                            )
                        }
                        Some(Drag::Scale {
                            pivot,
                            start_control_distance,
                            controls,
                            moved,
                        }) => {
                            *moved = true;
                            let control_distance =
                                ((world - *pivot).norm() - GIZMO_PADDING / self.scale).max(0.0);
                            let mut factor =
                                (control_distance / *start_control_distance).clamp(1.0e-4, 1.0e4);
                            if ctx.input(|input| input.modifiers.shift) {
                                factor = ((factor * 10.0).round() / 10.0).clamp(0.1, 1.0e4);
                            }
                            Some(
                                controls
                                    .iter()
                                    .map(|(control, point)| {
                                        (*control, *pivot + (*point - *pivot) * factor)
                                    })
                                    .collect::<Vec<_>>(),
                            )
                        }
                        Some(Drag::Pivot { offset, .. }) => {
                            self.gizmo_pivot = Some(world + *offset);
                            None
                        }
                        Some(Drag::Marquee {
                            anchor,
                            current,
                            base,
                            operation,
                        }) => {
                            *current = p;
                            marquee_update = Some((*anchor, *current, base.clone(), *operation));
                            None
                        }
                        None => None,
                    };
                    if self.drag.is_none() {
                        self.pending_span_click = None;
                    }
                    if let Some(updates) = updates {
                        let result = self.editor.set_control_points(&updates);
                        self.error(result);
                        if !self.selected_spans.is_empty() {
                            self.gizmo_pivot = pivot_after.or(self.gizmo_pivot);
                        }
                    }
                    if let Some((anchor, current, base, operation)) = marquee_update {
                        let hits = self.spans_in_marquee(Rect::from_two_pos(anchor, current), r);
                        self.apply_marquee_selection(&base, hits, operation);
                    }
                }
                if response.double_clicked()
                    && self.interaction_mode == InteractionMode::Select
                    && !space
                    && (self.hit_far_field(p, r)
                        || (!self.hit_source(p, r)
                            && self.hit_handle(p, r).is_none()
                            && self.hit_internal_handle(p, r).is_none()))
                {
                    self.editor.commit();
                    self.probe_drag = None;
                    self.drag = None;
                    self.pending_span_click = None;
                    if self.hit_far_field(p, r) {
                        self.far_field_open = true;
                    } else if let Some(id) = self.hit_probe(p, r).map(ProbeHit::id) {
                        self.select_probe(id);
                        self.probe_windows.insert(id);
                    } else if let Some((id, t)) = self.hit_curve(p, r) {
                        let result = self.editor.insert(id, t);
                        if let Some(index) = self.error(result) {
                            self.select_control(GeometryControl::Loop(id, index));
                        }
                    } else if let Some((id, parameter)) = self.hit_internal_curve(p, r) {
                        let result = self.editor.insert_internal_boundary(id, parameter);
                        if let Some(index) = self.error(result) {
                            self.select_control(GeometryControl::Baffle(id, index));
                        }
                    }
                } else if response.clicked() && !space && !self.panning {
                    match self.interaction_mode {
                        InteractionMode::DrawPreset { role } => {
                            let center = self.world(p, r);
                            self.creation_role = role;
                            if role == CreationRole::InternalBoundary {
                                let result = self.editor.create_internal_boundary(
                                    OpenCubicSpline::uniform(vec![
                                        center + Point2::new(-0.25, 0.0),
                                        center + Point2::new(-0.08, 0.0),
                                        center + Point2::new(0.08, 0.0),
                                        center + Point2::new(0.25, 0.0),
                                    ])
                                    .unwrap(),
                                    self.region_at(center),
                                );
                                if let Some(id) = self.error(result) {
                                    self.select_baffle(id);
                                    self.interaction_mode = InteractionMode::Select;
                                }
                            } else {
                                let result = self.create_spline(
                                    PeriodicCubicSpline::rounded(center, 0.15),
                                    center,
                                );
                                if let Some(id) = self.error(result) {
                                    self.select_loop(id);
                                    self.interaction_mode = InteractionMode::Select;
                                }
                            }
                        }
                        InteractionMode::DrawCustom { role } => {
                            self.creation_role = role;
                            if role != CreationRole::InternalBoundary
                                && self.custom.len() >= 4
                                && self.screen(self.custom[0], r).distance(p) < 10.0
                            {
                                self.finish_custom();
                            } else if self.custom.len() < 128 {
                                self.custom.push(self.world(p, r));
                            } else {
                                self.message = "Maximum 128 control points".into();
                            }
                        }
                        InteractionMode::PlacePulse => {
                            self.wave_pending_pulse = Some(self.world(p, r));
                        }
                        InteractionMode::PlaceProbe => {
                            let result = self.editor.create_point_probe(self.world(p, r));
                            if let Some(id) = self.error(result) {
                                self.select_probe(id);
                                self.probe_windows.insert(id);
                            }
                        }
                        InteractionMode::PlaceSegmentProbe => {
                            let point = self.world(p, r);
                            if let Some(start) = self.segment_probe_start.take() {
                                let result = self.editor.create_segment_probe(start, point);
                                if let Some(id) = self.error(result) {
                                    self.select_probe(id);
                                    self.probe_windows.insert(id);
                                }
                            } else {
                                self.segment_probe_start = Some(point);
                            }
                        }
                        InteractionMode::PlaceAreaDisk => {
                            let point = self.world(p, r);
                            if let Some(center) = self.area_probe_center.take() {
                                let radius = (point - center).norm();
                                let result = self.editor.create_area_disk_probe(center, radius);
                                if let Some(id) = self.error(result) {
                                    self.select_probe(id);
                                    self.probe_windows.insert(id);
                                }
                            } else {
                                self.area_probe_center = Some(point);
                            }
                        }
                        InteractionMode::PlaceAreaRegion => {
                            let point = self.world(p, r);
                            if let Some(region) = self
                                .mesh
                                .as_ref()
                                .and_then(|mesh| mesh_region_at(mesh, point))
                            {
                                let result = self.editor.create_area_region_probe(region);
                                if let Some(id) = self.error(result) {
                                    self.select_probe(id);
                                    self.probe_windows.insert(id);
                                }
                            } else {
                                self.message = "Click inside a simulated subdomain".into();
                            }
                        }
                        InteractionMode::Select => {}
                    }
                }
            }
        }
        if !ctx.input(|i| i.pointer.primary_down()) {
            if std::mem::take(&mut self.source_dragging) {
                self.editor.commit();
            }
            if self.probe_drag.take().is_some() {
                self.editor.commit();
            }
            let drag = self.drag.take();
            let moved = matches!(
                &drag,
                Some(
                    Drag::Translate { moved: true, .. }
                        | Drag::Rotate { moved: true, .. }
                        | Drag::Scale { moved: true, .. }
                )
            );
            if matches!(
                &drag,
                Some(Drag::Translate { .. } | Drag::Rotate { .. } | Drag::Scale { .. })
            ) {
                self.editor.commit();
            }
            if let Some(Drag::Marquee {
                anchor,
                current,
                base,
                operation,
            }) = &drag
            {
                if anchor.distance(*current) >= 4.0 {
                    let hits = self.spans_in_marquee(Rect::from_two_pos(*anchor, *current), r);
                    self.apply_marquee_selection(base, hits, *operation);
                } else {
                    self.set_span_selection(base.clone());
                    if matches!(operation, MarqueeOperation::Replace) {
                        self.set_span_selection(vec![]);
                        self.region_selection = self.region_at(self.world(*anchor, r));
                    }
                }
            }
            if !moved && let Some(action) = self.pending_span_click.take() {
                match action {
                    PendingSpanClick::Collapse(span) => self.set_span_selection(vec![span]),
                    PendingSpanClick::Toggle(span) => self.toggle_span(span),
                }
            } else if moved {
                self.pending_span_click = None;
            }
            if !ctx.input(|i| i.pointer.button_down(egui::PointerButton::Secondary)) {
                self.panning = false;
            }
        }
        self.refresh_curves();
        painter.rect_filled(r, 0.0, Color32::from_rgb(16, 23, 31));
        if self.grid {
            for i in -10..=10 {
                let t = i as f64 / 10.0;
                let color = if i == 0 {
                    Color32::from_rgb(40, 57, 70)
                } else {
                    Color32::from_rgb(27, 39, 49)
                };
                for (a, b) in [
                    (Point2::new(t, -1.0), Point2::new(t, 1.0)),
                    (Point2::new(-1.0, t), Point2::new(1.0, t)),
                ] {
                    painter.line_segment(
                        [self.screen(a, r), self.screen(b, r)],
                        Stroke::new(1.0, color),
                    );
                }
            }
        }
        if self.show_materials
            && let Some(mesh) = &self.mesh
        {
            let mut regions = egui::Mesh::default();
            regions.reserve_vertices(mesh.triangles.len() * 3);
            regions.reserve_triangles(mesh.triangles.len());
            for triangle in &mesh.triangles {
                let color = self
                    .mesh_committed_scene
                    .region_material(triangle.region)
                    .map_or([70, 85, 96], |material| material.color);
                let alpha = if triangle.region == self.region_selection {
                    145
                } else {
                    85
                };
                let color = Color32::from_rgba_unmultiplied(color[0], color[1], color[2], alpha);
                let base = regions.vertices.len() as u32;
                for index in triangle.vertices {
                    regions.colored_vertex(self.screen(mesh.vertices[index].point, r), color);
                }
                regions.add_triangle(base, base + 1, base + 2);
            }
            painter.add(egui::Shape::mesh(regions));
        }
        if self.show_field
            && let Some(display) = wave_display
            && display.generation > 0
            && let Some(operator) = self.wave_display_operator(display)
        {
            let mut field = egui::Mesh::default();
            field.reserve_vertices(operator.degrees_of_freedom());
            field.reserve_triangles(operator.element_nodes().len() * 6);
            for (point, value) in operator.node_points().iter().zip(&display.current) {
                field.colored_vertex(self.screen(*point, r), field_color(*value, self.field_gain));
            }
            for nodes in operator.element_nodes() {
                for [a, b] in [[0, 3], [3, 1], [1, 4], [4, 2], [2, 5], [5, 0]] {
                    field.add_triangle(nodes[a], nodes[b], nodes[6]);
                }
            }
            painter.add(egui::Shape::mesh(field));
        }
        if self.show_amr_target
            && let (Some(mesh), Some(result)) = (&self.mesh, &self.solution_indicator_result)
            && result.element_targets.len() == mesh.triangles.len()
        {
            let minimum = self.amr_minimum_edge;
            let span = (self.amr_maximum_edge - minimum).max(f64::MIN_POSITIVE);
            let mut target_mesh = egui::Mesh::default();
            target_mesh.reserve_vertices(mesh.triangles.len() * 3);
            target_mesh.reserve_triangles(mesh.triangles.len());
            for (triangle, target) in mesh.triangles.iter().zip(&result.element_targets) {
                let fraction = ((*target - minimum) / span).clamp(0.0, 1.0) as f32;
                let color = amr_target_color(fraction);
                let base = target_mesh.vertices.len() as u32;
                for vertex in triangle.vertices {
                    target_mesh.colored_vertex(self.screen(mesh.vertices[vertex].point, r), color);
                }
                target_mesh.add_triangle(base, base + 1, base + 2);
            }
            painter.add(egui::Shape::mesh(target_mesh));
        }
        let domain = [
            Point2::new(-1.0, -1.0),
            Point2::new(1.0, -1.0),
            Point2::new(1.0, 1.0),
            Point2::new(-1.0, 1.0),
            Point2::new(-1.0, -1.0),
        ];
        painter.add(egui::Shape::line(
            domain.map(|p| self.screen(p, r)).to_vec(),
            Stroke::new(1.5, Color32::from_rgb(100, 123, 140)),
        ));
        if self.show_mesh
            && let Some(mesh) = &self.mesh
        {
            for (triangle_index, triangle) in mesh.triangles.iter().enumerate() {
                let low_quality = self.mesh_low_quality[triangle_index];
                let mesh_stroke = Stroke::new(
                    if low_quality { 1.1 } else { 0.65 },
                    if low_quality {
                        Color32::from_rgba_unmultiplied(248, 196, 112, 165)
                    } else {
                        Color32::from_rgba_unmultiplied(78, 123, 151, 105)
                    },
                );
                let positions = triangle
                    .vertices
                    .map(|index| self.screen(mesh.vertices[index].point, r));
                for edge in [[0, 1], [1, 2], [2, 0]] {
                    painter.line_segment([positions[edge[0]], positions[edge[1]]], mesh_stroke);
                }
            }
            if self.show_mesh_boundary {
                for edge in &mesh.boundary_edges {
                    let color = match edge.label {
                        BoundaryLabel::Outer(_) => Color32::from_rgb(142, 161, 175),
                        BoundaryLabel::Obstacle(_) => Color32::from_rgb(119, 155, 255),
                        BoundaryLabel::MaterialInterface(_) => Color32::from_rgb(102, 210, 178),
                        BoundaryLabel::Wall { side, .. } => match side {
                            BoundarySide::Exterior => Color32::from_rgb(235, 132, 115),
                            BoundarySide::Interior => Color32::from_rgb(235, 183, 115),
                        },
                        BoundaryLabel::InternalBoundary { side, .. } => match side {
                            InternalBoundarySide::Left => Color32::from_rgb(235, 132, 115),
                            InternalBoundarySide::Right => Color32::from_rgb(235, 183, 115),
                        },
                    };
                    painter.line_segment(
                        edge.vertices
                            .map(|index| self.screen(mesh.vertices[index].point, r)),
                        Stroke::new(2.0, color),
                    );
                }
            }
        }
        for side in self.selected_spans.iter().filter_map(|span| match span {
            GeometrySpan::Outer(side) => Some(*side),
            _ => None,
        }) {
            let selected_outer = match side {
                OuterSide::Bottom => [domain[0], domain[1]],
                OuterSide::Right => [domain[1], domain[2]],
                OuterSide::Top => [domain[2], domain[3]],
                OuterSide::Left => [domain[3], domain[4]],
            };
            painter.line_segment(
                selected_outer.map(|point| self.screen(point, r)),
                Stroke::new(3.5, SELECT),
            );
        }
        if self.reference && self.editor.document.draft != self.editor.document.accepted {
            for curve in &self.accepted_curves {
                self.draw_curve(&painter, r, curve, Color32::from_rgb(66, 100, 98), 3.0);
            }
            for curve in &self.accepted_internal_curves {
                self.draw_internal_curve(&painter, r, curve, Color32::from_rgb(85, 83, 70), 3.0);
            }
        }
        let color = match self.editor.acceptance {
            Acceptance::Valid => TEAL,
            Acceptance::Pending => GOLD,
            Acceptance::Invalid(_) => RED,
        };
        for curve in &self.draft_curves {
            self.draw_curve(&painter, r, curve, color, 2.0);
        }
        for curve in &self.draft_internal_curves {
            self.draw_internal_curve(&painter, r, curve, color, 3.0);
        }
        if self.show_boundary_conditions {
            for side in OuterSide::ALL {
                let points = outer_side_points(side).map(|point| self.screen(point, r));
                painter.line_segment(
                    points,
                    Stroke::new(
                        2.5,
                        outer_boundary_condition_color(
                            self.editor.document.draft.outer_boundaries.get(side),
                        ),
                    ),
                );
            }
            for obstacle in &self.editor.document.draft.obstacles {
                if !matches!(obstacle.role, LoopRole::Hole { .. }) {
                    continue;
                }
                let Some(curve) = self
                    .draft_curves
                    .iter()
                    .find(|curve| curve.id == obstacle.id)
                else {
                    continue;
                };
                for (span, condition) in obstacle.span_conditions.iter().copied().enumerate() {
                    if let Some(bounds) = obstacle.spline.span_bounds(span) {
                        self.draw_curve_span_colored(
                            &painter,
                            r,
                            curve,
                            bounds,
                            boundary_condition_color(condition),
                            2.5,
                        );
                    }
                }
            }
            for boundary in &self.editor.document.draft.internal_boundaries {
                let Some(curve) = self
                    .draft_internal_curves
                    .iter()
                    .find(|curve| curve.id == boundary.id)
                else {
                    continue;
                };
                for (span, law) in boundary.span_laws.iter().copied().enumerate() {
                    let Some(bounds) = boundary.spline.span_bounds(span) else {
                        continue;
                    };
                    match law.coupling {
                        InternalBoundaryCoupling::Independent => {
                            self.draw_internal_span_condition(
                                &painter,
                                r,
                                curve,
                                bounds,
                                InternalBoundarySide::Left,
                                boundary_condition_color(law.left),
                            );
                            self.draw_internal_span_condition(
                                &painter,
                                r,
                                curve,
                                bounds,
                                InternalBoundarySide::Right,
                                boundary_condition_color(law.right),
                            );
                        }
                        InternalBoundaryCoupling::ThinGap { .. } => {
                            for side in [InternalBoundarySide::Left, InternalBoundarySide::Right] {
                                self.draw_internal_span_condition(
                                    &painter,
                                    r,
                                    curve,
                                    bounds,
                                    side,
                                    Color32::from_rgb(215, 123, 244),
                                );
                            }
                        }
                    }
                }
            }
        }
        for selected in &self.selected_spans {
            match *selected {
                GeometrySpan::Loop(id, span) => {
                    if let Some(curve) = self.draft_curves.iter().find(|curve| curve.id == id)
                        && let Some(bounds) = self
                            .editor
                            .obstacle(id)
                            .and_then(|obstacle| obstacle.spline.span_bounds(span))
                    {
                        self.draw_curve_span(&painter, r, curve, bounds);
                    }
                }
                GeometrySpan::Baffle(id, span) => {
                    if let Some(curve) = self
                        .draft_internal_curves
                        .iter()
                        .find(|curve| curve.id == id)
                        && let Some(bounds) = self
                            .editor
                            .internal_boundary(id)
                            .and_then(|boundary| boundary.spline.span_bounds(span))
                    {
                        self.draw_internal_span_face(&painter, r, curve, bounds, self.baffle_face);
                    }
                }
                GeometrySpan::Outer(side) => {
                    painter.line_segment(
                        outer_side_points(side).map(|point| self.screen(point, r)),
                        Stroke::new(3.5, Color32::WHITE),
                    );
                }
            }
        }
        if let Some(endpoint) = self.selected_topology_endpoint() {
            let center = self.screen(endpoint, r);
            let radius = 6.0;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    center + egui::vec2(0.0, -radius),
                    center + egui::vec2(radius, 0.0),
                    center + egui::vec2(0.0, radius),
                    center + egui::vec2(-radius, 0.0),
                ],
                GOLD,
                Stroke::new(1.5, Color32::from_rgb(16, 23, 31)),
            ));
        }
        // Repeated knots are curve points rather than spline controls. Show
        // them independently so C1 joins and C0 corners cannot be mistaken for
        // the circular control handles.
        if self.handles {
            for obstacle in &self.editor.document.draft.obstacles {
                for (breakpoint, multiplicity) in
                    obstacle.spline.multiplicities().iter().copied().enumerate()
                {
                    if multiplicity <= 1 {
                        continue;
                    }
                    let parameter = obstacle.spline.knots()[breakpoint];
                    let center = self.screen(obstacle.spline.evaluate(parameter), r);
                    let radius = if multiplicity == 3 { 5.0 } else { 4.0 };
                    let points = [
                        center + egui::vec2(0.0, -radius),
                        center + egui::vec2(radius, 0.0),
                        center + egui::vec2(0.0, radius),
                        center + egui::vec2(-radius, 0.0),
                    ];
                    painter.add(egui::Shape::convex_polygon(
                        points.to_vec(),
                        if multiplicity == 3 {
                            GOLD
                        } else {
                            Color32::from_rgb(23, 34, 44)
                        },
                        Stroke::new(1.5, GOLD),
                    ));
                }
            }
            for boundary in &self.editor.document.draft.internal_boundaries {
                for (slot, multiplicity) in
                    boundary.spline.multiplicities().iter().copied().enumerate()
                {
                    if multiplicity <= 1 {
                        continue;
                    }
                    let breakpoint = slot + 1;
                    let parameter = boundary.spline.breakpoint(breakpoint).unwrap();
                    let center = self.screen(boundary.spline.evaluate(parameter), r);
                    let radius = if multiplicity == 3 { 5.0 } else { 4.0 };
                    let points = [
                        center + egui::vec2(0.0, -radius),
                        center + egui::vec2(radius, 0.0),
                        center + egui::vec2(0.0, radius),
                        center + egui::vec2(-radius, 0.0),
                    ];
                    painter.add(egui::Shape::convex_polygon(
                        points.to_vec(),
                        if multiplicity == 3 {
                            GOLD
                        } else {
                            Color32::from_rgb(23, 34, 44)
                        },
                        Stroke::new(1.5, GOLD),
                    ));
                }
            }
        }
        for o in &self.editor.document.draft.obstacles {
            let selected = self.selection.is_some_and(|s| s.0 == o.id)
                || self
                    .selected_spans
                    .iter()
                    .any(|span| matches!(span, GeometrySpan::Loop(id, _) if *id == o.id));
            let points = o.spline.controls();
            if self.polygon {
                let mut polygon: Vec<_> = points.iter().map(|p| self.screen(*p, r)).collect();
                polygon.push(polygon[0]);
                painter.add(egui::Shape::line(
                    polygon,
                    Stroke::new(
                        1.0,
                        if selected {
                            Color32::from_rgb(91, 110, 127)
                        } else {
                            Color32::from_rgb(46, 66, 80)
                        },
                    ),
                ));
            }
            if self.handles {
                for (i, p) in points.iter().enumerate() {
                    let pos = self.screen(*p, r);
                    let active = self.selection == Some((o.id, Some(i)));
                    painter.circle_filled(
                        pos,
                        if active { 6.0 } else { 4.0 },
                        if active {
                            GOLD
                        } else {
                            Color32::from_rgb(23, 34, 44)
                        },
                    );
                    painter.circle_stroke(
                        pos,
                        if active { 6.0 } else { 4.0 },
                        Stroke::new(
                            1.5,
                            if selected {
                                SELECT
                            } else {
                                Color32::from_rgb(106, 133, 150)
                            },
                        ),
                    );
                }
            }
        }
        for boundary in &self.editor.document.draft.internal_boundaries {
            let selected =
                self.internal_selection
                    .is_some_and(|selection| selection.0 == boundary.id)
                    || self.selected_spans.iter().any(
                        |span| matches!(span, GeometrySpan::Baffle(id, _) if *id == boundary.id),
                    );
            let points = boundary.spline.controls();
            if self.polygon {
                painter.add(egui::Shape::line(
                    points.iter().map(|point| self.screen(*point, r)).collect(),
                    Stroke::new(
                        1.0,
                        if selected {
                            Color32::from_rgb(120, 103, 82)
                        } else {
                            Color32::from_rgb(68, 60, 52)
                        },
                    ),
                ));
            }
            if self.handles {
                for (index, point) in points.iter().enumerate() {
                    let position = self.screen(*point, r);
                    let active = self.internal_selection == Some((boundary.id, Some(index)));
                    painter.circle_filled(
                        position,
                        if active { 6.0 } else { 4.0 },
                        if active {
                            GOLD
                        } else {
                            Color32::from_rgb(23, 34, 44)
                        },
                    );
                    painter.circle_stroke(
                        position,
                        if active { 6.0 } else { 4.0 },
                        Stroke::new(1.5, if selected { SELECT } else { GOLD }),
                    );
                }
            }
        }
        if self.transformable_curve_controls().is_some()
            && let Some(pivot) = self.selection_pivot()
        {
            let center = self.screen(pivot, r);
            let radius = self.gizmo_radius(r, pivot);
            painter.circle_stroke(center, radius, Stroke::new(1.5, SELECT));
            painter.circle_filled(center + egui::vec2(radius, 0.0), 4.0, SELECT);
            let scale_handle = self.scale_handle_position(r, pivot);
            let scale_rect = Rect::from_center_size(scale_handle, egui::vec2(10.0, 10.0));
            painter.rect_filled(scale_rect, 1.0, Color32::from_rgb(16, 23, 31));
            painter.rect_stroke(
                scale_rect,
                1.0,
                Stroke::new(2.0, SELECT),
                egui::StrokeKind::Inside,
            );
            painter.circle_filled(center, 5.0, GOLD);
            painter.line_segment(
                [
                    center + egui::vec2(-8.0, 0.0),
                    center + egui::vec2(8.0, 0.0),
                ],
                Stroke::new(1.0, GOLD),
            );
            painter.line_segment(
                [
                    center + egui::vec2(0.0, -8.0),
                    center + egui::vec2(0.0, 8.0),
                ],
                Stroke::new(1.0, GOLD),
            );
        }
        if !self.custom.is_empty() {
            let mut points = self.custom.clone();
            if over
                && let Some(p) = pointer
                && points.len() < 128
            {
                points.push(self.world(p, r));
            }
            let polygon: Vec<_> = points.iter().map(|p| self.screen(*p, r)).collect();
            painter.add(egui::Shape::line(polygon, Stroke::new(1.0, GOLD)));
            for (i, p) in self.custom.iter().enumerate() {
                painter.circle_stroke(
                    self.screen(*p, r),
                    if i == 0 { 7.0 } else { 4.0 },
                    Stroke::new(1.5, GOLD),
                );
            }
            if self.custom.len() >= 4 {
                let options = SamplingOptions {
                    tolerance: 0.6 / self.scale,
                    ..Default::default()
                };
                if self.creation_role == CreationRole::InternalBoundary {
                    if let Ok(spline) = OpenCubicSpline::uniform(self.custom.clone())
                        && let Ok(samples) = sample_open(&spline, options)
                    {
                        self.draw_internal_curve(
                            &painter,
                            r,
                            &InternalCurve {
                                id: InternalBoundaryId(0),
                                samples,
                            },
                            GOLD,
                            2.0,
                        );
                    }
                } else if let Ok(spline) = PeriodicCubicSpline::uniform(self.custom.clone())
                    && let Ok(samples) = sample(&spline, options)
                {
                    self.draw_curve(
                        &painter,
                        r,
                        &Curve {
                            id: ObstacleId(0),
                            samples,
                        },
                        GOLD,
                        2.0,
                    );
                }
            }
        }
        if let Some(Drag::Marquee {
            anchor,
            current,
            operation,
            ..
        }) = &self.drag
        {
            let marquee = Rect::from_two_pos(*anchor, *current).intersect(r);
            let color = match operation {
                MarqueeOperation::Replace => SELECT,
                MarqueeOperation::Add => GOLD,
                MarqueeOperation::Subtract => RED,
            };
            painter.rect_filled(
                marquee,
                0.0,
                Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 24),
            );
            painter.rect_stroke(
                marquee,
                0.0,
                Stroke::new(1.0, color),
                egui::StrokeKind::Inside,
            );
        }
        if self.sampling_warning {
            painter.text(
                r.left_bottom() + egui::vec2(18.0, -18.0),
                egui::Align2::LEFT_BOTTOM,
                "Render sampling limit reached; zoom out to see curves",
                egui::FontId::proportional(12.0),
                GOLD,
            );
        }
        if let Some(pointer) = pointer.filter(|point| r.contains(*point)) {
            match self.interaction_mode {
                InteractionMode::DrawPreset {
                    role: CreationRole::InternalBoundary,
                } => {
                    let center = self.world(pointer, r);
                    painter.line_segment(
                        [
                            self.screen(center + Point2::new(-0.25, 0.0), r),
                            self.screen(center + Point2::new(0.25, 0.0), r),
                        ],
                        Stroke::new(2.0, GOLD),
                    );
                }
                InteractionMode::DrawPreset { .. } => {
                    painter.circle_stroke(
                        pointer,
                        (0.15 * self.scale) as f32,
                        Stroke::new(2.0, GOLD),
                    );
                }
                InteractionMode::PlacePulse => {
                    painter.circle_stroke(
                        pointer,
                        (self.pulse_width as f64 * self.scale) as f32,
                        Stroke::new(1.5, GOLD),
                    );
                }
                InteractionMode::PlaceProbe => {
                    painter.circle_filled(pointer, 5.0, Color32::from_rgb(16, 23, 31));
                    painter.circle_stroke(pointer, 7.0, Stroke::new(2.0, SELECT));
                }
                InteractionMode::PlaceSegmentProbe => {
                    if let Some(start) = self.segment_probe_start {
                        painter.line_segment(
                            [self.screen(start, r), pointer],
                            Stroke::new(2.0, SELECT),
                        );
                        painter.circle_filled(self.screen(start, r), 4.0, SELECT);
                    } else {
                        painter.circle_stroke(pointer, 6.0, Stroke::new(2.0, SELECT));
                    }
                }
                InteractionMode::PlaceAreaDisk => {
                    if let Some(center) = self.area_probe_center {
                        let center = self.screen(center, r);
                        painter.circle_filled(
                            center,
                            center.distance(pointer),
                            Color32::from_rgba_unmultiplied(72, 166, 255, 24),
                        );
                        painter.circle_stroke(
                            center,
                            center.distance(pointer),
                            Stroke::new(2.0, SELECT),
                        );
                    } else {
                        painter.circle_stroke(pointer, 6.0, Stroke::new(2.0, SELECT));
                    }
                }
                InteractionMode::PlaceAreaRegion => {
                    painter.circle_filled(pointer, 7.0, SELECT);
                    painter.text(
                        pointer,
                        egui::Align2::CENTER_CENTER,
                        "A",
                        egui::FontId::monospace(9.0),
                        Color32::WHITE,
                    );
                }
                _ => {}
            }
        }
        if self.editor.document.far_field.enabled && self.show_far_field_contour {
            let half_extent = 1.0 - self.editor.document.far_field.inset;
            let corners = [
                Point2::new(-half_extent, -half_extent),
                Point2::new(half_extent, -half_extent),
                Point2::new(half_extent, half_extent),
                Point2::new(-half_extent, half_extent),
                Point2::new(-half_extent, -half_extent),
            ]
            .map(|point| self.screen(point, r));
            painter.add(egui::Shape::line(
                corners.to_vec(),
                Stroke::new(1.5, Color32::from_rgb(172, 122, 255)),
            ));
            painter.text(
                corners[2] + egui::vec2(-4.0, 4.0),
                egui::Align2::RIGHT_TOP,
                "FF",
                egui::FontId::monospace(9.0),
                Color32::from_rgb(203, 177, 255),
            );
        }
        if self.wave_source.enabled {
            let center = self.screen(self.wave_source.position, r);
            painter.circle_stroke(center, 7.0, Stroke::new(2.0, GOLD));
            painter.line_segment(
                [
                    center + egui::vec2(-10.0, 0.0),
                    center + egui::vec2(10.0, 0.0),
                ],
                Stroke::new(1.0, GOLD),
            );
            painter.line_segment(
                [
                    center + egui::vec2(0.0, -10.0),
                    center + egui::vec2(0.0, 10.0),
                ],
                Stroke::new(1.0, GOLD),
            );
        }
        if self.show_point_probes
            || self.show_line_probes
            || self.show_boundary_probes
            || self.show_area_probes
        {
            for probe in &self.editor.document.probes {
                if !self.probe_visible(probe.target) {
                    continue;
                }
                let selected = self.selected_probe == Some(probe.id);
                let color = if self.probe_status.contains_key(&probe.id) {
                    RED
                } else if !probe.enabled {
                    Color32::from_rgb(112, 130, 143)
                } else {
                    Color32::from_rgb(probe.color[0], probe.color[1], probe.color[2])
                };
                let label_at = match probe.target {
                    ProbeTarget::Point(position) => {
                        let center = self.screen(position, r);
                        painter.circle_filled(center, if selected { 6.0 } else { 4.5 }, color);
                        painter.circle_stroke(
                            center,
                            if selected { 9.0 } else { 7.0 },
                            Stroke::new(if selected { 2.0 } else { 1.3 }, Color32::WHITE),
                        );
                        center
                    }
                    ProbeTarget::Segment { start, end, .. } => {
                        let a = self.screen(start, r);
                        let b = self.screen(end, r);
                        painter.line_segment(
                            [a, b],
                            Stroke::new(if selected { 3.0 } else { 2.0 }, color),
                        );
                        let midpoint = a + (b - a) * 0.5;
                        let direction = b - a;
                        let length = direction.length().max(1.0);
                        let normal = egui::vec2(direction.y, -direction.x) / length;
                        let tip = midpoint + normal * 18.0;
                        painter.arrow(midpoint, tip - midpoint, Stroke::new(1.5, color));
                        if selected {
                            for endpoint in [a, b] {
                                painter.circle_filled(endpoint, 5.0, color);
                                painter.circle_stroke(
                                    endpoint,
                                    7.0,
                                    Stroke::new(1.5, Color32::WHITE),
                                );
                            }
                        }
                        midpoint
                    }
                    ProbeTarget::Boundary(target) => {
                        let Some(path) =
                            Self::boundary_probe_path(&self.editor.document.draft, target)
                        else {
                            continue;
                        };
                        for segment in &path.segments {
                            painter.line_segment(
                                [
                                    self.screen(segment.points[0], r),
                                    self.screen(segment.points[1], r),
                                ],
                                Stroke::new(if selected { 4.0 } else { 2.5 }, color),
                            );
                        }
                        let mut remaining = path.length * 0.5;
                        let mut midpoint = self.screen(path.segments[0].points[0], r);
                        for segment in &path.segments {
                            let length = (segment.points[1] - segment.points[0]).norm();
                            if remaining <= length {
                                midpoint = self.screen(
                                    segment.points[0].lerp(
                                        segment.points[1],
                                        if length > 0.0 {
                                            remaining / length
                                        } else {
                                            0.0
                                        },
                                    ),
                                    r,
                                );
                                break;
                            }
                            remaining -= length;
                        }
                        painter.circle_filled(midpoint, if selected { 7.0 } else { 5.5 }, color);
                        painter.circle_stroke(
                            midpoint,
                            if selected { 9.0 } else { 7.5 },
                            Stroke::new(1.5, Color32::WHITE),
                        );
                        midpoint
                    }
                    ProbeTarget::AreaDisk { center, radius } => {
                        let center = self.screen(center, r);
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
                        if selected {
                            let handle = center + egui::vec2(radius, 0.0);
                            painter.circle_filled(handle, 5.0, color);
                            painter.circle_stroke(handle, 7.0, Stroke::new(1.5, Color32::WHITE));
                        }
                        center
                    }
                    ProbeTarget::AreaRegion { region } => {
                        if region == BACKGROUND_REGION {
                            let domain = Rect::from_two_pos(
                                self.screen(Point2::new(-1.0, 1.0), r),
                                self.screen(Point2::new(1.0, -1.0), r),
                            );
                            painter.rect_stroke(
                                domain,
                                0.0,
                                Stroke::new(if selected { 3.0 } else { 1.5 }, color),
                                egui::StrokeKind::Inside,
                            );
                        } else if let Some(loop_) = self
                            .editor
                            .document
                            .draft
                            .obstacles
                            .iter()
                            .find(|loop_| loop_.role.interior() == Some(region))
                            && let Some(curve) =
                                self.draft_curves.iter().find(|curve| curve.id == loop_.id)
                        {
                            self.draw_curve(
                                &painter,
                                r,
                                curve,
                                color,
                                if selected { 4.0 } else { 2.5 },
                            );
                        }
                        let center = self.screen(self.area_region_anchor(region), r);
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
                        center
                    }
                };
                painter.text(
                    label_at + egui::vec2(10.0, -10.0),
                    egui::Align2::LEFT_BOTTOM,
                    probe.id.0.to_string(),
                    egui::FontId::monospace(10.0),
                    color,
                );
            }
        }
        if let Some(texture) = &self.logo_texture {
            let width = (r.width() * 0.22)
                .clamp(96.0, 210.0)
                .min((r.width() - 28.0).max(0.0));
            if width >= 96.0 {
                let size = egui::vec2(width, width * 88.0 / 216.0);
                let logo_rect =
                    Rect::from_min_size(Pos2::new(r.right() - size.x - 14.0, r.top() + 14.0), size);
                painter.image(
                    texture.id(),
                    logo_rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }
        self.interaction_mode_overlay(ctx, r);
        r
    }

    fn interaction_mode_overlay(&mut self, ctx: &egui::Context, viewport: Rect) {
        let (title, hint) = match self.interaction_mode {
            InteractionMode::Select => return,
            InteractionMode::DrawPreset { role } => (
                match role {
                    CreationRole::Hole => "Drawing hole · Circle",
                    CreationRole::MaterialInterface => "Drawing interface · Circle",
                    CreationRole::InternalBoundary => "Drawing baffle · Straight",
                },
                "Click to place",
            ),
            InteractionMode::DrawCustom { role } => (
                match role {
                    CreationRole::Hole => "Drawing hole · Custom",
                    CreationRole::MaterialInterface => "Drawing interface · Custom",
                    CreationRole::InternalBoundary => "Drawing baffle · Custom",
                },
                "Click to add control points",
            ),
            InteractionMode::PlacePulse => ("Placing pulse", "Click repeatedly to inject"),
            InteractionMode::PlaceProbe => ("Placing point probes", "Click repeatedly to add"),
            InteractionMode::PlaceSegmentProbe => (
                "Placing line probes",
                if self.segment_probe_start.is_some() {
                    "Click the end point"
                } else {
                    "Click the start point"
                },
            ),
            InteractionMode::PlaceAreaDisk => (
                "Placing disk probes",
                if self.area_probe_center.is_some() {
                    "Click the radius"
                } else {
                    "Click the center"
                },
            ),
            InteractionMode::PlaceAreaRegion => {
                ("Placing subdomain probes", "Click inside a subdomain")
            }
        };
        egui::Area::new("interaction_mode_overlay".into())
            .fixed_pos(viewport.left_top() + egui::vec2(12.0, 12.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.strong(title);
                        ui.label(hint);
                        if matches!(self.interaction_mode, InteractionMode::DrawCustom { .. }) {
                            ui.label(format!("{} points", self.custom.len()));
                            if ui
                                .add_enabled(self.custom.len() >= 4, egui::Button::new("Finish"))
                                .clicked()
                            {
                                self.finish_custom();
                            }
                        }
                        let cancel_label = if matches!(
                            self.interaction_mode,
                            InteractionMode::PlacePulse
                                | InteractionMode::PlaceProbe
                                | InteractionMode::PlaceSegmentProbe
                                | InteractionMode::PlaceAreaDisk
                                | InteractionMode::PlaceAreaRegion
                        ) {
                            "Done"
                        } else {
                            "Cancel"
                        };
                        if ui.button(cancel_label).clicked() {
                            self.custom.clear();
                            self.segment_probe_start = None;
                            self.area_probe_center = None;
                            self.interaction_mode = InteractionMode::Select;
                        }
                    });
                });
            });
    }

    fn hit_outer_boundary(&self, p: Pos2, r: Rect) -> Option<OuterSide> {
        let point = self.world(p, r);
        [
            (
                OuterSide::Bottom,
                Point2::new(-1.0, -1.0),
                Point2::new(1.0, -1.0),
            ),
            (
                OuterSide::Right,
                Point2::new(1.0, -1.0),
                Point2::new(1.0, 1.0),
            ),
            (
                OuterSide::Top,
                Point2::new(1.0, 1.0),
                Point2::new(-1.0, 1.0),
            ),
            (
                OuterSide::Left,
                Point2::new(-1.0, 1.0),
                Point2::new(-1.0, -1.0),
            ),
        ]
        .into_iter()
        .map(|(side, a, b)| (side, point_segment_distance(point, a, b)))
        .filter(|(_, distance)| *distance * self.scale <= 8.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(side, _)| side)
    }

    fn curve_spans(&self, span: GeometrySpan) -> Vec<GeometrySpan> {
        match span {
            GeometrySpan::Outer(_) => OuterSide::ALL
                .into_iter()
                .map(GeometrySpan::Outer)
                .collect(),
            GeometrySpan::Loop(id, _) => self
                .editor
                .obstacle(id)
                .map(|obstacle| {
                    (0..obstacle.spline.intervals().len())
                        .map(|span| GeometrySpan::Loop(id, span))
                        .collect()
                })
                .unwrap_or_default(),
            GeometrySpan::Baffle(id, _) => self
                .editor
                .internal_boundary(id)
                .map(|boundary| {
                    (0..boundary.spline.intervals().len())
                        .map(|span| GeometrySpan::Baffle(id, span))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    fn hit_gizmo(&self, point: Pos2, r: Rect) -> Option<GizmoHit> {
        self.transformable_curve_controls()?;
        let pivot = self.selection_pivot()?;
        let center = self.screen(pivot, r);
        let distance = center.distance(point);
        if distance <= 8.0 {
            Some(GizmoHit::Pivot)
        } else if self.scale_handle_position(r, pivot).distance(point) <= 8.0 {
            Some(GizmoHit::Scale)
        } else if (distance - self.gizmo_radius(r, pivot)).abs() <= 7.0 {
            Some(GizmoHit::Rotate)
        } else {
            None
        }
    }

    fn gizmo_radius(&self, r: Rect, pivot: Point2) -> f32 {
        let center = self.screen(pivot, r);
        self.selected_control_points()
            .iter()
            .map(|(_, point)| center.distance(self.screen(*point, r)))
            .fold(22.0_f32, f32::max)
            + GIZMO_PADDING as f32
    }

    fn scale_handle_position(&self, r: Rect, pivot: Point2) -> Pos2 {
        let center = self.screen(pivot, r);
        let offset = self.gizmo_radius(r, pivot) * std::f32::consts::FRAC_1_SQRT_2;
        center + egui::vec2(offset, offset)
    }

    fn hit_handle(&self, p: Pos2, r: Rect) -> Option<(ObstacleId, usize)> {
        if !self.handles {
            return None;
        }
        self.editor
            .document
            .draft
            .obstacles
            .iter()
            .flat_map(|o| {
                o.spline
                    .controls()
                    .iter()
                    .enumerate()
                    .map(move |(i, c)| (o.id, i, self.screen(*c, r).distance(p)))
            })
            .filter(|x| x.2 <= 10.0)
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(id, i, _)| (id, i))
    }

    fn hit_source(&self, point: Pos2, viewport: Rect) -> bool {
        self.wave_source.enabled
            && self
                .screen(self.wave_source.position, viewport)
                .distance(point)
                <= 12.0
    }

    fn hit_far_field(&self, point: Pos2, viewport: Rect) -> bool {
        if !self.editor.document.far_field.enabled || !self.show_far_field_contour {
            return false;
        }
        let half_extent = 1.0 - self.editor.document.far_field.inset;
        let corners = [
            Point2::new(-half_extent, -half_extent),
            Point2::new(half_extent, -half_extent),
            Point2::new(half_extent, half_extent),
            Point2::new(-half_extent, half_extent),
        ]
        .map(|world| self.screen(world, viewport));
        corners
            .iter()
            .copied()
            .zip(corners.iter().copied().cycle().skip(1))
            .take(4)
            .any(|(start, end)| {
                point_segment_distance(
                    Point2::new(point.x as f64, point.y as f64),
                    Point2::new(start.x as f64, start.y as f64),
                    Point2::new(end.x as f64, end.y as f64),
                ) <= 7.0
            })
            || corners[2].distance(point) <= 24.0
    }

    fn area_region_anchor(&self, region: RegionId) -> Point2 {
        self.editor
            .document
            .draft
            .obstacles
            .iter()
            .find(|loop_| loop_.role.interior() == Some(region))
            .map(|loop_| {
                let controls = loop_.spline.controls();
                controls
                    .iter()
                    .copied()
                    .fold(Point2::default(), |sum, point| sum + point)
                    / controls.len().max(1) as f64
            })
            .unwrap_or_else(|| Point2::new(-0.86, 0.86))
    }

    fn hit_probe(&self, point: Pos2, viewport: Rect) -> Option<ProbeHit> {
        self.editor
            .document
            .probes
            .iter()
            .rev()
            .filter(|probe| self.probe_visible(probe.target))
            .find_map(|probe| match probe.target {
                ProbeTarget::Point(position) => (self.screen(position, viewport).distance(point)
                    <= 10.0)
                    .then_some(ProbeHit::Point(probe.id)),
                ProbeTarget::Segment { start, end, .. } => {
                    let a = self.screen(start, viewport);
                    let b = self.screen(end, viewport);
                    if a.distance(point) <= 10.0 {
                        Some(ProbeHit::SegmentEndpoint(probe.id, true))
                    } else if b.distance(point) <= 10.0 {
                        Some(ProbeHit::SegmentEndpoint(probe.id, false))
                    } else if point_segment_distance(
                        Point2::new(point.x as f64, point.y as f64),
                        Point2::new(a.x as f64, a.y as f64),
                        Point2::new(b.x as f64, b.y as f64),
                    ) <= 7.0
                    {
                        Some(ProbeHit::SegmentBody(probe.id))
                    } else {
                        None
                    }
                }
                ProbeTarget::Boundary(target) => {
                    let path = Self::boundary_probe_path(&self.editor.document.draft, target)?;
                    let mut remaining = path.length * 0.5;
                    let mut badge = path.segments.first()?.points[0];
                    for segment in path.segments {
                        let length = (segment.points[1] - segment.points[0]).norm();
                        if remaining <= length {
                            badge = segment.points[0].lerp(
                                segment.points[1],
                                if length > 0.0 {
                                    remaining / length
                                } else {
                                    0.0
                                },
                            );
                            break;
                        }
                        remaining -= length;
                    }
                    (self.screen(badge, viewport).distance(point) <= 11.0)
                        .then_some(ProbeHit::Boundary(probe.id))
                }
                ProbeTarget::AreaDisk { center, radius } => {
                    let center = self.screen(center, viewport);
                    let radius = (radius * self.scale) as f32;
                    let radius_handle = center + egui::vec2(radius, 0.0);
                    if radius_handle.distance(point) <= 10.0 {
                        Some(ProbeHit::AreaDiskRadius(probe.id))
                    } else if center.distance(point) <= radius + 7.0 {
                        Some(ProbeHit::AreaDiskBody(probe.id))
                    } else {
                        None
                    }
                }
                ProbeTarget::AreaRegion { region } => (self
                    .screen(self.area_region_anchor(region), viewport)
                    .distance(point)
                    <= 12.0)
                    .then_some(ProbeHit::AreaRegion(probe.id)),
            })
    }

    fn probe_visible(&self, target: ProbeTarget) -> bool {
        match target {
            ProbeTarget::Point(_) => self.show_point_probes,
            ProbeTarget::Segment { .. } => self.show_line_probes,
            ProbeTarget::Boundary(_) => self.show_boundary_probes,
            ProbeTarget::AreaDisk { .. } | ProbeTarget::AreaRegion { .. } => self.show_area_probes,
        }
    }
    fn hit_internal_handle(&self, p: Pos2, r: Rect) -> Option<(InternalBoundaryId, usize)> {
        if !self.handles {
            return None;
        }
        self.editor
            .document
            .draft
            .internal_boundaries
            .iter()
            .flat_map(|boundary| {
                boundary
                    .spline
                    .controls()
                    .iter()
                    .enumerate()
                    .map(move |(index, control)| {
                        (boundary.id, index, self.screen(*control, r).distance(p))
                    })
            })
            .filter(|candidate| candidate.2 <= 10.0)
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(id, index, _)| (id, index))
    }
    fn hit_curve(&self, p: Pos2, r: Rect) -> Option<(ObstacleId, f64)> {
        let world = self.world(p, r);
        self.draft_curves
            .iter()
            .filter_map(|c| {
                let distance = c
                    .samples
                    .windows(2)
                    .map(|s| point_segment_distance(world, s[0].point, s[1].point))
                    .fold(f64::INFINITY, f64::min);
                (distance * self.scale <= 8.0).then_some((c, distance))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(c, _)| {
                let spline = &self.editor.obstacle(c.id).unwrap().spline;
                let nearest_knot = spline
                    .knots()
                    .iter()
                    .copied()
                    .map(|t| (t, self.screen(spline.evaluate(t), r).distance(p)))
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                let t = match nearest_knot {
                    Some((t, distance)) if distance <= 4.0 => t,
                    _ => closest_parameter(spline, &c.samples, world),
                };
                (c.id, t)
            })
    }

    fn hit_internal_curve(&self, p: Pos2, r: Rect) -> Option<(InternalBoundaryId, f64)> {
        let world = self.world(p, r);
        self.draft_internal_curves
            .iter()
            .filter_map(|curve| {
                let distance = curve
                    .samples
                    .windows(2)
                    .map(|samples| {
                        point_segment_distance(world, samples[0].point, samples[1].point)
                    })
                    .fold(f64::INFINITY, f64::min);
                (distance * self.scale <= 8.0).then_some((curve, distance))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(curve, _)| {
                let spline = &self.editor.internal_boundary(curve.id).unwrap().spline;
                (
                    curve.id,
                    closest_open_parameter(spline, &curve.samples, world),
                )
            })
    }

    fn draw_curve(
        &self,
        painter: &egui::Painter,
        r: Rect,
        curve: &Curve,
        color: Color32,
        width: f32,
    ) {
        if curve.samples.len() > 1 {
            painter.add(egui::Shape::line(
                curve
                    .samples
                    .iter()
                    .map(|s| self.screen(s.point, r))
                    .collect(),
                Stroke::new(width, color),
            ));
        }
    }
    fn draw_internal_curve(
        &self,
        painter: &egui::Painter,
        r: Rect,
        curve: &InternalCurve,
        color: Color32,
        width: f32,
    ) {
        if curve.samples.len() > 1 {
            painter.add(egui::Shape::line(
                curve
                    .samples
                    .iter()
                    .map(|sample| self.screen(sample.point, r))
                    .collect(),
                Stroke::new(width, color),
            ));
        }
    }

    fn draw_curve_span(&self, painter: &egui::Painter, r: Rect, curve: &Curve, bounds: [f64; 2]) {
        self.draw_curve_span_colored(painter, r, curve, bounds, Color32::WHITE, 4.0);
    }

    fn draw_curve_span_colored(
        &self,
        painter: &egui::Painter,
        r: Rect,
        curve: &Curve,
        bounds: [f64; 2],
        color: Color32,
        width: f32,
    ) {
        for segment in curve.samples.windows(2) {
            let parameter = 0.5 * (segment[0].t + segment[1].t);
            if parameter >= bounds[0] && parameter <= bounds[1] {
                painter.line_segment(
                    [
                        self.screen(segment[0].point, r),
                        self.screen(segment[1].point, r),
                    ],
                    Stroke::new(width, color),
                );
            }
        }
    }

    fn draw_internal_span_condition(
        &self,
        painter: &egui::Painter,
        r: Rect,
        curve: &InternalCurve,
        bounds: [f64; 2],
        side: InternalBoundarySide,
        color: Color32,
    ) {
        for segment in curve.samples.windows(2) {
            let parameter = 0.5 * (segment[0].t + segment[1].t);
            if parameter < bounds[0] || parameter > bounds[1] {
                continue;
            }
            let points = [
                self.screen(segment[0].point, r),
                self.screen(segment[1].point, r),
            ];
            let tangent = points[1] - points[0];
            let length = tangent.length();
            if length <= f32::EPSILON {
                continue;
            }
            let mut normal = egui::vec2(tangent.y, -tangent.x) * (4.0 / length);
            if side == InternalBoundarySide::Right {
                normal = -normal;
            }
            painter.line_segment(
                [points[0] + normal, points[1] + normal],
                Stroke::new(2.5, color),
            );
        }
    }

    fn draw_internal_span_face(
        &self,
        painter: &egui::Painter,
        r: Rect,
        curve: &InternalCurve,
        bounds: [f64; 2],
        side: InternalBoundarySide,
    ) {
        let mut arrow = None;
        for segment in curve.samples.windows(2) {
            let parameter = 0.5 * (segment[0].t + segment[1].t);
            if parameter < bounds[0] || parameter > bounds[1] {
                continue;
            }
            let points = [
                self.screen(segment[0].point, r),
                self.screen(segment[1].point, r),
            ];
            let tangent = points[1] - points[0];
            let length = tangent.length();
            if length <= f32::EPSILON {
                continue;
            }
            let mut normal = egui::vec2(tangent.y, -tangent.x) * (4.0 / length);
            if side == InternalBoundarySide::Right {
                normal = -normal;
            }
            painter.line_segment(
                [points[0] + normal, points[1] + normal],
                Stroke::new(3.0, Color32::WHITE),
            );
            let score = (parameter - 0.5 * (bounds[0] + bounds[1])).abs();
            if arrow.is_none_or(|(best, _, _)| score < best) {
                arrow = Some((score, points[0], points[1]));
            }
        }
        if let Some((_, start, end)) = arrow {
            let tangent = end - start;
            let length = tangent.length();
            if length > f32::EPSILON {
                let direction = tangent / length;
                let normal = egui::vec2(-direction.y, direction.x);
                let center = start + 0.5 * tangent;
                let tip = center + direction * 7.0;
                let tail = center - direction * 7.0;
                painter.line_segment([tail, tip], Stroke::new(1.5, GOLD));
                painter.line_segment(
                    [tip, tip - direction * 5.0 + normal * 3.0],
                    Stroke::new(1.5, GOLD),
                );
                painter.line_segment(
                    [tip, tip - direction * 5.0 - normal * 3.0],
                    Stroke::new(1.5, GOLD),
                );
            }
        }
    }
}

fn outer_side_points(side: OuterSide) -> [Point2; 2] {
    match side {
        OuterSide::Bottom => [Point2::new(-1.0, -1.0), Point2::new(1.0, -1.0)],
        OuterSide::Right => [Point2::new(1.0, -1.0), Point2::new(1.0, 1.0)],
        OuterSide::Top => [Point2::new(1.0, 1.0), Point2::new(-1.0, 1.0)],
        OuterSide::Left => [Point2::new(-1.0, 1.0), Point2::new(-1.0, -1.0)],
    }
}

fn geometry_span_key(span: GeometrySpan) -> (u8, u64, usize) {
    match span {
        GeometrySpan::Outer(side) => (0, 0, side.index()),
        GeometrySpan::Loop(id, index) => (1, id.0, index),
        GeometrySpan::Baffle(id, index) => (2, id.0, index),
    }
}

fn segment_intersects_rect(a: Pos2, b: Pos2, rect: Rect) -> bool {
    if rect.contains(a) || rect.contains(b) {
        return true;
    }
    let delta = b - a;
    let mut minimum = 0.0_f32;
    let mut maximum = 1.0_f32;
    for (direction, distance) in [
        (-delta.x, a.x - rect.min.x),
        (delta.x, rect.max.x - a.x),
        (-delta.y, a.y - rect.min.y),
        (delta.y, rect.max.y - a.y),
    ] {
        if direction.abs() <= f32::EPSILON {
            if distance < 0.0 {
                return false;
            }
            continue;
        }
        let parameter = distance / direction;
        if direction < 0.0 {
            minimum = minimum.max(parameter);
        } else {
            maximum = maximum.min(parameter);
        }
        if minimum > maximum {
            return false;
        }
    }
    true
}

fn field_color(value: f32, gain: f32) -> Color32 {
    let value = if value.is_finite() {
        (value * gain).tanh()
    } else {
        0.0
    };
    let neutral = [16.0, 23.0, 31.0];
    let target = if value >= 0.0 {
        [244.0, 105.0, 122.0]
    } else {
        [63.0, 144.0, 239.0]
    };
    let amount = value.abs();
    Color32::from_rgba_unmultiplied(
        (neutral[0] + amount * (target[0] - neutral[0])) as u8,
        (neutral[1] + amount * (target[1] - neutral[1])) as u8,
        (neutral[2] + amount * (target[2] - neutral[2])) as u8,
        220,
    )
}

fn boundary_condition_color(condition: FaceBoundaryCondition) -> Color32 {
    match condition {
        FaceBoundaryCondition::Reflecting => Color32::from_rgb(142, 161, 175),
        FaceBoundaryCondition::Impedance { .. } => Color32::from_rgb(63, 144, 239),
        FaceBoundaryCondition::SecondOrderOutgoing => Color32::from_rgb(155, 126, 222),
        FaceBoundaryCondition::Neumann { .. } => GOLD,
        FaceBoundaryCondition::Dirichlet { .. } => RED,
    }
}

fn amr_target_color(fraction: f32) -> Color32 {
    let fine = [240.0, 82.0, 92.0];
    let coarse = [49.0, 154.0, 224.0];
    Color32::from_rgba_unmultiplied(
        egui::lerp(fine[0]..=coarse[0], fraction) as u8,
        egui::lerp(fine[1]..=coarse[1], fraction) as u8,
        egui::lerp(fine[2]..=coarse[2], fraction) as u8,
        105,
    )
}

fn outer_boundary_condition_color(condition: OuterBoundaryCondition) -> Color32 {
    match condition {
        OuterBoundaryCondition::Reflecting => Color32::from_rgb(142, 161, 175),
        OuterBoundaryCondition::FirstOrderOutgoing => Color32::from_rgb(63, 144, 239),
        OuterBoundaryCondition::SecondOrderOutgoing => Color32::from_rgb(155, 126, 222),
        OuterBoundaryCondition::Neumann { .. } => GOLD,
        OuterBoundaryCondition::Dirichlet { .. } => RED,
    }
}

fn mesh_region_at(mesh: &TriMesh, point: Point2) -> Option<RegionId> {
    mesh.triangles.iter().find_map(|triangle| {
        let [a, b, c] = triangle.vertices.map(|index| mesh.vertices[index].point);
        let orientation = (b - a).cross(c - a);
        let inside = orientation > 0.0
            && (b - a).cross(point - a) >= -1.0e-12
            && (c - b).cross(point - b) >= -1.0e-12
            && (a - c).cross(point - c) >= -1.0e-12;
        inside.then_some(triangle.region)
    })
}

fn highest_forcing_frequency(scene: &Scene, source: SourceSettings) -> f64 {
    let mut frequency = if source.enabled && source.amplitude != 0.0 {
        source.frequency_hz as f64
    } else {
        0.0
    };
    for condition in scene.outer_boundaries.sides {
        if let Some(signal) = condition.signal()
            && signal.amplitude != 0.0
        {
            frequency = frequency.max(signal.frequency_hz);
        }
    }
    let mut include = |condition: FaceBoundaryCondition| {
        if let Some(signal) = condition.signal()
            && signal.amplitude != 0.0
        {
            frequency = frequency.max(signal.frequency_hz);
        }
    };
    for obstacle in &scene.obstacles {
        for condition in &obstacle.span_conditions {
            include(*condition);
        }
    }
    for boundary in &scene.internal_boundaries {
        for law in &boundary.span_laws {
            include(law.left);
            include(law.right);
        }
    }
    frequency
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AmrTransactionDecision {
    refine: bool,
    coarsen: bool,
}

fn amr_transaction_decision(
    mesh: &TriMesh,
    result: &SolutionIndicatorResult,
) -> AmrTransactionDecision {
    let candidate_threshold = (mesh.triangles.len() / 1_000).max(4);
    let mut refinement_severity = 1.0_f64;
    let mut coarsening_severity = 1.0_f64;
    for (triangle, target) in mesh.triangles.iter().zip(&result.element_targets) {
        let points = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
        let lengths = [
            (points[1] - points[0]).norm(),
            (points[2] - points[1]).norm(),
            (points[0] - points[2]).norm(),
        ];
        let maximum_edge = lengths.iter().copied().fold(0.0_f64, f64::max);
        let minimum_edge = lengths.iter().copied().fold(f64::INFINITY, f64::min);
        refinement_severity = refinement_severity.max(maximum_edge / target);
        coarsening_severity = coarsening_severity.max(target / minimum_edge.max(f64::MIN_POSITIVE));
    }
    AmrTransactionDecision {
        refine: result.report.refine_candidates >= candidate_threshold
            || refinement_severity >= 1.5,
        coarsen: result.report.coarsen_candidates >= candidate_threshold
            || coarsening_severity >= 1.8,
    }
}

fn update_coarsening_confirmation(streak: &mut u8, requested: bool) -> bool {
    if requested {
        *streak = streak.saturating_add(1);
    } else {
        *streak = 0;
    }
    *streak >= 2
}

fn automatic_adaptation_work_limit(mesh: &TriMesh, topology_budget: usize) -> usize {
    // A topology change restarts a global candidate scan. Paired traces can add
    // two vertices, four triangles, and two constrained edges in one change.
    // Budget for those growing scans rather than applying the small fixed limit
    // used by explicit adaptation transactions.
    let scan_units = mesh
        .vertices
        .len()
        .saturating_add(mesh.triangles.len())
        .saturating_add(mesh.boundary_edges.len())
        .saturating_add(topology_budget.saturating_mul(8));
    let scan_passes = topology_budget.saturating_mul(2).saturating_add(32);
    scan_units
        .saturating_mul(scan_passes)
        .max(MeshAdaptationOptions::default().max_work_units)
}

#[allow(clippy::too_many_arguments)]
pub fn frame(
    mut contexts: EguiContexts,
    mut state: ResMut<Playground>,
    time: Res<Time>,
    mut request: ResMut<WaveGpuRequest>,
    display: Res<WaveDisplay>,
    probe_display: Res<ProbeDisplay>,
    curve_probe_display: Res<CurveProbeDisplay>,
    area_probe_display: Res<AreaProbeDisplay>,
    far_field_display: Res<FarFieldDisplay>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut commands: Commands,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    if !state.ready {
        ctx.set_visuals(egui::Visuals::dark());
        ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.button_padding = egui::vec2(8.0, 4.0);
            style.spacing.interact_size.y = 24.0;
            style.visuals.selection.bg_fill = Color32::from_rgb(38, 94, 135);
            style.visuals.selection.stroke = Stroke::new(1.0, SELECT);
        });
        ctx.options_mut(|o| o.max_passes = 1.try_into().unwrap());
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
    state.ingest_probe_samples(&probe_display);
    state.ingest_curve_probe_samples(&curve_probe_display);
    state.ingest_area_probe_samples(&area_probe_display);
    state.ingest_far_field_samples(&far_field_display);
    let mut root = egui::Ui::new(
        ctx.clone(),
        "root".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.viewport_rect()),
    );
    state.show(&mut root, Some(&display));
    state.editor.validate_frame(12_000);
    state.refresh_mesh();
    state.refresh_mesh_adaptation();
    state.refresh_wave(
        &mut request,
        &display,
        &mut assets,
        &mut commands,
        time.delta_secs_f64(),
    );
    state.refresh_probe_gpu(&mut request, &mut assets, &mut commands);
    state.refresh_solution_amr(&request, &display);
    Ok(())
}

/// Opt-in scripted edits in the real native renderer, including validation,
/// scheduling, mesh publication and the visible production-mesh overlay.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource, Default)]
pub struct MeshBenchmark {
    edits: usize,
    start: Option<Instant>,
    target: Option<Scene>,
}

#[cfg(not(target_arch = "wasm32"))]
pub fn mesh_benchmark_scene() -> Playground {
    let mut state = Playground {
        automated_benchmark: true,
        mesh_max_edge: 0.08,
        show_mesh: true,
        ..Default::default()
    };
    let scene = Scene {
        obstacles: (0..8)
            .map(|i| {
                Obstacle::hole(
                    ObstacleId(i + 1),
                    PeriodicCubicSpline::rounded(
                        Point2::new(-0.66 + (i % 4) as f64 * 0.44, -0.4 + (i / 4) as f64 * 0.8),
                        0.12,
                    ),
                )
            })
            .collect(),
        ..Scene::default()
    };
    state
        .editor
        .replace_validated(funfern_app::editor::Document {
            draft: scene.clone(),
            accepted: scene,
            probes: vec![],
            source: SourceSettings::default(),
            far_field: Default::default(),
        });
    state
}

#[cfg(not(target_arch = "wasm32"))]
pub fn wave_gpu_check_scene() -> Playground {
    let mut state = Playground {
        automated_benchmark: true,
        wave_running: false,
        ..Default::default()
    };
    let scene = Scene {
        obstacles: vec![Obstacle::with_role(
            ObstacleId(1),
            PeriodicCubicSpline::rounded(Point2::default(), 0.30),
            LoopRole::Wall {
                exterior: BACKGROUND_REGION,
                interior: RegionId(2),
            },
        )],
        internal_boundaries: vec![InternalBoundary {
            id: InternalBoundaryId(1),
            spline: OpenCubicSpline::uniform(vec![
                Point2::new(-0.841_274_799_6, -0.460_310_562_0),
                Point2::new(-0.702_926_820_9, -0.393_873_880_9),
                Point2::new(-0.564_578_842_4, -0.327_437_199_8),
                Point2::new(-0.426_230_863_7, -0.261_000_518_7),
            ])
            .unwrap(),
            region: BACKGROUND_REGION,
            span_laws: vec![InternalBoundaryLaw {
                left: FaceBoundaryCondition::Dirichlet {
                    signal: BoundarySignal {
                        offset: 0.01,
                        amplitude: 0.03,
                        frequency_hz: 1.25,
                        phase_radians: 0.2,
                    },
                },
                right: FaceBoundaryCondition::Neumann {
                    signal: BoundarySignal {
                        amplitude: 0.15,
                        frequency_hz: 0.75,
                        ..BoundarySignal::ZERO
                    },
                },
                coupling: InternalBoundaryCoupling::Independent,
            }],
        }],
        materials: vec![
            Material::default_medium(),
            Material {
                id: MaterialId(2),
                name: "Isolated interior".into(),
                mass_density: 1.0,
                stiffness: 1.0,
                damping: 0.0,
                color: [77, 121, 164],
            },
        ],
        regions: vec![
            Region {
                id: BACKGROUND_REGION,
                material: DEFAULT_MATERIAL,
            },
            Region {
                id: RegionId(2),
                material: MaterialId(2),
            },
        ],
        outer_boundaries: OuterBoundaryConditions::default(),
    };
    state
        .editor
        .replace_validated(funfern_app::editor::Document {
            draft: scene.clone(),
            accepted: scene,
            probes: vec![
                ProbeDefinition {
                    id: ProbeId(1),
                    name: "GPU check".into(),
                    color: [63, 144, 239],
                    enabled: true,
                    target: ProbeTarget::Point(Point2::new(-0.2, 0.42)),
                },
                ProbeDefinition {
                    id: ProbeId(2),
                    name: "GPU line check".into(),
                    color: [78, 201, 176],
                    enabled: true,
                    target: ProbeTarget::Segment {
                        start: Point2::new(-0.7, 0.42),
                        end: Point2::new(0.7, 0.42),
                        preset: ProbeSamplingPreset::Medium,
                    },
                },
                ProbeDefinition {
                    id: ProbeId(3),
                    name: "GPU partial line check".into(),
                    color: [244, 105, 122],
                    enabled: true,
                    target: ProbeTarget::Segment {
                        start: Point2::new(-1.2, 0.7),
                        end: Point2::new(1.2, 0.7),
                        preset: ProbeSamplingPreset::Medium,
                    },
                },
                ProbeDefinition {
                    id: ProbeId(4),
                    name: "GPU area check".into(),
                    color: [164, 126, 232],
                    enabled: true,
                    target: ProbeTarget::AreaDisk {
                        center: Point2::new(-0.42, 0.11),
                        radius: 0.18,
                    },
                },
            ],
            source: SourceSettings::default(),
            far_field: FarFieldSettings {
                enabled: true,
                ..Default::default()
            },
        });
    state
}

#[cfg(not(target_arch = "wasm32"))]
pub fn amr_check_scene() -> Playground {
    let mut state = Playground {
        automated_benchmark: true,
        mesh_max_edge: 0.16,
        wave_running: false,
        ..Default::default()
    };
    state
        .editor
        .create_point_probe(Point2::new(-0.25, 0.25))
        .unwrap();
    state
}

#[cfg(not(target_arch = "wasm32"))]
pub fn wave_transfer_check_scene() -> Playground {
    let mut state = Playground {
        automated_benchmark: true,
        wave_running: false,
        ..Default::default()
    };
    state
        .editor
        .create_point_probe(Point2::new(-0.55, 0.35))
        .unwrap();
    state
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource)]
pub struct AmrBenchmark {
    started: Instant,
    phase: u8,
    source_mesh_revision: u64,
    first_mesh_revision: u64,
    first_insertions: usize,
    first_triangles: usize,
    handoff_exposed_nodes: Option<usize>,
    simulation_time: f64,
    probe_time_before: Option<f64>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for AmrBenchmark {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            phase: 0,
            source_mesh_revision: 0,
            first_mesh_revision: 0,
            first_insertions: 0,
            first_triangles: 0,
            handoff_exposed_nodes: None,
            simulation_time: 0.0,
            probe_time_before: None,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn amr_options() -> MeshAdaptationOptions {
    MeshAdaptationOptions {
        meshing: MeshingOptions {
            curve_tolerance: 1.5e-3,
            target_edge_length: 0.16 / 1.05,
            minimum_angle_degrees: 12.0,
            max_vertices: 50_000,
            max_triangles: 100_000,
            max_refinement_steps: 50_000,
        },
        minimum_target_edge_length: 0.05,
        maximum_target_edge_length: 0.16,
        max_topology_changes: 700,
        ..Default::default()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn amr_radial_field(center: Point2) -> Arc<dyn MeshSizeField> {
    Arc::new(move |point: Point2, _region: RegionId| {
        let distance = (point - center).norm();
        if distance <= 0.28 {
            0.05
        } else if distance >= 0.48 {
            0.16
        } else {
            let x = (distance - 0.28) / 0.20;
            let smooth = x * x * (3.0 - 2.0 * x);
            0.05 + 0.11 * smooth
        }
    })
}

/// Opt-in end-to-end AMR transaction check. It first requires an automatic,
/// solution-driven estimate from the ordinary frame controller, then exercises
/// two deterministic moving-target handoffs for refinement/coarsening invariants.
#[cfg(not(target_arch = "wasm32"))]
pub fn amr_benchmark(
    mut benchmark: ResMut<AmrBenchmark>,
    mut state: ResMut<Playground>,
    mut request: ResMut<WaveGpuRequest>,
    display: Res<WaveDisplay>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut exit: MessageWriter<bevy::app::AppExit>,
) {
    if benchmark.started.elapsed().as_secs_f64() > 90.0 {
        error!(phase = benchmark.phase, "AMR check timed out");
        exit.write(bevy::app::AppExit::error());
        return;
    }
    if let Some(candidate) = &state.simulation_candidate {
        benchmark.handoff_exposed_nodes = Some(candidate.exposed_nodes);
    }
    match benchmark.phase {
        0 => {
            let (Some(mesh), Some(operator)) = (
                state.wave_mesh.as_ref().cloned(),
                state.wave_operator.as_ref().cloned(),
            ) else {
                return;
            };
            if !request.ready()
                || state.simulation_candidate.is_some()
                || state.mesh_adaptation_job.is_some()
                || state.solution_indicator_report.is_none()
                || state
                    .probe_compiled
                    .as_ref()
                    .is_none_or(|compiled| compiled.generation != request.generation())
            {
                return;
            }
            state.wave_running = false;
            if let Err(error) = request.inject_pulse(
                &mut assets,
                &mesh,
                &operator,
                PulseSettings {
                    position: Point2::new(-0.72, 0.38),
                    amplitude: 0.65,
                    width: 0.07,
                    region: BACKGROUND_REGION,
                },
            ) {
                error!(%error, "AMR pulse setup failed");
                exit.write(bevy::app::AppExit::error());
                return;
            }
            request.request_steps(24);
            benchmark.phase = 1;
        }
        1 => {
            if state.mesh_error.is_some() || state.amr_error.is_some() {
                error!(mesh_error = ?state.mesh_error, amr_error = ?state.amr_error, "Automatic solution AMR failed");
                exit.write(bevy::app::AppExit::error());
                return;
            }
            let energy_estimated = state
                .solution_indicator_report
                .as_ref()
                .is_some_and(|report| {
                    report.maximum_indicator > 1.0e-8
                        && report.boundary_edges_evaluated > 0
                        && [
                            report.recovery_contribution,
                            report.cell_residual_contribution,
                            report.interior_jump_contribution,
                            report.boundary_residual_contribution,
                            report.maximum_dirichlet_mismatch,
                        ]
                        .into_iter()
                        .all(f64::is_finite)
                });
            if !energy_estimated {
                return;
            }
            let Some(probe_time) = state
                .probe_traces
                .values()
                .next()
                .and_then(|trace| trace.samples.back())
                .map(|sample| sample.time)
            else {
                return;
            };
            benchmark.probe_time_before = Some(probe_time);
            // A nonzero solution estimate has reached the automatic controller.
            // Let any transaction it opened finish, then use deterministic target
            // fields for the remaining topology invariants.
            state.amr_enabled = false;
            state.solution_indicator_job = None;
            state.solution_indicator_source = None;
            if state.mesh_adaptation_job.is_some() || state.simulation_candidate.is_some() {
                return;
            }
            benchmark.simulation_time = state.wave_time_offset
                + request.stats().completed_steps() as f64 * state.wave_time_step;
            benchmark.source_mesh_revision = state.wave_mesh.as_ref().unwrap().mesh_revision;
            benchmark.handoff_exposed_nodes = None;
            state.wave_running = true;
            if let Err(error) = state
                .start_mesh_adaptation(amr_radial_field(Point2::new(-0.55, 0.45)), amr_options())
            {
                error!(%error, "Could not start first AMR transaction");
                exit.write(bevy::app::AppExit::error());
                return;
            }
            benchmark.phase = 2;
        }
        2 => {
            if state.mesh_adaptation_job.is_some() || state.simulation_candidate.is_some() {
                return;
            }
            let (Some(mesh), Some(report)) = (&state.wave_mesh, &state.mesh_adaptation_report)
            else {
                return;
            };
            if mesh.mesh_revision == benchmark.source_mesh_revision {
                return;
            }
            if report.inserted_vertices == 0
                || benchmark.handoff_exposed_nodes != Some(0)
                || state.wave_error.is_some()
                || state.mesh_error.is_some()
            {
                error!(?report, wave_error = ?state.wave_error, mesh_error = ?state.mesh_error, "First AMR transaction failed checks");
                exit.write(bevy::app::AppExit::error());
                return;
            }
            benchmark.first_mesh_revision = mesh.mesh_revision;
            benchmark.first_insertions = report.inserted_vertices;
            benchmark.first_triangles = mesh.triangles.len();
            benchmark.handoff_exposed_nodes = None;
            if let Err(error) = state
                .start_mesh_adaptation(amr_radial_field(Point2::new(0.55, -0.45)), amr_options())
            {
                error!(%error, "Could not start second AMR transaction");
                exit.write(bevy::app::AppExit::error());
                return;
            }
            benchmark.phase = 3;
        }
        3 => {
            if state.mesh_adaptation_job.is_some() || state.simulation_candidate.is_some() {
                return;
            }
            let (Some(mesh), Some(operator), Some(report), Some(adaptation_state)) = (
                &state.wave_mesh,
                &state.wave_operator,
                &state.mesh_adaptation_report,
                &state.mesh_adaptation_state,
            ) else {
                return;
            };
            let simulation_time = state.wave_time_offset
                + request.stats().completed_steps() as f64 * state.wave_time_step;
            let probe_times = state
                .probe_traces
                .values()
                .next()
                .map(|trace| {
                    trace
                        .samples
                        .iter()
                        .map(|sample| sample.time)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let largest_probe_gap = probe_times
                .windows(2)
                .map(|pair| pair[1] - pair[0])
                .fold(0.0_f64, f64::max);
            // Rebinding can miss several cadence slots while the new render-world
            // bind group becomes visible. The retired double-offset path jumped
            // by the complete elapsed simulation time instead.
            let maximum_expected_probe_gap =
                (8.0 / state.probe_sample_rate).max(4.0 * state.wave_time_step);
            let probe_ready = state
                .probe_compiled
                .as_ref()
                .is_some_and(|compiled| compiled.generation == request.generation())
                && benchmark
                    .probe_time_before
                    .is_some_and(|before| probe_times.last().is_some_and(|time| *time > before));
            if !probe_ready {
                return;
            }
            let probe_continuous = benchmark.probe_time_before.is_some_and(|before| {
                probe_times.contains(&before)
                    && probe_times.last().is_some_and(|time| *time > before)
                    && largest_probe_gap <= maximum_expected_probe_gap
            });
            let valid = mesh.mesh_revision != benchmark.first_mesh_revision
                && mesh.mesh_revision == operator.mesh_revision()
                && mesh.mesh_revision == adaptation_state.mesh_revision
                && mesh.geometry_revision == operator.geometry_revision()
                && report.inserted_vertices > 0
                && report.collapsed_vertices > 0
                && benchmark.handoff_exposed_nodes == Some(0)
                && display.current.len() == operator.degrees_of_freedom()
                && display.previous.len() == operator.degrees_of_freedom()
                && display.auxiliary.len() == operator.degrees_of_freedom()
                && display.indicator_displacement.len() == operator.degrees_of_freedom()
                && display.indicator_velocity.len() == operator.degrees_of_freedom()
                && display.indicator_acceleration.len() == operator.degrees_of_freedom()
                && display.current.iter().any(|value| value.abs() > 1.0e-7)
                && simulation_time >= benchmark.simulation_time
                && probe_continuous
                && state.wave_running
                && state.wave_error.is_none()
                && state.mesh_error.is_none();
            info!(
                first_insertions = benchmark.first_insertions,
                first_triangles = benchmark.first_triangles,
                second_insertions = report.inserted_vertices,
                second_collapses = report.collapsed_vertices,
                second_triangles = mesh.triangles.len(),
                topology_changes = report.topology_changes,
                work_units = report.work_units,
                converged = report.converged,
                limit = ?report.limit,
                preserved_triangles = report.preserved_triangles,
                minimum_target = report.minimum_target,
                maximum_target = report.maximum_target,
                indicator_work_ms = state.amr_work_ms,
                indicator_max_slice_ms = state.amr_max_slice_ms,
                indicator_boundary_edges = state
                    .solution_indicator_report
                    .as_ref()
                    .map_or(0, |report| report.boundary_edges_evaluated),
                indicator_boundary_contribution = state
                    .solution_indicator_report
                    .as_ref()
                    .map_or(0.0, |report| report.boundary_residual_contribution),
                indicator_dirichlet_mismatch = state
                    .solution_indicator_report
                    .as_ref()
                    .map_or(0.0, |report| report.maximum_dirichlet_mismatch),
                dofs = operator.degrees_of_freedom(),
                solver_dt = state.wave_time_step,
                probe_samples = probe_times.len(),
                largest_probe_gap,
                maximum_expected_probe_gap,
                probe_continuous,
                elapsed_ms = benchmark.started.elapsed().as_secs_f64() * 1000.0,
                "AMR check complete"
            );
            if valid {
                exit.write(bevy::app::AppExit::Success);
            } else {
                error!(?report, wave_error = ?state.wave_error, mesh_error = ?state.mesh_error, "AMR result failed invariants");
                exit.write(bevy::app::AppExit::error());
            }
            benchmark.phase = 4;
        }
        _ => {}
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource)]
pub struct WaveGpuBenchmark {
    started: Instant,
    solve_started: Option<Instant>,
    generation: u64,
    expected_current: Vec<f64>,
    expected_previous: Vec<f64>,
    expected_auxiliary: Vec<f64>,
    exterior_nodes: Vec<bool>,
    prepared: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for WaveGpuBenchmark {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            solve_started: None,
            generation: 0,
            expected_current: Vec::new(),
            expected_previous: Vec::new(),
            expected_auxiliary: Vec::new(),
            exterior_nodes: Vec::new(),
            prepared: false,
        }
    }
}

/// Native opt-in validation of the actual WGSL gather kernel against the f64
/// reference on the same assembled mesh and conservative timestep.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
pub fn wave_gpu_benchmark(
    mut benchmark: ResMut<WaveGpuBenchmark>,
    mut state: ResMut<Playground>,
    mut request: ResMut<WaveGpuRequest>,
    display: Res<WaveDisplay>,
    probe_display: Res<ProbeDisplay>,
    curve_probe_display: Res<CurveProbeDisplay>,
    area_probe_display: Res<AreaProbeDisplay>,
    far_field_display: Res<FarFieldDisplay>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut exit: MessageWriter<bevy::app::AppExit>,
) {
    if benchmark.started.elapsed().as_secs_f64() > 60.0 {
        error!("Wave GPU check timed out");
        exit.write(bevy::app::AppExit::error());
        return;
    }
    if !benchmark.prepared {
        let mut target = OuterBoundaryConditions::default();
        target.sides[OuterSide::Bottom.index()] = OuterBoundaryCondition::Dirichlet {
            signal: BoundarySignal {
                offset: 0.02,
                amplitude: 0.04,
                frequency_hz: 2.0,
                phase_radians: 0.1,
            },
        };
        target.sides[OuterSide::Top.index()] = OuterBoundaryCondition::Neumann {
            signal: BoundarySignal {
                amplitude: 0.3,
                frequency_hz: 1.5,
                ..BoundarySignal::ZERO
            },
        };
        target.sides[OuterSide::Left.index()] = OuterBoundaryCondition::SecondOrderOutgoing;
        target.sides[OuterSide::Right.index()] = OuterBoundaryCondition::FirstOrderOutgoing;
        if state.editor.document.accepted.outer_boundaries != target {
            state.editor.document.draft.outer_boundaries = target;
            state.editor.document.accepted.outer_boundaries = target;
            return;
        }
        if state.wave_boundary_committed != target || state.simulation_candidate.is_some() {
            return;
        }
        state.wave_running = false;
        state.wave_accumulator = 0.0;
        let Some(operator) = &state.wave_operator else {
            return;
        };
        let Some(mesh) = &state.wave_mesh else {
            return;
        };
        let position = Point2::new(-0.42, 0.11);
        let amplitude = 0.65_f32;
        let width = 0.06_f32;
        if let Err(error) = request.inject_pulse(
            &mut assets,
            mesh,
            operator,
            PulseSettings {
                position,
                amplitude,
                width,
                region: BACKGROUND_REGION,
            },
        ) {
            error!("Wave GPU pulse setup failed: {error}");
            exit.write(bevy::app::AppExit::error());
            return;
        }
        let mut cpu = QuadraticWaveState::zero(operator, state.wave_time_step).unwrap();
        benchmark.exterior_nodes = vec![false; operator.degrees_of_freedom()];
        for (triangle, nodes) in mesh.triangles.iter().zip(operator.element_nodes()) {
            if triangle.region == BACKGROUND_REGION {
                for node in nodes {
                    benchmark.exterior_nodes[*node as usize] = true;
                }
            }
        }
        let pulse: Vec<_> = forcing_weights(mesh, operator, position, width, BACKGROUND_REGION)
            .unwrap()
            .into_iter()
            .map(|weight| (amplitude * weight) as f64)
            .collect();
        cpu.add_displacement(&pulse).unwrap();
        for _ in 0..1024 {
            cpu.step(operator, &[]).unwrap();
        }
        benchmark.expected_current = cpu.current().to_vec();
        benchmark.expected_previous = cpu.previous().to_vec();
        benchmark.expected_auxiliary = cpu.auxiliary().to_vec();
        benchmark.generation = request.generation();
        benchmark.prepared = true;
        benchmark.solve_started = Some(Instant::now());
        request.request_steps(1024);
        let minimum_edge = mesh
            .triangles
            .iter()
            .flat_map(|triangle| {
                let points = triangle.vertices.map(|index| mesh.vertices[index].point);
                [
                    (points[1] - points[0]).norm(),
                    (points[2] - points[1]).norm(),
                    (points[0] - points[2]).norm(),
                ]
            })
            .fold(f64::INFINITY, f64::min);
        info!(
            dofs = operator.degrees_of_freedom(),
            dt = state.wave_time_step,
            min_edge = minimum_edge,
            min_angle = mesh.quality.minimum_angle_degrees,
            "Wave GPU check started"
        );
        return;
    }
    if request.stats().completed_steps() < 1024
        || display.completed_steps < 1024
        || display.generation != benchmark.generation
        || display.current.len() != benchmark.expected_current.len()
        || display.auxiliary.len() != benchmark.expected_auxiliary.len()
        || probe_display.generation != benchmark.generation
        || probe_display.records.is_empty()
        || curve_probe_display.generation != benchmark.generation
        || curve_probe_display.records.is_empty()
        || area_probe_display.generation != benchmark.generation
        || area_probe_display.records.is_empty()
        || far_field_display.generation != benchmark.generation
        || far_field_display.records.is_empty()
    {
        return;
    }
    let Some(operator) = &state.wave_operator else {
        return;
    };
    let error_norm = |actual: &[f32], expected: &[f64]| {
        let numerator = actual
            .iter()
            .zip(expected)
            .zip(operator.lumped_mass())
            .map(|((actual, expected), mass)| mass * (*actual as f64 - expected).powi(2))
            .sum::<f64>();
        let denominator = expected
            .iter()
            .zip(operator.lumped_mass())
            .map(|(expected, mass)| mass * expected * expected)
            .sum::<f64>();
        (numerator / denominator.max(f64::MIN_POSITIVE)).sqrt()
    };
    let current_error = error_norm(&display.current, &benchmark.expected_current);
    let previous_error = error_norm(&display.previous, &benchmark.expected_previous);
    let auxiliary_error = error_norm(&display.auxiliary, &benchmark.expected_auxiliary);
    let max_difference = display
        .current
        .iter()
        .zip(&benchmark.expected_current)
        .enumerate()
        .max_by(|(_, (actual_a, expected_a)), (_, (actual_b, expected_b))| {
            (**actual_a as f64 - **expected_a)
                .abs()
                .total_cmp(&(**actual_b as f64 - **expected_b).abs())
        })
        .map(|(node, (actual, expected))| {
            (
                node,
                *actual,
                *expected,
                operator.node_points()[node],
                operator.dirichlet_sides()[node],
                operator.normalized_neumann_weights()[node],
            )
        });
    let actual_peak = display
        .current
        .iter()
        .map(|value| value.abs())
        .fold(0.0_f32, f32::max);
    let expected_peak = benchmark
        .expected_current
        .iter()
        .map(|value| value.abs())
        .fold(0.0_f64, f64::max);
    let isolated_peak = display
        .current
        .iter()
        .zip(&benchmark.exterior_nodes)
        .filter(|(_, exterior)| !**exterior)
        .map(|(value, _)| value.abs())
        .fold(0.0_f32, f32::max);
    let solve_seconds = benchmark
        .solve_started
        .map_or(0.0, |start| start.elapsed().as_secs_f64());
    info!(
        current_relative_l2 = current_error,
        previous_relative_l2 = previous_error,
        auxiliary_relative_l2 = auxiliary_error,
        actual_peak,
        expected_peak,
        isolated_peak,
        elapsed_ms = benchmark.started.elapsed().as_secs_f64() * 1000.0,
        solve_readback_ms = solve_seconds * 1000.0,
        simulated_seconds_per_wall_second = 1024.0 * state.wave_time_step / solve_seconds,
        "Wave GPU check complete"
    );
    let probe_finite = probe_display.records.iter().all(|record| {
        record.time.is_finite()
            && record.displacement.is_finite()
            && record.velocity.is_finite()
            && record.energy_density.is_finite()
            && record.energy_density >= 0.0
    });
    let curve_probe_shape = curve_probe_display.records.iter().all(|record| {
        record.time.is_finite()
            && record.displacement.len() == ProbeSamplingPreset::Medium.spatial_points()
            && record.energy_density.len() == record.displacement.len()
            && record.normal_flux.len() == record.displacement.len()
            && record
                .displacement
                .iter()
                .zip(&record.energy_density)
                .zip(&record.normal_flux)
                .all(|((field, energy), flux)| {
                    field.is_finite() && energy.is_finite() && *energy >= 0.0 && flux.is_finite()
                        || field.is_nan() && energy.is_nan() && flux.is_nan()
                })
    });
    let complete_line_records = curve_probe_display
        .records
        .iter()
        .filter(|record| record.probe_id == 2)
        .collect::<Vec<_>>();
    let complete_line = !complete_line_records.is_empty()
        && complete_line_records
            .iter()
            .all(|record| record.displacement.iter().all(|value| value.is_finite()));
    let partial_line_records = curve_probe_display
        .records
        .iter()
        .filter(|record| record.probe_id == 3)
        .collect::<Vec<_>>();
    let partial_line = !partial_line_records.is_empty()
        && partial_line_records.iter().all(|record| {
            record.displacement.iter().any(|value| value.is_finite())
                && record.displacement.iter().any(|value| value.is_nan())
        });
    let area_probe_valid = area_probe_display.records.iter().all(|record| {
        record.time.is_finite()
            && record.mean_displacement.is_finite()
            && record.rms_displacement.is_finite()
            && record.rms_displacement >= 0.0
            && record.mean_energy_density.is_finite()
            && record.mean_energy_density >= 0.0
            && record.total_energy.is_finite()
            && record.total_energy >= 0.0
            && record.covered_area.is_finite()
            && record.covered_area > 0.0
            && record.coverage.is_finite()
            && record.coverage > 0.0
            && record.coverage <= 1.0
    });
    let far_field_valid = far_field_display.records.iter().all(|record| {
        record.time.is_finite()
            && record.amplitude.len() == FAR_FIELD_DIRECTIONS
            && record.intensity.len() == FAR_FIELD_DIRECTIONS
            && record
                .amplitude
                .iter()
                .zip(&record.intensity)
                .all(|(amplitude, intensity)| {
                    amplitude.is_finite() && intensity.is_finite() && *intensity >= 0.0
                })
    });
    if let Some((node, actual, expected, point, dirichlet, neumann)) = max_difference {
        info!(
            node,
            actual,
            expected,
            x = point.x,
            y = point.y,
            ?dirichlet,
            ?neumann,
            "Wave GPU maximum pointwise difference"
        );
    }
    if current_error <= 2.0e-4
        && previous_error <= 2.0e-4
        && auxiliary_error <= 2.0e-4
        && isolated_peak <= 1.0e-7
        && probe_finite
        && curve_probe_shape
        && complete_line
        && partial_line
        && area_probe_valid
        && far_field_valid
    {
        exit.write(bevy::app::AppExit::Success);
    } else {
        error!("Wave GPU result differs from the CPU reference");
        exit.write(bevy::app::AppExit::error());
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource)]
pub struct WaveTransferBenchmark {
    started: Instant,
    phase: u8,
    generation: u64,
    source_current: Vec<f64>,
    source_velocity: Vec<f64>,
    source_auxiliary: Vec<f64>,
    expected_current: Vec<f64>,
    expected_previous: Vec<f64>,
    expected_auxiliary: Vec<f64>,
    transfer_started: Option<Instant>,
    boundary_started: Option<Instant>,
    material_started: Option<Instant>,
    probe_time_before: f64,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for WaveTransferBenchmark {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            phase: 0,
            generation: 0,
            source_current: vec![],
            source_velocity: vec![],
            source_auxiliary: vec![],
            expected_current: vec![],
            expected_previous: vec![],
            expected_auxiliary: vec![],
            transfer_started: None,
            boundary_started: None,
            material_started: None,
            probe_time_before: 0.0,
        }
    }
}

/// Exercises a real edit transaction and compares both transferred GPU levels
/// with the same enriched-quadratic and centered-level formulas in f64.
#[cfg(not(target_arch = "wasm32"))]
pub fn wave_transfer_benchmark(
    mut benchmark: ResMut<WaveTransferBenchmark>,
    mut state: ResMut<Playground>,
    mut request: ResMut<WaveGpuRequest>,
    display: Res<WaveDisplay>,
    probe_display: Res<ProbeDisplay>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut exit: MessageWriter<bevy::app::AppExit>,
) {
    if benchmark.started.elapsed().as_secs_f64() > 60.0 {
        error!("Wave transfer check timed out at phase {}", benchmark.phase);
        exit.write(bevy::app::AppExit::error());
        return;
    }
    match benchmark.phase {
        0 => {
            let target =
                OuterBoundaryConditions::uniform(OuterBoundaryCondition::SecondOrderOutgoing);
            let hole_dirichlet = FaceBoundaryCondition::Dirichlet {
                signal: BoundarySignal {
                    offset: 0.015,
                    amplitude: 0.025,
                    frequency_hz: 1.1,
                    phase_radians: 0.3,
                },
            };
            let hole_neumann = FaceBoundaryCondition::Neumann {
                signal: BoundarySignal {
                    amplitude: 0.12,
                    frequency_hz: 0.8,
                    ..BoundarySignal::ZERO
                },
            };
            if state.editor.document.accepted.obstacles[0].span_conditions[0] != hole_dirichlet
                || state.editor.document.accepted.obstacles[0].span_conditions[2] != hole_neumann
            {
                state.editor.document.draft.obstacles[0].span_conditions[0] = hole_dirichlet;
                state.editor.document.draft.obstacles[0].span_conditions[2] = hole_neumann;
                state.editor.document.accepted.obstacles[0].span_conditions[0] = hole_dirichlet;
                state.editor.document.accepted.obstacles[0].span_conditions[2] = hole_neumann;
                return;
            }
            if state.editor.document.accepted.outer_boundaries != target {
                state.editor.document.draft.outer_boundaries = target;
                state.editor.document.accepted.outer_boundaries = target;
                return;
            }
            if state.wave_boundary_committed != target || state.simulation_candidate.is_some() {
                return;
            }
            if !request.ready()
                || state.wave_operator.is_none()
                || state
                    .probe_compiled
                    .as_ref()
                    .is_none_or(|compiled| compiled.generation != request.generation())
            {
                return;
            }
            state.wave_source.enabled = false;
            if let Err(error) = request.inject_pulse(
                &mut assets,
                state.wave_mesh.as_deref().unwrap(),
                state.wave_operator.as_deref().unwrap(),
                PulseSettings {
                    position: Point2::new(-0.82, 0.11),
                    amplitude: 0.65,
                    width: 0.06,
                    region: BACKGROUND_REGION,
                },
            ) {
                error!("Wave transfer pulse setup failed: {error}");
                exit.write(bevy::app::AppExit::error());
                return;
            }
            benchmark.generation = request.generation();
            request.request_steps(32);
            benchmark.phase = 1;
        }
        1 => {
            let Some(operator) = &state.wave_operator else {
                return;
            };
            if request.stats().completed_steps() < 32
                || display.generation != benchmark.generation
                || display.completed_steps < 32
                || display.current.len() != operator.degrees_of_freedom()
            {
                return;
            }
            let Some(probe_time) = state
                .probe_traces
                .values()
                .next()
                .and_then(|trace| trace.samples.back())
                .map(|sample| sample.time)
            else {
                return;
            };
            benchmark.probe_time_before = probe_time;
            benchmark.source_current = display.current.iter().map(|value| *value as f64).collect();
            benchmark.source_auxiliary = display
                .auxiliary
                .iter()
                .map(|value| *value as f64)
                .collect();
            if benchmark
                .source_current
                .iter()
                .map(|value| value.abs())
                .fold(0.0_f64, f64::max)
                < 1.0e-6
            {
                error!("Wave transfer source field is unexpectedly zero");
                exit.write(bevy::app::AppExit::error());
                return;
            }
            if benchmark
                .source_auxiliary
                .iter()
                .map(|value| value.abs())
                .fold(0.0_f64, f64::max)
                < 1.0e-6
            {
                error!("Second-order boundary memory is unexpectedly zero");
                exit.write(bevy::app::AppExit::error());
                return;
            }
            let source_previous: Vec<_> =
                display.previous.iter().map(|value| *value as f64).collect();
            let stiffness = operator.apply_stiffness(&benchmark.source_current).unwrap();
            let auxiliary = operator
                .apply_auxiliary_stiffness(&benchmark.source_auxiliary)
                .unwrap();
            let source_time = state.wave_time_offset
                + request.stats().completed_steps() as f64 * state.wave_time_step;
            benchmark.source_velocity = benchmark
                .source_current
                .iter()
                .zip(&source_previous)
                .zip(stiffness)
                .zip(auxiliary)
                .zip(operator.lumped_mass())
                .zip(operator.lumped_damping())
                .enumerate()
                .map(
                    |(node, (((((current, previous), ku), auxiliary), mass), damping))| {
                        if operator.prescribed_value(node, source_time).is_some() {
                            (operator
                                .prescribed_value(node, source_time + state.wave_time_step)
                                .unwrap()
                                - operator
                                    .prescribed_value(node, source_time - state.wave_time_step)
                                    .unwrap())
                                / (2.0 * state.wave_time_step)
                        } else {
                            centered_velocity(
                                *previous,
                                *current,
                                -(ku + auxiliary) / mass
                                    + operator.neumann_acceleration(node, source_time),
                                damping / mass,
                                state.wave_time_step,
                            )
                            .unwrap()
                        }
                    },
                )
                .collect();
            let point = state.editor.document.accepted.obstacles[0]
                .spline
                .controls()[0];
            state.editor.begin();
            state
                .editor
                .set_point(ObstacleId(1), 0, point + Point2::new(0.002, 0.001))
                .unwrap();
            state.editor.commit();
            benchmark.phase = 2;
        }
        2 => {
            let Some(candidate) = &state.simulation_candidate else {
                return;
            };
            let (Some(map), Some(generation)) = (&candidate.transfer, candidate.generation) else {
                return;
            };
            benchmark.expected_current = map.interpolate(&benchmark.source_current, 0.0).unwrap();
            let mut velocity = map.interpolate(&benchmark.source_velocity, 0.0).unwrap();
            benchmark.expected_auxiliary =
                map.interpolate(&benchmark.source_auxiliary, 0.0).unwrap();
            for (node, velocity) in velocity.iter_mut().enumerate() {
                if let Some(value) = candidate
                    .operator
                    .prescribed_value(node, candidate.simulation_time)
                {
                    benchmark.expected_current[node] = value;
                    *velocity = (candidate
                        .operator
                        .prescribed_value(node, candidate.simulation_time + candidate.time_step)
                        .unwrap()
                        - candidate
                            .operator
                            .prescribed_value(node, candidate.simulation_time - candidate.time_step)
                            .unwrap())
                        / (2.0 * candidate.time_step);
                    benchmark.expected_auxiliary[node] = 0.0;
                }
            }
            let stiffness = candidate
                .operator
                .apply_stiffness(&benchmark.expected_current)
                .unwrap();
            let auxiliary = candidate
                .operator
                .apply_auxiliary_stiffness(&benchmark.expected_auxiliary)
                .unwrap();
            benchmark.expected_previous = benchmark
                .expected_current
                .iter()
                .zip(velocity)
                .zip(stiffness)
                .zip(auxiliary)
                .zip(candidate.operator.lumped_mass())
                .zip(candidate.operator.lumped_damping())
                .enumerate()
                .map(
                    |(node, (((((current, velocity), ku), auxiliary), mass), damping))| {
                        if candidate
                            .operator
                            .prescribed_value(node, candidate.simulation_time)
                            .is_some()
                        {
                            candidate
                                .operator
                                .prescribed_value(
                                    node,
                                    candidate.simulation_time - candidate.time_step,
                                )
                                .unwrap()
                        } else {
                            centered_previous(
                                *current,
                                velocity,
                                -(ku + auxiliary) / mass
                                    + candidate
                                        .operator
                                        .neumann_acceleration(node, candidate.simulation_time),
                                damping / mass,
                                candidate.time_step,
                            )
                            .unwrap()
                        }
                    },
                )
                .collect();
            benchmark.generation = generation;
            benchmark.transfer_started = Some(Instant::now());
            benchmark.phase = 3;
        }
        3 => {
            let Some(operator) = &state.wave_operator else {
                return;
            };
            if state.simulation_candidate.is_some()
                || display.generation != benchmark.generation
                || display.current.len() != benchmark.expected_current.len()
                || display.auxiliary.len() != benchmark.expected_auxiliary.len()
            {
                return;
            }
            let relative_error = |actual: &[f32], expected: &[f64]| {
                let numerator = actual
                    .iter()
                    .zip(expected)
                    .zip(operator.lumped_mass())
                    .map(|((actual, expected), mass)| mass * (*actual as f64 - expected).powi(2))
                    .sum::<f64>();
                let denominator = expected
                    .iter()
                    .zip(operator.lumped_mass())
                    .map(|(value, mass)| mass * value * value)
                    .sum::<f64>();
                (numerator / denominator.max(f64::MIN_POSITIVE)).sqrt()
            };
            let current_error = relative_error(&display.current, &benchmark.expected_current);
            let previous_error = relative_error(&display.previous, &benchmark.expected_previous);
            let auxiliary_error = relative_error(&display.auxiliary, &benchmark.expected_auxiliary);
            info!(
                current_relative_l2 = current_error,
                previous_relative_l2 = previous_error,
                auxiliary_relative_l2 = auxiliary_error,
                elapsed_ms = benchmark.started.elapsed().as_secs_f64() * 1000.0,
                mesh_request_to_commit_ms = state.mesh_build_ms,
                operator_map_ms = state.wave_prepare_ms,
                transfer_to_readback_ms = benchmark
                    .transfer_started
                    .map_or(0.0, |started| started.elapsed().as_secs_f64() * 1000.0),
                "Geometry wave transfer check complete"
            );
            if current_error > 3.0e-5 || previous_error > 3.0e-5 || auxiliary_error > 3.0e-5 {
                error!("Transferred GPU levels differ from the f64 reference");
                exit.write(bevy::app::AppExit::error());
                return;
            }

            benchmark.source_current = display.current.iter().map(|value| *value as f64).collect();
            benchmark.source_auxiliary = display
                .auxiliary
                .iter()
                .map(|value| *value as f64)
                .collect();
            let source_previous: Vec<_> =
                display.previous.iter().map(|value| *value as f64).collect();
            let stiffness = operator.apply_stiffness(&benchmark.source_current).unwrap();
            let auxiliary = operator
                .apply_auxiliary_stiffness(&benchmark.source_auxiliary)
                .unwrap();
            let source_time = state.wave_time_offset
                + request.stats().completed_steps() as f64 * state.wave_time_step;
            benchmark.source_velocity = benchmark
                .source_current
                .iter()
                .zip(&source_previous)
                .zip(stiffness)
                .zip(auxiliary)
                .zip(operator.lumped_mass())
                .zip(operator.lumped_damping())
                .enumerate()
                .map(
                    |(node, (((((current, previous), ku), auxiliary), mass), damping))| {
                        if operator.prescribed_value(node, source_time).is_some() {
                            (operator
                                .prescribed_value(node, source_time + state.wave_time_step)
                                .unwrap()
                                - operator
                                    .prescribed_value(node, source_time - state.wave_time_step)
                                    .unwrap())
                                / (2.0 * state.wave_time_step)
                        } else {
                            centered_velocity(
                                *previous,
                                *current,
                                -(ku + auxiliary) / mass
                                    + operator.neumann_acceleration(node, source_time),
                                damping / mass,
                                state.wave_time_step,
                            )
                            .unwrap()
                        }
                    },
                )
                .collect();
            let target =
                OuterBoundaryConditions::uniform(OuterBoundaryCondition::FirstOrderOutgoing);
            state.editor.document.draft.outer_boundaries = target;
            state.editor.document.accepted.outer_boundaries = target;
            benchmark.boundary_started = Some(Instant::now());
            benchmark.phase = 4;
        }
        4 => {
            let Some(candidate) = &state.simulation_candidate else {
                return;
            };
            let (Some(map), Some(generation)) = (&candidate.transfer, candidate.generation) else {
                return;
            };
            if candidate.boundary
                != OuterBoundaryConditions::uniform(OuterBoundaryCondition::FirstOrderOutgoing)
                || !state
                    .wave_mesh
                    .as_ref()
                    .is_some_and(|mesh| Arc::ptr_eq(mesh, &candidate.mesh))
            {
                error!("Boundary transaction rebuilt or selected the wrong operator");
                exit.write(bevy::app::AppExit::error());
                return;
            }
            benchmark.expected_current = map.interpolate(&benchmark.source_current, 0.0).unwrap();
            let mut velocity = map.interpolate(&benchmark.source_velocity, 0.0).unwrap();
            benchmark.expected_auxiliary = vec![0.0; candidate.operator.degrees_of_freedom()];
            for (node, velocity) in velocity.iter_mut().enumerate() {
                if let Some(value) = candidate
                    .operator
                    .prescribed_value(node, candidate.simulation_time)
                {
                    benchmark.expected_current[node] = value;
                    *velocity = (candidate
                        .operator
                        .prescribed_value(node, candidate.simulation_time + candidate.time_step)
                        .unwrap()
                        - candidate
                            .operator
                            .prescribed_value(node, candidate.simulation_time - candidate.time_step)
                            .unwrap())
                        / (2.0 * candidate.time_step);
                }
            }
            let stiffness = candidate
                .operator
                .apply_stiffness(&benchmark.expected_current)
                .unwrap();
            benchmark.expected_previous = benchmark
                .expected_current
                .iter()
                .zip(velocity)
                .zip(stiffness)
                .zip(candidate.operator.lumped_mass())
                .zip(candidate.operator.lumped_damping())
                .enumerate()
                .map(|(node, ((((current, velocity), ku), mass), damping))| {
                    if candidate
                        .operator
                        .prescribed_value(node, candidate.simulation_time)
                        .is_some()
                    {
                        candidate
                            .operator
                            .prescribed_value(node, candidate.simulation_time - candidate.time_step)
                            .unwrap()
                    } else {
                        centered_previous(
                            *current,
                            velocity,
                            -ku / mass
                                + candidate
                                    .operator
                                    .neumann_acceleration(node, candidate.simulation_time),
                            damping / mass,
                            candidate.time_step,
                        )
                        .unwrap()
                    }
                })
                .collect();
            benchmark.generation = generation;
            benchmark.phase = 5;
        }
        5 => {
            let Some(operator) = &state.wave_operator else {
                return;
            };
            if state.simulation_candidate.is_some()
                || display.generation != benchmark.generation
                || display.current.len() != benchmark.expected_current.len()
                || display.auxiliary.len() != benchmark.expected_auxiliary.len()
            {
                return;
            }
            let relative_error = |actual: &[f32], expected: &[f64]| {
                let numerator = actual
                    .iter()
                    .zip(expected)
                    .zip(operator.lumped_mass())
                    .map(|((actual, expected), mass)| mass * (*actual as f64 - expected).powi(2))
                    .sum::<f64>();
                let denominator = expected
                    .iter()
                    .zip(operator.lumped_mass())
                    .map(|(value, mass)| mass * value * value)
                    .sum::<f64>();
                (numerator / denominator.max(f64::MIN_POSITIVE)).sqrt()
            };
            let current_error = relative_error(&display.current, &benchmark.expected_current);
            let previous_error = relative_error(&display.previous, &benchmark.expected_previous);
            let auxiliary_error = display
                .auxiliary
                .iter()
                .zip(&benchmark.expected_auxiliary)
                .map(|(actual, expected)| (*actual as f64 - expected).abs())
                .fold(0.0_f64, f64::max);
            info!(
                current_relative_l2 = current_error,
                previous_relative_l2 = previous_error,
                auxiliary_max_error = auxiliary_error,
                boundary_transaction_ms = benchmark
                    .boundary_started
                    .map_or(0.0, |started| started.elapsed().as_secs_f64() * 1000.0),
                operator_map_ms = state.wave_prepare_ms,
                "Boundary-condition wave transfer check complete"
            );
            if current_error > 3.0e-5
                || previous_error > 3.0e-5
                || auxiliary_error > 1.0e-7
                || operator.outer_boundary() != OuterBoundaryCondition::FirstOrderOutgoing
            {
                error!("Boundary-condition GPU transaction differs from the f64 reference");
                exit.write(bevy::app::AppExit::error());
                return;
            }

            benchmark.source_current = display.current.iter().map(|value| *value as f64).collect();
            let source_previous: Vec<_> =
                display.previous.iter().map(|value| *value as f64).collect();
            let stiffness = operator.apply_stiffness(&benchmark.source_current).unwrap();
            let source_time = state.wave_time_offset
                + request.stats().completed_steps() as f64 * state.wave_time_step;
            benchmark.source_velocity = benchmark
                .source_current
                .iter()
                .zip(source_previous)
                .zip(stiffness)
                .zip(operator.lumped_mass())
                .zip(operator.lumped_damping())
                .enumerate()
                .map(|(node, ((((current, previous), ku), mass), damping))| {
                    if operator.prescribed_value(node, source_time).is_some() {
                        (operator
                            .prescribed_value(node, source_time + state.wave_time_step)
                            .unwrap()
                            - operator
                                .prescribed_value(node, source_time - state.wave_time_step)
                                .unwrap())
                            / (2.0 * state.wave_time_step)
                    } else {
                        centered_velocity(
                            previous,
                            *current,
                            -ku / mass + operator.neumann_acceleration(node, source_time),
                            damping / mass,
                            state.wave_time_step,
                        )
                        .unwrap()
                    }
                })
                .collect();
            let material = state.editor.add_material().unwrap();
            let mut values = state
                .editor
                .document
                .draft
                .material(material)
                .unwrap()
                .clone();
            values.mass_density = 1.7;
            values.stiffness = 0.8;
            values.damping = 0.05;
            state.editor.update_material(values).unwrap();
            state
                .editor
                .set_region_material(BACKGROUND_REGION, material)
                .unwrap();
            benchmark.material_started = Some(Instant::now());
            benchmark.phase = 6;
        }
        6 => {
            let Some(candidate) = &state.simulation_candidate else {
                return;
            };
            let (Some(map), Some(generation)) = (&candidate.transfer, candidate.generation) else {
                return;
            };
            if state.mesh_job.is_some()
                || !state
                    .wave_mesh
                    .as_ref()
                    .is_some_and(|mesh| Arc::ptr_eq(mesh, &candidate.mesh))
            {
                error!("Material transaction rebuilt the mesh");
                exit.write(bevy::app::AppExit::error());
                return;
            }
            benchmark.expected_current = map.interpolate(&benchmark.source_current, 0.0).unwrap();
            let mut velocity = map.interpolate(&benchmark.source_velocity, 0.0).unwrap();
            for (node, velocity) in velocity.iter_mut().enumerate() {
                if let Some(value) = candidate
                    .operator
                    .prescribed_value(node, candidate.simulation_time)
                {
                    benchmark.expected_current[node] = value;
                    *velocity = (candidate
                        .operator
                        .prescribed_value(node, candidate.simulation_time + candidate.time_step)
                        .unwrap()
                        - candidate
                            .operator
                            .prescribed_value(node, candidate.simulation_time - candidate.time_step)
                            .unwrap())
                        / (2.0 * candidate.time_step);
                }
            }
            let stiffness = candidate
                .operator
                .apply_stiffness(&benchmark.expected_current)
                .unwrap();
            benchmark.expected_previous = benchmark
                .expected_current
                .iter()
                .zip(velocity)
                .zip(stiffness)
                .zip(candidate.operator.lumped_mass())
                .zip(candidate.operator.lumped_damping())
                .enumerate()
                .map(|(node, ((((current, velocity), ku), mass), damping))| {
                    if candidate
                        .operator
                        .prescribed_value(node, candidate.simulation_time)
                        .is_some()
                    {
                        candidate
                            .operator
                            .prescribed_value(node, candidate.simulation_time - candidate.time_step)
                            .unwrap()
                    } else {
                        centered_previous(
                            *current,
                            velocity,
                            -ku / mass
                                + candidate
                                    .operator
                                    .neumann_acceleration(node, candidate.simulation_time),
                            damping / mass,
                            candidate.time_step,
                        )
                        .unwrap()
                    }
                })
                .collect();
            benchmark.expected_auxiliary = vec![0.0; candidate.operator.degrees_of_freedom()];
            benchmark.generation = generation;
            benchmark.phase = 7;
        }
        7 => {
            let Some(operator) = &state.wave_operator else {
                return;
            };
            if state.simulation_candidate.is_some()
                || display.generation != benchmark.generation
                || display.current.len() != benchmark.expected_current.len()
            {
                return;
            }
            let relative_error = |actual: &[f32], expected: &[f64]| {
                let numerator = actual
                    .iter()
                    .zip(expected)
                    .zip(operator.lumped_mass())
                    .map(|((actual, expected), mass)| mass * (*actual as f64 - expected).powi(2))
                    .sum::<f64>();
                let denominator = expected
                    .iter()
                    .zip(operator.lumped_mass())
                    .map(|(value, mass)| mass * value * value)
                    .sum::<f64>();
                (numerator / denominator.max(f64::MIN_POSITIVE)).sqrt()
            };
            let current_error = relative_error(&display.current, &benchmark.expected_current);
            let previous_error = relative_error(&display.previous, &benchmark.expected_previous);
            info!(
                current_relative_l2 = current_error,
                previous_relative_l2 = previous_error,
                material_transaction_ms = benchmark
                    .material_started
                    .map_or(0.0, |started| started.elapsed().as_secs_f64() * 1000.0),
                operator_map_ms = state.wave_prepare_ms,
                "Material wave transfer check complete"
            );
            if current_error <= 3.0e-5
                && previous_error <= 3.0e-5
                && operator.outer_boundary() == OuterBoundaryCondition::FirstOrderOutgoing
            {
                request.request_steps(8);
                benchmark.phase = 8;
            } else {
                error!("Material GPU transaction differs from the f64 reference");
                exit.write(bevy::app::AppExit::error());
            }
        }
        _ => {
            if request.stats().completed_steps() < 8
                || probe_display.generation != benchmark.generation
                || state
                    .probe_compiled
                    .as_ref()
                    .is_none_or(|compiled| compiled.generation != benchmark.generation)
            {
                return;
            }
            let Some(trace) = state.probe_traces.values().next() else {
                return;
            };
            let times = trace
                .samples
                .iter()
                .map(|sample| sample.time)
                .collect::<Vec<_>>();
            let largest_gap = times
                .windows(2)
                .map(|pair| pair[1] - pair[0])
                .fold(0.0_f64, f64::max);
            let maximum_expected_gap =
                (2.5 / state.probe_sample_rate).max(2.5 * state.wave_time_step);
            let continued = times
                .last()
                .is_some_and(|time| *time > benchmark.probe_time_before);
            info!(
                samples = times.len(),
                largest_gap, maximum_expected_gap, "Probe handoff clock check complete"
            );
            if continued && largest_gap <= maximum_expected_gap {
                exit.write(bevy::app::AppExit::Success);
            } else {
                error!("Probe history did not remain continuous across solver handoffs");
                exit.write(bevy::app::AppExit::error());
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn mesh_benchmark(
    mut state: ResMut<Playground>,
    mut benchmark: ResMut<MeshBenchmark>,
    mut exit: MessageWriter<AppExit>,
) {
    if benchmark
        .start
        .is_some_and(|start| start.elapsed().as_secs_f64() > 60.0)
    {
        eprintln!(
            "NATIVE_MESH_BENCHMARK timed out: editing={} acceptance={:?} requested_revision={} mesh_revision={:?}",
            state.editor.editing(),
            state.editor.acceptance,
            state.editor.revision,
            state.mesh.as_ref().map(|m| m.geometry_revision)
        );
        exit.write(AppExit::Error(std::num::NonZeroU8::new(1).unwrap()));
        return;
    }
    if let Some(error) = &state.mesh_error {
        eprintln!("NATIVE_MESH_BENCHMARK failed: {error}");
        exit.write(AppExit::Error(std::num::NonZeroU8::new(1).unwrap()));
        return;
    }
    if state.mesh.is_none()
        || state.mesh_job.is_some()
        || state.editor.acceptance != Acceptance::Valid
        || state.mesh_committed_scene != state.editor.document.accepted
        || benchmark
            .target
            .as_ref()
            .is_some_and(|target| target != &state.mesh_committed_scene)
    {
        return;
    }
    let mesh = state.mesh.as_ref().unwrap();
    println!(
        "NATIVE_MESH_BENCHMARK edit={} edit_to_ready_ms={:.2} request_to_ready_ms={:.2} active_ms={:.2} gap_ms={:.2} max_slice_ms={:.2} frame_ms={:.2} triangles={} report={:?}",
        benchmark.edits,
        benchmark
            .start
            .map_or(state.mesh_build_ms, |t| t.elapsed().as_secs_f64() * 1000.0),
        state.mesh_build_ms,
        state.mesh_work_ms,
        (state.mesh_build_ms - state.mesh_work_ms).max(0.0),
        state.mesh_max_slice_ms,
        state.frame_ms,
        mesh.triangles.len(),
        state.mesh_report
    );
    if benchmark.edits == 3 {
        exit.write(AppExit::Success);
        return;
    }
    let delta = [
        Point2::new(0.005, 0.0),
        Point2::new(0.0, 0.005),
        Point2::new(-0.005, -0.005),
    ][benchmark.edits];
    let p = state.editor.document.accepted.obstacles[0]
        .spline
        .controls()[0];
    benchmark.start = Some(Instant::now());
    state.editor.begin();
    state.editor.set_point(ObstacleId(1), 0, p + delta).unwrap();
    state.editor.commit();
    benchmark.target = Some(state.editor.document.draft.clone());
    benchmark.edits += 1;
}

impl Playground {
    fn show(&mut self, root: &mut egui::Ui, wave_display: Option<&WaveDisplay>) -> Rect {
        // Remember capture before panels can end a text edit this frame.
        self.keyboard_captured = root.ctx().text_edit_focused();
        if self.wave_source != self.editor.document.source {
            self.wave_source = self.editor.document.source;
            self.wave_source_dirty = true;
        }
        self.expire_notice();
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
        let state = self;
        egui::Panel::top("topbar")
            .exact_size(42.0)
            .resizable(false)
            .show(root, |ui| state.top_bar(ui));
        egui::Panel::bottom("status")
            .exact_size(29.0)
            .resizable(false)
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    let (text, color) = match &state.editor.acceptance {
                        Acceptance::Valid => ("Accepted · geometry valid".into(), TEAL),
                        Acceptance::Pending => ("Checking draft…".into(), GOLD),
                        Acceptance::Invalid(issue) => {
                            (format!("Draft not accepted · {issue}"), RED)
                        }
                    };
                    ui.colored_label(color, text);
                    if state.load.is_some() {
                        ui.colored_label(GOLD, "Validating scene file…");
                    } else if let Some(job) = &state.mesh_job {
                        ui.colored_label(GOLD, format!("Mesh rebuilding: {}", job.phase()));
                    } else if let Some(job) = &state.solution_indicator_job {
                        ui.colored_label(GOLD, job.phase());
                    } else if let Some(job) = &state.mesh_adaptation_job {
                        ui.colored_label(GOLD, format!("Mesh adapting: {}", job.phase()));
                    } else if let Some(candidate) = &state.simulation_candidate {
                        ui.colored_label(
                            GOLD,
                            if candidate.generation.is_some() {
                                "Uploading solver state…".into()
                            } else {
                                format!(
                                    "Preparing solver handoff · {} vertices · {} triangles",
                                    candidate.mesh.vertices.len(),
                                    candidate.mesh.triangles.len()
                                )
                            },
                        );
                    } else if state.mesh_error.is_some()
                        || state.wave_error.is_some()
                        || state.amr_error.is_some()
                    {
                        ui.colored_label(RED, "Attention required");
                    } else if state.mesh.is_none() {
                        ui.colored_label(GOLD, "Preparing initial mesh…");
                    }
                    if !state.message.is_empty() {
                        ui.colored_label(RED, &state.message);
                    }
                    if let Some(notice) = &state.notice {
                        ui.colored_label(GOLD, &notice.text);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let warning = state.performance_warning();
                        let summary = if ui.ctx().content_rect().width() < 1100.0 {
                            state.compact_performance_summary()
                        } else {
                            state.performance_summary()
                        };
                        let label = if warning {
                            format!("⚠ {summary}")
                        } else {
                            summary
                        };
                        if ui
                            .small_button(label)
                            .on_hover_text("Open performance diagnostics")
                            .clicked()
                        {
                            state.performance_open = !state.performance_open;
                        }
                    });
                });
            });
        if state.inspector_panel.is_some() {
            egui::Panel::right("inspector")
                .default_size(340.0)
                .min_size(280.0)
                .max_size(430.0)
                .show(root, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let enabled = !state.file_busy && state.load.is_none();
                        ui.add_enabled_ui(enabled, |ui| state.panel(ui));
                    });
                });
        }
        state.add_geometry_popover(root.ctx());
        state.example_gallery(root.ctx());
        state.performance_window(root.ctx());
        state.probe_readout_windows(root.ctx());
        state.far_field_readout_window(root.ctx());
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(root, |ui| state.viewport(ui, wave_display))
            .inner
    }
}

fn paint_example_thumbnail(
    ui: &mut egui::Ui,
    example: &examples::ExampleScene,
    size: egui::Vec2,
) -> egui::Response {
    let scene = &example.document.accepted;
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let painter = ui.painter_at(rect);
    let background = scene
        .region_material(BACKGROUND_REGION)
        .map(|material| Color32::from_rgb(material.color[0], material.color[1], material.color[2]))
        .unwrap_or(Color32::from_rgb(47, 73, 88));
    painter.rect_filled(rect, 5.0, background);
    painter.rect_stroke(
        rect,
        5.0,
        Stroke::new(1.0, Color32::from_rgb(123, 151, 160)),
        egui::StrokeKind::Inside,
    );
    let project = |point: Point2| {
        egui::pos2(
            egui::lerp(
                rect.left() + 7.0..=rect.right() - 7.0,
                ((point.x + 1.0) * 0.5) as f32,
            ),
            egui::lerp(
                rect.bottom() - 7.0..=rect.top() + 7.0,
                ((point.y + 1.0) * 0.5) as f32,
            ),
        )
    };
    for obstacle in &scene.obstacles {
        let points = (0..=64)
            .map(|index| {
                project(
                    obstacle
                        .spline
                        .evaluate(obstacle.spline.period() * index as f64 / 64.0),
                )
            })
            .collect::<Vec<_>>();
        let fill = match obstacle.role {
            LoopRole::Hole { .. } | LoopRole::Wall { .. } => Color32::from_rgb(16, 23, 31),
            LoopRole::MaterialInterface { interior, .. } => scene
                .region_material(interior)
                .map(|material| {
                    Color32::from_rgb(material.color[0], material.color[1], material.color[2])
                })
                .unwrap_or(background),
        };
        painter.add(egui::Shape::convex_polygon(
            points,
            fill,
            Stroke::new(1.5, TEAL),
        ));
    }
    for boundary in &scene.internal_boundaries {
        let points = (0..=48)
            .map(|index| {
                project(
                    boundary
                        .spline
                        .evaluate(boundary.spline.period() * index as f64 / 48.0),
                )
            })
            .collect();
        painter.add(egui::Shape::line(points, Stroke::new(2.0, TEAL)));
    }
    let side_points = [
        (rect.left_bottom(), rect.right_bottom()),
        (rect.right_bottom(), rect.right_top()),
        (rect.right_top(), rect.left_top()),
        (rect.left_top(), rect.left_bottom()),
    ];
    for side in OuterSide::ALL {
        let (start, end) = side_points[side.index()];
        painter.line_segment(
            [start, end],
            Stroke::new(
                3.0,
                outer_boundary_condition_color(scene.outer_boundaries.get(side)),
            ),
        );
    }
    if example.simulation.source.enabled {
        let center = project(example.simulation.source.position);
        painter.circle_stroke(center, 5.0, Stroke::new(1.8, GOLD));
        painter.line_segment(
            [
                center + egui::vec2(-7.0, 0.0),
                center + egui::vec2(7.0, 0.0),
            ],
            Stroke::new(1.0, GOLD),
        );
        painter.line_segment(
            [
                center + egui::vec2(0.0, -7.0),
                center + egui::vec2(0.0, 7.0),
            ],
            Stroke::new(1.0, GOLD),
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers, PointerButton};
    struct Harness {
        state: Playground,
        ctx: egui::Context,
        rect: Rect,
        time: f64,
        size: egui::Vec2,
        texts: Vec<(String, Rect)>,
    }
    impl Harness {
        fn new() -> Self {
            let ctx = egui::Context::default();
            ctx.options_mut(|o| o.max_passes = 1.try_into().unwrap());
            let mut h = Self {
                state: Playground::default(),
                ctx,
                rect: Rect::NOTHING,
                time: 0.0,
                size: egui::vec2(1280.0, 800.0),
                texts: vec![],
            };
            h.frame(vec![]);
            h.frame(vec![]);
            h
        }
        fn frame(&mut self, events: Vec<Event>) {
            self.time += 1.0 / 60.0;
            let output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
                    time: Some(self.time),
                    events,
                    focused: true,
                    ..Default::default()
                },
                |ui| self.rect = self.state.show(ui, None),
            );
            self.texts.clear();
            fn collect(shape: &egui::Shape, texts: &mut Vec<(String, Rect)>) {
                match shape {
                    egui::Shape::Text(t) => texts.push((
                        t.galley.job.text.clone(),
                        Rect::from_min_size(t.pos, t.galley.size()),
                    )),
                    egui::Shape::Vec(shapes) => {
                        for shape in shapes {
                            collect(shape, texts);
                        }
                    }
                    _ => {}
                }
            }
            for shape in &output.shapes {
                collect(&shape.shape, &mut self.texts);
            }
            output.drop_without_applying_deltas();
            self.state.editor.validate_frame(12_000);
        }
        fn point(&self, p: Point2) -> Pos2 {
            self.state.screen(p, self.rect)
        }
        fn scale_drag_end(&self, pivot: Point2, factor: f32) -> Pos2 {
            let center = self.point(pivot);
            let start = self.state.scale_handle_position(self.rect, pivot);
            let radial = start - center;
            let distance = (radial.length() - GIZMO_PADDING as f32) * factor + GIZMO_PADDING as f32;
            center + radial.normalized() * distance
        }
        fn move_to(&mut self, p: Pos2) {
            self.frame(vec![Event::PointerMoved(p)]);
        }
        fn button(&mut self, p: Pos2, button: PointerButton, pressed: bool) {
            self.frame(vec![
                Event::PointerMoved(p),
                Event::PointerButton {
                    pos: p,
                    button,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ]);
        }
        fn click(&mut self, p: Pos2) {
            self.move_to(p);
            self.button(p, PointerButton::Primary, true);
            self.button(p, PointerButton::Primary, false);
        }
        fn click_with_modifiers(&mut self, p: Pos2, modifiers: Modifiers) {
            self.frame(vec![
                Event::ModifiersChanged(modifiers),
                Event::PointerMoved(p),
                Event::PointerButton {
                    pos: p,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers,
                },
            ]);
            self.frame(vec![
                Event::PointerButton {
                    pos: p,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers,
                },
                Event::ModifiersChanged(Modifiers::NONE),
            ]);
        }
        fn drag_with_modifiers(&mut self, start: Pos2, end: Pos2, modifiers: Modifiers) {
            self.frame(vec![
                Event::ModifiersChanged(modifiers),
                Event::PointerMoved(start),
                Event::PointerButton {
                    pos: start,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers,
                },
            ]);
            self.frame(vec![Event::PointerMoved(end)]);
            self.frame(vec![
                Event::PointerButton {
                    pos: end,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers,
                },
                Event::ModifiersChanged(Modifiers::NONE),
            ]);
        }
        fn click_text(&mut self, text: &str) {
            self.frame(vec![]);
            let position = self
                .texts
                .iter()
                .find(|(candidate, _)| candidate == text)
                .map(|(_, rect)| rect.center())
                .unwrap_or_else(|| panic!("missing UI text {text:?}"));
            self.click(position);
        }
        fn key(&mut self, key: Key, modifiers: Modifiers) {
            self.frame(vec![
                Event::ModifiersChanged(modifiers),
                Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                },
            ]);
            self.frame(vec![
                Event::Key {
                    key,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers,
                },
                Event::ModifiersChanged(Modifiers::NONE),
            ]);
        }
        fn wheel(&mut self, p: Pos2) {
            self.frame(vec![
                Event::PointerMoved(p),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 60.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: Modifiers::NONE,
                },
            ]);
        }
        fn settle(&mut self) {
            for _ in 0..130 {
                self.frame(vec![]);
                if self.state.editor.acceptance != Acceptance::Pending {
                    return;
                }
            }
            panic!("did not settle");
        }
    }

    fn build_mesh_candidate(state: &mut Playground) {
        for _ in 0..5_000 {
            state.refresh_mesh();
            if state.simulation_candidate.is_some() || state.mesh_error.is_some() {
                return;
            }
        }
        panic!("mesh candidate did not finish");
    }

    #[test]
    fn custom_open_baffle_creation_and_handle_drag() {
        let mut harness = Harness::new();
        harness.state.creation_role = CreationRole::InternalBoundary;
        harness.state.interaction_mode = InteractionMode::DrawCustom {
            role: CreationRole::InternalBoundary,
        };
        for point in [
            Point2::new(-0.65, 0.48),
            Point2::new(-0.25, 0.62),
            Point2::new(0.25, 0.45),
            Point2::new(0.65, 0.56),
        ] {
            harness.click(harness.point(point));
        }
        harness.key(Key::Enter, Modifiers::NONE);
        harness.settle();
        assert!(matches!(harness.state.editor.acceptance, Acceptance::Valid));
        assert_eq!(
            harness
                .state
                .editor
                .document
                .draft
                .internal_boundaries
                .len(),
            1
        );
        let id = harness.state.editor.document.draft.internal_boundaries[0].id;
        assert_eq!(harness.state.internal_selection, None);
        assert_eq!(
            harness.state.focused_feature,
            Some(FocusedFeature::Baffle(id))
        );
        assert_eq!(
            harness.state.selected_spans,
            vec![GeometrySpan::Baffle(id, 0)]
        );
        assert_eq!(harness.state.baffle_face, InternalBoundarySide::Left);

        let history_before_laws = harness.state.editor.history_len().0;
        harness.click_text("Reflecting");
        harness.click_text("First-order outgoing / impedance");
        harness.settle();
        assert_eq!(
            harness
                .state
                .editor
                .internal_boundary(id)
                .unwrap()
                .span_laws[0]
                .left,
            FaceBoundaryCondition::Impedance { ratio: 1.0 }
        );
        harness.click_text("Independent faces");
        harness.click_text("Coupled thin gap");
        harness.settle();
        assert!(matches!(
            harness
                .state
                .editor
                .internal_boundary(id)
                .unwrap()
                .span_laws[0]
                .coupling,
            InternalBoundaryCoupling::ThinGap {
                stiffness_ratio: 1.0
            }
        ));
        assert_eq!(
            harness
                .state
                .editor
                .internal_boundary(id)
                .unwrap()
                .span_laws[0]
                .left,
            FaceBoundaryCondition::Reflecting
        );
        assert_eq!(
            harness.state.editor.history_len().0,
            history_before_laws + 2
        );

        let before = harness.state.editor.history_len().0;
        let control = harness
            .state
            .editor
            .internal_boundary(id)
            .unwrap()
            .spline
            .controls()[1];
        let start = harness.point(control);
        harness.button(start, PointerButton::Primary, true);
        let end = harness.point(control + Point2::new(0.04, 0.03));
        harness.move_to(end);
        harness.button(end, PointerButton::Primary, false);
        harness.settle();
        assert_eq!(harness.state.editor.history_len().0, before + 1);
        assert_eq!(harness.state.internal_selection, Some((id, Some(1))));
    }

    #[test]
    fn hole_curve_selection_assigns_the_clicked_span_condition() {
        let mut harness = Harness::new();
        let point = harness
            .state
            .editor
            .obstacle(ObstacleId(1))
            .unwrap()
            .spline
            .evaluate(3.5);
        harness.click(harness.point(point));
        assert_eq!(
            harness.state.selected_spans,
            vec![GeometrySpan::Loop(ObstacleId(1), 3)]
        );

        let history = harness.state.editor.history_len().0;
        harness.click_text("Reflecting");
        harness.click_text("First-order outgoing / impedance");
        harness.settle();
        assert_eq!(
            harness
                .state
                .editor
                .obstacle(ObstacleId(1))
                .unwrap()
                .span_conditions[3],
            FaceBoundaryCondition::Impedance { ratio: 1.0 }
        );
        assert_eq!(harness.state.editor.history_len().0, history + 1);
    }

    #[test]
    fn face_condition_picker_exposes_driven_and_second_order_conditions() {
        let mut harness = Harness::new();
        let history = harness.state.editor.history_len().0;
        harness.click_text("Reflecting");
        harness.click_text("Dirichlet · prescribed value");
        harness.settle();
        assert!(
            harness
                .state
                .editor
                .obstacle(ObstacleId(1))
                .unwrap()
                .span_conditions
                .iter()
                .all(|condition| *condition
                    == FaceBoundaryCondition::Dirichlet {
                        signal: BoundarySignal::ZERO
                    })
        );

        harness.click_text("Prescribed Dirichlet");
        harness.click_text("Second-order outgoing");
        harness.settle();
        assert!(
            harness
                .state
                .editor
                .obstacle(ObstacleId(1))
                .unwrap()
                .span_conditions
                .iter()
                .all(|condition| *condition == FaceBoundaryCondition::SecondOrderOutgoing)
        );
        assert_eq!(harness.state.editor.history_len().0, history + 2);
    }

    #[test]
    fn open_curve_click_selects_its_logical_knot_span() {
        let mut harness = Harness::new();
        let id = harness
            .state
            .editor
            .create_internal_boundary(
                OpenCubicSpline::uniform(vec![
                    Point2::new(-0.8, 0.58),
                    Point2::new(-0.45, 0.72),
                    Point2::new(0.0, 0.50),
                    Point2::new(0.45, 0.70),
                    Point2::new(0.8, 0.56),
                ])
                .unwrap(),
                BACKGROUND_REGION,
            )
            .unwrap();
        harness.settle();
        harness.state.selection = None;
        harness.state.internal_selection = None;
        harness.state.set_span_selection(vec![]);
        let point = harness
            .state
            .editor
            .internal_boundary(id)
            .unwrap()
            .spline
            .evaluate(1.5);
        harness.click(harness.point(point));
        assert_eq!(harness.state.internal_selection, None);
        assert_eq!(harness.state.focused_feature, None);
        assert_eq!(
            harness.state.selected_spans,
            vec![GeometrySpan::Baffle(id, 1)]
        );
    }

    #[test]
    fn outer_edge_click_selects_and_configures_that_boundary() {
        let mut harness = Harness::new();
        harness.click(harness.point(Point2::new(0.35, 1.0)));
        assert_eq!(
            harness.state.selected_spans,
            vec![GeometrySpan::Outer(OuterSide::Top)]
        );
        assert_eq!(harness.state.selection, None);
        assert_eq!(harness.state.internal_selection, None);

        let history = harness.state.editor.history_len().0;
        harness.click_text("Second-order outgoing");
        harness.click_text("First-order outgoing / impedance");
        harness.settle();
        assert_eq!(
            harness
                .state
                .editor
                .document
                .draft
                .outer_boundaries
                .get(OuterSide::Top),
            OuterBoundaryCondition::FirstOrderOutgoing
        );
        assert_eq!(harness.state.editor.history_len().0, history + 1);
    }

    fn commit_mesh_without_gpu(state: &mut Playground) {
        let candidate = state
            .simulation_candidate
            .take()
            .expect("completed mesh candidate");
        state.mesh = Some(candidate.mesh.clone());
        state.mesh_low_quality = candidate.low_quality;
        state.mesh_committed_scene = candidate.scene;
        state.mesh_committed_max_edge = candidate.max_edge;
        state.wave_mesh = Some(candidate.mesh);
        state.wave_operator = Some(candidate.operator);
        state.wave_boundary_committed = candidate.boundary;
        state.wave_time_step = candidate.time_step;
    }
    #[test]
    fn pointer_drag_escape_and_one_entry_history() {
        let mut h = Harness::new();
        let before = h.state.editor.document.clone();
        let p = h.point(Point2::new(0.15, 0.0));
        h.click(p);
        assert_eq!(h.state.selection, Some((ObstacleId(1), Some(0))));
        assert_eq!(h.state.editor.history_len(), (0, 0));
        h.button(p, PointerButton::Primary, true);
        let moved = h.point(Point2::new(0.3, 0.1));
        h.move_to(moved);
        h.move_to(moved + egui::vec2(15.0, 0.0));
        h.button(moved, PointerButton::Primary, false);
        h.settle();
        assert_eq!(h.state.editor.history_len(), (1, 0));
        assert_ne!(h.state.editor.document, before);
        h.key(Key::Z, Modifiers::COMMAND);
        assert_eq!(h.state.editor.document, before);
        h.key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
        assert_ne!(h.state.editor.document, before);
        let before = h.state.editor.document.clone();
        let p = h.point(before.draft.obstacles[0].spline.controls()[0]);
        h.button(p, PointerButton::Primary, true);
        h.move_to(p + egui::vec2(25.0, 0.0));
        h.key(Key::Escape, Modifiers::NONE);
        h.button(p, PointerButton::Primary, false);
        assert_eq!(h.state.editor.document, before);
        assert_eq!(h.state.editor.history_len(), (1, 0));
    }

    #[test]
    fn handles_are_exclusive_and_snapped_curve_drag_is_rigid() {
        let mut harness = Harness::new();
        let id = ObstacleId(1);
        let controls = harness
            .state
            .editor
            .obstacle(id)
            .unwrap()
            .spline
            .controls()
            .to_vec();
        harness.click(harness.point(controls[0]));
        harness.click_with_modifiers(harness.point(controls[2]), Modifiers::SHIFT);
        assert_eq!(harness.state.selection, Some((id, Some(2))));
        assert!(harness.state.selected_spans.is_empty());

        harness.state.select_loop(id);
        harness.state.snap_to_grid = true;
        harness.state.snap_step = 0.1;
        let curve_point = harness
            .state
            .editor
            .obstacle(id)
            .unwrap()
            .spline
            .evaluate(0.5);
        let start = harness.point(curve_point);
        harness.button(start, PointerButton::Primary, true);
        harness.move_to(start + egui::vec2(19.0, -13.0));
        harness.button(
            start + egui::vec2(19.0, -13.0),
            PointerButton::Primary,
            false,
        );
        harness.settle();
        let moved = harness.state.editor.obstacle(id).unwrap().spline.controls();
        let delta = moved[0] - controls[0];
        assert!(delta.norm() > 0.0);
        assert!(
            moved
                .iter()
                .zip(&controls)
                .all(|(moved, original)| (*moved - *original - delta).norm() < 1.0e-12)
        );
        assert_eq!(harness.state.editor.history_len(), (1, 0));
    }

    #[test]
    fn shift_selects_spans_and_command_selects_the_whole_curve() {
        let mut harness = Harness::new();
        let id = ObstacleId(1);
        harness.state.set_span_selection(vec![]);
        let spline = &harness.state.editor.obstacle(id).unwrap().spline;
        let first = harness.point(spline.evaluate(0.5));
        let third = harness.point(spline.evaluate(2.5));
        harness.click(first);
        harness.click_with_modifiers(third, Modifiers::SHIFT);
        assert_eq!(
            harness.state.selected_spans,
            vec![GeometrySpan::Loop(id, 0), GeometrySpan::Loop(id, 2)]
        );
        assert!(harness.state.transformable_curve_controls().is_none());

        harness.click_with_modifiers(first, Modifiers::COMMAND);
        assert_eq!(harness.state.selected_spans.len(), 8);
        assert!(harness.state.transformable_curve_controls().is_some());
    }

    #[test]
    fn marquee_filter_selects_and_subtracts_baffle_spans_without_history() {
        let mut harness = Harness::new();
        let id = harness
            .state
            .editor
            .create_internal_boundary(
                OpenCubicSpline::uniform(vec![
                    Point2::new(-0.8, 0.6),
                    Point2::new(-0.4, 0.7),
                    Point2::new(0.0, 0.55),
                    Point2::new(0.4, 0.7),
                    Point2::new(0.8, 0.6),
                ])
                .unwrap(),
                BACKGROUND_REGION,
            )
            .unwrap();
        harness.settle();
        harness.state.span_selection_filter = SpanSelectionFilter::Baffles;
        harness.state.set_span_selection(vec![]);
        harness.frame(vec![]);
        let history = harness.state.editor.history_len();
        let start = harness.point(Point2::new(-0.9, 0.43));
        let end = harness.point(Point2::new(0.9, 0.78));
        harness.drag_with_modifiers(start, end, Modifiers::NONE);
        assert_eq!(
            harness.state.selected_spans,
            vec![GeometrySpan::Baffle(id, 0), GeometrySpan::Baffle(id, 1)]
        );
        assert_eq!(harness.state.editor.history_len(), history);

        harness.drag_with_modifiers(start, end, Modifiers::ALT);
        assert!(harness.state.selected_spans.is_empty());
        assert_eq!(harness.state.editor.history_len(), history);

        harness.move_to(harness.rect.center());
        harness.key(Key::A, Modifiers::COMMAND);
        assert_eq!(
            harness.state.selected_spans,
            vec![GeometrySpan::Baffle(id, 0), GeometrySpan::Baffle(id, 1)]
        );
    }

    #[test]
    fn marquee_escape_restores_the_previous_span_selection() {
        let mut harness = Harness::new();
        let previous = vec![GeometrySpan::Loop(ObstacleId(1), 2)];
        harness.state.set_span_selection(previous.clone());
        let start = harness.point(Point2::new(-0.8, 0.7));
        let end = harness.point(Point2::new(0.8, -0.7));
        harness.button(start, PointerButton::Primary, true);
        harness.move_to(end);
        harness.key(Key::Escape, Modifiers::NONE);
        harness.button(end, PointerButton::Primary, false);
        assert_eq!(harness.state.selected_spans, previous);
        assert_eq!(harness.state.editor.history_len(), (0, 0));
    }

    #[test]
    fn selected_baffle_spans_share_one_coherent_face() {
        let mut harness = Harness::new();
        let id = harness
            .state
            .editor
            .create_internal_boundary(
                OpenCubicSpline::uniform(vec![
                    Point2::new(-0.8, 0.6),
                    Point2::new(-0.4, 0.7),
                    Point2::new(0.0, 0.55),
                    Point2::new(0.4, 0.7),
                    Point2::new(0.8, 0.6),
                ])
                .unwrap(),
                BACKGROUND_REGION,
            )
            .unwrap();
        harness.state.set_span_selection(vec![
            GeometrySpan::Baffle(id, 0),
            GeometrySpan::Baffle(id, 1),
        ]);
        harness.state.baffle_face = InternalBoundarySide::Right;
        let targets = harness.state.selected_boundary_targets().unwrap();
        assert_eq!(
            targets,
            vec![
                BoundaryFaceTarget::Baffle(id, 0, InternalBoundarySide::Right),
                BoundaryFaceTarget::Baffle(id, 1, InternalBoundarySide::Right),
            ]
        );
        harness
            .state
            .editor
            .set_boundary_face_conditions(&targets, FaceBoundaryCondition::SecondOrderOutgoing)
            .unwrap();
        for law in &harness
            .state
            .editor
            .internal_boundary(id)
            .unwrap()
            .span_laws
        {
            assert_eq!(law.left, FaceBoundaryCondition::Reflecting);
            assert_eq!(law.right, FaceBoundaryCondition::SecondOrderOutgoing);
        }
    }

    #[test]
    fn rotation_ring_is_rigid_and_pivot_drag_is_transient() {
        let mut harness = Harness::new();
        let id = ObstacleId(1);
        let before = harness
            .state
            .editor
            .obstacle(id)
            .unwrap()
            .spline
            .controls()
            .to_vec();
        let pivot = harness.state.selection_pivot().unwrap();
        let center = harness.point(pivot);
        let radius = harness.state.gizmo_radius(harness.rect, pivot);
        let start = center + egui::vec2(radius, 0.0);
        let end = center + egui::vec2(0.0, -radius);
        harness.button(start, PointerButton::Primary, true);
        harness.move_to(end);
        harness.button(end, PointerButton::Primary, false);
        harness.settle();
        let rotated = harness
            .state
            .editor
            .obstacle(id)
            .unwrap()
            .spline
            .controls()
            .to_vec();
        for i in 0..before.len() {
            for j in 0..before.len() {
                assert!(
                    ((before[i] - before[j]).norm() - (rotated[i] - rotated[j]).norm()).abs()
                        < 1.0e-10
                );
            }
        }
        assert_eq!(harness.state.editor.history_len(), (1, 0));

        let document = harness.state.editor.document.clone();
        let moved_center = center + egui::vec2(17.0, -11.0);
        harness.button(center, PointerButton::Primary, true);
        harness.move_to(moved_center);
        harness.button(moved_center, PointerButton::Primary, false);
        assert_eq!(harness.state.editor.document, document);
        assert_eq!(harness.state.editor.history_len(), (1, 0));
        assert!((harness.state.gizmo_pivot.unwrap() - pivot).norm() > 0.0);
    }

    #[test]
    fn scale_handle_scales_about_pivot_as_one_undoable_drag() {
        let mut harness = Harness::new();
        let id = ObstacleId(1);
        let document = harness.state.editor.document.clone();
        let before = harness
            .state
            .editor
            .obstacle(id)
            .unwrap()
            .spline
            .controls()
            .to_vec();
        let pivot = harness.state.selection_pivot().unwrap();
        let start = harness.state.scale_handle_position(harness.rect, pivot);
        assert!(matches!(
            harness.state.hit_gizmo(start, harness.rect),
            Some(GizmoHit::Scale)
        ));
        let end = harness.scale_drag_end(pivot, 1.25);
        harness.button(start, PointerButton::Primary, true);
        harness.move_to(end);
        harness.button(end, PointerButton::Primary, false);
        harness.settle();

        let scaled = harness.state.editor.obstacle(id).unwrap().spline.controls();
        assert!(scaled.iter().zip(&before).all(|(scaled, before)| {
            (*scaled - pivot - (*before - pivot) * 1.25).norm() < 1.0e-6
        }));
        assert!((harness.state.selection_pivot().unwrap() - pivot).norm() < 1.0e-10);
        assert_eq!(harness.state.editor.history_len(), (1, 0));
        harness.state.editor.undo();
        assert_eq!(harness.state.editor.document, document);
    }

    #[test]
    fn shift_snaps_scale_and_escape_cancels_scale_drag() {
        let mut harness = Harness::new();
        let id = ObstacleId(1);
        let before = harness
            .state
            .editor
            .obstacle(id)
            .unwrap()
            .spline
            .controls()
            .to_vec();
        let pivot = harness.state.selection_pivot().unwrap();
        let start = harness.state.scale_handle_position(harness.rect, pivot);
        let end = harness.scale_drag_end(pivot, 1.26);
        harness.drag_with_modifiers(start, end, Modifiers::SHIFT);
        harness.settle();
        let scaled = harness.state.editor.obstacle(id).unwrap().spline.controls();
        assert!(scaled.iter().zip(&before).all(|(scaled, before)| {
            (*scaled - pivot - (*before - pivot) * 1.3).norm() < 1.0e-10
        }));

        let document = harness.state.editor.document.clone();
        let pivot = harness.state.selection_pivot().unwrap();
        let start = harness.state.scale_handle_position(harness.rect, pivot);
        let end = harness.scale_drag_end(pivot, 0.7);
        harness.button(start, PointerButton::Primary, true);
        harness.move_to(end);
        harness.key(Key::Escape, Modifiers::NONE);
        harness.button(end, PointerButton::Primary, false);
        assert_eq!(harness.state.editor.document, document);
        assert_eq!(harness.state.editor.history_len(), (1, 0));
    }

    #[test]
    fn invalid_scale_stays_in_draft_and_preserves_accepted_scene() {
        let mut harness = Harness::new();
        let accepted = harness.state.editor.document.accepted.clone();
        harness.state.scale = 100.0;
        let pivot = harness.state.selection_pivot().unwrap();
        let start = harness.state.scale_handle_position(harness.rect, pivot);
        let end = harness.scale_drag_end(pivot, 9.0);
        harness.button(start, PointerButton::Primary, true);
        harness.move_to(end);
        harness.button(end, PointerButton::Primary, false);
        harness.settle();

        assert!(matches!(
            harness.state.editor.acceptance,
            Acceptance::Invalid(_)
        ));
        assert_eq!(harness.state.editor.document.accepted, accepted);
        assert_ne!(harness.state.editor.document.draft, accepted);
        assert_eq!(harness.state.editor.history_len(), (1, 0));
    }

    #[test]
    fn shift_click_toggles_but_shift_drag_adds_moves_and_grid_snaps() {
        let mut harness = Harness::new();
        let id = ObstacleId(1);
        let spline = &harness.state.editor.obstacle(id).unwrap().spline;
        let first = harness.point(spline.evaluate(0.5));
        harness.click_with_modifiers(first, Modifiers::SHIFT);
        assert_eq!(harness.state.selected_spans.len(), 7);
        assert!(
            !harness
                .state
                .selected_spans
                .contains(&GeometrySpan::Loop(id, 0))
        );
        assert_eq!(harness.state.editor.history_len(), (0, 0));

        let before = harness
            .state
            .editor
            .obstacle(id)
            .unwrap()
            .spline
            .controls()
            .to_vec();
        harness.state.snap_to_grid = false;
        harness.state.snap_step = 0.1;
        let start_world = harness
            .state
            .editor
            .obstacle(id)
            .unwrap()
            .spline
            .evaluate(0.5);
        let start = harness.point(start_world);
        let end = harness.point(start_world + Point2::new(0.07, 0.06));
        harness.drag_with_modifiers(start, end, Modifiers::SHIFT);
        harness.settle();

        assert_eq!(harness.state.selected_spans.len(), 8);
        let moved = harness.state.editor.obstacle(id).unwrap().spline.controls();
        let delta = moved[0] - before[0];
        assert!(
            moved
                .iter()
                .zip(&before)
                .all(|(moved, before)| (*moved - *before - delta).norm() < 1.0e-12)
        );
        let pivot = harness.state.selection_pivot().unwrap();
        assert!((pivot.x / 0.1 - (pivot.x / 0.1).round()).abs() < 1.0e-10);
        assert!((pivot.y / 0.1 - (pivot.y / 0.1).round()).abs() < 1.0e-10);
        assert_eq!(harness.state.editor.history_len(), (1, 0));

        let start_world = harness
            .state
            .editor
            .obstacle(id)
            .unwrap()
            .spline
            .evaluate(2.5);
        harness.drag_with_modifiers(
            harness.point(start_world),
            harness.point(start_world + Point2::new(0.07, -0.06)),
            Modifiers::SHIFT,
        );
        harness.settle();
        assert_eq!(harness.state.selected_spans.len(), 8);
        assert_eq!(harness.state.editor.history_len(), (2, 0));
    }

    #[test]
    fn shift_drag_snaps_a_control_when_persistent_snap_is_off() {
        let mut harness = Harness::new();
        let id = ObstacleId(1);
        harness.state.snap_to_grid = false;
        harness.state.snap_step = 0.1;
        let before = harness.state.editor.obstacle(id).unwrap().spline.controls()[0];
        let start = harness.point(before);
        let end = harness.point(before + Point2::new(0.07, 0.06));
        harness.drag_with_modifiers(start, end, Modifiers::SHIFT);
        harness.settle();
        let moved = harness.state.editor.obstacle(id).unwrap().spline.controls()[0];
        assert!((moved.x / 0.1 - (moved.x / 0.1).round()).abs() < 1.0e-10);
        assert!((moved.y / 0.1 - (moved.y / 0.1).round()).abs() < 1.0e-10);
        assert_eq!(harness.state.selection, Some((id, Some(0))));
        assert_eq!(harness.state.editor.history_len(), (1, 0));
    }

    #[test]
    fn whole_curve_drag_and_panel_transform_move_all_controls() {
        let mut harness = Harness::new();
        let id = ObstacleId(1);
        let before = harness
            .state
            .editor
            .obstacle(id)
            .unwrap()
            .spline
            .controls()
            .to_vec();
        let curve_point = harness
            .state
            .editor
            .obstacle(id)
            .unwrap()
            .spline
            .evaluate(0.5);
        let start = harness.point(curve_point);
        assert!(harness.state.hit_handle(start, harness.rect).is_none());
        let delta = Point2::new(0.04, -0.02);
        harness.button(start, PointerButton::Primary, true);
        harness.move_to(harness.point(curve_point + delta));
        harness.button(
            harness.point(curve_point + delta),
            PointerButton::Primary,
            false,
        );
        harness.settle();
        let moved = harness
            .state
            .editor
            .obstacle(id)
            .unwrap()
            .spline
            .controls()
            .to_vec();
        assert!(
            moved
                .iter()
                .zip(&before)
                .all(|(moved, before)| (*moved - *before - delta).norm() < 1.0e-6)
        );
        assert_eq!(harness.state.editor.history_len(), (1, 0));

        harness.state.transform_translation = Point2::new(-0.02, 0.01);
        harness.state.transform_rotation_degrees = 30.0;
        harness.state.transform_scale = 0.9;
        harness.state.apply_selection_transform();
        harness.settle();
        assert_eq!(harness.state.editor.history_len(), (2, 0));
        assert_eq!(harness.state.transform_translation, Point2::default());
        assert_eq!(harness.state.transform_rotation_degrees, 0.0);
        assert_eq!(harness.state.transform_scale, 1.0);
    }

    #[test]
    fn c0_isolated_baffle_span_transforms_rigidly_as_one_history_action() {
        let mut harness = Harness::new();
        let id = harness
            .state
            .editor
            .create_internal_boundary(
                OpenCubicSpline::new(
                    vec![
                        Point2::new(-0.8, 0.55),
                        Point2::new(-0.6, 0.72),
                        Point2::new(-0.2, 0.48),
                        Point2::new(0.15, 0.7),
                        Point2::new(0.5, 0.48),
                        Point2::new(0.8, 0.6),
                    ],
                    vec![0.8, 1.1, 0.9],
                )
                .unwrap(),
                BACKGROUND_REGION,
            )
            .unwrap();
        harness
            .state
            .set_span_selection(vec![GeometrySpan::Baffle(id, 1)]);
        assert!(harness.state.transformable_curve_controls().is_none());
        let (loop_breakpoints, baffle_breakpoints) =
            harness.state.exposed_selection_breakpoints().unwrap();
        assert!(loop_breakpoints.is_empty());
        assert_eq!(baffle_breakpoints, [(id, 1), (id, 2)]);
        let isolation_history = harness.state.editor.history_len().0;
        harness.click_text("Isolate selection at C0");
        assert_eq!(harness.state.editor.history_len().0, isolation_history + 1);
        let before = harness
            .state
            .editor
            .internal_boundary(id)
            .unwrap()
            .spline
            .clone();
        let controls = harness.state.transformable_curve_controls().unwrap();
        assert_eq!(controls.len(), 1);
        assert_eq!(controls[0].len(), 4);
        let history = harness.state.editor.history_len().0;
        let delta = Point2::new(0.025, -0.015);
        harness.state.transform_translation = delta;
        harness.state.apply_selection_transform();
        let after = &harness.state.editor.internal_boundary(id).unwrap().spline;
        let bounds = before.span_bounds(1).unwrap();
        for index in 0..=20 {
            let parameter = bounds[0] + (bounds[1] - bounds[0]) * index as f64 / 20.0;
            assert!(
                (after.evaluate(parameter) - before.evaluate(parameter) - delta).norm() < 1.0e-10
            );
        }
        assert!((after.evaluate(0.0) - before.evaluate(0.0)).norm() < 1.0e-12);
        assert!(
            (after.evaluate(after.period()) - before.evaluate(before.period())).norm() < 1.0e-12
        );
        assert_eq!(harness.state.editor.history_len().0, history + 1);
        harness.state.editor.undo();
        assert_eq!(
            harness.state.editor.internal_boundary(id).unwrap().spline,
            before
        );

        let pivot = harness.state.selection_pivot().unwrap();
        let start = harness.state.scale_handle_position(harness.rect, pivot);
        let end = harness.scale_drag_end(pivot, 1.2);
        harness.button(start, PointerButton::Primary, true);
        harness.move_to(end);
        harness.button(end, PointerButton::Primary, false);
        let after = &harness.state.editor.internal_boundary(id).unwrap().spline;
        for index in 0..=20 {
            let parameter = bounds[0] + (bounds[1] - bounds[0]) * index as f64 / 20.0;
            let expected = pivot + (before.evaluate(parameter) - pivot) * 1.2;
            assert!((after.evaluate(parameter) - expected).norm() < 1.0e-6);
        }
        assert_eq!(harness.state.editor.history_len().0, history + 1);
    }

    #[test]
    fn c0_isolated_loop_selection_can_wrap_the_periodic_seam() {
        let mut harness = Harness::new();
        let id = ObstacleId(1);
        harness
            .state
            .editor
            .set_obstacle_continuity(id, 7, 0)
            .unwrap();
        harness
            .state
            .editor
            .set_obstacle_continuity(id, 1, 0)
            .unwrap();
        harness
            .state
            .set_span_selection(vec![GeometrySpan::Loop(id, 7), GeometrySpan::Loop(id, 0)]);
        let groups = harness.state.transformable_curve_controls().unwrap();
        assert_eq!(groups.len(), 1);
        let before = harness.state.editor.obstacle(id).unwrap().spline.clone();
        let untouched = before.evaluate(3.5);
        let history = harness.state.editor.history_len().0;
        let delta = Point2::new(-0.012, 0.018);
        harness.state.transform_translation = delta;
        harness.state.apply_selection_transform();
        let after = &harness.state.editor.obstacle(id).unwrap().spline;
        for parameter in [7.2, 7.8, 0.2, 0.8] {
            assert!(
                (after.evaluate(parameter) - before.evaluate(parameter) - delta).norm() < 1.0e-10
            );
        }
        assert!((after.evaluate(3.5) - untouched).norm() < 1.0e-12);
        assert_eq!(harness.state.editor.history_len().0, history + 1);
    }
    #[test]
    fn both_creation_workflows_and_custom_cancel() {
        let mut h = Harness::new();
        h.state.interaction_mode = InteractionMode::DrawPreset {
            role: CreationRole::Hole,
        };
        h.click(h.point(Point2::new(0.5, 0.4)));
        assert_eq!(h.state.editor.document.draft.obstacles.len(), 2);
        assert_eq!(h.state.interaction_mode, InteractionMode::Select);
        assert_eq!(h.state.editor.history_len().0, 1);
        h.state.interaction_mode = InteractionMode::DrawCustom {
            role: CreationRole::Hole,
        };
        for p in [
            Point2::new(-0.7, -0.4),
            Point2::new(-0.4, -0.4),
            Point2::new(-0.4, -0.7),
            Point2::new(-0.7, -0.7),
        ] {
            h.click(h.point(p));
        }
        assert_eq!(h.state.custom.len(), 4);
        h.key(Key::Enter, Modifiers::NONE);
        assert_eq!(h.state.editor.document.draft.obstacles.len(), 3);
        assert_eq!(h.state.editor.history_len().0, 2);
        h.settle();
        assert_eq!(h.state.editor.acceptance, Acceptance::Valid);
        h.state.interaction_mode = InteractionMode::DrawCustom {
            role: CreationRole::Hole,
        };
        h.click(h.point(Point2::new(0.4, -0.3)));
        h.click(h.point(Point2::new(0.6, -0.3)));
        h.key(Key::Backspace, Modifiers::NONE);
        assert_eq!(h.state.custom.len(), 1);
        h.key(Key::Escape, Modifiers::NONE);
        assert!(h.state.custom.is_empty());
        assert_eq!(h.state.editor.history_len().0, 2);
    }
    #[test]
    fn zoom_is_cursor_centered_and_panel_scroll_does_not_zoom() {
        let mut h = Harness::new();
        let p = h.point(Point2::new(0.4, 0.4));
        h.move_to(p);
        let before = h.state.world(p, h.rect);
        let scale = h.state.scale;
        h.wheel(p);
        assert!(h.state.scale > scale);
        assert!((h.state.world(p, h.rect) - before).norm() < 1e-6);
        for _ in 0..40 {
            h.frame(vec![]);
        }
        let scale = h.state.scale;
        h.wheel(Pos2::new(h.size.x - 80.0, 500.0));
        h.move_to(p);
        for _ in 0..20 {
            h.frame(vec![]);
        }
        assert_eq!(h.state.scale, scale);
        assert_eq!(h.state.editor.history_len(), (0, 0));
        let center = h.state.center;
        h.size = egui::vec2(900.0, 600.0);
        h.frame(vec![]);
        assert_eq!(h.state.center, center);
        let p = Point2::new(-0.7, 0.3);
        assert!((h.state.world(h.point(p), h.rect) - p).norm() < 1e-6);
    }

    #[test]
    fn contextual_shell_exposes_tools_and_add_geometry_popover() {
        let mut h = Harness::new();
        assert!(h.state.logo_texture.is_some());
        assert!(!h.texts.iter().any(|(text, _)| text == "funfern"));
        let undo_center_y = h
            .texts
            .iter()
            .find(|(text, _)| text == "Undo")
            .unwrap()
            .1
            .center()
            .y;
        assert!((undo_center_y - 21.0).abs() < 4.0);
        for label in [
            "Save",
            "Load",
            "Examples",
            "Export",
            "Edit",
            "View",
            "Simulation",
            "Materials",
            "+ Draw",
        ] {
            assert!(
                h.texts.iter().any(|(text, _)| text == label),
                "missing {label}"
            );
        }
        assert!(!h.texts.iter().any(|(text, _)| text == "Revert draft"));
        h.click_text("+ Draw");
        assert!(h.state.add_geometry_open);
        h.frame(vec![]);
        assert!(h.texts.iter().any(|(text, _)| text == "Draw geometry"));
        assert!(h.texts.iter().any(|(text, _)| text == "Circle"));
        assert!(h.texts.iter().any(|(text, _)| text == "Custom"));
        h.click_text("Baffle");
        assert!(matches!(
            h.state.creation_role,
            CreationRole::InternalBoundary
        ));
        assert!(h.texts.iter().any(|(text, _)| text == "Straight"));
        assert!(!h.texts.iter().any(|(text, _)| text == "Circle"));
    }

    #[test]
    fn example_gallery_previews_catalog_and_opening_is_undoable() {
        let mut h = Harness::new();
        h.click_text("Examples");
        assert!(h.state.examples_open);
        h.frame(vec![]);
        assert!(
            h.texts
                .iter()
                .any(|(text, _)| text == "Choose a ready-to-run scene.")
        );
        assert!(
            !h.texts
                .iter()
                .any(|(text, _)| text.contains("undoable document change"))
        );
        for name in ["Starter obstacle", "Double slit", "Material lens"] {
            assert!(
                h.texts.iter().any(|(text, _)| text == name),
                "missing {name}"
            );
        }
        assert_eq!(examples::catalog().len(), 4);

        build_mesh_candidate(&mut h.state);
        commit_mesh_without_gpu(&mut h.state);
        let before = h.state.editor.document.clone();
        h.state.startup_load_checked = true;
        let expected_source = examples::catalog()[1].simulation.source;
        h.state.start_example_load(&examples::catalog()[1]);
        for _ in 0..100 {
            h.state.update_files();
            if h.state.load.is_none() {
                break;
            }
        }
        assert_eq!(h.state.editor.document, examples::catalog()[1].document);
        assert!(h.state.wave_source.enabled);
        assert_eq!(h.state.wave_source.position, expected_source.position);
        assert_eq!(
            h.state.wave_source.frequency_hz,
            expected_source.frequency_hz
        );
        assert!(h.state.wave_source_dirty);
        assert!(h.state.fresh_simulation_requested);
        assert!(h.state.show_boundary_conditions);
        assert_eq!(h.state.editor.history_len(), (1, 0));

        build_mesh_candidate(&mut h.state);
        let candidate = h.state.simulation_candidate.as_ref().unwrap();
        assert!(candidate.fresh);
        assert!(candidate.transfer.is_none());

        h.state.editor.undo();
        assert_eq!(h.state.editor.document, before);
    }

    #[test]
    fn clean_startup_loads_the_first_catalog_example() {
        let mut h = Harness::new();
        h.state.startup_load_checked = true;
        h.state.start_initial_example_load();
        for _ in 0..100 {
            h.state.update_files();
            if h.state.load.is_none() {
                break;
            }
        }
        let first = &examples::catalog()[0];
        assert_eq!(h.state.editor.document, first.document);
        assert_eq!(
            h.state.wave_source.position,
            first.simulation.source.position
        );
        assert_eq!(
            h.state.wave_source.frequency_hz,
            first.simulation.source.frequency_hz
        );
        assert!(h.state.wave_source.enabled);
        assert_eq!(h.state.editor.history_len(), (0, 0));
        assert_eq!(h.state.editor.document.probes.len(), 1);
    }

    #[test]
    fn loading_a_scene_restores_its_continuous_source() {
        let mut h = Harness::new();
        h.state.startup_load_checked = true;
        let mut document = h.state.editor.document.clone();
        document.source = SourceSettings {
            enabled: true,
            position: Point2::new(0.38, -0.26),
            amplitude: 27.0,
            width: 0.04,
            frequency_hz: 3.75,
            region: BACKGROUND_REGION,
        };
        let bytes = persistence::save(&document).unwrap();
        h.state.start_load(
            persistence::parse(bytes.as_bytes()).unwrap(),
            LoadMode::Replace,
            "",
        );
        for _ in 0..100 {
            h.state.update_files();
            if h.state.load.is_none() {
                break;
            }
        }
        assert_eq!(h.state.editor.document.source, document.source);
        assert_eq!(h.state.wave_source, document.source);
        assert!(h.state.wave_source_dirty);
    }

    #[test]
    fn top_bar_progressively_compacts_file_and_panel_controls() {
        let mut h = Harness::new();
        h.state.inspector_panel = None;
        h.size = egui::vec2(760.0, 800.0);
        h.frame(vec![]);
        h.frame(vec![]);
        assert!(h.texts.iter().any(|(text, _)| text == "File"));
        assert!(h.texts.iter().any(|(text, _)| text == "Panels"));
        assert!(h.texts.iter().any(|(text, _)| text == "+ Draw"));
        assert!(!h.texts.iter().any(|(text, _)| text == "Save"));
        assert!(!h.texts.iter().any(|(text, _)| text == "Edit"));
        assert!(h.rect.top() < 50.0, "viewport={:?}", h.rect);
        let draw = h.texts.iter().find(|(text, _)| text == "+ Draw").unwrap().1;
        let pause = h.texts.iter().find(|(text, _)| text == "Pause").unwrap().1;
        assert!(draw.right() < pause.left(), "draw={draw:?} pause={pause:?}");

        h.size = egui::vec2(1000.0, 800.0);
        h.frame(vec![]);
        assert!(h.texts.iter().any(|(text, _)| text == "File"));
        assert!(!h.texts.iter().any(|(text, _)| text == "Panels"));
        for label in ["Edit", "View", "Simulation", "Materials", "Probes"] {
            assert!(h.texts.iter().any(|(text, _)| text == label));
        }
        let draw = h.texts.iter().find(|(text, _)| text == "+ Draw").unwrap().1;
        let pause = h.texts.iter().find(|(text, _)| text == "Pause").unwrap().1;
        assert!(draw.right() < pause.left(), "draw={draw:?} pause={pause:?}");

        h.size = egui::vec2(1200.0, 800.0);
        h.frame(vec![]);
        assert!(!h.texts.iter().any(|(text, _)| text == "File"));
        for label in ["Save", "Load", "Examples", "Export"] {
            assert!(h.texts.iter().any(|(text, _)| text == label));
        }
    }

    #[test]
    fn inspector_panel_switches_can_hide_and_restore_the_right_panel() {
        let mut h = Harness::new();
        h.click_text("Edit");
        assert_eq!(h.state.inspector_panel, None);
        h.click_text("Edit");
        assert_eq!(h.state.inspector_panel, Some(InspectorPanel::Edit));
    }

    #[test]
    fn inspector_panel_switches_show_their_content() {
        let mut h = Harness::new();
        h.click_text("View");
        assert_eq!(h.state.inspector_panel, Some(InspectorPanel::View));
        h.click_text("Simulation");
        assert_eq!(h.state.inspector_panel, Some(InspectorPanel::Simulation));
        h.click_text("Materials");
        assert_eq!(h.state.inspector_panel, Some(InspectorPanel::Materials));
        h.click_text("Probes");
        assert_eq!(h.state.inspector_panel, Some(InspectorPanel::Probes));
    }

    #[test]
    fn view_panel_toggles_each_probe_kind_independently() {
        let mut h = Harness::new();
        h.click_text("View");
        for label in [
            "Point probes",
            "Line probes",
            "Boundary probes",
            "Area probes",
        ] {
            assert!(h.texts.iter().any(|(text, _)| text == label));
        }
        h.click_text("Point probes");
        assert!(!h.state.show_point_probes);
        assert!(h.state.show_line_probes);
        assert!(h.state.show_boundary_probes);
        assert!(h.state.show_area_probes);
        h.click_text("Area probes");
        assert!(!h.state.show_point_probes);
        assert!(!h.state.show_area_probes);
        h.click_text("Restore view defaults");
        assert!(h.state.show_point_probes);
        assert!(h.state.show_line_probes);
        assert!(h.state.show_boundary_probes);
        assert!(h.state.show_area_probes);
    }

    #[test]
    fn point_probe_placement_and_drag_are_persistent_and_undoable() {
        let mut h = Harness::new();
        h.click_text("Probes");
        h.click_text("+ Point probe");
        assert_eq!(h.state.interaction_mode, InteractionMode::PlaceProbe);
        let position = Point2::new(0.45, -0.35);
        h.click(h.point(position));
        assert_eq!(h.state.editor.document.probes.len(), 1);
        assert_eq!(h.state.interaction_mode, InteractionMode::PlaceProbe);
        assert_eq!(h.state.editor.history_len(), (1, 0));
        let id = h.state.editor.document.probes[0].id;
        h.state.probe_windows.clear();
        h.state.interaction_mode = InteractionMode::Select;
        let target = Point2::new(0.55, -0.2);
        h.drag_with_modifiers(h.point(position), h.point(target), Modifiers::NONE);
        assert_eq!(h.state.selected_probe, Some(id));
        let ProbeTarget::Point(actual) = h.state.editor.document.probes[0].target else {
            panic!("expected point probe")
        };
        assert!((actual - target).norm() < 1.0e-6);
        assert_eq!(h.state.editor.history_len(), (2, 0));
        h.state.editor.undo();
        let ProbeTarget::Point(actual) = h.state.editor.document.probes[0].target else {
            panic!("expected point probe")
        };
        assert!((actual - position).norm() < 1.0e-6);
    }

    #[test]
    fn line_probe_placement_and_rigid_drag_are_undoable() {
        let mut h = Harness::new();
        h.click_text("Probes");
        h.click_text("+ Line probe");
        assert_eq!(h.state.interaction_mode, InteractionMode::PlaceSegmentProbe);
        let start = Point2::new(-0.45, 0.45);
        let end = Point2::new(0.45, 0.45);
        h.click(h.point(start));
        assert!(
            h.state
                .segment_probe_start
                .is_some_and(|actual| (actual - start).norm() < 1.0e-6)
        );
        h.click(h.point(end));
        assert_eq!(h.state.editor.document.probes.len(), 1);
        assert_eq!(h.state.interaction_mode, InteractionMode::PlaceSegmentProbe);
        assert_eq!(h.state.editor.history_len(), (1, 0));

        h.state.probe_windows.clear();
        h.click(h.point(Point2::new(-0.2, -0.5)));
        h.key(Key::Escape, Modifiers::NONE);
        assert_eq!(h.state.interaction_mode, InteractionMode::PlaceSegmentProbe);
        assert!(h.state.segment_probe_start.is_none());
        h.key(Key::Escape, Modifiers::NONE);
        assert_eq!(h.state.interaction_mode, InteractionMode::Select);

        let delta = Point2::new(0.1, -0.15);
        h.drag_with_modifiers(
            h.point((start + end) * 0.5),
            h.point((start + end) * 0.5 + delta),
            Modifiers::NONE,
        );
        let ProbeTarget::Segment {
            start: moved_start,
            end: moved_end,
            preset,
        } = h.state.editor.document.probes[0].target
        else {
            panic!("expected line probe")
        };
        assert!((moved_start - start - delta).norm() < 1.0e-6);
        assert!((moved_end - end - delta).norm() < 1.0e-6);
        assert_eq!(preset, ProbeSamplingPreset::Medium);
        assert_eq!(h.state.editor.history_len(), (2, 0));
        h.state.editor.undo();
        assert!(matches!(
            h.state.editor.document.probes[0].target,
            ProbeTarget::Segment { start: actual, .. } if (actual - start).norm() < 1.0e-6
        ));
    }

    #[test]
    fn disk_probe_placement_body_drag_and_radius_drag_are_undoable() {
        let mut h = Harness::new();
        h.click_text("Probes");
        h.click_text("+ Area probe");
        h.click_text("Disk");
        assert_eq!(h.state.interaction_mode, InteractionMode::PlaceAreaDisk);
        let center = Point2::new(-0.5, -0.5);
        let radius_point = center + Point2::new(0.15, 0.0);
        h.click(h.point(center));
        h.click(h.point(radius_point));
        assert_eq!(h.state.editor.document.probes.len(), 1);
        let id = h.state.editor.document.probes[0].id;
        assert!(matches!(
            h.state.editor.document.probes[0].target,
            ProbeTarget::AreaDisk { center: actual, radius }
                if (actual - center).norm() < 1.0e-6 && (radius - 0.15).abs() < 1.0e-6
        ));

        h.state.interaction_mode = InteractionMode::Select;
        h.state.probe_windows.clear();
        let moved = Point2::new(-0.3, -0.42);
        h.drag_with_modifiers(h.point(center), h.point(moved), Modifiers::NONE);
        assert_eq!(h.state.selected_probe, Some(id));
        assert!(matches!(
            h.state.editor.document.probes[0].target,
            ProbeTarget::AreaDisk { center: actual, radius }
                if (actual - moved).norm() < 1.0e-6 && (radius - 0.15).abs() < 1.0e-6
        ));

        let new_radius = 0.24;
        h.drag_with_modifiers(
            h.point(moved + Point2::new(0.15, 0.0)),
            h.point(moved + Point2::new(new_radius, 0.0)),
            Modifiers::NONE,
        );
        assert!(matches!(
            h.state.editor.document.probes[0].target,
            ProbeTarget::AreaDisk { center: actual, radius }
                if (actual - moved).norm() < 1.0e-6 && (radius - new_radius).abs() < 1.0e-6
        ));
        assert_eq!(h.state.editor.history_len(), (3, 0));
        h.state.editor.undo();
        assert!(matches!(
            h.state.editor.document.probes[0].target,
            ProbeTarget::AreaDisk { radius, .. } if (radius - 0.15).abs() < 1.0e-6
        ));
    }

    #[test]
    fn subdomain_probe_tool_creates_the_region_under_the_pointer() {
        let mut h = Harness::new();
        build_mesh_candidate(&mut h.state);
        commit_mesh_without_gpu(&mut h.state);
        h.click_text("Probes");
        h.click_text("+ Area probe");
        h.click_text("Subdomain");
        assert_eq!(h.state.interaction_mode, InteractionMode::PlaceAreaRegion);
        h.click(h.point(Point2::new(0.8, 0.8)));
        assert!(matches!(
            h.state.editor.document.probes[0].target,
            ProbeTarget::AreaRegion { region } if region == BACKGROUND_REGION
        ));
        assert_eq!(h.state.interaction_mode, InteractionMode::PlaceAreaRegion);
    }

    #[test]
    fn area_readout_defaults_to_mean_field_and_total_energy() {
        let view = ProbeViewState::new(10.0);
        assert!(view.area_mean_field);
        assert!(!view.area_rms_field);
        assert!(!view.area_mean_energy);
        assert!(view.area_total_energy);
    }

    #[test]
    fn line_probe_integrals_use_only_contiguous_valid_intervals() {
        let frame = CurveProbeRecord {
            probe_id: 1,
            time: 0.5,
            displacement: vec![0.0, 1.0, f32::NAN, 2.0, 4.0],
            energy_density: vec![2.0, 4.0, f32::NAN, 8.0, 10.0],
            normal_flux: vec![1.0, 3.0, f32::NAN, -2.0, 2.0],
        };
        let (field, field_coverage) =
            Playground::curve_probe_integral(&frame, 2.0, LineProbeQuantity::Field, false);
        let (power, coverage) =
            Playground::curve_probe_integral(&frame, 2.0, LineProbeQuantity::Flux, false);
        let (energy, energy_coverage) =
            Playground::curve_probe_integral(&frame, 2.0, LineProbeQuantity::Energy, false);
        assert!((field - 1.75).abs() < 1.0e-12);
        assert!((power - 1.0).abs() < 1.0e-12);
        assert!((energy - 6.0).abs() < 1.0e-12);
        assert!((field_coverage - 0.5).abs() < 1.0e-12);
        assert!((coverage - 0.5).abs() < 1.0e-12);
        assert!((energy_coverage - 0.5).abs() < 1.0e-12);
    }

    #[test]
    fn line_waterfall_pans_on_its_vertical_time_axis() {
        let mut h = Harness::new();
        let id = h
            .state
            .editor
            .create_segment_probe(Point2::new(-0.5, 0.5), Point2::new(0.5, 0.5))
            .unwrap();
        h.state.probe_windows.insert(id);
        h.state.curve_probe_traces.insert(
            id,
            CurveProbeTrace {
                frames: (0..=100)
                    .map(|index| CurveProbeRecord {
                        probe_id: id.0,
                        time: index as f64 * 0.1,
                        displacement: vec![index as f32; 64],
                        energy_density: vec![index as f32; 64],
                        normal_flux: vec![index as f32; 64],
                    })
                    .collect(),
                accept_after: 0.0,
            },
        );
        h.frame(vec![]);
        h.frame(vec![]);
        let label = h
            .texts
            .iter()
            .find(|(text, _)| text == "Field waterfall")
            .unwrap_or_else(|| panic!("texts: {:?}", h.texts))
            .1;
        let start = egui::pos2(label.left() + 180.0, label.bottom() + 80.0);
        h.frame(vec![
            Event::PointerMoved(start),
            Event::PointerButton {
                pos: start,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ]);
        let horizontal = start + egui::vec2(60.0, 0.0);
        h.frame(vec![Event::PointerMoved(horizontal)]);
        assert!((h.state.probe_views[&id].end_time - 10.0).abs() < 1.0e-12);

        let upward = horizontal + egui::vec2(0.0, -60.0);
        h.frame(vec![Event::PointerMoved(upward)]);
        assert!(h.state.probe_views[&id].end_time < 10.0);
        h.frame(vec![Event::PointerButton {
            pos: upward,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]);
    }

    #[test]
    fn line_probe_readout_defaults_to_four_of_nine_plots() {
        let view = ProbeViewState::new(10.0);
        assert_eq!(
            view.line_plots.iter().filter(|enabled| **enabled).count(),
            4
        );
        assert!(view.line_plots[LineProbeQuantity::Field.offset()]);
        assert!(
            view.line_plots
                [LineProbeQuantity::Field.offset() + LineProbeRepresentation::Waterfall.offset()]
        );
        assert!(
            view.line_plots
                [LineProbeQuantity::Flux.offset() + LineProbeRepresentation::Integral.offset()]
        );
        assert!(
            view.line_plots
                [LineProbeQuantity::Energy.offset() + LineProbeRepresentation::Integral.offset()]
        );
    }

    #[test]
    fn boundary_probe_selection_requires_one_contiguous_curve_run() {
        let mut state = Playground::default();
        state.set_span_selection(vec![
            GeometrySpan::Loop(ObstacleId(1), 7),
            GeometrySpan::Loop(ObstacleId(1), 0),
            GeometrySpan::Loop(ObstacleId(1), 1),
        ]);
        let target = state.boundary_probe_target_from_selection().unwrap();
        assert_eq!(target.feature, BoundaryProbeFeature::Loop(ObstacleId(1)));
        assert_eq!(target.start_span, 7);
        assert_eq!(target.span_count, 3);
        assert_eq!(target.side, BoundaryProbeSide::Domain);

        state.set_span_selection(vec![
            GeometrySpan::Loop(ObstacleId(1), 0),
            GeometrySpan::Loop(ObstacleId(1), 2),
        ]);
        assert!(state.boundary_probe_target_from_selection().is_none());
    }

    #[test]
    fn probes_panel_creates_boundary_probe_from_selected_spans() {
        let mut h = Harness::new();
        h.state.inspector_panel = Some(InspectorPanel::Probes);
        h.state.set_span_selection(vec![
            GeometrySpan::Loop(ObstacleId(1), 7),
            GeometrySpan::Loop(ObstacleId(1), 0),
        ]);
        h.frame(vec![]);
        h.click_text("+ From selected spans");
        assert_eq!(h.state.editor.document.probes.len(), 1);
        let ProbeTarget::Boundary(target) = h.state.editor.document.probes[0].target else {
            panic!("expected boundary probe")
        };
        assert_eq!(target.spans(8), vec![7, 0]);
        assert!(
            h.state
                .probe_windows
                .contains(&h.state.editor.document.probes[0].id)
        );
    }

    #[test]
    fn closed_curve_integral_includes_the_periodic_seam() {
        let frame = CurveProbeRecord {
            probe_id: 1,
            time: 0.0,
            displacement: vec![1.0, 1.0, 1.0, 1.0],
            energy_density: vec![0.0; 4],
            normal_flux: vec![0.0; 4],
        };
        let (integral, coverage) =
            Playground::curve_probe_integral(&frame, 2.5, LineProbeQuantity::Field, true);
        assert!((integral - 2.5).abs() < 1.0e-12);
        assert_eq!(coverage, 1.0);
    }

    #[test]
    fn line_probe_plot_picker_stays_open_for_multiple_toggles() {
        let mut h = Harness::new();
        let id = h
            .state
            .editor
            .create_segment_probe(Point2::new(-0.5, 0.5), Point2::new(0.5, 0.5))
            .unwrap();
        h.state.probe_windows.insert(id);
        h.frame(vec![]);
        h.click_text("Plots (4)");
        h.frame(vec![]);

        let checkbox_position = |texts: &[(String, Rect)], row: &str, column: &str| {
            let row = texts
                .iter()
                .find(|(text, _)| text == row)
                .unwrap_or_else(|| panic!("missing picker row in {texts:?}"))
                .1;
            let column = texts
                .iter()
                .find(|(text, _)| text == column)
                .unwrap_or_else(|| panic!("missing picker column in {texts:?}"))
                .1;
            egui::pos2(column.left() + 8.0, row.center().y)
        };

        let field_integral = checkbox_position(&h.texts, "Field", "∫ vs t");
        h.click(field_integral);
        assert_eq!(
            h.state.probe_views[&id]
                .line_plots
                .iter()
                .filter(|enabled| **enabled)
                .count(),
            5
        );
        assert!(h.texts.iter().any(|(text, _)| text == "Waterfall gain"));

        let flux_waterfall = checkbox_position(&h.texts, "Normal flux", "Waterfall");
        h.click(flux_waterfall);
        assert_eq!(
            h.state.probe_views[&id]
                .line_plots
                .iter()
                .filter(|enabled| **enabled)
                .count(),
            6
        );
        assert!(h.texts.iter().any(|(text, _)| text == "Waterfall gain"));

        h.key(Key::Escape, Modifiers::NONE);
        assert!(!h.texts.iter().any(|(text, _)| text == "Waterfall gain"));
    }

    #[test]
    fn double_clicking_a_probe_marker_opens_its_readout() {
        let mut h = Harness::new();
        let position = Point2::new(0.45, -0.35);
        let id = h.state.editor.create_point_probe(position).unwrap();
        h.state.interaction_mode = InteractionMode::Select;
        h.state.probe_windows.clear();
        h.frame(vec![]);

        let marker = h.point(position);
        h.click(marker);
        assert!(!h.state.probe_windows.contains(&id));
        h.click(marker);
        assert!(h.state.probe_windows.contains(&id));
        assert_eq!(h.state.selected_probe, Some(id));
    }

    #[test]
    fn double_clicking_the_far_field_contour_opens_its_readout() {
        let mut h = Harness::new();
        h.state
            .editor
            .set_far_field(FarFieldSettings {
                enabled: true,
                inset: 0.12,
            })
            .unwrap();
        h.state.far_field_open = false;
        h.frame(vec![]);

        let contour = h.point(Point2::new(0.0, -0.88));
        h.click(contour);
        assert!(!h.state.far_field_open);
        h.click(contour);
        assert!(h.state.far_field_open);
    }

    #[test]
    fn far_field_average_follows_the_visible_time_window() {
        let frames = [(0.0, 100.0), (1.0, 1.0), (2.0, 3.0), (3.0, 5.0)]
            .into_iter()
            .map(|(time, intensity)| FarFieldRecord {
                time,
                amplitude: vec![0.0; FAR_FIELD_DIRECTIONS],
                intensity: vec![intensity; FAR_FIELD_DIRECTIONS],
            })
            .collect::<Vec<_>>();
        let times = frames
            .iter()
            .map(|frame| PointProbeRecord {
                probe_id: 0,
                time: frame.time,
                displacement: 0.0,
                velocity: 0.0,
                energy_density: 0.0,
            })
            .collect::<Vec<_>>();
        let mut view = ProbeViewState::new(2.0);
        view.live = false;
        view.end_time = 3.0;
        view.span = 2.0;
        let average = Playground::far_field_average(&frames, &times, &view);
        assert_eq!(average, vec![3.0; FAR_FIELD_DIRECTIONS]);
    }

    #[test]
    fn clearing_a_probe_does_not_reimport_the_gpu_ring() {
        let mut state = Playground::default();
        let id = state
            .editor
            .create_point_probe(Point2::new(0.4, 0.4))
            .unwrap();
        state.probe_compiled = Some(CompiledProbeState {
            generation: 3,
            revision: 5,
            curve_revision: 0,
            area_revision: 0,
            far_field_revision: 0,
            mesh_revision: 1,
            probes: state.editor.document.probes.clone(),
            sample_rate: 120.0,
            time_step: 0.01,
            far_field: FarFieldSettings::default(),
            far_field_wave_speed: None,
        });
        let mut display = ProbeDisplay {
            generation: 3,
            revision: 5,
            records: vec![PointProbeRecord {
                probe_id: id.0,
                time: 0.5,
                displacement: 1.0,
                velocity: 2.0,
                energy_density: 3.0,
            }],
            readbacks: 1,
        };
        state.ingest_probe_samples(&display);
        assert_eq!(state.probe_traces[&id].samples.len(), 1);
        state.clear_probe_trace(id);
        display.readbacks += 1;
        state.ingest_probe_samples(&display);
        assert!(state.probe_traces[&id].samples.is_empty());
    }

    #[test]
    fn far_field_contour_is_counterclockwise_with_outward_normals() {
        let inset = 0.12;
        let half_extent = 1.0 - inset;
        let contour = Playground::far_field_contour(inset);
        assert_eq!(contour.len(), FAR_FIELD_CONTOUR_POINTS);
        assert_eq!(contour[0].1, Point2::new(0.0, -1.0));
        assert_eq!(
            contour[FAR_FIELD_CONTOUR_POINTS / 4].1,
            Point2::new(1.0, 0.0)
        );
        assert_eq!(
            contour[FAR_FIELD_CONTOUR_POINTS / 2].1,
            Point2::new(0.0, 1.0)
        );
        assert_eq!(
            contour[3 * FAR_FIELD_CONTOUR_POINTS / 4].1,
            Point2::new(-1.0, 0.0)
        );
        assert!(contour[0].0.x < contour[1].0.x);
        assert!(
            contour[FAR_FIELD_CONTOUR_POINTS / 4].0.y
                < contour[FAR_FIELD_CONTOUR_POINTS / 4 + 1].0.y
        );
        assert!(contour.iter().all(|(point, _)| {
            (point.x.abs() - half_extent).abs() < 1.0e-12
                || (point.y.abs() - half_extent).abs() < 1.0e-12
        }));
        let spacing = 8.0 * half_extent / FAR_FIELD_CONTOUR_POINTS as f64;
        assert!((spacing - (contour[1].0 - contour[0].0).norm()).abs() < 1.0e-12);
    }

    #[test]
    fn far_field_compile_checks_enclosure_and_lossless_background() {
        let scene = Scene::default();
        let mesh = mesh_scene(
            &scene,
            1,
            MeshingOptions {
                target_edge_length: 0.18,
                minimum_angle_degrees: 10.0,
                ..Default::default()
            },
        )
        .unwrap();
        let operator = QuadraticWaveOperator::assemble_scene_with_boundaries(
            &mesh,
            &scene,
            scene.outer_boundaries,
        )
        .unwrap();
        let settings = FarFieldSettings {
            enabled: true,
            inset: 0.12,
        };
        let input = Playground::compile_far_field(&mesh, &operator, &scene, settings).unwrap();
        assert_eq!(input.samples.len(), FAR_FIELD_CONTOUR_POINTS);
        assert!((input.wave_speed - 1.0).abs() < 1.0e-12);

        let mut damped = scene.clone();
        damped.materials[0].damping = 0.1;
        assert!(
            Playground::compile_far_field(&mesh, &operator, &damped, settings)
                .unwrap_err()
                .contains("lossless")
        );

        let mut outside = scene.clone();
        outside.obstacles.push(Obstacle::hole(
            ObstacleId(9),
            PeriodicCubicSpline::rounded(Point2::new(0.9, 0.0), 0.03),
        ));
        assert!(
            Playground::compile_far_field(&mesh, &operator, &outside, settings)
                .unwrap_err()
                .contains("enclose")
        );
    }

    #[test]
    fn far_field_history_accepts_a_continuous_new_solver_generation() {
        let mut state = Playground::default();
        let compile = |generation, revision| CompiledProbeState {
            generation,
            revision: 0,
            curve_revision: 0,
            area_revision: 0,
            far_field_revision: revision,
            mesh_revision: generation,
            probes: vec![],
            sample_rate: 120.0,
            time_step: 0.01,
            far_field: FarFieldSettings {
                enabled: true,
                ..Default::default()
            },
            far_field_wave_speed: Some(1.0),
        };
        let display = |generation, revision, times: &[f64], readbacks| FarFieldDisplay {
            generation,
            revision,
            records: times
                .iter()
                .map(|time| FarFieldRecord {
                    time: *time,
                    amplitude: vec![1.0; FAR_FIELD_DIRECTIONS],
                    intensity: vec![1.0; FAR_FIELD_DIRECTIONS],
                })
                .collect(),
            readbacks,
        };
        state.probe_compiled = Some(compile(3, 5));
        state.ingest_far_field_samples(&display(3, 5, &[0.9, 1.0], 1));
        state.probe_compiled = Some(compile(4, 7));
        state.far_field_display_readback = 0;
        state.ingest_far_field_samples(&display(4, 7, &[1.1, 1.2], 1));
        assert_eq!(
            state
                .far_field_trace
                .frames
                .iter()
                .map(|frame| frame.time)
                .collect::<Vec<_>>(),
            vec![0.9, 1.0, 1.1, 1.2]
        );
    }

    #[test]
    fn probe_history_accepts_a_continuous_new_solver_generation() {
        let mut state = Playground::default();
        let id = state
            .editor
            .create_point_probe(Point2::new(0.4, 0.4))
            .unwrap();
        let compile = |generation, revision, probes: Vec<ProbeDefinition>| CompiledProbeState {
            generation,
            revision,
            curve_revision: 0,
            area_revision: 0,
            far_field_revision: 0,
            mesh_revision: generation,
            probes,
            sample_rate: 120.0,
            time_step: 0.01,
            far_field: FarFieldSettings::default(),
            far_field_wave_speed: None,
        };
        let records = |generation, revision, times: &[f64], readbacks| ProbeDisplay {
            generation,
            revision,
            records: times
                .iter()
                .map(|time| PointProbeRecord {
                    probe_id: id.0,
                    time: *time,
                    displacement: 1.0,
                    velocity: 2.0,
                    energy_density: 3.0,
                })
                .collect(),
            readbacks,
        };

        state.probe_compiled = Some(compile(3, 5, state.editor.document.probes.clone()));
        state.ingest_probe_samples(&records(3, 5, &[0.9, 1.0], 1));
        state.probe_compiled = Some(compile(4, 7, state.editor.document.probes.clone()));
        state.probe_display_readback = 0;
        state.ingest_probe_samples(&records(4, 7, &[1.1, 1.2], 1));

        let times = state.probe_traces[&id]
            .samples
            .iter()
            .map(|sample| sample.time)
            .collect::<Vec<_>>();
        assert_eq!(times, vec![0.9, 1.0, 1.1, 1.2]);
    }

    #[test]
    fn area_probe_history_stays_continuous_across_solver_generations() {
        let mut state = Playground::default();
        let id = state
            .editor
            .create_area_disk_probe(Point2::new(0.4, 0.4), 0.15)
            .unwrap();
        let compile =
            |generation, area_revision, probes: Vec<ProbeDefinition>| CompiledProbeState {
                generation,
                revision: 0,
                curve_revision: 0,
                area_revision,
                far_field_revision: 0,
                mesh_revision: generation,
                probes,
                sample_rate: 120.0,
                time_step: 0.01,
                far_field: FarFieldSettings::default(),
                far_field_wave_speed: None,
            };
        let records = |generation, revision, times: &[f64], readbacks| AreaProbeDisplay {
            generation,
            revision,
            records: times
                .iter()
                .map(|time| AreaProbeRecord {
                    probe_id: id.0,
                    time: *time,
                    mean_displacement: 1.0,
                    rms_displacement: 2.0,
                    mean_energy_density: 3.0,
                    total_energy: 4.0,
                    covered_area: 0.1,
                    coverage: 1.0,
                })
                .collect(),
            readbacks,
        };

        state.probe_compiled = Some(compile(3, 5, state.editor.document.probes.clone()));
        state.ingest_area_probe_samples(&records(3, 5, &[0.9, 1.0], 1));
        state.probe_compiled = Some(compile(4, 7, state.editor.document.probes.clone()));
        state.area_probe_display_readback = 0;
        state.ingest_area_probe_samples(&records(4, 7, &[1.1, 1.2], 1));

        let times = state.area_probe_traces[&id]
            .samples
            .iter()
            .map(|sample| sample.time)
            .collect::<Vec<_>>();
        assert_eq!(times, vec![0.9, 1.0, 1.1, 1.2]);
    }

    #[test]
    fn probe_time_window_fills_available_history_and_clamps_panning() {
        let samples = [0.0, 0.5, 1.0]
            .into_iter()
            .map(|time| PointProbeRecord {
                probe_id: 1,
                time,
                displacement: 0.0,
                velocity: 0.0,
                energy_density: 0.0,
            })
            .collect::<Vec<_>>();
        let mut view = ProbeViewState::new(10.0);
        assert_eq!(
            Playground::probe_time_window(&samples, &mut view),
            Some((0.0, 1.0))
        );

        view.live = false;
        view.span = 0.4;
        view.end_time = -20.0;
        assert_eq!(
            Playground::probe_time_window(&samples, &mut view),
            Some((0.0, 0.4))
        );
        view.end_time = 20.0;
        assert_eq!(
            Playground::probe_time_window(&samples, &mut view),
            Some((0.6, 1.0))
        );
    }

    #[test]
    fn dragging_a_probe_plot_pans_the_shared_time_window() {
        let mut h = Harness::new();
        let id = h
            .state
            .editor
            .create_point_probe(Point2::new(0.4, 0.4))
            .unwrap();
        h.state.probe_windows.insert(id);
        h.state.probe_traces.insert(
            id,
            ProbeTrace {
                samples: (0..=100)
                    .map(|index| PointProbeRecord {
                        probe_id: id.0,
                        time: index as f64 * 0.1,
                        displacement: index as f64,
                        velocity: 0.0,
                        energy_density: 0.0,
                    })
                    .collect(),
                accept_after: 0.0,
            },
        );
        h.frame(vec![]);
        h.frame(vec![]);
        let field_label = h
            .texts
            .iter()
            .filter(|(text, _)| text == "Field")
            .max_by(|(_, left), (_, right)| left.top().total_cmp(&right.top()))
            .unwrap_or_else(|| panic!("texts: {:?}", h.texts))
            .1;
        let start = egui::pos2(field_label.left() + 180.0, field_label.bottom() + 46.0);
        h.frame(vec![
            Event::PointerMoved(start),
            Event::PointerButton {
                pos: start,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ]);
        let halfway = start + egui::vec2(50.0, 0.0);
        h.frame(vec![Event::PointerMoved(halfway)]);
        let halfway_end = h.state.probe_views[&id].end_time;
        assert!(halfway_end < 10.0);

        // A held pointer commonly has frames with no movement. The old code
        // interpreted the zero per-frame delta as zero total displacement and
        // sprang back to the starting time here.
        h.frame(vec![Event::PointerMoved(halfway)]);
        assert_eq!(h.state.probe_views[&id].end_time, halfway_end);

        let end = start + egui::vec2(100.0, 0.0);
        h.frame(vec![Event::PointerMoved(end)]);
        h.frame(vec![Event::PointerButton {
            pos: end,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]);

        let view = &h.state.probe_views[&id];
        assert!(!view.live);
        assert!(view.end_time < 10.0);
        assert!((view.end_time - 9.5).abs() < 0.15, "{}", view.end_time);
        let panned_end = view.end_time;
        h.state
            .probe_traces
            .get_mut(&id)
            .unwrap()
            .samples
            .push_back(PointProbeRecord {
                probe_id: id.0,
                time: 10.1,
                displacement: 101.0,
                velocity: 0.0,
                energy_density: 0.0,
            });
        h.frame(vec![]);
        assert!((h.state.probe_views[&id].end_time - panned_end).abs() < 1.0e-12);

        h.frame(vec![
            Event::PointerMoved(start),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -1_000.0),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
            },
        ]);
        assert!(h.state.probe_views[&id].live);
        assert_eq!(h.state.probe_views[&id].end_time, 10.1);
    }

    #[test]
    fn probe_readout_window_keeps_its_dragged_position() {
        let mut h = Harness::new();
        let id = h
            .state
            .editor
            .create_point_probe(Point2::new(0.4, 0.4))
            .unwrap();
        let title = format!("{} · point probe", h.state.editor.document.probes[0].name);
        h.state.probe_windows.insert(id);
        h.frame(vec![]);
        h.frame(vec![]);
        let before = h
            .texts
            .iter()
            .find(|(text, _)| text == &title)
            .unwrap_or_else(|| panic!("texts: {:?}", h.texts))
            .1
            .center();
        h.drag_with_modifiers(before, before + egui::vec2(100.0, 50.0), Modifiers::NONE);
        h.frame(vec![]);
        let after = h
            .texts
            .iter()
            .find(|(text, _)| text == &title)
            .unwrap()
            .1
            .center();
        assert!(
            (after.x - before.x - 100.0).abs() < 2.0,
            "{before:?} {after:?}"
        );
        assert!(
            (after.y - before.y - 50.0).abs() < 2.0,
            "{before:?} {after:?}"
        );
    }

    #[test]
    fn inspector_switches_preserve_visible_placement_modes() {
        let mut h = Harness::new();
        h.state.interaction_mode = InteractionMode::PlacePulse;
        h.click_text("View");
        assert_eq!(h.state.inspector_panel, Some(InspectorPanel::View));
        assert_eq!(h.state.interaction_mode, InteractionMode::PlacePulse);
        assert!(h.texts.iter().any(|(text, _)| text == "Placing pulse"));
    }

    #[test]
    fn mixed_curve_span_selection_hides_whole_feature_actions() {
        let mut h = Harness::new();
        let second = h
            .state
            .editor
            .create_loop(
                PeriodicCubicSpline::rounded(Point2::new(0.55, 0.55), 0.1),
                LoopRole::Hole {
                    exterior: BACKGROUND_REGION,
                },
            )
            .unwrap();
        h.state.set_span_selection(vec![
            GeometrySpan::Loop(ObstacleId(1), 0),
            GeometrySpan::Loop(second, 0),
        ]);
        h.frame(vec![]);
        assert_eq!(h.state.focused_feature, None);
        assert!(
            h.texts
                .iter()
                .any(|(text, _)| text == "2 spans across 2 boundaries")
        );
        assert!(!h.texts.iter().any(|(text, _)| text == "Loop role"));
        assert!(!h.texts.iter().any(|(text, _)| text == "Delete loop"));
    }

    #[test]
    fn view_exposes_boundary_assignments_and_closed_wall_is_not_offered() {
        let mut h = Harness::new();
        h.click_text("View");
        h.click_text("Boundary conditions");
        assert!(h.state.show_boundary_conditions);
        assert!(EDITABLE_LOOP_KINDS.contains(&(LoopKind::Hole, "Hole")));
        assert!(EDITABLE_LOOP_KINDS.contains(&(LoopKind::MaterialInterface, "Material interface")));
        assert!(
            !EDITABLE_LOOP_KINDS
                .iter()
                .any(|(kind, _)| *kind == LoopKind::Wall)
        );
    }

    #[test]
    fn wave_starts_running_and_playback_controls_live_in_top_bar() {
        let h = Harness::new();
        assert!(h.state.wave_running);
        assert!(h.texts.iter().any(|(text, _)| text == "Pause"));
        assert!(h.texts.iter().any(|(text, _)| text == "Step"));
        assert!(h.texts.iter().any(|(text, _)| text == "Reset"));
    }

    #[test]
    fn handoff_status_is_shown_in_status_bar() {
        let mut h = Harness::new();
        build_mesh_candidate(&mut h.state);
        h.frame(vec![]);
        assert!(
            h.texts
                .iter()
                .any(|(text, _)| text.starts_with("Preparing solver handoff"))
        );
        assert!(
            !h.texts
                .iter()
                .any(|(text, _)| text.starts_with("Committing candidate"))
        );
    }

    #[test]
    fn transient_commit_message_is_rendered_in_status_bar() {
        let mut h = Harness::new();
        h.state.notify("Simulation mesh committed");
        h.frame(vec![]);
        let (_, rect) = h
            .texts
            .iter()
            .find(|(text, _)| text == "Simulation mesh committed")
            .expect("commit message should be visible");
        assert!(rect.center().y > h.size.y - 50.0);
    }

    #[test]
    fn performance_warning_opens_diagnostics_with_mesh_focus() {
        let mut h = Harness::new();
        assert!(!h.state.performance_open);
        h.state.mesh_error = Some("synthetic mesh warning".into());
        h.frame(vec![]);
        assert!(h.state.performance_open);
        assert!(h.state.performance_warning_active);
    }

    #[test]
    fn completed_mesh_slice_does_not_keep_warning_badge_lit() {
        let mut h = Harness::new();
        h.state.mesh_max_slice_ms = 12.0;
        h.frame(vec![]);
        assert!(!h.state.performance_warning_active);
        assert!(!h.state.performance_open);
    }

    #[test]
    fn performance_summary_contains_solver_and_mesh_metrics() {
        let mut h = Harness::new();
        h.state.wave_steps_per_second = 42.0;
        let summary = h.state.performance_summary();
        assert!(summary.contains("FPS"));
        assert!(summary.contains("steps/s 42.0"));
        assert!(summary.contains("DOFs"));
        assert!(summary.contains("mesh"));
        assert!(summary.contains("dt"));
    }

    #[test]
    fn performance_diagnostics_explain_local_retries_and_fallback_causes() {
        let mut h = Harness::new();
        h.state.performance_open = true;
        h.state.mesh_report = Some(MeshUpdateReport {
            local_attempted: true,
            repair_attempts: 3,
            repair_vertices: 120,
            repair_triangles: 210,
            moved_vertices: 18,
            repaired_baffles: 2,
            paired_trace_segments: 24,
            retry_failures: vec![MeshUpdateFailure {
                kind: MeshUpdateFailureKind::ElementInversion,
                detail: "synthetic inversion".into(),
            }],
            fallback_failure: Some(MeshUpdateFailure {
                kind: MeshUpdateFailureKind::RefinementLimit,
                detail: "synthetic refinement exhaustion".into(),
            }),
            ..Default::default()
        });
        h.state
            .mesh_fallback_causes
            .insert(MeshUpdateFailureKind::RefinementLimit, 2);
        h.frame(vec![]);
        h.frame(vec![]);
        assert!(
            h.texts.iter().any(|(text, _)| text.contains("attempts 3")),
            "{:?}",
            h.texts.iter().map(|(text, _)| text).collect::<Vec<_>>()
        );
        assert!(
            h.texts
                .iter()
                .any(|(text, _)| text.contains("Retry 1: local motion inverted"))
        );
        assert!(
            h.texts
                .iter()
                .any(|(text, _)| text.contains("Baffles 2 · paired trace segments 24"))
        );
        assert!(
            h.texts
                .iter()
                .any(|(text, _)| text.contains("Full rebuild: local refinement limit"))
        );
        assert!(
            h.texts
                .iter()
                .any(|(text, _)| text.contains("local refinement limit · 2"))
        );
    }

    #[test]
    fn panning_and_panel_capture_never_edit_geometry() {
        let mut h = Harness::new();
        let before = h.state.editor.document.clone();
        let center = h.state.center;
        let p = h.rect.center();
        h.button(p, PointerButton::Secondary, true);
        h.move_to(p + egui::vec2(40.0, 20.0));
        h.button(p + egui::vec2(40.0, 20.0), PointerButton::Secondary, false);
        assert_ne!(h.state.center, center);
        assert_eq!(h.state.editor.document, before);
        let center = h.state.center;
        let panel = Pos2::new(100.0, 500.0);
        h.button(panel, PointerButton::Primary, true);
        h.move_to(p);
        h.button(p, PointerButton::Primary, false);
        assert_eq!(h.state.center, center);
        assert_eq!(h.state.editor.document, before);
        assert_eq!(h.state.editor.history_len(), (0, 0));
    }
    #[test]
    fn double_click_inserts_once_and_existing_knot_selects() {
        let mut h = Harness::new();
        h.state.handles = false;
        let spline = h.state.editor.document.draft.obstacles[0].spline.clone();
        let p = h.point(spline.evaluate(0.5));
        h.click(p);
        h.click(p);
        assert_eq!(
            h.state.editor.document.draft.obstacles[0]
                .spline
                .controls()
                .len(),
            9
        );
        assert_eq!(h.state.editor.history_len().0, 1);
        h.settle();
        assert_eq!(h.state.editor.acceptance, Acceptance::Valid);
        h.time += 1.0;
        h.click(p);
        h.click(p);
        assert_eq!(
            h.state.editor.document.draft.obstacles[0]
                .spline
                .controls()
                .len(),
            9
        );
        assert_eq!(h.state.editor.history_len().0, 1);
        assert!(h.state.selection.unwrap().1.is_some());
        h.key(Key::Delete, Modifiers::NONE);
        assert_eq!(
            h.state.editor.document.draft.obstacles[0]
                .spline
                .controls()
                .len(),
            8
        );
        assert_eq!(h.state.editor.history_len().0, 2);
    }
    #[test]
    fn numeric_edit_commits_once_and_typing_captures_editor_keys() {
        let mut h = Harness::new();
        h.click(h.point(Point2::new(0.15, 0.0)));
        h.state.interaction_mode = InteractionMode::DrawCustom {
            role: CreationRole::Hole,
        };
        h.state.custom = vec![
            Point2::new(-0.7, 0.4),
            Point2::new(-0.4, 0.4),
            Point2::new(-0.4, 0.7),
            Point2::new(-0.7, 0.7),
        ];
        h.frame(vec![]);
        let x = h
            .texts
            .iter()
            .find(|(text, _)| text.starts_with("x "))
            .unwrap()
            .1
            .center();
        h.click(x);
        assert!(h.ctx.text_edit_focused());
        h.move_to(h.rect.center());
        h.key(Key::Delete, Modifiers::NONE);
        assert_eq!(
            h.state.editor.document.draft.obstacles[0]
                .spline
                .controls()
                .len(),
            8
        );
        h.frame(vec![Event::Text("0.25".into())]);
        h.key(Key::Enter, Modifiers::NONE);
        assert_eq!(h.state.editor.document.draft.obstacles.len(), 1);
        assert_eq!(h.state.custom.len(), 4);
        assert_eq!(
            h.state.editor.document.draft.obstacles[0].spline.controls()[0].x,
            0.25
        );
        assert_eq!(h.state.editor.history_len(), (1, 0));
        h.key(Key::Z, Modifiers::COMMAND);
        assert_eq!(
            h.state.editor.document.draft.obstacles[0].spline.controls()[0].x,
            0.15
        );
    }
    #[test]
    fn space_pan_and_custom_close_by_first_handle() {
        let mut h = Harness::new();
        let center = h.state.center;
        let p = h.rect.center();
        h.move_to(p);
        h.frame(vec![Event::Key {
            key: Key::Space,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        h.button(p, PointerButton::Primary, true);
        h.move_to(p + egui::vec2(50.0, 0.0));
        h.button(p + egui::vec2(50.0, 0.0), PointerButton::Primary, false);
        h.frame(vec![Event::Key {
            key: Key::Space,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        assert_ne!(center, h.state.center);
        assert_eq!(h.state.editor.history_len(), (0, 0));
        h.state.interaction_mode = InteractionMode::DrawCustom {
            role: CreationRole::Hole,
        };
        let points = [
            Point2::new(-0.7, 0.4),
            Point2::new(-0.4, 0.4),
            Point2::new(-0.4, 0.7),
            Point2::new(-0.7, 0.7),
        ];
        for p in points {
            h.click(h.point(p));
        }
        h.click(h.point(points[0]));
        assert!(h.state.custom.is_empty());
        assert_eq!(h.state.editor.document.draft.obstacles.len(), 2);
        assert_eq!(h.state.editor.history_len(), (1, 0));
    }

    #[test]
    fn pulse_is_transient_and_continuous_source_is_directly_draggable() {
        let mut h = Harness::new();
        build_mesh_candidate(&mut h.state);
        commit_mesh_without_gpu(&mut h.state);
        h.click_text("Simulation");
        h.click_text("Place pulse");
        assert_eq!(h.state.interaction_mode, InteractionMode::PlacePulse);
        h.click_text("Place pulse");
        assert_eq!(h.state.interaction_mode, InteractionMode::Select);
        assert!(!h.texts.iter().any(|(text, _)| text == "Move source"));

        let document = h.state.editor.document.clone();
        h.state.interaction_mode = InteractionMode::PlacePulse;
        let pulse = Point2::new(0.45, -0.3);
        h.click(h.point(pulse));
        assert!((h.state.wave_pending_pulse.unwrap() - pulse).norm() < 1.0e-6);
        assert_eq!(h.state.interaction_mode, InteractionMode::PlacePulse);
        let second_pulse = Point2::new(0.2, -0.1);
        h.click(h.point(second_pulse));
        assert!((h.state.wave_pending_pulse.unwrap() - second_pulse).norm() < 1.0e-6);
        assert_eq!(h.state.editor.document, document);

        h.state.interaction_mode = InteractionMode::Select;
        h.click_text("Continuous source");
        assert!(h.state.wave_source.enabled);
        assert_eq!(h.state.editor.document.source, h.state.wave_source);
        let source = Point2::new(-0.55, 0.25);
        h.drag_with_modifiers(
            h.point(h.state.wave_source.position),
            h.point(source),
            Modifiers::NONE,
        );
        assert!((h.state.wave_source.position - source).norm() < 1.0e-6);
        assert_eq!(h.state.editor.document.source, h.state.wave_source);
        assert!(h.state.wave_source_dirty);
        assert_eq!(h.state.interaction_mode, InteractionMode::Select);
        assert_eq!(h.state.editor.history_len(), (2, 0));

        let cancelled = Point2::new(-0.2, 0.4);
        h.button(h.point(source), PointerButton::Primary, true);
        h.move_to(h.point(cancelled));
        h.key(Key::Escape, Modifiers::NONE);
        h.button(h.point(cancelled), PointerButton::Primary, false);
        assert!((h.state.wave_source.position - source).norm() < 1.0e-6);
        assert_eq!(h.state.editor.document.source, h.state.wave_source);
        assert_eq!(h.state.editor.history_len(), (2, 0));

        h.key(Key::Z, Modifiers::COMMAND);
        assert!(
            (h.state.wave_source.position - SourceSettings::default().position).norm() < 1.0e-6
        );
        h.key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
        assert!((h.state.wave_source.position - source).norm() < 1.0e-6);

        let positive = field_color(0.5, 2.0);
        let negative = field_color(-0.5, 2.0);
        assert!(positive.r() > positive.b());
        assert!(negative.b() > negative.r());
        assert_eq!(field_color(f32::NAN, 2.0), field_color(0.0, 2.0));
    }

    #[test]
    fn accepted_mesh_finishes_cooperatively_and_survives_an_invalid_draft() {
        let mut h = Harness::new();
        build_mesh_candidate(&mut h.state);
        commit_mesh_without_gpu(&mut h.state);
        let mesh = h.state.mesh.clone().expect("initial accepted mesh");
        assert_eq!(mesh.geometry_revision, 0);
        assert!(mesh.triangles.len() > 2_500);
        assert!(mesh.quality.maximum_edge_length <= 0.08);
        assert_eq!(h.state.mesh_low_quality.len(), mesh.triangles.len());

        h.state.editor.begin();
        h.state
            .editor
            .set_point(ObstacleId(1), 0, Point2::new(8.0, 0.0))
            .unwrap();
        h.state.editor.commit();
        h.settle();
        assert!(matches!(h.state.editor.acceptance, Acceptance::Invalid(_)));
        h.state.refresh_mesh();
        assert_eq!(h.state.mesh.as_ref().unwrap(), &mesh);
        assert!(h.state.mesh_job.is_none());

        // Resolution is part of the mesh request even when the accepted scene
        // and document revision stay unchanged. It does not enter edit history.
        let document = h.state.editor.document.clone();
        let history = h.state.editor.history_len();
        h.state.mesh_max_edge = 0.16;
        build_mesh_candidate(&mut h.state);
        assert!(Arc::ptr_eq(h.state.mesh.as_ref().unwrap(), &mesh));
        commit_mesh_without_gpu(&mut h.state);
        assert_eq!(h.state.mesh_source_max_edge, 0.16);
        let preview = h.state.mesh.as_ref().unwrap();
        assert!(preview.quality.maximum_edge_length <= 0.16);
        assert!(preview.triangles.len() < mesh.triangles.len() / 2);
        assert_eq!(h.state.editor.document, document);
        assert_eq!(h.state.editor.history_len(), history);
    }

    #[test]
    fn boundary_change_reuses_mesh_and_prepares_a_field_transfer() {
        let mut h = Harness::new();
        build_mesh_candidate(&mut h.state);
        commit_mesh_without_gpu(&mut h.state);
        let mesh = h.state.mesh.clone().expect("initial accepted mesh");

        let target = OuterBoundaryConditions::uniform(OuterBoundaryCondition::FirstOrderOutgoing);
        h.state.editor.document.draft.outer_boundaries = target;
        h.state.editor.document.accepted.outer_boundaries = target;
        h.state.refresh_mesh();

        let candidate = h
            .state
            .simulation_candidate
            .as_ref()
            .expect("boundary candidate");
        assert!(Arc::ptr_eq(&candidate.mesh, &mesh));
        assert!(h.state.mesh_job.is_none());
        assert_eq!(candidate.operator.outer_boundaries(), target);
        assert_eq!(candidate.exposed_nodes, 0);
        assert!(candidate.transfer.is_some());

        commit_mesh_without_gpu(&mut h.state);
        assert_eq!(h.state.wave_boundary_committed, target);
        assert!(Arc::ptr_eq(h.state.mesh.as_ref().unwrap(), &mesh));
    }

    #[test]
    fn baffle_law_change_reuses_mesh_and_prepares_a_field_transfer() {
        let mut h = Harness::new();
        let id = h
            .state
            .editor
            .create_internal_boundary(
                OpenCubicSpline::uniform(vec![
                    Point2::new(-0.7, 0.55),
                    Point2::new(-0.25, 0.65),
                    Point2::new(0.25, 0.52),
                    Point2::new(0.7, 0.62),
                ])
                .unwrap(),
                BACKGROUND_REGION,
            )
            .unwrap();
        h.settle();
        build_mesh_candidate(&mut h.state);
        commit_mesh_without_gpu(&mut h.state);
        let mesh = h.state.mesh.clone().expect("baffle mesh");
        let old_damping: f64 = h
            .state
            .wave_operator
            .as_ref()
            .unwrap()
            .lumped_damping()
            .iter()
            .sum();

        h.state
            .editor
            .set_internal_boundary_law(
                id,
                0,
                InternalBoundaryLaw {
                    left: FaceBoundaryCondition::Impedance { ratio: 1.0 },
                    ..InternalBoundaryLaw::REFLECTING
                },
            )
            .unwrap();
        h.settle();
        h.state.refresh_mesh();

        let candidate = h
            .state
            .simulation_candidate
            .as_ref()
            .expect("baffle-law candidate");
        assert!(Arc::ptr_eq(&candidate.mesh, &mesh));
        assert!(h.state.mesh_job.is_none());
        assert!(candidate.operator.lumped_damping().iter().sum::<f64>() > old_damping);
        assert_eq!(candidate.exposed_nodes, 0);
        assert!(candidate.transfer.is_some());
    }

    #[test]
    fn moved_baffle_repairs_locally_and_prepares_a_face_aware_field_transfer() {
        let mut h = Harness::new();
        let id = h
            .state
            .editor
            .create_internal_boundary(
                OpenCubicSpline::uniform(vec![
                    Point2::new(-0.7, 0.55),
                    Point2::new(-0.25, 0.65),
                    Point2::new(0.25, 0.52),
                    Point2::new(0.7, 0.62),
                ])
                .unwrap(),
                BACKGROUND_REGION,
            )
            .unwrap();
        h.settle();
        build_mesh_candidate(&mut h.state);
        commit_mesh_without_gpu(&mut h.state);
        let old_revision = h.state.wave_mesh.as_ref().unwrap().geometry_revision;

        let point = h
            .state
            .editor
            .internal_boundary(id)
            .unwrap()
            .spline
            .controls()[1];
        h.state.editor.begin();
        h.state
            .editor
            .set_internal_boundary_point(id, 1, point + Point2::new(0.015, -0.01))
            .unwrap();
        h.state.editor.commit();
        h.settle();
        build_mesh_candidate(&mut h.state);

        let candidate = h
            .state
            .simulation_candidate
            .as_ref()
            .expect("moved-baffle candidate");
        assert_ne!(candidate.mesh.geometry_revision, old_revision);
        assert!(candidate.transfer.is_some());
        assert_eq!(candidate.exposed_nodes, 0);
        let report = h.state.mesh_report.as_ref().unwrap();
        assert!(report.used_local, "{report:?}");
        assert_eq!(report.repaired_baffles, 1);
        assert!(report.paired_trace_segments > 0);
    }

    #[test]
    fn hole_law_change_reuses_mesh_and_prepares_a_field_transfer() {
        let mut h = Harness::new();
        build_mesh_candidate(&mut h.state);
        commit_mesh_without_gpu(&mut h.state);
        let mesh = h.state.mesh.clone().expect("initial accepted mesh");
        let old_damping: f64 = h
            .state
            .wave_operator
            .as_ref()
            .unwrap()
            .lumped_damping()
            .iter()
            .sum();

        h.state
            .editor
            .set_obstacle_boundary_condition(
                ObstacleId(1),
                0,
                FaceBoundaryCondition::Impedance { ratio: 1.0 },
            )
            .unwrap();
        h.settle();
        h.state.refresh_mesh();

        let candidate = h
            .state
            .simulation_candidate
            .as_ref()
            .expect("hole-law candidate");
        assert!(Arc::ptr_eq(&candidate.mesh, &mesh));
        assert!(h.state.mesh_job.is_none());
        assert!(candidate.operator.lumped_damping().iter().sum::<f64>() > old_damping);
        assert_eq!(candidate.exposed_nodes, 0);
        assert!(candidate.transfer.is_some());
    }

    #[test]
    fn material_change_reuses_mesh_and_prepares_a_field_transfer() {
        let mut h = Harness::new();
        build_mesh_candidate(&mut h.state);
        commit_mesh_without_gpu(&mut h.state);
        let mesh = h.state.mesh.clone().expect("initial accepted mesh");
        let old_mass: f64 = h
            .state
            .wave_operator
            .as_ref()
            .unwrap()
            .lumped_mass()
            .iter()
            .sum();

        let material = h.state.editor.add_material().unwrap();
        let mut values = h
            .state
            .editor
            .document
            .draft
            .material(material)
            .unwrap()
            .clone();
        values.mass_density = 2.0;
        h.state.editor.update_material(values).unwrap();
        h.state
            .editor
            .set_region_material(BACKGROUND_REGION, material)
            .unwrap();
        h.settle();
        h.state.refresh_mesh();

        let candidate = h
            .state
            .simulation_candidate
            .as_ref()
            .expect("material candidate");
        assert!(Arc::ptr_eq(&candidate.mesh, &mesh));
        assert!(h.state.mesh_job.is_none());
        assert_eq!(candidate.scene, h.state.editor.document.accepted);
        assert!(candidate.transfer.is_some());
        let new_mass: f64 = candidate.operator.lumped_mass().iter().sum();
        assert!((new_mass - 2.0 * old_mass).abs() < 1.0e-10);
    }

    #[test]
    fn mesh_edits_use_committed_source_and_failed_build_keeps_displayed_mesh() {
        let mut h = Harness::new();
        build_mesh_candidate(&mut h.state);
        commit_mesh_without_gpu(&mut h.state);
        let original = h.state.mesh.clone().unwrap();
        let original_scene = h.state.mesh_committed_scene.clone();
        for delta in [Point2::new(0.003, 0.0), Point2::new(0.003, 0.002)] {
            let p = original_scene.obstacles[0].spline.controls()[0];
            h.state.editor.begin();
            h.state
                .editor
                .set_point(ObstacleId(1), 0, p + delta)
                .unwrap();
            h.state.editor.commit();
            h.settle();
            h.state.refresh_mesh();
            assert!(h.state.mesh_job.is_some());
            assert!(Arc::ptr_eq(h.state.mesh.as_ref().unwrap(), &original));
            assert_eq!(h.state.mesh_committed_scene, original_scene);
        }
        let history = h.state.editor.history_len();
        build_mesh_candidate(&mut h.state);
        assert!(h.state.mesh_error.is_none());
        assert!(h.state.mesh_report.as_ref().unwrap().used_local);
        assert!(Arc::ptr_eq(h.state.mesh.as_ref().unwrap(), &original));
        assert_eq!(h.state.mesh_committed_scene, original_scene);
        commit_mesh_without_gpu(&mut h.state);
        assert_eq!(
            h.state.mesh.as_ref().unwrap().geometry_revision,
            h.state.editor.revision
        );
        assert_eq!(
            h.state.mesh_committed_scene,
            h.state.editor.document.accepted
        );
        assert_eq!(h.state.editor.history_len(), history);
        assert!(h.state.mesh_build_ms >= h.state.mesh_work_ms);
        assert_eq!(
            h.state.mesh_attempts, 1,
            "superseded jobs are not completed edits"
        );
        let displayed = h.state.mesh.clone().unwrap();
        // Deliberately exceed the fixed mesh capacity to exercise the error path.
        h.state.mesh_max_edge = 0.00001;
        for _ in 0..5000 {
            h.state.refresh_mesh();
            if h.state.mesh_job.is_none() {
                break;
            }
        }
        assert!(h.state.mesh_error.is_some());
        assert!(Arc::ptr_eq(h.state.mesh.as_ref().unwrap(), &displayed));
        assert_eq!(h.state.mesh_committed_max_edge, 0.08);
    }

    #[test]
    fn candidate_readback_renders_with_candidate_operator_during_commit_frame() {
        let mut h = Harness::new();
        build_mesh_candidate(&mut h.state);
        commit_mesh_without_gpu(&mut h.state);
        let active_revision = h.state.wave_operator.as_ref().unwrap().mesh_revision();
        let active_dofs = h.state.wave_operator.as_ref().unwrap().degrees_of_freedom();

        h.state.mesh_max_edge = 0.16;
        build_mesh_candidate(&mut h.state);
        let candidate = h.state.simulation_candidate.as_mut().unwrap();
        candidate.generation = Some(42);
        let candidate_revision = candidate.operator.mesh_revision();
        let candidate_dofs = candidate.operator.degrees_of_freedom();
        assert_ne!(candidate_revision, active_revision);

        let candidate_display = WaveDisplay {
            generation: 42,
            current: vec![0.0; candidate_dofs],
            ..Default::default()
        };
        assert_eq!(
            h.state
                .wave_display_operator(&candidate_display)
                .unwrap()
                .mesh_revision(),
            candidate_revision
        );

        let active_display = WaveDisplay {
            generation: 41,
            current: vec![0.0; active_dofs],
            ..Default::default()
        };
        assert_eq!(
            h.state
                .wave_display_operator(&active_display)
                .unwrap()
                .mesh_revision(),
            active_revision
        );
    }

    #[test]
    fn automatic_adaptation_controls_are_exposed_and_enabled_by_default() {
        let mut h = Harness::new();
        h.click_text("Simulation");
        assert!(h.state.amr_enabled);
        for label in ["Automatic adaptation", "Adapt mesh to the wave", "Balanced"] {
            assert!(
                h.texts.iter().any(|(text, _)| text == label),
                "missing {label}"
            );
        }
    }

    #[test]
    fn automatic_adaptation_budget_scales_with_mesh_and_preset() {
        let mut h = Harness::new();
        build_mesh_candidate(&mut h.state);
        let mesh = &h.state.simulation_candidate.as_ref().unwrap().mesh;
        let default_limit = MeshAdaptationOptions::default().max_work_units;
        let fast = automatic_adaptation_work_limit(mesh, AmrQuality::Fast.topology_budget());
        let detailed =
            automatic_adaptation_work_limit(mesh, AmrQuality::Detailed.topology_budget());
        assert!(fast >= default_limit);
        assert!(detailed > fast);
    }

    #[test]
    fn candidate_handoff_preserves_the_user_run_preference() {
        let mut h = Harness::new();
        build_mesh_candidate(&mut h.state);
        assert!(h.state.simulation_candidate.is_some());
        assert!(h.state.wave_running);

        let mut request = WaveGpuRequest::default();
        request.request_steps(1);
        let display = WaveDisplay::default();
        let mut assets = Assets::<ShaderBuffer>::default();
        let world = World::new();
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = Commands::new(&mut queue, &world);
        h.state.refresh_wave(
            &mut request,
            &display,
            &mut assets,
            &mut commands,
            1.0 / 60.0,
        );

        assert!(h.state.simulation_candidate.is_some());
        assert!(h.state.wave_running);
    }

    #[test]
    fn automatic_coarsening_requires_two_estimates_and_has_compatible_thresholds() {
        let mut streak = 0;
        assert!(!update_coarsening_confirmation(&mut streak, true));
        assert!(update_coarsening_confirmation(&mut streak, true));
        assert!(!update_coarsening_confirmation(&mut streak, false));
        assert_eq!(streak, 0);

        for quality in AmrQuality::ALL {
            assert!(quality.maximum_coarsening_scale() * quality.collapse_ratio() > 1.0);
        }
    }

    #[test]
    fn automatic_adaptation_work_limit_retains_the_committed_mesh() {
        let mut h = Harness::new();
        build_mesh_candidate(&mut h.state);
        commit_mesh_without_gpu(&mut h.state);
        let committed = h.state.mesh.clone().unwrap();
        let options = MeshAdaptationOptions {
            minimum_target_edge_length: 0.02,
            maximum_target_edge_length: 0.08,
            max_topology_changes: 1,
            max_work_units: 1,
            ..Default::default()
        };
        h.state
            .start_mesh_adaptation(Arc::new(|_: Point2, _: RegionId| 0.08), options)
            .unwrap();
        h.state.mesh_adaptation_automatic = true;

        h.state.refresh_mesh_adaptation();

        assert!(h.state.mesh_adaptation_job.is_none());
        assert!(h.state.mesh_error.is_none());
        assert!(h.state.amr_error.is_none());
        assert_eq!(
            h.state.amr_status,
            "adaptation budget reached; mesh retained"
        );
        assert!(Arc::ptr_eq(h.state.mesh.as_ref().unwrap(), &committed));
        assert_eq!(
            h.state.mesh_adaptation_report.as_ref().unwrap().work_units,
            2
        );
    }

    #[test]
    fn wavelength_guard_uses_every_active_time_varying_driver() {
        let mut scene = Scene::initial();
        scene.outer_boundaries.sides[0] = OuterBoundaryCondition::Dirichlet {
            signal: BoundarySignal {
                amplitude: 1.0,
                frequency_hz: 3.0,
                ..BoundarySignal::ZERO
            },
        };
        scene.obstacles[0].span_conditions[0] = FaceBoundaryCondition::Neumann {
            signal: BoundarySignal {
                amplitude: 2.0,
                frequency_hz: 5.0,
                ..BoundarySignal::ZERO
            },
        };
        let source = SourceSettings {
            enabled: true,
            amplitude: 1.0,
            frequency_hz: 4.0,
            ..SourceSettings::default()
        };
        assert_eq!(highest_forcing_frequency(&scene, source), 5.0);
        scene.obstacles[0].span_conditions[0] = FaceBoundaryCondition::Neumann {
            signal: BoundarySignal {
                amplitude: 0.0,
                frequency_hz: 9.0,
                ..BoundarySignal::ZERO
            },
        };
        assert_eq!(highest_forcing_frequency(&scene, source), 4.0);
    }

    #[test]
    fn indicator_auxiliary_is_aligned_to_the_centered_gpu_snapshot() {
        let mut h = Harness::new();
        build_mesh_candidate(&mut h.state);
        commit_mesh_without_gpu(&mut h.state);
        let operator = h.state.wave_operator.as_ref().unwrap();
        let count = operator.degrees_of_freedom();
        let display = WaveDisplay {
            current: vec![3.0; count],
            auxiliary: vec![10.0; count],
            indicator_displacement: vec![1.0; count],
            ..Default::default()
        };

        let aligned = aligned_indicator_auxiliary(&display, operator, 0.2, 1).unwrap();
        let active = operator
            .auxiliary_active()
            .iter()
            .zip(operator.dirichlet_signals())
            .position(|(active, dirichlet)| *active && dirichlet.is_none())
            .expect("second-order boundary node");
        let inactive = operator
            .auxiliary_active()
            .iter()
            .position(|active| !*active)
            .expect("interior node");
        assert!((aligned[active] - 9.6).abs() < 1.0e-12);
        assert_eq!(aligned[inactive], 10.0);
        assert_eq!(
            aligned_indicator_auxiliary(&display, operator, 0.2, 0).unwrap(),
            vec![10.0; count]
        );
    }
}
