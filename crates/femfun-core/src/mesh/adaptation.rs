use super::*;
use std::sync::Arc;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshUpdateReport {
    pub local_attempted: bool,
    pub used_local: bool,
    pub fallback_reason: Option<String>,
    pub original_triangles: usize,
    /// Same vertex identities AND exactly the same coordinates.
    pub preserved_triangles: usize,
    pub preserved_connectivity: usize,
    pub moved_vertices: usize,
    pub inserted_vertices: usize,
    pub collapsed_vertices: usize,
    pub repair_triangles: usize,
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
    Expand(usize),
    Smooth {
        pass: usize,
        index: usize,
    },
    Move(usize),
    Check(usize),
    Boundary(usize),
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
    options: MeshingOptions,
    phase: Phase,
    builder: Option<MeshBuilder>,
    job: Option<MeshingJob>,
    report: MeshUpdateReport,
    work: usize,
    displacement: Vec<Point2>,
    next_displacement: Vec<Point2>,
    moving: Vec<Point2>,
    mobile: Vec<bool>,
    smooth_vertices: Vec<usize>,
    allowed: Vec<bool>,
    radius: f64,
    next_boundary: Vec<Option<usize>>,
    seeds: BTreeMap<u64, usize>,
    loop_seeds: Vec<usize>,
    old_keys: BTreeSet<[usize; 3]>,
    output: Option<TriMesh>,
    remap: Vec<usize>,
    compact_vertices: Vec<MeshVertex>,
    boundary_splits: usize,
}

fn key(mut vertices: [usize; 3]) -> [usize; 3] {
    vertices.sort();
    vertices
}

impl MeshUpdateJob {
    pub fn new(
        previous: Option<(Arc<TriMesh>, Scene)>,
        scene: Scene,
        revision: u64,
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
            options,
            phase: Phase::Done,
            builder: None,
            job: None,
            report: MeshUpdateReport::default(),
            work: 0,
            displacement: vec![],
            next_displacement: vec![],
            moving: vec![],
            mobile: vec![],
            smooth_vertices: vec![],
            allowed: vec![],
            radius: 0.0,
            next_boundary: vec![],
            seeds: BTreeMap::new(),
            loop_seeds: vec![],
            old_keys: BTreeSet::new(),
            output: None,
            remap: vec![],
            compact_vertices: vec![],
            boundary_splits: 0,
        };
        if let Some(mesh) = &result.previous {
            result.report.local_attempted = true;
            result.report.original_triangles = mesh.triangles.len();
            let compatible = result
                .previous_scene
                .obstacles
                .iter()
                .chain(&result.scene.obstacles)
                .all(|loop_| matches!(loop_.role, LoopRole::Hole { .. }))
                && result.previous_scene.obstacles.len() == result.scene.obstacles.len()
                && result.scene.obstacles.iter().all(|new| {
                    result.previous_scene.obstacles.iter().any(|old| {
                        old.id == new.id
                            && old.role == new.role
                            && old.spline.intervals() == new.spline.intervals()
                    })
                });
            if compatible {
                let mut builder = MeshBuilder::new(options);
                builder.loop_ids = result
                    .scene
                    .obstacles
                    .iter()
                    .map(|loop_| loop_.id)
                    .collect();
                builder.loop_roles = result
                    .scene
                    .obstacles
                    .iter()
                    .map(|loop_| loop_.role)
                    .collect();
                result.builder = Some(builder);
                result.phase =
                    Phase::Validate(Box::new(ValidationJob::new(result.scene.clone(), revision)));
            } else {
                result.fallback("loop topology, role, or knot vector changed".into());
            }
        } else {
            result.full();
        }
        result
    }

    fn full(&mut self) {
        self.job = Some(MeshingJob::new(
            self.scene.clone(),
            self.revision,
            self.options,
        ));
        self.phase = Phase::Full;
    }

    fn fallback(&mut self, reason: String) {
        self.report.fallback_reason = Some(reason);
        self.report.used_local = false;
        self.builder = None;
        self.full();
    }

    pub fn phase(&self) -> &'static str {
        match &self.phase {
            Phase::Full => self.job.as_ref().unwrap().phase(),
            Phase::Validate(_) => "Validating edit",
            Phase::Vertices(_) | Phase::Triangles(_) | Phase::Edges(_) => "Reusing mesh",
            Phase::Region(_) | Phase::Expand(_) => "Selecting repair region",
            Phase::Smooth { .. } | Phase::Move(_) => "Moving local mesh",
            Phase::Coarsen(_) => "Coarsening local mesh",
            Phase::Repair => self.job.as_ref().unwrap().phase(),
            Phase::Check(_) | Phase::Boundary(_) | Phase::Loops { .. } => "Checking local geometry",
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
                self.fallback("local repair work limit reached".into());
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
                Err(error) => self.fallback(error.to_string()),
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
                    self.phase = Phase::Vertices(0);
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
                if let Some(BoundaryPoint {
                    label: BoundaryLabel::Obstacle(id),
                    parameter,
                }) = vertex.boundary
                {
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
                    if !parameter.is_finite() {
                        return Err(MeshError::Topology("invalid boundary parameter"));
                    }
                    if (after.spline.evaluate(parameter) - before.spline.evaluate(parameter)).norm()
                        > 1e-12
                    {
                        delta = after.spline.evaluate(parameter) - vertex.point;
                        if delta.norm() > 2.0 * self.options.target_edge_length {
                            return Err(MeshError::Topology(
                                "boundary motion exceeds local repair radius",
                            ));
                        }
                        self.moving.push(vertex.point);
                        self.radius = self.radius.max(4.0 * delta.norm());
                    }
                }
                self.builder
                    .as_mut()
                    .unwrap()
                    .add_vertex(vertex.point, vertex.boundary)?;
                self.displacement.push(delta);
                self.next_displacement.push(delta);
                self.mobile.push(delta != Point2::default());
                self.allowed.push(delta != Point2::default());
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
                    self.radius = self.radius.max(4.0 * self.options.target_edge_length);
                    self.phase = Phase::Region(0);
                    return Ok(None);
                }
                let edge = old.boundary_edges[i];
                if edge.vertices.iter().any(|v| *v >= self.next_boundary.len()) {
                    return Err(MeshError::Topology("invalid source boundary"));
                }
                if self.next_boundary[edge.vertices[0]]
                    .replace(edge.vertices[1])
                    .is_some()
                {
                    return Err(MeshError::Topology("non-manifold source boundary"));
                }
                let label = match edge.label {
                    BoundaryLabel::Outer(_) => 0,
                    BoundaryLabel::Obstacle(id) => id.0,
                    BoundaryLabel::MaterialInterface(id) => id.0,
                    BoundaryLabel::Wall { loop_id, .. } => loop_id.0,
                };
                self.seeds.entry(label).or_insert(edge.vertices[0]);
                self.builder.as_mut().unwrap().add_boundary_edge(edge);
                self.phase = Phase::Edges(i + 1);
            }
            Phase::Region(i) => {
                let b = self.builder.as_mut().unwrap();
                if i == b.vertices.len() {
                    self.phase = Phase::Expand(0);
                    return Ok(None);
                }
                if b.vertices[i].boundary.is_none()
                    && self
                        .moving
                        .iter()
                        .any(|p| (b.point(i) - *p).norm() < self.radius)
                {
                    self.mobile[i] = true;
                    self.allowed[i] = true;
                    self.smooth_vertices.push(i);
                }
                self.phase = Phase::Region(i + 1);
            }
            Phase::Expand(i) => {
                let b = self.builder.as_mut().unwrap();
                if i == b.triangles.len() {
                    b.repair_region = Some(std::mem::take(&mut self.allowed));
                    self.phase = Phase::Smooth { pass: 0, index: 0 };
                    return Ok(None);
                }
                let triangle = b.triangles[i];
                if triangle.vertices.iter().any(|v| self.mobile[*v]) {
                    self.report.repair_triangles += 1;
                    if self.report.repair_triangles > (old.triangles.len() / 3).max(256) {
                        return Err(MeshError::Topology(
                            "edit affects too much of the mesh for local repair",
                        ));
                    }
                    for v in triangle.vertices {
                        self.allowed[v] = true;
                    }
                }
                self.phase = Phase::Expand(i + 1);
            }
            Phase::Smooth { pass, index } => {
                if index == self.smooth_vertices.len() {
                    std::mem::swap(&mut self.displacement, &mut self.next_displacement);
                    self.phase = if pass == 23 {
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
                    self.phase = Phase::Coarsen(0);
                    return Ok(None);
                }
                let edge = b.boundary_edges[i];
                if let BoundaryLabel::Obstacle(id) = edge.label {
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
                        if self.boundary_splits >= 256 {
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
            Phase::Coarsen(i) => {
                let b = self.builder.as_mut().unwrap();
                if i == self.smooth_vertices.len() {
                    self.loop_seeds = self.seeds.values().copied().collect();
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
                if loop_index == self.loop_seeds.len() {
                    let mut builder = self.builder.take().unwrap();
                    builder.options.max_refinement_steps =
                        builder.options.max_refinement_steps.min(512);
                    self.job = Some(MeshingJob {
                        scene: self.scene.clone(),
                        geometry_revision: self.revision,
                        builder,
                        legalization_work: 0,
                        state: MeshingJobState::Legalize,
                    });
                    self.phase = Phase::Repair;
                    return Ok(None);
                }
                let start = self.loop_seeds[loop_index];
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
    if adjacent.len() != 1 {
        return Err(MeshError::Topology("non-manifold boundary edge"));
    }
    let opposite = adjacent[0].1;
    let [a, c] = edge.vertices.map(|v| b.point(v));
    let d = b.point(opposite);
    let sign = orient2d(a, c, d);
    if orient2d(a, point, d) != sign || orient2d(point, c, d) != sign {
        return Err(MeshError::Topology(
            "curved boundary refinement inverted an element",
        ));
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
