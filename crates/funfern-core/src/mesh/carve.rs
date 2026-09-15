//! Incremental mesh repair by carving.
//!
//! A topology mesh plan is a set of straight boundary atoms with the faces
//! beside them. When the plan changes, only the atoms that differ matter to
//! the mesh. The triangles they touched are removed, kept triangles whose face
//! changed are relabeled, the changed atoms are expanded into chains again,
//! and every face's cavity between the surviving mesh and the new chains is
//! bridged, clipped, legalized and refined by the same steps a full rebuild
//! uses. Nothing outside the carved band changes, the imported triangles are
//! frozen while the band is filled, and there is no cap on how far a boundary
//! may move: a long drag simply carves a longer band.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::topology_plan::{expand_step_chain, split_slit_runs, topology_label};
use super::{
    BoundaryEdge, BoundaryLabel, CurveTraceSide, MeshBuilder, MeshError, MeshTriangle,
    MeshingOptions, PlannedBoundaryEdge, PlannedBoundarySource, PlannedFaceStep, PolygonLocation,
    SegmentRelation, TopologyMeshPlan, TopologyMeshingJob, TriMesh, TriangulationDomain, edge_key,
    locate_in_polygon, on_segment, point_in_triangle, segment_relation, valid_options,
};
use crate::{
    CurveId, CurveSpanId, OuterSide, Point2, RegionId, SpanBehavior, TopologySnapshot,
    TraceVertexId,
};

/// What a carve did, for the handoff record and the Performance panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CarveReport {
    /// Triangles imported unchanged from the previous mesh.
    pub kept_triangles: usize,
    /// Previous triangles the carve removed.
    pub removed_triangles: usize,
    /// Kept triangles whose face now belongs to another region.
    pub relabeled_triangles: usize,
    /// Triangles the cavities were filled with.
    pub inserted_triangles: usize,
    /// Atoms that differ between the plans, old and new together.
    pub changed_atoms: usize,
    /// Separated curves rebuilt whole because they or a neighbour changed.
    pub rebuilt_curves: usize,
    /// Cavity polygons that were triangulated.
    pub cavities: usize,
}

/// Everything about an atom the mesh's boundary can observe: its label and
/// its geometry. Trace ids are left out because every compile reissues them,
/// and faces and regions are left out because they belong to the triangles,
/// which are relabeled from the new topology; so a face that only changes
/// its material or its id keeps every atom and carves nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct AtomKey {
    source: PlannedBoundarySource,
    separated: bool,
    parameter: [u64; 2],
    points: [[u64; 2]; 2],
}

fn atom_key(atom: &PlannedBoundaryEdge) -> AtomKey {
    AtomKey {
        source: atom.source,
        separated: matches!(atom.behavior, Some(SpanBehavior::Separated { .. })),
        parameter: atom.parameter.map(f64::to_bits),
        points: atom
            .points
            .map(|point| [point.x.to_bits(), point.y.to_bits()]),
    }
}

fn separated_curve(atom: &PlannedBoundaryEdge) -> Option<CurveId> {
    match (atom.source, atom.behavior) {
        (PlannedBoundarySource::Curve { curve, .. }, Some(SpanBehavior::Separated { .. })) => {
            Some(curve)
        }
        _ => None,
    }
}

fn atom_curve(atom: &PlannedBoundaryEdge) -> Option<CurveId> {
    match atom.source {
        PlannedBoundarySource::Curve { curve, .. } => Some(curve),
        PlannedBoundarySource::Outer(_) => None,
    }
}

/// An orderable stand-in for the topology boundary labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum LabelKey {
    Outer(OuterSide),
    Curve {
        curve: CurveId,
        span: CurveSpanId,
        side: CurveTraceSide,
        separated: bool,
    },
}

fn label_key(label: BoundaryLabel) -> Result<LabelKey, MeshError> {
    match label {
        BoundaryLabel::Outer(side) => Ok(LabelKey::Outer(side)),
        BoundaryLabel::Curve {
            curve,
            span,
            side,
            separated,
        } => Ok(LabelKey::Curve {
            curve,
            span,
            side,
            separated,
        }),
        _ => Err(MeshError::Topology(
            "carving needs a mesh built from a topology plan",
        )),
    }
}

fn parameter_close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 32.0 * f64::EPSILON * (1.0 + a.abs().max(b.abs()))
}

fn point_bits(point: Point2) -> [u64; 2] {
    [point.x.to_bits(), point.y.to_bits()]
}

/// The vertex a boundary edge of label `key` has at `parameter`, exactly or
/// within a few ulps, among the recorded edge endpoints.
fn endpoint_vertex(
    endpoints: &BTreeMap<LabelKey, Vec<(f64, usize)>>,
    key: LabelKey,
    parameter: f64,
) -> Option<usize> {
    let candidates = endpoints.get(&key)?;
    candidates
        .iter()
        .find(|(candidate, _)| *candidate == parameter)
        .or_else(|| {
            candidates
                .iter()
                .find(|(candidate, _)| parameter_close(*candidate, parameter))
        })
        .map(|(_, vertex)| *vertex)
}

fn signed_area(points: impl Iterator<Item = Point2>) -> f64 {
    let points = points.collect::<Vec<_>>();
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| a.cross(*b))
        .sum::<f64>()
        * 0.5
}

/// Whether a new boundary segment passes through a triangle, so the triangle
/// cannot stay. Running along a constrained edge does not count: the new
/// boundary then coincides with an old one and the triangle lies beside it.
/// A triangle corner strictly inside the segment does count, so no kept
/// vertex ends up on a new chain.
fn segment_crosses_triangle(
    a: Point2,
    b: Point2,
    triangle: [Point2; 3],
    constrained: [bool; 3],
) -> bool {
    if point_in_triangle(a, triangle) == PolygonLocation::Inside
        || point_in_triangle(b, triangle) == PolygonLocation::Inside
    {
        return true;
    }
    (0..3).any(|corner| {
        let (p, q) = (triangle[corner], triangle[(corner + 1) % 3]);
        match segment_relation(a, b, p, q) {
            SegmentRelation::ProperIntersection => true,
            SegmentRelation::Overlapping => !constrained[corner],
            SegmentRelation::Touching => p != a && p != b && on_segment(p, a, b),
            SegmentRelation::Disjoint => false,
        }
    })
}

/// Uniform bins over a set of triangles, for segment sweeps and point lookups.
struct TriangleGrid {
    minimum: Point2,
    cell: f64,
    dimension: usize,
    cells: Vec<Vec<u32>>,
    triangles: Vec<[Point2; 3]>,
    /// Per triangle, whether the edge after each corner is a boundary edge.
    constrained: Vec<[bool; 3]>,
}

impl TriangleGrid {
    fn new(triangles: Vec<[Point2; 3]>, constrained: Vec<[bool; 3]>) -> Self {
        let mut minimum = Point2::new(f64::INFINITY, f64::INFINITY);
        let mut maximum = Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
        for point in triangles.iter().flatten() {
            minimum.x = minimum.x.min(point.x);
            minimum.y = minimum.y.min(point.y);
            maximum.x = maximum.x.max(point.x);
            maximum.y = maximum.y.max(point.y);
        }
        if triangles.is_empty() {
            minimum = Point2::default();
            maximum = Point2::default();
        }
        let extent = (maximum.x - minimum.x)
            .max(maximum.y - minimum.y)
            .max(1.0e-12);
        let dimension = ((triangles.len() as f64).sqrt().ceil() as usize).clamp(1, 512);
        let cell = extent / dimension as f64;
        let mut grid = Self {
            minimum,
            cell,
            dimension,
            cells: vec![vec![]; dimension * dimension],
            triangles: vec![],
            constrained,
        };
        for (index, triangle) in triangles.iter().enumerate() {
            let (low, high) = bounds(triangle.iter().copied());
            let (x0, x1, y0, y1) = grid.cell_range(low, high);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    grid.cells[y * dimension + x].push(index as u32);
                }
            }
        }
        grid.triangles = triangles;
        grid
    }

    fn cell_range(&self, low: Point2, high: Point2) -> (usize, usize, usize, usize) {
        let last = self.dimension as f64 - 1.0;
        let index =
            |value: f64, origin: f64| ((value - origin) / self.cell).floor().clamp(0.0, last);
        (
            index(low.x, self.minimum.x) as usize,
            index(high.x, self.minimum.x) as usize,
            index(low.y, self.minimum.y) as usize,
            index(high.y, self.minimum.y) as usize,
        )
    }

    /// Every triangle a segment touches, crosses or ends inside.
    fn crossing(&self, a: Point2, b: Point2) -> BTreeSet<usize> {
        let (low, high) = bounds([a, b].into_iter());
        let (x0, x1, y0, y1) = self.cell_range(low, high);
        let mut found = BTreeSet::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                for &index in &self.cells[y * self.dimension + x] {
                    let index = index as usize;
                    if !found.contains(&index)
                        && segment_crosses_triangle(
                            a,
                            b,
                            self.triangles[index],
                            self.constrained[index],
                        )
                    {
                        found.insert(index);
                    }
                }
            }
        }
        found
    }

    /// A triangle containing the point, preferring one that holds it strictly.
    fn containing(&self, point: Point2) -> Option<usize> {
        let (x0, x1, y0, y1) = self.cell_range(point, point);
        let mut boundary = None;
        for y in y0..=y1 {
            for x in x0..=x1 {
                for &index in &self.cells[y * self.dimension + x] {
                    let index = index as usize;
                    match point_in_triangle(point, self.triangles[index]) {
                        PolygonLocation::Inside => return Some(index),
                        PolygonLocation::Boundary => boundary = boundary.or(Some(index)),
                        PolygonLocation::Outside => {}
                    }
                }
            }
        }
        boundary
    }
}

/// The triangles on both sides of the edge `u`–`v`, given each vertex's
/// incident triangles.
fn across(incident: &[Vec<usize>], u: usize, v: usize) -> impl Iterator<Item = usize> + '_ {
    incident[u]
        .iter()
        .copied()
        .filter(move |triangle| incident[v].contains(triangle))
}

fn bounds(points: impl Iterator<Item = Point2>) -> (Point2, Point2) {
    points.fold(
        (
            Point2::new(f64::INFINITY, f64::INFINITY),
            Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY),
        ),
        |(low, high), point| {
            (
                Point2::new(low.x.min(point.x), low.y.min(point.y)),
                Point2::new(high.x.max(point.x), high.y.max(point.y)),
            )
        },
    )
}

/// Edge-length targets for refilling a carved band: the sizes adaptation
/// requested for the triangles the carve removed, looked up by point. The
/// sizes those triangles actually had are deliberately not consulted. A
/// refill has to meet frozen rim vertices and the boundary's own chords with
/// elements smaller than the target, and repairing the same band again would
/// read that smallness back as a target: over repeated drags of one curve the
/// mesh only ever got finer. A request is copied, never re-measured, so it is
/// bounded by what adaptation asked for and carries nothing the refill made.
pub(super) struct LocalSizeField {
    grid: TriangleGrid,
    /// Per removed triangle, the coarser of the size adaptation requested for
    /// it and the size it had: the request is a ceiling on refinement and the
    /// existing density is the floor, so a refill never refines the cavity
    /// finer than the frozen rim that bounds it.
    targets: Vec<f64>,
    /// Per removed triangle, the request itself, inherited by the refill's
    /// triangles so the next repair still knows what adaptation asked for.
    requests: Vec<f64>,
}

impl LocalSizeField {
    fn new(triangles: Vec<[Point2; 3]>, requests: Vec<f64>) -> Self {
        let targets = triangles
            .iter()
            .zip(&requests)
            .map(|(triangle, request)| {
                let longest = (0..3)
                    .map(|corner| (triangle[(corner + 1) % 3] - triangle[corner]).norm())
                    .fold(0.0, f64::max);
                request.max(longest)
            })
            .collect();
        let constrained = vec![[false; 3]; triangles.len()];
        Self {
            grid: TriangleGrid::new(triangles, constrained),
            targets,
            requests,
        }
    }

    /// The edge length the refill works towards at `point`.
    pub(super) fn size_at(&self, point: Point2) -> Option<f64> {
        self.grid.containing(point).map(|index| self.targets[index])
    }

    /// The size adaptation requested for the removed triangle under `point`.
    fn request_at(&self, point: Point2) -> Option<f64> {
        self.grid
            .containing(point)
            .map(|index| self.requests[index])
    }
}

enum CarvePhase {
    Index(usize),
    Select,
    ImportVertices(usize),
    ImportTriangles(usize),
    ImportEdges(usize),
    Traces,
    Expand,
    Cycles,
    Mesh(Box<TopologyMeshingJob>),
    Compact(Option<TriMesh>),
    Done,
}

/// Cooperative repair of a topology mesh for a changed plan.
pub struct TopologyCarveJob {
    previous: Arc<TriMesh>,
    old_atoms: Vec<PlannedBoundaryEdge>,
    plan: TopologyMeshPlan,
    topology: Arc<TopologySnapshot>,
    mesh_revision: u64,
    options: MeshingOptions,
    /// Whether the refill honours the sizes adaptation requested for the
    /// removed triangles; otherwise the band comes back at the meshing target.
    preserve_adaptation: bool,
    size_field: Option<Arc<LocalSizeField>>,
    kept_keys: BTreeSet<AtomKey>,
    rebuilt_curves: BTreeSet<CurveId>,
    /// Junction points whose sector structure changed; every atom ending at
    /// one of them is rebuilt so the sectors come out of the new plan.
    rebuilt_points: BTreeSet<[u64; 2]>,
    changed_new_keys: BTreeSet<AtomKey>,
    changed_edge: Vec<bool>,
    deleted: Vec<bool>,
    relabel: Vec<Option<RegionId>>,
    kept_order: Vec<usize>,
    /// Previous vertex index to builder index; `usize::MAX` for vertices only
    /// the removed band used, which are not imported at all so nothing can
    /// snap to them.
    vertex_map: Vec<usize>,
    /// Triangles of the previous mesh around each of its vertices.
    incident: Vec<Vec<usize>>,
    areas: Vec<f64>,
    /// The smallest angle in the previous mesh; the refill may not do worse
    /// than half of it.
    previous_worst_angle: f64,
    grid: Option<TriangleGrid>,
    old_boundary_edges: BTreeMap<(usize, usize), usize>,
    builder: Option<MeshBuilder>,
    trace_vertices: BTreeMap<TraceVertexId, usize>,
    cavity_edges: Vec<(RegionId, usize, usize)>,
    slit_runs: Vec<(RegionId, Vec<PlannedFaceStep>)>,
    report: CarveReport,
    phase: CarvePhase,
}

impl TopologyCarveJob {
    /// Prepares a repair of `previous`, a mesh of `previous_plan`, for `plan`.
    /// `topology` is the compiled arrangement the new plan was built from; it
    /// decides which face a kept triangle now lies in. With
    /// `preserve_adaptation` the refill follows the sizes adaptation requested
    /// for the removed triangles, so an adapted band stays adapted.
    pub fn new(
        previous: Arc<TriMesh>,
        previous_plan: &TopologyMeshPlan,
        plan: TopologyMeshPlan,
        topology: Arc<TopologySnapshot>,
        mesh_revision: u64,
        options: MeshingOptions,
        preserve_adaptation: bool,
    ) -> Self {
        let old_keys = previous_plan
            .boundaries
            .iter()
            .map(atom_key)
            .collect::<BTreeSet<_>>();
        let new_keys = plan
            .boundaries
            .iter()
            .map(atom_key)
            .collect::<BTreeSet<_>>();
        let kept_keys = old_keys
            .intersection(&new_keys)
            .copied()
            .collect::<BTreeSet<_>>();
        let rebuilt_curves = previous_plan
            .boundaries
            .iter()
            .chain(&plan.boundaries)
            .filter(|atom| !kept_keys.contains(&atom_key(atom)))
            .filter_map(separated_curve)
            .collect();
        Self {
            previous,
            old_atoms: previous_plan.boundaries.clone(),
            plan,
            topology,
            mesh_revision,
            options,
            preserve_adaptation,
            size_field: None,
            kept_keys,
            rebuilt_curves,
            rebuilt_points: BTreeSet::new(),
            changed_new_keys: BTreeSet::new(),
            changed_edge: vec![],
            deleted: vec![],
            relabel: vec![],
            kept_order: vec![],
            vertex_map: vec![],
            incident: vec![],
            areas: vec![],
            previous_worst_angle: f64::INFINITY,
            grid: None,
            old_boundary_edges: BTreeMap::new(),
            builder: None,
            trace_vertices: BTreeMap::new(),
            cavity_edges: vec![],
            slit_runs: vec![],
            report: CarveReport::default(),
            phase: CarvePhase::Index(0),
        }
    }

    pub fn phase(&self) -> &'static str {
        match &self.phase {
            CarvePhase::Index(_) => "Indexing the previous mesh",
            CarvePhase::Select => "Selecting the changed band",
            CarvePhase::ImportVertices(_)
            | CarvePhase::ImportTriangles(_)
            | CarvePhase::ImportEdges(_) => "Importing the kept mesh",
            CarvePhase::Traces => "Pairing boundary traces",
            CarvePhase::Expand => "Expanding changed boundaries",
            CarvePhase::Cycles => "Tracing cavities",
            CarvePhase::Mesh(job) => job.phase(),
            CarvePhase::Compact(_) => "Compacting the mesh",
            CarvePhase::Done => "Finished",
        }
    }

    pub fn report(&self) -> CarveReport {
        self.report
    }

    pub fn advance(&mut self, budget: usize) -> Option<Result<TriMesh, MeshError>> {
        for _ in 0..budget {
            if matches!(self.phase, CarvePhase::Done) {
                return None;
            }
            match self.step() {
                Ok(None) => {}
                Ok(Some(mesh)) => {
                    self.phase = CarvePhase::Done;
                    return Some(Ok(mesh));
                }
                Err(error) => {
                    self.phase = CarvePhase::Done;
                    return Some(Err(error));
                }
            }
        }
        None
    }

    fn step(&mut self) -> Result<Option<TriMesh>, MeshError> {
        let phase = std::mem::replace(&mut self.phase, CarvePhase::Done);
        self.phase = match phase {
            CarvePhase::Index(index) => {
                self.index(index)?;
                if index == self.previous.triangles.len() {
                    CarvePhase::Select
                } else {
                    CarvePhase::Index(index + 1)
                }
            }
            CarvePhase::Select => {
                self.select()?;
                CarvePhase::ImportVertices(0)
            }
            CarvePhase::ImportVertices(index) => {
                if index == 0 {
                    // The mesher's caps bound what a repair adds, not what it
                    // inherits: an adapted mesh may already exceed them.
                    let mut options = self.options;
                    options.max_vertices = options
                        .max_vertices
                        .saturating_add(self.previous.vertices.len());
                    options.max_triangles = options
                        .max_triangles
                        .saturating_add(self.previous.triangles.len());
                    let mut builder = MeshBuilder::new(options, self.plan.domain);
                    for atom in &self.plan.boundaries {
                        let label = topology_label(*atom);
                        if let Some((_, regions)) = builder
                            .boundary_regions
                            .iter_mut()
                            .find(|(candidate, _)| *candidate == label)
                        {
                            regions.insert(atom.region);
                        } else {
                            builder
                                .boundary_regions
                                .push((label, BTreeSet::from([atom.region])));
                        }
                    }
                    self.builder = Some(builder);
                }
                if index == self.previous.vertices.len() {
                    let builder = self.builder.as_mut().unwrap();
                    builder.frozen_vertices = builder.vertices.len();
                    CarvePhase::ImportTriangles(0)
                } else {
                    if self.vertex_map[index] != usize::MAX {
                        let vertex = self.previous.vertices[index];
                        let builder = self.builder.as_mut().unwrap();
                        // Traces are snapshot-local and are reassigned from
                        // the new plan; old ids would mislead slit recovery.
                        self.vertex_map[index] =
                            builder.add_vertex(vertex.point, vertex.boundary)?;
                    }
                    CarvePhase::ImportVertices(index + 1)
                }
            }
            CarvePhase::ImportTriangles(index) => {
                let builder = self.builder.as_mut().unwrap();
                if index == self.kept_order.len() {
                    builder.dirty_edges.clear();
                    builder.queued_edges.clear();
                    builder.bad_triangles.clear();
                    builder.scores.fill(None);
                    builder.frozen_triangles = builder.triangles.len();
                    self.report.kept_triangles = builder.triangles.len();
                    CarvePhase::ImportEdges(0)
                } else {
                    let old = self.kept_order[index];
                    let triangle = self.previous.triangles[old];
                    builder.push_triangle(MeshTriangle {
                        vertices: triangle.vertices.map(|vertex| self.vertex_map[vertex]),
                        region: self.relabel[old].unwrap_or(triangle.region),
                    })?;
                    CarvePhase::ImportTriangles(index + 1)
                }
            }
            CarvePhase::ImportEdges(index) => {
                if index == self.previous.boundary_edges.len() {
                    CarvePhase::Traces
                } else {
                    if !self.changed_edge[index] {
                        let mut edge = self.previous.boundary_edges[index];
                        edge.vertices = edge.vertices.map(|vertex| self.vertex_map[vertex]);
                        self.builder.as_mut().unwrap().add_boundary_edge(edge);
                    }
                    CarvePhase::ImportEdges(index + 1)
                }
            }
            CarvePhase::Traces => {
                self.pair_traces()?;
                CarvePhase::Expand
            }
            CarvePhase::Expand => {
                self.expand()?;
                CarvePhase::Cycles
            }
            CarvePhase::Cycles => {
                let domains = self.cavity_domains()?;
                self.report.cavities = domains.len();
                let mut builder = self.builder.take().unwrap();
                builder.domains = domains;
                if self.preserve_adaptation {
                    let (removed, requests): (Vec<_>, Vec<_>) = self
                        .deleted
                        .iter()
                        .enumerate()
                        .filter(|(_, deleted)| **deleted)
                        .filter_map(|(index, _)| {
                            let request = self.previous.requested_size(index)?;
                            let points = self.previous.triangles[index]
                                .vertices
                                .map(|vertex| self.previous.vertices[vertex].point);
                            Some((points, request))
                        })
                        .unzip();
                    if !removed.is_empty() {
                        self.size_field = Some(Arc::new(LocalSizeField::new(removed, requests)));
                    }
                }
                builder.size_field = self.size_field.clone();
                let frozen = builder.frozen_triangles;
                CarvePhase::Mesh(Box::new(TopologyMeshingJob::resume(
                    self.plan.clone(),
                    self.mesh_revision,
                    builder,
                    std::mem::take(&mut self.trace_vertices),
                    std::mem::take(&mut self.slit_runs),
                    frozen,
                )))
            }
            CarvePhase::Mesh(mut job) => match job.advance(1) {
                None => CarvePhase::Mesh(job),
                Some(result) => CarvePhase::Compact(Some(result?)),
            },
            CarvePhase::Compact(mesh) => {
                let mesh = self.compact(mesh.expect("compaction has a mesh"))?;
                self.report.inserted_triangles = mesh
                    .triangles
                    .len()
                    .saturating_sub(self.report.kept_triangles);
                verify_repair_quality(
                    &mesh,
                    self.report.kept_triangles,
                    self.previous_worst_angle,
                    self.options,
                )?;
                return Ok(Some(mesh));
            }
            CarvePhase::Done => CarvePhase::Done,
        };
        Ok(None)
    }

    /// Decides which triangles go: those touching a changed old atom, those a
    /// changed new atom crosses, and kept islands whose face is now excluded.
    /// Kept components that moved to another face are relabeled. Any
    /// separated curve a removed triangle touches is rebuilt whole, so slit
    /// recovery always sees complete runs; that can remove more triangles, so
    /// the selection repeats until it is stable.
    /// One triangle of the previous mesh: its incidence and area. The first
    /// call validates the inputs and indexes the boundary edges, the last one
    /// bins the triangles for segment sweeps.
    fn index(&mut self, index: usize) -> Result<(), MeshError> {
        let mesh = &self.previous;
        if index == 0 {
            if !valid_options(self.options) {
                return Err(MeshError::InvalidOptions);
            }
            if mesh.mesh_revision == self.mesh_revision {
                return Err(MeshError::Topology("carving needs a new mesh revision"));
            }
            self.incident = vec![vec![]; mesh.vertices.len()];
            self.areas = Vec::with_capacity(mesh.triangles.len());
            for (index, edge) in mesh.boundary_edges.iter().enumerate() {
                label_key(edge.label)?;
                if edge
                    .vertices
                    .iter()
                    .any(|vertex| *vertex >= mesh.vertices.len())
                {
                    return Err(MeshError::Topology(
                        "previous mesh has an invalid boundary edge",
                    ));
                }
                self.old_boundary_edges
                    .insert(edge_key(edge.vertices[0], edge.vertices[1]), index);
            }
        }
        if index == mesh.triangles.len() {
            self.grid = Some(TriangleGrid::new(
                mesh.triangles
                    .iter()
                    .map(|triangle| triangle.vertices.map(|vertex| mesh.vertices[vertex].point))
                    .collect(),
                mesh.triangles
                    .iter()
                    .map(|triangle| {
                        [0, 1, 2].map(|corner| {
                            self.old_boundary_edges.contains_key(&edge_key(
                                triangle.vertices[corner],
                                triangle.vertices[(corner + 1) % 3],
                            ))
                        })
                    })
                    .collect(),
            ));
            return Ok(());
        }
        let triangle = mesh.triangles[index];
        for vertex in triangle.vertices {
            if vertex >= mesh.vertices.len() {
                return Err(MeshError::Topology("previous mesh has an invalid triangle"));
            }
            self.incident[vertex].push(index);
        }
        let [a, b, c] = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
        self.areas.push((b - a).cross(c - a).abs());
        if let Some(quality) = mesh.triangle_quality(index) {
            self.previous_worst_angle =
                self.previous_worst_angle.min(quality.minimum_angle_degrees);
        }
        Ok(())
    }

    fn select(&mut self) -> Result<(), MeshError> {
        let mesh = self.previous.clone();
        let count = mesh.triangles.len();
        let incident = std::mem::take(&mut self.incident);
        let grid = self.grid.take().expect("indexing binned the triangles");
        let areas = std::mem::take(&mut self.areas);

        loop {
            let rebuilt = |atom: &PlannedBoundaryEdge| {
                atom_curve(atom).is_some_and(|curve| self.rebuilt_curves.contains(&curve))
            };
            let changed = |atom: &PlannedBoundaryEdge| {
                !self.kept_keys.contains(&atom_key(atom))
                    || rebuilt(atom)
                    || atom
                        .points
                        .iter()
                        .any(|point| self.rebuilt_points.contains(&point_bits(*point)))
            };
            let changed_old = self
                .old_atoms
                .iter()
                .filter(|atom| changed(atom))
                .copied()
                .collect::<Vec<_>>();
            let changed_new = self
                .plan
                .boundaries
                .iter()
                .filter(|atom| changed(atom))
                .copied()
                .collect::<Vec<_>>();

            let mut ranges = BTreeMap::<LabelKey, Vec<[f64; 2]>>::new();
            for atom in &changed_old {
                let [a, b] = atom.parameter;
                ranges
                    .entry(label_key(topology_label(*atom))?)
                    .or_default()
                    .push([a.min(b), a.max(b)]);
            }
            let mut changed_edge = vec![false; mesh.boundary_edges.len()];
            for (index, edge) in mesh.boundary_edges.iter().enumerate() {
                let middle = 0.5 * (edge.parameters[0] + edge.parameters[1]);
                changed_edge[index] = ranges.get(&label_key(edge.label)?).is_some_and(|ranges| {
                    ranges
                        .iter()
                        .any(|[low, high]| middle >= *low && middle <= *high)
                });
            }

            let mut deleted = vec![false; count];
            for (index, edge) in mesh.boundary_edges.iter().enumerate() {
                if changed_edge[index] {
                    for vertex in edge.vertices {
                        for &triangle in &incident[vertex] {
                            deleted[triangle] = true;
                        }
                    }
                }
            }
            for atom in &changed_new {
                for triangle in grid.crossing(atom.points[0], atom.points[1]) {
                    deleted[triangle] = true;
                }
            }
            // One more ring gives the cavity room: elements beside the rim
            // cannot be flipped or split across it, so the rim should not
            // hug the new boundary.
            let mut touched = vec![false; mesh.vertices.len()];
            for (index, triangle) in mesh.triangles.iter().enumerate() {
                if deleted[index] {
                    for vertex in triangle.vertices {
                        touched[vertex] = true;
                    }
                }
            }
            for (index, triangle) in mesh.triangles.iter().enumerate() {
                if triangle.vertices.iter().any(|vertex| touched[*vertex]) {
                    deleted[index] = true;
                }
            }

            // Kept triangles connected across unconstrained edges lie in one
            // face of the new topology: no changed boundary survives among
            // them and every new boundary crosses removed triangles only.
            let mut relabel = vec![None; count];
            let mut relabeled = 0;
            let mut visited = vec![false; count];
            for seed in 0..count {
                if deleted[seed] || visited[seed] {
                    continue;
                }
                let mut component = vec![];
                let mut pending = vec![seed];
                visited[seed] = true;
                while let Some(index) = pending.pop() {
                    component.push(index);
                    let triangle = mesh.triangles[index];
                    for corner in 0..3 {
                        let (u, v) = (
                            triangle.vertices[corner],
                            triangle.vertices[(corner + 1) % 3],
                        );
                        if self.old_boundary_edges.contains_key(&edge_key(u, v)) {
                            continue;
                        }
                        for neighbor in across(&incident, u, v) {
                            if neighbor != index && !deleted[neighbor] && !visited[neighbor] {
                                visited[neighbor] = true;
                                pending.push(neighbor);
                            }
                        }
                    }
                }
                // Tiny islands are not worth freezing inside a cavity.
                if component.len() <= 4 {
                    for index in component {
                        deleted[index] = true;
                    }
                    continue;
                }
                let representative = component
                    .iter()
                    .copied()
                    .max_by(|a, b| areas[*a].total_cmp(&areas[*b]))
                    .unwrap();
                let [a, b, c] = mesh.triangles[representative]
                    .vertices
                    .map(|vertex| mesh.vertices[vertex].point);
                let region = self
                    .topology
                    .face_at((a + b + c) / 3.0)
                    .and_then(|face| self.plan.region_for_face(face));
                match region {
                    None => {
                        for index in component {
                            deleted[index] = true;
                        }
                    }
                    Some(region) => {
                        for index in component {
                            if mesh.triangles[index].region != region {
                                relabel[index] = Some(region);
                                relabeled += 1;
                            }
                        }
                    }
                }
            }

            let mut grew = false;
            // Kept atoms meeting at one point must agree on its sectors: one
            // old vertex per new trace and one new trace per old vertex. A
            // junction that gained or lost sectors, for instance because a
            // divider attached to the wall turned reflecting, fails that, and
            // every atom meeting there is rebuilt from the new plan.
            let changed_new_keys = changed_new.iter().map(atom_key).collect::<BTreeSet<_>>();
            let mut endpoints = BTreeMap::<LabelKey, Vec<(f64, usize)>>::new();
            for (index, edge) in mesh.boundary_edges.iter().enumerate() {
                if changed_edge[index] {
                    continue;
                }
                let key = label_key(edge.label)?;
                for endpoint in 0..2 {
                    endpoints
                        .entry(key)
                        .or_default()
                        .push((edge.parameters[endpoint], edge.vertices[endpoint]));
                }
            }
            let mut claims = BTreeMap::<usize, TraceVertexId>::new();
            let mut owners = BTreeMap::<TraceVertexId, usize>::new();
            for atom in self
                .plan
                .boundaries
                .iter()
                .filter(|atom| !changed_new_keys.contains(&atom_key(atom)))
            {
                let key = label_key(topology_label(*atom))?;
                for endpoint in 0..2 {
                    let Some(vertex) = endpoint_vertex(&endpoints, key, atom.parameter[endpoint])
                    else {
                        continue;
                    };
                    let trace = atom.traces[endpoint];
                    let conflict = claims
                        .insert(vertex, trace)
                        .is_some_and(|existing| existing != trace)
                        || owners
                            .insert(trace, vertex)
                            .is_some_and(|existing| existing != vertex);
                    if conflict
                        && self
                            .rebuilt_points
                            .insert(point_bits(atom.points[endpoint]))
                    {
                        grew = true;
                    }
                }
            }
            for (index, edge) in mesh.boundary_edges.iter().enumerate() {
                if changed_edge[index] {
                    continue;
                }
                let BoundaryLabel::Curve {
                    curve,
                    separated: true,
                    ..
                } = edge.label
                else {
                    continue;
                };
                let touched = edge
                    .vertices
                    .iter()
                    .any(|vertex| incident[*vertex].iter().any(|triangle| deleted[*triangle]));
                if touched && self.rebuilt_curves.insert(curve) {
                    grew = true;
                }
            }
            if grew {
                continue;
            }

            self.report.changed_atoms = changed_old.len() + changed_new.len();
            self.report.rebuilt_curves = self.rebuilt_curves.len();
            self.report.removed_triangles = deleted.iter().filter(|deleted| **deleted).count();
            self.report.relabeled_triangles = relabeled;
            self.changed_new_keys = changed_new_keys;
            self.kept_order = (0..count).filter(|index| !deleted[*index]).collect();
            let mut vertex_map = vec![usize::MAX; mesh.vertices.len()];
            for &index in &self.kept_order {
                for vertex in mesh.triangles[index].vertices {
                    vertex_map[vertex] = 0;
                }
            }
            for (index, edge) in mesh.boundary_edges.iter().enumerate() {
                if !changed_edge[index] {
                    for vertex in edge.vertices {
                        vertex_map[vertex] = 0;
                    }
                }
            }
            self.vertex_map = vertex_map;
            self.changed_edge = changed_edge;
            self.deleted = deleted;
            self.relabel = relabel;
            self.incident = incident;
            return Ok(());
        }
    }

    /// Gives every kept atom's endpoints their new-plan trace ids, so a
    /// changed atom that ends at a kept vertex reuses it instead of opening a
    /// crack. Endpoints are found through the kept boundary edges, whose
    /// labels and parameters are exact copies of the plan's.
    fn pair_traces(&mut self) -> Result<(), MeshError> {
        let builder = self.builder.as_mut().unwrap();
        let mut endpoints = BTreeMap::<LabelKey, Vec<(f64, usize)>>::new();
        for edge in &builder.boundary_edges {
            let key = label_key(edge.label)?;
            for endpoint in 0..2 {
                endpoints
                    .entry(key)
                    .or_default()
                    .push((edge.parameters[endpoint], edge.vertices[endpoint]));
            }
        }
        for atom in self
            .plan
            .boundaries
            .iter()
            .filter(|atom| !self.changed_new_keys.contains(&atom_key(atom)))
        {
            let key = label_key(topology_label(*atom))?;
            for endpoint in 0..2 {
                let vertex = endpoint_vertex(&endpoints, key, atom.parameter[endpoint]).ok_or(
                    MeshError::Topology("a kept atom endpoint is missing from the previous mesh"),
                )?;
                let trace = atom.traces[endpoint];
                if builder.vertices[vertex]
                    .trace
                    .is_some_and(|existing| existing != trace)
                {
                    return Err(MeshError::Topology(
                        "a kept vertex is claimed by two traces",
                    ));
                }
                builder.vertices[vertex].trace = Some(trace);
                if self
                    .trace_vertices
                    .insert(trace, vertex)
                    .is_some_and(|existing| existing != vertex)
                {
                    return Err(MeshError::Topology(
                        "a kept trace is represented by two vertices",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Expands the changed atoms of every face into vertex chains, recording
    /// the cavity edges they contribute, and collects the rim edges of the
    /// removed triangles.
    fn expand(&mut self) -> Result<(), MeshError> {
        let builder = self.builder.as_mut().unwrap();
        let trace_points = self
            .plan
            .vertices
            .iter()
            .map(|vertex| (vertex.id, vertex.point))
            .collect::<BTreeMap<_, _>>();
        let mut chains = BTreeMap::<(usize, Option<CurveTraceSide>), _>::new();
        let mut free_slits = BTreeMap::<(RegionId, CurveId), Vec<PlannedFaceStep>>::new();
        for domain in &self.plan.domains {
            let mut edge_counts = BTreeMap::<usize, usize>::new();
            for step in domain.steps.iter().flatten() {
                *edge_counts.entry(step.edge).or_default() += 1;
            }
            for step in domain.steps.iter().flatten() {
                if !self.changed_new_keys.contains(&atom_key(&step.boundary)) {
                    continue;
                }
                let separated =
                    matches!(step.boundary.behavior, Some(SpanBehavior::Separated { .. }));
                if separated && edge_counts.get(&step.edge).copied().unwrap_or(0) > 1 {
                    let PlannedBoundarySource::Curve { curve, .. } = step.boundary.source else {
                        continue;
                    };
                    free_slits
                        .entry((domain.region, curve))
                        .or_default()
                        .push(*step);
                    continue;
                }
                let side = match step.boundary.source {
                    PlannedBoundarySource::Curve { side, .. } if separated => Some(side),
                    _ => None,
                };
                let key = (step.edge, side);
                if let std::collections::btree_map::Entry::Vacant(entry) = chains.entry(key) {
                    entry.insert(expand_step_chain(
                        builder,
                        &trace_points,
                        &mut self.trace_vertices,
                        step,
                    )?);
                }
                let chain = &chains[&key];
                let (oriented, parameters) = if chain.traces == step.boundary.traces {
                    (chain.vertices.clone(), chain.parameters.clone())
                } else if chain.traces == [step.boundary.traces[1], step.boundary.traces[0]] {
                    (
                        chain.vertices.iter().rev().copied().collect::<Vec<_>>(),
                        chain.parameters.iter().rev().copied().collect::<Vec<_>>(),
                    )
                } else {
                    return Err(MeshError::Topology("topology trace chain is inconsistent"));
                };
                let label = topology_label(step.boundary);
                for (pair, parameter) in oriented.windows(2).zip(parameters.windows(2)) {
                    self.cavity_edges.push((domain.region, pair[0], pair[1]));
                    let key = edge_key(pair[0], pair[1]);
                    if !builder.boundary_keys.contains(&key) {
                        builder.add_boundary_edge(BoundaryEdge {
                            vertices: [pair[0], pair[1]],
                            label,
                            parameters: [parameter[0], parameter[1]],
                        });
                    }
                }
            }
        }
        for ((region, _), steps) in free_slits {
            self.slit_runs.extend(
                split_slit_runs(&steps)?
                    .into_iter()
                    .map(|run| (region, run)),
            );
        }

        // Rim edges: each removed triangle's edges that face a kept triangle
        // or carry a kept boundary label, oriented with the removed side on
        // the left, belong to the cavity of the face on that side.
        let mesh = self.previous.clone();
        for (index, triangle) in mesh.triangles.iter().enumerate() {
            if !self.deleted[index] {
                continue;
            }
            for corner in 0..3 {
                let (u, v) = (
                    triangle.vertices[corner],
                    triangle.vertices[(corner + 1) % 3],
                );
                let key = edge_key(u, v);
                let mapped = (self.vertex_map[u], self.vertex_map[v]);
                if let Some(&edge_index) = self.old_boundary_edges.get(&key) {
                    if self.changed_edge[edge_index] {
                        continue;
                    }
                    let region = self.side_region(&mesh.boundary_edges[edge_index], u)?;
                    self.cavity_edges.push((region, mapped.0, mapped.1));
                    continue;
                }
                let neighbors = across(&self.incident, u, v).collect::<Vec<_>>();
                let kept = neighbors
                    .iter()
                    .copied()
                    .find(|neighbor| *neighbor != index && !self.deleted[*neighbor]);
                match kept {
                    Some(neighbor) => {
                        let region =
                            self.relabel[neighbor].unwrap_or(mesh.triangles[neighbor].region);
                        self.cavity_edges.push((region, mapped.0, mapped.1));
                    }
                    None if neighbors.len() == 1 => {
                        return Err(MeshError::Topology(
                            "previous mesh has an unlabeled open edge",
                        ));
                    }
                    None => {}
                }
            }
        }
        Ok(())
    }

    /// The region on the side of a kept boundary edge that a removed triangle
    /// occupied, read from the new plan's atom for that side.
    fn side_region(&self, edge: &BoundaryEdge, from: usize) -> Result<RegionId, MeshError> {
        let (start, end) = if edge.vertices[0] == from {
            (edge.parameters[0], edge.parameters[1])
        } else {
            (edge.parameters[1], edge.parameters[0])
        };
        let source = match edge.label {
            BoundaryLabel::Outer(side) => PlannedBoundarySource::Outer(side),
            BoundaryLabel::Curve {
                curve,
                span,
                side,
                separated,
            } => PlannedBoundarySource::Curve {
                curve,
                span,
                side: if separated {
                    side
                } else if start < end {
                    CurveTraceSide::Left
                } else {
                    CurveTraceSide::Right
                },
            },
            _ => return Err(MeshError::Topology("carving needs a topology mesh")),
        };
        self.plan
            .boundary_at(source, 0.5 * (start + end))
            .map(|atom| atom.region)
            .ok_or(MeshError::Topology(
                "a kept boundary edge has no atom in the new plan",
            ))
    }

    /// Walks the cavity edges of each region into closed cycles with the
    /// cavity on the left. Counter-clockwise cycles bound cavity components,
    /// clockwise ones are holes of the smallest component containing them.
    fn cavity_domains(&self) -> Result<Vec<TriangulationDomain>, MeshError> {
        let builder = self.builder.as_ref().unwrap();
        let point = |index: usize| builder.vertices[index].point;
        let mut by_region = BTreeMap::<RegionId, BTreeSet<(usize, usize)>>::new();
        for (region, u, v) in &self.cavity_edges {
            if u == v {
                return Err(MeshError::Topology("cavity edge is degenerate"));
            }
            by_region.entry(*region).or_default().insert((*u, *v));
        }
        let mut domains = vec![];
        for (region, edges) in by_region {
            let mut outgoing = BTreeMap::<usize, Vec<usize>>::new();
            for (u, v) in &edges {
                outgoing.entry(*u).or_default().push(*v);
            }
            // At a vertex, continue along the outgoing edge met first when
            // turning clockwise from the reversed incoming direction; that
            // keeps the cavity interior on the left through pinch vertices.
            let next = |from: usize, at: usize| -> Result<usize, MeshError> {
                let incoming = point(from) - point(at);
                let base = incoming.y.atan2(incoming.x);
                outgoing
                    .get(&at)
                    .into_iter()
                    .flatten()
                    .map(|to| {
                        let direction = point(*to) - point(at);
                        let mut clockwise = (base - direction.y.atan2(direction.x))
                            .rem_euclid(std::f64::consts::TAU);
                        if clockwise <= 1.0e-12 || *to == from {
                            clockwise = std::f64::consts::TAU;
                        }
                        (clockwise, *to)
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
                    .map(|(_, to)| to)
                    .ok_or(MeshError::Topology("cavity boundary has a dead end"))
            };
            let mut used = BTreeSet::new();
            let mut cycles = vec![];
            for start in &edges {
                if used.contains(start) {
                    continue;
                }
                used.insert(*start);
                let mut cycle = vec![start.0];
                let mut current = *start;
                loop {
                    cycle.push(current.1);
                    let to = next(current.0, current.1)?;
                    let edge = (current.1, to);
                    if used.contains(&edge) {
                        if edge == *start {
                            break;
                        }
                        return Err(MeshError::Topology("cavity boundary is not a closed walk"));
                    }
                    used.insert(edge);
                    current = edge;
                }
                cycle.pop();
                if cycle.len() < 3 {
                    return Err(MeshError::Topology("cavity cycle is too short"));
                }
                let area = signed_area(cycle.iter().map(|index| point(*index)));
                cycles.push((area, cycle));
            }
            let mut outers = cycles
                .iter()
                .filter(|(area, _)| *area > 0.0)
                .map(|(area, cycle)| (*area, cycle.clone(), Vec::<Vec<usize>>::new()))
                .collect::<Vec<_>>();
            for (area, hole) in &cycles {
                if *area > 0.0 {
                    continue;
                }
                if *area == 0.0 {
                    return Err(MeshError::Topology("cavity cycle has no area"));
                }
                let candidates = hole
                    .iter()
                    .map(|index| point(*index))
                    .chain(
                        hole.iter()
                            .zip(hole.iter().cycle().skip(1))
                            .map(|(a, b)| point(*a).lerp(point(*b), 0.5)),
                    )
                    .collect::<Vec<_>>();
                let mut container = None::<(f64, usize)>;
                for wanted in [PolygonLocation::Inside, PolygonLocation::Boundary] {
                    for (index, (area, outer, _)) in outers.iter().enumerate() {
                        let polygon = outer.iter().map(|index| point(*index)).collect::<Vec<_>>();
                        if candidates
                            .iter()
                            .any(|candidate| locate_in_polygon(*candidate, &polygon) == wanted)
                            && container.is_none_or(|(best, _)| *area < best)
                        {
                            container = Some((*area, index));
                        }
                    }
                    if container.is_some() {
                        break;
                    }
                }
                let (_, index) = container.ok_or(MeshError::Topology(
                    "cavity hole lies outside every cavity component",
                ))?;
                outers[index].2.push(hole.clone());
            }
            domains.extend(
                outers
                    .into_iter()
                    .map(|(_, outer, holes)| TriangulationDomain {
                        region,
                        outer,
                        holes,
                    }),
            );
        }
        Ok(domains)
    }

    /// Drops vertices the removed band left behind and renumbers the rest.
    fn compact(&self, mut mesh: TriMesh) -> Result<TriMesh, MeshError> {
        // Kept triangles keep the size adaptation requested for them; the
        // refill's triangles take the request of the removed triangle under
        // their centroid, so the next repair preserves the band again.
        let kept = self.kept_order.len();
        let requested = mesh
            .triangles
            .iter()
            .enumerate()
            .map(|(index, triangle)| {
                if index < kept {
                    self.previous.requested_size(self.kept_order[index])
                } else {
                    let [a, b, c] = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
                    self.size_field
                        .as_ref()
                        .and_then(|field| field.request_at((a + b + c) / 3.0))
                }
            })
            .collect::<Vec<_>>();
        mesh.requested_sizes = if requested.iter().any(Option::is_some) {
            requested
        } else {
            vec![]
        };
        let mut used = vec![false; mesh.vertices.len()];
        for triangle in &mesh.triangles {
            for vertex in triangle.vertices {
                used[vertex] = true;
            }
        }
        let mut remap = vec![usize::MAX; mesh.vertices.len()];
        let mut vertices = Vec::with_capacity(mesh.vertices.len());
        for (index, vertex) in mesh.vertices.iter().enumerate() {
            if used[index] {
                remap[index] = vertices.len();
                vertices.push(*vertex);
            }
        }
        for triangle in &mut mesh.triangles {
            triangle.vertices = triangle.vertices.map(|vertex| remap[vertex]);
        }
        for edge in &mut mesh.boundary_edges {
            edge.vertices = edge.vertices.map(|vertex| remap[vertex]);
            if edge.vertices.contains(&usize::MAX) {
                return Err(MeshError::Topology(
                    "a boundary edge survived without an element",
                ));
            }
        }
        mesh.vertices = vertices;
        mesh.geometry_revision = self.plan.geometry_revision;
        mesh.mesh_revision = self.mesh_revision;
        Ok(mesh)
    }
}

/// Refuses a refill with elements the solver could not step. Refinement
/// beside frozen rim edges accepts elements it cannot fix, so the floor is
/// generous: half the mesher's minimum angle, or half the worst angle the
/// repaired mesh already had, whichever is lower. Anything below is a refill
/// defect, and the error says so by name and place, so a fallback that shows
/// it in the panel is a bug report rather than noise.
fn verify_repair_quality(
    mesh: &TriMesh,
    kept: usize,
    previous_worst_angle: f64,
    options: MeshingOptions,
) -> Result<(), MeshError> {
    let floor = 0.5 * options.minimum_angle_degrees.min(previous_worst_angle);
    let mut count = 0;
    let mut worst = f64::INFINITY;
    let mut point = Point2::default();
    for index in kept..mesh.triangles.len() {
        let angle = mesh
            .triangle_quality(index)
            .map_or(0.0, |quality| quality.minimum_angle_degrees);
        if angle < floor {
            count += 1;
            if angle < worst {
                worst = angle;
                let [a, b, c] = mesh.triangles[index]
                    .vertices
                    .map(|vertex| mesh.vertices[vertex].point);
                point = (a + b + c) / 3.0;
            }
        }
    }
    if count > 0 {
        return Err(MeshError::DegenerateRepair {
            count,
            worst_angle_degrees: worst,
            floor_degrees: floor,
            point,
        });
    }
    Ok(())
}

/// Runs a carve to completion.
pub fn carve_topology_mesh(
    previous: Arc<TriMesh>,
    previous_plan: &TopologyMeshPlan,
    plan: TopologyMeshPlan,
    topology: Arc<TopologySnapshot>,
    mesh_revision: u64,
    options: MeshingOptions,
    preserve_adaptation: bool,
) -> Result<(TriMesh, CarveReport), MeshError> {
    let mut job = TopologyCarveJob::new(
        previous,
        previous_plan,
        plan,
        topology,
        mesh_revision,
        options,
        preserve_adaptation,
    );
    loop {
        if let Some(result) = job.advance(4096) {
            return result.map(|mesh| (mesh, job.report()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CurveSpan, CurveSpline, FaceRegionAssignment, MeshAdaptationJob, MeshAdaptationOptions,
        MeshAdaptationResult, MeshAdaptationState, MeshQuality, MeshSizeField, MeshVertex,
        OpenCubicSpline, OuterSide, PeriodicCubicSpline, TopologyCurve, TopologyGeometry,
        TopologyVertex, TopologyVertexId, TopologyVertexLocation, compile_topology,
        mesh_topology_plan,
    };

    fn spans(start: u64, count: usize, behavior: SpanBehavior) -> Vec<CurveSpan> {
        (0..count)
            .map(|index| CurveSpan {
                id: CurveSpanId(start + index as u64),
                behavior,
            })
            .collect()
    }

    fn options(target: f64) -> MeshingOptions {
        MeshingOptions {
            curve_tolerance: 8.0e-4,
            target_edge_length: target,
            minimum_angle_degrees: 10.0,
            max_vertices: 40_000,
            max_triangles: 80_000,
            max_refinement_steps: 40_000,
        }
    }

    fn hole_geometry(center: Point2) -> TopologyGeometry {
        TopologyGeometry {
            curves: vec![
                TopologyCurve::new(
                    CurveId(4),
                    CurveSpline::Closed(PeriodicCubicSpline::rounded(center, 0.3)),
                    spans(40, 8, SpanBehavior::REFLECTING),
                )
                .unwrap(),
            ],
            ..TopologyGeometry::default()
        }
    }

    /// Plan for a compiled scene: the face containing `inside` is either
    /// excluded or given region 2, everything else is region 1.
    fn plan_for(
        topology: &TopologySnapshot,
        inside: Point2,
        inner_region: Option<RegionId>,
        options: MeshingOptions,
    ) -> TopologyMeshPlan {
        let inner = topology.face_at(inside).unwrap();
        let assignments = topology
            .faces
            .iter()
            .map(|face| FaceRegionAssignment {
                face: face.id,
                region: if face.id == inner {
                    inner_region
                } else {
                    Some(RegionId(1))
                },
            })
            .collect::<Vec<_>>();
        TopologyMeshPlan::new(topology, &assignments)
            .unwrap()
            .coarsened(
                topology,
                super::super::AtomCoarsening::from_meshing(options),
            )
            .unwrap()
    }

    fn single_face_plan(topology: &TopologySnapshot, options: MeshingOptions) -> TopologyMeshPlan {
        let assignments = topology
            .faces
            .iter()
            .map(|face| FaceRegionAssignment {
                face: face.id,
                region: Some(RegionId(1)),
            })
            .collect::<Vec<_>>();
        TopologyMeshPlan::new(topology, &assignments)
            .unwrap()
            .coarsened(
                topology,
                super::super::AtomCoarsening::from_meshing(options),
            )
            .unwrap()
    }

    fn carve(
        previous: &TriMesh,
        previous_plan: &TopologyMeshPlan,
        plan: &TopologyMeshPlan,
        topology: &TopologySnapshot,
        options: MeshingOptions,
    ) -> (TriMesh, CarveReport) {
        carve_topology_mesh(
            Arc::new(previous.clone()),
            previous_plan,
            plan.clone(),
            Arc::new(topology.clone()),
            previous.mesh_revision + 1,
            options,
            true,
        )
        .unwrap()
    }

    /// The adaptation contract is the strictest reader of a topology mesh:
    /// it checks every trace, every atom's coverage, the metadata of every
    /// boundary vertex and the pairing of separated sides.
    fn assert_contract(mesh: &TriMesh, plan: &TopologyMeshPlan, options: MeshingOptions) {
        let target = options.target_edge_length;
        let mut job = MeshAdaptationJob::new_topology(
            Arc::new(mesh.clone()),
            plan,
            MeshAdaptationState::from_mesh(mesh),
            mesh.mesh_revision + 1,
            Arc::new(move |_, _| target),
            MeshAdaptationOptions {
                meshing: options,
                minimum_target_edge_length: target * 0.25,
                maximum_target_edge_length: target,
                max_topology_changes: 4_000,
                max_work_units: 50_000_000,
                ..MeshAdaptationOptions::default()
            },
        );
        loop {
            if let Some(result) = job.advance(4096) {
                result.expect("carved mesh satisfies the adaptation contract");
                return;
            }
        }
    }

    type TriangleKey = ([[u64; 2]; 3], RegionId);

    fn triangle_keys(mesh: &TriMesh) -> BTreeSet<TriangleKey> {
        mesh.triangles
            .iter()
            .map(|triangle| {
                let mut points = triangle
                    .vertices
                    .map(|vertex| mesh.vertices[vertex].point)
                    .map(|point| [point.x.to_bits(), point.y.to_bits()]);
                points.sort_unstable();
                (points, triangle.region)
            })
            .collect()
    }

    fn region_areas(mesh: &TriMesh) -> BTreeMap<RegionId, f64> {
        let mut areas = BTreeMap::new();
        for triangle in &mesh.triangles {
            let [a, b, c] = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
            *areas.entry(triangle.region).or_insert(0.0) += (b - a).cross(c - a) * 0.5;
        }
        areas
    }

    fn assert_same_areas(a: &TriMesh, b: &TriMesh) {
        let (a, b) = (region_areas(a), region_areas(b));
        assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>());
        for (region, area) in &a {
            assert!(
                (area - b[region]).abs() < 1.0e-9,
                "region {region:?}: {area} vs {}",
                b[region]
            );
        }
    }

    fn centroid(mesh: &TriMesh, triangle: &MeshTriangle) -> Point2 {
        let [a, b, c] = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
        (a + b + c) / 3.0
    }

    fn max_edge(mesh: &TriMesh, triangle: &MeshTriangle) -> f64 {
        let points = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
        (0..3)
            .map(|corner| (points[(corner + 1) % 3] - points[corner]).norm())
            .fold(0.0, f64::max)
    }

    struct MovedHole {
        options: MeshingOptions,
        before_plan: TopologyMeshPlan,
        before: TriMesh,
        after_topology: TopologySnapshot,
        after_plan: TopologyMeshPlan,
        fresh: TriMesh,
    }

    fn moved_hole(from: Point2, to: Point2, target: f64) -> MovedHole {
        let options = options(target);
        let before_topology = compile_topology(&hole_geometry(from), 1).unwrap();
        let before_plan = plan_for(&before_topology, from, None, options);
        let before = mesh_topology_plan(&before_plan, 10, options).unwrap();
        let after_topology = compile_topology(&hole_geometry(to), 2).unwrap();
        let after_plan = plan_for(&after_topology, to, None, options);
        let fresh = mesh_topology_plan(&after_plan, 11, options).unwrap();
        MovedHole {
            options,
            before_plan,
            before,
            after_topology,
            after_plan,
            fresh,
        }
    }

    /// A small drag carves a band around the hole and leaves the rest of the
    /// mesh exactly as it was: every kept triangle reappears unchanged, the
    /// regions cover the same areas as a fresh mesh, and the contract holds.
    #[test]
    fn a_nudged_hole_is_carved_locally_and_the_rest_is_untouched() {
        let scene = moved_hole(Point2::new(0.0, 0.0), Point2::new(0.04, 0.03), 0.12);
        let (carved, report) = carve(
            &scene.before,
            &scene.before_plan,
            &scene.after_plan,
            &scene.after_topology,
            scene.options,
        );
        assert_contract(&carved, &scene.after_plan, scene.options);
        assert_same_areas(&carved, &scene.fresh);
        assert!(report.kept_triangles > 0, "{report:?}");
        assert!(
            report.removed_triangles * 2 < scene.before.triangles.len(),
            "{report:?}"
        );
        assert_eq!(report.rebuilt_curves, 1, "{report:?}");
        let before = triangle_keys(&scene.before);
        let after = triangle_keys(&carved);
        assert!(
            before.intersection(&after).count() >= report.kept_triangles,
            "{report:?}"
        );
        assert_eq!(
            carved.triangles.len(),
            report.kept_triangles + report.inserted_triangles
        );
        assert_eq!(carved.mesh_revision, scene.before.mesh_revision + 1);
        assert_eq!(carved.geometry_revision, scene.after_plan.geometry_revision);
    }

    /// A drag far longer than any repair radius the legacy job allowed is
    /// still a local operation: the band covers the sweep, everything else
    /// survives.
    #[test]
    fn a_hole_dragged_across_the_domain_needs_no_motion_cap() {
        let scene = moved_hole(Point2::new(-0.45, -0.35), Point2::new(0.45, 0.4), 0.12);
        let (carved, report) = carve(
            &scene.before,
            &scene.before_plan,
            &scene.after_plan,
            &scene.after_topology,
            scene.options,
        );
        assert_contract(&carved, &scene.after_plan, scene.options);
        assert_same_areas(&carved, &scene.fresh);
        assert!(report.kept_triangles > 0, "{report:?}");
        let before = triangle_keys(&scene.before);
        let after = triangle_keys(&carved);
        assert!(before.intersection(&after).count() >= report.kept_triangles);
    }

    /// A hole brought closer to the outer wall than one edge length leaves a
    /// gap too thin for the old triangles; carving triangulates it afresh.
    #[test]
    fn a_hole_moved_against_the_outer_wall_is_carved() {
        let scene = moved_hole(Point2::new(0.0, 0.0), Point2::new(0.685, 0.0), 0.12);
        let (carved, _) = carve(
            &scene.before,
            &scene.before_plan,
            &scene.after_plan,
            &scene.after_topology,
            scene.options,
        );
        assert_contract(&carved, &scene.after_plan, scene.options);
        assert_same_areas(&carved, &scene.fresh);
    }

    /// Carving with the plan it was built from is the identity.
    #[test]
    fn carving_with_an_unchanged_plan_changes_nothing() {
        let scene = moved_hole(Point2::new(0.0, 0.0), Point2::new(0.0, 0.0), 0.12);
        let (carved, report) = carve(
            &scene.before,
            &scene.before_plan,
            &scene.before_plan,
            &scene.after_topology,
            scene.options,
        );
        assert_eq!(report.removed_triangles, 0);
        assert_eq!(report.changed_atoms, 0);
        assert_eq!(report.inserted_triangles, 0);
        assert_eq!(triangle_keys(&carved), triangle_keys(&scene.before));
        assert_eq!(carved.vertices.len(), scene.before.vertices.len());
        assert_eq!(
            carved.boundary_edges.len(),
            scene.before.boundary_edges.len()
        );
    }

    /// The band is refilled at the density the removed triangles had, so
    /// adaptive refinement around a boundary survives a nudge of that
    /// boundary, and the refinement away from it is not touched at all.
    #[test]
    fn carving_an_adapted_mesh_keeps_the_band_dense() {
        let options = options(0.28);
        let from = Point2::new(0.0, 0.0);
        let to = Point2::new(0.03, 0.02);
        let before_topology = compile_topology(&hole_geometry(from), 1).unwrap();
        let before_plan = plan_for(&before_topology, from, None, options);
        let base = mesh_topology_plan(&before_plan, 10, options).unwrap();
        let fine = 0.07;
        let adapted = adapt_around(&base, &before_plan, from, fine, options);
        assert!(adapted.triangles.len() > 3 * base.triangles.len());

        let after_topology = compile_topology(&hole_geometry(to), 2).unwrap();
        let after_plan = plan_for(&after_topology, to, None, options);
        let (carved, report) = carve(
            &adapted,
            &before_plan,
            &after_plan,
            &after_topology,
            options,
        );
        assert_contract(&carved, &after_plan, options);
        assert!(
            report.kept_triangles * 2 > adapted.triangles.len(),
            "{report:?}"
        );

        let kept = triangle_keys(&adapted);
        let new_band = carved
            .triangles
            .iter()
            .filter(|triangle| !kept.contains(&triangle_keys_one(&carved, triangle)))
            .filter(|triangle| (centroid(&carved, triangle) - to).norm() < 0.45)
            .collect::<Vec<_>>();
        assert!(!new_band.is_empty());
        let longest = new_band
            .iter()
            .map(|triangle| max_edge(&carved, triangle))
            .fold(0.0, f64::max);
        assert!(longest <= fine * 1.6, "longest new edge {longest}");
    }

    fn triangle_keys_one(mesh: &TriMesh, triangle: &MeshTriangle) -> TriangleKey {
        let mut points = triangle
            .vertices
            .map(|vertex| mesh.vertices[vertex].point)
            .map(|point| [point.x.to_bits(), point.y.to_bits()]);
        points.sort_unstable();
        (points, triangle.region)
    }

    fn baffle_geometry(points: Vec<Point2>) -> TopologyGeometry {
        TopologyGeometry {
            curves: vec![
                TopologyCurve::new(
                    CurveId(3),
                    CurveSpline::Open(OpenCubicSpline::polyline(points).unwrap()),
                    spans(30, 3, SpanBehavior::REFLECTING),
                )
                .unwrap(),
            ],
            ..TopologyGeometry::default()
        }
    }

    /// Moving one end of a baffle rebuilds the whole curve, so its two
    /// coincident sides are expanded together and stay paired piece for
    /// piece, which the contract's separated-coverage rule checks.
    #[test]
    fn a_moved_baffle_is_rebuilt_whole_with_paired_sides() {
        let options = options(0.12);
        let before_geometry = baffle_geometry(vec![
            Point2::new(-0.6, 0.0),
            Point2::new(-0.2, 0.0),
            Point2::new(0.2, 0.0),
            Point2::new(0.6, 0.0),
        ]);
        let before_topology = compile_topology(&before_geometry, 1).unwrap();
        let before_plan = single_face_plan(&before_topology, options);
        let before = mesh_topology_plan(&before_plan, 10, options).unwrap();
        let after_geometry = baffle_geometry(vec![
            Point2::new(-0.6, 0.0),
            Point2::new(-0.2, 0.0),
            Point2::new(0.2, 0.0),
            Point2::new(0.6, 0.25),
        ]);
        let after_topology = compile_topology(&after_geometry, 2).unwrap();
        let after_plan = single_face_plan(&after_topology, options);
        let kept_atoms = after_plan
            .boundaries
            .iter()
            .filter(|atom| {
                before_plan
                    .boundaries
                    .iter()
                    .any(|old| atom_key(old) == atom_key(atom))
            })
            .filter(|atom| matches!(atom.source, PlannedBoundarySource::Curve { .. }))
            .count();
        assert!(kept_atoms > 0, "the unmoved spans keep their atoms");
        let (carved, report) = carve(&before, &before_plan, &after_plan, &after_topology, options);
        assert_contract(&carved, &after_plan, options);
        assert_eq!(report.rebuilt_curves, 1, "{report:?}");
        assert!(report.kept_triangles > 0, "{report:?}");
        let fresh = mesh_topology_plan(&after_plan, 11, options).unwrap();
        assert_same_areas(&carved, &fresh);
        let sides = |mesh: &TriMesh, wanted: CurveTraceSide| {
            mesh.boundary_edges
                .iter()
                .filter(|edge| {
                    matches!(edge.label, BoundaryLabel::Curve { side, separated: true, .. } if side == wanted)
                })
                .count()
        };
        assert_eq!(
            sides(&carved, CurveTraceSide::Left),
            sides(&carved, CurveTraceSide::Right)
        );
        assert!(sides(&carved, CurveTraceSide::Left) > 0);
    }

    fn arms_geometry(center: Point2) -> TopologyGeometry {
        let junction = TopologyVertexId(30);
        let mut horizontal = TopologyCurve::new(
            CurveId(30),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![
                    Point2::new(-0.7, 0.0),
                    Point2::new(0.0, 0.0),
                    Point2::new(0.7, 0.0),
                ])
                .unwrap(),
            ),
            spans(300, 2, SpanBehavior::REFLECTING),
        )
        .unwrap();
        horizontal.nodes[1].vertex = Some(junction);
        let mut branch = TopologyCurve::new(
            CurveId(31),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![Point2::new(0.0, 0.0), Point2::new(0.0, 0.7)])
                    .unwrap(),
            ),
            spans(302, 1, SpanBehavior::REFLECTING),
        )
        .unwrap();
        branch.nodes[0].vertex = Some(junction);
        let mut geometry = TopologyGeometry {
            curves: vec![horizontal, branch],
            vertices: vec![TopologyVertex {
                id: junction,
                location: TopologyVertexLocation::Interior(center),
            }],
            ..TopologyGeometry::default()
        };
        geometry.synchronize_vertices().unwrap();
        geometry
    }

    /// Moving a separated junction moves all three arms; the carved mesh has
    /// one trace vertex per sector at the new point, exactly like a fresh one.
    #[test]
    fn a_moved_separated_junction_keeps_three_sector_traces() {
        let options = options(0.12);
        let before_topology = compile_topology(&arms_geometry(Point2::new(0.0, 0.0)), 1).unwrap();
        let before_plan = single_face_plan(&before_topology, options);
        let before = mesh_topology_plan(&before_plan, 10, options).unwrap();
        let to = Point2::new(0.1, 0.05);
        let after_topology = compile_topology(&arms_geometry(to), 2).unwrap();
        let after_plan = single_face_plan(&after_topology, options);
        let (carved, report) = carve(&before, &before_plan, &after_plan, &after_topology, options);
        assert_contract(&carved, &after_plan, options);
        assert_eq!(report.rebuilt_curves, 2, "{report:?}");
        assert!(report.kept_triangles > 0, "{report:?}");
        let expected = after_plan
            .vertices
            .iter()
            .filter(|vertex| vertex.point == to)
            .map(|vertex| vertex.id)
            .collect::<BTreeSet<_>>();
        assert_eq!(expected.len(), 3);
        let actual = carved
            .vertices
            .iter()
            .filter(|vertex| vertex.point == to)
            .filter_map(|vertex| vertex.trace)
            .collect::<BTreeSet<_>>();
        assert_eq!(actual, expected);
        let fresh = mesh_topology_plan(&after_plan, 11, options).unwrap();
        assert_same_areas(&carved, &fresh);
    }

    /// Activating a hole as a subdomain gives its face a cavity that is the
    /// whole face; the mesh around it is rebuilt only as far as the shared
    /// curve requires, and the new region has the hole's area.
    #[test]
    fn a_hole_that_becomes_a_domain_is_filled_in_place() {
        let options = options(0.12);
        let center = Point2::new(0.1, -0.05);
        let topology = compile_topology(&hole_geometry(center), 1).unwrap();
        let excluded = plan_for(&topology, center, None, options);
        let before = mesh_topology_plan(&excluded, 10, options).unwrap();
        let active = plan_for(&topology, center, Some(RegionId(2)), options);
        let (carved, report) = carve(&before, &excluded, &active, &topology, options);
        assert_contract(&carved, &active, options);
        assert!(report.kept_triangles > 0, "{report:?}");
        assert_eq!(report.rebuilt_curves, 1, "{report:?}");
        let fresh = mesh_topology_plan(&active, 11, options).unwrap();
        assert_same_areas(&carved, &fresh);
        let areas = region_areas(&carved);
        assert!(
            areas[&RegionId(2)] > 0.2 && areas[&RegionId(2)] < 0.3,
            "{areas:?}"
        );
        let before_keys = triangle_keys(&before);
        assert!(before_keys.intersection(&triangle_keys(&carved)).count() >= report.kept_triangles);
    }

    /// The cooperative job gives the same mesh for every slice size.
    #[test]
    fn carving_is_deterministic_across_work_slice_sizes() {
        let scene = moved_hole(Point2::new(0.0, 0.0), Point2::new(0.05, -0.02), 0.14);
        let run = |slice: usize| {
            let mut job = TopologyCarveJob::new(
                Arc::new(scene.before.clone()),
                &scene.before_plan,
                scene.after_plan.clone(),
                Arc::new(scene.after_topology.clone()),
                scene.before.mesh_revision + 1,
                scene.options,
                true,
            );
            loop {
                if let Some(result) = job.advance(slice) {
                    return result.unwrap();
                }
            }
        };
        let a = run(1);
        let b = run(977);
        assert_eq!(a, b);
    }

    /// A vertical divider from the bottom to the top wall at `x`, attached to
    /// the outer boundary by authored vertices.
    fn vertical_divider(
        id: u64,
        x: f64,
        behavior: SpanBehavior,
    ) -> (TopologyCurve, [TopologyVertex; 2]) {
        let bottom = TopologyVertexId(id * 10);
        let top = TopologyVertexId(id * 10 + 1);
        let mut divider = TopologyCurve::new(
            CurveId(id),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![Point2::new(x, -1.0), Point2::new(x, 1.0)]).unwrap(),
            ),
            spans(id * 10, 1, behavior),
        )
        .unwrap();
        divider.nodes[0].vertex = Some(bottom);
        divider.nodes[1].vertex = Some(top);
        let fraction = (x + 1.0) / 2.0;
        (
            divider,
            [
                TopologyVertex {
                    id: bottom,
                    location: TopologyVertexLocation::Outer {
                        side: OuterSide::Bottom,
                        fraction,
                    },
                },
                TopologyVertex {
                    id: top,
                    location: TopologyVertexLocation::Outer {
                        side: OuterSide::Top,
                        fraction,
                    },
                },
            ],
        )
    }

    /// A horizontal transmitting divider with three spans and a second one
    /// with two, meeting at an authored junction at `(x, 0)`; all four arms
    /// end on the outer walls at fixed points, so moving `x` bends the second
    /// divider and changes the horizontal spans beside the junction only. The
    /// horizontal divider's first span, from the left wall to `x = -0.4`, and
    /// every wall atom do not depend on `x`.
    fn cross_geometry(x: f64) -> TopologyGeometry {
        let junction = TopologyVertexId(99);
        let mut horizontal = TopologyCurve::new(
            CurveId(2),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![
                    Point2::new(-1.0, 0.0),
                    Point2::new(-0.4, 0.0),
                    Point2::new(x, 0.0),
                    Point2::new(1.0, 0.0),
                ])
                .unwrap(),
            ),
            spans(20, 3, SpanBehavior::Transmitting),
        )
        .unwrap();
        horizontal.nodes[0].vertex = Some(TopologyVertexId(20));
        horizontal.nodes[2].vertex = Some(junction);
        horizontal.nodes[3].vertex = Some(TopologyVertexId(21));
        let mut vertical = TopologyCurve::new(
            CurveId(1),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![
                    Point2::new(0.5, -1.0),
                    Point2::new(x, 0.0),
                    Point2::new(0.5, 1.0),
                ])
                .unwrap(),
            ),
            spans(10, 2, SpanBehavior::Transmitting),
        )
        .unwrap();
        vertical.nodes[0].vertex = Some(TopologyVertexId(10));
        vertical.nodes[1].vertex = Some(junction);
        vertical.nodes[2].vertex = Some(TopologyVertexId(11));
        let outer = |id: u64, side: OuterSide, fraction: f64| TopologyVertex {
            id: TopologyVertexId(id),
            location: TopologyVertexLocation::Outer { side, fraction },
        };
        let mut geometry = TopologyGeometry {
            curves: vec![horizontal, vertical],
            vertices: vec![
                outer(20, OuterSide::Left, 0.5),
                outer(21, OuterSide::Right, 0.5),
                // The bottom side runs left to right and the top side right
                // to left, so these fractions both land at x = 0.5.
                outer(10, OuterSide::Bottom, 0.75),
                outer(11, OuterSide::Top, 0.25),
                TopologyVertex {
                    id: junction,
                    location: TopologyVertexLocation::Interior(Point2::new(x, 0.0)),
                },
            ],
            ..TopologyGeometry::default()
        };
        geometry.synchronize_vertices().unwrap();
        geometry
    }

    fn geometry_of(parts: Vec<(TopologyCurve, [TopologyVertex; 2])>) -> TopologyGeometry {
        let mut geometry = TopologyGeometry::default();
        for (curve, vertices) in parts {
            geometry.curves.push(curve);
            geometry.vertices.extend(vertices);
        }
        geometry.synchronize_vertices().unwrap();
        geometry
    }

    /// Every face gets its own region, numbered by the face's position.
    fn plan_by_face(topology: &TopologySnapshot, options: MeshingOptions) -> TopologyMeshPlan {
        let assignments = topology
            .faces
            .iter()
            .enumerate()
            .map(|(index, face)| FaceRegionAssignment {
                face: face.id,
                region: Some(RegionId(index as u64 + 1)),
            })
            .collect::<Vec<_>>();
        TopologyMeshPlan::new(topology, &assignments)
            .unwrap()
            .coarsened(
                topology,
                super::super::AtomCoarsening::from_meshing(options),
            )
            .unwrap()
    }

    /// Faces and regions belong to the triangles, not to the atoms: a face
    /// that only changes its region keeps every atom, so nothing is carved and
    /// the face's triangles are relabeled in place.
    #[test]
    fn a_face_reassignment_relabels_kept_triangles_without_carving() {
        let options = options(0.14);
        let topology = compile_topology(
            &geometry_of(vec![vertical_divider(1, 0.0, SpanBehavior::Transmitting)]),
            1,
        )
        .unwrap();
        let before_plan = plan_by_face(&topology, options);
        let before = mesh_topology_plan(&before_plan, 10, options).unwrap();
        let second = topology.faces[1].id;
        let assignments = topology
            .faces
            .iter()
            .map(|face| FaceRegionAssignment {
                face: face.id,
                region: Some(if face.id == second {
                    RegionId(3)
                } else {
                    RegionId(1)
                }),
            })
            .collect::<Vec<_>>();
        let after_plan = TopologyMeshPlan::new(&topology, &assignments)
            .unwrap()
            .coarsened(
                &topology,
                super::super::AtomCoarsening::from_meshing(options),
            )
            .unwrap();
        let (carved, report) = carve(&before, &before_plan, &after_plan, &topology, options);
        assert_contract(&carved, &after_plan, options);
        assert_eq!(report.removed_triangles, 0, "{report:?}");
        assert_eq!(report.inserted_triangles, 0, "{report:?}");
        assert_eq!(report.changed_atoms, 0, "{report:?}");
        assert!(report.relabeled_triangles > 0, "{report:?}");
        assert_eq!(carved.triangles.len(), before.triangles.len());
        let before_areas = region_areas(&before);
        let after_areas = region_areas(&carved);
        assert!((after_areas[&RegionId(3)] - before_areas[&RegionId(2)]).abs() < 1.0e-12);
        assert!((after_areas[&RegionId(1)] - before_areas[&RegionId(1)]).abs() < 1.0e-12);
    }

    /// A new baffle inside a face carves the band it passes through and is
    /// cut as a slit inside it; the rest of the face is untouched.
    #[test]
    fn adding_a_baffle_carves_only_its_band() {
        let options = options(0.12);
        let empty = compile_topology(&TopologyGeometry::default(), 1).unwrap();
        let before_plan = single_face_plan(&empty, options);
        let before = mesh_topology_plan(&before_plan, 10, options).unwrap();
        let with_baffle = compile_topology(
            &baffle_geometry(vec![
                Point2::new(-0.5, 0.1),
                Point2::new(-0.1, 0.2),
                Point2::new(0.3, 0.1),
                Point2::new(0.6, -0.1),
            ]),
            2,
        )
        .unwrap();
        let after_plan = single_face_plan(&with_baffle, options);
        let (carved, report) = carve(&before, &before_plan, &after_plan, &with_baffle, options);
        assert_contract(&carved, &after_plan, options);
        assert_eq!(report.rebuilt_curves, 1, "{report:?}");
        assert!(
            report.kept_triangles * 2 > before.triangles.len(),
            "{report:?}"
        );
        let fresh = mesh_topology_plan(&after_plan, 11, options).unwrap();
        assert_same_areas(&carved, &fresh);
        assert!(carved.boundary_edges.iter().any(|edge| {
            matches!(
                edge.label,
                BoundaryLabel::Curve {
                    separated: true,
                    ..
                }
            )
        }));
    }

    /// Removing a divider merges its two faces: the strip where it ran is
    /// refilled and the absorbed face's triangles are relabeled to the
    /// survivor, nothing else changes.
    #[test]
    fn removing_a_divider_merges_the_faces_in_place() {
        let options = options(0.12);
        let divided = compile_topology(
            &geometry_of(vec![vertical_divider(1, 0.1, SpanBehavior::Transmitting)]),
            1,
        )
        .unwrap();
        let before_plan = plan_by_face(&divided, options);
        let before = mesh_topology_plan(&before_plan, 10, options).unwrap();
        assert_eq!(region_areas(&before).len(), 2);
        let empty = compile_topology(&TopologyGeometry::default(), 2).unwrap();
        let after_plan = single_face_plan(&empty, options);
        let (carved, report) = carve(&before, &before_plan, &after_plan, &empty, options);
        assert_contract(&carved, &after_plan, options);
        assert!(report.relabeled_triangles > 0, "{report:?}");
        assert!(
            report.removed_triangles * 2 < before.triangles.len(),
            "{report:?}"
        );
        assert!(report.kept_triangles > 0, "{report:?}");
        let areas = region_areas(&carved);
        assert_eq!(areas.len(), 1);
        assert!((areas[&RegionId(1)] - 4.0).abs() < 1.0e-9, "{areas:?}");
    }

    /// Flipping a divider from transmitting to reflecting changes the label of
    /// every atom on it; the band along the curve is rebuilt with two sides.
    #[test]
    fn a_behaviour_flip_rebuilds_the_curve_band() {
        let options = options(0.12);
        let transmitting = compile_topology(
            &geometry_of(vec![vertical_divider(1, 0.0, SpanBehavior::Transmitting)]),
            1,
        )
        .unwrap();
        let before_plan = plan_by_face(&transmitting, options);
        let before = mesh_topology_plan(&before_plan, 10, options).unwrap();
        let reflecting = compile_topology(
            &geometry_of(vec![vertical_divider(1, 0.0, SpanBehavior::REFLECTING)]),
            2,
        )
        .unwrap();
        let after_plan = plan_by_face(&reflecting, options);
        // A fresh mesh of a two-sided reflecting divider must itself satisfy
        // the contract: refinement splits both sides of a separated span.
        let fresh = mesh_topology_plan(&after_plan, 11, options).unwrap();
        assert_contract(&fresh, &after_plan, options);
        let (carved, report) = carve(&before, &before_plan, &after_plan, &reflecting, options);
        assert_contract(&carved, &after_plan, options);
        assert!(
            report.kept_triangles * 2 > before.triangles.len(),
            "{report:?}"
        );
        assert_eq!(report.rebuilt_curves, 1, "{report:?}");
        let sides = carved
            .boundary_edges
            .iter()
            .filter_map(|edge| match edge.label {
                BoundaryLabel::Curve {
                    side,
                    separated: true,
                    ..
                } => Some(side),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(sides.len(), 2);
        assert_same_areas(&carved, &fresh);
    }

    /// Two transmitting dividers meet at an authored junction. Moving the
    /// junction moves the vertical divider and the horizontal spans beside
    /// it, while the horizontal divider's far span is kept as it was.
    #[test]
    fn a_moved_crossing_carves_both_curves_near_it() {
        let options = options(0.12);
        let before_topology = compile_topology(&cross_geometry(0.5), 1).unwrap();
        assert_eq!(before_topology.faces.len(), 4, "four quadrants");
        let before_plan = plan_by_face(&before_topology, options);
        let before = mesh_topology_plan(&before_plan, 10, options).unwrap();
        let after_topology = compile_topology(&cross_geometry(0.2), 2).unwrap();
        let after_plan = plan_by_face(&after_topology, options);
        let (carved, report) = carve(&before, &before_plan, &after_plan, &after_topology, options);
        assert_contract(&carved, &after_plan, options);
        assert!(report.kept_triangles > 0, "{report:?}");
        let wall_atoms_changed = before_plan
            .boundaries
            .iter()
            .filter(|atom| matches!(atom.source, PlannedBoundarySource::Outer(_)))
            .filter(|atom| {
                !after_plan
                    .boundaries
                    .iter()
                    .any(|new| atom_key(new) == atom_key(atom))
            })
            .count();
        assert_eq!(
            wall_atoms_changed, 0,
            "the walls are untouched by the junction move"
        );
        // The band reaches the end of the moved span at x = -0.4 and one ring
        // beyond; everything further left is kept triangle for triangle. The
        // regions are compared separately, since the faces are renumbered by
        // the compile and the kept triangles are relabeled accordingly.
        let far_left = |mesh: &TriMesh| {
            mesh.triangles
                .iter()
                .filter(|triangle| {
                    triangle
                        .vertices
                        .iter()
                        .all(|vertex| mesh.vertices[*vertex].point.x < -0.65)
                })
                .map(|triangle| triangle_keys_one(mesh, triangle).0)
                .collect::<BTreeSet<_>>()
        };
        assert!(!far_left(&before).is_empty());
        assert_eq!(far_left(&carved), far_left(&before));
        let fresh = mesh_topology_plan(&after_plan, 11, options).unwrap();
        assert_same_areas(&carved, &fresh);
    }

    /// Runs one adaptation of `base`, a mesh of `plan`, against `field`.
    fn adapt_from(
        base: &TriMesh,
        plan: &TopologyMeshPlan,
        state: MeshAdaptationState,
        field: Arc<dyn MeshSizeField>,
        minimum: f64,
        options: MeshingOptions,
    ) -> MeshAdaptationResult {
        let mut job = MeshAdaptationJob::new_topology(
            Arc::new(base.clone()),
            plan,
            state,
            base.mesh_revision + 1,
            field,
            MeshAdaptationOptions {
                meshing: options,
                minimum_target_edge_length: minimum,
                maximum_target_edge_length: options.target_edge_length,
                max_topology_changes: 8_000,
                max_work_units: 50_000_000,
                ..MeshAdaptationOptions::default()
            },
        );
        loop {
            if let Some(result) = job.advance(4096) {
                break result.unwrap();
            }
        }
    }

    /// Refines `base` around `center`: `fine` within 0.5 of it, the meshing
    /// target beyond 0.7, graded between.
    fn adapt_around(
        base: &TriMesh,
        plan: &TopologyMeshPlan,
        center: Point2,
        fine: f64,
        options: MeshingOptions,
    ) -> TriMesh {
        let coarse = options.target_edge_length;
        let field = move |point: Point2, _: RegionId| {
            let distance = (point - center).norm();
            if distance <= 0.5 {
                fine
            } else if distance >= 0.7 {
                coarse
            } else {
                fine + (distance - 0.5) / 0.2 * (coarse - fine)
            }
        };
        let result = adapt_from(
            base,
            plan,
            MeshAdaptationState::from_mesh(base),
            Arc::new(field),
            fine,
            options,
        );
        assert!(result.report.converged, "{:?}", result.report);
        result.mesh
    }

    fn shortest_edge(mesh: &TriMesh) -> f64 {
        mesh.triangles
            .iter()
            .flat_map(|triangle| {
                let points = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
                (0..3).map(move |corner| (points[(corner + 1) % 3] - points[corner]).norm())
            })
            .fold(f64::INFINITY, f64::min)
    }

    /// Repairs `mesh`, a mesh of `plan` with the hole at `from`, back and
    /// forth between `to` and `from` for an even number of passes, returning
    /// the mesh back at `from`, its plan, and the triangle count after every
    /// pass.
    fn drag_back_and_forth(
        mesh: TriMesh,
        plan: TopologyMeshPlan,
        from: Point2,
        to: Point2,
        passes: usize,
        options: MeshingOptions,
    ) -> (TriMesh, TopologyMeshPlan, Vec<usize>) {
        assert_eq!(passes % 2, 0);
        let mut mesh = mesh;
        let mut plan = plan;
        let mut counts = vec![];
        for pass in 0..passes {
            let center = if pass % 2 == 0 { to } else { from };
            let topology = compile_topology(&hole_geometry(center), pass as u64 + 2).unwrap();
            let next = plan_for(&topology, center, None, options);
            let (carved, _) = carve(&mesh, &plan, &next, &topology, options);
            counts.push(carved.triangles.len());
            mesh = carved;
            plan = next;
        }
        (mesh, plan, counts)
    }

    /// Dragging one curve back and forth over the same ground used to refine
    /// the mesh without bound. The refill read the sizes of the triangles it
    /// removed, the boundary's own chords and the slivers beside frozen rim
    /// vertices among them, and every repair took the minimum again: fourteen
    /// repairs of this nudge doubled the mesh and cut its shortest edge
    /// twentyfold. The refill follows requested sizes only now, and a mesh no
    /// adaptation has touched has none, so it comes back at the target.
    #[test]
    fn repeated_repairs_of_one_curve_do_not_refine_the_mesh() {
        let options = options(0.06);
        let from = Point2::new(0.0, 0.0);
        let to = Point2::new(0.05, 0.02);
        let topology = compile_topology(&hole_geometry(from), 1).unwrap();
        let plan = plan_for(&topology, from, None, options);
        let fresh = mesh_topology_plan(&plan, 10, options).unwrap();
        let (mesh, plan, counts) = drag_back_and_forth(fresh.clone(), plan, from, to, 14, options);
        assert_contract(&mesh, &plan, options);
        assert!(mesh.requested_sizes.is_empty());
        assert!(
            mesh.triangles.len() <= fresh.triangles.len() * 105 / 100,
            "{} triangles after {counts:?}, fresh {}",
            mesh.triangles.len(),
            fresh.triangles.len()
        );
        assert!(
            shortest_edge(&mesh) >= shortest_edge(&fresh) * 0.95,
            "shortest edge {} after {counts:?}, fresh {}",
            shortest_edge(&mesh),
            shortest_edge(&fresh)
        );
    }

    /// An adapted mesh repaired back and forth stays where adaptation put it:
    /// the requested sizes the refill copies are bounded by what the field
    /// asked for and carry nothing the refill itself produced.
    #[test]
    fn repeated_repairs_of_an_adapted_mesh_stay_at_the_requested_sizes() {
        let options = options(0.12);
        let from = Point2::new(0.0, 0.0);
        let to = Point2::new(0.05, 0.02);
        let topology = compile_topology(&hole_geometry(from), 1).unwrap();
        let plan = plan_for(&topology, from, None, options);
        let base = mesh_topology_plan(&plan, 10, options).unwrap();
        let adapted = adapt_around(&base, &plan, from, 0.04, options);
        assert!(adapted.triangles.len() > 2 * base.triangles.len());
        let (mesh, plan, counts) =
            drag_back_and_forth(adapted.clone(), plan, from, to, 12, options);
        assert_contract(&mesh, &plan, options);
        for count in &counts {
            assert!(
                count * 10 <= adapted.triangles.len() * 11
                    && count * 10 >= adapted.triangles.len() * 9,
                "{counts:?} against {}",
                adapted.triangles.len()
            );
        }
        assert!(shortest_edge(&mesh) >= shortest_edge(&adapted) * 0.95);
        assert_eq!(mesh.requested_sizes.len(), mesh.triangles.len());
        let requested = mesh.requested_sizes.iter().flatten().count();
        assert!(
            requested * 20 >= mesh.triangles.len() * 19,
            "{requested} of {} triangles carry a request",
            mesh.triangles.len()
        );
    }

    /// Kept triangles keep their requested size and the refill's triangles
    /// take the request of the removed triangle beneath them, so the next
    /// repair finds the band still asked for.
    #[test]
    fn a_repair_carries_requested_sizes_into_the_refilled_band() {
        let options = options(0.12);
        let from = Point2::new(0.0, 0.0);
        let to = Point2::new(0.05, 0.02);
        let fine = 0.04;
        let before_topology = compile_topology(&hole_geometry(from), 1).unwrap();
        let before_plan = plan_for(&before_topology, from, None, options);
        let base = mesh_topology_plan(&before_plan, 10, options).unwrap();
        let adapted = adapt_around(&base, &before_plan, from, fine, options);
        let after_topology = compile_topology(&hole_geometry(to), 2).unwrap();
        let after_plan = plan_for(&after_topology, to, None, options);
        let (carved, report) = carve(
            &adapted,
            &before_plan,
            &after_plan,
            &after_topology,
            options,
        );
        assert_eq!(carved.requested_sizes.len(), carved.triangles.len());
        let previous = adapted
            .triangles
            .iter()
            .enumerate()
            .map(|(index, triangle)| {
                (
                    triangle_keys_one(&adapted, triangle),
                    adapted.requested_size(index),
                )
            })
            .collect::<BTreeMap<_, _>>();
        // The ground the hole uncovered had no triangles before, so nothing
        // there has an opinion: those refill triangles carry no request and
        // come back at the target until adaptation looks at them.
        let (mut kept, mut inherited, mut uncovered) = (0, 0, 0);
        for (index, triangle) in carved.triangles.iter().enumerate() {
            match previous.get(&triangle_keys_one(&carved, triangle)) {
                Some(request) => {
                    kept += 1;
                    assert_eq!(carved.requested_size(index), *request);
                }
                None => {
                    let distance = (centroid(&carved, triangle) - from).norm();
                    let request = carved.requested_size(index);
                    if distance < 0.24 {
                        uncovered += 1;
                        assert_eq!(request, None, "{distance}");
                    } else if distance > 0.32 {
                        inherited += 1;
                        let request = request.expect("a refill triangle over removed ground");
                        assert!(
                            request >= fine && request <= options.target_edge_length,
                            "{request}"
                        );
                        if distance < 0.45 {
                            assert!(request <= fine * 1.0001, "{request}");
                        }
                    }
                }
            }
        }
        assert!(kept >= report.kept_triangles, "{kept} {report:?}");
        assert!(inherited > 0 && uncovered > 0, "{inherited} {uncovered}");
    }

    /// Without preserving adaptation the refill returns to the meshing target
    /// and carries no request, while kept triangles keep theirs.
    #[test]
    fn a_repair_without_preserving_adaptation_refills_at_the_target() {
        let options = options(0.12);
        let from = Point2::new(0.0, 0.0);
        let to = Point2::new(0.05, 0.02);
        let fine = 0.04;
        let before_topology = compile_topology(&hole_geometry(from), 1).unwrap();
        let before_plan = plan_for(&before_topology, from, None, options);
        let base = mesh_topology_plan(&before_plan, 10, options).unwrap();
        let adapted = adapt_around(&base, &before_plan, from, fine, options);
        let after_topology = compile_topology(&hole_geometry(to), 2).unwrap();
        let after_plan = plan_for(&after_topology, to, None, options);
        let (preserving, _) = carve(
            &adapted,
            &before_plan,
            &after_plan,
            &after_topology,
            options,
        );
        let (carved, report) = carve_topology_mesh(
            Arc::new(adapted.clone()),
            &before_plan,
            after_plan.clone(),
            Arc::new(after_topology),
            12,
            options,
            false,
        )
        .unwrap();
        assert_contract(&carved, &after_plan, options);
        let kept = triangle_keys(&adapted);
        let band = |mesh: &TriMesh| {
            mesh.triangles
                .iter()
                .enumerate()
                .filter(|(_, triangle)| !kept.contains(&triangle_keys_one(mesh, triangle)))
                .map(|(index, triangle)| (index, max_edge(mesh, triangle)))
                .collect::<Vec<_>>()
        };
        let coarse = band(&carved);
        let dense = band(&preserving);
        assert!(!coarse.is_empty() && !dense.is_empty());
        assert!(
            coarse
                .iter()
                .all(|(index, _)| carved.requested_size(*index).is_none())
        );
        let longest =
            |band: &[(usize, f64)]| band.iter().map(|(_, edge)| *edge).fold(0.0, f64::max);
        assert!(
            longest(&coarse) > longest(&dense) * 1.3,
            "longest refill edge {} against {} when preserving",
            longest(&coarse),
            longest(&dense)
        );
        assert!(
            coarse.len() * 2 < dense.len(),
            "{} against {}",
            coarse.len(),
            dense.len()
        );
        assert_eq!(carved.requested_sizes.len(), carved.triangles.len());
        assert!(
            carved.requested_sizes[..report.kept_triangles]
                .iter()
                .all(Option::is_some)
        );
    }

    /// A refilled band is ordinary mesh to adaptation: once the field no
    /// longer wants it fine, its vertices collapse like any other, so a
    /// repair can never leave density behind that adaptation cannot remove.
    #[test]
    fn adaptation_coarsens_a_repaired_band_it_no_longer_wants_fine() {
        let options = options(0.12);
        let from = Point2::new(0.0, 0.0);
        let to = Point2::new(0.05, 0.02);
        let fine = 0.04;
        let before_topology = compile_topology(&hole_geometry(from), 1).unwrap();
        let before_plan = plan_for(&before_topology, from, None, options);
        let base = mesh_topology_plan(&before_plan, 10, options).unwrap();
        let adapted = adapt_around(&base, &before_plan, from, fine, options);
        let after_topology = compile_topology(&hole_geometry(to), 2).unwrap();
        let after_plan = plan_for(&after_topology, to, None, options);
        let (carved, _) = carve(
            &adapted,
            &before_plan,
            &after_plan,
            &after_topology,
            options,
        );
        let coarse: Arc<dyn MeshSizeField> =
            Arc::new(move |_: Point2, _: RegionId| options.target_edge_length);
        let mut mesh = carved.clone();
        let mut state = MeshAdaptationState::from_mesh(&mesh);
        for _ in 0..3 {
            let result = adapt_from(&mesh, &after_plan, state, coarse.clone(), fine, options);
            state = result.state;
            mesh = result.mesh;
        }
        assert!(
            mesh.triangles.len() * 10 <= carved.triangles.len() * 6,
            "{} triangles after coarsening {}",
            mesh.triangles.len(),
            carved.triangles.len()
        );
        assert!(
            mesh.requested_sizes
                .iter()
                .all(|request| *request == Some(options.target_edge_length))
        );
    }

    /// The inclusion of a user's scene: a closed two-sided curve between two
    /// active materials, at its authored controls except the fifth.
    fn inclusion_geometry(fifth: Point2) -> TopologyGeometry {
        let mut controls = vec![
            Point2::new(0.37743732302351307, -0.41795853199421185),
            Point2::new(0.3882473077514753, 0.18781370939737263),
            Point2::new(0.3682460466589229, 0.43497683417714367),
            Point2::new(0.0059816045202802925, 0.657774891371061),
            Point2::new(-0.30916722337740166, 0.7926036509316865),
            Point2::new(-0.5397094743163924, 0.3830903697048762),
            Point2::new(-0.13801587861941852, 0.25352651762887213),
            Point2::new(-0.21215140207099628, -0.042954644373032855),
            Point2::new(-0.11644986653581002, -0.13581191948070834),
        ];
        controls[5] = fifth;
        let spline = PeriodicCubicSpline::new_with_multiplicities(
            controls,
            vec![1.0; 7],
            vec![3, 1, 1, 1, 1, 1, 1],
        )
        .unwrap();
        TopologyGeometry {
            domain: crate::DomainRect {
                min_x: -2.0843804802813337,
                max_x: 1.9812315400322542,
                min_y: -1.2682000961099806,
                max_y: 1.8085514175081223,
            },
            curves: vec![
                TopologyCurve::new(
                    CurveId(9),
                    CurveSpline::Closed(spline),
                    spans(67, 7, SpanBehavior::REFLECTING),
                )
                .unwrap(),
            ],
            ..TopologyGeometry::default()
        }
    }

    fn worst_angle(mesh: &TriMesh) -> f64 {
        (0..mesh.triangles.len())
            .map(|index| mesh.triangle_quality(index).unwrap().minimum_angle_degrees)
            .fold(f64::INFINITY, f64::min)
    }

    /// A user adapted this inclusion, then nudged one control by 3 mm. With
    /// requests finer than the mesh around the band, the refill refined the
    /// frozen cavity towards them: 3,692 triangles inserted for 534 removed
    /// and 115 of them under one degree, because a sliver across nearly
    /// collinear rim vertices can be neither flipped nor split. Requests are
    /// a ceiling; the density that was there is the floor.
    #[test]
    fn a_repair_never_refines_a_cavity_finer_than_its_rim() {
        let options = MeshingOptions {
            target_edge_length: 0.08,
            curve_tolerance: 5.0e-4,
            ..MeshingOptions::default()
        };
        let rest = Point2::new(-0.5397094743163924, 0.3830903697048762);
        let inside = Point2::new(0.1, 0.2);
        let topology = compile_topology(&inclusion_geometry(rest), 1).unwrap();
        let plan = plan_for(&topology, inside, Some(RegionId(2)), options);
        let fresh = mesh_topology_plan(&plan, 10, options).unwrap();
        let nudged = rest + Point2::new(0.003, 0.0015);
        let moved = compile_topology(&inclusion_geometry(nudged), 2).unwrap();
        let next = plan_for(&moved, inside, Some(RegionId(2)), options);
        let constant = vec![Some(0.04); fresh.triangles.len()];
        let stepped = fresh
            .triangles
            .iter()
            .map(|triangle| Some((0.6 * max_edge(&fresh, triangle)).clamp(0.02, 0.16)))
            .collect::<Vec<_>>();
        for (name, requests) in [("constant", constant), ("stepped", stepped)] {
            let mut mesh = fresh.clone();
            mesh.requested_sizes = requests;
            let (carved, report) = carve(&mesh, &plan, &next, &moved, options);
            assert_contract(&carved, &next, options);
            assert!(
                worst_angle(&carved) >= worst_angle(&fresh) - 1.0,
                "{name}: worst angle {} after the repair, {} before",
                worst_angle(&carved),
                worst_angle(&fresh)
            );
            assert!(
                report.inserted_triangles <= report.removed_triangles * 2,
                "{name}: {report:?}"
            );
            if name == "constant" {
                assert!(
                    carved.requested_sizes[report.kept_triangles..]
                        .iter()
                        .flatten()
                        .all(|request| *request == 0.04)
                );
            }
        }
    }

    fn flat_mesh(points: &[[f64; 2]], triangles: &[[usize; 3]]) -> TriMesh {
        TriMesh {
            geometry_revision: 1,
            mesh_revision: 1,
            vertices: points
                .iter()
                .map(|[x, y]| MeshVertex {
                    point: Point2::new(*x, *y),
                    boundary: None,
                    trace: None,
                })
                .collect(),
            triangles: triangles
                .iter()
                .map(|vertices| MeshTriangle {
                    vertices: *vertices,
                    region: RegionId(1),
                })
                .collect(),
            boundary_edges: vec![],
            quality: MeshQuality {
                minimum_angle_degrees: 0.0,
                maximum_edge_length: 1.0,
            },
            requested_sizes: vec![],
        }
    }

    /// A refill that leaves a degenerate element is refused with the count,
    /// the worst angle and where it is, so the fallback names the defect.
    #[test]
    fn a_repair_with_degenerate_elements_is_refused_by_name() {
        let mesh = flat_mesh(
            &[
                [0.0, 0.0],
                [1.0, 0.0],
                [0.0, 1.0],
                [2.0, 0.0],
                [1.5, 1.0],
                [0.5, 1.0e-5],
            ],
            &[[0, 1, 2], [1, 3, 4], [0, 1, 5]],
        );
        let options = MeshingOptions::default();
        assert!(verify_repair_quality(&mesh, 3, 45.0, options).is_ok());
        assert!(verify_repair_quality(&mesh, 2, 45.0, options).is_err());
        let error = verify_repair_quality(&mesh, 1, 45.0, options).unwrap_err();
        let MeshError::DegenerateRepair {
            count,
            worst_angle_degrees,
            floor_degrees,
            point,
        } = error.clone()
        else {
            panic!("{error:?}");
        };
        assert_eq!(count, 1);
        assert!(worst_angle_degrees < 0.01, "{worst_angle_degrees}");
        assert_eq!(floor_degrees, 9.0);
        assert!((point - Point2::new(0.5, 1.0e-5 / 3.0)).norm() < 1.0e-9);
        let message = error.to_string();
        assert!(message.contains("1 degenerate element"), "{message}");
        assert!(message.contains("9.0° floor"), "{message}");
        // A mesh that already had a poor angle lowers the floor with it.
        assert!(verify_repair_quality(&mesh, 1, 0.001, options).is_ok());
    }

    /// An adapted mesh may already exceed the mesher's caps; they bound what
    /// a repair adds, not what it inherits.
    #[test]
    fn a_repair_of_a_mesh_beyond_the_mesher_caps_still_fits() {
        let scene = moved_hole(Point2::new(0.0, 0.0), Point2::new(0.04, 0.03), 0.06);
        assert!(scene.before.vertices.len() > 1_000 && scene.before.triangles.len() > 2_000);
        let capped = MeshingOptions {
            max_vertices: 1_000,
            max_triangles: 2_000,
            ..scene.options
        };
        let (carved, report) = carve(
            &scene.before,
            &scene.before_plan,
            &scene.after_plan,
            &scene.after_topology,
            capped,
        );
        assert_contract(&carved, &scene.after_plan, scene.options);
        assert_same_areas(&carved, &scene.fresh);
        assert!(report.kept_triangles > 0, "{report:?}");
    }
}
