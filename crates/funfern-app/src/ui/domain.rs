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
