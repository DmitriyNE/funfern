//! Linear CPU reference for the direct canonical `(Q, b)` formulation.
//!
//! It keeps each nodal constitutive contribution and each quadrature vector
//! sample explicit. The f64 implementation remains the numerical oracle for
//! the production f32 GPU path and its source, boundary and transfer stages.

use std::{collections::BTreeSet, sync::Arc};

use crate::{
    CompiledVolumeSources, DirectionalWaveCoefficients, ElectromagneticPolarization, Material,
    MaterialFrame, OuterBoundaryCondition, OuterSide, OwnedTopologyWaveModel, PhysicsModel, Point2,
    PointSource, QuadraticWaveOperator, Scene, SymmetricTensor2, ThinGapSample, TimeSignal,
    TopologyWaveModel, TriMesh, VolumeSource, WaveError, enriched_quadratic_basis_gradients,
    source_reach::point_source_squared_distances,
};

const LOCAL_NODES: usize = 7;
const QUADRATURE_SAMPLES: usize = 6;
const MASS_WEIGHTS: [f64; LOCAL_NODES] = [
    1.0 / 20.0,
    1.0 / 20.0,
    1.0 / 20.0,
    2.0 / 15.0,
    2.0 / 15.0,
    2.0 / 15.0,
    9.0 / 20.0,
];

/// The time every integrated source is anchored at: the application
/// compiles its point and region sources and its Neumann sides against
/// absolute time, so a harmonic source drives the rate `∫₀ᵗ a` and a Neumann
/// side imposes `σ·n = ∫₀ᵗ g` on the flux the direct state holds, whatever
/// generation either was compiled in. The estimator reads that same integral
/// at a wall.
pub const SOURCE_ANCHOR_TIME: f64 = 0.0;

/// Time law for an integrated source in the direct first-order system.
///
/// `Direct` is already authored in primary-field-rate units. The legacy form
/// is the analytic integral of a version-22 acceleration and therefore remains
/// finite and continuous at zero frequency instead of dividing by `omega`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CanonicalRateDrive {
    Direct(TimeSignal),
    LegacyIntegratedHarmonic {
        acceleration: TimeSignal,
        anchor_time: f64,
        rate_anchor: f64,
    },
}

impl CanonicalRateDrive {
    pub fn direct(signal: TimeSignal) -> Result<Self, WaveError> {
        signal
            .valid()
            .then_some(Self::Direct(signal))
            .ok_or(WaveError::Unsupported(
                "a source signal is not a valid waveform",
            ))
    }

    pub fn legacy(signal: TimeSignal, anchor_time: f64) -> Result<Self, WaveError> {
        Self::legacy_with_rate_anchor(signal, anchor_time, 0.0)
    }

    /// The drive an authored source or Neumann signal compiles to: a
    /// harmonic is the version-22 acceleration, integrated from
    /// `anchor_time`, and a pulse is imposed as authored.
    pub fn authored(signal: TimeSignal, anchor_time: f64) -> Result<Self, WaveError> {
        if signal.is_pulsed() {
            Self::direct(signal)
        } else {
            Self::legacy(signal, anchor_time)
        }
    }

    pub fn legacy_with_rate_anchor(
        acceleration: TimeSignal,
        anchor_time: f64,
        rate_anchor: f64,
    ) -> Result<Self, WaveError> {
        // A pulse is never integrated: its area would stay behind as a
        // steady drive once it was over.
        if acceleration.valid()
            && !acceleration.is_pulsed()
            && anchor_time.is_finite()
            && rate_anchor.is_finite()
        {
            Ok(Self::LegacyIntegratedHarmonic {
                acceleration,
                anchor_time,
                rate_anchor,
            })
        } else {
            Err(WaveError::Unsupported(
                "a source rate anchor is not a valid waveform",
            ))
        }
    }

    pub fn value(self, time: f64) -> Result<f64, WaveError> {
        if !time.is_finite() {
            return Err(WaveError::InvalidState);
        }
        let value = match self {
            Self::Direct(signal) => signal.value(time),
            Self::LegacyIntegratedHarmonic {
                acceleration,
                anchor_time,
                rate_anchor,
            } => {
                let [offset, amplitude, frequency_hz, phase] = acceleration.carrier();
                let elapsed = time - anchor_time;
                let omega_elapsed = std::f64::consts::TAU * frequency_hz * elapsed;
                rate_anchor
                    + offset * elapsed
                    + amplitude
                        * elapsed
                        * sinc(0.5 * omega_elapsed)
                        * (phase + 0.5 * omega_elapsed).sin()
            }
        };
        value
            .is_finite()
            .then_some(value)
            .ok_or(WaveError::InvalidState)
    }

    pub fn frequency_ceiling_hz(self) -> f64 {
        match self {
            Self::Direct(signal) => signal.frequency_ceiling_hz(),
            Self::LegacyIntegratedHarmonic { acceleration, .. } => {
                acceleration.frequency_ceiling_hz()
            }
        }
    }
}

/// One spatially integrated source channel. `weights` have the units of the
/// canonical `Q`; the drive supplies the primary-field rate.
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalSource {
    weights: Vec<f64>,
    /// Structural GPU contribution slots. A supported node may carry a zero
    /// instantaneous weight; keeping that slot is what lets a moving source
    /// cross floating-point underflow boundaries without changing the packed
    /// solver layout.
    support: Vec<bool>,
    drive: CanonicalRateDrive,
}

impl CanonicalSource {
    pub fn new(
        operator: &CanonicalWaveOperator,
        weights: Vec<f64>,
        drive: CanonicalRateDrive,
    ) -> Result<Self, WaveError> {
        let support = weights.iter().map(|weight| *weight != 0.0).collect();
        Self::new_with_support(operator, weights, support, drive)
    }

    pub fn new_with_support(
        operator: &CanonicalWaveOperator,
        weights: Vec<f64>,
        support: Vec<bool>,
        drive: CanonicalRateDrive,
    ) -> Result<Self, WaveError> {
        operator.validate_primary(&weights)?;
        if support.len() != weights.len()
            || weights
                .iter()
                .zip(&support)
                .any(|(weight, supported)| *weight != 0.0 && !supported)
        {
            return Err(WaveError::Unsupported(
                "a source drives a node its own support does not cover",
            ));
        }
        Ok(Self {
            weights,
            support,
            drive,
        })
    }

    pub fn direct(
        operator: &CanonicalWaveOperator,
        weights: Vec<f64>,
        signal: TimeSignal,
    ) -> Result<Self, WaveError> {
        Self::new(operator, weights, CanonicalRateDrive::direct(signal)?)
    }

    pub fn legacy(
        operator: &CanonicalWaveOperator,
        weights: Vec<f64>,
        acceleration: TimeSignal,
        anchor_time: f64,
    ) -> Result<Self, WaveError> {
        Self::new(
            operator,
            weights,
            CanonicalRateDrive::legacy(acceleration, anchor_time)?,
        )
    }

    /// A source on an authored signal, through `CanonicalRateDrive::authored`.
    pub fn authored(
        operator: &CanonicalWaveOperator,
        weights: Vec<f64>,
        signal: TimeSignal,
        anchor_time: f64,
    ) -> Result<Self, WaveError> {
        Self::new(
            operator,
            weights,
            CanonicalRateDrive::authored(signal, anchor_time)?,
        )
    }

    pub fn weights(&self) -> &[f64] {
        &self.weights
    }

    pub fn support(&self) -> &[bool] {
        &self.support
    }

    pub fn drive(&self) -> CanonicalRateDrive {
        self.drive
    }
}

/// What authored a source channel, so a handoff can tell which channel of
/// the next generation carries on which of this one's. Channels are compiled
/// boundaries first, and a position alone shifted with every driven side
/// added or taken away ahead of the point source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CanonicalSourceKey {
    /// A driven outer side.
    Side(OuterSide),
    /// A face's loads on one side of its curves, gathered by signal: faces
    /// sharing a signal share a channel.
    Face { slot: u8, signal: TimeSignal },
    /// The scene's point source.
    Point,
    /// An authored volume source, by its slot.
    Volume(u32),
    /// A channel pushed without a key, known only by its position.
    Unkeyed(u32),
}

/// CPU reference forcing. Source and prescribed-primary data remain distinct
/// because their units, event ownership, and energy exchange are different.
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalForcing {
    sources: Vec<CanonicalSource>,
    /// What authored each of `sources`, in step with it.
    keys: Vec<CanonicalSourceKey>,
    prescribed: Vec<Option<TimeSignal>>,
}

impl CanonicalForcing {
    pub fn none(operator: &CanonicalWaveOperator) -> Self {
        Self {
            sources: Vec::new(),
            keys: Vec::new(),
            prescribed: vec![None; operator.degrees_of_freedom()],
        }
    }

    pub fn from_prescribed(
        operator: &CanonicalWaveOperator,
        prescribed: Vec<Option<TimeSignal>>,
    ) -> Result<Self, WaveError> {
        if prescribed.len() != operator.degrees_of_freedom()
            || prescribed.iter().flatten().any(|signal| !signal.valid())
        {
            return Err(WaveError::Unsupported(
                "prescribed boundary data does not match the generation",
            ));
        }
        Ok(Self {
            sources: Vec::new(),
            keys: Vec::new(),
            prescribed,
        })
    }

    pub fn sources(&self) -> &[CanonicalSource] {
        &self.sources
    }

    /// What authored each source channel, in the order of [`Self::sources`].
    pub fn keys(&self) -> &[CanonicalSourceKey] {
        &self.keys
    }

    /// Whether anything here puts energy in or takes it out. A composition
    /// that does not is the free bulk, and steppers take their cheap path.
    pub fn drives_any(&self) -> bool {
        !self.sources.is_empty() || self.prescribed.iter().any(Option::is_some)
    }

    pub fn prescribed(&self) -> &[Option<TimeSignal>] {
        &self.prescribed
    }

    /// Adds a channel known only by its position.
    pub fn push_source(&mut self, source: CanonicalSource) -> Result<(), WaveError> {
        let position = u32::try_from(self.sources.len()).map_err(|_| WaveError::InvalidState)?;
        self.push_keyed_source(source, CanonicalSourceKey::Unkeyed(position))
    }

    /// Adds a channel and what authored it.
    pub fn push_keyed_source(
        &mut self,
        source: CanonicalSource,
        key: CanonicalSourceKey,
    ) -> Result<(), WaveError> {
        if source.weights.len() != self.prescribed.len() {
            return Err(WaveError::SizeMismatch {
                expected: self.prescribed.len(),
                actual: source.weights.len(),
            });
        }
        self.sources.push(source);
        self.keys.push(key);
        Ok(())
    }

    /// Compiles old normalized volume accelerations into immutable reference-
    /// mass integrated channels and analytically migrated rate drives; a
    /// pulsed channel is imposed as authored.
    pub fn extend_legacy_volume(
        &mut self,
        operator: &CanonicalWaveOperator,
        volume: &CompiledVolumeSources,
        anchor_time: f64,
    ) -> Result<(), WaveError> {
        if volume.nodes.len() != operator.degrees_of_freedom() {
            return Err(WaveError::SizeMismatch {
                expected: operator.degrees_of_freedom(),
                actual: volume.nodes.len(),
            });
        }
        for (channel, &signal) in volume.signals.iter().enumerate() {
            let mut weights = vec![0.0; operator.degrees_of_freedom()];
            for (node, entry) in volume.nodes.iter().enumerate() {
                for contribution in &entry.contributions {
                    if contribution.channel as usize == channel {
                        weights[node] += operator.primary_mass[node] * contribution.weight;
                    }
                }
            }
            self.push_source(CanonicalSource::authored(
                operator,
                weights,
                signal,
                anchor_time,
            )?)?;
        }
        Ok(())
    }

    /// Compiles every authored topology volume-source slot, including disabled
    /// slots with zero carrier weights. Keeping the slots stable prevents a
    /// later channel from inheriting another source's runtime phase merely
    /// because an earlier region was toggled off.
    pub fn extend_legacy_volume_slots(
        &mut self,
        operator: &CanonicalWaveOperator,
        volume: &CompiledVolumeSources,
        authored: &[VolumeSource],
        anchor_time: f64,
    ) -> Result<(), WaveError> {
        if volume.nodes.len() != operator.degrees_of_freedom() {
            return Err(WaveError::SizeMismatch {
                expected: operator.degrees_of_freedom(),
                actual: volume.nodes.len(),
            });
        }
        if authored.iter().filter(|source| source.enabled).count() != volume.signals.len() {
            return Err(WaveError::InvalidState);
        }
        let mut enabled_channel = 0_usize;
        for (slot, source) in authored.iter().enumerate() {
            let channel = source.enabled.then_some(enabled_channel);
            enabled_channel += usize::from(source.enabled);
            let mut weights = vec![0.0; operator.degrees_of_freedom()];
            if let Some(channel) = channel {
                for (node, entry) in volume.nodes.iter().enumerate() {
                    for contribution in &entry.contributions {
                        if contribution.channel as usize == channel {
                            weights[node] += operator.primary_mass[node] * contribution.weight;
                        }
                    }
                }
            }
            let slot = u32::try_from(slot).map_err(|_| WaveError::InvalidState)?;
            self.push_keyed_source(
                CanonicalSource::authored(operator, weights, source.signal, anchor_time)?,
                CanonicalSourceKey::Volume(slot),
            )?;
        }
        Ok(())
    }

    /// Compiles the old weak boundary acceleration storage into edge-integrated
    /// source weights, integrated for a harmonic side and imposed as authored
    /// for a pulsed one. Prescribed signals are copied as field-valued
    /// ownership.
    pub fn from_legacy_boundaries(
        operator: &CanonicalWaveOperator,
        quadratic: &QuadraticWaveOperator,
        anchor_time: f64,
    ) -> Result<Self, WaveError> {
        if quadratic.degrees_of_freedom() != operator.degrees_of_freedom() {
            return Err(WaveError::InvalidMesh(
                "canonical and scalar boundary layouts disagree",
            ));
        }
        let mut forcing = Self::from_prescribed(operator, quadratic.dirichlet_signals().to_vec())?;
        for side in OuterSide::ALL {
            let OuterBoundaryCondition::Neumann { signal } = quadratic.outer_boundaries().get(side)
            else {
                continue;
            };
            let weights = quadratic
                .normalized_neumann_weights()
                .iter()
                .zip(operator.primary_mass())
                .map(|(by_side, mass)| by_side[side.index()] * mass)
                .collect();
            forcing.push_keyed_source(
                CanonicalSource::authored(operator, weights, signal, anchor_time)?,
                CanonicalSourceKey::Side(side),
            )?;
        }
        for slot in 0..2_u8 {
            let mut by_signal = Vec::<(TimeSignal, Vec<f64>)>::new();
            for (node, loads) in quadratic.face_neumann_loads().iter().enumerate() {
                let load = loads[usize::from(slot)];
                if load.normalized_weight == 0.0 {
                    continue;
                }
                let channel = if let Some(index) = by_signal
                    .iter()
                    .position(|(signal, _)| *signal == load.signal)
                {
                    index
                } else {
                    by_signal.push((load.signal, vec![0.0; operator.degrees_of_freedom()]));
                    by_signal.len() - 1
                };
                by_signal[channel].1[node] += load.normalized_weight * operator.primary_mass[node];
            }
            for (signal, weights) in by_signal {
                forcing.push_keyed_source(
                    CanonicalSource::authored(operator, weights, signal, anchor_time)?,
                    CanonicalSourceKey::Face { slot, signal },
                )?;
            }
        }
        Ok(forcing)
    }

    /// Peak-one Gaussian point carrier multiplied by immutable generation
    /// reference mass, over `elements`, the elements of the source's region.
    /// It drives only the nodes `point_source_squared_distances` finds it
    /// sees, so nothing behind a wall. Support is every node of those elements
    /// wherever the source sits, so moving it only reweights it.
    pub fn point_source(
        operator: &CanonicalWaveOperator,
        source: PointSource,
        elements: &[bool],
        anchor_time: f64,
    ) -> Result<CanonicalSource, WaveError> {
        if !source.valid() || elements.len() != operator.element_nodes().len() {
            return Err(WaveError::Unsupported(
                "a point source does not match the generation it drives",
            ));
        }
        let mut support = vec![false; operator.degrees_of_freedom()];
        for (nodes, included) in operator.element_nodes().iter().zip(elements) {
            if *included {
                for node in nodes {
                    support[*node as usize] = true;
                }
            }
        }
        let weights = if source.enabled {
            Self::point_profile(operator, elements, source.position, source.width)?
                .iter()
                .zip(operator.primary_mass())
                .map(|(profile, mass)| mass * profile)
                .collect()
        } else {
            vec![0.0; operator.degrees_of_freedom()]
        };
        CanonicalSource::new_with_support(
            operator,
            weights,
            support,
            CanonicalRateDrive::authored(source.signal, anchor_time)?,
        )
    }

    /// A peak-one Gaussian of `width` about `position`, at each node a point
    /// source there drives over `elements`: nothing behind a wall, nothing of
    /// another region and nothing beyond its reach, where it is zero. The
    /// point source's carrier, and a pulse's shape, so neither reaches a node
    /// the other does not.
    pub fn point_profile(
        operator: &CanonicalWaveOperator,
        elements: &[bool],
        position: Point2,
        width: f64,
    ) -> Result<Vec<f64>, WaveError> {
        if elements.len() != operator.element_nodes().len() || !width.is_finite() || width <= 0.0 {
            return Err(WaveError::Unsupported(
                "a point profile does not match the generation it reaches",
            ));
        }
        let variance = width * width;
        Ok(point_source_squared_distances(
            operator.node_points(),
            operator.element_nodes(),
            elements,
            position,
            width,
        )
        .iter()
        .map(|squared| (-0.5 * squared / variance).exp())
        .collect())
    }

    /// Integrated nodal source rate at one physical time. Exposed for
    /// synchronized diagnostics and the static-linear AMR defect; evolution
    /// uses the same evaluation internally.
    pub fn integrated_rate(&self, time: f64) -> Result<Vec<f64>, WaveError> {
        let mut rate = vec![0.0; self.prescribed.len()];
        for source in &self.sources {
            let value = source.drive.value(time)?;
            for (sum, weight) in rate.iter_mut().zip(&source.weights) {
                *sum += weight * value;
            }
        }
        finite_values(&rate)?;
        Ok(rate)
    }
}

/// Energy ledger for one accepted CPU reference operation. Signs are from the
/// field's point of view: sources and prescribed exchange may have either sign;
/// passive losses are reported as nonnegative removed energy.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanonicalStepAccounting {
    pub source_work: f64,
    pub prescribed_exchange: f64,
    pub primary_loss: f64,
    pub complementary_loss: f64,
    pub boundary_loss: f64,
    pub filter_removed: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalThinGapMemory {
    pub key: crate::ThinGapTraceKey,
    pub jump: f64,
}

/// Identifies the geometry and discretization whose constitutive caches an
/// operator owns. Material edits build another operator rather than mutating
/// these immutable samples in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CanonicalGenerationTag {
    pub geometry_revision: u64,
    pub mesh_revision: u64,
    pub constitutive_revision: u64,
}

impl CanonicalGenerationTag {
    pub fn from_mesh(mesh: &TriMesh, constitutive_revision: u64) -> Self {
        Self {
            geometry_revision: mesh.geometry_revision,
            mesh_revision: mesh.mesh_revision,
            constitutive_revision,
        }
    }
}

/// One term in the assembled nodal map `Q_i = sum_c w_c p_c u_i`.
/// Contributions remain separate even when they refer to the same authored
/// material, because their region frames and sampled coefficients may differ.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearPrimaryContribution {
    pub node: u32,
    pub element: u32,
    pub local_node: u8,
    pub geometric_weight: f64,
    pub reference_coefficient: f64,
}

/// One independent two-component complementary flux sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearConstitutiveSample {
    pub element: u32,
    pub barycentric: [f64; 3],
    pub point: Point2,
    pub integration_weight: f64,
    /// The immutable direct constitutive coefficient `C` at this sample:
    /// reciprocal stiffness in Mechanical, permeability in TM, and
    /// permittivity in TE.
    pub complementary_reference: SymmetricTensor2,
    /// `J = R A R^T`, where `A` is the existing scalar-wave stiffness tensor.
    /// Thus `C^T W J C` exactly reproduces `G^T W A G` for `C = R G`.
    pub complementary_inverse: SymmetricTensor2,
    curls: [Point2; LOCAL_NODES],
}

impl LinearConstitutiveSample {
    pub fn curls(&self) -> &[Point2; LOCAL_NODES] {
        &self.curls
    }
}

/// Physical state that cannot be folded into bulk Q or b. The linear variant
/// owns thin-gap jumps and energy-normalized outgoing pole coordinates;
/// oscillator media add separately named variants in their later gate.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum CanonicalAuxiliaryState {
    #[default]
    None,
    Linear(CanonicalLinearAuxiliaries),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CanonicalLinearAuxiliaries {
    thin_gap_jump: Vec<f64>,
    outgoing_z: Vec<f64>,
}

impl CanonicalLinearAuxiliaries {
    pub fn thin_gap_jump(&self) -> &[f64] {
        &self.thin_gap_jump
    }

    pub fn outgoing_z(&self) -> &[f64] {
        &self.outgoing_z
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalOutgoingMode {
    pub eigenvalue: f64,
    pub decay: f64,
    /// Row of `T=V^T D^(1/2) R` in [`CanonicalOutgoingBoundary::trace_nodes`]
    /// order.
    trace: Vec<f64>,
    /// Offset of the three energy-normalized pole states, absent for `d=0`.
    auxiliary_offset: Option<usize>,
}

impl CanonicalOutgoingMode {
    pub fn trace(&self) -> &[f64] {
        &self.trace
    }

    pub fn auxiliary_offset(&self) -> Option<usize> {
        self.auxiliary_offset
    }
}

/// Dense correctness representation of the adopted passive rational trace.
/// Production GPU layout/solve choices remain a later stage; this owns the
/// exact modal transform and energy-normalized state contract used as oracle.
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalOutgoingBoundary {
    trace_nodes: Vec<u32>,
    modes: Vec<CanonicalOutgoingMode>,
    auxiliary_count: usize,
    signature: CanonicalOutgoingSignature,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct CanonicalOutgoingSignature {
    trace_nodes: Vec<u32>,
    damping: Vec<f64>,
    /// Sparse normalized-operator input before division by trace damping.
    entries: Vec<(u32, u32, f64)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalOutgoingPhysicalMemory {
    /// One physical pole-current value per outgoing trace node, in `trace_nodes`
    /// order, for poles 0, 1, 2.
    pub pole_currents: [Vec<f64>; 3],
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanonicalHistoryProjection {
    pub projected_energy: f64,
    pub physical_residual_norm: f64,
}

impl CanonicalOutgoingBoundary {
    pub fn trace_nodes(&self) -> &[u32] {
        &self.trace_nodes
    }

    pub fn modes(&self) -> &[CanonicalOutgoingMode] {
        &self.modes
    }

    pub fn auxiliary_count(&self) -> usize {
        self.auxiliary_count
    }

    pub fn dense_transform_bytes(&self) -> usize {
        self.modes
            .iter()
            .map(|mode| mode.trace.len() * std::mem::size_of::<f64>())
            .sum()
    }

    fn matches_quadratic(&self, quadratic: &QuadraticWaveOperator) -> Result<bool, WaveError> {
        Ok(outgoing_signature(quadratic)?.as_ref() == Some(&self.signature))
    }

    /// Applies the autonomous passive boundary generator to trace `Q` followed
    /// by energy-normalized pole memory. This is the diagnostic/AMR oracle for
    /// checking an accepted endpoint pair; production stepping uses its
    /// prepared midpoint factor.
    pub fn diagnostic_derivative(
        &self,
        operator: &CanonicalWaveOperator,
        trace_primary_flux: &[f64],
        normalized_memory: &[f64],
    ) -> Result<Vec<f64>, WaveError> {
        self.diagnostic_derivative_with(
            operator,
            operator.primary_mass(),
            trace_primary_flux,
            normalized_memory,
        )
    }

    /// The same derivative against a nodal mass supplied by the caller, which
    /// is what a driven generation's diagnostics need: the trace admittance and
    /// the modal couplings both divide by the mass in force at the instant
    /// being measured, not by the authored one.
    pub fn diagnostic_derivative_with(
        &self,
        operator: &CanonicalWaveOperator,
        mass: &[f64],
        trace_primary_flux: &[f64],
        normalized_memory: &[f64],
    ) -> Result<Vec<f64>, WaveError> {
        if trace_primary_flux.len() != self.trace_nodes.len()
            || normalized_memory.len() != self.auxiliary_count
        {
            return Err(WaveError::InvalidState);
        }
        let mut state = Vec::with_capacity(trace_primary_flux.len() + normalized_memory.len());
        state.extend_from_slice(trace_primary_flux);
        state.extend_from_slice(normalized_memory);
        apply_outgoing_generator(operator, self, mass, &state)
    }

    pub fn physical_memory(
        &self,
        energy_normalized: &[f64],
    ) -> Result<CanonicalOutgoingPhysicalMemory, WaveError> {
        if energy_normalized.len() != self.auxiliary_count
            || energy_normalized.iter().any(|value| !value.is_finite())
        {
            return Err(WaveError::InvalidState);
        }
        let (_, inverse) = pole_energy_transform()?;
        let node_count = self.trace_nodes.len();
        let damping = self.trace_damping(node_count);
        let mut pole_currents = std::array::from_fn(|_| vec![0.0; node_count]);
        for mode in &self.modes {
            let Some(offset) = mode.auxiliary_offset else {
                continue;
            };
            let mut raw = [0.0; 3];
            for pole in 0..3 {
                raw[pole] = (0..3)
                    .map(|column| inverse[pole][column] * energy_normalized[offset + column])
                    .sum();
            }
            for node in 0..node_count {
                if damping[node] == 0.0 {
                    continue;
                }
                let eigenvector = mode.trace[node] / damping[node].sqrt();
                for pole in 0..3 {
                    pole_currents[pole][node] += eigenvector * mode.decay.sqrt() * raw[pole];
                }
            }
        }
        Ok(CanonicalOutgoingPhysicalMemory { pole_currents })
    }

    pub fn project_physical_memory(
        &self,
        memory: &CanonicalOutgoingPhysicalMemory,
    ) -> Result<(Vec<f64>, CanonicalHistoryProjection), WaveError> {
        let node_count = self.trace_nodes.len();
        if memory.pole_currents.iter().any(|values| {
            values.len() != node_count || values.iter().any(|value| !value.is_finite())
        }) {
            return Err(WaveError::InvalidState);
        }
        let (transform, _) = pole_energy_transform()?;
        let damping = self.trace_damping(node_count);
        let mut normalized = vec![0.0; self.auxiliary_count];
        for mode in &self.modes {
            let Some(offset) = mode.auxiliary_offset else {
                continue;
            };
            let mut raw = [0.0; 3];
            for (pole, raw_value) in raw.iter_mut().enumerate() {
                *raw_value = (0..node_count)
                    .filter(|node| damping[*node] > 0.0)
                    .map(|node| {
                        mode.trace[node] / damping[node].sqrt() * memory.pole_currents[pole][node]
                    })
                    .sum::<f64>()
                    / mode.decay.sqrt();
            }
            for row in 0..3 {
                normalized[offset + row] =
                    (0..3).map(|pole| transform[row][pole] * raw[pole]).sum();
            }
        }
        let represented = self.physical_memory(&normalized)?;
        let residual_squared = represented
            .pole_currents
            .iter()
            .zip(&memory.pole_currents)
            .flat_map(|(represented, input)| represented.iter().zip(input))
            .map(|(represented, input)| (represented - input).powi(2))
            .sum::<f64>();
        Ok((
            normalized.clone(),
            CanonicalHistoryProjection {
                projected_energy: 0.5 * dot(&normalized, &normalized),
                physical_residual_norm: residual_squared.sqrt(),
            },
        ))
    }

    fn trace_damping(&self, node_count: usize) -> Vec<f64> {
        (0..node_count)
            .map(|node| {
                self.modes
                    .iter()
                    .map(|mode| mode.trace[node] * mode.trace[node])
                    .sum()
            })
            .collect()
    }
}

/// Immutable linear constitutive and incidence data for the direct CPU path.
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalWaveOperator {
    generation: CanonicalGenerationTag,
    physics: PhysicsModel,
    orientation: f64,
    node_points: Vec<Point2>,
    element_nodes: Vec<[u32; LOCAL_NODES]>,
    primary_contributions: Vec<LinearPrimaryContribution>,
    geometric_support: Vec<f64>,
    primary_mass: Vec<f64>,
    primary_loss_rate: Vec<f64>,
    samples: Vec<LinearConstitutiveSample>,
    complementary_loss_rate: Vec<f64>,
    thin_gap_samples: Vec<ThinGapSample>,
    first_order_boundary_damping: Vec<f64>,
    outgoing_boundary: Option<Arc<CanonicalOutgoingBoundary>>,
    component_labels: Vec<u32>,
    component_count: usize,
    maximum_time_step: f64,
    grid_filter_reach: Vec<f64>,
}

impl CanonicalWaveOperator {
    /// Compiles the direct interior and linear boundary composition against an
    /// already assembled quadratic comparison operator.
    pub fn compile_scene(
        mesh: &TriMesh,
        quadratic: &QuadraticWaveOperator,
        scene: &Scene,
        constitutive_revision: u64,
    ) -> Result<Self, WaveError> {
        Self::compile(
            mesh,
            quadratic,
            TopologyWaveModel::from_scene(scene),
            constitutive_revision,
        )
    }

    pub fn compile(
        mesh: &TriMesh,
        quadratic: &QuadraticWaveOperator,
        model: TopologyWaveModel<'_>,
        constitutive_revision: u64,
    ) -> Result<Self, WaveError> {
        let mut job = CanonicalAssemblyJob::new(
            Arc::new(mesh.clone()),
            Arc::new(quadratic.clone()),
            model,
            constitutive_revision,
        )?;
        loop {
            if let Some(result) = job.advance(4096) {
                return result;
            }
        }
    }

    pub fn generation(&self) -> CanonicalGenerationTag {
        self.generation
    }

    pub fn physics(&self) -> PhysicsModel {
        self.physics
    }

    /// `+1` for TM and `-1` for TE/Mechanical.
    pub fn orientation(&self) -> f64 {
        self.orientation
    }

    pub fn node_points(&self) -> &[Point2] {
        &self.node_points
    }

    pub fn element_nodes(&self) -> &[[u32; LOCAL_NODES]] {
        &self.element_nodes
    }

    pub fn primary_contributions(&self) -> &[LinearPrimaryContribution] {
        &self.primary_contributions
    }

    pub fn primary_mass(&self) -> &[f64] {
        &self.primary_mass
    }

    /// Coefficient-independent positive nodal support used by conservative
    /// transfer. It is deliberately not interchangeable with material mass.
    pub fn geometric_support(&self) -> &[f64] {
        &self.geometric_support
    }

    pub fn primary_loss_rate(&self) -> &[f64] {
        &self.primary_loss_rate
    }

    pub fn complementary_loss_rate(&self) -> &[f64] {
        &self.complementary_loss_rate
    }

    pub fn thin_gap_samples(&self) -> &[ThinGapSample] {
        &self.thin_gap_samples
    }

    pub fn first_order_boundary_damping(&self) -> &[f64] {
        &self.first_order_boundary_damping
    }

    pub fn outgoing_boundary(&self) -> Option<&CanonicalOutgoingBoundary> {
        self.outgoing_boundary.as_deref()
    }

    pub fn outgoing_boundary_handle(&self) -> Option<Arc<CanonicalOutgoingBoundary>> {
        self.outgoing_boundary.clone()
    }

    pub fn constitutive_samples(&self) -> &[LinearConstitutiveSample] {
        &self.samples
    }

    pub fn degrees_of_freedom(&self) -> usize {
        self.primary_mass.len()
    }

    pub fn complementary_degrees_of_freedom(&self) -> usize {
        self.samples.len()
    }

    pub fn maximum_time_step(&self) -> f64 {
        self.maximum_time_step
    }

    /// The production integrator's step, [`CanonicalIntegrator::recommended`]
    /// of the leapfrog bound.
    pub fn recommended_time_step(&self) -> f64 {
        CanonicalIntegrator::default().recommended() * self.maximum_time_step
    }

    /// The grid filter's reach `1/Λ̃` at each node, from
    /// [`grid_filter_reach`].
    pub fn grid_filter_reach(&self) -> &[f64] {
        &self.grid_filter_reach
    }

    /// The same operator with its filter reach replaced, so a test can hold
    /// the filter to one bound everywhere.
    #[cfg(test)]
    fn with_grid_filter_reach(mut self, reach: Vec<f64>) -> Self {
        self.grid_filter_reach = reach;
        self
    }

    pub fn component_labels(&self) -> &[u32] {
        &self.component_labels
    }

    pub fn component_count(&self) -> usize {
        self.component_count
    }

    pub fn estimated_operator_bytes(&self) -> usize {
        self.node_points.len() * std::mem::size_of::<Point2>()
            + self.element_nodes.len() * std::mem::size_of::<[u32; LOCAL_NODES]>()
            + self.primary_contributions.len() * std::mem::size_of::<LinearPrimaryContribution>()
            + self.geometric_support.len() * std::mem::size_of::<f64>()
            + self.primary_mass.len() * std::mem::size_of::<f64>()
            + self.primary_loss_rate.len() * std::mem::size_of::<f64>()
            + self.samples.len() * std::mem::size_of::<LinearConstitutiveSample>()
            + self.complementary_loss_rate.len() * std::mem::size_of::<f64>()
            + self.thin_gap_samples.len() * std::mem::size_of::<ThinGapSample>()
            + self.first_order_boundary_damping.len() * std::mem::size_of::<f64>()
            + self
                .outgoing_boundary
                .as_ref()
                .map_or(0, |boundary| boundary.dense_transform_bytes())
            + self.component_labels.len() * std::mem::size_of::<u32>()
            + self.grid_filter_reach.len() * std::mem::size_of::<f64>()
    }

    /// Converts the integrated primary flux to the synchronized scalar field.
    pub fn primary_field(&self, primary_flux: &[f64]) -> Result<Vec<f64>, WaveError> {
        self.validate_primary(primary_flux)?;
        Ok(primary_flux
            .iter()
            .zip(&self.primary_mass)
            .map(|(flux, mass)| flux / mass)
            .collect())
    }

    /// Applies the pointwise linear complementary inverse `v = J b`.
    pub fn complementary_field(
        &self,
        complementary_flux: &[Point2],
    ) -> Result<Vec<Point2>, WaveError> {
        self.validate_complementary(complementary_flux)?;
        Ok(self
            .samples
            .iter()
            .zip(complementary_flux)
            .map(|(sample, flux)| sample.complementary_inverse.apply(*flux))
            .collect())
    }

    /// `F(b) = eta C^T W J b`, gathered in deterministic element/sample/local
    /// order. No CSR projection of `b` is involved.
    pub fn force(&self, complementary_flux: &[Point2]) -> Result<Vec<f64>, WaveError> {
        self.validate_complementary(complementary_flux)?;
        let mut force = vec![0.0; self.degrees_of_freedom()];
        for (sample_index, (sample, flux)) in
            self.samples.iter().zip(complementary_flux).enumerate()
        {
            let field = sample.complementary_inverse.apply(*flux);
            let nodes = self.element_nodes[sample_index / QUADRATURE_SAMPLES];
            for local in 0..LOCAL_NODES {
                force[nodes[local] as usize] +=
                    self.orientation * sample.integration_weight * sample.curls[local].dot(field);
            }
        }
        finite_values(&force)?;
        Ok(force)
    }

    /// Compatible flux `b = eta C psi`. The difference form annihilates a
    /// constant potential exactly in floating point.
    pub fn compatible_flux(&self, potential: &[f64]) -> Result<Vec<Point2>, WaveError> {
        self.validate_primary(potential)?;
        let mut flux = Vec::with_capacity(self.samples.len());
        for (sample_index, sample) in self.samples.iter().enumerate() {
            let nodes = self.element_nodes[sample_index / QUADRATURE_SAMPLES];
            let reference = potential[nodes[0] as usize];
            let mut curl = Point2::default();
            for local in 1..LOCAL_NODES {
                curl = curl + sample.curls[local] * (potential[nodes[local] as usize] - reference);
            }
            flux.push(curl * self.orientation);
        }
        if flux.iter().any(|value| !value.finite()) {
            return Err(WaveError::InvalidState);
        }
        Ok(flux)
    }

    /// `K psi` through the direct vector path. This is the Stage 2 parity
    /// oracle for the existing enriched-quadratic stiffness matrix.
    pub fn compatible_stiffness(&self, potential: &[f64]) -> Result<Vec<f64>, WaveError> {
        self.force(&self.compatible_flux(potential)?)
    }

    /// Returns the component of an independent complementary flux that lies in
    /// `ker(C^T W J)`. It is a reference diagnostic/filter oracle; evolution
    /// never projects this legitimate stationary state away.
    pub fn stationary_complementary_component(
        &self,
        flux: &[Point2],
    ) -> Result<Vec<Point2>, WaveError> {
        let force = self.force(flux)?;
        let compatible = self.compatible_flux(&self.solve_stiffness(&force)?)?;
        let stationary = flux
            .iter()
            .zip(compatible)
            .map(|(all, image)| *all - image)
            .collect::<Vec<_>>();
        let residual = self.force(&stationary)?;
        let scale = force
            .iter()
            .map(|value| value * value)
            .sum::<f64>()
            .sqrt()
            .max(1.0);
        if residual
            .iter()
            .map(|value| value * value)
            .sum::<f64>()
            .sqrt()
            > 2.0e-10 * scale
        {
            return Err(WaveError::InvalidState);
        }
        Ok(stationary)
    }

    fn drift(
        &self,
        complementary_flux: &mut [Point2],
        primary_field: &[f64],
        time_step: f64,
    ) -> Result<(), WaveError> {
        self.validate_complementary(complementary_flux)?;
        self.validate_primary(primary_field)?;
        for (sample_index, (sample, flux)) in
            self.samples.iter().zip(complementary_flux).enumerate()
        {
            let nodes = self.element_nodes[sample_index / QUADRATURE_SAMPLES];
            let reference = primary_field[nodes[0] as usize];
            let mut curl = Point2::default();
            for local in 1..LOCAL_NODES {
                curl = curl
                    + sample.curls[local]
                        * stable_difference(primary_field[nodes[local] as usize], reference);
            }
            *flux = *flux + curl * (self.orientation * time_step);
            if !flux.finite() {
                return Err(WaveError::InvalidState);
            }
        }
        Ok(())
    }

    fn solve_stiffness(&self, right_hand_side: &[f64]) -> Result<Vec<f64>, WaveError> {
        self.validate_primary(right_hand_side)?;
        let mut rhs = right_hand_side.to_vec();
        let scale = rhs.iter().map(|value| value.abs()).sum::<f64>();
        let mut totals = vec![0.0; self.component_count];
        for (node, value) in rhs.iter().enumerate() {
            totals[self.component_labels[node] as usize] += value;
        }
        let compatibility_tolerance = 2.0e-12 * scale.max(1.0);
        if totals
            .iter()
            .any(|total| total.abs() > compatibility_tolerance)
        {
            return Err(WaveError::InvalidState);
        }
        project_component_constants(&mut rhs, &self.component_labels, self.component_count);
        let norm = dot(&rhs, &rhs).sqrt();
        if norm == 0.0 {
            return Ok(vec![0.0; rhs.len()]);
        }

        let mut solution = vec![0.0; rhs.len()];
        let mut residual = rhs.clone();
        let mut direction = residual.clone();
        let mut residual_squared = dot(&residual, &residual);
        let tolerance = 2.0e-12 * norm.max(1.0);
        let maximum_iterations = rhs.len().saturating_mul(40).max(256);
        for _ in 0..maximum_iterations {
            let product = self.compatible_stiffness(&direction)?;
            let denominator = dot(&direction, &product);
            if !denominator.is_finite() || denominator <= 0.0 {
                return Err(WaveError::InvalidState);
            }
            let alpha = residual_squared / denominator;
            for i in 0..solution.len() {
                solution[i] += alpha * direction[i];
                residual[i] -= alpha * product[i];
            }
            project_component_constants(
                &mut residual,
                &self.component_labels,
                self.component_count,
            );
            let next_squared = dot(&residual, &residual);
            if next_squared.sqrt() <= tolerance {
                project_component_constants(
                    &mut solution,
                    &self.component_labels,
                    self.component_count,
                );
                finite_values(&solution)?;
                return Ok(solution);
            }
            let beta = next_squared / residual_squared;
            for i in 0..direction.len() {
                direction[i] = residual[i] + beta * direction[i];
            }
            project_component_constants(
                &mut direction,
                &self.component_labels,
                self.component_count,
            );
            residual_squared = next_squared;
        }
        Err(WaveError::InvalidState)
    }

    fn validate_primary(&self, values: &[f64]) -> Result<(), WaveError> {
        if values.len() != self.degrees_of_freedom() {
            return Err(WaveError::SizeMismatch {
                expected: self.degrees_of_freedom(),
                actual: values.len(),
            });
        }
        finite_values(values)
    }

    fn validate_complementary(&self, values: &[Point2]) -> Result<(), WaveError> {
        if values.len() != self.samples.len() {
            return Err(WaveError::SizeMismatch {
                expected: self.samples.len(),
                actual: values.len(),
            });
        }
        if values.iter().any(|value| !value.finite()) {
            return Err(WaveError::InvalidState);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
struct CachedAuxiliaryElimination {
    mode_index: usize,
    offset: usize,
    inverse: [[f64; 3]; 3],
    aqz_coefficient: [f64; 3],
    solved_column_coefficient: [f64; 3],
}

/// The reduced outgoing midpoint solve, held in a form that carries no nodal
/// mass.
///
/// The matrix this used to factorize is `I + (h/2) K M^-1`, and `K` does not
/// depend on the mass: it is `diag(D + Gamma)` - the first-order impedance plus
/// the second-order trace impedance - plus `sum_k (a_k - 1) t_k t_k^T` over the
/// modes that carry a pole block. Eliminating those blocks never touches the
/// mass either; it only rescales a mode's coefficient by a number built from
/// the mode's decay and the step. So the whole preparation is mass-free and the
/// stage supplies its own mass at the solve, which is what lets a driven
/// generation reuse one preparation instead of refactorizing every stage.
///
/// The solve is a Jacobi iteration preconditioned by the diagonal, rather than
/// a factorization, because the three-pole DtN residues `6/7, -8/7, 2/7` sum to
/// zero: that forces `a_k - 1 = (1/7)(h decay_k)^2` to leading order, so the
/// correction is a small perturbation of a diagonal and the iteration contracts
/// by `max_k |a_k - 1|` per sweep whatever the mass is. It costs
/// `sweeps * O(trace * modes)` and needs no cubic work at any point.
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalOutgoingMidpointFactor {
    duration: f64,
    trace_count: usize,
    /// `D + Gamma` per trace position, mass-free. Divided by the stage's mass
    /// at the solve and never before.
    diagonal: Vec<f64>,
    /// `a_k - 1` per mode, zero on every mode without a pole block.
    modal_correction: Vec<f64>,
    /// `max_k |a_k - 1|`. An upper bound on the iteration's spectral radius for
    /// every positive nodal mass and every prescribed pattern, because the
    /// iteration matrix is similar to `Y^1/2 C Y^1/2` with
    /// `0 < Y < diag(D + Gamma)^-1`, and restricting to the free rows is a
    /// principal submatrix.
    contraction: f64,
    sweeps: usize,
    /// The count an f32 backend runs; see [`Self::device_sweeps`].
    device_sweeps: usize,
    /// A dense inverse of the trace system for one nodal mass, kept only by a
    /// generation whose mass cannot move.
    ///
    /// The sweep is what makes a driven wall possible, but it is not free: it
    /// costs `sweeps` passes over the modes where a triangular solve costs one
    /// pass, and the fixed path has no reason to pay that. So a fixed state
    /// still inverts once at construction, exactly as it always did, and the
    /// sweep serves the path whose mass belongs to the stage. Both solve the
    /// same system - the test holds both against the dense generator - and the
    /// lane is chosen by whether the mass presented matches the one inverted.
    direct: Option<CanonicalOutgoingDirectTrace>,
    eliminated: Vec<CachedAuxiliaryElimination>,
}

#[derive(Clone, Debug, PartialEq)]
struct CanonicalOutgoingDirectTrace {
    trace_mass: Vec<f64>,
    inverse: Vec<f64>,
}

impl CanonicalOutgoingDirectTrace {
    fn apply(&self, reduced: &[f64]) -> Result<Vec<f64>, WaveError> {
        let count = self.trace_mass.len();
        if reduced.len() != count {
            return Err(WaveError::InvalidState);
        }
        let solution = (0..count)
            .map(|row| {
                self.inverse[row * count..(row + 1) * count]
                    .iter()
                    .zip(reduced)
                    .map(|(coefficient, value)| coefficient * value)
                    .sum::<f64>()
            })
            .collect::<Vec<_>>();
        finite_values(&solution)?;
        Ok(solution)
    }
}

/// Sweeps past this are refused rather than run. The bound is a property of the
/// boundary and the step, so the refusal is a computed statement about this
/// generation and not a blanket rule about driven media.
const OUTGOING_TRACE_SWEEP_LIMIT: usize = 32;

/// Sweeps from zero until `contraction^n` falls below `precision`, plus two
/// spare for the bound's own slack, rounded up to an even count.
fn sweeps_to(contraction: f64, precision: f64) -> usize {
    if contraction <= precision {
        return 2;
    }
    let needed = (precision.ln() / contraction.ln()).ceil().max(1.0);
    (needed as usize).saturating_add(2).next_multiple_of(2)
}

/// One diagonal-preconditioned Jacobi solve of `I + (h/2) K M^-1`, shared by the
/// reference factor and the backend-neutral export so their agreement is
/// structural. Prescribed rows are held at their own right-hand side and read
/// by the others, which is what zeroing those rows of the matrix meant.
#[allow(clippy::too_many_arguments)]
fn solve_outgoing_trace(
    boundary: &CanonicalOutgoingBoundary,
    diagonal: &[f64],
    modal_correction: &[f64],
    sweeps: usize,
    half_duration: f64,
    mass: &[f64],
    reduced: &[f64],
    prescribed: &[bool],
) -> Result<Vec<f64>, WaveError> {
    let trace_count = boundary.trace_nodes.len();
    if diagonal.len() != trace_count
        || modal_correction.len() != boundary.modes.len()
        || reduced.len() != trace_count
        || (!prescribed.is_empty() && prescribed.len() != trace_count)
    {
        return Err(WaveError::InvalidState);
    }
    let mut scale = Vec::with_capacity(trace_count);
    let mut inverse_mass = Vec::with_capacity(trace_count);
    for (position, node) in boundary.trace_nodes.iter().copied().enumerate() {
        let value = mass
            .get(node as usize)
            .copied()
            .ok_or(WaveError::InvalidState)?;
        if value <= 0.0 || !value.is_finite() {
            return Err(WaveError::InvalidState);
        }
        inverse_mass.push(1.0 / value);
        scale.push(1.0 / (1.0 + half_duration * diagonal[position] / value));
    }
    let held = |row: usize| prescribed.get(row).copied().unwrap_or(false);
    let mut current = (0..trace_count)
        .map(|row| if held(row) { reduced[row] } else { 0.0 })
        .collect::<Vec<_>>();
    let mut next = vec![0.0; trace_count];
    let mut weighted = vec![0.0; trace_count];
    for _ in 0..sweeps {
        for ((weighted, value), inverse) in weighted.iter_mut().zip(&current).zip(&inverse_mass) {
            *weighted = value * inverse;
        }
        next.copy_from_slice(reduced);
        for (mode, correction) in boundary.modes.iter().zip(modal_correction) {
            if *correction == 0.0 {
                continue;
            }
            let modal = mode
                .trace
                .iter()
                .zip(&weighted)
                .map(|(trace, value)| trace * value)
                .sum::<f64>();
            let coefficient = half_duration * correction * modal;
            for (value, trace) in next.iter_mut().zip(&mode.trace) {
                *value -= coefficient * trace;
            }
        }
        for (row, value) in next.iter_mut().enumerate() {
            if held(row) {
                *value = reduced[row];
            } else {
                *value *= scale[row];
            }
        }
        std::mem::swap(&mut current, &mut next);
    }
    finite_values(&current)?;
    Ok(current)
}

/// Backend-neutral data needed to apply the reduced outgoing midpoint solve.
/// The CPU factor keeps its pivoted LU representation; GPU backends consume
/// this dense inverse so each trace row can be evaluated deterministically in
/// parallel without porting a serial pivoting algorithm into a shader.
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalOutgoingFactorExport {
    pub duration: f64,
    pub trace_count: usize,
    pub energy_transform: [[f64; 3]; 3],
    pub inverse_energy_transform: [[f64; 3]; 3],
    /// `D + Gamma` per trace position, mass-free.
    pub diagonal: Vec<f64>,
    /// `a_k - 1` per mode, zero wherever a mode carries no pole block.
    pub modal_correction: Vec<f64>,
    /// Sweep count for the diagonal-preconditioned solve. Always even, so a
    /// backend ping-ponging two lanes finishes on the first.
    pub sweeps: usize,
    /// The same, converged only to f32 precision, for a backend that holds
    /// f32. Always even and never more than `sweeps`.
    pub device_sweeps: usize,
    /// The trace system's inverse at the authored mass, row-major, when the
    /// factor holds one: a generation whose mass cannot move. A backend can
    /// apply it in one pass where the sweep takes `2 x device_sweeps`. It is
    /// the unconstrained inverse, so it is withheld whenever a trace row is
    /// prescribed and the sweep, which holds such a row at its own value,
    /// serves instead.
    pub direct_inverse: Option<Vec<f64>>,
    pub eliminated: Vec<CanonicalOutgoingEliminationExport>,
    pub prescribed_trace: Vec<bool>,
}

impl CanonicalOutgoingFactorExport {
    fn solve(
        &self,
        mass: &[f64],
        boundary: &CanonicalOutgoingBoundary,
        right: &[f64],
    ) -> Result<Vec<f64>, WaveError> {
        let dimension = self.trace_count
            + self
                .eliminated
                .iter()
                .map(|mode| mode.offset + 3)
                .max()
                .unwrap_or(0);
        if right.len() != dimension
            || boundary.trace_nodes.len() != self.trace_count
            || self.diagonal.len() != self.trace_count
            || self.prescribed_trace.len() != self.trace_count
        {
            return Err(WaveError::InvalidState);
        }
        let mut reduced = right[..self.trace_count].to_vec();
        let mut solved_right = Vec::with_capacity(self.eliminated.len());
        for mode in &self.eliminated {
            let boundary_mode = boundary
                .modes
                .get(mode.mode_index)
                .ok_or(WaveError::InvalidState)?;
            if boundary_mode.auxiliary_offset != Some(mode.offset) {
                return Err(WaveError::InvalidState);
            }
            let input = [
                right[self.trace_count + mode.offset],
                right[self.trace_count + mode.offset + 1],
                right[self.trace_count + mode.offset + 2],
            ];
            let solved: [f64; 3] = std::array::from_fn(|row| {
                (0..3)
                    .map(|column| mode.inverse[row][column] * input[column])
                    .sum()
            });
            let coupling = mode
                .aqz_coefficient
                .iter()
                .zip(solved)
                .map(|(left, right)| left * right)
                .sum::<f64>();
            for (row, (reduced_value, trace)) in
                reduced.iter_mut().zip(&boundary_mode.trace).enumerate()
            {
                if !self.prescribed_trace[row] {
                    *reduced_value -= trace * coupling;
                }
            }
            solved_right.push(solved);
        }
        let trace = solve_outgoing_trace(
            boundary,
            &self.diagonal,
            &self.modal_correction,
            self.sweeps,
            0.5 * self.duration,
            mass,
            &reduced,
            &self.prescribed_trace,
        )?;
        let mut solution = vec![0.0; dimension];
        solution[..self.trace_count].copy_from_slice(&trace);
        for (mode, mut auxiliary) in self.eliminated.iter().zip(solved_right) {
            let boundary_mode = &boundary.modes[mode.mode_index];
            let modal_trace = trace
                .iter()
                .zip(&boundary_mode.trace)
                .zip(&boundary.trace_nodes)
                .map(|((value, coefficient), node)| value * coefficient / mass[*node as usize])
                .sum::<f64>();
            for (value, coefficient) in auxiliary.iter_mut().zip(mode.solved_column_coefficient) {
                *value -= coefficient * modal_trace;
            }
            solution[self.trace_count + mode.offset..self.trace_count + mode.offset + 3]
                .copy_from_slice(&auxiliary);
        }
        finite_values(&solution)?;
        Ok(solution)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalOutgoingEliminationExport {
    pub mode_index: usize,
    pub offset: usize,
    pub inverse: [[f64; 3]; 3],
    pub aqz_coefficient: [f64; 3],
    pub solved_column_coefficient: [f64; 3],
}

/// The time integrator a state steps with.
///
/// Both are the same kick-drift-kick composition with the same Strang loss
/// halves around it; see `docs/spikes/funfern-fourth-order-step.md`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CanonicalIntegrator {
    /// Störmer-Verlet: the kicks use the force at the step's ends and the
    /// drift reads the midpoint field `u`. Second order; kept for tests and
    /// comparison.
    Leapfrog,
    /// The modified-equation Störmer step (Dablain 1986), fourth order on the
    /// linear conservative bulk and stable while `ω dt < 2√3`. Its `dt²/12`
    /// correction sits where the generation's laws leave it symplectic; see
    /// [`FourthOrderForm`]. The kicks integrate a source rate to fourth order,
    /// `s − (s⁺ − 2s + s⁻)/12` at each end. Boundary damping, losses and the
    /// short-wave viscosity keep their second-order places in the step, and a
    /// prescribed node's drift reads its signal expanded as the free nodes'
    /// field is. The production step.
    #[default]
    FourthOrder,
}

/// `√6/2`: past `ω dt = √6` the fourth-order step's modified frequency
/// `ω² (1 − ω² dt²/12)` falls again as `ω` rises, so the shortest waves would
/// run backwards, though the step stays stable to `2√3`.
const FOURTH_ORDER_CEILING: f64 = 1.224_744_871_391_589;

impl CanonicalIntegrator {
    /// The largest step this integrator admits, over the leapfrog bound
    /// `2/ω_max` an operator's `maximum_time_step` reports: the leapfrog's
    /// own stability limit, and for the fourth-order step the last step at
    /// which its dispersion stays monotone.
    pub const fn ceiling(self) -> f64 {
        match self {
            Self::Leapfrog => 1.0,
            Self::FourthOrder => FOURTH_ORDER_CEILING,
        }
    }

    /// The step a caller should take, over the same leapfrog bound: 0.9 for
    /// the leapfrog, and 1.2 for the fourth-order step, below the turn of
    /// its dispersion (`docs/spikes/funfern-fourth-order-step.md`).
    pub const fn recommended(self) -> f64 {
        match self {
            Self::Leapfrog => 0.9,
            Self::FourthOrder => 1.2,
        }
    }
}

/// Where the fourth-order step puts its correction.
///
/// Each form is the exact gradient of a modified store while the other side
/// of the system is quadratic, and so keeps the step symplectic there. A
/// generation with a field law on the stiffness side takes the kick form;
/// every other generation, linear ones included, takes the drift form, the
/// more accurate of the two where both apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FourthOrderForm {
    /// The drift and everything that drifts beside `b` read
    /// `ũ = u − dt²/12 · A (L u − ṡ)`: `A` is `∂u/∂Q`, `L` the stiffness of
    /// everything that drifts and `ṡ` the source rate's derivative. The
    /// gradient of `E_Q − dt²/24 · uᵀ L u`, exact under any mass-side law.
    Drift,
    /// Each kick uses `F̃ = F + dt²/12 · L_b u̇` with `u̇ = M⁻¹ (s − F)`. The
    /// gradient of `U − dt²/24 · Fᵀ M⁻¹ F`, exact under any stiffness-side or
    /// restoring law over a linear mass.
    Kick,
}

/// The fourth-order step's source rate at a kick: the rate at the kick's
/// instant less a twelfth of its second difference over the step, so the
/// two kicks of a step integrate the source as Simpson's rule does.
pub(crate) fn fourth_order_source(
    forcing: &CanonicalForcing,
    time: f64,
    duration: f64,
    rate: &mut [f64],
) -> Result<(), WaveError> {
    if forcing.sources.is_empty() {
        return Ok(());
    }
    let later = forcing.integrated_rate(time + duration)?;
    let earlier = forcing.integrated_rate(time - duration)?;
    for ((value, later), earlier) in rate.iter_mut().zip(later).zip(earlier) {
        *value -= (later - 2.0 * *value + earlier) / 12.0;
    }
    finite_values(rate)
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalWaveState {
    primary_flux: Vec<f64>,
    complementary_flux: Vec<Point2>,
    auxiliaries: CanonicalAuxiliaryState,
    boundary_cache: Option<Arc<CanonicalOutgoingMidpointFactor>>,
    boundary_prescribed_cache: Option<(Vec<bool>, Arc<CanonicalOutgoingFactorExport>)>,
    time_step: f64,
    steps: u64,
    integrator: CanonicalIntegrator,
}

impl CanonicalWaveState {
    pub fn new(
        operator: &CanonicalWaveOperator,
        time_step: f64,
        primary_flux: Vec<f64>,
        complementary_flux: Vec<Point2>,
    ) -> Result<Self, WaveError> {
        validate_time_step(operator, time_step, CanonicalIntegrator::default())?;
        operator.validate_primary(&primary_flux)?;
        operator.validate_complementary(&complementary_flux)?;
        Self::assemble(operator, time_step, primary_flux, complementary_flux, true)
    }

    /// The same state without the inverted trace lane, for a generation whose
    /// backend will sweep the trace rather than apply an inverse.
    ///
    /// That is a driven generation: its mass moves, so an inverse at the
    /// authored mass is never the system its stages solve, and the export it
    /// packs is the mass-free part alone - the diagonal, the modal
    /// corrections, the sweep count and the eliminated modes. A fixed
    /// generation builds with [`Self::new`] or [`Self::zero`] instead, whose
    /// export carries the inverse its backend applies in one pass. Stepping
    /// one of these is correct and takes the sweep.
    pub fn for_backend(
        operator: &CanonicalWaveOperator,
        time_step: f64,
        primary_flux: Vec<f64>,
        complementary_flux: Vec<Point2>,
    ) -> Result<Self, WaveError> {
        validate_time_step(operator, time_step, CanonicalIntegrator::default())?;
        operator.validate_primary(&primary_flux)?;
        operator.validate_complementary(&complementary_flux)?;
        Self::assemble(operator, time_step, primary_flux, complementary_flux, false)
    }

    pub fn zero_for_backend(
        operator: &CanonicalWaveOperator,
        time_step: f64,
    ) -> Result<Self, WaveError> {
        Self::for_backend(
            operator,
            time_step,
            vec![0.0; operator.degrees_of_freedom()],
            vec![Point2::default(); operator.complementary_degrees_of_freedom()],
        )
    }

    fn assemble(
        operator: &CanonicalWaveOperator,
        time_step: f64,
        primary_flux: Vec<f64>,
        complementary_flux: Vec<Point2>,
        invert_trace: bool,
    ) -> Result<Self, WaveError> {
        let outgoing_count = operator
            .outgoing_boundary
            .as_ref()
            .map_or(0, |boundary| boundary.auxiliary_count);
        let auxiliaries = if operator.thin_gap_samples.is_empty() && outgoing_count == 0 {
            CanonicalAuxiliaryState::None
        } else {
            CanonicalAuxiliaryState::Linear(CanonicalLinearAuxiliaries {
                thin_gap_jump: vec![0.0; operator.thin_gap_samples.len()],
                outgoing_z: vec![0.0; outgoing_count],
            })
        };
        let boundary_cache = operator
            .outgoing_boundary()
            .map(|boundary| {
                if invert_trace {
                    CanonicalOutgoingMidpointFactor::prepare_static(
                        operator,
                        boundary,
                        0.5 * time_step,
                        operator.primary_mass(),
                    )
                } else {
                    CanonicalOutgoingMidpointFactor::prepare(operator, boundary, 0.5 * time_step)
                }
                .map(Arc::new)
            })
            .transpose()?;
        Ok(Self {
            primary_flux,
            complementary_flux,
            auxiliaries,
            boundary_cache,
            boundary_prescribed_cache: None,
            time_step,
            steps: 0,
            integrator: CanonicalIntegrator::default(),
        })
    }

    pub fn from_primary_and_potential(
        operator: &CanonicalWaveOperator,
        time_step: f64,
        primary_field: &[f64],
        potential: &[f64],
    ) -> Result<Self, WaveError> {
        operator.validate_primary(primary_field)?;
        operator.validate_primary(potential)?;
        let primary_flux = primary_field
            .iter()
            .zip(operator.primary_mass())
            .map(|(field, mass)| field * mass)
            .collect();
        Self::new(
            operator,
            time_step,
            primary_flux,
            operator.compatible_flux(potential)?,
        )
    }

    /// Initializes the compatible branch for a scalar field and its endpoint
    /// velocity by solving `K psi = -M udot`, then setting `b = eta C psi`.
    /// A nonzero mass-weighted mean velocity on a free component is rejected.
    pub fn from_primary_velocity(
        operator: &CanonicalWaveOperator,
        time_step: f64,
        primary_field: &[f64],
        velocity: &[f64],
    ) -> Result<Self, WaveError> {
        operator.validate_primary(primary_field)?;
        operator.validate_primary(velocity)?;
        let right_hand_side = velocity
            .iter()
            .zip(operator.primary_mass())
            .map(|(velocity, mass)| -mass * velocity)
            .collect::<Vec<_>>();
        let potential = operator.solve_stiffness(&right_hand_side)?;
        Self::from_primary_and_potential(operator, time_step, primary_field, &potential)
    }

    pub fn zero(operator: &CanonicalWaveOperator, time_step: f64) -> Result<Self, WaveError> {
        Self::new(
            operator,
            time_step,
            vec![0.0; operator.degrees_of_freedom()],
            vec![Point2::default(); operator.complementary_degrees_of_freedom()],
        )
    }

    pub fn primary_flux(&self) -> &[f64] {
        &self.primary_flux
    }

    pub fn complementary_flux(&self) -> &[Point2] {
        &self.complementary_flux
    }

    pub fn auxiliaries(&self) -> &CanonicalAuxiliaryState {
        &self.auxiliaries
    }

    pub fn time_step(&self) -> f64 {
        self.time_step
    }

    pub fn integrator(&self) -> CanonicalIntegrator {
        self.integrator
    }

    pub fn with_integrator(mut self, integrator: CanonicalIntegrator) -> Self {
        self.integrator = integrator;
        self
    }

    pub fn steps(&self) -> u64 {
        self.steps
    }

    pub fn time(&self) -> f64 {
        self.steps as f64 * self.time_step
    }

    pub fn estimated_state_bytes(&self) -> usize {
        self.primary_flux.len() * std::mem::size_of::<f64>()
            + self.complementary_flux.len() * std::mem::size_of::<Point2>()
            + match &self.auxiliaries {
                CanonicalAuxiliaryState::None => 0,
                CanonicalAuxiliaryState::Linear(auxiliaries) => {
                    (auxiliaries.thin_gap_jump.len() + auxiliaries.outgoing_z.len())
                        * std::mem::size_of::<f64>()
                }
            }
    }

    pub fn estimated_solver_bytes(&self) -> usize {
        self.estimated_state_bytes()
            + self
                .boundary_cache
                .as_ref()
                .map_or(0, |factor| factor.estimated_bytes())
    }

    /// Prepared reduced trace factor used by both half-kicks. Exposed so the
    /// actual nonlocal solve can be timed independently of interior stepping.
    pub fn outgoing_midpoint_factor(&self) -> Option<&CanonicalOutgoingMidpointFactor> {
        self.boundary_cache.as_deref()
    }

    pub fn primary_field(&self, operator: &CanonicalWaveOperator) -> Result<Vec<f64>, WaveError> {
        operator.primary_field(&self.primary_flux)
    }

    pub fn complementary_field(
        &self,
        operator: &CanonicalWaveOperator,
    ) -> Result<Vec<Point2>, WaveError> {
        operator.complementary_field(&self.complementary_flux)
    }

    pub fn thin_gap_memory(
        &self,
        operator: &CanonicalWaveOperator,
    ) -> Result<Vec<CanonicalThinGapMemory>, WaveError> {
        if operator.thin_gap_samples().is_empty() {
            return Ok(Vec::new());
        }
        let CanonicalAuxiliaryState::Linear(auxiliaries) = &self.auxiliaries else {
            return Err(WaveError::InvalidState);
        };
        if auxiliaries.thin_gap_jump.len() != operator.thin_gap_samples().len() {
            return Err(WaveError::InvalidState);
        }
        Ok(operator
            .thin_gap_samples()
            .iter()
            .zip(&auxiliaries.thin_gap_jump)
            .map(|(sample, jump)| CanonicalThinGapMemory {
                key: sample.key,
                jump: *jump,
            })
            .collect())
    }

    /// Installs already matched thin-gap history. Orientation is explicit so a
    /// reversed physical trace negates the jump rather than silently copying it.
    pub fn set_thin_gap_memory(
        &mut self,
        operator: &CanonicalWaveOperator,
        memory: &[CanonicalThinGapMemory],
        orientation: f64,
    ) -> Result<(), WaveError> {
        if orientation != 1.0 && orientation != -1.0 {
            return Err(WaveError::Unsupported(
                "thin-gap history was matched with an orientation that is neither forward nor reversed",
            ));
        }
        if operator.thin_gap_samples().is_empty() {
            return memory
                .is_empty()
                .then_some(())
                .ok_or(WaveError::InvalidState);
        }
        let CanonicalAuxiliaryState::Linear(auxiliaries) = &mut self.auxiliaries else {
            return Err(WaveError::InvalidState);
        };
        for (sample, target) in operator
            .thin_gap_samples()
            .iter()
            .zip(&mut auxiliaries.thin_gap_jump)
        {
            *target = memory
                .iter()
                .find(|entry| entry.key == sample.key)
                .map_or(0.0, |entry| orientation * entry.jump);
        }
        Ok(())
    }

    pub fn outgoing_physical_memory(
        &self,
        operator: &CanonicalWaveOperator,
    ) -> Result<Option<CanonicalOutgoingPhysicalMemory>, WaveError> {
        let Some(boundary) = operator.outgoing_boundary() else {
            return Ok(None);
        };
        let normalized = match &self.auxiliaries {
            CanonicalAuxiliaryState::None if boundary.auxiliary_count() == 0 => &[][..],
            CanonicalAuxiliaryState::Linear(auxiliaries) => &auxiliaries.outgoing_z,
            _ => return Err(WaveError::InvalidState),
        };
        boundary.physical_memory(normalized).map(Some)
    }

    pub fn set_outgoing_physical_memory(
        &mut self,
        operator: &CanonicalWaveOperator,
        memory: &CanonicalOutgoingPhysicalMemory,
    ) -> Result<CanonicalHistoryProjection, WaveError> {
        let boundary = operator
            .outgoing_boundary()
            .ok_or(WaveError::InvalidState)?;
        let (normalized, projection) = boundary.project_physical_memory(memory)?;
        if normalized.is_empty() {
            return Ok(projection);
        }
        let CanonicalAuxiliaryState::Linear(auxiliaries) = &mut self.auxiliaries else {
            return Err(WaveError::InvalidState);
        };
        auxiliaries.outgoing_z = normalized;
        Ok(projection)
    }

    pub fn energy(&self, operator: &CanonicalWaveOperator) -> Result<f64, WaveError> {
        let primary = self.primary_field(operator)?;
        let complementary = self.complementary_field(operator)?;
        let primary_energy = self
            .primary_flux
            .iter()
            .zip(primary)
            .map(|(flux, field)| 0.5 * flux * field)
            .sum::<f64>();
        let complementary_energy = self
            .complementary_flux
            .iter()
            .zip(complementary)
            .zip(operator.constitutive_samples())
            .map(|((flux, field), sample)| 0.5 * sample.integration_weight * flux.dot(field))
            .sum::<f64>();
        let gap_energy = match &self.auxiliaries {
            CanonicalAuxiliaryState::None => 0.0,
            CanonicalAuxiliaryState::Linear(auxiliaries) => auxiliaries
                .thin_gap_jump
                .iter()
                .zip(operator.thin_gap_samples())
                .map(|(jump, sample)| 0.5 * sample.stiffness * jump * jump)
                .sum(),
        };
        let outgoing_energy = match &self.auxiliaries {
            CanonicalAuxiliaryState::None => 0.0,
            CanonicalAuxiliaryState::Linear(auxiliaries) => {
                0.5 * dot(&auxiliaries.outgoing_z, &auxiliaries.outgoing_z)
            }
        };
        let energy = primary_energy + complementary_energy + gap_energy + outgoing_energy;
        energy
            .is_finite()
            .then_some(energy)
            .ok_or(WaveError::InvalidState)
    }

    /// One endpoint KDK step with the operator's fixed physical loss channels.
    /// Source-free lossless operators retain the Stage 2 recurrence exactly.
    pub fn step(&mut self, operator: &CanonicalWaveOperator) -> Result<(), WaveError> {
        self.step_with_forcing(operator, &CanonicalForcing::none(operator))
            .map(|_| ())
    }

    /// Symmetric fixed-rate loss around source-aware endpoint KDK. A future
    /// state-varying rate may freeze each half-stage rate, but that composition
    /// is deliberately not claimed by this fixed linear reference.
    pub fn step_with_forcing(
        &mut self,
        operator: &CanonicalWaveOperator,
        forcing: &CanonicalForcing,
    ) -> Result<CanonicalStepAccounting, WaveError> {
        validate_time_step(operator, self.time_step, self.integrator)?;
        operator.validate_primary(&self.primary_flux)?;
        operator.validate_complementary(&self.complementary_flux)?;
        if forcing.prescribed.len() != operator.degrees_of_freedom() {
            return Err(WaveError::SizeMismatch {
                expected: operator.degrees_of_freedom(),
                actual: forcing.prescribed.len(),
            });
        }
        let mut accounting = CanonicalStepAccounting::default();
        let start_time = self.time();
        let half_step = 0.5 * self.time_step;

        accounting.prescribed_exchange += self.enforce_prescribed(operator, forcing, start_time)?;
        let (primary_loss, complementary_loss) = self.apply_loss(operator, half_step)?;
        accounting.primary_loss += primary_loss;
        accounting.complementary_loss += complementary_loss;

        let mut first_source = forcing.integrated_rate(start_time)?;
        if self.integrator == CanonicalIntegrator::FourthOrder {
            fourth_order_source(forcing, start_time, self.time_step, &mut first_source)?;
        }
        let mut first_force = operator.force(&self.complementary_flux)?;
        self.add_thin_gap_force(operator, &mut first_force)?;
        let (source_work, boundary_loss, prescribed_exchange) = self.force_coupled_kick(
            operator,
            &first_force,
            &first_source,
            half_step,
            forcing,
            start_time + half_step,
        )?;
        accounting.source_work += source_work;
        accounting.boundary_loss += boundary_loss;
        accounting.prescribed_exchange += prescribed_exchange;
        let mut primary = operator.primary_field(&self.primary_flux)?;
        if self.integrator == CanonicalIntegrator::FourthOrder {
            primary = fourth_order_field(
                operator,
                forcing,
                &primary,
                start_time + half_step,
                self.time_step,
            )?;
        }
        operator.drift(&mut self.complementary_flux, &primary, self.time_step)?;
        self.drift_thin_gaps(operator, &primary, self.time_step)?;
        let mut second_force = operator.force(&self.complementary_flux)?;
        self.add_thin_gap_force(operator, &mut second_force)?;
        let end_time = start_time + self.time_step;
        let mut second_source = forcing.integrated_rate(end_time)?;
        if self.integrator == CanonicalIntegrator::FourthOrder {
            fourth_order_source(forcing, end_time, self.time_step, &mut second_source)?;
        }
        let (source_work, boundary_loss, prescribed_exchange) = self.force_coupled_kick(
            operator,
            &second_force,
            &second_source,
            half_step,
            forcing,
            end_time,
        )?;
        accounting.source_work += source_work;
        accounting.boundary_loss += boundary_loss;
        accounting.prescribed_exchange += prescribed_exchange;
        let (primary_loss, complementary_loss) = self.apply_loss(operator, half_step)?;
        accounting.primary_loss += primary_loss;
        accounting.complementary_loss += complementary_loss;
        accounting.prescribed_exchange += self.enforce_prescribed(operator, forcing, end_time)?;
        self.steps = self.steps.checked_add(1).ok_or(WaveError::InvalidState)?;
        Ok(accounting)
    }

    /// Applies a field-valued pulse as the constitutive delta `Delta Q=M Delta
    /// u`. Prescribed ownership is restored at the same accepted event time.
    pub fn apply_primary_pulse(
        &mut self,
        operator: &CanonicalWaveOperator,
        forcing: &CanonicalForcing,
        field_increment: &[f64],
    ) -> Result<CanonicalStepAccounting, WaveError> {
        operator.validate_primary(field_increment)?;
        let integrated = field_increment
            .iter()
            .zip(operator.primary_mass())
            .map(|(field, mass)| field * mass)
            .collect::<Vec<_>>();
        let mut accounting = CanonicalStepAccounting {
            source_work: self.apply_primary_increment(operator, &integrated)?,
            ..CanonicalStepAccounting::default()
        };
        accounting.prescribed_exchange = self.enforce_prescribed(operator, forcing, self.time())?;
        Ok(accounting)
    }

    /// Paired fixed-linear grid filter from specification section 7.3. Both
    /// updates are formed from the same pre-filter state; stationary flux in
    /// `ker(C^T W J)` is intentionally untouched.
    ///
    /// Each node's reach `s = 1/Λ̃` sits where the global bound's `1/Λ` did:
    /// `Q ← Q − α K S² M⁻¹ K M⁻¹ Q` and `b ← b − α C S M⁻¹ K M⁻¹ S CᵀWJ b`.
    /// A wave is then damped by its share of its own rows' ceiling rather than
    /// the stiffest row's anywhere on the mesh, which on an adapted or
    /// contrasted mesh is what reaches the coarse and the slow parts at all.
    /// Every correction still begins with `K` or `CᵀWJ` and ends in `K` or
    /// `C`, so constants, component totals, compatibility and the stationary
    /// flux come through as before.
    pub fn apply_grid_filter(
        &mut self,
        operator: &CanonicalWaveOperator,
        forcing: &CanonicalForcing,
        strength: f64,
    ) -> Result<CanonicalStepAccounting, WaveError> {
        if !strength.is_finite() || !(0.0..=crate::GRID_SCALE_FILTER_LIMIT).contains(&strength) {
            return Err(WaveError::Unsupported(
                "the grid filter strength is outside the range its bound admits",
            ));
        }
        if strength == 0.0 {
            return Ok(CanonicalStepAccounting::default());
        }
        let before = self.energy(operator)?;
        let reach = operator.grid_filter_reach();

        let old_primary = self.primary_flux.clone();
        let old_complementary = self.complementary_flux.clone();
        let primary_field = operator.primary_field(&old_primary)?;
        let first = operator.compatible_stiffness(&primary_field)?;
        let first_over_mass = first
            .iter()
            .zip(operator.primary_mass())
            .zip(reach)
            .map(|((value, mass), reach)| value * reach * reach / mass)
            .collect::<Vec<_>>();
        let second = operator.compatible_stiffness(&first_over_mass)?;

        let gathered = operator.force(&old_complementary)?;
        let gathered_over_mass = gathered
            .iter()
            .zip(operator.primary_mass())
            .zip(reach)
            .map(|((value, mass), reach)| value * reach / mass)
            .collect::<Vec<_>>();
        let stiffness_gathered = operator.compatible_stiffness(&gathered_over_mass)?;
        let twice_over_mass = stiffness_gathered
            .iter()
            .zip(operator.primary_mass())
            .zip(reach)
            .map(|((value, mass), reach)| value * reach / mass)
            .collect::<Vec<_>>();
        let complementary_correction = operator.compatible_flux(&twice_over_mass)?;
        let scale = strength;

        for node in 0..self.primary_flux.len() {
            if forcing.prescribed[node].is_none() {
                self.primary_flux[node] = old_primary[node] - scale * second[node];
            }
        }
        for sample in 0..self.complementary_flux.len() {
            self.complementary_flux[sample] =
                old_complementary[sample] - complementary_correction[sample] * scale;
        }
        operator.validate_primary(&self.primary_flux)?;
        operator.validate_complementary(&self.complementary_flux)?;
        let prescribed_exchange = self.enforce_prescribed(operator, forcing, self.time())?;
        let after = self.energy(operator)?;
        let tolerance = 2.0e-12 * before.abs().max(after.abs()).max(1.0);
        if after > before + tolerance {
            self.primary_flux = old_primary;
            self.complementary_flux = old_complementary;
            return Err(WaveError::InvalidState);
        }
        Ok(CanonicalStepAccounting {
            prescribed_exchange,
            filter_removed: (before - after).max(0.0),
            ..CanonicalStepAccounting::default()
        })
    }

    /// Corrects only roundoff-sized drift from already accounted component
    /// totals. A larger mismatch is physical or a broken event ledger and is
    /// rejected rather than projected away.
    pub fn maintain_component_totals(
        &mut self,
        operator: &CanonicalWaveOperator,
        forcing: &CanonicalForcing,
        intended_totals: &[f64],
    ) -> Result<f64, WaveError> {
        if intended_totals.len() != operator.component_count()
            || forcing.prescribed.len() != operator.degrees_of_freedom()
            || intended_totals.iter().any(|value| !value.is_finite())
        {
            return Err(WaveError::InvalidState);
        }
        let scale = self
            .primary_flux
            .iter()
            .map(|value| value.abs())
            .sum::<f64>()
            .max(1.0);
        let mut maximum = 0.0_f64;
        for (component, intended) in intended_totals.iter().enumerate() {
            let current = self
                .primary_flux
                .iter()
                .zip(operator.component_labels())
                .filter(|(_, label)| **label as usize == component)
                .map(|(value, _)| value)
                .sum::<f64>();
            let delta = intended - current;
            maximum = maximum.max(delta.abs());
            if delta.abs() > 1.0e-10 * scale {
                return Err(WaveError::InvalidState);
            }
            let eligible_mass = operator
                .primary_mass()
                .iter()
                .zip(operator.component_labels())
                .zip(&forcing.prescribed)
                .filter(|((_, label), prescribed)| {
                    **label as usize == component && prescribed.is_none()
                })
                .map(|((mass, _), _)| mass)
                .sum::<f64>();
            if delta != 0.0 && eligible_mass == 0.0 {
                return Err(WaveError::InvalidState);
            }
            for node in 0..self.primary_flux.len() {
                if operator.component_labels()[node] as usize == component
                    && forcing.prescribed[node].is_none()
                {
                    self.primary_flux[node] +=
                        delta * operator.primary_mass()[node] / eligible_mass;
                }
            }
        }
        operator.validate_primary(&self.primary_flux)?;
        Ok(maximum)
    }

    fn primary_energy(&self, operator: &CanonicalWaveOperator) -> f64 {
        self.primary_flux
            .iter()
            .zip(operator.primary_mass())
            .map(|(flux, mass)| 0.5 * flux * flux / mass)
            .sum()
    }

    fn complementary_energy(&self, operator: &CanonicalWaveOperator) -> f64 {
        self.complementary_flux
            .iter()
            .zip(operator.constitutive_samples())
            .map(|(flux, sample)| {
                0.5 * sample.integration_weight
                    * flux.dot(sample.complementary_inverse.apply(*flux))
            })
            .sum()
    }

    fn apply_primary_increment(
        &mut self,
        operator: &CanonicalWaveOperator,
        increment: &[f64],
    ) -> Result<f64, WaveError> {
        operator.validate_primary(increment)?;
        let before = self.primary_energy(operator);
        for (flux, increment) in self.primary_flux.iter_mut().zip(increment) {
            *flux += increment;
        }
        operator.validate_primary(&self.primary_flux)?;
        let work = self.primary_energy(operator) - before;
        work.is_finite()
            .then_some(work)
            .ok_or(WaveError::InvalidState)
    }

    fn enforce_prescribed(
        &mut self,
        operator: &CanonicalWaveOperator,
        forcing: &CanonicalForcing,
        time: f64,
    ) -> Result<f64, WaveError> {
        if forcing.prescribed.iter().all(Option::is_none) {
            return Ok(0.0);
        }
        let before = self.primary_energy(operator);
        for (node, signal) in forcing.prescribed.iter().enumerate() {
            if let Some(signal) = signal {
                self.primary_flux[node] = operator.primary_mass[node] * signal.value(time);
            }
        }
        operator.validate_primary(&self.primary_flux)?;
        let exchange = self.primary_energy(operator) - before;
        exchange
            .is_finite()
            .then_some(exchange)
            .ok_or(WaveError::InvalidState)
    }

    fn apply_loss(
        &mut self,
        operator: &CanonicalWaveOperator,
        duration: f64,
    ) -> Result<(f64, f64), WaveError> {
        if operator.primary_loss_rate().iter().all(|rate| *rate == 0.0)
            && operator
                .complementary_loss_rate()
                .iter()
                .all(|rate| *rate == 0.0)
        {
            return Ok((0.0, 0.0));
        }
        let primary_before = self.primary_energy(operator);
        let complementary_before = self.complementary_energy(operator);
        for (flux, rate) in self
            .primary_flux
            .iter_mut()
            .zip(operator.primary_loss_rate())
        {
            *flux *= (-duration * rate).exp();
        }
        for (flux, rate) in self
            .complementary_flux
            .iter_mut()
            .zip(operator.complementary_loss_rate())
        {
            *flux = *flux * (-duration * rate).exp();
        }
        operator.validate_primary(&self.primary_flux)?;
        operator.validate_complementary(&self.complementary_flux)?;
        let primary = primary_before - self.primary_energy(operator);
        let complementary = complementary_before - self.complementary_energy(operator);
        if primary < -1.0e-12 || complementary < -1.0e-12 {
            return Err(WaveError::InvalidState);
        }
        Ok((primary.max(0.0), complementary.max(0.0)))
    }

    /// Implicit-midpoint local impedance kick with the held interior/gap force
    /// on the same right-hand side. This is the fixed-CFL correction selected
    /// by the scattering spike; splitting an independent damping map around an
    /// explicit force kick would converge to the wrong admittance.
    fn force_coupled_kick(
        &mut self,
        operator: &CanonicalWaveOperator,
        force: &[f64],
        source: &[f64],
        duration: f64,
        forcing: &CanonicalForcing,
        target_time: f64,
    ) -> Result<(f64, f64, f64), WaveError> {
        operator.validate_primary(force)?;
        operator.validate_primary(source)?;
        if operator.outgoing_boundary().is_none()
            && operator
                .first_order_boundary_damping()
                .iter()
                .all(|damping| *damping == 0.0)
            && forcing.sources.is_empty()
            && forcing.prescribed.iter().all(Option::is_none)
        {
            for (flux, force) in self.primary_flux.iter_mut().zip(force) {
                *flux -= duration * force;
            }
            operator.validate_primary(&self.primary_flux)?;
            return Ok((0.0, 0.0, 0.0));
        }
        let mut source_work = 0.0;
        let mut boundary_loss = 0.0;
        let mut prescribed_exchange = 0.0;
        let outgoing_nodes = operator
            .outgoing_boundary()
            .map(|boundary| {
                boundary
                    .trace_nodes()
                    .iter()
                    .map(|node| *node as usize)
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        for node in 0..self.primary_flux.len() {
            if outgoing_nodes.contains(&node) {
                continue;
            }
            let old = self.primary_flux[node];
            let mass = operator.primary_mass[node];
            let damping = operator.first_order_boundary_damping[node];
            let ratio = 0.5 * duration * damping / mass;
            let rhs = source[node] - force[node];
            let unconstrained = ((1.0 - ratio) * old + duration * rhs) / (1.0 + ratio);
            let new = forcing.prescribed[node]
                .map_or(unconstrained, |signal| mass * signal.value(target_time));
            if !new.is_finite() {
                return Err(WaveError::InvalidState);
            }
            let midpoint_field = 0.5 * (old + new) / mass;
            let node_source_work = duration * midpoint_field * source[node];
            let node_force_work = duration * midpoint_field * force[node];
            let node_boundary_loss = duration * damping * midpoint_field * midpoint_field;
            let energy_change = 0.5 * (new * new - old * old) / mass;
            source_work += node_source_work;
            boundary_loss += node_boundary_loss;
            if forcing.prescribed[node].is_some() {
                prescribed_exchange +=
                    energy_change - node_source_work + node_force_work + node_boundary_loss;
            }
            self.primary_flux[node] = new;
        }
        if let Some(boundary) = operator.outgoing_boundary() {
            let (outgoing_source, outgoing_loss, outgoing_prescribed) = self
                .force_coupled_outgoing_kick(
                    operator,
                    boundary,
                    force,
                    source,
                    duration,
                    forcing,
                    target_time,
                )?;
            source_work += outgoing_source;
            boundary_loss += outgoing_loss;
            prescribed_exchange += outgoing_prescribed;
        }
        if source_work.is_finite()
            && boundary_loss.is_finite()
            && boundary_loss >= 0.0
            && prescribed_exchange.is_finite()
        {
            Ok((source_work, boundary_loss, prescribed_exchange))
        } else {
            Err(WaveError::InvalidState)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn force_coupled_outgoing_kick(
        &mut self,
        operator: &CanonicalWaveOperator,
        boundary: &CanonicalOutgoingBoundary,
        force: &[f64],
        source: &[f64],
        duration: f64,
        forcing: &CanonicalForcing,
        target_time: f64,
    ) -> Result<(f64, f64, f64), WaveError> {
        let mut outgoing_z = match &self.auxiliaries {
            CanonicalAuxiliaryState::None if boundary.auxiliary_count == 0 => Vec::new(),
            CanonicalAuxiliaryState::Linear(auxiliaries)
                if auxiliaries.outgoing_z.len() == boundary.auxiliary_count =>
            {
                auxiliaries.outgoing_z.clone()
            }
            _ => return Err(WaveError::InvalidState),
        };
        let cache = self
            .boundary_cache
            .as_ref()
            .ok_or(WaveError::InvalidState)?
            .clone();
        let mut prescribed_cache = self.boundary_prescribed_cache.take();
        let result = force_coupled_outgoing_kick_with(
            &mut self.primary_flux,
            &mut outgoing_z,
            &mut prescribed_cache,
            &cache,
            operator,
            boundary,
            operator.primary_mass(),
            force,
            source,
            duration,
            forcing,
            target_time,
            TracePinField::Carried,
        );
        self.boundary_prescribed_cache = prescribed_cache;
        if boundary.auxiliary_count > 0 {
            let CanonicalAuxiliaryState::Linear(auxiliaries) = &mut self.auxiliaries else {
                return Err(WaveError::InvalidState);
            };
            auxiliaries.outgoing_z.copy_from_slice(&outgoing_z);
        }
        result
    }

    fn add_thin_gap_force(
        &self,
        operator: &CanonicalWaveOperator,
        force: &mut [f64],
    ) -> Result<(), WaveError> {
        if operator.thin_gap_samples().is_empty() {
            return Ok(());
        }
        let CanonicalAuxiliaryState::Linear(auxiliaries) = &self.auxiliaries else {
            return Err(WaveError::InvalidState);
        };
        if auxiliaries.thin_gap_jump.len() != operator.thin_gap_samples().len() {
            return Err(WaveError::InvalidState);
        }
        for (sample, jump) in operator
            .thin_gap_samples()
            .iter()
            .zip(&auxiliaries.thin_gap_jump)
        {
            let value = sample.stiffness * jump;
            force[sample.left_node as usize] += value;
            force[sample.right_node as usize] -= value;
        }
        finite_values(force)
    }

    fn drift_thin_gaps(
        &mut self,
        operator: &CanonicalWaveOperator,
        primary_field: &[f64],
        duration: f64,
    ) -> Result<(), WaveError> {
        if operator.thin_gap_samples().is_empty() {
            return Ok(());
        }
        let CanonicalAuxiliaryState::Linear(auxiliaries) = &mut self.auxiliaries else {
            return Err(WaveError::InvalidState);
        };
        for (sample, jump) in operator
            .thin_gap_samples()
            .iter()
            .zip(&mut auxiliaries.thin_gap_jump)
        {
            *jump += duration
                * (primary_field[sample.left_node as usize]
                    - primary_field[sample.right_node as usize]);
            if !jump.is_finite() {
                return Err(WaveError::InvalidState);
            }
        }
        Ok(())
    }
}

/// The field the fourth-order drift reads: `ũ = u − dt²/12 · M⁻¹ (L u − ṡ)`
/// at a free node; a prescribed one reads its signal less `h²/24` of its
/// second difference, the offset a free node's `ũ` carries. `L` is the bulk stiffness and the gap springs, the stiffness of
/// everything the drift moves; `ṡ` is the central difference of the source
/// rate over the step, the two instants the kicks integrate it at.
fn fourth_order_field(
    operator: &CanonicalWaveOperator,
    forcing: &CanonicalForcing,
    field: &[f64],
    middle_time: f64,
    duration: f64,
) -> Result<Vec<f64>, WaveError> {
    let mut stiffness = operator.compatible_stiffness(field)?;
    for sample in operator.thin_gap_samples() {
        let (left, right) = (sample.left_node as usize, sample.right_node as usize);
        let value = sample.stiffness * (field[left] - field[right]);
        stiffness[left] += value;
        stiffness[right] -= value;
    }
    if !forcing.sources.is_empty() {
        let later = forcing.integrated_rate(middle_time + 0.5 * duration)?;
        let earlier = forcing.integrated_rate(middle_time - 0.5 * duration)?;
        for ((value, later), earlier) in stiffness.iter_mut().zip(later).zip(earlier) {
            *value -= (later - earlier) / duration;
        }
    }
    let scale = duration * duration / 12.0;
    let corrected = field
        .iter()
        .zip(&stiffness)
        .zip(operator.primary_mass())
        .zip(&forcing.prescribed)
        .map(|(((field, stiffness), mass), signal)| match signal {
            Some(signal) => {
                let value = signal.value(middle_time);
                value
                    - (signal.value(middle_time + duration) - 2.0 * value
                        + signal.value(middle_time - duration))
                        / 24.0
            }
            None => field - scale * stiffness / mass,
        })
        .collect::<Vec<_>>();
    finite_values(&corrected)?;
    Ok(corrected)
}

/// The field a pinned trace node holds through an outgoing kick, where it
/// enters the wall's implicit solve beside the free trace nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TracePinField {
    /// The mean of the flux the node carried in and the flux it is pinned to,
    /// over the one mass: the fixed path, which pins its first kick half a
    /// step in and whose mass does not move.
    Carried,
    /// The signal at the kick's own instant, as a pinned interior node holds
    /// it. A time-driven path pins at its stages, and the flux a node carries
    /// into a kick was pinned against the other stage's mass, so the mean
    /// would mix two masses: an error of order `h` in the wall's input,
    /// measured as the order falling from 2 towards 1 under a pump.
    Staged,
}

/// The force-coupled outgoing kick, over explicit state rather than a state's
/// own fields.
///
/// Both paths run this one body. The fixed path hands it its cached
/// factorization and the operator's nodal mass; a time-driven one hands it a
/// factorization built for the stage's own mass, because the trace admittance
/// and the modal couplings move with it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn force_coupled_outgoing_kick_with(
    primary_flux: &mut [f64],
    outgoing_z: &mut [f64],
    prescribed_cache: &mut Option<(Vec<bool>, Arc<CanonicalOutgoingFactorExport>)>,
    cache: &CanonicalOutgoingMidpointFactor,
    operator: &CanonicalWaveOperator,
    boundary: &CanonicalOutgoingBoundary,
    mass: &[f64],
    force: &[f64],
    source: &[f64],
    duration: f64,
    forcing: &CanonicalForcing,
    target_time: f64,
    pin_field: TracePinField,
) -> Result<(f64, f64, f64), WaveError> {
    let trace_count = boundary.trace_nodes.len();
    let auxiliary_count = boundary.auxiliary_count;
    let dimension = trace_count + auxiliary_count;
    let trace_position = boundary
        .trace_nodes
        .iter()
        .enumerate()
        .map(|(position, node)| (*node as usize, position))
        .collect::<Vec<_>>();
    if outgoing_z.len() != auxiliary_count {
        return Err(WaveError::InvalidState);
    }
    let old_z = outgoing_z.to_vec();
    let mut old = vec![0.0; dimension];
    for &(node, position) in &trace_position {
        old[position] = primary_flux[node];
    }
    old[trace_count..].copy_from_slice(&old_z);
    let has_prescribed_trace = trace_position
        .iter()
        .any(|(node, _)| forcing.prescribed[*node].is_some());
    let prescribed_trace = trace_position
        .iter()
        .map(|(node, _)| forcing.prescribed[*node].is_some())
        .collect::<Vec<_>>();
    // A staged pin enters the solve at its signal, while its energy change
    // is still measured from the flux it stores on entry.
    let staged = pin_field == TracePinField::Staged && has_prescribed_trace;
    let stored_pins = staged.then(|| old[..trace_count].to_vec());
    if staged {
        for &(node, position) in &trace_position {
            if let Some(signal) = forcing.prescribed[node] {
                old[position] = mass[node] * signal.value(target_time);
            }
        }
    }
    let derivative = apply_outgoing_generator(operator, boundary, mass, &old)?;
    let mut right = old
        .iter()
        .zip(derivative)
        .map(|(old, derivative)| old + 0.5 * duration * derivative)
        .collect::<Vec<_>>();
    if right.len() != dimension {
        return Err(WaveError::InvalidState);
    }
    for &(node, position) in &trace_position {
        right[position] += duration * (source[node] - force[node]);
    }
    let new = if !has_prescribed_trace {
        if cache.duration != duration || cache.trace_count != trace_count {
            return Err(WaveError::InvalidState);
        }
        cache.solve(boundary, mass, &right)?
    } else {
        for &(node, position) in &trace_position {
            if let Some(signal) = forcing.prescribed[node] {
                right[position] = mass[node] * signal.value(target_time);
            }
        }
        let rebuild = prescribed_cache
            .as_ref()
            .is_none_or(|(pattern, _)| pattern != &prescribed_trace);
        if rebuild {
            let factor = cache.export_with_prescribed(&prescribed_trace)?;
            *prescribed_cache = Some((prescribed_trace.clone(), Arc::new(factor)));
        }
        prescribed_cache
            .as_ref()
            .ok_or(WaveError::InvalidState)?
            .1
            .solve(mass, boundary, &right)?
    };
    for &(node, position) in &trace_position {
        primary_flux[node] = new[position];
    }
    outgoing_z.copy_from_slice(&new[trace_count..]);

    let mut source_work = 0.0;
    let mut force_work = 0.0;
    let mut first_order_loss = 0.0;
    let mut primary_energy_change = 0.0;
    let mut midpoint_field = vec![0.0; operator.degrees_of_freedom()];
    let mut has_prescribed = false;
    let stored = stored_pins.as_deref().unwrap_or(&old[..trace_count]);
    for &(node, position) in &trace_position {
        let mass = mass[node];
        let midpoint = 0.5 * (old[position] + new[position]) / mass;
        midpoint_field[node] = midpoint;
        source_work += duration * midpoint * source[node];
        force_work += duration * midpoint * force[node];
        first_order_loss +=
            duration * operator.first_order_boundary_damping[node] * midpoint * midpoint;
        primary_energy_change +=
            0.5 * (new[position] * new[position] - stored[position] * stored[position]) / mass;
        has_prescribed |= forcing.prescribed[node].is_some();
    }
    let auxiliary_energy_change =
        0.5 * (dot(&new[trace_count..], &new[trace_count..]) - dot(&old_z, &old_z));
    let midpoint_z = old_z
        .iter()
        .zip(&new[trace_count..])
        .map(|(old, new)| 0.5 * (old + new))
        .collect::<Vec<_>>();
    let outgoing_loss = modal_outgoing_loss(boundary, &midpoint_field, &midpoint_z, duration)?;
    let boundary_loss = first_order_loss + outgoing_loss;
    let balance =
        primary_energy_change + auxiliary_energy_change - source_work + force_work + boundary_loss;
    let scale = primary_energy_change
        .abs()
        .max(auxiliary_energy_change.abs())
        .max(source_work.abs())
        .max(force_work.abs())
        .max(boundary_loss.abs())
        .max(1.0);
    if !has_prescribed && balance.abs() > 2.0e-10 * scale {
        return Err(WaveError::InvalidState);
    }
    Ok((
        source_work,
        boundary_loss.max(0.0),
        if has_prescribed { balance } else { 0.0 },
    ))
}

/// The outgoing wall's dissipation over one kick, `τ Σ (w + memory)²`, from the
/// trace field and the auxiliaries at the kick's midpoint.
fn modal_outgoing_loss(
    boundary: &CanonicalOutgoingBoundary,
    midpoint_field: &[f64],
    midpoint_z: &[f64],
    duration: f64,
) -> Result<f64, WaveError> {
    let (_, inverse_energy_transform) = pole_energy_transform()?;
    let ell_b = (31.0_f64 / 7.0).sqrt();
    let ell = [0.0, 1.0 - ell_b, 2.0 * ell_b - 4.0];
    let mut outgoing_loss = 0.0;
    for mode in &boundary.modes {
        let w = mode
            .trace
            .iter()
            .zip(&boundary.trace_nodes)
            .map(|(trace, node)| trace * midpoint_field[*node as usize])
            .sum::<f64>();
        let memory = if let Some(offset) = mode.auxiliary_offset {
            let mut x = [0.0; 3];
            for pole in 0..3 {
                x[pole] = (0..3)
                    .map(|column| {
                        inverse_energy_transform[pole][column] * midpoint_z[offset + column]
                    })
                    .sum();
            }
            mode.decay.sqrt()
                * ell
                    .iter()
                    .zip(x)
                    .map(|(coefficient, state)| coefficient * state)
                    .sum::<f64>()
        } else {
            0.0
        };
        outgoing_loss += duration * (w + memory).powi(2);
    }
    Ok(outgoing_loss)
}

/// A trace node's constitutive map, as the nonlinear boundary kick reads it.
pub(crate) trait TraceConstitutive {
    /// The discrete gradient `ū = [T(new) − T(old)] / (new − old)` of the
    /// node's stored energy, its limit `U(old)` when the two coincide, and its
    /// slope `dū/d new`, which is positive for every monotone map.
    fn discrete_gradient(&self, node: usize, old: f64, new: f64) -> Result<(f64, f64), WaveError>;
    /// Stored energy `T(Q)` at one node.
    fn energy(&self, node: usize, flux: f64) -> Result<f64, WaveError>;
    /// The flux `P(u)` a node holds at field `u`, which is where a pin puts
    /// it.
    fn flux_of_field(&self, node: usize, field: f64) -> Result<f64, WaveError>;
}

/// Newton iterations the nonlinear boundary kick may take before its failure
/// is reported. Each is one linear trace solve.
const NONLINEAR_TRACE_ITERATIONS: usize = 60;

/// The force-coupled boundary kick for a trace whose primary map is
/// nonlinear: the discrete-gradient counterpart of
/// [`force_coupled_outgoing_kick_with`].
///
/// The linear kick is the implicit midpoint rule of `Ẋ = G(u, z) + [s − F, 0]`
/// for `X = (Q_Γ, z)`, with the trace field at the midpoint,
/// `u_mid = (Q_old + Q_new)/2m`. Here that field becomes the discrete gradient
/// `ū` of each node's stored energy. The auxiliaries keep their own midpoint,
/// because their energy is quadratic. Because `ΔT = ū·ΔQ` exactly and
/// `Δ(½|z|²) = z_mid·Δz`, the step's energy change is
/// `τ (ū, z_mid)·G(ū, z_mid) + τ ū·(s − F)`, term for term the linear
/// balance with `ū` in place of `u_mid`. The wall stays passive, and the
/// balance is exact to the solve's tolerance.
///
/// The system is solved by Newton on `Q_new`. Linearizing
/// `ū ≈ ū_k + g_k (Q − Q_k)` with `g = dū/dQ_new > 0` turns each iteration
/// into the linear kick itself, at the per-node mass `m_k = 1/(2 g_k)` and
/// with the constant `c_k = ū_k − g_k Q_k` moved to the right-hand side. So
/// the existing trace factor, sweeps included, is the whole inner solve. A
/// linear node has `g = 1/2m` and `c = Q_old/2m`, and reproduces the linear
/// kick in one iteration.
///
/// A pinned trace node holds `Q = P(g(t))` at the kick's instant, and enters
/// the solve as [`TracePinField::Staged`] has it on a linear trace: its flux
/// is held on entry as well, so its discrete gradient is the signal itself
/// and its row is the constrained factor's identity row in every iteration.
/// Its energy change is measured from the flux it stored, and what the pins
/// put in is the balance the free rows leave, returned as the third value.
#[allow(clippy::too_many_arguments)]
pub(crate) fn nonlinear_outgoing_kick_with(
    primary_flux: &mut [f64],
    outgoing_z: &mut [f64],
    cache: &CanonicalOutgoingMidpointFactor,
    operator: &CanonicalWaveOperator,
    boundary: &CanonicalOutgoingBoundary,
    mass: &[f64],
    trace_map: &dyn TraceConstitutive,
    force: &[f64],
    source: &[f64],
    duration: f64,
    forcing: &CanonicalForcing,
    target_time: f64,
) -> Result<(f64, f64, f64), WaveError> {
    let trace_count = boundary.trace_nodes.len();
    let auxiliary_count = boundary.auxiliary_count;
    let dimension = trace_count + auxiliary_count;
    if outgoing_z.len() != auxiliary_count
        || cache.duration != duration
        || cache.trace_count != trace_count
    {
        return Err(WaveError::InvalidState);
    }
    let nodes = boundary
        .trace_nodes
        .iter()
        .map(|node| *node as usize)
        .collect::<Vec<_>>();
    let stored_q = nodes
        .iter()
        .map(|node| primary_flux[*node])
        .collect::<Vec<_>>();
    let pinned = nodes
        .iter()
        .map(|node| {
            forcing.prescribed[*node]
                .map(|signal| trace_map.flux_of_field(*node, signal.value(target_time)))
                .transpose()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let old_q = stored_q
        .iter()
        .zip(&pinned)
        .map(|(stored, pin)| pin.unwrap_or(*stored))
        .collect::<Vec<_>>();
    let constrained = if pinned.iter().any(Option::is_some) {
        let pattern = pinned.iter().map(Option::is_some).collect::<Vec<_>>();
        Some(cache.export_with_prescribed(&pattern)?)
    } else {
        None
    };
    let old_z = outgoing_z.to_vec();

    // Everything on the right that does not depend on the trace field: the
    // old state, the held interior force and source, and the auxiliaries'
    // own half of the explicit generator.
    let mut auxiliary_only = vec![0.0; dimension];
    auxiliary_only[trace_count..].copy_from_slice(&old_z);
    let auxiliary_derivative = apply_outgoing_generator(operator, boundary, mass, &auxiliary_only)?;
    let mut fixed_right = vec![0.0; dimension];
    for position in 0..trace_count {
        fixed_right[position] = old_q[position]
            + duration * (source[nodes[position]] - force[nodes[position]])
            + 0.5 * duration * auxiliary_derivative[position];
    }
    for row in trace_count..dimension {
        fixed_right[row] = old_z[row - trace_count] + 0.5 * duration * auxiliary_derivative[row];
    }
    let unit_mass = vec![1.0; operator.degrees_of_freedom()];
    let mut current = old_q.clone();
    let mut solution = Vec::new();
    let mut converged = false;
    let mut previous_step = f64::INFINITY;
    let mut iteration_mass = mass.to_vec();
    for _ in 0..NONLINEAR_TRACE_ITERATIONS {
        let mut offset = vec![0.0; dimension];
        for position in 0..trace_count {
            let (gradient, slope) =
                trace_map.discrete_gradient(nodes[position], old_q[position], current[position])?;
            if !slope.is_finite() || slope <= 0.0 {
                return Err(WaveError::InvalidState);
            }
            offset[position] = gradient - slope * current[position];
            iteration_mass[nodes[position]] = 0.5 / slope;
        }
        let offset_derivative = apply_outgoing_generator(operator, boundary, &unit_mass, &offset)?;
        let mut right = fixed_right
            .iter()
            .zip(&offset_derivative)
            .map(|(fixed, derivative)| fixed + duration * derivative)
            .collect::<Vec<_>>();
        solution = match &constrained {
            Some(factor) => {
                for (position, pin) in pinned.iter().enumerate() {
                    if let Some(flux) = pin {
                        right[position] = *flux;
                    }
                }
                factor.solve(&iteration_mass, boundary, &right)?
            }
            None => cache.solve(boundary, &iteration_mass, &right)?,
        };
        let step = solution[..trace_count]
            .iter()
            .zip(&current)
            .map(|(next, current)| (next - current).abs())
            .fold(0.0_f64, f64::max);
        current.copy_from_slice(&solution[..trace_count]);
        // The slopes are quotients of energy differences, so the iteration
        // settles onto a roundoff floor rather than to zero. It has converged
        // once a step is below `1e-12` of the trace, or has stopped
        // shrinking somewhere below `1e-10` of it.
        let scale = current
            .iter()
            .chain(&old_q)
            .map(|value| value.abs())
            .fold(f64::MIN_POSITIVE, f64::max);
        if step <= 1e-12 * scale || (step <= 1e-10 * scale && step >= 0.5 * previous_step) {
            converged = true;
            break;
        }
        previous_step = step;
    }
    if !converged {
        return Err(WaveError::Unsupported(
            "the nonlinear boundary kick did not converge",
        ));
    }
    for (position, node) in nodes.iter().enumerate() {
        primary_flux[*node] = solution[position];
    }
    outgoing_z.copy_from_slice(&solution[trace_count..]);

    let mut source_work = 0.0;
    let mut force_work = 0.0;
    let mut first_order_loss = 0.0;
    let mut primary_energy_change = 0.0;
    let mut gradient_field = vec![0.0; operator.degrees_of_freedom()];
    for (position, node) in nodes.iter().copied().enumerate() {
        let (gradient, _) =
            trace_map.discrete_gradient(node, old_q[position], solution[position])?;
        gradient_field[node] = gradient;
        source_work += duration * gradient * source[node];
        force_work += duration * gradient * force[node];
        first_order_loss +=
            duration * operator.first_order_boundary_damping[node] * gradient * gradient;
        primary_energy_change += trace_map.energy(node, solution[position])?
            - trace_map.energy(node, stored_q[position])?;
    }
    let new_z = &solution[trace_count..];
    let auxiliary_energy_change = 0.5 * (dot(new_z, new_z) - dot(&old_z, &old_z));
    let midpoint_z = old_z
        .iter()
        .zip(new_z)
        .map(|(old, new)| 0.5 * (old + new))
        .collect::<Vec<_>>();
    let outgoing_loss = modal_outgoing_loss(boundary, &gradient_field, &midpoint_z, duration)?;
    let boundary_loss = first_order_loss + outgoing_loss;
    let balance =
        primary_energy_change + auxiliary_energy_change - source_work + force_work + boundary_loss;
    let magnitude = primary_energy_change
        .abs()
        .max(auxiliary_energy_change.abs())
        .max(source_work.abs())
        .max(force_work.abs())
        .max(boundary_loss.abs())
        .max(1.0);
    if constrained.is_some() {
        return Ok((source_work, boundary_loss.max(0.0), balance));
    }
    if balance.abs() > 2.0e-10 * magnitude {
        return Err(WaveError::InvalidState);
    }
    Ok((source_work, boundary_loss.max(0.0), 0.0))
}

impl CanonicalOutgoingMidpointFactor {
    pub(crate) fn prepare(
        operator: &CanonicalWaveOperator,
        boundary: &CanonicalOutgoingBoundary,
        duration: f64,
    ) -> Result<Self, WaveError> {
        let trace_count = boundary.trace_nodes.len();
        let half_duration = 0.5 * duration;
        // `sum_k t_k t_k^T` is `diag(Gamma)` exactly, because the modes are a
        // complete orthonormal set scaled by `sqrt(Gamma)`. Summing it here
        // rather than reading the authored trace impedance keeps the factor
        // describable from the boundary alone.
        let mut diagonal = boundary
            .trace_nodes
            .iter()
            .map(|node| operator.first_order_boundary_damping[*node as usize])
            .collect::<Vec<_>>();
        for mode in &boundary.modes {
            for (value, trace) in diagonal.iter_mut().zip(&mode.trace) {
                *value += trace * trace;
            }
        }
        if diagonal
            .iter()
            .any(|value| *value <= 0.0 || !value.is_finite())
        {
            return Err(WaveError::InvalidMesh(
                "an outgoing trace node has no positive impedance",
            ));
        }
        let mut modal_correction = vec![0.0; boundary.modes.len()];
        let (energy_transform, inverse_energy_transform) = pole_energy_transform()?;
        let residues = [6.0 / 7.0, -8.0 / 7.0, 2.0 / 7.0];
        let residue_in_z: [f64; 3] = std::array::from_fn(|column| {
            (0..3)
                .map(|pole| residues[pole] * inverse_energy_transform[pole][column])
                .sum()
        });
        let mut eliminated = Vec::new();
        for (mode_index, mode) in boundary.modes.iter().enumerate() {
            // A mode with no pole block keeps its coefficient exactly, so it
            // contributes nothing beyond the `Gamma` already in the diagonal.
            let Some(offset) = mode.auxiliary_offset else {
                continue;
            };
            let root_decay = mode.decay.sqrt();
            let input_gain: [f64; 3] =
                std::array::from_fn(|row| root_decay * energy_transform[row].iter().sum::<f64>());
            let mut block = Vec::with_capacity(9);
            for (row, transform_row) in energy_transform.iter().enumerate() {
                for column in [0, 1, 2] {
                    let generator = (0..3)
                        .map(|pole| {
                            transform_row[pole]
                                * (-mode.decay * pole as f64)
                                * inverse_energy_transform[pole][column]
                        })
                        .sum::<f64>();
                    block.push(f64::from(row == column) - half_duration * generator);
                }
            }
            let mut inverse = [[0.0; 3]; 3];
            for column in 0..3 {
                let mut basis = vec![0.0; 3];
                basis[column] = 1.0;
                let solved = solve_dense(block.clone(), basis, 3)?;
                for row in 0..3 {
                    inverse[row][column] = solved[row];
                }
            }
            // Factored so the step divides out: the eliminated coefficient is
            // `half_duration * unit_aqz . solved_column`, and what the trace
            // system needs is that coefficient relative to `half_duration`.
            // Taking the ratio by construction keeps a zero step from dividing.
            let unit_aqz = residue_in_z.map(|coefficient| root_decay * coefficient);
            let aqz_coefficient = unit_aqz.map(|coefficient| half_duration * coefficient);
            let azq_coefficient = input_gain.map(|coefficient| -half_duration * coefficient);
            let solved_column_coefficient: [f64; 3] = std::array::from_fn(|row| {
                (0..3)
                    .map(|auxiliary| inverse[row][auxiliary] * azq_coefficient[auxiliary])
                    .sum()
            });
            modal_correction[mode_index] = -unit_aqz
                .iter()
                .zip(solved_column_coefficient)
                .map(|(left, right)| left * right)
                .sum::<f64>();
            eliminated.push(CachedAuxiliaryElimination {
                mode_index,
                offset,
                inverse,
                aqz_coefficient,
                solved_column_coefficient,
            });
        }
        let contraction = modal_correction
            .iter()
            .map(|value| value.abs())
            .fold(0.0, f64::max);
        if !contraction.is_finite() || contraction >= 1.0 {
            return Err(WaveError::InvalidMesh(
                "the outgoing trace system is too strongly coupled to solve by sweeps",
            ));
        }
        // From zero the error after `n` sweeps is at most `contraction^n` of the
        // answer. Two spare sweeps cover the bound's own slack, and an even
        // count lets a backend ping-pong two lanes and land on the first.
        let sweeps = sweeps_to(contraction, f64::EPSILON);
        // A device holding f32 cannot use convergence past its own rounding,
        // and every sweep past it costs two dispatches a half-kick for
        // nothing. The host solve keeps the f64 count.
        let device_sweeps = sweeps_to(contraction, f64::from(f32::EPSILON));
        if sweeps > OUTGOING_TRACE_SWEEP_LIMIT {
            return Err(WaveError::InvalidMesh(
                "the outgoing trace system needs too many sweeps to solve",
            ));
        }
        Ok(Self {
            duration,
            trace_count,
            diagonal,
            modal_correction,
            contraction,
            sweeps,
            device_sweeps,
            direct: None,
            eliminated,
        })
    }

    /// The same preparation for a generation whose nodal mass cannot move,
    /// which additionally inverts the trace system once so its solves stay a
    /// single pass. A driven generation has no such mass and uses `prepare`.
    pub(crate) fn prepare_static(
        operator: &CanonicalWaveOperator,
        boundary: &CanonicalOutgoingBoundary,
        duration: f64,
        mass: &[f64],
    ) -> Result<Self, WaveError> {
        let mut factor = Self::prepare(operator, boundary, duration)?;
        let trace_count = factor.trace_count;
        let half_duration = 0.5 * duration;
        let mut trace_mass = Vec::with_capacity(trace_count);
        for node in boundary.trace_nodes.iter().copied() {
            let value = mass
                .get(node as usize)
                .copied()
                .ok_or(WaveError::InvalidState)?;
            if value <= 0.0 || !value.is_finite() {
                return Err(WaveError::InvalidState);
            }
            trace_mass.push(value);
        }
        // The trace system `I + (h/2) K M^-1` is `S M^-1` with
        // `S = M + (h/2) K`, and `S` is symmetric positive definite: `K` is
        // `diag(D + Gamma)` plus modal corrections no larger than the
        // contraction times `Gamma`. So the inverse is `M S^-1`, taken through
        // a Cholesky factor with no pivoting, and only the lower triangle of
        // `S` is ever built.
        let mut system = vec![0.0; trace_count * trace_count];
        for row in 0..trace_count {
            system[row * trace_count + row] =
                trace_mass[row] + half_duration * factor.diagonal[row];
        }
        for (mode, correction) in boundary.modes.iter().zip(&factor.modal_correction) {
            if *correction == 0.0 {
                continue;
            }
            for row in 0..trace_count {
                let scaled = half_duration * correction * mode.trace[row];
                let lower = &mut system[row * trace_count..row * trace_count + row + 1];
                for (entry, trace) in lower.iter_mut().zip(&mode.trace) {
                    *entry += scaled * trace;
                }
            }
        }
        let mut inverse = invert_symmetric_positive(system, trace_count)?;
        for (row, mass) in trace_mass.iter().enumerate() {
            for entry in &mut inverse[row * trace_count..(row + 1) * trace_count] {
                *entry *= mass;
            }
        }
        factor.direct = Some(CanonicalOutgoingDirectTrace {
            trace_mass,
            inverse,
        });
        Ok(factor)
    }

    /// The inverted lane, when this factor holds one and the mass presented is
    /// the one it was inverted for. A generation that moves its mass falls
    /// through to the sweep, which is the whole point of the split.
    fn direct_trace(
        &self,
        boundary: &CanonicalOutgoingBoundary,
        mass: &[f64],
    ) -> Option<&CanonicalOutgoingDirectTrace> {
        let direct = self.direct.as_ref()?;
        boundary
            .trace_nodes
            .iter()
            .zip(&direct.trace_mass)
            .all(|(node, inverted)| mass.get(*node as usize) == Some(inverted))
            .then_some(direct)
    }

    /// The per-sweep contraction bound, valid for every positive nodal mass.
    pub fn contraction(&self) -> f64 {
        self.contraction
    }

    /// How many sweeps the solve runs. Always even.
    pub fn sweeps(&self) -> usize {
        self.sweeps
    }

    /// How many sweeps an f32 backend runs: convergence to its own precision.
    /// Always even and never more than [`Self::sweeps`].
    pub fn device_sweeps(&self) -> usize {
        self.device_sweeps
    }

    pub fn dimension(&self) -> usize {
        self.trace_count
            + self
                .eliminated
                .iter()
                .map(|mode| mode.offset + 3)
                .max()
                .unwrap_or(0)
    }

    pub fn solve(
        &self,
        boundary: &CanonicalOutgoingBoundary,
        mass: &[f64],
        right: &[f64],
    ) -> Result<Vec<f64>, WaveError> {
        let dimension = self.dimension();
        if right.len() != dimension || boundary.trace_nodes.len() != self.trace_count {
            return Err(WaveError::InvalidState);
        }
        let mut reduced = right[..self.trace_count].to_vec();
        let mut solved_right = Vec::with_capacity(self.eliminated.len());
        for mode in &self.eliminated {
            let boundary_mode = boundary
                .modes
                .get(mode.mode_index)
                .ok_or(WaveError::InvalidState)?;
            if boundary_mode.auxiliary_offset != Some(mode.offset) {
                return Err(WaveError::InvalidState);
            }
            let input = [
                right[self.trace_count + mode.offset],
                right[self.trace_count + mode.offset + 1],
                right[self.trace_count + mode.offset + 2],
            ];
            let solved: [f64; 3] = std::array::from_fn(|row| {
                (0..3)
                    .map(|column| mode.inverse[row][column] * input[column])
                    .sum::<f64>()
            });
            let coupling = mode
                .aqz_coefficient
                .iter()
                .zip(solved)
                .map(|(left, right)| left * right)
                .sum::<f64>();
            for (reduced_value, trace) in reduced.iter_mut().zip(&boundary_mode.trace) {
                *reduced_value -= trace * coupling;
            }
            solved_right.push(solved);
        }
        let trace = match self.direct_trace(boundary, mass) {
            Some(direct) => direct.apply(&reduced)?,
            None => solve_outgoing_trace(
                boundary,
                &self.diagonal,
                &self.modal_correction,
                self.sweeps,
                0.5 * self.duration,
                mass,
                &reduced,
                &[],
            )?,
        };
        let mut solution = vec![0.0; dimension];
        solution[..self.trace_count].copy_from_slice(&trace);
        for (mode, mut auxiliary) in self.eliminated.iter().zip(solved_right) {
            let boundary_mode = &boundary.modes[mode.mode_index];
            let modal_trace = trace
                .iter()
                .zip(&boundary_mode.trace)
                .zip(&boundary.trace_nodes)
                .map(|((value, trace), node)| value * trace / mass[*node as usize])
                .sum::<f64>();
            for (value, coefficient) in auxiliary.iter_mut().zip(mode.solved_column_coefficient) {
                *value -= coefficient * modal_trace;
            }
            solution[self.trace_count + mode.offset..self.trace_count + mode.offset + 3]
                .copy_from_slice(&auxiliary);
        }
        finite_values(&solution)?;
        Ok(solution)
    }

    pub fn estimated_bytes(&self) -> usize {
        (self.diagonal.len() + self.modal_correction.len()) * std::mem::size_of::<f64>()
            + self
                .eliminated
                .iter()
                .map(|_| std::mem::size_of::<CachedAuxiliaryElimination>())
                .sum::<usize>()
    }

    /// Exports a backend-neutral parallel-solve representation. Preparation is
    /// event work; evolution never reconstructs it, and because the export is
    /// mass-free a driven generation reuses it across every stage.
    pub fn export(&self) -> Result<CanonicalOutgoingFactorExport, WaveError> {
        let (energy_transform, inverse_energy_transform) = pole_energy_transform()?;
        Ok(CanonicalOutgoingFactorExport {
            duration: self.duration,
            trace_count: self.trace_count,
            energy_transform,
            inverse_energy_transform,
            diagonal: self.diagonal.clone(),
            modal_correction: self.modal_correction.clone(),
            sweeps: self.sweeps,
            device_sweeps: self.device_sweeps,
            direct_inverse: self.direct.as_ref().map(|direct| direct.inverse.clone()),
            eliminated: self
                .eliminated
                .iter()
                .map(|mode| CanonicalOutgoingEliminationExport {
                    mode_index: mode.mode_index,
                    offset: mode.offset,
                    inverse: mode.inverse,
                    aqz_coefficient: mode.aqz_coefficient,
                    solved_column_coefficient: mode.solved_column_coefficient,
                })
                .collect(),
            prescribed_trace: vec![false; self.trace_count],
        })
    }

    /// Exports the same reduced factor with prescribed trace rows replaced by
    /// exact ownership equations. The pattern is generation/runtime metadata;
    /// changing it prepares another immutable factor before acceptance.
    ///
    /// Zeroing a row of the matrix and putting one on its diagonal is what the
    /// sweep does by holding that row at its own right-hand side, so the
    /// pattern is now the whole representation and nothing is refactorized.
    pub fn export_with_prescribed(
        &self,
        prescribed_trace: &[bool],
    ) -> Result<CanonicalOutgoingFactorExport, WaveError> {
        if prescribed_trace.len() != self.trace_count {
            return Err(WaveError::SizeMismatch {
                expected: self.trace_count,
                actual: prescribed_trace.len(),
            });
        }
        let mut export = self.export()?;
        export.prescribed_trace.copy_from_slice(prescribed_trace);
        if prescribed_trace.iter().any(|held| *held) {
            export.direct_inverse = None;
        }
        Ok(export)
    }
}

/// Resumable compiler that owns the mesh, legacy numbering, and material model
/// it reads. One work unit compiles one element; the accepted generation may
/// keep evolving while this candidate is prepared.
pub struct CanonicalAssemblyJob {
    mesh: Arc<TriMesh>,
    quadratic: Arc<QuadraticWaveOperator>,
    model: OwnedTopologyWaveModel,
    generation: CanonicalGenerationTag,
    phase: CanonicalAssemblyPhase,
    primary_contributions: Vec<LinearPrimaryContribution>,
    geometric_support: Vec<f64>,
    primary_mass: Vec<f64>,
    primary_loss_weighted: Vec<f64>,
    samples: Vec<LinearConstitutiveSample>,
    complementary_loss_rate: Vec<f64>,
    interior_columns: Vec<Vec<u32>>,
    outgoing_job: Option<CanonicalOutgoingBoundaryJob>,
    outgoing_boundary: Option<Arc<CanonicalOutgoingBoundary>>,
}

#[derive(Clone, Copy)]
enum CanonicalAssemblyPhase {
    Elements(usize),
    ValidateMass(usize),
    ValidateInterior(usize),
    Outgoing,
    Finish,
    Done,
}

impl CanonicalAssemblyJob {
    pub fn new(
        mesh: Arc<TriMesh>,
        quadratic: Arc<QuadraticWaveOperator>,
        model: TopologyWaveModel<'_>,
        constitutive_revision: u64,
    ) -> Result<Self, WaveError> {
        Self::new_with_outgoing_reuse(mesh, quadratic, model, constitutive_revision, None)
    }

    pub fn new_with_outgoing_reuse(
        mesh: Arc<TriMesh>,
        quadratic: Arc<QuadraticWaveOperator>,
        model: TopologyWaveModel<'_>,
        constitutive_revision: u64,
        previous_outgoing: Option<Arc<CanonicalOutgoingBoundary>>,
    ) -> Result<Self, WaveError> {
        if mesh.geometry_revision != quadratic.geometry_revision()
            || mesh.mesh_revision != quadratic.mesh_revision()
            || mesh.triangles.len() != quadratic.element_nodes().len()
            || quadratic.node_points().len() != quadratic.lumped_mass().len()
        {
            return Err(WaveError::InvalidMesh(
                "the canonical compiler received mismatched mesh and wave generations",
            ));
        }
        let generation = CanonicalGenerationTag::from_mesh(&mesh, constitutive_revision);
        let mut interior_columns = vec![Vec::new(); quadratic.degrees_of_freedom()];
        for sample in quadratic.thin_gap_samples() {
            interior_columns[sample.left_node as usize]
                .extend([sample.left_node, sample.right_node]);
            interior_columns[sample.right_node as usize]
                .extend([sample.left_node, sample.right_node]);
        }
        let outgoing_boundary = match previous_outgoing {
            Some(boundary) if boundary.matches_quadratic(&quadratic)? => Some(boundary),
            _ => None,
        };
        Ok(Self {
            mesh,
            geometric_support: vec![0.0; quadratic.degrees_of_freedom()],
            primary_mass: vec![0.0; quadratic.degrees_of_freedom()],
            primary_loss_weighted: vec![0.0; quadratic.degrees_of_freedom()],
            primary_contributions: Vec::with_capacity(
                quadratic.element_nodes().len() * LOCAL_NODES,
            ),
            samples: Vec::with_capacity(quadratic.element_nodes().len() * QUADRATURE_SAMPLES),
            complementary_loss_rate: Vec::with_capacity(
                quadratic.element_nodes().len() * QUADRATURE_SAMPLES,
            ),
            interior_columns,
            quadratic,
            model: model.to_owned(),
            generation,
            phase: CanonicalAssemblyPhase::Elements(0),
            outgoing_job: None,
            outgoing_boundary,
        })
    }

    pub fn phase(&self) -> &'static str {
        match self.phase {
            CanonicalAssemblyPhase::Elements(_) => "Compiling canonical constitutive samples",
            CanonicalAssemblyPhase::ValidateMass(_)
            | CanonicalAssemblyPhase::ValidateInterior(_) => "Validating canonical operator",
            CanonicalAssemblyPhase::Outgoing => "Compiling outgoing boundary modes",
            CanonicalAssemblyPhase::Finish => "Finishing canonical operator",
            CanonicalAssemblyPhase::Done => "Canonical operator ready",
        }
    }

    /// Runs up to `budget` bounded work units. An element, a validation row, or
    /// a small block of outgoing-boundary rotations is one unit, so even the
    /// dense non-local boundary eigensolve remains cooperative.
    pub fn advance(&mut self, budget: usize) -> Option<Result<CanonicalWaveOperator, WaveError>> {
        for _ in 0..budget {
            match self.phase {
                CanonicalAssemblyPhase::Elements(index) => {
                    if index < self.mesh.triangles.len() {
                        if let Err(error) = self.compile_element(index) {
                            self.phase = CanonicalAssemblyPhase::Done;
                            return Some(Err(error));
                        }
                        self.phase = CanonicalAssemblyPhase::Elements(index + 1);
                    } else if self.samples.len() != self.mesh.triangles.len() * QUADRATURE_SAMPLES {
                        self.phase = CanonicalAssemblyPhase::Done;
                        return Some(Err(WaveError::Unsupported(
                            "the assembly produced the wrong number of quadrature samples",
                        )));
                    } else {
                        self.phase = CanonicalAssemblyPhase::ValidateMass(0);
                    }
                }
                CanonicalAssemblyPhase::ValidateMass(index) => {
                    if index == self.primary_mass.len() {
                        self.phase = CanonicalAssemblyPhase::ValidateInterior(0);
                        continue;
                    }
                    let canonical = self.primary_mass[index];
                    let legacy = self.quadratic.lumped_mass()[index];
                    if !canonical.is_finite() || canonical <= 0.0 {
                        self.phase = CanonicalAssemblyPhase::Done;
                        // Not a material's fault: this is what the assembly
                        // produced from them, so it says so rather than sharing
                        // the message a bad authored coefficient uses.
                        return Some(Err(WaveError::InvalidMesh(
                            "an assembled nodal mass is not finite and positive",
                        )));
                    }
                    let tolerance = 2.0e-12 * canonical.abs().max(legacy.abs()).max(1.0);
                    if (canonical - legacy).abs() > tolerance {
                        self.phase = CanonicalAssemblyPhase::Done;
                        return Some(Err(WaveError::InvalidMesh(
                            "canonical and scalar nodal constitutive maps disagree",
                        )));
                    }
                    self.phase = CanonicalAssemblyPhase::ValidateMass(index + 1);
                }
                CanonicalAssemblyPhase::ValidateInterior(row) => {
                    if row == self.interior_columns.len() {
                        self.phase = CanonicalAssemblyPhase::Outgoing;
                        continue;
                    }
                    let expected = &mut self.interior_columns[row];
                    expected.sort_unstable();
                    expected.dedup();
                    let start = self.quadratic.row_offsets()[row] as usize;
                    let end = self.quadratic.row_offsets()[row + 1] as usize;
                    if expected.len() != end - start
                        || self.quadratic.columns()[start..end]
                            .iter()
                            .zip(expected.iter())
                            .any(|(actual, expected)| actual != expected)
                    {
                        self.phase = CanonicalAssemblyPhase::Done;
                        return Some(Err(WaveError::InvalidMesh(
                            "the scalar operator contains an unsupported stiffness coupling",
                        )));
                    }
                    // Release per-row adjacency storage cooperatively too. Dropping
                    // every row with the finished job made the otherwise small
                    // final work unit grow with the whole adapted mesh.
                    self.interior_columns[row].clear();
                    self.phase = CanonicalAssemblyPhase::ValidateInterior(row + 1);
                }
                CanonicalAssemblyPhase::Outgoing => {
                    if self.outgoing_boundary.is_some() {
                        self.phase = CanonicalAssemblyPhase::Finish;
                        continue;
                    }
                    if self.outgoing_job.is_none() {
                        match CanonicalOutgoingBoundaryJob::new(&self.quadratic) {
                            Ok(Some(job)) => self.outgoing_job = Some(job),
                            Ok(None) => {
                                self.phase = CanonicalAssemblyPhase::Finish;
                                continue;
                            }
                            Err(error) => {
                                self.phase = CanonicalAssemblyPhase::Done;
                                return Some(Err(error));
                            }
                        }
                    }
                    let result = self.outgoing_job.as_mut().unwrap().advance(32);
                    let Some(result) = result else { continue };
                    self.outgoing_job = None;
                    match result {
                        Ok(boundary) => {
                            self.outgoing_boundary = Some(Arc::new(boundary));
                            self.phase = CanonicalAssemblyPhase::Finish;
                        }
                        Err(error) => {
                            self.phase = CanonicalAssemblyPhase::Done;
                            return Some(Err(error));
                        }
                    }
                }
                CanonicalAssemblyPhase::Finish => {
                    self.phase = CanonicalAssemblyPhase::Done;
                    return Some(self.finish());
                }
                CanonicalAssemblyPhase::Done => return None,
            }
        }
        None
    }

    fn compile_element(&mut self, element: usize) -> Result<(), WaveError> {
        let triangle = self.mesh.triangles[element];
        let nodes = self.quadratic.element_nodes()[element];
        let points = triangle
            .vertices
            .map(|vertex| self.mesh.vertices[vertex].point);
        let twice_area = (points[1] - points[0]).cross(points[2] - points[0]);
        if !twice_area.is_finite() || twice_area <= 0.0 {
            return Err(WaveError::InvalidMesh(
                "a canonical element has non-positive area",
            ));
        }
        let area = 0.5 * twice_area;
        let barycentric_gradients = [
            Point2::new(points[1].y - points[2].y, points[2].x - points[1].x) / twice_area,
            Point2::new(points[2].y - points[0].y, points[0].x - points[2].x) / twice_area,
            Point2::new(points[0].y - points[1].y, points[1].x - points[0].x) / twice_area,
        ];
        let model = self.model.as_model();
        for row in nodes {
            self.interior_columns[row as usize].extend(nodes);
        }
        for local in 0..LOCAL_NODES {
            let node = nodes[local] as usize;
            let point = self.quadratic.node_points()[node];
            let (values, primary_loss, _) = linear_material_sample(model, triangle.region, point)?;
            let geometric_weight = area * MASS_WEIGHTS[local];
            self.geometric_support[node] += geometric_weight;
            self.primary_mass[node] += geometric_weight * values.mass_density;
            self.primary_loss_weighted[node] +=
                geometric_weight * values.mass_density * primary_loss;
            self.primary_contributions.push(LinearPrimaryContribution {
                node: nodes[local],
                element: element as u32,
                local_node: local as u8,
                geometric_weight,
                reference_coefficient: values.mass_density,
            });
        }
        for (barycentric, reference_weight) in crate::wave_quadratic::stiffness_quadrature() {
            let point = points[0] * barycentric[0]
                + points[1] * barycentric[1]
                + points[2] * barycentric[2];
            let (values, _, complementary_loss) =
                linear_material_sample(model, triangle.region, point)?;
            let complementary_inverse = rotate_tensor(values);
            self.samples.push(LinearConstitutiveSample {
                element: element as u32,
                barycentric,
                point,
                integration_weight: area * reference_weight,
                complementary_reference: inverse_tensor(complementary_inverse).ok_or(
                    WaveError::Unsupported("an element's complementary tensor cannot be inverted"),
                )?,
                complementary_inverse,
                curls: enriched_quadratic_basis_gradients(barycentric, barycentric_gradients)
                    .map(rotate_vector),
            });
            self.complementary_loss_rate.push(complementary_loss);
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<CanonicalWaveOperator, WaveError> {
        let (component_labels, component_count) = connected_components(
            self.quadratic.degrees_of_freedom(),
            self.quadratic.element_nodes(),
            self.quadratic.thin_gap_samples(),
        );
        let primary_loss_rate = self
            .primary_loss_weighted
            .iter()
            .zip(&self.primary_mass)
            .map(|(weighted, mass)| weighted / mass)
            .collect();
        let grid_filter_reach = grid_filter_reach(
            self.quadratic.element_nodes(),
            &self.samples,
            |_| FilterSampleBound {
                largest: 1.0,
                turning: 0.0,
            },
            &self.primary_mass,
            &[],
        )?;
        let operator = CanonicalWaveOperator {
            generation: self.generation,
            physics: self.model.physics,
            orientation: orientation(self.model.physics),
            node_points: self.quadratic.node_points().to_vec(),
            element_nodes: self.quadratic.element_nodes().to_vec(),
            primary_contributions: std::mem::take(&mut self.primary_contributions),
            geometric_support: std::mem::take(&mut self.geometric_support),
            primary_mass: std::mem::take(&mut self.primary_mass),
            primary_loss_rate,
            samples: std::mem::take(&mut self.samples),
            complementary_loss_rate: std::mem::take(&mut self.complementary_loss_rate),
            thin_gap_samples: self.quadratic.thin_gap_samples().to_vec(),
            first_order_boundary_damping: self
                .quadratic
                .first_order_boundary_damping()
                .iter()
                .zip(self.quadratic.dirichlet_signals())
                .map(|(damping, prescribed)| if prescribed.is_some() { 0.0 } else { *damping })
                .collect(),
            outgoing_boundary: self.outgoing_boundary.take(),
            component_labels,
            component_count,
            maximum_time_step: self.quadratic.maximum_time_step(),
            grid_filter_reach,
        };
        Ok(operator)
    }
}

fn orientation(physics: PhysicsModel) -> f64 {
    match physics {
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        } => 1.0,
        PhysicsModel::Mechanical
        | PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        } => -1.0,
    }
}

/// The strength the solver's own every-sixteen-step filter runs at, against
/// the [`GRID_SCALE_FILTER_LIMIT`] its bound admits. Measured with
/// `grid_filter_calibration` over gallery scenes and in the app with
/// adaptation on (engineering log, 2026-09-28): the element-scale band a sharp
/// event, a remap or a Kerr harmonic leaves e-folds in 0.1-2.4 s, where the
/// global bound it replaced took 9-93 s, and a wave at the twelve nodes per
/// wavelength adaptation asks for keeps 97-98% of its energy over 32 s, 89-93%
/// at eight. Half the strength halves both rates. Twice it clears a released
/// pulse's residue no faster and costs a resolved wave twice as much.
///
/// [`GRID_SCALE_FILTER_LIMIT`]: crate::GRID_SCALE_FILTER_LIMIT
pub const CANONICAL_GRID_FILTER_STRENGTH: f64 = 1.0;

/// The most one constitutive sample's map can reach against its reference
/// tensor `J`. A linear or driven map is `a J` with `a ≤ largest`. A field
/// law's tangent is `J (a P⊥ + b P∥)` about the flux's direction, with both
/// `a, b ≤ largest` and `|b − a| ≤ turning`: it turns a gradient as well as
/// scaling it, by at most `turning`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FilterSampleBound {
    pub largest: f64,
    pub turning: f64,
}

/// The grid filter's reach at each node, `sᵢ = 1/Λ̃ᵢ`.
///
/// `Rᵢ` bounds row `i` of `H = M^{-1/2} K M^{-1/2}`, `Σⱼ |Hᵢⱼ| ≤ Rᵢ`: each
/// sample's `w |gᵢ·J gⱼ|` is added without cancellation, at the most the
/// sample's map reaches and over the least mass each node reaches, so it
/// holds at every instant of a trajectory and for every tangent a field law
/// takes. A restoring law adds its node's curvature to the row. `Λ̃ᵢ` is the
/// largest `R` on the node's elements, which makes `Λ̃ᵢ ≥ Rⱼ` for every `j`
/// that `H` couples to `i`, and by the Schur test both `‖S H‖` and
/// `‖S^{1/2} H S^{1/2}‖` are at most one with `S = diag(s)`. Each half of
/// the filter is then a contraction for any strength up to two, wherever
/// the mesh is fine or coarse and the medium fast or slow.
pub(crate) fn grid_filter_reach(
    element_nodes: &[[u32; LOCAL_NODES]],
    samples: &[LinearConstitutiveSample],
    sample_bound: impl Fn(usize) -> FilterSampleBound,
    minimum_mass: &[f64],
    curvature: &[f64],
) -> Result<Vec<f64>, WaveError> {
    if samples.len() != element_nodes.len() * QUADRATURE_SAMPLES
        || (!curvature.is_empty() && curvature.len() != minimum_mass.len())
    {
        return Err(WaveError::InvalidState);
    }
    let mut rows = curvature.to_vec();
    rows.resize(minimum_mass.len(), 0.0);
    for (index, sample) in samples.iter().enumerate() {
        let nodes = element_nodes[index / QUADRATURE_SAMPLES].map(|node| node as usize);
        // The curls act on differences from local node 0, which therefore
        // takes minus their sum.
        let mut gradients = *sample.curls();
        gradients[0] = gradients[1..]
            .iter()
            .fold(Point2::default(), |sum, curl| sum - *curl);
        let root_mass = nodes.map(|node| minimum_mass[node].sqrt());
        let bound = sample_bound(index);
        let tensor = sample.complementary_inverse;
        // `|gᵢ·J gⱼ|`, and its Cauchy-Schwarz ceiling `‖J‖ |gᵢ| |gⱼ|`, which
        // is what a map that turns can reach.
        let exact = |row: usize| {
            (0..LOCAL_NODES)
                .map(|column| {
                    gradients[row].dot(tensor.apply(gradients[column])).abs()
                        / (root_mass[row] * root_mass[column])
                })
                .sum::<f64>()
        };
        let norm = tensor.eigenvalues()[1];
        let reach: [f64; LOCAL_NODES] =
            std::array::from_fn(|local| gradients[local].norm() / root_mass[local]);
        let total = reach.iter().sum::<f64>();
        let isotropic = tensor.xy == 0.0 && tensor.xx == tensor.yy;
        for row in 0..LOCAL_NODES {
            let ceiling = norm * reach[row] * total;
            let value = if bound.turning == 0.0 {
                bound.largest * exact(row)
            } else if isotropic {
                (bound.largest * exact(row) + bound.turning * ceiling).min(bound.largest * ceiling)
            } else {
                bound.largest * ceiling
            };
            rows[nodes[row]] += sample.integration_weight * value;
        }
    }
    let element_largest = element_nodes
        .iter()
        .map(|nodes| {
            nodes
                .iter()
                .map(|node| rows[*node as usize])
                .fold(0.0_f64, f64::max)
        })
        .collect::<Vec<_>>();
    let mut largest = vec![0.0_f64; rows.len()];
    for (nodes, value) in element_nodes.iter().zip(element_largest) {
        for node in nodes {
            largest[*node as usize] = largest[*node as usize].max(value);
        }
    }
    let reach = largest.into_iter().map(f64::recip).collect::<Vec<_>>();
    if reach
        .iter()
        .any(|value| !value.is_finite() || *value <= 0.0)
    {
        return Err(WaveError::InvalidMesh(
            "a node carries no finite positive grid-filter bound",
        ));
    }
    Ok(reach)
}

/// Evaluates the fixed linear constitutive rows while admitting the Stage 3
/// constant physical loss channels. The legacy scalar evaluator intentionally
/// continues to reject any authored law, so the production solver cannot
/// silently ignore these channels.
fn linear_material_sample(
    model: TopologyWaveModel<'_>,
    region_id: crate::RegionId,
    point: Point2,
) -> Result<(DirectionalWaveCoefficients, f64, f64), WaveError> {
    let region = model.region(region_id).ok_or(WaveError::Unsupported(
        "an element names a region the scene does not hold",
    ))?;
    let material = model
        .material(region.material)
        .ok_or(WaveError::Unsupported(
            "a region names a material the scene does not hold",
        ))?;
    // Naming the slot matters more than it looks. This shares its error with a
    // coefficient that is genuinely not positive and finite, and the two read
    // identically in the application - "wave coefficients must be finite with
    // positive mass and stiffness" for a material that carries a perfectly good
    // pump. A report of that is unactionable; this says which material and
    // which row.
    let carried = if !material.mass_law.is_linear() {
        Some("mass law")
    } else if !material.stiffness_law.is_linear() {
        Some("stiffness law")
    } else if !material.restoring.is_none() {
        Some("restoring law")
    } else {
        None
    };
    if let Some(coefficient) = carried {
        return Err(WaveError::MaterialEvaluation {
            material: material.name.clone(),
            coefficient,
            point,
            reason: "the fixed assembly cannot execute an authored law; a driven \
                     generation assembles from the stripped model instead"
                .into(),
        });
    }
    let coordinates = region.frame.coordinates(point);
    let evaluate = |field: &crate::ScalarField,
                    coefficient: &'static str,
                    positive: bool|
     -> Result<f64, WaveError> {
        let value = field
            .evaluate(coordinates, &material.parameters)
            .map_err(|error| WaveError::MaterialEvaluation {
                material: material.name.clone(),
                coefficient,
                point,
                reason: error.to_string(),
            })?;
        if !value.is_finite() || (positive && value <= 0.0) || (!positive && value < 0.0) {
            return Err(WaveError::MaterialEvaluation {
                material: material.name.clone(),
                coefficient,
                point,
                reason: if positive {
                    "value must be positive".into()
                } else {
                    "value must be nonnegative".into()
                },
            });
        }
        Ok(value)
    };
    let properties = crate::EvaluatedMaterial {
        mass_density: evaluate(&material.mass_density, "density", true)?,
        stiffness: evaluate(&material.stiffness, "stiffness", true)?,
        damping: evaluate(&material.damping, "legacy damping", false)?,
        axis_ratio: evaluate(&material.axis_ratio, "axis ratio", true)?,
    };
    if properties.axis_ratio < 1.0 {
        return Err(WaveError::MaterialEvaluation {
            material: material.name.clone(),
            coefficient: "axis ratio",
            point,
            reason: "value must be at least one; the ratio names the stiffer axis".into(),
        });
    }
    let has_physical_loss = material.electric_loss.is_some() || material.magnetic_loss.is_some();
    if has_physical_loss && properties.damping != 0.0 {
        return Err(WaveError::MaterialEvaluation {
            material: material.name.clone(),
            coefficient: "loss",
            point,
            reason: "legacy damping and named physical loss cannot both be active".into(),
        });
    }
    let electric = linear_loss_rate(
        material,
        material.electric_loss.as_ref(),
        region.frame,
        point,
    )?;
    let magnetic = linear_loss_rate(
        material,
        material.magnetic_loss.as_ref(),
        region.frame,
        point,
    )?;
    let (primary_loss, complementary_loss) = if has_physical_loss {
        match model.physics {
            PhysicsModel::Mechanical => (magnetic, electric),
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            } => (electric, magnetic),
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            } => (magnetic, electric),
        }
    } else {
        // Version-22 damping was a normalized primary-field rate. Migration
        // assigns it to exactly that physical channel, never to both sides.
        (properties.damping, 0.0)
    };
    Ok((
        model
            .physics
            .directional_wave_coefficients(properties, region.frame),
        primary_loss,
        complementary_loss,
    ))
}

fn linear_loss_rate(
    material: &Material,
    channel: Option<&crate::LossChannel>,
    frame: MaterialFrame,
    point: Point2,
) -> Result<f64, WaveError> {
    let Some(channel) = channel else {
        return Ok(0.0);
    };
    if !channel.law.is_constant() {
        return Err(WaveError::MaterialEvaluation {
            material: material.name.clone(),
            coefficient: "loss law",
            point,
            reason: "field- or time-varying loss is gated beyond the linear Stage 3 core".into(),
        });
    }
    let rate = channel
        .base_rate
        .evaluate(frame.coordinates(point), &material.parameters)
        .map_err(|error| WaveError::MaterialEvaluation {
            material: material.name.clone(),
            coefficient: "loss rate",
            point,
            reason: error.to_string(),
        })?;
    if rate.is_finite() && rate >= 0.0 {
        Ok(rate)
    } else {
        Err(WaveError::MaterialEvaluation {
            material: material.name.clone(),
            coefficient: "loss rate",
            point,
            reason: "value must be finite and nonnegative".into(),
        })
    }
}

/// The second-order wall's trace and tangential operator. A node pinned to a
/// zero signal - the corner an electric wall, a magnetic wall or a zero
/// Dirichlet side or span shares with the wall - has no field, so it is not
/// on the trace: the tangential operator ends there as at a Dirichlet end,
/// its modes vanish there, and the entries coupling a trace row to it are
/// dropped, since they multiply zero. Kept, the corner would give the
/// operator a free end whose modes do not vanish where the field does, and a
/// mode that does would read as the wrong tangential wavenumbers: beside
/// pinned sides the wall reflected 2.5 to 10 times what its law says. A node
/// pinned to a driven signal stays on the trace as a held row.
fn outgoing_signature(
    quadratic: &QuadraticWaveOperator,
) -> Result<Option<CanonicalOutgoingSignature>, WaveError> {
    let zero_pinned =
        |node: usize| quadratic.dirichlet_signals()[node].is_some_and(TimeSignal::is_zero);
    let trace_nodes = quadratic
        .second_order_boundary_damping()
        .iter()
        .enumerate()
        .filter_map(|(node, damping)| (*damping > 0.0 && !zero_pinned(node)).then_some(node as u32))
        .collect::<Vec<_>>();
    if trace_nodes.is_empty() {
        if quadratic
            .auxiliary_stiffness_values()
            .iter()
            .any(|value| value.abs() > 1.0e-14)
        {
            return Err(WaveError::InvalidMesh(
                "an outgoing tangential operator has no positive trace impedance",
            ));
        }
        return Ok(None);
    }
    let mut trace_position = vec![usize::MAX; quadratic.degrees_of_freedom()];
    for (position, node) in trace_nodes.iter().enumerate() {
        trace_position[*node as usize] = position;
    }
    let damping = trace_nodes
        .iter()
        .map(|node| quadratic.second_order_boundary_damping()[*node as usize])
        .collect::<Vec<_>>();
    let mut entries = Vec::new();
    for (trace_row, &node) in trace_nodes.iter().enumerate() {
        let row = node as usize;
        for entry in
            quadratic.row_offsets()[row] as usize..quadratic.row_offsets()[row + 1] as usize
        {
            let column = quadratic.columns()[entry] as usize;
            let trace_column = trace_position[column];
            let value = quadratic.auxiliary_stiffness_values()[entry];
            if trace_column == usize::MAX {
                if value.abs() > 2.0e-12 && !zero_pinned(column) {
                    return Err(WaveError::InvalidMesh(
                        "the outgoing tangential operator leaves its physical trace",
                    ));
                }
                continue;
            }
            entries.push((trace_row as u32, trace_column as u32, value));
        }
    }
    Ok(Some(CanonicalOutgoingSignature {
        trace_nodes,
        damping,
        entries,
    }))
}

/// Cooperative compiler for the dense non-local outgoing boundary. The trace
/// eigensolve used to run inside the canonical assembly's final work unit; its
/// cubic cost made that single supposedly time-budgeted unit hundreds of
/// milliseconds on an adapted mesh.
struct CanonicalOutgoingBoundaryJob {
    signature: CanonicalOutgoingSignature,
    trace_nodes: Vec<u32>,
    damping: Vec<f64>,
    eigen: Option<SymmetricEigenJob>,
    eigenvalues: Vec<f64>,
    eigenvectors: Vec<f64>,
    largest: f64,
    next_mode: usize,
    modes: Vec<CanonicalOutgoingMode>,
    auxiliary_count: usize,
}

impl CanonicalOutgoingBoundaryJob {
    fn new(quadratic: &QuadraticWaveOperator) -> Result<Option<Self>, WaveError> {
        let Some(signature) = outgoing_signature(quadratic)? else {
            return Ok(None);
        };
        let trace_nodes = signature.trace_nodes.clone();
        let damping = signature.damping.clone();
        let count = trace_nodes.len();
        let mut normalized = vec![0.0; count * count];
        for &(row, column, value) in &signature.entries {
            let row = row as usize;
            let column = column as usize;
            normalized[row * count + column] = value / (damping[row] * damping[column]).sqrt();
        }
        let matrix_scale = normalized
            .iter()
            .map(|value| value.abs())
            .sum::<f64>()
            .max(1.0);
        for row in 0..count {
            for column in 0..row {
                if (normalized[row * count + column] - normalized[column * count + row]).abs()
                    > 2.0e-11 * matrix_scale
                {
                    return Err(WaveError::InvalidMesh(
                        "the outgoing normalized trace operator is not symmetric",
                    ));
                }
            }
        }
        Ok(Some(Self {
            signature,
            trace_nodes,
            damping,
            eigen: Some(SymmetricEigenJob::new(normalized, count)?),
            eigenvalues: Vec::new(),
            eigenvectors: Vec::new(),
            largest: 1.0,
            next_mode: 0,
            modes: Vec::with_capacity(count),
            auxiliary_count: 0,
        }))
    }

    fn advance(&mut self, budget: usize) -> Option<Result<CanonicalOutgoingBoundary, WaveError>> {
        for _ in 0..budget {
            if let Some(eigen) = &mut self.eigen {
                let Some(result) = eigen.advance(1) else {
                    continue;
                };
                match result {
                    Ok((values, vectors)) => {
                        self.largest = values.last().copied().unwrap_or(0.0).max(1.0);
                        self.eigenvalues = values;
                        self.eigenvectors = vectors;
                        self.eigen = None;
                    }
                    Err(error) => return Some(Err(error)),
                }
                continue;
            }
            if self.next_mode < self.trace_nodes.len() {
                let mode = self.next_mode;
                let eigenvalue = if self.eigenvalues[mode].abs() <= 2.0e-11 * self.largest {
                    0.0
                } else if self.eigenvalues[mode] > 0.0 {
                    self.eigenvalues[mode]
                } else {
                    return Some(Err(WaveError::InvalidMesh(
                        "the outgoing tangential operator is not positive semidefinite",
                    )));
                };
                let decay = (7.0 * eigenvalue / 4.0).sqrt();
                let count = self.trace_nodes.len();
                let trace = self.eigenvectors[mode * count..(mode + 1) * count]
                    .iter()
                    .zip(&self.damping)
                    .map(|(component, damping)| component * damping.sqrt())
                    .collect();
                let auxiliary_offset = if decay > 2.0e-12 * self.largest.sqrt() {
                    let offset = self.auxiliary_count;
                    self.auxiliary_count += 3;
                    Some(offset)
                } else {
                    None
                };
                self.modes.push(CanonicalOutgoingMode {
                    eigenvalue,
                    decay,
                    trace,
                    auxiliary_offset,
                });
                self.next_mode += 1;
                continue;
            }
            return Some(Ok(CanonicalOutgoingBoundary {
                trace_nodes: std::mem::take(&mut self.trace_nodes),
                modes: std::mem::take(&mut self.modes),
                auxiliary_count: self.auxiliary_count,
                signature: std::mem::take(&mut self.signature),
            }));
        }
        None
    }
}

#[cfg(test)]
/// The nodal mass these outgoing maps are built against.
///
/// It is a parameter rather than read from the operator because a time-driven
/// generation's mass moves: the trace admittance, the modal couplings and the
/// Schur complement all scale with it, so a driven path passes the mass in
/// force at the stage while the fixed path passes the operator's own. One
/// implementation serves both, which is what makes their agreement structural
/// instead of something to test for.
fn outgoing_generator(
    operator: &CanonicalWaveOperator,
    boundary: &CanonicalOutgoingBoundary,
    mass: &[f64],
) -> Result<Vec<f64>, WaveError> {
    let trace_count = boundary.trace_nodes.len();
    let dimension = trace_count + boundary.auxiliary_count;
    let mut generator = vec![0.0; dimension * dimension];
    for (position, node) in boundary.trace_nodes.iter().copied().enumerate() {
        let node = node as usize;
        generator[position * dimension + position] -=
            operator.first_order_boundary_damping[node] / mass[node];
    }
    let (energy_transform, inverse_energy_transform) = pole_energy_transform()?;
    let residues = [6.0 / 7.0, -8.0 / 7.0, 2.0 / 7.0];
    for mode in &boundary.modes {
        for row in 0..trace_count {
            let row_trace = mode.trace[row];
            for (column, column_node) in boundary.trace_nodes.iter().copied().enumerate() {
                generator[row * dimension + column] -=
                    row_trace * mode.trace[column] / mass[column_node as usize];
            }
        }
        let Some(offset) = mode.auxiliary_offset else {
            continue;
        };
        let root_decay = mode.decay.sqrt();
        let mut residue_in_z = [0.0; 3];
        for (column, value) in residue_in_z.iter_mut().enumerate() {
            *value = (0..3)
                .map(|pole| residues[pole] * inverse_energy_transform[pole][column])
                .sum();
        }
        for row in 0..trace_count {
            for auxiliary in 0..3 {
                generator[row * dimension + trace_count + offset + auxiliary] -=
                    mode.trace[row] * root_decay * residue_in_z[auxiliary];
            }
        }
        for (auxiliary_row, transform_row) in energy_transform.iter().enumerate() {
            let z_row = trace_count + offset + auxiliary_row;
            let input_gain = root_decay * transform_row.iter().sum::<f64>();
            for (column, node) in boundary.trace_nodes.iter().copied().enumerate() {
                generator[z_row * dimension + column] +=
                    input_gain * mode.trace[column] / mass[node as usize];
            }
            for auxiliary_column in 0..3 {
                generator[z_row * dimension + trace_count + offset + auxiliary_column] += (0..3)
                    .map(|pole| {
                        transform_row[pole]
                            * (-mode.decay * pole as f64)
                            * inverse_energy_transform[pole][auxiliary_column]
                    })
                    .sum::<f64>();
            }
        }
    }
    Ok(generator)
}

/// Applies the passive trace generator without materializing its dense
/// `(trace + poles)^2` matrix. The dense form remains the preparation oracle
/// for the cached Schur factor and the uncommon prescribed-trace path.
fn apply_outgoing_generator(
    operator: &CanonicalWaveOperator,
    boundary: &CanonicalOutgoingBoundary,
    mass: &[f64],
    state: &[f64],
) -> Result<Vec<f64>, WaveError> {
    let trace_count = boundary.trace_nodes.len();
    let dimension = trace_count + boundary.auxiliary_count;
    if state.len() != dimension || state.iter().any(|value| !value.is_finite()) {
        return Err(WaveError::InvalidState);
    }
    let mut derivative = vec![0.0; dimension];
    for (position, node) in boundary.trace_nodes.iter().copied().enumerate() {
        let node = node as usize;
        derivative[position] -=
            operator.first_order_boundary_damping[node] * state[position] / mass[node];
    }
    let (energy_transform, inverse_energy_transform) = pole_energy_transform()?;
    let residues = [6.0 / 7.0, -8.0 / 7.0, 2.0 / 7.0];
    for mode in &boundary.modes {
        let modal_field = mode
            .trace
            .iter()
            .zip(&boundary.trace_nodes)
            .enumerate()
            .map(|(position, (trace, node))| trace * state[position] / mass[*node as usize])
            .sum::<f64>();
        for (row, trace) in mode.trace.iter().copied().enumerate() {
            derivative[row] -= trace * modal_field;
        }
        let Some(offset) = mode.auxiliary_offset else {
            continue;
        };
        let root_decay = mode.decay.sqrt();
        let z = &state[trace_count + offset..trace_count + offset + 3];
        let mut residue_in_z = [0.0; 3];
        for (column, value) in residue_in_z.iter_mut().enumerate() {
            *value = (0..3)
                .map(|pole| residues[pole] * inverse_energy_transform[pole][column])
                .sum();
        }
        let memory = root_decay
            * residue_in_z
                .iter()
                .zip(z)
                .map(|(coefficient, state)| coefficient * state)
                .sum::<f64>();
        for (row, trace) in mode.trace.iter().copied().enumerate() {
            derivative[row] -= trace * memory;
        }
        for (auxiliary_row, transform_row) in energy_transform.iter().enumerate() {
            let z_row = trace_count + offset + auxiliary_row;
            let input_gain = root_decay * transform_row.iter().sum::<f64>();
            derivative[z_row] += input_gain * modal_field;
            derivative[z_row] += (0..3)
                .map(|pole| {
                    let raw_decay = -mode.decay * pole as f64;
                    (0..3)
                        .map(|column| {
                            transform_row[pole]
                                * raw_decay
                                * inverse_energy_transform[pole][column]
                                * z[column]
                        })
                        .sum::<f64>()
                })
                .sum::<f64>();
        }
    }
    finite_values(&derivative)?;
    Ok(derivative)
}

/// Inner iterations one work unit of [`SymmetricEigenJob`] may spend. A unit
/// is a block of columns or rotations sized to this, or one step whose own
/// cost is linear in the trace, so a unit's time does not grow with the
/// square of the trace the way a whole Householder step or QL iteration does.
/// The assembly runs 32 units at a time: at 1,042 trace nodes such a step
/// took 0.12 ms typically and 0.36 ms at the 99th percentile, well inside the
/// 4 ms a browser frame lends preparation.
const EIGEN_UNIT_WORK: usize = 1 << 13;

/// Implicit QL iterations one eigenvalue may take before the solve is refused.
/// EISPACK's bound; the trace operators here take under two on average and
/// five at most. LAPACK's `dsteqr` budgets 30·n for the whole solve instead;
/// see "Worth checking sometime" in `docs/plan.md`.
const EIGEN_MAXIMUM_ITERATIONS: usize = 30;

/// Dependency-free, cooperative eigensolver for the symmetric trace oracle:
/// Householder tridiagonalization, then implicit QL, as EISPACK's `tred2` and
/// `tql2` in the form JAMA gives them. It replaced a cyclic Jacobi solve that
/// took 25 s at 1,042 trace nodes; this one takes about 1.2 s there, with
/// eigen residuals near 1e-14 of the matrix where the Jacobi's tolerance left
/// 4e-11. Its arithmetic is `sqrt` and the four operations, so the trace basis
/// does not depend on a platform's transcendental functions.
///
/// `vectors` is column-major, `vectors[column * count + row]`: it starts as
/// the matrix, holds the accumulated transformations, and ends as one
/// eigenvector per contiguous column. Eigenpairs come out ascending.
struct SymmetricEigenJob {
    vectors: Vec<f64>,
    /// `d`: the Householder vector and scales while reducing, then the
    /// diagonal, then the eigenvalues.
    diagonal: Vec<f64>,
    /// `e`: the reduction's scratch, then the subdiagonal.
    off_diagonal: Vec<f64>,
    count: usize,
    phase: EigenPhase,
    /// The current Householder step's `h`, or accumulation step's divisor.
    step_scale: f64,
    /// QL's accumulated shift `f` and its running negligibility scale `tst1`.
    shift: f64,
    negligible: f64,
    iterations: usize,
    /// One QL iteration's plane rotations `(c, s)`, from row `m - 1` down to
    /// `l`, kept so their application to the vectors can be split into units.
    rotations: Vec<(f64, f64)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EigenPhase {
    ReduceSetup {
        step: usize,
    },
    ReduceMultiply {
        step: usize,
        column: usize,
    },
    ReduceScale {
        step: usize,
    },
    ReduceUpdate {
        step: usize,
        column: usize,
    },
    AccumulateSetup {
        step: usize,
    },
    AccumulateColumns {
        step: usize,
        column: usize,
    },
    AccumulateFinish,
    Deflate {
        eigenvalue: usize,
    },
    Sweep {
        eigenvalue: usize,
        end: usize,
    },
    Rotate {
        eigenvalue: usize,
        end: usize,
        next: usize,
    },
    Sort {
        position: usize,
    },
    Done,
}

type SymmetricEigenResult = (Vec<f64>, Vec<f64>);

/// `sqrt(a^2 + b^2)` without overflow, from `sqrt` alone.
fn portable_hypot(a: f64, b: f64) -> f64 {
    let (a, b) = (a.abs(), b.abs());
    let (large, small) = if a > b { (a, b) } else { (b, a) };
    if large == 0.0 {
        return 0.0;
    }
    let ratio = small / large;
    large * (1.0 + ratio * ratio).sqrt()
}

impl SymmetricEigenJob {
    fn new(matrix: Vec<f64>, count: usize) -> Result<Self, WaveError> {
        if matrix.len() != count * count || matrix.iter().any(|value| !value.is_finite()) {
            return Err(WaveError::InvalidMesh(
                "the outgoing trace matrix is invalid",
            ));
        }
        let diagonal = (0..count)
            .map(|column| matrix[column * count + count - 1])
            .collect();
        let phase = match count {
            0 => EigenPhase::Sort { position: 0 },
            1 => EigenPhase::AccumulateFinish,
            _ => EigenPhase::ReduceSetup { step: count - 1 },
        };
        Ok(Self {
            vectors: matrix,
            diagonal,
            off_diagonal: vec![0.0; count],
            count,
            phase,
            step_scale: 0.0,
            shift: 0.0,
            negligible: 0.0,
            iterations: 0,
            rotations: Vec::with_capacity(count),
        })
    }

    fn advance(&mut self, budget: usize) -> Option<Result<SymmetricEigenResult, WaveError>> {
        for _ in 0..budget {
            match self.phase {
                EigenPhase::Done => return None,
                EigenPhase::Sort { position } if position + 1 >= self.count => {
                    self.phase = EigenPhase::Done;
                    return Some(self.finish());
                }
                _ => {}
            }
            if let Err(error) = self.unit() {
                self.phase = EigenPhase::Done;
                return Some(Err(error));
            }
        }
        None
    }

    fn finish(&mut self) -> Result<SymmetricEigenResult, WaveError> {
        let values = std::mem::take(&mut self.diagonal);
        let vectors = std::mem::take(&mut self.vectors);
        if values
            .iter()
            .chain(&vectors)
            .any(|value| !value.is_finite())
        {
            return Err(WaveError::InvalidMesh(
                "the outgoing trace eigensolve did not converge",
            ));
        }
        Ok((values, vectors))
    }

    /// `V[row][column]`.
    fn at(&self, row: usize, column: usize) -> f64 {
        self.vectors[column * self.count + row]
    }

    fn set(&mut self, row: usize, column: usize, value: f64) {
        self.vectors[column * self.count + row] = value;
    }

    /// One bounded unit of whichever phase is current.
    fn unit(&mut self) -> Result<(), WaveError> {
        let n = self.count;
        match self.phase {
            // Householder step `i` reduces row `i` onto the subdiagonal. `d`
            // holds that row, which the step turns into its reflector.
            EigenPhase::ReduceSetup { step: i } => {
                let scale = self.diagonal[..i]
                    .iter()
                    .map(|value| value.abs())
                    .sum::<f64>();
                if scale == 0.0 {
                    self.off_diagonal[i] = self.diagonal[i - 1];
                    for j in 0..i {
                        self.diagonal[j] = self.at(i - 1, j);
                        self.set(i, j, 0.0);
                        self.set(j, i, 0.0);
                    }
                    self.diagonal[i] = 0.0;
                    self.phase = self.after_reduction(i);
                    return Ok(());
                }
                let mut h = 0.0;
                for value in &mut self.diagonal[..i] {
                    *value /= scale;
                    h += *value * *value;
                }
                let f = self.diagonal[i - 1];
                let g = if f > 0.0 { -h.sqrt() } else { h.sqrt() };
                self.off_diagonal[i] = scale * g;
                h -= f * g;
                self.diagonal[i - 1] = f - g;
                self.off_diagonal[..i].fill(0.0);
                self.step_scale = h;
                self.phase = EigenPhase::ReduceMultiply { step: i, column: 0 };
            }
            // `e = A u`, from the lower triangle only.
            EigenPhase::ReduceMultiply { step: i, column } => {
                let mut j = column;
                let mut work = 0;
                while j < i && work < EIGEN_UNIT_WORK {
                    let f = self.diagonal[j];
                    self.set(j, i, f);
                    let column = &self.vectors[j * n + j..j * n + i];
                    let mut g = self.off_diagonal[j] + column[0] * f;
                    for ((entry, d), e) in column[1..]
                        .iter()
                        .zip(&self.diagonal[j + 1..i])
                        .zip(&mut self.off_diagonal[j + 1..i])
                    {
                        g += entry * d;
                        *e += entry * f;
                    }
                    self.off_diagonal[j] = g;
                    work += i - j;
                    j += 1;
                }
                self.phase = if j == i {
                    EigenPhase::ReduceScale { step: i }
                } else {
                    EigenPhase::ReduceMultiply { step: i, column: j }
                };
            }
            EigenPhase::ReduceScale { step: i } => {
                let h = self.step_scale;
                let mut f = 0.0;
                for j in 0..i {
                    self.off_diagonal[j] /= h;
                    f += self.off_diagonal[j] * self.diagonal[j];
                }
                let hh = f / (h + h);
                for j in 0..i {
                    self.off_diagonal[j] -= hh * self.diagonal[j];
                }
                self.phase = EigenPhase::ReduceUpdate { step: i, column: 0 };
            }
            // The rank-two update `A - u q^T - q u^T` of the leading block.
            EigenPhase::ReduceUpdate { step: i, column } => {
                let mut j = column;
                let mut work = 0;
                while j < i && work < EIGEN_UNIT_WORK {
                    let f = self.diagonal[j];
                    let g = self.off_diagonal[j];
                    for ((entry, e), d) in self.vectors[j * n + j..j * n + i]
                        .iter_mut()
                        .zip(&self.off_diagonal[j..i])
                        .zip(&self.diagonal[j..i])
                    {
                        *entry -= f * e + g * d;
                    }
                    self.diagonal[j] = self.at(i - 1, j);
                    self.set(i, j, 0.0);
                    work += i - j + 1;
                    j += 1;
                }
                if j == i {
                    self.diagonal[i] = self.step_scale;
                    self.phase = self.after_reduction(i);
                } else {
                    self.phase = EigenPhase::ReduceUpdate { step: i, column: j };
                }
            }
            // Accumulation step `i` applies reflector `i + 1` to the
            // transformations gathered so far.
            EigenPhase::AccumulateSetup { step: i } => {
                self.set(n - 1, i, self.at(i, i));
                self.set(i, i, 1.0);
                let h = self.diagonal[i + 1];
                if h != 0.0 {
                    for k in 0..=i {
                        self.diagonal[k] = self.at(k, i + 1) / h;
                    }
                    self.phase = EigenPhase::AccumulateColumns { step: i, column: 0 };
                } else {
                    self.phase = self.after_accumulation(i);
                }
            }
            EigenPhase::AccumulateColumns { step: i, column } => {
                let (columns, reflector) = self.vectors.split_at_mut((i + 1) * n);
                let reflector = &reflector[..=i];
                let scaled = &self.diagonal[..=i];
                let mut j = column;
                let mut work = 0;
                while j <= i && work < EIGEN_UNIT_WORK {
                    let column = &mut columns[j * n..j * n + i + 1];
                    let g = reflector
                        .iter()
                        .zip(column.iter())
                        .map(|(a, b)| a * b)
                        .sum::<f64>();
                    for (entry, d) in column.iter_mut().zip(scaled) {
                        *entry -= g * d;
                    }
                    work += 2 * (i + 1);
                    j += 1;
                }
                self.phase = if j > i {
                    self.after_accumulation(i)
                } else {
                    EigenPhase::AccumulateColumns { step: i, column: j }
                };
            }
            EigenPhase::AccumulateFinish => {
                for j in 0..n {
                    self.diagonal[j] = self.at(n - 1, j);
                    self.set(n - 1, j, 0.0);
                }
                self.set(n - 1, n - 1, 1.0);
                // The subdiagonal moves up one place for QL.
                self.off_diagonal.copy_within(1.., 0);
                self.off_diagonal[n - 1] = 0.0;
                self.phase = EigenPhase::Deflate { eigenvalue: 0 };
            }
            // Eigenvalue `l` is settled once the subdiagonal next to it is
            // negligible; `m` bounds the unreduced block it sits in.
            EigenPhase::Deflate { eigenvalue: l } => {
                if l == n {
                    self.phase = EigenPhase::Sort { position: 0 };
                    return Ok(());
                }
                self.negligible = self
                    .negligible
                    .max(self.diagonal[l].abs() + self.off_diagonal[l].abs());
                let threshold = f64::EPSILON * self.negligible;
                let m = (l..n)
                    .find(|&m| self.off_diagonal[m].abs() <= threshold)
                    .unwrap_or(n - 1);
                if m == l {
                    self.settle(l);
                } else {
                    self.iterations = 0;
                    self.phase = EigenPhase::Sweep {
                        eigenvalue: l,
                        end: m,
                    };
                }
            }
            // One implicit QL iteration on the scalars, recording its
            // rotations for the vectors.
            EigenPhase::Sweep {
                eigenvalue: l,
                end: m,
            } => {
                self.iterations += 1;
                if self.iterations > EIGEN_MAXIMUM_ITERATIONS {
                    return Err(WaveError::InvalidMesh(
                        "the outgoing trace eigensolve did not converge",
                    ));
                }
                let d = &mut self.diagonal;
                let e = &mut self.off_diagonal;
                let mut g = d[l];
                let mut p = (d[l + 1] - g) / (2.0 * e[l]);
                let mut r = portable_hypot(p, 1.0);
                if p < 0.0 {
                    r = -r;
                }
                d[l] = e[l] / (p + r);
                d[l + 1] = e[l] * (p + r);
                let dl1 = d[l + 1];
                let mut h = g - d[l];
                for value in &mut d[l + 2..] {
                    *value -= h;
                }
                self.shift += h;
                p = d[m];
                let mut c = 1.0;
                let mut c2 = c;
                let mut c3 = c;
                let el1 = e[l + 1];
                let mut s = 0.0;
                let mut s2 = 0.0;
                self.rotations.clear();
                for i in (l..m).rev() {
                    c3 = c2;
                    c2 = c;
                    s2 = s;
                    g = c * e[i];
                    h = c * p;
                    r = portable_hypot(p, e[i]);
                    e[i + 1] = s * r;
                    s = e[i] / r;
                    c = p / r;
                    p = c * d[i] - s * g;
                    d[i + 1] = h + s * (c * g + s * d[i]);
                    self.rotations.push((c, s));
                }
                p = -s * s2 * c3 * el1 * e[l] / dl1;
                e[l] = s * p;
                d[l] = c * p;
                self.phase = EigenPhase::Rotate {
                    eigenvalue: l,
                    end: m,
                    next: 0,
                };
            }
            // The iteration's rotations on the vectors, a run of them at a
            // time: each mixes two whole columns, and every row still meets
            // them in their order.
            EigenPhase::Rotate {
                eigenvalue: l,
                end: m,
                next,
            } => {
                let stop = (next + (EIGEN_UNIT_WORK / n).max(1)).min(self.rotations.len());
                for (index, &(c, s)) in self.rotations[next..stop].iter().enumerate() {
                    let i = m - 1 - (next + index);
                    let (left, right) = self.vectors.split_at_mut((i + 1) * n);
                    for (current, following) in left[i * n..].iter_mut().zip(&mut right[..n]) {
                        let h = *following;
                        *following = s * *current + c * h;
                        *current = c * *current - s * h;
                    }
                }
                if stop < self.rotations.len() {
                    self.phase = EigenPhase::Rotate {
                        eigenvalue: l,
                        end: m,
                        next: stop,
                    };
                } else if self.off_diagonal[l].abs() > f64::EPSILON * self.negligible {
                    self.phase = EigenPhase::Sweep {
                        eigenvalue: l,
                        end: m,
                    };
                } else {
                    self.settle(l);
                }
            }
            // Selection sort, ascending, moving whole columns with their
            // eigenvalues. Ties keep their first position.
            EigenPhase::Sort { position } => {
                let mut l = position;
                let mut work = 0;
                while l + 1 < n && work < EIGEN_UNIT_WORK {
                    let mut smallest = l;
                    for k in l + 1..n {
                        if self.diagonal[k] < self.diagonal[smallest] {
                            smallest = k;
                        }
                    }
                    if smallest != l {
                        self.diagonal.swap(l, smallest);
                        let (left, right) = self.vectors.split_at_mut(smallest * n);
                        left[l * n..(l + 1) * n].swap_with_slice(&mut right[..n]);
                    }
                    work += 2 * n - l;
                    l += 1;
                }
                self.phase = EigenPhase::Sort { position: l };
            }
            EigenPhase::Done => {}
        }
        Ok(())
    }

    fn after_reduction(&self, step: usize) -> EigenPhase {
        if step > 1 {
            EigenPhase::ReduceSetup { step: step - 1 }
        } else {
            EigenPhase::AccumulateSetup { step: 0 }
        }
    }

    fn after_accumulation(&mut self, step: usize) -> EigenPhase {
        let reflector = (step + 1) * self.count;
        self.vectors[reflector..=reflector + step].fill(0.0);
        if step + 2 < self.count {
            EigenPhase::AccumulateSetup { step: step + 1 }
        } else {
            EigenPhase::AccumulateFinish
        }
    }

    /// Eigenvalue `l` is final: restore the shifts QL took from it.
    fn settle(&mut self, l: usize) {
        self.diagonal[l] += self.shift;
        self.off_diagonal[l] = 0.0;
        self.phase = EigenPhase::Deflate { eigenvalue: l + 1 };
    }
}

/// Returns `S=L^T` and `S^-1` for `H=L L^T`, so `z=Sx` and
/// `|z|^2=x^T H x`. A Cholesky energy coordinate is as valid as the symmetric
/// square root and is cheaper to reproduce exactly on CPU/GPU.
type Matrix3 = [[f64; 3]; 3];

pub(crate) fn pole_energy_transform() -> Result<(Matrix3, Matrix3), WaveError> {
    let b = (31.0_f64 / 7.0).sqrt();
    let ell = [0.0, 1.0 - b, 2.0 * b - 4.0];
    let mut h = [[0.0; 3]; 3];
    h[0][0] = 6.0 / 7.0;
    for row in 1..3 {
        for column in 1..3 {
            h[row][column] = 2.0 * ell[row] * ell[column] / (row as f64 + column as f64);
        }
    }
    let mut lower = [[0.0; 3]; 3];
    for row in 0..3 {
        for column in 0..=row {
            let remainder = h[row][column]
                - (0..column)
                    .map(|index| lower[row][index] * lower[column][index])
                    .sum::<f64>();
            lower[row][column] = if row == column {
                if remainder <= 0.0 || !remainder.is_finite() {
                    return Err(WaveError::Unsupported(
                        "a compiled element matrix is not positive definite",
                    ));
                }
                remainder.sqrt()
            } else {
                remainder / lower[column][column]
            };
        }
    }
    let mut transform = [[0.0; 3]; 3];
    for row in 0..3 {
        for column in 0..3 {
            transform[row][column] = lower[column][row];
        }
    }
    let mut inverse = [[0.0; 3]; 3];
    for column in [0, 1, 2] {
        for row in (0..3).rev() {
            let rhs = f64::from(row == column)
                - (row + 1..3)
                    .map(|index| transform[row][index] * inverse[index][column])
                    .sum::<f64>();
            inverse[row][column] = rhs / transform[row][row];
        }
    }
    Ok((transform, inverse))
}

/// The inverse of a dense column-major-agnostic `count x count` system, by one
/// elimination rather than one per column.
///
/// Solving for each basis vector separately re-factorizes the same matrix
/// `count` times, which is `O(count^4)`; the trace count grows as the square
/// root of the mesh, so that is quadratic in the degrees of freedom and it was
/// the whole cost of packing a generation - 613 ms at 288 trace nodes, against
/// 19 ms for the packing it was preparing. Eliminating once and carrying the
/// identity along is the same arithmetic in `O(count^3)`.
///
/// Pivoting, the singularity threshold and the finiteness check match
/// [`solve_dense`], so a system either function rejects is rejected by both.
/// Inverts a symmetric positive-definite matrix through its Cholesky factor,
/// reading only the lower triangle. Every inner loop runs along a row, which
/// is what keeps a trace of several hundred nodes to tens of milliseconds: the
/// pivoted elimination this replaced walked columns of the inverse in its back
/// substitution and took several times as long for the same answer.
fn invert_symmetric_positive(mut matrix: Vec<f64>, count: usize) -> Result<Vec<f64>, WaveError> {
    if matrix.len() != count * count || matrix.iter().any(|value| !value.is_finite()) {
        return Err(WaveError::InvalidState);
    }
    let dot = |left: &[f64], right: &[f64]| {
        left.iter()
            .zip(right)
            .map(|(left, right)| left * right)
            .sum::<f64>()
    };
    // `L` overwrites the lower triangle, row by row.
    for row in 0..count {
        for column in 0..=row {
            let (above, current) = matrix.split_at_mut(row * count);
            let prefix = if column == row {
                dot(&current[..column], &current[..column])
            } else {
                dot(
                    &current[..column],
                    &above[column * count..column * count + column],
                )
            };
            let value = current[column] - prefix;
            if column == row {
                if !value.is_finite() || value <= 1.0e-14 * current[column].abs().max(1.0) {
                    return Err(WaveError::InvalidState);
                }
                current[column] = value.sqrt();
            } else {
                current[column] = value / above[column * count + column];
            }
        }
    }
    // `Y = L^-1`, lower triangular, one row from the rows above it.
    let mut inverse = vec![0.0; count * count];
    for row in 0..count {
        let (above, current) = inverse.split_at_mut(row * count);
        current[row] = 1.0;
        for index in 0..row {
            let factor = matrix[row * count + index];
            if factor != 0.0 {
                let source = &above[index * count..index * count + index + 1];
                for (target, value) in current[..=index].iter_mut().zip(source) {
                    *target -= factor * value;
                }
            }
        }
        let diagonal = matrix[row * count + row];
        for value in &mut current[..=row] {
            *value /= diagonal;
        }
    }
    // `S^-1 = L^-T Y`, one row from the rows below it.
    for row in (0..count).rev() {
        let (current, below) = inverse.split_at_mut((row + 1) * count);
        let current = &mut current[row * count..];
        for index in row + 1..count {
            let factor = matrix[index * count + row];
            if factor != 0.0 {
                let source = &below[(index - row - 1) * count..(index - row) * count];
                for (target, value) in current.iter_mut().zip(source) {
                    *target -= factor * value;
                }
            }
        }
        let diagonal = matrix[row * count + row];
        for value in current.iter_mut() {
            *value /= diagonal;
        }
    }
    finite_values(&inverse)?;
    Ok(inverse)
}

fn solve_dense(
    mut matrix: Vec<f64>,
    mut right: Vec<f64>,
    count: usize,
) -> Result<Vec<f64>, WaveError> {
    if matrix.len() != count * count
        || right.len() != count
        || matrix.iter().any(|value| !value.is_finite())
        || right.iter().any(|value| !value.is_finite())
    {
        return Err(WaveError::InvalidState);
    }
    for pivot in 0..count {
        let best = (pivot..count)
            .max_by(|left, right_row| {
                matrix[*left * count + pivot]
                    .abs()
                    .total_cmp(&matrix[*right_row * count + pivot].abs())
            })
            .ok_or(WaveError::InvalidState)?;
        let scale = matrix[best * count + pivot].abs();
        if !scale.is_finite() || scale <= 1.0e-14 {
            return Err(WaveError::InvalidState);
        }
        if best != pivot {
            for column in 0..count {
                matrix.swap(pivot * count + column, best * count + column);
            }
            right.swap(pivot, best);
        }
        for row in pivot + 1..count {
            let factor = matrix[row * count + pivot] / matrix[pivot * count + pivot];
            matrix[row * count + pivot] = 0.0;
            for column in pivot + 1..count {
                matrix[row * count + column] -= factor * matrix[pivot * count + column];
            }
            right[row] -= factor * right[pivot];
        }
    }
    let mut solution = vec![0.0; count];
    for row in (0..count).rev() {
        let residual = right[row]
            - (row + 1..count)
                .map(|column| matrix[row * count + column] * solution[column])
                .sum::<f64>();
        solution[row] = residual / matrix[row * count + row];
    }
    finite_values(&solution)?;
    Ok(solution)
}

fn rotate_vector(vector: Point2) -> Point2 {
    Point2::new(-vector.y, vector.x)
}

fn rotate_tensor(values: DirectionalWaveCoefficients) -> SymmetricTensor2 {
    let tensor = values.stiffness;
    SymmetricTensor2::new(tensor.yy, -tensor.xy, tensor.xx)
}

fn inverse_tensor(tensor: SymmetricTensor2) -> Option<SymmetricTensor2> {
    tensor.inverse()
}

/// A state is built under the production integrator's ceiling and steps
/// under its own, so a leapfrog state past the leapfrog's bound fails its
/// first step.
fn validate_time_step(
    operator: &CanonicalWaveOperator,
    time_step: f64,
    integrator: CanonicalIntegrator,
) -> Result<(), WaveError> {
    let maximum = integrator.ceiling() * operator.maximum_time_step();
    if !time_step.is_finite() || time_step <= 0.0 || time_step > maximum {
        Err(WaveError::InvalidTimeStep {
            requested: time_step,
            maximum,
        })
    } else {
        Ok(())
    }
}

fn finite_values(values: &[f64]) -> Result<(), WaveError> {
    if values.iter().any(|value| !value.is_finite()) {
        Err(WaveError::InvalidState)
    } else {
        Ok(())
    }
}

fn sinc(value: f64) -> f64 {
    if value.abs() < 1.0e-4 {
        let squared = value * value;
        1.0 - squared / 6.0 + squared * squared / 120.0
    } else {
        value.sin() / value
    }
}

fn dot(left: &[f64], right: &[f64]) -> f64 {
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}

/// Division by different lumped masses can return adjacent representations of
/// the same authored constant even when every `Q_i` was formed as `M_i u`.
/// Suppress only that roundoff-sized difference before applying a gradient.
fn stable_difference(value: f64, reference: f64) -> f64 {
    let difference = value - reference;
    let roundoff = 8.0 * f64::EPSILON * value.abs().max(reference.abs()).max(1.0);
    if difference.abs() <= roundoff {
        0.0
    } else {
        difference
    }
}

fn project_component_constants(values: &mut [f64], labels: &[u32], count: usize) {
    let mut sums = vec![0.0; count];
    let mut sizes = vec![0usize; count];
    for (value, label) in values.iter().zip(labels) {
        sums[*label as usize] += value;
        sizes[*label as usize] += 1;
    }
    for (value, label) in values.iter_mut().zip(labels) {
        *value -= sums[*label as usize] / sizes[*label as usize] as f64;
    }
}

fn connected_components(
    node_count: usize,
    elements: &[[u32; LOCAL_NODES]],
    thin_gaps: &[ThinGapSample],
) -> (Vec<u32>, usize) {
    let mut parent = (0..node_count).collect::<Vec<_>>();
    for nodes in elements {
        let first = nodes[0] as usize;
        for &node in &nodes[1..] {
            union(&mut parent, first, node as usize);
        }
    }
    for gap in thin_gaps {
        union(&mut parent, gap.left_node as usize, gap.right_node as usize);
    }
    let mut roots = Vec::<usize>::new();
    let labels = (0..node_count)
        .map(|node| {
            let root = find(&mut parent, node);
            if let Some(label) = roots.iter().position(|candidate| *candidate == root) {
                label as u32
            } else {
                roots.push(root);
                (roots.len() - 1) as u32
            }
        })
        .collect();
    (labels, roots.len())
}

fn find(parent: &mut [usize], node: usize) -> usize {
    let mut root = node;
    while parent[root] != root {
        root = parent[root];
    }
    let mut here = node;
    while parent[here] != here {
        let next = parent[here];
        parent[here] = root;
        here = next;
    }
    root
}

fn union(parent: &mut [usize], left: usize, right: usize) {
    let left = find(parent, left);
    let right = find(parent, right);
    if left != right {
        parent[right] = left;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BACKGROUND_REGION, BoundaryEdge, BoundaryLabel, DampingLaw, LossChannel, MaterialFrame,
        MeshQuality, MeshTriangle, MeshVertex, OuterBoundaryCondition, PulseEnvelope,
        QuadraticWaveState, RateLaw, ScalarField, TimeDrive, VolumeSourceContribution,
        VolumeSourceNode,
    };

    /// The one inversion has to give what the per-column solves gave, on a
    /// system with the shape `prepare_static` builds: a dominant diagonal plus
    /// a few rank-one modal corrections, sized like a real trace.
    #[test]
    fn one_elimination_inverts_what_per_column_solves_did() {
        let count = 48;
        let mut system = vec![0.0; count * count];
        for row in 0..count {
            system[row * count + row] = 1.0 + 0.37 * (row as f64 + 1.0);
        }
        for mode in 0..3 {
            let trace = (0..count)
                .map(|index| ((index * (mode + 2)) as f64 * 0.19).sin())
                .collect::<Vec<_>>();
            let correction = 0.21 * (mode as f64 + 1.0);
            for row in 0..count {
                for column in 0..count {
                    system[row * count + column] += correction * trace[row] * trace[column];
                }
            }
        }

        let mut per_column = vec![0.0; count * count];
        for column in 0..count {
            let mut basis = vec![0.0; count];
            basis[column] = 1.0;
            let solved = solve_dense(system.clone(), basis, count).unwrap();
            for (row, value) in solved.into_iter().enumerate() {
                per_column[row * count + column] = value;
            }
        }
        let once = invert_symmetric_positive(system.clone(), count).unwrap();
        for (left, right) in once.iter().zip(&per_column) {
            assert!(
                (left - right).abs() < 1.0e-12 * left.abs().max(1.0),
                "{left} against {right}"
            );
        }

        // And it is an inverse, not merely the same answer as before.
        for row in 0..count {
            for column in 0..count {
                let entry = (0..count)
                    .map(|index| system[row * count + index] * once[index * count + column])
                    .sum::<f64>();
                let expected = f64::from(row == column);
                assert!(
                    (entry - expected).abs() < 1.0e-10,
                    "{row},{column}: {entry}"
                );
            }
        }
    }

    /// A singular or indefinite system is refused rather than inverted.
    #[test]
    fn a_singular_system_has_no_inverse() {
        // Rank one: every entry one.
        assert_eq!(
            invert_symmetric_positive(vec![1.0; 9], 3),
            Err(WaveError::InvalidState),
            "a singular trace system has no direct lane"
        );
        assert_eq!(
            invert_symmetric_positive(vec![1.0, 2.0, 2.0, 1.0], 2),
            Err(WaveError::InvalidState),
            "an indefinite one has none either"
        );
        assert_eq!(
            invert_symmetric_positive(vec![f64::NAN; 4], 2),
            Err(WaveError::InvalidState)
        );
        assert_eq!(
            invert_symmetric_positive(vec![1.0; 3], 2),
            Err(WaveError::InvalidState)
        );
    }

    fn square() -> TriMesh {
        TriMesh {
            geometry_revision: 17,
            mesh_revision: 23,
            vertices: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
                .into_iter()
                .map(|[x, y]| MeshVertex {
                    point: Point2::new(x, y),
                    boundary: None,
                    trace: None,
                })
                .collect(),
            triangles: vec![
                MeshTriangle {
                    vertices: [0, 1, 2],
                    region: BACKGROUND_REGION,
                },
                MeshTriangle {
                    vertices: [0, 2, 3],
                    region: BACKGROUND_REGION,
                },
            ],
            boundary_edges: vec![],
            quality: MeshQuality {
                minimum_angle_degrees: 45.0,
                maximum_edge_length: 2.0_f64.sqrt(),
            },
            requested_sizes: vec![],
        }
    }

    fn disconnected_triangles() -> TriMesh {
        TriMesh {
            geometry_revision: 31,
            mesh_revision: 37,
            vertices: [
                [0.0, 0.0],
                [1.0, 0.0],
                [0.0, 1.0],
                [2.0, 0.0],
                [3.0, 0.0],
                [2.0, 1.0],
            ]
            .into_iter()
            .map(|[x, y]| MeshVertex {
                point: Point2::new(x, y),
                boundary: None,
                trace: None,
            })
            .collect(),
            triangles: vec![
                MeshTriangle {
                    vertices: [0, 1, 2],
                    region: BACKGROUND_REGION,
                },
                MeshTriangle {
                    vertices: [3, 4, 5],
                    region: BACKGROUND_REGION,
                },
            ],
            boundary_edges: vec![],
            quality: MeshQuality {
                minimum_angle_degrees: 45.0,
                maximum_edge_length: 2.0_f64.sqrt(),
            },
            requested_sizes: vec![],
        }
    }

    fn square_with_outer_boundary() -> TriMesh {
        let mut mesh = square();
        mesh.boundary_edges = [
            ([0, 1], OuterSide::Bottom),
            ([1, 2], OuterSide::Right),
            ([2, 3], OuterSide::Top),
            ([3, 0], OuterSide::Left),
        ]
        .map(|(vertices, side)| BoundaryEdge {
            vertices,
            label: BoundaryLabel::Outer(side),
            parameters: [0.0, 1.0],
        })
        .to_vec();
        mesh
    }

    fn compile(scene: &Scene) -> (QuadraticWaveOperator, CanonicalWaveOperator) {
        let mesh = square();
        let quadratic =
            QuadraticWaveOperator::assemble_scene(&mesh, scene, OuterBoundaryCondition::Reflecting)
                .unwrap();
        let canonical = CanonicalWaveOperator::compile_scene(&mesh, &quadratic, scene, 29).unwrap();
        (quadratic, canonical)
    }

    fn configured_scene(physics: PhysicsModel) -> Scene {
        let mut scene = Scene {
            physics,
            ..Scene::default()
        };
        scene.materials[0].mass_density = ScalarField::constant(2.5);
        scene.materials[0].stiffness = ScalarField::constant(1.7);
        scene.materials[0].axis_ratio = ScalarField::constant(3.2);
        scene.regions[0].frame = MaterialFrame {
            origin: Point2::new(0.2, -0.1),
            angle_radians: 0.37,
            attachment: crate::MaterialFrameAttachment::World,
        };
        scene
    }

    fn test_potential(operator: &CanonicalWaveOperator) -> Vec<f64> {
        operator
            .node_points()
            .iter()
            .enumerate()
            .map(|(index, point)| {
                0.3 * point.x - 0.2 * point.y + 0.17 * point.x * point.y - 0.013 * index as f64
            })
            .collect()
    }

    fn maximum_difference(left: &[f64], right: &[f64]) -> f64 {
        left.iter()
            .zip(right)
            .map(|(left, right)| (left - right).abs())
            .fold(0.0, f64::max)
    }

    #[test]
    fn compatible_force_matches_the_existing_tensor_stiffness_for_every_skin() {
        for physics in [
            PhysicsModel::Mechanical,
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            },
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            },
        ] {
            let scene = configured_scene(physics);
            let (quadratic, canonical) = compile(&scene);
            let potential = test_potential(&canonical);
            let legacy = quadratic.apply_stiffness(&potential).unwrap();
            let direct = canonical.compatible_stiffness(&potential).unwrap();
            assert!(
                maximum_difference(&legacy, &direct) < 2.0e-13,
                "{physics:?}: {legacy:?} versus {direct:?}"
            );
            assert_eq!(canonical.primary_mass(), quadratic.lumped_mass());
            assert_eq!(
                canonical.orientation(),
                if matches!(
                    physics,
                    PhysicsModel::Electromagnetic {
                        polarization: ElectromagneticPolarization::Tm
                    }
                ) {
                    1.0
                } else {
                    -1.0
                }
            );
        }
    }

    #[test]
    fn compiler_keeps_geometric_weights_coefficients_and_rotated_inverse_separate() {
        let scene = configured_scene(PhysicsModel::Mechanical);
        let (_, canonical) = compile(&scene);
        assert_eq!(canonical.primary_contributions().len(), 14);
        assert_eq!(canonical.constitutive_samples().len(), 12);
        let mut assembled = vec![0.0; canonical.degrees_of_freedom()];
        for contribution in canonical.primary_contributions() {
            assembled[contribution.node as usize] +=
                contribution.geometric_weight * contribution.reference_coefficient;
        }
        assert_eq!(assembled, canonical.primary_mass());

        let sample = canonical.constitutive_samples()[0];
        let scalar = scene.materials[0]
            .evaluate(scene.regions[0].frame, sample.point)
            .unwrap();
        let tensor = scene
            .physics
            .directional_wave_coefficients(scalar, scene.regions[0].frame)
            .stiffness;
        assert_eq!(
            sample.complementary_inverse,
            SymmetricTensor2::new(tensor.yy, -tensor.xy, tensor.xx)
        );
        let product = sample
            .complementary_reference
            .apply(sample.complementary_inverse.apply(Point2::new(0.3, -0.8)));
        assert!((product - Point2::new(0.3, -0.8)).norm() < 2.0e-15);
        assert!(sample.complementary_inverse.xy.abs() > 0.1);
    }

    #[test]
    fn resumable_compiler_owns_its_material_snapshot() {
        let mesh = Arc::new(square());
        let mut scene = configured_scene(PhysicsModel::Mechanical);
        let quadratic = Arc::new(
            QuadraticWaveOperator::assemble_scene(
                &mesh,
                &scene,
                OuterBoundaryCondition::Reflecting,
            )
            .unwrap(),
        );
        let expected = CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 43).unwrap();
        let mut job =
            CanonicalAssemblyJob::new(mesh, quadratic, TopologyWaveModel::from_scene(&scene), 43)
                .unwrap();
        scene.materials[0].mass_density = ScalarField::constant(99.0);
        assert!(job.advance(1).is_none());
        let actual = loop {
            if let Some(result) = job.advance(1) {
                break result.unwrap();
            }
        };
        assert_eq!(actual, expected);
        assert_eq!(actual.generation().geometry_revision, 17);
        assert_eq!(actual.generation().mesh_revision, 23);
        assert_eq!(actual.generation().constitutive_revision, 43);
    }

    /// Runs the cooperative eigensolve to completion, one unit at a time, and
    /// returns its result with the number of units it took.
    fn eigensolve(matrix: &[f64], count: usize) -> (Vec<f64>, Vec<f64>, usize) {
        let mut job = SymmetricEigenJob::new(matrix.to_vec(), count).unwrap();
        let mut units = 1;
        loop {
            if let Some(result) = job.advance(1) {
                let (values, vectors) = result.unwrap();
                return (values, vectors, units);
            }
            units += 1;
        }
    }

    /// The largest eigen-residual `|A v - lambda v|` over the matrix's largest
    /// row sum, and the largest departure of the vectors from orthonormality.
    fn eigen_errors(matrix: &[f64], count: usize, values: &[f64], vectors: &[f64]) -> [f64; 2] {
        let norm = (0..count)
            .map(|row| {
                (0..count)
                    .map(|column| matrix[row * count + column].abs())
                    .sum::<f64>()
            })
            .fold(0.0, f64::max)
            .max(f64::MIN_POSITIVE);
        let column = |mode: usize| &vectors[mode * count..(mode + 1) * count];
        let mut residual = 0.0_f64;
        for (mode, value) in values.iter().enumerate() {
            for row in 0..count {
                let product = (0..count)
                    .map(|entry| matrix[row * count + entry] * column(mode)[entry])
                    .sum::<f64>();
                residual = residual.max((product - value * column(mode)[row]).abs());
            }
        }
        let mut orthogonality = 0.0_f64;
        for left in 0..count {
            for right in left..count {
                let dot = column(left)
                    .iter()
                    .zip(column(right))
                    .map(|(a, b)| a * b)
                    .sum::<f64>();
                let expected = if left == right { 1.0 } else { 0.0 };
                orthogonality = orthogonality.max((dot - expected).abs());
            }
        }
        [residual / norm, orthogonality]
    }

    /// A closed trace is a ring: its tangential operator has one zero mode and
    /// every other eigenvalue twice, `2 - 2 cos(2 pi k / n)` for a uniform one.
    /// The solve finds them to roundoff, with orthonormal vectors, and yields
    /// many times on the way instead of spending one long unit.
    #[test]
    fn the_trace_eigensolve_finds_a_rings_spectrum_in_bounded_units() {
        let count = 160;
        let mut matrix = vec![0.0; count * count];
        for row in 0..count {
            let next = (row + 1) % count;
            matrix[row * count + row] = 2.0;
            matrix[row * count + next] = -1.0;
            matrix[next * count + row] = -1.0;
        }
        let (values, vectors, units) = eigensolve(&matrix, count);
        let mut expected = (0..count)
            .map(|k| 2.0 - 2.0 * (std::f64::consts::TAU * k as f64 / count as f64).cos())
            .collect::<Vec<_>>();
        expected.sort_by(f64::total_cmp);
        for (value, expected) in values.iter().zip(&expected) {
            assert!(
                (value - expected).abs() < 1.0e-13,
                "{value} against {expected}"
            );
        }
        assert!(values.windows(2).all(|pair| pair[0] <= pair[1]));
        let [residual, orthogonality] = eigen_errors(&matrix, count, &values, &vectors);
        assert!(residual < 1.0e-14, "{residual:e}");
        assert!(orthogonality < 1.0e-13, "{orthogonality:e}");
        // One unit per Householder and accumulation step at the least.
        assert!(units > 2 * count, "{units}");
    }

    /// A trace in pieces, with a node coupled to nothing, is block diagonal:
    /// the reduction meets rows that are already reduced. A general
    /// symmetric matrix, a diagonal one out of order, and one and two nodes
    /// take the same path.
    #[test]
    fn the_trace_eigensolve_takes_disconnected_general_and_small_matrices() {
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut random = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1_u64 << 53) as f64 - 0.5
        };
        let mut cases = Vec::new();
        let count = 40;
        let mut pieces = vec![0.0; count * count];
        for row in 0..count - 1 {
            pieces[row * count + row] = 1.0 + row as f64 * 0.1;
            if row + 1 < count - 1 && row != 16 {
                pieces[row * count + row + 1] = -0.7;
                pieces[(row + 1) * count + row] = -0.7;
            }
        }
        pieces[count * count - 1] = 3.0;
        cases.push((pieces, count));
        let count = 57;
        let mut general = vec![0.0; count * count];
        for row in 0..count {
            for column in 0..=row {
                let value = random();
                general[row * count + column] = value;
                general[column * count + row] = value;
            }
        }
        cases.push((general, count));
        let count = 6;
        let mut diagonal = vec![0.0; count * count];
        for (row, value) in [3.0, -1.0, 0.0, 2.5, -1.0, 7.0].into_iter().enumerate() {
            diagonal[row * count + row] = value;
        }
        cases.push((diagonal, count));
        cases.push((vec![4.0], 1));
        cases.push((vec![2.0, -1.0, -1.0, 2.0], 2));
        for (matrix, count) in cases {
            let (values, vectors, _) = eigensolve(&matrix, count);
            assert_eq!(values.len(), count);
            assert!(
                values.windows(2).all(|pair| pair[0] <= pair[1]),
                "{values:?}"
            );
            let trace = (0..count).map(|row| matrix[row * count + row]).sum::<f64>();
            assert!((values.iter().sum::<f64>() - trace).abs() < 1.0e-12 * count as f64);
            let [residual, orthogonality] = eigen_errors(&matrix, count, &values, &vectors);
            assert!(residual < 1.0e-14, "{count}: {residual:e}");
            assert!(orthogonality < 1.0e-13, "{count}: {orthogonality:e}");
        }
        let (values, _, _) = eigensolve(&[2.0, -1.0, -1.0, 2.0], 2);
        assert!((values[0] - 1.0).abs() < 1.0e-15 && (values[1] - 3.0).abs() < 1.0e-15);
    }

    #[test]
    fn stage_three_compiler_migrates_legacy_loss_as_a_primary_rate() {
        let mesh = square();
        let mut scene = Scene::default();
        scene.materials[0].damping = ScalarField::constant(0.1);
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let operator = CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 7).unwrap();
        assert!(
            operator
                .primary_loss_rate()
                .iter()
                .all(|rate| (*rate - 0.1).abs() < 1.0e-14)
        );
        assert!(
            operator
                .complementary_loss_rate()
                .iter()
                .all(|rate| *rate == 0.0)
        );

        let dt = 0.2 * operator.maximum_time_step();
        let mut state = CanonicalWaveState::from_primary_and_potential(
            &operator,
            dt,
            &vec![0.7; operator.degrees_of_freedom()],
            &vec![0.0; operator.degrees_of_freedom()],
        )
        .unwrap();
        state.step(&operator).unwrap();
        let expected = 0.7 * (-0.1 * dt).exp();
        assert!(
            state
                .primary_field(&operator)
                .unwrap()
                .iter()
                .all(|value| (*value - expected).abs() < 2.0e-15)
        );
    }

    #[test]
    fn named_electric_and_magnetic_loss_follow_physical_fields_across_skins() {
        for (physics, expected_primary, expected_complementary) in [
            (PhysicsModel::Mechanical, 0.7, 0.3),
            (
                PhysicsModel::Electromagnetic {
                    polarization: ElectromagneticPolarization::Tm,
                },
                0.3,
                0.7,
            ),
            (
                PhysicsModel::Electromagnetic {
                    polarization: ElectromagneticPolarization::Te,
                },
                0.7,
                0.3,
            ),
        ] {
            let mesh = square();
            let mut scene = configured_scene(physics);
            scene.materials[0].electric_loss = Some(LossChannel {
                base_rate: ScalarField::constant(0.3),
                law: DampingLaw {
                    rate: RateLaw::Constant,
                    drive: TimeDrive::None,
                    gate: None,
                },
            });
            scene.materials[0].magnetic_loss = Some(LossChannel {
                base_rate: ScalarField::constant(0.7),
                law: DampingLaw {
                    rate: RateLaw::Constant,
                    drive: TimeDrive::None,
                    gate: None,
                },
            });
            let mut legacy_geometry = scene.clone();
            legacy_geometry.materials[0].electric_loss = None;
            legacy_geometry.materials[0].magnetic_loss = None;
            let quadratic = QuadraticWaveOperator::assemble_scene(
                &mesh,
                &legacy_geometry,
                OuterBoundaryCondition::Reflecting,
            )
            .unwrap();
            let operator =
                CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 13).unwrap();
            assert!(
                operator
                    .primary_loss_rate()
                    .iter()
                    .all(|rate| (*rate - expected_primary).abs() < 1.0e-13)
            );
            assert!(
                operator
                    .complementary_loss_rate()
                    .iter()
                    .all(|rate| (*rate - expected_complementary).abs() < 1.0e-13)
            );
        }
    }

    #[test]
    fn spatial_fixed_loss_rates_are_frozen_at_their_physical_support() {
        let mesh = square();
        let mut scene = configured_scene(PhysicsModel::Mechanical);
        scene.materials[0].electric_loss = Some(LossChannel {
            base_rate: ScalarField::formula("0.2 + 0.1*x").unwrap(),
            law: DampingLaw {
                rate: RateLaw::Constant,
                drive: TimeDrive::None,
                gate: None,
            },
        });
        scene.materials[0].magnetic_loss = Some(LossChannel {
            base_rate: ScalarField::formula("0.4 + 0.1*y").unwrap(),
            law: DampingLaw {
                rate: RateLaw::Constant,
                drive: TimeDrive::None,
                gate: None,
            },
        });
        let mut legacy_geometry = scene.clone();
        legacy_geometry.materials[0].electric_loss = None;
        legacy_geometry.materials[0].magnetic_loss = None;
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &legacy_geometry,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let operator = CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 19).unwrap();
        assert!(
            operator
                .primary_loss_rate()
                .windows(2)
                .any(|rates| rates[0] != rates[1])
        );
        assert!(
            operator
                .complementary_loss_rate()
                .windows(2)
                .any(|rates| rates[0] != rates[1])
        );
        let primary = operator
            .node_points()
            .iter()
            .map(|point| 0.5 + 0.2 * point.x)
            .collect::<Vec<_>>();
        let potential = test_potential(&operator);
        let mut state = CanonicalWaveState::from_primary_and_potential(
            &operator,
            0.1 * operator.maximum_time_step(),
            &primary,
            &potential,
        )
        .unwrap();
        let accounting = state
            .step_with_forcing(&operator, &CanonicalForcing::none(&operator))
            .unwrap();
        assert!(accounting.primary_loss > 0.0);
        assert!(accounting.complementary_loss > 0.0);
    }

    #[test]
    fn legacy_source_integral_is_continuous_at_zero_frequency_and_drives_q_directly() {
        let (_, operator) = compile(&Scene::default());
        let signal = TimeSignal::harmonic(2.0, 3.0, 0.0, std::f64::consts::FRAC_PI_2);
        let drive = CanonicalRateDrive::legacy(signal, 0.0).unwrap();
        assert_eq!(drive.value(0.0).unwrap(), 0.0);
        assert!((drive.value(0.25).unwrap() - 1.25).abs() < 2.0e-15);

        let dt = 0.1 * operator.maximum_time_step();
        let mut forcing = CanonicalForcing::none(&operator);
        forcing
            .push_source(
                CanonicalSource::new(&operator, operator.primary_mass().to_vec(), drive).unwrap(),
            )
            .unwrap();
        let mut state = CanonicalWaveState::zero(&operator, dt).unwrap();
        let accounting = state.step_with_forcing(&operator, &forcing).unwrap();
        let expected = 0.5 * dt * drive.value(dt).unwrap();
        assert!(
            state
                .primary_field(&operator)
                .unwrap()
                .iter()
                .all(|value| (*value - expected).abs() < 2.0e-14)
        );
        assert!(accounting.source_work > 0.0);
    }

    #[test]
    fn legacy_volume_and_weak_boundary_sources_compile_to_integrated_q_weights() {
        let (_, operator) = compile(&Scene::default());
        let signal = TimeSignal::harmonic(0.2, 0.4, 1.3, 0.7);
        let volume = CompiledVolumeSources {
            signals: vec![signal],
            nodes: operator
                .primary_mass()
                .iter()
                .enumerate()
                .map(|(node, _)| VolumeSourceNode {
                    contributions: vec![VolumeSourceContribution {
                        channel: 0,
                        weight: 0.3 + 0.01 * node as f64,
                    }],
                })
                .collect(),
        };
        let mut forcing = CanonicalForcing::none(&operator);
        forcing
            .extend_legacy_volume(&operator, &volume, 0.0)
            .unwrap();
        assert_eq!(forcing.sources().len(), 1);
        assert!(
            forcing.sources()[0]
                .weights()
                .iter()
                .zip(operator.primary_mass())
                .enumerate()
                .all(|(node, (weight, mass))| {
                    (*weight - mass * (0.3 + 0.01 * node as f64)).abs() < 2.0e-15
                })
        );

        let authored = [
            VolumeSource {
                region: crate::RegionId(4),
                enabled: false,
                profile: crate::ScalarField::constant(1.0),
                parameters: vec![],
                signal: TimeSignal::harmonic(0.0, 0.7, 2.0, 0.1),
            },
            VolumeSource {
                region: crate::RegionId(5),
                enabled: true,
                profile: crate::ScalarField::constant(1.0),
                parameters: vec![],
                signal,
            },
        ];
        let mut slotted = CanonicalForcing::none(&operator);
        slotted
            .extend_legacy_volume_slots(&operator, &volume, &authored, 0.0)
            .unwrap();
        assert_eq!(slotted.sources().len(), 2);
        assert!(
            slotted.sources()[0]
                .weights()
                .iter()
                .all(|weight| *weight == 0.0)
        );
        assert_eq!(
            slotted.sources()[1].weights(),
            forcing.sources()[0].weights()
        );

        let mesh = square_with_outer_boundary();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &Scene::default(),
            OuterBoundaryCondition::Neumann { signal },
        )
        .unwrap();
        let canonical =
            CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &Scene::default(), 17).unwrap();
        let forcing =
            CanonicalForcing::from_legacy_boundaries(&canonical, &quadratic, 0.0).unwrap();
        assert_eq!(forcing.sources().len(), 4);
        for source in forcing.sources() {
            assert!(
                (source.weights().iter().sum::<f64>() - 1.0).abs() < 2.0e-14,
                "weak source was normalized instead of edge-integrated"
            );
            assert!(matches!(
                source.drive(),
                CanonicalRateDrive::LegacyIntegratedHarmonic { acceleration, .. }
                    if acceleration == signal
            ));
        }
    }

    /// A pulse is imposed as authored by every consumer, so once it is over
    /// nothing is left driving. A closed reflecting cavity keeps in its
    /// constant mode whatever a source put there, which for a sine burst is
    /// nothing; an integrated pulse would instead leave its area behind as a
    /// steady rate, and the cavity would drift for as long as it ran.
    #[test]
    fn a_pulse_drives_as_authored_and_leaves_a_closed_cavity_still() {
        let envelope = PulseEnvelope::FlatTop {
            duration: 0.6,
            edge: 0.2,
        };
        let pulse = TimeSignal::pulsed([0.0, 2.0, 3.0, 0.0], envelope, 0.1, 0.0);
        assert!(CanonicalRateDrive::legacy(pulse, 0.0).is_err());
        assert_eq!(
            CanonicalRateDrive::authored(pulse, 0.0).unwrap(),
            CanonicalRateDrive::Direct(pulse)
        );
        assert!(matches!(
            CanonicalRateDrive::authored(TimeSignal::harmonic(0.0, 2.0, 3.0, 0.0), 0.0).unwrap(),
            CanonicalRateDrive::LegacyIntegratedHarmonic { .. }
        ));

        let (_, operator) = compile(&Scene::default());
        let elements = vec![true; operator.element_nodes().len()];
        let source = CanonicalForcing::point_source(
            &operator,
            PointSource {
                enabled: true,
                position: Point2::new(0.5, 0.5),
                width: 0.3,
                signal: pulse,
                ..PointSource::default()
            },
            &elements,
            0.0,
        )
        .unwrap();
        assert_eq!(source.drive(), CanonicalRateDrive::Direct(pulse));
        let mut forcing = CanonicalForcing::none(&operator);
        forcing.push_source(source).unwrap();
        let dt = 0.05 * operator.maximum_time_step();
        let total = |state: &CanonicalWaveState| state.primary_flux().iter().sum::<f64>();
        let mut state = CanonicalWaveState::zero(&operator, dt).unwrap();
        let mut peak = 0.0_f64;
        while state.time() < 0.8 {
            state.step_with_forcing(&operator, &forcing).unwrap();
            peak = peak.max(total(&state).abs());
        }
        let after = total(&state);
        while state.time() < 2.5 {
            state.step_with_forcing(&operator, &forcing).unwrap();
        }
        assert!(peak > 1.0e-3, "the pulse put in only {peak}");
        assert!(
            (total(&state) - after).abs() < 1.0e-12 * peak,
            "the cavity drifted from {after} to {}",
            total(&state)
        );
        // The burst itself has no area. What is left is the step's trapezoid
        // rule on it: 1.5e-5 of the peak here, and under an eighth of that
        // at each halving of the step.
        assert!(
            after.abs() < 1.0e-4 * peak,
            "the burst left {after} of {peak}"
        );
        assert!(
            forcing
                .integrated_rate(1.0)
                .unwrap()
                .iter()
                .all(|rate| *rate == 0.0)
        );

        // A pulsed Neumann side holds its flux as authored.
        let mesh = square_with_outer_boundary();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &Scene::default(),
            OuterBoundaryCondition::Neumann { signal: pulse },
        )
        .unwrap();
        let canonical =
            CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &Scene::default(), 17).unwrap();
        let walls = CanonicalForcing::from_legacy_boundaries(&canonical, &quadratic, 0.0).unwrap();
        assert!(!walls.sources().is_empty());
        assert!(
            walls
                .sources()
                .iter()
                .all(|source| source.drive() == CanonicalRateDrive::Direct(pulse))
        );

        // A pulsed Dirichlet node follows its signal, and rests at zero once
        // the pulse is over.
        let flash = TimeSignal::pulsed([0.3, 0.0, 1.0, 0.0], envelope, 0.1, 0.0);
        let mut prescribed = vec![None; operator.degrees_of_freedom()];
        prescribed[0] = Some(flash);
        let pinned = CanonicalForcing::from_prescribed(&operator, prescribed).unwrap();
        let mut state = CanonicalWaveState::zero(&operator, dt).unwrap();
        let mut largest = 0.0_f64;
        while state.time() < 1.0 {
            state.step_with_forcing(&operator, &pinned).unwrap();
            let field = state.primary_field(&operator).unwrap()[0];
            assert!((field - flash.value(state.time())).abs() < 1.0e-12);
            largest = largest.max(field);
        }
        assert!(largest > 0.29, "the pinned node reached only {largest}");
        assert_eq!(state.primary_field(&operator).unwrap()[0], 0.0);
    }

    #[test]
    fn point_source_support_is_structural_across_gaussian_underflow() {
        let (_, operator) = compile(&Scene::default());
        let elements = vec![true; operator.element_nodes().len()];
        let membership = vec![true; operator.degrees_of_freedom()];
        let first_point = operator.node_points()[0];
        let second_node = operator
            .node_points()
            .iter()
            .position(|point| *point != first_point)
            .unwrap();
        let source = |position| PointSource {
            enabled: true,
            position,
            width: 1.0e-12,
            ..PointSource::default()
        };
        let first =
            CanonicalForcing::point_source(&operator, source(first_point), &elements, 0.0).unwrap();
        let second = CanonicalForcing::point_source(
            &operator,
            source(operator.node_points()[second_node]),
            &elements,
            0.0,
        )
        .unwrap();

        assert_eq!(first.support(), membership);
        assert_eq!(second.support(), membership);
        assert_ne!(
            first
                .weights()
                .iter()
                .map(|weight| *weight != 0.0)
                .collect::<Vec<_>>(),
            second
                .weights()
                .iter()
                .map(|weight| *weight != 0.0)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn pulse_and_prescribed_primary_use_explicit_constitutive_exchange() {
        let (_, operator) = compile(&Scene::default());
        let dt = 0.1 * operator.maximum_time_step();
        let signal = TimeSignal::harmonic(0.4, 0.0, 1.0, 0.0);
        let forcing = CanonicalForcing::from_prescribed(
            &operator,
            vec![Some(signal); operator.degrees_of_freedom()],
        )
        .unwrap();
        let mut state = CanonicalWaveState::zero(&operator, dt).unwrap();
        let accounting = state
            .apply_primary_pulse(
                &operator,
                &forcing,
                &vec![0.9; operator.degrees_of_freedom()],
            )
            .unwrap();
        assert!(accounting.source_work > 0.0);
        assert!(accounting.prescribed_exchange < 0.0);
        assert!(
            state
                .primary_field(&operator)
                .unwrap()
                .iter()
                .all(|value| (*value - 0.4).abs() < 2.0e-15)
        );
    }

    #[test]
    fn paired_filter_reduces_energy_preserves_total_and_leaves_stationary_flux() {
        let (_, operator) = compile(&Scene::default());
        let dt = 0.1 * operator.maximum_time_step();
        let primary = test_potential(&operator);
        let potential = primary
            .iter()
            .enumerate()
            .map(|(index, value)| value + (7.0 * index as f64).sin())
            .collect::<Vec<_>>();
        let mut state =
            CanonicalWaveState::from_primary_and_potential(&operator, dt, &primary, &potential)
                .unwrap();
        let total_before = state.primary_flux().iter().sum::<f64>();
        let energy_before = state.energy(&operator).unwrap();
        let accounting = state
            .apply_grid_filter(&operator, &CanonicalForcing::none(&operator), 0.8)
            .unwrap();
        assert!(state.energy(&operator).unwrap() < energy_before);
        assert!(accounting.filter_removed > 0.0);
        assert!((state.primary_flux().iter().sum::<f64>() - total_before).abs() < 2.0e-12);

        let arbitrary = (0..operator.complementary_degrees_of_freedom())
            .map(|index| Point2::new((index as f64 + 0.3).sin(), (0.7 * index as f64).cos()))
            .collect::<Vec<_>>();
        let force = operator.force(&arbitrary).unwrap();
        let compatible = operator
            .compatible_flux(&operator.solve_stiffness(&force).unwrap())
            .unwrap();
        let stationary = arbitrary
            .iter()
            .zip(compatible)
            .map(|(all, image)| *all - image)
            .collect::<Vec<_>>();
        let mut stationary_state = CanonicalWaveState::new(
            &operator,
            dt,
            vec![0.0; operator.degrees_of_freedom()],
            stationary.clone(),
        )
        .unwrap();
        stationary_state
            .apply_grid_filter(&operator, &CanonicalForcing::none(&operator), 0.8)
            .unwrap();
        assert!(
            stationary_state
                .complementary_flux()
                .iter()
                .zip(stationary)
                .all(|(actual, expected)| (*actual - expected).norm() < 2.0e-11)
        );
    }

    /// With every node's reach at the global bound's `1/Λ`, the filter is the
    /// global paired polynomial it replaced: `α (K M⁻¹)² Q / Λ²` off `Q` and
    /// `α C M⁻¹ K M⁻¹ CᵀWJ b / Λ²` off `b`.
    #[test]
    fn a_uniform_reach_is_the_global_filter() {
        let (_, operator) = compile(&configured_scene(PhysicsModel::Mechanical));
        let bound = 4.0 / operator.maximum_time_step().powi(2);
        let count = operator.degrees_of_freedom();
        let operator = operator.with_grid_filter_reach(vec![1.0 / bound; count]);
        let dt = 0.1 * operator.maximum_time_step();
        let primary = test_potential(&operator);
        let complementary = (0..operator.complementary_degrees_of_freedom())
            .map(|index| Point2::new((index as f64 + 0.3).sin(), (0.7 * index as f64).cos()))
            .collect::<Vec<_>>();
        let mut state =
            CanonicalWaveState::new(&operator, dt, primary.clone(), complementary.clone()).unwrap();
        let strength = 0.8;
        state
            .apply_grid_filter(&operator, &CanonicalForcing::none(&operator), strength)
            .unwrap();

        let over_mass = |values: Vec<f64>| {
            values
                .iter()
                .zip(operator.primary_mass())
                .map(|(value, mass)| value / mass)
                .collect::<Vec<_>>()
        };
        let scale = strength / (bound * bound);
        let field = over_mass(primary.clone());
        let primary_image = operator
            .compatible_stiffness(&over_mass(operator.compatible_stiffness(&field).unwrap()))
            .unwrap();
        let complementary_image = operator
            .compatible_flux(&over_mass(
                operator
                    .compatible_stiffness(&over_mass(operator.force(&complementary).unwrap()))
                    .unwrap(),
            ))
            .unwrap();
        let primary_scale = primary.iter().fold(0.0_f64, |a, b| a.max(b.abs()));
        for ((actual, before), image) in
            state.primary_flux().iter().zip(&primary).zip(primary_image)
        {
            assert!((actual - (before - scale * image)).abs() <= 1e-12 * primary_scale);
        }
        for ((actual, before), image) in state
            .complementary_flux()
            .iter()
            .zip(&complementary)
            .zip(complementary_image)
        {
            assert!((*actual - (*before - image * scale)).norm() <= 1e-12);
        }
    }

    #[test]
    fn paired_filter_holds_prescribed_primary_nodes_and_accounts_exchange() {
        let (_, operator) = compile(&Scene::default());
        let dt = 0.1 * operator.maximum_time_step();
        let signal = TimeSignal::harmonic(0.37, 0.0, 1.0, 0.0);
        let mut prescribed = vec![None; operator.degrees_of_freedom()];
        prescribed[0] = Some(signal);
        let forcing = CanonicalForcing::from_prescribed(&operator, prescribed).unwrap();
        let mut primary = test_potential(&operator);
        primary[0] = signal.value(0.0);
        let potential = test_potential(&operator)
            .iter()
            .enumerate()
            .map(|(index, value)| value + (2.7 * index as f64).cos())
            .collect::<Vec<_>>();
        let mut state =
            CanonicalWaveState::from_primary_and_potential(&operator, dt, &primary, &potential)
                .unwrap();
        let accounting = state.apply_grid_filter(&operator, &forcing, 0.5).unwrap();
        assert_eq!(
            state.primary_flux()[0],
            operator.primary_mass()[0] * signal.value(0.0)
        );
        assert!(accounting.filter_removed >= 0.0);
        assert!(accounting.prescribed_exchange.is_finite());
    }

    #[test]
    fn first_order_outgoing_uses_the_force_coupled_midpoint_kick() {
        let mesh = square_with_outer_boundary();
        let scene = Scene::default();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::FirstOrderOutgoing,
        )
        .unwrap();
        let operator = CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 71).unwrap();
        assert!(
            operator
                .first_order_boundary_damping()
                .iter()
                .any(|value| *value > 0.0)
        );
        assert!(operator.outgoing_boundary().is_none());
        let dt = 0.2 * operator.maximum_time_step();
        let mut state = CanonicalWaveState::from_primary_and_potential(
            &operator,
            dt,
            &vec![1.0; operator.degrees_of_freedom()],
            &vec![0.0; operator.degrees_of_freedom()],
        )
        .unwrap();
        let before = state.energy(&operator).unwrap();
        let accounting = state
            .step_with_forcing(&operator, &CanonicalForcing::none(&operator))
            .unwrap();
        assert!(accounting.boundary_loss > 0.0);
        assert!(state.energy(&operator).unwrap() < before);
    }

    /// The mass-free part of the export is the same whichever way the state was
    /// built, so a driven generation can pack a state that skipped the dense
    /// inversion. The inverse itself is exported only by the state that paid
    /// for it, and it is the same solve the sweep converges to.
    #[test]
    fn a_backend_state_exports_what_a_stepping_state_exports() {
        let mesh = Arc::new(square_with_outer_boundary());
        let scene = Scene::default();
        let quadratic = Arc::new(
            QuadraticWaveOperator::assemble_scene(
                &mesh,
                &scene,
                OuterBoundaryCondition::SecondOrderOutgoing,
            )
            .unwrap(),
        );
        let operator = CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 72).unwrap();
        let step = operator.recommended_time_step();

        let stepping = CanonicalWaveState::zero(&operator, step).unwrap();
        let backend = CanonicalWaveState::zero_for_backend(&operator, step).unwrap();
        let stepping_factor = stepping.outgoing_midpoint_factor().unwrap();
        let backend_factor = backend.outgoing_midpoint_factor().unwrap();
        let mut stepping_export = stepping_factor.export().unwrap();
        let backend_export = backend_factor.export().unwrap();
        assert!(backend_export.direct_inverse.is_none());
        let inverse = stepping_export
            .direct_inverse
            .take()
            .expect("a fixed state exports its inverse");
        assert_eq!(
            stepping_export, backend_export,
            "the mass-free export must not depend on the inverted lane"
        );

        // Applied to a right-hand side, the exported inverse is the sweep's
        // answer - the whole of what a backend may substitute it for.
        let boundary = operator.outgoing_boundary().unwrap();
        let trace_count = boundary.trace_nodes.len();
        let reduced = (0..trace_count)
            .map(|trace| (0.37 * trace as f64).sin() + 0.2)
            .collect::<Vec<_>>();
        let swept = solve_outgoing_trace(
            boundary,
            &backend_export.diagonal,
            &backend_export.modal_correction,
            backend_export.sweeps,
            0.5 * backend_export.duration,
            operator.primary_mass(),
            &reduced,
            &[],
        )
        .unwrap();
        let scale = swept.iter().map(|value| value.abs()).fold(0.0, f64::max);
        for (row, expected) in swept.iter().enumerate() {
            let direct = inverse[row * trace_count..(row + 1) * trace_count]
                .iter()
                .zip(&reduced)
                .map(|(coefficient, value)| coefficient * value)
                .sum::<f64>();
            assert!(
                (direct - expected).abs() <= 1.0e-12 * scale,
                "row {row}: {direct} against {expected}"
            );
        }
        // A held trace row is outside what the unconstrained inverse solves.
        let mut held = vec![false; trace_count];
        held[0] = true;
        assert!(
            stepping_factor
                .export_with_prescribed(&held)
                .unwrap()
                .direct_inverse
                .is_none()
        );

        // And the one that skipped it really did skip it, rather than the two
        // having quietly converged on the same construction.
        assert!(
            backend_factor.estimated_bytes() <= stepping_factor.estimated_bytes(),
            "the backend factor carries no more than the stepping one"
        );

        // Stepping a backend state is correct; it takes the sweep instead.
        let forcing = CanonicalForcing::none(&operator);
        let mut swept = backend;
        let mut inverted = stepping;
        swept.step_with_forcing(&operator, &forcing).unwrap();
        inverted.step_with_forcing(&operator, &forcing).unwrap();
        for (left, right) in swept.primary_flux().iter().zip(inverted.primary_flux()) {
            assert!((left - right).abs() < 1.0e-12, "{left} against {right}");
        }
    }

    #[test]
    fn canonical_reassembly_retains_an_identical_outgoing_trace_system() {
        let mesh = Arc::new(square_with_outer_boundary());
        let scene = Scene::default();
        let quadratic = Arc::new(
            QuadraticWaveOperator::assemble_scene(
                &mesh,
                &scene,
                OuterBoundaryCondition::SecondOrderOutgoing,
            )
            .unwrap(),
        );
        let previous = CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 72).unwrap();
        let previous_boundary = previous.outgoing_boundary_handle().unwrap();
        let mut job = CanonicalAssemblyJob::new_with_outgoing_reuse(
            mesh.clone(),
            quadratic.clone(),
            TopologyWaveModel::from_scene(&scene),
            73,
            Some(previous_boundary.clone()),
        )
        .unwrap();
        let reassembled = loop {
            if let Some(result) = job.advance(1) {
                break result.unwrap();
            }
        };
        assert!(Arc::ptr_eq(
            &previous_boundary,
            &reassembled.outgoing_boundary_handle().unwrap()
        ));

        let reflecting = Arc::new(
            QuadraticWaveOperator::assemble_scene(
                &mesh,
                &scene,
                OuterBoundaryCondition::Reflecting,
            )
            .unwrap(),
        );
        let mut job = CanonicalAssemblyJob::new_with_outgoing_reuse(
            mesh,
            reflecting,
            TopologyWaveModel::from_scene(&scene),
            74,
            Some(previous_boundary),
        )
        .unwrap();
        let changed = loop {
            if let Some(result) = job.advance(1) {
                break result.unwrap();
            }
        };
        assert!(changed.outgoing_boundary().is_none());
    }

    #[test]
    fn invariant_maintenance_repairs_only_roundoff_sized_unaccounted_drift() {
        let (_, operator) = compile(&Scene::default());
        let forcing = CanonicalForcing::none(&operator);
        let dt = 0.1 * operator.maximum_time_step();
        let mut state = CanonicalWaveState::zero(&operator, dt).unwrap();
        let mut pulse = vec![0.0; operator.degrees_of_freedom()];
        pulse[0] = 1.0e-12 / operator.primary_mass()[0];
        state
            .apply_primary_pulse(&operator, &forcing, &pulse)
            .unwrap();
        state
            .maintain_component_totals(&operator, &forcing, &[0.0])
            .unwrap();
        assert!(state.primary_flux().iter().sum::<f64>().abs() < 1.0e-25);

        pulse[0] = 1.0e-4 / operator.primary_mass()[0];
        state
            .apply_primary_pulse(&operator, &forcing, &pulse)
            .unwrap();
        assert!(
            state
                .maintain_component_totals(&operator, &forcing, &[0.0])
                .is_err()
        );
    }

    /// The device count stops where f32 does. It has to be the fewer sweeps -
    /// otherwise it buys nothing - and still land on the dense solve to the
    /// precision the device holds, for every mass a driven medium produces.
    #[test]
    fn the_device_sweeps_stop_at_f32_precision() {
        let mesh = square_with_outer_boundary();
        let scene = Scene::default();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::SecondOrderOutgoing,
        )
        .unwrap();
        let operator = CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 73).unwrap();
        let boundary = operator.outgoing_boundary().unwrap();
        let dimension = boundary.trace_nodes().len() + boundary.auxiliary_count();
        let probe = (0..dimension)
            .map(|index| (0.17 * index as f64 + 0.3).sin())
            .collect::<Vec<_>>();
        let authored = operator.primary_mass().to_vec();
        let pumped = authored.iter().map(|value| 2.7 * value).collect::<Vec<_>>();
        let travelling = authored
            .iter()
            .enumerate()
            .map(|(node, value)| value * (1.0 + 0.4 * (0.9 * node as f64).sin()))
            .collect::<Vec<_>>();
        let precision = f64::from(f32::EPSILON);

        for fraction in [0.08, 0.5, 1.0] {
            let kick = 0.5 * fraction * operator.maximum_time_step();
            let cache =
                CanonicalOutgoingMidpointFactor::prepare(&operator, boundary, kick).unwrap();
            let device = cache.device_sweeps();
            assert!(device.is_multiple_of(2) && device <= cache.sweeps());
            assert_eq!(cache.export().unwrap().device_sweeps, device);
            // Two of the sweeps are spare, so the rest already meet the bound.
            assert!(cache.contraction().powi(device as i32 - 2) <= precision);
            let mut truncated = cache.clone();
            truncated.sweeps = device;
            for mass in [&authored, &pumped, &travelling] {
                let dense = outgoing_generator(&operator, boundary, mass).unwrap();
                let mut midpoint = vec![0.0; dimension * dimension];
                for row in 0..dimension {
                    for column in 0..dimension {
                        midpoint[row * dimension + column] =
                            f64::from(row == column) - 0.5 * kick * dense[row * dimension + column];
                    }
                }
                let oracle = solve_dense(midpoint, probe.clone(), dimension).unwrap();
                let magnitude = oracle.iter().map(|value| value.abs()).fold(0.0, f64::max);
                let swept = truncated.solve(boundary, mass, &probe).unwrap();
                let difference = maximum_difference(&swept, &oracle);
                assert!(
                    difference <= precision * magnitude,
                    "{device} sweeps missed the dense solve by {difference:e} of \
                     {magnitude:e} at {fraction} of the step bound"
                );
            }
        }
        // At the step bound the device count is strictly the smaller, or the
        // split would cost a field and save nothing.
        let cache = CanonicalOutgoingMidpointFactor::prepare(
            &operator,
            boundary,
            0.5 * operator.maximum_time_step(),
        )
        .unwrap();
        assert!(
            cache.device_sweeps() < cache.sweeps(),
            "{} device sweeps against {}",
            cache.device_sweeps(),
            cache.sweeps()
        );
    }

    /// One preparation, any mass. `prepare` no longer takes a mass at all, so
    /// the claim to test is that the sweep it runs instead still lands on the
    /// matrix the dense generator describes, for masses that differ from the
    /// authored one by a uniform pump and by an arbitrary non-uniform wobble -
    /// the two shapes a time-driven medium actually produces. It also records
    /// the contraction, because a bound that quietly drifted toward one would
    /// still pass an accuracy check while costing every sweep it could.
    #[test]
    fn one_outgoing_preparation_solves_every_nodal_mass() {
        let mesh = square_with_outer_boundary();
        let scene = Scene::default();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::SecondOrderOutgoing,
        )
        .unwrap();
        let operator = CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 73).unwrap();
        let boundary = operator.outgoing_boundary().unwrap();
        let dimension = boundary.trace_nodes().len() + boundary.auxiliary_count();
        let probe = (0..dimension)
            .map(|index| (0.17 * index as f64 + 0.3).sin())
            .collect::<Vec<_>>();

        let authored = operator.primary_mass().to_vec();
        let pumped = authored.iter().map(|value| 2.7 * value).collect::<Vec<_>>();
        let travelling = authored
            .iter()
            .enumerate()
            .map(|(node, value)| value * (1.0 + 0.4 * (0.9 * node as f64).sin()))
            .collect::<Vec<_>>();

        for fraction in [0.08, 0.5, 1.0] {
            let kick = 0.5 * fraction * operator.maximum_time_step();
            let cache =
                CanonicalOutgoingMidpointFactor::prepare(&operator, boundary, kick).unwrap();
            assert!(
                cache.contraction() < 0.05,
                "contraction {:e} at {fraction} of the step bound",
                cache.contraction()
            );
            assert!(cache.sweeps().is_multiple_of(2));
            for mass in [&authored, &pumped, &travelling] {
                let dense = outgoing_generator(&operator, boundary, mass).unwrap();
                let mut midpoint = vec![0.0; dimension * dimension];
                for row in 0..dimension {
                    for column in 0..dimension {
                        midpoint[row * dimension + column] =
                            f64::from(row == column) - 0.5 * kick * dense[row * dimension + column];
                    }
                }
                let oracle = solve_dense(midpoint, probe.clone(), dimension).unwrap();
                let swept = cache.solve(boundary, mass, &probe).unwrap();
                let difference = maximum_difference(&swept, &oracle);
                assert!(
                    difference < 2.0e-11,
                    "the swept solve missed the dense one by {difference:e} \
                     at {fraction} of the step bound"
                );
            }
        }

        // The recorded contraction is what the sweep count is derived from, so
        // it has to bound the decay the sweep actually achieves rather than
        // merely be small. Halving the passes must leave an error no larger
        // than the bound raised to the passes removed.
        let kick = 0.5 * operator.maximum_time_step();
        let cache = CanonicalOutgoingMidpointFactor::prepare(&operator, boundary, kick).unwrap();
        let dense = outgoing_generator(&operator, boundary, &travelling).unwrap();
        let mut midpoint = vec![0.0; dimension * dimension];
        for row in 0..dimension {
            for column in 0..dimension {
                midpoint[row * dimension + column] =
                    f64::from(row == column) - 0.5 * kick * dense[row * dimension + column];
            }
        }
        let oracle = solve_dense(midpoint, probe.clone(), dimension).unwrap();
        let magnitude = oracle.iter().map(|value| value.abs()).fold(0.0, f64::max);
        // From zero the error before any pass is the answer itself.
        let mut previous = magnitude;
        for passes in 1..=cache.sweeps() {
            let mut truncated = cache.clone();
            truncated.sweeps = passes;
            let swept = truncated.solve(boundary, &travelling, &probe).unwrap();
            let error = maximum_difference(&swept, &oracle);
            assert!(
                error <= previous * cache.contraction() * 4.0 + 1.0e-14 * magnitude,
                "pass {passes} left {error:e} against {previous:e}, a decay worse \
                 than the recorded bound {:e}",
                cache.contraction()
            );
            previous = error;
        }

        // A prescribed trace row owns its own value, which the sweep expresses
        // by holding that row rather than by rebuilding a constrained matrix.
        let mut prescribed_pattern = vec![false; boundary.trace_nodes().len()];
        prescribed_pattern[0] = true;
        let export = cache.export_with_prescribed(&prescribed_pattern).unwrap();
        let mut constrained = vec![0.0; dimension * dimension];
        for row in 0..dimension {
            for column in 0..dimension {
                constrained[row * dimension + column] =
                    f64::from(row == column) - 0.5 * kick * dense[row * dimension + column];
            }
        }
        constrained[0..dimension].fill(0.0);
        constrained[0] = 1.0;
        let oracle = solve_dense(constrained, probe.clone(), dimension).unwrap();
        let swept = export.solve(&travelling, boundary, &probe).unwrap();
        assert!(maximum_difference(&swept, &oracle) < 2.0e-11);
    }

    #[test]
    fn passive_second_order_trace_has_normalized_memory_and_contracts_long_runs() {
        let mesh = square_with_outer_boundary();
        let scene = Scene::default();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::SecondOrderOutgoing,
        )
        .unwrap();
        let operator = CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 73).unwrap();
        let boundary = operator.outgoing_boundary().unwrap();
        assert!(!boundary.trace_nodes().is_empty());
        assert_eq!(boundary.modes().len(), boundary.trace_nodes().len());
        assert!(boundary.auxiliary_count() > 0);
        assert!(boundary.modes().iter().any(|mode| mode.decay == 0.0));

        let dt = 0.08 * operator.maximum_time_step();
        let dimension = boundary.trace_nodes().len() + boundary.auxiliary_count();
        let probe = (0..dimension)
            .map(|index| (0.17 * index as f64 + 0.3).sin())
            .collect::<Vec<_>>();
        let dense = outgoing_generator(&operator, boundary, operator.primary_mass()).unwrap();
        let dense_product = (0..dimension)
            .map(|row| {
                (0..dimension)
                    .map(|column| dense[row * dimension + column] * probe[column])
                    .sum::<f64>()
            })
            .collect::<Vec<_>>();
        let matrix_free =
            apply_outgoing_generator(&operator, boundary, operator.primary_mass(), &probe).unwrap();
        assert!(maximum_difference(&dense_product, &matrix_free) < 2.0e-12);

        let kick = 0.5 * dt;
        let cache = CanonicalOutgoingMidpointFactor::prepare(&operator, boundary, kick).unwrap();
        let mut midpoint_matrix = vec![0.0; dimension * dimension];
        for row in 0..dimension {
            for column in 0..dimension {
                midpoint_matrix[row * dimension + column] =
                    f64::from(row == column) - 0.5 * kick * dense[row * dimension + column];
            }
        }
        let cached = cache
            .solve(boundary, operator.primary_mass(), &probe)
            .unwrap();
        let oracle = solve_dense(midpoint_matrix, probe.clone(), dimension).unwrap();
        assert!(maximum_difference(&cached, &oracle) < 2.0e-11);

        let prescribed_position = 0;
        let mut constrained_matrix = vec![0.0; dimension * dimension];
        for row in 0..dimension {
            for column in 0..dimension {
                constrained_matrix[row * dimension + column] =
                    f64::from(row == column) - 0.5 * kick * dense[row * dimension + column];
            }
        }
        constrained_matrix[prescribed_position * dimension..(prescribed_position + 1) * dimension]
            .fill(0.0);
        constrained_matrix[prescribed_position * dimension + prescribed_position] = 1.0;
        let constrained_oracle = solve_dense(constrained_matrix, probe.clone(), dimension).unwrap();
        let mut prescribed_pattern = vec![false; boundary.trace_nodes().len()];
        prescribed_pattern[prescribed_position] = true;
        let constrained_export = cache
            .export_with_prescribed(&prescribed_pattern)
            .unwrap()
            .solve(operator.primary_mass(), boundary, &probe)
            .unwrap();
        assert!(maximum_difference(&constrained_export, &constrained_oracle) < 2.0e-11);

        let prescribed_node = boundary.trace_nodes()[0] as usize;
        let prescribed_signal = TimeSignal::harmonic(0.23, 0.0, 1.0, 0.0);
        let mut prescribed = vec![None; operator.degrees_of_freedom()];
        prescribed[prescribed_node] = Some(prescribed_signal);
        let forcing = CanonicalForcing::from_prescribed(&operator, prescribed).unwrap();
        let mut constrained = CanonicalWaveState::zero(&operator, dt).unwrap();
        let accounting = constrained.step_with_forcing(&operator, &forcing).unwrap();
        assert_eq!(
            constrained.primary_flux()[prescribed_node],
            operator.primary_mass()[prescribed_node] * prescribed_signal.value(dt)
        );
        assert!(accounting.prescribed_exchange.is_finite());

        let primary = operator
            .node_points()
            .iter()
            .map(|point| (2.3 * point.x).cos() * (1.7 * point.y).sin())
            .collect::<Vec<_>>();
        let mut state = CanonicalWaveState::from_primary_and_potential(
            &operator,
            dt,
            &primary,
            &vec![0.0; operator.degrees_of_freedom()],
        )
        .unwrap();
        let initial = state.energy(&operator).unwrap();
        let mut removed = 0.0;
        for _ in 0..400 {
            let accounting = state
                .step_with_forcing(&operator, &CanonicalForcing::none(&operator))
                .unwrap();
            removed += accounting.boundary_loss;
        }
        assert!(removed > 0.0);
        assert!(state.energy(&operator).unwrap() < 0.2 * initial);
        let CanonicalAuxiliaryState::Linear(auxiliaries) = state.auxiliaries() else {
            panic!("second-order boundary did not allocate physical memory");
        };
        assert_eq!(auxiliaries.outgoing_z().len(), boundary.auxiliary_count());

        let normalized = (0..boundary.auxiliary_count())
            .map(|index| (0.37 * index as f64 + 0.2).sin())
            .collect::<Vec<_>>();
        let physical = boundary.physical_memory(&normalized).unwrap();
        let mut changed_basis = boundary.clone();
        changed_basis.modes.reverse();
        let mut next = 0;
        for (index, mode) in changed_basis.modes.iter_mut().enumerate() {
            if index.is_multiple_of(2) {
                for value in &mut mode.trace {
                    *value = -*value;
                }
            }
            if mode.auxiliary_offset.is_some() {
                mode.auxiliary_offset = Some(next);
                next += 3;
            }
        }
        changed_basis.auxiliary_count = next;
        let (changed_state, projection) = changed_basis.project_physical_memory(&physical).unwrap();
        let represented = changed_basis.physical_memory(&changed_state).unwrap();
        assert!(projection.physical_residual_norm < 2.0e-10);
        let difference = represented
            .pole_currents
            .iter()
            .zip(&physical.pole_currents)
            .flat_map(|(left, right)| left.iter().zip(right))
            .map(|(left, right)| (left - right).abs())
            .fold(0.0, f64::max);
        assert!(
            difference < 2.0e-10,
            "basis-dependent history {difference:e}"
        );

        let mut rotated_basis = boundary.clone();
        let pair = (0..rotated_basis.modes.len())
            .flat_map(|left| (left + 1..rotated_basis.modes.len()).map(move |right| (left, right)))
            .find(|(left, right)| {
                rotated_basis.modes[*left].auxiliary_offset.is_some()
                    && rotated_basis.modes[*right].auxiliary_offset.is_some()
                    && (rotated_basis.modes[*left].decay - rotated_basis.modes[*right].decay).abs()
                        < 2.0e-11 * rotated_basis.modes[*left].decay.max(1.0)
            })
            .expect("the symmetric square trace should contain a degenerate active eigenspace");
        let angle = 0.37_f64;
        let (sine, cosine) = angle.sin_cos();
        let left = rotated_basis.modes[pair.0].trace.clone();
        let right = rotated_basis.modes[pair.1].trace.clone();
        for node in 0..left.len() {
            rotated_basis.modes[pair.0].trace[node] = cosine * left[node] - sine * right[node];
            rotated_basis.modes[pair.1].trace[node] = sine * left[node] + cosine * right[node];
        }
        let (rotated_state, projection) = rotated_basis.project_physical_memory(&physical).unwrap();
        let represented = rotated_basis.physical_memory(&rotated_state).unwrap();
        let difference = represented
            .pole_currents
            .iter()
            .zip(&physical.pole_currents)
            .flat_map(|(left, right)| left.iter().zip(right))
            .map(|(left, right)| (left - right).abs())
            .fold(0.0, f64::max);
        assert!(projection.physical_residual_norm < 2.0e-10);
        assert!(
            difference < 2.0e-10,
            "degenerate rotation changed physical history {difference:e}"
        );
    }

    #[test]
    fn constant_primary_field_is_exactly_stationary() {
        let (_, operator) = compile(&Scene::default());
        let dt = 0.4 * operator.maximum_time_step();
        let primary = vec![0.7; operator.degrees_of_freedom()];
        let mut state = CanonicalWaveState::from_primary_and_potential(
            &operator,
            dt,
            &primary,
            &vec![0.0; operator.degrees_of_freedom()],
        )
        .unwrap();
        let initial_flux = state.primary_flux().to_vec();
        for _ in 0..500 {
            state.step(&operator).unwrap();
        }
        assert_eq!(state.primary_flux(), initial_flux);
        assert!(
            state
                .complementary_flux()
                .iter()
                .all(|flux| *flux == Point2::default())
        );
        assert!(maximum_difference(&state.primary_field(&operator).unwrap(), &primary) < 2.0e-15);
    }

    #[test]
    fn compatible_velocity_initializer_solves_the_scoped_scalar_condition() {
        let (_, operator) = compile(&configured_scene(PhysicsModel::Mechanical));
        let primary = operator
            .node_points()
            .iter()
            .map(|point| 0.4 + point.y)
            .collect::<Vec<_>>();
        let mut velocity = operator
            .node_points()
            .iter()
            .map(|point| point.x - 0.3 * point.y)
            .collect::<Vec<_>>();
        let weighted_mean = velocity
            .iter()
            .zip(operator.primary_mass())
            .map(|(value, mass)| value * mass)
            .sum::<f64>()
            / operator.primary_mass().iter().sum::<f64>();
        for value in &mut velocity {
            *value -= weighted_mean;
        }
        let state = CanonicalWaveState::from_primary_velocity(
            &operator,
            0.2 * operator.maximum_time_step(),
            &primary,
            &velocity,
        )
        .unwrap();
        let force = operator.force(state.complementary_flux()).unwrap();
        let expected = velocity
            .iter()
            .zip(operator.primary_mass())
            .map(|(velocity, mass)| -mass * velocity)
            .collect::<Vec<_>>();
        assert!(maximum_difference(&force, &expected) < 2.0e-11);

        assert!(
            CanonicalWaveState::from_primary_velocity(
                &operator,
                0.2 * operator.maximum_time_step(),
                &primary,
                &vec![1.0; operator.degrees_of_freedom()],
            )
            .is_err()
        );
    }

    #[test]
    fn velocity_compatibility_is_enforced_per_free_component() {
        let mesh = disconnected_triangles();
        let scene = Scene::default();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let operator = CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 53).unwrap();
        assert_eq!(operator.component_count(), 2);
        let primary = vec![0.0; operator.degrees_of_freedom()];
        let globally_balanced = operator
            .component_labels()
            .iter()
            .map(|component| if *component == 0 { 1.0 } else { -1.0 })
            .collect::<Vec<_>>();
        assert!(
            globally_balanced
                .iter()
                .zip(operator.primary_mass())
                .map(|(velocity, mass)| velocity * mass)
                .sum::<f64>()
                .abs()
                < 2.0e-15
        );
        assert!(
            CanonicalWaveState::from_primary_velocity(
                &operator,
                0.2 * operator.maximum_time_step(),
                &primary,
                &globally_balanced,
            )
            .is_err(),
            "opposite component means must not cancel globally"
        );

        let mut compatible = operator
            .node_points()
            .iter()
            .map(|point| point.x)
            .collect::<Vec<_>>();
        for component in 0..operator.component_count() {
            let total_mass = operator
                .primary_mass()
                .iter()
                .zip(operator.component_labels())
                .filter(|(_, label)| **label as usize == component)
                .map(|(mass, _)| mass)
                .sum::<f64>();
            let mean = compatible
                .iter()
                .zip(operator.primary_mass())
                .zip(operator.component_labels())
                .filter(|(_, label)| **label as usize == component)
                .map(|((velocity, mass), _)| velocity * mass)
                .sum::<f64>()
                / total_mass;
            for (velocity, label) in compatible.iter_mut().zip(operator.component_labels()) {
                if *label as usize == component {
                    *velocity -= mean;
                }
            }
        }
        CanonicalWaveState::from_primary_velocity(
            &operator,
            0.2 * operator.maximum_time_step(),
            &primary,
            &compatible,
        )
        .unwrap();
    }

    #[test]
    fn direct_state_retains_a_nonpotential_stationary_flux() {
        let (_, operator) = compile(&Scene::default());
        let arbitrary = (0..operator.complementary_degrees_of_freedom())
            .map(|index| Point2::new((index as f64 + 0.3).sin(), (0.7 * index as f64).cos()))
            .collect::<Vec<_>>();
        let arbitrary_force = operator.force(&arbitrary).unwrap();
        let potential = operator.solve_stiffness(&arbitrary_force).unwrap();
        let compatible = operator.compatible_flux(&potential).unwrap();
        let stationary = arbitrary
            .iter()
            .zip(compatible)
            .map(|(arbitrary, compatible)| *arbitrary - compatible)
            .collect::<Vec<_>>();
        let force = operator.force(&stationary).unwrap();
        assert!(force.iter().map(|value| value.abs()).fold(0.0, f64::max) < 3.0e-11);
        assert!(
            stationary
                .iter()
                .map(|value| value.norm())
                .fold(0.0, f64::max)
                > 0.1
        );
        let dt = 0.2 * operator.maximum_time_step();
        let mut state = CanonicalWaveState::new(
            &operator,
            dt,
            vec![0.0; operator.degrees_of_freedom()],
            stationary.clone(),
        )
        .unwrap();
        for _ in 0..100 {
            state.step(&operator).unwrap();
        }
        assert!(
            state
                .primary_flux()
                .iter()
                .map(|value| value.abs())
                .fold(0.0, f64::max)
                < 2.0e-8
        );
        assert!(
            state
                .complementary_flux()
                .iter()
                .zip(stationary)
                .map(|(actual, expected)| (*actual - expected).norm())
                .fold(0.0, f64::max)
                < 2.0e-8
        );
    }

    #[test]
    fn direct_kdk_matches_the_existing_scalar_recurrence_on_compatible_data() {
        let (quadratic, operator) = compile(&configured_scene(PhysicsModel::Mechanical));
        let dt = 0.2 * operator.maximum_time_step();
        let primary = operator
            .node_points()
            .iter()
            .map(|point| (2.0 * point.x).sin() + 0.3 * point.y)
            .collect::<Vec<_>>();
        let velocity = vec![0.0; operator.degrees_of_freedom()];
        // The scalar recurrence is the leapfrog's.
        let mut direct =
            CanonicalWaveState::from_primary_velocity(&operator, dt, &primary, &velocity)
                .unwrap()
                .with_integrator(CanonicalIntegrator::Leapfrog);
        let mut scalar =
            QuadraticWaveState::new(&quadratic, dt, primary.clone(), velocity).unwrap();
        for _ in 0..80 {
            direct.step(&operator).unwrap();
            scalar.step(&quadratic, &[]).unwrap();
            assert!(
                maximum_difference(&direct.primary_field(&operator).unwrap(), scalar.current())
                    < 2.0e-10
            );
        }
    }

    #[test]
    fn conservative_step_has_second_order_endpoint_error_and_energy_defect() {
        let (_, operator) = compile(&configured_scene(PhysicsModel::Mechanical));
        let initial = operator
            .node_points()
            .iter()
            .map(|point| (1.7 * point.x).cos() - 0.4 * point.y)
            .collect::<Vec<_>>();
        let total_time = 4.0 * operator.maximum_time_step();
        let run = |steps: usize| {
            let dt = total_time / steps as f64;
            let mut state = CanonicalWaveState::from_primary_velocity(
                &operator,
                dt,
                &initial,
                &vec![0.0; operator.degrees_of_freedom()],
            )
            .unwrap();
            let initial_energy = state.energy(&operator).unwrap();
            for _ in 0..steps {
                state.step(&operator).unwrap();
            }
            (
                state.primary_field(&operator).unwrap(),
                (state.energy(&operator).unwrap() - initial_energy).abs(),
            )
        };
        let (coarse, coarse_energy) = run(24);
        let (medium, medium_energy) = run(48);
        let (fine, fine_energy) = run(96);
        let (reference, _) = run(768);
        let error = |values: &[f64]| maximum_difference(values, &reference);
        assert!(error(&coarse) / error(&medium) > 3.5);
        assert!(error(&medium) / error(&fine) > 3.5);
        assert!(coarse_energy / medium_energy > 3.5);
        assert!(medium_energy / fine_energy > 3.5);
    }

    /// The default scene's box meshed at `edge` with `sides`, assembled and
    /// compiled. The mesh does not read the sides, so two boxes that differ
    /// only in theirs share every node.
    fn walled_box(
        sides: crate::OuterBoundaryConditions,
        edge: f64,
    ) -> (TriMesh, QuadraticWaveOperator, CanonicalWaveOperator) {
        let scene = Scene::default();
        let mesh = crate::mesh_scene(
            &scene,
            1,
            crate::MeshingOptions {
                target_edge_length: edge,
                ..crate::MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic =
            QuadraticWaveOperator::assemble_scene_with_boundaries(&mesh, &scene, sides).unwrap();
        let canonical = CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();
        (mesh, quadratic, canonical)
    }

    /// A second-order wall on the right side of the box, between two sides
    /// pinned to zero, ends its trace at the pinned corners: its tangential
    /// operator has Dirichlet ends, so its modes are the sines of the side,
    /// `λ_n = (nπ/H)²/2` at unit speed and impedance, with no constant mode
    /// and poles on every mode. Between free sides the same wall keeps the
    /// corners and the cosines, the constant mode included; between driven
    /// sides it keeps the corners too, as held rows.
    #[test]
    fn a_zero_pinned_side_ends_the_outgoing_trace_and_a_driven_one_does_not() {
        let mut sides = crate::OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting);
        sides.sides[crate::OuterSide::Right.index()] = OuterBoundaryCondition::SecondOrderOutgoing;
        let (mesh, quadratic, free) = walled_box(sides, 0.1);
        let zero = OuterBoundaryCondition::Dirichlet {
            signal: TimeSignal::ZERO,
        };
        sides.sides[crate::OuterSide::Top.index()] = zero;
        sides.sides[crate::OuterSide::Bottom.index()] = zero;
        let (pinned_mesh, pinned_quadratic, pinned) = walled_box(sides, 0.1);
        assert_eq!(pinned_mesh.vertices, mesh.vertices);
        let free_boundary = free.outgoing_boundary().unwrap();
        let pinned_boundary = pinned.outgoing_boundary().unwrap();

        // The right side's nodes, less the two the pinned sides share with it.
        let side_nodes = quadratic
            .second_order_boundary_damping()
            .iter()
            .filter(|damping| **damping > 0.0)
            .count();
        assert_eq!(free_boundary.trace_nodes().len(), side_nodes);
        let corners = free_boundary
            .trace_nodes()
            .iter()
            .copied()
            .filter(|node| pinned_quadratic.dirichlet_signals()[*node as usize].is_some())
            .collect::<Vec<_>>();
        assert_eq!(corners.len(), 2);
        for node in &corners {
            let point = pinned.node_points()[*node as usize];
            assert!(point.x > 0.999 && point.y.abs() > 0.999, "{point:?}");
        }
        let expected = free_boundary
            .trace_nodes()
            .iter()
            .copied()
            .filter(|node| !corners.contains(node))
            .collect::<Vec<_>>();
        assert_eq!(pinned_boundary.trace_nodes(), expected.as_slice());

        assert!(
            free_boundary
                .modes()
                .iter()
                .any(|mode| mode.eigenvalue == 0.0)
        );
        assert!(
            pinned_boundary
                .modes()
                .iter()
                .all(|mode| mode.eigenvalue > 0.0)
        );
        assert_eq!(
            pinned_boundary.auxiliary_count(),
            3 * pinned_boundary.trace_nodes().len()
        );
        let spectrum = |boundary: &CanonicalOutgoingBoundary| {
            let mut eigenvalues = boundary
                .modes()
                .iter()
                .map(|mode| mode.eigenvalue)
                .collect::<Vec<_>>();
            eigenvalues.sort_by(f64::total_cmp);
            eigenvalues
        };
        let height = mesh
            .vertices
            .iter()
            .map(|v| v.point.y)
            .fold(f64::MIN, f64::max)
            - mesh
                .vertices
                .iter()
                .map(|v| v.point.y)
                .fold(f64::MAX, f64::min);
        let exact = |n: usize| (n as f64 * std::f64::consts::PI / height).powi(2) / 2.0;
        let (free_spectrum, pinned_spectrum) = (spectrum(free_boundary), spectrum(pinned_boundary));
        for n in 1..=3 {
            eprintln!(
                "n {n}: sine {:.5} cosine {:.5} exact {:.5} -> {:.2e} {:.2e}",
                pinned_spectrum[n - 1],
                free_spectrum[n],
                exact(n),
                pinned_spectrum[n - 1] / exact(n) - 1.0,
                free_spectrum[n] / exact(n) - 1.0
            );
            assert!((pinned_spectrum[n - 1] / exact(n) - 1.0).abs() < 1.0e-4);
            assert!((free_spectrum[n] / exact(n) - 1.0).abs() < 1.0e-4);
        }

        // The pinned wall runs: the lowest sine mode along it, stepped with
        // its pins, radiates. Every stage keeps its balance or the step fails;
        // the field's own energy wobbles at the step's order and the memory
        // gives some back, so only the whole run is held to a loss.
        let forcing =
            CanonicalForcing::from_legacy_boundaries(&pinned, &pinned_quadratic, 0.0).unwrap();
        let primary = pinned
            .node_points()
            .iter()
            .zip(pinned.primary_mass())
            .map(|(point, mass)| {
                mass * (std::f64::consts::PI * (point.y + 1.0) / height).sin()
                    * (0.5 * std::f64::consts::PI * (point.x + 1.0)).sin()
            })
            .collect::<Vec<_>>();
        let dt = 0.5 * pinned.maximum_time_step();
        let mut state = CanonicalWaveState::new(
            &pinned,
            dt,
            primary,
            vec![Point2::default(); pinned.constitutive_samples().len()],
        )
        .unwrap();
        let start = state.energy(&pinned).unwrap();
        for _ in 0..400 {
            state.step_with_forcing(&pinned, &forcing).unwrap();
            assert!(state.energy(&pinned).unwrap() < start * (1.0 + 1.0e-6));
        }
        let energy = state.energy(&pinned).unwrap();
        eprintln!(
            "energy {start:.4} -> {energy:.4} over {:.3} s",
            state.time()
        );
        assert!(energy < 0.9 * start);

        let driven = OuterBoundaryCondition::Dirichlet {
            signal: TimeSignal::harmonic(0.2, 0.1, 1.0, 0.0),
        };
        sides.sides[crate::OuterSide::Top.index()] = driven;
        sides.sides[crate::OuterSide::Bottom.index()] = driven;
        let (_, _, held) = walled_box(sides, 0.1);
        assert_eq!(
            held.outgoing_boundary().unwrap().trace_nodes(),
            free_boundary.trace_nodes()
        );
    }
}
