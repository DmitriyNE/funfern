//! CPU reference maps for integrated primary flux and independent vector flux.
//!
//! These are event/handoff oracles, not a live CPU readback service. Geometry
//! prepares every stencil; later GPU stages apply the same maps to the latest
//! accepted state.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use crate::{
    CanonicalOutgoingBoundary, CanonicalOutgoingPhysicalMemory, CanonicalTemporalWaveOperator,
    CanonicalThinGapMemory, CanonicalWaveOperator, Point2, QuadraticTransferMap,
    QuadraticTransferSample, ThinGapSample, ThinGapTraceKey, TriMesh, WaveError,
};

const QUADRATURE_SAMPLES: usize = 6;
const CORRECTION_LIMIT: f64 = 0.05;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanonicalTransferReport {
    pub maximum_correction_ratio: f64,
    pub exposed_values: usize,
    pub extended_values: usize,
}

/// Backend-neutral scalar-transfer row. Coefficients already include the
/// source and target geometric supports, so applying a row directly to
/// integrated source `Q` produces integrated target `Q`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalPrimaryTransferTarget {
    pub source_nodes: [u32; 7],
    pub coefficients: [f64; 7],
    pub source_count: u8,
    pub target_component: u32,
    pub target_support: f64,
    pub exact: bool,
}

/// Support-aware scalar map. Desired component totals are supplied by the
/// prepared topology transaction because split/merge retained shares are
/// semantic data, not something point interpolation can infer.
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalPrimaryTransferMap {
    identity: bool,
    target_nodes: usize,
    samples: Vec<Option<QuadraticTransferSample>>,
    source_support: Vec<f64>,
    target_support: Vec<f64>,
    target_components: Vec<u32>,
    component_count: usize,
    exact: Vec<bool>,
    extensions: Vec<Option<[u32; 7]>>,
}

impl CanonicalPrimaryTransferMap {
    pub fn prepare(
        interpolation: &QuadraticTransferMap,
        source: &CanonicalWaveOperator,
        target: &CanonicalWaveOperator,
    ) -> Result<Self, WaveError> {
        if interpolation.source_dofs() != source.degrees_of_freedom()
            || interpolation.samples().len() != target.degrees_of_freedom()
        {
            return Err(WaveError::InvalidMesh(
                "the scalar transfer map does not match the canonical generations",
            ));
        }
        let identity = interpolation.source_dofs() == target.degrees_of_freedom()
            && interpolation
                .samples()
                .iter()
                .enumerate()
                .all(|(target_node, sample)| {
                    sample.as_ref().is_some_and(|sample| {
                        sample.nodes[0] as usize == target_node
                            && sample.weights[0] == 1.0
                            && sample.weights[1..].iter().all(|weight| *weight == 0.0)
                            && source.geometric_support()[target_node].to_bits()
                                == target.geometric_support()[target_node].to_bits()
                    })
                });
        if identity {
            return Ok(Self {
                identity: true,
                target_nodes: target.degrees_of_freedom(),
                samples: Vec::new(),
                source_support: source.geometric_support().to_vec(),
                target_support: Vec::new(),
                target_components: target.component_labels().to_vec(),
                component_count: target.component_count(),
                exact: Vec::new(),
                extensions: Vec::new(),
            });
        }
        let exact = interpolation
            .samples()
            .iter()
            .enumerate()
            .map(|(target_node, sample)| {
                sample.as_ref().is_some_and(|sample| {
                    let source_node = sample.nodes[0] as usize;
                    sample.weights[0] == 1.0
                        && sample.weights[1..].iter().all(|weight| *weight == 0.0)
                        && source
                            .geometric_support()
                            .get(source_node)
                            .is_some_and(|support| {
                                support.to_bits()
                                    == target.geometric_support()[target_node].to_bits()
                            })
                })
            })
            .collect();
        Ok(Self {
            identity: false,
            target_nodes: target.degrees_of_freedom(),
            samples: interpolation.samples().to_vec(),
            source_support: source.geometric_support().to_vec(),
            target_support: target.geometric_support().to_vec(),
            target_components: target.component_labels().to_vec(),
            component_count: target.component_count(),
            exact,
            extensions: vec![None; target.degrees_of_freedom()],
        })
    }

    /// Adds the agreed bounded constant-preserving initialization for target
    /// support outside the old domain but connected within two element rings.
    /// The already prepared quadratic map supplies all side restrictions.
    pub fn prepare_with_meshes(
        interpolation: &QuadraticTransferMap,
        source_mesh: &TriMesh,
        source: &CanonicalWaveOperator,
        target_mesh: &TriMesh,
        target: &CanonicalWaveOperator,
    ) -> Result<Self, WaveError> {
        if source_mesh.triangles.len() != source.element_nodes().len()
            || target_mesh.triangles.len() != target.element_nodes().len()
        {
            return Err(WaveError::InvalidMesh(
                "the scalar extension meshes do not match their canonical operators",
            ));
        }
        let mut map = Self::prepare(interpolation, source, target)?;
        if map.identity {
            return Ok(map);
        }
        let mut distance = vec![usize::MAX; target_mesh.triangles.len()];
        for (element, nodes) in target.element_nodes().iter().enumerate() {
            if nodes
                .iter()
                .any(|node| interpolation.samples()[*node as usize].is_some())
            {
                distance[element] = 0;
            }
        }
        extend_element_distances(target_mesh, &mut distance, 2);
        for node in 0..map.samples.len() {
            if map.samples[node].is_some() {
                continue;
            }
            let target_elements = target
                .element_nodes()
                .iter()
                .enumerate()
                .filter(|(_, nodes)| nodes.contains(&(node as u32)))
                .map(|(element, _)| element)
                .collect::<Vec<_>>();
            if target_elements.is_empty()
                || target_elements.iter().all(|element| distance[*element] > 2)
            {
                continue;
            }
            let target_region = target_elements
                .iter()
                .min_by_key(|element| distance[**element])
                .map(|element| target_mesh.triangles[*element].region)
                .ok_or(WaveError::InvalidState)?;
            let point = target.node_points()[node];
            let donor = source_mesh
                .triangles
                .iter()
                .enumerate()
                .filter(|(_, triangle)| triangle.region == target_region)
                .min_by(|(_, left), (_, right)| {
                    let left = triangle_centroid(source_mesh, left.vertices) - point;
                    let right = triangle_centroid(source_mesh, right.vertices) - point;
                    left.dot(left).total_cmp(&right.dot(right))
                })
                .map(|(element, _)| source.element_nodes()[element]);
            map.extensions[node] = donor;
        }
        Ok(map)
    }

    pub fn exact_nodes(&self) -> usize {
        if self.identity {
            return self.target_nodes;
        }
        self.exact.iter().filter(|exact| **exact).count()
    }

    pub fn is_identity(&self) -> bool {
        self.identity
    }

    pub fn target_node_count(&self) -> usize {
        self.target_nodes
    }

    pub fn source_support(&self) -> &[f64] {
        &self.source_support
    }

    pub fn target_component_count(&self) -> usize {
        self.component_count
    }

    /// Exports the prepared density interpolation/extension without exposing
    /// the quadratic mesher's internal sample representation.
    pub fn targets(&self) -> Vec<CanonicalPrimaryTransferTarget> {
        if self.identity {
            return (0..self.target_nodes)
                .map(|target| CanonicalPrimaryTransferTarget {
                    source_nodes: [target as u32, 0, 0, 0, 0, 0, 0],
                    coefficients: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                    source_count: 7,
                    target_component: self.target_components[target],
                    target_support: self.source_support[target],
                    exact: true,
                })
                .collect();
        }
        self.samples
            .iter()
            .enumerate()
            .map(|(target, sample)| {
                let mut source_nodes = [0_u32; 7];
                let mut coefficients = [0.0; 7];
                let source_count = match sample {
                    Some(sample) => {
                        source_nodes = sample.nodes;
                        for (slot, (source, weight)) in
                            sample.nodes.iter().zip(sample.weights).enumerate()
                        {
                            coefficients[slot] = weight * self.target_support[target]
                                / self.source_support[*source as usize];
                        }
                        7
                    }
                    None => match self.extensions[target] {
                        Some(nodes) => {
                            source_nodes = nodes;
                            for (slot, source) in nodes.iter().enumerate() {
                                coefficients[slot] = self.target_support[target]
                                    / (nodes.len() as f64 * self.source_support[*source as usize]);
                            }
                            7
                        }
                        None => 0,
                    },
                };
                CanonicalPrimaryTransferTarget {
                    source_nodes,
                    coefficients,
                    source_count,
                    target_component: self.target_components[target],
                    target_support: self.target_support[target],
                    exact: self.exact[target],
                }
            })
            .collect()
    }

    pub fn transfer(
        &self,
        source_flux: &[f64],
        desired_component_totals: &[Option<f64>],
        prescribed_target: &[bool],
    ) -> Result<(Vec<f64>, CanonicalTransferReport), WaveError> {
        if source_flux.len() != self.source_support.len()
            || desired_component_totals.len() != self.component_count
            || prescribed_target.len() != self.target_nodes
            || source_flux.iter().any(|value| !value.is_finite())
        {
            return Err(WaveError::InvalidState);
        }
        if self.identity {
            return Ok((source_flux.to_vec(), CanonicalTransferReport::default()));
        }
        let density = source_flux
            .iter()
            .zip(&self.source_support)
            .map(|(flux, support)| flux / support)
            .collect::<Vec<_>>();
        let mut target = Vec::with_capacity(self.samples.len());
        let mut exposed = 0;
        let mut extended = 0;
        for (node, sample) in self.samples.iter().enumerate() {
            let value = match sample {
                Some(sample) => {
                    sample
                        .nodes
                        .iter()
                        .zip(sample.weights)
                        .map(|(source, weight)| density[*source as usize] * weight)
                        .sum::<f64>()
                        * self.target_support[node]
                }
                None => match self.extensions[node] {
                    Some(nodes) => {
                        extended += 1;
                        nodes
                            .iter()
                            .map(|source| density[*source as usize] / nodes.len() as f64)
                            .sum::<f64>()
                            * self.target_support[node]
                    }
                    None => {
                        exposed += 1;
                        0.0
                    }
                },
            };
            target.push(value);
        }
        let rms_density = (density.iter().map(|value| value * value).sum::<f64>()
            / density.len().max(1) as f64)
            .sqrt();
        let floor = rms_density.max(1.0e-12);
        let mut maximum_ratio = 0.0_f64;
        for (component, desired) in desired_component_totals.iter().enumerate() {
            let Some(desired) = desired else { continue };
            if !desired.is_finite() {
                return Err(WaveError::InvalidState);
            }
            let affected = (0..target.len())
                .filter(|node| {
                    self.target_components[*node] as usize == component
                        && !self.exact[*node]
                        && !prescribed_target[*node]
                })
                .collect::<Vec<_>>();
            let total = target
                .iter()
                .zip(&self.target_components)
                .filter(|(_, label)| **label as usize == component)
                .map(|(value, _)| value)
                .sum::<f64>();
            let delta = desired - total;
            let scale = affected
                .iter()
                .map(|node| target[*node].abs() + floor * self.target_support[*node])
                .sum::<f64>();
            if affected.is_empty() {
                if delta.abs() > 1.0e-11 * desired.abs().max(1.0) {
                    return Err(WaveError::InvalidState);
                }
                continue;
            }
            let ratio = delta.abs() / scale.max(1.0e-300);
            maximum_ratio = maximum_ratio.max(ratio);
            if ratio > CORRECTION_LIMIT {
                return Err(WaveError::InvalidState);
            }
            let support = affected
                .iter()
                .map(|node| self.target_support[*node])
                .sum::<f64>();
            for node in affected {
                target[node] += delta * self.target_support[node] / support;
            }
            let corrected = target
                .iter()
                .zip(&self.target_components)
                .filter(|(_, label)| **label as usize == component)
                .map(|(value, _)| value)
                .sum::<f64>();
            if (corrected - desired).abs() > 1.0e-11 * scale.max(desired.abs()).max(1.0) {
                return Err(WaveError::InvalidState);
            }
        }
        if target.iter().any(|value| !value.is_finite()) {
            return Err(WaveError::InvalidState);
        }
        Ok((
            target,
            CanonicalTransferReport {
                maximum_correction_ratio: maximum_ratio,
                exposed_values: exposed,
                extended_values: extended,
            },
        ))
    }
}

#[derive(Clone, Debug, PartialEq)]
enum VectorTarget {
    Reconstruct {
        source_element: u32,
        weights: [f64; 6],
    },
    ExtendConstant {
        source_element: u32,
    },
    Exposed,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanonicalVectorTransferTarget {
    pub source_samples: [u32; QUADRATURE_SAMPLES],
    pub weights: [f64; QUADRATURE_SAMPLES],
    pub source_count: u8,
    pub exact: bool,
}

/// Quadratic physical-coordinate reconstruction for the six independent
/// complementary samples in each old element.
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalVectorTransferMap {
    source_samples: usize,
    identity: bool,
    targets: Vec<VectorTarget>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CanonicalVectorTransferPhase {
    Validate,
    Bins(usize),
    Locate(usize),
    PrepareExtension,
    Extend(usize),
    Done,
}

impl CanonicalVectorTransferPhase {
    const fn label(self) -> &'static str {
        match self {
            Self::Validate => "Checking complementary transfer",
            Self::Bins(_) => "Indexing complementary transfer",
            Self::Locate(_) => "Locating complementary samples",
            Self::PrepareExtension => "Indexing complementary extension",
            Self::Extend(_) => "Extending complementary samples",
            Self::Done => "Finished",
        }
    }
}

/// Uniform source-element grid for complementary sample transfer. Canonical
/// samples are much more numerous than scalar nodes, so scanning every source
/// triangle for every target sample made an adaptive handoff quadratic in mesh
/// size. The grid is constructed cooperatively, one triangle per work unit,
/// and keeps the existing region-side restriction at lookup time.
struct CanonicalSourceBins {
    minimum: Point2,
    maximum: Point2,
    cell: Point2,
    dimension: usize,
    bins: Vec<Vec<u32>>,
}

impl CanonicalSourceBins {
    fn new(source: &TriMesh) -> Result<Self, WaveError> {
        let mut minimum = Point2::new(f64::INFINITY, f64::INFINITY);
        let mut maximum = Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
        for vertex in &source.vertices {
            minimum.x = minimum.x.min(vertex.point.x);
            minimum.y = minimum.y.min(vertex.point.y);
            maximum.x = maximum.x.max(vertex.point.x);
            maximum.y = maximum.y.max(vertex.point.y);
        }
        let extent = Point2::new(maximum.x - minimum.x, maximum.y - minimum.y);
        if !(extent.x > 0.0 && extent.y > 0.0) {
            return Err(WaveError::InvalidMesh(
                "the complementary transfer source has no extent",
            ));
        }
        let dimension = (source.triangles.len() as f64)
            .sqrt()
            .ceil()
            .clamp(8.0, 512.0) as usize;
        Ok(Self {
            minimum,
            maximum,
            cell: Point2::new(extent.x / dimension as f64, extent.y / dimension as f64),
            dimension,
            bins: vec![Vec::new(); dimension * dimension],
        })
    }

    fn insert(&mut self, source: &TriMesh, element: usize) -> Result<(), WaveError> {
        let triangle = source.triangles[element];
        let points = triangle
            .vertices
            .map(|vertex| source.vertices.get(vertex).map(|vertex| vertex.point));
        let [Some(a), Some(b), Some(c)] = points else {
            return Err(WaveError::InvalidMesh(
                "the complementary transfer source has invalid triangles",
            ));
        };
        let points = [a, b, c];
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
        let [x0, y0] = self.bin_index(lo);
        let [x1, y1] = self.bin_index(hi);
        let element = u32::try_from(element)
            .map_err(|_| WaveError::InvalidMesh("too many complementary donor elements"))?;
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.bins[y * self.dimension + x].push(element);
            }
        }
        Ok(())
    }

    fn locate(
        &self,
        source: &TriMesh,
        point: Point2,
        region: crate::RegionId,
    ) -> Option<(usize, [f64; 3])> {
        if point.x < self.minimum.x
            || point.x > self.maximum.x
            || point.y < self.minimum.y
            || point.y > self.maximum.y
        {
            return None;
        }
        let [x, y] = self.bin_index(point);
        self.bins[y * self.dimension + x]
            .iter()
            .find_map(|element| {
                let triangle = source.triangles[*element as usize];
                if triangle.region != region {
                    return None;
                }
                barycentric(source, triangle.vertices, point)
                    .filter(|weights| weights.iter().all(|weight| *weight >= -2.0e-11))
                    .map(|weights| (*element as usize, weights))
            })
    }

    fn bin_index(&self, point: Point2) -> [usize; 2] {
        let index = |value: f64, minimum: f64, width: f64| {
            (((value - minimum) / width).floor() as isize).clamp(0, self.dimension as isize - 1)
                as usize
        };
        [
            index(point.x, self.minimum.x, self.cell.x),
            index(point.y, self.minimum.y, self.cell.y),
        ]
    }
}

struct CanonicalVectorTransferWork {
    phase: CanonicalVectorTransferPhase,
    identity: bool,
    bins: Option<CanonicalSourceBins>,
    targets: Vec<VectorTarget>,
    distance: Vec<usize>,
}

impl CanonicalVectorTransferWork {
    fn new() -> Self {
        Self {
            phase: CanonicalVectorTransferPhase::Validate,
            identity: false,
            bins: None,
            targets: Vec::new(),
            distance: Vec::new(),
        }
    }

    fn step(
        &mut self,
        source_mesh: &TriMesh,
        source: &CanonicalWaveOperator,
        target_mesh: &TriMesh,
        target: &CanonicalWaveOperator,
    ) -> Result<Option<CanonicalVectorTransferMap>, WaveError> {
        match self.phase {
            CanonicalVectorTransferPhase::Validate => {
                if source_mesh.triangles.len() * QUADRATURE_SAMPLES
                    != source.complementary_degrees_of_freedom()
                    || target_mesh.triangles.len() * QUADRATURE_SAMPLES
                        != target.complementary_degrees_of_freedom()
                {
                    return Err(WaveError::InvalidMesh(
                        "the vector transfer generations have inconsistent sample layouts",
                    ));
                }
                // Reusing the exact immutable operator proves every carrier,
                // coordinate and ordering identical. Source-layout-only full
                // handoffs must not spend one cooperative work unit checking
                // every complementary sample again.
                if std::ptr::eq(source, target) {
                    self.phase = CanonicalVectorTransferPhase::Done;
                    return Ok(Some(CanonicalVectorTransferMap {
                        source_samples: source.complementary_degrees_of_freedom(),
                        identity: true,
                        targets: Vec::new(),
                    }));
                }
                // Revisions identify transactions, not discretizations. A
                // law-only rebuild commonly republishes an identical mesh
                // under a new revision; its quadrature carriers still copy
                // one-for-one and must not fall through to spatial search.
                self.identity = source_mesh.vertices == target_mesh.vertices
                    && source_mesh.triangles == target_mesh.triangles
                    && source.constitutive_samples().len() == target.constitutive_samples().len();
                if self.identity {
                    self.phase = CanonicalVectorTransferPhase::Locate(0);
                } else {
                    self.targets = Vec::with_capacity(target.complementary_degrees_of_freedom());
                    self.bins = Some(CanonicalSourceBins::new(source_mesh)?);
                    self.phase = CanonicalVectorTransferPhase::Bins(0);
                }
            }
            CanonicalVectorTransferPhase::Bins(index) => {
                if index == source_mesh.triangles.len() {
                    self.phase = CanonicalVectorTransferPhase::Locate(0);
                } else {
                    self.bins.as_mut().unwrap().insert(source_mesh, index)?;
                    self.phase = CanonicalVectorTransferPhase::Bins(index + 1);
                }
            }
            CanonicalVectorTransferPhase::Locate(target_index) => {
                let samples = target.constitutive_samples();
                if target_index == samples.len() {
                    if self.identity {
                        self.phase = CanonicalVectorTransferPhase::Done;
                        return Ok(Some(CanonicalVectorTransferMap {
                            source_samples: source.complementary_degrees_of_freedom(),
                            identity: true,
                            targets: Vec::new(),
                        }));
                    }
                    self.phase = CanonicalVectorTransferPhase::PrepareExtension;
                    return Ok(None);
                }
                let sample = &samples[target_index];
                if self.identity {
                    if source.constitutive_samples()[target_index].point != sample.point
                        || source.constitutive_samples()[target_index].barycentric
                            != sample.barycentric
                    {
                        return Err(WaveError::InvalidMesh(
                            "identity complementary samples do not share physical coordinates",
                        ));
                    }
                    self.phase = CanonicalVectorTransferPhase::Locate(target_index + 1);
                    return Ok(None);
                }
                let target_region = target_mesh.triangles[sample.element as usize].region;
                let donor = self
                    .bins
                    .as_ref()
                    .and_then(|bins| bins.locate(source_mesh, sample.point, target_region));
                if let Some((source_element, source_barycentric)) = donor {
                    self.targets.push(VectorTarget::Reconstruct {
                        source_element: source_element as u32,
                        weights: quadratic_sample_weights(
                            &source.constitutive_samples()[source_element * QUADRATURE_SAMPLES
                                ..(source_element + 1) * QUADRATURE_SAMPLES],
                            source_barycentric,
                        )?,
                    });
                } else {
                    self.targets.push(VectorTarget::Exposed);
                }
                self.phase = CanonicalVectorTransferPhase::Locate(target_index + 1);
            }
            CanonicalVectorTransferPhase::PrepareExtension => {
                self.distance = vec![usize::MAX; target_mesh.triangles.len()];
                for (sample_index, target_sample) in self.targets.iter().enumerate() {
                    if !matches!(target_sample, VectorTarget::Exposed) {
                        self.distance[sample_index / QUADRATURE_SAMPLES] = 0;
                    }
                }
                extend_element_distances(target_mesh, &mut self.distance, 2);
                self.phase = CanonicalVectorTransferPhase::Extend(0);
            }
            CanonicalVectorTransferPhase::Extend(target_index) => {
                if target_index == self.targets.len() {
                    self.phase = CanonicalVectorTransferPhase::Done;
                    return Ok(Some(CanonicalVectorTransferMap {
                        source_samples: source.complementary_degrees_of_freedom(),
                        identity: false,
                        targets: std::mem::take(&mut self.targets),
                    }));
                }
                let entry = &mut self.targets[target_index];
                if matches!(entry, VectorTarget::Exposed)
                    && self.distance[target_index / QUADRATURE_SAMPLES] <= 2
                {
                    let sample = &target.constitutive_samples()[target_index];
                    let target_region = target_mesh.triangles[sample.element as usize].region;
                    let donor = source_mesh
                        .triangles
                        .iter()
                        .enumerate()
                        .filter(|(_, triangle)| triangle.region == target_region)
                        .min_by(|(_, left), (_, right)| {
                            let left = triangle_centroid(source_mesh, left.vertices) - sample.point;
                            let right =
                                triangle_centroid(source_mesh, right.vertices) - sample.point;
                            left.dot(left).total_cmp(&right.dot(right))
                        })
                        .map(|(element, _)| element as u32);
                    if let Some(source_element) = donor {
                        *entry = VectorTarget::ExtendConstant { source_element };
                    }
                }
                self.phase = CanonicalVectorTransferPhase::Extend(target_index + 1);
            }
            CanonicalVectorTransferPhase::Done => {}
        }
        Ok(None)
    }
}

/// Resumable construction of the local physical-coordinate complementary map.
/// It owns both generations so a UI preparation can yield without borrowing
/// mutable application state.
pub struct CanonicalVectorTransferJob {
    source_mesh: Arc<TriMesh>,
    source: Arc<CanonicalWaveOperator>,
    target_mesh: Arc<TriMesh>,
    target: Arc<CanonicalWaveOperator>,
    work: CanonicalVectorTransferWork,
    done: bool,
}

impl CanonicalVectorTransferJob {
    pub fn new(
        source_mesh: Arc<TriMesh>,
        source: Arc<CanonicalWaveOperator>,
        target_mesh: Arc<TriMesh>,
        target: Arc<CanonicalWaveOperator>,
    ) -> Self {
        Self {
            source_mesh,
            source,
            target_mesh,
            target,
            work: CanonicalVectorTransferWork::new(),
            done: false,
        }
    }

    pub fn phase(&self) -> &'static str {
        self.work.phase.label()
    }

    pub fn advance(
        &mut self,
        budget: usize,
    ) -> Option<Result<CanonicalVectorTransferMap, WaveError>> {
        for _ in 0..budget {
            if self.done {
                return None;
            }
            match self.work.step(
                &self.source_mesh,
                &self.source,
                &self.target_mesh,
                &self.target,
            ) {
                Ok(Some(map)) => {
                    self.done = true;
                    return Some(Ok(map));
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

impl CanonicalVectorTransferMap {
    pub fn prepare(
        source_mesh: &TriMesh,
        source: &CanonicalWaveOperator,
        target_mesh: &TriMesh,
        target: &CanonicalWaveOperator,
    ) -> Result<Self, WaveError> {
        let mut work = CanonicalVectorTransferWork::new();
        loop {
            if let Some(map) = work.step(source_mesh, source, target_mesh, target)? {
                return Ok(map);
            }
        }
    }

    pub fn exact_samples(&self) -> usize {
        if self.identity {
            return self.source_samples;
        }
        0
    }

    pub fn source_sample_count(&self) -> usize {
        self.source_samples
    }

    pub fn is_identity(&self) -> bool {
        self.identity
    }

    pub fn targets(&self) -> Vec<CanonicalVectorTransferTarget> {
        if self.identity {
            return (0..self.source_samples)
                .map(|index| CanonicalVectorTransferTarget {
                    source_samples: [index as u32, 0, 0, 0, 0, 0],
                    weights: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                    source_count: 1,
                    exact: true,
                })
                .collect();
        }
        self.targets
            .iter()
            .map(|target| {
                let mut source_samples = [0_u32; QUADRATURE_SAMPLES];
                let mut weights = [0.0; QUADRATURE_SAMPLES];
                let (source_count, exact) = match target {
                    VectorTarget::Reconstruct {
                        source_element,
                        weights: prepared,
                    } => {
                        for (local, source_sample) in source_samples.iter_mut().enumerate() {
                            *source_sample =
                                *source_element * QUADRATURE_SAMPLES as u32 + local as u32;
                        }
                        weights = *prepared;
                        (QUADRATURE_SAMPLES as u8, false)
                    }
                    VectorTarget::ExtendConstant { source_element } => {
                        for (local, (source_sample, weight)) in source_samples
                            .iter_mut()
                            .zip(weights.iter_mut())
                            .enumerate()
                        {
                            *source_sample =
                                *source_element * QUADRATURE_SAMPLES as u32 + local as u32;
                            *weight = 1.0 / QUADRATURE_SAMPLES as f64;
                        }
                        (QUADRATURE_SAMPLES as u8, false)
                    }
                    VectorTarget::Exposed => (0, false),
                };
                CanonicalVectorTransferTarget {
                    source_samples,
                    weights,
                    source_count,
                    exact,
                }
            })
            .collect()
    }

    pub fn transfer(
        &self,
        source: &[Point2],
    ) -> Result<(Vec<Point2>, CanonicalTransferReport), WaveError> {
        if source.len() != self.source_samples || source.iter().any(|value| !value.finite()) {
            return Err(WaveError::InvalidState);
        }
        if self.identity {
            return Ok((source.to_vec(), CanonicalTransferReport::default()));
        }
        let mut exposed = 0;
        let mut extended = 0;
        let values = self
            .targets
            .iter()
            .map(|target| match target {
                VectorTarget::Reconstruct {
                    source_element,
                    weights,
                } => {
                    let start = *source_element as usize * QUADRATURE_SAMPLES;
                    weights
                        .iter()
                        .enumerate()
                        .fold(Point2::default(), |value, (local, weight)| {
                            value + source[start + local] * *weight
                        })
                }
                VectorTarget::ExtendConstant { source_element } => {
                    extended += 1;
                    let start = *source_element as usize * QUADRATURE_SAMPLES;
                    (0..QUADRATURE_SAMPLES).fold(Point2::default(), |value, local| {
                        value + source[start + local] / QUADRATURE_SAMPLES as f64
                    })
                }
                VectorTarget::Exposed => {
                    exposed += 1;
                    Point2::default()
                }
            })
            .collect::<Vec<_>>();
        if values.iter().any(|value| !value.finite()) {
            return Err(WaveError::InvalidState);
        }
        Ok((
            values,
            CanonicalTransferReport {
                maximum_correction_ratio: 0.0,
                exposed_values: exposed,
                extended_values: extended,
            },
        ))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanonicalHistoryTransferReport {
    pub source_energy: f64,
    pub target_energy: f64,
    /// Energy assigned to the topology/material edit. Positive means the edit
    /// introduced stored history energy; negative means it removed energy.
    pub edit_exchange: f64,
    pub physical_residual_norm: f64,
    pub exact_samples: usize,
    pub new_samples: usize,
    pub deleted_samples: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum GapGroup {
    Legacy(crate::InternalBoundaryId),
    Topology(crate::CurveId, crate::CurveSpanId),
}

#[derive(Clone, Debug, PartialEq)]
enum GapTarget {
    Exact(usize),
    Interpolate {
        donors: Vec<(usize, f64)>,
        orientation: f64,
    },
    New,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalThinGapTransferTarget {
    pub donors: Vec<(u32, f64)>,
    pub exact: bool,
}

/// Prepared transfer of the physical thin-gap jump, independent of Q and b.
/// Matching is by stable object/span identity and authored parameter, so mesh
/// split/merge order cannot turn a physical trace into an unrelated donor.
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalThinGapHistoryTransferMap {
    source_samples: Vec<ThinGapSample>,
    target_samples: Vec<ThinGapSample>,
    targets: Vec<GapTarget>,
    deleted_samples: usize,
}

impl CanonicalThinGapHistoryTransferMap {
    pub fn prepare(source: &[ThinGapSample], target: &[ThinGapSample]) -> Result<Self, WaveError> {
        let mut source_keys = BTreeSet::new();
        let mut target_keys = BTreeSet::new();
        if source.iter().any(|sample| {
            let (_, start, end, local) = gap_key_parts(sample.key);
            !sample.stiffness.is_finite()
                || sample.stiffness <= 0.0
                || !start.is_finite()
                || !end.is_finite()
                || start == end
                || local > 2
                || !source_keys.insert(sample.key)
        }) || target.iter().any(|sample| {
            let (_, start, end, local) = gap_key_parts(sample.key);
            !sample.stiffness.is_finite()
                || sample.stiffness <= 0.0
                || !start.is_finite()
                || !end.is_finite()
                || start == end
                || local > 2
                || !target_keys.insert(sample.key)
        }) {
            return Err(WaveError::InvalidMesh(
                "thin-gap history keys must be unique with positive stiffness",
            ));
        }
        let mut used = vec![false; source.len()];
        let mut targets = Vec::with_capacity(target.len());
        for target_sample in target {
            if let Some(index) = source
                .iter()
                .position(|source_sample| source_sample.key == target_sample.key)
            {
                used[index] = true;
                targets.push(GapTarget::Exact(index));
                continue;
            }
            let (target_group, target_start, target_end, target_local) =
                gap_key_parts(target_sample.key);
            let target_parameter = local_parameter(target_start, target_end, target_local);
            let donor_segment = source
                .iter()
                .enumerate()
                .filter_map(|(index, source_sample)| {
                    let (group, start, end, _) = gap_key_parts(source_sample.key);
                    if group != target_group
                        || target_parameter < start.min(end) - 2.0e-12
                        || target_parameter > start.max(end) + 2.0e-12
                    {
                        return None;
                    }
                    Some((index, start, end, (end - start).abs()))
                })
                .min_by(|left, right| left.3.total_cmp(&right.3));
            let Some((_, source_start, source_end, _)) = donor_segment else {
                targets.push(GapTarget::New);
                continue;
            };
            let mut segment_samples = Vec::new();
            for (index, source_sample) in source.iter().enumerate() {
                let (group, start, end, local) = gap_key_parts(source_sample.key);
                if group == target_group
                    && start.to_bits() == source_start.to_bits()
                    && end.to_bits() == source_end.to_bits()
                {
                    segment_samples.push((index, local_parameter(source_start, source_end, local)));
                }
            }
            if segment_samples.is_empty()
                || segment_samples
                    .iter()
                    .any(|(_, parameter)| !parameter.is_finite())
            {
                return Err(WaveError::InvalidMesh(
                    "a thin-gap segment has no valid history samples",
                ));
            }
            let donors = segment_samples
                .iter()
                .enumerate()
                .map(|(here, (index, parameter))| {
                    let weight = segment_samples
                        .iter()
                        .enumerate()
                        .filter(|(other, _)| *other != here)
                        .map(|(_, (_, other_parameter))| {
                            (target_parameter - other_parameter) / (parameter - other_parameter)
                        })
                        .product::<f64>();
                    (*index, weight)
                })
                .collect::<Vec<_>>();
            let orientation = if (source_end - source_start) * (target_end - target_start) >= 0.0 {
                1.0
            } else {
                -1.0
            };
            for (donor, _) in &donors {
                used[*donor] = true;
            }
            targets.push(GapTarget::Interpolate {
                donors,
                orientation,
            });
        }
        Ok(Self {
            source_samples: source.to_vec(),
            target_samples: target.to_vec(),
            targets,
            deleted_samples: used.iter().filter(|used| !**used).count(),
        })
    }

    pub fn transfer(
        &self,
        source: &[CanonicalThinGapMemory],
    ) -> Result<(Vec<CanonicalThinGapMemory>, CanonicalHistoryTransferReport), WaveError> {
        if source.len() != self.source_samples.len() {
            return Err(WaveError::InvalidState);
        }
        let by_key = source
            .iter()
            .map(|entry| (entry.key, entry.jump))
            .collect::<BTreeMap<_, _>>();
        if by_key.len() != source.len() || source.iter().any(|entry| !entry.jump.is_finite()) {
            return Err(WaveError::InvalidState);
        }
        let source_values = self
            .source_samples
            .iter()
            .map(|sample| {
                by_key
                    .get(&sample.key)
                    .copied()
                    .ok_or(WaveError::InvalidState)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut exact_samples = 0;
        let mut new_samples = 0;
        let target = self
            .targets
            .iter()
            .zip(&self.target_samples)
            .map(|(mapping, sample)| {
                let jump = match mapping {
                    GapTarget::Exact(index) => {
                        exact_samples += 1;
                        source_values[*index]
                    }
                    GapTarget::Interpolate {
                        donors,
                        orientation,
                    } => {
                        orientation
                            * donors
                                .iter()
                                .map(|(donor, weight)| source_values[*donor] * weight)
                                .sum::<f64>()
                    }
                    GapTarget::New => {
                        new_samples += 1;
                        0.0
                    }
                };
                CanonicalThinGapMemory {
                    key: sample.key,
                    jump,
                }
            })
            .collect::<Vec<_>>();
        let source_energy = source_values
            .iter()
            .zip(&self.source_samples)
            .map(|(jump, sample)| 0.5 * sample.stiffness * jump * jump)
            .sum::<f64>();
        let target_energy = target
            .iter()
            .zip(&self.target_samples)
            .map(|(memory, sample)| 0.5 * sample.stiffness * memory.jump * memory.jump)
            .sum::<f64>();
        Ok((
            target,
            CanonicalHistoryTransferReport {
                source_energy,
                target_energy,
                edit_exchange: target_energy - source_energy,
                exact_samples,
                new_samples,
                deleted_samples: self.deleted_samples,
                ..CanonicalHistoryTransferReport::default()
            },
        ))
    }

    pub fn targets(&self) -> Vec<CanonicalThinGapTransferTarget> {
        self.targets
            .iter()
            .map(|target| match target {
                GapTarget::Exact(index) => CanonicalThinGapTransferTarget {
                    donors: vec![(*index as u32, 1.0)],
                    exact: true,
                },
                GapTarget::Interpolate {
                    donors,
                    orientation,
                } => CanonicalThinGapTransferTarget {
                    donors: donors
                        .iter()
                        .map(|(index, weight)| (*index as u32, *weight * *orientation))
                        .collect(),
                    exact: false,
                },
                GapTarget::New => CanonicalThinGapTransferTarget {
                    donors: Vec::new(),
                    exact: false,
                },
            })
            .collect()
    }

    pub fn source_energy_weights(&self) -> Vec<f64> {
        self.source_samples
            .iter()
            .map(|sample| sample.stiffness)
            .collect()
    }

    pub fn target_energy_weights(&self) -> Vec<f64> {
        self.target_samples
            .iter()
            .map(|sample| sample.stiffness)
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
enum TraceTarget {
    Map(Vec<(usize, f64)>),
    New,
}

/// Stable physical-trace transfer for outgoing pole currents. Modal indices
/// never cross the generation boundary; the target boundary performs its own
/// minimum-energy projection after this map is applied.
#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalOutgoingHistoryTransferMap {
    targets: Vec<TraceTarget>,
    source_trace_count: usize,
    deleted_samples: usize,
}

impl CanonicalOutgoingHistoryTransferMap {
    pub fn prepare(
        interpolation: &QuadraticTransferMap,
        source: &CanonicalWaveOperator,
        target: &CanonicalWaveOperator,
    ) -> Result<Self, WaveError> {
        if interpolation.source_dofs() != source.degrees_of_freedom()
            || interpolation.samples().len() != target.degrees_of_freedom()
        {
            return Err(WaveError::InvalidMesh(
                "the outgoing history map does not match the canonical generations",
            ));
        }
        let source_nodes = source
            .outgoing_boundary()
            .map(|boundary| boundary.trace_nodes())
            .unwrap_or_default();
        let source_positions = source_nodes
            .iter()
            .enumerate()
            .map(|(position, node)| (*node, position))
            .collect::<BTreeMap<_, _>>();
        let target_nodes = target
            .outgoing_boundary()
            .map(|boundary| boundary.trace_nodes())
            .unwrap_or_default();
        let mut used = vec![false; source_nodes.len()];
        let mut targets = Vec::with_capacity(target_nodes.len());
        for node in target_nodes {
            let Some(sample) = interpolation.samples()[*node as usize] else {
                targets.push(TraceTarget::New);
                continue;
            };
            let mut donors = BTreeMap::<usize, f64>::new();
            let mut valid = true;
            for (source_node, weight) in sample.nodes.iter().zip(sample.weights) {
                if weight.abs() <= 2.0e-13 {
                    continue;
                }
                let Some(position) = source_positions.get(source_node).copied() else {
                    valid = false;
                    break;
                };
                *donors.entry(position).or_default() += weight;
            }
            if !valid || donors.is_empty() {
                targets.push(TraceTarget::New);
                continue;
            }
            let weight_sum = donors.values().sum::<f64>();
            if (weight_sum - 1.0).abs() > 2.0e-11 {
                return Err(WaveError::InvalidMesh(
                    "the outgoing trace interpolation does not preserve constants",
                ));
            }
            for donor in donors.keys() {
                used[*donor] = true;
            }
            targets.push(TraceTarget::Map(donors.into_iter().collect()));
        }
        Ok(Self {
            targets,
            source_trace_count: source_nodes.len(),
            deleted_samples: used.iter().filter(|used| !**used).count(),
        })
    }

    pub fn transfer(
        &self,
        source_operator: &CanonicalWaveOperator,
        target_operator: &CanonicalWaveOperator,
        source: Option<&CanonicalOutgoingPhysicalMemory>,
    ) -> Result<
        (
            Option<CanonicalOutgoingPhysicalMemory>,
            CanonicalHistoryTransferReport,
        ),
        WaveError,
    > {
        let source_energy = match (source_operator.outgoing_boundary(), source) {
            (None, None) => 0.0,
            (Some(boundary), Some(memory)) => {
                if memory
                    .pole_currents
                    .iter()
                    .any(|values| values.len() != self.source_trace_count)
                {
                    return Err(WaveError::InvalidState);
                }
                boundary.project_physical_memory(memory)?.1.projected_energy
            }
            _ => return Err(WaveError::InvalidState),
        };
        let Some(target_boundary) = target_operator.outgoing_boundary() else {
            return Ok((
                None,
                CanonicalHistoryTransferReport {
                    source_energy,
                    edit_exchange: -source_energy,
                    deleted_samples: self.deleted_samples,
                    ..CanonicalHistoryTransferReport::default()
                },
            ));
        };
        let zero_source = CanonicalOutgoingPhysicalMemory {
            pole_currents: std::array::from_fn(|_| vec![0.0; self.source_trace_count]),
        };
        let source = source.unwrap_or(&zero_source);
        let mut exact_samples = 0;
        let mut new_samples = 0;
        let pole_currents = std::array::from_fn(|pole| {
            self.targets
                .iter()
                .map(|target| match target {
                    TraceTarget::Map(donors) => {
                        if donors.len() == 1 && donors[0].1 == 1.0 {
                            exact_samples += usize::from(pole == 0);
                        }
                        donors
                            .iter()
                            .map(|(donor, weight)| source.pole_currents[pole][*donor] * weight)
                            .sum()
                    }
                    TraceTarget::New => {
                        new_samples += usize::from(pole == 0);
                        0.0
                    }
                })
                .collect::<Vec<_>>()
        });
        let physical = CanonicalOutgoingPhysicalMemory { pole_currents };
        let (_, projection) = target_boundary.project_physical_memory(&physical)?;
        let target_energy = projection.projected_energy;
        Ok((
            Some(physical),
            CanonicalHistoryTransferReport {
                source_energy,
                target_energy,
                edit_exchange: target_energy - source_energy,
                physical_residual_norm: projection.physical_residual_norm,
                exact_samples,
                new_samples,
                deleted_samples: self.deleted_samples,
            },
        ))
    }

    /// Composes physical-trace interpolation with the source and target
    /// energy-normalized modal bases. The returned row-major matrix maps the
    /// source normalized auxiliary vector directly to the target vector while
    /// remaining invariant to modal signs, ordering, and degenerate rotations.
    pub fn normalized_matrix(
        &self,
        source_operator: &CanonicalWaveOperator,
        target_operator: &CanonicalWaveOperator,
    ) -> Result<CanonicalOutgoingNormalizedTransfer, WaveError> {
        let mut work = CanonicalOutgoingNormalizedTransferWork::new();
        loop {
            if let Some(map) = work.step(self, source_operator, target_operator)? {
                return Ok(map);
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalOutgoingNormalizedTransfer {
    pub source_count: usize,
    pub target_count: usize,
    pub identity: bool,
    pub values: Vec<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CanonicalOutgoingNormalizedTransferPhase {
    Validate,
    Pair(usize),
    ValidateValues(usize),
    Done,
}

impl CanonicalOutgoingNormalizedTransferPhase {
    const fn label(self) -> &'static str {
        match self {
            Self::Validate => "Checking outgoing-history transfer",
            Self::Pair(_) => "Composing outgoing-history bases",
            Self::ValidateValues(_) => "Validating outgoing-history transfer",
            Self::Done => "Finished",
        }
    }
}

struct CanonicalOutgoingNormalizedTransferWork {
    phase: CanonicalOutgoingNormalizedTransferPhase,
    source_count: usize,
    target_count: usize,
    source_damping: Vec<f64>,
    target_damping: Vec<f64>,
    pole_transform: [[f64; 3]; 3],
    values: Vec<f64>,
}

impl CanonicalOutgoingNormalizedTransferWork {
    fn new() -> Self {
        Self {
            phase: CanonicalOutgoingNormalizedTransferPhase::Validate,
            source_count: 0,
            target_count: 0,
            source_damping: Vec::new(),
            target_damping: Vec::new(),
            pole_transform: [[0.0; 3]; 3],
            values: Vec::new(),
        }
    }

    fn step(
        &mut self,
        map: &CanonicalOutgoingHistoryTransferMap,
        source_operator: &CanonicalWaveOperator,
        target_operator: &CanonicalWaveOperator,
    ) -> Result<Option<CanonicalOutgoingNormalizedTransfer>, WaveError> {
        match self.phase {
            CanonicalOutgoingNormalizedTransferPhase::Validate => {
                self.source_count = source_operator
                    .outgoing_boundary()
                    .map_or(0, CanonicalOutgoingBoundary::auxiliary_count);
                self.target_count = target_operator
                    .outgoing_boundary()
                    .map_or(0, CanonicalOutgoingBoundary::auxiliary_count);
                let (Some(source), Some(target)) = (
                    source_operator.outgoing_boundary(),
                    target_operator.outgoing_boundary(),
                ) else {
                    self.phase = CanonicalOutgoingNormalizedTransferPhase::Done;
                    return Ok(Some(CanonicalOutgoingNormalizedTransfer {
                        source_count: self.source_count,
                        target_count: self.target_count,
                        identity: self.source_count == 0 && self.target_count == 0,
                        values: Vec::new(),
                    }));
                };
                if map.source_trace_count != source.trace_nodes().len()
                    || map.targets.len() != target.trace_nodes().len()
                {
                    return Err(WaveError::InvalidState);
                }
                let identity = self.source_count == self.target_count
                    && source == target
                    && map.targets.iter().enumerate().all(|(index, target)| {
                        matches!(target, TraceTarget::Map(donors) if donors.as_slice() == [(index, 1.0)])
                    });
                if identity {
                    self.phase = CanonicalOutgoingNormalizedTransferPhase::Done;
                    return Ok(Some(CanonicalOutgoingNormalizedTransfer {
                        source_count: self.source_count,
                        target_count: self.target_count,
                        identity: true,
                        values: Vec::new(),
                    }));
                }
                self.values = vec![0.0; self.source_count * self.target_count];
                let (energy, inverse_energy) = crate::canonical_wave::pole_energy_transform()?;
                self.pole_transform = std::array::from_fn(|row| {
                    std::array::from_fn(|column| {
                        (0..3)
                            .map(|index| energy[row][index] * inverse_energy[index][column])
                            .sum()
                    })
                });
                self.source_damping = (0..source.trace_nodes().len())
                    .map(|trace| {
                        source
                            .modes()
                            .iter()
                            .map(|mode| mode.trace()[trace].powi(2))
                            .sum::<f64>()
                    })
                    .collect();
                self.target_damping = (0..target.trace_nodes().len())
                    .map(|trace| {
                        target
                            .modes()
                            .iter()
                            .map(|mode| mode.trace()[trace].powi(2))
                            .sum::<f64>()
                    })
                    .collect();
                self.phase = CanonicalOutgoingNormalizedTransferPhase::Pair(0);
            }
            CanonicalOutgoingNormalizedTransferPhase::Pair(pair) => {
                let source = source_operator
                    .outgoing_boundary()
                    .ok_or(WaveError::InvalidState)?;
                let target = target_operator
                    .outgoing_boundary()
                    .ok_or(WaveError::InvalidState)?;
                let pair_count = source.modes().len() * target.modes().len();
                if pair == pair_count {
                    self.phase = CanonicalOutgoingNormalizedTransferPhase::ValidateValues(0);
                    return Ok(None);
                }
                let source_modes = source.modes().len();
                let target_mode = &target.modes()[pair / source_modes];
                let source_mode = &source.modes()[pair % source_modes];
                if let (Some(target_offset), Some(source_offset)) = (
                    target_mode.auxiliary_offset(),
                    source_mode.auxiliary_offset(),
                ) {
                    let mut overlap = 0.0;
                    for (target_trace, mapping) in map.targets.iter().enumerate() {
                        let target_weight = if self.target_damping[target_trace] > 0.0 {
                            target_mode.trace()[target_trace]
                                / self.target_damping[target_trace].sqrt()
                        } else {
                            0.0
                        };
                        let source_weight = match mapping {
                            TraceTarget::Map(donors) => donors
                                .iter()
                                .map(|(source_trace, weight)| {
                                    if self.source_damping[*source_trace] > 0.0 {
                                        weight * source_mode.trace()[*source_trace]
                                            / self.source_damping[*source_trace].sqrt()
                                    } else {
                                        0.0
                                    }
                                })
                                .sum::<f64>(),
                            TraceTarget::New => 0.0,
                        };
                        overlap += target_weight * source_weight;
                    }
                    let spatial = overlap * (source_mode.decay / target_mode.decay).sqrt();
                    for target_row in 0..3 {
                        for source_column in 0..3 {
                            self.values[(target_offset + target_row) * self.source_count
                                + source_offset
                                + source_column] +=
                                spatial * self.pole_transform[target_row][source_column];
                        }
                    }
                }
                self.phase = CanonicalOutgoingNormalizedTransferPhase::Pair(pair + 1);
            }
            CanonicalOutgoingNormalizedTransferPhase::ValidateValues(start) => {
                const VALIDATION_BLOCK: usize = 4096;
                let end = (start + VALIDATION_BLOCK).min(self.values.len());
                if self.values[start..end]
                    .iter()
                    .any(|value| !value.is_finite())
                {
                    return Err(WaveError::InvalidState);
                }
                if end < self.values.len() {
                    self.phase = CanonicalOutgoingNormalizedTransferPhase::ValidateValues(end);
                } else {
                    self.phase = CanonicalOutgoingNormalizedTransferPhase::Done;
                    return Ok(Some(CanonicalOutgoingNormalizedTransfer {
                        source_count: self.source_count,
                        target_count: self.target_count,
                        identity: false,
                        values: std::mem::take(&mut self.values),
                    }));
                }
            }
            CanonicalOutgoingNormalizedTransferPhase::Done => {}
        }
        Ok(None)
    }
}

/// Resumable composition of physical outgoing-trace correspondence with the
/// source and target energy-normalized modal bases.
pub struct CanonicalOutgoingNormalizedTransferJob {
    map: Arc<CanonicalOutgoingHistoryTransferMap>,
    source: Arc<CanonicalWaveOperator>,
    target: Arc<CanonicalWaveOperator>,
    work: CanonicalOutgoingNormalizedTransferWork,
    done: bool,
}

impl CanonicalOutgoingNormalizedTransferJob {
    pub fn new(
        map: Arc<CanonicalOutgoingHistoryTransferMap>,
        source: Arc<CanonicalWaveOperator>,
        target: Arc<CanonicalWaveOperator>,
    ) -> Self {
        Self {
            map,
            source,
            target,
            work: CanonicalOutgoingNormalizedTransferWork::new(),
            done: false,
        }
    }

    pub fn phase(&self) -> &'static str {
        self.work.phase.label()
    }

    pub fn advance(
        &mut self,
        budget: usize,
    ) -> Option<Result<CanonicalOutgoingNormalizedTransfer, WaveError>> {
        for _ in 0..budget {
            if self.done {
                return None;
            }
            match self.work.step(&self.map, &self.source, &self.target) {
                Ok(Some(map)) => {
                    self.done = true;
                    return Some(Ok(map));
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

fn gap_key_parts(key: ThinGapTraceKey) -> (GapGroup, f64, f64, u8) {
    match key {
        ThinGapTraceKey::Legacy {
            boundary,
            start_bits,
            end_bits,
            local,
        } => (
            GapGroup::Legacy(boundary),
            f64::from_bits(start_bits),
            f64::from_bits(end_bits),
            local,
        ),
        ThinGapTraceKey::Topology {
            curve,
            span,
            start_bits,
            end_bits,
            local,
        } => (
            GapGroup::Topology(curve, span),
            f64::from_bits(start_bits),
            f64::from_bits(end_bits),
            local,
        ),
    }
}

fn local_parameter(start: f64, end: f64, local: u8) -> f64 {
    match local {
        0 => start,
        1 => 0.5 * (start + end),
        2 => end,
        _ => f64::NAN,
    }
}

fn triangle_centroid(mesh: &TriMesh, vertices: [usize; 3]) -> Point2 {
    vertices.iter().fold(Point2::default(), |sum, vertex| {
        sum + mesh.vertices[*vertex].point
    }) / 3.0
}

fn extend_element_distances(mesh: &TriMesh, distance: &mut [usize], rings: usize) {
    let mut sides = BTreeMap::<(usize, usize), Vec<usize>>::new();
    for (triangle, element) in mesh.triangles.iter().enumerate() {
        for [left, right] in [
            [element.vertices[0], element.vertices[1]],
            [element.vertices[1], element.vertices[2]],
            [element.vertices[2], element.vertices[0]],
        ] {
            sides
                .entry((left.min(right), left.max(right)))
                .or_default()
                .push(triangle);
        }
    }
    let mut adjacent = vec![Vec::new(); mesh.triangles.len()];
    for triangles in sides.values() {
        for &left in triangles {
            for &right in triangles {
                if left != right {
                    adjacent[left].push(right);
                }
            }
        }
    }
    for ring in 0..rings {
        for left in 0..mesh.triangles.len() {
            if distance[left] != ring {
                continue;
            }
            for &right in &adjacent[left] {
                if distance[right] > ring + 1 {
                    distance[right] = ring + 1;
                }
            }
        }
    }
}

fn barycentric(mesh: &TriMesh, vertices: [usize; 3], point: Point2) -> Option<[f64; 3]> {
    let [a, b, c] = vertices.map(|vertex| mesh.vertices.get(vertex).map(|vertex| vertex.point));
    let [a, b, c] = [a?, b?, c?];
    let area = (b - a).cross(c - a);
    if !area.is_finite() || area <= 0.0 {
        return None;
    }
    let l1 = (point - a).cross(c - a) / area;
    let l2 = (b - a).cross(point - a) / area;
    Some([1.0 - l1 - l2, l1, l2])
}

fn monomials([_l0, l1, l2]: [f64; 3]) -> [f64; 6] {
    [1.0, l1, l2, l1 * l1, l1 * l2, l2 * l2]
}

fn quadratic_sample_weights(
    samples: &[crate::LinearConstitutiveSample],
    target: [f64; 3],
) -> Result<[f64; 6], WaveError> {
    if samples.len() != QUADRATURE_SAMPLES {
        return Err(WaveError::InvalidMesh(
            "a vector donor element has the wrong quadrature layout",
        ));
    }
    // A^T w=p(target), where rows of A evaluate the quadratic monomials at
    // the six old samples.
    let mut matrix = [[0.0; 6]; 6];
    for (sample, row) in samples.iter().zip(0..6) {
        let values = monomials(sample.barycentric);
        for column in 0..6 {
            matrix[column][row] = values[column];
        }
    }
    solve_six(matrix, monomials(target))
}

fn solve_six(mut matrix: [[f64; 6]; 6], mut right: [f64; 6]) -> Result<[f64; 6], WaveError> {
    for pivot in 0..6 {
        let best = (pivot..6)
            .max_by(|left, right_row| {
                matrix[*left][pivot]
                    .abs()
                    .total_cmp(&matrix[*right_row][pivot].abs())
            })
            .ok_or(WaveError::InvalidMesh(
                "the vector reconstruction is singular",
            ))?;
        if matrix[best][pivot].abs() <= 1.0e-14 {
            return Err(WaveError::InvalidMesh(
                "the vector reconstruction is singular",
            ));
        }
        matrix.swap(pivot, best);
        right.swap(pivot, best);
        let pivot_values = matrix[pivot];
        for row in pivot + 1..6 {
            let factor = matrix[row][pivot] / matrix[pivot][pivot];
            for (column, value) in matrix[row].iter_mut().enumerate().skip(pivot) {
                *value -= factor * pivot_values[column];
            }
            right[row] -= factor * right[pivot];
        }
    }
    let mut solution = [0.0; 6];
    for row in (0..6).rev() {
        solution[row] = (right[row]
            - (row + 1..6)
                .map(|column| matrix[row][column] * solution[column])
                .sum::<f64>())
            / matrix[row][row];
    }
    if solution.iter().all(|value| value.is_finite()) {
        Ok(solution)
    } else {
        Err(WaveError::InvalidState)
    }
}

/// Gate O: what became of the integrated field `r = ∫u dt` across a handoff.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CanonicalIntegratedFieldTransfer {
    /// The target's `r`, one value per target node, or empty when the target
    /// carries no restoring law.
    pub field: Vec<f64>,
    /// Target nodes the source does not cover, which start at `r = 0`.
    pub exposed_nodes: usize,
    /// The source held an `r` and the target has no restoring law to keep it.
    pub discarded: bool,
    /// The target has a restoring law and the source had no `r` (a static or
    /// driven generation), so the target starts from `r = 0` everywhere.
    pub started_at_zero: bool,
}

/// Hands the integrated field to the next generation.
///
/// `r` is a nodal field, not an integrated quantity like `Q`, so it is
/// interpolated as the displayed field is, through the same quadratic map;
/// nothing about it is conserved. Its uniform part is physical for every
/// restoring law, so it is carried rather than rebuilt from `b`. A node the
/// source does not cover starts at `r = 0`: the vacuum of Klein-Gordon and
/// sine-Gordon, the unstable top of φ⁴. The report says when a target drops
/// an `r` it has no law for, and when one starts from zero because its
/// source had none.
pub fn transfer_integrated_field(
    interpolation: &QuadraticTransferMap,
    source_field: &[f64],
    target: &CanonicalTemporalWaveOperator,
) -> Result<CanonicalIntegratedFieldTransfer, WaveError> {
    let target_nodes = target.base().degrees_of_freedom();
    if interpolation.samples().len() != target_nodes {
        return Err(WaveError::SizeMismatch {
            expected: target_nodes,
            actual: interpolation.samples().len(),
        });
    }
    if !target.has_restoring() {
        return Ok(CanonicalIntegratedFieldTransfer {
            discarded: !source_field.is_empty(),
            ..CanonicalIntegratedFieldTransfer::default()
        });
    }
    if source_field.is_empty() {
        return Ok(CanonicalIntegratedFieldTransfer {
            field: vec![0.0; target_nodes],
            started_at_zero: true,
            ..CanonicalIntegratedFieldTransfer::default()
        });
    }
    if source_field.len() != interpolation.source_dofs() {
        return Err(WaveError::SizeMismatch {
            expected: interpolation.source_dofs(),
            actual: source_field.len(),
        });
    }
    let field = interpolation
        .interpolate(source_field, 0.0)
        .map_err(|_| WaveError::InvalidState)?;
    Ok(CanonicalIntegratedFieldTransfer {
        field,
        exposed_nodes: interpolation.exposed_nodes(),
        ..CanonicalIntegratedFieldTransfer::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BACKGROUND_REGION, BoundaryEdge, BoundaryLabel, CanonicalWaveOperator, InternalBoundaryId,
        MeshQuality, MeshTriangle, MeshVertex, MeshingOptions, OuterBoundaryCondition, OuterSide,
        QuadraticWaveOperator, Scene, mesh_scene,
    };

    fn mesh(revision: u64, points: &[[f64; 2]], triangles: &[[usize; 3]]) -> TriMesh {
        TriMesh {
            geometry_revision: 5,
            mesh_revision: revision,
            vertices: points
                .iter()
                .map(|point| MeshVertex {
                    point: Point2::new(point[0], point[1]),
                    boundary: None,
                    trace: None,
                })
                .collect(),
            triangles: triangles
                .iter()
                .map(|vertices| MeshTriangle {
                    vertices: *vertices,
                    region: BACKGROUND_REGION,
                })
                .collect(),
            boundary_edges: vec![],
            quality: MeshQuality {
                minimum_angle_degrees: 45.0,
                maximum_edge_length: 2.0_f64.sqrt(),
            },
            requested_sizes: vec![],
        }
    }

    fn compile(mesh: &TriMesh) -> (QuadraticWaveOperator, CanonicalWaveOperator) {
        let scene = Scene::default();
        compile_scene(mesh, &scene)
    }

    fn compile_scene(
        mesh: &TriMesh,
        scene: &Scene,
    ) -> (QuadraticWaveOperator, CanonicalWaveOperator) {
        let quadratic =
            QuadraticWaveOperator::assemble_scene(mesh, scene, OuterBoundaryCondition::Reflecting)
                .unwrap();
        let canonical = CanonicalWaveOperator::compile_scene(mesh, &quadratic, scene, 1).unwrap();
        (quadratic, canonical)
    }

    fn weighted_nodal_error(
        flux: &[f64],
        operator: &CanonicalWaveOperator,
        expected: impl Fn(Point2) -> f64,
    ) -> f64 {
        let (error, scale) = flux
            .iter()
            .zip(operator.geometric_support())
            .zip(operator.node_points())
            .fold((0.0, 0.0), |(error, scale), ((flux, support), point)| {
                let actual = flux / support;
                let expected = expected(*point);
                (
                    error + support * (actual - expected).powi(2),
                    scale + support * expected.powi(2),
                )
            });
        (error / scale).sqrt()
    }

    fn weighted_vector_error(
        values: &[Point2],
        operator: &CanonicalWaveOperator,
        expected: impl Fn(Point2) -> Point2,
    ) -> f64 {
        let (error, scale) = values.iter().zip(operator.constitutive_samples()).fold(
            (0.0, 0.0),
            |(error, scale), (actual, sample)| {
                let expected = expected(sample.point);
                (
                    error
                        + sample.integration_weight * (*actual - expected).dot(*actual - expected),
                    scale + sample.integration_weight * expected.dot(expected),
                )
            },
        );
        (error / scale).sqrt()
    }

    fn compile_with_boundary(
        mesh: &TriMesh,
        boundary: OuterBoundaryCondition,
    ) -> (QuadraticWaveOperator, CanonicalWaveOperator) {
        let scene = Scene::default();
        let quadratic = QuadraticWaveOperator::assemble_scene(mesh, &scene, boundary).unwrap();
        let canonical = CanonicalWaveOperator::compile_scene(mesh, &quadratic, &scene, 1).unwrap();
        (quadratic, canonical)
    }

    fn bounded_square(revision: u64) -> TriMesh {
        let mut mesh = mesh(
            revision,
            &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            &[[0, 1, 2], [0, 2, 3]],
        );
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
    fn unchanged_scalar_and_vector_samples_copy_bit_exactly() {
        let mesh = mesh(
            1,
            &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            &[[0, 1, 2], [0, 2, 3]],
        );
        let (quadratic, canonical) = compile(&mesh);
        let nodal = QuadraticTransferMap::identity_on_mesh(&mesh, &quadratic, &quadratic).unwrap();
        let primary = CanonicalPrimaryTransferMap::prepare(&nodal, &canonical, &canonical).unwrap();
        assert!(primary.is_identity());
        assert!(primary.samples.is_empty());
        let q = (0..canonical.degrees_of_freedom())
            .map(|index| (index as f64 + 0.25).sin())
            .collect::<Vec<_>>();
        let totals = vec![Some(q.iter().sum())];
        let (copied, report) = primary
            .transfer(&q, &totals, &vec![false; q.len()])
            .unwrap();
        assert_eq!(copied, q);
        assert_eq!(report.maximum_correction_ratio, 0.0);

        let vector =
            CanonicalVectorTransferMap::prepare(&mesh, &canonical, &mesh, &canonical).unwrap();
        assert!(vector.is_identity());
        assert!(vector.targets.is_empty());
        let b = canonical
            .constitutive_samples()
            .iter()
            .map(|sample| Point2::new(sample.point.x.exp(), sample.point.y.sin()))
            .collect::<Vec<_>>();
        let (copied, report) = vector.transfer(&b).unwrap();
        assert_eq!(copied, b);
        assert_eq!(report.exposed_values, 0);

        let mut republished = mesh.clone();
        republished.geometry_revision += 1;
        republished.mesh_revision += 1;
        let (_, republished_canonical) = compile(&republished);
        let vector = CanonicalVectorTransferMap::prepare(
            &mesh,
            &canonical,
            &republished,
            &republished_canonical,
        )
        .unwrap();
        assert!(vector.is_identity());
        assert!(vector.targets.is_empty());
        assert_eq!(
            vector.exact_samples(),
            canonical.complementary_degrees_of_freedom(),
            "transaction revisions do not make an unchanged discretization non-identity",
        );

        let mut moved = mesh.clone();
        moved.mesh_revision = 3;
        moved.vertices[2].point = Point2::new(0.9, 0.9);
        let (_, moved_canonical) = compile(&moved);
        let vector =
            CanonicalVectorTransferMap::prepare(&mesh, &canonical, &moved, &moved_canonical)
                .unwrap();
        assert!(!vector.is_identity());
        assert_eq!(
            vector.exact_samples(),
            0,
            "equal connectivity is not an identity transfer after coordinates move",
        );
    }

    #[test]
    fn quadratic_vector_reconstruction_is_exact_across_refinement() {
        let coarse = mesh(
            1,
            &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            &[[0, 1, 2], [0, 2, 3]],
        );
        let fine = mesh(
            2,
            &[
                [0.0, 0.0],
                [0.5, 0.0],
                [1.0, 0.0],
                [0.0, 0.5],
                [0.5, 0.5],
                [1.0, 0.5],
                [0.0, 1.0],
                [0.5, 1.0],
                [1.0, 1.0],
            ],
            &[
                [0, 1, 4],
                [0, 4, 3],
                [1, 2, 5],
                [1, 5, 4],
                [3, 4, 7],
                [3, 7, 6],
                [4, 5, 8],
                [4, 8, 7],
            ],
        );
        let (_, source) = compile(&coarse);
        let (_, target) = compile(&fine);
        let polynomial = |point: Point2| {
            Point2::new(
                0.2 + point.x - 0.4 * point.y + 0.7 * point.x * point.y,
                -0.3 + 0.2 * point.x * point.x + 0.5 * point.y * point.y,
            )
        };
        let old = source
            .constitutive_samples()
            .iter()
            .map(|sample| polynomial(sample.point))
            .collect::<Vec<_>>();
        let map = CanonicalVectorTransferMap::prepare(&coarse, &source, &fine, &target).unwrap();
        let mut job = CanonicalVectorTransferJob::new(
            Arc::new(coarse.clone()),
            Arc::new(source.clone()),
            Arc::new(fine.clone()),
            Arc::new(target.clone()),
        );
        let mut slices = 0;
        let prepared = loop {
            slices += 1;
            if let Some(result) = job.advance(3) {
                break result.unwrap();
            }
        };
        assert!(slices > 1);
        assert_eq!(prepared, map);
        let (new, report) = map.transfer(&old).unwrap();
        assert_eq!(report.exposed_values, 0);
        let error = new
            .iter()
            .zip(target.constitutive_samples())
            .map(|(actual, sample)| (*actual - polynomial(sample.point)).norm())
            .fold(0.0, f64::max);
        assert!(error < 2.0e-13, "quadratic transfer error {error:e}");
    }

    #[test]
    fn reused_operator_complementary_identity_finishes_in_one_work_unit() {
        let mesh = Arc::new(mesh(
            1,
            &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            &[[0, 1, 2], [0, 2, 3]],
        ));
        let (_, operator) = compile(&mesh);
        let operator = Arc::new(operator);
        let mut job =
            CanonicalVectorTransferJob::new(mesh.clone(), operator.clone(), mesh, operator);

        let map = job.advance(1).unwrap().unwrap();
        assert!(map.is_identity());
        assert_eq!(job.phase(), "Finished");
    }

    #[test]
    fn support_aware_q_transfer_conserves_totals_through_repeated_mesh_cycles() {
        let coarse = mesh(
            1,
            &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            &[[0, 1, 2], [0, 2, 3]],
        );
        let fine = mesh(
            2,
            &[
                [0.0, 0.0],
                [0.5, 0.0],
                [1.0, 0.0],
                [0.0, 0.5],
                [0.5, 0.5],
                [1.0, 0.5],
                [0.0, 1.0],
                [0.5, 1.0],
                [1.0, 1.0],
            ],
            &[
                [0, 1, 4],
                [0, 4, 3],
                [1, 2, 5],
                [1, 5, 4],
                [3, 4, 7],
                [3, 7, 6],
                [4, 5, 8],
                [4, 8, 7],
            ],
        );
        let (coarse_quadratic, coarse_operator) = compile(&coarse);
        let (fine_quadratic, fine_operator) = compile(&fine);
        let to_fine =
            QuadraticTransferMap::build(&coarse, &coarse_quadratic, &fine, &fine_quadratic)
                .unwrap();
        let to_coarse =
            QuadraticTransferMap::build(&fine, &fine_quadratic, &coarse, &coarse_quadratic)
                .unwrap();
        let to_fine =
            CanonicalPrimaryTransferMap::prepare(&to_fine, &coarse_operator, &fine_operator)
                .unwrap();
        let to_coarse =
            CanonicalPrimaryTransferMap::prepare(&to_coarse, &fine_operator, &coarse_operator)
                .unwrap();
        let initial = coarse_operator
            .node_points()
            .iter()
            .zip(coarse_operator.geometric_support())
            .map(|(point, support)| support * (1.0 + 0.1 * point.x - 0.08 * point.y))
            .collect::<Vec<_>>();
        let expected_total = initial.iter().sum::<f64>();
        let mut coarse_q = initial.clone();
        for _ in 0..12 {
            let (fine_q, fine_report) = to_fine
                .transfer(
                    &coarse_q,
                    &[Some(expected_total)],
                    &vec![false; fine_operator.degrees_of_freedom()],
                )
                .unwrap();
            assert!(fine_report.maximum_correction_ratio <= CORRECTION_LIMIT);
            let (next, coarse_report) = to_coarse
                .transfer(
                    &fine_q,
                    &[Some(expected_total)],
                    &vec![false; coarse_operator.degrees_of_freedom()],
                )
                .unwrap();
            assert!(coarse_report.maximum_correction_ratio <= CORRECTION_LIMIT);
            coarse_q = next;
        }
        let error = coarse_q
            .iter()
            .zip(&initial)
            .map(|(actual, expected)| (actual - expected).powi(2))
            .sum::<f64>()
            .sqrt()
            / initial
                .iter()
                .map(|value| value * value)
                .sum::<f64>()
                .sqrt();
        assert!(error < 0.05, "repeated scalar transfer error {error:e}");
        assert!((coarse_q.iter().sum::<f64>() - expected_total).abs() < 1.0e-11);
    }

    #[test]
    fn connected_new_support_gets_bounded_constant_extension_but_new_island_stays_zero() {
        let source_mesh = mesh(1, &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], &[[0, 1, 2]]);
        let target_mesh = mesh(
            2,
            &[
                [0.0, 0.0],
                [1.0, 0.0],
                [0.0, 1.0],
                [1.0, 1.0],
                [3.0, 0.0],
                [4.0, 0.0],
                [3.0, 1.0],
            ],
            &[[0, 1, 2], [1, 3, 2], [4, 5, 6]],
        );
        let (source_quadratic, source) = compile(&source_mesh);
        let (target_quadratic, target) = compile(&target_mesh);
        let interpolation = QuadraticTransferMap::build(
            &source_mesh,
            &source_quadratic,
            &target_mesh,
            &target_quadratic,
        )
        .unwrap();
        let transfer = CanonicalPrimaryTransferMap::prepare_with_meshes(
            &interpolation,
            &source_mesh,
            &source,
            &target_mesh,
            &target,
        )
        .unwrap();
        let source_q = source
            .geometric_support()
            .iter()
            .map(|support| 2.0 * support)
            .collect::<Vec<_>>();
        let (target_q, report) = transfer
            .transfer(
                &source_q,
                &[None, None],
                &vec![false; target.degrees_of_freedom()],
            )
            .unwrap();
        assert!(report.extended_values > 0);
        assert!(report.exposed_values > 0);
        for (element, nodes) in target.element_nodes().iter().enumerate() {
            for node in nodes {
                let density = target_q[*node as usize] / target.geometric_support()[*node as usize];
                if element < 2 {
                    assert!((density - 2.0).abs() < 2.0e-13);
                } else {
                    assert_eq!(density, 0.0);
                }
            }
        }

        let vector =
            CanonicalVectorTransferMap::prepare(&source_mesh, &source, &target_mesh, &target)
                .unwrap();
        let source_b = vec![Point2::new(0.4, -0.2); source.complementary_degrees_of_freedom()];
        let (target_b, vector_report) = vector.transfer(&source_b).unwrap();
        assert!(vector_report.extended_values > 0);
        assert!(vector_report.exposed_values > 0);
        for (element, samples) in target_b.chunks_exact(QUADRATURE_SAMPLES).enumerate() {
            if element < 2 {
                assert!(
                    samples
                        .iter()
                        .all(|value| (*value - Point2::new(0.4, -0.2)).norm() < 2.0e-13)
                );
            } else {
                assert!(samples.iter().all(|value| *value == Point2::default()));
            }
        }
    }

    #[test]
    fn irregular_production_meshes_meet_one_handoff_and_repeated_transfer_limits() {
        let scene = Scene::initial();
        let coarse_mesh = mesh_scene(
            &scene,
            41,
            MeshingOptions {
                target_edge_length: 0.28,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let fine_mesh = mesh_scene(
            &scene,
            42,
            MeshingOptions {
                target_edge_length: 0.18,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        let (coarse_quadratic, coarse) = compile_scene(&coarse_mesh, &scene);
        let (fine_quadratic, fine) = compile_scene(&fine_mesh, &scene);
        let coarse_to_fine = QuadraticTransferMap::build(
            &coarse_mesh,
            &coarse_quadratic,
            &fine_mesh,
            &fine_quadratic,
        )
        .unwrap();
        let fine_to_coarse = QuadraticTransferMap::build(
            &fine_mesh,
            &fine_quadratic,
            &coarse_mesh,
            &coarse_quadratic,
        )
        .unwrap();
        let q_to_fine = CanonicalPrimaryTransferMap::prepare_with_meshes(
            &coarse_to_fine,
            &coarse_mesh,
            &coarse,
            &fine_mesh,
            &fine,
        )
        .unwrap();
        let q_to_coarse = CanonicalPrimaryTransferMap::prepare_with_meshes(
            &fine_to_coarse,
            &fine_mesh,
            &fine,
            &coarse_mesh,
            &coarse,
        )
        .unwrap();
        let b_to_fine =
            CanonicalVectorTransferMap::prepare(&coarse_mesh, &coarse, &fine_mesh, &fine).unwrap();
        let b_to_coarse =
            CanonicalVectorTransferMap::prepare(&fine_mesh, &fine, &coarse_mesh, &coarse).unwrap();
        let q_field = |point: Point2| 1.0 + 0.04 * (2.1 * point.x).sin() * (1.7 * point.y).cos();
        let b_field = |point: Point2| {
            Point2::new(
                0.3 + 0.05 * (1.9 * point.x + 0.3 * point.y).sin(),
                -0.2 + 0.04 * (0.4 * point.x - 1.6 * point.y).cos(),
            )
        };
        let initial_q = coarse
            .node_points()
            .iter()
            .zip(coarse.geometric_support())
            .map(|(point, support)| support * q_field(*point))
            .collect::<Vec<_>>();
        let initial_b = coarse
            .constitutive_samples()
            .iter()
            .map(|sample| b_field(sample.point))
            .collect::<Vec<_>>();
        let total = initial_q.iter().sum::<f64>();
        let (fine_q, _) = q_to_fine
            .transfer(
                &initial_q,
                &[Some(total)],
                &vec![false; fine.degrees_of_freedom()],
            )
            .unwrap();
        let (fine_b, _) = b_to_fine.transfer(&initial_b).unwrap();
        let q_error = weighted_nodal_error(&fine_q, &fine, q_field);
        let b_error = weighted_vector_error(&fine_b, &fine, b_field);
        assert!(q_error <= 0.03, "irregular Q handoff error {q_error:e}");
        assert!(b_error <= 0.03, "irregular b handoff error {b_error:e}");

        let mut coarse_q = initial_q.clone();
        let mut coarse_b = initial_b.clone();
        for _ in 0..12 {
            let (fine_q, _) = q_to_fine
                .transfer(
                    &coarse_q,
                    &[Some(total)],
                    &vec![false; fine.degrees_of_freedom()],
                )
                .unwrap();
            let (fine_b, _) = b_to_fine.transfer(&coarse_b).unwrap();
            coarse_q = q_to_coarse
                .transfer(
                    &fine_q,
                    &[Some(total)],
                    &vec![false; coarse.degrees_of_freedom()],
                )
                .unwrap()
                .0;
            coarse_b = b_to_coarse.transfer(&fine_b).unwrap().0;
        }
        let q_error = weighted_nodal_error(&coarse_q, &coarse, q_field);
        let b_error = weighted_vector_error(&coarse_b, &coarse, b_field);
        assert!(q_error <= 0.05, "repeated irregular Q error {q_error:e}");
        assert!(b_error <= 0.05, "repeated irregular b error {b_error:e}");

        let arbitrary = coarse
            .constitutive_samples()
            .iter()
            .enumerate()
            .map(|(index, sample)| {
                Point2::new(
                    (0.17 * index as f64 + sample.point.x).sin(),
                    (0.11 * index as f64 - sample.point.y).cos(),
                )
            })
            .collect::<Vec<_>>();
        let stationary = coarse
            .stationary_complementary_component(&arbitrary)
            .unwrap();
        let identity =
            CanonicalVectorTransferMap::prepare(&coarse_mesh, &coarse, &coarse_mesh, &coarse)
                .unwrap();
        let copied = identity.transfer(&stationary).unwrap().0;
        let force = coarse.force(&copied).unwrap();
        let leakage = force.iter().map(|value| value * value).sum::<f64>().sqrt();
        let scale = stationary
            .iter()
            .map(|value| value.dot(*value))
            .sum::<f64>()
            .sqrt()
            .max(1.0);
        assert!(leakage / scale <= 1.0e-3);
    }

    #[test]
    fn thin_gap_history_handles_split_reverse_new_and_deleted_energy() {
        let sample = |start: f64, end: f64, local: u8, stiffness: f64| ThinGapSample {
            key: ThinGapTraceKey::Legacy {
                boundary: InternalBoundaryId(4),
                start_bits: start.to_bits(),
                end_bits: end.to_bits(),
                local,
            },
            left_node: local as u32,
            right_node: local as u32 + 10,
            stiffness,
        };
        let source = (0..3)
            .map(|local| sample(0.0, 1.0, local, 1.0))
            .collect::<Vec<_>>();
        let split = [(0.0, 0.5), (0.5, 1.0)]
            .into_iter()
            .flat_map(|(start, end)| (0..3).map(move |local| sample(start, end, local, 0.5)))
            .collect::<Vec<_>>();
        let memory = [0.0_f64, 0.5, 1.0]
            .into_iter()
            .zip(&source)
            .map(|(jump, sample)| CanonicalThinGapMemory {
                key: sample.key,
                jump,
            })
            .collect::<Vec<_>>();
        let map = CanonicalThinGapHistoryTransferMap::prepare(&source, &split).unwrap();
        let (transferred, report) = map.transfer(&memory).unwrap();
        let expected = [0.0, 0.25, 0.5, 0.5, 0.75, 1.0];
        assert!(
            transferred
                .iter()
                .zip(expected)
                .all(|(actual, expected)| (actual.jump - expected).abs() < 2.0e-15)
        );
        assert_eq!(report.new_samples, 0);
        assert_eq!(report.deleted_samples, 0);

        let reversed = (0..3)
            .map(|local| sample(1.0, 0.0, local, 1.0))
            .collect::<Vec<_>>();
        let reverse_map = CanonicalThinGapHistoryTransferMap::prepare(&source, &reversed).unwrap();
        let (transferred, _) = reverse_map.transfer(&memory).unwrap();
        assert_eq!(
            transferred
                .iter()
                .map(|entry| entry.jump)
                .collect::<Vec<_>>(),
            vec![-1.0, -0.5, 0.0]
        );

        // A reconnected baffle tip can omit the coincident endpoint lump. The
        // remaining samples still provide a well-defined linear history map.
        let tip_source = source[1..].to_vec();
        let tip_memory = memory[1..].to_vec();
        let tip_target = vec![sample(0.5, 1.0, 1, 1.0)];
        let tip_map =
            CanonicalThinGapHistoryTransferMap::prepare(&tip_source, &tip_target).unwrap();
        let (tip_transferred, _) = tip_map.transfer(&tip_memory).unwrap();
        assert!((tip_transferred[0].jump - 0.75).abs() < 2.0e-15);

        let deleted = CanonicalThinGapHistoryTransferMap::prepare(&source, &[]).unwrap();
        let (_, report) = deleted.transfer(&memory).unwrap();
        assert_eq!(report.deleted_samples, 3);
        assert_eq!(report.edit_exchange, -report.source_energy);
    }

    #[test]
    fn outgoing_history_maps_physical_trace_and_accounts_creation_and_deletion() {
        let mesh = bounded_square(3);
        let (second_quadratic, second) =
            compile_with_boundary(&mesh, OuterBoundaryCondition::SecondOrderOutgoing);
        let (reflecting_quadratic, reflecting) =
            compile_with_boundary(&mesh, OuterBoundaryCondition::Reflecting);
        let identity =
            QuadraticTransferMap::identity_on_mesh(&mesh, &second_quadratic, &second_quadratic)
                .unwrap();
        let boundary = second.outgoing_boundary().unwrap();
        let normalized = (0..boundary.auxiliary_count())
            .map(|index| (0.31 * index as f64 + 0.4).sin())
            .collect::<Vec<_>>();
        let physical = boundary.physical_memory(&normalized).unwrap();
        let map =
            CanonicalOutgoingHistoryTransferMap::prepare(&identity, &second, &second).unwrap();
        let normalized_map = map.normalized_matrix(&second, &second).unwrap();
        assert!(normalized_map.identity);
        let mapped = normalized.clone();
        assert!(
            mapped
                .iter()
                .zip(&normalized)
                .all(|(actual, expected)| (actual - expected).abs() < 3.0e-12)
        );
        let (copied, report) = map.transfer(&second, &second, Some(&physical)).unwrap();
        assert_eq!(copied.unwrap(), physical);
        assert!(report.physical_residual_norm < 2.0e-10);
        assert!(report.edit_exchange.abs() < 2.0e-10 * report.source_energy.max(1.0));

        let mut changed_scene = Scene::default();
        changed_scene.materials[0].mass_density = crate::ScalarField::constant(1.7);
        changed_scene.materials[0].stiffness = crate::ScalarField::constant(2.3);
        let changed_quadratic = QuadraticWaveOperator::assemble_scene(
            &mesh,
            &changed_scene,
            OuterBoundaryCondition::SecondOrderOutgoing,
        )
        .unwrap();
        let changed =
            CanonicalWaveOperator::compile_scene(&mesh, &changed_quadratic, &changed_scene, 2)
                .unwrap();
        let changed_identity =
            QuadraticTransferMap::identity_on_mesh(&mesh, &second_quadratic, &changed_quadratic)
                .unwrap();
        let changed_map =
            CanonicalOutgoingHistoryTransferMap::prepare(&changed_identity, &second, &changed)
                .unwrap();
        let changed_normalized = changed_map.normalized_matrix(&second, &changed).unwrap();
        assert!(!changed_normalized.identity);
        assert_eq!(
            changed_normalized.values.len(),
            changed_normalized.source_count * changed_normalized.target_count
        );
        let mut normalized_job = CanonicalOutgoingNormalizedTransferJob::new(
            Arc::new(changed_map.clone()),
            Arc::new(second.clone()),
            Arc::new(changed.clone()),
        );
        let mut slices = 0;
        let prepared_normalized = loop {
            slices += 1;
            if let Some(result) = normalized_job.advance(7) {
                break result.unwrap();
            }
        };
        assert!(slices > 1);
        assert_eq!(prepared_normalized, changed_normalized);
        let (changed_physical, report) = changed_map
            .transfer(&second, &changed, Some(&physical))
            .unwrap();
        assert_eq!(changed_physical.unwrap(), physical);
        assert!(report.edit_exchange.is_finite());
        assert!(report.physical_residual_norm < 2.0e-10);

        let remove_identity =
            QuadraticTransferMap::identity_on_mesh(&mesh, &second_quadratic, &reflecting_quadratic)
                .unwrap();
        let remove =
            CanonicalOutgoingHistoryTransferMap::prepare(&remove_identity, &second, &reflecting)
                .unwrap();
        let (none, report) = remove
            .transfer(&second, &reflecting, Some(&physical))
            .unwrap();
        assert!(none.is_none());
        assert_eq!(report.edit_exchange, -report.source_energy);

        let create_identity =
            QuadraticTransferMap::identity_on_mesh(&mesh, &reflecting_quadratic, &second_quadratic)
                .unwrap();
        let create =
            CanonicalOutgoingHistoryTransferMap::prepare(&create_identity, &reflecting, &second)
                .unwrap();
        let (created, report) = create.transfer(&reflecting, &second, None).unwrap();
        assert!(
            created
                .unwrap()
                .pole_currents
                .iter()
                .flatten()
                .all(|value| *value == 0.0)
        );
        assert_eq!(report.target_energy, 0.0);
        assert!(report.new_samples > 0);
    }
}
