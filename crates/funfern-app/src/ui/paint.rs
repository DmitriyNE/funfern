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
            // scalar before exposure. On an oscillator, the integrated field
            // `r` when that view is chosen: its own live stream, which a
            // handoff's receipt already seeds for the new generation, and the
            // latest full snapshot until the first copy lands.
            let nodes = active.operator.degrees_of_freedom();
            let integrated_shown = self.integrated_field_shown();
            self.follow_field_quantity(integrated_shown);
            let integrated = integrated_shown
                .then(|| {
                    [&display.live_integrated, &display.snapshot_integrated]
                        .into_iter()
                        .find(|values| values.len() == nodes)
                })
                .flatten();
            let field_values: Arc<[f32]> = match integrated {
                Some(values) => values.clone().into(),
                None => display.current.clone().into(),
            };
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
            let owner = layout.map(|layout| VectorOverlayFilterOwner {
                mesh_revision: layout.key.mesh_revision,
                physics: layout.key.physics,
            });
            // Only the streamline lattice is a regular grid the lines can
            // read; an arrow layout still in use after a style change is
            // drawn as the arrows it is.
            let lattice = layout.and_then(|layout| {
                (layout.key.style == VectorOverlayStyle::Streamlines)
                    .then_some(layout.key.world_spacing)
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
                                point.key,
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
                r,
                samples,
                lattice,
                owner.unwrap_or(VectorOverlayFilterOwner {
                    mesh_revision: active.mesh.mesh_revision,
                    physics: active.bundle.authored.physics,
                }),
                vector_display.completed_steps,
                vector_display.absolute_time,
            );
        } else {
            self.clear_vector_overlay_filter();
            self.vector_overlay_filter = None;
        }
    }

    /// Drops every arrow's filter history and the mesh it belonged to, and
    /// the streamlines' seeds with them.
    pub(super) fn clear_vector_overlay_filter(&mut self) {
        self.vector_overlay_filter_state.clear();
        self.vector_overlay_filter_owner = None;
        self.vector_overlay_filter_step = u64::MAX;
        self.vector_overlay_streamlines.lines.clear();
        self.streamline_dash_time = None;
    }

    /// Filters and draws the overlay's samples, `(key, screen origin, value,
    /// pre-filter complementary)`. With `lattice`, the world spacing of a
    /// streamline lattice the samples are the cell centres of, the flow is
    /// drawn as streamlines where the style asks for them; otherwise as
    /// arrows.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_vector_overlay(
        &mut self,
        painter: &egui::Painter,
        viewport: Rect,
        mut samples: Vec<(u64, Pos2, Point2, Point2)>,
        lattice: Option<f64>,
        owner: VectorOverlayFilterOwner,
        completed_steps: u64,
        absolute_time: f64,
    ) {
        let settings = self.editor.document.presentation;
        let mode = settings
            .vector_overlay
            .resolved(self.editor.document.model.draft.physics);
        if self.vector_overlay_mode != mode {
            self.clear_vector_overlay_filter();
            self.vector_overlay_filter = None;
            self.vector_overlay_exposure.clear();
            self.vector_overlay_mode = mode;
        }
        let filter = match mode {
            VectorOverlay::ComplementaryField if settings.vector_overlay_ac_coupled => {
                Some(VectorFilter::AcCoupled)
            }
            VectorOverlay::RelativeEnergyFlow if settings.vector_overlay_lowpass => {
                Some(VectorFilter::LowPass)
            }
            _ => None,
        };
        if self.vector_overlay_filter != filter {
            self.clear_vector_overlay_filter();
            self.vector_overlay_exposure.clear();
            self.vector_overlay_filter = filter;
        }
        let magnitudes_of = |samples: &[(u64, Pos2, Point2, Point2)]| {
            samples
                .iter()
                .map(|(_, _, value, _)| value.norm())
                .filter(|magnitude| magnitude.is_finite() && *magnitude > 0.0)
                .collect::<Vec<_>>()
        };
        // The low-pass is exposed by what it is fed. Measured on its own
        // output, a region where the wave stands has next to no mean flow and
        // the scale renormalises the ripple the average leaves there back to
        // full length; measured on the raw samples, that region draws short,
        // which is the physics, and a travelling one draws about as long as
        // its peaks do unaveraged.
        let raw_magnitudes =
            (filter == Some(VectorFilter::LowPass)).then(|| magnitudes_of(&samples));
        let remap_radius = settings.vector_overlay_density * 1.5;
        match filter {
            Some(filter) => {
                self.retain_vector_overlay_filter_owner(
                    filter,
                    owner,
                    &samples,
                    completed_steps,
                    absolute_time,
                    remap_radius,
                );
                match filter {
                    VectorFilter::AcCoupled => self.ac_couple_vector_samples(
                        &mut samples,
                        completed_steps,
                        absolute_time,
                        resident_filter_boundary(self.grid_filter_running(), completed_steps),
                    ),
                    VectorFilter::LowPass => self.low_pass_vector_samples(
                        &mut samples,
                        completed_steps,
                        absolute_time,
                        std::f64::consts::TAU * settings.vector_overlay_lowpass_hz,
                        remap_radius,
                    ),
                }
            }
            None => {
                self.vector_overlay_filter_state.clear();
                self.vector_overlay_filter_owner = Some(owner);
                self.vector_overlay_filter_step = completed_steps;
            }
        }
        let mut magnitudes = raw_magnitudes.unwrap_or_else(|| magnitudes_of(&samples));
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
        if let Some(world_spacing) = lattice
            && settings.vector_overlay_style.resolved(mode) == VectorOverlayStyle::Streamlines
        {
            self.draw_streamlines(
                painter,
                viewport,
                &samples,
                world_spacing,
                reference,
                visibility,
                absolute_time,
            );
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

    /// The flow as streamlines through the lattice of filtered samples,
    /// placed a line spacing apart from the previous frame's seeds, each
    /// drawn as dashes drifting along the flow in simulated time, their
    /// alpha from the local magnitude as the arrows' length is.
    #[allow(clippy::too_many_arguments)]
    fn draw_streamlines(
        &mut self,
        painter: &egui::Painter,
        viewport: Rect,
        samples: &[(u64, Pos2, Point2, Point2)],
        world_spacing: f64,
        reference: f64,
        visibility: f64,
        absolute_time: f64,
    ) {
        let settings = self.editor.document.presentation;
        let scale = self.scale;
        if !(scale.is_finite() && scale > 0.0 && viewport.is_positive()) {
            return;
        }
        let Some(lattice) = VectorLattice::new(
            world_spacing,
            samples
                .iter()
                .map(|(_, origin, value, _)| (self.world(*origin, viewport), *value)),
        ) else {
            return;
        };
        let corners = [
            self.world(viewport.min, viewport),
            self.world(viewport.max, viewport),
        ];
        let view = WorldRect {
            min: Point2::new(
                corners[0].x.min(corners[1].x),
                corners[0].y.min(corners[1].y),
            ),
            max: Point2::new(
                corners[0].x.max(corners[1].x),
                corners[0].y.max(corners[1].y),
            ),
        };
        let separation = f64::from(settings.vector_overlay_density) / scale;
        let parameters = StreamlineParameters {
            separation,
            step: separation / 8.0,
            // The same floor the arrows are culled at.
            floor: reference * 0.015,
            maximum_length: f64::from(viewport.size().length()) / scale * 2.0,
        };
        self.vector_overlay_streamlines
            .place(&lattice, &parameters, &view);
        if absolute_time.is_finite() {
            if let Some(last) = self.streamline_dash_time {
                let elapsed = (absolute_time - last).clamp(0.0, 1.0);
                self.streamline_dash_phase = (self.streamline_dash_phase
                    + elapsed * STREAMLINE_DASH_SPEED)
                    .rem_euclid(STREAMLINE_DASH_PERIOD);
            }
            self.streamline_dash_time = Some(absolute_time);
        }
        let phase = self.streamline_dash_phase;
        let stroke_color = |alpha: u8| Color32::from_rgba_unmultiplied(116, 232, 210, alpha);
        for line in &self.vector_overlay_streamlines.lines {
            let screen = line
                .points
                .iter()
                .map(|point| {
                    let at = self.screen(*point, viewport);
                    Point2::new(f64::from(at.x), f64::from(at.y))
                })
                .collect::<Vec<_>>();
            let arcs = line
                .cumulative_arcs()
                .into_iter()
                .map(|arc| arc * scale)
                .collect::<Vec<_>>();
            let Some(length) = arcs.last().copied() else {
                continue;
            };
            for (from, to) in dash_intervals(
                length,
                arcs[line.seed_index],
                phase,
                STREAMLINE_DASH_ON,
                STREAMLINE_DASH_PERIOD,
            ) {
                let dash = section(&screen, &arcs, from, to);
                let (Some(first), Some(last)) = (dash.first(), dash.last()) else {
                    continue;
                };
                let middle = first.lerp(*last, 0.5);
                let magnitude = lattice
                    .sample(self.world(Pos2::new(middle.x as f32, middle.y as f32), viewport))
                    .map_or(0.0, Point2::norm);
                // The exposed strength, as the arrows' length is, over a
                // floor: a short arrow is still a full stroke, where a dash
                // at the same alpha would vanish, and the stop at the floor
                // has already removed what is too quiet to follow. The
                // whole overlay still fades out with the visibility.
                let exposed = vector_arrow_length(
                    magnitude,
                    reference,
                    settings.vector_overlay_gain,
                    1.0,
                    1.0,
                );
                let alpha = (visibility
                    * (STREAMLINE_ALPHA_FLOOR
                        + (1.0 - STREAMLINE_ALPHA_FLOOR) * f64::from(exposed))
                    * 230.0)
                    .round();
                if alpha.is_nan() || alpha < 2.0 {
                    continue;
                }
                let points = dash
                    .iter()
                    .map(|point| Pos2::new(point.x as f32, point.y as f32))
                    .collect::<Vec<_>>();
                painter.add(egui::Shape::line(
                    points,
                    Stroke::new(1.45, stroke_color(alpha as u8)),
                ));
            }
        }
    }

    /// Keeps the arrows' filter history across a solver handoff on the same
    /// mesh and physics, carries it to the nearest new sample across a
    /// same-physics remesh, and drops it for anything else.
    pub(super) fn retain_vector_overlay_filter_owner(
        &mut self,
        filter: VectorFilter,
        owner: VectorOverlayFilterOwner,
        samples: &[(u64, Pos2, Point2, Point2)],
        completed_steps: u64,
        absolute_time: f64,
        remap_radius: f32,
    ) {
        if self.vector_overlay_filter_owner != Some(owner) {
            let compatible_remesh = self
                .vector_overlay_filter_owner
                .is_some_and(|previous| previous.physics == owner.physics)
                && completed_steps >= self.vector_overlay_filter_step
                && absolute_time.is_finite();
            if compatible_remesh {
                let previous = std::mem::take(&mut self.vector_overlay_filter_state);
                let grid = OriginGrid::new(previous.values(), remap_radius);
                for (key, origin, value, _) in samples {
                    if let Some(state) = grid.nearest(*origin) {
                        let elapsed = (absolute_time - state.time).max(0.0);
                        // Remeshing is a zero-duration representation change.
                        // The raw input is rebased to the transferred new
                        // sample either way; the visible AC state is carried
                        // through its own decay, and the average is carried
                        // whole, since it is of the same flow on the new mesh.
                        let output = match filter {
                            VectorFilter::AcCoupled => {
                                state.output * (-VECTOR_DC_REJECTION_RATE * elapsed).exp()
                            }
                            VectorFilter::LowPass => state.output,
                        };
                        self.vector_overlay_filter_state.insert(
                            *key,
                            VectorFilterState {
                                input: *value,
                                output,
                                inner: state.inner,
                                step: completed_steps,
                                time: absolute_time,
                                origin: *origin,
                            },
                        );
                    }
                }
                self.vector_overlay_filter_step = completed_steps;
            } else {
                self.vector_overlay_filter_state.clear();
                self.vector_overlay_filter_step = u64::MAX;
            }
            self.vector_overlay_filter_owner = Some(owner);
        }
    }

    /// Removes only the slowly varying presentation baseline from the sampled
    /// complementary field. The exact pole and trapezoidal input difference
    /// keep the corner stable across solver steps and readback batching without
    /// attenuating ordinary source frequencies.
    pub(super) fn ac_couple_vector_samples(
        &mut self,
        samples: &mut [(u64, Pos2, Point2, Point2)],
        completed_steps: u64,
        absolute_time: f64,
        maintenance_discontinuity: bool,
    ) {
        let restarted = self.vector_overlay_filter_step == u64::MAX
            || completed_steps < self.vector_overlay_filter_step
            || !absolute_time.is_finite();
        if restarted {
            self.vector_overlay_filter_state.clear();
        }
        for (key, origin, value, pre_filter_value) in samples {
            match self.vector_overlay_filter_state.entry(*key) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(VectorFilterState {
                        input: *value,
                        output: Point2::default(),
                        inner: Default::default(),
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
        if restarted || completed_steps > self.vector_overlay_filter_step {
            self.vector_overlay_filter_step = completed_steps;
        }
    }

    /// Averages the sampled energy flow below the corner `rate` (radians per
    /// simulated second), presentation only. `VECTOR_LOW_PASS_STAGES`
    /// identical first-order stages, advanced by the chain's exact response
    /// to the new sample held over the simulated time since the previous
    /// readback, so the corner does not move with readback cadence or
    /// simulation speed.
    ///
    /// A key without history would take the whole settle time to grow in
    /// from silence, and every key is new when the spacing changes, so it
    /// seeds instead from the nearest arrow updated at the previous readback
    /// within `remap_radius`: a changed spacing or a pan keeps the picture,
    /// and the seed converges from there. With no such neighbour it starts
    /// silent and grows in as the average forms, which is the honest
    /// picture of a first enable.
    pub(super) fn low_pass_vector_samples(
        &mut self,
        samples: &mut [(u64, Pos2, Point2, Point2)],
        completed_steps: u64,
        absolute_time: f64,
        rate: f64,
        remap_radius: f32,
    ) {
        let restarted = self.vector_overlay_filter_step == u64::MAX
            || completed_steps < self.vector_overlay_filter_step
            || !absolute_time.is_finite();
        if restarted {
            self.vector_overlay_filter_state.clear();
        }
        // Seeds are chosen before any arrow is updated, so the order of the
        // samples does not decide which arrows count as live.
        let previous_step = self.vector_overlay_filter_step;
        let live = OriginGrid::new(
            self.vector_overlay_filter_state
                .values()
                .filter(|state| state.step == previous_step),
            remap_radius,
        );
        let seeded = samples
            .iter()
            .filter(|(key, ..)| !self.vector_overlay_filter_state.contains_key(key))
            .map(|(key, origin, value, _)| {
                let seed = live.nearest(*origin).copied().unwrap_or_default();
                (
                    *key,
                    VectorFilterState {
                        input: *value,
                        step: completed_steps,
                        time: absolute_time,
                        origin: *origin,
                        ..seed
                    },
                )
            })
            .collect::<Vec<_>>();
        self.vector_overlay_filter_state.extend(seeded);
        for (key, origin, value, _) in samples {
            let state = self
                .vector_overlay_filter_state
                .get_mut(key)
                .expect("every sample was seeded above");
            state.origin = *origin;
            if completed_steps > state.step {
                let elapsed = (absolute_time - state.time).max(0.0);
                low_pass_advance(state, *value, rate * elapsed);
                state.input = *value;
                state.step = completed_steps;
                state.time = absolute_time;
            }
            *value = state.output;
        }
        if restarted || completed_steps > self.vector_overlay_filter_step {
            self.vector_overlay_filter_step = completed_steps;
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
            let faint = GOLD.gamma_multiply(0.55);
            if points.len() > 1 {
                painter.add(egui::Shape::line(points.clone(), Stroke::new(1.0, GOLD)));
            }
            if draw.tool.closes_on_first() && points.len() > 2 {
                painter.line_segment(
                    [points[points.len() - 1], points[0]],
                    Stroke::new(1.0, faint),
                );
            }
            // The curve Finish would make now, from the points placed so far:
            // with no pointer on a touchscreen, it is still what a tap commits.
            if let Ok(curve) = draw.curve() {
                let options = SamplingOptions {
                    tolerance: 0.6 / self.scale,
                    ..SamplingOptions::default()
                };
                let (samples, closed) = match &curve {
                    CurveSpline::Closed(spline) => (sample(spline, options), true),
                    CurveSpline::Open(spline) => (sample_open(spline, options), false),
                };
                if let Ok(samples) = samples {
                    let line = samples
                        .iter()
                        .map(|sample| self.screen(sample.point, r))
                        .collect::<Vec<_>>();
                    let stroke = Stroke::new(2.0, GOLD);
                    painter.add(if closed {
                        egui::Shape::closed_line(line, stroke)
                    } else {
                        egui::Shape::line(line, stroke)
                    });
                }
            }
            for (index, p) in points.iter().enumerate() {
                let first = index == 0 && draw.tool.closes_on_first();
                if draw.tool.places_vertices() {
                    let size = if first { 9.0 } else { 7.0 };
                    painter.rect_filled(
                        Rect::from_center_size(*p, egui::vec2(size, size)),
                        1.0,
                        GOLD,
                    );
                } else {
                    let radius = if first { 7.0 } else { 4.0 };
                    painter.circle_stroke(*p, radius, Stroke::new(1.5, GOLD));
                }
            }
            if let (Some(last), Some(pointer)) = (points.last(), painter.ctx().pointer_hover_pos())
            {
                // Where the click will land, not where the cursor is.
                let target = self
                    .draw_attachment_hit(ScreenPoint::new(pointer.x as f64, pointer.y as f64), r)
                    .map(|hit| self.screen(hit.point, r))
                    .unwrap_or_else(|| {
                        if self.snapping(painter.ctx().input(|input| input.modifiers.shift)) {
                            self.screen(
                                Self::snap_point(self.world(pointer, r), self.snap_step()),
                                r,
                            )
                        } else {
                            pointer
                        }
                    });
                let rubber = Stroke::new(1.2, Color32::from_rgba_unmultiplied(248, 196, 112, 180));
                if draw.tool == DrawTool::Rectangle {
                    painter.rect_stroke(
                        Rect::from_two_pos(*last, target),
                        0.0,
                        rubber,
                        egui::StrokeKind::Inside,
                    );
                } else {
                    painter.line_segment([*last, target], rubber);
                }
            }
        }
        if let Some(pointer) = painter.ctx().pointer_hover_pos() {
            let current = self.world(pointer, r);
            // Where the click will land, not where the cursor is.
            let snap = self.snapping(painter.ctx().input(|input| input.modifiers.shift));
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

/// Filter states bucketed by their screen origin in cells of the search
/// radius, so the nearest state within the radius is found among the nine
/// cells around a point rather than by a scan of every state. A lattice of
/// thousands of samples replaces every key on a zoom step or a remesh, and a
/// scan per key there was a hitch of tens of milliseconds.
struct OriginGrid<'a> {
    radius_squared: f32,
    cell: f32,
    buckets: std::collections::HashMap<(i32, i32), Vec<&'a VectorFilterState>>,
}

impl<'a> OriginGrid<'a> {
    fn new(states: impl Iterator<Item = &'a VectorFilterState>, radius: f32) -> Self {
        let cell = if radius.is_finite() && radius > 0.0 {
            radius
        } else {
            1.0
        };
        let mut buckets = std::collections::HashMap::<(i32, i32), Vec<_>>::new();
        for state in states {
            if state.origin.x.is_finite() && state.origin.y.is_finite() {
                buckets
                    .entry(Self::bucket(state.origin, cell))
                    .or_default()
                    .push(state);
            }
        }
        Self {
            radius_squared: radius * radius,
            cell,
            buckets,
        }
    }

    fn bucket(origin: Pos2, cell: f32) -> (i32, i32) {
        (
            (origin.x / cell).floor() as i32,
            (origin.y / cell).floor() as i32,
        )
    }

    fn nearest(&self, origin: Pos2) -> Option<&'a VectorFilterState> {
        if !origin.x.is_finite() || !origin.y.is_finite() {
            return None;
        }
        let (column, row) = Self::bucket(origin, self.cell);
        let mut best: Option<(f32, &'a VectorFilterState)> = None;
        for column in column.saturating_sub(1)..=column.saturating_add(1) {
            for row in row.saturating_sub(1)..=row.saturating_add(1) {
                let Some(states) = self.buckets.get(&(column, row)) else {
                    continue;
                };
                for state in states {
                    let distance = state.origin.distance_sq(origin);
                    if distance <= self.radius_squared
                        && best.is_none_or(|(nearest, _)| distance < nearest)
                    {
                        best = Some((distance, state));
                    }
                }
            }
        }
        best.map(|(_, state)| state)
    }
}

/// Advances the low-pass chain by every stage's response to the sample
/// `input` held for `a = rate × elapsed`. With `d` a stage's deviation from
/// the held input, `d₁ → d₁e^{−a}`, `d₂ → (d₂ + a d₁)e^{−a}`,
/// `d₃ → (d₃ + a d₂ + a²d₁/2)e^{−a}`, and so on down the chain: exact for a
/// held input, so the readback cadence is not in the answer.
fn low_pass_advance(state: &mut VectorFilterState, input: Point2, a: f64) {
    let decay = (-a).exp();
    let mut stages = [Point2::default(); VECTOR_LOW_PASS_STAGES];
    stages[..VECTOR_LOW_PASS_STAGES - 1].copy_from_slice(&state.inner);
    stages[VECTOR_LOW_PASS_STAGES - 1] = state.output;
    let deviations = stages.map(|stage| stage - input);
    for (index, stage) in stages.iter_mut().enumerate() {
        let mut sum = Point2::default();
        let mut weight = 1.0;
        for (order, deviation) in deviations[..=index].iter().rev().enumerate() {
            sum = sum + *deviation * weight;
            weight *= a / (order + 1) as f64;
        }
        *stage = input + sum * decay;
    }
    state
        .inner
        .copy_from_slice(&stages[..VECTOR_LOW_PASS_STAGES - 1]);
    state.output = stages[VECTOR_LOW_PASS_STAGES - 1];
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
                        .then_some((key, point.stencil.element))
                })
                .collect::<BTreeMap<_, _>>()
        };
        let before = interior(&points);
        assert!(!before.is_empty());
        assert_eq!(before, interior(&shifted));
    }

    /// The streamline lattice samples the centre of every cell an element
    /// holds, in that element, so the field between samples reads bilinearly
    /// off a regular grid; a centre no element holds has no sample; and a
    /// pan keeps every interior cell's key and element.
    #[test]
    fn the_streamline_lattice_samples_cell_centres_inside_their_elements() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        let active = activate_at(&mut state, 0.08);
        let viewport = viewport();
        let separation = 54.0;
        let (world_spacing, visible_bins) =
            streamline_lattice(state.scale, separation, state.center, viewport).unwrap();
        let pixel_spacing = world_spacing * state.scale;
        let target = f64::from(separation / 3.0).max(f64::from(STREAMLINE_LATTICE_FLOOR_PIXELS));
        assert!(
            pixel_spacing >= target - 1.0e-9
                && pixel_spacing <= target * std::f64::consts::SQRT_2 + 1.0e-9,
            "{pixel_spacing} px for a target of {target}"
        );
        let points = streamline_layout(
            active.fixed_model(),
            &active.mesh,
            &active.operator,
            world_spacing,
            visible_bins,
        );
        assert!(!points.is_empty());
        let vertices_of = |element: u32| {
            active.mesh.triangles[element as usize]
                .vertices
                .map(|index| active.mesh.vertices[index].point)
        };
        for point in &points {
            let weights = point.stencil.barycentric;
            assert!(weights.iter().all(|weight| (0.0..=1.0).contains(weight)));
            assert!((weights.iter().sum::<f64>() - 1.0).abs() < 1.0e-12);
            let [a, b, c] = vertices_of(point.stencil.element);
            let held = a * weights[0] + b * weights[1] + c * weights[2];
            assert!(
                (held - point.point).norm() < 1.0e-9,
                "{held:?} vs {:?}",
                point.point
            );
            let column = (point.point.x / world_spacing).floor();
            let row = (point.point.y / world_spacing).floor();
            assert!((point.point.x / world_spacing - column - 0.5).abs() < 1.0e-9);
            assert!((point.point.y / world_spacing - row - 0.5).abs() < 1.0e-9);
            assert_eq!(point.key, lattice_cell_key(column as i64, row as i64));
            assert_eq!(
                point.stencil.value_weights,
                enriched_quadratic_basis(weights)
            );
        }
        // Exactly the cells some element holds the centre of, by brute force.
        let mut held = 0;
        for column in visible_bins[0]..=visible_bins[1] {
            for row in visible_bins[2]..=visible_bins[3] {
                let center = Point2::new(
                    (column as f64 + 0.5) * world_spacing,
                    (row as f64 + 0.5) * world_spacing,
                );
                if (0..active.mesh.triangles.len() as u32)
                    .any(|element| barycentric_in(center, vertices_of(element)).is_some())
                {
                    held += 1;
                }
            }
        }
        assert_eq!(points.len(), held);
        assert!(
            held < (visible_bins[1] - visible_bins[0] + 1) as usize
                * (visible_bins[3] - visible_bins[2] + 1) as usize,
            "the apron reaches outside the domain"
        );

        let shifted_center = state.center + Point2::new(world_spacing * 0.35, 0.0);
        let (shifted_spacing, shifted_bins) =
            streamline_lattice(state.scale, separation, shifted_center, viewport).unwrap();
        assert_eq!(shifted_spacing, world_spacing);
        let shifted = streamline_layout(
            active.fixed_model(),
            &active.mesh,
            &active.operator,
            shifted_spacing,
            shifted_bins,
        )
        .into_iter()
        .map(|point| (point.key, point.stencil.element))
        .collect::<BTreeMap<_, _>>();
        let mut common = 0;
        for point in &points {
            let column = (point.point.x / world_spacing).floor() as i64;
            if column > visible_bins[0].max(shifted_bins[0])
                && column < visible_bins[1].min(shifted_bins[1])
            {
                assert_eq!(shifted.get(&point.key), Some(&point.stencil.element));
                common += 1;
            }
        }
        assert!(common > 0);
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
        let covered = regions(&mut points.iter().map(|point| point.stencil.element as usize));
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
        let owner = VectorOverlayFilterOwner {
            mesh_revision: 17,
            physics: PhysicsModel::Mechanical,
        };
        state.retain_vector_overlay_filter_owner(
            VectorFilter::AcCoupled,
            owner,
            &[],
            100,
            2.0,
            80.0,
        );
        let mut first = vec![(3, Pos2::ZERO, Point2::new(1.0, 0.0), Point2::new(1.0, 0.0))];
        state.ac_couple_vector_samples(&mut first, 100, 2.0, false);
        let mut changing = vec![(3, Pos2::ZERO, Point2::new(1.4, 0.0), Point2::new(1.4, 0.0))];
        state.ac_couple_vector_samples(&mut changing, 110, 2.1, false);
        assert!(changing[0].2.x > 0.39);

        // A GPU generation is deliberately absent from the owner. Rebinding
        // material-dependent stencils on the same mesh therefore retains the
        // temporal baseline and continues at the transferred absolute time.
        state.retain_vector_overlay_filter_owner(
            VectorFilter::AcCoupled,
            owner,
            &[],
            120,
            2.2,
            80.0,
        );
        let mut after_handoff = vec![(3, Pos2::ZERO, Point2::new(1.5, 0.0), Point2::new(1.5, 0.0))];
        state.ac_couple_vector_samples(&mut after_handoff, 120, 2.2, false);
        assert!(after_handoff[0].2.x > 0.45, "handoff cold-started arrows");

        state.retain_vector_overlay_filter_owner(
            VectorFilter::AcCoupled,
            VectorOverlayFilterOwner {
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
        let old_owner = VectorOverlayFilterOwner {
            mesh_revision: 17,
            physics: PhysicsModel::Mechanical,
        };
        state.retain_vector_overlay_filter_owner(
            VectorFilter::AcCoupled,
            old_owner,
            &[],
            100,
            2.0,
            80.0,
        );
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
        state.retain_vector_overlay_filter_owner(
            VectorFilter::AcCoupled,
            VectorOverlayFilterOwner {
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
        assert_eq!(state.vector_overlay_filter_state[&91].input.x, 1.45);
    }

    /// The point of the low-pass: a harmonic flow ripples at twice its
    /// source's frequency by as much as its mean, and comes out as the mean.
    #[test]
    fn a_low_pass_keeps_a_steady_flow_and_removes_its_ripple() {
        let mut state = Playground::default();
        let rate = std::f64::consts::TAU * 0.5;
        let mean = Point2::new(0.8, -0.6);
        let (mut least, mut greatest) = (f64::INFINITY, 0.0_f64);
        for frame in 0..600_u64 {
            let time = frame as f64 / 60.0;
            // A 4 Hz source.
            let value = mean * (1.0 + (std::f64::consts::TAU * 8.0 * time).cos());
            let mut samples = vec![(0, Pos2::ZERO, value, value)];
            state.low_pass_vector_samples(&mut samples, frame * 10, time, rate, 80.0);
            if time >= 6.0 {
                let magnitude = samples[0].2.norm();
                least = least.min(magnitude);
                greatest = greatest.max(magnitude);
                assert!(samples[0].2.dot(mean) > 0.0, "the average turned round");
            }
        }
        let magnitude = mean.norm();
        assert!(
            (greatest - magnitude).abs() < 0.005 * magnitude,
            "settled at {greatest} against {magnitude}"
        );
        assert!(
            greatest - least < 0.005 * magnitude,
            "ripple left: {:.4} of the mean",
            (greatest - least) / magnitude
        );
    }

    /// The corner is in simulated time: a step held for two seconds reaches
    /// the chain's own response, `1 − e^{−λt}(1 + λt + λ²t²/2)`, whether the
    /// display sampled it twelve times a second or sixty.
    #[test]
    fn a_low_pass_runs_in_simulated_time() {
        let rate = std::f64::consts::TAU * 0.5;
        let value = Point2::new(1.0, 0.0);
        let run = |per_second: u64| {
            let mut state = Playground::default();
            let mut samples = vec![(0, Pos2::ZERO, value, value)];
            for frame in 0..=2 * per_second {
                samples[0].2 = value;
                let time = frame as f64 / per_second as f64;
                state.low_pass_vector_samples(&mut samples, frame, time, rate, 80.0);
            }
            samples[0].2.x
        };
        let x = rate * 2.0;
        let expected = 1.0 - (-x).exp() * (1.0 + x + x * x / 2.0);
        for per_second in [12, 60] {
            let reached = run(per_second);
            assert!(
                (reached - expected).abs() < 1.0e-12,
                "{per_second} a second reached {reached} against {expected}"
            );
        }
        // And in 1.7 s at 0.5 Hz the average is nine tenths of the way.
        let x = rate * 1.7;
        let settled = 1.0 - (-x).exp() * (1.0 + x + x * x / 2.0);
        assert!((0.9..0.91).contains(&settled), "{settled}");
    }

    /// A key without history takes the average of the nearest arrow updated
    /// at the previous readback, so a changed spacing keeps the picture. A
    /// stale or distant one does not count, and the arrow starts silent.
    #[test]
    fn a_new_flow_arrow_seeds_from_a_live_neighbour_or_starts_silent() {
        let mut state = Playground::default();
        let rate = std::f64::consts::TAU * 0.5;
        let value = Point2::new(1.0, 0.0);
        let at = |state: &mut Playground, keys: &[(u64, Pos2)], step: u64, time: f64| {
            let mut samples = keys
                .iter()
                .map(|(key, origin)| (*key, *origin, value, value))
                .collect::<Vec<_>>();
            state.low_pass_vector_samples(&mut samples, step, time, rate, 80.0);
            samples
                .into_iter()
                .map(|sample| sample.2)
                .collect::<Vec<_>>()
        };
        let (near, far) = (Pos2::new(40.0, 50.0), Pos2::new(400.0, 50.0));
        at(&mut state, &[(3, near), (7, far)], 100, 2.0);
        // Key 7 leaves the lattice and its history goes stale.
        let settled = at(&mut state, &[(3, near)], 110, 2.1)[0];
        assert!(settled.x > 0.0);

        let appeared = at(
            &mut state,
            &[
                (3, near),
                (91, Pos2::new(43.0, 48.0)),
                (92, Pos2::new(403.0, 52.0)),
                (93, Pos2::new(600.0, 600.0)),
            ],
            120,
            2.2,
        );
        assert_eq!(appeared[1], settled, "no seed from the live neighbour");
        assert_eq!(appeared[2], Point2::default(), "seeded from a stale arrow");
        assert_eq!(appeared[3], Point2::default(), "seeded from nothing near");
        assert!(appeared[0].x > settled.x, "the live arrow stopped");

        // The seed is a start, not a copy: each converges on its own.
        let later = at(
            &mut state,
            &[(3, near), (91, Pos2::new(43.0, 48.0))],
            130,
            2.3,
        );
        assert!(later[1].x > settled.x && later[1].x < 1.0);
    }

    /// Rewinding the steps is a reset: the averages are dropped, not carried
    /// into the new run.
    #[test]
    fn a_reset_clears_the_low_pass() {
        let mut state = Playground::default();
        let rate = std::f64::consts::TAU * 0.5;
        let value = Point2::new(1.0, 0.0);
        for (step, time) in [(100, 2.0), (110, 2.1), (120, 2.2)] {
            let mut samples = vec![(0, Pos2::ZERO, value, value)];
            state.low_pass_vector_samples(&mut samples, step, time, rate, 80.0);
        }
        let mut after_reset = vec![(0, Pos2::ZERO, value, value)];
        state.low_pass_vector_samples(&mut after_reset, 5, 0.1, rate, 80.0);
        assert_eq!(after_reset[0].2, Point2::default());
        assert_eq!(state.vector_overlay_filter_step, 5);
    }

    #[test]
    fn low_pass_history_survives_a_handoff_and_is_rebased_across_a_remesh() {
        let mut state = Playground::default();
        let rate = std::f64::consts::TAU * 0.5;
        let value = Point2::new(1.0, 0.0);
        let owner = VectorOverlayFilterOwner {
            mesh_revision: 17,
            physics: PhysicsModel::Mechanical,
        };
        let filter = VectorFilter::LowPass;
        state.retain_vector_overlay_filter_owner(filter, owner, &[], 100, 2.0, 80.0);
        let origin = Pos2::new(40.0, 50.0);
        let mut first = vec![(3, origin, value, value)];
        state.low_pass_vector_samples(&mut first, 100, 2.0, rate, 80.0);
        let mut second = vec![(3, origin, value, value)];
        state.low_pass_vector_samples(&mut second, 110, 2.1, rate, 80.0);
        let before = second[0].2;
        assert!(before.x > 0.0);

        // A same-mesh handoff keeps the history and carries on from it.
        state.retain_vector_overlay_filter_owner(filter, owner, &[], 120, 2.2, 80.0);
        let mut after_handoff = vec![(3, origin, value, value)];
        state.low_pass_vector_samples(&mut after_handoff, 120, 2.2, rate, 80.0);
        assert!(
            after_handoff[0].2.x > before.x,
            "handoff cold-started arrows"
        );
        let carried = state.vector_overlay_filter_state[&3];

        // A remesh carries the average whole to the nearest new sample.
        let new_samples = vec![(91, Pos2::new(43.0, 48.0), value, value)];
        state.retain_vector_overlay_filter_owner(
            filter,
            VectorOverlayFilterOwner {
                mesh_revision: 18,
                ..owner
            },
            &new_samples,
            120,
            2.2,
            80.0,
        );
        let mut accepted = new_samples;
        state.low_pass_vector_samples(&mut accepted, 120, 2.2, rate, 80.0);
        assert_eq!(accepted[0].2, carried.output, "remesh blinked the average");
        assert_eq!(state.vector_overlay_filter_state[&91].inner, carried.inner);

        // A physics change discards it.
        state.retain_vector_overlay_filter_owner(
            filter,
            VectorOverlayFilterOwner {
                mesh_revision: 18,
                physics: PhysicsModel::Electromagnetic {
                    polarization: ElectromagneticPolarization::Tm,
                },
            },
            &[],
            130,
            2.3,
            80.0,
        );
        assert!(state.vector_overlay_filter_state.is_empty());
    }

    /// Every arrow starts silent on the first frame, so a scale measured on
    /// the averaged arrows would have nothing to measure; it is measured on
    /// the raw ones, and a standing region then draws short rather than
    /// having what the average leaves renormalised to full length.
    #[test]
    fn low_passed_flow_arrows_are_exposed_by_the_raw_level() {
        let mut state = Playground::default();
        state.editor.document.presentation.vector_overlay = VectorOverlay::RelativeEnergyFlow;
        assert!(state.editor.document.presentation.vector_overlay_lowpass);
        let context = egui::Context::default();
        let painter = egui::Painter::new(context, egui::LayerId::background(), Rect::EVERYTHING);
        let owner = VectorOverlayFilterOwner {
            mesh_revision: 1,
            physics: state.editor.document.model.draft.physics,
        };
        let samples = (0..20u32)
            .map(|index| {
                let value = Point2::new(1.0 + f64::from(index) * 0.1, 0.0);
                (
                    u64::from(index),
                    Pos2::new(index as f32 * 60.0, 0.0),
                    value,
                    value,
                )
            })
            .collect::<Vec<_>>();
        state.draw_vector_overlay(&painter, viewport(), samples, None, owner, 10, 0.1);
        assert_eq!(state.vector_overlay_filter, Some(VectorFilter::LowPass));
        let reference = state
            .vector_overlay_exposure
            .reference()
            .expect("a scale from the raw samples");
        assert!((reference - 2.7).abs() < 1.0e-9, "{reference}");
    }

    /// With the streamline style and a lattice layout, the flow is placed
    /// as lines through the samples; the dashes drift with simulated time,
    /// so a paused simulation holds them still; and an arrow layout, with
    /// no lattice to read, is drawn as arrows whatever the style.
    #[test]
    fn streamlines_are_placed_from_the_lattice_and_drift_in_simulated_time() {
        let mut state = Playground::default();
        let presentation = &mut state.editor.document.presentation;
        presentation.vector_overlay = VectorOverlay::RelativeEnergyFlow;
        presentation.vector_overlay_style = VectorOverlayStyle::Streamlines;
        presentation.vector_overlay_lowpass = false;
        let viewport = viewport();
        let context = egui::Context::default();
        let painter = egui::Painter::new(context, egui::LayerId::background(), Rect::EVERYTHING);
        let owner = VectorOverlayFilterOwner {
            mesh_revision: 1,
            physics: state.editor.document.model.draft.physics,
        };
        let world_spacing = 20.0 / state.scale;
        let corners = [
            state.world(viewport.min, viewport),
            state.world(viewport.max, viewport),
        ];
        let bin = |value: f64| (value / world_spacing).floor() as i64;
        let (columns, rows) = (
            bin(corners[0].x.min(corners[1].x)) - 1..=bin(corners[0].x.max(corners[1].x)) + 1,
            bin(corners[0].y.min(corners[1].y)) - 1..=bin(corners[0].y.max(corners[1].y)) + 1,
        );
        let samples = columns
            .flat_map(|column| {
                rows.clone().map(move |row| {
                    let center = Point2::new(column as f64 + 0.5, row as f64 + 0.5) * world_spacing;
                    (lattice_cell_key(column, row), center)
                })
            })
            .map(|(key, center)| {
                let value = Point2::new(1.0, 0.2);
                (key, state.screen(center, viewport), value, value)
            })
            .collect::<Vec<_>>();
        state.draw_vector_overlay(
            &painter,
            viewport,
            samples.clone(),
            Some(world_spacing),
            owner,
            10,
            0.1,
        );
        let lines = state.vector_overlay_streamlines.lines.len();
        assert!(lines >= 5, "{lines} lines");
        assert!(state.vector_overlay_streamlines.lines.iter().all(|line| {
            line.points
                .windows(2)
                .all(|pair| (pair[1] - pair[0]).dot(Point2::new(1.0, 0.2)) > 0.0)
        }));
        let phase = state.streamline_dash_phase;
        state.draw_vector_overlay(
            &painter,
            viewport,
            samples.clone(),
            Some(world_spacing),
            owner,
            10,
            0.1,
        );
        assert_eq!(state.streamline_dash_phase, phase, "paused: no drift");
        assert_eq!(state.vector_overlay_streamlines.lines.len(), lines);
        state.draw_vector_overlay(
            &painter,
            viewport,
            samples.clone(),
            Some(world_spacing),
            owner,
            20,
            0.2,
        );
        assert!(
            (state.streamline_dash_phase - phase - 0.1 * STREAMLINE_DASH_SPEED).abs() < 1.0e-9,
            "drifted to {}",
            state.streamline_dash_phase
        );

        state.draw_vector_overlay(&painter, viewport, samples, None, owner, 30, 0.3);
        assert_eq!(
            state.vector_overlay_streamlines.lines.len(),
            lines,
            "arrows drawn from an arrow layout leave the lines' seeds alone"
        );
        state.clear_vector_overlay_filter();
        assert!(state.vector_overlay_streamlines.lines.is_empty());
        assert_eq!(state.streamline_dash_time, None);
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
