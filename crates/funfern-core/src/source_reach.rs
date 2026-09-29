//! Where a point source's Gaussian reaches. A wall splits the mesh's nodes, so
//! its two faces are different nodes at one place. A source drives the nodes
//! it sees in a straight line, at their distance from it, and nothing behind a
//! wall: not the wall's far face, and not the shadow of a free tip.

use std::collections::HashMap;

use crate::{Point2, PredicateSign, orient2d};

/// Widths beyond which nothing is driven: the Gaussian there,
/// `exp(−81/2) ≈ 2.6e-18`, is under the last bit of its peak.
pub(crate) const POINT_SOURCE_REACH: f64 = 9.0;

/// The squared distance from `position` of each node a point source there
/// drives, or infinity where it drives none: nodes of no element in
/// `elements`, nodes farther than [`POINT_SOURCE_REACH`] widths and nodes it
/// does not see.
///
/// A wall is an element edge no other element shares, which is a face of a
/// baffle, a separated curve or a thin gap, a hole's rim or the outer
/// boundary. A node is seen when no wall properly crosses the segment from
/// the source to it and the segment passes through no point where a wall has
/// split its nodes, and, for a node on a wall, when the source lies in front
/// of that node's face.
pub(crate) fn point_source_squared_distances(
    node_points: &[Point2],
    element_nodes: &[[u32; 7]],
    elements: &[bool],
    position: Point2,
    width: f64,
) -> Vec<f64> {
    let reach = POINT_SOURCE_REACH * width;
    let reach_squared = reach * reach;
    let point = |node: u32| node_points[node as usize];
    // An element holding a point of the disc of reach has a bounding box that
    // meets the disc's, and so does every element sharing an edge that does.
    let local = element_nodes
        .iter()
        .enumerate()
        .filter(|(_, nodes)| {
            let [a, b, c] = [0, 1, 2].map(|vertex| point(nodes[vertex]));
            a.x.min(b.x).min(c.x) <= position.x + reach
                && a.x.max(b.x).max(c.x) >= position.x - reach
                && a.y.min(b.y).min(c.y) <= position.y + reach
                && a.y.max(b.y).max(c.y) >= position.y - reach
        })
        .map(|(element, _)| element)
        .collect::<Vec<_>>();

    // Local edge `k` runs from vertex `k` to vertex `k + 1`, through node `3 + k`.
    let mut edges = HashMap::<(u32, u32), (usize, usize)>::with_capacity(3 * local.len());
    for &element in &local {
        let nodes = element_nodes[element];
        for edge in 0..3 {
            let (a, b) = (nodes[edge], nodes[(edge + 1) % 3]);
            edges
                .entry((a.min(b), a.max(b)))
                .and_modify(|entry| entry.0 += 1)
                .or_insert((1, element * 3 + edge));
        }
    }
    let mut walls = Vec::new();
    let mut on_wall = vec![false; node_points.len()];
    let mut wall_nodes = Vec::new();
    for &(count, key) in edges.values() {
        if count != 1 {
            continue;
        }
        let (element, edge) = (key / 3, key % 3);
        let nodes = element_nodes[element];
        let (a, b) = (nodes[edge], nodes[(edge + 1) % 3]);
        walls.push((point(a), point(b)));
        for node in [a, nodes[3 + edge], b] {
            if !on_wall[node as usize] {
                on_wall[node as usize] = true;
                wall_nodes.push(node);
            }
        }
    }
    // Where a wall has split a point into two nodes, a segment through that
    // point crosses the wall even when it meets no chord properly; a free tip
    // keeps one node and a segment may graze it.
    wall_nodes.sort_by(|left, right| {
        let (left, right) = (point(*left), point(*right));
        left.x.total_cmp(&right.x).then(left.y.total_cmp(&right.y))
    });
    let mut splits = wall_nodes
        .windows(2)
        .filter(|pair| point(pair[0]) == point(pair[1]))
        .map(|pair| point(pair[0]))
        .collect::<Vec<_>>();
    splits.dedup();
    let directions = Directions::new(position, walls.len());
    // A wall in line with the source crosses no segment from it properly.
    let wall_bins = directions.file(
        walls
            .iter()
            .enumerate()
            .filter(|(_, (a, b))| orient2d(*a, *b, position) != PredicateSign::Zero)
            .map(|(index, (a, b))| (index, directions.span(*a, *b))),
    );
    let split_bins = directions.file(
        splits
            .iter()
            .enumerate()
            .filter(|(_, split)| **split != position)
            .map(|(index, split)| (index, directions.span(*split, *split))),
    );

    // The elements holding each wall node, with its place in them.
    let mut faces = HashMap::<u32, Vec<(usize, usize)>>::new();
    let mut included = vec![false; node_points.len()];
    for &element in &local {
        for (place, node) in element_nodes[element].iter().enumerate() {
            if on_wall[*node as usize] {
                faces.entry(*node).or_default().push((element, place));
            }
            included[*node as usize] |= elements[element];
        }
    }
    let seen = |node: usize| {
        let target = node_points[node];
        if target == position {
            return true;
        }
        if on_wall[node]
            && !faces[&(node as u32)].iter().any(|&(element, place)| {
                faces_towards(node_points, element_nodes[element], place, position)
            })
        {
            return false;
        }
        let bin = directions.bin(target);
        !wall_bins.get(bin).iter().any(|&wall| {
            let (a, b) = walls[wall as usize];
            properly_crosses(position, target, a, b)
        }) && !split_bins
            .get(bin)
            .iter()
            .any(|&split| strictly_between(position, target, splits[split as usize]))
    };

    let mut squared = vec![f64::INFINITY; node_points.len()];
    for (node, target) in node_points.iter().enumerate() {
        if !included[node] {
            continue;
        }
        let delta = *target - position;
        let length_squared = delta.dot(delta);
        if length_squared <= reach_squared && seen(node) {
            squared[node] = length_squared;
        }
    }
    squared
}

/// Directions from the source cut into equal bins of the pseudo-angle, which
/// orders them as the angle does with no transcendental function, so a node
/// tests only the walls and split points filed under its own direction.
struct Directions {
    position: Point2,
    count: usize,
}

/// Items filed under bins: bin `b` holds `entries[start[b]..start[b + 1]]`.
struct Filed {
    start: Vec<u32>,
    entries: Vec<u32>,
}

impl Filed {
    fn get(&self, bin: usize) -> &[u32] {
        &self.entries[self.start[bin] as usize..self.start[bin + 1] as usize]
    }
}

impl Directions {
    fn new(position: Point2, walls: usize) -> Self {
        Self {
            position,
            count: (2 * walls).clamp(64, 1 << 14),
        }
    }

    /// The pseudo-angle of `point` from the source, over `(0, 4]`.
    fn key(&self, point: Point2) -> f64 {
        let delta = point - self.position;
        crate::pseudo_angle(delta.x, delta.y) + 2.0
    }

    /// The bin of the direction to `point`; `−x`, the top of the key's range,
    /// is next to the bottom and wraps to the first bin.
    fn bin(&self, point: Point2) -> usize {
        (self.key(point) / 4.0 * self.count as f64) as usize % self.count
    }

    /// The bins, first and last, that the directions from `a` round to `b`
    /// the short way run through, a bin wider either side for the rounding. A
    /// span so near half a turn that rounding could take the long way instead
    /// covers every bin.
    fn span(&self, a: Point2, b: Point2) -> (usize, usize) {
        let (first, second) = (self.key(a), self.key(b));
        let (low, high) = (first.min(second), first.max(second));
        let bin = |key: f64| (key / 4.0 * self.count as f64) as usize;
        if (high - low - 2.0).abs() < 1.0e-9 {
            (0, self.count - 1)
        } else if high - low < 2.0 {
            (bin(low) + self.count - 1, bin(high) + 1)
        } else {
            (bin(high) + self.count - 1, bin(low) + self.count + 1)
        }
    }

    /// Files each item under every bin of its span, which wraps past the last.
    fn file(&self, items: impl Iterator<Item = (usize, (usize, usize))>) -> Filed {
        let mut pairs = Vec::new();
        for (item, (first, last)) in items {
            let bins = (last + self.count - first) % self.count + 1;
            for offset in 0..bins.min(self.count) {
                pairs.push(((first + offset) % self.count, item as u32));
            }
        }
        pairs.sort_unstable();
        let mut start = vec![0_u32; self.count + 1];
        for (bin, _) in &pairs {
            start[bin + 1] += 1;
        }
        for bin in 0..self.count {
            start[bin + 1] += start[bin];
        }
        Filed {
            start,
            entries: pairs.into_iter().map(|(_, item)| item).collect(),
        }
    }
}

/// Whether the direction from the node at `place` in the element `nodes`
/// towards `source` points into that element: into its angle at a vertex, or
/// to its side of the edge at a midpoint.
fn faces_towards(node_points: &[Point2], nodes: [u32; 7], place: usize, source: Point2) -> bool {
    let vertex = |index: usize| node_points[nodes[index % 3] as usize];
    let orientation = orient2d(vertex(0), vertex(1), vertex(2));
    let agrees = |sign: PredicateSign| sign == PredicateSign::Zero || sign == orientation;
    match place {
        0..=2 => {
            let (at, next, previous) = (vertex(place), vertex(place + 1), vertex(place + 2));
            agrees(orient2d(at, next, source)) && agrees(orient2d(at, source, previous))
        }
        3..=5 => agrees(orient2d(vertex(place - 3), vertex(place - 2), source)),
        _ => true,
    }
}

fn opposite(left: PredicateSign, right: PredicateSign) -> bool {
    matches!(
        (left, right),
        (PredicateSign::Positive, PredicateSign::Negative)
            | (PredicateSign::Negative, PredicateSign::Positive)
    )
}

/// Whether the segments `start`–`end` and `a`–`b` cross at a point interior to both.
fn properly_crosses(start: Point2, end: Point2, a: Point2, b: Point2) -> bool {
    opposite(orient2d(a, b, start), orient2d(a, b, end))
        && opposite(orient2d(start, end, a), orient2d(start, end, b))
}

/// Whether `point` lies on the segment `start`–`end` short of both its ends.
fn strictly_between(start: Point2, end: Point2, point: Point2) -> bool {
    point != start
        && point != end
        && orient2d(start, end, point) == PredicateSign::Zero
        && (point - start).dot(end - start) > 0.0
        && (point - end).dot(start - end) > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BACKGROUND_REGION, InternalBoundary, InternalBoundaryId, InternalBoundaryLaw,
        MeshingOptions, Obstacle, ObstacleId, OpenCubicSpline, OuterBoundaryCondition,
        PeriodicCubicSpline, QuadraticWaveOperator, Scene, mesh_scene,
    };

    fn operator(scene: &Scene, edge: f64) -> QuadraticWaveOperator {
        let mesh = mesh_scene(
            scene,
            3,
            MeshingOptions {
                target_edge_length: edge,
                minimum_angle_degrees: 14.0,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        QuadraticWaveOperator::assemble_scene(&mesh, scene, OuterBoundaryCondition::Reflecting)
            .unwrap()
    }

    /// A reflecting baffle along `y = 0` through the given x.
    fn baffle(xs: [f64; 4]) -> Scene {
        let mut scene = Scene::default();
        scene.internal_boundaries.push(InternalBoundary {
            id: InternalBoundaryId(1),
            spline: OpenCubicSpline::uniform(xs.map(|x| Point2::new(x, 0.0)).to_vec()).unwrap(),
            region: BACKGROUND_REGION,
            span_laws: vec![InternalBoundaryLaw::REFLECTING],
        });
        scene
    }

    fn distances(operator: &QuadraticWaveOperator, position: Point2, width: f64) -> Vec<f64> {
        point_source_squared_distances(
            operator.node_points(),
            operator.element_nodes(),
            &vec![true; operator.element_nodes().len()],
            position,
            width,
        )
    }

    fn squared(a: Point2, b: Point2) -> f64 {
        let delta = a - b;
        delta.dot(delta)
    }

    /// Both faces of a baffle are nodes at one place: the face towards the
    /// source is read in a straight line and the one behind it not at all,
    /// nor is anything else behind the baffle, whose tips are out of reach.
    #[test]
    fn a_source_drives_only_its_side_of_a_baffle() {
        let operator = operator(&baffle([-0.72, -0.25, 0.25, 0.72]), 0.1);
        let (position, width) = (Point2::new(0.0, 0.05), 0.06);
        let reach = POINT_SOURCE_REACH * width;
        let distances = distances(&operator, position, width);
        let mut faces = 0;
        for (node, point) in operator.node_points().iter().enumerate() {
            let plain = squared(*point, position);
            if plain > reach * reach || point.y < 0.0 {
                assert_eq!(distances[node], f64::INFINITY, "{point:?}");
            } else if point.y > 0.0 {
                assert_eq!(distances[node], plain, "{point:?}");
            } else {
                let twins = operator
                    .node_points()
                    .iter()
                    .enumerate()
                    .filter(|(_, other)| *other == point)
                    .map(|(other, _)| distances[other])
                    .collect::<Vec<_>>();
                assert_eq!(twins.len(), 2, "{point:?}");
                assert!(twins.contains(&plain) && twins.contains(&f64::INFINITY));
                faces += 1;
            }
        }
        assert!(faces > 10, "{faces} face nodes in reach");
    }

    /// A free tip near the source casts a shadow the source does not drive;
    /// past the tip, and along the line it grazes, it drives as in the open.
    #[test]
    fn a_free_tips_shadow_is_not_driven() {
        let operator = operator(&baffle([0.0, 0.3, 0.6, 0.9]), 0.05);
        let (position, width) = (Point2::new(0.08, 0.04), 0.05);
        let reach = POINT_SOURCE_REACH * width;
        let distances = distances(&operator, position, width);
        let (mut shadowed, mut past) = (0, 0);
        for (node, point) in operator.node_points().iter().enumerate() {
            let plain = squared(*point, position);
            // Where the straight line meets y = 0, if it runs to the other side.
            let crossing =
                position.x + (point.x - position.x) * position.y / (position.y - point.y);
            if plain > reach * reach || (point.y < 0.0 && crossing.abs() < 1.0e-9) {
                continue;
            }
            if point.y < 0.0 && crossing > 0.0 {
                assert_eq!(distances[node], f64::INFINITY, "{point:?}");
                shadowed += 1;
            } else if point.y == 0.0 && point.x > 0.0 {
                let twins = operator
                    .node_points()
                    .iter()
                    .enumerate()
                    .filter(|(_, other)| *other == point)
                    .map(|(other, _)| distances[other])
                    .collect::<Vec<_>>();
                assert_eq!(twins.len(), 2, "{point:?}");
                assert!(twins.contains(&plain) && twins.contains(&f64::INFINITY));
            } else {
                assert_eq!(distances[node], plain, "{point:?}");
                past += usize::from(point.y < 0.0);
            }
        }
        assert!(
            shadowed > 20 && past > 20,
            "{shadowed} shadowed, {past} past the tip"
        );
    }

    /// With no wall in reach the source reads every node in a straight line,
    /// bit for bit, and nothing beyond its reach or outside its elements.
    #[test]
    fn a_source_without_walls_keeps_the_plain_gaussian_within_its_reach() {
        let operator = operator(&Scene::default(), 0.1);
        let (position, width) = (Point2::new(0.1, -0.2), 0.15);
        let reach = POINT_SOURCE_REACH * width;
        let elements = operator
            .element_nodes()
            .iter()
            .map(|nodes| operator.node_points()[nodes[6] as usize].x < 0.3)
            .collect::<Vec<_>>();
        let mut included = vec![false; operator.degrees_of_freedom()];
        for (nodes, inside) in operator.element_nodes().iter().zip(&elements) {
            if *inside {
                nodes
                    .iter()
                    .for_each(|node| included[*node as usize] = true);
            }
        }
        let distances = point_source_squared_distances(
            operator.node_points(),
            operator.element_nodes(),
            &elements,
            position,
            width,
        );
        let (mut beyond, mut outside) = (0, 0);
        for (node, point) in operator.node_points().iter().enumerate() {
            let plain = squared(*point, position);
            if !included[node] {
                outside += 1;
                assert_eq!(distances[node], f64::INFINITY);
            } else if plain > reach * reach {
                beyond += 1;
                assert_eq!(distances[node], f64::INFINITY);
            } else {
                assert_eq!(distances[node], plain, "{point:?}");
            }
        }
        assert!(beyond > 0 && outside > 0);
    }

    /// A hole's rim is a wall: the source does not drive what lies behind it.
    #[test]
    fn a_hole_hides_its_far_side() {
        let mut scene = Scene::default();
        scene.obstacles.push(Obstacle::hole(
            ObstacleId(1),
            PeriodicCubicSpline::rounded(Point2::new(0.0, 0.0), 0.2),
        ));
        let operator = operator(&scene, 0.05);
        let (position, width) = (Point2::new(-0.25, 0.0), 0.08);
        let distances = distances(&operator, position, width);
        let mut behind = 0;
        for (node, point) in operator.node_points().iter().enumerate() {
            if point.x > 0.1 && point.y.abs() < 0.05 {
                assert_eq!(distances[node], f64::INFINITY, "{point:?}");
                behind += 1;
            }
        }
        assert!(behind > 0);
    }
}
