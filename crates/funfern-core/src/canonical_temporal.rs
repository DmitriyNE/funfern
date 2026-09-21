//! Dormant f64 reference for Stage 7 time-driven, field-linear media.
//!
//! The accepted production operator remains the fixed linear
//! [`CanonicalWaveOperator`]. This wrapper compiles exact material-frame law
//! samples beside that operator and evaluates them at synchronized stage
//! times. Keeping the paths separate until the temporal gates close prevents
//! a valid authored drive from being silently executed as a static material.

use std::collections::BTreeSet;

use crate::{
    CanonicalPointSample, CanonicalPointStencil, CanonicalWaveOperator, CoefficientLaw,
    CoefficientLawValues, DampingLaw, DampingLawValues, ElectromagneticPolarization, FieldLaw,
    LossChannel, Material, MaterialCoordinates, MaterialError, MaterialId, MaterialSwitchRuntime,
    PhysicsModel, Point2, QuadraticPointStencil, QuadraticWaveOperator, RateLaw, RateLawValues,
    RestoringLaw, Scene, TimeDriveRuntime, TimeDriveValues, TopologyWaveModel, TriMesh, WaveError,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanonicalMaterialDrive {
    MassCoefficient,
    StiffnessCoefficient,
    ElectricLoss,
    MagneticLoss,
}

impl CanonicalMaterialDrive {
    const COUNT: usize = 4;

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
        let mut flux = Point2::default();
        for local in 0..self.fixed.complementary_samples.len() {
            let sample = self.fixed.complementary_samples[local] as usize;
            let Some(value) = complementary_flux.get(sample) else {
                return Err(WaveError::InvalidState);
            };
            flux = flux + *value * self.fixed.complementary_weights[local];
        }
        let primary_factor = self.primary.factor_at(time, runtime)?;
        let complementary_factor = self.complementary.factor_at(time, runtime)?;
        let complementary = self.fixed.complementary_inverse.apply(flux) / complementary_factor;
        let energy_density = 0.5
            * (self.fixed.primary_reference * primary_factor * primary * primary
                + flux.dot(complementary));
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
        let conservative_bulk_supported = !has_loss
            && base.thin_gap_samples().is_empty()
            && base
                .first_order_boundary_damping()
                .iter()
                .all(|value| *value == 0.0)
            && base.outgoing_boundary().is_none()
            && quadratic.dirichlet_signals().iter().all(Option::is_none)
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
        Ok(Self {
            base,
            primary,
            complementary,
            initial_runtime,
            has_temporal_laws,
            has_loss,
            conservative_bulk_supported,
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
    pub temporal_work: f64,
    pub energy_change: f64,
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
        if !operator.conservative_bulk_supported()
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

    pub fn energy(&self, operator: &CanonicalTemporalWaveOperator) -> Result<f64, WaveError> {
        operator.energy_at(
            &self.primary_flux,
            &self.complementary_flux,
            self.time,
            &self.runtime,
        )
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

    /// Signed stepping is exposed for the reversibility gate. Production uses
    /// `step`; a negative duration applies the exact inverse composition.
    pub fn step_by(
        &mut self,
        operator: &CanonicalTemporalWaveOperator,
        duration: f64,
    ) -> Result<CanonicalTemporalStepAccounting, WaveError> {
        if !operator.conservative_bulk_supported()
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
        let first_force = operator.force_at(&complementary, start_time, &self.runtime)?;
        for (flux, force) in primary.iter_mut().zip(first_force) {
            *flux -= 0.5 * duration * force;
        }
        validate_finite(&primary)?;

        let (_, primary_rate_middle) =
            operator.primary_energy_and_rate(&primary, middle_time, &self.runtime)?;
        operator.drift_at(
            &mut complementary,
            &primary,
            middle_time,
            duration,
            &self.runtime,
        )?;
        let (_, complementary_rate_end) =
            operator.complementary_energy_and_rate(&complementary, end_time, &self.runtime)?;

        let second_force = operator.force_at(&complementary, end_time, &self.runtime)?;
        for (flux, force) in primary.iter_mut().zip(second_force) {
            *flux -= 0.5 * duration * force;
        }
        validate_finite(&primary)?;
        let after = operator.energy_at(&primary, &complementary, end_time, &self.runtime)?;
        let temporal_work = duration
            * (0.5 * complementary_rate_start + primary_rate_middle + 0.5 * complementary_rate_end);
        let energy_change = after - before;
        let accounting = CanonicalTemporalStepAccounting {
            temporal_work,
            energy_change,
            splitting_residual: energy_change - temporal_work,
        };
        if !accounting.temporal_work.is_finite()
            || !accounting.energy_change.is_finite()
            || !accounting.splitting_residual.is_finite()
        {
            return Err(WaveError::InvalidState);
        }
        self.primary_flux = primary;
        self.complementary_flux = complementary;
        self.time = end_time;
        Ok(accounting)
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

fn coefficient_for(
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
        BACKGROUND_REGION, CanonicalWaveState, ElectromagneticPolarization, LoopRole,
        MaterialFrame, MeshingOptions, Obstacle, ObstacleId, OuterBoundaryCondition,
        PeriodicCubicSpline, Region, RegionId, ScalarField, SymmetricTensor2, TimeDrive,
        enriched_quadratic_basis, mesh_scene,
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

    #[test]
    fn temporal_point_consumer_uses_endpoint_mass_and_the_probe_point_law() {
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
        let expected_complementary = fixed.complementary_inverse.apply(flux) / complementary_factor;
        let expected_energy = 0.5
            * (fixed.primary_reference * primary_factor * current_value.powi(2)
                + flux.dot(expected_complementary));
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
    fn bulk_symplectic_state_rejects_loss_and_open_boundaries() {
        let mut lossy_scene = Scene::initial();
        lossy_scene.materials[0].damping = ScalarField::constant(0.1);
        let lossy = compile(&lossy_scene).unwrap();
        assert!(lossy.has_loss());
        assert!(!lossy.conservative_bulk_supported());
        assert!(matches!(
            CanonicalTemporalWaveState::zero(&lossy, 0.1 * lossy.maximum_time_step()),
            Err(WaveError::InvalidCoefficients)
        ));

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
            OuterBoundaryCondition::FirstOrderOutgoing,
        )
        .unwrap();
        let open =
            CanonicalTemporalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 1).unwrap();
        assert!(!open.conservative_bulk_supported());
        assert!(matches!(
            CanonicalTemporalWaveState::zero(&open, 0.1 * open.maximum_time_step()),
            Err(WaveError::InvalidCoefficients)
        ));
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
