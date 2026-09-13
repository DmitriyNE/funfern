//! Mesh-facing projection of a compiled curve arrangement.
//!
//! The projection resolves each geometric arrangement vertex into one or more
//! finite-element trace vertices. It is deliberately independent from the old
//! obstacle/interface/baffle labels so the triangulator and solver can migrate
//! without reconstructing sectors from coordinates.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    CompiledBoundaryStep, CompiledEdge, CompiledEdgeSource, CurveId, CurveSpanId, FaceId, Point2,
    RegionId, SpanBehavior, TopologySnapshot, TraceVertexId,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceRegionAssignment {
    pub face: FaceId,
    /// `None` excludes the face from the simulation, as for a hole.
    pub region: Option<RegionId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveTraceSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedFaceDomain {
    pub face: FaceId,
    pub region: RegionId,
    /// Directed trace-vertex cycles with the active face on the left. The first
    /// is the outer cycle; later cycles are holes or slit components.
    pub cycles: Vec<Vec<TraceVertexId>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlannedTraceVertex {
    pub id: TraceVertexId,
    pub point: Point2,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopologyMeshPlan {
    pub geometry_revision: u64,
    pub vertices: Vec<PlannedTraceVertex>,
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
            let cycles = face
                .boundaries
                .iter()
                .map(|cycle| compile_cycle(topology, face.id, cycle))
                .collect::<Result<Vec<_>, _>>()?;
            if cycles.first().is_none_or(|cycle| cycle.len() < 3) {
                return Err(TopologyMeshPlanError::BrokenCycle(face.id));
            }
            domains.push(PlannedFaceDomain {
                face: face.id,
                region,
                cycles,
            });
        }

        let mut boundaries = vec![];
        for edge in &topology.edges {
            append_boundary_sides(edge, &assigned, &mut boundaries)?;
        }
        let vertices = trace_points
            .into_iter()
            .map(|(id, point)| PlannedTraceVertex { id, point })
            .collect();
        Ok(Self {
            geometry_revision: topology.revision,
            vertices,
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
    steps: &[CompiledBoundaryStep],
) -> Result<Vec<TraceVertexId>, TopologyMeshPlanError> {
    let mut cycle = vec![];
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
        expected = Some(end);
    }
    if cycle.first().copied() != expected {
        return Err(TopologyMeshPlanError::BrokenCycle(face));
    }
    Ok(cycle)
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
