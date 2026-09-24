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
//! Only laws that run are listed. Kerr and saturable response (M-F1, M-F2)
//! run on the device since Stage 9, self-focusing only: a defocusing law needs
//! an authored amplitude bound, which a preset slider cannot promise to keep
//! valid. Signed χ₁ is an authored type with no solver behind it, and section
//! 11 of the material-laws plan asks for a preset behind an open design gate
//! to be unavailable rather than offered and refused - so every material a
//! user can author this way assembles.
//!
//! Restoring laws (R1-R3, Gate O) are a second family beside the response
//! presets, because they are not a coefficient: they add a force on the
//! integrated field `r = ∫u dt`, and a material can carry one beside any
//! response - sine-Gordon in a Kerr medium is one material. Their names say
//! what `r` is in each skin ([`restoring_preset_text`]).
//!
//! The editor's simple view offers whole media instead ([`medium_presets`]):
//! each response preset alone, and the Gate O examples on linear rows - the
//! three restoring laws, a self-oscillating medium and a lattice of van der
//! Pol oscillators. A material is either one of those or linear there; any
//! other composition is authored in Advanced.

use crate::material::{MaterialParameter, ScalarField};
use crate::material_law::{
    CoefficientLaw, DampingLaw, FieldLaw, LossChannel, RateLaw, RestoringLaw, TimeDrive,
};
use crate::{
    ElectromagneticPolarization, MAX_MATERIAL_PARAMETERS, Material, MaterialError, PhysicsModel,
};

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
    Kerr,
    Saturable,
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

const KERR_CHI: LawPresetVariable = LawPresetVariable {
    parameter: "kerr_chi",
    label: "Nonlinearity χ",
    default: 0.8,
    minimum: 0.0,
    maximum: 1.0e3,
};
const SATURATION: LawPresetVariable = LawPresetVariable {
    parameter: "saturation",
    label: "Saturation field",
    default: 1.0,
    minimum: 1.0e-6,
    maximum: 1.0e3,
};

const KERR_VARIABLES: &[LawPresetVariable] = &[KERR_CHI];
const SATURABLE_VARIABLES: &[LawPresetVariable] = &[KERR_CHI, SATURATION];
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
    LawPreset {
        id: "M-F1",
        name: "Kerr medium",
        phenomenon: "the wave slows where it is strong: self-focusing and self-phase modulation",
        row: LawPresetRow::Mass,
        variables: KERR_VARIABLES,
        shape: LawPresetShape::Kerr,
    },
    LawPreset {
        id: "M-F2",
        name: "Saturable medium",
        phenomenon: "Kerr that levels off, so a focusing beam narrows without collapsing",
        row: LawPresetRow::Mass,
        variables: SATURABLE_VARIABLES,
        shape: LawPresetShape::Saturable,
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
    // A response preset names the two constitutive rows and nothing else.
    // Each row's loss has its own editor beside it since Stage 10, and a
    // restoring law has its own selector since Gate O, so neither makes the
    // rows Custom: a lossy Kerr medium is a Kerr medium, and so is one that
    // also carries sine-Gordon.
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
        .map(|found| found.parameters.clone());
    rebind(
        material,
        existing,
        outgoing.map(|found| found.parameters),
        preset.variables,
        |material| {
            material.mass_law = CoefficientLaw::linear();
            material.stiffness_law = CoefficientLaw::linear();
        },
        |material, names| {
            let (mass, stiffness) = preset.laws(names);
            material.mass_law = mass;
            material.stiffness_law = stiffness;
        },
    )
}

/// The factory both preset families share. `existing` is the parameters the
/// material already binds to this same preset, which re-applying keeps with
/// their tuned values; `outgoing` is those of the preset being replaced,
/// which leave with it.
///
/// A preset owns the parameters it created, so the one being replaced takes
/// its own with it. Leaving them behind orphans a value with no law referring
/// to it and, four presets later, exhausts the material's parameter budget so
/// the next choice is refused outright. The laws are cleared first, so a
/// parameter is judged against the material it is leaving rather than the
/// one it arrived in, and anything the user pointed at from a base
/// coefficient or another slot stays.
fn rebind(
    material: &Material,
    existing: Option<Vec<String>>,
    outgoing: Option<Vec<String>>,
    variables: &[LawPresetVariable],
    clear: impl Fn(&mut Material),
    write: impl Fn(&mut Material, &[String]),
) -> Result<Material, MaterialError> {
    let mut applied = material.clone();
    if existing.is_none()
        && let Some(outgoing) = &outgoing
    {
        clear(&mut applied);
        let referenced = applied
            .parameter_names()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        applied.parameters.retain(|parameter| {
            !outgoing.contains(&parameter.name) || referenced.contains(&parameter.name)
        });
    }
    let mut names = Vec::with_capacity(variables.len());
    for (index, variable) in variables.iter().enumerate() {
        if let Some(found) = &existing {
            names.push(found[index].clone());
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
    write(&mut applied, &names);
    Ok(applied)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RestoringShape {
    None,
    KleinGordon,
    SineGordon,
    Phi4,
}

/// A restoring law from the catalogue's slot R (Gate O): a force `−m₀V′(r)`
/// on the integrated field `r = ∫u dt`, which the step carries as state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RestoringPreset {
    /// The catalogue ID, empty for the absence of a law.
    pub id: &'static str,
    /// The equation's own name; [`restoring_preset_text`] says what it is in
    /// a skin.
    pub name: &'static str,
    pub variables: &'static [LawPresetVariable],
    shape: RestoringShape,
}

const OMEGA0: LawPresetVariable = LawPresetVariable {
    parameter: "omega0",
    label: "Cutoff ω₀",
    default: 3.0,
    minimum: 0.0,
    maximum: 1.0e3,
};
const LAMBDA: LawPresetVariable = LawPresetVariable {
    parameter: "lambda",
    label: "Well depth λ",
    default: 16.0,
    minimum: 0.0,
    maximum: 1.0e4,
};
// The wells sit at ±1, so the field has to be allowed past them.
const PHI4_BOUND: LawPresetVariable = LawPresetVariable {
    parameter: "phi4_bound",
    label: "Amplitude bound",
    default: 1.6,
    minimum: 1.0,
    maximum: 100.0,
};

const RESTORING_PRESETS: &[RestoringPreset] = &[
    RestoringPreset {
        id: "",
        name: "None",
        variables: &[],
        shape: RestoringShape::None,
    },
    RestoringPreset {
        id: "R1",
        name: "Klein-Gordon",
        variables: &[OMEGA0],
        shape: RestoringShape::KleinGordon,
    },
    RestoringPreset {
        id: "R2",
        name: "sine-Gordon",
        variables: &[OMEGA0],
        shape: RestoringShape::SineGordon,
    },
    RestoringPreset {
        id: "R3",
        name: "φ⁴ double well",
        variables: &[LAMBDA, PHI4_BOUND],
        shape: RestoringShape::Phi4,
    },
];

/// Every restoring law a material can be given, in the order the selector
/// shows them.
pub fn restoring_presets() -> &'static [RestoringPreset] {
    RESTORING_PRESETS
}

#[derive(Clone, Debug, PartialEq)]
pub struct RestoringPresetMatch {
    pub preset: &'static RestoringPreset,
    /// One parameter name per entry of `preset.variables`, in that order.
    pub parameters: Vec<String>,
}

/// Recovers the restoring preset behind a material's slot R, or `None` for
/// one no preset writes (a constant `ω₀`, an expression), which the editor
/// reads as Custom.
pub fn identify_restoring_preset(material: &Material) -> Option<RestoringPresetMatch> {
    for preset in RESTORING_PRESETS {
        let Some(names) = (0..preset.variables.len())
            .map(|index| preset.bound_parameter(material, index))
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        if material.restoring == preset.law(&names) {
            return Some(RestoringPresetMatch {
                preset,
                parameters: names,
            });
        }
    }
    None
}

/// Writes a restoring preset onto a material, leaving its rows and losses as
/// they are.
pub fn apply_restoring_preset(
    preset: &'static RestoringPreset,
    material: &Material,
) -> Result<Material, MaterialError> {
    let outgoing = identify_restoring_preset(material);
    let existing = outgoing
        .as_ref()
        .filter(|found| found.preset == preset)
        .map(|found| found.parameters.clone());
    rebind(
        material,
        existing,
        outgoing.map(|found| found.parameters),
        preset.variables,
        |material| material.restoring = RestoringLaw::None,
        |material, names| material.restoring = preset.law(names),
    )
}

impl RestoringPreset {
    fn law(&self, names: &[String]) -> RestoringLaw {
        let field = |index: usize| ScalarField::formula(&names[index]).expect("parameter name");
        match self.shape {
            RestoringShape::None => RestoringLaw::None,
            RestoringShape::KleinGordon => RestoringLaw::KleinGordon { omega0: field(0) },
            RestoringShape::SineGordon => RestoringLaw::SineGordon { omega0: field(0) },
            RestoringShape::Phi4 => RestoringLaw::Phi4 {
                lambda: field(0),
                amplitude_bound: field(1),
            },
        }
    }

    fn bound_parameter(&self, material: &Material, index: usize) -> Option<String> {
        let slot = match (self.shape, &material.restoring) {
            (RestoringShape::KleinGordon, RestoringLaw::KleinGordon { omega0 })
            | (RestoringShape::SineGordon, RestoringLaw::SineGordon { omega0 })
                if index == 0 =>
            {
                omega0
            }
            (
                RestoringShape::Phi4,
                RestoringLaw::Phi4 {
                    lambda,
                    amplitude_bound,
                },
            ) => [lambda, amplitude_bound].into_iter().nth(index)?,
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

/// What a restoring preset is in one skin: the same equation throughout, named
/// for what its integrated field `r = ∫u dt` is there. Nothing is relabelled:
/// the displayed field stays `u`, and the text says so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoringPresetText {
    pub name: String,
    /// What the law does, in this skin's quantities.
    pub phenomenon: String,
    /// The equation and what `r` is, common to every law.
    pub equation: String,
}

/// The name, phenomenon and equation of a restoring preset in a skin, after
/// the table in `docs/spikes/funfern-gate-o.md`.
pub fn restoring_preset_text(
    preset: &RestoringPreset,
    physics: PhysicsModel,
) -> RestoringPresetText {
    let (u, r) = integrated_field_names(physics);
    let (name, phenomenon) = match (preset.shape, physics) {
        (RestoringShape::None, _) => ("None".to_owned(), "no restoring force".to_owned()),
        (RestoringShape::KleinGordon, PhysicsModel::Mechanical) => (
            "Klein-Gordon: a cutoff on the displacement".to_owned(),
            format!("waves below ω₀ do not propagate; {r} is pulled back to zero"),
        ),
        (
            RestoringShape::KleinGordon,
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            },
        ) => (
            "Klein-Gordon: cold plasma".to_owned(),
            format!(
                "a plasma cutoff at ω₀: waves below it do not propagate; {r} is the vector potential"
            ),
        ),
        (RestoringShape::KleinGordon, _) => (
            "Klein-Gordon: the dual plasma".to_owned(),
            format!("a cutoff at ω₀ on {u}: waves below it do not propagate"),
        ),
        (
            RestoringShape::SineGordon,
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            },
        ) => (
            "sine-Gordon: Josephson line".to_owned(),
            format!(
                "kinks in {r} are fluxons, each one step of 2π; {u} shows it as a voltage pulse"
            ),
        ),
        (RestoringShape::SineGordon, _) => (
            format!("sine-Gordon: kinks in {r}"),
            format!("kinks and breathers in {r}, each kink one step of 2π"),
        ),
        (RestoringShape::Phi4, _) => (
            format!("φ⁴: a double well in {r}"),
            format!("two vacua at {r} = ±1 with domain walls between; zero is the unstable top"),
        ),
    };
    let equation = if preset.shape == RestoringShape::None {
        String::new()
    } else {
        format!(
            "M₀r̈ + Kr + M₀V′(r) = 0 with r = {r}. The field shown is {u} = ṙ, so a static kink \
             shows {u} = 0; the Integrated field view shows r. ω₀ is the cutoff as authored: a \
             drive on the mass row moves it as ω₀√(m₀/m)."
        )
    };
    RestoringPresetText {
        name,
        phenomenon,
        equation,
    }
}

/// The skin's displayed field and its time integral.
fn integrated_field_names(physics: PhysicsModel) -> (&'static str, &'static str) {
    match physics {
        PhysicsModel::Mechanical => ("u", "∫u dt"),
        PhysicsModel::Electromagnetic { polarization } => match polarization {
            ElectromagneticPolarization::Tm => ("E_z", "−A_z"),
            ElectromagneticPolarization::Te => ("H_z", "∫H_z dt"),
        },
    }
}

/// The self-oscillating loss (catalogue D3) as a skin names it: gain below
/// its threshold, loss above, on the displayed field.
pub fn van_der_pol_text(physics: PhysicsModel) -> String {
    let (u, _) = integrated_field_names(physics);
    format!(
        "Self-oscillating (van der Pol): a rate γ₀(|{u}|²/a² − 1) that gives energy below the \
         threshold a and takes it above, so a small field grows and saturates; beside \
         Klein-Gordon it is a lattice of oscillators whose rate amplitude settles near 2a/√3. \
         Its energy is counted as active gain, of either sign, not as loss. It runs only beside \
         a linear response."
    )
}

const GAIN: LawPresetVariable = LawPresetVariable {
    parameter: "gain",
    label: "Gain rate γ₀",
    default: 0.5,
    minimum: 0.0,
    maximum: 1.0e3,
};
const THRESHOLD: LawPresetVariable = LawPresetVariable {
    parameter: "threshold",
    label: "Threshold a",
    default: 1.0,
    minimum: 1.0e-6,
    maximum: 1.0e3,
};
const SELF_OSCILLATION_VARIABLES: &[LawPresetVariable] = &[GAIN, THRESHOLD];
// Only a threshold past this is refused; a preset has no amplitude it could
// promise to stay under.
const SELF_OSCILLATION_BOUND: f64 = 1.0e3;

/// A whole medium from the catalogue: a response on the two rows, a restoring
/// law on the integrated field, and whether the primary loss self-oscillates
/// (catalogue D3). This is what the editor's simple view offers, where a
/// material is either linear or one of these examples; composing the parts by
/// hand is Advanced.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MediumPreset {
    response: usize,
    restoring: usize,
    pub self_oscillating: bool,
}

impl MediumPreset {
    pub fn response(&self) -> &'static LawPreset {
        &PRESETS[self.response]
    }

    pub fn restoring(&self) -> &'static RestoringPreset {
        &RESTORING_PRESETS[self.restoring]
    }

    /// Every value the medium asks for, response first, then the restoring
    /// law, then the self-oscillation.
    pub fn variables(&self) -> impl Iterator<Item = &'static LawPresetVariable> {
        let oscillation: &'static [LawPresetVariable] = if self.self_oscillating {
            SELF_OSCILLATION_VARIABLES
        } else {
            &[]
        };
        self.response()
            .variables
            .iter()
            .chain(self.restoring().variables)
            .chain(oscillation)
    }
}

const fn medium(response: usize, restoring: usize, self_oscillating: bool) -> MediumPreset {
    MediumPreset {
        response,
        restoring,
        self_oscillating,
    }
}

const MEDIA: &[MediumPreset] = &[
    medium(0, 0, false),
    medium(1, 0, false),
    medium(2, 0, false),
    medium(3, 0, false),
    medium(4, 0, false),
    medium(5, 0, false),
    medium(6, 0, false),
    medium(7, 0, false),
    medium(8, 0, false),
    medium(9, 0, false),
    medium(10, 0, false),
    medium(11, 0, false),
    // Gate O, on linear rows: the three restoring laws, a medium that
    // self-oscillates, and the lattice of van der Pol oscillators a
    // Klein-Gordon cutoff makes of it.
    medium(0, 1, false),
    medium(0, 2, false),
    medium(0, 3, false),
    medium(0, 0, true),
    medium(0, 1, true),
];

/// Every medium the simple view offers, in the order the selector shows them.
pub fn medium_presets() -> &'static [MediumPreset] {
    MEDIA
}

/// Which medium a material is, and the parameters each part is bound to.
#[derive(Clone, Debug, PartialEq)]
pub struct MediumPresetMatch {
    pub preset: &'static MediumPreset,
    pub response: LawPresetMatch,
    pub restoring: RestoringPresetMatch,
    /// The gain and threshold parameters of a self-oscillating medium.
    pub self_oscillation: Option<Vec<String>>,
}

impl MediumPresetMatch {
    /// One parameter name per entry of [`MediumPreset::variables`], in that
    /// order.
    pub fn parameters(&self) -> impl Iterator<Item = &String> {
        self.response
            .parameters
            .iter()
            .chain(&self.restoring.parameters)
            .chain(self.self_oscillation.iter().flatten())
    }
}

/// Recovers the medium a material is, or `None` for one the simple view could
/// not show whole: a law written by hand, a response beside a restoring law,
/// a driven or field-dependent loss. The editor reads that as Custom and
/// leaves it for Advanced. A constant loss on either row is part of any
/// medium, since the simple view edits it.
pub fn identify_medium_preset(
    material: &Material,
    physics: PhysicsModel,
) -> Option<MediumPresetMatch> {
    let response = identify_law_preset(material)?;
    let restoring = identify_restoring_preset(material)?;
    let self_oscillation = identify_self_oscillation(material, physics);
    let primary_electric = primary_loss_is_electric(physics);
    for (electric, channel) in [
        (true, &material.electric_loss),
        (false, &material.magnetic_loss),
    ] {
        let Some(channel) = channel else { continue };
        let oscillating = electric == primary_electric && self_oscillation.is_some();
        if !oscillating
            && (channel.law.rate != RateLaw::Constant || channel.law.drive != TimeDrive::None)
        {
            return None;
        }
    }
    let preset = MEDIA.iter().find(|entry| {
        entry.response() == response.preset
            && entry.restoring() == restoring.preset
            && entry.self_oscillating == self_oscillation.is_some()
    })?;
    Some(MediumPresetMatch {
        preset,
        response,
        restoring,
        self_oscillation,
    })
}

/// Writes a whole medium onto a material: its rows, its restoring law and its
/// primary loss kind. What the simple view cannot show goes, so the material
/// is then exactly that medium: a loss drive, a field-dependent loss rate, a
/// law another preset left. A constant loss rate stays, and so do values the
/// user tuned when a part is re-applied.
pub fn apply_medium_preset(
    preset: &'static MediumPreset,
    material: &Material,
    physics: PhysicsModel,
) -> Result<Material, MaterialError> {
    let mut applied = apply_law_preset(preset.response(), material)?;
    applied = apply_restoring_preset(preset.restoring(), &applied)?;
    applied = apply_self_oscillation(preset.self_oscillating, &applied, physics)?;
    let oscillating = preset
        .self_oscillating
        .then(|| primary_loss_is_electric(physics));
    for (electric, channel) in [
        (true, &mut applied.electric_loss),
        (false, &mut applied.magnetic_loss),
    ] {
        if oscillating == Some(electric) {
            continue;
        }
        if let Some(found) = channel {
            found.law = DampingLaw::constant();
            if found.base_rate == ScalarField::constant(0.0) {
                *channel = None;
            }
        }
    }
    Ok(applied)
}

/// Whether a skin's primary loss channel, the one on the displayed field, is
/// the electric one: in TM it is, and in TE and Mechanical it is the magnetic
/// one, which the Mechanical adapter puts on the density.
pub fn primary_loss_is_electric(physics: PhysicsModel) -> bool {
    matches!(
        physics,
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm
        }
    )
}

fn primary_loss(material: &Material, physics: PhysicsModel) -> &Option<LossChannel> {
    if primary_loss_is_electric(physics) {
        &material.electric_loss
    } else {
        &material.magnetic_loss
    }
}

fn primary_loss_mut(material: &mut Material, physics: PhysicsModel) -> &mut Option<LossChannel> {
    if primary_loss_is_electric(physics) {
        &mut material.electric_loss
    } else {
        &mut material.magnetic_loss
    }
}

/// The gain and threshold parameters of a primary loss the self-oscillation
/// preset wrote, or `None`.
fn identify_self_oscillation(material: &Material, physics: PhysicsModel) -> Option<Vec<String>> {
    let channel = primary_loss(material, physics).as_ref()?;
    let RateLaw::VanDerPol {
        threshold,
        amplitude_bound,
    } = &channel.law.rate
    else {
        return None;
    };
    if channel.law.drive != TimeDrive::None
        || *amplitude_bound != ScalarField::constant(SELF_OSCILLATION_BOUND)
    {
        return None;
    }
    [&channel.base_rate, threshold]
        .into_iter()
        .map(|slot| {
            let source = slot.source()?;
            material
                .parameters
                .iter()
                .any(|parameter| parameter.name == source)
                .then(|| source.to_owned())
        })
        .collect()
}

/// Makes the primary loss self-oscillating with bound parameters, or takes a
/// self-oscillating one away. Its rate is a gain, not a loss, so turning it
/// off removes the channel rather than leaving that rate to damp.
fn apply_self_oscillation(
    chosen: bool,
    material: &Material,
    physics: PhysicsModel,
) -> Result<Material, MaterialError> {
    let outgoing = identify_self_oscillation(material, physics);
    let existing = outgoing.clone().filter(|_| chosen);
    let variables: &[LawPresetVariable] = if chosen {
        SELF_OSCILLATION_VARIABLES
    } else {
        &[]
    };
    let drop_oscillation = |material: &mut Material| {
        let channel = primary_loss_mut(material, physics);
        if channel
            .as_ref()
            .is_some_and(|found| matches!(found.law.rate, RateLaw::VanDerPol { .. }))
        {
            *channel = None;
        }
    };
    rebind(
        material,
        existing,
        outgoing,
        variables,
        drop_oscillation,
        |material, names| {
            if !chosen {
                drop_oscillation(material);
                return;
            }
            let field = |index: usize| ScalarField::formula(&names[index]).expect("parameter name");
            // A legacy damping on the same field would sum with the gain.
            material.damping = ScalarField::constant(0.0);
            *primary_loss_mut(material, physics) = Some(LossChannel {
                base_rate: field(0),
                law: DampingLaw {
                    rate: RateLaw::VanDerPol {
                        threshold: field(1),
                        amplitude_bound: ScalarField::constant(SELF_OSCILLATION_BOUND),
                    },
                    drive: TimeDrive::None,
                },
            });
        },
    )
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
            LawPresetShape::Kerr => {
                written.field = FieldLaw::Polynomial {
                    chi1: ScalarField::constant(0.0),
                    chi2: field(0),
                    amplitude_bound: None,
                };
            }
            LawPresetShape::Saturable => {
                written.field = FieldLaw::Saturable {
                    chi: field(0),
                    saturation: field(1),
                };
            }
        }
        let linear = CoefficientLaw::linear();
        match (self.row, self.shape) {
            (LawPresetRow::Mass, _) => (written, linear),
            (LawPresetRow::Stiffness, _) => (linear, written),
            // The impedance `sqrt(m K)` holds still when the scalar mass is
            // multiplied by what the scalar stiffness is divided by. Every
            // stiffness-row law already divides `K`: it multiplies μ, which
            // `K = 1/μ` divides by, and in Mechanical it multiplies the
            // reciprocal stiffness s₀. So the same drive on both rows, neither
            // inverted, is the pair; the speed `sqrt(K/m)` then carries the
            // whole modulation. Inverting one row instead held the speed and
            // moved the impedance, the interface that reflects most.
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
            (LawPresetShape::Kerr, TimeDrive::None) => match &law.field {
                FieldLaw::Polynomial { chi2, .. } if index == 0 => chi2,
                _ => return None,
            },
            (LawPresetShape::Saturable, TimeDrive::None) => match &law.field {
                FieldLaw::Saturable { chi, saturation } => {
                    [chi, saturation].into_iter().nth(index)?
                }
                _ => return None,
            },
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

        // The same pump copied onto the other row is the reflectionless
        // pair, which is a preset of its own; an inverted copy is not.
        let mut copied = applied;
        copied.stiffness_law = copied.mass_law.clone();
        assert_eq!(
            identify_law_preset(&copied).map(|found| found.preset.name),
            Some("Reflectionless time interface")
        );
        copied.stiffness_law.inverted = true;
        assert_eq!(identify_law_preset(&copied), None);
    }

    /// The impedance-preserving pair, measured on the coefficients the solver
    /// steps with: at a pump crest `h = 1.5` the scalar mass and stiffness
    /// move oppositely in every skin, so `√(mK)` holds and the speed moves by
    /// the whole factor. Its effective-law text says the same thing.
    #[test]
    fn the_reflectionless_pair_holds_the_impedance_and_moves_the_speed() {
        use crate::{
            ElectromagneticPolarization, MaterialFrame, Point2, Region, RegionId,
            canonical_temporal::CanonicalMaterialRuntimeState,
            wave::evaluate_timed_directional_material_library_at,
        };
        let entry = preset("M-T2/K-T2", "Reflectionless time interface");
        let mut applied = apply_law_preset(entry, &Material::default_medium()).unwrap();
        assert!(!applied.mass_law.inverted && !applied.stiffness_law.inverted);
        assert_eq!(applied.mass_law.drive, applied.stiffness_law.drive);
        let lines =
            material_law_summary(&applied, PhysicsModel::Mechanical, LawSummaryDetail::Named)
                .unwrap();
        assert!(lines[0].response.starts_with("ρ₀ · "), "{lines:?}");
        assert!(lines[1].response.starts_with("s₀ · "), "{lines:?}");

        // A still pump at zero phase sits at its crest.
        for parameter in &mut applied.parameters {
            parameter.value = match parameter.name.as_str() {
                "depth" => 0.5,
                _ => 0.0,
            };
        }
        let region = Region {
            id: RegionId(1),
            material: applied.id,
            frame: MaterialFrame::world(),
        };
        let materials = [applied];
        let runtime = CanonicalMaterialRuntimeState::authored(materials.iter().cloned()).unwrap();
        for physics in [
            PhysicsModel::Mechanical,
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            },
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            },
        ] {
            let coefficients = evaluate_timed_directional_material_library_at(
                physics,
                &materials,
                &[region],
                region.id,
                Point2::default(),
                0.0,
                &runtime,
            )
            .unwrap();
            let mass = coefficients.instantaneous.mass_density / coefficients.authored.mass_density;
            let stiffness =
                coefficients.instantaneous.stiffness.xx / coefficients.authored.stiffness.xx;
            assert!(
                (mass * stiffness - 1.0).abs() < 1e-12,
                "{physics:?}: impedance moved"
            );
            assert!(
                ((stiffness / mass).sqrt() - 1.0).abs() > 0.3,
                "{physics:?}: the speed did not move"
            );
        }
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
                let values = law
                    .evaluate_at(
                        crate::MaterialCoordinates {
                            x: 0.0,
                            y: 0.0,
                            r: 0.0,
                            theta: 0.0,
                        },
                        &applied.parameters,
                    )
                    .unwrap();
                assert!(
                    values.field.executable(values.inverted).is_ok(),
                    "{} writes a field law the solver does not execute",
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

    /// A response preset names the rows only. A loss on a row and a restoring
    /// law each have their own editor, so neither turns a Kerr medium into
    /// Custom.
    #[test]
    fn a_loss_or_a_restoring_law_leaves_the_response_named() {
        let kerr =
            apply_law_preset(preset("M-F1", "Kerr medium"), &Material::default_medium()).unwrap();
        let mut material = apply_restoring_preset(restoring("R2"), &kerr).unwrap();
        material.magnetic_loss = Some(crate::LossChannel {
            base_rate: ScalarField::constant(0.2),
            law: crate::DampingLaw::constant(),
        });
        assert_eq!(
            identify_law_preset(&material).unwrap().preset.name,
            "Kerr medium"
        );
        assert_eq!(
            identify_restoring_preset(&material).unwrap().preset.name,
            "sine-Gordon"
        );
    }

    fn restoring(id: &str) -> &'static RestoringPreset {
        RESTORING_PRESETS
            .iter()
            .find(|preset| preset.id == id)
            .expect("restoring catalogue entry")
    }

    /// Every restoring preset is recovered from what it writes, with its
    /// defaults, and switching between them takes the old parameters along
    /// while a response preset's stay.
    #[test]
    fn every_restoring_preset_is_recovered_and_retires_its_parameters() {
        let pumped = apply_law_preset(
            preset("M-T2", "Parametric pump"),
            &Material::default_medium(),
        )
        .unwrap();
        for entry in RESTORING_PRESETS {
            let applied = apply_restoring_preset(entry, &pumped).unwrap();
            let found = identify_restoring_preset(&applied).expect(entry.name);
            assert_eq!(found.preset.id, entry.id);
            for (name, variable) in found.parameters.iter().zip(entry.variables) {
                let value = applied
                    .parameters
                    .iter()
                    .find(|parameter| parameter.name == *name)
                    .unwrap()
                    .value;
                assert_eq!(value, variable.default);
            }
            assert!(applied.valid(), "{}", entry.name);
            assert_eq!(
                identify_law_preset(&applied).unwrap().preset.name,
                "Parametric pump"
            );
        }
        // Chaining never accumulates, and the pump's three stay throughout.
        let mut chained = pumped;
        for entry in RESTORING_PRESETS
            .iter()
            .cycle()
            .take(RESTORING_PRESETS.len() * 3)
        {
            chained = apply_restoring_preset(entry, &chained).unwrap();
            assert_eq!(
                chained.parameters.len(),
                3 + entry.variables.len(),
                "{}",
                entry.name
            );
        }
        // Re-applying keeps a tuned value.
        let mut tuned =
            apply_restoring_preset(restoring("R1"), &Material::default_medium()).unwrap();
        tuned.parameters[0].value = 7.5;
        assert_eq!(
            apply_restoring_preset(restoring("R1"), &tuned).unwrap(),
            tuned
        );
        // A constant ω₀ written by hand is Custom.
        let mut custom = tuned;
        custom.restoring = crate::RestoringLaw::KleinGordon {
            omega0: ScalarField::constant(2.0),
        };
        assert_eq!(identify_restoring_preset(&custom), None);
    }

    fn skins() -> [PhysicsModel; 3] {
        use crate::ElectromagneticPolarization;
        [
            PhysicsModel::Mechanical,
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            },
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            },
        ]
    }

    /// The simple view's catalogue lists every response preset on its own
    /// once, and every medium comes back from what it writes in every skin,
    /// with its defaults.
    #[test]
    fn every_medium_is_recovered_in_every_skin() {
        for response in PRESETS {
            let alone = MEDIA
                .iter()
                .filter(|entry| {
                    entry.response() == response
                        && entry.restoring().id.is_empty()
                        && !entry.self_oscillating
                })
                .count();
            assert_eq!(alone, 1, "{}", response.name);
        }
        for physics in skins() {
            for entry in MEDIA {
                let applied =
                    apply_medium_preset(entry, &Material::default_medium(), physics).unwrap();
                assert!(applied.valid());
                let found = identify_medium_preset(&applied, physics).expect("recovered");
                assert_eq!(found.preset, entry);
                let names = found.parameters().collect::<Vec<_>>();
                assert_eq!(names.len(), entry.variables().count());
                assert_eq!(applied.parameters.len(), names.len());
                for (name, variable) in names.into_iter().zip(entry.variables()) {
                    let value = applied
                        .parameters
                        .iter()
                        .find(|parameter| parameter.name == *name)
                        .unwrap()
                        .value;
                    assert_eq!(value, variable.default);
                }
                assert_eq!(
                    primary_loss(&applied, physics).is_some(),
                    entry.self_oscillating
                );
            }
        }
    }

    /// Choosing a medium replaces the whole of the last one: nothing the
    /// simple view cannot show survives it, and cycling the catalogue never
    /// accumulates parameters. Re-applying keeps a tuned value.
    #[test]
    fn a_medium_replaces_the_one_before_it() {
        let physics = skins()[1];
        let mut material = Material::default_medium();
        for entry in MEDIA.iter().cycle().take(MEDIA.len() * 2) {
            material = apply_medium_preset(entry, &material, physics).unwrap();
            assert_eq!(material.parameters.len(), entry.variables().count());
            assert_eq!(
                identify_medium_preset(&material, physics).unwrap().preset,
                entry
            );
        }
        let lattice = MEDIA
            .iter()
            .find(|entry| entry.self_oscillating && entry.restoring().id == "R1")
            .unwrap();
        let mut tuned = apply_medium_preset(lattice, &material, physics).unwrap();
        for parameter in &mut tuned.parameters {
            parameter.value *= 1.5;
        }
        assert_eq!(
            apply_medium_preset(lattice, &tuned, physics).unwrap(),
            tuned
        );
        let linear = apply_medium_preset(&MEDIA[0], &tuned, physics).unwrap();
        assert_eq!(linear.restoring, RestoringLaw::None);
        assert_eq!(linear.electric_loss, None);
        assert!(linear.parameters.is_empty());
    }

    /// A material composed by hand is not a medium, and so reads Custom in
    /// the simple view: a response beside a restoring law, a driven loss, a
    /// self-oscillation with a constant threshold. A constant loss is part of
    /// any medium.
    #[test]
    fn a_composed_material_is_not_a_medium() {
        let physics = skins()[1];
        let kerr =
            apply_law_preset(preset("M-F1", "Kerr medium"), &Material::default_medium()).unwrap();
        let composed = apply_restoring_preset(restoring("R2"), &kerr).unwrap();
        assert_eq!(identify_medium_preset(&composed, physics), None);

        let mut lossy = kerr.clone();
        lossy.magnetic_loss = Some(LossChannel {
            base_rate: ScalarField::constant(0.2),
            law: DampingLaw::constant(),
        });
        assert!(identify_medium_preset(&lossy, physics).is_some());
        lossy.magnetic_loss.as_mut().unwrap().law.drive = TimeDrive::ParametricPump {
            depth: ScalarField::constant(0.1),
            frequency_hz: ScalarField::constant(1.0),
            phase_radians: ScalarField::constant(0.0),
        };
        assert_eq!(identify_medium_preset(&lossy, physics), None);
        // Applying a medium over it clears the drive and keeps the rate.
        let cleaned = apply_medium_preset(&MEDIA[0], &lossy, physics).unwrap();
        assert_eq!(
            cleaned.magnetic_loss,
            Some(LossChannel {
                base_rate: ScalarField::constant(0.2),
                law: DampingLaw::constant(),
            })
        );

        let mut constant = Material::default_medium();
        constant.electric_loss = Some(LossChannel {
            base_rate: ScalarField::constant(0.5),
            law: DampingLaw {
                rate: RateLaw::VanDerPol {
                    threshold: ScalarField::constant(1.0),
                    amplitude_bound: ScalarField::constant(SELF_OSCILLATION_BOUND),
                },
                drive: TimeDrive::None,
            },
        });
        assert_eq!(identify_medium_preset(&constant, physics), None);
        // A self-oscillation in TE sits on the magnetic channel, so the same
        // electric one is a field-dependent secondary loss there.
        assert_eq!(identify_medium_preset(&constant, skins()[2]), None);
    }

    /// Every skin names each law for what its integrated field is there, and
    /// every law's text carries the equation and what a static kink shows.
    #[test]
    fn each_skin_names_each_restoring_law_for_its_integrated_field() {
        use crate::ElectromagneticPolarization;
        let skins = [
            (PhysicsModel::Mechanical, "∫u dt"),
            (
                PhysicsModel::Electromagnetic {
                    polarization: ElectromagneticPolarization::Tm,
                },
                "−A_z",
            ),
            (
                PhysicsModel::Electromagnetic {
                    polarization: ElectromagneticPolarization::Te,
                },
                "∫H_z dt",
            ),
        ];
        for entry in RESTORING_PRESETS
            .iter()
            .filter(|entry| !entry.id.is_empty())
        {
            let mut names = Vec::new();
            for (physics, integrated) in skins {
                let text = restoring_preset_text(entry, physics);
                assert!(text.equation.contains("r = "), "{}", entry.name);
                assert!(
                    text.equation.contains(integrated),
                    "{}: {physics:?}",
                    entry.name
                );
                assert!(text.equation.contains("static kink"));
                names.push(text.name);
            }
            // The names differ where the physics differs; φ⁴ and the
            // Mechanical and TE sine-Gordon are the same equation named by
            // their own `r`.
            assert!(names.iter().any(|name| name != &names[0]) || entry.id == "R3");
        }
        assert!(van_der_pol_text(PhysicsModel::Mechanical).contains("active gain"));
    }

    /// A restoring preset survives a skin change and comes back.
    #[test]
    fn every_restoring_preset_survives_a_skin_change() {
        use crate::ElectromagneticPolarization;
        let mechanical = PhysicsModel::Mechanical;
        let tm = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        };
        for entry in RESTORING_PRESETS {
            let applied = apply_restoring_preset(entry, &Material::default_medium()).unwrap();
            let converted = mechanical.convert_material(tm, &applied).unwrap();
            assert_eq!(
                tm.convert_material(mechanical, &converted).unwrap(),
                applied
            );
            assert_eq!(
                identify_restoring_preset(&converted).unwrap().preset.id,
                entry.id
            );
        }
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
