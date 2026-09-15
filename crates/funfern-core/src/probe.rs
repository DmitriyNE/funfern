use std::collections::BTreeSet;

use crate::{
    BoundaryLabel, EvaluatedMaterial, PlannedBoundarySource, Point2, PointSource,
    QuadraticWaveOperator, RegionId, Scene, TopologyMeshPlan, TopologyWaveModel, TriMesh,
    VolumeSource, WaveCoefficients, enriched_quadratic_basis, enriched_quadratic_basis_gradients,
    point_segment_distance,
};

const BOUNDARY_TOLERANCE: f64 = 1.0e-9;
const MIN_DISK_POLYGON_SIDES: usize = 24;
const MAX_DISK_POLYGON_SIDES: usize = 128;
const DISK_APPROXIMATION_TOLERANCE: f64 = 2.0e-4;
pub const MAX_FAR_FIELD_CONTOUR_POINTS: usize = 4096;

#[derive(Clone, Copy)]
enum ProbeModel<'a> {
    Scene(&'a Scene),
    Topology {
        plan: &'a TopologyMeshPlan,
        model: TopologyWaveModel<'a>,
    },
}

impl ProbeModel<'_> {
    fn valid(self) -> bool {
        match self {
            Self::Scene(scene) => scene.structure_valid(),
            Self::Topology { plan, model } => model.valid_for(plan),
        }
    }

    fn contains_region(self, region: RegionId) -> bool {
        match self {
            Self::Scene(scene) => scene.region(region).is_some(),
            Self::Topology { plan, .. } => {
                plan.domains.iter().any(|domain| domain.region == region)
            }
        }
    }

    fn directional_material_at(
        self,
        region: RegionId,
        point: Point2,
    ) -> Result<crate::DirectionalWaveCoefficients, crate::MaterialError> {
        match self {
            Self::Scene(scene) => scene.directional_material_at(region, point),
            Self::Topology { model, .. } => model.directional_material_at(region, point),
        }
    }

    fn contains_boundary_target(
        self,
        label: BoundaryLabel,
        parameter: f64,
        period: f64,
        region: RegionId,
    ) -> bool {
        let Self::Topology { plan, .. } = self else {
            return true;
        };
        (-1..=1).any(|shift| {
            let parameter = parameter + shift as f64 * period;
            plan.boundaries.iter().any(|boundary| {
                let [a, b] = boundary.parameter;
                let contains = parameter >= a.min(b) - BOUNDARY_TOLERANCE
                    && parameter <= a.max(b) + BOUNDARY_TOLERANCE;
                if !contains || boundary.region != region {
                    return false;
                }
                match (label, boundary.source, boundary.behavior) {
                    (
                        BoundaryLabel::Outer(label_side),
                        crate::PlannedBoundarySource::Outer(side),
                        _,
                    ) => label_side == side,
                    (
                        BoundaryLabel::Curve {
                            curve,
                            span,
                            side,
                            separated: true,
                        },
                        crate::PlannedBoundarySource::Curve {
                            curve: candidate_curve,
                            span: candidate_span,
                            side: candidate_side,
                        },
                        Some(crate::SpanBehavior::Separated { .. }),
                    ) => {
                        curve == candidate_curve && span == candidate_span && side == candidate_side
                    }
                    (
                        BoundaryLabel::Curve {
                            curve,
                            span,
                            separated: false,
                            ..
                        },
                        crate::PlannedBoundarySource::Curve {
                            curve: candidate_curve,
                            span: candidate_span,
                            ..
                        },
                        Some(crate::SpanBehavior::Transmitting),
                    ) => curve == candidate_curve && span == candidate_span,
                    _ => false,
                }
            })
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuadraticPointStencil {
    pub nodes: [u32; 7],
    pub value_weights: [f64; 7],
    pub gradient_weights: [Point2; 7],
    pub region: RegionId,
    pub mass_density: f64,
    pub stiffness: crate::SymmetricTensor2,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuadraticBoundaryStencil {
    pub stencil: QuadraticPointStencil,
    pub point: Point2,
    /// Unit normal pointing away from the sampled trace's adjacent element.
    pub outward_normal: Point2,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoundaryStencilTarget {
    pub label: BoundaryLabel,
    pub parameter: f64,
    pub period: f64,
    pub region: RegionId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QuadraticFarFieldStencil {
    pub samples: Vec<(QuadraticPointStencil, Point2, Point2)>,
    pub exterior_region: RegionId,
    pub wave_speed: f64,
    pub sample_spacing: f64,
    pub delay_margin: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FarFieldCompileOptions<'a> {
    pub inset: f64,
    pub sample_count: usize,
    pub point_source: Option<PointSource>,
    pub volume_sources: &'a [VolumeSource],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FarFieldCompileError {
    InvalidInputs,
    InvalidInset,
    InvalidSampleCount,
    GeometryOutsideContour,
    MultipleExteriorFaces,
    NonUniformExterior,
    LossyExterior,
    AnisotropicExterior,
    DrivenExterior,
    InvalidWaveSpeed,
    ContourUnavailable(PointProbeError),
    ContourLeavesExterior,
}

impl std::fmt::Display for FarFieldCompileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInputs => {
                formatter.write_str("far-field mesh, topology, or material model is invalid")
            }
            Self::InvalidInset => formatter
                .write_str("far-field inset must leave a nonempty contour inside the domain"),
            Self::InvalidSampleCount => {
                formatter.write_str("far-field contour sample count is outside its bounds")
            }
            Self::GeometryOutsideContour => formatter
                .write_str("decrease the inset: the far-field contour must enclose all geometry"),
            Self::MultipleExteriorFaces => formatter
                .write_str("far-field projection requires one exterior face outside the contour"),
            Self::NonUniformExterior => {
                formatter.write_str("far-field projection requires a uniform exterior medium")
            }
            Self::LossyExterior => {
                formatter.write_str("far-field projection requires a lossless exterior medium")
            }
            Self::AnisotropicExterior => {
                formatter.write_str("far-field projection requires an isotropic exterior medium")
            }
            Self::DrivenExterior => {
                formatter.write_str("far-field projection requires a source-free exterior medium")
            }
            Self::InvalidWaveSpeed => formatter.write_str("the exterior wave speed is invalid"),
            Self::ContourUnavailable(error) => {
                write!(
                    formatter,
                    "the far-field contour is unavailable on the mesh: {error}"
                )
            }
            Self::ContourLeavesExterior => {
                formatter.write_str("the far-field contour leaves its uniform exterior face")
            }
        }
    }
}

impl std::error::Error for FarFieldCompileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ContourUnavailable(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointProbeSample {
    pub displacement: f64,
    pub velocity: f64,
    pub gradient: Point2,
    pub energy_density: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AreaProbeShape {
    Disk { center: Point2, radius: f64 },
    Region(RegionId),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuadraticAreaElement {
    pub nodes: [u32; 7],
    /// Vertices of the clipped integration triangle in parent-element coordinates.
    pub barycentric_vertices: [[f64; 3]; 3],
    pub barycentric_gradients: [Point2; 3],
    pub region: RegionId,
    pub coefficients: [crate::DirectionalWaveCoefficients; 12],
    pub area: f64,
}

pub const QUADRATIC_AREA_MATRIX_ENTRIES: usize = 28;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuadraticAreaMatrices {
    pub field: [f64; 7],
    /// Symmetric upper triangle in row-major `(0,0), (0,1), ... (6,6)` order.
    pub mass: [f64; QUADRATIC_AREA_MATRIX_ENTRIES],
    /// Density-weighted symmetric mass matrix used for kinetic energy.
    pub density: [f64; QUADRATIC_AREA_MATRIX_ENTRIES],
    /// Symmetric upper triangle in the same order as `mass`.
    pub stiffness: [f64; QUADRATIC_AREA_MATRIX_ENTRIES],
    /// Gradient matrix weighted by the square of the local stiffness coefficient.
    pub stiffness_squared: [f64; QUADRATIC_AREA_MATRIX_ENTRIES],
}

impl QuadraticAreaElement {
    pub fn integrated_matrices(self) -> QuadraticAreaMatrices {
        let mut result = QuadraticAreaMatrices {
            field: [0.0; 7],
            mass: [0.0; QUADRATIC_AREA_MATRIX_ENTRIES],
            density: [0.0; QUADRATIC_AREA_MATRIX_ENTRIES],
            stiffness: [0.0; QUADRATIC_AREA_MATRIX_ENTRIES],
            stiffness_squared: [0.0; QUADRATIC_AREA_MATRIX_ENTRIES],
        };
        for ((local, weight), coefficients) in area_quadrature().into_iter().zip(self.coefficients)
        {
            let barycentric = std::array::from_fn(|coordinate| {
                self.barycentric_vertices[0][coordinate] * local[0]
                    + self.barycentric_vertices[1][coordinate] * local[1]
                    + self.barycentric_vertices[2][coordinate] * local[2]
            });
            let values = enriched_quadratic_basis(barycentric);
            let gradients =
                enriched_quadratic_basis_gradients(barycentric, self.barycentric_gradients);
            let physical_weight = self.area * weight;
            for (field, value) in result.field.iter_mut().zip(values) {
                *field += physical_weight * value;
            }
            let mut entry = 0;
            for (row, row_value) in values.iter().copied().enumerate() {
                for (column, column_value) in values.iter().copied().enumerate().skip(row) {
                    result.mass[entry] += physical_weight * row_value * column_value;
                    result.stiffness[entry] += physical_weight
                        * gradients[row].dot(coefficients.stiffness.apply(gradients[column]));
                    result.stiffness_squared[entry] += physical_weight
                        * coefficients
                            .stiffness
                            .apply(gradients[row])
                            .dot(coefficients.stiffness.apply(gradients[column]));
                    result.density[entry] +=
                        physical_weight * coefficients.mass_density * row_value * column_value;
                    entry += 1;
                }
            }
        }
        result
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct QuadraticAreaStencil {
    pub shape: AreaProbeShape,
    pub elements: Vec<QuadraticAreaElement>,
    pub covered_area: f64,
    pub target_area: f64,
    pub degrees_of_freedom: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaProbeSample {
    pub mean_displacement: f64,
    pub rms_displacement: f64,
    pub mean_energy_density: f64,
    pub total_energy: f64,
    pub covered_area: f64,
    pub coverage: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AreaProbeError {
    InvalidTarget,
    MissingRegion,
    EmptyCoverage,
    InvalidMesh,
    SizeMismatch,
    NonFiniteValues,
}

impl std::fmt::Display for AreaProbeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidTarget => "area probe target is invalid",
            Self::MissingRegion => "area probe references a missing region",
            Self::EmptyCoverage => "area probe does not cover the simulated domain",
            Self::InvalidMesh => "area probe cannot use the current mesh",
            Self::SizeMismatch => "area probe field does not match the wave operator",
            Self::NonFiniteValues => "area probe field contains non-finite values",
        })
    }
}

impl std::error::Error for AreaProbeError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointProbeError {
    OutsideDomain,
    AmbiguousBoundary,
    InvalidMesh,
    SizeMismatch,
    NonFiniteValues,
}

impl std::fmt::Display for PointProbeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::OutsideDomain => "probe is outside the simulated domain",
            Self::AmbiguousBoundary => "probe is on a boundary with two field traces",
            Self::InvalidMesh => "probe cannot use the current mesh",
            Self::SizeMismatch => "probe field does not match the wave operator",
            Self::NonFiniteValues => "probe field contains non-finite values",
        })
    }
}

impl std::error::Error for PointProbeError {}

impl QuadraticPointStencil {
    pub fn build(
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        scene: &Scene,
        point: Point2,
    ) -> Result<Self, PointProbeError> {
        Self::build_with_model(mesh, operator, ProbeModel::Scene(scene), point)
    }

    /// Builds a point stencil from explicit topology face assignments and the
    /// shared material library. Points on a curve constraint are rejected because
    /// their material/trace side is ambiguous without a boundary target.
    pub fn build_topology(
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        plan: &TopologyMeshPlan,
        model: TopologyWaveModel<'_>,
        point: Point2,
    ) -> Result<Self, PointProbeError> {
        Self::build_with_model(mesh, operator, ProbeModel::Topology { plan, model }, point)
    }

    fn build_with_model(
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        model: ProbeModel<'_>,
        point: Point2,
    ) -> Result<Self, PointProbeError> {
        if !point.finite()
            || !model.valid()
            || mesh.geometry_revision != operator.geometry_revision()
            || mesh.mesh_revision != operator.mesh_revision()
            || mesh.triangles.len() != operator.element_nodes().len()
        {
            return Err(PointProbeError::InvalidMesh);
        }
        for edge in &mesh.boundary_edges {
            if !matches!(
                edge.label,
                BoundaryLabel::MaterialInterface(_)
                    | BoundaryLabel::Wall { .. }
                    | BoundaryLabel::InternalBoundary { .. }
                    | BoundaryLabel::Curve { .. }
            ) {
                continue;
            }
            let [a, b] = edge.vertices;
            let (Some(a), Some(b)) = (mesh.vertices.get(a), mesh.vertices.get(b)) else {
                return Err(PointProbeError::InvalidMesh);
            };
            if point_segment_distance(point, a.point, b.point) <= BOUNDARY_TOLERANCE {
                return Err(PointProbeError::AmbiguousBoundary);
            }
        }

        let mut located = None;
        for (triangle_index, triangle) in mesh.triangles.iter().enumerate() {
            let [a, b, c] = triangle.vertices;
            let (Some(a), Some(b), Some(c)) = (
                mesh.vertices.get(a),
                mesh.vertices.get(b),
                mesh.vertices.get(c),
            ) else {
                return Err(PointProbeError::InvalidMesh);
            };
            let twice_area = (b.point - a.point).cross(c.point - a.point);
            if !twice_area.is_finite() || twice_area <= 0.0 {
                return Err(PointProbeError::InvalidMesh);
            }
            let barycentric = [
                (b.point - point).cross(c.point - point) / twice_area,
                (c.point - point).cross(a.point - point) / twice_area,
                (a.point - point).cross(b.point - point) / twice_area,
            ];
            if barycentric.iter().all(|weight| *weight >= -1.0e-10) {
                let gradients = [
                    Point2::new(b.point.y - c.point.y, c.point.x - b.point.x) / twice_area,
                    Point2::new(c.point.y - a.point.y, a.point.x - c.point.x) / twice_area,
                    Point2::new(a.point.y - b.point.y, b.point.x - a.point.x) / twice_area,
                ];
                located = Some((triangle_index, triangle.region, barycentric, gradients));
                break;
            }
        }
        let Some((triangle_index, region, barycentric, gradients)) = located else {
            return Err(PointProbeError::OutsideDomain);
        };
        Self::from_element(
            mesh,
            operator,
            model,
            triangle_index,
            region,
            barycentric,
            gradients,
        )
    }

    fn from_element(
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        model: ProbeModel<'_>,
        triangle_index: usize,
        region: RegionId,
        barycentric: [f64; 3],
        gradients: [Point2; 3],
    ) -> Result<Self, PointProbeError> {
        let point = mesh.triangles[triangle_index]
            .vertices
            .map(|index| mesh.vertices[index].point);
        let point =
            point[0] * barycentric[0] + point[1] * barycentric[1] + point[2] * barycentric[2];
        if !model.contains_region(region) {
            return Err(PointProbeError::InvalidMesh);
        }
        let material = model
            .directional_material_at(region, point)
            .map_err(|_| PointProbeError::InvalidMesh)?;
        let nodes = *operator
            .element_nodes()
            .get(triangle_index)
            .ok_or(PointProbeError::InvalidMesh)?;
        Ok(Self {
            nodes,
            value_weights: enriched_quadratic_basis(barycentric),
            gradient_weights: enriched_quadratic_basis_gradients(barycentric, gradients),
            region,
            mass_density: material.mass_density,
            stiffness: material.stiffness,
        })
    }

    pub fn sample(
        &self,
        displacement: &[f64],
        velocity: &[f64],
    ) -> Result<PointProbeSample, PointProbeError> {
        let required = self.nodes.iter().copied().max().unwrap_or(0) as usize + 1;
        if displacement.len() < required || velocity.len() < required {
            return Err(PointProbeError::SizeMismatch);
        }
        let mut field = 0.0;
        let mut speed = 0.0;
        let mut gradient = Point2::default();
        for local in 0..7 {
            let node = self.nodes[local] as usize;
            let u = displacement[node];
            let v = velocity[node];
            if !u.is_finite() || !v.is_finite() {
                return Err(PointProbeError::NonFiniteValues);
            }
            field += self.value_weights[local] * u;
            speed += self.value_weights[local] * v;
            gradient = gradient + self.gradient_weights[local] * u;
        }
        let energy_density =
            0.5 * (self.mass_density * speed * speed + self.stiffness.quadratic_form(gradient));
        if !field.is_finite() || !speed.is_finite() || !energy_density.is_finite() {
            return Err(PointProbeError::NonFiniteValues);
        }
        Ok(PointProbeSample {
            displacement: field,
            velocity: speed,
            gradient,
            energy_density,
        })
    }
}

impl QuadraticAreaStencil {
    pub fn build(
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        scene: &Scene,
        shape: AreaProbeShape,
    ) -> Result<Self, AreaProbeError> {
        Self::build_with_model(mesh, operator, ProbeModel::Scene(scene), shape)
    }

    /// Builds a disk or active-face-region integral from a compiled topology
    /// plan. Region targets must name an active face assignment, even if the
    /// material library still contains an unused region with the same ID.
    pub fn build_topology(
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        plan: &TopologyMeshPlan,
        model: TopologyWaveModel<'_>,
        shape: AreaProbeShape,
    ) -> Result<Self, AreaProbeError> {
        Self::build_with_model(mesh, operator, ProbeModel::Topology { plan, model }, shape)
    }

    fn build_with_model(
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        model: ProbeModel<'_>,
        shape: AreaProbeShape,
    ) -> Result<Self, AreaProbeError> {
        if mesh.geometry_revision != operator.geometry_revision()
            || mesh.mesh_revision != operator.mesh_revision()
            || mesh.triangles.len() != operator.element_nodes().len()
            || !model.valid()
        {
            return Err(AreaProbeError::InvalidMesh);
        }
        let clip = match shape {
            AreaProbeShape::Disk { center, radius } => {
                if !center.finite()
                    || !radius.is_finite()
                    || radius <= 0.0
                    || !(std::f64::consts::PI * radius * radius).is_finite()
                {
                    return Err(AreaProbeError::InvalidTarget);
                }
                Some(disk_polygon(center, radius))
            }
            AreaProbeShape::Region(region) => {
                if !model.contains_region(region) {
                    return Err(AreaProbeError::MissingRegion);
                }
                None
            }
        };
        let mut elements = Vec::new();
        let mut covered_area = 0.0;
        for (triangle_index, triangle) in mesh.triangles.iter().enumerate() {
            if matches!(shape, AreaProbeShape::Region(region) if triangle.region != region) {
                continue;
            }
            let points = triangle
                .vertices
                .map(|index| mesh.vertices.get(index).map(|vertex| vertex.point));
            let [Some(a), Some(b), Some(c)] = points else {
                return Err(AreaProbeError::InvalidMesh);
            };
            let twice_area = (b - a).cross(c - a);
            if !twice_area.is_finite() || twice_area <= 0.0 {
                return Err(AreaProbeError::InvalidMesh);
            }
            let gradients = [
                Point2::new(b.y - c.y, c.x - b.x) / twice_area,
                Point2::new(c.y - a.y, a.x - c.x) / twice_area,
                Point2::new(a.y - b.y, b.x - a.x) / twice_area,
            ];
            let mut polygon = vec![a, b, c];
            if let Some(clip) = &clip {
                let triangle_min = Point2::new(a.x.min(b.x).min(c.x), a.y.min(b.y).min(c.y));
                let triangle_max = Point2::new(a.x.max(b.x).max(c.x), a.y.max(b.y).max(c.y));
                let disk_min = clip[0];
                let disk_max = clip[1];
                if triangle_max.x < disk_min.x
                    || triangle_min.x > disk_max.x
                    || triangle_max.y < disk_min.y
                    || triangle_min.y > disk_max.y
                {
                    continue;
                }
                polygon = clip_convex_polygon(&polygon, &clip[2..]);
            }
            if polygon.len() < 3 {
                continue;
            }
            let nodes = *operator
                .element_nodes()
                .get(triangle_index)
                .ok_or(AreaProbeError::InvalidMesh)?;
            for index in 1..polygon.len() - 1 {
                let subtriangle = [polygon[0], polygon[index], polygon[index + 1]];
                let area =
                    0.5 * (subtriangle[1] - subtriangle[0]).cross(subtriangle[2] - subtriangle[0]);
                if !area.is_finite() || area <= f64::EPSILON {
                    continue;
                }
                covered_area += area;
                let barycentric_vertices = subtriangle.map(|point| {
                    [
                        (b - point).cross(c - point) / twice_area,
                        (c - point).cross(a - point) / twice_area,
                        (a - point).cross(b - point) / twice_area,
                    ]
                });
                let mut coefficients = [crate::DirectionalWaveCoefficients {
                    mass_density: 1.0,
                    stiffness: crate::SymmetricTensor2::isotropic(1.0),
                    damping: 0.0,
                }; 12];
                for (slot, (local, _)) in area_quadrature().into_iter().enumerate() {
                    let point = subtriangle[0] * local[0]
                        + subtriangle[1] * local[1]
                        + subtriangle[2] * local[2];
                    coefficients[slot] = model
                        .directional_material_at(triangle.region, point)
                        .map_err(|_| AreaProbeError::InvalidMesh)?;
                }
                elements.push(QuadraticAreaElement {
                    nodes,
                    barycentric_vertices,
                    barycentric_gradients: gradients,
                    region: triangle.region,
                    coefficients,
                    area,
                });
            }
        }
        if !covered_area.is_finite() || covered_area <= f64::EPSILON {
            return Err(AreaProbeError::EmptyCoverage);
        }
        let target_area = match shape {
            AreaProbeShape::Disk { radius, .. } => std::f64::consts::PI * radius * radius,
            AreaProbeShape::Region(_) => covered_area,
        };
        Ok(Self {
            shape,
            elements,
            covered_area,
            target_area,
            degrees_of_freedom: operator.degrees_of_freedom(),
        })
    }

    pub fn sample(
        &self,
        displacement: &[f64],
        velocity: &[f64],
    ) -> Result<AreaProbeSample, AreaProbeError> {
        if displacement.len() != self.degrees_of_freedom
            || velocity.len() != self.degrees_of_freedom
        {
            return Err(AreaProbeError::SizeMismatch);
        }
        if displacement
            .iter()
            .chain(velocity)
            .any(|value| !value.is_finite())
        {
            return Err(AreaProbeError::NonFiniteValues);
        }
        let mut displacement_integral = 0.0;
        let mut displacement_squared_integral = 0.0;
        let mut total_energy = 0.0;
        for element in &self.elements {
            let local_displacement = element.nodes.map(|node| displacement[node as usize]);
            let local_velocity = element.nodes.map(|node| velocity[node as usize]);
            let matrices = element.integrated_matrices();
            for (weight, value) in matrices.field.iter().zip(local_displacement) {
                displacement_integral += weight * value;
            }
            let mut field_squared = 0.0;
            let mut speed_squared = 0.0;
            let mut gradient_squared = 0.0;
            let mut entry = 0;
            for row in 0..7 {
                for column in row..7 {
                    let symmetry = if row == column { 1.0 } else { 2.0 };
                    field_squared += symmetry
                        * matrices.mass[entry]
                        * local_displacement[row]
                        * local_displacement[column];
                    speed_squared += symmetry
                        * matrices.density[entry]
                        * local_velocity[row]
                        * local_velocity[column];
                    gradient_squared += symmetry
                        * matrices.stiffness[entry]
                        * local_displacement[row]
                        * local_displacement[column];
                    entry += 1;
                }
            }
            displacement_squared_integral += field_squared;
            total_energy += 0.5 * (speed_squared + gradient_squared);
        }
        let mean_displacement = displacement_integral / self.covered_area;
        let rms_displacement = (displacement_squared_integral / self.covered_area)
            .max(0.0)
            .sqrt();
        let mean_energy_density = total_energy / self.covered_area;
        let coverage = (self.covered_area / self.target_area).clamp(0.0, 1.0);
        if !mean_displacement.is_finite()
            || !rms_displacement.is_finite()
            || !mean_energy_density.is_finite()
            || !total_energy.is_finite()
            || !coverage.is_finite()
        {
            return Err(AreaProbeError::NonFiniteValues);
        }
        Ok(AreaProbeSample {
            mean_displacement,
            rms_displacement,
            mean_energy_density,
            total_energy,
            covered_area: self.covered_area,
            coverage,
        })
    }
}

/// Returns `[bbox_min, bbox_max, vertices...]` for a bounded CCW circle approximation.
fn disk_polygon(center: Point2, radius: f64) -> Vec<Point2> {
    let tolerance = DISK_APPROXIMATION_TOLERANCE.min(radius * 1.0e-3);
    let cosine = (1.0 - tolerance / radius).clamp(-1.0, 1.0);
    let sides = (std::f64::consts::PI / cosine.acos()).ceil() as usize;
    let sides = sides.clamp(MIN_DISK_POLYGON_SIDES, MAX_DISK_POLYGON_SIDES);
    let mut result = Vec::with_capacity(sides + 2);
    result.push(Point2::new(center.x - radius, center.y - radius));
    result.push(Point2::new(center.x + radius, center.y + radius));
    result.extend((0..sides).map(|index| {
        let angle = std::f64::consts::TAU * index as f64 / sides as f64;
        center + Point2::new(angle.cos(), angle.sin()) * radius
    }));
    result
}

fn clip_convex_polygon(subject: &[Point2], clip: &[Point2]) -> Vec<Point2> {
    let mut output = subject.to_vec();
    for index in 0..clip.len() {
        let a = clip[index];
        let b = clip[(index + 1) % clip.len()];
        let input = std::mem::take(&mut output);
        if input.is_empty() {
            break;
        }
        let mut previous = *input.last().unwrap();
        let mut previous_inside = (b - a).cross(previous - a) >= -1.0e-12;
        for current in input {
            let current_inside = (b - a).cross(current - a) >= -1.0e-12;
            if current_inside != previous_inside {
                let direction = current - previous;
                let denominator = (b - a).cross(direction);
                if denominator.abs() > f64::EPSILON {
                    let fraction = (b - a).cross(a - previous) / denominator;
                    output.push(previous + direction * fraction.clamp(0.0, 1.0));
                }
            }
            if current_inside {
                output.push(current);
            }
            previous = current;
            previous_inside = current_inside;
        }
    }
    output
}

/// Twelve-point degree-six Dunavant rule with weights normalized to sum to one.
fn area_quadrature() -> [([f64; 3], f64); 12] {
    const A: f64 = 0.873_821_971_016_996;
    const B: f64 = 0.063_089_014_491_502;
    const C: f64 = 0.501_426_509_658_179;
    const D: f64 = 0.249_286_745_170_910;
    const E: f64 = 0.636_502_499_121_399;
    const F: f64 = 0.310_352_451_033_785;
    const G: f64 = 0.053_145_049_844_816;
    const W0: f64 = 0.050_844_906_370_207;
    const W1: f64 = 0.116_786_275_726_379;
    const W2: f64 = 0.082_851_075_618_374;
    [
        ([A, B, B], W0),
        ([B, A, B], W0),
        ([B, B, A], W0),
        ([C, D, D], W1),
        ([D, C, D], W1),
        ([D, D, C], W1),
        ([E, F, G], W2),
        ([E, G, F], W2),
        ([F, E, G], W2),
        ([F, G, E], W2),
        ([G, E, F], W2),
        ([G, F, E], W2),
    ]
}

impl QuadraticBoundaryStencil {
    pub fn build(
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        scene: &Scene,
        label: BoundaryLabel,
        parameter: f64,
        period: f64,
        region: RegionId,
    ) -> Result<Self, PointProbeError> {
        Self::build_with_model(
            mesh,
            operator,
            ProbeModel::Scene(scene),
            label,
            parameter,
            period,
            region,
        )
    }

    /// Builds a trace-specific boundary stencil from a topology curve label.
    /// The explicit region selects the adjacent face at transmitting interfaces;
    /// separated curves additionally retain their left/right trace identity.
    pub fn build_topology(
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        plan: &TopologyMeshPlan,
        model: TopologyWaveModel<'_>,
        target: BoundaryStencilTarget,
    ) -> Result<Self, PointProbeError> {
        let mesh_label = match target.label {
            BoundaryLabel::Curve {
                curve,
                span,
                separated: false,
                ..
            } => BoundaryLabel::Curve {
                curve,
                span,
                side: crate::CurveTraceSide::Left,
                separated: false,
            },
            label => label,
        };
        if !(ProbeModel::Topology { plan, model }).contains_boundary_target(
            target.label,
            target.parameter,
            target.period,
            target.region,
        ) {
            return Err(PointProbeError::InvalidMesh);
        }
        Self::build_with_model(
            mesh,
            operator,
            ProbeModel::Topology { plan, model },
            mesh_label,
            target.parameter,
            target.period,
            target.region,
        )
    }

    fn build_with_model(
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        model: ProbeModel<'_>,
        label: BoundaryLabel,
        parameter: f64,
        period: f64,
        region: RegionId,
    ) -> Result<Self, PointProbeError> {
        if !parameter.is_finite()
            || !period.is_finite()
            || period <= 0.0
            || !model.valid()
            || !model.contains_region(region)
            || !model.contains_boundary_target(label, parameter, period, region)
            || mesh.geometry_revision != operator.geometry_revision()
            || mesh.mesh_revision != operator.mesh_revision()
            || mesh.triangles.len() != operator.element_nodes().len()
        {
            return Err(PointProbeError::InvalidMesh);
        }
        let mut located = None;
        'edges: for edge in &mesh.boundary_edges {
            if edge.label != label {
                continue;
            }
            let denominator = edge.parameters[1] - edge.parameters[0];
            if denominator.abs() <= f64::EPSILON {
                continue;
            }
            for shift in -1..=1 {
                let shifted = parameter + shift as f64 * period;
                let fraction = (shifted - edge.parameters[0]) / denominator;
                if !(-1.0e-9..=1.0 + 1.0e-9).contains(&fraction) {
                    continue;
                }
                let fraction = fraction.clamp(0.0, 1.0);
                let [a_index, b_index] = edge.vertices;
                let (Some(a), Some(b)) = (mesh.vertices.get(a_index), mesh.vertices.get(b_index))
                else {
                    return Err(PointProbeError::InvalidMesh);
                };
                let point = a.point.lerp(b.point, fraction);
                for (triangle_index, triangle) in mesh.triangles.iter().enumerate() {
                    if triangle.region != region
                        || !triangle.vertices.contains(&a_index)
                        || !triangle.vertices.contains(&b_index)
                    {
                        continue;
                    }
                    let [ia, ib, ic] = triangle.vertices;
                    let (Some(pa), Some(pb), Some(pc)) = (
                        mesh.vertices.get(ia),
                        mesh.vertices.get(ib),
                        mesh.vertices.get(ic),
                    ) else {
                        return Err(PointProbeError::InvalidMesh);
                    };
                    let twice_area = (pb.point - pa.point).cross(pc.point - pa.point);
                    if !twice_area.is_finite() || twice_area <= 0.0 {
                        return Err(PointProbeError::InvalidMesh);
                    }
                    let barycentric = [
                        (pb.point - point).cross(pc.point - point) / twice_area,
                        (pc.point - point).cross(pa.point - point) / twice_area,
                        (pa.point - point).cross(pb.point - point) / twice_area,
                    ];
                    let gradients = [
                        Point2::new(pb.point.y - pc.point.y, pc.point.x - pb.point.x) / twice_area,
                        Point2::new(pc.point.y - pa.point.y, pa.point.x - pc.point.x) / twice_area,
                        Point2::new(pa.point.y - pb.point.y, pb.point.x - pa.point.x) / twice_area,
                    ];
                    let mut tangent = b.point - a.point;
                    if denominator < 0.0 {
                        tangent = tangent * -1.0;
                    }
                    let length = tangent.norm();
                    if length <= f64::EPSILON {
                        return Err(PointProbeError::InvalidMesh);
                    }
                    tangent = tangent / length;
                    let left = Point2::new(-tangent.y, tangent.x);
                    let centroid = (pa.point + pb.point + pc.point) / 3.0;
                    let outward_normal = if left.dot(centroid - point) <= 0.0 {
                        left
                    } else {
                        left * -1.0
                    };
                    let stencil = QuadraticPointStencil::from_element(
                        mesh,
                        operator,
                        model,
                        triangle_index,
                        region,
                        barycentric,
                        gradients,
                    )?;
                    located = Some(Self {
                        stencil,
                        point,
                        outward_normal,
                    });
                    break 'edges;
                }
            }
        }
        located.ok_or(PointProbeError::OutsideDomain)
    }
}

impl QuadraticFarFieldStencil {
    /// Compiles a rectangular Huygens contour from the unified topology model.
    /// Every modeled curve must lie strictly inside the contour, leaving one
    /// source-free, uniform, isotropic, lossless exterior face around it.
    pub fn build_topology(
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
        plan: &TopologyMeshPlan,
        model: TopologyWaveModel<'_>,
        options: FarFieldCompileOptions<'_>,
    ) -> Result<Self, FarFieldCompileError> {
        let FarFieldCompileOptions {
            inset,
            sample_count,
            point_source,
            volume_sources,
        } = options;
        let active_regions = plan
            .domains
            .iter()
            .map(|domain| domain.region)
            .collect::<BTreeSet<_>>();
        let sources_valid = point_source.is_none_or(|source| {
            source.valid()
                && active_regions.contains(&source.region)
                && model.region(source.region).is_some()
        }) && volume_sources.len() <= crate::MAX_VOLUME_SOURCES
            && volume_sources.iter().enumerate().all(|(index, source)| {
                source.valid()
                    && active_regions.contains(&source.region)
                    && model.region(source.region).is_some()
                    && !volume_sources[..index]
                        .iter()
                        .any(|previous| previous.region == source.region)
            });
        if !model.valid_for(plan)
            || !sources_valid
            || mesh.geometry_revision != plan.geometry_revision
            || mesh.geometry_revision != operator.geometry_revision()
            || mesh.mesh_revision != operator.mesh_revision()
            || mesh.triangles.len() != operator.element_nodes().len()
            || mesh
                .triangles
                .iter()
                .any(|triangle| !active_regions.contains(&triangle.region))
        {
            return Err(FarFieldCompileError::InvalidInputs);
        }
        if !inset.is_finite() || inset <= 0.0 || 2.0 * inset >= plan.domain.minimum_extent() {
            return Err(FarFieldCompileError::InvalidInset);
        }
        if !(4..=MAX_FAR_FIELD_CONTOUR_POINTS).contains(&sample_count) {
            return Err(FarFieldCompileError::InvalidSampleCount);
        }

        let exterior_faces = plan
            .boundaries
            .iter()
            .filter(|boundary| matches!(boundary.source, PlannedBoundarySource::Outer(_)))
            .map(|boundary| (boundary.face, boundary.region))
            .collect::<BTreeSet<_>>();
        let mut exterior_faces = exterior_faces.into_iter();
        let Some((exterior_face, exterior_region)) = exterior_faces.next() else {
            return Err(FarFieldCompileError::InvalidInputs);
        };
        if exterior_faces.next().is_some() {
            return Err(FarFieldCompileError::MultipleExteriorFaces);
        }

        let margin = plan.domain.tolerance() * 0.25;
        let enclosed = |point: Point2| {
            point.x > plan.domain.min_x + inset + margin
                && point.x < plan.domain.max_x - inset - margin
                && point.y > plan.domain.min_y + inset + margin
                && point.y < plan.domain.max_y - inset - margin
        };
        if plan.boundaries.iter().any(|boundary| {
            matches!(boundary.source, PlannedBoundarySource::Curve { .. })
                && boundary.points.into_iter().any(|point| !enclosed(point))
        }) {
            return Err(FarFieldCompileError::GeometryOutsideContour);
        }
        if point_source.is_some_and(|source| source.enabled && !enclosed(source.position)) {
            return Err(FarFieldCompileError::DrivenExterior);
        }

        if !plan
            .domains
            .iter()
            .any(|domain| domain.face == exterior_face && domain.region == exterior_region)
        {
            return Err(FarFieldCompileError::InvalidInputs);
        }
        let region = model
            .region(exterior_region)
            .ok_or(FarFieldCompileError::InvalidInputs)?;
        let material = model
            .material(region.material)
            .and_then(crate::Material::uniform)
            .ok_or(FarFieldCompileError::NonUniformExterior)?;
        validate_far_field_material(material, volume_sources, exterior_region)?;
        let wave_speed = model.physics.wave_speed(WaveCoefficients {
            mass_density: material.mass_density,
            stiffness: material.stiffness,
            damping: material.damping,
        });
        if !wave_speed.is_finite() || wave_speed <= 0.0 {
            return Err(FarFieldCompileError::InvalidWaveSpeed);
        }

        let samples = rectangular_far_field_contour(plan.domain, inset, sample_count)
            .into_iter()
            .map(|(position, normal)| {
                let stencil =
                    QuadraticPointStencil::build_topology(mesh, operator, plan, model, position)
                        .map_err(FarFieldCompileError::ContourUnavailable)?;
                if stencil.region != exterior_region {
                    return Err(FarFieldCompileError::ContourLeavesExterior);
                }
                Ok((stencil, position, normal))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let perimeter = 2.0 * (plan.domain.width() + plan.domain.height() - 4.0 * inset);
        Ok(Self {
            samples,
            exterior_region,
            wave_speed,
            sample_spacing: perimeter / sample_count as f64,
            delay_margin: plan
                .domain
                .corners()
                .into_iter()
                .map(Point2::norm)
                .fold(0.0, f64::max)
                / wave_speed,
        })
    }
}

fn validate_far_field_material(
    material: EvaluatedMaterial,
    sources: &[VolumeSource],
    exterior_region: RegionId,
) -> Result<(), FarFieldCompileError> {
    if material.damping.abs() > 1.0e-12 {
        return Err(FarFieldCompileError::LossyExterior);
    }
    if (material.axis_ratio - 1.0).abs() > 1.0e-12 {
        return Err(FarFieldCompileError::AnisotropicExterior);
    }
    if sources
        .iter()
        .any(|source| source.enabled && source.region == exterior_region)
    {
        return Err(FarFieldCompileError::DrivenExterior);
    }
    Ok(())
}

fn rectangular_far_field_contour(
    domain: crate::DomainRect,
    inset: f64,
    sample_count: usize,
) -> Vec<(Point2, Point2)> {
    let min_x = domain.min_x + inset;
    let max_x = domain.max_x - inset;
    let min_y = domain.min_y + inset;
    let max_y = domain.max_y - inset;
    let width = max_x - min_x;
    let height = max_y - min_y;
    let perimeter = 2.0 * (width + height);
    let spacing = perimeter / sample_count as f64;
    (0..sample_count)
        .map(|index| {
            let distance = (index as f64 + 0.5) * spacing;
            if distance < width {
                (Point2::new(min_x + distance, min_y), Point2::new(0.0, -1.0))
            } else if distance < width + height {
                (
                    Point2::new(max_x, min_y + distance - width),
                    Point2::new(1.0, 0.0),
                )
            } else if distance < 2.0 * width + height {
                (
                    Point2::new(max_x - (distance - width - height), max_y),
                    Point2::new(0.0, 1.0),
                )
            } else {
                (
                    Point2::new(min_x, max_y - (distance - 2.0 * width - height)),
                    Point2::new(-1.0, 0.0),
                )
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BACKGROUND_REGION, BoundaryEdge, CurveId, CurveNode, CurveSpan, CurveSpanId, CurveSpline,
        DEFAULT_MATERIAL, FaceBoundaryCondition, FaceRegionAssignment, InternalBoundaryCoupling,
        Material, MaterialId, MeshQuality, MeshTriangle, MeshVertex, ObstacleId, OpenCubicSpline,
        OuterBoundaryCondition, OuterSide, PeriodicCubicSpline, QuadraticWaveOperator, Region,
        ScalarField, SpanBehavior, TimeSignal, TopologyCurve, TopologyGeometry, TopologyVertex,
        TopologyVertexId, TopologyVertexLocation, compile_topology, mesh_topology_plan,
    };

    fn fixture() -> (TriMesh, Scene, QuadraticWaveOperator) {
        let mesh = TriMesh {
            geometry_revision: 3,
            mesh_revision: 7,
            vertices: vec![
                MeshVertex {
                    point: Point2::new(0.0, 0.0),
                    boundary: None,
                    trace: None,
                },
                MeshVertex {
                    point: Point2::new(1.0, 0.0),
                    boundary: None,
                    trace: None,
                },
                MeshVertex {
                    point: Point2::new(0.0, 1.0),
                    boundary: None,
                    trace: None,
                },
            ],
            triangles: vec![MeshTriangle {
                vertices: [0, 1, 2],
                region: BACKGROUND_REGION,
            }],
            boundary_edges: vec![],
            requested_sizes: vec![],
            quality: MeshQuality {
                minimum_angle_degrees: 45.0,
                maximum_edge_length: 2.0_f64.sqrt(),
            },
        };
        let scene = Scene {
            materials: vec![Material {
                mass_density: crate::ScalarField::constant(2.0),
                stiffness: crate::ScalarField::constant(3.0),
                ..Material::default_medium()
            }],
            regions: vec![Region {
                id: BACKGROUND_REGION,
                material: DEFAULT_MATERIAL,
                frame: crate::MaterialFrame::world(),
            }],
            ..Scene::default()
        };
        let operator = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        (mesh, scene, operator)
    }

    fn topology_fixture(
        geometry: TopologyGeometry,
    ) -> (TopologyMeshPlan, TriMesh, Scene, QuadraticWaveOperator) {
        let topology = compile_topology(&geometry, 40).unwrap();
        let assignments = topology
            .faces
            .iter()
            .enumerate()
            .map(|(index, face)| FaceRegionAssignment {
                face: face.id,
                region: Some(RegionId(index as u64 + 1)),
            })
            .collect::<Vec<_>>();
        let plan = TopologyMeshPlan::new(&topology, &assignments).unwrap();
        let mesh = mesh_topology_plan(
            &plan,
            41,
            crate::MeshingOptions {
                target_edge_length: 0.35,
                minimum_angle_degrees: 8.0,
                max_vertices: 20_000,
                max_triangles: 40_000,
                max_refinement_steps: 20_000,
                ..crate::MeshingOptions::default()
            },
        )
        .unwrap();
        let scene = Scene {
            regions: assignments
                .iter()
                .map(|assignment| Region {
                    id: assignment.region.unwrap(),
                    material: DEFAULT_MATERIAL,
                    frame: crate::MaterialFrame::world(),
                })
                .chain(std::iter::once(Region {
                    id: RegionId(99),
                    material: DEFAULT_MATERIAL,
                    frame: crate::MaterialFrame::world(),
                }))
                .collect(),
            ..Scene::default()
        };
        let operator = QuadraticWaveOperator::assemble_topology(
            &mesh,
            &plan,
            TopologyWaveModel::from_scene(&scene),
        )
        .unwrap();
        (plan, mesh, scene, operator)
    }

    fn topology_baffle() -> TopologyGeometry {
        TopologyGeometry {
            curves: vec![
                TopologyCurve::new(
                    CurveId(1),
                    CurveSpline::Open(
                        OpenCubicSpline::polyline(vec![
                            Point2::new(-0.55, -0.06),
                            Point2::new(0.55, 0.08),
                        ])
                        .unwrap(),
                    ),
                    vec![CurveSpan {
                        id: CurveSpanId(1),
                        behavior: SpanBehavior::Separated {
                            left: FaceBoundaryCondition::Reflecting,
                            right: FaceBoundaryCondition::Reflecting,
                            coupling: InternalBoundaryCoupling::Independent,
                        },
                    }],
                )
                .unwrap(),
            ],
            ..TopologyGeometry::default()
        }
    }

    fn topology_divider() -> TopologyGeometry {
        let endpoints = [TopologyVertexId(1), TopologyVertexId(2)];
        let mut curve = TopologyCurve::new(
            CurveId(2),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![Point2::new(0.0, -1.0), Point2::new(0.0, 1.0)])
                    .unwrap(),
            ),
            vec![CurveSpan {
                id: CurveSpanId(2),
                behavior: SpanBehavior::Transmitting,
            }],
        )
        .unwrap();
        curve.nodes = endpoints
            .map(|vertex| CurveNode {
                vertex: Some(vertex),
            })
            .to_vec();
        TopologyGeometry {
            curves: vec![curve],
            vertices: vec![
                TopologyVertex {
                    id: endpoints[0],
                    location: TopologyVertexLocation::Outer {
                        side: OuterSide::Bottom,
                        fraction: 0.5,
                    },
                },
                TopologyVertex {
                    id: endpoints[1],
                    location: TopologyVertexLocation::Outer {
                        side: OuterSide::Top,
                        fraction: 0.5,
                    },
                },
            ],
            ..TopologyGeometry::default()
        }
    }

    fn topology_inclusion() -> TopologyGeometry {
        TopologyGeometry {
            curves: vec![
                TopologyCurve::new(
                    CurveId(4),
                    CurveSpline::Closed(
                        PeriodicCubicSpline::polygon(vec![
                            Point2::new(-0.3, -0.3),
                            Point2::new(0.3, -0.3),
                            Point2::new(0.3, 0.3),
                            Point2::new(-0.3, 0.3),
                        ])
                        .unwrap(),
                    ),
                    (0..4)
                        .map(|index| CurveSpan {
                            id: CurveSpanId(10 + index),
                            behavior: SpanBehavior::Transmitting,
                        })
                        .collect(),
                )
                .unwrap(),
            ],
            ..TopologyGeometry::default()
        }
    }

    #[test]
    fn samples_affine_field_and_local_energy() {
        let (mesh, scene, operator) = fixture();
        let point = Point2::new(0.2, 0.3);
        let stencil = QuadraticPointStencil::build(&mesh, &operator, &scene, point).unwrap();
        let displacement = operator
            .node_points()
            .iter()
            .map(|p| 1.0 + 2.0 * p.x - p.y)
            .collect::<Vec<_>>();
        let velocity = vec![4.0; operator.degrees_of_freedom()];
        let sample = stencil.sample(&displacement, &velocity).unwrap();
        assert!((sample.displacement - 1.1).abs() < 1.0e-12);
        assert!((sample.velocity - 4.0).abs() < 1.0e-12);
        assert!((sample.gradient - Point2::new(2.0, -1.0)).norm() < 1.0e-12);
        assert!((sample.energy_density - (0.5 * 2.0 * 16.0 + 0.5 * 3.0 * 5.0)).abs() < 1.0e-11);
    }

    #[test]
    fn point_probe_uses_coefficients_at_the_sample_position() {
        let (mesh, mut scene, _) = fixture();
        scene.materials[0].mass_density = crate::ScalarField::formula("1 + x").unwrap();
        scene.materials[0].stiffness = crate::ScalarField::formula("2 + y").unwrap();
        let operator = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let point = Point2::new(0.2, 0.3);
        let stencil = QuadraticPointStencil::build(&mesh, &operator, &scene, point).unwrap();
        let displacement = operator
            .node_points()
            .iter()
            .map(|p| 2.0 * p.x - p.y)
            .collect::<Vec<_>>();
        let velocity = vec![4.0; operator.degrees_of_freedom()];
        let sample = stencil.sample(&displacement, &velocity).unwrap();
        let expected = 0.5 * 1.2 * 16.0 + 0.5 * 2.3 * 5.0;
        assert!((sample.energy_density - expected).abs() < 1.0e-11);
    }

    #[test]
    fn point_probe_energy_uses_the_directional_quadratic_form() {
        let (mesh, mut scene, _) = fixture();
        scene.materials[0].axis_ratio = crate::ScalarField::constant(4.0);
        let operator = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let stencil =
            QuadraticPointStencil::build(&mesh, &operator, &scene, Point2::new(0.2, 0.3)).unwrap();
        let displacement = operator
            .node_points()
            .iter()
            .map(|point| 2.0 * point.x - point.y)
            .collect::<Vec<_>>();
        let sample = stencil
            .sample(&displacement, &vec![4.0; operator.degrees_of_freedom()])
            .unwrap();
        assert!((sample.energy_density - 40.375).abs() < 1.0e-11);
    }

    #[test]
    fn integrates_region_field_and_energy() {
        let (mesh, scene, operator) = fixture();
        let stencil = QuadraticAreaStencil::build(
            &mesh,
            &operator,
            &scene,
            AreaProbeShape::Region(BACKGROUND_REGION),
        )
        .unwrap();
        let displacement = operator
            .node_points()
            .iter()
            .map(|point| 1.0 + 2.0 * point.x - point.y)
            .collect::<Vec<_>>();
        let velocity = vec![4.0; operator.degrees_of_freedom()];
        let sample = stencil.sample(&displacement, &velocity).unwrap();
        assert!((sample.covered_area - 0.5).abs() < 1.0e-12);
        assert_eq!(sample.coverage, 1.0);
        assert!((sample.mean_displacement - 4.0 / 3.0).abs() < 1.0e-12);
        assert!((sample.rms_displacement - (13.0_f64 / 6.0).sqrt()).abs() < 1.0e-12);
        assert!((sample.mean_energy_density - 23.5).abs() < 1.0e-11);
        assert!((sample.total_energy - 11.75).abs() < 1.0e-11);
    }

    #[test]
    fn disk_uses_world_space_clipping_and_reports_coverage() {
        let (mesh, scene, operator) = fixture();
        let radius = 0.1;
        let stencil = QuadraticAreaStencil::build(
            &mesh,
            &operator,
            &scene,
            AreaProbeShape::Disk {
                center: Point2::new(0.25, 0.25),
                radius,
            },
        )
        .unwrap();
        let sample = stencil
            .sample(
                &vec![2.0; operator.degrees_of_freedom()],
                &vec![3.0; operator.degrees_of_freedom()],
            )
            .unwrap();
        assert!((sample.mean_displacement - 2.0).abs() < 1.0e-12);
        assert!((sample.rms_displacement - 2.0).abs() < 1.0e-12);
        assert!((sample.mean_energy_density - 9.0).abs() < 1.0e-11);
        assert!(sample.coverage > 0.998 && sample.coverage <= 1.0);

        let partial = QuadraticAreaStencil::build(
            &mesh,
            &operator,
            &scene,
            AreaProbeShape::Disk {
                center: Point2::new(0.0, 0.0),
                radius,
            },
        )
        .unwrap();
        assert!(partial.covered_area / partial.target_area > 0.24);
        assert!(partial.covered_area / partial.target_area < 0.26);
    }

    #[test]
    fn rejects_invalid_area_targets_and_fields() {
        let (mesh, scene, operator) = fixture();
        assert_eq!(
            QuadraticAreaStencil::build(
                &mesh,
                &operator,
                &scene,
                AreaProbeShape::Disk {
                    center: Point2::default(),
                    radius: 0.0,
                },
            ),
            Err(AreaProbeError::InvalidTarget)
        );
        assert_eq!(
            QuadraticAreaStencil::build(
                &mesh,
                &operator,
                &scene,
                AreaProbeShape::Region(RegionId(999)),
            ),
            Err(AreaProbeError::MissingRegion)
        );
        let stencil = QuadraticAreaStencil::build(
            &mesh,
            &operator,
            &scene,
            AreaProbeShape::Region(BACKGROUND_REGION),
        )
        .unwrap();
        assert_eq!(stencil.sample(&[], &[]), Err(AreaProbeError::SizeMismatch));
    }

    #[test]
    fn boundary_stencil_uses_selected_edge_and_outward_normal() {
        let (mut mesh, scene, operator) = fixture();
        mesh.boundary_edges.push(BoundaryEdge {
            vertices: [0, 1],
            label: BoundaryLabel::Outer(crate::OuterSide::Bottom),
            parameters: [0.0, 1.0],
        });
        let sample = QuadraticBoundaryStencil::build(
            &mesh,
            &operator,
            &scene,
            BoundaryLabel::Outer(crate::OuterSide::Bottom),
            0.25,
            1.0,
            BACKGROUND_REGION,
        )
        .unwrap();
        assert!((sample.point - Point2::new(0.25, 0.0)).norm() < 1.0e-12);
        assert!((sample.outward_normal - Point2::new(0.0, -1.0)).norm() < 1.0e-12);
        assert_eq!(sample.stencil.region, BACKGROUND_REGION);
    }

    #[test]
    fn boundary_stencil_selects_material_interface_trace() {
        let mut scene = Scene::initial();
        let interior = RegionId(2);
        scene.regions.push(Region {
            id: interior,
            material: DEFAULT_MATERIAL,
            frame: crate::MaterialFrame::world(),
        });
        scene.obstacles[0].role = crate::LoopRole::MaterialInterface {
            exterior: BACKGROUND_REGION,
            interior,
        };
        let mesh = crate::mesh_scene(&scene, 4, crate::MeshingOptions::default()).unwrap();
        let operator = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let parameter = 0.5;
        let exterior = QuadraticBoundaryStencil::build(
            &mesh,
            &operator,
            &scene,
            BoundaryLabel::MaterialInterface(ObstacleId(1)),
            parameter,
            scene.obstacles[0].spline.period(),
            BACKGROUND_REGION,
        )
        .unwrap();
        let interior_sample = QuadraticBoundaryStencil::build(
            &mesh,
            &operator,
            &scene,
            BoundaryLabel::MaterialInterface(ObstacleId(1)),
            parameter,
            scene.obstacles[0].spline.period(),
            interior,
        )
        .unwrap();
        assert_eq!(exterior.stencil.region, BACKGROUND_REGION);
        assert_eq!(interior_sample.stencil.region, interior);
        assert!(exterior.outward_normal.dot(interior_sample.outward_normal) < -0.99);
    }

    #[test]
    fn boundary_stencil_keeps_baffle_faces_distinct() {
        let mut scene = Scene::default();
        let spline = crate::OpenCubicSpline::uniform(vec![
            Point2::new(-0.7, 0.0),
            Point2::new(-0.25, 0.0),
            Point2::new(0.25, 0.0),
            Point2::new(0.7, 0.0),
        ])
        .unwrap();
        let period = spline.period();
        scene.internal_boundaries.push(crate::InternalBoundary {
            id: crate::InternalBoundaryId(1),
            spline,
            region: BACKGROUND_REGION,
            span_laws: vec![crate::InternalBoundaryLaw::REFLECTING],
        });
        let mesh = crate::mesh_scene(&scene, 5, crate::MeshingOptions::default()).unwrap();
        let operator = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let sample = |side| {
            QuadraticBoundaryStencil::build(
                &mesh,
                &operator,
                &scene,
                BoundaryLabel::InternalBoundary {
                    id: crate::InternalBoundaryId(1),
                    side,
                },
                period * 0.5,
                period,
                BACKGROUND_REGION,
            )
            .unwrap()
        };
        let left = sample(crate::InternalBoundarySide::Left);
        let right = sample(crate::InternalBoundarySide::Right);
        assert!(left.outward_normal.dot(right.outward_normal) < -0.99);
        assert_ne!(left.stencil.nodes, right.stencil.nodes);
    }

    #[test]
    fn rejects_points_outside_the_domain() {
        let (mesh, scene, operator) = fixture();
        assert_eq!(
            QuadraticPointStencil::build(&mesh, &operator, &scene, Point2::new(2.0, 2.0)),
            Err(PointProbeError::OutsideDomain)
        );
    }

    #[test]
    fn rejects_a_point_on_a_two_trace_boundary() {
        let (mut mesh, scene, operator) = fixture();
        mesh.boundary_edges.push(BoundaryEdge {
            vertices: [0, 1],
            label: BoundaryLabel::MaterialInterface(ObstacleId(3)),
            parameters: [0.0, 1.0],
        });
        assert_eq!(
            QuadraticPointStencil::build(&mesh, &operator, &scene, Point2::new(0.4, 0.0)),
            Err(PointProbeError::AmbiguousBoundary)
        );
    }

    #[test]
    fn topology_point_and_area_stencils_use_active_face_regions() {
        let (plan, mesh, scene, operator) = topology_fixture(TopologyGeometry::default());
        let model = TopologyWaveModel::from_scene(&scene);
        let point = Point2::new(0.2, 0.3);
        let stencil =
            QuadraticPointStencil::build_topology(&mesh, &operator, &plan, model, point).unwrap();
        assert_eq!(stencil.region, BACKGROUND_REGION);
        let area = QuadraticAreaStencil::build_topology(
            &mesh,
            &operator,
            &plan,
            model,
            AreaProbeShape::Region(BACKGROUND_REGION),
        )
        .unwrap();
        assert!((area.covered_area - 4.0).abs() < 1.0e-9);
        assert_eq!(
            QuadraticAreaStencil::build_topology(
                &mesh,
                &operator,
                &plan,
                model,
                AreaProbeShape::Region(RegionId(99)),
            ),
            Err(AreaProbeError::MissingRegion)
        );
    }

    #[test]
    fn topology_line_samples_gap_at_a_curve_and_boundary_sides_stay_distinct() {
        let (plan, mesh, scene, operator) = topology_fixture(topology_baffle());
        let model = TopologyWaveModel::from_scene(&scene);
        let left_edge = mesh
            .boundary_edges
            .iter()
            .find(|edge| {
                matches!(
                    edge.label,
                    BoundaryLabel::Curve {
                        side: crate::CurveTraceSide::Left,
                        separated: true,
                        ..
                    }
                )
            })
            .copied()
            .unwrap();
        let right_edge = mesh
            .boundary_edges
            .iter()
            .find(|edge| {
                matches!(
                    edge.label,
                    BoundaryLabel::Curve {
                        side: crate::CurveTraceSide::Right,
                        separated: true,
                        ..
                    }
                ) && edge.parameters[0].min(edge.parameters[1])
                    == left_edge.parameters[0].min(left_edge.parameters[1])
                    && edge.parameters[0].max(edge.parameters[1])
                        == left_edge.parameters[0].max(left_edge.parameters[1])
            })
            .copied()
            .unwrap();
        let parameter = 0.5 * (left_edge.parameters[0] + left_edge.parameters[1]);
        let left = QuadraticBoundaryStencil::build_topology(
            &mesh,
            &operator,
            &plan,
            model,
            BoundaryStencilTarget {
                label: left_edge.label,
                parameter,
                period: 1.0,
                region: BACKGROUND_REGION,
            },
        )
        .unwrap();
        let right = QuadraticBoundaryStencil::build_topology(
            &mesh,
            &operator,
            &plan,
            model,
            BoundaryStencilTarget {
                label: right_edge.label,
                parameter,
                period: 1.0,
                region: BACKGROUND_REGION,
            },
        )
        .unwrap();
        assert_ne!(left.stencil.nodes, right.stencil.nodes);
        assert!(left.outward_normal.dot(right.outward_normal) < -0.99);
        assert_eq!(
            QuadraticPointStencil::build_topology(&mesh, &operator, &plan, model, left.point,),
            Err(PointProbeError::AmbiguousBoundary)
        );
        assert!(
            QuadraticPointStencil::build_topology(
                &mesh,
                &operator,
                &plan,
                model,
                left.point + Point2::new(0.0, 0.1),
            )
            .is_ok()
        );
        assert!(
            QuadraticPointStencil::build_topology(
                &mesh,
                &operator,
                &plan,
                model,
                left.point - Point2::new(0.0, 0.1),
            )
            .is_ok()
        );
    }

    #[test]
    fn topology_probes_follow_transmitting_face_assignments() {
        let (plan, mesh, scene, operator) = topology_fixture(topology_divider());
        let model = TopologyWaveModel::from_scene(&scene);
        let regions = [Point2::new(-0.5, 0.2), Point2::new(0.5, 0.2)].map(|point| {
            QuadraticPointStencil::build_topology(&mesh, &operator, &plan, model, point)
                .unwrap()
                .region
        });
        assert_ne!(regions[0], regions[1]);
        for region in regions {
            let area = QuadraticAreaStencil::build_topology(
                &mesh,
                &operator,
                &plan,
                model,
                AreaProbeShape::Region(region),
            )
            .unwrap();
            assert!((area.covered_area - 2.0).abs() < 1.0e-9);
        }
        assert_eq!(
            QuadraticPointStencil::build_topology(
                &mesh,
                &operator,
                &plan,
                model,
                Point2::new(0.0, 0.2),
            ),
            Err(PointProbeError::AmbiguousBoundary)
        );

        let edge = mesh
            .boundary_edges
            .iter()
            .find(|edge| {
                matches!(
                    edge.label,
                    BoundaryLabel::Curve {
                        separated: false,
                        ..
                    }
                )
            })
            .unwrap();
        let parameter = 0.5 * (edge.parameters[0] + edge.parameters[1]);
        let BoundaryLabel::Curve { curve, span, .. } = edge.label else {
            unreachable!()
        };
        let traces = [
            (regions[0], crate::CurveTraceSide::Left),
            (regions[1], crate::CurveTraceSide::Right),
        ]
        .map(|(region, side)| {
            QuadraticBoundaryStencil::build_topology(
                &mesh,
                &operator,
                &plan,
                model,
                BoundaryStencilTarget {
                    label: BoundaryLabel::Curve {
                        curve,
                        span,
                        side,
                        separated: false,
                    },
                    parameter,
                    period: 1.0,
                    region,
                },
            )
            .unwrap()
        });
        assert_eq!(traces[0].stencil.region, regions[0]);
        assert_eq!(traces[1].stencil.region, regions[1]);
        assert!(traces[0].outward_normal.dot(traces[1].outward_normal) < -0.99);
    }

    #[test]
    fn topology_far_field_compiles_one_uniform_exterior_around_an_internal_baffle() {
        let (plan, mesh, scene, operator) = topology_fixture(topology_baffle());
        let compiled = QuadraticFarFieldStencil::build_topology(
            &mesh,
            &operator,
            &plan,
            TopologyWaveModel::from_scene(&scene),
            FarFieldCompileOptions {
                inset: 0.12,
                sample_count: 64,
                point_source: Some(PointSource {
                    enabled: true,
                    ..PointSource::default()
                }),
                volume_sources: &[],
            },
        )
        .unwrap();
        assert_eq!(compiled.samples.len(), 64);
        assert_eq!(compiled.exterior_region, BACKGROUND_REGION);
        assert!((compiled.wave_speed - 1.0).abs() < 1.0e-12);
        assert!((compiled.sample_spacing - 7.04 / 64.0).abs() < 1.0e-12);
        assert!((compiled.delay_margin - 2.0_f64.sqrt()).abs() < 1.0e-12);
        assert!(compiled.samples.iter().all(|(stencil, point, normal)| {
            stencil.region == BACKGROUND_REGION
                && (normal.norm() - 1.0).abs() < 1.0e-12
                && normal.dot(*point) > 0.0
        }));

        let (inclusion_plan, inclusion_mesh, mut inclusion_scene, _) =
            topology_fixture(topology_inclusion());
        let exterior_region = inclusion_plan
            .boundaries
            .iter()
            .find_map(|boundary| {
                matches!(boundary.source, PlannedBoundarySource::Outer(_))
                    .then_some(boundary.region)
            })
            .unwrap();
        let interior_region = inclusion_plan
            .domains
            .iter()
            .find_map(|domain| (domain.region != exterior_region).then_some(domain.region))
            .unwrap();
        inclusion_scene.materials.push(Material {
            id: MaterialId(2),
            mass_density: ScalarField::formula("1 + 0.1 * x").unwrap(),
            ..Material::default_medium()
        });
        inclusion_scene
            .regions
            .iter_mut()
            .find(|region| region.id == interior_region)
            .unwrap()
            .material = MaterialId(2);
        let interior_source = VolumeSource {
            region: interior_region,
            enabled: true,
            profile: ScalarField::constant(1.0),
            parameters: vec![],
            signal: TimeSignal::ZERO,
        };
        let inclusion_operator = QuadraticWaveOperator::assemble_topology(
            &inclusion_mesh,
            &inclusion_plan,
            TopologyWaveModel::from_scene(&inclusion_scene),
        )
        .unwrap();
        QuadraticFarFieldStencil::build_topology(
            &inclusion_mesh,
            &inclusion_operator,
            &inclusion_plan,
            TopologyWaveModel::from_scene(&inclusion_scene),
            FarFieldCompileOptions {
                inset: 0.12,
                sample_count: 64,
                point_source: None,
                volume_sources: &[interior_source],
            },
        )
        .unwrap();
    }

    #[test]
    fn topology_far_field_rejects_nonuniform_driven_or_partitioned_exteriors() {
        let (plan, mesh, scene, operator) = topology_fixture(TopologyGeometry::default());
        let compile = |scene: &Scene, sources: &[VolumeSource]| {
            QuadraticFarFieldStencil::build_topology(
                &mesh,
                &operator,
                &plan,
                TopologyWaveModel::from_scene(scene),
                FarFieldCompileOptions {
                    inset: 0.12,
                    sample_count: 64,
                    point_source: None,
                    volume_sources: sources,
                },
            )
        };

        let mut varying = scene.clone();
        varying.materials[0].mass_density = ScalarField::formula("1 + 0.1 * x").unwrap();
        assert_eq!(
            compile(&varying, &[]),
            Err(FarFieldCompileError::NonUniformExterior)
        );

        let mut anisotropic = scene.clone();
        anisotropic.materials[0].axis_ratio = ScalarField::constant(1.5);
        assert_eq!(
            compile(&anisotropic, &[]),
            Err(FarFieldCompileError::AnisotropicExterior)
        );

        let source = VolumeSource {
            region: BACKGROUND_REGION,
            enabled: true,
            profile: ScalarField::constant(1.0),
            parameters: vec![],
            signal: TimeSignal::ZERO,
        };
        assert_eq!(
            compile(&scene, &[source]),
            Err(FarFieldCompileError::DrivenExterior)
        );
        assert_eq!(
            QuadraticFarFieldStencil::build_topology(
                &mesh,
                &operator,
                &plan,
                TopologyWaveModel::from_scene(&scene),
                FarFieldCompileOptions {
                    inset: 0.12,
                    sample_count: 64,
                    point_source: Some(PointSource {
                        enabled: true,
                        position: Point2::new(0.95, 0.0),
                        ..PointSource::default()
                    }),
                    volume_sources: &[],
                },
            ),
            Err(FarFieldCompileError::DrivenExterior)
        );

        let (divider_plan, divider_mesh, divider_scene, divider_operator) =
            topology_fixture(topology_divider());
        assert_eq!(
            QuadraticFarFieldStencil::build_topology(
                &divider_mesh,
                &divider_operator,
                &divider_plan,
                TopologyWaveModel::from_scene(&divider_scene),
                FarFieldCompileOptions {
                    inset: 0.12,
                    sample_count: 64,
                    point_source: None,
                    volume_sources: &[],
                },
            ),
            Err(FarFieldCompileError::MultipleExteriorFaces)
        );

        let near_shell = TopologyGeometry {
            curves: vec![
                TopologyCurve::new(
                    CurveId(3),
                    CurveSpline::Open(
                        OpenCubicSpline::polyline(vec![
                            Point2::new(-0.5, -0.1),
                            Point2::new(0.95, 0.1),
                        ])
                        .unwrap(),
                    ),
                    vec![CurveSpan {
                        id: CurveSpanId(3),
                        behavior: SpanBehavior::Separated {
                            left: FaceBoundaryCondition::Reflecting,
                            right: FaceBoundaryCondition::Reflecting,
                            coupling: InternalBoundaryCoupling::Independent,
                        },
                    }],
                )
                .unwrap(),
            ],
            ..TopologyGeometry::default()
        };
        let (near_plan, near_mesh, near_scene, near_operator) = topology_fixture(near_shell);
        assert_eq!(
            QuadraticFarFieldStencil::build_topology(
                &near_mesh,
                &near_operator,
                &near_plan,
                TopologyWaveModel::from_scene(&near_scene),
                FarFieldCompileOptions {
                    inset: 0.12,
                    sample_count: 64,
                    point_source: None,
                    volume_sources: &[],
                },
            ),
            Err(FarFieldCompileError::GeometryOutsideContour)
        );
    }
}
