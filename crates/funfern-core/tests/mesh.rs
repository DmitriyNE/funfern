//! The topology mesher's properties on whole scenes, through the public
//! path the runtime takes: a topology scene compiled, its plan coarsened to
//! the meshing tolerance, and meshed. These tests held the legacy `Scene`
//! mesher until its retirement; the cases a topology test already covered
//! went with it.
use std::collections::{BTreeMap, BTreeSet};

use funfern_core::*;

/// A curve over `spline`, every span with `behavior`, its spans numbered
/// from `10 * id`.
fn curve(id: u64, spline: CurveSpline, behavior: SpanBehavior) -> TopologyCurve {
    let spans = (0..spline.span_count() as u64)
        .map(|index| CurveSpan {
            id: CurveSpanId(10 * id + index),
            behavior,
        })
        .collect();
    TopologyCurve::new(CurveId(id), spline, spans).unwrap()
}

/// Adds a closed curve whose inside is `region`, or a hole without one.
fn add_loop(
    scene: &mut TopologyScene,
    id: u64,
    spline: PeriodicCubicSpline,
    behavior: SpanBehavior,
    region: Option<RegionId>,
) {
    let curve = curve(id, CurveSpline::Closed(spline), behavior);
    let [start, end] = curve.spline.span_bounds(0).unwrap();
    scene.face_assignments.push(AuthoredFaceAssignment {
        anchor: FaceAnchor::Curve {
            curve: curve.id,
            span: curve.spans[0].id,
            side: CurveTraceSide::Left,
            parameter: 0.5 * (start + end),
        },
        region,
    });
    scene.geometry.curves.push(curve);
}

fn add_hole(scene: &mut TopologyScene, id: u64, spline: PeriodicCubicSpline) {
    add_loop(scene, id, spline, SpanBehavior::REFLECTING, None);
}

/// Adds a free reflecting baffle over `spline`.
fn add_baffle(scene: &mut TopologyScene, id: u64, spline: OpenCubicSpline) {
    scene.geometry.curves.push(curve(
        id,
        CurveSpline::Open(spline),
        SpanBehavior::REFLECTING,
    ));
}

/// The default scene and a hole of radius 0.15 at the origin.
fn initial() -> TopologyScene {
    let mut scene = TopologyScene::default();
    add_hole(
        &mut scene,
        1,
        PeriodicCubicSpline::rounded(Point2::default(), 0.15),
    );
    scene
}

/// `scene`'s plan compiled at `revision` and coarsened for `options`.
fn plan(scene: &TopologyScene, revision: u64, options: MeshingOptions) -> TopologyMeshPlan {
    let compiled = scene.compile(revision).unwrap();
    compiled
        .plan
        .coarsened(&compiled.topology, AtomCoarsening::from_meshing(options))
        .unwrap()
}

fn mesh_with(
    scene: &TopologyScene,
    revision: u64,
    options: MeshingOptions,
) -> Result<TriMesh, MeshError> {
    mesh_topology_plan(&plan(scene, revision, options), revision, options)
}

fn mesh(scene: &TopologyScene) -> TriMesh {
    mesh_with(
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

/// The labels of `mesh`'s edges on `curve` and `side`.
fn sided(mesh: &TriMesh, curve: u64, side: CurveTraceSide) -> Vec<&BoundaryEdge> {
    mesh.boundary_edges
        .iter()
        .filter(|edge| {
            matches!(
                edge.label,
                BoundaryLabel::Curve { curve: id, side: candidate, .. }
                    if id == CurveId(curve) && candidate == side
            )
        })
        .collect()
}

fn area(mesh: &TriMesh) -> f64 {
    mesh.triangles
        .iter()
        .map(|triangle| {
            let [a, b, c] = triangle.vertices.map(|index| mesh.vertices[index].point);
            0.5 * (b - a).cross(c - a)
        })
        .sum()
}

/// The default scene with a rounded loop of radius 0.42 at the origin, an
/// inclusion of a stiffer medium inside, its rim with `behavior`.
fn two_region_scene(behavior: SpanBehavior) -> TopologyScene {
    let mut scene = TopologyScene::default();
    scene
        .materials
        .push(medium(2, "Inclusion", 2.5, [180, 90, 70]));
    scene.regions.push(Region {
        id: RegionId(2),
        material: MaterialId(2),
        frame: MaterialFrame::world(),
    });
    add_loop(
        &mut scene,
        20,
        PeriodicCubicSpline::rounded(Point2::default(), 0.42),
        behavior,
        Some(RegionId(2)),
    );
    scene
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

fn medium(id: u64, name: &str, stiffness: f64, color: [u8; 3]) -> Material {
    Material {
        id: MaterialId(id),
        name: name.into(),
        mass_density: ScalarField::constant(1.0),
        stiffness: ScalarField::constant(stiffness),
        axis_ratio: ScalarField::constant(1.0),
        parameters: vec![],
        color,
        ..Material::default_medium()
    }
}

#[test]
fn rounded_hole_is_constrained_classified_and_refined() {
    let mesh = mesh(&initial());
    assert_eq!(mesh.geometry_revision, 17);
    assert_mesh_invariants(&mesh, 1);
    assert!(mesh.quality.maximum_edge_length <= 0.28 * 1.05 + 1.0e-10);
    assert!(mesh.quality.minimum_angle_degrees >= 12.0 - 1.0e-8);
    assert!(!sided(&mesh, 1, CurveTraceSide::Right).is_empty());
    for triangle in &mesh.triangles {
        let center = triangle
            .vertices
            .map(|index| mesh.vertices[index].point)
            .into_iter()
            .fold(Point2::default(), |sum, point| sum + point)
            / 3.0;
        assert!(center.x.abs() < 1.0 && center.y.abs() < 1.0);
        assert!(center.norm() > 0.09, "triangle centroid entered the hole");
    }
}

#[test]
fn several_holes_keep_labels_and_topology() {
    let mut scene = TopologyScene::default();
    for (index, (x, y)) in [(-0.55, -0.35), (0.45, -0.35), (-0.35, 0.48)]
        .into_iter()
        .enumerate()
    {
        add_hole(
            &mut scene,
            index as u64 + 10,
            PeriodicCubicSpline::rounded(Point2::new(x, y), 0.13),
        );
    }
    let mesh = mesh(&scene);
    assert_mesh_invariants(&mesh, 3);
    let labels: BTreeSet<_> = mesh
        .boundary_edges
        .iter()
        .filter_map(|edge| match edge.label {
            BoundaryLabel::Curve { curve, .. } => Some(curve.0),
            _ => None,
        })
        .collect();
    assert_eq!(labels, BTreeSet::from([10, 11, 12]));
}

#[test]
fn concave_hole_meshes_without_filling_it() {
    let controls = (0..12)
        .map(|index| {
            let angle = index as f64 * std::f64::consts::TAU / 12.0;
            let radius = if index % 2 == 0 { 0.42 } else { 0.2 };
            Point2::new(angle.cos() * radius, angle.sin() * radius)
        })
        .collect();
    let mut scene = TopologyScene::default();
    add_hole(
        &mut scene,
        91,
        PeriodicCubicSpline::uniform(controls).unwrap(),
    );
    let mesh = mesh(&scene);
    assert_mesh_invariants(&mesh, 1);
    assert!(mesh.boundary_edges.iter().all(|edge| {
        !matches!(edge.label, BoundaryLabel::Curve { curve, .. } if curve != CurveId(91))
    }));
}

#[test]
fn invalid_options_and_capacity_fail_explicitly() {
    let scene = initial();
    assert!(matches!(
        mesh_topology_plan(
            &plan(&scene, 0, MeshingOptions::default()),
            0,
            MeshingOptions {
                target_edge_length: 0.0,
                ..Default::default()
            }
        ),
        Err(MeshError::InvalidOptions)
    ));
    assert!(matches!(
        mesh_with(
            &scene,
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
fn cooperative_job_advances_refinement_in_bounded_units() {
    let options = MeshingOptions {
        target_edge_length: 0.3,
        minimum_angle_degrees: 10.0,
        ..Default::default()
    };
    let scene = initial();
    let expected = mesh_with(&scene, 44, options).unwrap();
    let mut job = TopologyMeshingJob::new(plan(&scene, 44, options), 44, options);
    assert!(job.advance(0).is_none());
    assert!(job.advance(1).is_none());
    let mut phases = BTreeMap::new();
    for _ in 0..2_000_000 {
        *phases.entry(job.phase()).or_insert(0) += 1;
        let before = job.stats();
        if let Some(result) = job.advance(1) {
            assert_eq!(result.unwrap(), expected);
            assert!(job.advance(1).is_none());
            for phase in [
                "Connecting holes",
                "Triangulating",
                "Legalizing edges",
                "Refining",
                "Recovering separated curves",
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
fn limits_terminate_once_and_obsolete_jobs_can_be_replaced() {
    let options = MeshingOptions {
        max_refinement_steps: 1,
        ..Default::default()
    };
    let mut job = TopologyMeshingJob::new(plan(&initial(), 1, options), 1, options);
    loop {
        if let Some(result) = job.advance(1000) {
            assert!(matches!(result, Err(MeshError::RefinementLimit { .. })));
            assert_eq!(job.stats().refinement_insertions, 1);
            assert!(job.advance(1000).is_none());
            break;
        }
    }
    let options = MeshingOptions::default();
    let mut job = TopologyMeshingJob::new(plan(&initial(), 9, options), 9, options);
    assert!(job.advance(100).is_none());
    job = TopologyMeshingJob::new(plan(&TopologyScene::default(), 10, options), 10, options);
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
fn many_holes_stay_within_work_and_quality_limits() {
    let mut scene = TopologyScene::default();
    for i in 0..32 {
        add_hole(
            &mut scene,
            i + 1,
            PeriodicCubicSpline::rounded(
                Point2::new(
                    -0.85 + 1.7 * (i % 8) as f64 / 7.0,
                    -0.65 + 1.3 * (i / 8) as f64 / 3.0,
                ),
                0.07,
            ),
        );
    }
    let options = MeshingOptions {
        curve_tolerance: 0.0015,
        target_edge_length: 0.16,
        minimum_angle_degrees: 12.0,
        max_vertices: 8000,
        max_triangles: 16000,
        max_refinement_steps: 5000,
    };
    let mut job = TopologyMeshingJob::new(plan(&scene, 32, options), 32, options);
    for _ in 0..500 {
        if let Some(result) = job.advance(10_000) {
            let mesh = result.unwrap();
            assert_mesh_invariants(&mesh, 32);
            assert!(mesh.quality.maximum_edge_length <= 0.16 * 1.05);
            assert!(mesh.quality.minimum_angle_degrees >= 12.0 - 1e-9);
            return;
        }
    }
    panic!("the 32-hole scene exceeded five million elementary units");
}

#[test]
fn rectangular_domain_controls_outer_mesh_geometry() {
    let domain = DomainRect::new(-2.0, 1.0, -0.5, 1.5);
    let scene = TopologyScene {
        geometry: TopologyGeometry {
            domain,
            ..TopologyGeometry::default()
        },
        ..TopologyScene::default()
    };
    let mesh = mesh_with(
        &scene,
        2,
        MeshingOptions {
            target_edge_length: 0.4,
            minimum_angle_degrees: 10.0,
            ..Default::default()
        },
    )
    .unwrap();
    assert!((area(&mesh) - domain.width() * domain.height()).abs() < 1.0e-10);
    assert!(mesh.vertices.iter().all(|vertex| {
        (domain.min_x..=domain.max_x).contains(&vertex.point.x)
            && (domain.min_y..=domain.max_y).contains(&vertex.point.y)
    }));
    for side in OuterSide::ALL {
        assert!(
            mesh.boundary_edges
                .iter()
                .any(|edge| edge.label == BoundaryLabel::Outer(side))
        );
    }
}

#[test]
fn eight_holes_mesh_within_limits() {
    let mut scene = TopologyScene::default();
    for index in 0..8 {
        add_hole(
            &mut scene,
            index + 1,
            PeriodicCubicSpline::rounded(
                Point2::new(
                    -0.66 + (index % 4) as f64 * 0.44,
                    -0.4 + (index / 4) as f64 * 0.8,
                ),
                0.12,
            ),
        );
    }
    let mesh = mesh_with(
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

#[test]
fn closed_wall_duplicates_the_two_traces_and_retains_its_interior() {
    let mesh = mesh(&two_region_scene(SpanBehavior::REFLECTING));
    let adjacency = edge_adjacency(&mesh);
    let exterior = sided(&mesh, 20, CurveTraceSide::Right);
    let interior = sided(&mesh, 20, CurveTraceSide::Left);
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
fn baffle_insertion_avoids_accidental_cfl_slivers() {
    let mut scene = TopologyScene::default();
    scene
        .materials
        .push(medium(2, "Inside", 1.0, [77, 121, 164]));
    scene.regions.push(Region {
        id: RegionId(2),
        material: MaterialId(2),
        frame: MaterialFrame::world(),
    });
    add_loop(
        &mut scene,
        1,
        PeriodicCubicSpline::rounded(Point2::default(), 0.30),
        SpanBehavior::REFLECTING,
        Some(RegionId(2)),
    );
    add_baffle(
        &mut scene,
        2,
        OpenCubicSpline::uniform(vec![
            Point2::new(-0.841_274_799_6, -0.460_310_562_0),
            Point2::new(-0.702_926_820_9, -0.393_873_880_9),
            Point2::new(-0.564_578_842_4, -0.327_437_199_8),
            Point2::new(-0.426_230_863_7, -0.261_000_518_7),
        ])
        .unwrap(),
    );
    let options = MeshingOptions {
        curve_tolerance: 0.0015,
        target_edge_length: 0.08 / 1.05,
        minimum_angle_degrees: 18.0,
        ..MeshingOptions::default()
    };
    let plan = plan(&scene, 23, options);
    let mesh = mesh_topology_plan(&plan, 23, options).unwrap();
    assert!(mesh.quality.minimum_angle_degrees > 15.0);
    let operator = QuadraticWaveOperator::assemble_topology(
        &mesh,
        &plan,
        TopologyWaveModel::from_topology_scene(&scene),
    )
    .unwrap();
    assert!(operator.recommended_time_step() > 0.002);
}

#[test]
fn multiple_open_baffles_keep_independent_labeled_faces() {
    let mut scene = TopologyScene::default();
    for (id, y) in [(3, -0.38), (8, 0.42)] {
        add_baffle(
            &mut scene,
            id,
            OpenCubicSpline::uniform(vec![
                Point2::new(-0.62, y),
                Point2::new(-0.2, y + 0.05),
                Point2::new(0.2, y - 0.04),
                Point2::new(0.62, y),
            ])
            .unwrap(),
        );
    }
    let mesh = mesh(&scene);
    assert_mesh_invariants(&mesh, 2);
    for id in [3, 8] {
        let left = sided(&mesh, id, CurveTraceSide::Left).len();
        let right = sided(&mesh, id, CurveTraceSide::Right).len();
        assert_eq!(left, right);
        assert!(left >= 2);
    }
}

/// The default caps stop the empty 2 × 2 domain short of an edge of 0.032:
/// refinement runs out of insertions with edges still longer than asked, and
/// says so. The caps sized for the domain's area mesh it, and for a small
/// domain they are the defaults.
#[test]
fn caps_sized_for_the_domain_mesh_what_the_defaults_refuse() {
    let scene = TopologyScene::default();
    let domain = scene.geometry.domain;
    let area = domain.width() * domain.height();
    let options = MeshingOptions {
        target_edge_length: 0.032,
        ..Default::default()
    };
    let refused = mesh_with(&scene, 0, options).unwrap_err();
    assert!(
        matches!(
            refused,
            MeshError::RefinementLimit {
                insertions: 8_000,
                ..
            }
        ),
        "{refused}"
    );
    assert!(
        refused.to_string().contains("against the 0.032 asked for"),
        "{refused}"
    );
    let sized = options.sized_for_area(area);
    let mesh = mesh_with(&scene, 0, sized).unwrap();
    assert!(
        mesh.quality.maximum_edge_length <= 0.032 * 1.1,
        "{:?}",
        mesh.quality
    );
    assert!(mesh.triangles.len() < sized.max_triangles / 3);
    assert_eq!(
        MeshingOptions::default().sized_for_area(0.01),
        MeshingOptions::default()
    );
}
