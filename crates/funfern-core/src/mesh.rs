use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::{
    BACKGROUND_REGION, InternalBoundaryId, LoopRole, ObstacleId, OpenSampler, Point2,
    PredicateSign, RegionId, Sample, Sampler, SamplingOptions, ValidationIssue, ValidationJob,
    incircle, orient2d,
};
use crate::{Scene, WORLD_TOLERANCE};
mod adaptation;
pub use adaptation::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OuterSide {
    Bottom,
    Right,
    Top,
    Left,
}

impl OuterSide {
    pub const ALL: [Self; 4] = [Self::Bottom, Self::Right, Self::Top, Self::Left];

    pub const fn index(self) -> usize {
        match self {
            Self::Bottom => 0,
            Self::Right => 1,
            Self::Top => 2,
            Self::Left => 3,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Bottom => "Bottom",
            Self::Right => "Right",
            Self::Top => "Top",
            Self::Left => "Left",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryLabel {
    Outer(OuterSide),
    Obstacle(ObstacleId),
    MaterialInterface(ObstacleId),
    Wall {
        loop_id: ObstacleId,
        side: BoundarySide,
    },
    InternalBoundary {
        id: InternalBoundaryId,
        side: InternalBoundarySide,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundarySide {
    Exterior,
    Interior,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InternalBoundarySide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoundaryPoint {
    pub label: BoundaryLabel,
    pub parameter: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshVertex {
    pub point: Point2,
    /// A boundary vertex can be shared by two outer-side labels at a corner;
    /// edge labels remain authoritative in that case.
    pub boundary: Option<BoundaryPoint>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshTriangle {
    pub vertices: [usize; 3],
    pub region: RegionId,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoundaryEdge {
    pub vertices: [usize; 2],
    pub label: BoundaryLabel,
    /// Continuous parameters along this oriented edge. The last obstacle edge
    /// ends at one period rather than wrapping its second value to zero.
    pub parameters: [f64; 2],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshQuality {
    pub minimum_angle_degrees: f64,
    pub maximum_edge_length: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TriMesh {
    pub geometry_revision: u64,
    pub vertices: Vec<MeshVertex>,
    pub triangles: Vec<MeshTriangle>,
    pub boundary_edges: Vec<BoundaryEdge>,
    pub quality: MeshQuality,
}

impl TriMesh {
    pub fn triangle_quality(&self, index: usize) -> Option<MeshQuality> {
        let triangle = self.triangles.get(index)?;
        let points = triangle.vertices.map(|vertex| self.vertices[vertex].point);
        let sides = [
            (points[1] - points[2]).norm(),
            (points[2] - points[0]).norm(),
            (points[0] - points[1]).norm(),
        ];
        Some(MeshQuality {
            minimum_angle_degrees: [
                angle_from_sides(sides[0], sides[1], sides[2]),
                angle_from_sides(sides[1], sides[2], sides[0]),
                angle_from_sides(sides[2], sides[0], sides[1]),
            ]
            .into_iter()
            .fold(f64::INFINITY, f64::min),
            maximum_edge_length: sides.into_iter().fold(0.0, f64::max),
        })
    }

    pub fn poor_triangles(&self, minimum_angle_degrees: f64) -> Vec<usize> {
        self.triangles
            .iter()
            .enumerate()
            .filter_map(|(index, _)| {
                (self.triangle_quality(index)?.minimum_angle_degrees < minimum_angle_degrees)
                    .then_some(index)
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshingOptions {
    pub curve_tolerance: f64,
    pub target_edge_length: f64,
    pub minimum_angle_degrees: f64,
    pub max_vertices: usize,
    pub max_triangles: usize,
    pub max_refinement_steps: usize,
}

impl Default for MeshingOptions {
    fn default() -> Self {
        Self {
            curve_tolerance: 5.0e-4,
            target_edge_length: 0.18,
            minimum_angle_degrees: 18.0,
            max_vertices: 12_000,
            max_triangles: 24_000,
            max_refinement_steps: 8_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum MeshError {
    InvalidOptions,
    InvalidGeometry(ValidationIssue),
    Sampling(ObstacleId),
    Capacity { vertices: usize, triangles: usize },
    Topology(&'static str),
    RefinementLimit(MeshQuality),
}

impl std::fmt::Display for MeshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidOptions => write!(f, "Invalid meshing options"),
            Self::InvalidGeometry(issue) => write!(f, "Geometry is not accepted: {issue}"),
            Self::Sampling(id) => write!(f, "Obstacle {} exceeded the mesh sampling budget", id.0),
            Self::Capacity {
                vertices,
                triangles,
            } => write!(
                f,
                "Mesh capacity reached at {vertices} vertices and {triangles} triangles"
            ),
            Self::Topology(reason) => write!(f, "Could not construct a constrained mesh: {reason}"),
            Self::RefinementLimit(quality) => write!(
                f,
                "Refinement limit reached (minimum angle {:.1}°, maximum edge {:.3})",
                quality.minimum_angle_degrees, quality.maximum_edge_length
            ),
        }
    }
}

impl std::error::Error for MeshError {}

#[derive(Clone)]
struct Polygon {
    vertices: Vec<usize>,
}

#[derive(Clone)]
struct TriangulationDomain {
    region: RegionId,
    outer: Vec<usize>,
    holes: Vec<Vec<usize>>,
}

struct MeshBuilder {
    vertices: Vec<MeshVertex>,
    triangles: Vec<MeshTriangle>,
    boundary_edges: Vec<BoundaryEdge>,
    boundary_keys: BTreeSet<(usize, usize)>,
    domain_loops: Vec<Polygon>,
    interior_loops: Vec<Option<Polygon>>,
    loop_ids: Vec<ObstacleId>,
    loop_roles: Vec<LoopRole>,
    internal_boundary_ids: Vec<InternalBoundaryId>,
    internal_boundary_regions: Vec<RegionId>,
    internal_samples: Vec<Vec<Sample>>,
    internal_chains: Vec<Vec<(usize, f64)>>,
    internal_trace_vertices: BTreeSet<usize>,
    domains: Vec<TriangulationDomain>,
    options: MeshingOptions,
    adjacency: BTreeMap<(usize, usize), Vec<(usize, usize)>>,
    dirty_edges: VecDeque<(usize, usize)>,
    queued_edges: BTreeSet<(usize, usize)>,
    bad_triangles: BTreeSet<(u64, usize)>,
    scores: Vec<Option<u64>>,
    stats: MeshingStats,
    incident: Vec<BTreeSet<usize>>,
    repair_region: Option<Vec<bool>>,
}

/// Deterministic counters for profiling without introducing a clock dependency.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MeshingStats {
    pub work_units: usize,
    pub quality_evaluations: usize,
    pub edge_tests: usize,
    pub edge_flips: usize,
    pub refinement_insertions: usize,
}

fn edge_key(a: usize, b: usize) -> (usize, usize) {
    if a < b { (a, b) } else { (b, a) }
}

fn polygon_area(points: impl Iterator<Item = Point2>) -> f64 {
    let points: Vec<_> = points.collect();
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| a.cross(*b))
        .sum::<f64>()
        * 0.5
}

fn on_segment(point: Point2, a: Point2, b: Point2) -> bool {
    orient2d(a, b, point) == PredicateSign::Zero
        && point.x >= a.x.min(b.x)
        && point.x <= a.x.max(b.x)
        && point.y >= a.y.min(b.y)
        && point.y <= a.y.max(b.y)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentRelation {
    Disjoint,
    Touching,
    ProperIntersection,
    Overlapping,
}

/// Exact-sign segment classification. Bounding comparisons are exact comparisons
/// of the supplied floating-point coordinates; no geometric epsilon is used.
pub fn segment_relation(a: Point2, b: Point2, c: Point2, d: Point2) -> SegmentRelation {
    if a.x.max(b.x) < c.x.min(d.x)
        || c.x.max(d.x) < a.x.min(b.x)
        || a.y.max(b.y) < c.y.min(d.y)
        || c.y.max(d.y) < a.y.min(b.y)
    {
        return SegmentRelation::Disjoint;
    }
    let ab_c = orient2d(a, b, c);
    let ab_d = orient2d(a, b, d);
    let cd_a = orient2d(c, d, a);
    let cd_b = orient2d(c, d, b);
    if ab_c == PredicateSign::Zero
        && ab_d == PredicateSign::Zero
        && cd_a == PredicateSign::Zero
        && cd_b == PredicateSign::Zero
    {
        let overlap_x = a.x.max(b.x).min(c.x.max(d.x)) - a.x.min(b.x).max(c.x.min(d.x));
        let overlap_y = a.y.max(b.y).min(c.y.max(d.y)) - a.y.min(b.y).max(c.y.min(d.y));
        let overlap = if (a.x - b.x).abs() >= (a.y - b.y).abs() {
            overlap_x
        } else {
            overlap_y
        };
        return if overlap > 0.0 {
            SegmentRelation::Overlapping
        } else if overlap == 0.0 {
            SegmentRelation::Touching
        } else {
            SegmentRelation::Disjoint
        };
    }
    if ab_c != PredicateSign::Zero
        && ab_d != PredicateSign::Zero
        && cd_a != PredicateSign::Zero
        && cd_b != PredicateSign::Zero
        && ab_c != ab_d
        && cd_a != cd_b
    {
        return SegmentRelation::ProperIntersection;
    }
    if (ab_c == PredicateSign::Zero && on_segment(c, a, b))
        || (ab_d == PredicateSign::Zero && on_segment(d, a, b))
        || (cd_a == PredicateSign::Zero && on_segment(a, c, d))
        || (cd_b == PredicateSign::Zero && on_segment(b, c, d))
    {
        SegmentRelation::Touching
    } else {
        SegmentRelation::Disjoint
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolygonLocation {
    Outside,
    Inside,
    Boundary,
}

pub fn locate_in_polygon(point: Point2, polygon: &[Point2]) -> PolygonLocation {
    let mut inside = false;
    for (a, b) in polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
    {
        if on_segment(point, *a, *b) {
            return PolygonLocation::Boundary;
        }
        if (a.y > point.y) != (b.y > point.y) {
            let side = orient2d(*a, *b, point);
            if (b.y > a.y && side == PredicateSign::Positive)
                || (b.y < a.y && side == PredicateSign::Negative)
            {
                inside = !inside;
            }
        }
    }
    if inside {
        PolygonLocation::Inside
    } else {
        PolygonLocation::Outside
    }
}

fn point_in_triangle(point: Point2, triangle: [Point2; 3]) -> PolygonLocation {
    if point.x < triangle[0].x.min(triangle[1].x).min(triangle[2].x)
        || point.x > triangle[0].x.max(triangle[1].x).max(triangle[2].x)
        || point.y < triangle[0].y.min(triangle[1].y).min(triangle[2].y)
        || point.y > triangle[0].y.max(triangle[1].y).max(triangle[2].y)
    {
        return PolygonLocation::Outside;
    }
    let signs = triangle.map_with_index(|index, a| orient2d(a, triangle[(index + 1) % 3], point));
    if signs.contains(&PredicateSign::Negative) {
        PolygonLocation::Outside
    } else if signs.contains(&PredicateSign::Zero) {
        PolygonLocation::Boundary
    } else {
        PolygonLocation::Inside
    }
}

trait ArrayMapWithIndex<T, const N: usize> {
    fn map_with_index<U>(self, map: impl FnMut(usize, T) -> U) -> [U; N];
}

impl<T, const N: usize> ArrayMapWithIndex<T, N> for [T; N] {
    fn map_with_index<U>(self, mut map: impl FnMut(usize, T) -> U) -> [U; N] {
        let mut index = 0;
        self.map(|value| {
            let result = map(index, value);
            index += 1;
            result
        })
    }
}

impl MeshBuilder {
    fn new(options: MeshingOptions) -> Self {
        Self {
            vertices: vec![],
            triangles: vec![],
            boundary_edges: vec![],
            boundary_keys: BTreeSet::new(),
            domain_loops: vec![],
            interior_loops: vec![],
            loop_ids: vec![],
            loop_roles: vec![],
            internal_boundary_ids: vec![],
            internal_boundary_regions: vec![],
            internal_samples: vec![],
            internal_chains: vec![],
            internal_trace_vertices: BTreeSet::new(),
            domains: vec![],
            options,
            adjacency: BTreeMap::new(),
            dirty_edges: VecDeque::new(),
            queued_edges: BTreeSet::new(),
            bad_triangles: BTreeSet::new(),
            scores: vec![],
            stats: MeshingStats::default(),
            incident: vec![],
            repair_region: None,
        }
    }

    fn queue_edge(&mut self, edge: (usize, usize)) {
        if self.queued_edges.insert(edge) {
            self.dirty_edges.push_back(edge);
        }
    }

    fn register_triangle(&mut self, index: usize) {
        let triangle = self.triangles[index];
        for vertex in triangle.vertices {
            self.incident[vertex].insert(index);
        }
        for opposite in 0..3 {
            let edge = edge_key(
                triangle.vertices[(opposite + 1) % 3],
                triangle.vertices[(opposite + 2) % 3],
            );
            self.adjacency
                .entry(edge)
                .or_default()
                .push((index, triangle.vertices[opposite]));
            self.queue_edge(edge);
        }
        let quality = self.triangle_quality(triangle);
        self.stats.quality_evaluations += 1;
        let touches_trace = triangle
            .vertices
            .iter()
            .any(|vertex| self.internal_trace_vertices.contains(vertex));
        let long = quality.maximum_edge_length > self.options.target_edge_length * 1.05;
        let narrow = !touches_trace
            && quality.minimum_angle_degrees + 1.0e-9 < self.options.minimum_angle_degrees;
        if long || narrow {
            // Nonnegative IEEE floats have the same ordering as their bit patterns.
            let score = (quality.maximum_edge_length / self.options.target_edge_length)
                .max(if touches_trace {
                    0.0
                } else {
                    self.options.minimum_angle_degrees / quality.minimum_angle_degrees
                })
                .to_bits();
            self.scores[index] = Some(score);
            self.bad_triangles.insert((score, index));
        }
    }

    fn unregister_triangle(&mut self, index: usize) {
        let old = self.triangles[index];
        for vertex in old.vertices {
            self.incident[vertex].remove(&index);
        }
        for opposite in 0..3 {
            let edge = edge_key(
                old.vertices[(opposite + 1) % 3],
                old.vertices[(opposite + 2) % 3],
            );
            let sides = self.adjacency.get_mut(&edge).expect("registered triangle");
            sides.retain(|(owner, _)| *owner != index);
            if sides.is_empty() {
                self.adjacency.remove(&edge);
            }
        }
        if let Some(score) = self.scores[index].take() {
            self.bad_triangles.remove(&(score, index));
        }
    }

    fn replace_triangle(&mut self, index: usize, triangle: MeshTriangle) {
        self.unregister_triangle(index);
        self.triangles[index] = triangle;
        self.register_triangle(index);
    }

    fn push_triangle(&mut self, triangle: MeshTriangle) -> Result<(), MeshError> {
        if self.triangles.len() >= self.options.max_triangles {
            return Err(self.capacity_error());
        }
        let index = self.triangles.len();
        self.triangles.push(triangle);
        self.scores.push(None);
        self.register_triangle(index);
        Ok(())
    }

    fn add_vertex(
        &mut self,
        point: Point2,
        boundary: Option<BoundaryPoint>,
    ) -> Result<usize, MeshError> {
        if self.vertices.len() >= self.options.max_vertices {
            return Err(self.capacity_error());
        }
        let index = self.vertices.len();
        self.vertices.push(MeshVertex { point, boundary });
        self.incident.push(BTreeSet::new());
        if let Some(region) = &mut self.repair_region {
            region.push(true);
        }
        Ok(index)
    }

    fn add_boundary_edge(&mut self, edge: BoundaryEdge) {
        self.boundary_keys
            .insert(edge_key(edge.vertices[0], edge.vertices[1]));
        self.boundary_edges.push(edge);
    }

    fn capacity_error(&self) -> MeshError {
        MeshError::Capacity {
            vertices: self.vertices.len(),
            triangles: self.triangles.len(),
        }
    }

    fn point(&self, index: usize) -> Point2 {
        self.vertices[index].point
    }

    fn ccw_triangle(
        &self,
        vertices: [usize; 3],
        region: RegionId,
    ) -> Result<MeshTriangle, MeshError> {
        match orient2d(
            self.point(vertices[0]),
            self.point(vertices[1]),
            self.point(vertices[2]),
        ) {
            PredicateSign::Positive => Ok(MeshTriangle { vertices, region }),
            PredicateSign::Negative => Ok(MeshTriangle {
                vertices: [vertices[0], vertices[2], vertices[1]],
                region,
            }),
            PredicateSign::Zero => Err(MeshError::Topology("zero-area triangle")),
        }
    }

    fn add_outer(&mut self) -> Result<Polygon, MeshError> {
        let per_side = (2.0 / self.options.target_edge_length).ceil() as usize;
        let per_side = per_side.max(1);
        if per_side > self.options.max_vertices / 4 {
            return Err(self.capacity_error());
        }
        let sides = [
            (
                Point2::new(-1.0, -1.0),
                Point2::new(1.0, -1.0),
                OuterSide::Bottom,
            ),
            (
                Point2::new(1.0, -1.0),
                Point2::new(1.0, 1.0),
                OuterSide::Right,
            ),
            (
                Point2::new(1.0, 1.0),
                Point2::new(-1.0, 1.0),
                OuterSide::Top,
            ),
            (
                Point2::new(-1.0, 1.0),
                Point2::new(-1.0, -1.0),
                OuterSide::Left,
            ),
        ];
        let mut vertices = vec![];
        let mut edge_metadata = vec![];
        for (start, end, side) in sides {
            for index in 0..per_side {
                let parameter = index as f64 / per_side as f64;
                let vertex = self.add_vertex(
                    start.lerp(end, parameter),
                    Some(BoundaryPoint {
                        label: BoundaryLabel::Outer(side),
                        parameter,
                    }),
                )?;
                vertices.push(vertex);
                edge_metadata.push((
                    BoundaryLabel::Outer(side),
                    parameter,
                    (index + 1) as f64 / per_side as f64,
                ));
            }
        }
        for index in 0..vertices.len() {
            let (label, start, end) = edge_metadata[index];
            self.add_boundary_edge(BoundaryEdge {
                vertices: [vertices[index], vertices[(index + 1) % vertices.len()]],
                label,
                parameters: [start, end],
            });
        }
        Ok(Polygon { vertices })
    }

    fn add_obstacle(
        &mut self,
        obstacle: &crate::Obstacle,
        sampled: Vec<Sample>,
    ) -> Result<(Polygon, Option<Polygon>), MeshError> {
        let mut points = Vec::new();
        for span in sampled.windows(2) {
            let length = (span[1].point - span[0].point).norm();
            let pieces = (length / self.options.target_edge_length).ceil().max(1.0) as usize;
            if pieces
                > self
                    .options
                    .max_vertices
                    .saturating_sub(self.vertices.len() + points.len())
            {
                return Err(self.capacity_error());
            }
            for piece in 0..pieces {
                let fraction = piece as f64 / pieces as f64;
                points.push((
                    span[0].point.lerp(span[1].point, fraction),
                    span[0].t + (span[1].t - span[0].t) * fraction,
                ));
            }
        }
        if points.len() < 3 {
            return Err(MeshError::Topology(
                "obstacle sampling produced fewer than three points",
            ));
        }
        if polygon_area(points.iter().map(|(point, _)| *point)) > 0.0 {
            points.reverse();
        }
        let label = match obstacle.role {
            LoopRole::Hole { .. } => BoundaryLabel::Obstacle(obstacle.id),
            LoopRole::MaterialInterface { .. } => BoundaryLabel::MaterialInterface(obstacle.id),
            LoopRole::Wall { .. } => BoundaryLabel::Wall {
                loop_id: obstacle.id,
                side: BoundarySide::Exterior,
            },
        };
        let mut vertices = vec![];
        for (point, parameter) in &points {
            vertices.push(self.add_vertex(
                *point,
                Some(BoundaryPoint {
                    label,
                    parameter: *parameter,
                }),
            )?);
        }
        for index in 0..vertices.len() {
            let next = (index + 1) % vertices.len();
            let start = points[index].1;
            let mut end = points[next].1;
            let period = obstacle.spline.period();
            if end - start > period * 0.5 {
                end -= period;
            } else if end - start < -period * 0.5 {
                end += period;
            }
            self.add_boundary_edge(BoundaryEdge {
                vertices: [vertices[index], vertices[next]],
                label,
                parameters: [start, end],
            });
        }
        let exterior = Polygon {
            vertices: vertices.clone(),
        };
        let interior = if matches!(obstacle.role, LoopRole::Wall { .. }) {
            let interior_label = BoundaryLabel::Wall {
                loop_id: obstacle.id,
                side: BoundarySide::Interior,
            };
            let mut interior_vertices = Vec::with_capacity(points.len());
            for (point, parameter) in points.iter().rev() {
                interior_vertices.push(self.add_vertex(
                    *point,
                    Some(BoundaryPoint {
                        label: interior_label,
                        parameter: *parameter,
                    }),
                )?);
            }
            for index in 0..interior_vertices.len() {
                let next = (index + 1) % interior_vertices.len();
                let start = self.vertices[interior_vertices[index]]
                    .boundary
                    .unwrap()
                    .parameter;
                let mut end = self.vertices[interior_vertices[next]]
                    .boundary
                    .unwrap()
                    .parameter;
                let period = obstacle.spline.period();
                if end - start > period * 0.5 {
                    end -= period;
                } else if end - start < -period * 0.5 {
                    end += period;
                }
                self.add_boundary_edge(BoundaryEdge {
                    vertices: [interior_vertices[index], interior_vertices[next]],
                    label: interior_label,
                    parameters: [start, end],
                });
            }
            Some(Polygon {
                vertices: interior_vertices,
            })
        } else {
            None
        };
        Ok((exterior, interior))
    }

    fn prepare_domains(&mut self) -> Result<(), MeshError> {
        self.domains.clear();
        let outer = self
            .domain_loops
            .first()
            .ok_or(MeshError::Topology("outer boundary is missing"))?
            .vertices
            .clone();
        let holes_for = |region: RegionId, loops: &[Polygon], roles: &[LoopRole]| {
            roles
                .iter()
                .enumerate()
                .filter(|(_, role)| role.exterior() == region)
                .map(|(index, _)| loops[index + 1].vertices.clone())
                .collect::<Vec<_>>()
        };
        self.domains.push(TriangulationDomain {
            region: BACKGROUND_REGION,
            outer,
            holes: holes_for(BACKGROUND_REGION, &self.domain_loops, &self.loop_roles),
        });
        for (index, role) in self.loop_roles.iter().copied().enumerate() {
            let Some(region) = role.interior() else {
                continue;
            };
            let outer = match role {
                LoopRole::MaterialInterface { .. } => {
                    let mut vertices = self.domain_loops[index + 1].vertices.clone();
                    vertices.reverse();
                    vertices
                }
                LoopRole::Wall { .. } => self.interior_loops[index]
                    .as_ref()
                    .ok_or(MeshError::Topology("wall interior trace is missing"))?
                    .vertices
                    .clone(),
                LoopRole::Hole { .. } => unreachable!(),
            };
            self.domains.push(TriangulationDomain {
                region,
                outer,
                holes: holes_for(region, &self.domain_loops, &self.loop_roles),
            });
        }
        Ok(())
    }

    fn region_at(&self, point: Point2) -> Option<RegionId> {
        if self.domain_loops.is_empty() {
            return None;
        }
        let points = |polygon: &Polygon| {
            polygon
                .vertices
                .iter()
                .map(|index| self.point(*index))
                .collect::<Vec<_>>()
        };
        if locate_in_polygon(point, &points(&self.domain_loops[0])) == PolygonLocation::Outside {
            return None;
        }
        let mut containing = None::<(f64, LoopRole)>;
        for (polygon, role) in self.domain_loops[1..].iter().zip(&self.loop_roles) {
            if locate_in_polygon(point, &points(polygon)) != PolygonLocation::Outside {
                let area =
                    polygon_area(polygon.vertices.iter().map(|index| self.point(*index))).abs();
                if containing.is_none_or(|current| area < current.0) {
                    containing = Some((area, *role));
                }
            }
        }
        match containing.map(|(_, role)| role) {
            None => Some(BACKGROUND_REGION),
            Some(LoopRole::Hole { .. }) => None,
            Some(
                LoopRole::MaterialInterface { interior, .. } | LoopRole::Wall { interior, .. },
            ) => Some(interior),
        }
    }

    fn boundary_relevant_to_region(&self, label: BoundaryLabel, region: RegionId) -> bool {
        match label {
            BoundaryLabel::Outer(_) => region == BACKGROUND_REGION,
            BoundaryLabel::Obstacle(id) => self
                .loop_role(id)
                .is_some_and(|role| role.exterior() == region),
            BoundaryLabel::MaterialInterface(id) => self
                .loop_role(id)
                .is_some_and(|role| role.exterior() == region || role.interior() == Some(region)),
            BoundaryLabel::Wall { loop_id, side } => {
                self.loop_role(loop_id).is_some_and(|role| match side {
                    BoundarySide::Exterior => role.exterior() == region,
                    BoundarySide::Interior => role.interior() == Some(region),
                })
            }
            BoundaryLabel::InternalBoundary { id, .. } => self
                .internal_boundary_ids
                .iter()
                .position(|candidate| *candidate == id)
                .is_some_and(|index| self.internal_boundary_regions[index] == region),
        }
    }

    fn loop_role(&self, id: ObstacleId) -> Option<LoopRole> {
        self.loop_ids
            .iter()
            .position(|candidate| *candidate == id)
            .map(|index| self.loop_roles[index])
    }

    /// Test one queued edge, updating only the two incident triangles after a flip.
    fn legalize_one(&mut self) -> Result<(), MeshError> {
        let Some(edge @ (a, b)) = self.dirty_edges.pop_front() else {
            return Ok(());
        };
        self.queued_edges.remove(&edge);
        self.stats.edge_tests += 1;
        let Some(sides) = self.adjacency.get(&edge) else {
            return Ok(());
        };
        if sides.len() != 2 || self.boundary_keys.contains(&edge) {
            return Ok(());
        }
        let [(left_index, c), (right_index, d)] = [sides[0], sides[1]];
        let region = self.triangles[left_index].region;
        if self.triangles[right_index].region != region {
            return Err(MeshError::Topology(
                "an unconstrained edge crosses a material interface",
            ));
        }
        let [pa, pb, pc, pd] = [a, b, c, d].map(|v| self.point(v));
        // Both diagonals must lie inside a strictly convex quadrilateral.
        let opposite = |x, y| {
            matches!(
                (x, y),
                (PredicateSign::Positive, PredicateSign::Negative)
                    | (PredicateSign::Negative, PredicateSign::Positive)
            )
        };
        if !opposite(orient2d(pa, pb, pc), orient2d(pa, pb, pd))
            || !opposite(orient2d(pc, pd, pa), orient2d(pc, pd, pb))
        {
            return Ok(());
        }
        let [x, y, z] = self.triangle_points(self.triangles[left_index]);
        if incircle(x, y, z, pd) != PredicateSign::Positive {
            return Ok(());
        }
        if self
            .repair_region
            .as_ref()
            .is_some_and(|region| [a, b, c, d].iter().any(|i| !region[*i]))
        {
            return Err(MeshError::Topology(
                "edge legalization reached the fixed patch boundary",
            ));
        }
        let left = self.ccw_triangle([c, d, a], region)?;
        let right = self.ccw_triangle([d, c, b], region)?;
        self.replace_triangle(left_index, left);
        self.replace_triangle(right_index, right);
        self.stats.edge_flips += 1;
        Ok(())
    }

    fn triangle_points(&self, triangle: MeshTriangle) -> [Point2; 3] {
        triangle.vertices.map(|vertex| self.point(vertex))
    }

    fn triangle_quality(&self, triangle: MeshTriangle) -> MeshQuality {
        let points = self.triangle_points(triangle);
        let sides = [
            (points[1] - points[2]).norm(),
            (points[2] - points[0]).norm(),
            (points[0] - points[1]).norm(),
        ];
        let angles = [
            angle_from_sides(sides[0], sides[1], sides[2]),
            angle_from_sides(sides[1], sides[2], sides[0]),
            angle_from_sides(sides[2], sides[0], sides[1]),
        ];
        MeshQuality {
            minimum_angle_degrees: angles.into_iter().fold(f64::INFINITY, f64::min),
            maximum_edge_length: sides.into_iter().fold(0.0, f64::max),
        }
    }

    fn quality(&self) -> MeshQuality {
        self.triangles.iter().fold(
            MeshQuality {
                minimum_angle_degrees: 180.0,
                maximum_edge_length: 0.0,
            },
            |quality, triangle| {
                let triangle = self.triangle_quality(*triangle);
                MeshQuality {
                    minimum_angle_degrees: quality
                        .minimum_angle_degrees
                        .min(triangle.minimum_angle_degrees),
                    maximum_edge_length: quality
                        .maximum_edge_length
                        .max(triangle.maximum_edge_length),
                }
            },
        )
    }

    fn circumcenter(&self, triangle: MeshTriangle) -> Option<Point2> {
        let [a, b, c] = self.triangle_points(triangle);
        let denominator = 2.0 * (b - a).cross(c - a);
        if denominator == 0.0 {
            return None;
        }
        let b2 = (b - a).dot(b - a);
        let c2 = (c - a).dot(c - a);
        Some(
            a + Point2::new(
                ((c.y - a.y) * b2 - (b.y - a.y) * c2) / denominator,
                ((b.x - a.x) * c2 - (c.x - a.x) * b2) / denominator,
            ),
        )
    }

    fn containing_triangle(&self, point: Point2) -> Option<(usize, PolygonLocation)> {
        let mut boundary = None;
        for (index, triangle) in self.triangles.iter().enumerate() {
            let location = point_in_triangle(point, self.triangle_points(*triangle));
            if location == PolygonLocation::Inside {
                return Some((index, location));
            }
            if location == PolygonLocation::Boundary && boundary.is_none() {
                boundary = Some((index, location));
            }
        }
        boundary
    }

    fn split_boundary(&mut self, edge_index: usize) -> Result<(), MeshError> {
        let edge = self.boundary_edges[edge_index];
        let point = self
            .point(edge.vertices[0])
            .lerp(self.point(edge.vertices[1]), 0.5);
        let parameter = (edge.parameters[0] + edge.parameters[1]) * 0.5;
        let vertex = self.add_vertex(
            point,
            Some(BoundaryPoint {
                label: edge.label,
                parameter,
            }),
        )?;
        self.boundary_keys
            .remove(&edge_key(edge.vertices[0], edge.vertices[1]));
        self.boundary_edges[edge_index] = BoundaryEdge {
            vertices: [edge.vertices[0], vertex],
            label: edge.label,
            parameters: [edge.parameters[0], parameter],
        };
        self.boundary_keys
            .insert(edge_key(edge.vertices[0], vertex));
        self.add_boundary_edge(BoundaryEdge {
            vertices: [vertex, edge.vertices[1]],
            label: edge.label,
            parameters: [parameter, edge.parameters[1]],
        });
        self.split_edge(edge.vertices, vertex)
    }

    fn split_edge(&mut self, edge: [usize; 2], vertex: usize) -> Result<(), MeshError> {
        let adjacent = self
            .adjacency
            .get(&edge_key(edge[0], edge[1]))
            .cloned()
            .unwrap_or_default();
        if adjacent.is_empty() || adjacent.len() > 2 {
            return Err(MeshError::Topology("edge has invalid triangle adjacency"));
        }
        if self.repair_region.as_ref().is_some_and(|region| {
            adjacent
                .iter()
                .any(|(i, _)| self.triangles[*i].vertices.iter().any(|v| !region[*v]))
        }) {
            return Err(MeshError::Topology(
                "edge split reached the fixed patch boundary",
            ));
        }
        for (index, opposite) in adjacent.into_iter().rev() {
            let region = self.triangles[index].region;
            self.replace_triangle(
                index,
                self.ccw_triangle([edge[0], vertex, opposite], region)?,
            );
            self.push_triangle(self.ccw_triangle([vertex, edge[1], opposite], region)?)?;
        }
        Ok(())
    }

    fn insert_point(&mut self, point: Point2) -> Result<(), MeshError> {
        let (triangle_index, location) = self.containing_triangle(point).ok_or(
            MeshError::Topology("refinement point is outside the domain"),
        )?;
        if location == PolygonLocation::Boundary
            && self.triangles[triangle_index]
                .vertices
                .iter()
                .any(|vertex| {
                    (self.point(*vertex) - point).norm() <= self.options.curve_tolerance * 0.1
                })
        {
            return Err(MeshError::Topology(
                "refinement point duplicates an existing vertex",
            ));
        }
        let vertex = self.add_vertex(point, None)?;
        let triangle = self.triangles[triangle_index];
        if location == PolygonLocation::Boundary {
            let edge = (0..3)
                .map(|index| [triangle.vertices[index], triangle.vertices[(index + 1) % 3]])
                .find(|edge| on_segment(point, self.point(edge[0]), self.point(edge[1])))
                .ok_or(MeshError::Topology("could not locate containing edge"))?;
            if let Some(boundary_index) = self.boundary_edges.iter().position(|boundary| {
                edge_key(boundary.vertices[0], boundary.vertices[1]) == edge_key(edge[0], edge[1])
            }) {
                // The caller should normally catch encroachment first. Reuse the
                // exact candidate only when it is the segment midpoint.
                let midpoint = self.point(edge[0]).lerp(self.point(edge[1]), 0.5);
                self.vertices.pop();
                self.incident.pop();
                if let Some(region) = &mut self.repair_region {
                    region.pop();
                }
                if midpoint != point {
                    return Err(MeshError::Topology(
                        "refinement point lies on a constrained edge",
                    ));
                }
                return self.split_boundary(boundary_index);
            }
            self.split_edge(edge, vertex)
        } else {
            self.replace_triangle(
                triangle_index,
                self.ccw_triangle(
                    [triangle.vertices[0], triangle.vertices[1], vertex],
                    triangle.region,
                )?,
            );
            self.push_triangle(self.ccw_triangle(
                [triangle.vertices[1], triangle.vertices[2], vertex],
                triangle.region,
            )?)?;
            self.push_triangle(self.ccw_triangle(
                [triangle.vertices[2], triangle.vertices[0], vertex],
                triangle.region,
            )?)?;
            Ok(())
        }
    }

    fn insert_constraint_point(
        &mut self,
        boundary: usize,
        point: Point2,
    ) -> Result<usize, MeshError> {
        if let Some((index, _)) =
            self.vertices.iter().enumerate().find(|(_, vertex)| {
                (vertex.point - point).norm() <= self.options.curve_tolerance * 0.1
            })
        {
            return Ok(index);
        }
        // Inserting a constraint point just beside a free bulk vertex creates a
        // tiny element and can collapse an explicit solver's CFL timestep. Move
        // the free vertex onto the exact curve sample when its complete triangle
        // fan remains oriented and belongs to this material region. Constraint
        // vertices and points already used by another open chain stay fixed.
        let protected = self
            .internal_chains
            .iter()
            .flat_map(|chain| chain.iter().map(|(vertex, _)| *vertex))
            .chain(self.internal_trace_vertices.iter().copied())
            .collect::<BTreeSet<_>>();
        let target_region = self.internal_boundary_regions[boundary];
        let snap_distance = self.options.target_edge_length * 0.3;
        let relocatable = self
            .vertices
            .iter()
            .enumerate()
            .filter(|(index, vertex)| {
                vertex.boundary.is_none()
                    && !protected.contains(index)
                    && (vertex.point - point).norm() <= snap_distance
                    && !self.incident[*index].is_empty()
                    && self.incident[*index].iter().all(|triangle_index| {
                        let triangle = self.triangles[*triangle_index];
                        if triangle.region != target_region {
                            return false;
                        }
                        let points = triangle.vertices.map(|vertex| {
                            if vertex == *index {
                                point
                            } else {
                                self.point(vertex)
                            }
                        });
                        orient2d(points[0], points[1], points[2]) == PredicateSign::Positive
                    })
            })
            .min_by(|(_, left), (_, right)| {
                (left.point - point)
                    .norm()
                    .total_cmp(&(right.point - point).norm())
            })
            .map(|(index, _)| index);
        if let Some(vertex) = relocatable {
            let incident = self.incident[vertex].iter().copied().collect::<Vec<_>>();
            self.vertices[vertex].point = point;
            for triangle in incident {
                let unchanged = self.triangles[triangle];
                self.replace_triangle(triangle, unchanged);
            }
            return Ok(vertex);
        }
        let (triangle_index, location) = self.containing_triangle(point).ok_or(
            MeshError::Topology("internal boundary leaves its material region"),
        )?;
        let triangle = self.triangles[triangle_index];
        if triangle.region != self.internal_boundary_regions[boundary] {
            return Err(MeshError::Topology(
                "internal boundary has the wrong containing region",
            ));
        }
        let vertex = self.add_vertex(point, None)?;
        if location == PolygonLocation::Boundary {
            let edge = (0..3)
                .map(|index| [triangle.vertices[index], triangle.vertices[(index + 1) % 3]])
                .find(|edge| on_segment(point, self.point(edge[0]), self.point(edge[1])))
                .ok_or(MeshError::Topology(
                    "could not locate internal-boundary edge",
                ))?;
            if self.boundary_keys.contains(&edge_key(edge[0], edge[1])) {
                return Err(MeshError::Topology(
                    "internal boundary touches an existing constrained boundary",
                ));
            }
            self.split_edge(edge, vertex)?;
        } else {
            self.replace_triangle(
                triangle_index,
                self.ccw_triangle(
                    [triangle.vertices[0], triangle.vertices[1], vertex],
                    triangle.region,
                )?,
            );
            self.push_triangle(self.ccw_triangle(
                [triangle.vertices[1], triangle.vertices[2], vertex],
                triangle.region,
            )?)?;
            self.push_triangle(self.ccw_triangle(
                [triangle.vertices[2], triangle.vertices[0], vertex],
                triangle.region,
            )?)?;
        }
        Ok(vertex)
    }

    /// Recovers one constrained segment by flipping one intersecting diagonal.
    /// Returns true once the requested edge exists.
    fn recover_constraint_edge(&mut self, requested: [usize; 2]) -> Result<bool, MeshError> {
        let requested_key = edge_key(requested[0], requested[1]);
        if self.adjacency.contains_key(&requested_key) {
            return Ok(true);
        }
        let a = self.point(requested[0]);
        let b = self.point(requested[1]);
        let crossing = self.adjacency.keys().copied().find(|edge| {
            !self.boundary_keys.contains(edge)
                && ![edge.0, edge.1]
                    .iter()
                    .any(|vertex| requested.contains(vertex))
                && segment_relation(a, b, self.point(edge.0), self.point(edge.1))
                    == SegmentRelation::ProperIntersection
                && self.adjacency.get(edge).is_some_and(|sides| {
                    if sides.len() != 2 {
                        return false;
                    }
                    let c = self.point(sides[0].1);
                    let d = self.point(sides[1].1);
                    matches!(
                        (
                            orient2d(c, d, self.point(edge.0)),
                            orient2d(c, d, self.point(edge.1))
                        ),
                        (PredicateSign::Positive, PredicateSign::Negative)
                            | (PredicateSign::Negative, PredicateSign::Positive)
                    ) && segment_relation(a, b, c, d) != SegmentRelation::ProperIntersection
                })
        });
        let Some(edge) = crossing else {
            return Err(MeshError::Topology(
                "could not recover an internal-boundary segment",
            ));
        };
        let sides = self
            .adjacency
            .get(&edge)
            .cloned()
            .ok_or(MeshError::Topology("missing intersecting-edge adjacency"))?;
        if sides.len() != 2 {
            return Err(MeshError::Topology(
                "internal-boundary recovery reached a mesh boundary",
            ));
        }
        let [(first_index, c), (second_index, d)] = [sides[0], sides[1]];
        let first = self.triangles[first_index];
        let second = self.triangles[second_index];
        if first.region != second.region {
            return Err(MeshError::Topology(
                "internal boundary crosses a material interface",
            ));
        }
        let pa = self.point(edge.0);
        let pb = self.point(edge.1);
        let pc = self.point(c);
        let pd = self.point(d);
        let opposite = |x, y| {
            matches!(
                (x, y),
                (PredicateSign::Positive, PredicateSign::Negative)
                    | (PredicateSign::Negative, PredicateSign::Positive)
            )
        };
        if !opposite(orient2d(pc, pd, pa), orient2d(pc, pd, pb)) {
            return Err(MeshError::Topology(
                "internal-boundary recovery found a non-convex edge",
            ));
        }
        let region = first.region;
        let first_new = self.ccw_triangle([c, d, edge.0], region)?;
        let second_new = self.ccw_triangle([d, c, edge.1], region)?;
        self.replace_triangle(first_index, first_new);
        self.replace_triangle(second_index, second_new);
        self.stats.edge_flips += 1;
        Ok(false)
    }

    fn cut_internal_boundary(&mut self, index: usize) -> Result<(), MeshError> {
        let chain = self.internal_chains[index].clone();
        if chain.len() < 3 {
            return Err(MeshError::Topology(
                "internal boundary needs at least two mesh segments",
            ));
        }
        for pair in chain.windows(2) {
            if self
                .adjacency
                .get(&edge_key(pair[0].0, pair[1].0))
                .is_none_or(|sides| sides.len() != 2)
            {
                return Err(MeshError::Topology(
                    "internal-boundary segment was not recovered",
                ));
            }
        }
        if self.vertices.len() + chain.len() - 2 > self.options.max_vertices {
            return Err(self.capacity_error());
        }
        let id = self.internal_boundary_ids[index];
        let left_label = BoundaryLabel::InternalBoundary {
            id,
            side: InternalBoundarySide::Left,
        };
        let right_label = BoundaryLabel::InternalBoundary {
            id,
            side: InternalBoundarySide::Right,
        };
        let mut duplicates = BTreeMap::new();
        self.internal_trace_vertices
            .extend(chain.iter().map(|(vertex, _)| *vertex));
        self.vertices[chain[0].0].boundary = Some(BoundaryPoint {
            label: left_label,
            parameter: chain[0].1,
        });
        self.vertices[chain[chain.len() - 1].0].boundary = Some(BoundaryPoint {
            label: left_label,
            parameter: chain[chain.len() - 1].1,
        });
        for &(vertex, parameter) in &chain[1..chain.len() - 1] {
            self.vertices[vertex].boundary = Some(BoundaryPoint {
                label: left_label,
                parameter,
            });
            let duplicate = self.add_vertex(
                self.point(vertex),
                Some(BoundaryPoint {
                    label: right_label,
                    parameter,
                }),
            )?;
            duplicates.insert(vertex, duplicate);
            self.internal_trace_vertices.insert(duplicate);
        }

        let mut replacements = BTreeMap::<usize, Vec<(usize, usize)>>::new();
        for local in 1..chain.len() - 1 {
            let vertex = chain[local].0;
            let previous_edge = edge_key(chain[local - 1].0, vertex);
            let next_edge = edge_key(vertex, chain[local + 1].0);
            let seed = self
                .adjacency
                .get(&next_edge)
                .and_then(|sides| {
                    sides.iter().find_map(|(triangle, opposite)| {
                        (orient2d(
                            self.point(vertex),
                            self.point(chain[local + 1].0),
                            self.point(*opposite),
                        ) == PredicateSign::Negative)
                            .then_some(*triangle)
                    })
                })
                .ok_or(MeshError::Topology(
                    "internal-boundary trace has no right-side element",
                ))?;
            let mut sector = BTreeSet::from([seed]);
            let mut pending = vec![seed];
            while let Some(triangle_index) = pending.pop() {
                let triangle = self.triangles[triangle_index];
                for other in triangle
                    .vertices
                    .iter()
                    .copied()
                    .filter(|candidate| *candidate != vertex)
                {
                    let edge = edge_key(vertex, other);
                    if edge == previous_edge || edge == next_edge {
                        continue;
                    }
                    if let Some(sides) = self.adjacency.get(&edge) {
                        for (neighbor, _) in sides {
                            if *neighbor != triangle_index
                                && self.triangles[*neighbor].vertices.contains(&vertex)
                                && sector.insert(*neighbor)
                            {
                                pending.push(*neighbor);
                            }
                        }
                    }
                }
            }
            for triangle_index in sector {
                replacements
                    .entry(triangle_index)
                    .or_default()
                    .push((vertex, duplicates[&vertex]));
            }
        }
        for (triangle_index, replacements) in replacements {
            let mut triangle = self.triangles[triangle_index];
            for vertex in &mut triangle.vertices {
                if let Some((_, replacement)) = replacements
                    .iter()
                    .find(|(original, _)| *original == *vertex)
                {
                    *vertex = *replacement;
                }
            }
            self.replace_triangle(triangle_index, triangle);
        }

        let right_vertex = |local: usize| {
            let vertex = chain[local].0;
            duplicates.get(&vertex).copied().unwrap_or(vertex)
        };
        for local in 0..chain.len() - 1 {
            self.add_boundary_edge(BoundaryEdge {
                vertices: [chain[local].0, chain[local + 1].0],
                label: left_label,
                parameters: [chain[local].1, chain[local + 1].1],
            });
            self.add_boundary_edge(BoundaryEdge {
                vertices: [right_vertex(local + 1), right_vertex(local)],
                label: right_label,
                parameters: [chain[local + 1].1, chain[local].1],
            });
        }
        let trace_triangles = chain
            .into_iter()
            .map(|(vertex, _)| vertex)
            .chain(duplicates.values().copied())
            .flat_map(|vertex| self.incident[vertex].iter().copied())
            .collect::<BTreeSet<_>>();
        for triangle in trace_triangles {
            let unchanged = self.triangles[triangle];
            self.replace_triangle(triangle, unchanged);
        }
        if self.triangles.iter().any(|triangle| {
            let [a, b, c] = self.triangle_points(*triangle);
            orient2d(a, b, c) != PredicateSign::Positive
        }) {
            return Err(MeshError::Topology(
                "internal-boundary cut created a degenerate element",
            ));
        }
        Ok(())
    }

    /// Performs one bounded refinement insertion. Returns true when all quality
    /// targets are satisfied.
    fn refine_once(&mut self) -> Result<bool, MeshError> {
        let Some(&(_, triangle_index)) = self.bad_triangles.last() else {
            return Ok(true);
        };
        if self.vertices.len() >= self.options.max_vertices
            || self.triangles.len() + 2 > self.options.max_triangles
        {
            return Err(self.capacity_error());
        }
        let triangle = self.triangles[triangle_index];
        if self
            .repair_region
            .as_ref()
            .is_some_and(|region| triangle.vertices.iter().any(|i| !region[*i]))
        {
            return Err(MeshError::Topology(
                "quality repair reached the fixed patch boundary",
            ));
        }
        let points = self.triangle_points(triangle);
        let centroid = (points[0] + points[1] + points[2]) / 3.0;
        let mut candidate = self
            .circumcenter(triangle)
            .filter(|point| {
                self.containing_triangle(*point).is_some_and(|(index, _)| {
                    self.triangles[index].region == triangle.region
                        && self.repair_region.as_ref().is_none_or(|region| {
                            self.triangles[index].vertices.iter().all(|v| region[*v])
                        })
                })
            })
            .unwrap_or(centroid);
        if self.region_at(candidate) != Some(triangle.region) {
            candidate = centroid;
        }
        // Coincident two-faced traces are distinct topological vertices. A
        // circumcenter can land exactly on the opposite trace even though it is
        // unrelated to the element being refined; use the element centroid in
        // that ambiguous geometric case.
        for attempt in 0..8 {
            if !self.vertices.iter().any(|vertex| {
                (vertex.point - candidate).norm() <= self.options.curve_tolerance * 0.1
            }) {
                break;
            }
            candidate = centroid.lerp(points[attempt % 3], 0.01 * (attempt + 1) as f64);
        }
        if self
            .vertices
            .iter()
            .any(|vertex| (vertex.point - candidate).norm() <= self.options.curve_tolerance * 0.1)
        {
            if let Some(score) = self.scores[triangle_index].take() {
                self.bad_triangles.remove(&(score, triangle_index));
            }
            return Ok(false);
        }
        if let Some(edge_index) = self.boundary_edges.iter().position(|edge| {
            if matches!(edge.label, BoundaryLabel::InternalBoundary { .. }) {
                return false;
            }
            if !self.boundary_relevant_to_region(edge.label, triangle.region) {
                return false;
            }
            let a = self.point(edge.vertices[0]);
            let b = self.point(edge.vertices[1]);
            (candidate - a).dot(candidate - b) < 0.0
        }) {
            if self.repair_region.as_ref().is_some_and(|region| {
                self.boundary_edges[edge_index]
                    .vertices
                    .iter()
                    .any(|v| !region[*v])
            }) {
                return Err(MeshError::Topology("boundary repair left the local patch"));
            }
            self.split_boundary(edge_index)?;
        } else {
            self.insert_point(candidate)?;
        }
        self.stats.refinement_insertions += 1;
        Ok(false)
    }
}

fn resample_internal_boundary(
    samples: Vec<Sample>,
    options: MeshingOptions,
) -> Result<Vec<Sample>, MeshError> {
    let mut result = Vec::new();
    for span in samples.windows(2) {
        let length = (span[1].point - span[0].point).norm();
        let pieces = (length / options.target_edge_length).ceil().max(1.0) as usize;
        if result.len() + pieces + 1 > options.max_vertices {
            return Err(MeshError::Capacity {
                vertices: result.len(),
                triangles: 0,
            });
        }
        for piece in 0..pieces {
            let fraction = piece as f64 / pieces as f64;
            result.push(Sample {
                t: span[0].t + (span[1].t - span[0].t) * fraction,
                point: span[0].point.lerp(span[1].point, fraction),
            });
        }
    }
    result.push(*samples.last().ok_or(MeshError::Topology(
        "internal-boundary sampling produced no points",
    ))?);
    if result.len() == 2 {
        result.insert(
            1,
            Sample {
                t: (result[0].t + result[1].t) * 0.5,
                point: result[0].point.lerp(result[1].point, 0.5),
            },
        );
    }
    Ok(result)
}

fn angle_from_sides(opposite: f64, adjacent_a: f64, adjacent_b: f64) -> f64 {
    let cosine = ((adjacent_a * adjacent_a + adjacent_b * adjacent_b - opposite * opposite)
        / (2.0 * adjacent_a * adjacent_b))
        .clamp(-1.0, 1.0);
    cosine.acos().to_degrees()
}

fn valid_options(options: MeshingOptions) -> bool {
    options.curve_tolerance.is_finite()
        && options.curve_tolerance > 0.0
        && options.curve_tolerance <= WORLD_TOLERANCE * 10.0
        && options.target_edge_length.is_finite()
        && options.target_edge_length > options.curve_tolerance * 4.0
        && options.minimum_angle_degrees.is_finite()
        && (0.0..30.0).contains(&options.minimum_angle_degrees)
        && options.max_vertices >= 16
        && options.max_triangles >= 16
        && options.max_refinement_steps > 0
}

/// Builds the same mesh as the cooperative job, running it to completion.
pub fn mesh_scene(
    scene: &Scene,
    geometry_revision: u64,
    options: MeshingOptions,
) -> Result<TriMesh, MeshError> {
    let mut job = MeshingJob::new(scene.clone(), geometry_revision, options);
    loop {
        if let Some(result) = job.advance(4096) {
            return result;
        }
    }
}

struct BridgeSearch {
    polygon: Vec<usize>,
    holes: Vec<Vec<usize>>,
    hole: usize,
    domain: usize,
    region: RegionId,
    outer_index: usize,
    hole_index: usize,
    best: Option<(f64, usize, usize)>,
    visibility: Option<(u8, usize, f64)>,
    seeding: bool,
    testing_seed: bool,
}

struct EarSearch {
    polygon: Vec<usize>,
    domain: usize,
    region: RegionId,
    index: usize,
    degenerate_pass: bool,
    stage: EarStage,
}

enum EarStage {
    Start,
    Diagonal(usize),
    Contains(usize),
}

enum MeshingJobState {
    Validate(Box<ValidationJob>),
    Outer,
    Sample {
        obstacle: usize,
        sampler: Option<Sampler>,
    },
    SampleInternal {
        boundary: usize,
        sampler: Option<OpenSampler>,
    },
    Bridge(BridgeSearch),
    Clip(EarSearch),
    LegalizeInitial,
    RefineBeforeInternalBoundaries,
    LegalizeBeforeInternalBoundaries,
    InsertInternalPoints {
        boundary: usize,
        sample: usize,
    },
    RecoverInternalEdges {
        boundary: usize,
        segment: usize,
        attempts: usize,
    },
    CutInternalBoundary {
        boundary: usize,
    },
    LegalizeInternalCut,
    Legalize,
    Refine,
    VerifyTriangles {
        index: usize,
        quality: MeshQuality,
    },
    VerifyBoundary {
        index: usize,
        quality: MeshQuality,
    },
    Done,
}

/// Resumable constrained meshing. A unit advances validation/sampling, tests one
/// segment or vertex of a bridge/ear candidate, legalizes one edge, inserts one
/// point, or verifies one output element. Boundary assembly, point location and
/// domain classification still have linear capacity-bounded scans; callers can
/// check a clock between units. This is a work bound, not a realtime guarantee.
pub struct MeshingJob {
    geometry_revision: u64,
    scene: Scene,
    builder: MeshBuilder,
    legalization_work: usize,
    state: MeshingJobState,
}

impl MeshingJob {
    pub fn new(scene: Scene, geometry_revision: u64, options: MeshingOptions) -> Self {
        Self {
            geometry_revision,
            state: MeshingJobState::Validate(Box::new(ValidationJob::new(
                scene.clone(),
                geometry_revision,
            ))),
            scene,
            builder: MeshBuilder::new(options),
            legalization_work: 0,
        }
    }

    pub fn stats(&self) -> MeshingStats {
        self.builder.stats
    }

    pub fn phase(&self) -> &'static str {
        match self.state {
            MeshingJobState::Validate(_) => "Validating",
            MeshingJobState::Outer
            | MeshingJobState::Sample { .. }
            | MeshingJobState::SampleInternal { .. } => "Sampling boundaries",
            MeshingJobState::Bridge(_) => "Connecting holes",
            MeshingJobState::Clip(_) => "Triangulating",
            MeshingJobState::LegalizeInitial
            | MeshingJobState::LegalizeBeforeInternalBoundaries
            | MeshingJobState::LegalizeInternalCut
            | MeshingJobState::Legalize => "Legalizing edges",
            MeshingJobState::InsertInternalPoints { .. }
            | MeshingJobState::RecoverInternalEdges { .. }
            | MeshingJobState::CutInternalBoundary { .. } => "Cutting internal boundaries",
            MeshingJobState::RefineBeforeInternalBoundaries => "Refining",
            MeshingJobState::Refine => "Refining",
            MeshingJobState::VerifyTriangles { .. } | MeshingJobState::VerifyBoundary { .. } => {
                "Checking mesh"
            }
            MeshingJobState::Done => "Finished",
        }
    }

    /// Runs at most budget work units. Zero is a no-op; a completed job returns
    /// its result exactly once. Work scheduling never changes the output.
    pub fn advance(&mut self, budget: usize) -> Option<Result<TriMesh, MeshError>> {
        for _ in 0..budget {
            if matches!(self.state, MeshingJobState::Done) {
                return None;
            }
            self.builder.stats.work_units += 1;
            let result = self.step();
            match result {
                Ok(Some(mesh)) => return Some(Ok(mesh)),
                Ok(None) => {}
                Err(error) => {
                    self.state = MeshingJobState::Done;
                    return Some(Err(error));
                }
            }
        }
        None
    }

    fn step(&mut self) -> Result<Option<TriMesh>, MeshError> {
        if !valid_options(self.builder.options) {
            return Err(MeshError::InvalidOptions);
        }
        // Also cap pathological bridge/ear searches, independently of frame slices.
        if self.builder.stats.work_units > 50_000_000 {
            return Err(MeshError::Topology("meshing work limit reached"));
        }
        let state = std::mem::replace(&mut self.state, MeshingJobState::Done);
        let b = &mut self.builder;
        self.state = match state {
            MeshingJobState::Validate(mut job) => {
                if let Some(result) = job.advance(1) {
                    if let Some(issue) = result.issue {
                        return Err(MeshError::InvalidGeometry(issue));
                    }
                    MeshingJobState::Outer
                } else {
                    MeshingJobState::Validate(job)
                }
            }
            MeshingJobState::Outer => {
                let outer = b.add_outer()?;
                b.domain_loops.push(outer);
                b.loop_ids = self.scene.obstacles.iter().map(|loop_| loop_.id).collect();
                b.loop_roles = self
                    .scene
                    .obstacles
                    .iter()
                    .map(|loop_| loop_.role)
                    .collect();
                MeshingJobState::Sample {
                    obstacle: 0,
                    sampler: None,
                }
            }
            MeshingJobState::Sample { obstacle, sampler } => {
                if obstacle == self.scene.obstacles.len() {
                    b.internal_boundary_ids = self
                        .scene
                        .internal_boundaries
                        .iter()
                        .map(|boundary| boundary.id)
                        .collect();
                    b.internal_boundary_regions = self
                        .scene
                        .internal_boundaries
                        .iter()
                        .map(|boundary| boundary.region)
                        .collect();
                    MeshingJobState::SampleInternal {
                        boundary: 0,
                        sampler: None,
                    }
                } else {
                    let mut sampler = sampler.unwrap_or_else(|| {
                        Sampler::new(
                            &self.scene.obstacles[obstacle].spline,
                            SamplingOptions {
                                tolerance: b.options.curve_tolerance,
                                max_depth: 18,
                                max_points: 8192,
                            },
                        )
                    });
                    if sampler.step() {
                        let samples = sampler
                            .finish()
                            .map_err(|_| MeshError::Sampling(self.scene.obstacles[obstacle].id))?;
                        let (polygon, interior) =
                            b.add_obstacle(&self.scene.obstacles[obstacle], samples)?;
                        b.domain_loops.push(polygon);
                        b.interior_loops.push(interior);
                        MeshingJobState::Sample {
                            obstacle: obstacle + 1,
                            sampler: None,
                        }
                    } else {
                        MeshingJobState::Sample {
                            obstacle,
                            sampler: Some(sampler),
                        }
                    }
                }
            }
            MeshingJobState::SampleInternal { boundary, sampler } => {
                if boundary == self.scene.internal_boundaries.len() {
                    b.prepare_domains()?;
                    let domain = b
                        .domains
                        .first()
                        .ok_or(MeshError::Topology("no material domain to triangulate"))?
                        .clone();
                    MeshingJobState::Bridge(BridgeSearch {
                        polygon: domain.outer,
                        holes: domain.holes,
                        hole: 0,
                        domain: 0,
                        region: domain.region,
                        outer_index: 0,
                        hole_index: 0,
                        best: None,
                        visibility: None,
                        seeding: true,
                        testing_seed: false,
                    })
                } else {
                    let mut sampler = sampler.unwrap_or_else(|| {
                        OpenSampler::new(
                            &self.scene.internal_boundaries[boundary].spline,
                            SamplingOptions {
                                tolerance: b.options.curve_tolerance,
                                max_depth: 18,
                                max_points: 8192,
                            },
                        )
                    });
                    if sampler.step() {
                        let samples = sampler.finish().map_err(|_| {
                            MeshError::Topology("internal-boundary sampling failed")
                        })?;
                        b.internal_samples
                            .push(resample_internal_boundary(samples, b.options)?);
                        MeshingJobState::SampleInternal {
                            boundary: boundary + 1,
                            sampler: None,
                        }
                    } else {
                        MeshingJobState::SampleInternal {
                            boundary,
                            sampler: Some(sampler),
                        }
                    }
                }
            }
            MeshingJobState::Bridge(mut search) => {
                if search.hole == search.holes.len() {
                    MeshingJobState::Clip(EarSearch {
                        polygon: search.polygon,
                        domain: search.domain,
                        region: search.region,
                        index: 0,
                        degenerate_pass: false,
                        stage: EarStage::Start,
                    })
                } else {
                    let hole = &search.holes[search.hole];
                    if search.seeding {
                        // Try the globally shortest pair first. If it is visible
                        // no other bridge can improve it; otherwise use the full
                        // visibility search below. This scan is itself resumable.
                        if search.outer_index == search.polygon.len() {
                            let (length, outer_index, hole_index) = search
                                .best
                                .take()
                                .ok_or(MeshError::Topology("no bridge candidate"))?;
                            search.outer_index = outer_index;
                            search.hole_index = hole_index;
                            search.visibility = Some((0, 0, length));
                            search.seeding = false;
                            search.testing_seed = true;
                        } else {
                            let length = (b.point(search.polygon[search.outer_index])
                                - b.point(hole[search.hole_index]))
                            .norm();
                            if search.best.is_none_or(|best| length < best.0) {
                                search.best = Some((length, search.outer_index, search.hole_index));
                            }
                            search.hole_index += 1;
                            if search.hole_index == hole.len() {
                                search.hole_index = 0;
                                search.outer_index += 1;
                            }
                        }
                    } else if search.outer_index == search.polygon.len() {
                        let (_, outer_index, hole_index) = search
                            .best
                            .ok_or(MeshError::Topology("no visible bridge to obstacle"))?;
                        let mut splice = Vec::with_capacity(hole.len() + 2);
                        for offset in 0..hole.len() {
                            splice.push(hole[(hole_index + offset) % hole.len()]);
                        }
                        splice.push(hole[hole_index]);
                        splice.push(search.polygon[outer_index]);
                        search
                            .polygon
                            .splice(outer_index + 1..outer_index + 1, splice);
                        search.hole += 1;
                        search.outer_index = 0;
                        search.hole_index = 0;
                        search.best = None;
                        search.seeding = true;
                    } else {
                        let a_index = search.polygon[search.outer_index];
                        let v_index = hole[search.hole_index];
                        let a = b.point(a_index);
                        let v = b.point(v_index);
                        let mut next_candidate = false;
                        if let Some((stage, index, length)) = search.visibility {
                            let edge = if stage == 0 {
                                b.boundary_edges.get(index).map(|edge| {
                                    if b.boundary_relevant_to_region(edge.label, search.region) {
                                        edge.vertices
                                    } else {
                                        [a_index, v_index]
                                    }
                                })
                            } else if index < search.polygon.len() {
                                Some([
                                    search.polygon[index],
                                    search.polygon[(index + 1) % search.polygon.len()],
                                ])
                            } else {
                                None
                            };
                            if let Some(edge) = edge {
                                if !edge.contains(&a_index) && !edge.contains(&v_index) {
                                    let relation =
                                        segment_relation(a, v, b.point(edge[0]), b.point(edge[1]));
                                    next_candidate = if stage == 0 {
                                        relation != SegmentRelation::Disjoint
                                    } else {
                                        relation == SegmentRelation::ProperIntersection
                                    };
                                }
                                search.visibility = Some((stage, index + 1, length));
                            } else if stage == 0 {
                                search.visibility = Some((1, 0, length));
                            } else {
                                if b.region_at(a.lerp(v, 0.5)) == Some(search.region) {
                                    search.best =
                                        Some((length, search.outer_index, search.hole_index));
                                }
                                next_candidate = true;
                            }
                        } else {
                            let length = (a - v).norm();
                            if a != v && search.best.is_none_or(|best| length < best.0) {
                                search.visibility = Some((0, 0, length));
                            } else {
                                next_candidate = true;
                            }
                        }
                        if next_candidate {
                            search.visibility = None;
                            if search.testing_seed {
                                search.testing_seed = false;
                                search.outer_index = if search.best.is_some() {
                                    search.polygon.len()
                                } else {
                                    0
                                };
                                search.hole_index = 0;
                            } else {
                                search.hole_index += 1;
                                if search.hole_index == hole.len() {
                                    search.hole_index = 0;
                                    search.outer_index += 1;
                                }
                            }
                        }
                    }
                    MeshingJobState::Bridge(search)
                }
            }
            MeshingJobState::Clip(mut search) => {
                let polygon = &mut search.polygon;
                if polygon.len() == 3 {
                    b.push_triangle(
                        b.ccw_triangle([polygon[0], polygon[1], polygon[2]], search.region)?,
                    )?;
                    let next_domain = search.domain + 1;
                    if let Some(domain) = b.domains.get(next_domain).cloned() {
                        MeshingJobState::Bridge(BridgeSearch {
                            polygon: domain.outer,
                            holes: domain.holes,
                            hole: 0,
                            domain: next_domain,
                            region: domain.region,
                            outer_index: 0,
                            hole_index: 0,
                            best: None,
                            visibility: None,
                            seeding: true,
                            testing_seed: false,
                        })
                    } else {
                        MeshingJobState::LegalizeInitial
                    }
                } else if search.index == polygon.len() {
                    if search.degenerate_pass {
                        return Err(MeshError::Topology("ear clipping stalled"));
                    }
                    search.degenerate_pass = true;
                    search.index = 0;
                    search.stage = EarStage::Start;
                    MeshingJobState::Clip(search)
                } else {
                    let index = search.index;
                    let previous = (index + polygon.len() - 1) % polygon.len();
                    let next = (index + 1) % polygon.len();
                    let [a, v, c] = [polygon[previous], polygon[index], polygon[next]];
                    let mut reject = false;
                    let mut remove = false;
                    match search.stage {
                        EarStage::Start => {
                            let sign = orient2d(b.point(a), b.point(v), b.point(c));
                            let candidate = if search.degenerate_pass {
                                a == v || v == c || a == c || sign == PredicateSign::Zero
                            } else {
                                a != v && v != c && a != c && sign == PredicateSign::Positive
                            };
                            if candidate {
                                search.stage = EarStage::Diagonal(0);
                            } else {
                                reject = true;
                            }
                        }
                        EarStage::Diagonal(other) => {
                            if other == polygon.len() {
                                if search.degenerate_pass {
                                    remove = true;
                                } else {
                                    search.stage = EarStage::Contains(0);
                                }
                            } else {
                                let [x, y] = [polygon[other], polygon[(other + 1) % polygon.len()]];
                                reject = x != a
                                    && x != c
                                    && y != a
                                    && y != c
                                    && segment_relation(
                                        b.point(a),
                                        b.point(c),
                                        b.point(x),
                                        b.point(y),
                                    ) != SegmentRelation::Disjoint;
                                search.stage = EarStage::Diagonal(other + 1);
                            }
                        }
                        EarStage::Contains(other_index) => {
                            if other_index == polygon.len() {
                                remove = true;
                            } else {
                                let other = polygon[other_index];
                                reject = ![previous, index, next].contains(&other_index)
                                    && ![a, v, c].contains(&other)
                                    && point_in_triangle(
                                        b.point(other),
                                        [b.point(a), b.point(v), b.point(c)],
                                    ) != PolygonLocation::Outside;
                                search.stage = EarStage::Contains(other_index + 1);
                            }
                        }
                    }
                    if remove {
                        if !search.degenerate_pass {
                            b.push_triangle(MeshTriangle {
                                vertices: [a, v, c],
                                region: search.region,
                            })?;
                        }
                        polygon.remove(index);
                        search.index = 0;
                        search.degenerate_pass = false;
                        search.stage = EarStage::Start;
                    } else if reject {
                        search.index += 1;
                        search.stage = EarStage::Start;
                    }
                    MeshingJobState::Clip(search)
                }
            }
            MeshingJobState::LegalizeInitial => {
                if b.dirty_edges.is_empty() {
                    self.legalization_work = 0;
                    if b.internal_samples.is_empty() {
                        MeshingJobState::Refine
                    } else {
                        MeshingJobState::RefineBeforeInternalBoundaries
                    }
                } else {
                    self.legalization_work += 1;
                    if self.legalization_work > b.options.max_triangles.saturating_mul(128) {
                        return Err(MeshError::Topology("edge legalization work limit reached"));
                    }
                    b.legalize_one()?;
                    MeshingJobState::LegalizeInitial
                }
            }
            MeshingJobState::RefineBeforeInternalBoundaries => {
                if !b.bad_triangles.is_empty()
                    && b.stats.refinement_insertions >= b.options.max_refinement_steps
                {
                    return Err(MeshError::RefinementLimit(b.quality()));
                }
                if b.refine_once()? {
                    b.internal_chains.push(Vec::new());
                    MeshingJobState::InsertInternalPoints {
                        boundary: 0,
                        sample: 0,
                    }
                } else {
                    MeshingJobState::LegalizeBeforeInternalBoundaries
                }
            }
            MeshingJobState::LegalizeBeforeInternalBoundaries => {
                if b.dirty_edges.is_empty() {
                    self.legalization_work = 0;
                    MeshingJobState::RefineBeforeInternalBoundaries
                } else {
                    self.legalization_work += 1;
                    if self.legalization_work > b.options.max_triangles.saturating_mul(128) {
                        return Err(MeshError::Topology("edge legalization work limit reached"));
                    }
                    b.legalize_one()?;
                    MeshingJobState::LegalizeBeforeInternalBoundaries
                }
            }
            MeshingJobState::InsertInternalPoints { boundary, sample } => {
                if sample == b.internal_samples[boundary].len() {
                    MeshingJobState::RecoverInternalEdges {
                        boundary,
                        segment: 0,
                        attempts: 0,
                    }
                } else {
                    let Sample { t, point } = b.internal_samples[boundary][sample];
                    let vertex = b.insert_constraint_point(boundary, point)?;
                    b.internal_chains[boundary].push((vertex, t));
                    MeshingJobState::InsertInternalPoints {
                        boundary,
                        sample: sample + 1,
                    }
                }
            }
            MeshingJobState::RecoverInternalEdges {
                boundary,
                segment,
                attempts,
            } => {
                if segment + 1 == b.internal_chains[boundary].len() {
                    MeshingJobState::CutInternalBoundary { boundary }
                } else {
                    let requested = [
                        b.internal_chains[boundary][segment].0,
                        b.internal_chains[boundary][segment + 1].0,
                    ];
                    let requested_start = b.point(requested[0]);
                    let requested_end = b.point(requested[1]);
                    let requested_delta = requested_end - requested_start;
                    if let Some((vertex, fraction)) = b
                        .vertices
                        .iter()
                        .enumerate()
                        .filter(|(vertex, _)| !requested.contains(vertex))
                        .filter(|(_, candidate)| {
                            orient2d(requested_start, requested_end, candidate.point)
                                == PredicateSign::Zero
                                && on_segment(candidate.point, requested_start, requested_end)
                        })
                        .map(|(vertex, candidate)| {
                            let fraction = (candidate.point - requested_start).dot(requested_delta)
                                / requested_delta.dot(requested_delta);
                            (vertex, fraction)
                        })
                        .filter(|(_, fraction)| *fraction > 0.0 && *fraction < 1.0)
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                    {
                        let start = b.internal_chains[boundary][segment].1;
                        let end = b.internal_chains[boundary][segment + 1].1;
                        b.internal_chains[boundary]
                            .insert(segment + 1, (vertex, start + (end - start) * fraction));
                        MeshingJobState::RecoverInternalEdges {
                            boundary,
                            segment,
                            attempts,
                        }
                    } else if b.recover_constraint_edge(requested)? {
                        MeshingJobState::RecoverInternalEdges {
                            boundary,
                            segment: segment + 1,
                            attempts: 0,
                        }
                    } else {
                        let attempts = attempts + 1;
                        if attempts > b.options.max_triangles.saturating_mul(4) {
                            return Err(MeshError::Topology(
                                "internal-boundary edge recovery work limit reached",
                            ));
                        }
                        MeshingJobState::RecoverInternalEdges {
                            boundary,
                            segment,
                            attempts,
                        }
                    }
                }
            }
            MeshingJobState::CutInternalBoundary { boundary } => {
                b.cut_internal_boundary(boundary)?;
                if boundary + 1 == b.internal_samples.len() {
                    self.legalization_work = 0;
                    MeshingJobState::LegalizeInternalCut
                } else {
                    b.internal_chains.push(Vec::new());
                    MeshingJobState::InsertInternalPoints {
                        boundary: boundary + 1,
                        sample: 0,
                    }
                }
            }
            MeshingJobState::LegalizeInternalCut => {
                if b.dirty_edges.is_empty() {
                    MeshingJobState::VerifyTriangles {
                        index: 0,
                        quality: MeshQuality {
                            minimum_angle_degrees: 180.0,
                            maximum_edge_length: 0.0,
                        },
                    }
                } else {
                    self.legalization_work += 1;
                    if self.legalization_work > b.options.max_triangles.saturating_mul(128) {
                        return Err(MeshError::Topology("edge legalization work limit reached"));
                    }
                    b.legalize_one()?;
                    MeshingJobState::LegalizeInternalCut
                }
            }
            MeshingJobState::Legalize => {
                if b.dirty_edges.is_empty() {
                    self.legalization_work = 0;
                    MeshingJobState::Refine
                } else {
                    self.legalization_work += 1;
                    if self.legalization_work > b.options.max_triangles.saturating_mul(128) {
                        return Err(MeshError::Topology("edge legalization work limit reached"));
                    }
                    b.legalize_one()?;
                    MeshingJobState::Legalize
                }
            }
            MeshingJobState::Refine => {
                if !b.bad_triangles.is_empty()
                    && b.stats.refinement_insertions >= b.options.max_refinement_steps
                {
                    return Err(MeshError::RefinementLimit(b.quality()));
                }
                if b.refine_once()? {
                    MeshingJobState::VerifyTriangles {
                        index: 0,
                        quality: MeshQuality {
                            minimum_angle_degrees: 180.0,
                            maximum_edge_length: 0.0,
                        },
                    }
                } else {
                    MeshingJobState::Legalize
                }
            }
            MeshingJobState::VerifyTriangles { index, mut quality } => {
                if index == b.triangles.len() {
                    MeshingJobState::VerifyBoundary { index: 0, quality }
                } else {
                    let triangle = b.triangles[index];
                    let [a, v, c] = b.triangle_points(triangle);
                    if orient2d(a, v, c) != PredicateSign::Positive {
                        return Err(MeshError::Topology("mesh contains an inverted triangle"));
                    }
                    let changed_region = b
                        .repair_region
                        .as_ref()
                        .is_none_or(|region| triangle.vertices.iter().any(|v| region[*v]));
                    if changed_region && b.region_at((a + v + c) / 3.0) != Some(triangle.region) {
                        return Err(MeshError::Topology(
                            "triangle has the wrong material-region label",
                        ));
                    }
                    for opposite in 0..3 {
                        let edge = edge_key(
                            triangle.vertices[(opposite + 1) % 3],
                            triangle.vertices[(opposite + 2) % 3],
                        );
                        let sides = b
                            .adjacency
                            .get(&edge)
                            .ok_or(MeshError::Topology("missing adjacency"))?;
                        let expected = b
                            .boundary_edges
                            .iter()
                            .find(|boundary| {
                                edge_key(boundary.vertices[0], boundary.vertices[1]) == edge
                            })
                            .map_or(2, |boundary| match boundary.label {
                                BoundaryLabel::MaterialInterface(_) => 2,
                                BoundaryLabel::Outer(_)
                                | BoundaryLabel::Obstacle(_)
                                | BoundaryLabel::Wall { .. }
                                | BoundaryLabel::InternalBoundary { .. } => 1,
                            });
                        if sides.len() != expected
                            || !sides.contains(&(index, triangle.vertices[opposite]))
                        {
                            return Err(MeshError::Topology(
                                "mesh has a crack or non-manifold edge",
                            ));
                        }
                    }
                    let q = b.triangle_quality(triangle);
                    quality.minimum_angle_degrees =
                        quality.minimum_angle_degrees.min(q.minimum_angle_degrees);
                    quality.maximum_edge_length =
                        quality.maximum_edge_length.max(q.maximum_edge_length);
                    MeshingJobState::VerifyTriangles {
                        index: index + 1,
                        quality,
                    }
                }
            }
            MeshingJobState::VerifyBoundary { index, quality } => {
                if index == b.boundary_edges.len() {
                    return Ok(Some(TriMesh {
                        geometry_revision: self.geometry_revision,
                        vertices: std::mem::take(&mut b.vertices),
                        triangles: std::mem::take(&mut b.triangles),
                        boundary_edges: std::mem::take(&mut b.boundary_edges),
                        quality,
                    }));
                }
                let edge = b.boundary_edges[index].vertices;
                let expected = match b.boundary_edges[index].label {
                    BoundaryLabel::MaterialInterface(_) => 2,
                    BoundaryLabel::Outer(_)
                    | BoundaryLabel::Obstacle(_)
                    | BoundaryLabel::Wall { .. }
                    | BoundaryLabel::InternalBoundary { .. } => 1,
                };
                if b.adjacency
                    .get(&edge_key(edge[0], edge[1]))
                    .is_none_or(|sides| sides.len() != expected)
                {
                    return Err(MeshError::Topology(
                        "constrained edge has incorrect adjacency",
                    ));
                }
                MeshingJobState::VerifyBoundary {
                    index: index + 1,
                    quality,
                }
            }
            MeshingJobState::Done => MeshingJobState::Done,
        };
        Ok(None)
    }
}
