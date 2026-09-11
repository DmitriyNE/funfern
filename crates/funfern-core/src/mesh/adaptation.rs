use super::*;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MeshUpdateFailureKind {
    IncompatibleGeometry,
    UnsupportedBoundaryTopology,
    MotionTooLarge,
    PatchTooLarge,
    FixedPatchBoundary,
    ElementInversion,
    BoundarySubdivisionLimit,
    RefinementLimit,
    WorkLimit,
    InvalidSource,
    InvalidGeometry,
    InvalidOptions,
    Capacity,
    BoundarySampling,
    PairedTraceMismatch,
    BaffleTipTopology,
    RepairFailure,
}

impl MeshUpdateFailureKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::IncompatibleGeometry => "geometry topology changed",
            Self::UnsupportedBoundaryTopology => "unsupported boundary topology",
            Self::MotionTooLarge => "motion exceeds local limit",
            Self::PatchTooLarge => "repair patch is too large",
            Self::FixedPatchBoundary => "repair reached the fixed patch boundary",
            Self::ElementInversion => "local motion inverted an element",
            Self::BoundarySubdivisionLimit => "boundary subdivision limit",
            Self::RefinementLimit => "local refinement limit",
            Self::WorkLimit => "local work limit",
            Self::InvalidSource => "invalid source mesh",
            Self::InvalidGeometry => "invalid target geometry",
            Self::InvalidOptions => "invalid meshing options",
            Self::Capacity => "mesh capacity",
            Self::BoundarySampling => "boundary sampling",
            Self::PairedTraceMismatch => "paired baffle trace mismatch",
            Self::BaffleTipTopology => "invalid baffle tip topology",
            Self::RepairFailure => "other local repair failure",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshUpdateFailure {
    pub kind: MeshUpdateFailureKind,
    pub detail: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshUpdateReport {
    pub local_attempted: bool,
    pub used_local: bool,
    pub repair_attempts: usize,
    pub retry_failures: Vec<MeshUpdateFailure>,
    pub fallback_failure: Option<MeshUpdateFailure>,
    pub original_triangles: usize,
    /// Same vertex identities AND exactly the same coordinates.
    pub preserved_triangles: usize,
    pub preserved_connectivity: usize,
    pub moved_vertices: usize,
    pub inserted_vertices: usize,
    pub collapsed_vertices: usize,
    pub repair_vertices: usize,
    pub repair_triangles: usize,
    pub repaired_baffles: usize,
    pub paired_trace_segments: usize,
}

pub struct MeshUpdateResult {
    pub mesh: TriMesh,
    pub report: MeshUpdateReport,
}

enum Phase {
    Validate(Box<ValidationJob>),
    Vertices(usize),
    Triangles(usize),
    Edges(usize),
    Region(usize),
    Expand {
        ring: usize,
        index: usize,
    },
    Patch(usize),
    Smooth {
        pass: usize,
        index: usize,
    },
    Move(usize),
    Check(usize),
    Boundary(usize),
    BaffleBoundary(usize),
    Coarsen(usize),
    Loops {
        loop_index: usize,
        current: Option<usize>,
    },
    Repair,
    CompactVertices(usize),
    CompactTriangles(usize),
    CompactEdges(usize),
    Full,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum MovingBoundaryId {
    Loop(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct TraceSegmentKey {
    id: InternalBoundaryId,
    start: u64,
    end: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TracePair {
    id: InternalBoundaryId,
    start: u64,
    end: u64,
    left: usize,
    right: usize,
}

/// Reuses a previously accepted mesh for control-coordinate changes with the
/// same spline knot vectors and meshing options. The source must be a mesh of
/// `previous_scene` produced by this mesher; scene identity is supplied by the
/// caller. Changing resolution should pass None. Failed repairs automatically
/// switch to full construction, leaving the shared source untouched.
///
/// Import/verification/compaction are linear passes over the mesh. Geometry and
/// topology changes are confined to a frozen patch, not the whole domain.
pub struct MeshUpdateJob {
    previous: Option<Arc<TriMesh>>,
    previous_scene: Scene,
    scene: Scene,
    revision: u64,
    mesh_revision: u64,
    options: MeshingOptions,
    phase: Phase,
    builder: Option<MeshBuilder>,
    job: Option<MeshingJob>,
    report: MeshUpdateReport,
    work: usize,
    displacement: Vec<Point2>,
    next_displacement: Vec<Point2>,
    smooth_vertices: Vec<usize>,
    allowed: Vec<bool>,
    next_allowed: Vec<bool>,
    radius: f64,
    next_boundary: Vec<Option<usize>>,
    boundary_seeds: Vec<Option<usize>>,
    old_keys: BTreeSet<[usize; 3]>,
    output: Option<TriMesh>,
    remap: Vec<usize>,
    compact_vertices: Vec<MeshVertex>,
    boundary_splits: usize,
    attempt: usize,
    moving_bounds: BTreeMap<MovingBoundaryId, [Point2; 2]>,
    trace_edges: BTreeMap<TraceSegmentKey, [Option<usize>; 2]>,
    trace_pairs: Vec<TracePair>,
    moved_baffles: BTreeSet<InternalBoundaryId>,
    moving_trace_segments: Vec<[Point2; 4]>,
}

fn key(mut vertices: [usize; 3]) -> [usize; 3] {
    vertices.sort();
    vertices
}

fn classify_failure(error: &MeshError) -> MeshUpdateFailure {
    let detail = error.to_string();
    let kind = match error {
        MeshError::InvalidOptions => MeshUpdateFailureKind::InvalidOptions,
        MeshError::InvalidGeometry(_) => MeshUpdateFailureKind::InvalidGeometry,
        MeshError::Sampling(_) => MeshUpdateFailureKind::BoundarySampling,
        MeshError::Capacity { .. } => MeshUpdateFailureKind::Capacity,
        MeshError::RefinementLimit(_) => MeshUpdateFailureKind::RefinementLimit,
        MeshError::Topology(reason) => {
            if reason.contains("paired baffle trace") {
                MeshUpdateFailureKind::PairedTraceMismatch
            } else if reason.contains("baffle tip") {
                MeshUpdateFailureKind::BaffleTipTopology
            } else if reason.contains("fixed patch boundary") || reason.contains("left the patch") {
                MeshUpdateFailureKind::FixedPatchBoundary
            } else if reason.contains("inverted") {
                MeshUpdateFailureKind::ElementInversion
            } else if reason.contains("subdivision limit") {
                MeshUpdateFailureKind::BoundarySubdivisionLimit
            } else if reason.contains("too much of the mesh") {
                MeshUpdateFailureKind::PatchTooLarge
            } else if reason.contains("motion exceeds") {
                MeshUpdateFailureKind::MotionTooLarge
            } else if reason.contains("source")
                || reason.contains("boundary has a gap")
                || reason.contains("boundary cycle")
            {
                MeshUpdateFailureKind::InvalidSource
            } else {
                MeshUpdateFailureKind::RepairFailure
            }
        }
    };
    MeshUpdateFailure { kind, detail }
}

fn retryable(kind: MeshUpdateFailureKind) -> bool {
    matches!(
        kind,
        MeshUpdateFailureKind::FixedPatchBoundary
            | MeshUpdateFailureKind::ElementInversion
            | MeshUpdateFailureKind::BoundarySubdivisionLimit
            | MeshUpdateFailureKind::RefinementLimit
    )
}

fn point_in_expanded_bounds(point: Point2, bounds: [Point2; 2], radius: f64) -> bool {
    point.x >= bounds[0].x - radius
        && point.x <= bounds[1].x + radius
        && point.y >= bounds[0].y - radius
        && point.y <= bounds[1].y + radius
}

fn point_near_moving_segment(point: Point2, segment: [Point2; 4], radius: f64) -> bool {
    crate::point_segment_distance(point, segment[0], segment[1]) < radius
        || crate::point_segment_distance(point, segment[2], segment[3]) < radius
        || crate::point_segment_distance(point, segment[0], segment[2]) < radius
        || crate::point_segment_distance(point, segment[1], segment[3]) < radius
}

fn record_motion(
    radius: &mut f64,
    moving_bounds: &mut BTreeMap<MovingBoundaryId, [Point2; 2]>,
    id: Option<MovingBoundaryId>,
    source: Point2,
    target: Point2,
    edge_length: f64,
) -> Result<Point2, MeshError> {
    let delta = target - source;
    if delta.norm() > 4.0 * edge_length {
        return Err(MeshError::Topology(
            "boundary motion exceeds local repair radius",
        ));
    }
    *radius = radius.max(4.0 * delta.norm());
    if let Some(id) = id {
        let bounds = moving_bounds.entry(id).or_insert([
            Point2::new(f64::INFINITY, f64::INFINITY),
            Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY),
        ]);
        for point in [source, target] {
            bounds[0].x = bounds[0].x.min(point.x);
            bounds[0].y = bounds[0].y.min(point.y);
            bounds[1].x = bounds[1].x.max(point.x);
            bounds[1].y = bounds[1].y.max(point.y);
        }
    }
    Ok(delta)
}

fn trace_key(
    id: InternalBoundaryId,
    side: InternalBoundarySide,
    parameters: [f64; 2],
) -> Result<(TraceSegmentKey, usize), MeshError> {
    let [a, b] = parameters;
    if !a.is_finite() || !b.is_finite() || a == b {
        return Err(MeshError::Topology(
            "paired baffle trace has invalid parameters",
        ));
    }
    let (start, end, side_index) = match side {
        InternalBoundarySide::Left if a < b => (a, b, 0),
        InternalBoundarySide::Right if b < a => (b, a, 1),
        _ => {
            return Err(MeshError::Topology(
                "paired baffle trace has inconsistent orientation",
            ));
        }
    };
    Ok((
        TraceSegmentKey {
            id,
            start: start.to_bits(),
            end: end.to_bits(),
        },
        side_index,
    ))
}

impl MeshUpdateJob {
    pub fn new(
        previous: Option<(Arc<TriMesh>, Scene)>,
        scene: Scene,
        revision: u64,
        options: MeshingOptions,
    ) -> Self {
        Self::new_versioned(previous, scene, revision, revision, options)
    }

    pub fn new_versioned(
        previous: Option<(Arc<TriMesh>, Scene)>,
        scene: Scene,
        revision: u64,
        mesh_revision: u64,
        options: MeshingOptions,
    ) -> Self {
        let (mesh, previous_scene) = previous.map_or((None, Scene::default()), |(mesh, scene)| {
            (Some(mesh), scene)
        });
        let mut result = Self {
            previous: mesh,
            previous_scene,
            scene,
            revision,
            mesh_revision,
            options,
            phase: Phase::Done,
            builder: None,
            job: None,
            report: MeshUpdateReport::default(),
            work: 0,
            displacement: vec![],
            next_displacement: vec![],
            smooth_vertices: vec![],
            allowed: vec![],
            next_allowed: vec![],
            radius: 0.0,
            next_boundary: vec![],
            boundary_seeds: vec![],
            old_keys: BTreeSet::new(),
            output: None,
            remap: vec![],
            compact_vertices: vec![],
            boundary_splits: 0,
            attempt: 0,
            moving_bounds: BTreeMap::new(),
            trace_edges: BTreeMap::new(),
            trace_pairs: vec![],
            moved_baffles: BTreeSet::new(),
            moving_trace_segments: vec![],
        };
        if let Some(mesh) = &result.previous {
            result.report.local_attempted = true;
            result.report.original_triangles = mesh.triangles.len();
            let supported = result
                .previous_scene
                .obstacles
                .iter()
                .chain(&result.scene.obstacles)
                .all(|loop_| !matches!(loop_.role, LoopRole::Wall { .. }));
            let compatible = supported
                && result.previous_scene.obstacles.len() == result.scene.obstacles.len()
                && result.scene.obstacles.iter().all(|new| {
                    result.previous_scene.obstacles.iter().any(|old| {
                        old.id == new.id
                            && old.role == new.role
                            && old.spline.intervals() == new.spline.intervals()
                    })
                })
                && result.previous_scene.internal_boundaries.len()
                    == result.scene.internal_boundaries.len()
                && result.scene.internal_boundaries.iter().all(|new| {
                    result.previous_scene.internal_boundaries.iter().any(|old| {
                        old.id == new.id
                            && old.region == new.region
                            && old.spline.controls().len() == new.spline.controls().len()
                            && old.spline.intervals() == new.spline.intervals()
                            && old.spline.multiplicities() == new.spline.multiplicities()
                    })
                });
            if compatible {
                result.phase =
                    Phase::Validate(Box::new(ValidationJob::new(result.scene.clone(), revision)));
            } else {
                let kind = if supported {
                    MeshUpdateFailureKind::IncompatibleGeometry
                } else {
                    MeshUpdateFailureKind::UnsupportedBoundaryTopology
                };
                result.fallback(MeshUpdateFailure {
                    kind,
                    detail: kind.label().into(),
                });
            }
        } else {
            result.full();
        }
        result
    }

    fn full(&mut self) {
        self.job = Some(MeshingJob::new_versioned(
            self.scene.clone(),
            self.revision,
            self.mesh_revision,
            self.options,
        ));
        self.phase = Phase::Full;
    }

    fn fallback(&mut self, failure: MeshUpdateFailure) {
        self.report.fallback_failure = Some(failure);
        self.report.used_local = false;
        self.builder = None;
        self.full();
    }

    fn start_attempt(&mut self) {
        let old = self
            .previous
            .as_ref()
            .expect("local attempt needs a source mesh");
        let mut builder = MeshBuilder::new(self.options);
        builder.loop_ids = self.scene.obstacles.iter().map(|loop_| loop_.id).collect();
        builder.loop_roles = self
            .scene
            .obstacles
            .iter()
            .map(|loop_| loop_.role)
            .collect();
        builder.internal_boundary_ids = self
            .scene
            .internal_boundaries
            .iter()
            .map(|boundary| boundary.id)
            .collect();
        builder.internal_boundary_regions = self
            .scene
            .internal_boundaries
            .iter()
            .map(|boundary| boundary.region)
            .collect();
        self.builder = Some(builder);
        self.job = None;
        self.displacement.clear();
        self.next_displacement.clear();
        self.smooth_vertices.clear();
        self.allowed.clear();
        self.next_allowed.clear();
        self.radius = 0.0;
        self.next_boundary.clear();
        self.boundary_seeds = vec![None; self.scene.obstacles.len() + 1];
        self.old_keys.clear();
        self.output = None;
        self.remap.clear();
        self.compact_vertices.clear();
        self.boundary_splits = 0;
        self.moving_bounds.clear();
        self.trace_edges.clear();
        self.trace_pairs.clear();
        self.moved_baffles.clear();
        self.moving_trace_segments.clear();
        self.report.repair_attempts += 1;
        self.report.preserved_triangles = 0;
        self.report.preserved_connectivity = 0;
        self.report.moved_vertices = 0;
        self.report.inserted_vertices = 0;
        self.report.collapsed_vertices = 0;
        self.report.repair_vertices = 0;
        self.report.repair_triangles = 0;
        self.report.repaired_baffles = 0;
        self.report.paired_trace_segments = 0;
        debug_assert_eq!(self.report.original_triangles, old.triangles.len());
        self.phase = Phase::Vertices(0);
    }

    fn handle_local_failure(&mut self, error: MeshError) {
        let failure = classify_failure(&error);
        if retryable(failure.kind) && self.attempt < 2 {
            self.report.retry_failures.push(failure);
            self.attempt += 1;
            self.start_attempt();
        } else {
            self.fallback(failure);
        }
    }

    fn finish_trace_import(&mut self) -> Result<(), MeshError> {
        let b = self.builder.as_mut().unwrap();
        let mut pairs = Vec::with_capacity(self.trace_edges.len());
        for boundary in &self.scene.internal_boundaries {
            let mut boundary_pairs = self
                .trace_edges
                .iter()
                .filter(|(key, _)| key.id == boundary.id)
                .map(|(key, faces)| {
                    let [Some(left), Some(right)] = *faces else {
                        return Err(MeshError::Topology(
                            "paired baffle trace is missing one face",
                        ));
                    };
                    Ok(TracePair {
                        id: key.id,
                        start: key.start,
                        end: key.end,
                        left,
                        right,
                    })
                })
                .collect::<Result<Vec<_>, MeshError>>()?;
            boundary_pairs
                .sort_by(|a, b| f64::from_bits(a.start).total_cmp(&f64::from_bits(b.start)));
            if boundary_pairs.len() < 2 {
                return Err(MeshError::Topology(
                    "paired baffle trace needs at least two segments",
                ));
            }
            let period = boundary.spline.period();
            if f64::from_bits(boundary_pairs[0].start) != 0.0
                || f64::from_bits(boundary_pairs.last().unwrap().end) != period
            {
                return Err(MeshError::Topology(
                    "paired baffle trace does not cover the spline",
                ));
            }
            let mut chain = Vec::with_capacity(boundary_pairs.len() + 1);
            for (index, pair) in boundary_pairs.iter().enumerate() {
                let left = b.boundary_edges[pair.left];
                let right = b.boundary_edges[pair.right];
                if left.label
                    != (BoundaryLabel::InternalBoundary {
                        id: boundary.id,
                        side: InternalBoundarySide::Left,
                    })
                    || right.label
                        != (BoundaryLabel::InternalBoundary {
                            id: boundary.id,
                            side: InternalBoundarySide::Right,
                        })
                    || left.parameters != [f64::from_bits(pair.start), f64::from_bits(pair.end)]
                    || right.parameters != [f64::from_bits(pair.end), f64::from_bits(pair.start)]
                {
                    return Err(MeshError::Topology(
                        "paired baffle trace labels or parameters disagree",
                    ));
                }
                for edge in [left, right] {
                    if b.adjacency
                        .get(&edge_key(edge.vertices[0], edge.vertices[1]))
                        .is_none_or(|adjacent| adjacent.len() != 1)
                    {
                        return Err(MeshError::Topology(
                            "paired baffle trace has incorrect adjacency",
                        ));
                    }
                    b.internal_trace_vertices.extend(edge.vertices);
                }
                let tolerance = b.options.curve_tolerance * 0.1;
                if (b.point(left.vertices[0]) - b.point(right.vertices[1])).norm() > tolerance
                    || (b.point(left.vertices[1]) - b.point(right.vertices[0])).norm() > tolerance
                {
                    return Err(MeshError::Topology(
                        "paired baffle trace faces are not coincident",
                    ));
                }
                if index == 0 {
                    if left.vertices[0] != right.vertices[1] {
                        return Err(MeshError::Topology(
                            "baffle tip is not shared by both faces",
                        ));
                    }
                    chain.push((left.vertices[0], f64::from_bits(pair.start)));
                } else {
                    let previous = boundary_pairs[index - 1];
                    let previous_left = b.boundary_edges[previous.left];
                    let previous_right = b.boundary_edges[previous.right];
                    if previous.end != pair.start
                        || previous_left.vertices[1] != left.vertices[0]
                        || previous_right.vertices[0] != right.vertices[1]
                    {
                        return Err(MeshError::Topology("paired baffle trace has a chain gap"));
                    }
                    if left.vertices[0] == right.vertices[1] {
                        return Err(MeshError::Topology(
                            "paired baffle trace shares an interior vertex",
                        ));
                    }
                }
                if index + 1 == boundary_pairs.len() && left.vertices[1] != right.vertices[0] {
                    return Err(MeshError::Topology(
                        "baffle tip is not shared by both faces",
                    ));
                }
                chain.push((left.vertices[1], f64::from_bits(pair.end)));
            }
            b.internal_chains.push(chain);
            pairs.extend(boundary_pairs);
        }
        if pairs.len() != self.trace_edges.len() {
            return Err(MeshError::Topology(
                "paired baffle trace has an unknown boundary id",
            ));
        }
        self.trace_pairs = pairs;
        self.report.repaired_baffles = self.moved_baffles.len();
        self.report.paired_trace_segments = self
            .trace_pairs
            .iter()
            .filter(|pair| self.moved_baffles.contains(&pair.id))
            .count();
        Ok(())
    }

    pub fn phase(&self) -> &'static str {
        if self.attempt > 0 && !matches!(self.phase, Phase::Full | Phase::Done | Phase::Validate(_))
        {
            return "Expanding local repair";
        }
        match &self.phase {
            Phase::Full => self.job.as_ref().unwrap().phase(),
            Phase::Validate(_) => "Validating edit",
            Phase::Vertices(_) | Phase::Triangles(_) | Phase::Edges(_) => "Reusing mesh",
            Phase::Region(_) | Phase::Expand { .. } | Phase::Patch(_) => "Selecting repair region",
            Phase::Smooth { .. } | Phase::Move(_) => "Moving local mesh",
            Phase::Coarsen(_) => "Coarsening local mesh",
            Phase::Repair => self.job.as_ref().unwrap().phase(),
            Phase::Check(_)
            | Phase::Boundary(_)
            | Phase::BaffleBoundary(_)
            | Phase::Loops { .. } => "Checking local geometry",
            Phase::CompactVertices(_) | Phase::CompactTriangles(_) | Phase::CompactEdges(_) => {
                "Measuring mesh reuse"
            }
            Phase::Done => "Finished",
        }
    }

    pub fn report(&self) -> &MeshUpdateReport {
        &self.report
    }

    pub fn advance(&mut self, budget: usize) -> Option<Result<MeshUpdateResult, MeshError>> {
        for _ in 0..budget {
            if matches!(self.phase, Phase::Done) {
                return None;
            }
            if matches!(self.phase, Phase::Full) {
                if let Some(result) = self.job.as_mut().unwrap().advance(1) {
                    self.phase = Phase::Done;
                    return Some(result.map(|mesh| MeshUpdateResult {
                        mesh,
                        report: self.report.clone(),
                    }));
                }
                continue;
            }
            self.work += 1;
            if self.work > 5_000_000 {
                self.fallback(MeshUpdateFailure {
                    kind: MeshUpdateFailureKind::WorkLimit,
                    detail: "local repair work limit reached".into(),
                });
                continue;
            }
            match self.step() {
                Ok(Some(mesh)) => {
                    self.phase = Phase::Done;
                    self.report.used_local = true;
                    return Some(Ok(MeshUpdateResult {
                        mesh,
                        report: self.report.clone(),
                    }));
                }
                Ok(None) => {}
                Err(error) => self.handle_local_failure(error),
            }
        }
        None
    }

    fn step(&mut self) -> Result<Option<TriMesh>, MeshError> {
        let phase = std::mem::replace(&mut self.phase, Phase::Done);
        let old = self.previous.as_ref().unwrap();
        // The builder moves into the normal resumable legalization/refinement
        // job after local preparation; use it only in the preparation phases.
        match phase {
            Phase::Validate(mut validation) => {
                if !valid_options(self.options) {
                    return Err(MeshError::InvalidOptions);
                }
                if let Some(result) = validation.advance(1) {
                    if let Some(issue) = result.issue {
                        return Err(MeshError::InvalidGeometry(issue));
                    }
                    self.start_attempt();
                } else {
                    self.phase = Phase::Validate(validation);
                }
            }
            Phase::Vertices(i) => {
                if i == old.vertices.len() {
                    self.phase = Phase::Triangles(0);
                    return Ok(None);
                }
                let vertex = old.vertices[i];
                if !vertex.point.finite() {
                    return Err(MeshError::Topology("invalid source vertex"));
                }
                let mut delta = Point2::default();
                if let Some(BoundaryPoint { label, parameter }) = vertex.boundary {
                    if !parameter.is_finite() {
                        return Err(MeshError::Topology("invalid boundary parameter"));
                    }
                    match label {
                        BoundaryLabel::Obstacle(id) | BoundaryLabel::MaterialInterface(id) => {
                            let before = self
                                .previous_scene
                                .obstacles
                                .iter()
                                .find(|o| o.id == id)
                                .ok_or(MeshError::Topology("missing source obstacle"))?;
                            let after = self
                                .scene
                                .obstacles
                                .iter()
                                .find(|o| o.id == id)
                                .ok_or(MeshError::Topology("missing target obstacle"))?;
                            let before_point = before.spline.evaluate(parameter);
                            let target = after.spline.evaluate(parameter);
                            if (target - before_point).norm() > 1e-12 {
                                delta = record_motion(
                                    &mut self.radius,
                                    &mut self.moving_bounds,
                                    Some(MovingBoundaryId::Loop(id.0)),
                                    vertex.point,
                                    target,
                                    self.options.target_edge_length,
                                )?;
                            }
                        }
                        BoundaryLabel::InternalBoundary { id, .. } => {
                            let before = self
                                .previous_scene
                                .internal_boundaries
                                .iter()
                                .find(|boundary| boundary.id == id)
                                .ok_or(MeshError::Topology("missing source baffle"))?;
                            let after = self
                                .scene
                                .internal_boundaries
                                .iter()
                                .find(|boundary| boundary.id == id)
                                .ok_or(MeshError::Topology("missing target baffle"))?;
                            if before.spline.span_index(parameter).is_none()
                                || after.spline.span_index(parameter).is_none()
                            {
                                return Err(MeshError::Topology(
                                    "paired baffle trace has an invalid parameter",
                                ));
                            }
                            let before_point = before.spline.evaluate(parameter);
                            let target = after.spline.evaluate(parameter);
                            if (target - before_point).norm() > 1e-12 {
                                delta = record_motion(
                                    &mut self.radius,
                                    &mut self.moving_bounds,
                                    None,
                                    vertex.point,
                                    target,
                                    self.options.target_edge_length,
                                )?;
                                self.moved_baffles.insert(id);
                            }
                        }
                        BoundaryLabel::Outer(_) | BoundaryLabel::Wall { .. } => {}
                    }
                }
                self.builder
                    .as_mut()
                    .unwrap()
                    .add_vertex(vertex.point, vertex.boundary)?;
                self.displacement.push(delta);
                self.next_displacement.push(delta);
                self.allowed.push(delta != Point2::default());
                self.next_allowed.push(false);
                self.next_boundary.push(None);
                self.phase = Phase::Vertices(i + 1);
            }
            Phase::Triangles(i) => {
                let b = self.builder.as_mut().unwrap();
                if i == old.triangles.len() {
                    self.phase = Phase::Edges(0);
                    return Ok(None);
                }
                let triangle = old.triangles[i];
                if triangle.vertices.iter().any(|v| *v >= b.vertices.len()) {
                    return Err(MeshError::Topology("invalid source triangle"));
                }
                if b.triangles.len() >= b.options.max_triangles {
                    return Err(b.capacity_error());
                }
                // Import connectivity only. No global quality queue or flips.
                b.triangles.push(triangle);
                b.scores.push(None);
                for opposite in 0..3 {
                    let vertex = triangle.vertices[opposite];
                    b.incident[vertex].insert(i);
                    let edge = edge_key(
                        triangle.vertices[(opposite + 1) % 3],
                        triangle.vertices[(opposite + 2) % 3],
                    );
                    b.adjacency.entry(edge).or_default().push((i, vertex));
                }
                self.old_keys.insert(key(triangle.vertices));
                self.phase = Phase::Triangles(i + 1);
            }
            Phase::Edges(i) => {
                if i == old.boundary_edges.len() {
                    self.finish_trace_import()?;
                    self.radius = self.radius.max(4.0 * self.options.target_edge_length)
                        + self.attempt as f64 * 2.0 * self.options.target_edge_length;
                    self.phase = Phase::Region(0);
                    return Ok(None);
                }
                let edge = old.boundary_edges[i];
                if edge.vertices.iter().any(|v| *v >= self.next_boundary.len()) {
                    return Err(MeshError::Topology("invalid source boundary"));
                }
                match edge.label {
                    BoundaryLabel::Outer(_)
                    | BoundaryLabel::Obstacle(_)
                    | BoundaryLabel::MaterialInterface(_) => {
                        if self.next_boundary[edge.vertices[0]]
                            .replace(edge.vertices[1])
                            .is_some()
                        {
                            return Err(MeshError::Topology("non-manifold source boundary"));
                        }
                        let loop_index = match edge.label {
                            BoundaryLabel::Outer(_) => 0,
                            BoundaryLabel::Obstacle(id) | BoundaryLabel::MaterialInterface(id) => {
                                self.scene
                                    .obstacles
                                    .iter()
                                    .position(|loop_| loop_.id == id)
                                    .map(|index| index + 1)
                                    .ok_or(MeshError::Topology("missing target boundary loop"))?
                            }
                            _ => unreachable!(),
                        };
                        self.boundary_seeds[loop_index].get_or_insert(edge.vertices[0]);
                    }
                    BoundaryLabel::InternalBoundary { id, side } => {
                        if side == InternalBoundarySide::Left
                            && edge
                                .vertices
                                .iter()
                                .any(|vertex| self.displacement[*vertex] != Point2::default())
                        {
                            self.moving_trace_segments.push([
                                old.vertices[edge.vertices[0]].point,
                                old.vertices[edge.vertices[1]].point,
                                old.vertices[edge.vertices[0]].point
                                    + self.displacement[edge.vertices[0]],
                                old.vertices[edge.vertices[1]].point
                                    + self.displacement[edge.vertices[1]],
                            ]);
                        }
                        let (key, side_index) = trace_key(id, side, edge.parameters)?;
                        if self.trace_edges.entry(key).or_insert([None, None])[side_index]
                            .replace(i)
                            .is_some()
                        {
                            return Err(MeshError::Topology(
                                "paired baffle trace contains a duplicate face",
                            ));
                        }
                    }
                    BoundaryLabel::Wall { .. } => {
                        return Err(MeshError::Topology("unsupported source boundary topology"));
                    }
                }
                self.builder.as_mut().unwrap().add_boundary_edge(edge);
                self.phase = Phase::Edges(i + 1);
            }
            Phase::Region(i) => {
                let b = self.builder.as_mut().unwrap();
                if i == b.vertices.len() {
                    self.next_allowed.clone_from(&self.allowed);
                    self.phase = Phase::Expand { ring: 0, index: 0 };
                    return Ok(None);
                }
                if b.vertices[i].boundary.is_none()
                    && (self
                        .moving_bounds
                        .values()
                        .any(|bounds| point_in_expanded_bounds(b.point(i), *bounds, self.radius))
                        || self.moving_trace_segments.iter().any(|segment| {
                            point_near_moving_segment(b.point(i), *segment, self.radius)
                        }))
                {
                    self.allowed[i] = true;
                    self.smooth_vertices.push(i);
                }
                self.phase = Phase::Region(i + 1);
            }
            Phase::Expand { ring, index } => {
                let b = self.builder.as_mut().unwrap();
                if index == b.triangles.len() {
                    self.allowed.clone_from(&self.next_allowed);
                    if ring + 1 < self.attempt + 1 {
                        self.next_allowed.clone_from(&self.allowed);
                        self.phase = Phase::Expand {
                            ring: ring + 1,
                            index: 0,
                        };
                    } else {
                        self.report.repair_vertices =
                            self.allowed.iter().filter(|allowed| **allowed).count();
                        self.phase = Phase::Patch(0);
                    }
                    return Ok(None);
                }
                let triangle = b.triangles[index];
                if triangle.vertices.iter().any(|v| self.allowed[*v]) {
                    for v in triangle.vertices {
                        self.next_allowed[v] = true;
                    }
                }
                self.phase = Phase::Expand {
                    ring,
                    index: index + 1,
                };
            }
            Phase::Patch(i) => {
                let b = self.builder.as_mut().unwrap();
                if i == b.triangles.len() {
                    b.repair_region = Some(std::mem::take(&mut self.allowed));
                    self.phase = Phase::Smooth { pass: 0, index: 0 };
                    return Ok(None);
                }
                let triangle = b.triangles[i];
                if triangle.vertices.iter().all(|v| self.allowed[*v]) {
                    self.report.repair_triangles += 1;
                    if self.report.repair_triangles > (old.triangles.len() / 3).max(256) {
                        return Err(MeshError::Topology(
                            "edit affects too much of the mesh for local repair",
                        ));
                    }
                }
                self.phase = Phase::Patch(i + 1);
            }
            Phase::Smooth { pass, index } => {
                if index == self.smooth_vertices.len() {
                    std::mem::swap(&mut self.displacement, &mut self.next_displacement);
                    self.phase = if pass + 1 == 24 * (self.attempt + 1) {
                        Phase::Move(0)
                    } else {
                        Phase::Smooth {
                            pass: pass + 1,
                            index: 0,
                        }
                    };
                    return Ok(None);
                }
                let b = self.builder.as_ref().unwrap();
                let v = self.smooth_vertices[index];
                let mut sum = Point2::default();
                let mut count = 0;
                for triangle in &b.incident[v] {
                    for neighbor in b.triangles[*triangle].vertices {
                        if neighbor != v {
                            sum = sum + self.displacement[neighbor];
                            count += 1;
                        }
                    }
                }
                if count > 0 {
                    self.next_displacement[v] = sum / count as f64;
                }
                self.phase = Phase::Smooth {
                    pass,
                    index: index + 1,
                };
            }
            Phase::Move(i) => {
                let b = self.builder.as_mut().unwrap();
                if i == b.vertices.len() {
                    self.phase = Phase::Check(0);
                    return Ok(None);
                }
                if self.displacement[i] != Point2::default() {
                    b.vertices[i].point = b.point(i) + self.displacement[i];
                    self.report.moved_vertices += 1;
                }
                self.phase = Phase::Move(i + 1);
            }
            Phase::Check(i) => {
                let b = self.builder.as_mut().unwrap();
                if i == b.triangles.len() {
                    self.phase = Phase::Boundary(0);
                    return Ok(None);
                }
                let triangle = b.triangles[i];
                if triangle
                    .vertices
                    .iter()
                    .any(|v| b.repair_region.as_ref().unwrap()[*v])
                {
                    let [a, c, d] = b.triangle_points(triangle);
                    if orient2d(a, c, d) != PredicateSign::Positive {
                        return Err(MeshError::Topology("local motion inverted an element"));
                    }
                    b.replace_triangle(i, triangle);
                }
                self.phase = Phase::Check(i + 1);
            }
            Phase::Boundary(i) => {
                let b = self.builder.as_mut().unwrap();
                if i == b.boundary_edges.len() {
                    self.phase = Phase::BaffleBoundary(0);
                    return Ok(None);
                }
                let edge = b.boundary_edges[i];
                if let BoundaryLabel::Obstacle(id) | BoundaryLabel::MaterialInterface(id) =
                    edge.label
                {
                    let spline = &self
                        .scene
                        .obstacles
                        .iter()
                        .find(|o| o.id == id)
                        .ok_or(MeshError::Topology("missing obstacle label"))?
                        .spline;
                    let [t0, t1] = edge.parameters;
                    let a = spline.evaluate(t0);
                    let d = spline.evaluate(t1);
                    let hull = [
                        a,
                        a + spline.derivative(t0, 1) * ((t1 - t0) / 3.0),
                        d - spline.derivative(t1, 1) * ((t1 - t0) / 3.0),
                        d,
                    ];
                    if hull.iter().any(|p| {
                        crate::point_segment_distance(
                            *p,
                            b.point(edge.vertices[0]),
                            b.point(edge.vertices[1]),
                        ) > b.options.curve_tolerance * (1.0 + 1e-9)
                    }) {
                        if self.boundary_splits >= (256 << self.attempt) {
                            return Err(MeshError::Topology("local boundary subdivision limit"));
                        }
                        let parameter = (t0 + t1) * 0.5;
                        let point = spline.evaluate(parameter);
                        split_curved_boundary(b, i, point, parameter)?;
                        let vertex = b.vertices.len() - 1;
                        self.next_boundary[edge.vertices[0]] = Some(vertex);
                        self.next_boundary.push(Some(edge.vertices[1]));
                        self.boundary_splits += 1;
                        self.phase = Phase::Boundary(i);
                        return Ok(None);
                    }
                }
                self.phase = Phase::Boundary(i + 1);
            }
            Phase::BaffleBoundary(i) => {
                if i == self.trace_pairs.len() {
                    self.phase = Phase::Coarsen(0);
                    return Ok(None);
                }
                let pair = self.trace_pairs[i];
                let spline = &self
                    .scene
                    .internal_boundaries
                    .iter()
                    .find(|boundary| boundary.id == pair.id)
                    .ok_or(MeshError::Topology("missing target baffle"))?
                    .spline;
                let [t0, t1] = [f64::from_bits(pair.start), f64::from_bits(pair.end)];
                let b = self.builder.as_mut().unwrap();
                let edge = b.boundary_edges[pair.left];
                let a = spline.evaluate(t0);
                let d = spline.evaluate(t1);
                let hull = [
                    a,
                    a + spline.derivative(t0, 1) * ((t1 - t0) / 3.0),
                    d - spline.derivative(t1, 1) * ((t1 - t0) / 3.0),
                    d,
                ];
                let chord_too_long = (b.point(edge.vertices[1]) - b.point(edge.vertices[0])).norm()
                    > b.options.target_edge_length * (1.0 + 1e-9);
                let curve_too_far = hull.iter().any(|point| {
                    crate::point_segment_distance(
                        *point,
                        b.point(edge.vertices[0]),
                        b.point(edge.vertices[1]),
                    ) > b.options.curve_tolerance * (1.0 + 1e-9)
                });
                if chord_too_long || curve_too_far {
                    if self.boundary_splits >= (256 << self.attempt) {
                        return Err(MeshError::Topology("paired baffle subdivision limit"));
                    }
                    let parameter = 0.5 * (t0 + t1);
                    let children =
                        split_baffle_pair(b, pair, spline.evaluate(parameter), parameter)?;
                    self.trace_pairs[i] = children[0];
                    self.trace_pairs.push(children[1]);
                    self.boundary_splits += 1;
                    self.report.paired_trace_segments = self
                        .trace_pairs
                        .iter()
                        .filter(|pair| self.moved_baffles.contains(&pair.id))
                        .count();
                    self.phase = Phase::BaffleBoundary(i);
                } else {
                    self.phase = Phase::BaffleBoundary(i + 1);
                }
            }
            Phase::Coarsen(i) => {
                let b = self.builder.as_mut().unwrap();
                if i == self.smooth_vertices.len() {
                    self.phase = Phase::Loops {
                        loop_index: 0,
                        current: None,
                    };
                    return Ok(None);
                }
                let v = self.smooth_vertices[i];
                let neighbors: BTreeSet<_> = b.incident[v]
                    .iter()
                    .flat_map(|t| b.triangles[*t].vertices)
                    .filter(|u| *u < v)
                    .collect();
                for u in neighbors {
                    if (b.point(u) - b.point(v)).norm() < b.options.target_edge_length * 0.35
                        && collapse(b, v, u)?
                    {
                        self.report.collapsed_vertices += 1;
                        break;
                    }
                }
                self.phase = Phase::Coarsen(i + 1);
            }
            Phase::Loops {
                loop_index,
                current,
            } => {
                let b = self.builder.as_mut().unwrap();
                if loop_index == self.boundary_seeds.len() {
                    let mut builder = self.builder.take().unwrap();
                    builder.interior_loops = vec![None; self.scene.obstacles.len()];
                    builder.options.max_refinement_steps = builder
                        .options
                        .max_refinement_steps
                        .min(512 << self.attempt);
                    self.job = Some(MeshingJob {
                        scene: self.scene.clone(),
                        geometry_revision: self.revision,
                        mesh_revision: self.mesh_revision,
                        builder,
                        legalization_work: 0,
                        state: MeshingJobState::Legalize,
                    });
                    self.phase = Phase::Repair;
                    return Ok(None);
                }
                let start = self.boundary_seeds[loop_index]
                    .ok_or(MeshError::Topology("source boundary loop is missing"))?;
                let v = current.unwrap_or(start);
                if current.is_none() {
                    b.domain_loops.push(Polygon { vertices: vec![] });
                }
                let polygon = &mut b.domain_loops[loop_index];
                if polygon.vertices.len() > b.boundary_edges.len() {
                    return Err(MeshError::Topology("source boundary cycle does not close"));
                }
                polygon.vertices.push(v);
                let next = self.next_boundary[v]
                    .ok_or(MeshError::Topology("source boundary has a gap"))?;
                self.phase = if next == start {
                    Phase::Loops {
                        loop_index: loop_index + 1,
                        current: None,
                    }
                } else {
                    Phase::Loops {
                        loop_index,
                        current: Some(next),
                    }
                };
            }
            Phase::Repair => {
                if let Some(result) = self.job.as_mut().unwrap().advance(1) {
                    let mesh = result?;
                    self.report.inserted_vertices =
                        mesh.vertices.len().saturating_sub(old.vertices.len());
                    self.output = Some(mesh);
                    self.phase = Phase::CompactVertices(0);
                } else {
                    self.phase = Phase::Repair;
                }
            }
            Phase::CompactVertices(i) => {
                let mesh = self.output.as_ref().unwrap();
                if i == mesh.vertices.len() {
                    self.phase = Phase::CompactTriangles(0);
                    return Ok(None);
                }
                if self.job.as_ref().unwrap().builder.incident[i].is_empty() {
                    self.remap.push(usize::MAX);
                } else {
                    self.remap.push(self.compact_vertices.len());
                    self.compact_vertices.push(mesh.vertices[i]);
                }
                self.phase = Phase::CompactVertices(i + 1);
            }
            Phase::CompactTriangles(i) => {
                let mesh = self.output.as_mut().unwrap();
                if i == mesh.triangles.len() {
                    self.phase = Phase::CompactEdges(0);
                    return Ok(None);
                }
                let triangle = mesh.triangles[i];
                if self.old_keys.contains(&key(triangle.vertices)) {
                    self.report.preserved_connectivity += 1;
                    if triangle
                        .vertices
                        .iter()
                        .all(|v| mesh.vertices[*v].point == old.vertices[*v].point)
                    {
                        self.report.preserved_triangles += 1;
                    }
                }
                mesh.triangles[i].vertices = triangle.vertices.map(|v| self.remap[v]);
                self.phase = Phase::CompactTriangles(i + 1);
            }
            Phase::CompactEdges(i) => {
                let mesh = self.output.as_mut().unwrap();
                if i == mesh.boundary_edges.len() {
                    mesh.vertices = std::mem::take(&mut self.compact_vertices);
                    return Ok(self.output.take());
                }
                mesh.boundary_edges[i].vertices =
                    mesh.boundary_edges[i].vertices.map(|v| self.remap[v]);
                self.phase = Phase::CompactEdges(i + 1);
            }
            Phase::Full | Phase::Done => unreachable!(),
        }
        Ok(None)
    }
}

fn split_curved_boundary(
    b: &mut MeshBuilder,
    index: usize,
    point: Point2,
    parameter: f64,
) -> Result<(), MeshError> {
    let edge = b.boundary_edges[index];
    let adjacent = b
        .adjacency
        .get(&edge_key(edge.vertices[0], edge.vertices[1]))
        .ok_or(MeshError::Topology("missing boundary edge"))?;
    if adjacent.is_empty() || adjacent.len() > 2 {
        return Err(MeshError::Topology("non-manifold boundary edge"));
    }
    let [a, c] = edge.vertices.map(|v| b.point(v));
    for (_, opposite) in adjacent {
        let d = b.point(*opposite);
        let sign = orient2d(a, c, d);
        if orient2d(a, point, d) != sign || orient2d(point, c, d) != sign {
            return Err(MeshError::Topology(
                "curved boundary refinement inverted an element",
            ));
        }
    }
    if edge
        .vertices
        .iter()
        .any(|v| !b.repair_region.as_ref().unwrap()[*v])
    {
        return Err(MeshError::Topology(
            "boundary sampling repair left the patch",
        ));
    }
    let vertex = b.add_vertex(
        point,
        Some(BoundaryPoint {
            label: edge.label,
            parameter,
        }),
    )?;
    b.boundary_keys
        .remove(&edge_key(edge.vertices[0], edge.vertices[1]));
    b.boundary_edges[index] = BoundaryEdge {
        vertices: [edge.vertices[0], vertex],
        label: edge.label,
        parameters: [edge.parameters[0], parameter],
    };
    b.boundary_keys.insert(edge_key(edge.vertices[0], vertex));
    b.add_boundary_edge(BoundaryEdge {
        vertices: [vertex, edge.vertices[1]],
        label: edge.label,
        parameters: [parameter, edge.parameters[1]],
    });
    b.split_edge(edge.vertices, vertex)
}

fn split_baffle_pair(
    b: &mut MeshBuilder,
    pair: TracePair,
    point: Point2,
    parameter: f64,
) -> Result<[TracePair; 2], MeshError> {
    let left = b.boundary_edges[pair.left];
    let right = b.boundary_edges[pair.right];
    let [start, end] = [f64::from_bits(pair.start), f64::from_bits(pair.end)];
    if left.label
        != (BoundaryLabel::InternalBoundary {
            id: pair.id,
            side: InternalBoundarySide::Left,
        })
        || right.label
            != (BoundaryLabel::InternalBoundary {
                id: pair.id,
                side: InternalBoundarySide::Right,
            })
        || left.parameters != [start, end]
        || right.parameters != [end, start]
    {
        return Err(MeshError::Topology(
            "paired baffle trace changed during repair",
        ));
    }
    if b.vertices.len() + 2 > b.options.max_vertices
        || b.triangles.len() + 2 > b.options.max_triangles
    {
        return Err(b.capacity_error());
    }
    preflight_baffle_split(b, left, point)?;
    preflight_baffle_split(b, right, point)?;

    let left_new_edge = b.boundary_edges.len();
    let left_vertex = b.add_vertex(
        point,
        Some(BoundaryPoint {
            label: left.label,
            parameter,
        }),
    )?;
    split_boundary_record(b, pair.left, left_vertex, parameter)?;
    let right_new_edge = b.boundary_edges.len();
    let right_vertex = b.add_vertex(
        point,
        Some(BoundaryPoint {
            label: right.label,
            parameter,
        }),
    )?;
    split_boundary_record(b, pair.right, right_vertex, parameter)?;
    b.internal_trace_vertices.insert(left_vertex);
    b.internal_trace_vertices.insert(right_vertex);

    let midpoint = parameter.to_bits();
    Ok([
        TracePair {
            id: pair.id,
            start: pair.start,
            end: midpoint,
            left: pair.left,
            right: right_new_edge,
        },
        TracePair {
            id: pair.id,
            start: midpoint,
            end: pair.end,
            left: left_new_edge,
            right: pair.right,
        },
    ])
}

fn preflight_baffle_split(
    b: &MeshBuilder,
    edge: BoundaryEdge,
    point: Point2,
) -> Result<(), MeshError> {
    let adjacent = b
        .adjacency
        .get(&edge_key(edge.vertices[0], edge.vertices[1]))
        .ok_or(MeshError::Topology(
            "paired baffle trace edge has no adjacent element",
        ))?;
    if adjacent.len() != 1 {
        return Err(MeshError::Topology(
            "paired baffle trace edge has incorrect adjacency",
        ));
    }
    let triangle = b.triangles[adjacent[0].0];
    if triangle
        .vertices
        .iter()
        .any(|vertex| !b.repair_region.as_ref().unwrap()[*vertex])
    {
        return Err(MeshError::Topology(
            "paired baffle split reached the fixed patch boundary",
        ));
    }
    let [a, c] = edge.vertices.map(|vertex| b.point(vertex));
    let d = b.point(adjacent[0].1);
    let sign = orient2d(a, c, d);
    if sign == PredicateSign::Zero || orient2d(a, point, d) != sign || orient2d(point, c, d) != sign
    {
        return Err(MeshError::Topology(
            "paired baffle refinement inverted an element",
        ));
    }
    Ok(())
}

fn split_boundary_record(
    b: &mut MeshBuilder,
    index: usize,
    vertex: usize,
    parameter: f64,
) -> Result<(), MeshError> {
    let edge = b.boundary_edges[index];
    b.boundary_keys
        .remove(&edge_key(edge.vertices[0], edge.vertices[1]));
    b.boundary_edges[index] = BoundaryEdge {
        vertices: [edge.vertices[0], vertex],
        label: edge.label,
        parameters: [edge.parameters[0], parameter],
    };
    b.boundary_keys.insert(edge_key(edge.vertices[0], vertex));
    b.add_boundary_edge(BoundaryEdge {
        vertices: [vertex, edge.vertices[1]],
        label: edge.label,
        parameters: [parameter, edge.parameters[1]],
    });
    b.split_edge(edge.vertices, vertex)
}

/// Conservative interior edge collapse: preserve the link condition, the patch
/// boundary, positive orientation, and all quality targets. The 0.35 h trigger
/// is deliberately separated from the 1.05 h split threshold (hysteresis).
fn collapse(b: &mut MeshBuilder, remove: usize, keep: usize) -> Result<bool, MeshError> {
    if b.vertices[remove].boundary.is_some()
        || b.vertices[keep].boundary.is_some()
        || b.incident[remove].is_empty()
    {
        return Ok(false);
    }
    let neighbors = |v: usize| {
        b.incident[v]
            .iter()
            .flat_map(|t| b.triangles[*t].vertices)
            .filter(|u| *u != v)
            .collect::<BTreeSet<_>>()
    };
    if neighbors(remove).intersection(&neighbors(keep)).count() != 2 {
        return Ok(false);
    }
    let mut replacements = vec![];
    let mut deleted = vec![];
    for &index in &b.incident[remove] {
        let triangle = b.triangles[index];
        if triangle
            .vertices
            .iter()
            .any(|v| !b.repair_region.as_ref().unwrap()[*v])
        {
            return Ok(false);
        }
        if triangle.vertices.contains(&keep) {
            deleted.push(index);
            continue;
        }
        let triangle = MeshTriangle {
            vertices: triangle
                .vertices
                .map(|v| if v == remove { keep } else { v }),
            region: triangle.region,
        };
        let [a, c, d] = b.triangle_points(triangle);
        if orient2d(a, c, d) != PredicateSign::Positive {
            return Ok(false);
        }
        let quality = b.triangle_quality(triangle);
        if quality.minimum_angle_degrees < b.options.minimum_angle_degrees
            || quality.maximum_edge_length > b.options.target_edge_length * 1.05
        {
            return Ok(false);
        }
        replacements.push((index, triangle));
    }
    if deleted.len() != 2 {
        return Ok(false);
    }
    for (index, triangle) in replacements {
        b.replace_triangle(index, triangle);
    }
    deleted.sort_unstable();
    for index in deleted.into_iter().rev() {
        b.unregister_triangle(index);
        let last = b.triangles.len() - 1;
        if last != index {
            b.unregister_triangle(last);
        }
        b.triangles.swap_remove(index);
        b.scores.swap_remove(index);
        if last != index {
            b.register_triangle(index);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interior_collapse_restores_a_refined_fan_without_touching_boundary() {
        let mut b = MeshBuilder::new(MeshingOptions {
            target_edge_length: 3.0,
            minimum_angle_degrees: 10.0,
            ..Default::default()
        });
        for p in [
            Point2::new(-1.0, -1.0),
            Point2::new(1.0, -1.0),
            Point2::new(1.0, 1.0),
            Point2::new(-1.0, 1.0),
            Point2::default(),
        ] {
            b.add_vertex(p, None).unwrap();
        }
        for i in 0..4 {
            b.push_triangle(
                b.ccw_triangle([i, (i + 1) % 4, 4], BACKGROUND_REGION)
                    .unwrap(),
            )
            .unwrap();
        }
        let original: BTreeSet<_> = b.triangles.iter().map(|t| key(t.vertices)).collect();
        b.repair_region = Some(vec![true; 5]);
        b.insert_point(Point2::new(0.0, -0.1)).unwrap();
        assert_eq!(b.triangles.len(), 6);
        assert!(collapse(&mut b, 5, 4).unwrap());
        assert_eq!(b.triangles.len(), 4);
        assert!(b.incident[5].is_empty());
        assert_eq!(
            b.triangles
                .iter()
                .map(|t| key(t.vertices))
                .collect::<BTreeSet<_>>(),
            original
        );
        for (index, triangle) in b.triangles.iter().enumerate() {
            for v in triangle.vertices {
                assert!(b.incident[v].contains(&index));
            }
            let [a, c, d] = b.triangle_points(*triangle);
            assert_eq!(orient2d(a, c, d), PredicateSign::Positive);
        }
        for sides in b.adjacency.values() {
            for (i, v) in sides {
                assert!(b.triangles[*i].vertices.contains(v));
            }
        }
    }

    #[test]
    fn coarsening_cannot_cross_the_frozen_patch() {
        let mut b = MeshBuilder::new(MeshingOptions {
            target_edge_length: 3.0,
            ..Default::default()
        });
        for p in [
            Point2::new(-1.0, -1.0),
            Point2::new(1.0, -1.0),
            Point2::new(1.0, 1.0),
            Point2::new(-1.0, 1.0),
            Point2::default(),
        ] {
            b.add_vertex(p, None).unwrap();
        }
        for i in 0..4 {
            b.push_triangle(
                b.ccw_triangle([i, (i + 1) % 4, 4], BACKGROUND_REGION)
                    .unwrap(),
            )
            .unwrap();
        }
        b.insert_point(Point2::new(0.0, -0.1)).unwrap();
        b.repair_region = Some(vec![false, false, true, true, true, true]);
        let original = b.triangles.clone();
        assert!(!collapse(&mut b, 5, 4).unwrap());
        assert_eq!(b.triangles, original);
    }

    #[test]
    fn retryable_failures_start_fresh_attempts_and_terminal_failure_falls_back() {
        let options = MeshingOptions {
            target_edge_length: 0.08,
            minimum_angle_degrees: 12.0,
            ..Default::default()
        };
        let scene = Scene::initial();
        let mesh = Arc::new(mesh_scene(&scene, 0, options).unwrap());
        let mut next = scene.clone();
        let point = next.obstacles[0].spline.controls()[0];
        next.obstacles[0]
            .spline
            .set_control(0, point + Point2::new(0.004, 0.0))
            .unwrap();
        let mut job = MeshUpdateJob::new(Some((mesh, scene)), next, 1, options);
        while !matches!(job.phase, Phase::Vertices(_)) {
            assert!(job.advance(1).is_none());
        }
        for _ in 0..4 {
            assert!(job.advance(1).is_none());
        }
        assert_eq!(job.builder.as_ref().unwrap().vertices.len(), 4);

        job.handle_local_failure(MeshError::Topology("local motion inverted an element"));
        assert_eq!(job.attempt, 1);
        assert_eq!(job.report.repair_attempts, 2);
        assert_eq!(job.report.retry_failures.len(), 1);
        assert!(matches!(job.phase, Phase::Vertices(0)));
        assert_eq!(job.phase(), "Expanding local repair");
        assert!(job.builder.as_ref().unwrap().vertices.is_empty());

        job.handle_local_failure(MeshError::RefinementLimit(MeshQuality {
            minimum_angle_degrees: 4.0,
            maximum_edge_length: 0.2,
        }));
        assert_eq!(job.attempt, 2);
        assert_eq!(job.report.repair_attempts, 3);
        assert_eq!(job.report.retry_failures.len(), 2);
        assert!(job.builder.as_ref().unwrap().vertices.is_empty());

        job.handle_local_failure(MeshError::Topology("local boundary subdivision limit"));
        assert!(matches!(job.phase, Phase::Full));
        assert_eq!(
            job.report.fallback_failure.as_ref().unwrap().kind,
            MeshUpdateFailureKind::BoundarySubdivisionLimit
        );
        assert_eq!(job.report.retry_failures.len(), 2);
    }

    #[test]
    fn failure_classification_separates_retryable_and_terminal_causes() {
        for error in [
            MeshError::Topology("edge split reached the fixed patch boundary"),
            MeshError::Topology("mesh contains an inverted triangle"),
            MeshError::Topology("local boundary subdivision limit"),
            MeshError::RefinementLimit(MeshQuality {
                minimum_angle_degrees: 1.0,
                maximum_edge_length: 1.0,
            }),
        ] {
            assert!(retryable(classify_failure(&error).kind));
        }
        for error in [
            MeshError::InvalidOptions,
            MeshError::Capacity {
                vertices: 1,
                triangles: 1,
            },
            MeshError::Topology("invalid source vertex"),
            MeshError::Topology("edit affects too much of the mesh for local repair"),
        ] {
            assert!(!retryable(classify_failure(&error).kind));
        }
        assert_eq!(
            classify_failure(&MeshError::Topology(
                "paired baffle trace is missing one face"
            ))
            .kind,
            MeshUpdateFailureKind::PairedTraceMismatch
        );
        assert_eq!(
            classify_failure(&MeshError::Topology(
                "baffle tip is not shared by both faces"
            ))
            .kind,
            MeshUpdateFailureKind::BaffleTipTopology
        );
    }

    #[test]
    fn update_compacts_a_coarsened_vertex_and_preserves_boundary_labels() {
        let options = MeshingOptions {
            target_edge_length: 0.04 / 1.05,
            curve_tolerance: 0.0008,
            minimum_angle_degrees: 12.0,
            ..Default::default()
        };
        let scene = Scene::initial();
        let mesh = Arc::new(mesh_scene(&scene, 0, options).unwrap());
        let mut import = MeshUpdateJob::new(
            Some((mesh.clone(), scene.clone())),
            scene.clone(),
            1,
            options,
        );
        while !matches!(import.phase, Phase::Region(_)) {
            assert!(import.advance(1).is_none());
        }
        let b = import.builder.as_mut().unwrap();
        let triangle = *b
            .triangles
            .iter()
            .find(|t| {
                let points = b.triangle_points(**t);
                let center = (points[0] + points[1] + points[2]) / 3.0;
                center.x > 0.1
                    && center.norm() < 0.25
                    && b.triangle_quality(**t).minimum_angle_degrees > 40.0
                    && t.vertices.iter().all(|v| b.vertices[*v].boundary.is_none())
            })
            .unwrap();
        let points = b.triangle_points(triangle);
        let center = (points[0] + points[1] + points[2]) / 3.0;
        b.insert_point(points[0].lerp(center, 0.65)).unwrap();
        while !b.dirty_edges.is_empty() {
            b.legalize_one().unwrap();
        }
        let quality = b.quality();
        assert!(quality.minimum_angle_degrees >= 12.0 && quality.maximum_edge_length <= 0.04);
        let refined = Arc::new(TriMesh {
            geometry_revision: 0,
            mesh_revision: 0,
            vertices: b.vertices.clone(),
            triangles: b.triangles.clone(),
            boundary_edges: b.boundary_edges.clone(),
            quality,
        });
        let mut next = scene.clone();
        let p = next.obstacles[0].spline.controls()[0];
        next.obstacles[0]
            .spline
            .set_control(0, p + Point2::new(0.0001, 0.0))
            .unwrap();
        let mut job = MeshUpdateJob::new(Some((refined.clone(), scene)), next, 2, options);
        let result = loop {
            if let Some(result) = job.advance(10000) {
                break result.unwrap();
            }
        };
        assert!(result.report.used_local, "{:?}", result.report);
        assert!(result.report.collapsed_vertices > 0, "{:?}", result.report);
        assert_eq!(
            result.mesh.vertices.len(),
            refined.vertices.len() + result.report.inserted_vertices
                - result.report.collapsed_vertices
        );
        let mut used = vec![false; result.mesh.vertices.len()];
        let mut adjacency = BTreeMap::<_, usize>::new();
        for triangle in &result.mesh.triangles {
            for v in triangle.vertices {
                used[v] = true;
            }
            let [a, c, d] = triangle.vertices.map(|v| result.mesh.vertices[v].point);
            assert_eq!(orient2d(a, c, d), PredicateSign::Positive);
            for i in 0..3 {
                *adjacency
                    .entry(edge_key(
                        triangle.vertices[i],
                        triangle.vertices[(i + 1) % 3],
                    ))
                    .or_default() += 1;
            }
        }
        assert!(used.into_iter().all(|v| v));
        assert_eq!(
            result.mesh.vertices.len() as isize - adjacency.len() as isize
                + result.mesh.triangles.len() as isize,
            0
        );
        for edge in result.mesh.boundary_edges {
            assert_eq!(adjacency[&edge_key(edge.vertices[0], edge.vertices[1])], 1);
            assert_eq!(
                result.mesh.vertices[edge.vertices[0]]
                    .boundary
                    .unwrap()
                    .label,
                refined.vertices[refined
                    .boundary_edges
                    .iter()
                    .find(|e| e.label == edge.label)
                    .unwrap()
                    .vertices[0]]
                    .boundary
                    .unwrap()
                    .label
            );
        }
    }
}
