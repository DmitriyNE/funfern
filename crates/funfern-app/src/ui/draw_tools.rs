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
        let finish = gesture.tool == DrawTool::Rectangle && gesture.points.len() == 2;
        self.draw = Some(gesture);
        if finish {
            self.finish_draw();
        }
    }
    pub(super) fn finish_draw(&mut self) {
        let Some(gesture) = self.draw.take() else {
            return;
        };
        let result: Result<CurveId, String> = match gesture.tool {
            DrawTool::Rectangle if gesture.points.len() == 2 => {
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
            DrawTool::Polygon if gesture.points.len() >= 3 => {
                PeriodicCubicSpline::polygon(gesture.points.clone())
                    .map_err(|e| e.to_string())
                    .and_then(|s| self.create_closed(s))
            }
            DrawTool::ClosedSpline if gesture.points.len() >= 4 => {
                PeriodicCubicSpline::uniform(gesture.points.clone())
                    .map_err(|e| e.to_string())
                    .and_then(|s| self.create_closed(s))
            }
            DrawTool::Polyline if gesture.points.len() >= 2 => {
                OpenCubicSpline::polyline(gesture.points.clone())
                    .map_err(|e| e.to_string())
                    .and_then(|s| self.create_open(s, &gesture))
            }
            DrawTool::OpenSpline if gesture.points.len() == 2 => {
                OpenCubicSpline::polyline(gesture.points.clone())
                    .map_err(|e| e.to_string())
                    .and_then(|s| self.create_open(s, &gesture))
            }
            DrawTool::OpenSpline if gesture.points.len() >= 4 => {
                OpenCubicSpline::uniform(gesture.points.clone())
                    .map_err(|e| e.to_string())
                    .and_then(|s| self.create_open(s, &gesture))
            }
            _ => Err("Add enough points to finish this curve".into()),
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
