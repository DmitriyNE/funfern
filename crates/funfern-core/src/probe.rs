use crate::{
    BoundaryLabel, Point2, QuadraticWaveOperator, RegionId, Scene, TriMesh,
    enriched_quadratic_basis, enriched_quadratic_basis_gradients, point_segment_distance,
};

const BOUNDARY_TOLERANCE: f64 = 1.0e-9;

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
pub struct PointProbeSample {
    pub displacement: f64,
    pub velocity: f64,
    pub energy_density: f64,
}

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
            energy_density,
        })
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
        assert!((sample.energy_density - (0.5 * 2.0 * 16.0 + 0.5 * 3.0 * 5.0)).abs() < 1.0e-11);
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
