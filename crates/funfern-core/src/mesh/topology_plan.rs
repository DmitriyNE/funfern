//! Mesh-facing projection of a compiled curve arrangement.
//!
//! The projection resolves each geometric arrangement vertex into one or more
//! finite-element trace vertices. It is deliberately independent from the old
//! obstacle/interface/baffle labels so the triangulator and solver can migrate
//! without reconstructing sectors from coordinates.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    BoundaryEdge, BoundaryLabel, BoundaryPoint, MeshBuilder, MeshError, MeshTriangle,
    OpenConstraintKind, PolygonLocation, SegmentRelation, TriMesh, TriangulationDomain, edge_key,
    point_in_triangle, segment_relation, valid_options,
};
use crate::{
    CompiledBoundaryStep, CompiledEdge, CompiledEdgeSource, CurveId, CurveSpanId, FaceId, Point2,
    PredicateSign, RegionId, SpanBehavior, TopologySnapshot, TraceVertexId, orient2d,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceRegionAssignment {
    pub face: FaceId,
    /// `None` excludes the face from the simulation, as for a hole.
    pub region: Option<RegionId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CurveTraceSide {
    Left,
    Right,
}

#[derive(Clone, Debug)]
struct ExpandedChain {
    traces: [TraceVertexId; 2],
    vertices: Vec<usize>,
    parameters: Vec<f64>,
}

/// Builds a constrained triangular mesh directly from the compiled topology
/// contract. This is the robust full-rebuild baseline for the unified model;
/// it never reconstructs legacy obstacle, divider, or baffle objects.
pub fn mesh_topology_plan(
    plan: &TopologyMeshPlan,
    mesh_revision: u64,
    options: super::MeshingOptions,
) -> Result<TriMesh, MeshError> {
    if !valid_options(options) {
        return Err(MeshError::InvalidOptions);
    }
    let mut builder = MeshBuilder::new(options, plan.domain);
    let trace_points = plan
        .vertices
        .iter()
        .map(|vertex| (vertex.id, vertex.point))
        .collect::<BTreeMap<_, _>>();
    let mut trace_vertices = BTreeMap::new();
    let mut chains = BTreeMap::<(usize, Option<CurveTraceSide>), ExpandedChain>::new();
    let mut free_slits = BTreeMap::<(RegionId, CurveId), Vec<PlannedFaceStep>>::new();

    for domain in &plan.domains {
        let mut edge_counts = BTreeMap::<usize, usize>::new();
        for step in domain.steps.iter().flatten() {
            *edge_counts.entry(step.edge).or_default() += 1;
        }
        let is_slit = |step: &PlannedFaceStep| {
            edge_counts.get(&step.edge).copied().unwrap_or(0) > 1
                && matches!(step.boundary.behavior, Some(SpanBehavior::Separated { .. }))
        };
        for step in domain.steps.iter().flatten().filter(|step| is_slit(step)) {
            let PlannedBoundarySource::Curve { curve, .. } = step.boundary.source else {
                continue;
            };
            free_slits
                .entry((domain.region, curve))
                .or_default()
                .push(*step);
        }
        let mut cycles = Vec::with_capacity(domain.steps.len());
        for cycle in &domain.steps {
            let mut polygon = vec![];
            for step in cycle {
                if is_slit(step) {
                    continue;
                }
                let separated =
                    matches!(step.boundary.behavior, Some(SpanBehavior::Separated { .. }));
                let side = match step.boundary.source {
                    PlannedBoundarySource::Curve { side, .. } if separated => Some(side),
                    _ => None,
                };
                let key = (step.edge, side);
                if let std::collections::btree_map::Entry::Vacant(entry) = chains.entry(key) {
                    let chain =
                        expand_step_chain(&mut builder, &trace_points, &mut trace_vertices, step)?;
                    entry.insert(chain);
                }
                let chain = &chains[&key];
                let (oriented, parameters) = if chain.traces == step.boundary.traces {
                    (chain.vertices.clone(), chain.parameters.clone())
                } else if chain.traces == [step.boundary.traces[1], step.boundary.traces[0]] {
                    (
                        chain.vertices.iter().rev().copied().collect(),
                        chain.parameters.iter().rev().copied().collect(),
                    )
                } else {
                    return Err(MeshError::Topology("topology trace chain is inconsistent"));
                };
                polygon.extend_from_slice(&oriented[..oriented.len() - 1]);

                let label = topology_label(step.boundary);
                if let Some((_, regions)) = builder
                    .boundary_regions
                    .iter_mut()
                    .find(|(candidate, _)| *candidate == label)
                {
                    regions.insert(domain.region);
                } else {
                    builder
                        .boundary_regions
                        .push((label, BTreeSet::from([domain.region])));
                }
                for (pair, parameter) in oriented.windows(2).zip(parameters.windows(2)) {
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
            if polygon.is_empty() {
                continue;
            }
            if polygon.len() < 3 {
                return Err(MeshError::Topology("topology face cycle is incomplete"));
            }
            cycles.push(polygon);
        }
        let outer = cycles
            .first()
            .cloned()
            .ok_or(MeshError::Topology("topology face has no outer cycle"))?;
        builder.domains.push(TriangulationDomain {
            region: domain.region,
            outer,
            holes: cycles.into_iter().skip(1).collect(),
        });
    }

    triangulate_topology_domains(&mut builder)?;
    for ((region, _), steps) in free_slits {
        for run in split_slit_runs(&steps)? {
            cut_free_slit(&mut builder, region, &run, &trace_vertices)?;
        }
    }
    while !builder.dirty_edges.is_empty() {
        builder.legalize_one()?;
    }
    verify_topology_mesh(&builder)?;
    let quality = builder.quality();
    Ok(TriMesh {
        geometry_revision: plan.geometry_revision,
        mesh_revision,
        vertices: builder.vertices,
        triangles: builder.triangles,
        boundary_edges: builder.boundary_edges,
        quality,
    })
}

fn split_slit_runs(steps: &[PlannedFaceStep]) -> Result<Vec<Vec<PlannedFaceStep>>, MeshError> {
    let mut left = steps
        .iter()
        .filter(|step| {
            matches!(
                step.boundary.source,
                PlannedBoundarySource::Curve {
                    side: CurveTraceSide::Left,
                    ..
                }
            )
        })
        .copied()
        .collect::<Vec<_>>();
    left.sort_by(|a, b| a.boundary.parameter[0].total_cmp(&b.boundary.parameter[0]));
    if left.is_empty() {
        return Err(MeshError::Topology("free slit has no left trace"));
    }
    let mut ranges = vec![];
    let mut start = left[0].boundary.parameter[0];
    let mut end = left[0].boundary.parameter[1];
    for step in &left[1..] {
        if step.boundary.parameter[0] != end {
            ranges.push([start, end]);
            start = step.boundary.parameter[0];
        }
        end = step.boundary.parameter[1];
    }
    ranges.push([start, end]);
    Ok(ranges
        .into_iter()
        .map(|[start, end]| {
            steps
                .iter()
                .filter(|step| {
                    let midpoint = 0.5 * (step.boundary.parameter[0] + step.boundary.parameter[1]);
                    midpoint >= start && midpoint <= end
                })
                .copied()
                .collect()
        })
        .collect())
}

fn cut_free_slit(
    builder: &mut MeshBuilder,
    region: RegionId,
    steps: &[PlannedFaceStep],
    trace_vertices: &BTreeMap<TraceVertexId, usize>,
) -> Result<(), MeshError> {
    let mut left = steps
        .iter()
        .filter(|step| {
            matches!(
                step.boundary.source,
                PlannedBoundarySource::Curve {
                    side: CurveTraceSide::Left,
                    ..
                }
            )
        })
        .copied()
        .collect::<Vec<_>>();
    left.sort_by(|a, b| a.boundary.parameter[0].total_cmp(&b.boundary.parameter[0]));
    let first = left
        .first()
        .ok_or(MeshError::Topology("free slit has no left trace"))?;
    let curve = match first.boundary.source {
        PlannedBoundarySource::Curve { curve, .. } => curve,
        PlannedBoundarySource::Outer(_) => unreachable!(),
    };
    if left.iter().any(|step| {
        !matches!(step.boundary.source, PlannedBoundarySource::Curve { curve: candidate, .. } if candidate == curve)
    }) {
        return Err(MeshError::Topology("a free slit cycle contains multiple curves"));
    }
    let mut samples = vec![];
    for step in &left {
        let edge = step.boundary;
        let length = (edge.points[1] - edge.points[0]).norm();
        let pieces = (length / builder.options.target_edge_length)
            .ceil()
            .max(1.0) as usize;
        for piece in 0..pieces {
            let fraction = piece as f64 / pieces as f64;
            samples.push((
                crate::Sample {
                    t: edge.parameter[0] + (edge.parameter[1] - edge.parameter[0]) * fraction,
                    point: edge.points[0].lerp(edge.points[1], fraction),
                },
                (piece == 0).then_some(edge.traces[0]),
            ));
        }
    }
    let last = left.last().unwrap().boundary;
    samples.push((
        crate::Sample {
            t: last.parameter[1],
            point: last.points[1],
        },
        Some(last.traces[1]),
    ));
    if samples.len() == 2 {
        samples.insert(
            1,
            (
                crate::Sample {
                    t: (samples[0].0.t + samples[1].0.t) * 0.5,
                    point: samples[0].0.point.lerp(samples[1].0.point, 0.5),
                },
                None,
            ),
        );
    }

    let constraint = builder.internal_samples.len();
    builder
        .internal_samples
        .push(samples.iter().map(|(sample, _)| *sample).collect());
    builder.constraint_target_regions.push(region);
    builder.constraint_kinds.push(OpenConstraintKind::Baffle {
        id: crate::InternalBoundaryId(curve.0),
        region,
    });
    builder.internal_chains.push(vec![]);
    for (sample, trace) in samples {
        let vertex = trace
            .and_then(|trace| trace_vertices.get(&trace).copied())
            .map_or_else(
                || builder.insert_constraint_point(constraint, sample.point),
                Ok,
            )?;
        builder.internal_chains[constraint].push((vertex, sample.t));
    }
    let chain = builder.internal_chains[constraint].clone();
    for pair in chain.windows(2) {
        let requested = [pair[0].0, pair[1].0];
        let mut attempts = 0usize;
        while !builder.recover_constraint_edge(requested)? {
            attempts += 1;
            if attempts > builder.options.max_triangles.saturating_mul(8) {
                return Err(MeshError::Topology("free-slit recovery work limit reached"));
            }
        }
    }
    builder.cut_internal_boundary(constraint)?;

    let legacy_id = crate::InternalBoundaryId(curve.0);
    for endpoint in [0, chain.len() - 1] {
        let left_vertex = chain[endpoint].0;
        let point = builder.point(left_vertex);
        let right_trace = steps.iter().find_map(|step| {
            let PlannedBoundarySource::Curve {
                side: CurveTraceSide::Right,
                ..
            } = step.boundary.source
            else {
                return None;
            };
            step.boundary
                .points
                .iter()
                .zip(step.boundary.traces)
                .find_map(|(candidate, trace)| (*candidate == point).then_some(trace))
        });
        if let Some(replacement) = right_trace.and_then(|trace| trace_vertices.get(&trace).copied())
            && replacement != left_vertex
        {
            rewire_attached_slit_endpoint(builder, legacy_id, left_vertex, replacement)?;
        }
    }
    for edge in &mut builder.boundary_edges {
        let BoundaryLabel::InternalBoundary { id, side } = edge.label else {
            continue;
        };
        if id != legacy_id {
            continue;
        }
        let parameter = 0.5 * (edge.parameters[0] + edge.parameters[1]);
        let planned = left
            .iter()
            .find(|step| {
                let [a, b] = step.boundary.parameter;
                parameter >= a.min(b) && parameter <= a.max(b)
            })
            .ok_or(MeshError::Topology("free-slit span lineage is missing"))?;
        let span = match planned.boundary.source {
            PlannedBoundarySource::Curve { span, .. } => span,
            PlannedBoundarySource::Outer(_) => unreachable!(),
        };
        edge.label = BoundaryLabel::Curve {
            curve,
            span,
            side: match side {
                super::InternalBoundarySide::Left => CurveTraceSide::Left,
                super::InternalBoundarySide::Right => CurveTraceSide::Right,
            },
            separated: true,
        };
    }
    apply_slit_trace_lineage(builder, steps)?;
    builder.boundary_regions.retain(|(label, _)| {
        !matches!(label, BoundaryLabel::InternalBoundary { id, .. } if *id == legacy_id)
    });
    for side in [CurveTraceSide::Left, CurveTraceSide::Right] {
        for step in &left {
            let PlannedBoundarySource::Curve { span, .. } = step.boundary.source else {
                unreachable!()
            };
            builder.boundary_regions.push((
                BoundaryLabel::Curve {
                    curve,
                    span,
                    side,
                    separated: true,
                },
                BTreeSet::from([region]),
            ));
        }
    }
    Ok(())
}

fn apply_slit_trace_lineage(
    builder: &mut MeshBuilder,
    steps: &[PlannedFaceStep],
) -> Result<(), MeshError> {
    for step in steps {
        let label = topology_label(step.boundary);
        for (point, trace) in step.boundary.points.into_iter().zip(step.boundary.traces) {
            let vertices = builder
                .boundary_edges
                .iter()
                .filter(|edge| edge.label == label)
                .flat_map(|edge| edge.vertices)
                .filter(|vertex| builder.point(*vertex) == point)
                .collect::<BTreeSet<_>>();
            let vertex = vertices.into_iter().next().ok_or(MeshError::Topology(
                "slit trace lineage endpoint is missing",
            ))?;
            if builder.vertices[vertex]
                .trace
                .is_some_and(|existing| existing != trace)
            {
                return Err(MeshError::Topology("slit trace lineage is inconsistent"));
            }
            builder.vertices[vertex].trace = Some(trace);
        }
    }
    Ok(())
}

fn rewire_attached_slit_endpoint(
    builder: &mut MeshBuilder,
    id: crate::InternalBoundaryId,
    endpoint: usize,
    replacement: usize,
) -> Result<(), MeshError> {
    let boundary_index = builder
        .boundary_edges
        .iter()
        .position(|edge| {
            edge.vertices.contains(&endpoint)
                && edge.label
                    == BoundaryLabel::InternalBoundary {
                        id,
                        side: super::InternalBoundarySide::Right,
                    }
        })
        .ok_or(MeshError::Topology(
            "attached slit has no right endpoint edge",
        ))?;
    let boundary = builder.boundary_edges[boundary_index];
    let seed = *builder
        .adjacency
        .get(&edge_key(boundary.vertices[0], boundary.vertices[1]))
        .and_then(|sides| sides.first())
        .map(|(triangle, _)| triangle)
        .ok_or(MeshError::Topology("attached slit endpoint has no element"))?;
    let mut sector = BTreeSet::from([seed]);
    let mut pending = vec![seed];
    while let Some(triangle_index) = pending.pop() {
        let triangle = builder.triangles[triangle_index];
        for other in triangle
            .vertices
            .iter()
            .copied()
            .filter(|candidate| *candidate != endpoint)
        {
            let edge = edge_key(endpoint, other);
            if builder.boundary_keys.contains(&edge) {
                continue;
            }
            if let Some(sides) = builder.adjacency.get(&edge) {
                for (neighbor, _) in sides {
                    if *neighbor != triangle_index
                        && builder.triangles[*neighbor].vertices.contains(&endpoint)
                        && sector.insert(*neighbor)
                    {
                        pending.push(*neighbor);
                    }
                }
            }
        }
    }
    for triangle_index in sector {
        let mut triangle = builder.triangles[triangle_index];
        if triangle.vertices.contains(&replacement) {
            return Err(MeshError::Topology(
                "attached slit endpoint would create a degenerate element",
            ));
        }
        for vertex in &mut triangle.vertices {
            if *vertex == endpoint {
                *vertex = replacement;
            }
        }
        builder.replace_triangle(triangle_index, triangle);
    }
    builder
        .boundary_keys
        .remove(&edge_key(boundary.vertices[0], boundary.vertices[1]));
    for vertex in &mut builder.boundary_edges[boundary_index].vertices {
        if *vertex == endpoint {
            *vertex = replacement;
        }
    }
    let rewired = builder.boundary_edges[boundary_index].vertices;
    builder
        .boundary_keys
        .insert(edge_key(rewired[0], rewired[1]));
    Ok(())
}

fn topology_label(boundary: PlannedBoundaryEdge) -> BoundaryLabel {
    match boundary.source {
        PlannedBoundarySource::Outer(side) => BoundaryLabel::Outer(side),
        PlannedBoundarySource::Curve { curve, span, side } => {
            let separated = matches!(boundary.behavior, Some(SpanBehavior::Separated { .. }));
            BoundaryLabel::Curve {
                curve,
                span,
                side: if separated {
                    side
                } else {
                    CurveTraceSide::Left
                },
                separated,
            }
        }
    }
}

fn trace_mesh_vertex(
    builder: &mut MeshBuilder,
    points: &BTreeMap<TraceVertexId, Point2>,
    vertices: &mut BTreeMap<TraceVertexId, usize>,
    trace: TraceVertexId,
) -> Result<usize, MeshError> {
    if let Some(vertex) = vertices.get(&trace) {
        return Ok(*vertex);
    }
    let point = *points
        .get(&trace)
        .ok_or(MeshError::Topology("topology trace vertex is missing"))?;
    let vertex = builder.add_vertex(point, None)?;
    builder.vertices[vertex].trace = Some(trace);
    vertices.insert(trace, vertex);
    Ok(vertex)
}

fn expand_step_chain(
    builder: &mut MeshBuilder,
    points: &BTreeMap<TraceVertexId, Point2>,
    trace_vertices: &mut BTreeMap<TraceVertexId, usize>,
    step: &PlannedFaceStep,
) -> Result<ExpandedChain, MeshError> {
    let boundary = step.boundary;
    let label = topology_label(boundary);
    let length = (boundary.points[1] - boundary.points[0]).norm();
    let pieces = (length / builder.options.target_edge_length)
        .ceil()
        .max(1.0) as usize;
    let mut vertices = Vec::with_capacity(pieces + 1);
    let mut parameters = Vec::with_capacity(pieces + 1);
    vertices.push(trace_mesh_vertex(
        builder,
        points,
        trace_vertices,
        boundary.traces[0],
    )?);
    parameters.push(boundary.parameter[0]);
    for piece in 1..pieces {
        let fraction = piece as f64 / pieces as f64;
        let parameter =
            boundary.parameter[0] + (boundary.parameter[1] - boundary.parameter[0]) * fraction;
        vertices.push(builder.add_vertex(
            boundary.points[0].lerp(boundary.points[1], fraction),
            Some(BoundaryPoint { label, parameter }),
        )?);
        parameters.push(parameter);
    }
    vertices.push(trace_mesh_vertex(
        builder,
        points,
        trace_vertices,
        boundary.traces[1],
    )?);
    parameters.push(boundary.parameter[1]);
    for (index, vertex) in vertices.iter().copied().enumerate() {
        if builder.vertices[vertex].boundary.is_none() {
            let fraction = index as f64 / pieces as f64;
            builder.vertices[vertex].boundary = Some(BoundaryPoint {
                label,
                parameter: boundary.parameter[0]
                    + (boundary.parameter[1] - boundary.parameter[0]) * fraction,
            });
        }
    }
    Ok(ExpandedChain {
        traces: boundary.traces,
        vertices,
        parameters,
    })
}

fn triangulate_topology_domains(builder: &mut MeshBuilder) -> Result<(), MeshError> {
    for domain in builder.domains.clone() {
        let mut polygon = domain.outer;
        for hole in domain.holes {
            bridge_topology_hole(builder, &mut polygon, &hole, domain.region)?;
        }
        clip_topology_polygon(builder, polygon, domain.region)?;
    }

    let mut legalization_work = 0usize;
    loop {
        while !builder.dirty_edges.is_empty() {
            legalization_work += 1;
            if legalization_work > builder.options.max_triangles.saturating_mul(128) {
                return Err(MeshError::Topology("edge legalization work limit reached"));
            }
            builder.legalize_one()?;
        }
        if !builder.bad_triangles.is_empty()
            && builder.stats.refinement_insertions >= builder.options.max_refinement_steps
        {
            return Err(MeshError::RefinementLimit(builder.quality()));
        }
        if builder.refine_once()? {
            break;
        }
    }
    Ok(())
}

fn bridge_topology_hole(
    builder: &MeshBuilder,
    polygon: &mut Vec<usize>,
    hole: &[usize],
    region: RegionId,
) -> Result<(), MeshError> {
    let mut best = None::<(f64, usize, usize)>;
    for (outer_index, outer) in polygon.iter().copied().enumerate() {
        for (hole_index, inner) in hole.iter().copied().enumerate() {
            let a = builder.point(outer);
            let b = builder.point(inner);
            if a == b {
                continue;
            }
            let crosses_boundary = builder.boundary_edges.iter().any(|edge| {
                builder.boundary_relevant_to_region(edge.label, region)
                    && !edge.vertices.contains(&outer)
                    && !edge.vertices.contains(&inner)
                    && segment_relation(
                        a,
                        b,
                        builder.point(edge.vertices[0]),
                        builder.point(edge.vertices[1]),
                    ) != SegmentRelation::Disjoint
            });
            if crosses_boundary {
                continue;
            }
            let crosses_polygon = polygon
                .iter()
                .copied()
                .zip(polygon.iter().copied().cycle().skip(1))
                .take(polygon.len())
                .any(|(x, y)| {
                    ![x, y].contains(&outer)
                        && ![x, y].contains(&inner)
                        && segment_relation(a, b, builder.point(x), builder.point(y))
                            == SegmentRelation::ProperIntersection
                });
            if crosses_polygon || builder.region_at(a.lerp(b, 0.5)) != Some(region) {
                continue;
            }
            let length = (a - b).norm();
            if best.is_none_or(|candidate| length < candidate.0) {
                best = Some((length, outer_index, hole_index));
            }
        }
    }
    let (_, outer_index, hole_index) =
        best.ok_or(MeshError::Topology("no visible bridge to topology cycle"))?;
    let mut splice = Vec::with_capacity(hole.len() + 2);
    for offset in 0..hole.len() {
        splice.push(hole[(hole_index + offset) % hole.len()]);
    }
    splice.push(hole[hole_index]);
    splice.push(polygon[outer_index]);
    polygon.splice(outer_index + 1..outer_index + 1, splice);
    Ok(())
}

fn clip_topology_polygon(
    builder: &mut MeshBuilder,
    mut polygon: Vec<usize>,
    region: RegionId,
) -> Result<(), MeshError> {
    let mut degenerate_pass = false;
    while polygon.len() > 3 {
        let mut removed = false;
        for index in 0..polygon.len() {
            let previous = (index + polygon.len() - 1) % polygon.len();
            let next = (index + 1) % polygon.len();
            let [a, v, c] = [polygon[previous], polygon[index], polygon[next]];
            let sign = orient2d(builder.point(a), builder.point(v), builder.point(c));
            if degenerate_pass {
                if a == v || v == c || a == c || sign == PredicateSign::Zero {
                    polygon.remove(index);
                    removed = true;
                    break;
                }
                continue;
            }
            if a == v || v == c || a == c || sign != PredicateSign::Positive {
                continue;
            }
            let diagonal_blocked = polygon
                .iter()
                .copied()
                .zip(polygon.iter().copied().cycle().skip(1))
                .take(polygon.len())
                .any(|(x, y)| {
                    x != a
                        && x != c
                        && y != a
                        && y != c
                        && segment_relation(
                            builder.point(a),
                            builder.point(c),
                            builder.point(x),
                            builder.point(y),
                        ) != SegmentRelation::Disjoint
                });
            if diagonal_blocked {
                continue;
            }
            let contains_vertex =
                polygon
                    .iter()
                    .copied()
                    .enumerate()
                    .any(|(other_index, other)| {
                        ![previous, index, next].contains(&other_index)
                            && ![a, v, c].contains(&other)
                            && point_in_triangle(
                                builder.point(other),
                                [builder.point(a), builder.point(v), builder.point(c)],
                            ) != PolygonLocation::Outside
                    });
            if contains_vertex {
                continue;
            }
            builder.push_triangle(MeshTriangle {
                vertices: [a, v, c],
                region,
            })?;
            polygon.remove(index);
            removed = true;
            degenerate_pass = false;
            break;
        }
        if !removed {
            if degenerate_pass {
                return Err(MeshError::Topology("topology ear clipping stalled"));
            }
            degenerate_pass = true;
        }
    }
    if polygon.len() == 3 {
        builder
            .push_triangle(builder.ccw_triangle([polygon[0], polygon[1], polygon[2]], region)?)?;
    }
    Ok(())
}

fn verify_topology_mesh(builder: &MeshBuilder) -> Result<(), MeshError> {
    for (index, triangle) in builder.triangles.iter().copied().enumerate() {
        let [a, b, c] = builder.triangle_points(triangle);
        if orient2d(a, b, c) != PredicateSign::Positive {
            return Err(MeshError::Topology("mesh contains an inverted triangle"));
        }
        if builder.region_at((a + b + c) / 3.0) != Some(triangle.region) {
            return Err(MeshError::Topology(
                "triangle has the wrong topology face region",
            ));
        }
        for opposite in 0..3 {
            let edge = edge_key(
                triangle.vertices[(opposite + 1) % 3],
                triangle.vertices[(opposite + 2) % 3],
            );
            let sides = builder
                .adjacency
                .get(&edge)
                .ok_or(MeshError::Topology("missing adjacency"))?;
            let expected = builder
                .boundary_edges
                .iter()
                .find(|boundary| edge_key(boundary.vertices[0], boundary.vertices[1]) == edge)
                .map_or(2, |boundary| boundary_adjacency(boundary.label));
            if sides.len() != expected || !sides.contains(&(index, triangle.vertices[opposite])) {
                return Err(MeshError::Topology("mesh has a crack or non-manifold edge"));
            }
        }
    }
    for boundary in &builder.boundary_edges {
        let expected = boundary_adjacency(boundary.label);
        if builder
            .adjacency
            .get(&edge_key(boundary.vertices[0], boundary.vertices[1]))
            .is_none_or(|sides| sides.len() != expected)
        {
            return Err(MeshError::Topology(
                "constrained edge has incorrect adjacency",
            ));
        }
    }
    Ok(())
}

fn boundary_adjacency(label: BoundaryLabel) -> usize {
    match label {
        BoundaryLabel::Curve {
            separated: false, ..
        } => 2,
        BoundaryLabel::Curve {
            separated: true, ..
        }
        | BoundaryLabel::Outer(_)
        | BoundaryLabel::Obstacle(_)
        | BoundaryLabel::Wall { .. }
        | BoundaryLabel::InternalBoundary { .. } => 1,
        BoundaryLabel::MaterialInterface(_) | BoundaryLabel::OpenMaterialInterface(_) => 2,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PlannedBoundarySource {
    Outer(crate::OuterSide),
    Curve {
        curve: CurveId,
        span: CurveSpanId,
        side: CurveTraceSide,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlannedBoundaryEdge {
    pub source: PlannedBoundarySource,
    pub behavior: Option<SpanBehavior>,
    pub face: FaceId,
    pub region: RegionId,
    pub traces: [TraceVertexId; 2],
    pub points: [Point2; 2],
    pub parameter: [f64; 2],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlannedFaceStep {
    /// Snapshot-local compiled edge index. Both active sides of a transmitting
    /// edge use the same index, allowing the mesher to share its entire chain.
    pub edge: usize,
    pub boundary: PlannedBoundaryEdge,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlannedFaceDomain {
    pub face: FaceId,
    pub region: RegionId,
    /// Directed trace-vertex cycles with the active face on the left. The first
    /// is the outer cycle; later cycles are holes or slit components.
    pub cycles: Vec<Vec<TraceVertexId>>,
    /// Directed edge steps matching `cycles`, with this face on the left.
    pub steps: Vec<Vec<PlannedFaceStep>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlannedTraceVertex {
    pub id: TraceVertexId,
    pub point: Point2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedJunctionTraces {
    pub vertex: crate::TopologyVertexId,
    /// One entry per sector trace. Repeated face IDs are meaningful for a
    /// separated same-face junction.
    pub faces: Vec<FaceId>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopologyMeshPlan {
    pub geometry_revision: u64,
    pub domain: crate::DomainRect,
    pub vertices: Vec<PlannedTraceVertex>,
    pub junctions: Vec<PlannedJunctionTraces>,
    pub domains: Vec<PlannedFaceDomain>,
    pub boundaries: Vec<PlannedBoundaryEdge>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopologyMeshPlanError {
    DuplicateFace(FaceId),
    MissingFace(FaceId),
    UnknownFace(FaceId),
    InvalidRegion(RegionId),
    DuplicateRegion(RegionId),
    NoActiveFaces,
    MissingTrace(TraceVertexId),
    BrokenCycle(FaceId),
    MissingCurve(CurveSpanId),
    TransmittingExcludedFace(CurveSpanId),
    CoupledExcludedFace(CurveSpanId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopologyMeshUpdateAction {
    /// Geometry and topology are identical; the current discretization remains valid.
    Reuse,
    FullRebuild(TopologyFullRebuildReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopologyFullRebuildReason {
    DomainChanged,
    FaceAssignmentsChanged,
    CurveOrSpanTopologyChanged,
    SpanBehaviorChanged,
    TraceEquivalenceChanged,
    /// The existing patch repair needs spline evaluation and endpoint-sector
    /// rewiring from the new topology model. That migration is deliberately
    /// deferred until all numerical consumers use `TopologyMeshPlan`.
    CoordinateRepairDeferred,
}

/// Classifies reuse before starting mesh work. It intentionally admits only
/// exact reuse today. Every changed plan receives a stable, inspectable full
/// rebuild reason rather than entering the legacy object-specific repair path.
pub fn topology_mesh_update_action(
    previous: &TopologyMeshPlan,
    next: &TopologyMeshPlan,
) -> TopologyMeshUpdateAction {
    use TopologyFullRebuildReason as Reason;
    if previous.domain != next.domain {
        return TopologyMeshUpdateAction::FullRebuild(Reason::DomainChanged);
    }
    let face_signature = |plan: &TopologyMeshPlan| {
        plan.domains
            .iter()
            .map(|domain| (domain.face, domain.region))
            .collect::<Vec<_>>()
    };
    if face_signature(previous) != face_signature(next) {
        return TopologyMeshUpdateAction::FullRebuild(Reason::FaceAssignmentsChanged);
    }
    let source_signature = |plan: &TopologyMeshPlan| {
        plan.boundaries
            .iter()
            .map(|boundary| (boundary.source, boundary.face, boundary.region))
            .collect::<BTreeSet<_>>()
    };
    if source_signature(previous) != source_signature(next) {
        return TopologyMeshUpdateAction::FullRebuild(Reason::CurveOrSpanTopologyChanged);
    }
    let behavior_signature = |plan: &TopologyMeshPlan| {
        plan.boundaries
            .iter()
            .map(|boundary| {
                (
                    boundary.source,
                    matches!(boundary.behavior, Some(SpanBehavior::Separated { .. })),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    if behavior_signature(previous) != behavior_signature(next) {
        return TopologyMeshUpdateAction::FullRebuild(Reason::SpanBehaviorChanged);
    }
    if previous.junctions != next.junctions {
        return TopologyMeshUpdateAction::FullRebuild(Reason::TraceEquivalenceChanged);
    }
    let same_geometry = previous
        .boundaries
        .iter()
        .zip(&next.boundaries)
        .all(|(a, b)| a.points == b.points && a.parameter == b.parameter)
        && previous
            .vertices
            .iter()
            .map(|vertex| vertex.point)
            .eq(next.vertices.iter().map(|vertex| vertex.point));
    if same_geometry {
        TopologyMeshUpdateAction::Reuse
    } else {
        TopologyMeshUpdateAction::FullRebuild(Reason::CoordinateRepairDeferred)
    }
}

impl std::fmt::Display for TopologyMeshPlanError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for TopologyMeshPlanError {}

impl TopologyMeshPlan {
    pub fn new(
        topology: &TopologySnapshot,
        assignments: &[FaceRegionAssignment],
    ) -> Result<Self, TopologyMeshPlanError> {
        let topology_faces = topology
            .faces
            .iter()
            .map(|face| face.id)
            .collect::<BTreeSet<_>>();
        let mut assigned = BTreeMap::new();
        let mut regions = BTreeSet::new();
        for assignment in assignments {
            if !topology_faces.contains(&assignment.face) {
                return Err(TopologyMeshPlanError::UnknownFace(assignment.face));
            }
            if assigned
                .insert(assignment.face, assignment.region)
                .is_some()
            {
                return Err(TopologyMeshPlanError::DuplicateFace(assignment.face));
            }
            if let Some(region) = assignment.region {
                if region.0 == 0 {
                    return Err(TopologyMeshPlanError::InvalidRegion(region));
                }
                if !regions.insert(region) {
                    return Err(TopologyMeshPlanError::DuplicateRegion(region));
                }
            }
        }
        for face in &topology.faces {
            if !assigned.contains_key(&face.id) {
                return Err(TopologyMeshPlanError::MissingFace(face.id));
            }
        }
        if regions.is_empty() {
            return Err(TopologyMeshPlanError::NoActiveFaces);
        }

        validate_span_sides(topology, &assigned)?;

        let mut trace_points = BTreeMap::<TraceVertexId, Point2>::new();
        for vertex in &topology.vertices {
            for trace in &vertex.traces {
                if let Some(existing) = trace_points.insert(trace.id, vertex.point)
                    && existing != vertex.point
                {
                    return Err(TopologyMeshPlanError::MissingTrace(trace.id));
                }
            }
        }

        let mut domains = vec![];
        for face in &topology.faces {
            let Some(region) = assigned[&face.id] else {
                continue;
            };
            let compiled = face
                .boundaries
                .iter()
                .map(|cycle| compile_cycle(topology, face.id, region, cycle))
                .collect::<Result<Vec<_>, _>>()?;
            let cycles = compiled
                .iter()
                .map(|(cycle, _)| cycle.clone())
                .collect::<Vec<_>>();
            let steps = compiled
                .into_iter()
                .map(|(_, steps)| steps)
                .collect::<Vec<_>>();
            if cycles.first().is_none_or(|cycle| cycle.len() < 3) {
                return Err(TopologyMeshPlanError::BrokenCycle(face.id));
            }
            domains.push(PlannedFaceDomain {
                face: face.id,
                region,
                cycles,
                steps,
            });
        }

        let mut boundaries = vec![];
        for edge in &topology.edges {
            append_boundary_sides(edge, &assigned, &mut boundaries)?;
        }
        let used_traces = domains
            .iter()
            .flat_map(|domain| domain.cycles.iter().flatten().copied())
            .chain(boundaries.iter().flat_map(|boundary| boundary.traces))
            .collect::<BTreeSet<_>>();
        let vertices = trace_points
            .into_iter()
            .filter(|(id, _)| used_traces.contains(id))
            .map(|(id, point)| PlannedTraceVertex { id, point })
            .collect();
        let junctions = topology
            .vertices
            .iter()
            .filter_map(|vertex| {
                vertex.authored.map(|authored| {
                    let mut faces = vertex
                        .traces
                        .iter()
                        .filter(|trace| assigned.get(&trace.face).copied().flatten().is_some())
                        .map(|trace| trace.face)
                        .collect::<Vec<_>>();
                    faces.sort();
                    PlannedJunctionTraces {
                        vertex: authored,
                        faces,
                    }
                })
            })
            .collect();
        Ok(Self {
            geometry_revision: topology.revision,
            domain: topology.domain,
            vertices,
            junctions,
            domains,
            boundaries,
        })
    }

    pub fn region_for_face(&self, face: FaceId) -> Option<RegionId> {
        self.domains
            .iter()
            .find(|domain| domain.face == face)
            .map(|domain| domain.region)
    }

    pub fn trace_point(&self, trace: TraceVertexId) -> Option<Point2> {
        self.vertices
            .binary_search_by_key(&trace, |vertex| vertex.id)
            .ok()
            .map(|index| self.vertices[index].point)
    }
}

fn validate_span_sides(
    topology: &TopologySnapshot,
    assigned: &BTreeMap<FaceId, Option<RegionId>>,
) -> Result<(), TopologyMeshPlanError> {
    for edge in &topology.edges {
        let CompiledEdgeSource::Curve(span) = edge.source else {
            continue;
        };
        let left = assigned.get(&edge.left).copied().flatten();
        let right = assigned.get(&edge.right).copied().flatten();
        match edge.behavior {
            Some(SpanBehavior::Transmitting) if left.is_none() || right.is_none() => {
                return Err(TopologyMeshPlanError::TransmittingExcludedFace(span));
            }
            Some(SpanBehavior::Separated { coupling, .. })
                if !matches!(coupling, crate::InternalBoundaryCoupling::Independent)
                    && (left.is_none() || right.is_none()) =>
            {
                return Err(TopologyMeshPlanError::CoupledExcludedFace(span));
            }
            Some(_) => {}
            None => return Err(TopologyMeshPlanError::MissingCurve(span)),
        }
    }
    Ok(())
}

fn compile_cycle(
    topology: &TopologySnapshot,
    face: FaceId,
    region: RegionId,
    steps: &[CompiledBoundaryStep],
) -> Result<(Vec<TraceVertexId>, Vec<PlannedFaceStep>), TopologyMeshPlanError> {
    let mut cycle = vec![];
    let mut planned = vec![];
    let mut expected = None;
    for step in steps {
        let edge = topology
            .edges
            .get(step.edge)
            .ok_or(TopologyMeshPlanError::BrokenCycle(face))?;
        let [start, end] = oriented_face_traces(edge, face, step.reversed)
            .ok_or(TopologyMeshPlanError::BrokenCycle(face))?;
        if expected.is_some_and(|expected| expected != start) {
            return Err(TopologyMeshPlanError::BrokenCycle(face));
        }
        cycle.push(start);
        planned.push(PlannedFaceStep {
            edge: step.edge,
            boundary: planned_face_edge(edge, face, region, step.reversed)?,
        });
        expected = Some(end);
    }
    if cycle.first().copied() != expected {
        return Err(TopologyMeshPlanError::BrokenCycle(face));
    }
    Ok((cycle, planned))
}

fn planned_face_edge(
    edge: &CompiledEdge,
    face: FaceId,
    region: RegionId,
    reversed: bool,
) -> Result<PlannedBoundaryEdge, TopologyMeshPlanError> {
    let [start, end] = oriented_face_traces(edge, face, reversed)
        .ok_or(TopologyMeshPlanError::BrokenCycle(face))?;
    let (source, points, parameter) = match (edge.source, reversed) {
        (CompiledEdgeSource::Outer(side), false) => (
            PlannedBoundarySource::Outer(side),
            edge.points,
            edge.parameter,
        ),
        (CompiledEdgeSource::Curve(span), false) => (
            PlannedBoundarySource::Curve {
                curve: edge
                    .curve
                    .ok_or(TopologyMeshPlanError::MissingCurve(span))?,
                span,
                side: CurveTraceSide::Left,
            },
            edge.points,
            edge.parameter,
        ),
        (CompiledEdgeSource::Curve(span), true) => (
            PlannedBoundarySource::Curve {
                curve: edge
                    .curve
                    .ok_or(TopologyMeshPlanError::MissingCurve(span))?,
                span,
                side: CurveTraceSide::Right,
            },
            [edge.points[1], edge.points[0]],
            [edge.parameter[1], edge.parameter[0]],
        ),
        (CompiledEdgeSource::Outer(_), true) => {
            return Err(TopologyMeshPlanError::BrokenCycle(face));
        }
    };
    Ok(PlannedBoundaryEdge {
        source,
        behavior: edge.behavior,
        face,
        region,
        traces: [start, end],
        points,
        parameter,
    })
}

fn oriented_face_traces(
    edge: &CompiledEdge,
    face: FaceId,
    reversed: bool,
) -> Option<[TraceVertexId; 2]> {
    match (reversed, edge.left == face, edge.right == face) {
        (false, true, _) => Some([edge.traces[0].left, edge.traces[1].left]),
        (true, _, true) => Some([edge.traces[1].right, edge.traces[0].right]),
        _ => None,
    }
}

fn append_boundary_sides(
    edge: &CompiledEdge,
    assigned: &BTreeMap<FaceId, Option<RegionId>>,
    output: &mut Vec<PlannedBoundaryEdge>,
) -> Result<(), TopologyMeshPlanError> {
    let side = |face: FaceId| assigned.get(&face).copied().flatten();
    match edge.source {
        CompiledEdgeSource::Outer(outer) => {
            if let Some(region) = side(edge.left) {
                output.push(PlannedBoundaryEdge {
                    source: PlannedBoundarySource::Outer(outer),
                    behavior: None,
                    face: edge.left,
                    region,
                    traces: [edge.traces[0].left, edge.traces[1].left],
                    points: edge.points,
                    parameter: edge.parameter,
                });
            }
        }
        CompiledEdgeSource::Curve(span) => {
            let curve = edge
                .curve
                .ok_or(TopologyMeshPlanError::MissingCurve(span))?;
            if let Some(region) = side(edge.left) {
                output.push(PlannedBoundaryEdge {
                    source: PlannedBoundarySource::Curve {
                        curve,
                        span,
                        side: CurveTraceSide::Left,
                    },
                    behavior: edge.behavior,
                    face: edge.left,
                    region,
                    traces: [edge.traces[0].left, edge.traces[1].left],
                    points: edge.points,
                    parameter: edge.parameter,
                });
            }
            if let Some(region) = side(edge.right)
                && (edge.behavior != Some(SpanBehavior::Transmitting) || edge.right != edge.left)
            {
                output.push(PlannedBoundaryEdge {
                    source: PlannedBoundarySource::Curve {
                        curve,
                        span,
                        side: CurveTraceSide::Right,
                    },
                    behavior: edge.behavior,
                    face: edge.right,
                    region,
                    traces: [edge.traces[1].right, edge.traces[0].right],
                    points: [edge.points[1], edge.points[0]],
                    parameter: [edge.parameter[1], edge.parameter[0]],
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CurveNode, CurveSpan, CurveSpline, DomainRect, OpenCubicSpline, PeriodicCubicSpline,
        TopologyCurve, TopologyGeometry, TopologyVertex, TopologyVertexId, TopologyVertexLocation,
        compile_topology,
    };

    fn spans(start: u64, count: usize, behavior: SpanBehavior) -> Vec<CurveSpan> {
        (0..count)
            .map(|index| CurveSpan {
                id: CurveSpanId(start + index as u64),
                behavior,
            })
            .collect()
    }

    fn square(behavior: SpanBehavior) -> TopologyCurve {
        let spline = PeriodicCubicSpline::polygon(vec![
            Point2::new(-0.5, -0.5),
            Point2::new(0.5, -0.5),
            Point2::new(0.5, 0.5),
            Point2::new(-0.5, 0.5),
        ])
        .unwrap();
        TopologyCurve::new(
            CurveId(1),
            CurveSpline::Closed(spline),
            spans(1, 4, behavior),
        )
        .unwrap()
    }

    fn assign_each_face(topology: &TopologySnapshot) -> Vec<FaceRegionAssignment> {
        topology
            .faces
            .iter()
            .enumerate()
            .map(|(index, face)| FaceRegionAssignment {
                face: face.id,
                region: Some(RegionId(index as u64 + 1)),
            })
            .collect()
    }

    fn mesh_options() -> super::super::MeshingOptions {
        super::super::MeshingOptions {
            target_edge_length: 0.35,
            minimum_angle_degrees: 8.0,
            max_vertices: 20_000,
            max_triangles: 40_000,
            max_refinement_steps: 20_000,
            ..super::super::MeshingOptions::default()
        }
    }

    fn mesh_snapshot(topology: &TopologySnapshot, assignments: &[FaceRegionAssignment]) -> TriMesh {
        let plan = TopologyMeshPlan::new(topology, assignments).unwrap();
        mesh_topology_plan(&plan, 77, mesh_options()).unwrap()
    }

    fn mesh_area(mesh: &TriMesh) -> f64 {
        mesh.triangles
            .iter()
            .map(|triangle| {
                let [a, b, c] = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
                (b - a).cross(c - a) * 0.5
            })
            .sum()
    }

    #[test]
    fn topology_mesher_builds_the_empty_domain_with_trace_lineage() {
        let topology = compile_topology(&TopologyGeometry::default(), 14).unwrap();
        let mesh = mesh_snapshot(&topology, &assign_each_face(&topology));
        assert_eq!(mesh.geometry_revision, 14);
        assert_eq!(mesh.mesh_revision, 77);
        assert!((mesh_area(&mesh) - 4.0).abs() < 1.0e-10);
        assert!(
            mesh.vertices
                .iter()
                .filter(|vertex| vertex.trace.is_some())
                .count()
                >= 4
        );
        assert!(
            mesh.boundary_edges
                .iter()
                .all(|edge| matches!(edge.label, BoundaryLabel::Outer(_)))
        );
    }

    #[test]
    fn topology_mesher_shares_a_transmitting_chain_between_regions() {
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![square(SpanBehavior::Transmitting)],
                ..TopologyGeometry::default()
            },
            15,
        )
        .unwrap();
        let mesh = mesh_snapshot(&topology, &assign_each_face(&topology));
        assert!((mesh_area(&mesh) - 4.0).abs() < 1.0e-9);
        assert_eq!(
            mesh.triangles
                .iter()
                .map(|triangle| triangle.region)
                .collect::<BTreeSet<_>>()
                .len(),
            2
        );
        for edge in mesh.boundary_edges.iter().filter(|edge| {
            matches!(
                edge.label,
                BoundaryLabel::Curve {
                    separated: false,
                    ..
                }
            )
        }) {
            assert_eq!(
                mesh.triangles
                    .iter()
                    .filter(|triangle| {
                        triangle.vertices.contains(&edge.vertices[0])
                            && triangle.vertices.contains(&edge.vertices[1])
                    })
                    .count(),
                2
            );
        }
    }

    #[test]
    fn topology_mesher_excludes_a_hole_face() {
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![square(SpanBehavior::REFLECTING)],
                ..TopologyGeometry::default()
            },
            16,
        )
        .unwrap();
        let hole = topology.face_at(Point2::new(0.0, 0.0)).unwrap();
        let assignments = topology
            .faces
            .iter()
            .map(|face| FaceRegionAssignment {
                face: face.id,
                region: (face.id != hole).then_some(RegionId(1)),
            })
            .collect::<Vec<_>>();
        let mesh = mesh_snapshot(&topology, &assignments);
        assert!((mesh_area(&mesh) - 3.0).abs() < 1.0e-9);
        assert!(mesh.boundary_edges.iter().any(|edge| matches!(
            edge.label,
            BoundaryLabel::Curve {
                separated: true,
                ..
            }
        )));
    }

    #[test]
    fn topology_mesher_cuts_a_free_baffle_into_two_traces() {
        let spline = OpenCubicSpline::polyline(vec![
            Point2::new(-0.6, 0.0),
            Point2::new(0.0, 0.0),
            Point2::new(0.6, 0.0),
        ])
        .unwrap();
        let curve = TopologyCurve::new(
            CurveId(3),
            CurveSpline::Open(spline),
            spans(30, 2, SpanBehavior::REFLECTING),
        )
        .unwrap();
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![curve],
                ..TopologyGeometry::default()
            },
            17,
        )
        .unwrap();
        let mesh = mesh_snapshot(&topology, &assign_each_face(&topology));
        assert!((mesh_area(&mesh) - 4.0).abs() < 1.0e-9);
        let sides = mesh
            .boundary_edges
            .iter()
            .filter_map(|edge| match edge.label {
                BoundaryLabel::Curve { side, .. } => Some(side),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            sides,
            BTreeSet::from([CurveTraceSide::Left, CurveTraceSide::Right])
        );
    }

    #[test]
    fn topology_mesher_handles_an_outer_to_outer_divider_and_t_junction() {
        let bottom = TopologyVertexId(10);
        let left = TopologyVertexId(11);
        let right = TopologyVertexId(12);
        let center = TopologyVertexId(13);
        let mut horizontal = TopologyCurve::new(
            CurveId(10),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![
                    Point2::new(-1.0, 0.0),
                    Point2::new(0.0, 0.0),
                    Point2::new(1.0, 0.0),
                ])
                .unwrap(),
            ),
            spans(100, 2, SpanBehavior::Transmitting),
        )
        .unwrap();
        horizontal.nodes[0].vertex = Some(left);
        horizontal.nodes[1].vertex = Some(center);
        horizontal.nodes[2].vertex = Some(right);
        let mut branch = TopologyCurve::new(
            CurveId(11),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![Point2::new(0.0, -1.0), Point2::new(0.0, 0.0)])
                    .unwrap(),
            ),
            spans(102, 1, SpanBehavior::Transmitting),
        )
        .unwrap();
        branch.nodes[0].vertex = Some(bottom);
        branch.nodes[1].vertex = Some(center);
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![horizontal, branch],
                vertices: vec![
                    TopologyVertex {
                        id: bottom,
                        location: TopologyVertexLocation::Outer {
                            side: crate::OuterSide::Bottom,
                            fraction: 0.5,
                        },
                    },
                    TopologyVertex {
                        id: left,
                        location: TopologyVertexLocation::Outer {
                            side: crate::OuterSide::Left,
                            fraction: 0.5,
                        },
                    },
                    TopologyVertex {
                        id: right,
                        location: TopologyVertexLocation::Outer {
                            side: crate::OuterSide::Right,
                            fraction: 0.5,
                        },
                    },
                    TopologyVertex {
                        id: center,
                        location: TopologyVertexLocation::Interior(Point2::new(0.0, 0.0)),
                    },
                ],
                ..TopologyGeometry::default()
            },
            18,
        )
        .unwrap();
        assert_eq!(topology.faces.len(), 3);
        let mesh = mesh_snapshot(&topology, &assign_each_face(&topology));
        assert!((mesh_area(&mesh) - 4.0).abs() < 1.0e-9);
        assert_eq!(
            mesh.triangles
                .iter()
                .map(|triangle| triangle.region)
                .collect::<BTreeSet<_>>()
                .len(),
            3
        );
    }

    #[test]
    fn topology_mesher_handles_a_one_ended_baffle() {
        let outer = TopologyVertexId(20);
        let mut curve = TopologyCurve::new(
            CurveId(20),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![
                    Point2::new(0.0, 0.0),
                    Point2::new(0.5, 0.0),
                    Point2::new(1.0, 0.0),
                ])
                .unwrap(),
            ),
            spans(200, 2, SpanBehavior::REFLECTING),
        )
        .unwrap();
        curve.nodes[2].vertex = Some(outer);
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![curve],
                vertices: vec![TopologyVertex {
                    id: outer,
                    location: TopologyVertexLocation::Outer {
                        side: crate::OuterSide::Right,
                        fraction: 0.5,
                    },
                }],
                ..TopologyGeometry::default()
            },
            19,
        )
        .unwrap();
        let plan = TopologyMeshPlan::new(&topology, &assign_each_face(&topology)).unwrap();
        let expected_traces = plan
            .vertices
            .iter()
            .map(|vertex| vertex.id)
            .collect::<BTreeSet<_>>();
        let mesh = mesh_topology_plan(&plan, 77, mesh_options()).unwrap();
        assert!((mesh_area(&mesh) - 4.0).abs() < 1.0e-9);
        let actual_traces = mesh
            .vertices
            .iter()
            .filter_map(|vertex| vertex.trace)
            .collect::<BTreeSet<_>>();
        assert_eq!(actual_traces, expected_traces);
    }

    #[test]
    fn topology_update_policy_reuses_exact_geometry_and_types_rebuilds() {
        let geometry = TopologyGeometry {
            curves: vec![square(SpanBehavior::Transmitting)],
            ..TopologyGeometry::default()
        };
        let first_snapshot = compile_topology(&geometry, 1).unwrap();
        let first =
            TopologyMeshPlan::new(&first_snapshot, &assign_each_face(&first_snapshot)).unwrap();
        let identical_snapshot = compile_topology(&geometry, 2).unwrap();
        let identical =
            TopologyMeshPlan::new(&identical_snapshot, &assign_each_face(&identical_snapshot))
                .unwrap();
        assert_eq!(
            topology_mesh_update_action(&first, &identical),
            TopologyMeshUpdateAction::Reuse
        );

        let mut moved_geometry = geometry.clone();
        moved_geometry.curves[0]
            .spline
            .set_node_point(0, Point2::new(-0.6, -0.5))
            .unwrap();
        let moved_snapshot = compile_topology(&moved_geometry, 3).unwrap();
        let moved =
            TopologyMeshPlan::new(&moved_snapshot, &assign_each_face(&moved_snapshot)).unwrap();
        assert_eq!(
            topology_mesh_update_action(&first, &moved),
            TopologyMeshUpdateAction::FullRebuild(
                TopologyFullRebuildReason::CoordinateRepairDeferred
            )
        );

        let mut reassigned = identical.clone();
        reassigned.domains[0].region = RegionId(99);
        assert_eq!(
            topology_mesh_update_action(&first, &reassigned),
            TopologyMeshUpdateAction::FullRebuild(
                TopologyFullRebuildReason::FaceAssignmentsChanged
            )
        );

        let separated_snapshot = compile_topology(
            &TopologyGeometry {
                curves: vec![square(SpanBehavior::REFLECTING)],
                ..TopologyGeometry::default()
            },
            4,
        )
        .unwrap();
        let separated =
            TopologyMeshPlan::new(&separated_snapshot, &assign_each_face(&separated_snapshot))
                .unwrap();
        let mut changed_law = separated.clone();
        for boundary in &mut changed_law.boundaries {
            if let Some(SpanBehavior::Separated { left, .. }) = &mut boundary.behavior {
                *left = crate::FaceBoundaryCondition::Impedance { ratio: 2.0 };
            }
        }
        assert_eq!(
            topology_mesh_update_action(&separated, &changed_law),
            TopologyMeshUpdateAction::Reuse
        );
    }

    #[test]
    fn transmitting_loop_shares_trace_vertices_between_region_domains() {
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![square(SpanBehavior::Transmitting)],
                ..TopologyGeometry::default()
            },
            9,
        )
        .unwrap();
        let plan = TopologyMeshPlan::new(&topology, &assign_each_face(&topology)).unwrap();
        assert_eq!(plan.geometry_revision, 9);
        assert_eq!(plan.domains.len(), 2);
        let curve_edges = plan
            .boundaries
            .iter()
            .filter(|edge| matches!(edge.source, PlannedBoundarySource::Curve { .. }))
            .collect::<Vec<_>>();
        assert_eq!(curve_edges.len(), 8);
        let unique = curve_edges
            .iter()
            .flat_map(|edge| edge.traces)
            .collect::<BTreeSet<_>>();
        assert_eq!(unique.len(), 4);
    }

    #[test]
    fn excluded_loop_face_becomes_a_one_sided_boundary() {
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![square(SpanBehavior::REFLECTING)],
                ..TopologyGeometry::default()
            },
            0,
        )
        .unwrap();
        let inner = topology.face_at(Point2::new(0.0, 0.0)).unwrap();
        let assignments = topology
            .faces
            .iter()
            .map(|face| FaceRegionAssignment {
                face: face.id,
                region: (face.id != inner).then_some(RegionId(1)),
            })
            .collect::<Vec<_>>();
        let plan = TopologyMeshPlan::new(&topology, &assignments).unwrap();
        assert_eq!(plan.domains.len(), 1);
        assert_eq!(
            plan.boundaries
                .iter()
                .filter(|edge| matches!(edge.source, PlannedBoundarySource::Curve { .. }))
                .count(),
            4
        );
    }

    #[test]
    fn free_baffle_keeps_separate_interior_traces_and_reconnects_tips() {
        let spline = OpenCubicSpline::polyline(vec![
            Point2::new(-0.5, 0.0),
            Point2::new(0.0, 0.0),
            Point2::new(0.5, 0.0),
        ])
        .unwrap();
        let curve = TopologyCurve::new(
            CurveId(1),
            CurveSpline::Open(spline),
            spans(1, 2, SpanBehavior::REFLECTING),
        )
        .unwrap();
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![curve],
                ..TopologyGeometry::default()
            },
            0,
        )
        .unwrap();
        let plan = TopologyMeshPlan::new(
            &topology,
            &[FaceRegionAssignment {
                face: topology.faces[0].id,
                region: Some(RegionId(1)),
            }],
        )
        .unwrap();
        let traces_at = |point: Point2| {
            plan.boundaries
                .iter()
                .filter(|boundary| matches!(boundary.source, PlannedBoundarySource::Curve { .. }))
                .flat_map(|boundary| boundary.points.into_iter().zip(boundary.traces))
                .filter_map(|(candidate, trace)| (candidate == point).then_some(trace))
                .collect::<BTreeSet<_>>()
        };
        assert_eq!(traces_at(Point2::new(-0.5, 0.0)).len(), 1);
        assert_eq!(traces_at(Point2::new(0.5, 0.0)).len(), 1);
        assert_eq!(traces_at(Point2::new(0.0, 0.0)).len(), 2);
    }

    #[test]
    fn transmitting_face_cannot_be_excluded_and_assignments_are_total() {
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![square(SpanBehavior::Transmitting)],
                ..TopologyGeometry::default()
            },
            0,
        )
        .unwrap();
        assert!(matches!(
            TopologyMeshPlan::new(
                &topology,
                &[FaceRegionAssignment {
                    face: topology.faces[0].id,
                    region: Some(RegionId(1)),
                }]
            ),
            Err(TopologyMeshPlanError::MissingFace(_))
        ));
        let assignments = topology
            .faces
            .iter()
            .enumerate()
            .map(|(index, face)| FaceRegionAssignment {
                face: face.id,
                region: (index == 0).then_some(RegionId(1)),
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            TopologyMeshPlan::new(&topology, &assignments),
            Err(TopologyMeshPlanError::TransmittingExcludedFace(_))
        ));
    }

    #[test]
    fn outer_resize_and_junction_trace_membership_survive_projection() {
        let junction = TopologyVertexId(1);
        let mut spline =
            OpenCubicSpline::polyline(vec![Point2::new(0.0, -1.0), Point2::new(0.0, 1.0)]).unwrap();
        spline
            .set_breakpoint_point(0, Point2::new(0.0, -1.0))
            .unwrap();
        let mut curve = TopologyCurve::new(
            CurveId(1),
            CurveSpline::Open(spline),
            spans(1, 1, SpanBehavior::Transmitting),
        )
        .unwrap();
        curve.nodes = vec![
            CurveNode {
                vertex: Some(junction),
            },
            CurveNode {
                vertex: Some(TopologyVertexId(2)),
            },
        ];
        let topology = compile_topology(
            &TopologyGeometry {
                domain: DomainRect::UNIT,
                curves: vec![curve],
                vertices: vec![
                    TopologyVertex {
                        id: junction,
                        location: TopologyVertexLocation::Outer {
                            side: crate::OuterSide::Bottom,
                            fraction: 0.5,
                        },
                    },
                    TopologyVertex {
                        id: TopologyVertexId(2),
                        location: TopologyVertexLocation::Outer {
                            side: crate::OuterSide::Top,
                            fraction: 0.5,
                        },
                    },
                ],
            },
            0,
        )
        .unwrap();
        let plan = TopologyMeshPlan::new(&topology, &assign_each_face(&topology)).unwrap();
        assert!(
            plan.vertices
                .iter()
                .any(|vertex| vertex.point == Point2::new(0.0, -1.0))
        );
    }
}
