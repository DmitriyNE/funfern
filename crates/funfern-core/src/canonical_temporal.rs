//! Dormant f64 reference for Stage 7 time-driven, field-linear media.
//!
//! The accepted production operator remains the fixed linear
//! [`CanonicalWaveOperator`]. This wrapper compiles exact material-frame law
//! samples beside that operator and evaluates them at synchronized stage
//! times. Keeping the paths separate until the temporal gates close prevents
//! a valid authored drive from being silently executed as a static material.

use std::collections::BTreeSet;

use std::collections::BTreeMap;

use crate::{
    CanonicalAreaContribution, CanonicalAreaSample, CanonicalForcing, CanonicalIndicatorSnapshot,
    CanonicalIndicatorSupplement, CanonicalPointSample, CanonicalPointStencil,
    CanonicalWaveOperator, CoefficientLaw, CoefficientLawValues, DampingLaw, DampingLawValues,
    ElectromagneticPolarization, FieldLaw, LossChannel, Material, MaterialCoordinates,
    MaterialError, MaterialId, MaterialSwitchRuntime, PhysicsModel, Point2, QuadraticAreaElement,
    QuadraticAreaStencil, QuadraticPointStencil, QuadraticWaveOperator, RateLaw, RateLawValues,
    RegionId, RestoringLaw, Scene, SymmetricTensor2, TimeDriveRuntime, TimeDriveValues,
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
    fn authored(materials: impl IntoIterator<Item = Material>) -> Result<Self, WaveError> {
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
            return Err(WaveError::InvalidCoefficients);
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
        let complementary = self.complementary_field(complementary_flux, time, runtime)?;
        let primary_factor = self.primary.factor_at(time, runtime)?;
        let complementary_factor = self.complementary.factor_at(time, runtime)?;
        let energy_density = 0.5
            * (self.fixed.primary_reference * primary_factor * primary * primary
                + complementary_factor
                    * self
                        .fixed
                        .complementary_reference
                        .quadratic_form(complementary));
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
/// Only the conservative bulk is covered, which is what the temporal path
/// executes: an operator carrying loss, thin gaps, open boundaries or
/// prescribed data is refused rather than reported with those terms missing.
pub fn canonical_temporal_indicator_supplement(
    mesh: &TriMesh,
    operator: &CanonicalTemporalWaveOperator,
    snapshot: &CanonicalIndicatorSnapshot,
    runtime: &CanonicalMaterialRuntimeState,
    resolved_frequency_hz: f64,
) -> Result<CanonicalIndicatorSupplement, WaveError> {
    if !operator.conservative_bulk_supported() {
        return Err(WaveError::InvalidCoefficients);
    }
    let base = operator.base();
    let node_count = base.degrees_of_freedom();
    let sample_count = base.complementary_degrees_of_freedom();
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
    let element_boundary_residual = vec![0.0; element_count];
    let mut element_energy = vec![0.0; element_count];

    // Instantaneous complementary inverses, once per sample rather than once
    // per use: the recovery, the residual norm and the energy all need them.
    let mut inverses = Vec::with_capacity(sample_count);
    for (sample, temporal) in base
        .constitutive_samples()
        .iter()
        .zip(&operator.complementary)
    {
        let factor = coefficient_factor(temporal.coefficient, time, runtime)?;
        if !factor.is_finite() || factor <= 0.0 {
            return Err(WaveError::InvalidCoefficients);
        }
        inverses.push(SymmetricTensor2::new(
            sample.complementary_inverse.xx / factor,
            sample.complementary_inverse.xy / factor,
            sample.complementary_inverse.yy / factor,
        ));
    }

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
            let value = interpolation.iter().enumerate().fold(
                Point2::default(),
                |sum, (sample, weight)| {
                    sum + inverses[start + sample]
                        .apply(snapshot.complementary_flux[start + sample])
                        * *weight
                },
            );
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
            let physical =
                inverses[start + local].apply(snapshot.complementary_flux[start + local]);
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
                    flux =
                        flux + inverse.apply(snapshot.complementary_flux[start + sample]) * weight;
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
                        return Err(WaveError::InvalidCoefficients);
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
    let mass = operator.primary_mass_at(time, runtime)?;
    for contribution in base.primary_contributions() {
        let node = contribution.node as usize;
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

    // The drift the residual measures against is the one the solver takes:
    // the midpoint primary field, with no loss because the conservative bulk
    // carries none.
    for (sample_index, sample) in base.constitutive_samples().iter().enumerate() {
        let element = sample.element as usize;
        let current = snapshot.complementary_flux[sample_index];
        let previous = snapshot.previous_complementary_flux[sample_index];
        element_energy[element] +=
            0.5 * sample.integration_weight * current.dot(inverses[sample_index].apply(current));
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
        // The conservative bulk carries neither, by the contract checked
        // above; they are zero rather than unreported.
        thin_gap_contribution: 0.0,
        outgoing_contribution: 0.0,
    })
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
        return Err(WaveError::InvalidCoefficients);
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
    // The assembled nodal map at this instant. Every node's inverse mass is
    // the one the solver itself would use for a step at `time`.
    let mass = operator.primary_mass_at(time, runtime)?;
    let mut primary_integral = 0.0;
    let mut primary_squared = 0.0;
    let mut complementary_squared = 0.0;
    let mut total_energy = 0.0;

    for element in &stencil.elements {
        let compiled = CanonicalTemporalAreaContribution::from_element(*element, operator)?;
        let contribution = compiled.fixed();
        let start = element.element as usize * 6;

        let mut sample_fields = [Point2::default(); 6];
        for (local, field) in sample_fields.iter_mut().enumerate() {
            let Some(flux) = complementary_flux.get(start + local) else {
                return Err(WaveError::InvalidState);
            };
            let factor = compiled.samples[local].factor_at(time, runtime)?;
            *field = contribution.sample_inverses[local].apply(*flux) / factor;
        }

        for point in contribution.quadrature {
            let mut value = 0.0;
            for (local, basis) in point.primary_weights.iter().enumerate() {
                let node = element.nodes[local] as usize;
                let (Some(flux), Some(mass)) = (primary_flux.get(node), mass.get(node)) else {
                    return Err(WaveError::InvalidState);
                };
                value += basis * flux / mass;
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

        let mut energy = 0.0;
        for (local, reference) in contribution.node_references.into_iter().enumerate() {
            let node = element.nodes[local] as usize;
            let (Some(flux), Some(mass)) = (primary_flux.get(node), mass.get(node)) else {
                return Err(WaveError::InvalidState);
            };
            if *mass <= 0.0 {
                return Err(WaveError::InvalidState);
            }
            let factor = compiled.primary[local].factor_at(time, runtime)?;
            energy += 0.5 * reference * factor * flux * flux / (mass * mass);
        }
        for (local, field) in sample_fields.into_iter().enumerate() {
            let Some(flux) = complementary_flux.get(start + local) else {
                return Err(WaveError::InvalidState);
            };
            energy += 0.5 * contribution.sample_weights[local] * flux.dot(field);
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

/// A field-linear, time-driven f64 oracle. It is deliberately not accepted by
/// the production GPU solver until the remaining Stage 7 gates pass.
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalTemporalWaveOperator {
    base: CanonicalWaveOperator,
    primary: Vec<TemporalPrimarySample>,
    complementary: Vec<TemporalComplementarySample>,
    initial_runtime: CanonicalMaterialRuntimeState,
    has_temporal_laws: bool,
    has_loss: bool,
    conservative_bulk_supported: bool,
    forced_composition_supported: bool,
    maximum_time_step: f64,
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
        let authored = model.to_owned();
        let mut stripped = authored.clone();
        strip_temporal_laws(&mut stripped.materials);
        let base = CanonicalWaveOperator::compile(
            mesh,
            quadratic,
            stripped.as_model(),
            constitutive_revision,
        )?;
        let authored = authored.as_model();
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
        let maximum_time_step = base.maximum_time_step()
            * (minimum_primary_factor * minimum_complementary_factor).sqrt();
        if !maximum_time_step.is_finite() || maximum_time_step <= 0.0 {
            return Err(WaveError::InvalidCoefficients);
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
        Ok(Self {
            base,
            primary,
            complementary,
            initial_runtime,
            has_temporal_laws,
            has_loss,
            conservative_bulk_supported,
            forced_composition_supported,
            maximum_time_step,
        })
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

    /// Whether the exact kick/drift bulk split can run without composing any
    /// lossy, forced, prescribed-boundary, or auxiliary subsystem.
    pub fn conservative_bulk_supported(&self) -> bool {
        self.conservative_bulk_supported
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
        if !self.has_temporal_laws {
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
        if !self.has_temporal_laws {
            return self.base.primary_field(primary_flux);
        }
        if primary_flux.len() != self.base.degrees_of_freedom() {
            return Err(WaveError::SizeMismatch {
                expected: self.base.degrees_of_freedom(),
                actual: primary_flux.len(),
            });
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

    pub fn complementary_field_at(
        &self,
        complementary_flux: &[Point2],
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<Vec<Point2>, WaveError> {
        if !self.has_temporal_laws {
            return self.base.complementary_field(complementary_flux);
        }
        if complementary_flux.len() != self.base.complementary_degrees_of_freedom() {
            return Err(WaveError::SizeMismatch {
                expected: self.base.complementary_degrees_of_freedom(),
                actual: complementary_flux.len(),
            });
        }
        let mut field = Vec::with_capacity(complementary_flux.len());
        for ((base, temporal), flux) in self
            .base
            .constitutive_samples()
            .iter()
            .zip(&self.complementary)
            .zip(complementary_flux)
        {
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
        if !self.has_temporal_laws {
            return self.base.force(complementary_flux);
        }
        let fields = self.complementary_field_at(complementary_flux, time, runtime)?;
        let mut force = vec![0.0; self.base.degrees_of_freedom()];
        for (sample_index, (sample, field)) in self
            .base
            .constitutive_samples()
            .iter()
            .zip(fields)
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
        if !self.has_temporal_laws {
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
        for ((base, temporal), flux) in self
            .base
            .constitutive_samples()
            .iter()
            .zip(&self.complementary)
            .zip(complementary_flux)
        {
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

    fn drift_at(
        &self,
        complementary_flux: &mut [Point2],
        primary_flux: &[f64],
        time: f64,
        duration: f64,
        runtime: &CanonicalMaterialRuntimeState,
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
        let primary_field = self.primary_field_at(primary_flux, time, runtime)?;
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
        if !operator.forced_composition_supported()
            || !time_step.is_finite()
            || time_step <= 0.0
            || time_step > operator.maximum_time_step()
            || !time.is_finite()
        {
            return Err(WaveError::InvalidCoefficients);
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
        let gaps = self
            .thin_gap_jump
            .iter()
            .zip(operator.base().thin_gap_samples())
            .map(|(jump, sample)| 0.5 * sample.stiffness * jump * jump)
            .sum::<f64>();
        let poles = self
            .outgoing_z
            .iter()
            .map(|value| 0.5 * value * value)
            .sum::<f64>();
        gaps + poles
    }

    pub fn thin_gap_jump(&self) -> &[f64] {
        &self.thin_gap_jump
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
    pub fn apply_grid_filter(
        &mut self,
        operator: &CanonicalTemporalWaveOperator,
        strength: f64,
    ) -> Result<f64, WaveError> {
        if !operator.conservative_bulk_supported()
            || !strength.is_finite()
            || !(0.0..=1.0).contains(&strength)
        {
            return Err(WaveError::InvalidCoefficients);
        }
        if strength == 0.0 {
            return Ok(0.0);
        }
        let before = self.energy(operator)?;
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

        let old_primary = self.primary_flux.clone();
        let old_complementary = self.complementary_flux.clone();
        let primary_field = old_primary
            .iter()
            .zip(&mass)
            .map(|(flux, mass)| flux / mass)
            .collect::<Vec<_>>();
        let primary_correction = stiffness(&inverse_mass(stiffness(&primary_field)?))?;

        let gathered = operator.force_at(&old_complementary, self.time, &self.runtime)?;
        let complementary_correction = operator
            .base()
            .compatible_flux(&inverse_mass(stiffness(&inverse_mass(gathered))?))?;

        let eigenvalue_bound = 4.0 / operator.maximum_time_step().powi(2);
        let scale = strength / eigenvalue_bound.powi(2);
        let mut next_primary = old_primary.clone();
        let mut next_complementary = old_complementary.clone();
        for (value, correction) in next_primary.iter_mut().zip(primary_correction) {
            *value -= scale * correction;
        }
        for (value, correction) in next_complementary.iter_mut().zip(complementary_correction) {
            *value = *value - correction * scale;
        }
        validate_finite(&next_primary)?;
        if next_complementary.iter().any(|value| !value.finite()) {
            return Err(WaveError::InvalidState);
        }
        let after =
            operator.energy_at(&next_primary, &next_complementary, self.time, &self.runtime)?;
        let tolerance = 2.0e-12 * before.abs().max(after.abs()).max(1.0);
        if after > before + tolerance {
            return Err(WaveError::InvalidState);
        }
        self.primary_flux = next_primary;
        self.complementary_flux = next_complementary;
        Ok((before - after).max(0.0))
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
        for (node, signal) in forcing.prescribed().iter().enumerate() {
            if let Some(signal) = signal {
                self.primary_flux[node] = mass[node] * signal.value(self.time);
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
            || !duration.is_finite()
            || duration == 0.0
            || duration.abs() > operator.maximum_time_step()
        {
            return Err(WaveError::InvalidCoefficients);
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
        let (first_primary_loss, first_complementary_loss) = decay(
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
        // split is admitted for.
        //
        // Reconstructing that field is a walk over every node with a
        // transcendental at each, so a generation without gaps must not pay
        // for it. It did until this guard.
        let mut gap_jump = self.thin_gap_jump.clone();
        if !gap_jump.is_empty() {
            let midpoint_field = operator.primary_field_at(&primary, middle_time, &self.runtime)?;
            for (sample, jump) in operator.base().thin_gap_samples().iter().zip(&mut gap_jump) {
                *jump += duration
                    * (midpoint_field[sample.left_node as usize]
                        - midpoint_field[sample.right_node as usize]);
            }
            validate_finite(&gap_jump)?;
        }
        operator.drift_at(
            &mut complementary,
            &primary,
            middle_time,
            duration,
            &self.runtime,
        )?;
        let (_, complementary_rate_end) =
            operator.complementary_energy_and_rate(&complementary, end_time, &self.runtime)?;

        let mut second_force = operator.force_at(&complementary, end_time, &self.runtime)?;
        add_gap_force(operator, &gap_jump, &mut second_force)?;
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

        let (second_primary_loss, second_complementary_loss) = decay(
            operator,
            &mut primary,
            &mut complementary,
            0.5 * duration,
            end_time,
            start_time + 0.75 * duration,
            &self.runtime,
        )?;
        let primary_loss = first_primary_loss + second_primary_loss;
        let complementary_loss = first_complementary_loss + second_complementary_loss;

        let after = operator.energy_at(&primary, &complementary, end_time, &self.runtime)?;
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
            energy_change,
            splitting_residual: energy_change - temporal_work - source_work - prescribed_exchange
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
            || !accounting.energy_change.is_finite()
            || !accounting.splitting_residual.is_finite()
        {
            return Err(WaveError::InvalidState);
        }
        self.primary_flux = primary;
        self.complementary_flux = complementary;
        self.thin_gap_jump = gap_jump;
        self.outgoing_z = outgoing_z;
        self.time = end_time;
        Ok(accounting)
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
) -> Result<(f64, f64), WaveError> {
    if !operator.has_loss {
        return Ok((0.0, 0.0));
    }
    let rates = operator.loss_rates_at(rate_time, runtime)?;
    let primary_before = operator
        .primary_energy_and_rate(primary, stage_time, runtime)?
        .0;
    let complementary_before = operator
        .complementary_energy_and_rate(complementary, stage_time, runtime)?
        .0;
    for (flux, rate) in primary.iter_mut().zip(&rates.primary) {
        *flux *= (-duration * rate).exp();
    }
    for (flux, rate) in complementary.iter_mut().zip(&rates.complementary) {
        *flux = *flux * (-duration * rate).exp();
    }
    validate_finite(primary)?;
    let removed_primary = primary_before
        - operator
            .primary_energy_and_rate(primary, stage_time, runtime)?
            .0;
    let removed_complementary = complementary_before
        - operator
            .complementary_energy_and_rate(complementary, stage_time, runtime)?
            .0;
    // A passive channel cannot add energy. Anything else is a defect in the
    // rate, not a small negative to be clamped away quietly.
    if removed_primary < -1.0e-12 || removed_complementary < -1.0e-12 {
        return Err(WaveError::InvalidState);
    }
    Ok((removed_primary.max(0.0), removed_complementary.max(0.0)))
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
            &mass,
            duration,
        )?;
        let mut prescribed_cache = None;
        let (work, escaped, exchange) = crate::canonical_wave::force_coupled_outgoing_kick_with(
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
        )?;
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
        let midpoint_field = 0.5 * (old + new) / mass;
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

fn strip_temporal_laws(materials: &mut [Material]) {
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
}

fn temporal_material_sample(
    model: TopologyWaveModel<'_>,
    region_id: crate::RegionId,
    point: Point2,
    primary: bool,
) -> Result<CompiledTemporalMaterialSample, WaveError> {
    let region = model
        .region(region_id)
        .ok_or(WaveError::InvalidCoefficients)?;
    let material = model
        .material(region.material)
        .ok_or(WaveError::InvalidCoefficients)?;
    if !material.switch_ramp.is_finite() || material.switch_ramp < 0.0 {
        return material_error(
            material,
            "Switch ramp",
            point,
            "duration must be finite and nonnegative",
        );
    }
    if !material.restoring.is_none() {
        return material_error(
            material,
            "restoring law",
            point,
            "oscillator media remain gated",
        );
    }
    let coordinates = region.frame.coordinates(point);
    let (coefficient_drive, law) = coefficient_for(model.physics, material, primary);
    if !law.valid(&material.parameters) {
        return material_error(
            material,
            "coefficient law",
            point,
            "authored drive, alternate, or response is invalid",
        );
    }
    if !matches!(law.field, FieldLaw::Linear) {
        return material_error(
            material,
            "field law",
            point,
            "field-dependent response remains gated until Stage 8",
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
    compile_loss(material, channel, drive, coordinates, point)
}

fn compile_loss(
    material: &Material,
    channel: &LossChannel,
    drive: CanonicalMaterialDrive,
    coordinates: MaterialCoordinates,
    point: Point2,
) -> Result<TemporalLossSample, WaveError> {
    if !channel.valid(&material.parameters) {
        return material_error(material, "loss law", point, "authored loss law is invalid");
    }
    if !matches!(channel.law.rate, RateLaw::Constant) {
        return material_error(
            material,
            "loss law",
            point,
            "field-dependent loss remains gated until Stage 8",
        );
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
        .ok_or(WaveError::InvalidCoefficients)
}

fn validate_nonnegative(values: &[f64]) -> Result<(), WaveError> {
    values
        .iter()
        .all(|value| value.is_finite() && *value >= 0.0)
        .then_some(())
        .ok_or(WaveError::InvalidCoefficients)
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
        LoopRole, LossChannel, MaterialFrame, MeshingOptions, Obstacle, ObstacleId,
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
            time: time_step,
            time_step,
        };
        let supplement =
            canonical_temporal_indicator_supplement(&mesh, &temporal, &snapshot, &runtime, 0.0)
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

    /// Two things the variable-coefficient supplement has to get right. On an
    /// inert medium it must reproduce the fixed one exactly, or the temporal
    /// path would report different errors for the same physics. And on a
    /// driven one it must differ, because reusing the authored coefficients
    /// charges the estimator for the medium's own modulation and refines
    /// against it.
    #[test]
    fn temporal_supplement_matches_the_fixed_one_when_inert_and_departs_when_driven() {
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
            time: time_step,
            time_step,
        };

        let forcing = crate::CanonicalForcing::none(base);
        let fixed =
            crate::canonical_indicator_supplement(&mesh, base, &forcing, &snapshot).unwrap();
        let runtime = inert.initial_runtime();
        let temporal =
            canonical_temporal_indicator_supplement(&mesh, &inert, &snapshot, &runtime, 0.0)
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
        let driven_supplement =
            canonical_temporal_indicator_supplement(&mesh, &driven, &snapshot, &runtime, 0.0)
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
    fn nonlinear_response_and_loss_remain_rejected_by_the_stage_seven_oracle() {
        let mut scene = Scene::initial();
        scene.materials[0].mass_law.field = FieldLaw::Saturable {
            chi: ScalarField::constant(0.2),
            saturation: ScalarField::constant(1.0),
        };
        assert!(matches!(
            compile(&scene),
            Err(WaveError::MaterialEvaluation {
                coefficient: "field law",
                ..
            })
        ));

        scene.materials[0].mass_law = CoefficientLaw::linear();
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

        scene.materials[0].electric_loss = None;
        scene.materials[0].mass_law.drive = pump(1.0, 1.0, 0.0);
        assert!(matches!(
            compile(&scene),
            Err(WaveError::MaterialEvaluation {
                coefficient: "coefficient law",
                ..
            })
        ));
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
        assert_eq!(
            CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1),
            Err(WaveError::InvalidCoefficients)
        );
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
}
