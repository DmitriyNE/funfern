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

/// The law slots of one row: field response, drive, Switch alternate and
/// divide.
pub(super) fn law_slots_editor(
    ui: &mut egui::Ui,
    material: &mut Material,
    row: LawPresetRow,
    sources: &[(String, f64)],
    formulas: &mut FormulaEdits,
) {
    let id = material.id.0;
    let parameters = material.parameters.clone();
    let (base, law) = match row {
        LawPresetRow::Stiffness => (STIFFNESS_ROW, &mut material.stiffness_law),
        _ => (MASS_ROW, &mut material.mass_law),
    };
    coefficient_law_editor(ui, id, base, law, &parameters, sources, formulas);
}

/// Which named loss channel damps the field a row's coefficient belongs to.
/// In the EM skins the mass row is ε and the other is μ whatever the
/// polarization, so electric loss sits with ε and magnetic with μ. In
/// Mechanical the adapter maps magnetic loss onto the density row.
pub(super) fn row_is_electric(physics: PhysicsModel, row: LawPresetRow) -> bool {
    match physics {
        PhysicsModel::Mechanical => row == LawPresetRow::Stiffness,
        PhysicsModel::Electromagnetic { .. } => row == LawPresetRow::Mass,
    }
}

/// The row legacy `damping` acts on: the primary one, which is ε in TM, μ in
/// TE and the density in Mechanical.
pub(super) fn legacy_damping_row(physics: PhysicsModel) -> LawPresetRow {
    match physics {
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        } => LawPresetRow::Stiffness,
        _ => LawPresetRow::Mass,
    }
}

fn row_channel(
    material: &mut Material,
    physics: PhysicsModel,
    row: LawPresetRow,
) -> &mut Option<LossChannel> {
    if row_is_electric(physics, row) {
        &mut material.electric_loss
    } else {
        &mut material.magnetic_loss
    }
}

/// Moves a legacy `damping` into the named channel of the row it acts on, so
/// the two can never be authored together. Called only when a loss is edited;
/// opening a legacy material does not rewrite it.
pub(super) fn adopt_legacy_damping(material: &mut Material, physics: PhysicsModel) {
    if material.damping == ScalarField::constant(0.0) {
        return;
    }
    let rate = std::mem::replace(&mut material.damping, ScalarField::constant(0.0));
    let channel = row_channel(material, physics, legacy_damping_row(physics));
    match channel {
        Some(channel) => channel.base_rate = rate,
        None => {
            *channel = Some(LossChannel {
                base_rate: rate,
                law: DampingLaw {
                    rate: RateLaw::Constant,
                    drive: TimeDrive::None,
                },
            })
        }
    }
}

/// One row's loss rate. It shows the row's named channel, or a legacy
/// `damping` where that is what acts on this row, and any edit writes the
/// named channel. A rate edited back to zero with no drive removes the
/// channel, so an untouched material and one set back to zero are the same.
pub(super) fn loss_rate_editor(
    ui: &mut egui::Ui,
    material: &mut Material,
    physics: PhysicsModel,
    row: LawPresetRow,
    advanced: bool,
    formulas: &mut FormulaEdits,
) {
    let id = material.id.0;
    let base = if row == LawPresetRow::Stiffness {
        MAGNETIC_LOSS
    } else {
        ELECTRIC_LOSS
    };
    let legacy =
        row == legacy_damping_row(physics) && material.damping != ScalarField::constant(0.0);
    let shown = if legacy {
        material.damping.clone()
    } else {
        row_channel(material, physics, row)
            .as_ref()
            .map_or(ScalarField::constant(0.0), |channel| {
                channel.base_rate.clone()
            })
    };
    let mut edited = shown.clone();
    let parameters = material.parameters.clone();
    let label = if legacy {
        "Loss rate (legacy)"
    } else {
        "Loss rate"
    };
    field_row(
        ui,
        (id, base),
        label,
        &mut edited,
        &parameters,
        0.0,
        formulas,
    )
    .on_hover_text(loss_hover(physics, row, legacy));
    if edited != shown {
        adopt_legacy_damping(material, physics);
        let channel = row_channel(material, physics, row);
        match channel {
            Some(found) => found.base_rate = edited,
            None => {
                *channel = Some(LossChannel {
                    base_rate: edited,
                    law: DampingLaw {
                        rate: RateLaw::Constant,
                        drive: TimeDrive::None,
                    },
                })
            }
        }
        if let Some(found) = channel
            && found.base_rate == ScalarField::constant(0.0)
            && found.law.rate == RateLaw::Constant
            && found.law.drive == TimeDrive::None
        {
            *channel = None;
        }
    }
    let channel = row_channel(material, physics, row);
    let Some(found) = channel else { return };
    if found.law.rate != RateLaw::Constant {
        ui.small("A field-dependent loss rate does not run yet; it is kept as authored.");
    }
    if advanced {
        let before = found.law.drive.clone();
        let mut drive = before.clone();
        drive_editor(ui, id, base + 4, &mut drive, &parameters, &[], formulas);
        if drive != before {
            adopt_legacy_damping(material, physics);
            if let Some(found) = row_channel(material, physics, row) {
                found.law.drive = drive;
            }
        }
    }
}

fn loss_hover(physics: PhysicsModel, row: LawPresetRow, legacy: bool) -> String {
    let field = match (physics, row) {
        (PhysicsModel::Mechanical, LawPresetRow::Stiffness) => "the stress-like complementary flux",
        (PhysicsModel::Mechanical, _) => "the displacement flux",
        (
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            },
            LawPresetRow::Stiffness,
        ) => "the in-plane H",
        (
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            },
            _,
        ) => "E_z",
        (
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            },
            LawPresetRow::Stiffness,
        ) => "H_z",
        (
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            },
            _,
        ) => "the in-plane E",
    };
    let channel = if row_is_electric(physics, row) {
        "electric"
    } else {
        "magnetic"
    };
    if legacy {
        format!(
            "A rate per second that damps {field}. This material still carries the older \
             Damping σ, which damps whichever field the skin makes primary; editing it \
             moves it to the {channel} loss, which stays on its physical field across a \
             skin change."
        )
    } else {
        format!("The {channel} loss: a rate per second that damps {field} in this skin.")
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

    /// Both rows as the panel draws them: each row's loss and, in Advanced,
    /// its law slots.
    fn rows(
        ui: &mut egui::Ui,
        material: &mut Material,
        physics: PhysicsModel,
        advanced: bool,
        sources: &[(String, f64)],
        formulas: &mut FormulaEdits,
    ) {
        for row in [LawPresetRow::Mass, LawPresetRow::Stiffness] {
            loss_rate_editor(ui, material, physics, row, advanced, formulas);
            if advanced {
                law_slots_editor(ui, material, row, sources, formulas);
            }
        }
    }

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
            rows(
                ui,
                &mut material,
                PhysicsModel::Mechanical,
                true,
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
                    rows(
                        ui,
                        &mut material,
                        physics,
                        true,
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

    const SKINS: [PhysicsModel; 3] = [
        PhysicsModel::Mechanical,
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        },
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        },
    ];

    /// A legacy material is shown, in either view and every skin, without
    /// being rewritten: its damping reads as the primary row's loss.
    #[test]
    fn viewing_a_legacy_damping_leaves_it_where_it_is() {
        let mut material = Scene::initial().materials[0].clone();
        material.damping = ScalarField::constant(0.3);
        let before = material.clone();
        for physics in SKINS {
            for advanced in [false, true] {
                let (mut edit, mut error) = edits();
                let context = egui::Context::default();
                for _ in 0..2 {
                    let _ = context.run_ui(egui::RawInput::default(), |ui| {
                        rows(
                            ui,
                            &mut material,
                            physics,
                            advanced,
                            &[],
                            &mut FormulaEdits {
                                edits: &mut edit,
                                errors: &mut error,
                            },
                        );
                    });
                }
                assert_eq!(material, before, "{physics:?}, advanced {advanced}");
            }
        }
    }

    /// The electric channel sits with ε and the magnetic with μ in both EM
    /// polarizations; Mechanical puts magnetic on the density. Legacy damping
    /// belongs to the primary row, and adopting it lands in that row's channel.
    #[test]
    fn each_row_owns_the_loss_channel_of_its_field() {
        use LawPresetRow::{Mass, Stiffness};
        assert!(!row_is_electric(SKINS[0], Mass) && row_is_electric(SKINS[0], Stiffness));
        for physics in &SKINS[1..] {
            assert!(row_is_electric(*physics, Mass) && !row_is_electric(*physics, Stiffness));
        }
        assert_eq!(
            SKINS.map(legacy_damping_row),
            [Mass, Mass, Stiffness],
            "the primary row: density, ε for TM, μ for TE"
        );
        for (physics, electric) in [(SKINS[0], false), (SKINS[1], true), (SKINS[2], false)] {
            let mut material = Scene::initial().materials[0].clone();
            material.damping = ScalarField::constant(0.3);
            adopt_legacy_damping(&mut material, physics);
            assert_eq!(material.damping, ScalarField::constant(0.0));
            let (moved, other) = if electric {
                (&material.electric_loss, &material.magnetic_loss)
            } else {
                (&material.magnetic_loss, &material.electric_loss)
            };
            assert_eq!(
                moved.as_ref().map(|channel| channel.base_rate.clone()),
                Some(ScalarField::constant(0.3)),
                "{physics:?}"
            );
            assert!(other.is_none(), "{physics:?}");
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
