//! The transform gizmo and the cursors: what the pointer is over, what a
//! click would do, and the ring and grips that say it.

use bevy_egui::egui::{self, Color32, Pos2, Rect, Stroke};
use funfern_app::topology_viewport::{
    RigidTransform, ScreenPoint, TopologyHit, TopologySpanTarget, plan_rigid_transform,
};
use funfern_core::*;
use std::collections::BTreeSet;

use super::*;

impl Playground {
    pub(super) fn transform_gizmo(&self, r: Rect) -> Option<(Point2, Pos2, f32, f32, f32)> {
        if self.draw.is_some() || self.pulse_mode || self.probe_mode.is_some() {
            return None;
        }
        let selected = self.selection.spans()?;
        if selected.is_empty()
            || selected
                .iter()
                .any(|target| matches!(target, TopologySpanTarget::Outer(_)))
        {
            return None;
        }
        let spans = selected
            .iter()
            .filter_map(|target| match target {
                TopologySpanTarget::Curve(span) => Some(*span),
                TopologySpanTarget::Outer(_) => None,
            })
            .collect::<BTreeSet<_>>();
        let pivot = self.gizmo_pivot_for(selected, &spans)?;
        let updates = plan_rigid_transform(
            &self.editor.document.model.draft.geometry,
            selected,
            RigidTransform {
                pivot,
                translation: Point2::default(),
                rotation_radians: 0.0,
                scale: 1.0,
            },
        )
        .ok()?;
        let center = self.screen(pivot, r);
        let radius = updates
            .iter()
            .map(|update| match update {
                funfern_app::topology_viewport::TopologyTransformUpdate::Control {
                    point, ..
                }
                | funfern_app::topology_viewport::TopologyTransformUpdate::Vertex {
                    point, ..
                } => center.distance(self.screen(*point, r)),
            })
            .fold(22.0_f32, f32::max)
            + GIZMO_PADDING;
        let x_radius = updates
            .iter()
            .map(|update| match update {
                funfern_app::topology_viewport::TopologyTransformUpdate::Control {
                    point, ..
                }
                | funfern_app::topology_viewport::TopologyTransformUpdate::Vertex {
                    point, ..
                } => (self.screen(*point, r).x - center.x).abs(),
            })
            .fold(22.0_f32, f32::max)
            + GIZMO_PADDING;
        let y_radius = updates
            .iter()
            .map(|update| match update {
                funfern_app::topology_viewport::TopologyTransformUpdate::Control {
                    point, ..
                }
                | funfern_app::topology_viewport::TopologyTransformUpdate::Vertex {
                    point, ..
                } => (self.screen(*point, r).y - center.y).abs(),
            })
            .fold(22.0_f32, f32::max)
            + GIZMO_PADDING;
        Some((pivot, center, radius, x_radius, y_radius))
    }
    pub(super) fn draw_transform_gizmo(&self, painter: &egui::Painter, r: Rect) {
        if self.capturing() {
            return;
        }
        let Some((_, center, radius, x_radius, y_radius)) = self.transform_gizmo(r) else {
            return;
        };
        painter.circle_stroke(
            center,
            radius,
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(72, 166, 255, 150)),
        );
        painter.line_segment(
            [center, center + egui::vec2(x_radius, 0.0)],
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(72, 166, 255, 110)),
        );
        painter.line_segment(
            [center, center + egui::vec2(0.0, -y_radius)],
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(72, 166, 255, 110)),
        );
        painter.circle_filled(center, 6.0, Color32::from_rgb(16, 23, 31));
        painter.circle_stroke(center, 6.0, Stroke::new(2.0, SELECT));
        painter.line_segment(
            [
                center - egui::vec2(11.0, 0.0),
                center + egui::vec2(11.0, 0.0),
            ],
            Stroke::new(1.5, Color32::WHITE),
        );
        painter.line_segment(
            [
                center - egui::vec2(0.0, 11.0),
                center + egui::vec2(0.0, 11.0),
            ],
            Stroke::new(1.5, Color32::WHITE),
        );
        let diagonal = radius * std::f32::consts::FRAC_1_SQRT_2;
        painter.rect_filled(
            Rect::from_center_size(
                center + egui::vec2(diagonal, diagonal),
                egui::vec2(9.0, 9.0),
            ),
            1.0,
            SELECT,
        );
        painter.rect_filled(
            Rect::from_center_size(center + egui::vec2(x_radius, 0.0), egui::vec2(7.0, 11.0)),
            1.0,
            SELECT,
        );
        painter.rect_filled(
            Rect::from_center_size(center + egui::vec2(0.0, -y_radius), egui::vec2(11.0, 7.0)),
            1.0,
            SELECT,
        );
    }
    pub(super) fn scale_axis_cursor(axis: GizmoScaleAxis) -> egui::CursorIcon {
        match axis {
            GizmoScaleAxis::Uniform => egui::CursorIcon::ResizeNwSe,
            GizmoScaleAxis::X => egui::CursorIcon::ResizeHorizontal,
            GizmoScaleAxis::Y => egui::CursorIcon::ResizeVertical,
        }
    }

    pub(super) fn outer_side_cursor(side: OuterSide) -> egui::CursorIcon {
        match side {
            OuterSide::Left | OuterSide::Right => egui::CursorIcon::ResizeHorizontal,
            OuterSide::Bottom | OuterSide::Top => egui::CursorIcon::ResizeVertical,
        }
    }

    /// A mode that owns the whole viewport says so with the cursor, since there
    /// is no handle anywhere to carry the meaning.
    pub(super) fn modal_cursor(&self, pos: Pos2, r: Rect) -> Option<egui::CursorIcon> {
        if let Some(pending) = &self.pending_merge {
            // Only the candidates are clickable; everywhere else the click does
            // nothing and the cursor should not promise otherwise.
            return self
                .draft_region_at(self.world(pos, r))
                .filter(|region| pending.choices.contains(region))
                .map(|_| egui::CursorIcon::PointingHand);
        }
        (self.draw.is_some() || self.pulse_mode || self.probe_mode.is_some())
            .then_some(egui::CursorIcon::Crosshair)
    }

    /// The cursor a live gesture holds on to, so what appeared under the pointer
    /// does not vanish the moment the drag begins.
    pub(super) fn drag_cursor(&self) -> Option<egui::CursorIcon> {
        Some(match self.drag.as_ref()? {
            DragGesture::Pivot { .. } | DragGesture::Spans { .. } => egui::CursorIcon::Move,
            DragGesture::Rotate { .. } => egui::CursorIcon::Grabbing,
            DragGesture::Scale { axis, .. } => Self::scale_axis_cursor(*axis),
            DragGesture::Domain {
                drag: DomainDrag::Corner { index, .. },
            } => Self::domain_corner_cursor(*index),
            DragGesture::Domain {
                drag: DomainDrag::Side { side, .. },
            } => Self::outer_side_cursor(*side),
            DragGesture::MaterialFrame { hit, .. } => match hit {
                MaterialFrameGizmoHit::Origin => egui::CursorIcon::Move,
                MaterialFrameGizmoHit::Rotate => egui::CursorIcon::Grabbing,
            },
            DragGesture::Probe { hit, .. } => match hit {
                ProbeHit::AreaDiskRadius(_) => egui::CursorIcon::ResizeHorizontal,
                ProbeHit::AreaDiskBody(_) | ProbeHit::SegmentBody(_) => egui::CursorIcon::Move,
                _ => return None,
            },
            _ => return None,
        })
    }

    /// What the pointer is over while editing. Only an affordance the drawing
    /// does not already announce, or one whose direction matters, earns a
    /// cursor: a drawn handle that moves itself is its own announcement. The
    /// order matches the one the press handler resolves grabs in.
    pub(super) fn hover_cursor(&self, pos: Pos2, r: Rect) -> Option<egui::CursorIcon> {
        if self.selected_material_frame().is_some()
            && let Some(hit) = self.hit_material_frame_gizmo(pos, r)
        {
            return Some(match hit {
                MaterialFrameGizmoHit::Origin => egui::CursorIcon::Move,
                MaterialFrameGizmoHit::Rotate => egui::CursorIcon::Grab,
            });
        }
        if let Some((hit, _)) = self.hit_transform_gizmo(pos, r) {
            return Some(match hit {
                TransformGizmoHit::Pivot => egui::CursorIcon::Move,
                TransformGizmoHit::Rotate => egui::CursorIcon::Grab,
                TransformGizmoHit::Scale(axis) => Self::scale_axis_cursor(axis),
            });
        }
        if let Some(hit) = self.hit_probe(pos, r) {
            return match hit {
                ProbeHit::AreaDiskRadius(_) => Some(egui::CursorIcon::ResizeHorizontal),
                ProbeHit::AreaDiskBody(_) | ProbeHit::SegmentBody(_) => {
                    Some(egui::CursorIcon::Move)
                }
                _ => None,
            };
        }
        if let Some(index) = self.hit_domain_corner(pos, r) {
            return Some(Self::domain_corner_cursor(index));
        }
        let hit = self.sampled.as_ref().and_then(|sampled| {
            sampled.hit_test(
                self.transform(r),
                ScreenPoint::new(pos.x as f64, pos.y as f64),
                self.hit_tolerance(13.0) as f64,
                self.hit_tolerance(9.0) as f64,
            )
        })?;
        let TopologyHit::Span { target, .. } = hit else {
            return None;
        };
        if let TopologySpanTarget::Outer(side) = target {
            return Some(Self::outer_side_cursor(side));
        }
        // Pressing a span of the selection drags the whole of it, but only when
        // that selection can move rigidly. The gizmo answers exactly that, so
        // its absence is what tells the user to widen the selection.
        let selected = self
            .selection
            .spans()
            .is_some_and(|spans| spans.contains(&target));
        (selected && self.transform_gizmo(r).is_some()).then_some(egui::CursorIcon::Move)
    }

    pub(super) fn hit_transform_gizmo(
        &self,
        point: Pos2,
        r: Rect,
    ) -> Option<(TransformGizmoHit, Point2)> {
        let (pivot, center, radius, x_radius, y_radius) = self.transform_gizmo(r)?;
        if center.distance(point) <= 12.0 {
            return Some((TransformGizmoHit::Pivot, pivot));
        }
        let diagonal = radius * std::f32::consts::FRAC_1_SQRT_2;
        for (axis, offset) in [
            (GizmoScaleAxis::Uniform, egui::vec2(diagonal, diagonal)),
            (GizmoScaleAxis::X, egui::vec2(x_radius, 0.0)),
            (GizmoScaleAxis::Y, egui::vec2(0.0, -y_radius)),
        ] {
            if (center + offset).distance(point) <= 10.0 {
                return Some((TransformGizmoHit::Scale(axis), pivot));
            }
        }
        ((center.distance(point) - radius).abs() <= 7.0)
            .then_some((TransformGizmoHit::Rotate, pivot))
    }
}
