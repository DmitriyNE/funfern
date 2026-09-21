//! The Edit inspector: what a selected handle, span or region offers, and
//! the prompts that resolve a removal or a merge.

use bevy::prelude::*;
use bevy_egui::egui::{self, Color32, Pos2, Rect, Stroke};
use funfern_app::topology_viewport::{
    RigidTransform, TopologyHandle, TopologySelection, TopologySpanTarget, plan_handle_drag,
    plan_rigid_transform, span_context,
};
use funfern_core::*;
use std::collections::BTreeSet;

use super::*;

impl Playground {
    pub(super) fn handle_inspector(&mut self, ui: &mut egui::Ui, handle: TopologyHandle) {
        let geometry = &self.editor.document.model.draft.geometry;
        let point = match handle {
            TopologyHandle::Control { curve, control } => geometry
                .curves
                .iter()
                .find(|item| item.id == curve)
                .and_then(|curve| match &curve.spline {
                    CurveSpline::Closed(s) => s.controls().get(control),
                    CurveSpline::Open(s) => s.controls().get(control),
                })
                .copied(),
            TopologyHandle::Junction(vertex) => geometry
                .vertices
                .iter()
                .find(|item| item.id == vertex)
                .and_then(|vertex| vertex.point(geometry.domain)),
        };
        let Some(mut point) = point else { return };
        ui.label(match handle {
            TopologyHandle::Control { curve, control } => {
                format!("Control {} · Curve {}", control + 1, curve.0)
            }
            TopologyHandle::Junction(vertex) => format!("Junction {}", vertex.0),
        });
        let before = point;
        ui.horizontal(|ui| {
            ui.label("x");
            ui.add(egui::DragValue::new(&mut point.x).speed(0.005));
            ui.label("y");
            ui.add(egui::DragValue::new(&mut point.y).speed(0.005));
        });
        if point != before {
            let result =
                plan_handle_drag(&self.editor.document.model.draft.geometry, handle, point)
                    .map_err(|e| e.to_string())
                    .and_then(|update| self.editor.apply_transform_updates(&[update]));
            match result {
                Ok(()) => self.invalidate_samples(),
                Err(error) => self.notify(error),
            }
        }
        if let TopologyHandle::Control { curve, control } = handle {
            // Ask the command itself whether it would succeed, so the button is
            // live exactly when the deletion is.
            let refusal = self.editor.control_removal_error(curve, control);
            let response = ui
                .add_enabled(refusal.is_none(), egui::Button::new("Delete control"))
                .on_disabled_hover_text(refusal.unwrap_or_default());
            if response.clicked() {
                match self.editor.remove_control(curve, control) {
                    Ok(()) => {
                        self.selection = TopologySelection::None;
                        self.invalidate_samples();
                    }
                    Err(error) => self.notify(error),
                }
            }
        }
        if let TopologyHandle::Junction(vertex) = handle {
            let endpoints = self
                .editor
                .document
                .model
                .draft
                .geometry
                .curves
                .iter()
                .filter_map(|curve| {
                    if !curve.spline.is_open() {
                        return None;
                    }
                    if curve
                        .nodes
                        .first()
                        .is_some_and(|node| node.vertex == Some(vertex))
                    {
                        Some((curve.id, 0))
                    } else if curve
                        .nodes
                        .last()
                        .is_some_and(|node| node.vertex == Some(vertex))
                    {
                        Some((curve.id, 1))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            for (curve, endpoint) in endpoints {
                if ui
                    .button(format!(
                        "Detach curve {} {}",
                        curve.0,
                        if endpoint == 0 { "start" } else { "end" }
                    ))
                    .clicked()
                {
                    match self.editor.detach_endpoint(curve, endpoint) {
                        Ok(()) => {
                            self.selection = TopologySelection::None;
                            self.invalidate_samples();
                        }
                        Err(error) => self.notify(error),
                    }
                }
            }
        }
    }
    /// Drops a staged survivor question when undo, a reload, or another edit
    /// moved the geometry out from under it, rather than act on something else.
    pub(super) fn prune_stale_pending_merge(&mut self) {
        if self.pending_merge.as_ref().is_some_and(|pending| {
            let geometry = &self.editor.document.model.draft.geometry;
            match &pending.action {
                MergeAction::Delete(spans) => !spans.iter().all(|span| {
                    geometry
                        .curves
                        .iter()
                        .any(|curve| curve.spans.iter().any(|candidate| candidate.id == *span))
                }),
                MergeAction::Weld { curve, .. } => geometry.curve(*curve).is_none(),
            }
        }) {
            self.pending_merge = None;
        }
    }
    /// The region owning the draft face under a world point, from the editor's
    /// own compile so the answer does not wait on the GPU runtime.
    pub(super) fn draft_region_at(&self, point: Point2) -> Option<RegionId> {
        let compiled = self
            .editor
            .compiled_draft
            .as_ref()
            .unwrap_or(&self.editor.compiled_accepted);
        let face = compiled.topology.face_at(point)?;
        compiled
            .assignments
            .iter()
            .find(|assignment| assignment.face == face)
            .and_then(|assignment| assignment.region)
    }
    pub(super) fn material_name(&self, region: RegionId) -> String {
        let draft = &self.editor.document.model.draft;
        draft
            .region(region)
            .and_then(|region| draft.material(region.material))
            .map_or_else(|| format!("Region {}", region.0), |m| m.name.clone())
    }
    /// Highlights every subdomain a staged deletion may keep: the mesh of each
    /// candidate filled gold, its boundary stroked, and its material named at
    /// the face centre. The one under the cursor reads stronger.
    pub(super) fn draw_removal_candidates(&self, painter: &egui::Painter, r: Rect) {
        if self.capturing() {
            return;
        }
        let Some(pending) = &self.pending_merge else {
            return;
        };
        let candidates = pending.choices.iter().copied().collect::<BTreeSet<_>>();
        let hovered = painter
            .ctx()
            .pointer_hover_pos()
            .filter(|pos| r.contains(*pos))
            .and_then(|pos| self.draft_region_at(self.world(pos, r)))
            .filter(|region| candidates.contains(region));
        let fill = |region: RegionId| {
            Color32::from_rgba_unmultiplied(
                248,
                196,
                112,
                if hovered == Some(region) { 150 } else { 85 },
            )
        };
        if let Some(active) = self.runtime.active() {
            // One mesh, so the highlight does not print the triangulation on
            // the subdomain the user is being asked to look at.
            let mesh = &active.mesh;
            let mut highlight = egui::Mesh::default();
            for triangle in &mesh.triangles {
                if !candidates.contains(&triangle.region) {
                    continue;
                }
                let color = fill(triangle.region);
                let first = highlight.vertices.len() as u32;
                for index in triangle.vertices {
                    highlight.colored_vertex(self.screen(mesh.vertices[index].point, r), color);
                }
                highlight.add_triangle(first, first + 1, first + 2);
            }
            if !highlight.is_empty() {
                painter.add(egui::Shape::mesh(highlight));
            }
        }
        let compiled = self
            .editor
            .compiled_draft
            .as_ref()
            .unwrap_or(&self.editor.compiled_accepted);
        let region_of = |face: FaceId| {
            compiled
                .assignments
                .iter()
                .find(|assignment| assignment.face == face)
                .and_then(|assignment| assignment.region)
                .filter(|region| candidates.contains(region))
        };
        for edge in &compiled.topology.edges {
            if region_of(edge.left).is_some() || region_of(edge.right).is_some() {
                painter.line_segment(
                    [
                        self.screen(edge.points[0], r),
                        self.screen(edge.points[1], r),
                    ],
                    Stroke::new(2.4, GOLD),
                );
            }
        }
        for face in &compiled.topology.faces {
            let Some(region) = region_of(face.id) else {
                continue;
            };
            let Some(centroid) = face.centroid() else {
                continue;
            };
            let point = self.screen(centroid, r);
            let label = format!("Keep {}", self.material_name(region));
            let galley =
                painter.layout_no_wrap(label, egui::FontId::proportional(13.0), Color32::WHITE);
            let rect = egui::Rect::from_center_size(point, galley.size() + egui::vec2(14.0, 8.0));
            painter.rect_filled(rect, 4.0, Color32::from_rgba_unmultiplied(8, 13, 18, 210));
            painter.rect_stroke(rect, 4.0, Stroke::new(1.0, GOLD), egui::StrokeKind::Outside);
            painter.galley(rect.min + egui::vec2(7.0, 4.0), galley, Color32::WHITE);
        }
    }
    /// The question a staged deletion asks, anchored over the viewport so it is
    /// visible whatever panels are open.
    pub(super) fn removal_prompt(&mut self, ctx: &egui::Context, viewport: Rect) {
        if self.pending_merge.is_none() || self.capturing() {
            return;
        }
        let mut cancel = false;
        egui::Window::new("Merge subdomains")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(Pos2::new(viewport.center().x, viewport.top() + 14.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Click the subdomain that keeps its material").strong(),
                    );
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel {
            self.pending_merge = None;
        }
    }
    /// Answers the staged deletion with the candidate under a click; a click
    /// anywhere else leaves the question open.
    pub(super) fn pick_merge_survivor(&mut self, point: Point2) {
        let Some(pending) = self.pending_merge.clone() else {
            return;
        };
        let Some(region) = self
            .draft_region_at(point)
            .filter(|region| pending.choices.contains(region))
        else {
            return;
        };
        match &pending.action {
            MergeAction::Delete(spans) => {
                let outcome = self.editor.removal_target(spans).and_then(|target| {
                    self.editor
                        .remove(&target, Some(region))
                        .map(|removal| (target, removal))
                });
                match outcome {
                    Ok((target, removal)) => {
                        self.pending_merge = None;
                        self.selection = TopologySelection::None;
                        self.invalidate_samples();
                        self.report_removal(&target, &removal);
                    }
                    Err(error) => self.message = error,
                }
            }
            MergeAction::Weld {
                curve,
                node,
                endpoint,
                target,
            } => {
                let (curve, node, endpoint, target) = (*curve, *node, *endpoint, *target);
                if self.weld(curve, node, endpoint, target, Some(region)) {
                    self.pending_merge = None;
                }
            }
        }
    }
    /// The closed-curve Subdomain/Hole switch; acts on the complete curve
    /// selection.
    pub(super) fn span_inspector(
        &mut self,
        ui: &mut egui::Ui,
        spans: BTreeSet<TopologySpanTarget>,
    ) {
        ui.label(format!(
            "{} span{}",
            spans.len(),
            if spans.len() == 1 { "" } else { "s" }
        ));
        let curve_spans = spans
            .iter()
            .filter_map(|target| match target {
                TopologySpanTarget::Curve(span) => Some(*span),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        // Two states the boundary settings below do not describe. A span with
        // an excluded face on both sides, two holes say, bounds nothing the
        // simulation solves. A transmitting span with an excluded face on one
        // side has nothing to transmit into, and the plan walls it.
        if let Some(compiled) = self.editor.compiled_draft.as_ref() {
            let contexts = curve_spans
                .iter()
                .filter_map(|span| span_context(compiled, *span))
                .collect::<Vec<_>>();
            let inactive = contexts
                .iter()
                .filter(|context| !context.left.active && !context.right.active)
                .count();
            let walled = contexts
                .iter()
                .filter(|context| {
                    context.behavior == SpanBehavior::Transmitting
                        && context.left.active != context.right.active
                })
                .count();
            let badge = |count: usize, one: &str, many: &str| {
                if count == curve_spans.len() && count == 1 {
                    one.to_owned()
                } else if count == curve_spans.len() {
                    format!("All {many}")
                } else {
                    format!("{count} of {} {many}", curve_spans.len())
                }
            };
            if inactive > 0 {
                ui.colored_label(GOLD, badge(inactive, "Inactive", "inactive"))
                    .on_hover_text(
                        "Excluded on both sides, so nothing here reaches the simulation",
                    );
            }
            if walled > 0 {
                ui.colored_label(GOLD, badge(walled, "Walled", "walled"))
                    .on_hover_text(
                        "Transmit meets an excluded face here, so the span reflects until the far side carries a material",
                    );
            }
        }
        if !curve_spans.is_empty() {
            // The boxes report which of the two states the selection is in
            // rather than offering an action. A mixed selection ticks neither,
            // and unticking the state a span is already in would leave it in no
            // state at all, so only a tick applies anything.
            let state =
                span_behavior_state(&self.editor.document.model.draft.geometry, &curve_spans);
            ui.horizontal(|ui| {
                for (value, label, hint, behavior) in [
                    (
                        SpanBehaviorState::Transmit,
                        "Transmit",
                        "The field crosses these spans",
                        SpanBehavior::Transmitting,
                    ),
                    (
                        SpanBehaviorState::Boundary,
                        "Boundary",
                        "Each side of these spans carries its own condition",
                        SpanBehavior::REFLECTING,
                    ),
                ] {
                    let mut checked = state == Some(value);
                    if ui
                        .checkbox(&mut checked, label)
                        .on_hover_text(hint)
                        .changed()
                        && checked
                        && let Err(error) = self.editor.set_span_behavior(&curve_spans, behavior)
                    {
                        self.notify(error);
                    }
                }
                if state.is_none() {
                    ui.weak("Mixed").on_hover_text(
                        "These spans are not all in the same state; tick one to put them there",
                    );
                }
            });
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.selected_side, CurveTraceSide::Left, "Left");
                ui.selectable_value(&mut self.selected_side, CurveTraceSide::Right, "Right");
            });
            let first_behavior = self
                .editor
                .document
                .model
                .draft
                .geometry
                .curves
                .iter()
                .flat_map(|curve| &curve.spans)
                .find(|span| curve_spans.contains(&span.id))
                .map(|span| span.behavior);
            if let Some(behavior) = first_behavior {
                let mut condition = match behavior {
                    SpanBehavior::Transmitting => FaceBoundaryCondition::Reflecting,
                    SpanBehavior::Separated { left, right, .. } => match self.selected_side {
                        CurveTraceSide::Left => left,
                        CurveTraceSide::Right => right,
                    },
                };
                if edit_face_condition(ui, self.editor.document.model.draft.physics, &mut condition)
                    && let Err(error) = self.editor.set_span_face_condition(
                        &curve_spans,
                        self.selected_side,
                        condition,
                    )
                {
                    self.notify(error);
                }
                let (mut thin_gap, mut stiffness) = match behavior {
                    SpanBehavior::Separated {
                        coupling: InternalBoundaryCoupling::ThinGap { stiffness_ratio },
                        ..
                    } => (true, stiffness_ratio),
                    _ => (false, 1.0),
                };
                let gap_changed = ui.checkbox(&mut thin_gap, "Thin gap coupling").changed();
                let stiffness_changed = thin_gap
                    && ui
                        .add(
                            egui::DragValue::new(&mut stiffness)
                                .speed(0.02)
                                .range(1.0e-6..=1.0e6)
                                .prefix("Stiffness "),
                        )
                        .changed();
                if (gap_changed || stiffness_changed)
                    && let Err(error) = self.editor.set_span_coupling(
                        &curve_spans,
                        if thin_gap {
                            InternalBoundaryCoupling::ThinGap {
                                stiffness_ratio: stiffness,
                            }
                        } else {
                            InternalBoundaryCoupling::Independent
                        },
                    )
                {
                    self.notify(error);
                }
            }
            if curve_spans.len() == 1 {
                let span = *curve_spans.first().unwrap();
                if let Some(context) = self
                    .editor
                    .compiled_draft
                    .as_ref()
                    .and_then(|scene| span_context(scene, span))
                {
                    ui.weak(format!(
                        "{} side · {}",
                        if self.selected_side == CurveTraceSide::Left {
                            "Left"
                        } else {
                            "Right"
                        },
                        if match self.selected_side {
                            CurveTraceSide::Left => context.left.active,
                            CurveTraceSide::Right => context.right.active,
                        } {
                            "active"
                        } else {
                            "excluded"
                        }
                    ));
                }
                if let Some((curve, breakpoint, continuity, attached)) =
                    self.selected_end_continuity(span)
                {
                    ui.label(format!("End knot · C{continuity}"));
                    ui.horizontal_wrapped(|ui| {
                        for (target, label) in
                            [(2, "C2 smooth"), (1, "C1 tangent"), (0, "C0 corner")]
                        {
                            let response = ui.add_enabled(
                                target == 0 || !attached,
                                egui::Button::selectable(continuity == target, label),
                            );
                            if response.clicked() && continuity != target {
                                match self.editor.set_curve_continuity(curve, breakpoint, target) {
                                    Ok(displacement) => {
                                        self.invalidate_samples();
                                        if displacement > 0.0 {
                                            self.notify(format!(
                                                "Curve smoothed; maximum displacement {displacement:.3e}"
                                            ));
                                        }
                                    }
                                    Err(error) => self.notify(error),
                                }
                            }
                        }
                    });
                }
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button("Straighten spans")
                    .on_hover_text("Make each selected span straight between its own endpoints")
                    .clicked()
                {
                    match self.editor.straighten_spans(&curve_spans) {
                        Ok(()) => self.invalidate_samples(),
                        Err(error) => self.notify(error),
                    }
                }
                let contiguous = self.editor.selection_is_contiguous(&curve_spans);
                if ui
                    .add_enabled(contiguous, egui::Button::new("Straighten selection"))
                    .on_hover_text(if contiguous {
                        "Lay the whole selected run on one straight chord"
                    } else {
                        "Select one contiguous run of spans on each curve"
                    })
                    .clicked()
                {
                    match self.editor.straighten_span_sections(&curve_spans) {
                        Ok(()) => self.invalidate_samples(),
                        Err(error) => self.notify(error),
                    }
                }
                if ui
                    .button("Isolate at C0")
                    .on_hover_text("Add exact corners at the selected section boundaries")
                    .clicked()
                {
                    match self.editor.isolate_span_boundaries(&curve_spans) {
                        Ok(()) => self.invalidate_samples(),
                        Err(error) => self.notify(error),
                    }
                }
            });
            let pivot = self
                .gizmo_pivot_for(&spans, &curve_spans)
                .unwrap_or_default();
            let transform = RigidTransform {
                pivot,
                translation: Point2::default(),
                rotation_radians: 0.0,
                scale: 1.0,
            };
            // A selection that cannot move rigidly simply gets no gizmo. What is
            // holding it is visible in the scene, and a marquee fixes it.
            let transform_allowed = plan_rigid_transform(
                &self.editor.document.model.draft.geometry,
                &spans,
                transform,
            )
            .is_ok();
            if transform_allowed {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.transform_translation.x)
                            .speed(0.01)
                            .prefix("Δx "),
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.transform_translation.y)
                            .speed(0.01)
                            .prefix("Δy "),
                    );
                });
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.transform_rotation_degrees)
                            .speed(0.5)
                            .suffix("°"),
                    );
                    ui.add(
                        egui::DragValue::new(&mut self.transform_scale)
                            .speed(0.01)
                            .range(0.01..=100.0)
                            .prefix("Scale "),
                    );
                });
                if ui.button("Apply transform").clicked() {
                    let transform = RigidTransform {
                        pivot,
                        translation: self.transform_translation,
                        rotation_radians: self.transform_rotation_degrees.to_radians(),
                        scale: self.transform_scale,
                    };
                    match plan_rigid_transform(
                        &self.editor.document.model.draft.geometry,
                        &spans,
                        transform,
                    )
                    .map_err(|error| error.to_string())
                    .and_then(|updates| self.editor.apply_transform_updates(&updates))
                    {
                        Ok(()) => {
                            if self
                                .gizmo_pivot
                                .as_ref()
                                .is_some_and(|(selection, _)| selection == &spans)
                            {
                                self.gizmo_pivot =
                                    Some((spans.clone(), pivot + self.transform_translation));
                            }
                            self.transform_translation = Point2::default();
                            self.transform_rotation_degrees = 0.0;
                            self.transform_scale = 1.0;
                            self.invalidate_samples();
                        }
                        Err(error) => self.notify(error),
                    }
                }
            }
            // One button for every selection, running the same path as the
            // Delete key: whole curves, partial runs, and the survivor picker
            // when a deletion merges two subdomains.
            let whole = self.selected_complete_curves(&curve_spans).len();
            if ui
                .button("Delete")
                .on_hover_text(match whole {
                    0 => "Delete the selected spans and leave the rest as baffles",
                    1 => "Delete the selected curve",
                    _ => "Delete the selected curves",
                })
                .clicked()
            {
                self.delete_selection();
                self.invalidate_samples();
            }
        }
        let outer = spans
            .iter()
            .filter_map(|target| match target {
                TopologySpanTarget::Outer(side) => Some(*side),
                _ => None,
            })
            .collect::<Vec<_>>();
        if !outer.is_empty() {
            let sides = outer.iter().copied().collect::<BTreeSet<_>>();
            let mut condition =
                self.editor.document.model.draft.outer_boundaries.sides[outer[0].index()];
            if edit_outer_condition(ui, self.editor.document.model.draft.physics, &mut condition)
                && let Err(error) = self.editor.set_outer_condition(&sides, condition)
            {
                self.notify(error);
            }
            let mut domain = self.editor.document.model.draft.geometry.domain;
            let before = domain;
            ui.label("Domain extents");
            ui.horizontal(|ui| {
                ui.label("Left");
                ui.add(egui::DragValue::new(&mut domain.min_x).speed(0.01));
                ui.label("Right");
                ui.add(egui::DragValue::new(&mut domain.max_x).speed(0.01));
            });
            ui.horizontal(|ui| {
                ui.label("Bottom");
                ui.add(egui::DragValue::new(&mut domain.min_y).speed(0.01));
                ui.label("Top");
                ui.add(egui::DragValue::new(&mut domain.max_y).speed(0.01));
            });
            if domain != before {
                match self.editor.set_domain(domain) {
                    Ok(()) => self.invalidate_samples(),
                    Err(error) => self.notify(error),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use funfern_app::topology_editor::{ClosedCurvePurpose, TopologyEditor};
    use funfern_app::topology_viewport::screen_side;

    /// The picker lists one set of names and the two condition enums answer
    /// with their own, so a condition used to rename itself the moment it was
    /// chosen. They are held to the same words here, in both directions: every
    /// kind is reachable and every condition reads back as the kind that lists
    /// it.
    #[test]
    fn boundary_names_agree_across_every_source() {
        let all = [
            BoundaryKind::Reflecting,
            BoundaryKind::FirstOrder,
            BoundaryKind::SecondOrder,
            BoundaryKind::ElectricWall,
            BoundaryKind::MagneticWall,
            BoundaryKind::Neumann,
            BoundaryKind::Dirichlet,
        ];
        let signal = TimeSignal::harmonic(0.0, 1.0, 1.0, 0.0);
        let faces = [
            FaceBoundaryCondition::Reflecting,
            FaceBoundaryCondition::Impedance { ratio: 1.0 },
            FaceBoundaryCondition::SecondOrderOutgoing,
            FaceBoundaryCondition::ElectricWall,
            FaceBoundaryCondition::MagneticWall,
            FaceBoundaryCondition::Neumann { signal },
            FaceBoundaryCondition::Dirichlet { signal },
        ];
        for condition in faces {
            assert_eq!(
                face_kind(condition).label(),
                condition.label(),
                "{condition:?} is listed under another name"
            );
        }
        let outers = [
            OuterBoundaryCondition::Reflecting,
            OuterBoundaryCondition::FirstOrderOutgoing,
            OuterBoundaryCondition::SecondOrderOutgoing,
            OuterBoundaryCondition::ElectricWall,
            OuterBoundaryCondition::MagneticWall,
            OuterBoundaryCondition::Neumann { signal },
            OuterBoundaryCondition::Dirichlet { signal },
        ];
        for condition in outers {
            assert_eq!(
                outer_kind(condition).label(),
                condition.label(),
                "{condition:?} is listed under another name"
            );
        }
        // Each list covers every kind the picker offers, so nothing is
        // unreachable and no two kinds share a name.
        for kinds in [
            faces.map(face_kind).to_vec(),
            outers.map(outer_kind).to_vec(),
        ] {
            assert_eq!(kinds, all.to_vec());
        }
        assert_eq!(
            all.iter()
                .map(|kind| kind.label())
                .collect::<BTreeSet<_>>()
                .len(),
            all.len()
        );
    }

    #[test]
    fn boundary_picker_uses_the_active_skins_physical_vocabulary() {
        let mechanical = BoundaryKind::choices(PhysicsModel::Mechanical);
        assert!(mechanical.contains(&BoundaryKind::Reflecting));
        assert!(!mechanical.contains(&BoundaryKind::ElectricWall));
        assert!(!mechanical.contains(&BoundaryKind::MagneticWall));
        assert_eq!(
            BoundaryKind::Reflecting.label_for(PhysicsModel::Mechanical),
            "Free boundary"
        );

        let tm = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        };
        let te = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        };
        for physics in [tm, te] {
            let choices = BoundaryKind::choices(physics);
            assert!(!choices.contains(&BoundaryKind::Reflecting));
            assert!(choices.contains(&BoundaryKind::ElectricWall));
            assert!(choices.contains(&BoundaryKind::MagneticWall));
        }
        assert_eq!(
            BoundaryKind::Reflecting.presented(tm),
            BoundaryKind::MagneticWall,
            "the natural TM scalar boundary is a magnetic wall"
        );
        assert_eq!(
            BoundaryKind::Reflecting.presented(te),
            BoundaryKind::ElectricWall,
            "the natural TE scalar boundary is an electric wall"
        );
        assert_eq!(
            BoundaryKind::ElectricWall.presented(PhysicsModel::Mechanical),
            BoundaryKind::Dirichlet
        );
        assert_eq!(
            BoundaryKind::MagneticWall.presented(PhysicsModel::Mechanical),
            BoundaryKind::Reflecting
        );
    }

    #[test]
    fn a_span_selection_reports_one_state_only_when_every_span_agrees() {
        let mut editor = TopologyEditor::default();
        editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(0.0, 0.0), 0.3),
                ClosedCurvePurpose::Subdomain {
                    material: DEFAULT_MATERIAL,
                },
            )
            .unwrap();
        let spans = editor.document.model.draft.geometry.curves[0]
            .spans
            .iter()
            .map(|span| span.id)
            .collect::<BTreeSet<_>>();
        assert!(spans.len() > 1);
        let first = BTreeSet::from([*spans.first().unwrap()]);
        let state = |editor: &TopologyEditor, spans: &BTreeSet<CurveSpanId>| {
            span_behavior_state(&editor.document.model.draft.geometry, spans)
        };

        // A closed subdomain's spans start out transmitting.
        assert_eq!(state(&editor, &spans), Some(SpanBehaviorState::Transmit));

        editor
            .set_span_behavior(&first, SpanBehavior::REFLECTING)
            .unwrap();
        assert_eq!(state(&editor, &spans), None);
        assert_eq!(state(&editor, &first), Some(SpanBehaviorState::Boundary));

        // A boundary stays a boundary whatever its faces carry, which is what
        // the old pair of buttons could not say.
        editor
            .set_span_face_condition(
                &first,
                CurveTraceSide::Left,
                FaceBoundaryCondition::Impedance { ratio: 2.0 },
            )
            .unwrap();
        assert_eq!(state(&editor, &first), Some(SpanBehaviorState::Boundary));

        editor
            .set_span_behavior(&spans, SpanBehavior::REFLECTING)
            .unwrap();
        assert_eq!(state(&editor, &spans), Some(SpanBehaviorState::Boundary));
    }

    #[test]
    fn the_side_band_falls_where_a_click_reads_the_same_side() {
        // A span drawn left to right on screen runs along +x in the world, so
        // its left side is up the screen.
        assert_eq!(
            side_offset(egui::vec2(2.0, 0.0), CurveTraceSide::Left),
            egui::vec2(0.0, -1.0)
        );
        for (x, y) in [
            (1.0, 0.0),
            (0.0, 1.0),
            (-1.0, 0.0),
            (0.0, -1.0),
            (3.0, -2.0),
            (-1.5, -4.0),
        ] {
            let a = ScreenPoint::new(0.0, 0.0);
            let b = ScreenPoint::new(f64::from(x), f64::from(y));
            for side in [CurveTraceSide::Left, CurveTraceSide::Right] {
                let offset = side_offset(egui::vec2(x, y), side) * 3.0;
                let point = ScreenPoint::new(f64::from(offset.x), f64::from(offset.y));
                assert_eq!(
                    screen_side(a, b, point),
                    side,
                    "a band on the {side:?} of ({x}, {y}) reads back as the other side"
                );
            }
        }
        assert_eq!(
            side_offset(egui::Vec2::ZERO, CurveTraceSide::Left),
            egui::Vec2::ZERO
        );
    }
}
