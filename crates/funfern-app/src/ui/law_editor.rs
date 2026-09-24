//! The Advanced material view: every law slot of both coefficient rows, the
//! named loss channels, and the effective law evaluated at a point.
//!
//! It is a way of looking at the same material as the Simplified view, not a
//! solver mode. Nothing here rewrites a formula merely because the view is
//! open; an edit is made only when a control changes, and it is committed
//! through the panel's Apply like any other. Only laws the solver runs are
//! offered: a slot that already holds one it does not run is shown as such and
//! left alone, never converted.

use std::collections::BTreeMap;

use bevy_egui::egui;
use funfern_core::*;

use super::*;

/// Keys for the formula editors, beyond the four base slots.
const ROW_SLOTS: u8 = 16;
const MASS_ROW: u8 = 16;
const STIFFNESS_ROW: u8 = 32;
const ELECTRIC_LOSS: u8 = 48;
const MAGNETIC_LOSS: u8 = 64;
pub(super) const RECIPROCAL_STIFFNESS: u8 = 80;

pub(super) struct FormulaEdits<'a> {
    pub edits: &'a mut BTreeMap<(u64, u8), String>,
    pub errors: &'a mut BTreeMap<(u64, u8), String>,
}

/// The mechanical complementary row's base coefficient, presented as the
/// reciprocal stiffness `s₀ = 1/k₀` it actually is to the solver. The stored
/// `k₀` is rewritten only when `s₀` is edited, and then as `1/s₀` with the
/// reciprocal cancelled, so a round trip neither grows the formula nor
/// changes its map.
pub(super) fn reciprocal_stiffness_editor(
    ui: &mut egui::Ui,
    material: &mut Material,
    formulas: &mut FormulaEdits,
) {
    let Ok(shown) = material.stiffness.reciprocal() else {
        ui.small("Reciprocal stiffness s₀ cannot be formed from this k₀");
        return;
    };
    let mut edited = shown.clone();
    material_scalar_editor(
        ui,
        (material.id.0, RECIPROCAL_STIFFNESS),
        "Reciprocal stiffness s₀",
        &mut edited,
        &material.parameters,
        0.000001,
        formulas.edits,
        formulas.errors,
    );
    if edited != shown {
        match edited.reciprocal() {
            Ok(stiffness) => material.stiffness = stiffness,
            Err(error) => {
                formulas
                    .errors
                    .insert((material.id.0, RECIPROCAL_STIFFNESS), error.to_string());
            }
        }
    }
    ui.small(format!(
        "Effective stiffness k₀ = {}",
        field_text(&material.stiffness)
    ));
}

/// Both rows' law slots and both loss channels.
pub(super) fn advanced_law_editor(
    ui: &mut egui::Ui,
    material: &mut Material,
    physics: PhysicsModel,
    sources: &[(String, f64)],
    formulas: &mut FormulaEdits,
) {
    let id = material.id.0;
    let parameters = material.parameters.clone();
    for (row, base, law) in [
        (LawPresetRow::Mass, MASS_ROW, &mut material.mass_law),
        (
            LawPresetRow::Stiffness,
            STIFFNESS_ROW,
            &mut material.stiffness_law,
        ),
    ] {
        ui.separator();
        ui.label(law_row_label(physics, row));
        coefficient_law_editor(ui, id, base, law, &parameters, sources, formulas);
    }
    ui.separator();
    let legacy_damping = material.damping != ScalarField::constant(0.0);
    for (label, base, channel, field) in [
        (
            "Electric loss",
            ELECTRIC_LOSS,
            &mut material.electric_loss,
            channel_field(physics, true),
        ),
        (
            "Magnetic loss",
            MAGNETIC_LOSS,
            &mut material.magnetic_loss,
            channel_field(physics, false),
        ),
    ] {
        loss_channel_editor(ui, id, base, label, field, channel, &parameters, formulas);
    }
    if legacy_damping && (material.electric_loss.is_some() || material.magnetic_loss.is_some()) {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            "Set the legacy damping to zero: it cannot run beside a named loss channel",
        );
    }
}

/// What each named channel damps in this skin, for its hover text.
fn channel_field(physics: PhysicsModel, electric: bool) -> &'static str {
    match (physics, electric) {
        (PhysicsModel::Mechanical, true) => "damps the stress-like complementary flux",
        (PhysicsModel::Mechanical, false) => "damps the displacement flux, as Damping σ did",
        (
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            },
            true,
        ) => "damps E_z, the primary field",
        (
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            },
            false,
        ) => "damps the in-plane H, the complementary field",
        (
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            },
            true,
        ) => "damps the in-plane E, the complementary field",
        (
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            },
            false,
        ) => "damps H_z, the primary field",
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ResponseKind {
    Linear,
    Kerr,
    Saturable,
    /// A law the solver does not run, kept as authored.
    Unsupported,
}

fn response_kind(field: &FieldLaw) -> ResponseKind {
    match field {
        FieldLaw::Linear => ResponseKind::Linear,
        FieldLaw::Polynomial { chi1, .. } if *chi1 == ScalarField::constant(0.0) => {
            ResponseKind::Kerr
        }
        FieldLaw::Polynomial { .. } => ResponseKind::Unsupported,
        FieldLaw::Saturable { .. } => ResponseKind::Saturable,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DriveKind {
    None,
    Pump,
    TimeCrystal,
    Travelling,
}

fn drive_kind(drive: &TimeDrive) -> DriveKind {
    match drive {
        TimeDrive::None => DriveKind::None,
        TimeDrive::ParametricPump { .. } => DriveKind::Pump,
        TimeDrive::TimeCrystal { .. } => DriveKind::TimeCrystal,
        TimeDrive::TravellingModulation { .. } => DriveKind::Travelling,
    }
}

fn coefficient_law_editor(
    ui: &mut egui::Ui,
    id: u64,
    base: u8,
    law: &mut CoefficientLaw,
    parameters: &[MaterialParameter],
    sources: &[(String, f64)],
    formulas: &mut FormulaEdits,
) {
    let kind = response_kind(&law.field);
    let mut chosen = kind;
    ui.horizontal(|ui| {
        ui.label("Field response");
        egui::ComboBox::from_id_salt(("advanced-response", id, base))
            .selected_text(match kind {
                ResponseKind::Linear => "Linear",
                ResponseKind::Kerr => "Kerr",
                ResponseKind::Saturable => "Saturable",
                ResponseKind::Unsupported => "Not executable",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut chosen, ResponseKind::Linear, "Linear");
                ui.selectable_value(&mut chosen, ResponseKind::Kerr, "Kerr")
                    .on_hover_text("ḡ = 1 + χ|u|²");
                ui.selectable_value(&mut chosen, ResponseKind::Saturable, "Saturable")
                    .on_hover_text("ḡ = 1 + χ|u|² / (1 + |u|²/σ²)");
            });
    });
    if chosen != kind {
        law.field = match chosen {
            ResponseKind::Linear | ResponseKind::Unsupported => FieldLaw::Linear,
            ResponseKind::Kerr => FieldLaw::Polynomial {
                chi1: ScalarField::constant(0.0),
                chi2: ScalarField::constant(0.8),
                amplitude_bound: None,
            },
            ResponseKind::Saturable => FieldLaw::Saturable {
                chi: ScalarField::constant(0.8),
                saturation: ScalarField::constant(1.0),
            },
        };
        forget(formulas, id, base, 0..4);
    }
    let unsupported = response_kind(&law.field) == ResponseKind::Unsupported;
    match &mut law.field {
        FieldLaw::Linear => {}
        FieldLaw::Polynomial { .. } if unsupported => {
            ui.small(
                "This polynomial has a linear term, which the solver does not run; it is \
                 kept as authored.",
            );
        }
        FieldLaw::Polynomial {
            chi2,
            amplitude_bound,
            ..
        } => {
            field_row(
                ui,
                (id, base),
                RELATIVE_CHI,
                chi2,
                parameters,
                -1.0e6,
                formulas,
            )
            .on_hover_text(RELATIVE_CHI_HOVER);
            let mut bounded = amplitude_bound.is_some();
            if ui
                .checkbox(&mut bounded, "Amplitude bound")
                .on_hover_text(
                    "The largest |u| the law may see. A defocusing law (χ < 0) needs one, \
                     and the step is chosen from the tangent up to it.",
                )
                .changed()
            {
                *amplitude_bound = bounded.then(|| ScalarField::constant(1.0));
                forget(formulas, id, base, 1..2);
            }
            if let Some(bound) = amplitude_bound {
                field_row(
                    ui,
                    (id, base + 1),
                    "Bound",
                    bound,
                    parameters,
                    0.000001,
                    formulas,
                );
            }
        }
        FieldLaw::Saturable { chi, saturation } => {
            field_row(
                ui,
                (id, base),
                RELATIVE_CHI,
                chi,
                parameters,
                -1.0e6,
                formulas,
            )
            .on_hover_text(RELATIVE_CHI_HOVER);
            field_row(
                ui,
                (id, base + 1),
                "Saturation σ",
                saturation,
                parameters,
                0.000001,
                formulas,
            );
        }
    }
    drive_editor(
        ui,
        id,
        base + 4,
        &mut law.drive,
        parameters,
        sources,
        formulas,
    );
    let mut switchable = law.alternate.is_some();
    if ui
        .checkbox(&mut switchable, "Switch alternate")
        .on_hover_text("The factor this coefficient takes while the material is switched")
        .changed()
    {
        law.alternate = switchable.then(|| ScalarField::constant(2.0));
        forget(formulas, id, base + 10, 0..1);
    }
    if let Some(alternate) = &mut law.alternate {
        field_row(
            ui,
            (id, base + 10),
            "Alternate factor",
            alternate,
            parameters,
            0.000001,
            formulas,
        );
    }
    if law.field == FieldLaw::Linear {
        ui.checkbox(&mut law.inverted, "Divide the coefficient")
            .on_hover_text(
                "The drive and Switch divide this coefficient instead of multiplying it",
            );
    } else if law.inverted {
        ui.small("A divided field response does not run; clear it or choose Linear.");
    }
}

const RELATIVE_CHI: &str = "Nonlinearity χ";
const RELATIVE_CHI_HOVER: &str = "Relative: the coefficient is c₀(1 + χ|u|²), so χ has units of 1/|u|². The \
     absolute cubic coefficient of the expanded map c₀u + a₃|u|²u is a₃ = c₀χ, a \
     different quantity with the base coefficient's own spatial dependence.";

fn drive_editor(
    ui: &mut egui::Ui,
    id: u64,
    base: u8,
    drive: &mut TimeDrive,
    parameters: &[MaterialParameter],
    sources: &[(String, f64)],
    formulas: &mut FormulaEdits,
) {
    let kind = drive_kind(drive);
    let mut chosen = kind;
    ui.horizontal(|ui| {
        ui.label("Drive");
        egui::ComboBox::from_id_salt(("advanced-drive", id, base))
            .selected_text(match kind {
                DriveKind::None => "None",
                DriveKind::Pump => "Parametric pump",
                DriveKind::TimeCrystal => "Time crystal",
                DriveKind::Travelling => "Travelling modulation",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut chosen, DriveKind::None, "None");
                ui.selectable_value(&mut chosen, DriveKind::Pump, "Parametric pump")
                    .on_hover_text("1 + depth·cos(2πft + phase)");
                ui.selectable_value(&mut chosen, DriveKind::TimeCrystal, "Time crystal")
                    .on_hover_text("1 + depth·tanh(s·cos(2πft + phase))/tanh(s)");
                ui.selectable_value(&mut chosen, DriveKind::Travelling, "Travelling modulation")
                    .on_hover_text("1 + depth·cos(2πft − q·x + phase)");
            });
    });
    if chosen != kind {
        let constant = ScalarField::constant;
        *drive = match chosen {
            DriveKind::None => TimeDrive::None,
            DriveKind::Pump => TimeDrive::ParametricPump {
                depth: constant(0.2),
                frequency_hz: constant(1.0),
                phase_radians: constant(0.0),
            },
            DriveKind::TimeCrystal => TimeDrive::TimeCrystal {
                depth: constant(0.2),
                frequency_hz: constant(1.0),
                phase_radians: constant(0.0),
                sharpness: constant(4.0),
            },
            DriveKind::Travelling => TimeDrive::TravellingModulation {
                depth: constant(0.2),
                frequency_hz: constant(1.0),
                phase_radians: constant(0.0),
                wavenumber: constant(2.0),
                angle_radians: constant(0.0),
            },
        };
        forget(formulas, id, base, 0..5);
    }
    // A constant pump frequency can be set from a source; one written as a
    // formula is the author's, and is left to them.
    let frequency = match drive {
        TimeDrive::ParametricPump { frequency_hz, .. }
        | TimeDrive::TimeCrystal { frequency_hz, .. }
        | TimeDrive::TravellingModulation { frequency_hz, .. } => Some(frequency_hz),
        TimeDrive::None => None,
    };
    if let Some(ScalarField::Constant(value)) = frequency
        && !sources.is_empty()
    {
        ui.horizontal(|ui| {
            ui.small("Frequency from");
            super::materials::double_source_button(ui, sources, value);
        });
    }
    let mut row = |slot: u8, label: &str, field: &mut ScalarField, minimum: f64| {
        field_row(
            ui,
            (id, base + slot),
            label,
            field,
            parameters,
            minimum,
            formulas,
        );
    };
    match drive {
        TimeDrive::None => {}
        TimeDrive::ParametricPump {
            depth,
            frequency_hz,
            phase_radians,
        } => {
            row(0, "Depth", depth, 0.0);
            row(1, "Frequency", frequency_hz, 0.0);
            row(2, "Phase", phase_radians, -1.0e6);
        }
        TimeDrive::TimeCrystal {
            depth,
            frequency_hz,
            phase_radians,
            sharpness,
        } => {
            row(0, "Depth", depth, 0.0);
            row(1, "Frequency", frequency_hz, 0.0);
            row(2, "Phase", phase_radians, -1.0e6);
            row(3, "Edge sharpness", sharpness, 0.000001);
        }
        TimeDrive::TravellingModulation {
            depth,
            frequency_hz,
            phase_radians,
            wavenumber,
            angle_radians,
        } => {
            row(0, "Depth", depth, 0.0);
            row(1, "Frequency", frequency_hz, 0.0);
            row(2, "Phase", phase_radians, -1.0e6);
            row(3, "Wavenumber", wavenumber, 0.0);
            row(4, "Direction", angle_radians, -1.0e6);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn loss_channel_editor(
    ui: &mut egui::Ui,
    id: u64,
    base: u8,
    label: &str,
    what: &str,
    channel: &mut Option<LossChannel>,
    parameters: &[MaterialParameter],
    formulas: &mut FormulaEdits,
) {
    let mut enabled = channel.is_some();
    if ui
        .checkbox(&mut enabled, label)
        .on_hover_text(format!("A flux-rate loss that {what} in this skin"))
        .changed()
    {
        *channel = enabled.then(|| LossChannel {
            base_rate: ScalarField::constant(0.2),
            law: DampingLaw {
                rate: RateLaw::Constant,
                drive: TimeDrive::None,
            },
        });
        forget(formulas, id, base, 0..ROW_SLOTS);
    }
    let Some(channel) = channel else { return };
    field_row(
        ui,
        (id, base),
        "Rate",
        &mut channel.base_rate,
        parameters,
        0.0,
        formulas,
    );
    if channel.law.rate != RateLaw::Constant {
        ui.small("A field-dependent loss rate does not run yet; it is kept as authored.");
    }
    drive_editor(
        ui,
        id,
        base + 4,
        &mut channel.law.drive,
        parameters,
        &[],
        formulas,
    );
}

/// The effective law with every expression evaluated at the material frame's
/// origin: the numbers behind the Simplified view's named summary.
pub(super) fn numeric_law_summary(ui: &mut egui::Ui, material: &Material, physics: PhysicsModel) {
    let origin = MaterialCoordinates {
        x: 0.0,
        y: 0.0,
        r: 0.0,
        theta: 0.0,
    };
    match material_law_summary(material, physics, LawSummaryDetail::Numeric(origin)) {
        Ok(lines) if lines.is_empty() => {}
        Ok(lines) => {
            ui.small("At the frame origin:");
            for line in lines {
                ui.small(format!("{} = {}", line.subject, line.response));
            }
        }
        Err(error) => {
            ui.small(format!("Effective law unavailable: {error}"));
        }
    }
}

fn field_row(
    ui: &mut egui::Ui,
    key: (u64, u8),
    label: &str,
    field: &mut ScalarField,
    parameters: &[MaterialParameter],
    minimum: f64,
    formulas: &mut FormulaEdits,
) -> egui::Response {
    ui.scope(|ui| {
        material_scalar_editor(
            ui,
            key,
            label,
            field,
            parameters,
            minimum,
            formulas.edits,
            formulas.errors,
        );
    })
    .response
}

/// Drops any half-typed formula text under slots a structural change just
/// replaced, so a stale edit cannot be committed into the new slot.
fn forget(formulas: &mut FormulaEdits, id: u64, base: u8, slots: std::ops::Range<u8>) {
    for slot in slots {
        formulas.edits.remove(&(id, base + slot));
        formulas.errors.remove(&(id, base + slot));
    }
}

fn field_text(field: &ScalarField) -> String {
    match field {
        ScalarField::Constant(value) => format!("{value}"),
        ScalarField::Formula(formula) => formula.source().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Texts = BTreeMap<(u64, u8), String>;

    fn edits() -> (Texts, Texts) {
        (BTreeMap::new(), BTreeMap::new())
    }

    /// Opening Advanced shows `s₀` without touching `k₀`: rendering the
    /// editor with no input leaves the material exactly as it was.
    #[test]
    fn showing_the_reciprocal_stiffness_does_not_rewrite_the_stiffness() {
        let mut material = Scene::initial().materials[0].clone();
        material.stiffness = ScalarField::formula("2 + 0.5 * x").unwrap();
        let before = material.clone();
        let (mut edit, mut error) = edits();
        let context = egui::Context::default();
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            reciprocal_stiffness_editor(
                ui,
                &mut material,
                &mut FormulaEdits {
                    edits: &mut edit,
                    errors: &mut error,
                },
            );
            advanced_law_editor(
                ui,
                &mut material,
                PhysicsModel::Mechanical,
                &[],
                &mut FormulaEdits {
                    edits: &mut edit,
                    errors: &mut error,
                },
            );
        });
        assert_eq!(material, before);
        assert!(error.is_empty(), "{error:?}");
    }

    /// A Custom material - a law no preset writes, one slot the solver does
    /// not run, a loss channel - is shown in every skin and left intact.
    #[test]
    fn the_advanced_view_leaves_a_custom_material_intact_in_every_skin() {
        let mut material = Scene::initial().materials[0].clone();
        material.parameters.push(MaterialParameter {
            name: "q".into(),
            value: 1.5,
        });
        material.mass_law.field = FieldLaw::Polynomial {
            chi1: ScalarField::constant(0.3),
            chi2: ScalarField::formula("0.4 * q").unwrap(),
            amplitude_bound: Some(ScalarField::constant(2.0)),
        };
        material.mass_law.drive = TimeDrive::TravellingModulation {
            depth: ScalarField::constant(0.1),
            frequency_hz: ScalarField::formula("2 * q").unwrap(),
            phase_radians: ScalarField::constant(-0.4),
            wavenumber: ScalarField::constant(3.0),
            angle_radians: ScalarField::constant(0.2),
        };
        material.stiffness_law.field = FieldLaw::Saturable {
            chi: ScalarField::constant(6.0),
            saturation: ScalarField::formula("0.2 + 0.01 * x").unwrap(),
        };
        material.stiffness_law.alternate = Some(ScalarField::constant(3.0));
        material.magnetic_loss = Some(LossChannel {
            base_rate: ScalarField::constant(0.3),
            law: DampingLaw {
                rate: RateLaw::SaturableAbsorption {
                    saturation: ScalarField::constant(1.0),
                },
                drive: TimeDrive::ParametricPump {
                    depth: ScalarField::constant(0.2),
                    frequency_hz: ScalarField::constant(1.0),
                    phase_radians: ScalarField::constant(0.0),
                },
            },
        });
        let before = material.clone();
        for physics in [
            PhysicsModel::Mechanical,
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            },
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            },
        ] {
            let (mut edit, mut error) = edits();
            let context = egui::Context::default();
            for _ in 0..2 {
                let _ = context.run_ui(egui::RawInput::default(), |ui| {
                    advanced_law_editor(
                        ui,
                        &mut material,
                        physics,
                        &[("point source".to_owned(), 2.5)],
                        &mut FormulaEdits {
                            edits: &mut edit,
                            errors: &mut error,
                        },
                    );
                    numeric_law_summary(ui, &material, physics);
                });
            }
            assert_eq!(material, before, "{physics:?}");
            assert!(error.is_empty(), "{physics:?}: {error:?}");
        }
    }

    /// An edited `s₀` is written back as the reciprocal it names, with the
    /// reciprocal cancelled: `1/(3 + x)` stores `k₀ = 3 + x`.
    #[test]
    fn an_edited_reciprocal_stiffness_stores_its_reciprocal() {
        let s0 = ScalarField::formula("1 / (3 + x)").unwrap();
        let stiffness = s0.reciprocal().unwrap();
        assert_eq!(stiffness, ScalarField::formula("3 + x").unwrap());
    }

    #[test]
    fn only_executable_responses_are_classified_as_editable() {
        assert!(response_kind(&FieldLaw::Linear) == ResponseKind::Linear);
        let kerr = FieldLaw::Polynomial {
            chi1: ScalarField::constant(0.0),
            chi2: ScalarField::constant(0.8),
            amplitude_bound: None,
        };
        assert!(response_kind(&kerr) == ResponseKind::Kerr);
        let signed = FieldLaw::Polynomial {
            chi1: ScalarField::constant(0.3),
            chi2: ScalarField::constant(0.8),
            amplitude_bound: None,
        };
        assert!(response_kind(&signed) == ResponseKind::Unsupported);
    }
}
