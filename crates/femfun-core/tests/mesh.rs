use std::collections::{BTreeMap, BTreeSet};

use femfun_core::*;

fn mesh(scene: &Scene) -> TriMesh {
    mesh_scene(
        scene,
        17,
        MeshingOptions {
            target_edge_length: 0.28,
            minimum_angle_degrees: 12.0,
            ..Default::default()
        },
    )
    .unwrap()
}

fn edge_key(a: usize, b: usize) -> (usize, usize) {
    if a < b { (a, b) } else { (b, a) }
}

fn assert_mesh_invariants(mesh: &TriMesh, holes: usize) {
    let mut adjacency: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for triangle in &mesh.triangles {
        let [a, b, c] = triangle.vertices.map(|index| mesh.vertices[index].point);
        assert_eq!(orient2d(a, b, c), PredicateSign::Positive);
        for edge in [
            edge_key(triangle.vertices[0], triangle.vertices[1]),
            edge_key(triangle.vertices[1], triangle.vertices[2]),
            edge_key(triangle.vertices[2], triangle.vertices[0]),
        ] {
            *adjacency.entry(edge).or_default() += 1;
        }
    }
    let boundary: BTreeSet<_> = mesh
        .boundary_edges
        .iter()
        .map(|edge| edge_key(edge.vertices[0], edge.vertices[1]))
        .collect();
    assert_eq!(boundary.len(), mesh.boundary_edges.len());
    for (edge, count) in &adjacency {
        assert_eq!(*count, if boundary.contains(edge) { 1 } else { 2 });
    }
    let euler =
        mesh.vertices.len() as isize - adjacency.len() as isize + mesh.triangles.len() as isize;
    assert_eq!(euler, 1 - holes as isize);

    let triangle_area = mesh
        .triangles
        .iter()
        .map(|triangle| {
            let [a, b, c] = triangle.vertices.map(|index| mesh.vertices[index].point);
            (b - a).cross(c - a) * 0.5
        })
        .sum::<f64>();
    let boundary_area = mesh
        .boundary_edges
        .iter()
        .map(|edge| {
            mesh.vertices[edge.vertices[0]]
                .point
                .cross(mesh.vertices[edge.vertices[1]].point)
                * 0.5
        })
        .sum::<f64>();
    assert!((triangle_area - boundary_area).abs() < 1.0e-10);
}

#[test]
fn exact_segment_relations_cover_degenerate_cases() {
    let p = Point2::new;
    assert_eq!(
        segment_relation(p(0.0, 0.0), p(1.0, 1.0), p(0.0, 1.0), p(1.0, 0.0)),
        SegmentRelation::ProperIntersection
    );
    assert_eq!(
        segment_relation(p(0.0, 0.0), p(1.0, 0.0), p(1.0, 0.0), p(2.0, 0.0)),
        SegmentRelation::Touching
    );
    assert_eq!(
        segment_relation(p(0.0, 0.0), p(2.0, 0.0), p(1.0, 0.0), p(3.0, 0.0)),
        SegmentRelation::Overlapping
    );
    assert_eq!(
        segment_relation(
            p(0.0, 0.0),
            p(1.0, 1.0e-20),
            p(0.0, 1.0e-20),
            p(1.0, 2.0e-20),
        ),
        SegmentRelation::Disjoint
    );
}

#[test]
fn polygon_location_distinguishes_boundary_without_epsilon() {
    let square = [
        Point2::new(-1.0, -1.0),
        Point2::new(1.0, -1.0),
        Point2::new(1.0, 1.0),
        Point2::new(-1.0, 1.0),
    ];
    assert_eq!(
        locate_in_polygon(Point2::new(0.0, 0.0), &square),
        PolygonLocation::Inside
    );
    assert_eq!(
        locate_in_polygon(Point2::new(1.0, 0.0), &square),
        PolygonLocation::Boundary
    );
    assert_eq!(
        locate_in_polygon(Point2::new(1.0 + f64::EPSILON, 0.0), &square),
        PolygonLocation::Outside
    );
}

#[test]
fn rounded_hole_is_constrained_classified_and_refined() {
    let scene = Scene::initial();
    let mesh = mesh(&scene);
    assert_eq!(mesh.geometry_revision, 17);
    assert_mesh_invariants(&mesh, 1);
    assert!(mesh.quality.maximum_edge_length <= 0.28 * 1.05 + 1.0e-10);
    assert!(mesh.quality.minimum_angle_degrees >= 12.0 - 1.0e-8);
    assert!(
        mesh.boundary_edges
            .iter()
            .any(|edge| edge.label == BoundaryLabel::Obstacle(ObstacleId(1)))
    );
    for triangle in &mesh.triangles {
        let center = triangle
            .vertices
            .map(|index| mesh.vertices[index].point)
            .into_iter()
            .fold(Point2::default(), |sum, point| sum + point)
            / 3.0;
        assert!(center.x.abs() < 1.0 && center.y.abs() < 1.0);
        assert!(
            center.norm() > 0.09,
            "triangle centroid entered the obstacle"
        );
    }
}

#[test]
fn several_holes_keep_labels_and_topology() {
    let scene = Scene {
        obstacles: [(-0.55, -0.35), (0.45, -0.35), (-0.35, 0.48)]
            .into_iter()
            .enumerate()
            .map(|(index, (x, y))| Obstacle {
                id: ObstacleId(index as u64 + 10),
                spline: PeriodicCubicSpline::rounded(Point2::new(x, y), 0.13),
            })
            .collect(),
    };
    let mesh = mesh(&scene);
    assert_mesh_invariants(&mesh, 3);
    let labels: BTreeSet<_> = mesh
        .boundary_edges
        .iter()
        .filter_map(|edge| match edge.label {
            BoundaryLabel::Obstacle(id) => Some(id.0),
            BoundaryLabel::Outer(_) => None,
        })
        .collect();
    assert_eq!(labels, BTreeSet::from([10, 11, 12]));
}

#[test]
fn concave_obstacle_meshes_without_filling_the_hole() {
    let controls = (0..12)
        .map(|index| {
            let angle = index as f64 * std::f64::consts::TAU / 12.0;
            let radius = if index % 2 == 0 { 0.42 } else { 0.2 };
            Point2::new(angle.cos() * radius, angle.sin() * radius)
        })
        .collect();
    let scene = Scene {
        obstacles: vec![Obstacle {
            id: ObstacleId(91),
            spline: PeriodicCubicSpline::uniform(controls).unwrap(),
        }],
    };
    assert!(validate(&scene).valid());
    let mesh = mesh(&scene);
    assert_mesh_invariants(&mesh, 1);
    assert!(mesh.boundary_edges.iter().all(|edge| {
        !matches!(edge.label, BoundaryLabel::Obstacle(id) if id != ObstacleId(91))
    }));
}

#[test]
fn invalid_geometry_options_and_capacity_fail_explicitly() {
    let invalid = Scene {
        obstacles: vec![Obstacle {
            id: ObstacleId(1),
            spline: PeriodicCubicSpline::rounded(Point2::new(0.96, 0.0), 0.2),
        }],
    };
    assert!(matches!(
        mesh_scene(&invalid, 0, MeshingOptions::default()),
        Err(MeshError::InvalidGeometry(ValidationIssue::Outside(_)))
    ));
    assert!(matches!(
        mesh_scene(
            &Scene::initial(),
            0,
            MeshingOptions {
                target_edge_length: 0.0,
                ..Default::default()
            }
        ),
        Err(MeshError::InvalidOptions)
    ));
    assert!(matches!(
        mesh_scene(
            &Scene::initial(),
            0,
            MeshingOptions {
                target_edge_length: 0.05,
                max_vertices: 16,
                max_triangles: 16,
                ..Default::default()
            }
        ),
        Err(MeshError::Capacity { .. })
    ));
}

#[test]
fn meshing_is_deterministic() {
    let options = MeshingOptions {
        target_edge_length: 0.3,
        minimum_angle_degrees: 10.0,
        ..Default::default()
    };
    let first = mesh_scene(&Scene::initial(), 5, options).unwrap();
    let second = mesh_scene(&Scene::initial(), 5, options).unwrap();
    assert_eq!(first, second);
}

#[test]
fn cooperative_job_advances_refinement_in_bounded_units() {
    let options = MeshingOptions {
        target_edge_length: 0.3,
        minimum_angle_degrees: 10.0,
        ..Default::default()
    };
    let expected = mesh_scene(&Scene::initial(), 44, options).unwrap();
    let mut job = MeshingJob::new(Scene::initial(), 44, options);
    assert!(job.advance(0).is_none());
    // The first unit prepares topology but performs no quality insertion.
    assert!(job.advance(1).is_none());
    for _ in 0..options.max_refinement_steps + 1 {
        if let Some(result) = job.advance(1) {
            assert_eq!(result.unwrap(), expected);
            return;
        }
    }
    panic!("cooperative meshing job did not terminate within its stated budget");
}

#[test]
fn empty_domain_and_outer_side_labels_mesh() {
    let scene = Scene::default();
    let mesh = mesh_scene(
        &scene,
        1,
        MeshingOptions {
            target_edge_length: 0.35,
            minimum_angle_degrees: 10.0,
            ..Default::default()
        },
    )
    .unwrap();
    assert_mesh_invariants(&mesh, 0);
    let sides: BTreeSet<_> = mesh
        .boundary_edges
        .iter()
        .filter_map(|edge| match edge.label {
            BoundaryLabel::Outer(side) => Some(side as u8),
            BoundaryLabel::Obstacle(_) => None,
        })
        .collect();
    assert_eq!(sides.len(), 4);
}

#[test]
fn representative_eight_obstacle_scene_meshes_within_limits() {
    let scene = Scene {
        obstacles: (0..8)
            .map(|index| Obstacle {
                id: ObstacleId(index + 1),
                spline: PeriodicCubicSpline::rounded(
                    Point2::new(
                        -0.66 + (index % 4) as f64 * 0.44,
                        -0.4 + (index / 4) as f64 * 0.8,
                    ),
                    0.12,
                ),
            })
            .collect(),
    };
    let mesh = mesh_scene(
        &scene,
        8,
        MeshingOptions {
            curve_tolerance: 2.0e-3,
            target_edge_length: 0.3,
            minimum_angle_degrees: 8.0,
            max_vertices: 5_000,
            max_triangles: 10_000,
            max_refinement_steps: 3_000,
        },
    )
    .unwrap();
    assert_mesh_invariants(&mesh, 8);
    assert!(mesh.vertices.len() < 5_000);
    assert!(mesh.triangles.len() < 10_000);
}
