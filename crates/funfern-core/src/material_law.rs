//! Authored material-law building blocks. These are presentation and document
//! data; compiling them into physical electric, magnetic, or mechanical maps is
//! a separate step, and unsupported blocks must not reach the legacy solver.
//!
//! `inverted` is an algebraic authoring representation, not by itself a proof
//! that a nonlinear law is portable between physics skins. The physical field
//! argument and scalar/vector placement are assigned explicitly below.

use crate::{
    ElectromagneticPolarization, Material, MaterialCoordinates, MaterialError, MaterialParameter,
    PhysicsModel, ScalarField,
};

// ---------------------------------------------------------------------------
// Authored forms
// ---------------------------------------------------------------------------

/// How one coefficient follows the field at the same node.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldLaw {
    Linear,
    /// `1 + chi1·u + chi2·u²`. Kerr is `chi1 = 0`, the quadratic law of
    /// nonlinear acoustics is `chi2 = 0`.
    Polynomial {
        chi1: ScalarField,
        chi2: ScalarField,
        /// The largest `|u|` the law is meant to see. Required whenever the
        /// multiplier is not monotone for every `u`, because the step is
        /// chosen from the tangent over this range.
        amplitude_bound: Option<ScalarField>,
    },
    /// `1 + chi·u² / (1 + u²/saturation²)`: Kerr that levels off, so a
    /// self-focusing beam narrows without collapsing.
    Saturable {
        chi: ScalarField,
        saturation: ScalarField,
    },
}

/// How one coefficient follows time. Each variant is named for what it does
/// to a wave; the form is in its comment.
#[derive(Clone, Debug, PartialEq)]
pub enum TimeDrive {
    None,
    /// `1 + depth·cos(2π f t + phase)`: parametric amplification when `f` is
    /// twice a mode's frequency.
    ParametricPump {
        depth: ScalarField,
        frequency_hz: ScalarField,
        phase_radians: ScalarField,
    },
    /// The smoothed square `1 + depth·tanh(sharpness·cos(2π f t + phase)) /
    /// tanh(sharpness)`: a train of temporal interfaces, each edge reflecting
    /// part of the wave backwards in time.
    TimeCrystal {
        depth: ScalarField,
        frequency_hz: ScalarField,
        phase_radians: ScalarField,
        sharpness: ScalarField,
    },
    /// `1 + depth·cos(2π f t − q·x + phase)`, with `q` of the given magnitude
    /// along the given direction in the material frame: a modulation that
    /// moves, so waves running with it and against it see different media.
    TravellingModulation {
        depth: ScalarField,
        frequency_hz: ScalarField,
        phase_radians: ScalarField,
        wavenumber: ScalarField,
        angle_radians: ScalarField,
    },
}

/// Everything one coefficient row (density, or stiffness) can carry.
#[derive(Clone, Debug, PartialEq)]
pub struct CoefficientLaw {
    pub field: FieldLaw,
    pub drive: TimeDrive,
    /// The factor the coefficient takes while the material is switched. The
    /// switch itself is runtime state; this is what it switches to.
    pub alternate: Option<ScalarField>,
    /// The whole multiplier divides the coefficient instead of multiplying it.
    pub inverted: bool,
}

/// How the damping *rate* follows the field. Damping is modulated as a rate,
/// not as a coefficient, because the rate is what survives a physics switch.
#[derive(Clone, Debug, PartialEq)]
pub enum RateLaw {
    Constant,
    /// `1 / (1 + u²/saturation²)`: loss that a strong field bleaches.
    SaturableAbsorption {
        saturation: ScalarField,
    },
    /// `1 + beta1·u + beta2·u²`.
    Polynomial {
        beta1: ScalarField,
        beta2: ScalarField,
        amplitude_bound: Option<ScalarField>,
    },
    /// Reserved active-law authoring data. It remains non-executable until the
    /// oscillator equation and its energy/gain contract close Gate O.
    VanDerPol {
        threshold: ScalarField,
        amplitude_bound: ScalarField,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct DampingLaw {
    pub rate: RateLaw,
    pub drive: TimeDrive,
}

/// One physically named flux-loss channel. `base_rate` has inverse-time units;
/// `law` modulates that rate by its own physical field and optional drive.
#[derive(Clone, Debug, PartialEq)]
pub struct LossChannel {
    pub base_rate: ScalarField,
    pub law: DampingLaw,
}

impl LossChannel {
    pub fn valid(&self, parameters: &[MaterialParameter]) -> bool {
        passes(&self.base_rate, parameters, |rate| rate >= 0.0) && self.law.valid(parameters)
    }

    pub fn parameter_names(&self) -> impl Iterator<Item = &str> {
        self.base_rate
            .parameter_names()
            .chain(self.law.parameter_names())
    }

    pub fn varies_in_space(&self) -> bool {
        !self.base_rate.spatially_constant() || self.law.varies_in_space()
    }

    pub fn uses_frame(&self) -> bool {
        self.varies_in_space() || self.law.uses_frame()
    }

    pub fn rename_parameter(&self, old: &str, new: &str) -> Result<Self, MaterialError> {
        Ok(Self {
            base_rate: self.base_rate.rename_parameter(old, new)?,
            law: self.law.rename_parameter(old, new)?,
        })
    }
}

/// Reserved oscillator authoring data. The formulas are candidate restoring
/// functions only; their physical state, energy and skin mapping remain behind
/// Gate O and are not interpreted by the legacy solver.
#[derive(Clone, Debug, PartialEq)]
pub enum RestoringLaw {
    None,
    /// `V′ = omega0²·u`: dispersion with a cutoff at `omega0`.
    KleinGordon {
        omega0: ScalarField,
    },
    /// `V′ = omega0²·sin(u)`: kinks, antikinks and breathers.
    SineGordon {
        omega0: ScalarField,
    },
    /// `V′ = lambda·(u³ − u)`: two wells at `u = ±1`, domain walls between.
    Phi4 {
        lambda: ScalarField,
        amplitude_bound: ScalarField,
    },
}

impl CoefficientLaw {
    pub const fn linear() -> Self {
        Self {
            field: FieldLaw::Linear,
            drive: TimeDrive::None,
            alternate: None,
            inverted: false,
        }
    }

    pub fn is_linear(&self) -> bool {
        matches!(self.field, FieldLaw::Linear) && self.drive.is_none() && self.alternate.is_none()
    }

    /// The same law seen from the reciprocal coefficient. What multiplied `k`
    /// divides `ε = 1/k`, so the flag flips and nothing else moves.
    pub fn through_reciprocal(&self) -> Self {
        if self.is_linear() {
            return Self::linear();
        }
        Self {
            inverted: !self.inverted,
            ..self.clone()
        }
    }

    /// Removes representation-only state from an identity law. This keeps a
    /// reciprocal round trip from manufacturing an active `÷1` law.
    pub fn normalized(&self) -> Self {
        if self.is_linear() {
            Self::linear()
        } else {
            self.clone()
        }
    }

    pub fn valid(&self, parameters: &[MaterialParameter]) -> bool {
        let alternate_valid = self
            .alternate
            .as_ref()
            .is_none_or(|factor| constant_passes(factor, parameters, |a| a > 0.0));
        alternate_valid
            && self.drive.valid(parameters)
            && self.field.valid(parameters, self.inverted)
    }

    pub fn parameter_names(&self) -> impl Iterator<Item = &str> {
        self.fields()
            .into_iter()
            .flat_map(ScalarField::parameter_names)
    }

    pub fn varies_in_space(&self) -> bool {
        self.fields()
            .iter()
            .any(|field| !field.spatially_constant())
    }

    pub fn uses_frame(&self) -> bool {
        self.varies_in_space() || self.drive.uses_frame()
    }

    pub fn rename_parameter(&self, old: &str, new: &str) -> Result<Self, MaterialError> {
        Ok(Self {
            field: self.field.rename_parameter(old, new)?,
            drive: self.drive.rename_parameter(old, new)?,
            alternate: self
                .alternate
                .as_ref()
                .map(|factor| factor.rename_parameter(old, new))
                .transpose()?,
            inverted: self.inverted,
        })
    }

    /// The law's numbers at one point of the material frame.
    pub fn evaluate_at(
        &self,
        coordinates: MaterialCoordinates,
        parameters: &[MaterialParameter],
    ) -> Result<CoefficientLawValues, MaterialError> {
        Ok(CoefficientLawValues {
            field: self.field.evaluate_at(coordinates, parameters)?,
            drive: self.drive.evaluate(parameters)?,
            alternate: self
                .alternate
                .as_ref()
                .map(|factor| factor.evaluate_constant(parameters))
                .transpose()?,
            inverted: self.inverted,
        })
    }

    /// The law's numbers when nothing in it varies in space, which is what
    /// the editor can know before assembly samples the node points.
    pub fn constant_values(
        &self,
        parameters: &[MaterialParameter],
    ) -> Result<Option<CoefficientLawValues>, MaterialError> {
        if self.varies_in_space() {
            return Ok(None);
        }
        self.evaluate_at(origin(), parameters).map(Some)
    }

    fn fields(&self) -> Vec<&ScalarField> {
        let mut fields = self.field.fields();
        fields.extend(self.drive.fields());
        fields.extend(self.alternate.as_ref());
        fields
    }
}

impl FieldLaw {
    pub fn valid(&self, parameters: &[MaterialParameter], inverted: bool) -> bool {
        match self {
            Self::Linear => true,
            Self::Polynomial {
                chi1,
                chi2,
                amplitude_bound,
            } => {
                let bound_valid = amplitude_bound
                    .as_ref()
                    .is_none_or(|bound| constant_passes(bound, parameters, |b| b > 0.0));
                // The turnover of `u / ḡ` is not a property of the numbers but
                // of the form, so the bound is required before any evaluation.
                let bound_present = !inverted || amplitude_bound.is_some();
                bound_valid
                    && bound_present
                    && each_finite(&[chi1, chi2], parameters)
                    && self.monotone_where_known(parameters, inverted)
            }
            Self::Saturable { chi, saturation } => {
                each_finite(&[chi], parameters)
                    && passes(saturation, parameters, |s| s > 0.0)
                    && self.monotone_where_known(parameters, inverted)
            }
        }
    }

    /// The pointwise solve needs one root; when the coefficients are known at
    /// edit time that is checked here, and when they vary in space it is
    /// checked at the node points during assembly instead.
    fn monotone_where_known(&self, parameters: &[MaterialParameter], inverted: bool) -> bool {
        if self
            .fields()
            .iter()
            .any(|field| !field.spatially_constant())
        {
            return true;
        }
        match self.evaluate_at(origin(), parameters) {
            Ok(values) => values.tangent_range(inverted).is_some(),
            Err(_) => false,
        }
    }

    pub fn evaluate_at(
        &self,
        coordinates: MaterialCoordinates,
        parameters: &[MaterialParameter],
    ) -> Result<FieldLawValues, MaterialError> {
        Ok(match self {
            Self::Linear => FieldLawValues::Linear,
            Self::Polynomial {
                chi1,
                chi2,
                amplitude_bound,
            } => FieldLawValues::Polynomial {
                chi1: chi1.evaluate(coordinates, parameters)?,
                chi2: chi2.evaluate(coordinates, parameters)?,
                amplitude_bound: amplitude_bound
                    .as_ref()
                    .map(|bound| bound.evaluate_constant(parameters))
                    .transpose()?,
            },
            Self::Saturable { chi, saturation } => FieldLawValues::Saturable {
                chi: chi.evaluate(coordinates, parameters)?,
                saturation: saturation.evaluate(coordinates, parameters)?,
            },
        })
    }

    fn rename_parameter(&self, old: &str, new: &str) -> Result<Self, MaterialError> {
        Ok(match self {
            Self::Linear => Self::Linear,
            Self::Polynomial {
                chi1,
                chi2,
                amplitude_bound,
            } => Self::Polynomial {
                chi1: chi1.rename_parameter(old, new)?,
                chi2: chi2.rename_parameter(old, new)?,
                amplitude_bound: amplitude_bound
                    .as_ref()
                    .map(|bound| bound.rename_parameter(old, new))
                    .transpose()?,
            },
            Self::Saturable { chi, saturation } => Self::Saturable {
                chi: chi.rename_parameter(old, new)?,
                saturation: saturation.rename_parameter(old, new)?,
            },
        })
    }

    fn fields(&self) -> Vec<&ScalarField> {
        match self {
            Self::Linear => vec![],
            Self::Polynomial {
                chi1,
                chi2,
                amplitude_bound,
            } => {
                let mut fields = vec![chi1, chi2];
                fields.extend(amplitude_bound.as_ref());
                fields
            }
            Self::Saturable { chi, saturation } => vec![chi, saturation],
        }
    }
}

impl TimeDrive {
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    pub fn uses_frame(&self) -> bool {
        matches!(self, Self::TravellingModulation { .. })
    }

    pub fn valid(&self, parameters: &[MaterialParameter]) -> bool {
        let common = |depth: &ScalarField, frequency: &ScalarField, phase: &ScalarField| {
            constant_passes(depth, parameters, |d| (0.0..1.0).contains(&d))
                && constant_passes(frequency, parameters, |f| f >= 0.0)
                && constant_passes(phase, parameters, |_| true)
        };
        match self {
            Self::None => true,
            Self::ParametricPump {
                depth,
                frequency_hz,
                phase_radians,
            } => common(depth, frequency_hz, phase_radians),
            Self::TimeCrystal {
                depth,
                frequency_hz,
                phase_radians,
                sharpness,
            } => {
                common(depth, frequency_hz, phase_radians)
                    && constant_passes(sharpness, parameters, |s| s > 0.0)
            }
            Self::TravellingModulation {
                depth,
                frequency_hz,
                phase_radians,
                wavenumber,
                angle_radians,
            } => {
                common(depth, frequency_hz, phase_radians)
                    && constant_passes(wavenumber, parameters, |q| q >= 0.0)
                    && constant_passes(angle_radians, parameters, |_| true)
            }
        }
    }

    pub fn evaluate(
        &self,
        parameters: &[MaterialParameter],
    ) -> Result<TimeDriveValues, MaterialError> {
        let constant = |field: &ScalarField| field.evaluate_constant(parameters);
        Ok(match self {
            Self::None => TimeDriveValues::None,
            Self::ParametricPump {
                depth,
                frequency_hz,
                phase_radians,
            } => TimeDriveValues::ParametricPump {
                depth: constant(depth)?,
                frequency_hz: constant(frequency_hz)?,
                phase_radians: constant(phase_radians)?,
            },
            Self::TimeCrystal {
                depth,
                frequency_hz,
                phase_radians,
                sharpness,
            } => TimeDriveValues::TimeCrystal {
                depth: constant(depth)?,
                frequency_hz: constant(frequency_hz)?,
                phase_radians: constant(phase_radians)?,
                sharpness: constant(sharpness)?,
            },
            Self::TravellingModulation {
                depth,
                frequency_hz,
                phase_radians,
                wavenumber,
                angle_radians,
            } => TimeDriveValues::TravellingModulation {
                depth: constant(depth)?,
                frequency_hz: constant(frequency_hz)?,
                phase_radians: constant(phase_radians)?,
                wavenumber: constant(wavenumber)?,
                angle_radians: constant(angle_radians)?,
            },
        })
    }

    fn rename_parameter(&self, old: &str, new: &str) -> Result<Self, MaterialError> {
        let rename = |field: &ScalarField| field.rename_parameter(old, new);
        Ok(match self {
            Self::None => Self::None,
            Self::ParametricPump {
                depth,
                frequency_hz,
                phase_radians,
            } => Self::ParametricPump {
                depth: rename(depth)?,
                frequency_hz: rename(frequency_hz)?,
                phase_radians: rename(phase_radians)?,
            },
            Self::TimeCrystal {
                depth,
                frequency_hz,
                phase_radians,
                sharpness,
            } => Self::TimeCrystal {
                depth: rename(depth)?,
                frequency_hz: rename(frequency_hz)?,
                phase_radians: rename(phase_radians)?,
                sharpness: rename(sharpness)?,
            },
            Self::TravellingModulation {
                depth,
                frequency_hz,
                phase_radians,
                wavenumber,
                angle_radians,
            } => Self::TravellingModulation {
                depth: rename(depth)?,
                frequency_hz: rename(frequency_hz)?,
                phase_radians: rename(phase_radians)?,
                wavenumber: rename(wavenumber)?,
                angle_radians: rename(angle_radians)?,
            },
        })
    }

    fn fields(&self) -> Vec<&ScalarField> {
        match self {
            Self::None => vec![],
            Self::ParametricPump {
                depth,
                frequency_hz,
                phase_radians,
            } => vec![depth, frequency_hz, phase_radians],
            Self::TimeCrystal {
                depth,
                frequency_hz,
                phase_radians,
                sharpness,
            } => vec![depth, frequency_hz, phase_radians, sharpness],
            Self::TravellingModulation {
                depth,
                frequency_hz,
                phase_radians,
                wavenumber,
                angle_radians,
            } => vec![
                depth,
                frequency_hz,
                phase_radians,
                wavenumber,
                angle_radians,
            ],
        }
    }
}

impl DampingLaw {
    pub const fn constant() -> Self {
        Self {
            rate: RateLaw::Constant,
            drive: TimeDrive::None,
        }
    }

    pub fn is_constant(&self) -> bool {
        matches!(self.rate, RateLaw::Constant) && self.drive.is_none()
    }

    pub fn valid(&self, parameters: &[MaterialParameter]) -> bool {
        self.drive.valid(parameters) && self.rate.valid(parameters)
    }

    pub fn parameter_names(&self) -> impl Iterator<Item = &str> {
        self.fields()
            .into_iter()
            .flat_map(ScalarField::parameter_names)
    }

    pub fn varies_in_space(&self) -> bool {
        self.fields()
            .iter()
            .any(|field| !field.spatially_constant())
    }

    pub fn uses_frame(&self) -> bool {
        self.varies_in_space() || self.drive.uses_frame()
    }

    pub fn rename_parameter(&self, old: &str, new: &str) -> Result<Self, MaterialError> {
        Ok(Self {
            rate: self.rate.rename_parameter(old, new)?,
            drive: self.drive.rename_parameter(old, new)?,
        })
    }

    fn fields(&self) -> Vec<&ScalarField> {
        let mut fields = self.rate.fields();
        fields.extend(self.drive.fields());
        fields
    }
}

impl RateLaw {
    pub fn valid(&self, parameters: &[MaterialParameter]) -> bool {
        match self {
            Self::Constant => true,
            Self::SaturableAbsorption { saturation } => passes(saturation, parameters, |s| s > 0.0),
            Self::Polynomial {
                beta1,
                beta2,
                amplitude_bound,
            } => {
                each_finite(&[beta1, beta2], parameters)
                    && amplitude_bound
                        .as_ref()
                        .is_none_or(|bound| constant_passes(bound, parameters, |b| b > 0.0))
                    && self.passive_where_known(parameters)
            }
            Self::VanDerPol {
                threshold,
                amplitude_bound,
            } => {
                passes(threshold, parameters, |t| t > 0.0)
                    && constant_passes(amplitude_bound, parameters, |b| b > 0.0)
            }
        }
    }

    fn passive_where_known(&self, parameters: &[MaterialParameter]) -> bool {
        if self
            .fields()
            .iter()
            .any(|field| !field.spatially_constant())
        {
            return true;
        }
        self.evaluate_at(origin(), parameters)
            .is_ok_and(|values| values.passive_range().is_some())
    }

    pub fn evaluate_at(
        &self,
        coordinates: MaterialCoordinates,
        parameters: &[MaterialParameter],
    ) -> Result<RateLawValues, MaterialError> {
        Ok(match self {
            Self::Constant => RateLawValues::Constant,
            Self::SaturableAbsorption { saturation } => RateLawValues::SaturableAbsorption {
                saturation: saturation.evaluate(coordinates, parameters)?,
            },
            Self::Polynomial {
                beta1,
                beta2,
                amplitude_bound,
            } => RateLawValues::Polynomial {
                beta1: beta1.evaluate(coordinates, parameters)?,
                beta2: beta2.evaluate(coordinates, parameters)?,
                amplitude_bound: amplitude_bound
                    .as_ref()
                    .map(|bound| bound.evaluate_constant(parameters))
                    .transpose()?,
            },
            Self::VanDerPol {
                threshold,
                amplitude_bound,
            } => RateLawValues::VanDerPol {
                threshold: threshold.evaluate(coordinates, parameters)?,
                amplitude_bound: amplitude_bound.evaluate_constant(parameters)?,
            },
        })
    }

    fn rename_parameter(&self, old: &str, new: &str) -> Result<Self, MaterialError> {
        Ok(match self {
            Self::Constant => Self::Constant,
            Self::SaturableAbsorption { saturation } => Self::SaturableAbsorption {
                saturation: saturation.rename_parameter(old, new)?,
            },
            Self::Polynomial {
                beta1,
                beta2,
                amplitude_bound,
            } => Self::Polynomial {
                beta1: beta1.rename_parameter(old, new)?,
                beta2: beta2.rename_parameter(old, new)?,
                amplitude_bound: amplitude_bound
                    .as_ref()
                    .map(|bound| bound.rename_parameter(old, new))
                    .transpose()?,
            },
            Self::VanDerPol {
                threshold,
                amplitude_bound,
            } => Self::VanDerPol {
                threshold: threshold.rename_parameter(old, new)?,
                amplitude_bound: amplitude_bound.rename_parameter(old, new)?,
            },
        })
    }

    fn fields(&self) -> Vec<&ScalarField> {
        match self {
            Self::Constant => vec![],
            Self::SaturableAbsorption { saturation } => vec![saturation],
            Self::Polynomial {
                beta1,
                beta2,
                amplitude_bound,
            } => {
                let mut fields = vec![beta1, beta2];
                fields.extend(amplitude_bound.as_ref());
                fields
            }
            Self::VanDerPol {
                threshold,
                amplitude_bound,
            } => vec![threshold, amplitude_bound],
        }
    }
}

impl RestoringLaw {
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    pub fn valid(&self, parameters: &[MaterialParameter]) -> bool {
        match self {
            Self::None => true,
            Self::KleinGordon { omega0 } | Self::SineGordon { omega0 } => {
                passes(omega0, parameters, |w| w >= 0.0)
            }
            Self::Phi4 {
                lambda,
                amplitude_bound,
            } => {
                passes(lambda, parameters, |l| l >= 0.0)
                    && constant_passes(amplitude_bound, parameters, |b| b > 0.0)
            }
        }
    }

    pub fn parameter_names(&self) -> impl Iterator<Item = &str> {
        self.fields()
            .into_iter()
            .flat_map(ScalarField::parameter_names)
    }

    pub fn varies_in_space(&self) -> bool {
        self.fields()
            .iter()
            .any(|field| !field.spatially_constant())
    }

    pub fn evaluate_at(
        &self,
        coordinates: MaterialCoordinates,
        parameters: &[MaterialParameter],
    ) -> Result<RestoringLawValues, MaterialError> {
        Ok(match self {
            Self::None => RestoringLawValues::None,
            Self::KleinGordon { omega0 } => RestoringLawValues::KleinGordon {
                omega0: omega0.evaluate(coordinates, parameters)?,
            },
            Self::SineGordon { omega0 } => RestoringLawValues::SineGordon {
                omega0: omega0.evaluate(coordinates, parameters)?,
            },
            Self::Phi4 {
                lambda,
                amplitude_bound,
            } => RestoringLawValues::Phi4 {
                lambda: lambda.evaluate(coordinates, parameters)?,
                amplitude_bound: amplitude_bound.evaluate_constant(parameters)?,
            },
        })
    }

    pub fn rename_parameter(&self, old: &str, new: &str) -> Result<Self, MaterialError> {
        Ok(match self {
            Self::None => Self::None,
            Self::KleinGordon { omega0 } => Self::KleinGordon {
                omega0: omega0.rename_parameter(old, new)?,
            },
            Self::SineGordon { omega0 } => Self::SineGordon {
                omega0: omega0.rename_parameter(old, new)?,
            },
            Self::Phi4 {
                lambda,
                amplitude_bound,
            } => Self::Phi4 {
                lambda: lambda.rename_parameter(old, new)?,
                amplitude_bound: amplitude_bound.rename_parameter(old, new)?,
            },
        })
    }

    fn fields(&self) -> Vec<&ScalarField> {
        match self {
            Self::None => vec![],
            Self::KleinGordon { omega0 } | Self::SineGordon { omega0 } => vec![omega0],
            Self::Phi4 {
                lambda,
                amplitude_bound,
            } => vec![lambda, amplitude_bound],
        }
    }
}

fn origin() -> MaterialCoordinates {
    MaterialCoordinates {
        x: 0.0,
        y: 0.0,
        r: 0.0,
        theta: 0.0,
    }
}

/// A coefficient that may vary in space passes a check at edit time when it is
/// known and satisfies it, or when it varies — its check then runs at the node
/// points during assembly. One that cannot be evaluated fails.
fn passes(
    field: &ScalarField,
    parameters: &[MaterialParameter],
    check: impl Fn(f64) -> bool,
) -> bool {
    if !field.spatially_constant() {
        return true;
    }
    field.evaluate_constant(parameters).is_ok_and(check)
}

/// A coefficient that must not vary in space: a drive parameter, an alternate
/// factor, an amplitude bound.
fn constant_passes(
    field: &ScalarField,
    parameters: &[MaterialParameter],
    check: impl Fn(f64) -> bool,
) -> bool {
    field.evaluate_constant(parameters).is_ok_and(check)
}

fn each_finite(fields: &[&ScalarField], parameters: &[MaterialParameter]) -> bool {
    fields
        .iter()
        .all(|field| passes(field, parameters, |value| value.is_finite()))
}

// ---------------------------------------------------------------------------
// Evaluated forms: the numbers the solver and the readout work with
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FieldLawValues {
    Linear,
    Polynomial {
        chi1: f64,
        chi2: f64,
        amplitude_bound: Option<f64>,
    },
    Saturable {
        chi: f64,
        saturation: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimeDriveValues {
    None,
    ParametricPump {
        depth: f64,
        frequency_hz: f64,
        phase_radians: f64,
    },
    TimeCrystal {
        depth: f64,
        frequency_hz: f64,
        phase_radians: f64,
        sharpness: f64,
    },
    TravellingModulation {
        depth: f64,
        frequency_hz: f64,
        phase_radians: f64,
        wavenumber: f64,
        angle_radians: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoefficientLawValues {
    pub field: FieldLawValues,
    pub drive: TimeDriveValues,
    pub alternate: Option<f64>,
    pub inverted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RateLawValues {
    Constant,
    SaturableAbsorption {
        saturation: f64,
    },
    Polynomial {
        beta1: f64,
        beta2: f64,
        amplitude_bound: Option<f64>,
    },
    VanDerPol {
        threshold: f64,
        amplitude_bound: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RestoringLawValues {
    None,
    KleinGordon { omega0: f64 },
    SineGordon { omega0: f64 },
    Phi4 { lambda: f64, amplitude_bound: f64 },
}

impl FieldLawValues {
    /// The multiplier `ḡ(u)`.
    pub fn multiplier(self, u: f64) -> f64 {
        match self {
            Self::Linear => 1.0,
            Self::Polynomial { chi1, chi2, .. } => 1.0 + chi1 * u + chi2 * u * u,
            Self::Saturable { chi, saturation } => {
                let v = (u / saturation) * (u / saturation);
                1.0 + chi * u * u / (1.0 + v)
            }
        }
    }

    /// `dḡ/du`.
    pub fn multiplier_slope(self, u: f64) -> f64 {
        match self {
            Self::Linear => 0.0,
            Self::Polynomial { chi1, chi2, .. } => chi1 + 2.0 * chi2 * u,
            Self::Saturable { chi, saturation } => {
                let v = (u / saturation) * (u / saturation);
                chi * 2.0 * u / ((1.0 + v) * (1.0 + v))
            }
        }
    }

    /// `d(ḡ·u)/du`, or `d(u/ḡ)/du` when the law is inverted: the slope of the
    /// conserved variable against the field, which is what the pointwise solve
    /// needs to be positive and what the step bound has to see.
    pub fn tangent(self, u: f64, inverted: bool) -> f64 {
        let multiplier = self.multiplier(u);
        let slope = self.multiplier_slope(u);
        if inverted {
            (multiplier - u * slope) / (multiplier * multiplier)
        } else {
            multiplier + u * slope
        }
    }

    pub fn amplitude_bound(self) -> Option<f64> {
        match self {
            Self::Polynomial {
                amplitude_bound, ..
            } => amplitude_bound,
            Self::Linear | Self::Saturable { .. } => None,
        }
    }

    /// The range the tangent takes over the amplitudes the law admits — the
    /// declared bound when there is one, every `u` otherwise — or `None` when
    /// the law is not monotone there, so the pointwise solve would have no
    /// single root. An unbounded polynomial's tangent has no upper limit;
    /// that end is `f64::INFINITY`.
    pub fn tangent_range(self, inverted: bool) -> Option<(f64, f64)> {
        match self {
            Self::Linear => Some((1.0, 1.0)),
            Self::Polynomial {
                chi1,
                chi2,
                amplitude_bound,
            } => match amplitude_bound {
                Some(bound) if !bound.is_finite() || bound <= 0.0 => None,
                Some(bound) if inverted => {
                    if !polynomial_positive_on_interval(chi1, chi2, bound)
                        || (chi2 > 0.0 && 1.0 - chi2 * bound * bound <= 0.0)
                    {
                        return None;
                    }
                    let mut candidates = vec![-bound, bound];
                    candidates.extend(inverted_polynomial_stationary_points(chi1, chi2, bound));
                    self.range_at(&candidates, true)
                }
                Some(bound) => {
                    // The tangent `1 + 2χ₁u + 3χ₂u²` is a parabola; its
                    // extremes over the interval sit at the ends or the vertex.
                    let mut candidates = vec![-bound, bound];
                    if chi2 != 0.0 {
                        let vertex = -chi1 / (3.0 * chi2);
                        if vertex.abs() <= bound {
                            candidates.push(vertex);
                        }
                    }
                    self.range_at(&candidates, false)
                }
                None if inverted => None,
                None => {
                    if chi1 == 0.0 && chi2 == 0.0 {
                        return Some((1.0, 1.0));
                    }
                    // Positive for every `u` only when the parabola never
                    // touches zero: it opens upwards and its discriminant is
                    // negative.
                    if chi2 <= 0.0 || chi1 * chi1 >= 3.0 * chi2 {
                        return None;
                    }
                    Some((1.0 - chi1 * chi1 / (3.0 * chi2), f64::INFINITY))
                }
            },
            Self::Saturable { chi, saturation } => {
                if !chi.is_finite() || !saturation.is_finite() || saturation <= 0.0 {
                    return None;
                }
                let a = chi * saturation * saturation;
                // `ḡ` runs from 1 at rest to `1 + χ·s²` far out, so this is its
                // positivity for every `u`.
                if 1.0 + a <= 0.0 {
                    return None;
                }
                if inverted {
                    // With v=u²/s², the nontrivial extremum is at
                    // v=3/(1+a), with value (8-a)/(8(1+a)). It is a maximum
                    // for -1<a<0 and a minimum for a>0.
                    let extremum = (8.0 - a) / (8.0 * (1.0 + a));
                    if a < 0.0 {
                        Some((1.0, extremum))
                    } else if a < 8.0 {
                        Some((extremum, 1.0))
                    } else {
                        None
                    }
                } else {
                    // `ḡ + uḡ′ = 1 + χ·s²·g(v)` with `v = u²/s²` and
                    // `g = v(v+3)/(1+v)²`, which rises from 0 to 9/8 at `v = 3`
                    // and settles to 1: the extremes are at rest, at that
                    // point, and in the limit.
                    let extremum = 1.0 + 9.0 * a / 8.0;
                    if a < 0.0 {
                        (extremum > 0.0).then_some((extremum, 1.0))
                    } else {
                        Some((1.0, extremum))
                    }
                }
            }
        }
    }

    fn range_at(self, points: &[f64], inverted: bool) -> Option<(f64, f64)> {
        let mut minimum = f64::INFINITY;
        let mut maximum = f64::NEG_INFINITY;
        for &u in points {
            if self.multiplier(u) <= 0.0 {
                return None;
            }
            let tangent = self.tangent(u, inverted);
            minimum = minimum.min(tangent);
            maximum = maximum.max(tangent);
        }
        (minimum > 0.0 && minimum.is_finite()).then_some((minimum, maximum))
    }
}

fn polynomial_positive_on_interval(chi1: f64, chi2: f64, bound: f64) -> bool {
    let mut minimum =
        (1.0 - chi1 * bound + chi2 * bound * bound).min(1.0 + chi1 * bound + chi2 * bound * bound);
    if chi2 > 0.0 {
        let vertex = -chi1 / (2.0 * chi2);
        if vertex.abs() <= bound {
            minimum = minimum.min(1.0 + chi1 * vertex + chi2 * vertex * vertex);
        }
    }
    minimum.is_finite() && minimum > 0.0
}

/// All stationary points of `(1-χ₂u²)/(1+χ₁u+χ₂u²)²` in the admitted
/// interval. The derivative's roots are isolated by its analytic turning
/// points, then bracketed to full f64 precision; there is no sampling bound.
fn inverted_polynomial_stationary_points(chi1: f64, chi2: f64, bound: f64) -> Vec<f64> {
    if chi2 == 0.0 {
        return Vec::new();
    }
    let polynomial = |u: f64| chi1 + 3.0 * chi2 * u - chi2 * chi2 * u * u * u;
    let mut cuts = vec![-bound, bound];
    if chi2 > 0.0 {
        let turning = chi2.sqrt().recip();
        if turning < bound {
            cuts.extend([-turning, turning]);
        }
    }
    cuts.sort_by(f64::total_cmp);
    cuts.dedup_by(|left, right| *left == *right);

    let mut roots = Vec::new();
    for &cut in &cuts {
        if polynomial(cut) == 0.0 {
            roots.push(cut);
        }
    }
    for interval in cuts.windows(2) {
        let mut low = interval[0];
        let mut high = interval[1];
        let mut low_value = polynomial(low);
        let high_value = polynomial(high);
        if low_value == 0.0 || high_value == 0.0 || low_value.signum() == high_value.signum() {
            continue;
        }
        for _ in 0..80 {
            let middle = 0.5 * (low + high);
            let middle_value = polynomial(middle);
            if middle_value == 0.0 {
                low = middle;
                high = middle;
                break;
            }
            if low_value.signum() == middle_value.signum() {
                low = middle;
                low_value = middle_value;
            } else {
                high = middle;
            }
        }
        roots.push(0.5 * (low + high));
    }
    roots.sort_by(f64::total_cmp);
    roots.dedup_by(|left, right| (*left - *right).abs() <= f64::EPSILON * bound.max(1.0));
    roots
}

impl TimeDriveValues {
    /// The multiplier at time `t` and at a point of the material frame.
    pub fn multiplier(self, time: f64, coordinates: MaterialCoordinates) -> f64 {
        match self {
            Self::None => 1.0,
            Self::ParametricPump {
                depth,
                frequency_hz,
                phase_radians,
            } => {
                1.0 + depth
                    * reduced_cos(std::f64::consts::TAU * frequency_hz * time + phase_radians)
            }
            Self::TimeCrystal {
                depth,
                frequency_hz,
                phase_radians,
                sharpness,
            } => {
                let carrier =
                    reduced_cos(std::f64::consts::TAU * frequency_hz * time + phase_radians);
                let square = if sharpness.abs() < 1.0e-6 {
                    // tanh(s c)/tanh(s) = c[1 + s²(1-c²)/3 + O(s⁴)].
                    carrier * (1.0 + sharpness * sharpness * (1.0 - carrier * carrier) / 3.0)
                } else {
                    (sharpness * carrier).tanh() / sharpness.tanh()
                };
                1.0 + depth * square
            }
            Self::TravellingModulation {
                depth,
                frequency_hz,
                phase_radians,
                wavenumber,
                angle_radians,
            } => {
                let along =
                    coordinates.x * angle_radians.cos() + coordinates.y * angle_radians.sin();
                1.0 + depth
                    * reduced_cos(
                        std::f64::consts::TAU * frequency_hz * time - wavenumber * along
                            + phase_radians,
                    )
            }
        }
    }

    pub fn depth(self) -> f64 {
        match self {
            Self::None => 0.0,
            Self::ParametricPump { depth, .. }
            | Self::TimeCrystal { depth, .. }
            | Self::TravellingModulation { depth, .. } => depth,
        }
    }

    /// The multiplier's range over a cycle.
    pub fn range(self) -> (f64, f64) {
        let depth = self.depth();
        (1.0 - depth, 1.0 + depth)
    }
}

fn reduced_cos(phase: f64) -> f64 {
    phase.rem_euclid(std::f64::consts::TAU).cos()
}

impl CoefficientLawValues {
    /// The range of `d(coefficient·u)/du` over the field's amplitude, the
    /// drive's cycle and both switch states, in units of the base coefficient
    /// — or `None` when the field law is not monotone. The step bound scales
    /// with its minimum and the readout prints both ends.
    pub fn tangent_range(self) -> Option<(f64, f64)> {
        let (field_low, field_high) = self.field.tangent_range(self.inverted)?;
        let (drive_low, drive_high) = self.drive.range();
        let (switch_low, switch_high) = match self.alternate {
            Some(alternate) => (alternate.min(1.0), alternate.max(1.0)),
            None => (1.0, 1.0),
        };
        let (time_low, time_high) = (drive_low * switch_low, drive_high * switch_high);
        // Inverting the multiplier inverts the time factors; the field's
        // tangent was already taken of the inverted form.
        let (time_low, time_high) = if self.inverted {
            (1.0 / time_high, 1.0 / time_low)
        } else {
            (time_low, time_high)
        };
        Some((field_low * time_low, field_high * time_high))
    }

    /// The range of the local wave speed relative to the base coefficient's,
    /// for a law on the mass row: `c/c₀ = 1/√ĝ`.
    pub fn mass_speed_ratio_range(self) -> Option<(f64, f64)> {
        let (low, high) = self.tangent_range()?;
        Some((1.0 / high.sqrt(), 1.0 / low.sqrt()))
    }

    /// The same for a law on the stiffness row: `c/c₀ = √ĥ`.
    pub fn stiffness_speed_ratio_range(self) -> Option<(f64, f64)> {
        let (low, high) = self.tangent_range()?;
        Some((low.sqrt(), high.sqrt()))
    }
}

impl RateLawValues {
    /// The rate multiplier `w(u)`.
    pub fn multiplier(self, u: f64) -> f64 {
        match self {
            Self::Constant => 1.0,
            Self::SaturableAbsorption { saturation } => {
                1.0 / (1.0 + (u / saturation) * (u / saturation))
            }
            Self::Polynomial { beta1, beta2, .. } => 1.0 + beta1 * u + beta2 * u * u,
            Self::VanDerPol { threshold, .. } => (u / threshold) * (u / threshold) - 1.0,
        }
    }

    /// The exact multiplier range over the authored amplitude domain. `None`
    /// means a passive loss law becomes negative somewhere in that domain.
    pub fn passive_range(self) -> Option<(f64, f64)> {
        match self {
            Self::Constant => Some((1.0, 1.0)),
            Self::SaturableAbsorption { saturation }
                if saturation.is_finite() && saturation > 0.0 =>
            {
                Some((0.0, 1.0))
            }
            Self::SaturableAbsorption { .. } => None,
            Self::Polynomial {
                beta1,
                beta2,
                amplitude_bound,
            } => {
                if !beta1.is_finite() || !beta2.is_finite() {
                    return None;
                }
                match amplitude_bound {
                    Some(bound) if bound.is_finite() && bound > 0.0 => {
                        let mut values = vec![
                            1.0 - beta1 * bound + beta2 * bound * bound,
                            1.0 + beta1 * bound + beta2 * bound * bound,
                        ];
                        if beta2 != 0.0 {
                            let vertex = -beta1 / (2.0 * beta2);
                            if vertex.abs() <= bound {
                                values.push(1.0 + beta1 * vertex + beta2 * vertex * vertex);
                            }
                        }
                        let low = values.iter().copied().fold(f64::INFINITY, f64::min);
                        let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                        (low >= 0.0 && high.is_finite()).then_some((low, high))
                    }
                    Some(_) => None,
                    None if beta1 == 0.0 && beta2 == 0.0 => Some((1.0, 1.0)),
                    None if beta2 > 0.0 => {
                        let low = 1.0 - beta1 * beta1 / (4.0 * beta2);
                        (low >= 0.0).then_some((low, f64::INFINITY))
                    }
                    None => None,
                }
            }
            Self::VanDerPol { .. } => None,
        }
    }
}

impl RestoringLawValues {
    /// `V′(u)`, the acceleration taken away at a node.
    pub fn slope(self, u: f64) -> f64 {
        match self {
            Self::None => 0.0,
            Self::KleinGordon { omega0 } => omega0 * omega0 * u,
            Self::SineGordon { omega0 } => omega0 * omega0 * u.sin(),
            Self::Phi4 { lambda, .. } => lambda * (u * u * u - u),
        }
    }

    /// The largest `|V″|` over the amplitudes the law admits: the restoring
    /// law's contribution to the step bound.
    pub fn curvature_bound(self) -> f64 {
        match self {
            Self::None => 0.0,
            Self::KleinGordon { omega0 } | Self::SineGordon { omega0 } => omega0 * omega0,
            Self::Phi4 {
                lambda,
                amplitude_bound,
            } => {
                lambda
                    * (3.0 * amplitude_bound * amplitude_bound - 1.0)
                        .abs()
                        .max(1.0)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The effective law as text
// ---------------------------------------------------------------------------

/// How a coefficient is spelled in the effective-law line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LawTextStyle {
    /// Formulas as written, so a preset's parameter names show.
    Names,
    /// Every spatially constant expression evaluated to its number.
    Numbers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicalFieldArgument {
    MechanicalPrimary,
    MechanicalComplementary,
    Electric,
    Magnetic,
}

impl PhysicalFieldArgument {
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::MechanicalPrimary => "u",
            Self::MechanicalComplementary => "e",
            Self::Electric => "E",
            Self::Magnetic => "H",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstitutivePlacement {
    PrimaryScalar,
    ComplementaryVector,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicalCoefficient {
    MassDensity,
    ReciprocalStiffness,
    ElectricPermittivity,
    MagneticPermeability,
}

#[derive(Clone, Copy, Debug)]
pub struct CoefficientLawRole<'a> {
    pub coefficient: PhysicalCoefficient,
    pub argument: PhysicalFieldArgument,
    pub placement: ConstitutivePlacement,
    pub law: &'a CoefficientLaw,
}

/// Assigns authored rows to physical fields before the solver decides where a
/// scalar or vector constitutive inverse is evaluated. This mapping, rather
/// than row order, is the skin-portability contract.
pub fn coefficient_law_roles(
    material: &Material,
    physics: PhysicsModel,
) -> [CoefficientLawRole<'_>; 2] {
    match physics {
        PhysicsModel::Mechanical => [
            CoefficientLawRole {
                coefficient: PhysicalCoefficient::MassDensity,
                argument: PhysicalFieldArgument::MechanicalPrimary,
                placement: ConstitutivePlacement::PrimaryScalar,
                law: &material.mass_law,
            },
            CoefficientLawRole {
                coefficient: PhysicalCoefficient::ReciprocalStiffness,
                argument: PhysicalFieldArgument::MechanicalComplementary,
                placement: ConstitutivePlacement::ComplementaryVector,
                law: &material.stiffness_law,
            },
        ],
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        } => [
            CoefficientLawRole {
                coefficient: PhysicalCoefficient::ElectricPermittivity,
                argument: PhysicalFieldArgument::Electric,
                placement: ConstitutivePlacement::PrimaryScalar,
                law: &material.mass_law,
            },
            CoefficientLawRole {
                coefficient: PhysicalCoefficient::MagneticPermeability,
                argument: PhysicalFieldArgument::Magnetic,
                placement: ConstitutivePlacement::ComplementaryVector,
                law: &material.stiffness_law,
            },
        ],
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        } => [
            CoefficientLawRole {
                coefficient: PhysicalCoefficient::ElectricPermittivity,
                argument: PhysicalFieldArgument::Electric,
                placement: ConstitutivePlacement::ComplementaryVector,
                law: &material.mass_law,
            },
            CoefficientLawRole {
                coefficient: PhysicalCoefficient::MagneticPermeability,
                argument: PhysicalFieldArgument::Magnetic,
                placement: ConstitutivePlacement::PrimaryScalar,
                law: &material.stiffness_law,
            },
        ],
    }
}

/// One line per authored coefficient or loss channel, with its physical field
/// argument. A line can describe valid persisted data that is not executable
/// until its implementation gate closes.
pub fn effective_law_lines(
    material: &Material,
    physics: PhysicsModel,
    style: LawTextStyle,
) -> Vec<String> {
    let names = SkinNames::of(physics);
    let parameters = &material.parameters;
    let mut lines = Vec::new();
    for (law, coefficient) in [
        (&material.mass_law, names.mass),
        (&material.stiffness_law, names.stiffness),
    ] {
        if let Some(line) = coefficient_line(
            law,
            coefficient.symbol,
            coefficient.argument,
            parameters,
            style,
        ) {
            lines.push(line);
        }
    }
    for (loss, rate_name, field_name) in [
        (material.electric_loss.as_ref(), "γ_E", "E"),
        (material.magnetic_loss.as_ref(), "γ_H", "H"),
    ] {
        let Some(loss) = loss else {
            continue;
        };
        let mut factors = Vec::new();
        if let Some(text) = rate_text(&loss.law.rate, field_name, parameters, style) {
            factors.push(text);
        }
        if let Some(text) = drive_text(&loss.law.drive, parameters, style) {
            factors.push(text);
        }
        let arguments = arguments(
            !matches!(loss.law.rate, RateLaw::Constant),
            !loss.law.drive.is_none(),
            matches!(loss.law.drive, TimeDrive::TravellingModulation { .. }),
            field_name,
        );
        let head = if arguments.is_empty() {
            rate_name.to_owned()
        } else {
            format!("{rate_name}({arguments})")
        };
        let base = scalar_text(&loss.base_rate, parameters, style);
        let product = if factors.is_empty() {
            base
        } else {
            format!("{base} · {}", factors.join(" · "))
        };
        lines.push(format!("{head} = {product}"));
    }
    if let Some(text) = restoring_text(&material.restoring, names.primary_field, parameters, style)
    {
        lines.push(format!("Reserved oscillator (not executable): {text}"));
    }
    lines
}

struct SkinNames {
    mass: CoefficientName,
    stiffness: CoefficientName,
    primary_field: &'static str,
}

#[derive(Clone, Copy)]
struct CoefficientName {
    symbol: &'static str,
    argument: &'static str,
}

impl SkinNames {
    fn of(physics: PhysicsModel) -> Self {
        match physics {
            PhysicsModel::Mechanical => Self {
                mass: CoefficientName {
                    symbol: "ρ",
                    argument: "u",
                },
                stiffness: CoefficientName {
                    symbol: "s",
                    argument: "e",
                },
                primary_field: "u",
            },
            PhysicsModel::Electromagnetic { polarization } => Self {
                mass: CoefficientName {
                    symbol: "ε",
                    argument: "E",
                },
                stiffness: CoefficientName {
                    symbol: "μ",
                    argument: "H",
                },
                primary_field: match polarization {
                    ElectromagneticPolarization::Tm => "E",
                    ElectromagneticPolarization::Te => "H",
                },
            },
        }
    }
}

fn coefficient_line(
    law: &CoefficientLaw,
    coefficient: &str,
    field: &str,
    parameters: &[MaterialParameter],
    style: LawTextStyle,
) -> Option<String> {
    if law.is_linear() {
        return None;
    }
    let mut factors = Vec::new();
    if let Some(text) = drive_text(&law.drive, parameters, style) {
        factors.push(text);
    }
    if let Some(text) = field_text(&law.field, field, parameters, style) {
        factors.push(text);
    }
    let arguments = arguments(
        !matches!(law.field, FieldLaw::Linear),
        !law.drive.is_none(),
        matches!(law.drive, TimeDrive::TravellingModulation { .. }),
        field,
    );
    let head = if arguments.is_empty() {
        coefficient.to_owned()
    } else {
        format!("{coefficient}({arguments})")
    };
    let product = if factors.is_empty() {
        "1".to_owned()
    } else {
        factors.join(" · ")
    };
    let mut line = if law.inverted {
        if factors.len() > 1 {
            format!("{head} = {coefficient}₀ / [{product}]")
        } else {
            format!("{head} = {coefficient}₀ / {product}")
        }
    } else {
        format!("{head} = {coefficient}₀ · {product}")
    };
    if let Some(alternate) = &law.alternate {
        let factor = scalar_text(alternate, parameters, style);
        if law.inverted {
            line.push_str(&format!(" — switched: ÷{factor}"));
        } else {
            line.push_str(&format!(" — switched: ×{factor}"));
        }
    }
    Some(line)
}

fn arguments(field: bool, time: bool, space: bool, field_name: &str) -> String {
    let mut parts = Vec::new();
    if field {
        parts.push(field_name);
    }
    if time {
        parts.push("t");
    }
    if space {
        parts.push("x");
    }
    parts.join(", ")
}

fn field_text(
    law: &FieldLaw,
    field: &str,
    parameters: &[MaterialParameter],
    style: LawTextStyle,
) -> Option<String> {
    let text = |value: &ScalarField| scalar_text(value, parameters, style);
    match law {
        FieldLaw::Linear => None,
        FieldLaw::Polynomial { chi1, chi2, .. } => {
            let mut terms = vec!["1".to_owned()];
            if chi1.constant_value() != Some(0.0) {
                terms.push(format!("{}·{field}", text(chi1)));
            }
            if chi2.constant_value() != Some(0.0) {
                terms.push(format!("{}·{field}²", text(chi2)));
            }
            Some(format!("({})", terms.join(" + ")))
        }
        FieldLaw::Saturable { chi, saturation } => Some(format!(
            "(1 + {}·{field}² / (1 + {field}²/{}²))",
            text(chi),
            text(saturation)
        )),
    }
}

fn drive_text(
    drive: &TimeDrive,
    parameters: &[MaterialParameter],
    style: LawTextStyle,
) -> Option<String> {
    let text = |value: &ScalarField| scalar_text(value, parameters, style);
    match drive {
        TimeDrive::None => None,
        TimeDrive::ParametricPump {
            depth,
            frequency_hz,
            phase_radians,
        } => Some(format!(
            "(1 + {}·cos(2π·{}·t{}))",
            text(depth),
            text(frequency_hz),
            phase_text(phase_radians, parameters, style)
        )),
        TimeDrive::TimeCrystal {
            depth,
            frequency_hz,
            phase_radians,
            sharpness,
        } => Some(format!(
            "(1 + {}·square(2π·{}·t{}; {}))",
            text(depth),
            text(frequency_hz),
            phase_text(phase_radians, parameters, style),
            text(sharpness)
        )),
        TimeDrive::TravellingModulation {
            depth,
            frequency_hz,
            phase_radians,
            wavenumber,
            angle_radians,
        } => Some(format!(
            "(1 + {}·cos(2π·{}·t − {}·x∠{}{}))",
            text(depth),
            text(frequency_hz),
            text(wavenumber),
            text(angle_radians),
            phase_text(phase_radians, parameters, style)
        )),
    }
}

fn phase_text(
    phase: &ScalarField,
    parameters: &[MaterialParameter],
    style: LawTextStyle,
) -> String {
    if phase.constant_value() == Some(0.0) {
        String::new()
    } else {
        format!(" + {}", scalar_text(phase, parameters, style))
    }
}

fn rate_text(
    rate: &RateLaw,
    field: &str,
    parameters: &[MaterialParameter],
    style: LawTextStyle,
) -> Option<String> {
    let text = |value: &ScalarField| scalar_text(value, parameters, style);
    match rate {
        RateLaw::Constant => None,
        RateLaw::SaturableAbsorption { saturation } => {
            Some(format!("1 / (1 + {field}²/{}²)", text(saturation)))
        }
        RateLaw::Polynomial { beta1, beta2, .. } => {
            let mut terms = vec!["1".to_owned()];
            if beta1.constant_value() != Some(0.0) {
                terms.push(format!("{}·{field}", text(beta1)));
            }
            if beta2.constant_value() != Some(0.0) {
                terms.push(format!("{}·{field}²", text(beta2)));
            }
            Some(format!("({})", terms.join(" + ")))
        }
        RateLaw::VanDerPol { threshold, .. } => {
            Some(format!("({field}²/{}² − 1)", text(threshold)))
        }
    }
}

fn restoring_text(
    law: &RestoringLaw,
    field: &str,
    parameters: &[MaterialParameter],
    style: LawTextStyle,
) -> Option<String> {
    let text = |value: &ScalarField| scalar_text(value, parameters, style);
    match law {
        RestoringLaw::None => None,
        RestoringLaw::KleinGordon { omega0 } => Some(format!("{}²·{field}", text(omega0))),
        RestoringLaw::SineGordon { omega0 } => Some(format!("{}²·sin({field})", text(omega0))),
        RestoringLaw::Phi4 { lambda, .. } => Some(format!("{}·({field}³ − {field})", text(lambda))),
    }
}

fn scalar_text(
    field: &ScalarField,
    parameters: &[MaterialParameter],
    style: LawTextStyle,
) -> String {
    match (field, style) {
        (ScalarField::Constant(value), _) => number_text(*value),
        (ScalarField::Formula(formula), LawTextStyle::Names) => formula.source().to_owned(),
        (ScalarField::Formula(formula), LawTextStyle::Numbers) => {
            match field.evaluate_constant(parameters) {
                Ok(value) => number_text(value),
                Err(_) => formula.source().to_owned(),
            }
        }
    }
}

/// A coefficient printed the way a person would write it: four decimals at
/// most, no trailing zeros.
pub fn number_text(value: f64) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    let text = format!("{value:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-0" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parameters() -> Vec<MaterialParameter> {
        vec![
            MaterialParameter {
                name: "chi".into(),
                value: 0.2,
            },
            MaterialParameter {
                name: "u_sat".into(),
                value: 1.0,
            },
        ]
    }

    fn constant(value: f64) -> ScalarField {
        ScalarField::constant(value)
    }

    fn polynomial(chi1: f64, chi2: f64, bound: Option<f64>) -> FieldLaw {
        FieldLaw::Polynomial {
            chi1: constant(chi1),
            chi2: constant(chi2),
            amplitude_bound: bound.map(constant),
        }
    }

    fn saturable(chi: f64, saturation: f64) -> FieldLaw {
        FieldLaw::Saturable {
            chi: constant(chi),
            saturation: constant(saturation),
        }
    }

    fn law(field: FieldLaw, inverted: bool) -> CoefficientLaw {
        CoefficientLaw {
            field,
            inverted,
            ..CoefficientLaw::linear()
        }
    }

    #[test]
    fn the_inert_laws_are_valid_and_name_nothing() {
        let none = &[][..];
        assert!(CoefficientLaw::linear().valid(none));
        assert!(DampingLaw::constant().valid(none));
        assert!(RestoringLaw::None.valid(none));
        assert_eq!(CoefficientLaw::linear().parameter_names().count(), 0);
        assert!(!CoefficientLaw::linear().varies_in_space());
        assert_eq!(
            CoefficientLaw::linear()
                .constant_values(none)
                .unwrap()
                .unwrap()
                .tangent_range(),
            Some((1.0, 1.0))
        );
    }

    #[test]
    fn a_polynomial_is_monotone_without_a_bound_only_when_its_parabola_stays_positive() {
        let none = &[][..];
        // Kerr: chi1 = 0, chi2 > 0.
        assert!(law(polynomial(0.0, 1.0, None), false).valid(none));
        // chi1² < 3·chi2 keeps the tangent positive everywhere.
        assert!(law(polynomial(1.0, 0.5, None), false).valid(none));
        // chi1² ≥ 3·chi2 does not, so a bound is required...
        assert!(!law(polynomial(1.0, 0.1, None), false).valid(none));
        // ...and suffices where the tangent stays positive over it,
        assert!(law(polynomial(1.0, 0.1, Some(0.2)), false).valid(none));
        // but not where it turns negative: 1 − 4 + 1.2 at u = −2.
        assert!(!law(polynomial(1.0, 0.1, Some(2.0)), false).valid(none));
        // Self-defocusing needs a bound too.
        assert!(!law(polynomial(0.0, -1.0, None), false).valid(none));
        assert!(law(polynomial(0.0, -1.0, Some(0.5)), false).valid(none));
    }

    #[test]
    fn an_inverted_polynomial_always_needs_a_bound_below_its_turnover() {
        let none = &[][..];
        assert!(!law(polynomial(0.0, 1.0, None), true).valid(none));
        // u/ḡ turns over at |u| = 1/√chi2 = 1.
        assert!(law(polynomial(0.0, 1.0, Some(0.9)), true).valid(none));
        assert!(!law(polynomial(0.0, 1.0, Some(1.1)), true).valid(none));
        // chi1 cancels from the inverted tangent, so it does not move the turnover.
        assert!(law(polynomial(0.3, 1.0, Some(0.9)), true).valid(none));
    }

    #[test]
    fn a_saturable_law_is_bounded_by_its_own_shape() {
        let none = &[][..];
        assert!(law(saturable(0.2, 1.0), false).valid(none));
        assert!(law(saturable(50.0, 1.0), false).valid(none));
        // Self-defocusing: |chi|·s² < 8/9 keeps the tangent positive.
        assert!(law(saturable(-0.5, 1.0), false).valid(none));
        assert!(!law(saturable(-0.9, 1.0), false).valid(none));
        // Inverted: chi·s² < 8 for focusing, |chi|·s² < 1 for defocusing.
        assert!(law(saturable(7.0, 1.0), true).valid(none));
        assert!(!law(saturable(9.0, 1.0), true).valid(none));
        assert!(law(saturable(-0.5, 1.0), true).valid(none));
        assert!(!law(saturable(-1.5, 1.0), true).valid(none));
        assert!(!law(saturable(0.2, 0.0), false).valid(none));
    }

    #[test]
    fn tangent_ranges_match_the_closed_forms() {
        let close = |range: Option<(f64, f64)>, low: f64, high: f64| {
            let (a, b) = range.unwrap();
            assert!((a - low).abs() < 1.0e-6, "low {a} vs {low}");
            assert!(
                (b - high).abs() < 1.0e-6 || (b.is_infinite() && high.is_infinite()),
                "high {b} vs {high}"
            );
        };
        // Kerr, unbounded: the tangent starts at 1 and grows without limit.
        close(
            FieldLawValues::Polynomial {
                chi1: 0.0,
                chi2: 1.0,
                amplitude_bound: None,
            }
            .tangent_range(false),
            1.0,
            f64::INFINITY,
        );
        // Bounded at 2: 1 + 3·4 at the ends.
        close(
            FieldLawValues::Polynomial {
                chi1: 0.0,
                chi2: 1.0,
                amplitude_bound: Some(2.0),
            }
            .tangent_range(false),
            1.0,
            13.0,
        );
        // Saturable focusing: peak 1 + 9χs²/8 at v = 3.
        close(
            FieldLawValues::Saturable {
                chi: 0.2,
                saturation: 1.0,
            }
            .tangent_range(false),
            1.0,
            1.225,
        );
        // Saturable defocusing: trough 1 − 9|χ|s²/8.
        close(
            FieldLawValues::Saturable {
                chi: -0.5,
                saturation: 1.0,
            }
            .tangent_range(false),
            0.4375,
            1.0,
        );
        // Inverted saturable, focusing: the finite-amplitude extremum is below
        // the asymptotic 1/(1+χs²) limit.
        let (low, high) = FieldLawValues::Saturable {
            chi: 3.0,
            saturation: 1.0,
        }
        .tangent_range(true)
        .unwrap();
        assert!((low - 5.0 / 32.0).abs() < 1.0e-12, "{low}");
        assert_eq!(high, 1.0);
    }

    #[test]
    fn a_drive_and_a_switch_widen_the_range_and_inversion_flips_them() {
        let pumped = CoefficientLawValues {
            field: FieldLawValues::Linear,
            drive: TimeDriveValues::ParametricPump {
                depth: 0.3,
                frequency_hz: 2.0,
                phase_radians: 0.0,
            },
            alternate: Some(1.6),
            inverted: false,
        };
        let (low, high) = pumped.tangent_range().unwrap();
        assert!((low - 0.7).abs() < 1.0e-12);
        assert!((high - 1.3 * 1.6).abs() < 1.0e-12);
        let inverted = CoefficientLawValues {
            inverted: true,
            ..pumped
        };
        let (low, high) = inverted.tangent_range().unwrap();
        assert!((low - 1.0 / (1.3 * 1.6)).abs() < 1.0e-12);
        assert!((high - 1.0 / 0.7).abs() < 1.0e-12);
        let (slow, fast) = pumped.mass_speed_ratio_range().unwrap();
        assert!((slow - 1.0 / (1.3 * 1.6f64).sqrt()).abs() < 1.0e-12);
        assert!((fast - 1.0 / 0.7f64.sqrt()).abs() < 1.0e-12);
    }

    #[test]
    fn drives_must_be_spatially_constant_and_inside_their_ranges() {
        let none = &[][..];
        let pump = |depth: ScalarField| TimeDrive::ParametricPump {
            depth,
            frequency_hz: constant(2.0),
            phase_radians: constant(0.0),
        };
        assert!(pump(constant(0.3)).valid(none));
        assert!(!pump(constant(1.0)).valid(none));
        assert!(!pump(constant(-0.1)).valid(none));
        assert!(!pump(ScalarField::formula("0.1 + x").unwrap()).valid(none));
        assert!(pump(ScalarField::formula("chi").unwrap()).valid(&parameters()));
        assert!(!pump(ScalarField::formula("missing").unwrap()).valid(&parameters()));
        let mut law = CoefficientLaw::linear();
        law.alternate = Some(constant(0.0));
        assert!(!law.valid(none));
        law.alternate = Some(ScalarField::formula("2*x").unwrap());
        assert!(!law.valid(none));
        law.alternate = Some(constant(1.6));
        assert!(law.valid(none));
    }

    #[test]
    fn a_spatial_field_coefficient_defers_its_checks_to_assembly() {
        // chi = −2 at x = 1 would fail the saturable bound, but nothing at edit
        // time knows where the material is sampled; the bound is checked at
        // the node points.
        let spatial = law(
            FieldLaw::Saturable {
                chi: ScalarField::formula("-2*x").unwrap(),
                saturation: constant(1.0),
            },
            false,
        );
        assert!(spatial.valid(&[]));
        assert!(spatial.varies_in_space());
        assert_eq!(spatial.constant_values(&[]).unwrap(), None);
        let at = spatial
            .evaluate_at(
                MaterialCoordinates {
                    x: 0.25,
                    y: 0.0,
                    r: 0.25,
                    theta: 0.0,
                },
                &[],
            )
            .unwrap();
        assert_eq!(
            at.field,
            FieldLawValues::Saturable {
                chi: -0.5,
                saturation: 1.0
            }
        );
    }

    #[test]
    fn renaming_a_parameter_reaches_every_law_coefficient() {
        let law = CoefficientLaw {
            field: FieldLaw::Saturable {
                chi: ScalarField::formula("chi").unwrap(),
                saturation: ScalarField::formula("u_sat").unwrap(),
            },
            drive: TimeDrive::ParametricPump {
                depth: ScalarField::formula("chi / 2").unwrap(),
                frequency_hz: constant(2.0),
                phase_radians: constant(0.0),
            },
            alternate: Some(ScalarField::formula("1 + chi").unwrap()),
            inverted: false,
        };
        let renamed = law.rename_parameter("chi", "kappa").unwrap();
        let names: Vec<&str> = renamed.parameter_names().collect();
        assert_eq!(names, ["kappa", "u_sat", "kappa", "kappa"]);
        assert!(renamed.valid(&[
            MaterialParameter {
                name: "kappa".into(),
                value: 0.2
            },
            MaterialParameter {
                name: "u_sat".into(),
                value: 1.0
            },
        ]));
    }

    #[test]
    fn material_parameter_edits_cover_loss_and_reserved_oscillator_fields_atomically() {
        let mut material = Material::default_medium();
        material.parameters = vec![
            MaterialParameter {
                name: "p".into(),
                value: 0.2,
            },
            MaterialParameter {
                name: "unused".into(),
                value: 1.0,
            },
        ];
        material.mass_density = ScalarField::formula("1 + p").unwrap();
        material.mass_law.field = FieldLaw::Saturable {
            chi: ScalarField::formula("p").unwrap(),
            saturation: constant(1.0),
        };
        material.electric_loss = Some(LossChannel {
            base_rate: ScalarField::formula("p").unwrap(),
            law: DampingLaw {
                rate: RateLaw::Polynomial {
                    beta1: ScalarField::formula("p").unwrap(),
                    beta2: constant(1.0),
                    amplitude_bound: Some(constant(0.1)),
                },
                drive: TimeDrive::None,
            },
        });
        material.restoring = RestoringLaw::KleinGordon {
            omega0: ScalarField::formula("p").unwrap(),
        };

        let before = material.clone();
        assert_eq!(
            material.rename_parameter(0, "x".into()),
            Err(MaterialError::InvalidValue)
        );
        assert_eq!(material, before, "a rejected rename must change nothing");

        material.rename_parameter(0, "gain".into()).unwrap();
        assert!(material.parameter_names().all(|name| name != "p"));
        assert!(material.parameter_names().any(|name| name == "gain"));
        assert!(material.remove_parameter(0).is_err());
        assert_eq!(material.remove_parameter(1).unwrap().name, "unused");
        assert!(material.valid());
    }

    #[test]
    fn volume_source_parameter_edits_use_their_own_complete_namespace() {
        let mut source = crate::VolumeSource {
            region: crate::BACKGROUND_REGION,
            enabled: true,
            profile: ScalarField::formula("exp(-r*r/width)").unwrap(),
            parameters: vec![
                MaterialParameter {
                    name: "width".into(),
                    value: 0.2,
                },
                MaterialParameter {
                    name: "unused".into(),
                    value: 1.0,
                },
            ],
            signal: crate::TimeSignal::harmonic(0.0, 1.0, 2.0, 0.0),
        };
        source.rename_parameter(0, "waist".into()).unwrap();
        assert_eq!(source.parameter_names().collect::<Vec<_>>(), ["waist"]);
        assert!(source.remove_parameter(0).is_err());
        assert_eq!(source.remove_parameter(1).unwrap().name, "unused");
        assert!(source.valid());
    }

    #[test]
    fn restoring_laws_report_their_curvature_bound() {
        let none = &[][..];
        assert!(
            RestoringLaw::KleinGordon {
                omega0: constant(4.0)
            }
            .valid(none)
        );
        assert!(
            !RestoringLaw::KleinGordon {
                omega0: constant(-1.0)
            }
            .valid(none)
        );
        assert!(
            !RestoringLaw::Phi4 {
                lambda: constant(1.0),
                amplitude_bound: constant(0.0),
            }
            .valid(none)
        );
        let phi4 = RestoringLawValues::Phi4 {
            lambda: 2.0,
            amplitude_bound: 1.5,
        };
        assert!((phi4.curvature_bound() - 2.0 * (3.0 * 2.25 - 1.0)).abs() < 1.0e-12);
        assert!((phi4.slope(1.0)).abs() < 1.0e-12);
        assert!(
            (RestoringLawValues::SineGordon { omega0: 3.0 }.slope(std::f64::consts::FRAC_PI_2)
                - 9.0)
                .abs()
                < 1.0e-12
        );
    }

    #[test]
    fn the_drive_multiplier_has_the_promised_shape() {
        let pump = TimeDriveValues::ParametricPump {
            depth: 0.3,
            frequency_hz: 1.0,
            phase_radians: 0.0,
        };
        assert!((pump.multiplier(0.0, origin()) - 1.3).abs() < 1.0e-12);
        assert!((pump.multiplier(0.25, origin()) - 1.0).abs() < 1.0e-12);
        assert_eq!(pump.range(), (0.7, 1.3));
        let crystal = TimeDriveValues::TimeCrystal {
            depth: 0.3,
            frequency_hz: 1.0,
            phase_radians: 0.0,
            sharpness: 8.0,
        };
        assert!((crystal.multiplier(0.0, origin()) - 1.3).abs() < 1.0e-12);
        assert!((crystal.multiplier(0.5, origin()) - 0.7).abs() < 1.0e-12);
        let cosine_limit = TimeDriveValues::TimeCrystal {
            depth: 0.3,
            frequency_hz: 1.0,
            phase_radians: 0.0,
            sharpness: 1.0e-12,
        };
        assert!((cosine_limit.multiplier(0.25, origin()) - 1.0).abs() < 1.0e-12);
        let travelling = TimeDriveValues::TravellingModulation {
            depth: 0.3,
            frequency_hz: 1.0,
            phase_radians: 0.0,
            wavenumber: std::f64::consts::TAU,
            angle_radians: 0.0,
        };
        // One wavelength along x brings the phase back.
        let at = |x: f64| {
            travelling.multiplier(
                0.1,
                MaterialCoordinates {
                    x,
                    y: 0.0,
                    r: x,
                    theta: 0.0,
                },
            )
        };
        assert!((at(0.0) - at(1.0)).abs() < 1.0e-12);
        assert!((at(0.0) - at(0.5)).abs() > 0.1);
    }

    #[test]
    fn travelling_drives_require_the_material_frame_even_with_constant_parameters() {
        let drive = TimeDrive::TravellingModulation {
            depth: constant(0.2),
            frequency_hz: constant(1.0),
            phase_radians: constant(0.0),
            wavenumber: constant(2.0),
            angle_radians: constant(0.0),
        };
        assert!(drive.uses_frame());
        assert!(
            !drive
                .fields()
                .iter()
                .any(|field| !field.spatially_constant())
        );
        let mut material = Material::default_medium();
        material.mass_law.drive = drive;
        assert!(material.uses_frame());
    }

    #[test]
    fn reciprocal_identity_normalizes_to_the_inert_law() {
        let reciprocal = CoefficientLaw::linear().through_reciprocal();
        assert_eq!(reciprocal, CoefficientLaw::linear());
        let mut encoded_identity = CoefficientLaw::linear();
        encoded_identity.inverted = true;
        assert!(encoded_identity.is_linear());
        assert_eq!(encoded_identity.normalized(), CoefficientLaw::linear());
    }

    #[test]
    fn passive_polynomial_rates_are_checked_analytically() {
        let passive = RateLaw::Polynomial {
            beta1: constant(0.5),
            beta2: constant(0.25),
            amplitude_bound: None,
        };
        assert!(passive.valid(&[]));
        let active_unbounded = RateLaw::Polynomial {
            beta1: constant(2.1),
            beta2: constant(1.0),
            amplitude_bound: None,
        };
        assert!(!active_unbounded.valid(&[]));
        let bounded = RateLaw::Polynomial {
            beta1: constant(2.1),
            beta2: constant(1.0),
            amplitude_bound: Some(constant(0.2)),
        };
        assert!(bounded.valid(&[]));
    }

    #[test]
    fn coefficient_roles_keep_physical_arguments_across_polarizations() {
        let material = Material::default_medium();
        let tm = coefficient_law_roles(
            &material,
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            },
        );
        let te = coefficient_law_roles(
            &material,
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            },
        );
        assert_eq!(tm[0].argument, PhysicalFieldArgument::Electric);
        assert_eq!(te[0].argument, PhysicalFieldArgument::Electric);
        assert_eq!(tm[0].placement, ConstitutivePlacement::PrimaryScalar);
        assert_eq!(te[0].placement, ConstitutivePlacement::ComplementaryVector);
        assert_eq!(tm[1].argument, PhysicalFieldArgument::Magnetic);
        assert_eq!(te[1].argument, PhysicalFieldArgument::Magnetic);
    }

    #[test]
    fn effective_law_lines_spell_out_the_product() {
        let mut material = Material::default_medium();
        material.parameters = parameters();
        material.mass_law = CoefficientLaw {
            field: FieldLaw::Saturable {
                chi: ScalarField::formula("chi").unwrap(),
                saturation: ScalarField::formula("u_sat").unwrap(),
            },
            drive: TimeDrive::ParametricPump {
                depth: constant(0.15),
                frequency_hz: constant(6.0),
                phase_radians: constant(0.0),
            },
            alternate: Some(constant(1.6)),
            inverted: false,
        };
        material.restoring = RestoringLaw::SineGordon {
            omega0: constant(4.0),
        };
        let tm = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        };
        assert_eq!(
            effective_law_lines(&material, tm, LawTextStyle::Names),
            [
                "ε(E, t) = ε₀ · (1 + 0.15·cos(2π·6·t)) · (1 + chi·E² / (1 + E²/u_sat²)) — switched: ×1.6",
                "Reserved oscillator (not executable): 4²·sin(E)",
            ]
        );
        assert_eq!(
            effective_law_lines(&material, PhysicsModel::Mechanical, LawTextStyle::Numbers)[0],
            "ρ(u, t) = ρ₀ · (1 + 0.15·cos(2π·6·t)) · (1 + 0.2·u² / (1 + u²/1²)) — switched: ×1.6"
        );
        material.mass_law.inverted = true;
        material.mass_law.drive = TimeDrive::None;
        let te = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        };
        assert_eq!(
            effective_law_lines(&material, te, LawTextStyle::Numbers)[0],
            "ε(E) = ε₀ / (1 + 0.2·E² / (1 + E²/1²)) — switched: ÷1.6"
        );
        assert!(
            effective_law_lines(&Material::default_medium(), tm, LawTextStyle::Names).is_empty()
        );
    }

    #[test]
    fn loss_text_keeps_electric_and_magnetic_arguments_in_te() {
        let mut material = Material::default_medium();
        material.electric_loss = Some(LossChannel {
            base_rate: constant(0.2),
            law: DampingLaw {
                rate: RateLaw::SaturableAbsorption {
                    saturation: constant(1.5),
                },
                drive: TimeDrive::None,
            },
        });
        material.magnetic_loss = Some(LossChannel {
            base_rate: constant(0.1),
            law: DampingLaw::constant(),
        });
        let te = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        };
        assert_eq!(
            effective_law_lines(&material, te, LawTextStyle::Names),
            ["γ_E(E) = 0.2 · 1 / (1 + E²/1.5²)", "γ_H = 0.1",]
        );
    }

    #[test]
    fn the_legacy_material_evaluator_refuses_authored_laws() {
        let mut material = Material::default_medium();
        material.mass_law.field = saturable(0.2, 1.0);
        assert_eq!(
            material.evaluate(crate::MaterialFrame::world(), crate::Point2::default()),
            Err(MaterialError::UnsupportedMaterialLaw)
        );
        assert_eq!(material.uniform(), None);
    }

    #[test]
    fn numbers_print_the_way_a_person_writes_them() {
        assert_eq!(number_text(6.0), "6");
        assert_eq!(number_text(0.15), "0.15");
        assert_eq!(number_text(1.0 / 3.0), "0.3333");
        assert_eq!(number_text(-0.0), "0");
        assert_eq!(number_text(0.00001), "0");
    }
}
