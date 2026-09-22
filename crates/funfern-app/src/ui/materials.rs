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
            // The law catalogue. A preset writes the slots and creates the
            // parameters it exposes; after that the material stands on its own,
            // so editing a slot by hand leaves it Custom rather than being
            // refitted to the preset it came from.
            ui.separator();
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
            // Recomputed, because applying a preset above changed the material
            // the rest of this section describes.
            if let Some(found) = identify_law_preset(&material) {
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

            ui.collapsing("Parameters", |ui| {
                let mut remove = None;
                let referenced_names = material
                    .parameter_names()
                    .map(str::to_owned)
                    .collect::<BTreeSet<_>>();
                for (index, parameter) in material.parameters.iter_mut().enumerate() {
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
