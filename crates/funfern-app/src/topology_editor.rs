//! Headless document and command layer for the unified topology application.
//!
//! This module is intentionally not wired into the visible editor until its
//! semantic commands and persistence contract are complete.

use crate::editor::{FarFieldSettings, PresentationSettings, ProbeId, ProbeSamplingPreset};
use funfern_core::*;
use std::collections::BTreeSet;

const HISTORY_LIMIT: usize = 100;

#[derive(Clone, Debug, PartialEq)]
pub struct TopologyBoundaryProbeTarget {
    pub curve: CurveId,
    pub spans: Vec<CurveSpanId>,
    pub side: CurveTraceSide,
    pub reversed: bool,
    pub preset: ProbeSamplingPreset,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TopologyProbeTarget {
    Point(Point2),
    Segment {
        start: Point2,
        end: Point2,
        preset: ProbeSamplingPreset,
    },
    Boundary(TopologyBoundaryProbeTarget),
    AreaDisk {
        center: Point2,
        radius: f64,
    },
    AreaRegion(RegionId),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopologyProbeDefinition {
    pub id: ProbeId,
    pub name: String,
    pub target: TopologyProbeTarget,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopologyDocumentModel {
    pub draft: TopologyScene,
    pub accepted: TopologyScene,
    pub probes: Vec<TopologyProbeDefinition>,
    pub source: PointSource,
    pub far_field: FarFieldSettings,
}

impl Default for TopologyDocumentModel {
    fn default() -> Self {
        let scene = TopologyScene::default();
        Self {
            draft: scene.clone(),
            accepted: scene,
            probes: vec![],
            source: PointSource::default(),
            far_field: FarFieldSettings::default(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TopologyDocument {
    pub model: TopologyDocumentModel,
    pub presentation: PresentationSettings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopologyAcceptance {
    Pending,
    Valid,
    Invalid(TopologySceneIssue),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClosedCurvePurpose {
    Subdomain { material: MaterialId },
    Hole,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenCurvePurpose {
    SubdomainSeparator { material: MaterialId },
    BoundaryBaffle,
}

pub struct TopologyEditor {
    pub document: TopologyDocument,
    pub revision: u64,
    pub acceptance: TopologyAcceptance,
    pub compiled_draft: Option<CompiledTopologyScene>,
    pub compiled_accepted: CompiledTopologyScene,
    undo: Vec<TopologyDocumentModel>,
    redo: Vec<TopologyDocumentModel>,
    before: Option<TopologyDocumentModel>,
    job: Option<TopologySceneJob>,
    next_curve: u64,
    next_span: u64,
    next_region: u64,
}

impl Default for TopologyEditor {
    fn default() -> Self {
        let document = TopologyDocument::default();
        let compiled = document.model.accepted.compile(0).unwrap();
        Self {
            document,
            revision: 0,
            acceptance: TopologyAcceptance::Valid,
            compiled_draft: Some(compiled.clone()),
            compiled_accepted: compiled,
            undo: vec![],
            redo: vec![],
            before: None,
            job: None,
            next_curve: 1,
            next_span: 1,
            next_region: BACKGROUND_REGION.0 + 1,
        }
    }
}

impl TopologyEditor {
    pub fn begin(&mut self) {
        if self.before.is_none() {
            self.before = Some(self.document.model.clone());
        }
    }

    pub fn commit(&mut self) {
        let Some(before) = self.before.take() else {
            return;
        };
        if before == self.document.model {
            return;
        }
        self.undo.push(before);
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn cancel(&mut self) {
        if let Some(before) = self.before.take() {
            self.document.model = before;
            self.changed();
        }
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.before = None;
        self.redo
            .push(std::mem::replace(&mut self.document.model, previous));
        self.changed();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.before = None;
        self.undo
            .push(std::mem::replace(&mut self.document.model, next));
        self.changed();
        true
    }

    pub fn history_len(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }

    pub fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.acceptance = TopologyAcceptance::Pending;
        self.compiled_draft = None;
        self.job = Some(TopologySceneJob::new(
            self.document.model.draft.clone(),
            self.revision,
        ));
    }

    pub fn validate_frame(&mut self, budget: usize) {
        let Some(result) = self.job.as_mut().and_then(|job| job.advance(budget)) else {
            return;
        };
        self.job = None;
        match result {
            Ok(compiled) if compiled.topology.revision == self.revision => {
                self.document.model.accepted = self.document.model.draft.clone();
                self.compiled_accepted = compiled.clone();
                self.compiled_draft = Some(compiled);
                self.acceptance = TopologyAcceptance::Valid;
            }
            Err(issue) => self.acceptance = TopologyAcceptance::Invalid(issue),
            Ok(_) => {}
        }
    }

    pub fn create_closed_curve(
        &mut self,
        spline: PeriodicCubicSpline,
        purpose: ClosedCurvePurpose,
    ) -> Result<CurveId, String> {
        let curve_id = self.allocate_curve()?;
        let behavior = match purpose {
            ClosedCurvePurpose::Subdomain { material } => {
                self.require_material(material)?;
                SpanBehavior::Transmitting
            }
            ClosedCurvePurpose::Hole => SpanBehavior::REFLECTING,
        };
        let spans = self.allocate_spans(spline.intervals().len(), behavior)?;
        let curve = TopologyCurve::new(curve_id, CurveSpline::Closed(spline), spans)
            .map_err(|issue| issue.to_string())?;

        let mut candidate = self.document.model.draft.clone();
        candidate.geometry.curves.push(curve);
        let topology = compile_topology(&candidate.geometry, self.revision.wrapping_add(1))
            .map_err(|issue| issue.to_string())?;
        let assigned = candidate
            .face_assignments
            .iter()
            .map(|assignment| assignment.anchor.resolve(&topology))
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(|issue| issue.to_string())?;
        let curve = candidate.geometry.curves.last().unwrap();
        let bounds = curve.spline.span_bounds(0).unwrap();
        let parameter = (bounds[0] + bounds[1]) * 0.5;
        let mut anchors =
            [CurveTraceSide::Left, CurveTraceSide::Right].map(|side| FaceAnchor::Curve {
                curve: curve_id,
                span: curve.spans[0].id,
                side,
                parameter,
            });
        let anchor = anchors
            .iter_mut()
            .find(|anchor| {
                anchor
                    .resolve(&topology)
                    .is_ok_and(|face| !assigned.contains(&face))
            })
            .copied()
            .ok_or("Closed curve does not create one new interior face")?;
        let region = match purpose {
            ClosedCurvePurpose::Subdomain { material } => {
                let id = self.allocate_region()?;
                candidate.regions.push(Region {
                    id,
                    material,
                    frame: MaterialFrame::world(),
                });
                Some(id)
            }
            ClosedCurvePurpose::Hole => None,
        };
        candidate
            .face_assignments
            .push(AuthoredFaceAssignment { anchor, region });
        candidate
            .compile(self.revision.wrapping_add(1))
            .map_err(|issue| issue.to_string())?;
        self.begin();
        self.document.model.draft = candidate;
        self.changed();
        self.commit();
        Ok(curve_id)
    }

    pub fn create_boundary_baffle(&mut self, spline: OpenCubicSpline) -> Result<CurveId, String> {
        let curve_id = self.allocate_curve()?;
        let spans = self.allocate_spans(spline.intervals().len(), SpanBehavior::REFLECTING)?;
        let curve = TopologyCurve::new(curve_id, CurveSpline::Open(spline), spans)
            .map_err(|issue| issue.to_string())?;
        let mut candidate = self.document.model.draft.clone();
        candidate.geometry.curves.push(curve);
        candidate
            .compile(self.revision.wrapping_add(1))
            .map_err(|issue| issue.to_string())?;
        self.begin();
        self.document.model.draft = candidate;
        self.changed();
        self.commit();
        Ok(curve_id)
    }

    pub fn set_control(
        &mut self,
        curve: CurveId,
        control: usize,
        point: Point2,
    ) -> Result<(), String> {
        let curve = self
            .document
            .model
            .draft
            .geometry
            .curves
            .iter_mut()
            .find(|candidate| candidate.id == curve)
            .ok_or("Curve no longer exists")?;
        match &mut curve.spline {
            CurveSpline::Closed(spline) => spline.set_control(control, point),
            CurveSpline::Open(spline) => spline.set_control(control, point),
        }
        .map_err(|error| error.to_string())?;
        self.changed();
        Ok(())
    }

    pub fn set_span_behavior(
        &mut self,
        spans: &BTreeSet<CurveSpanId>,
        behavior: SpanBehavior,
    ) -> Result<(), String> {
        if !behavior.valid() {
            return Err("Boundary behavior is invalid".into());
        }
        self.begin();
        let mut changed = false;
        for curve in &mut self.document.model.draft.geometry.curves {
            for span in &mut curve.spans {
                if spans.contains(&span.id) && span.behavior != behavior {
                    span.behavior = behavior;
                    changed = true;
                }
            }
        }
        if !changed {
            self.before = None;
            return Err("Select existing curve spans".into());
        }
        self.changed();
        self.commit();
        Ok(())
    }

    fn require_material(&self, material: MaterialId) -> Result<(), String> {
        self.document
            .model
            .draft
            .material(material)
            .map(|_| ())
            .ok_or("Choose an existing material".into())
    }

    fn allocate_curve(&mut self) -> Result<CurveId, String> {
        let id = CurveId(self.next_curve);
        self.next_curve = self
            .next_curve
            .checked_add(1)
            .ok_or("Curve IDs exhausted")?;
        Ok(id)
    }

    fn allocate_spans(
        &mut self,
        count: usize,
        behavior: SpanBehavior,
    ) -> Result<Vec<CurveSpan>, String> {
        (0..count)
            .map(|_| {
                let id = CurveSpanId(self.next_span);
                self.next_span = self.next_span.checked_add(1).ok_or("Span IDs exhausted")?;
                Ok(CurveSpan { id, behavior })
            })
            .collect()
    }

    fn allocate_region(&mut self) -> Result<RegionId, String> {
        let id = RegionId(self.next_region);
        self.next_region = self
            .next_region
            .checked_add(1)
            .ok_or("Region IDs exhausted")?;
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(editor: &mut TopologyEditor) {
        for _ in 0..100_000 {
            editor.validate_frame(64);
            if editor.acceptance != TopologyAcceptance::Pending {
                return;
            }
        }
        panic!("topology editor validation did not finish");
    }

    #[test]
    fn closed_subdomain_and_hole_are_atomic_history_commands() {
        let mut editor = TopologyEditor::default();
        let original = editor.document.model.clone();
        editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(-0.35, 0.0), 0.2),
                ClosedCurvePurpose::Subdomain {
                    material: DEFAULT_MATERIAL,
                },
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(editor.document.model.draft.regions.len(), 2);
        assert_eq!(editor.history_len(), (1, 0));

        editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(0.35, 0.0), 0.2),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.compiled_accepted.plan.domains.len(), 2);
        assert_eq!(editor.history_len(), (2, 0));
        assert!(editor.undo());
        settle(&mut editor);
        assert_eq!(editor.document.model.draft.geometry.curves.len(), 1);
        assert!(editor.undo());
        settle(&mut editor);
        assert_eq!(editor.document.model, original);
    }

    #[test]
    fn invalid_drag_preserves_accepted_scene_and_cancels_exactly() {
        let mut editor = TopologyEditor::default();
        let curve = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.2),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        settle(&mut editor);
        let accepted = editor.document.model.accepted.clone();
        editor.begin();
        editor.set_control(curve, 0, Point2::new(4.0, 0.0)).unwrap();
        settle(&mut editor);
        assert!(matches!(editor.acceptance, TopologyAcceptance::Invalid(_)));
        assert_eq!(editor.document.model.accepted, accepted);
        editor.cancel();
        settle(&mut editor);
        assert_eq!(editor.document.model.draft, accepted);
        assert_eq!(editor.history_len(), (1, 0));
    }

    #[test]
    fn bulk_span_behavior_is_one_undoable_action() {
        let mut editor = TopologyEditor::default();
        let _curve = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.2),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        settle(&mut editor);
        let spans = editor.document.model.draft.geometry.curves[0]
            .spans
            .iter()
            .take(3)
            .map(|span| span.id)
            .collect();
        let coupled = SpanBehavior::Separated {
            left: FaceBoundaryCondition::Reflecting,
            right: FaceBoundaryCondition::Reflecting,
            coupling: InternalBoundaryCoupling::ThinGap {
                stiffness_ratio: 1.0,
            },
        };
        editor.set_span_behavior(&spans, coupled).unwrap();
        settle(&mut editor);
        assert!(matches!(editor.acceptance, TopologyAcceptance::Invalid(_)));
        assert_eq!(editor.history_len(), (2, 0));
        assert!(editor.undo());
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert!(
            editor.document.model.draft.geometry.curves[0]
                .spans
                .iter()
                .all(|span| span.behavior == SpanBehavior::REFLECTING)
        );
    }

    #[test]
    fn free_baffle_is_valid_and_keeps_both_trace_sides() {
        let mut editor = TopologyEditor::default();
        let curve = editor
            .create_boundary_baffle(
                OpenCubicSpline::polyline(vec![
                    Point2::new(-0.5, 0.0),
                    Point2::new(0.0, 0.2),
                    Point2::new(0.5, 0.0),
                ])
                .unwrap(),
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        let sides = editor
            .compiled_accepted
            .plan
            .boundaries
            .iter()
            .filter_map(|boundary| match boundary.source {
                PlannedBoundarySource::Curve {
                    curve: candidate,
                    side,
                    ..
                } if candidate == curve => Some(side),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            sides,
            BTreeSet::from([CurveTraceSide::Left, CurveTraceSide::Right])
        );
        assert_eq!(editor.history_len(), (1, 0));
    }
}
