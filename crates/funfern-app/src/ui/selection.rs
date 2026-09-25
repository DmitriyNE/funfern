//! What is selected and where its pivot sits: marquee resolution, span and
//! curve selection, and the continuity a selected knot reports.

use bevy::prelude::*;
use bevy_egui::egui::{self};
use funfern_app::document::ProbeSamplingPreset;
use funfern_app::topology_editor::TopologyProbeTarget;
use funfern_app::topology_viewport::{
    ScreenPoint, TopologyHit, TopologySelection, TopologySpanTarget,
};
use funfern_core::*;
use std::collections::BTreeSet;

use super::*;

impl Playground {
    pub(super) fn selection_pivot(&self, spans: &BTreeSet<CurveSpanId>) -> Option<Point2> {
        let mut sum = Point2::default();
        let mut count = 0usize;
        for curve in &self.editor.document.model.draft.geometry.curves {
            for (index, span) in curve.spans.iter().enumerate() {
                if spans.contains(&span.id) {
                    let [a, b] = curve.spline.span_bounds(index)?;
                    sum = sum
                        + match &curve.spline {
                            CurveSpline::Closed(s) => s.evaluate((a + b) * 0.5),
                            CurveSpline::Open(s) => s.evaluate((a + b) * 0.5),
                        };
                    count += 1;
                }
            }
        }
        (count > 0).then(|| sum / count as f64)
    }
    pub(super) fn gizmo_pivot_for(
        &self,
        selected: &BTreeSet<TopologySpanTarget>,
        spans: &BTreeSet<CurveSpanId>,
    ) -> Option<Point2> {
        self.gizmo_pivot
            .as_ref()
            .filter(|(selection, _)| selection == selected)
            .map(|(_, point)| *point)
            .or_else(|| self.selection_pivot(spans))
    }
    /// One click while a probe is being placed. `raw` is where the pointer is;
    /// `snap` puts it on the grid Shift snaps everything else to, with the same
    /// conventions a probe already follows when it is dragged - a position goes
    /// onto the grid, a radius is itself a multiple of it.
    pub(super) fn probe_placement_click(&mut self, raw: Point2, snap: bool) {
        let Some(mode) = self.probe_mode else {
            return;
        };
        let step = self.snap_step();
        let point = if snap {
            Self::snap_point(raw, step)
        } else {
            raw
        };
        let target = match mode {
            ProbePlacement::Point => Some(TopologyProbeTarget::Point(point)),
            ProbePlacement::Segment { start: None } => {
                self.probe_mode = Some(ProbePlacement::Segment { start: Some(point) });
                self.notify("Choose the line end");
                None
            }
            ProbePlacement::Segment { start: Some(start) } => {
                self.probe_mode = Some(ProbePlacement::Segment { start: None });
                Some(TopologyProbeTarget::Segment {
                    start,
                    end: point,
                    preset: ProbeSamplingPreset::Medium,
                })
            }
            ProbePlacement::Disk { center: None } => {
                self.probe_mode = Some(ProbePlacement::Disk {
                    center: Some(point),
                });
                self.notify("Choose the disk radius");
                None
            }
            ProbePlacement::Disk {
                center: Some(center),
            } => {
                self.probe_mode = Some(ProbePlacement::Disk { center: None });
                Some(TopologyProbeTarget::AreaDisk {
                    center,
                    radius: Self::placed_disk_radius(center, raw, snap, step),
                })
            }
            // A region is picked by the face under the pointer, which
            // the grid has nothing to say about.
            ProbePlacement::Region => self.runtime.active().and_then(|active| {
                let face = active.bundle.snapshot.face_at(raw)?;
                active
                    .bundle
                    .plan
                    .domains
                    .iter()
                    .find(|domain| domain.face == face)
                    .map(|domain| TopologyProbeTarget::AreaRegion(domain.region))
            }),
        };
        if let Some(target) = target {
            match self.editor.create_probe(
                format!("Probe {}", self.editor.document.model.probes.len() + 1),
                [91, 220, 194],
                target,
            ) {
                Ok(id) => {
                    self.probe_windows.insert(id);
                    self.notify("Probe added");
                }
                Err(error) => self.notify(error),
            }
        }
    }
    pub(super) fn snap_point(point: Point2, step: f64) -> Point2 {
        Point2::new(
            (point.x / step).round() * step,
            (point.y / step).round() * step,
        )
    }
    pub(super) fn scale_drag_distance(axis: GizmoScaleAxis, relative: Point2) -> f64 {
        match axis {
            GizmoScaleAxis::Uniform => relative.norm(),
            GizmoScaleAxis::X => relative.x.abs(),
            GizmoScaleAxis::Y => relative.y.abs(),
        }
    }
    pub(super) fn marquee_operation(modifiers: egui::Modifiers) -> MarqueeOperation {
        if modifiers.alt {
            MarqueeOperation::Subtract
        } else if modifiers.shift {
            MarqueeOperation::Add
        } else {
            MarqueeOperation::Replace
        }
    }
    pub(super) fn marquee_result(
        base: &BTreeSet<TopologySpanTarget>,
        hits: BTreeSet<TopologySpanTarget>,
        operation: MarqueeOperation,
    ) -> BTreeSet<TopologySpanTarget> {
        match operation {
            MarqueeOperation::Replace => hits,
            MarqueeOperation::Add => base.union(&hits).copied().collect(),
            MarqueeOperation::Subtract => base.difference(&hits).copied().collect(),
        }
    }
    pub(super) fn selection_from_spans(spans: BTreeSet<TopologySpanTarget>) -> TopologySelection {
        if spans.is_empty() {
            TopologySelection::None
        } else {
            TopologySelection::Spans(spans)
        }
    }
    /// What a press or a click at `screen` lands on. Handles take part only
    /// while they are shown. With spans selected, a press within reach of
    /// one of them is the selection's, so a drag moves it, unless it lands on
    /// a control's own dot: on a polyline every segment carries two controls
    /// on the curve itself, and letting the generous handle radius win there
    /// made a selected curve impossible to move by its body.
    pub(super) fn topology_hit(&self, screen: ScreenPoint, r: egui::Rect) -> Option<TopologyHit> {
        let sampled = self.sampled.as_ref()?;
        let transform = self.transform(r);
        let handles = self.editor.document.presentation.handles;
        let span_radius = self.hit_tolerance(9.0) as f64;
        if let Some(selected) = self.selection.spans().filter(|spans| !spans.is_empty()) {
            let direct = handles
                .then(|| {
                    sampled.hit_test(
                        transform,
                        screen,
                        self.hit_tolerance(DIRECT_HANDLE_RADIUS) as f64,
                        -1.0,
                    )
                })
                .flatten();
            if direct.is_some() {
                return direct;
            }
            if let Some(hit) = sampled.span_hit(transform, screen, span_radius, |target| {
                selected.contains(&target)
            }) {
                return Some(hit);
            }
        }
        if handles {
            sampled.hit_test(
                transform,
                screen,
                self.hit_tolerance(13.0) as f64,
                span_radius,
            )
        } else {
            sampled.span_hit(transform, screen, span_radius, |_| true)
        }
    }
    pub(super) fn drag_starts_inside_span_selection(
        selection: &TopologySelection,
        hit: TopologyHit,
    ) -> bool {
        matches!(
            hit,
            TopologyHit::Span { target, .. }
                if selection
                    .spans()
                    .is_some_and(|spans| spans.contains(&target))
        )
    }
    pub(super) fn selected_end_continuity(
        &self,
        span: CurveSpanId,
    ) -> Option<(CurveId, usize, u8, bool)> {
        let curve = self
            .editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.spans.iter().any(|candidate| candidate.id == span))?;
        let index = curve
            .spans
            .iter()
            .position(|candidate| candidate.id == span)?;
        let breakpoint = if curve.spline.is_open() {
            index + 1
        } else {
            (index + 1) % curve.spans.len()
        };
        let continuity = match &curve.spline {
            CurveSpline::Closed(spline) => spline.continuity(breakpoint),
            CurveSpline::Open(spline) => spline.continuity(breakpoint),
        }?;
        let attached = curve
            .nodes
            .get(breakpoint)
            .is_some_and(|node| node.vertex.is_some());
        Some((curve.id, breakpoint, continuity, attached))
    }
    pub(super) fn selected_complete_curves(&self, spans: &BTreeSet<CurveSpanId>) -> Vec<CurveId> {
        self.editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .filter(|curve| curve.spans.iter().all(|span| spans.contains(&span.id)))
            .map(|curve| curve.id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use funfern_app::topology_viewport::{TopologyHandle, TopologyHit};

    #[test]
    fn marquee_operation_and_direction_are_live_conventions() {
        assert_eq!(
            Playground::marquee_operation(egui::Modifiers::NONE),
            MarqueeOperation::Replace
        );
        assert_eq!(
            Playground::marquee_operation(egui::Modifiers::SHIFT),
            MarqueeOperation::Add
        );
        assert_eq!(
            Playground::marquee_operation(egui::Modifiers::ALT),
            MarqueeOperation::Subtract
        );
        assert_eq!(
            MarqueeContainment::from_drag(Pos2::ZERO, Pos2::new(10.0, 4.0)),
            MarqueeContainment::Enclosed
        );
        assert_eq!(
            MarqueeContainment::from_drag(Pos2::ZERO, Pos2::new(-10.0, 4.0)),
            MarqueeContainment::Crossing
        );
    }

    #[test]
    fn marquee_add_and_subtract_apply_against_the_drag_baseline() {
        let a = TopologySpanTarget::Curve(CurveSpanId(1));
        let b = TopologySpanTarget::Curve(CurveSpanId(2));
        let base = BTreeSet::from([a]);
        let hits = BTreeSet::from([b]);
        assert_eq!(
            Playground::marquee_result(&base, hits.clone(), MarqueeOperation::Replace),
            BTreeSet::from([b])
        );
        assert_eq!(
            Playground::marquee_result(&base, hits.clone(), MarqueeOperation::Add),
            BTreeSet::from([a, b])
        );
        assert_eq!(
            Playground::marquee_result(&BTreeSet::from([a, b]), hits, MarqueeOperation::Subtract,),
            BTreeSet::from([a])
        );
    }

    #[test]
    fn dragging_a_selected_span_preserves_the_complete_selection() {
        let a = TopologySpanTarget::Curve(CurveSpanId(1));
        let b = TopologySpanTarget::Curve(CurveSpanId(2));
        let selection = TopologySelection::Spans(BTreeSet::from([a, b]));

        assert!(Playground::drag_starts_inside_span_selection(
            &selection,
            TopologyHit::Span {
                target: a,
                distance: 0.0,
            },
        ));
        assert!(!Playground::drag_starts_inside_span_selection(
            &selection,
            TopologyHit::Span {
                target: TopologySpanTarget::Curve(CurveSpanId(3)),
                distance: 0.0,
            },
        ));
        assert!(!Playground::drag_starts_inside_span_selection(
            &selection,
            TopologyHit::Handle {
                handle: TopologyHandle::Control {
                    curve: CurveId(1),
                    control: 0,
                },
                distance: 0.0,
            },
        ));
    }

    /// A polyline carries two controls on each of its segments, so a press on
    /// its body is nearly always within the handle radius of one. Unselected,
    /// that press still takes the control; on a selected curve it takes the
    /// selection unless it lands on the control's own dot; and with handles
    /// hidden nothing unseen takes it at all.
    #[test]
    fn a_press_on_a_selected_curve_moves_it_rather_than_a_control_beside_it() {
        let r = test_support::viewport();
        let mut state = Playground::default();
        test_support::settle(&mut state.editor);
        let curve = state
            .editor
            .create_boundary_baffle(
                OpenCubicSpline::polyline(vec![Point2::new(-0.3, 0.8), Point2::new(0.3, 0.8)])
                    .unwrap(),
            )
            .unwrap();
        test_support::settle(&mut state.editor);
        state.invalidate_samples();
        state.refresh_samples(r);
        let geometry = &state.editor.document.model.draft.geometry;
        let spans = geometry
            .curve(curve)
            .unwrap()
            .spans
            .iter()
            .map(|span| TopologySpanTarget::Curve(span.id))
            .collect::<BTreeSet<_>>();
        let inner = state.screen(Point2::new(-0.1, 0.8), r);
        let beside = ScreenPoint::new(inner.x as f64 + 8.0, inner.y as f64);
        let on_dot = ScreenPoint::new(inner.x as f64 + 1.0, inner.y as f64);
        let is_handle = |hit: Option<TopologyHit>| matches!(hit, Some(TopologyHit::Handle { .. }));
        let is_selected_span = |hit: Option<TopologyHit>| matches!(hit, Some(TopologyHit::Span { target, .. }) if spans.contains(&target));

        assert!(
            is_handle(state.topology_hit(beside, r)),
            "unselected, the control wins"
        );
        state.selection = TopologySelection::Spans(spans.clone());
        assert!(
            is_selected_span(state.topology_hit(beside, r)),
            "selected, the body moves the selection"
        );
        assert!(
            is_handle(state.topology_hit(on_dot, r)),
            "the dot itself still takes the control"
        );
        state.editor.document.presentation.handles = false;
        assert!(
            is_selected_span(state.topology_hit(on_dot, r)),
            "a hidden control takes nothing"
        );
        state.selection = TopologySelection::None;
        assert!(
            matches!(
                state.topology_hit(on_dot, r),
                Some(TopologyHit::Span { .. })
            ),
            "a hidden control takes nothing from an unselected curve either"
        );
    }
}
