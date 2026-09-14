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

#[test]
fn repeated_knot_control_removal_is_bounded() {
    let points = vec![
        Point2::new(-0.5, -0.5),
        Point2::new(0.5, -0.5),
        Point2::new(0.5, 0.5),
        Point2::new(-0.5, 0.5),
    ];
    let mut periodic = PeriodicCubicSpline::polygon(points.clone()).unwrap();
    let periodic_before = periodic.clone();
    assert_eq!(
        periodic.remove(periodic.controls().len() - 1),
        Err(SplineError::Index)
    );
    assert_eq!(periodic, periodic_before);

    let mut open = OpenCubicSpline::polyline(points).unwrap();
    let open_before = open.clone();
    assert_eq!(
        open.remove(open.controls().len() - 1),
        Err(SplineError::Index)
    );
    assert_eq!(open, open_before);
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
fn repeated_knots_preserve_open_and_periodic_curves_and_split_exactly() {
    let original = open_irregular();
    let mut refined = original.clone();
    refined.increase_multiplicity(1).unwrap();
    refined.increase_multiplicity(1).unwrap();
    assert_eq!(refined.continuity(1), Some(0));
    assert_eq!(refined.intervals(), original.intervals());
    for index in 0..=400 {
        let parameter = index as f64 * original.period() / 400.0;
        near(
            refined.evaluate(parameter),
            original.evaluate(parameter),
            2.0e-11,
        );
    }
    let (left, right) = refined.split(1).unwrap();
    for index in 0..=200 {
        let parameter = index as f64 * left.period() / 200.0;
        near(
            left.evaluate(parameter),
            original.evaluate(parameter),
            2.0e-11,
        );
    }
    for index in 0..=200 {
        let parameter = index as f64 * right.period() / 200.0;
        near(
            right.evaluate(parameter),
            original.evaluate(parameter + left.period()),
            2.0e-11,
        );
    }
    let joined = left.join(right, 1.0e-12).unwrap();
    for index in 0..=400 {
        let parameter = index as f64 * original.period() / 400.0;
        near(
            joined.evaluate(parameter),
            original.evaluate(parameter),
            2.0e-11,
        );
    }

    let periodic = irregular();
    for breakpoint in [0, 2] {
        let mut cornered = periodic.clone();
        cornered.increase_multiplicity(breakpoint).unwrap();
        cornered.increase_multiplicity(breakpoint).unwrap();
        assert_eq!(cornered.continuity(breakpoint), Some(0));
        assert_eq!(cornered.intervals(), periodic.intervals());
        for index in 0..500 {
            let parameter = index as f64 * periodic.period() / 500.0;
            near(
                cornered.evaluate(parameter),
                periodic.evaluate(parameter),
                3.0e-10,
            );
        }
    }
}

#[test]
fn knot_removal_is_exact_when_possible_and_reports_approximation_error() {
    let open = open_irregular();
    let mut refined = open.clone();
    refined.increase_multiplicity(1).unwrap();
    refined.increase_multiplicity(1).unwrap();
    refined.decrease_multiplicity(1, 1.0e-10).unwrap();
    refined.decrease_multiplicity(1, 1.0e-10).unwrap();
    assert_eq!(refined.multiplicities(), open.multiplicities());
    for index in 0..=200 {
        let parameter = index as f64 * open.period() / 200.0;
        near(
            refined.evaluate(parameter),
            open.evaluate(parameter),
            2.0e-10,
        );
    }

    let periodic = irregular();
    for breakpoint in [0, 3] {
        let mut refined = periodic.clone();
        refined.increase_multiplicity(breakpoint).unwrap();
        refined.increase_multiplicity(breakpoint).unwrap();
        refined.decrease_multiplicity(breakpoint, 1.0e-10).unwrap();
        refined.decrease_multiplicity(breakpoint, 1.0e-10).unwrap();
        assert_eq!(refined.multiplicities(), periodic.multiplicities());
        for index in 0..200 {
            let parameter = index as f64 * periodic.period() / 200.0;
            near(
                refined.evaluate(parameter),
                periodic.evaluate(parameter),
                3.0e-10,
            );
        }
    }

    let mut edited = open.clone();
    edited.increase_multiplicity(1).unwrap();
    edited.increase_multiplicity(1).unwrap();
    let corner_control = edited.span_control_indices(0).unwrap()[3];
    let moved = edited.controls()[corner_control] + Point2::new(0.03, -0.02);
    edited.set_control(corner_control, moved).unwrap();
    let before = edited.clone();
    assert_eq!(
        edited.decrease_multiplicity(1, 1.0e-10),
        Err(SplineError::NotRemovable)
    );
    assert_eq!(edited, before);

    let displacement_bound = edited.decrease_multiplicity_approximate(1).unwrap();
    assert!(displacement_bound > 0.0);
    assert_eq!(edited.continuity(1), Some(1));
    let maximum_sampled_displacement = (0..=500)
        .map(|index| {
            let parameter = index as f64 * before.period() / 500.0;
            (edited.evaluate(parameter) - before.evaluate(parameter)).norm()
        })
        .fold(0.0_f64, f64::max);
    assert!(maximum_sampled_displacement <= displacement_bound * (1.0 + 1.0e-12));

    let second_bound = edited.decrease_multiplicity_approximate(1).unwrap();
    assert!(second_bound > 0.0);
    assert_eq!(edited.continuity(1), Some(2));

    let mut edited_periodic = periodic.clone();
    edited_periodic.increase_multiplicity(0).unwrap();
    edited_periodic.increase_multiplicity(0).unwrap();
    let seam_control = edited_periodic
        .span_control_indices(edited_periodic.intervals().len() - 1)
        .unwrap()[3];
    let moved = edited_periodic.controls()[seam_control] + Point2::new(-0.015, 0.025);
    edited_periodic.set_control(seam_control, moved).unwrap();
    let before_periodic = edited_periodic.clone();
    assert_eq!(
        edited_periodic.decrease_multiplicity(0, 1.0e-10),
        Err(SplineError::NotRemovable)
    );
    let seam_bound = edited_periodic
        .decrease_multiplicity_approximate(0)
        .unwrap();
    assert!(seam_bound > 0.0);
    assert_eq!(edited_periodic.continuity(0), Some(1));
    let seam_sampled_displacement = (0..500)
        .map(|index| {
            let parameter = index as f64 * before_periodic.period() / 500.0;
            (edited_periodic.evaluate(parameter) - before_periodic.evaluate(parameter)).norm()
        })
        .fold(0.0_f64, f64::max);
    assert!(seam_sampled_displacement <= seam_bound * (1.0 + 1.0e-12));
}

#[test]
fn open_spline_rejects_malformed_inputs() {
    assert!(OpenCubicSpline::uniform(vec![Point2::default(); 3]).is_err());
    assert!(OpenCubicSpline::new(vec![Point2::default(); 4], vec![1.0, 1.0]).is_err());
    assert!(OpenCubicSpline::new(vec![Point2::default(); 4], vec![0.0]).is_err());
    assert!(OpenCubicSpline::new(vec![Point2::new(f64::NAN, 0.0); 4], vec![1.0]).is_err());
}

#[test]
fn polyline_constructor_interpolates_exact_straight_pieces() {
    let vertices = vec![
        Point2::new(-0.8, -0.4),
        Point2::new(-0.1, 0.5),
        Point2::new(0.75, 0.15),
    ];
    let spline = OpenCubicSpline::polyline(vertices.clone()).unwrap();
    assert_eq!(spline.multiplicities(), &[3]);
    for (index, edge) in vertices.windows(2).enumerate() {
        let [start, end] = spline.span_bounds(index).unwrap();
        near(spline.evaluate(start), edge[0], 1.0e-12);
        near(spline.evaluate(end), edge[1], 1.0e-12);
        for step in 0..=8 {
            let fraction = step as f64 / 8.0;
            near(
                spline.evaluate(start + (end - start) * fraction),
                edge[0].lerp(edge[1], fraction),
                1.0e-12,
            );
        }
    }
    assert!(OpenCubicSpline::polyline(vec![vertices[0]]).is_err());
    assert!(OpenCubicSpline::polyline(vec![vertices[0], vertices[0]]).is_err());
    assert!(OpenCubicSpline::polyline(vec![Point2::default(); 44]).is_err());
}

#[test]
fn polygon_constructor_interpolates_exact_closed_pieces() {
    let vertices = vec![
        Point2::new(-0.6, -0.4),
        Point2::new(0.7, -0.4),
        Point2::new(0.4, 0.65),
        Point2::new(-0.5, 0.45),
    ];
    let spline = PeriodicCubicSpline::polygon(vertices.clone()).unwrap();
    assert_eq!(spline.multiplicities(), &[3, 3, 3, 3]);
    for index in 0..vertices.len() {
        let edge = [vertices[index], vertices[(index + 1) % vertices.len()]];
        let [start, end] = spline.span_bounds(index).unwrap();
        near(spline.evaluate(start), edge[0], 1.0e-12);
        near(spline.evaluate(end), edge[1], 1.0e-12);
        for step in 0..=8 {
            let fraction = step as f64 / 8.0;
            near(
                spline.evaluate(start + (end - start) * fraction),
                edge[0].lerp(edge[1], fraction),
                1.0e-12,
            );
        }
    }
    near(
        spline.evaluate(0.0),
        spline.evaluate(spline.period()),
        1.0e-12,
    );
    assert!(PeriodicCubicSpline::polygon(vertices[..2].to_vec()).is_err());
    assert!(PeriodicCubicSpline::polygon(vec![vertices[0], vertices[1], vertices[1]]).is_err());
    assert!(PeriodicCubicSpline::polygon(vec![Point2::default(); 43]).is_err());
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
    assert!(
        matches!(
            validate(&contact).issue,
            Some(ValidationIssue::BoundaryContact(_))
        ),
        "{:?}",
        validate(&contact).issue
    );

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
fn validation_uses_scene_domain_and_scale_relative_clearance() {
    let mut custom = scene(vec![PeriodicCubicSpline::rounded(
        Point2::new(2.0, 3.0),
        0.15,
    )]);
    custom.domain = DomainRect::new(1.0, 3.0, 2.0, 4.0);
    assert!(validate(&custom).valid());

    custom.domain = DomainRect::new(1.0, 2.05, 2.0, 4.0);
    assert!(matches!(
        validate(&custom).issue,
        Some(ValidationIssue::Outside(_))
    ));
    assert!(!DomainRect::new(0.0, 0.01, -1.0, 1.0).valid());
    assert!(!DomainRect::new(0.0, f64::INFINITY, -1.0, 1.0).valid());
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

fn junction_branch(
    id: u64,
    node: u64,
    center_junction: JunctionId,
    outer_junction: JunctionId,
    end: Point2,
    left: RegionId,
    right: RegionId,
) -> MaterialInterface {
    MaterialInterface {
        id: MaterialInterfaceId(id),
        spline: InterfaceSpline::Open(
            OpenCubicSpline::polyline(vec![Point2::default(), end]).unwrap(),
        ),
        nodes: vec![
            InterfaceNode {
                id: InterfaceNodeId(node),
                junction: Some(center_junction),
            },
            InterfaceNode {
                id: InterfaceNodeId(node + 1),
                junction: Some(outer_junction),
            },
        ],
        span_sides: vec![InterfaceSpanSides { left, right }],
    }
}

#[test]
fn explicit_three_region_junction_has_consistent_sector_order() {
    let mut scene = Scene::default();
    for id in 2..=3 {
        scene.regions.push(Region {
            id: RegionId(id),
            material: DEFAULT_MATERIAL,
            frame: MaterialFrame::world(),
        });
    }
    let center = JunctionId(1);
    scene.junctions = vec![
        Junction {
            id: center,
            location: JunctionLocation::Interior,
        },
        Junction {
            id: JunctionId(2),
            location: JunctionLocation::Outer {
                side: OuterSide::Right,
                fraction: 0.5,
            },
        },
        Junction {
            id: JunctionId(3),
            location: JunctionLocation::Outer {
                side: OuterSide::Left,
                fraction: 0.1,
            },
        },
        Junction {
            id: JunctionId(4),
            location: JunctionLocation::Outer {
                side: OuterSide::Left,
                fraction: 0.9,
            },
        },
    ];
    scene.material_interfaces = vec![
        junction_branch(
            1,
            1,
            center,
            JunctionId(2),
            Point2::new(1.0, 0.0),
            RegionId(2),
            RegionId(1),
        ),
        junction_branch(
            2,
            3,
            center,
            JunctionId(3),
            Point2::new(-1.0, 0.8),
            RegionId(3),
            RegionId(2),
        ),
        junction_branch(
            3,
            5,
            center,
            JunctionId(4),
            Point2::new(-1.0, -0.8),
            RegionId(1),
            RegionId(3),
        ),
    ];

    assert!(validate(&scene).valid(), "{:?}", validate(&scene));
}

#[test]
fn free_transmitting_endpoint_is_an_incomplete_draft() {
    let mut scene = Scene::default();
    scene.regions.push(Region {
        id: RegionId(2),
        material: DEFAULT_MATERIAL,
        frame: MaterialFrame::world(),
    });
    let mut interface = junction_branch(
        1,
        1,
        JunctionId(1),
        JunctionId(2),
        Point2::new(1.0, 0.0),
        RegionId(2),
        RegionId(1),
    );
    interface.nodes[1].junction = None;
    scene.material_interfaces.push(interface);
    scene.junctions.push(Junction {
        id: JunctionId(1),
        location: JunctionLocation::Outer {
            side: OuterSide::Left,
            fraction: 0.5,
        },
    });

    assert!(matches!(
        validate(&scene).issue,
        Some(ValidationIssue::FreeInterfaceEnd(MaterialInterfaceId(1)))
    ));
}

/// A face centroid must sit inside the face, with its holes removed rather than
/// averaged in.
#[test]
fn compiled_face_centroid_excludes_holes() {
    let square = CompiledFace {
        id: FaceId(1),
        cycles: vec![vec![
            Point2::new(-1.0, -1.0),
            Point2::new(1.0, -1.0),
            Point2::new(1.0, 1.0),
            Point2::new(-1.0, 1.0),
        ]],
        boundaries: vec![],
        area: 4.0,
    };
    let centroid = square.centroid().unwrap();
    assert!(centroid.norm() < 1.0e-12, "{centroid:?}");

    let mut with_hole = square.clone();
    // A clockwise hole in the right half pulls the centroid left.
    with_hole.cycles.push(vec![
        Point2::new(0.2, -0.6),
        Point2::new(0.2, 0.6),
        Point2::new(0.9, 0.6),
        Point2::new(0.9, -0.6),
    ]);
    let shifted = with_hole.centroid().unwrap();
    assert!(
        shifted.x < -0.05,
        "hole did not shift the centroid: {shifted:?}"
    );
    assert!(shifted.y.abs() < 1.0e-9);

    let degenerate = CompiledFace {
        id: FaceId(2),
        cycles: vec![vec![Point2::new(3.0, 4.0), Point2::new(3.0, 4.0)]],
        boundaries: vec![],
        area: 0.0,
    };
    assert_eq!(degenerate.centroid(), Some(Point2::new(3.0, 4.0)));
}

/// Cutting a loop open must reproduce the same curve, sampled from the cut.
#[test]
fn periodic_open_at_reproduces_the_loop_from_its_cut() {
    let loop_spline = PeriodicCubicSpline::rounded(Point2::new(0.2, -0.1), 0.4);
    for breakpoint in 0..loop_spline.intervals().len() {
        let opened = loop_spline
            .clone()
            .open_at(breakpoint)
            .unwrap_or_else(|error| panic!("open_at({breakpoint}) failed: {error}"));
        assert_eq!(opened.intervals().len(), loop_spline.intervals().len());
        assert_eq!(
            opened.multiplicities().len() + 1,
            opened.intervals().len(),
            "open splines carry one multiplicity per interior breakpoint"
        );
        let offset = loop_spline.span_bounds(breakpoint).unwrap()[0];
        let period = loop_spline.period();
        assert!((opened.period() - period).abs() < 1.0e-12);
        for step in 0..=240 {
            let local = period * step as f64 / 240.0;
            let expected = loop_spline.evaluate((offset + local) % period);
            let actual = opened.evaluate(local);
            assert!(
                (actual - expected).norm() < 1.0e-9,
                "cut at {breakpoint} drifted by {:.3e} at t={local:.4}",
                (actual - expected).norm()
            );
        }
        // Both clamped ends land on the breakpoint they were cut at.
        let corner = loop_spline.evaluate(offset);
        assert!((opened.evaluate(0.0) - corner).norm() < 1.0e-9);
        assert!((opened.evaluate(opened.period()) - corner).norm() < 1.0e-9);
    }
}

/// A pre-existing corner must survive the cut, and an out-of-range breakpoint
/// must be refused rather than wrapped.
#[test]
fn periodic_open_at_keeps_corners_and_rejects_bad_breakpoints() {
    let polygon = PeriodicCubicSpline::polygon(vec![
        Point2::new(-0.4, -0.3),
        Point2::new(0.5, -0.3),
        Point2::new(0.5, 0.4),
        Point2::new(-0.4, 0.4),
    ])
    .unwrap();
    let period = polygon.period();
    let opened = polygon.clone().open_at(2).unwrap();
    let offset = polygon.span_bounds(2).unwrap()[0];
    for step in 0..=200 {
        let local = period * step as f64 / 200.0;
        let expected = polygon.evaluate((offset + local) % period);
        assert!((opened.evaluate(local) - expected).norm() < 1.0e-9);
    }
    assert_eq!(
        polygon.clone().open_at(polygon.intervals().len()),
        Err(SplineError::Index)
    );

    // Cutting at a C2 knot inserts the corner without moving the curve.
    let smooth = PeriodicCubicSpline::uniform(vec![
        Point2::new(-0.3, 0.0),
        Point2::new(0.0, -0.35),
        Point2::new(0.35, 0.0),
        Point2::new(0.0, 0.3),
        Point2::new(-0.15, 0.2),
    ])
    .unwrap();
    assert_eq!(smooth.continuity(1), Some(2));
    let opened = smooth.clone().open_at(1).unwrap();
    let offset = smooth.span_bounds(1).unwrap()[0];
    let period = smooth.period();
    for step in 0..=200 {
        let local = period * step as f64 / 200.0;
        let expected = smooth.evaluate((offset + local) % period);
        assert!(
            (opened.evaluate(local) - expected).norm() < 1.0e-9,
            "C2 cut drifted at t={local:.4}"
        );
    }
}

/// Closing an opened loop must give the loop back, rotated to the cut, with a
/// C0 seam at parameter 0; ends that do not meet and single spans are refused.
#[test]
fn open_close_round_trips_the_loop_and_refuses_bad_input() {
    let polygon = PeriodicCubicSpline::polygon(vec![
        Point2::new(-0.4, -0.3),
        Point2::new(0.5, -0.3),
        Point2::new(0.5, 0.4),
        Point2::new(-0.4, 0.4),
    ])
    .unwrap();
    for loop_spline in [irregular(), polygon] {
        let period = loop_spline.period();
        for breakpoint in 0..loop_spline.intervals().len() {
            let offset = loop_spline.span_bounds(breakpoint).unwrap()[0];
            let closed = loop_spline
                .clone()
                .open_at(breakpoint)
                .unwrap()
                .close(1.0e-12)
                .unwrap_or_else(|error| panic!("close after open_at({breakpoint}): {error}"));
            assert_eq!(closed.intervals().len(), loop_spline.intervals().len());
            assert_eq!(closed.continuity(0), Some(0), "the seam is a corner");
            assert!((closed.period() - period).abs() < 1.0e-12);
            for step in 0..=240 {
                let local = period * step as f64 / 240.0;
                let expected = loop_spline.evaluate((offset + local) % period);
                assert!(
                    (closed.evaluate(local) - expected).norm() < 1.0e-9,
                    "cut {breakpoint} drifted at t={local:.4}"
                );
            }
        }
    }
    assert_eq!(
        open_irregular().close(1.0e-9),
        Err(SplineError::InvalidInterval)
    );
    let single = OpenCubicSpline::new(
        vec![
            Point2::new(0.0, 0.0),
            Point2::new(0.5, 0.5),
            Point2::new(-0.5, 0.5),
            Point2::new(0.0, 0.0),
        ],
        vec![1.0],
    )
    .unwrap();
    assert_eq!(single.close(1.0e-9), Err(SplineError::ControlCount));
}

/// Reversal maps `t` to `period - t` exactly and undoes itself.
#[test]
fn open_reversed_maps_parameter_exactly_and_is_an_involution() {
    let spline = open_irregular();
    let reversed = spline.reversed();
    let period = spline.period();
    assert!((reversed.period() - period).abs() < 1.0e-12);
    for step in 0..=240 {
        let t = period * step as f64 / 240.0;
        near(reversed.evaluate(t), spline.evaluate(period - t), 1.0e-12);
    }
    assert_eq!(reversed.reversed(), spline);
}

/// `join` checks the gap before the control budget, and enforces both.
#[test]
fn join_refuses_gaps_and_control_overflow() {
    let apart = OpenCubicSpline::new(
        vec![
            Point2::new(2.0, 0.0),
            Point2::new(2.5, 0.5),
            Point2::new(3.0, 0.0),
            Point2::new(3.5, 0.5),
        ],
        vec![1.0],
    )
    .unwrap();
    assert_eq!(
        open_irregular().join(apart, 1.0e-9),
        Err(SplineError::InvalidInterval)
    );
    let line = |start: f64| {
        OpenCubicSpline::new(
            (0..65)
                .map(|index| Point2::new(start + index as f64 * 0.01, 0.0))
                .collect(),
            vec![1.0; 62],
        )
        .unwrap()
    };
    let first = line(0.0);
    let second = line(first.evaluate(first.period()).x);
    assert_eq!(first.join(second, 1.0e-9), Err(SplineError::ControlCount));
}

/// Reversing a topology curve keeps every face's law by swapping the sides,
/// and reverses spans and nodes with the spline.
#[test]
fn topology_curve_reversal_swaps_sides_and_reverses_spans_and_nodes() {
    let wall = SpanBehavior::Separated {
        left: FaceBoundaryCondition::Reflecting,
        right: FaceBoundaryCondition::Impedance { ratio: 1.0 },
        coupling: InternalBoundaryCoupling::Independent,
    };
    let mut curve = TopologyCurve::new(
        CurveId(7),
        CurveSpline::Open(open_irregular()),
        vec![
            CurveSpan {
                id: CurveSpanId(1),
                behavior: SpanBehavior::Transmitting,
            },
            CurveSpan {
                id: CurveSpanId(2),
                behavior: wall,
            },
            CurveSpan {
                id: CurveSpanId(3),
                behavior: SpanBehavior::REFLECTING,
            },
        ],
    )
    .unwrap();
    curve.nodes[0].vertex = Some(TopologyVertexId(4));
    let reversed = curve.reversed().unwrap();
    assert_eq!(reversed.id, curve.id);
    assert_eq!(
        reversed
            .spans
            .iter()
            .map(|span| span.id)
            .collect::<Vec<_>>(),
        vec![CurveSpanId(3), CurveSpanId(2), CurveSpanId(1)]
    );
    assert_eq!(
        reversed.spans[1].behavior,
        SpanBehavior::Separated {
            left: FaceBoundaryCondition::Impedance { ratio: 1.0 },
            right: FaceBoundaryCondition::Reflecting,
            coupling: InternalBoundaryCoupling::Independent,
        }
    );
    assert_eq!(reversed.spans[2].behavior, SpanBehavior::Transmitting);
    assert_eq!(reversed.nodes[3].vertex, Some(TopologyVertexId(4)));
    assert_eq!(reversed.nodes[0].vertex, None);
    assert_eq!(
        SpanBehavior::Transmitting.mirrored(),
        SpanBehavior::Transmitting
    );
    assert_eq!(wall.mirrored().mirrored(), wall);
    assert_eq!(CurveTraceSide::Left.opposite(), CurveTraceSide::Right);
    let loop_curve = TopologyCurve::new(
        CurveId(8),
        CurveSpline::Closed(irregular()),
        (1..=5)
            .map(|index| CurveSpan {
                id: CurveSpanId(index),
                behavior: SpanBehavior::Transmitting,
            })
            .collect(),
    )
    .unwrap();
    assert_eq!(loop_curve.reversed(), Err(TopologyIssue::Structure));
}

/// Every error a user can read in the status bar is a sentence, not a debug
/// dump. The structure behind it stays available through `Debug` for the
/// diagnostics window.
#[test]
fn user_facing_errors_read_as_sentences() {
    let mut messages: Vec<String> = vec![];
    for issue in [
        TopologyIssue::Structure,
        TopologyIssue::WorkLimit,
        TopologyIssue::Sampling(CurveId(8)),
        TopologyIssue::MissingVertex {
            curve: CurveId(1),
            vertex: TopologyVertexId(2),
        },
        TopologyIssue::NonC0Attachment {
            curve: CurveId(1),
            node: 3,
        },
        TopologyIssue::VertexMismatch {
            curve: CurveId(1),
            vertex: TopologyVertexId(2),
        },
        TopologyIssue::Outside(CurveId(3)),
        TopologyIssue::Degenerate(CurveId(3)),
        TopologyIssue::NearContact {
            first: CompiledEdgeSource::Curve(CurveSpanId(57)),
            second: CompiledEdgeSource::Curve(CurveSpanId(65)),
        },
        TopologyIssue::Overlap {
            first: CompiledEdgeSource::Outer(OuterSide::Bottom),
            second: CompiledEdgeSource::Curve(CurveSpanId(4)),
        },
        TopologyIssue::IncompatibleCrossing {
            first: CompiledEdgeSource::Curve(CurveSpanId(1)),
            second: CompiledEdgeSource::Curve(CurveSpanId(2)),
        },
        TopologyIssue::FreeTransmittingEnd(CurveId(2)),
        TopologyIssue::TooManySegments,
        TopologyIssue::TooManyFaces,
    ] {
        assert_ne!(
            issue.to_string(),
            format!("{issue:?}"),
            "{issue:?} still prints its debug form"
        );
        messages.push(issue.to_string());
    }
    for error in [
        SplineError::ControlCount,
        SplineError::IntervalCount,
        SplineError::NonFinite,
        SplineError::InvalidInterval,
        SplineError::IllConditionedKnots,
        SplineError::NotRemovable,
        SplineError::Index,
    ] {
        assert_ne!(
            error.to_string(),
            format!("{error:?}"),
            "{error:?} still prints its debug form"
        );
        messages.push(error.to_string());
    }
    for message in &messages {
        assert!(
            !message.contains(['{', '}', '"', '(']) && !message.contains("::"),
            "not a sentence: {message}"
        );
        assert!(
            message.split_whitespace().count() >= 4,
            "too terse to read: {message}"
        );
    }
}
