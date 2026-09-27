//! The drawing tools: placing points, finishing a curve, creating it in the
//! topology, and deleting a selection back out of it.

use bevy::prelude::*;
use bevy_egui::egui::Rect;
use funfern_app::topology_editor::{
    ClosedCurvePurpose, OpenCurvePurpose, TopologyRemoval, TopologyRemovalTarget,
};
use funfern_app::topology_viewport::{
    ScreenPoint, TopologyHandle, TopologySelection, TopologySpanTarget,
};
use funfern_core::*;
use std::collections::BTreeSet;

use super::*;

impl Playground {
    pub(super) fn draw_click(
        &mut self,
        mut point: Point2,
        screen: ScreenPoint,
        r: Rect,
        snap_to_grid: bool,
    ) {
        let hit = self.draw_attachment_hit(screen, r);
        let Some(mut gesture) = self.draw.take() else {
            return;
        };
        let open = matches!(gesture.tool, DrawTool::Polyline | DrawTool::OpenSpline);
        let mut attachment = None;
        // An attachment is a snap of its own and outranks the grid: the point
        // being welded to is where the curve has to land.
        if open && let Some(hit) = hit {
            point = hit.point;
            attachment = Some(hit.attachment);
        } else if snap_to_grid {
            point = Self::snap_point(point, self.snap_step());
        }
        if gesture.tool == DrawTool::Circle {
            let spline = PeriodicCubicSpline::rounded(point, 0.15);
            let purpose = match self.closed_purpose {
                ClosedPurpose::Subdomain => ClosedCurvePurpose::Subdomain {
                    material: self.resolved_material_selection(),
                },
                ClosedPurpose::Hole => ClosedCurvePurpose::Hole,
            };
            match self.editor.create_closed_curve(spline, purpose) {
                Ok(curve) => {
                    self.select_curve(curve);
                    self.notify("Closed curve added");
                }
                Err(error) => self.notify(error),
            }
            self.invalidate_samples();
            return;
        }
        if gesture
            .points
            .first()
            .is_some_and(|first| (point - *first).norm() < 8.0 / self.scale)
            && gesture.points.len() >= 3
            && !open
        {
            self.draw = Some(gesture);
            self.finish_draw();
            return;
        }
        gesture.points.push(point);
        gesture.attachments.push(attachment);
        // An open curve attaches only at its ends, so an attachment after the
        // first point is where it ends. Kept as an inner point it was drawn
        // through as if it were free, and the curve crossed what it touched.
        let ends_here = attachment.is_some() && gesture.points.len() > 1;
        let finish = gesture.tool == DrawTool::Rectangle && gesture.points.len() == 2;
        self.draw = Some(gesture);
        if finish {
            self.finish_draw();
        } else if ends_here {
            self.finish_draw();
            // Refused: the attachment comes back off, so the curve can still be
            // taken elsewhere, and the reason stays in the status.
            if let Some(gesture) = &mut self.draw {
                gesture.points.pop();
                gesture.attachments.pop();
            }
        }
    }
    pub(super) fn finish_draw(&mut self) {
        let Some(gesture) = self.draw.take() else {
            return;
        };
        let result: Result<CurveId, String> = match gesture.tool {
            _ if !gesture.tool.finishes_with(gesture.points.len()) => {
                Err("Add enough points to finish this curve".into())
            }
            DrawTool::Rectangle => {
                let a = gesture.points[0];
                let b = gesture.points[1];
                let points = vec![
                    Point2::new(a.x, a.y),
                    Point2::new(b.x, a.y),
                    Point2::new(b.x, b.y),
                    Point2::new(a.x, b.y),
                ];
                PeriodicCubicSpline::polygon(points)
                    .map_err(|e| e.to_string())
                    .and_then(|s| self.create_closed(s))
            }
            DrawTool::Polygon => PeriodicCubicSpline::polygon(gesture.points.clone())
                .map_err(|e| e.to_string())
                .and_then(|s| self.create_closed(s)),
            DrawTool::ClosedSpline => PeriodicCubicSpline::uniform(gesture.points.clone())
                .map_err(|e| e.to_string())
                .and_then(|s| self.create_closed(s)),
            DrawTool::Polyline => OpenCubicSpline::polyline(gesture.points.clone())
                .map_err(|e| e.to_string())
                .and_then(|s| self.create_open(s, &gesture)),
            DrawTool::OpenSpline if gesture.points.len() == 2 => {
                OpenCubicSpline::polyline(gesture.points.clone())
                    .map_err(|e| e.to_string())
                    .and_then(|s| self.create_open(s, &gesture))
            }
            // Three points are one arc through the two ends, drawn toward the
            // middle one as the spline draws toward every inner control: the
            // quadratic they define, raised exactly to the cubic a curve is.
            DrawTool::OpenSpline if gesture.points.len() == 3 => {
                let [start, middle, end] = [0, 1, 2].map(|index| gesture.points[index]);
                OpenCubicSpline::uniform(vec![
                    start,
                    start.lerp(middle, 2.0 / 3.0),
                    end.lerp(middle, 2.0 / 3.0),
                    end,
                ])
                .map_err(|e| e.to_string())
                .and_then(|s| self.create_open(s, &gesture))
            }
            DrawTool::OpenSpline => OpenCubicSpline::uniform(gesture.points.clone())
                .map_err(|e| e.to_string())
                .and_then(|s| self.create_open(s, &gesture)),
            DrawTool::Circle => Err("A circle is placed with one click".into()),
        };
        match result {
            Ok(curve) => {
                self.select_curve(curve);
                self.notify("Curve added");
                self.invalidate_samples();
            }
            Err(error) => {
                self.message = error;
                self.draw = Some(gesture);
            }
        }
    }
    /// Takes the latest point back out of the drawing: Backspace, or Undo
    /// point in the Draw palette.
    pub(super) fn undo_draw_point(&mut self) {
        if let Some(draw) = &mut self.draw {
            draw.points.pop();
            draw.attachments.pop();
        }
    }
    /// Opens or closes the Draw palette. It holds the drawing's own controls,
    /// so closing it gives the drawing up rather than leave one going that
    /// only a keyboard could finish or cancel.
    pub(super) fn set_draw_open(&mut self, open: bool) {
        if !open {
            self.draw = None;
        }
        self.draw_open = open;
    }
    pub(super) fn create_closed(&mut self, spline: PeriodicCubicSpline) -> Result<CurveId, String> {
        let purpose = match self.closed_purpose {
            ClosedPurpose::Subdomain => ClosedCurvePurpose::Subdomain {
                material: self.resolved_material_selection(),
            },
            ClosedPurpose::Hole => ClosedCurvePurpose::Hole,
        };
        self.editor.create_closed_curve(spline, purpose)
    }
    pub(super) fn create_open(
        &mut self,
        spline: OpenCubicSpline,
        gesture: &DrawGesture,
    ) -> Result<CurveId, String> {
        let purpose = match self.open_purpose {
            OpenPurpose::Separator => OpenCurvePurpose::SubdomainSeparator {
                material: self.new_separator_material,
            },
            OpenPurpose::Baffle => OpenCurvePurpose::BoundaryBaffle,
        };
        let start = gesture.attachments.first().copied().flatten();
        let end = gesture.attachments.last().copied().flatten();
        self.editor
            .create_open_curve(spline, purpose, start, end)
            .map(|edit| edit.curve)
    }
    pub(super) fn select_curve(&mut self, id: CurveId) {
        if let Some(curve) = self
            .editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == id)
        {
            self.selection = TopologySelection::Spans(
                curve
                    .spans
                    .iter()
                    .map(|span| TopologySpanTarget::Curve(span.id))
                    .collect(),
            );
        }
    }
    pub(super) fn delete_selection(&mut self) {
        if let Some(probe) = self.selected_probe {
            match self.editor.delete_probe(probe) {
                Ok(()) => {
                    self.selected_probe = None;
                    self.probe_windows.remove(&probe);
                }
                Err(error) => self.message = error,
            }
            return;
        }
        if let TopologySelection::Handle(TopologyHandle::Control { curve, control }) =
            &self.selection
        {
            let (curve, control) = (*curve, *control);
            match self.editor.remove_control(curve, control) {
                Ok(()) => {
                    self.selection = TopologySelection::None;
                    self.invalidate_samples();
                }
                Err(error) => self.message = error,
            }
            return;
        }
        let TopologySelection::Spans(targets) = &self.selection else {
            return;
        };
        let spans = targets
            .iter()
            .filter_map(|target| match target {
                TopologySpanTarget::Curve(span) => Some(*span),
                TopologySpanTarget::Outer(_) => None,
            })
            .collect::<BTreeSet<_>>();
        if spans.is_empty() {
            return;
        }
        // The whole selection is one removal, planned once. Which subdomains a
        // deletion merges is a property of all of it together, so asking curve
        // by curve asked the wrong question, closed the history entry to ask it,
        // and left everything after the first question undeleted.
        let target = match self.editor.removal_target(&spans) {
            Ok(target) => target,
            Err(error) => {
                self.message = error;
                return;
            }
        };
        let choices = match self.editor.removal_choices(&target) {
            Ok(choices) => choices,
            Err(error) => {
                self.message = error;
                return;
            }
        };
        // Merging two assigned subdomains needs an explicit survivor, so hand
        // the choice to the scene instead of failing the gesture.
        if choices.len() > 1 {
            self.pending_merge = Some(PendingMerge {
                action: MergeAction::Delete(spans),
                choices,
            });
            return;
        }
        match self.editor.remove(&target, choices.first().copied()) {
            Ok(removal) => {
                self.selection = TopologySelection::None;
                self.material_edit = None;
                self.material_formula_edits.clear();
                self.material_formula_errors.clear();
                self.invalidate_samples();
                self.report_removal(&target, &removal);
            }
            Err(error) => self.message = error,
        }
    }
    /// Says what the deletion did beyond the selection: a curve promoted to a
    /// baffle or a probe dropped is not something to discover later.
    pub(super) fn report_removal(
        &mut self,
        target: &TopologyRemovalTarget,
        removal: &TopologyRemoval,
    ) {
        let curves = target.whole_curves();
        let pieces = removal.pieces.len();
        let lead = match (curves, pieces) {
            (0 | 1, 0) => "Curve deleted".to_owned(),
            (0, 1) => "Deleted spans; the rest is a baffle".to_owned(),
            (0, pieces) => format!("Deleted spans; split into {pieces} baffles"),
            (curves, 0) => format!("{curves} curves deleted"),
            (curves, pieces) => format!(
                "{curves} curve{} deleted and one cut, leaving {pieces} baffle{}",
                if curves == 1 { "" } else { "s" },
                if pieces == 1 { "" } else { "s" },
            ),
        };
        let TopologyRemoval {
            promoted,
            joined,
            removed_probes,
            removed_regions,
            ..
        } = removal;
        let mut parts = vec![lead];
        if !promoted.is_empty() {
            parts.push(format!(
                "{} attached curve{} promoted to baffles",
                promoted.len(),
                if promoted.len() == 1 { "" } else { "s" }
            ));
        }
        let closed = joined
            .iter()
            .filter(|record| record.survivor == record.absorbed)
            .count();
        let fused = joined.len() - closed;
        if fused > 0 {
            parts.push(format!(
                "{} pair{} of loose ends welded into one curve",
                fused,
                if fused == 1 { "" } else { "s" }
            ));
        }
        if closed > 0 {
            parts.push(format!(
                "{} curve{} closed into a loop",
                closed,
                if closed == 1 { "" } else { "s" }
            ));
        }
        if !removed_probes.is_empty() {
            parts.push(format!("{} probe(s) removed", removed_probes.len()));
        }
        if !removed_regions.is_empty() {
            parts.push(format!("{} subdomain(s) merged", removed_regions.len()));
        }
        self.notify(parts.join(" · "));
    }
    /// Stable region owning the committed face under a world point.
    pub(super) fn region_at(&self, point: Point2) -> Option<RegionId> {
        let active = self.runtime.active()?;
        let face = active.bundle.snapshot.face_at(point)?;
        active
            .bundle
            .plan
            .domains
            .iter()
            .find(|domain| domain.face == face)
            .map(|domain| domain.region)
    }
    pub(super) fn place_pulse(&mut self, point: Point2) {
        let Some(active) = self.runtime.active() else {
            return;
        };
        let region = active.bundle.snapshot.face_at(point).and_then(|face| {
            active
                .bundle
                .plan
                .domains
                .iter()
                .find(|domain| domain.face == face)
                .map(|domain| domain.region)
        });
        let Some(region) = region else {
            self.message = "Pulse must be inside an active subdomain".into();
            return;
        };
        self.pending_pulse = Some((point, region));
        self.message = format!("Pulse queued in region {}", region.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use funfern_app::topology_editor::{TopologyAcceptance, TopologyEditor};

    /// A selection made in one document is not a material of the next. An
    /// undo past the material's creation leaves the Materials panel pointing
    /// at nothing, and the subdomain drawn next was refused with no control in
    /// Draw to pick another. It now falls back to the default material, and a
    /// selection that does exist is kept.
    #[test]
    fn a_subdomain_drawn_after_its_material_vanished_takes_the_default() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            closed_purpose: ClosedPurpose::Subdomain,
            ..Playground::default()
        };
        let added = state.editor.add_material().unwrap();
        state.material_selection = added;
        assert!(state.editor.undo(), "the material's creation is undone");
        assert!(state.editor.document.model.draft.material(added).is_none());

        state
            .create_closed(PeriodicCubicSpline::rounded(Point2::new(0.5, 0.5), 0.1))
            .expect("the subdomain is drawn with a material that exists");
        let draft = &state.editor.document.model.draft;
        let materials = draft
            .regions
            .iter()
            .map(|region| region.material)
            .collect::<Vec<_>>();
        assert!(
            materials
                .iter()
                .all(|material| draft.material(*material).is_some())
        );
        assert_eq!(state.material_selection, DEFAULT_MATERIAL);

        let kept = state.editor.add_material().unwrap();
        state.material_selection = kept;
        state
            .create_closed(PeriodicCubicSpline::rounded(Point2::new(-0.5, -0.5), 0.1))
            .unwrap();
        assert_eq!(state.material_selection, kept);
        assert!(
            state
                .editor
                .document
                .model
                .draft
                .regions
                .iter()
                .any(|region| region.material == kept)
        );
    }

    /// Two subdomains selected and deleted in one gesture. Asking curve by
    /// curve asked about the first one only, closed the history entry to ask,
    /// and then returned - so the rest of the selection was never deleted and
    /// nothing said so. One question over the whole deletion, one entry.
    #[test]
    fn a_multi_curve_deletion_asks_once_and_takes_everything() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        for centre in [Point2::new(-0.4, 0.0), Point2::new(0.4, 0.0)] {
            state
                .editor
                .create_closed_curve(
                    PeriodicCubicSpline::rounded(centre, 0.25),
                    ClosedCurvePurpose::Subdomain {
                        material: DEFAULT_MATERIAL,
                    },
                )
                .unwrap();
            settle(&mut state.editor);
        }
        state.selection = TopologySelection::Spans(every_span(&state));
        let history = state.editor.history_len();

        state.delete_selection();
        let pending = state.pending_merge.clone().expect("a survivor question");
        assert_eq!(
            pending.choices.len(),
            3,
            "both subdomains and the background meet in one face: {:?}",
            pending.choices
        );
        assert_eq!(
            state.editor.history_len(),
            history,
            "nothing is deleted until the question is answered"
        );

        state.pick_merge_survivor(Point2::new(0.0, 0.95));
        settle(&mut state.editor);
        assert!(state.pending_merge.is_none());
        assert!(
            state.editor.document.model.draft.geometry.curves.is_empty(),
            "the whole selection goes, not the curve the question was about"
        );
        assert_eq!(
            state.editor.history_len(),
            (history.0 + 1, history.1),
            "one gesture, one entry"
        );
        assert!(state.editor.undo());
        settle(&mut state.editor);
        assert_eq!(
            state.editor.document.model.draft.geometry.curves.len(),
            2,
            "one undo brings the whole gesture back"
        );
    }

    /// A selection covering one curve whole and part of another used to delete
    /// the whole one and ignore the spans on the other without a word.
    #[test]
    fn a_deletion_of_one_whole_curve_and_part_of_another_takes_both() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        let mut holes = vec![];
        for centre in [Point2::new(-0.4, 0.0), Point2::new(0.4, 0.0)] {
            holes.push(
                state
                    .editor
                    .create_closed_curve(
                        PeriodicCubicSpline::rounded(centre, 0.25),
                        ClosedCurvePurpose::Hole,
                    )
                    .unwrap(),
            );
            settle(&mut state.editor);
        }
        let geometry = &state.editor.document.model.draft.geometry;
        let whole = geometry.curve(holes[0]).unwrap();
        let cut = geometry.curve(holes[1]).unwrap();
        let spans = whole
            .spans
            .iter()
            .map(|span| TopologySpanTarget::Curve(span.id))
            .chain(
                cut.spans
                    .iter()
                    .take(2)
                    .map(|span| TopologySpanTarget::Curve(span.id)),
            )
            .collect::<BTreeSet<_>>();
        let survivors = cut.spans.len() - 2;
        state.selection = TopologySelection::Spans(spans);
        let history = state.editor.history_len();

        state.delete_selection();
        settle(&mut state.editor);
        assert!(state.pending_merge.is_none(), "{}", state.message);
        let curves = &state.editor.document.model.draft.geometry.curves;
        assert_eq!(curves.len(), 1, "one baffle is left, and only that");
        assert_eq!(
            curves[0].spans.len(),
            survivors,
            "the run went, the rest stayed"
        );
        assert_eq!(
            state.editor.history_len(),
            (history.0 + 1, history.1),
            "one gesture, one entry"
        );
    }
    /// The frame gizmo must be reachable wherever the numeric placement controls
    /// are, otherwise a region-local profile can only be aligned by typing.
    /// Deleting a selection is one gesture, so it is one undo step however many
    /// curves it covers.
    #[test]
    fn deleting_several_curves_is_one_history_entry() {
        // The default playground opens an example, which this test is not about.
        let mut state = Playground {
            editor: funfern_app::topology_editor::TopologyEditor::default(),
            ..Playground::default()
        };
        let settle = |state: &mut Playground| {
            for _ in 0..100_000 {
                state.editor.validate_frame(4096);
                if state.editor.acceptance != TopologyAcceptance::Pending {
                    return;
                }
            }
            panic!("validation did not terminate")
        };
        settle(&mut state);
        let mut curves = vec![];
        for (index, y) in [0.3f64, 0.0, -0.3].into_iter().enumerate() {
            let x = -0.4 + 0.1 * index as f64;
            curves.push(
                state
                    .editor
                    .create_boundary_baffle(
                        OpenCubicSpline::polyline(vec![
                            Point2::new(x, y),
                            Point2::new(x + 0.2, y + 0.08),
                            Point2::new(x + 0.4, y),
                        ])
                        .unwrap(),
                    )
                    .unwrap(),
            );
            settle(&mut state);
        }
        assert_eq!(state.editor.acceptance, TopologyAcceptance::Valid);
        let before = state.editor.history_len().0;
        state.selection = TopologySelection::Spans(
            state
                .editor
                .document
                .model
                .draft
                .geometry
                .curves
                .iter()
                .flat_map(|curve| curve.spans.iter())
                .map(|span| TopologySpanTarget::Curve(span.id))
                .collect(),
        );

        state.delete_selection();
        settle(&mut state);
        assert_eq!(state.editor.acceptance, TopologyAcceptance::Valid);
        assert!(
            state.editor.document.model.draft.geometry.curves.is_empty(),
            "every selected curve went"
        );
        assert_eq!(
            state.editor.history_len().0,
            before + 1,
            "three curves, one entry"
        );

        assert!(state.editor.undo());
        settle(&mut state);
        assert_eq!(
            state.editor.document.model.draft.geometry.curves.len(),
            curves.len(),
            "one undo brings all three back"
        );
    }
    /// Shift places a point on the same 0.05 grid every drag snaps to. A
    /// default editor has compiled nothing, so no attachment can outrank it and
    /// this is the grid path.
    #[test]
    fn shift_places_a_drawn_point_on_the_grid() {
        let viewport = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        // The grid belongs to the zoom, so the zoom is named: 400 pixels per
        // unit divides fifths of a unit in four, landing on 0.05.
        let mut state = Playground {
            scale: 400.0,
            ..Playground::default()
        };
        state.begin_draw(DrawTool::Polyline);
        state.draw_click(
            Point2::new(0.117, -0.233),
            ScreenPoint::new(0.0, 0.0),
            viewport,
            true,
        );
        state.draw_click(
            Point2::new(-0.481, 0.062),
            ScreenPoint::new(0.0, 0.0),
            viewport,
            false,
        );
        let points = &state.draw.as_ref().unwrap().points;
        assert_eq!(points[0], Point2::new(0.10, -0.25));
        assert_eq!(
            points[1],
            Point2::new(-0.481, 0.062),
            "without shift the point stays where it was put"
        );

        // Every tool goes through the same place, the two-click rectangle
        // included. Clear of the default scene's loop, so it compiles.
        let before = state.editor.document.model.draft.geometry.curves.len();
        state.begin_draw(DrawTool::Rectangle);
        for point in [Point2::new(0.537, 0.562), Point2::new(0.873, 0.818)] {
            state.draw_click(point, ScreenPoint::new(0.0, 0.0), viewport, true);
        }
        assert_eq!(
            state.editor.document.model.draft.geometry.curves.len(),
            before + 1,
            "the rectangle was refused: {}",
            state.message
        );
        let rectangle = state
            .editor
            .document
            .model
            .draft
            .geometry
            .curves
            .last()
            .expect("the rectangle was created")
            .spline
            .clone();
        assert_eq!(rectangle.node_count(), 4);
        for index in 0..rectangle.node_count() {
            let corner = rectangle.node_point(index).unwrap();
            for value in [corner.x, corner.y] {
                let steps = value / 0.05;
                assert!(
                    (steps - steps.round()).abs() < 1.0e-9,
                    "corner off the grid at {value}"
                );
            }
        }
    }

    fn click_at(state: &mut Playground, point: Point2) {
        let viewport = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let screen = state.screen(point, viewport);
        state.draw_click(
            point,
            ScreenPoint::new(screen.x as f64, screen.y as f64),
            viewport,
            false,
        );
    }

    fn last_curve(state: &Playground) -> &TopologyCurve {
        state
            .editor
            .document
            .model
            .draft
            .geometry
            .curves
            .last()
            .unwrap()
    }

    /// An open curve attaches only at its ends, so clicking an attachment after
    /// the first point ends it there. The attachment used to be kept as an
    /// inner point: the polyline through it was refused for touching the
    /// baffle with no junction, and the spline was drawn past it unattached.
    #[test]
    fn an_attachment_ends_the_curve_drawn_to_it() {
        for tool in [DrawTool::Polyline, DrawTool::OpenSpline] {
            let mut state = with_baffles(&[]);
            state.begin_draw(tool);
            for point in [
                Point2::new(-0.6, 0.3),
                Point2::new(-0.3, 0.25),
                Point2::new(0.0, 0.1),
            ] {
                click_at(&mut state, point);
            }
            assert!(state.draw.is_none(), "{tool:?}: {}", state.message);
            let nodes = &last_curve(&state).nodes;
            assert!(nodes[0].vertex.is_none(), "{tool:?} starts free");
            assert!(
                nodes.last().unwrap().vertex.is_some(),
                "{tool:?} ends attached"
            );

            // Starting on one leaves the curve open; the next ends it.
            settle(&mut state.editor);
            state.begin_draw(tool);
            click_at(&mut state, Point2::new(0.0, -0.2));
            assert_eq!(state.draw.as_ref().map(|draw| draw.points.len()), Some(1));
            click_at(&mut state, Point2::new(-1.0, -0.3));
            assert!(state.draw.is_none(), "{tool:?}: {}", state.message);
            let nodes = &last_curve(&state).nodes;
            assert!(nodes.iter().all(|node| node.vertex.is_some()), "{tool:?}");
            settle(&mut state.editor);
            assert_eq!(state.editor.acceptance, TopologyAcceptance::Valid);
        }
    }

    /// An ending that is refused takes the attachment back off: the drawing
    /// stays open with the reason, and the attachment never becomes an inner
    /// point of whatever is drawn next.
    #[test]
    fn a_refused_ending_takes_the_attachment_back_off() {
        let mut state = with_baffles(&[[Point2::new(-0.3, -0.5), Point2::new(-0.3, 0.5)]]);
        let curves = state.editor.document.model.draft.geometry.curves.len();
        state.begin_draw(DrawTool::Polyline);
        click_at(&mut state, Point2::new(-0.6, 0.0));
        // Straight across the other baffle to this one.
        click_at(&mut state, Point2::new(0.0, 0.1));
        let draw = state.draw.as_ref().expect("the drawing stays open");
        assert_eq!(draw.points, vec![Point2::new(-0.6, 0.0)]);
        assert_eq!(draw.attachments, vec![None]);
        assert!(state.message.contains("cross"), "{}", state.message);
        assert_eq!(
            state.editor.document.model.draft.geometry.curves.len(),
            curves
        );

        click_at(&mut state, Point2::new(-0.6, 0.7));
        state.finish_draw();
        assert!(state.draw.is_none(), "{}", state.message);
        assert!(
            last_curve(&state)
                .nodes
                .iter()
                .all(|node| node.vertex.is_none())
        );
    }

    /// Three points make a spline too: the arc through the two ends that the
    /// middle one draws toward. It used to take two points or four and more.
    #[test]
    fn three_points_make_a_spline_arc() {
        let mut state = with_baffles(&[]);
        state.begin_draw(DrawTool::OpenSpline);
        let [start, middle, end] = [
            Point2::new(-0.6, 0.6),
            Point2::new(-0.3, 0.9),
            Point2::new(0.0, 0.6),
        ];
        for point in [start, middle, end] {
            click_at(&mut state, point);
        }
        state.finish_draw();
        assert!(state.draw.is_none(), "{}", state.message);
        let CurveSpline::Open(spline) = &last_curve(&state).spline else {
            panic!("an open curve");
        };
        let close = |a: Point2, b: Point2| (a - b).norm() < 1.0e-12;
        assert!(close(spline.evaluate(0.0), start));
        assert!(close(spline.evaluate(spline.period()), end));
        assert!(close(
            spline.evaluate(0.5 * spline.period()),
            (start + middle * 2.0 + end) * 0.25
        ));
    }

    /// What Finish offers is what finishing takes: the button is enabled by
    /// the rule `finish_draw` refuses by, for every tool and count.
    #[test]
    fn finishing_takes_what_the_finish_button_offers() {
        for tool in [
            DrawTool::Circle,
            DrawTool::Rectangle,
            DrawTool::Polygon,
            DrawTool::ClosedSpline,
            DrawTool::Polyline,
            DrawTool::OpenSpline,
        ] {
            for count in 0..=5 {
                let mut state = Playground {
                    editor: TopologyEditor::default(),
                    ..Playground::default()
                };
                let points = (0..count)
                    .map(|index| {
                        let angle = std::f64::consts::TAU * index as f64 / 5.0;
                        Point2::new(0.4 + 0.3 * angle.cos(), 0.4 + 0.3 * angle.sin())
                    })
                    .collect::<Vec<_>>();
                state.draw = Some(DrawGesture {
                    tool,
                    attachments: vec![None; count],
                    points,
                });
                state.finish_draw();
                assert_eq!(
                    state.draw.is_none(),
                    tool.finishes_with(count),
                    "{tool:?} with {count}: {}",
                    state.message
                );
            }
        }
    }

    /// The palette carries the drawing's controls, so closing it gives the
    /// drawing up: it used to leave one going that only a keyboard could
    /// finish or cancel, and the next tap added a point to it.
    #[test]
    fn closing_the_draw_palette_gives_the_drawing_up() {
        let mut state = Playground {
            draw_open: true,
            ..Playground::default()
        };
        state.begin_draw(DrawTool::Polyline);
        state.set_draw_open(false);
        assert!(state.draw.is_none());
        assert!(!state.draw_open);
        state.set_draw_open(true);
        assert!(state.draw.is_none(), "opening it again starts nothing");
    }

    /// A drawing owns the viewport. A double click beside a curve while
    /// drawing a loop places the drawing's points and leaves the curve
    /// alone, and a double click whose first click ends an open curve on a
    /// baffle does not go on to insert a control into it.
    #[test]
    fn a_double_click_while_drawing_stays_with_the_drawing() {
        let controls =
            |state: &Playground| match &state.editor.document.model.draft.geometry.curves[0].spline
            {
                CurveSpline::Open(spline) => spline.controls().len(),
                CurveSpline::Closed(spline) => spline.controls().len(),
            };
        let mut state = with_baffles(&[]);
        let context = viewport_context(&mut state);
        let before = controls(&state);
        let on_baffle = state.screen(Point2::new(0.0, 0.1), viewport());
        state.begin_draw(DrawTool::Polygon);
        let mut time = 1.0;
        for _ in 0..2 {
            time = viewport_click(&mut state, &context, time, egui::Modifiers::NONE, on_baffle);
        }
        assert_eq!(controls(&state), before, "{}", state.message);
        assert_eq!(state.draw.as_ref().map(|draw| draw.points.len()), Some(2));

        let mut state = with_baffles(&[]);
        let context = viewport_context(&mut state);
        state.begin_draw(DrawTool::Polyline);
        let start = state.screen(Point2::new(-0.6, 0.3), viewport());
        let mut time = viewport_click(&mut state, &context, 1.0, egui::Modifiers::NONE, start);
        time += 1.0;
        for _ in 0..2 {
            time = viewport_click(&mut state, &context, time, egui::Modifiers::NONE, on_baffle);
        }
        assert!(state.draw.is_none());
        assert_eq!(state.message, "Curve added");
        assert!(
            matches!(state.selection, TopologySelection::Spans(_)),
            "the new curve stays selected: {:?}",
            state.selection
        );
    }

    /// The Snap setting lands a click on the grid as Shift does, and Shift
    /// inverts it, so a touchscreen can snap and a keyboard can still place a
    /// point off the grid.
    #[test]
    fn shift_inverts_the_snap_setting() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            scale: 400.0,
            snap_to_grid: true,
            ..Playground::default()
        };
        let context = viewport_context(&mut state);
        state.begin_draw(DrawTool::Polygon);
        let at = egui::pos2(437.3, 211.9);
        let time = viewport_click(&mut state, &context, 1.0, egui::Modifiers::NONE, at);
        viewport_click(&mut state, &context, time + 1.0, egui::Modifiers::SHIFT, at);
        let points = &state.draw.as_ref().unwrap().points;
        let on_grid = |point: Point2| {
            [point.x, point.y]
                .iter()
                .all(|value| ((value / 0.05) - (value / 0.05).round()).abs() < 1.0e-9)
        };
        assert!(on_grid(points[0]), "{:?}", points[0]);
        assert!(!on_grid(points[1]), "{:?}", points[1]);
        assert_eq!(points[1], state.world(at, viewport()));
    }

    /// Picking a tool starts the gesture and leaves the palette up, so one
    /// primitive can follow another without a trip back to the toolbar.
    #[test]
    fn the_draw_palette_outlives_the_gesture_it_starts() {
        let mut state = Playground {
            draw_open: true,
            ..Playground::default()
        };
        state.begin_draw(DrawTool::Circle);
        assert!(state.draw.is_some());
        assert!(state.draw_open, "the palette closes only when it is closed");
        state.begin_draw(DrawTool::Polyline);
        assert!(state.draw_open);
    }
}
