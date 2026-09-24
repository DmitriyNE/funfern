use std::collections::BTreeMap;

use crate::{
    BACKGROUND_REGION, CanonicalMaterialRuntimeState, EvaluatedMaterial, FieldLaw, FieldLawValues,
    Material, MaterialError, MaterialFrame, OuterSide, Point2, Region, RegionId, SymmetricTensor2,
    TriMesh,
};

pub(crate) fn evaluate_material_library_at(
    physics: PhysicsModel,
    materials: &[Material],
    regions: &[Region],
    region: RegionId,
    point: Point2,
) -> Result<EvaluatedMaterial, MaterialError> {
    let region = regions
        .iter()
        .find(|candidate| candidate.id == region)
        .ok_or(MaterialError::InvalidValue)?;
    let properties = materials
        .iter()
        .find(|material| material.id == region.material)
        .ok_or(MaterialError::InvalidValue)?
        .evaluate(region.frame, point)?;
    let values = physics.wave_coefficients(WaveCoefficients {
        mass_density: properties.mass_density,
        stiffness: properties.stiffness,
        damping: properties.damping,
    });
    values
        .valid()
        .then_some(EvaluatedMaterial {
            mass_density: values.mass_density,
            stiffness: values.stiffness,
            damping: values.damping,
            axis_ratio: properties.axis_ratio,
        })
        .ok_or(MaterialError::InvalidValue)
}

pub(crate) fn evaluate_directional_material_library_at(
    physics: PhysicsModel,
    materials: &[Material],
    regions: &[Region],
    region: RegionId,
    point: Point2,
) -> Result<DirectionalWaveCoefficients, MaterialError> {
    let region = regions
        .iter()
        .find(|candidate| candidate.id == region)
        .ok_or(MaterialError::InvalidValue)?;
    let properties = materials
        .iter()
        .find(|material| material.id == region.material)
        .ok_or(MaterialError::InvalidValue)?
        .evaluate(region.frame, point)?;
    let values = physics.directional_wave_coefficients(properties, region.frame);
    values
        .valid()
        .then_some(values)
        .ok_or(MaterialError::InvalidValue)
}

/// One material sample in both of the forms an estimator needs: at the
/// authored coefficients, and at the instant a snapshot belongs to.
pub(crate) struct TimedDirectionalWaveCoefficients {
    pub authored: DirectionalWaveCoefficients,
    pub instantaneous: DirectionalWaveCoefficients,
}

impl TimedDirectionalWaveCoefficients {
    /// A time-invariant medium, whose two forms are the same sample.
    pub fn fixed(coefficients: DirectionalWaveCoefficients) -> Self {
        Self {
            authored: coefficients,
            instantaneous: coefficients,
        }
    }
}

/// Evaluates one material sample both as authored and as it stands at `time`.
///
/// Both forms come out of a single material lookup: the instantaneous one is
/// the authored one with each solver row scaled by the drive and Switch factor
/// in force. The primary row is the mass, which the factor multiplies. The
/// complementary row is the reciprocal of the stiffness tensor, so its factor
/// divides that tensor instead. Which authored law owns which row is the
/// physics skin's business and `coefficient_for` answers it, which is why this
/// holds for all three skins without a case of its own.
pub(crate) fn evaluate_timed_directional_material_library_at(
    physics: PhysicsModel,
    materials: &[Material],
    regions: &[Region],
    region: RegionId,
    point: Point2,
    time: f64,
    runtime: &CanonicalMaterialRuntimeState,
) -> Result<TimedDirectionalWaveCoefficients, MaterialError> {
    if !time.is_finite() {
        return Err(MaterialError::InvalidValue);
    }
    let region = regions
        .iter()
        .find(|candidate| candidate.id == region)
        .ok_or(MaterialError::InvalidValue)?;
    let material = materials
        .iter()
        .find(|material| material.id == region.material)
        .ok_or(MaterialError::InvalidValue)?;
    // Only the two constitutive rows are applied here, so a medium whose law
    // reaches anywhere else is refused rather than half-evaluated. That is the
    // same line the temporal supplement draws: it covers the conservative bulk
    // and refuses an operator carrying loss.
    if material.electric_loss.is_some()
        || material.magnetic_loss.is_some()
        || !material.restoring.is_none()
    {
        return Err(MaterialError::UnsupportedMaterialLaw);
    }
    let properties = material.evaluate_base(region.frame, point)?;
    let authored = physics.directional_wave_coefficients(properties, region.frame);
    if !authored.valid() {
        return Err(MaterialError::InvalidValue);
    }
    let coordinates = region.frame.coordinates(point);
    let mut factors = [1.0; 2];
    for (factor, primary) in factors.iter_mut().zip([true, false]) {
        let (drive, law) = crate::canonical_temporal::coefficient_for(physics, material, primary);
        let mut law = law.evaluate_at(coordinates, &material.parameters)?;
        // A field law is read at its small-signal limit, `ḡ(0) = 1`: the
        // instantaneous material here sizes the mesh by its wave speed, and
        // a field-free speed is the one the medium has wherever the field is
        // weak. Kerr with χ > 0 is slower where the field is strong, so an
        // amplitude-aware size rule is a carried item, not this.
        if law.field.executable(law.inverted).is_err() {
            return Err(MaterialError::UnsupportedMaterialLaw);
        }
        law.field = FieldLawValues::Linear;
        *factor = runtime.coefficient_law_factor(material.id, drive, law, coordinates, time)?;
        if !factor.is_finite() || *factor <= 0.0 {
            return Err(MaterialError::InvalidValue);
        }
    }
    let [mass, stiffness] = factors;
    let instantaneous = DirectionalWaveCoefficients {
        mass_density: authored.mass_density * mass,
        stiffness: SymmetricTensor2::new(
            authored.stiffness.xx / stiffness,
            authored.stiffness.xy / stiffness,
            authored.stiffness.yy / stiffness,
        ),
        damping: authored.damping,
    };
    instantaneous
        .valid()
        .then_some(TimedDirectionalWaveCoefficients {
            authored,
            instantaneous,
        })
        .ok_or(MaterialError::InvalidValue)
}

/// Constant material coefficients for the scalar wave model
/// `mass_density * u_tt + damping * u_t - div(stiffness * grad(u)) = f`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaveCoefficients {
    pub mass_density: f64,
    pub stiffness: f64,
    pub damping: f64,
}

/// Coefficients after a material's directional law and local frame have been
/// resolved at one world-space point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectionalWaveCoefficients {
    pub mass_density: f64,
    pub stiffness: SymmetricTensor2,
    pub damping: f64,
}

impl DirectionalWaveCoefficients {
    pub fn valid(self) -> bool {
        self.mass_density.is_finite()
            && self.mass_density > 0.0
            && self.stiffness.finite_spd()
            && self.damping.is_finite()
            && self.damping >= 0.0
    }

    pub fn minimum_wave_speed(self) -> f64 {
        (self.stiffness.eigenvalues()[0] / self.mass_density).sqrt()
    }

    pub fn maximum_wave_speed(self) -> f64 {
        (self.stiffness.eigenvalues()[1] / self.mass_density).sqrt()
    }

    pub fn normal_impedance(self, normal: Point2) -> f64 {
        (self.mass_density * self.stiffness.quadratic_form(normal)).sqrt()
    }

    pub fn geometric_mean_stiffness(self) -> f64 {
        self.stiffness.determinant().sqrt()
    }
}

impl WaveCoefficients {
    pub fn valid(self) -> bool {
        self.mass_density.is_finite()
            && self.mass_density > 0.0
            && self.stiffness.is_finite()
            && self.stiffness > 0.0
            && self.damping.is_finite()
            && self.damping >= 0.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PhysicsModel {
    #[default]
    Mechanical,
    Electromagnetic {
        polarization: ElectromagneticPolarization,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ElectromagneticPolarization {
    #[default]
    Tm,
    Te,
}

impl PhysicsModel {
    pub fn wave_coefficients(self, properties: WaveCoefficients) -> WaveCoefficients {
        match self {
            Self::Mechanical => properties,
            Self::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            } => WaveCoefficients {
                mass_density: properties.mass_density,
                stiffness: properties.stiffness.recip(),
                damping: properties.mass_density * properties.damping,
            },
            Self::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            } => WaveCoefficients {
                mass_density: properties.stiffness,
                stiffness: properties.mass_density.recip(),
                damping: properties.stiffness * properties.damping,
            },
        }
    }

    pub fn wave_speed(self, properties: WaveCoefficients) -> f64 {
        match self {
            Self::Mechanical => (properties.stiffness / properties.mass_density).sqrt(),
            Self::Electromagnetic { .. } => (properties.mass_density * properties.stiffness)
                .sqrt()
                .recip(),
        }
    }

    pub fn impedance(self, properties: WaveCoefficients) -> f64 {
        match self {
            Self::Mechanical => (properties.mass_density * properties.stiffness).sqrt(),
            Self::Electromagnetic { .. } => (properties.stiffness / properties.mass_density).sqrt(),
        }
    }

    pub fn directional_wave_coefficients(
        self,
        properties: EvaluatedMaterial,
        frame: MaterialFrame,
    ) -> DirectionalWaveCoefficients {
        let scalar = self.wave_coefficients(WaveCoefficients {
            mass_density: properties.mass_density,
            stiffness: properties.stiffness,
            damping: properties.damping,
        });
        let ratio = properties.axis_ratio;
        DirectionalWaveCoefficients {
            mass_density: scalar.mass_density,
            stiffness: frame.tensor_from_local(scalar.stiffness * ratio, scalar.stiffness / ratio),
            damping: scalar.damping,
        }
    }

    /// Converts a material law to `target` while preserving its local wave
    /// speed, characteristic impedance, and normalized damping rate.
    pub fn convert_material(
        self,
        target: Self,
        material: &Material,
    ) -> Result<Material, MaterialError> {
        let crossing_to_em =
            matches!(self, Self::Mechanical) && matches!(target, Self::Electromagnetic { .. });
        let crossing_to_mechanical =
            matches!(self, Self::Electromagnetic { .. }) && matches!(target, Self::Mechanical);
        if !crossing_to_em && !crossing_to_mechanical {
            // TM and TE name the same electric and magnetic data; only which
            // one the solver carries nodally differs, so there is nothing in
            // the material to rewrite.
            return Ok(material.clone());
        }

        // The mechanical adapter is `s₀ = ε = 1/k₀` and `μ = ρ`, so the two
        // stored slots hold different physical quantities in each skin and a
        // law moves with the quantity it was authored against, not with the
        // slot it happens to sit in. Each law already acts on that quantity:
        // the mass row on `ρ` or `μ`, and the stiffness row on the reciprocal
        // stiffness `s₀`, which the solver multiplies directly. So a law moves
        // unchanged, and only the stored base expression reciprocates
        // (`ε = 1/k₀`). Reciprocating the law as well, as this did until
        // 24 September, ran a stiffness-row drive backwards in the other skin.
        //
        // The same holds for a field law, which is what gate C's
        // Mechanical ↔ TE contract asks for: `ξ = s₀(1 + χ|e|²)e` is
        // `D = ε(1 + χ|E|²)E` with `e ↔ E`, and `ρ(u)` is `μ(H)`. The laws that
        // do not carry are the ones the solver does not execute either: a
        // signed χ₁ or a reciprocal field response has no settled vector law
        // to carry to. A restoring law has no counterpart to move to. Loss
        // channels stay attached to their own physical field, and their rate
        // conversion is a contract of its own, so they are refused rather than
        // guessed at.
        for law in [&material.mass_law, &material.stiffness_law] {
            if matches!(law.field, FieldLaw::Linear) {
                continue;
            }
            let signed = matches!(
                &law.field,
                FieldLaw::Polynomial { chi1, .. } if chi1.constant_value() != Some(0.0)
            );
            if signed || law.inverted {
                return Err(MaterialError::UnconvertibleMaterialLaw(
                    "a signed or reciprocal field response",
                ));
            }
        }
        if !material.restoring.is_none() {
            return Err(MaterialError::UnconvertibleMaterialLaw("a restoring law"));
        }
        if material.electric_loss.is_some() || material.magnetic_loss.is_some() {
            return Err(MaterialError::UnconvertibleMaterialLaw(
                "a named loss channel",
            ));
        }

        let mut converted = material.clone();
        if crossing_to_em {
            converted.mass_density = material.stiffness.reciprocal()?;
            converted.stiffness = material.mass_density.clone();
            converted.damping = material.damping.divide(&material.mass_density)?;
            converted.mass_law = material.stiffness_law.clone();
            converted.stiffness_law = material.mass_law.clone();
        } else {
            converted.mass_density = material.stiffness.clone();
            converted.stiffness = material.mass_density.reciprocal()?;
            converted.damping = material.damping.multiply(&material.stiffness)?;
            converted.mass_law = material.stiffness_law.clone();
            converted.stiffness_law = material.mass_law.clone();
        }
        // An otherwise linear law that only carries `inverted` is the multiplier
        // one, so it normalizes away.
        converted.mass_law = converted.mass_law.normalized();
        converted.stiffness_law = converted.stiffness_law.normalized();
        Ok(converted)
    }
}

impl Default for WaveCoefficients {
    fn default() -> Self {
        Self {
            mass_density: 1.0,
            stiffness: 1.0,
            damping: 0.0,
        }
    }
}

/// A bounded temporal drive shared by point and region sources and prescribed
/// boundary data. Additional waveform variants can extend this representation
/// without changing the spatial source carriers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimeSignal {
    Harmonic {
        offset: f64,
        amplitude: f64,
        frequency_hz: f64,
        phase_radians: f64,
    },
}

impl TimeSignal {
    pub const ZERO: Self = Self::Harmonic {
        offset: 0.0,
        amplitude: 0.0,
        frequency_hz: 1.0,
        phase_radians: 0.0,
    };

    pub const fn harmonic(
        offset: f64,
        amplitude: f64,
        frequency_hz: f64,
        phase_radians: f64,
    ) -> Self {
        Self::Harmonic {
            offset,
            amplitude,
            frequency_hz,
            phase_radians,
        }
    }

    pub fn valid(self) -> bool {
        let Self::Harmonic {
            offset,
            amplitude,
            frequency_hz,
            phase_radians,
        } = self;
        offset.is_finite()
            && amplitude.is_finite()
            && frequency_hz.is_finite()
            && frequency_hz >= 0.0
            && phase_radians.is_finite()
    }

    pub fn value(self, time: f64) -> f64 {
        let Self::Harmonic {
            offset,
            amplitude,
            frequency_hz,
            phase_radians,
        } = self;
        offset + amplitude * (std::f64::consts::TAU * frequency_hz * time + phase_radians).sin()
    }

    /// Exact time derivative of the authored signal. Keeping this analytic is
    /// important for prescribed canonical fields: differencing two stored f32
    /// endpoints loses the small per-step change long before the field itself.
    pub fn derivative(self, time: f64) -> f64 {
        let Self::Harmonic {
            amplitude,
            frequency_hz,
            phase_radians,
            ..
        } = self;
        let omega = std::f64::consts::TAU * frequency_hz;
        amplitude * omega * (omega * time + phase_radians).cos()
    }

    pub const fn harmonic_parameters(self) -> [f64; 4] {
        let Self::Harmonic {
            offset,
            amplitude,
            frequency_hz,
            phase_radians,
        } = self;
        [offset, amplitude, frequency_hz, phase_radians]
    }

    pub fn harmonic_parameters_mut(&mut self) -> (&mut f64, &mut f64, &mut f64, &mut f64) {
        let Self::Harmonic {
            offset,
            amplitude,
            frequency_hz,
            phase_radians,
        } = self;
        (offset, amplitude, frequency_hz, phase_radians)
    }

    pub fn frequency_ceiling_hz(self) -> f64 {
        let [_, amplitude, frequency_hz, _] = self.harmonic_parameters();
        if amplitude == 0.0 { 0.0 } else { frequency_hz }
    }

    pub fn characteristic_amplitude(self) -> f64 {
        let [offset, amplitude, _, _] = self.harmonic_parameters();
        if amplitude == 0.0 { offset } else { amplitude }
    }
}

impl Default for TimeSignal {
    fn default() -> Self {
        Self::ZERO
    }
}

/// Periods of the slowest oscillating source that the switch-on ramp spans.
///
/// A sine started from rest at a phase whose cosine is not zero hands the domain
/// a net impulse: integrating `A sin(w t + p)` from rest leaves the term
/// `(A cos(p) / w) t`, a uniform drift. Nothing in a closed or radiating scene
/// removes it — a constant is in the stiffness operator's null space, and an
/// outgoing wall damps velocity rather than position — so in a cavity the
/// offset ramps without bound. Easing the amplitude in suppresses that impulse
/// to about `1 / (w * ramp)` of what it would otherwise be.
///
/// One envelope is shared by every source in a scene rather than one per source,
/// because a phased array steers its beam with the phases *between* its sources
/// and a per-source delay would turn the beam. Sizing the ramp on the slowest
/// source keeps the suppression at least this good for all of them.
pub const SOURCE_RAMP_PERIODS: f64 = 4.0;

/// How long the switch-on envelope runs for a scene whose slowest oscillating
/// source is at `lowest_frequency_hz`.
///
/// Zero when nothing oscillates, which leaves the envelope out of the way: a
/// steady bias accelerates a free domain however gently it is introduced, so
/// there is no impulse for a ramp to suppress.
pub fn source_ramp_seconds(lowest_frequency_hz: f64) -> f64 {
    if !lowest_frequency_hz.is_finite() || lowest_frequency_hz <= 0.0 {
        return 0.0;
    }
    SOURCE_RAMP_PERIODS / lowest_frequency_hz
}

/// Smooth switch-on: zero at the start with zero slope, one after `ramp`. The
/// vanishing slope at both ends is what leaves the residual impulse at second
/// order in `1 / (w * ramp)` rather than first. `wave.wgsl` mirrors this.
pub fn source_envelope(time: f64, ramp: f64) -> f64 {
    if !ramp.is_finite() || ramp <= 0.0 || !time.is_finite() {
        return 1.0;
    }
    let fraction = (time / ramp).clamp(0.0, 1.0);
    fraction * fraction * (3.0 - 2.0 * fraction)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointSource {
    pub enabled: bool,
    pub position: Point2,
    pub width: f64,
    pub region: RegionId,
    pub signal: TimeSignal,
}

impl PointSource {
    pub fn valid(self) -> bool {
        self.position.finite() && self.width.is_finite() && self.width > 0.0 && self.signal.valid()
    }

    pub fn spatial_eq(self, other: Self) -> bool {
        self.position == other.position && self.width == other.width && self.region == other.region
    }
}

impl Default for PointSource {
    fn default() -> Self {
        Self {
            enabled: false,
            position: Point2::new(-0.45, 0.0),
            width: 0.06,
            region: BACKGROUND_REGION,
            // A quarter turn, not zero. Switching a sinusoid on at t = 0 leaves
            // the field a mean velocity of `amplitude * cos(phase) / omega`,
            // because that is what the forcing's running integral keeps. In an
            // open domain it drains through the boundary; a region sealed by
            // reflecting walls has nowhere to put it, so its level rises without
            // bound for as long as the run lasts. A cosine start carries no such
            // impulse and looks the same. Any other phase the user picks does
            // carry one, and that is theirs to choose.
            signal: TimeSignal::harmonic(0.0, 18.0, 2.5, std::f64::consts::FRAC_PI_2),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum OuterBoundaryCondition {
    #[default]
    Reflecting,
    /// Local impedance condition `partial_n u = -u_t / c`.
    FirstOrderOutgoing,
    /// Second-order Engquist-Majda condition using boundary memory `psi_t = u`.
    SecondOrderOutgoing,
    /// A perfect electric wall. It resolves to zero primary field for TM and
    /// zero normal flux for TE.
    ElectricWall,
    /// A perfect magnetic wall. It resolves to zero normal flux for TM and zero
    /// primary field for TE.
    MagneticWall,
    /// Prescribed outward flux `stiffness * partial_n u = value(t)`.
    Neumann { signal: TimeSignal },
    /// Strongly prescribed displacement `u = value(t)`.
    Dirichlet { signal: TimeSignal },
}

impl OuterBoundaryCondition {
    pub fn label(self) -> &'static str {
        match self {
            Self::Reflecting => "Reflecting",
            Self::FirstOrderOutgoing => "First-order outgoing",
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
        self.signal().is_none_or(TimeSignal::valid)
    }

    pub fn resolved(self, physics: PhysicsModel) -> Self {
        match (self, physics) {
            (
                Self::ElectricWall,
                PhysicsModel::Electromagnetic {
                    polarization: ElectromagneticPolarization::Tm,
                },
            )
            | (
                Self::MagneticWall,
                PhysicsModel::Electromagnetic {
                    polarization: ElectromagneticPolarization::Te,
                },
            )
            | (Self::ElectricWall, PhysicsModel::Mechanical) => Self::Dirichlet {
                signal: TimeSignal::ZERO,
            },
            (Self::ElectricWall | Self::MagneticWall, _) => Self::Reflecting,
            _ => self,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OuterBoundaryConditions {
    pub sides: [OuterBoundaryCondition; 4],
}

impl Default for OuterBoundaryConditions {
    fn default() -> Self {
        Self::uniform(OuterBoundaryCondition::SecondOrderOutgoing)
    }
}

impl OuterBoundaryConditions {
    pub const fn uniform(condition: OuterBoundaryCondition) -> Self {
        Self {
            sides: [condition; 4],
        }
    }

    pub const fn get(self, side: OuterSide) -> OuterBoundaryCondition {
        self.sides[side.index()]
    }

    pub fn valid(self) -> bool {
        self.sides.into_iter().all(OuterBoundaryCondition::valid)
    }

    pub fn label(self) -> &'static str {
        if self.sides[1..]
            .iter()
            .all(|condition| *condition == self.sides[0])
        {
            self.sides[0].label()
        } else {
            "Mixed"
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum WaveError {
    /// The one refusal the shared sentence actually describes: the reference
    /// P1 operator was handed coefficients outside its domain. Nothing the
    /// application prepares reaches it - it is the convergence example and the
    /// tests - so a user who sees this text is running the reference, not a
    /// scene. Every other refusal names itself through `Unsupported`.
    InvalidCoefficients,
    /// Something the solver will not accept, named at the point it was found.
    ///
    /// This variant exists because `InvalidCoefficients` used to be shared by
    /// dozens of unrelated checks, and they all reached a user as one sentence
    /// about mass and stiffness - a source whose weights miss their support
    /// read as a bad material, and a refusal could not be located from what the
    /// user could see. Anything reachable while a generation is prepared says
    /// which check it was.
    Unsupported(&'static str),
    MaterialEvaluation {
        material: String,
        coefficient: &'static str,
        point: Point2,
        reason: String,
    },
    InvalidMesh(&'static str),
    InvalidTimeStep {
        requested: f64,
        maximum: f64,
    },
    InvalidState,
    SizeMismatch {
        expected: usize,
        actual: usize,
    },
}

impl std::fmt::Display for WaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCoefficients => write!(
                f,
                "Wave coefficients must be finite with positive mass and stiffness and nonnegative damping"
            ),
            Self::Unsupported(reason) => write!(f, "{reason}"),
            Self::MaterialEvaluation {
                material,
                coefficient,
                point,
                reason,
            } => write!(
                f,
                "Material `{material}` {coefficient} is invalid at ({:.4}, {:.4}): {reason}",
                point.x, point.y
            ),
            Self::InvalidMesh(reason) => write!(f, "Cannot assemble wave operator: {reason}"),
            Self::InvalidTimeStep { requested, maximum } => write!(
                f,
                "Time step {requested:.6} exceeds the conservative maximum {maximum:.6}"
            ),
            Self::InvalidState => write!(f, "Wave state contains a non-finite value"),
            Self::SizeMismatch { expected, actual } => {
                write!(f, "Expected {expected} nodal values, received {actual}")
            }
        }
    }
}

impl std::error::Error for WaveError {}

/// Row-wise P1 stiffness matrix with lumped mass and damping.
///
/// The CSR layout is also the upload format for the GPU gather kernel. Every
/// row contains its diagonal. Reflecting boundaries are the natural Neumann
/// condition of the assembled weak form, so no boundary DOFs are eliminated.
#[derive(Clone, Debug, PartialEq)]
pub struct WaveOperator {
    geometry_revision: u64,
    mesh_revision: u64,
    row_offsets: Vec<u32>,
    columns: Vec<u32>,
    stiffness: Vec<f64>,
    lumped_mass: Vec<f64>,
    lumped_damping: Vec<f64>,
    maximum_eigenvalue_bound: f64,
    maximum_time_step: f64,
}

impl WaveOperator {
    pub fn assemble(mesh: &TriMesh, coefficients: WaveCoefficients) -> Result<Self, WaveError> {
        if !coefficients.mass_density.is_finite()
            || coefficients.mass_density <= 0.0
            || !coefficients.stiffness.is_finite()
            || coefficients.stiffness <= 0.0
            || !coefficients.damping.is_finite()
            || coefficients.damping < 0.0
        {
            return Err(WaveError::InvalidCoefficients);
        }
        let count = mesh.vertices.len();
        if count == 0 || mesh.triangles.is_empty() {
            return Err(WaveError::InvalidMesh("the mesh is empty"));
        }
        if count > u32::MAX as usize {
            return Err(WaveError::InvalidMesh("the mesh has too many vertices"));
        }

        let mut rows = vec![BTreeMap::<usize, f64>::new(); count];
        let mut mass = vec![0.0; count];
        let mut damping = vec![0.0; count];
        for triangle in &mesh.triangles {
            let [i0, i1, i2] = triangle.vertices;
            if i0 >= count || i1 >= count || i2 >= count || i0 == i1 || i1 == i2 || i2 == i0 {
                return Err(WaveError::InvalidMesh(
                    "a triangle has invalid vertex indices",
                ));
            }
            let points = [
                mesh.vertices[i0].point,
                mesh.vertices[i1].point,
                mesh.vertices[i2].point,
            ];
            if points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
                return Err(WaveError::InvalidMesh("a vertex is non-finite"));
            }
            let twice_area = (points[1] - points[0]).cross(points[2] - points[0]);
            if !twice_area.is_finite() || twice_area <= 0.0 {
                return Err(WaveError::InvalidMesh("a triangle has non-positive area"));
            }
            let area = 0.5 * twice_area;
            let gradients = [
                Point2::new(points[1].y - points[2].y, points[2].x - points[1].x) / twice_area,
                Point2::new(points[2].y - points[0].y, points[0].x - points[2].x) / twice_area,
                Point2::new(points[0].y - points[1].y, points[1].x - points[0].x) / twice_area,
            ];
            let indices = [i0, i1, i2];
            for local_i in 0..3 {
                let i = indices[local_i];
                mass[i] += coefficients.mass_density * area / 3.0;
                damping[i] += coefficients.damping * area / 3.0;
                for local_j in 0..3 {
                    let j = indices[local_j];
                    let value =
                        coefficients.stiffness * area * gradients[local_i].dot(gradients[local_j]);
                    *rows[i].entry(j).or_default() += value;
                }
            }
        }
        if mass.iter().any(|value| !value.is_finite() || *value <= 0.0)
            || damping
                .iter()
                .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err(WaveError::InvalidMesh("a vertex has invalid lumped mass"));
        }

        let entries = rows.iter().map(BTreeMap::len).sum::<usize>();
        if entries > u32::MAX as usize {
            return Err(WaveError::InvalidMesh(
                "the wave operator has too many entries",
            ));
        }
        let mut row_offsets = Vec::with_capacity(count + 1);
        let mut columns = Vec::with_capacity(entries);
        let mut stiffness = Vec::with_capacity(entries);
        row_offsets.push(0);
        let mut maximum_eigenvalue_bound = 0.0_f64;
        for (row, values) in rows.into_iter().enumerate() {
            let absolute_sum = values.values().map(|value| value.abs()).sum::<f64>();
            maximum_eigenvalue_bound = maximum_eigenvalue_bound.max(absolute_sum / mass[row]);
            for (column, value) in values {
                if !value.is_finite() {
                    return Err(WaveError::InvalidMesh("the stiffness matrix is non-finite"));
                }
                columns.push(column as u32);
                stiffness.push(value);
            }
            row_offsets.push(columns.len() as u32);
        }
        if !maximum_eigenvalue_bound.is_finite() || maximum_eigenvalue_bound <= 0.0 {
            return Err(WaveError::InvalidMesh(
                "the stiffness bound is not positive",
            ));
        }
        let maximum_time_step = 2.0 / maximum_eigenvalue_bound.sqrt();
        Ok(Self {
            geometry_revision: mesh.geometry_revision,
            mesh_revision: mesh.mesh_revision,
            row_offsets,
            columns,
            stiffness,
            lumped_mass: mass,
            lumped_damping: damping,
            maximum_eigenvalue_bound,
            maximum_time_step,
        })
    }

    pub fn geometry_revision(&self) -> u64 {
        self.geometry_revision
    }

    pub fn mesh_revision(&self) -> u64 {
        self.mesh_revision
    }

    pub fn degrees_of_freedom(&self) -> usize {
        self.lumped_mass.len()
    }

    pub fn row_offsets(&self) -> &[u32] {
        &self.row_offsets
    }

    pub fn columns(&self) -> &[u32] {
        &self.columns
    }

    pub fn stiffness_values(&self) -> &[f64] {
        &self.stiffness
    }

    pub fn lumped_mass(&self) -> &[f64] {
        &self.lumped_mass
    }

    pub fn lumped_damping(&self) -> &[f64] {
        &self.lumped_damping
    }

    pub fn maximum_eigenvalue_bound(&self) -> f64 {
        self.maximum_eigenvalue_bound
    }

    pub fn maximum_time_step(&self) -> f64 {
        self.maximum_time_step
    }

    pub fn recommended_time_step(&self) -> f64 {
        0.9 * self.maximum_time_step
    }

    pub fn apply_stiffness(&self, values: &[f64]) -> Result<Vec<f64>, WaveError> {
        if values.len() != self.degrees_of_freedom() {
            return Err(WaveError::SizeMismatch {
                expected: self.degrees_of_freedom(),
                actual: values.len(),
            });
        }
        let mut result = vec![0.0; values.len()];
        for (row, output) in result.iter_mut().enumerate() {
            let start = self.row_offsets[row] as usize;
            let end = self.row_offsets[row + 1] as usize;
            *output = (start..end)
                .map(|entry| self.stiffness[entry] * values[self.columns[entry] as usize])
                .sum();
        }
        Ok(result)
    }

    /// Values for the GPU gather kernel, normalized row-wise by lumped mass.
    pub fn normalized_stiffness_f32(&self) -> Result<Vec<f32>, WaveError> {
        let mut result = Vec::with_capacity(self.stiffness.len());
        for row in 0..self.degrees_of_freedom() {
            let start = self.row_offsets[row] as usize;
            let end = self.row_offsets[row + 1] as usize;
            for entry in start..end {
                let value = (self.stiffness[entry] / self.lumped_mass[row]) as f32;
                if !value.is_finite() {
                    return Err(WaveError::InvalidMesh("the f32 GPU operator overflows"));
                }
                result.push(value);
            }
        }
        Ok(result)
    }

    pub fn damping_ratios_f32(&self) -> Result<Vec<f32>, WaveError> {
        self.lumped_damping
            .iter()
            .zip(&self.lumped_mass)
            .map(|(damping, mass)| {
                let value = (damping / mass) as f32;
                value
                    .is_finite()
                    .then_some(value)
                    .ok_or(WaveError::InvalidMesh(
                        "the f32 GPU damping ratio overflows",
                    ))
            })
            .collect()
    }

    pub fn discrete_energy(
        &self,
        current: &[f64],
        previous: &[f64],
        time_step: f64,
    ) -> Result<f64, WaveError> {
        self.validate_levels(current, previous, time_step)?;
        let stiffness_previous = self.apply_stiffness(previous)?;
        let kinetic = current
            .iter()
            .zip(previous)
            .zip(&self.lumped_mass)
            .map(|((current, previous), mass)| {
                let velocity = (current - previous) / time_step;
                0.5 * mass * velocity * velocity
            })
            .sum::<f64>();
        let potential = 0.5
            * current
                .iter()
                .zip(stiffness_previous)
                .map(|(current, force)| current * force)
                .sum::<f64>();
        let energy = kinetic + potential;
        if energy.is_finite() {
            Ok(energy)
        } else {
            Err(WaveError::InvalidState)
        }
    }

    fn validate_levels(
        &self,
        current: &[f64],
        previous: &[f64],
        time_step: f64,
    ) -> Result<(), WaveError> {
        for values in [current, previous] {
            if values.len() != self.degrees_of_freedom() {
                return Err(WaveError::SizeMismatch {
                    expected: self.degrees_of_freedom(),
                    actual: values.len(),
                });
            }
            if values.iter().any(|value| !value.is_finite()) {
                return Err(WaveError::InvalidState);
            }
        }
        if !time_step.is_finite() || time_step <= 0.0 || time_step > self.maximum_time_step {
            return Err(WaveError::InvalidTimeStep {
                requested: time_step,
                maximum: self.maximum_time_step,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WaveState {
    current: Vec<f64>,
    previous: Vec<f64>,
    scratch: Vec<f64>,
    time_step: f64,
    steps: u64,
}

impl WaveState {
    pub fn new(
        operator: &WaveOperator,
        time_step: f64,
        displacement: Vec<f64>,
        velocity: Vec<f64>,
    ) -> Result<Self, WaveError> {
        operator.validate_levels(&displacement, &velocity, time_step)?;
        let stiffness = operator.apply_stiffness(&displacement)?;
        let previous = displacement
            .iter()
            .zip(&velocity)
            .zip(stiffness)
            .zip(&operator.lumped_mass)
            .zip(&operator.lumped_damping)
            .map(|((((displacement, velocity), stiffness), mass), damping)| {
                let acceleration = (-stiffness - damping * velocity) / mass;
                displacement - time_step * velocity + 0.5 * time_step * time_step * acceleration
            })
            .collect::<Vec<_>>();
        if previous.iter().any(|value| !value.is_finite()) {
            return Err(WaveError::InvalidState);
        }
        Ok(Self {
            current: displacement,
            previous,
            scratch: vec![0.0; operator.degrees_of_freedom()],
            time_step,
            steps: 0,
        })
    }

    pub fn zero(operator: &WaveOperator, time_step: f64) -> Result<Self, WaveError> {
        Self::new(
            operator,
            time_step,
            vec![0.0; operator.degrees_of_freedom()],
            vec![0.0; operator.degrees_of_freedom()],
        )
    }

    pub fn current(&self) -> &[f64] {
        &self.current
    }

    pub fn previous(&self) -> &[f64] {
        &self.previous
    }

    pub fn time_step(&self) -> f64 {
        self.time_step
    }

    pub fn steps(&self) -> u64 {
        self.steps
    }

    pub fn time(&self) -> f64 {
        self.steps as f64 * self.time_step
    }

    /// Advance one centered step. `acceleration` is the nodal source `f / M`;
    /// pass an empty slice for an unforced step.
    pub fn step(&mut self, operator: &WaveOperator, acceleration: &[f64]) -> Result<(), WaveError> {
        operator.validate_levels(&self.current, &self.previous, self.time_step)?;
        if !acceleration.is_empty() && acceleration.len() != self.current.len() {
            return Err(WaveError::SizeMismatch {
                expected: self.current.len(),
                actual: acceleration.len(),
            });
        }
        if acceleration.iter().any(|value| !value.is_finite()) {
            return Err(WaveError::InvalidState);
        }
        let dt = self.time_step;
        let dt2 = dt * dt;
        for i in 0..self.current.len() {
            let start = operator.row_offsets[i] as usize;
            let end = operator.row_offsets[i + 1] as usize;
            let stiffness = (start..end)
                .map(|entry| {
                    operator.stiffness[entry] * self.current[operator.columns[entry] as usize]
                })
                .sum::<f64>();
            let gamma = operator.lumped_damping[i] / operator.lumped_mass[i];
            let source = acceleration.get(i).copied().unwrap_or(0.0);
            let value = (2.0 * self.current[i]
                - (1.0 - 0.5 * gamma * dt) * self.previous[i]
                - dt2 * stiffness / operator.lumped_mass[i]
                + dt2 * source)
                / (1.0 + 0.5 * gamma * dt);
            if !value.is_finite() {
                return Err(WaveError::InvalidState);
            }
            self.scratch[i] = value;
        }
        std::mem::swap(&mut self.previous, &mut self.current);
        std::mem::swap(&mut self.current, &mut self.scratch);
        self.steps = self.steps.checked_add(1).ok_or(WaveError::InvalidState)?;
        Ok(())
    }

    pub fn add_displacement(&mut self, values: &[f64]) -> Result<(), WaveError> {
        if values.len() != self.current.len() {
            return Err(WaveError::SizeMismatch {
                expected: self.current.len(),
                actual: values.len(),
            });
        }
        for ((current, previous), addition) in
            self.current.iter_mut().zip(&mut self.previous).zip(values)
        {
            if !addition.is_finite() {
                return Err(WaveError::InvalidState);
            }
            *current += addition;
            *previous += addition;
        }
        Ok(())
    }

    pub fn energy(&self, operator: &WaveOperator) -> Result<f64, WaveError> {
        operator.discrete_energy(&self.current, &self.previous, self.time_step)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BACKGROUND_REGION, MeshQuality, MeshTriangle, MeshVertex, ScalarField};

    #[test]
    fn harmonic_time_signal_evaluates_and_reports_its_bandwidth() {
        let signal = TimeSignal::harmonic(0.25, 2.0, 3.0, 0.5);
        assert!(signal.valid());
        assert!((signal.value(0.0) - (0.25 + 2.0 * 0.5_f64.sin())).abs() < 1.0e-14);
        assert!(
            (signal.value(0.125) - (0.25 + 2.0 * (0.375 * std::f64::consts::TAU + 0.5).sin()))
                .abs()
                < 1.0e-14
        );
        assert_eq!(signal.frequency_ceiling_hz(), 3.0);
        assert_eq!(signal.characteristic_amplitude(), 2.0);

        let constant = TimeSignal::harmonic(-0.4, 0.0, 8.0, 1.2);
        assert_eq!(constant.value(10.0), -0.4);
        assert_eq!(constant.frequency_ceiling_hz(), 0.0);
        assert_eq!(constant.characteristic_amplitude(), -0.4);
    }

    #[test]
    fn time_signal_and_point_source_validation_reject_malformed_values() {
        assert!(!TimeSignal::harmonic(0.0, 1.0, -1.0, 0.0).valid());
        assert!(!TimeSignal::harmonic(f64::NAN, 1.0, 1.0, 0.0).valid());
        assert!(!TimeSignal::harmonic(0.0, 1.0, 1.0, f64::INFINITY).valid());

        let source = PointSource {
            enabled: true,
            ..Default::default()
        };
        assert!(source.valid());
        assert!(source.spatial_eq(PointSource {
            enabled: false,
            signal: TimeSignal::harmonic(1.0, 4.0, 7.0, 0.3),
            ..source
        }));
        assert!(!source.spatial_eq(PointSource {
            width: source.width * 2.0,
            ..source
        }));
        assert!(
            !PointSource {
                width: 0.0,
                ..source
            }
            .valid()
        );
    }

    fn grid(n: usize) -> TriMesh {
        let mut vertices = Vec::new();
        for y in 0..=n {
            for x in 0..=n {
                vertices.push(MeshVertex {
                    point: Point2::new(
                        -1.0 + 2.0 * x as f64 / n as f64,
                        -1.0 + 2.0 * y as f64 / n as f64,
                    ),
                    boundary: None,
                    trace: None,
                });
            }
        }
        let index = |x: usize, y: usize| y * (n + 1) + x;
        let mut triangles = Vec::new();
        for y in 0..n {
            for x in 0..n {
                let a = index(x, y);
                let b = index(x + 1, y);
                let c = index(x, y + 1);
                let d = index(x + 1, y + 1);
                if (x + y) % 2 == 0 {
                    triangles.push(MeshTriangle {
                        vertices: [a, b, d],
                        region: BACKGROUND_REGION,
                    });
                    triangles.push(MeshTriangle {
                        vertices: [a, d, c],
                        region: BACKGROUND_REGION,
                    });
                } else {
                    triangles.push(MeshTriangle {
                        vertices: [a, b, c],
                        region: BACKGROUND_REGION,
                    });
                    triangles.push(MeshTriangle {
                        vertices: [b, d, c],
                        region: BACKGROUND_REGION,
                    });
                }
            }
        }
        TriMesh {
            geometry_revision: 7,
            mesh_revision: 7,
            vertices,
            triangles,
            boundary_edges: Vec::new(),
            requested_sizes: vec![],
            quality: MeshQuality {
                minimum_angle_degrees: 45.0,
                maximum_edge_length: 2.0_f64.sqrt() * 2.0 / n as f64,
            },
        }
    }

    #[test]
    fn assembly_is_symmetric_and_annihilates_constants() {
        let operator = WaveOperator::assemble(&grid(5), WaveCoefficients::default()).unwrap();
        let constant = vec![1.0; operator.degrees_of_freedom()];
        let applied = operator.apply_stiffness(&constant).unwrap();
        assert!(applied.iter().all(|value| value.abs() < 1.0e-12));
        for row in 0..operator.degrees_of_freedom() {
            for entry in operator.row_offsets[row] as usize..operator.row_offsets[row + 1] as usize
            {
                let column = operator.columns[entry] as usize;
                let reverse = (operator.row_offsets[column] as usize
                    ..operator.row_offsets[column + 1] as usize)
                    .find(|other| operator.columns[*other] as usize == row)
                    .unwrap();
                assert!((operator.stiffness[entry] - operator.stiffness[reverse]).abs() < 1.0e-12);
            }
        }
        assert_eq!(operator.geometry_revision(), 7);
        assert!(operator.recommended_time_step() > 0.0);
    }

    #[test]
    fn reflecting_constant_state_is_stationary() {
        let operator = WaveOperator::assemble(&grid(6), WaveCoefficients::default()).unwrap();
        let dt = operator.recommended_time_step();
        let count = operator.degrees_of_freedom();
        let mut state = WaveState::new(&operator, dt, vec![0.7; count], vec![0.0; count]).unwrap();
        for _ in 0..500 {
            state.step(&operator, &[]).unwrap();
        }
        assert!(
            state
                .current()
                .iter()
                .all(|value| (*value - 0.7).abs() < 1.0e-11)
        );
    }

    #[test]
    fn undamped_discrete_energy_is_conserved() {
        let mesh = grid(12);
        let operator = WaveOperator::assemble(&mesh, WaveCoefficients::default()).unwrap();
        let dt = 0.5 * operator.maximum_time_step();
        let initial = mesh
            .vertices
            .iter()
            .map(|vertex| (std::f64::consts::PI * (vertex.point.x + 1.0)).cos())
            .collect();
        let mut state = WaveState::new(
            &operator,
            dt,
            initial,
            vec![0.0; operator.degrees_of_freedom()],
        )
        .unwrap();
        state.step(&operator, &[]).unwrap();
        let energy = state.energy(&operator).unwrap();
        for _ in 0..2_000 {
            state.step(&operator, &[]).unwrap();
            let relative = (state.energy(&operator).unwrap() - energy).abs() / energy;
            assert!(relative < 2.0e-10, "relative drift {relative}");
        }
    }

    #[test]
    fn damping_reduces_energy() {
        let mesh = grid(10);
        let operator = WaveOperator::assemble(
            &mesh,
            WaveCoefficients {
                damping: 0.4,
                ..Default::default()
            },
        )
        .unwrap();
        let dt = 0.5 * operator.maximum_time_step();
        let initial = mesh.vertices.iter().map(|v| v.point.x).collect();
        let mut state = WaveState::new(
            &operator,
            dt,
            initial,
            vec![0.0; operator.degrees_of_freedom()],
        )
        .unwrap();
        state.step(&operator, &[]).unwrap();
        let first = state.energy(&operator).unwrap();
        for _ in 0..300 {
            state.step(&operator, &[]).unwrap();
        }
        assert!(state.energy(&operator).unwrap() < 0.2 * first);
    }

    #[test]
    fn box_mode_converges_under_spatial_refinement() {
        fn error(n: usize) -> f64 {
            let mesh = grid(n);
            let operator = WaveOperator::assemble(&mesh, WaveCoefficients::default()).unwrap();
            let dt = 0.18 * operator.maximum_time_step();
            let wave_number = 2.0 * std::f64::consts::PI;
            let initial: Vec<_> = mesh
                .vertices
                .iter()
                .map(|vertex| (wave_number * (vertex.point.x + 1.0) / 2.0).cos())
                .collect();
            let mut state = WaveState::new(
                &operator,
                dt,
                initial.clone(),
                vec![0.0; operator.degrees_of_freedom()],
            )
            .unwrap();
            let target = 2.0;
            while state.time() + 0.5 * dt < target {
                state.step(&operator, &[]).unwrap();
            }
            let exact_factor = (wave_number * state.time() / 2.0).cos();
            let numerator = state
                .current()
                .iter()
                .zip(&initial)
                .zip(operator.lumped_mass())
                .map(|((actual, spatial), mass)| mass * (actual - exact_factor * spatial).powi(2))
                .sum::<f64>();
            let denominator = initial
                .iter()
                .zip(operator.lumped_mass())
                .map(|(value, mass)| mass * value * value)
                .sum::<f64>();
            (numerator / denominator).sqrt()
        }
        let coarse = error(12);
        let fine = error(24);
        assert!(fine < 0.4 * coarse, "coarse {coarse}, fine {fine}");
    }

    #[test]
    fn rejects_bad_coefficients_mesh_state_and_timestep() {
        let mesh = grid(2);
        assert!(matches!(
            WaveOperator::assemble(
                &mesh,
                WaveCoefficients {
                    mass_density: 0.0,
                    ..Default::default()
                }
            ),
            Err(WaveError::InvalidCoefficients)
        ));
        let operator = WaveOperator::assemble(&mesh, WaveCoefficients::default()).unwrap();
        assert!(matches!(
            WaveState::zero(&operator, operator.maximum_time_step() * 1.01),
            Err(WaveError::InvalidTimeStep { .. })
        ));
        let mut broken = mesh;
        broken.triangles[0].vertices = [0, 0, 1];
        assert!(matches!(
            WaveOperator::assemble(&broken, WaveCoefficients::default()),
            Err(WaveError::InvalidMesh(_))
        ));
    }

    #[test]
    fn scalar_skins_compile_material_properties_consistently() {
        let raw = WaveCoefficients {
            mass_density: 4.0,
            stiffness: 9.0,
            damping: 0.5,
        };
        assert_eq!(PhysicsModel::Mechanical.wave_coefficients(raw), raw);
        let tm = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        };
        let te = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        };
        assert_eq!(
            tm.wave_coefficients(raw),
            WaveCoefficients {
                mass_density: 4.0,
                stiffness: 1.0 / 9.0,
                damping: 2.0,
            }
        );
        assert_eq!(
            te.wave_coefficients(raw),
            WaveCoefficients {
                mass_density: 9.0,
                stiffness: 0.25,
                damping: 4.5,
            }
        );
        for physics in [tm, te] {
            assert!((physics.wave_speed(raw) - 1.0 / 6.0).abs() < 1.0e-15);
            assert!((physics.impedance(raw) - 1.5).abs() < 1.0e-15);
        }
    }

    #[test]
    fn physics_material_conversion_preserves_characteristics_and_stays_bounded() {
        let mechanical = PhysicsModel::Mechanical;
        let tm = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        };
        let te = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        };
        let mut material = Material::default_medium();
        material.mass_density = ScalarField::formula("2 + 0.2*x*x").unwrap();
        material.stiffness = ScalarField::formula("5 - 0.3*y").unwrap();
        material.damping = ScalarField::formula("0.1 + 0.02*r").unwrap();
        material.axis_ratio = ScalarField::formula("1 + abs(x)").unwrap();

        let electromagnetic = mechanical.convert_material(tm, &material).unwrap();
        assert_eq!(
            tm.convert_material(te, &electromagnetic).unwrap(),
            electromagnetic
        );
        for point in [
            Point2::new(-0.7, -0.2),
            Point2::new(0.0, 0.0),
            Point2::new(0.4, 0.8),
        ] {
            let frame = crate::MaterialFrame::world();
            let old = material.evaluate(frame, point).unwrap();
            let new = electromagnetic.evaluate(frame, point).unwrap();
            assert_eq!(old.axis_ratio, new.axis_ratio);
            let old = WaveCoefficients {
                mass_density: old.mass_density,
                stiffness: old.stiffness,
                damping: old.damping,
            };
            let new = WaveCoefficients {
                mass_density: new.mass_density,
                stiffness: new.stiffness,
                damping: new.damping,
            };
            assert!((mechanical.wave_speed(old) - tm.wave_speed(new)).abs() < 1.0e-14);
            assert!((mechanical.impedance(old) - tm.impedance(new)).abs() < 1.0e-14);
            assert!((old.damping / old.mass_density - new.damping).abs() < 1.0e-14);
        }

        let mut physics = tm;
        let mut cycled = electromagnetic;
        let initial_lengths = [
            cycled.mass_density.source().unwrap().len(),
            cycled.stiffness.source().unwrap().len(),
            cycled.damping.source().unwrap().len(),
        ];
        for _ in 0..32 {
            cycled = physics.convert_material(mechanical, &cycled).unwrap();
            physics = mechanical;
            cycled = physics.convert_material(tm, &cycled).unwrap();
            physics = tm;
        }
        assert_eq!(
            [
                cycled.mass_density.source().unwrap().len(),
                cycled.stiffness.source().unwrap().len(),
                cycled.damping.source().unwrap().len(),
            ],
            initial_lengths
        );
        assert_eq!(cycled, mechanical.convert_material(tm, &material).unwrap());

        let mut inert_reciprocal = material.clone();
        inert_reciprocal.mass_law.inverted = true;
        let converted = mechanical.convert_material(tm, &inert_reciprocal).unwrap();
        assert_eq!(converted.mass_law, crate::CoefficientLaw::linear());

        // A direct Kerr or saturable law carries to the same physical
        // coefficient; a signed or reciprocal one has nowhere settled to go.
        let mut nonlinear = material;
        let saturable = crate::FieldLaw::Saturable {
            chi: ScalarField::constant(0.2),
            saturation: ScalarField::constant(1.0),
        };
        nonlinear.mass_law.field = saturable.clone();
        let converted = mechanical.convert_material(tm, &nonlinear).unwrap();
        assert_eq!(converted.stiffness_law.field, saturable);
        assert!(!converted.stiffness_law.inverted);
        let returned = tm.convert_material(mechanical, &converted).unwrap();
        assert_eq!(returned.mass_law, nonlinear.mass_law);
        assert_eq!(returned.stiffness_law, nonlinear.stiffness_law);
        let refused = Err(MaterialError::UnconvertibleMaterialLaw(
            "a signed or reciprocal field response",
        ));
        let mut signed = nonlinear.clone();
        signed.mass_law.field = crate::FieldLaw::Polynomial {
            chi1: ScalarField::constant(0.3),
            chi2: ScalarField::constant(0.0),
            amplitude_bound: Some(ScalarField::constant(0.5)),
        };
        assert_eq!(mechanical.convert_material(tm, &signed), refused);
        let mut reciprocal = nonlinear;
        reciprocal.mass_law.inverted = true;
        assert_eq!(mechanical.convert_material(tm, &reciprocal), refused);
    }

    /// A drive follows the physical coefficient it was authored against, not
    /// the slot it sits in. Under the mechanical adapter `ε = s₀ = 1/k₀` and
    /// `μ = ρ`, and a law on the stiffness row acts on `s₀`, which the solver
    /// multiplies directly. So the law lands on the permittivity row
    /// unchanged. This measures that: the converted permittivity must equal
    /// the original's `s₀(t) = h(t)/k₀` at every instant. An earlier version
    /// asserted `1/(k₀ h)`, the stored stiffness driven, and so pinned a
    /// conversion that ran the drive backwards in the other skin.
    #[test]
    fn a_drive_crosses_a_skin_on_the_coefficient_it_was_authored_against() {
        use crate::{
            CoefficientLaw, MaterialCoordinates, MaterialSwitchRuntime, ScalarField, TimeDrive,
            TimeDriveRuntime,
        };
        let mechanical = PhysicsModel::Mechanical;
        let tm = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        };
        let origin = MaterialCoordinates {
            x: 0.0,
            y: 0.0,
            r: 0.0,
            theta: 0.0,
        };
        let pump = TimeDrive::ParametricPump {
            depth: ScalarField::constant(0.3),
            frequency_hz: ScalarField::constant(1.7),
            phase_radians: ScalarField::constant(0.4),
        };

        let mut material = Material::default_medium();
        material.mass_density = ScalarField::constant(2.5);
        material.stiffness = ScalarField::constant(4.0);
        material.stiffness_law.drive = pump.clone();
        material.stiffness_law.alternate = Some(ScalarField::constant(1.6));

        let converted = mechanical.convert_material(tm, &material).unwrap();
        // The law left the stiffness row entirely and arrived as it was.
        assert_eq!(converted.stiffness_law, CoefficientLaw::linear());
        assert_eq!(converted.mass_law, material.stiffness_law);

        let coefficient = |field: &ScalarField, law: &CoefficientLaw, time: f64| {
            let values = law.evaluate_at(origin, &material.parameters).unwrap();
            let runtime = TimeDriveRuntime::authored(values.drive).unwrap();
            // Mid-ramp at every sample below, so the alternate participates
            // rather than sitting at a blend of zero where it cancels.
            let switch = MaterialSwitchRuntime::restored(0.0, 1.0, 0.0, 2.0).unwrap();
            field.evaluate(origin, &material.parameters).unwrap()
                * values
                    .temporal_factor(time, origin, runtime, switch)
                    .unwrap()
        };
        for step in 0..12 {
            let time = 0.07 * f64::from(step);
            let reciprocal_stiffness = coefficient(
                &material.stiffness.reciprocal().unwrap(),
                &material.stiffness_law,
                time,
            );
            let permittivity = coefficient(&converted.mass_density, &converted.mass_law, time);
            assert!(
                (permittivity - reciprocal_stiffness).abs() < 1.0e-13,
                "at t={time}: ε {permittivity:e} against s₀ {reciprocal_stiffness:e}"
            );
        }

        // And the round trip is exact.
        let returned = tm.convert_material(mechanical, &converted).unwrap();
        assert_eq!(returned, material);
    }

    /// A drive on the row that does not reciprocate carries over untouched, and
    /// the two skins that share their material data do not rewrite it at all.
    #[test]
    fn the_matched_row_carries_over_and_tm_te_rewrites_nothing() {
        use crate::{ScalarField, TimeDrive};
        let mechanical = PhysicsModel::Mechanical;
        let tm = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        };
        let te = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        };
        let mut material = Material::default_medium();
        material.mass_law.drive = TimeDrive::TimeCrystal {
            depth: ScalarField::constant(0.25),
            frequency_hz: ScalarField::constant(0.8),
            phase_radians: ScalarField::constant(0.0),
            sharpness: ScalarField::constant(3.0),
        };
        let converted = mechanical.convert_material(tm, &material).unwrap();
        assert_eq!(converted.stiffness_law, material.mass_law);
        assert!(!converted.stiffness_law.inverted);
        assert_eq!(tm.convert_material(te, &converted).unwrap(), converted);
    }

    /// What a skin change still refuses, and why each one is named. The editor
    /// prints these, so a user in the preset view learns which slot to clear
    /// rather than being told a material is unsupported.
    #[test]
    fn a_skin_change_names_the_law_it_cannot_carry() {
        use crate::{RestoringLaw, ScalarField};
        let mechanical = PhysicsModel::Mechanical;
        let tm = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        };
        let base = Material::default_medium();

        let mut restoring = base.clone();
        restoring.restoring = RestoringLaw::KleinGordon {
            omega0: ScalarField::constant(2.0),
        };
        assert_eq!(
            mechanical.convert_material(tm, &restoring),
            Err(MaterialError::UnconvertibleMaterialLaw("a restoring law"))
        );

        let mut lossy = base;
        lossy.electric_loss = Some(crate::LossChannel {
            base_rate: ScalarField::constant(0.1),
            law: crate::DampingLaw {
                rate: crate::RateLaw::Constant,
                drive: crate::TimeDrive::None,
            },
        });
        assert_eq!(
            mechanical.convert_material(tm, &lossy),
            Err(MaterialError::UnconvertibleMaterialLaw(
                "a named loss channel"
            ))
        );
    }

    #[test]
    fn directional_skin_keeps_ratio_and_geometric_mean() {
        let properties = EvaluatedMaterial {
            mass_density: 4.0,
            stiffness: 9.0,
            damping: 0.5,
            axis_ratio: 3.0,
        };
        let frame = MaterialFrame {
            angle_radians: 0.41,
            ..MaterialFrame::world()
        };
        for physics in [
            PhysicsModel::Mechanical,
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            },
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            },
        ] {
            let coefficients = physics.directional_wave_coefficients(properties, frame);
            let eigenvalues = coefficients.stiffness.eigenvalues();
            assert!((eigenvalues[1] / eigenvalues[0] - 9.0).abs() < 1.0e-12);
            assert!(coefficients.valid());
            assert!(
                (coefficients.maximum_wave_speed() / coefficients.minimum_wave_speed() - 3.0).abs()
                    < 1.0e-12
            );
        }
    }

    #[test]
    fn electromagnetic_walls_resolve_for_tm_and_te() {
        let tm = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        };
        let te = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        };
        assert!(matches!(
            OuterBoundaryCondition::ElectricWall.resolved(tm),
            OuterBoundaryCondition::Dirichlet {
                signal: TimeSignal::ZERO
            }
        ));
        assert_eq!(
            OuterBoundaryCondition::MagneticWall.resolved(tm),
            OuterBoundaryCondition::Reflecting
        );
        assert_eq!(
            OuterBoundaryCondition::ElectricWall.resolved(te),
            OuterBoundaryCondition::Reflecting
        );
        assert!(matches!(
            OuterBoundaryCondition::MagneticWall.resolved(te),
            OuterBoundaryCondition::Dirichlet {
                signal: TimeSignal::ZERO
            }
        ));
    }
}
