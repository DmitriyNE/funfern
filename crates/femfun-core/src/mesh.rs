use std::collections::{BTreeMap, BTreeSet};

use crate::{
    ObstacleId, Point2, PredicateSign, SamplingOptions, ValidationIssue, incircle, orient2d,
    sample, validate,
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
        }
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

    fn add_obstacle(&mut self, obstacle: &crate::Obstacle) -> Result<Polygon, MeshError> {
        let sampled = sample(
            &obstacle.spline,
            SamplingOptions {
                tolerance: self.options.curve_tolerance,
                max_depth: 18,
                max_points: 8_192,
            },
        )
        .map_err(|_| MeshError::Sampling(obstacle.id))?;
        let mut points = Vec::new();
        for span in sampled.windows(2) {
            let length = (span[1].point - span[0].point).norm();
            let pieces = (length / self.options.target_edge_length).ceil().max(1.0) as usize;
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

    fn bridge_visible(
        &self,
        outer: &[usize],
        outer_index: usize,
        hole: &[usize],
        hole_index: usize,
    ) -> bool {
        let a_index = outer[outer_index];
        let b_index = hole[hole_index];
        let a = self.point(a_index);
        let b = self.point(b_index);
        if a == b {
            return false;
        }
        for edge in &self.boundary_edges {
            if edge.vertices.contains(&a_index) || edge.vertices.contains(&b_index) {
                continue;
            }
            if segment_relation(
                a,
                b,
                self.point(edge.vertices[0]),
                self.point(edge.vertices[1]),
            ) != SegmentRelation::Disjoint
            {
                return false;
            }
        }
        for index in 0..outer.len() {
            let edge = [outer[index], outer[(index + 1) % outer.len()]];
            if edge.contains(&a_index) || edge.contains(&b_index) {
                continue;
            }
            if segment_relation(a, b, self.point(edge[0]), self.point(edge[1]))
                == SegmentRelation::ProperIntersection
            {
                return false;
            }
        }
        let midpoint = a.lerp(b, 0.5);
        self.in_domain(midpoint)
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

    fn bridge_hole(&self, outer: &mut Vec<usize>, hole: &[usize]) -> Result<(), MeshError> {
        let mut candidates = vec![];
        for outer_index in 0..outer.len() {
            for hole_index in 0..hole.len() {
                if self.bridge_visible(outer, outer_index, hole, hole_index) {
                    let length =
                        (self.point(outer[outer_index]) - self.point(hole[hole_index])).norm();
                    candidates.push((length, outer_index, hole_index));
                }
            }
        }
        let (_, outer_index, hole_index) = candidates
            .into_iter()
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .ok_or(MeshError::Topology("no visible bridge to obstacle"))?;
        let mut splice = Vec::with_capacity(hole.len() + 2);
        for offset in 0..hole.len() {
            splice.push(hole[(hole_index + offset) % hole.len()]);
        }
        splice.push(hole[hole_index]);
        splice.push(outer[outer_index]);
        outer.splice(outer_index + 1..outer_index + 1, splice);
        Ok(())
    }

    fn diagonal_clear(&self, polygon: &[usize], left: usize, right: usize) -> bool {
        let a = polygon[left];
        let b = polygon[right];
        for index in 0..polygon.len() {
            let c = polygon[index];
            let d = polygon[(index + 1) % polygon.len()];
            if c == a || c == b || d == a || d == b {
                continue;
            }
            if segment_relation(self.point(a), self.point(b), self.point(c), self.point(d))
                != SegmentRelation::Disjoint
            {
                return false;
            }
        }
        true
    }

    fn triangulate_polygon(&mut self, polygon: Vec<usize>) -> Result<(), MeshError> {
        let mut polygon = polygon;
        let mut attempts_without_progress = 0;
        while polygon.len() > 3 {
            let mut clipped = false;
            for index in 0..polygon.len() {
                let previous = (index + polygon.len() - 1) % polygon.len();
                let next = (index + 1) % polygon.len();
                let [a, b, c] = [polygon[previous], polygon[index], polygon[next]];
                if a == b || b == c || a == c {
                    continue;
                }
                if orient2d(self.point(a), self.point(b), self.point(c)) != PredicateSign::Positive
                    || !self.diagonal_clear(&polygon, previous, next)
                {
                    continue;
                }
                let triangle = [self.point(a), self.point(b), self.point(c)];
                let contains_vertex = polygon.iter().enumerate().any(|(other_index, other)| {
                    if [previous, index, next].contains(&other_index) || [a, b, c].contains(other) {
                        return false;
                    }
                    point_in_triangle(self.point(*other), triangle) != PolygonLocation::Outside
                });
                if contains_vertex {
                    continue;
                }
                self.triangles.push(MeshTriangle {
                    vertices: [a, b, c],
                });
                polygon.remove(index);
                clipped = true;
                break;
            }
            if clipped {
                attempts_without_progress = 0;
                if self.triangles.len() > self.options.max_triangles {
                    return Err(self.capacity_error());
                }
                continue;
            }
            attempts_without_progress += 1;
            // Weakly-simple bridge polygons contain duplicated bridge endpoints.
            // Remove a zero-area occurrence once all ordinary ears are exhausted.
            if let Some(index) = (0..polygon.len()).find(|index| {
                let previous = (*index + polygon.len() - 1) % polygon.len();
                let next = (*index + 1) % polygon.len();
                let a = polygon[previous];
                let b = polygon[*index];
                let c = polygon[next];
                (a == b
                    || b == c
                    || a == c
                    || orient2d(self.point(a), self.point(b), self.point(c)) == PredicateSign::Zero)
                    && self.diagonal_clear(&polygon, previous, next)
            }) {
                polygon.remove(index);
            } else {
                return Err(MeshError::Topology("ear clipping stalled"));
            }
            if attempts_without_progress > polygon.len() + 2 {
                return Err(MeshError::Topology("degenerate bridge polygon"));
            }
        }
        if polygon.len() == 3 {
            self.triangles
                .push(self.ccw_triangle([polygon[0], polygon[1], polygon[2]])?);
        }
        Ok(())
    }

    fn adjacency(&self) -> BTreeMap<(usize, usize), Vec<(usize, usize)>> {
        let mut adjacency: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for (triangle_index, triangle) in self.triangles.iter().enumerate() {
            for opposite in 0..3 {
                let a = triangle.vertices[(opposite + 1) % 3];
                let b = triangle.vertices[(opposite + 2) % 3];
                adjacency
                    .entry(edge_key(a, b))
                    .or_default()
                    .push((triangle_index, triangle.vertices[opposite]));
            }
        }
        adjacency
    }

    fn legalize(&mut self) -> Result<(), MeshError> {
        let max_sweeps = 128;
        for _ in 0..max_sweeps {
            let adjacency = self.adjacency();
            let mut flips = vec![];
            let mut claimed_triangles = BTreeSet::new();
            for ((a, b), sides) in adjacency {
                if sides.len() != 2 || self.boundary_keys.contains(&(a, b)) {
                    continue;
                }
                let [(left_index, c), (right_index, d)] = [sides[0], sides[1]];
                let pa = self.point(a);
                let pb = self.point(b);
                let pc = self.point(c);
                let pd = self.point(d);
                if orient2d(pa, pb, pc) == orient2d(pa, pb, pd) {
                    continue;
                }
                let left = self.triangles[left_index];
                let [x, y, z] = left.vertices.map(|vertex| self.point(vertex));
                if incircle(x, y, z, pd) != PredicateSign::Positive {
                    continue;
                }
                if !claimed_triangles.contains(&left_index)
                    && !claimed_triangles.contains(&right_index)
                {
                    claimed_triangles.insert(left_index);
                    claimed_triangles.insert(right_index);
                    flips.push((left_index, right_index, [c, d, a], [d, c, b]));
                }
            }
            if flips.is_empty() {
                return Ok(());
            }
            for (left_index, right_index, left, right) in flips {
                self.triangles[left_index] = self.ccw_triangle(left)?;
                self.triangles[right_index] = self.ccw_triangle(right)?;
            }
        }
        Err(MeshError::Topology("edge legalization did not converge"))
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
        let adjacent: Vec<_> = self
            .triangles
            .iter()
            .enumerate()
            .filter_map(|(index, triangle)| {
                (triangle.vertices.contains(&edge[0]) && triangle.vertices.contains(&edge[1]))
                    .then_some((index, *triangle))
            })
            .collect();
        if adjacent.is_empty() || adjacent.len() > 2 {
            return Err(MeshError::Topology("edge has invalid triangle adjacency"));
        }
        for (index, triangle) in adjacent.into_iter().rev() {
            let opposite = triangle
                .vertices
                .into_iter()
                .find(|candidate| !edge.contains(candidate))
                .unwrap();
            self.triangles[index] = self.ccw_triangle([edge[0], vertex, opposite])?;
            self.triangles
                .push(self.ccw_triangle([vertex, edge[1], opposite])?);
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
            self.triangles[triangle_index] =
                self.ccw_triangle([triangle.vertices[0], triangle.vertices[1], vertex])?;
            self.triangles.push(self.ccw_triangle([
                triangle.vertices[1],
                triangle.vertices[2],
                vertex,
            ])?);
            self.triangles.push(self.ccw_triangle([
                triangle.vertices[2],
                triangle.vertices[0],
                vertex,
            ])?);
            Ok(())
        }
    }

    /// Performs one bounded refinement insertion. Returns true when all quality
    /// targets are satisfied.
    fn refine_once(&mut self) -> Result<bool, MeshError> {
        let bad = self
            .triangles
            .iter()
            .enumerate()
            .map(|(index, triangle)| (index, self.triangle_quality(*triangle)))
            .filter(|(_, quality)| {
                quality.maximum_edge_length > self.options.target_edge_length * 1.05
                    || quality.minimum_angle_degrees + 1.0e-9 < self.options.minimum_angle_degrees
            })
            .max_by(|a, b| {
                let a_score = (a.1.maximum_edge_length / self.options.target_edge_length)
                    .max(self.options.minimum_angle_degrees / a.1.minimum_angle_degrees);
                let b_score = (b.1.maximum_edge_length / self.options.target_edge_length)
                    .max(self.options.minimum_angle_degrees / b.1.minimum_angle_degrees);
                a_score.total_cmp(&b_score)
            });
        let Some((triangle_index, _)) = bad else {
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
        self.legalize()?;
        Ok(false)
    }

    fn verify(&self) -> Result<(), MeshError> {
        if self.triangles.len() > self.options.max_triangles
            || self.vertices.len() > self.options.max_vertices
        {
            return Err(self.capacity_error());
        }
        let adjacency = self.adjacency();
        for edge in &self.boundary_edges {
            if adjacency
                .get(&edge_key(edge.vertices[0], edge.vertices[1]))
                .is_none_or(|triangles| triangles.len() != 1)
            {
                return Err(MeshError::Topology(
                    "boundary edge is not represented exactly once",
                ));
            }
        }
        for (edge, triangles) in adjacency {
            let expected = if self.boundary_keys.contains(&edge) {
                1
            } else {
                2
            };
            if triangles.len() != expected {
                return Err(MeshError::Topology("mesh has a crack or non-manifold edge"));
            }
        }
        for triangle in &self.triangles {
            let [a, b, c] = self.triangle_points(*triangle);
            if orient2d(a, b, c) != PredicateSign::Positive {
                return Err(MeshError::Topology("mesh contains an inverted triangle"));
            }
            if !self.in_domain((a + b + c) / 3.0) {
                return Err(MeshError::Topology(
                    "triangle was classified outside the domain",
                ));
            }
        }
        Ok(())
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

/// Builds a constrained, locally Delaunay triangle mesh of the fixed square
/// minus all obstacle interiors. Every sampled geometry segment survives as a
/// labeled mesh edge. Refinement has explicit vertex, triangle, and step limits.
pub fn mesh_scene(
    scene: &Scene,
    geometry_revision: u64,
    options: MeshingOptions,
) -> Result<TriMesh, MeshError> {
    let mut builder = prepare_builder(scene, options)?;
    for _ in 0..options.max_refinement_steps {
        if builder.refine_once()? {
            return finish_builder(builder, geometry_revision);
        }
    }
    Err(MeshError::RefinementLimit(builder.quality()))
}

fn prepare_builder(scene: &Scene, options: MeshingOptions) -> Result<MeshBuilder, MeshError> {
    if !valid_options(options) {
        return Err(MeshError::InvalidOptions);
    }
    let validation = validate(scene);
    if let Some(issue) = validation.issue {
        return Err(MeshError::InvalidGeometry(issue));
    }
    let mut builder = MeshBuilder::new(options);
    let outer = builder.add_outer()?;
    builder.domain_loops.push(outer.clone());
    for obstacle in &scene.obstacles {
        let hole = builder.add_obstacle(obstacle)?;
        builder.domain_loops.push(hole);
    }
    let mut stitched = outer.vertices;
    for hole in &builder.domain_loops[1..] {
        builder.bridge_hole(&mut stitched, &hole.vertices)?;
    }
    builder.triangulate_polygon(stitched)?;
    builder.legalize()?;
    Ok(builder)
}

fn finish_builder(builder: MeshBuilder, geometry_revision: u64) -> Result<TriMesh, MeshError> {
    builder.verify()?;
    let quality = builder.quality();
    Ok(TriMesh {
        geometry_revision,
        vertices: builder.vertices,
        triangles: builder.triangles,
        boundary_edges: builder.boundary_edges,
        quality,
    })
}

enum MeshingJobState {
    Pending(Option<Scene>),
    Refining(Option<MeshBuilder>),
    Done,
}

/// Cooperative mesh construction. Topology preparation is one bounded phase;
/// each subsequent work unit performs at most one quality-refinement insertion
/// and its local-Delaunay legalization. Replacing the job discards obsolete work.
pub struct MeshingJob {
    geometry_revision: u64,
    options: MeshingOptions,
    refinement_steps: usize,
    state: MeshingJobState,
}

impl MeshingJob {
    pub fn new(scene: Scene, geometry_revision: u64, options: MeshingOptions) -> Self {
        Self {
            geometry_revision,
            options,
            refinement_steps: 0,
            state: MeshingJobState::Pending(Some(scene)),
        }
    }

    pub fn advance(&mut self, budget: usize) -> Option<Result<TriMesh, MeshError>> {
        for _ in 0..budget {
            let state = std::mem::replace(&mut self.state, MeshingJobState::Done);
            match state {
                MeshingJobState::Pending(mut scene) => {
                    match prepare_builder(scene.take().as_ref().unwrap(), self.options) {
                        Ok(builder) => self.state = MeshingJobState::Refining(Some(builder)),
                        Err(error) => return Some(Err(error)),
                    }
                }
                MeshingJobState::Refining(mut builder) => {
                    let mut builder = builder.take().unwrap();
                    if self.refinement_steps >= self.options.max_refinement_steps {
                        return Some(Err(MeshError::RefinementLimit(builder.quality())));
                    }
                    match builder.refine_once() {
                        Ok(true) => return Some(finish_builder(builder, self.geometry_revision)),
                        Ok(false) => {
                            self.refinement_steps += 1;
                            self.state = MeshingJobState::Refining(Some(builder));
                        }
                        Err(error) => return Some(Err(error)),
                    }
                }
                MeshingJobState::Done => return None,
            }
        }
        None
    }
}
