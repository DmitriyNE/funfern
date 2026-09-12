use std::{collections::BTreeMap, sync::Arc};

use crate::{Point2, QuadraticWaveOperator, RegionId, Scene, TimeSignal, TriMesh, VolumeSource};

const MASS_WEIGHTS: [f64; 7] = [
    1.0 / 20.0,
    1.0 / 20.0,
    1.0 / 20.0,
    2.0 / 15.0,
    2.0 / 15.0,
    2.0 / 15.0,
    9.0 / 20.0,
];
pub const NO_VOLUME_SOURCE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumeSourceNode {
    pub channels: [u32; 2],
    pub weights: [f64; 2],
}

impl Default for VolumeSourceNode {
    fn default() -> Self {
        Self {
            channels: [NO_VOLUME_SOURCE; 2],
            weights: [0.0; 2],
        }
    }
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
                node.channels
                    .into_iter()
                    .zip(node.weights)
                    .filter_map(|(channel, weight)| {
                        self.signals
                            .get(channel as usize)
                            .map(|signal| weight * signal.value(time))
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
    TooManySourcesAtNode {
        point: Point2,
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
            Self::TooManySourcesAtNode { point } => write!(
                formatter,
                "more than two volume-source regions meet at ({:.4}, {:.4})",
                point.x, point.y
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
    scene: Scene,
    sources: Vec<VolumeSource>,
    channels: BTreeMap<RegionId, usize>,
    accumulated: Vec<Vec<(usize, f64)>>,
    nodes: Vec<VolumeSourceNode>,
    phase: Phase,
    cursor: usize,
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
        let channels = sources
            .iter()
            .enumerate()
            .map(|(index, source)| (source.region, index))
            .collect();
        let count = operator.degrees_of_freedom();
        Ok(Self {
            mesh,
            operator,
            scene,
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
            .scene
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
                .scene
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
        if self.accumulated[index].len() > 2 {
            return Err(VolumeSourceError::TooManySourcesAtNode {
                point: self.operator.node_points()[index],
            });
        }
        let mass = self.operator.lumped_mass()[index];
        for (slot, &(channel, contribution)) in self.accumulated[index].iter().enumerate() {
            let weight = contribution / mass;
            if !weight.is_finite() {
                return Err(VolumeSourceError::Evaluation {
                    region: self.sources[channel].region,
                    point: self.operator.node_points()[index],
                    reason: "normalized value cannot be represented".into(),
                });
            }
            self.nodes[index].channels[slot] = channel as u32;
            self.nodes[index].weights[slot] = weight;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BACKGROUND_REGION, MeshQuality, MeshTriangle, MeshVertex, ScalarField, WaveCoefficients,
    };

    fn triangle_mesh() -> Arc<TriMesh> {
        Arc::new(TriMesh {
            geometry_revision: 4,
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
}
