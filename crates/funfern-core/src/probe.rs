use crate::{
    BoundaryLabel, Point2, QuadraticWaveOperator, RegionId, Scene, TriMesh,
    enriched_quadratic_basis, enriched_quadratic_basis_gradients, point_segment_distance,
};

const BOUNDARY_TOLERANCE: f64 = 1.0e-9;
const MIN_DISK_POLYGON_SIDES: usize = 24;
const MAX_DISK_POLYGON_SIDES: usize = 128;
const DISK_APPROXIMATION_TOLERANCE: f64 = 2.0e-4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuadraticPointStencil {
    pub nodes: [u32; 7],
    pub value_weights: [f64; 7],
    pub gradient_weights: [Point2; 7],
    pub region: RegionId,
    pub mass_density: f64,
    pub stiffness: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuadraticBoundaryStencil {
    pub stencil: QuadraticPointStencil,
    pub point: Point2,
    /// Unit normal pointing away from the sampled trace's adjacent element.
    pub outward_normal: Point2,
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
    pub mass_density: f64,
    pub stiffness: f64,
    pub area: f64,
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
        if !point.finite()
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
            operator,
            scene,
            triangle_index,
            region,
            barycentric,
            gradients,
        )
    }

    fn from_element(
        operator: &QuadraticWaveOperator,
        scene: &Scene,
        triangle_index: usize,
        region: RegionId,
        barycentric: [f64; 3],
        gradients: [Point2; 3],
    ) -> Result<Self, PointProbeError> {
        let material = scene
            .region_material(region)
            .ok_or(PointProbeError::InvalidMesh)?;
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
            0.5 * (self.mass_density * speed * speed + self.stiffness * gradient.dot(gradient));
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
        if mesh.geometry_revision != operator.geometry_revision()
            || mesh.mesh_revision != operator.mesh_revision()
            || mesh.triangles.len() != operator.element_nodes().len()
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
                if scene.region(region).is_none() {
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
            let material = scene
                .region_material(triangle.region)
                .ok_or(AreaProbeError::InvalidMesh)?;
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
                elements.push(QuadraticAreaElement {
                    nodes,
                    barycentric_vertices,
                    barycentric_gradients: gradients,
                    region: triangle.region,
                    mass_density: material.mass_density,
                    stiffness: material.stiffness,
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
            for (local, weight) in area_quadrature() {
                let barycentric = std::array::from_fn(|coordinate| {
                    element.barycentric_vertices[0][coordinate] * local[0]
                        + element.barycentric_vertices[1][coordinate] * local[1]
                        + element.barycentric_vertices[2][coordinate] * local[2]
                });
                let value_weights = enriched_quadratic_basis(barycentric);
                let gradient_weights =
                    enriched_quadratic_basis_gradients(barycentric, element.barycentric_gradients);
                let mut field = 0.0;
                let mut speed = 0.0;
                let mut gradient = Point2::default();
                for node in 0..7 {
                    field += value_weights[node] * local_displacement[node];
                    speed += value_weights[node] * local_velocity[node];
                    gradient = gradient + gradient_weights[node] * local_displacement[node];
                }
                let energy_density = 0.5
                    * (element.mass_density * speed * speed
                        + element.stiffness * gradient.dot(gradient));
                let physical_weight = element.area * weight;
                displacement_integral += physical_weight * field;
                displacement_squared_integral += physical_weight * field * field;
                total_energy += physical_weight * energy_density;
            }
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
        if !parameter.is_finite()
            || !period.is_finite()
            || period <= 0.0
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
                        operator,
                        scene,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BACKGROUND_REGION, BoundaryEdge, DEFAULT_MATERIAL, Material, MeshQuality, MeshTriangle,
        MeshVertex, ObstacleId, OuterBoundaryCondition, QuadraticWaveOperator, Region,
    };

    fn fixture() -> (TriMesh, Scene, QuadraticWaveOperator) {
        let mesh = TriMesh {
            geometry_revision: 3,
            mesh_revision: 7,
            vertices: vec![
                MeshVertex {
                    point: Point2::new(0.0, 0.0),
                    boundary: None,
                },
                MeshVertex {
                    point: Point2::new(1.0, 0.0),
                    boundary: None,
                },
                MeshVertex {
                    point: Point2::new(0.0, 1.0),
                    boundary: None,
                },
            ],
            triangles: vec![MeshTriangle {
                vertices: [0, 1, 2],
                region: BACKGROUND_REGION,
            }],
            boundary_edges: vec![],
            quality: MeshQuality {
                minimum_angle_degrees: 45.0,
                maximum_edge_length: 2.0_f64.sqrt(),
            },
        };
        let scene = Scene {
            materials: vec![Material {
                mass_density: 2.0,
                stiffness: 3.0,
                ..Material::default_medium()
            }],
            regions: vec![Region {
                id: BACKGROUND_REGION,
                material: DEFAULT_MATERIAL,
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
}
