//! f64 reference for time-driven and field-dependent media.
//!
//! The fixed linear [`CanonicalWaveOperator`] stays the base. This wrapper
//! compiles exact material-frame law samples beside it and evaluates them at
//! synchronized stage times. Time-driven, field-linear generations run on the
//! device from this contract (Stage 7). Field-dependent ones (Kerr and
//! saturable, Stage 8) run here only: each stage inverts the assembled nodal
//! map and the radial quadrature map with a bracketed solve, and the device
//! refuses them until Stage 9 ports that solve.

use std::collections::BTreeSet;

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{
    CanonicalAreaContribution, CanonicalAreaSample, CanonicalForcing, CanonicalIndicatorSnapshot,
    CanonicalIndicatorSupplement, CanonicalPointSample, CanonicalPointStencil,
    CanonicalWaveOperator, CoefficientLaw, CoefficientLawValues, ConstitutiveInverseError,
    ConstitutiveSite, ConstitutiveTerm, DampingLaw, DampingLawValues, ElectromagneticPolarization,
    FieldLawValues, LossChannel, Material, MaterialCoordinates, MaterialError, MaterialId,
    MaterialSwitchRuntime, PhysicsModel, Point2, QuadraticAreaElement, QuadraticAreaStencil,
    QuadraticPointStencil, QuadraticWaveOperator, RateLaw, RateLawValues, RegionId, RestoringLaw,
    RestoringLawValues, Scene, SymmetricTensor2, TimeDriveRuntime, TimeDriveValues,
    TopologyWaveModel, TriMesh, WaveError, canonical_area_contribution,
    complementary_interpolation_weights,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanonicalMaterialDrive {
    MassCoefficient,
    StiffnessCoefficient,
    ElectricLoss,
    MagneticLoss,
}

impl CanonicalMaterialDrive {
    /// Independently driven rows per material: the two constitutive
    /// coefficients and the two named loss channels.
    pub const COUNT: usize = 4;

    const fn index(self) -> usize {
        match self {
            Self::MassCoefficient => 0,
            Self::StiffnessCoefficient => 1,
            Self::ElectricLoss => 2,
            Self::MagneticLoss => 3,
        }
    }
}

/// Runtime state whose stable owner is an authored material ID, never an
/// editor array position or a mesh-local region number.
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalMaterialRuntimeRecord {
    material: MaterialId,
    material_name: String,
    drives: [TimeDriveRuntime; CanonicalMaterialDrive::COUNT],
    switch: MaterialSwitchRuntime,
}

impl CanonicalMaterialRuntimeRecord {
    fn authored(material: &Material) -> Result<Self, MaterialError> {
        let none = TimeDriveValues::None;
        let values = [
            material.mass_law.drive.evaluate(&material.parameters)?,
            material
                .stiffness_law
                .drive
                .evaluate(&material.parameters)?,
            material.electric_loss.as_ref().map_or(Ok(none), |loss| {
                loss.law.drive.evaluate(&material.parameters)
            })?,
            material.magnetic_loss.as_ref().map_or(Ok(none), |loss| {
                loss.law.drive.evaluate(&material.parameters)
            })?,
        ];
        let drives = values.map(TimeDriveRuntime::authored);
        let [mass, stiffness, electric_loss, magnetic_loss] = drives;
        Ok(Self {
            material: material.id,
            material_name: material.name.clone(),
            drives: [mass?, stiffness?, electric_loss?, magnetic_loss?],
            switch: MaterialSwitchRuntime::default(),
        })
    }

    pub fn material(&self) -> MaterialId {
        self.material
    }

    pub fn material_name(&self) -> &str {
        &self.material_name
    }

    pub fn switch(&self) -> MaterialSwitchRuntime {
        self.switch
    }

    pub fn drive(&self, drive: CanonicalMaterialDrive) -> TimeDriveRuntime {
        self.drives[drive.index()]
    }
}

/// CPU representation of the material-runtime table that later occupies the
/// reserved accepted/candidate GPU runtime slot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CanonicalMaterialRuntimeState {
    records: Vec<CanonicalMaterialRuntimeRecord>,
}

impl CanonicalMaterialRuntimeState {
    pub(crate) fn authored(
        materials: impl IntoIterator<Item = Material>,
    ) -> Result<Self, WaveError> {
        let mut records = materials
            .into_iter()
            .map(|material| {
                CanonicalMaterialRuntimeRecord::authored(&material).map_err(|error| {
                    WaveError::MaterialEvaluation {
                        material: material.name,
                        coefficient: "time drive",
                        point: Point2::default(),
                        reason: error.to_string(),
                    }
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        records.sort_by_key(|record| record.material);
        if records
            .windows(2)
            .any(|pair| pair[0].material == pair[1].material)
        {
            return Err(WaveError::Unsupported(
                "one material holds two runtime records",
            ));
        }
        Ok(Self { records })
    }

    pub fn records(&self) -> &[CanonicalMaterialRuntimeRecord] {
        &self.records
    }

    /// Adopts one material's accepted anchors and Switch trajectory as read
    /// back from the solver.
    ///
    /// The material set, its IDs and its names stay as the operator compiled
    /// them; only the runtime values move. They have to come from the solver
    /// rather than be recomputed here, because a Switch is stamped at its
    /// actual GPU commit boundary and a frequency edit re-anchors a carrier
    /// at one, neither of which the host can reconstruct from the clock.
    pub fn adopt(
        &mut self,
        material: MaterialId,
        drives: [TimeDriveRuntime; CanonicalMaterialDrive::COUNT],
        switch: MaterialSwitchRuntime,
    ) -> Result<(), WaveError> {
        let record = self.record_mut(material)?;
        record.drives = drives;
        record.switch = switch;
        Ok(())
    }

    pub fn begin_switch(
        &mut self,
        material: MaterialId,
        switched: bool,
        commit_time: f64,
        duration: f64,
    ) -> Result<(), WaveError> {
        let record = self.record_mut(material)?;
        record.switch = record
            .switch
            .begin(switched, commit_time, duration)
            .map_err(|_| WaveError::InvalidState)?;
        Ok(())
    }

    /// Reanchors a frequency edit. Explicit phase edits instead replace this
    /// lane with `TimeDriveRuntime::authored(new_drive)`.
    pub fn preserve_carrier(
        &mut self,
        material: MaterialId,
        drive: CanonicalMaterialDrive,
        old_drive: TimeDriveValues,
        commit_time: f64,
    ) -> Result<(), WaveError> {
        let record = self.record_mut(material)?;
        record.drives[drive.index()] = TimeDriveRuntime::preserving_carrier_from(
            old_drive,
            record.drives[drive.index()],
            commit_time,
        )
        .map_err(|_| WaveError::InvalidState)?;
        Ok(())
    }

    pub fn reset_phase(
        &mut self,
        material: MaterialId,
        drive: CanonicalMaterialDrive,
        new_drive: TimeDriveValues,
    ) -> Result<(), WaveError> {
        let record = self.record_mut(material)?;
        record.drives[drive.index()] =
            TimeDriveRuntime::authored(new_drive).map_err(|_| WaveError::InvalidState)?;
        Ok(())
    }

    /// The instantaneous factor one authored coefficient law reaches under
    /// this runtime.
    ///
    /// The operator's own samples carry their law already compiled, so they go
    /// through the internal path. This is for a consumer that samples
    /// materials at its own points instead of the operator's - the scalar
    /// estimator does, at element vertices, interior quadrature and edge
    /// quadrature - and so holds the law rather than a compiled sample.
    pub fn coefficient_law_factor(
        &self,
        material: MaterialId,
        drive: CanonicalMaterialDrive,
        law: CoefficientLawValues,
        coordinates: MaterialCoordinates,
        time: f64,
    ) -> Result<f64, MaterialError> {
        let record = self
            .records
            .binary_search_by_key(&material, |record| record.material)
            .map(|index| &self.records[index])
            .map_err(|_| MaterialError::InvalidValue)?;
        law.temporal_factor(time, coordinates, record.drive(drive), record.switch)
    }

    fn record(&self, material: MaterialId) -> Result<&CanonicalMaterialRuntimeRecord, WaveError> {
        self.records
            .binary_search_by_key(&material, |record| record.material)
            .ok()
            .map(|index| &self.records[index])
            .ok_or(WaveError::InvalidState)
    }

    fn record_mut(
        &mut self,
        material: MaterialId,
    ) -> Result<&mut CanonicalMaterialRuntimeRecord, WaveError> {
        self.records
            .binary_search_by_key(&material, |record| record.material)
            .ok()
            .map(|index| &mut self.records[index])
            .ok_or(WaveError::InvalidState)
    }
}

/// Probe and area consumers interpolate linear maps. On a field-dependent
/// medium they would read the wrong field without a word, so they refuse
/// until they are ported onto the solver-site inverses.
const CONSUMER_FIELD_LAWS: &str =
    "probes and area readouts are not yet ported to field-dependent media";

#[derive(Clone, Copy, Debug, PartialEq)]
struct TemporalCoefficientSample {
    material: MaterialId,
    point: Point2,
    coordinates: MaterialCoordinates,
    drive: CanonicalMaterialDrive,
    law: CoefficientLawValues,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TemporalLossSample {
    material: MaterialId,
    point: Point2,
    coordinates: MaterialCoordinates,
    drive: Option<CanonicalMaterialDrive>,
    base_rate: f64,
    law: DampingLawValues,
}

impl TemporalLossSample {
    fn zero(material: MaterialId, point: Point2, coordinates: MaterialCoordinates) -> Self {
        Self {
            material,
            point,
            coordinates,
            drive: None,
            base_rate: 0.0,
            law: DampingLawValues {
                rate: RateLawValues::Constant,
                drive: TimeDriveValues::None,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TemporalPrimarySample {
    coefficient: TemporalCoefficientSample,
    loss: TemporalLossSample,
    /// The restoring law on the integrated field at this contribution's
    /// point. Gate O: it acts on `r = ∫u dt` with the authored mass.
    restoring: RestoringLawValues,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TemporalComplementarySample {
    coefficient: TemporalCoefficientSample,
    loss: TemporalLossSample,
}

/// GPU-facing, field-linear coefficient metadata at one compiled physical
/// sample. Runtime phase/Switch ownership remains material-wide and is exposed
/// separately through [`CanonicalMaterialRuntimeState`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalTemporalCoefficientSample {
    pub material: MaterialId,
    pub coordinates: MaterialCoordinates,
    pub drive: CanonicalMaterialDrive,
    pub law: CoefficientLawValues,
}

impl CanonicalTemporalCoefficientSample {
    pub fn factor_at(
        self,
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<f64, WaveError> {
        if !time.is_finite() {
            return Err(WaveError::InvalidState);
        }
        let record = runtime.record(self.material)?;
        self.law
            .temporal_factor(
                time,
                self.coordinates,
                record.drive(self.drive),
                record.switch(),
            )
            .map_err(|_| WaveError::InvalidState)
    }
}

/// Synchronized endpoint reconstruction for a point consumer in a driven
/// field-linear material. Nodal primary masses use the complete assembled map
/// (including junction contributions), while the local energy and
/// complementary observable use the owning element's law at the actual probe
/// point rather than borrowing one quadrature sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalTemporalPointStencil {
    fixed: CanonicalPointStencil,
    primary: CanonicalTemporalCoefficientSample,
    /// The law at each of the element's own samples, where the constitutive
    /// inverse is applied before the physical field is interpolated.
    sample_coefficients: [CanonicalTemporalCoefficientSample; 6],
    /// The same law at the probe point, for the pointwise energy density.
    complementary: CanonicalTemporalCoefficientSample,
}

impl CanonicalTemporalPointStencil {
    pub fn from_quadratic(
        stencil: QuadraticPointStencil,
        operator: &CanonicalTemporalWaveOperator,
    ) -> Result<Self, WaveError> {
        let fixed = CanonicalPointStencil::from_quadratic(stencil, operator.base())?;
        let element = stencil.element as usize;
        let start = element
            .checked_mul(6)
            .ok_or(WaveError::InvalidMesh("invalid complementary sample range"))?;
        let samples =
            operator
                .complementary
                .get(start..start + 6)
                .ok_or(WaveError::InvalidMesh(
                    "the point stencil has no temporal complementary samples",
                ))?;
        let material = samples[0].coefficient.material;
        if samples
            .iter()
            .any(|sample| sample.coefficient.material != material)
        {
            return Err(WaveError::InvalidMesh(
                "one element has several temporal materials",
            ));
        }
        let x = fixed
            .complementary_weights
            .iter()
            .zip(samples)
            .map(|(weight, sample)| weight * sample.coefficient.coordinates.x)
            .sum::<f64>();
        let y = fixed
            .complementary_weights
            .iter()
            .zip(samples)
            .map(|(weight, sample)| weight * sample.coefficient.coordinates.y)
            .sum::<f64>();
        let coordinates = MaterialCoordinates {
            x,
            y,
            r: x.hypot(y),
            theta: y.atan2(x),
        };
        let sample_coefficients: [CanonicalTemporalCoefficientSample; 6] =
            std::array::from_fn(|local| samples[local].coefficient.into());
        let mut complementary: CanonicalTemporalCoefficientSample = samples[0].coefficient.into();
        complementary.coordinates = coordinates;
        let mut primary: CanonicalTemporalCoefficientSample = operator
            .base
            .primary_contributions()
            .iter()
            .zip(&operator.primary)
            .find(|(contribution, sample)| {
                contribution.element as usize == element && sample.coefficient.material == material
            })
            .map(|(_, sample)| sample.coefficient.into())
            .ok_or(WaveError::InvalidMesh(
                "the point stencil has no temporal primary samples",
            ))?;
        primary.coordinates = coordinates;
        Ok(Self {
            fixed,
            primary,
            sample_coefficients,
            complementary,
        })
    }

    pub fn fixed(&self) -> CanonicalPointStencil {
        self.fixed
    }

    pub fn element(&self) -> u32 {
        self.fixed.complementary_samples[0] / 6
    }

    pub fn primary_coefficient(&self) -> CanonicalTemporalCoefficientSample {
        self.primary
    }

    pub fn complementary_coefficient(&self) -> CanonicalTemporalCoefficientSample {
        self.complementary
    }

    pub fn sample_coefficients(&self) -> [CanonicalTemporalCoefficientSample; 6] {
        self.sample_coefficients
    }

    /// The physical complementary field at the probe point. Each sample is
    /// divided by its own instantaneous factor before interpolation, so a
    /// travelling modulation is resolved at the samples rather than smeared
    /// through one factor at the probe.
    pub fn complementary_field(
        &self,
        complementary_flux: &[Point2],
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<Point2, WaveError> {
        // Without the operator a field law has no inverse to apply; the
        // nonlinear field is read through [`Self::sample`].
        if self
            .sample_coefficients
            .iter()
            .any(|sample| sample.law.field != FieldLawValues::Linear)
        {
            return Err(WaveError::Unsupported(CONSUMER_FIELD_LAWS));
        }
        let mut field = Point2::default();
        for local in 0..self.fixed.complementary_samples.len() {
            let sample = self.fixed.complementary_samples[local] as usize;
            let Some(flux) = complementary_flux.get(sample) else {
                return Err(WaveError::InvalidState);
            };
            let factor = self.sample_coefficients[local].factor_at(time, runtime)?;
            field = field
                + self.fixed.sample_inverses[local].apply(*flux) / factor
                    * self.fixed.complementary_weights[local];
        }
        if field.finite() {
            Ok(field)
        } else {
            Err(WaveError::InvalidState)
        }
    }

    /// Samples an ordinary completed step. `previous_primary_flux` belongs to
    /// `time-time_step`; zero-duration filter/event boundaries must continue
    /// to use their explicit consumer deferral/rebase policy instead.
    #[allow(clippy::too_many_arguments)]
    pub fn sample(
        &self,
        operator: &CanonicalTemporalWaveOperator,
        primary_flux: &[f64],
        previous_primary_flux: &[f64],
        complementary_flux: &[Point2],
        time: f64,
        time_step: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<CanonicalPointSample, WaveError> {
        if !time.is_finite() || !time_step.is_finite() || time_step <= 0.0 {
            return Err(WaveError::InvalidState);
        }
        let current = operator.primary_field_at(primary_flux, time, runtime)?;
        let previous =
            operator.primary_field_at(previous_primary_flux, time - time_step, runtime)?;
        let mut primary = 0.0;
        let mut previous_primary = 0.0;
        for local in 0..self.fixed.nodes.len() {
            let node = self.fixed.nodes[local] as usize;
            let (Some(current), Some(previous)) = (current.get(node), previous.get(node)) else {
                return Err(WaveError::InvalidState);
            };
            primary += self.fixed.primary_weights[local] * current;
            previous_primary += self.fixed.primary_weights[local] * previous;
        }
        let primary_rate = (primary - previous_primary) / time_step;
        let primary_factor = self.primary.factor_at(time, runtime)?;
        let complementary_factor = self.complementary.factor_at(time, runtime)?;
        let (complementary, energy_density) = if operator.has_field_laws {
            // Each sample is inverted at its own site, as the solver does,
            // and only the physical fields are interpolated. The density is
            // the stored energy of the interpolated fields under the laws at
            // the probe, `c (ḡ(r) r² − G(r))` per row, whose linear form is
            // the `c r²/2` below.
            let mut field = Point2::default();
            for local in 0..self.fixed.complementary_samples.len() {
                let sample = self.fixed.complementary_samples[local] as usize;
                let flux = complementary_flux
                    .get(sample)
                    .ok_or(WaveError::InvalidState)?;
                field = field
                    + operator.complementary_sample_field(sample, *flux, time, runtime)?
                        * self.fixed.complementary_weights[local];
            }
            let store = |law: FieldLawValues, r: f64| law.multiplier(r) * r * r - law.coenergy(r);
            let reference = self.fixed.complementary_reference;
            let scale = reference.xx.abs().max(reference.yy.abs());
            let isotropic = self.complementary.law.field == FieldLawValues::Linear
                || (reference.xy.abs() <= 1e-12 * scale
                    && (reference.xx - reference.yy).abs() <= 1e-12 * scale);
            if !isotropic {
                return Err(WaveError::InvalidState);
            }
            let complementary_store = if self.complementary.law.field == FieldLawValues::Linear {
                0.5 * reference.quadratic_form(field)
            } else {
                reference.xx * store(self.complementary.law.field, field.norm())
            };
            (
                field,
                self.fixed.primary_reference
                    * primary_factor
                    * store(self.primary.law.field, primary.abs())
                    + complementary_factor * complementary_store,
            )
        } else {
            let complementary = self.complementary_field(complementary_flux, time, runtime)?;
            (
                complementary,
                0.5 * (self.fixed.primary_reference * primary_factor * primary * primary
                    + complementary_factor
                        * self
                            .fixed
                            .complementary_reference
                            .quadratic_form(complementary)),
            )
        };
        let energy_flow =
            Point2::new(-complementary.y, complementary.x) * (self.fixed.orientation * primary);
        if [
            primary,
            primary_rate,
            energy_density,
            energy_flow.x,
            energy_flow.y,
        ]
        .into_iter()
        .all(f64::is_finite)
            && complementary.finite()
        {
            Ok(CanonicalPointSample {
                primary,
                primary_rate,
                complementary,
                energy_density,
                energy_flow,
            })
        } else {
            Err(WaveError::InvalidState)
        }
    }
}

/// One clipped area piece compiled against a time-driven generation.
///
/// The fixed record already carries the geometry, the samples' constitutive
/// inverses and each local node's time-independent mass contribution. What a
/// driven material adds is a factor on each of those: one per primary
/// contribution and one per complementary sample.
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalTemporalAreaContribution {
    fixed: CanonicalAreaContribution,
    primary: [CanonicalTemporalCoefficientSample; 7],
    samples: [CanonicalTemporalCoefficientSample; 6],
}

impl CanonicalTemporalAreaContribution {
    pub fn from_element(
        element: QuadraticAreaElement,
        operator: &CanonicalTemporalWaveOperator,
    ) -> Result<Self, WaveError> {
        let fixed = canonical_area_contribution(element, operator.base())?;
        let parent = element.element as usize;
        let start = parent
            .checked_mul(6)
            .ok_or(WaveError::InvalidMesh("invalid complementary sample range"))?;
        let samples =
            operator
                .complementary
                .get(start..start + 6)
                .ok_or(WaveError::InvalidMesh(
                    "the area element has no temporal complementary samples",
                ))?;
        // The parent's seven contributions are emitted together in local
        // order, so they are a direct slice rather than a scan.
        let contributions = operator.base().primary_contributions();
        let first = parent
            .checked_mul(7)
            .ok_or(WaveError::InvalidMesh("invalid primary contribution range"))?;
        let owned = contributions
            .get(first..first + 7)
            .filter(|owned| {
                owned.iter().enumerate().all(|(local, contribution)| {
                    contribution.element as usize == parent
                        && contribution.local_node as usize == local
                        && contribution.node == element.nodes[local]
                })
            })
            .ok_or(WaveError::InvalidMesh(
                "area element does not match the canonical primary contributions",
            ))?;
        let temporal =
            operator
                .primary
                .get(first..first + owned.len())
                .ok_or(WaveError::InvalidMesh(
                    "the area element has no temporal primary samples",
                ))?;
        let primary: [CanonicalTemporalCoefficientSample; 7] =
            std::array::from_fn(|local| temporal[local].coefficient.into());
        Ok(Self {
            fixed,
            primary,
            samples: std::array::from_fn(|local| samples[local].coefficient.into()),
        })
    }

    pub fn fixed(&self) -> &CanonicalAreaContribution {
        &self.fixed
    }

    pub fn primary_coefficients(&self) -> [CanonicalTemporalCoefficientSample; 7] {
        self.primary
    }

    pub fn sample_coefficients(&self) -> [CanonicalTemporalCoefficientSample; 6] {
        self.samples
    }
}

/// The indicator's direct-state defect terms over a time-driven generation.
///
/// This is the variable-coefficient counterpart of
/// [`canonical_indicator_supplement`]. Every map is evaluated at the time it
/// belongs to rather than at the authored coefficients: the two endpoint
/// fields use the mass at their own endpoints, the drift the residual
/// measures against uses the midpoint law, and the energy and recovery norms
/// use the instantaneous constitutive inverse. Reusing the fixed maps would
/// charge the estimator for the medium's own modulation and refine against
/// it.
///
/// Thin gaps and open boundaries are covered: their defects are the fixed
/// path's own, with the mass and force in force at the instant standing in for
/// the authored ones. Loss, a damped boundary and prescribed boundary data are
/// refused rather than reported with those terms missing, because each puts a
/// term in the evolution the defect would otherwise charge to the mesh.
///
/// Gate O: an oscillator medium's store includes `Σ m₀ V(r)`, split over the
/// elements each contribution belongs to, and the outgoing trace's force
/// includes the restoring force at each endpoint's own `r`. The drift of `r`
/// adds no term of its own: it is `ṙ = u` on the same midpoint field whose
/// defect the `b` drift residual already measures through `ηC`, and its
/// uniform part is not a spatial error. A kink's steepness is in `b = ηC r`,
/// where the complementary recovery sees it.
pub fn canonical_temporal_indicator_supplement(
    mesh: &TriMesh,
    operator: &CanonicalTemporalWaveOperator,
    forcing: &CanonicalForcing,
    snapshot: &CanonicalIndicatorSnapshot,
    runtime: &CanonicalMaterialRuntimeState,
    resolved_frequency_hz: f64,
) -> Result<CanonicalIndicatorSupplement, WaveError> {
    if !operator.indicator_supplement_supported() {
        return Err(WaveError::Unsupported(
            "no adaptive estimate for a driven medium carrying loss, a damped \
             boundary or prescribed boundary data",
        ));
    }
    let base = operator.base();
    let node_count = base.degrees_of_freedom();
    let sample_count = base.complementary_degrees_of_freedom();
    let gap_count = base.thin_gap_samples().len();
    let outgoing_count = base
        .outgoing_boundary()
        .map_or(0, |boundary| boundary.auxiliary_count());
    if snapshot.auxiliary.len() != gap_count + outgoing_count
        || snapshot.previous_auxiliary.len() != gap_count + outgoing_count
    {
        return Err(WaveError::InvalidState);
    }
    let integrated_count = if operator.has_restoring() {
        node_count
    } else {
        0
    };
    if snapshot.integrated_field.len() != integrated_count
        || snapshot.previous_integrated_field.len() != integrated_count
    {
        return Err(WaveError::InvalidState);
    }
    if snapshot.mesh_revision != mesh.mesh_revision
        || base.generation().mesh_revision != mesh.mesh_revision
        || base.element_nodes().len() != mesh.triangles.len()
        || snapshot.primary_flux.len() != node_count
        || snapshot.previous_primary_flux.len() != node_count
        || snapshot.complementary_flux.len() != sample_count
        || snapshot.previous_complementary_flux.len() != sample_count
        || !snapshot.time.is_finite()
        || !snapshot.time_step.is_finite()
        || snapshot.time_step <= 0.0
        || !resolved_frequency_hz.is_finite()
        || resolved_frequency_hz < 0.0
    {
        return Err(WaveError::InvalidState);
    }
    let time = snapshot.time;
    let previous_time = time - snapshot.time_step;

    // Each endpoint field divides by the mass in force at that endpoint. The
    // average of the two is the estimator's midpoint proxy, as on the fixed
    // path; what changes is that the two masses now differ.
    let current_field = operator.primary_field_at(&snapshot.primary_flux, time, runtime)?;
    let previous_field =
        operator.primary_field_at(&snapshot.previous_primary_flux, previous_time, runtime)?;
    let midpoint_field = current_field
        .iter()
        .zip(&previous_field)
        .map(|(current, previous)| 0.5 * (current + previous))
        .collect::<Vec<_>>();

    let element_count = mesh.triangles.len();
    let mut element_complementary_recovery = vec![0.0; element_count];
    let mut element_cell_residual = vec![0.0; element_count];
    let mut element_boundary_residual = vec![0.0; element_count];
    let mut element_energy = vec![0.0; element_count];

    // Instantaneous complementary inverses, once per sample rather than once
    // per use: the recovery, the residual norm and the energy all need them.
    //
    // On a field-dependent medium the physical field is the nonlinear inverse
    // of `b`, and every norm is the tangent map at the snapshot, `J_b`: the
    // energy of a small defect `δ` is `½ δ·J_b δ`, which is what the linear
    // `J` meant. A linear sample's tangent is its own `J / factor`.
    let nonlinear = operator.has_field_laws();
    let mut inverses = Vec::with_capacity(sample_count);
    let mut sample_fields = Vec::new();
    if nonlinear {
        inverses =
            operator.complementary_tangents_at(&snapshot.complementary_flux, time, runtime)?;
        sample_fields =
            operator.complementary_field_at(&snapshot.complementary_flux, time, runtime)?;
    }
    for (sample, temporal) in base
        .constitutive_samples()
        .iter()
        .zip(&operator.complementary)
        .filter(|_| !nonlinear)
    {
        let factor = coefficient_factor(temporal.coefficient, time, runtime)?;
        if !factor.is_finite() || factor <= 0.0 {
            return Err(WaveError::Unsupported(
                "a driven medium's instantaneous complementary factor is not positive",
            ));
        }
        inverses.push(SymmetricTensor2::new(
            sample.complementary_inverse.xx / factor,
            sample.complementary_inverse.xy / factor,
            sample.complementary_inverse.yy / factor,
        ));
    }

    // The physical complementary field at every sample.
    let physical_at = |index: usize| -> Point2 {
        if nonlinear {
            sample_fields[index]
        } else {
            inverses[index].apply(snapshot.complementary_flux[index])
        }
    };

    let omega = (std::f64::consts::TAU * resolved_frequency_hz).max(1.0);
    let mut recovered = BTreeMap::<(usize, RegionId), (Point2, f64)>::new();
    let sample_points: [[f64; 3]; 6] = base
        .constitutive_samples()
        .get(..6)
        .ok_or(WaveError::InvalidState)?
        .iter()
        .map(|sample| sample.barycentric)
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|_| WaveError::InvalidState)?;
    let vertex_weights = [0, 1, 2]
        .map(|local| {
            let target = std::array::from_fn(|coordinate| (coordinate == local) as u8 as f64);
            complementary_interpolation_weights(sample_points, target)
        })
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    for (element, triangle) in mesh.triangles.iter().enumerate() {
        let points = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
        let area = 0.5 * (points[1] - points[0]).cross(points[2] - points[0]);
        if !area.is_finite() || area <= 0.0 {
            return Err(WaveError::InvalidMesh("invalid temporal indicator element"));
        }
        let start = element * 6;
        for (local, interpolation) in vertex_weights.iter().enumerate() {
            let value = interpolation
                .iter()
                .enumerate()
                .fold(Point2::default(), |sum, (sample, weight)| {
                    sum + physical_at(start + sample) * *weight
                });
            let entry = recovered
                .entry((triangle.vertices[local], triangle.region))
                .or_default();
            entry.0 = entry.0 + value * area;
            entry.1 += area;
        }
    }

    for (element, triangle) in mesh.triangles.iter().enumerate() {
        let start = element * 6;
        for local in 0..6 {
            let sample = base.constitutive_samples()[start + local];
            let smoothed = triangle.vertices.iter().zip(sample.barycentric).fold(
                Point2::default(),
                |sum, (vertex, weight)| {
                    let entry = recovered[&(*vertex, triangle.region)];
                    sum + entry.0 * (weight / entry.1)
                },
            );
            let physical = physical_at(start + local);
            let defect = physical - smoothed;
            let reference = inverses[start + local]
                .inverse()
                .ok_or(WaveError::InvalidState)?;
            element_complementary_recovery[element] +=
                omega * omega * sample.integration_weight * defect.dot(reference.apply(defect));
        }
    }
    let complementary_recovery_contribution = element_complementary_recovery.iter().sum();

    // Candidate replacement for the scalar interior flux jump, measured here
    // but summed into nothing: whether it converges is the question that
    // decides whether the estimator should switch to it.
    //
    // The direct state keeps its complementary variable in a frame rotated by
    // a quarter turn - the reference map is `rotate_tensor(stiffness)` - and
    // `b` evolves from the curl of the primary field. So `(S b) . n` across a
    // face is a tangential derivative of a single-valued edge trace and is
    // identical from both sides by construction: measured, that jump is `1e-32`
    // against a scalar jump of `1e-5`, which is structural zero, not a small
    // error. The informative component is the tangential one, which is the
    // scalar `[[A grad(u) . n]]` carried through that rotation.
    //
    // What makes it a candidate at all is where it comes from. The scalar term
    // differentiates the nodal primary quotient `Q/M`, and a spatially
    // patterned mass makes that quotient carry a patterned lumping error. This
    // one reads a variable the solver stores independently per element and
    // inverts at that element's own six samples, so no nodal mass enters it.
    let mut element_complementary_jump = vec![0.0; element_count];
    let mut faces = BTreeMap::<(usize, usize), Vec<usize>>::new();
    for (element, triangle) in mesh.triangles.iter().enumerate() {
        for local in 0..3 {
            let start = triangle.vertices[local];
            let end = triangle.vertices[(local + 1) % 3];
            faces
                .entry((start.min(end), start.max(end)))
                .or_default()
                .push(element);
        }
    }
    for ((start_vertex, end_vertex), sides) in &faces {
        let [left, right] = sides[..] else {
            continue;
        };
        let ends = [
            mesh.vertices[*start_vertex].point,
            mesh.vertices[*end_vertex].point,
        ];
        let length = (ends[1] - ends[0]).norm();
        if !length.is_finite() || length <= 0.0 {
            return Err(WaveError::InvalidMesh("invalid temporal indicator face"));
        }
        let mut integrals = [0.0; 2];
        for (fraction, weight) in [
            (0.112_701_665_379_258_3, 5.0 / 18.0),
            (0.5, 8.0 / 18.0),
            (0.887_298_334_620_741_7, 5.0 / 18.0),
        ] {
            let mut difference = Point2::default();
            let mut scales = [0.0; 2];
            for (side, element) in [left, right].into_iter().enumerate() {
                let triangle = mesh.triangles[element];
                let points = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
                // The point sits on one of this element's own edges, so its
                // barycentric coordinates are exact rather than solved for.
                let local_start = triangle
                    .vertices
                    .iter()
                    .position(|vertex| vertex == start_vertex)
                    .ok_or(WaveError::InvalidMesh("a face is not on its own element"))?;
                let local_end = triangle
                    .vertices
                    .iter()
                    .position(|vertex| vertex == end_vertex)
                    .ok_or(WaveError::InvalidMesh("a face is not on its own element"))?;
                let mut barycentric = [0.0; 3];
                barycentric[local_start] = 1.0 - fraction;
                barycentric[local_end] = fraction;
                let interpolation =
                    complementary_interpolation_weights(sample_points, barycentric)?;
                let start = element * 6;
                let mut flux = Point2::default();
                let mut map = SymmetricTensor2::default();
                for (sample, weight) in interpolation.into_iter().enumerate() {
                    let inverse = inverses[start + sample];
                    flux = flux + physical_at(start + sample) * weight;
                    map = SymmetricTensor2::new(
                        map.xx + inverse.xx * weight,
                        map.xy + inverse.xy * weight,
                        map.yy + inverse.yy * weight,
                    );
                }
                // The interpolated map normalizes the jump exactly as the
                // scalar term's pointwise stiffness does. An extrapolating
                // weight can leave it indefinite, which the six-sample mean
                // cannot, and the two differ by `O(h)` in a smooth medium.
                let scale = map.determinant().sqrt();
                scales[side] = if scale.is_finite() && scale > 0.0 {
                    scale
                } else {
                    let mean =
                        (start..start + 6).fold(SymmetricTensor2::default(), |sum, index| {
                            SymmetricTensor2::new(
                                sum.xx + inverses[index].xx / 6.0,
                                sum.xy + inverses[index].xy / 6.0,
                                sum.yy + inverses[index].yy / 6.0,
                            )
                        });
                    let mean = mean.determinant().sqrt();
                    if mean.is_finite() && mean > 0.0 {
                        mean
                    } else {
                        return Err(WaveError::Unsupported(
                            "an element's mean constitutive inverse is not positive",
                        ));
                    }
                };
                let _ = points;
                difference = difference + flux * if side == 0 { 1.0 } else { -1.0 };
            }
            let tangent = (ends[1] - ends[0]) / length;
            let jump = difference.dot(tangent);
            for (integral, scale) in integrals.iter_mut().zip(scales) {
                *integral += weight * length * jump * jump / scale;
            }
        }
        for (element, integral) in [left, right].into_iter().zip(integrals) {
            element_complementary_jump[element] += 0.5 * length * integral;
        }
    }
    let complementary_jump_contribution = element_complementary_jump.iter().sum();

    // Primary energy uses the mass in force now, so a modulated element is
    // not credited with the storage its authored coefficient would have.
    //
    // A nonlinear node's store splits exactly by term: with
    // `Q = Σ mᵢ ḡᵢ(U) U`, `T = U·Q − Σ mᵢ Gᵢ(U) = Σ mᵢ (ḡᵢ U² − Gᵢ(U))`.
    let mass = operator.primary_mass_at(time, runtime)?;
    let primary_terms = if nonlinear {
        Some(operator.primary_terms_at(time, runtime)?.0)
    } else {
        None
    };
    for (index, contribution) in base.primary_contributions().iter().enumerate() {
        let node = contribution.node as usize;
        if let Some(terms) = &primary_terms {
            let field = current_field[node].abs();
            let range = operator.primary_range(node);
            let position = operator.node_contributions[range.clone()]
                .iter()
                .position(|entry| *entry as usize == index)
                .ok_or(WaveError::InvalidState)?;
            let term = terms[range.start + position];
            element_energy[contribution.element as usize] += term.coefficient
                * (term.law.multiplier(field) * field * field - term.law.coenergy(field));
            continue;
        }
        let element = contribution.element as usize;
        let factor = coefficient_factor(
            operator.primary[element * 7 + contribution.local_node as usize].coefficient,
            time,
            runtime,
        )?;
        let share = contribution.geometric_weight * contribution.reference_coefficient * factor;
        element_energy[element] +=
            0.5 * share * snapshot.primary_flux[node] * snapshot.primary_flux[node]
                / (mass[node] * mass[node]);
    }

    // The restoring store, each contribution's share on its own element.
    if operator.has_restoring() {
        for (contribution, sample) in base.primary_contributions().iter().zip(&operator.primary) {
            element_energy[contribution.element as usize] += contribution.geometric_weight
                * contribution.reference_coefficient
                * sample
                    .restoring
                    .potential(snapshot.integrated_field[contribution.node as usize]);
        }
    }

    // The drift the residual measures against is the one the solver takes:
    // the midpoint primary field, with no loss because the conservative bulk
    // carries none.
    for (sample_index, sample) in base.constitutive_samples().iter().enumerate() {
        let element = sample.element as usize;
        let current = snapshot.complementary_flux[sample_index];
        let previous = snapshot.previous_complementary_flux[sample_index];
        element_energy[element] += if nonlinear {
            operator.complementary_sample_energy(sample_index, current, time, runtime)?
        } else {
            0.5 * sample.integration_weight * current.dot(inverses[sample_index].apply(current))
        };
        let nodes = base.element_nodes()[element];
        let reference = midpoint_field[nodes[0] as usize];
        let mut curl = Point2::default();
        for local in 1..nodes.len() {
            curl =
                curl + sample.curls()[local] * (midpoint_field[nodes[local] as usize] - reference);
        }
        let expected = previous + curl * (base.orientation() * snapshot.time_step);
        let defect = current - expected;
        element_cell_residual[element] +=
            0.5 * sample.integration_weight * defect.dot(inverses[sample_index].apply(defect));
    }
    let drift_contribution = element_cell_residual.iter().sum();

    // Which elements own a node, so a boundary defect measured at the trace
    // lands on the elements that would have to be refined for it.
    let mut node_elements = vec![Vec::new(); node_count];
    for (element, nodes) in base.element_nodes().iter().enumerate() {
        for node in nodes {
            if !node_elements[*node as usize].contains(&element) {
                node_elements[*node as usize].push(element);
            }
        }
    }

    // A gap stores `stiffness * jump^2 / 2` against the field across it, and
    // the field is the instantaneous one computed above.
    let mut thin_gap_contribution = 0.0;
    for (index, gap) in base.thin_gap_samples().iter().enumerate() {
        let left = gap.left_node as usize;
        let right = gap.right_node as usize;
        let expected = snapshot.time_step
            * 0.5
            * ((current_field[left] - current_field[right])
                + (previous_field[left] - previous_field[right]));
        let defect = snapshot.auxiliary[index] - snapshot.previous_auxiliary[index] - expected;
        let residual = 0.5 * gap.stiffness * defect * defect;
        thin_gap_contribution += residual;
        let mut owners = node_elements[left].clone();
        for element in &node_elements[right] {
            if !owners.contains(element) {
                owners.push(*element);
            }
        }
        let share = residual / owners.len().max(1) as f64;
        let energy_share =
            0.5 * gap.stiffness * snapshot.auxiliary[index].powi(2) / owners.len().max(1) as f64;
        for element in owners {
            element_boundary_residual[element] += share;
            element_energy[element] += energy_share;
        }
    }

    // The outgoing wall's own defect. Everything that divides by the nodal
    // mass takes the one in force at this instant, which is the whole
    // difference from the fixed term: the trace admittance and the modal
    // couplings inside the generator, and the residual's own normalization.
    let mut outgoing_contribution = 0.0;
    if let Some(boundary) = base.outgoing_boundary() {
        let current_z = &snapshot.auxiliary[gap_count..];
        let previous_z = &snapshot.previous_auxiliary[gap_count..];
        let trace_of = |flux: &[f64]| {
            boundary
                .trace_nodes()
                .iter()
                .map(|node| flux[*node as usize])
                .collect::<Vec<_>>()
        };
        let current_trace = trace_of(&snapshot.primary_flux);
        let previous_trace = trace_of(&snapshot.previous_primary_flux);
        let midpoint_trace = current_trace
            .iter()
            .zip(&previous_trace)
            .map(|(current, previous)| 0.5 * (current + previous))
            .collect::<Vec<_>>();
        let midpoint_z = current_z
            .iter()
            .zip(previous_z)
            .map(|(current, previous)| 0.5 * (current + previous))
            .collect::<Vec<_>>();
        let midpoint_mass = operator.primary_mass_at(time - 0.5 * snapshot.time_step, runtime)?;
        // A nonlinear trace is stepped on the discrete gradient `ū`, so the
        // generator is read at `ū` with unit mass - the linear kick's
        // `Q_mid/m` in field form - and the residual is weighted by the
        // tangent `∂U/∂Q` in place of `1/m`.
        let (mut derivative, residual_weight) = if nonlinear {
            let midpoint_time = time - 0.5 * snapshot.time_step;
            let (terms, _) = operator.primary_terms_at(midpoint_time, runtime)?;
            let mut gradient = Vec::with_capacity(midpoint_trace.len());
            for (trace, node) in boundary.trace_nodes().iter().enumerate() {
                let (value, _) = operator.primary_discrete_gradient(
                    &terms,
                    *node as usize,
                    previous_trace[trace],
                    current_trace[trace],
                    runtime,
                )?;
                gradient.push(value);
            }
            let unit = vec![1.0; node_count];
            let field = operator.primary_field_at(
                &midpoint_flux_of(&snapshot.primary_flux, &snapshot.previous_primary_flux),
                midpoint_time,
                runtime,
            )?;
            let weight = operator.primary_tangent_inverse_at(&field, midpoint_time, runtime)?;
            (
                boundary.diagnostic_derivative_with(base, &unit, &gradient, &midpoint_z)?,
                weight,
            )
        } else {
            (
                boundary.diagnostic_derivative_with(
                    base,
                    &midpoint_mass,
                    &midpoint_trace,
                    &midpoint_z,
                )?,
                midpoint_mass.iter().map(|mass| 1.0 / mass).collect(),
            )
        };
        let instantaneous_force = |complementary: &[Point2],
                                   auxiliary: &[f64],
                                   integrated: &[f64],
                                   at: f64|
         -> Result<Vec<f64>, WaveError> {
            let mut force = operator.force_at(complementary, at, runtime)?;
            add_gap_force(operator, &auxiliary[..gap_count], &mut force)?;
            add_restoring_force(operator, integrated, &mut force)?;
            Ok(force)
        };
        let current_force = instantaneous_force(
            &snapshot.complementary_flux,
            &snapshot.auxiliary,
            &snapshot.integrated_field,
            time,
        )?;
        let previous_force = instantaneous_force(
            &snapshot.previous_complementary_flux,
            &snapshot.previous_auxiliary,
            &snapshot.previous_integrated_field,
            previous_time,
        )?;
        let current_source = forcing.integrated_rate(time)?;
        let previous_source = forcing.integrated_rate(previous_time)?;
        for (trace, node) in boundary.trace_nodes().iter().enumerate() {
            if forcing.prescribed()[*node as usize].is_none() {
                derivative[trace] += 0.5
                    * (current_source[*node as usize] + previous_source[*node as usize]
                        - current_force[*node as usize]
                        - previous_force[*node as usize]);
            }
        }
        let mut residual = 0.0;
        for (trace, node) in boundary.trace_nodes().iter().enumerate() {
            if forcing.prescribed()[*node as usize].is_some() {
                continue;
            }
            let defect = current_trace[trace]
                - previous_trace[trace]
                - snapshot.time_step * derivative[trace];
            residual += 0.5 * defect * defect * residual_weight[*node as usize];
        }
        for auxiliary in 0..outgoing_count {
            let defect = current_z[auxiliary]
                - previous_z[auxiliary]
                - snapshot.time_step * derivative[boundary.trace_nodes().len() + auxiliary];
            residual += 0.5 * defect * defect;
        }
        outgoing_contribution = residual;
        let mut owners = Vec::new();
        for node in boundary.trace_nodes() {
            for element in &node_elements[*node as usize] {
                if !owners.contains(element) {
                    owners.push(*element);
                }
            }
        }
        let share = residual / owners.len().max(1) as f64;
        for element in owners {
            element_boundary_residual[element] += share;
        }
    }

    Ok(CanonicalIndicatorSupplement {
        mesh_revision: mesh.mesh_revision,
        element_complementary_recovery,
        element_complementary_jump: Some(element_complementary_jump),
        element_cell_residual,
        element_boundary_residual,
        element_energy,
        drift_contribution,
        complementary_recovery_contribution,
        complementary_jump_contribution,
        thin_gap_contribution,
        outgoing_contribution,
    })
}

fn midpoint_flux_of(current: &[f64], previous: &[f64]) -> Vec<f64> {
    current
        .iter()
        .zip(previous)
        .map(|(current, previous)| 0.5 * (current + previous))
        .collect()
}

/// What a driven medium demands of the mesh, beyond what the sources ask.
///
/// A modulated coefficient is not just a moving number. It mixes with the
/// wave to make sidebands the mesh has to resolve, and a travelling
/// modulation writes a spatial pattern into the operator itself that the mesh
/// has to resolve whether or not a wave is present.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalTemporalResolution {
    /// Highest temporal frequency the field is expected to carry, given a
    /// source at `source_frequency_hz`. Zero when nothing is driven.
    pub frequency_hz: f64,
    /// Sideband order the frequency above accounts for. One means only the
    /// first pair; zero means the drive contributes no resolvable sideband.
    pub sideband_order: u32,
    /// Shortest spatial period any travelling modulation writes into the
    /// coefficients, or infinity where none does. This is a property of the
    /// operator, so it binds even on a quiet field.
    pub coefficient_wavelength: f64,
}

impl CanonicalTemporalResolution {
    /// Sideband amplitude below which a pair is not worth resolving. A
    /// first-order pair carries about `depth/2` of the carrier, and each
    /// further order multiplies by roughly the same factor, so this bounds
    /// the order rather than fixing it.
    const SIDEBAND_FLOOR: f64 = 1.0e-2;
    /// Sidebands counted at most, whatever the depth. Depth is bounded below
    /// one by the positivity requirement on the multiplier, so this is a
    /// guard against a pathological authored value rather than a physical
    /// limit.
    const MAX_SIDEBAND_ORDER: u32 = 8;

    /// Harmonics a drive's own waveform carries, which multiply its frequency
    /// before any mixing. A cosine pump has one; a smoothed square has odd
    /// harmonics whose amplitude falls with the sharpness that produced them.
    fn harmonic_order(drive: TimeDriveValues) -> u32 {
        match drive {
            TimeDriveValues::None => 0,
            TimeDriveValues::ParametricPump { .. }
            | TimeDriveValues::TravellingModulation { .. } => 1,
            TimeDriveValues::TimeCrystal { sharpness, .. } => {
                // tanh(s cos t)/tanh(s) approaches a square wave as `s`
                // grows, and its odd harmonics decay on a scale set by `s`.
                // Counting `1 + 2s` of them keeps the retained content above
                // the same floor the sidebands use without pretending a
                // sharp square is band-limited.
                let order = (1.0 + 2.0 * sharpness.abs()).ceil();
                if order.is_finite() {
                    (order as u32).clamp(1, Self::MAX_SIDEBAND_ORDER)
                } else {
                    Self::MAX_SIDEBAND_ORDER
                }
            }
        }
    }

    fn sideband_order(depth: f64) -> u32 {
        let depth = depth.abs();
        if depth <= 0.0 {
            return 0;
        }
        // Each further order costs roughly another factor of `depth/2`.
        let ratio = (0.5 * depth).min(0.99);
        if ratio <= 0.0 {
            return 0;
        }
        let order = (Self::SIDEBAND_FLOOR.ln() / ratio.ln()).ceil();
        if order.is_finite() {
            (order.max(1.0) as u32).min(Self::MAX_SIDEBAND_ORDER)
        } else {
            Self::MAX_SIDEBAND_ORDER
        }
    }

    fn drive_depth(drive: TimeDriveValues) -> f64 {
        match drive {
            TimeDriveValues::None => 0.0,
            TimeDriveValues::ParametricPump { depth, .. }
            | TimeDriveValues::TimeCrystal { depth, .. }
            | TimeDriveValues::TravellingModulation { depth, .. } => depth,
        }
    }

    fn drive_frequency_hz(drive: TimeDriveValues) -> f64 {
        match drive {
            TimeDriveValues::None => 0.0,
            TimeDriveValues::ParametricPump { frequency_hz, .. }
            | TimeDriveValues::TimeCrystal { frequency_hz, .. }
            | TimeDriveValues::TravellingModulation { frequency_hz, .. } => frequency_hz.abs(),
        }
    }
}

impl CanonicalTemporalResolution {
    /// Accumulates the demand of a set of evaluated drives over the sources
    /// already accounted for by `source_frequency_hz`.
    pub fn of_drives(
        drives: impl IntoIterator<Item = TimeDriveValues>,
        source_frequency_hz: f64,
    ) -> Self {
        let source_frequency_hz = source_frequency_hz.max(0.0);
        let mut demand = Self {
            frequency_hz: source_frequency_hz,
            sideband_order: 0,
            coefficient_wavelength: f64::INFINITY,
        };
        for drive in drives {
            let harmonics = Self::harmonic_order(drive);
            let order = Self::sideband_order(Self::drive_depth(drive));
            if harmonics == 0 || order == 0 {
                continue;
            }
            let reach = f64::from(order * harmonics) * Self::drive_frequency_hz(drive);
            if reach.is_finite() {
                demand.frequency_hz = demand.frequency_hz.max(source_frequency_hz + reach);
                demand.sideband_order = demand.sideband_order.max(order);
            }
            if let TimeDriveValues::TravellingModulation { wavenumber, .. } = drive {
                let wavenumber = wavenumber.abs();
                if wavenumber > 0.0 {
                    demand.coefficient_wavelength = demand
                        .coefficient_wavelength
                        .min(std::f64::consts::TAU / wavenumber);
                }
            }
        }
        demand
    }

    /// The demand of authored materials, for callers sizing a mesh before a
    /// temporal operator exists.
    pub fn of_materials<'a>(
        materials: impl IntoIterator<Item = &'a Material>,
        source_frequency_hz: f64,
    ) -> Result<Self, MaterialError> {
        let mut drives = Vec::new();
        for material in materials {
            let parameters = &material.parameters;
            drives.push(material.mass_law.drive.evaluate(parameters)?);
            drives.push(material.stiffness_law.drive.evaluate(parameters)?);
            for channel in [&material.electric_loss, &material.magnetic_loss]
                .into_iter()
                .flatten()
            {
                drives.push(channel.law.drive.evaluate(parameters)?);
            }
        }
        Ok(Self::of_drives(drives, source_frequency_hz))
    }
}

impl CanonicalTemporalWaveOperator {
    /// What this operator's drives demand of the mesh, given the sources
    /// already accounted for by `source_frequency_hz`.
    ///
    /// The mesh-size rule cannot keep using the source frequency alone once a
    /// medium is driven. Mixing puts energy at `f_source +/- n f_drive`, and a
    /// travelling drive additionally patterns the coefficients in space, which
    /// constrains the mesh even where the field is quiet.
    pub fn resolution_demand(&self, source_frequency_hz: f64) -> CanonicalTemporalResolution {
        if !self.has_temporal_laws {
            return CanonicalTemporalResolution::of_drives([], source_frequency_hz);
        }
        CanonicalTemporalResolution::of_drives(
            self.primary
                .iter()
                .map(|sample| sample.coefficient.law.drive)
                .chain(
                    self.complementary
                        .iter()
                        .map(|sample| sample.coefficient.law.drive),
                )
                .chain(self.primary.iter().map(|sample| sample.loss.law.drive))
                .chain(
                    self.complementary
                        .iter()
                        .map(|sample| sample.loss.law.drive),
                ),
            source_frequency_hz,
        )
    }
}

/// Bulk energy split by storage, with the power an authored material
/// trajectory is pumping into it at this instant.
///
/// `temporal_power` is the explicit partial time derivative of the
/// Hamiltonian at fixed canonical state, evaluated from analytic coefficient
/// rates. It is not a finite difference between steps, so a consumer can
/// report it from one snapshot without keeping history, and it is the term
/// that makes a driven medium's energy change legitimate rather than drift.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanonicalTemporalEnergyBreakdown {
    pub primary: f64,
    pub complementary: f64,
    pub temporal_power: f64,
}

impl CanonicalTemporalEnergyBreakdown {
    pub fn total(self) -> f64 {
        self.primary + self.complementary
    }
}

/// Splits the conservative bulk energy and reports the instantaneous material
/// pump power.
///
/// The conservative bulk contract excludes gaps, open boundaries, losses and
/// forcing, so unlike the fixed breakdown there are no auxiliary terms to
/// report; a generation carrying them is rejected rather than summarised
/// with the driven terms missing.
pub fn canonical_temporal_energy_breakdown(
    operator: &CanonicalTemporalWaveOperator,
    primary_flux: &[f64],
    complementary_flux: &[Point2],
    time: f64,
    runtime: &CanonicalMaterialRuntimeState,
) -> Result<CanonicalTemporalEnergyBreakdown, WaveError> {
    if !operator.conservative_bulk_supported() {
        return Err(WaveError::Unsupported(
            "no energy breakdown for a generation beyond the conservative bulk",
        ));
    }
    let (primary, primary_rate) = operator.primary_energy_and_rate(primary_flux, time, runtime)?;
    let (complementary, complementary_rate) =
        operator.complementary_energy_and_rate(complementary_flux, time, runtime)?;
    let result = CanonicalTemporalEnergyBreakdown {
        primary,
        complementary,
        temporal_power: primary_rate + complementary_rate,
    };
    if [result.primary, result.complementary, result.temporal_power]
        .into_iter()
        .all(f64::is_finite)
    {
        Ok(result)
    } else {
        Err(WaveError::InvalidState)
    }
}

/// Area statistics and energy over a time-driven generation.
///
/// The field statistics are moments of the interpolated physical fields, and
/// the energy is the solver's own discrete energy restricted to the covered
/// elements, both evaluated with the laws in force at `time`.
pub fn sample_temporal_canonical_area(
    stencil: &QuadraticAreaStencil,
    operator: &CanonicalTemporalWaveOperator,
    primary_flux: &[f64],
    complementary_flux: &[Point2],
    time: f64,
    runtime: &CanonicalMaterialRuntimeState,
) -> Result<CanonicalAreaSample, WaveError> {
    if primary_flux.len() != operator.base().degrees_of_freedom()
        || complementary_flux.len() != operator.base().complementary_degrees_of_freedom()
        || stencil.covered_area <= 0.0
        || stencil.target_area <= 0.0
        || !time.is_finite()
    {
        return Err(WaveError::InvalidState);
    }
    // The nodal field the solver would step with at `time`: the assembled
    // map's inverse, which on a linear node is the division by its mass.
    let nodal_field = operator.primary_field_at(primary_flux, time, runtime)?;
    let mut primary_integral = 0.0;
    let mut primary_squared = 0.0;
    let mut complementary_squared = 0.0;
    let mut total_energy = 0.0;

    for element in &stencil.elements {
        let compiled = CanonicalTemporalAreaContribution::from_element(*element, operator)?;
        let contribution = compiled.fixed();
        let start = element.element as usize * 6;

        let mut sample_fields = [Point2::default(); 6];
        let mut sample_energy = 0.0;
        for (local, field) in sample_fields.iter_mut().enumerate() {
            let Some(flux) = complementary_flux.get(start + local) else {
                return Err(WaveError::InvalidState);
            };
            let (value, energy) = operator.complementary_sample_field_and_energy(
                start + local,
                *flux,
                time,
                runtime,
            )?;
            *field = value;
            sample_energy += energy;
        }

        for point in contribution.quadrature {
            let mut value = 0.0;
            for (local, basis) in point.primary_weights.iter().enumerate() {
                let node = element.nodes[local] as usize;
                let Some(field) = nodal_field.get(node) else {
                    return Err(WaveError::InvalidState);
                };
                value += basis * field;
            }
            let complementary = point
                .complementary_weights
                .iter()
                .zip(sample_fields)
                .fold(Point2::default(), |sum, (weight, field)| {
                    sum + field * *weight
                });
            primary_integral += point.physical_weight * value;
            primary_squared += point.physical_weight * value * value;
            complementary_squared += point.physical_weight * complementary.dot(complementary);
        }

        // A node's stored energy splits exactly over the materials meeting
        // there: with `Q = Σ m_c ḡ_c(U) U`, it is `Σ m_c (ḡ_c(U) U² − G_c(U))`,
        // `G_c` the unit law's co-energy. On a linear contribution that is
        // `½ m_c U²`, the mass-weighted share it always was.
        let mut energy = sample_energy;
        for (local, reference) in contribution.node_references.into_iter().enumerate() {
            let node = element.nodes[local] as usize;
            let Some(field) = nodal_field.get(node) else {
                return Err(WaveError::InvalidState);
            };
            let sample = compiled.primary[local];
            let coefficient = reference * sample.factor_at(time, runtime)?;
            let r = field.abs();
            let law = sample.law.field;
            energy += coefficient * (law.multiplier(r) * r * r - law.coenergy(r));
        }
        total_energy += contribution.covered_fraction * energy;
    }

    let result = CanonicalAreaSample {
        mean_primary: primary_integral / stencil.covered_area,
        rms_primary: (primary_squared / stencil.covered_area).max(0.0).sqrt(),
        rms_complementary: (complementary_squared / stencil.covered_area)
            .max(0.0)
            .sqrt(),
        mean_energy_density: total_energy / stencil.covered_area,
        total_energy,
        covered_area: stencil.covered_area,
        coverage: (stencil.covered_area / stencil.target_area).clamp(0.0, 1.0),
    };
    if [
        result.mean_primary,
        result.rms_primary,
        result.rms_complementary,
        result.mean_energy_density,
        result.total_energy,
    ]
    .into_iter()
    .all(f64::is_finite)
    {
        Ok(result)
    } else {
        Err(WaveError::InvalidState)
    }
}

impl From<TemporalLossSample> for CanonicalTemporalLossSample {
    fn from(sample: TemporalLossSample) -> Self {
        Self {
            material: sample.material,
            coordinates: sample.coordinates,
            drive: sample.drive,
            base_rate: sample.base_rate,
            law: sample.law,
        }
    }
}

impl From<TemporalCoefficientSample> for CanonicalTemporalCoefficientSample {
    fn from(sample: TemporalCoefficientSample) -> Self {
        Self {
            material: sample.material,
            coordinates: sample.coordinates,
            drive: sample.drive,
            law: sample.law,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalTemporalLossRates {
    pub primary: Vec<f64>,
    pub complementary: Vec<f64>,
}

/// The time-driven and field-dependent f64 oracle. The kick/drift split is
/// unchanged by a field law, because the Hamiltonian stays separable,
/// `H_Q(Q, t) + H_b(b, t)`: only the two observables `U(Q)` and `v(b)` become
/// inverses of nonlinear maps.
/// One loss contribution or sample as the device packs it: the channel's base
/// rate and its drive, evaluated where the solver evaluates it. `drive` names
/// the runtime lane its carrier phase lives in; `None` is a site no channel
/// reaches, whose rate is zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalTemporalLossSample {
    pub material: MaterialId,
    pub coordinates: MaterialCoordinates,
    pub drive: Option<CanonicalMaterialDrive>,
    pub base_rate: f64,
    pub law: DampingLawValues,
}

/// How far one field-dependent material has moved from its small-signal
/// response: the largest `ḡ(|field|) − 1` over its sites on each row. For Kerr
/// that is `χ|u|²`, the relative change of the coefficient itself; a row
/// without a field law reads zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalNonlinearStrength {
    pub material: MaterialId,
    pub primary: f64,
    pub complementary: f64,
}

/// What sets a time-driven generation's timestep ceiling. `trajectory` is
/// `fixed · √(primary_floor · complementary_floor)`, each floor the lowest
/// tangent factor that row reaches over every drive phase, Switch state and
/// admitted amplitude. A floor of one leaves the fixed medium's ceiling; a
/// self-focusing field law never lowers it, because it only slows the wave.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalTimeStepBound {
    pub fixed: f64,
    pub trajectory: f64,
    pub primary_floor: f64,
    pub complementary_floor: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalTemporalWaveOperator {
    /// Shared, because an application that assembled this base through its own
    /// resumable job holds it too and must not carry a second copy.
    base: Arc<CanonicalWaveOperator>,
    primary: Vec<TemporalPrimarySample>,
    complementary: Vec<TemporalComplementarySample>,
    initial_runtime: CanonicalMaterialRuntimeState,
    has_temporal_laws: bool,
    /// Whether any sample's coefficient follows its own field. Such a
    /// generation inverts its constitutive maps by the bracketed solve at
    /// every stage; without one, every map is the linear division it was.
    has_field_laws: bool,
    /// Primary contributions grouped by node, in contribution order, so each
    /// node's assembled nonlinear map is one contiguous run of terms. Empty
    /// when no law follows the field.
    node_contribution_offsets: Vec<usize>,
    node_contributions: Vec<u32>,
    has_loss: bool,
    conservative_bulk_supported: bool,
    indicator_supplement_supported: bool,
    forced_composition_supported: bool,
    maximum_time_step: f64,
    primary_floor: f64,
    complementary_floor: f64,
    /// Whether any contribution carries a restoring law, so the state holds
    /// the integrated field `r = ∫u dt` and the kick feels `−Σ m₀ V′(r)`.
    has_restoring: bool,
    /// Whether any primary contribution carries a van der Pol channel, whose
    /// rate follows the field and is stepped by its exact node map.
    has_active_loss: bool,
}

impl CanonicalTemporalWaveOperator {
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
        let mut stripped = model.to_owned();
        strip_temporal_laws(&mut stripped.materials);
        let base = CanonicalWaveOperator::compile(
            mesh,
            quadratic,
            stripped.as_model(),
            constitutive_revision,
        )?;
        Self::from_base(Arc::new(base), mesh, quadratic, model)
    }

    /// The law samples alone, over a base someone else has already compiled.
    ///
    /// The base must come from this model with its temporal laws stripped,
    /// which is what [`Self::compile`] does before calling this. An
    /// application whose assembly is incremental builds that base through its
    /// own resumable job and would otherwise compile it twice: once for the
    /// solver and once inside here. Everything this adds is per-sample law
    /// evaluation with no linear algebra, so it is cheap next to the assembly
    /// it reuses.
    ///
    /// Splitting it out is also what lets an application hold a law-carrying
    /// document at all. The fixed compiler refuses one - that is the gate
    /// stopping a law from being executed as a static medium - so the stripped
    /// model is not an optimization there, it is the only thing that compiles.
    pub fn from_base(
        base: Arc<CanonicalWaveOperator>,
        mesh: &TriMesh,
        quadratic: &QuadraticWaveOperator,
        model: TopologyWaveModel<'_>,
    ) -> Result<Self, WaveError> {
        let authored_owned = model.to_owned();
        let authored = authored_owned.as_model();
        let mut primary = Vec::with_capacity(base.primary_contributions().len());
        let mut complementary = Vec::with_capacity(base.constitutive_samples().len());
        let mut used_materials = BTreeSet::new();
        let mut has_temporal_laws = false;

        for contribution in base.primary_contributions() {
            let triangle =
                mesh.triangles
                    .get(contribution.element as usize)
                    .ok_or(WaveError::InvalidMesh(
                        "a canonical contribution has no element",
                    ))?;
            let point = base.node_points()[contribution.node as usize];
            let sample = temporal_material_sample(authored, triangle.region, point, true)?;
            has_temporal_laws |= sample.coefficient.law.drive != TimeDriveValues::None
                || sample.coefficient.law.alternate.is_some()
                || sample.loss.law.drive != TimeDriveValues::None;
            used_materials.insert(sample.coefficient.material);
            primary.push(TemporalPrimarySample {
                coefficient: sample.coefficient,
                loss: sample.loss,
                restoring: sample.restoring,
            });
        }
        for sample in base.constitutive_samples() {
            let triangle = mesh
                .triangles
                .get(sample.element as usize)
                .ok_or(WaveError::InvalidMesh("a canonical sample has no element"))?;
            let temporal =
                temporal_material_sample(authored, triangle.region, sample.point, false)?;
            has_temporal_laws |= temporal.coefficient.law.drive != TimeDriveValues::None
                || temporal.coefficient.law.alternate.is_some()
                || temporal.loss.law.drive != TimeDriveValues::None;
            used_materials.insert(temporal.coefficient.material);
            complementary.push(TemporalComplementarySample {
                coefficient: temporal.coefficient,
                loss: temporal.loss,
            });
        }
        let has_field_laws = primary
            .iter()
            .map(|sample| sample.coefficient)
            .chain(complementary.iter().map(|sample| sample.coefficient))
            .any(|coefficient| coefficient.law.field != FieldLawValues::Linear);
        let (node_contribution_offsets, node_contributions) = if has_field_laws {
            nonlinear_admission(&base, &primary, &complementary, authored)?;
            group_by_node(base.primary_contributions(), base.degrees_of_freedom())
        } else {
            (Vec::new(), Vec::new())
        };
        let initial_runtime = CanonicalMaterialRuntimeState::authored(
            authored
                .materials
                .iter()
                .filter(|material| used_materials.contains(&material.id))
                .cloned(),
        )?;
        let has_loss = primary.iter().any(|sample| sample.loss.base_rate != 0.0)
            || complementary
                .iter()
                .any(|sample| sample.loss.base_rate != 0.0);
        let minimum_primary_factor = primary
            .iter()
            .filter_map(|sample| sample.coefficient.law.tangent_range().map(|range| range.0))
            .fold(f64::INFINITY, f64::min);
        let minimum_complementary_factor = complementary
            .iter()
            .filter_map(|sample| sample.coefficient.law.tangent_range().map(|range| range.0))
            .fold(f64::INFINITY, f64::min);
        // A row with no samples leaves the other to set the bound alone.
        let primary_floor = if minimum_primary_factor.is_finite() {
            minimum_primary_factor
        } else {
            1.0
        };
        let complementary_floor = if minimum_complementary_factor.is_finite() {
            minimum_complementary_factor
        } else {
            1.0
        };
        let maximum_time_step = base.maximum_time_step()
            * (minimum_primary_factor * minimum_complementary_factor).sqrt();
        // Gate O: Verlet on `M⁻¹K + V″` is stable for `h²(λ + V″)/4 ≤ 1`.
        // The restoring force uses the authored mass while the step divides by
        // the mass in force, so its curvature is scaled by the lowest mass
        // factor the trajectory reaches.
        let has_restoring = primary.iter().any(|sample| !sample.restoring.is_none());
        let has_active_loss = primary
            .iter()
            .any(|sample| matches!(sample.loss.law.rate, RateLawValues::VanDerPol { .. }));
        if has_active_loss && has_field_laws {
            return Err(WaveError::Unsupported(
                "van der Pol's node map assumes a linear primary map; it does not compose \
                 with a field law yet",
            ));
        }
        let restoring_curvature = primary
            .iter()
            .map(|sample| sample.restoring.curvature_bound())
            .fold(0.0_f64, f64::max)
            / primary_floor_of(minimum_primary_factor);
        let maximum_time_step = if restoring_curvature > 0.0 {
            1.0 / (1.0 / (maximum_time_step * maximum_time_step) + 0.25 * restoring_curvature)
                .sqrt()
        } else {
            maximum_time_step
        };
        if !maximum_time_step.is_finite() || maximum_time_step <= 0.0 {
            return Err(WaveError::Unsupported(
                "a driven medium's coefficient trajectory leaves no usable timestep",
            ));
        }
        // This reference state deliberately covers only the freely evolving
        // bulk Poisson system. Open boundaries, imposed fields, sources,
        // losses and auxiliary memories keep their existing passive or
        // transactional compositions; they do not justify a global solve just
        // to attach a full-system symplectic label.
        // What the stepper can compose, and what the conservative-bulk claim
        // covers, are two different questions. Prescribed data and sources are
        // stepped exactly - each is an accounted exchange lane of its own - but
        // they put energy in and take it out, so the freely evolving Poisson
        // system is no longer the whole story and the bulk claim has to
        // exclude them. Loss, thin gaps, boundary damping and open boundaries
        // are excluded from both until each closes its own gate.
        // First-order outgoing is admitted here; it is a local damping term in
        // the kick and nothing more. The second-order boundary carries pole
        // currents of its own and stays refused until that state exists on
        // this path, as do thin gaps.
        // Thin gaps are admitted: a gap is a local spring with its own
        // displacement, which the specification calls a cheap exact local
        // split, and it carries its own stored energy into the balance.
        // Every boundary capability composes now. What is left in the bulk
        // claim is the absence of each, not the inability to run any. The one
        // combination still refused is prescribed data sitting on an outgoing
        // trace, which is its own composition and has had no tests.
        let open = base.outgoing_boundary().is_some();
        let prescribed_on_trace = base.outgoing_boundary().is_some_and(|boundary| {
            boundary
                .trace_nodes()
                .iter()
                .any(|node| quadratic.dirichlet_signals()[*node as usize].is_some())
        });
        let passive_composition = !prescribed_on_trace;
        let ungapped = base.thin_gap_samples().is_empty();
        let undamped_boundary = base
            .first_order_boundary_damping()
            .iter()
            .all(|value| *value == 0.0);
        let undriven_boundary = quadratic.dirichlet_signals().iter().all(Option::is_none)
            && quadratic
                .normalized_neumann_weights()
                .iter()
                .flatten()
                .all(|weight| *weight == 0.0)
            && quadratic
                .face_neumann_loads()
                .iter()
                .flatten()
                .all(|load| load.normalized_weight == 0.0);
        let forced_composition_supported = passive_composition;
        let conservative_bulk_supported =
            !open && ungapped && undamped_boundary && !has_loss && undriven_boundary;
        // What the error estimate's defect terms cover. Open boundaries and
        // thin gaps are in, because their defects are the fixed path's own with
        // the instantaneous mass and force in place of the authored ones. Loss,
        // a damped boundary and prescribed boundary data are not: each puts a
        // term in the evolution that the defect would otherwise charge to the
        // mesh, and none has been derived here.
        // A field-dependent medium's estimator reads its nonlinear observables
        // and weighs every defect by the tangent maps at the snapshot.
        // An oscillator medium's restoring store and trace force are in the
        // estimate (Gate O); van der Pol is a loss channel, refused with loss.
        let indicator_supplement_supported = undamped_boundary && !has_loss && undriven_boundary;
        Ok(Self {
            base,
            primary,
            complementary,
            initial_runtime,
            has_temporal_laws,
            has_field_laws,
            node_contribution_offsets,
            node_contributions,
            has_loss,
            conservative_bulk_supported,
            indicator_supplement_supported,
            forced_composition_supported,
            maximum_time_step,
            primary_floor,
            complementary_floor,
            has_restoring,
            has_active_loss,
        })
    }

    /// Whether the generation carries a restoring law (Gate O).
    pub fn has_restoring(&self) -> bool {
        self.has_restoring
    }

    /// Each node's rate as `β + α u²`, mass-weighted over its contributions
    /// at `time`, with the mass it is weighed by: a constant channel adds its
    /// rate to `β`; a van der Pol one `−base` to `β` and `base/a²` to `α`.
    fn active_loss_coefficients(
        &self,
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<ActiveLossCoefficients, WaveError> {
        let count = self.base.degrees_of_freedom();
        let (mut mass, mut beta, mut alpha) =
            (vec![0.0; count], vec![0.0; count], vec![0.0; count]);
        for (contribution, sample) in self.base.primary_contributions().iter().zip(&self.primary) {
            let share = contribution.geometric_weight
                * contribution.reference_coefficient
                * coefficient_factor(sample.coefficient, time, runtime)?;
            let node = contribution.node as usize;
            mass[node] += share;
            match sample.loss.law.rate {
                RateLawValues::VanDerPol { threshold, .. } => {
                    beta[node] -= share * sample.loss.base_rate;
                    alpha[node] += share * sample.loss.base_rate / (threshold * threshold);
                }
                _ => beta[node] += share * loss_rate(sample.loss, time, runtime)?,
            }
        }
        validate_positive(&mass)?;
        for node in 0..count {
            beta[node] /= mass[node];
            alpha[node] /= mass[node];
        }
        Ok((beta, alpha, mass))
    }

    /// `R_i(r) = Σ_c m₀_c V_c′(r_i)`: each contribution's restoring force on
    /// the integrated field, at its authored lumped mass. A node past a law's
    /// declared amplitude bound refuses the step, as a field law's inverse
    /// does; nothing is clipped.
    fn restoring_force(&self, integrated: &[f64]) -> Result<Vec<f64>, WaveError> {
        let mut force = vec![0.0; self.base.degrees_of_freedom()];
        if !self.has_restoring {
            return Ok(force);
        }
        for (contribution, sample) in self.base.primary_contributions().iter().zip(&self.primary) {
            if sample.restoring.is_none() {
                continue;
            }
            let node = contribution.node as usize;
            let r = integrated[node];
            if sample
                .restoring
                .amplitude_bound()
                .is_some_and(|bound| r.abs() > bound)
            {
                return Err(WaveError::Unsupported(
                    "the integrated field passed its restoring law's amplitude bound",
                ));
            }
            let mass = contribution.geometric_weight * contribution.reference_coefficient;
            force[node] += mass * sample.restoring.slope(r);
        }
        validate_finite(&force)?;
        Ok(force)
    }

    /// `Σ_i Σ_c m₀_c V_c(r_i)`, the restoring store.
    fn restoring_energy(&self, integrated: &[f64]) -> f64 {
        if !self.has_restoring {
            return 0.0;
        }
        self.base
            .primary_contributions()
            .iter()
            .zip(&self.primary)
            .map(|(contribution, sample)| {
                contribution.geometric_weight
                    * contribution.reference_coefficient
                    * sample
                        .restoring
                        .potential(integrated[contribution.node as usize])
            })
            .sum()
    }

    /// The timestep ceiling and what lowers it below the fixed medium's.
    pub fn time_step_bound(&self) -> CanonicalTimeStepBound {
        CanonicalTimeStepBound {
            fixed: self.base.maximum_time_step(),
            trajectory: self.maximum_time_step,
            primary_floor: self.primary_floor,
            complementary_floor: self.complementary_floor,
        }
    }

    /// Each field-dependent material's peak response at the given fluxes,
    /// read through the maps the solver inverts: the nodal field for the
    /// primary row, each sample's complementary field for the other. Materials
    /// without a field law are left out.
    pub fn nonlinear_strength(
        &self,
        primary_flux: &[f64],
        complementary_flux: &[Point2],
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<Vec<CanonicalNonlinearStrength>, WaveError> {
        let mut strengths: Vec<CanonicalNonlinearStrength> = Vec::new();
        fn slot(
            strengths: &mut Vec<CanonicalNonlinearStrength>,
            material: MaterialId,
        ) -> &mut CanonicalNonlinearStrength {
            let index = match strengths
                .iter()
                .position(|found| found.material == material)
            {
                Some(index) => index,
                None => {
                    strengths.push(CanonicalNonlinearStrength {
                        material,
                        primary: 0.0,
                        complementary: 0.0,
                    });
                    strengths.len() - 1
                }
            };
            &mut strengths[index]
        }
        if !self.has_field_laws {
            return Ok(Vec::new());
        }
        let field = self.primary_field_at(primary_flux, time, runtime)?;
        for (node, value) in field.iter().enumerate() {
            for index in &self.node_contributions[self.primary_range(node)] {
                let coefficient = self.primary[*index as usize].coefficient;
                if coefficient.law.field == FieldLawValues::Linear {
                    continue;
                }
                let response = coefficient.law.field.multiplier(value.abs()) - 1.0;
                let found = slot(&mut strengths, coefficient.material);
                found.primary = found.primary.max(response);
            }
        }
        let fields = self.complementary_field_at(complementary_flux, time, runtime)?;
        for (sample, value) in self.complementary.iter().zip(&fields) {
            let coefficient = sample.coefficient;
            if coefficient.law.field == FieldLawValues::Linear {
                continue;
            }
            let response = coefficient.law.field.multiplier(value.norm()) - 1.0;
            let found = slot(&mut strengths, coefficient.material);
            found.complementary = found.complementary.max(response);
        }
        Ok(strengths)
    }

    pub fn base(&self) -> &CanonicalWaveOperator {
        &self.base
    }

    pub fn initial_runtime(&self) -> CanonicalMaterialRuntimeState {
        self.initial_runtime.clone()
    }

    pub fn has_temporal_laws(&self) -> bool {
        self.has_temporal_laws
    }

    pub fn has_loss(&self) -> bool {
        self.has_loss
    }

    /// Whether any coefficient follows its own field, so that the maps are
    /// inverted by the bracketed solve. The device does not execute such a
    /// generation yet.
    pub fn has_field_laws(&self) -> bool {
        self.has_field_laws
    }

    /// Whether the exact kick/drift bulk split can run without composing any
    /// lossy, forced, prescribed-boundary, or auxiliary subsystem.
    pub fn conservative_bulk_supported(&self) -> bool {
        self.conservative_bulk_supported
    }

    /// Whether [`canonical_temporal_indicator_supplement`] covers this
    /// generation's defect terms.
    pub fn indicator_supplement_supported(&self) -> bool {
        self.indicator_supplement_supported
    }

    /// Whether the stepper can compose prescribed data, volume sources and
    /// loss with this generation. Weaker than
    /// [`Self::conservative_bulk_supported`], which additionally requires that
    /// nothing drives the boundary and that nothing dissipates, because an
    /// accounted exchange lane is stepped without being part of the freely
    /// evolving system.
    pub fn forced_composition_supported(&self) -> bool {
        self.forced_composition_supported
    }

    /// Conservative spatial CFL bound over the entire authored coefficient
    /// trajectory. Resolving a temporal carrier is a separate admission gate.
    pub fn maximum_time_step(&self) -> f64 {
        self.maximum_time_step
    }

    /// The step a caller should actually take, carrying the fixed path's
    /// safety margin onto the trajectory bound.
    ///
    /// A driven generation's ceiling is the tighter of the two, because the
    /// bound covers every phase the coefficients reach rather than one set of
    /// them. That is where a drive's cost lands: measured on the GPU core it
    /// is free per step and about `1.23x` per simulated second, and this is
    /// the factor.
    pub fn recommended_time_step(&self) -> f64 {
        let base = self.base.maximum_time_step();
        if base > 0.0 {
            self.base.recommended_time_step() * (self.maximum_time_step / base)
        } else {
            self.maximum_time_step
        }
    }

    pub fn primary_coefficient_samples(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalTemporalCoefficientSample> + '_ {
        self.primary.iter().map(|sample| sample.coefficient.into())
    }

    pub fn complementary_coefficient_samples(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalTemporalCoefficientSample> + '_ {
        self.complementary
            .iter()
            .map(|sample| sample.coefficient.into())
    }

    /// Each primary contribution's loss, in contribution order.
    pub fn primary_loss_samples(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalTemporalLossSample> + '_ {
        self.primary.iter().map(|sample| sample.loss.into())
    }

    /// Each complementary sample's loss, in sample order.
    pub fn complementary_loss_samples(
        &self,
    ) -> impl ExactSizeIterator<Item = CanonicalTemporalLossSample> + '_ {
        self.complementary.iter().map(|sample| sample.loss.into())
    }

    pub fn primary_mass_at(
        &self,
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<Vec<f64>, WaveError> {
        self.primary_mass_and_rate_at(time, runtime)
            .map(|(mass, _)| mass)
    }

    pub fn primary_mass_and_rate_at(
        &self,
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<(Vec<f64>, Vec<f64>), WaveError> {
        if !self.has_temporal_laws && !self.has_field_laws {
            return Ok((
                self.base.primary_mass().to_vec(),
                vec![0.0; self.base.degrees_of_freedom()],
            ));
        }
        let mut mass = vec![0.0; self.base.degrees_of_freedom()];
        let mut rate = vec![0.0; self.base.degrees_of_freedom()];
        for (contribution, temporal) in self.base.primary_contributions().iter().zip(&self.primary)
        {
            let (factor, factor_rate) =
                coefficient_factor_and_rate(temporal.coefficient, time, runtime)?;
            let reference = contribution.geometric_weight * contribution.reference_coefficient;
            mass[contribution.node as usize] += reference * factor;
            rate[contribution.node as usize] += reference * factor_rate;
        }
        validate_positive(&mass)?;
        validate_finite(&rate)?;
        Ok((mass, rate))
    }

    pub fn primary_field_at(
        &self,
        primary_flux: &[f64],
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<Vec<f64>, WaveError> {
        if !self.has_temporal_laws && !self.has_field_laws {
            return self.base.primary_field(primary_flux);
        }
        if primary_flux.len() != self.base.degrees_of_freedom() {
            return Err(WaveError::SizeMismatch {
                expected: self.base.degrees_of_freedom(),
                actual: primary_flux.len(),
            });
        }
        if self.has_field_laws {
            let (terms, _) = self.primary_terms_at(time, runtime)?;
            let field = primary_flux
                .iter()
                .enumerate()
                .map(|(node, flux)| self.primary_inverse(&terms, node, *flux, runtime))
                .collect::<Result<Vec<_>, _>>()?;
            validate_finite(&field)?;
            return Ok(field);
        }
        let mass = self.primary_mass_at(time, runtime)?;
        let field = primary_flux
            .iter()
            .zip(mass)
            .map(|(flux, mass)| flux / mass)
            .collect::<Vec<_>>();
        validate_finite(&field)?;
        Ok(field)
    }

    /// Every primary contribution's constitutive term at `time`, in the
    /// node-grouped order of `node_contributions`, with each coefficient's
    /// explicit time derivative beside it.
    fn primary_terms_at(
        &self,
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<(Vec<ConstitutiveTerm>, Vec<f64>), WaveError> {
        let contributions = self.base.primary_contributions();
        let mut terms = Vec::with_capacity(self.node_contributions.len());
        let mut rates = Vec::with_capacity(self.node_contributions.len());
        for index in &self.node_contributions {
            let contribution = &contributions[*index as usize];
            let temporal = &self.primary[*index as usize];
            let (factor, factor_rate) =
                coefficient_factor_and_rate(temporal.coefficient, time, runtime)?;
            let reference = contribution.geometric_weight * contribution.reference_coefficient;
            terms.push(ConstitutiveTerm {
                coefficient: reference * factor,
                law: temporal.coefficient.law.field,
            });
            rates.push(reference * factor_rate);
        }
        Ok((terms, rates))
    }

    fn primary_range(&self, node: usize) -> std::ops::Range<usize> {
        self.node_contribution_offsets[node]..self.node_contribution_offsets[node + 1]
    }

    /// The field at one node from its flux, through the node's assembled map.
    fn primary_inverse(
        &self,
        terms: &[ConstitutiveTerm],
        node: usize,
        flux: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<f64, WaveError> {
        let range = self.primary_range(node);
        let site = ConstitutiveSite::new(&terms[range.clone()]);
        signed_inverse(site, flux).map_err(|error| {
            let contribution = self.node_contributions[range.start] as usize;
            let coefficient = self.primary[contribution].coefficient;
            inverse_error(error, coefficient, runtime)
        })
    }

    /// The flux a node holds at `field`, and the energy it stores there: the
    /// forward map a prescribed value or a field pulse is written through.
    ///
    /// A field past a declared amplitude bound is refused, as the inverse
    /// refuses the flux that would hold it.
    fn primary_flux_and_energy_of_field(
        &self,
        terms: &[ConstitutiveTerm],
        node: usize,
        field: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<(f64, f64), WaveError> {
        let range = self.primary_range(node);
        let site = ConstitutiveSite::new(&terms[range.clone()]);
        if !field.is_finite()
            || site
                .amplitude_bound()
                .is_some_and(|bound| field.abs() > bound)
        {
            let contribution = self.node_contributions[range.start] as usize;
            return Err(inverse_error(
                ConstitutiveInverseError::OutsideDomain,
                self.primary[contribution].coefficient,
                runtime,
            ));
        }
        let magnitude = site.value(field.abs());
        Ok((
            magnitude.copysign(field),
            site.energy(magnitude, field.abs()),
        ))
    }

    /// Energy stored at one node holding `flux`.
    fn primary_node_energy(
        &self,
        terms: &[ConstitutiveTerm],
        node: usize,
        flux: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<f64, WaveError> {
        let field = self.primary_inverse(terms, node, flux, runtime)?;
        let site = ConstitutiveSite::new(&terms[self.primary_range(node)]);
        Ok(site.energy(flux.abs(), field.abs()))
    }

    /// The discrete gradient `ū = [T(new) − T(old)]/(new − old)` of one
    /// node's stored energy and its slope `dū/d new`.
    ///
    /// The quotient cancels as `new → old`, so below a relative separation
    /// of `1e-4` the mean of `U` over the interval is taken by four-point
    /// Gauss-Legendre instead. That is the same integral, to an error many
    /// orders below roundoff at that separation. The slope there is its limit,
    /// `1/(2P′)` at the midpoint.
    fn primary_discrete_gradient(
        &self,
        terms: &[ConstitutiveTerm],
        node: usize,
        old: f64,
        new: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<(f64, f64), WaveError> {
        let site = ConstitutiveSite::new(&terms[self.primary_range(node)]);
        let separation = new - old;
        if separation.abs() > 1e-4 * old.abs().max(new.abs()) {
            let gradient = (self.primary_node_energy(terms, node, new, runtime)?
                - self.primary_node_energy(terms, node, old, runtime)?)
                / separation;
            let field = self.primary_inverse(terms, node, new, runtime)?;
            return Ok((gradient, (field - gradient) / separation));
        }
        const POINTS: [(f64, f64); 4] = [
            (-0.861_136_311_594_052_6, 0.347_854_845_137_453_9),
            (-0.339_981_043_584_856_3, 0.652_145_154_862_546_1),
            (0.339_981_043_584_856_3, 0.652_145_154_862_546_1),
            (0.861_136_311_594_052_6, 0.347_854_845_137_453_9),
        ];
        let middle = 0.5 * (old + new);
        let mut gradient = 0.0;
        for (point, weight) in POINTS {
            gradient += 0.5
                * weight
                * self.primary_inverse(terms, node, middle + 0.5 * point * separation, runtime)?;
        }
        let field = self.primary_inverse(terms, node, middle, runtime)?;
        Ok((gradient, 0.5 / site.tangent(field.abs())))
    }

    /// `Q` solving `Q − old = impulse − τd·ū(old, Q)` at one absorbing node:
    /// the discrete-gradient first-order wall. `f(Q)` rises with slope at
    /// least one, so every trial `x` brackets the root with `x − f(x)`.
    fn damped_nonlinear_kick(
        &self,
        terms: &[ConstitutiveTerm],
        node: usize,
        old: f64,
        impulse: f64,
        admittance: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<f64, WaveError> {
        let residual = |flux: f64| -> Result<(f64, f64), WaveError> {
            let (gradient, slope) =
                self.primary_discrete_gradient(terms, node, old, flux, runtime)?;
            Ok((
                flux - old - impulse + admittance * gradient,
                1.0 + admittance * slope,
            ))
        };
        let mut flux = old + impulse;
        let (value, _) = residual(flux)?;
        let (mut low, mut high) = if value > 0.0 {
            (flux - value, flux)
        } else {
            (flux, flux - value)
        };
        let scale = old.abs().max(impulse.abs()).max(f64::MIN_POSITIVE);
        for _ in 0..100 {
            let (value, slope) = residual(flux)?;
            if value == 0.0 {
                return Ok(flux);
            }
            if value > 0.0 {
                high = high.min(flux);
            } else {
                low = low.max(flux);
            }
            let newton = flux - value / slope;
            let next = if newton > low && newton < high {
                newton
            } else {
                0.5 * (low + high)
            };
            if (next - flux).abs() <= 1e-13 * scale.max(next.abs()) {
                return Ok(next);
            }
            flux = next;
        }
        Err(WaveError::Unsupported(
            "the nonlinear absorbing-wall kick did not converge",
        ))
    }

    /// `∂U/∂Q = 1/P′(U)` at every node, for the field `field` already
    /// reconstructed from the flux.
    fn primary_tangent_inverse_at(
        &self,
        field: &[f64],
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<Vec<f64>, WaveError> {
        let (terms, _) = self.primary_terms_at(time, runtime)?;
        let tangent = field
            .iter()
            .enumerate()
            .map(|(node, field)| {
                1.0 / ConstitutiveSite::new(&terms[self.primary_range(node)]).tangent(field.abs())
            })
            .collect::<Vec<_>>();
        validate_positive(&tangent)?;
        Ok(tangent)
    }

    /// `J_b = ∂v/∂b` at every sample and the current flux.
    ///
    /// For the radial map `|b| = c′ ḡ(r) r` it is
    /// `(r/|b|)(I − b̂b̂ᵀ) + b̂b̂ᵀ / (c′ (ḡ + rḡ′))`: the secant response across
    /// the field and the radial tangent along it. A linear sample's is its
    /// `J / factor`.
    fn complementary_tangents_at(
        &self,
        complementary_flux: &[Point2],
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<Vec<SymmetricTensor2>, WaveError> {
        let mut tangents = Vec::with_capacity(complementary_flux.len());
        for (index, ((base, temporal), flux)) in self
            .base
            .constitutive_samples()
            .iter()
            .zip(&self.complementary)
            .zip(complementary_flux)
            .enumerate()
        {
            if temporal.coefficient.law.field == FieldLawValues::Linear {
                let factor = coefficient_factor(temporal.coefficient, time, runtime)?;
                let inverse = base.complementary_inverse;
                tangents.push(SymmetricTensor2::new(
                    inverse.xx / factor,
                    inverse.xy / factor,
                    inverse.yy / factor,
                ));
                continue;
            }
            let (term, _) = self.complementary_term_at(index, time, runtime)?;
            let terms = [term];
            let site = ConstitutiveSite::new(&terms);
            let magnitude = flux.norm();
            let radius = radial_inverse(term, *flux, temporal.coefficient, runtime)?.norm();
            let radial = 1.0 / site.tangent(radius);
            if magnitude == 0.0 {
                tangents.push(SymmetricTensor2::isotropic(radial));
                continue;
            }
            let secant = radius / magnitude;
            let (x, y) = (flux.x / magnitude, flux.y / magnitude);
            tangents.push(SymmetricTensor2::new(
                secant + (radial - secant) * x * x,
                (radial - secant) * x * y,
                secant + (radial - secant) * y * y,
            ));
        }
        Ok(tangents)
    }

    /// The physical complementary field at one sample holding `flux`.
    pub fn complementary_sample_field(
        &self,
        index: usize,
        flux: Point2,
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<Point2, WaveError> {
        let sample = self
            .base
            .constitutive_samples()
            .get(index)
            .ok_or(WaveError::InvalidState)?;
        let temporal = self.complementary[index].coefficient;
        if temporal.law.field == FieldLawValues::Linear {
            let factor = coefficient_factor(temporal, time, runtime)?;
            return Ok(sample.complementary_inverse.apply(flux) / factor);
        }
        let (term, _) = self.complementary_term_at(index, time, runtime)?;
        radial_inverse(term, flux, temporal, runtime)
    }

    /// Stored energy of one sample holding `flux`, weight included.
    fn complementary_sample_energy(
        &self,
        index: usize,
        flux: Point2,
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<f64, WaveError> {
        let sample = &self.base.constitutive_samples()[index];
        let temporal = self.complementary[index].coefficient;
        if temporal.law.field == FieldLawValues::Linear {
            let factor = coefficient_factor(temporal, time, runtime)?;
            return Ok(0.5
                * sample.integration_weight
                * flux.dot(sample.complementary_inverse.apply(flux))
                / factor);
        }
        let (term, _) = self.complementary_term_at(index, time, runtime)?;
        let radius = radial_inverse(term, flux, temporal, runtime)?.norm();
        let terms = [term];
        Ok(sample.integration_weight * ConstitutiveSite::new(&terms).energy(flux.norm(), radius))
    }

    /// One nonlinear complementary sample's map at `time`: the direct
    /// coefficient over the isotropic reference inverse, `|b| = (c/j)·ḡ(r)·r`,
    /// with that coefficient's time derivative.
    fn complementary_term_at(
        &self,
        index: usize,
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<(ConstitutiveTerm, f64), WaveError> {
        let temporal = self.complementary[index].coefficient;
        let (factor, factor_rate) = coefficient_factor_and_rate(temporal, time, runtime)?;
        let reference = self.base.constitutive_samples()[index]
            .complementary_inverse
            .xx;
        Ok((
            ConstitutiveTerm {
                coefficient: factor / reference,
                law: temporal.law.field,
            },
            factor_rate / reference,
        ))
    }

    pub fn complementary_field_at(
        &self,
        complementary_flux: &[Point2],
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<Vec<Point2>, WaveError> {
        if !self.has_temporal_laws && !self.has_field_laws {
            return self.base.complementary_field(complementary_flux);
        }
        if complementary_flux.len() != self.base.complementary_degrees_of_freedom() {
            return Err(WaveError::SizeMismatch {
                expected: self.base.complementary_degrees_of_freedom(),
                actual: complementary_flux.len(),
            });
        }
        let mut field = Vec::with_capacity(complementary_flux.len());
        for (index, ((base, temporal), flux)) in self
            .base
            .constitutive_samples()
            .iter()
            .zip(&self.complementary)
            .zip(complementary_flux)
            .enumerate()
        {
            if temporal.coefficient.law.field != FieldLawValues::Linear {
                let (term, _) = self.complementary_term_at(index, time, runtime)?;
                field.push(radial_inverse(term, *flux, temporal.coefficient, runtime)?);
                continue;
            }
            let factor = coefficient_factor(temporal.coefficient, time, runtime)?;
            field.push(base.complementary_inverse.apply(*flux) / factor);
        }
        if field.iter().all(|value| value.finite()) {
            Ok(field)
        } else {
            Err(WaveError::InvalidState)
        }
    }

    pub fn force_at(
        &self,
        complementary_flux: &[Point2],
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<Vec<f64>, WaveError> {
        if !self.has_temporal_laws && !self.has_field_laws {
            return self.base.force(complementary_flux);
        }
        let fields = self.complementary_field_at(complementary_flux, time, runtime)?;
        self.gather_force(&fields)
    }

    /// `Cᵀ W v`: the nodal force of a complementary field given at every
    /// sample.
    fn gather_force(&self, fields: &[Point2]) -> Result<Vec<f64>, WaveError> {
        if fields.len() != self.base.complementary_degrees_of_freedom() {
            return Err(WaveError::InvalidState);
        }
        let mut force = vec![0.0; self.base.degrees_of_freedom()];
        for (sample_index, (sample, field)) in self
            .base
            .constitutive_samples()
            .iter()
            .zip(fields.iter().copied())
            .enumerate()
        {
            let nodes = self.base.element_nodes()[sample_index / 6];
            for (node, curl) in nodes.iter().zip(sample.curls()) {
                force[*node as usize] +=
                    self.base.orientation() * sample.integration_weight * curl.dot(field);
            }
        }
        validate_finite(&force)?;
        Ok(force)
    }

    pub fn loss_rates_at(
        &self,
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<CanonicalTemporalLossRates, WaveError> {
        if !self.has_temporal_laws && !self.has_field_laws {
            return Ok(CanonicalTemporalLossRates {
                primary: self.base.primary_loss_rate().to_vec(),
                complementary: self.base.complementary_loss_rate().to_vec(),
            });
        }
        let mut mass = vec![0.0; self.base.degrees_of_freedom()];
        let mut weighted_loss = vec![0.0; self.base.degrees_of_freedom()];
        for (contribution, temporal) in self.base.primary_contributions().iter().zip(&self.primary)
        {
            let coefficient = coefficient_factor(temporal.coefficient, time, runtime)?;
            let share =
                contribution.geometric_weight * contribution.reference_coefficient * coefficient;
            let rate = loss_rate(temporal.loss, time, runtime)?;
            mass[contribution.node as usize] += share;
            weighted_loss[contribution.node as usize] += share * rate;
        }
        validate_positive(&mass)?;
        let primary = weighted_loss
            .into_iter()
            .zip(mass)
            .map(|(loss, mass)| loss / mass)
            .collect::<Vec<_>>();
        let complementary = self
            .complementary
            .iter()
            .map(|sample| loss_rate(sample.loss, time, runtime))
            .collect::<Result<Vec<_>, _>>()?;
        validate_nonnegative(&primary)?;
        validate_nonnegative(&complementary)?;
        Ok(CanonicalTemporalLossRates {
            primary,
            complementary,
        })
    }

    /// Physical bulk Hamiltonian and its explicit partial time derivative at
    /// fixed canonical state. The derivative is the power exchanged with an
    /// authored material trajectory, not a finite difference between steps.
    pub fn energy_and_rate_at(
        &self,
        primary_flux: &[f64],
        complementary_flux: &[Point2],
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<(f64, f64), WaveError> {
        if !time.is_finite() {
            return Err(WaveError::InvalidState);
        }
        let (primary_energy, primary_rate) =
            self.primary_energy_and_rate(primary_flux, time, runtime)?;
        let (complementary_energy, complementary_rate) =
            self.complementary_energy_and_rate(complementary_flux, time, runtime)?;
        let energy = primary_energy + complementary_energy;
        let rate = primary_rate + complementary_rate;
        if energy.is_finite() && rate.is_finite() {
            Ok((energy, rate))
        } else {
            Err(WaveError::InvalidState)
        }
    }

    pub fn energy_at(
        &self,
        primary_flux: &[f64],
        complementary_flux: &[Point2],
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<f64, WaveError> {
        self.energy_and_rate_at(primary_flux, complementary_flux, time, runtime)
            .map(|(energy, _)| energy)
    }

    fn primary_energy_and_rate(
        &self,
        primary_flux: &[f64],
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<(f64, f64), WaveError> {
        if primary_flux.len() != self.base.degrees_of_freedom() {
            return Err(WaveError::SizeMismatch {
                expected: self.base.degrees_of_freedom(),
                actual: primary_flux.len(),
            });
        }
        if self.has_field_laws {
            // `T = Σ ∫₀^Q U`, and at fixed Q its explicit rate is minus the
            // co-energy's: `∂T/∂t = −Σ ṁᵢ Gᵢ(U)`.
            let (terms, term_rates) = self.primary_terms_at(time, runtime)?;
            let mut energy = 0.0;
            let mut rate = 0.0;
            for (node, flux) in primary_flux.iter().enumerate() {
                let field = self.primary_inverse(&terms, node, *flux, runtime)?;
                let range = self.primary_range(node);
                let site = ConstitutiveSite::new(&terms[range.clone()]);
                energy += site.energy(flux.abs(), field.abs());
                for (term, term_rate) in terms[range.clone()].iter().zip(&term_rates[range]) {
                    rate -= term_rate * term.law.coenergy(field.abs());
                }
            }
            return if energy.is_finite() && rate.is_finite() {
                Ok((energy, rate))
            } else {
                Err(WaveError::InvalidState)
            };
        }
        let (mass, mass_rate) = self.primary_mass_and_rate_at(time, runtime)?;
        let mut energy = 0.0;
        let mut rate = 0.0;
        for ((flux, mass), mass_rate) in primary_flux.iter().zip(mass).zip(mass_rate) {
            energy += 0.5 * flux * flux / mass;
            rate -= 0.5 * flux * flux * mass_rate / (mass * mass);
        }
        if energy.is_finite() && rate.is_finite() {
            Ok((energy, rate))
        } else {
            Err(WaveError::InvalidState)
        }
    }

    /// One sample's complementary field and stored energy, its weight
    /// included, through the same map the solver inverts there.
    fn complementary_sample_field_and_energy(
        &self,
        index: usize,
        flux: Point2,
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<(Point2, f64), WaveError> {
        let base = self
            .base
            .constitutive_samples()
            .get(index)
            .ok_or(WaveError::InvalidState)?;
        let temporal = self
            .complementary
            .get(index)
            .ok_or(WaveError::InvalidState)?;
        if temporal.coefficient.law.field != FieldLawValues::Linear {
            let (term, _) = self.complementary_term_at(index, time, runtime)?;
            let field = radial_inverse(term, flux, temporal.coefficient, runtime)?;
            let site_terms = [term];
            let energy = base.integration_weight
                * ConstitutiveSite::new(&site_terms).energy(flux.norm(), field.norm());
            return Ok((field, energy));
        }
        let factor = coefficient_factor(temporal.coefficient, time, runtime)?;
        let field = base.complementary_inverse.apply(flux) / factor;
        Ok((field, 0.5 * base.integration_weight * flux.dot(field)))
    }

    fn complementary_energy_and_rate(
        &self,
        complementary_flux: &[Point2],
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<(f64, f64), WaveError> {
        if complementary_flux.len() != self.base.complementary_degrees_of_freedom() {
            return Err(WaveError::SizeMismatch {
                expected: self.base.complementary_degrees_of_freedom(),
                actual: complementary_flux.len(),
            });
        }
        let mut energy = 0.0;
        let mut rate = 0.0;
        for (index, ((base, temporal), flux)) in self
            .base
            .constitutive_samples()
            .iter()
            .zip(&self.complementary)
            .zip(complementary_flux)
            .enumerate()
        {
            if temporal.coefficient.law.field != FieldLawValues::Linear {
                let (term, term_rate) = self.complementary_term_at(index, time, runtime)?;
                let field = radial_inverse(term, *flux, temporal.coefficient, runtime)?;
                let (target, r) = (flux.norm(), field.norm());
                let site_terms = [term];
                let site = ConstitutiveSite::new(&site_terms);
                energy += base.integration_weight * site.energy(target, r);
                rate -= base.integration_weight * term_rate * term.law.coenergy(r);
                continue;
            }
            let (factor, factor_rate) =
                coefficient_factor_and_rate(temporal.coefficient, time, runtime)?;
            let reference =
                0.5 * base.integration_weight * flux.dot(base.complementary_inverse.apply(*flux));
            energy += reference / factor;
            rate -= reference * factor_rate / (factor * factor);
        }
        if energy.is_finite() && rate.is_finite() {
            Ok((energy, rate))
        } else {
            Err(WaveError::InvalidState)
        }
    }

    /// Drifts `b` on a primary field the caller has already reconstructed at
    /// the drift's own instant, prescribed values included.
    fn drift_on(
        &self,
        complementary_flux: &mut [Point2],
        primary_field: &[f64],
        duration: f64,
    ) -> Result<(), WaveError> {
        if complementary_flux.len() != self.base.complementary_degrees_of_freedom() {
            return Err(WaveError::SizeMismatch {
                expected: self.base.complementary_degrees_of_freedom(),
                actual: complementary_flux.len(),
            });
        }
        if !duration.is_finite() {
            return Err(WaveError::InvalidState);
        }
        if primary_field.len() != self.base.degrees_of_freedom() {
            return Err(WaveError::InvalidState);
        }
        for (sample_index, (sample, flux)) in self
            .base
            .constitutive_samples()
            .iter()
            .zip(complementary_flux)
            .enumerate()
        {
            let nodes = self.base.element_nodes()[sample_index / 6];
            let reference = primary_field[nodes[0] as usize];
            let mut curl = Point2::default();
            for (node, shape_curl) in nodes[1..].iter().zip(&sample.curls()[1..]) {
                curl = curl
                    + *shape_curl * stable_difference(primary_field[*node as usize], reference);
            }
            *flux = *flux + curl * (self.base.orientation() * duration);
            if !flux.finite() {
                return Err(WaveError::InvalidState);
            }
        }
        Ok(())
    }
}

/// Per-step ledger for the conservative bulk split. `temporal_work` is the
/// explicit material-pump exchange from the extended `(t, p_t)` Hamiltonian;
/// `splitting_residual` is the remaining discrete defect.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanonicalTemporalStepAccounting {
    /// Energy the medium's own modulation put into the field.
    pub temporal_work: f64,
    /// Energy volume sources put into the field.
    pub source_work: f64,
    /// Energy the primary loss channel removed. Never negative.
    pub primary_loss: f64,
    /// Energy the complementary loss channel removed. Never negative.
    pub complementary_loss: f64,
    /// Energy an absorbing wall carried out of the domain. Never negative.
    pub boundary_loss: f64,
    /// Energy that crossed a prescribed node, either sign.
    ///
    /// A prescribed node holds `Q = M(t) g(t)`, so under modulation this is
    /// nonzero even when `g` is constant: holding a field fixed while the
    /// medium's inertia breathes takes work, and it arrives from outside the
    /// domain rather than from the drive.
    pub prescribed_exchange: f64,
    /// Energy an active (van der Pol) channel put into the field, either
    /// sign: gain below its threshold, loss above it. Passive channels on the
    /// same node's other materials are counted here with it, because one map
    /// acts on the node.
    pub active_gain: f64,
    pub energy_change: f64,
    /// What the three lanes above fail to account for. The splitting's own
    /// error and nothing else.
    pub splitting_residual: f64,
}

/// Dormant Stage 7 CPU oracle for the exact bulk kick/drift subflows. The
/// autonomous extension is symplectic on every nondegenerate leaf of the bulk
/// Poisson system. Boundary, loss, source, filter and handoff compositions are
/// intentionally outside this claim.
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalTemporalWaveState {
    primary_flux: Vec<f64>,
    complementary_flux: Vec<Point2>,
    /// Pole currents for a second-order outgoing boundary, in the compiled
    /// auxiliary order. Empty on any other generation.
    outgoing_z: Vec<f64>,
    /// One integrated field jump per thin-gap sample. The gap is a spring
    /// across a trace with a displacement of its own, and it stores
    /// `stiffness * jump^2 / 2`, so it belongs to the state and to the energy
    /// rather than being reconstructible from the bulk.
    thin_gap_jump: Vec<f64>,
    /// Gate O: the integrated primary field `r = ∫u dt` per node, on a
    /// generation carrying a restoring law. Its uniform part is physical, so
    /// it is authoritative state and not reconstructed from `b`.
    integrated_field: Vec<f64>,
    runtime: CanonicalMaterialRuntimeState,
    time_step: f64,
    time: f64,
}

impl CanonicalTemporalWaveState {
    pub fn new(
        operator: &CanonicalTemporalWaveOperator,
        time_step: f64,
        primary_flux: Vec<f64>,
        complementary_flux: Vec<Point2>,
    ) -> Result<Self, WaveError> {
        Self::new_at(operator, time_step, primary_flux, complementary_flux, 0.0)
    }

    /// Constructs an accepted conservative-bulk state at an existing clock
    /// boundary. This is used by generation handoff and clock-rebase
    /// validation; the authored material runtime remains anchored to absolute
    /// time until an explicit accepted event changes it.
    pub fn new_at(
        operator: &CanonicalTemporalWaveOperator,
        time_step: f64,
        primary_flux: Vec<f64>,
        complementary_flux: Vec<Point2>,
        time: f64,
    ) -> Result<Self, WaveError> {
        // A state exists wherever the stepper can compose, which is now wider
        // than the free bulk: loss, prescribed data, sources, an absorbing
        // wall and thin gaps are accounted lanes or local states of their own.
        // What the bulk claim still refuses - a second-order open boundary -
        // has no state here either.
        if !operator.forced_composition_supported() {
            return Err(WaveError::Unsupported(
                "the time-driven operator cannot compose a forced step for this scene",
            ));
        }
        if !time_step.is_finite() || time_step <= 0.0 {
            return Err(WaveError::Unsupported(
                "the time-driven state was given a time step that is not positive and finite",
            ));
        }
        if time_step > operator.maximum_time_step() {
            return Err(WaveError::Unsupported(
                "the time step exceeds what the time-driven trajectory holds stable",
            ));
        }
        if !time.is_finite() {
            return Err(WaveError::Unsupported(
                "the time-driven state was given a start time that is not finite",
            ));
        }
        if primary_flux.len() != operator.base().degrees_of_freedom() {
            return Err(WaveError::SizeMismatch {
                expected: operator.base().degrees_of_freedom(),
                actual: primary_flux.len(),
            });
        }
        if complementary_flux.len() != operator.base().complementary_degrees_of_freedom() {
            return Err(WaveError::SizeMismatch {
                expected: operator.base().complementary_degrees_of_freedom(),
                actual: complementary_flux.len(),
            });
        }
        validate_finite(&primary_flux)?;
        if complementary_flux.iter().any(|value| !value.finite()) {
            return Err(WaveError::InvalidState);
        }
        if operator.has_field_laws() {
            // A state has to be one the maps can hold: a flux past a declared
            // amplitude bound has no field, and is refused here rather than
            // on the first step.
            let runtime = operator.initial_runtime();
            operator.primary_field_at(&primary_flux, time, &runtime)?;
            operator.complementary_field_at(&complementary_flux, time, &runtime)?;
        }
        Ok(Self {
            primary_flux,
            complementary_flux,
            // A new generation's gaps start closed, which is the unexcited
            // physical history the specification asks for. A nonzero one needs
            // an explicit initializer rather than being implied by zero bulk.
            outgoing_z: vec![
                0.0;
                operator
                    .base()
                    .outgoing_boundary()
                    .map_or(0, |boundary| boundary.auxiliary_count())
            ],
            thin_gap_jump: vec![0.0; operator.base().thin_gap_samples().len()],
            // A fresh generation starts at rest in `r` too: the vacuum of
            // Klein-Gordon and sine-Gordon, the unstable top of φ⁴.
            integrated_field: vec![
                0.0;
                if operator.has_restoring() {
                    operator.base().degrees_of_freedom()
                } else {
                    0
                }
            ],
            runtime: operator.initial_runtime(),
            time_step,
            time,
        })
    }

    pub fn zero(
        operator: &CanonicalTemporalWaveOperator,
        time_step: f64,
    ) -> Result<Self, WaveError> {
        Self::new(
            operator,
            time_step,
            vec![0.0; operator.base().degrees_of_freedom()],
            vec![Point2::default(); operator.base().complementary_degrees_of_freedom()],
        )
    }

    pub fn primary_flux(&self) -> &[f64] {
        &self.primary_flux
    }

    pub fn complementary_flux(&self) -> &[Point2] {
        &self.complementary_flux
    }

    pub fn runtime(&self) -> &CanonicalMaterialRuntimeState {
        &self.runtime
    }

    pub fn runtime_mut(&mut self) -> &mut CanonicalMaterialRuntimeState {
        &mut self.runtime
    }

    pub fn time_step(&self) -> f64 {
        self.time_step
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    /// Energy stored in the thin-gap springs and the outgoing pole currents,
    /// which is part of the state's total and not reconstructible from the
    /// bulk fields. The pole currents are already in energy coordinates, which
    /// is what the compiled transform is for, so their store is a plain sum of
    /// squares.
    fn history_energy(&self, operator: &CanonicalTemporalWaveOperator) -> f64 {
        history_energy_of(
            operator,
            &self.thin_gap_jump,
            &self.outgoing_z,
            &self.integrated_field,
        )
    }

    pub fn thin_gap_jump(&self) -> &[f64] {
        &self.thin_gap_jump
    }

    /// The integrated field `r`, empty without a restoring law.
    pub fn integrated_field(&self) -> &[f64] {
        &self.integrated_field
    }

    /// The same state holding a given integrated field, which is how a kink,
    /// a domain wall or a displaced vacuum is initialized.
    pub fn with_integrated_field(
        mut self,
        operator: &CanonicalTemporalWaveOperator,
        integrated_field: Vec<f64>,
    ) -> Result<Self, WaveError> {
        if !operator.has_restoring()
            || integrated_field.len() != operator.base().degrees_of_freedom()
        {
            return Err(WaveError::InvalidState);
        }
        validate_finite(&integrated_field)?;
        operator.restoring_force(&integrated_field)?;
        self.integrated_field = integrated_field;
        Ok(self)
    }

    pub fn outgoing_pole_currents(&self) -> &[f64] {
        &self.outgoing_z
    }

    pub fn energy(&self, operator: &CanonicalTemporalWaveOperator) -> Result<f64, WaveError> {
        Ok(operator.energy_at(
            &self.primary_flux,
            &self.complementary_flux,
            self.time,
            &self.runtime,
        )? + self.history_energy(operator))
    }

    /// Zero-duration paired grid filter with the material maps frozen at the
    /// accepted event time. This is the time-driven extension of the fixed
    /// linear polynomial: every `M^-1` and `J` application uses the same
    /// instantaneous coefficients, while the trajectory-wide CFL bound keeps
    /// the polynomial contract valid for every authored phase and Switch.
    /// A pulse authored as a field increment, added at the mass the medium has
    /// right now.
    ///
    /// The canonical state is the integrated nodal flux, so a field increment
    /// becomes a flux increment only once it is scaled by the nodal mass. On a
    /// time-driven medium that mass is the one at this instant, and it is not
    /// the authored one: a pump whose factor bottoms near a fifth of its
    /// authored value would take a pulse several times too large or too small
    /// if the authored mass were used. Nothing here can be deferred to the
    /// caller, because the caller does not know when the pulse will land.
    ///
    /// Free primary nodes only. A pinned node's flux is whatever its prescribed
    /// data says it is, so a pulse cannot move it.
    pub fn apply_primary_pulse(
        &mut self,
        operator: &CanonicalTemporalWaveOperator,
        forcing: &CanonicalForcing,
        field_increment: &[f64],
    ) -> Result<(), WaveError> {
        if field_increment.len() != operator.base().degrees_of_freedom()
            || field_increment.iter().any(|value| !value.is_finite())
        {
            return Err(WaveError::InvalidState);
        }
        if operator.has_field_laws {
            // The increment is to the field, so on a nonlinear node it lands
            // through the map: `Q ← P(U(Q) + δ)`, not `Q + m·δ`.
            let (terms, _) = operator.primary_terms_at(self.time, &self.runtime)?;
            let mut next = self.primary_flux.clone();
            for (node, (flux, increment)) in next.iter_mut().zip(field_increment).enumerate() {
                if forcing.prescribed()[node].is_some() {
                    continue;
                }
                let field = operator.primary_inverse(&terms, node, *flux, &self.runtime)?;
                *flux = operator
                    .primary_flux_and_energy_of_field(
                        &terms,
                        node,
                        field + increment,
                        &self.runtime,
                    )?
                    .0;
            }
            validate_finite(&next)?;
            self.primary_flux = next;
            return Ok(());
        }
        let mass = operator.primary_mass_at(self.time, &self.runtime)?;
        let mut next = self.primary_flux.clone();
        for (node, (flux, (increment, mass))) in next
            .iter_mut()
            .zip(field_increment.iter().zip(&mass))
            .enumerate()
        {
            if forcing.prescribed()[node].is_some() {
                continue;
            }
            *flux += increment * mass;
        }
        validate_finite(&next)?;
        self.primary_flux = next;
        Ok(())
    }

    pub fn apply_grid_filter(
        &mut self,
        operator: &CanonicalTemporalWaveOperator,
        strength: f64,
    ) -> Result<f64, WaveError> {
        let forcing = CanonicalForcing::none(operator.base());
        self.apply_grid_filter_with_forcing(operator, &forcing, strength)
    }

    /// The grid filter beside the forced composition: open walls of either
    /// order, thin gaps, loss and prescribed data.
    ///
    /// The fixed path's rules carry over unchanged. The correction is a
    /// `Cᵀ(·)` in `Q` and a `C(·)` in `b`, so constants, component totals and
    /// compatibility hold whatever the boundary does. A prescribed node is
    /// skipped: its flux is whatever its data made it at this endpoint, so
    /// there is no exchange to account. The pole currents and gap jumps are
    /// left as they are, and their stored energy is part of the commit test,
    /// which admits only a candidate whose total energy does not rise.
    ///
    /// Gate O: on an oscillator medium the complementary correction is a
    /// correction to the integrated field, `δb = ηC δψ`, and `r` takes the
    /// same `δψ`, so the step's invariant `b = ηC r` survives the filter. It
    /// filters the total force on `ψ`, `δψ = −s A K A (F + R)`, whose
    /// first-order energy change `−s (F + R)ᵀ A K A (F + R)` is not positive,
    /// and which is zero at an equilibrium: a static kink, a wall or a well
    /// balances `F(b) + R(r) = 0` with `F` itself nonzero, and a filter on `F`
    /// alone would wear it away event by event.
    pub fn apply_grid_filter_with_forcing(
        &mut self,
        operator: &CanonicalTemporalWaveOperator,
        forcing: &CanonicalForcing,
        strength: f64,
    ) -> Result<f64, WaveError> {
        if !operator.forced_composition_supported() {
            return Err(WaveError::Unsupported(
                "the time-driven grid filter needs a composition this scene does not have",
            ));
        }
        if forcing.prescribed().len() != operator.base().degrees_of_freedom() {
            return Err(WaveError::SizeMismatch {
                expected: operator.base().degrees_of_freedom(),
                actual: forcing.prescribed().len(),
            });
        }
        if !strength.is_finite() || !(0.0..=1.0).contains(&strength) {
            return Err(WaveError::Unsupported(
                "the time-driven grid filter strength is not a fraction between zero and one",
            ));
        }
        if strength == 0.0 {
            return Ok(0.0);
        }
        let before = self.energy(operator)?;
        let (primary_correction, integrated_correction) = if operator.has_field_laws {
            self.tangent_grid_filter_corrections(operator)?
        } else {
            self.frozen_grid_filter_corrections(operator)?
        };

        let eigenvalue_bound = 4.0 / operator.maximum_time_step().powi(2);
        let scale = strength / eigenvalue_bound.powi(2);
        let mut next_primary = self.primary_flux.clone();
        let mut next_complementary = self.complementary_flux.clone();
        for ((value, correction), pinned) in next_primary
            .iter_mut()
            .zip(primary_correction)
            .zip(forcing.prescribed())
        {
            if pinned.is_none() {
                *value -= scale * correction;
            }
        }
        let complementary_correction = operator.base().compatible_flux(&integrated_correction)?;
        for (value, correction) in next_complementary.iter_mut().zip(complementary_correction) {
            *value = *value - correction * scale;
        }
        let mut next_integrated = self.integrated_field.clone();
        for (value, correction) in next_integrated.iter_mut().zip(&integrated_correction) {
            *value -= scale * correction;
        }
        validate_finite(&next_primary)?;
        validate_finite(&next_integrated)?;
        if next_complementary.iter().any(|value| !value.finite()) {
            return Err(WaveError::InvalidState);
        }
        let after =
            operator.energy_at(&next_primary, &next_complementary, self.time, &self.runtime)?
                + history_energy_of(
                    operator,
                    &self.thin_gap_jump,
                    &self.outgoing_z,
                    &next_integrated,
                );
        let tolerance = 2.0e-12 * before.abs().max(after.abs()).max(1.0);
        if after > before + tolerance {
            return Err(WaveError::InvalidState);
        }
        self.primary_flux = next_primary;
        self.complementary_flux = next_complementary;
        self.integrated_field = next_integrated;
        Ok((before - after).max(0.0))
    }

    /// The time-driven linear polynomial's two corrections, every map frozen
    /// at the event instant: the one to `Q`, and the one to the integrated
    /// field, which `b` takes through `ηC`.
    fn frozen_grid_filter_corrections(
        &self,
        operator: &CanonicalTemporalWaveOperator,
    ) -> Result<(Vec<f64>, Vec<f64>), WaveError> {
        let mass = operator.primary_mass_at(self.time, &self.runtime)?;
        let inverse_mass = |values: Vec<f64>| {
            values
                .into_iter()
                .zip(&mass)
                .map(|(value, mass)| value / mass)
                .collect::<Vec<_>>()
        };
        let stiffness = |field: &[f64]| -> Result<Vec<f64>, WaveError> {
            let flux = operator.base().compatible_flux(field)?;
            operator.force_at(&flux, self.time, &self.runtime)
        };
        let primary_field = self
            .primary_flux
            .iter()
            .zip(&mass)
            .map(|(flux, mass)| flux / mass)
            .collect::<Vec<_>>();
        let primary_correction = stiffness(&inverse_mass(stiffness(&primary_field)?))?;
        let mut gathered = operator.force_at(&self.complementary_flux, self.time, &self.runtime)?;
        add_restoring_force(operator, &self.integrated_field, &mut gathered)?;
        let integrated_correction = inverse_mass(stiffness(&inverse_mass(gathered))?);
        Ok((primary_correction, integrated_correction))
    }

    /// Corrects only roundoff-sized drift from already accounted component
    /// totals, as the fixed path's `maintain_component_totals` does, with the
    /// correction spread by positive tangent weights.
    ///
    /// The fixed path spreads `δ` by nodal mass, which shifts the field
    /// uniformly. On a field-dependent medium the weight that does the same is
    /// the tangent `P′(U)`, and it is the instantaneous mass at a linear node.
    /// The corrected fluxes are then reinverted: a correction that would carry
    /// a node past its declared bound is refused and the state is kept. A
    /// mismatch above roundoff is physical or a broken ledger, and is refused
    /// rather than projected away.
    pub fn maintain_component_totals(
        &mut self,
        operator: &CanonicalTemporalWaveOperator,
        forcing: &CanonicalForcing,
        intended_totals: &[f64],
    ) -> Result<f64, WaveError> {
        let base = operator.base();
        if intended_totals.len() != base.component_count()
            || forcing.prescribed().len() != base.degrees_of_freedom()
            || intended_totals.iter().any(|value| !value.is_finite())
        {
            return Err(WaveError::InvalidState);
        }
        let weights = if operator.has_field_laws {
            let field = operator.primary_field_at(&self.primary_flux, self.time, &self.runtime)?;
            operator
                .primary_tangent_inverse_at(&field, self.time, &self.runtime)?
                .into_iter()
                .map(|inverse| 1.0 / inverse)
                .collect::<Vec<_>>()
        } else {
            operator.primary_mass_at(self.time, &self.runtime)?
        };
        let scale = self
            .primary_flux
            .iter()
            .map(|value| value.abs())
            .sum::<f64>()
            .max(1.0);
        let mut next = self.primary_flux.clone();
        let mut maximum = 0.0_f64;
        for (component, intended) in intended_totals.iter().enumerate() {
            let owned = |node: usize| base.component_labels()[node] as usize == component;
            let current = (0..next.len())
                .filter(|node| owned(*node))
                .map(|node| next[node])
                .sum::<f64>();
            let delta = intended - current;
            maximum = maximum.max(delta.abs());
            if delta.abs() > 1.0e-10 * scale {
                return Err(WaveError::InvalidState);
            }
            let eligible = (0..next.len())
                .filter(|node| owned(*node) && forcing.prescribed()[*node].is_none())
                .map(|node| weights[node])
                .sum::<f64>();
            if delta != 0.0 && eligible == 0.0 {
                return Err(WaveError::InvalidState);
            }
            for node in 0..next.len() {
                if owned(node) && forcing.prescribed()[node].is_none() {
                    next[node] += delta * weights[node] / eligible;
                }
            }
        }
        validate_finite(&next)?;
        if operator.has_field_laws {
            operator.primary_field_at(&next, self.time, &self.runtime)?;
        }
        self.primary_flux = next;
        Ok(maximum)
    }

    /// Gate F: the grid filter on a field-dependent medium.
    ///
    /// The linear polynomial with every map frozen at its tangent at the
    /// event state:
    /// - `M⁻¹` becomes `A = ∂U/∂Q`, the diagonal `1/P′(U)`;
    /// - `J` becomes `J_b = ∂v/∂b` at each sample;
    /// - the outer operators act on the actual observables `U(Q)` and `F(b)`.
    ///
    /// ```text
    /// Q ← Q − α K_t A K_t U / Λ²
    /// b ← b − α C A K_t A F(b) / Λ²,   K_t = Cᵀ W J_b C
    /// ```
    ///
    /// To first order in `α` the energy change is
    /// `−α/Λ² [(K_t U)ᵀ A (K_t U) + Fᵀ A K_t A F] ≤ 0`, because `A` and
    /// `K_t` are positive (semi)definite for every executed law. The
    /// trajectory bound `Λ = 4/dt_max²` covers the tangent over all admitted
    /// amplitudes. The correction is a `Cᵀ(·)` in `Q` and a `C(·)` in `b`,
    /// so a constant field, component totals and compatibility are kept
    /// exactly. At small amplitude every tangent is the linear map and this is
    /// the linear filter. The higher orders are not signed, so the commit
    /// keeps the existing rule: only when the nonlinear energy does not rise
    /// and the new state is inside its domain.
    fn tangent_grid_filter_corrections(
        &self,
        operator: &CanonicalTemporalWaveOperator,
    ) -> Result<(Vec<f64>, Vec<f64>), WaveError> {
        let (time, runtime) = (self.time, &self.runtime);
        let field = operator.primary_field_at(&self.primary_flux, time, runtime)?;
        let tangent = operator.primary_tangent_inverse_at(&field, time, runtime)?;
        let sample_tangents =
            operator.complementary_tangents_at(&self.complementary_flux, time, runtime)?;
        let weighted = |values: Vec<f64>| {
            values
                .into_iter()
                .zip(&tangent)
                .map(|(value, tangent)| value * tangent)
                .collect::<Vec<_>>()
        };
        let stiffness = |field: &[f64]| -> Result<Vec<f64>, WaveError> {
            let flux = operator.base().compatible_flux(field)?;
            let fields = flux
                .iter()
                .zip(&sample_tangents)
                .map(|(flux, tangent)| tangent.apply(*flux))
                .collect::<Vec<_>>();
            operator.gather_force(&fields)
        };
        let primary_correction = stiffness(&weighted(stiffness(&field)?))?;
        let mut gathered = operator.force_at(&self.complementary_flux, time, runtime)?;
        add_restoring_force(operator, &self.integrated_field, &mut gathered)?;
        let integrated_correction = weighted(stiffness(&weighted(gathered))?);
        Ok((primary_correction, integrated_correction))
    }

    pub fn step(
        &mut self,
        operator: &CanonicalTemporalWaveOperator,
    ) -> Result<CanonicalTemporalStepAccounting, WaveError> {
        self.step_by(operator, self.time_step)
    }

    /// A state whose prescribed nodes already hold `Q = M(t) g(t)`.
    ///
    /// Anything else is not a state of the constrained system, and stepping it
    /// charges the difference to the boundary on the first step alone.
    pub fn pinned(
        mut self,
        operator: &CanonicalTemporalWaveOperator,
        forcing: &CanonicalForcing,
    ) -> Result<Self, WaveError> {
        if forcing.prescribed().len() != operator.base().degrees_of_freedom() {
            return Err(WaveError::SizeMismatch {
                expected: operator.base().degrees_of_freedom(),
                actual: forcing.prescribed().len(),
            });
        }
        let mass = operator.primary_mass_at(self.time, &self.runtime)?;
        let terms = if operator.has_field_laws {
            Some(operator.primary_terms_at(self.time, &self.runtime)?.0)
        } else {
            None
        };
        for (node, signal) in forcing.prescribed().iter().enumerate() {
            if let Some(signal) = signal {
                let value = signal.value(self.time);
                self.primary_flux[node] = match terms.as_deref() {
                    Some(terms) => {
                        operator
                            .primary_flux_and_energy_of_field(terms, node, value, &self.runtime)?
                            .0
                    }
                    None => mass[node] * value,
                };
            }
        }
        validate_finite(&self.primary_flux)?;
        Ok(self)
    }

    /// One step with prescribed data and volume sources composed into it.
    ///
    /// The ordering is the fixed path's, stage for stage, so the two can be
    /// compared directly and an inert generation reproduces
    /// `CanonicalWaveState::step_with_forcing` exactly. What changes is that
    /// every place the fixed path multiplies by the nodal mass, this one
    /// multiplies by the mass in force at that stage's own instant.
    ///
    /// That is the whole content of the composition. A prescribed node pins
    /// `Q = M(t) g(t)`, so a constant `g` over a breathing `M` still moves
    /// flux across the boundary, and the accounting has to call that
    /// prescribed exchange rather than temporal work. Sources integrate at the
    /// two endpoints as they do on the fixed path, which is what preserves a
    /// run's frozen startup envelope through a material edit.
    ///
    /// Initialization is a requirement, not a convenience: a prescribed node's
    /// initial flux must already satisfy `Q = M(t0) g(t0)`. The fixed path
    /// hides a violation by pinning before its first kick and charging the
    /// correction to prescribed exchange; this one pins at the stage, so an
    /// inconsistent start shows up as a first step that does not balance.
    /// [`Self::pinned`] builds a consistent state.
    pub fn step_with_forcing(
        &mut self,
        operator: &CanonicalTemporalWaveOperator,
        forcing: &CanonicalForcing,
    ) -> Result<CanonicalTemporalStepAccounting, WaveError> {
        self.step_with_forcing_by(operator, forcing, self.time_step)
    }

    /// Signed stepping is exposed for the reversibility gate. Production uses
    /// `step`; a negative duration applies the exact inverse composition.
    pub fn step_by(
        &mut self,
        operator: &CanonicalTemporalWaveOperator,
        duration: f64,
    ) -> Result<CanonicalTemporalStepAccounting, WaveError> {
        let forcing = CanonicalForcing::none(operator.base());
        self.step_with_forcing_by(operator, &forcing, duration)
    }

    /// Signed stepping with forcing, for the reversibility gate.
    pub fn step_with_forcing_by(
        &mut self,
        operator: &CanonicalTemporalWaveOperator,
        forcing: &CanonicalForcing,
        duration: f64,
    ) -> Result<CanonicalTemporalStepAccounting, WaveError> {
        if !operator.forced_composition_supported()
            || forcing.prescribed().len() != operator.base().degrees_of_freedom()
        {
            return Err(WaveError::Unsupported(
                "the time-driven operator cannot compose the forcing it was handed",
            ));
        }
        if !duration.is_finite() || duration == 0.0 || duration.abs() > operator.maximum_time_step()
        {
            return Err(WaveError::Unsupported(
                "the signed step duration is outside what the time-driven trajectory allows",
            ));
        }
        let start_time = self.time;
        let middle_time = start_time + 0.5 * duration;
        let end_time = start_time + duration;
        if !middle_time.is_finite() || !end_time.is_finite() {
            return Err(WaveError::InvalidState);
        }
        let before = self.energy(operator)?;
        let (_, complementary_rate_start) = operator.complementary_energy_and_rate(
            &self.complementary_flux,
            start_time,
            &self.runtime,
        )?;

        let mut primary = self.primary_flux.clone();
        let mut complementary = self.complementary_flux.clone();
        let mut outgoing_z = self.outgoing_z.clone();
        let mut source_work = 0.0;
        let mut prescribed_exchange = 0.0;
        let mut boundary_loss = 0.0;

        // Strang: half the dissipation, the conservative core, half again.
        // Each half map is instantaneous at the step endpoint it sits on - so
        // the temporal-work quadrature between those endpoints is untouched -
        // but it stands for evolution over its own half interval, so its rate
        // is read at that interval's midpoint. Reading it at the endpoint
        // instead would be first order for a driven loss.
        let (first_primary_loss, first_complementary_loss, first_gain) = decay(
            operator,
            &mut primary,
            &mut complementary,
            0.5 * duration,
            start_time,
            start_time + 0.25 * duration,
            &self.runtime,
        )?;

        // Both kicks pin and sample at their own stage instant, which is the
        // step's endpoints - the same two instants the autonomous extension
        // stages at and the same two the temporal-work quadrature integrates
        // between. The fixed path instead pins the first kick half a step in,
        // which is free to choose when the mass is constant. It is not free
        // here: a pin at an instant the work quadrature does not know about
        // leaves a first-order hole in the energy balance, measured at order
        // 0.99 before this was moved.
        let mut first_force = operator.force_at(&complementary, start_time, &self.runtime)?;
        add_gap_force(operator, &self.thin_gap_jump, &mut first_force)?;
        add_restoring_force(operator, &self.integrated_field, &mut first_force)?;
        let first_source = forcing.integrated_rate(start_time)?;
        let (work, exchange, escaped) = forced_kick(
            operator,
            &mut primary,
            &mut outgoing_z,
            &first_force,
            &first_source,
            0.5 * duration,
            forcing,
            start_time,
            &self.runtime,
        )?;
        source_work += work;
        prescribed_exchange += exchange;
        boundary_loss += escaped;
        validate_finite(&primary)?;

        let (_, primary_rate_middle) =
            operator.primary_energy_and_rate(&primary, middle_time, &self.runtime)?;
        // The gap's own displacement drifts on the same field and over the
        // same interval as the complementary flux does: both are the drift
        // subflow, and splitting them would break the exactness the local gap
        // split is admitted for, so both read the one field built here.
        //
        // A prescribed node's field during the drift is its signal at the
        // drift's instant. Its flux is pinned at the endpoints, where the
        // kicks and the work quadrature stage, so reading the field back from
        // that flux would hand the drift `g(t_n)` - and, under a breathing
        // mass, `M(t_n) g(t_n) / M(t_n+½)` - which is first order in both the
        // trajectory and the balance. The energy that crosses the pin this way
        // is the pinned nodes' force work, which the kicks already charge to
        // the prescribed exchange.
        let midpoint_field = {
            let mut field = operator.primary_field_at(&primary, middle_time, &self.runtime)?;
            for (value, signal) in field.iter_mut().zip(forcing.prescribed()) {
                if let Some(signal) = signal {
                    *value = signal.value(middle_time);
                }
            }
            field
        };
        let mut gap_jump = self.thin_gap_jump.clone();
        if !gap_jump.is_empty() {
            for (sample, jump) in operator.base().thin_gap_samples().iter().zip(&mut gap_jump) {
                *jump += duration
                    * (midpoint_field[sample.left_node as usize]
                        - midpoint_field[sample.right_node as usize]);
            }
            validate_finite(&gap_jump)?;
        }
        operator.drift_on(&mut complementary, &midpoint_field, duration)?;
        // The integrated field drifts with `b`, on the same midpoint field and
        // over the same interval: `ṙ = u` is the same subflow as `ḃ = ηCu`.
        let mut integrated = self.integrated_field.clone();
        for (value, field) in integrated.iter_mut().zip(&midpoint_field) {
            *value += duration * field;
        }
        validate_finite(&integrated)?;
        let (_, complementary_rate_end) =
            operator.complementary_energy_and_rate(&complementary, end_time, &self.runtime)?;

        let mut second_force = operator.force_at(&complementary, end_time, &self.runtime)?;
        add_gap_force(operator, &gap_jump, &mut second_force)?;
        add_restoring_force(operator, &integrated, &mut second_force)?;
        let second_source = forcing.integrated_rate(end_time)?;
        let (work, exchange, escaped) = forced_kick(
            operator,
            &mut primary,
            &mut outgoing_z,
            &second_force,
            &second_source,
            0.5 * duration,
            forcing,
            end_time,
            &self.runtime,
        )?;
        source_work += work;
        prescribed_exchange += exchange;
        boundary_loss += escaped;
        validate_finite(&primary)?;

        let (second_primary_loss, second_complementary_loss, second_gain) = decay(
            operator,
            &mut primary,
            &mut complementary,
            0.5 * duration,
            end_time,
            start_time + 0.75 * duration,
            &self.runtime,
        )?;
        let primary_loss = first_primary_loss + second_primary_loss;
        let active_gain = first_gain + second_gain;
        let complementary_loss = first_complementary_loss + second_complementary_loss;

        // Measured like `before`, stores included. Leaving them out charged
        // every gap spring and pole current to the splitting residual.
        let after = operator.energy_at(&primary, &complementary, end_time, &self.runtime)?
            + history_energy_of(operator, &gap_jump, &outgoing_z, &integrated);
        let temporal_work = duration
            * (0.5 * complementary_rate_start + primary_rate_middle + 0.5 * complementary_rate_end);
        let energy_change = after - before;
        let accounting = CanonicalTemporalStepAccounting {
            temporal_work,
            source_work,
            primary_loss,
            complementary_loss,
            boundary_loss,
            prescribed_exchange,
            active_gain,
            energy_change,
            splitting_residual: energy_change
                - temporal_work
                - source_work
                - prescribed_exchange
                - active_gain
                + primary_loss
                + complementary_loss
                + boundary_loss,
        };
        if !accounting.temporal_work.is_finite()
            || !accounting.source_work.is_finite()
            || !accounting.primary_loss.is_finite()
            || !accounting.complementary_loss.is_finite()
            || !accounting.boundary_loss.is_finite()
            || !accounting.prescribed_exchange.is_finite()
            || !accounting.active_gain.is_finite()
            || !accounting.energy_change.is_finite()
            || !accounting.splitting_residual.is_finite()
        {
            return Err(WaveError::InvalidState);
        }
        self.primary_flux = primary;
        self.complementary_flux = complementary;
        self.thin_gap_jump = gap_jump;
        self.outgoing_z = outgoing_z;
        self.integrated_field = integrated;
        self.time = end_time;
        Ok(accounting)
    }
}

/// The stores the bulk fields do not hold: the gap springs, the outgoing pole
/// currents (already in energy coordinates, so a plain sum of squares) and the
/// restoring potential on the integrated field.
fn history_energy_of(
    operator: &CanonicalTemporalWaveOperator,
    gaps: &[f64],
    poles: &[f64],
    integrated: &[f64],
) -> f64 {
    let gaps = gaps
        .iter()
        .zip(operator.base().thin_gap_samples())
        .map(|(jump, sample)| 0.5 * sample.stiffness * jump * jump)
        .sum::<f64>();
    let poles = poles.iter().map(|value| 0.5 * value * value).sum::<f64>();
    gaps + poles + operator.restoring_energy(integrated)
}

/// The lowest primary tangent factor, or one where no row has any samples.
fn primary_floor_of(minimum: f64) -> f64 {
    if minimum.is_finite() && minimum > 0.0 {
        minimum
    } else {
        1.0
    }
}

/// Adds the thin-gap spring force, which pushes the two sides of a trace apart
/// in proportion to the field jump the gap has integrated.
fn add_gap_force(
    operator: &CanonicalTemporalWaveOperator,
    gap_jump: &[f64],
    force: &mut [f64],
) -> Result<(), WaveError> {
    if gap_jump.is_empty() {
        return Ok(());
    }
    if gap_jump.len() != operator.base().thin_gap_samples().len() {
        return Err(WaveError::InvalidState);
    }
    for (sample, jump) in operator.base().thin_gap_samples().iter().zip(gap_jump) {
        let value = sample.stiffness * jump;
        force[sample.left_node as usize] += value;
        force[sample.right_node as usize] -= value;
    }
    validate_finite(force)
}

/// Adds the restoring force on the integrated field, which acts like a spring
/// on `r` in the same kick slot as the gap springs.
fn add_restoring_force(
    operator: &CanonicalTemporalWaveOperator,
    integrated: &[f64],
    force: &mut [f64],
) -> Result<(), WaveError> {
    if !operator.has_restoring() {
        return Ok(());
    }
    for (total, value) in force.iter_mut().zip(operator.restoring_force(integrated)?) {
        *total += value;
    }
    validate_finite(force)
}

/// One half of the Strang dissipation map, and the energy it removed.
///
/// `stage_time` is where the map sits, which is what the energies are measured
/// against; `rate_time` is the midpoint of the half interval it stands for,
/// which is what the decay rate is read at. On a fixed rate the two choices
/// coincide and this reduces to the fixed path's exponential exactly.
fn decay(
    operator: &CanonicalTemporalWaveOperator,
    primary: &mut [f64],
    complementary: &mut [Point2],
    duration: f64,
    stage_time: f64,
    rate_time: f64,
    runtime: &CanonicalMaterialRuntimeState,
) -> Result<(f64, f64, f64), WaveError> {
    if !operator.has_loss {
        return Ok((0.0, 0.0, 0.0));
    }
    let rates = operator.loss_rates_at(rate_time, runtime)?;
    let active = if operator.has_active_loss {
        Some(operator.active_loss_coefficients(rate_time, runtime)?)
    } else {
        None
    };
    let primary_before = operator
        .primary_energy_and_rate(primary, stage_time, runtime)?
        .0;
    let complementary_before = operator
        .complementary_energy_and_rate(complementary, stage_time, runtime)?
        .0;
    let primary_mass = operator.primary_mass_at(stage_time, runtime)?;
    let node_energy = |flux: &[f64]| -> Result<Vec<f64>, WaveError> {
        if operator.has_field_laws {
            return Ok(Vec::new());
        }
        Ok(flux
            .iter()
            .zip(&primary_mass)
            .map(|(flux, mass)| 0.5 * flux * flux / mass)
            .collect())
    };
    let before_nodes = node_energy(primary)?;
    match &active {
        Some((beta, alpha, mass)) => {
            for (node, flux) in primary.iter_mut().enumerate() {
                *flux = bernoulli_map(
                    *flux,
                    beta[node],
                    alpha[node] / (mass[node] * mass[node]),
                    duration,
                );
            }
        }
        None => {
            for (flux, rate) in primary.iter_mut().zip(&rates.primary) {
                *flux *= (-duration * rate).exp();
            }
        }
    }
    for (flux, rate) in complementary.iter_mut().zip(&rates.complementary) {
        *flux = *flux * (-duration * rate).exp();
    }
    validate_finite(primary)?;
    let removed_complementary = complementary_before
        - operator
            .complementary_energy_and_rate(complementary, stage_time, runtime)?
            .0;
    let Some((_, alpha, _)) = &active else {
        let removed_primary = primary_before
            - operator
                .primary_energy_and_rate(primary, stage_time, runtime)?
                .0;
        // A passive channel cannot add energy. Anything else is a defect in
        // the rate, not a small negative to be clamped away quietly.
        if removed_primary < -1.0e-12 || removed_complementary < -1.0e-12 {
            return Err(WaveError::InvalidState);
        }
        return Ok((
            removed_primary.max(0.0),
            removed_complementary.max(0.0),
            0.0,
        ));
    };
    // An active node's change is gain, of either sign; the rest is passive.
    let after_nodes = node_energy(primary)?;
    let (mut removed_primary, mut gained) = (0.0, 0.0);
    for node in 0..primary.len() {
        let change = after_nodes[node] - before_nodes[node];
        if alpha[node] != 0.0 {
            gained += change;
        } else {
            removed_primary -= change;
        }
    }
    if removed_primary < -1.0e-12 || removed_complementary < -1.0e-12 {
        return Err(WaveError::InvalidState);
    }
    Ok((
        removed_primary.max(0.0),
        removed_complementary.max(0.0),
        gained,
    ))
}

/// Per node: the rate's constant part `β`, its `u²` coefficient `α`, and the
/// mass they were weighed by.
type ActiveLossCoefficients = (Vec<f64>, Vec<f64>, Vec<f64>);

/// `Q̇ = −(β + k Q²) Q` over `duration`, exactly: in `y = Q²` it is the
/// Bernoulli equation `ẏ = −2(β + k y) y`, whose solution keeps `Q`'s sign.
/// A van der Pol node has `β < 0` (gain below threshold) and `k > 0`.
fn bernoulli_map(flux: f64, beta: f64, k: f64, duration: f64) -> f64 {
    let y = flux * flux;
    let next = if beta.abs() * duration < 1.0e-12 {
        y / (1.0 + 2.0 * k * y * duration)
    } else {
        let decay = (-2.0 * beta * duration).exp();
        // `1 − e^{−2βτ}` over `β`, formed without cancellation.
        let grown = -(-2.0 * beta * duration).exp_m1() / beta;
        y * decay / (1.0 + k * y * grown)
    };
    flux.signum() * next.max(0.0).sqrt()
}

/// One half kick with the source and any prescribed pin folded into it, as the
/// fixed path's `force_coupled_kick` does with boundary damping and open
/// boundaries excluded - both are refused by `forced_composition_supported`.
///
/// The mass is the one in force at `target_time`, which is the instant the
/// prescribed signal is sampled at, so a pinned node's `Q` and the `M` it is
/// pinned against belong to the same moment.
#[allow(clippy::too_many_arguments)]
fn forced_kick(
    operator: &CanonicalTemporalWaveOperator,
    primary: &mut [f64],
    outgoing_z: &mut [f64],
    force: &[f64],
    source: &[f64],
    duration: f64,
    forcing: &CanonicalForcing,
    target_time: f64,
    runtime: &CanonicalMaterialRuntimeState,
) -> Result<(f64, f64, f64), WaveError> {
    let damping = operator.base().first_order_boundary_damping();
    let damped = damping.iter().any(|value| *value != 0.0);
    if !forcing.drives_any() && !damped && operator.base().outgoing_boundary().is_none() {
        for (flux, force) in primary.iter_mut().zip(force) {
            *flux -= duration * force;
        }
        return Ok((0.0, 0.0, 0.0));
    }
    let mass = operator.primary_mass_at(target_time, runtime)?;
    let nonlinear = if operator.has_field_laws {
        Some(operator.primary_terms_at(target_time, runtime)?.0)
    } else {
        None
    };
    let mut source_work = 0.0;
    let mut prescribed_exchange = 0.0;
    let mut boundary_loss = 0.0;

    // A second-order wall is a nonlocal implicit solve over its trace and its
    // pole currents, and the trace admittance, the modal couplings and the
    // Schur complement all scale with the nodal mass. A fixed generation
    // factorizes once at construction; a driven one cannot, because the mass
    // it is built from belongs to the stage. The trace nodes it owns are then
    // skipped by the local loop below, exactly as on the fixed path.
    let mut trace_nodes = BTreeSet::new();
    if let Some(boundary) = operator.base().outgoing_boundary() {
        let factor = crate::canonical_wave::CanonicalOutgoingMidpointFactor::prepare(
            operator.base(),
            boundary,
            duration,
        )?;
        let nonlinear_trace = nonlinear.as_deref().filter(|terms| {
            boundary.trace_nodes().iter().any(|node| {
                !ConstitutiveSite::new(&terms[operator.primary_range(*node as usize)]).is_linear()
            })
        });
        let (work, escaped, exchange) = if let Some(terms) = nonlinear_trace {
            // Admission refuses prescribed data on a trace, so the nonlinear
            // kick has no pinned row to hold.
            let trace = TemporalTrace {
                operator,
                terms,
                runtime,
            };
            let (work, escaped) = crate::canonical_wave::nonlinear_outgoing_kick_with(
                primary,
                outgoing_z,
                &factor,
                operator.base(),
                boundary,
                &mass,
                &trace,
                force,
                source,
                duration,
            )?;
            (work, escaped, 0.0)
        } else {
            let mut prescribed_cache = None;
            crate::canonical_wave::force_coupled_outgoing_kick_with(
                primary,
                outgoing_z,
                &mut prescribed_cache,
                &factor,
                operator.base(),
                boundary,
                &mass,
                force,
                source,
                duration,
                forcing,
                target_time,
            )?
        };
        source_work += work;
        boundary_loss += escaped;
        prescribed_exchange += exchange;
        trace_nodes.extend(boundary.trace_nodes().iter().map(|node| *node as usize));
    }

    for node in 0..primary.len() {
        if trace_nodes.contains(&node) {
            continue;
        }
        let old = primary[node];
        if let Some(terms) = nonlinear.as_deref()
            && !ConstitutiveSite::new(&terms[operator.primary_range(node)]).is_linear()
        {
            // The kick charges its work through the discrete gradient
            // `ū = ΔT/ΔQ`, the one field whose work across the kick is exactly
            // the stored-energy change. The midpoint field the linear path
            // uses is that same quotient only for a quadratic `T`. An
            // absorbing wall makes the kick implicit in `ū`:
            // `Q − Q_old = τ(s − F − d·ū(Q))`, which is increasing in `Q`
            // with slope at least one, so its root is bracketed by any trial
            // point `x` and `x − f(x)` and found by safeguarded Newton.
            let old_energy = operator.primary_node_energy(terms, node, old, runtime)?;
            let rhs = source[node] - force[node];
            let (new, new_energy, gradient) = match forcing.prescribed()[node] {
                Some(signal) => {
                    let value = signal.value(target_time);
                    let (new, energy) =
                        operator.primary_flux_and_energy_of_field(terms, node, value, runtime)?;
                    (new, energy, value)
                }
                None => {
                    let new = if damping[node] == 0.0 {
                        old + duration * rhs
                    } else {
                        operator.damped_nonlinear_kick(
                            terms,
                            node,
                            old,
                            duration * rhs,
                            duration * damping[node],
                            runtime,
                        )?
                    };
                    let (gradient, _) =
                        operator.primary_discrete_gradient(terms, node, old, new, runtime)?;
                    (
                        new,
                        operator.primary_node_energy(terms, node, new, runtime)?,
                        gradient,
                    )
                }
            };
            if !new.is_finite() {
                return Err(WaveError::InvalidState);
            }
            let node_source_work = duration * gradient * source[node];
            let node_force_work = duration * gradient * force[node];
            let node_boundary_loss = duration * damping[node] * gradient * gradient;
            source_work += node_source_work;
            boundary_loss += node_boundary_loss;
            if forcing.prescribed()[node].is_some() {
                prescribed_exchange += new_energy - old_energy - node_source_work
                    + node_force_work
                    + node_boundary_loss;
            }
            primary[node] = new;
            continue;
        }
        let mass = mass[node];
        // The absorbing wall's admittance is `damping / mass`, and the mass is
        // the instantaneous one while the damping is not: it was assembled
        // from the authored medium and stays there. That is the frozen
        // reference impedance the specification allows only as a documented,
        // tested approximation, and this is the line it lives on.
        let ratio = 0.5 * duration * damping[node] / mass;
        let rhs = source[node] - force[node];
        let unconstrained = ((1.0 - ratio) * old + duration * rhs) / (1.0 + ratio);
        let new = forcing.prescribed()[node]
            .map_or(unconstrained, |signal| mass * signal.value(target_time));
        if !new.is_finite() {
            return Err(WaveError::InvalidState);
        }
        // A pinned node's field at the stage is its signal. Its work into the
        // bulk is then the trapezoid of `g·F` over the step, which is what the
        // drift's midpoint `g(t_n+½)` exchanges to second order. The quotient
        // of the pin's flux jump would mix in the mass it was pinned against
        // at the other endpoint, an error of order `h` under a pump.
        let midpoint_field = match forcing.prescribed()[node] {
            Some(signal) => signal.value(target_time),
            None => 0.5 * (old + new) / mass,
        };
        let node_source_work = duration * midpoint_field * source[node];
        let node_force_work = duration * midpoint_field * force[node];
        let node_boundary_loss = duration * damping[node] * midpoint_field * midpoint_field;
        source_work += node_source_work;
        boundary_loss += node_boundary_loss;
        if forcing.prescribed()[node].is_some() {
            let energy_change = 0.5 * (new * new - old * old) / mass;
            prescribed_exchange +=
                energy_change - node_source_work + node_force_work + node_boundary_loss;
        }
        primary[node] = new;
    }
    if source_work.is_finite() && prescribed_exchange.is_finite() && boundary_loss >= 0.0 {
        Ok((source_work, prescribed_exchange, boundary_loss))
    } else {
        Err(WaveError::InvalidState)
    }
}

pub(crate) fn strip_temporal_laws(materials: &mut [Material]) {
    for material in materials {
        material.mass_law = CoefficientLaw::linear();
        material.stiffness_law = CoefficientLaw::linear();
        if let Some(loss) = &mut material.electric_loss {
            loss.law = DampingLaw::constant();
        }
        if let Some(loss) = &mut material.magnetic_loss {
            loss.law = DampingLaw::constant();
        }
        material.restoring = RestoringLaw::None;
    }
}

#[derive(Clone, Copy)]
struct CompiledTemporalMaterialSample {
    coefficient: TemporalCoefficientSample,
    loss: TemporalLossSample,
    restoring: RestoringLawValues,
}

fn temporal_material_sample(
    model: TopologyWaveModel<'_>,
    region_id: crate::RegionId,
    point: Point2,
    primary: bool,
) -> Result<CompiledTemporalMaterialSample, WaveError> {
    let region = model.region(region_id).ok_or(WaveError::Unsupported(
        "a compiled element names a region the scene does not hold",
    ))?;
    let material = model
        .material(region.material)
        .ok_or(WaveError::Unsupported(
            "a region names a material the scene does not hold",
        ))?;
    if !material.switch_ramp.is_finite() || material.switch_ramp < 0.0 {
        return material_error(
            material,
            "Switch ramp",
            point,
            "duration must be finite and nonnegative",
        );
    }
    let coordinates = region.frame.coordinates(point);
    // Gate O: a restoring law acts on the integrated primary field, so only
    // the primary row carries it.
    let restoring = if primary && !material.restoring.is_none() {
        if !material.restoring.valid(&material.parameters) {
            return material_error(
                material,
                "restoring law",
                point,
                "authored frequency, strength or bound is invalid",
            );
        }
        material
            .restoring
            .evaluate_at(coordinates, &material.parameters)
            .map_err(|error| WaveError::MaterialEvaluation {
                material: material.name.clone(),
                coefficient: "restoring law",
                point,
                reason: error.to_string(),
            })?
    } else {
        RestoringLawValues::None
    };
    let (coefficient_drive, law) = coefficient_for(model.physics, material, primary);
    if !law.valid(&material.parameters) {
        return material_error(
            material,
            "coefficient law",
            point,
            "authored drive, alternate, or response is invalid",
        );
    }
    let law = law
        .evaluate_at(coordinates, &material.parameters)
        .map_err(|error| WaveError::MaterialEvaluation {
            material: material.name.clone(),
            coefficient: "coefficient law",
            point,
            reason: error.to_string(),
        })?;
    if let Err(refusal) = law.field.executable(law.inverted) {
        return material_error(material, "field law", point, refusal.reason());
    }
    let (minimum_tangent, maximum_tangent) =
        law.tangent_range()
            .ok_or_else(|| WaveError::MaterialEvaluation {
                material: material.name.clone(),
                coefficient: "coefficient trajectory",
                point,
                reason: "the full drive/Switch trajectory is not positive".into(),
            })?;
    if !minimum_tangent.is_finite()
        || minimum_tangent <= 0.0
        || maximum_tangent.is_nan()
        || maximum_tangent <= 0.0
    {
        return material_error(
            material,
            "coefficient trajectory",
            point,
            "the full drive/Switch tangent range must stay positive",
        );
    }
    let loss = loss_for(model.physics, material, primary, coordinates, point)?;
    Ok(CompiledTemporalMaterialSample {
        restoring,
        coefficient: TemporalCoefficientSample {
            material: material.id,
            point,
            coordinates,
            drive: coefficient_drive,
            law,
        },
        loss,
    })
}

/// Which authored coefficient law drives one solver row. The physics skin
/// decides: the TE skin carries the medium's permeability on the primary row,
/// so the authored mass and stiffness laws swap places there.
pub(crate) fn coefficient_for(
    physics: PhysicsModel,
    material: &Material,
    primary: bool,
) -> (CanonicalMaterialDrive, &CoefficientLaw) {
    let mass = (CanonicalMaterialDrive::MassCoefficient, &material.mass_law);
    let stiffness = (
        CanonicalMaterialDrive::StiffnessCoefficient,
        &material.stiffness_law,
    );
    match (physics, primary) {
        (PhysicsModel::Mechanical, true)
        | (
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            },
            true,
        )
        | (
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            },
            false,
        ) => mass,
        _ => stiffness,
    }
}

fn loss_for(
    physics: PhysicsModel,
    material: &Material,
    primary: bool,
    coordinates: MaterialCoordinates,
    point: Point2,
) -> Result<TemporalLossSample, WaveError> {
    let physical = material.electric_loss.is_some() || material.magnetic_loss.is_some();
    if !physical {
        if !primary {
            return Ok(TemporalLossSample::zero(material.id, point, coordinates));
        }
        let base_rate = evaluated_nonnegative(
            material,
            &material.damping,
            "legacy damping",
            coordinates,
            point,
        )?;
        return Ok(TemporalLossSample {
            material: material.id,
            point,
            coordinates,
            drive: None,
            base_rate,
            law: DampingLawValues {
                rate: RateLawValues::Constant,
                drive: TimeDriveValues::None,
            },
        });
    }
    let (channel, drive) = match (physics, primary) {
        (PhysicsModel::Mechanical, true)
        | (
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            },
            true,
        ) => (
            material.magnetic_loss.as_ref(),
            CanonicalMaterialDrive::MagneticLoss,
        ),
        (PhysicsModel::Mechanical, false)
        | (
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            },
            false,
        ) => (
            material.electric_loss.as_ref(),
            CanonicalMaterialDrive::ElectricLoss,
        ),
        (
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            },
            true,
        ) => (
            material.electric_loss.as_ref(),
            CanonicalMaterialDrive::ElectricLoss,
        ),
        (
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            },
            false,
        ) => (
            material.magnetic_loss.as_ref(),
            CanonicalMaterialDrive::MagneticLoss,
        ),
    };
    let Some(channel) = channel else {
        return Ok(TemporalLossSample::zero(material.id, point, coordinates));
    };
    compile_loss(material, channel, drive, primary, coordinates, point)
}

fn compile_loss(
    material: &Material,
    channel: &LossChannel,
    drive: CanonicalMaterialDrive,
    primary: bool,
    coordinates: MaterialCoordinates,
    point: Point2,
) -> Result<TemporalLossSample, WaveError> {
    if !channel.valid(&material.parameters) {
        return material_error(material, "loss law", point, "authored loss law is invalid");
    }
    match &channel.law.rate {
        RateLaw::Constant => {}
        // Gate O: van der Pol acts on the primary field, where the node map
        // is solved exactly; an undriven rate keeps that map closed-form.
        RateLaw::VanDerPol { .. } if primary && channel.law.drive.is_none() => {}
        RateLaw::VanDerPol { .. } => {
            return material_error(
                material,
                "loss law",
                point,
                "van der Pol acts on the primary field only, without a drive",
            );
        }
        _ => {
            return material_error(
                material,
                "loss law",
                point,
                "a field-dependent passive loss remains gated (C)",
            );
        }
    }
    let base_rate = evaluated_nonnegative(
        material,
        &channel.base_rate,
        "loss rate",
        coordinates,
        point,
    )?;
    let law = channel
        .law
        .evaluate_at(coordinates, &material.parameters)
        .map_err(|error| WaveError::MaterialEvaluation {
            material: material.name.clone(),
            coefficient: "loss law",
            point,
            reason: error.to_string(),
        })?;
    Ok(TemporalLossSample {
        material: material.id,
        point,
        coordinates,
        drive: Some(drive),
        base_rate,
        law,
    })
}

fn evaluated_nonnegative(
    material: &Material,
    field: &crate::ScalarField,
    coefficient: &'static str,
    coordinates: MaterialCoordinates,
    point: Point2,
) -> Result<f64, WaveError> {
    let value = field
        .evaluate(coordinates, &material.parameters)
        .map_err(|error| WaveError::MaterialEvaluation {
            material: material.name.clone(),
            coefficient,
            point,
            reason: error.to_string(),
        })?;
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        material_error(
            material,
            coefficient,
            point,
            "value must be finite and nonnegative",
        )
    }
}

fn coefficient_factor(
    sample: TemporalCoefficientSample,
    time: f64,
    runtime: &CanonicalMaterialRuntimeState,
) -> Result<f64, WaveError> {
    coefficient_factor_and_rate(sample, time, runtime).map(|(factor, _)| factor)
}

fn coefficient_factor_and_rate(
    sample: TemporalCoefficientSample,
    time: f64,
    runtime: &CanonicalMaterialRuntimeState,
) -> Result<(f64, f64), WaveError> {
    let runtime = runtime.record(sample.material)?;
    sample
        .law
        .temporal_factor_and_rate(
            time,
            sample.coordinates,
            runtime.drive(sample.drive),
            runtime.switch,
        )
        .map_err(|error| WaveError::MaterialEvaluation {
            material: runtime.material_name.clone(),
            coefficient: "coefficient trajectory",
            point: sample.point,
            reason: error.to_string(),
        })
}

fn loss_rate(
    sample: TemporalLossSample,
    time: f64,
    runtime: &CanonicalMaterialRuntimeState,
) -> Result<f64, WaveError> {
    let record = runtime.record(sample.material)?;
    let drive_runtime = match sample.drive {
        Some(drive) => record.drive(drive),
        None => TimeDriveRuntime::authored(TimeDriveValues::None)
            .map_err(|_| WaveError::InvalidState)?,
    };
    let multiplier = sample
        .law
        .multiplier(0.0, time, sample.coordinates, drive_runtime)
        .map_err(|error| WaveError::MaterialEvaluation {
            material: record.material_name.clone(),
            coefficient: "loss trajectory",
            point: sample.point,
            reason: error.to_string(),
        })?;
    let rate = sample.base_rate * multiplier;
    if rate.is_finite() && rate >= 0.0 {
        Ok(rate)
    } else {
        Err(WaveError::InvalidState)
    }
}

fn validate_positive(values: &[f64]) -> Result<(), WaveError> {
    values
        .iter()
        .all(|value| value.is_finite() && *value > 0.0)
        .then_some(())
        .ok_or(WaveError::Unsupported(
            "a driven medium's instantaneous mass is not finite and positive",
        ))
}

fn validate_nonnegative(values: &[f64]) -> Result<(), WaveError> {
    values
        .iter()
        .all(|value| value.is_finite() && *value >= 0.0)
        .then_some(())
        .ok_or(WaveError::Unsupported(
            "a driven medium's instantaneous loss rate is negative or not finite",
        ))
}

fn validate_finite(values: &[f64]) -> Result<(), WaveError> {
    values
        .iter()
        .all(|value| value.is_finite())
        .then_some(())
        .ok_or(WaveError::InvalidState)
}

fn stable_difference(value: f64, reference: f64) -> f64 {
    let difference = value - reference;
    let roundoff = 8.0 * f64::EPSILON * value.abs().max(reference.abs()).max(1.0);
    if difference.abs() <= roundoff {
        0.0
    } else {
        difference
    }
}

/// A time-driven generation's trace maps at one stage, as the nonlinear
/// boundary kick reads them.
struct TemporalTrace<'a> {
    operator: &'a CanonicalTemporalWaveOperator,
    terms: &'a [ConstitutiveTerm],
    runtime: &'a CanonicalMaterialRuntimeState,
}

impl crate::canonical_wave::TraceConstitutive for TemporalTrace<'_> {
    fn discrete_gradient(&self, node: usize, old: f64, new: f64) -> Result<(f64, f64), WaveError> {
        self.operator
            .primary_discrete_gradient(self.terms, node, old, new, self.runtime)
    }

    fn energy(&self, node: usize, flux: f64) -> Result<f64, WaveError> {
        self.operator
            .primary_node_energy(self.terms, node, flux, self.runtime)
    }
}

/// `U = P⁻¹(Q)` at a scalar site. Every executed law is even, so the map is
/// odd and the solve runs on `|Q|`.
fn signed_inverse(site: ConstitutiveSite<'_>, flux: f64) -> Result<f64, ConstitutiveInverseError> {
    site.invert(flux.abs(), f64::NAN)
        .map(|field| field.copysign(flux))
}

/// `v` from independent `b` at one isotropic quadrature sample: the radius
/// from the scalar solve, the direction from `b` itself, and the zero vector
/// explicitly.
fn radial_inverse(
    term: ConstitutiveTerm,
    flux: Point2,
    coefficient: TemporalCoefficientSample,
    runtime: &CanonicalMaterialRuntimeState,
) -> Result<Point2, WaveError> {
    let magnitude = flux.norm();
    if magnitude == 0.0 {
        return Ok(Point2::default());
    }
    let terms = [term];
    let radius = ConstitutiveSite::new(&terms)
        .invert(magnitude, f64::NAN)
        .map_err(|error| inverse_error(error, coefficient, runtime))?;
    Ok(flux * (radius / magnitude))
}

fn inverse_error(
    error: ConstitutiveInverseError,
    coefficient: TemporalCoefficientSample,
    runtime: &CanonicalMaterialRuntimeState,
) -> WaveError {
    let reason = match error {
        ConstitutiveInverseError::OutsideDomain => {
            "the field has left the law's declared amplitude bound"
        }
        ConstitutiveInverseError::NotConverged => "the constitutive inverse did not converge",
        ConstitutiveInverseError::InvalidInput => "the constitutive map is not positive and finite",
    };
    WaveError::MaterialEvaluation {
        material: runtime
            .record(coefficient.material)
            .map_or_else(|_| String::new(), |record| record.material_name.clone()),
        coefficient: "field response",
        point: coefficient.point,
        reason: reason.into(),
    }
}

/// What a field-dependent generation composes today, checked once at
/// compile time so an unsupported combination is refused with its material
/// rather than stepped.
///
/// - A nonlinear complementary law needs an isotropic reference tensor: the
///   radial map `|b| = c·ḡ(|v|)·|v|/j` is the whole vector law only there.
///   Nonlinear anisotropy is Gate C's.
///
/// A nonlinear primary map at an outgoing trace or an absorbing wall is
/// admitted: its kick is the discrete-gradient counterpart of the linear one
/// (`nonlinear_outgoing_kick_with`, `damped_nonlinear_kick`).
fn nonlinear_admission(
    base: &CanonicalWaveOperator,
    _primary: &[TemporalPrimarySample],
    complementary: &[TemporalComplementarySample],
    model: TopologyWaveModel<'_>,
) -> Result<(), WaveError> {
    let name = |material: MaterialId| {
        model
            .material(material)
            .map_or_else(String::new, |material| material.name.clone())
    };
    for (sample, temporal) in base.constitutive_samples().iter().zip(complementary) {
        let coefficient = temporal.coefficient;
        if coefficient.law.field == FieldLawValues::Linear {
            continue;
        }
        let tensor = sample.complementary_inverse;
        let scale = tensor.xx.abs().max(tensor.yy.abs());
        if tensor.xy.abs() > 1e-12 * scale || (tensor.xx - tensor.yy).abs() > 1e-12 * scale {
            return Err(WaveError::MaterialEvaluation {
                material: name(coefficient.material),
                coefficient: "field law",
                point: coefficient.point,
                reason: "a field response on an anisotropic medium awaits its vector law \
                         (Gate C)"
                    .into(),
            });
        }
    }
    Ok(())
}

/// Contribution indices grouped by node, stable within each node, as CSR
/// offsets and entries.
fn group_by_node(
    contributions: &[crate::LinearPrimaryContribution],
    node_count: usize,
) -> (Vec<usize>, Vec<u32>) {
    let mut offsets = vec![0usize; node_count + 1];
    for contribution in contributions {
        offsets[contribution.node as usize + 1] += 1;
    }
    for node in 0..node_count {
        offsets[node + 1] += offsets[node];
    }
    let mut cursor = offsets.clone();
    let mut entries = vec![0u32; contributions.len()];
    for (index, contribution) in contributions.iter().enumerate() {
        let slot = &mut cursor[contribution.node as usize];
        entries[*slot] = index as u32;
        *slot += 1;
    }
    (offsets, entries)
}

fn material_error<T>(
    material: &Material,
    coefficient: &'static str,
    point: Point2,
    reason: &str,
) -> Result<T, WaveError> {
    Err(WaveError::MaterialEvaluation {
        material: material.name.clone(),
        coefficient,
        point,
        reason: reason.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BACKGROUND_REGION, CanonicalSource, CanonicalWaveState, ElectromagneticPolarization,
        FieldLaw, LoopRole, LossChannel, MaterialFrame, MeshingOptions, Obstacle, ObstacleId,
        OuterBoundaryCondition, PeriodicCubicSpline, QuadraticSolutionSnapshot, Region, RegionId,
        ScalarField, SolutionIndicatorJob, SolutionIndicatorOptions, SymmetricTensor2, TimeDrive,
        TimeSignal, enriched_quadratic_basis, mesh_scene, sample_canonical_area,
    };

    fn compile(scene: &Scene) -> Result<CanonicalTemporalWaveOperator, WaveError> {
        let mut base_scene = scene.clone();
        strip_temporal_laws(&mut base_scene.materials);
        let mesh = mesh_scene(
            &base_scene,
            1,
            MeshingOptions {
                target_edge_length: 0.3,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &base_scene,
            OuterBoundaryCondition::Reflecting,
        )?;
        CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, scene, 1)
    }

    fn pump(depth: f64, frequency_hz: f64, phase_radians: f64) -> TimeDrive {
        TimeDrive::ParametricPump {
            depth: ScalarField::constant(depth),
            frequency_hz: ScalarField::constant(frequency_hz),
            phase_radians: ScalarField::constant(phase_radians),
        }
    }

    #[test]
    fn inert_temporal_wrapper_is_exactly_the_fixed_operator() {
        let temporal = compile(&Scene::initial()).unwrap();
        assert!(!temporal.has_temporal_laws());
        let runtime = temporal.initial_runtime();
        let primary = temporal
            .base()
            .primary_mass()
            .iter()
            .enumerate()
            .map(|(index, mass)| mass * (0.2 + index as f64 * 0.001))
            .collect::<Vec<_>>();
        let complementary = temporal
            .base()
            .constitutive_samples()
            .iter()
            .enumerate()
            .map(|(index, _)| Point2::new(index as f64 * 0.002, -0.1))
            .collect::<Vec<_>>();
        assert_eq!(
            temporal.primary_field_at(&primary, 91.0, &runtime).unwrap(),
            temporal.base().primary_field(&primary).unwrap()
        );
        assert_eq!(
            temporal
                .complementary_field_at(&complementary, 91.0, &runtime)
                .unwrap(),
            temporal.base().complementary_field(&complementary).unwrap()
        );
        assert_eq!(
            temporal.force_at(&complementary, 91.0, &runtime).unwrap(),
            temporal.base().force(&complementary).unwrap()
        );
        assert_eq!(
            temporal.loss_rates_at(91.0, &runtime).unwrap(),
            CanonicalTemporalLossRates {
                primary: temporal.base().primary_loss_rate().to_vec(),
                complementary: temporal.base().complementary_loss_rate().to_vec(),
            }
        );
    }

    #[test]
    fn frozen_time_filter_reduces_energy_and_preserves_primary_total() {
        let mut scene = Scene::initial();
        scene.materials[0].mass_law.drive = TimeDrive::TravellingModulation {
            depth: ScalarField::constant(0.22),
            frequency_hz: ScalarField::constant(0.8),
            phase_radians: ScalarField::constant(0.31),
            wavenumber: ScalarField::constant(2.6),
            angle_radians: ScalarField::constant(-0.37),
        };
        scene.materials[0].stiffness_law.drive = TimeDrive::TimeCrystal {
            depth: ScalarField::constant(0.16),
            frequency_hz: ScalarField::constant(0.55),
            phase_radians: ScalarField::constant(-0.21),
            sharpness: ScalarField::constant(2.9),
        };
        let temporal = compile(&scene).unwrap();
        let time_step = 0.35 * temporal.maximum_time_step();
        let initial_runtime = temporal.initial_runtime();
        let instantaneous_mass = temporal.primary_mass_at(0.0, &initial_runtime).unwrap();
        let constant_primary = instantaneous_mass
            .iter()
            .map(|mass| 0.037 * mass)
            .collect::<Vec<_>>();
        let mut constant_state = CanonicalTemporalWaveState::new(
            &temporal,
            time_step,
            constant_primary.clone(),
            vec![Point2::default(); temporal.base().complementary_degrees_of_freedom()],
        )
        .unwrap();
        assert_eq!(
            constant_state.apply_grid_filter(&temporal, 0.72).unwrap(),
            0.0
        );
        assert_eq!(constant_state.primary_flux(), constant_primary);

        let potential = (0..temporal.base().degrees_of_freedom())
            .map(|index| (index as f64 * 1.713).sin())
            .collect::<Vec<_>>();
        let compatible = temporal.base().compatible_flux(&potential).unwrap();
        let mut compatible_state = CanonicalTemporalWaveState::new(
            &temporal,
            time_step,
            vec![0.0; temporal.base().degrees_of_freedom()],
            compatible,
        )
        .unwrap();
        compatible_state.apply_grid_filter(&temporal, 0.72).unwrap();
        let stationary = temporal
            .base()
            .stationary_complementary_component(compatible_state.complementary_flux())
            .unwrap();
        let stationary_norm = stationary
            .iter()
            .map(|value| value.norm().powi(2))
            .sum::<f64>()
            .sqrt();
        let compatible_norm = compatible_state
            .complementary_flux()
            .iter()
            .map(|value| value.norm().powi(2))
            .sum::<f64>()
            .sqrt();
        assert!(
            stationary_norm < 2.0e-10 * compatible_norm.max(1.0),
            "stationary residual {stationary_norm:e} for compatible norm {compatible_norm:e}"
        );

        let arbitrary = (0..temporal.base().complementary_degrees_of_freedom())
            .map(|index| Point2::new((index as f64 * 1.137).sin(), (index as f64 * 0.831).cos()))
            .collect::<Vec<_>>();
        let stationary = temporal
            .base()
            .stationary_complementary_component(&arbitrary)
            .unwrap();
        let mut stationary_state = CanonicalTemporalWaveState::new(
            &temporal,
            time_step,
            vec![0.0; temporal.base().degrees_of_freedom()],
            stationary.clone(),
        )
        .unwrap();
        stationary_state.apply_grid_filter(&temporal, 0.72).unwrap();
        assert!(
            stationary_state
                .complementary_flux()
                .iter()
                .zip(stationary)
                .all(|(actual, expected)| (*actual - expected).norm() < 2.0e-10)
        );

        let primary = temporal
            .base()
            .primary_mass()
            .iter()
            .enumerate()
            .map(|(index, mass)| mass * (0.03 + 0.025 * (index as f64 * 2.173).sin()))
            .collect::<Vec<_>>();
        let complementary = temporal
            .base()
            .constitutive_samples()
            .iter()
            .enumerate()
            .map(|(index, _)| {
                Point2::new(
                    0.018 * (index as f64 * 1.713).cos(),
                    -0.014 * (index as f64 * 1.291).sin(),
                )
            })
            .collect::<Vec<_>>();
        let mut state =
            CanonicalTemporalWaveState::new(&temporal, time_step, primary, complementary).unwrap();
        for _ in 0..7 {
            state.step(&temporal).unwrap();
        }
        let before = state.energy(&temporal).unwrap();
        let total = state.primary_flux().iter().sum::<f64>();
        let time = state.time();
        let removed = state.apply_grid_filter(&temporal, 0.72).unwrap();
        let after = state.energy(&temporal).unwrap();
        let next_total = state.primary_flux().iter().sum::<f64>();
        assert!(removed > 0.0);
        assert!(after < before);
        assert_eq!(state.time(), time);
        assert!((next_total - total).abs() < 2.0e-12 * total.abs().max(1.0));

        let unchanged = state.clone();
        assert_eq!(state.apply_grid_filter(&temporal, 0.0).unwrap(), 0.0);
        assert_eq!(state, unchanged);
    }

    /// The reason the consumers carry per-sample data at all. A travelling
    /// complementary drive gives each of an element's six samples a different
    /// instantaneous factor, so inverting at the samples and interpolating the
    /// physical field is not the same as interpolating the flux and inverting
    /// once at the probe. Nonlinear laws cannot do the latter at all.
    #[test]
    fn temporal_point_consumer_inverts_at_the_samples_and_weighs_density_at_the_point() {
        let mut scene = Scene::initial();
        scene.materials[0].mass_law.drive = TimeDrive::TravellingModulation {
            depth: ScalarField::constant(0.24),
            frequency_hz: ScalarField::constant(0.8),
            phase_radians: ScalarField::constant(0.31),
            wavenumber: ScalarField::constant(2.7),
            angle_radians: ScalarField::constant(-0.4),
        };
        scene.materials[0].stiffness_law.drive = TimeDrive::TimeCrystal {
            depth: ScalarField::constant(0.17),
            frequency_hz: ScalarField::constant(0.6),
            phase_radians: ScalarField::constant(-0.23),
            sharpness: ScalarField::constant(3.2),
        };
        let operator = compile(&scene).unwrap();
        let barycentric = [0.19, 0.33, 0.48];
        let nodes = operator.base().element_nodes()[0];
        let quadratic = QuadraticPointStencil {
            element: 0,
            barycentric,
            nodes,
            value_weights: enriched_quadratic_basis(barycentric),
            gradient_weights: [Point2::default(); 7],
            region: BACKGROUND_REGION,
            mass_density: 2.3,
            stiffness: SymmetricTensor2::new(1.7, 0.18, 1.25),
        };
        let stencil = CanonicalTemporalPointStencil::from_quadratic(quadratic, &operator).unwrap();
        let expected_point = stencil
            .fixed()
            .primary_weights
            .iter()
            .zip(nodes)
            .fold(Point2::default(), |sum, (weight, node)| {
                sum + operator.base().node_points()[node as usize] * *weight
            });
        let coordinates = stencil.primary_coefficient().coordinates;
        assert!((Point2::new(coordinates.x, coordinates.y) - expected_point).norm() < 2.0e-12);

        let runtime = operator.initial_runtime();
        let time_step = 0.21 * operator.maximum_time_step();
        let time = 0.37;
        let current_value = 0.42;
        let previous_value = 0.39;
        let primary = operator
            .primary_mass_at(time, &runtime)
            .unwrap()
            .into_iter()
            .map(|mass| mass * current_value)
            .collect::<Vec<_>>();
        let previous = operator
            .primary_mass_at(time - time_step, &runtime)
            .unwrap()
            .into_iter()
            .map(|mass| mass * previous_value)
            .collect::<Vec<_>>();
        let flux = Point2::new(0.31, -0.22);
        let complementary = vec![flux; operator.base().complementary_degrees_of_freedom()];
        let sample = stencil
            .sample(
                &operator,
                &primary,
                &previous,
                &complementary,
                time,
                time_step,
                &runtime,
            )
            .unwrap();
        let fixed = stencil.fixed();
        let primary_factor = stencil
            .primary_coefficient()
            .factor_at(time, &runtime)
            .unwrap();
        let complementary_factor = stencil
            .complementary_coefficient()
            .factor_at(time, &runtime)
            .unwrap();
        // The rule, spelled out: invert at each of the element's own samples,
        // divide by that sample's own factor, interpolate the physical field,
        // and only then evaluate the density with the probe point's forward
        // coefficient.
        let mut expected_complementary = Point2::default();
        for local in 0..6 {
            let factor = stencil.sample_coefficients()[local]
                .factor_at(time, &runtime)
                .unwrap();
            expected_complementary = expected_complementary
                + fixed.sample_inverses[local].apply(flux) / factor
                    * fixed.complementary_weights[local];
        }
        let expected_energy = 0.5
            * (fixed.primary_reference * primary_factor * current_value.powi(2)
                + complementary_factor
                    * fixed
                        .complementary_reference
                        .quadratic_form(expected_complementary));
        assert!((sample.primary - current_value).abs() < 2.0e-12);
        assert!(
            (sample.primary_rate - (current_value - previous_value) / time_step).abs() < 2.0e-12
        );
        assert!((sample.complementary - expected_complementary).norm() < 2.0e-12);
        assert!((sample.energy_density - expected_energy).abs() < 2.0e-12);
        assert!(
            (sample.energy_flow
                - Point2::new(-expected_complementary.y, expected_complementary.x)
                    * (fixed.orientation * current_value))
                .norm()
                < 2.0e-12
        );
    }

    /// The scalar estimator's own material samples, which it takes from the
    /// scene rather than from the operator.
    ///
    /// Two contracts, checked on each constitutive row in turn. A uniform pump
    /// is exactly a scene whose coefficient was authored at the pumped value,
    /// so every error term has to come out identical to that scene's - that is
    /// what makes the instantaneous samples a reconstruction of the medium
    /// rather than a correction to it. And the wavelength limit has to stay on
    /// the authored speed, because a limit that breathed with the drive would
    /// retarget the same element every cycle.
    ///
    /// Running both rows is the point. They scale in opposite directions - the
    /// primary row multiplies the mass, while the complementary row is
    /// authored on the reciprocal stiffness, so its factor divides the
    /// stiffness tensor - and a test on one row alone cannot see the other's
    /// direction at all.
    #[test]
    fn instantaneous_samples_match_a_scene_authored_at_the_driven_value() {
        const BASE_MASS: f64 = 1.3;
        const BASE_STIFFNESS: f64 = 0.7;
        for mass_row in [true, false] {
            let mut driven = Scene::default();
            driven.materials[0].mass_density = ScalarField::constant(BASE_MASS);
            driven.materials[0].stiffness = ScalarField::constant(BASE_STIFFNESS);
            let pump = TimeDrive::ParametricPump {
                depth: ScalarField::constant(0.2),
                frequency_hz: ScalarField::constant(0.8),
                phase_radians: ScalarField::constant(0.4),
            };
            if mass_row {
                driven.materials[0].mass_law.drive = pump;
            } else {
                driven.materials[0].stiffness_law.drive = pump;
            }
            let mut authored = driven.clone();
            strip_temporal_laws(&mut authored.materials);

            let mesh = std::sync::Arc::new(
                mesh_scene(
                    &authored,
                    1,
                    MeshingOptions {
                        target_edge_length: 0.3,
                        ..MeshingOptions::default()
                    },
                )
                .unwrap(),
            );
            let quadratic = std::sync::Arc::new(
                QuadraticWaveOperator::assemble_scene(
                    &mesh,
                    &authored,
                    OuterBoundaryCondition::Reflecting,
                )
                .unwrap(),
            );
            let temporal =
                CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &driven, 1)
                    .unwrap();
            let runtime = temporal.initial_runtime();
            let time = 0.37;

            // The pump is uniform in space, so the whole row moves by one
            // factor. Reading it off the operator's own two evaluations of
            // that row derives the expectation from a path that is not the one
            // under test.
            let factor = if mass_row {
                let pumped = temporal.primary_mass_at(time, &runtime).unwrap();
                let base = temporal.base().primary_mass();
                let factor = pumped[0] / base[0];
                for (pumped, base) in pumped.iter().zip(base) {
                    assert!((pumped / base - factor).abs() < 1.0e-12);
                }
                factor
            } else {
                let probe =
                    vec![Point2::new(1.0, 0.0); temporal.base().complementary_degrees_of_freedom()];
                let pumped = temporal
                    .complementary_field_at(&probe, time, &runtime)
                    .unwrap();
                let base = temporal.base().complementary_field(&probe).unwrap();
                let factor = base[0].x / pumped[0].x;
                for (pumped, base) in pumped.iter().zip(&base) {
                    assert!((base.x / pumped.x - factor).abs() < 1.0e-12);
                }
                factor
            };
            assert!(factor.is_finite() && (factor - 1.0).abs() > 1.0e-3);

            let mut equivalent = authored.clone();
            if mass_row {
                equivalent.materials[0].mass_density = ScalarField::constant(BASE_MASS * factor);
            } else {
                equivalent.materials[0].stiffness = ScalarField::constant(BASE_STIFFNESS / factor);
            }

            let count = quadratic.degrees_of_freedom();
            let points = quadratic.node_points();
            let snapshot = QuadraticSolutionSnapshot {
                mesh_revision: mesh.mesh_revision,
                displacement: points
                    .iter()
                    .map(|point| 0.08 * (1.7 * point.x - 1.1 * point.y).sin())
                    .collect(),
                velocity: points
                    .iter()
                    .map(|point| 0.05 * (0.9 * point.x + 1.4 * point.y).cos())
                    .collect(),
                acceleration: vec![0.0; count],
                auxiliary: vec![0.0; count],
                volume_acceleration: vec![0.0; count],
                time,
                time_step: 1.0e-3,
            };
            let options = SolutionIndicatorOptions {
                resolved_frequency_hz: 2.0,
                ..SolutionIndicatorOptions::default()
            };
            let report = |scene: &Scene, runtime: Option<&CanonicalMaterialRuntimeState>| {
                let mut job = SolutionIndicatorJob::new(
                    mesh.clone(),
                    quadratic.clone(),
                    scene.clone(),
                    snapshot.clone(),
                    options,
                );
                if let Some(runtime) = runtime {
                    job = job.with_instantaneous_materials(runtime.clone());
                }
                loop {
                    if let Some(result) = job.advance(8_192) {
                        return result.unwrap().report;
                    }
                }
            };

            let instantaneous = report(&driven, Some(&runtime));
            let equivalent = report(&equivalent, None);
            let close = |left: f64, right: f64| {
                (left - right).abs() <= 1.0e-12 * left.abs().max(right.abs()).max(1.0)
            };
            assert!(close(instantaneous.total_energy, equivalent.total_energy));
            assert!(close(
                instantaneous.global_indicator,
                equivalent.global_indicator
            ));
            assert!(close(
                instantaneous.interior_jump_contribution,
                equivalent.interior_jump_contribution
            ));
            assert!(close(
                instantaneous.displacement_recovery_contribution,
                equivalent.displacement_recovery_contribution
            ));

            // A pumped row moves the wave speed, so the equivalent scene's
            // wavelength target moved with it. The driven run's did not,
            // because the limit reads the authored medium.
            let inert = report(&authored, None);
            assert!(close(
                instantaneous.smallest_wavelength_target,
                inert.smallest_wavelength_target
            ));
            assert!(!close(
                instantaneous.smallest_wavelength_target,
                equivalent.smallest_wavelength_target
            ));
        }
    }

    /// The substitution that makes the estimate survive a patterned medium:
    /// under a runtime the gradient-based terms read the solver's own flux
    /// instead of differentiating the reconstructed scalar field.
    ///
    /// Three things have to hold at once. The scalar interior jump has to step
    /// aside entirely, or its stalling term would still dominate. The scalar
    /// displacement recovery has to leave the total while staying in the
    /// breakdown, because that is what shows the substitution happened. And a
    /// runtime paired with a supplement that carries no flux jump must change
    /// nothing at all, or a caller could silently replace the scalar terms
    /// with nothing.
    #[test]
    fn a_runtime_moves_the_gradient_terms_onto_the_solver_flux() {
        let mut scene = Scene::default();
        scene.materials[0].mass_law.drive = TimeDrive::TravellingModulation {
            depth: ScalarField::constant(0.22),
            frequency_hz: ScalarField::constant(0.9),
            phase_radians: ScalarField::constant(0.15),
            wavenumber: ScalarField::constant(3.0),
            angle_radians: ScalarField::constant(0.3),
        };
        let mut authored = scene.clone();
        strip_temporal_laws(&mut authored.materials);

        let mesh = std::sync::Arc::new(
            mesh_scene(
                &authored,
                1,
                MeshingOptions {
                    target_edge_length: 0.3,
                    ..MeshingOptions::default()
                },
            )
            .unwrap(),
        );
        let quadratic = std::sync::Arc::new(
            QuadraticWaveOperator::assemble_scene(
                &mesh,
                &authored,
                OuterBoundaryCondition::Reflecting,
            )
            .unwrap(),
        );
        let temporal =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();
        let runtime = temporal.initial_runtime();
        let base = temporal.base();
        let time_step = 0.4 * temporal.maximum_time_step();
        let primary = base
            .node_points()
            .iter()
            .map(|point| 0.07 + 0.04 * (1.9 * point.x - 1.2 * point.y).sin())
            .collect::<Vec<_>>();
        let potential = base
            .node_points()
            .iter()
            .map(|point| 0.03 * (1.4 * point.x + 1.1 * point.y).cos())
            .collect::<Vec<_>>();
        let state =
            CanonicalWaveState::from_primary_and_potential(base, time_step, &primary, &potential)
                .unwrap();
        let mut next = state.clone();
        next.step(base).unwrap();
        let snapshot = CanonicalIndicatorSnapshot {
            mesh_revision: mesh.mesh_revision,
            primary_flux: next.primary_flux().to_vec(),
            previous_primary_flux: state.primary_flux().to_vec(),
            complementary_flux: next.complementary_flux().to_vec(),
            previous_complementary_flux: state.complementary_flux().to_vec(),
            auxiliary: vec![],
            previous_auxiliary: vec![],
            integrated_field: vec![],
            previous_integrated_field: vec![],
            time: time_step,
            time_step,
        };
        let supplement = canonical_temporal_indicator_supplement(
            &mesh,
            &temporal,
            &CanonicalForcing::none(temporal.base()),
            &snapshot,
            &runtime,
            0.0,
        )
        .unwrap();
        assert!(supplement.element_complementary_jump.is_some());
        assert!(supplement.complementary_jump_contribution > 0.0);

        let count = quadratic.degrees_of_freedom();
        let field = temporal
            .primary_field_at(&snapshot.primary_flux, snapshot.time, &runtime)
            .unwrap();
        let scalar_snapshot = QuadraticSolutionSnapshot {
            mesh_revision: mesh.mesh_revision,
            displacement: field,
            velocity: vec![0.0; count],
            acceleration: vec![0.0; count],
            auxiliary: vec![0.0; count],
            volume_acceleration: vec![0.0; count],
            time: snapshot.time,
            time_step,
        };
        let options = SolutionIndicatorOptions::default();
        let report = |supplement: CanonicalIndicatorSupplement, driven: bool| {
            let mut job = SolutionIndicatorJob::new(
                mesh.clone(),
                quadratic.clone(),
                if driven {
                    scene.clone()
                } else {
                    authored.clone()
                },
                scalar_snapshot.clone(),
                options,
            )
            .with_canonical_supplement(supplement);
            if driven {
                job = job.with_instantaneous_materials(runtime.clone());
            }
            loop {
                if let Some(result) = job.advance(8_192) {
                    return result.unwrap().report;
                }
            }
        };

        let substituted = report(supplement.clone(), true);
        // The scalar jump contributed nothing, so the whole interior jump is
        // the flux one.
        let close = |left: f64, right: f64| {
            (left - right).abs() <= 1.0e-12 * left.abs().max(right.abs()).max(1.0)
        };
        assert!(close(
            substituted.interior_jump_contribution,
            substituted.complementary_jump_contribution
        ));
        // Measured but not counted.
        assert!(substituted.displacement_recovery_contribution > 0.0);
        assert!(close(
            substituted.recovery_contribution,
            supplement.complementary_recovery_contribution
        ));

        // The reported indicator is the calibrated one, and the calibration
        // applies exactly where the substitution does. Checking it against the
        // report's own residual and energy is what keeps the constant from
        // drifting away from the number it is documented as.
        let raw = |report: &crate::SolutionIndicatorReport| {
            (report.total_residual / report.total_energy).sqrt()
        };
        let calibration = substituted.global_indicator / raw(&substituted);
        assert!(
            (calibration - 1.88).abs() < 1.0e-9,
            "the driven estimate must carry its calibration, got {calibration}"
        );

        // Same supplement, no runtime: the scalar terms keep the estimate and
        // the flux jump is not folded in on top of them.
        let scalar = report(supplement.clone(), false);
        assert!((scalar.global_indicator / raw(&scalar) - 1.0).abs() < 1.0e-9);
        assert!(scalar.interior_jump_contribution > substituted.interior_jump_contribution);
        assert!(scalar.recovery_contribution > substituted.recovery_contribution);

        // A runtime with a supplement that carries no flux jump must not
        // substitute, or the estimate would silently lose its gradient terms.
        let mut without = supplement.clone();
        without.element_complementary_jump = None;
        let unsubstituted = report(without, true);
        assert!(unsubstituted.interior_jump_contribution > substituted.interior_jump_contribution);
        assert!(unsubstituted.displacement_recovery_contribution > 0.0);
        assert!(
            unsubstituted.recovery_contribution > supplement.complementary_recovery_contribution
        );
    }

    /// An inert medium must not notice the runtime at all, or the temporal
    /// path would report a different error for the same physics.
    #[test]
    fn instantaneous_samples_leave_an_inert_medium_untouched() {
        let scene = Scene::default();
        let mesh = std::sync::Arc::new(
            mesh_scene(
                &scene,
                1,
                MeshingOptions {
                    target_edge_length: 0.3,
                    ..MeshingOptions::default()
                },
            )
            .unwrap(),
        );
        let quadratic = std::sync::Arc::new(
            QuadraticWaveOperator::assemble_scene(
                &mesh,
                &scene,
                OuterBoundaryCondition::Reflecting,
            )
            .unwrap(),
        );
        let temporal =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();
        let runtime = temporal.initial_runtime();

        let count = quadratic.degrees_of_freedom();
        let points = quadratic.node_points();
        let snapshot = QuadraticSolutionSnapshot {
            mesh_revision: mesh.mesh_revision,
            displacement: points
                .iter()
                .map(|point| 0.08 * (1.7 * point.x - 1.1 * point.y).sin())
                .collect(),
            velocity: points
                .iter()
                .map(|point| 0.05 * (0.9 * point.x + 1.4 * point.y).cos())
                .collect(),
            acceleration: vec![0.0; count],
            auxiliary: vec![0.0; count],
            volume_acceleration: vec![0.0; count],
            time: 0.37,
            time_step: 1.0e-3,
        };
        let options = SolutionIndicatorOptions {
            resolved_frequency_hz: 2.0,
            ..SolutionIndicatorOptions::default()
        };
        let report = |runtime: Option<&CanonicalMaterialRuntimeState>| {
            let mut job = SolutionIndicatorJob::new(
                mesh.clone(),
                quadratic.clone(),
                scene.clone(),
                snapshot.clone(),
                options,
            );
            if let Some(runtime) = runtime {
                job = job.with_instantaneous_materials(runtime.clone());
            }
            loop {
                if let Some(result) = job.advance(8_192) {
                    return result.unwrap().report;
                }
            }
        };
        assert_eq!(report(Some(&runtime)), report(None));
    }

    /// The composition's own falsifier, with every other physics removed.
    ///
    /// Hold the whole field at one constant value through prescribed nodes
    /// everywhere and pump the mass. The truth is known without a solver: the
    /// field is that constant, the complementary flux stays zero, no wave ever
    /// moves, and the energy is `0.5 M(t) g^2`, which breathes because `M`
    /// does. Every joule that moves came across the prescribed boundary.
    ///
    /// This is what separates the two accounting lanes. Charging that breathing
    /// to temporal work would make the balance close for entirely the wrong
    /// reason, and no fixture with a real wave in it could tell the difference.
    #[test]
    fn a_constant_prescribed_field_over_a_pumped_mass_is_all_boundary_exchange() {
        let mut scene = Scene::default();
        scene.materials[0].mass_law.drive = TimeDrive::ParametricPump {
            depth: ScalarField::constant(0.3),
            frequency_hz: ScalarField::constant(1.1),
            phase_radians: ScalarField::constant(0.35),
        };
        let operator = compile(&scene).unwrap();
        let base = operator.base();
        const HELD: f64 = 0.4;

        // Every node prescribed, so nothing is free to evolve.
        let prescribed = vec![
            Some(TimeSignal::Harmonic {
                offset: HELD,
                amplitude: 0.0,
                frequency_hz: 0.0,
                phase_radians: 0.0,
            });
            base.degrees_of_freedom()
        ];
        let forcing = CanonicalForcing::from_prescribed(base, prescribed).unwrap();

        let runtime = operator.initial_runtime();
        let ceiling = 0.4 * operator.maximum_time_step();
        let target = 0.35;
        let mut previous: Option<(f64, f64)> = None;
        for refinement in [1.0, 0.5, 0.25] {
            let steps = (target / (ceiling * refinement)).ceil() as u64;
            let time_step = target / steps as f64;
            let primary = operator
                .primary_mass_at(0.0, &runtime)
                .unwrap()
                .iter()
                .map(|mass| mass * HELD)
                .collect::<Vec<_>>();
            let complementary = vec![Point2::default(); base.complementary_degrees_of_freedom()];
            let mut state =
                CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary)
                    .unwrap();
            let before = state.energy(&operator).unwrap();
            let mut exchange = 0.0;
            let mut work = 0.0;
            for _ in 0..steps {
                let accounting = state.step_with_forcing(&operator, &forcing).unwrap();
                exchange += accounting.prescribed_exchange;
                work += accounting.temporal_work;
            }
            let after = state.energy(&operator).unwrap();

            // The field never moved, so nothing radiated and it is still held.
            assert!(
                state
                    .complementary_flux()
                    .iter()
                    .all(|flux| flux.norm() < 1.0e-12),
                "a held field must not radiate"
            );
            let mass = operator.primary_mass_at(state.time(), &runtime).unwrap();
            for (flux, mass) in state.primary_flux().iter().zip(&mass) {
                assert!((flux / mass - HELD).abs() < 1.0e-12);
            }

            // Both lanes are genuinely active, and in the ratio the stage
            // equations predict: holding `u` fixed while `M` breathes gives
            // the drive `-M' g^2 / 2` and the boundary `+M' g^2`.
            assert!(work.abs() > 1.0e-3, "the fixture must pump, got {work}");
            assert!(
                (exchange + 2.0 * work).abs() < 2.0e-2 * work.abs(),
                "exchange should be twice the drive and opposite, got {exchange} against {work}"
            );

            let residual = after - before - work - exchange;
            if let Some((coarse_step, coarse_residual)) = previous {
                let order = (coarse_residual.abs() / residual.abs()).log2()
                    / (coarse_step / time_step).log2();
                assert!(
                    order > 1.7,
                    "the unaccounted remainder must be the splitting's own, which is second \
                     order; measured {order:.2} between dt {coarse_step:.3e} and {time_step:.3e}"
                );
            }
            previous = Some((time_step, residual));
        }
    }

    /// An inert generation must step a source and a prescribed wall exactly as
    /// the fixed path does, or the two paths mean different things by the same
    /// scene and no comparison between them is worth anything.
    ///
    /// One deliberate difference is checked rather than hidden: the fixed path
    /// samples a prescribed signal half a step in on the first kick, and the
    /// temporal path samples it at the stage. With a constant signal the two
    /// coincide exactly, which is what this pins down; a varying signal differs
    /// at second order, which is the accuracy both paths already claim.
    #[test]
    fn an_inert_generation_steps_forcing_exactly_as_the_fixed_path_does() {
        let scene = Scene::default();
        let operator = compile(&scene).unwrap();
        let base = operator.base();
        let count = base.degrees_of_freedom();

        // A wall of prescribed nodes on one side, at a constant value.
        let mut prescribed = vec![None; count];
        for (node, point) in base.node_points().iter().enumerate() {
            if point.x < -0.999 {
                prescribed[node] = Some(TimeSignal::Harmonic {
                    offset: 0.17,
                    amplitude: 0.0,
                    frequency_hz: 0.0,
                    phase_radians: 0.0,
                });
            }
        }
        assert!(
            prescribed.iter().any(Option::is_some),
            "the fixture needs a prescribed wall"
        );
        let mut forcing = CanonicalForcing::from_prescribed(base, prescribed).unwrap();
        // And a volume source, so both exchange lanes are exercised at once.
        forcing
            .push_source(
                CanonicalSource::direct(
                    base,
                    base.primary_mass().to_vec(),
                    TimeSignal::harmonic(0.0, 0.9, 1.7, 0.4),
                )
                .unwrap(),
            )
            .unwrap();

        let time_step = 0.4 * operator.maximum_time_step();
        let primary = base
            .node_points()
            .iter()
            .map(|point| 0.06 * (1.3 * point.x - 0.9 * point.y).sin())
            .collect::<Vec<_>>();
        let potential = base
            .node_points()
            .iter()
            .map(|point| 0.03 * (0.8 * point.x + 1.2 * point.y).cos())
            .collect::<Vec<_>>();
        let complementary = base.compatible_flux(&potential).unwrap();

        // Both paths start from a state that already satisfies the constraint,
        // which is what the constrained system's initial condition means. The
        // fixed path would otherwise absorb the violation into its first step.
        let temporal =
            CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary.clone())
                .unwrap();
        let mut temporal = temporal.pinned(&operator, &forcing).unwrap();
        let mut fixed = CanonicalWaveState::new(
            base,
            time_step,
            temporal.primary_flux().to_vec(),
            complementary,
        )
        .unwrap();

        for _ in 0..24 {
            let temporal_accounting = temporal.step_with_forcing(&operator, &forcing).unwrap();
            let fixed_accounting = fixed.step_with_forcing(base, &forcing).unwrap();
            assert!(
                (temporal_accounting.source_work - fixed_accounting.source_work).abs() < 1.0e-12
            );
            assert!(
                (temporal_accounting.prescribed_exchange - fixed_accounting.prescribed_exchange)
                    .abs()
                    < 1.0e-12
            );
            // An inert medium does no temporal work, whatever else it does.
            assert!(temporal_accounting.temporal_work.abs() < 1.0e-12);
        }
        for (temporal, fixed) in temporal.primary_flux().iter().zip(fixed.primary_flux()) {
            assert!((temporal - fixed).abs() < 1.0e-12);
        }
        for (temporal, fixed) in temporal
            .complementary_flux()
            .iter()
            .zip(fixed.complementary_flux())
        {
            assert!((*temporal - *fixed).norm() < 1.0e-12);
        }
    }

    /// A dissipating medium whose rate is itself driven.
    ///
    /// The specification concedes first-order accuracy for a varying loss rate
    /// and keeps the second-order lossless step. That concession is about
    /// resolving the *rate*, not about the accounting, so both are measured
    /// rather than assumed: the composed step keeps a second-order energy
    /// balance and the dissipation lanes stay passive.
    #[test]
    fn a_driven_loss_dissipates_passively_and_still_balances_to_second_order() {
        let mut scene = Scene::default();
        scene.materials[0].mass_law.drive = TimeDrive::ParametricPump {
            depth: ScalarField::constant(0.25),
            frequency_hz: ScalarField::constant(1.3),
            phase_radians: ScalarField::constant(0.2),
        };
        scene.materials[0].electric_loss = Some(LossChannel {
            base_rate: ScalarField::constant(0.6),
            law: DampingLaw {
                rate: RateLaw::Constant,
                drive: TimeDrive::ParametricPump {
                    depth: ScalarField::constant(0.5),
                    frequency_hz: ScalarField::constant(0.8),
                    phase_radians: ScalarField::constant(-0.3),
                },
            },
        });
        let operator = compile(&scene).unwrap();
        assert!(
            operator.forced_composition_supported(),
            "a dissipating generation must still be steppable"
        );
        assert!(
            !operator.conservative_bulk_supported(),
            "but it is not a conservative bulk"
        );
        let base = operator.base();
        let forcing = CanonicalForcing::none(base);

        let ceiling = 0.4 * operator.maximum_time_step();
        let target = 0.3;
        let mut previous: Option<(f64, f64)> = None;
        for refinement in [1.0, 0.5, 0.25] {
            let steps = (target / (ceiling * refinement)).ceil() as u64;
            let time_step = target / steps as f64;
            let primary = base
                .node_points()
                .iter()
                .map(|point| 0.07 * (1.5 * point.x - 1.1 * point.y).sin())
                .collect::<Vec<_>>();
            let potential = base
                .node_points()
                .iter()
                .map(|point| 0.04 * (0.9 * point.x + 1.3 * point.y).cos())
                .collect::<Vec<_>>();
            let complementary = base.compatible_flux(&potential).unwrap();
            let mut state =
                CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary)
                    .unwrap();

            let before = state.energy(&operator).unwrap();
            let mut work = 0.0;
            let mut removed = 0.0;
            for _ in 0..steps {
                let accounting = state.step_with_forcing(&operator, &forcing).unwrap();
                assert!(
                    accounting.primary_loss >= 0.0 && accounting.complementary_loss >= 0.0,
                    "a passive channel cannot add energy"
                );
                work += accounting.temporal_work;
                removed += accounting.primary_loss + accounting.complementary_loss;
            }
            let after = state.energy(&operator).unwrap();
            assert!(removed > 1.0e-6, "the fixture must actually dissipate");

            let residual = after - before - work + removed;
            if let Some((coarse_step, coarse_residual)) = previous {
                let order = (coarse_residual.abs() / residual.abs()).log2()
                    / (coarse_step / time_step).log2();
                assert!(
                    order > 1.7,
                    "the composed step must keep its second-order balance, measured \
                     {order:.2} between dt {coarse_step:.3e} and {time_step:.3e}"
                );
            }
            previous = Some((time_step, residual));
        }
    }

    /// The dissipation map must reduce to the fixed path's exponential when
    /// nothing is driven, or the same scene would decay differently depending
    /// on which stepper ran it.
    #[test]
    fn an_inert_constant_loss_decays_exactly_as_the_fixed_path_does() {
        let mut scene = Scene::default();
        scene.materials[0].damping = ScalarField::constant(0.45);
        let operator = compile(&scene).unwrap();
        let base = operator.base();
        let forcing = CanonicalForcing::none(base);
        let time_step = 0.4 * operator.maximum_time_step();

        let primary = base
            .node_points()
            .iter()
            .map(|point| 0.05 * (1.2 * point.x + 0.7 * point.y).sin())
            .collect::<Vec<_>>();
        let potential = base
            .node_points()
            .iter()
            .map(|point| 0.03 * (1.1 * point.x - 0.6 * point.y).cos())
            .collect::<Vec<_>>();
        let complementary = base.compatible_flux(&potential).unwrap();
        let mut temporal = CanonicalTemporalWaveState::new(
            &operator,
            time_step,
            primary.clone(),
            complementary.clone(),
        )
        .unwrap();
        let mut fixed = CanonicalWaveState::new(base, time_step, primary, complementary).unwrap();

        for _ in 0..24 {
            let temporal_accounting = temporal.step_with_forcing(&operator, &forcing).unwrap();
            let fixed_accounting = fixed.step_with_forcing(base, &forcing).unwrap();
            assert!(
                (temporal_accounting.primary_loss - fixed_accounting.primary_loss).abs() < 1.0e-12
            );
            assert!(
                (temporal_accounting.complementary_loss - fixed_accounting.complementary_loss)
                    .abs()
                    < 1.0e-12
            );
        }
        for (temporal, fixed) in temporal.primary_flux().iter().zip(fixed.primary_flux()) {
            assert!((temporal - fixed).abs() < 1.0e-12);
        }
        for (temporal, fixed) in temporal
            .complementary_flux()
            .iter()
            .zip(fixed.complementary_flux())
        {
            assert!((*temporal - *fixed).norm() < 1.0e-12);
        }
    }

    /// What the frozen reference impedance costs, as far as this fixture can
    /// say.
    ///
    /// An absorbing wall is assembled from the medium it was built against and
    /// stays there; a medium that has since moved leaves the wall mistuned by
    /// exactly that ratio. The specification permits this only as a documented,
    /// tested approximation, so the cost is measured rather than asserted.
    ///
    /// A Switch is the right instrument: it moves the mass to a new constant
    /// value, so the mismatch is steady rather than smeared over a drive's
    /// cycle. Two confounds had to be removed before the signal appeared at
    /// all. Cumulative escape says nothing, because reflected energy simply
    /// leaves on its next encounter; and a heavier medium is slower by
    /// `sqrt(f)`, so a fixed clock scores a wave that has not yet reached the
    /// wall as reflected.
    #[test]
    fn a_frozen_wall_reflects_more_as_the_medium_moves_away_from_it() {
        let mut remaining = Vec::new();
        for alternate in [1.0, 2.2, 6.0] {
            let mut scene = Scene::default();
            if alternate != 1.0 {
                scene.materials[0].mass_law.alternate = Some(ScalarField::constant(alternate));
            }
            scene.materials[0].switch_ramp = 0.0;
            let mut base_scene = scene.clone();
            strip_temporal_laws(&mut base_scene.materials);
            let mesh = mesh_scene(
                &base_scene,
                1,
                MeshingOptions {
                    target_edge_length: 0.22,
                    ..MeshingOptions::default()
                },
            )
            .unwrap();
            let quadratic = QuadraticWaveOperator::assemble_scene(
                &mesh,
                &base_scene,
                OuterBoundaryCondition::FirstOrderOutgoing,
            )
            .unwrap();
            let operator =
                CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();
            assert!(
                operator.forced_composition_supported(),
                "an absorbing wall must be steppable"
            );
            let base = operator.base();
            let forcing = CanonicalForcing::none(base);
            let time_step = 0.4 * operator.maximum_time_step();

            // A smooth blob in the middle, so what leaves is radiation rather
            // than a boundary artefact.
            let primary = base
                .node_points()
                .iter()
                .map(|point| {
                    let radius = point.norm();
                    0.1 * (-12.0 * radius * radius).exp()
                })
                .collect::<Vec<_>>();
            let complementary = vec![Point2::default(); base.complementary_degrees_of_freedom()];
            let mut state =
                CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary)
                    .unwrap();
            // Land the Switch immediately, so the medium is already at its new
            // value while the wall still holds the old one.
            if alternate != 1.0 {
                let material = operator.initial_runtime().records()[0].material();
                state
                    .runtime_mut()
                    .begin_switch(material, true, 0.0, 0.0)
                    .unwrap();
            }

            let initial = state.energy(&operator).unwrap();
            // One encounter, not many. Over a long run reflected energy simply
            // leaves on its next pass, so cumulative escape says nothing about
            // the reflection coefficient; what is still inside just after the
            // wave has reached the wall does.
            // Equal propagation distance, not equal time: a heavier medium is
            // slower by `sqrt(f)`, and comparing at a fixed clock would score
            // a wave that has not reached the wall yet as reflected energy.
            let steps = (1.6 * alternate.sqrt() / time_step).round() as u64;
            for _ in 0..steps {
                let accounting = state.step_with_forcing(&operator, &forcing).unwrap();
                assert!(accounting.boundary_loss >= 0.0, "a wall cannot inject");
            }
            remaining.push(state.energy(&operator).unwrap() / initial);
        }

        // A matched wall lets most of one encounter through; what stays is the
        // first-order condition's own angular imperfection, which is large
        // enough that it dominates a small mismatch.
        assert!(
            remaining[0] < 0.15,
            "a matched wall should pass most of the wave, left {}",
            remaining[0]
        );
        // The mismatch is real and grows with it. This deliberately does not
        // assert the continuous normal-incidence coefficient: measured, a
        // mismatch of `f = 2.2` leaves about `0.003` more behind against a
        // predicted `R^2 = 0.038`, and only by `f = 6` does the excess
        // (`0.085`) approach the predicted `0.177`. A blob radiating into a
        // square box is not a normal-incidence experiment, and the matched
        // wall's own residual swamps the moderate case; calibrating that curve
        // needs packet tracking rather than this residual, and is recorded as
        // open rather than asserted here.
        for pair in remaining.windows(2) {
            assert!(
                pair[1] > pair[0],
                "a worse mismatch cannot keep less behind: {remaining:?}"
            );
        }
        assert!(
            remaining[2] > 1.5 * remaining[0],
            "a sixfold mismatch must be unmistakable, {remaining:?}"
        );
    }

    /// An absorbing wall must damp exactly as the fixed path's does when
    /// nothing is driven, or the same scene would radiate differently
    /// depending on which stepper ran it.
    #[test]
    fn an_inert_absorbing_wall_damps_exactly_as_the_fixed_path_does() {
        let scene = Scene::default();
        let mesh = mesh_scene(
            &scene,
            1,
            MeshingOptions {
                target_edge_length: 0.3,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::FirstOrderOutgoing,
        )
        .unwrap();
        let operator =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();
        let base = operator.base();
        assert!(
            base.first_order_boundary_damping()
                .iter()
                .any(|value| *value != 0.0),
            "the fixture needs an absorbing wall"
        );
        let forcing = CanonicalForcing::none(base);
        let time_step = 0.4 * operator.maximum_time_step();

        let primary = base
            .node_points()
            .iter()
            .map(|point| 0.06 * (1.4 * point.x - 0.8 * point.y).sin())
            .collect::<Vec<_>>();
        let potential = base
            .node_points()
            .iter()
            .map(|point| 0.03 * (0.9 * point.x + 1.2 * point.y).cos())
            .collect::<Vec<_>>();
        let complementary = base.compatible_flux(&potential).unwrap();
        let mut temporal = CanonicalTemporalWaveState::new(
            &operator,
            time_step,
            primary.clone(),
            complementary.clone(),
        )
        .unwrap();
        let mut fixed = CanonicalWaveState::new(base, time_step, primary, complementary).unwrap();

        let mut escaped = 0.0;
        for _ in 0..24 {
            let temporal_accounting = temporal.step_with_forcing(&operator, &forcing).unwrap();
            let fixed_accounting = fixed.step_with_forcing(base, &forcing).unwrap();
            assert!(
                (temporal_accounting.boundary_loss - fixed_accounting.boundary_loss).abs()
                    < 1.0e-12
            );
            escaped += temporal_accounting.boundary_loss;
        }
        assert!(escaped > 1.0e-6, "the wall must actually radiate");
        for (temporal, fixed) in temporal.primary_flux().iter().zip(fixed.primary_flux()) {
            assert!((temporal - fixed).abs() < 1.0e-12);
        }
        for (temporal, fixed) in temporal
            .complementary_flux()
            .iter()
            .zip(fixed.complementary_flux())
        {
            assert!((*temporal - *fixed).norm() < 1.0e-12);
        }
    }

    /// A second-order outgoing boundary in a driven medium.
    ///
    /// This is the wall whose factorization cannot be cached: the trace
    /// admittance, the modal couplings and the Schur complement all scale with
    /// the nodal mass, so a pumped medium rebuilds them at every stage. What
    /// has to survive that is the balance - the pole currents store energy,
    /// the wall carries some out, and the drive puts some in.
    #[test]
    fn a_driven_open_boundary_keeps_its_balance_second_order() {
        let mut scene = Scene::default();
        scene.materials[0].mass_law.drive = TimeDrive::ParametricPump {
            depth: ScalarField::constant(0.18),
            frequency_hz: ScalarField::constant(1.2),
            phase_radians: ScalarField::constant(0.25),
        };
        let mut base_scene = scene.clone();
        strip_temporal_laws(&mut base_scene.materials);
        let mesh = mesh_scene(
            &base_scene,
            1,
            MeshingOptions {
                target_edge_length: 0.3,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &base_scene,
            OuterBoundaryCondition::SecondOrderOutgoing,
        )
        .unwrap();
        let operator =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();
        let base = operator.base();
        let forcing = CanonicalForcing::none(base);

        let ceiling = 0.4 * operator.maximum_time_step();
        let target = 0.2;
        let mut previous: Option<(f64, f64)> = None;
        for refinement in [1.0, 0.5, 0.25] {
            let steps = (target / (ceiling * refinement)).ceil() as u64;
            let time_step = target / steps as f64;
            let primary = base
                .node_points()
                .iter()
                .map(|point| 0.05 * (1.5 * point.x - 0.9 * point.y).sin())
                .collect::<Vec<_>>();
            let potential = base
                .node_points()
                .iter()
                .map(|point| 0.03 * (1.0 * point.x + 1.3 * point.y).cos())
                .collect::<Vec<_>>();
            let complementary = base.compatible_flux(&potential).unwrap();
            let mut state =
                CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary)
                    .unwrap();

            let before = state.energy(&operator).unwrap();
            let mut work = 0.0;
            let mut escaped = 0.0;
            for _ in 0..steps {
                let accounting = state.step_with_forcing(&operator, &forcing).unwrap();
                assert!(accounting.boundary_loss >= 0.0, "a wall cannot inject");
                work += accounting.temporal_work;
                escaped += accounting.boundary_loss;
            }
            let after = state.energy(&operator).unwrap();
            assert!(escaped > 1.0e-6, "the wall must actually radiate");

            let residual = after - before - work + escaped;
            if let Some((coarse_step, coarse_residual)) = previous {
                let order = (coarse_residual.abs() / residual.abs()).log2()
                    / (coarse_step / time_step).log2();
                assert!(
                    order > 1.7,
                    "a nonlocal wall must not cost the balance its order, measured \
                     {order:.2} between dt {coarse_step:.3e} and {time_step:.3e}"
                );
            }
            previous = Some((time_step, residual));
        }
    }

    /// A second-order outgoing boundary, inert, must evolve exactly as the
    /// fixed path's does - including its pole currents, which are the only
    /// state on this path that is neither a field nor a local spring.
    #[test]
    fn an_inert_open_boundary_evolves_exactly_as_the_fixed_path_does() {
        let scene = Scene::default();
        let mesh = mesh_scene(
            &scene,
            1,
            MeshingOptions {
                target_edge_length: 0.3,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::SecondOrderOutgoing,
        )
        .unwrap();
        let operator =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();
        let base = operator.base();
        assert!(base.outgoing_boundary().is_some());
        assert!(operator.forced_composition_supported());
        assert!(!operator.conservative_bulk_supported());
        let forcing = CanonicalForcing::none(base);
        let time_step = 0.4 * operator.maximum_time_step();

        let primary = base
            .node_points()
            .iter()
            .map(|point| 0.05 * (1.5 * point.x - 0.9 * point.y).sin())
            .collect::<Vec<_>>();
        let potential = base
            .node_points()
            .iter()
            .map(|point| 0.03 * (1.0 * point.x + 1.3 * point.y).cos())
            .collect::<Vec<_>>();
        let complementary = base.compatible_flux(&potential).unwrap();
        let mut temporal = CanonicalTemporalWaveState::new(
            &operator,
            time_step,
            primary.clone(),
            complementary.clone(),
        )
        .unwrap();
        let mut fixed = CanonicalWaveState::new(base, time_step, primary, complementary).unwrap();

        let mut escaped = 0.0;
        for _ in 0..24 {
            let temporal_accounting = temporal.step_with_forcing(&operator, &forcing).unwrap();
            let fixed_accounting = fixed.step_with_forcing(base, &forcing).unwrap();
            assert!(
                (temporal_accounting.boundary_loss - fixed_accounting.boundary_loss).abs()
                    < 1.0e-12,
                "boundary loss diverged"
            );
            escaped += temporal_accounting.boundary_loss;
        }
        assert!(escaped > 1.0e-6, "the wall must actually radiate");
        for (temporal, fixed) in temporal.primary_flux().iter().zip(fixed.primary_flux()) {
            assert!((temporal - fixed).abs() < 1.0e-12);
        }
        for (temporal, fixed) in temporal
            .complementary_flux()
            .iter()
            .zip(fixed.complementary_flux())
        {
            assert!((*temporal - *fixed).norm() < 1.0e-12);
        }
    }

    /// The same gap, inert, must evolve exactly as the fixed path's does.
    #[test]
    fn an_inert_thin_gap_evolves_exactly_as_the_fixed_path_does() {
        let mut scene = Scene::default();
        scene.internal_boundaries.push(crate::InternalBoundary {
            id: crate::InternalBoundaryId(1),
            spline: crate::OpenCubicSpline::uniform(vec![
                Point2::new(-0.65, 0.0),
                Point2::new(-0.2, 0.0),
                Point2::new(0.2, 0.0),
                Point2::new(0.65, 0.0),
            ])
            .unwrap(),
            region: BACKGROUND_REGION,
            span_laws: vec![crate::InternalBoundaryLaw {
                coupling: crate::InternalBoundaryCoupling::ThinGap {
                    stiffness_ratio: 120.0,
                },
                ..crate::InternalBoundaryLaw::REFLECTING
            }],
        });
        let mesh = mesh_scene(
            &scene,
            17,
            MeshingOptions {
                target_edge_length: 0.18,
                minimum_angle_degrees: 14.0,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let operator =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();
        let base = operator.base();
        assert!(!base.thin_gap_samples().is_empty());
        let forcing = CanonicalForcing::none(base);
        let time_step = 0.4 * operator.maximum_time_step();

        let primary = base
            .node_points()
            .iter()
            .map(|point| 0.05 * (1.6 * point.x - 1.0 * point.y).sin())
            .collect::<Vec<_>>();
        let potential = base
            .node_points()
            .iter()
            .map(|point| 0.03 * (0.8 * point.x + 1.4 * point.y).cos())
            .collect::<Vec<_>>();
        let complementary = base.compatible_flux(&potential).unwrap();
        let mut temporal = CanonicalTemporalWaveState::new(
            &operator,
            time_step,
            primary.clone(),
            complementary.clone(),
        )
        .unwrap();
        let mut fixed = CanonicalWaveState::new(base, time_step, primary, complementary).unwrap();

        for _ in 0..24 {
            temporal.step_with_forcing(&operator, &forcing).unwrap();
            fixed.step_with_forcing(base, &forcing).unwrap();
        }
        assert!(
            temporal
                .thin_gap_jump()
                .iter()
                .any(|jump| jump.abs() > 1.0e-9),
            "the gap must actually open"
        );
        for (temporal, fixed) in temporal.primary_flux().iter().zip(fixed.primary_flux()) {
            assert!((temporal - fixed).abs() < 1.0e-12);
        }
        for (temporal, fixed) in temporal
            .complementary_flux()
            .iter()
            .zip(fixed.complementary_flux())
        {
            assert!((*temporal - *fixed).norm() < 1.0e-12);
        }
        assert!(
            (temporal.energy(&operator).unwrap() - fixed.energy(base).unwrap()).abs() < 1.0e-12
        );
    }

    /// A thin gap across a baffle, in a driven medium.
    ///
    /// The gap is a spring with a displacement of its own, so it stores energy
    /// the bulk fields cannot account for. Two things follow, and both are
    /// checked: that store belongs to the state's total energy, or the balance
    /// charges it to the splitting remainder; and its drift belongs to the
    /// same subflow as the complementary flux's, over the same interval and on
    /// the same midpoint field, which is what makes the local split exact.
    #[test]
    fn a_thin_gap_stores_energy_and_keeps_the_balance_second_order() {
        let mut scene = Scene::default();
        scene.internal_boundaries.push(crate::InternalBoundary {
            id: crate::InternalBoundaryId(1),
            spline: crate::OpenCubicSpline::uniform(vec![
                Point2::new(-0.65, 0.0),
                Point2::new(-0.2, 0.0),
                Point2::new(0.2, 0.0),
                Point2::new(0.65, 0.0),
            ])
            .unwrap(),
            region: BACKGROUND_REGION,
            span_laws: vec![crate::InternalBoundaryLaw {
                coupling: crate::InternalBoundaryCoupling::ThinGap {
                    stiffness_ratio: 120.0,
                },
                ..crate::InternalBoundaryLaw::REFLECTING
            }],
        });
        scene.materials[0].mass_law.drive = TimeDrive::ParametricPump {
            depth: ScalarField::constant(0.2),
            frequency_hz: ScalarField::constant(1.1),
            phase_radians: ScalarField::constant(0.3),
        };

        let mut base_scene = scene.clone();
        strip_temporal_laws(&mut base_scene.materials);
        let mesh = mesh_scene(
            &base_scene,
            17,
            MeshingOptions {
                target_edge_length: 0.18,
                minimum_angle_degrees: 14.0,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &base_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let operator =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();
        let base = operator.base();
        assert!(
            !base.thin_gap_samples().is_empty(),
            "the fixture needs a thin gap"
        );
        assert!(operator.forced_composition_supported());
        assert!(!operator.conservative_bulk_supported());
        let forcing = CanonicalForcing::none(base);

        let ceiling = 0.4 * operator.maximum_time_step();
        let target = 0.25;
        let mut previous: Option<(f64, f64)> = None;
        for refinement in [1.0, 0.5, 0.25] {
            let steps = (target / (ceiling * refinement)).ceil() as u64;
            let time_step = target / steps as f64;
            let primary = base
                .node_points()
                .iter()
                .map(|point| 0.05 * (1.6 * point.x - 1.0 * point.y).sin())
                .collect::<Vec<_>>();
            let potential = base
                .node_points()
                .iter()
                .map(|point| 0.03 * (0.8 * point.x + 1.4 * point.y).cos())
                .collect::<Vec<_>>();
            let complementary = base.compatible_flux(&potential).unwrap();
            let mut state =
                CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary)
                    .unwrap();

            let before = state.energy(&operator).unwrap();
            let mut work = 0.0;
            for _ in 0..steps {
                work += state
                    .step_with_forcing(&operator, &forcing)
                    .unwrap()
                    .temporal_work;
            }
            let after = state.energy(&operator).unwrap();
            assert!(
                state.thin_gap_jump().iter().any(|jump| jump.abs() > 1.0e-9),
                "the gap must actually open"
            );

            let residual = after - before - work;
            if let Some((coarse_step, coarse_residual)) = previous {
                let order = (coarse_residual.abs() / residual.abs()).log2()
                    / (coarse_step / time_step).log2();
                assert!(
                    order > 1.7,
                    "a gap must not cost the balance its order, measured {order:.2} between \
                     dt {coarse_step:.3e} and {time_step:.3e}"
                );
            }
            previous = Some((time_step, residual));
        }
    }

    /// What an application holding a law-carrying document actually hits.
    #[test]
    fn a_law_carrying_scene_needs_a_stripped_base_the_temporal_operator_can_reuse() {
        let mut scene = Scene::default();
        scene.materials[0].mass_law.drive = TimeDrive::ParametricPump {
            depth: ScalarField::constant(0.2),
            frequency_hz: ScalarField::constant(1.0),
            phase_radians: ScalarField::constant(0.0),
        };
        let mut stripped = scene.clone();
        strip_temporal_laws(&mut stripped.materials);
        let mesh = mesh_scene(
            &stripped,
            1,
            MeshingOptions {
                target_edge_length: 0.3,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &stripped,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        // The fixed compiler refuses it, by the same gate that stops a law
        // being executed as a static medium.
        assert!(crate::CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).is_err());
        // The temporal one strips for its base and keeps the laws for itself.
        let whole =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();

        // And an application that already built that stripped base reuses it
        // rather than compiling the assembly twice. The two routes must agree,
        // or the reuse would be a second definition of the operator.
        let base = std::sync::Arc::new(
            crate::CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &stripped, 1).unwrap(),
        );
        let reused = CanonicalTemporalWaveOperator::from_base(
            base,
            &mesh,
            &quadratic,
            crate::TopologyWaveModel::from_scene(&scene),
        )
        .unwrap();
        assert_eq!(
            whole.maximum_time_step(),
            reused.maximum_time_step(),
            "the reused base must give the same trajectory bound"
        );
        assert_eq!(
            whole.forced_composition_supported(),
            reused.forced_composition_supported()
        );
        assert_eq!(
            whole.conservative_bulk_supported(),
            reused.conservative_bulk_supported()
        );
        let runtime = whole.initial_runtime();
        assert_eq!(
            whole.primary_mass_at(0.3, &runtime).unwrap(),
            reused.primary_mass_at(0.3, &runtime).unwrap(),
            "the reused base must give the same instantaneous coefficients"
        );
    }

    /// Two things the variable-coefficient supplement has to get right. On an
    /// inert medium it must reproduce the fixed one exactly, or the temporal
    /// path would report different errors for the same physics. And on a
    /// driven one it must differ, because reusing the authored coefficients
    /// charges the estimator for the medium's own modulation and refines
    /// against it.
    /// The supplement's boundary defect terms, against the fixed path's own.
    ///
    /// An outgoing wall used to be refused outright here, on a contract written
    /// when the temporal path executed only the conservative bulk. It executes
    /// open boundaries now, so the estimate has to carry their defect - and a
    /// document with an outgoing wall is the default one, which is how this
    /// reached a user as "AMR fails at load". The earlier parity fixture is a
    /// reflecting box, so it could not have caught it.
    fn open_boundary_supplement_matches_the_fixed_one_when_inert() {
        let scene = Scene::initial();
        let mesh = mesh_scene(
            &scene,
            1,
            MeshingOptions {
                target_edge_length: 0.3,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::SecondOrderOutgoing,
        )
        .unwrap();
        let inert =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();
        let base = inert.base();
        assert!(
            base.outgoing_boundary().is_some(),
            "the fixture needs the wall it is about"
        );
        let time_step = 0.4 * inert.maximum_time_step();
        let primary = base
            .node_points()
            .iter()
            .map(|point| 0.05 * (1.7 * point.x - 0.9 * point.y).sin())
            .collect::<Vec<_>>();
        let potential = base
            .node_points()
            .iter()
            .map(|point| 0.03 * (1.1 * point.x + 1.4 * point.y).cos())
            .collect::<Vec<_>>();
        let state =
            CanonicalWaveState::from_primary_and_potential(base, time_step, &primary, &potential)
                .unwrap();
        let forcing = CanonicalForcing::none(base);
        let mut next = state.clone();
        next.step_with_forcing(base, &forcing).unwrap();
        let auxiliary_of = |state: &CanonicalWaveState| match state.auxiliaries() {
            crate::CanonicalAuxiliaryState::Linear(auxiliaries) => auxiliaries
                .thin_gap_jump()
                .iter()
                .chain(auxiliaries.outgoing_z())
                .copied()
                .collect::<Vec<_>>(),
            crate::CanonicalAuxiliaryState::None => Vec::new(),
        };
        let snapshot = CanonicalIndicatorSnapshot {
            mesh_revision: mesh.mesh_revision,
            primary_flux: next.primary_flux().to_vec(),
            previous_primary_flux: state.primary_flux().to_vec(),
            complementary_flux: next.complementary_flux().to_vec(),
            previous_complementary_flux: state.complementary_flux().to_vec(),
            auxiliary: auxiliary_of(&next),
            previous_auxiliary: auxiliary_of(&state),
            integrated_field: vec![],
            previous_integrated_field: vec![],
            time: time_step,
            time_step,
        };
        let fixed =
            crate::canonical_indicator_supplement(&mesh, base, &forcing, &snapshot).unwrap();
        let runtime = inert.initial_runtime();
        let temporal = canonical_temporal_indicator_supplement(
            &mesh, &inert, &forcing, &snapshot, &runtime, 0.0,
        )
        .expect("an open boundary must not refuse the estimate");
        assert!(
            fixed.outgoing_contribution > 0.0,
            "the fixture must exercise the term it is checking"
        );
        assert!(
            (temporal.outgoing_contribution - fixed.outgoing_contribution).abs()
                <= 1.0e-10 * fixed.outgoing_contribution.abs().max(1.0e-12),
            "inert outgoing defect differs: {} against {}",
            temporal.outgoing_contribution,
            fixed.outgoing_contribution
        );
        for (element, (left, right)) in temporal
            .element_boundary_residual
            .iter()
            .zip(&fixed.element_boundary_residual)
            .enumerate()
        {
            assert!(
                (left - right).abs() <= 1.0e-10 * right.abs().max(1.0e-12),
                "inert boundary residual differs at {element}: {left} against {right}"
            );
        }
    }

    #[test]
    fn temporal_supplement_matches_the_fixed_one_when_inert_and_departs_when_driven() {
        open_boundary_supplement_matches_the_fixed_one_when_inert();
        let inert_scene = Scene::initial();
        let mesh = mesh_scene(
            &inert_scene,
            1,
            MeshingOptions {
                target_edge_length: 0.3,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &inert_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();

        let inert =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &inert_scene, 1)
                .unwrap();
        let base = inert.base();
        let time_step = 0.4 * inert.maximum_time_step();
        let primary = base
            .node_points()
            .iter()
            .map(|point| 0.09 + 0.05 * (2.0 * point.x - 1.3 * point.y).sin())
            .collect::<Vec<_>>();
        let potential = base
            .node_points()
            .iter()
            .map(|point| 0.04 * (1.6 * point.x + 0.9 * point.y).cos())
            .collect::<Vec<_>>();
        let state =
            CanonicalWaveState::from_primary_and_potential(base, time_step, &primary, &potential)
                .unwrap();
        let mut next = state.clone();
        next.step(base).unwrap();
        let snapshot = CanonicalIndicatorSnapshot {
            mesh_revision: mesh.mesh_revision,
            primary_flux: next.primary_flux().to_vec(),
            previous_primary_flux: state.primary_flux().to_vec(),
            complementary_flux: next.complementary_flux().to_vec(),
            previous_complementary_flux: state.complementary_flux().to_vec(),
            auxiliary: vec![],
            previous_auxiliary: vec![],
            integrated_field: vec![],
            previous_integrated_field: vec![],
            time: time_step,
            time_step,
        };

        let forcing = crate::CanonicalForcing::none(base);
        let fixed =
            crate::canonical_indicator_supplement(&mesh, base, &forcing, &snapshot).unwrap();
        let runtime = inert.initial_runtime();
        let temporal = canonical_temporal_indicator_supplement(
            &mesh,
            &inert,
            &CanonicalForcing::none(inert.base()),
            &snapshot,
            &runtime,
            0.0,
        )
        .unwrap();
        for (element, (left, right)) in temporal
            .element_energy
            .iter()
            .zip(&fixed.element_energy)
            .enumerate()
        {
            assert!(
                (left - right).abs() <= 1.0e-12 * right.abs().max(1.0e-12),
                "inert energy differs at {element}: {left} against {right}"
            );
        }
        assert!(
            (temporal.drift_contribution - fixed.drift_contribution).abs()
                <= 1.0e-12 * fixed.drift_contribution.abs().max(1.0e-12),
            "inert drift differs: {} against {}",
            temporal.drift_contribution,
            fixed.drift_contribution
        );
        assert!(
            (temporal.complementary_recovery_contribution
                - fixed.complementary_recovery_contribution)
                .abs()
                <= 1.0e-12 * fixed.complementary_recovery_contribution.abs().max(1.0e-12)
        );

        // Now the same state under a driven medium. The fixed supplement is
        // blind to the modulation; the temporal one is not.
        let mut driven_scene = Scene::initial();
        driven_scene.materials[0].mass_law.drive = pump(0.35, 0.8, 0.4);
        driven_scene.materials[0].stiffness_law.drive = pump(0.3, 0.6, -0.2);
        let driven =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &driven_scene, 1)
                .unwrap();
        let runtime = driven.initial_runtime();
        let driven_supplement = canonical_temporal_indicator_supplement(
            &mesh,
            &driven,
            &CanonicalForcing::none(driven.base()),
            &snapshot,
            &runtime,
            0.0,
        )
        .unwrap();
        let energy: f64 = driven_supplement.element_energy.iter().sum();
        let inert_energy: f64 = fixed.element_energy.iter().sum();
        assert!(
            (energy - inert_energy).abs() > 1.0e-3 * inert_energy.abs(),
            "the drive must move the estimator's energy: {energy} against {inert_energy}"
        );
        assert!(
            driven_supplement
                .element_energy
                .iter()
                .all(|value| value.is_finite() && *value >= 0.0)
        );

        // And it reads the runtime it is given. A frequency edit committed
        // some time before the snapshot keeps the old carrier's phase at the
        // commit, so by the snapshot the carrier has drifted from the authored
        // anchor by the frequency change times the elapsed time - here about
        // 1.7 rad. A caller passing the authored runtime estimates a field
        // under a coefficient the solver never used.
        let mut carried = runtime.clone();
        let old_drive = pump(0.35, 1.7, 0.4)
            .evaluate(&driven_scene.materials[0].parameters)
            .unwrap();
        carried
            .preserve_carrier(
                driven_scene.materials[0].id,
                CanonicalMaterialDrive::MassCoefficient,
                old_drive,
                -0.3,
            )
            .unwrap();
        let carried_supplement = canonical_temporal_indicator_supplement(
            &mesh,
            &driven,
            &CanonicalForcing::none(driven.base()),
            &snapshot,
            &carried,
            0.0,
        )
        .unwrap();
        let carried_energy: f64 = carried_supplement.element_energy.iter().sum();
        assert!(
            (carried_energy - energy).abs() > 1.0e-3 * energy.abs(),
            "the carried phase must move the estimate: {carried_energy} against {energy}"
        );
    }

    /// End to end: a travelling modulation must actually shrink the mesh the
    /// indicator asks for, everywhere the pattern reaches, and it must do so
    /// on a field quiet enough that no error estimate would have asked.
    #[test]
    fn a_travelling_modulation_sizes_the_mesh_even_on_a_quiet_field() {
        let mut scene = Scene::initial();
        let wavenumber = 24.0;
        scene.materials[0].stiffness_law.drive = TimeDrive::TravellingModulation {
            depth: ScalarField::constant(0.25),
            frequency_hz: ScalarField::constant(0.4),
            phase_radians: ScalarField::constant(0.0),
            wavenumber: ScalarField::constant(wavenumber),
            angle_radians: ScalarField::constant(0.2),
        };

        let mut base_scene = scene.clone();
        strip_temporal_laws(&mut base_scene.materials);
        let mesh = std::sync::Arc::new(
            mesh_scene(
                &base_scene,
                1,
                MeshingOptions {
                    target_edge_length: 0.25,
                    ..MeshingOptions::default()
                },
            )
            .unwrap(),
        );
        let quadratic = std::sync::Arc::new(
            QuadraticWaveOperator::assemble_scene(
                &mesh,
                &base_scene,
                OuterBoundaryCondition::Reflecting,
            )
            .unwrap(),
        );
        let operator =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();

        let demand = operator.resolution_demand(0.0);
        let elements_per_wavelength = 5.0;
        let pattern_limit = demand.coefficient_wavelength / elements_per_wavelength;
        assert!(
            (demand.coefficient_wavelength - std::f64::consts::TAU / wavenumber).abs() < 1.0e-12
        );

        // A field small enough that the accuracy estimate has nothing to say.
        let count = quadratic.degrees_of_freedom();
        let snapshot = crate::QuadraticSolutionSnapshot {
            mesh_revision: mesh.mesh_revision,
            displacement: vec![1.0e-9; count],
            velocity: vec![0.0; count],
            acceleration: vec![0.0; count],
            volume_acceleration: vec![0.0; count],
            auxiliary: vec![0.0; count],
            time: 0.0,
            time_step: 0.4 * operator.maximum_time_step(),
        };
        let mut job = crate::SolutionIndicatorJob::new(
            mesh.clone(),
            quadratic.clone(),
            base_scene,
            snapshot,
            crate::SolutionIndicatorOptions {
                minimum_edge_length: 0.005,
                maximum_edge_length: 0.25,
                elements_per_wavelength,
                forcing_frequency_hz: demand.frequency_hz,
                coefficient_wavelength: demand.coefficient_wavelength,
                ..Default::default()
            },
        );
        let result = loop {
            if let Some(result) = job.advance(4_096) {
                break result.unwrap();
            }
        };

        assert!(
            result.report.smallest_wavelength_target <= pattern_limit * (1.0 + 1.0e-9),
            "the pattern must reach the size rule: {} against {pattern_limit}",
            result.report.smallest_wavelength_target
        );
        assert!(
            result.report.limit_refine_candidates > 0,
            "a mesh at 0.25 cannot carry a {:.4} pattern and must be asked to refine",
            demand.coefficient_wavelength
        );
        assert_eq!(
            result.report.error_refine_candidates, 0,
            "the field is quiet, so nothing here is an accuracy decision"
        );
        for target in &result.element_targets {
            assert!(
                *target <= pattern_limit * (1.0 + 1.0e-9),
                "an element was left at {target}, above the pattern limit {pattern_limit}"
            );
        }
    }

    /// A source frequency alone cannot size a mesh in a driven medium. The
    /// medium mixes, putting energy at `f_source +/- n f_drive`, and a
    /// travelling drive writes a spatial pattern into the coefficients that
    /// the mesh must resolve even where the field is quiet.
    #[test]
    fn resolution_demand_accounts_for_sidebands_and_the_coefficient_pattern() {
        // An inert medium asks for nothing beyond the source.
        let inert = compile(&Scene::initial()).unwrap();
        let quiet = inert.resolution_demand(2.0);
        assert_eq!(quiet.frequency_hz, 2.0);
        assert_eq!(quiet.sideband_order, 0);
        assert!(quiet.coefficient_wavelength.is_infinite());

        // A shallow pump reaches one sideband pair; a deep one reaches more.
        let pumped = |depth: f64| {
            let mut scene = Scene::initial();
            scene.materials[0].mass_law.drive = pump(depth, 0.5, 0.0);
            compile(&scene).unwrap().resolution_demand(2.0)
        };
        let shallow = pumped(0.02);
        let deep = pumped(0.9);
        assert_eq!(shallow.sideband_order, 1);
        assert!(
            deep.sideband_order > shallow.sideband_order,
            "a deeper drive carries further, got {} against {}",
            deep.sideband_order,
            shallow.sideband_order
        );
        assert!((shallow.frequency_hz - 2.5).abs() < 1.0e-12);
        assert!(deep.frequency_hz > shallow.frequency_hz);

        // A smoothed square carries its own odd harmonics before mixing, so
        // it reaches further than a cosine of the same depth and frequency.
        let mut crystal_scene = Scene::initial();
        crystal_scene.materials[0].mass_law.drive = TimeDrive::TimeCrystal {
            depth: ScalarField::constant(0.02),
            frequency_hz: ScalarField::constant(0.5),
            phase_radians: ScalarField::constant(0.0),
            sharpness: ScalarField::constant(3.0),
        };
        let crystal = compile(&crystal_scene).unwrap().resolution_demand(2.0);
        assert!(
            crystal.frequency_hz > shallow.frequency_hz,
            "a sharpened square reaches past a cosine: {} against {}",
            crystal.frequency_hz,
            shallow.frequency_hz
        );

        // A travelling drive patterns the operator in space. That binds the
        // mesh on its own, independently of any source.
        let mut travelling_scene = Scene::initial();
        travelling_scene.materials[0].stiffness_law.drive = TimeDrive::TravellingModulation {
            depth: ScalarField::constant(0.2),
            frequency_hz: ScalarField::constant(0.4),
            phase_radians: ScalarField::constant(0.0),
            wavenumber: ScalarField::constant(8.0),
            angle_radians: ScalarField::constant(0.3),
        };
        let travelling = compile(&travelling_scene).unwrap();
        let demand = travelling.resolution_demand(0.0);
        assert!(
            (demand.coefficient_wavelength - std::f64::consts::TAU / 8.0).abs() < 1.0e-12,
            "got {}",
            demand.coefficient_wavelength
        );
        assert!(
            demand.frequency_hz > 0.0,
            "a driven medium is not quiet just because no source is on"
        );
        // A non-travelling drive writes no spatial pattern.
        assert!(pumped(0.2).coefficient_wavelength.is_infinite());
    }

    /// The pump power must be the actual time derivative of the energy at
    /// fixed state, or a consumer reporting it would mislabel drift as
    /// physics. Checked against a central difference of the energy with the
    /// canonical state held still.
    #[test]
    fn temporal_energy_breakdown_reports_the_actual_pump_power() {
        let mut scene = Scene::initial();
        scene.materials[0].mass_law.drive = pump(0.27, 0.72, 0.19);
        scene.materials[0].stiffness_law.drive = TimeDrive::TimeCrystal {
            depth: ScalarField::constant(0.16),
            frequency_hz: ScalarField::constant(0.54),
            phase_radians: ScalarField::constant(-0.21),
            sharpness: ScalarField::constant(2.6),
        };
        let operator = compile(&scene).unwrap();
        let runtime = operator.initial_runtime();
        let primary = operator
            .base()
            .node_points()
            .iter()
            .map(|point| 0.11 + 0.19 * (0.9 * point.x - 1.3 * point.y).sin())
            .collect::<Vec<_>>();
        let potential = operator
            .base()
            .node_points()
            .iter()
            .map(|point| 0.06 * (1.4 * point.x + 0.5 * point.y).cos())
            .collect::<Vec<_>>();
        let complementary = operator.base().compatible_flux(&potential).unwrap();

        let time = 0.41;
        let breakdown = canonical_temporal_energy_breakdown(
            &operator,
            &primary,
            &complementary,
            time,
            &runtime,
        )
        .unwrap();
        let total = operator
            .energy_at(&primary, &complementary, time, &runtime)
            .unwrap();
        assert!((breakdown.total() - total).abs() < 1.0e-12);
        assert!(breakdown.primary > 0.0 && breakdown.complementary > 0.0);

        let step = 1.0e-5;
        let ahead = operator
            .energy_at(&primary, &complementary, time + step, &runtime)
            .unwrap();
        let behind = operator
            .energy_at(&primary, &complementary, time - step, &runtime)
            .unwrap();
        let difference = (ahead - behind) / (2.0 * step);
        assert!(
            (breakdown.temporal_power - difference).abs() < 1.0e-6 * difference.abs().max(1.0),
            "analytic pump power {} against difference {difference}",
            breakdown.temporal_power
        );
        assert!(
            breakdown.temporal_power.abs() > 1.0e-6,
            "the fixture must pump"
        );
    }

    /// The breakdown covers only the free bulk, so a generation carrying loss
    /// or an open boundary is refused rather than reported with its other
    /// exchanges silently missing.
    #[test]
    fn temporal_energy_breakdown_refuses_a_composed_system() {
        let mut scene = Scene::initial();
        scene.materials[0].mass_law.drive = pump(0.2, 0.6, 0.0);
        scene.materials[0].damping = ScalarField::constant(0.35);
        let operator = compile(&scene).unwrap();
        let runtime = operator.initial_runtime();
        let primary = vec![0.1; operator.base().degrees_of_freedom()];
        let complementary =
            vec![Point2::default(); operator.base().complementary_degrees_of_freedom()];
        assert!(!operator.conservative_bulk_supported());
        assert!(
            canonical_temporal_energy_breakdown(
                &operator,
                &primary,
                &complementary,
                0.3,
                &runtime,
            )
            .is_err()
        );
    }

    /// A probe over every face must still report the solver's own energy when
    /// the material is driven, which means evaluating the assembled nodal map
    /// at the sampled instant rather than reusing the authored one.
    #[test]
    fn temporal_area_probe_over_every_face_reports_the_instantaneous_energy() {
        let mut scene = Scene::initial();
        scene.materials[0].mass_law.drive = TimeDrive::TravellingModulation {
            depth: ScalarField::constant(0.31),
            frequency_hz: ScalarField::constant(0.65),
            phase_radians: ScalarField::constant(0.22),
            wavenumber: ScalarField::constant(3.4),
            angle_radians: ScalarField::constant(0.4),
        };
        scene.materials[0].stiffness_law.drive = pump(0.19, 0.5, -0.3);

        let mut base_scene = scene.clone();
        strip_temporal_laws(&mut base_scene.materials);
        let mesh = mesh_scene(
            &base_scene,
            1,
            MeshingOptions {
                target_edge_length: 0.3,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &base_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let operator =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();
        let stencil = QuadraticAreaStencil::build(
            &mesh,
            &quadratic,
            &base_scene,
            crate::AreaProbeShape::Region(BACKGROUND_REGION),
        )
        .unwrap();

        let runtime = operator.initial_runtime();
        let time = 0.53;
        let primary = operator
            .base()
            .node_points()
            .iter()
            .map(|point| 0.14 + 0.21 * (1.1 * point.x - 0.6 * point.y).sin())
            .collect::<Vec<_>>();
        let potential = operator
            .base()
            .node_points()
            .iter()
            .map(|point| 0.07 * (0.8 * point.x + 1.2 * point.y).cos())
            .collect::<Vec<_>>();
        let complementary = operator.base().compatible_flux(&potential).unwrap();

        let sample = sample_temporal_canonical_area(
            &stencil,
            &operator,
            &primary,
            &complementary,
            time,
            &runtime,
        )
        .unwrap();
        let expected = operator
            .energy_at(&primary, &complementary, time, &runtime)
            .unwrap();
        assert!((sample.coverage - 1.0).abs() < 1.0e-9);
        assert!(
            (sample.total_energy - expected).abs() < 1.0e-9 * expected,
            "area probe reported {} against the solver's {expected}",
            sample.total_energy
        );
        // The driven answer must actually differ from the inert one, or this
        // fixture would pass without exercising any of the new evaluation.
        let inert =
            sample_canonical_area(&stencil, operator.base(), &primary, &complementary).unwrap();
        assert!(
            relative_gap(sample.total_energy, inert.total_energy) > 1.0e-3,
            "the drive must move the energy: {} against {}",
            sample.total_energy,
            inert.total_energy
        );
        assert!(relative_gap(sample.rms_complementary, inert.rms_complementary) > 1.0e-3);
    }

    /// On a field-dependent medium the area readout reads fields through the
    /// maps the solver inverts, and splits each node's stored energy over the
    /// materials meeting there, so a probe over every face reports exactly
    /// the solver's energy, junction nodes included.
    #[test]
    fn a_nonlinear_area_probe_over_every_face_reports_the_solver_energy() {
        let mut scene = Scene::initial();
        scene.materials[0].mass_law.field = kerr(0.8);
        scene.materials[0].stiffness_law.field = saturable_law(6.0, 0.3);
        let mut base_scene = scene.clone();
        strip_temporal_laws(&mut base_scene.materials);
        let mesh = mesh_scene(
            &base_scene,
            1,
            MeshingOptions {
                target_edge_length: 0.3,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &base_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let operator =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();
        let runtime = operator.initial_runtime();
        let (primary, complementary) = strong_fluxes(&operator, 6.0);
        let expected = operator
            .energy_at(&primary, &complementary, 0.0, &runtime)
            .unwrap();
        let mut total = 0.0;
        for region in &base_scene.regions {
            let stencil = QuadraticAreaStencil::build(
                &mesh,
                &quadratic,
                &base_scene,
                crate::AreaProbeShape::Region(region.id),
            )
            .unwrap();
            total += sample_temporal_canonical_area(
                &stencil,
                &operator,
                &primary,
                &complementary,
                0.0,
                &runtime,
            )
            .unwrap()
            .total_energy;
        }
        assert!(
            (total - expected).abs() < 1.0e-9 * expected,
            "the faces summed to {total} against the solver's {expected}"
        );
        let linear = sample_canonical_area_total(
            &mesh,
            &quadratic,
            &base_scene,
            &operator,
            &primary,
            &complementary,
        );
        assert!(
            relative_gap(total, linear) > 1.0e-2,
            "a strong field must read differently from the linear map: {total} against {linear}"
        );
    }

    fn sample_canonical_area_total(
        mesh: &crate::TriMesh,
        quadratic: &QuadraticWaveOperator,
        scene: &Scene,
        operator: &CanonicalTemporalWaveOperator,
        primary: &[f64],
        complementary: &[Point2],
    ) -> f64 {
        scene
            .regions
            .iter()
            .map(|region| {
                let stencil = QuadraticAreaStencil::build(
                    mesh,
                    quadratic,
                    scene,
                    crate::AreaProbeShape::Region(region.id),
                )
                .unwrap();
                sample_canonical_area(&stencil, operator.base(), primary, complementary)
                    .unwrap()
                    .total_energy
            })
            .sum()
    }

    /// The size rule on a self-focusing medium divides its wavelength floor by
    /// `√(ḡ + Aḡ′)` at the field's envelope `A`, `1 + 3χA²` for Kerr, and reads
    /// the same at every phase of a cycle, so it does not retarget elements
    /// twice a period.
    #[test]
    fn the_size_rule_resolves_the_wavelength_a_strong_kerr_field_makes() {
        let target = |chi: f64, phase: f64| {
            let mut scene = Scene::initial();
            scene.materials[0].mass_law.field = kerr(chi);
            let mut base_scene = scene.clone();
            strip_temporal_laws(&mut base_scene.materials);
            let mesh = std::sync::Arc::new(
                mesh_scene(
                    &base_scene,
                    1,
                    MeshingOptions {
                        target_edge_length: 0.25,
                        ..MeshingOptions::default()
                    },
                )
                .unwrap(),
            );
            let quadratic = std::sync::Arc::new(
                QuadraticWaveOperator::assemble_scene(
                    &mesh,
                    &base_scene,
                    OuterBoundaryCondition::Reflecting,
                )
                .unwrap(),
            );
            let operator =
                CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();
            let frequency = 2.0;
            let omega = std::f64::consts::TAU * frequency;
            let count = quadratic.degrees_of_freedom();
            // A uniform harmonic field of envelope one.
            let snapshot = crate::QuadraticSolutionSnapshot {
                mesh_revision: mesh.mesh_revision,
                displacement: vec![phase.cos(); count],
                velocity: vec![-omega * phase.sin(); count],
                acceleration: vec![0.0; count],
                volume_acceleration: vec![0.0; count],
                auxiliary: vec![0.0; count],
                time: 0.0,
                time_step: 0.4 * operator.maximum_time_step(),
            };
            let mut job = crate::SolutionIndicatorJob::new(
                mesh.clone(),
                quadratic.clone(),
                scene,
                snapshot,
                crate::SolutionIndicatorOptions {
                    minimum_edge_length: 1.0e-4,
                    maximum_edge_length: 10.0,
                    resolved_frequency_hz: frequency,
                    ..Default::default()
                },
            )
            .with_instantaneous_materials(operator.initial_runtime());
            let report = loop {
                if let Some(result) = job.advance(4_096) {
                    break result.unwrap().report;
                }
            };
            (
                report.smallest_wavelength_target,
                report.largest_field_tangent,
            )
        };
        let (linear, unit) = target(0.0, 0.3);
        assert_eq!(unit, 1.0);
        let (kerr_target, tangent) = target(0.8, 0.3);
        assert!(
            (tangent - (1.0 + 3.0 * 0.8)).abs() < 1.0e-12,
            "tangent {tangent}"
        );
        assert!(
            (kerr_target * tangent.sqrt() - linear).abs() < 1.0e-9 * linear,
            "{kerr_target} against {linear}"
        );
        let (other_phase, _) = target(0.8, 1.9);
        assert!((other_phase - kerr_target).abs() < 1.0e-9 * kerr_target);
    }

    fn relative_gap(left: f64, right: f64) -> f64 {
        (left - right).abs() / left.abs().max(right.abs()).max(1.0e-12)
    }

    #[test]
    fn travelling_complementary_drive_is_resolved_at_each_sample() {
        let mut scene = Scene::initial();
        scene.materials[0].stiffness_law.drive = TimeDrive::TravellingModulation {
            depth: ScalarField::constant(0.42),
            frequency_hz: ScalarField::constant(0.7),
            phase_radians: ScalarField::constant(0.15),
            wavenumber: ScalarField::constant(9.0),
            angle_radians: ScalarField::constant(0.3),
        };
        let operator = compile(&scene).unwrap();
        let barycentric = [0.21, 0.37, 0.42];
        let nodes = operator.base().element_nodes()[0];
        let quadratic = QuadraticPointStencil {
            element: 0,
            barycentric,
            nodes,
            value_weights: enriched_quadratic_basis(barycentric),
            gradient_weights: [Point2::default(); 7],
            region: BACKGROUND_REGION,
            mass_density: 1.0,
            stiffness: SymmetricTensor2::isotropic(1.0),
        };
        let stencil = CanonicalTemporalPointStencil::from_quadratic(quadratic, &operator).unwrap();
        let runtime = operator.initial_runtime();
        let time = 0.44;
        let flux = Point2::new(0.27, -0.19);
        let complementary = vec![flux; operator.base().complementary_degrees_of_freedom()];

        let factors = (0..6)
            .map(|local| {
                stencil.sample_coefficients()[local]
                    .factor_at(time, &runtime)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let spread = factors.iter().copied().fold(f64::MIN, f64::max)
            - factors.iter().copied().fold(f64::MAX, f64::min);
        assert!(
            spread > 0.05,
            "the fixture must actually vary across the element, got {spread}"
        );

        let fixed = stencil.fixed();
        let mut expected = Point2::default();
        for (local, factor) in factors.iter().enumerate() {
            expected = expected
                + fixed.sample_inverses[local].apply(flux) / *factor
                    * fixed.complementary_weights[local];
        }
        let actual = stencil
            .complementary_field(&complementary, time, &runtime)
            .unwrap();
        assert!((actual - expected).norm() < 1.0e-12);

        // The order that a nonlinear law could not serve, kept here only to
        // show the two answers are genuinely different under modulation.
        let probe_factor = stencil
            .complementary_coefficient()
            .factor_at(time, &runtime)
            .unwrap();
        let flux_first = fixed.sample_inverses[0].apply(flux) / probe_factor;
        assert!((actual - flux_first).norm() > 1.0e-4);
    }

    #[test]
    fn travelling_primary_drive_is_sampled_in_the_region_material_frame() {
        let mut scene = Scene::initial();
        scene.regions[0].frame = MaterialFrame {
            origin: Point2::new(0.2, -0.1),
            angle_radians: 0.37,
            ..MaterialFrame::world()
        };
        scene.materials[0].mass_law.drive = TimeDrive::TravellingModulation {
            depth: ScalarField::constant(0.25),
            frequency_hz: ScalarField::constant(0.0),
            phase_radians: ScalarField::constant(0.2),
            wavenumber: ScalarField::constant(4.0),
            angle_radians: ScalarField::constant(-0.3),
        };
        let temporal = compile(&scene).unwrap();
        let runtime = temporal.initial_runtime();
        let mass = temporal.primary_mass_at(0.0, &runtime).unwrap();
        let mut expected = vec![0.0; temporal.base().degrees_of_freedom()];
        for (contribution, sample) in temporal
            .base()
            .primary_contributions()
            .iter()
            .zip(&temporal.primary)
        {
            let factor = coefficient_factor(sample.coefficient, 0.0, &runtime).unwrap();
            expected[contribution.node as usize] +=
                contribution.geometric_weight * contribution.reference_coefficient * factor;
        }
        assert_eq!(mass, expected);
        assert!(
            mass.iter()
                .zip(temporal.base().primary_mass())
                .any(|(actual, base)| (actual / base - 1.0).abs() > 0.05)
        );
    }

    #[test]
    fn shared_nodes_gather_independently_driven_material_contributions() {
        let mut scene = Scene::default();
        let mut second = scene.materials[0].clone();
        second.id = MaterialId(2);
        second.name = "Second".into();
        scene.materials[0].mass_law.drive = pump(0.2, 0.0, 0.0);
        second.mass_law.drive = pump(0.2, 0.0, std::f64::consts::PI);
        scene.materials.push(second);
        scene.regions.push(Region {
            id: RegionId(2),
            material: MaterialId(2),
            frame: MaterialFrame::world(),
        });
        scene.obstacles.push(Obstacle::with_role(
            ObstacleId(1),
            PeriodicCubicSpline::rounded(Point2::new(0.5, 0.5), 0.2),
            LoopRole::MaterialInterface {
                exterior: BACKGROUND_REGION,
                interior: RegionId(2),
            },
        ));
        let mut base_scene = scene.clone();
        strip_temporal_laws(&mut base_scene.materials);
        let mesh = mesh_scene(
            &base_scene,
            1,
            MeshingOptions {
                target_edge_length: 0.15,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &base_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let temporal =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();
        let runtime = temporal.initial_runtime();
        let mass = temporal.primary_mass_at(0.0, &runtime).unwrap();
        let mut owners = vec![BTreeSet::new(); temporal.base().degrees_of_freedom()];
        for (contribution, sample) in temporal
            .base()
            .primary_contributions()
            .iter()
            .zip(&temporal.primary)
        {
            owners[contribution.node as usize].insert(sample.coefficient.material);
        }
        let mut shared = 0;
        let mut unshared_changed = 0;
        for ((mass, base), owners) in mass.iter().zip(temporal.base().primary_mass()).zip(owners) {
            if owners.len() == 2 {
                shared += 1;
                let ratio = mass / base;
                assert!(ratio > 0.8 + 1.0e-12 && ratio < 1.2 - 1.0e-12);
            } else if (mass - base).abs() > 1.0e-12 {
                unshared_changed += 1;
            }
        }
        assert!(shared >= 3);
        assert!(unshared_changed > 0);
    }

    #[test]
    fn te_places_mass_row_drive_on_the_complementary_field() {
        let mut scene = Scene::initial();
        scene.physics = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        };
        scene.materials[0].mass_law.drive = pump(0.25, 0.0, 0.0);
        let temporal = compile(&scene).unwrap();
        let runtime = temporal.initial_runtime();
        assert_eq!(
            temporal.primary_mass_at(0.0, &runtime).unwrap(),
            temporal.base().primary_mass()
        );
        let flux = vec![Point2::new(0.3, -0.2); temporal.base().complementary_degrees_of_freedom()];
        let base = temporal.base().complementary_field(&flux).unwrap();
        let driven = temporal
            .complementary_field_at(&flux, 0.0, &runtime)
            .unwrap();
        for (base, driven) in base.iter().zip(driven) {
            assert!((driven.x - base.x / 1.25).abs() < 1.0e-12);
            assert!((driven.y - base.y / 1.25).abs() < 1.0e-12);
        }
    }

    #[test]
    fn switch_and_reciprocal_drive_modify_the_complete_primary_map() {
        let mut scene = Scene::initial();
        scene.materials[0].mass_law.alternate = Some(ScalarField::constant(4.0));
        scene.materials[0].mass_law.inverted = true;
        scene.materials[0].switch_ramp = 2.0;
        let temporal = compile(&scene).unwrap();
        let mut runtime = temporal.initial_runtime();
        runtime
            .begin_switch(scene.materials[0].id, true, 0.0, 2.0)
            .unwrap();
        let mass = temporal.primary_mass_at(1.0, &runtime).unwrap();
        for (mass, base) in mass.iter().zip(temporal.base().primary_mass()) {
            assert!((mass / base - 0.4).abs() < 1.0e-12);
        }
    }

    #[test]
    fn constant_loss_drive_follows_the_physical_channel() {
        let mut scene = Scene::initial();
        scene.physics = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        };
        scene.materials[0].electric_loss = Some(LossChannel {
            base_rate: ScalarField::constant(0.4),
            law: DampingLaw {
                rate: RateLaw::Constant,
                drive: pump(0.25, 1.0, 0.0),
            },
        });
        let temporal = compile(&scene).unwrap();
        let runtime = temporal.initial_runtime();
        let high = temporal.loss_rates_at(0.0, &runtime).unwrap();
        let low = temporal.loss_rates_at(0.5, &runtime).unwrap();
        assert!(
            high.primary
                .iter()
                .all(|rate| (*rate - 0.5).abs() < 1.0e-12)
        );
        assert!(low.primary.iter().all(|rate| (*rate - 0.3).abs() < 1.0e-12));
        assert!(high.complementary.iter().all(|rate| *rate == 0.0));
    }

    #[test]
    fn ordinary_production_compiler_still_rejects_a_driven_material() {
        let mut scene = Scene::initial();
        scene.materials[0].mass_law.drive = pump(0.2, 1.0, 0.0);
        let mesh = mesh_scene(&scene, 1, MeshingOptions::default()).unwrap();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        // The refusal names the material and the row, because it shares its
        // error with a genuinely invalid coefficient and the two are
        // indistinguishable to anyone reading the application's message.
        let refusal = CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1);
        let Err(WaveError::MaterialEvaluation {
            material,
            coefficient,
            ..
        }) = &refusal
        else {
            panic!("expected a named refusal, got {refusal:?}");
        };
        assert_eq!(material, &scene.materials[0].name);
        assert_eq!(*coefficient, "mass law");
        assert_eq!(scene.regions[0].id, BACKGROUND_REGION);
    }

    fn reference_fluxes(operator: &CanonicalTemporalWaveOperator) -> (Vec<f64>, Vec<Point2>) {
        let primary = operator
            .base()
            .primary_mass()
            .iter()
            .enumerate()
            .map(|(index, mass)| mass * (0.07 * (index as f64 * 0.37).sin() + 0.03))
            .collect();
        let complementary = operator
            .base()
            .constitutive_samples()
            .iter()
            .enumerate()
            .map(|(index, _)| {
                Point2::new(
                    0.02 * (index as f64 * 0.19).cos(),
                    0.015 * (index as f64 * 0.23).sin(),
                )
            })
            .collect();
        (primary, complementary)
    }

    #[test]
    fn inert_bulk_split_matches_the_existing_kdk_step() {
        let operator = compile(&Scene::initial()).unwrap();
        assert!(operator.conservative_bulk_supported());
        let time_step = 0.2 * operator.maximum_time_step();
        let (primary, complementary) = reference_fluxes(&operator);
        let mut fixed = CanonicalWaveState::new(
            operator.base(),
            time_step,
            primary.clone(),
            complementary.clone(),
        )
        .unwrap();
        let mut temporal =
            CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary).unwrap();
        fixed.step(operator.base()).unwrap();
        temporal.step(&operator).unwrap();
        assert_eq!(temporal.primary_flux(), fixed.primary_flux());
        assert_eq!(temporal.complementary_flux(), fixed.complementary_flux());
    }

    #[test]
    fn driven_bulk_split_is_reversible_to_roundoff() {
        let mut scene = Scene::initial();
        scene.materials[0].mass_law.drive = pump(0.24, 0.8, 0.31);
        scene.materials[0].stiffness_law.drive = pump(0.17, 0.6, -0.23);
        let operator = compile(&scene).unwrap();
        let time_step = 0.35 * operator.maximum_time_step();
        let (primary, complementary) = reference_fluxes(&operator);
        let mut state = CanonicalTemporalWaveState::new(
            &operator,
            time_step,
            primary.clone(),
            complementary.clone(),
        )
        .unwrap();
        state.step_by(&operator, time_step).unwrap();
        state.step_by(&operator, -time_step).unwrap();
        assert!(state.time().abs() < 1.0e-15);
        for (actual, expected) in state.primary_flux().iter().zip(primary) {
            assert!((actual - expected).abs() < 2.0e-14);
        }
        for (actual, expected) in state.complementary_flux().iter().zip(complementary) {
            assert!((actual.x - expected.x).abs() < 2.0e-14);
            assert!((actual.y - expected.y).abs() < 2.0e-14);
        }
    }

    #[test]
    fn bulk_energy_rate_is_the_fixed_state_time_derivative() {
        let mut scene = Scene::initial();
        scene.materials[0].mass_law.drive = pump(0.22, 0.9, 0.41);
        scene.materials[0].stiffness_law.drive = pump(0.13, 0.7, -0.19);
        let operator = compile(&scene).unwrap();
        let runtime = operator.initial_runtime();
        let (primary, complementary) = reference_fluxes(&operator);
        let time = 0.37;
        let epsilon = 1.0e-6;
        let (_, rate) = operator
            .energy_and_rate_at(&primary, &complementary, time, &runtime)
            .unwrap();
        let before = operator
            .energy_at(&primary, &complementary, time - epsilon, &runtime)
            .unwrap();
        let after = operator
            .energy_at(&primary, &complementary, time + epsilon, &runtime)
            .unwrap();
        let numerical = (after - before) / (2.0 * epsilon);
        assert!((rate - numerical).abs() < 2.0e-9 * rate.abs().max(1.0));
    }

    #[test]
    fn temporal_work_residual_converges_at_second_order() {
        let mut scene = Scene::initial();
        scene.materials[0].mass_law.drive = pump(0.31, 1.1, 0.27);
        scene.materials[0].stiffness_law.drive = pump(0.21, 0.9, -0.34);
        let operator = compile(&scene).unwrap();
        let coarse_step = 0.6 * operator.maximum_time_step();
        let (primary, complementary) = reference_fluxes(&operator);
        let accumulated_residual = |time_step: f64, steps: usize| {
            let mut state = CanonicalTemporalWaveState::new(
                &operator,
                time_step,
                primary.clone(),
                complementary.clone(),
            )
            .unwrap();
            (0..steps)
                .map(|_| state.step(&operator).unwrap().splitting_residual)
                .sum::<f64>()
        };
        let coarse = accumulated_residual(coarse_step, 16).abs();
        let fine = accumulated_residual(0.5 * coarse_step, 32).abs();
        assert!(coarse > 1.0e-12);
        assert!(fine < 0.35 * coarse, "coarse={coarse:e}, fine={fine:e}");
    }

    #[test]
    fn trajectory_cfl_uses_the_worst_bulk_coefficient_factors() {
        let mut scene = Scene::initial();
        scene.materials[0].mass_law.drive = pump(0.36, 0.8, 0.0);
        scene.materials[0].stiffness_law.drive = pump(0.19, 0.7, 0.0);
        let operator = compile(&scene).unwrap();
        let expected = operator.base().maximum_time_step() * (0.64_f64 * 0.81).sqrt();
        assert!((operator.maximum_time_step() - expected).abs() < 1.0e-14);
    }

    #[test]
    fn the_bulk_claim_narrows_as_each_capability_composes() {
        // Loss used to have no state here at all. It now steps as an accounted
        // dissipation lane, so what it loses is the conservative-bulk claim
        // rather than the ability to run.
        let mut lossy_scene = Scene::initial();
        lossy_scene.materials[0].damping = ScalarField::constant(0.1);
        let lossy = compile(&lossy_scene).unwrap();
        assert!(lossy.has_loss());
        assert!(!lossy.conservative_bulk_supported());
        assert!(lossy.forced_composition_supported());
        assert!(CanonicalTemporalWaveState::zero(&lossy, 0.1 * lossy.maximum_time_step()).is_ok());

        let scene = Scene::initial();
        let mesh = mesh_scene(
            &scene,
            1,
            MeshingOptions {
                target_edge_length: 0.3,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        // A first-order wall is a local damping term in the kick and composes.
        let first_order = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::FirstOrderOutgoing,
        )
        .unwrap();
        let damped =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &first_order, &scene, 1).unwrap();
        assert!(!damped.conservative_bulk_supported());
        assert!(damped.forced_composition_supported());
        assert!(
            CanonicalTemporalWaveState::zero(&damped, 0.1 * damped.maximum_time_step()).is_ok()
        );

        // A second-order wall carries pole currents of its own, and now has
        // state for them.
        let second_order = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::SecondOrderOutgoing,
        )
        .unwrap();
        let open =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &second_order, &scene, 1).unwrap();
        assert!(!open.conservative_bulk_supported());
        assert!(open.forced_composition_supported());
        assert!(CanonicalTemporalWaveState::zero(&open, 0.1 * open.maximum_time_step()).is_ok());
    }

    #[test]
    fn temporal_tensor_scaling_keeps_the_base_anisotropy_shape() {
        let tensor = SymmetricTensor2 {
            xx: 2.0,
            xy: 0.4,
            yy: 1.0,
        };
        let factor = 1.7;
        let scaled = SymmetricTensor2 {
            xx: tensor.xx / factor,
            xy: tensor.xy / factor,
            yy: tensor.yy / factor,
        };
        assert!((scaled.xx / scaled.yy - tensor.xx / tensor.yy).abs() < 1.0e-12);
        assert!((scaled.xy / scaled.yy - tensor.xy / tensor.yy).abs() < 1.0e-12);
    }

    // -----------------------------------------------------------------------
    // Stage 8: field-dependent response
    // -----------------------------------------------------------------------

    fn kerr(chi2: f64) -> FieldLaw {
        FieldLaw::Polynomial {
            chi1: ScalarField::constant(0.0),
            chi2: ScalarField::constant(chi2),
            amplitude_bound: None,
        }
    }

    fn bounded_kerr(chi2: f64, bound: f64) -> FieldLaw {
        FieldLaw::Polynomial {
            chi1: ScalarField::constant(0.0),
            chi2: ScalarField::constant(chi2),
            amplitude_bound: Some(ScalarField::constant(bound)),
        }
    }

    fn saturable_law(chi: f64, saturation: f64) -> FieldLaw {
        FieldLaw::Saturable {
            chi: ScalarField::constant(chi),
            saturation: ScalarField::constant(saturation),
        }
    }

    /// Reference fluxes scaled to an amplitude where the laws below move the
    /// maps by tens of percent.
    fn strong_fluxes(
        operator: &CanonicalTemporalWaveOperator,
        amplitude: f64,
    ) -> (Vec<f64>, Vec<Point2>) {
        let (primary, complementary) = reference_fluxes(operator);
        (
            primary.into_iter().map(|flux| amplitude * flux).collect(),
            complementary
                .into_iter()
                .map(|flux| flux * amplitude)
                .collect(),
        )
    }

    fn kerr_scene() -> Scene {
        let mut scene = Scene::initial();
        scene.materials[0].mass_law.field = kerr(0.8);
        scene.materials[0].stiffness_law.field = kerr(20.0);
        scene
    }

    fn field_law_refusal(scene: &Scene) -> Option<String> {
        match compile(scene) {
            Err(WaveError::MaterialEvaluation {
                coefficient: "field law",
                reason,
                ..
            }) => Some(reason),
            _ => None,
        }
    }

    #[test]
    fn field_responses_compile_where_executed_and_refuse_their_gates() {
        let operator = compile(&kerr_scene()).unwrap();
        assert!(operator.has_field_laws());
        assert!(!operator.has_temporal_laws());
        // Self-focusing Kerr never tightens the step: its weakest tangent is
        // the linear one, at rest.
        assert_eq!(
            operator.maximum_time_step(),
            operator.base().maximum_time_step()
        );
        let mut scene = Scene::initial();
        scene.materials[0].stiffness_law.field = saturable_law(-0.2, 2.0);
        let operator = compile(&scene).unwrap();
        let expected = operator.base().maximum_time_step() * (1.0_f64 + 9.0 * -0.8 / 8.0).sqrt();
        assert!((operator.maximum_time_step() - expected).abs() < 1e-14 * expected);

        let mut scene = Scene::initial();
        scene.materials[0].mass_law.field = FieldLaw::Polynomial {
            chi1: ScalarField::constant(0.3),
            chi2: ScalarField::constant(0.8),
            amplitude_bound: None,
        };
        assert!(field_law_refusal(&scene).unwrap().contains("Gate C"));
        scene.materials[0].mass_law.field = bounded_kerr(0.8, 0.5);
        scene.materials[0].mass_law.inverted = true;
        assert!(field_law_refusal(&scene).unwrap().contains("Gate C"));
        scene.materials[0].mass_law.inverted = false;
        scene.materials[0].mass_law.field = kerr(-0.2);
        // Rejected at authoring validation already: an unbounded defocusing
        // Kerr law has no positive tangent.
        assert!(compile(&scene).is_err());

        // Nonlinear anisotropy is Gate C's; a nonlinear mass row on an
        // anisotropic medium is fine, because the primary map is a scalar.
        let mut scene = Scene::initial();
        scene.materials[0].axis_ratio = ScalarField::constant(1.6);
        scene.materials[0].mass_law.field = kerr(0.8);
        assert!(compile(&scene).unwrap().has_field_laws());
        scene.materials[0].stiffness_law.field = kerr(0.8);
        assert!(
            field_law_refusal(&scene)
                .unwrap()
                .contains("anisotropic medium")
        );

        // Field-dependent loss stays gated.
        let mut scene = Scene::initial();
        scene.materials[0].electric_loss = Some(LossChannel {
            base_rate: ScalarField::constant(0.1),
            law: DampingLaw {
                rate: RateLaw::SaturableAbsorption {
                    saturation: ScalarField::constant(1.0),
                },
                drive: TimeDrive::None,
            },
        });
        assert!(matches!(
            compile(&scene),
            Err(WaveError::MaterialEvaluation {
                coefficient: "loss law",
                ..
            })
        ));
    }

    fn walled(condition: OuterBoundaryCondition, scene: &Scene) -> CanonicalTemporalWaveOperator {
        let mut base_scene = scene.clone();
        strip_temporal_laws(&mut base_scene.materials);
        let mesh = mesh_scene(
            &base_scene,
            1,
            MeshingOptions {
                target_edge_length: 0.3,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic =
            QuadraticWaveOperator::assemble_scene(&mesh, &base_scene, condition).unwrap();
        CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, scene, 1).unwrap()
    }

    const WALLS: [OuterBoundaryCondition; 2] = [
        OuterBoundaryCondition::FirstOrderOutgoing,
        OuterBoundaryCondition::SecondOrderOutgoing,
    ];

    #[test]
    fn a_zero_response_on_a_wall_kicks_as_the_linear_wall_does() {
        // χ = 0 runs the Newton trace solve and the discrete gradient, which
        // must converge onto the linear midpoint kick.
        for condition in WALLS {
            let mut scene = Scene::default();
            let linear = walled(condition, &scene);
            scene.materials[0].mass_law.field = kerr(0.0);
            let nonlinear = walled(condition, &scene);
            assert!(nonlinear.has_field_laws());
            let time_step = 0.4 * linear.maximum_time_step();
            let (primary, complementary) = strong_fluxes(&linear, 6.0);
            let run = |operator: &CanonicalTemporalWaveOperator| {
                let mut state = CanonicalTemporalWaveState::new(
                    operator,
                    time_step,
                    primary.clone(),
                    complementary.clone(),
                )
                .unwrap();
                let mut escaped = 0.0;
                for _ in 0..20 {
                    escaped += state.step(operator).unwrap().boundary_loss;
                }
                (state.primary_flux().to_vec(), escaped)
            };
            let (expected, expected_loss) = run(&linear);
            let (actual, actual_loss) = run(&nonlinear);
            let scale = expected.iter().fold(0.0_f64, |a, b| a.max(b.abs()));
            for (a, b) in expected.iter().zip(&actual) {
                assert!((a - b).abs() <= 1e-12 * scale, "{condition:?}");
            }
            assert!(expected_loss > 0.0);
            assert!((expected_loss - actual_loss).abs() <= 1e-11 * expected_loss);
        }
    }

    #[test]
    fn a_nonlinear_trace_radiates_passively_and_balances_at_second_order() {
        for (condition, pumped) in [(WALLS[0], false), (WALLS[1], false), (WALLS[1], true)] {
            let mut scene = Scene::default();
            scene.materials[0].mass_law.field = kerr(0.8);
            scene.materials[0].stiffness_law.field = saturable_law(6.0, 0.3);
            if pumped {
                // The trace mass then moves under the wall as well.
                scene.materials[0].mass_law.drive = pump(0.2, 1.1, 0.3);
            }
            let operator = walled(condition, &scene);
            let forcing = CanonicalForcing::none(operator.base());
            let (total, _) = nonlinear_balance_is_second_order(&operator, &forcing, 0.2, 0.5);
            assert!(total.boundary_loss > 1e-5, "{condition:?}: {total:?}");
        }
    }

    #[test]
    fn a_nonlinear_wall_reflects_as_the_linear_one_at_small_amplitude() {
        // The wall's departure from its linear control is cubic in the
        // amplitude, like the bulk's: nothing at the wall is first order in
        // the nonlinearity.
        for condition in WALLS {
            let mut scene = Scene::default();
            let linear = walled(condition, &scene);
            scene.materials[0].mass_law.field = kerr(0.8);
            let nonlinear = walled(condition, &scene);
            let time_step = 0.4 * linear.maximum_time_step();
            let departure = |amplitude: f64| {
                let run = |operator: &CanonicalTemporalWaveOperator| {
                    let (primary, complementary) = strong_fluxes(&linear, amplitude);
                    let mut state = CanonicalTemporalWaveState::new(
                        operator,
                        time_step,
                        primary,
                        complementary,
                    )
                    .unwrap();
                    for _ in 0..40 {
                        state.step(operator).unwrap();
                    }
                    state.primary_flux().to_vec()
                };
                let control = run(&linear);
                let actual = run(&nonlinear);
                let norm = control
                    .iter()
                    .map(|value| value * value)
                    .sum::<f64>()
                    .sqrt();
                control
                    .iter()
                    .zip(&actual)
                    .map(|(a, b)| (a - b) * (a - b))
                    .sum::<f64>()
                    .sqrt()
                    / norm
            };
            let ratio = departure(0.5) / departure(0.25);
            assert!((ratio - 4.0).abs() < 0.3, "{condition:?}: ratio {ratio}");
        }
    }

    #[test]
    fn a_strong_kerr_pulse_leaves_through_an_outgoing_wall() {
        // A pulse strong enough that the trace map departs from linear by
        // over 50% crosses the second-order wall. Nothing may pile up at the
        // wall: most of the energy leaves, and the budget closes.
        let mut scene = Scene::default();
        scene.materials[0].mass_law.field = kerr(0.8);
        let operator = walled(OuterBoundaryCondition::SecondOrderOutgoing, &scene);
        let base = operator.base();
        let primary = base
            .node_points()
            .iter()
            .zip(base.primary_mass())
            .map(|(point, mass)| {
                let u = 1.2 * (-(point.x * point.x + point.y * point.y) / 0.08).exp();
                mass * (1.0 + 0.8 * u * u) * u
            })
            .collect::<Vec<_>>();
        let complementary = vec![Point2::default(); base.complementary_degrees_of_freedom()];
        let time_step = 0.4 * operator.maximum_time_step();
        let mut state =
            CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary).unwrap();
        let initial = state.energy(&operator).unwrap();
        let mut escaped = 0.0;
        let steps = (3.0 / time_step).ceil() as usize;
        for _ in 0..steps {
            // The split conserves a nearby energy, not this one, so the
            // stored energy is not monotone step by step; the wall's own
            // dissipation is, and the budget below closes.
            let accounting = state.step(&operator).unwrap();
            assert!(accounting.boundary_loss >= 0.0);
            escaped += accounting.boundary_loss;
        }
        let previous = state.energy(&operator).unwrap();
        assert!(
            previous < 0.2 * initial,
            "{} of the energy stayed",
            previous / initial
        );
        assert!((initial - previous - escaped).abs() < 1e-3 * initial);
    }

    /// A snapshot across one step of `operator` from a strong state, and the
    /// mesh it was compiled on, on the second-order wall.
    fn nonlinear_snapshot(
        scene: &Scene,
    ) -> (
        TriMesh,
        CanonicalTemporalWaveOperator,
        CanonicalIndicatorSnapshot,
    ) {
        let mut base_scene = scene.clone();
        strip_temporal_laws(&mut base_scene.materials);
        let mesh = mesh_scene(
            &base_scene,
            1,
            MeshingOptions {
                target_edge_length: 0.3,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &base_scene,
            OuterBoundaryCondition::SecondOrderOutgoing,
        )
        .unwrap();
        let operator =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, scene, 1).unwrap();
        let (primary, complementary) = strong_fluxes(&operator, 6.0);
        let time_step = 0.4 * operator.maximum_time_step();
        let state =
            CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary).unwrap();
        let mut next = state.clone();
        next.step(&operator).unwrap();
        let snapshot = CanonicalIndicatorSnapshot {
            mesh_revision: mesh.mesh_revision,
            primary_flux: next.primary_flux().to_vec(),
            previous_primary_flux: state.primary_flux().to_vec(),
            complementary_flux: next.complementary_flux().to_vec(),
            previous_complementary_flux: state.complementary_flux().to_vec(),
            auxiliary: next.outgoing_pole_currents().to_vec(),
            previous_auxiliary: state.outgoing_pole_currents().to_vec(),
            integrated_field: vec![],
            previous_integrated_field: vec![],
            time: time_step,
            time_step,
        };
        (mesh, operator, snapshot)
    }

    #[test]
    fn a_point_probe_reads_the_nonlinear_field_the_solver_inverts() {
        let scene = kerr_scene();
        let mut base_scene = scene.clone();
        strip_temporal_laws(&mut base_scene.materials);
        let mesh = mesh_scene(
            &base_scene,
            1,
            MeshingOptions {
                target_edge_length: 0.3,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &base_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let operator =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();
        let runtime = operator.initial_runtime();
        // A uniform field and a uniform complementary field: the probe must
        // read exactly those, and the energy density of the stored maps.
        let (u, v) = (0.4, Point2::new(0.05, -0.03));
        let primary = operator
            .base()
            .primary_mass()
            .iter()
            .map(|mass| mass * (1.0 + 0.8 * u * u) * u)
            .collect::<Vec<_>>();
        let r = v.norm();
        let complementary = operator
            .base()
            .constitutive_samples()
            .iter()
            .map(|sample| {
                let j = sample.complementary_inverse.xx;
                v * ((1.0 + 20.0 * r * r) / j)
            })
            .collect::<Vec<_>>();
        let point = Point2::new(0.1, 0.2);
        let stencil = QuadraticPointStencil::build(&mesh, &quadratic, &base_scene, point).unwrap();
        let probe = CanonicalTemporalPointStencil::from_quadratic(stencil, &operator).unwrap();
        let sample = probe
            .sample(
                &operator,
                &primary,
                &primary,
                &complementary,
                0.1,
                0.1,
                &runtime,
            )
            .unwrap();
        assert!((sample.primary - u).abs() < 1e-12);
        assert!((sample.complementary - v).norm() < 1e-12);
        assert!(sample.primary_rate.abs() < 1e-10);
        let density = probe.fixed().primary_reference * (0.5 * u * u + 0.75 * 0.8 * u.powi(4))
            + probe.fixed().complementary_reference.xx * (0.5 * r * r + 0.75 * 20.0 * r.powi(4));
        assert!((sample.energy_density - density).abs() < 1e-12 * density);
        assert!(
            probe
                .complementary_field(&complementary, 0.1, &runtime)
                .is_err()
        );
    }

    #[test]
    fn nonlinear_invariant_maintenance_shifts_the_field_and_respects_the_domain() {
        let mut scene = Scene::initial();
        scene.materials[0].mass_law.field = bounded_kerr(-0.2, 1.0);
        let operator = compile(&scene).unwrap();
        let base = operator.base();
        let forcing = CanonicalForcing::none(base);
        let field = |u: f64| (1.0 - 0.2 * u * u) * u;
        let uniform = |u: f64| {
            base.primary_mass()
                .iter()
                .map(|mass| mass * field(u))
                .collect::<Vec<_>>()
        };
        let time_step = 0.3 * operator.maximum_time_step();
        let empty = vec![Point2::default(); base.complementary_degrees_of_freedom()];
        let mut state =
            CanonicalTemporalWaveState::new(&operator, time_step, uniform(0.5), empty.clone())
                .unwrap();
        let intended = state.primary_flux().iter().sum::<f64>();
        let drift = 1e-12 * intended;
        state.primary_flux[0] -= drift;
        state.primary_flux[1] += 0.5 * drift;
        let repaired = state
            .maintain_component_totals(&operator, &forcing, &[intended])
            .unwrap();
        assert!((repaired - 0.5 * drift).abs() <= 1e-3 * drift);
        assert!((state.primary_flux().iter().sum::<f64>() - intended).abs() <= 1e-15 * intended);
        // A drift beyond roundoff is refused, not projected away.
        state.primary_flux[0] += 1e-6 * intended;
        let perturbed = state.clone();
        assert!(
            state
                .maintain_component_totals(&operator, &forcing, &[intended])
                .is_err()
        );
        assert_eq!(state, perturbed);
        // At the declared bound a correction outward cannot land.
        let at_bound = uniform(1.0);
        let mut state =
            CanonicalTemporalWaveState::new(&operator, time_step, at_bound, empty).unwrap();
        let total = state.primary_flux().iter().sum::<f64>();
        let kept = state.clone();
        assert!(
            state
                .maintain_component_totals(&operator, &forcing, &[total * (1.0 + 1e-11)])
                .is_err()
        );
        assert_eq!(state, kept);
    }

    #[test]
    fn the_nonlinear_estimate_is_the_linear_one_at_zero_response() {
        let linear_scene = Scene::default();
        let mut scene = Scene::default();
        scene.materials[0].mass_law.field = kerr(0.0);
        scene.materials[0].stiffness_law.field = saturable_law(0.0, 1.0);
        let (mesh, linear, snapshot) = nonlinear_snapshot(&linear_scene);
        let (_, nonlinear, _) = nonlinear_snapshot(&scene);
        assert!(nonlinear.indicator_supplement_supported());
        let forcing = CanonicalForcing::none(linear.base());
        let estimate = |operator: &CanonicalTemporalWaveOperator| {
            canonical_temporal_indicator_supplement(
                &mesh,
                operator,
                &forcing,
                &snapshot,
                &operator.initial_runtime(),
                1.0,
            )
            .unwrap()
        };
        let (expected, actual) = (estimate(&linear), estimate(&nonlinear));
        let close = |a: &[f64], b: &[f64]| {
            let scale = a.iter().fold(0.0_f64, |m, v| m.max(v.abs())).max(1e-300);
            a.iter().zip(b).all(|(a, b)| (a - b).abs() <= 1e-9 * scale)
        };
        assert!(close(&expected.element_energy, &actual.element_energy));
        assert!(close(
            &expected.element_complementary_recovery,
            &actual.element_complementary_recovery
        ));
        assert!(close(
            &expected.element_cell_residual,
            &actual.element_cell_residual
        ));
        assert!(close(
            &expected.element_boundary_residual,
            &actual.element_boundary_residual
        ));
        assert!(expected.outgoing_contribution > 0.0);
    }

    #[test]
    fn the_nonlinear_estimate_splits_the_solver_energy_and_departs_from_linear() {
        let mut scene = Scene::default();
        scene.materials[0].mass_law.field = kerr(0.8);
        scene.materials[0].stiffness_law.field = saturable_law(6.0, 0.3);
        let (mesh, operator, snapshot) = nonlinear_snapshot(&scene);
        let forcing = CanonicalForcing::none(operator.base());
        let runtime = operator.initial_runtime();
        let estimate = canonical_temporal_indicator_supplement(
            &mesh, &operator, &forcing, &snapshot, &runtime, 1.0,
        )
        .unwrap();
        // The element energies are the solver's own nonlinear store, split.
        let stored = operator
            .energy_at(
                &snapshot.primary_flux,
                &snapshot.complementary_flux,
                snapshot.time,
                &runtime,
            )
            .unwrap();
        let split = estimate.element_energy.iter().sum::<f64>();
        assert!(
            (split - stored).abs() <= 1e-12 * stored,
            "{split} against {stored}"
        );
        // Read with the linear maps, the same state is mis-weighed.
        let (_, linear, _) = nonlinear_snapshot(&Scene::default());
        let linear_estimate = canonical_temporal_indicator_supplement(
            &mesh,
            &linear,
            &forcing,
            &snapshot,
            &linear.initial_runtime(),
            1.0,
        )
        .unwrap();
        let linear_split = linear_estimate.element_energy.iter().sum::<f64>();
        assert!((linear_split - stored).abs() > 0.05 * stored);
        assert!(estimate.drift_contribution.is_finite() && estimate.drift_contribution > 0.0);
        assert!(estimate.outgoing_contribution.is_finite() && estimate.outgoing_contribution > 0.0);
    }

    /// The filtered state and the energy it removed, from `primary, complementary`.
    fn filtered(
        operator: &CanonicalTemporalWaveOperator,
        primary: &[f64],
        complementary: &[Point2],
        strength: f64,
    ) -> (CanonicalTemporalWaveState, f64) {
        let mut state = CanonicalTemporalWaveState::new(
            operator,
            0.4 * operator.maximum_time_step(),
            primary.to_vec(),
            complementary.to_vec(),
        )
        .unwrap();
        let removed = state.apply_grid_filter(operator, strength).unwrap();
        (state, removed)
    }

    #[test]
    fn the_tangent_filter_is_the_linear_filter_at_zero_response() {
        let mut scene = Scene::initial();
        scene.materials[0].mass_law.field = kerr(0.0);
        scene.materials[0].stiffness_law.field = saturable_law(0.0, 1.0);
        let nonlinear = compile(&scene).unwrap();
        let linear = compile(&Scene::initial()).unwrap();
        let (primary, complementary) = strong_fluxes(&linear, 6.0);
        let (expected, expected_removed) = filtered(&linear, &primary, &complementary, 0.8);
        let (actual, actual_removed) = filtered(&nonlinear, &primary, &complementary, 0.8);
        assert!(expected_removed > 0.0);
        assert!((expected_removed - actual_removed).abs() <= 1e-10 * expected_removed);
        let scale = primary.iter().fold(0.0_f64, |a, b| a.max(b.abs()));
        for (a, b) in expected.primary_flux().iter().zip(actual.primary_flux()) {
            assert!((a - b).abs() <= 1e-12 * scale);
        }
        for (a, b) in expected
            .complementary_flux()
            .iter()
            .zip(actual.complementary_flux())
        {
            assert!((*a - *b).norm() <= 1e-12);
        }
    }

    #[test]
    fn the_tangent_filter_keeps_its_invariants_and_never_adds_energy() {
        let operator = compile(&kerr_scene()).unwrap();
        let base = operator.base();
        // A constant field is untouched, whatever the map: K_t U = 0.
        let held = base
            .primary_mass()
            .iter()
            .map(|mass| mass * (1.0 + 0.8 * 0.09) * 0.3)
            .collect::<Vec<_>>();
        let empty = vec![Point2::default(); base.complementary_degrees_of_freedom()];
        let (state, removed) = filtered(&operator, &held, &empty, 1.0);
        assert_eq!(removed, 0.0);
        assert_eq!(state.primary_flux(), held);

        // A strong state keeps its total and its compatibility.
        let (primary, _) = strong_fluxes(&operator, 6.0);
        let potential = (0..base.degrees_of_freedom())
            .map(|index| 0.2 * (index as f64 * 1.713).sin())
            .collect::<Vec<_>>();
        let compatible = base.compatible_flux(&potential).unwrap();
        let (state, removed) = filtered(&operator, &primary, &compatible, 1.0);
        assert!(removed > 0.0);
        let total = |values: &[f64]| values.iter().sum::<f64>();
        let magnitude = primary.iter().map(|value| value.abs()).sum::<f64>();
        assert!((total(state.primary_flux()) - total(&primary)).abs() <= 1e-13 * magnitude);
        let stationary = base
            .stationary_complementary_component(state.complementary_flux())
            .unwrap();
        let norm = |values: &[Point2]| values.iter().map(|v| v.norm().powi(2)).sum::<f64>().sqrt();
        assert!(norm(&stationary) <= 1e-10 * norm(state.complementary_flux()));

        // Interleaved with steps at full strength, every filter removes.
        let mut state = CanonicalTemporalWaveState::new(
            &operator,
            0.4 * operator.maximum_time_step(),
            primary,
            compatible,
        )
        .unwrap();
        for _ in 0..20 {
            state.step(&operator).unwrap();
            assert!(state.apply_grid_filter(&operator, 1.0).unwrap() >= 0.0);
        }
    }

    #[test]
    fn the_tangent_filter_departs_from_the_linear_one_at_the_square_of_the_amplitude() {
        let nonlinear = compile(&kerr_scene()).unwrap();
        let linear = compile(&Scene::initial()).unwrap();
        let departure = |amplitude: f64| {
            let (primary, complementary) = strong_fluxes(&linear, amplitude);
            let (expected, _) = filtered(&linear, &primary, &complementary, 1.0);
            let (actual, _) = filtered(&nonlinear, &primary, &complementary, 1.0);
            // Compare the corrections, not the states: the correction is what
            // the filter adds, and it is small next to the state.
            let correction = |state: &CanonicalTemporalWaveState| {
                state
                    .primary_flux()
                    .iter()
                    .zip(&primary)
                    .map(|(after, before)| after - before)
                    .collect::<Vec<_>>()
            };
            let (a, b) = (correction(&expected), correction(&actual));
            let norm = a.iter().map(|v| v * v).sum::<f64>().sqrt();
            a.iter()
                .zip(&b)
                .map(|(x, y)| (x - y) * (x - y))
                .sum::<f64>()
                .sqrt()
                / norm
        };
        let ratio = departure(0.5) / departure(0.25);
        assert!((ratio - 4.0).abs() < 0.3, "ratio {ratio}");
    }

    #[test]
    fn a_zero_response_steps_as_the_linear_medium_does() {
        // χ = 0 takes every nonlinear code path (inverse, discrete energy,
        // forward map) and must reproduce the linear division.
        let mut scene = Scene::initial();
        scene.materials[0].mass_law.field = kerr(0.0);
        scene.materials[0].stiffness_law.field = saturable_law(0.0, 1.0);
        let nonlinear = compile(&scene).unwrap();
        assert!(nonlinear.has_field_laws());
        let linear = compile(&Scene::initial()).unwrap();
        let time_step = 0.4 * linear.maximum_time_step();
        let (primary, complementary) = strong_fluxes(&linear, 6.0);
        let mut left = CanonicalTemporalWaveState::new(
            &linear,
            time_step,
            primary.clone(),
            complementary.clone(),
        )
        .unwrap();
        let mut right =
            CanonicalTemporalWaveState::new(&nonlinear, time_step, primary, complementary).unwrap();
        for _ in 0..20 {
            left.step(&linear).unwrap();
            right.step(&nonlinear).unwrap();
        }
        let scale = left
            .primary_flux()
            .iter()
            .fold(0.0_f64, |a, b| a.max(b.abs()));
        for (a, b) in left.primary_flux().iter().zip(right.primary_flux()) {
            assert!((a - b).abs() <= 1e-13 * scale);
        }
        let energy = left.energy(&linear).unwrap();
        assert!((energy - right.energy(&nonlinear).unwrap()).abs() <= 1e-13 * energy);
    }

    #[test]
    fn the_stored_energy_is_the_potential_of_both_observables() {
        // The Hamiltonian pairing the split relies on: ∂H/∂Q = U and
        // ∂H/∂b = W·v, with the nonlinear maps.
        let operator = compile(&kerr_scene()).unwrap();
        let runtime = operator.initial_runtime();
        let (primary, complementary) = strong_fluxes(&operator, 6.0);
        let field = operator.primary_field_at(&primary, 0.0, &runtime).unwrap();
        let vector = operator
            .complementary_field_at(&complementary, 0.0, &runtime)
            .unwrap();
        let energy = |p: &[f64], c: &[Point2]| operator.energy_at(p, c, 0.0, &runtime).unwrap();
        for node in [0, 3, primary.len() / 2, primary.len() - 1] {
            let h = 1e-6 * primary[node].abs().max(1e-6);
            let mut plus = primary.clone();
            let mut minus = primary.clone();
            plus[node] += h;
            minus[node] -= h;
            let slope =
                (energy(&plus, &complementary) - energy(&minus, &complementary)) / (2.0 * h);
            assert!((slope - field[node]).abs() <= 1e-6 * field[node].abs().max(1e-3));
        }
        for sample in [0, 7, complementary.len() - 1] {
            let weight = operator.base().constitutive_samples()[sample].integration_weight;
            let h = 1e-4;
            let mut plus = complementary.clone();
            let mut minus = complementary.clone();
            plus[sample].x += h;
            minus[sample].x -= h;
            let slope = (energy(&primary, &plus) - energy(&primary, &minus)) / (2.0 * h);
            assert!(
                (slope - weight * vector[sample].x).abs()
                    <= 1e-6 * weight * vector[sample].norm() + 1e-12
            );
        }
        // And the maps really are nonlinear at this amplitude.
        let linear = operator.base().primary_field(&primary).unwrap();
        let departure = field
            .iter()
            .zip(&linear)
            .map(|(a, b)| (a - b).abs() / b.abs().max(1e-12))
            .fold(0.0_f64, f64::max);
        assert!(departure > 0.1, "{departure}");
    }

    #[test]
    fn a_kerr_bulk_conserves_its_energy_to_second_order() {
        let operator = compile(&kerr_scene()).unwrap();
        let (primary, complementary) = strong_fluxes(&operator, 6.0);
        let deviation = |fraction: f64, steps: usize| {
            let time_step = fraction * operator.maximum_time_step();
            let mut state = CanonicalTemporalWaveState::new(
                &operator,
                time_step,
                primary.clone(),
                complementary.clone(),
            )
            .unwrap();
            let initial = state.energy(&operator).unwrap();
            let mut worst = 0.0_f64;
            for _ in 0..steps {
                let accounting = state.step(&operator).unwrap();
                assert_eq!(accounting.temporal_work, 0.0);
                worst = worst.max((state.energy(&operator).unwrap() - initial).abs() / initial);
            }
            worst
        };
        let coarse = deviation(0.5, 200);
        let fine = deviation(0.25, 400);
        assert!(coarse < 2e-4, "{coarse:e}");
        assert!(fine < 0.35 * coarse, "coarse {coarse:e}, fine {fine:e}");
    }

    #[test]
    fn a_driven_kerr_bulk_is_reversible_and_balances_its_temporal_work() {
        let mut scene = kerr_scene();
        scene.materials[0].mass_law.drive = pump(0.24, 0.8, 0.31);
        scene.materials[0].stiffness_law.drive = pump(0.17, 0.6, -0.23);
        let operator = compile(&scene).unwrap();
        assert!(operator.has_field_laws() && operator.has_temporal_laws());
        let (primary, complementary) = strong_fluxes(&operator, 6.0);

        let time_step = 0.35 * operator.maximum_time_step();
        let mut state = CanonicalTemporalWaveState::new(
            &operator,
            time_step,
            primary.clone(),
            complementary.clone(),
        )
        .unwrap();
        state.step_by(&operator, time_step).unwrap();
        state.step_by(&operator, -time_step).unwrap();
        let scale = primary.iter().fold(0.0_f64, |a, b| a.max(b.abs()));
        for (actual, expected) in state.primary_flux().iter().zip(&primary) {
            assert!((actual - expected).abs() < 1e-13 * scale);
        }

        // The explicit rate at fixed state is the time derivative of H.
        let runtime = operator.initial_runtime();
        let (_, rate) = operator
            .energy_and_rate_at(&primary, &complementary, 0.37, &runtime)
            .unwrap();
        let epsilon = 1e-6;
        let numerical = (operator
            .energy_at(&primary, &complementary, 0.37 + epsilon, &runtime)
            .unwrap()
            - operator
                .energy_at(&primary, &complementary, 0.37 - epsilon, &runtime)
                .unwrap())
            / (2.0 * epsilon);
        assert!(
            (rate - numerical).abs() < 2e-8 * rate.abs().max(1e-6),
            "{rate} {numerical}"
        );

        let residual = |fraction: f64, steps: usize| {
            let mut state = CanonicalTemporalWaveState::new(
                &operator,
                fraction * operator.maximum_time_step(),
                primary.clone(),
                complementary.clone(),
            )
            .unwrap();
            (0..steps)
                .map(|_| state.step(&operator).unwrap().splitting_residual)
                .sum::<f64>()
                .abs()
        };
        let coarse = residual(0.6, 16);
        let fine = residual(0.3, 32);
        assert!(coarse > 1e-12);
        assert!(fine < 0.35 * coarse, "coarse {coarse:e}, fine {fine:e}");
    }

    #[test]
    fn a_junction_node_holds_the_sum_of_its_materials_maps() {
        let mut scene = Scene::default();
        let mut second = scene.materials[0].clone();
        second.id = MaterialId(2);
        second.name = "Second".into();
        second.mass_density = ScalarField::constant(2.5);
        scene.materials[0].mass_law.field = kerr(0.8);
        second.mass_law.field = saturable_law(1.5, 0.3);
        scene.materials.push(second);
        scene.regions.push(Region {
            id: RegionId(2),
            material: MaterialId(2),
            frame: MaterialFrame::world(),
        });
        scene.obstacles.push(Obstacle::with_role(
            ObstacleId(1),
            PeriodicCubicSpline::rounded(Point2::new(0.5, 0.5), 0.2),
            LoopRole::MaterialInterface {
                exterior: BACKGROUND_REGION,
                interior: RegionId(2),
            },
        ));
        let operator = compile(&scene).unwrap();
        let runtime = operator.initial_runtime();
        let (primary, complementary) = strong_fluxes(&operator, 6.0);
        let field = operator.primary_field_at(&primary, 0.0, &runtime).unwrap();
        let mut assembled = vec![0.0; primary.len()];
        let mut owners = vec![BTreeSet::new(); primary.len()];
        for (contribution, sample) in operator
            .base()
            .primary_contributions()
            .iter()
            .zip(&operator.primary)
        {
            let node = contribution.node as usize;
            let u = field[node];
            assembled[node] += contribution.geometric_weight
                * contribution.reference_coefficient
                * sample.coefficient.law.field.multiplier(u.abs())
                * u;
            owners[node].insert(sample.coefficient.material);
        }
        let mut shared = 0;
        for node in 0..primary.len() {
            shared += usize::from(owners[node].len() == 2);
            assert!(
                (assembled[node] - primary[node]).abs() <= 1e-14 * primary[node].abs().max(1e-12)
            );
        }
        assert!(shared >= 3);
        let time_step = 0.4 * operator.maximum_time_step();
        let mut state =
            CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary).unwrap();
        let initial = state.energy(&operator).unwrap();
        for _ in 0..50 {
            state.step(&operator).unwrap();
        }
        assert!((state.energy(&operator).unwrap() - initial).abs() < 1e-3 * initial);
    }

    #[test]
    fn tm_and_te_exchange_electric_and_magnetic_laws() {
        // Duality: an electric law in TM and the same magnetic law in TE put
        // the identical map on the primary row, so the two evolve alike; the
        // same law on the same slot does not.
        let skin = |polarization| {
            let mut scene = Scene::initial();
            scene.physics = PhysicsModel::Electromagnetic { polarization };
            scene
        };
        let mut tm = skin(ElectromagneticPolarization::Tm);
        tm.materials[0].mass_law.field = kerr(0.8);
        let mut te = skin(ElectromagneticPolarization::Te);
        te.materials[0].stiffness_law.field = kerr(0.8);
        let tm = compile(&tm).unwrap();
        let te_operator = compile(&te).unwrap();
        assert_eq!(tm.base().primary_mass(), te_operator.base().primary_mass());
        // TE runs the curl with the opposite orientation, so the dual state
        // carries the opposite complementary flux: `(Q, b) ↦ (Q, −b)` maps
        // one evolution onto the other because every executed map is odd.
        assert_eq!(tm.base().orientation(), -te_operator.base().orientation());
        let run = |operator: &CanonicalTemporalWaveOperator| {
            let (primary, complementary) = strong_fluxes(operator, 6.0);
            let orientation = operator.base().orientation();
            let complementary = complementary
                .into_iter()
                .map(|flux| flux * orientation)
                .collect();
            let mut state = CanonicalTemporalWaveState::new(
                operator,
                0.4 * operator.maximum_time_step(),
                primary,
                complementary,
            )
            .unwrap();
            for _ in 0..30 {
                state.step(operator).unwrap();
            }
            state.primary_flux().to_vec()
        };
        let reference = run(&tm);
        let dual = run(&te_operator);
        let scale = reference.iter().fold(0.0_f64, |a, b| a.max(b.abs()));
        for (a, b) in reference.iter().zip(&dual) {
            assert!((a - b).abs() <= 1e-12 * scale);
        }
        te.materials[0].stiffness_law.field = FieldLaw::Linear;
        te.materials[0].mass_law.field = kerr(0.8);
        let swapped = run(&compile(&te).unwrap());
        let difference = reference
            .iter()
            .zip(&swapped)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f64, f64::max);
        assert!(difference > 1e-4 * scale);
    }

    #[test]
    fn a_flux_past_a_declared_bound_is_refused_and_leaves_the_state_intact() {
        let mut scene = Scene::initial();
        scene.materials[0].mass_law.field = bounded_kerr(-0.2, 1.0);
        let operator = compile(&scene).unwrap();
        let mass = operator.base().primary_mass().to_vec();
        let time_step = 0.4 * operator.maximum_time_step();
        // P(1) = m·0.8: a field of 0.9 is inside, a flux of 0.9·m is not.
        let mut outside = vec![0.0; mass.len()];
        outside[4] = 0.9 * mass[4];
        let empty = vec![Point2::default(); operator.base().complementary_degrees_of_freedom()];
        assert!(matches!(
            CanonicalTemporalWaveState::new(&operator, time_step, outside, empty.clone()),
            Err(WaveError::MaterialEvaluation {
                coefficient: "field response",
                ..
            })
        ));
        // Q = P(0.3) = m·(1 − 0.2·0.09)·0.3 everywhere.
        let inside = mass
            .iter()
            .map(|m| m * (1.0 - 0.2 * 0.09) * 0.3)
            .collect::<Vec<_>>();
        let mut state =
            CanonicalTemporalWaveState::new(&operator, time_step, inside, empty).unwrap();
        let forcing = CanonicalForcing::none(operator.base());
        let before = state.clone();
        let mut pulse = vec![0.0; mass.len()];
        pulse[4] = 0.8;
        assert!(
            state
                .apply_primary_pulse(&operator, &forcing, &pulse)
                .is_err()
        );
        assert_eq!(state, before);
        // A pulse the domain holds lands through the map: the field moves by
        // exactly the increment.
        pulse[4] = 0.5;
        state
            .apply_primary_pulse(&operator, &forcing, &pulse)
            .unwrap();
        let runtime = operator.initial_runtime();
        let field = operator
            .primary_field_at(state.primary_flux(), 0.0, &runtime)
            .unwrap();
        assert!((field[4] - 0.8).abs() < 1e-14, "{}", field[4]);
        assert!((field[5] - 0.3).abs() < 1e-14);
    }

    #[test]
    fn kerr_departs_from_its_linear_control_at_the_square_of_the_amplitude() {
        // The leading nonlinear effect of a cubic law on a fixed-time
        // trajectory is third order in the amplitude, so its departure from
        // the linear control, relative to the trajectory, grows as A².
        let nonlinear = compile(&kerr_scene()).unwrap();
        let linear = compile(&Scene::initial()).unwrap();
        let time_step = 0.4 * linear.maximum_time_step();
        let departure = |amplitude: f64| {
            let run = |operator: &CanonicalTemporalWaveOperator| {
                let (primary, complementary) = strong_fluxes(&linear, amplitude);
                let mut state =
                    CanonicalTemporalWaveState::new(operator, time_step, primary, complementary)
                        .unwrap();
                for _ in 0..60 {
                    state.step(operator).unwrap();
                }
                state.primary_flux().to_vec()
            };
            let control = run(&linear);
            let actual = run(&nonlinear);
            let norm = control
                .iter()
                .map(|value| value * value)
                .sum::<f64>()
                .sqrt();
            control
                .iter()
                .zip(&actual)
                .map(|(a, b)| (a - b) * (a - b))
                .sum::<f64>()
                .sqrt()
                / norm
        };
        let small = departure(0.25);
        let double = departure(0.5);
        let ratio = double / small;
        assert!((ratio - 4.0).abs() < 0.2, "ratio {ratio}");
    }

    #[test]
    fn a_saturated_medium_behaves_as_its_limiting_linear_one() {
        // Far above saturation `ḡ → 1 + χσ²`, so a very strong field runs as
        // a linear medium with that coefficient: the response stays bounded.
        let (chi, saturation) = (0.6, 0.002);
        let mut scene = Scene::initial();
        scene.materials[0].mass_law.field =
            saturable_law(chi / (saturation * saturation), saturation);
        let saturated = compile(&scene).unwrap();
        let mut limit = Scene::initial();
        limit.materials[0].mass_density = ScalarField::constant(
            limit.materials[0].mass_density.constant_value().unwrap() * (1.0 + chi),
        );
        let limit = compile(&limit).unwrap();
        let time_step = 0.4 * saturated.maximum_time_step().min(limit.maximum_time_step());
        let (primary, complementary) = strong_fluxes(&limit, 30.0);
        let run = |operator: &CanonicalTemporalWaveOperator| {
            let mut state = CanonicalTemporalWaveState::new(
                operator,
                time_step,
                primary.clone(),
                complementary.clone(),
            )
            .unwrap();
            for _ in 0..60 {
                state.step(operator).unwrap();
            }
            state.primary_flux().to_vec()
        };
        let expected = run(&limit);
        let actual = run(&saturated);
        let norm = expected
            .iter()
            .map(|value| value * value)
            .sum::<f64>()
            .sqrt();
        let difference = expected
            .iter()
            .zip(&actual)
            .map(|(a, b)| (a - b) * (a - b))
            .sum::<f64>()
            .sqrt();
        assert!(difference < 2e-3 * norm, "{}", difference / norm);
    }

    /// Steps `operator` to `target` at three refinements from a strong field
    /// and asserts that what the lanes leave unaccounted converges at second
    /// order: the composition's own splitting error and nothing else. Returns
    /// the finest run's `(lane total, unaccounted)` for the caller's checks.
    fn nonlinear_balance_is_second_order(
        operator: &CanonicalTemporalWaveOperator,
        forcing: &CanonicalForcing,
        target: f64,
        amplitude: f64,
    ) -> (CanonicalTemporalStepAccounting, f64) {
        let base = operator.base();
        let ceiling = 0.4 * operator.maximum_time_step();
        let mut previous: Option<(f64, f64)> = None;
        let mut finest = (CanonicalTemporalStepAccounting::default(), 0.0);
        for refinement in [1.0, 0.5, 0.25] {
            let steps = (target / (ceiling * refinement)).ceil() as u64;
            let time_step = target / steps as f64;
            let mass = base.primary_mass();
            let primary = base
                .node_points()
                .iter()
                .zip(mass)
                .map(|(point, mass)| amplitude * mass * (1.4 * point.x - 0.9 * point.y).sin())
                .collect::<Vec<_>>();
            let potential = base
                .node_points()
                .iter()
                .map(|point| 0.6 * amplitude * (0.8 * point.x + 1.2 * point.y).cos())
                .collect::<Vec<_>>();
            let complementary = base.compatible_flux(&potential).unwrap();
            let mut state =
                CanonicalTemporalWaveState::new(operator, time_step, primary, complementary)
                    .unwrap()
                    .pinned(operator, forcing)
                    .unwrap();
            if operator.has_restoring() {
                // Far enough from zero that sine-Gordon is not its tangent.
                let integrated = base
                    .node_points()
                    .iter()
                    .map(|point| 1.2 * (0.7 * point.x + 0.5 * point.y).cos())
                    .collect();
                state = state.with_integrated_field(operator, integrated).unwrap();
            }
            let before = state.energy(operator).unwrap();
            let mut total = CanonicalTemporalStepAccounting::default();
            for _ in 0..steps {
                let step = state.step_with_forcing(operator, forcing).unwrap();
                assert!(step.primary_loss >= 0.0 && step.complementary_loss >= 0.0);
                assert!(step.boundary_loss >= 0.0);
                total.temporal_work += step.temporal_work;
                total.source_work += step.source_work;
                total.prescribed_exchange += step.prescribed_exchange;
                total.primary_loss += step.primary_loss;
                total.complementary_loss += step.complementary_loss;
                total.boundary_loss += step.boundary_loss;
                total.active_gain += step.active_gain;
            }
            let after = state.energy(operator).unwrap();
            let unaccounted = after
                - before
                - total.temporal_work
                - total.source_work
                - total.prescribed_exchange
                - total.active_gain
                + total.primary_loss
                + total.complementary_loss
                + total.boundary_loss;
            if let Some((coarse_step, coarse)) = previous {
                let order =
                    (coarse.abs() / unaccounted.abs()).log2() / (coarse_step / time_step).log2();
                assert!(
                    order > 1.7,
                    "measured order {order:.2} between dt {coarse_step:.3e} and {time_step:.3e}"
                );
            }
            previous = Some((time_step, unaccounted));
            finest = (total, unaccounted);
        }
        finest
    }

    fn nonlinear_default_scene() -> Scene {
        let mut scene = Scene::default();
        scene.materials[0].mass_law.field = kerr(0.8);
        scene.materials[0].stiffness_law.field = saturable_law(6.0, 0.3);
        scene
    }

    #[test]
    fn nonlinear_media_compose_sources_and_prescribed_data() {
        let mut scene = nonlinear_default_scene();
        scene.materials[0].mass_law.drive = pump(0.2, 1.1, 0.3);
        let operator = compile(&scene).unwrap();
        let base = operator.base();
        let mut prescribed = vec![None; base.degrees_of_freedom()];
        for (node, point) in base.node_points().iter().enumerate() {
            if point.x < -0.999 {
                prescribed[node] = Some(TimeSignal::harmonic(0.3, 0.2, 1.3, 0.2));
            }
        }
        assert!(prescribed.iter().any(Option::is_some));
        let mut forcing = CanonicalForcing::from_prescribed(base, prescribed).unwrap();
        forcing
            .push_source(
                CanonicalSource::direct(
                    base,
                    base.primary_mass().to_vec(),
                    TimeSignal::harmonic(0.0, 0.9, 1.7, 0.4),
                )
                .unwrap(),
            )
            .unwrap();
        let (total, _) = nonlinear_balance_is_second_order(&operator, &forcing, 0.3, 0.5);
        assert!(total.source_work.abs() > 1e-4, "{total:?}");
        assert!(total.prescribed_exchange.abs() > 1e-4, "{total:?}");
    }

    #[test]
    fn a_pinned_nonlinear_node_holds_its_field_not_its_linear_flux() {
        let operator = compile(&nonlinear_default_scene()).unwrap();
        let base = operator.base();
        let held = 0.7;
        let prescribed = vec![
            Some(TimeSignal::Harmonic {
                offset: held,
                amplitude: 0.0,
                frequency_hz: 0.0,
                phase_radians: 0.0,
            });
            base.degrees_of_freedom()
        ];
        let forcing = CanonicalForcing::from_prescribed(base, prescribed).unwrap();
        let mut state =
            CanonicalTemporalWaveState::zero(&operator, 0.3 * operator.maximum_time_step())
                .unwrap()
                .pinned(&operator, &forcing)
                .unwrap();
        for _ in 0..5 {
            let step = state.step_with_forcing(&operator, &forcing).unwrap();
            // A held field in a fixed medium exchanges nothing.
            assert!(step.prescribed_exchange.abs() < 1e-13);
        }
        let runtime = operator.initial_runtime();
        let field = operator
            .primary_field_at(state.primary_flux(), state.time(), &runtime)
            .unwrap();
        for (field, (flux, mass)) in field
            .iter()
            .zip(state.primary_flux().iter().zip(base.primary_mass()))
        {
            assert!((field - held).abs() < 1e-13);
            assert!((flux - mass * (1.0 + 0.8 * held * held) * held).abs() < 1e-13 * flux);
        }
    }

    #[test]
    fn a_lossy_nonlinear_medium_dissipates_passively_at_second_order() {
        let mut scene = nonlinear_default_scene();
        let channel = |rate: f64| LossChannel {
            base_rate: ScalarField::constant(rate),
            law: DampingLaw {
                rate: RateLaw::Constant,
                drive: TimeDrive::None,
            },
        };
        scene.materials[0].electric_loss = Some(channel(0.5));
        scene.materials[0].magnetic_loss = Some(channel(0.3));
        let operator = compile(&scene).unwrap();
        let forcing = CanonicalForcing::none(operator.base());
        let (total, _) = nonlinear_balance_is_second_order(&operator, &forcing, 0.3, 0.5);
        assert!(
            total.primary_loss > 1e-4 && total.complementary_loss > 1e-4,
            "{total:?}"
        );
    }

    #[test]
    fn a_thin_gap_in_a_nonlinear_medium_keeps_the_balance_second_order() {
        let mut scene = nonlinear_default_scene();
        scene.internal_boundaries.push(crate::InternalBoundary {
            id: crate::InternalBoundaryId(1),
            spline: crate::OpenCubicSpline::uniform(vec![
                Point2::new(-0.65, 0.0),
                Point2::new(-0.2, 0.0),
                Point2::new(0.2, 0.0),
                Point2::new(0.65, 0.0),
            ])
            .unwrap(),
            region: BACKGROUND_REGION,
            span_laws: vec![crate::InternalBoundaryLaw {
                coupling: crate::InternalBoundaryCoupling::ThinGap {
                    stiffness_ratio: 120.0,
                },
                ..crate::InternalBoundaryLaw::REFLECTING
            }],
        });
        let mut base_scene = scene.clone();
        strip_temporal_laws(&mut base_scene.materials);
        let mesh = mesh_scene(
            &base_scene,
            17,
            MeshingOptions {
                target_edge_length: 0.18,
                minimum_angle_degrees: 14.0,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &base_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let operator =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();
        assert!(!operator.base().thin_gap_samples().is_empty());
        let forcing = CanonicalForcing::none(operator.base());
        nonlinear_balance_is_second_order(&operator, &forcing, 0.25, 0.5);
    }

    #[test]
    fn a_complementary_nonlinearity_radiates_through_both_outgoing_walls() {
        for condition in [
            OuterBoundaryCondition::FirstOrderOutgoing,
            OuterBoundaryCondition::SecondOrderOutgoing,
        ] {
            let mut scene = Scene::default();
            let mesh = mesh_scene(
                &scene,
                1,
                MeshingOptions {
                    target_edge_length: 0.3,
                    ..MeshingOptions::default()
                },
            )
            .unwrap();
            let quadratic =
                QuadraticWaveOperator::assemble_scene(&mesh, &scene, condition).unwrap();
            scene.materials[0].stiffness_law.field = saturable_law(6.0, 0.3);
            let operator =
                CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();
            let forcing = CanonicalForcing::none(operator.base());
            let (total, _) = nonlinear_balance_is_second_order(&operator, &forcing, 0.2, 0.5);
            assert!(total.boundary_loss > 1e-5, "{condition:?}: {total:?}");
        }
    }

    /// Prescribed data used to reach the drift through its endpoint flux, so
    /// the drift saw `g(t_n)` and the trajectory was first order (ratios 2.2
    /// and 2.1 here, against the fixed path's 4.1). The drift now reads the
    /// signal at its own instant.
    #[test]
    fn a_varying_prescribed_signal_steps_at_second_order() {
        let operator = compile(&Scene::default()).unwrap();
        let base = operator.base();
        let mut prescribed = vec![None; base.degrees_of_freedom()];
        for (node, point) in base.node_points().iter().enumerate() {
            if point.x < -0.999 {
                prescribed[node] = Some(TimeSignal::harmonic(0.3, 0.2, 1.3, 0.2));
            }
        }
        let forcing = CanonicalForcing::from_prescribed(base, prescribed).unwrap();
        let run = |steps: u64| {
            let time_step = 0.3 / steps as f64;
            let mut state = CanonicalTemporalWaveState::zero(&operator, time_step)
                .unwrap()
                .pinned(&operator, &forcing)
                .unwrap();
            for _ in 0..steps {
                state.step_with_forcing(&operator, &forcing).unwrap();
            }
            state.complementary_flux().to_vec()
        };
        let reference = run(2000);
        let error = |steps| {
            run(steps)
                .iter()
                .zip(&reference)
                .map(|(a, b)| (*a - *b).norm())
                .fold(0.0_f64, f64::max)
        };
        let (coarse, middle, fine) = (error(50), error(100), error(200));
        assert!(coarse / middle > 3.6, "{coarse:e} {middle:e}");
        assert!(middle / fine > 3.6, "{middle:e} {fine:e}");
    }

    /// The same defect under a breathing mass: a constant held value beside a
    /// moving free field balanced at order 0.5-0.9.
    #[test]
    fn prescribed_data_beside_a_pumped_mass_balances_at_second_order() {
        for signal in [
            TimeSignal::Harmonic {
                offset: 0.3,
                amplitude: 0.0,
                frequency_hz: 0.0,
                phase_radians: 0.0,
            },
            TimeSignal::harmonic(0.3, 0.2, 1.3, 0.2),
        ] {
            let mut scene = Scene::default();
            scene.materials[0].mass_law.drive = pump(0.2, 1.1, 0.3);
            let operator = compile(&scene).unwrap();
            let base = operator.base();
            let mut prescribed = vec![None; base.degrees_of_freedom()];
            for (node, point) in base.node_points().iter().enumerate() {
                if point.x < -0.999 {
                    prescribed[node] = Some(signal);
                }
            }
            let forcing = CanonicalForcing::from_prescribed(base, prescribed).unwrap();
            let (total, _) = nonlinear_balance_is_second_order(&operator, &forcing, 0.3, 0.5);
            assert!(total.prescribed_exchange.abs() > 1e-4, "{total:?}");
        }
    }

    /// Mechanical ↔ TE is a presentation change: the compiled maps, and so
    /// the evolution, must be the same before and after. Until 24 September a
    /// stiffness-row drive crossed reciprocated and ran backwards in TE
    /// (`v = 1.3 b` against `b / 1.3` at a pump crest).
    #[test]
    fn mechanical_and_te_evolve_alike_through_a_skin_change() {
        let te = PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        };
        let mut pumped = Scene::initial();
        pumped.materials[0].stiffness_law.drive = pump(0.3, 0.9, 0.2);
        pumped.materials[0].stiffness_law.alternate = Some(ScalarField::constant(1.4));
        pumped.materials[0].mass_law.drive = pump(0.2, 0.7, -0.1);
        let mut kerr_medium = kerr_scene();
        kerr_medium.materials[0].stiffness_law.drive = pump(0.2, 0.9, 0.2);
        for (label, mechanical) in [("pumped", pumped), ("kerr", kerr_medium)] {
            let mut electromagnetic = mechanical.clone();
            electromagnetic.physics = te;
            electromagnetic.materials = mechanical
                .materials
                .iter()
                .map(|material| {
                    PhysicsModel::Mechanical
                        .convert_material(te, material)
                        .unwrap()
                })
                .collect();
            let left = compile(&mechanical).unwrap();
            let right = compile(&electromagnetic).unwrap();
            assert_eq!(left.base().orientation(), right.base().orientation());
            assert_eq!(left.maximum_time_step(), right.maximum_time_step());
            let run = |operator: &CanonicalTemporalWaveOperator| {
                let (primary, complementary) = strong_fluxes(operator, 4.0);
                let mut state = CanonicalTemporalWaveState::new(
                    operator,
                    0.4 * operator.maximum_time_step(),
                    primary,
                    complementary,
                )
                .unwrap();
                for _ in 0..40 {
                    state.step(operator).unwrap();
                }
                (
                    state.primary_flux().to_vec(),
                    state.complementary_flux().to_vec(),
                )
            };
            let (expected_primary, expected_complementary) = run(&left);
            let (actual_primary, actual_complementary) = run(&right);
            let scale = expected_primary.iter().fold(0.0_f64, |a, b| a.max(b.abs()));
            for (a, b) in expected_primary.iter().zip(&actual_primary) {
                assert!((a - b).abs() <= 1e-12 * scale, "{label}");
            }
            for (a, b) in expected_complementary.iter().zip(&actual_complementary) {
                assert!((*a - *b).norm() <= 1e-12, "{label}");
            }
        }
    }

    /// Each composition the stepper admits beside the bulk: both walls, a
    /// prescribed wall, a thin gap and a constant loss.
    fn filter_compositions() -> Vec<(&'static str, Scene, OuterBoundaryCondition)> {
        let mut gapped = Scene::default();
        gapped.internal_boundaries.push(crate::InternalBoundary {
            id: crate::InternalBoundaryId(1),
            spline: crate::OpenCubicSpline::uniform(vec![
                Point2::new(-0.65, 0.0),
                Point2::new(-0.2, 0.0),
                Point2::new(0.2, 0.0),
                Point2::new(0.65, 0.0),
            ])
            .unwrap(),
            region: BACKGROUND_REGION,
            span_laws: vec![crate::InternalBoundaryLaw {
                coupling: crate::InternalBoundaryCoupling::ThinGap {
                    stiffness_ratio: 120.0,
                },
                ..crate::InternalBoundaryLaw::REFLECTING
            }],
        });
        let mut lossy = Scene::default();
        lossy.materials[0].damping = ScalarField::constant(0.45);
        vec![
            (
                "first-order wall",
                Scene::default(),
                OuterBoundaryCondition::FirstOrderOutgoing,
            ),
            (
                "second-order wall",
                Scene::default(),
                OuterBoundaryCondition::SecondOrderOutgoing,
            ),
            (
                "prescribed wall",
                Scene::default(),
                OuterBoundaryCondition::Reflecting,
            ),
            ("thin gap", gapped, OuterBoundaryCondition::Reflecting),
            ("loss", lossy, OuterBoundaryCondition::Reflecting),
        ]
    }

    fn filter_operator(
        scene: &Scene,
        condition: OuterBoundaryCondition,
    ) -> CanonicalTemporalWaveOperator {
        let mut base_scene = scene.clone();
        strip_temporal_laws(&mut base_scene.materials);
        let mesh = mesh_scene(
            &base_scene,
            17,
            MeshingOptions {
                target_edge_length: 0.2,
                minimum_angle_degrees: 14.0,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic =
            QuadraticWaveOperator::assemble_scene(&mesh, &base_scene, condition).unwrap();
        CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, scene, 1).unwrap()
    }

    /// A constant prescribed wall on the left for the case that asks for one.
    fn filter_forcing(label: &str, base: &CanonicalWaveOperator) -> CanonicalForcing {
        if label != "prescribed wall" {
            return CanonicalForcing::none(base);
        }
        let prescribed = base
            .node_points()
            .iter()
            .map(|point| {
                (point.x < -0.999).then_some(TimeSignal::Harmonic {
                    offset: 0.17,
                    amplitude: 0.0,
                    frequency_hz: 0.0,
                    phase_radians: 0.0,
                })
            })
            .collect::<Vec<_>>();
        assert!(prescribed.iter().any(Option::is_some));
        CanonicalForcing::from_prescribed(base, prescribed).unwrap()
    }

    fn filter_fluxes(base: &CanonicalWaveOperator, amplitude: f64) -> (Vec<f64>, Vec<Point2>) {
        let primary = base
            .node_points()
            .iter()
            .zip(base.primary_mass())
            .map(|(point, mass)| {
                // Grid-scale content on a smooth carrier, so the filter has
                // something to remove.
                let rough = if (point.x * 37.0 + point.y * 53.0).sin() > 0.0 {
                    0.3
                } else {
                    -0.3
                };
                amplitude * mass * ((1.4 * point.x - 0.9 * point.y).sin() + rough)
            })
            .collect::<Vec<_>>();
        let potential = base
            .node_points()
            .iter()
            .map(|point| 0.6 * amplitude * (0.8 * point.x + 1.2 * point.y).cos())
            .collect::<Vec<_>>();
        (primary, base.compatible_flux(&potential).unwrap())
    }

    /// Inert, the filter beside every composition is the fixed path's filter.
    #[test]
    fn an_inert_filter_beside_each_composition_is_the_fixed_filter() {
        for (label, scene, condition) in filter_compositions() {
            let operator = filter_operator(&scene, condition);
            let base = operator.base();
            assert!(operator.forced_composition_supported(), "{label}");
            // The prescribed wall is forcing handed to the step, not authored
            // data, so only its operator still reads as a closed bulk.
            assert_eq!(
                operator.conservative_bulk_supported(),
                label == "prescribed wall",
                "{label}"
            );
            let forcing = filter_forcing(label, base);
            let time_step = 0.4 * operator.maximum_time_step();
            let (primary, complementary) = filter_fluxes(base, 0.05);
            let mut temporal =
                CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary)
                    .unwrap()
                    .pinned(&operator, &forcing)
                    .unwrap();
            let mut fixed = CanonicalWaveState::new(
                base,
                time_step,
                temporal.primary_flux().to_vec(),
                temporal.complementary_flux().to_vec(),
            )
            .unwrap();
            for _ in 0..12 {
                temporal.step_with_forcing(&operator, &forcing).unwrap();
                fixed.step_with_forcing(base, &forcing).unwrap();
            }
            let removed = temporal
                .apply_grid_filter_with_forcing(&operator, &forcing, 0.8)
                .unwrap();
            let accounting = fixed.apply_grid_filter(base, &forcing, 0.8).unwrap();
            assert!(removed > 0.0, "{label}");
            assert!(
                (removed - accounting.filter_removed).abs() <= 1e-12 * removed.max(1e-12),
                "{label}: {removed:e} against {:e}",
                accounting.filter_removed
            );
            assert!(accounting.prescribed_exchange.abs() < 1e-14, "{label}");
            for (a, b) in temporal.primary_flux().iter().zip(fixed.primary_flux()) {
                assert!((a - b).abs() < 1e-12, "{label}");
            }
            for (a, b) in temporal
                .complementary_flux()
                .iter()
                .zip(fixed.complementary_flux())
            {
                assert!((*a - *b).norm() < 1e-12, "{label}");
            }
            assert!(
                (temporal.energy(&operator).unwrap() - fixed.energy(base).unwrap()).abs() < 1e-12,
                "{label}"
            );
        }
    }

    /// Driven and field-dependent media beside every composition: each filter
    /// commits, removes energy, leaves the pins, the pole currents and the gap
    /// jumps where they were, and keeps a free component's total.
    #[test]
    fn a_driven_or_nonlinear_filter_beside_each_composition_only_removes_energy() {
        type Author = fn(&mut Scene);
        let media: [(&str, Author); 5] = [
            ("pumped", |scene| {
                scene.materials[0].mass_law.drive = pump(0.2, 1.1, 0.3);
            }),
            ("sine-gordon", |scene| {
                scene.materials[0].restoring = sine_gordon(3.0);
            }),
            ("kerr sine-gordon", |scene| {
                scene.materials[0].restoring = sine_gordon(3.0);
                scene.materials[0].mass_law.field = kerr(0.8);
            }),
            ("kerr and saturable", |scene| {
                scene.materials[0].mass_law.field = kerr(0.8);
                scene.materials[0].stiffness_law.field = saturable_law(6.0, 0.3);
            }),
            ("pumped kerr", |scene| {
                scene.materials[0].mass_law.field = kerr(0.8);
                scene.materials[0].mass_law.drive = pump(0.2, 1.1, 0.3);
            }),
        ];
        for (label, scene, condition) in filter_compositions() {
            for (medium, author) in &media {
                let mut scene = scene.clone();
                author(&mut scene);
                let operator = filter_operator(&scene, condition);
                let base = operator.base();
                let forcing = filter_forcing(label, base);
                let (primary, complementary) = filter_fluxes(base, 0.5);
                let mut state = CanonicalTemporalWaveState::new(
                    &operator,
                    0.4 * operator.maximum_time_step(),
                    primary,
                    complementary,
                )
                .unwrap()
                .pinned(&operator, &forcing)
                .unwrap();
                let mut total_removed = 0.0;
                for step in 1..=40 {
                    state.step_with_forcing(&operator, &forcing).unwrap();
                    if step % 4 != 0 {
                        continue;
                    }
                    let pins = state.primary_flux().to_vec();
                    let gaps = state.thin_gap_jump().to_vec();
                    let poles = state.outgoing_pole_currents().to_vec();
                    let total = state.primary_flux().iter().sum::<f64>();
                    let before = state.energy(&operator).unwrap();
                    let removed = state
                        .apply_grid_filter_with_forcing(&operator, &forcing, 1.0)
                        .unwrap_or_else(|error| panic!("{label}, {medium}: {error:?}"));
                    let after = state.energy(&operator).unwrap();
                    assert!(removed >= 0.0, "{label}, {medium}");
                    assert!(
                        (before - after - removed).abs() <= 1e-12 * before.abs().max(1.0),
                        "{label}, {medium}"
                    );
                    total_removed += removed;
                    assert_eq!(state.thin_gap_jump(), gaps, "{label}, {medium}");
                    assert_eq!(state.outgoing_pole_currents(), poles, "{label}, {medium}");
                    for ((after, before), pinned) in state
                        .primary_flux()
                        .iter()
                        .zip(&pins)
                        .zip(forcing.prescribed())
                    {
                        if pinned.is_some() {
                            assert_eq!(after, before, "{label}, {medium}");
                        }
                    }
                    if forcing.prescribed().iter().all(Option::is_none) {
                        let scale = pins.iter().map(|value| value.abs()).sum::<f64>();
                        let moved = state.primary_flux().iter().sum::<f64>() - total;
                        assert!(moved.abs() <= 1e-13 * scale, "{label}, {medium}");
                    }
                }
                assert!(total_removed > 0.0, "{label}, {medium}");
            }
        }
    }

    /// The readout is the coefficient's own relative change: `χu²` for Kerr
    /// at a uniform field, and below `χσ²` for a saturable row however strong
    /// the field.
    #[test]
    fn nonlinear_strength_reads_each_rows_coefficient_change() {
        let mut scene = Scene::initial();
        scene.materials[0].mass_law.field = kerr(0.8);
        scene.materials[0].stiffness_law.field = saturable_law(6.0, 0.3);
        let operator = compile(&scene).unwrap();
        let runtime = operator.initial_runtime();
        let (terms, _) = operator.primary_terms_at(0.0, &runtime).unwrap();
        let field = 0.5;
        let primary = (0..operator.base().degrees_of_freedom())
            .map(|node| {
                operator
                    .primary_flux_and_energy_of_field(&terms, node, field, &runtime)
                    .unwrap()
                    .0
            })
            .collect::<Vec<_>>();
        let (_, complementary) = strong_fluxes(&operator, 40.0);
        let strengths = operator
            .nonlinear_strength(&primary, &complementary, 0.0, &runtime)
            .unwrap();
        assert_eq!(strengths.len(), 1);
        let strength = strengths[0];
        assert_eq!(strength.material, scene.materials[0].id);
        assert!(
            (strength.primary - 0.8 * field * field).abs() < 1e-12,
            "{strength:?}"
        );
        assert!(strength.complementary > 0.1, "{strength:?}");
        assert!(strength.complementary < 6.0 * 0.3 * 0.3, "{strength:?}");

        // A linear generation has nothing to report.
        let linear = compile(&Scene::initial()).unwrap();
        let (primary, complementary) = reference_fluxes(&linear);
        assert!(
            linear
                .nonlinear_strength(&primary, &complementary, 0.0, &runtime)
                .unwrap()
                .is_empty()
        );
    }

    /// A pump lowers its row's floor to its lowest factor, and the ceiling
    /// by its square root; a self-focusing law leaves the fixed ceiling.
    #[test]
    fn the_time_step_bound_names_the_row_that_lowers_it() {
        let mut pumped = Scene::initial();
        pumped.materials[0].mass_law.drive = pump(0.2, 1.1, 0.3);
        let bound = compile(&pumped).unwrap().time_step_bound();
        assert!((bound.primary_floor - 0.8).abs() < 1e-12, "{bound:?}");
        assert_eq!(bound.complementary_floor, 1.0);
        assert!((bound.trajectory - bound.fixed * 0.8_f64.sqrt()).abs() < 1e-15);

        let bound = compile(&kerr_scene()).unwrap().time_step_bound();
        assert_eq!(bound.primary_floor, 1.0);
        assert_eq!(bound.complementary_floor, 1.0);
        assert_eq!(bound.trajectory, bound.fixed);
    }

    /// A plane pulse launched along +x in a reflecting channel, stepped for
    /// `seconds`, and the share of its field energy found behind where it
    /// started: what a modulation sent backwards.
    fn backward_share(scene: &Scene, sign: f64, seconds: f64) -> f64 {
        let mut base_scene = scene.clone();
        strip_temporal_laws(&mut base_scene.materials);
        let mesh = mesh_scene(
            &base_scene,
            1,
            MeshingOptions {
                target_edge_length: 0.08,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &base_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let operator =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, scene, 1).unwrap();
        let base = operator.base();
        let (start, width) = (-0.35, 0.08);
        let pulse = |x: f64| (-((x - start) / width).powi(2)).exp();
        // `U = g(x − ct)` with `b = Cψ` and `ψ_t = U`: `ψ = −G/c`, `G′ = g`,
        // by a cumulative sum along x of the same Gaussian.
        let erf_like = |x: f64| {
            let steps = 400;
            let a = start - 6.0 * width;
            let h = (x - a) / steps as f64;
            if h <= 0.0 {
                return 0.0;
            }
            (0..steps)
                .map(|k| pulse(a + (k as f64 + 0.5) * h) * h)
                .sum::<f64>()
        };
        let primary = base
            .node_points()
            .iter()
            .zip(base.primary_mass())
            .map(|(point, mass)| mass * pulse(point.x))
            .collect::<Vec<_>>();
        let potential = base
            .node_points()
            .iter()
            .map(|point| -sign * erf_like(point.x))
            .collect::<Vec<_>>();
        let complementary = base.compatible_flux(&potential).unwrap();
        let time_step = 0.4 * operator.maximum_time_step();
        let mut state =
            CanonicalTemporalWaveState::new(&operator, time_step, primary, complementary).unwrap();
        for _ in 0..(seconds / time_step).round() as u64 {
            state.step(&operator).unwrap();
        }
        let field = operator
            .primary_field_at(state.primary_flux(), state.time(), state.runtime())
            .unwrap();
        let weight = |(point, (value, mass)): (&Point2, (&f64, &f64))| -> (bool, f64) {
            (point.x < start - 3.0 * width, mass * value * value)
        };
        let (mut behind, mut total) = (0.0, 0.0);
        for (is_behind, energy) in base
            .node_points()
            .iter()
            .zip(field.iter().zip(base.primary_mass()))
            .map(weight)
        {
            total += energy;
            if is_behind {
                behind += energy;
            }
        }
        behind / total
    }

    fn modulated(depth: f64, invert_stiffness: bool) -> Scene {
        let mut scene = Scene::default();
        scene.materials[0].mass_law.drive = pump(depth, 2.0, 0.0);
        scene.materials[0].stiffness_law.drive = pump(depth, 2.0, 0.0);
        scene.materials[0].stiffness_law.inverted = invert_stiffness;
        scene
    }

    /// The catalogue's falsifiable claim for the reflectionless time
    /// interface. Moving `m` and `K` together so that `√(mK)` holds is, in
    /// rescaled time `dτ = dt/h`, the unmodulated wave equation, so a pulse
    /// running forward sends nothing back. Holding the speed and moving the
    /// impedance - which is what the preset did while it inverted one row -
    /// sends a measurable share back.
    #[test]
    fn a_constant_impedance_modulation_sends_nothing_back() {
        let matched = backward_share(&modulated(0.4, false), 1.0, 0.5);
        let impedance_only = backward_share(&modulated(0.4, true), 1.0, 0.5);
        assert!(matched < 1.0e-3, "matched pair sent back {matched:.3e}");
        assert!(
            impedance_only > 0.02,
            "an impedance modulation sent back only {impedance_only:.3e}"
        );
    }

    /// A reflecting unit box of the default medium carrying one restoring law.
    fn restoring_operator(law: crate::RestoringLaw, edge: f64) -> CanonicalTemporalWaveOperator {
        let mut scene = Scene::default();
        scene.materials[0].restoring = law;
        let mut base_scene = scene.clone();
        strip_temporal_laws(&mut base_scene.materials);
        let mesh = mesh_scene(
            &base_scene,
            1,
            MeshingOptions {
                target_edge_length: edge,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &base_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap()
    }

    fn klein_gordon(omega0: f64) -> crate::RestoringLaw {
        crate::RestoringLaw::KleinGordon {
            omega0: ScalarField::constant(omega0),
        }
    }

    fn sine_gordon(omega0: f64) -> crate::RestoringLaw {
        crate::RestoringLaw::SineGordon {
            omega0: ScalarField::constant(omega0),
        }
    }

    /// The uniform mode carries no gradient, so it is a lone oscillator at
    /// `ω₀`, and velocity Verlet steps it exactly: `u_n = cos(nθ)` and
    /// `r_n = h sin(nθ)/sin θ` with `cos θ = 1 − (ω₀h)²/2`.
    #[test]
    fn a_klein_gordon_uniform_mode_is_the_verlet_oscillator_exactly() {
        let omega0 = 3.0;
        let operator = restoring_operator(klein_gordon(omega0), 0.4);
        assert!(operator.has_restoring());
        let base = operator.base();
        let h = 0.4 * operator.maximum_time_step();
        let primary = base.primary_mass().to_vec();
        let complementary = vec![Point2::default(); base.complementary_degrees_of_freedom()];
        let mut state =
            CanonicalTemporalWaveState::new(&operator, h, primary, complementary).unwrap();
        let theta = (1.0 - 0.5 * (omega0 * h).powi(2)).acos();
        for n in 1..=200 {
            state.step(&operator).unwrap();
            let u = state.primary_flux()[0] / base.primary_mass()[0];
            let r = state.integrated_field()[0];
            assert!(
                (u - (n as f64 * theta).cos()).abs() < 1.0e-11,
                "step {n}: u {u}"
            );
            assert!(
                (r - h * (n as f64 * theta).sin() / theta.sin()).abs() < 1.0e-11,
                "step {n}: r {r}"
            );
        }
    }

    /// A spatial mode's frequency moves by exactly the cutoff:
    /// `ω_KG² − ω_linear² = ω₀²`.
    #[test]
    fn klein_gordon_adds_its_cutoff_to_a_modes_frequency() {
        let omega0 = 2.5;
        let frequency = |law: crate::RestoringLaw| {
            let operator = restoring_operator(law, 0.25);
            let base = operator.base();
            let h = 0.3 * operator.maximum_time_step();
            // The box's lowest Neumann mode along x.
            let k = 0.5 * std::f64::consts::PI;
            let primary = base
                .node_points()
                .iter()
                .zip(base.primary_mass())
                .map(|(point, mass)| mass * (k * (point.x + 1.0)).cos())
                .collect::<Vec<_>>();
            let complementary = vec![Point2::default(); base.complementary_degrees_of_freedom()];
            let mut state =
                CanonicalTemporalWaveState::new(&operator, h, primary, complementary).unwrap();
            let node = base
                .node_points()
                .iter()
                .enumerate()
                .min_by(|a, b| (a.1.x + 1.0).abs().total_cmp(&(b.1.x + 1.0).abs()))
                .unwrap()
                .0;
            // Zero crossings of the wall node's field over several periods.
            let mut crossings = Vec::new();
            let mut previous = state.primary_flux()[node];
            while crossings.len() < 9 {
                state.step(&operator).unwrap();
                let current = state.primary_flux()[node];
                if previous.signum() != current.signum() {
                    let fraction = previous / (previous - current);
                    crossings.push(state.time() - h + fraction * h);
                }
                previous = current;
            }
            let period = 2.0 * (crossings[8] - crossings[0]) / 8.0;
            std::f64::consts::TAU / period
        };
        let linear = frequency(klein_gordon(0.0));
        let shifted = frequency(klein_gordon(omega0));
        let ratio = (shifted * shifted - linear * linear) / (omega0 * omega0);
        assert!((ratio - 1.0).abs() < 0.02, "ω² moved by {ratio:.4} ω₀²");
    }

    /// Both laws balance at second order, and a step and its inverse return
    /// the state, integrated field included.
    #[test]
    fn restoring_media_balance_at_second_order_and_reverse() {
        for law in [klein_gordon(2.0), sine_gordon(2.0)] {
            let operator = restoring_operator(law, 0.3);
            let forcing = CanonicalForcing::none(operator.base());
            nonlinear_balance_is_second_order(&operator, &forcing, 0.4, 0.5);

            let (primary, complementary) = reference_fluxes(&operator);
            let h = 0.4 * operator.maximum_time_step();
            let start =
                CanonicalTemporalWaveState::new(&operator, h, primary, complementary).unwrap();
            let mut state = start.clone();
            for _ in 0..40 {
                state.step_by(&operator, h).unwrap();
            }
            assert!(state.integrated_field().iter().any(|r| r.abs() > 1.0e-3));
            for _ in 0..40 {
                state.step_by(&operator, -h).unwrap();
            }
            for (a, b) in state
                .integrated_field()
                .iter()
                .zip(start.integrated_field())
            {
                assert!((a - b).abs() < 1.0e-10);
            }
            for (a, b) in state.primary_flux().iter().zip(start.primary_flux()) {
                assert!((a - b).abs() < 1.0e-10);
            }
        }
    }

    /// A sine-Gordon kink `4·atan(e^{γ(x − x₀)/ℓ})` across the box, with
    /// `ℓ = c/ω₀` and its velocity field, as the state it is.
    fn kink(
        operator: &CanonicalTemporalWaveOperator,
        length: f64,
        start: f64,
        speed: f64,
    ) -> CanonicalTemporalWaveState {
        let base = operator.base();
        let gamma = 1.0 / (1.0 - speed * speed).sqrt();
        let r = base
            .node_points()
            .iter()
            .map(|point| 4.0 * (gamma * (point.x - start) / length).exp().atan())
            .collect::<Vec<_>>();
        let primary = base
            .node_points()
            .iter()
            .zip(base.primary_mass())
            .map(|(point, mass)| {
                let s = gamma * (point.x - start) / length;
                mass * (-speed * 2.0 * gamma / length / s.cosh())
            })
            .collect::<Vec<_>>();
        let complementary = base.compatible_flux(&r).unwrap();
        CanonicalTemporalWaveState::new(
            operator,
            0.4 * operator.maximum_time_step(),
            primary,
            complementary,
        )
        .unwrap()
        .with_integrated_field(operator, r)
        .unwrap()
    }

    /// The kink's centre: across a 0→2π kink the box integrates
    /// `∫∫ r dA = 4π(1 − x_c)`, exact for a profile symmetric about it.
    fn kink_centre(
        operator: &CanonicalTemporalWaveOperator,
        state: &CanonicalTemporalWaveState,
    ) -> f64 {
        let integral = state
            .integrated_field()
            .iter()
            .zip(operator.base().primary_mass())
            .map(|(r, mass)| r * mass)
            .sum::<f64>();
        1.0 - integral / (4.0 * std::f64::consts::PI)
    }

    fn steepest(state: &CanonicalTemporalWaveState) -> f64 {
        state
            .complementary_flux()
            .iter()
            .map(|flux| flux.norm())
            .fold(0.0, f64::max)
    }

    /// A kink at rest stays at rest; one launched at `v` runs at `v`,
    /// contracted by `γ`.
    #[test]
    fn a_sine_gordon_kink_holds_still_or_runs_at_its_launched_speed() {
        let omega0 = 4.0;
        let length = 1.0 / omega0;
        let operator = restoring_operator(sine_gordon(omega0), 0.08);
        let run = |speed: f64, start: f64, seconds: f64| {
            let mut state = kink(&operator, length, start, speed);
            let before = (kink_centre(&operator, &state), steepest(&state));
            let energy = state.energy(&operator).unwrap();
            let steps = (seconds / state.time_step()).round() as usize;
            for _ in 0..steps {
                state.step(&operator).unwrap();
            }
            let drift = (state.energy(&operator).unwrap() - energy).abs() / energy;
            assert!(drift < 1.0e-3, "energy moved by {drift:.2e}");
            (
                before,
                (kink_centre(&operator, &state), steepest(&state)),
                state.time(),
            )
        };
        let ((centre, rest_slope), (after, _), _) = run(0.0, 0.0, 1.0);
        assert!(centre.abs() < 1.0e-6, "the kink started at {centre}");
        assert!(after.abs() < 0.01, "a kink at rest moved to {after}");

        // A reflecting wall mirrors a kink into an antikink, and the two
        // attract: a kink near one wall is slowed, near the other sped up.
        // Measured on a path symmetric between them the pulls cancel, and the
        // launched speed is recovered (0.4990 of 0.5 at every mesh tried;
        // from −0.4 to 0 it reads 0.478 whatever the mesh).
        let speed = 0.5;
        let ((start, moving_slope), (end, _), time) = run(speed, -0.3, 1.2);
        let measured = (end - start) / time;
        assert!((measured - speed).abs() < 0.01, "ran at {measured}");
        let contraction = moving_slope / rest_slope;
        let gamma = 1.0 / (1.0 - speed * speed).sqrt();
        assert!(
            (contraction - gamma).abs() < 0.05 * gamma,
            "contracted by {contraction}"
        );
    }

    fn phi4(lambda: f64, bound: f64) -> crate::RestoringLaw {
        crate::RestoringLaw::Phi4 {
            lambda: ScalarField::constant(lambda),
            amplitude_bound: ScalarField::constant(bound),
        }
    }

    fn resting_state(
        operator: &CanonicalTemporalWaveOperator,
        r: Vec<f64>,
    ) -> CanonicalTemporalWaveState {
        let base = operator.base();
        let complementary = base.compatible_flux(&r).unwrap();
        CanonicalTemporalWaveState::new(
            operator,
            0.4 * operator.maximum_time_step(),
            vec![0.0; base.degrees_of_freedom()],
            complementary,
        )
        .unwrap()
        .with_integrated_field(operator, r)
        .unwrap()
    }

    /// φ⁴'s wells hold still exactly, its wall `tanh(x/(√2ℓ))`, `ℓ = c/√λ`,
    /// holds still, and a field set just off the unstable top falls into the
    /// two wells once a loss lets it settle.
    #[test]
    fn phi4_holds_its_wells_and_its_wall_and_breaks_symmetry() {
        let lambda: f64 = 16.0;
        let width = 2.0_f64.sqrt() / lambda.sqrt();
        // The bound only has to clear what the field reaches, √2 from the
        // top; it sets the curvature, and with it the step.
        let operator = restoring_operator(phi4(lambda, 1.6), 0.1);
        let count = operator.base().degrees_of_freedom();
        for well in [1.0, -1.0] {
            let mut state = resting_state(&operator, vec![well; count]);
            for _ in 0..50 {
                state.step(&operator).unwrap();
            }
            assert!(
                state
                    .integrated_field()
                    .iter()
                    .all(|r| (r - well).abs() < 1.0e-12)
            );
        }

        let wall = operator
            .base()
            .node_points()
            .iter()
            .map(|point| (point.x / width).tanh())
            .collect::<Vec<_>>();
        let mut state = resting_state(&operator, wall.clone());
        let energy = state.energy(&operator).unwrap();
        let steps = (1.0 / state.time_step()).round() as usize;
        for _ in 0..steps {
            state.step(&operator).unwrap();
        }
        let moved = state
            .integrated_field()
            .iter()
            .zip(&wall)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
        assert!(moved < 0.02, "the wall moved by {moved}");
        let drift = (state.energy(&operator).unwrap() - energy).abs() / energy;
        assert!(drift < 1.0e-3, "energy moved by {drift:.2e}");

        // Just off the top, with a loss to settle into.
        let mut scene = Scene::default();
        scene.materials[0].restoring = phi4(lambda, 1.6);
        scene.materials[0].damping = ScalarField::constant(2.0);
        let mut base_scene = scene.clone();
        strip_temporal_laws(&mut base_scene.materials);
        let mesh = mesh_scene(
            &base_scene,
            1,
            MeshingOptions {
                target_edge_length: 0.15,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &base_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let lossy =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();
        let forcing = CanonicalForcing::none(lossy.base());
        let settle = |seed: &dyn Fn(Point2) -> f64| {
            let noise = lossy
                .base()
                .node_points()
                .iter()
                .map(|point| 1.0e-3 * seed(*point))
                .collect::<Vec<_>>();
            let mut state = resting_state(&lossy, noise);
            let steps = (6.0 / state.time_step()).round() as usize;
            for _ in 0..steps {
                state.step_with_forcing(&lossy, &forcing).unwrap();
            }
            state
        };
        // The top stores `λ/4` per unit area, 16 over the box.
        let barrier = 0.25 * lambda * 4.0;
        // A seed of one sign falls wholly into that well.
        let state = settle(&|point| 1.0 + 0.5 * (7.0 * point.x + 3.0 * point.y).sin());
        let left = state.energy(&lossy).unwrap() / barrier;
        assert!(
            state.integrated_field().iter().all(|r| *r > 0.9),
            "not all in the +1 well"
        );
        assert!(left < 1.0e-3, "kept {left:.2e} of the barrier");
        // An antisymmetric one settles into one wall along x = 0, whose
        // tension is `σ = (2√2/3) c √λ`: `2σ` over the box's height.
        let state = settle(&|point| (0.5 * std::f64::consts::PI * point.x).sin());
        let tension = 2.0 * 2.0_f64.sqrt() / 3.0 * lambda.sqrt();
        let ratio = state.energy(&lossy).unwrap() / (2.0 * tension);
        assert!(state.integrated_field().iter().any(|r| *r > 0.9));
        assert!(state.integrated_field().iter().any(|r| *r < -0.9));
        assert!(
            (ratio - 1.0).abs() < 0.05,
            "the wall holds {ratio:.3} of 2σ"
        );
    }

    /// The van der Pol node map is the exact flow of `Q̇ = −(β + kQ²)Q`,
    /// against a fine Runge-Kutta integration of the same equation.
    #[test]
    fn the_van_der_pol_node_map_is_its_ode_flow() {
        for (flux, beta, k) in [
            (0.3, -1.2, 4.0),
            (-0.8, -0.5, 2.0),
            (1.1, 0.7, 3.0),
            (0.4, 0.0, 5.0),
        ] {
            let duration = 0.37;
            let mut q: f64 = flux;
            let steps = 20_000;
            let h = duration / steps as f64;
            let f = |q: f64| -(beta + k * q * q) * q;
            for _ in 0..steps {
                let a = f(q);
                let b = f(q + 0.5 * h * a);
                let c = f(q + 0.5 * h * b);
                let d = f(q + h * c);
                q += h * (a + 2.0 * b + 2.0 * c + d) / 6.0;
            }
            let mapped = bernoulli_map(flux, beta, k, duration);
            assert!(
                (mapped - q).abs() < 1.0e-12,
                "{flux}, {beta}, {k}: {mapped} against {q}"
            );
        }
    }

    fn van_der_pol_operator(
        gain: f64,
        threshold: f64,
        omega0: f64,
    ) -> CanonicalTemporalWaveOperator {
        let mut scene = Scene::default();
        scene.materials[0].restoring = klein_gordon(omega0);
        scene.materials[0].magnetic_loss = Some(crate::LossChannel {
            base_rate: ScalarField::constant(gain),
            law: DampingLaw {
                rate: crate::RateLaw::VanDerPol {
                    threshold: ScalarField::constant(threshold),
                    amplitude_bound: ScalarField::constant(10.0),
                },
                drive: TimeDrive::None,
            },
        });
        let mut base_scene = scene.clone();
        strip_temporal_laws(&mut base_scene.materials);
        let mesh = mesh_scene(
            &base_scene,
            1,
            MeshingOptions {
                target_edge_length: 0.5,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &base_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap()
    }

    /// Beside Klein-Gordon the node equation is Rayleigh's form of van der
    /// Pol, `r̈ + γ₀(ṙ²/a² − 1)ṙ + ω₀²r = 0`: a small uniform field grows,
    /// and settles on the limit cycle whose rate amplitude is `2a/√3` for
    /// weak gain. The gain lane accounts for the energy it put in.
    #[test]
    fn van_der_pol_grows_to_its_limit_cycle() {
        let (gain, threshold, omega0) = (1.0, 0.5, 3.0);
        let operator = van_der_pol_operator(gain, threshold, omega0);
        let base = operator.base();
        let forcing = CanonicalForcing::none(base);
        let primary = base
            .primary_mass()
            .iter()
            .map(|mass| 1.0e-3 * mass)
            .collect();
        let complementary = vec![Point2::default(); base.complementary_degrees_of_freedom()];
        let mut state = CanonicalTemporalWaveState::new(
            &operator,
            0.2 * operator.maximum_time_step(),
            primary,
            complementary,
        )
        .unwrap();
        let start = state.energy(&operator).unwrap();
        let mut gained = 0.0;
        let mut peak: f64 = 0.0;
        let steps = (30.0 / state.time_step()).round() as usize;
        for step in 0..steps {
            let accounting = state.step_with_forcing(&operator, &forcing).unwrap();
            gained += accounting.active_gain;
            // The last few periods only.
            if step > steps - (4.0 / state.time_step()) as usize {
                peak = peak.max((state.primary_flux()[0] / base.primary_mass()[0]).abs());
            }
        }
        let predicted = 2.0 * threshold / 3.0_f64.sqrt();
        assert!(
            (peak - predicted).abs() < 0.05 * predicted,
            "limit cycle at {peak} against {predicted}"
        );
        let change = state.energy(&operator).unwrap() - start;
        assert!(
            (change - gained).abs() < 1.0e-3 * change.abs(),
            "the gain lane holds {gained} of {change}"
        );
    }

    /// Every oscillator medium beside every composition balances at second
    /// order: walls of both orders, prescribed data, a thin gap, loss and a
    /// source, with a pumped mass row and a Kerr row beside sine-Gordon.
    /// The restoring force enters the same kick as the gaps and the wall
    /// terms, and `r` drifts beside `b`, so nothing here is new code; this is
    /// what shows it composes.
    #[test]
    fn oscillator_media_balance_at_second_order_beside_each_composition() {
        type Author = fn(&mut Scene);
        let media: [(&str, Author); 5] = [
            ("klein-gordon", |scene| {
                scene.materials[0].restoring = klein_gordon(2.0);
            }),
            ("sine-gordon", |scene| {
                scene.materials[0].restoring = sine_gordon(3.0);
            }),
            ("pumped sine-gordon", |scene| {
                scene.materials[0].restoring = sine_gordon(3.0);
                scene.materials[0].mass_law.drive = pump(0.2, 1.1, 0.3);
            }),
            ("kerr sine-gordon", |scene| {
                scene.materials[0].restoring = sine_gordon(3.0);
                scene.materials[0].mass_law.field = kerr(0.8);
            }),
            ("van der pol", |scene| {
                scene.materials[0].restoring = klein_gordon(2.0);
                scene.materials[0].magnetic_loss = Some(LossChannel {
                    base_rate: ScalarField::constant(0.8),
                    law: DampingLaw {
                        rate: RateLaw::VanDerPol {
                            threshold: ScalarField::constant(0.4),
                            amplitude_bound: ScalarField::constant(10.0),
                        },
                        drive: TimeDrive::None,
                    },
                });
            }),
        ];
        let mut compositions = filter_compositions();
        compositions.push((
            "source",
            Scene::default(),
            OuterBoundaryCondition::Reflecting,
        ));
        for (label, scene, condition) in compositions {
            for (medium, author) in &media {
                // Van der Pol is the primary row's loss channel; the lossy
                // composition already holds that row's loss.
                if label == "loss" && *medium == "van der pol" {
                    continue;
                }
                let mut scene = scene.clone();
                author(&mut scene);
                let operator = filter_operator(&scene, condition);
                assert!(operator.has_restoring(), "{label}, {medium}");
                let base = operator.base();
                let mut forcing = filter_forcing(label, base);
                if label == "source" {
                    forcing
                        .push_source(
                            CanonicalSource::direct(
                                base,
                                base.primary_mass().to_vec(),
                                TimeSignal::harmonic(0.0, 0.9, 1.7, 0.4),
                            )
                            .unwrap(),
                        )
                        .unwrap();
                }
                let (total, _) = nonlinear_balance_is_second_order(&operator, &forcing, 0.3, 0.5);
                let exchanged = match label {
                    "first-order wall" | "second-order wall" => total.boundary_loss,
                    "prescribed wall" => total.prescribed_exchange.abs(),
                    "loss" => total.primary_loss,
                    "source" => total.source_work.abs(),
                    _ => 1.0,
                };
                assert!(exchanged > 1e-5, "{label}, {medium}: {total:?}");
                if *medium == "van der pol" {
                    assert!(total.active_gain.abs() > 1e-5, "{label}: {total:?}");
                }
            }
        }
    }

    /// A pulse is an increment of `u`, and `r = ∫u dt` is continuous in
    /// time, so a pulse leaves it where it was.
    #[test]
    fn a_pulse_leaves_the_integrated_field_where_it_was() {
        let operator = restoring_operator(sine_gordon(3.0), 0.3);
        let base = operator.base();
        let forcing = CanonicalForcing::none(base);
        let integrated = base
            .node_points()
            .iter()
            .map(|point| 1.2 * point.x)
            .collect::<Vec<_>>();
        let mut state =
            CanonicalTemporalWaveState::zero(&operator, 0.4 * operator.maximum_time_step())
                .unwrap()
                .with_integrated_field(&operator, integrated.clone())
                .unwrap();
        let increment = vec![0.5; base.degrees_of_freedom()];
        state
            .apply_primary_pulse(&operator, &forcing, &increment)
            .unwrap();
        assert_eq!(state.integrated_field(), integrated);
        for (flux, mass) in state.primary_flux().iter().zip(base.primary_mass()) {
            assert!((flux - 0.5 * mass).abs() < 1e-14);
        }
    }

    /// The largest gap between `b` and `ηC r`, which the step keeps without
    /// complementary loss and the filter now keeps too.
    fn integrated_mismatch(
        operator: &CanonicalTemporalWaveOperator,
        state: &CanonicalTemporalWaveState,
    ) -> f64 {
        operator
            .base()
            .compatible_flux(state.integrated_field())
            .unwrap()
            .iter()
            .zip(state.complementary_flux())
            .map(|(a, b)| (*a - *b).norm())
            .fold(0.0, f64::max)
    }

    /// A static kink and a φ⁴ wall balance `F(b) + R(r) = 0` with `F`
    /// nonzero, so the filter acts on the total force and moves `r` with
    /// `b`. Filtered at every cadence, each follows its unfiltered run, and
    /// `b = ηC r` holds through every event.
    ///
    /// A filter on `F` alone that left `r` where it was would part `b` from
    /// `ηC r` by 0.11 on this kink (1.4% of its steepest flux) within 5 s, and
    /// keep that offset; it also took 37% more energy over 20 s. The
    /// centre drifts the same with or without any filter: a kink midway
    /// between two reflecting walls is pulled equally by its two images, an
    /// unstable balance that grows about sixfold every 5 s.
    #[test]
    fn the_grid_filter_keeps_an_oscillator_equilibrium_and_b_equal_to_eta_c_r() {
        let omega0 = 4.0;
        let sine = restoring_operator(sine_gordon(omega0), 0.1);
        let lambda: f64 = 16.0;
        let width = 2.0_f64.sqrt() / lambda.sqrt();
        let quartic = restoring_operator(phi4(lambda, 1.6), 0.1);
        let wall = quartic
            .base()
            .node_points()
            .iter()
            .map(|point| (point.x / width).tanh())
            .collect::<Vec<_>>();
        let cases = [
            ("kink", &sine, kink(&sine, 1.0 / omega0, 0.0, 0.0)),
            ("wall", &quartic, resting_state(&quartic, wall)),
        ];
        for (label, operator, start) in cases {
            let energy = start.energy(operator).unwrap();
            let run = |strength: f64| {
                let mut state = start.clone();
                let mut removed = 0.0;
                let steps = (1.0 / state.time_step()).round() as usize;
                for step in 1..=steps {
                    state.step(operator).unwrap();
                    if step % crate::GRID_SCALE_FILTER_CADENCE as usize == 0 {
                        removed += state.apply_grid_filter(operator, strength).unwrap();
                        let mismatch = integrated_mismatch(operator, &state);
                        assert!(
                            mismatch < 1e-11 * steepest(&state),
                            "{label}, step {step}: {mismatch:e}"
                        );
                    }
                }
                (state, removed)
            };
            let (free, _) = run(0.0);
            let (filtered, removed) = run(1.0);
            assert!(removed > 0.0, "{label}");
            assert!(
                removed < 1e-4 * energy,
                "{label}: removed {removed:e} of {energy}"
            );
            let apart = filtered
                .integrated_field()
                .iter()
                .zip(free.integrated_field())
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f64::max);
            assert!(apart < 5e-3, "{label}: the filtered run is {apart:e} away");
            if label == "kink" {
                let shift = kink_centre(operator, &filtered) - kink_centre(operator, &free);
                assert!(shift.abs() < 1e-8, "the filter moved the kink by {shift:e}");
            }
        }
    }

    /// One generation of a scene: its mesh, its quadratic operator and its
    /// time-driven operator, for handoff tests that need all three.
    fn generation(
        scene: &Scene,
        edge: f64,
        revision: u64,
    ) -> (
        TriMesh,
        QuadraticWaveOperator,
        CanonicalTemporalWaveOperator,
    ) {
        let mut base_scene = scene.clone();
        strip_temporal_laws(&mut base_scene.materials);
        let mesh = mesh_scene(
            &base_scene,
            revision,
            MeshingOptions {
                target_edge_length: edge,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &base_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let operator =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, scene, revision)
                .unwrap();
        (mesh, quadratic, operator)
    }

    /// Hands a state to a new generation the way a topology transaction
    /// does: `Q` conservatively, `b` by its vector reconstruction, `r` by
    /// interpolation.
    fn hand_off(
        source: &(
            TriMesh,
            QuadraticWaveOperator,
            CanonicalTemporalWaveOperator,
        ),
        state: &CanonicalTemporalWaveState,
        target: &(
            TriMesh,
            QuadraticWaveOperator,
            CanonicalTemporalWaveOperator,
        ),
    ) -> (
        CanonicalTemporalWaveState,
        crate::CanonicalIntegratedFieldTransfer,
    ) {
        let same_mesh = std::ptr::eq(source, target);
        let interpolation = if same_mesh {
            crate::QuadraticTransferMap::identity_on_mesh(&source.0, &source.1, &target.1)
        } else {
            crate::QuadraticTransferMap::build(&source.0, &source.1, &target.0, &target.1)
        }
        .unwrap();
        let (source_base, target_base) = (source.2.base(), target.2.base());
        let primary =
            crate::CanonicalPrimaryTransferMap::prepare(&interpolation, source_base, target_base)
                .unwrap()
                .transfer(
                    state.primary_flux(),
                    &vec![None; target_base.component_count()],
                    &vec![false; target_base.degrees_of_freedom()],
                )
                .unwrap()
                .0;
        let complementary = crate::CanonicalVectorTransferMap::prepare(
            &source.0,
            source_base,
            &target.0,
            target_base,
        )
        .unwrap()
        .transfer(state.complementary_flux())
        .unwrap()
        .0;
        let transfer =
            crate::transfer_integrated_field(&interpolation, state.integrated_field(), &target.2)
                .unwrap();
        let mut handed = CanonicalTemporalWaveState::new_at(
            &target.2,
            state.time_step().min(0.4 * target.2.maximum_time_step()),
            primary,
            complementary,
            state.time(),
        )
        .unwrap();
        if !transfer.field.is_empty() {
            handed = handed
                .with_integrated_field(&target.2, transfer.field.clone())
                .unwrap();
        }
        (handed, transfer)
    }

    /// An identity handoff copies `r` exactly and the run goes on as if
    /// nothing had happened.
    #[test]
    fn an_identity_handoff_carries_the_integrated_field_exactly() {
        let mut scene = Scene::default();
        scene.materials[0].restoring = sine_gordon(3.0);
        let generation = generation(&scene, 0.3, 1);
        let operator = &generation.2;
        let forcing = CanonicalForcing::none(operator.base());
        let (primary, complementary) = reference_fluxes(operator);
        let mut state = CanonicalTemporalWaveState::new(
            operator,
            0.4 * operator.maximum_time_step(),
            primary,
            complementary,
        )
        .unwrap();
        for _ in 0..30 {
            state.step_with_forcing(operator, &forcing).unwrap();
        }
        let (mut handed, transfer) = hand_off(&generation, &state, &generation);
        assert_eq!(transfer.exposed_nodes, 0);
        assert!(!transfer.discarded && !transfer.started_at_zero);
        assert_eq!(handed.integrated_field(), state.integrated_field());
        assert_eq!(
            handed.energy(operator).unwrap(),
            state.energy(operator).unwrap()
        );
        for _ in 0..20 {
            state.step_with_forcing(operator, &forcing).unwrap();
            handed.step_with_forcing(operator, &forcing).unwrap();
        }
        assert_eq!(handed.integrated_field(), state.integrated_field());
        assert_eq!(handed.primary_flux(), state.primary_flux());
    }

    /// A generation without `r` hands a restoring target zero, which it
    /// reports; a target without a restoring law drops the source's `r`, and
    /// reports that.
    #[test]
    fn a_handoff_reports_an_integrated_field_started_or_dropped() {
        let linear = generation(&Scene::default(), 0.3, 1);
        let mut scene = Scene::default();
        scene.materials[0].restoring = klein_gordon(2.0);
        let oscillator = generation(&scene, 0.25, 2);
        let (primary, complementary) = reference_fluxes(&linear.2);
        let state = CanonicalTemporalWaveState::new(
            &linear.2,
            0.4 * oscillator.2.maximum_time_step(),
            primary,
            complementary,
        )
        .unwrap();
        let (handed, transfer) = hand_off(&linear, &state, &oscillator);
        assert!(transfer.started_at_zero && !transfer.discarded);
        assert!(handed.integrated_field().iter().all(|r| *r == 0.0));
        assert_eq!(
            handed.integrated_field().len(),
            oscillator.2.base().degrees_of_freedom()
        );

        let mut moved = handed.clone();
        for _ in 0..10 {
            moved.step(&oscillator.2).unwrap();
        }
        assert!(moved.integrated_field().iter().any(|r| *r != 0.0));
        let (back, transfer) = hand_off(&oscillator, &moved, &linear);
        assert!(transfer.discarded && !transfer.started_at_zero);
        assert!(transfer.field.is_empty() && back.integrated_field().is_empty());
    }

    /// Across a remesh a kink at rest stays at rest and one in flight keeps
    /// its speed: `r` is interpolated, `b` reconstructed and `Q` conserved,
    /// and the three agree to the new mesh's interpolation error.
    #[test]
    fn a_kink_crosses_a_remesh_at_rest_or_at_its_speed() {
        let omega0 = 4.0;
        let length = 1.0 / omega0;
        let mut scene = Scene::default();
        scene.materials[0].restoring = sine_gordon(omega0);
        let coarse = generation(&scene, 0.1, 1);
        let fine = generation(&scene, 0.07, 2);
        let seconds = |state: &CanonicalTemporalWaveState, time: f64| {
            (time / state.time_step()).round() as usize
        };
        // Both run 1.2 s, handed over at 0.4 s: the moving kink on the path
        // symmetric between the walls, as without a handoff.
        for speed in [0.0, 0.5] {
            let start = if speed == 0.0 { 0.0 } else { -0.3 };
            let mut state = kink(&coarse.2, length, start, speed);
            let centre = kink_centre(&coarse.2, &state);
            assert!(centre.abs() < 1e-6 || speed != 0.0);
            for _ in 0..seconds(&state, 0.4) {
                state.step(&coarse.2).unwrap();
            }
            let before = (
                kink_centre(&coarse.2, &state),
                state.energy(&coarse.2).unwrap(),
                state.time(),
            );
            let (mut handed, transfer) = hand_off(&coarse, &state, &fine);
            assert_eq!(transfer.exposed_nodes, 0);
            let after = (
                kink_centre(&fine.2, &handed),
                handed.energy(&fine.2).unwrap(),
            );
            let mismatch = integrated_mismatch(&fine.2, &handed) / steepest(&handed);
            for _ in 0..seconds(&handed, 0.8) {
                handed.step(&fine.2).unwrap();
            }
            let end = kink_centre(&fine.2, &handed);
            let measured = (end - centre) / handed.time();
            assert!(
                (after.0 - before.0).abs() < 1e-6,
                "the handoff moved the kink from {} to {}",
                before.0,
                after.0
            );
            let energy = (after.1 - before.1).abs() / before.1;
            assert!(
                energy < 1e-4,
                "the handoff moved the energy by {energy:.2e}"
            );
            // Interpolating `r` and reconstructing `b` err differently, and
            // the step keeps whatever offset they leave (3–4% of the
            // steepest flux here; the log has the comparison).
            assert!(mismatch < 0.1, "b and ηC r disagree by {mismatch:.2e}");
            assert!(
                (measured - speed).abs() < 0.01,
                "ran at {measured} after the handoff"
            );
        }
    }

    /// An oscillator state one step on, as the estimator is handed it.
    fn oscillator_snapshot(
        condition: OuterBoundaryCondition,
    ) -> (
        TriMesh,
        CanonicalTemporalWaveOperator,
        CanonicalTemporalWaveState,
        CanonicalIndicatorSnapshot,
    ) {
        let mut scene = Scene::default();
        scene.materials[0].restoring = sine_gordon(3.0);
        let mesh = mesh_scene(
            &scene,
            1,
            MeshingOptions {
                target_edge_length: 0.3,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let mut stripped = scene.clone();
        stripped.materials[0].restoring = crate::RestoringLaw::None;
        let quadratic = QuadraticWaveOperator::assemble_scene(&mesh, &stripped, condition).unwrap();
        let operator =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();
        let (primary, complementary) = reference_fluxes(&operator);
        let integrated = operator
            .base()
            .node_points()
            .iter()
            .map(|point| 1.2 * (0.7 * point.x + 0.5 * point.y).cos())
            .collect();
        let state = CanonicalTemporalWaveState::new(
            &operator,
            0.4 * operator.maximum_time_step(),
            primary,
            complementary,
        )
        .unwrap()
        .with_integrated_field(&operator, integrated)
        .unwrap();
        let mut next = state.clone();
        next.step(&operator).unwrap();
        let snapshot = CanonicalIndicatorSnapshot {
            mesh_revision: mesh.mesh_revision,
            primary_flux: next.primary_flux().to_vec(),
            previous_primary_flux: state.primary_flux().to_vec(),
            complementary_flux: next.complementary_flux().to_vec(),
            previous_complementary_flux: state.complementary_flux().to_vec(),
            auxiliary: next.outgoing_pole_currents().to_vec(),
            previous_auxiliary: state.outgoing_pole_currents().to_vec(),
            integrated_field: next.integrated_field().to_vec(),
            previous_integrated_field: state.integrated_field().to_vec(),
            time: next.time(),
            time_step: next.time_step(),
        };
        (mesh, operator, next, snapshot)
    }

    /// The estimate on an oscillator medium splits the solver's whole store,
    /// restoring potential included, and checks the outgoing trace's kick
    /// against the force it was stepped with: leave `R(r)` out and the trace
    /// residual is the restoring force itself.
    #[test]
    fn the_oscillator_estimate_splits_the_store_and_charges_the_trace_its_own_force() {
        let (mesh, operator, state, snapshot) =
            oscillator_snapshot(OuterBoundaryCondition::Reflecting);
        assert!(operator.indicator_supplement_supported());
        let forcing = CanonicalForcing::none(operator.base());
        let runtime = operator.initial_runtime();
        let estimate = canonical_temporal_indicator_supplement(
            &mesh, &operator, &forcing, &snapshot, &runtime, 1.0,
        )
        .unwrap();
        let stored = state.energy(&operator).unwrap();
        let split = estimate.element_energy.iter().sum::<f64>();
        assert!(
            (split - stored).abs() <= 1e-12 * stored,
            "{split} against {stored}"
        );

        // A snapshot that leaves out `r`, or holds the wrong length, is
        // refused rather than estimated without the store.
        let mut short = snapshot.clone();
        short.integrated_field.clear();
        short.previous_integrated_field.clear();
        assert!(
            canonical_temporal_indicator_supplement(
                &mesh, &operator, &forcing, &short, &runtime, 1.0
            )
            .is_err()
        );

        let (mesh, operator, _, snapshot) =
            oscillator_snapshot(OuterBoundaryCondition::SecondOrderOutgoing);
        let forcing = CanonicalForcing::none(operator.base());
        let consistent = canonical_temporal_indicator_supplement(
            &mesh, &operator, &forcing, &snapshot, &runtime, 1.0,
        )
        .unwrap()
        .outgoing_contribution;
        let mut blind = snapshot.clone();
        blind.integrated_field.iter_mut().for_each(|r| *r = 0.0);
        blind
            .previous_integrated_field
            .iter_mut()
            .for_each(|r| *r = 0.0);
        let without = canonical_temporal_indicator_supplement(
            &mesh, &operator, &forcing, &blind, &runtime, 1.0,
        )
        .unwrap()
        .outgoing_contribution;
        // Measured 2.7e-7 against 6.9e-4.
        assert!(
            without > 500.0 * consistent,
            "trace residual {consistent:e} with the force, {without:e} without"
        );
    }

    /// Van der Pol is a loss channel, and the estimate still refuses loss.
    #[test]
    fn the_estimate_still_refuses_van_der_pol() {
        assert!(!van_der_pol_operator(1.0, 0.5, 3.0).indicator_supplement_supported());
    }
}
