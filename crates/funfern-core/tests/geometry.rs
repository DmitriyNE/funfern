use funfern_core::*;
fn near(a: Point2, b: Point2, tol: f64) {
    assert!((a - b).norm() < tol, "{a:?} != {b:?}");
}
fn irregular() -> PeriodicCubicSpline {
    PeriodicCubicSpline::new(
        vec![
            Point2::new(-0.5, 0.0),
            Point2::new(-0.2, 0.6),
            Point2::new(0.6, 0.3),
            Point2::new(0.5, -0.4),
            Point2::new(-0.2, -0.5),
        ],
        vec![0.7, 1.3, 0.4, 1.8, 0.9],
    )
    .unwrap()
}
#[test]
fn periodic_derivatives_and_finite_differences() {
    let s = irregular();
    for o in 0..=2 {
        near(s.derivative(-1e-8, o), s.derivative(1e-8, o), 1e-6);
        for i in 0..100 {
            let t = i as f64 * 0.051;
            near(s.derivative(t, o), s.derivative(t + s.period(), o), 1e-12);
        }
    }
    for i in 0..100 {
        let t = i as f64 * 0.051;
        for o in 1..=2 {
            near(
                s.derivative(t, o),
                (s.derivative(t + 1e-6, o - 1) - s.derivative(t - 1e-6, o - 1)) / 2e-6,
                1e-5,
            )
        }
    }
}
#[test]
fn independent_uniform_formula() {
    let s = PeriodicCubicSpline::uniform(irregular().controls().to_vec()).unwrap();
    let n = s.controls().len();
    for k in 0..n {
        for i in 0..30 {
            let u = i as f64 / 30.0;
            let weights = [
                (1.0 - u).powi(3) / 6.0,
                (3.0 * u.powi(3) - 6.0 * u * u + 4.0) / 6.0,
                (-3.0 * u.powi(3) + 3.0 * u * u + 3.0 * u + 1.0) / 6.0,
                u.powi(3) / 6.0,
            ];
            let p = (0..4).fold(Point2::default(), |p, j| {
                p + s.controls()[(k + n - 3 + j) % n] * weights[j]
            });
            near(s.evaluate(k as f64 + u), p, 1e-12);
        }
    }
}
#[test]
fn affine_invariance() {
    let s = irregular();
    let f = |p: Point2| Point2::new(2.0 * p.x + 0.3 * p.y + 2.0, -0.5 * p.x + 0.8 * p.y - 1.0);
    let q = PeriodicCubicSpline::new(
        s.controls().iter().copied().map(f).collect(),
        s.intervals().to_vec(),
    )
    .unwrap();
    for i in 0..100 {
        let t = i as f64 / 20.0;
        near(f(s.evaluate(t)), q.evaluate(t), 1e-12);
    }
}
#[test]
fn repeated_insertion_preserves_shape_and_derivatives_including_seam() {
    let s = irregular();
    let mut q = s.clone();
    for t in [0.0001, 5.0999, 0.25, 3.9, 1.5, 0.00005, 5.09995, 0.125, 2.1] {
        assert!(matches!(q.insert(t).unwrap(), Insertion::Inserted(_)));
        for i in 0..1000 {
            let t = i as f64 * s.period() / 1000.0;
            for o in 0..=2 {
                near(s.derivative(t, o), q.derivative(t, o), 2e-7);
            }
        }
    }
    let n = q.controls().len();
    assert!(matches!(q.insert(0.0).unwrap(), Insertion::Existing(0)));
    assert_eq!(n, q.controls().len());
}
#[test]
fn removal_and_malformed_inputs() {
    let mut s = irregular();
    s.remove(0).unwrap();
    assert!(s.remove(0).is_err());
    assert!(s.set_control(0, Point2::new(f64::NAN, 0.0)).is_err());
    assert!(PeriodicCubicSpline::uniform(vec![Point2::default(); 3]).is_err());
    assert!(
        PeriodicCubicSpline::new(vec![Point2::default(); 4], vec![1.0, 0.0, 1.0, 1.0]).is_err()
    );
    assert!(PeriodicCubicSpline::new(vec![Point2::default(); 4], vec![1.0; 5]).is_err());
    assert!(PeriodicCubicSpline::new(vec![Point2::default(); 4], vec![f64::INFINITY; 4]).is_err());
}

fn open_irregular() -> OpenCubicSpline {
    OpenCubicSpline::new(
        vec![
            Point2::new(-0.8, -0.2),
            Point2::new(-0.45, 0.5),
            Point2::new(-0.05, -0.35),
            Point2::new(0.35, 0.45),
            Point2::new(0.8, 0.1),
            Point2::new(0.9, -0.25),
        ],
        vec![0.7, 1.4, 0.9],
    )
    .unwrap()
}

#[test]
fn open_spline_has_distinct_endpoints_and_correct_derivatives() {
    let spline = open_irregular();
    near(spline.evaluate(0.0), spline.controls()[0], 1.0e-14);
    near(
        spline.evaluate(spline.period()),
        *spline.controls().last().unwrap(),
        1.0e-14,
    );
    for index in 1..100 {
        let parameter = index as f64 * spline.period() / 100.0;
        for order in 1..=2 {
            near(
                spline.derivative(parameter, order),
                (spline.derivative(parameter + 1.0e-6, order - 1)
                    - spline.derivative(parameter - 1.0e-6, order - 1))
                    / 2.0e-6,
                2.0e-4,
            );
        }
    }
}

#[test]
fn open_spline_insertion_is_shape_preserving_and_affine_invariant() {
    let original = open_irregular();
    let affine = |point: Point2| {
        Point2::new(
            1.7 * point.x - 0.2 * point.y + 0.4,
            0.3 * point.x + 0.9 * point.y - 0.1,
        )
    };
    let transformed = OpenCubicSpline::new(
        original.controls().iter().copied().map(affine).collect(),
        original.intervals().to_vec(),
    )
    .unwrap();
    for index in 0..=100 {
        let parameter = index as f64 * original.period() / 100.0;
        near(
            transformed.evaluate(parameter),
            affine(original.evaluate(parameter)),
            2.0e-12,
        );
    }

    let mut inserted = original.clone();
    for parameter in [0.13, 2.85, 0.8, 1.75] {
        assert!(matches!(
            inserted.insert(parameter).unwrap(),
            Insertion::Inserted(_)
        ));
        for index in 0..=300 {
            let parameter = index as f64 * original.period() / 300.0;
            for order in 0..=2 {
                near(
                    inserted.derivative(parameter, order),
                    original.derivative(parameter, order),
                    2.0e-9,
                );
            }
        }
    }
}

#[test]
fn open_spline_rejects_malformed_inputs() {
    assert!(OpenCubicSpline::uniform(vec![Point2::default(); 3]).is_err());
    assert!(OpenCubicSpline::new(vec![Point2::default(); 4], vec![1.0, 1.0]).is_err());
    assert!(OpenCubicSpline::new(vec![Point2::default(); 4], vec![0.0]).is_err());
    assert!(OpenCubicSpline::new(vec![Point2::new(f64::NAN, 0.0); 4], vec![1.0]).is_err());
}

#[test]
fn open_spline_sampling_keeps_both_endpoints_and_parameter_search() {
    let spline = open_irregular();
    let samples = sample_open(&spline, SamplingOptions::default()).unwrap();
    assert_eq!(samples.first().unwrap().t, 0.0);
    assert_eq!(samples.last().unwrap().t, spline.period());
    near(
        samples.first().unwrap().point,
        spline.controls()[0],
        1.0e-14,
    );
    near(
        samples.last().unwrap().point,
        *spline.controls().last().unwrap(),
        1.0e-14,
    );
    let expected = spline.period() * 0.37;
    let point = spline.evaluate(expected);
    let found = closest_open_parameter(&spline, &samples, point);
    assert!((found - expected).abs() < 1.0e-5);
}

fn open_boundary(id: u64, y: f64) -> InternalBoundary {
    InternalBoundary {
        id: InternalBoundaryId(id),
        spline: OpenCubicSpline::uniform(vec![
            Point2::new(-0.65, y),
            Point2::new(-0.2, y),
            Point2::new(0.2, y),
            Point2::new(0.65, y),
        ])
        .unwrap(),
        region: BACKGROUND_REGION,
        span_laws: vec![InternalBoundaryLaw::REFLECTING],
    }
}

#[test]
fn open_boundary_validation_checks_ends_crossings_and_contacts() {
    let mut valid = Scene::default();
    valid.internal_boundaries.push(open_boundary(1, 0.0));
    assert!(validate(&valid).valid());

    let mut crossing = Scene::default();
    crossing.internal_boundaries.push(InternalBoundary {
        id: InternalBoundaryId(1),
        spline: OpenCubicSpline::uniform(vec![
            Point2::new(-0.7, -0.5),
            Point2::new(0.7, 0.5),
            Point2::new(-0.7, 0.5),
            Point2::new(0.7, -0.5),
        ])
        .unwrap(),
        region: BACKGROUND_REGION,
        span_laws: vec![InternalBoundaryLaw::REFLECTING],
    });
    assert!(matches!(
        validate(&crossing).issue,
        Some(ValidationIssue::BoundarySelfContact(_))
    ));

    let mut contact = valid.clone();
    contact.internal_boundaries.push(open_boundary(2, 0.0001));
    assert!(matches!(
        validate(&contact).issue,
        Some(ValidationIssue::BoundaryContact(_))
    ));

    let mut outside = Scene::default();
    let mut boundary = open_boundary(1, 0.0);
    boundary
        .spline
        .set_control(3, Point2::new(1.1, 0.0))
        .unwrap();
    outside.internal_boundaries.push(boundary);
    assert!(matches!(
        validate(&outside).issue,
        Some(ValidationIssue::BoundaryOutside(_))
    ));
}

#[test]
fn internal_boundary_span_laws_match_knots_and_reject_bad_coefficients() {
    let mut scene = Scene::default();
    scene.internal_boundaries.push(open_boundary(1, 0.0));
    assert!(scene.structure_valid());
    scene.internal_boundaries[0].span_laws.clear();
    assert!(!scene.structure_valid());
    scene.internal_boundaries[0].span_laws = vec![InternalBoundaryLaw {
        left: FaceBoundaryCondition::Impedance { ratio: f64::NAN },
        ..InternalBoundaryLaw::REFLECTING
    }];
    assert!(!scene.structure_valid());
    scene.internal_boundaries[0].span_laws = vec![InternalBoundaryLaw {
        coupling: InternalBoundaryCoupling::ThinGap {
            stiffness_ratio: 0.0,
        },
        ..InternalBoundaryLaw::REFLECTING
    }];
    assert!(!scene.structure_valid());
    scene.internal_boundaries[0].span_laws = vec![InternalBoundaryLaw {
        left: FaceBoundaryCondition::Impedance { ratio: 1.0 },
        coupling: InternalBoundaryCoupling::ThinGap {
            stiffness_ratio: 1.0,
        },
        ..InternalBoundaryLaw::REFLECTING
    }];
    assert!(!scene.structure_valid());
}

#[test]
fn hole_span_conditions_match_knots_validate_and_do_not_change_geometry() {
    let scene = Scene::initial();
    let mut changed = scene.clone();
    changed.obstacles[0].span_conditions[0] = FaceBoundaryCondition::Impedance { ratio: 1.5 };
    assert!(changed.structure_valid());
    assert!(scene.geometry_eq(&changed));

    changed.obstacles[0].span_conditions.pop();
    assert!(!changed.structure_valid());
    changed.obstacles[0].span_conditions =
        vec![FaceBoundaryCondition::Reflecting; changed.obstacles[0].spline.intervals().len()];
    changed.obstacles[0].span_conditions[0] = FaceBoundaryCondition::Impedance { ratio: 0.0 };
    assert!(!changed.structure_valid());
}

fn scene(loops: Vec<PeriodicCubicSpline>) -> Scene {
    Scene {
        obstacles: loops
            .into_iter()
            .enumerate()
            .map(|(i, spline)| Obstacle::hole(ObstacleId(i as u64 + 1), spline))
            .collect(),
        ..Scene::default()
    }
}
#[test]
fn validation_cases() {
    let circle = |x, r| PeriodicCubicSpline::rounded(Point2::new(x, 0.0), r);
    assert!(validate(&scene(vec![circle(-0.3, 0.15), circle(0.3, 0.15)])).valid());
    assert!(matches!(
        validate(&scene(vec![circle(0.98, 0.15)])).issue,
        Some(ValidationIssue::Outside(_))
    ));
    assert!(matches!(
        validate(&scene(vec![circle(0.0, 0.5), circle(0.0, 0.15)])).issue,
        Some(ValidationIssue::Nested(..))
    ));
    assert!(matches!(
        validate(&scene(vec![circle(0.0, 0.3), circle(0.3, 0.3)])).issue,
        Some(ValidationIssue::ObstacleContact(..))
    ));
    let r = circle(0.0, 0.15).evaluate(0.0).norm();
    assert!(
        validate(&scene(vec![
            circle(0.0, 0.15),
            circle(2.0 * r + 0.0001, 0.15)
        ]))
        .issue
        .is_some()
    );
    assert!(matches!(
        validate(&scene(vec![
            PeriodicCubicSpline::uniform(vec![Point2::default(); 4]).unwrap()
        ]))
        .issue,
        Some(ValidationIssue::Degenerate(_))
    ));
    let crossing = PeriodicCubicSpline::uniform(vec![
        Point2::new(-0.8, -0.6),
        Point2::new(0.8, 0.6),
        Point2::new(-0.8, 0.6),
        Point2::new(0.8, -0.6),
        Point2::new(0.5, -0.7),
    ])
    .unwrap();
    assert!(validate(&scene(vec![crossing])).issue.is_some());
}
#[test]
fn sampling_exhaustion_is_not_accepted() {
    let s = Scene::initial();
    let mut job = ValidationJob::with_options(
        s,
        7,
        SamplingOptions {
            tolerance: 1e-12,
            max_depth: 0,
            max_points: 4,
        },
    );
    assert!(matches!(
        job.advance(10).unwrap().issue,
        Some(ValidationIssue::Subdivision(_))
    ));
}
#[test]
fn validation_is_incremental_and_independent_of_render_tolerance() {
    let s = Scene::initial();
    let expected = validate(&s);
    for tolerance in [0.1, 0.01, 0.00001] {
        let _ = sample(
            &s.obstacles[0].spline,
            SamplingOptions {
                tolerance,
                ..Default::default()
            },
        );
        let mut job = ValidationJob::new(s.clone(), 42);
        assert!(job.advance(1).is_none());
        loop {
            if let Some(r) = job.advance(17) {
                assert_eq!(r.issue, expected.issue);
                assert_eq!(r.revision, 42);
                break;
            }
        }
    }
}

#[test]
fn close_knot_insertions_do_not_create_false_self_contacts() {
    let mut scene = Scene::initial();
    for t in [0.00001, 0.00002, 7.99999, 0.99999, 1.00001] {
        let span = scene.obstacles[0].spline.span_index(t).unwrap();
        let inherited = scene.obstacles[0].span_conditions[span];
        if matches!(
            scene.obstacles[0].spline.insert(t).unwrap(),
            Insertion::Inserted(_)
        ) {
            scene.obstacles[0]
                .span_conditions
                .insert(span + 1, inherited);
        }
        assert!(validate(&scene).valid(), "{:?}", validate(&scene));
    }
}

#[test]
fn maximum_scene_validates_with_bounded_frame_slices() {
    let scene = scene(
        (0..32)
            .map(|i| {
                PeriodicCubicSpline::rounded(
                    Point2::new(-0.84 + (i % 8) as f64 * 0.24, -0.6 + (i / 8) as f64 * 0.4),
                    0.08,
                )
            })
            .collect(),
    );
    let mut job = ValidationJob::new(scene, 1);
    for _ in 0..128 {
        if let Some(result) = job.advance(12_000) {
            assert!(result.valid(), "{result:?}");
            return;
        }
    }
    panic!("Representative 32-loop scene exceeded 128 frame slices");
}
