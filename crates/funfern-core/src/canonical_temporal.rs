//! Dormant f64 reference for Stage 7 time-driven, field-linear media.
//!
//! The accepted production operator remains the fixed linear
//! [`CanonicalWaveOperator`]. This wrapper compiles exact material-frame law
//! samples beside that operator and evaluates them at synchronized stage
//! times. Keeping the paths separate until the temporal gates close prevents
//! a valid authored drive from being silently executed as a static material.

use std::collections::BTreeSet;

use crate::{
    CanonicalWaveOperator, CoefficientLaw, CoefficientLawValues, DampingLaw, DampingLawValues,
    ElectromagneticPolarization, FieldLaw, LossChannel, Material, MaterialCoordinates,
    MaterialError, MaterialId, MaterialSwitchRuntime, PhysicsModel, Point2, QuadraticWaveOperator,
    RateLaw, RateLawValues, RestoringLaw, Scene, TimeDriveRuntime, TimeDriveValues,
    TopologyWaveModel, TriMesh, WaveError,
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
        Ok(Self {
            base,
            primary,
            complementary,
            initial_runtime,
            has_temporal_laws,
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

    pub fn primary_mass_at(
        &self,
        time: f64,
        runtime: &CanonicalMaterialRuntimeState,
    ) -> Result<Vec<f64>, WaveError> {
        if !self.has_temporal_laws {
            return Ok(self.base.primary_mass().to_vec());
        }
        let mut mass = vec![0.0; self.base.degrees_of_freedom()];
        for (contribution, temporal) in self.base.primary_contributions().iter().zip(&self.primary)
        {
            let factor = coefficient_factor(temporal.coefficient, time, runtime)?;
            mass[contribution.node as usize] +=
                contribution.geometric_weight * contribution.reference_coefficient * factor;
        }
        validate_positive(&mass)?;
        Ok(mass)
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
    let runtime = runtime.record(sample.material)?;
    sample
        .law
        .temporal_factor(
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
        BACKGROUND_REGION, ElectromagneticPolarization, LoopRole, MaterialFrame, MeshingOptions,
        Obstacle, ObstacleId, OuterBoundaryCondition, PeriodicCubicSpline, Region, RegionId,
        ScalarField, SymmetricTensor2, TimeDrive, mesh_scene,
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
