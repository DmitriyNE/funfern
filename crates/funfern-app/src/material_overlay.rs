use funfern_core::{MaterialCoordinates, QuadraticWaveOperator, RegionId, Scene, TriMesh};
use std::collections::BTreeMap;
use std::sync::Arc;

const SUBTRIANGLES: [[usize; 3]; 6] = [
    [0, 3, 6],
    [3, 1, 6],
    [1, 4, 6],
    [4, 2, 6],
    [2, 5, 6],
    [5, 0, 6],
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialOverlay {
    Off,
    Regions,
    Property(MaterialProperty),
}

impl MaterialOverlay {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Regions => "Material regions",
            Self::Property(property) => property.label(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialProperty {
    Density,
    Stiffness,
    Damping,
    WaveSpeed,
    Impedance,
}

impl MaterialProperty {
    pub const ALL: [Self; 5] = [
        Self::Density,
        Self::Stiffness,
        Self::Damping,
        Self::WaveSpeed,
        Self::Impedance,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Density => "Density",
            Self::Stiffness => "Stiffness",
            Self::Damping => "Damping",
            Self::WaveSpeed => "Wave speed",
            Self::Impedance => "Impedance",
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::Density => 0,
            Self::Stiffness => 1,
            Self::Damping => 2,
            Self::WaveSpeed => 3,
            Self::Impedance => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayRange {
    pub minimum: f64,
    pub maximum: f64,
}

impl OverlayRange {
    pub fn normalized(self, value: f64, logarithmic: bool) -> Option<f32> {
        let (minimum, maximum, value) = if logarithmic {
            if self.minimum <= 0.0 || self.maximum <= 0.0 {
                return None;
            }
            (
                self.minimum.ln(),
                self.maximum.ln(),
                value.max(self.minimum).ln(),
            )
        } else {
            (self.minimum, self.maximum, value)
        };
        let span = maximum - minimum;
        if !span.is_finite() || span <= 0.0 {
            return Some(0.5);
        }
        Some(((value - minimum) / span).clamp(0.0, 1.0) as f32)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OverlayKey {
    pub mesh_revision: u64,
    pub scene: Scene,
}

#[derive(Clone, Debug)]
pub struct OverlaySample {
    pub point: funfern_core::Point2,
    pub region: RegionId,
    pub material_name: String,
    pub coordinates: MaterialCoordinates,
    values: [Option<f64>; 5],
    errors: [Option<String>; 5],
}

impl OverlaySample {
    pub fn value(&self, property: MaterialProperty) -> Result<f64, &str> {
        let index = property.index();
        self.values[index].ok_or_else(|| self.errors[index].as_deref().unwrap_or("invalid value"))
    }
}

#[derive(Clone, Debug)]
pub struct MaterialOverlaySnapshot {
    pub key: OverlayKey,
    pub samples: Vec<OverlaySample>,
    pub triangles: Vec<[u32; 3]>,
    linear_ranges: [Option<OverlayRange>; 5],
    log_ranges: [Option<OverlayRange>; 5],
}

impl MaterialOverlaySnapshot {
    pub fn range(&self, property: MaterialProperty, logarithmic: bool) -> Option<OverlayRange> {
        if logarithmic {
            self.log_ranges[property.index()]
        } else {
            self.linear_ranges[property.index()]
        }
    }

    pub fn invalid_count(&self, property: MaterialProperty) -> usize {
        self.samples
            .iter()
            .filter(|sample| sample.value(property).is_err())
            .count()
    }
}

pub struct MaterialOverlayJob {
    key: OverlayKey,
    mesh: Arc<TriMesh>,
    operator: Arc<QuadraticWaveOperator>,
    vertices: BTreeMap<(u32, RegionId), u32>,
    topology_cursor: usize,
    requests: Vec<(funfern_core::Point2, RegionId)>,
    triangles: Vec<[u32; 3]>,
    samples: Vec<OverlaySample>,
    cursor: usize,
}

impl MaterialOverlayJob {
    pub fn new(
        mesh: Arc<TriMesh>,
        operator: Arc<QuadraticWaveOperator>,
        scene: Scene,
    ) -> Result<Self, String> {
        if mesh.mesh_revision != operator.mesh_revision()
            || mesh.triangles.len() != operator.element_nodes().len()
        {
            return Err("Material overlay mesh/operator mismatch".into());
        }
        Ok(Self {
            key: OverlayKey {
                mesh_revision: mesh.mesh_revision,
                scene,
            },
            mesh,
            operator,
            vertices: BTreeMap::new(),
            topology_cursor: 0,
            samples: vec![],
            requests: vec![],
            triangles: vec![],
            cursor: 0,
        })
    }

    pub fn key(&self) -> &OverlayKey {
        &self.key
    }
    pub fn progress(&self) -> (usize, usize) {
        (
            self.topology_cursor + self.cursor,
            self.mesh.triangles.len() + self.requests.len(),
        )
    }

    pub fn advance(&mut self, budget: usize) -> Option<MaterialOverlaySnapshot> {
        let topology_end = (self.topology_cursor + budget).min(self.mesh.triangles.len());
        for index in self.topology_cursor..topology_end {
            let triangle = self.mesh.triangles[index];
            let nodes = self.operator.element_nodes()[index];
            let mut local = [0_u32; 7];
            for (slot, node) in nodes.iter().enumerate() {
                let key = (*node, triangle.region);
                local[slot] = *self.vertices.entry(key).or_insert_with(|| {
                    let index = self.requests.len() as u32;
                    self.requests
                        .push((self.operator.node_points()[*node as usize], triangle.region));
                    index
                });
            }
            self.triangles
                .extend(SUBTRIANGLES.map(|triangle| triangle.map(|slot| local[slot])));
        }
        let topology_work = topology_end - self.topology_cursor;
        self.topology_cursor = topology_end;
        if self.topology_cursor != self.mesh.triangles.len() {
            return None;
        }
        if self.samples.capacity() < self.requests.len() {
            self.samples
                .reserve(self.requests.len() - self.samples.len());
        }
        let remaining = budget.saturating_sub(topology_work);
        let end = (self.cursor + remaining).min(self.requests.len());
        for &(point, region_id) in &self.requests[self.cursor..end] {
            self.samples.push(sample(&self.key.scene, region_id, point));
        }
        self.cursor = end;
        if self.cursor != self.requests.len() {
            return None;
        }
        let mut linear_ranges = [None; 5];
        let mut log_ranges = [None; 5];
        for property in MaterialProperty::ALL {
            let values = self.samples.iter().filter_map(|sample| {
                sample
                    .value(property)
                    .ok()
                    .map(|value| (sample.region, value))
            });
            linear_ranges[property.index()] = region_aware_robust_range(values.clone());
            log_ranges[property.index()] =
                region_aware_robust_range(values.filter(|(_, value)| *value > 0.0));
        }
        Some(MaterialOverlaySnapshot {
            key: self.key.clone(),
            samples: std::mem::take(&mut self.samples),
            triangles: std::mem::take(&mut self.triangles),
            linear_ranges,
            log_ranges,
        })
    }
}

pub fn sample(scene: &Scene, region_id: RegionId, point: funfern_core::Point2) -> OverlaySample {
    let fallback_coordinates = MaterialCoordinates {
        x: point.x,
        y: point.y,
        r: point.norm(),
        theta: point.y.atan2(point.x),
    };
    let Some(region) = scene.region(region_id) else {
        return failed_sample(point, region_id, "Missing region", fallback_coordinates);
    };
    let coordinates = region.frame.coordinates(point);
    let Some(material) = scene.material(region.material) else {
        return failed_sample(point, region_id, "Missing material", coordinates);
    };
    let mut values = [None; 5];
    let mut errors: [Option<String>; 5] = std::array::from_fn(|_| None);
    let fields = [
        &material.mass_density,
        &material.stiffness,
        &material.damping,
    ];
    for (index, field) in fields.into_iter().enumerate() {
        match field.evaluate(coordinates, &material.parameters) {
            Ok(value)
                if value.is_finite()
                    && (index == 2 && value >= 0.0 || index != 2 && value > 0.0) =>
            {
                values[index] = Some(value)
            }
            Ok(_) => {
                errors[index] = Some(
                    if index == 2 {
                        "must be non-negative"
                    } else {
                        "must be positive"
                    }
                    .into(),
                )
            }
            Err(error) => errors[index] = Some(error.to_string()),
        }
    }
    match (values[0], values[1]) {
        (Some(density), Some(stiffness)) => {
            values[3] = Some((stiffness / density).sqrt());
            values[4] = Some((stiffness * density).sqrt());
        }
        _ => {
            let error = errors[0]
                .as_deref()
                .or(errors[1].as_deref())
                .unwrap_or("invalid coefficient")
                .to_owned();
            errors[3] = Some(error.clone());
            errors[4] = Some(error);
        }
    }
    OverlaySample {
        point,
        region: region_id,
        material_name: material.name.clone(),
        coordinates,
        values,
        errors,
    }
}

fn failed_sample(
    point: funfern_core::Point2,
    region: RegionId,
    message: &str,
    coordinates: MaterialCoordinates,
) -> OverlaySample {
    OverlaySample {
        point,
        region,
        material_name: "Missing".into(),
        coordinates,
        values: [None; 5],
        errors: std::array::from_fn(|_| Some(message.into())),
    }
}

fn robust_range(values: &mut Vec<f64>) -> Option<OverlayRange> {
    values.retain(|value| value.is_finite());
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let (minimum, maximum) = if values.len() < 50 {
        (values[0], *values.last().unwrap())
    } else {
        (
            values[(values.len() - 1) * 2 / 100],
            values[(values.len() - 1) * 98 / 100],
        )
    };
    if minimum < maximum {
        Some(OverlayRange { minimum, maximum })
    } else {
        let padding = minimum.abs().max(1.0) * 1.0e-6;
        Some(OverlayRange {
            minimum: minimum - padding,
            maximum: maximum + padding,
        })
    }
}

fn region_aware_robust_range(
    values: impl IntoIterator<Item = (RegionId, f64)>,
) -> Option<OverlayRange> {
    let mut values_by_region = BTreeMap::<RegionId, Vec<f64>>::new();
    for (region, value) in values {
        if value.is_finite() {
            values_by_region.entry(region).or_default().push(value);
        }
    }

    values_by_region
        .values_mut()
        .filter_map(robust_range)
        .reduce(|combined, region| OverlayRange {
            minimum: combined.minimum.min(region.minimum),
            maximum: combined.maximum.max(region.maximum),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use funfern_core::{MeshQuality, MeshTriangle, MeshVertex, Point2, WaveCoefficients};

    #[test]
    fn derived_properties_and_robust_range_are_well_defined() {
        let sample = sample(
            &Scene::default(),
            funfern_core::BACKGROUND_REGION,
            funfern_core::Point2::new(0.2, 0.3),
        );
        assert_eq!(sample.value(MaterialProperty::WaveSpeed), Ok(1.0));
        assert_eq!(sample.value(MaterialProperty::Impedance), Ok(1.0));
        let mut values = (0..100).map(f64::from).collect::<Vec<_>>();
        values.push(1.0e12);
        let range = robust_range(&mut values).unwrap();
        assert!(range.maximum < 1000.0);
        assert_eq!(range.normalized(range.minimum, false), Some(0.0));
    }

    #[test]
    fn automatic_range_keeps_small_regions_visible() {
        let dominant_region = RegionId(1);
        let small_region = RegionId(2);
        let values = (0..1000)
            .map(|index| (dominant_region, 1.0 + f64::from(index) * 1.0e-4))
            .chain((0..8).map(|_| (small_region, 10.0)))
            .collect::<Vec<_>>();

        let mut globally_weighted = values.iter().map(|(_, value)| *value).collect();
        let old_range = robust_range(&mut globally_weighted).unwrap();
        let range = region_aware_robust_range(values).unwrap();

        assert!(old_range.maximum < 2.0);
        assert!(range.minimum <= 1.01);
        assert!(range.maximum >= 10.0);
    }

    #[test]
    fn topology_duplicates_every_shared_trace_node_by_region() {
        let mesh = Arc::new(TriMesh {
            geometry_revision: 1,
            mesh_revision: 2,
            vertices: vec![
                MeshVertex {
                    point: Point2::new(-1.0, -1.0),
                    boundary: None,
                },
                MeshVertex {
                    point: Point2::new(1.0, -1.0),
                    boundary: None,
                },
                MeshVertex {
                    point: Point2::new(1.0, 1.0),
                    boundary: None,
                },
                MeshVertex {
                    point: Point2::new(-1.0, 1.0),
                    boundary: None,
                },
            ],
            triangles: vec![
                MeshTriangle {
                    vertices: [0, 1, 2],
                    region: RegionId(1),
                },
                MeshTriangle {
                    vertices: [0, 2, 3],
                    region: RegionId(2),
                },
            ],
            boundary_edges: vec![],
            quality: MeshQuality {
                minimum_angle_degrees: 45.0,
                maximum_edge_length: 2.0_f64.sqrt() * 2.0,
            },
        });
        let mut operator_mesh = (*mesh).clone();
        operator_mesh.triangles[1].region = RegionId(1);
        let operator = Arc::new(
            QuadraticWaveOperator::assemble(&operator_mesh, WaveCoefficients::default()).unwrap(),
        );
        let mut job = MaterialOverlayJob::new(mesh, operator, Scene::default()).unwrap();
        assert!(job.advance(1).is_none());
        let snapshot = loop {
            if let Some(snapshot) = job.advance(1) {
                break snapshot;
            }
        };
        assert_eq!(snapshot.samples.len(), 14);
        assert_eq!(snapshot.triangles.len(), 12);
        assert_eq!(
            snapshot
                .samples
                .iter()
                .filter(|sample| sample.region == RegionId(1))
                .count(),
            7
        );
        assert_eq!(
            snapshot
                .samples
                .iter()
                .filter(|sample| sample.region == RegionId(2))
                .count(),
            7
        );
    }
}
