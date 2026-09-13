use std::{collections::BTreeMap, sync::Arc};

use crate::{
    EvaluatedMaterial, MAX_VOLUME_SOURCES, Material, MaterialError, PhysicsModel, Point2,
    QuadraticWaveOperator, Region, RegionId, Scene, TimeSignal, TopologyMeshPlan,
    TopologyWaveModel, TriMesh, VolumeSource,
};

const MASS_WEIGHTS: [f64; 7] = [
    1.0 / 20.0,
    1.0 / 20.0,
    1.0 / 20.0,
    2.0 / 15.0,
    2.0 / 15.0,
    2.0 / 15.0,
    9.0 / 20.0,
];
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumeSourceContribution {
    pub channel: u32,
    pub weight: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VolumeSourceNode {
    pub contributions: Vec<VolumeSourceContribution>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledVolumeSources {
    pub signals: Vec<TimeSignal>,
    pub nodes: Vec<VolumeSourceNode>,
}

impl CompiledVolumeSources {
    pub fn empty(degrees_of_freedom: usize) -> Self {
        Self {
            signals: vec![],
            nodes: vec![VolumeSourceNode::default(); degrees_of_freedom],
        }
    }

    pub fn acceleration(&self, time: f64) -> Vec<f64> {
        self.nodes
            .iter()
            .map(|node| {
                node.contributions
                    .iter()
                    .filter_map(|contribution| {
                        self.signals
                            .get(contribution.channel as usize)
                            .map(|signal| contribution.weight * signal.value(time))
                    })
                    .sum()
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum VolumeSourceError {
    InvalidInputs,
    Evaluation {
        region: RegionId,
        point: Point2,
        reason: String,
    },
}

impl std::fmt::Display for VolumeSourceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInputs => write!(
                formatter,
                "volume-source mesh, operator, or scene is invalid"
            ),
            Self::Evaluation {
                region,
                point,
                reason,
            } => write!(
                formatter,
                "volume source in region {} failed at ({:.4}, {:.4}): {reason}",
                region.0, point.x, point.y
            ),
        }
    }
}

impl std::error::Error for VolumeSourceError {}

enum Phase {
    Triangles,
    Nodes,
    Done,
}

pub struct VolumeSourceCompileJob {
    mesh: Arc<TriMesh>,
    operator: Arc<QuadraticWaveOperator>,
    medium: VolumeSourceMedium,
    sources: Vec<VolumeSource>,
    channels: BTreeMap<RegionId, usize>,
    accumulated: Vec<Vec<(usize, f64)>>,
    nodes: Vec<VolumeSourceNode>,
    phase: Phase,
    cursor: usize,
}

#[derive(Clone, Debug)]
struct VolumeSourceMedium {
    physics: PhysicsModel,
    materials: Vec<Material>,
    regions: Vec<Region>,
}

impl VolumeSourceMedium {
    fn from_scene(scene: &Scene) -> Self {
        Self {
            physics: scene.physics,
            materials: scene.materials.clone(),
            regions: scene.regions.clone(),
        }
    }

    fn from_topology(model: TopologyWaveModel<'_>) -> Self {
        Self {
            physics: model.physics,
            materials: model.materials.to_vec(),
            regions: model.regions.to_vec(),
        }
    }

    fn region(&self, id: RegionId) -> Option<&Region> {
        self.regions.iter().find(|region| region.id == id)
    }

    fn material_at(
        &self,
        region: RegionId,
        point: Point2,
    ) -> Result<EvaluatedMaterial, MaterialError> {
        crate::wave::evaluate_material_library_at(
            self.physics,
            &self.materials,
            &self.regions,
            region,
            point,
        )
    }
}

impl VolumeSourceCompileJob {
    pub fn new(
        mesh: Arc<TriMesh>,
        operator: Arc<QuadraticWaveOperator>,
        scene: Scene,
    ) -> Result<Self, VolumeSourceError> {
        if !scene.structure_valid()
            || mesh.mesh_revision != operator.mesh_revision()
            || mesh.geometry_revision != operator.geometry_revision()
            || mesh.triangles.len() != operator.element_nodes().len()
        {
            return Err(VolumeSourceError::InvalidInputs);
        }
        let sources = scene
            .volume_sources
            .iter()
            .filter(|source| source.enabled)
            .cloned()
            .collect::<Vec<_>>();
        Self::from_parts(
            mesh,
            operator,
            VolumeSourceMedium::from_scene(&scene),
            sources,
        )
    }

    /// Starts source compilation from the unified topology contract. The job
    /// owns a compact material/source snapshot so it remains deterministic
    /// while the editor advances it over multiple frames.
    pub fn new_topology(
        mesh: Arc<TriMesh>,
        operator: Arc<QuadraticWaveOperator>,
        plan: &TopologyMeshPlan,
        model: TopologyWaveModel<'_>,
        sources: &[VolumeSource],
    ) -> Result<Self, VolumeSourceError> {
        let active_regions = plan
            .domains
            .iter()
            .map(|domain| domain.region)
            .collect::<std::collections::BTreeSet<_>>();
        let sources_valid = sources.len() <= MAX_VOLUME_SOURCES
            && sources.iter().enumerate().all(|(index, source)| {
                source.valid()
                    && model.region(source.region).is_some()
                    && active_regions.contains(&source.region)
                    && !sources[..index]
                        .iter()
                        .any(|previous| previous.region == source.region)
            });
        let mesh_regions_valid = mesh
            .triangles
            .iter()
            .all(|triangle| active_regions.contains(&triangle.region));
        if !model.valid_for(plan) || !sources_valid || !mesh_regions_valid {
            return Err(VolumeSourceError::InvalidInputs);
        }
        Self::from_parts(
            mesh,
            operator,
            VolumeSourceMedium::from_topology(model),
            sources
                .iter()
                .filter(|source| source.enabled)
                .cloned()
                .collect(),
        )
    }

    fn from_parts(
        mesh: Arc<TriMesh>,
        operator: Arc<QuadraticWaveOperator>,
        medium: VolumeSourceMedium,
        sources: Vec<VolumeSource>,
    ) -> Result<Self, VolumeSourceError> {
        if mesh.mesh_revision != operator.mesh_revision()
            || mesh.geometry_revision != operator.geometry_revision()
            || mesh.triangles.len() != operator.element_nodes().len()
        {
            return Err(VolumeSourceError::InvalidInputs);
        }
        let channels = sources
            .iter()
            .enumerate()
            .map(|(index, source)| (source.region, index))
            .collect();
        let count = operator.degrees_of_freedom();
        Ok(Self {
            mesh,
            operator,
            medium,
            sources,
            channels,
            accumulated: vec![vec![]; count],
            nodes: vec![VolumeSourceNode::default(); count],
            phase: Phase::Triangles,
            cursor: 0,
        })
    }

    pub fn progress(&self) -> (usize, usize) {
        let triangles = self.mesh.triangles.len();
        match self.phase {
            Phase::Triangles => (self.cursor, triangles + self.nodes.len()),
            Phase::Nodes => (triangles + self.cursor, triangles + self.nodes.len()),
            Phase::Done => (triangles + self.nodes.len(), triangles + self.nodes.len()),
        }
    }

    pub fn advance(
        &mut self,
        budget: usize,
    ) -> Option<Result<CompiledVolumeSources, VolumeSourceError>> {
        for _ in 0..budget {
            match self.phase {
                Phase::Triangles => {
                    if self.cursor == self.mesh.triangles.len() {
                        self.phase = Phase::Nodes;
                        self.cursor = 0;
                        continue;
                    }
                    if let Err(error) = self.compile_triangle(self.cursor) {
                        self.phase = Phase::Done;
                        return Some(Err(error));
                    }
                    self.cursor += 1;
                }
                Phase::Nodes => {
                    if self.cursor == self.nodes.len() {
                        self.phase = Phase::Done;
                        return Some(Ok(self.result()));
                    }
                    if let Err(error) = self.compile_node(self.cursor) {
                        self.phase = Phase::Done;
                        return Some(Err(error));
                    }
                    self.cursor += 1;
                }
                Phase::Done => return None,
            }
        }
        None
    }

    fn compile_triangle(&mut self, index: usize) -> Result<(), VolumeSourceError> {
        let triangle = self.mesh.triangles[index];
        let Some(&channel) = self.channels.get(&triangle.region) else {
            return Ok(());
        };
        let source = &self.sources[channel];
        let region = self
            .medium
            .region(triangle.region)
            .ok_or(VolumeSourceError::InvalidInputs)?;
        let points = triangle
            .vertices
            .map(|vertex| self.mesh.vertices[vertex].point);
        let area = 0.5 * (points[1] - points[0]).cross(points[2] - points[0]);
        if !area.is_finite() || area <= 0.0 {
            return Err(VolumeSourceError::InvalidInputs);
        }
        for (local, &node) in self.operator.element_nodes()[index].iter().enumerate() {
            let node = node as usize;
            let point = self.operator.node_points()[node];
            let density = self
                .medium
                .material_at(triangle.region, point)
                .map_err(|error| VolumeSourceError::Evaluation {
                    region: triangle.region,
                    point,
                    reason: error.to_string(),
                })?
                .mass_density;
            let profile = source.evaluate(region.frame, point).map_err(|error| {
                VolumeSourceError::Evaluation {
                    region: triangle.region,
                    point,
                    reason: error.to_string(),
                }
            })?;
            let contribution = density * area * MASS_WEIGHTS[local] * profile;
            if !contribution.is_finite() {
                return Err(VolumeSourceError::Evaluation {
                    region: triangle.region,
                    point,
                    reason: "value cannot be represented".into(),
                });
            }
            if let Some((_, value)) = self.accumulated[node]
                .iter_mut()
                .find(|(candidate, _)| *candidate == channel)
            {
                *value += contribution;
            } else {
                self.accumulated[node].push((channel, contribution));
            }
        }
        Ok(())
    }

    fn compile_node(&mut self, index: usize) -> Result<(), VolumeSourceError> {
        let mass = self.operator.lumped_mass()[index];
        for &(channel, contribution) in &self.accumulated[index] {
            let weight = contribution / mass;
            if !weight.is_finite() {
                return Err(VolumeSourceError::Evaluation {
                    region: self.sources[channel].region,
                    point: self.operator.node_points()[index],
                    reason: "normalized value cannot be represented".into(),
                });
            }
            self.nodes[index]
                .contributions
                .push(VolumeSourceContribution {
                    channel: channel as u32,
                    weight,
                });
        }
        Ok(())
    }

    fn result(&self) -> CompiledVolumeSources {
        CompiledVolumeSources {
            signals: self.sources.iter().map(|source| source.signal).collect(),
            nodes: self.nodes.clone(),
        }
    }
}

pub fn compile_volume_sources(
    mesh: Arc<TriMesh>,
    operator: Arc<QuadraticWaveOperator>,
    scene: Scene,
) -> Result<CompiledVolumeSources, VolumeSourceError> {
    let mut job = VolumeSourceCompileJob::new(mesh, operator, scene)?;
    loop {
        if let Some(result) = job.advance(usize::MAX) {
            return result;
        }
    }
}

pub fn compile_topology_volume_sources(
    mesh: Arc<TriMesh>,
    operator: Arc<QuadraticWaveOperator>,
    plan: &TopologyMeshPlan,
    model: TopologyWaveModel<'_>,
    sources: &[VolumeSource],
) -> Result<CompiledVolumeSources, VolumeSourceError> {
    let mut job = VolumeSourceCompileJob::new_topology(mesh, operator, plan, model, sources)?;
    loop {
        if let Some(result) = job.advance(usize::MAX) {
            return result;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BACKGROUND_REGION, CurveId, CurveNode, CurveSpan, CurveSpanId, CurveSpline,
        FaceRegionAssignment, MaterialFrame, MeshQuality, MeshTriangle, MeshVertex, MeshingOptions,
        OpenCubicSpline, OuterSide, ScalarField, SpanBehavior, TopologyCurve, TopologyGeometry,
        TopologyVertex, TopologyVertexId, TopologyVertexLocation, WaveCoefficients,
        compile_topology, mesh_topology_plan,
    };

    fn triangle_mesh() -> Arc<TriMesh> {
        Arc::new(TriMesh {
            geometry_revision: 4,
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
            quality: MeshQuality {
                minimum_angle_degrees: 45.0,
                maximum_edge_length: 2.0_f64.sqrt(),
            },
        })
    }

    #[test]
    fn constant_profile_compiles_to_mass_normalized_acceleration() {
        let mesh = triangle_mesh();
        let operator =
            Arc::new(QuadraticWaveOperator::assemble(&mesh, WaveCoefficients::default()).unwrap());
        let mut scene = Scene::default();
        scene.volume_sources.push(VolumeSource {
            region: BACKGROUND_REGION,
            enabled: true,
            profile: ScalarField::constant(2.5),
            parameters: vec![],
            signal: TimeSignal::Harmonic {
                offset: 3.0,
                amplitude: 0.0,
                frequency_hz: 1.0,
                phase_radians: 0.0,
            },
        });
        let compiled = compile_volume_sources(mesh, operator, scene).unwrap();
        assert_eq!(compiled.signals.len(), 1);
        for value in compiled.acceleration(0.37) {
            assert!((value - 7.5).abs() < 1.0e-12, "{value}");
        }
    }

    #[test]
    fn compiler_is_resumable_and_disabled_sources_are_omitted() {
        let mesh = triangle_mesh();
        let operator =
            Arc::new(QuadraticWaveOperator::assemble(&mesh, WaveCoefficients::default()).unwrap());
        let mut scene = Scene::default();
        scene.volume_sources.push(VolumeSource {
            region: BACKGROUND_REGION,
            enabled: false,
            profile: ScalarField::formula("1 + x").unwrap(),
            parameters: vec![],
            signal: TimeSignal::ZERO,
        });
        let mut job = VolumeSourceCompileJob::new(mesh, operator, scene).unwrap();
        let mut slices = 0;
        let compiled = loop {
            slices += 1;
            if let Some(result) = job.advance(1) {
                break result.unwrap();
            }
        };
        assert!(slices > 1);
        assert!(compiled.signals.is_empty());
        assert!(compiled.acceleration(1.0).iter().all(|value| *value == 0.0));
    }

    fn topology_domain() -> (TopologyMeshPlan, Arc<TriMesh>, Scene) {
        let snapshot = compile_topology(&TopologyGeometry::default(), 19).unwrap();
        let plan = TopologyMeshPlan::new(
            &snapshot,
            &[FaceRegionAssignment {
                face: snapshot.faces[0].id,
                region: Some(BACKGROUND_REGION),
            }],
        )
        .unwrap();
        let mesh = Arc::new(
            mesh_topology_plan(
                &plan,
                23,
                MeshingOptions {
                    target_edge_length: 0.45,
                    minimum_angle_degrees: 8.0,
                    max_vertices: 10_000,
                    max_triangles: 20_000,
                    max_refinement_steps: 10_000,
                    ..MeshingOptions::default()
                },
            )
            .unwrap(),
        );
        (plan, mesh, Scene::default())
    }

    #[test]
    fn topology_compiler_uses_region_frame_and_remains_resumable() {
        let (plan, mesh, mut scene) = topology_domain();
        scene.regions[0].frame = MaterialFrame {
            origin: Point2::new(0.25, -0.1),
            ..MaterialFrame::world()
        };
        let source = VolumeSource {
            region: BACKGROUND_REGION,
            enabled: true,
            profile: ScalarField::formula("2 + x").unwrap(),
            parameters: vec![],
            signal: TimeSignal::harmonic(1.0, 0.0, 1.0, 0.0),
        };
        let model = TopologyWaveModel::from_scene(&scene);
        assert_eq!(
            model
                .material_at(BACKGROUND_REGION, Point2::new(0.6, -0.2))
                .unwrap(),
            scene
                .material_at(BACKGROUND_REGION, Point2::new(0.6, -0.2))
                .unwrap()
        );
        let operator =
            Arc::new(QuadraticWaveOperator::assemble_topology(&mesh, &plan, model).unwrap());
        let mut job = VolumeSourceCompileJob::new_topology(
            mesh.clone(),
            operator.clone(),
            &plan,
            model,
            std::slice::from_ref(&source),
        )
        .unwrap();
        assert!(job.advance(1).is_none());
        let compiled = loop {
            if let Some(result) = job.advance(3) {
                break result.unwrap();
            }
        };
        for (point, value) in operator
            .node_points()
            .iter()
            .zip(compiled.acceleration(0.37))
        {
            let expected = 2.0 + point.x - 0.25;
            assert!((value - expected).abs() < 1.0e-11, "{point:?}: {value}");
        }
    }

    #[test]
    fn topology_compiler_rejects_sources_outside_active_faces() {
        let (plan, mesh, mut scene) = topology_domain();
        let inactive = RegionId(2);
        scene.regions.push(Region {
            id: inactive,
            material: scene.materials[0].id,
            frame: MaterialFrame::world(),
        });
        let operator = Arc::new(
            QuadraticWaveOperator::assemble_topology(
                &mesh,
                &plan,
                TopologyWaveModel::from_scene(&scene),
            )
            .unwrap(),
        );
        let source = VolumeSource {
            region: inactive,
            enabled: true,
            profile: ScalarField::constant(1.0),
            parameters: vec![],
            signal: TimeSignal::ZERO,
        };
        assert!(matches!(
            VolumeSourceCompileJob::new_topology(
                mesh,
                operator,
                &plan,
                TopologyWaveModel::from_scene(&scene),
                &[source],
            ),
            Err(VolumeSourceError::InvalidInputs)
        ));
    }

    #[test]
    fn topology_compiler_supports_all_sources_at_a_four_face_junction() {
        let endpoint_ids = [
            TopologyVertexId(1),
            TopologyVertexId(2),
            TopologyVertexId(3),
            TopologyVertexId(4),
        ];
        let make_curve = |id, points: Vec<Point2>, endpoints: [TopologyVertexId; 2]| {
            let mut curve = TopologyCurve::new(
                CurveId(id),
                CurveSpline::Open(OpenCubicSpline::polyline(points).unwrap()),
                vec![CurveSpan {
                    id: CurveSpanId(id),
                    behavior: SpanBehavior::Transmitting,
                }],
            )
            .unwrap();
            curve.nodes = endpoints
                .map(|vertex| CurveNode {
                    vertex: Some(vertex),
                })
                .to_vec();
            curve
        };
        let snapshot = compile_topology(
            &TopologyGeometry {
                curves: vec![
                    make_curve(
                        1,
                        vec![Point2::new(-1.0, 0.0), Point2::new(1.0, 0.0)],
                        [endpoint_ids[0], endpoint_ids[1]],
                    ),
                    make_curve(
                        2,
                        vec![Point2::new(0.0, -1.0), Point2::new(0.0, 1.0)],
                        [endpoint_ids[2], endpoint_ids[3]],
                    ),
                ],
                vertices: vec![
                    TopologyVertex {
                        id: endpoint_ids[0],
                        location: TopologyVertexLocation::Outer {
                            side: OuterSide::Left,
                            fraction: 0.5,
                        },
                    },
                    TopologyVertex {
                        id: endpoint_ids[1],
                        location: TopologyVertexLocation::Outer {
                            side: OuterSide::Right,
                            fraction: 0.5,
                        },
                    },
                    TopologyVertex {
                        id: endpoint_ids[2],
                        location: TopologyVertexLocation::Outer {
                            side: OuterSide::Bottom,
                            fraction: 0.5,
                        },
                    },
                    TopologyVertex {
                        id: endpoint_ids[3],
                        location: TopologyVertexLocation::Outer {
                            side: OuterSide::Top,
                            fraction: 0.5,
                        },
                    },
                ],
                ..TopologyGeometry::default()
            },
            31,
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
        assert_eq!(assignments.len(), 4);
        let plan = TopologyMeshPlan::new(&snapshot, &assignments).unwrap();
        let mesh = Arc::new(
            mesh_topology_plan(
                &plan,
                32,
                MeshingOptions {
                    target_edge_length: 0.4,
                    minimum_angle_degrees: 8.0,
                    max_vertices: 20_000,
                    max_triangles: 40_000,
                    max_refinement_steps: 20_000,
                    ..MeshingOptions::default()
                },
            )
            .unwrap(),
        );
        let mut scene = Scene {
            regions: assignments
                .iter()
                .map(|assignment| Region {
                    id: assignment.region.unwrap(),
                    material: crate::DEFAULT_MATERIAL,
                    frame: MaterialFrame::world(),
                })
                .collect(),
            ..Scene::default()
        };
        scene.volume_sources = scene
            .regions
            .iter()
            .enumerate()
            .map(|(index, region)| VolumeSource {
                region: region.id,
                enabled: true,
                profile: ScalarField::constant(1.0),
                parameters: vec![],
                signal: TimeSignal::harmonic(index as f64 + 1.0, 0.0, 1.0, 0.0),
            })
            .collect();
        let model = TopologyWaveModel::from_scene(&scene);
        let operator =
            Arc::new(QuadraticWaveOperator::assemble_topology(&mesh, &plan, model).unwrap());
        let compiled = compile_topology_volume_sources(
            mesh.clone(),
            operator.clone(),
            &plan,
            model,
            &scene.volume_sources,
        )
        .unwrap();
        let junction = operator
            .node_points()
            .iter()
            .position(|point| point.norm() < 1.0e-12)
            .unwrap();
        assert_eq!(compiled.nodes[junction].contributions.len(), 4);
        assert!(
            compiled.nodes[junction]
                .contributions
                .iter()
                .all(|contribution| contribution.weight > 0.0)
        );
        let weight_sum = compiled.nodes[junction]
            .contributions
            .iter()
            .map(|contribution| contribution.weight)
            .sum::<f64>();
        assert!((weight_sum - 1.0).abs() < 1.0e-11, "{weight_sum}");

        let single = compile_topology_volume_sources(
            mesh.clone(),
            operator.clone(),
            &plan,
            model,
            &scene.volume_sources[..1],
        )
        .unwrap();
        let target = scene.volume_sources[0].region;
        let mut memberships =
            vec![std::collections::BTreeSet::new(); operator.degrees_of_freedom()];
        for (triangle, nodes) in mesh.triangles.iter().zip(operator.element_nodes()) {
            for &node in nodes {
                memberships[node as usize].insert(triangle.region);
            }
        }
        for (membership, node) in memberships.iter().zip(&single.nodes) {
            if !membership.contains(&target) {
                assert!(node.contributions.is_empty());
            }
        }
    }
}
