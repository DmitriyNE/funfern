//! User-authored semantic state for the unified curve topology.
//!
//! Compiled face identifiers belong to one [`TopologySnapshot`] and are never
//! suitable for persistence. This module binds stable regions and exclusions to
//! faces through oriented curve or outer-boundary anchors.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    BACKGROUND_REGION, CompiledEdge, CompiledEdgeSource, CurveId, CurveSpanId, CurveTraceSide,
    DEFAULT_MATERIAL, FaceId, FaceRegionAssignment, MAX_MATERIALS, MAX_TOPOLOGY_FACES,
    MAX_VOLUME_SOURCES, Material, MaterialFrame, MaterialFrameAttachment, MaterialId,
    OuterBoundaryConditions, OuterSide, PhysicsModel, Region, RegionId, TopologyGeometry,
    TopologyIssue, TopologyJob, TopologyMeshPlan, TopologyMeshPlanError, TopologySnapshot,
    VolumeSource, compile_topology,
};

/// A stable locator for one side of a user-authored boundary.
///
/// Parameters must lie strictly inside one compiled atomic edge. Anchors at a
/// breakpoint or crossing are deliberately rejected because more than one face
/// may meet there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FaceAnchor {
    Outer {
        side: OuterSide,
        fraction: f64,
    },
    Curve {
        curve: CurveId,
        span: CurveSpanId,
        side: CurveTraceSide,
        parameter: f64,
    },
}

impl FaceAnchor {
    pub fn resolve(self, topology: &TopologySnapshot) -> Result<FaceId, FaceAnchorIssue> {
        match self {
            Self::Outer { side, fraction } => {
                if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
                    return Err(FaceAnchorIssue::InvalidParameter);
                }
                resolve_unique_edge(
                    topology
                        .edges
                        .iter()
                        .filter(|edge| edge.source == CompiledEdgeSource::Outer(side)),
                    fraction,
                    |edge| edge.left,
                )
            }
            Self::Curve {
                curve,
                span,
                side,
                parameter,
            } => {
                if curve.0 == 0 || span.0 == 0 || !parameter.is_finite() {
                    return Err(FaceAnchorIssue::InvalidParameter);
                }
                let span_exists = topology
                    .edges
                    .iter()
                    .any(|edge| edge.source == CompiledEdgeSource::Curve(span));
                if !span_exists {
                    return Err(FaceAnchorIssue::UnknownSpan(span));
                }
                let curve_exists = topology.edges.iter().any(|edge| edge.curve == Some(curve));
                if !curve_exists {
                    return Err(FaceAnchorIssue::UnknownCurve(curve));
                }
                let matching = topology.edges.iter().filter(|edge| {
                    edge.source == CompiledEdgeSource::Curve(span) && edge.curve == Some(curve)
                });
                resolve_unique_edge(matching, parameter, |edge| match side {
                    CurveTraceSide::Left => edge.left,
                    CurveTraceSide::Right => edge.right,
                })
            }
        }
    }

    /// Resolves two boundary anchors and verifies that they address the same
    /// compiled face. Active/excluded classification belongs to the compiled
    /// scene plan rather than to the geometric snapshot.
    fn shared_face_with(
        self,
        start: FaceAnchor,
        topology: &TopologySnapshot,
    ) -> Result<FaceId, SeparatorAttachmentIssue> {
        let start_face = start
            .resolve(topology)
            .map_err(SeparatorAttachmentIssue::Start)?;
        let end_face = self
            .resolve(topology)
            .map_err(SeparatorAttachmentIssue::End)?;
        if start_face != end_face {
            return Err(SeparatorAttachmentIssue::DifferentFaces {
                start: start_face,
                end: end_face,
            });
        }
        Ok(start_face)
    }
}

fn resolve_unique_edge<'a>(
    edges: impl Iterator<Item = &'a CompiledEdge>,
    parameter: f64,
    face: impl Fn(&CompiledEdge) -> FaceId,
) -> Result<FaceId, FaceAnchorIssue> {
    let mut containing = vec![];
    let mut touches_vertex = false;
    for edge in edges {
        let low = edge.parameter[0].min(edge.parameter[1]);
        let high = edge.parameter[0].max(edge.parameter[1]);
        let scale = (high - low).abs().max(1.0);
        let tolerance = scale * 1.0e-10;
        if parameter >= low - tolerance && parameter <= high + tolerance {
            containing.push(face(edge));
            touches_vertex |= parameter <= low + tolerance || parameter >= high - tolerance;
        }
    }
    containing.sort_unstable();
    containing.dedup();
    match containing.as_slice() {
        [face] => Ok(*face),
        [] => Err(FaceAnchorIssue::NotOnBoundary),
        _ if touches_vertex => Err(FaceAnchorIssue::AtVertex),
        _ => Err(FaceAnchorIssue::Ambiguous),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceAnchorIssue {
    InvalidParameter,
    UnknownCurve(CurveId),
    UnknownSpan(CurveSpanId),
    AtVertex,
    NotOnBoundary,
    Ambiguous,
}

impl std::fmt::Display for FaceAnchorIssue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for FaceAnchorIssue {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeparatorAttachmentIssue {
    Start(FaceAnchorIssue),
    End(FaceAnchorIssue),
    DifferentFaces { start: FaceId, end: FaceId },
    ExcludedFace(FaceId),
}

impl std::fmt::Display for SeparatorAttachmentIssue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for SeparatorAttachmentIssue {}

/// A persisted disposition for one bounded compiled face.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AuthoredFaceAssignment {
    pub anchor: FaceAnchor,
    /// `None` deliberately excludes the face from the simulation.
    pub region: Option<RegionId>,
}

/// Complete authored simulation state using unified open and closed curves.
#[derive(Clone, Debug, PartialEq)]
pub struct TopologyScene {
    pub geometry: TopologyGeometry,
    pub physics: PhysicsModel,
    pub materials: Vec<Material>,
    pub regions: Vec<Region>,
    pub face_assignments: Vec<AuthoredFaceAssignment>,
    pub volume_sources: Vec<VolumeSource>,
    pub outer_boundaries: OuterBoundaryConditions,
}

impl Default for TopologyScene {
    fn default() -> Self {
        Self {
            geometry: TopologyGeometry::default(),
            physics: PhysicsModel::Mechanical,
            materials: vec![Material::default_medium()],
            regions: vec![Region {
                id: BACKGROUND_REGION,
                material: DEFAULT_MATERIAL,
                frame: MaterialFrame::world(),
            }],
            face_assignments: vec![AuthoredFaceAssignment {
                anchor: FaceAnchor::Outer {
                    side: OuterSide::Bottom,
                    fraction: 0.5,
                },
                region: Some(BACKGROUND_REGION),
            }],
            volume_sources: vec![],
            outer_boundaries: OuterBoundaryConditions::default(),
        }
    }
}

impl TopologyScene {
    pub fn material(&self, id: MaterialId) -> Option<&Material> {
        self.materials.iter().find(|material| material.id == id)
    }

    pub fn region(&self, id: RegionId) -> Option<&Region> {
        self.regions.iter().find(|region| region.id == id)
    }

    /// Checks semantic state that does not depend on a compiled arrangement.
    pub fn validate_structure(&self) -> Result<(), TopologySceneIssue> {
        if !self.geometry.domain.valid()
            || self.materials.is_empty()
            || self.materials.len() > MAX_MATERIALS
            || self.regions.is_empty()
            || self.regions.len() > MAX_TOPOLOGY_FACES
            || self.face_assignments.len() > MAX_TOPOLOGY_FACES
            || self.volume_sources.len() > MAX_VOLUME_SOURCES
            || !self.outer_boundaries.valid()
        {
            return Err(TopologySceneIssue::Structure);
        }

        let mut material_ids = BTreeSet::new();
        if self
            .materials
            .iter()
            .any(|material| !material.valid() || !material_ids.insert(material.id))
        {
            return Err(TopologySceneIssue::Material);
        }

        let mut region_ids = BTreeSet::new();
        for region in &self.regions {
            if region.id.0 == 0
                || !region_ids.insert(region.id)
                || !region.frame.valid()
                || self.material(region.material).is_none()
                || (region.id == BACKGROUND_REGION
                    && region.frame.attachment != MaterialFrameAttachment::World)
            {
                return Err(TopologySceneIssue::Region(region.id));
            }
        }
        if !region_ids.contains(&BACKGROUND_REGION) {
            return Err(TopologySceneIssue::Region(BACKGROUND_REGION));
        }

        let mut source_regions = BTreeSet::new();
        for source in &self.volume_sources {
            if !source.valid()
                || !region_ids.contains(&source.region)
                || !source_regions.insert(source.region)
            {
                return Err(TopologySceneIssue::VolumeSource(source.region));
            }
        }

        for [first, second] in [[0, 1], [1, 2], [2, 3], [3, 0]] {
            if let (
                crate::OuterBoundaryCondition::Dirichlet { signal: first },
                crate::OuterBoundaryCondition::Dirichlet { signal: second },
            ) = (
                self.outer_boundaries.sides[first].resolved(self.physics),
                self.outer_boundaries.sides[second].resolved(self.physics),
            ) && first != second
            {
                return Err(TopologySceneIssue::OuterBoundaryCorner);
            }
        }
        Ok(())
    }

    /// Resolves authored face dispositions against one immutable topology result.
    pub fn resolve_face_assignments(
        &self,
        topology: &TopologySnapshot,
    ) -> Result<Vec<FaceRegionAssignment>, TopologySceneIssue> {
        self.validate_structure()?;
        if topology.domain != self.geometry.domain {
            return Err(TopologySceneIssue::RevisionMismatch);
        }

        let known_regions = self
            .regions
            .iter()
            .map(|region| region.id)
            .collect::<BTreeSet<_>>();
        let mut by_face = BTreeMap::<FaceId, Option<RegionId>>::new();
        let mut assigned_regions = BTreeSet::new();
        for (index, assignment) in self.face_assignments.iter().enumerate() {
            self.validate_anchor_geometry(assignment.anchor)
                .map_err(|source| TopologySceneIssue::FaceAnchor { index, source })?;
            let face = assignment
                .anchor
                .resolve(topology)
                .map_err(|source| TopologySceneIssue::FaceAnchor { index, source })?;
            if by_face.insert(face, assignment.region).is_some() {
                return Err(TopologySceneIssue::DuplicateFace(face));
            }
            if let Some(region) = assignment.region {
                if !known_regions.contains(&region) {
                    return Err(TopologySceneIssue::Region(region));
                }
                if !assigned_regions.insert(region) {
                    return Err(TopologySceneIssue::DuplicateRegion(region));
                }
            }
        }

        for face in &topology.faces {
            if !by_face.contains_key(&face.id) {
                return Err(TopologySceneIssue::UnassignedFace(face.id));
            }
        }
        for region in known_regions {
            if !assigned_regions.contains(&region) {
                return Err(TopologySceneIssue::UnassignedRegion(region));
            }
        }

        Ok(topology
            .faces
            .iter()
            .map(|face| FaceRegionAssignment {
                face: face.id,
                region: by_face[&face.id],
            })
            .collect())
    }

    fn validate_anchor_geometry(&self, anchor: FaceAnchor) -> Result<(), FaceAnchorIssue> {
        match anchor {
            FaceAnchor::Outer { fraction, .. } => {
                if !fraction.is_finite() || fraction <= 0.0 || fraction >= 1.0 {
                    Err(FaceAnchorIssue::AtVertex)
                } else {
                    Ok(())
                }
            }
            FaceAnchor::Curve {
                curve,
                span,
                parameter,
                ..
            } => {
                let curve = self
                    .geometry
                    .curves
                    .iter()
                    .find(|candidate| candidate.id == curve)
                    .ok_or(FaceAnchorIssue::UnknownCurve(curve))?;
                let span_index = curve
                    .spans
                    .iter()
                    .position(|candidate| candidate.id == span)
                    .ok_or(FaceAnchorIssue::UnknownSpan(span))?;
                let bounds = curve
                    .spline
                    .span_bounds(span_index)
                    .ok_or(FaceAnchorIssue::UnknownSpan(span))?;
                let scale = (bounds[1] - bounds[0]).abs().max(1.0);
                let tolerance = scale * 1.0e-10;
                if !parameter.is_finite()
                    || parameter <= bounds[0].min(bounds[1]) + tolerance
                    || parameter >= bounds[0].max(bounds[1]) - tolerance
                {
                    Err(FaceAnchorIssue::AtVertex)
                } else {
                    Ok(())
                }
            }
        }
    }

    pub fn compile(&self, revision: u64) -> Result<CompiledTopologyScene, TopologySceneIssue> {
        self.validate_structure()?;
        let topology =
            compile_topology(&self.geometry, revision).map_err(TopologySceneIssue::Topology)?;
        let assignments = self.resolve_face_assignments(&topology)?;
        let plan =
            TopologyMeshPlan::new(&topology, &assignments).map_err(TopologySceneIssue::Plan)?;
        Ok(CompiledTopologyScene {
            topology,
            assignments,
            plan,
        })
    }
}

#[derive(Clone, Debug)]
pub struct CompiledTopologyScene {
    pub topology: TopologySnapshot,
    pub assignments: Vec<FaceRegionAssignment>,
    pub plan: TopologyMeshPlan,
}

impl CompiledTopologyScene {
    /// Returns the existing active region that a new transmitting separator may
    /// split. Both anchors must address boundaries of the same active face.
    pub fn separator_region(
        &self,
        start: FaceAnchor,
        end: FaceAnchor,
    ) -> Result<RegionId, SeparatorAttachmentIssue> {
        let face = end.shared_face_with(start, &self.topology)?;
        self.assignments
            .iter()
            .find(|assignment| assignment.face == face)
            .and_then(|assignment| assignment.region)
            .ok_or(SeparatorAttachmentIssue::ExcludedFace(face))
    }
}

/// Resumable authored-scene compilation. Arrangement work is delegated to the
/// bounded topology job; semantic assignment and plan projection run only after
/// that immutable snapshot is complete.
pub struct TopologySceneJob {
    revision: u64,
    scene: TopologyScene,
    topology: Option<TopologyJob>,
    done: bool,
}

impl TopologySceneJob {
    pub fn new(scene: TopologyScene, revision: u64) -> Self {
        let topology = TopologyJob::new(scene.geometry.clone(), revision);
        Self {
            revision,
            scene,
            topology: Some(topology),
            done: false,
        }
    }

    pub fn phase(&self) -> &'static str {
        if self.done {
            "Finished"
        } else {
            "Compiling topology"
        }
    }

    /// Advances at most `budget` topology work units and returns a result once.
    pub fn advance(
        &mut self,
        budget: usize,
    ) -> Option<Result<CompiledTopologyScene, TopologySceneIssue>> {
        if self.done || budget == 0 {
            return None;
        }
        if let Err(issue) = self.scene.validate_structure() {
            self.done = true;
            self.topology = None;
            return Some(Err(issue));
        }
        let result = self.topology.as_mut()?.advance(budget)?;
        self.done = true;
        self.topology = None;
        Some(match result.result {
            Ok(topology) => {
                if result.revision != self.revision {
                    Err(TopologySceneIssue::RevisionMismatch)
                } else {
                    self.scene
                        .resolve_face_assignments(&topology)
                        .and_then(|assignments| {
                            TopologyMeshPlan::new(&topology, &assignments)
                                .map_err(TopologySceneIssue::Plan)
                                .map(|plan| CompiledTopologyScene {
                                    topology,
                                    assignments,
                                    plan,
                                })
                        })
                }
            }
            Err(issue) => Err(TopologySceneIssue::Topology(issue)),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopologySceneIssue {
    Structure,
    Material,
    Region(RegionId),
    VolumeSource(RegionId),
    OuterBoundaryCorner,
    RevisionMismatch,
    FaceAnchor {
        index: usize,
        source: FaceAnchorIssue,
    },
    DuplicateFace(FaceId),
    DuplicateRegion(RegionId),
    UnassignedFace(FaceId),
    UnassignedRegion(RegionId),
    Topology(TopologyIssue),
    Plan(TopologyMeshPlanError),
}

impl std::fmt::Display for TopologySceneIssue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for TopologySceneIssue {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CurveNode, CurveSpan, CurveSpline, PeriodicCubicSpline, Point2, SpanBehavior, TopologyCurve,
    };

    fn closed_curve(id: u64, span_base: u64, center: Point2, radius: f64) -> TopologyCurve {
        let spline = PeriodicCubicSpline::rounded(center, radius);
        let spans = (0..spline.intervals().len())
            .map(|index| CurveSpan {
                id: CurveSpanId(span_base + index as u64),
                behavior: SpanBehavior::Transmitting,
            })
            .collect();
        TopologyCurve::new(CurveId(id), CurveSpline::Closed(spline), spans).unwrap()
    }

    fn loop_scene() -> TopologyScene {
        let mut scene = TopologyScene::default();
        let curve = closed_curve(1, 10, Point2::new(0.0, 0.0), 0.35);
        let parameter = curve.spline.node_parameter(0).unwrap()
            + (curve.spline.node_parameter(1).unwrap() - curve.spline.node_parameter(0).unwrap())
                * 0.5;
        scene.geometry.curves.push(curve);
        scene.regions.push(Region {
            id: RegionId(2),
            material: DEFAULT_MATERIAL,
            frame: MaterialFrame::world(),
        });
        scene.face_assignments.push(AuthoredFaceAssignment {
            anchor: FaceAnchor::Curve {
                curve: CurveId(1),
                span: CurveSpanId(10),
                side: CurveTraceSide::Left,
                parameter,
            },
            region: Some(RegionId(2)),
        });
        scene
    }

    #[test]
    fn default_scene_resolves_background_without_storing_face_id() {
        let compiled = TopologyScene::default().compile(7).unwrap();
        assert_eq!(compiled.topology.revision, 7);
        assert_eq!(compiled.assignments.len(), 1);
        assert_eq!(compiled.assignments[0].region, Some(BACKGROUND_REGION));
        assert_eq!(compiled.plan.domains.len(), 1);
    }

    #[test]
    fn oriented_curve_anchors_resolve_both_loop_faces() {
        let mut scene = loop_scene();
        let topology = compile_topology(&scene.geometry, 1).unwrap();
        let anchor = scene.face_assignments[1].anchor;
        let inside = anchor.resolve(&topology).unwrap();
        let outside = match anchor {
            FaceAnchor::Curve {
                curve,
                span,
                parameter,
                ..
            } => FaceAnchor::Curve {
                curve,
                span,
                side: CurveTraceSide::Right,
                parameter,
            }
            .resolve(&topology)
            .unwrap(),
            FaceAnchor::Outer { .. } => unreachable!(),
        };
        assert_ne!(inside, outside);
        assert_eq!(
            outside,
            scene.face_assignments[0].anchor.resolve(&topology).unwrap()
        );
        assert_eq!(scene.resolve_face_assignments(&topology).unwrap().len(), 2);

        scene.face_assignments[1].region = None;
        scene.regions.pop();
        let resolved = scene.resolve_face_assignments(&topology).unwrap();
        assert!(
            resolved
                .iter()
                .any(|assignment| assignment.region.is_none())
        );
    }

    #[test]
    fn separator_completion_requires_two_boundaries_of_the_same_face() {
        let scene = loop_scene();
        let compiled = scene.compile(1).unwrap();
        let start = FaceAnchor::Outer {
            side: OuterSide::Bottom,
            fraction: 0.25,
        };
        let curve = &scene.geometry.curves[0];
        let bounds = curve.spline.span_bounds(0).unwrap();
        let background_target = FaceAnchor::Curve {
            curve: curve.id,
            span: curve.spans[0].id,
            side: CurveTraceSide::Right,
            parameter: (bounds[0] + bounds[1]) * 0.5,
        };
        assert_eq!(
            compiled.separator_region(start, background_target).unwrap(),
            BACKGROUND_REGION
        );

        let interior_target = FaceAnchor::Curve {
            curve: curve.id,
            span: curve.spans[0].id,
            side: CurveTraceSide::Left,
            parameter: (bounds[0] + bounds[1]) * 0.5,
        };
        assert!(matches!(
            compiled.separator_region(start, interior_target),
            Err(SeparatorAttachmentIssue::DifferentFaces { .. })
        ));
    }

    #[test]
    fn separator_completion_rejects_an_excluded_face() {
        let mut scene = loop_scene();
        scene.regions.pop();
        scene.face_assignments[1].region = None;
        for span in &mut scene.geometry.curves[0].spans {
            span.behavior = SpanBehavior::REFLECTING;
        }
        let compiled = scene.compile(1).unwrap();
        let curve = &scene.geometry.curves[0];
        let first = curve.spline.span_bounds(0).unwrap();
        let second = curve.spline.span_bounds(2).unwrap();
        let anchor = |span: usize, bounds: [f64; 2]| FaceAnchor::Curve {
            curve: curve.id,
            span: curve.spans[span].id,
            side: CurveTraceSide::Left,
            parameter: (bounds[0] + bounds[1]) * 0.5,
        };
        assert!(matches!(
            compiled.separator_region(anchor(0, first), anchor(2, second)),
            Err(SeparatorAttachmentIssue::ExcludedFace(_))
        ));
    }

    #[test]
    fn transmitting_curve_against_excluded_face_becomes_reflecting() {
        let mut scene = loop_scene();
        let transmitting = scene.compile(0).unwrap();
        assert!(transmitting.plan.boundaries.iter().any(|boundary| {
            matches!(boundary.source, crate::PlannedBoundarySource::Curve { .. })
                && boundary.behavior == Some(SpanBehavior::Transmitting)
        }));
        scene.regions.pop();
        scene.face_assignments[1].region = None;
        let compiled = scene.compile(1).unwrap();
        assert_eq!(compiled.plan.domains.len(), 1);
        assert!(compiled.plan.boundaries.iter().all(|boundary| {
            !matches!(boundary.source, crate::PlannedBoundarySource::Curve { .. })
                || boundary.behavior == Some(SpanBehavior::REFLECTING)
        }));
        assert!(
            scene.geometry.curves[0]
                .spans
                .iter()
                .all(|span| span.behavior == SpanBehavior::Transmitting)
        );
    }

    #[test]
    fn transmitting_curve_between_two_excluded_faces_is_inert_but_valid() {
        let mut scene = TopologyScene::default();
        let mut hole = closed_curve(1, 10, Point2::default(), 0.6);
        for span in &mut hole.spans {
            span.behavior = SpanBehavior::REFLECTING;
        }
        let separator = closed_curve(2, 30, Point2::default(), 0.2);
        let anchor = |curve: &TopologyCurve, side| {
            let bounds = curve.spline.span_bounds(0).unwrap();
            FaceAnchor::Curve {
                curve: curve.id,
                span: curve.spans[0].id,
                side,
                parameter: (bounds[0] + bounds[1]) * 0.5,
            }
        };
        scene.face_assignments.push(AuthoredFaceAssignment {
            anchor: anchor(&hole, CurveTraceSide::Left),
            region: None,
        });
        scene.face_assignments.push(AuthoredFaceAssignment {
            anchor: anchor(&separator, CurveTraceSide::Left),
            region: None,
        });
        scene.geometry.curves = vec![hole, separator];
        let compiled = scene.compile(1).unwrap();
        assert_eq!(compiled.plan.domains.len(), 1);
        assert!(compiled.plan.boundaries.iter().all(|boundary| {
            !matches!(
                boundary.source,
                crate::PlannedBoundarySource::Curve {
                    curve: CurveId(2),
                    ..
                }
            )
        }));
    }

    #[test]
    fn coordinate_motion_preserves_region_anchor() {
        let mut scene = loop_scene();
        let before = scene.compile(1).unwrap();
        let before_face = scene.face_assignments[1]
            .anchor
            .resolve(&before.topology)
            .unwrap();
        let curve = &mut scene.geometry.curves[0];
        let CurveSpline::Closed(spline) = &mut curve.spline else {
            unreachable!()
        };
        for index in 0..spline.controls().len() {
            let point = spline.controls()[index] + Point2::new(0.2, -0.1);
            spline.set_control(index, point).unwrap();
        }
        let after = scene.compile(2).unwrap();
        let after_face = scene.face_assignments[1]
            .anchor
            .resolve(&after.topology)
            .unwrap();
        assert!(before.topology.face(before_face).unwrap().area > 0.0);
        assert!(after.topology.face(after_face).unwrap().area > 0.0);
        assert_eq!(after.plan.domains.len(), 2);
    }

    #[test]
    fn missing_and_duplicate_dispositions_are_specific() {
        let mut scene = loop_scene();
        scene.face_assignments.pop();
        let topology = compile_topology(&scene.geometry, 1).unwrap();
        assert!(matches!(
            scene.resolve_face_assignments(&topology),
            Err(TopologySceneIssue::UnassignedFace(_))
        ));

        let mut scene = loop_scene();
        scene.face_assignments.push(scene.face_assignments[1]);
        let topology = compile_topology(&scene.geometry, 1).unwrap();
        assert!(matches!(
            scene.resolve_face_assignments(&topology),
            Err(TopologySceneIssue::DuplicateFace(_))
        ));
    }

    #[test]
    fn anchors_at_breakpoints_are_rejected_as_ambiguous() {
        let mut scene = loop_scene();
        let topology = compile_topology(&scene.geometry, 1).unwrap();
        let curve = &scene.geometry.curves[0];
        let parameter = curve.spline.node_parameter(0).unwrap();
        scene.face_assignments[1].anchor = FaceAnchor::Curve {
            curve: curve.id,
            span: curve.spans[0].id,
            side: CurveTraceSide::Left,
            parameter,
        };
        assert!(matches!(
            scene.resolve_face_assignments(&topology),
            Err(TopologySceneIssue::FaceAnchor {
                source: FaceAnchorIssue::AtVertex,
                ..
            })
        ));
    }

    #[test]
    fn every_region_requires_exactly_one_active_face() {
        let mut scene = TopologyScene::default();
        scene.regions.push(Region {
            id: RegionId(2),
            material: DEFAULT_MATERIAL,
            frame: MaterialFrame::world(),
        });
        let topology = compile_topology(&scene.geometry, 1).unwrap();
        assert_eq!(
            scene.resolve_face_assignments(&topology),
            Err(TopologySceneIssue::UnassignedRegion(RegionId(2)))
        );
    }

    #[test]
    fn curve_span_mismatch_is_not_accepted() {
        let scene = loop_scene();
        let topology = compile_topology(&scene.geometry, 1).unwrap();
        let anchor = FaceAnchor::Curve {
            curve: CurveId(99),
            span: CurveSpanId(10),
            side: CurveTraceSide::Left,
            parameter: 0.5,
        };
        assert_eq!(
            anchor.resolve(&topology),
            Err(FaceAnchorIssue::UnknownCurve(CurveId(99)))
        );
    }

    #[test]
    fn attached_nodes_remain_part_of_authored_geometry() {
        let scene = loop_scene();
        assert!(
            scene.geometry.curves[0]
                .nodes
                .iter()
                .all(|node| *node == CurveNode::default())
        );
    }

    #[test]
    fn resumable_scene_compilation_matches_synchronous_result() {
        let scene = loop_scene();
        let expected = scene.compile(42).unwrap();
        let mut job = TopologySceneJob::new(scene, 42);
        assert!(job.advance(0).is_none());
        let actual = loop {
            if let Some(result) = job.advance(1) {
                break result.unwrap();
            }
        };
        assert_eq!(actual.topology, expected.topology);
        assert_eq!(actual.assignments, expected.assignments);
        assert_eq!(actual.plan, expected.plan);
        assert!(job.advance(usize::MAX).is_none());
        assert_eq!(job.phase(), "Finished");
    }
}
