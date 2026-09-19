//! Linear CPU reference for the direct canonical `(Q, b)` formulation.
//!
//! This module deliberately does not replace the production scalar solver yet.
//! It keeps each nodal constitutive contribution and each quadrature vector
//! sample explicit, providing an oracle for the later source, boundary,
//! transfer, and GPU stages.

use std::{collections::BTreeSet, sync::Arc};

use crate::{
    DirectionalWaveCoefficients, ElectromagneticPolarization, OwnedTopologyWaveModel, PhysicsModel,
    Point2, QuadraticWaveOperator, Scene, SymmetricTensor2, TopologyWaveModel, TriMesh, WaveError,
    enriched_quadratic_basis_gradients,
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

/// Stage 2 has no physical auxiliary equations. Keeping an explicit owned slot
/// prevents later trace or oscillator state from being smuggled into `Q` or
/// `b`; later stages add physically named variants here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum CanonicalAuxiliaryState {
    #[default]
    None,
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
    primary_mass: Vec<f64>,
    samples: Vec<LinearConstitutiveSample>,
    component_labels: Vec<u32>,
    component_count: usize,
    maximum_time_step: f64,
}

impl CanonicalWaveOperator {
    /// Compiles the interior direct operator against an already assembled
    /// quadratic operator. Boundary composition intentionally remains Stage 3.
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

    pub fn recommended_time_step(&self) -> f64 {
        0.9 * self.maximum_time_step
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
            + self.primary_mass.len() * std::mem::size_of::<f64>()
            + self.samples.len() * std::mem::size_of::<LinearConstitutiveSample>()
            + self.component_labels.len() * std::mem::size_of::<u32>()
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

/// Owned synchronized direct state. Derived `u`, `v`, and forces are not
/// authoritative and are reconstructed from these arrays.
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalWaveState {
    primary_flux: Vec<f64>,
    complementary_flux: Vec<Point2>,
    auxiliaries: CanonicalAuxiliaryState,
    time_step: f64,
    steps: u64,
}

impl CanonicalWaveState {
    pub fn new(
        operator: &CanonicalWaveOperator,
        time_step: f64,
        primary_flux: Vec<f64>,
        complementary_flux: Vec<Point2>,
    ) -> Result<Self, WaveError> {
        validate_time_step(operator, time_step)?;
        operator.validate_primary(&primary_flux)?;
        operator.validate_complementary(&complementary_flux)?;
        Ok(Self {
            primary_flux,
            complementary_flux,
            auxiliaries: CanonicalAuxiliaryState::None,
            time_step,
            steps: 0,
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

    pub fn steps(&self) -> u64 {
        self.steps
    }

    pub fn time(&self) -> f64 {
        self.steps as f64 * self.time_step
    }

    pub fn estimated_state_bytes(&self) -> usize {
        self.primary_flux.len() * std::mem::size_of::<f64>()
            + self.complementary_flux.len() * std::mem::size_of::<Point2>()
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
        let energy = primary_energy + complementary_energy;
        energy
            .is_finite()
            .then_some(energy)
            .ok_or(WaveError::InvalidState)
    }

    /// One lossless endpoint KDK step.
    pub fn step(&mut self, operator: &CanonicalWaveOperator) -> Result<(), WaveError> {
        validate_time_step(operator, self.time_step)?;
        operator.validate_primary(&self.primary_flux)?;
        operator.validate_complementary(&self.complementary_flux)?;
        let half_step = 0.5 * self.time_step;
        let first_force = operator.force(&self.complementary_flux)?;
        for (flux, force) in self.primary_flux.iter_mut().zip(first_force) {
            *flux -= half_step * force;
        }
        let primary = operator.primary_field(&self.primary_flux)?;
        operator.drift(&mut self.complementary_flux, &primary, self.time_step)?;
        let second_force = operator.force(&self.complementary_flux)?;
        for (flux, force) in self.primary_flux.iter_mut().zip(second_force) {
            *flux -= half_step * force;
            if !flux.is_finite() {
                return Err(WaveError::InvalidState);
            }
        }
        self.steps = self.steps.checked_add(1).ok_or(WaveError::InvalidState)?;
        Ok(())
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
    next_element: usize,
    primary_contributions: Vec<LinearPrimaryContribution>,
    primary_mass: Vec<f64>,
    samples: Vec<LinearConstitutiveSample>,
    interior_columns: Vec<BTreeSet<u32>>,
    done: bool,
}

impl CanonicalAssemblyJob {
    pub fn new(
        mesh: Arc<TriMesh>,
        quadratic: Arc<QuadraticWaveOperator>,
        model: TopologyWaveModel<'_>,
        constitutive_revision: u64,
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
        if quadratic.lumped_damping().iter().any(|value| *value != 0.0)
            || quadratic
                .auxiliary_stiffness_values()
                .iter()
                .any(|value| *value != 0.0)
            || quadratic.dirichlet_signals().iter().any(Option::is_some)
            || quadratic
                .normalized_neumann_weights()
                .iter()
                .flatten()
                .any(|value| *value != 0.0)
            || quadratic
                .face_neumann_loads()
                .iter()
                .flatten()
                .any(|load| load.normalized_weight != 0.0)
        {
            return Err(WaveError::InvalidMesh(
                "loss, driven boundaries, and boundary memory enter the canonical core in Stage 3",
            ));
        }
        let generation = CanonicalGenerationTag::from_mesh(&mesh, constitutive_revision);
        Ok(Self {
            mesh,
            primary_mass: vec![0.0; quadratic.degrees_of_freedom()],
            primary_contributions: Vec::with_capacity(
                quadratic.element_nodes().len() * LOCAL_NODES,
            ),
            samples: Vec::with_capacity(quadratic.element_nodes().len() * QUADRATURE_SAMPLES),
            interior_columns: vec![BTreeSet::new(); quadratic.degrees_of_freedom()],
            quadratic,
            model: model.to_owned(),
            generation,
            next_element: 0,
            done: false,
        })
    }

    pub fn phase(&self) -> &'static str {
        if self.next_element < self.mesh.triangles.len() {
            "Compiling canonical constitutive samples"
        } else {
            "Finishing canonical operator"
        }
    }

    /// Runs up to `budget` element/finish work units.
    pub fn advance(&mut self, budget: usize) -> Option<Result<CanonicalWaveOperator, WaveError>> {
        for _ in 0..budget {
            if self.done {
                return None;
            }
            if self.next_element < self.mesh.triangles.len() {
                if let Err(error) = self.compile_element(self.next_element) {
                    self.done = true;
                    return Some(Err(error));
                }
                self.next_element += 1;
            } else {
                self.done = true;
                return Some(self.finish());
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
            let values = model
                .directional_material_at(triangle.region, self.quadratic.node_points()[node])
                .map_err(|_| WaveError::InvalidCoefficients)?;
            let geometric_weight = area * MASS_WEIGHTS[local];
            self.primary_mass[node] += geometric_weight * values.mass_density;
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
            let values = model
                .directional_material_at(triangle.region, point)
                .map_err(|_| WaveError::InvalidCoefficients)?;
            let complementary_inverse = rotate_tensor(values);
            self.samples.push(LinearConstitutiveSample {
                element: element as u32,
                barycentric,
                point,
                integration_weight: area * reference_weight,
                complementary_reference: inverse_tensor(complementary_inverse)
                    .ok_or(WaveError::InvalidCoefficients)?,
                complementary_inverse,
                curls: enriched_quadratic_basis_gradients(barycentric, barycentric_gradients)
                    .map(rotate_vector),
            });
        }
        Ok(())
    }

    fn finish(&self) -> Result<CanonicalWaveOperator, WaveError> {
        if self
            .primary_mass
            .iter()
            .any(|mass| !mass.is_finite() || *mass <= 0.0)
            || self.samples.len() != self.mesh.triangles.len() * QUADRATURE_SAMPLES
        {
            return Err(WaveError::InvalidCoefficients);
        }
        for (canonical, legacy) in self.primary_mass.iter().zip(self.quadratic.lumped_mass()) {
            let tolerance = 2.0e-12 * canonical.abs().max(legacy.abs()).max(1.0);
            if (canonical - legacy).abs() > tolerance {
                return Err(WaveError::InvalidMesh(
                    "canonical and scalar nodal constitutive maps disagree",
                ));
            }
        }
        for (row, expected) in self.interior_columns.iter().enumerate() {
            let start = self.quadratic.row_offsets()[row] as usize;
            let end = self.quadratic.row_offsets()[row + 1] as usize;
            if expected.len() != end - start
                || self.quadratic.columns()[start..end]
                    .iter()
                    .any(|column| !expected.contains(column))
            {
                return Err(WaveError::InvalidMesh(
                    "the scalar operator contains Stage 3 boundary stiffness",
                ));
            }
        }
        let (component_labels, component_count) = connected_components(
            self.quadratic.degrees_of_freedom(),
            self.quadratic.element_nodes(),
        );
        let operator = CanonicalWaveOperator {
            generation: self.generation,
            physics: self.model.physics,
            orientation: orientation(self.model.physics),
            node_points: self.quadratic.node_points().to_vec(),
            element_nodes: self.quadratic.element_nodes().to_vec(),
            primary_contributions: self.primary_contributions.clone(),
            primary_mass: self.primary_mass.clone(),
            samples: self.samples.clone(),
            component_labels,
            component_count,
            maximum_time_step: self.quadratic.maximum_time_step(),
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

fn rotate_vector(vector: Point2) -> Point2 {
    Point2::new(-vector.y, vector.x)
}

fn rotate_tensor(values: DirectionalWaveCoefficients) -> SymmetricTensor2 {
    let tensor = values.stiffness;
    SymmetricTensor2::new(tensor.yy, -tensor.xy, tensor.xx)
}

fn inverse_tensor(tensor: SymmetricTensor2) -> Option<SymmetricTensor2> {
    let determinant = tensor.determinant();
    (tensor.finite_spd() && determinant.is_finite() && determinant > 0.0).then(|| {
        SymmetricTensor2::new(
            tensor.yy / determinant,
            -tensor.xy / determinant,
            tensor.xx / determinant,
        )
    })
}

fn validate_time_step(operator: &CanonicalWaveOperator, time_step: f64) -> Result<(), WaveError> {
    if !time_step.is_finite() || time_step <= 0.0 || time_step > operator.maximum_time_step() {
        Err(WaveError::InvalidTimeStep {
            requested: time_step,
            maximum: operator.maximum_time_step(),
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

fn connected_components(node_count: usize, elements: &[[u32; LOCAL_NODES]]) -> (Vec<u32>, usize) {
    let mut parent = (0..node_count).collect::<Vec<_>>();
    for nodes in elements {
        let first = nodes[0] as usize;
        for &node in &nodes[1..] {
            union(&mut parent, first, node as usize);
        }
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
        BACKGROUND_REGION, MaterialFrame, MeshQuality, MeshTriangle, MeshVertex,
        OuterBoundaryCondition, QuadraticWaveState, ScalarField,
    };

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

    #[test]
    fn stage_two_compiler_refuses_loss_instead_of_silently_ignoring_it() {
        let mesh = square();
        let mut scene = Scene::default();
        scene.materials[0].damping = ScalarField::constant(0.1);
        let quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        assert!(CanonicalWaveOperator::compile_scene(&mesh, &quadratic, &scene, 7).is_err());
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
        let mut direct =
            CanonicalWaveState::from_primary_velocity(&operator, dt, &primary, &velocity).unwrap();
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
}
