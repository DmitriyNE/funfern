use std::collections::{BTreeMap, BTreeSet};

use crate::{BoundaryLabel, OuterBoundaryCondition, Point2, TriMesh, WaveCoefficients, WaveError};

/// Seven-node mass-lumped triangle: `P2` enriched by the cubic interior bubble.
/// Vertex, edge-midpoint, and centroid masses use the positive degree-three nodal
/// quadrature weights 1/20, 2/15, and 9/20 of the element area.
#[derive(Clone, Debug, PartialEq)]
pub struct QuadraticWaveOperator {
    geometry_revision: u64,
    outer_boundary: OuterBoundaryCondition,
    node_points: Vec<Point2>,
    element_nodes: Vec<[u32; 7]>,
    row_offsets: Vec<u32>,
    columns: Vec<u32>,
    stiffness: Vec<f64>,
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
        validate_coefficients(coefficients)?;
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
        let mut mass = vec![0.0; count];
        let mut damping = vec![0.0; count];
        for (triangle, indices) in mesh.triangles.iter().zip(&local_nodes) {
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
        if outer_boundary == OuterBoundaryCondition::FirstOrderOutgoing {
            let impedance = (coefficients.mass_density * coefficients.stiffness).sqrt();
            let mut visited = BTreeSet::new();
            for boundary in &mesh.boundary_edges {
                if !matches!(boundary.label, BoundaryLabel::Outer(_)) {
                    continue;
                }
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
                let scale = impedance * length;
                damping[a] += scale / 6.0;
                damping[midpoint] += 2.0 * scale / 3.0;
                damping[b] += scale / 6.0;
            }
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

        let entries = rows.iter().map(BTreeMap::len).sum::<usize>();
        if entries > u32::MAX as usize {
            return Err(WaveError::InvalidMesh(
                "the quadratic operator has too many matrix entries",
            ));
        }
        let mut row_offsets = Vec::with_capacity(count + 1);
        let mut columns = Vec::with_capacity(entries);
        let mut stiffness = Vec::with_capacity(entries);
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
            outer_boundary,
            node_points,
            element_nodes,
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

    pub fn outer_boundary(&self) -> OuterBoundaryCondition {
        self.outer_boundary
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
        self.row_offsets.len() * 4 + self.columns.len() * 8 + self.degrees_of_freedom() * 32
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
        operator.discrete_energy(&self.current, &self.previous, self.time_step)
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
        for i in 0..self.current.len() {
            let stiffness = (operator.row_offsets[i] as usize
                ..operator.row_offsets[i + 1] as usize)
                .map(|entry| {
                    operator.stiffness[entry] * self.current[operator.columns[entry] as usize]
                })
                .sum::<f64>();
            let gamma = operator.lumped_damping[i] / operator.lumped_mass[i];
            let source = acceleration.get(i).copied().unwrap_or(0.0);
            self.scratch[i] = (2.0 * self.current[i]
                - (1.0 - 0.5 * gamma * dt) * self.previous[i]
                - dt2 * stiffness / operator.lumped_mass[i]
                + dt2 * source)
                / (1.0 + 0.5 * gamma * dt);
            if !self.scratch[i].is_finite() {
                return Err(WaveError::InvalidState);
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
    use crate::{BoundaryEdge, MeshQuality, MeshTriangle, MeshVertex, OuterSide};

    fn square() -> TriMesh {
        TriMesh {
            geometry_revision: 9,
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
                },
                MeshTriangle {
                    vertices: [0, 2, 3],
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
    }

    #[test]
    fn outgoing_boundary_rejects_malformed_edges() {
        let mut mesh = square_with_outer_boundary();
        mesh.boundary_edges[0].vertices = [0, 99];
        assert!(matches!(
            QuadraticWaveOperator::assemble_with_boundary(
                &mesh,
                WaveCoefficients::default(),
                OuterBoundaryCondition::FirstOrderOutgoing,
            ),
            Err(WaveError::InvalidMesh(_))
        ));
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
    }
}
