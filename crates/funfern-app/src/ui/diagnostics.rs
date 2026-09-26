//! The status strip and the windows behind it: diagnostics sections, the
//! event log, the example gallery and the formula reference.

use crate::recording::{self};
use crate::wave_gpu::MAX_STEPS_PER_FRAME;
use bevy::prelude::*;
use bevy_egui::egui::{self, Color32, Pos2, Sense, Stroke};
use funfern_app::topology_editor::{TopologyAcceptance, TopologyDocument};
use funfern_app::topology_runtime::TopologyPreparationPhase;
use funfern_core::*;

use super::*;

impl Playground {
    pub(super) fn diagnostics_warning(&self) -> bool {
        self.runtime.last_error().is_some() || self.amr_error.is_some() || self.unseen_error
    }
    /// Appends what each transient channel says, the moment it changes. Every
    /// one of them is overwritten by whatever happens next - a status line by
    /// the next message, a preparation or adaptation error by the next
    /// success, a repair fallback by the next transaction - so a change is the
    /// only moment the value can be caught. Watching the values rather than
    /// the two dozen places that set them is what makes the log complete.
    pub(super) fn record_events(&mut self, now: f64) {
        let status = (!self.message.is_empty()).then(|| self.message.clone());
        if status != self.logged_status {
            self.logged_status = status.clone();
            if let Some(text) = status {
                self.push_event(now, EventSource::Status, text);
            }
        }
        let preparation = self.runtime.last_error().map(|error| error.to_string());
        if preparation != self.logged_preparation {
            self.logged_preparation = preparation.clone();
            if let Some(text) = preparation {
                self.push_event(now, EventSource::Preparation, text);
            }
        }
        let adaptation = self.amr_error.clone();
        if adaptation != self.logged_adaptation {
            self.logged_adaptation = adaptation.clone();
            if let Some(text) = adaptation {
                self.push_event(now, EventSource::Adaptation, text);
            }
        }
        for text in std::mem::take(&mut self.pending_repairs) {
            self.push_event(now, EventSource::Repair, text);
        }
    }
    pub(super) fn push_event(&mut self, now: f64, source: EventSource, text: String) {
        if let Some(last) = self.events.back_mut()
            && last.source == source
            && last.text == text
        {
            last.repeats += 1;
            last.time = now;
            return;
        }
        if self.events.len() == EVENT_LOG_ENTRIES {
            self.events.pop_front();
        }
        self.events.push_back(EventEntry {
            time: now,
            source,
            text,
            repeats: 1,
        });
        if source.error() {
            self.unseen_error = true;
        }
    }
    pub(super) fn summary_line(&self) -> String {
        let active = self.runtime.active();
        format!(
            "{:.0} fps · {:.0} steps/s · {} dofs · {} elements · dt {}",
            1000.0 / self.frame_ms.max(0.01),
            self.steps_per_second,
            active.map_or(0, |v| v.operator.degrees_of_freedom()),
            active.map_or(0, |v| v.mesh.triangles.len()),
            if active.is_some() {
                format!("{:.2e}", self.solver_time_step())
            } else {
                "—".into()
            }
        )
    }
    /// One draggable window over the whole transaction: the frame it costs, the
    /// log of what the transient channels said, and then the topology and mesh
    /// it produced, the handoff waits and the running solver. A preparation or
    /// adaptation error lights the status marker and keeps it lit until this
    /// is opened, rather than opening it; an ordinary rebuild does neither.
    pub(super) fn diagnostics_window(&mut self, ctx: &egui::Context) {
        self.record_events(ctx.input(|input| input.time));
        if self.frame_ms.is_finite() && self.frame_ms > 0.0 {
            if self.frame_history.len() == FRAME_HISTORY {
                self.frame_history.pop_front();
            }
            self.frame_history.push_back(self.frame_ms);
        }
        if !self.diagnostics_open {
            return;
        }
        // Open is seen: the log below holds whatever lit the marker.
        self.unseen_error = false;
        let mut open = true;
        egui::Window::new("Performance diagnostics")
            .open(&mut open)
            .default_width(420.0)
            .resizable(true)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.monospace(self.summary_line());
                    ui.separator();
                    self.frame_section(ui);
                    // Above the sections whose height follows whatever the
                    // last transaction did, so reading the log does not mean
                    // chasing it down the window.
                    self.log_section(ui);
                    self.topology_section(ui);
                    self.mesh_section(ui);
                    self.handoff_section(ui);
                    self.solver_section(ui);
                });
            });
        self.diagnostics_open = open;
    }
    /// The `?` beside a place where formulas are typed.
    pub(super) fn formula_help_toggle(&mut self, ui: &mut egui::Ui) {
        if ui
            .small_button("?")
            .on_hover_text("Formula syntax reference")
            .clicked()
        {
            self.formula_help_open = !self.formula_help_open;
        }
    }

    /// The example gallery: the catalog in its sections, a tile per scene with
    /// its thumbnail and name, and what the scene shows on hover. A pick opens
    /// the scene and closes the gallery, so the scene is not left behind it;
    /// the scene card steps on from there and comes back here.
    pub(super) fn examples_window(&mut self, ctx: &egui::Context) {
        if !self.examples_open {
            return;
        }
        let catalog = funfern_app::topology_examples::catalog();
        // At most one preview is built per frame. The largest example takes
        // about twenty milliseconds to compile, so building all of them at once
        // would drop a frame outright; this way a tile shows its name
        // immediately and its thumbnail a few frames later.
        if let Some(index) = self.example_previews.iter().position(Option::is_none) {
            self.example_previews[index] = Some(build_example_preview(&catalog[index]));
        }
        let previews = &self.example_previews;
        let opened = self.example_opened;
        let mut open = true;
        let mut selected = None;
        // A section a row, or as many tiles as a narrow screen has room for,
        // and as tall as the screen allows below the top bar; the grid sets
        // the window's size.
        // It is kept between the top bar and the status strip: its first,
        // invisible sizing frame lays the whole catalog out unscrolled, and
        // held only to the screen that would push it over the top bar.
        let screen = ctx.content_rect();
        let between_bars =
            egui::Rect::from_x_y_ranges(screen.x_range(), self.viewport_rect.y_range());
        let columns = gallery_columns(screen.width() - 48.0).min(GALLERY_COLUMNS);
        let height = (between_bars.height() - 100.0).clamp(200.0, 620.0);
        egui::Window::new("Examples")
            .id(egui::Id::new("examples"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .constrain_to(between_bars)
            .default_pos(between_bars.left_top() + egui::vec2(16.0, 8.0))
            .show(ctx, |ui| {
                ui.label("Pick a scene to open it. Hover one for what it shows.");
                // The minimum as well as the maximum: an auto-sized window
                // offers its content the size it had the frame before, so a
                // scroll area left to shrink never grows past its first frame.
                egui::ScrollArea::vertical()
                    .min_scrolled_height(height)
                    .max_height(height)
                    .show(ui, |ui| {
                        ui.set_width(gallery_width(columns));
                        ui.spacing_mut().item_spacing.x = EXAMPLE_TILE_GAP;
                        for group in ExampleGroup::ALL {
                            ui.add_space(8.0);
                            ui.strong(group.label());
                            ui.add_space(2.0);
                            let members = (0..catalog.len())
                                .filter(|&index| catalog[index].group == group)
                                .collect::<Vec<_>>();
                            for row in members.chunks(columns) {
                                ui.horizontal_top(|ui| {
                                    for &index in row {
                                        let tile = example_tile(
                                            ui,
                                            &catalog[index],
                                            previews[index].as_ref(),
                                            opened == Some(index),
                                        );
                                        if tile.clicked() {
                                            selected = Some(index);
                                        }
                                    }
                                });
                            }
                        }
                    });
            });
        self.examples_open = open;
        if let Some(index) = selected {
            self.pick_example(index);
        }
    }

    /// A pick in the gallery: the scene opens and the gallery gets out of
    /// its way.
    pub(super) fn pick_example(&mut self, index: usize) {
        self.examples_open = false;
        self.open_example(index);
    }

    pub(super) fn open_example(&mut self, index: usize) {
        let catalog = funfern_app::topology_examples::catalog();
        match self.set_document(catalog[index].document.clone(), true, true) {
            Ok(()) => {
                self.example_opened = Some(index);
                self.scene_card.dismissed = false;
                self.scene_card.hint = false;
                self.notify(format!("Opened {}", catalog[index].name));
            }
            Err(error) => self.notify(error),
        }
    }

    /// A scene to start from: nothing drawn, one background material, every wall
    /// second-order outgoing, and the point source switched on, because an empty
    /// scene that makes no wave is a still picture rather than a starting point.
    pub(super) fn new_scene(&mut self) {
        let mut document = TopologyDocument::default();
        document.model.source.enabled = true;
        match self.set_document(document, true, true) {
            Ok(()) => self.notify("New scene"),
            Err(error) => self.notify(error),
        }
    }

    /// Opens a catalog entry at random, for a launch with nothing to restore,
    /// which is a first launch but for an autosave gone missing. Its card
    /// opens with a line saying there are more.
    pub(super) fn open_random_example(&mut self) {
        let catalog = funfern_app::topology_examples::catalog();
        let index = random_example_index(random_fraction(), catalog.len());
        if let Err(error) = self.set_document(catalog[index].document.clone(), false, true) {
            self.notify(error);
        } else {
            self.example_opened = Some(index);
            self.scene_card.dismissed = false;
            self.scene_card.hint = true;
        }
    }

    pub(super) fn formula_help_window(&mut self, ctx: &egui::Context) {
        if !self.formula_help_open {
            return;
        }
        let mut open = true;
        egui::Window::new("Formula syntax")
            .id(egui::Id::new("formula_help"))
            .open(&mut open)
            .default_width(340.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.small("Every material coefficient and every source profile takes either a constant or a formula in these terms.");
                ui.separator();
                ui.strong("Coordinates and constants");
                egui::Grid::new("formula_help_symbols")
                    .num_columns(2)
                    .spacing([12.0, 2.0])
                    .show(ui, |ui| {
                        for (names, meaning) in FORMULA_SYMBOLS {
                            ui.monospace(names);
                            ui.small(meaning);
                            ui.end_row();
                        }
                    });
                ui.small("A material's own named parameters can be used directly.");
                ui.separator();
                ui.strong("Operators");
                ui.monospace("+  -  *  /  ^  ( )");
                ui.small("^ raises to a power and groups to the right.");
                ui.separator();
                ui.strong("Functions");
                egui::Grid::new("formula_help_functions")
                    .num_columns(2)
                    .spacing([12.0, 2.0])
                    .show(ui, |ui| {
                        for (signature, meaning, _) in FORMULA_FUNCTIONS {
                            ui.monospace(signature);
                            ui.small(meaning);
                            ui.end_row();
                        }
                    });
                ui.separator();
                ui.small("A radial profile, for example: parameter R = 0.35 with stiffness 2 - clamp(0, 1, r / R)^2.");
                ui.separator();
                ui.strong("Laws");
                egui::Grid::new("formula_help_laws")
                    .num_columns(2)
                    .spacing([12.0, 2.0])
                    .show(ui, |ui| {
                        for (form, meaning) in FORMULA_LAWS {
                            ui.monospace(form);
                            ui.small(meaning);
                            ui.end_row();
                        }
                    });
            });
        self.formula_help_open = open;
    }

    pub(super) fn frame_section(&self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Frame")
            .default_open(true)
            .show(ui, |ui| {
                ui.label(format!(
                    "{:.2} ms · {:.0} px/unit",
                    self.frame_ms, self.scale
                ));
                let samples = self.frame_history.len();
                let peak = self.frame_history.iter().copied().fold(0.0_f32, f32::max);
                let average =
                    self.frame_history.iter().sum::<f32>() / self.frame_history.len().max(1) as f32;
                let mut ordered = self.frame_history.iter().copied().collect::<Vec<_>>();
                ordered.sort_by(f32::total_cmp);
                let p95 = ordered
                    .get(((samples as f32 * 0.95).ceil() as usize).saturating_sub(1))
                    .copied()
                    .unwrap_or(0.0);
                ui.small(format!(
                    "Average {average:.2} ms · p95 {p95:.2} ms · peak {peak:.2} ms · {samples} samples"
                ));
                let (response, painter) =
                    ui.allocate_painter(egui::vec2(ui.available_width(), 46.0), Sense::hover());
                let rect = response.rect;
                painter.rect_filled(rect, 3.0, Color32::from_rgb(10, 16, 22));
                if samples > 1 && peak > 0.0 {
                    let points = self
                        .frame_history
                        .iter()
                        .enumerate()
                        .map(|(index, value)| {
                            Pos2::new(
                                egui::lerp(
                                    rect.left()..=rect.right(),
                                    index as f32 / (samples - 1) as f32,
                                ),
                                rect.bottom() - rect.height() * (value / peak),
                            )
                        })
                        .collect();
                    painter.add(egui::Shape::line(points, Stroke::new(1.5, TEAL)));
                }
            });
    }
    pub(super) fn topology_section(&self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Topology")
            .default_open(true)
            .show(ui, |ui| {
                match self.editor.acceptance {
                    TopologyAcceptance::Valid => ui.label("Draft accepted"),
                    TopologyAcceptance::Pending => ui.label("Draft compiling"),
                    // The status bar says this in words; the diagnostics window
                    // is where the structure behind it belongs.
                    TopologyAcceptance::Invalid(issue) => {
                        ui.colored_label(RED, format!("Draft invalid: {issue:?}"))
                    }
                };
                ui.small(format!(
                    "Document revision {} · {} curves · {} authored regions",
                    self.editor.revision,
                    self.editor.document.model.draft.geometry.curves.len(),
                    self.editor.document.model.draft.regions.len(),
                ));
                match self.runtime.active() {
                    Some(active) => ui.small(format!(
                        "Committed token: document {} · topology {} · mesh generation {}",
                        active.bundle.token.document_revision,
                        active.bundle.token.topology_revision,
                        active.bundle.token.mesh_generation,
                    )),
                    None => ui.small("No committed topology"),
                };
                match (self.preparation_phase(), self.preparation_timing()) {
                    (Some(phase), Some(timing)) => {
                        ui.small(format!(
                            "Preparing: {}",
                            self.preparation_detail().unwrap_or(phase.label())
                        ));
                        ui.small(timing_line(timing));
                    }
                    (Some(phase), None) => {
                        ui.small(format!("Candidate: {}", phase.label()));
                    }
                    _ => {
                        ui.small("No candidate in preparation");
                    }
                }
                if let Some(error) = self.runtime.last_error() {
                    ui.colored_label(RED, format!("{error}"));
                }
            });
    }
    pub(super) fn mesh_section(&self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Mesh")
            .default_open(true)
            .show(ui, |ui| {
                match self.runtime.active() {
                    Some(active) => {
                        ui.label(format!(
                            "{} vertices · {} triangles · {} active regions",
                            active.mesh.vertices.len(),
                            active.mesh.triangles.len(),
                            active.bundle.plan.domains.len(),
                        ));
                        ui.small(format!(
                            "Minimum angle {:.1}° · maximum edge {:.3} · target {:.3}",
                            active.mesh.quality.minimum_angle_degrees,
                            active.mesh.quality.maximum_edge_length,
                            active.meshing.target_edge_length,
                        ));
                    }
                    None => {
                        ui.label("No committed mesh");
                    }
                }
                ui.small(format!("Adaptation: {}", self.amr_status));
                if let Some(result) = &self.amr_indicator_result {
                    let report = &result.report;
                    ui.small(format!(
                        "Indicator {:.2e}–{:.2e} · target {:.3}–{:.3}",
                        report.minimum_indicator,
                        report.maximum_indicator,
                        report.minimum_target,
                        report.maximum_target,
                    ));
                    if report.dormant {
                        ui.small("Whole field dormant · relative error suppressed");
                    } else {
                        ui.small(format!(
                            "Whole field {:.2}% · target {:.0}% · {}",
                            100.0 * report.global_indicator,
                            self.editor.document.presentation.adaptation.accuracy_percent,
                            adaptation_decision(report, self.amr_target_accuracy()).label(),
                        ));
                    }
                    ui.small(format!(
                        "Refine candidates {} ({} error, {} limit) · coarsen candidates {} · \
                         {} work units",
                        report.refine_candidates,
                        report.error_refine_candidates,
                        report.limit_refine_candidates,
                        report.coarsen_candidates,
                        report.work_units,
                    ));
                    ui.small(format!(
                        "Residual: primary {:.2e} + complementary {:.2e} recovery · cell {:.2e} · jump {:.2e} · boundary {:.2e}",
                        report.displacement_recovery_contribution,
                        report.complementary_recovery_contribution,
                        report.cell_residual_contribution,
                        report.interior_jump_contribution,
                        report.boundary_residual_contribution,
                    ));
                    if report.smallest_wavelength_target.is_finite() {
                        ui.small(format!(
                            "Forced wavelength wants {:.4}",
                            report.smallest_wavelength_target
                        ));
                    }
                }
                if let Some(report) = &self.amr_report {
                    ui.small(format!(
                        "Last adaptation: {} refinements · {} coarsenings · {} rejected collapses",
                        report
                            .topology_changes
                            .saturating_sub(report.coarsening_changes),
                        report.coarsening_changes,
                        report.skipped_collapses,
                    ));
                    ui.small(format!(
                        "{} refine passes · {} coarsen passes · {} work units",
                        report.refine_passes, report.collapse_passes, report.work_units,
                    ));
                    ui.small(format!(
                        "{:.0}% of the source triangles survived · {} still oversized",
                        100.0 * report.preserved_triangles as f64
                            / report.original_triangles.max(1) as f64,
                        report.remaining_oversized_triangles,
                    ));
                }
                if let Some(error) = &self.amr_error {
                    ui.colored_label(RED, error);
                }
            });
    }
    pub(super) fn handoff_section(&self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Handoff")
            .default_open(true)
            .show(ui, |ui| {
                if self.uploading.is_some() {
                    ui.label("Uploading the candidate to the GPU");
                } else if self.source_commit.is_some() {
                    ui.label("Applying source parameters at a solver boundary");
                } else if self
                    .gpu_upload_preparation
                    .as_ref()
                    .is_some_and(|job| job.result.is_none())
                {
                    ui.label("Packing candidate GPU buffers in the background");
                } else if self.gpu_upload_preparation.is_some() {
                    ui.label(format!(
                        "GPU buffers ready · draining {} requested steps",
                        self.step_backlog
                    ));
                } else if self.runtime.ready().is_some() {
                    ui.label(format!(
                        "Candidate ready · draining {} requested steps",
                        self.step_backlog
                    ));
                } else if self.preparation_in_progress() {
                    ui.label("Preparing a candidate on the CPU");
                } else {
                    ui.label("Idle");
                }
                ui.small("The committed field keeps running until a candidate is acknowledged.");
                match &self.last_handoff {
                    Some(record) => {
                        ui.small(format!(
                            "Last handoff: prepare {:.1} ms · pack {:.1} ms · drain {:.1} ms · upload {:.1} ms",
                            record.prepare_ms,
                            record.pack_ms,
                            record.drain_ms,
                            record.upload_ms,
                        ));
                        ui.small(timing_line(record.timing));
                        ui.small(match record.action {
                            TopologyMeshUpdateAction::Reuse => {
                                if record.adapted {
                                    "Adapted mesh · operator reassembled".to_owned()
                                } else {
                                    format!(
                                        "Reused the committed mesh · operator {}",
                                        if record.operator_reused {
                                            "reused"
                                        } else {
                                            "reassembled"
                                        }
                                    )
                                }
                            }
                            TopologyMeshUpdateAction::Repair(reason) => match record.carve {
                                Some(carve) => format!(
                                    "Mesh repaired: {} · kept {} · removed {} · inserted {}",
                                    reason.label(),
                                    carve.kept_triangles,
                                    carve.removed_triangles,
                                    carve.inserted_triangles,
                                ),
                                None => format!("Mesh repaired: {}", reason.label()),
                            },
                            TopologyMeshUpdateAction::FullRebuild(reason) => {
                                match &record.repair_fallback {
                                    Some(message) => {
                                        format!("Full rebuild: {} ({message})", reason.label())
                                    }
                                    None => format!("Full rebuild: {}", reason.label()),
                                }
                            }
                        });
                        ui.small(format!(
                            "{} · {} dofs · {} triangles",
                            if record.fresh {
                                "Fresh field".to_owned()
                            } else if record.transferred {
                                format!(
                                    "Field transferred · {} of {} nodes copied exactly",
                                    record.exact_nodes, record.degrees_of_freedom
                                )
                            } else {
                                "Field preserved in place".to_owned()
                            },
                            record.degrees_of_freedom,
                            record.triangles,
                        ));
                    }
                    None => {
                        ui.small("No handoff has completed yet.");
                    }
                }
            });
    }
    pub(super) fn solver_section(&self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Solver")
            .default_open(true)
            .show(ui, |ui| {
                ui.label(format!(
                    "GPU {} · {} dispatches",
                    self.gpu_status, self.gpu_dispatches
                ));
                match self.runtime.active() {
                    Some(active) => {
                        let dt = self.solver_time_step();
                        let solver_bytes = self
                            .canonical_gpu_bytes
                            .unwrap_or_else(|| active.operator.estimated_gpu_bytes());
                        ui.small(format!(
                            "{} dofs · {:.2} MiB canonical solver storage",
                            active.operator.degrees_of_freedom(),
                            solver_bytes as f64 / (1024.0 * 1024.0),
                        ));
                        ui.small(format!(
                            "dt {dt:.3e} · {:.0} steps/s · {:.2} simulated s per wall s",
                            self.steps_per_second,
                            self.steps_per_second * dt,
                        ));
                        ui.small(format!(
                            "Simulated time {:.4} s · {} completed steps",
                            self.simulated_time(),
                            self.completed_steps,
                        ));
                        if let Some(temporal) = &active.canonical_temporal_operator {
                            ui.small(time_step_bound_line(
                                temporal.time_step_bound(),
                                self.editor.document.model.accepted.physics,
                            ))
                            .on_hover_text(
                                "The stability ceiling covers every coefficient the medium can \
                                 reach: each drive's lowest phase, each Switch state and every \
                                 admitted field amplitude. A self-focusing law only slows the \
                                 wave, so it never lowers it.",
                            );
                        }
                    }
                    None => {
                        ui.small("Waiting for an accepted mesh and wave operator.");
                    }
                }
                let withheld = self.runtime.ready().is_some() || self.uploading.is_some();
                ui.small(format!(
                    "Backlog {} steps · {} per frame ceiling · scheduling {}",
                    self.step_backlog,
                    MAX_STEPS_PER_FRAME,
                    if withheld {
                        "withheld for a pending handoff"
                    } else if self.wave_running {
                        "running"
                    } else {
                        "paused"
                    },
                ));
                if let Some(energy) = self.wave_energy {
                    ui.small(format!(
                        "Canonical discrete energy {energy:.6e} · bulk + gaps + outgoing memory"
                    ));
                }
            });
    }
    /// What the transient channels said, kept after they were overwritten.
    pub(super) fn log_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Log")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .small_button("Clear")
                        .on_hover_text("Forget every entry below")
                        .clicked()
                    {
                        self.events.clear();
                    }
                    if ui
                        .small_button("Copy")
                        .on_hover_text("Copy the whole log, oldest first")
                        .clicked()
                    {
                        let text = self
                            .events
                            .iter()
                            .map(event_line)
                            .collect::<Vec<_>>()
                            .join("\n");
                        ui.ctx().copy_text(text);
                    }
                    ui.weak(format!("{} of {EVENT_LOG_ENTRIES}", self.events.len()));
                });
                if self.events.is_empty() {
                    ui.weak("Nothing recorded yet");
                    return;
                }
                egui::ScrollArea::vertical()
                    .max_height(160.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for entry in self.events.iter().rev() {
                            let line = event_line(entry);
                            match entry.source {
                                EventSource::Preparation | EventSource::Adaptation => {
                                    ui.colored_label(RED, line)
                                }
                                EventSource::Repair => ui.colored_label(GOLD, line),
                                EventSource::Status => ui.small(line),
                            };
                        }
                    });
            });
    }
    /// What the status bar says, and whose work is in flight: the user's
    /// (an edit's topology or preparation), the app's own (adaptation
    /// estimating, computing a mesh, or preparing the one it computed), or
    /// none. The latest request owns whatever is preparing or uploading, and
    /// an edit's request replaces an adaptation's.
    pub(super) fn status(&self) -> (String, Activity) {
        let phase = match self.editor.acceptance {
            TopologyAcceptance::Invalid(issue) => {
                return (format!("Geometry invalid: {issue}"), Activity::Idle);
            }
            TopologyAcceptance::Pending => {
                return (
                    "Topology rebuilding: tracing faces".into(),
                    Activity::Requested,
                );
            }
            TopologyAcceptance::Valid => self.preparation_phase(),
        };
        match phase {
            Some(phase) => {
                let text = self.preparation_detail().unwrap_or(phase.label());
                if phase == TopologyPreparationPhase::Failed {
                    (text.into(), Activity::Idle)
                } else if self.runtime.requested_adaptation() {
                    let mut characters = text.chars();
                    let lowered = characters
                        .next()
                        .map(|first| first.to_lowercase().chain(characters).collect::<String>())
                        .unwrap_or_default();
                    (format!("Adapting mesh: {lowered}"), Activity::Automatic)
                } else {
                    (text.into(), Activity::Requested)
                }
            }
            None if self.adaptation_in_flight() => ("Adapting mesh".into(), Activity::Automatic),
            None if self.estimate_in_flight() => ("Simulation ready".into(), Activity::Automatic),
            None => ("Simulation ready".into(), Activity::Idle),
        }
    }

    pub(super) fn status_bar(&mut self, root: &mut egui::Ui) {
        egui::Panel::bottom("status")
            .exact_size(29.0)
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    let (status, activity) = self.status();
                    if activity == Activity::Idle {
                        ui.label(status);
                    } else {
                        let base = match activity {
                            Activity::Requested => GOLD,
                            _ => ui.visuals().text_color(),
                        };
                        let font = egui::TextStyle::Body.resolve(ui.style());
                        let time = ui.input(|input| input.time);
                        ui.label(sheen_text(&status, base, activity, font, time));
                        // The band moves only while frames are drawn; a paused
                        // simulation draws none, and 30 a second is plenty for
                        // a band this soft.
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(33));
                    }
                    if self.recording_state == RecordingState::SelectingDestination {
                        ui.colored_label(GOLD, "Choosing recording destination…");
                    } else if matches!(
                        self.recording_state,
                        RecordingState::Requested
                            | RecordingState::Preparing
                            | RecordingState::Starting
                    ) {
                        ui.colored_label(GOLD, "Preparing recording…");
                    } else if self.recording_state == RecordingState::Recording {
                        let elapsed = self
                            .recording_started
                            .map(recording::elapsed_label)
                            .unwrap_or_else(|| "00:00".into());
                        ui.colored_label(RED, format!("● REC {elapsed}"));
                        if self.recording_dropped_frames > 0 {
                            ui.colored_label(
                                GOLD,
                                format!("{} dropped", self.recording_dropped_frames),
                            );
                        }
                        if ui.small_button("Stop").clicked() {
                            self.stop_video_recording();
                        }
                    } else if self.recording_state == RecordingState::Finalizing {
                        ui.colored_label(GOLD, "Finalizing recording…");
                    } else if self.snapshot_state != SnapshotState::Idle {
                        ui.colored_label(GOLD, "Capturing snapshot…");
                    }
                    if !self.message.is_empty() {
                        ui.separator();
                        ui.label(&self.message);
                    }
                    let warning = self.diagnostics_warning();
                    let summary = self.summary_line();
                    let toggled = ui
                        .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let clicked = ui
                                .add(egui::Button::new(summary).frame(false))
                                .on_hover_text("Open performance diagnostics")
                                .clicked();
                            if warning {
                                ui.colored_label(GOLD, "⚠").on_hover_text(
                                    "Preparation or adaptation reported an error; the diagnostics log keeps it",
                                );
                            }
                            clicked
                        })
                        .inner;
                    if toggled {
                        self.diagnostics_open = !self.diagnostics_open;
                    }
                });
            });
    }
}

/// What sets a time-driven generation's step ceiling, in the skin's names.
fn time_step_bound_line(bound: CanonicalTimeStepBound, physics: PhysicsModel) -> String {
    let mut lowered = [
        (LawPresetRow::Mass, bound.primary_floor),
        (LawPresetRow::Stiffness, bound.complementary_floor),
    ]
    .into_iter()
    .filter(|(_, floor)| *floor < 1.0)
    .map(|(row, floor)| {
        format!(
            "{} falls to {floor:.2}× at its lowest",
            law_row_label(physics, row)
        )
    })
    .collect::<Vec<_>>();
    // Gate O: a restoring law is not a coefficient, but its curvature
    // tightens the step as `1/dt² = 1/dt_bound² + V″/4`.
    if bound.restoring_curvature > 0.0 {
        lowered.push(format!(
            "the restoring law's curvature V″ reaches {:.3}",
            bound.restoring_curvature
        ));
    }
    if lowered.is_empty() {
        return format!(
            "Step ceiling {:.3e} s, the fixed medium's: no law here lowers it",
            bound.trajectory
        );
    }
    format!(
        "Step ceiling {:.3e} s, {:.2}× the fixed medium's: {}",
        bound.trajectory,
        bound.trajectory / bound.fixed,
        lowered.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material_overlay::MaterialProperty;
    use crate::ui::test_support::*;
    use funfern_app::topology_editor::TopologyEditor;
    use std::sync::Arc;

    /// The status bar tells the user's work from the app's own: an edit's
    /// topology and preparation, through to the upload, are the user's; the
    /// preparation of an adapted mesh, an adaptation computing one and an
    /// error estimate are the app's; an edit made meanwhile is the user's
    /// again; and a finished or failed preparation is neither.
    #[test]
    fn the_status_bar_tells_the_users_work_from_the_apps_own() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        settle(&mut state.editor);
        let active = activate(&mut state);
        assert_eq!(state.status(), ("Simulation ready".into(), Activity::Idle));

        let options = MeshingOptions {
            target_edge_length: 0.18,
            ..MeshingOptions::default()
        };
        let request = |state: &mut Playground| {
            state
                .runtime
                .request(
                    state.editor.revision,
                    &state.editor.document,
                    state.editor.compiled_accepted.clone(),
                    options,
                    false,
                )
                .unwrap()
        };
        let finish = |state: &mut Playground, token| {
            while state.runtime.advance(4096).is_none() {}
            let ready = state.status();
            state.runtime.commit_ready(token).unwrap();
            ready
        };

        let token = request(&mut state);
        assert_eq!(state.status().1, Activity::Requested);
        let ready = finish(&mut state, token);
        assert_eq!(ready, ("Ready for GPU upload".into(), Activity::Requested));
        assert_eq!(state.status().1, Activity::Idle);

        let mut mesh = active.mesh.as_ref().clone();
        mesh.mesh_revision = state.runtime.reserve_mesh_revision();
        state
            .runtime
            .request_adapted(state.editor.revision, &state.editor.document, mesh)
            .unwrap();
        let (text, activity) = state.status();
        assert_eq!(activity, Activity::Automatic);
        assert!(text.starts_with("Adapting mesh: "), "{text}");
        let token = request(&mut state);
        assert_eq!(state.status().1, Activity::Requested);
        finish(&mut state, token);

        let active = state.runtime.active().unwrap().clone();
        state.amr_adaptation_job = Some(MeshAdaptationJob::new_topology(
            active.mesh.clone(),
            &active.bundle.plan,
            MeshAdaptationState::from_mesh(&active.mesh),
            state.runtime.reserve_mesh_revision(),
            Arc::new(|_, _| 0.09),
            MeshAdaptationOptions::default(),
        ));
        assert_eq!(
            state.status(),
            ("Adapting mesh".into(), Activity::Automatic)
        );
        state.amr_adaptation_job = None;

        state.editor.acceptance = TopologyAcceptance::Pending;
        assert_eq!(state.status().1, Activity::Requested);
    }

    /// The sheen: idle text keeps its colour; a busy line's band lifts the
    /// characters under it towards a tint and leaves distant ones alone; and
    /// the app's own work shimmers present but far fainter than the user's.
    #[test]
    fn the_sheen_is_plain_for_the_users_work_and_faint_for_the_apps() {
        let apart = |a: Color32, b: Color32| {
            [(a.r(), b.r()), (a.g(), b.g()), (a.b(), b.b())]
                .into_iter()
                .map(|(x, y)| x.abs_diff(y))
                .max()
                .unwrap()
        };
        let strongest = |activity| {
            (0..400)
                .flat_map(|step| {
                    (0..20).map(move |index| {
                        apart(
                            GOLD,
                            sheen_color(GOLD, activity, index, 20, step as f64 * 0.01),
                        )
                    })
                })
                .max()
                .unwrap()
        };
        assert_eq!(strongest(Activity::Idle), 0);
        let (requested, automatic) = (
            strongest(Activity::Requested),
            strongest(Activity::Automatic),
        );
        assert!(automatic > 0, "the app's own work still shimmers");
        assert!(requested > 3 * automatic, "{requested} against {automatic}");
        // At t = 0 the band sits before the text: the far end is untouched.
        assert_eq!(sheen_color(GOLD, Activity::Requested, 19, 20, 0.0), GOLD);
    }

    /// Every shape the catalogue offers is explained in the Laws section, so
    /// the reference cannot fall behind the presets.
    #[test]
    fn the_laws_reference_covers_every_offered_law() {
        let text = FORMULA_LAWS
            .iter()
            .map(|(form, meaning)| format!("{form} {meaning}"))
            .collect::<String>();
        for name in [
            "Kerr",
            "Saturable",
            "Pump",
            "Time crystal",
            "Travelling",
            "Switch",
            "Divide",
        ] {
            assert!(text.contains(name), "{name} is missing from the Laws help");
        }
        for preset in law_presets() {
            let covered = match preset.name {
                "Linear" => true,
                "Switchable medium" | "Reflectionless time interface" => text.contains("Switch"),
                "Parametric pump" => text.contains("Pump"),
                "Time crystal" => text.contains("Time crystal"),
                "Travelling modulation" => text.contains("Travelling"),
                "Kerr medium" => text.contains("Kerr"),
                "Saturable medium" => text.contains("Saturable"),
                other => panic!("the Laws help does not know the {other} preset"),
            };
            assert!(covered, "{}", preset.name);
        }
        for preset in funfern_core::restoring_presets() {
            let covered = match preset.id {
                "" => true,
                "R1" => text.contains("Klein-Gordon"),
                "R2" => text.contains("sine-Gordon"),
                "R3" => text.contains("φ⁴"),
                other => panic!("the Laws help does not know restoring law {other}"),
            };
            assert!(covered, "{}", preset.name);
        }
        assert!(text.contains("van der Pol") && text.contains("r = ∫u dt"));
    }

    #[test]
    fn the_step_ceiling_line_names_the_row_that_lowers_it() {
        let physics = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        };
        let pumped = CanonicalTimeStepBound {
            fixed: 2.0e-3,
            trajectory: 2.0e-3 * 0.8_f64.sqrt(),
            primary_floor: 0.8,
            complementary_floor: 1.0,
            restoring_curvature: 0.0,
        };
        assert_eq!(
            time_step_bound_line(pumped, physics),
            "Step ceiling 1.789e-3 s, 0.89× the fixed medium's: Permittivity ε falls to \
             0.80× at its lowest"
        );
        let kerr = CanonicalTimeStepBound {
            trajectory: 2.0e-3,
            primary_floor: 1.0,
            ..pumped
        };
        assert_eq!(
            time_step_bound_line(kerr, physics),
            "Step ceiling 2.000e-3 s, the fixed medium's: no law here lowers it"
        );
        let sine_gordon = CanonicalTimeStepBound {
            trajectory: 1.0e-3,
            restoring_curvature: 9.0,
            ..kerr
        };
        assert_eq!(
            time_step_bound_line(sine_gordon, physics),
            "Step ceiling 1.000e-3 s, 0.50× the fixed medium's: the restoring law's curvature \
             V″ reaches 9.000"
        );
    }

    #[test]
    fn the_formula_reference_names_what_the_parser_accepts() {
        let origin = MaterialCoordinates {
            x: 0.0,
            y: 0.0,
            r: 0.0,
            theta: 0.0,
        };
        for (names, _) in FORMULA_SYMBOLS {
            for name in names.split(", ") {
                // An unknown name parses as a material parameter and only fails
                // once it is evaluated without one, so the evaluation is what
                // proves the reference still names something built in.
                let field = ScalarField::formula(name).expect("a listed symbol parses");
                assert!(
                    field.evaluate(origin, &[]).is_ok(),
                    "the reference lists `{name}`, which the parser does not know"
                );
            }
        }
        for (signature, _, example) in FORMULA_FUNCTIONS {
            let field = ScalarField::formula(example)
                .unwrap_or_else(|error| panic!("{signature}: `{example}` is rejected: {error}"));
            assert!(
                field.evaluate(origin, &[]).is_ok(),
                "{signature}: `{example}` does not evaluate"
            );
        }
        for retired in ["ln(1 + r)", "pow(r, 2)"] {
            assert!(
                ScalarField::formula(retired).is_err(),
                "`{retired}` parses, so the reference should be listing it"
            );
        }
    }

    /// Every channel the log watches overwrites itself, so a change is the only
    /// moment its value can be caught. Holding a value logs it once, clearing
    /// and returning without anything in between counts a repeat rather than
    /// filling the ring, and an error keeps the status marker lit until the
    /// window is opened on it.
    #[test]
    fn the_log_keeps_one_entry_per_change_of_a_transient_channel() {
        let mut state = Playground::default();
        state.record_events(1.0);
        assert!(state.events.is_empty());
        assert!(!state.diagnostics_warning());

        state.message = "Pulse queued in region 1".into();
        state.record_events(2.0);
        state.record_events(3.0);
        assert_eq!(state.events.len(), 1, "a held value is logged once");
        assert_eq!(state.events[0].source, EventSource::Status);
        assert_eq!(state.events[0].repeats, 1);
        assert!(
            !state.diagnostics_warning(),
            "a status line is not an error"
        );

        state.amr_error = Some("Invalid adaptation source".into());
        state.record_events(4.0);
        assert_eq!(state.events.len(), 2);
        assert_eq!(state.events[1].source, EventSource::Adaptation);
        assert!(state.unseen_error);

        // The adaptation clears itself and fails again with nothing logged in
        // between, which is the shape that would otherwise flood the ring.
        for time in [5.0, 6.0, 7.0, 8.0] {
            state.amr_error = (time as u64)
                .is_multiple_of(2)
                .then(|| "Invalid adaptation source".to_owned());
            state.record_events(time);
        }
        assert_eq!(state.events.len(), 2, "{:?}", state.events);
        assert_eq!(state.events[1].repeats, 3);

        // A different message is its own entry, and the marker survives the
        // error clearing.
        state.amr_error = None;
        state.message = "Simulation topology committed".into();
        state.record_events(9.0);
        assert_eq!(state.events.len(), 3);
        assert_eq!(state.events[2].source, EventSource::Status);
        assert!(
            state.diagnostics_warning(),
            "the marker stays lit for an error the window has not been opened on"
        );

        state.unseen_error = false;
        assert!(!state.diagnostics_warning());
        assert_eq!(
            event_line(&state.events[1]),
            "0:08.0 · adaptation · Invalid adaptation source ×3"
        );

        // A repair fallback is queued by the transaction that reported it and
        // stamped on the next frame, so two transactions that fall back the
        // same way are counted rather than reading as one.
        state.pending_repairs = vec!["mesh repair failed".into(), "mesh repair failed".into()];
        state.record_events(10.0);
        assert!(state.pending_repairs.is_empty());
        assert_eq!(state.events.len(), 4);
        assert_eq!(state.events[3].source, EventSource::Repair);
        assert_eq!(state.events[3].repeats, 2);
        assert!(
            !state.diagnostics_warning(),
            "a rebuild that carried the edit through is not an error"
        );
    }
    /// A thumbnail is rasterized, not traced, so a face covers area rather than
    /// only an outline, and every quad lands inside the scene it came from.
    #[test]
    fn a_thumbnail_fills_the_faces_of_the_scene_it_previews() {
        let example = &funfern_app::topology_examples::catalog()[0];
        let preview = build_example_preview(example);
        let domain = example.document.model.accepted.geometry.domain;
        assert!(preview.quads.len() > PREVIEW_ROWS, "a face was not filled");
        assert!(!preview.strokes.is_empty(), "nothing was outlined");
        for quad in &preview.quads {
            assert!(quad.low.x < quad.high.x && quad.low.y < quad.high.y);
            assert!(
                quad.low.x >= domain.min_x - 1e-9
                    && quad.high.x <= domain.max_x + 1e-9
                    && quad.low.y >= domain.min_y - 1e-9
                    && quad.high.y <= domain.max_y + 1e-9,
                "a quad left the domain"
            );
        }
        let colors = preview
            .quads
            .iter()
            .map(|quad| quad.color.to_array())
            .collect::<BTreeSet<_>>();
        assert!(
            colors.len() > 1,
            "the obstacle is the same colour as the background it sits in"
        );
    }

    /// The reason a thumbnail samples cell centres rather than polygon corners:
    /// every corner of a Luneburg lens sits on the same circle, so a profile
    /// read at the corners alone is one flat colour.
    #[test]
    fn a_radial_material_profile_reaches_the_thumbnail() {
        let example = funfern_app::topology_examples::catalog()
            .iter()
            .find(|example| example.name == "Luneburg lens")
            .expect("the catalog still carries the Luneburg lens");
        assert!(matches!(
            example.document.presentation.material_overlay,
            MaterialOverlay::Property(MaterialProperty::Density)
        ));
        let preview = build_example_preview(example);
        let shades = preview
            .quads
            .iter()
            .map(|quad| quad.color.to_array())
            .collect::<BTreeSet<_>>();
        assert!(
            shades.len() > 8,
            "the lens reads as {} colour(s), not a profile",
            shades.len()
        );
    }

    /// Even-odd pairing across every cycle at once is what keeps a face out of
    /// its own holes, and a slit traced out and back contributes nothing.
    #[test]
    fn scanline_spans_skip_the_holes_in_a_face() {
        let outer = vec![
            Point2::new(-1.0, -1.0),
            Point2::new(1.0, -1.0),
            Point2::new(1.0, 1.0),
            Point2::new(-1.0, 1.0),
        ];
        let hole = vec![
            Point2::new(-0.5, -0.5),
            Point2::new(-0.5, 0.5),
            Point2::new(0.5, 0.5),
            Point2::new(0.5, -0.5),
        ];
        assert_eq!(
            face_spans(std::slice::from_ref(&outer), 0.0),
            vec![(-1.0, 1.0)]
        );
        assert_eq!(
            face_spans(&[outer.clone(), hole], 0.0),
            vec![(-1.0, -0.5), (0.5, 1.0)]
        );
        // A line above the face crosses nothing.
        assert!(face_spans(&[outer], 2.0).is_empty());
    }

    /// New is a scene that can be run, not just an empty one.
    #[test]
    fn a_new_scene_is_empty_outgoing_and_driven() {
        let mut state = Playground::default();
        state.new_scene();
        let scene = &state.editor.document.model.accepted;
        assert!(scene.geometry.curves.is_empty());
        assert_eq!(scene.regions.len(), 1);
        for side in OuterSide::ALL {
            assert_eq!(
                scene.outer_boundaries.sides[side.index()],
                OuterBoundaryCondition::SecondOrderOutgoing
            );
        }
        assert!(state.editor.document.model.source.enabled);
        assert_eq!(state.example_opened, None, "a new scene is not an example");
        assert!(state.editor.undo(), "New is one undoable action");
    }

    /// A full gallery row holds the largest section, a width one short of a
    /// tile drops one, and a screen narrowed to nothing still shows one.
    #[test]
    fn a_gallery_row_holds_a_whole_section() {
        let catalog = funfern_app::topology_examples::catalog();
        for group in ExampleGroup::ALL {
            let members = catalog.iter().filter(|e| e.group == group).count();
            assert!(
                members <= GALLERY_COLUMNS,
                "{} needs two rows",
                group.label()
            );
        }
        let full = gallery_width(GALLERY_COLUMNS);
        assert_eq!(gallery_columns(full), GALLERY_COLUMNS);
        assert_eq!(gallery_columns(full - 1.0), GALLERY_COLUMNS - 1);
        assert_eq!(gallery_columns(0.0), 1);
    }

    /// A pick closes the gallery, so the scene is not left behind it, and
    /// the pick is what changes the document.
    #[test]
    fn picking_an_example_closes_the_gallery_and_marks_the_tile() {
        let mut state = Playground {
            examples_open: true,
            ..Playground::default()
        };
        let index = funfern_app::topology_examples::catalog()
            .iter()
            .position(|example| example.name == "Double slit")
            .unwrap();
        state.pick_example(index);
        assert!(!state.examples_open, "the gallery stayed over the scene");
        assert_eq!(state.example_opened, Some(index));
        assert_eq!(
            state.editor.document.model,
            funfern_app::topology_examples::catalog()[index]
                .document
                .model
        );
        state.new_scene();
        assert_eq!(state.example_opened, None, "the marker outlived its scene");
    }

    /// Every catalog entry has to reach the gallery, and building them all is
    /// spread over frames because the largest one costs a frame by itself.
    #[test]
    fn every_example_previews_and_the_cache_fills_one_per_frame() {
        let mut state = Playground::default();
        let total = funfern_app::topology_examples::catalog().len();
        assert_eq!(state.example_previews.len(), total);
        assert!(state.example_previews.iter().all(Option::is_none));
        for filled in 1..=total {
            let index = state
                .example_previews
                .iter()
                .position(Option::is_none)
                .expect("an unbuilt preview");
            state.example_previews[index] = Some(build_example_preview(
                &funfern_app::topology_examples::catalog()[index],
            ));
            assert_eq!(
                state
                    .example_previews
                    .iter()
                    .filter(|p| p.is_some())
                    .count(),
                filled
            );
        }
        for (index, preview) in state.example_previews.iter().enumerate() {
            let preview = preview.as_ref().unwrap();
            assert!(
                !preview.quads.is_empty(),
                "{} previews as nothing",
                funfern_app::topology_examples::catalog()[index].name
            );
        }
    }

    /// The random start has to land on every example and never off the end.
    #[test]
    fn the_random_start_stays_inside_the_catalog() {
        let total = funfern_app::topology_examples::catalog().len();
        let mut seen = BTreeSet::new();
        for step in 0..=1000 {
            let index = random_example_index(f64::from(step) / 1000.0, total);
            assert!(index < total, "{step} lands past the catalog");
            seen.insert(index);
        }
        assert_eq!(seen.len(), total, "some example can never open at startup");
        assert!((0.0..1.0).contains(&random_fraction()));
    }
}
