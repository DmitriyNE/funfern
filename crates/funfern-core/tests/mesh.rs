use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use funfern_core::*;

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
    let mut opposites: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for triangle in &mesh.triangles {
        let [a, b, c] = triangle.vertices.map(|index| mesh.vertices[index].point);
        assert_eq!(orient2d(a, b, c), PredicateSign::Positive);
        for i in 0..3 {
            opposites
                .entry(edge_key(
                    triangle.vertices[(i + 1) % 3],
                    triangle.vertices[(i + 2) % 3],
                ))
                .or_default()
                .push(triangle.vertices[i]);
        }
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
        if !boundary.contains(edge) {
            let (a, b) = *edge;
            let [c, d] = [opposites[edge][0], opposites[edge][1]];
            let [mut a, mut b, c, d] = [a, b, c, d].map(|i| mesh.vertices[i].point);
            let ca = orient2d(c, d, a);
            let cb = orient2d(c, d, b);
            if ca != cb && ca != PredicateSign::Zero && cb != PredicateSign::Zero {
                if orient2d(a, b, c) == PredicateSign::Negative {
                    std::mem::swap(&mut a, &mut b);
                }
                assert_ne!(
                    incircle(a, b, c, d),
                    PredicateSign::Positive,
                    "convex unconstrained edge must be locally Delaunay"
                );
            }
        }
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
            .map(|(index, (x, y))| {
                Obstacle::hole(
                    ObstacleId(index as u64 + 10),
                    PeriodicCubicSpline::rounded(Point2::new(x, y), 0.13),
                )
            })
            .collect(),
        ..Scene::default()
    };
    let mesh = mesh(&scene);
    assert_mesh_invariants(&mesh, 3);
    let labels: BTreeSet<_> = mesh
        .boundary_edges
        .iter()
        .filter_map(|edge| match edge.label {
            BoundaryLabel::Obstacle(id) => Some(id.0),
            BoundaryLabel::Outer(_)
            | BoundaryLabel::MaterialInterface(_)
            | BoundaryLabel::Wall { .. }
            | BoundaryLabel::InternalBoundary { .. } => None,
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
        obstacles: vec![Obstacle::hole(
            ObstacleId(91),
            PeriodicCubicSpline::uniform(controls).unwrap(),
        )],
        ..Scene::default()
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
        obstacles: vec![Obstacle::hole(
            ObstacleId(1),
            PeriodicCubicSpline::rounded(Point2::new(0.96, 0.0), 0.2),
        )],
        ..Scene::default()
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
    // Even validation/preparation can yield; no unit performs a whole rebuild.
    assert!(job.advance(1).is_none());
    let mut phases = BTreeMap::new();
    for _ in 0..2_000_000 {
        *phases.entry(job.phase()).or_insert(0) += 1;
        let before = job.stats();
        if let Some(result) = job.advance(1) {
            assert_eq!(result.unwrap(), expected);
            assert!(job.advance(1).is_none());
            for phase in [
                "Validating",
                "Sampling boundaries",
                "Connecting holes",
                "Triangulating",
                "Legalizing edges",
                "Checking mesh",
            ] {
                assert!(phases[phase] > 1, "{phase} must be resumable");
            }
            return;
        }
        let after = job.stats();
        assert_eq!(after.work_units, before.work_units + 1);
        assert!(after.edge_tests - before.edge_tests <= 1);
        assert!(after.edge_flips - before.edge_flips <= 1);
        assert!(after.refinement_insertions - before.refinement_insertions <= 1);
        assert!(
            after.quality_evaluations - before.quality_evaluations <= 4,
            "only the changed triangles should have their quality recomputed"
        );
    }
    panic!("cooperative meshing job did not terminate within its stated budget");
}

#[test]
fn mesh_output_is_independent_of_work_slice_size() {
    let options = MeshingOptions {
        target_edge_length: 0.3,
        minimum_angle_degrees: 10.0,
        ..Default::default()
    };
    let expected = mesh_scene(&Scene::initial(), 123, options).unwrap();
    for budget in [7, 257, 10_000] {
        let mut job = MeshingJob::new(Scene::initial(), 123, options);
        loop {
            if let Some(result) = job.advance(budget) {
                assert_eq!(result.unwrap(), expected);
                break;
            }
        }
    }
}

#[test]
fn limits_terminate_once_and_obsolete_jobs_can_be_replaced() {
    let mut job = MeshingJob::new(
        Scene::initial(),
        1,
        MeshingOptions {
            max_refinement_steps: 1,
            ..Default::default()
        },
    );
    loop {
        if let Some(result) = job.advance(1000) {
            assert!(matches!(result, Err(MeshError::RefinementLimit(_))));
            assert_eq!(job.stats().refinement_insertions, 1);
            assert!(job.advance(1000).is_none());
            break;
        }
    }
    let mut job = MeshingJob::new(Scene::initial(), 9, MeshingOptions::default());
    assert!(job.advance(100).is_none());
    job = MeshingJob::new(Scene::default(), 10, MeshingOptions::default());
    loop {
        if let Some(result) = job.advance(1000) {
            let mesh = result.unwrap();
            assert_eq!(mesh.geometry_revision, 10);
            assert_mesh_invariants(&mesh, 0);
            break;
        }
    }
}

#[test]
fn maximum_obstacle_scene_stays_within_work_and_quality_limits() {
    let scene = Scene {
        obstacles: (0..32)
            .map(|i| {
                Obstacle::hole(
                    ObstacleId(i + 1),
                    PeriodicCubicSpline::rounded(
                        Point2::new(
                            -0.85 + 1.7 * (i % 8) as f64 / 7.0,
                            -0.65 + 1.3 * (i / 8) as f64 / 3.0,
                        ),
                        0.07,
                    ),
                )
            })
            .collect(),
        ..Scene::default()
    };
    let mut job = MeshingJob::new(
        scene,
        32,
        MeshingOptions {
            curve_tolerance: 0.0015,
            target_edge_length: 0.16,
            minimum_angle_degrees: 12.0,
            max_vertices: 8000,
            max_triangles: 16000,
            max_refinement_steps: 5000,
        },
    );
    for _ in 0..500 {
        if let Some(result) = job.advance(10_000) {
            let mesh = result.unwrap();
            assert_mesh_invariants(&mesh, 32);
            assert!(mesh.quality.maximum_edge_length <= 0.16 * 1.05);
            assert!(mesh.quality.minimum_angle_degrees >= 12.0 - 1e-9);
            return;
        }
    }
    panic!("representative maximum scene exceeded five million elementary units");
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
            BoundaryLabel::Obstacle(_)
            | BoundaryLabel::MaterialInterface(_)
            | BoundaryLabel::Wall { .. }
            | BoundaryLabel::InternalBoundary { .. } => None,
        })
        .collect();
    assert_eq!(sides.len(), 4);
}

#[test]
fn representative_eight_obstacle_scene_meshes_within_limits() {
    let scene = Scene {
        obstacles: (0..8)
            .map(|index| {
                Obstacle::hole(
                    ObstacleId(index + 1),
                    PeriodicCubicSpline::rounded(
                        Point2::new(
                            -0.66 + (index % 4) as f64 * 0.44,
                            -0.4 + (index / 4) as f64 * 0.8,
                        ),
                        0.12,
                    ),
                )
            })
            .collect(),
        ..Scene::default()
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

fn medium(id: u64, name: &str, stiffness: f64, color: [u8; 3]) -> Material {
    Material {
        id: MaterialId(id),
        name: name.into(),
        mass_density: 1.0,
        stiffness,
        damping: 0.0,
        color,
    }
}

fn two_region_scene(role: impl FnOnce(RegionId, RegionId) -> LoopRole) -> Scene {
    Scene {
        obstacles: vec![Obstacle::with_role(
            ObstacleId(20),
            PeriodicCubicSpline::rounded(Point2::default(), 0.42),
            role(BACKGROUND_REGION, RegionId(2)),
        )],
        internal_boundaries: vec![],
        materials: vec![
            Material::default_medium(),
            medium(2, "Inclusion", 2.5, [180, 90, 70]),
        ],
        regions: vec![
            Region {
                id: BACKGROUND_REGION,
                material: DEFAULT_MATERIAL,
            },
            Region {
                id: RegionId(2),
                material: MaterialId(2),
            },
        ],
        outer_boundaries: OuterBoundaryConditions::default(),
    }
}

fn edge_adjacency(mesh: &TriMesh) -> BTreeMap<(usize, usize), Vec<usize>> {
    let mut adjacency = BTreeMap::<_, Vec<_>>::new();
    for (triangle_index, triangle) in mesh.triangles.iter().enumerate() {
        for edge in [
            edge_key(triangle.vertices[0], triangle.vertices[1]),
            edge_key(triangle.vertices[1], triangle.vertices[2]),
            edge_key(triangle.vertices[2], triangle.vertices[0]),
        ] {
            adjacency.entry(edge).or_default().push(triangle_index);
        }
    }
    adjacency
}

#[test]
fn material_interface_retains_both_regions_with_a_shared_trace() {
    let scene =
        two_region_scene(|exterior, interior| LoopRole::MaterialInterface { exterior, interior });
    assert!(validate(&scene).valid());
    let mesh = mesh(&scene);
    let regions = mesh
        .triangles
        .iter()
        .map(|triangle| triangle.region)
        .collect::<BTreeSet<_>>();
    assert_eq!(regions, BTreeSet::from([BACKGROUND_REGION, RegionId(2)]));
    let adjacency = edge_adjacency(&mesh);
    let interfaces = mesh
        .boundary_edges
        .iter()
        .filter(|edge| matches!(edge.label, BoundaryLabel::MaterialInterface(ObstacleId(20))))
        .collect::<Vec<_>>();
    assert!(!interfaces.is_empty());
    for edge in interfaces {
        let sides = &adjacency[&edge_key(edge.vertices[0], edge.vertices[1])];
        assert_eq!(sides.len(), 2);
        assert_ne!(
            mesh.triangles[sides[0]].region,
            mesh.triangles[sides[1]].region
        );
    }
    let area = mesh
        .triangles
        .iter()
        .map(|triangle| {
            let [a, b, c] = triangle.vertices.map(|index| mesh.vertices[index].point);
            0.5 * (b - a).cross(c - a)
        })
        .sum::<f64>();
    assert!((area - 4.0).abs() < 1.0e-10);
}

#[test]
fn closed_wall_duplicates_the_two_traces_and_retains_its_interior() {
    let scene = two_region_scene(|exterior, interior| LoopRole::Wall { exterior, interior });
    assert!(validate(&scene).valid());
    let mesh = mesh(&scene);
    let adjacency = edge_adjacency(&mesh);
    let exterior = mesh
        .boundary_edges
        .iter()
        .filter(|edge| {
            edge.label
                == BoundaryLabel::Wall {
                    loop_id: ObstacleId(20),
                    side: BoundarySide::Exterior,
                }
        })
        .collect::<Vec<_>>();
    let interior = mesh
        .boundary_edges
        .iter()
        .filter(|edge| {
            edge.label
                == BoundaryLabel::Wall {
                    loop_id: ObstacleId(20),
                    side: BoundarySide::Interior,
                }
        })
        .collect::<Vec<_>>();
    assert_eq!(exterior.len(), interior.len());
    assert!(!exterior.is_empty());
    for edge in exterior.iter().chain(&interior) {
        assert_eq!(
            adjacency[&edge_key(edge.vertices[0], edge.vertices[1])].len(),
            1
        );
    }
    assert!(exterior.iter().all(|outside| interior.iter().any(|inside| {
        outside
            .vertices
            .map(|index| mesh.vertices[index].point)
            .into_iter()
            .all(|point| {
                inside
                    .vertices
                    .map(|index| mesh.vertices[index].point)
                    .contains(&point)
            })
    })));
    assert!(
        mesh.triangles
            .iter()
            .any(|triangle| triangle.region == RegionId(2))
    );
}

#[test]
fn open_reflecting_boundary_cuts_two_traces_and_reconnects_at_free_tips() {
    let mut scene = Scene::default();
    scene.internal_boundaries.push(InternalBoundary {
        id: InternalBoundaryId(7),
        spline: OpenCubicSpline::uniform(vec![
            Point2::new(-0.72, -0.12),
            Point2::new(-0.35, 0.28),
            Point2::new(0.05, -0.22),
            Point2::new(0.42, 0.24),
            Point2::new(0.73, 0.04),
        ])
        .unwrap(),
        region: BACKGROUND_REGION,
        span_laws: vec![InternalBoundaryLaw::REFLECTING; 2],
    });
    assert!(validate(&scene).valid());
    let mesh = mesh(&scene);
    assert_mesh_invariants(&mesh, 1);
    let adjacency = edge_adjacency(&mesh);
    let left = mesh
        .boundary_edges
        .iter()
        .filter(|edge| {
            edge.label
                == BoundaryLabel::InternalBoundary {
                    id: InternalBoundaryId(7),
                    side: InternalBoundarySide::Left,
                }
        })
        .collect::<Vec<_>>();
    let right = mesh
        .boundary_edges
        .iter()
        .filter(|edge| {
            edge.label
                == BoundaryLabel::InternalBoundary {
                    id: InternalBoundaryId(7),
                    side: InternalBoundarySide::Right,
                }
        })
        .collect::<Vec<_>>();
    assert_eq!(left.len(), right.len());
    assert!(left.len() >= 2);
    for edge in left.iter().chain(&right) {
        assert_eq!(
            adjacency[&edge_key(edge.vertices[0], edge.vertices[1])].len(),
            1
        );
    }
    for left_edge in &left {
        let left_points = left_edge.vertices.map(|vertex| mesh.vertices[vertex].point);
        assert!(right.iter().any(|right_edge| {
            let right_points = right_edge
                .vertices
                .map(|vertex| mesh.vertices[vertex].point);
            left_points[0] == right_points[1] && left_points[1] == right_points[0]
        }));
    }
    let start = scene.internal_boundaries[0].spline.evaluate(0.0);
    let end = scene.internal_boundaries[0]
        .spline
        .evaluate(scene.internal_boundaries[0].spline.period());
    for tip in [start, end] {
        assert_eq!(
            mesh.vertices
                .iter()
                .filter(|vertex| vertex.point == tip)
                .count(),
            1,
            "free tips must reconnect both traces"
        );
    }
}

#[test]
fn baffle_insertion_avoids_accidental_cfl_slivers() {
    let mut scene = Scene::default();
    scene.obstacles.push(Obstacle::with_role(
        ObstacleId(1),
        PeriodicCubicSpline::rounded(Point2::default(), 0.30),
        LoopRole::Wall {
            exterior: BACKGROUND_REGION,
            interior: RegionId(2),
        },
    ));
    scene.internal_boundaries.push(InternalBoundary {
        id: InternalBoundaryId(1),
        spline: OpenCubicSpline::uniform(vec![
            Point2::new(-0.841_274_799_6, -0.460_310_562_0),
            Point2::new(-0.702_926_820_9, -0.393_873_880_9),
            Point2::new(-0.564_578_842_4, -0.327_437_199_8),
            Point2::new(-0.426_230_863_7, -0.261_000_518_7),
        ])
        .unwrap(),
        region: BACKGROUND_REGION,
        span_laws: vec![InternalBoundaryLaw::REFLECTING],
    });
    scene
        .materials
        .push(medium(2, "Inside", 1.0, [77, 121, 164]));
    scene.regions.push(Region {
        id: RegionId(2),
        material: MaterialId(2),
    });
    let mesh = mesh_scene(
        &scene,
        23,
        MeshingOptions {
            curve_tolerance: 0.0015,
            target_edge_length: 0.08 / 1.05,
            minimum_angle_degrees: 18.0,
            ..MeshingOptions::default()
        },
    )
    .unwrap();
    assert!(mesh.quality.minimum_angle_degrees > 15.0);
    let operator = QuadraticWaveOperator::assemble_scene_with_boundaries(
        &mesh,
        &scene,
        scene.outer_boundaries,
    )
    .unwrap();
    assert!(operator.recommended_time_step() > 0.002);
}

#[test]
fn multiple_open_baffles_keep_independent_labeled_faces() {
    let mut scene = Scene::default();
    for (id, y) in [(3, -0.38), (8, 0.42)] {
        scene.internal_boundaries.push(InternalBoundary {
            id: InternalBoundaryId(id),
            spline: OpenCubicSpline::uniform(vec![
                Point2::new(-0.62, y),
                Point2::new(-0.2, y + 0.05),
                Point2::new(0.2, y - 0.04),
                Point2::new(0.62, y),
            ])
            .unwrap(),
            region: BACKGROUND_REGION,
            span_laws: vec![InternalBoundaryLaw::REFLECTING],
        });
    }
    let mesh = mesh(&scene);
    assert_mesh_invariants(&mesh, 2);
    for id in [InternalBoundaryId(3), InternalBoundaryId(8)] {
        let left = mesh
            .boundary_edges
            .iter()
            .filter(|edge| {
                edge.label
                    == BoundaryLabel::InternalBoundary {
                        id,
                        side: InternalBoundarySide::Left,
                    }
            })
            .count();
        let right = mesh
            .boundary_edges
            .iter()
            .filter(|edge| {
                edge.label
                    == BoundaryLabel::InternalBoundary {
                        id,
                        side: InternalBoundarySide::Right,
                    }
            })
            .count();
        assert_eq!(left, right);
        assert!(left >= 2);
    }
}

#[test]
fn nested_material_regions_follow_explicit_region_ownership() {
    let mut scene =
        two_region_scene(|exterior, interior| LoopRole::MaterialInterface { exterior, interior });
    scene.materials.push(medium(3, "Core", 0.6, [80, 130, 190]));
    scene.regions.push(Region {
        id: RegionId(3),
        material: MaterialId(3),
    });
    scene.obstacles.push(Obstacle::with_role(
        ObstacleId(21),
        PeriodicCubicSpline::rounded(Point2::default(), 0.18),
        LoopRole::MaterialInterface {
            exterior: RegionId(2),
            interior: RegionId(3),
        },
    ));
    assert!(validate(&scene).valid());
    let mesh = mesh(&scene);
    assert_eq!(
        mesh.triangles
            .iter()
            .map(|triangle| triangle.region)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([BACKGROUND_REGION, RegionId(2), RegionId(3)])
    );

    scene.obstacles[1].role = LoopRole::MaterialInterface {
        exterior: BACKGROUND_REGION,
        interior: RegionId(3),
    };
    assert!(matches!(
        validate(&scene).issue,
        Some(ValidationIssue::RegionTopology(ObstacleId(21)))
    ));
}

fn finish_update(mut job: MeshUpdateJob, budget: usize) -> MeshUpdateResult {
    for _ in 0..2_000_000 {
        if let Some(result) = job.advance(budget) {
            assert!(job.advance(1).is_none());
            return result.unwrap();
        }
    }
    panic!("mesh update did not terminate");
}

fn geometric_keys(mesh: &TriMesh, filter: impl Fn(Point2) -> bool) -> BTreeSet<[(u64, u64); 3]> {
    mesh.triangles
        .iter()
        .filter_map(|t| {
            let points = t.vertices.map(|v| mesh.vertices[v].point);
            if !points.iter().all(|p| filter(*p)) {
                return None;
            }
            let mut key = points.map(|p| (p.x.to_bits(), p.y.to_bits()));
            key.sort();
            Some(key)
        })
        .collect()
}

#[test]
fn repeated_control_edits_preserve_distant_elements_and_quality() {
    let options = MeshingOptions {
        target_edge_length: 0.04 / 1.05,
        curve_tolerance: 0.0008,
        minimum_angle_degrees: 12.0,
        max_vertices: 50_000,
        max_triangles: 100_000,
        max_refinement_steps: 50_000,
    };
    let mut scene = Scene::initial();
    let mut mesh = Arc::new(mesh_scene(&scene, 0, options).unwrap());
    for revision in 1..=6 {
        let original = mesh.clone();
        let mut next = scene.clone();
        let point = next.obstacles[0].spline.controls()[0];
        let delta = if revision % 2 == 1 {
            Point2::new(0.004, 0.002)
        } else {
            Point2::new(-0.004, -0.002)
        };
        next.obstacles[0]
            .spline
            .set_control(0, point + delta)
            .unwrap();
        let job = MeshUpdateJob::new(
            Some((mesh.clone(), scene.clone())),
            next.clone(),
            revision,
            options,
        );
        let result = finish_update(job, 257);
        assert!(result.report.used_local, "{:?}", result.report);
        assert!(result.report.preserved_triangles as f64 / mesh.triangles.len() as f64 > 0.85);
        assert_eq!(
            geometric_keys(&mesh, |p| p.norm() > 0.5),
            geometric_keys(&result.mesh, |p| p.norm() > 0.5)
        );
        let unchanged = geometric_keys(&mesh, |_| true)
            .intersection(&geometric_keys(&result.mesh, |_| true))
            .count();
        assert_eq!(unchanged, result.report.preserved_triangles);
        assert_mesh_invariants(&result.mesh, 1);
        assert!(result.mesh.quality.maximum_edge_length <= 0.04);
        assert!(result.mesh.quality.minimum_angle_degrees >= 12.0 - 1e-9);
        assert_eq!(result.mesh.geometry_revision, revision);
        assert_eq!(original, mesh, "source mesh must remain immutable");
        mesh = Arc::new(result.mesh);
        scene = next;
    }
}

#[test]
fn adaptation_scheduling_and_fallback_are_explicit() {
    let options = MeshingOptions {
        target_edge_length: 0.06,
        minimum_angle_degrees: 12.0,
        ..Default::default()
    };
    let scene = Scene::initial();
    let mesh = Arc::new(mesh_scene(&scene, 1, options).unwrap());
    let mut next = scene.clone();
    let p = next.obstacles[0].spline.controls()[0];
    next.obstacles[0]
        .spline
        .set_control(0, p + Point2::new(0.003, 0.0))
        .unwrap();
    let mut job = MeshUpdateJob::new(
        Some((mesh.clone(), scene.clone())),
        next.clone(),
        2,
        options,
    );
    assert!(job.advance(0).is_none());
    assert!(job.advance(1).is_none());
    let a = finish_update(job, 1);
    let b = finish_update(
        MeshUpdateJob::new(Some((mesh.clone(), scene.clone())), next, 2, options),
        10000,
    );
    assert!(a.report.used_local);
    assert_eq!(a.mesh, b.mesh);
    assert_eq!(a.report, b.report);

    let mut cases = vec![Scene::default()];
    let mut inserted = scene.clone();
    let inherited = inserted.obstacles[0].span_conditions[0];
    inserted.obstacles[0].spline.insert(0.2).unwrap();
    inserted.obstacles[0].span_conditions.insert(1, inherited);
    cases.push(inserted);
    let mut shifted = scene.clone();
    for i in 0..8 {
        let p = shifted.obstacles[0].spline.controls()[i];
        shifted.obstacles[0]
            .spline
            .set_control(i, p + Point2::new(0.4, 0.0))
            .unwrap();
    }
    cases.push(shifted);
    for next in cases {
        let holes = next.obstacles.len();
        let result = finish_update(
            MeshUpdateJob::new(Some((mesh.clone(), scene.clone())), next, 9, options),
            10000,
        );
        assert!(!result.report.used_local);
        assert!(result.report.fallback_reason.is_some());
        assert_mesh_invariants(&result.mesh, holes);
    }
}
