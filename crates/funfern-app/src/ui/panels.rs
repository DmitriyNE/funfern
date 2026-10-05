//! The View and Simulation inspectors, including the adaptation accuracy
//! controls and the advanced settings folded away beneath them.

use crate::material_overlay::{MaterialOverlay, MaterialProperty};
use bevy::prelude::*;
use bevy_egui::egui::{self};
use funfern_app::document::{VECTOR_LOWPASS_HZ_RANGE, VectorOverlay, VectorOverlayStyle};
use funfern_core::*;

use super::*;

/// The one rule of thumb the low-pass control needs.
const LOW_PASS_HELP: &str = "Average the energy-flow arrows below this corner. A harmonic \
     flow ripples at twice its source's frequency; a corner four times under that leaves a \
     few percent of the ripple and settles in 5.3 / (2π·f) seconds, 1.7 s at 0.5 Hz. The \
     canonical field, probes and energy remain unchanged.";

const STYLE_HELP: &str = "Arrows at every lattice point, or streamlines an even spacing \
     apart that follow the flow through a lens, along a fiber or round a resonator. The \
     dashes drift the way the energy goes.";

impl Playground {
    /// Whether the active generation carries a restoring law, and so an
    /// integrated field `r` to show (Gate O).
    pub(super) fn oscillator_active(&self) -> bool {
        self.runtime.active().is_some_and(|active| {
            active
                .canonical_temporal_operator
                .as_ref()
                .is_some_and(|operator| operator.has_restoring())
        })
    }

    /// Whether the field is painted as `r`: the document asks for it and the
    /// generation has one.
    pub(super) fn integrated_field_shown(&self) -> bool {
        self.editor.document.presentation.integrated_field && self.oscillator_active()
    }

    pub(super) fn view_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("View");
        let field_reference = self.field_exposure.reference();
        // Gate O: an oscillator generation carries `r = ∫u dt`, which is
        // where its kinks and domains show; the displayed field is its rate.
        let oscillator = self.oscillator_active();
        let overlay_error = self.material_overlay_error.clone();
        let overlay_progress = self.material_overlay_job.as_ref().map(|job| job.progress());
        let overlay_invalid = match self.editor.document.presentation.material_overlay {
            MaterialOverlay::Property(property) => self
                .material_overlay_snapshot
                .as_ref()
                .map(|snapshot| snapshot.invalid_count(property)),
            _ => None,
        };
        ui.checkbox(&mut self.editor.document.presentation.grid, "Grid");
        self.snap_checkbox(ui);
        let p = &mut self.editor.document.presentation;
        ui.checkbox(&mut p.control_polygons, "Control polygons");
        ui.checkbox(&mut p.handles, "Handles");
        ui.checkbox(&mut p.boundary_conditions, "Boundary conditions");
        ui.checkbox(&mut p.mesh, "Mesh");
        ui.checkbox(&mut p.mesh_boundaries, "Mesh boundaries");
        ui.checkbox(&mut p.field, "Field");
        if oscillator && p.field {
            let (u, r) = integrated_field_labels(self.editor.document.model.draft.physics);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut p.integrated_field, false, u);
                ui.selectable_value(&mut p.integrated_field, true, r)
                    .on_hover_text(
                        "The integrated field the restoring law acts on. A static kink or a \
                         domain wall shows here and not in the field itself, which is its \
                         rate. It refreshes with each full snapshot.",
                    );
            });
        }
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
        let overlay = sized_combo(ui, "vector-overlay", VectorOverlay::choices(physics).len())
            .selected_text(p.vector_overlay.label(physics))
            .show_ui(ui, |ui| {
                for mode in VectorOverlay::choices(physics) {
                    let entry =
                        ui.selectable_value(&mut p.vector_overlay, *mode, mode.label(physics));
                    self.spotlights
                        .record(Spotlight::OverlayChoice(*mode), entry.rect);
                }
            });
        self.spotlights
            .record(Spotlight::VectorOverlay, overlay.response.rect);
        if p.vector_overlay != VectorOverlay::Off {
            if p.vector_overlay == VectorOverlay::ComplementaryField {
                ui.checkbox(&mut p.vector_overlay_ac_coupled, "AC-couple arrows")
                    .on_hover_text(
                        "Subtract a slowly varying presentation baseline from the arrows. \
                         The canonical field, probes and energy remain unchanged.",
                    );
            }
            if p.vector_overlay == VectorOverlay::RelativeEnergyFlow {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut p.vector_overlay_lowpass, "Low-pass")
                        .on_hover_text(LOW_PASS_HELP);
                    ui.add_enabled(
                        p.vector_overlay_lowpass,
                        egui::DragValue::new(&mut p.vector_overlay_lowpass_hz)
                            .range(VECTOR_LOWPASS_HZ_RANGE)
                            .speed(0.01)
                            .fixed_decimals(2)
                            .suffix(" Hz")
                            .update_while_editing(false),
                    )
                    .on_hover_text(LOW_PASS_HELP);
                });
                ui.horizontal(|ui| {
                    ui.label("Style");
                    sized_combo(
                        ui,
                        "vector-overlay-style",
                        VectorOverlayStyle::CHOICES.len(),
                    )
                    .selected_text(p.vector_overlay_style.label())
                    .show_ui(ui, |ui| {
                        for style in VectorOverlayStyle::CHOICES {
                            ui.selectable_value(&mut p.vector_overlay_style, style, style.label());
                        }
                    })
                    .response
                    .on_hover_text(STYLE_HELP);
                });
            }
            let lines = p.vector_overlay_style.resolved(p.vector_overlay)
                == VectorOverlayStyle::Streamlines;
            ui.add(
                egui::Slider::new(&mut p.vector_overlay_density, 28.0..=120.0).text(if lines {
                    "Line spacing"
                } else {
                    "Arrow spacing"
                }),
            );
            ui.add(
                egui::Slider::new(&mut p.vector_overlay_gain, 0.1..=5.0).text(if lines {
                    "Line gain"
                } else {
                    "Arrow gain"
                }),
            );
        }
        ui.separator();
        ui.label("Overlay");
        const LAYERS: [MaterialOverlay; 3] = [
            MaterialOverlay::Regions,
            MaterialOverlay::Subdomains,
            MaterialOverlay::AdaptationTarget,
        ];
        const PROPERTIES: [MaterialProperty; 7] = [
            MaterialProperty::Density,
            MaterialProperty::Stiffness,
            MaterialProperty::Damping,
            MaterialProperty::WaveSpeed,
            MaterialProperty::Impedance,
            MaterialProperty::Anisotropy,
            MaterialProperty::VolumeSource,
        ];
        sized_combo(ui, "overlay", 1 + LAYERS.len() + PROPERTIES.len())
            .selected_text(
                p.material_overlay
                    .label_for(self.editor.document.model.draft.physics),
            )
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut p.material_overlay, MaterialOverlay::Off, "Off");
                for overlay in LAYERS {
                    ui.selectable_value(&mut p.material_overlay, overlay, overlay.label());
                }
                for property in PROPERTIES {
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
    /// What the speed row says, and whether it says the solver is short of
    /// the rate asked for.
    ///
    /// The row is always there, so the panel below it stays put: a note that
    /// came and went as the rate wandered about the margin moved every
    /// control under it, eight times in 37 s on the parametric fiber. It is
    /// not silenced while a handoff is pending either. Adaptation keeps one
    /// pending much of the time on exactly the scenes heavy enough to fall
    /// short; the held rate already rides over the few frames a handoff
    /// withholds.
    pub(super) fn simulation_speed_line(&self) -> (String, bool) {
        let asked = self.editor.document.presentation.simulation_speed;
        let reached = self.speed_reached;
        if !self.wave_running {
            ("Paused".into(), false)
        } else if !reached.is_finite() || reached <= 0.0 {
            ("Running at —".into(), false)
        } else if self.speed_short {
            (format!("Reaching {reached:.2}×"), true)
        } else {
            (format!("Running at {:.2}×", reached.min(asked)), false)
        }
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
        let (speed, short) = self.simulation_speed_line();
        if short {
            ui.colored_label(GOLD, speed).on_hover_text(
                "The scene costs more per step than the frame budget allows, so simulated \
                 time runs slower than asked. A coarser mesh buys it back.",
            );
        } else {
            ui.weak(speed);
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
        let domain = self.editor.document.model.draft.geometry.domain;
        let (triangles, dofs) = mesh_estimate(
            domain.width() * domain.height(),
            self.editor.document.presentation.mesh_edge,
        );
        ui.weak(format!(
            "≈ {} triangles · ≈ {} DOFs",
            compact_count(triangles),
            compact_count(dofs)
        ))
        .on_hover_text(
            "Estimated from the domain's area at the target edge; curves, grading and holes \
             move it either way",
        );
        if dofs > LARGE_MESH_DOFS {
            ui.colored_label(
                GOLD,
                "Large mesh: preparing it takes a while and the solver steps slower",
            );
        }
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
        // Shown only while adaptation is on. What it shifts when it comes or
        // goes sits below the checkbox that did it, never under the pointer.
        if self.editor.document.presentation.adaptation.enabled {
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
            // One row each whatever they say, cut short with the whole on
            // hover: they change several times a second, and a line that
            // wrapped for some of it moved the controls below.
            ui.add(egui::Label::new(egui::RichText::new(&self.amr_status).small()).truncate());
            ui.add(
                egui::Label::new(egui::RichText::new(self.amr_estimate_line()).small()).truncate(),
            );
            // The element sizes the estimate asks for, one row that a notice
            // takes over in gold, so a notice that comes or goes - a slider
            // crossing the forcing's floor, an adaptation that fails - moves
            // nothing below it and leaves no gap when there is none.
            let (line, hover, notice) = self.amr_size_line();
            let text = egui::RichText::new(line).small();
            let text = if notice {
                text.color(GOLD)
            } else {
                text.color(ui.visuals().weak_text_color())
            };
            ui.add(egui::Label::new(text).truncate())
                .on_hover_text(hover);
        }
        ui.separator();
        ui.label("Point source");
        let mut source = self.editor.document.model.source;
        let before = source;
        ui.horizontal(|ui| {
            ui.checkbox(&mut source.enabled, "Enabled");
            ui.add(
                egui::DragValue::new(&mut source.width)
                    .speed(0.002)
                    .range(0.001..=1.0)
                    .prefix("Width "),
            );
        });
        let fire_at = self.fire_times();
        edit_time_signal(
            ui,
            &mut source.signal,
            SignalUse::Source,
            fire_at,
            &mut self.pulse_preview,
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

    /// The adaptation block's size row: what it says, its hover, and whether
    /// it is a notice. An adaptation error takes it first, since it is rarer
    /// and matters more; then a forcing that wants elements under the
    /// smallest allowed, which nothing else in the panel explains while the
    /// accuracy target reads satisfied; otherwise the element sizes the
    /// estimate asks for against the sizes allowed. It reads the shown
    /// report, so a handoff, which drops the estimate, changes nothing here
    /// until the next one.
    pub(super) fn amr_size_line(&self) -> (String, String, bool) {
        if let Some(error) = &self.amr_error {
            return (error.clone(), error.clone(), true);
        }
        let settings = self.editor.document.presentation.adaptation;
        let (floor, ceiling) = (settings.minimum_edge, settings.maximum_edge);
        let report = self.amr_shown_report.as_ref();
        if let Some(report) = report
            && report.smallest_wavelength_target < floor
        {
            let wanted = report.smallest_wavelength_target;
            return (
                format!("Forcing wants {wanted:.3} < smallest {floor:.3}: mesh at its floor"),
                format!(
                    "The forcing wants elements of {wanted:.3}, under the smallest allowed of \
                     {floor:.3}, so the mesh sits at its floor whatever the accuracy asks"
                ),
                true,
            );
        }
        let wanted = report
            .filter(|report| report.minimum_target.is_finite() && report.maximum_target > 0.0)
            .map_or("—".to_owned(), |report| {
                format!("{:.3}–{:.3}", report.minimum_target, report.maximum_target)
            });
        (
            format!("Elements wanted {wanted} · allowed {floor:.3}–{ceiling:.3}"),
            "The element sizes the last estimate asks for, between the smallest and largest \
             allowed"
                .into(),
            false,
        )
    }

    /// What the panel says about the estimate, whether or not one is in hand.
    /// Committing a mesh drops the estimate it was measured against, so a line
    /// that exists only while there is one comes and goes with every adaptation
    /// and shifts the panel out from under the pointer. It holds its place, and
    /// reads the last estimate's report until the next replaces it.
    pub(super) fn amr_estimate_line(&self) -> String {
        match &self.amr_shown_report {
            Some(report) if report.dormant => format!(
                "Estimated error dormant · target {:.0}%",
                self.editor
                    .document
                    .presentation
                    .adaptation
                    .accuracy_percent
            ),
            Some(report) => format!(
                "Estimated error {:.1}% · target {:.0}%{}",
                100.0 * report.global_indicator,
                self.editor
                    .document
                    .presentation
                    .adaptation
                    .accuracy_percent,
                if size_rule_refines(report, self.amr_target_accuracy()) {
                    " · refining to resolve wavelengths"
                } else {
                    ""
                },
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
                         This removes it within a second or two, at a cost of two or three \
                         percent of its energy per half minute to a wave resolved as finely \
                         as the adaptation above aims for. \
                         It preserves constants and stationary force-free flux; it is not a \
                         terminal-silence or DC-removal control. \
                         Turn it off to see the untouched scheme.",
                );
                if self.grid_filter_refused {
                    ui.label(
                        egui::RichText::new(
                            "Off for this scene: the running generation does not admit the \
                             filter for its combination of boundaries and media.",
                        )
                        .small()
                        .weak(),
                    );
                }
            });
    }
}

/// Above this many solver unknowns the Simulation panel warns that the mesh
/// is large: an edge of about 0.03 on the 2 × 2 domain, where preparing takes
/// several seconds natively and the solver steps about twice as slowly as at
/// the Fine preset's 34 thousand.
const LARGE_MESH_DOFS: f64 = 60_000.0;

/// Triangles and solver unknowns a domain of `area` holds at `edge`. The
/// enriched quadratic element has seven nodes; shared, that is half a vertex,
/// one and a half edge midpoints and a bubble for each triangle, so three
/// unknowns a triangle, as a scene at 0.02 measured (123,422 on 40,945).
fn mesh_estimate(area: f64, edge: f64) -> (f64, f64) {
    let triangles = MeshingOptions::expected_triangles(area, edge);
    (triangles, 3.0 * triangles)
}

/// A count for a glance: 800, 9.2k, 41k, 1.2M.
fn compact_count(value: f64) -> String {
    match value {
        v if v >= 1.0e6 => format!("{:.1}M", v / 1.0e6),
        v if v >= 1.0e4 => format!("{:.0}k", v / 1.0e3),
        v if v >= 1.0e3 => format!("{:.1}k", v / 1.0e3),
        v => format!("{v:.0}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The style is the energy flow's: its choice and the lines' labels show
    /// for the energy flow, and the complementary field keeps the arrows and
    /// their labels whatever the style says.
    #[test]
    fn the_style_choice_belongs_to_the_energy_flow() {
        let mut state = Playground {
            inspector: Some(InspectorPanel::View),
            ..Playground::default()
        };
        state.editor.document.model.draft.physics = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        };
        let context = egui::Context::default();
        theme::apply(&context);
        context.enable_accesskit();
        let mut time = 0.0;
        let mut labels = |state: &mut Playground| {
            time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 900.0),
                )),
                time: Some(time),
                ..egui::RawInput::default()
            };
            let output = context.run_ui(input, |ui| state.side_panel(ui));
            super::super::test_support::laid_out(&output)
                .into_iter()
                .map(|widget| widget.label)
                .collect::<Vec<_>>()
        };
        let has = |labels: &[String], label: &str| labels.iter().any(|shown| shown == label);

        state.editor.document.presentation.vector_overlay = VectorOverlay::RelativeEnergyFlow;
        let shown = labels(&mut state);
        assert!(has(&shown, "Arrows"), "{shown:?}");
        assert!(has(&shown, "Arrow spacing") && has(&shown, "Arrow gain"));

        state.editor.document.presentation.vector_overlay_style = VectorOverlayStyle::Streamlines;
        let shown = labels(&mut state);
        assert!(has(&shown, "Streamlines"), "{shown:?}");
        assert!(has(&shown, "Line spacing") && has(&shown, "Line gain"));

        state.editor.document.presentation.vector_overlay = VectorOverlay::ComplementaryField;
        let shown = labels(&mut state);
        assert!(
            !has(&shown, "Streamlines") && !has(&shown, "Arrows"),
            "{shown:?}"
        );
        assert!(has(&shown, "Arrow spacing") && has(&shown, "Arrow gain"));
    }

    /// The vector overlay's list shows all of its entries after the physics
    /// changes under it. Opened once on Mechanical's two, it showed two and a
    /// half of the EM skins' three, the third cut after 6 of its 23 px, every
    /// time after: egui kept the popup's size from its last showing.
    #[test]
    fn the_vector_overlay_list_shows_every_entry_after_the_physics_changes() {
        let mut state = Playground {
            inspector: Some(InspectorPanel::View),
            ..Playground::default()
        };
        let context = egui::Context::default();
        theme::apply(&context);
        context.enable_accesskit();
        let mut time = 0.0;
        let mut frame = |state: &mut Playground, events: Vec<egui::Event>| {
            time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 900.0),
                )),
                time: Some(time),
                events,
                ..egui::RawInput::default()
            };
            context.run_ui(input, |ui| state.side_panel(ui))
        };
        frame(&mut state, vec![]);
        for physics in [
            PhysicsModel::Mechanical,
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            },
        ] {
            state.editor.document.model.draft.physics = physics;
            let shown = VectorOverlay::Off.label(physics);
            let output = frame(&mut state, vec![]);
            let combo = super::super::test_support::laid_out(&output)
                .into_iter()
                .find(|widget| widget.label == shown)
                .expect("the vector overlay combo");
            frame(&mut state, super::super::test_support::click(&combo));
            let output = frame(&mut state, vec![]);
            let popup = context.memory(|memory| {
                memory
                    .areas()
                    .visible_layer_ids()
                    .into_iter()
                    .filter(|layer| layer.order == egui::Order::Foreground)
                    .find_map(|layer| memory.area_rect(layer.id))
                    .expect("the open list")
            });
            let choices = VectorOverlay::choices(physics);
            let entries: Vec<_> = super::super::test_support::laid_out(&output)
                .into_iter()
                .filter(|widget| widget.rect != combo.rect && popup.intersects(widget.rect))
                .filter(|widget| {
                    choices
                        .iter()
                        .any(|choice| choice.label(physics) == widget.label)
                })
                .collect();
            assert_eq!(entries.len(), choices.len(), "{physics:?}");
            for entry in &entries {
                assert!(
                    popup.contains_rect(entry.rect),
                    "{physics:?}: {} at {:?} cut by the list at {popup:?}",
                    entry.label,
                    entry.rect
                );
            }
            frame(
                &mut state,
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            frame(&mut state, vec![]);
        }
    }

    /// The estimate under the slider against meshes actually built: the
    /// empty 2 × 2 domain at 0.08, 0.04 and 0.03 (2,772, 11,190 and 20,031
    /// triangles), and a 1.854 × 2 domain with a curved hole at 0.02 (40,945
    /// triangles, 123,422 unknowns). The warning starts between the Fine
    /// preset and the finest of these.
    #[test]
    fn the_mesh_estimate_follows_meshes_actually_built() {
        for (area, edge, built) in [
            (4.0, 0.08, 2_772.0),
            (4.0, 0.04, 11_190.0),
            (4.0, 0.03, 20_031.0),
            (1.8538 * 2.0, 0.02, 40_945.0),
        ] {
            let (triangles, _) = mesh_estimate(area, edge);
            assert!(
                (triangles / built - 1.0).abs() < 0.1,
                "{triangles:.0} estimated against {built} built at {edge}"
            );
        }
        let (_, dofs) = mesh_estimate(1.8538 * 2.0, 0.02);
        assert!((dofs / 123_422.0 - 1.0).abs() < 0.1, "{dofs:.0}");
        assert!(mesh_estimate(4.0, 0.04).1 < LARGE_MESH_DOFS);
        assert!(mesh_estimate(4.0, 0.025).1 > LARGE_MESH_DOFS);
        assert_eq!(compact_count(812.0), "812");
        assert_eq!(compact_count(9_216.0), "9.2k");
        assert_eq!(compact_count(40_945.0), "41k");
        assert_eq!(compact_count(1_234_567.0), "1.2M");
    }

    /// Where `label` sits in the simulation panel, in the inspector as a
    /// screen `width` wide shows it - docked on a desktop, floating on a
    /// phone - for `state`.
    fn simulation_panel_row(
        state: &mut Playground,
        context: &egui::Context,
        width: f32,
        label: &str,
    ) -> Option<egui::Rect> {
        state.inspector = Some(InspectorPanel::Simulation);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 4000.0),
            )),
            ..egui::RawInput::default()
        };
        let output = context.run_ui(input, |ui| state.side_panel(ui));
        super::super::test_support::laid_out(&output)
            .into_iter()
            .find(|widget| widget.label.starts_with(label))
            .map(|widget| widget.rect)
    }

    /// Nothing in the simulation panel that changes while the solver runs
    /// moves the controls under it, docked on a desktop or floating on a
    /// phone. The speed row reads paused, keeping up or short in one row; the
    /// adaptation status and estimate hold one row each however long they
    /// get; and the note on a forcing under the floor reads the last
    /// estimate's report, which a handoff does not drop. On the parametric
    /// fiber the speed note came and went eight times in 37 s, the forcing
    /// note five times in 6 s. No status says as much as the one below
    /// today, and both inspectors fit the longest that do; the row holds
    /// whatever one does say. The forcing note and an adaptation error take
    /// over the row of element sizes the estimate asks for, which is there
    /// whatever the state. With adaptation off the block under its checkbox
    /// is hidden: what that shifts sits below the checkbox that did it.
    #[test]
    fn what_changes_while_running_keeps_the_panel_still() {
        for width in [1400.0, 390.0] {
            let mut state = Playground::default();
            state.editor.document.presentation.adaptation.enabled = true;
            state.editor.document.presentation.simulation_speed = 0.09;
            let context = egui::Context::default();
            theme::apply(&context);
            context.enable_accesskit();
            let row = |state: &mut Playground, label| {
                simulation_panel_row(state, &context, width, label)
                    .unwrap_or_else(|| panic!("no {label:?} at width {width}"))
                    .min
                    .y
            };
            row(&mut state, "Mesh resolution");
            let paused = row(&mut state, "Mesh resolution");
            state.wave_running = true;
            let starting = row(&mut state, "Mesh resolution");
            state.speed_reached = 0.086;
            let keeping_up = row(&mut state, "Mesh resolution");
            state.speed_short = true;
            state.speed_reached = 0.071;
            let short = row(&mut state, "Mesh resolution");
            assert_eq!(
                state.simulation_speed_line(),
                ("Reaching 0.07×".into(), true)
            );
            assert_eq!(
                [starting, keeping_up, short],
                [paused; 3],
                "at width {width}"
            );

            let quiet = row(&mut state, "Point source");
            row(&mut state, "Elements wanted —");
            state.amr_status = "adaptation discarded: the mesh changed underneath it, \
                                and the estimate it was measured against with it"
                .into();
            let report = SolutionIndicatorReport {
                limit_refine_candidates: 2000,
                refine_candidates: 2000,
                global_indicator: 0.0262,
                minimum_target: 0.031,
                maximum_target: 0.144,
                ..Default::default()
            };
            state.amr_shown_report = Some(report.clone());
            assert_eq!(
                state.amr_size_line().0,
                "Elements wanted 0.031–0.144 · allowed 0.020–0.160"
            );
            assert!(
                state
                    .amr_estimate_line()
                    .ends_with("refining to resolve wavelengths"),
                "{}",
                state.amr_estimate_line()
            );
            assert_eq!(row(&mut state, "Point source"), quiet, "at width {width}");

            // A forcing under the floor is told from the shown report, with no
            // estimate in hand, as just after a handoff; it and an error take
            // over the size row, the error first.
            let floored = SolutionIndicatorReport {
                smallest_wavelength_target: 0.01,
                ..report.clone()
            };
            state.amr_shown_report = Some(floored.clone());
            assert!(state.amr_indicator_result.is_none());
            row(&mut state, "Forcing wants");
            assert_eq!(row(&mut state, "Point source"), quiet, "at width {width}");
            state.amr_error = Some(
                "adaptation failed: the adapted mesh does not match the active topology, \
                 and the estimate it was measured against went with it"
                    .into(),
            );
            row(&mut state, "adaptation failed");
            assert_eq!(row(&mut state, "Point source"), quiet, "at width {width}");
            state.amr_shown_report = Some(report);
            assert_eq!(row(&mut state, "Point source"), quiet, "at width {width}");
            state.amr_error = None;
            state.amr_shown_report = Some(floored);
            // Turning adaptation off forgets it, and the block under the
            // checkbox goes with it.
            state.stop_adaptation_work();
            assert!(state.amr_shown_report.is_none());
            state.editor.document.presentation.adaptation.enabled = false;
            assert!(simulation_panel_row(&mut state, &context, width, "Elements wanted").is_none());
            assert!(simulation_panel_row(&mut state, &context, width, "Adapt mesh").is_some());
        }
    }
}
