//! The viewport frame: the interaction surface, the grid drawn under it, and
//! the vector overlay lattice refreshed behind it.

use crate::material_overlay::MaterialOverlay;
use crate::wave_gpu::{VectorOverlayDisplay, WaveDisplay, WaveGpuRequest};
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use bevy_egui::egui::{self, Color32, Pos2, Rect, Sense, Stroke};
use funfern_app::document::VectorOverlay;
use funfern_app::topology_editor::TopologyAcceptance;
use funfern_app::topology_runtime::PreparedTopology;
use funfern_app::topology_viewport::SampledTopologyGeometry;
use funfern_core::*;
use std::sync::Arc;

use super::*;

impl Playground {
    pub(super) fn viewport(
        &mut self,
        ui: &mut egui::Ui,
        display: &WaveDisplay,
        vector_display: &VectorOverlayDisplay,
    ) -> Rect {
        let available = ui.available_size();
        let (response, painter) = ui.allocate_painter(available, Sense::click_and_drag());
        let viewport = response.rect;
        self.viewport_rect = viewport;
        if self.fit {
            self.fit_view(viewport);
        }
        self.refresh_samples(viewport);
        let transform = self.transform(viewport);
        self.draw_solution(&painter, viewport, display, vector_display);
        if self.editor.document.presentation.grid {
            self.draw_grid(&painter, viewport);
        }
        if matches!(self.editor.acceptance, TopologyAcceptance::Invalid(_))
            && self.editor.document.presentation.accepted_reference
        {
            if let Ok(reference) = SampledTopologyGeometry::new(
                &self.editor.document.model.accepted.geometry,
                transform,
                0.8,
            ) {
                self.draw_sampled(
                    &painter,
                    viewport,
                    &reference,
                    Color32::from_gray(65),
                    1.0,
                    false,
                );
            }
        }
        if let Some(sampled) = &self.sampled {
            let color = if matches!(self.editor.acceptance, TopologyAcceptance::Invalid(_)) {
                RED
            } else {
                TEAL
            };
            self.draw_sampled(&painter, viewport, sampled, color, 2.0, true);
        }
        self.draw_weld_targets(&painter, viewport);
        self.draw_domain_handles(&painter, viewport);
        self.draw_markers(&painter, viewport);
        self.draw_material_frame(&painter, viewport);
        self.draw_transform_gizmo(&painter, viewport);
        self.prune_stale_pending_merge();
        self.draw_removal_candidates(&painter, viewport);
        let ctx = ui.ctx().clone();
        self.removal_prompt(&ctx, viewport);
        if let Some(logo) = &self.logo_texture {
            let size = egui::vec2(140.0, 57.0);
            let logo_rect = Rect::from_min_size(
                Pos2::new(viewport.right() - size.x - 14.0, viewport.top() + 12.0),
                size,
            );
            painter.image(
                logo.id(),
                logo_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if let MaterialOverlay::Property(property) =
            self.editor.document.presentation.material_overlay
            && let Some(pointer) = response.hover_pos()
            && let Some(sample) = self
                .material_overlay_snapshot
                .as_ref()
                .and_then(|snapshot| {
                    snapshot.samples.iter().min_by(|left, right| {
                        self.screen(left.point, viewport)
                            .distance(pointer)
                            .total_cmp(&self.screen(right.point, viewport).distance(pointer))
                    })
                })
            && self.screen(sample.point, viewport).distance(pointer) <= 14.0
        {
            let value = sample
                .value(property)
                .map_or_else(|error| error.into(), |value| format!("{value:.5}"));
            response.clone().on_hover_text(format!(
                "{} · region {} · {value}\nlocal x {:.3}, y {:.3}, r {:.3}, θ {:.1}°",
                sample.material_name,
                sample.region.0,
                sample.coordinates.x,
                sample.coordinates.y,
                sample.coordinates.r,
                sample.coordinates.theta.to_degrees(),
            ));
        }
        self.handle_viewport_input(ui, &response, viewport);
        viewport
    }
    /// Drawn over the field rather than under it. The field wash is opaque, so
    /// a grid beneath it is only ever visible where nothing is meshed; these
    /// lines tint instead, faint enough not to compete with the wave and light
    /// enough to read on the dark base beside it.
    /// The spacing and the world coordinates the grid draws at. Both axes walk
    /// upwards from the lower corner of the view: `world` flips y, so the
    /// bottom of the screen is the smaller world coordinate.
    pub(super) fn grid_axes(&self, r: Rect, step: f64) -> (Vec<f64>, Vec<f64>) {
        let minimum = self.world(r.left_bottom(), r);
        let maximum = self.world(r.right_top(), r);
        (
            grid_lines(minimum.x, maximum.x, step),
            grid_lines(minimum.y, maximum.y, step),
        )
    }

    /// The lattice Shift lands on: the grid's own fine step, so what is drawn
    /// is what a snapped position can reach.
    pub(super) fn snap_step(&self) -> f64 {
        grid_steps(self.scale).1
    }
    pub(super) fn draw_grid(&self, painter: &egui::Painter, r: Rect) {
        let (step, fine) = grid_steps(self.scale);
        let (columns, rows) = self.grid_axes(r, fine);
        // The fine lattice is what Shift lands on, so it is drawn - faintly,
        // because it is there to be aimed at rather than read.
        let stroke = |value: f64| {
            if value.abs() < fine * 0.1 {
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(148, 163, 184, 90))
            } else if (value / step - (value / step).round()).abs() < 1.0e-6 {
                Stroke::new(0.5, Color32::from_rgba_unmultiplied(148, 163, 184, 45))
            } else {
                Stroke::new(0.5, Color32::from_rgba_unmultiplied(148, 163, 184, 20))
            }
        };
        for x in columns {
            let sx = self.screen(Point2::new(x, 0.0), r).x;
            painter.line_segment(
                [Pos2::new(sx, r.top()), Pos2::new(sx, r.bottom())],
                stroke(x),
            );
        }
        for y in rows {
            let sy = self.screen(Point2::new(0.0, y), r).y;
            painter.line_segment(
                [Pos2::new(r.left(), sy), Pos2::new(r.right(), sy)],
                stroke(y),
            );
        }
    }
    /// A reset zeroes the field and a mesh handoff renumbers it; both bump the
    /// solver's generation, and either can move the field's scale outright.
    /// Measuring a new field against the old reference would paint its first
    /// seconds wrong, which is what loading one example over another used to do
    /// to the arrows. How loud the run has been is kept, so a field decaying
    /// through an adaptation handoff is not renormalized back into view.
    /// A handoff carries the field onto a new mesh — the same field, renumbered
    /// — so its scale carries across untouched. Only a field replaced with zeros
    /// starts a new run.
    ///
    /// Keying this on the solver's generation instead was wrong twice over:
    /// adaptation bumps the generation every second or two, and each bump
    /// dropped the scale onto the instantaneous level. A decaying field then
    /// fell in visible steps — 8.09e-3 to 1.21e-3 across one handoff — instead
    /// of easing down at the release rate.
    pub(super) fn restart_exposures_after_handoff(&mut self, fresh: bool) {
        if fresh {
            self.field_exposure.restart();
            self.vector_overlay_exposure.restart();
            self.vector_overlay_ac_state.clear();
            self.vector_overlay_ac_owner = None;
            self.vector_overlay_dc_step = u64::MAX;
        }
    }

    pub(super) fn refresh_vector_overlay(
        &mut self,
        recorders: &mut WaveGpuRequest,
        display: &VectorOverlayDisplay,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        active: Option<(&Arc<PreparedTopology>, RecorderSource<'_>)>,
        generation: u64,
    ) {
        let mode = self
            .editor
            .document
            .presentation
            .vector_overlay
            .resolved(self.editor.document.model.draft.physics);
        if mode == VectorOverlay::Off {
            self.vector_overlay_previous_layout = None;
            if self.vector_overlay_layout.take().is_some() || recorders.vector_overlay.is_some() {
                recorders.clear_vector_overlay(assets, commands);
            }
            return;
        }
        // During GPU handoff the request/display generation can lead the
        // runtime commit for a frame. Keep the completed old lattice visible;
        // clearing it here creates exactly the mesh-handoff blink this cache
        // exists to bridge.
        let Some((active, source)) = active else {
            return;
        };
        if generation == 0 || !self.viewport_rect.is_positive() {
            return;
        }
        let Some((world_spacing, visible_bins)) = vector_overlay_lattice(
            self.scale,
            self.editor.document.presentation.vector_overlay_density,
            self.center,
            self.viewport_rect,
        ) else {
            return;
        };
        let key = VectorOverlayLayoutKey {
            mesh_revision: active.mesh.mesh_revision,
            generation,
            physics: active.bundle.authored.physics,
            world_spacing,
            visible_bins,
        };
        // Keep at most one GPU lattice replacement in flight. Replacing its
        // readback every camera frame can otherwise starve the overlay until
        // pan/zoom stops. The last completed world-space lattice is still
        // reprojected by `draw_solution` while this one catches up. Ownership
        // and a deadline matter: a dropped/superseded readback must not leave
        // the coalescer waiting forever (reset used to be the only escape).
        let current_complete = self
            .vector_overlay_layout
            .as_ref()
            .is_some_and(|layout| layout.matches(display));
        if self.vector_overlay_layout.as_ref().is_some_and(|layout| {
            layout.key == key && (layout.points.is_empty() || current_complete)
        }) {
            return;
        }
        if self.vector_overlay_layout.as_ref().is_some_and(|layout| {
            !layout.points.is_empty()
                && !current_complete
                && vector_overlay_revision_owned(
                    layout.key.generation,
                    layout.revision,
                    recorders.generation(),
                    recorders.vector_overlay_revision(),
                    recorders.vector_overlay.is_some(),
                )
                && layout.submitted_at.elapsed() < VECTOR_OVERLAY_READBACK_TIMEOUT
        }) {
            return;
        }
        if let Some(previous) = self.vector_overlay_layout.take()
            && previous.matches(display)
        {
            self.vector_overlay_previous_layout = Some(previous);
        }
        let points = vector_overlay_layout(
            active.fixed_model(),
            &active.mesh,
            &active.operator,
            key.world_spacing,
            key.visible_bins,
        );
        let stencils = points.iter().map(|point| point.stencil).collect::<Vec<_>>();
        match source.vector_overlay(recorders, assets, commands, &stencils) {
            Ok(()) => {
                if points.is_empty() {
                    self.vector_overlay_previous_layout = None;
                }
                self.vector_overlay_layout = Some(VectorOverlayLayout {
                    key,
                    revision: recorders.vector_overlay_revision(),
                    points,
                    submitted_at: Instant::now(),
                });
            }
            Err(error) => {
                recorders.clear_vector_overlay(assets, commands);
                self.vector_overlay_layout = None;
                self.vector_overlay_previous_layout = None;
                self.message = error;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both axes step upwards from the lower bound. The vertical one used to
    /// start at the top of the view and test against the bottom, so the grid
    /// had only ever been columns.
    #[test]
    fn the_grid_covers_both_axes_of_the_view() {
        // An 800x600 view at 300 pixels per world unit, centred on the origin.
        let state = Playground {
            scale: 300.0,
            center: Point2::default(),
            ..Playground::default()
        };
        let view = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let (step, fine) = grid_steps(state.scale);
        assert_eq!(step, 0.5, "70 px at this zoom lands on the half-unit");
        assert_eq!(fine, 0.1, "which divides into five");
        let (columns, rows) = state.grid_axes(view, step);
        assert_eq!(columns, vec![-1.5, -1.0, -0.5, 0.0, 0.5, 1.0]);
        assert_eq!(rows, vec![-1.0, -0.5, 0.0, 0.5, 1.0]);
        assert!(
            rows.iter().any(|y| y.abs() < step * 0.1),
            "the horizontal axis is among them, and is the emphasised line"
        );
        // The fine lattice nests inside the drawn one, so every line that is
        // drawn is one Shift can land on.
        assert!(
            (step / fine - 5.0).abs() < 1.0e-9,
            "a half-unit divides in five"
        );
        let (fine_columns, fine_rows) = state.grid_axes(view, fine);
        assert!(fine_columns.len() >= columns.len() * 4);
        assert!(fine_rows.len() >= rows.len() * 4);

        // A bound the wrong way round draws nothing rather than looping.
        assert!(grid_lines(1.0, -1.0, step).is_empty());
        assert!(grid_lines(-1.0, 1.0, 0.0).is_empty());
        assert!(grid_lines(f64::NAN, 1.0, step).is_empty());
        // A degenerate zoom cannot hang the painter.
        assert!(grid_lines(-1.0, 1.0, grid_steps(0.0).0).is_empty());
        assert!(grid_lines(-1.0e9, 1.0e9, 1.0e-9).len() <= 4096);

        // The spacing holds its decade: about 70 pixels apart at any zoom, and
        // the step Shift lands on is always a round division of it.
        for scale in [12.0, 37.0, 300.0, 1_500.0, 9_000.0] {
            let (step, fine) = grid_steps(scale);
            let pixels = step * scale;
            assert!(
                (35.0..=180.0).contains(&pixels),
                "{scale} pixels per unit put lines {pixels} apart"
            );
            let divisions = step / fine;
            assert!(
                (divisions - divisions.round()).abs() < 1.0e-9 && (4.0..=5.0).contains(&divisions),
                "{scale} divides {step} into {divisions}"
            );
        }
    }
}
