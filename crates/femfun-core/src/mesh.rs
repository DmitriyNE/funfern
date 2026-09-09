use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::{
    ObstacleId, Point2, PredicateSign, Sample, Sampler, SamplingOptions, ValidationIssue,
    ValidationJob, incircle, orient2d,
};
use crate::{Scene, WORLD_TOLERANCE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OuterSide {
    Bottom,
    Right,
    Top,
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryLabel {
    Outer(OuterSide),
    Obstacle(ObstacleId),
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

struct MeshBuilder {
    vertices: Vec<MeshVertex>,
    triangles: Vec<MeshTriangle>,
    boundary_edges: Vec<BoundaryEdge>,
    boundary_keys: BTreeSet<(usize, usize)>,
    domain_loops: Vec<Polygon>,
    options: MeshingOptions,
    adjacency: BTreeMap<(usize, usize), Vec<(usize, usize)>>,
    dirty_edges: VecDeque<(usize, usize)>,
    queued_edges: BTreeSet<(usize, usize)>,
    bad_triangles: BTreeSet<(u64, usize)>,
    scores: Vec<Option<u64>>,
    stats: MeshingStats,
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
            options,
            adjacency: BTreeMap::new(),
            dirty_edges: VecDeque::new(),
            queued_edges: BTreeSet::new(),
            bad_triangles: BTreeSet::new(),
            scores: vec![],
            stats: MeshingStats::default(),
        }
    }

    fn queue_edge(&mut self, edge: (usize, usize)) {
        if self.queued_edges.insert(edge) {
            self.dirty_edges.push_back(edge);
        }
    }

    fn register_triangle(&mut self, index: usize) {
        let triangle = self.triangles[index];
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
        if quality.maximum_edge_length > self.options.target_edge_length * 1.05
            || quality.minimum_angle_degrees + 1.0e-9 < self.options.minimum_angle_degrees
        {
            // Nonnegative IEEE floats have the same ordering as their bit patterns.
            let score = (quality.maximum_edge_length / self.options.target_edge_length)
                .max(self.options.minimum_angle_degrees / quality.minimum_angle_degrees)
                .to_bits();
            self.scores[index] = Some(score);
            self.bad_triangles.insert((score, index));
        }
    }

    fn replace_triangle(&mut self, index: usize, triangle: MeshTriangle) {
        let old = self.triangles[index];
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

    fn ccw_triangle(&self, vertices: [usize; 3]) -> Result<MeshTriangle, MeshError> {
        match orient2d(
            self.point(vertices[0]),
            self.point(vertices[1]),
            self.point(vertices[2]),
        ) {
            PredicateSign::Positive => Ok(MeshTriangle { vertices }),
            PredicateSign::Negative => Ok(MeshTriangle {
                vertices: [vertices[0], vertices[2], vertices[1]],
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
    ) -> Result<Polygon, MeshError> {
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
        let label = BoundaryLabel::Obstacle(obstacle.id);
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
        Ok(Polygon { vertices })
    }

    fn in_domain(&self, point: Point2) -> bool {
        if self.domain_loops.is_empty() {
            return false;
        }
        let points = |polygon: &Polygon| {
            polygon
                .vertices
                .iter()
                .map(|index| self.point(*index))
                .collect::<Vec<_>>()
        };
        if locate_in_polygon(point, &points(&self.domain_loops[0])) == PolygonLocation::Outside {
            return false;
        }
        self.domain_loops[1..]
            .iter()
            .all(|hole| locate_in_polygon(point, &points(hole)) == PolygonLocation::Outside)
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
        let left = self.ccw_triangle([c, d, a])?;
        let right = self.ccw_triangle([d, c, b])?;
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
        self.triangles
            .iter()
            .enumerate()
            .find_map(|(index, triangle)| {
                let location = point_in_triangle(point, self.triangle_points(*triangle));
                (location != PolygonLocation::Outside).then_some((index, location))
            })
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
        for (index, opposite) in adjacent.into_iter().rev() {
            self.replace_triangle(index, self.ccw_triangle([edge[0], vertex, opposite])?);
            self.push_triangle(self.ccw_triangle([vertex, edge[1], opposite])?)?;
        }
        Ok(())
    }

    fn insert_point(&mut self, point: Point2) -> Result<(), MeshError> {
        if self
            .vertices
            .iter()
            .any(|vertex| (vertex.point - point).norm() <= self.options.curve_tolerance * 0.1)
        {
            return Err(MeshError::Topology(
                "refinement point duplicates an existing vertex",
            ));
        }
        let (triangle_index, location) = self.containing_triangle(point).ok_or(
            MeshError::Topology("refinement point is outside the domain"),
        )?;
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
                self.ccw_triangle([triangle.vertices[0], triangle.vertices[1], vertex])?,
            );
            self.push_triangle(self.ccw_triangle([
                triangle.vertices[1],
                triangle.vertices[2],
                vertex,
            ])?)?;
            self.push_triangle(self.ccw_triangle([
                triangle.vertices[2],
                triangle.vertices[0],
                vertex,
            ])?)?;
            Ok(())
        }
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
        let points = self.triangle_points(triangle);
        let centroid = (points[0] + points[1] + points[2]) / 3.0;
        let mut candidate = self
            .circumcenter(triangle)
            .filter(|point| self.containing_triangle(*point).is_some())
            .unwrap_or(centroid);
        if !self.in_domain(candidate) {
            candidate = centroid;
        }
        if let Some(edge_index) = self.boundary_edges.iter().position(|edge| {
            let a = self.point(edge.vertices[0]);
            let b = self.point(edge.vertices[1]);
            (candidate - a).dot(candidate - b) < 0.0
        }) {
            self.split_boundary(edge_index)?;
        } else {
            self.insert_point(candidate)?;
        }
        self.stats.refinement_insertions += 1;
        Ok(false)
    }
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
    hole: usize,
    outer_index: usize,
    hole_index: usize,
    best: Option<(f64, usize, usize)>,
    visibility: Option<(u8, usize, f64)>,
    seeding: bool,
    testing_seed: bool,
}

struct EarSearch {
    polygon: Vec<usize>,
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
    Bridge(BridgeSearch),
    Clip(EarSearch),
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
            MeshingJobState::Outer | MeshingJobState::Sample { .. } => "Sampling boundaries",
            MeshingJobState::Bridge(_) => "Connecting holes",
            MeshingJobState::Clip(_) => "Triangulating",
            MeshingJobState::Legalize => "Legalizing edges",
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
                MeshingJobState::Sample {
                    obstacle: 0,
                    sampler: None,
                }
            }
            MeshingJobState::Sample { obstacle, sampler } => {
                if obstacle == self.scene.obstacles.len() {
                    MeshingJobState::Bridge(BridgeSearch {
                        polygon: b.domain_loops[0].vertices.clone(),
                        hole: 1,
                        outer_index: 0,
                        hole_index: 0,
                        best: None,
                        visibility: None,
                        seeding: true,
                        testing_seed: false,
                    })
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
                        let polygon = b.add_obstacle(&self.scene.obstacles[obstacle], samples)?;
                        b.domain_loops.push(polygon);
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
            MeshingJobState::Bridge(mut search) => {
                if search.hole == b.domain_loops.len() {
                    MeshingJobState::Clip(EarSearch {
                        polygon: search.polygon,
                        index: 0,
                        degenerate_pass: false,
                        stage: EarStage::Start,
                    })
                } else {
                    let hole = &b.domain_loops[search.hole].vertices;
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
                                b.boundary_edges.get(index).map(|edge| edge.vertices)
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
                                if b.in_domain(a.lerp(v, 0.5)) {
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
                    b.push_triangle(b.ccw_triangle([polygon[0], polygon[1], polygon[2]])?)?;
                    MeshingJobState::Legalize
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
                    if !b.in_domain((a + v + c) / 3.0) {
                        return Err(MeshError::Topology(
                            "triangle was classified outside the domain",
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
                        let expected = if b.boundary_keys.contains(&edge) {
                            1
                        } else {
                            2
                        };
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
                if b.adjacency
                    .get(&edge_key(edge[0], edge[1]))
                    .is_none_or(|sides| sides.len() != 1)
                {
                    return Err(MeshError::Topology(
                        "boundary edge is not represented exactly once",
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
