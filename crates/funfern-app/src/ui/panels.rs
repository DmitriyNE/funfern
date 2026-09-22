//! The View and Simulation inspectors, including the adaptation accuracy
//! controls and the advanced settings folded away beneath them.

use crate::material_overlay::{MaterialOverlay, MaterialProperty};
use bevy::prelude::*;
use bevy_egui::egui::{self};
use funfern_app::document::VectorOverlay;
use funfern_core::*;

use super::*;

impl Playground {
    pub(super) fn view_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("View");
        let field_reference = self.field_exposure.reference();
        let overlay_error = self.material_overlay_error.clone();
        let overlay_progress = self.material_overlay_job.as_ref().map(|job| job.progress());
        let overlay_invalid = match self.editor.document.presentation.material_overlay {
            MaterialOverlay::Property(property) => self
                .material_overlay_snapshot
                .as_ref()
                .map(|snapshot| snapshot.invalid_count(property)),
            _ => None,
        };
        let p = &mut self.editor.document.presentation;
        ui.checkbox(&mut p.grid, "Grid");
        ui.checkbox(&mut p.control_polygons, "Control polygons");
        ui.checkbox(&mut p.handles, "Handles");
        ui.checkbox(&mut p.boundary_conditions, "Boundary conditions");
        ui.checkbox(&mut p.mesh, "Mesh");
        ui.checkbox(&mut p.mesh_boundaries, "Mesh boundaries");
        ui.checkbox(&mut p.field, "Field");
        ui.add(egui::Slider::new(&mut p.field_gain, 0.25..=12.0).text("Field intensity"));
        if p.field {
            ui.checkbox(&mut p.field_auto_exposure, "Auto exposure")
                .on_hover_text(
                    "Scale the field's colours from what is on screen, so a quiet scene reads \
                     like a loud one. Off, the intensity slider is the whole scale.",
                );
            // The colours are relative, so the level they are relative to has to
            // be readable somewhere or a decaying field looks like a steady one.
            if p.field_auto_exposure
                && let Some(reference) = field_reference
            {
                ui.small(format!("Auto scale {reference:.2e}"));
            }
        }
        let physics = self.editor.document.model.draft.physics;
        p.vector_overlay = p.vector_overlay.resolved(physics);
        ui.separator();
        ui.label("Vector overlay");
        egui::ComboBox::from_id_salt("vector-overlay")
            .selected_text(p.vector_overlay.label(physics))
            .show_ui(ui, |ui| {
                for mode in VectorOverlay::choices(physics) {
                    ui.selectable_value(&mut p.vector_overlay, *mode, mode.label(physics));
                }
            });
        if p.vector_overlay != VectorOverlay::Off {
            if p.vector_overlay == VectorOverlay::ComplementaryField {
                ui.checkbox(&mut p.vector_overlay_ac_coupled, "AC-couple arrows")
                    .on_hover_text(
                        "Subtract a slowly varying presentation baseline from the arrows. \
                         The canonical field, probes and energy remain unchanged.",
                    );
            }
            ui.add(
                egui::Slider::new(&mut p.vector_overlay_density, 28.0..=120.0)
                    .text("Arrow spacing"),
            );
            ui.add(egui::Slider::new(&mut p.vector_overlay_gain, 0.1..=5.0).text("Arrow gain"));
        }
        ui.separator();
        ui.label("Overlay");
        egui::ComboBox::from_id_salt("overlay")
            .selected_text(
                p.material_overlay
                    .label_for(self.editor.document.model.draft.physics),
            )
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut p.material_overlay, MaterialOverlay::Off, "Off");
                for overlay in [
                    MaterialOverlay::Regions,
                    MaterialOverlay::Subdomains,
                    MaterialOverlay::AdaptationTarget,
                ] {
                    ui.selectable_value(&mut p.material_overlay, overlay, overlay.label());
                }
                for property in [
                    MaterialProperty::Density,
                    MaterialProperty::Stiffness,
                    MaterialProperty::Damping,
                    MaterialProperty::WaveSpeed,
                    MaterialProperty::Impedance,
                    MaterialProperty::Anisotropy,
                    MaterialProperty::VolumeSource,
                ] {
                    ui.selectable_value(
                        &mut p.material_overlay,
                        MaterialOverlay::Property(property),
                        property.label_for(self.editor.document.model.draft.physics),
                    );
                }
            });
        ui.add(
            egui::Slider::new(&mut p.material_overlay_opacity, 0.05..=1.0)
                .text("Overlay intensity"),
        );
        if p.material_overlay == MaterialOverlay::AdaptationTarget {
            ui.small("Blue where the estimate wants the finest elements, orange the coarsest");
            // The overlay has three ways of being empty and none of them used to
            // say anything, which is how it came to look broken.
            if !p.adaptation.enabled {
                ui.colored_label(GOLD, "Adaptation is off in Simulation");
            } else {
                match &self.amr_indicator_result {
                    None => {
                        ui.colored_label(GOLD, format!("No estimate yet · {}", self.amr_status))
                    }
                    Some(result) => {
                        let triangles = self
                            .runtime
                            .active()
                            .map(|active| active.mesh.triangles.len());
                        if triangles.is_some_and(|count| count != result.element_targets.len()) {
                            ui.colored_label(GOLD, "The estimate is behind the current mesh")
                        } else {
                            ui.small(format!(
                                "Targets {:.3}–{:.3}",
                                result.report.minimum_target, result.report.maximum_target
                            ))
                        }
                    }
                };
            }
        }
        if matches!(p.material_overlay, MaterialOverlay::Property(_)) {
            ui.checkbox(&mut p.material_overlay_auto_range, "Automatic range");
            ui.checkbox(&mut p.material_overlay_logarithmic, "Logarithmic scale");
            if !p.material_overlay_auto_range {
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut p.material_overlay_manual_min).prefix("Min "));
                    ui.add(egui::DragValue::new(&mut p.material_overlay_manual_max).prefix("Max "));
                });
            }
            if let Some(error) = &overlay_error {
                ui.colored_label(RED, error);
            } else if let Some((done, total)) = overlay_progress {
                ui.label(format!("Sampling property… {done}/{total}"));
            } else if let Some(invalid) = overlay_invalid.filter(|count| *count > 0) {
                ui.colored_label(RED, format!("{invalid} invalid samples"));
            }
        }
        ui.separator();
        ui.label("Probes");
        ui.checkbox(&mut p.point_probes, "Points");
        ui.checkbox(&mut p.line_probes, "Lines");
        ui.checkbox(&mut p.boundary_probes, "Boundaries");
        ui.checkbox(&mut p.area_probes, "Areas");
        ui.checkbox(&mut p.far_field_contour, "Far field");
        ui.checkbox(&mut p.probe_labels, "Probe names");
    }
    /// The rate the solver is actually reaching, when it is short of the one
    /// asked for and is genuinely trying to reach it.
    pub(super) fn simulation_speed_shortfall(&self) -> Option<f64> {
        // Not suppressed while a handoff is pending. Adaptation keeps one
        // pending much of the time on exactly the scenes heavy enough to fall
        // short, which silenced the note when it mattered most; the held rate
        // already rides over the few frames a handoff withholds.
        let stepping = self.runtime.active().is_some() && self.wave_running;
        speed_shortfall(
            self.speed_reached,
            self.editor.document.presentation.simulation_speed,
            stepping,
        )
    }

    pub(super) fn simulation_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Simulation");
        let mut physics = self.editor.document.model.draft.physics;
        egui::ComboBox::from_id_salt("physics")
            .selected_text(match physics {
                PhysicsModel::Mechanical => "Mechanical",
                PhysicsModel::Electromagnetic {
                    polarization: ElectromagneticPolarization::Tm,
                } => "EM · TM",
                PhysicsModel::Electromagnetic {
                    polarization: ElectromagneticPolarization::Te,
                } => "EM · TE",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut physics, PhysicsModel::Mechanical, "Mechanical");
                ui.selectable_value(
                    &mut physics,
                    PhysicsModel::Electromagnetic {
                        polarization: ElectromagneticPolarization::Tm,
                    },
                    "EM · TM",
                );
                ui.selectable_value(
                    &mut physics,
                    PhysicsModel::Electromagnetic {
                        polarization: ElectromagneticPolarization::Te,
                    },
                    "EM · TE",
                );
            });
        if physics != self.editor.document.model.draft.physics {
            if let Err(error) = self.editor.set_physics(physics) {
                self.notify(error)
            }
        }
        ui.separator();
        ui.label("Speed");
        ui.add(
            egui::Slider::new(
                &mut self.editor.document.presentation.simulation_speed,
                0.02..=2.0,
            )
            .logarithmic(true)
            .suffix("×")
            .text("Simulated s per wall s"),
        )
        .on_hover_text(
            "A ceiling on how fast simulated time runs against the clock. The solver falls \
             short of it when a step costs more than a frame can afford.",
        );
        if let Some(reached) = self.simulation_speed_shortfall() {
            ui.colored_label(GOLD, format!("Reaching {reached:.2}×"))
                .on_hover_text(
                    "The scene costs more per step than the frame budget allows, so simulated \
                     time runs slower than asked. A coarser mesh buys it back.",
                );
        }
        ui.separator();
        ui.label("Mesh resolution");
        const PRESETS: [(f64, &str); 3] = [(0.16, "Coarse"), (0.08, "Medium"), (0.04, "Fine")];
        let preset_name = |edge: f64| {
            PRESETS
                .iter()
                .find(|(value, _)| (edge - value).abs() < 1.0e-9)
                .map_or("Custom", |(_, name)| name)
        };
        egui::ComboBox::from_id_salt("mesh_resolution")
            .width(ui.available_width())
            .selected_text(format!(
                "{} · target edge {:.3}",
                preset_name(self.editor.document.presentation.mesh_edge),
                self.editor.document.presentation.mesh_edge
            ))
            .show_ui(ui, |ui| {
                for (value, name) in PRESETS {
                    ui.selectable_value(
                        &mut self.editor.document.presentation.mesh_edge,
                        value,
                        format!("{name} · h ≤ {value:.2}"),
                    );
                }
            });
        let slider = ui.add(
            egui::Slider::new(
                &mut self.editor.document.presentation.mesh_edge,
                0.02..=0.25,
            )
            .logarithmic(true)
            .text("Target edge"),
        );
        self.mesh_edge_dragging = slider.dragged();
        ui.horizontal(|ui| {
            if ui
                .button("Remesh")
                .on_hover_text(
                    "Rebuild the mesh at this resolution, leaving any adapted mesh behind",
                )
                .clicked()
            {
                self.remesh_requested = true;
            }
            if let Some(active) = self.runtime.active()
                && (active.meshing.target_edge_length - self.editor.document.presentation.mesh_edge)
                    .abs()
                    > 1.0e-9
            {
                ui.small(format!(
                    "Active {:.3} · requested {:.3}",
                    active.meshing.target_edge_length, self.editor.document.presentation.mesh_edge
                ));
            }
        });
        ui.separator();
        let before_amr = self.amr_settings();
        ui.checkbox(
            &mut self.editor.document.presentation.adaptation.enabled,
            "Adapt mesh to the wave",
        );
        ui.add_enabled_ui(self.editor.document.presentation.adaptation.enabled, |ui| {
            let preset = amr_accuracy_preset_name(
                self.editor
                    .document
                    .presentation
                    .adaptation
                    .accuracy_percent,
            );
            egui::ComboBox::from_id_salt("amr_accuracy")
                .width(ui.available_width())
                .selected_text(format!(
                    "{preset} · target accuracy {:.0}%",
                    self.editor
                        .document
                        .presentation
                        .adaptation
                        .accuracy_percent
                ))
                .show_ui(ui, |ui| {
                    for (value, name) in AMR_ACCURACY_PRESETS {
                        ui.selectable_value(
                            &mut self
                                .editor
                                .document
                                .presentation
                                .adaptation
                                .accuracy_percent,
                            value,
                            format!("{name} · {value:.0}%"),
                        );
                    }
                });
            ui.add(
                egui::Slider::new(
                    &mut self
                        .editor
                        .document
                        .presentation
                        .adaptation
                        .accuracy_percent,
                    2.0..=50.0,
                )
                .logarithmic(true)
                .suffix("%")
                .text("Target accuracy"),
            )
            .on_hover_text(
                "How much estimated error the whole field is allowed to carry. \
                 Refinement the error estimate asks for stops once the field is inside \
                 this; carrying a forced wavelength and staying under the largest \
                 element allowed are floors, and go on regardless.",
            );
            ui.small(&self.amr_status);
            ui.small(self.amr_estimate_line());
            // Nothing else in the panel explains a mesh pinned at its floor
            // while the accuracy target reads satisfied. It is a standing
            // condition rather than a passing one, so it may take its own line.
            if let Some(result) = &self.amr_indicator_result
                && result.report.smallest_wavelength_target
                    < self.editor.document.presentation.adaptation.minimum_edge
            {
                ui.colored_label(
                    GOLD,
                    format!(
                        "The forcing wants elements of {:.3}, under the smallest allowed \
                         of {:.3}, so the mesh sits at its floor whatever the accuracy asks",
                        result.report.smallest_wavelength_target,
                        self.editor.document.presentation.adaptation.minimum_edge,
                    ),
                );
            }
            if let Some(error) = &self.amr_error {
                ui.colored_label(RED, error);
            }
        });
        ui.separator();
        ui.label("Continuous source");
        let mut source = self.editor.document.model.source;
        let before = source;
        ui.checkbox(&mut source.enabled, "Enabled");
        let (_, amplitude, frequency, phase) = source.signal.harmonic_parameters_mut();
        ui.add(
            egui::DragValue::new(amplitude)
                .speed(0.05)
                .prefix("Amplitude "),
        );
        ui.add(
            egui::DragValue::new(frequency)
                .speed(0.1)
                .range(0.0..=1.0e5)
                .prefix("Frequency "),
        );
        ui.add(egui::DragValue::new(phase).speed(0.05).prefix("Phase "));
        ui.add(
            egui::DragValue::new(&mut source.width)
                .speed(0.002)
                .range(0.001..=1.0)
                .prefix("Width "),
        );
        ui.small(
            "Version-22 acceleration control: the canonical solver integrates this signal \
             analytically into a primary-field-rate drive and applies the accepted \
             generation's immutable reference mass.",
        );
        if source != before {
            if let Err(error) = self.editor.set_point_source(source) {
                self.notify(error)
            }
        }
        ui.separator();
        // Framed, because arming a mode is an action. A bare selectable label
        // reads as a caption until it is switched on.
        if ui
            .add(egui::Button::new("Place pulse").selected(self.pulse_mode))
            .on_hover_text("Click in the scene to drop a pulse; click here again to stop")
            .clicked()
        {
            self.pulse_mode = !self.pulse_mode;
            self.probe_mode = None;
        }
        ui.add(egui::Slider::new(&mut self.pulse_amplitude, -5.0..=5.0).text("Pulse amplitude"));
        ui.add(egui::Slider::new(&mut self.pulse_width, 0.01..=0.25).text("Pulse width"));
        if self.runtime.active().is_some() {
            ui.separator();
            ui.label(format!(
                "Canonical stored energy: {}",
                self.wave_energy.map_or("—".into(), |v| format!("{v:.4e}"))
            ));
        }
        ui.separator();
        self.advanced_settings(ui);
        if self.amr_settings() != before_amr {
            self.cancel_background_amr();
            self.amr_indicator_job = None;
            self.amr_indicator_completed = None;
            self.amr_indicator_source = None;
            self.amr_adaptation_job = None;
            self.amr_adaptation_completed = None;
            self.amr_adaptation_source = None;
            self.amr_pending_state = None;
            self.amr_last_analyzed_step = None;
            self.amr_last_started = None;
            self.amr_coarsen_streak = 0;
            self.amr_error = None;
        }
    }

    /// What the panel says about the estimate, whether or not one is in hand.
    /// Committing a mesh drops the estimate it was measured against, so a line
    /// that exists only while there is one comes and goes with every adaptation
    /// and shifts the panel out from under the pointer. It holds its place and
    /// says it has nothing instead.
    pub(super) fn amr_estimate_line(&self) -> String {
        match &self.amr_indicator_result {
            Some(result) if result.report.dormant => format!(
                "Estimated error dormant · target {:.0}%",
                self.editor
                    .document
                    .presentation
                    .adaptation
                    .accuracy_percent
            ),
            Some(result) => format!(
                "Estimated error {:.1}% · target {:.0}%",
                100.0 * result.report.global_indicator,
                self.editor
                    .document
                    .presentation
                    .adaptation
                    .accuracy_percent,
            ),
            None => format!(
                "Estimated error — · target {:.0}%",
                self.editor
                    .document
                    .presentation
                    .adaptation
                    .accuracy_percent
            ),
        }
    }

    /// The accuracy target as the estimate states it, rather than as the
    /// control shows it.
    pub(super) fn amr_target_accuracy(&self) -> f64 {
        self.editor
            .document
            .presentation
            .adaptation
            .accuracy_percent
            / 100.0
    }

    /// Everything an estimate is built from. A change to any of it drops the
    /// work in flight rather than letting it finish against settings nobody
    /// asked for - which is why the settings folded away below are read here
    /// too, and why this is compared after the whole panel has been drawn.
    pub(super) fn amr_settings(&self) -> (bool, f64, f64, f64, f64) {
        (
            self.editor.document.presentation.adaptation.enabled,
            self.editor
                .document
                .presentation
                .adaptation
                .accuracy_percent,
            self.editor
                .document
                .presentation
                .adaptation
                .elements_per_wavelength,
            self.editor.document.presentation.adaptation.minimum_edge,
            self.editor.document.presentation.adaptation.maximum_edge,
        )
    }

    /// Numerical hygiene rather than physics, so it is folded away by default:
    /// the limits adaptation works between, and what the scheme does with detail
    /// no mesh can carry. Anything here changes what the solver does, not what
    /// it shows.
    pub(super) fn advanced_settings(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Advanced settings")
            .default_open(false)
            .show(ui, |ui| {
                ui.label("Adaptation");
                ui.add_enabled_ui(self.editor.document.presentation.adaptation.enabled, |ui| {
                    ui.add(
                        egui::DragValue::new(
                            &mut self
                                .editor
                                .document
                                .presentation
                                .adaptation
                                .elements_per_wavelength,
                        )
                        .speed(0.1)
                        .range(2.0..=16.0)
                        .prefix("Elements per wavelength "),
                    )
                    .on_hover_text(
                        "How finely a forced wave is carried, wherever a source or a \
                         boundary signal forces one. Quadratic elements put two nodes on \
                         every edge, so six elements is twelve nodes a wavelength. This is \
                         a floor the accuracy target does not lift.",
                    );
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(
                                &mut self.editor.document.presentation.adaptation.minimum_edge,
                            )
                            .speed(0.002)
                            .range(0.005..=1.0)
                            .prefix("Min "),
                        )
                        .on_hover_text("The smallest element adaptation may build");
                        ui.add(
                            egui::DragValue::new(
                                &mut self.editor.document.presentation.adaptation.maximum_edge,
                            )
                            .speed(0.005)
                            .range(0.005..=1.0)
                            .prefix("Max "),
                        )
                        .on_hover_text("The largest element adaptation may leave standing");
                    });
                    self.editor.document.presentation.adaptation.minimum_edge = self
                        .editor
                        .document
                        .presentation
                        .adaptation
                        .minimum_edge
                        .min(self.editor.document.presentation.adaptation.maximum_edge)
                        .max(0.005);
                    self.editor.document.presentation.adaptation.maximum_edge = self
                        .editor
                        .document
                        .presentation
                        .adaptation
                        .maximum_edge
                        .max(self.editor.document.presentation.adaptation.minimum_edge);
                });
                ui.separator();
                ui.label("Solver");
                ui.checkbox(
                    &mut self.editor.document.presentation.grid_scale_filter,
                    "Damp unresolvable detail",
                )
                .on_hover_text(
                    "The scheme does not dissipate at any wavelength, and the fastest \
                         modes a mesh can hold barely travel, so a sharp event - deleting a \
                         wall the field had a step across, or a source narrower than a few \
                         nodes - leaves a speckle that stays put for the rest of the run. \
                         This removes it, at a cost of well under a percent per half minute \
                         to a wave resolved as finely as the adaptation above aims for. \
                         It preserves constants and stationary force-free flux; it is not a \
                         terminal-silence or DC-removal control. \
                         Turn it off to see the untouched scheme.",
                );
            });
    }
}
