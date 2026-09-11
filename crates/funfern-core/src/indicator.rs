use std::{collections::BTreeMap, sync::Arc};

use crate::{
    BoundaryLabel, BoundarySide, FaceBoundaryCondition, InternalBoundaryCoupling,
    InternalBoundaryId, InternalBoundarySide, LoopRole, MeshSizeField, OuterBoundaryCondition,
    Point2, QuadraticWaveOperator, RegionId, Scene, TriMesh, enriched_quadratic_basis,
    enriched_quadratic_basis_gradients, enriched_quadratic_basis_laplacians,
};

#[derive(Clone, Debug, PartialEq)]
pub struct QuadraticSolutionSnapshot {
    pub mesh_revision: u64,
    pub displacement: Vec<f64>,
    pub velocity: Vec<f64>,
    pub acceleration: Vec<f64>,
    /// Second-order boundary memory aligned with `time` and `displacement`.
    pub auxiliary: Vec<f64>,
    pub volume_acceleration: Vec<f64>,
    pub time: f64,
    pub time_step: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolutionIndicatorOptions {
    pub minimum_edge_length: f64,
    pub maximum_edge_length: f64,
    pub relative_tolerance: f64,
    pub elements_per_wavelength: f64,
    pub forcing_frequency_hz: f64,
    pub grading_ratio: f64,
    pub minimum_scale: f64,
    pub maximum_scale: f64,
    pub coarsen_ratio: f64,
    pub amplitude_floor: f64,
    pub max_work_units: usize,
}

impl Default for SolutionIndicatorOptions {
    fn default() -> Self {
        Self {
            minimum_edge_length: 0.02,
            maximum_edge_length: 0.16,
            relative_tolerance: 0.06,
            elements_per_wavelength: 5.0,
            forcing_frequency_hz: 0.0,
            grading_ratio: 1.5,
            minimum_scale: 0.6,
            maximum_scale: 2.2,
            coarsen_ratio: 0.65,
            amplitude_floor: 1.0e-8,
            max_work_units: 5_000_000,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SolutionIndicatorReport {
    pub work_units: usize,
    pub minimum_indicator: f64,
    pub maximum_indicator: f64,
    pub minimum_target: f64,
    pub maximum_target: f64,
    pub refine_candidates: usize,
    pub coarsen_candidates: usize,
    pub recovery_contribution: f64,
    pub cell_residual_contribution: f64,
    pub interior_jump_contribution: f64,
    pub boundary_residual_contribution: f64,
    pub boundary_edges_evaluated: usize,
    pub maximum_dirichlet_mismatch: f64,
}

#[derive(Clone, Debug)]
pub struct SolutionIndicatorResult {
    pub field: Arc<AdaptiveSizeField>,
    pub element_indicators: Vec<f64>,
    pub element_targets: Vec<f64>,
    pub report: SolutionIndicatorReport,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SolutionIndicatorError {
    InvalidOptions,
    InvalidMesh,
    InvalidOperator,
    InvalidScene,
    InvalidSnapshot,
    WorkLimit,
}

impl std::fmt::Display for SolutionIndicatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::InvalidOptions => "Invalid solution-indicator options",
                Self::InvalidMesh => "Invalid solution-indicator mesh",
                Self::InvalidOperator => "Wave operator does not match the indicator mesh",
                Self::InvalidScene => "Scene coefficients do not match the indicator mesh",
                Self::InvalidSnapshot => "Invalid solution-indicator field snapshot",
                Self::WorkLimit => "Solution-indicator work limit reached",
            }
        )
    }
}

impl std::error::Error for SolutionIndicatorError {}

#[derive(Clone, Debug)]
pub struct AdaptiveSizeField {
    mesh: Arc<TriMesh>,
    triangle_targets: Vec<[f64; 3]>,
    bins: Vec<Vec<u32>>,
    dimension: usize,
    minimum: Point2,
    cell: Point2,
    fallback: f64,
    lower_bound: f64,
    upper_bound: f64,
}

impl AdaptiveSizeField {
    fn new(
        mesh: Arc<TriMesh>,
        triangle_targets: Vec<[f64; 3]>,
        lower_bound: f64,
        upper_bound: f64,
    ) -> Self {
        let mut minimum = Point2::new(f64::INFINITY, f64::INFINITY);
        let mut maximum = Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
        for vertex in &mesh.vertices {
            minimum.x = minimum.x.min(vertex.point.x);
            minimum.y = minimum.y.min(vertex.point.y);
            maximum.x = maximum.x.max(vertex.point.x);
            maximum.y = maximum.y.max(vertex.point.y);
        }
        let dimension = (mesh.triangles.len() as f64).sqrt().ceil() as usize;
        let dimension = dimension.clamp(8, 128);
        let extent = Point2::new(
            (maximum.x - minimum.x).max(1.0e-12),
            (maximum.y - minimum.y).max(1.0e-12),
        );
        let cell = extent / dimension as f64;
        let mut bins = vec![Vec::new(); dimension * dimension];
        for (index, triangle) in mesh.triangles.iter().enumerate() {
            let points = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
            let lo = Point2::new(
                points
                    .iter()
                    .map(|point| point.x)
                    .fold(f64::INFINITY, f64::min),
                points
                    .iter()
                    .map(|point| point.y)
                    .fold(f64::INFINITY, f64::min),
            );
            let hi = Point2::new(
                points
                    .iter()
                    .map(|point| point.x)
                    .fold(f64::NEG_INFINITY, f64::max),
                points
                    .iter()
                    .map(|point| point.y)
                    .fold(f64::NEG_INFINITY, f64::max),
            );
            let [x0, y0] = bin_index(lo, minimum, cell, dimension);
            let [x1, y1] = bin_index(hi, minimum, cell, dimension);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    bins[y * dimension + x].push(index as u32);
                }
            }
        }
        Self {
            mesh,
            triangle_targets,
            bins,
            dimension,
            minimum,
            cell,
            fallback: upper_bound,
            lower_bound,
            upper_bound,
        }
    }

    pub fn element_target(&self, index: usize) -> Option<f64> {
        self.triangle_targets
            .get(index)
            .map(|values| values.iter().sum::<f64>() / 3.0)
    }
}

impl MeshSizeField for AdaptiveSizeField {
    fn target_edge_length(&self, point: Point2, region: RegionId) -> f64 {
        let [x, y] = bin_index(point, self.minimum, self.cell, self.dimension);
        let mut target = None::<f64>;
        for &triangle_index in &self.bins[y * self.dimension + x] {
            let index = triangle_index as usize;
            let triangle = self.mesh.triangles[index];
            if triangle.region != region {
                continue;
            }
            let points = triangle
                .vertices
                .map(|vertex| self.mesh.vertices[vertex].point);
            if let Some(weights) = barycentric(point, points) {
                let value = weights
                    .into_iter()
                    .zip(self.triangle_targets[index])
                    .map(|(weight, value)| weight * value)
                    .sum::<f64>();
                target = Some(target.map_or(value, |current| current.min(value)));
            }
        }
        target
            .unwrap_or(self.fallback)
            .clamp(self.lower_bound, self.upper_bound)
    }
}

#[derive(Clone, Copy, Default)]
struct Recovery {
    displacement: Point2,
    velocity: Point2,
    weight: f64,
}

#[derive(Clone, Copy, Default)]
struct ElementEstimate {
    recovery: f64,
    cell_residual: f64,
    interior_jump: f64,
    boundary_residual: f64,
    energy: f64,
    area: f64,
    edge: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct BoundaryPairKey {
    id: InternalBoundaryId,
    start: u64,
    end: u64,
}

#[derive(Clone, Copy)]
struct BoundaryRecord {
    triangle: usize,
    nodes: [usize; 3],
    condition: FaceBoundaryCondition,
    mass_density: f64,
    stiffness: f64,
    pair: Option<(BoundaryPairKey, usize, f64)>,
}

enum IndicatorPhase {
    Validate,
    BuildEdges(usize),
    BuildBoundary(usize),
    Recover(usize),
    Estimate(usize),
    Jump(usize),
    Boundary(usize),
    Target(usize),
    Grade { pass: usize, edge: usize },
    Finish,
    Done,
}

pub struct SolutionIndicatorJob {
    mesh: Arc<TriMesh>,
    operator: Arc<QuadraticWaveOperator>,
    scene: Scene,
    snapshot: QuadraticSolutionSnapshot,
    options: SolutionIndicatorOptions,
    phase: IndicatorPhase,
    edges: BTreeMap<(usize, usize), Vec<usize>>,
    edge_list: Vec<((usize, usize), Vec<usize>)>,
    boundary_records: Vec<BoundaryRecord>,
    boundary_pairs: BTreeMap<BoundaryPairKey, [Option<usize>; 2]>,
    recovery: BTreeMap<(usize, RegionId), Recovery>,
    estimates: Vec<ElementEstimate>,
    indicators: Vec<f64>,
    targets: Vec<f64>,
    total_energy: f64,
    total_area: f64,
    report: SolutionIndicatorReport,
}

impl SolutionIndicatorJob {
    pub fn new(
        mesh: Arc<TriMesh>,
        operator: Arc<QuadraticWaveOperator>,
        scene: Scene,
        snapshot: QuadraticSolutionSnapshot,
        options: SolutionIndicatorOptions,
    ) -> Self {
        let count = mesh.triangles.len();
        Self {
            mesh,
            operator,
            scene,
            snapshot,
            options,
            phase: IndicatorPhase::Validate,
            edges: BTreeMap::new(),
            edge_list: Vec::new(),
            boundary_records: Vec::new(),
            boundary_pairs: BTreeMap::new(),
            recovery: BTreeMap::new(),
            estimates: vec![ElementEstimate::default(); count],
            indicators: vec![0.0; count],
            targets: vec![0.0; count],
            total_energy: 0.0,
            total_area: 0.0,
            report: SolutionIndicatorReport {
                minimum_indicator: f64::INFINITY,
                minimum_target: f64::INFINITY,
                ..Default::default()
            },
        }
    }

    pub fn phase(&self) -> &'static str {
        match self.phase {
            IndicatorPhase::Validate
            | IndicatorPhase::BuildEdges(_)
            | IndicatorPhase::BuildBoundary(_) => "Preparing AMR estimate",
            IndicatorPhase::Recover(_) => "Recovering wave gradients",
            IndicatorPhase::Estimate(_) | IndicatorPhase::Jump(_) | IndicatorPhase::Boundary(_) => {
                "Estimating wave error"
            }
            IndicatorPhase::Target(_) | IndicatorPhase::Grade { .. } => "Grading AMR target",
            IndicatorPhase::Finish | IndicatorPhase::Done => "AMR estimate ready",
        }
    }

    pub fn report(&self) -> &SolutionIndicatorReport {
        &self.report
    }

    pub fn advance(
        &mut self,
        budget: usize,
    ) -> Option<Result<SolutionIndicatorResult, SolutionIndicatorError>> {
        for _ in 0..budget {
            if matches!(self.phase, IndicatorPhase::Done) {
                return None;
            }
            self.report.work_units += 1;
            if self.report.work_units > self.options.max_work_units {
                self.phase = IndicatorPhase::Done;
                return Some(Err(SolutionIndicatorError::WorkLimit));
            }
            match self.step() {
                Ok(Some(result)) => {
                    self.phase = IndicatorPhase::Done;
                    return Some(Ok(result));
                }
                Ok(None) => {}
                Err(error) => {
                    self.phase = IndicatorPhase::Done;
                    return Some(Err(error));
                }
            }
        }
        None
    }

    fn step(&mut self) -> Result<Option<SolutionIndicatorResult>, SolutionIndicatorError> {
        let phase = std::mem::replace(&mut self.phase, IndicatorPhase::Done);
        match phase {
            IndicatorPhase::Validate => {
                self.validate()?;
                self.phase = IndicatorPhase::BuildEdges(0);
            }
            IndicatorPhase::BuildEdges(index) => {
                if index == self.mesh.triangles.len() {
                    self.edge_list = std::mem::take(&mut self.edges).into_iter().collect();
                    self.phase = IndicatorPhase::BuildBoundary(0);
                } else {
                    let vertices = self.mesh.triangles[index].vertices;
                    for edge in [
                        [vertices[0], vertices[1]],
                        [vertices[1], vertices[2]],
                        [vertices[2], vertices[0]],
                    ] {
                        self.edges.entry(edge_key(edge)).or_default().push(index);
                    }
                    self.phase = IndicatorPhase::BuildEdges(index + 1);
                }
            }
            IndicatorPhase::BuildBoundary(index) => self.build_boundary(index)?,
            IndicatorPhase::Recover(index) => self.recover(index)?,
            IndicatorPhase::Estimate(index) => self.estimate(index)?,
            IndicatorPhase::Jump(index) => self.jump(index)?,
            IndicatorPhase::Boundary(index) => self.boundary(index)?,
            IndicatorPhase::Target(index) => self.target(index)?,
            IndicatorPhase::Grade { pass, edge } => self.grade(pass, edge),
            IndicatorPhase::Finish => return Ok(Some(self.finish())),
            IndicatorPhase::Done => unreachable!(),
        }
        Ok(None)
    }

    fn validate(&self) -> Result<(), SolutionIndicatorError> {
        let options = self.options;
        if !options.minimum_edge_length.is_finite()
            || !options.maximum_edge_length.is_finite()
            || options.minimum_edge_length <= 0.0
            || options.maximum_edge_length < options.minimum_edge_length
            || !options.relative_tolerance.is_finite()
            || options.relative_tolerance <= 0.0
            || !options.elements_per_wavelength.is_finite()
            || options.elements_per_wavelength <= 0.0
            || !options.forcing_frequency_hz.is_finite()
            || options.forcing_frequency_hz < 0.0
            || !options.grading_ratio.is_finite()
            || options.grading_ratio <= 1.0
            || !options.minimum_scale.is_finite()
            || !options.maximum_scale.is_finite()
            || options.minimum_scale <= 0.0
            || options.minimum_scale >= 1.0
            || options.maximum_scale <= 1.0
            || options.maximum_scale <= options.minimum_scale
            || !options.coarsen_ratio.is_finite()
            || options.coarsen_ratio <= 0.0
            || options.coarsen_ratio >= 1.0
            || !options.amplitude_floor.is_finite()
            || options.amplitude_floor <= 0.0
            || options.max_work_units == 0
        {
            return Err(SolutionIndicatorError::InvalidOptions);
        }
        if self.mesh.vertices.is_empty() || self.mesh.triangles.is_empty() {
            return Err(SolutionIndicatorError::InvalidMesh);
        }
        if self.operator.mesh_revision() != self.mesh.mesh_revision
            || self.operator.geometry_revision() != self.mesh.geometry_revision
            || self.operator.element_nodes().len() != self.mesh.triangles.len()
        {
            return Err(SolutionIndicatorError::InvalidOperator);
        }
        if !self.scene.structure_valid()
            || self
                .mesh
                .triangles
                .iter()
                .any(|triangle| self.scene.region_material(triangle.region).is_none())
        {
            return Err(SolutionIndicatorError::InvalidScene);
        }
        let count = self.operator.degrees_of_freedom();
        if self.snapshot.mesh_revision != self.mesh.mesh_revision
            || self.snapshot.displacement.len() != count
            || self.snapshot.velocity.len() != count
            || self.snapshot.acceleration.len() != count
            || self.snapshot.auxiliary.len() != count
            || self.snapshot.volume_acceleration.len() != count
            || !self.snapshot.time.is_finite()
            || !self.snapshot.time_step.is_finite()
            || self.snapshot.time_step <= 0.0
            || self
                .snapshot
                .displacement
                .iter()
                .chain(&self.snapshot.velocity)
                .chain(&self.snapshot.acceleration)
                .chain(&self.snapshot.auxiliary)
                .chain(&self.snapshot.volume_acceleration)
                .any(|value| !value.is_finite())
        {
            return Err(SolutionIndicatorError::InvalidSnapshot);
        }
        Ok(())
    }

    fn build_boundary(&mut self, index: usize) -> Result<(), SolutionIndicatorError> {
        if index == self.mesh.boundary_edges.len() {
            self.phase = IndicatorPhase::Recover(0);
            return Ok(());
        }
        let edge = self.mesh.boundary_edges[index];
        if edge.vertices[0] >= self.mesh.vertices.len()
            || edge.vertices[1] >= self.mesh.vertices.len()
            || edge.vertices[0] == edge.vertices[1]
            || !edge.parameters.iter().all(|value| value.is_finite())
            || edge.parameters[0] == edge.parameters[1]
        {
            return Err(SolutionIndicatorError::InvalidMesh);
        }
        let key = edge_key(edge.vertices);
        let sides = self
            .edge_list
            .binary_search_by_key(&key, |(candidate, _)| *candidate)
            .ok()
            .map(|found| &self.edge_list[found].1)
            .ok_or(SolutionIndicatorError::InvalidMesh)?;
        if matches!(edge.label, BoundaryLabel::MaterialInterface(_)) {
            if sides.len() != 2 {
                return Err(SolutionIndicatorError::InvalidMesh);
            }
            self.phase = IndicatorPhase::BuildBoundary(index + 1);
            return Ok(());
        }
        if sides.len() != 1 {
            return Err(SolutionIndicatorError::InvalidMesh);
        }
        let triangle = sides[0];
        let region = self.mesh.triangles[triangle].region;
        let material = self
            .scene
            .region_material(region)
            .ok_or(SolutionIndicatorError::InvalidScene)?;
        let raw_nodes = self.boundary_edge_nodes(triangle, edge.vertices)?;
        let nodes = if edge.parameters[0] < edge.parameters[1] {
            raw_nodes
        } else {
            [raw_nodes[2], raw_nodes[1], raw_nodes[0]]
        };
        let (condition, pair) = match edge.label {
            BoundaryLabel::Outer(side) => {
                if region != crate::BACKGROUND_REGION {
                    return Err(SolutionIndicatorError::InvalidMesh);
                }
                let condition = match self.scene.outer_boundaries.get(side) {
                    OuterBoundaryCondition::Reflecting => FaceBoundaryCondition::Reflecting,
                    OuterBoundaryCondition::FirstOrderOutgoing => {
                        FaceBoundaryCondition::Impedance { ratio: 1.0 }
                    }
                    OuterBoundaryCondition::SecondOrderOutgoing => {
                        FaceBoundaryCondition::SecondOrderOutgoing
                    }
                    OuterBoundaryCondition::Neumann { signal } => {
                        FaceBoundaryCondition::Neumann { signal }
                    }
                    OuterBoundaryCondition::Dirichlet { signal } => {
                        FaceBoundaryCondition::Dirichlet { signal }
                    }
                };
                (condition, None)
            }
            BoundaryLabel::Obstacle(id) => {
                let obstacle = self
                    .scene
                    .obstacles
                    .iter()
                    .find(|obstacle| obstacle.id == id)
                    .ok_or(SolutionIndicatorError::InvalidScene)?;
                let LoopRole::Hole { exterior } = obstacle.role else {
                    return Err(SolutionIndicatorError::InvalidMesh);
                };
                if exterior != region {
                    return Err(SolutionIndicatorError::InvalidMesh);
                }
                let span = obstacle
                    .spline
                    .span_index(0.5 * (edge.parameters[0] + edge.parameters[1]))
                    .ok_or(SolutionIndicatorError::InvalidMesh)?;
                (obstacle.span_conditions[span], None)
            }
            BoundaryLabel::Wall { loop_id, side } => {
                let obstacle = self
                    .scene
                    .obstacles
                    .iter()
                    .find(|obstacle| obstacle.id == loop_id)
                    .ok_or(SolutionIndicatorError::InvalidScene)?;
                let LoopRole::Wall { exterior, interior } = obstacle.role else {
                    return Err(SolutionIndicatorError::InvalidMesh);
                };
                let expected = match side {
                    BoundarySide::Exterior => exterior,
                    BoundarySide::Interior => interior,
                };
                if expected != region {
                    return Err(SolutionIndicatorError::InvalidMesh);
                }
                (FaceBoundaryCondition::Reflecting, None)
            }
            BoundaryLabel::InternalBoundary { id, side } => {
                let boundary = self
                    .scene
                    .internal_boundaries
                    .iter()
                    .find(|boundary| boundary.id == id)
                    .ok_or(SolutionIndicatorError::InvalidScene)?;
                if boundary.region != region {
                    return Err(SolutionIndicatorError::InvalidMesh);
                }
                let span = boundary
                    .spline
                    .span_index(0.5 * (edge.parameters[0] + edge.parameters[1]))
                    .ok_or(SolutionIndicatorError::InvalidMesh)?;
                let law = boundary.span_laws[span];
                let condition = match side {
                    InternalBoundarySide::Left => law.left,
                    InternalBoundarySide::Right => law.right,
                };
                let pair = match law.coupling {
                    InternalBoundaryCoupling::Independent => None,
                    InternalBoundaryCoupling::ThinGap { stiffness_ratio } => {
                        let start = edge.parameters[0].min(edge.parameters[1]).to_bits();
                        let end = edge.parameters[0].max(edge.parameters[1]).to_bits();
                        let slot = match side {
                            InternalBoundarySide::Left => 0,
                            InternalBoundarySide::Right => 1,
                        };
                        Some((
                            BoundaryPairKey { id, start, end },
                            slot,
                            stiffness_ratio * material.stiffness,
                        ))
                    }
                };
                (condition, pair)
            }
            BoundaryLabel::MaterialInterface(_) => unreachable!(),
        };
        let record_index = self.boundary_records.len();
        self.boundary_records.push(BoundaryRecord {
            triangle,
            nodes,
            condition,
            mass_density: material.mass_density,
            stiffness: material.stiffness,
            pair,
        });
        if let Some((key, slot, _)) = pair {
            let entry = self.boundary_pairs.entry(key).or_default();
            if entry[slot].replace(record_index).is_some() {
                return Err(SolutionIndicatorError::InvalidMesh);
            }
        }
        self.phase = IndicatorPhase::BuildBoundary(index + 1);
        Ok(())
    }

    fn boundary_edge_nodes(
        &self,
        triangle_index: usize,
        edge: [usize; 2],
    ) -> Result<[usize; 3], SolutionIndicatorError> {
        let triangle = self.mesh.triangles[triangle_index];
        let a = triangle
            .vertices
            .iter()
            .position(|vertex| *vertex == edge[0])
            .ok_or(SolutionIndicatorError::InvalidMesh)?;
        let b = triangle
            .vertices
            .iter()
            .position(|vertex| *vertex == edge[1])
            .ok_or(SolutionIndicatorError::InvalidMesh)?;
        let midpoint = match (a.min(b), a.max(b)) {
            (0, 1) => 3,
            (1, 2) => 4,
            (0, 2) => 5,
            _ => return Err(SolutionIndicatorError::InvalidMesh),
        };
        Ok([
            edge[0],
            self.operator.element_nodes()[triangle_index][midpoint] as usize,
            edge[1],
        ])
    }

    fn recover(&mut self, index: usize) -> Result<(), SolutionIndicatorError> {
        if index == self.mesh.triangles.len() {
            self.phase = IndicatorPhase::Estimate(0);
            return Ok(());
        }
        let triangle = self.mesh.triangles[index];
        let geometry = element_geometry(&self.mesh, triangle.vertices)?;
        let nodes = self.operator.element_nodes()[index].map(|node| node as usize);
        let material = self
            .scene
            .region_material(triangle.region)
            .ok_or(SolutionIndicatorError::InvalidScene)?;
        for local in 0..3 {
            let mut barycentric = [0.0; 3];
            barycentric[local] = 1.0;
            let gradients = enriched_quadratic_basis_gradients(barycentric, geometry.gradients);
            let displacement = gradient(&self.snapshot.displacement, nodes, gradients);
            let velocity = gradient(&self.snapshot.velocity, nodes, gradients);
            let entry = self
                .recovery
                .entry((triangle.vertices[local], triangle.region))
                .or_default();
            entry.displacement =
                entry.displacement + displacement * (material.stiffness * geometry.area);
            entry.velocity = entry.velocity + velocity * (material.stiffness * geometry.area);
            entry.weight += geometry.area;
        }
        self.phase = IndicatorPhase::Recover(index + 1);
        Ok(())
    }

    fn estimate(&mut self, index: usize) -> Result<(), SolutionIndicatorError> {
        if index == self.mesh.triangles.len() {
            self.phase = IndicatorPhase::Jump(0);
            return Ok(());
        }
        let triangle = self.mesh.triangles[index];
        let geometry = element_geometry(&self.mesh, triangle.vertices)?;
        let nodes = self.operator.element_nodes()[index].map(|node| node as usize);
        let material = self
            .scene
            .region_material(triangle.region)
            .ok_or(SolutionIndicatorError::InvalidScene)?;
        let omega = (std::f64::consts::TAU * self.options.forcing_frequency_hz).max(1.0);
        let recovered = triangle.vertices.map(|vertex| {
            let value = self.recovery[&(vertex, triangle.region)];
            (
                value.displacement / value.weight,
                value.velocity / value.weight,
            )
        });
        let mut estimate = ElementEstimate {
            area: geometry.area,
            edge: geometry.maximum_edge,
            ..Default::default()
        };
        for (barycentric, weight) in quadrature() {
            let basis = enriched_quadratic_basis(barycentric);
            let gradients = enriched_quadratic_basis_gradients(barycentric, geometry.gradients);
            let laplacians = enriched_quadratic_basis_laplacians(barycentric, geometry.gradients);
            let u = scalar(&self.snapshot.displacement, nodes, basis);
            let v = scalar(&self.snapshot.velocity, nodes, basis);
            let a = scalar(&self.snapshot.acceleration, nodes, basis);
            let source = scalar(&self.snapshot.volume_acceleration, nodes, basis);
            let grad_u = gradient(&self.snapshot.displacement, nodes, gradients);
            let grad_v = gradient(&self.snapshot.velocity, nodes, gradients);
            let laplace_u = scalar(&self.snapshot.displacement, nodes, laplacians);
            let recovered_u = recovered[0].0 * barycentric[0]
                + recovered[1].0 * barycentric[1]
                + recovered[2].0 * barycentric[2];
            let recovered_v = recovered[0].1 * barycentric[0]
                + recovered[1].1 * barycentric[1]
                + recovered[2].1 * barycentric[2];
            let flux_u = grad_u * material.stiffness;
            let flux_v = grad_v * material.stiffness;
            estimate.recovery += weight
                * geometry.area
                * ((flux_u - recovered_u).dot(flux_u - recovered_u)
                    + (flux_v - recovered_v).dot(flux_v - recovered_v) / (omega * omega))
                / material.stiffness;
            let residual = material.mass_density * (a - source) + material.damping * v
                - material.stiffness * laplace_u;
            estimate.cell_residual +=
                weight * geometry.area * geometry.maximum_edge.powi(2) * residual.powi(2)
                    / material.stiffness;
            estimate.energy += weight
                * geometry.area
                * (material.stiffness * grad_u.dot(grad_u)
                    + material.mass_density * (omega * omega * u * u + v * v));
        }
        self.total_energy += estimate.energy;
        self.total_area += estimate.area;
        self.estimates[index] = estimate;
        self.phase = IndicatorPhase::Estimate(index + 1);
        Ok(())
    }

    fn jump(&mut self, index: usize) -> Result<(), SolutionIndicatorError> {
        if index == self.edge_list.len() {
            self.phase = IndicatorPhase::Boundary(0);
            return Ok(());
        }
        let (edge, sides) = &self.edge_list[index];
        if sides.len() == 2 {
            let points = [
                self.mesh.vertices[edge.0].point,
                self.mesh.vertices[edge.1].point,
            ];
            let length = (points[1] - points[0]).norm();
            let mut integral = 0.0;
            for (fraction, weight) in [
                (0.112_701_665_379_258_3, 5.0 / 18.0),
                (0.5, 8.0 / 18.0),
                (0.887_298_334_620_741_7, 5.0 / 18.0),
            ] {
                let point = points[0].lerp(points[1], fraction);
                let mut jump = 0.0;
                for triangle_index in sides {
                    let triangle = self.mesh.triangles[*triangle_index];
                    let geometry = element_geometry(&self.mesh, triangle.vertices)?;
                    let barycentric = barycentric(point, geometry.points)
                        .ok_or(SolutionIndicatorError::InvalidMesh)?;
                    let gradients =
                        enriched_quadratic_basis_gradients(barycentric, geometry.gradients);
                    let nodes =
                        self.operator.element_nodes()[*triangle_index].map(|node| node as usize);
                    let grad = gradient(&self.snapshot.displacement, nodes, gradients);
                    let stiffness = self
                        .scene
                        .region_material(triangle.region)
                        .ok_or(SolutionIndicatorError::InvalidScene)?
                        .stiffness;
                    jump += stiffness * grad.dot(outward_normal(geometry.points, points));
                }
                integral += weight * length * jump * jump;
            }
            for triangle_index in sides {
                let stiffness = self
                    .scene
                    .region_material(self.mesh.triangles[*triangle_index].region)
                    .ok_or(SolutionIndicatorError::InvalidScene)?
                    .stiffness;
                self.estimates[*triangle_index].interior_jump +=
                    0.5 * length * integral / stiffness;
            }
        }
        self.phase = IndicatorPhase::Jump(index + 1);
        Ok(())
    }

    fn boundary(&mut self, index: usize) -> Result<(), SolutionIndicatorError> {
        if index == self.boundary_records.len() {
            self.phase = IndicatorPhase::Target(0);
            return Ok(());
        }
        let record = self.boundary_records[index];
        let triangle = self.mesh.triangles[record.triangle];
        let geometry = element_geometry(&self.mesh, triangle.vertices)?;
        let edge_points = [
            self.operator.node_points()[record.nodes[0]],
            self.operator.node_points()[record.nodes[2]],
        ];
        let length = (edge_points[1] - edge_points[0]).norm();
        if !length.is_finite() || length <= 0.0 {
            return Err(SolutionIndicatorError::InvalidMesh);
        }
        let normal = outward_normal(geometry.points, edge_points);
        let auxiliary_second =
            line_second_derivative(&self.snapshot.auxiliary, record.nodes, length);
        let paired_nodes = if let Some((key, slot, spring)) = record.pair {
            let pair = self
                .boundary_pairs
                .get(&key)
                .ok_or(SolutionIndicatorError::InvalidMesh)?;
            let partner_index = pair[1 - slot].ok_or(SolutionIndicatorError::InvalidMesh)?;
            let partner = self.boundary_records[partner_index];
            let Some((partner_key, partner_slot, partner_spring)) = partner.pair else {
                return Err(SolutionIndicatorError::InvalidMesh);
            };
            if partner_key != key
                || partner_slot == slot
                || (partner_spring - spring).abs() > 1.0e-12 * spring.abs().max(1.0)
                || [0, 2].into_iter().any(|endpoint| {
                    (self.operator.node_points()[record.nodes[endpoint]]
                        - self.operator.node_points()[partner.nodes[endpoint]])
                        .norm()
                        > 1.0e-10 * length.max(1.0)
                })
            {
                return Err(SolutionIndicatorError::InvalidMesh);
            }
            Some((partner.nodes, spring))
        } else {
            None
        };
        let mut integral = 0.0;
        for (fraction, weight) in line_quadrature() {
            let point = edge_points[0].lerp(edge_points[1], fraction);
            let barycentric =
                barycentric(point, geometry.points).ok_or(SolutionIndicatorError::InvalidMesh)?;
            let gradients = enriched_quadratic_basis_gradients(barycentric, geometry.gradients);
            let nodes = self.operator.element_nodes()[record.triangle].map(|node| node as usize);
            let flux = record.stiffness
                * gradient(&self.snapshot.displacement, nodes, gradients).dot(normal);
            let displacement = line_scalar(&self.snapshot.displacement, record.nodes, fraction);
            let velocity = line_scalar(&self.snapshot.velocity, record.nodes, fraction);
            if let FaceBoundaryCondition::Dirichlet { signal } = record.condition {
                self.report.maximum_dirichlet_mismatch = self
                    .report
                    .maximum_dirichlet_mismatch
                    .max((displacement - signal.value(self.snapshot.time)).abs());
                continue;
            }
            let mut residual = match record.condition {
                FaceBoundaryCondition::Reflecting => flux,
                FaceBoundaryCondition::Neumann { signal } => {
                    flux - signal.value(self.snapshot.time)
                }
                FaceBoundaryCondition::Impedance { ratio } => {
                    let impedance = ratio * (record.mass_density * record.stiffness).sqrt();
                    flux + impedance * velocity
                }
                FaceBoundaryCondition::SecondOrderOutgoing => {
                    let impedance = (record.mass_density * record.stiffness).sqrt();
                    let wave_speed = (record.stiffness / record.mass_density).sqrt();
                    flux + impedance * velocity
                        - 0.5 * record.stiffness * wave_speed * auxiliary_second
                }
                FaceBoundaryCondition::Dirichlet { .. } => unreachable!(),
            };
            if let Some((partner, spring)) = paired_nodes {
                residual += spring
                    * (displacement - line_scalar(&self.snapshot.displacement, partner, fraction));
            }
            integral += weight * length * residual * residual;
        }
        self.estimates[record.triangle].boundary_residual += length * integral / record.stiffness;
        self.report.boundary_edges_evaluated += 1;
        self.phase = IndicatorPhase::Boundary(index + 1);
        Ok(())
    }

    fn target(&mut self, index: usize) -> Result<(), SolutionIndicatorError> {
        if index == self.mesh.triangles.len() {
            self.phase = IndicatorPhase::Grade { pass: 0, edge: 0 };
            return Ok(());
        }
        let estimate = self.estimates[index];
        let floor =
            self.options.amplitude_floor * self.total_energy.max(f64::MIN_POSITIVE) * estimate.area
                / self.total_area.max(f64::MIN_POSITIVE);
        let residual = estimate.recovery
            + estimate.cell_residual
            + estimate.interior_jump
            + estimate.boundary_residual;
        let indicator = (residual / (estimate.energy + floor)).sqrt();
        let scale = if indicator <= f64::MIN_POSITIVE {
            self.options.maximum_scale
        } else {
            (self.options.relative_tolerance / indicator)
                .sqrt()
                .clamp(self.options.minimum_scale, self.options.maximum_scale)
        };
        let material = self
            .scene
            .region_material(self.mesh.triangles[index].region)
            .ok_or(SolutionIndicatorError::InvalidScene)?;
        let wavelength_target = if self.options.forcing_frequency_hz > 0.0 {
            (material.stiffness / material.mass_density).sqrt()
                / (self.options.forcing_frequency_hz * self.options.elements_per_wavelength)
        } else {
            self.options.maximum_edge_length
        };
        let target = (estimate.edge * scale).min(wavelength_target).clamp(
            self.options.minimum_edge_length,
            self.options.maximum_edge_length,
        );
        self.indicators[index] = indicator;
        self.targets[index] = target;
        self.report.minimum_indicator = self.report.minimum_indicator.min(indicator);
        self.report.maximum_indicator = self.report.maximum_indicator.max(indicator);
        self.phase = IndicatorPhase::Target(index + 1);
        Ok(())
    }

    fn grade(&mut self, pass: usize, edge: usize) {
        if pass == 2 {
            self.phase = IndicatorPhase::Finish;
            return;
        }
        if edge == self.edge_list.len() {
            self.phase = IndicatorPhase::Grade {
                pass: pass + 1,
                edge: 0,
            };
            return;
        }
        let sides = &self.edge_list[edge].1;
        if sides.len() == 2 {
            let [a, b] = [sides[0], sides[1]];
            let low = self.targets[a].min(self.targets[b]);
            let high = self.targets[a].max(self.targets[b]);
            if high > low * self.options.grading_ratio {
                if self.targets[a] > self.targets[b] {
                    self.targets[a] = low * self.options.grading_ratio;
                } else {
                    self.targets[b] = low * self.options.grading_ratio;
                }
            }
        }
        self.phase = IndicatorPhase::Grade {
            pass,
            edge: edge + 1,
        };
    }

    fn finish(&mut self) -> SolutionIndicatorResult {
        for estimate in &self.estimates {
            self.report.recovery_contribution += estimate.recovery;
            self.report.cell_residual_contribution += estimate.cell_residual;
            self.report.interior_jump_contribution += estimate.interior_jump;
            self.report.boundary_residual_contribution += estimate.boundary_residual;
        }
        let triangle_targets = self
            .targets
            .iter()
            .map(|target| [*target; 3])
            .collect::<Vec<_>>();
        self.report.minimum_target = self.targets.iter().copied().fold(f64::INFINITY, f64::min);
        self.report.maximum_target = self.targets.iter().copied().fold(0.0, f64::max);
        for (triangle, target) in self.mesh.triangles.iter().zip(&self.targets) {
            let points = triangle
                .vertices
                .map(|vertex| self.mesh.vertices[vertex].point);
            let lengths = [
                (points[1] - points[0]).norm(),
                (points[2] - points[1]).norm(),
                (points[0] - points[2]).norm(),
            ];
            if lengths.iter().copied().fold(0.0, f64::max) > 1.05 * target {
                self.report.refine_candidates += 1;
            }
            if lengths
                .iter()
                .any(|length| *length < self.options.coarsen_ratio * target)
            {
                self.report.coarsen_candidates += 1;
            }
        }
        let field = Arc::new(AdaptiveSizeField::new(
            self.mesh.clone(),
            triangle_targets,
            self.options.minimum_edge_length,
            self.options.maximum_edge_length,
        ));
        SolutionIndicatorResult {
            field,
            element_indicators: self.indicators.clone(),
            element_targets: self.targets.clone(),
            report: self.report.clone(),
        }
    }
}

#[derive(Clone, Copy)]
struct ElementGeometry {
    points: [Point2; 3],
    gradients: [Point2; 3],
    area: f64,
    maximum_edge: f64,
}

fn element_geometry(
    mesh: &TriMesh,
    vertices: [usize; 3],
) -> Result<ElementGeometry, SolutionIndicatorError> {
    if vertices.iter().any(|vertex| *vertex >= mesh.vertices.len()) {
        return Err(SolutionIndicatorError::InvalidMesh);
    }
    let points = vertices.map(|vertex| mesh.vertices[vertex].point);
    let twice_area = (points[1] - points[0]).cross(points[2] - points[0]);
    if !twice_area.is_finite() || twice_area <= 0.0 {
        return Err(SolutionIndicatorError::InvalidMesh);
    }
    let gradients = [
        Point2::new(points[1].y - points[2].y, points[2].x - points[1].x) / twice_area,
        Point2::new(points[2].y - points[0].y, points[0].x - points[2].x) / twice_area,
        Point2::new(points[0].y - points[1].y, points[1].x - points[0].x) / twice_area,
    ];
    let maximum_edge = [
        (points[1] - points[0]).norm(),
        (points[2] - points[1]).norm(),
        (points[0] - points[2]).norm(),
    ]
    .into_iter()
    .fold(0.0, f64::max);
    Ok(ElementGeometry {
        points,
        gradients,
        area: 0.5 * twice_area,
        maximum_edge,
    })
}

fn scalar(values: &[f64], nodes: [usize; 7], basis: [f64; 7]) -> f64 {
    nodes
        .into_iter()
        .zip(basis)
        .map(|(node, weight)| values[node] * weight)
        .sum()
}

fn gradient(values: &[f64], nodes: [usize; 7], basis: [Point2; 7]) -> Point2 {
    nodes
        .into_iter()
        .zip(basis)
        .fold(Point2::default(), |sum, (node, weight)| {
            sum + weight * values[node]
        })
}

fn line_scalar(values: &[f64], nodes: [usize; 3], fraction: f64) -> f64 {
    let basis = [
        (1.0 - fraction) * (1.0 - 2.0 * fraction),
        4.0 * fraction * (1.0 - fraction),
        fraction * (2.0 * fraction - 1.0),
    ];
    nodes
        .into_iter()
        .zip(basis)
        .map(|(node, weight)| values[node] * weight)
        .sum()
}

fn line_second_derivative(values: &[f64], nodes: [usize; 3], length: f64) -> f64 {
    4.0 * (values[nodes[0]] - 2.0 * values[nodes[1]] + values[nodes[2]]) / (length * length)
}

fn line_quadrature() -> [(f64, f64); 3] {
    [
        (0.112_701_665_379_258_3, 5.0 / 18.0),
        (0.5, 8.0 / 18.0),
        (0.887_298_334_620_741_7, 5.0 / 18.0),
    ]
}

fn edge_key([a, b]: [usize; 2]) -> (usize, usize) {
    if a < b { (a, b) } else { (b, a) }
}

fn outward_normal(triangle: [Point2; 3], edge: [Point2; 2]) -> Point2 {
    let tangent = edge[1] - edge[0];
    let mut normal = Point2::new(tangent.y, -tangent.x) / tangent.norm();
    let third = triangle
        .into_iter()
        .find(|point| *point != edge[0] && *point != edge[1])
        .unwrap();
    if normal.dot(third - edge[0]) > 0.0 {
        normal = normal * -1.0;
    }
    normal
}

fn bin_index(point: Point2, minimum: Point2, cell: Point2, dimension: usize) -> [usize; 2] {
    let index = |value: f64, min: f64, width: f64| {
        (((value - min) / width).floor() as isize).clamp(0, dimension as isize - 1) as usize
    };
    [
        index(point.x, minimum.x, cell.x),
        index(point.y, minimum.y, cell.y),
    ]
}

fn barycentric(point: Point2, triangle: [Point2; 3]) -> Option<[f64; 3]> {
    let denominator = (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]);
    if !denominator.is_finite() || denominator <= 0.0 {
        return None;
    }
    let w1 = (point - triangle[0]).cross(triangle[2] - triangle[0]) / denominator;
    let w2 = (triangle[1] - triangle[0]).cross(point - triangle[0]) / denominator;
    let values = [1.0 - w1 - w2, w1, w2];
    values
        .iter()
        .all(|value| *value >= -1.0e-10 && *value <= 1.0 + 1.0e-10)
        .then_some(values)
}

fn quadrature() -> [([f64; 3], f64); 6] {
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
        BACKGROUND_REGION, BoundarySignal, InternalBoundary, InternalBoundaryLaw,
        MeshAdaptationJob, MeshAdaptationOptions, MeshAdaptationState, MeshQuality, MeshTriangle,
        MeshVertex, MeshingOptions, OpenCubicSpline, OuterBoundaryCondition,
        OuterBoundaryConditions, OuterSide, mesh_scene,
    };

    fn square() -> Arc<TriMesh> {
        Arc::new(TriMesh {
            geometry_revision: 4,
            mesh_revision: 7,
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
        })
    }

    fn setup() -> (Arc<TriMesh>, Arc<QuadraticWaveOperator>, Scene) {
        let mesh = square();
        let scene = Scene {
            outer_boundaries: OuterBoundaryConditions::default(),
            ..Scene::default()
        };
        let operator = Arc::new(
            QuadraticWaveOperator::assemble_scene_with_boundaries(
                &mesh,
                &scene,
                scene.outer_boundaries,
            )
            .unwrap(),
        );
        (mesh, operator, scene)
    }

    fn snapshot(
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        value: impl Fn(Point2) -> f64,
    ) -> QuadraticSolutionSnapshot {
        let displacement = operator.node_points().iter().copied().map(value).collect();
        let zeros = vec![0.0; operator.degrees_of_freedom()];
        QuadraticSolutionSnapshot {
            mesh_revision: mesh.mesh_revision,
            displacement,
            velocity: zeros.clone(),
            acceleration: zeros.clone(),
            auxiliary: zeros.clone(),
            volume_acceleration: zeros,
            time: 0.25,
            time_step: 0.01,
        }
    }

    fn meshed_setup(
        scene: Scene,
        revision: u64,
    ) -> (Arc<TriMesh>, Arc<QuadraticWaveOperator>, Scene) {
        let mesh = Arc::new(
            mesh_scene(
                &scene,
                revision,
                MeshingOptions {
                    curve_tolerance: 1.0e-3,
                    target_edge_length: 0.18,
                    minimum_angle_degrees: 12.0,
                    max_vertices: 20_000,
                    max_triangles: 40_000,
                    max_refinement_steps: 20_000,
                },
            )
            .unwrap(),
        );
        let operator = Arc::new(
            QuadraticWaveOperator::assemble_scene_with_boundaries(
                &mesh,
                &scene,
                scene.outer_boundaries,
            )
            .unwrap(),
        );
        (mesh, operator, scene)
    }

    fn constant_signal(value: f64) -> BoundarySignal {
        BoundarySignal {
            offset: value,
            ..BoundarySignal::ZERO
        }
    }

    fn test_boundary_nodes(
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        edge: &crate::BoundaryEdge,
    ) -> [usize; 3] {
        let (triangle_index, triangle) = mesh
            .triangles
            .iter()
            .enumerate()
            .find(|(_, triangle)| {
                edge.vertices
                    .iter()
                    .all(|vertex| triangle.vertices.contains(vertex))
            })
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
        let midpoint = match (a.min(b), a.max(b)) {
            (0, 1) => 3,
            (1, 2) => 4,
            (0, 2) => 5,
            _ => unreachable!(),
        };
        [
            edge.vertices[0],
            operator.element_nodes()[triangle_index][midpoint] as usize,
            edge.vertices[1],
        ]
    }

    fn run(
        mut job: SolutionIndicatorJob,
        budget: usize,
    ) -> Result<SolutionIndicatorResult, SolutionIndicatorError> {
        loop {
            if let Some(result) = job.advance(budget) {
                return result;
            }
        }
    }

    #[test]
    fn affine_solution_has_zero_defect_and_deterministic_slices() {
        let (mesh, operator, scene) = setup();
        let state = snapshot(&mesh, &operator, |point| point.x + 2.0 * point.y);
        let options = SolutionIndicatorOptions {
            maximum_edge_length: 2.0,
            ..Default::default()
        };
        let one = run(
            SolutionIndicatorJob::new(
                mesh.clone(),
                operator.clone(),
                scene.clone(),
                state.clone(),
                options,
            ),
            1,
        )
        .unwrap();
        let many = run(
            SolutionIndicatorJob::new(mesh.clone(), operator, scene, state, options),
            10_000,
        )
        .unwrap();
        assert_eq!(one.element_targets, many.element_targets);
        assert!(one.element_indicators.iter().all(|value| *value < 1.0e-10));
        assert!(one.element_targets.iter().all(|value| value.is_finite()));
        assert!(
            one.field
                .target_edge_length(Point2::new(0.25, 0.25), BACKGROUND_REGION)
                .is_finite()
        );
    }

    #[test]
    fn manufactured_outer_laws_have_zero_boundary_residual_and_detect_mismatch() {
        let mut scene = Scene::default();
        scene.outer_boundaries.sides[OuterSide::Bottom.index()] = OuterBoundaryCondition::Neumann {
            signal: constant_signal(-2.0),
        };
        scene.outer_boundaries.sides[OuterSide::Right.index()] =
            OuterBoundaryCondition::FirstOrderOutgoing;
        scene.outer_boundaries.sides[OuterSide::Top.index()] =
            OuterBoundaryCondition::SecondOrderOutgoing;
        scene.outer_boundaries.sides[OuterSide::Left.index()] = OuterBoundaryCondition::Neumann {
            signal: constant_signal(-1.0),
        };
        let (mesh, operator, scene) = meshed_setup(scene, 71);
        let mut state = snapshot(&mesh, &operator, |point| point.x + 2.0 * point.y);
        state.velocity.fill(-1.0);
        state.auxiliary = operator
            .node_points()
            .iter()
            .map(|point| point.x * point.x)
            .collect();
        let options = SolutionIndicatorOptions::default();
        let one = run(
            SolutionIndicatorJob::new(
                mesh.clone(),
                operator.clone(),
                scene.clone(),
                state.clone(),
                options,
            ),
            1,
        )
        .unwrap();
        let many = run(
            SolutionIndicatorJob::new(
                mesh.clone(),
                operator.clone(),
                scene.clone(),
                state.clone(),
                options,
            ),
            10_000,
        )
        .unwrap();
        assert_eq!(one.element_indicators, many.element_indicators);
        assert_eq!(one.report, many.report);
        assert_eq!(
            one.report.boundary_edges_evaluated,
            mesh.boundary_edges.len()
        );
        assert!(
            one.report.boundary_residual_contribution < 1.0e-20,
            "{:?}",
            one.report
        );

        state.velocity.fill(0.0);
        let mismatch = run(
            SolutionIndicatorJob::new(mesh, operator, scene, state, options),
            10_000,
        )
        .unwrap();
        assert!(mismatch.report.boundary_residual_contribution > 1.0e-3);
    }

    #[test]
    fn dirichlet_mismatch_is_diagnostic_only() {
        let scene = Scene {
            outer_boundaries: OuterBoundaryConditions::uniform(OuterBoundaryCondition::Dirichlet {
                signal: BoundarySignal::ZERO,
            }),
            ..Scene::default()
        };
        let (mesh, operator, scene) = meshed_setup(scene, 72);
        let exact = run(
            SolutionIndicatorJob::new(
                mesh.clone(),
                operator.clone(),
                scene.clone(),
                snapshot(&mesh, &operator, |_| 0.0),
                SolutionIndicatorOptions::default(),
            ),
            10_000,
        )
        .unwrap();
        assert_eq!(exact.report.maximum_dirichlet_mismatch, 0.0);
        assert_eq!(exact.report.boundary_residual_contribution, 0.0);

        let mismatch = run(
            SolutionIndicatorJob::new(
                mesh.clone(),
                operator.clone(),
                scene,
                snapshot(&mesh, &operator, |_| 0.25),
                SolutionIndicatorOptions::default(),
            ),
            10_000,
        )
        .unwrap();
        assert!((mismatch.report.maximum_dirichlet_mismatch - 0.25).abs() < 1.0e-12);
        assert_eq!(mismatch.report.boundary_residual_contribution, 0.0);
    }

    #[test]
    fn hole_impedance_contributes_to_its_adjacent_elements() {
        let mut scene = Scene::initial();
        scene.outer_boundaries =
            OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting);
        scene.obstacles[0]
            .span_conditions
            .fill(FaceBoundaryCondition::Impedance { ratio: 1.0 });
        let (mesh, operator, scene) = meshed_setup(scene, 73);
        let mut state = snapshot(&mesh, &operator, |_| 0.0);
        state.velocity.fill(1.0);
        let result = run(
            SolutionIndicatorJob::new(
                mesh.clone(),
                operator,
                scene,
                state,
                SolutionIndicatorOptions::default(),
            ),
            10_000,
        )
        .unwrap();
        assert!(result.report.boundary_residual_contribution > 0.0);
        assert_eq!(
            result.report.boundary_edges_evaluated,
            mesh.boundary_edges.len()
        );
    }

    #[test]
    fn thin_gap_faces_add_without_cancelling_and_require_a_pair() {
        let mut scene = Scene {
            outer_boundaries: OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting),
            ..Scene::default()
        };
        scene.internal_boundaries.push(InternalBoundary {
            id: InternalBoundaryId(4),
            spline: OpenCubicSpline::uniform(vec![
                Point2::new(-0.75, 0.0),
                Point2::new(-0.25, 0.0),
                Point2::new(0.25, 0.0),
                Point2::new(0.75, 0.0),
            ])
            .unwrap(),
            region: BACKGROUND_REGION,
            span_laws: vec![InternalBoundaryLaw {
                left: FaceBoundaryCondition::Reflecting,
                right: FaceBoundaryCondition::Reflecting,
                coupling: InternalBoundaryCoupling::ThinGap {
                    stiffness_ratio: 3.0,
                },
            }],
        });
        let (mesh, gap_operator, gap_scene) = meshed_setup(scene, 74);
        let mut state = snapshot(&mesh, &gap_operator, |_| 0.0);
        for (node, point) in gap_operator.node_points().iter().enumerate() {
            state.displacement[node] = if point.y > 1.0e-10 {
                1.0
            } else if point.y < -1.0e-10 {
                -1.0
            } else {
                0.0
            };
        }
        for edge in &mesh.boundary_edges {
            let BoundaryLabel::InternalBoundary { side, .. } = edge.label else {
                continue;
            };
            let value = match side {
                InternalBoundarySide::Left => 1.0,
                InternalBoundarySide::Right => -1.0,
            };
            for node in test_boundary_nodes(&mesh, &gap_operator, edge) {
                state.displacement[node] = value;
            }
        }
        let malformed_state = state.clone();
        let options = SolutionIndicatorOptions::default();
        let gap = run(
            SolutionIndicatorJob::new(
                mesh.clone(),
                gap_operator.clone(),
                gap_scene.clone(),
                state.clone(),
                options,
            ),
            10_000,
        )
        .unwrap();

        let mut independent_scene = gap_scene.clone();
        independent_scene.internal_boundaries[0].span_laws[0].coupling =
            InternalBoundaryCoupling::Independent;
        let independent_operator = Arc::new(
            QuadraticWaveOperator::assemble_scene_with_boundaries(
                &mesh,
                &independent_scene,
                independent_scene.outer_boundaries,
            )
            .unwrap(),
        );
        assert_eq!(
            independent_operator.node_points(),
            gap_operator.node_points()
        );
        let independent = run(
            SolutionIndicatorJob::new(
                mesh.clone(),
                independent_operator,
                independent_scene,
                state,
                options,
            ),
            10_000,
        )
        .unwrap();
        assert!(
            gap.report.boundary_residual_contribution
                > independent.report.boundary_residual_contribution
        );

        let mut malformed = mesh.as_ref().clone();
        let right = malformed
            .boundary_edges
            .iter()
            .position(|edge| {
                matches!(
                    edge.label,
                    BoundaryLabel::InternalBoundary {
                        side: InternalBoundarySide::Right,
                        ..
                    }
                )
            })
            .unwrap();
        malformed.boundary_edges.remove(right);
        let malformed = Arc::new(malformed);
        let result = run(
            SolutionIndicatorJob::new(
                malformed.clone(),
                gap_operator,
                gap_scene,
                malformed_state,
                options,
            ),
            10_000,
        );
        assert_eq!(result.unwrap_err(), SolutionIndicatorError::InvalidMesh);
    }

    #[test]
    fn relative_indicator_is_invariant_to_field_amplitude() {
        let (mesh, operator, scene) = setup();
        let options = SolutionIndicatorOptions {
            maximum_edge_length: 2.0,
            ..Default::default()
        };
        let first = run(
            SolutionIndicatorJob::new(
                mesh.clone(),
                operator.clone(),
                scene.clone(),
                snapshot(&mesh, &operator, |point| point.x * point.x),
                options,
            ),
            100,
        )
        .unwrap();
        let scaled = run(
            SolutionIndicatorJob::new(
                mesh.clone(),
                operator.clone(),
                scene,
                snapshot(&mesh, &operator, |point| 7.0 * point.x * point.x),
                options,
            ),
            100,
        )
        .unwrap();
        for (a, b) in first
            .element_indicators
            .iter()
            .zip(&scaled.element_indicators)
        {
            assert!((a - b).abs() < 1.0e-10, "{a} != {b}");
        }
    }

    #[test]
    fn matched_volume_acceleration_removes_the_cell_residual() {
        let (mesh, operator, scene) = setup();
        let mut state = snapshot(&mesh, &operator, |_| 0.0);
        state.acceleration.fill(3.0);
        state.volume_acceleration.fill(3.0);
        let result = run(
            SolutionIndicatorJob::new(
                mesh,
                operator,
                scene,
                state,
                SolutionIndicatorOptions::default(),
            ),
            100,
        )
        .unwrap();
        assert_eq!(result.report.maximum_indicator, 0.0);
    }

    #[test]
    fn repeated_quiet_estimates_coarsen_a_fine_mesh_without_refinement() {
        let scene = Scene::default();
        let meshing = MeshingOptions {
            curve_tolerance: 1.0e-3,
            target_edge_length: 0.12,
            minimum_angle_degrees: 12.0,
            max_vertices: 20_000,
            max_triangles: 40_000,
            max_refinement_steps: 20_000,
        };
        let mut mesh = Arc::new(mesh_scene(&scene, 10, meshing).unwrap());
        let mut adaptation_state = MeshAdaptationState::from_mesh(&mesh);
        let initial_triangles = mesh.triangles.len();
        let mut coarsening_changes = 0;

        for pass in 0..2 {
            let operator = Arc::new(
                QuadraticWaveOperator::assemble_scene_with_boundaries(
                    &mesh,
                    &scene,
                    scene.outer_boundaries,
                )
                .unwrap(),
            );
            let estimate = run(
                SolutionIndicatorJob::new(
                    mesh.clone(),
                    operator.clone(),
                    scene.clone(),
                    snapshot(&mesh, &operator, |_| 0.0),
                    SolutionIndicatorOptions {
                        minimum_edge_length: 0.06,
                        maximum_edge_length: 0.30,
                        maximum_scale: 2.2,
                        coarsen_ratio: 0.65,
                        ..Default::default()
                    },
                ),
                1_000,
            )
            .unwrap();
            assert_eq!(estimate.report.refine_candidates, 0);

            let mut job = MeshAdaptationJob::new(
                mesh.clone(),
                scene.clone(),
                adaptation_state,
                11 + pass,
                estimate.field,
                MeshAdaptationOptions {
                    meshing,
                    minimum_target_edge_length: 0.06,
                    maximum_target_edge_length: 0.30,
                    collapse_ratio: 0.65,
                    max_topology_changes: 160,
                    max_coarsening_changes: 80,
                    max_work_units: 20_000_000,
                    ..Default::default()
                },
            );
            let adapted = loop {
                if let Some(result) = job.advance(1_000) {
                    break result.unwrap();
                }
            };
            assert_eq!(adapted.report.inserted_vertices, 0);
            coarsening_changes += adapted.report.coarsening_changes;
            mesh = Arc::new(adapted.mesh);
            adaptation_state = adapted.state;
        }

        assert!(coarsening_changes > 0);
        assert!(mesh.triangles.len() < initial_triangles);
    }

    #[test]
    fn active_frequency_caps_targets_by_local_wavelength() {
        let (mesh, operator, scene) = setup();
        let state = snapshot(&mesh, &operator, |point| point.x);
        let result = run(
            SolutionIndicatorJob::new(
                mesh,
                operator,
                scene,
                state,
                SolutionIndicatorOptions {
                    minimum_edge_length: 0.005,
                    maximum_edge_length: 2.0,
                    forcing_frequency_hz: 10.0,
                    elements_per_wavelength: 5.0,
                    ..Default::default()
                },
            ),
            100,
        )
        .unwrap();
        assert!(
            result
                .element_targets
                .iter()
                .all(|target| *target <= 0.020_000_001)
        );
    }

    #[test]
    fn spatial_interpolation_cannot_escape_configured_size_limits() {
        let mesh = square();
        let field = AdaptiveSizeField::new(
            mesh,
            vec![[0.02, 0.02, 0.16], [0.16, 0.16, 0.16]],
            0.02,
            0.16,
        );
        // The point is within the deliberate barycentric edge tolerance. Its
        // tiny negative weight would extrapolate below 0.02 without the final
        // clamp and be rejected by MeshAdaptationJob.
        assert_eq!(
            field.target_edge_length(Point2::new(0.5, -5.0e-11), BACKGROUND_REGION),
            0.02
        );
        assert_eq!(
            field.target_edge_length(Point2::new(2.0, 2.0), BACKGROUND_REGION),
            0.16
        );
    }

    #[test]
    fn rejects_stale_and_malformed_snapshots() {
        let (mesh, operator, scene) = setup();
        let mut state = snapshot(&mesh, &operator, |_| 0.0);
        state.mesh_revision += 1;
        assert_eq!(
            run(
                SolutionIndicatorJob::new(
                    mesh.clone(),
                    operator.clone(),
                    scene.clone(),
                    state,
                    SolutionIndicatorOptions::default(),
                ),
                100,
            )
            .unwrap_err(),
            SolutionIndicatorError::InvalidSnapshot
        );
        let mut state = snapshot(&mesh, &operator, |_| 0.0);
        state.velocity.pop();
        assert_eq!(
            run(
                SolutionIndicatorJob::new(
                    mesh.clone(),
                    operator.clone(),
                    scene.clone(),
                    state,
                    SolutionIndicatorOptions::default(),
                ),
                100,
            )
            .unwrap_err(),
            SolutionIndicatorError::InvalidSnapshot
        );
        let mut state = snapshot(&mesh, &operator, |_| 0.0);
        state.auxiliary.pop();
        assert_eq!(
            run(
                SolutionIndicatorJob::new(
                    mesh.clone(),
                    operator.clone(),
                    scene.clone(),
                    state,
                    SolutionIndicatorOptions::default(),
                ),
                100,
            )
            .unwrap_err(),
            SolutionIndicatorError::InvalidSnapshot
        );
        let mut state = snapshot(&mesh, &operator, |_| 0.0);
        state.auxiliary[0] = f64::NAN;
        assert_eq!(
            run(
                SolutionIndicatorJob::new(
                    mesh,
                    operator,
                    scene,
                    state,
                    SolutionIndicatorOptions::default(),
                ),
                100,
            )
            .unwrap_err(),
            SolutionIndicatorError::InvalidSnapshot
        );
    }
}
