use super::*;
use std::sync::Arc;

/// Immutable spatial target used by one adaptation transaction.
pub trait MeshSizeField: Send + Sync {
    fn target_edge_length(&self, point: Point2, region: RegionId) -> f64;
}

impl<F> MeshSizeField for F
where
    F: Fn(Point2, RegionId) -> f64 + Send + Sync,
{
    fn target_edge_length(&self, point: Point2, region: RegionId) -> f64 {
        self(point, region)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshAdaptationOptions {
    pub meshing: MeshingOptions,
    pub minimum_target_edge_length: f64,
    pub maximum_target_edge_length: f64,
    pub refine_ratio: f64,
    pub collapse_ratio: f64,
    pub max_topology_changes: usize,
    pub max_coarsening_changes: usize,
    pub max_work_units: usize,
    pub cooldown_generations: u64,
}

impl Default for MeshAdaptationOptions {
    fn default() -> Self {
        let meshing = MeshingOptions::default();
        Self {
            minimum_target_edge_length: meshing.target_edge_length * 0.25,
            maximum_target_edge_length: meshing.target_edge_length,
            meshing,
            refine_ratio: 1.05,
            collapse_ratio: 0.35,
            max_topology_changes: 512,
            max_coarsening_changes: usize::MAX,
            max_work_units: 5_000_000,
            cooldown_generations: 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshAdaptationState {
    pub mesh_revision: u64,
    pub generation: u64,
    pub vertex_lineage: Vec<u64>,
    pub last_modified: Vec<u64>,
    next_lineage: u64,
}

impl MeshAdaptationState {
    pub fn from_mesh(mesh: &TriMesh) -> Self {
        let count = mesh.vertices.len() as u64;
        Self {
            mesh_revision: mesh.mesh_revision,
            generation: 0,
            vertex_lineage: (0..count).collect(),
            last_modified: vec![0; count as usize],
            next_lineage: count,
        }
    }

    pub fn next_lineage(&self) -> u64 {
        self.next_lineage
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshAdaptationLimit {
    TopologyChanges,
    Capacity,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshAdaptationReport {
    pub generation: u64,
    pub converged: bool,
    pub limit: Option<MeshAdaptationLimit>,
    pub work_units: usize,
    pub topology_changes: usize,
    pub coarsening_changes: usize,
    pub inserted_vertices: usize,
    pub collapsed_vertices: usize,
    pub boundary_insertions: usize,
    pub boundary_collapses: usize,
    pub skipped_collapses: usize,
    pub preserved_triangles: usize,
    pub original_triangles: usize,
    pub remaining_oversized_triangles: usize,
    pub minimum_target: f64,
    pub maximum_target: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeshAdaptationResult {
    pub mesh: TriMesh,
    pub state: MeshAdaptationState,
    pub report: MeshAdaptationReport,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MeshAdaptationError {
    InvalidOptions,
    InvalidSource(&'static str),
    InvalidState,
    InvalidMeshRevision,
    InvalidTarget { point: Point2, value: f64 },
    WorkLimit,
    Mesh(MeshError),
}

impl std::fmt::Display for MeshAdaptationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidOptions => write!(f, "Invalid mesh-adaptation options"),
            Self::InvalidSource(reason) => write!(f, "Invalid adaptation source: {reason}"),
            Self::InvalidState => write!(f, "Adaptation state does not match the source mesh"),
            Self::InvalidMeshRevision => {
                write!(f, "Target mesh revision must differ from the source")
            }
            Self::InvalidTarget { point, value } => write!(
                f,
                "Invalid target edge length {value} at ({:.6}, {:.6})",
                point.x, point.y
            ),
            Self::WorkLimit => write!(f, "Mesh adaptation work limit reached"),
            Self::Mesh(error) => write!(f, "Mesh adaptation failed: {error}"),
        }
    }
}

impl std::error::Error for MeshAdaptationError {}

impl From<MeshError> for MeshAdaptationError {
    fn from(value: MeshError) -> Self {
        Self::Mesh(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CollapseKey {
    remove: usize,
    keep: usize,
}

#[derive(Clone, Copy)]
struct CollapseCandidate {
    score: u64,
    key: CollapseKey,
    boundary: bool,
}

#[derive(Clone)]
struct BoundaryCollapsePlan {
    keep: usize,
    replacements: Vec<(usize, MeshTriangle)>,
    deleted_triangles: BTreeSet<usize>,
    deleted_edges: BTreeSet<usize>,
    merged: Option<BoundaryEdge>,
}

enum AdaptationInput {
    Scene(Scene),
    Topology(TopologyAdaptationContract),
}

#[derive(Clone)]
struct TopologyBoundaryAtom {
    boundary: PlannedBoundaryEdge,
    label: BoundaryLabel,
    regions: BTreeSet<RegionId>,
}

struct TopologyAdaptationContract {
    plan: TopologyMeshPlan,
    active_regions: BTreeSet<RegionId>,
    trace_points: BTreeMap<crate::TraceVertexId, Point2>,
    atoms: Vec<TopologyBoundaryAtom>,
    seen_traces: BTreeSet<crate::TraceVertexId>,
    seen_regions: BTreeSet<RegionId>,
    coverage: Vec<Vec<[f64; 2]>>,
}

impl TopologyAdaptationContract {
    fn new(plan: &TopologyMeshPlan) -> Self {
        let active_regions = plan
            .domains
            .iter()
            .map(|domain| domain.region)
            .collect::<BTreeSet<_>>();
        let trace_points = plan
            .vertices
            .iter()
            .map(|vertex| (vertex.id, vertex.point))
            .collect::<BTreeMap<_, _>>();
        let mut atoms = vec![];
        for boundary in plan.boundaries.iter().copied().filter(|boundary| {
            !matches!(
                boundary,
                PlannedBoundaryEdge {
                    source: PlannedBoundarySource::Curve {
                        side: CurveTraceSide::Right,
                        ..
                    },
                    behavior: Some(crate::SpanBehavior::Transmitting),
                    ..
                }
            )
        }) {
            let label = topology_label(boundary);
            let mut regions = BTreeSet::from([boundary.region]);
            if matches!(boundary.behavior, Some(crate::SpanBehavior::Transmitting)) {
                for candidate in &plan.boundaries {
                    if same_physical_plan_atom(boundary, *candidate) {
                        regions.insert(candidate.region);
                    }
                }
            }
            atoms.push(TopologyBoundaryAtom {
                boundary,
                label,
                regions,
            });
        }
        let coverage = vec![vec![]; atoms.len()];
        Self {
            plan: plan.clone(),
            active_regions,
            trace_points,
            atoms,
            seen_traces: BTreeSet::new(),
            seen_regions: BTreeSet::new(),
            coverage,
        }
    }

    fn reset_coverage(&mut self) {
        for intervals in &mut self.coverage {
            intervals.clear();
        }
    }
}

fn same_physical_plan_atom(a: PlannedBoundaryEdge, b: PlannedBoundaryEdge) -> bool {
    match (a.source, b.source) {
        (
            PlannedBoundarySource::Curve {
                curve: a_curve,
                span: a_span,
                ..
            },
            PlannedBoundarySource::Curve {
                curve: b_curve,
                span: b_span,
                ..
            },
        ) => {
            a_curve == b_curve
                && a_span == b_span
                && a.parameter[0].min(a.parameter[1]) == b.parameter[0].min(b.parameter[1])
                && a.parameter[0].max(a.parameter[1]) == b.parameter[0].max(b.parameter[1])
        }
        (PlannedBoundarySource::Outer(a_side), PlannedBoundarySource::Outer(b_side)) => {
            a_side == b_side
                && a.parameter[0].min(a.parameter[1]) == b.parameter[0].min(b.parameter[1])
                && a.parameter[0].max(a.parameter[1]) == b.parameter[0].max(b.parameter[1])
        }
        _ => false,
    }
}

fn topology_point_on_atom(atom: PlannedBoundaryEdge, parameter: f64) -> Option<Point2> {
    let denominator = atom.parameter[1] - atom.parameter[0];
    if denominator == 0.0 {
        return None;
    }
    let fraction = (parameter - atom.parameter[0]) / denominator;
    (-1.0e-12..=1.0 + 1.0e-12)
        .contains(&fraction)
        .then(|| atom.points[0].lerp(atom.points[1], fraction.clamp(0.0, 1.0)))
}

fn parameter_close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 32.0 * f64::EPSILON * (1.0 + a.abs().max(b.abs()))
}

fn lower_parameter_vertex(vertices: [usize; 2], parameters: [f64; 2]) -> usize {
    if parameters[0] <= parameters[1] {
        vertices[0]
    } else {
        vertices[1]
    }
}

fn topology_atom_index(contract: &TopologyAdaptationContract, edge: BoundaryEdge) -> Option<usize> {
    let midpoint = 0.5 * (edge.parameters[0] + edge.parameters[1]);
    let mut matches = contract
        .atoms
        .iter()
        .enumerate()
        .filter(|(_, atom)| {
            let [a, b] = atom.boundary.parameter;
            atom.label == edge.label
                && midpoint > a.min(b)
                && midpoint < a.max(b)
                && edge.parameters[0] >= a.min(b)
                && edge.parameters[0] <= a.max(b)
                && edge.parameters[1] >= a.min(b)
                && edge.parameters[1] <= a.max(b)
                && (boundary_adjacency(edge.label) == 2
                    || (edge.parameters[1] - edge.parameters[0]).signum() == (b - a).signum())
        })
        .map(|(index, _)| index);
    let found = matches.next()?;
    matches.next().is_none().then_some(found)
}

fn record_topology_boundary(
    builder: &MeshBuilder,
    contract: &mut TopologyAdaptationContract,
    edge: BoundaryEdge,
) -> Result<(), MeshAdaptationError> {
    let Some(atom_index) = topology_atom_index(contract, edge) else {
        return Err(MeshAdaptationError::InvalidSource(
            "constrained edge does not belong to one topology interval",
        ));
    };
    let atom = &contract.atoms[atom_index];
    let adjacency = builder
        .adjacency
        .get(&edge_key(edge.vertices[0], edge.vertices[1]))
        .ok_or(MeshAdaptationError::InvalidSource(
            "constrained edge has no adjacent triangle",
        ))?;
    if adjacency.len() != boundary_adjacency(edge.label) {
        return Err(MeshAdaptationError::InvalidSource(
            "constrained edge has incorrect topology adjacency",
        ));
    }
    let regions = adjacency
        .iter()
        .map(|(triangle, _)| builder.triangles[*triangle].region)
        .collect::<BTreeSet<_>>();
    if regions != atom.regions {
        return Err(MeshAdaptationError::InvalidSource(
            "constrained edge has incorrect incident regions",
        ));
    }
    let tolerance = 1.0e-10
        * contract
            .plan
            .domain
            .width()
            .max(contract.plan.domain.height());
    for endpoint in 0..2 {
        let parameter = edge.parameters[endpoint];
        let expected = topology_point_on_atom(atom.boundary, parameter).ok_or(
            MeshAdaptationError::InvalidSource("invalid topology interval geometry"),
        )?;
        let vertex = builder.vertices[edge.vertices[endpoint]];
        if (vertex.point - expected).norm() > tolerance {
            return Err(MeshAdaptationError::InvalidSource(
                "constrained edge departed from its topology interval",
            ));
        }
        let planned_endpoint = atom
            .boundary
            .parameter
            .iter()
            .position(|planned| parameter_close(parameter, *planned));
        if let Some(planned_endpoint) = planned_endpoint {
            if vertex.trace != Some(atom.boundary.traces[planned_endpoint]) {
                return Err(MeshAdaptationError::InvalidSource(
                    "topology interval endpoint lost its trace identity",
                ));
            }
        } else if vertex.trace.is_some()
            || vertex.boundary
                != Some(BoundaryPoint {
                    label: edge.label,
                    parameter,
                })
        {
            return Err(MeshAdaptationError::InvalidSource(
                "adaptive boundary vertex has invalid metadata",
            ));
        }
    }
    contract.coverage[atom_index].push([
        edge.parameters[0].min(edge.parameters[1]),
        edge.parameters[0].max(edge.parameters[1]),
    ]);
    Ok(())
}

fn validate_atom_coverage(
    atom: &TopologyBoundaryAtom,
    intervals: &mut [[f64; 2]],
) -> Result<(), MeshAdaptationError> {
    intervals.sort_by(|left, right| {
        left[0]
            .total_cmp(&right[0])
            .then(left[1].total_cmp(&right[1]))
    });
    let start = atom.boundary.parameter[0].min(atom.boundary.parameter[1]);
    let end = atom.boundary.parameter[0].max(atom.boundary.parameter[1]);
    let mut cursor = start;
    for interval in intervals.iter() {
        if !parameter_close(interval[0], cursor) || interval[1] <= interval[0] {
            return Err(MeshAdaptationError::InvalidSource(
                "topology interval coverage has a gap or overlap",
            ));
        }
        cursor = interval[1];
    }
    if intervals.is_empty() || !parameter_close(cursor, end) {
        return Err(MeshAdaptationError::InvalidSource(
            "topology interval coverage is incomplete",
        ));
    }
    Ok(())
}

fn validate_separated_coverage(
    contract: &TopologyAdaptationContract,
) -> Result<(), MeshAdaptationError> {
    for (index, atom) in contract.atoms.iter().enumerate() {
        let BoundaryLabel::Curve {
            curve,
            span,
            side: CurveTraceSide::Left,
            separated: true,
        } = atom.label
        else {
            continue;
        };
        let opposite = contract.atoms.iter().position(|candidate| {
            candidate.label
                == BoundaryLabel::Curve {
                    curve,
                    span,
                    side: CurveTraceSide::Right,
                    separated: true,
                }
                && same_physical_plan_atom(atom.boundary, candidate.boundary)
        });
        if let Some(opposite) = opposite
            && contract.coverage[index] != contract.coverage[opposite]
        {
            return Err(MeshAdaptationError::InvalidSource(
                "separated topology traces have different subdivisions",
            ));
        }
    }
    Ok(())
}

enum AdaptationPhase {
    ImportVertices(usize),
    ImportTriangles(usize),
    ImportBoundary(usize),
    ValidateImportCoverage(usize),
    FindCollapse(usize),
    ApplyCollapse(Option<CollapseCandidate>),
    LegalizeAfterCollapse,
    FindRefine {
        index: usize,
        best: Option<(u64, [usize; 2])>,
    },
    ApplyRefine(Option<[usize; 2]>),
    LegalizeAfterRefine,
    VerifyTriangles(usize, MeshQuality),
    VerifyBoundary(usize, MeshQuality),
    VerifyTopologyCoverage(usize, MeshQuality),
    CompactVertices(usize),
    CompactTriangles(usize),
    CompactBoundary(usize),
    Done,
}

pub struct MeshAdaptationJob {
    source: Arc<TriMesh>,
    input: AdaptationInput,
    target_mesh_revision: u64,
    field: Arc<dyn MeshSizeField>,
    options: MeshAdaptationOptions,
    phase: AdaptationPhase,
    builder: MeshBuilder,
    state: MeshAdaptationState,
    next_generation: u64,
    blocked: BTreeSet<CollapseKey>,
    collapse_candidates: Vec<CollapseCandidate>,
    report: MeshAdaptationReport,
    source_triangle_keys: BTreeSet<[u64; 3]>,
    source_points_by_lineage: BTreeMap<u64, Point2>,
    remap: Vec<usize>,
    compact_vertices: Vec<MeshVertex>,
    compact_lineage: Vec<u64>,
    compact_modified: Vec<u64>,
    output: Option<TriMesh>,
}

impl MeshAdaptationJob {
    pub fn new(
        source: Arc<TriMesh>,
        scene: Scene,
        state: MeshAdaptationState,
        target_mesh_revision: u64,
        field: Arc<dyn MeshSizeField>,
        options: MeshAdaptationOptions,
    ) -> Self {
        let domain = scene.domain;
        Self::from_input(
            source,
            AdaptationInput::Scene(scene),
            domain,
            state,
            target_mesh_revision,
            field,
            options,
        )
    }

    /// Starts a fixed-geometry adaptation transaction from a compiled topology
    /// plan. The plan is cloned so every yielded slice observes one immutable
    /// boundary and trace contract.
    pub fn new_topology(
        source: Arc<TriMesh>,
        plan: &TopologyMeshPlan,
        state: MeshAdaptationState,
        target_mesh_revision: u64,
        field: Arc<dyn MeshSizeField>,
        options: MeshAdaptationOptions,
    ) -> Self {
        Self::from_input(
            source,
            AdaptationInput::Topology(TopologyAdaptationContract::new(plan)),
            plan.domain,
            state,
            target_mesh_revision,
            field,
            options,
        )
    }

    fn from_input(
        source: Arc<TriMesh>,
        input: AdaptationInput,
        domain: crate::DomainRect,
        state: MeshAdaptationState,
        target_mesh_revision: u64,
        field: Arc<dyn MeshSizeField>,
        options: MeshAdaptationOptions,
    ) -> Self {
        let next_generation = state.generation.saturating_add(1);
        let mut report = MeshAdaptationReport {
            generation: next_generation,
            original_triangles: source.triangles.len(),
            minimum_target: f64::INFINITY,
            maximum_target: 0.0,
            ..Default::default()
        };
        if source.triangles.is_empty() {
            report.minimum_target = 0.0;
        }
        Self {
            source,
            input,
            target_mesh_revision,
            field,
            options,
            phase: AdaptationPhase::ImportVertices(0),
            builder: MeshBuilder::new(options.meshing, domain),
            state,
            next_generation,
            blocked: BTreeSet::new(),
            collapse_candidates: Vec::new(),
            report,
            source_triangle_keys: BTreeSet::new(),
            source_points_by_lineage: BTreeMap::new(),
            remap: vec![],
            compact_vertices: vec![],
            compact_lineage: vec![],
            compact_modified: vec![],
            output: None,
        }
    }

    pub fn phase(&self) -> &'static str {
        match self.phase {
            AdaptationPhase::ImportVertices(_)
            | AdaptationPhase::ImportTriangles(_)
            | AdaptationPhase::ImportBoundary(_)
            | AdaptationPhase::ValidateImportCoverage(_) => "Importing adaptive mesh",
            AdaptationPhase::FindCollapse(_) | AdaptationPhase::ApplyCollapse(_) => {
                "Coarsening adaptive mesh"
            }
            AdaptationPhase::LegalizeAfterCollapse | AdaptationPhase::LegalizeAfterRefine => {
                "Legalizing adaptive mesh"
            }
            AdaptationPhase::FindRefine { .. } | AdaptationPhase::ApplyRefine(_) => {
                "Refining adaptive mesh"
            }
            AdaptationPhase::VerifyTriangles(_, _)
            | AdaptationPhase::VerifyBoundary(_, _)
            | AdaptationPhase::VerifyTopologyCoverage(_, _) => "Checking adaptive mesh",
            AdaptationPhase::CompactVertices(_)
            | AdaptationPhase::CompactTriangles(_)
            | AdaptationPhase::CompactBoundary(_) => "Publishing adaptive mesh",
            AdaptationPhase::Done => "Finished",
        }
    }

    pub fn report(&self) -> &MeshAdaptationReport {
        &self.report
    }

    pub fn advance(
        &mut self,
        budget: usize,
    ) -> Option<Result<MeshAdaptationResult, MeshAdaptationError>> {
        for _ in 0..budget {
            if matches!(self.phase, AdaptationPhase::Done) {
                return None;
            }
            self.report.work_units += 1;
            if self.report.work_units > self.options.max_work_units {
                self.phase = AdaptationPhase::Done;
                return Some(Err(MeshAdaptationError::WorkLimit));
            }
            match self.step() {
                Ok(Some(result)) => {
                    self.phase = AdaptationPhase::Done;
                    return Some(Ok(result));
                }
                Ok(None) => {}
                Err(error) => {
                    self.phase = AdaptationPhase::Done;
                    return Some(Err(error));
                }
            }
        }
        None
    }

    fn step(&mut self) -> Result<Option<MeshAdaptationResult>, MeshAdaptationError> {
        let phase = std::mem::replace(&mut self.phase, AdaptationPhase::Done);
        match phase {
            AdaptationPhase::ImportVertices(index) => self.import_vertex(index)?,
            AdaptationPhase::ImportTriangles(index) => self.import_triangle(index)?,
            AdaptationPhase::ImportBoundary(index) => self.import_boundary(index)?,
            AdaptationPhase::ValidateImportCoverage(index) => {
                self.validate_topology_coverage(index, None)?;
            }
            AdaptationPhase::FindCollapse(index) => self.find_collapse(index)?,
            AdaptationPhase::ApplyCollapse(candidate) => self.apply_collapse(candidate)?,
            AdaptationPhase::LegalizeAfterCollapse => {
                if self.builder.dirty_edges.is_empty() {
                    self.phase = AdaptationPhase::FindCollapse(0);
                } else {
                    self.builder.legalize_one()?;
                    self.phase = AdaptationPhase::LegalizeAfterCollapse;
                }
            }
            AdaptationPhase::FindRefine { index, best } => self.find_refine(index, best)?,
            AdaptationPhase::ApplyRefine(edge) => self.apply_refine(edge)?,
            AdaptationPhase::LegalizeAfterRefine => {
                if self.builder.dirty_edges.is_empty() {
                    self.phase = AdaptationPhase::FindRefine {
                        index: 0,
                        best: None,
                    };
                } else {
                    self.builder.legalize_one()?;
                    self.phase = AdaptationPhase::LegalizeAfterRefine;
                }
            }
            AdaptationPhase::VerifyTriangles(index, quality) => {
                self.verify_triangle(index, quality)?;
            }
            AdaptationPhase::VerifyBoundary(index, quality) => {
                self.verify_boundary(index, quality)?;
            }
            AdaptationPhase::VerifyTopologyCoverage(index, quality) => {
                self.validate_topology_coverage(index, Some(quality))?;
            }
            AdaptationPhase::CompactVertices(index) => self.compact_vertex(index),
            AdaptationPhase::CompactTriangles(index) => self.compact_triangle(index),
            AdaptationPhase::CompactBoundary(index) => {
                return self.compact_boundary(index);
            }
            AdaptationPhase::Done => unreachable!(),
        }
        Ok(None)
    }

    fn valid_options(&self) -> bool {
        super::valid_options(self.options.meshing)
            && self.options.minimum_target_edge_length.is_finite()
            && self.options.maximum_target_edge_length.is_finite()
            && self.options.minimum_target_edge_length > 0.0
            && self.options.maximum_target_edge_length >= self.options.minimum_target_edge_length
            && self.options.refine_ratio.is_finite()
            && self.options.refine_ratio > 1.0
            && self.options.collapse_ratio.is_finite()
            && self.options.collapse_ratio > 0.0
            && self.options.collapse_ratio < 1.0
            && self.options.max_topology_changes > 0
            && self.options.max_work_units > 0
    }

    fn import_vertex(&mut self, index: usize) -> Result<(), MeshAdaptationError> {
        if index == 0 {
            if !self.valid_options() {
                return Err(MeshAdaptationError::InvalidOptions);
            }
            if self.source.vertices.is_empty() || self.source.triangles.is_empty() {
                return Err(MeshAdaptationError::InvalidSource("empty mesh"));
            }
            match &self.input {
                AdaptationInput::Scene(scene) if !scene.structure_valid() => {
                    return Err(MeshAdaptationError::InvalidSource(
                        "invalid scene structure",
                    ));
                }
                AdaptationInput::Topology(contract)
                    if contract.active_regions.is_empty() || contract.atoms.is_empty() =>
                {
                    return Err(MeshAdaptationError::InvalidSource(
                        "empty topology adaptation contract",
                    ));
                }
                _ => {}
            }
            if self.target_mesh_revision == self.source.mesh_revision {
                return Err(MeshAdaptationError::InvalidMeshRevision);
            }
            if self.state.mesh_revision != self.source.mesh_revision
                || self.state.vertex_lineage.len() != self.source.vertices.len()
                || self.state.last_modified.len() != self.source.vertices.len()
                || self
                    .state
                    .vertex_lineage
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .len()
                    != self.source.vertices.len()
                || self
                    .state
                    .vertex_lineage
                    .iter()
                    .any(|lineage| *lineage >= self.state.next_lineage)
            {
                return Err(MeshAdaptationError::InvalidState);
            }
            if let AdaptationInput::Scene(scene) = &self.input {
                self.builder.loop_ids = scene.obstacles.iter().map(|loop_| loop_.id).collect();
                self.builder.loop_roles = scene.obstacles.iter().map(|loop_| loop_.role).collect();
                self.builder.internal_boundary_ids = scene
                    .internal_boundaries
                    .iter()
                    .map(|boundary| boundary.id)
                    .collect();
                self.builder.internal_boundary_regions = scene
                    .internal_boundaries
                    .iter()
                    .map(|boundary| boundary.region)
                    .collect();
            }
        }
        if index == self.source.vertices.len() {
            self.phase = AdaptationPhase::ImportTriangles(0);
            return Ok(());
        }
        let vertex = self.source.vertices[index];
        if !vertex.point.finite() {
            return Err(MeshAdaptationError::InvalidSource("non-finite vertex"));
        }
        let added = self.builder.add_vertex(vertex.point, vertex.boundary)?;
        self.builder.vertices[added].trace = vertex.trace;
        if let AdaptationInput::Topology(contract) = &mut self.input
            && let Some(trace) = vertex.trace
            && (contract.trace_points.get(&trace).copied() != Some(vertex.point)
                || !contract.seen_traces.insert(trace))
        {
            return Err(MeshAdaptationError::InvalidSource(
                "invalid topology trace vertex",
            ));
        }
        self.source_points_by_lineage
            .insert(self.state.vertex_lineage[index], vertex.point);
        self.phase = AdaptationPhase::ImportVertices(index + 1);
        Ok(())
    }

    fn import_triangle(&mut self, index: usize) -> Result<(), MeshAdaptationError> {
        if index == self.source.triangles.len() {
            self.builder.dirty_edges.clear();
            self.builder.queued_edges.clear();
            self.builder.bad_triangles.clear();
            self.builder.scores.fill(None);
            self.phase = AdaptationPhase::ImportBoundary(0);
            return Ok(());
        }
        let triangle = self.source.triangles[index];
        if let AdaptationInput::Topology(contract) = &mut self.input {
            if !contract.active_regions.contains(&triangle.region) {
                return Err(MeshAdaptationError::InvalidSource(
                    "triangle uses an inactive topology region",
                ));
            }
            contract.seen_regions.insert(triangle.region);
        }
        if triangle
            .vertices
            .iter()
            .any(|vertex| *vertex >= self.builder.vertices.len())
            || orient2d(
                self.builder.point(triangle.vertices[0]),
                self.builder.point(triangle.vertices[1]),
                self.builder.point(triangle.vertices[2]),
            ) != PredicateSign::Positive
        {
            return Err(MeshAdaptationError::InvalidSource("invalid triangle"));
        }
        self.builder.push_triangle(triangle)?;
        let mut lineage = triangle
            .vertices
            .map(|vertex| self.state.vertex_lineage[vertex]);
        lineage.sort_unstable();
        self.source_triangle_keys.insert(lineage);
        self.phase = AdaptationPhase::ImportTriangles(index + 1);
        Ok(())
    }

    fn import_boundary(&mut self, index: usize) -> Result<(), MeshAdaptationError> {
        if index == self.source.boundary_edges.len() {
            self.builder.dirty_edges.clear();
            self.builder.queued_edges.clear();
            self.builder.bad_triangles.clear();
            self.builder.scores.fill(None);
            self.phase = if matches!(self.input, AdaptationInput::Topology(_)) {
                AdaptationPhase::ValidateImportCoverage(0)
            } else {
                AdaptationPhase::FindCollapse(0)
            };
            return Ok(());
        }
        let edge = self.source.boundary_edges[index];
        if edge.vertices[0] >= self.builder.vertices.len()
            || edge.vertices[1] >= self.builder.vertices.len()
            || edge.vertices[0] == edge.vertices[1]
            || !edge
                .parameters
                .iter()
                .all(|parameter| parameter.is_finite())
        {
            return Err(MeshAdaptationError::InvalidSource(
                "invalid constrained edge",
            ));
        }
        if let AdaptationInput::Topology(contract) = &mut self.input {
            record_topology_boundary(&self.builder, contract, edge)?;
        }
        self.builder.add_boundary_edge(edge);
        self.phase = AdaptationPhase::ImportBoundary(index + 1);
        Ok(())
    }

    fn validate_topology_coverage(
        &mut self,
        index: usize,
        quality: Option<MeshQuality>,
    ) -> Result<(), MeshAdaptationError> {
        let AdaptationInput::Topology(contract) = &mut self.input else {
            return Err(MeshAdaptationError::InvalidSource(
                "topology coverage requested for a legacy scene",
            ));
        };
        if index == contract.atoms.len() {
            if contract.seen_traces.len() != contract.trace_points.len()
                || contract.seen_regions != contract.active_regions
            {
                return Err(MeshAdaptationError::InvalidSource(
                    "topology trace or region coverage is incomplete",
                ));
            }
            validate_separated_coverage(contract)?;
            contract.reset_coverage();
            if let Some(quality) = quality {
                self.begin_publish(quality);
            } else {
                self.phase = AdaptationPhase::FindCollapse(0);
            }
            return Ok(());
        }
        validate_atom_coverage(&contract.atoms[index], &mut contract.coverage[index])?;
        self.phase = match quality {
            Some(quality) => AdaptationPhase::VerifyTopologyCoverage(index + 1, quality),
            None => AdaptationPhase::ValidateImportCoverage(index + 1),
        };
        Ok(())
    }

    fn target(&mut self, point: Point2, region: RegionId) -> Result<f64, MeshAdaptationError> {
        let value = self.field.target_edge_length(point, region);
        if !value.is_finite()
            || value < self.options.minimum_target_edge_length
            || value > self.options.maximum_target_edge_length
        {
            return Err(MeshAdaptationError::InvalidTarget { point, value });
        }
        self.report.minimum_target = self.report.minimum_target.min(value);
        self.report.maximum_target = self.report.maximum_target.max(value);
        Ok(value)
    }

    fn triangle_target(&mut self, triangle: MeshTriangle) -> Result<f64, MeshAdaptationError> {
        let points = self.builder.triangle_points(triangle);
        let mut target = self.target((points[0] + points[1] + points[2]) / 3.0, triangle.region)?;
        for point in points {
            target = target.min(self.target(point, triangle.region)?);
        }
        for [a, b] in [[0, 1], [1, 2], [2, 0]] {
            target = target.min(self.target(points[a].lerp(points[b], 0.5), triangle.region)?);
        }
        Ok(target)
    }

    fn vertex_ready(&self, vertex: usize) -> bool {
        self.next_generation
            .saturating_sub(self.state.last_modified[vertex])
            >= self.options.cooldown_generations
    }

    fn find_collapse(&mut self, index: usize) -> Result<(), MeshAdaptationError> {
        if self.report.topology_changes >= self.options.max_topology_changes {
            self.collapse_candidates.clear();
            self.report.limit = Some(MeshAdaptationLimit::TopologyChanges);
            self.phase = AdaptationPhase::FindRefine {
                index: 0,
                best: None,
            };
            return Ok(());
        }
        if self.report.coarsening_changes >= self.options.max_coarsening_changes {
            self.collapse_candidates.clear();
            self.phase = AdaptationPhase::FindRefine {
                index: 0,
                best: None,
            };
            return Ok(());
        }
        if index == self.builder.vertices.len() {
            self.collapse_candidates.sort_unstable_by(|left, right| {
                (right.score, right.key).cmp(&(left.score, left.key))
            });
            self.phase = AdaptationPhase::ApplyCollapse(self.collapse_candidates.pop());
            return Ok(());
        }
        if self.builder.incident[index].is_empty()
            || self.builder.vertices[index].trace.is_some()
            || !self.vertex_ready(index)
        {
            self.phase = AdaptationPhase::FindCollapse(index + 1);
            return Ok(());
        }
        let candidate = if self.builder.vertices[index].boundary.is_some() {
            self.boundary_candidate(index)?
        } else {
            self.interior_candidate(index)?
        };
        if let Some(candidate) = candidate
            && !self.blocked.contains(&candidate.key)
        {
            self.collapse_candidates.push(candidate);
        }
        self.phase = AdaptationPhase::FindCollapse(index + 1);
        Ok(())
    }

    fn interior_candidate(
        &mut self,
        remove: usize,
    ) -> Result<Option<CollapseCandidate>, MeshAdaptationError> {
        let neighbors = self.builder.incident[remove]
            .iter()
            .flat_map(|triangle| self.builder.triangles[*triangle].vertices)
            .filter(|neighbor| {
                *neighbor != remove
                    && self.builder.vertices[*neighbor].boundary.is_none()
                    && self.builder.vertices[*neighbor].trace.is_none()
                    && self.state.vertex_lineage[remove] > self.state.vertex_lineage[*neighbor]
            })
            .collect::<BTreeSet<_>>();
        let mut best: Option<(f64, usize)> = None;
        for keep in neighbors {
            let midpoint = self
                .builder
                .point(remove)
                .lerp(self.builder.point(keep), 0.5);
            let region = self.builder.triangles
                [*self.builder.incident[remove].iter().next().unwrap()]
            .region;
            let target = self.target(midpoint, region)?;
            let ratio = (self.builder.point(remove) - self.builder.point(keep)).norm() / target;
            if ratio < self.options.collapse_ratio && best.is_none_or(|current| ratio < current.0) {
                best = Some((ratio, keep));
            }
        }
        Ok(best.map(|(ratio, keep)| CollapseCandidate {
            score: ratio.to_bits(),
            key: CollapseKey { remove, keep },
            boundary: false,
        }))
    }

    fn boundary_candidate(
        &mut self,
        remove: usize,
    ) -> Result<Option<CollapseCandidate>, MeshAdaptationError> {
        let Some(point) = self.builder.vertices[remove].boundary else {
            return Ok(None);
        };
        if !self.canonical_boundary_face(point.label) || self.is_boundary_anchor(point) {
            return Ok(None);
        }
        let Some((incoming, outgoing)) = self.face_edges(remove, point.label) else {
            return Ok(None);
        };
        let endpoints = [
            self.builder.boundary_edges[incoming].vertices[0],
            self.builder.boundary_edges[outgoing].vertices[1],
        ];
        let keep = lower_parameter_vertex(
            endpoints,
            [
                self.builder.boundary_edges[incoming].parameters[0],
                self.builder.boundary_edges[outgoing].parameters[1],
            ],
        );
        let length = (self.builder.point(endpoints[0]) - self.builder.point(endpoints[1])).norm();
        let region =
            self.builder.triangles[*self.builder.incident[remove].iter().next().unwrap()].region;
        let target = self.target(
            self.builder
                .point(endpoints[0])
                .lerp(self.builder.point(endpoints[1]), 0.5),
            region,
        )?;
        let short = [incoming, outgoing].iter().any(|edge| {
            let vertices = self.builder.boundary_edges[*edge].vertices;
            (self.builder.point(vertices[0]) - self.builder.point(vertices[1])).norm()
                < self.options.collapse_ratio * target
        });
        if !short || length > target {
            return Ok(None);
        }
        Ok(Some(CollapseCandidate {
            score: (length / target).to_bits(),
            key: CollapseKey { remove, keep },
            boundary: true,
        }))
    }

    fn canonical_boundary_face(&self, label: BoundaryLabel) -> bool {
        match label {
            BoundaryLabel::InternalBoundary {
                side: InternalBoundarySide::Right,
                ..
            }
            | BoundaryLabel::Wall {
                side: BoundarySide::Interior,
                ..
            } => false,
            BoundaryLabel::Curve {
                side: CurveTraceSide::Right,
                separated: true,
                ..
            } => self.paired_label(label).is_none(),
            _ => true,
        }
    }

    fn is_boundary_anchor(&self, point: BoundaryPoint) -> bool {
        let parameter = point.parameter;
        if let AdaptationInput::Topology(contract) = &self.input {
            return contract.atoms.iter().any(|atom| {
                atom.label == point.label
                    && atom
                        .boundary
                        .parameter
                        .iter()
                        .any(|endpoint| parameter_close(parameter, *endpoint))
            });
        }
        let AdaptationInput::Scene(scene) = &self.input else {
            unreachable!()
        };
        match point.label {
            BoundaryLabel::Outer(_) => parameter == 0.0 || parameter == 1.0,
            BoundaryLabel::Obstacle(id)
            | BoundaryLabel::MaterialInterface(id)
            | BoundaryLabel::Wall { loop_id: id, .. } => scene
                .obstacles
                .iter()
                .find(|obstacle| obstacle.id == id)
                .is_none_or(|obstacle| {
                    let mut breakpoint = 0.0;
                    if parameter == 0.0 || parameter == obstacle.spline.period() {
                        return true;
                    }
                    obstacle.spline.intervals().iter().any(|interval| {
                        breakpoint += interval;
                        parameter == breakpoint
                    })
                }),
            BoundaryLabel::InternalBoundary { id, .. } => scene
                .internal_boundaries
                .iter()
                .find(|boundary| boundary.id == id)
                .is_none_or(|boundary| {
                    let mut breakpoint = 0.0;
                    if parameter == 0.0 || parameter == boundary.spline.period() {
                        return true;
                    }
                    boundary.spline.intervals().iter().any(|interval| {
                        breakpoint += interval;
                        parameter == breakpoint
                    })
                }),
            BoundaryLabel::OpenMaterialInterface(id) => scene
                .material_interfaces
                .iter()
                .find(|interface| interface.id == id)
                .is_none_or(|interface| {
                    let crate::InterfaceSpline::Open(spline) = &interface.spline else {
                        return true;
                    };
                    let mut breakpoint = 0.0;
                    if parameter == 0.0 || parameter == spline.period() {
                        return true;
                    }
                    spline.intervals().iter().any(|interval| {
                        breakpoint += interval;
                        parameter == breakpoint
                    })
                }),
            BoundaryLabel::Curve { .. } => true,
        }
    }

    fn face_edges(&self, vertex: usize, label: BoundaryLabel) -> Option<(usize, usize)> {
        let incoming = self
            .builder
            .boundary_edges
            .iter()
            .position(|edge| edge.label == label && edge.vertices[1] == vertex)?;
        let outgoing = self
            .builder
            .boundary_edges
            .iter()
            .position(|edge| edge.label == label && edge.vertices[0] == vertex)?;
        Some((incoming, outgoing))
    }

    fn apply_collapse(
        &mut self,
        candidate: Option<CollapseCandidate>,
    ) -> Result<(), MeshAdaptationError> {
        let Some(candidate) = candidate else {
            self.phase = AdaptationPhase::FindRefine {
                index: 0,
                best: None,
            };
            return Ok(());
        };
        let pair_count = if candidate.boundary
            && self.builder.vertices[candidate.key.remove]
                .boundary
                .and_then(|point| self.paired_label(point.label))
                .is_some()
        {
            2
        } else {
            1
        };
        let changed = if candidate.boundary {
            self.collapse_boundary(candidate.key.remove)?
        } else {
            self.collapse_interior(candidate.key)?
        };
        if changed {
            self.collapse_candidates.clear();
            self.report.topology_changes += 1;
            self.report.coarsening_changes += 1;
            self.report.collapsed_vertices += pair_count;
            self.phase = AdaptationPhase::LegalizeAfterCollapse;
        } else {
            self.report.skipped_collapses += 1;
            self.blocked.insert(candidate.key);
            self.phase = AdaptationPhase::ApplyCollapse(self.collapse_candidates.pop());
        }
        Ok(())
    }

    fn collapse_interior(&mut self, key: CollapseKey) -> Result<bool, MeshAdaptationError> {
        let Some(plan) = self.collapse_plan(key.remove, key.keep)? else {
            return Ok(false);
        };
        self.apply_collapse_plans(&[plan]);
        Ok(true)
    }

    fn collapse_boundary(&mut self, remove: usize) -> Result<bool, MeshAdaptationError> {
        let point = self.builder.vertices[remove].boundary.unwrap();
        let Some(plan) = self.boundary_plan(remove, point.label)? else {
            return Ok(false);
        };
        let mut plans = vec![plan];
        if let Some(pair_label) = self.paired_label(point.label) {
            let partner = self
                .builder
                .vertices
                .iter()
                .enumerate()
                .find(|(index, vertex)| {
                    *index != remove
                        && vertex.boundary.is_some_and(|candidate| {
                            candidate.label == pair_label
                                && if matches!(self.input, AdaptationInput::Topology(_)) {
                                    parameter_close(candidate.parameter, point.parameter)
                                } else {
                                    vertex.point == self.builder.point(remove)
                                }
                        })
                })
                .map(|(index, _)| index)
                .ok_or(MeshAdaptationError::InvalidSource(
                    "paired boundary vertex is missing",
                ))?;
            let Some(pair) = self.boundary_plan(partner, pair_label)? else {
                return Ok(false);
            };
            if self.builder.point(plans[0].keep) != self.builder.point(pair.keep) {
                return Err(MeshAdaptationError::InvalidSource(
                    "paired boundary collapses choose different parameters",
                ));
            }
            plans.push(pair);
        }
        self.apply_collapse_plans(&plans);
        self.report.boundary_collapses += 1;
        Ok(true)
    }

    fn paired_label(&self, label: BoundaryLabel) -> Option<BoundaryLabel> {
        let pair = match label {
            BoundaryLabel::InternalBoundary {
                id,
                side: InternalBoundarySide::Left,
            } => Some(BoundaryLabel::InternalBoundary {
                id,
                side: InternalBoundarySide::Right,
            }),
            BoundaryLabel::Wall {
                loop_id,
                side: BoundarySide::Exterior,
            } => Some(BoundaryLabel::Wall {
                loop_id,
                side: BoundarySide::Interior,
            }),
            BoundaryLabel::Curve {
                curve,
                span,
                side,
                separated: true,
            } => Some(BoundaryLabel::Curve {
                curve,
                span,
                side: match side {
                    CurveTraceSide::Left => CurveTraceSide::Right,
                    CurveTraceSide::Right => CurveTraceSide::Left,
                },
                separated: true,
            }),
            _ => None,
        }?;
        if matches!(label, BoundaryLabel::Curve { .. })
            && let AdaptationInput::Topology(contract) = &self.input
            && !contract.atoms.iter().any(|atom| atom.label == pair)
        {
            return None;
        }
        Some(pair)
    }

    fn boundary_plan(
        &mut self,
        remove: usize,
        label: BoundaryLabel,
    ) -> Result<Option<BoundaryCollapsePlan>, MeshAdaptationError> {
        let Some((incoming, outgoing)) = self.face_edges(remove, label) else {
            return Err(MeshAdaptationError::InvalidSource(
                "boundary vertex does not have one incoming and outgoing edge",
            ));
        };
        let before = self.builder.boundary_edges[incoming];
        let after = self.builder.boundary_edges[outgoing];
        let endpoints = [before.vertices[0], after.vertices[1]];
        let keep = lower_parameter_vertex(endpoints, [before.parameters[0], after.parameters[1]]);
        let merged = BoundaryEdge {
            vertices: endpoints,
            label,
            parameters: [before.parameters[0], after.parameters[1]],
        };
        if !self.merged_curve_is_acceptable(merged)? {
            return Ok(None);
        }
        let Some(mut plan) = self.collapse_plan(remove, keep)? else {
            return Ok(None);
        };
        plan.deleted_edges = BTreeSet::from([incoming, outgoing]);
        plan.merged = Some(merged);
        Ok(Some(plan))
    }

    fn merged_curve_is_acceptable(
        &mut self,
        edge: BoundaryEdge,
    ) -> Result<bool, MeshAdaptationError> {
        let a = self.builder.point(edge.vertices[0]);
        let d = self.builder.point(edge.vertices[1]);
        let region = self.builder.triangles[*self.builder.incident[edge.vertices[0]]
            .iter()
            .next()
            .ok_or(MeshAdaptationError::InvalidSource("orphan boundary vertex"))?]
        .region;
        let target = self.target(a.lerp(d, 0.5), region)?;
        if (d - a).norm() > target {
            return Ok(false);
        }
        if let AdaptationInput::Topology(contract) = &self.input {
            return Ok(topology_atom_index(contract, edge).is_some());
        }
        let AdaptationInput::Scene(scene) = &self.input else {
            unreachable!()
        };
        let [t0, t1] = edge.parameters;
        let hull = match edge.label {
            BoundaryLabel::Outer(_) => return Ok(true),
            BoundaryLabel::Obstacle(id)
            | BoundaryLabel::MaterialInterface(id)
            | BoundaryLabel::Wall { loop_id: id, .. } => {
                let spline = &scene
                    .obstacles
                    .iter()
                    .find(|obstacle| obstacle.id == id)
                    .ok_or(MeshAdaptationError::InvalidSource("unknown loop boundary"))?
                    .spline;
                [
                    spline.evaluate(t0),
                    spline.evaluate(t0) + spline.derivative(t0, 1) * ((t1 - t0) / 3.0),
                    spline.evaluate(t1) - spline.derivative(t1, 1) * ((t1 - t0) / 3.0),
                    spline.evaluate(t1),
                ]
            }
            BoundaryLabel::InternalBoundary { id, .. } => {
                let spline = &scene
                    .internal_boundaries
                    .iter()
                    .find(|boundary| boundary.id == id)
                    .ok_or(MeshAdaptationError::InvalidSource("unknown open boundary"))?
                    .spline;
                [
                    spline.evaluate(t0),
                    spline.evaluate(t0) + spline.derivative(t0, 1) * ((t1 - t0) / 3.0),
                    spline.evaluate(t1) - spline.derivative(t1, 1) * ((t1 - t0) / 3.0),
                    spline.evaluate(t1),
                ]
            }
            BoundaryLabel::OpenMaterialInterface(id) => {
                let interface = scene
                    .material_interfaces
                    .iter()
                    .find(|interface| interface.id == id)
                    .ok_or(MeshAdaptationError::InvalidSource(
                        "unknown material interface",
                    ))?;
                let crate::InterfaceSpline::Open(spline) = &interface.spline else {
                    return Err(MeshAdaptationError::InvalidSource(
                        "unsupported closed graph interface",
                    ));
                };
                [
                    spline.evaluate(t0),
                    spline.evaluate(t0) + spline.derivative(t0, 1) * ((t1 - t0) / 3.0),
                    spline.evaluate(t1) - spline.derivative(t1, 1) * ((t1 - t0) / 3.0),
                    spline.evaluate(t1),
                ]
            }
            BoundaryLabel::Curve { .. } => unreachable!(),
        };
        Ok(hull.iter().all(|point| {
            crate::point_segment_distance(*point, a, d)
                <= self.options.meshing.curve_tolerance * (1.0 + 1e-9)
        }))
    }

    fn collapse_plan(
        &mut self,
        remove: usize,
        keep: usize,
    ) -> Result<Option<BoundaryCollapsePlan>, MeshAdaptationError> {
        if remove == keep
            || self.builder.incident[remove].is_empty()
            || !self.builder.adjacency.contains_key(&edge_key(remove, keep))
        {
            return Ok(None);
        }
        let neighbors = |builder: &MeshBuilder, vertex: usize| {
            builder.incident[vertex]
                .iter()
                .flat_map(|triangle| builder.triangles[*triangle].vertices)
                .filter(|neighbor| *neighbor != vertex)
                .collect::<BTreeSet<_>>()
        };
        let shared = neighbors(&self.builder, remove)
            .intersection(&neighbors(&self.builder, keep))
            .count();
        let mut replacements = vec![];
        let mut deleted_triangles = BTreeSet::new();
        let incident = self.builder.incident[remove]
            .iter()
            .copied()
            .collect::<Vec<_>>();
        for index in incident {
            let triangle = self.builder.triangles[index];
            if triangle.vertices.contains(&keep) {
                deleted_triangles.insert(index);
                continue;
            }
            let replacement = MeshTriangle {
                vertices: triangle
                    .vertices
                    .map(|vertex| if vertex == remove { keep } else { vertex }),
                region: triangle.region,
            };
            let points = self.builder.triangle_points(replacement);
            if orient2d(points[0], points[1], points[2]) != PredicateSign::Positive {
                return Ok(None);
            }
            let quality = self.builder.triangle_quality(replacement);
            let target = self.triangle_target(replacement)?;
            let touches_trace = replacement.vertices.iter().any(|vertex| {
                matches!(
                    self.builder.vertices[*vertex]
                        .boundary
                        .map(|point| point.label),
                    Some(
                        BoundaryLabel::InternalBoundary { .. }
                            | BoundaryLabel::Wall { .. }
                            | BoundaryLabel::Curve {
                                separated: true,
                                ..
                            }
                    )
                ) || self.builder.boundary_edges.iter().any(|edge| {
                    edge.vertices.contains(vertex)
                        && matches!(
                            edge.label,
                            BoundaryLabel::Curve {
                                separated: true,
                                ..
                            }
                        )
                })
            });
            if quality.maximum_edge_length > target
                || (!touches_trace
                    && quality.minimum_angle_degrees + 1e-9
                        < self.options.meshing.minimum_angle_degrees)
            {
                return Ok(None);
            }
            replacements.push((index, replacement));
        }
        if deleted_triangles.is_empty() || shared != deleted_triangles.len() {
            return Ok(None);
        }
        Ok(Some(BoundaryCollapsePlan {
            keep,
            replacements,
            deleted_triangles,
            deleted_edges: BTreeSet::new(),
            merged: None,
        }))
    }

    fn apply_collapse_plans(&mut self, plans: &[BoundaryCollapsePlan]) {
        let deleted = plans
            .iter()
            .flat_map(|plan| plan.deleted_triangles.iter().copied())
            .collect::<BTreeSet<_>>();
        for plan in plans {
            for (index, triangle) in &plan.replacements {
                if !deleted.contains(index) {
                    self.builder.replace_triangle(*index, *triangle);
                }
            }
        }
        self.remove_triangles(deleted);
        let deleted_edges = plans
            .iter()
            .flat_map(|plan| plan.deleted_edges.iter().copied())
            .collect::<BTreeSet<_>>();
        self.remove_boundary_edges(deleted_edges);
        for plan in plans {
            if let Some(merged) = plan.merged {
                self.builder.add_boundary_edge(merged);
            }
            self.state.last_modified[plan.keep] = self.next_generation;
        }
    }

    fn remove_triangles(&mut self, mut deleted: BTreeSet<usize>) {
        while let Some(index) = deleted.pop_last() {
            if index >= self.builder.triangles.len() {
                continue;
            }
            let last = self.builder.triangles.len() - 1;
            self.builder.unregister_triangle(index);
            if last != index {
                self.builder.unregister_triangle(last);
            }
            self.builder.triangles.swap_remove(index);
            self.builder.scores.swap_remove(index);
            if last != index {
                if deleted.remove(&last) {
                    deleted.insert(index);
                } else {
                    self.builder.register_triangle(index);
                }
            }
        }
    }

    fn remove_boundary_edges(&mut self, mut deleted: BTreeSet<usize>) {
        while let Some(index) = deleted.pop_last() {
            if index >= self.builder.boundary_edges.len() {
                continue;
            }
            let last = self.builder.boundary_edges.len() - 1;
            let removed = self.builder.boundary_edges[index];
            self.builder
                .boundary_keys
                .remove(&edge_key(removed.vertices[0], removed.vertices[1]));
            self.builder.boundary_edges.swap_remove(index);
            if last != index && deleted.remove(&last) {
                deleted.insert(index);
            }
        }
    }

    fn find_refine(
        &mut self,
        index: usize,
        mut best: Option<(u64, [usize; 2])>,
    ) -> Result<(), MeshAdaptationError> {
        if index == self.builder.triangles.len() {
            if self.report.topology_changes >= self.options.max_topology_changes {
                self.report.limit = Some(MeshAdaptationLimit::TopologyChanges);
                self.phase = AdaptationPhase::ApplyRefine(None);
            } else {
                self.phase = AdaptationPhase::ApplyRefine(best.map(|(_, edge)| edge));
            }
            return Ok(());
        }
        let triangle = self.builder.triangles[index];
        let target = self.triangle_target(triangle)?;
        let edges = [
            [triangle.vertices[0], triangle.vertices[1]],
            [triangle.vertices[1], triangle.vertices[2]],
            [triangle.vertices[2], triangle.vertices[0]],
        ];
        let (length, edge) = edges
            .into_iter()
            .map(|edge| {
                (
                    (self.builder.point(edge[0]) - self.builder.point(edge[1])).norm(),
                    edge,
                )
            })
            .max_by(|left, right| left.0.total_cmp(&right.0))
            .unwrap();
        let ratio = length / target;
        if ratio > self.options.refine_ratio
            && best.is_none_or(|current| ratio.to_bits() > current.0)
        {
            best = Some((ratio.to_bits(), edge));
        }
        self.phase = AdaptationPhase::FindRefine {
            index: index + 1,
            best,
        };
        Ok(())
    }

    fn apply_refine(&mut self, edge: Option<[usize; 2]>) -> Result<(), MeshAdaptationError> {
        let Some(edge) = edge else {
            self.report.remaining_oversized_triangles = self.count_oversized()?;
            self.report.converged =
                self.report.remaining_oversized_triangles == 0 && self.report.limit.is_none();
            if let AdaptationInput::Topology(contract) = &mut self.input {
                contract.seen_regions.clear();
            }
            self.phase = AdaptationPhase::VerifyTriangles(
                0,
                MeshQuality {
                    minimum_angle_degrees: 180.0,
                    maximum_edge_length: 0.0,
                },
            );
            return Ok(());
        };
        if self.builder.vertices.len() >= self.options.meshing.max_vertices
            || self.builder.triangles.len() + 4 > self.options.meshing.max_triangles
        {
            self.report.limit = Some(MeshAdaptationLimit::Capacity);
            self.phase = AdaptationPhase::ApplyRefine(None);
            return Ok(());
        }
        let key = edge_key(edge[0], edge[1]);
        if let Some(found) = self
            .builder
            .boundary_edges
            .iter()
            .position(|boundary| edge_key(boundary.vertices[0], boundary.vertices[1]) == key)
        {
            let boundary = self.builder.boundary_edges[found];
            let index = if self.canonical_boundary_face(boundary.label) {
                found
            } else {
                let canonical = match boundary.label {
                    BoundaryLabel::InternalBoundary {
                        id,
                        side: InternalBoundarySide::Right,
                    } => BoundaryLabel::InternalBoundary {
                        id,
                        side: InternalBoundarySide::Left,
                    },
                    BoundaryLabel::Wall {
                        loop_id,
                        side: BoundarySide::Interior,
                    } => BoundaryLabel::Wall {
                        loop_id,
                        side: BoundarySide::Exterior,
                    },
                    BoundaryLabel::Curve {
                        curve,
                        span,
                        side: CurveTraceSide::Right,
                        separated: true,
                    } => BoundaryLabel::Curve {
                        curve,
                        span,
                        side: CurveTraceSide::Left,
                        separated: true,
                    },
                    _ => boundary.label,
                };
                self.builder
                    .boundary_edges
                    .iter()
                    .position(|candidate| self.opposite_trace_edge(boundary, *candidate, canonical))
                    .ok_or(MeshAdaptationError::InvalidSource(
                        "canonical paired edge is missing",
                    ))?
            };
            self.split_boundary(index)?;
            self.report.boundary_insertions += 1;
        } else {
            let point = self
                .builder
                .point(edge[0])
                .lerp(self.builder.point(edge[1]), 0.5);
            let vertex = self.builder.add_vertex(point, None)?;
            self.add_lineage(vertex);
            self.builder.split_edge(edge, vertex)?;
            self.report.inserted_vertices += 1;
        }
        self.report.topology_changes += 1;
        self.phase = AdaptationPhase::LegalizeAfterRefine;
        Ok(())
    }

    fn split_boundary(&mut self, index: usize) -> Result<(), MeshAdaptationError> {
        let edge = self.builder.boundary_edges[index];
        let parameter = 0.5 * (edge.parameters[0] + edge.parameters[1]);
        let point = self.boundary_point(edge.label, parameter)?;
        if let Some(pair_label) = self.paired_label(edge.label) {
            let pair_index = self
                .builder
                .boundary_edges
                .iter()
                .position(|candidate| self.opposite_trace_edge(edge, *candidate, pair_label))
                .ok_or(MeshAdaptationError::InvalidSource(
                    "paired constrained edge is missing",
                ))?;
            let pair = self.builder.boundary_edges[pair_index];
            let pair_parameter = 0.5 * (pair.parameters[0] + pair.parameters[1]);
            self.split_boundary_face(index, point, parameter)?;
            self.split_boundary_face(pair_index, point, pair_parameter)?;
            let left = self.builder.vertices.len() - 2;
            let right = self.builder.vertices.len() - 1;
            if self.builder.point(left) != self.builder.point(right) {
                return Err(MeshAdaptationError::InvalidSource(
                    "paired constrained split lost coincidence",
                ));
            }
            self.report.inserted_vertices += 2;
        } else {
            self.split_boundary_face(index, point, parameter)?;
            self.report.inserted_vertices += 1;
        }
        Ok(())
    }

    fn opposite_trace_edge(
        &self,
        edge: BoundaryEdge,
        candidate: BoundaryEdge,
        label: BoundaryLabel,
    ) -> bool {
        candidate.label == label
            && if matches!(self.input, AdaptationInput::Topology(_)) {
                parameter_close(candidate.parameters[0], edge.parameters[1])
                    && parameter_close(candidate.parameters[1], edge.parameters[0])
            } else {
                self.builder.point(candidate.vertices[0]) == self.builder.point(edge.vertices[1])
                    && self.builder.point(candidate.vertices[1])
                        == self.builder.point(edge.vertices[0])
            }
    }

    fn split_boundary_face(
        &mut self,
        index: usize,
        point: Point2,
        parameter: f64,
    ) -> Result<(), MeshAdaptationError> {
        let edge = self.builder.boundary_edges[index];
        let vertex = self.builder.add_vertex(
            point,
            Some(BoundaryPoint {
                label: edge.label,
                parameter,
            }),
        )?;
        self.add_lineage(vertex);
        self.builder
            .boundary_keys
            .remove(&edge_key(edge.vertices[0], edge.vertices[1]));
        self.builder.boundary_edges[index] = BoundaryEdge {
            vertices: [edge.vertices[0], vertex],
            label: edge.label,
            parameters: [edge.parameters[0], parameter],
        };
        self.builder
            .boundary_keys
            .insert(edge_key(edge.vertices[0], vertex));
        self.builder.add_boundary_edge(BoundaryEdge {
            vertices: [vertex, edge.vertices[1]],
            label: edge.label,
            parameters: [parameter, edge.parameters[1]],
        });
        self.builder.split_edge(edge.vertices, vertex)?;
        Ok(())
    }

    fn boundary_point(
        &self,
        label: BoundaryLabel,
        parameter: f64,
    ) -> Result<Point2, MeshAdaptationError> {
        if let AdaptationInput::Topology(contract) = &self.input {
            let mut atoms = contract.atoms.iter().filter(|atom| {
                let [a, b] = atom.boundary.parameter;
                atom.label == label && parameter > a.min(b) && parameter < a.max(b)
            });
            let atom = atoms.next().filter(|_| atoms.next().is_none()).ok_or(
                MeshAdaptationError::InvalidSource(
                    "boundary split does not belong to one topology interval",
                ),
            )?;
            return topology_point_on_atom(atom.boundary, parameter).ok_or(
                MeshAdaptationError::InvalidSource("invalid topology interval geometry"),
            );
        }
        let AdaptationInput::Scene(scene) = &self.input else {
            unreachable!()
        };
        match label {
            BoundaryLabel::Outer(side) => {
                let corners = scene.domain.corners();
                let [a, b] = match side {
                    OuterSide::Bottom => [corners[0], corners[1]],
                    OuterSide::Right => [corners[1], corners[2]],
                    OuterSide::Top => [corners[2], corners[3]],
                    OuterSide::Left => [corners[3], corners[0]],
                };
                Ok(a.lerp(b, parameter))
            }
            BoundaryLabel::Obstacle(id)
            | BoundaryLabel::MaterialInterface(id)
            | BoundaryLabel::Wall { loop_id: id, .. } => scene
                .obstacles
                .iter()
                .find(|obstacle| obstacle.id == id)
                .map(|obstacle| obstacle.spline.evaluate(parameter))
                .ok_or(MeshAdaptationError::InvalidSource("unknown loop boundary")),
            BoundaryLabel::InternalBoundary { id, .. } => scene
                .internal_boundaries
                .iter()
                .find(|boundary| boundary.id == id)
                .map(|boundary| boundary.spline.evaluate(parameter))
                .ok_or(MeshAdaptationError::InvalidSource("unknown open boundary")),
            BoundaryLabel::OpenMaterialInterface(id) => scene
                .material_interfaces
                .iter()
                .find(|interface| interface.id == id)
                .and_then(|interface| match &interface.spline {
                    crate::InterfaceSpline::Open(spline) => Some(spline.evaluate(parameter)),
                    crate::InterfaceSpline::Closed(_) => None,
                })
                .ok_or(MeshAdaptationError::InvalidSource(
                    "unknown material interface",
                )),
            BoundaryLabel::Curve { .. } => Err(MeshAdaptationError::InvalidSource(
                "unified topology curve geometry is unavailable to legacy AMR",
            )),
        }
    }

    fn add_lineage(&mut self, vertex: usize) {
        debug_assert_eq!(vertex, self.state.vertex_lineage.len());
        self.state.vertex_lineage.push(self.state.next_lineage);
        self.state.last_modified.push(self.next_generation);
        self.state.next_lineage += 1;
    }

    fn count_oversized(&mut self) -> Result<usize, MeshAdaptationError> {
        let mut count = 0;
        for index in 0..self.builder.triangles.len() {
            let triangle = self.builder.triangles[index];
            let target = self.triangle_target(triangle)?;
            if self.builder.triangle_quality(triangle).maximum_edge_length
                > self.options.refine_ratio * target
            {
                count += 1;
            }
        }
        Ok(count)
    }

    fn verify_triangle(
        &mut self,
        index: usize,
        mut quality: MeshQuality,
    ) -> Result<(), MeshAdaptationError> {
        if index == self.builder.triangles.len() {
            self.phase = AdaptationPhase::VerifyBoundary(0, quality);
            return Ok(());
        }
        let triangle = self.builder.triangles[index];
        if let AdaptationInput::Topology(contract) = &mut self.input {
            if !contract.active_regions.contains(&triangle.region) {
                return Err(MeshAdaptationError::InvalidSource(
                    "adaptation produced an inactive topology region",
                ));
            }
            contract.seen_regions.insert(triangle.region);
        }
        let points = self.builder.triangle_points(triangle);
        if orient2d(points[0], points[1], points[2]) != PredicateSign::Positive {
            return Err(MeshAdaptationError::InvalidSource(
                "adaptation produced an inverted triangle",
            ));
        }
        for opposite in 0..3 {
            let edge = edge_key(
                triangle.vertices[(opposite + 1) % 3],
                triangle.vertices[(opposite + 2) % 3],
            );
            let sides = self
                .builder
                .adjacency
                .get(&edge)
                .ok_or(MeshAdaptationError::InvalidSource("missing adjacency"))?;
            let expected = self
                .builder
                .boundary_edges
                .iter()
                .find(|boundary| edge_key(boundary.vertices[0], boundary.vertices[1]) == edge)
                .map_or(2, |boundary| boundary_adjacency(boundary.label));
            if sides.len() != expected || !sides.contains(&(index, triangle.vertices[opposite])) {
                return Err(MeshAdaptationError::InvalidSource(
                    "adaptation produced non-manifold adjacency",
                ));
            }
        }
        let current = self.builder.triangle_quality(triangle);
        quality.minimum_angle_degrees = quality
            .minimum_angle_degrees
            .min(current.minimum_angle_degrees);
        quality.maximum_edge_length = quality.maximum_edge_length.max(current.maximum_edge_length);
        let mut lineage = triangle
            .vertices
            .map(|vertex| self.state.vertex_lineage[vertex]);
        lineage.sort_unstable();
        if self.source_triangle_keys.contains(&lineage)
            && triangle.vertices.iter().all(|vertex| {
                self.source_points_by_lineage
                    .get(&self.state.vertex_lineage[*vertex])
                    .is_some_and(|point| *point == self.builder.point(*vertex))
            })
        {
            self.report.preserved_triangles += 1;
        }
        self.phase = AdaptationPhase::VerifyTriangles(index + 1, quality);
        Ok(())
    }

    fn verify_boundary(
        &mut self,
        index: usize,
        quality: MeshQuality,
    ) -> Result<(), MeshAdaptationError> {
        if index == self.builder.boundary_edges.len() {
            if matches!(self.input, AdaptationInput::Topology(_)) {
                self.phase = AdaptationPhase::VerifyTopologyCoverage(0, quality);
            } else {
                self.begin_publish(quality);
            }
            return Ok(());
        }
        let boundary = self.builder.boundary_edges[index];
        let expected = boundary_adjacency(boundary.label);
        if self
            .builder
            .adjacency
            .get(&edge_key(boundary.vertices[0], boundary.vertices[1]))
            .is_none_or(|sides| sides.len() != expected)
        {
            return Err(MeshAdaptationError::InvalidSource(
                "adaptation broke a constrained edge",
            ));
        }
        if let AdaptationInput::Topology(contract) = &mut self.input {
            record_topology_boundary(&self.builder, contract, boundary)?;
        }
        self.phase = AdaptationPhase::VerifyBoundary(index + 1, quality);
        Ok(())
    }

    fn begin_publish(&mut self, quality: MeshQuality) {
        self.output = Some(TriMesh {
            geometry_revision: self.source.geometry_revision,
            mesh_revision: self.target_mesh_revision,
            vertices: vec![],
            triangles: self.builder.triangles.clone(),
            boundary_edges: self.builder.boundary_edges.clone(),
            quality,
        });
        self.remap = Vec::with_capacity(self.builder.vertices.len());
        self.phase = AdaptationPhase::CompactVertices(0);
    }

    fn compact_vertex(&mut self, index: usize) {
        if index == self.builder.vertices.len() {
            self.phase = AdaptationPhase::CompactTriangles(0);
            return;
        }
        if self.builder.incident[index].is_empty() {
            self.remap.push(usize::MAX);
        } else {
            self.remap.push(self.compact_vertices.len());
            self.compact_vertices.push(self.builder.vertices[index]);
            self.compact_lineage.push(self.state.vertex_lineage[index]);
            self.compact_modified.push(self.state.last_modified[index]);
        }
        self.phase = AdaptationPhase::CompactVertices(index + 1);
    }

    fn compact_triangle(&mut self, index: usize) {
        let output = self.output.as_mut().unwrap();
        if index == output.triangles.len() {
            self.phase = AdaptationPhase::CompactBoundary(0);
            return;
        }
        output.triangles[index].vertices = output.triangles[index]
            .vertices
            .map(|vertex| self.remap[vertex]);
        self.phase = AdaptationPhase::CompactTriangles(index + 1);
    }

    fn compact_boundary(
        &mut self,
        index: usize,
    ) -> Result<Option<MeshAdaptationResult>, MeshAdaptationError> {
        let output = self.output.as_mut().unwrap();
        if index < output.boundary_edges.len() {
            output.boundary_edges[index].vertices = output.boundary_edges[index]
                .vertices
                .map(|vertex| self.remap[vertex]);
            self.phase = AdaptationPhase::CompactBoundary(index + 1);
            return Ok(None);
        }
        output.vertices = std::mem::take(&mut self.compact_vertices);
        self.state.mesh_revision = self.target_mesh_revision;
        self.state.generation = self.next_generation;
        self.state.vertex_lineage = std::mem::take(&mut self.compact_lineage);
        self.state.last_modified = std::mem::take(&mut self.compact_modified);
        Ok(Some(MeshAdaptationResult {
            mesh: self.output.take().unwrap(),
            state: self.state.clone(),
            report: self.report.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CurveId, CurveNode, CurveSpan, CurveSpanId, CurveSpline, DEFAULT_MATERIAL,
        FaceBoundaryCondition, FaceRegionAssignment, InternalBoundary, InternalBoundaryLaw,
        Material, MaterialId, Obstacle, OpenCubicSpline, PeriodicCubicSpline, QuadraticTransferMap,
        QuadraticWaveOperator, Region, SpanBehavior, TopologyCurve, TopologyGeometry,
        TopologyVertex, TopologyVertexId, TopologyVertexLocation, TopologyWaveModel,
        compile_topology, mesh_topology_plan,
    };

    fn options(maximum: f64, minimum: f64) -> MeshAdaptationOptions {
        MeshAdaptationOptions {
            meshing: MeshingOptions {
                curve_tolerance: 8.0e-4,
                target_edge_length: maximum / 1.05,
                minimum_angle_degrees: 10.0,
                max_vertices: 20_000,
                max_triangles: 40_000,
                max_refinement_steps: 20_000,
            },
            minimum_target_edge_length: minimum,
            maximum_target_edge_length: maximum,
            max_topology_changes: 1_000,
            ..Default::default()
        }
    }

    fn run(mut job: MeshAdaptationJob, slice: usize) -> MeshAdaptationResult {
        loop {
            if let Some(result) = job.advance(slice) {
                return result.unwrap();
            }
        }
    }

    fn radial(center: Point2, fine: f64, coarse: f64) -> Arc<dyn MeshSizeField> {
        Arc::new(move |point: Point2, _region: RegionId| {
            let distance = (point - center).norm();
            if distance <= 0.24 {
                fine
            } else if distance >= 0.42 {
                coarse
            } else {
                let x = (distance - 0.24) / 0.18;
                let smooth = x * x * (3.0 - 2.0 * x);
                fine + (coarse - fine) * smooth
            }
        })
    }

    fn two_region_scene(role: LoopRole) -> Scene {
        Scene {
            obstacles: vec![Obstacle::with_role(
                ObstacleId(20),
                PeriodicCubicSpline::rounded(Point2::default(), 0.42),
                role,
            )],
            materials: vec![
                Material::default_medium(),
                Material {
                    id: MaterialId(2),
                    name: "Inclusion".into(),
                    mass_density: crate::ScalarField::constant(1.0),
                    stiffness: crate::ScalarField::constant(2.0),
                    damping: crate::ScalarField::constant(0.0),
                    axis_ratio: crate::ScalarField::constant(1.0),
                    parameters: vec![],
                    color: [180, 90, 70],
                },
            ],
            regions: vec![
                Region {
                    id: BACKGROUND_REGION,
                    material: DEFAULT_MATERIAL,
                    frame: crate::MaterialFrame::world(),
                },
                Region {
                    id: RegionId(2),
                    material: MaterialId(2),
                    frame: crate::MaterialFrame::world(),
                },
            ],
            ..Scene::default()
        }
    }

    fn topology_setup(
        geometry: TopologyGeometry,
        revision: u64,
        maximum: f64,
        minimum: f64,
    ) -> (TopologyMeshPlan, Arc<TriMesh>, MeshAdaptationOptions) {
        let topology = compile_topology(&geometry, revision).unwrap();
        let assignments = topology
            .faces
            .iter()
            .enumerate()
            .map(|(index, face)| FaceRegionAssignment {
                face: face.id,
                region: Some(RegionId(index as u64 + 1)),
            })
            .collect::<Vec<_>>();
        let plan = TopologyMeshPlan::new(&topology, &assignments).unwrap();
        let configuration = options(maximum, minimum);
        let mut initial_meshing = configuration.meshing;
        initial_meshing.target_edge_length = maximum * 1.25;
        let mesh = Arc::new(mesh_topology_plan(&plan, revision + 100, initial_meshing).unwrap());
        (plan, mesh, configuration)
    }

    fn separated_baffle() -> TopologyGeometry {
        let curve = TopologyCurve::new(
            CurveId(1),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![Point2::new(-0.55, -0.06), Point2::new(0.55, 0.08)])
                    .unwrap(),
            ),
            vec![CurveSpan {
                id: CurveSpanId(1),
                behavior: SpanBehavior::Separated {
                    left: FaceBoundaryCondition::Reflecting,
                    right: FaceBoundaryCondition::Reflecting,
                    coupling: crate::InternalBoundaryCoupling::Independent,
                },
            }],
        )
        .unwrap();
        TopologyGeometry {
            curves: vec![curve],
            ..TopologyGeometry::default()
        }
    }

    fn transmitting_divider() -> TopologyGeometry {
        let endpoints = [TopologyVertexId(1), TopologyVertexId(2)];
        let mut curve = TopologyCurve::new(
            CurveId(2),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![Point2::new(0.0, -1.0), Point2::new(0.0, 1.0)])
                    .unwrap(),
            ),
            vec![CurveSpan {
                id: CurveSpanId(2),
                behavior: SpanBehavior::Transmitting,
            }],
        )
        .unwrap();
        curve.nodes = endpoints
            .map(|vertex| CurveNode {
                vertex: Some(vertex),
            })
            .to_vec();
        TopologyGeometry {
            curves: vec![curve],
            vertices: vec![
                TopologyVertex {
                    id: endpoints[0],
                    location: TopologyVertexLocation::Outer {
                        side: OuterSide::Bottom,
                        fraction: 0.5,
                    },
                },
                TopologyVertex {
                    id: endpoints[1],
                    location: TopologyVertexLocation::Outer {
                        side: OuterSide::Top,
                        fraction: 0.5,
                    },
                },
            ],
            ..TopologyGeometry::default()
        }
    }

    fn separated_t_junction() -> TopologyGeometry {
        let center = TopologyVertexId(10);
        let behavior = SpanBehavior::REFLECTING;
        let mut horizontal = TopologyCurve::new(
            CurveId(10),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![
                    Point2::new(-0.7, 0.0),
                    Point2::new(0.0, 0.0),
                    Point2::new(0.7, 0.0),
                ])
                .unwrap(),
            ),
            vec![
                CurveSpan {
                    id: CurveSpanId(100),
                    behavior,
                },
                CurveSpan {
                    id: CurveSpanId(101),
                    behavior,
                },
            ],
        )
        .unwrap();
        horizontal.nodes[1].vertex = Some(center);
        let mut branch = TopologyCurve::new(
            CurveId(11),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![Point2::new(0.0, 0.0), Point2::new(0.0, 0.7)])
                    .unwrap(),
            ),
            vec![CurveSpan {
                id: CurveSpanId(102),
                behavior,
            }],
        )
        .unwrap();
        branch.nodes[0].vertex = Some(center);
        TopologyGeometry {
            curves: vec![horizontal, branch],
            vertices: vec![TopologyVertex {
                id: center,
                location: TopologyVertexLocation::Interior(Point2::default()),
            }],
            ..TopologyGeometry::default()
        }
    }

    fn mixed_junction() -> TopologyGeometry {
        let [left, right, center] = [
            TopologyVertexId(20),
            TopologyVertexId(21),
            TopologyVertexId(22),
        ];
        let mut divider = TopologyCurve::new(
            CurveId(20),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![
                    Point2::new(-1.0, 0.0),
                    Point2::new(0.0, 0.0),
                    Point2::new(1.0, 0.0),
                ])
                .unwrap(),
            ),
            vec![
                CurveSpan {
                    id: CurveSpanId(200),
                    behavior: SpanBehavior::Transmitting,
                },
                CurveSpan {
                    id: CurveSpanId(201),
                    behavior: SpanBehavior::Transmitting,
                },
            ],
        )
        .unwrap();
        divider.nodes[0].vertex = Some(left);
        divider.nodes[1].vertex = Some(center);
        divider.nodes[2].vertex = Some(right);
        let mut branch = TopologyCurve::new(
            CurveId(21),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![Point2::new(0.0, 0.0), Point2::new(0.0, 0.7)])
                    .unwrap(),
            ),
            vec![CurveSpan {
                id: CurveSpanId(202),
                behavior: SpanBehavior::REFLECTING,
            }],
        )
        .unwrap();
        branch.nodes[0].vertex = Some(center);
        TopologyGeometry {
            curves: vec![divider, branch],
            vertices: vec![
                TopologyVertex {
                    id: left,
                    location: TopologyVertexLocation::Outer {
                        side: OuterSide::Left,
                        fraction: 0.5,
                    },
                },
                TopologyVertex {
                    id: right,
                    location: TopologyVertexLocation::Outer {
                        side: OuterSide::Right,
                        fraction: 0.5,
                    },
                },
                TopologyVertex {
                    id: center,
                    location: TopologyVertexLocation::Interior(Point2::default()),
                },
            ],
            ..TopologyGeometry::default()
        }
    }

    #[test]
    fn uniform_field_is_deterministic_and_mesh_revision_is_distinct() {
        let scene = Scene::initial();
        let configuration = options(0.24, 0.08);
        let source = Arc::new(
            mesh_scene(&scene, 7, configuration.meshing)
                .expect("initial mesh should be constructible"),
        );
        let mut no_collapse = configuration;
        no_collapse.collapse_ratio = 1.0e-6;
        let field: Arc<dyn MeshSizeField> = Arc::new(|_, _| 0.24);
        let one = run(
            MeshAdaptationJob::new(
                source.clone(),
                scene.clone(),
                MeshAdaptationState::from_mesh(&source),
                21,
                field.clone(),
                no_collapse,
            ),
            1,
        );
        let many = run(
            MeshAdaptationJob::new(
                source.clone(),
                scene,
                MeshAdaptationState::from_mesh(&source),
                21,
                field,
                no_collapse,
            ),
            10_000,
        );
        assert_eq!(one.mesh, many.mesh);
        assert_eq!(one.state, many.state);
        assert_eq!(one.mesh.geometry_revision, source.geometry_revision);
        assert_eq!(one.mesh.mesh_revision, 21);
        assert_eq!(one.report.topology_changes, 0);
        assert!(one.report.converged);
    }

    #[test]
    fn topology_rectangle_adapts_deterministically_and_preserves_trace_vertices() {
        let (plan, source, mut configuration) =
            topology_setup(TopologyGeometry::default(), 201, 0.28, 0.08);
        configuration.collapse_ratio = 1.0e-6;
        let field: Arc<dyn MeshSizeField> =
            Arc::new(|point: Point2, _| if point.x < -0.45 { 0.08 } else { 0.28 });
        let one = run(
            MeshAdaptationJob::new_topology(
                source.clone(),
                &plan,
                MeshAdaptationState::from_mesh(&source),
                302,
                field.clone(),
                configuration,
            ),
            1,
        );
        let many = run(
            MeshAdaptationJob::new_topology(
                source.clone(),
                &plan,
                MeshAdaptationState::from_mesh(&source),
                302,
                field,
                configuration,
            ),
            10_000,
        );
        assert_eq!(one.mesh, many.mesh);
        assert_eq!(one.state, many.state);
        assert!(one.report.inserted_vertices > 0, "{:?}", one.report);
        let traces = one
            .mesh
            .vertices
            .iter()
            .filter_map(|vertex| vertex.trace.map(|trace| (trace, vertex.point)))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            traces,
            plan.vertices
                .iter()
                .map(|vertex| (vertex.id, vertex.point))
                .collect()
        );
    }

    #[test]
    fn topology_separated_baffle_refines_and_coarsens_both_traces_atomically() {
        let (plan, source, mut configuration) = topology_setup(separated_baffle(), 211, 0.28, 0.07);
        configuration.max_topology_changes = 4_000;
        configuration.max_work_units = 20_000_000;
        let fine: Arc<dyn MeshSizeField> = Arc::new(
            |point: Point2, _| {
                if point.y.abs() < 0.18 { 0.07 } else { 0.28 }
            },
        );
        let refined = run(
            MeshAdaptationJob::new_topology(
                source.clone(),
                &plan,
                MeshAdaptationState::from_mesh(&source),
                312,
                fine,
                configuration,
            ),
            37,
        );
        assert!(
            refined.report.boundary_insertions > 0,
            "{:?}",
            refined.report
        );
        let refined_segments = refined
            .mesh
            .boundary_edges
            .iter()
            .filter(|edge| {
                matches!(
                    edge.label,
                    BoundaryLabel::Curve {
                        separated: true,
                        ..
                    }
                )
            })
            .count();
        let refined_mesh = Arc::new(refined.mesh);
        let coarse = run(
            MeshAdaptationJob::new_topology(
                refined_mesh.clone(),
                &plan,
                refined.state,
                313,
                Arc::new(|_, _| 0.28),
                configuration,
            ),
            29,
        );
        assert!(coarse.report.boundary_collapses > 0, "{:?}", coarse.report);
        let coarse_segments = coarse
            .mesh
            .boundary_edges
            .iter()
            .filter(|edge| {
                matches!(
                    edge.label,
                    BoundaryLabel::Curve {
                        separated: true,
                        ..
                    }
                )
            })
            .count();
        assert!(coarse_segments < refined_segments);
        let mut pairs = BTreeMap::<(u64, u64), [bool; 2]>::new();
        for edge in &coarse.mesh.boundary_edges {
            let BoundaryLabel::Curve {
                side,
                separated: true,
                ..
            } = edge.label
            else {
                continue;
            };
            let key = (
                edge.parameters[0].min(edge.parameters[1]).to_bits(),
                edge.parameters[0].max(edge.parameters[1]).to_bits(),
            );
            pairs.entry(key).or_default()[match side {
                CurveTraceSide::Left => 0,
                CurveTraceSide::Right => 1,
            }] = true;
        }
        assert!(pairs.values().all(|sides| *sides == [true, true]));

        let scene = Scene::default();
        let source_operator = QuadraticWaveOperator::assemble_topology(
            &source,
            &plan,
            TopologyWaveModel::from_scene(&scene),
        )
        .unwrap();
        let target_operator = QuadraticWaveOperator::assemble_topology(
            &coarse.mesh,
            &plan,
            TopologyWaveModel::from_scene(&scene),
        )
        .unwrap();
        let transfer =
            QuadraticTransferMap::build(&source, &source_operator, &coarse.mesh, &target_operator)
                .unwrap();
        assert_eq!(transfer.exposed_nodes(), 0);
        let polynomial = |point: Point2| 0.3 + point.x - 0.4 * point.y + point.x * point.y;
        let values = source_operator
            .node_points()
            .iter()
            .map(|point| polynomial(*point))
            .collect::<Vec<_>>();
        for (actual, point) in transfer
            .interpolate(&values, 0.0)
            .unwrap()
            .iter()
            .zip(target_operator.node_points())
        {
            assert!((actual - polynomial(*point)).abs() < 2.0e-10);
        }
    }

    #[test]
    fn topology_transmitting_divider_remains_shared_between_regions() {
        let (plan, source, mut configuration) =
            topology_setup(transmitting_divider(), 221, 0.28, 0.07);
        configuration.max_topology_changes = 4_000;
        configuration.max_work_units = 20_000_000;
        let refined = run(
            MeshAdaptationJob::new_topology(
                source.clone(),
                &plan,
                MeshAdaptationState::from_mesh(&source),
                322,
                Arc::new(|point: Point2, region: RegionId| {
                    if region == RegionId(1) && point.x.abs() < 0.3 {
                        0.07
                    } else {
                        0.28
                    }
                }),
                configuration,
            ),
            31,
        );
        assert!(
            refined.report.boundary_insertions > 0,
            "{:?}",
            refined.report
        );
        for edge in refined.mesh.boundary_edges.iter().filter(|edge| {
            matches!(
                edge.label,
                BoundaryLabel::Curve {
                    separated: false,
                    ..
                }
            )
        }) {
            let adjacent = refined
                .mesh
                .triangles
                .iter()
                .filter(|triangle| {
                    edge.vertices
                        .iter()
                        .all(|vertex| triangle.vertices.contains(vertex))
                })
                .map(|triangle| triangle.region)
                .collect::<BTreeSet<_>>();
            assert_eq!(adjacent, BTreeSet::from([RegionId(1), RegionId(2)]));
        }
    }

    #[test]
    fn topology_preflight_rejects_a_lost_trace_without_mutating_source() {
        let (plan, source, configuration) =
            topology_setup(TopologyGeometry::default(), 231, 0.28, 0.08);
        let mut malformed = source.as_ref().clone();
        malformed
            .vertices
            .iter_mut()
            .find(|vertex| vertex.trace.is_some())
            .unwrap()
            .trace = None;
        let malformed = Arc::new(malformed);
        let before = malformed.as_ref().clone();
        let mut job = MeshAdaptationJob::new_topology(
            malformed.clone(),
            &plan,
            MeshAdaptationState::from_mesh(&malformed),
            332,
            Arc::new(|_, _| 0.28),
            configuration,
        );
        let error = loop {
            if let Some(result) = job.advance(17) {
                break result.unwrap_err();
            }
        };
        assert!(matches!(error, MeshAdaptationError::InvalidSource(_)));
        assert_eq!(malformed.as_ref(), &before);
    }

    #[test]
    fn topology_hole_boundary_adapts_without_requiring_an_excluded_partner() {
        let curve = TopologyCurve::new(
            CurveId(3),
            CurveSpline::Closed(
                PeriodicCubicSpline::polygon(vec![
                    Point2::new(-0.45, -0.45),
                    Point2::new(0.45, -0.45),
                    Point2::new(0.45, 0.45),
                    Point2::new(-0.45, 0.45),
                ])
                .unwrap(),
            ),
            (0..4)
                .map(|index| CurveSpan {
                    id: CurveSpanId(30 + index),
                    behavior: SpanBehavior::REFLECTING,
                })
                .collect(),
        )
        .unwrap();
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![curve],
                ..TopologyGeometry::default()
            },
            241,
        )
        .unwrap();
        let hole = topology.face_at(Point2::default()).unwrap();
        let assignments = topology
            .faces
            .iter()
            .map(|face| FaceRegionAssignment {
                face: face.id,
                region: (face.id != hole).then_some(RegionId(1)),
            })
            .collect::<Vec<_>>();
        let plan = TopologyMeshPlan::new(&topology, &assignments).unwrap();
        let mut configuration = options(0.28, 0.07);
        configuration.max_topology_changes = 4_000;
        configuration.max_work_units = 20_000_000;
        let source = Arc::new(mesh_topology_plan(&plan, 341, configuration.meshing).unwrap());
        let refined = run(
            MeshAdaptationJob::new_topology(
                source.clone(),
                &plan,
                MeshAdaptationState::from_mesh(&source),
                342,
                Arc::new(|_, _| 0.07),
                configuration,
            ),
            43,
        );
        assert!(
            refined.report.boundary_insertions > 0,
            "{:?}",
            refined.report
        );
        assert!(refined.mesh.boundary_edges.iter().any(|edge| {
            matches!(
                edge.label,
                BoundaryLabel::Curve {
                    separated: true,
                    ..
                }
            )
        }));
        let refined_mesh = Arc::new(refined.mesh);
        let coarse = run(
            MeshAdaptationJob::new_topology(
                refined_mesh.clone(),
                &plan,
                refined.state,
                343,
                Arc::new(|_, _| 0.28),
                configuration,
            ),
            37,
        );
        assert!(coarse.report.boundary_collapses > 0, "{:?}", coarse.report);
    }

    #[test]
    fn topology_separated_t_junction_keeps_three_pinned_sectors() {
        let (plan, source, mut configuration) =
            topology_setup(separated_t_junction(), 251, 0.28, 0.07);
        configuration.max_topology_changes = 4_000;
        configuration.max_work_units = 20_000_000;
        let expected = plan
            .vertices
            .iter()
            .filter(|vertex| vertex.point == Point2::default())
            .map(|vertex| vertex.id)
            .collect::<BTreeSet<_>>();
        assert_eq!(expected.len(), 3);
        let result = run(
            MeshAdaptationJob::new_topology(
                source.clone(),
                &plan,
                MeshAdaptationState::from_mesh(&source),
                352,
                Arc::new(
                    |point: Point2, _| {
                        if point.norm() < 0.45 { 0.07 } else { 0.28 }
                    },
                ),
                configuration,
            ),
            41,
        );
        let actual = result
            .mesh
            .vertices
            .iter()
            .filter(|vertex| vertex.point == Point2::default())
            .filter_map(|vertex| vertex.trace)
            .collect::<BTreeSet<_>>();
        assert_eq!(actual, expected);
        assert!(result.report.boundary_insertions > 0, "{:?}", result.report);
    }

    #[test]
    fn topology_mixed_junction_preserves_its_conforming_center_trace() {
        let (plan, source, mut configuration) = topology_setup(mixed_junction(), 261, 0.28, 0.07);
        configuration.max_topology_changes = 4_000;
        configuration.max_work_units = 20_000_000;
        let expected = plan
            .vertices
            .iter()
            .filter(|vertex| vertex.point == Point2::default())
            .map(|vertex| vertex.id)
            .collect::<BTreeSet<_>>();
        assert_eq!(expected.len(), 1);
        let result = run(
            MeshAdaptationJob::new_topology(
                source.clone(),
                &plan,
                MeshAdaptationState::from_mesh(&source),
                362,
                Arc::new(
                    |point: Point2, _| {
                        if point.norm() < 0.45 { 0.07 } else { 0.28 }
                    },
                ),
                configuration,
            ),
            41,
        );
        let actual = result
            .mesh
            .vertices
            .iter()
            .filter(|vertex| vertex.point == Point2::default())
            .filter_map(|vertex| vertex.trace)
            .collect::<BTreeSet<_>>();
        assert_eq!(actual, expected);
        assert!(result.report.boundary_insertions > 0, "{:?}", result.report);
    }

    #[test]
    fn moved_size_field_refines_then_coarsens_with_persistent_lineage() {
        let scene = Scene::initial();
        let configuration = options(0.24, 0.08);
        let source = Arc::new(mesh_scene(&scene, 3, configuration.meshing).unwrap());
        let first = run(
            MeshAdaptationJob::new(
                source.clone(),
                scene.clone(),
                MeshAdaptationState::from_mesh(&source),
                31,
                radial(Point2::new(-0.55, 0.45), 0.08, 0.24),
                configuration,
            ),
            37,
        );
        assert!(first.report.inserted_vertices > 0, "{:?}", first.report);
        let first_lineage = first
            .state
            .vertex_lineage
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let first_mesh = Arc::new(first.mesh);
        let second = run(
            MeshAdaptationJob::new(
                first_mesh.clone(),
                scene,
                first.state,
                32,
                radial(Point2::new(0.55, -0.45), 0.08, 0.24),
                configuration,
            ),
            19,
        );
        assert!(second.report.inserted_vertices > 0, "{:?}", second.report);
        assert!(second.report.collapsed_vertices > 0, "{:?}", second.report);
        assert!(
            second
                .state
                .vertex_lineage
                .iter()
                .any(|lineage| first_lineage.contains(lineage))
        );
        assert_eq!(second.mesh.geometry_revision, 3);
        assert_eq!(second.mesh.mesh_revision, 32);
        assert!(second.mesh.triangles.iter().all(|triangle| {
            let points = triangle
                .vertices
                .map(|vertex| second.mesh.vertices[vertex].point);
            orient2d(points[0], points[1], points[2]) == PredicateSign::Positive
        }));
    }

    #[test]
    fn coarsening_quota_preserves_capacity_for_refinement() {
        let scene = Scene::initial();
        let mut configuration = options(0.24, 0.08);
        configuration.max_coarsening_changes = 0;
        let source = Arc::new(mesh_scene(&scene, 33, configuration.meshing).unwrap());
        let result = run(
            MeshAdaptationJob::new(
                source.clone(),
                scene,
                MeshAdaptationState::from_mesh(&source),
                34,
                radial(Point2::new(-0.55, 0.45), 0.08, 0.24),
                configuration,
            ),
            37,
        );
        assert_eq!(result.report.coarsening_changes, 0);
        assert_eq!(result.report.collapsed_vertices, 0);
        assert!(result.report.inserted_vertices > 0, "{:?}", result.report);
    }

    #[test]
    fn invalid_field_and_stale_state_are_rejected_without_touching_source() {
        let scene = Scene::initial();
        let configuration = options(0.24, 0.08);
        let source = Arc::new(mesh_scene(&scene, 4, configuration.meshing).unwrap());
        let before = source.as_ref().clone();
        let mut stale = MeshAdaptationState::from_mesh(&source);
        stale.mesh_revision += 1;
        let mut job = MeshAdaptationJob::new(
            source.clone(),
            scene.clone(),
            stale,
            40,
            Arc::new(|_, _| 0.2),
            configuration,
        );
        assert_eq!(
            job.advance(1).unwrap().unwrap_err(),
            MeshAdaptationError::InvalidState
        );
        let mut job = MeshAdaptationJob::new(
            source.clone(),
            scene.clone(),
            MeshAdaptationState::from_mesh(&source),
            source.mesh_revision,
            Arc::new(|_, _| 0.2),
            configuration,
        );
        assert_eq!(
            job.advance(1).unwrap().unwrap_err(),
            MeshAdaptationError::InvalidMeshRevision
        );
        let mut job = MeshAdaptationJob::new(
            source.clone(),
            scene,
            MeshAdaptationState::from_mesh(&source),
            41,
            Arc::new(|_, _| f64::NAN),
            configuration,
        );
        loop {
            if let Some(result) = job.advance(100) {
                assert!(matches!(
                    result,
                    Err(MeshAdaptationError::InvalidTarget { .. })
                ));
                break;
            }
        }
        assert_eq!(source.as_ref(), &before);
    }

    #[test]
    fn capacity_limit_commits_a_valid_partial_mesh_and_work_limit_aborts() {
        let scene = Scene::initial();
        let mut configuration = options(0.24, 0.05);
        let source = Arc::new(mesh_scene(&scene, 44, configuration.meshing).unwrap());
        configuration.meshing.max_vertices = source.vertices.len();
        configuration.meshing.max_triangles = source.triangles.len() + 4;
        let limited = run(
            MeshAdaptationJob::new(
                source.clone(),
                scene.clone(),
                MeshAdaptationState::from_mesh(&source),
                45,
                Arc::new(|_, _| 0.05),
                configuration,
            ),
            13,
        );
        assert_eq!(limited.report.limit, Some(MeshAdaptationLimit::Capacity));
        assert!(!limited.report.converged);
        assert!(limited.report.remaining_oversized_triangles > 0);
        assert_eq!(limited.mesh.vertices, source.vertices);
        assert_eq!(limited.mesh.triangles, source.triangles);

        configuration.max_work_units = 1;
        let mut job = MeshAdaptationJob::new(
            source.clone(),
            scene,
            MeshAdaptationState::from_mesh(&source),
            46,
            Arc::new(|_, _| 0.05),
            configuration,
        );
        assert_eq!(
            job.advance(2).unwrap().unwrap_err(),
            MeshAdaptationError::WorkLimit
        );
    }

    #[test]
    fn paired_baffle_trace_refines_and_coarsens_atomically() {
        let mut scene = Scene::default();
        scene.internal_boundaries.push(InternalBoundary {
            id: InternalBoundaryId(9),
            spline: OpenCubicSpline::uniform(vec![
                Point2::new(-0.55, 0.0),
                Point2::new(-0.2, 0.0),
                Point2::new(0.2, 0.0),
                Point2::new(0.55, 0.0),
            ])
            .unwrap(),
            region: BACKGROUND_REGION,
            span_laws: vec![InternalBoundaryLaw::REFLECTING],
        });
        let configuration = options(0.28, 0.07);
        let source = Arc::new(mesh_scene(&scene, 8, configuration.meshing).unwrap());
        let fine: Arc<dyn MeshSizeField> = Arc::new(|point: Point2, _| {
            if point.y.abs() < 0.2 && point.x.abs() < 0.7 {
                0.07
            } else {
                0.28
            }
        });
        let refined = run(
            MeshAdaptationJob::new(
                source.clone(),
                scene.clone(),
                MeshAdaptationState::from_mesh(&source),
                81,
                fine,
                configuration,
            ),
            41,
        );
        assert!(
            refined.report.boundary_insertions > 0,
            "{:?}",
            refined.report
        );
        let refined_segments = refined
            .mesh
            .boundary_edges
            .iter()
            .filter(|edge| matches!(edge.label, BoundaryLabel::InternalBoundary { .. }))
            .count();
        let refined_mesh = Arc::new(refined.mesh);
        let coarse = run(
            MeshAdaptationJob::new(
                refined_mesh.clone(),
                scene,
                refined.state,
                82,
                Arc::new(|_, _| 0.28),
                configuration,
            ),
            17,
        );
        assert!(coarse.report.boundary_collapses > 0, "{:?}", coarse.report);
        let coarse_segments = coarse
            .mesh
            .boundary_edges
            .iter()
            .filter(|edge| matches!(edge.label, BoundaryLabel::InternalBoundary { .. }))
            .count();
        assert!(coarse_segments < refined_segments);
        let mut paired = BTreeMap::<(u64, u64), [Option<BoundaryEdge>; 2]>::new();
        for edge in &coarse.mesh.boundary_edges {
            let BoundaryLabel::InternalBoundary { side, .. } = edge.label else {
                continue;
            };
            let (key, face) = match side {
                InternalBoundarySide::Left => (
                    (edge.parameters[0].to_bits(), edge.parameters[1].to_bits()),
                    0,
                ),
                InternalBoundarySide::Right => (
                    (edge.parameters[1].to_bits(), edge.parameters[0].to_bits()),
                    1,
                ),
            };
            paired.entry(key).or_insert([None, None])[face] = Some(*edge);
        }
        assert!(!paired.is_empty());
        for [left, right] in paired.values() {
            let (left, right) = (left.unwrap(), right.unwrap());
            assert_eq!(
                coarse.mesh.vertices[left.vertices[0]].point,
                coarse.mesh.vertices[right.vertices[1]].point
            );
            assert_eq!(
                coarse.mesh.vertices[left.vertices[1]].point,
                coarse.mesh.vertices[right.vertices[0]].point
            );
        }
    }

    #[test]
    fn material_interface_and_closed_wall_keep_their_topology_through_adaptation() {
        for role in [
            LoopRole::MaterialInterface {
                exterior: BACKGROUND_REGION,
                interior: RegionId(2),
            },
            LoopRole::Wall {
                exterior: BACKGROUND_REGION,
                interior: RegionId(2),
            },
        ] {
            let scene = two_region_scene(role);
            let mut configuration = options(0.28, 0.02);
            configuration.meshing.curve_tolerance = 0.002;
            configuration.max_topology_changes = 15_000;
            configuration.max_work_units = 30_000_000;
            let source = Arc::new(mesh_scene(&scene, 91, configuration.meshing).unwrap());
            let focus = scene.obstacles[0].spline.evaluate(0.5);
            let fine: Arc<dyn MeshSizeField> = Arc::new(move |point: Point2, _| {
                if (point - focus).norm() < 0.08 {
                    0.02
                } else {
                    0.28
                }
            });
            let refined = run(
                MeshAdaptationJob::new(
                    source.clone(),
                    scene.clone(),
                    MeshAdaptationState::from_mesh(&source),
                    92,
                    fine,
                    configuration,
                ),
                31,
            );
            assert!(
                refined.report.boundary_insertions > 0,
                "{role:?}: {:?}",
                refined.report
            );
            let refined_edges = refined
                .mesh
                .boundary_edges
                .iter()
                .filter(|edge| {
                    matches!(
                        edge.label,
                        BoundaryLabel::MaterialInterface(ObstacleId(20))
                            | BoundaryLabel::Wall {
                                loop_id: ObstacleId(20),
                                ..
                            }
                    )
                })
                .count();
            let refined_mesh = Arc::new(refined.mesh);
            let coarse = run(
                MeshAdaptationJob::new(
                    refined_mesh.clone(),
                    scene,
                    refined.state,
                    93,
                    Arc::new(|_, _| 0.28),
                    configuration,
                ),
                29,
            );
            assert!(
                coarse.report.boundary_collapses > 0,
                "{role:?}: {:?}",
                coarse.report
            );
            let coarse_edges = coarse
                .mesh
                .boundary_edges
                .iter()
                .filter(|edge| {
                    matches!(
                        edge.label,
                        BoundaryLabel::MaterialInterface(ObstacleId(20))
                            | BoundaryLabel::Wall {
                                loop_id: ObstacleId(20),
                                ..
                            }
                    )
                })
                .count();
            assert!(coarse_edges < refined_edges, "{role:?}");
            let regions = coarse
                .mesh
                .triangles
                .iter()
                .map(|triangle| triangle.region)
                .collect::<BTreeSet<_>>();
            assert_eq!(regions, BTreeSet::from([BACKGROUND_REGION, RegionId(2)]));

            if matches!(role, LoopRole::Wall { .. }) {
                let exterior = coarse
                    .mesh
                    .boundary_edges
                    .iter()
                    .filter(|edge| {
                        matches!(
                            edge.label,
                            BoundaryLabel::Wall {
                                side: BoundarySide::Exterior,
                                ..
                            }
                        )
                    })
                    .collect::<Vec<_>>();
                let interior = coarse
                    .mesh
                    .boundary_edges
                    .iter()
                    .filter(|edge| {
                        matches!(
                            edge.label,
                            BoundaryLabel::Wall {
                                side: BoundarySide::Interior,
                                ..
                            }
                        )
                    })
                    .collect::<Vec<_>>();
                assert_eq!(exterior.len(), interior.len());
                assert!(exterior.iter().all(|outside| interior.iter().any(|inside| {
                    coarse.mesh.vertices[outside.vertices[0]].point
                        == coarse.mesh.vertices[inside.vertices[1]].point
                        && coarse.mesh.vertices[outside.vertices[1]].point
                            == coarse.mesh.vertices[inside.vertices[0]].point
                })));
            }
        }
    }

    #[test]
    fn adaptive_mesh_transfer_reproduces_quadratic_state_without_exposed_nodes() {
        let scene = Scene::initial();
        let configuration = options(0.24, 0.08);
        let source = Arc::new(mesh_scene(&scene, 12, configuration.meshing).unwrap());
        let source_operator = QuadraticWaveOperator::assemble_scene_with_boundaries(
            &source,
            &scene,
            scene.outer_boundaries,
        )
        .unwrap();
        let adapted = run(
            MeshAdaptationJob::new(
                source.clone(),
                scene.clone(),
                MeshAdaptationState::from_mesh(&source),
                121,
                radial(Point2::new(-0.5, 0.45), 0.08, 0.24),
                configuration,
            ),
            23,
        );
        let target_operator = QuadraticWaveOperator::assemble_scene_with_boundaries(
            &adapted.mesh,
            &scene,
            scene.outer_boundaries,
        )
        .unwrap();
        let map =
            QuadraticTransferMap::build(&source, &source_operator, &adapted.mesh, &target_operator)
                .unwrap();
        assert_eq!(map.exposed_nodes(), 0);
        let polynomial = |point: Point2| {
            0.7 + 0.2 * point.x - 0.3 * point.y + 0.4 * point.x * point.x - 0.25 * point.x * point.y
                + 0.1 * point.y * point.y
        };
        let source_values = source_operator
            .node_points()
            .iter()
            .map(|point| polynomial(*point))
            .collect::<Vec<_>>();
        let transferred = map.interpolate(&source_values, 0.0).unwrap();
        for (actual, point) in transferred.iter().zip(target_operator.node_points()) {
            assert!((actual - polynomial(*point)).abs() < 2.0e-11);
        }
    }
}
