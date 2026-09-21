//! What is selected and where its pivot sits: marquee resolution, span and
//! curve selection, and the continuity a selected knot reports.

use bevy::prelude::*;
use bevy_egui::egui::{self};
use funfern_app::document::ProbeSamplingPreset;
use funfern_app::topology_editor::TopologyProbeTarget;
use funfern_app::topology_viewport::{TopologyHit, TopologySelection, TopologySpanTarget};
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
}
