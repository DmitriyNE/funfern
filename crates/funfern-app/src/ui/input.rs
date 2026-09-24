//! Viewport pointer and keyboard handling: one pass over the frame's input
//! that decides what the gesture in progress does.

use bevy::prelude::*;
use bevy_egui::egui::{self, Rect};
use funfern_app::topology_viewport::{
    RigidTransform, ScreenPoint, TopologyHandle, TopologyHit, TopologySelection,
    TopologySpanTarget, plan_axis_scale, plan_handle_drag, plan_rigid_transform,
};
use funfern_core::*;
use std::collections::BTreeSet;

use super::*;

impl Playground {
    pub(super) fn handle_viewport_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        r: Rect,
    ) {
        let pointer = response.interact_pointer_pos();
        // egui reports `drag_started` only once the pointer has travelled past
        // `max_click_dist`, so by then the live position has already left
        // whatever the user aimed at. Every grab test uses the press origin.
        let press = ui.input(|input| input.pointer.press_origin()).or(pointer);
        let typing = ui.ctx().egui_wants_keyboard_input();
        if !typing && ui.input(|i| i.modifiers.is_none() && i.key_pressed(egui::Key::S)) {
            self.request_material_switch();
        }
        let touch_active = ui.input(|input| input.any_touches());
        self.touch_active = touch_active;
        let multi_touch = ui.input(|input| input.multi_touch());
        if !touch_active {
            // Hover, not interaction: `interact_pointer_pos` is `None` until a
            // gesture is already under way, which is precisely when a cursor has
            // nothing left to announce.
            let hovering = response.hover_pos();
            let cursor = hovering
                .and_then(|pos| self.modal_cursor(pos, r))
                .or_else(|| self.drag_cursor())
                .or_else(|| hovering.and_then(|pos| self.hover_cursor(pos, r)));
            if let Some(cursor) = cursor {
                ui.ctx().set_cursor_icon(cursor);
            }
        }
        if let Some(gesture) = multi_touch
            && (self.touch_navigation || r.contains(gesture.center_pos))
        {
            if !self.touch_navigation {
                self.cancel_interaction();
                self.touch_navigation = true;
                self.suppress_touch_click = true;
            }
            self.center = self.center
                + Point2::new(
                    -gesture.translation_delta.x as f64 / self.scale,
                    gesture.translation_delta.y as f64 / self.scale,
                );
            let before = self.world(gesture.center_pos, r);
            self.scale = (self.scale * gesture.zoom_delta as f64).clamp(20.0, 5000.0);
            let after = self.world(gesture.center_pos, r);
            self.center = self.center + before - after;
            self.invalidate_samples();
            return;
        }
        if self.touch_navigation {
            if !touch_active {
                self.touch_navigation = false;
            }
            return;
        }
        if self.suppress_touch_click {
            if !touch_active {
                self.suppress_touch_click = false;
            }
            return;
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                    let before = self.world(pos, r);
                    self.scale = (self.scale * (scroll as f64 * 0.0015).exp()).clamp(20.0, 5000.0);
                    let after = self.world(pos, r);
                    self.center = self.center + (before - after);
                    self.invalidate_samples();
                }
            }
        }
        if response.dragged_by(egui::PointerButton::Secondary) {
            let delta = response.drag_delta();
            self.center = self.center - Point2::new(delta.x as f64, -delta.y as f64) / self.scale;
            self.invalidate_samples();
        }
        // The first Escape leaves the text field, which egui has already done by
        // now; only a second one reaches the viewport.
        if !typing
            && !self.keyboard_focus_previous
            && ui.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.cancel_interaction();
            return;
        }
        if self.pending_merge.is_some() {
            // The survivor question owns the viewport until it is answered or
            // cancelled: a click picks, everything else waits.
            if response.clicked_by(egui::PointerButton::Primary)
                && let Some(pos) = pointer
            {
                self.pick_merge_survivor(self.world(pos, r));
            }
            return;
        }
        if response.double_clicked()
            && let Some(pos) = pointer
        {
            // The same lookup a single click uses, so a double click honours the
            // View visibility toggles and opens the probe on top rather than the
            // one underneath.
            if let Some(hit) = self.hit_probe(pos, r) {
                self.probe_windows.insert(hit.id());
                return;
            }
            if self.editor.document.model.far_field.enabled {
                let domain = self.editor.document.model.draft.geometry.domain;
                let inset = self.editor.document.model.far_field.inset;
                let min = self.screen(Point2::new(domain.min_x + inset, domain.max_y - inset), r);
                let max = self.screen(Point2::new(domain.max_x - inset, domain.min_y + inset), r);
                let contour = Rect::from_min_max(min, max);
                let distance = [
                    (pos.x - contour.left()).abs(),
                    (pos.x - contour.right()).abs(),
                    (pos.y - contour.top()).abs(),
                    (pos.y - contour.bottom()).abs(),
                ]
                .into_iter()
                .fold(f32::INFINITY, f32::min);
                if contour.expand(9.0).contains(pos) && distance <= 9.0 {
                    self.far_field_window = true;
                    return;
                }
            }
            if let Some((curve, parameter)) = self
                .sampled
                .as_ref()
                .and_then(|sampled| closest_curve_parameter(sampled, self.transform(r), pos, 8.0))
            {
                match self.editor.insert_control(curve, parameter) {
                    Ok((control, _)) => {
                        self.selection =
                            TopologySelection::Handle(TopologyHandle::Control { curve, control });
                        self.invalidate_samples();
                        self.notify("Control inserted without changing the curve");
                    }
                    Err(error) => self.notify(error),
                }
                return;
            }
        }
        if let Some(draw) = self.draw.as_ref().map(|d| d.tool) {
            if response.clicked_by(egui::PointerButton::Primary) {
                if let Some(pos) = pointer {
                    self.draw_click(
                        self.world(pos, r),
                        ScreenPoint::new(pos.x as f64, pos.y as f64),
                        r,
                        ui.input(|input| input.modifiers.shift),
                    );
                }
            }
            if !typing && ui.input(|i| i.key_pressed(egui::Key::Backspace)) {
                if let Some(draw) = &mut self.draw {
                    draw.points.pop();
                    draw.attachments.pop();
                }
            }
            if !typing && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.finish_draw();
            }
            let _ = draw;
            return;
        }
        if self.pulse_mode && response.clicked() {
            if let Some(pos) = pointer {
                self.place_pulse(self.world(pos, r));
            }
            return;
        }
        if self.probe_mode.is_some() && response.clicked() {
            if let Some(pos) = pointer {
                self.probe_placement_click(
                    self.world(pos, r),
                    ui.input(|input| input.modifiers.shift),
                );
            }
            return;
        }
        if response.drag_started_by(egui::PointerButton::Primary) {
            if let (Some(pos), Some(grab)) = (pointer, press) {
                if let Some(hit) = self.hit_material_frame_gizmo(grab, r)
                    && let Some((region, start)) = self.selected_material_frame()
                {
                    let world = self.world(grab, r);
                    self.editor.begin();
                    self.drag = Some(DragGesture::MaterialFrame {
                        region,
                        start,
                        hit,
                        grab: match hit {
                            MaterialFrameGizmoHit::Origin => 0.0,
                            MaterialFrameGizmoHit::Rotate => {
                                let relative = world - start.origin;
                                relative.y.atan2(relative.x)
                            }
                        },
                    });
                    return;
                }
                if let Some((hit, pivot)) = self.hit_transform_gizmo(grab, r) {
                    let relative = self.world(pos, r) - pivot;
                    self.drag = Some(match hit {
                        TransformGizmoHit::Pivot => DragGesture::Pivot {
                            previous: self.gizmo_pivot.clone(),
                            offset: pivot - self.world(pos, r),
                        },
                        TransformGizmoHit::Rotate => {
                            self.editor.begin();
                            DragGesture::Rotate {
                                pivot,
                                start_angle: relative.y.atan2(relative.x),
                                geometry: self.editor.document.model.draft.geometry.clone(),
                            }
                        }
                        TransformGizmoHit::Scale(axis) => {
                            self.editor.begin();
                            DragGesture::Scale {
                                axis,
                                pivot,
                                start_distance: (Self::scale_drag_distance(axis, relative)
                                    - GIZMO_PADDING as f64 / self.scale)
                                    .max(1.0e-12),
                                geometry: self.editor.document.model.draft.geometry.clone(),
                            }
                        }
                    });
                    return;
                }
                let screen = ScreenPoint::new(grab.x as f64, grab.y as f64);
                let source = self.editor.document.model.source;
                if source.enabled
                    && self.screen(source.position, r).distance(grab) <= self.hit_tolerance(13.0)
                {
                    self.editor.begin();
                    self.drag = Some(DragGesture::Source);
                    return;
                }
                if let Some(hit) = self.hit_probe(grab, r) {
                    self.selected_probe = Some(hit.id());
                    self.selection = TopologySelection::None;
                    if hit.draggable()
                        && let Some(original) = self
                            .editor
                            .document
                            .model
                            .probes
                            .iter()
                            .find(|probe| probe.id == hit.id())
                            .map(|probe| probe.target.clone())
                    {
                        self.editor.begin();
                        self.drag = Some(DragGesture::Probe {
                            hit,
                            grab: self.world(pos, r),
                            original,
                        });
                    }
                    return;
                }
                // A corner of the outer rectangle outranks the two sides that
                // meet there, so both a corner and a side drag stay reachable.
                if let Some(index) = self.hit_domain_corner(grab, r) {
                    self.selected_probe = None;
                    self.selection = TopologySelection::Spans(
                        [
                            TopologySpanTarget::Outer(OuterSide::ALL[index]),
                            TopologySpanTarget::Outer(
                                OuterSide::ALL
                                    [(index + OuterSide::ALL.len() - 1) % OuterSide::ALL.len()],
                            ),
                        ]
                        .into_iter()
                        .collect(),
                    );
                    self.editor.begin();
                    self.drag = Some(DragGesture::Domain {
                        drag: DomainDrag::Corner {
                            index,
                            start: self.editor.document.model.draft.geometry.domain,
                        },
                    });
                    return;
                }
                let hit = self.sampled.as_ref().and_then(|sampled| {
                    sampled.hit_test(
                        self.transform(r),
                        screen,
                        self.hit_tolerance(13.0) as f64,
                        self.hit_tolerance(9.0) as f64,
                    )
                });
                if let Some(hit) = hit {
                    self.selected_probe = None;
                    let shift = ui.input(|i| i.modifiers.shift);
                    if !Self::drag_starts_inside_span_selection(&self.selection, hit) {
                        self.selection.apply_hit(
                            &self.editor.document.model.draft.geometry,
                            hit,
                            shift,
                            ui.input(|i| i.modifiers.command),
                        );
                    }
                    self.editor.begin();
                    let loose_end = match hit {
                        TopologyHit::Handle {
                            handle: TopologyHandle::Control { curve, control },
                            ..
                        } => self
                            .loose_end_of_control(curve, control)
                            .map(|node| (curve, node)),
                        _ => None,
                    };
                    let outer_side = match hit {
                        TopologyHit::Span {
                            target: TopologySpanTarget::Outer(side),
                            ..
                        } if !shift && !ui.input(|i| i.modifiers.command) => Some(side),
                        _ => None,
                    };
                    self.drag = Some(if let Some(side) = outer_side {
                        DragGesture::Domain {
                            drag: DomainDrag::Side {
                                side,
                                start: self.editor.document.model.draft.geometry.domain,
                            },
                        }
                    } else if let Some((curve, node)) = loose_end {
                        DragGesture::Endpoint {
                            curve,
                            node,
                            snap: None,
                        }
                    } else {
                        match hit {
                            TopologyHit::Handle { handle, .. } => DragGesture::Handle { handle },
                            TopologyHit::Span { .. } => {
                                let selected = self.selection.spans().cloned().unwrap_or_default();
                                let curve_spans = selected
                                    .iter()
                                    .filter_map(|target| match target {
                                        TopologySpanTarget::Curve(span) => Some(*span),
                                        TopologySpanTarget::Outer(_) => None,
                                    })
                                    .collect::<BTreeSet<_>>();
                                let custom_pivot = self
                                    .gizmo_pivot
                                    .as_ref()
                                    .is_some_and(|(selection, _)| selection == &selected);
                                DragGesture::Spans {
                                    start: self.world(pos, r),
                                    pivot: self
                                        .gizmo_pivot_for(&selected, &curve_spans)
                                        .unwrap_or_default(),
                                    custom_pivot,
                                    gizmo_before: self.gizmo_pivot.clone(),
                                    geometry: self.editor.document.model.draft.geometry.clone(),
                                }
                            }
                        }
                    });
                } else {
                    let base = self.selection.spans().cloned().unwrap_or_default();
                    self.drag = Some(DragGesture::Marquee {
                        start: pos,
                        current: pos,
                        base,
                        operation: Self::marquee_operation(ui.input(|input| input.modifiers)),
                    });
                }
            }
        }
        if response.dragged_by(egui::PointerButton::Primary) {
            if let (Some(pos), Some(drag)) = (pointer, self.drag.clone()) {
                let point = self.world(pos, r);
                let shift = ui.input(|input| input.modifiers.shift);
                let invalidates_geometry = !matches!(
                    &drag,
                    DragGesture::Marquee { .. } | DragGesture::Pivot { .. }
                );
                let step = self.snap_step();
                let result = match drag {
                    DragGesture::Endpoint { curve, node, .. } => {
                        let point = if shift {
                            Self::snap_point(point, step)
                        } else {
                            point
                        };
                        let moved = self
                            .endpoint_control(curve, node)
                            .ok_or_else(|| "Curve end no longer exists".to_owned())
                            .and_then(|control| {
                                plan_handle_drag(
                                    &self.editor.document.model.draft.geometry,
                                    TopologyHandle::Control { curve, control },
                                    point,
                                )
                                .map_err(|e| e.to_string())
                            })
                            .and_then(|update| {
                                self.editor.apply_transform_updates_during_edit(&[update])
                            });
                        let snap = self.weld_hit_for(pos, r, Some((curve, node)));
                        self.drag = Some(DragGesture::Endpoint { curve, node, snap });
                        moved
                    }
                    DragGesture::Domain { drag } => {
                        let point = if shift {
                            Self::snap_point(point, step)
                        } else {
                            point
                        };
                        let start = match drag {
                            DomainDrag::Side { start, .. } | DomainDrag::Corner { start, .. } => {
                                start
                            }
                        };
                        self.editor
                            .set_domain_during_edit(Self::resize_domain(start, drag, point))
                    }
                    DragGesture::Handle { handle } => {
                        let point = if shift {
                            Self::snap_point(point, step)
                        } else {
                            point
                        };
                        plan_handle_drag(&self.editor.document.model.draft.geometry, handle, point)
                            .map_err(|e| e.to_string())
                            .and_then(|update| {
                                self.editor.apply_transform_updates_during_edit(&[update])
                            })
                    }
                    DragGesture::Spans {
                        start,
                        pivot,
                        custom_pivot,
                        gizmo_before: _,
                        geometry,
                    } => {
                        let selected = self.selection.spans().cloned().unwrap_or_default();
                        let target = if shift {
                            Self::snap_point(pivot + point - start, step)
                        } else {
                            pivot + point - start
                        };
                        let translation = target - pivot;
                        plan_rigid_transform(
                            &geometry,
                            &selected,
                            RigidTransform {
                                pivot: Point2::default(),
                                translation,
                                rotation_radians: 0.0,
                                scale: 1.0,
                            },
                        )
                        .map_err(|e| e.to_string())
                        .and_then(|updates| {
                            self.editor.document.model.draft.geometry = geometry;
                            self.editor.apply_transform_updates_during_edit(&updates)?;
                            if custom_pivot {
                                self.gizmo_pivot = Some((selected, pivot + translation));
                            }
                            Ok(())
                        })
                    }
                    DragGesture::Rotate {
                        pivot,
                        start_angle,
                        geometry,
                    } => {
                        let selected = self.selection.spans().cloned().unwrap_or_default();
                        let relative = point - pivot;
                        let mut angle = relative.y.atan2(relative.x) - start_angle;
                        if shift {
                            let step = 15.0_f64.to_radians();
                            angle = (angle / step).round() * step;
                        }
                        plan_rigid_transform(
                            &geometry,
                            &selected,
                            RigidTransform {
                                pivot,
                                translation: Point2::default(),
                                rotation_radians: angle,
                                scale: 1.0,
                            },
                        )
                        .map_err(|e| e.to_string())
                        .and_then(|updates| {
                            self.editor.document.model.draft.geometry = geometry;
                            self.editor.apply_transform_updates_during_edit(&updates)
                        })
                    }
                    DragGesture::Scale {
                        axis,
                        pivot,
                        start_distance,
                        geometry,
                    } => {
                        let selected = self.selection.spans().cloned().unwrap_or_default();
                        let distance = (Self::scale_drag_distance(axis, point - pivot)
                            - GIZMO_PADDING as f64 / self.scale)
                            .max(0.0);
                        let mut factor = (distance / start_distance).max(0.01);
                        if shift {
                            factor = ((factor * 10.0).round() / 10.0).max(0.1);
                        }
                        plan_axis_scale(
                            &geometry,
                            &selected,
                            pivot,
                            if axis == GizmoScaleAxis::Y {
                                1.0
                            } else {
                                factor
                            },
                            if axis == GizmoScaleAxis::X {
                                1.0
                            } else {
                                factor
                            },
                        )
                        .map_err(|e| e.to_string())
                        .and_then(|updates| {
                            self.editor.document.model.draft.geometry = geometry;
                            self.editor.apply_transform_updates_during_edit(&updates)
                        })
                    }
                    DragGesture::Pivot { offset, .. } => {
                        let selected = self.selection.spans().cloned().unwrap_or_default();
                        let target = if shift {
                            Self::snap_point(point + offset, step)
                        } else {
                            point + offset
                        };
                        self.gizmo_pivot = Some((selected, target));
                        Ok(())
                    }
                    DragGesture::Marquee { start, base, .. } => {
                        let operation = Self::marquee_operation(ui.input(|input| input.modifiers));
                        let hits = self
                            .sampled
                            .as_ref()
                            .map(|sampled| {
                                sampled.marquee_hits(
                                    self.transform(r),
                                    ScreenPoint::new(start.x as f64, start.y as f64),
                                    ScreenPoint::new(pos.x as f64, pos.y as f64),
                                )
                            })
                            .unwrap_or_default();
                        self.selection = Self::selection_from_spans(Self::marquee_result(
                            &base, hits, operation,
                        ));
                        self.drag = Some(DragGesture::Marquee {
                            start,
                            current: pos,
                            base,
                            operation,
                        });
                        Ok(())
                    }
                    DragGesture::MaterialFrame {
                        region,
                        start,
                        hit,
                        grab,
                    } => {
                        let mut frame = start;
                        match hit {
                            MaterialFrameGizmoHit::Origin => {
                                frame.origin = if shift {
                                    Self::snap_point(point, step)
                                } else {
                                    point
                                };
                            }
                            MaterialFrameGizmoHit::Rotate => {
                                let relative = point - start.origin;
                                let mut angle =
                                    start.angle_radians + relative.y.atan2(relative.x) - grab;
                                if shift {
                                    let step = 15.0_f64.to_radians();
                                    angle = (angle / step).round() * step;
                                }
                                frame.angle_radians = angle;
                            }
                        }
                        self.editor.set_region_frame_during_edit(region, frame)
                    }
                    DragGesture::Source => {
                        let mut source = self.editor.document.model.source;
                        source.position = point;
                        if let Some(active) = self.runtime.active()
                            && let Some(face) = active.bundle.snapshot.face_at(point)
                            && let Some(region) = active
                                .bundle
                                .plan
                                .domains
                                .iter()
                                .find(|domain| domain.face == face)
                        {
                            source.region = region.region;
                        }
                        self.editor.set_point_source_during_edit(source)
                    }
                    DragGesture::Probe {
                        hit,
                        grab,
                        ref original,
                    } => {
                        // Snap what the drag moves, not the pointer: the grab
                        // offset would otherwise leave the probe off the grid by
                        // however far inside itself it was picked up.
                        self.drag_probe(hit, original, point - grab, shift);
                        Ok(())
                    }
                };
                if let Err(error) = result {
                    self.message = error;
                }
                if invalidates_geometry {
                    self.invalidate_samples();
                }
            }
        }
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            if let Some(drag) = self.drag.take() {
                match drag {
                    DragGesture::Marquee {
                        start,
                        current,
                        base,
                        ..
                    } => {
                        let release_modifiers = ui.input(|input| {
                            input
                                .raw
                                .events
                                .iter()
                                .find_map(|event| match event {
                                    egui::Event::PointerButton {
                                        button: egui::PointerButton::Primary,
                                        pressed: false,
                                        modifiers,
                                        ..
                                    } => Some(*modifiers),
                                    _ => None,
                                })
                                .unwrap_or(input.modifiers)
                        });
                        let operation = Self::marquee_operation(release_modifiers);
                        let end = pointer.unwrap_or(current);
                        let hits = self
                            .sampled
                            .as_ref()
                            .map(|sampled| {
                                sampled.marquee_hits(
                                    self.transform(r),
                                    ScreenPoint::new(start.x as f64, start.y as f64),
                                    ScreenPoint::new(end.x as f64, end.y as f64),
                                )
                            })
                            .unwrap_or_default();
                        self.selection = Self::selection_from_spans(Self::marquee_result(
                            &base, hits, operation,
                        ));
                    }
                    DragGesture::Pivot { .. } => {}
                    DragGesture::Endpoint { curve, node, .. } => {
                        self.finish_endpoint_drag(curve, node, pointer, r);
                        // A no-op after a successful weld, which committed the
                        // drag and the weld together; the drag alone otherwise.
                        self.editor.commit();
                    }
                    _ => self.editor.commit(),
                }
            }
        }
        if response.clicked() && self.drag.is_none() {
            if let Some(pos) = pointer {
                if let Some(hit) = self.hit_probe(pos, r) {
                    self.selected_probe = Some(hit.id());
                    self.selection = TopologySelection::None;
                    return;
                }
                let hit = self.sampled.as_ref().and_then(|sampled| {
                    sampled.hit_test(
                        self.transform(r),
                        ScreenPoint::new(pos.x as f64, pos.y as f64),
                        self.hit_tolerance(13.0) as f64,
                        self.hit_tolerance(9.0) as f64,
                    )
                });
                if let Some(hit) = hit {
                    self.selected_probe = None;
                    self.selection.apply_hit(
                        &self.editor.document.model.draft.geometry,
                        hit,
                        ui.input(|i| i.modifiers.shift),
                        ui.input(|i| i.modifiers.command),
                    );
                } else {
                    self.selection = TopologySelection::None;
                    self.selected_probe = None;
                    let point = self.world(pos, r);
                    if self.subdomain_listing == SubdomainListing::Faces {
                        if let Some(index) = self.draft_face_assignment_at(point) {
                            self.face_selection = index;
                            if let Some(region) =
                                self.editor.document.model.draft.face_assignments[index].region
                            {
                                self.select_region(region);
                            }
                        }
                    } else if let Some(region) = self.region_at(point) {
                        self.select_region(region);
                    }
                }
            }
        }
        if !typing
            && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
        {
            self.delete_selection();
        }
    }
}
