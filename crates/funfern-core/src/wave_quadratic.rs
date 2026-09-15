use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::{
    BACKGROUND_REGION, BoundaryLabel, DirectionalWaveCoefficients, FaceBoundaryCondition,
    InternalBoundaryCoupling, InternalBoundaryId, InternalBoundarySide, Material,
    OuterBoundaryCondition, OuterBoundaryConditions, OuterSide, PhysicsModel,
    PlannedBoundarySource, Point2, Region, RegionId, Scene, SpanBehavior, SymmetricTensor2,
    TimeSignal, TopologyMeshPlan, TriMesh, WaveCoefficients, WaveError,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoundaryLoad {
    pub signal: TimeSignal,
    pub normalized_weight: f64,
}

/// Material and presentation-independent physics data used by a compiled
/// topology. It borrows the document libraries without depending on legacy
/// obstacle, divider, or baffle collections.
#[derive(Clone, Copy, Debug)]
pub struct TopologyWaveModel<'a> {
    pub physics: PhysicsModel,
    pub materials: &'a [Material],
    pub regions: &'a [Region],
    pub outer_boundaries: OuterBoundaryConditions,
}

/// Owned material/physics snapshot for cooperative topology consumers.
#[derive(Clone, Debug, PartialEq)]
pub struct OwnedTopologyWaveModel {
    pub physics: PhysicsModel,
    pub materials: Vec<Material>,
    pub regions: Vec<Region>,
    pub outer_boundaries: OuterBoundaryConditions,
}

impl OwnedTopologyWaveModel {
    pub fn as_model(&self) -> TopologyWaveModel<'_> {
        TopologyWaveModel {
            physics: self.physics,
            materials: &self.materials,
            regions: &self.regions,
            outer_boundaries: self.outer_boundaries,
        }
    }
}

impl<'a> TopologyWaveModel<'a> {
    pub fn from_topology_scene(scene: &'a crate::TopologyScene) -> Self {
        Self {
            physics: scene.physics,
            materials: &scene.materials,
            regions: &scene.regions,
            outer_boundaries: scene.outer_boundaries,
        }
    }

    pub fn from_scene(scene: &'a Scene) -> Self {
        Self {
            physics: scene.physics,
            materials: &scene.materials,
            regions: &scene.regions,
            outer_boundaries: scene.outer_boundaries,
        }
    }

    pub fn to_owned(self) -> OwnedTopologyWaveModel {
        OwnedTopologyWaveModel {
            physics: self.physics,
            materials: self.materials.to_vec(),
            regions: self.regions.to_vec(),
            outer_boundaries: self.outer_boundaries,
        }
    }

    pub fn region(self, id: RegionId) -> Option<&'a Region> {
        self.regions.iter().find(|region| region.id == id)
    }

    pub fn material(self, id: crate::MaterialId) -> Option<&'a Material> {
        self.materials.iter().find(|material| material.id == id)
    }

    pub fn material_at(
        self,
        region: RegionId,
        point: Point2,
    ) -> Result<crate::EvaluatedMaterial, crate::MaterialError> {
        crate::wave::evaluate_material_library_at(
            self.physics,
            self.materials,
            self.regions,
            region,
            point,
        )
    }

    pub fn directional_material_at(
        self,
        region: RegionId,
        point: Point2,
    ) -> Result<DirectionalWaveCoefficients, crate::MaterialError> {
        crate::wave::evaluate_directional_material_library_at(
            self.physics,
            self.materials,
            self.regions,
            region,
            point,
        )
    }

    pub fn valid_for(self, plan: &TopologyMeshPlan) -> bool {
        self.outer_boundaries.valid()
            && self.materials.iter().all(Material::valid)
            && self.regions.iter().all(|region| {
                region.id.0 > 0
                    && region.frame.valid()
                    && self
                        .materials
                        .iter()
                        .any(|material| material.id == region.material)
            })
            && self.materials.iter().enumerate().all(|(index, material)| {
                !self.materials[..index]
                    .iter()
                    .any(|previous| previous.id == material.id)
            })
            && self.regions.iter().enumerate().all(|(index, region)| {
                !self.regions[..index]
                    .iter()
                    .any(|previous| previous.id == region.id)
            })
            && plan
                .domains
                .iter()
                .all(|domain| self.regions.iter().any(|region| region.id == domain.region))
    }
}

/// Seven-node mass-lumped triangle: `P2` enriched by the cubic interior bubble.
/// Vertex, edge-midpoint, and centroid masses use the positive degree-three nodal
/// quadrature weights 1/20, 2/15, and 9/20 of the element area.
#[derive(Clone, Debug, PartialEq)]
pub struct QuadraticWaveOperator {
    geometry_revision: u64,
    mesh_revision: u64,
    outer_boundaries: OuterBoundaryConditions,
    node_points: Vec<Point2>,
    element_nodes: Vec<[u32; 7]>,
    row_offsets: Vec<u32>,
    columns: Vec<u32>,
    stiffness: Vec<f64>,
    auxiliary_stiffness: Vec<f64>,
    auxiliary_active: Vec<bool>,
    dirichlet_sides: Vec<Option<OuterSide>>,
    dirichlet_signals: Vec<Option<TimeSignal>>,
    normalized_neumann_weights: Vec<[f64; 4]>,
    face_neumann_loads: Vec<[BoundaryLoad; 2]>,
    lumped_mass: Vec<f64>,
    lumped_damping: Vec<f64>,
    maximum_eigenvalue_bound: f64,
    maximum_time_step: f64,
}

impl QuadraticWaveOperator {
    pub fn assemble(mesh: &TriMesh, coefficients: WaveCoefficients) -> Result<Self, WaveError> {
        Self::assemble_with_boundary(mesh, coefficients, OuterBoundaryCondition::Reflecting)
    }

    pub fn assemble_with_boundary(
        mesh: &TriMesh,
        coefficients: WaveCoefficients,
        outer_boundary: OuterBoundaryCondition,
    ) -> Result<Self, WaveError> {
        Self::assemble_regions(
            mesh,
            BTreeMap::from([(BACKGROUND_REGION, coefficients)]),
            coefficients,
            OuterBoundaryConditions::uniform(outer_boundary),
            None,
        )
    }

    pub fn assemble_scene(
        mesh: &TriMesh,
        scene: &Scene,
        outer_boundary: OuterBoundaryCondition,
    ) -> Result<Self, WaveError> {
        Self::assemble_scene_with_boundaries(
            mesh,
            scene,
            OuterBoundaryConditions::uniform(outer_boundary),
        )
    }

    pub fn assemble_scene_with_boundaries(
        mesh: &TriMesh,
        scene: &Scene,
        outer_boundaries: OuterBoundaryConditions,
    ) -> Result<Self, WaveError> {
        if !scene.structure_valid() {
            return Err(WaveError::InvalidCoefficients);
        }
        Self::assemble_with_provider(
            mesh,
            CoefficientProvider::Scene(scene),
            outer_boundaries,
            Some(scene),
            None,
            scene.physics,
        )
    }

    /// Assembles materials and all curve-side laws from the unified topology
    /// contract. The mesh may be reused across a law-only plan revision.
    pub fn assemble_topology(
        mesh: &TriMesh,
        plan: &TopologyMeshPlan,
        model: TopologyWaveModel<'_>,
    ) -> Result<Self, WaveError> {
        if !model.valid_for(plan) {
            return Err(WaveError::InvalidCoefficients);
        }
        Self::assemble_with_provider(
            mesh,
            CoefficientProvider::Topology(model),
            model.outer_boundaries,
            None,
            Some(plan),
            model.physics,
        )
    }

    fn assemble_regions(
        mesh: &TriMesh,
        coefficients_by_region: BTreeMap<RegionId, WaveCoefficients>,
        outer_coefficients: WaveCoefficients,
        outer_boundaries: OuterBoundaryConditions,
        scene: Option<&Scene>,
    ) -> Result<Self, WaveError> {
        validate_coefficients(outer_coefficients)?;
        if !outer_boundaries.valid() {
            return Err(WaveError::InvalidCoefficients);
        }
        if coefficients_by_region
            .values()
            .copied()
            .any(|coefficients| validate_coefficients(coefficients).is_err())
        {
            return Err(WaveError::InvalidCoefficients);
        }
        Self::assemble_with_provider(
            mesh,
            CoefficientProvider::Constant(&coefficients_by_region),
            outer_boundaries,
            scene,
            None,
            scene.map_or(PhysicsModel::Mechanical, |scene| scene.physics),
        )
    }

    fn assemble_with_provider(
        mesh: &TriMesh,
        coefficients: CoefficientProvider<'_>,
        outer_boundaries: OuterBoundaryConditions,
        scene: Option<&Scene>,
        topology: Option<&TopologyMeshPlan>,
        physics: PhysicsModel,
    ) -> Result<Self, WaveError> {
        let mut work = QuadraticAssemblyWork::new(outer_boundaries, physics)?;
        loop {
            if let Some(operator) = work.step(mesh, coefficients, scene, topology, physics)? {
                return Ok(operator);
            }
        }
    }

    pub fn geometry_revision(&self) -> u64 {
        self.geometry_revision
    }

    pub fn mesh_revision(&self) -> u64 {
        self.mesh_revision
    }

    pub fn outer_boundaries(&self) -> OuterBoundaryConditions {
        self.outer_boundaries
    }

    /// Returns the common condition for operators assembled through the legacy
    /// uniform-boundary API.
    pub fn outer_boundary(&self) -> OuterBoundaryCondition {
        debug_assert!(
            self.outer_boundaries.sides[1..]
                .iter()
                .all(|condition| *condition == self.outer_boundaries.sides[0])
        );
        self.outer_boundaries.sides[0]
    }

    pub fn dirichlet_sides(&self) -> &[Option<OuterSide>] {
        &self.dirichlet_sides
    }

    pub fn normalized_neumann_weights(&self) -> &[[f64; 4]] {
        &self.normalized_neumann_weights
    }

    pub fn dirichlet_signals(&self) -> &[Option<TimeSignal>] {
        &self.dirichlet_signals
    }

    pub fn face_neumann_loads(&self) -> &[[BoundaryLoad; 2]] {
        &self.face_neumann_loads
    }

    pub fn prescribed_value(&self, node: usize, time: f64) -> Option<f64> {
        self.dirichlet_signals
            .get(node)
            .copied()
            .flatten()
            .map(|signal| signal.value(time))
    }

    pub fn neumann_acceleration(&self, node: usize, time: f64) -> f64 {
        self.normalized_neumann_weights
            .get(node)
            .map(|weights| {
                OuterSide::ALL
                    .into_iter()
                    .zip(weights)
                    .map(|(side, weight)| match self.outer_boundaries.get(side) {
                        OuterBoundaryCondition::Neumann { signal } => weight * signal.value(time),
                        _ => 0.0,
                    })
                    .sum()
            })
            .unwrap_or(0.0)
            + self
                .face_neumann_loads
                .get(node)
                .map(|loads| {
                    loads
                        .iter()
                        .map(|load| load.normalized_weight * load.signal.value(time))
                        .sum()
                })
                .unwrap_or(0.0)
    }

    pub fn node_points(&self) -> &[Point2] {
        &self.node_points
    }

    pub fn element_nodes(&self) -> &[[u32; 7]] {
        &self.element_nodes
    }

    /// Evaluates a nodal field and its world-space gradient in one element.
    /// The barycentric coordinates refer to the element's three vertex nodes.
    pub fn element_value_and_gradient(
        &self,
        element: usize,
        nodal_values: &[f32],
        barycentric: [f64; 3],
    ) -> Option<(f64, Point2)> {
        let nodes = *self.element_nodes.get(element)?;
        if nodal_values.len() != self.node_points.len()
            || barycentric.iter().any(|value| !value.is_finite())
        {
            return None;
        }
        let points = [
            self.node_points[nodes[0] as usize],
            self.node_points[nodes[1] as usize],
            self.node_points[nodes[2] as usize],
        ];
        let twice_area = (points[1] - points[0]).cross(points[2] - points[0]);
        if !twice_area.is_finite() || twice_area <= 0.0 {
            return None;
        }
        let barycentric_gradients = [
            Point2::new(points[1].y - points[2].y, points[2].x - points[1].x) / twice_area,
            Point2::new(points[2].y - points[0].y, points[0].x - points[2].x) / twice_area,
            Point2::new(points[0].y - points[1].y, points[1].x - points[0].x) / twice_area,
        ];
        let basis = enriched_quadratic_basis(barycentric);
        let gradients = enriched_quadratic_basis_gradients(barycentric, barycentric_gradients);
        let mut value = 0.0;
        let mut gradient = Point2::default();
        for local in 0..7 {
            let nodal = nodal_values[nodes[local] as usize] as f64;
            value += basis[local] * nodal;
            gradient = gradient + gradients[local] * nodal;
        }
        (value.is_finite() && gradient.finite()).then_some((value, gradient))
    }

    pub fn degrees_of_freedom(&self) -> usize {
        self.node_points.len()
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

    pub fn auxiliary_stiffness_values(&self) -> &[f64] {
        &self.auxiliary_stiffness
    }

    pub fn auxiliary_active(&self) -> &[bool] {
        &self.auxiliary_active
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
        if values.iter().any(|value| !value.is_finite()) {
            return Err(WaveError::InvalidState);
        }
        let mut result = vec![0.0; values.len()];
        for (row, output) in result.iter_mut().enumerate() {
            *output = (self.row_offsets[row] as usize..self.row_offsets[row + 1] as usize)
                .map(|entry| self.stiffness[entry] * values[self.columns[entry] as usize])
                .sum();
        }
        Ok(result)
    }

    pub fn apply_auxiliary_stiffness(&self, values: &[f64]) -> Result<Vec<f64>, WaveError> {
        if values.len() != self.degrees_of_freedom() {
            return Err(WaveError::SizeMismatch {
                expected: self.degrees_of_freedom(),
                actual: values.len(),
            });
        }
        if values.iter().any(|value| !value.is_finite()) {
            return Err(WaveError::InvalidState);
        }
        let mut result = vec![0.0; values.len()];
        for (row, output) in result.iter_mut().enumerate() {
            *output = (self.row_offsets[row] as usize..self.row_offsets[row + 1] as usize)
                .map(|entry| self.auxiliary_stiffness[entry] * values[self.columns[entry] as usize])
                .sum();
        }
        Ok(result)
    }

    /// Values for the GPU gather kernel, normalized row-wise by lumped mass.
    pub fn normalized_stiffness_f32(&self) -> Result<Vec<f32>, WaveError> {
        let mut result = Vec::with_capacity(self.stiffness.len());
        for row in 0..self.degrees_of_freedom() {
            for entry in self.row_offsets[row] as usize..self.row_offsets[row + 1] as usize {
                let value = (self.stiffness[entry] / self.lumped_mass[row]) as f32;
                if !value.is_finite() {
                    return Err(WaveError::InvalidMesh("the f32 GPU operator overflows"));
                }
                result.push(value);
            }
        }
        Ok(result)
    }

    pub fn normalized_auxiliary_stiffness_f32(&self) -> Result<Vec<f32>, WaveError> {
        let mut result = Vec::with_capacity(self.auxiliary_stiffness.len());
        for row in 0..self.degrees_of_freedom() {
            for entry in self.row_offsets[row] as usize..self.row_offsets[row + 1] as usize {
                let value = (self.auxiliary_stiffness[entry] / self.lumped_mass[row]) as f32;
                if !value.is_finite() {
                    return Err(WaveError::InvalidMesh(
                        "the f32 GPU auxiliary operator overflows",
                    ));
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

    pub fn estimated_gpu_bytes(&self) -> usize {
        self.row_offsets.len() * 4 + self.columns.len() * 12 + self.degrees_of_freedom() * 112
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
        energy
            .is_finite()
            .then_some(energy)
            .ok_or(WaveError::InvalidState)
    }

    pub fn discrete_energy_with_auxiliary(
        &self,
        current: &[f64],
        previous: &[f64],
        auxiliary: &[f64],
        time_step: f64,
    ) -> Result<f64, WaveError> {
        let interior = self.discrete_energy(current, previous, time_step)?;
        let auxiliary_force = self.apply_auxiliary_stiffness(auxiliary)?;
        let boundary = 0.5
            * auxiliary
                .iter()
                .zip(auxiliary_force)
                .map(|(value, force)| value * force)
                .sum::<f64>();
        let energy = interior + boundary;
        energy
            .is_finite()
            .then_some(energy)
            .ok_or(WaveError::InvalidState)
    }

    /// Electromagnetic energy for a TE/TM primary component and the scalar
    /// potential used to reconstruct its transverse counterpart.
    pub fn electromagnetic_energy(
        &self,
        primary: &[f64],
        transverse_potential: &[f64],
    ) -> Result<f64, WaveError> {
        if primary.len() != self.degrees_of_freedom()
            || transverse_potential.len() != self.degrees_of_freedom()
        {
            return Err(WaveError::SizeMismatch {
                expected: self.degrees_of_freedom(),
                actual: primary.len().min(transverse_potential.len()),
            });
        }
        if primary
            .iter()
            .chain(transverse_potential)
            .any(|value| !value.is_finite())
        {
            return Err(WaveError::InvalidState);
        }
        let potential_force = self.apply_stiffness(transverse_potential)?;
        let primary_energy = primary
            .iter()
            .zip(&self.lumped_mass)
            .map(|(value, mass)| 0.5 * mass * value * value)
            .sum::<f64>();
        let transverse_energy = transverse_potential
            .iter()
            .zip(potential_force)
            .map(|(value, force)| 0.5 * value * force)
            .sum::<f64>();
        let energy = primary_energy + transverse_energy;
        energy
            .is_finite()
            .then_some(energy)
            .ok_or(WaveError::InvalidState)
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
pub struct QuadraticWaveState {
    current: Vec<f64>,
    previous: Vec<f64>,
    scratch: Vec<f64>,
    auxiliary: Vec<f64>,
    time_step: f64,
    steps: u64,
}

impl QuadraticWaveState {
    pub fn new(
        operator: &QuadraticWaveOperator,
        time_step: f64,
        displacement: Vec<f64>,
        velocity: Vec<f64>,
    ) -> Result<Self, WaveError> {
        Self::new_with_auxiliary(
            operator,
            time_step,
            displacement,
            velocity,
            vec![0.0; operator.degrees_of_freedom()],
        )
    }

    pub fn new_with_auxiliary(
        operator: &QuadraticWaveOperator,
        time_step: f64,
        mut displacement: Vec<f64>,
        velocity: Vec<f64>,
        auxiliary: Vec<f64>,
    ) -> Result<Self, WaveError> {
        operator.validate_levels(&displacement, &velocity, time_step)?;
        if auxiliary.len() != operator.degrees_of_freedom() {
            return Err(WaveError::SizeMismatch {
                expected: operator.degrees_of_freedom(),
                actual: auxiliary.len(),
            });
        }
        if auxiliary.iter().any(|value| !value.is_finite()) {
            return Err(WaveError::InvalidState);
        }
        for (node, value) in displacement.iter_mut().enumerate() {
            if let Some(prescribed) = operator.prescribed_value(node, 0.0) {
                *value = prescribed;
            }
        }
        let stiffness = operator.apply_stiffness(&displacement)?;
        let auxiliary_force = operator.apply_auxiliary_stiffness(&auxiliary)?;
        let previous = displacement
            .iter()
            .zip(&velocity)
            .zip(stiffness)
            .zip(auxiliary_force)
            .zip(&operator.lumped_mass)
            .zip(&operator.lumped_damping)
            .enumerate()
            .map(
                |(node, (((((displacement, velocity), stiffness), auxiliary), mass), damping))| {
                    if let Some(prescribed) = operator.prescribed_value(node, -time_step) {
                        prescribed
                    } else {
                        let acceleration = (-stiffness - auxiliary - damping * velocity) / mass
                            + operator.neumann_acceleration(node, 0.0);
                        displacement - time_step * velocity
                            + 0.5 * time_step * time_step * acceleration
                    }
                },
            )
            .collect::<Vec<_>>();
        if previous.iter().any(|value| !value.is_finite()) {
            return Err(WaveError::InvalidState);
        }
        Ok(Self {
            current: displacement,
            previous,
            scratch: vec![0.0; operator.degrees_of_freedom()],
            auxiliary,
            time_step,
            steps: 0,
        })
    }

    pub fn zero(operator: &QuadraticWaveOperator, time_step: f64) -> Result<Self, WaveError> {
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

    pub fn auxiliary(&self) -> &[f64] {
        &self.auxiliary
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

    pub fn energy(&self, operator: &QuadraticWaveOperator) -> Result<f64, WaveError> {
        operator.discrete_energy_with_auxiliary(
            &self.current,
            &self.previous,
            &self.auxiliary,
            self.time_step,
        )
    }

    pub fn step(
        &mut self,
        operator: &QuadraticWaveOperator,
        acceleration: &[f64],
    ) -> Result<(), WaveError> {
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
        let time = self.time();
        for i in 0..self.current.len() {
            if let Some(prescribed) = operator.prescribed_value(i, time + dt) {
                self.scratch[i] = prescribed;
                continue;
            }
            // Both operators annihilate constants, so `K u` is evaluated on the
            // differences `u_j - u_i`. That form is exactly zero on a constant
            // field in floating point, where the plain row product leaves a
            // rounding residual that acts as a permanent force on the free
            // constant mode of a Neumann cavity. `wave.wgsl` uses the same form.
            let (displacement, memory) = (self.current[i], self.auxiliary[i]);
            let (stiffness, auxiliary) = (operator.row_offsets[i] as usize
                ..operator.row_offsets[i + 1] as usize)
                .fold((0.0, 0.0), |(stiffness, auxiliary), entry| {
                    let column = operator.columns[entry] as usize;
                    (
                        stiffness
                            + operator.stiffness[entry] * (self.current[column] - displacement),
                        auxiliary
                            + operator.auxiliary_stiffness[entry]
                                * (self.auxiliary[column] - memory),
                    )
                });
            let gamma = operator.lumped_damping[i] / operator.lumped_mass[i];
            let source = acceleration.get(i).copied().unwrap_or(0.0)
                + operator.neumann_acceleration(i, time);
            self.scratch[i] = (2.0 * self.current[i]
                - (1.0 - 0.5 * gamma * dt) * self.previous[i]
                - dt2 * (stiffness + auxiliary) / operator.lumped_mass[i]
                + dt2 * source)
                / (1.0 + 0.5 * gamma * dt);
            if !self.scratch[i].is_finite() {
                return Err(WaveError::InvalidState);
            }
        }
        for i in 0..self.auxiliary.len() {
            if operator.auxiliary_active[i] && operator.dirichlet_signals[i].is_none() {
                self.auxiliary[i] += 0.5 * dt * (self.current[i] + self.scratch[i]);
            } else {
                self.auxiliary[i] = 0.0;
            }
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
}

type TraceNodes = ([usize; 3], f64);

#[derive(Clone, Copy)]
enum CoefficientProvider<'a> {
    Constant(&'a BTreeMap<RegionId, WaveCoefficients>),
    Scene(&'a Scene),
    Topology(TopologyWaveModel<'a>),
}

impl CoefficientProvider<'_> {
    fn at(self, region: RegionId, point: Point2) -> Result<DirectionalWaveCoefficients, WaveError> {
        let values = match self {
            Self::Constant(values) => {
                let value = *values.get(&region).ok_or(WaveError::InvalidCoefficients)?;
                DirectionalWaveCoefficients {
                    mass_density: value.mass_density,
                    stiffness: SymmetricTensor2::isotropic(value.stiffness),
                    damping: value.damping,
                }
            }
            Self::Scene(scene) => {
                let region = scene
                    .region(region)
                    .ok_or(WaveError::InvalidMesh("a triangle has an unknown region"))?;
                let material = scene
                    .material(region.material)
                    .ok_or(WaveError::InvalidCoefficients)?;
                evaluate_directional_material(scene.physics, material, *region, point)?
            }
            Self::Topology(model) => {
                model.directional_material_at(region, point).map_err(|_| {
                    WaveError::InvalidMesh("a triangle has an unknown or invalid region material")
                })?
            }
        };
        if !values.valid() {
            return Err(WaveError::InvalidCoefficients);
        }
        Ok(values)
    }
}

fn evaluate_directional_material(
    physics: PhysicsModel,
    material: &Material,
    region: Region,
    point: Point2,
) -> Result<DirectionalWaveCoefficients, WaveError> {
    let coordinates = region.frame.coordinates(point);
    let evaluate = |field: &crate::ScalarField, coefficient: &'static str, positive: bool| {
        let value = field
            .evaluate(coordinates, &material.parameters)
            .map_err(|error| WaveError::MaterialEvaluation {
                material: material.name.clone(),
                coefficient,
                point,
                reason: error.to_string(),
            })?;
        if (positive && value <= 0.0) || (!positive && value < 0.0) {
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
        damping: evaluate(&material.damping, "damping", false)?,
        axis_ratio: evaluate(&material.axis_ratio, "axis ratio", true)?,
    };
    if properties.axis_ratio < 1.0 {
        return Err(WaveError::MaterialEvaluation {
            material: material.name.clone(),
            coefficient: "axis ratio",
            point,
            reason: "value must be at least one".into(),
        });
    }
    Ok(physics.directional_wave_coefficients(properties, region.frame))
}

struct BoundaryAssembly<'a> {
    edge_nodes: &'a BTreeMap<(usize, usize), usize>,
    rows: &'a mut [BTreeMap<usize, f64>],
    auxiliary_rows: &'a mut [BTreeMap<usize, f64>],
    auxiliary_active: &'a mut [bool],
    dirichlet_signals: &'a mut [Option<TimeSignal>],
    face_neumann_loads: &'a mut [[BoundaryLoad; 2]],
    damping: &'a mut [f64],
}

fn assemble_hole_boundary_conditions(
    mesh: &TriMesh,
    scene: &Scene,
    coefficients: CoefficientProvider<'_>,
    assembly: &mut BoundaryAssembly<'_>,
) -> Result<(), WaveError> {
    for edge in &mesh.boundary_edges {
        let BoundaryLabel::Obstacle(id) = edge.label else {
            continue;
        };
        let obstacle = scene
            .obstacles
            .iter()
            .find(|obstacle| obstacle.id == id)
            .ok_or(WaveError::InvalidMesh(
                "a hole boundary edge has an unknown ID",
            ))?;
        let crate::LoopRole::Hole { exterior } = obstacle.role else {
            return Err(WaveError::InvalidMesh(
                "a hole boundary edge references a non-hole loop",
            ));
        };
        let [a, b] = edge.vertices;
        if a >= mesh.vertices.len() || b >= mesh.vertices.len() || a == b {
            return Err(WaveError::InvalidMesh(
                "a hole boundary edge has invalid vertex indices",
            ));
        }
        let key = if a < b { (a, b) } else { (b, a) };
        let midpoint = *assembly.edge_nodes.get(&key).ok_or(WaveError::InvalidMesh(
            "a hole boundary edge does not belong to a triangle",
        ))?;
        let [parameter_a, parameter_b] = edge.parameters;
        if !parameter_a.is_finite() || !parameter_b.is_finite() || parameter_a == parameter_b {
            return Err(WaveError::InvalidMesh(
                "a hole boundary edge has invalid parameters",
            ));
        }
        let span = obstacle
            .spline
            .span_index(0.5 * (parameter_a + parameter_b))
            .ok_or(WaveError::InvalidMesh(
                "a hole boundary edge has an invalid spline parameter",
            ))?;
        let condition = obstacle.span_conditions[span];
        let length = (mesh.vertices[b].point - mesh.vertices[a].point).norm();
        if !length.is_finite() || length <= 0.0 {
            return Err(WaveError::InvalidMesh(
                "a hole boundary edge has invalid length",
            ));
        }
        let nodes = [a, midpoint, b];
        let values = [
            coefficients.at(exterior, mesh.vertices[a].point)?,
            coefficients.at(
                exterior,
                (mesh.vertices[a].point + mesh.vertices[b].point) / 2.0,
            )?,
            coefficients.at(exterior, mesh.vertices[b].point)?,
        ];
        let tangent = (mesh.vertices[b].point - mesh.vertices[a].point) / length;
        assemble_face_condition(
            condition.resolved(scene.physics),
            nodes,
            length,
            values,
            Point2::new(-tangent.y, tangent.x),
            assembly,
        )?;
    }
    Ok(())
}

fn assemble_internal_boundary_laws(
    mesh: &TriMesh,
    scene: &Scene,
    coefficients: CoefficientProvider<'_>,
    assembly: &mut BoundaryAssembly<'_>,
) -> Result<(), WaveError> {
    let mut traces = BTreeMap::<(InternalBoundaryId, u64, u64), [Option<TraceNodes>; 2]>::new();
    for edge in &mesh.boundary_edges {
        let BoundaryLabel::InternalBoundary { id, side } = edge.label else {
            continue;
        };
        let boundary = scene
            .internal_boundaries
            .iter()
            .find(|boundary| boundary.id == id)
            .ok_or(WaveError::InvalidMesh(
                "an internal-boundary edge has an unknown ID",
            ))?;
        let [a, b] = edge.vertices;
        if a >= mesh.vertices.len() || b >= mesh.vertices.len() || a == b {
            return Err(WaveError::InvalidMesh(
                "an internal-boundary edge has invalid vertex indices",
            ));
        }
        let key = if a < b { (a, b) } else { (b, a) };
        let midpoint = *assembly.edge_nodes.get(&key).ok_or(WaveError::InvalidMesh(
            "an internal-boundary edge does not belong to a triangle",
        ))?;
        let [parameter_a, parameter_b] = edge.parameters;
        if !parameter_a.is_finite() || !parameter_b.is_finite() || parameter_a == parameter_b {
            return Err(WaveError::InvalidMesh(
                "an internal-boundary edge has invalid parameters",
            ));
        }
        let parameter = 0.5 * (parameter_a + parameter_b);
        let span = boundary
            .spline
            .span_index(parameter)
            .ok_or(WaveError::InvalidMesh(
                "an internal-boundary edge is outside its spline parameter range",
            ))?;
        let law = boundary.span_laws[span];
        let condition = match side {
            InternalBoundarySide::Left => law.left,
            InternalBoundarySide::Right => law.right,
        };
        let length = (mesh.vertices[b].point - mesh.vertices[a].point).norm();
        if !length.is_finite() || length <= 0.0 {
            return Err(WaveError::InvalidMesh(
                "an internal-boundary edge has invalid length",
            ));
        }
        let nodes = if parameter_a < parameter_b {
            [a, midpoint, b]
        } else {
            [b, midpoint, a]
        };
        let values = [
            coefficients.at(
                boundary.region,
                assembly_point(nodes[0], mesh, midpoint, a, b),
            )?,
            coefficients.at(
                boundary.region,
                assembly_point(nodes[1], mesh, midpoint, a, b),
            )?,
            coefficients.at(
                boundary.region,
                assembly_point(nodes[2], mesh, midpoint, a, b),
            )?,
        ];
        let tangent = (mesh.vertices[b].point - mesh.vertices[a].point) / length;
        assemble_face_condition(
            condition.resolved(scene.physics),
            nodes,
            length,
            values,
            Point2::new(-tangent.y, tangent.x),
            assembly,
        )?;
        let (start, end) = if parameter_a < parameter_b {
            (parameter_a, parameter_b)
        } else {
            (parameter_b, parameter_a)
        };
        let pair = traces
            .entry((id, start.to_bits(), end.to_bits()))
            .or_default();
        let slot = match side {
            InternalBoundarySide::Left => 0,
            InternalBoundarySide::Right => 1,
        };
        if pair[slot].replace((nodes, length)).is_some() {
            return Err(WaveError::InvalidMesh(
                "an internal-boundary trace edge is duplicated",
            ));
        }
    }

    for ((id, start, end), pair) in traces {
        let [Some((left, left_length)), Some((right, right_length))] = pair else {
            return Err(WaveError::InvalidMesh(
                "an internal-boundary segment is missing one face",
            ));
        };
        if (left_length - right_length).abs() > 1.0e-10 * left_length.max(right_length).max(1.0) {
            return Err(WaveError::InvalidMesh(
                "paired internal-boundary faces have different lengths",
            ));
        }
        let boundary = scene
            .internal_boundaries
            .iter()
            .find(|boundary| boundary.id == id)
            .unwrap();
        let parameter = 0.5 * (f64::from_bits(start) + f64::from_bits(end));
        let span = boundary.spline.span_index(parameter).unwrap();
        let InternalBoundaryCoupling::ThinGap { stiffness_ratio } =
            boundary.span_laws[span].coupling
        else {
            continue;
        };
        let point = boundary.spline.evaluate(parameter);
        let spring = stiffness_ratio
            * coefficients
                .at(boundary.region, point)?
                .geometric_mean_stiffness();
        for ((left_node, right_node), weight) in
            left.into_iter()
                .zip(right)
                .zip([1.0 / 6.0, 2.0 / 3.0, 1.0 / 6.0])
        {
            if left_node == right_node {
                continue;
            }
            let scale = spring * 0.5 * (left_length + right_length) * weight;
            *assembly.rows[left_node].entry(left_node).or_default() += scale;
            *assembly.rows[left_node].entry(right_node).or_default() -= scale;
            *assembly.rows[right_node].entry(left_node).or_default() -= scale;
            *assembly.rows[right_node].entry(right_node).or_default() += scale;
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default)]
struct TopologyTracePair {
    traces: [Option<(TraceNodes, RegionId)>; 2],
    stiffness_ratio: f64,
    point: Point2,
}

fn topology_outer_region(
    plan: &TopologyMeshPlan,
    side: OuterSide,
    parameters: [f64; 2],
) -> Result<RegionId, WaveError> {
    let midpoint = 0.5 * (parameters[0] + parameters[1]);
    plan.boundary_at(PlannedBoundarySource::Outer(side), midpoint)
        .ok_or(WaveError::InvalidMesh(
            "a mesh boundary has no unique topology-plan source",
        ))
        .map(|boundary| boundary.region)
}

fn boundary_adjacent_regions(
    mesh: &TriMesh,
    vertices: [usize; 2],
) -> Result<Vec<RegionId>, WaveError> {
    if vertices[0] >= mesh.vertices.len()
        || vertices[1] >= mesh.vertices.len()
        || vertices[0] == vertices[1]
    {
        return Err(WaveError::InvalidMesh(
            "a topology boundary edge has invalid vertex indices",
        ));
    }
    let regions = mesh
        .triangles
        .iter()
        .filter(|triangle| {
            triangle.vertices.contains(&vertices[0]) && triangle.vertices.contains(&vertices[1])
        })
        .map(|triangle| triangle.region)
        .collect::<Vec<_>>();
    if regions.is_empty() || regions.len() > 2 {
        return Err(WaveError::InvalidMesh(
            "a topology boundary edge has invalid adjacency",
        ));
    }
    Ok(regions)
}

fn assemble_topology_boundary_laws(
    mesh: &TriMesh,
    plan: &TopologyMeshPlan,
    physics: PhysicsModel,
    coefficients: CoefficientProvider<'_>,
    assembly: &mut BoundaryAssembly<'_>,
) -> Result<(), WaveError> {
    let mut pairs =
        BTreeMap::<(crate::CurveId, crate::CurveSpanId, u64, u64), TopologyTracePair>::new();
    for edge in &mesh.boundary_edges {
        let BoundaryLabel::Curve {
            curve,
            span,
            side,
            separated,
        } = edge.label
        else {
            continue;
        };
        let [parameter_a, parameter_b] = edge.parameters;
        if !parameter_a.is_finite() || !parameter_b.is_finite() || parameter_a == parameter_b {
            return Err(WaveError::InvalidMesh(
                "a topology curve edge has invalid parameters",
            ));
        }
        let planned = plan
            .boundary_at(
                PlannedBoundarySource::Curve { curve, span, side },
                0.5 * (parameter_a + parameter_b),
            )
            .ok_or(WaveError::InvalidMesh(
                "a mesh boundary has no unique topology-plan source",
            ))?;
        let adjacent = boundary_adjacent_regions(mesh, edge.vertices)?;
        if !separated {
            if planned.behavior != Some(SpanBehavior::Transmitting)
                || adjacent.len() != 2
                || !adjacent.contains(&planned.region)
            {
                return Err(WaveError::InvalidMesh(
                    "a transmitting topology edge has inconsistent adjacency",
                ));
            }
            continue;
        }
        let SpanBehavior::Separated {
            left,
            right,
            coupling,
        } = planned.behavior.ok_or(WaveError::InvalidMesh(
            "a separated topology edge has no span behavior",
        ))?
        else {
            return Err(WaveError::InvalidMesh(
                "a separated mesh edge references a transmitting span",
            ));
        };
        if adjacent.as_slice() != [planned.region] {
            return Err(WaveError::InvalidMesh(
                "a separated topology edge has the wrong face region",
            ));
        }
        let [a, b] = edge.vertices;
        let key = if a < b { (a, b) } else { (b, a) };
        let midpoint = *assembly.edge_nodes.get(&key).ok_or(WaveError::InvalidMesh(
            "a topology boundary edge does not belong to a triangle",
        ))?;
        let length = (mesh.vertices[b].point - mesh.vertices[a].point).norm();
        if !length.is_finite() || length <= 0.0 {
            return Err(WaveError::InvalidMesh(
                "a topology boundary edge has invalid length",
            ));
        }
        let nodes = if parameter_a < parameter_b {
            [a, midpoint, b]
        } else {
            [b, midpoint, a]
        };
        let [value_a, value_midpoint, value_b] = nodes.map(|node| {
            coefficients.at(planned.region, assembly_point(node, mesh, midpoint, a, b))
        });
        let values = [value_a?, value_midpoint?, value_b?];
        let tangent = (mesh.vertices[b].point - mesh.vertices[a].point) / length;
        let condition = match side {
            crate::CurveTraceSide::Left => left,
            crate::CurveTraceSide::Right => right,
        };
        assemble_face_condition(
            condition.resolved(physics),
            nodes,
            length,
            values,
            Point2::new(tangent.y, -tangent.x),
            assembly,
        )?;

        let InternalBoundaryCoupling::ThinGap { stiffness_ratio } = coupling else {
            continue;
        };
        let (start, end) = if parameter_a < parameter_b {
            (parameter_a, parameter_b)
        } else {
            (parameter_b, parameter_a)
        };
        let pair = pairs
            .entry((curve, span, start.to_bits(), end.to_bits()))
            .or_insert(TopologyTracePair {
                traces: [None, None],
                stiffness_ratio,
                point: mesh.vertices[a].point.lerp(mesh.vertices[b].point, 0.5),
            });
        if pair.stiffness_ratio != stiffness_ratio {
            return Err(WaveError::InvalidMesh(
                "paired topology traces disagree on their coupling",
            ));
        }
        let slot = match side {
            crate::CurveTraceSide::Left => 0,
            crate::CurveTraceSide::Right => 1,
        };
        if pair.traces[slot]
            .replace(((nodes, length), planned.region))
            .is_some()
        {
            return Err(WaveError::InvalidMesh(
                "a topology trace edge is duplicated",
            ));
        }
    }

    for (_, pair) in pairs {
        let [
            Some(((left, left_length), left_region)),
            Some(((right, right_length), right_region)),
        ] = pair.traces
        else {
            return Err(WaveError::InvalidMesh(
                "a coupled topology segment is missing one trace",
            ));
        };
        if (left_length - right_length).abs() > 1.0e-10 * left_length.max(right_length).max(1.0) {
            return Err(WaveError::InvalidMesh(
                "paired topology traces have different lengths",
            ));
        }
        let left_stiffness = coefficients
            .at(left_region, pair.point)?
            .geometric_mean_stiffness();
        let right_stiffness = coefficients
            .at(right_region, pair.point)?
            .geometric_mean_stiffness();
        let spring = pair.stiffness_ratio * (left_stiffness * right_stiffness).sqrt();
        for ((left_node, right_node), weight) in
            left.into_iter()
                .zip(right)
                .zip([1.0 / 6.0, 2.0 / 3.0, 1.0 / 6.0])
        {
            if left_node == right_node {
                continue;
            }
            let scale = spring * 0.5 * (left_length + right_length) * weight;
            *assembly.rows[left_node].entry(left_node).or_default() += scale;
            *assembly.rows[left_node].entry(right_node).or_default() -= scale;
            *assembly.rows[right_node].entry(left_node).or_default() -= scale;
            *assembly.rows[right_node].entry(right_node).or_default() += scale;
        }
    }
    Ok(())
}

fn assemble_face_condition(
    condition: FaceBoundaryCondition,
    nodes: [usize; 3],
    length: f64,
    coefficients: [DirectionalWaveCoefficients; 3],
    normal: Point2,
    assembly: &mut BoundaryAssembly<'_>,
) -> Result<(), WaveError> {
    let weights = [1.0 / 6.0, 2.0 / 3.0, 1.0 / 6.0];
    match condition {
        FaceBoundaryCondition::Reflecting => {}
        FaceBoundaryCondition::Impedance { ratio } => {
            for ((node, weight), coefficients) in nodes
                .into_iter()
                .zip(weights)
                .zip(coefficients.iter().copied())
            {
                let impedance = ratio * coefficients.normal_impedance(normal);
                assembly.damping[node] += impedance * length * weight;
            }
        }
        FaceBoundaryCondition::SecondOrderOutgoing => {
            for ((node, weight), coefficients) in nodes
                .into_iter()
                .zip(weights)
                .zip(coefficients.iter().copied())
            {
                let impedance = coefficients.normal_impedance(normal);
                assembly.damping[node] += impedance * length * weight;
            }
            let coefficients = coefficients[1];
            assemble_auxiliary_line(
                nodes,
                length,
                coefficients.stiffness.determinant()
                    / (2.0 * coefficients.normal_impedance(normal)),
                assembly.auxiliary_rows,
                assembly.auxiliary_active,
            );
        }
        FaceBoundaryCondition::Neumann { signal } => {
            for (node, weight) in nodes.into_iter().zip(weights) {
                add_face_neumann_load(assembly.face_neumann_loads, node, signal, length * weight)?;
            }
        }
        FaceBoundaryCondition::Dirichlet { signal } => {
            for node in nodes {
                assign_dirichlet(assembly.dirichlet_signals, node, signal)?;
            }
        }
        FaceBoundaryCondition::ElectricWall | FaceBoundaryCondition::MagneticWall => {
            unreachable!()
        }
    }
    Ok(())
}

fn outer_normal(side: OuterSide) -> Point2 {
    match side {
        OuterSide::Bottom => Point2::new(0.0, -1.0),
        OuterSide::Right => Point2::new(1.0, 0.0),
        OuterSide::Top => Point2::new(0.0, 1.0),
        OuterSide::Left => Point2::new(-1.0, 0.0),
    }
}

fn assembly_point(node: usize, mesh: &TriMesh, midpoint: usize, a: usize, b: usize) -> Point2 {
    if node == midpoint {
        (mesh.vertices[a].point + mesh.vertices[b].point) / 2.0
    } else {
        mesh.vertices[node].point
    }
}

/// Phases of one operator assembly. Numbering and element work step one
/// triangle at a time and row compression one row at a time, so a job can
/// stop after any step; the boundary and finishing passes are single steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AssemblyPhase {
    Numbering(usize),
    Elements(usize),
    Boundary,
    Csr(usize),
    Finish,
    Done,
}

impl AssemblyPhase {
    const fn label(self) -> &'static str {
        match self {
            Self::Numbering(_) => "Numbering wave nodes",
            Self::Elements(_) => "Assembling wave elements",
            Self::Boundary => "Assembling boundary laws",
            Self::Csr(_) => "Compressing the wave operator",
            Self::Finish => "Bounding the time step",
            Self::Done => "Finished",
        }
    }
}

/// Resumable state of `QuadraticWaveOperator` assembly. `assemble_with_provider`
/// drives it to completion in one call and `QuadraticAssemblyJob` spreads the
/// same steps across frames, so the two paths produce equal operators.
struct QuadraticAssemblyWork {
    phase: AssemblyPhase,
    outer_boundaries: OuterBoundaryConditions,
    node_points: Vec<Point2>,
    edge_nodes: BTreeMap<(usize, usize), usize>,
    local_nodes: Vec<[usize; 7]>,
    rows: Vec<BTreeMap<usize, f64>>,
    auxiliary_rows: Vec<BTreeMap<usize, f64>>,
    auxiliary_active: Vec<bool>,
    dirichlet_sides: Vec<Option<OuterSide>>,
    dirichlet_signals: Vec<Option<TimeSignal>>,
    neumann_weights: Vec<[f64; 4]>,
    face_neumann_loads: Vec<[BoundaryLoad; 2]>,
    mass: Vec<f64>,
    damping: Vec<f64>,
    maximum_wave_speed: f64,
    row_offsets: Vec<u32>,
    columns: Vec<u32>,
    stiffness: Vec<f64>,
    auxiliary_stiffness: Vec<f64>,
    maximum_eigenvalue_bound: f64,
}

impl QuadraticAssemblyWork {
    fn new(
        outer_boundaries: OuterBoundaryConditions,
        physics: PhysicsModel,
    ) -> Result<Self, WaveError> {
        if !outer_boundaries.valid() {
            return Err(WaveError::InvalidCoefficients);
        }
        Ok(Self {
            phase: AssemblyPhase::Numbering(0),
            outer_boundaries: OuterBoundaryConditions {
                sides: outer_boundaries
                    .sides
                    .map(|condition| condition.resolved(physics)),
            },
            node_points: Vec::new(),
            edge_nodes: BTreeMap::new(),
            local_nodes: Vec::new(),
            rows: Vec::new(),
            auxiliary_rows: Vec::new(),
            auxiliary_active: Vec::new(),
            dirichlet_sides: Vec::new(),
            dirichlet_signals: Vec::new(),
            neumann_weights: Vec::new(),
            face_neumann_loads: Vec::new(),
            mass: Vec::new(),
            damping: Vec::new(),
            maximum_wave_speed: 0.0,
            row_offsets: Vec::new(),
            columns: Vec::new(),
            stiffness: Vec::new(),
            auxiliary_stiffness: Vec::new(),
            maximum_eigenvalue_bound: 0.0,
        })
    }

    fn step(
        &mut self,
        mesh: &TriMesh,
        coefficients: CoefficientProvider<'_>,
        scene: Option<&Scene>,
        topology: Option<&TopologyMeshPlan>,
        physics: PhysicsModel,
    ) -> Result<Option<QuadraticWaveOperator>, WaveError> {
        match self.phase {
            AssemblyPhase::Numbering(index) => self.number(mesh, index)?,
            AssemblyPhase::Elements(index) => self.element(mesh, coefficients, index)?,
            AssemblyPhase::Boundary => {
                self.boundaries(mesh, coefficients, scene, topology, physics)?;
            }
            AssemblyPhase::Csr(row) => self.compress(row)?,
            AssemblyPhase::Finish => return self.finish(mesh).map(Some),
            AssemblyPhase::Done => {}
        }
        Ok(None)
    }

    /// Numbers the seven nodes of triangle `index`: vertices, edge midpoints
    /// shared with neighbours, and the interior bubble.
    fn number(&mut self, mesh: &TriMesh, index: usize) -> Result<(), WaveError> {
        if index == 0 {
            if mesh.vertices.is_empty() || mesh.triangles.is_empty() {
                return Err(WaveError::InvalidMesh("the mesh is empty"));
            }
            self.node_points = mesh.vertices.iter().map(|vertex| vertex.point).collect();
            if self.node_points.iter().any(|point| !point.finite()) {
                return Err(WaveError::InvalidMesh("a vertex is non-finite"));
            }
            self.local_nodes = Vec::with_capacity(mesh.triangles.len());
        }
        if index == mesh.triangles.len() {
            if self.node_points.len() > u32::MAX as usize {
                return Err(WaveError::InvalidMesh(
                    "the quadratic operator has too many degrees of freedom",
                ));
            }
            let count = self.node_points.len();
            self.rows = vec![BTreeMap::new(); count];
            self.auxiliary_rows = vec![BTreeMap::new(); count];
            self.auxiliary_active = vec![false; count];
            self.dirichlet_sides = vec![None; count];
            self.dirichlet_signals = vec![None; count];
            self.neumann_weights = vec![[0.0; 4]; count];
            self.face_neumann_loads = vec![[BoundaryLoad::default(); 2]; count];
            self.mass = vec![0.0; count];
            self.damping = vec![0.0; count];
            self.phase = AssemblyPhase::Elements(0);
            return Ok(());
        }
        let [a, b, c] = mesh.triangles[index].vertices;
        if a >= mesh.vertices.len()
            || b >= mesh.vertices.len()
            || c >= mesh.vertices.len()
            || a == b
            || b == c
            || c == a
        {
            return Err(WaveError::InvalidMesh(
                "a triangle has invalid vertex indices",
            ));
        }
        let points = [
            self.node_points[a],
            self.node_points[b],
            self.node_points[c],
        ];
        let twice_area = (points[1] - points[0]).cross(points[2] - points[0]);
        if !twice_area.is_finite() || twice_area <= 0.0 {
            return Err(WaveError::InvalidMesh("a triangle has non-positive area"));
        }
        let mut edge = [0; 3];
        for (slot, pair) in [[a, b], [b, c], [c, a]].into_iter().enumerate() {
            let key = if pair[0] < pair[1] {
                (pair[0], pair[1])
            } else {
                (pair[1], pair[0])
            };
            let node_points = &mut self.node_points;
            edge[slot] = *self.edge_nodes.entry(key).or_insert_with(|| {
                let index = node_points.len();
                node_points.push((node_points[key.0] + node_points[key.1]) / 2.0);
                index
            });
        }
        let centroid = self.node_points.len();
        self.node_points
            .push((points[0] + points[1] + points[2]) / 3.0);
        self.local_nodes
            .push([a, b, c, edge[0], edge[1], edge[2], centroid]);
        self.phase = AssemblyPhase::Numbering(index + 1);
        Ok(())
    }

    /// Adds triangle `index`'s lumped mass, damping, and stiffness.
    fn element(
        &mut self,
        mesh: &TriMesh,
        coefficients: CoefficientProvider<'_>,
        index: usize,
    ) -> Result<(), WaveError> {
        if index == mesh.triangles.len() {
            self.phase = AssemblyPhase::Boundary;
            return Ok(());
        }
        let triangle = mesh.triangles[index];
        let indices = self.local_nodes[index];
        let points = triangle.vertices.map(|index| mesh.vertices[index].point);
        let twice_area = (points[1] - points[0]).cross(points[2] - points[0]);
        let area = 0.5 * twice_area;
        let barycentric_gradients = [
            Point2::new(points[1].y - points[2].y, points[2].x - points[1].x) / twice_area,
            Point2::new(points[2].y - points[0].y, points[0].x - points[2].x) / twice_area,
            Point2::new(points[0].y - points[1].y, points[1].x - points[0].x) / twice_area,
        ];

        const MASS_WEIGHTS: [f64; 7] = [
            1.0 / 20.0,
            1.0 / 20.0,
            1.0 / 20.0,
            2.0 / 15.0,
            2.0 / 15.0,
            2.0 / 15.0,
            9.0 / 20.0,
        ];
        for local in 0..7 {
            let values = coefficients.at(triangle.region, self.node_points[indices[local]])?;
            self.maximum_wave_speed = self.maximum_wave_speed.max(values.maximum_wave_speed());
            self.mass[indices[local]] += values.mass_density * area * MASS_WEIGHTS[local];
            self.damping[indices[local]] += values.damping * area * MASS_WEIGHTS[local];
        }

        let mut local_stiffness = [[0.0; 7]; 7];
        for (barycentric, weight) in stiffness_quadrature() {
            let point = points[0] * barycentric[0]
                + points[1] * barycentric[1]
                + points[2] * barycentric[2];
            let values = coefficients.at(triangle.region, point)?;
            self.maximum_wave_speed = self.maximum_wave_speed.max(values.maximum_wave_speed());
            let gradients = enriched_quadratic_basis_gradients(barycentric, barycentric_gradients);
            for i in 0..7 {
                for j in 0..7 {
                    local_stiffness[i][j] +=
                        weight * gradients[i].dot(values.stiffness.apply(gradients[j]));
                }
            }
        }
        for i in 0..7 {
            for j in 0..7 {
                *self.rows[indices[i]].entry(indices[j]).or_default() +=
                    area * local_stiffness[i][j];
            }
        }
        self.phase = AssemblyPhase::Elements(index + 1);
        Ok(())
    }

    /// Applies the outer, hole, baffle, and topology boundary laws, normalises
    /// the boundary loads by the lumped mass, and sizes the compressed rows.
    fn boundaries(
        &mut self,
        mesh: &TriMesh,
        coefficients: CoefficientProvider<'_>,
        scene: Option<&Scene>,
        topology: Option<&TopologyMeshPlan>,
        physics: PhysicsModel,
    ) -> Result<(), WaveError> {
        let mut visited = BTreeSet::new();
        for boundary in &mesh.boundary_edges {
            let BoundaryLabel::Outer(side) = boundary.label else {
                continue;
            };
            let condition = self.outer_boundaries.get(side);
            let [a, b] = boundary.vertices;
            if a >= mesh.vertices.len() || b >= mesh.vertices.len() || a == b {
                return Err(WaveError::InvalidMesh(
                    "an outer boundary edge has invalid vertex indices",
                ));
            }
            let key = if a < b { (a, b) } else { (b, a) };
            if !visited.insert(key) {
                return Err(WaveError::InvalidMesh(
                    "an outer boundary edge is duplicated",
                ));
            }
            let midpoint = *self.edge_nodes.get(&key).ok_or(WaveError::InvalidMesh(
                "an outer boundary edge does not belong to a triangle",
            ))?;
            let length = (mesh.vertices[b].point - mesh.vertices[a].point).norm();
            if !length.is_finite() || length <= 0.0 {
                return Err(WaveError::InvalidMesh(
                    "an outer boundary edge has invalid length",
                ));
            }
            let nodes = [a, b, midpoint];
            let region = if let Some(plan) = topology {
                topology_outer_region(plan, side, boundary.parameters)?
            } else {
                BACKGROUND_REGION
            };
            let node_coefficients = [
                coefficients.at(region, self.node_points[nodes[0]])?,
                coefficients.at(region, self.node_points[nodes[1]])?,
                coefficients.at(region, self.node_points[nodes[2]])?,
            ];
            let line_weights = [length / 6.0, length / 6.0, 2.0 * length / 3.0];
            let normal = outer_normal(side);
            match condition {
                OuterBoundaryCondition::Reflecting => {}
                OuterBoundaryCondition::Neumann { .. } => {
                    for (node, weight) in nodes.into_iter().zip(line_weights) {
                        self.neumann_weights[node][side.index()] += weight;
                    }
                }
                OuterBoundaryCondition::Dirichlet { signal } => {
                    for node in nodes {
                        if let Some(previous_side) = self.dirichlet_sides[node]
                            && self.outer_boundaries.get(previous_side).signal() != Some(signal)
                        {
                            return Err(WaveError::InvalidMesh(
                                "adjacent Dirichlet sides disagree at their shared corner",
                            ));
                        }
                        assign_dirichlet(&mut self.dirichlet_signals, node, signal)?;
                        self.dirichlet_sides[node] = Some(side);
                    }
                }
                OuterBoundaryCondition::FirstOrderOutgoing
                | OuterBoundaryCondition::SecondOrderOutgoing => {
                    for ((node, weight), values) in
                        nodes.into_iter().zip(line_weights).zip(node_coefficients)
                    {
                        let impedance = values.normal_impedance(normal);
                        self.damping[node] += impedance * weight;
                    }
                }
                OuterBoundaryCondition::ElectricWall | OuterBoundaryCondition::MagneticWall => {
                    unreachable!()
                }
            }
            if condition == OuterBoundaryCondition::SecondOrderOutgoing {
                // P2 line-element stiffness in endpoint/endpoint/midpoint order.
                // Sharing vertex indices across incident sides supplies the
                // corner coupling in the assembled tangential operator.
                let middle = node_coefficients[2];
                assemble_auxiliary_line(
                    nodes,
                    length,
                    middle.stiffness.determinant() / (2.0 * middle.normal_impedance(normal)),
                    &mut self.auxiliary_rows,
                    &mut self.auxiliary_active,
                );
            }
        }
        if let Some(scene) = scene {
            let mut assembly = BoundaryAssembly {
                edge_nodes: &self.edge_nodes,
                rows: &mut self.rows,
                auxiliary_rows: &mut self.auxiliary_rows,
                auxiliary_active: &mut self.auxiliary_active,
                dirichlet_signals: &mut self.dirichlet_signals,
                face_neumann_loads: &mut self.face_neumann_loads,
                damping: &mut self.damping,
            };
            assemble_hole_boundary_conditions(mesh, scene, coefficients, &mut assembly)?;
            assemble_internal_boundary_laws(mesh, scene, coefficients, &mut assembly)?;
        }
        if let Some(plan) = topology {
            let mut assembly = BoundaryAssembly {
                edge_nodes: &self.edge_nodes,
                rows: &mut self.rows,
                auxiliary_rows: &mut self.auxiliary_rows,
                auxiliary_active: &mut self.auxiliary_active,
                dirichlet_signals: &mut self.dirichlet_signals,
                face_neumann_loads: &mut self.face_neumann_loads,
                damping: &mut self.damping,
            };
            assemble_topology_boundary_laws(mesh, plan, physics, coefficients, &mut assembly)?;
        }
        if self
            .mass
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
            || self
                .damping
                .iter()
                .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err(WaveError::InvalidMesh(
                "a quadratic node has invalid lumped mass",
            ));
        }
        for (weights, mass) in self.neumann_weights.iter_mut().zip(&self.mass) {
            for weight in weights {
                *weight /= *mass;
            }
        }
        for (loads, mass) in self.face_neumann_loads.iter_mut().zip(&self.mass) {
            for load in loads {
                load.normalized_weight /= *mass;
            }
        }

        for (row, auxiliary) in self.rows.iter_mut().zip(&self.auxiliary_rows) {
            for column in auxiliary.keys() {
                row.entry(*column).or_default();
            }
        }
        let entries = self.rows.iter().map(BTreeMap::len).sum::<usize>();
        if entries > u32::MAX as usize {
            return Err(WaveError::InvalidMesh(
                "the quadratic operator has too many matrix entries",
            ));
        }
        self.row_offsets = Vec::with_capacity(self.rows.len() + 1);
        self.row_offsets.push(0);
        self.columns = Vec::with_capacity(entries);
        self.stiffness = Vec::with_capacity(entries);
        self.auxiliary_stiffness = Vec::with_capacity(entries);
        self.phase = AssemblyPhase::Csr(0);
        Ok(())
    }

    /// Compresses row `row` into the CSR arrays and folds it into the
    /// Gershgorin bound.
    fn compress(&mut self, row: usize) -> Result<(), WaveError> {
        if row == self.rows.len() {
            self.phase = AssemblyPhase::Finish;
            return Ok(());
        }
        let values = std::mem::take(&mut self.rows[row]);
        self.maximum_eigenvalue_bound = self
            .maximum_eigenvalue_bound
            .max(values.values().map(|value| value.abs()).sum::<f64>() / self.mass[row]);
        for (column, value) in values {
            if !value.is_finite() {
                return Err(WaveError::InvalidMesh("the stiffness matrix is non-finite"));
            }
            self.columns.push(column as u32);
            self.stiffness.push(value);
            self.auxiliary_stiffness.push(
                self.auxiliary_rows[row]
                    .get(&column)
                    .copied()
                    .unwrap_or_default(),
            );
        }
        self.row_offsets.push(self.columns.len() as u32);
        self.phase = AssemblyPhase::Csr(row + 1);
        Ok(())
    }

    fn finish(&mut self, mesh: &TriMesh) -> Result<QuadraticWaveOperator, WaveError> {
        if !self.maximum_eigenvalue_bound.is_finite() || self.maximum_eigenvalue_bound <= 0.0 {
            return Err(WaveError::InvalidMesh(
                "the quadratic stiffness bound is not positive",
            ));
        }
        let maximum_time_step = 2.0 / self.maximum_eigenvalue_bound.sqrt();
        let minimum_edge_length = mesh
            .triangles
            .iter()
            .flat_map(|triangle| {
                let points = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
                [
                    (points[1] - points[0]).norm(),
                    (points[2] - points[1]).norm(),
                    (points[0] - points[2]).norm(),
                ]
            })
            .fold(f64::INFINITY, f64::min);
        if !minimum_edge_length.is_finite()
            || !self.maximum_wave_speed.is_finite()
            || self.maximum_wave_speed <= 0.0
            || maximum_time_step < minimum_edge_length / self.maximum_wave_speed * 1.0e-6
        {
            return Err(WaveError::InvalidMesh(
                "a near-degenerate element collapses the explicit CFL timestep",
            ));
        }
        let element_nodes = std::mem::take(&mut self.local_nodes)
            .into_iter()
            .map(|indices| indices.map(|index| index as u32))
            .collect();
        self.phase = AssemblyPhase::Done;
        Ok(QuadraticWaveOperator {
            geometry_revision: mesh.geometry_revision,
            mesh_revision: mesh.mesh_revision,
            outer_boundaries: self.outer_boundaries,
            node_points: std::mem::take(&mut self.node_points),
            element_nodes,
            row_offsets: std::mem::take(&mut self.row_offsets),
            columns: std::mem::take(&mut self.columns),
            stiffness: std::mem::take(&mut self.stiffness),
            auxiliary_stiffness: std::mem::take(&mut self.auxiliary_stiffness),
            auxiliary_active: std::mem::take(&mut self.auxiliary_active),
            dirichlet_sides: std::mem::take(&mut self.dirichlet_sides),
            dirichlet_signals: std::mem::take(&mut self.dirichlet_signals),
            normalized_neumann_weights: std::mem::take(&mut self.neumann_weights),
            face_neumann_loads: std::mem::take(&mut self.face_neumann_loads),
            lumped_mass: std::mem::take(&mut self.mass),
            lumped_damping: std::mem::take(&mut self.damping),
            maximum_eigenvalue_bound: self.maximum_eigenvalue_bound,
            maximum_time_step,
        })
    }
}

/// Assembles a topology operator across frames. It owns everything it reads,
/// so a preparation can hold it while the document keeps changing.
pub struct QuadraticAssemblyJob {
    mesh: Arc<TriMesh>,
    plan: Arc<TopologyMeshPlan>,
    model: OwnedTopologyWaveModel,
    work: QuadraticAssemblyWork,
    done: bool,
}

impl QuadraticAssemblyJob {
    pub fn new_topology(
        mesh: Arc<TriMesh>,
        plan: Arc<TopologyMeshPlan>,
        model: TopologyWaveModel<'_>,
    ) -> Result<Self, WaveError> {
        if !model.valid_for(&plan) {
            return Err(WaveError::InvalidCoefficients);
        }
        let work = QuadraticAssemblyWork::new(model.outer_boundaries, model.physics)?;
        Ok(Self {
            mesh,
            plan,
            model: OwnedTopologyWaveModel {
                physics: model.physics,
                materials: model.materials.to_vec(),
                regions: model.regions.to_vec(),
                outer_boundaries: model.outer_boundaries,
            },
            work,
            done: false,
        })
    }

    pub fn phase(&self) -> &'static str {
        self.work.phase.label()
    }

    /// Runs up to `budget` steps. `Some` carries the finished operator or the
    /// first error, after which the job is spent.
    pub fn advance(&mut self, budget: usize) -> Option<Result<QuadraticWaveOperator, WaveError>> {
        for _ in 0..budget {
            if self.done {
                return None;
            }
            let model = self.model.as_model();
            match self.work.step(
                &self.mesh,
                CoefficientProvider::Topology(model),
                None,
                Some(&self.plan),
                model.physics,
            ) {
                Ok(Some(operator)) => {
                    self.done = true;
                    return Some(Ok(operator));
                }
                Ok(None) => {}
                Err(error) => {
                    self.done = true;
                    return Some(Err(error));
                }
            }
        }
        None
    }
}

fn assemble_auxiliary_line(
    nodes: [usize; 3],
    length: f64,
    scale: f64,
    auxiliary_rows: &mut [BTreeMap<usize, f64>],
    auxiliary_active: &mut [bool],
) {
    let local = [[7.0, 1.0, -8.0], [1.0, 7.0, -8.0], [-8.0, -8.0, 16.0]];
    for i in 0..3 {
        auxiliary_active[nodes[i]] = true;
        for j in 0..3 {
            *auxiliary_rows[nodes[i]].entry(nodes[j]).or_default() +=
                scale * local[i][j] / (3.0 * length);
        }
    }
}

fn assign_dirichlet(
    signals: &mut [Option<TimeSignal>],
    node: usize,
    signal: TimeSignal,
) -> Result<(), WaveError> {
    if let Some(previous) = signals[node]
        && previous != signal
    {
        return Err(WaveError::InvalidMesh(
            "Dirichlet conditions disagree at a shared boundary node",
        ));
    }
    signals[node] = Some(signal);
    Ok(())
}

fn add_face_neumann_load(
    loads: &mut [[BoundaryLoad; 2]],
    node: usize,
    signal: TimeSignal,
    weight: f64,
) -> Result<(), WaveError> {
    if let Some(load) = loads[node]
        .iter_mut()
        .find(|load| load.normalized_weight == 0.0 || load.signal == signal)
    {
        if load.normalized_weight == 0.0 {
            load.signal = signal;
        }
        load.normalized_weight += weight;
        Ok(())
    } else {
        Err(WaveError::InvalidMesh(
            "more than two Neumann signals meet at one boundary node",
        ))
    }
}

fn validate_coefficients(coefficients: WaveCoefficients) -> Result<(), WaveError> {
    if !coefficients.mass_density.is_finite()
        || coefficients.mass_density <= 0.0
        || !coefficients.stiffness.is_finite()
        || coefficients.stiffness <= 0.0
        || !coefficients.damping.is_finite()
        || coefficients.damping < 0.0
    {
        Err(WaveError::InvalidCoefficients)
    } else {
        Ok(())
    }
}

pub fn enriched_quadratic_basis_gradients(
    barycentric: [f64; 3],
    gradients: [Point2; 3],
) -> [Point2; 7] {
    let [l0, l1, l2] = barycentric;
    let [g0, g1, g2] = gradients;
    let bubble_gradient = (g0 * (l1 * l2) + g1 * (l0 * l2) + g2 * (l0 * l1)) * 27.0;
    [
        g0 * (4.0 * l0 - 1.0) + bubble_gradient / 9.0,
        g1 * (4.0 * l1 - 1.0) + bubble_gradient / 9.0,
        g2 * (4.0 * l2 - 1.0) + bubble_gradient / 9.0,
        (g0 * l1 + g1 * l0) * 4.0 - bubble_gradient * (4.0 / 9.0),
        (g1 * l2 + g2 * l1) * 4.0 - bubble_gradient * (4.0 / 9.0),
        (g2 * l0 + g0 * l2) * 4.0 - bubble_gradient * (4.0 / 9.0),
        bubble_gradient,
    ]
}

/// Laplacians of the seven enriched-quadratic basis functions. Barycentric
/// coordinate gradients are constant on an affine triangle.
pub fn enriched_quadratic_basis_laplacians(
    [l0, l1, l2]: [f64; 3],
    [g0, g1, g2]: [Point2; 3],
) -> [f64; 7] {
    let bubble = 54.0 * (l0 * g1.dot(g2) + l1 * g0.dot(g2) + l2 * g0.dot(g1));
    [
        4.0 * g0.dot(g0) + bubble / 9.0,
        4.0 * g1.dot(g1) + bubble / 9.0,
        4.0 * g2.dot(g2) + bubble / 9.0,
        8.0 * g0.dot(g1) - 4.0 * bubble / 9.0,
        8.0 * g1.dot(g2) - 4.0 * bubble / 9.0,
        8.0 * g2.dot(g0) - 4.0 * bubble / 9.0,
        bubble,
    ]
}

/// Hessians of the seven enriched-quadratic basis functions.
pub fn enriched_quadratic_basis_hessians(
    [l0, l1, l2]: [f64; 3],
    [g0, g1, g2]: [Point2; 3],
) -> [SymmetricTensor2; 7] {
    fn outer(a: Point2, b: Point2) -> SymmetricTensor2 {
        SymmetricTensor2::new(a.x * b.x, 0.5 * (a.x * b.y + a.y * b.x), a.y * b.y)
    }
    let bubble = SymmetricTensor2::new(
        54.0 * (l0 * g1.x * g2.x + l1 * g0.x * g2.x + l2 * g0.x * g1.x),
        27.0 * (l0 * (g1.x * g2.y + g1.y * g2.x)
            + l1 * (g0.x * g2.y + g0.y * g2.x)
            + l2 * (g0.x * g1.y + g0.y * g1.x)),
        54.0 * (l0 * g1.y * g2.y + l1 * g0.y * g2.y + l2 * g0.y * g1.y),
    );
    let add = |a: SymmetricTensor2, b: SymmetricTensor2| {
        SymmetricTensor2::new(a.xx + b.xx, a.xy + b.xy, a.yy + b.yy)
    };
    let scale = |a: SymmetricTensor2, s: f64| SymmetricTensor2::new(a.xx * s, a.xy * s, a.yy * s);
    [
        add(scale(outer(g0, g0), 4.0), scale(bubble, 1.0 / 9.0)),
        add(scale(outer(g1, g1), 4.0), scale(bubble, 1.0 / 9.0)),
        add(scale(outer(g2, g2), 4.0), scale(bubble, 1.0 / 9.0)),
        add(scale(outer(g0, g1), 8.0), scale(bubble, -4.0 / 9.0)),
        add(scale(outer(g1, g2), 8.0), scale(bubble, -4.0 / 9.0)),
        add(scale(outer(g2, g0), 8.0), scale(bubble, -4.0 / 9.0)),
        bubble,
    ]
}

/// Cardinal basis values for the seven local nodes in the order returned by
/// [`QuadraticWaveOperator::element_nodes`].
pub fn enriched_quadratic_basis([l0, l1, l2]: [f64; 3]) -> [f64; 7] {
    let bubble = 27.0 * l0 * l1 * l2;
    [
        l0 * (2.0 * l0 - 1.0) + bubble / 9.0,
        l1 * (2.0 * l1 - 1.0) + bubble / 9.0,
        l2 * (2.0 * l2 - 1.0) + bubble / 9.0,
        4.0 * l0 * l1 - 4.0 * bubble / 9.0,
        4.0 * l1 * l2 - 4.0 * bubble / 9.0,
        4.0 * l2 * l0 - 4.0 * bubble / 9.0,
        bubble,
    ]
}

fn stiffness_quadrature() -> [([f64; 3], f64); 6] {
    const A: f64 = 0.445_948_490_915_965;
    const B: f64 = 0.108_103_018_168_070;
    const C: f64 = 0.091_576_213_509_771;
    const D: f64 = 0.816_847_572_980_459;
    const W0: f64 = 0.223_381_589_678_011;
    const W1: f64 = 0.109_951_743_655_322;
    [
        ([A, A, B], W0),
        ([A, B, A], W0),
        ([B, A, A], W0),
        ([C, C, D], W1),
        ([C, D, C], W1),
        ([D, C, C], W1),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BACKGROUND_REGION, BoundaryEdge, CurveId, CurveNode, CurveSpan, CurveSpanId, CurveSpline,
        FaceRegionAssignment, InternalBoundary, InternalBoundaryLaw, Material, MaterialId,
        MeshQuality, MeshTriangle, MeshVertex, MeshingOptions, Obstacle, ObstacleId,
        OpenCubicSpline, OuterSide, PeriodicCubicSpline, Region, TopologyCurve, TopologyGeometry,
        TopologyVertex, TopologyVertexId, TopologyVertexLocation, compile_topology, mesh_scene,
        mesh_topology_plan,
    };

    fn topology_span(id: u64, behavior: SpanBehavior) -> CurveSpan {
        CurveSpan {
            id: CurveSpanId(id),
            behavior,
        }
    }

    fn topology_fixture(
        curves: Vec<TopologyCurve>,
        vertices: Vec<TopologyVertex>,
        revision: u64,
    ) -> (TopologyMeshPlan, TriMesh, Scene) {
        let snapshot = compile_topology(
            &TopologyGeometry {
                curves,
                vertices,
                ..TopologyGeometry::default()
            },
            revision,
        )
        .unwrap();
        let assignments = snapshot
            .faces
            .iter()
            .enumerate()
            .map(|(index, face)| FaceRegionAssignment {
                face: face.id,
                region: Some(RegionId(index as u64 + 1)),
            })
            .collect::<Vec<_>>();
        let plan = TopologyMeshPlan::new(&snapshot, &assignments).unwrap();
        let mesh = mesh_topology_plan(
            &plan,
            revision + 100,
            MeshingOptions {
                target_edge_length: 0.3,
                minimum_angle_degrees: 8.0,
                max_vertices: 20_000,
                max_triangles: 40_000,
                max_refinement_steps: 20_000,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let scene = Scene {
            regions: assignments
                .iter()
                .map(|assignment| Region {
                    id: assignment.region.unwrap(),
                    material: crate::DEFAULT_MATERIAL,
                    frame: crate::MaterialFrame::world(),
                })
                .collect(),
            ..Scene::default()
        };
        (plan, mesh, scene)
    }

    #[test]
    fn topology_solver_assembles_a_four_region_crossing_junction() {
        let ids = [
            TopologyVertexId(1),
            TopologyVertexId(2),
            TopologyVertexId(3),
            TopologyVertexId(4),
        ];
        let mut horizontal = TopologyCurve::new(
            CurveId(1),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![Point2::new(-1.0, 0.0), Point2::new(1.0, 0.0)])
                    .unwrap(),
            ),
            vec![topology_span(1, SpanBehavior::Transmitting)],
        )
        .unwrap();
        horizontal.nodes = vec![
            CurveNode {
                vertex: Some(ids[0]),
            },
            CurveNode {
                vertex: Some(ids[1]),
            },
        ];
        let mut vertical = TopologyCurve::new(
            CurveId(2),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![Point2::new(0.0, -1.0), Point2::new(0.0, 1.0)])
                    .unwrap(),
            ),
            vec![topology_span(2, SpanBehavior::Transmitting)],
        )
        .unwrap();
        vertical.nodes = vec![
            CurveNode {
                vertex: Some(ids[2]),
            },
            CurveNode {
                vertex: Some(ids[3]),
            },
        ];
        let vertices = vec![
            TopologyVertex {
                id: ids[0],
                location: TopologyVertexLocation::Outer {
                    side: OuterSide::Left,
                    fraction: 0.5,
                },
            },
            TopologyVertex {
                id: ids[1],
                location: TopologyVertexLocation::Outer {
                    side: OuterSide::Right,
                    fraction: 0.5,
                },
            },
            TopologyVertex {
                id: ids[2],
                location: TopologyVertexLocation::Outer {
                    side: OuterSide::Bottom,
                    fraction: 0.5,
                },
            },
            TopologyVertex {
                id: ids[3],
                location: TopologyVertexLocation::Outer {
                    side: OuterSide::Top,
                    fraction: 0.5,
                },
            },
        ];
        let (plan, mesh, scene) = topology_fixture(vec![horizontal, vertical], vertices, 30);
        assert_eq!(plan.domains.len(), 4);
        let center = mesh
            .vertices
            .iter()
            .position(|vertex| vertex.point == Point2::new(0.0, 0.0))
            .unwrap();
        let incident_regions = mesh
            .triangles
            .iter()
            .filter(|triangle| triangle.vertices.contains(&center))
            .map(|triangle| triangle.region)
            .collect::<BTreeSet<_>>();
        assert_eq!(incident_regions.len(), 4);

        let operator = QuadraticWaveOperator::assemble_topology(
            &mesh,
            &plan,
            TopologyWaveModel::from_scene(&scene),
        )
        .unwrap();
        let legacy = QuadraticWaveOperator::assemble_with_provider(
            &mesh,
            CoefficientProvider::Scene(&scene),
            scene.outer_boundaries,
            Some(&scene),
            None,
            scene.physics,
        )
        .unwrap();
        assert_eq!(operator, legacy);
        let force = operator
            .apply_stiffness(&vec![1.0; operator.degrees_of_freedom()])
            .unwrap();
        assert!(force.iter().all(|value| value.abs() < 3.0e-11));
        assert!(operator.lumped_mass()[center] > 0.0);
    }

    fn topology_baffle(
        behavior: SpanBehavior,
        revision: u64,
    ) -> (TopologyMeshPlan, TriMesh, Scene) {
        let curve = TopologyCurve::new(
            CurveId(8),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![
                    Point2::new(-0.6, 0.0),
                    Point2::new(0.0, 0.0),
                    Point2::new(0.6, 0.0),
                ])
                .unwrap(),
            ),
            vec![topology_span(80, behavior), topology_span(81, behavior)],
        )
        .unwrap();
        topology_fixture(vec![curve], vec![], revision)
    }

    #[test]
    fn topology_solver_applies_curve_side_laws_without_remeshing() {
        let signal = TimeSignal::harmonic(0.4, 0.2, 1.5, 0.1);
        let behavior = SpanBehavior::Separated {
            left: FaceBoundaryCondition::Impedance { ratio: 2.0 },
            right: FaceBoundaryCondition::Neumann { signal },
            coupling: InternalBoundaryCoupling::Independent,
        };
        let (plan, mesh, scene) = topology_baffle(behavior, 40);
        let driven = QuadraticWaveOperator::assemble_topology(
            &mesh,
            &plan,
            TopologyWaveModel::from_scene(&scene),
        )
        .unwrap();
        assert!(driven.lumped_damping().iter().sum::<f64>() > 0.0);
        assert!(
            driven
                .face_neumann_loads()
                .iter()
                .flatten()
                .any(|load| { load.signal == signal && load.normalized_weight > 0.0 })
        );

        let mut reflecting_plan = plan.clone();
        reflecting_plan.geometry_revision += 1;
        for boundary in &mut reflecting_plan.boundaries {
            if let Some(SpanBehavior::Separated { left, right, .. }) = &mut boundary.behavior {
                *left = FaceBoundaryCondition::Reflecting;
                *right = FaceBoundaryCondition::Reflecting;
            }
        }
        assert_eq!(
            crate::topology_mesh_update_action(&plan, &reflecting_plan),
            crate::TopologyMeshUpdateAction::Reuse
        );
        let reflecting = QuadraticWaveOperator::assemble_topology(
            &mesh,
            &reflecting_plan,
            TopologyWaveModel::from_scene(&scene),
        )
        .unwrap();
        assert_eq!(reflecting.geometry_revision(), mesh.geometry_revision);
        assert_eq!(reflecting.mesh_revision(), mesh.mesh_revision);
        assert!(
            reflecting.lumped_damping().iter().sum::<f64>()
                < driven.lumped_damping().iter().sum::<f64>()
        );
        assert!(
            reflecting
                .face_neumann_loads()
                .iter()
                .flatten()
                .all(|load| load.normalized_weight == 0.0)
        );
    }

    #[test]
    fn topology_solver_couples_paired_traces_with_a_thin_gap() {
        let (reflecting_plan, mesh, scene) = topology_baffle(SpanBehavior::REFLECTING, 50);
        let reflecting = QuadraticWaveOperator::assemble_topology(
            &mesh,
            &reflecting_plan,
            TopologyWaveModel::from_scene(&scene),
        )
        .unwrap();
        let mut gap_plan = reflecting_plan.clone();
        for boundary in &mut gap_plan.boundaries {
            if let Some(SpanBehavior::Separated { coupling, .. }) = &mut boundary.behavior {
                *coupling = InternalBoundaryCoupling::ThinGap {
                    stiffness_ratio: 1000.0,
                };
            }
        }
        let gap = QuadraticWaveOperator::assemble_topology(
            &mesh,
            &gap_plan,
            TopologyWaveModel::from_scene(&scene),
        )
        .unwrap();
        let trace_node = |side| {
            let edge = mesh
                .boundary_edges
                .iter()
                .find(|edge| {
                    matches!(
                        edge.label,
                        BoundaryLabel::Curve {
                            curve: CurveId(8),
                            side: candidate,
                            ..
                        } if candidate == side
                    )
                })
                .unwrap();
            boundary_edge_nodes(&mesh, &gap, edge)[1]
        };
        let left = trace_node(crate::CurveTraceSide::Left);
        let right = trace_node(crate::CurveTraceSide::Right);
        let mut jump = vec![0.0; gap.degrees_of_freedom()];
        jump[left] = 1.0;
        let base_force = reflecting.apply_stiffness(&jump).unwrap();
        let gap_force = gap.apply_stiffness(&jump).unwrap();
        let added_left = gap_force[left] - base_force[left];
        let added_right = gap_force[right] - base_force[right];
        assert!(added_left > 0.0);
        assert!((added_left + added_right).abs() < 1.0e-11);
        assert!(gap.maximum_time_step() < reflecting.maximum_time_step());
    }

    fn two_material_scene() -> Scene {
        Scene {
            domain: crate::DomainRect::default(),
            physics: PhysicsModel::Mechanical,
            obstacles: vec![Obstacle::with_role(
                ObstacleId(1),
                PeriodicCubicSpline::rounded(Point2::new(0.5, 0.5), 0.2),
                crate::LoopRole::MaterialInterface {
                    exterior: BACKGROUND_REGION,
                    interior: RegionId(2),
                },
            )],
            internal_boundaries: vec![],
            material_interfaces: vec![],
            junctions: vec![],
            materials: vec![
                Material {
                    id: MaterialId(1),
                    name: "Left".into(),
                    mass_density: crate::ScalarField::constant(2.0),
                    stiffness: crate::ScalarField::constant(3.0),
                    damping: crate::ScalarField::constant(0.5),
                    axis_ratio: crate::ScalarField::constant(1.0),
                    parameters: vec![],
                    color: [1, 2, 3],
                },
                Material {
                    id: MaterialId(2),
                    name: "Right".into(),
                    mass_density: crate::ScalarField::constant(4.0),
                    stiffness: crate::ScalarField::constant(7.0),
                    damping: crate::ScalarField::constant(1.5),
                    axis_ratio: crate::ScalarField::constant(1.0),
                    parameters: vec![],
                    color: [4, 5, 6],
                },
            ],
            regions: vec![
                Region {
                    id: BACKGROUND_REGION,
                    material: MaterialId(1),
                    frame: crate::MaterialFrame::world(),
                },
                Region {
                    id: RegionId(2),
                    material: MaterialId(2),
                    frame: crate::MaterialFrame::world(),
                },
            ],
            volume_sources: vec![],
            outer_boundaries: OuterBoundaryConditions::default(),
        }
    }

    fn square() -> TriMesh {
        TriMesh {
            geometry_revision: 9,
            mesh_revision: 9,
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
            requested_sizes: vec![],
            quality: MeshQuality {
                minimum_angle_degrees: 45.0,
                maximum_edge_length: 2.0_f64.sqrt(),
            },
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

    fn open_baffle_scene(law: InternalBoundaryLaw) -> (Scene, TriMesh) {
        let mut scene = Scene::default();
        scene.internal_boundaries.push(InternalBoundary {
            id: InternalBoundaryId(1),
            spline: OpenCubicSpline::uniform(vec![
                Point2::new(-0.65, 0.0),
                Point2::new(-0.2, 0.0),
                Point2::new(0.2, 0.0),
                Point2::new(0.65, 0.0),
            ])
            .unwrap(),
            region: BACKGROUND_REGION,
            span_laws: vec![law],
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
        (scene, mesh)
    }

    fn trace_edge_nodes(
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        side: InternalBoundarySide,
    ) -> [usize; 3] {
        let edge = mesh
            .boundary_edges
            .iter()
            .filter(|edge| {
                matches!(
                    edge.label,
                    BoundaryLabel::InternalBoundary {
                        id: InternalBoundaryId(1),
                        side: candidate,
                    } if candidate == side
                )
            })
            .max_by(|left, right| {
                let left_midpoint = 0.5 * (left.parameters[0] + left.parameters[1]);
                let right_midpoint = 0.5 * (right.parameters[0] + right.parameters[1]);
                (-(left_midpoint.abs())).total_cmp(&-right_midpoint.abs())
            })
            .unwrap();
        boundary_edge_nodes(mesh, operator, edge)
    }

    fn boundary_edge_nodes(
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        edge: &BoundaryEdge,
    ) -> [usize; 3] {
        let (triangle_index, triangle) = mesh
            .triangles
            .iter()
            .enumerate()
            .find(|(_, triangle)| edge.vertices.iter().all(|v| triangle.vertices.contains(v)))
            .unwrap();
        let a = triangle
            .vertices
            .iter()
            .position(|vertex| *vertex == edge.vertices[0])
            .unwrap();
        let b = triangle
            .vertices
            .iter()
            .position(|vertex| *vertex == edge.vertices[1])
            .unwrap();
        let midpoint_local = match (a.min(b), a.max(b)) {
            (0, 1) => 3,
            (1, 2) => 4,
            (0, 2) => 5,
            _ => unreachable!(),
        };
        [
            edge.vertices[0],
            operator.element_nodes()[triangle_index][midpoint_local] as usize,
            edge.vertices[1],
        ]
    }

    #[test]
    fn shares_edge_nodes_and_has_positive_exact_total_mass() {
        let operator = QuadraticWaveOperator::assemble(
            &square(),
            WaveCoefficients {
                mass_density: 2.5,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(operator.degrees_of_freedom(), 11); // 4 vertices + 5 edges + 2 bubbles
        assert_eq!(operator.element_nodes().len(), 2);
        assert_eq!(
            operator.element_nodes()[0][5],
            operator.element_nodes()[1][3]
        );
        assert!(operator.lumped_mass().iter().all(|mass| *mass > 0.0));
        assert!((operator.lumped_mass().iter().sum::<f64>() - 2.5).abs() < 1.0e-13);
        assert_eq!(operator.geometry_revision(), 9);
    }

    #[test]
    fn scene_assembly_uses_piecewise_material_coefficients() {
        let mut mesh = square();
        mesh.triangles[1].region = RegionId(2);
        let operator = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &two_material_scene(),
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();

        assert!((operator.lumped_mass().iter().sum::<f64>() - 3.0).abs() < 1.0e-13);
        assert!((operator.lumped_damping().iter().sum::<f64>() - 1.0).abs() < 1.0e-13);
        let x = operator
            .node_points()
            .iter()
            .map(|point| point.x)
            .collect::<Vec<_>>();
        let applied = operator.apply_stiffness(&x).unwrap();
        let energy = x.iter().zip(applied).map(|(x, kx)| x * kx).sum::<f64>();
        assert!((energy - 5.0).abs() < 2.0e-12);
        let constant = operator
            .apply_stiffness(&vec![1.0; operator.degrees_of_freedom()])
            .unwrap();
        assert!(constant.iter().all(|value| value.abs() < 5.0e-12));

        // Both elements use the same midpoint degree of freedom on their
        // shared interface, which enforces displacement continuity.
        assert_eq!(
            operator.element_nodes()[0][5],
            operator.element_nodes()[1][3]
        );
    }

    #[test]
    fn scene_assembly_samples_spatial_materials_at_quadrature_points() {
        let mut scene = Scene::default();
        let material = &mut scene.materials[0];
        material.mass_density = crate::ScalarField::formula("1 + x").unwrap();
        material.stiffness = crate::ScalarField::formula("2 + x").unwrap();
        material.damping = crate::ScalarField::formula("y").unwrap();
        let operator = QuadraticWaveOperator::assemble_scene(
            &square(),
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();

        assert!((operator.lumped_mass().iter().sum::<f64>() - 1.5).abs() < 1.0e-12);
        assert!((operator.lumped_damping().iter().sum::<f64>() - 0.5).abs() < 1.0e-12);
        let x = operator
            .node_points()
            .iter()
            .map(|point| point.x)
            .collect::<Vec<_>>();
        let applied = operator.apply_stiffness(&x).unwrap();
        let energy = x.iter().zip(applied).map(|(x, kx)| x * kx).sum::<f64>();
        assert!((energy - 2.5).abs() < 2.0e-12);
        let constant = operator
            .apply_stiffness(&vec![1.0; operator.degrees_of_freedom()])
            .unwrap();
        assert!(constant.iter().all(|value| value.abs() < 5.0e-12));
    }

    #[test]
    fn spatial_material_failure_identifies_coefficient_material_and_point() {
        let mut scene = Scene::default();
        scene.materials[0].mass_density = crate::ScalarField::formula("x").unwrap();
        let error = QuadraticWaveOperator::assemble_scene(
            &square(),
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            WaveError::MaterialEvaluation {
                material,
                coefficient: "density",
                point: Point2 { x: 0.0, y: 0.0 },
                ..
            } if material == "Background"
        ));
    }

    #[test]
    fn scene_assembly_rejects_an_unknown_triangle_region() {
        let mut mesh = square();
        mesh.triangles[0].region = RegionId(99);
        assert!(matches!(
            QuadraticWaveOperator::assemble_scene(
                &mesh,
                &two_material_scene(),
                OuterBoundaryCondition::Reflecting,
            ),
            Err(WaveError::InvalidMesh("a triangle has an unknown region"))
        ));
    }

    #[test]
    fn closed_wall_regions_evolve_as_disconnected_neumann_domains() {
        let mut scene = two_material_scene();
        scene.obstacles[0].role = crate::LoopRole::Wall {
            exterior: BACKGROUND_REGION,
            interior: RegionId(2),
        };
        let mesh = mesh_scene(&scene, 12, MeshingOptions::default()).unwrap();
        let operator = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let mut node_region = vec![None; operator.degrees_of_freedom()];
        for (triangle, nodes) in mesh.triangles.iter().zip(operator.element_nodes()) {
            for node in nodes {
                let assigned = &mut node_region[*node as usize];
                assert!(assigned.is_none_or(|region| region == triangle.region));
                *assigned = Some(triangle.region);
            }
        }
        let displacement = node_region
            .iter()
            .map(|region| f64::from(*region == Some(BACKGROUND_REGION)))
            .collect::<Vec<_>>();
        let mut state = QuadraticWaveState::new(
            &operator,
            operator.recommended_time_step(),
            displacement,
            vec![0.0; operator.degrees_of_freedom()],
        )
        .unwrap();
        for _ in 0..100 {
            state.step(&operator, &[]).unwrap();
        }
        for (value, region) in state.current().iter().zip(node_region) {
            let expected = f64::from(region == Some(BACKGROUND_REGION));
            assert!((value - expected).abs() < 1.0e-11);
        }
    }

    #[test]
    fn outgoing_boundary_adds_positive_exact_lumped_impedance() {
        let mesh = square_with_outer_boundary();
        let coefficients = WaveCoefficients {
            mass_density: 4.0,
            stiffness: 9.0,
            damping: 0.0,
        };
        let reflecting = QuadraticWaveOperator::assemble(&mesh, coefficients).unwrap();
        assert_eq!(
            reflecting.outer_boundary(),
            OuterBoundaryCondition::Reflecting
        );
        assert!(
            reflecting
                .lumped_damping()
                .iter()
                .all(|value| *value == 0.0)
        );

        let outgoing = QuadraticWaveOperator::assemble_with_boundary(
            &mesh,
            coefficients,
            OuterBoundaryCondition::FirstOrderOutgoing,
        )
        .unwrap();
        assert_eq!(
            outgoing.outer_boundary(),
            OuterBoundaryCondition::FirstOrderOutgoing
        );
        // Impedance sqrt(rho*k) = 6 times perimeter 4.
        assert!((outgoing.lumped_damping().iter().sum::<f64>() - 24.0).abs() < 1.0e-13);
        assert_eq!(
            outgoing
                .lumped_damping()
                .iter()
                .filter(|value| **value > 0.0)
                .count(),
            8
        );
        assert_eq!(reflecting.stiffness_values(), outgoing.stiffness_values());
        assert_eq!(reflecting.lumped_mass(), outgoing.lumped_mass());
        assert!(
            outgoing
                .auxiliary_stiffness_values()
                .iter()
                .all(|value| *value == 0.0)
        );

        let second_order = QuadraticWaveOperator::assemble_with_boundary(
            &mesh,
            coefficients,
            OuterBoundaryCondition::SecondOrderOutgoing,
        )
        .unwrap();
        assert_eq!(second_order.lumped_damping(), outgoing.lumped_damping());
        assert_eq!(
            second_order
                .auxiliary_active()
                .iter()
                .filter(|active| **active)
                .count(),
            8
        );
        assert!(
            second_order
                .auxiliary_stiffness_values()
                .iter()
                .any(|value| value.abs() > 0.0)
        );
        for row in 0..second_order.degrees_of_freedom() {
            let row_sum = (second_order.row_offsets[row] as usize
                ..second_order.row_offsets[row + 1] as usize)
                .map(|entry| second_order.auxiliary_stiffness[entry])
                .sum::<f64>();
            assert!(row_sum.abs() < 1.0e-13);
            for entry in
                second_order.row_offsets[row] as usize..second_order.row_offsets[row + 1] as usize
            {
                let column = second_order.columns[entry] as usize;
                let reverse = (second_order.row_offsets[column] as usize
                    ..second_order.row_offsets[column + 1] as usize)
                    .find(|other| second_order.columns[*other] as usize == row)
                    .unwrap();
                assert!(
                    (second_order.auxiliary_stiffness[entry]
                        - second_order.auxiliary_stiffness[reverse])
                        .abs()
                        < 1.0e-13
                );
            }
        }
        // Corner zero couples to the midpoint on each of its incident sides.
        let coupled_boundary_neighbors = (second_order.row_offsets[0] as usize
            ..second_order.row_offsets[1] as usize)
            .filter(|entry| {
                second_order.columns[*entry] as usize != 0
                    && second_order.auxiliary_stiffness[*entry].abs() > 0.0
            })
            .count();
        assert_eq!(coupled_boundary_neighbors, 4);
    }

    #[test]
    fn mixed_outer_sides_apply_time_varying_dirichlet_and_neumann_data() {
        let mesh = square_with_outer_boundary();
        let dirichlet = crate::TimeSignal::Harmonic {
            offset: 0.2,
            amplitude: 0.3,
            frequency_hz: 1.25,
            phase_radians: 0.4,
        };
        let neumann = crate::TimeSignal::harmonic(0.7, 0.0, 1.0, 0.0);
        let mut boundaries = OuterBoundaryConditions::default();
        boundaries.sides[OuterSide::Bottom.index()] =
            OuterBoundaryCondition::Dirichlet { signal: dirichlet };
        boundaries.sides[OuterSide::Top.index()] =
            OuterBoundaryCondition::Neumann { signal: neumann };
        boundaries.sides[OuterSide::Right.index()] = OuterBoundaryCondition::FirstOrderOutgoing;
        boundaries.sides[OuterSide::Left.index()] = OuterBoundaryCondition::SecondOrderOutgoing;
        let operator = QuadraticWaveOperator::assemble_scene_with_boundaries(
            &mesh,
            &Scene::default(),
            boundaries,
        )
        .unwrap();

        for (node, point) in operator.node_points().iter().enumerate() {
            if point.y == 0.0 {
                assert_eq!(operator.dirichlet_sides()[node], Some(OuterSide::Bottom));
            }
            if point.y == 1.0 && point.x > 0.0 && point.x < 1.0 {
                assert!(operator.normalized_neumann_weights()[node][OuterSide::Top.index()] > 0.0);
            }
        }

        let dt = 0.25 * operator.maximum_time_step();
        let mut state = QuadraticWaveState::zero(&operator, dt).unwrap();
        for (node, point) in operator.node_points().iter().enumerate() {
            if point.y == 0.0 {
                assert!((state.current()[node] - dirichlet.value(0.0)).abs() < 1.0e-14);
            }
        }
        state.step(&operator, &[]).unwrap();
        for (node, point) in operator.node_points().iter().enumerate() {
            if point.y == 0.0 {
                assert!((state.current()[node] - dirichlet.value(dt)).abs() < 1.0e-13);
            }
        }
        assert!(state.current().iter().any(|value| *value != 0.0));

        let mut contradictory = boundaries;
        contradictory.sides[OuterSide::Right.index()] = OuterBoundaryCondition::Dirichlet {
            signal: crate::TimeSignal::harmonic(9.0, 0.0, 1.0, 0.0),
        };
        assert!(
            QuadraticWaveOperator::assemble_scene_with_boundaries(
                &mesh,
                &Scene::default(),
                contradictory,
            )
            .is_err()
        );
    }

    #[test]
    fn second_order_auxiliary_state_remains_bounded_and_loses_energy() {
        let operator = QuadraticWaveOperator::assemble_with_boundary(
            &square_with_outer_boundary(),
            WaveCoefficients::default(),
            OuterBoundaryCondition::SecondOrderOutgoing,
        )
        .unwrap();
        let dt = operator.recommended_time_step();
        let initial = operator
            .node_points()
            .iter()
            .map(|point| (-4.0 * (point.x * point.x + point.y * point.y)).exp())
            .collect();
        let mut state = QuadraticWaveState::new(
            &operator,
            dt,
            initial,
            vec![0.0; operator.degrees_of_freedom()],
        )
        .unwrap();
        let initial_energy = state.energy(&operator).unwrap();
        for _ in 0..10_000 {
            state.step(&operator, &[]).unwrap();
        }
        let final_energy = state.energy(&operator).unwrap();
        assert!(final_energy.is_finite());
        assert!(final_energy < initial_energy * 1.0e-3);
        assert!(state.auxiliary().iter().all(|value| value.is_finite()));
    }

    /// Reproducer for the open item on second-order outgoing conditions on
    /// curved and baffle spans, which were reported to inject energy and
    /// diverge. A hole bounded by second-order faces and a curved baffle with
    /// second-order faces sit in a reflecting cavity, meshed through the
    /// topology path the app uses, and a pulse is released beside them. The
    /// absorbers may only remove energy.
    ///
    /// It reproduces the divergence today: the energy grows by eighteen
    /// orders of magnitude within 8,000 steps, independently of the time step
    /// and worse under interior refinement, while a straight baffle with the
    /// same law is stable. See the engineering log for 2026-09-15. It stays
    /// ignored until the condition is fixed; run it with `--ignored`.
    #[test]
    #[ignore = "reproduces the open second-order instability on curved spans"]
    fn second_order_curved_and_baffle_faces_never_add_energy() {
        let second_order = SpanBehavior::Separated {
            left: FaceBoundaryCondition::SecondOrderOutgoing,
            right: FaceBoundaryCondition::SecondOrderOutgoing,
            coupling: InternalBoundaryCoupling::Independent,
        };
        let hole_spline = PeriodicCubicSpline::rounded(Point2::new(0.1, -0.05), 0.3);
        let hole_spans = (0..hole_spline.intervals().len())
            .map(|index| topology_span(index as u64 + 1, second_order))
            .collect();
        let hole =
            TopologyCurve::new(CurveId(1), CurveSpline::Closed(hole_spline), hole_spans).unwrap();
        let baffle = TopologyCurve::new(
            CurveId(2),
            CurveSpline::Open(
                OpenCubicSpline::uniform(vec![
                    Point2::new(-0.7, 0.4),
                    Point2::new(-0.3, 0.6),
                    Point2::new(0.2, 0.55),
                    Point2::new(0.6, 0.4),
                ])
                .unwrap(),
            ),
            vec![topology_span(20, second_order)],
        )
        .unwrap();
        let topology = compile_topology(
            &TopologyGeometry {
                curves: vec![hole, baffle],
                ..TopologyGeometry::default()
            },
            61,
        )
        .unwrap();
        let inside = topology.face_at(Point2::new(0.1, -0.05)).unwrap();
        let assignments = topology
            .faces
            .iter()
            .map(|face| FaceRegionAssignment {
                face: face.id,
                region: (face.id != inside).then_some(BACKGROUND_REGION),
            })
            .collect::<Vec<_>>();
        let plan = TopologyMeshPlan::new(&topology, &assignments).unwrap();
        let mesh = mesh_topology_plan(
            &plan,
            161,
            MeshingOptions {
                target_edge_length: 0.15,
                minimum_angle_degrees: 12.0,
                max_vertices: 40_000,
                max_triangles: 80_000,
                max_refinement_steps: 40_000,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let scene = Scene::default();
        let operator = QuadraticWaveOperator::assemble_topology(
            &mesh,
            &plan,
            TopologyWaveModel::from_scene(&scene),
        )
        .unwrap();
        assert!(operator.auxiliary_active().iter().any(|active| *active));

        let dt = operator.recommended_time_step();
        let initial = operator
            .node_points()
            .iter()
            .map(|point| {
                let (dx, dy) = (point.x + 0.55, point.y + 0.35);
                (-(dx * dx + dy * dy) / 0.02).exp()
            })
            .collect();
        let mut state = QuadraticWaveState::new(
            &operator,
            dt,
            initial,
            vec![0.0; operator.degrees_of_freedom()],
        )
        .unwrap();
        let initial_energy = state.energy(&operator).unwrap();
        let mut peak = initial_energy;
        for step in 1..=8_000 {
            state.step(&operator, &[]).unwrap();
            if step % 50 == 0 {
                let energy = state.energy(&operator).unwrap();
                assert!(energy.is_finite(), "energy diverged at step {step}");
                peak = peak.max(energy);
            }
        }
        let final_energy = state.energy(&operator).unwrap();
        assert!(
            peak <= initial_energy * (1.0 + 1.0e-9),
            "the absorbers added energy: peak {peak:e} against {initial_energy:e}"
        );
        assert!(
            final_energy < initial_energy * 0.5,
            "the absorbers removed too little: {final_energy:e} of {initial_energy:e}"
        );
        assert!(state.auxiliary().iter().all(|value| value.is_finite()));
    }

    #[test]
    fn outgoing_boundary_rejects_malformed_edges() {
        for boundary in [
            OuterBoundaryCondition::FirstOrderOutgoing,
            OuterBoundaryCondition::SecondOrderOutgoing,
        ] {
            let mut mesh = square_with_outer_boundary();
            mesh.boundary_edges[0].vertices = [0, 99];
            assert!(matches!(
                QuadraticWaveOperator::assemble_with_boundary(
                    &mesh,
                    WaveCoefficients::default(),
                    boundary,
                ),
                Err(WaveError::InvalidMesh(_))
            ));
        }
    }

    #[test]
    fn open_baffle_face_impedance_adds_only_selected_trace_damping() {
        let (reflecting_scene, mesh) = open_baffle_scene(InternalBoundaryLaw::REFLECTING);
        let reflecting = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &reflecting_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let law = InternalBoundaryLaw {
            left: FaceBoundaryCondition::Impedance { ratio: 1.0 },
            ..InternalBoundaryLaw::REFLECTING
        };
        let mut impedance_scene = reflecting_scene.clone();
        impedance_scene.internal_boundaries[0].span_laws[0] = law;
        let impedance = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &impedance_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let left = trace_edge_nodes(&mesh, &reflecting, InternalBoundarySide::Left);
        let right = trace_edge_nodes(&mesh, &reflecting, InternalBoundarySide::Right);
        assert!(impedance.lumped_damping()[left[1]] > reflecting.lumped_damping()[left[1]]);
        assert_eq!(
            impedance.lumped_damping()[right[1]],
            reflecting.lumped_damping()[right[1]]
        );
    }

    #[test]
    fn driven_conditions_apply_to_hole_and_baffle_faces() {
        let dirichlet = TimeSignal::Harmonic {
            offset: 0.35,
            amplitude: 0.2,
            frequency_hz: 1.5,
            phase_radians: 0.4,
        };
        let neumann = TimeSignal::harmonic(1.25, 0.0, 1.0, 0.0);
        let law = InternalBoundaryLaw {
            left: FaceBoundaryCondition::Dirichlet { signal: dirichlet },
            right: FaceBoundaryCondition::Neumann { signal: neumann },
            coupling: InternalBoundaryCoupling::Independent,
        };
        let (scene, mesh) = open_baffle_scene(law);
        let operator = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let left = trace_edge_nodes(&mesh, &operator, InternalBoundarySide::Left)[1];
        let right = trace_edge_nodes(&mesh, &operator, InternalBoundarySide::Right)[1];
        assert_eq!(
            operator.prescribed_value(left, 0.3),
            Some(dirichlet.value(0.3))
        );
        assert!(operator.prescribed_value(right, 0.3).is_none());
        assert!(operator.neumann_acceleration(right, 0.3) > 0.0);

        let mut hole_scene = Scene::initial();
        hole_scene.obstacles[0].span_conditions[0] =
            FaceBoundaryCondition::Dirichlet { signal: dirichlet };
        hole_scene.obstacles[0].span_conditions[1] =
            FaceBoundaryCondition::Neumann { signal: neumann };
        let hole_mesh = mesh_scene(&hole_scene, 12, MeshingOptions::default()).unwrap();
        let hole_operator = QuadraticWaveOperator::assemble_scene(
            &hole_mesh,
            &hole_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let mut found_dirichlet = false;
        let mut found_neumann = false;
        for edge in &hole_mesh.boundary_edges {
            if edge.label != BoundaryLabel::Obstacle(ObstacleId(1)) {
                continue;
            }
            let span = hole_scene.obstacles[0]
                .spline
                .span_index(0.5 * (edge.parameters[0] + edge.parameters[1]))
                .unwrap();
            let midpoint = boundary_edge_nodes(&hole_mesh, &hole_operator, edge)[1];
            if span == 0 {
                found_dirichlet = true;
                assert_eq!(
                    hole_operator.prescribed_value(midpoint, 0.3),
                    Some(dirichlet.value(0.3))
                );
            } else if span == 1 {
                found_neumann = true;
                assert!(hole_operator.neumann_acceleration(midpoint, 0.3) > 0.0);
            }
        }
        assert!(found_dirichlet && found_neumann);
    }

    #[test]
    fn second_order_absorber_applies_to_curved_and_baffle_faces() {
        let mut hole_scene = Scene::initial();
        hole_scene.obstacles[0].span_conditions[0] = FaceBoundaryCondition::SecondOrderOutgoing;
        let hole_mesh = mesh_scene(&hole_scene, 12, MeshingOptions::default()).unwrap();
        let hole = QuadraticWaveOperator::assemble_scene(
            &hole_mesh,
            &hole_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        assert!(hole.auxiliary_active().iter().any(|active| *active));
        assert!(hole.lumped_damping().iter().any(|value| *value > 0.0));

        let law = InternalBoundaryLaw {
            left: FaceBoundaryCondition::SecondOrderOutgoing,
            ..InternalBoundaryLaw::REFLECTING
        };
        let (scene, mesh) = open_baffle_scene(law);
        let baffle = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let left = trace_edge_nodes(&mesh, &baffle, InternalBoundarySide::Left)[1];
        let right = trace_edge_nodes(&mesh, &baffle, InternalBoundarySide::Right)[1];
        assert!(baffle.auxiliary_active()[left]);
        assert!(!baffle.auxiliary_active()[right]);
        assert!(baffle.lumped_damping()[left] > baffle.lumped_damping()[right]);
    }

    #[test]
    fn hole_impedance_adds_damping_only_on_assigned_knot_span() {
        let reflecting_scene = Scene::initial();
        let mesh = mesh_scene(&reflecting_scene, 12, MeshingOptions::default()).unwrap();
        let reflecting = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &reflecting_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let mut impedance_scene = reflecting_scene.clone();
        impedance_scene.obstacles[0].span_conditions[0] =
            FaceBoundaryCondition::Impedance { ratio: 1.0 };
        let impedance = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &impedance_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();

        let mut assigned_edges = 0;
        let mut reflecting_edges = 0;
        for edge in &mesh.boundary_edges {
            if edge.label != BoundaryLabel::Obstacle(ObstacleId(1)) {
                continue;
            }
            let span = impedance_scene.obstacles[0]
                .spline
                .span_index(0.5 * (edge.parameters[0] + edge.parameters[1]))
                .unwrap();
            let midpoint = boundary_edge_nodes(&mesh, &impedance, edge)[1];
            if span == 0 {
                assigned_edges += 1;
                assert!(impedance.lumped_damping()[midpoint] > 0.0);
            } else {
                reflecting_edges += 1;
                assert_eq!(
                    impedance.lumped_damping()[midpoint],
                    reflecting.lumped_damping()[midpoint]
                );
            }
        }
        assert!(assigned_edges > 0);
        assert!(reflecting_edges > 0);
    }

    #[test]
    fn open_baffle_knot_spans_keep_distinct_face_conditions_after_meshing() {
        let mut scene = Scene::default();
        scene.internal_boundaries.push(InternalBoundary {
            id: InternalBoundaryId(1),
            spline: OpenCubicSpline::uniform(vec![
                Point2::new(-0.7, 0.0),
                Point2::new(-0.35, 0.0),
                Point2::new(0.0, 0.0),
                Point2::new(0.35, 0.0),
                Point2::new(0.7, 0.0),
            ])
            .unwrap(),
            region: BACKGROUND_REGION,
            span_laws: vec![
                InternalBoundaryLaw {
                    left: FaceBoundaryCondition::Impedance { ratio: 1.0 },
                    ..InternalBoundaryLaw::REFLECTING
                },
                InternalBoundaryLaw::REFLECTING,
            ],
        });
        let mesh = mesh_scene(
            &scene,
            18,
            MeshingOptions {
                target_edge_length: 0.18,
                minimum_angle_degrees: 14.0,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let operator = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let mut first_span = 0;
        let mut second_span = 0;
        for edge in &mesh.boundary_edges {
            if edge.label
                != (BoundaryLabel::InternalBoundary {
                    id: InternalBoundaryId(1),
                    side: InternalBoundarySide::Left,
                })
            {
                continue;
            }
            let midpoint = boundary_edge_nodes(&mesh, &operator, edge)[1];
            if 0.5 * (edge.parameters[0] + edge.parameters[1]) < 1.0 {
                first_span += 1;
                assert!(operator.lumped_damping()[midpoint] > 0.0);
            } else {
                second_span += 1;
                assert_eq!(operator.lumped_damping()[midpoint], 0.0);
            }
        }
        assert!(first_span > 0 && second_span > 0);
    }

    #[test]
    fn thin_gap_is_symmetric_conservative_and_enters_the_time_step_bound() {
        let (reflecting_scene, mesh) = open_baffle_scene(InternalBoundaryLaw::REFLECTING);
        let reflecting = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &reflecting_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let law = InternalBoundaryLaw {
            coupling: InternalBoundaryCoupling::ThinGap {
                stiffness_ratio: 10_000.0,
            },
            ..InternalBoundaryLaw::REFLECTING
        };
        let mut gap_scene = reflecting_scene.clone();
        gap_scene.internal_boundaries[0].span_laws[0] = law;
        let gap = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &gap_scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let left = trace_edge_nodes(&mesh, &gap, InternalBoundarySide::Left)[1];
        let right = trace_edge_nodes(&mesh, &gap, InternalBoundarySide::Right)[1];
        let mut jump = vec![0.0; gap.degrees_of_freedom()];
        jump[left] = 1.0;
        let reflecting_force = reflecting.apply_stiffness(&jump).unwrap();
        let gap_force = gap.apply_stiffness(&jump).unwrap();
        let added_left = gap_force[left] - reflecting_force[left];
        let added_right = gap_force[right] - reflecting_force[right];
        assert!(added_left > 0.0);
        assert!((added_left + added_right).abs() < 1.0e-12);
        assert!(gap.maximum_time_step() < reflecting.maximum_time_step());

        let mut state = QuadraticWaveState::new(
            &gap,
            0.5 * gap.maximum_time_step(),
            jump,
            vec![0.0; gap.degrees_of_freedom()],
        )
        .unwrap();
        let initial_energy = state.energy(&gap).unwrap();
        for _ in 0..200 {
            state.step(&gap, &[]).unwrap();
        }
        let drift = (state.energy(&gap).unwrap() - initial_energy).abs() / initial_energy;
        assert!(drift < 1.0e-10, "energy drift {drift:e}");
    }

    #[test]
    fn stiffness_is_symmetric_and_annihilates_constants() {
        let operator =
            QuadraticWaveOperator::assemble(&square(), WaveCoefficients::default()).unwrap();
        let applied = operator
            .apply_stiffness(&vec![1.0; operator.degrees_of_freedom()])
            .unwrap();
        assert!(applied.iter().all(|value| value.abs() < 2.0e-12));
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
    }

    /// The cooperative assembly runs the same steps one at a time, passes
    /// through every phase, and lands on the same operator as the one-shot path.
    #[test]
    fn the_assembly_job_matches_the_one_shot_operator() {
        let (plan, mesh, scene) = topology_baffle(
            SpanBehavior::Separated {
                left: FaceBoundaryCondition::SecondOrderOutgoing,
                right: FaceBoundaryCondition::Impedance { ratio: 0.5 },
                coupling: InternalBoundaryCoupling::Independent,
            },
            43,
        );
        let expected = QuadraticWaveOperator::assemble_topology(
            &mesh,
            &plan,
            TopologyWaveModel::from_scene(&scene),
        )
        .unwrap();
        let mut job = QuadraticAssemblyJob::new_topology(
            Arc::new(mesh.clone()),
            Arc::new(plan.clone()),
            TopologyWaveModel::from_scene(&scene),
        )
        .unwrap();
        let mut steps = 0usize;
        let mut phases: Vec<&'static str> = Vec::new();
        let assembled = loop {
            steps += 1;
            let phase = job.phase();
            if phases.last() != Some(&phase) {
                phases.push(phase);
            }
            if let Some(result) = job.advance(1) {
                break result.unwrap();
            }
        };
        assert_eq!(assembled, expected);
        assert!(steps > 2 * mesh.triangles.len(), "{steps}");
        assert_eq!(
            phases,
            [
                "Numbering wave nodes",
                "Assembling wave elements",
                "Assembling boundary laws",
                "Compressing the wave operator",
                "Bounding the time step",
            ]
        );
    }

    /// Every operator the solver can build must have exactly zero row sums in
    /// both stiffness matrices, because the time step applies them to the
    /// differences `u_j - u_i` and that form only equals `K u` when the rows
    /// annihilate constants. Outgoing edges, curved absorbers, thin gaps,
    /// material interfaces, and the topology assembly path are all covered.
    #[test]
    fn every_assembled_stiffness_row_sums_to_zero() {
        let mut hole_scene = Scene::initial();
        hole_scene.obstacles[0].span_conditions[0] = FaceBoundaryCondition::SecondOrderOutgoing;
        hole_scene.obstacles[0].span_conditions[1] =
            FaceBoundaryCondition::Impedance { ratio: 1.0 };
        let hole_mesh = mesh_scene(&hole_scene, 12, MeshingOptions::default()).unwrap();
        let (gap_scene, gap_mesh) = open_baffle_scene(InternalBoundaryLaw {
            coupling: InternalBoundaryCoupling::ThinGap {
                stiffness_ratio: 25.0,
            },
            ..InternalBoundaryLaw::REFLECTING
        });
        let materials = two_material_scene();
        let material_mesh = mesh_scene(&materials, 13, MeshingOptions::default()).unwrap();
        let (plan, topology_mesh, topology_scene) = topology_baffle(
            SpanBehavior::Separated {
                left: FaceBoundaryCondition::SecondOrderOutgoing,
                right: FaceBoundaryCondition::Impedance { ratio: 0.5 },
                coupling: InternalBoundaryCoupling::Independent,
            },
            41,
        );
        let operators = [
            (
                "second-order outer",
                true,
                QuadraticWaveOperator::assemble_with_boundary(
                    &square_with_outer_boundary(),
                    WaveCoefficients::default(),
                    OuterBoundaryCondition::SecondOrderOutgoing,
                )
                .unwrap(),
            ),
            (
                "curved absorber and impedance",
                true,
                QuadraticWaveOperator::assemble_scene(
                    &hole_mesh,
                    &hole_scene,
                    OuterBoundaryCondition::FirstOrderOutgoing,
                )
                .unwrap(),
            ),
            (
                "thin gap",
                false,
                QuadraticWaveOperator::assemble_scene(
                    &gap_mesh,
                    &gap_scene,
                    OuterBoundaryCondition::Reflecting,
                )
                .unwrap(),
            ),
            (
                "material interface",
                false,
                QuadraticWaveOperator::assemble_scene(
                    &material_mesh,
                    &materials,
                    OuterBoundaryCondition::Reflecting,
                )
                .unwrap(),
            ),
            (
                "topology baffle with a second-order face",
                true,
                QuadraticWaveOperator::assemble_topology(
                    &topology_mesh,
                    &plan,
                    TopologyWaveModel::from_scene(&topology_scene),
                )
                .unwrap(),
            ),
        ];
        for (name, expects_auxiliary, operator) in &operators {
            let mut saw_auxiliary = false;
            for row in 0..operator.degrees_of_freedom() {
                let entries =
                    operator.row_offsets[row] as usize..operator.row_offsets[row + 1] as usize;
                let (sum, scale) = entries.clone().fold((0.0, 0.0), |(sum, scale), entry| {
                    (
                        sum + operator.stiffness[entry],
                        scale + operator.stiffness[entry].abs(),
                    )
                });
                assert!(
                    scale > 0.0 && sum.abs() <= 1.0e-12 * scale,
                    "{name}: stiffness row {row} sums to {sum:e} against {scale:e}"
                );
                let (sum, scale) = entries.fold((0.0, 0.0), |(sum, scale), entry| {
                    (
                        sum + operator.auxiliary_stiffness[entry],
                        scale + operator.auxiliary_stiffness[entry].abs(),
                    )
                });
                saw_auxiliary |= scale > 0.0;
                assert!(
                    sum.abs() <= 1.0e-12 * scale.max(1.0e-300),
                    "{name}: auxiliary row {row} sums to {sum:e} against {scale:e}"
                );
            }
            assert_eq!(
                saw_auxiliary, *expects_auxiliary,
                "{name}: second-order edge coverage differs from the fixture's intent"
            );
        }
    }

    /// Mirrors `advance_wave` in `wave.wgsl` on the f32 operator the GPU
    /// receives. A reflecting cavity holding a constant field must keep it bit
    /// for bit; the plain row product drifts because its rounded rows do not
    /// sum to zero, which is the uniform offset that used to grow in enclosed
    /// Neumann subdomains.
    #[test]
    fn the_f32_kernel_form_holds_a_constant_field_exactly() {
        let mesh = mesh_scene(
            &Scene::default(),
            5,
            MeshingOptions {
                target_edge_length: 0.12,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let operator = QuadraticWaveOperator::assemble(&mesh, WaveCoefficients::default()).unwrap();
        let normalized = operator.normalized_stiffness_f32().unwrap();
        let dt2 = (operator.recommended_time_step() as f32).powi(2);
        let count = operator.degrees_of_freedom();
        let run = |difference_form: bool| {
            let mut previous = vec![1.0f32; count];
            let mut current = vec![1.0f32; count];
            let mut next = vec![0.0f32; count];
            for _ in 0..2_000 {
                for i in 0..count {
                    let row =
                        operator.row_offsets[i] as usize..operator.row_offsets[i + 1] as usize;
                    let mut ku = 0.0f32;
                    for (coefficient, column) in
                        normalized[row.clone()].iter().zip(&operator.columns[row])
                    {
                        let column = *column as usize;
                        ku += coefficient
                            * if difference_form {
                                current[column] - current[i]
                            } else {
                                current[column]
                            };
                    }
                    next[i] = 2.0 * current[i] - previous[i] - dt2 * ku;
                }
                std::mem::swap(&mut previous, &mut current);
                std::mem::swap(&mut current, &mut next);
            }
            current
                .iter()
                .map(|value| (value - 1.0).abs())
                .fold(0.0f32, f32::max)
        };
        assert_eq!(run(true), 0.0);
        assert!(
            run(false) > 1.0e-7,
            "the row-product form is expected to drift"
        );
    }

    #[test]
    fn basis_is_cardinal_and_reproduces_affine_fields() {
        let nodes = [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.5, 0.5, 0.0],
            [0.0, 0.5, 0.5],
            [0.5, 0.0, 0.5],
            [1.0 / 3.0; 3],
        ];
        for (node, barycentric) in nodes.into_iter().enumerate() {
            for (basis, value) in enriched_quadratic_basis(barycentric)
                .into_iter()
                .enumerate()
            {
                let expected = usize::from(node == basis) as f64;
                assert!((value - expected).abs() < 2.0e-15);
            }
        }

        let barycentric = [0.17, 0.29, 0.54];
        let values = enriched_quadratic_basis(barycentric);
        assert!((values.iter().sum::<f64>() - 1.0).abs() < 2.0e-15);
        let node_x = [0.0, 1.0, 0.0, 0.5, 0.5, 0.0, 1.0 / 3.0];
        let interpolated_x = values
            .iter()
            .zip(node_x)
            .map(|(basis, x)| basis * x)
            .sum::<f64>();
        assert!((interpolated_x - barycentric[1]).abs() < 2.0e-15);
    }

    #[test]
    fn public_element_sampler_reproduces_an_affine_value_and_gradient() {
        let mesh = square();
        let operator = QuadraticWaveOperator::assemble(&mesh, WaveCoefficients::default()).unwrap();
        let values = operator
            .node_points()
            .iter()
            .map(|point| (0.7 + 1.25 * point.x - 0.8 * point.y) as f32)
            .collect::<Vec<_>>();
        let (value, gradient) = operator
            .element_value_and_gradient(0, &values, [1.0 / 3.0; 3])
            .unwrap();
        let nodes = operator.element_nodes()[0];
        let centroid = (operator.node_points()[nodes[0] as usize]
            + operator.node_points()[nodes[1] as usize]
            + operator.node_points()[nodes[2] as usize])
            / 3.0;
        let expected = 0.7 + 1.25 * centroid.x - 0.8 * centroid.y;
        assert!((value - expected).abs() < 2.0e-7);
        assert!((gradient - Point2::new(1.25, -0.8)).norm() < 2.0e-7);
    }

    #[test]
    fn electromagnetic_energy_uses_primary_field_and_transverse_potential() {
        let mesh = square();
        let operator = QuadraticWaveOperator::assemble(&mesh, WaveCoefficients::default()).unwrap();
        let primary = vec![2.0; operator.degrees_of_freedom()];
        let transverse_potential = operator
            .node_points()
            .iter()
            .map(|point| point.x)
            .collect::<Vec<_>>();
        let energy = operator
            .electromagnetic_energy(&primary, &transverse_potential)
            .unwrap();
        assert!((energy - 2.5).abs() < 1.0e-12);
        assert!(
            operator
                .electromagnetic_energy(&primary[..2], &transverse_potential)
                .is_err()
        );
    }

    #[test]
    fn scene_assembly_compiles_tm_and_te_material_laws() {
        let mesh = square();
        let mut scene = Scene::default();
        scene.materials[0].mass_density = crate::ScalarField::constant(4.0);
        scene.materials[0].stiffness = crate::ScalarField::constant(9.0);
        scene.materials[0].damping = crate::ScalarField::constant(0.5);
        for (polarization, expected_mass, expected_damping) in [
            (crate::ElectromagneticPolarization::Tm, 4.0, 2.0),
            (crate::ElectromagneticPolarization::Te, 9.0, 4.5),
        ] {
            scene.physics = PhysicsModel::Electromagnetic { polarization };
            let operator = QuadraticWaveOperator::assemble_scene(
                &mesh,
                &scene,
                OuterBoundaryCondition::Reflecting,
            )
            .unwrap();
            assert!((operator.lumped_mass().iter().sum::<f64>() - expected_mass).abs() < 1e-12);
            assert!(
                (operator.lumped_damping().iter().sum::<f64>() - expected_damping).abs() < 1e-12
            );
        }
    }

    #[test]
    fn enriched_basis_laplacians_reproduce_a_quadratic() {
        let barycentric = [0.17, 0.29, 0.54];
        let gradients = [
            Point2::new(-1.0, -1.0),
            Point2::new(1.0, 0.0),
            Point2::new(0.0, 1.0),
        ];
        let laplacians = enriched_quadratic_basis_laplacians(barycentric, gradients);
        assert!(laplacians.iter().sum::<f64>().abs() < 2.0e-13);
        let x_squared = [0.0, 1.0, 0.0, 0.25, 0.25, 0.0, 1.0 / 9.0];
        let laplace = laplacians
            .into_iter()
            .zip(x_squared)
            .map(|(basis, value)| basis * value)
            .sum::<f64>();
        assert!((laplace - 2.0).abs() < 2.0e-13);
    }

    #[test]
    fn stiffness_quadrature_is_exact_through_degree_four() {
        fn factorial(value: usize) -> usize {
            (1..=value).product()
        }
        for a in 0..=4 {
            for b in 0..=4 - a {
                for c in 0..=4 - a - b {
                    let actual = stiffness_quadrature()
                        .into_iter()
                        .map(|(l, weight)| {
                            weight * l[0].powi(a as i32) * l[1].powi(b as i32) * l[2].powi(c as i32)
                        })
                        .sum::<f64>();
                    let exact = 2.0 * (factorial(a) * factorial(b) * factorial(c)) as f64
                        / factorial(a + b + c + 2) as f64;
                    assert!((actual - exact).abs() < 2.0e-14, "({a}, {b}, {c})");
                }
            }
        }
    }

    #[test]
    fn affine_field_has_exact_dirichlet_energy() {
        let operator = QuadraticWaveOperator::assemble(
            &square(),
            WaveCoefficients {
                stiffness: 3.25,
                ..Default::default()
            },
        )
        .unwrap();
        let x = operator
            .node_points()
            .iter()
            .map(|point| point.x)
            .collect::<Vec<_>>();
        let applied = operator.apply_stiffness(&x).unwrap();
        let energy = x.iter().zip(applied).map(|(x, kx)| x * kx).sum::<f64>();
        assert!((energy - 3.25).abs() < 2.0e-13);
    }

    #[test]
    fn constant_state_remains_stationary() {
        let operator =
            QuadraticWaveOperator::assemble(&square(), WaveCoefficients::default()).unwrap();
        let dt = 0.4 * operator.maximum_time_step();
        let mut state = QuadraticWaveState::new(
            &operator,
            dt,
            vec![0.7; operator.degrees_of_freedom()],
            vec![0.0; operator.degrees_of_freedom()],
        )
        .unwrap();
        for _ in 0..500 {
            state.step(&operator, &[]).unwrap();
        }
        assert!(
            state
                .current()
                .iter()
                .all(|value| (*value - 0.7).abs() < 1.0e-10)
        );
    }

    #[test]
    fn undamped_discrete_energy_is_conserved() {
        let operator =
            QuadraticWaveOperator::assemble(&square(), WaveCoefficients::default()).unwrap();
        let dt = 0.4 * operator.maximum_time_step();
        let initial = operator
            .node_points()
            .iter()
            .map(|point| (std::f64::consts::PI * point.x).cos())
            .collect();
        let mut state = QuadraticWaveState::new(
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
    fn rejects_invalid_mesh_coefficients_state_and_timestep() {
        let mesh = square();
        assert!(matches!(
            QuadraticWaveOperator::assemble(
                &mesh,
                WaveCoefficients {
                    stiffness: 0.0,
                    ..Default::default()
                }
            ),
            Err(WaveError::InvalidCoefficients)
        ));
        let operator = QuadraticWaveOperator::assemble(&mesh, WaveCoefficients::default()).unwrap();
        assert!(matches!(
            QuadraticWaveState::new(
                &operator,
                operator.maximum_time_step() * 1.01,
                vec![0.0; operator.degrees_of_freedom()],
                vec![0.0; operator.degrees_of_freedom()]
            ),
            Err(WaveError::InvalidTimeStep { .. })
        ));
        let mut invalid = vec![0.0; operator.degrees_of_freedom()];
        invalid[0] = f64::NAN;
        assert!(matches!(
            operator.apply_stiffness(&invalid),
            Err(WaveError::InvalidState)
        ));
        assert!(matches!(
            QuadraticWaveState::new_with_auxiliary(
                &operator,
                operator.recommended_time_step(),
                vec![0.0; operator.degrees_of_freedom()],
                vec![0.0; operator.degrees_of_freedom()],
                vec![0.0; operator.degrees_of_freedom() - 1],
            ),
            Err(WaveError::SizeMismatch { .. })
        ));
        assert!(matches!(
            operator.apply_auxiliary_stiffness(&invalid),
            Err(WaveError::InvalidState)
        ));

        let sliver = TriMesh {
            geometry_revision: 10,
            mesh_revision: 10,
            vertices: [[0.0, 0.0], [1.0, 0.0], [0.5, 1.0e-12]]
                .into_iter()
                .map(|[x, y]| MeshVertex {
                    point: Point2::new(x, y),
                    boundary: None,
                    trace: None,
                })
                .collect(),
            triangles: vec![MeshTriangle {
                vertices: [0, 1, 2],
                region: BACKGROUND_REGION,
            }],
            boundary_edges: vec![],
            requested_sizes: vec![],
            quality: MeshQuality {
                minimum_angle_degrees: 0.0,
                maximum_edge_length: 1.0,
            },
        };
        assert!(matches!(
            QuadraticWaveOperator::assemble(&sliver, WaveCoefficients::default()),
            Err(WaveError::InvalidMesh(
                "a near-degenerate element collapses the explicit CFL timestep"
            ))
        ));
    }

    #[test]
    fn directional_scene_assembles_rotated_tensor_energy() {
        let mut scene = Scene::default();
        scene.materials[0].axis_ratio = crate::ScalarField::constant(4.0);
        let operator = QuadraticWaveOperator::assemble_scene(
            &square(),
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let energy = |operator: &QuadraticWaveOperator, direction: Point2| {
            let values = operator
                .node_points()
                .iter()
                .map(|point| point.dot(direction))
                .collect::<Vec<_>>();
            let applied = operator.apply_stiffness(&values).unwrap();
            values.iter().zip(applied).map(|(a, b)| a * b).sum::<f64>()
        };
        assert!((energy(&operator, Point2::new(1.0, 0.0)) - 4.0).abs() < 1.0e-10);
        assert!((energy(&operator, Point2::new(0.0, 1.0)) - 0.25).abs() < 1.0e-10);

        scene.regions[0].frame.angle_radians = std::f64::consts::FRAC_PI_2;
        let rotated = QuadraticWaveOperator::assemble_scene(
            &square(),
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        assert!((energy(&rotated, Point2::new(1.0, 0.0)) - 0.25).abs() < 1.0e-10);
        assert!((energy(&rotated, Point2::new(0.0, 1.0)) - 4.0).abs() < 1.0e-10);
    }

    #[test]
    fn outgoing_impedance_uses_the_boundary_normal() {
        let mut scene = Scene::default();
        scene.materials[0].axis_ratio = crate::ScalarField::constant(4.0);
        let operator = QuadraticWaveOperator::assemble_scene(
            &square_with_outer_boundary(),
            &scene,
            OuterBoundaryCondition::FirstOrderOutgoing,
        )
        .unwrap();
        let damping_at = |point: Point2| {
            let index = operator
                .node_points()
                .iter()
                .position(|candidate| (*candidate - point).norm() < 1.0e-12)
                .unwrap();
            operator.lumped_damping()[index]
        };
        // The x-normal edge sees sqrt(A_xx)=2, while the y-normal edge sees 1/2.
        assert!((damping_at(Point2::new(1.0, 0.5)) - 4.0 / 3.0).abs() < 1.0e-12);
        assert!((damping_at(Point2::new(0.5, 0.0)) - 1.0 / 3.0).abs() < 1.0e-12);
    }
}
