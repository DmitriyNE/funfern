use crate::files::{self, FileEvent};
#[cfg(not(target_arch = "wasm32"))]
use crate::wave_gpu::forcing_weights;
use crate::wave_gpu::{PulseSettings, SourceSettings, WaveDisplay, WaveGpuRequest, WaveTransfer};
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use bevy_egui::{
    EguiContexts,
    egui::{self, Color32, Pos2, Rect, Stroke},
};
use funfern_app::{
    editor::{Acceptance, BoundaryFaceTarget, Editor, GeometryControl, LoopKind},
    persistence::{self, LoadCandidate},
};
use funfern_core::*;
use std::collections::{BTreeSet, VecDeque};
use std::sync::{
    Arc, Mutex,
    mpsc::{self, Receiver, Sender},
};
const TEAL: Color32 = Color32::from_rgb(91, 220, 194);
const RED: Color32 = Color32::from_rgb(255, 106, 123);
const GOLD: Color32 = Color32::from_rgb(248, 196, 112);
#[derive(Default, PartialEq)]
enum Mode {
    #[default]
    Select,
    Preset,
    Custom,
    Pulse,
    Source,
}
#[derive(Clone, Copy, Default, PartialEq)]
enum CreationRole {
    #[default]
    Hole,
    MaterialInterface,
    InternalBoundary,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ActiveTool {
    #[default]
    Select,
    AddGeometry,
    Materials,
    Simulation,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum InspectorPanel {
    #[default]
    Edit,
    View,
    Simulation,
    Materials,
}
impl InspectorPanel {
    const ALL: [Self; 4] = [Self::Edit, Self::View, Self::Simulation, Self::Materials];

    const fn label(self) -> &'static str {
        match self {
            Self::Edit => "Edit",
            Self::View => "View",
            Self::Simulation => "Simulation",
            Self::Materials => "Materials",
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
#[derive(Clone, Copy)]
enum MarqueeOperation {
    Replace,
    Add,
    Subtract,
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
    resume_running: bool,
    simulation_time: f64,
    exposed_nodes: usize,
    source_region: RegionId,
}
#[derive(Resource)]
pub struct Playground {
    automated_benchmark: bool,
    editor: Editor,
    active_tool: ActiveTool,
    inspector_panel: Option<InspectorPanel>,
    add_geometry_open: bool,
    performance_open: bool,
    performance_history: VecDeque<f32>,
    performance_warning_active: bool,
    mode: Mode,
    creation_role: CreationRole,
    material_selection: MaterialId,
    material_name_edit: Option<(MaterialId, String)>,
    region_selection: RegionId,
    selection: Option<(ObstacleId, Option<usize>)>,
    internal_selection: Option<(InternalBoundaryId, Option<usize>)>,
    selected_spans: Vec<GeometrySpan>,
    span_selection_filter: SpanSelectionFilter,
    baffle_face: InternalBoundarySide,
    gizmo_pivot: Option<Point2>,
    pending_span_collapse: Option<GeometrySpan>,
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
    cache_revision: u64,
    cache_scale: f64,
    cache_accepted: Scene,
    draft_curves: Vec<Curve>,
    accepted_curves: Vec<Curve>,
    draft_internal_curves: Vec<InternalCurve>,
    accepted_internal_curves: Vec<InternalCurve>,
    sampling_warning: bool,
    message: String,
    sender: Sender<FileEvent>,
    receiver: Mutex<Receiver<FileEvent>>,
    file_busy: bool,
    load: Option<LoadCandidate>,
    frame_ms: f32,
    ready: bool,
    keyboard_captured: bool,
    mesh: Option<Arc<TriMesh>>,
    simulation_candidate: Option<SimulationCandidate>,
    mesh_job: Option<MeshUpdateJob>,
    mesh_source: Scene,
    mesh_committed_scene: Scene,
    mesh_committed_max_edge: f64,
    mesh_report: Option<MeshUpdateReport>,
    mesh_attempts: usize,
    mesh_fallbacks: usize,
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
    wave_step_requested: bool,
    wave_pending_pulse: Option<Point2>,
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
            active_tool: ActiveTool::Select,
            inspector_panel: Some(InspectorPanel::Edit),
            add_geometry_open: false,
            performance_open: false,
            performance_history: VecDeque::with_capacity(90),
            performance_warning_active: false,
            mode: Mode::Select,
            creation_role: CreationRole::Hole,
            material_selection: DEFAULT_MATERIAL,
            material_name_edit: None,
            region_selection: BACKGROUND_REGION,
            selection: Some((ObstacleId(1), None)),
            internal_selection: None,
            selected_spans: (0..8)
                .map(|span| GeometrySpan::Loop(ObstacleId(1), span))
                .collect(),
            span_selection_filter: SpanSelectionFilter::All,
            baffle_face: InternalBoundarySide::Left,
            gizmo_pivot: None,
            pending_span_collapse: None,
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
            cache_revision: u64::MAX,
            cache_scale: 0.0,
            cache_accepted: Scene::default(),
            draft_curves: vec![],
            accepted_curves: vec![],
            draft_internal_curves: vec![],
            accepted_internal_curves: vec![],
            sampling_warning: false,
            message: String::new(),
            sender,
            receiver: Mutex::new(receiver),
            file_busy: false,
            load: None,
            frame_ms: 0.0,
            ready: false,
            keyboard_captured: false,
            mesh: None,
            simulation_candidate: None,
            mesh_job: None,
            mesh_source: Scene::default(),
            mesh_committed_scene: Scene::default(),
            mesh_committed_max_edge: 0.0,
            mesh_report: None,
            mesh_attempts: 0,
            mesh_fallbacks: 0,
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
            wave_step_requested: false,
            wave_pending_pulse: None,
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
    fn world(&self, p: Pos2, r: Rect) -> Point2 {
        Point2::new(
            self.center.x + (p.x - r.center().x) as f64 / self.scale,
            self.center.y - (p.y - r.center().y) as f64 / self.scale,
        )
    }
    fn clear_transient(&mut self) {
        self.selection = None;
        self.internal_selection = None;
        self.selected_spans.clear();
        self.gizmo_pivot = None;
        self.pending_span_collapse = None;
        self.region_selection = BACKGROUND_REGION;
        self.drag = None;
        self.panning = false;
        self.custom.clear();
        self.mode = Mode::Select;
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

    fn select_control(&mut self, control: GeometryControl) {
        self.selected_spans.clear();
        self.gizmo_pivot = None;
        self.pending_span_collapse = None;
        self.focus_control(Some(control));
    }

    fn focus_control(&mut self, control: Option<GeometryControl>) {
        match control {
            None => {
                self.selection = None;
                self.internal_selection = None;
            }
            Some(GeometryControl::Loop(id, index)) => {
                self.selection = Some((id, Some(index)));
                self.internal_selection = None;
            }
            Some(GeometryControl::Baffle(id, index)) => {
                self.selection = None;
                self.internal_selection = Some((id, Some(index)));
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
        self.selected_spans.clear();
        let mut seen = BTreeSet::new();
        for span in spans {
            if self.span_valid(span) && seen.insert(geometry_span_key(span)) {
                self.selected_spans.push(span);
            }
        }
        self.gizmo_pivot = None;
        self.pending_span_collapse = None;
        match self.selected_spans.last().copied() {
            Some(GeometrySpan::Loop(id, _)) => {
                self.selection = Some((id, None));
                self.internal_selection = None;
            }
            Some(GeometrySpan::Baffle(id, _)) => {
                self.selection = None;
                self.internal_selection = Some((id, None));
            }
            Some(GeometrySpan::Outer(_)) | None => {
                self.selection = None;
                self.internal_selection = None;
            }
        }
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
                self.mode = Mode::Select;
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
            self.mode = Mode::Select;
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
        let events: Vec<_> = self.receiver.lock().unwrap().try_iter().collect();
        for event in events {
            self.file_busy = false;
            match event {
                FileEvent::Loaded(bytes) => match persistence::parse(&bytes) {
                    Ok(load) => {
                        self.load = Some(load);
                        self.message = "Validating scene file…".into()
                    }
                    Err(e) => self.message = e,
                },
                FileEvent::Saved => self.message = "Scene saved".into(),
                FileEvent::Cancelled => {}
                FileEvent::Error(e) => self.message = e,
            }
        }
        if let Some(load) = &mut self.load
            && let Some(result) = load.advance(12_000)
        {
            self.load = None;
            match result {
                Ok(document) => {
                    self.editor.replace_validated(document);
                    self.clear_transient();
                    self.message = "Scene loaded; history cleared".into()
                }
                Err(e) => self.message = e,
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
        if self.editor.editing() || self.simulation_candidate.is_some() {
            return;
        }
        let start = Instant::now();
        let geometry_changed = !self.mesh_source.geometry_eq(&self.editor.document.accepted)
            || self.mesh_source_max_edge != self.mesh_max_edge;
        if geometry_changed {
            self.mesh_started = Some(start);
            self.mesh_source = self.editor.document.accepted.clone();
            self.mesh_source_max_edge = self.mesh_max_edge;
            let previous = self
                .mesh
                .as_ref()
                .filter(|_| self.mesh_committed_max_edge == self.mesh_max_edge)
                .map(|mesh| (mesh.clone(), self.mesh_committed_scene.clone()));
            self.mesh_job = Some(MeshUpdateJob::new(
                previous,
                self.mesh_source.clone(),
                self.editor.revision,
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
                            let transfer = self
                                .wave_mesh
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
                                .transpose();
                            match transfer {
                                Ok(transfer) => {
                                    let exposed_nodes = transfer
                                        .as_ref()
                                        .map_or(operator.degrees_of_freedom(), |map| {
                                            map.exposed_nodes()
                                        });
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
                                        resume_running: self.wave_running,
                                        simulation_time: 0.0,
                                        exposed_nodes,
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
            && self.editor.document.accepted != self.mesh_committed_scene
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
                    match QuadraticTransferMap::identity_on_mesh(mesh, source_operator, &operator) {
                        Ok(transfer) => {
                            let time_step = operator.recommended_time_step();
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
                                exposed_nodes: transfer.exposed_nodes(),
                                transfer: Some(transfer),
                                generation: None,
                                resume_running: self.wave_running,
                                simulation_time: 0.0,
                            });
                            self.wave_prepare_ms = prepare.elapsed().as_secs_f64() * 1000.0;
                            self.wave_error = None;
                            self.message = "Preparing material/boundary transaction…".into();
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
            self.wave_running = false;
            if request.caught_up() {
                let mut target_source = self.wave_source;
                target_source.region = candidate.source_region;
                candidate.simulation_time = self.wave_time_offset
                    + request.stats().completed_steps() as f64 * self.wave_time_step;
                let replacement = if let (Some(source_mesh), Some(source_operator), Some(map)) =
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
            let commit_message = if same_mesh && scene_settings_changed && boundary_changed {
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
            self.wave_boundary_committed = candidate.boundary;
            self.wave_time_step = candidate.time_step;
            self.wave_time_offset = candidate.simulation_time;
            self.wave_completed_steps = 0;
            self.wave_steps_per_second = 0.0;
            self.wave_rate_previous_completed = 0;
            self.wave_energy = None;
            self.wave_energy_step = u64::MAX;
            self.wave_source_dirty = false;
            self.wave_source.region = candidate.source_region;
            self.wave_running = candidate.resume_running;
            self.mesh_build_ms = self
                .mesh_started
                .map_or(0.0, |started| started.elapsed().as_secs_f64() * 1000.0);
            self.message = commit_message;
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
                    amplitude: 0.65,
                    width: 0.06,
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
        if self.wave_operator.is_some() && request.ready() {
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
        match panel {
            InspectorPanel::Edit => {
                if self.active_tool != ActiveTool::Select {
                    self.active_tool = ActiveTool::Select;
                }
            }
            InspectorPanel::View => self.active_tool = ActiveTool::Select,
            InspectorPanel::Simulation => self.active_tool = ActiveTool::Simulation,
            InspectorPanel::Materials => self.active_tool = ActiveTool::Materials,
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(egui::RichText::new("funfern").size(20.0).color(TEAL));
            ui.separator();
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
            if ui
                .add_enabled(file_enabled, egui::Button::new("Save"))
                .clicked()
            {
                self.editor.commit();
                match persistence::save(&self.editor.document) {
                    Ok(json) => {
                        files::save(self.sender.clone(), json.into_bytes());
                        self.file_busy = true;
                    }
                    Err(error) => self.message = error,
                }
            }
            if ui
                .add_enabled(file_enabled, egui::Button::new("Load"))
                .clicked()
            {
                self.editor.commit();
                files::load(self.sender.clone());
                self.file_busy = true;
            }
            if ui.button("Fit view").clicked() {
                self.fit = true;
            }
            ui.separator();
            for panel in InspectorPanel::ALL {
                if ui
                    .selectable_label(self.inspector_panel == Some(panel), panel.label())
                    .clicked()
                {
                    self.select_inspector_panel(panel);
                }
            }
            if ui.button("+ Add").clicked() {
                self.inspector_panel = Some(InspectorPanel::Edit);
                self.active_tool = ActiveTool::AddGeometry;
                self.add_geometry_open = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let wave_available = self.wave_operator.is_some();
                ui.add_enabled_ui(wave_available, |ui| {
                    if ui.button("Reset").clicked() {
                        self.wave_reset_requested = true;
                    }
                    if ui.button("Step").clicked() {
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
            });
        });
    }

    fn add_geometry_popover(&mut self, ctx: &egui::Context) {
        if !self.add_geometry_open {
            return;
        }
        let mut open = true;
        let mut close = false;
        egui::Window::new("Add geometry")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
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
                        "Rounded loop"
                    };
                    if ui.button(primitive_label).clicked() {
                        self.mode = Mode::Preset;
                        self.active_tool = ActiveTool::AddGeometry;
                        close = true;
                    }
                    if ui.button("Custom").clicked() {
                        self.mode = Mode::Custom;
                        self.active_tool = ActiveTool::AddGeometry;
                        self.custom.clear();
                        close = true;
                    }
                });
                ui.small("More standard primitives can be added here later.");
            });
        if close {
            open = false;
        }
        self.add_geometry_open = open;
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
        format!("FPS {fps:.0} · steps/s {steps_per_second:.1} · N {dofs} · mesh {mesh} · dt {dt}")
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
                ui.heading("Performance diagnostics");
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
                        ui.small(format!(
                            "Recent peak {:.2} ms · {} samples",
                            recent_max,
                            self.performance_history.len()
                        ));
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
                            "Completed edits {} · full rebuild fallbacks {}",
                            self.mesh_attempts, self.mesh_fallbacks
                        ));
                        if let Some(report) = self
                            .mesh_job
                            .as_ref()
                            .map(|job| job.report())
                            .or(self.mesh_report.as_ref())
                        {
                            if report.used_local {
                                ui.small(format!(
                                    "Local repair · {:.1}% triangles unchanged",
                                    100.0 * report.preserved_triangles as f64
                                        / report.original_triangles.max(1) as f64
                                ));
                            }
                            if let Some(reason) = &report.fallback_reason {
                                ui.colored_label(GOLD, format!("Full rebuild: {reason}"));
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
            Some(InspectorPanel::Edit) | None => self.edit_panel(ui),
        }
    }

    fn view_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.heading("View");
        ui.separator();
        ui.checkbox(&mut self.grid, "Grid");
        ui.checkbox(&mut self.polygon, "Control polygons");
        ui.checkbox(&mut self.handles, "Handles");
        ui.checkbox(&mut self.reference, "Accepted reference");
        ui.checkbox(&mut self.show_materials, "Material regions");
        ui.checkbox(&mut self.show_mesh, "Accepted triangle mesh");
        ui.add_enabled_ui(self.show_mesh, |ui| {
            ui.checkbox(&mut self.show_mesh_boundary, "Mesh boundary labels");
        });
        ui.checkbox(&mut self.show_field, "Field colors");
        ui.add(
            egui::Slider::new(&mut self.field_gain, 0.25..=12.0)
                .logarithmic(true)
                .text("field intensity"),
        );
    }

    fn materials_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.heading("Materials");
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
                        |loop_| format!("Loop {} interior", loop_.id.0),
                    )
            };
            let mut selected = region.material;
            if ui
                .selectable_label(self.region_selection == region.id, label)
                .clicked()
            {
                self.region_selection = region.id;
            }
            egui::ComboBox::from_id_salt(("region_material", region.id.0))
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
            if selected != region.material {
                let result = self.editor.set_region_material(region.id, selected);
                self.error(result);
            }
        }
        ui.separator();
        ui.label("Library");
        ui.horizontal(|ui| {
            ui.label("Material");
            if ui.small_button("+").clicked() {
                let result = self.editor.add_material();
                if let Some(id) = self.error(result) {
                    self.material_selection = id;
                }
            }
        });
        egui::ComboBox::from_id_salt("material_editor")
            .selected_text(
                materials
                    .iter()
                    .find(|material| material.id == self.material_selection)
                    .map_or("Missing", |material| material.name.as_str()),
            )
            .show_ui(ui, |ui| {
                for material in &materials {
                    ui.selectable_value(&mut self.material_selection, material.id, &material.name);
                }
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

    fn simulation_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.heading("Simulation");
        ui.add_space(8.0);
        ui.separator();
        if self.active_tool == ActiveTool::Simulation {
            ui.label("Accepted mesh");
            egui::ComboBox::from_id_salt("mesh_resolution")
                .selected_text(format!("Max edge {:.2}", self.mesh_max_edge))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.mesh_max_edge,
                        0.16,
                        "Coarse P2e · parent h ≤ 0.16",
                    );
                    ui.selectable_value(&mut self.mesh_max_edge, 0.08, "P2e · parent h ≤ 0.08");
                    ui.selectable_value(
                        &mut self.mesh_max_edge,
                        0.04,
                        "Fine P2e · parent h ≤ 0.04",
                    );
                });
            ui.small("Seven-node enriched quadratic wave basis.");
            if self.editor.editing() && self.mesh_source != self.editor.document.accepted {
                ui.small("Waiting for edit to finish…");
            } else if let Some(job) = &self.mesh_job {
                ui.small(format!("Mesh rebuilding: {}…", job.phase()));
            } else if let Some(error) = &self.mesh_error {
                ui.colored_label(RED, error);
                ui.small("Previous mesh retained.");
            } else if self.mesh.is_some() {
                ui.small("Mesh ready.");
            } else {
                ui.small("Preparing…");
            }
            let wave_available = self.wave_operator.is_some();
            ui.add_enabled_ui(wave_available, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Button::new("Place pulse").selected(self.mode == Mode::Pulse))
                        .clicked()
                    {
                        self.mode = Mode::Pulse;
                    }
                    if ui
                        .add(egui::Button::new("Move source").selected(self.mode == Mode::Source))
                        .clicked()
                    {
                        self.mode = Mode::Source;
                    }
                });
                ui.add(
                    egui::Slider::new(&mut self.wave_speed, 0.1..=4.0)
                        .logarithmic(true)
                        .text("simulation speed"),
                );
                if ui
                    .checkbox(&mut self.wave_source.enabled, "Continuous source")
                    .changed()
                {
                    self.wave_source_dirty = true;
                }
                ui.add_enabled_ui(self.wave_source.enabled, |ui| {
                    if ui
                        .add(
                            egui::Slider::new(&mut self.wave_source.frequency_hz, 0.25..=8.0)
                                .logarithmic(true)
                                .text("source frequency"),
                        )
                        .changed()
                    {
                        self.wave_source_dirty = true;
                    }
                    if ui
                        .add(
                            egui::Slider::new(&mut self.wave_source.amplitude, 1.0..=50.0)
                                .logarithmic(true)
                                .text("source strength"),
                        )
                        .changed()
                    {
                        self.wave_source_dirty = true;
                    }
                });
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
            ui.small(format!(
                "{} outer boundary · λ=0.4 source preset",
                self.wave_boundary_committed.label()
            ));
        }
    }

    fn edit_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.heading("Edit");
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
        if self.active_tool == ActiveTool::Select {
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
        }
        if self.active_tool == ActiveTool::AddGeometry && self.mode == Mode::Custom {
            ui.small(format!(
                "{} / 128 points · Backspace removes · Esc cancels",
                self.custom.len()
            ));
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
                            ) || self.selection.is_some_and(|s| s.0 == o.id),
                            format!(
                                "Loop {:02} · {}{} · {} controls",
                                o.id.0,
                                o.role.label(),
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
                            }) || self
                                .internal_selection
                                .is_some_and(|selection| selection.0 == boundary.id),
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
        if let Some((id, index)) = self.selection {
            if ui.button("Delete loop").clicked() {
                self.editor.delete_obstacle(id);
                self.clear_transient();
            }
            if index.is_none() && ui.button("Duplicate loop").clicked() {
                let result = self.editor.duplicate_obstacle(id, Point2::new(0.05, -0.05));
                if let Some(id) = self.error(result) {
                    self.select_loop(id);
                }
            }
            if index.is_none()
                && let Some(current_kind) = self.editor.loop_kind(id)
            {
                ui.add_space(6.0);
                ui.label("Loop role");
                let mut kind = current_kind;
                egui::ComboBox::from_id_salt(("loop_kind", id.0))
                    .selected_text(match current_kind {
                        LoopKind::Hole => "Hole",
                        LoopKind::MaterialInterface => "Material interface",
                        LoopKind::Wall => "Two-sided closed wall",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut kind, LoopKind::Hole, "Hole");
                        ui.selectable_value(
                            &mut kind,
                            LoopKind::MaterialInterface,
                            "Material interface",
                        );
                        ui.selectable_value(&mut kind, LoopKind::Wall, "Two-sided closed wall");
                    });
                if current_kind == LoopKind::Hole {
                    let materials = self.editor.document.draft.materials.clone();
                    egui::ComboBox::from_label("New interior material")
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
                if kind != current_kind {
                    let result = self.editor.set_loop_kind(id, kind, self.material_selection);
                    if self.error(result).is_some() {
                        self.select_loop(id);
                    }
                }
            }
            if let Some(index) = index
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
        }
        if let Some((id, index)) = self.internal_selection {
            if ui.button("Delete baffle").clicked() {
                self.editor.delete_internal_boundary(id);
                self.clear_transient();
            }
            if index.is_none() && ui.button("Duplicate baffle").clicked() {
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
            if let Some(index) = index
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
                    if ui.button("Align horizontal").clicked() {
                        self.align_selection(true);
                    }
                    if ui.button("Align vertical").clicked() {
                        self.align_selection(false);
                    }
                });
            }
        }
        self.topology_inspector(ui);
        self.boundary_inspector(ui);
        ui.add_space(12.0);
        ui.add_space(6.0);
        ui.small("Pan: middle drag / Space + drag\nZoom: wheel over viewport\nUndo: Ctrl/Cmd + Z · Shift for redo");
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

    fn topology_inspector(&mut self, ui: &mut egui::Ui) {
        if self.selected_spans.is_empty() {
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
                                        self.message = format!(
                                            "Continuity upgraded; curve reshaped by at most {displacement:.3e}"
                                        );
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
                                            self.message = format!(
                                                "Continuity upgraded; curve reshaped by at most {displacement:.3e}"
                                            );
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

        if let Some((loop_breakpoints, baffle_breakpoints)) = self.exposed_selection_breakpoints()
            && (!loop_breakpoints.is_empty() || !baffle_breakpoints.is_empty())
        {
            let needs_refinement = loop_breakpoints.iter().any(|(id, breakpoint)| {
                self.editor
                    .obstacle(*id)
                    .and_then(|obstacle| obstacle.spline.continuity(*breakpoint))
                    != Some(0)
            }) || baffle_breakpoints.iter().any(|(id, breakpoint)| {
                self.editor
                    .internal_boundary(*id)
                    .and_then(|boundary| boundary.spline.continuity(*breakpoint))
                    != Some(0)
            });
            if ui
                .add_enabled(
                    needs_refinement,
                    egui::Button::new("Isolate selection at C0"),
                )
                .clicked()
            {
                let result = self
                    .editor
                    .isolate_span_boundaries(&loop_breakpoints, &baffle_breakpoints);
                if self.error(result).is_some() {
                    self.gizmo_pivot = None;
                }
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
                    "Nearest tips are {:.4} apart; merge tolerance is {:.4}. Side laws follow the arrows.",
                    distance, merge_tolerance
                ));
            } else {
                ui.colored_label(
                    GOLD,
                    format!(
                        "Nearest tips are {:.4} apart; move them within {:.4} to enable merge.",
                        distance, merge_tolerance
                    ),
                );
            }
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
        ui.strong(format!("{} selected spans", self.selected_spans.len()));
        let has_baffles = self
            .selected_spans
            .iter()
            .any(|span| matches!(span, GeometrySpan::Baffle(_, _)));
        if has_baffles {
            ui.horizontal(|ui| {
                ui.label("Baffle face");
                ui.selectable_value(&mut self.baffle_face, InternalBoundarySide::Left, "Left");
                ui.selectable_value(&mut self.baffle_face, InternalBoundarySide::Right, "Right");
            });
        }

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
            let mut selected = None;
            egui::ComboBox::from_label("Span law")
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
                changed |= ui
                    .add(
                        egui::DragValue::new(stiffness_ratio)
                            .speed(0.02)
                            .range(0.01..=100.0)
                            .prefix("gap stiffness ")
                            .update_while_editing(false),
                    )
                    .changed();
                ui.small("A coupled law replaces both independent face conditions.");
            }
            if changed && let Some(coupling) = coupling {
                let result = self
                    .editor
                    .set_internal_boundary_couplings(&spans, coupling);
                self.error(result);
            }
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
        egui::ComboBox::from_label("Condition")
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
        if enabled {
            if !typing && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
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
                if drag.is_some() || self.editor.editing() {
                    self.editor.cancel();
                } else {
                    self.custom.clear();
                    self.mode = Mode::Select;
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
                if self.mode == Mode::Select
                    && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::A))
                {
                    self.select_filtered();
                }
                if self.mode == Mode::Custom {
                    if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.finish_custom();
                    }
                    if ctx.input(|i| i.key_pressed(egui::Key::Backspace)) {
                        self.custom.pop();
                    }
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
                let middle = ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Middle));
                let space = ctx.input(|i| i.key_down(egui::Key::Space));
                if middle || primary && space {
                    self.panning = true;
                } else if primary && self.mode == Mode::Select {
                    self.refresh_curves();
                    let modifiers = ctx.input(|input| input.modifiers);
                    if let Some((id, index)) = self.hit_handle(p, r) {
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
                                self.toggle_span(span);
                            } else if self.selected_spans.contains(&span) {
                                self.pending_span_collapse = Some(span);
                            } else {
                                self.set_span_selection(vec![span]);
                            }
                            if !modifiers.shift
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
                            self.pending_span_collapse = None;
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
                    let snap_to_grid = self.snap_to_grid;
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
                    && self.mode == Mode::Select
                    && !space
                    && self.hit_handle(p, r).is_none()
                    && self.hit_internal_handle(p, r).is_none()
                {
                    self.editor.commit();
                    self.drag = None;
                    if let Some((id, t)) = self.hit_curve(p, r) {
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
                    match self.mode {
                        Mode::Preset => {
                            let center = self.world(p, r);
                            if self.creation_role == CreationRole::InternalBoundary {
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
                                    self.mode = Mode::Select;
                                }
                            } else {
                                let result = self.create_spline(
                                    PeriodicCubicSpline::rounded(center, 0.15),
                                    center,
                                );
                                if let Some(id) = self.error(result) {
                                    self.select_loop(id);
                                    self.mode = Mode::Select;
                                }
                            }
                        }
                        Mode::Custom => {
                            if self.creation_role != CreationRole::InternalBoundary
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
                        Mode::Pulse => {
                            self.wave_pending_pulse = Some(self.world(p, r));
                        }
                        Mode::Source => {
                            self.wave_source.position = self.world(p, r);
                            self.wave_source.region = self
                                .wave_mesh
                                .as_ref()
                                .and_then(|mesh| mesh_region_at(mesh, self.wave_source.position))
                                .unwrap_or(RegionId(0));
                            self.wave_source_dirty = true;
                        }
                        Mode::Select => {}
                    }
                }
            }
        }
        if !ctx.input(|i| i.pointer.primary_down()) {
            let drag = self.drag.take();
            let moved = matches!(
                &drag,
                Some(Drag::Translate { moved: true, .. } | Drag::Rotate { moved: true, .. })
            );
            if matches!(&drag, Some(Drag::Translate { .. } | Drag::Rotate { .. })) {
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
            if !moved && let Some(span) = self.pending_span_collapse.take() {
                self.set_span_selection(vec![span]);
            } else if moved {
                self.pending_span_collapse = None;
            }
            if !ctx.input(|i| i.pointer.button_down(egui::PointerButton::Middle)) {
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
            && let (Some(operator), Some(display)) = (&self.wave_operator, wave_display)
            && display.generation > 0
            && display.current.len() == operator.degrees_of_freedom()
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
                Stroke::new(3.5, TEAL),
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
                GeometrySpan::Outer(_) => {}
            }
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
                                color
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
                        Stroke::new(1.5, if selected { color } else { GOLD }),
                    );
                }
            }
        }
        if self.transformable_curve_controls().is_some()
            && let Some(pivot) = self.selection_pivot()
        {
            let center = self.screen(pivot, r);
            let radius = self.gizmo_radius(r, pivot);
            painter.circle_stroke(center, radius, Stroke::new(1.5, TEAL));
            painter.circle_filled(center + egui::vec2(radius, 0.0), 4.0, TEAL);
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
                MarqueeOperation::Replace => TEAL,
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
        r
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
            + 18.0
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
        for segment in curve.samples.windows(2) {
            let parameter = 0.5 * (segment[0].t + segment[1].t);
            if parameter >= bounds[0] && parameter <= bounds[1] {
                painter.line_segment(
                    [
                        self.screen(segment[0].point, r),
                        self.screen(segment[1].point, r),
                    ],
                    Stroke::new(4.0, Color32::WHITE),
                );
            }
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

pub fn frame(
    mut contexts: EguiContexts,
    mut state: ResMut<Playground>,
    time: Res<Time>,
    mut request: ResMut<WaveGpuRequest>,
    display: Res<WaveDisplay>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut commands: Commands,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    if !state.ready {
        ctx.set_visuals(egui::Visuals::dark());
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
    state.refresh_wave(
        &mut request,
        &display,
        &mut assets,
        &mut commands,
        time.delta_secs_f64(),
    );
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
        });
    state
}

#[cfg(not(target_arch = "wasm32"))]
pub fn wave_gpu_check_scene() -> Playground {
    let mut state = Playground {
        automated_benchmark: true,
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
        });
    state
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
pub fn wave_gpu_benchmark(
    mut benchmark: ResMut<WaveGpuBenchmark>,
    mut state: ResMut<Playground>,
    mut request: ResMut<WaveGpuRequest>,
    display: Res<WaveDisplay>,
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
        for _ in 0..128 {
            cpu.step(operator, &[]).unwrap();
        }
        benchmark.expected_current = cpu.current().to_vec();
        benchmark.expected_previous = cpu.previous().to_vec();
        benchmark.expected_auxiliary = cpu.auxiliary().to_vec();
        benchmark.generation = request.generation();
        benchmark.prepared = true;
        benchmark.solve_started = Some(Instant::now());
        request.request_steps(128);
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
    if request.stats().completed_steps() < 128
        || display.completed_steps < 128
        || display.generation != benchmark.generation
        || display.current.len() != benchmark.expected_current.len()
        || display.auxiliary.len() != benchmark.expected_auxiliary.len()
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
        simulated_seconds_per_wall_second = 128.0 * state.wave_time_step / solve_seconds,
        "Wave GPU check complete"
    );
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
            if !request.ready() || state.wave_operator.is_none() {
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
        _ => {
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
                exit.write(bevy::app::AppExit::Success);
            } else {
                error!("Material GPU transaction differs from the f64 reference");
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
                    if state.mesh_job.is_some() {
                        ui.colored_label(GOLD, "Mesh work in progress");
                    } else if let Some(candidate) = &state.simulation_candidate {
                        ui.colored_label(
                            GOLD,
                            format!(
                                "Handoff in progress · {} vertices · {} triangles",
                                candidate.mesh.vertices.len(),
                                candidate.mesh.triangles.len()
                            ),
                        );
                    } else if state.mesh_error.is_some() || state.wave_error.is_some() {
                        ui.colored_label(RED, "Attention required");
                    }
                    if !state.message.is_empty() {
                        ui.colored_label(GOLD, &state.message);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let warning = state.performance_warning();
                        let summary = state.performance_summary();
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
        state.performance_window(root.ctx());
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(root, |ui| state.viewport(ui, wave_display))
            .inner
    }
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
        harness.state.mode = Mode::Custom;
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
        assert_eq!(harness.state.internal_selection, Some((id, None)));
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
        assert_eq!(harness.state.internal_selection, Some((id, None)));
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
        h.state.mode = Mode::Preset;
        h.click(h.point(Point2::new(0.5, 0.4)));
        assert_eq!(h.state.editor.document.draft.obstacles.len(), 2);
        assert!(h.state.mode == Mode::Select);
        assert_eq!(h.state.editor.history_len().0, 1);
        h.state.mode = Mode::Custom;
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
        h.state.mode = Mode::Custom;
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
        for label in ["Edit", "View", "Simulation", "Materials", "+ Add"] {
            assert!(
                h.texts.iter().any(|(text, _)| text == label),
                "missing {label}"
            );
        }
        assert!(!h.texts.iter().any(|(text, _)| text == "Revert draft"));
        h.click_text("+ Add");
        assert_eq!(h.state.active_tool, ActiveTool::AddGeometry);
        assert!(h.state.add_geometry_open);
        h.frame(vec![]);
        assert!(h.texts.iter().any(|(text, _)| text == "Add geometry"));
        assert!(h.texts.iter().any(|(text, _)| text == "Rounded loop"));
        assert!(h.texts.iter().any(|(text, _)| text == "Custom"));
        h.click_text("Baffle");
        assert!(matches!(
            h.state.creation_role,
            CreationRole::InternalBoundary
        ));
        assert!(h.texts.iter().any(|(text, _)| text == "Straight"));
        assert!(!h.texts.iter().any(|(text, _)| text == "Rounded loop"));
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
        h.state.active_tool = ActiveTool::Simulation;
        h.frame(vec![]);
        assert!(
            h.texts
                .iter()
                .any(|(text, _)| text.starts_with("Handoff in progress"))
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
        h.state.message = "Simulation mesh committed".into();
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
        assert!(summary.contains("N"));
        assert!(summary.contains("mesh"));
        assert!(summary.contains("dt"));
    }

    #[test]
    fn panning_and_panel_capture_never_edit_geometry() {
        let mut h = Harness::new();
        let before = h.state.editor.document.clone();
        let center = h.state.center;
        let p = h.rect.center();
        h.button(p, PointerButton::Middle, true);
        h.move_to(p + egui::vec2(40.0, 20.0));
        h.button(p + egui::vec2(40.0, 20.0), PointerButton::Middle, false);
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
        h.state.mode = Mode::Custom;
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
        h.state.mode = Mode::Custom;
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
    fn pulse_and_source_tools_only_change_transient_simulation_input() {
        let mut h = Harness::new();
        let document = h.state.editor.document.clone();
        h.state.mode = Mode::Pulse;
        let pulse = Point2::new(0.45, -0.3);
        h.click(h.point(pulse));
        assert!((h.state.wave_pending_pulse.unwrap() - pulse).norm() < 1.0e-6);
        assert!(h.state.mode == Mode::Pulse);
        let second_pulse = Point2::new(0.2, -0.1);
        h.click(h.point(second_pulse));
        assert!((h.state.wave_pending_pulse.unwrap() - second_pulse).norm() < 1.0e-6);
        h.state.mode = Mode::Source;
        let source = Point2::new(-0.55, 0.25);
        h.click(h.point(source));
        assert!((h.state.wave_source.position - source).norm() < 1.0e-6);
        assert!(h.state.wave_source_dirty);
        assert!(h.state.mode == Mode::Source);
        assert_eq!(h.state.editor.document, document);
        assert_eq!(h.state.editor.history_len(), (0, 0));

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
    fn moved_baffle_rebuild_prepares_a_face_aware_field_transfer() {
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
}
