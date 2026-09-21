//! The outer domain and the material frame: their handles, their cursors and
//! the overlay refresh behind them.

use crate::material_overlay::{
    MaterialOverlay, MaterialOverlayJob, MaterialProperty, OverlayKey, OverlayRange,
};
use bevy_egui::egui::{self, Color32, Pos2, Rect, Stroke};
use funfern_core::*;

use super::*;

impl Playground {
    pub(super) fn refresh_material_overlay(&mut self) {
        if !matches!(
            self.editor.document.presentation.material_overlay,
            MaterialOverlay::Property(_)
        ) {
            self.material_overlay_job = None;
            return;
        }
        let Some(active) = self.runtime.active() else {
            return;
        };
        let key = OverlayKey {
            mesh_revision: active.mesh.mesh_revision,
            scene: active.bundle.authored.as_ref().clone(),
        };
        if self
            .material_overlay_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.key == key)
        {
            self.material_overlay_job = None;
            return;
        }
        if !self
            .material_overlay_job
            .as_ref()
            .is_some_and(|job| job.key() == &key)
        {
            match MaterialOverlayJob::new(
                active.mesh.clone(),
                active.operator.clone(),
                key.scene.clone(),
            ) {
                Ok(job) => {
                    self.material_overlay_job = Some(job);
                    self.material_overlay_error = None;
                }
                Err(error) => {
                    self.material_overlay_job = None;
                    self.material_overlay_error = Some(error);
                    return;
                }
            }
        }
        if let Some(snapshot) = self
            .material_overlay_job
            .as_mut()
            .and_then(|job| job.advance(2_048))
        {
            if snapshot.key == key {
                self.material_overlay_snapshot = Some(snapshot);
                self.material_overlay_error = None;
            }
            self.material_overlay_job = None;
        }
    }
    pub(super) fn material_overlay_range(
        &self,
        property: MaterialProperty,
    ) -> Option<OverlayRange> {
        let presentation = self.editor.document.presentation;
        if presentation.material_overlay_auto_range {
            self.material_overlay_snapshot
                .as_ref()?
                .range(property, presentation.material_overlay_logarithmic)
        } else if presentation.material_overlay_manual_min.is_finite()
            && presentation.material_overlay_manual_max.is_finite()
            && presentation.material_overlay_manual_min < presentation.material_overlay_manual_max
        {
            Some(OverlayRange {
                minimum: presentation.material_overlay_manual_min,
                maximum: presentation.material_overlay_manual_max,
            })
        } else {
            None
        }
    }
    /// The frame a region-local material profile or volume source is written in,
    /// shown only while Materials is open and something actually uses it.
    pub(super) fn selected_material_frame(&self) -> Option<(RegionId, MaterialFrame)> {
        if self.inspector != Some(InspectorPanel::Materials) {
            return None;
        }
        let scene = &self.editor.document.model.draft;
        let region = scene.region(self.region_selection)?;
        let material_uses = scene
            .material(region.material)
            .is_some_and(Material::uses_frame);
        let source_uses = scene
            .volume_sources
            .iter()
            .any(|source| source.region == region.id && source.enabled && source.varying());
        (material_uses || source_uses).then_some((region.id, region.frame))
    }
    pub(super) fn hit_material_frame_gizmo(
        &self,
        point: Pos2,
        r: Rect,
    ) -> Option<MaterialFrameGizmoHit> {
        let (_, frame) = self.selected_material_frame()?;
        let distance = self.screen(frame.origin, r).distance(point);
        if distance <= self.hit_tolerance(11.0) {
            Some(MaterialFrameGizmoHit::Origin)
        } else if (distance - MATERIAL_FRAME_RADIUS).abs() <= self.hit_tolerance(9.0) {
            Some(MaterialFrameGizmoHit::Rotate)
        } else {
            None
        }
    }
    pub(super) fn draw_material_frame(&self, painter: &egui::Painter, r: Rect) {
        if self.capturing() {
            return;
        }
        let Some((_, frame)) = self.selected_material_frame() else {
            return;
        };
        let center = self.screen(frame.origin, r);
        let (sin, cos) = frame.angle_radians.sin_cos();
        let x_end = center + egui::vec2((cos * 33.0) as f32, (-sin * 33.0) as f32);
        let y_end = center + egui::vec2((-sin * 27.0) as f32, (-cos * 27.0) as f32);
        painter.circle_stroke(center, MATERIAL_FRAME_RADIUS, Stroke::new(1.5, TEAL));
        painter.circle_filled(
            center
                + egui::vec2(
                    cos as f32 * MATERIAL_FRAME_RADIUS,
                    -sin as f32 * MATERIAL_FRAME_RADIUS,
                ),
            4.5,
            TEAL,
        );
        painter.line_segment([center, x_end], Stroke::new(2.0, RED));
        painter.line_segment([center, y_end], Stroke::new(2.0, TEAL));
        painter.circle_filled(center, 6.0, Color32::from_rgb(16, 23, 31));
        painter.circle_stroke(center, 6.0, Stroke::new(2.0, GOLD));
        for (end, label, color) in [(x_end, "x", RED), (y_end, "y", TEAL)] {
            painter.text(
                end,
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::monospace(11.0),
                color,
            );
        }
    }
    /// Screen-space hit radius. Touch input keeps the drawn controls small but
    /// widens what counts as a hit.
    /// Small grips on the outer rectangle's corners, so the resize is something
    /// the user can see rather than have to know about. Hidden while a gesture
    /// that has nothing to do with the domain is running.
    pub(super) fn draw_domain_handles(&self, painter: &egui::Painter, r: Rect) {
        if self.capturing()
            || self.draw.is_some()
            || self.pending_merge.is_some()
            || matches!(
                self.drag,
                Some(
                    DragGesture::Marquee { .. }
                        | DragGesture::Spans { .. }
                        | DragGesture::Rotate { .. }
                        | DragGesture::Scale { .. }
                )
            )
        {
            return;
        }
        let dragging = matches!(self.drag, Some(DragGesture::Domain { .. }));
        for (index, corner) in self
            .editor
            .document
            .model
            .draft
            .geometry
            .domain
            .corners()
            .into_iter()
            .enumerate()
        {
            let center = self.screen(corner, r);
            if !r.contains(center) {
                continue;
            }
            let held = dragging
                && matches!(
                    self.drag,
                    Some(DragGesture::Domain {
                        drag: DomainDrag::Corner { index: held, .. },
                    }) if held == index
                );
            let half = if held { 5.0 } else { 4.0 };
            let rect = egui::Rect::from_center_size(center, egui::vec2(half * 2.0, half * 2.0));
            painter.rect_filled(rect, 1.0, Color32::from_rgba_unmultiplied(8, 13, 18, 220));
            painter.rect_stroke(
                rect,
                1.0,
                Stroke::new(1.0, if held { GOLD } else { TEAL }),
                egui::StrokeKind::Outside,
            );
        }
    }

    /// The outer rectangle's corner under the pointer, if any. Corners win over
    /// the sides they meet, so a corner drag is always reachable.
    pub(super) fn hit_domain_corner(&self, point: Pos2, viewport: Rect) -> Option<usize> {
        self.editor
            .document
            .model
            .draft
            .geometry
            .domain
            .corners()
            .into_iter()
            .enumerate()
            .map(|(index, corner)| (index, self.screen(corner, viewport).distance(point)))
            .filter(|(_, distance)| *distance <= self.hit_tolerance(10.0))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    }

    pub(super) fn domain_corner_cursor(index: usize) -> egui::CursorIcon {
        match index {
            0 | 2 => egui::CursorIcon::ResizeNeSw,
            _ => egui::CursorIcon::ResizeNwSe,
        }
    }

    /// Where the dragged side or corner lands, leaving the rest of the rectangle
    /// where it was.
    pub(super) fn resize_domain(start: DomainRect, drag: DomainDrag, point: Point2) -> DomainRect {
        let mut domain = start;
        match drag {
            DomainDrag::Side { side, .. } => match side {
                OuterSide::Bottom => domain.min_y = point.y,
                OuterSide::Right => domain.max_x = point.x,
                OuterSide::Top => domain.max_y = point.y,
                OuterSide::Left => domain.min_x = point.x,
            },
            DomainDrag::Corner { index, .. } => match index {
                0 => {
                    domain.min_x = point.x;
                    domain.min_y = point.y;
                }
                1 => {
                    domain.max_x = point.x;
                    domain.min_y = point.y;
                }
                2 => {
                    domain.max_x = point.x;
                    domain.max_y = point.y;
                }
                _ => {
                    domain.min_x = point.x;
                    domain.max_y = point.y;
                }
            },
        }
        domain
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use funfern_app::topology_editor::TopologyProbeTarget;

    /// egui only reports a drag once the pointer has passed `max_click_dist`,
    /// so a grab radius must still cover the control from that far away.
    /// Dragging the outer rectangle resizes it: a side moves only its own edge,
    /// a corner moves the two that meet there, and the rest stays put.
    #[test]
    fn domain_drags_move_one_side_or_one_corner() {
        let start = DomainRect::new(-1.0, 1.0, -1.0, 1.0);
        for (side, expected) in [
            (OuterSide::Left, DomainRect::new(-0.5, 1.0, -1.0, 1.0)),
            (OuterSide::Right, DomainRect::new(-1.0, -0.5, -1.0, 1.0)),
            (OuterSide::Bottom, DomainRect::new(-1.0, 1.0, -0.5, 1.0)),
            (OuterSide::Top, DomainRect::new(-1.0, 1.0, -1.0, -0.5)),
        ] {
            assert_eq!(
                Playground::resize_domain(
                    start,
                    DomainDrag::Side { side, start },
                    Point2::new(-0.5, -0.5),
                ),
                expected,
                "{side:?} moved the wrong edge"
            );
        }
        for (index, expected) in [
            (0usize, DomainRect::new(0.5, 1.0, 0.25, 1.0)),
            (1, DomainRect::new(-1.0, 0.5, 0.25, 1.0)),
            (2, DomainRect::new(-1.0, 0.5, -1.0, 0.25)),
            (3, DomainRect::new(0.5, 1.0, -1.0, 0.25)),
        ] {
            assert_eq!(
                Playground::resize_domain(
                    start,
                    DomainDrag::Corner { index, start },
                    Point2::new(0.5, 0.25),
                ),
                expected,
                "corner {index} moved the wrong pair"
            );
        }
    }

    /// The corners are grabbable at the same radius as any other handle, and
    /// each offers the diagonal cursor that matches it.
    #[test]
    fn domain_corners_are_grabbable_and_cursored() {
        let state = Playground::default();
        let domain = state.editor.document.model.draft.geometry.domain;
        for (index, corner) in domain.corners().into_iter().enumerate() {
            let centre = state.screen(corner, viewport());
            assert_eq!(
                state.hit_domain_corner(centre, viewport()),
                Some(index),
                "corner {index} is not grabbable at its own centre"
            );
            assert_eq!(
                state.hit_domain_corner(centre + egui::vec2(60.0, 60.0), viewport()),
                None,
                "corner {index} grabs far too wide"
            );
        }
        assert_eq!(
            Playground::domain_corner_cursor(0),
            egui::CursorIcon::ResizeNeSw
        );
        assert_eq!(
            Playground::domain_corner_cursor(1),
            egui::CursorIcon::ResizeNwSe
        );
    }

    #[test]
    fn grab_radii_absorb_the_drag_threshold() {
        let mut state = Playground::default();
        state
            .editor
            .create_probe(
                "Spot".into(),
                [91, 220, 194],
                TopologyProbeTarget::Point(Point2::new(0.0, 0.0)),
            )
            .unwrap();
        let id = state
            .editor
            .document
            .model
            .probes
            .iter()
            .find(|probe| probe.name == "Spot")
            .unwrap()
            .id;
        let centre = state.screen(Point2::new(0.0, 0.0), viewport());
        let threshold = egui::InputOptions::default().max_click_dist;
        // The drawn marker reaches 9 px with its selected ring.
        let visual = 9.0;
        assert!(
            state.hit_tolerance(13.0) >= visual + threshold * 0.5,
            "grab radius must exceed the drawn control plus half the drag threshold"
        );
        for offset in [0.0, 6.0, 12.0] {
            assert_eq!(
                state.hit_probe(centre + egui::vec2(offset, 0.0), viewport()),
                Some(ProbeHit::Point(id)),
                "probe lost {offset} px from its centre"
            );
        }
        assert_eq!(
            state.hit_probe(centre + egui::vec2(20.0, 0.0), viewport()),
            None
        );
    }

    /// A subdomain marker is placed from the compiled geometry, so it exists
    /// before any mesh does and sits where the region's area is.
    #[test]
    fn a_subdomain_marker_is_placed_from_the_geometry() {
        let mut state = Playground::default();
        let probe = state
            .editor
            .create_probe(
                "Field".into(),
                [91, 220, 194],
                TopologyProbeTarget::AreaRegion(RegionId(1)),
            )
            .unwrap();
        assert!(
            state.runtime.active().is_none(),
            "the scene has not been meshed"
        );
        state.refresh_probe_metadata();
        let anchor = state
            .probe_anchors
            .get(&probe)
            .copied()
            .expect("the marker is placed without a mesh");

        let scene = &state.editor.compiled_accepted;
        let hole = scene
            .assignments
            .iter()
            .find(|assignment| assignment.region.is_none())
            .and_then(|assignment| scene.topology.face(assignment.face))
            .and_then(|face| face.centroid())
            .expect("the default scene holds one excluded face");
        let center = scene.geometry.domain.center();
        // Taking area out of a shape moves its centroid away from where that
        // area was, and no further than the area which left could carry it.
        // Averaging triangle centroids moved it the other way, towards the
        // dense mesh the hole's boundary asks for.
        let moved = anchor - center;
        let away = center - hole;
        assert!(moved.norm() > 0.0, "the hole moved the marker");
        assert!(
            moved.dot(away) / (moved.norm() * away.norm()) > 0.999,
            "the marker moved {moved:?}, away from the hole is {away:?}"
        );
        assert!(moved.norm() < away.norm(), "the marker left the region");
    }

    /// The marker is the region's, not the mesh's, so meshing the same scene
    /// twice at different densities has to leave it exactly where it was.
    #[test]
    fn a_subdomain_marker_never_moves_with_the_mesh() {
        let mut state = Playground::default();
        let probe = state
            .editor
            .create_probe(
                "Field".into(),
                [91, 220, 194],
                TopologyProbeTarget::AreaRegion(RegionId(1)),
            )
            .unwrap();
        let mut placed = Vec::new();
        for target in [0.30, 0.09] {
            let active = activate_at(&mut state, target);
            let triangles = active.mesh.triangles.len();
            state.refresh_probe_metadata();
            let anchor = state
                .probe_anchors
                .get(&probe)
                .copied()
                .expect("the marker is placed");
            placed.push((triangles, anchor));
        }
        let [(coarse, first), (fine, second)] = placed[..] else {
            unreachable!()
        };
        assert!(fine > coarse * 4, "the two meshes differ: {coarse} {fine}");
        assert_eq!(
            first, second,
            "the marker moved between a {coarse} and a {fine} triangle mesh"
        );
    }
}
