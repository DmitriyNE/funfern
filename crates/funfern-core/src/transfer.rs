use std::collections::{BTreeMap, BTreeSet};

use crate::{
    BoundaryLabel, CurveId, CurveTraceSide, InternalBoundaryId, InternalBoundarySide, MeshVertex,
    Point2, QuadraticWaveOperator, TraceVertexId, TriMesh, enriched_quadratic_basis,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum BoundaryTraceKey {
    Internal {
        id: InternalBoundaryId,
        right: bool,
    },
    Curve {
        curve: CurveId,
        side: CurveTraceSide,
    },
}

impl BoundaryTraceKey {
    fn from_label(label: BoundaryLabel) -> Option<Self> {
        match label {
            BoundaryLabel::InternalBoundary { id, side } => Some(Self::Internal {
                id,
                right: side == InternalBoundarySide::Right,
            }),
            BoundaryLabel::Curve {
                curve,
                side,
                separated: true,
                ..
            } => Some(Self::Curve { curve, side }),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct TargetTracePreference {
    vertex: Option<TraceVertexId>,
    boundaries: BTreeSet<BoundaryTraceKey>,
}

#[derive(Clone, Debug, Default)]
struct SourceTraceTriangles {
    vertices: BTreeMap<TraceVertexId, BTreeSet<u32>>,
    boundaries: BTreeMap<BoundaryTraceKey, BTreeSet<u32>>,
}

#[derive(Clone, Copy)]
struct TraceRestrictions<'a> {
    target: &'a [TargetTracePreference],
    source: &'a SourceTraceTriangles,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransferSample {
    pub vertices: [u32; 3],
    pub weights: [f64; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransferMap {
    source_revision: u64,
    target_revision: u64,
    source_mesh_revision: u64,
    target_mesh_revision: u64,
    source_vertices: usize,
    samples: Vec<Option<TransferSample>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransferError {
    EmptySource,
    InvalidSource,
    InvalidTarget,
    SizeMismatch { expected: usize, actual: usize },
    NonFiniteValues,
}

impl std::fmt::Display for TransferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptySource => write!(f, "The source mesh has no triangles"),
            Self::InvalidSource => write!(f, "The source mesh is invalid for state transfer"),
            Self::InvalidTarget => write!(f, "The target mesh is invalid for state transfer"),
            Self::SizeMismatch { expected, actual } => {
                write!(f, "Expected {expected} source values, received {actual}")
            }
            Self::NonFiniteValues => write!(f, "The source field contains non-finite values"),
        }
    }
}

impl std::error::Error for TransferError {}

impl TransferMap {
    /// Locates every target vertex in the source triangulation. Vertices outside
    /// the old domain are deliberately left unmapped and initialize to zero.
    pub fn build(source: &TriMesh, target: &TriMesh) -> Result<Self, TransferError> {
        Self::build_restricted(
            source,
            target,
            &vec![None; target.vertices.len()],
            None,
            None,
        )
    }

    fn build_restricted(
        source: &TriMesh,
        target: &TriMesh,
        target_groups: &[Option<crate::RegionId>],
        region_groups: Option<&BTreeMap<crate::RegionId, crate::RegionId>>,
        trace_restrictions: Option<TraceRestrictions<'_>>,
    ) -> Result<Self, TransferError> {
        if source.triangles.is_empty() {
            return Err(TransferError::EmptySource);
        }
        validate_mesh(source, true)?;
        validate_mesh(target, false)?;
        if target_groups.len() != target.vertices.len() {
            return Err(TransferError::InvalidTarget);
        }
        if trace_restrictions.is_some_and(|traces| traces.target.len() != target.vertices.len()) {
            return Err(TransferError::InvalidTarget);
        }

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
            return Err(TransferError::InvalidSource);
        }
        let dimension = (source.triangles.len() as f64).sqrt().ceil() as usize;
        let dimension = dimension.clamp(8, 512);
        let mut bins = vec![Vec::<u32>::new(); dimension * dimension];
        let cell = Point2::new(extent.x / dimension as f64, extent.y / dimension as f64);
        for (triangle_index, triangle) in source.triangles.iter().enumerate() {
            let points = triangle.vertices.map(|i| source.vertices[i].point);
            let lo = Point2::new(
                points.iter().map(|p| p.x).fold(f64::INFINITY, f64::min),
                points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min),
            );
            let hi = Point2::new(
                points.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max),
                points.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max),
            );
            let [x0, y0] = bin_index(lo, minimum, cell, dimension);
            let [x1, y1] = bin_index(hi, minimum, cell, dimension);
            let triangle_index =
                u32::try_from(triangle_index).map_err(|_| TransferError::InvalidSource)?;
            for y in y0..=y1 {
                for x in x0..=x1 {
                    bins[y * dimension + x].push(triangle_index);
                }
            }
        }

        let mut samples = Vec::with_capacity(target.vertices.len());
        for (target_index, (vertex, target_group)) in
            target.vertices.iter().zip(target_groups).enumerate()
        {
            let point = vertex.point;
            if point.x < minimum.x
                || point.x > maximum.x
                || point.y < minimum.y
                || point.y > maximum.y
            {
                samples.push(None);
                continue;
            }
            let [x, y] = bin_index(point, minimum, cell, dimension);
            let mut found = None;
            let preferred = trace_restrictions.and_then(|traces| {
                preferred_trace_triangles(&traces.target[target_index], traces.source)
            });
            for require_preferred in [true, false] {
                if require_preferred && preferred.is_none() {
                    continue;
                }
                for &triangle_index in &bins[y * dimension + x] {
                    if require_preferred
                        && preferred
                            .as_ref()
                            .is_some_and(|triangles| !triangles.contains(&triangle_index))
                    {
                        continue;
                    }
                    let triangle = source.triangles[triangle_index as usize];
                    if let Some(target_group) = target_group {
                        let source_group = region_groups
                            .and_then(|groups| groups.get(&triangle.region))
                            .copied()
                            .unwrap_or(triangle.region);
                        if source_group != *target_group {
                            continue;
                        }
                    }
                    let points = triangle.vertices.map(|i| source.vertices[i].point);
                    if let Some(weights) = barycentric(point, points) {
                        found = Some(TransferSample {
                            vertices: triangle.vertices.map(|i| i as u32),
                            weights,
                        });
                        break;
                    }
                }
                if found.is_some() {
                    break;
                }
            }
            samples.push(found);
        }
        Ok(Self {
            source_revision: source.geometry_revision,
            target_revision: target.geometry_revision,
            source_mesh_revision: source.mesh_revision,
            target_mesh_revision: target.mesh_revision,
            source_vertices: source.vertices.len(),
            samples,
        })
    }

    pub fn source_revision(&self) -> u64 {
        self.source_revision
    }

    pub fn target_revision(&self) -> u64 {
        self.target_revision
    }

    pub fn source_vertices(&self) -> usize {
        self.source_vertices
    }

    pub fn samples(&self) -> &[Option<TransferSample>] {
        &self.samples
    }

    pub fn matches_meshes(&self, source: &TriMesh, target: &TriMesh) -> bool {
        self.source_revision == source.geometry_revision
            && self.target_revision == target.geometry_revision
            && self.source_mesh_revision == source.mesh_revision
            && self.target_mesh_revision == target.mesh_revision
            && self.source_vertices == source.vertices.len()
            && self.samples.len() == target.vertices.len()
    }

    pub fn exposed_vertices(&self) -> usize {
        self.samples
            .iter()
            .filter(|sample| sample.is_none())
            .count()
    }

    pub fn interpolate(
        &self,
        source_values: &[f64],
        exposed_value: f64,
    ) -> Result<Vec<f64>, TransferError> {
        if source_values.len() != self.source_vertices {
            return Err(TransferError::SizeMismatch {
                expected: self.source_vertices,
                actual: source_values.len(),
            });
        }
        if !exposed_value.is_finite() || source_values.iter().any(|value| !value.is_finite()) {
            return Err(TransferError::NonFiniteValues);
        }
        Ok(self
            .samples
            .iter()
            .map(|sample| match sample {
                Some(sample) => sample
                    .vertices
                    .iter()
                    .zip(sample.weights)
                    .map(|(&index, weight)| source_values[index as usize] * weight)
                    .sum(),
                None => exposed_value,
            })
            .collect())
    }
}

fn preferred_trace_triangles(
    target: &TargetTracePreference,
    source: &SourceTraceTriangles,
) -> Option<BTreeSet<u32>> {
    let boundary_triangles = target
        .boundaries
        .iter()
        .filter_map(|boundary| source.boundaries.get(boundary))
        .flatten()
        .copied()
        .collect::<BTreeSet<_>>();
    if boundary_triangles.is_empty() {
        return None;
    }

    // Trace IDs identify a particular angular sector at a junction. They are
    // snapshot-local, so use them only to narrow a stable curve-side match. If
    // an edit changed trace numbering, the curve side remains the safe lineage.
    if let Some(vertex_triangles) = target
        .vertex
        .and_then(|vertex| source.vertices.get(&vertex))
    {
        let intersection = boundary_triangles
            .intersection(vertex_triangles)
            .copied()
            .collect::<BTreeSet<_>>();
        if !intersection.is_empty() {
            return Some(intersection);
        }
    }
    Some(boundary_triangles)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuadraticTransferSample {
    pub nodes: [u32; 7],
    pub weights: [f64; 7],
}

#[derive(Clone, Debug, PartialEq)]
pub struct QuadraticTransferMap {
    source_revision: u64,
    target_revision: u64,
    source_mesh_revision: u64,
    target_mesh_revision: u64,
    source_dofs: usize,
    samples: Vec<Option<QuadraticTransferSample>>,
}

impl QuadraticTransferMap {
    /// Builds an exact nodal copy between two operators whose quadratic node
    /// layouts come from the same parent mesh. This is used for coefficient or
    /// boundary-condition transactions that do not change geometry.
    pub fn identity_on_mesh(
        mesh: &TriMesh,
        source_operator: &QuadraticWaveOperator,
        target_operator: &QuadraticWaveOperator,
    ) -> Result<Self, TransferError> {
        validate_quadratic_pair(mesh, source_operator, true)?;
        validate_quadratic_pair(mesh, target_operator, false)?;
        if source_operator.node_points() != target_operator.node_points()
            || source_operator.element_nodes() != target_operator.element_nodes()
            || source_operator.degrees_of_freedom() > u32::MAX as usize
        {
            return Err(TransferError::InvalidTarget);
        }
        let samples = (0..source_operator.degrees_of_freedom())
            .map(|index| {
                let mut nodes = [0; 7];
                let mut weights = [0.0; 7];
                nodes[0] = index as u32;
                weights[0] = 1.0;
                Some(QuadraticTransferSample { nodes, weights })
            })
            .collect();
        Ok(Self {
            source_revision: mesh.geometry_revision,
            target_revision: mesh.geometry_revision,
            source_mesh_revision: mesh.mesh_revision,
            target_mesh_revision: mesh.mesh_revision,
            source_dofs: source_operator.degrees_of_freedom(),
            samples,
        })
    }

    /// Locates every target quadratic node in the source parent triangulation and
    /// evaluates the source element's enriched quadratic basis at that point.
    pub fn build(
        source_mesh: &TriMesh,
        source_operator: &QuadraticWaveOperator,
        target_mesh: &TriMesh,
        target_operator: &QuadraticWaveOperator,
    ) -> Result<Self, TransferError> {
        validate_quadratic_pair(source_mesh, source_operator, true)?;
        validate_quadratic_pair(target_mesh, target_operator, false)?;

        // The P1 locator only needs target points; its triangle validation remains
        // valid because a quadratic operator starts with the parent mesh vertices.
        let mut expanded_target = target_mesh.clone();
        expanded_target.vertices = target_operator
            .node_points()
            .iter()
            .map(|point| MeshVertex {
                point: *point,
                boundary: None,
                trace: None,
            })
            .collect();
        // Unified topology meshes carry exact separated curve sides and sector
        // trace lineage. Region IDs may legitimately appear or disappear when a
        // transmitting divider splits or merges a face, so spatial transfer must
        // not require the same RegionId on both revisions. Legacy meshes retain
        // their region-component restriction for closed two-sided walls.
        let unified_topology =
            has_unified_topology(source_mesh) || has_unified_topology(target_mesh);
        let region_groups = (!unified_topology)
            .then(|| region_components(target_mesh))
            .transpose()?;
        let mut target_groups = vec![None; target_operator.degrees_of_freedom()];
        if let Some(region_groups) = &region_groups {
            for (triangle, nodes) in target_mesh
                .triangles
                .iter()
                .zip(target_operator.element_nodes())
            {
                let group = *region_groups
                    .get(&triangle.region)
                    .ok_or(TransferError::InvalidTarget)?;
                for node in nodes {
                    let assigned = &mut target_groups[*node as usize];
                    if assigned.is_some_and(|assigned| assigned != group) {
                        return Err(TransferError::InvalidTarget);
                    }
                    *assigned = Some(group);
                }
            }
        }
        let target_traces = quadratic_trace_nodes(target_mesh, target_operator)?;
        let source_traces = source_trace_triangles(source_mesh)?;
        let located = TransferMap::build_restricted(
            source_mesh,
            &expanded_target,
            &target_groups,
            region_groups.as_ref(),
            Some(TraceRestrictions {
                target: &target_traces,
                source: &source_traces,
            }),
        )?;

        let elements = source_mesh
            .triangles
            .iter()
            .zip(source_operator.element_nodes())
            .map(|(triangle, nodes)| (triangle.vertices.map(|index| index as u32), *nodes))
            .collect::<std::collections::BTreeMap<_, _>>();
        let samples = located
            .samples()
            .iter()
            .map(|sample| {
                let Some(sample) = sample else {
                    return Ok(None);
                };
                elements
                    .get(&sample.vertices)
                    .copied()
                    .map(|nodes| {
                        Some(QuadraticTransferSample {
                            nodes,
                            weights: enriched_quadratic_basis(sample.weights),
                        })
                    })
                    .ok_or(TransferError::InvalidSource)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            source_revision: source_mesh.geometry_revision,
            target_revision: target_mesh.geometry_revision,
            source_mesh_revision: source_mesh.mesh_revision,
            target_mesh_revision: target_mesh.mesh_revision,
            source_dofs: source_operator.degrees_of_freedom(),
            samples,
        })
    }

    pub fn source_dofs(&self) -> usize {
        self.source_dofs
    }

    pub fn samples(&self) -> &[Option<QuadraticTransferSample>] {
        &self.samples
    }

    pub fn matches(
        &self,
        source_mesh: &TriMesh,
        source_operator: &QuadraticWaveOperator,
        target_mesh: &TriMesh,
        target_operator: &QuadraticWaveOperator,
    ) -> bool {
        self.source_revision == source_mesh.geometry_revision
            && self.target_revision == target_mesh.geometry_revision
            && self.source_mesh_revision == source_mesh.mesh_revision
            && self.target_mesh_revision == target_mesh.mesh_revision
            && self.source_dofs == source_operator.degrees_of_freedom()
            && self.samples.len() == target_operator.degrees_of_freedom()
            && source_operator.geometry_revision() == source_mesh.geometry_revision
            && target_operator.geometry_revision() == target_mesh.geometry_revision
    }

    pub fn exposed_nodes(&self) -> usize {
        self.samples
            .iter()
            .filter(|sample| sample.is_none())
            .count()
    }

    pub fn interpolate(
        &self,
        source_values: &[f64],
        exposed_value: f64,
    ) -> Result<Vec<f64>, TransferError> {
        if source_values.len() != self.source_dofs {
            return Err(TransferError::SizeMismatch {
                expected: self.source_dofs,
                actual: source_values.len(),
            });
        }
        if !exposed_value.is_finite() || source_values.iter().any(|value| !value.is_finite()) {
            return Err(TransferError::NonFiniteValues);
        }
        Ok(self
            .samples
            .iter()
            .map(|sample| match sample {
                Some(sample) => sample
                    .nodes
                    .iter()
                    .zip(sample.weights)
                    .map(|(&index, weight)| source_values[index as usize] * weight)
                    .sum(),
                None => exposed_value,
            })
            .collect())
    }
}

fn quadratic_trace_nodes(
    mesh: &TriMesh,
    operator: &QuadraticWaveOperator,
) -> Result<Vec<TargetTracePreference>, TransferError> {
    let mut traces = vec![TargetTracePreference::default(); operator.degrees_of_freedom()];
    for (vertex_index, vertex) in mesh.vertices.iter().enumerate() {
        traces[vertex_index].vertex = vertex.trace;
        if let Some(boundary) = vertex
            .boundary
            .and_then(|boundary| BoundaryTraceKey::from_label(boundary.label))
        {
            traces[vertex_index].boundaries.insert(boundary);
        }
    }
    let owners = triangle_edge_owners(mesh);
    for edge in &mesh.boundary_edges {
        let Some(boundary) = BoundaryTraceKey::from_label(edge.label) else {
            continue;
        };
        let Some([(triangle_index, local_midpoint)]) = owners
            .get(&transfer_edge_key(edge.vertices))
            .map(Vec::as_slice)
        else {
            return Err(TransferError::InvalidTarget);
        };
        for vertex in edge.vertices {
            traces[vertex].boundaries.insert(boundary);
        }
        let node = operator.element_nodes()[*triangle_index][*local_midpoint] as usize;
        traces[node].boundaries.insert(boundary);
    }
    Ok(traces)
}

fn source_trace_triangles(mesh: &TriMesh) -> Result<SourceTraceTriangles, TransferError> {
    let mut traces = SourceTraceTriangles::default();
    let owners = triangle_edge_owners(mesh);
    for (triangle_index, triangle) in mesh.triangles.iter().enumerate() {
        let triangle_index =
            u32::try_from(triangle_index).map_err(|_| TransferError::InvalidSource)?;
        for vertex_index in triangle.vertices {
            if let Some(trace) = mesh.vertices[vertex_index].trace {
                traces
                    .vertices
                    .entry(trace)
                    .or_default()
                    .insert(triangle_index);
            }
        }
    }
    for edge in &mesh.boundary_edges {
        let Some(boundary) = BoundaryTraceKey::from_label(edge.label) else {
            continue;
        };
        let Some([(triangle, _)]) = owners
            .get(&transfer_edge_key(edge.vertices))
            .map(Vec::as_slice)
        else {
            return Err(TransferError::InvalidSource);
        };
        traces
            .boundaries
            .entry(boundary)
            .or_default()
            .insert(u32::try_from(*triangle).map_err(|_| TransferError::InvalidSource)?);
    }
    Ok(traces)
}

fn has_unified_topology(mesh: &TriMesh) -> bool {
    mesh.vertices.iter().any(|vertex| vertex.trace.is_some())
        || mesh
            .boundary_edges
            .iter()
            .any(|edge| matches!(edge.label, BoundaryLabel::Curve { .. }))
}

fn transfer_edge_key(vertices: [usize; 2]) -> (usize, usize) {
    (vertices[0].min(vertices[1]), vertices[0].max(vertices[1]))
}

fn triangle_edge_owners(mesh: &TriMesh) -> BTreeMap<(usize, usize), Vec<(usize, usize)>> {
    let mut owners = BTreeMap::<_, Vec<_>>::new();
    for (triangle_index, triangle) in mesh.triangles.iter().enumerate() {
        for (vertices, local_midpoint) in [
            ([triangle.vertices[0], triangle.vertices[1]], 3),
            ([triangle.vertices[1], triangle.vertices[2]], 4),
            ([triangle.vertices[0], triangle.vertices[2]], 5),
        ] {
            owners
                .entry(transfer_edge_key(vertices))
                .or_default()
                .push((triangle_index, local_midpoint));
        }
    }
    owners
}

fn validate_quadratic_pair(
    mesh: &TriMesh,
    operator: &QuadraticWaveOperator,
    source: bool,
) -> Result<(), TransferError> {
    let error = if source {
        TransferError::InvalidSource
    } else {
        TransferError::InvalidTarget
    };
    if operator.geometry_revision() != mesh.geometry_revision
        || operator.mesh_revision() != mesh.mesh_revision
        || operator.element_nodes().len() != mesh.triangles.len()
        || operator.node_points().len() < mesh.vertices.len()
        || operator
            .node_points()
            .iter()
            .zip(&mesh.vertices)
            .any(|(node, vertex)| *node != vertex.point)
        || operator
            .element_nodes()
            .iter()
            .flatten()
            .any(|index| *index as usize >= operator.degrees_of_freedom())
    {
        Err(error)
    } else {
        Ok(())
    }
}

/// Reconstructs velocity at the current level from a centered two-level state.
/// `undamped_acceleration` is forcing minus `M^-1 K u`, before `-gamma v`.
pub fn centered_velocity(
    previous: f64,
    current: f64,
    undamped_acceleration: f64,
    damping_ratio: f64,
    time_step: f64,
) -> Option<f64> {
    let velocity = ((current - previous) / time_step + 0.5 * time_step * undamped_acceleration)
        / (1.0 + 0.5 * damping_ratio * time_step);
    (previous.is_finite()
        && current.is_finite()
        && undamped_acceleration.is_finite()
        && damping_ratio.is_finite()
        && damping_ratio >= 0.0
        && time_step.is_finite()
        && time_step > 0.0
        && velocity.is_finite())
    .then_some(velocity)
}

pub fn centered_previous(
    current: f64,
    velocity: f64,
    undamped_acceleration: f64,
    damping_ratio: f64,
    time_step: f64,
) -> Option<f64> {
    let acceleration = undamped_acceleration - damping_ratio * velocity;
    let previous = current - time_step * velocity + 0.5 * time_step * time_step * acceleration;
    (current.is_finite()
        && velocity.is_finite()
        && undamped_acceleration.is_finite()
        && damping_ratio.is_finite()
        && damping_ratio >= 0.0
        && time_step.is_finite()
        && time_step > 0.0
        && previous.is_finite())
    .then_some(previous)
}

fn validate_mesh(mesh: &TriMesh, source: bool) -> Result<(), TransferError> {
    let error = if source {
        TransferError::InvalidSource
    } else {
        TransferError::InvalidTarget
    };
    if mesh.vertices.iter().any(|vertex| !vertex.point.finite())
        || mesh.triangles.iter().any(|triangle| {
            triangle
                .vertices
                .iter()
                .any(|&index| index >= mesh.vertices.len())
        })
        || mesh.vertices.len() > u32::MAX as usize
    {
        return Err(error);
    }
    if mesh.triangles.iter().any(|triangle| {
        let [a, b, c] = triangle.vertices.map(|index| mesh.vertices[index].point);
        let area = (b - a).cross(c - a);
        !area.is_finite() || area <= 0.0
    }) {
        return Err(error);
    }
    Ok(())
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

fn region_components(
    mesh: &TriMesh,
) -> Result<std::collections::BTreeMap<crate::RegionId, crate::RegionId>, TransferError> {
    let mut groups = mesh
        .triangles
        .iter()
        .map(|triangle| (triangle.region, triangle.region))
        .collect::<std::collections::BTreeMap<_, _>>();
    for boundary in &mesh.boundary_edges {
        if !matches!(boundary.label, crate::BoundaryLabel::MaterialInterface(_)) {
            continue;
        }
        let adjacent = mesh
            .triangles
            .iter()
            .filter(|triangle| {
                triangle.vertices.contains(&boundary.vertices[0])
                    && triangle.vertices.contains(&boundary.vertices[1])
            })
            .map(|triangle| triangle.region)
            .collect::<Vec<_>>();
        if adjacent.len() != 2 {
            return Err(TransferError::InvalidTarget);
        }
        let a = groups[&adjacent[0]];
        let b = groups[&adjacent[1]];
        let keep = a.min(b);
        let replace = a.max(b);
        for group in groups.values_mut() {
            if *group == replace {
                *group = keep;
            }
        }
    }
    Ok(groups)
}

fn barycentric(point: Point2, triangle: [Point2; 3]) -> Option<[f64; 3]> {
    let [a, b, c] = triangle;
    let denominator = (b - a).cross(c - a);
    if !denominator.is_finite() || denominator <= 0.0 {
        return None;
    }
    let w1 = (point - a).cross(c - a) / denominator;
    let w2 = (b - a).cross(point - a) / denominator;
    let weights = [1.0 - w1 - w2, w1, w2];
    let scale = triangle
        .iter()
        .map(|p| p.x.abs().max(p.y.abs()))
        .fold(1.0, f64::max);
    let tolerance = 64.0 * f64::EPSILON * scale;
    weights
        .iter()
        .all(|weight| *weight >= -tolerance && *weight <= 1.0 + tolerance)
        .then_some(weights)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BACKGROUND_REGION, BoundaryEdge, BoundaryLabel, BoundarySide, CurveNode, CurveSpan,
        CurveSpanId, CurveSpline, FaceRegionAssignment, LoopRole, Material, MaterialId,
        MeshQuality, MeshTriangle, MeshVertex, MeshingOptions, Obstacle, ObstacleId,
        OpenCubicSpline, OuterBoundaryCondition, OuterBoundaryConditions, PeriodicCubicSpline,
        Region, RegionId, Scene, SpanBehavior, TopologyCurve, TopologyGeometry, TopologyMeshPlan,
        TopologyVertex, TopologyVertexId, TopologyVertexLocation, TopologyWaveModel,
        WaveCoefficients, compile_topology, mesh_topology_plan,
    };

    fn mesh(revision: u64, points: &[[f64; 2]], triangles: &[[usize; 3]]) -> TriMesh {
        TriMesh {
            geometry_revision: revision,
            mesh_revision: revision,
            vertices: points
                .iter()
                .map(|p| MeshVertex {
                    point: Point2::new(p[0], p[1]),
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
                maximum_edge_length: 1.0,
            },
        }
    }

    fn topology_mesh(geometry: &TopologyGeometry, revision: u64) -> (TriMesh, TopologyMeshPlan) {
        let topology = compile_topology(geometry, revision).unwrap();
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
            revision,
            MeshingOptions {
                target_edge_length: 0.4,
                minimum_angle_degrees: 8.0,
                max_vertices: 20_000,
                max_triangles: 40_000,
                max_refinement_steps: 20_000,
                ..MeshingOptions::default()
            },
        )
        .unwrap();
        (mesh, plan)
    }

    fn transmitting_divider() -> TopologyGeometry {
        let bottom = TopologyVertexId(1);
        let top = TopologyVertexId(2);
        let mut divider = TopologyCurve::new(
            CurveId(1),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![Point2::new(0.0, -1.0), Point2::new(0.0, 1.0)])
                    .unwrap(),
            ),
            vec![CurveSpan {
                id: CurveSpanId(1),
                behavior: SpanBehavior::Transmitting,
            }],
        )
        .unwrap();
        divider.nodes = vec![
            CurveNode {
                vertex: Some(bottom),
            },
            CurveNode { vertex: Some(top) },
        ];
        TopologyGeometry {
            curves: vec![divider],
            vertices: vec![
                TopologyVertex {
                    id: bottom,
                    location: TopologyVertexLocation::Outer {
                        side: crate::OuterSide::Bottom,
                        fraction: 0.5,
                    },
                },
                TopologyVertex {
                    id: top,
                    location: TopologyVertexLocation::Outer {
                        side: crate::OuterSide::Top,
                        fraction: 0.5,
                    },
                },
            ],
            ..TopologyGeometry::default()
        }
    }

    #[test]
    fn affine_fields_transfer_exactly_and_revisions_are_recorded() {
        let source = mesh(
            3,
            &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            &[[0, 1, 2], [0, 2, 3]],
        );
        let target = mesh(8, &[[0.25, 0.1], [0.8, 0.4], [0.2, 0.9]], &[[0, 1, 2]]);
        let map = TransferMap::build(&source, &target).unwrap();
        let values: Vec<_> = source
            .vertices
            .iter()
            .map(|vertex| 2.0 * vertex.point.x - 3.0 * vertex.point.y + 0.4)
            .collect();
        let transferred = map.interpolate(&values, 0.0).unwrap();
        for (vertex, value) in target.vertices.iter().zip(transferred) {
            let exact = 2.0 * vertex.point.x - 3.0 * vertex.point.y + 0.4;
            assert!((value - exact).abs() < 1.0e-13);
        }
        assert_eq!(map.source_revision(), 3);
        assert_eq!(map.target_revision(), 8);
        let mut stale = source.clone();
        stale.geometry_revision += 1;
        assert!(!map.matches_meshes(&stale, &target));
    }

    #[test]
    fn quadratic_transfer_reproduces_quadratics_and_arbitrary_self_state() {
        let source = mesh(
            3,
            &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            &[[0, 1, 2], [0, 2, 3]],
        );
        let target = mesh(8, &[[0.25, 0.1], [0.8, 0.4], [0.2, 0.9]], &[[0, 1, 2]]);
        let source_operator =
            QuadraticWaveOperator::assemble(&source, WaveCoefficients::default()).unwrap();
        let target_operator =
            QuadraticWaveOperator::assemble(&target, WaveCoefficients::default()).unwrap();
        let map = QuadraticTransferMap::build(&source, &source_operator, &target, &target_operator)
            .unwrap();
        assert!(map.matches(&source, &source_operator, &target, &target_operator));
        assert_eq!(map.exposed_nodes(), 0);
        let polynomial = |point: Point2| {
            0.7 + 2.0 * point.x - 0.4 * point.y + 1.3 * point.x * point.x - 0.8 * point.x * point.y
                + 0.2 * point.y * point.y
        };
        let source_values = source_operator
            .node_points()
            .iter()
            .map(|point| polynomial(*point))
            .collect::<Vec<_>>();
        let transferred = map.interpolate(&source_values, -10.0).unwrap();
        for (point, value) in target_operator.node_points().iter().zip(transferred) {
            assert!((value - polynomial(*point)).abs() < 2.0e-13);
        }

        let self_map =
            QuadraticTransferMap::build(&source, &source_operator, &source, &source_operator)
                .unwrap();
        let arbitrary = (0..source_operator.degrees_of_freedom())
            .map(|index| (index as f64 * 1.7).sin())
            .collect::<Vec<_>>();
        let copied = self_map.interpolate(&arbitrary, 0.0).unwrap();
        for (actual, expected) in copied.iter().zip(arbitrary) {
            assert!((actual - expected).abs() < 2.0e-13);
        }

        let target_operator = QuadraticWaveOperator::assemble_with_boundary(
            &source,
            WaveCoefficients::default(),
            crate::OuterBoundaryCondition::FirstOrderOutgoing,
        )
        .unwrap();
        let identity =
            QuadraticTransferMap::identity_on_mesh(&source, &source_operator, &target_operator)
                .unwrap();
        assert_eq!(identity.interpolate(&copied, 0.0).unwrap(), copied);
    }

    #[test]
    fn quadratic_transfer_keeps_coincident_wall_traces_on_their_own_regions() {
        let points = [
            [0.0, 0.0],
            [1.0, 0.0],
            [0.0, 1.0],
            [0.0, 0.0],
            [1.0, 0.0],
            [0.0, 1.0],
        ];
        let mut source = mesh(3, &points, &[[0, 1, 2], [3, 4, 5]]);
        source.triangles[1].region = RegionId(2);
        source.boundary_edges = vec![
            BoundaryEdge {
                vertices: [0, 1],
                label: BoundaryLabel::Wall {
                    loop_id: ObstacleId(1),
                    side: BoundarySide::Exterior,
                },
                parameters: [0.0, 1.0],
            },
            BoundaryEdge {
                vertices: [3, 4],
                label: BoundaryLabel::Wall {
                    loop_id: ObstacleId(1),
                    side: BoundarySide::Interior,
                },
                parameters: [0.0, 1.0],
            },
        ];
        let mut target = source.clone();
        target.geometry_revision = 8;
        let scene = Scene {
            domain: crate::DomainRect::default(),
            physics: crate::PhysicsModel::Mechanical,
            obstacles: vec![Obstacle::with_role(
                ObstacleId(1),
                PeriodicCubicSpline::rounded(Point2::new(0.2, 0.2), 0.1),
                LoopRole::Wall {
                    exterior: BACKGROUND_REGION,
                    interior: RegionId(2),
                },
            )],
            internal_boundaries: vec![],
            material_interfaces: vec![],
            junctions: vec![],
            materials: vec![
                Material::default_medium(),
                Material {
                    id: MaterialId(2),
                    name: "Inside".into(),
                    mass_density: crate::ScalarField::constant(1.0),
                    stiffness: crate::ScalarField::constant(1.0),
                    damping: crate::ScalarField::constant(0.0),
                    axis_ratio: crate::ScalarField::constant(1.0),
                    parameters: vec![],
                    color: [1, 2, 3],
                },
            ],
            regions: vec![
                Region {
                    id: BACKGROUND_REGION,
                    material: MaterialId(1),
                    frame: crate::MaterialFrame::world(),
                },
                Region {
                    id: RegionId(2),
                    material: MaterialId(2),
                    frame: crate::MaterialFrame::world(),
                },
            ],
            volume_sources: vec![],
            outer_boundaries: OuterBoundaryConditions::default(),
        };
        let source_operator = QuadraticWaveOperator::assemble_scene(
            &source,
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let target_operator = QuadraticWaveOperator::assemble_scene(
            &target,
            &scene,
            OuterBoundaryCondition::Reflecting,
        )
        .unwrap();
        let map = QuadraticTransferMap::build(&source, &source_operator, &target, &target_operator)
            .unwrap();
        let mut values = vec![0.0; source_operator.degrees_of_freedom()];
        for node in source_operator.element_nodes()[0] {
            values[node as usize] = 1.0;
        }
        for node in source_operator.element_nodes()[1] {
            values[node as usize] = 9.0;
        }
        let transferred = map.interpolate(&values, -1.0).unwrap();
        for node in target_operator.element_nodes()[0] {
            assert!((transferred[node as usize] - 1.0).abs() < 1.0e-12);
        }
        for node in target_operator.element_nodes()[1] {
            assert!((transferred[node as usize] - 9.0).abs() < 1.0e-12);
        }
    }

    #[test]
    fn quadratic_transfer_keeps_coincident_baffle_faces_separate() {
        let points = [
            [0.0, 0.0],
            [1.0, 0.0],
            [0.0, 1.0],
            [0.0, 0.0],
            [1.0, 0.0],
            [0.0, 1.0],
        ];
        let mut source = mesh(3, &points, &[[0, 1, 2], [3, 4, 5]]);
        source.boundary_edges = vec![
            BoundaryEdge {
                vertices: [0, 1],
                label: BoundaryLabel::InternalBoundary {
                    id: InternalBoundaryId(4),
                    side: InternalBoundarySide::Left,
                },
                parameters: [0.0, 1.0],
            },
            BoundaryEdge {
                vertices: [4, 3],
                label: BoundaryLabel::InternalBoundary {
                    id: InternalBoundaryId(4),
                    side: InternalBoundarySide::Right,
                },
                parameters: [1.0, 0.0],
            },
        ];
        let mut target = source.clone();
        target.geometry_revision = 8;
        let source_operator =
            QuadraticWaveOperator::assemble(&source, WaveCoefficients::default()).unwrap();
        let target_operator =
            QuadraticWaveOperator::assemble(&target, WaveCoefficients::default()).unwrap();
        let map = QuadraticTransferMap::build(&source, &source_operator, &target, &target_operator)
            .unwrap();
        let left = source_operator.element_nodes()[0][3] as usize;
        let right = source_operator.element_nodes()[1][3] as usize;
        let target_left = target_operator.element_nodes()[0][3] as usize;
        let target_right = target_operator.element_nodes()[1][3] as usize;
        let mut values = vec![0.0; source_operator.degrees_of_freedom()];
        values[left] = 1.0;
        values[right] = 9.0;
        let transferred = map.interpolate(&values, -1.0).unwrap();
        assert!((transferred[target_left] - 1.0).abs() < 1.0e-12);
        assert!((transferred[target_right] - 9.0).abs() < 1.0e-12);
    }

    #[test]
    fn quadratic_transfer_keeps_unified_curve_sides_separate() {
        let points = [
            [0.0, 0.0],
            [1.0, 0.0],
            [0.0, 1.0],
            [0.0, 0.0],
            [1.0, 0.0],
            [0.0, 1.0],
        ];
        let mut source = mesh(3, &points, &[[0, 1, 2], [3, 4, 5]]);
        for vertex in &mut source.vertices[..3] {
            vertex.trace = Some(TraceVertexId(1));
        }
        for vertex in &mut source.vertices[3..] {
            vertex.trace = Some(TraceVertexId(2));
        }
        source.boundary_edges = vec![
            BoundaryEdge {
                vertices: [0, 1],
                label: BoundaryLabel::Curve {
                    curve: CurveId(7),
                    span: CurveSpanId(70),
                    side: CurveTraceSide::Left,
                    separated: true,
                },
                parameters: [0.0, 1.0],
            },
            BoundaryEdge {
                vertices: [4, 3],
                label: BoundaryLabel::Curve {
                    curve: CurveId(7),
                    span: CurveSpanId(70),
                    side: CurveTraceSide::Right,
                    separated: true,
                },
                parameters: [1.0, 0.0],
            },
        ];
        let mut target = source.clone();
        target.geometry_revision = 8;
        let source_operator =
            QuadraticWaveOperator::assemble(&source, WaveCoefficients::default()).unwrap();
        let target_operator =
            QuadraticWaveOperator::assemble(&target, WaveCoefficients::default()).unwrap();
        let map = QuadraticTransferMap::build(&source, &source_operator, &target, &target_operator)
            .unwrap();
        let mut values = vec![0.0; source_operator.degrees_of_freedom()];
        for node in source_operator.element_nodes()[0] {
            values[node as usize] = 1.0;
        }
        for node in source_operator.element_nodes()[1] {
            values[node as usize] = 9.0;
        }
        let transferred = map.interpolate(&values, -1.0).unwrap();
        for node in [
            target_operator.element_nodes()[0][0],
            target_operator.element_nodes()[0][1],
            target_operator.element_nodes()[0][3],
        ] {
            assert!((transferred[node as usize] - 1.0).abs() < 1.0e-12);
        }
        for node in [
            target_operator.element_nodes()[1][0],
            target_operator.element_nodes()[1][1],
            target_operator.element_nodes()[1][3],
        ] {
            assert!((transferred[node as usize] - 9.0).abs() < 1.0e-12);
        }
    }

    #[test]
    fn topology_transfer_survives_transmitting_face_split_and_merge() {
        let (unsplit, unsplit_plan) = topology_mesh(&TopologyGeometry::default(), 30);
        let (split, split_plan) = topology_mesh(&transmitting_divider(), 31);
        assert_eq!(
            split
                .triangles
                .iter()
                .map(|triangle| triangle.region)
                .collect::<BTreeSet<_>>()
                .len(),
            2
        );
        let materials = [Material::default_medium()];
        let regions = [
            Region {
                id: RegionId(1),
                material: MaterialId(1),
                frame: crate::MaterialFrame::world(),
            },
            Region {
                id: RegionId(2),
                material: MaterialId(1),
                frame: crate::MaterialFrame::world(),
            },
        ];
        let model = TopologyWaveModel {
            physics: crate::PhysicsModel::Mechanical,
            materials: &materials,
            regions: &regions,
            outer_boundaries: OuterBoundaryConditions::default(),
        };
        let unsplit_operator =
            QuadraticWaveOperator::assemble_topology(&unsplit, &unsplit_plan, model).unwrap();
        let split_operator =
            QuadraticWaveOperator::assemble_topology(&split, &split_plan, model).unwrap();
        let polynomial = |point: Point2| {
            0.3 - 0.7 * point.x + 1.1 * point.y + 0.4 * point.x * point.x - 0.2 * point.x * point.y
                + 0.6 * point.y * point.y
        };

        let unsplit_values = unsplit_operator
            .node_points()
            .iter()
            .map(|point| polynomial(*point))
            .collect::<Vec<_>>();
        let split_map =
            QuadraticTransferMap::build(&unsplit, &unsplit_operator, &split, &split_operator)
                .unwrap();
        assert_eq!(split_map.exposed_nodes(), 0);
        let split_values = split_map.interpolate(&unsplit_values, -10.0).unwrap();
        for (point, value) in split_operator.node_points().iter().zip(&split_values) {
            assert!((*value - polynomial(*point)).abs() < 3.0e-12);
        }

        let merge_map =
            QuadraticTransferMap::build(&split, &split_operator, &unsplit, &unsplit_operator)
                .unwrap();
        assert_eq!(merge_map.exposed_nodes(), 0);
        let merged_values = merge_map.interpolate(&split_values, -10.0).unwrap();
        for (point, value) in unsplit_operator.node_points().iter().zip(merged_values) {
            assert!((value - polynomial(*point)).abs() < 3.0e-12);
        }
    }

    #[test]
    fn topology_transfer_uses_junction_sector_trace_ids() {
        // Three sector vertices occupy the same junction point. Each boundary
        // side is also present on a neighboring sector, so the trace ID is what
        // narrows the candidate set to the correct angular sector.
        let points = [
            [0.0, 0.0],
            [1.0, 0.0],
            [0.0, 1.0],
            [0.0, 0.0],
            [0.0, 1.0],
            [-1.0, 0.0],
            [0.0, 0.0],
            [-1.0, 0.0],
            [0.0, -1.0],
        ];
        let mut source = mesh(40, &points, &[[0, 1, 2], [3, 4, 5], [6, 7, 8]]);
        source.vertices[0].trace = Some(TraceVertexId(1));
        source.vertices[3].trace = Some(TraceVertexId(2));
        source.vertices[6].trace = Some(TraceVertexId(3));
        let label = |curve, side| BoundaryLabel::Curve {
            curve: CurveId(curve),
            span: CurveSpanId(curve),
            side,
            separated: true,
        };
        source.boundary_edges = vec![
            BoundaryEdge {
                vertices: [0, 1],
                label: label(10, CurveTraceSide::Left),
                parameters: [0.0, 1.0],
            },
            BoundaryEdge {
                vertices: [2, 0],
                label: label(11, CurveTraceSide::Right),
                parameters: [1.0, 0.0],
            },
            BoundaryEdge {
                vertices: [3, 4],
                label: label(11, CurveTraceSide::Right),
                parameters: [0.0, 1.0],
            },
            BoundaryEdge {
                vertices: [5, 3],
                label: label(10, CurveTraceSide::Right),
                parameters: [1.0, 0.0],
            },
            BoundaryEdge {
                vertices: [6, 7],
                label: label(10, CurveTraceSide::Right),
                parameters: [0.0, 1.0],
            },
            BoundaryEdge {
                vertices: [8, 6],
                label: label(10, CurveTraceSide::Left),
                parameters: [1.0, 0.0],
            },
        ];
        let mut target = source.clone();
        target.geometry_revision = 41;
        let source_operator =
            QuadraticWaveOperator::assemble(&source, WaveCoefficients::default()).unwrap();
        let target_operator =
            QuadraticWaveOperator::assemble(&target, WaveCoefficients::default()).unwrap();
        let map = QuadraticTransferMap::build(&source, &source_operator, &target, &target_operator)
            .unwrap();
        let mut values = vec![0.0; source_operator.degrees_of_freedom()];
        values[source_operator.element_nodes()[0][0] as usize] = 1.0;
        values[source_operator.element_nodes()[1][0] as usize] = 2.0;
        values[source_operator.element_nodes()[2][0] as usize] = 3.0;
        let transferred = map.interpolate(&values, -10.0).unwrap();
        assert_eq!(
            transferred[target_operator.element_nodes()[0][0] as usize],
            1.0
        );
        assert_eq!(
            transferred[target_operator.element_nodes()[1][0] as usize],
            2.0
        );
        assert_eq!(
            transferred[target_operator.element_nodes()[2][0] as usize],
            3.0
        );
    }

    #[test]
    fn quadratic_transfer_rejects_mismatched_operator_revisions() {
        let source = mesh(3, &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], &[[0, 1, 2]]);
        let operator =
            QuadraticWaveOperator::assemble(&source, WaveCoefficients::default()).unwrap();
        let mut stale = source.clone();
        stale.geometry_revision += 1;
        assert_eq!(
            QuadraticTransferMap::build(&stale, &operator, &source, &operator),
            Err(TransferError::InvalidSource)
        );
        assert_eq!(
            QuadraticTransferMap::build(&source, &operator, &stale, &operator),
            Err(TransferError::InvalidTarget)
        );
        let mut stale_mesh = source.clone();
        stale_mesh.mesh_revision += 1;
        assert_eq!(
            QuadraticTransferMap::build(&stale_mesh, &operator, &source, &operator),
            Err(TransferError::InvalidSource)
        );
        assert_eq!(
            QuadraticTransferMap::build(&source, &operator, &stale_mesh, &operator),
            Err(TransferError::InvalidTarget)
        );
    }

    #[test]
    fn newly_exposed_vertices_use_the_requested_initial_value() {
        let source = mesh(1, &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], &[[0, 1, 2]]);
        let target = mesh(2, &[[0.2, 0.2], [0.8, 0.8], [-0.1, 0.1]], &[[0, 1, 2]]);
        let map = TransferMap::build(&source, &target).unwrap();
        assert_eq!(map.exposed_vertices(), 2);
        assert_eq!(
            map.interpolate(&[1.0, 2.0, 3.0], -7.0).unwrap()[1..],
            [-7.0, -7.0]
        );
    }

    #[test]
    fn malformed_inputs_are_rejected_without_partial_values() {
        let mut source = mesh(1, &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], &[[0, 1, 2]]);
        let target = source.clone();
        source.triangles[0].vertices[2] = 9;
        assert_eq!(
            TransferMap::build(&source, &target),
            Err(TransferError::InvalidSource)
        );
        let map = TransferMap::build(&target, &target).unwrap();
        assert!(matches!(
            map.interpolate(&[1.0], 0.0),
            Err(TransferError::SizeMismatch { .. })
        ));
        assert_eq!(
            map.interpolate(&[1.0, f64::NAN, 3.0], 0.0),
            Err(TransferError::NonFiniteValues)
        );
    }

    #[test]
    fn changing_timestep_preserves_the_reconstructed_current_velocity() {
        let current = 1.7;
        let velocity = -0.45;
        let undamped_acceleration = 0.8;
        let damping = 0.3;
        let old_dt = 0.04;
        let old_previous =
            centered_previous(current, velocity, undamped_acceleration, damping, old_dt).unwrap();
        let recovered = centered_velocity(
            old_previous,
            current,
            undamped_acceleration,
            damping,
            old_dt,
        )
        .unwrap();
        assert!((recovered - velocity).abs() < 1.0e-14);

        let new_dt = 0.013;
        let new_previous =
            centered_previous(current, recovered, undamped_acceleration, damping, new_dt).unwrap();
        let recovered_again = centered_velocity(
            new_previous,
            current,
            undamped_acceleration,
            damping,
            new_dt,
        )
        .unwrap();
        assert!((recovered_again - velocity).abs() < 1.0e-13);
        assert_ne!(new_previous, old_previous);
    }
}
