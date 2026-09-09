use crate::{Point2, TriMesh};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransferSample {
    pub vertices: [u32; 3],
    pub weights: [f64; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransferMap {
    source_revision: u64,
    target_revision: u64,
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
        if source.triangles.is_empty() {
            return Err(TransferError::EmptySource);
        }
        validate_mesh(source, true)?;
        validate_mesh(target, false)?;

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
        for vertex in &target.vertices {
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
            for &triangle_index in &bins[y * dimension + x] {
                let triangle = source.triangles[triangle_index as usize];
                let points = triangle.vertices.map(|i| source.vertices[i].point);
                if let Some(weights) = barycentric(point, points) {
                    found = Some(TransferSample {
                        vertices: triangle.vertices.map(|i| i as u32),
                        weights,
                    });
                    break;
                }
            }
            samples.push(found);
        }
        Ok(Self {
            source_revision: source.geometry_revision,
            target_revision: target.geometry_revision,
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
    use crate::{MeshQuality, MeshTriangle, MeshVertex};

    fn mesh(revision: u64, points: &[[f64; 2]], triangles: &[[usize; 3]]) -> TriMesh {
        TriMesh {
            geometry_revision: revision,
            vertices: points
                .iter()
                .map(|p| MeshVertex {
                    point: Point2::new(p[0], p[1]),
                    boundary: None,
                })
                .collect(),
            triangles: triangles
                .iter()
                .map(|vertices| MeshTriangle {
                    vertices: *vertices,
                })
                .collect(),
            boundary_edges: vec![],
            quality: MeshQuality {
                minimum_angle_degrees: 45.0,
                maximum_edge_length: 1.0,
            },
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
