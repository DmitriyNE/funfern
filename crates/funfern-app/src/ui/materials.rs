//! The material library and subdomain assignment: the Materials inspector,
//! its face and region listings, and the editor for one region's properties.

use bevy::prelude::*;
use bevy_egui::egui::{self, Color32};
use funfern_core::*;
use std::collections::BTreeSet;

use super::*;

impl Playground {
    pub(super) fn materials_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Materials");
        ui.horizontal(|ui| {
            ui.label("Subdomain assignment");
            for (mode, label, hint) in [
                (
                    SubdomainListing::Faces,
                    "Faces",
                    "Every compiled subdomain, holes included; a click in the scene picks one",
                ),
                (
                    SubdomainListing::Regions,
                    "Regions",
                    "Only the material regions; a click in the scene picks one",
                ),
            ] {
                if ui
                    .selectable_label(self.subdomain_listing == mode, label)
                    .on_hover_text(hint)
                    .clicked()
                {
                    self.subdomain_listing = mode;
                }
            }
        });
        let materials = self.editor.document.model.draft.materials.clone();
        if self.subdomain_listing == SubdomainListing::Faces {
            self.face_listing(ui, &materials);
        } else {
            self.region_listing(ui, &materials);
        }
        self.region_detail(ui);
    }

    /// One row per assigned face, so a hole is editable in the same place a
    /// subdomain is: it is simply the row whose material is Hole.
    fn face_listing(&mut self, ui: &mut egui::Ui, materials: &[Material]) {
        let assignments = self.editor.document.model.draft.face_assignments.clone();
        if self.face_selection >= assignments.len() {
            self.face_selection = 0;
        }
        for (index, assignment) in assignments.iter().enumerate() {
            ui.horizontal(|ui| {
                let (swatch, _) =
                    ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                ui.painter().rect_filled(
                    swatch,
                    2.0,
                    match assignment.region {
                        Some(region) => {
                            subdomain_color(&self.editor.document.model.draft, region, 1.0)
                        }
                        None => Color32::from_gray(70),
                    },
                );
                let name = match assignment.region {
                    Some(region) if region == BACKGROUND_REGION => "Background".to_owned(),
                    Some(region) => self.material_name(region),
                    None => "Hole".to_owned(),
                };
                if ui
                    .selectable_label(self.face_selection == index, name)
                    .on_hover_text("Select this subdomain and outline it in the scene")
                    .clicked()
                {
                    self.face_selection = index;
                    if let Some(region) = assignment.region {
                        self.region_selection = region;
                    }
                }
                let mut chosen = assignment.region.and_then(|region| {
                    self.editor
                        .document
                        .model
                        .draft
                        .region(region)
                        .map(|region| region.material)
                });
                egui::ComboBox::from_id_salt(("face", index))
                    .selected_text(match chosen {
                        Some(material) => materials
                            .iter()
                            .find(|item| item.id == material)
                            .map_or("Missing", |item| item.name.as_str()),
                        None => "Hole",
                    })
                    .show_ui(ui, |ui| {
                        for item in materials.iter() {
                            ui.selectable_value(&mut chosen, Some(item.id), &item.name);
                        }
                        ui.selectable_value(&mut chosen, None, "Hole")
                            .on_hover_text("Remove this subdomain and wall its boundary");
                    });
                let current = assignment.region.and_then(|region| {
                    self.editor
                        .document
                        .model
                        .draft
                        .region(region)
                        .map(|region| region.material)
                });
                if chosen != current
                    && let Err(error) = self.editor.set_face_disposition(index, chosen)
                {
                    self.notify(error);
                }
            });
        }
    }

    fn region_listing(&mut self, ui: &mut egui::Ui, materials: &[Material]) {
        let regions = self.editor.document.model.draft.regions.clone();
        for region in regions {
            ui.horizontal(|ui| {
                let (swatch, _) =
                    ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                ui.painter().rect_filled(
                    swatch,
                    2.0,
                    subdomain_color(&self.editor.document.model.draft, region.id, 1.0),
                );

                if ui
                    .selectable_label(
                        self.region_selection == region.id,
                        if region.id == BACKGROUND_REGION {
                            "Background".into()
                        } else {
                            format!("Region {}", region.id.0)
                        },
                    )
                    .clicked()
                {
                    self.region_selection = region.id;
                }
                let mut material = region.material;
                egui::ComboBox::from_id_salt(("region", region.id.0))
                    .selected_text(
                        materials
                            .iter()
                            .find(|item| item.id == material)
                            .map_or("Missing", |item| item.name.as_str()),
                    )
                    .show_ui(ui, |ui| {
                        for item in materials {
                            ui.selectable_value(&mut material, item.id, &item.name);
                        }
                    });
                if material != region.material {
                    if let Err(error) = self.editor.set_region_material(region.id, material) {
                        self.notify(error)
                    }
                }
            });
        }
    }

    /// The selected region's source and frame, plus the material library.
    fn region_detail(&mut self, ui: &mut egui::Ui) {
        let materials = self.editor.document.model.draft.materials.clone();
        if let Some(region) = self
            .editor
            .document
            .model
            .draft
            .region(self.region_selection)
            .copied()
        {
            ui.separator();
            let existing = self
                .editor
                .document
                .model
                .draft
                .volume_sources
                .iter()
                .find(|source| source.region == region.id)
                .cloned();
            let mut source = existing.clone().unwrap_or(VolumeSource {
                region: region.id,
                enabled: false,
                profile: ScalarField::constant(1.0),
                parameters: vec![],
                signal: TimeSignal::harmonic(0.0, 12.0, 3.0, 0.0),
            });
            let mut source_changed = false;
            ui.horizontal(|ui| {
                source_changed = ui.checkbox(&mut source.enabled, "Volume source").changed();
                // Only beside a visible profile editor: with the source off
                // there is no formula on screen to explain.
                if source.enabled {
                    self.formula_help_toggle(ui);
                }
            });
            // The editors follow the checkbox exactly. Turning the source off
            // keeps its profile and signal in the document, so turning it back
            // on restores what was there.
            if source.enabled {
                let before = source.profile.clone();
                material_scalar_editor(
                    ui,
                    (u64::MAX - region.id.0, 4),
                    "Profile",
                    &mut source.profile,
                    &source.parameters,
                    0.0,
                    &mut self.material_formula_edits,
                    &mut self.material_formula_errors,
                );
                source_changed |= source.profile != before;
                ui.label("Signal");
                let before = source.signal;
                edit_time_signal(ui, &mut source.signal);
                source_changed |= source.signal != before;
                ui.small(
                    "Version-22 acceleration control: this signal is analytically integrated \
                     into a canonical primary-field-rate drive using immutable \
                     accepted-generation normalization.",
                );
            }
            // Committed outside the block so unchecking is recorded rather than
            // springing back on the next frame.
            if source_changed
                && let Err(error) = self.editor.set_volume_source(region.id, Some(source))
            {
                self.notify(error);
            }

            let uses_frame = self
                .editor
                .document
                .model
                .draft
                .material(region.material)
                .is_some_and(Material::uses_frame)
                || existing
                    .as_ref()
                    .is_some_and(|source| source.enabled && source.varying());
            if uses_frame {
                ui.separator();
                ui.label("Profile placement");
                let mut frame = region.frame;
                let before = frame;
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut frame.origin.x)
                            .speed(0.01)
                            .prefix("x ")
                            .update_while_editing(false),
                    );
                    ui.add(
                        egui::DragValue::new(&mut frame.origin.y)
                            .speed(0.01)
                            .prefix("y ")
                            .update_while_editing(false),
                    );
                });
                let mut angle = frame.angle_radians.to_degrees();
                ui.add(
                    egui::DragValue::new(&mut angle)
                        .speed(0.5)
                        .suffix("°")
                        .prefix("Angle ")
                        .update_while_editing(false),
                );
                frame.angle_radians = angle.to_radians();
                if region.id != BACKGROUND_REGION {
                    egui::ComboBox::from_label("Attachment")
                        .selected_text(match frame.attachment {
                            MaterialFrameAttachment::World => "World",
                            MaterialFrameAttachment::FollowRegion => "Follow region",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut frame.attachment,
                                MaterialFrameAttachment::World,
                                "World",
                            );
                            ui.selectable_value(
                                &mut frame.attachment,
                                MaterialFrameAttachment::FollowRegion,
                                "Follow region",
                            );
                        });
                }
                if frame != before
                    && let Err(error) = self.editor.set_region_frame(region.id, frame)
                {
                    self.notify(error);
                }
            }
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Library");
            self.formula_help_toggle(ui);
            if ui.button("+").clicked() {
                match self.editor.add_material() {
                    Ok(id) => {
                        self.material_selection = id;
                        self.material_edit = None;
                        self.material_formula_edits.clear();
                        self.material_formula_errors.clear();
                    }
                    Err(error) => self.notify(error),
                }
            }
        });
        for material in &materials {
            ui.horizontal(|ui| {
                // `color_edit_button_srgb` reports a change on every frame of a
                // drag inside its popup, so stage the value and commit it once
                // the pointer is released.
                let mut color = self
                    .material_color_edit
                    .filter(|(id, _)| *id == material.id)
                    .map_or(material.color, |(_, color)| color);
                let response = ui
                    .color_edit_button_srgb(&mut color)
                    .on_hover_text("Colour used by the Materials overlay");
                if response.changed() {
                    self.material_color_edit = Some((material.id, color));
                }
                if let Some((id, staged)) = self.material_color_edit
                    && id == material.id
                    && !ui.ctx().egui_is_using_pointer()
                {
                    self.material_color_edit = None;
                    if staged != material.color {
                        let mut updated = material.clone();
                        updated.color = staged;
                        if let Err(error) = self.editor.update_material(updated) {
                            self.notify(error);
                        }
                    }
                }
                if ui
                    .selectable_label(self.material_selection == material.id, &material.name)
                    .clicked()
                {
                    self.material_selection = material.id;
                    self.material_edit = None;
                    self.material_formula_edits.clear();
                    self.material_formula_errors.clear();
                }
                if material.id == DEFAULT_MATERIAL {
                    ui.small("ambient");
                }
            });
        }
        if self
            .material_edit
            .as_ref()
            .is_none_or(|material| material.id != self.material_selection)
        {
            self.material_edit = self
                .editor
                .document
                .model
                .draft
                .materials
                .iter()
                .find(|item| item.id == self.material_selection)
                .cloned();
        }
        if let Some(mut material) = self.material_edit.take() {
            ui.separator();
            ui.text_edit_singleline(&mut material.name);
            // What kind of medium this is, which decides what follows. A preset
            // writes the law slots and creates the parameters it exposes; after
            // that the material stands on its own, so editing a slot by hand
            // leaves it Custom rather than being refitted to the preset it came
            // from.
            let physics = self.editor.document.model.draft.physics;
            let matched = identify_law_preset(&material);
            let mut chosen = None;
            ui.horizontal(|ui| {
                ui.label("Response");
                egui::ComboBox::from_id_salt(("material-response", material.id.0))
                    .selected_text(matched.as_ref().map_or_else(
                        || "Custom".to_owned(),
                        |found| law_preset_label(found.preset, physics),
                    ))
                    .show_ui(ui, |ui| {
                        for preset in law_presets() {
                            let current =
                                matched.as_ref().is_some_and(|found| found.preset == preset);
                            if ui
                                .selectable_label(current, law_preset_label(preset, physics))
                                .on_hover_text(preset.phenomenon)
                                .clicked()
                            {
                                chosen = Some(preset);
                            }
                        }
                    });
            });
            if let Some(preset) = chosen {
                match apply_law_preset(preset, &material) {
                    Ok(applied) => material = applied,
                    Err(error) => self.notify(error.to_string()),
                }
            }
            ui.separator();
            let labels = material_editor_labels(self.editor.document.model.draft.physics);
            material_scalar_editor(
                ui,
                (material.id.0, 0),
                labels.mass,
                &mut material.mass_density,
                &material.parameters,
                0.000001,
                &mut self.material_formula_edits,
                &mut self.material_formula_errors,
            );
            material_scalar_editor(
                ui,
                (material.id.0, 1),
                labels.stiffness,
                &mut material.stiffness,
                &material.parameters,
                0.000001,
                &mut self.material_formula_edits,
                &mut self.material_formula_errors,
            );
            material_scalar_editor(
                ui,
                (material.id.0, 2),
                labels.damping,
                &mut material.damping,
                &material.parameters,
                0.0,
                &mut self.material_formula_edits,
                &mut self.material_formula_errors,
            );
            material_scalar_editor(
                ui,
                (material.id.0, 3),
                labels.axis_ratio,
                &mut material.axis_ratio,
                &material.parameters,
                1.0,
                &mut self.material_formula_edits,
                &mut self.material_formula_errors,
            );
            // The preset's own values, below the base ones and separated from
            // them, because the two behave differently: a base coefficient
            // survives a change of preset, and these are replaced by it.
            // Recomputed, because applying a preset above changed the material.
            let matched = identify_law_preset(&material);
            let preset_owned = matched
                .as_ref()
                .map(|found| found.parameters.iter().cloned().collect::<BTreeSet<_>>())
                .unwrap_or_default();
            if matched
                .as_ref()
                .is_some_and(|found| !found.preset.variables.is_empty())
            {
                ui.separator();
            }
            if let Some(found) = &matched {
                for (variable, name) in found.preset.variables.iter().zip(&found.parameters) {
                    let Some(parameter) = material
                        .parameters
                        .iter_mut()
                        .find(|parameter| parameter.name == *name)
                    else {
                        continue;
                    };
                    ui.horizontal(|ui| {
                        ui.label(variable.label);
                        ui.add(
                            egui::DragValue::new(&mut parameter.value)
                                .speed(0.005)
                                .range(variable.minimum..=variable.maximum)
                                .update_while_editing(false),
                        );
                    });
                }
            }
            // A Switch's ramp is one number on the material rather than a slot
            // on a row, so it is edited here rather than exposed as a preset
            // variable. Zero is a hard temporal interface.
            if material.mass_law.alternate.is_some() || material.stiffness_law.alternate.is_some() {
                ui.horizontal(|ui| {
                    ui.label("Switch ramp");
                    ui.add(
                        egui::DragValue::new(&mut material.switch_ramp)
                            .speed(0.01)
                            .range(0.0..=60.0)
                            .suffix(" s")
                            .update_while_editing(false),
                    );
                });
            }
            // What the laws compose to, in the names the preset gave them.
            for line in material_law_summary(&material, physics, LawSummaryDetail::Named)
                .unwrap_or_default()
            {
                ui.small(format!("{} = {}", line.subject, line.response));
            }

            // Only the parameters the user made. A preset's own are above,
            // under the labels it gave them, and showing them again here was
            // two controls for one number.
            ui.collapsing("Parameters", |ui| {
                let mut remove = None;
                let referenced_names = material
                    .parameter_names()
                    .map(str::to_owned)
                    .collect::<BTreeSet<_>>();
                for (index, parameter) in material.parameters.iter_mut().enumerate() {
                    if preset_owned.contains(&parameter.name) {
                        continue;
                    }
                    let referenced = referenced_names.contains(&parameter.name);
                    ui.horizontal(|ui| {
                        ui.label(&parameter.name);
                        ui.add(
                            egui::DragValue::new(&mut parameter.value)
                                .speed(0.01)
                                .update_while_editing(false),
                        );
                        if ui
                            .add_enabled(!referenced, egui::Button::new("−"))
                            .on_hover_text(if referenced {
                                "Parameter is used by a formula"
                            } else {
                                "Delete parameter"
                            })
                            .clicked()
                        {
                            remove = Some(index);
                        }
                    });
                }
                if let Some(index) = remove {
                    debug_assert!(material.remove_parameter(index).is_ok());
                }
                if material.parameters.len() < MAX_MATERIAL_PARAMETERS
                    && ui.button("+ Parameter").clicked()
                {
                    let name = (1..)
                        .map(|index| format!("p{index}"))
                        .find(|name| {
                            material
                                .parameters
                                .iter()
                                .all(|parameter| parameter.name != *name)
                        })
                        .unwrap();
                    material
                        .parameters
                        .push(MaterialParameter { name, value: 1.0 });
                }
            });
            let stored = self
                .editor
                .document
                .model
                .draft
                .material(material.id)
                .cloned();
            let dirty = stored.as_ref() != Some(&material);
            if ui.add_enabled(dirty, egui::Button::new("Apply")).clicked() {
                if let Err(error) = self.editor.update_material(material.clone()) {
                    self.notify(error)
                }
            }
            if self.material_selection != DEFAULT_MATERIAL
                && !self
                    .editor
                    .document
                    .model
                    .draft
                    .regions
                    .iter()
                    .any(|region| region.material == self.material_selection)
                && ui.button("Delete material").clicked()
            {
                match self.editor.delete_material(self.material_selection) {
                    Ok(()) => self.material_selection = DEFAULT_MATERIAL,
                    Err(error) => self.notify(error),
                }
            }
            self.material_edit = Some(material);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The claim the Response selector makes: choosing a preset and applying
    /// it produces a document the solver compiles as a driven one. Everything
    /// the panel does between those two points is tested in the core, but the
    /// wiring from an authored material to a temporal operator is only true
    /// end to end, and until now every driven scene in this project was built
    /// in test code rather than authored.
    #[test]
    fn applying_a_pump_preset_prepares_a_driven_generation() {
        let mut state = Playground::default();
        let before = activate(&mut state);
        assert!(!before.driven(), "the default document is not driven");

        let material = state.editor.document.model.draft.materials[0].clone();
        let pump = law_presets()
            .iter()
            .find(|preset| preset.name == "Parametric pump" && preset.row == LawPresetRow::Mass)
            .expect("the catalogue offers a pump");
        let driven = apply_law_preset(pump, &material).unwrap();
        assert_eq!(
            identify_law_preset(&driven).unwrap().preset.name,
            "Parametric pump",
            "the selector must read back what it just applied"
        );
        state.editor.update_material(driven).unwrap();
        settle(&mut state.editor);

        let after = activate(&mut state);
        assert!(
            after.driven(),
            "an authored pump must reach the solver as a temporal generation"
        );
        assert!(
            after.recommended_time_step() < before.recommended_time_step(),
            "and its coefficient trajectory must tighten the step bound"
        );
    }

    #[test]
    fn the_law_rows_are_named_for_what_their_laws_multiply() {
        // Five of the six (skin, row) pairs multiply the coefficient stored
        // beside them. The mechanical complementary row does not, which
        // `a_pump_on_the_stiffness_row_lowers_the_mechanical_stiffness` in the
        // core measures, so calling it stiffness would read backwards.
        assert_eq!(
            law_row_label(PhysicsModel::Mechanical, LawPresetRow::Mass),
            "Density ρ₀"
        );
        assert_eq!(
            law_row_label(PhysicsModel::Mechanical, LawPresetRow::Stiffness),
            "Reciprocal stiffness s₀"
        );
        for polarization in [
            ElectromagneticPolarization::Tm,
            ElectromagneticPolarization::Te,
        ] {
            let physics = PhysicsModel::Electromagnetic { polarization };
            assert_eq!(law_row_label(physics, LawPresetRow::Mass), "Permittivity ε");
            assert_eq!(
                law_row_label(physics, LawPresetRow::Stiffness),
                "Permeability μ"
            );
        }

        let named = |name: &str, row: LawPresetRow| {
            law_presets()
                .iter()
                .find(|preset| preset.name == name && preset.row == row)
                .map(|preset| law_preset_label(preset, PhysicsModel::Mechanical))
                .expect(name)
        };
        // Linear names no coefficient, a one-row preset names the one it acts
        // on, and the impedance-preserving pair says it takes both.
        assert_eq!(named("Linear", LawPresetRow::Both), "Linear");
        assert_eq!(
            named("Parametric pump", LawPresetRow::Stiffness),
            "Parametric pump — Reciprocal stiffness s₀"
        );
        assert_eq!(
            named("Reflectionless time interface", LawPresetRow::Both),
            "Reflectionless time interface (both rows)"
        );
    }

    #[test]
    fn material_editor_names_the_active_physical_coefficients() {
        assert_eq!(
            material_editor_labels(PhysicsModel::Mechanical),
            MaterialEditorLabels {
                mass: "Density ρ₀",
                stiffness: "Stiffness k₀",
                damping: "Damping σ",
                axis_ratio: "Stiffness axis ratio",
            }
        );
        for polarization in [
            ElectromagneticPolarization::Tm,
            ElectromagneticPolarization::Te,
        ] {
            assert_eq!(
                material_editor_labels(PhysicsModel::Electromagnetic { polarization }),
                MaterialEditorLabels {
                    mass: "Permittivity ε",
                    stiffness: "Permeability μ",
                    damping: "Loss rate α",
                    axis_ratio: "Constitutive axis ratio",
                }
            );
        }
    }
}

#[cfg(test)]
mod second_preset_reproduction {
    use super::super::test_support::{activate, settle};
    use super::super::workers::compile_gpu_upload;
    use super::*;

    /// Reported from the running application: with a driven medium the phase
    /// label churned every frame, adaptation never ran, the frame rate fell and
    /// handoffs piled up - the host was preparing the same generation over and
    /// over.
    ///
    /// A driven generation runs at the tighter step its coefficient trajectory
    /// demands, and the pacing check compared that against the *base* operator's
    /// step instead. The two differ by more than the hysteresis, so every frame
    /// asked for a step the upload would never choose and cleared the requested
    /// revision, which is a full preparation per frame.
    #[test]
    fn pacing_does_not_re_request_a_generation_running_at_its_own_step() {
        let mut state = Playground::default();
        activate(&mut state);
        let material = state.editor.document.model.draft.materials[0].clone();
        let pump = law_presets()
            .iter()
            .find(|preset| preset.name == "Parametric pump")
            .expect("catalogue entry");
        state
            .editor
            .update_material(apply_law_preset(pump, &material).unwrap())
            .unwrap();
        settle(&mut state.editor);
        let token = state
            .runtime
            .request(
                state.editor.revision,
                &state.editor.document,
                state.editor.compiled_accepted.clone(),
                MeshingOptions {
                    target_edge_length: 0.18,
                    ..MeshingOptions::default()
                },
                false,
            )
            .unwrap();
        for _ in 0..1_000_000 {
            if let Some(result) = state.runtime.advance(4096) {
                result.unwrap();
                break;
            }
        }
        let active = state.runtime.commit_ready(token).unwrap();
        assert!(active.driven());

        // The gap the old comparison saw. Without it this test would pass on a
        // medium whose two bounds happen to agree, and prove nothing.
        let driven = active.recommended_time_step();
        let base = active.operator.recommended_time_step();
        assert!(
            (base / driven - 1.0).abs() > TIME_STEP_HYSTERESIS,
            "a driven generation must run tighter than its base operator by more \
             than the hysteresis for this to be the case it was reported as: \
             {driven:e} against {base:e}"
        );

        state.uploaded_time_step =
            paced_time_step(driven, state.editor.document.presentation.simulation_speed);
        state.requested_revision = Some(state.editor.revision);
        state.retime_for_speed();
        assert_eq!(
            state.requested_revision,
            Some(state.editor.revision),
            "a generation already running at its own step must not be re-requested"
        );
    }

    /// Reported from the running application: applying a preset ran the driven
    /// medium and then, a second or two later, reverted to a stationary one
    /// with the field reset.
    ///
    /// A preparation that reuses its operator has to reuse the temporal
    /// operator built over it. `operator_scene_eq` compares whole materials, so
    /// a reused operator means the laws are the ones it was compiled against -
    /// but the temporal operator was rebuilt only on the assembly path, so
    /// every preparation after the first read as inert. The medium stopped
    /// being driven on its own, and because drivenness had changed the two
    /// generations shared no state to transfer, which reset the field as well.
    #[test]
    fn a_driven_generation_stays_driven_across_repeated_preparations() {
        let mut state = Playground::default();
        activate(&mut state);
        let material = state.editor.document.model.draft.materials[0].clone();
        let pump = law_presets()
            .iter()
            .find(|preset| preset.name == "Parametric pump")
            .expect("catalogue entry");
        state
            .editor
            .update_material(apply_law_preset(pump, &material).unwrap())
            .unwrap();
        settle(&mut state.editor);

        // Nothing changes between rounds, so every one of them describes the
        // same driven medium. The application prepares repeatedly while it
        // runs, and it was the second round that undrove the scene.
        for round in 0..4 {
            let token = state
                .runtime
                .request(
                    state.editor.revision,
                    &state.editor.document,
                    state.editor.compiled_accepted.clone(),
                    MeshingOptions {
                        target_edge_length: 0.18,
                        ..MeshingOptions::default()
                    },
                    false,
                )
                .unwrap();
            for _ in 0..1_000_000 {
                if let Some(result) = state.runtime.advance(4096) {
                    result.unwrap();
                    break;
                }
            }
            let prepared = state.runtime.commit_ready(token).unwrap();
            assert!(
                prepared.driven(),
                "round {round} prepared a stationary medium from a driven document"
            );
        }
    }

    /// Reported from the running application: the first parametric pump
    /// applied, the next preset never committed, the runtime sat at "Ready for
    /// GPU upload" and Reset did nothing.
    ///
    /// This walks the reported sequence the way the application does - editing
    /// against a generation that is already running, so the candidates are not
    /// fresh. Two driven generations share a layout and hand off; a generation
    /// that stops being driven shares nothing and is installed instead. Which
    /// of those happened decides the generation the upload waits for, and
    /// reading the candidate's `fresh` flag instead left it waiting forever
    /// with Reset gated behind the upload it was stuck in.
    #[test]
    fn presets_hand_off_in_both_directions_of_a_drivenness_change() {
        let mut state = Playground::default();
        activate(&mut state);

        let apply = |state: &mut Playground, name: &str| {
            let material = state.editor.document.model.draft.materials[0].clone();
            let preset = law_presets()
                .iter()
                .find(|preset| preset.name == name)
                .expect("catalogue entry");
            state
                .editor
                .update_material(apply_law_preset(preset, &material).unwrap())
                .unwrap();
            settle(&mut state.editor);
            let token = state
                .runtime
                .request(
                    state.editor.revision,
                    &state.editor.document,
                    state.editor.compiled_accepted.clone(),
                    MeshingOptions {
                        target_edge_length: 0.18,
                        ..MeshingOptions::default()
                    },
                    false,
                )
                .unwrap();
            for _ in 0..1_000_000 {
                if let Some(result) = state.runtime.advance(4096) {
                    result.unwrap();
                    return state.runtime.commit_ready(token).unwrap();
                }
            }
            panic!("preparation did not finish");
        };

        // Each direction packs at the step the host would choose for it - the
        // candidate's - rather than at one generation's step for all of them.
        // A single shared step is the one thing that always works, and packing
        // that way hid a refusal for every switch that loosened it.
        let speed = state.editor.document.presentation.simulation_speed;
        let step =
            |prepared: &PreparedTopology| paced_time_step(prepared.recommended_time_step(), speed);

        let pumped = apply(&mut state, "Parametric pump");
        assert!(pumped.driven() && !pumped.fresh);

        let crystal = apply(&mut state, "Time crystal");
        assert!(crystal.driven() && !crystal.fresh);
        let packed = compile_gpu_upload(
            PreparedTopology::clone(&crystal),
            Some(pumped),
            step(&crystal),
            [0; 4],
        )
        .expect("one driven generation hands off to another");
        let transfer = packed
            .transfer
            .as_ref()
            .expect("two driven generations share a layout and transfer between them");
        // The handoff compares these against both plans. A transfer that never
        // had the material runtime mapping installed reads `(0, 0)` while the
        // plans hold a record each, and is refused with "canonical GPU handoff
        // layouts do not match" however well its state maps.
        assert_eq!(
            transfer.material_runtime_counts(),
            (1, 1),
            "a driven handoff must describe the runtime records its plans hold"
        );

        let inert = apply(&mut state, "Linear");
        assert!(!inert.driven(), "Linear must undrive the generation");
        assert!(
            !inert.fresh,
            "an edit against a running generation does not start from zero"
        );
        // A medium that stops being driven still shares its field with what
        // came before. Only the runtime bank goes, and a bank with no records
        // to write is nothing to carry, so this hands off like any other edit.
        let packed = compile_gpu_upload(
            PreparedTopology::clone(&inert),
            Some(crystal),
            step(&inert),
            [0; 4],
        )
        .expect("undriving a generation still packs");
        let transfer = packed
            .transfer
            .as_ref()
            .expect("a generation that stops being driven keeps its field");
        assert_eq!(
            transfer.material_runtime_counts(),
            (1, 0),
            "the bank it had is dropped and none is written"
        );
    }

    /// Reported from the running application, reproducibly: select the
    /// parametric pump, then switch back to linear, and after seconds of
    /// "waiting for GPU upload" the host refuses and silently keeps the pump.
    ///
    /// The host packs with the *candidate's* step. A driven generation runs at
    /// the tighter step its coefficient trajectory demands, so the linear
    /// candidate's step is the larger of the two - and the pack rebuilds the
    /// *source* plan at that same step to read its drives back. The pump will
    /// not hold it, so the source plan refuses and the whole upload dies.
    /// Nothing is wrong with either generation or with the handoff maps; the
    /// two just do not share a step, and only the source has to.
    ///
    /// The sibling test above hides this by packing every direction at the
    /// pump's step, which is the one step that always works.
    #[test]
    fn undriving_packs_at_the_step_the_host_actually_uses() {
        let mut state = Playground::default();
        activate(&mut state);

        let apply = |state: &mut Playground, name: &str| {
            let material = state.editor.document.model.draft.materials[0].clone();
            let preset = law_presets()
                .iter()
                .find(|preset| preset.name == name)
                .expect("catalogue entry");
            state
                .editor
                .update_material(apply_law_preset(preset, &material).unwrap())
                .unwrap();
            settle(&mut state.editor);
            let token = state
                .runtime
                .request(
                    state.editor.revision,
                    &state.editor.document,
                    state.editor.compiled_accepted.clone(),
                    MeshingOptions {
                        target_edge_length: 0.18,
                        ..MeshingOptions::default()
                    },
                    false,
                )
                .unwrap();
            for _ in 0..1_000_000 {
                if let Some(result) = state.runtime.advance(4096) {
                    result.unwrap();
                    return state.runtime.commit_ready(token).unwrap();
                }
            }
            panic!("preparation did not finish");
        };

        let pumped = apply(&mut state, "Parametric pump");
        assert!(pumped.driven());
        let inert = apply(&mut state, "Linear");
        assert!(!inert.driven());

        let speed = state.editor.document.presentation.simulation_speed;
        let candidate_step = paced_time_step(inert.recommended_time_step(), speed);
        assert!(
            candidate_step
                > pumped
                    .canonical_temporal_operator
                    .as_ref()
                    .expect("a driven generation carries a temporal operator")
                    .maximum_time_step(),
            "the reproduction needs the candidate step to overrun the pump, \
             candidate {candidate_step}, pump ceiling {}",
            pumped
                .canonical_temporal_operator
                .as_ref()
                .unwrap()
                .maximum_time_step()
        );

        compile_gpu_upload(
            PreparedTopology::clone(&inert),
            Some(pumped),
            candidate_step,
            [0; 4],
        )
        .expect("switching back to linear must pack at the step the host chose");
    }

    /// Switching a drive *on* is the direction that was reported: the field
    /// vanished the moment a non-stationary material was enabled.
    ///
    /// The two generations share everything that exists in both. What the
    /// target has and the source does not is the material runtime bank, and a
    /// record with no source is written from the target's own authored anchors,
    /// which is where a drive just switched on should begin. So there is
    /// nothing to invent and no reason to drop the field.
    #[test]
    fn enabling_a_drive_keeps_the_field() {
        let mut state = Playground::default();
        activate(&mut state);
        let prepare = |state: &mut Playground| {
            let token = state
                .runtime
                .request(
                    state.editor.revision,
                    &state.editor.document,
                    state.editor.compiled_accepted.clone(),
                    MeshingOptions {
                        target_edge_length: 0.18,
                        ..MeshingOptions::default()
                    },
                    false,
                )
                .unwrap();
            for _ in 0..1_000_000 {
                if let Some(result) = state.runtime.advance(4096) {
                    result.unwrap();
                    return state.runtime.commit_ready(token).unwrap();
                }
            }
            panic!("preparation did not finish");
        };
        let inert = prepare(&mut state);
        assert!(!inert.driven());

        let material = state.editor.document.model.draft.materials[0].clone();
        let pump = law_presets()
            .iter()
            .find(|preset| preset.name == "Parametric pump")
            .expect("catalogue entry");
        state
            .editor
            .update_material(apply_law_preset(pump, &material).unwrap())
            .unwrap();
        settle(&mut state.editor);
        let driven = prepare(&mut state);
        assert!(driven.driven() && !driven.fresh);

        let packed = compile_gpu_upload(
            PreparedTopology::clone(&driven),
            Some(inert),
            driven.recommended_time_step(),
            [0; 4],
        )
        .expect("enabling a drive still packs");
        let transfer = packed
            .transfer
            .as_ref()
            .expect("enabling a drive must carry the field across");
        assert_eq!(
            transfer.material_runtime_counts(),
            (0, 1),
            "the target's bank is written from its own anchors, with no source"
        );
    }
}
