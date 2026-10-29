use crate::{
    CoefficientLaw, EvaluatedMaterial, LossChannel, MAX_MATERIAL_PARAMETERS, MaterialError,
    MaterialFrame, MaterialParameter, Point2, RestoringLaw, ScalarField, TimeSignal,
    reserved_identifier, valid_identifier,
};
pub const MAX_MATERIALS: usize = 32;
/// The most volume sources a scene holds: one for each of the retired
/// `Scene`'s 32 obstacles and one for the background. Files written under the
/// limit must keep opening, so it stays.
pub const MAX_VOLUME_SOURCES: usize = 33;
pub const WORLD_TOLERANCE: f64 = 2.0e-4;
pub const MIN_DOMAIN_EXTENT: f64 = 0.1;
pub const MAX_DOMAIN_EXTENT: f64 = 100.0;
pub const BACKGROUND_REGION: RegionId = RegionId(1);
pub const DEFAULT_MATERIAL: MaterialId = MaterialId(1);

/// Axis-aligned simulated domain. Coordinates and extents are world-space values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DomainRect {
    pub min_x: f64,
    pub max_x: f64,
    pub min_y: f64,
    pub max_y: f64,
}

impl DomainRect {
    pub const UNIT: Self = Self {
        min_x: -1.0,
        max_x: 1.0,
        min_y: -1.0,
        max_y: 1.0,
    };

    pub const fn new(min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> Self {
        Self {
            min_x,
            max_x,
            min_y,
            max_y,
        }
    }

    pub fn valid(self) -> bool {
        let width = self.width();
        let height = self.height();
        [self.min_x, self.max_x, self.min_y, self.max_y]
            .into_iter()
            .all(f64::is_finite)
            && (MIN_DOMAIN_EXTENT..=MAX_DOMAIN_EXTENT).contains(&width)
            && (MIN_DOMAIN_EXTENT..=MAX_DOMAIN_EXTENT).contains(&height)
    }

    pub fn width(self) -> f64 {
        self.max_x - self.min_x
    }
    pub fn height(self) -> f64 {
        self.max_y - self.min_y
    }
    pub fn center(self) -> Point2 {
        Point2::new(
            (self.min_x + self.max_x) * 0.5,
            (self.min_y + self.max_y) * 0.5,
        )
    }
    pub fn minimum_extent(self) -> f64 {
        self.width().min(self.height())
    }
    pub fn tolerance(self) -> f64 {
        self.width().max(self.height()) * 1.0e-4
    }
    pub fn contains_with_margin(self, point: Point2, margin: f64) -> bool {
        point.x > self.min_x + margin
            && point.x < self.max_x - margin
            && point.y > self.min_y + margin
            && point.y < self.max_y - margin
    }
    pub fn corners(self) -> [Point2; 4] {
        [
            Point2::new(self.min_x, self.min_y),
            Point2::new(self.max_x, self.min_y),
            Point2::new(self.max_x, self.max_y),
            Point2::new(self.min_x, self.max_y),
        ]
    }
}

impl Default for DomainRect {
    fn default() -> Self {
        Self::UNIT
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegionId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MaterialId(pub u64);

/// The largest short-wave loss: past it the explicit viscous step would
/// overshoot at the mesh's ceiling.
pub const MAX_SHORT_WAVE_LOSS: f64 = 1.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub id: MaterialId,
    pub name: String,
    pub mass_density: ScalarField,
    pub stiffness: ScalarField,
    pub axis_ratio: ScalarField,
    pub parameters: Vec<MaterialParameter>,
    pub color: [u8; 3],
    /// What the density row (ε in the EM skins) does beyond its base value.
    pub mass_law: CoefficientLaw,
    /// What the physical complementary coefficient does beyond its base value:
    /// reciprocal stiffness s₀ in Mechanical, μ in the EM skins.
    pub stiffness_law: CoefficientLaw,
    /// Electric flux-rate loss, named by physical field rather than solver role.
    pub electric_loss: Option<LossChannel>,
    /// Magnetic flux-rate loss, named by physical field rather than solver role.
    pub magnetic_loss: Option<LossChannel>,
    /// An acceleration taken away at every node.
    pub restoring: RestoringLaw,
    /// Seconds a Switch takes to cross from the base factor to the alternate;
    /// zero is a hard temporal interface.
    pub switch_ramp: f64,
    /// The short-wave loss α, from 0 (off) to 1: a viscous stress on the
    /// field's gradient, `τ η C u` with `τ = α/(h G)` on each element, `G` the
    /// element's own ceiling and `h` the generation's largest step. A mode at
    /// that ceiling decays at about `α/(2h)` and a resolved wave by the square
    /// of its share of it, so it trims mesh-scale residue and spares the waves
    /// the mesh carries. Gate O's van der Pol limit is the same stress.
    pub short_wave_loss: f64,
}

impl Material {
    /// Whether this material is a plain linear, time-invariant wave medium:
    /// no field law, no drive and no Switch alternate on either constitutive
    /// row, no drive and no field-dependent rate on either loss channel, no
    /// restoring law and no short-wave loss. Consumers whose derivation
    /// assumes such a medium (the fixed solver path, the far field's
    /// free-space projection) test this rather than inspecting each slot. A
    /// restoring law is time invariant, but it is not this medium: its
    /// integrated field is state the fixed path does not carry, and
    /// Klein-Gordon has no wave-equation Green's function. Nor is a short-wave
    /// loss, which only the time-driven step carries and which damps each
    /// wavelength by its own rate.
    pub fn time_invariant(&self) -> bool {
        let channel_is_fixed = |channel: &Option<LossChannel>| {
            channel.as_ref().is_none_or(|channel| {
                channel.law.drive.is_none() && matches!(channel.law.rate, crate::RateLaw::Constant)
            })
        };
        self.mass_law.is_linear()
            && self.stiffness_law.is_linear()
            && channel_is_fixed(&self.electric_loss)
            && channel_is_fixed(&self.magnetic_loss)
            && self.restoring.is_none()
            && self.short_wave_loss == 0.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VolumeSource {
    pub region: RegionId,
    pub enabled: bool,
    pub profile: ScalarField,
    pub parameters: Vec<MaterialParameter>,
    pub signal: TimeSignal,
}

impl VolumeSource {
    pub fn parameter_names(&self) -> impl Iterator<Item = &str> {
        self.profile.parameter_names()
    }

    pub fn valid(&self) -> bool {
        let unique_parameters = self.parameters.len() <= MAX_MATERIAL_PARAMETERS
            && self
                .parameters
                .iter()
                .enumerate()
                .all(|(index, parameter)| {
                    parameter.valid()
                        && !self.parameters[..index]
                            .iter()
                            .any(|previous| previous.name == parameter.name)
                });
        let references_exist = self.parameter_names().all(|name| {
            self.parameters
                .iter()
                .any(|parameter| parameter.name == name)
        });
        self.region.0 > 0
            && unique_parameters
            && references_exist
            && self.signal.valid()
            && self
                .profile
                .constant_value()
                .is_none_or(|value| value.is_finite())
    }

    pub fn varying(&self) -> bool {
        self.profile.constant_value().is_none()
    }

    pub fn evaluate(&self, frame: MaterialFrame, point: Point2) -> Result<f64, MaterialError> {
        self.profile
            .evaluate(frame.coordinates(point), &self.parameters)
    }

    pub fn rename_parameter(&mut self, index: usize, name: String) -> Result<(), MaterialError> {
        if index >= self.parameters.len()
            || !valid_identifier(&name)
            || reserved_identifier(&name)
            || self
                .parameters
                .iter()
                .enumerate()
                .any(|(other, parameter)| other != index && parameter.name == name)
        {
            return Err(MaterialError::InvalidValue);
        }
        let old = self.parameters[index].name.clone();
        self.profile = self.profile.rename_parameter(&old, &name)?;
        self.parameters[index].name = name;
        Ok(())
    }

    pub fn remove_parameter(&mut self, index: usize) -> Result<MaterialParameter, MaterialError> {
        let parameter = self
            .parameters
            .get(index)
            .ok_or(MaterialError::InvalidValue)?;
        if self.parameter_names().any(|name| name == parameter.name) {
            return Err(MaterialError::InvalidValue);
        }
        Ok(self.parameters.remove(index))
    }
}

impl Material {
    pub fn default_medium() -> Self {
        Self {
            id: DEFAULT_MATERIAL,
            name: "Ambient".into(),
            mass_density: ScalarField::constant(1.0),
            stiffness: ScalarField::constant(1.0),
            axis_ratio: ScalarField::constant(1.0),
            parameters: vec![],
            // Readable against the dark canvas at the overlay's default opacity
            // while staying calmer than any assigned material.
            color: [86, 116, 138],
            mass_law: CoefficientLaw::linear(),
            stiffness_law: CoefficientLaw::linear(),
            electric_loss: None,
            magnetic_loss: None,
            restoring: RestoringLaw::None,
            switch_ramp: 0.0,
            short_wave_loss: 0.0,
        }
    }

    /// Every parameter any coefficient or law of the material refers to.
    pub fn parameter_names(&self) -> impl Iterator<Item = &str> {
        [&self.mass_density, &self.stiffness, &self.axis_ratio]
            .into_iter()
            .flat_map(ScalarField::parameter_names)
            .chain(self.mass_law.parameter_names())
            .chain(self.stiffness_law.parameter_names())
            .chain(
                self.electric_loss
                    .iter()
                    .flat_map(LossChannel::parameter_names),
            )
            .chain(
                self.magnetic_loss
                    .iter()
                    .flat_map(LossChannel::parameter_names),
            )
            .chain(self.restoring.parameter_names())
    }

    /// Whether any law changes what the material does beyond its base values.
    pub fn has_laws(&self) -> bool {
        !self.mass_law.is_linear()
            || !self.stiffness_law.is_linear()
            || self.electric_loss.is_some()
            || self.magnetic_loss.is_some()
            || !self.restoring.is_none()
    }

    pub fn valid(&self) -> bool {
        let unique_parameters = self.parameters.len() <= MAX_MATERIAL_PARAMETERS
            && self
                .parameters
                .iter()
                .enumerate()
                .all(|(index, parameter)| {
                    parameter.valid()
                        && !self.parameters[..index]
                            .iter()
                            .any(|previous| previous.name == parameter.name)
                });
        let references_exist = self.parameter_names().all(|name| {
            self.parameters
                .iter()
                .any(|parameter| parameter.name == name)
        });
        self.id.0 > 0
            && !self.name.trim().is_empty()
            && self.name.len() <= 64
            && unique_parameters
            && references_exist
            && self.mass_law.valid(&self.parameters)
            && self.stiffness_law.valid(&self.parameters)
            && self
                .electric_loss
                .as_ref()
                .is_none_or(|loss| loss.valid(&self.parameters))
            && self
                .magnetic_loss
                .as_ref()
                .is_none_or(|loss| loss.valid(&self.parameters))
            && self.restoring.valid(&self.parameters)
            && self.switch_ramp.is_finite()
            && self.switch_ramp >= 0.0
            && (0.0..=MAX_SHORT_WAVE_LOSS).contains(&self.short_wave_loss)
            && self
                .mass_density
                .constant_value()
                .is_none_or(|value| value.is_finite() && value > 0.0)
            && self
                .stiffness
                .constant_value()
                .is_none_or(|value| value.is_finite() && value > 0.0)
            && self
                .axis_ratio
                .constant_value()
                .is_none_or(|value| value.is_finite() && value >= 1.0)
    }

    pub fn varying(&self) -> bool {
        [&self.mass_density, &self.stiffness, &self.axis_ratio]
            .into_iter()
            .any(|field| field.constant_value().is_none())
            || self.mass_law.varies_in_space()
            || self.stiffness_law.varies_in_space()
            || self
                .electric_loss
                .as_ref()
                .is_some_and(LossChannel::varies_in_space)
            || self
                .magnetic_loss
                .as_ref()
                .is_some_and(LossChannel::varies_in_space)
            || self.restoring.varies_in_space()
    }

    pub fn uses_frame(&self) -> bool {
        self.varying()
            || self.axis_ratio.constant_value() != Some(1.0)
            || self.mass_law.uses_frame()
            || self.stiffness_law.uses_frame()
            || self
                .electric_loss
                .as_ref()
                .is_some_and(LossChannel::uses_frame)
            || self
                .magnetic_loss
                .as_ref()
                .is_some_and(LossChannel::uses_frame)
    }

    pub fn evaluate(
        &self,
        frame: MaterialFrame,
        point: Point2,
    ) -> Result<EvaluatedMaterial, MaterialError> {
        if self.has_laws() {
            return Err(MaterialError::UnsupportedMaterialLaw);
        }
        self.evaluate_base(frame, point)
    }

    /// The coefficients a static scalar operator sees.
    ///
    /// Like [`Self::evaluate`], but a constant named loss channel is not a
    /// law it has to refuse: it is a rate, and the channel on the field the
    /// skin makes primary - TM's electric loss, TE's and Mechanical's
    /// magnetic loss - is the scalar operator's `damping`. The complementary
    /// channel has no place in a scalar operator and is left to the canonical
    /// solver, which applies both.
    pub fn evaluate_static(
        &self,
        physics: crate::PhysicsModel,
        frame: MaterialFrame,
        point: Point2,
    ) -> Result<EvaluatedMaterial, MaterialError> {
        let constant_loss_only = self.mass_law.is_linear()
            && self.stiffness_law.is_linear()
            && self.restoring.is_none()
            && [&self.electric_loss, &self.magnetic_loss]
                .into_iter()
                .flatten()
                .all(|channel| channel.law.is_constant());
        if !constant_loss_only {
            return Err(MaterialError::UnsupportedMaterialLaw);
        }
        let mut values = self.evaluate_base(frame, point)?;
        if self.electric_loss.is_none() && self.magnetic_loss.is_none() {
            return Ok(values);
        }
        let primary = match physics {
            crate::PhysicsModel::Electromagnetic {
                polarization: crate::ElectromagneticPolarization::Tm,
            } => &self.electric_loss,
            _ => &self.magnetic_loss,
        };
        if let Some(channel) = primary {
            let rate = channel
                .base_rate
                .evaluate(frame.coordinates(point), &self.parameters)?;
            if !rate.is_finite() || rate < 0.0 {
                return Err(MaterialError::InvalidValue);
            }
            values.damping = rate;
        }
        Ok(values)
    }

    /// The base coefficients, with any authored law left unapplied.
    ///
    /// [`Self::evaluate`] refuses a law-carrying material on purpose, so that
    /// nothing executes an authored law as a static medium by accident. A
    /// consumer that applies the law itself needs the base underneath it, and
    /// declares that by calling this instead.
    pub fn evaluate_base(
        &self,
        frame: MaterialFrame,
        point: Point2,
    ) -> Result<EvaluatedMaterial, MaterialError> {
        let coordinates = frame.coordinates(point);
        let values = EvaluatedMaterial {
            mass_density: self.mass_density.evaluate(coordinates, &self.parameters)?,
            stiffness: self.stiffness.evaluate(coordinates, &self.parameters)?,
            // The primary field's loss rate is the primary channel's, which
            // the base leaves out and `evaluate_static` adds.
            damping: 0.0,
            axis_ratio: self.axis_ratio.evaluate(coordinates, &self.parameters)?,
        };
        values
            .valid()
            .then_some(values)
            .ok_or(MaterialError::InvalidValue)
    }

    pub fn uniform(&self) -> Option<EvaluatedMaterial> {
        if self.has_laws() {
            return None;
        }
        let values = EvaluatedMaterial {
            mass_density: self.mass_density.constant_value()?,
            stiffness: self.stiffness.constant_value()?,
            damping: 0.0,
            axis_ratio: self.axis_ratio.constant_value()?,
        };
        values.valid().then_some(values)
    }

    pub fn rename_parameter(&mut self, index: usize, name: String) -> Result<(), MaterialError> {
        if index >= self.parameters.len()
            || !valid_identifier(&name)
            || reserved_identifier(&name)
            || self
                .parameters
                .iter()
                .enumerate()
                .any(|(other, parameter)| other != index && parameter.name == name)
        {
            return Err(MaterialError::InvalidValue);
        }
        let old = self.parameters[index].name.clone();
        let mass_density = self.mass_density.rename_parameter(&old, &name)?;
        let stiffness = self.stiffness.rename_parameter(&old, &name)?;
        let axis_ratio = self.axis_ratio.rename_parameter(&old, &name)?;
        let mass_law = self.mass_law.rename_parameter(&old, &name)?;
        let stiffness_law = self.stiffness_law.rename_parameter(&old, &name)?;
        let electric_loss = self
            .electric_loss
            .as_ref()
            .map(|loss| loss.rename_parameter(&old, &name))
            .transpose()?;
        let magnetic_loss = self
            .magnetic_loss
            .as_ref()
            .map(|loss| loss.rename_parameter(&old, &name))
            .transpose()?;
        let restoring = self.restoring.rename_parameter(&old, &name)?;
        self.mass_density = mass_density;
        self.stiffness = stiffness;
        self.axis_ratio = axis_ratio;
        self.mass_law = mass_law;
        self.stiffness_law = stiffness_law;
        self.electric_loss = electric_loss;
        self.magnetic_loss = magnetic_loss;
        self.restoring = restoring;
        self.parameters[index].name = name;
        Ok(())
    }

    pub fn remove_parameter(&mut self, index: usize) -> Result<MaterialParameter, MaterialError> {
        let parameter = self
            .parameters
            .get(index)
            .ok_or(MaterialError::InvalidValue)?;
        if self.parameter_names().any(|name| name == parameter.name) {
            return Err(MaterialError::InvalidValue);
        }
        Ok(self.parameters.remove(index))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub id: RegionId,
    pub material: MaterialId,
    pub frame: MaterialFrame,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FaceBoundaryCondition {
    Reflecting,
    /// Local absorbing condition scaled by the adjacent material's characteristic
    /// impedance. A ratio of one is the matched first-order condition.
    Impedance {
        ratio: f64,
    },
    /// Second-order local outgoing condition with a tangential auxiliary field.
    SecondOrderOutgoing,
    ElectricWall,
    MagneticWall,
    /// Prescribed outward flux `stiffness * partial_n u = value(t)`.
    Neumann {
        signal: TimeSignal,
    },
    /// Strongly prescribed displacement `u = value(t)`.
    Dirichlet {
        signal: TimeSignal,
    },
}

impl FaceBoundaryCondition {
    pub fn label(self) -> &'static str {
        match self {
            Self::Reflecting => "Reflecting",
            Self::Impedance { .. } => "First-order outgoing",
            Self::SecondOrderOutgoing => "Second-order outgoing",
            Self::ElectricWall => "Electric wall",
            Self::MagneticWall => "Magnetic wall",
            Self::Neumann { .. } => "Prescribed Neumann",
            Self::Dirichlet { .. } => "Prescribed Dirichlet",
        }
    }

    pub fn signal(self) -> Option<TimeSignal> {
        match self {
            Self::Neumann { signal } | Self::Dirichlet { signal } => Some(signal),
            _ => None,
        }
    }

    pub fn valid(self) -> bool {
        match self {
            Self::Reflecting
            | Self::SecondOrderOutgoing
            | Self::ElectricWall
            | Self::MagneticWall => true,
            Self::Impedance { ratio } => ratio.is_finite() && ratio > 0.0,
            Self::Neumann { signal } | Self::Dirichlet { signal } => signal.valid(),
        }
    }

    pub fn resolved(self, physics: crate::PhysicsModel) -> Self {
        match (self, physics) {
            (
                Self::ElectricWall,
                crate::PhysicsModel::Electromagnetic {
                    polarization: crate::ElectromagneticPolarization::Tm,
                },
            )
            | (
                Self::MagneticWall,
                crate::PhysicsModel::Electromagnetic {
                    polarization: crate::ElectromagneticPolarization::Te,
                },
            )
            | (Self::ElectricWall, crate::PhysicsModel::Mechanical) => Self::Dirichlet {
                signal: TimeSignal::ZERO,
            },
            (Self::ElectricWall | Self::MagneticWall, _) => Self::Reflecting,
            _ => self,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InternalBoundaryCoupling {
    Independent,
    /// Conservative zero-thickness compliant layer. The coefficient is scaled
    /// by the adjacent material stiffness and couples the two trace jumps.
    ThinGap {
        stiffness_ratio: f64,
    },
}

impl InternalBoundaryCoupling {
    pub fn valid(self) -> bool {
        match self {
            Self::Independent => true,
            Self::ThinGap { stiffness_ratio } => {
                stiffness_ratio.is_finite() && stiffness_ratio > 0.0
            }
        }
    }
}
