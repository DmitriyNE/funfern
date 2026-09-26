use crate::{
    CoefficientLaw, EvaluatedMaterial, LossChannel, MAX_MATERIAL_PARAMETERS, MaterialError,
    MaterialFrame, MaterialFrameAttachment, MaterialParameter, OpenCubicSpline, OpenSampler,
    PeriodicCubicSpline, Point2, RestoringLaw, Sample, Sampler, SamplingOptions, ScalarField,
    SplineError, TimeSignal, point_segment_distance, reserved_identifier, valid_identifier,
};
use std::collections::{BTreeMap, BTreeSet};
pub const MAX_OBSTACLES: usize = 32;
pub const MAX_INTERNAL_BOUNDARIES: usize = 32;
pub const MAX_MATERIAL_INTERFACES: usize = 32;
pub const MAX_JUNCTIONS: usize = 64;
pub const MAX_MATERIALS: usize = 32;
pub const MAX_VOLUME_SOURCES: usize = MAX_OBSTACLES + 1;
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObstacleId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InternalBoundaryId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MaterialInterfaceId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InterfaceNodeId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JunctionId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegionId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MaterialId(pub u64);

#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub id: MaterialId,
    pub name: String,
    pub mass_density: ScalarField,
    pub stiffness: ScalarField,
    pub damping: ScalarField,
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
}

impl Material {
    /// Whether this material is a plain linear, time-invariant wave medium:
    /// no field law, no drive and no Switch alternate on either constitutive
    /// row, no drive and no field-dependent rate on either loss channel, and
    /// no restoring law. Consumers whose derivation assumes such a medium
    /// (the fixed solver path, the far field's free-space projection) test
    /// this rather than inspecting each slot. A restoring law is time
    /// invariant, but it is not this medium: its integrated field is state
    /// the fixed path does not carry, and Klein-Gordon has no wave-equation
    /// Green's function.
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
            name: "Background".into(),
            mass_density: ScalarField::constant(1.0),
            stiffness: ScalarField::constant(1.0),
            damping: ScalarField::constant(0.0),
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
        }
    }

    /// Every parameter any coefficient or law of the material refers to.
    pub fn parameter_names(&self) -> impl Iterator<Item = &str> {
        [
            &self.mass_density,
            &self.stiffness,
            &self.damping,
            &self.axis_ratio,
        ]
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
            && self
                .mass_density
                .constant_value()
                .is_none_or(|value| value.is_finite() && value > 0.0)
            && self
                .stiffness
                .constant_value()
                .is_none_or(|value| value.is_finite() && value > 0.0)
            && self
                .damping
                .constant_value()
                .is_none_or(|value| value.is_finite() && value >= 0.0)
            && self
                .axis_ratio
                .constant_value()
                .is_none_or(|value| value.is_finite() && value >= 1.0)
    }

    pub fn varying(&self) -> bool {
        [
            &self.mass_density,
            &self.stiffness,
            &self.damping,
            &self.axis_ratio,
        ]
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
    /// skin makes primary is exactly what legacy `damping` means - TM's
    /// electric loss, TE's and Mechanical's magnetic loss. That channel's rate
    /// stands in for `damping`. The complementary channel has no place in a
    /// scalar operator and is left to the canonical solver, which applies
    /// both. Legacy damping beside a named channel is refused, as the
    /// canonical compiler refuses it.
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
        if values.damping != 0.0 {
            return Err(MaterialError::InvalidValue);
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
            damping: self.damping.evaluate(coordinates, &self.parameters)?,
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
            damping: self.damping.constant_value()?,
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
        let damping = self.damping.rename_parameter(&old, &name)?;
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
        self.damping = damping;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopRole {
    Hole {
        exterior: RegionId,
    },
    #[doc(hidden)]
    MaterialInterface {
        exterior: RegionId,
        interior: RegionId,
    },
    Wall {
        exterior: RegionId,
        interior: RegionId,
    },
}

impl LoopRole {
    pub fn exterior(self) -> RegionId {
        match self {
            Self::Hole { exterior }
            | Self::MaterialInterface { exterior, .. }
            | Self::Wall { exterior, .. } => exterior,
        }
    }

    pub fn interior(self) -> Option<RegionId> {
        match self {
            Self::Hole { .. } => None,
            Self::MaterialInterface { interior, .. } | Self::Wall { interior, .. } => {
                Some(interior)
            }
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Hole { .. } => "Hole",
            Self::MaterialInterface { .. } => "Material interface",
            Self::Wall { .. } => "Two-sided wall",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Obstacle {
    pub id: ObstacleId,
    pub spline: PeriodicCubicSpline,
    pub role: LoopRole,
    /// One exterior-face condition for each periodic spline knot span.
    /// Conditions are currently assembled for holes; retaining the vector on
    /// every loop keeps spline edits and future closed-wall assignment uniform.
    pub span_conditions: Vec<FaceBoundaryCondition>,
}

/// The two material regions adjacent to an oriented interface span.
/// `left` and `right` are measured while traversing the spline in increasing
/// parameter order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterfaceSpanSides {
    pub left: RegionId,
    pub right: RegionId,
}

/// A stable topological point associated with one logical spline breakpoint.
/// Ordinary spline controls are not curve points and therefore cannot carry a
/// junction directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterfaceNode {
    pub id: InterfaceNodeId,
    pub junction: Option<JunctionId>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InterfaceSpline {
    Closed(PeriodicCubicSpline),
    Open(OpenCubicSpline),
}

impl InterfaceSpline {
    pub fn span_count(&self) -> usize {
        match self {
            Self::Closed(spline) => spline.intervals().len(),
            Self::Open(spline) => spline.intervals().len(),
        }
    }

    pub fn node_count(&self) -> usize {
        match self {
            Self::Closed(spline) => spline.intervals().len(),
            Self::Open(spline) => spline.intervals().len() + 1,
        }
    }

    pub fn node_point(&self, index: usize) -> Option<Point2> {
        match self {
            Self::Closed(spline) => {
                let parameter = *spline.knots().get(index)?;
                Some(spline.evaluate(parameter))
            }
            Self::Open(spline) => {
                let parameter = spline.breakpoint(index)?;
                Some(spline.evaluate(parameter))
            }
        }
    }

    pub fn node_parameter(&self, index: usize) -> Option<f64> {
        match self {
            Self::Closed(spline) => spline.knots().get(index).copied(),
            Self::Open(spline) => spline.breakpoint(index),
        }
    }

    pub fn continuity(&self, index: usize) -> Option<u8> {
        match self {
            Self::Closed(spline) => spline.continuity(index),
            Self::Open(spline) => {
                if index == 0 || index + 1 == self.node_count() {
                    Some(0)
                } else {
                    spline.continuity(index)
                }
            }
        }
    }

    pub fn is_open(&self) -> bool {
        matches!(self, Self::Open(_))
    }

    pub fn set_node_point(&mut self, index: usize, point: Point2) -> Result<(), SplineError> {
        match self {
            Self::Open(spline) => spline.set_breakpoint_point(index, point),
            Self::Closed(spline) => spline.set_breakpoint_point(index, point),
        }
    }
}

/// A user-facing transmitting curve. Region adjacency is stored per span so a
/// curve may pass through a junction where the material sector on one side
/// changes while the curve itself retains its stable identity.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialInterface {
    pub id: MaterialInterfaceId,
    pub spline: InterfaceSpline,
    pub nodes: Vec<InterfaceNode>,
    pub span_sides: Vec<InterfaceSpanSides>,
}

impl MaterialInterface {
    pub fn structure_valid(&self) -> bool {
        self.id.0 > 0
            && self.nodes.len() == self.spline.node_count()
            && self.span_sides.len() == self.spline.span_count()
            && self.nodes.iter().enumerate().all(|(index, node)| {
                node.id.0 > 0
                    && self.nodes[..index]
                        .iter()
                        .all(|previous| previous.id != node.id)
            })
            && self
                .span_sides
                .iter()
                .all(|sides| sides.left.0 > 0 && sides.right.0 > 0 && sides.left != sides.right)
    }

    pub fn node_index(&self, id: InterfaceNodeId) -> Option<usize> {
        self.nodes.iter().position(|node| node.id == id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JunctionLocation {
    Interior,
    Outer {
        side: crate::OuterSide,
        fraction: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Junction {
    pub id: JunctionId,
    pub location: JunctionLocation,
}

impl Junction {
    pub fn structure_valid(self) -> bool {
        self.id.0 > 0
            && match self.location {
                JunctionLocation::Interior => true,
                JunctionLocation::Outer { fraction, .. } => {
                    fraction.is_finite() && (0.0..=1.0).contains(&fraction)
                }
            }
    }
}

#[derive(Clone, Copy)]
struct JunctionArm {
    direction: Point2,
    left: RegionId,
    right: RegionId,
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InternalBoundaryLaw {
    pub left: FaceBoundaryCondition,
    pub right: FaceBoundaryCondition,
    pub coupling: InternalBoundaryCoupling,
}

impl InternalBoundaryLaw {
    pub const REFLECTING: Self = Self {
        left: FaceBoundaryCondition::Reflecting,
        right: FaceBoundaryCondition::Reflecting,
        coupling: InternalBoundaryCoupling::Independent,
    };

    pub fn valid(self) -> bool {
        self.left.valid()
            && self.right.valid()
            && self.coupling.valid()
            && (!matches!(self.coupling, InternalBoundaryCoupling::ThinGap { .. })
                || self.left == FaceBoundaryCondition::Reflecting
                    && self.right == FaceBoundaryCondition::Reflecting)
    }
}

impl Default for InternalBoundaryLaw {
    fn default() -> Self {
        Self::REFLECTING
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InternalBoundary {
    pub id: InternalBoundaryId,
    pub spline: OpenCubicSpline,
    pub region: RegionId,
    /// One law for each nonempty spline knot span.
    pub span_laws: Vec<InternalBoundaryLaw>,
}
impl Obstacle {
    pub fn with_role(id: ObstacleId, spline: PeriodicCubicSpline, role: LoopRole) -> Self {
        let span_conditions = vec![FaceBoundaryCondition::Reflecting; spline.intervals().len()];
        Self {
            id,
            spline,
            role,
            span_conditions,
        }
    }

    pub fn hole(id: ObstacleId, spline: PeriodicCubicSpline) -> Self {
        Self::with_role(
            id,
            spline,
            LoopRole::Hole {
                exterior: BACKGROUND_REGION,
            },
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub domain: DomainRect,
    pub physics: crate::PhysicsModel,
    pub obstacles: Vec<Obstacle>,
    pub internal_boundaries: Vec<InternalBoundary>,
    pub material_interfaces: Vec<MaterialInterface>,
    pub junctions: Vec<Junction>,
    pub materials: Vec<Material>,
    pub regions: Vec<Region>,
    pub volume_sources: Vec<VolumeSource>,
    pub outer_boundaries: crate::OuterBoundaryConditions,
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            domain: DomainRect::default(),
            physics: crate::PhysicsModel::Mechanical,
            obstacles: vec![],
            internal_boundaries: vec![],
            material_interfaces: vec![],
            junctions: vec![],
            materials: vec![Material::default_medium()],
            regions: vec![Region {
                id: BACKGROUND_REGION,
                material: DEFAULT_MATERIAL,
                frame: MaterialFrame::world(),
            }],
            volume_sources: vec![],
            outer_boundaries: crate::OuterBoundaryConditions::default(),
        }
    }
}
impl Scene {
    pub fn initial() -> Self {
        Self {
            obstacles: vec![Obstacle::hole(
                ObstacleId(1),
                PeriodicCubicSpline::rounded(Point2::default(), 0.15),
            )],
            ..Self::default()
        }
    }
    pub fn structure_valid(&self) -> bool {
        if self.obstacles.len() > MAX_OBSTACLES
            || self.internal_boundaries.len() > MAX_INTERNAL_BOUNDARIES
            || self.material_interfaces.len() > MAX_MATERIAL_INTERFACES
            || self.junctions.len() > MAX_JUNCTIONS
            || self.obstacles.len()
                + self.internal_boundaries.len()
                + self.material_interfaces.len()
                > MAX_OBSTACLES
            || self.materials.is_empty()
            || self.materials.len() > MAX_MATERIALS
            || self.regions.is_empty()
            || self.regions.len() > MAX_OBSTACLES + 1
            || self.volume_sources.len() > MAX_VOLUME_SOURCES
            || !self.domain.valid()
            || !self.outer_boundaries.valid()
        {
            return false;
        }
        let unique_obstacles = self.obstacles.iter().enumerate().all(|(i, o)| {
            o.id.0 > 0
                && o.span_conditions.len() == o.spline.intervals().len()
                && o.span_conditions.iter().all(|condition| condition.valid())
                && !self.obstacles[..i]
                    .iter()
                    .any(|previous| previous.id == o.id)
        });
        let unique_boundaries =
            self.internal_boundaries
                .iter()
                .enumerate()
                .all(|(index, boundary)| {
                    boundary.id.0 > 0
                        && self.region(boundary.region).is_some()
                        && boundary.span_laws.len() == boundary.spline.intervals().len()
                        && boundary.span_laws.iter().all(|law| law.valid())
                        && !self.internal_boundaries[..index]
                            .iter()
                            .any(|previous| previous.id == boundary.id)
                });
        let mut interface_nodes = BTreeSet::new();
        let unique_interfaces =
            self.material_interfaces
                .iter()
                .enumerate()
                .all(|(index, interface)| {
                    interface.structure_valid()
                        && interface
                            .nodes
                            .iter()
                            .all(|node| interface_nodes.insert(node.id))
                        && interface.span_sides.iter().all(|sides| {
                            self.region(sides.left).is_some() && self.region(sides.right).is_some()
                        })
                        && !self.material_interfaces[..index]
                            .iter()
                            .any(|previous| previous.id == interface.id)
                });
        let unique_junctions = self.junctions.iter().enumerate().all(|(index, junction)| {
            junction.structure_valid()
                && !self.junctions[..index]
                    .iter()
                    .any(|previous| previous.id == junction.id)
        });
        let junction_references_exist = self.material_interfaces.iter().all(|interface| {
            interface.nodes.iter().all(|node| {
                node.junction
                    .is_none_or(|id| self.junctions.iter().any(|junction| junction.id == id))
            })
        });
        let unique_materials = self.materials.iter().enumerate().all(|(i, material)| {
            material.valid()
                && !self.materials[..i]
                    .iter()
                    .any(|previous| previous.id == material.id)
        });
        let unique_regions = self.regions.iter().enumerate().all(|(i, region)| {
            region.id.0 > 0
                && self.material(region.material).is_some()
                && region.frame.valid()
                && (region.id != BACKGROUND_REGION
                    || region.frame.attachment == MaterialFrameAttachment::World)
                && !self.regions[..i]
                    .iter()
                    .any(|previous| previous.id == region.id)
        });
        let unique_sources = self
            .volume_sources
            .iter()
            .enumerate()
            .all(|(index, source)| {
                source.valid()
                    && self.region(source.region).is_some()
                    && !self.volume_sources[..index]
                        .iter()
                        .any(|previous| previous.region == source.region)
            });
        if !unique_obstacles
            || !unique_materials
            || !unique_regions
            || !unique_boundaries
            || !unique_interfaces
            || !unique_junctions
            || !junction_references_exist
            || !unique_sources
            || self.region(BACKGROUND_REGION).is_none()
        {
            return false;
        }
        for [first, second] in [[0, 1], [1, 2], [2, 3], [3, 0]] {
            if let (
                crate::OuterBoundaryCondition::Dirichlet { signal: first },
                crate::OuterBoundaryCondition::Dirichlet { signal: second },
            ) = (
                self.outer_boundaries.sides[first].resolved(self.physics),
                self.outer_boundaries.sides[second].resolved(self.physics),
            ) && first != second
            {
                return false;
            }
        }
        let mut interiors = Vec::new();
        for obstacle in &self.obstacles {
            if self.region(obstacle.role.exterior()).is_none() {
                return false;
            }
            if let Some(interior) = obstacle.role.interior() {
                if interior == BACKGROUND_REGION
                    || interior == obstacle.role.exterior()
                    || self.region(interior).is_none()
                    || interiors.contains(&interior)
                {
                    return false;
                }
                interiors.push(interior);
            }
        }
        self.regions.iter().all(|region| {
            region.id == BACKGROUND_REGION
                || interiors.contains(&region.id)
                || self.material_interfaces.iter().any(|interface| {
                    interface
                        .span_sides
                        .iter()
                        .any(|sides| sides.left == region.id || sides.right == region.id)
                })
        })
    }

    fn interface_arms_at_node(
        &self,
        interface: &MaterialInterface,
        node_index: usize,
    ) -> Option<Vec<JunctionArm>> {
        let count = interface.spline.span_count();
        let node = interface.spline.node_point(node_index)?;
        let mut arms = Vec::with_capacity(2);
        let mut add_span = |span: usize, forward: bool| {
            let bounds = match &interface.spline {
                InterfaceSpline::Closed(spline) => spline.span_bounds(span),
                InterfaceSpline::Open(spline) => spline.span_bounds(span),
            }?;
            let parameter = if forward {
                bounds[0] + (bounds[1] - bounds[0]) * 1.0e-6
            } else {
                bounds[1] - (bounds[1] - bounds[0]) * 1.0e-6
            };
            let nearby = match &interface.spline {
                InterfaceSpline::Closed(spline) => spline.evaluate(parameter),
                InterfaceSpline::Open(spline) => spline.evaluate(parameter),
            };
            let direction = nearby - node;
            if !direction.finite() || direction.norm() <= self.domain.tolerance() * 1.0e-4 {
                return None;
            }
            let sides = interface.span_sides[span];
            arms.push(if forward {
                JunctionArm {
                    direction,
                    left: sides.left,
                    right: sides.right,
                }
            } else {
                JunctionArm {
                    direction,
                    left: sides.right,
                    right: sides.left,
                }
            });
            Some(())
        };
        match interface.spline {
            InterfaceSpline::Closed(_) => {
                add_span((node_index + count - 1) % count, false)?;
                add_span(node_index, true)?;
            }
            InterfaceSpline::Open(_) => {
                if node_index > 0 {
                    add_span(node_index - 1, false)?;
                }
                if node_index < count {
                    add_span(node_index, true)?;
                }
            }
        }
        Some(arms)
    }

    pub fn junction_point(&self, junction: Junction) -> Option<Point2> {
        match junction.location {
            JunctionLocation::Interior => self.material_interfaces.iter().find_map(|interface| {
                interface
                    .nodes
                    .iter()
                    .position(|node| node.junction == Some(junction.id))
                    .and_then(|index| interface.spline.node_point(index))
            }),
            JunctionLocation::Outer { side, fraction } => {
                let [a, b] = match side {
                    crate::OuterSide::Bottom => [
                        Point2::new(self.domain.min_x, self.domain.min_y),
                        Point2::new(self.domain.max_x, self.domain.min_y),
                    ],
                    crate::OuterSide::Right => [
                        Point2::new(self.domain.max_x, self.domain.min_y),
                        Point2::new(self.domain.max_x, self.domain.max_y),
                    ],
                    crate::OuterSide::Top => [
                        Point2::new(self.domain.max_x, self.domain.max_y),
                        Point2::new(self.domain.min_x, self.domain.max_y),
                    ],
                    crate::OuterSide::Left => [
                        Point2::new(self.domain.min_x, self.domain.max_y),
                        Point2::new(self.domain.min_x, self.domain.min_y),
                    ],
                };
                Some(a.lerp(b, fraction))
            }
        }
    }

    fn interface_topology_issue(&self) -> Option<ValidationIssue> {
        let mut incidence = BTreeMap::<JunctionId, Vec<(&MaterialInterface, usize)>>::new();
        for interface in &self.material_interfaces {
            let count = interface.spline.span_count();
            for (node_index, node) in interface.nodes.iter().enumerate() {
                let previous = if node_index > 0 {
                    Some(node_index - 1)
                } else if matches!(interface.spline, InterfaceSpline::Closed(_)) {
                    Some(count - 1)
                } else {
                    None
                };
                let next = if node_index < count {
                    Some(node_index)
                } else if matches!(interface.spline, InterfaceSpline::Closed(_)) {
                    Some(0)
                } else {
                    None
                };
                if let Some(junction) = node.junction {
                    if interface.spline.continuity(node_index) != Some(0) {
                        return Some(ValidationIssue::InterfaceJunctionContinuity(interface.id));
                    }
                    incidence
                        .entry(junction)
                        .or_default()
                        .push((interface, node_index));
                } else {
                    if previous.is_none() || next.is_none() {
                        return Some(ValidationIssue::FreeInterfaceEnd(interface.id));
                    }
                    if interface.span_sides[previous.unwrap()]
                        != interface.span_sides[next.unwrap()]
                    {
                        return Some(ValidationIssue::InterfaceRegionTopology(interface.id));
                    }
                }
            }
        }
        let tolerance = self.domain.tolerance();
        for junction in &self.junctions {
            let Some(point) = self.junction_point(*junction) else {
                return Some(ValidationIssue::UnusedJunction(junction.id));
            };
            let Some(nodes) = incidence.get(&junction.id) else {
                return Some(ValidationIssue::UnusedJunction(junction.id));
            };
            let mut arms = Vec::new();
            for (interface, node_index) in nodes {
                let Some(candidate) = interface.spline.node_point(*node_index) else {
                    return Some(ValidationIssue::JunctionMismatch(junction.id));
                };
                if (candidate - point).norm() > tolerance {
                    return Some(ValidationIssue::JunctionMismatch(junction.id));
                }
                let Some(mut incident) = self.interface_arms_at_node(interface, *node_index) else {
                    return Some(ValidationIssue::JunctionDegenerate(junction.id));
                };
                arms.append(&mut incident);
            }
            arms.sort_by(|left, right| {
                crate::pseudo_angle(left.direction.x, left.direction.y)
                    .total_cmp(&crate::pseudo_angle(right.direction.x, right.direction.y))
            });
            if matches!(junction.location, JunctionLocation::Interior) {
                if arms.len() < 3 {
                    return Some(ValidationIssue::JunctionDegree(junction.id));
                }
                for index in 0..arms.len() {
                    let next = (index + 1) % arms.len();
                    let cross = arms[index].direction.cross(arms[next].direction);
                    let scale = arms[index].direction.norm() * arms[next].direction.norm();
                    // Collinear arms pointing away from one another are the
                    // ordinary straight-through pair of a T or X junction.
                    // Arms pointing in the same direction overlap and leave
                    // the material sector ordering ambiguous.
                    if cross.abs() <= scale * 1.0e-8
                        && arms[index].direction.dot(arms[next].direction) > 0.0
                    {
                        return Some(ValidationIssue::JunctionDegenerate(junction.id));
                    }
                    if arms[index].left != arms[next].right {
                        return Some(ValidationIssue::JunctionRegionTopology(junction.id));
                    }
                }
            } else if arms.is_empty() {
                return Some(ValidationIssue::JunctionDegree(junction.id));
            }
        }
        None
    }

    pub fn material(&self, id: MaterialId) -> Option<&Material> {
        self.materials.iter().find(|material| material.id == id)
    }

    pub fn region(&self, id: RegionId) -> Option<&Region> {
        self.regions.iter().find(|region| region.id == id)
    }

    pub fn region_material(&self, id: RegionId) -> Option<&Material> {
        self.region(id)
            .and_then(|region| self.material(region.material))
    }

    pub fn volume_source(&self, region: RegionId) -> Option<&VolumeSource> {
        self.volume_sources
            .iter()
            .find(|source| source.region == region)
    }

    pub fn material_at(
        &self,
        region: RegionId,
        point: Point2,
    ) -> Result<EvaluatedMaterial, MaterialError> {
        crate::wave::evaluate_material_library_at(
            self.physics,
            &self.materials,
            &self.regions,
            region,
            point,
        )
    }

    pub fn directional_material_at(
        &self,
        region: RegionId,
        point: Point2,
    ) -> Result<crate::DirectionalWaveCoefficients, MaterialError> {
        crate::wave::evaluate_directional_material_library_at(
            self.physics,
            &self.materials,
            &self.regions,
            region,
            point,
        )
    }

    pub fn has_varying_materials(&self) -> bool {
        self.regions.iter().any(|region| {
            self.material(region.material)
                .is_some_and(Material::varying)
        })
    }

    /// Equality of everything that contributes to the mesh or wave operator.
    /// Volume sources are compiled into independent forcing buffers.
    pub fn operator_eq(&self, other: &Self) -> bool {
        self.domain == other.domain
            && self.physics == other.physics
            && self.obstacles == other.obstacles
            && self.internal_boundaries == other.internal_boundaries
            && self.material_interfaces == other.material_interfaces
            && self.junctions == other.junctions
            && self.materials == other.materials
            && self.regions.len() == other.regions.len()
            && self
                .regions
                .iter()
                .zip(&other.regions)
                .all(|(left, right)| {
                    left.id == right.id
                        && left.material == right.material
                        && (!(self
                            .material(left.material)
                            .is_some_and(Material::uses_frame)
                            || other
                                .material(right.material)
                                .is_some_and(Material::uses_frame))
                            || left.frame == right.frame)
                })
            && self.outer_boundaries == other.outer_boundaries
    }

    /// Equality of region-source definitions and the frames used by their profiles.
    pub fn volume_sources_eq(&self, other: &Self) -> bool {
        self.volume_sources == other.volume_sources
            && self.volume_sources.iter().all(|source| {
                !source.varying()
                    || self.region(source.region).map(|region| region.frame)
                        == other.region(source.region).map(|region| region.frame)
            })
    }

    /// Equality of the source carriers compiled into nodal weights. Temporal
    /// signal changes can reuse these weights and replace only forcing data.
    pub fn volume_source_carriers_eq(&self, other: &Self) -> bool {
        self.volume_sources.len() == other.volume_sources.len()
            && self
                .volume_sources
                .iter()
                .zip(&other.volume_sources)
                .all(|(left, right)| {
                    left.region == right.region
                        && left.enabled == right.enabled
                        && left.profile == right.profile
                        && left.parameters == right.parameters
                        && (!left.varying()
                            || self.region(left.region).map(|region| region.frame)
                                == other.region(right.region).map(|region| region.frame))
                })
    }

    /// Geometry and topology equality excludes names, colors, coefficients, and
    /// region-to-material assignments so those edits can reuse the mesh.
    pub fn geometry_eq(&self, other: &Self) -> bool {
        self.domain == other.domain
            && self.obstacles.len() == other.obstacles.len()
            && self
                .obstacles
                .iter()
                .zip(&other.obstacles)
                .all(|(left, right)| {
                    left.id == right.id && left.spline == right.spline && left.role == right.role
                })
            && self.internal_boundaries.len() == other.internal_boundaries.len()
            && self
                .internal_boundaries
                .iter()
                .zip(&other.internal_boundaries)
                .all(|(left, right)| {
                    left.id == right.id
                        && left.spline == right.spline
                        && left.region == right.region
                })
            && self.material_interfaces == other.material_interfaces
            && self.junctions == other.junctions
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum ValidationIssue {
    Domain,
    Structure,
    Subdivision(ObstacleId),
    Outside(ObstacleId),
    Degenerate(ObstacleId),
    SelfContact(ObstacleId),
    ObstacleContact(ObstacleId, ObstacleId),
    Nested(ObstacleId, ObstacleId),
    RegionTopology(ObstacleId),
    BoundarySubdivision(InternalBoundaryId),
    BoundaryOutside(InternalBoundaryId),
    BoundaryDegenerate(InternalBoundaryId),
    BoundarySelfContact(InternalBoundaryId),
    BoundaryContact(InternalBoundaryId),
    BoundaryRegionTopology(InternalBoundaryId),
    InterfaceSubdivision(MaterialInterfaceId),
    InterfaceOutside(MaterialInterfaceId),
    InterfaceDegenerate(MaterialInterfaceId),
    InterfaceSelfContact(MaterialInterfaceId),
    InterfaceContact(MaterialInterfaceId),
    FreeInterfaceEnd(MaterialInterfaceId),
    InterfaceRegionTopology(MaterialInterfaceId),
    InterfaceJunctionContinuity(MaterialInterfaceId),
    UnusedJunction(JunctionId),
    JunctionMismatch(JunctionId),
    JunctionDegree(JunctionId),
    JunctionDegenerate(JunctionId),
    JunctionRegionTopology(JunctionId),
    WorkLimit,
}
impl std::fmt::Display for ValidationIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Domain => write!(
                f,
                "Domain bounds must be finite with width and height between {MIN_DOMAIN_EXTENT} and {MAX_DOMAIN_EXTENT}"
            ),
            Self::Structure => write!(
                f,
                "Scene has invalid IDs, materials, regions, or too many loops"
            ),
            Self::Subdivision(id) => write!(
                f,
                "Loop {}: sampling is exhausted or numerically ambiguous",
                id.0
            ),
            Self::Outside(id) => write!(f, "Loop {} leaves or nearly touches the domain", id.0),
            Self::Degenerate(id) => write!(f, "Loop {} is degenerate or too small", id.0),
            Self::SelfContact(id) => {
                write!(f, "Loop {} crosses or nearly touches itself", id.0)
            }
            Self::ObstacleContact(a, b) => {
                write!(f, "Loops {} and {} intersect or nearly touch", a.0, b.0)
            }
            Self::Nested(a, b) => write!(
                f,
                "Loops {} and {} have incompatible nesting roles",
                a.0, b.0
            ),
            Self::RegionTopology(id) => write!(
                f,
                "Loop {} has a region assignment inconsistent with its containment",
                id.0
            ),
            Self::BoundarySubdivision(id) => write!(
                f,
                "Internal boundary {}: sampling is exhausted or numerically ambiguous",
                id.0
            ),
            Self::BoundaryOutside(id) => write!(
                f,
                "Internal boundary {} leaves or nearly touches the domain",
                id.0
            ),
            Self::BoundaryDegenerate(id) => {
                write!(f, "Internal boundary {} is degenerate or too short", id.0)
            }
            Self::BoundarySelfContact(id) => write!(
                f,
                "Internal boundary {} crosses or nearly touches itself",
                id.0
            ),
            Self::BoundaryContact(id) => write!(
                f,
                "Internal boundary {} intersects or nearly touches another boundary",
                id.0
            ),
            Self::BoundaryRegionTopology(id) => write!(
                f,
                "Internal boundary {} has a region assignment inconsistent with its location",
                id.0
            ),
            Self::InterfaceSubdivision(id) => write!(
                f,
                "Material divider {}: sampling is exhausted or numerically ambiguous",
                id.0
            ),
            Self::InterfaceOutside(id) => {
                write!(f, "Material divider {} leaves the domain", id.0)
            }
            Self::InterfaceDegenerate(id) => {
                write!(f, "Material divider {} is degenerate or too short", id.0)
            }
            Self::InterfaceSelfContact(id) => {
                write!(
                    f,
                    "Material divider {} crosses or nearly touches itself",
                    id.0
                )
            }
            Self::InterfaceContact(id) => write!(
                f,
                "Material divider {} crosses or nearly touches incompatible geometry",
                id.0
            ),
            Self::FreeInterfaceEnd(id) => {
                write!(f, "Material interface {} has a free endpoint", id.0)
            }
            Self::InterfaceRegionTopology(id) => write!(
                f,
                "Material interface {} changes adjacent regions away from a junction",
                id.0
            ),
            Self::InterfaceJunctionContinuity(id) => write!(
                f,
                "Material interface {} must be C0 at an attached junction",
                id.0
            ),
            Self::UnusedJunction(id) => write!(f, "Junction {} has no attached curve", id.0),
            Self::JunctionMismatch(id) => {
                write!(f, "Curves attached to junction {} do not meet", id.0)
            }
            Self::JunctionDegree(id) => {
                write!(f, "Junction {} has too few incident curve arms", id.0)
            }
            Self::JunctionDegenerate(id) => write!(
                f,
                "Junction {} has coincident, tangent, or degenerate curve arms",
                id.0
            ),
            Self::JunctionRegionTopology(id) => {
                write!(f, "Junction {} has inconsistent material sectors", id.0)
            }
            Self::WorkLimit => write!(f, "Validation work limit reached; simplify the scene"),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ValidationResult {
    pub revision: u64,
    pub issue: Option<ValidationIssue>,
}
impl ValidationResult {
    pub fn valid(&self) -> bool {
        self.issue.is_none()
    }
}
#[derive(Clone, Copy)]
struct Segment {
    a: Point2,
    b: Point2,
    curve: usize,
    index: usize,
    arc_start: f64,
    arc_end: f64,
}
/// At most `budget` elementary operations per advance; no camera inputs.
pub struct ValidationJob {
    revision: u64,
    scene: Scene,
    options: SamplingOptions,
    sampler: Option<Sampler>,
    open_sampler: Option<OpenSampler>,
    loops: Vec<Vec<Sample>>,
    open_boundaries: Vec<Vec<Sample>>,
    sampled_interfaces: usize,
    segments: Vec<Segment>,
    bounds: Vec<(Point2, Point2)>,
    perimeters: Vec<f64>,
    pending_points: Option<Vec<Sample>>,
    build_index: usize,
    area: f64,
    perimeter: f64,
    lower: Point2,
    upper: Point2,
    i: usize,
    j: usize,
    nest_a: usize,
    nest_b: usize,
    nest_edge: usize,
    inside: bool,
    containment: Vec<Vec<bool>>,
    work: usize,
    result: Option<ValidationResult>,
}
impl ValidationJob {
    pub fn new(scene: Scene, revision: u64) -> Self {
        Self::with_options(scene, revision, SamplingOptions::default())
    }
    pub fn with_options(scene: Scene, revision: u64, options: SamplingOptions) -> Self {
        let loop_count = scene.obstacles.len();
        let issue = if !scene.domain.valid() {
            Some(ValidationIssue::Domain)
        } else if scene.structure_valid() {
            scene.interface_topology_issue()
        } else {
            Some(ValidationIssue::Structure)
        };
        Self {
            revision,
            scene,
            options,
            sampler: None,
            open_sampler: None,
            loops: vec![],
            open_boundaries: vec![],
            sampled_interfaces: 0,
            segments: vec![],
            bounds: vec![],
            perimeters: vec![],
            pending_points: None,
            build_index: 0,
            area: 0.0,
            perimeter: 0.0,
            lower: Point2::new(f64::INFINITY, f64::INFINITY),
            upper: Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY),
            i: 0,
            j: 1,
            nest_a: 0,
            nest_b: 1,
            nest_edge: 0,
            inside: false,
            containment: vec![vec![false; loop_count]; loop_count],
            work: 0,
            result: issue.map(|v| ValidationResult {
                revision,
                issue: Some(v),
            }),
        }
    }
    fn finish(&mut self, issue: Option<ValidationIssue>) {
        self.result = Some(ValidationResult {
            revision: self.revision,
            issue,
        });
    }
    pub fn advance(&mut self, budget: usize) -> Option<ValidationResult> {
        for _ in 0..budget {
            if self.result.is_some() {
                break;
            }
            self.work += 1;
            if self.work > 50_000_000 {
                self.finish(Some(ValidationIssue::WorkLimit));
                break;
            }
            if let Some(points) = &self.pending_points {
                let is_open = self.loops.len() == self.scene.obstacles.len();
                let curve = if is_open {
                    self.scene.obstacles.len() + self.open_boundaries.len()
                } else {
                    self.loops.len()
                };
                if self.build_index + 1 == points.len() {
                    let issue = if is_open {
                        let open_index = self.open_boundaries.len();
                        let degenerate = self.perimeter <= self.scene.domain.tolerance()
                            || (points.last().unwrap().point - points[0].point).norm()
                                <= self.scene.domain.tolerance();
                        if open_index < self.scene.internal_boundaries.len() {
                            degenerate.then_some(ValidationIssue::BoundaryDegenerate(
                                self.scene.internal_boundaries[open_index].id,
                            ))
                        } else {
                            degenerate.then_some(ValidationIssue::InterfaceDegenerate(
                                self.scene.material_interfaces
                                    [open_index - self.scene.internal_boundaries.len()]
                                .id,
                            ))
                        }
                    } else {
                        (self.area.abs() * 0.5
                            <= self.scene.domain.tolerance() * self.scene.domain.tolerance())
                        .then_some(ValidationIssue::Degenerate(
                            self.scene.obstacles[self.loops.len()].id,
                        ))
                    };
                    if issue.is_some() {
                        self.finish(issue);
                        continue;
                    }
                    if is_open {
                        self.open_boundaries
                            .push(self.pending_points.take().unwrap());
                    } else {
                        self.loops.push(self.pending_points.take().unwrap());
                    }
                    self.bounds.push((self.lower, self.upper));
                    self.perimeters.push(self.perimeter);
                    self.build_index = 0;
                    self.area = 0.0;
                    self.perimeter = 0.0;
                    self.lower = Point2::new(f64::INFINITY, f64::INFINITY);
                    self.upper = Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
                    continue;
                }
                let a = points[self.build_index].point;
                let b = points[self.build_index + 1].point;
                let margin = self.scene.domain.tolerance() + self.options.tolerance;
                let open_index = self.open_boundaries.len();
                let graph_interface = is_open && open_index >= self.scene.internal_boundaries.len();
                let outside = [a, b].iter().any(|point| {
                    if graph_interface {
                        point.x < self.scene.domain.min_x - margin
                            || point.x > self.scene.domain.max_x + margin
                            || point.y < self.scene.domain.min_y - margin
                            || point.y > self.scene.domain.max_y + margin
                    } else {
                        !self.scene.domain.contains_with_margin(*point, margin)
                    }
                });
                if outside {
                    let issue = if is_open {
                        if graph_interface {
                            ValidationIssue::InterfaceOutside(
                                self.scene.material_interfaces
                                    [open_index - self.scene.internal_boundaries.len()]
                                .id,
                            )
                        } else {
                            ValidationIssue::BoundaryOutside(
                                self.scene.internal_boundaries[open_index].id,
                            )
                        }
                    } else {
                        ValidationIssue::Outside(self.scene.obstacles[self.loops.len()].id)
                    };
                    self.finish(Some(issue));
                    continue;
                }
                self.lower.x = self.lower.x.min(a.x);
                self.lower.y = self.lower.y.min(a.y);
                self.upper.x = self.upper.x.max(a.x);
                self.upper.y = self.upper.y.max(a.y);
                self.lower.x = self.lower.x.min(b.x);
                self.lower.y = self.lower.y.min(b.y);
                self.upper.x = self.upper.x.max(b.x);
                self.upper.y = self.upper.y.max(b.y);
                self.area += a.cross(b);
                let arc_start = self.perimeter;
                self.perimeter += (b - a).norm();
                self.segments.push(Segment {
                    a,
                    b,
                    curve,
                    index: self.build_index,
                    arc_start,
                    arc_end: self.perimeter,
                });
                self.build_index += 1;
            } else if self.loops.len() < self.scene.obstacles.len() {
                let o = &self.scene.obstacles[self.loops.len()];
                let id = o.id;
                let sampler = self
                    .sampler
                    .get_or_insert_with(|| Sampler::new(&o.spline, self.options));
                if sampler.step() {
                    match self.sampler.take().unwrap().finish() {
                        Err(_) => self.finish(Some(ValidationIssue::Subdivision(id))),
                        Ok(points) => self.pending_points = Some(points),
                    }
                }
            } else if self.open_boundaries.len() < self.scene.internal_boundaries.len() {
                let boundary = &self.scene.internal_boundaries[self.open_boundaries.len()];
                let sampler = self
                    .open_sampler
                    .get_or_insert_with(|| OpenSampler::new(&boundary.spline, self.options));
                if sampler.step() {
                    match self.open_sampler.take().unwrap().finish() {
                        Err(_) => {
                            self.finish(Some(ValidationIssue::BoundarySubdivision(boundary.id)))
                        }
                        Ok(points) => self.pending_points = Some(points),
                    }
                }
            } else if self.sampled_interfaces < self.scene.material_interfaces.len() {
                let interface = &self.scene.material_interfaces[self.sampled_interfaces];
                let InterfaceSpline::Open(spline) = &interface.spline else {
                    self.finish(Some(ValidationIssue::InterfaceSubdivision(interface.id)));
                    continue;
                };
                let sampler = self
                    .open_sampler
                    .get_or_insert_with(|| OpenSampler::new(spline, self.options));
                if sampler.step() {
                    match self.open_sampler.take().unwrap().finish() {
                        Err(_) => {
                            self.finish(Some(ValidationIssue::InterfaceSubdivision(interface.id)))
                        }
                        Ok(points) => {
                            self.sampled_interfaces += 1;
                            self.pending_points = Some(points);
                        }
                    }
                }
            } else if self.i < self.segments.len() {
                if self.j >= self.segments.len() {
                    self.i += 1;
                    self.j = self.i + 1;
                    continue;
                }
                let a = self.segments[self.i];
                let b = self.segments[self.j];
                self.j += 1;
                let margin = self.scene.domain.tolerance() + 2.0 * self.options.tolerance;
                let mut local_neighbors = false;
                if a.curve == b.curve {
                    let closed = a.curve < self.loops.len();
                    let points = if closed {
                        &self.loops[a.curve]
                    } else {
                        &self.open_boundaries[a.curve - self.loops.len()]
                    };
                    let n = points.len() - 1;
                    if a.index.abs_diff(b.index) == 1
                        || (closed && a.index.abs_diff(b.index) == n - 1)
                    {
                        continue;
                    }
                    // Knot insertion can introduce arbitrarily short spans on an
                    // unchanged smooth arc. Suppress only local proximity, never
                    // a crossing, using arc distance instead of segment indices.
                    let direct_gap = b.arc_start - a.arc_end;
                    let gap = if closed {
                        direct_gap.min(self.perimeters[a.curve] - b.arc_end + a.arc_start)
                    } else {
                        direct_gap
                    };
                    local_neighbors = gap <= 4.0 * margin;
                } else {
                    // Open boundaries may form intentional tip junctions. The
                    // incident endpoint segments are topological neighbors even
                    // when the two pieces have different stable IDs.
                    let open_offset = self.scene.obstacles.len();
                    if a.curve >= open_offset && b.curve >= open_offset {
                        let a_open = a.curve - open_offset;
                        let b_open = b.curve - open_offset;
                        let a_points = &self.open_boundaries[a_open];
                        let b_points = &self.open_boundaries[b_open];
                        let both_interfaces = a_open >= self.scene.internal_boundaries.len()
                            && b_open >= self.scene.internal_boundaries.len();
                        if both_interfaces {
                            let shared_junction = [a.a, a.b].into_iter().any(|a_point| {
                                [b.a, b.b].into_iter().any(|b_point| {
                                    (a_point - b_point).norm() <= margin
                                        && self.scene.junctions.iter().any(|junction| {
                                            self.scene.junction_point(*junction).is_some_and(
                                                |junction_point| {
                                                    (junction_point - a_point).norm() <= margin
                                                },
                                            )
                                        })
                                })
                            });
                            if shared_junction {
                                continue;
                            }
                        }
                        let a_last_segment = a_points.len() - 2;
                        let b_last_segment = b_points.len() - 2;
                        let shared_incident_tip = [
                            (0, 0, a.index == 0 && b.index == 0),
                            (
                                0,
                                b_points.len() - 1,
                                a.index == 0 && b.index == b_last_segment,
                            ),
                            (
                                a_points.len() - 1,
                                0,
                                a.index == a_last_segment && b.index == 0,
                            ),
                            (
                                a_points.len() - 1,
                                b_points.len() - 1,
                                a.index == a_last_segment && b.index == b_last_segment,
                            ),
                        ]
                        .into_iter()
                        .any(|(a_tip, b_tip, incident)| {
                            if !incident
                                || (a_points[a_tip].point - b_points[b_tip].point).norm() > 1.0e-12
                            {
                                return false;
                            }
                            if a_open < self.scene.internal_boundaries.len()
                                && b_open < self.scene.internal_boundaries.len()
                            {
                                return self.scene.internal_boundaries[a_open].region
                                    == self.scene.internal_boundaries[b_open].region;
                            }
                            false
                        });
                        if shared_incident_tip {
                            continue;
                        }
                    }
                    let (amin, amax) = self.bounds[a.curve];
                    let (bmin, bmax) = self.bounds[b.curve];
                    if amax.x + margin < bmin.x
                        || bmax.x + margin < amin.x
                        || amax.y + margin < bmin.y
                        || bmax.y + margin < amin.y
                    {
                        // Segments are grouped by obstacle: skip the entire loop.
                        let point_count = if b.curve < self.loops.len() {
                            self.loops[b.curve].len()
                        } else {
                            self.open_boundaries[b.curve - self.loops.len()].len()
                        };
                        self.j += point_count - 2 - b.index;
                        continue;
                    }
                }
                if segments_close(
                    a.a,
                    a.b,
                    b.a,
                    b.b,
                    if local_neighbors { 0.0 } else { margin },
                ) {
                    let issue = if a.curve == b.curve {
                        if a.curve < self.scene.obstacles.len() {
                            ValidationIssue::SelfContact(self.scene.obstacles[a.curve].id)
                        } else {
                            let open = a.curve - self.scene.obstacles.len();
                            if open < self.scene.internal_boundaries.len() {
                                ValidationIssue::BoundarySelfContact(
                                    self.scene.internal_boundaries[open].id,
                                )
                            } else {
                                ValidationIssue::InterfaceSelfContact(
                                    self.scene.material_interfaces
                                        [open - self.scene.internal_boundaries.len()]
                                    .id,
                                )
                            }
                        }
                    } else if let Some(interface) =
                        [a.curve, b.curve].into_iter().find_map(|curve| {
                            let open = curve.checked_sub(self.scene.obstacles.len())?;
                            let interface =
                                open.checked_sub(self.scene.internal_boundaries.len())?;
                            self.scene.material_interfaces.get(interface)
                        })
                    {
                        ValidationIssue::InterfaceContact(interface.id)
                    } else if a.curve >= self.scene.obstacles.len() {
                        ValidationIssue::BoundaryContact(
                            self.scene.internal_boundaries[a.curve - self.scene.obstacles.len()].id,
                        )
                    } else if b.curve >= self.scene.obstacles.len() {
                        ValidationIssue::BoundaryContact(
                            self.scene.internal_boundaries[b.curve - self.scene.obstacles.len()].id,
                        )
                    } else {
                        ValidationIssue::ObstacleContact(
                            self.scene.obstacles[a.curve].id,
                            self.scene.obstacles[b.curve].id,
                        )
                    };
                    self.finish(Some(issue));
                }
            } else if self.nest_a < self.loops.len() {
                if self.nest_b >= self.loops.len() {
                    self.nest_a += 1;
                    self.nest_b = 0;
                    continue;
                }
                if self.nest_a == self.nest_b {
                    self.nest_b += 1;
                    continue;
                }
                let p = self.loops[self.nest_a][0].point;
                let (lower, upper) = self.bounds[self.nest_b];
                if p.x < lower.x || p.y < lower.y || p.x > upper.x || p.y > upper.y {
                    self.nest_b += 1;
                    continue;
                }
                let polygon = &self.loops[self.nest_b];
                if self.nest_edge + 1 >= polygon.len() {
                    if self.inside {
                        self.containment[self.nest_a][self.nest_b] = true;
                    }
                    self.nest_b += 1;
                    self.nest_edge = 0;
                    self.inside = false;
                    continue;
                }
                let a = polygon[self.nest_edge].point;
                let b = polygon[self.nest_edge + 1].point;
                self.nest_edge += 1;
                if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x
                {
                    self.inside = !self.inside;
                }
            } else {
                self.finish(self.region_topology_issue())
            }
        }
        self.result.clone()
    }

    fn region_topology_issue(&self) -> Option<ValidationIssue> {
        for (index, obstacle) in self.scene.obstacles.iter().enumerate() {
            let direct_container = (0..self.scene.obstacles.len())
                .filter(|container| self.containment[index][*container])
                .min_by(|a, b| self.perimeters[*a].total_cmp(&self.perimeters[*b]));
            let expected_exterior = match direct_container {
                None => BACKGROUND_REGION,
                Some(container) => match self.scene.obstacles[container].role.interior() {
                    Some(region) => region,
                    None => {
                        return Some(ValidationIssue::Nested(
                            obstacle.id,
                            self.scene.obstacles[container].id,
                        ));
                    }
                },
            };
            if obstacle.role.exterior() != expected_exterior {
                return Some(ValidationIssue::RegionTopology(obstacle.id));
            }
        }
        for (boundary, samples) in self
            .scene
            .internal_boundaries
            .iter()
            .zip(&self.open_boundaries)
        {
            let point = samples[0].point;
            let direct_container = self
                .loops
                .iter()
                .enumerate()
                .filter(|(_, polygon)| point_inside_samples(point, polygon))
                .min_by(|(a, _), (b, _)| self.perimeters[*a].total_cmp(&self.perimeters[*b]));
            let expected = match direct_container {
                None => Some(BACKGROUND_REGION),
                Some((index, _)) => self.scene.obstacles[index].role.interior(),
            };
            if expected != Some(boundary.region) {
                return Some(ValidationIssue::BoundaryRegionTopology(boundary.id));
            }
        }
        None
    }
}

fn point_inside_samples(point: Point2, polygon: &[Sample]) -> bool {
    let mut inside = false;
    for edge in polygon.windows(2) {
        let a = edge[0].point;
        let b = edge[1].point;
        if (a.y > point.y) != (b.y > point.y)
            && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
        {
            inside = !inside;
        }
    }
    inside
}

fn segments_close(a: Point2, b: Point2, c: Point2, d: Point2, tol: f64) -> bool {
    if a.x.max(b.x) + tol < c.x.min(d.x)
        || c.x.max(d.x) + tol < a.x.min(b.x)
        || a.y.max(b.y) + tol < c.y.min(d.y)
        || c.y.max(d.y) + tol < a.y.min(b.y)
    {
        return false;
    }
    let ab = b - a;
    let cd = d - c;
    let x1 = ab.cross(c - a);
    let x2 = ab.cross(d - a);
    let y1 = cd.cross(a - c);
    let y2 = cd.cross(b - c);
    if x1.signum() != x2.signum() && y1.signum() != y2.signum() {
        return true;
    }
    [
        point_segment_distance(a, c, d),
        point_segment_distance(b, c, d),
        point_segment_distance(c, a, b),
        point_segment_distance(d, a, b),
    ]
    .into_iter()
    .fold(f64::INFINITY, f64::min)
        <= tol
}
pub fn validate(scene: &Scene) -> ValidationResult {
    let mut job = ValidationJob::new(scene.clone(), 0);
    loop {
        if let Some(result) = job.advance(10000) {
            return result;
        }
    }
}
