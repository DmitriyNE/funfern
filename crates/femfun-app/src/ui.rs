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
use femfun_app::{
    editor::{Acceptance, Editor},
    persistence::{self, LoadCandidate},
};
use femfun_core::*;
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
struct Drag {
    id: ObstacleId,
    index: usize,
    offset: Point2,
}
struct InternalDrag {
    id: InternalBoundaryId,
    index: usize,
    offset: Point2,
}
struct Curve {
    id: ObstacleId,
    samples: Vec<Sample>,
}
struct InternalCurve {
    id: InternalBoundaryId,
    samples: Vec<Sample>,
}
struct SimulationCandidate {
    mesh: Arc<TriMesh>,
    scene: Scene,
    max_edge: f64,
    low_quality: Vec<bool>,
    operator: Arc<QuadraticWaveOperator>,
    boundary: OuterBoundaryCondition,
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
    mode: Mode,
    creation_role: CreationRole,
    material_selection: MaterialId,
    region_selection: RegionId,
    selection: Option<(ObstacleId, Option<usize>)>,
    internal_selection: Option<(InternalBoundaryId, Option<usize>)>,
    internal_span_selection: Option<(InternalBoundaryId, usize)>,
    internal_face_selection: InternalBoundarySide,
    custom: Vec<Point2>,
    drag: Option<Drag>,
    internal_drag: Option<InternalDrag>,
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
    wave_boundary: OuterBoundaryCondition,
    wave_boundary_committed: OuterBoundaryCondition,
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
            mode: Mode::Select,
            creation_role: CreationRole::Hole,
            material_selection: DEFAULT_MATERIAL,
            region_selection: BACKGROUND_REGION,
            selection: Some((ObstacleId(1), None)),
            internal_selection: None,
            internal_span_selection: None,
            internal_face_selection: InternalBoundarySide::Left,
            custom: vec![],
            drag: None,
            internal_drag: None,
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
            wave_boundary: OuterBoundaryCondition::Reflecting,
            wave_boundary_committed: OuterBoundaryCondition::Reflecting,
            wave_time_step: 0.0,
            wave_time_offset: 0.0,
            wave_running: false,
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
        self.internal_span_selection = None;
        self.region_selection = BACKGROUND_REGION;
        self.drag = None;
        self.internal_drag = None;
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
                self.selection = None;
                self.internal_selection = Some((id, None));
                self.internal_span_selection = Some((id, 0));
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
            self.selection = Some((id, None));
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
                    match QuadraticWaveOperator::assemble_scene(
                        &mesh,
                        &self.mesh_source,
                        self.wave_boundary,
                    ) {
                        Ok(operator) => {
                            let transfer =
                                if self.mesh_committed_scene.internal_boundaries.is_empty()
                                    && self.mesh_source.internal_boundaries.is_empty()
                                {
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
                                } else {
                                    Ok(None)
                                };
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
                                        boundary: self.wave_boundary,
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
            && (self.wave_boundary != self.wave_boundary_committed
                || self.editor.document.accepted != self.mesh_committed_scene)
            && let (Some(mesh), Some(source_operator)) =
                (self.wave_mesh.as_ref(), self.wave_operator.as_ref())
        {
            let prepare = Instant::now();
            match QuadraticWaveOperator::assemble_scene(
                mesh,
                &self.editor.document.accepted,
                self.wave_boundary,
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
                                boundary: self.wave_boundary,
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
            let material_changed = candidate.scene != self.mesh_committed_scene;
            let boundary_changed = candidate.boundary != self.wave_boundary_committed;
            let commit_message = if same_mesh && material_changed && boundary_changed {
                "Materials and outer boundary committed; live field preserved".into()
            } else if same_mesh && material_changed {
                "Materials committed; live field preserved".into()
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
        self.wave_completed_steps = request.stats().completed_steps();
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
    fn panel(&mut self, ui: &mut egui::Ui) {
        if self.automated_benchmark {
            ui.label("Automated mesh benchmark");
            ui.disable();
        }
        ui.add_space(10.0);
        ui.heading(egui::RichText::new("femfun").size(29.0).color(TEAL));
        ui.label(
            egui::RichText::new("GEOMETRY PLAYGROUND")
                .small()
                .color(Color32::GRAY),
        );
        ui.add_space(14.0);
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
        ui.label("Create geometry");
        egui::ComboBox::from_id_salt("creation_role")
            .selected_text(match self.creation_role {
                CreationRole::Hole => "Hole",
                CreationRole::MaterialInterface => "Material interface",
                CreationRole::InternalBoundary => "Reflecting baffle",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.creation_role, CreationRole::Hole, "Hole");
                ui.selectable_value(
                    &mut self.creation_role,
                    CreationRole::MaterialInterface,
                    "Material interface",
                );
                ui.selectable_value(
                    &mut self.creation_role,
                    CreationRole::InternalBoundary,
                    "Reflecting baffle",
                );
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
        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.mode == Mode::Select, "Select")
                .clicked()
            {
                self.mode = Mode::Select;
                self.custom.clear();
            }
            if ui
                .selectable_label(self.mode == Mode::Preset, "Rounded")
                .clicked()
            {
                self.mode = Mode::Preset;
                self.custom.clear();
            }
            if ui
                .selectable_label(self.mode == Mode::Custom, "Custom")
                .clicked()
            {
                self.mode = Mode::Custom;
                self.custom.clear();
            }
        });
        ui.small(match self.mode {
            Mode::Select => "Drag handles · double-click a curve to insert",
            Mode::Preset if self.creation_role == CreationRole::InternalBoundary => {
                "Click the viewport to place a length 0.5 reflecting baffle"
            }
            Mode::Preset => "Click the viewport to place a radius 0.15 loop",
            Mode::Custom if self.creation_role == CreationRole::InternalBoundary => {
                "Click control points; Enter finishes the open curve"
            }
            Mode::Custom => "Click control points; Enter or first point closes",
            Mode::Pulse => "Click the viewport to add a zero-velocity pulse",
            Mode::Source => "Click the viewport to move the continuous source",
        });
        if self.mode == Mode::Custom {
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
                for o in &self.editor.document.draft.obstacles {
                    if ui
                        .selectable_label(
                            self.selection.is_some_and(|s| s.0 == o.id),
                            format!(
                                "Loop {:02} · {} · {} controls",
                                o.id.0,
                                o.role.label(),
                                o.spline.controls().len()
                            ),
                        )
                        .clicked()
                    {
                        self.selection = Some((o.id, None));
                        self.internal_selection = None;
                        self.internal_span_selection = None;
                    }
                }
                for boundary in &self.editor.document.draft.internal_boundaries {
                    if ui
                        .selectable_label(
                            self.internal_selection
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
                        self.selection = None;
                        self.internal_selection = Some((boundary.id, None));
                        self.internal_span_selection = Some((boundary.id, 0));
                    }
                }
            });
        if let Some((id, index)) = self.selection {
            if ui.button("Delete loop").clicked() {
                self.editor.delete_obstacle(id);
                self.selection = None;
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
                let can_remove = self
                    .editor
                    .obstacle(id)
                    .is_some_and(|o| o.spline.controls().len() > 4);
                if ui
                    .add_enabled(
                        can_remove,
                        egui::Button::new("Remove control · reshapes curve"),
                    )
                    .clicked()
                {
                    let result = self.editor.remove_point(id, index);
                    if self.error(result).is_some() {
                        self.selection = Some((id, None));
                    }
                }
            }
        }
        if let Some((id, index)) = self.internal_selection {
            if ui.button("Delete baffle").clicked() {
                self.editor.delete_internal_boundary(id);
                self.internal_selection = None;
                self.internal_span_selection = None;
            }
            if let Some(boundary) = self.editor.internal_boundary(id) {
                let span_count = boundary.span_laws.len();
                let mut span = self
                    .internal_span_selection
                    .filter(|selection| selection.0 == id && selection.1 < span_count)
                    .map_or(0, |selection| selection.1);
                egui::ComboBox::from_id_salt(("baffle_span", id.0))
                    .selected_text(format!("Span {} / {span_count}", span + 1))
                    .show_ui(ui, |ui| {
                        for candidate in 0..span_count {
                            ui.selectable_value(
                                &mut span,
                                candidate,
                                format!("Span {}", candidate + 1),
                            );
                        }
                    });
                self.internal_span_selection = Some((id, span));
                ui.horizontal(|ui| {
                    ui.label("Face");
                    ui.selectable_value(
                        &mut self.internal_face_selection,
                        InternalBoundarySide::Left,
                        "Left",
                    );
                    ui.selectable_value(
                        &mut self.internal_face_selection,
                        InternalBoundarySide::Right,
                        "Right",
                    );
                });
                ui.small("Left/right follow the spline start → end direction");
                let mut law = boundary.span_laws[span];
                let face = match self.internal_face_selection {
                    InternalBoundarySide::Left => &mut law.left,
                    InternalBoundarySide::Right => &mut law.right,
                };
                let mut impedance = matches!(face, FaceBoundaryCondition::Impedance { .. });
                if ui
                    .checkbox(&mut impedance, "Matched impedance face")
                    .changed()
                {
                    *face = if impedance {
                        FaceBoundaryCondition::Impedance { ratio: 1.0 }
                    } else {
                        FaceBoundaryCondition::Reflecting
                    };
                    let result = self.editor.set_internal_boundary_law(id, span, law);
                    self.error(result);
                } else if let FaceBoundaryCondition::Impedance { ratio } = face {
                    let response = ui.add(
                        egui::DragValue::new(ratio)
                            .speed(0.02)
                            .range(0.01..=100.0)
                            .prefix("impedance ratio ")
                            .update_while_editing(false),
                    );
                    if response.changed() {
                        let result = self.editor.set_internal_boundary_law(id, span, law);
                        self.error(result);
                    }
                    ui.small("1.0 matches the adjacent medium");
                }
                let mut thin_gap = matches!(law.coupling, InternalBoundaryCoupling::ThinGap { .. });
                if ui.checkbox(&mut thin_gap, "Couple as thin gap").changed() {
                    law.coupling = if thin_gap {
                        InternalBoundaryCoupling::ThinGap {
                            stiffness_ratio: 1.0,
                        }
                    } else {
                        InternalBoundaryCoupling::Independent
                    };
                    let result = self.editor.set_internal_boundary_law(id, span, law);
                    self.error(result);
                } else if let InternalBoundaryCoupling::ThinGap { stiffness_ratio } =
                    &mut law.coupling
                {
                    let response = ui.add(
                        egui::DragValue::new(stiffness_ratio)
                            .speed(0.02)
                            .range(0.01..=100.0)
                            .prefix("gap stiffness ")
                            .update_while_editing(false),
                    );
                    if response.changed() {
                        let result = self.editor.set_internal_boundary_law(id, span, law);
                        self.error(result);
                    }
                    ui.small("Conservative paired-trace spring · tighter gaps reduce dt");
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
                let can_remove = self
                    .editor
                    .internal_boundary(id)
                    .is_some_and(|boundary| boundary.spline.controls().len() > 4);
                if ui
                    .add_enabled(
                        can_remove,
                        egui::Button::new("Remove control · reshapes curve"),
                    )
                    .clicked()
                {
                    let result = self.editor.remove_internal_boundary_point(id, index);
                    if self.error(result).is_some() {
                        self.internal_selection = Some((id, None));
                    }
                }
            }
        }
        ui.add_space(8.0);
        ui.collapsing("Regions and materials", |ui| {
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
                        ui.selectable_value(
                            &mut self.material_selection,
                            material.id,
                            &material.name,
                        );
                    }
                });
            if let Some(mut material) = self
                .editor
                .document
                .draft
                .material(self.material_selection)
                .cloned()
            {
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
                if self.material_selection != DEFAULT_MATERIAL
                    && ui.small_button("Delete unused material").clicked()
                {
                    let result = self.editor.delete_material(self.material_selection);
                    if self.error(result).is_some() {
                        self.material_selection = DEFAULT_MATERIAL;
                    }
                }
            }
            ui.small("Interfaces share a trace; closed walls keep separate traces.");
        });
        ui.add_space(8.0);
        ui.separator();
        ui.horizontal(|ui| {
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
            if ui.button("Fit View").clicked() {
                self.fit = true;
            }
        });
        if ui
            .add_enabled(
                self.editor.document.draft != self.editor.document.accepted,
                egui::Button::new("Revert Draft"),
            )
            .clicked()
        {
            self.editor.revert();
            self.clear_transient();
        }
        ui.horizontal(|ui| {
            if ui.button("Save scene").clicked() {
                self.editor.commit();
                match persistence::save(&self.editor.document) {
                    Ok(json) => {
                        files::save(self.sender.clone(), json.into_bytes());
                        self.file_busy = true;
                    }
                    Err(e) => self.message = e,
                }
            }
            if ui.button("Load scene").clicked() {
                self.editor.commit();
                files::load(self.sender.clone());
                self.file_busy = true;
            }
        });
        ui.add_space(8.0);
        ui.separator();
        ui.collapsing("Display", |ui| {
            ui.checkbox(&mut self.grid, "Grid");
            ui.checkbox(&mut self.polygon, "Control polygons");
            ui.checkbox(&mut self.handles, "Handles");
            ui.checkbox(&mut self.reference, "Accepted reference");
            ui.checkbox(&mut self.show_materials, "Material regions");
            ui.checkbox(&mut self.show_mesh, "Accepted triangle mesh");
            ui.add_enabled_ui(self.show_mesh, |ui| {
                ui.checkbox(&mut self.show_mesh_boundary, "Mesh boundary labels");
            });
        });
        ui.add_space(8.0);
        ui.separator();
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
                ui.selectable_value(&mut self.mesh_max_edge, 0.04, "Fine P2e · parent h ≤ 0.04");
            });
        ui.small("Seven-node enriched quadratic wave basis.");
        if self.editor.editing() && self.mesh_source != self.editor.document.accepted {
            ui.small("Waiting for edit to finish…");
        } else if let Some(job) = &self.mesh_job {
            ui.small(format!("{}…", job.phase()));
        } else if let Some(candidate) = &self.simulation_candidate {
            ui.small(format!(
                "Committing candidate · {} vertices · {} triangles",
                candidate.mesh.vertices.len(),
                candidate.mesh.triangles.len()
            ));
        } else if let Some(error) = &self.mesh_error {
            ui.colored_label(RED, error);
            ui.small("Previous mesh retained.");
        } else if let Some(mesh) = &self.mesh {
            let low_quality = self.mesh_low_quality.iter().filter(|poor| **poor).count();
            ui.small(format!(
                "{} vertices · {} triangles\nmin angle {:.1}° · max edge {:.3}\n{} elements below 15°",
                mesh.vertices.len(),
                mesh.triangles.len(),
                mesh.quality.minimum_angle_degrees,
                mesh.quality.maximum_edge_length,
                low_quality
            ));
        } else {
            ui.small("Preparing…");
        }
        if self.mesh_started.is_some() {
            ui.small(format!(
                "Request → ready {:.0} ms\nActive mesh {:.1} ms · between slices {:.1} ms\nLongest mesh slice {:.2} ms (2 ms target)",
                self.mesh_build_ms, self.mesh_work_ms, (self.mesh_build_ms-self.mesh_work_ms).max(0.0), self.mesh_max_slice_ms
            ));
        }
        if let Some(report) = self
            .mesh_job
            .as_ref()
            .map(|job| job.report())
            .or(self.mesh_report.as_ref())
        {
            if report.used_local {
                ui.small(format!("Local repair · {:.1}% of previous elements unchanged\n{} moved vertices · {} inserted · {} collapsed",
                    100.0 * report.preserved_triangles as f64 / report.original_triangles.max(1) as f64,
                    report.moved_vertices, report.inserted_vertices, report.collapsed_vertices));
            }
            if let Some(reason) = &report.fallback_reason {
                ui.small(format!("Full rebuild: {reason}"));
            }
        }
        if self.mesh_attempts > 0 {
            ui.small(format!(
                "Full fallbacks: {} / {} completed edits",
                self.mesh_fallbacks, self.mesh_attempts
            ));
        }
        ui.add_space(8.0);
        ui.label("Wave simulation");
        let wave_available = self.wave_operator.is_some();
        ui.add_enabled_ui(wave_available, |ui| {
            egui::ComboBox::from_label("Outer boundary")
                .selected_text(self.wave_boundary.label())
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.wave_boundary,
                        OuterBoundaryCondition::Reflecting,
                        "Reflecting",
                    );
                    ui.selectable_value(
                        &mut self.wave_boundary,
                        OuterBoundaryCondition::FirstOrderOutgoing,
                        "First-order outgoing",
                    );
                    ui.selectable_value(
                        &mut self.wave_boundary,
                        OuterBoundaryCondition::SecondOrderOutgoing,
                        "Second-order auxiliary",
                    );
                });
            ui.horizontal(|ui| {
                if ui
                    .button(if self.wave_running { "Pause" } else { "Run" })
                    .clicked()
                {
                    self.wave_running = !self.wave_running;
                }
                if ui.button("Step").clicked() {
                    self.wave_step_requested = true;
                }
                if ui.button("Reset").clicked() {
                    self.wave_reset_requested = true;
                }
            });
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(self.mode == Mode::Pulse, "Place pulse")
                    .clicked()
                {
                    self.mode = Mode::Pulse;
                }
                if ui
                    .selectable_label(self.mode == Mode::Source, "Move source")
                    .clicked()
                {
                    self.mode = Mode::Source;
                }
            });
            ui.checkbox(&mut self.show_field, "Field colors");
            ui.add(
                egui::Slider::new(&mut self.field_gain, 0.25..=12.0)
                    .logarithmic(true)
                    .text("color gain"),
            );
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
            let simulated_time =
                self.wave_time_offset + self.wave_completed_steps as f64 * self.wave_time_step;
            let throughput = if self.wave_active_wall_seconds > 0.0 {
                simulated_time / self.wave_active_wall_seconds
            } else {
                0.0
            };
            let gpu_bytes = operator.estimated_gpu_bytes() as f64;
            ui.small(format!(
                "GPU {} · {} DOFs · {:.2} MiB\ndt {:.6} · t {:.3} · {} substeps/frame\n{:.2} simulated s / wall s · operator/map {:.1} ms",
                self.wave_gpu_status,
                operator.degrees_of_freedom(),
                gpu_bytes / (1024.0 * 1024.0),
                self.wave_time_step,
                simulated_time,
                self.wave_substeps_last,
                throughput,
                self.wave_prepare_ms,
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
        ui.add_space(12.0);
        ui.small("Control points guide the spline; the curve does not pass through them.");
        ui.add_space(6.0);
        ui.small("Pan: middle drag / Space + drag\nZoom: wheel over viewport\nUndo: Ctrl/Cmd + Z · Shift for redo");
        if !self.message.is_empty() {
            ui.add_space(8.0);
            ui.colored_label(GOLD, &self.message);
        }
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
                if self.drag.take().is_some()
                    || self.internal_drag.take().is_some()
                    || self.editor.editing()
                {
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
                        self.selection = Some((id, None));
                    }
                } else if ctx.input(|i| i.key_pressed(egui::Key::Delete))
                    && let Some((id, Some(index))) = self.internal_selection
                {
                    let result = self.editor.remove_internal_boundary_point(id, index);
                    if self.error(result).is_some() {
                        self.internal_selection = Some((id, None));
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
                if wheel != 0.0
                    && self.drag.is_none()
                    && self.internal_drag.is_none()
                    && !self.panning
                {
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
                    if let Some((id, index)) = self.hit_handle(p, r) {
                        self.selection = Some((id, Some(index)));
                        self.internal_selection = None;
                        self.internal_span_selection = None;
                        self.editor.begin();
                        let point = self.editor.obstacle(id).unwrap().spline.controls()[index];
                        self.drag = Some(Drag {
                            id,
                            index,
                            offset: point - self.world(p, r),
                        });
                    } else if let Some((id, index)) = self.hit_internal_handle(p, r) {
                        self.selection = None;
                        self.internal_selection = Some((id, Some(index)));
                        self.internal_span_selection = None;
                        self.editor.begin();
                        let point =
                            self.editor.internal_boundary(id).unwrap().spline.controls()[index];
                        self.internal_drag = Some(InternalDrag {
                            id,
                            index,
                            offset: point - self.world(p, r),
                        });
                    } else {
                        self.selection = self.hit_curve(p, r).map(|(id, _)| (id, None));
                        let internal_hit = if self.selection.is_none() {
                            self.hit_internal_curve(p, r)
                        } else {
                            None
                        };
                        self.internal_selection = internal_hit.map(|(id, _)| (id, None));
                        self.internal_span_selection = internal_hit.and_then(|(id, parameter)| {
                            self.editor
                                .internal_boundary(id)
                                .and_then(|boundary| boundary.spline.span_index(parameter))
                                .map(|span| (id, span))
                        });
                        if self.selection.is_none() && self.internal_selection.is_none() {
                            self.region_selection = self.region_at(self.world(p, r));
                        }
                    }
                }
                if self.panning {
                    let delta = ctx.input(|i| i.pointer.delta());
                    self.center = self.center
                        + Point2::new(-delta.x as f64 / self.scale, delta.y as f64 / self.scale);
                } else if let Some(drag) = &self.drag
                    && response.dragged_by(egui::PointerButton::Primary)
                {
                    let result =
                        self.editor
                            .set_point(drag.id, drag.index, self.world(p, r) + drag.offset);
                    self.error(result);
                } else if let Some(drag) = &self.internal_drag
                    && response.dragged_by(egui::PointerButton::Primary)
                {
                    let result = self.editor.set_internal_boundary_point(
                        drag.id,
                        drag.index,
                        self.world(p, r) + drag.offset,
                    );
                    self.error(result);
                }
                if response.double_clicked()
                    && self.mode == Mode::Select
                    && !space
                    && self.hit_handle(p, r).is_none()
                    && self.hit_internal_handle(p, r).is_none()
                {
                    self.editor.commit();
                    self.drag = None;
                    self.internal_drag = None;
                    if let Some((id, t)) = self.hit_curve(p, r) {
                        let result = self.editor.insert(id, t);
                        if let Some(index) = self.error(result) {
                            self.selection = Some((id, Some(index)));
                            self.internal_selection = None;
                            self.internal_span_selection = None;
                        }
                    } else if let Some((id, parameter)) = self.hit_internal_curve(p, r) {
                        let result = self.editor.insert_internal_boundary(id, parameter);
                        if let Some(index) = self.error(result) {
                            self.selection = None;
                            self.internal_selection = Some((id, Some(index)));
                            self.internal_span_selection = None;
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
                                    self.selection = None;
                                    self.internal_selection = Some((id, None));
                                    self.internal_span_selection = Some((id, 0));
                                    self.mode = Mode::Select;
                                }
                            } else {
                                let result = self.create_spline(
                                    PeriodicCubicSpline::rounded(center, 0.15),
                                    center,
                                );
                                if let Some(id) = self.error(result) {
                                    self.selection = Some((id, None));
                                    self.internal_selection = None;
                                    self.internal_span_selection = None;
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
                            self.mode = Mode::Select;
                        }
                        Mode::Source => {
                            self.wave_source.position = self.world(p, r);
                            self.wave_source.region = self
                                .wave_mesh
                                .as_ref()
                                .and_then(|mesh| mesh_region_at(mesh, self.wave_source.position))
                                .unwrap_or(RegionId(0));
                            self.wave_source_dirty = true;
                            self.mode = Mode::Select;
                        }
                        Mode::Select => {}
                    }
                }
            }
        }
        if !ctx.input(|i| i.pointer.primary_down()) {
            if self.drag.take().is_some() || self.internal_drag.take().is_some() {
                self.editor.commit();
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
        if let Some((id, span)) = self.internal_span_selection
            && let Some(curve) = self
                .draft_internal_curves
                .iter()
                .find(|curve| curve.id == id)
            && let Some(bounds) = self
                .editor
                .internal_boundary(id)
                .and_then(|boundary| boundary.spline.span_bounds(span))
        {
            self.draw_internal_span_face(&painter, r, curve, bounds, self.internal_face_selection);
        }
        for o in &self.editor.document.draft.obstacles {
            let selected = self.selection.is_some_and(|s| s.0 == o.id);
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
            let selected = self
                .internal_selection
                .is_some_and(|selection| selection.0 == boundary.id);
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
        painter.text(
            r.left_top() + egui::vec2(18.0, 16.0),
            egui::Align2::LEFT_TOP,
            "FIXED DOMAIN  /  [−1, 1]²",
            egui::FontId::monospace(11.0),
            Color32::from_rgb(124, 145, 161),
        );
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

    fn draw_internal_span_face(
        &self,
        painter: &egui::Painter,
        r: Rect,
        curve: &InternalCurve,
        bounds: [f64; 2],
        side: InternalBoundarySide,
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
                Stroke::new(3.0, Color32::WHITE),
            );
        }
    }
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
        .replace_validated(femfun_app::editor::Document {
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
        obstacles: vec![Obstacle {
            id: ObstacleId(1),
            spline: PeriodicCubicSpline::rounded(Point2::default(), 0.30),
            role: LoopRole::Wall {
                exterior: BACKGROUND_REGION,
                interior: RegionId(2),
            },
        }],
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
                left: FaceBoundaryCondition::Impedance { ratio: 0.7 },
                coupling: InternalBoundaryCoupling::ThinGap {
                    stiffness_ratio: 2.0,
                },
                ..InternalBoundaryLaw::REFLECTING
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
    };
    state
        .editor
        .replace_validated(femfun_app::editor::Document {
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
        if state.wave_boundary != OuterBoundaryCondition::SecondOrderOutgoing {
            state.wave_boundary = OuterBoundaryCondition::SecondOrderOutgoing;
            return;
        }
        if state.wave_boundary_committed != OuterBoundaryCondition::SecondOrderOutgoing
            || state.simulation_candidate.is_some()
        {
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
        info!(
            dofs = operator.degrees_of_freedom(),
            dt = state.wave_time_step,
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
            if state.wave_boundary != OuterBoundaryCondition::SecondOrderOutgoing {
                state.wave_boundary = OuterBoundaryCondition::SecondOrderOutgoing;
                return;
            }
            if state.wave_boundary_committed != OuterBoundaryCondition::SecondOrderOutgoing
                || state.simulation_candidate.is_some()
            {
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
            benchmark.source_velocity = benchmark
                .source_current
                .iter()
                .zip(&source_previous)
                .zip(stiffness)
                .zip(auxiliary)
                .zip(operator.lumped_mass())
                .zip(operator.lumped_damping())
                .map(
                    |(((((current, previous), ku), auxiliary), mass), damping)| {
                        centered_velocity(
                            *previous,
                            *current,
                            -(ku + auxiliary) / mass,
                            damping / mass,
                            state.wave_time_step,
                        )
                        .unwrap()
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
            let velocity = map.interpolate(&benchmark.source_velocity, 0.0).unwrap();
            benchmark.expected_auxiliary =
                map.interpolate(&benchmark.source_auxiliary, 0.0).unwrap();
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
                .map(
                    |(((((current, velocity), ku), auxiliary), mass), damping)| {
                        centered_previous(
                            *current,
                            velocity,
                            -(ku + auxiliary) / mass,
                            damping / mass,
                            candidate.time_step,
                        )
                        .unwrap()
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
            benchmark.source_velocity = benchmark
                .source_current
                .iter()
                .zip(&source_previous)
                .zip(stiffness)
                .zip(auxiliary)
                .zip(operator.lumped_mass())
                .zip(operator.lumped_damping())
                .map(
                    |(((((current, previous), ku), auxiliary), mass), damping)| {
                        centered_velocity(
                            *previous,
                            *current,
                            -(ku + auxiliary) / mass,
                            damping / mass,
                            state.wave_time_step,
                        )
                        .unwrap()
                    },
                )
                .collect();
            state.wave_boundary = OuterBoundaryCondition::FirstOrderOutgoing;
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
            if candidate.boundary != OuterBoundaryCondition::FirstOrderOutgoing
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
            let velocity = map.interpolate(&benchmark.source_velocity, 0.0).unwrap();
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
                .map(|((((current, velocity), ku), mass), damping)| {
                    centered_previous(
                        *current,
                        velocity,
                        -ku / mass,
                        damping / mass,
                        candidate.time_step,
                    )
                    .unwrap()
                })
                .collect();
            benchmark.expected_auxiliary = vec![0.0; candidate.operator.degrees_of_freedom()];
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
            benchmark.source_velocity = benchmark
                .source_current
                .iter()
                .zip(source_previous)
                .zip(stiffness)
                .zip(operator.lumped_mass())
                .zip(operator.lumped_damping())
                .map(|((((current, previous), ku), mass), damping)| {
                    centered_velocity(
                        previous,
                        *current,
                        -ku / mass,
                        damping / mass,
                        state.wave_time_step,
                    )
                    .unwrap()
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
            let velocity = map.interpolate(&benchmark.source_velocity, 0.0).unwrap();
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
                .map(|((((current, velocity), ku), mass), damping)| {
                    centered_previous(
                        *current,
                        velocity,
                        -ku / mass,
                        damping / mass,
                        candidate.time_step,
                    )
                    .unwrap()
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
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.small(format!(
                            "{:.1} ms  ·  {:.0} px/unit  ·  {}",
                            state.frame_ms,
                            state.scale,
                            if cfg!(target_arch = "wasm32") {
                                "WebGPU"
                            } else {
                                "wgpu"
                            }
                        ));
                    });
                });
            });
        egui::Panel::left("tools")
            .exact_size(280.0)
            .resizable(false)
            .show(root, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let enabled = !state.file_busy && state.load.is_none();
                    ui.add_enabled_ui(enabled, |ui| state.panel(ui));
                });
            });
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
        assert_eq!(harness.state.internal_span_selection, Some((id, 0)));

        let history_before_laws = harness.state.editor.history_len().0;
        harness.click_text("Matched impedance face");
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
        harness.click_text("Couple as thin gap");
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
        harness.state.internal_span_selection = None;
        let point = harness
            .state
            .editor
            .internal_boundary(id)
            .unwrap()
            .spline
            .evaluate(1.5);
        harness.click(harness.point(point));
        assert_eq!(harness.state.internal_selection, Some((id, None)));
        assert_eq!(harness.state.internal_span_selection, Some((id, 1)));
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
        h.wheel(Pos2::new(100.0, 500.0));
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
        assert!(h.state.mode == Mode::Select);
        h.state.mode = Mode::Source;
        let source = Point2::new(-0.55, 0.25);
        h.click(h.point(source));
        assert!((h.state.wave_source.position - source).norm() < 1.0e-6);
        assert!(h.state.wave_source_dirty);
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

        h.state.wave_boundary = OuterBoundaryCondition::FirstOrderOutgoing;
        h.state.refresh_mesh();

        let candidate = h
            .state
            .simulation_candidate
            .as_ref()
            .expect("boundary candidate");
        assert!(Arc::ptr_eq(&candidate.mesh, &mesh));
        assert!(h.state.mesh_job.is_none());
        assert_eq!(
            candidate.operator.outer_boundary(),
            OuterBoundaryCondition::FirstOrderOutgoing
        );
        assert_eq!(candidate.exposed_nodes, 0);
        assert!(candidate.transfer.is_some());

        commit_mesh_without_gpu(&mut h.state);
        assert_eq!(
            h.state.wave_boundary_committed,
            OuterBoundaryCondition::FirstOrderOutgoing
        );
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
                    coupling: InternalBoundaryCoupling::ThinGap {
                        stiffness_ratio: 2.0,
                    },
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
