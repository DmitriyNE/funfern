//! Attaching an open end: what a drag can join to, the highlights that say
//! so, and the weld that commits it.

use bevy::prelude::*;
use bevy_egui::egui::{self, Color32, Pos2, Rect, Stroke};
use funfern_app::topology_editor::{TopologyAttachment, TopologyWeldOutcome};
use funfern_app::topology_viewport::{
    AttachmentHit, ScreenPoint, TopologyHandle, TopologySelection, weld_hit,
};
use funfern_core::*;

use super::*;

impl Playground {
    pub(super) fn draw_attachment_hit(
        &self,
        pointer: ScreenPoint,
        r: Rect,
    ) -> Option<AttachmentHit> {
        let draw = self.draw.as_ref()?;
        if !matches!(draw.tool, DrawTool::Polyline | DrawTool::OpenSpline) {
            return None;
        }
        let compiled = self.editor.compiled_draft.as_ref()?;
        // No face restriction, for either purpose. Which side of a boundary the
        // pointer is on is not what the user is choosing by clicking it, and
        // the editor settles the face from the drawn path instead.
        let hit = weld_hit(
            compiled,
            &self.editor.document.model.draft.geometry,
            self.transform(r),
            pointer,
            self.hit_tolerance(14.0) as f64,
            None,
            None,
        )?;
        Some(hit)
    }
    pub(super) fn draw_attachment_targets(
        &self,
        painter: &egui::Painter,
        r: Rect,
        draw: &DrawGesture,
    ) {
        if !matches!(draw.tool, DrawTool::Polyline | DrawTool::OpenSpline) {
            return;
        }
        let Some(compiled) = &self.editor.compiled_draft else {
            return;
        };
        // Every boundary is a target for either purpose. A separator no longer
        // has to divide something the moment it is drawn, and which side of a
        // boundary a click lands on is not a choice the user is making.
        let stroke = Stroke::new(2.2, Color32::from_rgba_unmultiplied(248, 196, 112, 105));
        for edge in &compiled.topology.edges {
            painter.line_segment(
                [
                    self.screen(edge.points[0], r),
                    self.screen(edge.points[1], r),
                ],
                stroke,
            );
        }
        for vertex in &compiled.topology.vertices {
            if vertex.authored.is_some() {
                painter.circle_filled(self.screen(vertex.point, r), 5.0, GOLD);
            }
        }
        self.draw_vertexless_nodes(painter, r, None);
        if let Some(pointer) = painter.ctx().pointer_hover_pos()
            && r.contains(pointer)
            && let Some(hit) =
                self.draw_attachment_hit(ScreenPoint::new(pointer.x as f64, pointer.y as f64), r)
        {
            self.draw_snap_ring(painter, r, &hit, None);
        }
    }
    /// Gold dots on every loose end and vertex-less breakpoint a curve may be
    /// welded onto. `dragged` is the end on the move: only its own other end
    /// stays eligible on that curve.
    pub(super) fn draw_vertexless_nodes(
        &self,
        painter: &egui::Painter,
        r: Rect,
        dragged: Option<(CurveId, usize)>,
    ) {
        for candidate in &self.editor.document.model.draft.geometry.curves {
            let open = candidate.spline.is_open();
            let count = candidate.nodes.len();
            for (index, node) in candidate.nodes.iter().enumerate() {
                let is_end = open && (index == 0 || index + 1 == count);
                let eligible = match dragged {
                    Some((curve, dragged_node)) if curve == candidate.id => {
                        is_end && index != dragged_node
                    }
                    _ => true,
                };
                if node.vertex.is_some() || !eligible {
                    continue;
                }
                let Some(point) = candidate.spline.node_point(index) else {
                    continue;
                };
                painter.circle_filled(self.screen(point, r), if is_end { 5.0 } else { 3.5 }, GOLD);
            }
        }
    }
    pub(super) fn draw_snap_ring(
        &self,
        painter: &egui::Painter,
        r: Rect,
        hit: &AttachmentHit,
        dragged: Option<CurveId>,
    ) {
        let point = self.screen(hit.point, r);
        painter.circle_filled(point, 4.0, GOLD);
        painter.circle_stroke(point, 9.0, Stroke::new(2.0, GOLD));
        let label = match hit.attachment {
            TopologyAttachment::LooseEnd { curve, .. } if Some(curve) == dragged => "Close",
            TopologyAttachment::LooseEnd { .. } => "Weld",
            _ => "Attach",
        };
        painter.text(
            point + egui::vec2(11.0, -11.0),
            egui::Align2::LEFT_BOTTOM,
            label,
            egui::FontId::proportional(11.0),
            GOLD,
        );
    }
    /// Targets and the live snap while a loose end is being dragged.
    pub(super) fn draw_weld_targets(&self, painter: &egui::Painter, r: Rect) {
        if self.capturing() {
            return;
        }
        let Some(DragGesture::Endpoint { curve, node, snap }) = &self.drag else {
            return;
        };
        self.draw_vertexless_nodes(painter, r, Some((*curve, *node)));
        if let Some(hit) = snap {
            self.draw_snap_ring(painter, r, hit, Some(*curve));
        }
    }
    /// The node of a loose end when `control` is that end's on-curve control.
    pub(super) fn loose_end_of_control(&self, curve: CurveId, control: usize) -> Option<usize> {
        let curve = self.editor.document.model.draft.geometry.curve(curve)?;
        let CurveSpline::Open(spline) = &curve.spline else {
            return None;
        };
        let node = if control == 0 {
            0
        } else if control + 1 == spline.controls().len() {
            curve.nodes.len() - 1
        } else {
            return None;
        };
        curve.nodes[node].vertex.is_none().then_some(node)
    }
    pub(super) fn endpoint_control(&self, curve: CurveId, node: usize) -> Option<usize> {
        let curve = self.editor.document.model.draft.geometry.curve(curve)?;
        let CurveSpline::Open(spline) = &curve.spline else {
            return None;
        };
        Some(if node == 0 {
            0
        } else {
            spline.controls().len() - 1
        })
    }
    /// What a dragged loose end would weld onto at `pos`. Point targets come
    /// from the draft; junctions and edges from the last valid compile, as the
    /// editor's weld command resolves them.
    pub(super) fn weld_hit_for(
        &self,
        pos: Pos2,
        r: Rect,
        exclude: Option<(CurveId, usize)>,
    ) -> Option<AttachmentHit> {
        let compiled = self
            .editor
            .compiled_draft
            .as_ref()
            .unwrap_or(&self.editor.compiled_accepted);
        weld_hit(
            compiled,
            &self.editor.document.model.draft.geometry,
            self.transform(r),
            ScreenPoint::new(pos.x as f64, pos.y as f64),
            self.hit_tolerance(14.0) as f64,
            exclude,
            None,
        )
    }
    /// Welds the released end onto whatever it landed on. The snap is recomputed
    /// here rather than taken from the gesture: one captured mid-drag may name a
    /// face of a snapshot that validation has since replaced.
    pub(super) fn finish_endpoint_drag(
        &mut self,
        curve: CurveId,
        node: usize,
        pointer: Option<Pos2>,
        r: Rect,
    ) {
        let Some(pos) = pointer else {
            return;
        };
        let Some(hit) = self.weld_hit_for(pos, r, Some((curve, node))) else {
            return;
        };
        let endpoint = if node == 0 { 0 } else { 1 };
        self.weld(curve, node, endpoint, hit.attachment, None);
    }

    /// Welds, or stages the question first. A weld that would fold two
    /// subdomains into one face reports the regions instead of performing it,
    /// and the scene asks the same way a deletion does. Answers whether the
    /// weld is settled, so a question that still stands is not cleared.
    pub(super) fn weld(
        &mut self,
        curve: CurveId,
        node: usize,
        endpoint: usize,
        attachment: TopologyAttachment,
        keep_region: Option<RegionId>,
    ) -> bool {
        let welded = match self
            .editor
            .weld_endpoint(curve, endpoint, attachment, keep_region)
        {
            Ok(TopologyWeldOutcome::Welded(weld)) => weld,
            Ok(TopologyWeldOutcome::NeedsSurvivor(choices)) => {
                self.pending_merge = Some(PendingMerge {
                    action: MergeAction::Weld {
                        curve,
                        node,
                        endpoint,
                        target: attachment,
                    },
                    choices,
                });
                return false;
            }
            Err(error) => {
                self.message = error;
                return false;
            }
        };
        self.selection = match welded.seam_control {
            Some(control) => TopologySelection::Handle(TopologyHandle::Control {
                curve: welded.curve,
                control,
            }),
            None => self
                .endpoint_control(curve, node)
                .map_or(TopologySelection::None, |control| {
                    TopologySelection::Handle(TopologyHandle::Control { curve, control })
                }),
        };
        self.invalidate_samples();
        let what = match attachment {
            TopologyAttachment::LooseEnd { curve: other, .. } if other == curve => {
                "Closed into a loop"
            }
            TopologyAttachment::LooseEnd { .. } => "Welded into one curve",
            TopologyAttachment::Junction { .. } | TopologyAttachment::Breakpoint { .. } => {
                "Attached to junction"
            }
            TopologyAttachment::Boundary(FaceAnchor::Outer { .. }) => {
                "Attached to the outer boundary"
            }
            TopologyAttachment::Boundary(FaceAnchor::Curve { .. }) => "Attached to the curve",
        };
        let mut parts = vec![what.to_owned()];
        if !welded.span_splits.is_empty() {
            parts.push("junction inserted".to_owned());
        }
        if welded.promoted {
            parts.push("curve promoted to a baffle".to_owned());
        }
        if !welded.removed_regions.is_empty() {
            parts.push(format!(
                "{} subdomain{} merged away",
                welded.removed_regions.len(),
                if welded.removed_regions.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ));
        }
        if !welded.removed_probes.is_empty() {
            parts.push(format!(
                "{} probe{} dropped",
                welded.removed_probes.len(),
                if welded.removed_probes.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ));
        }
        self.notify(parts.join(" · "));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use funfern_app::topology_editor::{
        OpenCurvePurpose, TopologyAcceptance, TopologyAttachment, TopologyEditor,
    };

    /// The scene asks for a weld the same way it asks for a deletion: the
    /// question is staged, nothing is welded, and the click that names a
    /// subdomain finishes the weld that raised it.
    #[test]
    fn a_weld_that_merges_subdomains_is_staged_for_the_picker() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        let free = state
            .editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    Point2::new(0.4, -0.3),
                    Point2::new(0.4, 0.0),
                    Point2::new(0.4, 0.3),
                ])
                .unwrap(),
                OpenCurvePurpose::BoundaryBaffle,
                None,
                None,
            )
            .unwrap()
            .curve;
        settle(&mut state.editor);
        let divider = state
            .editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    Point2::new(0.0, -0.8),
                    Point2::new(0.0, 0.0),
                    Point2::new(0.0, 0.8),
                ])
                .unwrap(),
                OpenCurvePurpose::BoundaryBaffle,
                Some(TopologyAttachment::Boundary(FaceAnchor::Outer {
                    side: OuterSide::Bottom,
                    fraction: 0.5,
                })),
                Some(TopologyAttachment::Boundary(FaceAnchor::Outer {
                    side: OuterSide::Top,
                    fraction: 0.5,
                })),
            )
            .unwrap()
            .curve;
        settle(&mut state.editor);
        assert_eq!(state.editor.document.model.draft.regions.len(), 2);

        // Anchor on the middle of the free baffle, whichever side resolves.
        let compiled = &state.editor.compiled_accepted;
        let owner = compiled.geometry.curve(free).unwrap();
        let span = owner.spans[1].id;
        let [a, b] = owner.spline.span_bounds(1).unwrap();
        let parameter = (a + b) * 0.5;
        let side = [CurveTraceSide::Left, CurveTraceSide::Right]
            .into_iter()
            .find(|side| {
                FaceAnchor::Curve {
                    curve: free,
                    span,
                    side: *side,
                    parameter,
                }
                .resolve(&compiled.topology)
                .is_ok()
            })
            .expect("a resolvable side");
        let onto = TopologyAttachment::Boundary(FaceAnchor::Curve {
            curve: free,
            span,
            side,
            parameter,
        });
        state.editor.detach_endpoint(divider, 1).unwrap();
        settle(&mut state.editor);
        let node = state
            .editor
            .document
            .model
            .draft
            .geometry
            .curve(divider)
            .unwrap()
            .nodes
            .len()
            - 1;

        state.weld(divider, node, 1, onto, None);
        let pending = state.pending_merge.clone().expect("a survivor question");
        assert!(matches!(pending.action, MergeAction::Weld { .. }));
        assert_eq!(pending.choices.len(), 2);
        assert_eq!(state.editor.document.model.draft.regions.len(), 2);

        state.pick_merge_survivor(Point2::new(-0.5, 0.0));
        settle(&mut state.editor);
        assert!(state.pending_merge.is_none(), "{}", state.message);
        assert_eq!(state.editor.document.model.draft.regions.len(), 1);
        assert_eq!(state.editor.acceptance, TopologyAcceptance::Valid);
    }
}
