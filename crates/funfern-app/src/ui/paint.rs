//! Everything the viewport paints from committed state: the field itself, the
//! vector overlay, probe markers and the geometry handles over them.

use crate::field_paint::{FieldPaintCallback, FieldPaintEdge, FieldPaintTopology};
use crate::material_overlay::{MaterialOverlay, MaterialProperty};
use crate::wave_gpu::{VectorOverlayDisplay, WaveDisplay};
use bevy::prelude::*;
use bevy_egui::egui::{self, Color32, Pos2, Rect, Stroke};
use funfern_app::document::VectorOverlay;
use funfern_app::topology_editor::TopologyProbeTarget;
use funfern_app::topology_viewport::{
    SampledTopologyGeometry, ScreenPoint, TopologyHandle, TopologySelection, TopologySpanTarget,
    selected_span_controls,
};
use funfern_core::*;
use std::collections::BTreeMap;
use std::sync::Arc;

use super::*;

impl Playground {
    pub(super) fn draw_solution(
        &mut self,
        painter: &egui::Painter,
        r: Rect,
        display: &WaveDisplay,
        vector_display: &VectorOverlayDisplay,
    ) {
        let Some(active) = self.runtime.active().cloned() else {
            return;
        };
        let mesh = &active.mesh;
        let presentation = self.editor.document.presentation;
        // One mesh rather than a polygon per triangle, for the reason the
        // categorical overlay gives below: per-polygon outlines would imprint
        // the mesh on the wash whether or not the user asked to see it.
        if presentation.material_overlay == MaterialOverlay::AdaptationTarget
            && let Some(result) = &self.amr_indicator_result
            && result.element_targets.len() == mesh.triangles.len()
        {
            let span = (self.editor.document.presentation.adaptation.maximum_edge
                - self.editor.document.presentation.adaptation.minimum_edge)
                .max(f64::MIN_POSITIVE);
            let alpha = (presentation.material_overlay_opacity * 210.0).round() as u8;
            let mut targets = egui::Mesh::default();
            targets.reserve_vertices(mesh.triangles.len() * 3);
            targets.reserve_triangles(mesh.triangles.len());
            for (triangle, target) in mesh.triangles.iter().zip(&result.element_targets) {
                let fraction =
                    ((*target - self.editor.document.presentation.adaptation.minimum_edge) / span)
                        .clamp(0.0, 1.0) as f32;
                let color = amr_target_color(fraction, alpha);
                let first = targets.vertices.len() as u32;
                for index in triangle.vertices {
                    targets.colored_vertex(self.screen(mesh.vertices[index].point, r), color);
                }
                targets.add_triangle(first, first + 1, first + 2);
            }
            if !targets.is_empty() {
                painter.add(egui::Shape::mesh(targets));
            }
        }
        if let MaterialOverlay::Property(property) = presentation.material_overlay
            && let Some(snapshot) = &self.material_overlay_snapshot
            && let Some(range) = self.material_overlay_range(property)
        {
            let alpha = (presentation.material_overlay_opacity * 255.0).round() as u8;
            let mut values = egui::Mesh::default();
            values.reserve_vertices(snapshot.samples.len());
            values.reserve_triangles(snapshot.triangles.len());
            for sample in &snapshot.samples {
                let color = sample
                    .value(property)
                    .ok()
                    .and_then(|value| {
                        range
                            .normalized(value, presentation.material_overlay_logarithmic)
                            .map(|fraction| overlay_property_color(property, fraction, alpha))
                    })
                    .unwrap_or(Color32::from_rgba_unmultiplied(255, 73, 91, alpha.max(150)));
                values.colored_vertex(self.screen(sample.point, r), color);
            }
            for triangle in &snapshot.triangles {
                values.add_triangle(triangle[0], triangle[1], triangle[2]);
            }
            painter.add(egui::Shape::mesh(values));
            if property == MaterialProperty::Anisotropy {
                let stride = (snapshot.samples.len() / 90).max(1);
                for sample in snapshot.samples.iter().step_by(stride) {
                    let Ok(ratio) = sample.value(property) else {
                        continue;
                    };
                    if ratio <= 1.0 + 1.0e-6 {
                        continue;
                    }
                    let angle = snapshot
                        .key
                        .scene
                        .region(sample.region)
                        .map_or(0.0, |region| region.frame.angle_radians);
                    let center = self.screen(sample.point, r);
                    let direction = egui::vec2(angle.cos() as f32, -angle.sin() as f32) * 7.0;
                    painter.line_segment(
                        [center - direction, center + direction],
                        Stroke::new(1.2, Color32::from_rgba_unmultiplied(232, 247, 242, 190)),
                    );
                }
            }
        }
        // Categorical colors remain flat per triangle, but their positions and
        // draw call live in the persistent field callback. Sending one egui
        // mesh per frame became a second topology-sized copy after the scalar
        // field itself moved to the GPU path.
        let categorical_colors: Option<Arc<[[u8; 4]]>> = matches!(
            presentation.material_overlay,
            MaterialOverlay::Regions | MaterialOverlay::Subdomains
        )
        .then(|| {
            mesh.triangles
                .iter()
                .map(|triangle| {
                    match presentation.material_overlay {
                        MaterialOverlay::Regions => active
                            .bundle
                            .authored
                            .region(triangle.region)
                            .and_then(|region| active.bundle.authored.material(region.material))
                            .map(|material| {
                                Color32::from_rgba_unmultiplied(
                                    material.color[0],
                                    material.color[1],
                                    material.color[2],
                                    (presentation.material_overlay_opacity * 210.0) as u8,
                                )
                            })
                            .unwrap_or(Color32::TRANSPARENT),
                        _ => subdomain_color(
                            &self.editor.document.model.draft,
                            triangle.region,
                            presentation.material_overlay_opacity,
                        ),
                    }
                    .to_array()
                })
                .collect::<Vec<_>>()
                .into()
        });
        let (field_values, color_scale): (Option<Arc<[f32]>>, f32) = if presentation.field
            && display.generation > 0
            && display.current.len() == active.operator.degrees_of_freedom()
        {
            // The canonical primary field is authoritative. Display no longer
            // removes a component mean or reconstructs a gauge-dependent
            // scalar before exposure.
            let field_values: Arc<[f32]> = display.current.clone().into();
            let level = exposure_level(
                &field_values,
                FIELD_EXPOSURE_QUANTILE,
                &mut self.exposure_scratch,
            );
            let reference = self.field_exposure.update(level, self.frame_delta);
            let visibility = if presentation.field_auto_exposure {
                self.field_exposure.visibility(level)
            } else {
                1.0
            };
            let scale = visibility
                * field_scale(
                    presentation.field_gain,
                    reference,
                    presentation.field_auto_exposure,
                );
            (Some(field_values), scale as f32)
        } else {
            (None, 0.0)
        };
        if field_values.is_some()
            || categorical_colors.is_some()
            || presentation.mesh
            || presentation.mesh_boundaries
        {
            if self.field_paint_topology.as_ref().is_none_or(|topology| {
                topology.mesh_revision != active.mesh.mesh_revision
                    || topology.positions.len() != active.operator.degrees_of_freedom()
                    || topology.triangles.len() != mesh.triangles.len()
            }) {
                let mut indices = Vec::with_capacity(active.operator.element_nodes().len() * 18);
                for nodes in active.operator.element_nodes() {
                    for [a, b] in [[0, 3], [3, 1], [1, 4], [4, 2], [2, 5], [5, 0]] {
                        indices.extend_from_slice(&[nodes[a], nodes[b], nodes[6]]);
                    }
                }
                let triangles = mesh
                    .triangles
                    .iter()
                    .map(|triangle| {
                        let [a, b, c] = triangle.vertices.map(|index| mesh.vertices[index].point);
                        [
                            a.x as f32, a.y as f32, b.x as f32, b.y as f32, c.x as f32, c.y as f32,
                        ]
                    })
                    .collect::<Vec<_>>();
                let mut edge_kinds = BTreeMap::<[usize; 2], bool>::new();
                for triangle in &mesh.triangles {
                    for [a, b] in [
                        [triangle.vertices[0], triangle.vertices[1]],
                        [triangle.vertices[1], triangle.vertices[2]],
                        [triangle.vertices[2], triangle.vertices[0]],
                    ] {
                        edge_kinds.entry([a.min(b), a.max(b)]).or_insert(false);
                    }
                }
                for edge in &mesh.boundary_edges {
                    let [a, b] = edge.vertices;
                    edge_kinds.insert([a.min(b), a.max(b)], true);
                }
                let edges = edge_kinds
                    .into_iter()
                    .map(|([a, b], boundary)| {
                        let a = mesh.vertices[a].point;
                        let b = mesh.vertices[b].point;
                        FieldPaintEdge {
                            endpoints: [a.x as f32, a.y as f32, b.x as f32, b.y as f32],
                            boundary: u32::from(boundary),
                        }
                    })
                    .collect::<Vec<_>>();
                let replacement = Arc::new(FieldPaintTopology {
                    mesh_revision: active.mesh.mesh_revision,
                    positions: active
                        .operator
                        .node_points()
                        .iter()
                        .map(|point| [point.x as f32, point.y as f32])
                        .collect::<Vec<_>>()
                        .into(),
                    indices: indices.into(),
                    triangles: triangles.into(),
                    edges: edges.into(),
                });
                self.field_paint_topology = Some(replacement);
            }
            painter.add(
                FieldPaintCallback {
                    topology: self.field_paint_topology.as_ref().unwrap().clone(),
                    values: field_values,
                    overlay_colors: categorical_colors,
                    world_center: [self.center.x as f32, self.center.y as f32],
                    world_to_clip: [
                        (2.0 * self.scale / f64::from(r.width())) as f32,
                        (2.0 * self.scale / f64::from(r.height())) as f32,
                    ],
                    point_to_clip: [2.0 / r.width(), 2.0 / r.height()],
                    color_scale,
                    over_overlay: presentation.material_overlay != MaterialOverlay::Off,
                    mesh_lines: presentation.mesh,
                    boundary_lines: presentation.mesh_boundaries,
                }
                .shape(r),
            );
        }
        let mode = presentation
            .vector_overlay
            .resolved(active.bundle.authored.physics);
        if mode != VectorOverlay::Off {
            let layout = self
                .vector_overlay_layout
                .as_ref()
                .filter(|layout| layout.matches(vector_display))
                .or_else(|| {
                    self.vector_overlay_previous_layout
                        .as_ref()
                        .filter(|layout| layout.matches(vector_display))
                })
                // A mesh handoff may briefly draw the conservatively
                // transferred old lattice over the new mesh, but a skin
                // change changes the physical meaning of both vectors.
                .filter(|layout| layout.key.physics == active.bundle.authored.physics);
            let owner = layout.map(|layout| VectorOverlayAcOwner {
                mesh_revision: layout.key.mesh_revision,
                physics: layout.key.physics,
            });
            let samples = layout
                .map(|layout| {
                    layout
                        .points
                        .iter()
                        .zip(&vector_display.samples)
                        .map(|(point, sample)| {
                            let value = match mode {
                                VectorOverlay::ComplementaryField => sample.complementary,
                                VectorOverlay::RelativeEnergyFlow => sample.energy_flow,
                                VectorOverlay::Off => Point2::default(),
                            };
                            (
                                point.element,
                                self.screen(point.point, r),
                                value,
                                sample.pre_filter_complementary,
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            self.draw_vector_overlay(
                painter,
                samples,
                owner.unwrap_or(VectorOverlayAcOwner {
                    mesh_revision: active.mesh.mesh_revision,
                    physics: active.bundle.authored.physics,
                }),
                vector_display.completed_steps,
                vector_display.absolute_time,
            );
        } else {
            self.vector_overlay_ac_state.clear();
            self.vector_overlay_ac_owner = None;
            self.vector_overlay_dc_step = u64::MAX;
            self.vector_overlay_dc_active = false;
        }
    }

    pub(super) fn draw_vector_overlay(
        &mut self,
        painter: &egui::Painter,
        mut samples: Vec<(u32, Pos2, Point2, Point2)>,
        owner: VectorOverlayAcOwner,
        completed_steps: u64,
        absolute_time: f64,
    ) {
        let settings = self.editor.document.presentation;
        let mode = settings
            .vector_overlay
            .resolved(self.editor.document.model.draft.physics);
        if self.vector_overlay_mode != mode {
            self.vector_overlay_ac_state.clear();
            self.vector_overlay_ac_owner = None;
            self.vector_overlay_dc_step = u64::MAX;
            self.vector_overlay_dc_active = false;
            self.vector_overlay_exposure.clear();
            self.vector_overlay_mode = mode;
        }
        let dc_active =
            mode == VectorOverlay::ComplementaryField && settings.vector_overlay_ac_coupled;
        if self.vector_overlay_dc_active != dc_active {
            self.vector_overlay_ac_state.clear();
            self.vector_overlay_ac_owner = None;
            self.vector_overlay_dc_step = u64::MAX;
            self.vector_overlay_exposure.clear();
            self.vector_overlay_dc_active = dc_active;
        }
        if dc_active {
            self.retain_vector_overlay_ac_owner(
                owner,
                &samples,
                completed_steps,
                absolute_time,
                settings.vector_overlay_density * 1.5,
            );
            self.ac_couple_vector_samples(
                &mut samples,
                completed_steps,
                absolute_time,
                resident_filter_boundary(self.grid_filter_running(), completed_steps),
            );
        } else {
            self.vector_overlay_ac_state.clear();
            self.vector_overlay_ac_owner = Some(owner);
            self.vector_overlay_dc_step = completed_steps;
        }
        let mut magnitudes = samples
            .iter()
            .map(|(_, _, value, _)| value.norm())
            .filter(|magnitude| magnitude.is_finite() && *magnitude > 0.0)
            .collect::<Vec<_>>();
        if magnitudes.is_empty() {
            return;
        }
        magnitudes.sort_by(f64::total_cmp);
        let instantaneous = magnitudes[(magnitudes.len() - 1) * 9 / 10];
        let Some(reference) = self
            .vector_overlay_exposure
            .update(instantaneous, self.frame_delta)
        else {
            return;
        };
        let visibility = self.vector_overlay_exposure.visibility(instantaneous);
        // Below this point even the longest possible arrow is sub-pixel. Do
        // not normalize its direction: f32 residue has no stable direction,
        // so drawing it only turns numerical noise into visible twitching.
        if visibility <= VECTOR_OVERLAY_VISIBILITY_CUTOFF {
            return;
        }
        let maximum_length = settings.vector_overlay_density * 0.46;
        for (_, origin, value, _) in samples {
            let magnitude = value.norm();
            if magnitude < reference * 0.015 || !magnitude.is_finite() {
                continue;
            }
            // Clamp the exposed arrow first and fade the result. Clamping
            // after multiplication by `visibility` let a sparse outlier undo
            // the quiet-tail fade and remain at full length.
            let length = vector_arrow_length(
                magnitude,
                reference,
                settings.vector_overlay_gain,
                maximum_length,
                visibility,
            );
            let direction = egui::vec2(value.x as f32, -value.y as f32).normalized();
            let tip = origin + direction * length;
            let normal = egui::vec2(-direction.y, direction.x);
            let head = 5.0_f32.min(length * 0.35);
            let stroke = Stroke::new(1.45, Color32::from_rgba_unmultiplied(116, 232, 210, 220));
            painter.line_segment([origin, tip], stroke);
            painter.line_segment([tip, tip - direction * head + normal * head * 0.55], stroke);
            painter.line_segment([tip, tip - direction * head - normal * head * 0.55], stroke);
        }
    }

    pub(super) fn retain_vector_overlay_ac_owner(
        &mut self,
        owner: VectorOverlayAcOwner,
        samples: &[(u32, Pos2, Point2, Point2)],
        completed_steps: u64,
        absolute_time: f64,
        remap_radius: f32,
    ) {
        if self.vector_overlay_ac_owner != Some(owner) {
            let compatible_remesh = self
                .vector_overlay_ac_owner
                .is_some_and(|previous| previous.physics == owner.physics)
                && completed_steps >= self.vector_overlay_dc_step
                && absolute_time.is_finite();
            if compatible_remesh {
                let previous = std::mem::take(&mut self.vector_overlay_ac_state);
                let radius_squared = remap_radius * remap_radius;
                for (key, origin, value, _) in samples {
                    let nearest = previous
                        .values()
                        .filter_map(|state| {
                            let distance = state.origin.distance_sq(*origin);
                            (distance <= radius_squared).then_some((distance, state))
                        })
                        .min_by(|left, right| left.0.total_cmp(&right.0));
                    if let Some((_, state)) = nearest {
                        let elapsed = (absolute_time - state.time).max(0.0);
                        self.vector_overlay_ac_state.insert(
                            *key,
                            VectorAcState {
                                // Remeshing is a zero-duration representation
                                // change. Carry the visible AC state, but rebase
                                // its raw input to the transferred new sample.
                                input: *value,
                                output: state.output * (-VECTOR_DC_REJECTION_RATE * elapsed).exp(),
                                step: completed_steps,
                                time: absolute_time,
                                origin: *origin,
                            },
                        );
                    }
                }
                self.vector_overlay_dc_step = completed_steps;
            } else {
                self.vector_overlay_ac_state.clear();
                self.vector_overlay_dc_step = u64::MAX;
            }
            self.vector_overlay_ac_owner = Some(owner);
        }
    }

    /// Removes only the slowly varying presentation baseline from the sampled
    /// complementary field. The exact pole and trapezoidal input difference
    /// keep the corner stable across solver steps and readback batching without
    /// attenuating ordinary source frequencies.
    pub(super) fn ac_couple_vector_samples(
        &mut self,
        samples: &mut [(u32, Pos2, Point2, Point2)],
        completed_steps: u64,
        absolute_time: f64,
        maintenance_discontinuity: bool,
    ) {
        let restarted = self.vector_overlay_dc_step == u64::MAX
            || completed_steps < self.vector_overlay_dc_step
            || !absolute_time.is_finite();
        if restarted {
            self.vector_overlay_ac_state.clear();
        }
        for (key, origin, value, pre_filter_value) in samples {
            match self.vector_overlay_ac_state.entry(*key) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(VectorAcState {
                        input: *value,
                        output: Point2::default(),
                        step: completed_steps,
                        time: absolute_time,
                        origin: *origin,
                    });
                    // A newly visible physical sample has no temporal history.
                    // Passing its first value through would interpret an
                    // unknown DC baseline as AC and flash whenever the view
                    // moves. Start silent; subsequent accepted samples provide
                    // the temporal difference the high-pass actually knows.
                    *value = Point2::default();
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    let state = entry.get_mut();
                    state.origin = *origin;
                    if completed_steps > state.step {
                        // The two-f32 clock is substantially more precise than
                        // one absolute f32, but a clock rebase can still round
                        // its reconstructed value a hair backwards. Step order
                        // is authoritative; clamp only that rounding residue.
                        let elapsed = (absolute_time - state.time).max(0.0);
                        let pole = (-VECTOR_DC_REJECTION_RATE * elapsed).exp();
                        if maintenance_discontinuity {
                            // First advance through the ordinary evolution up
                            // to the pre-filter endpoint, then rebase the input
                            // to the post-filter value without presenting the
                            // zero-duration numerical correction as a wave.
                            let input_gain = 0.5 * (1.0 + pole);
                            state.output = state.output * pole
                                + (*pre_filter_value - state.input) * input_gain;
                        } else {
                            let input_gain = 0.5 * (1.0 + pole);
                            state.output =
                                state.output * pole + (*value - state.input) * input_gain;
                        }
                        state.input = *value;
                        state.step = completed_steps;
                        state.time = absolute_time;
                    }
                    *value = state.output;
                }
            }
        }
        if restarted || completed_steps > self.vector_overlay_dc_step {
            self.vector_overlay_dc_step = completed_steps;
        }
    }
    pub(super) fn draw_sampled(
        &self,
        painter: &egui::Painter,
        r: Rect,
        sampled: &SampledTopologyGeometry,
        color: Color32,
        width: f32,
        interactive: bool,
    ) {
        if interactive && self.editor.document.presentation.control_polygons {
            for curve in &self.editor.document.model.draft.geometry.curves {
                let (controls, closed) = match &curve.spline {
                    CurveSpline::Closed(spline) => (spline.controls(), true),
                    CurveSpline::Open(spline) => (spline.controls(), false),
                };
                for pair in controls.windows(2) {
                    painter.line_segment(
                        [self.screen(pair[0], r), self.screen(pair[1], r)],
                        Stroke::new(0.8, Color32::from_rgba_unmultiplied(156, 171, 180, 100)),
                    );
                }
                if closed && controls.len() > 2 {
                    painter.line_segment(
                        [
                            self.screen(*controls.last().unwrap(), r),
                            self.screen(controls[0], r),
                        ],
                        Stroke::new(0.8, Color32::from_rgba_unmultiplied(156, 171, 180, 100)),
                    );
                }
            }
        }
        // The boundary-law strokes are a diagnostic layer: draw them first and
        // push them clear of a selected span so the selection always reads above.
        if interactive && self.editor.document.presentation.boundary_conditions {
            for span in &sampled.spans {
                let Some((left, right)) = self.span_condition_colors(span.target) else {
                    continue;
                };
                let offset = if self.span_selected(span.target) {
                    width * 0.5 + 4.0
                } else {
                    2.5
                };
                for segment in span.samples.windows(2) {
                    let a = self.screen(segment[0].point, r);
                    let b = self.screen(segment[1].point, r);
                    let tangent = b - a;
                    if tangent.length_sq() <= f32::EPSILON {
                        continue;
                    }
                    let normal = side_offset(tangent, CurveTraceSide::Left) * offset;
                    painter.line_segment([a + normal, b + normal], Stroke::new(1.4, left));
                    painter.line_segment([a - normal, b - normal], Stroke::new(1.4, right));
                }
            }
        }
        for span in &sampled.spans {
            let selected = interactive && self.span_selected(span.target);
            let points = span
                .samples
                .iter()
                .map(|sample| self.screen(sample.point, r))
                .collect::<Vec<_>>();
            if points.len() < 2 {
                continue;
            }
            if selected {
                // A dark halo keeps the blue readable over the law strokes and
                // over a bright field.
                painter.add(egui::Shape::line(
                    points.clone(),
                    Stroke::new(width + 4.5, Color32::from_rgba_unmultiplied(8, 13, 18, 190)),
                ));
            }
            painter.add(egui::Shape::line(
                points,
                Stroke::new(
                    if selected { width + 2.5 } else { width },
                    if selected { SELECT } else { color },
                ),
            ));
        }
        // The side the Boundary inspector is editing. A span's two traces are
        // geometrically coincident, so without a band in the scene the Left and
        // Right buttons name something the scene never shows. The arrow gives
        // the start-to-end direction those names are measured from.
        if interactive {
            // Clear of the condition strokes when they are on, and tight against
            // the span when they are not.
            let offset = width * 0.5
                + if self.editor.document.presentation.boundary_conditions {
                    7.5
                } else {
                    4.0
                };
            for span in &sampled.spans {
                if !matches!(span.target, TopologySpanTarget::Curve(_))
                    || !self.span_selected(span.target)
                {
                    continue;
                }
                let Some(middle) = span
                    .samples
                    .first()
                    .zip(span.samples.last())
                    .map(|(first, last)| 0.5 * (first.t + last.t))
                else {
                    continue;
                };
                let mut arrow: Option<(f64, Pos2, Pos2)> = None;
                for segment in span.samples.windows(2) {
                    let a = self.screen(segment[0].point, r);
                    let b = self.screen(segment[1].point, r);
                    let normal = side_offset(b - a, self.selected_side);
                    if normal == egui::Vec2::ZERO {
                        continue;
                    }
                    let normal = normal * offset;
                    painter
                        .line_segment([a + normal, b + normal], Stroke::new(3.0, Color32::WHITE));
                    let score = (0.5 * (segment[0].t + segment[1].t) - middle).abs();
                    if arrow.is_none_or(|(best, _, _)| score < best) {
                        arrow = Some((score, a, b));
                    }
                }
                if let Some((_, start, end)) = arrow {
                    let tangent = end - start;
                    let direction = tangent / tangent.length();
                    let normal = egui::vec2(-direction.y, direction.x);
                    let center = start + 0.5 * tangent;
                    painter.line_segment(
                        [center - direction * 7.0, center + direction * 7.0],
                        Stroke::new(1.5, GOLD),
                    );
                    let tip = center + direction * 7.0;
                    for barb in [normal, -normal] {
                        painter.line_segment(
                            [tip, tip - direction * 5.0 + barb * 3.0],
                            Stroke::new(1.5, GOLD),
                        );
                    }
                }
            }
        }
        if interactive && self.editor.document.presentation.handles {
            // A control belongs to a selection only through a selected span it
            // shapes; the rest of a long curve stays quiet.
            let owned_controls = self
                .selection
                .spans()
                .filter(|_| !self.capturing())
                .map(|spans| {
                    selected_span_controls(&self.editor.document.model.draft.geometry, spans)
                })
                .unwrap_or_default();
            for handle in &sampled.handles {
                let active = !self.capturing()
                    && matches!(self.selection, TopologySelection::Handle(value) if value == handle.handle);
                let owned = matches!(
                    handle.handle,
                    TopologyHandle::Control { curve, control } if owned_controls.contains(&(curve, control))
                );
                let point = self.screen(handle.point, r);
                let junction = matches!(handle.handle, TopologyHandle::Junction(_));
                let radius = match (junction, active) {
                    (true, true) => 7.0,
                    (true, false) => 5.5,
                    (false, true) => 6.0,
                    (false, false) => 4.0,
                };
                let fill = if active || junction {
                    GOLD
                } else {
                    Color32::from_rgb(23, 34, 44)
                };
                let ring = if active || owned {
                    SELECT
                } else if junction {
                    Color32::WHITE
                } else {
                    Color32::from_rgb(106, 133, 150)
                };
                painter.circle_filled(point, radius, fill);
                painter.circle_stroke(point, radius, Stroke::new(1.5, ring));
            }
        }
    }
    /// True while a PNG snapshot or a video frame is being taken. The capture
    /// crops to the viewport, so panels and the status bar are outside it by
    /// construction; what has to go is the chrome drawn inside the crop and the
    /// windows that float over it. `browser-checks.md` sets the rule: the field,
    /// the active View overlays, probes and the logo stay, while readouts,
    /// selection emphasis, gizmos, marquees and tool prompts do not.
    pub(super) fn capturing(&self) -> bool {
        matches!(
            self.snapshot_state,
            SnapshotState::Armed | SnapshotState::Capturing
        ) || matches!(
            self.recording_state,
            RecordingState::Preparing | RecordingState::Starting | RecordingState::Recording
        )
    }

    pub(super) fn span_selected(&self, target: TopologySpanTarget) -> bool {
        !self.capturing()
            && matches!(&self.selection, TopologySelection::Spans(spans) if spans.contains(&target))
    }
    pub(super) fn span_condition_colors(
        &self,
        target: TopologySpanTarget,
    ) -> Option<(Color32, Color32)> {
        match target {
            TopologySpanTarget::Outer(side) => {
                let color = outer_condition_color(
                    self.editor.document.model.draft.outer_boundaries.sides[side.index()],
                );
                Some((color, color))
            }
            TopologySpanTarget::Curve(id) => self
                .editor
                .document
                .model
                .draft
                .geometry
                .curves
                .iter()
                .flat_map(|curve| &curve.spans)
                .find(|span| span.id == id)
                .map(|span| match span.behavior {
                    SpanBehavior::Transmitting => (TEAL, TEAL),
                    SpanBehavior::Separated { left, right, .. } => {
                        (face_condition_color(left), face_condition_color(right))
                    }
                }),
        }
    }
    /// Probe markers and their names. Sizes and strokes follow the pre-topology
    /// viewport so every probe keeps a grabbable badge and a readable label.
    pub(super) fn draw_probes(&self, painter: &egui::Painter, r: Rect) {
        for probe in &self.editor.document.model.probes {
            if !self.probe_visible(&probe.target) {
                continue;
            }
            let selected = !self.capturing() && self.selected_probe == Some(probe.id);
            let color = self.probe_color(probe);
            match &probe.target {
                TopologyProbeTarget::Point(position) => {
                    let center = self.screen(*position, r);
                    painter.circle_filled(center, if selected { 6.0 } else { 4.5 }, color);
                    painter.circle_stroke(
                        center,
                        if selected { 9.0 } else { 7.0 },
                        Stroke::new(if selected { 2.0 } else { 1.3 }, Color32::WHITE),
                    );
                }
                TopologyProbeTarget::Segment { start, end, .. } => {
                    let a = self.screen(*start, r);
                    let b = self.screen(*end, r);
                    painter
                        .line_segment([a, b], Stroke::new(if selected { 3.0 } else { 2.0 }, color));
                    let midpoint = a + (b - a) * 0.5;
                    let direction = b - a;
                    let length = direction.length().max(1.0);
                    let normal = egui::vec2(direction.y, -direction.x) / length;
                    painter.arrow(midpoint, normal * 18.0, Stroke::new(1.5, color));
                    for endpoint in [a, b] {
                        painter.circle_filled(endpoint, if selected { 5.0 } else { 4.0 }, color);
                        painter.circle_stroke(
                            endpoint,
                            if selected { 7.0 } else { 6.0 },
                            Stroke::new(1.5, Color32::WHITE),
                        );
                    }
                }
                TopologyProbeTarget::Boundary(target) => {
                    let path = self.boundary_probe_polyline(target);
                    let width = if selected { 4.0 } else { 2.5 };
                    for pair in path.windows(2) {
                        painter.line_segment(
                            [self.screen(pair[0], r), self.screen(pair[1], r)],
                            Stroke::new(width, color),
                        );
                    }
                    let ring = if selected { 9.0 } else { 7.5 };
                    if let Some(badge) = Self::polyline_midpoint(&path) {
                        let badge = self.screen(badge, r);
                        painter.circle_filled(badge, if selected { 7.0 } else { 5.5 }, color);
                        painter.circle_stroke(badge, ring, Stroke::new(1.5, Color32::WHITE));
                    }
                    // A span's two traces lie on top of each other, so the
                    // scene has to say which one is read. A stem stands on that
                    // side and runs into the badge, so the reading arrives from
                    // the side the stem sits on, travelling the way positive
                    // flux points; the arclength axis leaves the middle of that
                    // stem, which keeps one mark rather than two that happen to
                    // touch.
                    if let Some((point, outward, along)) =
                        boundary_probe_orientation(&path, target.side, target.reversed)
                    {
                        let reach = 18.0;
                        let normal = screen_direction(outward);
                        // Landing the head on the ring ties the stem to the
                        // marker rather than leaving it beside the path.
                        let tail = self.screen(point, r) - normal * (ring + reach);
                        let stroke = Stroke::new(if selected { 2.0 } else { 1.5 }, color);
                        painter.arrow(tail, normal * reach, stroke);
                        painter.arrow(
                            tail + normal * (reach * 0.5),
                            screen_direction(along) * reach,
                            stroke,
                        );
                    }
                }
                TopologyProbeTarget::AreaDisk { center, radius } => {
                    let center = self.screen(*center, r);
                    let radius = (radius * self.scale) as f32;
                    painter.circle_filled(
                        center,
                        radius,
                        Color32::from_rgba_unmultiplied(
                            color.r(),
                            color.g(),
                            color.b(),
                            if selected { 32 } else { 18 },
                        ),
                    );
                    painter.circle_stroke(
                        center,
                        radius,
                        Stroke::new(if selected { 2.5 } else { 1.5 }, color),
                    );
                    painter.circle_filled(center, if selected { 5.0 } else { 3.5 }, color);
                    let handle = center + egui::vec2(radius, 0.0);
                    painter.circle_filled(handle, if selected { 5.0 } else { 4.0 }, color);
                    painter.circle_stroke(
                        handle,
                        if selected { 7.0 } else { 6.0 },
                        Stroke::new(1.5, Color32::WHITE),
                    );
                }
                TopologyProbeTarget::AreaRegion(region) => {
                    if let Some(sampled) = &self.sampled {
                        for span in &sampled.spans {
                            if !self.span_bounds_region(span.target, *region) {
                                continue;
                            }
                            for pair in span.samples.windows(2) {
                                painter.line_segment(
                                    [self.screen(pair[0].point, r), self.screen(pair[1].point, r)],
                                    Stroke::new(if selected { 4.0 } else { 2.5 }, color),
                                );
                            }
                        }
                    }
                    if let Some(anchor) = self.probe_anchors.get(&probe.id).copied() {
                        let center = self.screen(anchor, r);
                        painter.circle_filled(center, if selected { 9.0 } else { 7.0 }, color);
                        painter.circle_stroke(
                            center,
                            if selected { 11.0 } else { 9.0 },
                            Stroke::new(1.5, Color32::WHITE),
                        );
                        painter.text(
                            center,
                            egui::Align2::CENTER_CENTER,
                            "A",
                            egui::FontId::monospace(9.0),
                            Color32::WHITE,
                        );
                    }
                }
            }
            if !self.editor.document.presentation.probe_labels {
                continue;
            }
            if let Some(badge) = self.probe_badge(probe) {
                let origin = self.screen(badge, r) + egui::vec2(10.0, -10.0);
                let label = painter.layout_no_wrap(
                    probe.name.clone(),
                    egui::FontId::monospace(10.0),
                    color,
                );
                let rect = Rect::from_min_size(
                    Pos2::new(origin.x, origin.y - label.size().y),
                    label.size(),
                );
                painter.rect_filled(
                    rect.expand(2.0),
                    2.0,
                    Color32::from_rgba_unmultiplied(8, 13, 18, 170),
                );
                painter.galley(rect.min, label, color);
            }
        }
    }
    /// Whether a compiled span has the given region on either side, used to
    /// outline the face a region probe integrates over.
    /// The compiled face the Materials panel has selected, with its region, or
    /// `None` when that assignment no longer resolves.
    pub(super) fn selected_face(&self) -> Option<(FaceId, Option<RegionId>)> {
        let compiled = self
            .editor
            .compiled_draft
            .as_ref()
            .unwrap_or(&self.editor.compiled_accepted);
        let assignment = self
            .editor
            .document
            .model
            .draft
            .face_assignments
            .get(self.face_selection)?;
        let face = assignment.anchor.resolve(&compiled.topology).ok()?;
        Some((face, assignment.region))
    }

    /// Which assignment owns the face under a point, by document index, so a
    /// viewport click can select a hole as readily as a subdomain.
    pub(super) fn draft_face_assignment_at(&self, point: Point2) -> Option<usize> {
        let compiled = self
            .editor
            .compiled_draft
            .as_ref()
            .unwrap_or(&self.editor.compiled_accepted);
        let face = compiled.topology.face_at(point)?;
        self.editor
            .document
            .model
            .draft
            .face_assignments
            .iter()
            .position(|assignment| assignment.anchor.resolve(&compiled.topology) == Ok(face))
    }

    pub(super) fn span_bounds_face(&self, target: TopologySpanTarget, face: FaceId) -> bool {
        let compiled = self
            .editor
            .compiled_draft
            .as_ref()
            .unwrap_or(&self.editor.compiled_accepted);
        compiled.topology.edges.iter().any(|edge| {
            let matches_target = match target {
                TopologySpanTarget::Outer(side) => edge.source == CompiledEdgeSource::Outer(side),
                TopologySpanTarget::Curve(span) => edge.source == CompiledEdgeSource::Curve(span),
            };
            matches_target && (edge.left == face || edge.right == face)
        })
    }

    pub(super) fn span_bounds_region(&self, target: TopologySpanTarget, region: RegionId) -> bool {
        let Some(active) = self.runtime.active() else {
            return false;
        };
        let Some(face) = active
            .bundle
            .plan
            .domains
            .iter()
            .find(|domain| domain.region == region)
            .map(|domain| domain.face)
        else {
            return false;
        };
        active.bundle.snapshot.edges.iter().any(|edge| {
            let matches_target = match target {
                TopologySpanTarget::Outer(side) => edge.source == CompiledEdgeSource::Outer(side),
                TopologySpanTarget::Curve(span) => edge.source == CompiledEdgeSource::Curve(span),
            };
            matches_target && (edge.left == face || edge.right == face)
        })
    }
    pub(super) fn draw_markers(&self, painter: &egui::Painter, r: Rect) {
        let p = self.editor.document.presentation;
        let source = self.editor.document.model.source;
        if source.enabled {
            let center = self.screen(source.position, r);
            painter.circle_stroke(center, 7.0, Stroke::new(2.0, GOLD));
            for offset in [egui::vec2(10.0, 0.0), egui::vec2(0.0, 10.0)] {
                painter.line_segment([center - offset, center + offset], Stroke::new(1.0, GOLD));
            }
        }
        let show_region = !self.capturing()
            && (self.inspector == Some(InspectorPanel::Materials)
                || matches!(
                    p.material_overlay,
                    MaterialOverlay::Regions | MaterialOverlay::Subdomains
                ));
        // While the Materials panel lists faces, outline the selected face
        // instead of the selected region, so a hole can be picked out too.
        let selected_face = (self.inspector == Some(InspectorPanel::Materials)
            && self.subdomain_listing == SubdomainListing::Faces)
            .then(|| self.selected_face())
            .flatten();
        if show_region && let Some(sampled) = &self.sampled {
            let color = match selected_face {
                Some((_, Some(region))) => {
                    subdomain_color(&self.editor.document.model.draft, region, 1.0)
                }
                Some((_, None)) => Color32::from_gray(150),
                None => subdomain_color(
                    &self.editor.document.model.draft,
                    self.region_selection,
                    1.0,
                ),
            };
            for span in &sampled.spans {
                let bounds = match selected_face {
                    Some((face, _)) => self.span_bounds_face(span.target, face),
                    None => self.span_bounds_region(span.target, self.region_selection),
                };
                if !bounds {
                    continue;
                }
                for pair in span.samples.windows(2) {
                    painter.line_segment(
                        [self.screen(pair[0].point, r), self.screen(pair[1].point, r)],
                        Stroke::new(4.0, color),
                    );
                }
            }
        }
        self.draw_probes(painter, r);
        if p.far_field_contour && self.editor.document.model.far_field.enabled {
            let d = self.editor.document.model.draft.geometry.domain;
            let inset = self.editor.document.model.far_field.inset;
            let min = self.screen(Point2::new(d.min_x + inset, d.max_y - inset), r);
            let max = self.screen(Point2::new(d.max_x - inset, d.min_y + inset), r);
            painter.rect_stroke(
                Rect::from_min_max(min, max),
                0.0,
                Stroke::new(1.0, GOLD),
                egui::StrokeKind::Middle,
            );
            painter.text(
                Pos2::new(min.x + 5.0, min.y + 4.0),
                egui::Align2::LEFT_TOP,
                "FF",
                egui::FontId::proportional(12.0),
                GOLD,
            );
        }
        if !self.capturing()
            && let Some(DragGesture::Marquee {
                start,
                current,
                operation,
                ..
            }) = &self.drag
        {
            let marquee = Rect::from_two_pos(*start, *current).intersect(r);
            let operation_color = match operation {
                MarqueeOperation::Replace => SELECT,
                MarqueeOperation::Add => GOLD,
                MarqueeOperation::Subtract => RED,
            };
            let containment = MarqueeContainment::from_drag(*start, *current);
            let border_color = match containment {
                MarqueeContainment::Enclosed => SELECT,
                MarqueeContainment::Crossing => TEAL,
            };
            painter.rect_filled(
                marquee,
                0.0,
                Color32::from_rgba_unmultiplied(
                    operation_color.r(),
                    operation_color.g(),
                    operation_color.b(),
                    24,
                ),
            );
            if containment == MarqueeContainment::Enclosed {
                painter.rect_stroke(
                    marquee,
                    0.0,
                    Stroke::new(1.0, border_color),
                    egui::StrokeKind::Inside,
                );
            } else {
                let corners = [
                    marquee.left_top(),
                    marquee.right_top(),
                    marquee.right_bottom(),
                    marquee.left_bottom(),
                    marquee.left_top(),
                ];
                painter.extend(egui::Shape::dashed_line(
                    &corners,
                    Stroke::new(1.0, border_color),
                    5.0,
                    3.0,
                ));
            }
            painter.text(
                marquee.left_top() + egui::vec2(4.0, 4.0),
                egui::Align2::LEFT_TOP,
                format!("{} · {}", operation.label(), containment.label()),
                egui::FontId::monospace(9.0),
                border_color,
            );
        }
        if let Some(draw) = &self.draw.as_ref().filter(|_| !self.capturing()) {
            self.draw_attachment_targets(painter, r, draw);
            let points = draw
                .points
                .iter()
                .map(|p| self.screen(*p, r))
                .collect::<Vec<_>>();
            for p in &points {
                painter.circle_filled(*p, 4.0, GOLD);
            }
            if points.len() > 1 {
                painter.add(egui::Shape::line(points.clone(), Stroke::new(1.5, GOLD)));
            }
            if let (Some(last), Some(pointer)) = (points.last(), painter.ctx().pointer_hover_pos())
            {
                // Where the click will land, not where the cursor is.
                let target = self
                    .draw_attachment_hit(ScreenPoint::new(pointer.x as f64, pointer.y as f64), r)
                    .map(|hit| self.screen(hit.point, r))
                    .unwrap_or_else(|| {
                        if painter.ctx().input(|input| input.modifiers.shift) {
                            self.screen(
                                Self::snap_point(self.world(pointer, r), self.snap_step()),
                                r,
                            )
                        } else {
                            pointer
                        }
                    });
                painter.line_segment(
                    [*last, target],
                    Stroke::new(1.2, Color32::from_rgba_unmultiplied(248, 196, 112, 180)),
                );
            }
        }
        if let Some(pointer) = painter.ctx().pointer_hover_pos() {
            let current = self.world(pointer, r);
            // Where the click will land, not where the cursor is.
            let snap = painter.ctx().input(|input| input.modifiers.shift);
            match self.probe_mode {
                Some(ProbePlacement::Segment { start: Some(start) }) => {
                    let end = if snap {
                        self.screen(Self::snap_point(current, self.snap_step()), r)
                    } else {
                        pointer
                    };
                    painter.line_segment([self.screen(start, r), end], Stroke::new(1.5, TEAL));
                }
                Some(ProbePlacement::Disk {
                    center: Some(center),
                }) => {
                    painter.circle_stroke(
                        self.screen(center, r),
                        (Self::placed_disk_radius(center, current, snap, self.snap_step())
                            * self.scale) as f32,
                        Stroke::new(1.5, TEAL),
                    );
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use funfern_app::document::VectorOverlay;
    use funfern_app::topology_editor::TopologyEditor;

    #[test]
    fn vector_overlay_layout_is_world_anchored_and_pan_stable() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        let active = activate_at(&mut state, 0.08);
        let viewport = viewport();
        let spacing = 28.0;
        let (world_spacing, visible_bins) =
            vector_overlay_lattice(state.scale, spacing, state.center, viewport).unwrap();
        let points = vector_overlay_layout(
            active.fixed_model(),
            &active.mesh,
            &active.operator,
            world_spacing,
            visible_bins,
        );
        assert!(!points.is_empty());
        let keys = points
            .iter()
            .map(|point| {
                (
                    (point.point.x / world_spacing).floor() as i64,
                    (point.point.y / world_spacing).floor() as i64,
                )
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(keys.len(), points.len());
        let maximum_bins = (visible_bins[1] - visible_bins[0] + 1) as usize
            * (visible_bins[3] - visible_bins[2] + 1) as usize;
        assert!(points.len() <= maximum_bins);
        assert!(points.iter().all(|point| {
            point.point.x.is_finite()
                && point.point.y.is_finite()
                && point.stencil.element < active.mesh.triangles.len() as u32
        }));

        let shifted_center = state.center + Point2::new(world_spacing * 0.35, 0.0);
        let (shifted_spacing, shifted_bins) =
            vector_overlay_lattice(state.scale, spacing, shifted_center, viewport).unwrap();
        assert_eq!(shifted_spacing, world_spacing);
        let shifted = vector_overlay_layout(
            active.fixed_model(),
            &active.mesh,
            &active.operator,
            shifted_spacing,
            shifted_bins,
        );
        let common = [
            visible_bins[0].max(shifted_bins[0]) + 1,
            visible_bins[1].min(shifted_bins[1]) - 1,
            visible_bins[2].max(shifted_bins[2]) + 1,
            visible_bins[3].min(shifted_bins[3]) - 1,
        ];
        let interior = |points: &[VectorOverlayLayoutPoint]| {
            points
                .iter()
                .filter_map(|point| {
                    let key = (
                        (point.point.x / world_spacing).floor() as i64,
                        (point.point.y / world_spacing).floor() as i64,
                    );
                    (key.0 >= common[0]
                        && key.0 <= common[1]
                        && key.1 >= common[2]
                        && key.1 <= common[3])
                        .then_some((key, point.element))
                })
                .collect::<BTreeMap<_, _>>()
        };
        let before = interior(&points);
        assert!(!before.is_empty());
        assert_eq!(before, interior(&shifted));
    }

    /// The authored model refuses a law-carrying material, so a lattice that
    /// evaluated it dropped every element of a driven region and drew no arrow
    /// there. It reads the base the operator was compiled against instead.
    #[test]
    fn a_driven_region_gets_its_arrows() {
        let mut document = TopologyEditor::default().document;
        let drive = TimeDrive::ParametricPump {
            depth: ScalarField::constant(0.2),
            frequency_hz: ScalarField::constant(1.0),
            phase_radians: ScalarField::constant(0.25),
        };
        document.model.draft.materials[0].mass_law.drive = drive.clone();
        document.model.accepted.materials[0].mass_law.drive = drive;
        let mut state = Playground {
            editor: TopologyEditor::from_document(document).unwrap(),
            ..Playground::default()
        };
        let active = activate_at(&mut state, 0.08);
        assert!(active.driven());
        let (world_spacing, visible_bins) =
            vector_overlay_lattice(state.scale, 28.0, state.center, viewport()).unwrap();
        let points = vector_overlay_layout(
            active.fixed_model(),
            &active.mesh,
            &active.operator,
            world_spacing,
            visible_bins,
        );
        let regions = |elements: &mut dyn Iterator<Item = usize>| {
            elements
                .map(|element| active.mesh.triangles[element].region)
                .collect::<BTreeSet<_>>()
        };
        let meshed = regions(&mut (0..active.mesh.triangles.len()));
        let covered = regions(&mut points.iter().map(|point| point.element as usize));
        assert!(!points.is_empty());
        assert_eq!(covered, meshed, "every meshed region carries arrows");
        // The coefficients are the base ones the operator was built from.
        let base = active.bundle.model().to_owned().without_temporal_laws();
        for point in &points {
            let expected = base
                .as_model()
                .directional_material_at(point.stencil.region, point.point)
                .unwrap();
            assert_eq!(point.stencil.mass_density, expected.mass_density);
            assert_eq!(point.stencil.stiffness, expected.stiffness);
        }
    }

    #[test]
    fn vector_overlay_keeps_the_mechanical_skin_on_physical_observables() {
        assert_eq!(
            VectorOverlay::choices(PhysicsModel::Mechanical),
            &[VectorOverlay::Off, VectorOverlay::RelativeEnergyFlow]
        );
        assert_eq!(
            VectorOverlay::choices(PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            })
            .len(),
            3
        );
        assert_eq!(
            VectorOverlay::ComplementaryField.resolved(PhysicsModel::Mechanical),
            VectorOverlay::Off
        );
        assert_eq!(
            VectorOverlay::ComplementaryField.resolved(PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            }),
            VectorOverlay::ComplementaryField
        );
        assert_eq!(
            VectorOverlay::ComplementaryField.label(PhysicsModel::Mechanical),
            "Off"
        );
    }
    #[test]
    fn arrow_ac_coupling_rejects_static_state_in_simulation_time() {
        let mut state = Playground::default();
        let sample = |value| {
            let value = Point2::new(value, 0.0);
            vec![(0, Pos2::ZERO, value, value)]
        };

        let mut first = sample(1.0);
        state.ac_couple_vector_samples(&mut first, 0, 0.0, false);
        assert_eq!(first[0].2.x, 0.0);

        let mut after_one_second = sample(1.0);
        state.ac_couple_vector_samples(&mut after_one_second, 100, 1.0, false);
        assert_eq!(after_one_second[0].2.x, 0.0);

        let mut after_two_seconds = sample(1.0);
        state.ac_couple_vector_samples(&mut after_two_seconds, 200, 2.0, false);
        assert_eq!(after_two_seconds[0].2.x, 0.0);
    }

    #[test]
    fn amr_generation_change_orphans_an_in_flight_arrow_lattice() {
        assert!(vector_overlay_revision_owned(7, 12, 7, 12, true));
        assert!(
            !vector_overlay_revision_owned(7, 12, 8, 12, true),
            "AMR generation incorrectly kept the stale zoom readback alive"
        );
        assert!(!vector_overlay_revision_owned(7, 12, 7, 13, true));
        assert!(!vector_overlay_revision_owned(7, 12, 7, 12, false));
    }

    #[test]
    fn resident_filter_boundaries_are_not_ordinary_time_endpoints() {
        assert!(!resident_filter_boundary(true, 0));
        assert!(!resident_filter_boundary(true, 15));
        assert!(resident_filter_boundary(true, 16));
        assert!(resident_filter_boundary(true, 32));
        assert!(!resident_filter_boundary(false, 16));
    }

    #[test]
    fn arrow_ac_coupling_does_not_turn_filter_maintenance_into_a_wave() {
        let mut state = Playground::default();
        let mut first = vec![(0, Pos2::ZERO, Point2::default(), Point2::default())];
        state.ac_couple_vector_samples(&mut first, 14, 0.14, false);

        let mut ordinary = vec![(0, Pos2::ZERO, Point2::new(0.2, 0.0), Point2::new(0.2, 0.0))];
        state.ac_couple_vector_samples(&mut ordinary, 15, 0.15, false);
        assert!((0.19..0.21).contains(&ordinary[0].2.x));

        // Deliberately exaggerated maintenance correction. Feeding the raw
        // input jump to the high-pass would produce an arrow near 9.0.
        let mut filtered = vec![(0, Pos2::ZERO, Point2::new(9.0, 0.0), Point2::new(0.2, 0.0))];
        state.ac_couple_vector_samples(&mut filtered, 16, 0.16, true);
        assert!((0.19..0.21).contains(&filtered[0].2.x));

        let mut after = vec![(0, Pos2::ZERO, Point2::new(9.1, 0.0), Point2::new(9.1, 0.0))];
        state.ac_couple_vector_samples(&mut after, 17, 0.17, false);
        assert!((0.28..0.31).contains(&after[0].2.x));
    }

    #[test]
    fn arrow_ac_coupling_keeps_evolution_before_filter_maintenance() {
        let mut state = Playground::default();
        let mut first = vec![(0, Pos2::ZERO, Point2::new(0.2, 0.0), Point2::new(0.2, 0.0))];
        state.ac_couple_vector_samples(&mut first, 15, 0.15, false);

        // The ordinary endpoint advanced to 0.5 before an intentionally huge
        // same-time maintenance correction moved the accepted state to 9.0.
        // The wave increment must survive while the correction itself does not.
        let mut filtered = vec![(0, Pos2::ZERO, Point2::new(9.0, 0.0), Point2::new(0.5, 0.0))];
        state.ac_couple_vector_samples(&mut filtered, 16, 0.16, true);
        assert!((0.29..0.31).contains(&filtered[0].2.x));
    }

    #[test]
    fn arrow_ac_coupling_tracks_physical_samples_and_cold_starts_new_ones() {
        let mut state = Playground::default();
        let mut first = vec![(7, Pos2::ZERO, Point2::new(1.0, 0.0), Point2::new(1.0, 0.0))];
        state.ac_couple_vector_samples(&mut first, 0, 0.0, false);
        assert_eq!(first[0].2, Point2::default());

        // The same mesh element carries temporal history even if its screen
        // cell changes. A genuinely new element does not inherit that history
        // or flash its unknown baseline into the AC view.
        let mut moved = vec![
            (
                7,
                Pos2::new(80.0, 40.0),
                Point2::new(1.5, 0.0),
                Point2::new(1.5, 0.0),
            ),
            (11, Pos2::ZERO, Point2::new(9.0, 0.0), Point2::new(9.0, 0.0)),
        ];
        state.ac_couple_vector_samples(&mut moved, 1, 0.01, false);
        assert!(moved[0].2.x > 0.49, "lost physical-sample history");
        assert_eq!(moved[1].2, Point2::default(), "new sample flashed DC");

        // A lazily retained element uses its own last accepted step when it
        // returns to view; time spent off-screen still decays its baseline.
        let mut elsewhere = vec![(11, Pos2::ZERO, Point2::new(9.0, 0.0), Point2::new(9.0, 0.0))];
        state.ac_couple_vector_samples(&mut elsewhere, 100, 1.0, false);
        let mut returned = vec![(7, Pos2::ZERO, Point2::new(1.5, 0.0), Point2::new(1.5, 0.0))];
        state.ac_couple_vector_samples(&mut returned, 101, 1.01, false);
        assert!(
            (0.29..0.31).contains(&returned[0].2.x),
            "off-screen time was lost: {}",
            returned[0].2.x
        );
    }

    #[test]
    fn arrow_ac_history_survives_a_compatible_generation_handoff() {
        let mut state = Playground::default();
        let owner = VectorOverlayAcOwner {
            mesh_revision: 17,
            physics: PhysicsModel::Mechanical,
        };
        state.retain_vector_overlay_ac_owner(owner, &[], 100, 2.0, 80.0);
        let mut first = vec![(3, Pos2::ZERO, Point2::new(1.0, 0.0), Point2::new(1.0, 0.0))];
        state.ac_couple_vector_samples(&mut first, 100, 2.0, false);
        let mut changing = vec![(3, Pos2::ZERO, Point2::new(1.4, 0.0), Point2::new(1.4, 0.0))];
        state.ac_couple_vector_samples(&mut changing, 110, 2.1, false);
        assert!(changing[0].2.x > 0.39);

        // A GPU generation is deliberately absent from the owner. Rebinding
        // material-dependent stencils on the same mesh therefore retains the
        // temporal baseline and continues at the transferred absolute time.
        state.retain_vector_overlay_ac_owner(owner, &[], 120, 2.2, 80.0);
        let mut after_handoff = vec![(3, Pos2::ZERO, Point2::new(1.5, 0.0), Point2::new(1.5, 0.0))];
        state.ac_couple_vector_samples(&mut after_handoff, 120, 2.2, false);
        assert!(after_handoff[0].2.x > 0.45, "handoff cold-started arrows");

        state.retain_vector_overlay_ac_owner(
            VectorOverlayAcOwner {
                mesh_revision: 18,
                ..owner
            },
            &[],
            120,
            2.2,
            80.0,
        );
        let mut after_remesh = vec![(3, Pos2::ZERO, Point2::new(1.5, 0.0), Point2::new(1.5, 0.0))];
        state.ac_couple_vector_samples(&mut after_remesh, 120, 2.2, false);
        assert_eq!(after_remesh[0].2, Point2::default());
    }

    #[test]
    fn arrow_ac_history_is_spatially_rebased_across_a_remesh() {
        let mut state = Playground::default();
        let old_owner = VectorOverlayAcOwner {
            mesh_revision: 17,
            physics: PhysicsModel::Mechanical,
        };
        state.retain_vector_overlay_ac_owner(old_owner, &[], 100, 2.0, 80.0);
        let mut first = vec![(
            3,
            Pos2::new(40.0, 50.0),
            Point2::new(1.0, 0.0),
            Point2::new(1.0, 0.0),
        )];
        state.ac_couple_vector_samples(&mut first, 100, 2.0, false);
        let mut changing = vec![(
            3,
            Pos2::new(40.0, 50.0),
            Point2::new(1.4, 0.0),
            Point2::new(1.4, 0.0),
        )];
        state.ac_couple_vector_samples(&mut changing, 110, 2.1, false);
        let before = changing[0].2;

        let new_samples = vec![(
            91,
            Pos2::new(43.0, 48.0),
            Point2::new(1.45, 0.0),
            Point2::new(1.45, 0.0),
        )];
        state.retain_vector_overlay_ac_owner(
            VectorOverlayAcOwner {
                mesh_revision: 18,
                ..old_owner
            },
            &new_samples,
            110,
            2.1,
            80.0,
        );
        let mut accepted = new_samples;
        state.ac_couple_vector_samples(&mut accepted, 110, 2.1, false);
        assert_eq!(accepted[0].2, before, "remesh blinked the AC arrows");
        assert_eq!(state.vector_overlay_ac_state[&91].input.x, 1.45);
    }

    #[test]
    fn sparse_arrow_outliers_cannot_defeat_the_global_quiet_fade() {
        let maximum = 55.2;
        let visibility = VECTOR_OVERLAY_VISIBILITY_CUTOFF;
        let ordinary = vector_arrow_length(1.0, 1.0, 1.0, maximum, visibility);
        let outlier = vector_arrow_length(1.0e12, 1.0, 5.0, maximum, visibility);
        assert!((ordinary - outlier).abs() < f32::EPSILON);
        assert!(outlier < 0.11, "quiet outlier still spans {outlier} px");
    }

    #[test]
    fn arrow_ac_coupling_preserves_an_ordinary_source_frequency() {
        let mut state = Playground {
            uploaded_time_step: 1.0 / 600.0,
            ..Playground::default()
        };
        let frequency = 3.0;
        let mut input_square = 0.0;
        let mut output_square = 0.0;
        // The solver advances ten small steps between display-rate samples.
        for frame in 0..600_u64 {
            let step = frame * 10;
            let time = step as f64 * state.uploaded_time_step;
            let scalar = (std::f64::consts::TAU * frequency * time).sin();
            let value = Point2::new(scalar, 0.0);
            let mut samples = vec![(0, Pos2::ZERO, value, value)];
            state.ac_couple_vector_samples(&mut samples, step, time, false);
            if frame >= 300 {
                input_square += scalar * scalar;
                output_square += samples[0].2.x * samples[0].2.x;
            }
        }
        let retained = (output_square / input_square).sqrt();
        assert!(retained > 0.999, "3 Hz amplitude retention {retained}");
    }

    #[test]
    fn arrow_ac_coupling_does_not_leave_a_mean_on_a_dc_offset_sine() {
        let mut state = Playground::default();
        let sample_rate = 60.0;
        let frequency = 2.3;
        let mut mean = 0.0;
        let mut count = 0_u64;
        for frame in 0..1_800_u64 {
            let time = frame as f64 / sample_rate;
            let scalar = 4.0 + (std::f64::consts::TAU * frequency * time).sin();
            let value = Point2::new(scalar, 0.0);
            let mut samples = vec![(0, Pos2::ZERO, value, value)];
            state.ac_couple_vector_samples(&mut samples, frame, time, false);
            if time >= 20.0 {
                mean += samples[0].2.x;
                count += 1;
            }
        }
        mean /= count as f64;
        assert!(mean.abs() < 1.0e-4, "high-pass mean was {mean}");
    }
}
