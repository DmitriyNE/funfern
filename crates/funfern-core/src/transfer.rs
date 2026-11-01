use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::{
    BoundaryLabel, CurveId, CurveTraceSide, Point2, QuadraticWaveOperator, TraceVertexId, TriMesh,
    enriched_quadratic_basis,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum BoundaryTraceKey {
    Curve {
        curve: CurveId,
        side: CurveTraceSide,
    },
}

impl BoundaryTraceKey {
    fn from_label(label: BoundaryLabel) -> Option<Self> {
        match label {
            BoundaryLabel::Curve {
                curve,
                side,
                separated: true,
                ..
            } => Some(Self::Curve { curve, side }),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct TargetTracePreference {
    vertex: Option<TraceVertexId>,
    boundaries: BTreeSet<BoundaryTraceKey>,
}

#[derive(Clone, Debug, Default)]
struct SourceTraceTriangles {
    vertices: BTreeMap<TraceVertexId, BTreeSet<u32>>,
    boundaries: BTreeMap<BoundaryTraceKey, BTreeSet<u32>>,
}

#[derive(Clone, Copy)]
struct TraceRestrictions<'a> {
    target: &'a [TargetTracePreference],
    source: &'a SourceTraceTriangles,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransferSample {
    pub vertices: [u32; 3],
    pub weights: [f64; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransferMap {
    source_revision: u64,
    target_revision: u64,
    source_mesh_revision: u64,
    target_mesh_revision: u64,
    source_vertices: usize,
    samples: Vec<Option<TransferSample>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransferError {
    EmptySource,
    InvalidSource,
    InvalidTarget,
    SizeMismatch { expected: usize, actual: usize },
    NonFiniteValues,
}

impl std::fmt::Display for TransferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptySource => write!(f, "The source mesh has no triangles"),
            Self::InvalidSource => write!(f, "The source mesh is invalid for state transfer"),
            Self::InvalidTarget => write!(f, "The target mesh is invalid for state transfer"),
            Self::SizeMismatch { expected, actual } => {
                write!(f, "Expected {expected} source values, received {actual}")
            }
            Self::NonFiniteValues => write!(f, "The source field contains non-finite values"),
        }
    }
}

impl std::error::Error for TransferError {}

impl TransferMap {
    /// Locates every target vertex in the source triangulation. Vertices outside
    /// the old domain are deliberately left unmapped and initialize to zero.
    pub fn build(source: &TriMesh, target: &TriMesh) -> Result<Self, TransferError> {
        Self::build_restricted(
            source,
            target,
            &vec![None; target.vertices.len()],
            None,
            None,
        )
    }

    fn build_restricted(
        source: &TriMesh,
        target: &TriMesh,
        target_groups: &[Option<crate::RegionId>],
        region_groups: Option<&BTreeMap<crate::RegionId, crate::RegionId>>,
        trace_restrictions: Option<TraceRestrictions<'_>>,
    ) -> Result<Self, TransferError> {
        if source.triangles.is_empty() {
            return Err(TransferError::EmptySource);
        }
        validate_mesh(source, true)?;
        validate_mesh(target, false)?;
        if target_groups.len() != target.vertices.len() {
            return Err(TransferError::InvalidTarget);
        }
        if trace_restrictions.is_some_and(|traces| traces.target.len() != target.vertices.len()) {
            return Err(TransferError::InvalidTarget);
        }

        let mut bins = SourceBins::new(source)?;
        for index in 0..source.triangles.len() {
            bins.insert(source, index)?;
        }

        let mut samples = Vec::with_capacity(target.vertices.len());
        for (target_index, (vertex, target_group)) in
            target.vertices.iter().zip(target_groups).enumerate()
        {
            let preferred = trace_restrictions
                .map(|traces| {
                    preferred_trace_triangles(&traces.target[target_index], traces.source)
                })
                .unwrap_or_default();
            samples.push(
                bins.locate(
                    source,
                    vertex.point,
                    *target_group,
                    region_groups,
                    &preferred,
                )
                .map(|(triangle, weights)| TransferSample {
                    vertices: source.triangles[triangle as usize]
                        .vertices
                        .map(|i| i as u32),
                    weights,
                }),
            );
        }
        Ok(Self {
            source_revision: source.geometry_revision,
            target_revision: target.geometry_revision,
            source_mesh_revision: source.mesh_revision,
            target_mesh_revision: target.mesh_revision,
            source_vertices: source.vertices.len(),
            samples,
        })
    }

    pub fn source_revision(&self) -> u64 {
        self.source_revision
    }

    pub fn target_revision(&self) -> u64 {
        self.target_revision
    }

    pub fn source_vertices(&self) -> usize {
        self.source_vertices
    }

    pub fn samples(&self) -> &[Option<TransferSample>] {
        &self.samples
    }

    pub fn matches_meshes(&self, source: &TriMesh, target: &TriMesh) -> bool {
        self.source_revision == source.geometry_revision
            && self.target_revision == target.geometry_revision
            && self.source_mesh_revision == source.mesh_revision
            && self.target_mesh_revision == target.mesh_revision
            && self.source_vertices == source.vertices.len()
            && self.samples.len() == target.vertices.len()
    }

    pub fn exposed_vertices(&self) -> usize {
        self.samples
            .iter()
            .filter(|sample| sample.is_none())
            .count()
    }

    pub fn interpolate(
        &self,
        source_values: &[f64],
        exposed_value: f64,
    ) -> Result<Vec<f64>, TransferError> {
        if source_values.len() != self.source_vertices {
            return Err(TransferError::SizeMismatch {
                expected: self.source_vertices,
                actual: source_values.len(),
            });
        }
        if !exposed_value.is_finite() || source_values.iter().any(|value| !value.is_finite()) {
            return Err(TransferError::NonFiniteValues);
        }
        Ok(self
            .samples
            .iter()
            .map(|sample| match sample {
                Some(sample) => sample
                    .vertices
                    .iter()
                    .zip(sample.weights)
                    .map(|(&index, weight)| source_values[index as usize] * weight)
                    .sum(),
                None => exposed_value,
            })
            .collect())
    }
}

/// The source triangles a target node is to be found in, narrowest first:
/// `locate` tries each in turn before searching every triangle.
fn preferred_trace_triangles(
    target: &TargetTracePreference,
    source: &SourceTraceTriangles,
) -> Vec<BTreeSet<u32>> {
    let boundary_triangles = target
        .boundaries
        .iter()
        .filter_map(|boundary| source.boundaries.get(boundary))
        .flatten()
        .copied()
        .collect::<BTreeSet<_>>();
    if boundary_triangles.is_empty() {
        return Vec::new();
    }

    // Trace IDs identify a particular angular sector at a junction. They are
    // snapshot-local, so use them only to narrow a stable curve-side match. If
    // an edit changed trace numbering, the curve side remains the safe lineage,
    // tried next whenever the narrowed set does not hold the node: a set that
    // was not empty but missed it, renumbered onto another vertex on the same
    // side, went straight to the unrestricted search, where either of a wall's
    // coincident copies could answer (MESH-1 of the 2026-10-20 review).
    let mut tiers = Vec::new();
    if let Some(vertex_triangles) = target
        .vertex
        .and_then(|vertex| source.vertices.get(&vertex))
    {
        let intersection = boundary_triangles
            .intersection(vertex_triangles)
            .copied()
            .collect::<BTreeSet<_>>();
        if !intersection.is_empty() && intersection != boundary_triangles {
            tiers.push(intersection);
        }
    }
    tiers.push(boundary_triangles);
    tiers
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuadraticTransferSample {
    pub nodes: [u32; 7],
    pub weights: [f64; 7],
}

#[derive(Clone, Debug, PartialEq)]
pub struct QuadraticTransferMap {
    source_revision: u64,
    target_revision: u64,
    source_mesh_revision: u64,
    target_mesh_revision: u64,
    source_dofs: usize,
    samples: Vec<Option<QuadraticTransferSample>>,
    swept: Vec<u32>,
}

impl QuadraticTransferMap {
    /// Builds an exact nodal copy between two operators whose quadratic node
    /// layouts come from the same parent mesh. This is used for coefficient or
    /// boundary-condition transactions that do not change geometry.
    pub fn identity_on_mesh(
        mesh: &TriMesh,
        source_operator: &QuadraticWaveOperator,
        target_operator: &QuadraticWaveOperator,
    ) -> Result<Self, TransferError> {
        validate_quadratic_pair(mesh, source_operator, true)?;
        validate_quadratic_pair(mesh, target_operator, false)?;
        if source_operator.node_points() != target_operator.node_points()
            || source_operator.element_nodes() != target_operator.element_nodes()
            || source_operator.degrees_of_freedom() > u32::MAX as usize
        {
            return Err(TransferError::InvalidTarget);
        }
        let samples = (0..source_operator.degrees_of_freedom())
            .map(|index| {
                let mut nodes = [0; 7];
                let mut weights = [0.0; 7];
                nodes[0] = index as u32;
                weights[0] = 1.0;
                Some(QuadraticTransferSample { nodes, weights })
            })
            .collect();
        Ok(Self {
            source_revision: mesh.geometry_revision,
            target_revision: mesh.geometry_revision,
            source_mesh_revision: mesh.mesh_revision,
            target_mesh_revision: mesh.mesh_revision,
            source_dofs: source_operator.degrees_of_freedom(),
            samples,
            swept: Vec::new(),
        })
    }

    /// Locates every target quadratic node in the source parent triangulation and
    /// evaluates the source element's enriched quadratic basis at that point,
    /// but for a node on ground a moved wall swept over, which takes none
    /// ([`Self::swept_nodes`]).
    pub fn build(
        source_mesh: &TriMesh,
        source_operator: &QuadraticWaveOperator,
        target_mesh: &TriMesh,
        target_operator: &QuadraticWaveOperator,
    ) -> Result<Self, TransferError> {
        let mut work = QuadraticTransferWork::new();
        loop {
            if let Some(map) =
                work.step(source_mesh, source_operator, target_mesh, target_operator)?
            {
                return Ok(map);
            }
        }
    }

    pub fn source_dofs(&self) -> usize {
        self.source_dofs
    }

    pub fn samples(&self) -> &[Option<QuadraticTransferSample>] {
        &self.samples
    }

    /// The target nodes, in order, on ground a moved wall swept over: the
    /// old mesh holds their position on the wall's other side, so they take
    /// no sample and extend from their own side however far it lies.
    pub fn swept_nodes(&self) -> &[u32] {
        &self.swept
    }

    pub fn matches(
        &self,
        source_mesh: &TriMesh,
        source_operator: &QuadraticWaveOperator,
        target_mesh: &TriMesh,
        target_operator: &QuadraticWaveOperator,
    ) -> bool {
        self.source_revision == source_mesh.geometry_revision
            && self.target_revision == target_mesh.geometry_revision
            && self.source_mesh_revision == source_mesh.mesh_revision
            && self.target_mesh_revision == target_mesh.mesh_revision
            && self.source_dofs == source_operator.degrees_of_freedom()
            && self.samples.len() == target_operator.degrees_of_freedom()
            && source_operator.geometry_revision() == source_mesh.geometry_revision
            && target_operator.geometry_revision() == target_mesh.geometry_revision
    }

    pub fn exposed_nodes(&self) -> usize {
        self.samples
            .iter()
            .filter(|sample| sample.is_none())
            .count()
    }

    /// Target nodes that copy one source node exactly, because they sit at
    /// the same point; after a local repair that is every node outside the
    /// rebuilt band.
    pub fn exact_nodes(&self) -> usize {
        self.samples
            .iter()
            .filter(|sample| sample.as_ref().is_some_and(is_exact_copy))
            .count()
    }

    pub fn interpolate(
        &self,
        source_values: &[f64],
        exposed_value: f64,
    ) -> Result<Vec<f64>, TransferError> {
        if source_values.len() != self.source_dofs {
            return Err(TransferError::SizeMismatch {
                expected: self.source_dofs,
                actual: source_values.len(),
            });
        }
        if !exposed_value.is_finite() || source_values.iter().any(|value| !value.is_finite()) {
            return Err(TransferError::NonFiniteValues);
        }
        Ok(self
            .samples
            .iter()
            .map(|sample| match sample {
                Some(sample) => sample
                    .nodes
                    .iter()
                    .zip(sample.weights)
                    .map(|(&index, weight)| source_values[index as usize] * weight)
                    .sum(),
                None => exposed_value,
            })
            .collect())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransferPhase {
    Validate,
    Bins(usize),
    Locate(usize),
    Done,
}

impl TransferPhase {
    const fn label(self) -> &'static str {
        match self {
            Self::Validate => "Checking the field transfer",
            Self::Bins(_) => "Indexing the previous mesh",
            Self::Locate(_) => "Locating field samples",
            Self::Done => "Finished",
        }
    }
}

/// Resumable construction of a `QuadraticTransferMap`: validation and the
/// restriction tables in one step, then one source triangle per step into the
/// locator grid, then one target node per step. `QuadraticTransferMap::build`
/// drives it to completion in one call; `QuadraticTransferJob` spreads the same
/// steps across frames, so both produce the same map.
struct QuadraticTransferWork {
    phase: TransferPhase,
    bins: Option<SourceBins>,
    target_groups: Vec<Option<crate::RegionId>>,
    region_groups: Option<BTreeMap<crate::RegionId, crate::RegionId>>,
    target_traces: Vec<TargetTracePreference>,
    source_traces: SourceTraceTriangles,
    /// Source nodes by exact point. A target node at a point held by exactly
    /// one source node copies it; coincident nodes on separated sides stay
    /// with the trace-aware location below.
    source_by_point: BTreeMap<[u64; 2], Vec<u32>>,
    /// The separated sides each source node lies on.
    source_sides: Vec<BTreeSet<BoundaryTraceKey>>,
    /// The curves an edit moved, where it changed the geometry.
    swept_ground: Option<SweptGround>,
    samples: Vec<Option<QuadraticTransferSample>>,
    swept: Vec<u32>,
}

/// Whether a target node on one side of a separated curve would take a donor
/// that `held` places on the curve's other side alone: the source walled the
/// curve on that side only, so the target's side is a face new behind the
/// wall, and nothing of the source is its field. Copied from the other side,
/// a new face's wall nodes took the outer field while its interior stayed
/// zero (MESH-13 of the 2026-10-20 review). A donor on no separated side, as
/// a transmitting curve's, holds the field both sides share.
fn behind_the_wall(
    target: &TargetTracePreference,
    held: impl Fn(&BoundaryTraceKey) -> bool,
) -> bool {
    target.boundaries.iter().any(|key| {
        let BoundaryTraceKey::Curve { curve, side } = key;
        held(&BoundaryTraceKey::Curve {
            curve: *curve,
            side: side.opposite(),
        }) && !held(key)
    })
}

/// How far beyond its own side's nearest element a node on a separated
/// curve may lie and still take that element's field, in the element's
/// barycentric weights: a quarter of its height past an edge. A chord's
/// sagitta is an eighth of its length over the curve's radius in units of
/// a height, 0.14 for a chord as long as the radius; a node further off is
/// not beside its side but somewhere else, as on a curve an edit moved,
/// where the element holding it answers if it lay on the node's side
/// ([`SweptGround`]).
const BESIDE_ITS_SIDE: f64 = 0.25;

fn point_key(point: Point2) -> [u64; 2] {
    [point.x.to_bits(), point.y.to_bits()]
}

fn exact_sample(node: u32) -> QuadraticTransferSample {
    let mut sample = QuadraticTransferSample {
        nodes: [0; 7],
        weights: [0.0; 7],
    };
    sample.nodes[0] = node;
    sample.weights[0] = 1.0;
    sample
}

fn is_exact_copy(sample: &QuadraticTransferSample) -> bool {
    sample.weights[0] == 1.0 && sample.weights[1..].iter().all(|weight| *weight == 0.0)
}

impl QuadraticTransferWork {
    fn new() -> Self {
        Self {
            phase: TransferPhase::Validate,
            bins: None,
            target_groups: Vec::new(),
            region_groups: None,
            target_traces: Vec::new(),
            source_traces: SourceTraceTriangles::default(),
            source_by_point: BTreeMap::new(),
            source_sides: Vec::new(),
            swept_ground: None,
            samples: Vec::new(),
            swept: Vec::new(),
        }
    }

    fn step(
        &mut self,
        source_mesh: &TriMesh,
        source_operator: &QuadraticWaveOperator,
        target_mesh: &TriMesh,
        target_operator: &QuadraticWaveOperator,
    ) -> Result<Option<QuadraticTransferMap>, TransferError> {
        match self.phase {
            TransferPhase::Validate => {
                validate_quadratic_pair(source_mesh, source_operator, true)?;
                validate_quadratic_pair(target_mesh, target_operator, false)?;
                if source_mesh.triangles.is_empty() {
                    return Err(TransferError::EmptySource);
                }
                validate_mesh(source_mesh, true)?;
                validate_mesh(target_mesh, false)?;
                // Unified topology meshes carry exact separated curve sides and
                // sector trace lineage. Region IDs may legitimately appear or
                // disappear when a transmitting divider splits or merges a face,
                // so spatial transfer must not require the same RegionId on both
                // revisions. A mesh built by hand, which has neither, transfers
                // within each region.
                let unified_topology =
                    has_unified_topology(source_mesh) || has_unified_topology(target_mesh);
                self.region_groups = (!unified_topology).then(|| region_groups(target_mesh));
                self.target_groups = vec![None; target_operator.degrees_of_freedom()];
                if let Some(region_groups) = &self.region_groups {
                    for (triangle, nodes) in target_mesh
                        .triangles
                        .iter()
                        .zip(target_operator.element_nodes())
                    {
                        let group = *region_groups
                            .get(&triangle.region)
                            .ok_or(TransferError::InvalidTarget)?;
                        for node in nodes {
                            let assigned = &mut self.target_groups[*node as usize];
                            if assigned.is_some_and(|assigned| assigned != group) {
                                return Err(TransferError::InvalidTarget);
                            }
                            *assigned = Some(group);
                        }
                    }
                }
                self.target_traces = quadratic_trace_nodes(target_mesh, target_operator)?;
                self.source_traces = source_trace_triangles(source_mesh)?;
                self.source_sides = quadratic_trace_nodes(source_mesh, source_operator)?
                    .into_iter()
                    .map(|preference| preference.boundaries)
                    .collect();
                if unified_topology {
                    for (index, point) in source_operator.node_points().iter().enumerate() {
                        self.source_by_point
                            .entry(point_key(*point))
                            .or_default()
                            .push(index as u32);
                    }
                }
                self.swept_ground = SweptGround::new(source_mesh, target_mesh);
                self.bins = Some(SourceBins::new(source_mesh)?);
                self.samples = Vec::with_capacity(target_operator.degrees_of_freedom());
                self.phase = TransferPhase::Bins(0);
            }
            TransferPhase::Bins(index) => {
                if index == source_mesh.triangles.len() {
                    self.phase = TransferPhase::Locate(0);
                } else {
                    self.bins.as_mut().unwrap().insert(source_mesh, index)?;
                    self.phase = TransferPhase::Bins(index + 1);
                }
            }
            TransferPhase::Locate(index) => {
                let points = target_operator.node_points();
                if index == points.len() {
                    self.phase = TransferPhase::Done;
                    return Ok(Some(QuadraticTransferMap {
                        source_revision: source_mesh.geometry_revision,
                        target_revision: target_mesh.geometry_revision,
                        source_mesh_revision: source_mesh.mesh_revision,
                        target_mesh_revision: target_mesh.mesh_revision,
                        source_dofs: source_operator.degrees_of_freedom(),
                        samples: std::mem::take(&mut self.samples),
                        swept: std::mem::take(&mut self.swept),
                    }));
                }
                let target = &self.target_traces[index];
                let exact = match self
                    .source_by_point
                    .get(&point_key(points[index]))
                    .map(Vec::as_slice)
                {
                    Some([node])
                        if !behind_the_wall(target, |key| {
                            self.source_sides[*node as usize].contains(key)
                        }) =>
                    {
                        Some((exact_sample(*node), points[index], *node))
                    }
                    _ => None,
                };
                let new_behind_a_wall = behind_the_wall(target, |key| {
                    self.source_traces.boundaries.contains_key(key)
                });
                let sample = exact.or_else(|| {
                    if new_behind_a_wall {
                        return None;
                    }
                    let preferred = preferred_trace_triangles(target, &self.source_traces);
                    self.bins
                        .as_ref()
                        .unwrap()
                        .locate(
                            source_mesh,
                            points[index],
                            self.target_groups[index],
                            self.region_groups.as_ref(),
                            &preferred,
                        )
                        .map(|(triangle, weights)| {
                            let corners = source_mesh.triangles[triangle as usize]
                                .vertices
                                .map(|vertex| source_mesh.vertices[vertex].point);
                            (
                                QuadraticTransferSample {
                                    nodes: source_operator.element_nodes()[triangle as usize],
                                    weights: enriched_quadratic_basis(weights),
                                },
                                (corners[0] + corners[1] + corners[2]) / 3.0,
                                // The centre lies on no wall.
                                u32::MAX,
                            )
                        })
                });
                // Ground a moved wall swept over takes nothing from the side
                // it lay on before and is extended from its own. An exact
                // copy's old node speaks for its side by its walls.
                let none = BTreeSet::new();
                let swept = sample.as_ref().is_some_and(|(_, from, node)| {
                    self.swept_ground.as_ref().is_some_and(|ground| {
                        let walls = self.source_sides.get(*node as usize).unwrap_or(&none);
                        ground.across((*from, walls), (points[index], &target.boundaries))
                    })
                });
                if swept {
                    self.swept.push(index as u32);
                }
                self.samples
                    .push(sample.filter(|_| !swept).map(|(sample, _, _)| sample));
                self.phase = TransferPhase::Locate(index + 1);
            }
            TransferPhase::Done => {}
        }
        Ok(None)
    }
}

/// Builds a `QuadraticTransferMap` across frames. It owns both discretizations,
/// so a preparation can hold it while the document keeps changing.
pub struct QuadraticTransferJob {
    source_mesh: Arc<TriMesh>,
    source_operator: Arc<QuadraticWaveOperator>,
    target_mesh: Arc<TriMesh>,
    target_operator: Arc<QuadraticWaveOperator>,
    work: QuadraticTransferWork,
    done: bool,
}

impl QuadraticTransferJob {
    pub fn new(
        source_mesh: Arc<TriMesh>,
        source_operator: Arc<QuadraticWaveOperator>,
        target_mesh: Arc<TriMesh>,
        target_operator: Arc<QuadraticWaveOperator>,
    ) -> Self {
        Self {
            source_mesh,
            source_operator,
            target_mesh,
            target_operator,
            work: QuadraticTransferWork::new(),
            done: false,
        }
    }

    pub fn phase(&self) -> &'static str {
        self.work.phase.label()
    }

    /// Runs up to `budget` steps. `Some` carries the finished map or the first
    /// error, after which the job is spent.
    pub fn advance(
        &mut self,
        budget: usize,
    ) -> Option<Result<QuadraticTransferMap, TransferError>> {
        for _ in 0..budget {
            if self.done {
                return None;
            }
            match self.work.step(
                &self.source_mesh,
                &self.source_operator,
                &self.target_mesh,
                &self.target_operator,
            ) {
                Ok(Some(map)) => {
                    self.done = true;
                    return Some(Ok(map));
                }
                Ok(None) => {}
                Err(error) => {
                    self.done = true;
                    return Some(Err(error));
                }
            }
        }
        None
    }
}

fn quadratic_trace_nodes(
    mesh: &TriMesh,
    operator: &QuadraticWaveOperator,
) -> Result<Vec<TargetTracePreference>, TransferError> {
    let mut traces = vec![TargetTracePreference::default(); operator.degrees_of_freedom()];
    for (vertex_index, vertex) in mesh.vertices.iter().enumerate() {
        traces[vertex_index].vertex = vertex.trace;
        if let Some(boundary) = vertex
            .boundary
            .and_then(|boundary| BoundaryTraceKey::from_label(boundary.label))
        {
            traces[vertex_index].boundaries.insert(boundary);
        }
    }
    let owners = triangle_edge_owners(mesh);
    for edge in &mesh.boundary_edges {
        let Some(boundary) = BoundaryTraceKey::from_label(edge.label) else {
            continue;
        };
        let Some([(triangle_index, local_midpoint)]) = owners
            .get(&transfer_edge_key(edge.vertices))
            .map(Vec::as_slice)
        else {
            return Err(TransferError::InvalidTarget);
        };
        for vertex in edge.vertices {
            traces[vertex].boundaries.insert(boundary);
        }
        let node = operator.element_nodes()[*triangle_index][*local_midpoint] as usize;
        traces[node].boundaries.insert(boundary);
    }
    Ok(traces)
}

fn source_trace_triangles(mesh: &TriMesh) -> Result<SourceTraceTriangles, TransferError> {
    let mut traces = SourceTraceTriangles::default();
    let owners = triangle_edge_owners(mesh);
    for (triangle_index, triangle) in mesh.triangles.iter().enumerate() {
        let triangle_index =
            u32::try_from(triangle_index).map_err(|_| TransferError::InvalidSource)?;
        for vertex_index in triangle.vertices {
            if let Some(trace) = mesh.vertices[vertex_index].trace {
                traces
                    .vertices
                    .entry(trace)
                    .or_default()
                    .insert(triangle_index);
            }
        }
    }
    for edge in &mesh.boundary_edges {
        let Some(boundary) = BoundaryTraceKey::from_label(edge.label) else {
            continue;
        };
        let Some([(triangle, _)]) = owners
            .get(&transfer_edge_key(edge.vertices))
            .map(Vec::as_slice)
        else {
            return Err(TransferError::InvalidSource);
        };
        traces
            .boundaries
            .entry(boundary)
            .or_default()
            .insert(u32::try_from(*triangle).map_err(|_| TransferError::InvalidSource)?);
    }
    Ok(traces)
}

pub(crate) fn has_unified_topology(mesh: &TriMesh) -> bool {
    mesh.vertices.iter().any(|vertex| vertex.trace.is_some())
        || mesh
            .boundary_edges
            .iter()
            .any(|edge| matches!(edge.label, BoundaryLabel::Curve { .. }))
}

/// The separated curves an edit moved, each as the old mesh and the new one
/// trace it, for telling the side of a curve ground lay on before from the
/// side it lies on now. A side is that of the curve's nearest point, or none
/// where that point is a tip, as beyond a free end, where both sides meet. A
/// wall an edit moves sweeps over ground, and the value carried there by
/// position belongs to the side the ground lay on before: the moved wall's
/// own side took the other side's values, and the jump the wall held stayed
/// where the wall had been, free in the medium (IV-12 of the testing
/// campaign).
pub(crate) struct SweptGround {
    /// Each moved curve's trace on the old mesh and on the new.
    curves: Vec<(CurveId, SideTrace, SideTrace)>,
}

/// One curve's walls on one mesh, each segment once, turned so the curve's
/// left side lies on its left, on a uniform grid for the nearest segment.
struct SideTrace {
    segments: Vec<[Point2; 2]>,
    /// The segment ending where each begins and the one beginning where it
    /// ends; `None` at a tip.
    neighbours: Vec<[Option<u32>; 2]>,
    minimum: Point2,
    maximum: Point2,
    cell: Point2,
    dimension: usize,
    cells: Vec<Vec<u32>>,
}

impl SweptGround {
    /// The curves both meshes wall whose traces differ, or `None` where no
    /// such curve moved, as in a remesh of the same geometry. Within an
    /// edit, a curve meshed again with other chords counts as moved, its
    /// sides told apart all the same.
    pub(crate) fn new(source: &TriMesh, target: &TriMesh) -> Option<Self> {
        if source.geometry_revision == target.geometry_revision {
            return None;
        }
        let mut before = side_traces(source);
        let curves = side_traces(target)
            .into_iter()
            .filter_map(|(curve, after)| {
                let before = before.remove(&curve)?;
                (before.segments != after.segments).then_some((curve, before, after))
            })
            .collect::<Vec<_>>();
        (!curves.is_empty()).then_some(Self { curves })
    }

    /// Whether ground at `from` on the old mesh lay on another side of a
    /// moved curve than `to` lies on the new one, where neither is beyond a
    /// tip. Each point comes with the sides of the walls it lies on, which
    /// speak for it there. A pair whose box misses both traces' bounds is on
    /// one side: far from a curve, its sides differ only past a half turn,
    /// and ground the curve never swept keeps its place.
    pub(crate) fn across(
        &self,
        (from, from_walls): (Point2, &BTreeSet<BoundaryTraceKey>),
        (to, to_walls): (Point2, &BTreeSet<BoundaryTraceKey>),
    ) -> bool {
        let low = Point2::new(from.x.min(to.x), from.y.min(to.y));
        let high = Point2::new(from.x.max(to.x), from.y.max(to.y));
        self.curves.iter().any(|(curve, before, after)| {
            if !before.meets(low, high) && !after.meets(low, high) {
                return false;
            }
            let then = wall_side(from_walls, *curve).unwrap_or_else(|| before.side(from));
            let now = wall_side(to_walls, *curve).unwrap_or_else(|| after.side(to));
            matches!((then, now), (Some(then), Some(now)) if then != now)
        })
    }
}

/// The side of `curve` a point on the walls `walls` lies on, if it lies on
/// the curve's: none at a tip, where both sides meet.
fn wall_side(walls: &BTreeSet<BoundaryTraceKey>, curve: CurveId) -> Option<Option<CurveTraceSide>> {
    let mut sides = walls.iter().filter_map(|key| {
        let BoundaryTraceKey::Curve { curve: wall, side } = key;
        (*wall == curve).then_some(*side)
    });
    match (sides.next(), sides.next()) {
        (None, _) => None,
        (Some(side), None) => Some(Some(side)),
        (Some(_), Some(_)) => Some(None),
    }
}

/// Each separated curve's trace on `mesh`.
fn side_traces(mesh: &TriMesh) -> BTreeMap<CurveId, SideTrace> {
    let mut walls = BTreeMap::<(usize, usize), (CurveId, CurveTraceSide, [usize; 2])>::new();
    for edge in &mesh.boundary_edges {
        if let BoundaryLabel::Curve {
            curve,
            side,
            separated: true,
            ..
        } = edge.label
        {
            walls.insert(
                transfer_edge_key(edge.vertices),
                (curve, side, edge.vertices),
            );
        }
    }
    let mut segments = BTreeMap::<CurveId, BTreeMap<[[u64; 2]; 2], [Point2; 2]>>::new();
    for triangle in &mesh.triangles {
        let [a, b, c] = triangle.vertices;
        for (edge, opposite) in [([a, b], c), ([b, c], a), ([c, a], b)] {
            let Some((curve, side, vertices)) = walls.get(&transfer_edge_key(edge)) else {
                continue;
            };
            let [start, end] = vertices.map(|vertex| mesh.vertices[vertex].point);
            let owner_left = crate::orient2d(start, end, mesh.vertices[opposite].point)
                == crate::PredicateSign::Positive;
            let [start, end] = if owner_left == (*side == CurveTraceSide::Left) {
                [start, end]
            } else {
                [end, start]
            };
            segments
                .entry(*curve)
                .or_default()
                .insert([point_key(start), point_key(end)], [start, end]);
        }
    }
    segments
        .into_iter()
        .map(|(curve, segments)| (curve, SideTrace::new(segments.into_values().collect())))
        .collect()
}

impl SideTrace {
    fn new(segments: Vec<[Point2; 2]>) -> Self {
        let mut starts = BTreeMap::<[u64; 2], u32>::new();
        let mut ends = BTreeMap::<[u64; 2], u32>::new();
        for (index, [start, end]) in segments.iter().enumerate() {
            starts.entry(point_key(*start)).or_insert(index as u32);
            ends.entry(point_key(*end)).or_insert(index as u32);
        }
        let neighbours = segments
            .iter()
            .map(|[start, end]| {
                [
                    ends.get(&point_key(*start)).copied(),
                    starts.get(&point_key(*end)).copied(),
                ]
            })
            .collect();
        let mut minimum = Point2::new(f64::INFINITY, f64::INFINITY);
        let mut maximum = Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
        for point in segments.iter().flatten() {
            minimum.x = minimum.x.min(point.x);
            minimum.y = minimum.y.min(point.y);
            maximum.x = maximum.x.max(point.x);
            maximum.y = maximum.y.max(point.y);
        }
        let dimension = (segments.len() as f64).sqrt().ceil().clamp(1.0, 512.0) as usize;
        let width = |extent: f64| {
            if extent > 0.0 {
                extent / dimension as f64
            } else {
                1.0
            }
        };
        let mut trace = Self {
            segments,
            neighbours,
            minimum,
            maximum,
            cell: Point2::new(width(maximum.x - minimum.x), width(maximum.y - minimum.y)),
            dimension,
            cells: vec![Vec::new(); dimension * dimension],
        };
        for index in 0..trace.segments.len() {
            let [start, end] = trace.segments[index];
            let [x0, y0] = trace.bin(Point2::new(start.x.min(end.x), start.y.min(end.y)));
            let [x1, y1] = trace.bin(Point2::new(start.x.max(end.x), start.y.max(end.y)));
            for y in y0..=y1 {
                for x in x0..=x1 {
                    trace.cells[y * dimension + x].push(index as u32);
                }
            }
        }
        trace
    }

    fn meets(&self, low: Point2, high: Point2) -> bool {
        high.x >= self.minimum.x
            && low.x <= self.maximum.x
            && high.y >= self.minimum.y
            && low.y <= self.maximum.y
    }

    fn bin(&self, point: Point2) -> [usize; 2] {
        bin_index(point, self.minimum, self.cell, self.dimension)
    }

    /// The segment nearest `point`, the lowest-numbered of equals, from
    /// square rings of cells searched outward until nothing outside them can
    /// be nearer.
    fn nearest(&self, point: Point2) -> Option<usize> {
        let [cx, cy] = self.bin(point);
        let last = self.dimension - 1;
        let slack = 1.0e-9 * (self.cell.x + self.cell.y);
        let mut best = None::<(f64, u32)>;
        for ring in 0..=last {
            let (x0, x1) = (cx.saturating_sub(ring), (cx + ring).min(last));
            let (y0, y1) = (cy.saturating_sub(ring), (cy + ring).min(last));
            for y in y0..=y1 {
                let edge_row = y.abs_diff(cy) == ring;
                for x in x0..=x1 {
                    if !edge_row && x.abs_diff(cx) != ring {
                        continue;
                    }
                    for &index in &self.cells[y * self.dimension + x] {
                        let [start, end] = self.segments[index as usize];
                        let along = end - start;
                        let t = ((point - start).dot(along) / along.dot(along)).clamp(0.0, 1.0);
                        let offset = start + along * t - point;
                        let distance = offset.dot(offset);
                        if best.is_none_or(|(nearest, holder)| {
                            distance < nearest || (distance == nearest && index < holder)
                        }) {
                            best = Some((distance, index));
                        }
                    }
                }
            }
            let mut outside = f64::INFINITY;
            if x0 > 0 {
                outside = outside.min(point.x - (self.minimum.x + x0 as f64 * self.cell.x));
            }
            if x1 < last {
                outside = outside.min(self.minimum.x + (x1 + 1) as f64 * self.cell.x - point.x);
            }
            if y0 > 0 {
                outside = outside.min(point.y - (self.minimum.y + y0 as f64 * self.cell.y));
            }
            if y1 < last {
                outside = outside.min(self.minimum.y + (y1 + 1) as f64 * self.cell.y - point.y);
            }
            if outside == f64::INFINITY {
                break;
            }
            let reach = outside - slack;
            if reach > 0.0 && best.is_some_and(|(nearest, _)| nearest < reach * reach) {
                break;
            }
        }
        best.map(|(_, index)| index as usize)
    }

    /// The side of the curve `point` lies on: of the nearest segment, or at
    /// a corner nearest, of the two segments' normals together; none at a
    /// tip or on the curve.
    fn side(&self, point: Point2) -> Option<CurveTraceSide> {
        let index = self.nearest(point)?;
        let [start, end] = self.segments[index];
        let along = end - start;
        let t = (point - start).dot(along) / along.dot(along);
        let normal = |along: Point2| Point2::new(-along.y, along.x) / along.norm();
        let turn = if t <= 0.0 || t >= 1.0 {
            let (corner, other) = if t <= 0.0 {
                (start, self.neighbours[index][0]?)
            } else {
                (end, self.neighbours[index][1]?)
            };
            let [other_start, other_end] = self.segments[other as usize];
            (point - corner).dot(normal(along) + normal(other_end - other_start))
        } else {
            along.cross(point - start)
        };
        if turn > 0.0 {
            Some(CurveTraceSide::Left)
        } else if turn < 0.0 {
            Some(CurveTraceSide::Right)
        } else {
            None
        }
    }
}

fn transfer_edge_key(vertices: [usize; 2]) -> (usize, usize) {
    (vertices[0].min(vertices[1]), vertices[0].max(vertices[1]))
}

fn triangle_edge_owners(mesh: &TriMesh) -> BTreeMap<(usize, usize), Vec<(usize, usize)>> {
    let mut owners = BTreeMap::<_, Vec<_>>::new();
    for (triangle_index, triangle) in mesh.triangles.iter().enumerate() {
        for (vertices, local_midpoint) in [
            ([triangle.vertices[0], triangle.vertices[1]], 3),
            ([triangle.vertices[1], triangle.vertices[2]], 4),
            ([triangle.vertices[0], triangle.vertices[2]], 5),
        ] {
            owners
                .entry(transfer_edge_key(vertices))
                .or_default()
                .push((triangle_index, local_midpoint));
        }
    }
    owners
}

fn validate_quadratic_pair(
    mesh: &TriMesh,
    operator: &QuadraticWaveOperator,
    source: bool,
) -> Result<(), TransferError> {
    let error = if source {
        TransferError::InvalidSource
    } else {
        TransferError::InvalidTarget
    };
    if operator.geometry_revision() != mesh.geometry_revision
        || operator.mesh_revision() != mesh.mesh_revision
        || operator.element_nodes().len() != mesh.triangles.len()
        || operator.node_points().len() < mesh.vertices.len()
        || operator
            .node_points()
            .iter()
            .zip(&mesh.vertices)
            .any(|(node, vertex)| *node != vertex.point)
        || operator
            .element_nodes()
            .iter()
            .flatten()
            .any(|index| *index as usize >= operator.degrees_of_freedom())
    {
        Err(error)
    } else {
        Ok(())
    }
}

/// Reconstructs velocity at the current level from a centered two-level state.
/// `undamped_acceleration` is forcing minus `M^-1 K u`, before `-gamma v`.
pub fn centered_velocity(
    previous: f64,
    current: f64,
    undamped_acceleration: f64,
    damping_ratio: f64,
    time_step: f64,
) -> Option<f64> {
    let velocity = ((current - previous) / time_step + 0.5 * time_step * undamped_acceleration)
        / (1.0 + 0.5 * damping_ratio * time_step);
    (previous.is_finite()
        && current.is_finite()
        && undamped_acceleration.is_finite()
        && damping_ratio.is_finite()
        && damping_ratio >= 0.0
        && time_step.is_finite()
        && time_step > 0.0
        && velocity.is_finite())
    .then_some(velocity)
}

pub fn centered_previous(
    current: f64,
    velocity: f64,
    undamped_acceleration: f64,
    damping_ratio: f64,
    time_step: f64,
) -> Option<f64> {
    let acceleration = undamped_acceleration - damping_ratio * velocity;
    let previous = current - time_step * velocity + 0.5 * time_step * time_step * acceleration;
    (current.is_finite()
        && velocity.is_finite()
        && undamped_acceleration.is_finite()
        && damping_ratio.is_finite()
        && damping_ratio >= 0.0
        && time_step.is_finite()
        && time_step > 0.0
        && previous.is_finite())
    .then_some(previous)
}

fn validate_mesh(mesh: &TriMesh, source: bool) -> Result<(), TransferError> {
    let error = if source {
        TransferError::InvalidSource
    } else {
        TransferError::InvalidTarget
    };
    if mesh.vertices.iter().any(|vertex| !vertex.point.finite())
        || mesh.triangles.iter().any(|triangle| {
            triangle
                .vertices
                .iter()
                .any(|&index| index >= mesh.vertices.len())
        })
        || mesh.vertices.len() > u32::MAX as usize
    {
        return Err(error);
    }
    if mesh.triangles.iter().any(|triangle| {
        let [a, b, c] = triangle.vertices.map(|index| mesh.vertices[index].point);
        let area = (b - a).cross(c - a);
        !area.is_finite() || area <= 0.0
    }) {
        return Err(error);
    }
    Ok(())
}

/// Uniform grid over the source triangles' bounding box. It is filled one
/// triangle at a time and queried one point at a time so that a cooperative
/// job can spread both halves of the location work across frames.
struct SourceBins {
    minimum: Point2,
    maximum: Point2,
    cell: Point2,
    dimension: usize,
    bins: Vec<Vec<u32>>,
}

impl SourceBins {
    fn new(source: &TriMesh) -> Result<Self, TransferError> {
        let mut minimum = Point2::new(f64::INFINITY, f64::INFINITY);
        let mut maximum = Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
        for vertex in &source.vertices {
            minimum.x = minimum.x.min(vertex.point.x);
            minimum.y = minimum.y.min(vertex.point.y);
            maximum.x = maximum.x.max(vertex.point.x);
            maximum.y = maximum.y.max(vertex.point.y);
        }
        let extent = Point2::new(maximum.x - minimum.x, maximum.y - minimum.y);
        if !(extent.x > 0.0 && extent.y > 0.0) {
            return Err(TransferError::InvalidSource);
        }
        let dimension = (source.triangles.len() as f64).sqrt().ceil() as usize;
        let dimension = dimension.clamp(8, 512);
        Ok(Self {
            minimum,
            maximum,
            cell: Point2::new(extent.x / dimension as f64, extent.y / dimension as f64),
            dimension,
            bins: vec![Vec::<u32>::new(); dimension * dimension],
        })
    }

    fn insert(&mut self, source: &TriMesh, triangle_index: usize) -> Result<(), TransferError> {
        let triangle = source.triangles[triangle_index];
        let points = triangle.vertices.map(|i| source.vertices[i].point);
        let lo = Point2::new(
            points.iter().map(|p| p.x).fold(f64::INFINITY, f64::min),
            points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min),
        );
        let hi = Point2::new(
            points.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max),
            points.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max),
        );
        let [x0, y0] = bin_index(lo, self.minimum, self.cell, self.dimension);
        let [x1, y1] = bin_index(hi, self.minimum, self.cell, self.dimension);
        let triangle_index =
            u32::try_from(triangle_index).map_err(|_| TransferError::InvalidSource)?;
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.bins[y * self.dimension + x].push(triangle_index);
            }
        }
        Ok(())
    }

    /// The source triangle containing `point` and its barycentric weights.
    /// Each preferred set is tried in turn, then the triangle of the broadest
    /// the point lies beside, and only then every triangle; a region group
    /// restricts the candidates when the caller supplies one.
    fn locate(
        &self,
        source: &TriMesh,
        point: Point2,
        target_group: Option<crate::RegionId>,
        region_groups: Option<&BTreeMap<crate::RegionId, crate::RegionId>>,
        preferred: &[BTreeSet<u32>],
    ) -> Option<(u32, [f64; 3])> {
        if point.x < self.minimum.x
            || point.x > self.maximum.x
            || point.y < self.minimum.y
            || point.y > self.maximum.y
        {
            return None;
        }
        let admitted = |triangle_index: u32| {
            target_group.is_none_or(|target_group| {
                let region = source.triangles[triangle_index as usize].region;
                region_groups
                    .and_then(|groups| groups.get(&region))
                    .copied()
                    .unwrap_or(region)
                    == target_group
            })
        };
        let corners = |triangle_index: u32| {
            source.triangles[triangle_index as usize]
                .vertices
                .map(|i| source.vertices[i].point)
        };
        let [x, y] = bin_index(point, self.minimum, self.cell, self.dimension);
        let bin = &self.bins[y * self.dimension + x];
        for tier in preferred {
            for &triangle_index in bin {
                if !tier.contains(&triangle_index) || !admitted(triangle_index) {
                    continue;
                }
                if let Some(weights) = barycentric(point, corners(triangle_index)) {
                    return Some((triangle_index, weights));
                }
            }
        }
        // A node on a separated curve that no element of its own side holds
        // lies beside that side: each mesh traces the curve with its own
        // chords, and a node on the curve stands up to a chord's sagitta
        // outside the other mesh's elements on its side, inside those of the
        // other side. It takes its own side's field from the element it lies
        // least outside of, the element's quadratic carried that far beyond
        // it; searched for among every triangle, it took the field across the
        // wall (IV-10 of the testing campaign).
        if let Some(side) = preferred.last() {
            let beside = side
                .iter()
                .filter(|triangle_index| admitted(**triangle_index))
                .filter_map(|&triangle_index| {
                    let weights = barycentric_weights(point, corners(triangle_index))?;
                    let outside = weights.into_iter().fold(f64::INFINITY, f64::min);
                    Some((triangle_index, weights, outside))
                })
                .fold(
                    None,
                    |best: Option<(u32, [f64; 3], f64)>, candidate| match best {
                        Some(best) if best.2 >= candidate.2 => Some(best),
                        _ => Some(candidate),
                    },
                );
            if let Some((triangle_index, weights, outside)) = beside
                && outside >= -BESIDE_ITS_SIDE
            {
                return Some((triangle_index, weights));
            }
        }
        for &triangle_index in bin {
            if !admitted(triangle_index) {
                continue;
            }
            if let Some(weights) = barycentric(point, corners(triangle_index)) {
                return Some((triangle_index, weights));
            }
        }
        None
    }
}

fn bin_index(point: Point2, minimum: Point2, cell: Point2, dimension: usize) -> [usize; 2] {
    let index = |value: f64, min: f64, width: f64| {
        (((value - min) / width).floor() as isize).clamp(0, dimension as isize - 1) as usize
    };
    [
        index(point.x, minimum.x, cell.x),
        index(point.y, minimum.y, cell.y),
    ]
}

/// Each region its own group: the transfer stays within a region.
fn region_groups(mesh: &TriMesh) -> BTreeMap<crate::RegionId, crate::RegionId> {
    mesh.triangles
        .iter()
        .map(|triangle| (triangle.region, triangle.region))
        .collect()
}

/// `point`'s barycentric weights in `triangle`, wherever it lies.
fn barycentric_weights(point: Point2, triangle: [Point2; 3]) -> Option<[f64; 3]> {
    let [a, b, c] = triangle;
    let denominator = (b - a).cross(c - a);
    if !denominator.is_finite() || denominator <= 0.0 {
        return None;
    }
    let w1 = (point - a).cross(c - a) / denominator;
    let w2 = (b - a).cross(point - a) / denominator;
    Some([1.0 - w1 - w2, w1, w2])
}

fn barycentric(point: Point2, triangle: [Point2; 3]) -> Option<[f64; 3]> {
    let weights = barycentric_weights(point, triangle)?;
    let [a, b, c] = triangle;
    let denominator = (b - a).cross(c - a);
    // The weights round with the coordinates' magnitude, and more the smaller
    // the element is: a weight is a distance over the element's height. An
    // absolute tolerance refused points on a small element's own edge
    // (MESH-10 of the 2026-10-20 review).
    let scale = triangle
        .iter()
        .map(|p| p.x.abs().max(p.y.abs()))
        .fold(1.0, f64::max);
    let longest = [b - a, c - b, a - c]
        .iter()
        .map(|edge| edge.norm())
        .fold(0.0, f64::max);
    let height = denominator / longest;
    let tolerance = 64.0 * f64::EPSILON * scale / height.min(1.0);
    weights
        .iter()
        .all(|weight| *weight >= -tolerance && *weight <= 1.0 + tolerance)
        .then_some(weights)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AtomCoarsening, BACKGROUND_REGION, BoundaryEdge, BoundaryLabel, CurveNode, CurveSpan,
        CurveSpanId, CurveSpline, FaceRegionAssignment, Material, MaterialId, MeshQuality,
        MeshTriangle, MeshVertex, MeshingOptions, OpenCubicSpline, OuterBoundaryConditions,
        OwnedTopologyWaveModel, PeriodicCubicSpline, Region, RegionId, SpanBehavior, TopologyCurve,
        TopologyGeometry, TopologyMeshPlan, TopologyVertex, TopologyVertexId,
        TopologyVertexLocation, TopologyWaveModel, WaveCoefficients, carve_topology_mesh,
        compile_topology, mesh_topology_plan,
    };

    /// MESH-10 of the 2026-10-20 review: a point on an element's own edge lies
    /// in it whatever the element's size. The tolerance was absolute, while
    /// the weights' rounding grows as the element shrinks: with adaptation's
    /// minimum edge at 0.005 near walls, separated midpoints took the other
    /// side's value and new hole-rim points were extended, not interpolated.
    #[test]
    fn a_point_on_a_small_elements_edge_lies_in_it() {
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut uniform = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1_u64 << 53) as f64
        };
        let edge = 0.005;
        let mut refused = 0;
        for _ in 0..20_000 {
            let a = Point2::new(-0.9 + 1.8 * uniform(), -0.9 + 1.8 * uniform());
            let angle = std::f64::consts::TAU * uniform();
            let towards =
                |angle: f64| Point2::new(a.x + edge * angle.cos(), a.y + edge * angle.sin());
            let (b, c) = (towards(angle), towards(angle + 1.2));
            for (from, to) in [(a, b), (b, c), (c, a)] {
                if barycentric(from.lerp(to, uniform()), [a, b, c]).is_none() {
                    refused += 1;
                }
            }
        }
        assert_eq!(refused, 0, "points on their own element's edge refused");
    }

    /// A carved mesh keeps most of its nodes at exactly their old points, and
    /// the transfer copies those one to one, bit for bit, instead of locating
    /// and interpolating them. Nodes inside the rebuilt band still interpolate.
    #[test]
    fn carved_meshes_transfer_untouched_nodes_exactly() {
        let options = MeshingOptions {
            target_edge_length: 0.12,
            curve_tolerance: 8.0e-4,
            minimum_angle_degrees: 10.0,
            max_vertices: 40_000,
            max_triangles: 80_000,
            max_refinement_steps: 40_000,
        };
        let hole = |center: Point2| TopologyGeometry {
            curves: vec![
                TopologyCurve::new(
                    crate::CurveId(4),
                    CurveSpline::Closed(PeriodicCubicSpline::rounded(center, 0.3)),
                    (0..8)
                        .map(|index| CurveSpan {
                            id: CurveSpanId(40 + index),
                            behavior: SpanBehavior::REFLECTING,
                        })
                        .collect(),
                )
                .unwrap(),
            ],
            ..TopologyGeometry::default()
        };
        let plan_for = |topology: &crate::TopologySnapshot, center: Point2| {
            let inner = topology.face_at(center).unwrap();
            let assignments = topology
                .faces
                .iter()
                .map(|face| FaceRegionAssignment {
                    face: face.id,
                    region: (face.id != inner).then_some(RegionId(1)),
                })
                .collect::<Vec<_>>();
            TopologyMeshPlan::new(topology, &assignments)
                .unwrap()
                .coarsened(topology, AtomCoarsening::from_meshing(options))
                .unwrap()
        };
        let from = Point2::new(0.0, 0.0);
        let to = Point2::new(0.04, 0.03);
        let before_topology = compile_topology(&hole(from), 1).unwrap();
        let before_plan = plan_for(&before_topology, from);
        let before = mesh_topology_plan(&before_plan, 10, options).unwrap();
        let after_topology = compile_topology(&hole(to), 2).unwrap();
        let after_plan = plan_for(&after_topology, to);
        let (after, report) = carve_topology_mesh(
            Arc::new(before.clone()),
            &before_plan,
            after_plan.clone(),
            Arc::new(after_topology),
            11,
            options,
            true,
        )
        .unwrap();

        let materials = [Material::default_medium()];
        let regions = [Region {
            id: RegionId(1),
            material: MaterialId(1),
            frame: crate::MaterialFrame::world(),
        }];
        let model = TopologyWaveModel {
            physics: crate::PhysicsModel::Mechanical,
            materials: &materials,
            regions: &regions,
            outer_boundaries: &OuterBoundaryConditions::default(),
        };
        let source_operator =
            QuadraticWaveOperator::assemble_topology(&before, &before_plan, model).unwrap();
        let target_operator =
            QuadraticWaveOperator::assemble_topology(&after, &after_plan, model).unwrap();
        let map = QuadraticTransferMap::build(&before, &source_operator, &after, &target_operator)
            .unwrap();

        let field = |point: Point2| (3.1 * point.x).sin() * (2.3 * point.y).cos() + 0.37 * point.x;
        let source_values = source_operator
            .node_points()
            .iter()
            .map(|point| field(*point))
            .collect::<Vec<_>>();
        const EXPOSED: f64 = 1.0e9;
        let target_values = map.interpolate(&source_values, EXPOSED).unwrap();
        let source_by_point = source_operator
            .node_points()
            .iter()
            .zip(&source_values)
            .map(|(point, value)| (point_key(*point), *value))
            .collect::<BTreeMap<_, _>>();
        let mut coincident = 0;
        let mut exposed = 0;
        for (point, value) in target_operator.node_points().iter().zip(&target_values) {
            if let Some(source_value) = source_by_point.get(&point_key(*point)) {
                coincident += 1;
                assert_eq!(value.to_bits(), source_value.to_bits(), "{point:?}");
            }
            if *value == EXPOSED {
                // Only the area the hole uncovered has no source.
                exposed += 1;
                assert!((*point - from).norm() < 0.32, "{point:?}");
            }
        }
        assert!(report.kept_triangles > 0);
        assert_eq!(map.exact_nodes(), coincident);
        assert!(
            coincident * 10 > target_operator.degrees_of_freedom() * 8,
            "{coincident}"
        );
        assert!(coincident < target_operator.degrees_of_freedom());
        assert_eq!(map.exposed_nodes(), exposed);
        assert!(exposed > 0);
    }

    fn mesh(revision: u64, points: &[[f64; 2]], triangles: &[[usize; 3]]) -> TriMesh {
        TriMesh {
            geometry_revision: revision,
            mesh_revision: revision,
            vertices: points
                .iter()
                .map(|p| MeshVertex {
                    point: Point2::new(p[0], p[1]),
                    boundary: None,
                    trace: None,
                })
                .collect(),
            triangles: triangles
                .iter()
                .map(|vertices| MeshTriangle {
                    vertices: *vertices,
                    region: BACKGROUND_REGION,
                })
                .collect(),
            boundary_edges: vec![],
            requested_sizes: vec![],
            quality: MeshQuality {
                minimum_angle_degrees: 45.0,
                maximum_edge_length: 1.0,
            },
        }
    }

    fn topology_mesh(geometry: &TopologyGeometry, revision: u64) -> (TriMesh, TopologyMeshPlan) {
        let topology = compile_topology(geometry, revision).unwrap();
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
        let mesh = mesh_topology_plan(
            &plan,
            revision,
            MeshingOptions {
                target_edge_length: 0.4,
                minimum_angle_degrees: 8.0,
                max_vertices: 20_000,
                max_triangles: 40_000,
                max_refinement_steps: 20_000,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        (mesh, plan)
    }

    fn transmitting_divider() -> TopologyGeometry {
        let bottom = TopologyVertexId(1);
        let top = TopologyVertexId(2);
        let mut divider = TopologyCurve::new(
            CurveId(1),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![Point2::new(0.0, -1.0), Point2::new(0.0, 1.0)])
                    .unwrap(),
            ),
            vec![CurveSpan {
                id: CurveSpanId(1),
                behavior: SpanBehavior::Transmitting,
            }],
        )
        .unwrap();
        divider.nodes = vec![
            CurveNode {
                vertex: Some(bottom),
            },
            CurveNode { vertex: Some(top) },
        ];
        TopologyGeometry {
            curves: vec![divider],
            vertices: vec![
                TopologyVertex {
                    id: bottom,
                    location: TopologyVertexLocation::Outer {
                        side: crate::OuterSide::Bottom,
                        fraction: 0.5,
                    },
                },
                TopologyVertex {
                    id: top,
                    location: TopologyVertexLocation::Outer {
                        side: crate::OuterSide::Top,
                        fraction: 0.5,
                    },
                },
            ],
            ..TopologyGeometry::default()
        }
    }

    fn separated_t_junction() -> TopologyGeometry {
        let center = TopologyVertexId(10);
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
                    id: CurveSpanId(10),
                    behavior: SpanBehavior::REFLECTING,
                },
                CurveSpan {
                    id: CurveSpanId(11),
                    behavior: SpanBehavior::REFLECTING,
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
                id: CurveSpanId(12),
                behavior: SpanBehavior::REFLECTING,
            }],
        )
        .unwrap();
        branch.nodes[0].vertex = Some(center);
        TopologyGeometry {
            curves: vec![horizontal, branch],
            vertices: vec![TopologyVertex {
                id: center,
                location: TopologyVertexLocation::Interior(Point2::new(0.0, 0.0)),
            }],
            ..TopologyGeometry::default()
        }
    }

    #[test]
    fn affine_fields_transfer_exactly_and_revisions_are_recorded() {
        let source = mesh(
            3,
            &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            &[[0, 1, 2], [0, 2, 3]],
        );
        let target = mesh(8, &[[0.25, 0.1], [0.8, 0.4], [0.2, 0.9]], &[[0, 1, 2]]);
        let map = TransferMap::build(&source, &target).unwrap();
        let values: Vec<_> = source
            .vertices
            .iter()
            .map(|vertex| 2.0 * vertex.point.x - 3.0 * vertex.point.y + 0.4)
            .collect();
        let transferred = map.interpolate(&values, 0.0).unwrap();
        for (vertex, value) in target.vertices.iter().zip(transferred) {
            let exact = 2.0 * vertex.point.x - 3.0 * vertex.point.y + 0.4;
            assert!((value - exact).abs() < 1.0e-13);
        }
        assert_eq!(map.source_revision(), 3);
        assert_eq!(map.target_revision(), 8);
        let mut stale = source.clone();
        stale.geometry_revision += 1;
        assert!(!map.matches_meshes(&stale, &target));
    }

    #[test]
    fn quadratic_transfer_reproduces_quadratics_and_arbitrary_self_state() {
        let source = mesh(
            3,
            &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            &[[0, 1, 2], [0, 2, 3]],
        );
        let target = mesh(8, &[[0.25, 0.1], [0.8, 0.4], [0.2, 0.9]], &[[0, 1, 2]]);
        let source_operator =
            QuadraticWaveOperator::assemble(&source, WaveCoefficients::default()).unwrap();
        let target_operator =
            QuadraticWaveOperator::assemble(&target, WaveCoefficients::default()).unwrap();
        let map = QuadraticTransferMap::build(&source, &source_operator, &target, &target_operator)
            .unwrap();
        assert!(map.matches(&source, &source_operator, &target, &target_operator));
        assert_eq!(map.exposed_nodes(), 0);
        let polynomial = |point: Point2| {
            0.7 + 2.0 * point.x - 0.4 * point.y + 1.3 * point.x * point.x - 0.8 * point.x * point.y
                + 0.2 * point.y * point.y
        };
        let source_values = source_operator
            .node_points()
            .iter()
            .map(|point| polynomial(*point))
            .collect::<Vec<_>>();
        let transferred = map.interpolate(&source_values, -10.0).unwrap();
        for (point, value) in target_operator.node_points().iter().zip(transferred) {
            assert!((value - polynomial(*point)).abs() < 2.0e-13);
        }

        let self_map =
            QuadraticTransferMap::build(&source, &source_operator, &source, &source_operator)
                .unwrap();
        let arbitrary = (0..source_operator.degrees_of_freedom())
            .map(|index| (index as f64 * 1.7).sin())
            .collect::<Vec<_>>();
        let copied = self_map.interpolate(&arbitrary, 0.0).unwrap();
        for (actual, expected) in copied.iter().zip(arbitrary) {
            assert!((actual - expected).abs() < 2.0e-13);
        }

        let target_operator = QuadraticWaveOperator::assemble_with_boundary(
            &source,
            WaveCoefficients::default(),
            crate::OuterBoundaryCondition::FirstOrderOutgoing,
        )
        .unwrap();
        let identity =
            QuadraticTransferMap::identity_on_mesh(&source, &source_operator, &target_operator)
                .unwrap();
        assert_eq!(identity.interpolate(&copied, 0.0).unwrap(), copied);
    }

    /// A wall between two regions: one triangle on each side of a separated
    /// edge, whose two traces coincide node for node.
    #[test]
    fn quadratic_transfer_keeps_coincident_wall_traces_on_their_own_regions() {
        let points = [
            [0.0, 0.0],
            [1.0, 0.0],
            [0.5, 0.5],
            [0.0, 0.0],
            [1.0, 0.0],
            [0.5, -0.5],
        ];
        let mut source = mesh(3, &points, &[[0, 1, 2], [3, 5, 4]]);
        source.triangles[1].region = RegionId(2);
        for (vertex, trace) in [(0, 1), (3, 1), (1, 2), (4, 2)] {
            source.vertices[vertex].trace = Some(TraceVertexId(trace));
        }
        source.boundary_edges = vec![
            BoundaryEdge {
                vertices: [0, 1],
                label: BoundaryLabel::Curve {
                    curve: CurveId(1),
                    span: CurveSpanId(10),
                    side: CurveTraceSide::Left,
                    separated: true,
                },
                parameters: [0.0, 1.0],
            },
            BoundaryEdge {
                vertices: [4, 3],
                label: BoundaryLabel::Curve {
                    curve: CurveId(1),
                    span: CurveSpanId(10),
                    side: CurveTraceSide::Right,
                    separated: true,
                },
                parameters: [1.0, 0.0],
            },
        ];
        let mut target = source.clone();
        target.geometry_revision = 8;
        let media = OwnedTopologyWaveModel {
            materials: vec![
                Material::default_medium(),
                Material {
                    id: MaterialId(2),
                    name: "Inside".into(),
                    mass_density: crate::ScalarField::constant(1.0),
                    stiffness: crate::ScalarField::constant(1.0),
                    axis_ratio: crate::ScalarField::constant(1.0),
                    parameters: vec![],
                    color: [1, 2, 3],
                    ..Material::default_medium()
                },
            ],
            regions: vec![
                Region {
                    id: BACKGROUND_REGION,
                    material: MaterialId(1),
                    frame: crate::MaterialFrame::world(),
                },
                Region {
                    id: RegionId(2),
                    material: MaterialId(2),
                    frame: crate::MaterialFrame::world(),
                },
            ],
            ..crate::test_scenes::library(vec![])
        };
        let source_operator =
            QuadraticWaveOperator::assemble_model(&source, media.as_model()).unwrap();
        let target_operator =
            QuadraticWaveOperator::assemble_model(&target, media.as_model()).unwrap();
        let map = QuadraticTransferMap::build(&source, &source_operator, &target, &target_operator)
            .unwrap();
        // The cooperative job runs the same steps one at a time and lands on
        // the same map.
        let mut job = QuadraticTransferJob::new(
            Arc::new(source.clone()),
            Arc::new(source_operator.clone()),
            Arc::new(target.clone()),
            Arc::new(target_operator.clone()),
        );
        let mut steps = 0usize;
        let stepped = loop {
            steps += 1;
            if let Some(result) = job.advance(1) {
                break result.unwrap();
            }
        };
        assert_eq!(stepped, map);
        assert!(steps > source.triangles.len() + 1, "{steps}");
        let mut values = vec![0.0; source_operator.degrees_of_freedom()];
        for node in source_operator.element_nodes()[0] {
            values[node as usize] = 1.0;
        }
        for node in source_operator.element_nodes()[1] {
            values[node as usize] = 9.0;
        }
        let transferred = map.interpolate(&values, -1.0).unwrap();
        for node in target_operator.element_nodes()[0] {
            assert!((transferred[node as usize] - 1.0).abs() < 1.0e-12);
        }
        for node in target_operator.element_nodes()[1] {
            assert!((transferred[node as usize] - 9.0).abs() < 1.0e-12);
        }
    }

    #[test]
    fn quadratic_transfer_keeps_unified_curve_sides_separate() {
        let points = [
            [0.0, 0.0],
            [1.0, 0.0],
            [0.0, 1.0],
            [0.0, 0.0],
            [1.0, 0.0],
            [0.0, 1.0],
        ];
        let mut source = mesh(3, &points, &[[0, 1, 2], [3, 4, 5]]);
        for vertex in &mut source.vertices[..3] {
            vertex.trace = Some(TraceVertexId(1));
        }
        for vertex in &mut source.vertices[3..] {
            vertex.trace = Some(TraceVertexId(2));
        }
        source.boundary_edges = vec![
            BoundaryEdge {
                vertices: [0, 1],
                label: BoundaryLabel::Curve {
                    curve: CurveId(7),
                    span: CurveSpanId(70),
                    side: CurveTraceSide::Left,
                    separated: true,
                },
                parameters: [0.0, 1.0],
            },
            BoundaryEdge {
                vertices: [4, 3],
                label: BoundaryLabel::Curve {
                    curve: CurveId(7),
                    span: CurveSpanId(70),
                    side: CurveTraceSide::Right,
                    separated: true,
                },
                parameters: [1.0, 0.0],
            },
        ];
        let mut target = source.clone();
        target.geometry_revision = 8;
        let source_operator =
            QuadraticWaveOperator::assemble(&source, WaveCoefficients::default()).unwrap();
        let target_operator =
            QuadraticWaveOperator::assemble(&target, WaveCoefficients::default()).unwrap();
        let map = QuadraticTransferMap::build(&source, &source_operator, &target, &target_operator)
            .unwrap();
        let mut values = vec![0.0; source_operator.degrees_of_freedom()];
        for node in source_operator.element_nodes()[0] {
            values[node as usize] = 1.0;
        }
        for node in source_operator.element_nodes()[1] {
            values[node as usize] = 9.0;
        }
        let transferred = map.interpolate(&values, -1.0).unwrap();
        for node in [
            target_operator.element_nodes()[0][0],
            target_operator.element_nodes()[0][1],
            target_operator.element_nodes()[0][3],
        ] {
            assert!((transferred[node as usize] - 1.0).abs() < 1.0e-12);
        }
        for node in [
            target_operator.element_nodes()[1][0],
            target_operator.element_nodes()[1][1],
            target_operator.element_nodes()[1][3],
        ] {
            assert!((transferred[node as usize] - 9.0).abs() < 1.0e-12);
        }
    }

    #[test]
    fn topology_transfer_survives_transmitting_face_split_and_merge() {
        let (unsplit, unsplit_plan) = topology_mesh(&TopologyGeometry::default(), 30);
        let (split, split_plan) = topology_mesh(&transmitting_divider(), 31);
        assert_eq!(
            split
                .triangles
                .iter()
                .map(|triangle| triangle.region)
                .collect::<BTreeSet<_>>()
                .len(),
            2
        );
        let materials = [Material::default_medium()];
        let regions = [
            Region {
                id: RegionId(1),
                material: MaterialId(1),
                frame: crate::MaterialFrame::world(),
            },
            Region {
                id: RegionId(2),
                material: MaterialId(1),
                frame: crate::MaterialFrame::world(),
            },
        ];
        let model = TopologyWaveModel {
            physics: crate::PhysicsModel::Mechanical,
            materials: &materials,
            regions: &regions,
            outer_boundaries: &OuterBoundaryConditions::default(),
        };
        let unsplit_operator =
            QuadraticWaveOperator::assemble_topology(&unsplit, &unsplit_plan, model).unwrap();
        let split_operator =
            QuadraticWaveOperator::assemble_topology(&split, &split_plan, model).unwrap();
        let polynomial = |point: Point2| {
            0.3 - 0.7 * point.x + 1.1 * point.y + 0.4 * point.x * point.x - 0.2 * point.x * point.y
                + 0.6 * point.y * point.y
        };

        let unsplit_values = unsplit_operator
            .node_points()
            .iter()
            .map(|point| polynomial(*point))
            .collect::<Vec<_>>();
        let split_map =
            QuadraticTransferMap::build(&unsplit, &unsplit_operator, &split, &split_operator)
                .unwrap();
        assert_eq!(split_map.exposed_nodes(), 0);
        let split_values = split_map.interpolate(&unsplit_values, -10.0).unwrap();
        for (point, value) in split_operator.node_points().iter().zip(&split_values) {
            assert!((*value - polynomial(*point)).abs() < 3.0e-12);
        }

        let merge_map =
            QuadraticTransferMap::build(&split, &split_operator, &unsplit, &unsplit_operator)
                .unwrap();
        assert_eq!(merge_map.exposed_nodes(), 0);
        let merged_values = merge_map.interpolate(&split_values, -10.0).unwrap();
        for (point, value) in unsplit_operator.node_points().iter().zip(merged_values) {
            assert!((value - polynomial(*point)).abs() < 3.0e-12);
        }
    }

    /// Reported by the 2026-10-20 review (MESH-1). Trace IDs are local to a
    /// snapshot and handed out in sequence, so a hole drawn first that gains
    /// outline vertices shifts every ID on a wall drawn after it. Narrowed by
    /// the target's ID, a node's candidates on the wall named another vertex
    /// on its side; non-empty but missing the node, they went straight to the
    /// unrestricted search, skipping the curve-side set, and one of the wall's
    /// two coincident copies took the other side's value. Here the field is
    /// one inside the wall and zero outside, which every node has to keep.
    fn reflecting_loop(id: u64, span_base: u64, center: Point2, radius: f64) -> TopologyCurve {
        let spline = PeriodicCubicSpline::rounded(center, radius);
        let spans = (0..spline.intervals().len())
            .map(|index| CurveSpan {
                id: CurveSpanId(span_base + index as u64),
                behavior: SpanBehavior::REFLECTING,
            })
            .collect();
        TopologyCurve::new(CurveId(id), CurveSpline::Closed(spline), spans).unwrap()
    }

    /// `curves` in the default domain, each face its own region of the
    /// default medium, meshed with `options` from atoms merged to them, as
    /// the app's runtime meshes a scene.
    fn walled_mesh(
        curves: Vec<TopologyCurve>,
        options: MeshingOptions,
        revision: u64,
    ) -> (TriMesh, QuadraticWaveOperator) {
        let geometry = TopologyGeometry {
            curves,
            ..TopologyGeometry::default()
        };
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
        let plan = TopologyMeshPlan::new(&topology, &assignments)
            .unwrap()
            .coarsened(&topology, AtomCoarsening::from_meshing(options))
            .unwrap();
        let mesh = mesh_topology_plan(&plan, revision, options).unwrap();
        let regions = (1..=topology.faces.len() as u64)
            .map(|id| Region {
                id: RegionId(id),
                material: MaterialId(1),
                frame: crate::MaterialFrame::world(),
            })
            .collect::<Vec<_>>();
        let operator = QuadraticWaveOperator::assemble_topology(
            &mesh,
            &plan,
            TopologyWaveModel {
                physics: crate::PhysicsModel::Mechanical,
                materials: &[Material::default_medium()],
                regions: &regions,
                outer_boundaries: &OuterBoundaryConditions::default(),
            },
        )
        .unwrap();
        (mesh, operator)
    }

    /// One in the region holding `inside`, zero everywhere else, node by node
    /// from the element each copy belongs to.
    fn one_inside(mesh: &TriMesh, operator: &QuadraticWaveOperator, inside: Point2) -> Vec<f64> {
        let region = mesh
            .triangles
            .iter()
            .find(|triangle| {
                barycentric(
                    inside,
                    triangle.vertices.map(|index| mesh.vertices[index].point),
                )
                .is_some()
            })
            .expect("a triangle holds the point")
            .region;
        let mut values = vec![0.0; operator.degrees_of_freedom()];
        for (triangle, nodes) in mesh.triangles.iter().zip(operator.element_nodes()) {
            if triangle.region == region {
                for node in nodes {
                    values[*node as usize] = 1.0;
                }
            }
        }
        values
    }

    /// How many of `target`'s nodes the map from `source` gives a value
    /// other than [`one_inside`]'s own, and how many nodes there are.
    fn taken_across(
        (source, source_operator): (&TriMesh, &QuadraticWaveOperator),
        (target, target_operator): (&TriMesh, &QuadraticWaveOperator),
        inside: Point2,
    ) -> (usize, usize) {
        let map =
            QuadraticTransferMap::build(source, source_operator, target, target_operator).unwrap();
        let transferred = map
            .interpolate(&one_inside(source, source_operator, inside), 0.0)
            .unwrap();
        let expected = one_inside(target, target_operator, inside);
        let wrong = transferred
            .iter()
            .zip(&expected)
            .filter(|(actual, expected)| (*actual - *expected).abs() > 1.0e-9)
            .count();
        (wrong, expected.len())
    }

    #[test]
    fn a_renumbered_wall_trace_keeps_each_side_its_own_value() {
        let wall_center = Point2::new(0.4, 0.0);
        let mesh_with_hole = |hole_radius: f64, revision: u64| {
            walled_mesh(
                vec![
                    reflecting_loop(1, 10, Point2::new(-0.5, 0.0), hole_radius),
                    reflecting_loop(2, 30, wall_center, 0.35),
                ],
                MeshingOptions {
                    target_edge_length: 0.08,
                    ..MeshingOptions::default()
                },
                revision,
            )
        };
        let (source, source_operator) = mesh_with_hole(0.12, 1);
        let (target, target_operator) = mesh_with_hole(0.3, 2);
        let (wrong, nodes) = taken_across(
            (&source, &source_operator),
            (&target, &target_operator),
            wall_center,
        );
        assert_eq!(
            wrong, 0,
            "{wrong} of {nodes} nodes took the other side's value"
        );
    }

    /// IV-10 of the testing campaign: a curve is meshed as chords, each mesh
    /// its own, so a finer mesh's node on a convex wall lies up to a chord's
    /// sagitta outside the coarser mesh's elements on its side, and inside
    /// the other side's. Searched for among every triangle, an island's wall
    /// nodes took the field outside it when the mesh was made finer.
    #[test]
    fn a_wall_remeshed_finer_keeps_each_side_its_own_value() {
        let center = Point2::new(0.2, -0.1);
        // A tolerance loose enough that the edge, not the tolerance, sets
        // where each mesh samples the curve.
        let at = |edge: f64, revision: u64| {
            walled_mesh(
                vec![reflecting_loop(1, 10, center, 0.5)],
                MeshingOptions {
                    target_edge_length: edge,
                    curve_tolerance: 2.0e-3,
                    ..MeshingOptions::default()
                },
                revision,
            )
        };
        for (from, to) in [(0.07, 0.05), (0.05, 0.07)] {
            let (source, source_operator) = at(from, 1);
            let (target, target_operator) = at(to, 2);
            let (wrong, nodes) = taken_across(
                (&source, &source_operator),
                (&target, &target_operator),
                center,
            );
            assert_eq!(
                wrong, 0,
                "edge {from} to {to}: {wrong} of {nodes} nodes took the other side's value"
            );
        }
    }

    /// IV-12 of the testing campaign: ground a moved wall swept over takes
    /// nothing from the side it lay on before. Located by position alone,
    /// the nodes inside a loop moved by part of an element took the field
    /// outside it, and the jump the wall held stayed where the wall had been.
    #[test]
    fn a_moved_wall_keeps_each_side_its_own_value() {
        let at = |center: Point2, revision: u64| {
            walled_mesh(
                vec![reflecting_loop(1, 10, center, 0.5)],
                MeshingOptions {
                    target_edge_length: 0.07,
                    curve_tolerance: 2.0e-3,
                    ..MeshingOptions::default()
                },
                revision,
            )
        };
        let center = Point2::new(0.2, -0.1);
        let (source, source_operator) = at(center, 1);
        let inside = one_inside(&source, &source_operator, center);
        for shift in [0.02, 0.05, 0.15] {
            let moved = center + Point2::new(shift, 0.0);
            let (target, target_operator) = at(moved, 2);
            let map =
                QuadraticTransferMap::build(&source, &source_operator, &target, &target_operator)
                    .unwrap();
            let transferred = map.interpolate(&inside, -1.0).unwrap();
            let expected = one_inside(&target, &target_operator, moved);
            let unsampled = (0..expected.len())
                .filter(|node| map.samples()[*node].is_none())
                .map(|node| node as u32)
                .collect::<Vec<_>>();
            let wrong = transferred
                .iter()
                .zip(&expected)
                .filter(|(actual, expected)| {
                    **actual != -1.0 && (*actual - *expected).abs() > 1.0e-9
                })
                .count();
            assert_eq!(
                wrong,
                0,
                "moved {shift}: {wrong} of {} nodes took the other side's value",
                expected.len()
            );
            assert!(!unsampled.is_empty(), "moved {shift}: nothing was swept");
            assert_eq!(
                map.swept_nodes(),
                unsampled,
                "moved {shift}: the nodes left without a sample are not the swept ones"
            );
        }
    }

    /// MESH-13 of the 2026-10-20 review: a face that becomes active behind a
    /// separated wall takes nothing from the other side of it. A hole made a
    /// walled subdomain, the reverse of what an undo hands off after a
    /// subdomain is made a hole: the new face's wall nodes copied the outer
    /// field, exactly by point or from the outer triangle holding them, while
    /// its interior stayed zero, a ring that radiated into the face.
    #[test]
    fn a_face_new_behind_a_wall_takes_nothing_from_its_other_side() {
        let center = Point2::new(0.3, 0.0);
        let geometry = TopologyGeometry {
            curves: vec![
                TopologyCurve::new(
                    crate::CurveId(1),
                    CurveSpline::Closed(PeriodicCubicSpline::rounded(center, 0.35)),
                    (0..8)
                        .map(|index| CurveSpan {
                            id: CurveSpanId(10 + index),
                            behavior: SpanBehavior::REFLECTING,
                        })
                        .collect(),
                )
                .unwrap(),
            ],
            ..TopologyGeometry::default()
        };
        let mesh_with = |inner_active: bool, revision: u64| {
            let topology = compile_topology(&geometry, revision).unwrap();
            let inner = topology.face_at(center).unwrap();
            let assignments = topology
                .faces
                .iter()
                .map(|face| FaceRegionAssignment {
                    face: face.id,
                    region: if face.id == inner {
                        inner_active.then_some(RegionId(2))
                    } else {
                        Some(RegionId(1))
                    },
                })
                .collect::<Vec<_>>();
            let plan = TopologyMeshPlan::new(&topology, &assignments).unwrap();
            let mesh = mesh_topology_plan(
                &plan,
                revision,
                MeshingOptions {
                    target_edge_length: 0.08,
                    ..MeshingOptions::default()
                },
            )
            .unwrap();
            let regions = [RegionId(1), RegionId(2)].map(|id| Region {
                id,
                material: MaterialId(1),
                frame: crate::MaterialFrame::world(),
            });
            let operator = QuadraticWaveOperator::assemble_topology(
                &mesh,
                &plan,
                TopologyWaveModel {
                    physics: crate::PhysicsModel::Mechanical,
                    materials: &[Material::default_medium()],
                    regions: &regions,
                    outer_boundaries: &OuterBoundaryConditions::default(),
                },
            )
            .unwrap();
            (mesh, operator)
        };
        let (source, source_operator) = mesh_with(false, 1);
        let (target, target_operator) = mesh_with(true, 2);
        let map = QuadraticTransferMap::build(&source, &source_operator, &target, &target_operator)
            .unwrap();
        // One over the outer face, the only one the source has; nothing for
        // the new face, which reads the exposed value.
        let transferred = map
            .interpolate(&vec![1.0; source_operator.degrees_of_freedom()], 0.0)
            .unwrap();
        let mut taken = 0;
        let mut outer_lost = 0;
        for (triangle, nodes) in target.triangles.iter().zip(target_operator.element_nodes()) {
            for node in nodes {
                let value = transferred[*node as usize];
                match triangle.region {
                    RegionId(2) if value != 0.0 => taken += 1,
                    RegionId(1) if (value - 1.0).abs() > 1.0e-12 => outer_lost += 1,
                    _ => {}
                }
            }
        }
        assert_eq!(taken, 0, "nodes of the new face took the outer field");
        assert_eq!(
            outer_lost, 0,
            "nodes of the outer face lost their own field"
        );
    }

    #[test]
    fn topology_transfer_preserves_separated_junction_sectors() {
        let (source, plan) = topology_mesh(&separated_t_junction(), 40);
        assert!(
            plan.junctions
                .iter()
                .any(|junction| junction.faces.len() == 3)
        );
        let materials = [Material::default_medium()];
        let regions = [Region {
            id: RegionId(1),
            material: MaterialId(1),
            frame: crate::MaterialFrame::world(),
        }];
        let operator = QuadraticWaveOperator::assemble_topology(
            &source,
            &plan,
            TopologyWaveModel {
                physics: crate::PhysicsModel::Mechanical,
                materials: &materials,
                regions: &regions,
                outer_boundaries: &OuterBoundaryConditions::default(),
            },
        )
        .unwrap();
        let map = QuadraticTransferMap::build(&source, &operator, &source, &operator).unwrap();
        assert_eq!(map.exposed_nodes(), 0);
        let values = (0..operator.degrees_of_freedom())
            .map(|index| (index as f64 * 0.731).sin() + index as f64 * 0.01)
            .collect::<Vec<_>>();
        let transferred = map.interpolate(&values, -10.0).unwrap();
        for (actual, expected) in transferred.iter().zip(values) {
            assert!((actual - expected).abs() < 3.0e-12);
        }
    }

    #[test]
    fn quadratic_transfer_rejects_mismatched_operator_revisions() {
        let source = mesh(3, &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], &[[0, 1, 2]]);
        let operator =
            QuadraticWaveOperator::assemble(&source, WaveCoefficients::default()).unwrap();
        let mut stale = source.clone();
        stale.geometry_revision += 1;
        assert_eq!(
            QuadraticTransferMap::build(&stale, &operator, &source, &operator),
            Err(TransferError::InvalidSource)
        );
        assert_eq!(
            QuadraticTransferMap::build(&source, &operator, &stale, &operator),
            Err(TransferError::InvalidTarget)
        );
        let mut stale_mesh = source.clone();
        stale_mesh.mesh_revision += 1;
        assert_eq!(
            QuadraticTransferMap::build(&stale_mesh, &operator, &source, &operator),
            Err(TransferError::InvalidSource)
        );
        assert_eq!(
            QuadraticTransferMap::build(&source, &operator, &stale_mesh, &operator),
            Err(TransferError::InvalidTarget)
        );
    }

    #[test]
    fn newly_exposed_vertices_use_the_requested_initial_value() {
        let source = mesh(1, &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], &[[0, 1, 2]]);
        let target = mesh(2, &[[0.2, 0.2], [0.8, 0.8], [-0.1, 0.1]], &[[0, 1, 2]]);
        let map = TransferMap::build(&source, &target).unwrap();
        assert_eq!(map.exposed_vertices(), 2);
        assert_eq!(
            map.interpolate(&[1.0, 2.0, 3.0], -7.0).unwrap()[1..],
            [-7.0, -7.0]
        );
    }

    #[test]
    fn malformed_inputs_are_rejected_without_partial_values() {
        let mut source = mesh(1, &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], &[[0, 1, 2]]);
        let target = source.clone();
        source.triangles[0].vertices[2] = 9;
        assert_eq!(
            TransferMap::build(&source, &target),
            Err(TransferError::InvalidSource)
        );
        let map = TransferMap::build(&target, &target).unwrap();
        assert!(matches!(
            map.interpolate(&[1.0], 0.0),
            Err(TransferError::SizeMismatch { .. })
        ));
        assert_eq!(
            map.interpolate(&[1.0, f64::NAN, 3.0], 0.0),
            Err(TransferError::NonFiniteValues)
        );
    }

    #[test]
    fn changing_timestep_preserves_the_reconstructed_current_velocity() {
        let current = 1.7;
        let velocity = -0.45;
        let undamped_acceleration = 0.8;
        let damping = 0.3;
        let old_dt = 0.04;
        let old_previous =
            centered_previous(current, velocity, undamped_acceleration, damping, old_dt).unwrap();
        let recovered = centered_velocity(
            old_previous,
            current,
            undamped_acceleration,
            damping,
            old_dt,
        )
        .unwrap();
        assert!((recovered - velocity).abs() < 1.0e-14);

        let new_dt = 0.013;
        let new_previous =
            centered_previous(current, recovered, undamped_acceleration, damping, new_dt).unwrap();
        let recovered_again = centered_velocity(
            new_previous,
            current,
            undamped_acceleration,
            damping,
            new_dt,
        )
        .unwrap();
        assert!((recovered_again - velocity).abs() < 1.0e-13);
        assert_ne!(new_previous, old_previous);
    }
}
