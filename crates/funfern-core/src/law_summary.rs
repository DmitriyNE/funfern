//! The effective-law text: what a material's authored laws actually compose to.
//!
//! One line per coefficient a law modifies, and nothing for a coefficient left
//! alone. It lives here rather than in the editor because it has to stay true
//! to what the solver evaluates: every match below is exhaustive, so a new
//! entry in the law catalogue cannot be added without saying what it reads as.
//!
//! Two registers. [`LawSummaryDetail::Named`] prints the authored expressions,
//! so a preset's parameters appear under the names it gave them.
//! [`LawSummaryDetail::Numeric`] evaluates them at one point, which is what the
//! advanced view shows once the user is editing the slots themselves.

use crate::material::{MaterialCoordinates, MaterialParameter, ScalarField};
use crate::material_law::{CoefficientLaw, FieldLaw, RestoringLaw, TimeDrive};
use crate::{ElectromagneticPolarization, Material, MaterialError, PhysicsModel};

/// Which register the effective-law text is written in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LawSummaryDetail {
    /// Authored expressions, so a preset's parameters appear by name.
    Named,
    /// Expressions evaluated at this point in the material frame.
    Numeric(MaterialCoordinates),
}

/// One composed coefficient, as `subject = response`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LawSummaryLine {
    /// The quantity this line defines, named for the physics skin.
    pub subject: &'static str,
    /// Everything to the right of the equals sign.
    pub response: String,
}

/// How one skin names the quantities a law can reach.
///
/// The field arguments are the physical fields the constitutive rows relate,
/// not the solver's primary scalar. In TE the electric row still reads the
/// electric field, which is the vector there, so it must not be written as a
/// law of the scalar `H_z`.
struct SkinNames {
    mass: &'static str,
    mass_base: &'static str,
    mass_field: &'static str,
    stiffness: &'static str,
    stiffness_base: &'static str,
    stiffness_field: &'static str,
    /// The authored mass of the primary field, which weighs the restoring
    /// force: `ρ₀`, `ε₀` in TM, `μ₀` in TE.
    primary_base: &'static str,
}

const fn skin_names(physics: PhysicsModel) -> SkinNames {
    match physics {
        PhysicsModel::Mechanical => SkinNames {
            mass: "ρ",
            mass_base: "ρ₀",
            mass_field: "u",
            // The row's law multiplies the reciprocal stiffness, so it is
            // written as that: `s₀ · h` is a stiffness divided by `h`.
            stiffness: "s",
            stiffness_base: "s₀",
            stiffness_field: "e",
            primary_base: "ρ₀",
        },
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        } => SkinNames {
            mass: "ε",
            mass_base: "ε₀",
            mass_field: "E_z",
            stiffness: "μ",
            stiffness_base: "μ₀",
            stiffness_field: "H",
            primary_base: "ε₀",
        },
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        } => SkinNames {
            mass: "ε",
            mass_base: "ε₀",
            mass_field: "E",
            stiffness: "μ",
            stiffness_base: "μ₀",
            stiffness_field: "H_z",
            primary_base: "μ₀",
        },
    }
}

/// The effective-law text for one material: the constitutive rows that carry a
/// law, and the restoring row when it is set.
///
/// Loss channels are not here yet. Their slot is authored but no path executes
/// a driven one, and naming the two channels in the mechanical skin is a
/// question the advanced view has to settle rather than this function.
pub fn material_law_summary(
    material: &Material,
    physics: PhysicsModel,
    detail: LawSummaryDetail,
) -> Result<Vec<LawSummaryLine>, MaterialError> {
    let mut lines = Vec::new();
    for row in [crate::LawPresetRow::Mass, crate::LawPresetRow::Stiffness] {
        if let Some(line) = row_law_summary(material, physics, row, detail)? {
            lines.push(line);
        }
    }
    if let Some(line) = restoring_law_summary(material, physics, detail)? {
        lines.push(line);
    }
    Ok(lines)
}

/// One constitutive row's line, or `None` when no law modifies it. `Both`
/// names no single row and reads as `None`.
pub fn row_law_summary(
    material: &Material,
    physics: PhysicsModel,
    row: crate::LawPresetRow,
    detail: LawSummaryDetail,
) -> Result<Option<LawSummaryLine>, MaterialError> {
    let names = skin_names(physics);
    let (law, subject, base, field) = match row {
        crate::LawPresetRow::Mass => (
            &material.mass_law,
            names.mass,
            names.mass_base,
            names.mass_field,
        ),
        crate::LawPresetRow::Stiffness => (
            &material.stiffness_law,
            names.stiffness,
            names.stiffness_base,
            names.stiffness_field,
        ),
        crate::LawPresetRow::Both => return Ok(None),
    };
    if law.is_linear() {
        return Ok(None);
    }
    Ok(Some(LawSummaryLine {
        subject,
        response: coefficient_response(law, base, field, &material.parameters, detail)?,
    }))
}

/// The restoring force's line, when one is authored: `R = m₀·V′(r)` on the
/// integrated field `r = ∫u dt`, weighed by the primary row's authored mass.
pub fn restoring_law_summary(
    material: &Material,
    physics: PhysicsModel,
    detail: LawSummaryDetail,
) -> Result<Option<LawSummaryLine>, MaterialError> {
    let names = skin_names(physics);
    Ok(
        restoring_response(&material.restoring, &material.parameters, detail)?.map(|slope| {
            LawSummaryLine {
                subject: "R",
                response: format!("{}·{slope}", names.primary_base),
            }
        }),
    )
}

/// `c₀ · g · h · s`, or `c₀ / (g · h · s)` when the law is inverted, because
/// `inverted` divides by the whole multiplier rather than by one factor of it.
fn coefficient_response(
    law: &CoefficientLaw,
    base: &str,
    field: &str,
    parameters: &[MaterialParameter],
    detail: LawSummaryDetail,
) -> Result<String, MaterialError> {
    let mut factors = Vec::new();
    if let Some(text) = field_multiplier(&law.field, field, parameters, detail)? {
        factors.push(text);
    }
    if let Some(text) = drive_multiplier(&law.drive, parameters, detail)? {
        factors.push(text);
    }
    if let Some(alternate) = &law.alternate {
        factors.push(format!(
            "(1 + blend·({} − 1))",
            scalar(alternate, parameters, detail)?
        ));
    }
    if factors.is_empty() {
        return Ok(base.to_owned());
    }
    let product = factors.join(" · ");
    if law.inverted {
        // A single factor already carries its own parentheses.
        if factors.len() == 1 {
            return Ok(format!("{base} / {product}"));
        }
        return Ok(format!("{base} / ({product})"));
    }
    Ok(format!("{base} · {product}"))
}

fn field_multiplier(
    law: &FieldLaw,
    field: &str,
    parameters: &[MaterialParameter],
    detail: LawSummaryDetail,
) -> Result<Option<String>, MaterialError> {
    Ok(match law {
        FieldLaw::Linear => None,
        // Kerr is the polynomial with `chi1 = 0` and the quadratic law is the
        // one with `chi2 = 0`, so an authored zero is a specialization rather
        // than a term worth printing. A formula that happens to evaluate to
        // zero is not: its name is what the preset gave the user to turn.
        FieldLaw::Polynomial { chi1, chi2, .. } => {
            let mut terms = vec!["1".to_owned()];
            if !is_authored_zero(chi1) {
                terms.push(format!("{}·{field}", scalar(chi1, parameters, detail)?));
            }
            if !is_authored_zero(chi2) {
                terms.push(format!("{}·{field}²", scalar(chi2, parameters, detail)?));
            }
            Some(format!("({})", terms.join(" + ")))
        }
        FieldLaw::Saturable { chi, saturation } => Some(format!(
            "(1 + {}·{field}² / (1 + ({field}/{})²))",
            scalar(chi, parameters, detail)?,
            scalar(saturation, parameters, detail)?
        )),
    })
}

fn drive_multiplier(
    drive: &TimeDrive,
    parameters: &[MaterialParameter],
    detail: LawSummaryDetail,
) -> Result<Option<String>, MaterialError> {
    Ok(match drive {
        TimeDrive::None => None,
        TimeDrive::ParametricPump {
            depth,
            frequency_hz,
            phase_radians,
        } => Some(format!(
            "(1 + {}·cos(2π·{}·t + {}))",
            scalar(depth, parameters, detail)?,
            scalar(frequency_hz, parameters, detail)?,
            scalar(phase_radians, parameters, detail)?
        )),
        TimeDrive::TimeCrystal {
            depth,
            frequency_hz,
            phase_radians,
            sharpness,
        } => {
            let sharpness = scalar(sharpness, parameters, detail)?;
            Some(format!(
                "(1 + {}·tanh({sharpness}·cos(2π·{}·t + {})) / tanh({sharpness}))",
                scalar(depth, parameters, detail)?,
                scalar(frequency_hz, parameters, detail)?,
                scalar(phase_radians, parameters, detail)?
            ))
        }
        TimeDrive::TravellingModulation {
            depth,
            frequency_hz,
            phase_radians,
            wavenumber,
            angle_radians,
        } => Some(format!(
            "(1 + {}·cos(2π·{}·t − {}·(x·cos {} + y·sin {}) + {}))",
            scalar(depth, parameters, detail)?,
            scalar(frequency_hz, parameters, detail)?,
            scalar(wavenumber, parameters, detail)?,
            scalar(angle_radians, parameters, detail)?,
            scalar(angle_radians, parameters, detail)?,
            scalar(phase_radians, parameters, detail)?
        )),
    })
}

fn restoring_response(
    law: &RestoringLaw,
    parameters: &[MaterialParameter],
    detail: LawSummaryDetail,
) -> Result<Option<String>, MaterialError> {
    Ok(match law {
        RestoringLaw::None => None,
        RestoringLaw::KleinGordon { omega0 } => {
            Some(format!("{}²·r", scalar(omega0, parameters, detail)?))
        }
        RestoringLaw::SineGordon { omega0 } => {
            Some(format!("{}²·sin r", scalar(omega0, parameters, detail)?))
        }
        RestoringLaw::Phi4 { lambda, .. } => {
            Some(format!("{}·(r³ − r)", scalar(lambda, parameters, detail)?))
        }
    })
}

fn is_authored_zero(field: &ScalarField) -> bool {
    field.constant_value() == Some(0.0)
}

fn scalar(
    field: &ScalarField,
    parameters: &[MaterialParameter],
    detail: LawSummaryDetail,
) -> Result<String, MaterialError> {
    match detail {
        LawSummaryDetail::Named => Ok(match field {
            ScalarField::Constant(value) => number(*value),
            ScalarField::Formula(formula) => formula.source().to_owned(),
        }),
        LawSummaryDetail::Numeric(coordinates) => {
            field.evaluate(coordinates, parameters).map(number)
        }
    }
}

/// Short enough to read in a line of a panel, and without a trailing `.0` on a
/// whole number.
fn number(value: f64) -> String {
    let text = format!("{value:.4}");
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" {
        return "0".to_owned();
    }
    trimmed.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MaterialParameter, Scene};

    fn material() -> Material {
        Scene::default().materials[0].clone()
    }

    fn named(material: &Material, physics: PhysicsModel) -> Vec<LawSummaryLine> {
        material_law_summary(material, physics, LawSummaryDetail::Named).unwrap()
    }

    /// A coefficient nobody has touched gets no line. The panel that shows
    /// this has room for the laws that exist, not for restating the base
    /// values the editor above it already carries.
    #[test]
    fn an_untouched_material_composes_to_nothing() {
        assert!(named(&material(), PhysicsModel::Mechanical).is_empty());
    }

    /// Every named law in the catalogue reads back, on the row that carries
    /// it. The matches in this module are exhaustive, so a new entry cannot be
    /// added without a description; this checks the descriptions are the ones
    /// the solver evaluates rather than merely present.
    #[test]
    fn every_catalogue_law_reads_back_on_its_own_row() {
        type Case = (&'static str, fn(&mut Material), &'static str);
        let cases: [Case; 9] = [
            (
                "M-T1",
                |material| material.mass_law.alternate = Some(ScalarField::constant(2.0)),
                "ρ₀ · (1 + blend·(2 − 1))",
            ),
            (
                "M-T2",
                |material| {
                    material.mass_law.drive = TimeDrive::ParametricPump {
                        depth: ScalarField::constant(0.2),
                        frequency_hz: ScalarField::constant(1.5),
                        phase_radians: ScalarField::constant(0.0),
                    };
                },
                "ρ₀ · (1 + 0.2·cos(2π·1.5·t + 0))",
            ),
            (
                "M-T3",
                |material| {
                    material.mass_law.drive = TimeDrive::TravellingModulation {
                        depth: ScalarField::constant(0.1),
                        frequency_hz: ScalarField::constant(2.0),
                        phase_radians: ScalarField::constant(0.0),
                        wavenumber: ScalarField::constant(3.0),
                        angle_radians: ScalarField::constant(0.0),
                    };
                },
                "ρ₀ · (1 + 0.1·cos(2π·2·t − 3·(x·cos 0 + y·sin 0) + 0))",
            ),
            (
                "M-T4",
                |material| {
                    material.mass_law.drive = TimeDrive::TimeCrystal {
                        depth: ScalarField::constant(0.3),
                        frequency_hz: ScalarField::constant(1.0),
                        phase_radians: ScalarField::constant(0.0),
                        sharpness: ScalarField::constant(4.0),
                    };
                },
                "ρ₀ · (1 + 0.3·tanh(4·cos(2π·1·t + 0)) / tanh(4))",
            ),
            (
                "M-F1",
                |material| {
                    material.mass_law.field = FieldLaw::Polynomial {
                        chi1: ScalarField::constant(0.0),
                        chi2: ScalarField::constant(0.5),
                        amplitude_bound: None,
                    };
                },
                "ρ₀ · (1 + 0.5·u²)",
            ),
            (
                "M-F2",
                |material| {
                    material.mass_law.field = FieldLaw::Saturable {
                        chi: ScalarField::constant(0.4),
                        saturation: ScalarField::constant(2.0),
                    };
                },
                "ρ₀ · (1 + 0.4·u² / (1 + (u/2)²))",
            ),
            (
                "R1",
                |material| {
                    material.restoring = RestoringLaw::KleinGordon {
                        omega0: ScalarField::constant(3.0),
                    };
                },
                "ρ₀·3²·r",
            ),
            (
                "R2",
                |material| {
                    material.restoring = RestoringLaw::SineGordon {
                        omega0: ScalarField::constant(1.5),
                    };
                },
                "ρ₀·1.5²·sin r",
            ),
            (
                "R3",
                |material| {
                    material.restoring = RestoringLaw::Phi4 {
                        lambda: ScalarField::constant(0.75),
                        amplitude_bound: ScalarField::constant(2.0),
                    };
                },
                "ρ₀·0.75·(r³ − r)",
            ),
        ];
        for (id, apply, expected) in cases {
            let mut subject = material();
            apply(&mut subject);
            let lines = named(&subject, PhysicsModel::Mechanical);
            assert_eq!(lines.len(), 1, "{id} produced {lines:?}");
            assert_eq!(lines[0].response, expected, "{id}");
        }
        // Gate O: the force is on `r = ∫u dt`, weighed by each skin's own
        // primary mass.
        let mut subject = material();
        subject.restoring = RestoringLaw::SineGordon {
            omega0: ScalarField::constant(2.0),
        };
        for (physics, base) in [
            (
                PhysicsModel::Electromagnetic {
                    polarization: ElectromagneticPolarization::Tm,
                },
                "ε₀",
            ),
            (
                PhysicsModel::Electromagnetic {
                    polarization: ElectromagneticPolarization::Te,
                },
                "μ₀",
            ),
        ] {
            let lines = named(&subject, physics);
            assert_eq!(lines[0].subject, "R");
            assert_eq!(lines[0].response, format!("{base}·2²·sin r"));
        }
    }

    /// The same four drives compose on the stiffness row, which is what makes
    /// a reflectionless time interface expressible: one drive on each row with
    /// the multiplier inverted on one of them, so the impedance stays put
    /// while the speed moves.
    /// Each row reads alone, so the editor can put a row's law in that row's
    /// group; a linear row, and `Both`, read as nothing.
    #[test]
    fn each_row_reads_its_own_law() {
        let mut subject = material();
        subject.stiffness_law.drive = TimeDrive::ParametricPump {
            depth: ScalarField::constant(0.2),
            frequency_hz: ScalarField::constant(1.5),
            phase_radians: ScalarField::constant(0.0),
        };
        let row = |row| {
            row_law_summary(
                &subject,
                PhysicsModel::Mechanical,
                row,
                LawSummaryDetail::Named,
            )
            .unwrap()
        };
        assert_eq!(row(crate::LawPresetRow::Mass), None);
        assert_eq!(
            row(crate::LawPresetRow::Stiffness),
            Some(LawSummaryLine {
                subject: "s",
                response: "s₀ · (1 + 0.2·cos(2π·1.5·t + 0))".into(),
            })
        );
        assert_eq!(row(crate::LawPresetRow::Both), None);
    }

    #[test]
    fn a_constant_impedance_pair_reads_as_one_row_dividing() {
        let drive = || TimeDrive::ParametricPump {
            depth: ScalarField::constant(0.2),
            frequency_hz: ScalarField::constant(1.5),
            phase_radians: ScalarField::constant(0.0),
        };
        let mut subject = material();
        subject.mass_law.drive = drive();
        subject.stiffness_law.drive = drive();
        subject.stiffness_law.inverted = true;
        let lines = named(&subject, PhysicsModel::Mechanical);
        assert_eq!(
            lines,
            vec![
                LawSummaryLine {
                    subject: "ρ",
                    response: "ρ₀ · (1 + 0.2·cos(2π·1.5·t + 0))".into(),
                },
                LawSummaryLine {
                    subject: "s",
                    response: "s₀ / (1 + 0.2·cos(2π·1.5·t + 0))".into(),
                },
            ]
        );
    }

    /// In TE the roles swap, but the electric row still reads the electric
    /// field - which is the vector there. Writing it as a law of the scalar
    /// `H_z` would name the wrong physical field, and that is the one naming
    /// mistake this text is capable of making.
    #[test]
    fn the_electric_row_reads_the_electric_field_in_either_polarization() {
        let mut subject = material();
        subject.mass_law.field = FieldLaw::Polynomial {
            chi1: ScalarField::constant(0.0),
            chi2: ScalarField::constant(0.5),
            amplitude_bound: None,
        };
        for (polarization, argument) in [
            (ElectromagneticPolarization::Tm, "E_z"),
            (ElectromagneticPolarization::Te, "E"),
        ] {
            let lines = named(&subject, PhysicsModel::Electromagnetic { polarization });
            assert_eq!(lines[0].subject, "ε");
            assert_eq!(lines[0].response, format!("ε₀ · (1 + 0.5·{argument}²)"));
            assert!(!lines[0].response.contains("H_z"), "{polarization:?}");
        }
    }

    /// The two registers. A preset names its parameters and the text carries
    /// those names; the advanced view is editing the slots themselves, so it
    /// gets the numbers those names currently stand for.
    #[test]
    fn the_named_register_keeps_parameters_and_the_numeric_one_resolves_them() {
        let mut subject = material();
        subject.parameters.push(MaterialParameter {
            name: "depth".into(),
            value: 0.25,
        });
        subject.mass_law.drive = TimeDrive::ParametricPump {
            depth: ScalarField::formula("depth").unwrap(),
            frequency_hz: ScalarField::constant(1.0),
            phase_radians: ScalarField::constant(0.0),
        };
        assert_eq!(
            named(&subject, PhysicsModel::Mechanical)[0].response,
            "ρ₀ · (1 + depth·cos(2π·1·t + 0))"
        );
        let numeric = material_law_summary(
            &subject,
            PhysicsModel::Mechanical,
            LawSummaryDetail::Numeric(MaterialCoordinates {
                x: 0.0,
                y: 0.0,
                r: 0.0,
                theta: 0.0,
            }),
        )
        .unwrap();
        assert_eq!(numeric[0].response, "ρ₀ · (1 + 0.25·cos(2π·1·t + 0))");
    }
}
