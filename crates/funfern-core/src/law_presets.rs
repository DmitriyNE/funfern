//! Named points in the law catalogue that a user can apply to a material.
//!
//! A preset is a factory that writes material data, not a live binding: it
//! creates named parameters and wires the law slots to formulas referring to
//! them, and after that the material stands on its own. The editor recovers
//! which preset produced a material by matching its structure, so editing a
//! slot out from under one simply stops it matching and the material reads as
//! Custom.
//!
//! [`apply_law_preset`] and [`identify_law_preset`] are both written against
//! one constructor, so a preset cannot be applied in a shape its own matcher
//! would not recognise.
//!
//! Only laws that run are listed. The catalogue's field-driven rows, restoring
//! laws and driven loss channels are authored types with no solver behind them,
//! and section 11 of the material-laws plan asks for a preset behind an open
//! design gate to be unavailable rather than offered and refused - so every
//! material a user can author this way assembles.

use crate::material::{MaterialParameter, ScalarField};
use crate::material_law::{CoefficientLaw, TimeDrive};
use crate::{MAX_MATERIAL_PARAMETERS, Material, MaterialError};

/// Which constitutive row a preset writes.
///
/// Named for the row rather than the physical quantity because the quantity
/// depends on the skin: the mass row is the density in Mechanical and the
/// permittivity in the electromagnetic skins, and those are not the same thing.
/// The editor takes the label from the skin, so a pump stays a pump across a
/// physics change while its label follows the coefficient.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LawPresetRow {
    Mass,
    Stiffness,
    /// Both rows at once, which is what an impedance-preserving pair needs.
    Both,
}

/// One value a preset exposes, stored as a material parameter that its law
/// slots refer to by name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LawPresetVariable {
    /// The parameter name the preset prefers. A collision with a parameter the
    /// user already authored takes the next free suffix instead.
    pub parameter: &'static str,
    pub label: &'static str,
    pub default: f64,
    pub minimum: f64,
    pub maximum: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LawPresetShape {
    Linear,
    Switch,
    Pump,
    Crystal,
    Travelling,
    ConstantImpedancePump,
}

/// A named law from the catalogue, with the values it asks the user for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LawPreset {
    /// The catalogue ID, empty for the linear preset - which is the absence of
    /// a law rather than one of them.
    pub id: &'static str,
    pub name: &'static str,
    pub phenomenon: &'static str,
    pub row: LawPresetRow,
    pub variables: &'static [LawPresetVariable],
    shape: LawPresetShape,
}

const DEPTH: LawPresetVariable = LawPresetVariable {
    parameter: "depth",
    label: "Depth",
    default: 0.2,
    minimum: 0.0,
    maximum: 0.95,
};
const FREQUENCY: LawPresetVariable = LawPresetVariable {
    parameter: "pump_hz",
    label: "Frequency",
    default: 1.0,
    minimum: 0.0,
    maximum: 1.0e4,
};
const PHASE: LawPresetVariable = LawPresetVariable {
    parameter: "pump_phase",
    label: "Phase",
    default: 0.0,
    minimum: -std::f64::consts::TAU,
    maximum: std::f64::consts::TAU,
};
const EDGE: LawPresetVariable = LawPresetVariable {
    parameter: "edge",
    label: "Edge sharpness",
    default: 3.0,
    minimum: 0.0,
    maximum: 32.0,
};
const WAVENUMBER: LawPresetVariable = LawPresetVariable {
    parameter: "wavenumber",
    label: "Wavenumber",
    default: 2.0,
    minimum: 0.0,
    maximum: 1.0e3,
};
const ANGLE: LawPresetVariable = LawPresetVariable {
    parameter: "wave_angle",
    label: "Direction",
    default: 0.0,
    minimum: -std::f64::consts::TAU,
    maximum: std::f64::consts::TAU,
};
const TARGET: LawPresetVariable = LawPresetVariable {
    parameter: "switch_to",
    label: "Switched factor",
    default: 2.0,
    minimum: 1.0e-6,
    maximum: 1.0e3,
};

const PUMP_VARIABLES: &[LawPresetVariable] = &[DEPTH, FREQUENCY, PHASE];
const CRYSTAL_VARIABLES: &[LawPresetVariable] = &[DEPTH, FREQUENCY, PHASE, EDGE];
const TRAVELLING_VARIABLES: &[LawPresetVariable] = &[DEPTH, FREQUENCY, PHASE, WAVENUMBER, ANGLE];
const SWITCH_VARIABLES: &[LawPresetVariable] = &[TARGET];

const PRESETS: &[LawPreset] = &[
    LawPreset {
        id: "",
        name: "Linear",
        phenomenon: "a medium that does not move",
        row: LawPresetRow::Both,
        variables: &[],
        shape: LawPresetShape::Linear,
    },
    LawPreset {
        id: "M-T1",
        name: "Switchable medium",
        phenomenon: "temporal refraction and time reflection",
        row: LawPresetRow::Mass,
        variables: SWITCH_VARIABLES,
        shape: LawPresetShape::Switch,
    },
    LawPreset {
        id: "K-T1",
        name: "Switchable medium",
        phenomenon: "temporal refraction and time reflection",
        row: LawPresetRow::Stiffness,
        variables: SWITCH_VARIABLES,
        shape: LawPresetShape::Switch,
    },
    LawPreset {
        id: "M-T2",
        name: "Parametric pump",
        phenomenon: "amplification at twice a mode's frequency",
        row: LawPresetRow::Mass,
        variables: PUMP_VARIABLES,
        shape: LawPresetShape::Pump,
    },
    LawPreset {
        id: "K-T2",
        name: "Parametric pump",
        phenomenon: "amplification at twice a mode's frequency",
        row: LawPresetRow::Stiffness,
        variables: PUMP_VARIABLES,
        shape: LawPresetShape::Pump,
    },
    LawPreset {
        id: "M-T3",
        name: "Travelling modulation",
        phenomenon: "non-reciprocity and one-way bands",
        row: LawPresetRow::Mass,
        variables: TRAVELLING_VARIABLES,
        shape: LawPresetShape::Travelling,
    },
    LawPreset {
        id: "K-T3",
        name: "Travelling modulation",
        phenomenon: "non-reciprocity and one-way bands",
        row: LawPresetRow::Stiffness,
        variables: TRAVELLING_VARIABLES,
        shape: LawPresetShape::Travelling,
    },
    LawPreset {
        id: "M-T4",
        name: "Time crystal",
        phenomenon: "a train of temporal interfaces, sharper gaps than a sinusoid",
        row: LawPresetRow::Mass,
        variables: CRYSTAL_VARIABLES,
        shape: LawPresetShape::Crystal,
    },
    LawPreset {
        id: "K-T4",
        name: "Time crystal",
        phenomenon: "a train of temporal interfaces, sharper gaps than a sinusoid",
        row: LawPresetRow::Stiffness,
        variables: CRYSTAL_VARIABLES,
        shape: LawPresetShape::Crystal,
    },
    LawPreset {
        id: "M-T2/K-T2",
        name: "Reflectionless time interface",
        phenomenon: "the speed moves while the impedance holds still, so a temporal \
                     interface does not reflect",
        row: LawPresetRow::Both,
        variables: PUMP_VARIABLES,
        shape: LawPresetShape::ConstantImpedancePump,
    },
];

/// Every preset a material can be given, in the order the selector shows them.
pub fn law_presets() -> &'static [LawPreset] {
    PRESETS
}

/// Which preset produced a material's laws, and the parameter each of its
/// variables is bound to.
#[derive(Clone, Debug, PartialEq)]
pub struct LawPresetMatch {
    pub preset: &'static LawPreset,
    /// One parameter name per entry of `preset.variables`, in that order.
    pub parameters: Vec<String>,
}

/// Recovers the preset behind a material's laws, or `None` for a material
/// whose slots no preset would have written - which the editor reads as
/// Custom and leaves alone.
///
/// Matching is structural: the laws must be exactly what applying the preset
/// with those parameter names produces. A slot edited to a bare constant, or a
/// drive whose kind was changed by hand, stops matching, which is what makes a
/// preset a snapshot rather than a binding.
pub fn identify_law_preset(material: &Material) -> Option<LawPresetMatch> {
    // No preset reaches past the two constitutive rows, so a material carrying
    // a restoring law or a loss channel was not written by one - whatever its
    // rows look like. Calling that Linear would name a medium after the half of
    // it the selector happens to inspect.
    if !material.restoring.is_none()
        || material.electric_loss.is_some()
        || material.magnetic_loss.is_some()
    {
        return None;
    }
    for preset in PRESETS {
        let Some(names) = (0..preset.variables.len())
            .map(|index| preset.bound_parameter(material, index))
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let (mass, stiffness) = preset.laws(&names);
        if material.mass_law == mass && material.stiffness_law == stiffness {
            return Some(LawPresetMatch {
                preset,
                parameters: names,
            });
        }
    }
    None
}

/// Writes a preset's laws onto a material, creating the parameters it exposes.
///
/// Parameters the user authored are preserved; a name the preset wants that is
/// already taken gets the next free suffix. Re-applying the preset a material
/// already matches keeps the values the user has tuned, so the selector is not
/// a reset button.
pub fn apply_law_preset(
    preset: &'static LawPreset,
    material: &Material,
) -> Result<Material, MaterialError> {
    let outgoing = identify_law_preset(material);
    let existing = outgoing
        .as_ref()
        .filter(|found| found.preset == preset)
        .cloned();
    let mut applied = material.clone();
    if existing.is_none()
        && let Some(outgoing) = &outgoing
    {
        // A preset owns the parameters it created, so the one being replaced
        // takes its own with it. Leaving them behind orphans a value with no
        // law referring to it and, four presets later, exhausts the material's
        // parameter budget so the next choice is refused outright.
        //
        // The laws go first, so a parameter is judged against the material it
        // is leaving rather than the one it arrived in, and anything the user
        // pointed at from a base coefficient or another slot stays.
        applied.mass_law = CoefficientLaw::linear();
        applied.stiffness_law = CoefficientLaw::linear();
        let referenced = applied
            .parameter_names()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        applied.parameters.retain(|parameter| {
            !outgoing.parameters.contains(&parameter.name) || referenced.contains(&parameter.name)
        });
    }
    let mut names = Vec::with_capacity(preset.variables.len());
    for (index, variable) in preset.variables.iter().enumerate() {
        if let Some(found) = &existing {
            names.push(found.parameters[index].clone());
            continue;
        }
        let name = free_parameter_name(&applied, variable.parameter, &names);
        if applied.parameters.len() >= MAX_MATERIAL_PARAMETERS {
            return Err(MaterialError::ParameterLimit);
        }
        applied.parameters.push(MaterialParameter {
            name: name.clone(),
            value: variable.default,
        });
        names.push(name);
    }
    let (mass, stiffness) = preset.laws(&names);
    applied.mass_law = mass;
    applied.stiffness_law = stiffness;
    Ok(applied)
}

impl LawPreset {
    /// The laws this preset writes when its variables are bound to `names`.
    /// Both applying and matching go through here, so they cannot disagree.
    fn laws(&self, names: &[String]) -> (CoefficientLaw, CoefficientLaw) {
        let field = |index: usize| ScalarField::formula(&names[index]).expect("parameter name");
        let mut written = CoefficientLaw::linear();
        match self.shape {
            LawPresetShape::Linear => {}
            LawPresetShape::Switch => written.alternate = Some(field(0)),
            LawPresetShape::Pump | LawPresetShape::ConstantImpedancePump => {
                written.drive = TimeDrive::ParametricPump {
                    depth: field(0),
                    frequency_hz: field(1),
                    phase_radians: field(2),
                };
            }
            LawPresetShape::Crystal => {
                written.drive = TimeDrive::TimeCrystal {
                    depth: field(0),
                    frequency_hz: field(1),
                    phase_radians: field(2),
                    sharpness: field(3),
                };
            }
            LawPresetShape::Travelling => {
                written.drive = TimeDrive::TravellingModulation {
                    depth: field(0),
                    frequency_hz: field(1),
                    phase_radians: field(2),
                    wavenumber: field(3),
                    angle_radians: field(4),
                };
            }
        }
        let linear = CoefficientLaw::linear();
        match (self.row, self.shape) {
            (LawPresetRow::Mass, _) => (written, linear),
            (LawPresetRow::Stiffness, _) => (linear, written),
            // The impedance `sqrt(m K)` holds still only if one row divides by
            // what the other multiplies by; the speed `sqrt(K/m)` then carries
            // the whole modulation.
            (LawPresetRow::Both, LawPresetShape::ConstantImpedancePump) => {
                let mut reciprocal = written.clone();
                reciprocal.inverted = true;
                (written, reciprocal)
            }
            (LawPresetRow::Both, _) => (written.clone(), written),
        }
    }

    /// The parameter a slot of this preset refers to, when it refers to exactly
    /// one by name. Anything else - a bare constant, an expression - means the
    /// material was not written by this preset.
    fn bound_parameter(&self, material: &Material, index: usize) -> Option<String> {
        let law = match self.row {
            LawPresetRow::Stiffness => &material.stiffness_law,
            _ => &material.mass_law,
        };
        // Mirrors `laws` rather than reading a shared field list, so the two
        // cannot drift into binding different slots to the same variable.
        let slot = match (self.shape, &law.drive) {
            (LawPresetShape::Switch, _) => law.alternate.as_ref()?,
            (
                LawPresetShape::Pump | LawPresetShape::ConstantImpedancePump,
                TimeDrive::ParametricPump {
                    depth,
                    frequency_hz,
                    phase_radians,
                },
            ) => [depth, frequency_hz, phase_radians]
                .into_iter()
                .nth(index)?,
            (
                LawPresetShape::Crystal,
                TimeDrive::TimeCrystal {
                    depth,
                    frequency_hz,
                    phase_radians,
                    sharpness,
                },
            ) => [depth, frequency_hz, phase_radians, sharpness]
                .into_iter()
                .nth(index)?,
            (
                LawPresetShape::Travelling,
                TimeDrive::TravellingModulation {
                    depth,
                    frequency_hz,
                    phase_radians,
                    wavenumber,
                    angle_radians,
                },
            ) => [
                depth,
                frequency_hz,
                phase_radians,
                wavenumber,
                angle_radians,
            ]
            .into_iter()
            .nth(index)?,
            _ => return None,
        };
        let source = slot.source()?;
        material
            .parameters
            .iter()
            .any(|parameter| parameter.name == source)
            .then(|| source.to_owned())
    }
}

/// The preferred name, or the first free suffix of it. Names already taken by
/// the material, or claimed earlier in this same application, are skipped.
fn free_parameter_name(material: &Material, preferred: &str, claimed: &[String]) -> String {
    let taken = |name: &str| {
        material.parameters.iter().any(|entry| entry.name == name)
            || claimed.iter().any(|entry| entry == name)
    };
    if !taken(preferred) {
        return preferred.to_owned();
    }
    (2..)
        .map(|suffix| format!("{preferred}{suffix}"))
        .find(|candidate| !taken(candidate))
        .expect("an unbounded suffix sequence has a free name")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LawSummaryDetail, PhysicsModel, material_law_summary};

    fn preset(id: &str, name: &str) -> &'static LawPreset {
        PRESETS
            .iter()
            .find(|preset| preset.id == id && preset.name == name)
            .expect("catalogue entry")
    }

    /// Applying a preset and then recovering it is the whole contract: the
    /// selector shows a name only when the material's slots are exactly what
    /// that preset writes. Round-tripping every entry catches a preset whose
    /// matcher and factory disagree, which is the failure that would make the
    /// selector read Custom the instant a user picked something.
    #[test]
    fn every_preset_is_recovered_from_the_material_it_writes() {
        for entry in PRESETS {
            let applied = apply_law_preset(entry, &Material::default_medium()).unwrap();
            let found = identify_law_preset(&applied).expect(entry.name);
            assert_eq!(found.preset.id, entry.id, "{}", entry.name);
            assert_eq!(found.preset.name, entry.name);
            assert_eq!(found.parameters.len(), entry.variables.len());
            for (name, variable) in found.parameters.iter().zip(entry.variables) {
                let parameter = applied
                    .parameters
                    .iter()
                    .find(|parameter| parameter.name == *name)
                    .expect("the preset created its parameter");
                assert_eq!(parameter.value, variable.default);
            }
            assert!(applied.valid(), "{} wrote an invalid material", entry.name);
        }
    }

    /// A preset is a snapshot. Editing one of its slots by hand stops it
    /// matching, and the material reads as Custom rather than being quietly
    /// refitted to the preset it came from.
    #[test]
    fn a_slot_edited_by_hand_stops_matching() {
        let pump = preset("M-T2", "Parametric pump");
        let applied = apply_law_preset(pump, &Material::default_medium()).unwrap();
        assert!(identify_law_preset(&applied).is_some());

        let mut constant = applied.clone();
        let TimeDrive::ParametricPump { depth, .. } = &mut constant.mass_law.drive else {
            panic!("the pump preset writes a pump");
        };
        *depth = ScalarField::constant(0.2);
        assert_eq!(identify_law_preset(&constant), None);

        let mut moved = applied;
        moved.stiffness_law = moved.mass_law.clone();
        assert_eq!(identify_law_preset(&moved), None);
    }

    /// The impedance-preserving pair is the one preset that needs both rows,
    /// and the thing that makes it work is that one row divides by what the
    /// other multiplies by. Its effective-law text is where a user sees that,
    /// so this checks the two together.
    #[test]
    fn the_reflectionless_pair_divides_on_one_row() {
        let entry = preset("M-T2/K-T2", "Reflectionless time interface");
        let applied = apply_law_preset(entry, &Material::default_medium()).unwrap();
        assert!(!applied.mass_law.inverted);
        assert!(applied.stiffness_law.inverted);
        assert_eq!(applied.mass_law.drive, applied.stiffness_law.drive);

        let lines =
            material_law_summary(&applied, PhysicsModel::Mechanical, LawSummaryDetail::Named)
                .unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].response.starts_with("ρ₀ · "), "{lines:?}");
        assert!(lines[1].response.starts_with("k₀ / "), "{lines:?}");
    }

    /// Parameters the user authored survive, and a preset that wants a name
    /// already in use takes the next free one rather than overwriting the
    /// value behind it.
    #[test]
    fn a_preset_never_takes_a_parameter_the_user_already_authored() {
        let mut material = Material::default_medium();
        material.parameters.push(MaterialParameter {
            name: "depth".into(),
            value: 41.0,
        });
        let applied = apply_law_preset(preset("M-T2", "Parametric pump"), &material).unwrap();
        let authored = applied
            .parameters
            .iter()
            .find(|parameter| parameter.name == "depth")
            .unwrap();
        assert_eq!(authored.value, 41.0, "the user's parameter was overwritten");
        let found = identify_law_preset(&applied).unwrap();
        assert_eq!(found.parameters[0], "depth2");
    }

    /// Re-applying the preset a material already carries keeps the values the
    /// user has tuned. The selector is not a reset button, and it must not
    /// grow a second copy of every parameter either.
    #[test]
    fn reapplying_a_preset_keeps_its_tuned_values() {
        let entry = preset("M-T4", "Time crystal");
        let mut applied = apply_law_preset(entry, &Material::default_medium()).unwrap();
        for parameter in &mut applied.parameters {
            parameter.value = 0.37;
        }
        let again = apply_law_preset(entry, &applied).unwrap();
        assert_eq!(again.parameters, applied.parameters);
        assert_eq!(again.mass_law, applied.mass_law);
    }

    /// Only laws that run are offered. A preset behind an open design gate
    /// would let a user author a document that cannot assemble, which is the
    /// one thing the selector must not do.
    #[test]
    fn the_catalogue_offers_only_laws_that_execute() {
        for entry in PRESETS {
            let applied = apply_law_preset(entry, &Material::default_medium()).unwrap();
            assert!(
                applied.restoring.is_none()
                    && applied.electric_loss.is_none()
                    && applied.magnetic_loss.is_none(),
                "{} reaches a slot no path executes",
                entry.name
            );
            for law in [&applied.mass_law, &applied.stiffness_law] {
                assert!(
                    matches!(law.field, crate::FieldLaw::Linear),
                    "{} writes a field law, which assembly refuses",
                    entry.name
                );
            }
        }
    }

    /// Switching preset retires the parameters of the one being replaced.
    ///
    /// Reported from the running application: a pump applied, then Linear
    /// chosen, left `depth`, `pump_hz` and `pump_phase` behind with no law
    /// referring to them - and chaining presets reached seven of eight
    /// parameters, after which the next choice was refused and the selector
    /// appeared to do nothing.
    #[test]
    fn switching_preset_takes_the_old_parameters_with_it() {
        let pump = preset("M-T2", "Parametric pump");
        let pumped = apply_law_preset(pump, &Material::default_medium()).unwrap();
        let names = |material: &Material| {
            material
                .parameters
                .iter()
                .map(|parameter| parameter.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&pumped), ["depth", "pump_hz", "pump_phase"]);

        // Every onward choice ends with exactly its own parameters, whatever
        // it is replacing, so chaining presets cannot exhaust the budget.
        for (id, name, expected) in [
            ("", "Linear", &[][..]),
            (
                "M-T4",
                "Time crystal",
                &["depth", "pump_hz", "pump_phase", "edge"][..],
            ),
            (
                "K-T2",
                "Parametric pump",
                &["depth", "pump_hz", "pump_phase"][..],
            ),
            (
                "M-T3",
                "Travelling modulation",
                &["depth", "pump_hz", "pump_phase", "wavenumber", "wave_angle"][..],
            ),
        ] {
            let switched = apply_law_preset(preset(id, name), &pumped).unwrap();
            assert_eq!(names(&switched), expected, "{name}");
            assert!(switched.valid(), "{name} left an invalid material");
            if !expected.is_empty() {
                assert_eq!(identify_law_preset(&switched).unwrap().preset.name, name);
            }
        }

        // Chaining the whole catalogue never accumulates.
        let mut chained = Material::default_medium();
        for entry in PRESETS.iter().cycle().take(PRESETS.len() * 3) {
            chained = apply_law_preset(entry, &chained)
                .unwrap_or_else(|error| panic!("{}: {error}", entry.name));
            assert_eq!(
                chained.parameters.len(),
                entry.variables.len(),
                "{}",
                entry.name
            );
        }
    }

    /// A parameter the user pointed at from somewhere else is not the outgoing
    /// preset's to take, even when the preset created it.
    #[test]
    fn a_parameter_another_slot_uses_survives_the_switch() {
        let pump = preset("M-T2", "Parametric pump");
        let mut pumped = apply_law_preset(pump, &Material::default_medium()).unwrap();
        pumped.mass_density = ScalarField::formula("2 + depth").unwrap();
        let linear = apply_law_preset(preset("", "Linear"), &pumped).unwrap();
        assert_eq!(
            linear
                .parameters
                .iter()
                .map(|parameter| parameter.name.as_str())
                .collect::<Vec<_>>(),
            ["depth"],
            "the density still refers to it"
        );
        assert!(linear.valid());
    }

    /// A material carrying a law no preset writes is Custom, even when its
    /// constitutive rows are untouched. Reading it as Linear would name the
    /// medium after the half of it the selector looks at.
    #[test]
    fn a_law_outside_the_constitutive_rows_reads_as_custom() {
        let mut material = Material::default_medium();
        assert_eq!(
            identify_law_preset(&material).unwrap().preset.name,
            "Linear"
        );
        material.restoring = crate::RestoringLaw::KleinGordon {
            omega0: ScalarField::constant(2.0),
        };
        assert_eq!(identify_law_preset(&material), None);
    }

    /// A material with no room left for the parameters a preset needs is
    /// refused before anything is written, so a failed application leaves the
    /// material exactly as it was.
    #[test]
    fn a_full_material_refuses_a_preset_rather_than_half_writing_it() {
        let mut material = Material::default_medium();
        for index in 0..MAX_MATERIAL_PARAMETERS {
            material.parameters.push(MaterialParameter {
                name: format!("user{index}"),
                value: 1.0,
            });
        }
        assert_eq!(
            apply_law_preset(preset("M-T3", "Travelling modulation"), &material),
            Err(MaterialError::ParameterLimit)
        );
        assert_eq!(
            identify_law_preset(&material).unwrap().preset.name,
            "Linear"
        );
    }

    /// The one row in the matrix whose law does not multiply the coefficient
    /// it sits beside. The solver divides the complementary coefficient by the
    /// law's factor, and the mechanical adapter stores `k0` rather than its
    /// reciprocal, so a pump on that row raises the compliance and *lowers*
    /// the stiffness. The editor names it reciprocal stiffness for this
    /// reason; this measures the sign rather than restating it.
    #[test]
    fn a_pump_on_the_stiffness_row_lowers_the_mechanical_stiffness() {
        use crate::{
            MaterialFrame, Point2, Region, RegionId,
            canonical_temporal::CanonicalMaterialRuntimeState,
            wave::evaluate_timed_directional_material_library_at,
        };
        let mut material = apply_law_preset(
            preset("K-T2", "Parametric pump"),
            &Material::default_medium(),
        )
        .unwrap();
        // A still pump at zero phase sits at its crest, so the factor is
        // exactly `1 + depth` and the comparison needs no tolerance argument.
        for parameter in &mut material.parameters {
            parameter.value = match parameter.name.as_str() {
                "depth" => 0.5,
                _ => 0.0,
            };
        }
        let region = Region {
            id: RegionId(1),
            material: material.id,
            frame: MaterialFrame::world(),
        };
        let materials = [material.clone()];
        let runtime = CanonicalMaterialRuntimeState::authored(materials.iter().cloned()).unwrap();
        let coefficients = evaluate_timed_directional_material_library_at(
            PhysicsModel::Mechanical,
            &materials,
            &[region],
            region.id,
            Point2::new(0.0, 0.0),
            0.0,
            &runtime,
        )
        .unwrap();
        let authored = coefficients.authored.stiffness.xx;
        let instantaneous = coefficients.instantaneous.stiffness.xx;
        assert!(
            (instantaneous - authored / 1.5).abs() < 1.0e-12,
            "a crest of `1 + 0.5` should divide the stiffness: {instantaneous} against {authored}"
        );
        assert!(
            instantaneous < authored,
            "the row a pump raises is the compliance, so the stiffness falls"
        );
        // The mass row has the opposite sense, which is why only one of the two
        // is renamed.
        assert_eq!(
            coefficients.instantaneous.mass_density,
            coefficients.authored.mass_density
        );
    }

    /// A preset's laws are the ones a physics skin change can carry, so
    /// switching skins in the preset view keeps working rather than reporting
    /// a material the user cannot see into.
    #[test]
    fn every_preset_survives_a_skin_change() {
        use crate::ElectromagneticPolarization;
        let mechanical = PhysicsModel::Mechanical;
        let tm = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        };
        for entry in PRESETS {
            let applied = apply_law_preset(entry, &Material::default_medium()).unwrap();
            let converted = mechanical
                .convert_material(tm, &applied)
                .unwrap_or_else(|error| panic!("{}: {error}", entry.name));
            assert_eq!(
                tm.convert_material(mechanical, &converted).unwrap(),
                applied,
                "{} did not survive a round trip",
                entry.name
            );
        }
    }
}
