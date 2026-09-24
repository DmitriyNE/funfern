//! The material library and subdomain assignment: the Materials inspector,
//! its face and region listings, and the editor for one region's properties.

use bevy::prelude::*;
use bevy_egui::egui::{self, Color32};
use funfern_core::*;
use std::collections::BTreeSet;

use super::*;

impl Playground {
    /// The material a drawn subdomain is given. The selection outlives the
    /// document it was made in - a scene load, an undo past the material's
    /// creation - so it is resolved against the draft each time it is used
    /// rather than trusted: the default material if the draft has it,
    /// otherwise its first.
    pub(super) fn resolved_material_selection(&mut self) -> MaterialId {
        let materials = &self.editor.document.model.draft.materials;
        if !materials
            .iter()
            .any(|material| material.id == self.material_selection)
        {
            self.material_selection = materials
                .iter()
                .find(|material| material.id == DEFAULT_MATERIAL)
                .or_else(|| materials.first())
                .map_or(DEFAULT_MATERIAL, |material| material.id);
        }
        self.material_selection
    }

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
            // A view of the same materials, kept with the document's other
            // view settings rather than as an undoable edit.
            ui.checkbox(
                &mut self.editor.document.presentation.advanced_materials,
                "Advanced",
            )
            .on_hover_text(
                "Every law slot of both rows, the loss channels and the effective law \
                 in numbers, instead of the preset's named values",
            );
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
            let advanced = self.editor.document.presentation.advanced_materials;
            // The simple view is a linear material or one of the catalogue's
            // media, whole; composing laws by hand is Advanced, where Response
            // names only the two rows.
            if !advanced {
                self.medium_selector(ui, &mut material, physics);
            }
            let matched = identify_law_preset(&material);
            let mut chosen = None;
            if advanced {
                ui.horizontal(|ui| {
                    ui.label("Response");
                    egui::ComboBox::from_id_salt(("material-response", material.id.0))
                        .selected_text(matched.as_ref().map_or_else(
                            || "Custom".to_owned(),
                            |found| law_preset_label(found.preset, physics),
                        ))
                        .show_ui(ui, |ui| {
                            // A field-dependent response does not run beside van der
                            // Pol, so it is not offered there.
                            let self_oscillating = law_editor::self_oscillating(&material);
                            for preset in law_presets() {
                                let current =
                                    matched.as_ref().is_some_and(|found| found.preset == preset);
                                let offered =
                                    !(self_oscillating && preset.id.starts_with("M-F")) || current;
                                if ui
                                    .add_enabled(
                                        offered,
                                        egui::Button::selectable(
                                            current,
                                            law_preset_label(preset, physics),
                                        ),
                                    )
                                    .on_hover_text(preset.phenomenon)
                                    .on_disabled_hover_text(law_editor::SELF_OSCILLATING_RESPONSE)
                                    .clicked()
                                {
                                    chosen = Some(preset);
                                }
                            }
                        });
                });
            }
            if let Some(preset) = chosen {
                match apply_law_preset(preset, &material) {
                    Ok(applied) => material = applied,
                    Err(error) => self.notify(error.to_string()),
                }
            }
            let labels = material_editor_labels(physics);
            let sources = source_frequencies(
                &self.editor.document.model.source,
                &self.editor.document.model.draft,
            );
            // Recomputed, because applying a preset above changed the material.
            // The simple view shows a preset's values only for a medium it
            // names; a composed material reads Custom there.
            let medium = identify_medium_preset(&material, physics);
            let (matched, preset_owned) = if advanced {
                let matched = identify_law_preset(&material);
                let owned = matched
                    .as_ref()
                    .map(|found| found.parameters.iter().cloned().collect::<BTreeSet<_>>())
                    .unwrap_or_default();
                (matched, owned)
            } else {
                let owned = medium
                    .iter()
                    .flat_map(MediumPresetMatch::parameters)
                    .cloned()
                    .collect::<BTreeSet<_>>();
                (medium.as_ref().map(|found| found.response.clone()), owned)
            };
            if !advanced && let Some(found) = &medium {
                medium_values(ui, &mut material, found);
            }
            // A preset acting on both rows at once has one set of values for
            // the pair, so they sit here rather than under either row.
            if matched
                .as_ref()
                .is_some_and(|found| found.preset.row == LawPresetRow::Both)
            {
                preset_values(ui, &mut material, matched.as_ref(), None, &sources);
            }
            let strength = self
                .nonlinear_strength
                .iter()
                .find(|strength| strength.material == material.id)
                .copied();
            // `k₀` and `s₀` are one coefficient shown two ways; each editor
            // caches its own text, so the one not on screen forgets it rather
            // than reappearing with a value from before the other was edited.
            // The legacy damping slot has no editor of its own any more.
            let reciprocal = advanced && physics == PhysicsModel::Mechanical;
            for hidden in [
                if reciprocal {
                    1
                } else {
                    law_editor::RECIPROCAL_STIFFNESS
                },
                2,
            ] {
                self.material_formula_edits.remove(&(material.id.0, hidden));
                self.material_formula_errors
                    .remove(&(material.id.0, hidden));
            }
            // Advanced can write each row's law by its names or with every
            // expression evaluated; the names say where a number comes from,
            // the numbers what it is. A view setting, not an edit.
            let numbers = advanced && self.editor.document.presentation.law_formula_numbers;
            if advanced {
                ui.horizontal(|ui| {
                    ui.small("Effective law");
                    let presentation = &mut self.editor.document.presentation;
                    ui.selectable_value(&mut presentation.law_formula_numbers, false, "Names");
                    ui.selectable_value(&mut presentation.law_formula_numbers, true, "Numbers");
                });
            }
            // One group per coefficient: its base value, its loss, and every
            // law that multiplies it, so nothing about ε is found under μ.
            for row in [LawPresetRow::Mass, LawPresetRow::Stiffness] {
                let title = match row {
                    LawPresetRow::Stiffness if reciprocal => "Reciprocal stiffness s₀",
                    LawPresetRow::Stiffness => labels.stiffness,
                    _ => labels.mass,
                };
                egui::CollapsingHeader::new(title)
                    .id_salt(("material-row", material.id.0, row == LawPresetRow::Mass))
                    .default_open(true)
                    .show(ui, |ui| {
                        // What this row composes to, before the controls that
                        // compose it.
                        law_editor::row_formula(ui, &material, physics, row, numbers);
                        let mut formulas = law_editor::FormulaEdits {
                            edits: &mut self.material_formula_edits,
                            errors: &mut self.material_formula_errors,
                        };
                        match row {
                            LawPresetRow::Stiffness if reciprocal => {
                                law_editor::reciprocal_stiffness_editor(
                                    ui,
                                    &mut material,
                                    &mut formulas,
                                );
                            }
                            LawPresetRow::Stiffness => material_scalar_editor(
                                ui,
                                (material.id.0, 1),
                                "Base",
                                &mut material.stiffness,
                                &material.parameters,
                                0.000001,
                                formulas.edits,
                                formulas.errors,
                            ),
                            _ => material_scalar_editor(
                                ui,
                                (material.id.0, 0),
                                "Base",
                                &mut material.mass_density,
                                &material.parameters,
                                0.000001,
                                formulas.edits,
                                formulas.errors,
                            ),
                        }
                        law_editor::loss_rate_editor(
                            ui,
                            &mut material,
                            physics,
                            row,
                            advanced,
                            &mut formulas,
                        );
                        if advanced {
                            law_editor::law_slots_editor(
                                ui,
                                &mut material,
                                row,
                                &sources,
                                &mut formulas,
                            );
                        } else {
                            preset_values(ui, &mut material, matched.as_ref(), Some(row), &sources);
                        }
                        // How far the field has taken this coefficient from its
                        // small-signal value right now, so a Kerr run that is
                        // barely nonlinear is told apart from one running no
                        // law at all.
                        if let Some(strength) = strength
                            && let Some(line) =
                                nonlinear_strength_line(&material, &strength, physics, row)
                        {
                            ui.small(line).on_hover_text(
                                "The largest change of this coefficient anywhere in the \
                                 material, from the latest field: for Kerr it is χ|u|². A few \
                                 percent is nearly linear; self-focusing and harmonics become \
                                 plain towards 100%.",
                            );
                        }
                    });
            }
            // Gate O: the restoring force on the integrated field, which is
            // not a coefficient and so is not under either row. The simple
            // view reaches it through the medium presets.
            if advanced {
                egui::CollapsingHeader::new("Restoring force")
                    .id_salt(("material-restoring", material.id.0))
                    .default_open(!material.restoring.is_none())
                    .show(ui, |ui| {
                        let mut formulas = law_editor::FormulaEdits {
                            edits: &mut self.material_formula_edits,
                            errors: &mut self.material_formula_errors,
                        };
                        if let Some(error) = law_editor::restoring_editor(
                            ui,
                            &mut material,
                            physics,
                            numbers,
                            &mut formulas,
                        ) {
                            self.notify(error);
                        }
                    });
            }
            egui::CollapsingHeader::new("Anisotropy")
                .id_salt(("material-anisotropy", material.id.0))
                .default_open(material.axis_ratio != ScalarField::constant(1.0))
                .show(ui, |ui| {
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
                });
            // A Switch's ramp is one number on the material rather than a slot
            // on a row: it moves every row's alternate together. Zero is a
            // hard temporal interface.
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
                // Throwing the Switch is a run-time act, not an edit: it is
                // stamped on the device clock and leaves the document and its
                // undo history alone.
                let state = self
                    .switch_states
                    .iter()
                    .find(|(id, _, _)| *id == material.id)
                    .copied();
                let heading = self
                    .switch_targets
                    .get(&material.id)
                    .copied()
                    .or(state.map(|(_, target, _)| target >= 0.5));
                ui.horizontal(|ui| {
                    let label = if heading == Some(true) {
                        "Switch ◂"
                    } else {
                        "Switch ▸"
                    };
                    if ui
                        .add_enabled(state.is_some(), egui::Button::new(label))
                        .on_hover_text(
                            "Ramp this material to its alternate law, or back, over the \
                             Switch ramp. Hotkey S. It acts on the running medium and is \
                             not an edit.",
                        )
                        .on_disabled_hover_text("Runs once the medium is running")
                        .clicked()
                    {
                        self.pending_switch = Some(material.id);
                    }
                    if let Some((_, target, now)) = state {
                        ui.small(switch_state_text(target, now));
                    }
                });
            }
            // The restoring row belongs to neither coefficient.
            if let Ok(Some(line)) =
                restoring_law_summary(&material, physics, LawSummaryDetail::Named)
            {
                ui.small(format!("{} = {}", line.subject, line.response));
            }

            // Only the parameters the user made. A preset's own are above,
            // under the labels it gave them, and showing them again here was
            // two controls for one number.
            let material_id = material.id.0;
            let mut renames = Vec::new();
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
                        // Renamed on commit, every formula that uses the name
                        // rewritten with it or none at all.
                        // Keyed by position and the name it was opened on, so
                        // text typed against a name that has since changed
                        // (an undo, a preset) is dropped, not committed.
                        let key = (material_id, index);
                        let entry = self
                            .parameter_name_edits
                            .entry(key)
                            .or_insert_with(|| (parameter.name.clone(), parameter.name.clone()));
                        if entry.0 != parameter.name {
                            *entry = (parameter.name.clone(), parameter.name.clone());
                        }
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut entry.1)
                                .desired_width(72.0)
                                .hint_text("name"),
                        );
                        if response.lost_focus() && entry.1 != parameter.name {
                            renames.push((index, entry.1.clone()));
                        }
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
                if let Some(index) = remove
                    && let Err(error) =
                        delete_parameter(&mut material, index, &mut self.parameter_name_edits)
                {
                    self.notify(format!("Cannot delete the parameter: {error}"));
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
            for (index, name) in renames {
                self.parameter_name_edits.remove(&(material_id, index));
                if material.rename_parameter(index, name.clone()).is_err() {
                    self.notify(format!(
                        "Cannot rename to {name:?}: it must be a new, non-reserved identifier"
                    ));
                }
            }
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

/// Every enabled source with a frequency, named for the pump helper: the
/// point source and each region's volume source.
pub(super) fn source_frequencies(
    source: &PointSource,
    scene: &TopologyScene,
) -> Vec<(String, f64)> {
    let frequency = |signal: &TimeSignal| match signal {
        TimeSignal::Harmonic {
            amplitude,
            frequency_hz,
            ..
        } if *amplitude != 0.0 && *frequency_hz > 0.0 => Some(*frequency_hz),
        _ => None,
    };
    let mut found = Vec::new();
    if source.enabled
        && let Some(hz) = frequency(&source.signal)
    {
        found.push(("point source".to_owned(), hz));
    }
    for volume in scene.volume_sources.iter().filter(|volume| volume.enabled) {
        if let Some(hz) = frequency(&volume.signal) {
            found.push((
                if volume.region == BACKGROUND_REGION {
                    "background source".to_owned()
                } else {
                    format!("region {} source", volume.region.0)
                },
                hz,
            ));
        }
    }
    found
}

/// `= 2 × source`: a pump at twice a source's frequency amplifies what that
/// source launches. With several sources the button asks which.
pub(super) fn double_source_button(
    ui: &mut egui::Ui,
    sources: &[(String, f64)],
    frequency: &mut f64,
) -> bool {
    match sources {
        [] => {
            ui.add_enabled(false, egui::Button::new("= 2 × source").small())
                .on_disabled_hover_text("No enabled source has a frequency");
            false
        }
        [(name, hz)] => {
            let clicked = ui
                .add(egui::Button::new("= 2 × source").small())
                .on_hover_text(format!("Pump at twice the {name}'s {hz} Hz"))
                .clicked();
            if clicked {
                *frequency = 2.0 * hz;
            }
            clicked
        }
        several => {
            let mut chosen = None;
            ui.menu_button("= 2 × …", |ui| {
                for (name, hz) in several {
                    if ui.button(format!("{name} ({hz} Hz)")).clicked() {
                        chosen = Some(*hz);
                        ui.close();
                    }
                }
            });
            if let Some(hz) = chosen {
                *frequency = 2.0 * hz;
            }
            chosen.is_some()
        }
    }
}

/// Where a material's Switch stands: at either end, or how far along its
/// ramp towards the end it is headed for.
fn switch_state_text(target: f64, now: f64) -> String {
    let (end, progress) = if target >= 0.5 {
        ("alternate", now)
    } else {
        ("base", 1.0 - now)
    };
    if progress >= 0.999 {
        format!("At {end}")
    } else {
        format!("Ramping to {end}: {:.0}%", 100.0 * progress)
    }
}

impl Playground {
    /// The Switch the hotkey throws: the material open in the editor if it
    /// has one, otherwise the document's only Switch material.
    pub(super) fn request_material_switch(&mut self) {
        let switchable = self
            .editor
            .document
            .model
            .accepted
            .materials
            .iter()
            .filter(|material| {
                material.mass_law.alternate.is_some() || material.stiffness_law.alternate.is_some()
            })
            .map(|material| material.id)
            .collect::<Vec<_>>();
        let chosen = self
            .material_edit
            .as_ref()
            .map(|material| material.id)
            .filter(|id| switchable.contains(id))
            .or_else(|| (switchable.len() == 1).then(|| switchable[0]));
        match chosen {
            Some(material) => self.pending_switch = Some(material),
            None if switchable.is_empty() => {
                self.message = "No material here has a Switch".into();
            }
            None => {
                self.message = "Open the material to switch in the editor first".into();
            }
        }
    }
}

/// The medium a simple-view material is, from the catalogue, and a Custom
/// that says where a composed one is edited.
fn medium_label(preset: &MediumPreset, physics: PhysicsModel) -> String {
    let restoring = preset.restoring();
    match (preset.self_oscillating, restoring.id.is_empty()) {
        (true, false) => "Van der Pol oscillators".to_owned(),
        (true, true) => "Self-oscillating medium".to_owned(),
        (false, false) => restoring_preset_text(restoring, physics).name,
        (false, true) => law_preset_label(preset.response(), physics),
    }
}

fn medium_hover(preset: &MediumPreset, physics: PhysicsModel) -> String {
    let restoring = preset.restoring();
    let text = restoring_preset_text(restoring, physics);
    match (preset.self_oscillating, restoring.id.is_empty()) {
        (true, false) => format!(
            "A Klein-Gordon cutoff ω₀ makes every point an oscillator, and van der Pol makes \
             each one self-sustained: a small field grows to a limit cycle at ω₀.\n\n{}",
            van_der_pol_text(physics)
        ),
        (true, true) => van_der_pol_text(physics),
        (false, false) => format!("{}.\n\n{}", text.phenomenon, text.equation),
        (false, true) => preset.response().phenomenon.to_owned(),
    }
}

impl Playground {
    /// Response in the simple view: the whole medium, from the catalogue.
    fn medium_selector(
        &mut self,
        ui: &mut egui::Ui,
        material: &mut Material,
        physics: PhysicsModel,
    ) {
        let matched = identify_medium_preset(material, physics);
        let mut chosen = None;
        ui.horizontal(|ui| {
            ui.label("Response");
            egui::ComboBox::from_id_salt(("material-medium", material.id.0))
                .selected_text(matched.as_ref().map_or_else(
                    || "Custom".to_owned(),
                    |found| medium_label(found.preset, physics),
                ))
                .show_ui(ui, |ui| {
                    for preset in medium_presets() {
                        let current = matched.as_ref().is_some_and(|found| found.preset == preset);
                        if ui
                            .selectable_label(current, medium_label(preset, physics))
                            .on_hover_text(medium_hover(preset, physics))
                            .clicked()
                        {
                            chosen = Some(preset);
                        }
                    }
                });
        });
        if matched.is_none() {
            ui.small("Composed by hand: its laws are edited in Advanced.");
        }
        if let Some(preset) = chosen {
            match apply_medium_preset(preset, material, physics) {
                Ok(applied) => *material = applied,
                Err(error) => self.notify(error.to_string()),
            }
        }
    }
}

/// The values of a medium's restoring law and self-oscillation, which belong
/// to neither row, and the cutoff they set.
fn medium_values(ui: &mut egui::Ui, material: &mut Material, found: &MediumPresetMatch) {
    // The response's own values sit with the row they act on.
    let rows = found.preset.response().variables.len();
    for (variable, name) in found.preset.variables().zip(found.parameters()).skip(rows) {
        preset_variable(ui, material, variable, name, &[]);
    }
    law_editor::cutoff_line(ui, material);
}

/// One value a preset asks for, as the label it gave it.
fn preset_variable(
    ui: &mut egui::Ui,
    material: &mut Material,
    variable: &LawPresetVariable,
    name: &str,
    sources: &[(String, f64)],
) {
    let Some(parameter) = material
        .parameters
        .iter_mut()
        .find(|parameter| parameter.name == name)
    else {
        return;
    };
    ui.horizontal(|ui| {
        ui.label(variable.label);
        ui.add(
            egui::DragValue::new(&mut parameter.value)
                .speed(0.005)
                .range(variable.minimum..=variable.maximum)
                .update_while_editing(false),
        );
        if variable.parameter == "pump_hz" {
            double_source_button(ui, sources, &mut parameter.value);
        }
    });
}

/// The preset's own values, as the labels it gave them. With `row`, only a
/// one-row preset acting on that row; without, only a preset acting on both.
fn preset_values(
    ui: &mut egui::Ui,
    material: &mut Material,
    matched: Option<&LawPresetMatch>,
    row: Option<LawPresetRow>,
    sources: &[(String, f64)],
) {
    let Some(found) = matched else { return };
    let belongs = match row {
        Some(row) => found.preset.row == row,
        None => found.preset.row == LawPresetRow::Both,
    };
    if !belongs {
        return;
    }
    for (variable, name) in found.preset.variables.iter().zip(&found.parameters) {
        preset_variable(ui, material, variable, name, sources);
    }
}

/// Deletes one of a material's parameters, refused while a formula still uses
/// it, and drops the half-typed names keyed by position, which the removal
/// shifts. This used to run inside a `debug_assert!`, which a release build
/// compiles out with its argument, so the button did nothing there.
fn delete_parameter(
    material: &mut Material,
    index: usize,
    name_edits: &mut BTreeMap<(u64, usize), (String, String)>,
) -> Result<(), MaterialError> {
    material.remove_parameter(index)?;
    let owner = material.id.0;
    name_edits.retain(|(material, _), _| *material != owner);
    Ok(())
}

/// The row's peak change from its small-signal value, if its law follows the
/// field.
fn nonlinear_strength_line(
    material: &Material,
    strength: &CanonicalNonlinearStrength,
    physics: PhysicsModel,
    row: LawPresetRow,
) -> Option<String> {
    let (law, value) = match row {
        LawPresetRow::Stiffness => (&material.stiffness_law, strength.complementary),
        _ => (&material.mass_law, strength.primary),
    };
    if law.field == FieldLaw::Linear {
        return None;
    }
    let percent = 100.0 * value;
    let percent = if percent < 0.1 {
        format!("{percent:.2}%")
    } else {
        format!("{percent:.1}%")
    };
    Some(format!(
        "Now: {} up to +{percent} from its small-signal value",
        law_row_label(physics, row)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deleting a parameter removes it, in every build, and a parameter a
    /// formula uses is refused and kept.
    #[test]
    fn a_parameter_is_deleted_unless_a_formula_uses_it() {
        let mut material = Material::default_medium();
        material.parameters = vec![
            MaterialParameter {
                name: "p1".into(),
                value: 1.0,
            },
            MaterialParameter {
                name: "p2".into(),
                value: 2.0,
            },
        ];
        material.mass_density = ScalarField::formula("1 + p2").unwrap();
        let mut edits = BTreeMap::from([
            ((material.id.0, 1), ("p2".to_owned(), "p2x".to_owned())),
            ((material.id.0 + 1, 0), ("q".to_owned(), "q".to_owned())),
        ]);
        delete_parameter(&mut material, 0, &mut edits).unwrap();
        assert_eq!(
            material
                .parameters
                .iter()
                .map(|parameter| parameter.name.as_str())
                .collect::<Vec<_>>(),
            ["p2"]
        );
        // This material's positional edits go; another material's stay.
        assert_eq!(edits.len(), 1);
        assert!(edits.contains_key(&(material.id.0 + 1, 0)));
        assert!(delete_parameter(&mut material, 0, &mut edits).is_err());
        assert_eq!(material.parameters.len(), 1);
    }

    /// Only rows whose law follows the field are read out, in the skin's
    /// names and as the coefficient's percentage change.
    #[test]
    fn the_strength_readout_names_each_nonlinear_row() {
        let mut material = Scene::initial().materials[0].clone();
        material.mass_law.field = FieldLaw::Polynomial {
            chi1: ScalarField::constant(0.0),
            chi2: ScalarField::constant(0.8),
            amplitude_bound: None,
        };
        let strength = CanonicalNonlinearStrength {
            material: material.id,
            primary: 0.0534,
            complementary: 0.0004,
        };
        let tm = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        };
        assert_eq!(
            nonlinear_strength_line(&material, &strength, tm, LawPresetRow::Mass).as_deref(),
            Some("Now: Permittivity ε up to +5.3% from its small-signal value")
        );
        assert_eq!(
            nonlinear_strength_line(&material, &strength, tm, LawPresetRow::Stiffness),
            None,
            "a linear row has nothing to read out"
        );
        material.stiffness_law.field = FieldLaw::Saturable {
            chi: ScalarField::constant(6.0),
            saturation: ScalarField::constant(0.3),
        };
        assert_eq!(
            nonlinear_strength_line(
                &material,
                &strength,
                PhysicsModel::Mechanical,
                LawPresetRow::Stiffness
            )
            .as_deref(),
            Some("Now: Reciprocal stiffness s₀ up to +0.04% from its small-signal value")
        );
    }

    /// A loss on the complementary row has no legacy spelling at all. It must prepare, and reach the canonical solver as that row's
    /// loss.
    #[test]
    fn a_complementary_loss_prepares_and_reaches_the_solver() {
        let mut state = Playground::default();
        let mut material = state.editor.document.model.draft.materials[0].clone();
        let channel = Some(LossChannel {
            base_rate: ScalarField::constant(0.25),
            law: DampingLaw::constant(),
        });
        // The complementary row's channel in whatever skin the default
        // document uses: electric in Mechanical, magnetic in the EM skins.
        let physics = state.editor.document.model.draft.physics;
        if law_editor::row_is_electric(physics, LawPresetRow::Stiffness) {
            material.electric_loss = channel;
        } else {
            material.magnetic_loss = channel;
        }
        state.editor.update_material(material).unwrap();
        settle(&mut state.editor);
        let prepared = activate(&mut state);
        let rates = &prepared.canonical_operator;
        assert!(
            rates
                .complementary_loss_rate()
                .iter()
                .any(|rate| *rate > 0.0)
        );
        assert!(rates.primary_loss_rate().iter().all(|rate| *rate == 0.0));
    }

    /// Editing a legacy material's loss moves it to its named channel in one
    /// step, and the solver's rates are the same before and after.
    #[test]
    fn a_legacy_damping_moves_to_its_channel_with_the_same_rates() {
        let mut state = Playground::default();
        let mut material = state.editor.document.model.draft.materials[0].clone();
        material.damping = ScalarField::constant(0.3);
        state.editor.update_material(material.clone()).unwrap();
        settle(&mut state.editor);
        let legacy = activate(&mut state);
        let physics = state.editor.document.model.draft.physics;
        law_editor::adopt_legacy_damping(&mut material, physics);
        state.editor.update_material(material).unwrap();
        settle(&mut state.editor);
        let named = activate(&mut state);
        let rates = |prepared: &PreparedTopology| {
            (
                prepared.canonical_operator.primary_loss_rate().to_vec(),
                prepared
                    .canonical_operator
                    .complementary_loss_rate()
                    .to_vec(),
            )
        };
        assert!(rates(&legacy).0.iter().any(|rate| *rate > 0.0));
        assert_eq!(rates(&legacy), rates(&named));
    }

    #[test]
    fn the_advanced_view_is_presentation_not_an_edit() {
        let mut state = Playground::default();
        activate(&mut state);
        let model = state.editor.document.model.clone();
        let revision = state.editor.revision;
        state.editor.document.presentation.advanced_materials = true;
        assert_eq!(state.editor.document.model, model);
        assert_eq!(state.editor.revision, revision);
        assert!(!state.editor.undo(), "the toggle made an undo step");
        let saved = funfern_app::topology_persistence::save(&state.editor.document).unwrap();
        let loaded = funfern_app::topology_persistence::parse_document(saved.as_bytes()).unwrap();
        assert!(loaded.presentation.advanced_materials);
    }

    /// Viewing a material in the panel rewrites nothing, in either view and
    /// every skin: each medium of the catalogue, and a composition the simple
    /// view reads as Custom (Kerr beside sine-Gordon, van der Pol on constant
    /// values).
    #[test]
    fn viewing_any_medium_leaves_it_as_authored() {
        let skins = [
            PhysicsModel::Mechanical,
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            },
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            },
        ];
        for physics in skins {
            let base = Scene::initial().materials[0].clone();
            let mut materials = medium_presets()
                .iter()
                .map(|preset| apply_medium_preset(preset, &base, physics).unwrap())
                .collect::<Vec<_>>();
            let kerr = law_presets()
                .iter()
                .find(|preset| preset.id == "M-F1")
                .unwrap();
            let sine_gordon = restoring_presets()
                .iter()
                .find(|preset| preset.id == "R2")
                .unwrap();
            let composed =
                apply_restoring_preset(sine_gordon, &apply_law_preset(kerr, &base).unwrap())
                    .unwrap();
            assert_eq!(identify_medium_preset(&composed, physics), None);
            materials.push(composed);
            let mut constant = base.clone();
            law_editor::set_self_oscillating(
                &mut constant,
                physics,
                law_editor::legacy_damping_row(physics),
                true,
            );
            assert_eq!(identify_medium_preset(&constant, physics), None);
            materials.push(constant);
            for material in materials {
                for advanced in [false, true] {
                    let mut state = Playground::default();
                    state.editor.document.model.draft.physics = physics;
                    state.editor.document.presentation.advanced_materials = advanced;
                    let selection = state.resolved_material_selection();
                    let stored = Material {
                        id: selection,
                        ..material.clone()
                    };
                    let slot = state
                        .editor
                        .document
                        .model
                        .draft
                        .materials
                        .iter_mut()
                        .find(|item| item.id == selection)
                        .unwrap();
                    *slot = stored.clone();
                    let context = egui::Context::default();
                    for _ in 0..2 {
                        let _ = context.run_ui(egui::RawInput::default(), |ui| {
                            state.materials_panel(ui);
                        });
                    }
                    assert_eq!(
                        state.material_edit.as_ref(),
                        Some(&stored),
                        "{physics:?}, advanced {advanced}"
                    );
                }
            }
        }
    }

    /// The pump helper offers each enabled source that has a frequency, and
    /// only those.
    #[test]
    fn the_pump_helper_lists_every_source_with_a_frequency() {
        let mut source = PointSource {
            enabled: true,
            signal: TimeSignal::harmonic(0.0, 1.0, 2.5, 0.0),
            ..PointSource::default()
        };
        let mut scene = TopologyScene::default();
        assert_eq!(
            source_frequencies(&source, &scene),
            [("point source".to_owned(), 2.5)]
        );
        scene.volume_sources.push(VolumeSource {
            region: BACKGROUND_REGION,
            enabled: true,
            profile: ScalarField::constant(1.0),
            parameters: vec![],
            signal: TimeSignal::harmonic(0.0, 0.5, 4.0, 0.0),
        });
        assert_eq!(source_frequencies(&source, &scene).len(), 2);
        source.enabled = false;
        scene.volume_sources[0].signal = TimeSignal::harmonic(0.3, 0.0, 4.0, 0.0);
        assert!(source_frequencies(&source, &scene).is_empty());
    }

    #[test]
    fn the_switch_state_reads_its_end_or_its_progress() {
        assert_eq!(switch_state_text(0.0, 0.0), "At base");
        assert_eq!(switch_state_text(1.0, 1.0), "At alternate");
        assert_eq!(switch_state_text(1.0, 0.4), "Ramping to alternate: 40%");
        assert_eq!(switch_state_text(0.0, 0.25), "Ramping to base: 75%");
    }

    /// The hotkey finds the document's one Switch material, and the running
    /// generation compiles a runtime record the device event can name. The
    /// device side of the event is `canonical_gpu_temporal`'s gate.
    #[test]
    fn a_switch_preset_can_be_thrown_on_the_running_medium() {
        let mut state = Playground::default();
        activate(&mut state);
        state.request_material_switch();
        assert_eq!(state.pending_switch, None, "nothing here has a Switch");

        let material = state.editor.document.model.draft.materials[0].clone();
        let switch = law_presets()
            .iter()
            .find(|preset| preset.name == "Switchable medium" && preset.row == LawPresetRow::Mass)
            .expect("the catalogue offers a Switch");
        state
            .editor
            .update_material(apply_law_preset(switch, &material).unwrap())
            .unwrap();
        settle(&mut state.editor);
        let active = activate(&mut state);
        state.request_material_switch();
        assert_eq!(state.pending_switch, Some(material.id));

        let temporal = active
            .canonical_temporal_operator
            .as_ref()
            .expect("a Switch medium runs as a temporal generation");
        let runtime = temporal.initial_runtime();
        let record = runtime
            .records()
            .iter()
            .find(|record| record.material() == material.id)
            .expect("the Switch material has a runtime record");
        assert_eq!(record.switch().target_blend(), 0.0, "it starts at its base");
        funfern_app::canonical_gpu::CanonicalGpuLiveEvent::temporal_switch_in(
            &runtime,
            material.id,
            true,
            0.5,
            1,
        )
        .unwrap();
    }

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
