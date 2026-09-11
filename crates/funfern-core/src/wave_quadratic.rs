use std::collections::{BTreeMap, BTreeSet};

use crate::{
    BACKGROUND_REGION, BoundaryLabel, BoundarySignal, FaceBoundaryCondition,
    InternalBoundaryCoupling, InternalBoundaryId, InternalBoundarySide, OuterBoundaryCondition,
    OuterBoundaryConditions, OuterSide, Point2, RegionId, Scene, TriMesh, WaveCoefficients,
    WaveError,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoundaryLoad {
    pub signal: BoundarySignal,
    pub normalized_weight: f64,
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
    dirichlet_signals: Vec<Option<BoundarySignal>>,
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
        let mut coefficients = BTreeMap::new();
        for region in &scene.regions {
            let material = scene
                .material(region.material)
                .ok_or(WaveError::InvalidCoefficients)?;
            let values = WaveCoefficients {
                mass_density: material.mass_density,
                stiffness: material.stiffness,
                damping: material.damping,
            };
            validate_coefficients(values)?;
            coefficients.insert(region.id, values);
        }
        let outer = *coefficients
            .get(&BACKGROUND_REGION)
            .ok_or(WaveError::InvalidCoefficients)?;
        Self::assemble_regions(mesh, coefficients, outer, outer_boundaries, Some(scene))
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
        if mesh.vertices.is_empty() || mesh.triangles.is_empty() {
            return Err(WaveError::InvalidMesh("the mesh is empty"));
        }
        let mut node_points = mesh
            .vertices
            .iter()
            .map(|vertex| vertex.point)
            .collect::<Vec<_>>();
        if node_points.iter().any(|point| !point.finite()) {
            return Err(WaveError::InvalidMesh("a vertex is non-finite"));
        }

        let mut edge_nodes = BTreeMap::<(usize, usize), usize>::new();
        let mut local_nodes = Vec::with_capacity(mesh.triangles.len());
        for triangle in &mesh.triangles {
            let [a, b, c] = triangle.vertices;
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
            let points = [node_points[a], node_points[b], node_points[c]];
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
                edge[slot] = *edge_nodes.entry(key).or_insert_with(|| {
                    let index = node_points.len();
                    node_points.push((node_points[key.0] + node_points[key.1]) / 2.0);
                    index
                });
            }
            let centroid = node_points.len();
            node_points.push((points[0] + points[1] + points[2]) / 3.0);
            local_nodes.push([a, b, c, edge[0], edge[1], edge[2], centroid]);
        }
        if node_points.len() > u32::MAX as usize {
            return Err(WaveError::InvalidMesh(
                "the quadratic operator has too many degrees of freedom",
            ));
        }

        let count = node_points.len();
        let mut rows = vec![BTreeMap::<usize, f64>::new(); count];
        let mut auxiliary_rows = vec![BTreeMap::<usize, f64>::new(); count];
        let mut auxiliary_active = vec![false; count];
        let mut dirichlet_sides = vec![None; count];
        let mut dirichlet_signals = vec![None; count];
        let mut neumann_weights = vec![[0.0; 4]; count];
        let mut face_neumann_loads = vec![[BoundaryLoad::default(); 2]; count];
        let mut mass = vec![0.0; count];
        let mut damping = vec![0.0; count];
        for (triangle, indices) in mesh.triangles.iter().zip(&local_nodes) {
            let coefficients = *coefficients_by_region
                .get(&triangle.region)
                .ok_or(WaveError::InvalidMesh("a triangle has an unknown region"))?;
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
                mass[indices[local]] += coefficients.mass_density * area * MASS_WEIGHTS[local];
                damping[indices[local]] += coefficients.damping * area * MASS_WEIGHTS[local];
            }

            let mut local_stiffness = [[0.0; 7]; 7];
            for (barycentric, weight) in stiffness_quadrature() {
                let gradients = basis_gradients(barycentric, barycentric_gradients);
                for i in 0..7 {
                    for j in 0..7 {
                        local_stiffness[i][j] += weight * gradients[i].dot(gradients[j]);
                    }
                }
            }
            for i in 0..7 {
                for j in 0..7 {
                    *rows[indices[i]].entry(indices[j]).or_default() +=
                        coefficients.stiffness * area * local_stiffness[i][j];
                }
            }
        }
        let impedance = (outer_coefficients.mass_density * outer_coefficients.stiffness).sqrt();
        let wave_speed = (outer_coefficients.stiffness / outer_coefficients.mass_density).sqrt();
        let auxiliary_scale = 0.5 * outer_coefficients.stiffness * wave_speed;
        let mut visited = BTreeSet::new();
        for boundary in &mesh.boundary_edges {
            let BoundaryLabel::Outer(side) = boundary.label else {
                continue;
            };
            let condition = outer_boundaries.get(side);
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
            let midpoint = *edge_nodes.get(&key).ok_or(WaveError::InvalidMesh(
                "an outer boundary edge does not belong to a triangle",
            ))?;
            let length = (mesh.vertices[b].point - mesh.vertices[a].point).norm();
            if !length.is_finite() || length <= 0.0 {
                return Err(WaveError::InvalidMesh(
                    "an outer boundary edge has invalid length",
                ));
            }
            let nodes = [a, b, midpoint];
            let line_weights = [length / 6.0, length / 6.0, 2.0 * length / 3.0];
            match condition {
                OuterBoundaryCondition::Reflecting => {}
                OuterBoundaryCondition::Neumann { .. } => {
                    for (node, weight) in nodes.into_iter().zip(line_weights) {
                        neumann_weights[node][side.index()] += weight;
                    }
                }
                OuterBoundaryCondition::Dirichlet { signal } => {
                    for node in nodes {
                        if let Some(previous_side) = dirichlet_sides[node]
                            && outer_boundaries.get(previous_side).signal() != Some(signal)
                        {
                            return Err(WaveError::InvalidMesh(
                                "adjacent Dirichlet sides disagree at their shared corner",
                            ));
                        }
                        assign_dirichlet(&mut dirichlet_signals, node, signal)?;
                        dirichlet_sides[node] = Some(side);
                    }
                }
                OuterBoundaryCondition::FirstOrderOutgoing
                | OuterBoundaryCondition::SecondOrderOutgoing => {
                    let scale = impedance * length;
                    damping[a] += scale / 6.0;
                    damping[midpoint] += 2.0 * scale / 3.0;
                    damping[b] += scale / 6.0;
                }
            }
            if condition == OuterBoundaryCondition::SecondOrderOutgoing {
                // P2 line-element stiffness in endpoint/endpoint/midpoint order.
                // Sharing vertex indices across incident sides supplies the
                // corner coupling in the assembled tangential operator.
                let local = [[7.0, 1.0, -8.0], [1.0, 7.0, -8.0], [-8.0, -8.0, 16.0]];
                for i in 0..3 {
                    auxiliary_active[nodes[i]] = true;
                    for j in 0..3 {
                        *auxiliary_rows[nodes[i]].entry(nodes[j]).or_default() +=
                            auxiliary_scale * local[i][j] / (3.0 * length);
                    }
                }
            }
        }
        if let Some(scene) = scene {
            let mut assembly = BoundaryAssembly {
                edge_nodes: &edge_nodes,
                rows: &mut rows,
                auxiliary_rows: &mut auxiliary_rows,
                auxiliary_active: &mut auxiliary_active,
                dirichlet_signals: &mut dirichlet_signals,
                face_neumann_loads: &mut face_neumann_loads,
                damping: &mut damping,
            };
            assemble_hole_boundary_conditions(mesh, scene, &coefficients_by_region, &mut assembly)?;
            assemble_internal_boundary_laws(mesh, scene, &coefficients_by_region, &mut assembly)?;
        }
        if mass.iter().any(|value| !value.is_finite() || *value <= 0.0)
            || damping
                .iter()
                .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err(WaveError::InvalidMesh(
                "a quadratic node has invalid lumped mass",
            ));
        }
        for (weights, mass) in neumann_weights.iter_mut().zip(&mass) {
            for weight in weights {
                *weight /= *mass;
            }
        }
        for (loads, mass) in face_neumann_loads.iter_mut().zip(&mass) {
            for load in loads {
                load.normalized_weight /= *mass;
            }
        }

        for (row, auxiliary) in rows.iter_mut().zip(&auxiliary_rows) {
            for column in auxiliary.keys() {
                row.entry(*column).or_default();
            }
        }
        let entries = rows.iter().map(BTreeMap::len).sum::<usize>();
        if entries > u32::MAX as usize {
            return Err(WaveError::InvalidMesh(
                "the quadratic operator has too many matrix entries",
            ));
        }
        let mut row_offsets = Vec::with_capacity(count + 1);
        let mut columns = Vec::with_capacity(entries);
        let mut stiffness = Vec::with_capacity(entries);
        let mut auxiliary_stiffness = Vec::with_capacity(entries);
        let mut maximum_eigenvalue_bound = 0.0_f64;
        row_offsets.push(0);
        for (row, values) in rows.into_iter().enumerate() {
            maximum_eigenvalue_bound = maximum_eigenvalue_bound
                .max(values.values().map(|value| value.abs()).sum::<f64>() / mass[row]);
            for (column, value) in values {
                if !value.is_finite() {
                    return Err(WaveError::InvalidMesh("the stiffness matrix is non-finite"));
                }
                columns.push(column as u32);
                stiffness.push(value);
                auxiliary_stiffness.push(
                    auxiliary_rows[row]
                        .get(&column)
                        .copied()
                        .unwrap_or_default(),
                );
            }
            row_offsets.push(columns.len() as u32);
        }
        if !maximum_eigenvalue_bound.is_finite() || maximum_eigenvalue_bound <= 0.0 {
            return Err(WaveError::InvalidMesh(
                "the quadratic stiffness bound is not positive",
            ));
        }
        let maximum_time_step = 2.0 / maximum_eigenvalue_bound.sqrt();
        let element_nodes = local_nodes
            .into_iter()
            .map(|indices| indices.map(|index| index as u32))
            .collect();
        Ok(Self {
            geometry_revision: mesh.geometry_revision,
            mesh_revision: mesh.mesh_revision,
            outer_boundaries,
            node_points,
            element_nodes,
            row_offsets,
            columns,
            stiffness,
            auxiliary_stiffness,
            auxiliary_active,
            dirichlet_sides,
            dirichlet_signals,
            normalized_neumann_weights: neumann_weights,
            face_neumann_loads,
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

    pub fn dirichlet_signals(&self) -> &[Option<BoundarySignal>] {
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
        self.row_offsets.len() * 4 + self.columns.len() * 12 + self.degrees_of_freedom() * 96
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
            let stiffness = (operator.row_offsets[i] as usize
                ..operator.row_offsets[i + 1] as usize)
                .map(|entry| {
                    operator.stiffness[entry] * self.current[operator.columns[entry] as usize]
                })
                .sum::<f64>();
            let auxiliary = (operator.row_offsets[i] as usize
                ..operator.row_offsets[i + 1] as usize)
                .map(|entry| {
                    operator.auxiliary_stiffness[entry]
                        * self.auxiliary[operator.columns[entry] as usize]
                })
                .sum::<f64>();
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

struct BoundaryAssembly<'a> {
    edge_nodes: &'a BTreeMap<(usize, usize), usize>,
    rows: &'a mut [BTreeMap<usize, f64>],
    auxiliary_rows: &'a mut [BTreeMap<usize, f64>],
    auxiliary_active: &'a mut [bool],
    dirichlet_signals: &'a mut [Option<BoundarySignal>],
    face_neumann_loads: &'a mut [[BoundaryLoad; 2]],
    damping: &'a mut [f64],
}

fn assemble_hole_boundary_conditions(
    mesh: &TriMesh,
    scene: &Scene,
    coefficients_by_region: &BTreeMap<RegionId, WaveCoefficients>,
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
        let coefficients = *coefficients_by_region
            .get(&exterior)
            .ok_or(WaveError::InvalidCoefficients)?;
        let length = (mesh.vertices[b].point - mesh.vertices[a].point).norm();
        if !length.is_finite() || length <= 0.0 {
            return Err(WaveError::InvalidMesh(
                "a hole boundary edge has invalid length",
            ));
        }
        assemble_face_condition(condition, [a, midpoint, b], length, coefficients, assembly)?;
    }
    Ok(())
}

fn assemble_internal_boundary_laws(
    mesh: &TriMesh,
    scene: &Scene,
    coefficients_by_region: &BTreeMap<RegionId, WaveCoefficients>,
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
        let coefficients = *coefficients_by_region
            .get(&boundary.region)
            .ok_or(WaveError::InvalidCoefficients)?;
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
        assemble_face_condition(condition, nodes, length, coefficients, assembly)?;
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
        let coefficients = coefficients_by_region[&boundary.region];
        let spring = stiffness_ratio * coefficients.stiffness;
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
    coefficients: WaveCoefficients,
    assembly: &mut BoundaryAssembly<'_>,
) -> Result<(), WaveError> {
    let weights = [1.0 / 6.0, 2.0 / 3.0, 1.0 / 6.0];
    match condition {
        FaceBoundaryCondition::Reflecting => {}
        FaceBoundaryCondition::Impedance { ratio } => {
            let impedance = ratio * (coefficients.mass_density * coefficients.stiffness).sqrt();
            for (node, weight) in nodes.into_iter().zip(weights) {
                assembly.damping[node] += impedance * length * weight;
            }
        }
        FaceBoundaryCondition::SecondOrderOutgoing => {
            let impedance = (coefficients.mass_density * coefficients.stiffness).sqrt();
            for (node, weight) in nodes.into_iter().zip(weights) {
                assembly.damping[node] += impedance * length * weight;
            }
            let wave_speed = (coefficients.stiffness / coefficients.mass_density).sqrt();
            assemble_auxiliary_line(
                nodes,
                length,
                0.5 * coefficients.stiffness * wave_speed,
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
    }
    Ok(())
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
    signals: &mut [Option<BoundarySignal>],
    node: usize,
    signal: BoundarySignal,
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
    signal: BoundarySignal,
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

fn basis_gradients(barycentric: [f64; 3], gradients: [Point2; 3]) -> [Point2; 7] {
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
        BACKGROUND_REGION, BoundaryEdge, InternalBoundary, InternalBoundaryLaw, Material,
        MaterialId, MeshQuality, MeshTriangle, MeshVertex, MeshingOptions, Obstacle, ObstacleId,
        OpenCubicSpline, OuterSide, PeriodicCubicSpline, Region, mesh_scene,
    };

    fn two_material_scene() -> Scene {
        Scene {
            obstacles: vec![Obstacle::with_role(
                ObstacleId(1),
                PeriodicCubicSpline::rounded(Point2::new(0.5, 0.5), 0.2),
                crate::LoopRole::MaterialInterface {
                    exterior: BACKGROUND_REGION,
                    interior: RegionId(2),
                },
            )],
            internal_boundaries: vec![],
            materials: vec![
                Material {
                    id: MaterialId(1),
                    name: "Left".into(),
                    mass_density: 2.0,
                    stiffness: 3.0,
                    damping: 0.5,
                    color: [1, 2, 3],
                },
                Material {
                    id: MaterialId(2),
                    name: "Right".into(),
                    mass_density: 4.0,
                    stiffness: 7.0,
                    damping: 1.5,
                    color: [4, 5, 6],
                },
            ],
            regions: vec![
                Region {
                    id: BACKGROUND_REGION,
                    material: MaterialId(1),
                },
                Region {
                    id: RegionId(2),
                    material: MaterialId(2),
                },
            ],
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
        let dirichlet = crate::BoundarySignal {
            offset: 0.2,
            amplitude: 0.3,
            frequency_hz: 1.25,
            phase_radians: 0.4,
        };
        let neumann = crate::BoundarySignal {
            offset: 0.7,
            ..crate::BoundarySignal::ZERO
        };
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
            signal: crate::BoundarySignal {
                offset: 9.0,
                ..crate::BoundarySignal::ZERO
            },
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
        let dirichlet = BoundarySignal {
            offset: 0.35,
            amplitude: 0.2,
            frequency_hz: 1.5,
            phase_radians: 0.4,
        };
        let neumann = BoundarySignal {
            offset: 1.25,
            ..BoundarySignal::ZERO
        };
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
    }
}
