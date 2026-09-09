use femfun_core::*;
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
        scene.obstacles[0].spline.insert(t).unwrap();
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
