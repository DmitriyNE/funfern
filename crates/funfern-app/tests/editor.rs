use funfern_app::{editor::*, persistence::*};
use funfern_core::*;
fn settle(e: &mut Editor) {
    for _ in 0..10000 {
        e.validate_frame(1000);
        if e.acceptance != Acceptance::Pending {
            return;
        }
    }
    panic!("validation failed to terminate")
}
fn move_point(e: &mut Editor, p: Point2) {
    e.begin();
    e.set_point(ObstacleId(1), 0, p).unwrap();
    e.commit();
    settle(e);
}
#[test]
fn invalid_draft_persists_and_recovers() {
    let mut e = Editor::default();
    let accepted = e.document.accepted.clone();
    move_point(&mut e, Point2::new(8.0, 0.0));
    assert!(matches!(e.acceptance, Acceptance::Invalid(_)));
    assert_eq!(e.document.accepted, accepted);
    assert_ne!(e.document.draft, accepted);
    for _ in 0..10 {
        e.validate_frame(1000);
    }
    assert_ne!(e.document.draft, accepted);
    move_point(&mut e, Point2::new(0.15, 0.0));
    assert_eq!(e.acceptance, Acceptance::Valid);
    assert_eq!(e.document.accepted, e.document.draft);
}
#[test]
fn drag_is_one_entry_and_escape_restores_both_scenes() {
    let mut e = Editor::default();
    let original = e.document.clone();
    e.begin();
    for i in 0..20 {
        e.set_point(ObstacleId(1), 0, Point2::new(0.15 + i as f64 * 0.001, 0.0))
            .unwrap();
        settle(&mut e);
    }
    e.commit();
    assert_eq!(e.history_len(), (1, 0));
    let after = e.document.clone();
    e.undo();
    assert_eq!(e.document, original);
    e.redo();
    assert_eq!(e.document, after);
    e.begin();
    e.set_point(ObstacleId(1), 0, Point2::new(0.3, 0.0))
        .unwrap();
    settle(&mut e);
    e.cancel();
    assert_eq!(e.document, after);
    assert_eq!(e.history_len(), (1, 0));
}
#[test]
fn history_preserves_invalid_draft_and_accepted_pair() {
    let mut e = Editor::default();
    let original = e.document.clone();
    move_point(&mut e, Point2::new(8.0, 0.0));
    let invalid = e.document.clone();
    e.revert();
    settle(&mut e);
    assert_eq!(e.document, original);
    e.undo();
    assert_eq!(e.document, invalid);
    settle(&mut e);
    assert_eq!(e.document, invalid);
    e.undo();
    assert_eq!(e.document, original);
    e.redo();
    assert_eq!(e.document, invalid);
    move_point(&mut e, Point2::new(0.2, 0.0));
    assert_eq!(e.history_len().1, 0);
}
#[test]
fn stale_validation_cannot_accept_new_draft() {
    let mut e = Editor::default();
    e.begin();
    e.set_point(ObstacleId(1), 0, Point2::new(0.16, 0.0))
        .unwrap();
    let old = e.revision;
    let mut job = ValidationJob::new(e.document.draft.clone(), old);
    e.set_point(ObstacleId(1), 0, Point2::new(8.0, 0.0))
        .unwrap();
    let result = loop {
        if let Some(result) = job.advance(1000) {
            break result;
        }
    };
    let accepted = e.document.accepted.clone();
    e.apply_validation(result);
    assert_eq!(e.acceptance, Acceptance::Pending);
    assert_eq!(e.document.accepted, accepted);
    settle(&mut e);
    assert!(matches!(e.acceptance, Acceptance::Invalid(_)));
}

#[test]
fn outer_side_conditions_are_undoable_and_round_trip_with_time_signals() {
    let mut editor = Editor::default();
    let original = editor.document.clone();
    let signal = BoundarySignal {
        offset: 0.25,
        amplitude: 0.8,
        frequency_hz: 3.5,
        phase_radians: -0.2,
    };
    editor
        .set_outer_boundary_condition(
            OuterSide::Bottom,
            OuterBoundaryCondition::Dirichlet { signal },
        )
        .unwrap();
    settle(&mut editor);
    editor
        .set_outer_boundary_condition(OuterSide::Top, OuterBoundaryCondition::Neumann { signal })
        .unwrap();
    settle(&mut editor);
    assert_eq!(editor.history_len(), (2, 0));
    assert_eq!(
        editor
            .document
            .accepted
            .outer_boundaries
            .get(OuterSide::Bottom),
        OuterBoundaryCondition::Dirichlet { signal }
    );
    let json = save(&editor.document).unwrap();
    assert_eq!(decode(json.as_bytes()).unwrap(), editor.document);

    let mut version_five: serde_json::Value = serde_json::from_str(&json).unwrap();
    version_five["version"] = 5.into();
    version_five.as_object_mut().unwrap().remove("far_field");
    for scene in ["draft", "accepted"] {
        version_five[scene]
            .as_object_mut()
            .unwrap()
            .remove("outer_boundaries");
    }
    let migrated = decode(serde_json::to_string(&version_five).unwrap().as_bytes()).unwrap();
    assert_eq!(
        migrated.accepted.outer_boundaries,
        OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting)
    );

    editor.undo();
    settle(&mut editor);
    assert_eq!(
        editor
            .document
            .accepted
            .outer_boundaries
            .get(OuterSide::Top),
        OuterBoundaryCondition::SecondOrderOutgoing
    );
    editor.undo();
    settle(&mut editor);
    assert_eq!(editor.document, original);

    let mut malformed: serde_json::Value = serde_json::from_str(&json).unwrap();
    malformed["accepted"]["outer_boundaries"][0]["signal"]["frequency_hz"] = (-1.0).into();
    assert!(decode(serde_json::to_string(&malformed).unwrap().as_bytes()).is_err());
}
#[test]
fn history_bound_and_stable_ids() {
    let mut e = Editor::default();
    for i in 0..105 {
        move_point(&mut e, Point2::new(0.15 + i as f64 * 0.0001, 0.0));
    }
    assert_eq!(e.history_len().0, 100);
    let a = e
        .create(PeriodicCubicSpline::rounded(Point2::new(0.5, 0.0), 0.15))
        .unwrap();
    e.undo();
    let b = e
        .create(PeriodicCubicSpline::rounded(Point2::new(-0.5, 0.0), 0.15))
        .unwrap();
    assert_ne!(a, b);
}
fn decode(bytes: &[u8]) -> Result<Document, String> {
    let mut load = parse(bytes)?;
    loop {
        if let Some(result) = load.advance(1000) {
            return result;
        }
    }
}
#[test]
fn scene_round_trip_keeps_nonuniform_knots_and_invalid_drafts() {
    let mut e = Editor::default();
    e.insert(ObstacleId(1), 0.01).unwrap();
    settle(&mut e);
    move_point(&mut e, Point2::new(8.0, 0.0));
    let json = save(&e.document).unwrap();
    let document = decode(json.as_bytes()).unwrap();
    assert_eq!(document, e.document);
    e.replace_validated(document);
    assert_eq!(e.history_len(), (0, 0));
    settle(&mut e);
    assert!(matches!(e.acceptance, Acceptance::Invalid(_)));
    assert!(!json.contains("selection"));
    assert!(!json.contains("revision"));
}

#[test]
fn line_probes_are_undoable_bounded_and_round_trip() {
    let mut editor = Editor::default();
    let id = editor
        .create_segment_probe(Point2::new(-0.5, 0.2), Point2::new(0.5, 0.2))
        .unwrap();
    assert_eq!(editor.history_len(), (1, 0));
    let mut probe = editor.document.probes[0].clone();
    probe.target = ProbeTarget::Segment {
        start: Point2::new(0.5, 0.2),
        end: Point2::new(-0.5, 0.2),
        preset: ProbeSamplingPreset::High,
    };
    editor.update_probe(probe).unwrap();
    let document = editor.document.clone();
    assert_eq!(
        decode(save(&document).unwrap().as_bytes()).unwrap(),
        document
    );
    editor.undo();
    assert_eq!(editor.document.probes[0].id, id);
    editor.undo();
    assert!(editor.document.probes.is_empty());

    let mut invalid = document.clone();
    invalid.probes[0].target = ProbeTarget::Segment {
        start: Point2::default(),
        end: Point2::default(),
        preset: ProbeSamplingPreset::Medium,
    };
    assert!(
        Editor::default()
            .update_probe(invalid.probes[0].clone())
            .is_err()
    );
}

#[test]
fn area_probe_targets_and_far_field_settings_round_trip() {
    let mut editor = Editor::default();
    let disk = editor
        .create_area_disk_probe(Point2::new(0.35, -0.2), 0.18)
        .unwrap();
    let region = editor
        .create_region_loop(
            PeriodicCubicSpline::rounded(Point2::new(0.55, 0.45), 0.12),
            BACKGROUND_REGION,
            DEFAULT_MATERIAL,
            false,
        )
        .unwrap();
    settle(&mut editor);
    let interior = editor.obstacle(region).unwrap().role.interior().unwrap();
    let attached = editor.create_area_region_probe(interior).unwrap();
    editor
        .set_far_field(FarFieldSettings {
            enabled: true,
            inset: 0.17,
        })
        .unwrap();

    let json = save(&editor.document).unwrap();
    assert!(json.contains("\"version\": 14"));
    let decoded = decode(json.as_bytes()).unwrap();
    assert_eq!(decoded, editor.document);
    assert_eq!(decoded.far_field.inset, 0.17);
    assert!(matches!(
        decoded
            .probes
            .iter()
            .find(|probe| probe.id == disk)
            .unwrap()
            .target,
        ProbeTarget::AreaDisk { radius, .. } if radius == 0.18
    ));

    editor.delete_obstacle(region);
    assert!(editor.document.probes.iter().any(|probe| probe.id == disk));
    assert!(
        !editor
            .document
            .probes
            .iter()
            .any(|probe| probe.id == attached)
    );
    editor.undo();
    assert!(
        editor
            .document
            .probes
            .iter()
            .any(|probe| probe.id == attached)
    );
}

#[test]
fn malformed_area_and_far_field_targets_are_rejected() {
    let mut editor = Editor::default();
    editor
        .create_area_disk_probe(Point2::new(0.2, 0.1), 0.2)
        .unwrap();
    let json = save(&editor.document).unwrap();
    let mut malformed: serde_json::Value = serde_json::from_str(&json).unwrap();
    malformed["probes"][0]["target"]["radius"] = 0.into();
    assert!(decode(serde_json::to_string(&malformed).unwrap().as_bytes()).is_err());

    malformed = serde_json::from_str(&json).unwrap();
    malformed["far_field"]["inset"] = 1.into();
    assert!(decode(serde_json::to_string(&malformed).unwrap().as_bytes()).is_err());

    let mut legacy: serde_json::Value = serde_json::from_str(&json).unwrap();
    legacy["version"] = 12.into();
    legacy.as_object_mut().unwrap().remove("far_field");
    assert!(decode(serde_json::to_string(&legacy).unwrap().as_bytes()).is_err());
}

#[test]
fn version_nine_point_probes_remain_loadable() {
    let mut editor = Editor::default();
    editor.create_point_probe(Point2::new(0.1, -0.2)).unwrap();
    let json = save(&editor.document).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["version"] = 9.into();
    value.as_object_mut().unwrap().remove("source");
    value.as_object_mut().unwrap().remove("far_field");
    assert_eq!(
        decode(serde_json::to_string(&value).unwrap().as_bytes()).unwrap(),
        editor.document
    );
}

#[test]
fn continuous_source_round_trips_and_version_ten_uses_the_default() {
    let document = Document {
        source: SourceSettings {
            enabled: true,
            position: Point2::new(-0.37, 0.28),
            amplitude: 23.0,
            width: 0.045,
            frequency_hz: 3.25,
            region: BACKGROUND_REGION,
        },
        ..Default::default()
    };
    let json = save(&document).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["version"], 14);
    assert_eq!(decode(json.as_bytes()).unwrap(), document);

    value["version"] = 10.into();
    value.as_object_mut().unwrap().remove("source");
    value.as_object_mut().unwrap().remove("far_field");
    let legacy = decode(serde_json::to_string(&value).unwrap().as_bytes()).unwrap();
    assert_eq!(legacy.source, SourceSettings::default());

    let mut malformed: serde_json::Value = serde_json::from_str(&json).unwrap();
    malformed["source"]["width"] = 0.into();
    assert!(decode(serde_json::to_string(&malformed).unwrap().as_bytes()).is_err());
    malformed = serde_json::from_str(&json).unwrap();
    malformed["source"]["region"] = 999.into();
    assert!(decode(serde_json::to_string(&malformed).unwrap().as_bytes()).is_err());
}
#[test]
fn malformed_files_and_invalid_accepted_scene_rejected_without_replacement() {
    let e = Editor::default();
    let original = e.document.clone();
    let json = save(&original).unwrap();
    let base: serde_json::Value = serde_json::from_str(&json).unwrap();
    for mutation in 0..9 {
        let mut value = base.clone();
        match mutation {
            0 => value["version"] = 15.into(),
            1 => value["domain"][0] = 0.into(),
            2 => value["draft"]["loops"][0]["intervals"][0] = 0.into(),
            3 => {
                value["draft"]["loops"][0]["controls"] = serde_json::json!([[0, 0], [0, 0], [0, 0]])
            }
            4 => value["accepted"]["loops"][0]["controls"][0][0] = 100.into(),
            5 => value["draft"]["loops"][0]["id"] = 0.into(),
            6 => {
                let o = value["draft"]["loops"][0].clone();
                value["draft"]["loops"].as_array_mut().unwrap().push(o);
            }
            7 => value["draft"]["loops"][0]["unknown"] = true.into(),
            _ => {
                value["draft"]["loops"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("span_conditions");
            }
        };
        assert!(
            decode(serde_json::to_string(&value).unwrap().as_bytes()).is_err(),
            "mutation {mutation}"
        );
        assert_eq!(e.document, original);
    }
    assert!(parse(&vec![b' '; MAX_FILE_BYTES + 1]).is_err());
    assert!(parse(b"NaN").is_err());
}

#[test]
fn open_internal_boundary_round_trip_and_history() {
    let mut editor = Editor::default();
    let boundary = editor
        .create_internal_boundary(
            OpenCubicSpline::uniform(vec![
                Point2::new(-0.7, 0.45),
                Point2::new(-0.3, 0.60),
                Point2::new(0.3, 0.42),
                Point2::new(0.7, 0.55),
            ])
            .unwrap(),
            BACKGROUND_REGION,
        )
        .unwrap();
    settle(&mut editor);
    assert!(
        matches!(editor.acceptance, Acceptance::Valid),
        "{:?}",
        editor.acceptance
    );
    let created = editor.document.clone();
    editor.undo();
    assert!(editor.document.draft.internal_boundaries.is_empty());
    editor.redo();
    assert_eq!(editor.document, created);

    let law = InternalBoundaryLaw {
        coupling: InternalBoundaryCoupling::ThinGap {
            stiffness_ratio: 0.5,
        },
        ..InternalBoundaryLaw::REFLECTING
    };
    editor.set_internal_boundary_law(boundary, 0, law).unwrap();
    settle(&mut editor);
    editor.undo();
    assert_eq!(
        editor.internal_boundary(boundary).unwrap().span_laws,
        [InternalBoundaryLaw::REFLECTING]
    );
    editor.redo();
    settle(&mut editor);
    assert_eq!(editor.internal_boundary(boundary).unwrap().span_laws, [law]);

    let json = save(&editor.document).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json).unwrap()["version"],
        14
    );
    let decoded = decode(json.as_bytes()).unwrap();
    assert_eq!(decoded, editor.document);
    assert_eq!(decoded.draft.internal_boundaries[0].id, boundary);
    assert_eq!(decoded.draft.internal_boundaries[0].span_laws, [law]);

    let mut parallel_gap: serde_json::Value = serde_json::from_str(&json).unwrap();
    parallel_gap["version"] = 6.into();
    parallel_gap.as_object_mut().unwrap().remove("far_field");
    for scene in ["draft", "accepted"] {
        parallel_gap[scene]["internal_boundaries"][0]["span_laws"][0]["left"] =
            serde_json::json!({ "kind": "impedance", "ratio": 1.25 });
    }
    let migrated_gap = decode(serde_json::to_string(&parallel_gap).unwrap().as_bytes()).unwrap();
    assert_eq!(migrated_gap.draft.internal_boundaries[0].span_laws, [law]);

    let mut legacy: serde_json::Value = serde_json::from_str(&json).unwrap();
    legacy["version"] = 3.into();
    legacy.as_object_mut().unwrap().remove("far_field");
    for scene in ["draft", "accepted"] {
        let stored = &mut legacy[scene]["internal_boundaries"][0];
        stored.as_object_mut().unwrap().remove("span_laws");
        stored["law"] = serde_json::json!({ "kind": "reflecting" });
    }
    let migrated = decode(serde_json::to_string(&legacy).unwrap().as_bytes()).unwrap();
    assert_eq!(
        migrated.draft.internal_boundaries[0].span_laws,
        [InternalBoundaryLaw::REFLECTING]
    );
}

#[test]
fn baffle_span_laws_follow_insertion_and_guard_ambiguous_removal() {
    let mut editor = Editor::default();
    let id = editor
        .create_internal_boundary(
            OpenCubicSpline::uniform(vec![
                Point2::new(-0.8, 0.55),
                Point2::new(-0.4, 0.65),
                Point2::new(0.0, 0.55),
                Point2::new(0.4, 0.65),
                Point2::new(0.8, 0.55),
            ])
            .unwrap(),
            BACKGROUND_REGION,
        )
        .unwrap();
    settle(&mut editor);
    let assigned = InternalBoundaryLaw {
        left: FaceBoundaryCondition::Impedance { ratio: 0.75 },
        ..InternalBoundaryLaw::REFLECTING
    };
    editor.set_internal_boundary_law(id, 0, assigned).unwrap();
    settle(&mut editor);
    let history_before_insert = editor.history_len().0;
    editor.insert_internal_boundary(id, 0.5).unwrap();
    settle(&mut editor);
    let boundary = editor.internal_boundary(id).unwrap();
    assert_eq!(
        boundary.span_laws,
        [assigned, assigned, InternalBoundaryLaw::REFLECTING]
    );
    assert_eq!(editor.history_len().0, history_before_insert + 1);

    let before_rejected_remove = editor.document.clone();
    let history_before_remove = editor.history_len();
    assert!(editor.remove_internal_boundary_point(id, 3).is_err());
    assert_eq!(editor.document, before_rejected_remove);
    assert_eq!(editor.history_len(), history_before_remove);

    editor.set_internal_boundary_law(id, 2, assigned).unwrap();
    editor.remove_internal_boundary_point(id, 3).unwrap();
    settle(&mut editor);
    assert_eq!(
        editor.internal_boundary(id).unwrap().span_laws,
        [assigned, assigned]
    );
}

#[test]
fn continuity_split_merge_and_orientation_are_atomic_and_round_trip() {
    let mut editor = Editor::default();
    let spline = OpenCubicSpline::new(
        vec![
            Point2::new(-0.8, 0.55),
            Point2::new(-0.55, 0.75),
            Point2::new(-0.15, 0.45),
            Point2::new(0.2, 0.7),
            Point2::new(0.55, 0.45),
            Point2::new(0.8, 0.6),
        ],
        vec![0.7, 1.2, 0.9],
    )
    .unwrap();
    let original = spline.clone();
    let id = editor
        .create_internal_boundary(spline, BACKGROUND_REGION)
        .unwrap();
    let law = InternalBoundaryLaw {
        left: FaceBoundaryCondition::Dirichlet {
            signal: BoundarySignal::ZERO,
        },
        right: FaceBoundaryCondition::SecondOrderOutgoing,
        coupling: InternalBoundaryCoupling::Independent,
    };
    editor.set_internal_boundary_law(id, 0, law).unwrap();
    let history = editor.history_len().0;
    editor.set_internal_boundary_continuity(id, 1, 0).unwrap();
    assert_eq!(editor.history_len().0, history + 1);
    assert_eq!(
        editor.internal_boundary(id).unwrap().spline.continuity(1),
        Some(0)
    );
    assert_eq!(editor.internal_boundary(id).unwrap().span_laws[0], law);
    for index in 0..=200 {
        let parameter = index as f64 * original.period() / 200.0;
        assert!(
            (editor
                .internal_boundary(id)
                .unwrap()
                .spline
                .evaluate(parameter)
                - original.evaluate(parameter))
            .norm()
                < 1.0e-10
        );
    }

    let history = editor.history_len().0;
    let second = editor.split_internal_boundary(id, 1).unwrap();
    assert_eq!(editor.history_len().0, history + 1);
    assert_eq!(editor.internal_boundary(id).unwrap().span_laws, [law]);
    assert_eq!(editor.internal_boundary(second).unwrap().span_laws.len(), 2);
    settle(&mut editor);
    assert_eq!(editor.acceptance, Acceptance::Valid);
    let mesh = mesh_scene(
        &editor.document.draft,
        editor.revision,
        MeshingOptions {
            target_edge_length: 0.25,
            minimum_angle_degrees: 10.0,
            ..Default::default()
        },
    )
    .expect("a split baffle junction must remain meshable");
    QuadraticWaveOperator::assemble_scene(
        &mesh,
        &editor.document.draft,
        OuterBoundaryCondition::Reflecting,
    )
    .expect("a split baffle junction must assemble into the wave operator");

    // Merge with the second ID first, forcing orientation reversal. Face laws
    // must still follow the displayed start-to-end arrows.
    let history = editor.history_len().0;
    let kept = editor
        .merge_internal_boundaries(second, id, 1.0e-12)
        .unwrap();
    assert_eq!(kept, second);
    assert_eq!(editor.history_len().0, history + 1);
    assert!(editor.internal_boundary(id).is_none());
    let merged = editor.internal_boundary(kept).unwrap();
    assert_eq!(merged.spline.intervals().len(), 3);
    assert_eq!(merged.spline.multiplicities(), &[1, 3]);
    let reversed_law = merged.span_laws[2];
    assert_eq!(reversed_law.left, law.right);
    assert_eq!(reversed_law.right, law.left);
    settle(&mut editor);
    assert_eq!(editor.acceptance, Acceptance::Valid);

    let json = save(&editor.document).unwrap();
    let decoded = decode(json.as_bytes()).unwrap();
    assert_eq!(decoded, editor.document);
    assert_eq!(
        decoded.draft.internal_boundaries[0].spline.multiplicities(),
        &[1, 3]
    );
    editor.undo();
    assert!(editor.internal_boundary(id).is_some());
    assert!(editor.internal_boundary(second).is_some());
}

#[test]
fn continuity_smoothing_is_exact_or_an_undoable_reshape() {
    let mut editor = Editor::default();
    let id = editor
        .create_internal_boundary(
            OpenCubicSpline::uniform(vec![
                Point2::new(-0.7, 0.55),
                Point2::new(-0.35, 0.7),
                Point2::new(0.0, 0.5),
                Point2::new(0.35, 0.68),
                Point2::new(0.7, 0.55),
            ])
            .unwrap(),
            BACKGROUND_REGION,
        )
        .unwrap();
    let original = editor.internal_boundary(id).unwrap().spline.clone();
    editor.set_internal_boundary_continuity(id, 1, 0).unwrap();
    let exact_displacement = editor.set_internal_boundary_continuity(id, 1, 2).unwrap();
    assert_eq!(exact_displacement, 0.0);
    let smoothed = &editor.internal_boundary(id).unwrap().spline;
    assert_eq!(smoothed.multiplicities(), original.multiplicities());
    for index in 0..=100 {
        let parameter = index as f64 * original.period() / 100.0;
        assert!((smoothed.evaluate(parameter) - original.evaluate(parameter)).norm() < 1.0e-10);
    }

    editor.set_internal_boundary_continuity(id, 1, 0).unwrap();
    let corner = editor
        .internal_boundary(id)
        .unwrap()
        .spline
        .span_control_indices(0)
        .unwrap()[3];
    editor.begin();
    let moved =
        editor.internal_boundary(id).unwrap().spline.controls()[corner] + Point2::new(0.02, -0.01);
    editor
        .set_internal_boundary_point(id, corner, moved)
        .unwrap();
    editor.commit();
    let before = editor.document.clone();
    let history = editor.history_len();
    let displacement = editor.set_internal_boundary_continuity(id, 1, 1).unwrap();
    assert!(displacement > 0.0);
    assert_eq!(
        editor.internal_boundary(id).unwrap().spline.continuity(1),
        Some(1)
    );
    assert_ne!(editor.document, before);
    assert_eq!(editor.history_len().0, history.0 + 1);
    editor.undo();
    assert_eq!(editor.document, before);
}

#[test]
fn loop_role_conversion_owns_regions_and_rejects_nonempty_holes() {
    let mut editor = Editor::default();
    let material = editor.add_material().unwrap();
    let id = ObstacleId(1);
    let history = editor.history_len().0;
    editor
        .set_loop_kind(id, LoopKind::MaterialInterface, material)
        .unwrap();
    let interior = editor.obstacle(id).unwrap().role.interior().unwrap();
    assert_eq!(
        editor.document.draft.region(interior).unwrap().material,
        material
    );
    assert_eq!(editor.history_len().0, history + 1);
    settle(&mut editor);
    assert_eq!(editor.acceptance, Acceptance::Valid);

    editor.set_loop_kind(id, LoopKind::Wall, material).unwrap();
    assert_eq!(editor.obstacle(id).unwrap().role.interior(), Some(interior));
    assert_eq!(editor.loop_kind(id), Some(LoopKind::Wall));
    editor.set_loop_kind(id, LoopKind::Hole, material).unwrap();
    assert_eq!(editor.loop_kind(id), Some(LoopKind::Hole));
    assert!(editor.document.draft.region(interior).is_none());
    settle(&mut editor);
    assert_eq!(editor.acceptance, Acceptance::Valid);
    editor.undo();
    assert_eq!(editor.loop_kind(id), Some(LoopKind::Wall));
    assert!(editor.document.draft.region(interior).is_some());

    let child = editor
        .create_loop(
            PeriodicCubicSpline::rounded(Point2::default(), 0.04),
            LoopRole::Hole { exterior: interior },
        )
        .unwrap();
    settle(&mut editor);
    assert_eq!(editor.acceptance, Acceptance::Valid);
    let before = editor.document.clone();
    let history = editor.history_len();
    assert!(editor.set_loop_kind(id, LoopKind::Hole, material).is_err());
    assert_eq!(editor.document, before);
    assert_eq!(editor.history_len(), history);
    assert_eq!(editor.obstacle(child).unwrap().role.exterior(), interior);

    let decoded = decode(save(&editor.document).unwrap().as_bytes()).unwrap();
    assert_eq!(decoded, editor.document);
}

#[test]
fn hole_span_conditions_round_trip_follow_seam_insertion_and_guard_removal() {
    let mut editor = Editor::default();
    let id = ObstacleId(1);
    let assigned = FaceBoundaryCondition::Dirichlet {
        signal: BoundarySignal {
            offset: 0.2,
            amplitude: 0.7,
            frequency_hz: 2.5,
            phase_radians: 0.3,
        },
    };
    editor
        .set_obstacle_boundary_condition(id, 7, assigned)
        .unwrap();
    settle(&mut editor);
    assert_eq!(
        editor.document.accepted.obstacles[0].span_conditions[7],
        assigned
    );

    let history_before_insert = editor.history_len().0;
    editor.insert(id, 7.5).unwrap();
    settle(&mut editor);
    assert_eq!(
        &editor.obstacle(id).unwrap().span_conditions[7..],
        &[assigned, assigned]
    );
    assert_eq!(editor.history_len().0, history_before_insert + 1);

    let before_rejected_remove = editor.document.clone();
    let history_before_remove = editor.history_len();
    assert!(editor.remove_point(id, 0).is_err());
    assert_eq!(editor.document, before_rejected_remove);
    assert_eq!(editor.history_len(), history_before_remove);

    editor
        .set_obstacle_boundary_condition(id, 0, assigned)
        .unwrap();
    editor.remove_point(id, 0).unwrap();
    settle(&mut editor);
    assert_eq!(editor.obstacle(id).unwrap().span_conditions.len(), 8);
    assert_eq!(editor.obstacle(id).unwrap().span_conditions[7], assigned);

    let json = save(&editor.document).unwrap();
    let decoded = decode(json.as_bytes()).unwrap();
    assert_eq!(decoded, editor.document);

    let mut legacy: serde_json::Value = serde_json::from_str(&json).unwrap();
    legacy["version"] = 4.into();
    legacy.as_object_mut().unwrap().remove("far_field");
    for scene in ["draft", "accepted"] {
        legacy[scene]["loops"][0]
            .as_object_mut()
            .unwrap()
            .remove("span_conditions");
    }
    let migrated = decode(serde_json::to_string(&legacy).unwrap().as_bytes()).unwrap();
    assert!(
        migrated.draft.obstacles[0]
            .span_conditions
            .iter()
            .all(|condition| *condition == FaceBoundaryCondition::Reflecting)
    );
}

#[test]
fn insertion_and_removal_are_individual_actions() {
    let mut e = Editor::default();
    let before = e.document.clone();
    e.insert(ObstacleId(1), 0.3).unwrap();
    settle(&mut e);
    assert_eq!(e.history_len().0, 1);
    let inserted = e.document.clone();
    e.insert(ObstacleId(1), 0.3).unwrap();
    assert_eq!(e.history_len().0, 1);
    e.remove_point(ObstacleId(1), 2).unwrap();
    settle(&mut e);
    assert_eq!(e.history_len().0, 2);
    e.undo();
    assert_eq!(e.document, inserted);
    e.undo();
    assert_eq!(e.document, before);
}

#[test]
fn representative_example_loads_and_extreme_finite_draft_remains_editable() {
    let document = decode(include_bytes!("../../../examples/eight-obstacles.json")).unwrap();
    assert_eq!(document.draft.obstacles.len(), 8);
    let mut value: serde_json::Value = serde_json::from_str(&save(&document).unwrap()).unwrap();
    value["draft"]["loops"][0]["controls"][0][0] = serde_json::json!(1e100);
    let loaded = decode(serde_json::to_string(&value).unwrap().as_bytes()).unwrap();
    assert_eq!(loaded.accepted, document.accepted);
    assert_eq!(loaded.draft.obstacles[0].spline.controls()[0].x, 1e100);
}

#[test]
fn material_regions_round_trip_and_version_one_migrates_to_holes() {
    let mut editor = Editor::default();
    let material = editor.add_material().unwrap();
    let id = editor
        .create_region_loop(
            PeriodicCubicSpline::rounded(Point2::new(0.5, 0.0), 0.15),
            BACKGROUND_REGION,
            material,
            false,
        )
        .unwrap();
    settle(&mut editor);
    let mut values = editor.document.draft.material(material).unwrap().clone();
    values.mass_density = ScalarField::constant(2.5);
    values.stiffness = ScalarField::constant(6.0);
    values.damping = ScalarField::constant(0.1);
    editor.update_material(values).unwrap();
    settle(&mut editor);
    let document = decode(save(&editor.document).unwrap().as_bytes()).unwrap();
    assert_eq!(document, editor.document);
    assert!(matches!(
        document
            .draft
            .obstacles
            .iter()
            .find(|loop_| loop_.id == id)
            .unwrap()
            .role,
        LoopRole::MaterialInterface { .. }
    ));

    let legacy = br#"{
      "version": 1,
      "domain": [-1.0, 1.0, -1.0, 1.0],
      "draft": [{"id": 9, "controls": [[-0.2,0.0],[0.0,0.2],[0.2,0.0],[0.0,-0.2]], "intervals": [1.0,1.0,1.0,1.0]}],
      "accepted": [{"id": 9, "controls": [[-0.2,0.0],[0.0,0.2],[0.2,0.0],[0.0,-0.2]], "intervals": [1.0,1.0,1.0,1.0]}]
    }"#;
    let migrated = decode(legacy).unwrap();
    assert_eq!(migrated.draft.materials, Scene::default().materials);
    assert!(matches!(
        migrated.draft.obstacles[0].role,
        LoopRole::Hole {
            exterior: BACKGROUND_REGION
        }
    ));
}

#[test]
fn spatial_materials_parameters_and_frames_round_trip() {
    let mut editor = Editor::default();
    let material_id = editor.add_material().unwrap();
    let loop_id = editor
        .create_region_loop(
            PeriodicCubicSpline::rounded(Point2::new(0.5, 0.0), 0.12),
            BACKGROUND_REGION,
            material_id,
            false,
        )
        .unwrap();
    settle(&mut editor);
    let region_id = editor.obstacle(loop_id).unwrap().role.interior().unwrap();
    let mut material = editor.document.draft.material(material_id).unwrap().clone();
    material.parameters = vec![MaterialParameter {
        name: "R".into(),
        value: 0.35,
    }];
    material.mass_density = ScalarField::formula("1 + r / R").unwrap();
    material.stiffness = ScalarField::formula("2 - clamp(0, 1, r / R)").unwrap();
    editor.update_material(material).unwrap();
    editor
        .set_region_frame(
            region_id,
            MaterialFrame {
                origin: Point2::new(0.48, -0.03),
                angle_radians: 0.4,
                attachment: MaterialFrameAttachment::FollowRegion,
            },
        )
        .unwrap();
    settle(&mut editor);

    let json = save(&editor.document).unwrap();
    assert!(json.contains("\"version\": 14"));
    assert_eq!(decode(json.as_bytes()).unwrap(), editor.document);

    let mut malformed: serde_json::Value = serde_json::from_str(&json).unwrap();
    malformed["accepted"]["materials"][1]["mass_density"]["source"] = "sqrt(".into();
    assert!(decode(serde_json::to_string(&malformed).unwrap().as_bytes()).is_err());
    malformed = serde_json::from_str(&json).unwrap();
    malformed["accepted"]["regions"][1]["frame"]["angle_radians"] = "sideways".into();
    assert!(decode(serde_json::to_string(&malformed).unwrap().as_bytes()).is_err());
}

#[test]
fn version_thirteen_materials_gain_world_unit_region_frames() {
    let mut editor = Editor::default();
    let material_id = editor.add_material().unwrap();
    let loop_id = editor
        .create_region_loop(
            PeriodicCubicSpline::rounded(Point2::new(0.46, -0.21), 0.11),
            BACKGROUND_REGION,
            material_id,
            false,
        )
        .unwrap();
    settle(&mut editor);
    let region_id = editor.obstacle(loop_id).unwrap().role.interior().unwrap();
    let json = save(&editor.document).unwrap();
    let mut legacy: serde_json::Value = serde_json::from_str(&json).unwrap();
    legacy["version"] = 13.into();
    for scene_name in ["draft", "accepted"] {
        let scene = legacy[scene_name].as_object_mut().unwrap();
        for material in scene["materials"].as_array_mut().unwrap() {
            for coefficient in ["mass_density", "stiffness", "damping"] {
                let value = material[coefficient]["value"].clone();
                material[coefficient] = value;
            }
            material.as_object_mut().unwrap().remove("parameters");
        }
        for region in scene["regions"].as_array_mut().unwrap() {
            region.as_object_mut().unwrap().remove("frame");
        }
    }
    let migrated = decode(serde_json::to_string(&legacy).unwrap().as_bytes()).unwrap();
    assert_eq!(
        migrated.draft.region(BACKGROUND_REGION).unwrap().frame,
        MaterialFrame::world()
    );
    let frame = migrated.draft.region(region_id).unwrap().frame;
    assert_eq!(frame.attachment, MaterialFrameAttachment::FollowRegion);
    assert!((frame.origin - Point2::new(0.46, -0.21)).norm() < 1.0e-3);
    assert_eq!(frame.angle_radians, 0.0);
}

#[test]
fn attached_material_frame_follows_a_whole_loop_similarity() {
    let mut editor = Editor::default();
    let material_id = editor.add_material().unwrap();
    let loop_id = editor
        .create_region_loop(
            PeriodicCubicSpline::rounded(Point2::new(0.45, 0.0), 0.1),
            BACKGROUND_REGION,
            material_id,
            false,
        )
        .unwrap();
    settle(&mut editor);
    let region_id = editor.obstacle(loop_id).unwrap().role.interior().unwrap();
    let original_frame = MaterialFrame {
        origin: Point2::new(0.48, 0.04),
        angle_radians: 0.2,
        attachment: MaterialFrameAttachment::FollowRegion,
    };
    editor.set_region_frame(region_id, original_frame).unwrap();
    let controls = editor.obstacle(loop_id).unwrap().spline.controls().to_vec();
    let center = controls.iter().copied().reduce(|a, b| a + b).unwrap() / controls.len() as f64;
    let angle = 0.35_f64;
    let scale = 1.4;
    let translation = Point2::new(-0.12, 0.18);
    let (sin, cos) = angle.sin_cos();
    let transform = |point: Point2| {
        let relative = point - center;
        center
            + translation
            + Point2::new(
                scale * (cos * relative.x - sin * relative.y),
                scale * (sin * relative.x + cos * relative.y),
            )
    };
    let updates = controls
        .iter()
        .enumerate()
        .map(|(index, point)| (GeometryControl::Loop(loop_id, index), transform(*point)))
        .collect::<Vec<_>>();
    let history = editor.history_len().0;
    editor.begin();
    editor.set_control_points(&updates).unwrap();
    editor.commit();
    let frame = editor.document.draft.region(region_id).unwrap().frame;
    assert!((frame.origin - transform(original_frame.origin)).norm() < 1.0e-12);
    assert!((frame.angle_radians - (original_frame.angle_radians + angle)).abs() < 1.0e-12);
    assert_eq!(editor.history_len().0, history + 1);

    let before_partial = frame;
    editor.begin();
    editor
        .set_control_points(&[(
            GeometryControl::Loop(loop_id, 0),
            updates[0].1 + Point2::new(0.01, 0.0),
        )])
        .unwrap();
    assert_eq!(
        editor.document.draft.region(region_id).unwrap().frame,
        before_partial
    );
    editor.cancel();
}

#[test]
fn material_edits_and_region_assignments_are_undoable() {
    let mut editor = Editor::default();
    let material = editor.add_material().unwrap();
    let after_add = editor.document.clone();
    editor
        .set_region_material(BACKGROUND_REGION, material)
        .unwrap();
    assert_eq!(
        editor
            .document
            .draft
            .region(BACKGROUND_REGION)
            .unwrap()
            .material,
        material
    );
    editor.undo();
    assert_eq!(editor.document, after_add);
    editor.redo();
    assert_eq!(
        editor
            .document
            .draft
            .region(BACKGROUND_REGION)
            .unwrap()
            .material,
        material
    );
    assert!(editor.delete_material(material).is_err());
}

#[test]
fn mixed_control_update_is_one_history_action() {
    let mut editor = Editor::default();
    let baffle = editor
        .create_internal_boundary(
            OpenCubicSpline::uniform(vec![
                Point2::new(-0.7, 0.5),
                Point2::new(-0.25, 0.6),
                Point2::new(0.25, 0.4),
                Point2::new(0.7, 0.5),
            ])
            .unwrap(),
            BACKGROUND_REGION,
        )
        .unwrap();
    settle(&mut editor);
    let before = editor.document.clone();
    let history = editor.history_len().0;
    editor.begin();
    editor
        .set_control_points(&[
            (
                GeometryControl::Loop(ObstacleId(1), 0),
                Point2::new(0.2, 0.05),
            ),
            (GeometryControl::Baffle(baffle, 1), Point2::new(-0.2, 0.55)),
        ])
        .unwrap();
    editor.commit();
    assert_eq!(editor.history_len().0, history + 1);
    assert_ne!(editor.document, before);
    editor.undo();
    assert_eq!(editor.document, before);
}

#[test]
fn duplication_preserves_assignments_and_straightens_baffles() {
    let mut editor = Editor::default();
    editor.document.draft.obstacles[0].span_conditions[2] =
        FaceBoundaryCondition::SecondOrderOutgoing;
    editor.document.accepted = editor.document.draft.clone();
    let duplicate = editor
        .duplicate_obstacle(ObstacleId(1), Point2::new(0.4, 0.0))
        .unwrap();
    let source = editor.obstacle(ObstacleId(1)).unwrap();
    let copy = editor.obstacle(duplicate).unwrap();
    assert_eq!(copy.span_conditions, source.span_conditions);
    assert_eq!(copy.spline.intervals(), source.spline.intervals());
    assert!(
        copy.spline
            .controls()
            .iter()
            .zip(source.spline.controls())
            .all(|(copy, source)| (*copy - *source - Point2::new(0.4, 0.0)).norm() < 1.0e-12)
    );

    let law = InternalBoundaryLaw {
        left: FaceBoundaryCondition::SecondOrderOutgoing,
        ..InternalBoundaryLaw::REFLECTING
    };
    let baffle = editor
        .create_internal_boundary(
            OpenCubicSpline::uniform(vec![
                Point2::new(-0.8, 0.65),
                Point2::new(-0.4, 0.8),
                Point2::new(0.0, 0.55),
                Point2::new(0.4, 0.75),
                Point2::new(0.8, 0.6),
            ])
            .unwrap(),
            BACKGROUND_REGION,
        )
        .unwrap();
    editor.set_internal_boundary_law(baffle, 0, law).unwrap();
    let duplicate = editor
        .duplicate_internal_boundary(baffle, Point2::new(0.0, -0.25))
        .unwrap();
    assert_eq!(
        editor.internal_boundary(duplicate).unwrap().span_laws[0],
        law
    );
    editor.straighten_internal_boundary(duplicate).unwrap();
    let spline = &editor.internal_boundary(duplicate).unwrap().spline;
    let start = spline.evaluate(0.0);
    let direction = spline.evaluate(spline.period()) - start;
    for index in 0..=20 {
        let point = spline.evaluate(spline.period() * index as f64 / 20.0);
        assert!((point - start).cross(direction).abs() < 1.0e-12);
    }

    let material = editor.add_material().unwrap();
    let interface = editor
        .create_region_loop(
            PeriodicCubicSpline::rounded(Point2::new(0.55, -0.55), 0.08),
            BACKGROUND_REGION,
            material,
            false,
        )
        .unwrap();
    let source_region = editor.obstacle(interface).unwrap().role.interior().unwrap();
    let copy = editor
        .duplicate_obstacle(interface, Point2::new(-0.25, 0.0))
        .unwrap();
    let copy_region = editor.obstacle(copy).unwrap().role.interior().unwrap();
    assert_ne!(copy_region, source_region);
    assert_eq!(
        editor.document.draft.region(copy_region).unwrap().material,
        material
    );
    editor.undo();
    assert!(editor.obstacle(copy).is_none());
    assert!(editor.document.draft.region(copy_region).is_none());
}

#[test]
fn bulk_face_assignment_is_atomic_and_converts_thin_gaps() {
    let mut editor = Editor::default();
    let baffle = editor
        .create_internal_boundary(
            OpenCubicSpline::uniform(vec![
                Point2::new(-0.8, 0.6),
                Point2::new(-0.4, 0.7),
                Point2::new(0.0, 0.55),
                Point2::new(0.4, 0.7),
                Point2::new(0.8, 0.6),
            ])
            .unwrap(),
            BACKGROUND_REGION,
        )
        .unwrap();
    editor
        .set_internal_boundary_couplings(
            &[(baffle, 0), (baffle, 1)],
            InternalBoundaryCoupling::ThinGap {
                stiffness_ratio: 2.0,
            },
        )
        .unwrap();
    let before = editor.document.clone();
    let history = editor.history_len().0;
    let condition = FaceBoundaryCondition::Dirichlet {
        signal: BoundarySignal {
            offset: 0.2,
            amplitude: 0.5,
            frequency_hz: 3.0,
            phase_radians: 0.1,
        },
    };
    editor
        .set_boundary_face_conditions(
            &[
                BoundaryFaceTarget::Outer(OuterSide::Top),
                BoundaryFaceTarget::Hole(ObstacleId(1), 2),
                BoundaryFaceTarget::Baffle(baffle, 0, InternalBoundarySide::Right),
                BoundaryFaceTarget::Baffle(baffle, 1, InternalBoundarySide::Right),
            ],
            condition,
        )
        .unwrap();
    assert_eq!(editor.history_len().0, history + 1);
    assert_eq!(
        editor.document.draft.outer_boundaries.get(OuterSide::Top),
        OuterBoundaryCondition::Dirichlet {
            signal: condition.signal().unwrap()
        }
    );
    assert_eq!(
        editor.obstacle(ObstacleId(1)).unwrap().span_conditions[2],
        condition
    );
    for law in &editor.internal_boundary(baffle).unwrap().span_laws {
        assert_eq!(law.left, FaceBoundaryCondition::Reflecting);
        assert_eq!(law.right, condition);
        assert_eq!(law.coupling, InternalBoundaryCoupling::Independent);
    }
    editor.undo();
    assert_eq!(editor.document, before);

    let before = editor.document.clone();
    let history = editor.history_len();
    assert!(
        editor
            .set_boundary_face_conditions(
                &[
                    BoundaryFaceTarget::Hole(ObstacleId(1), 0),
                    BoundaryFaceTarget::Baffle(
                        InternalBoundaryId(u64::MAX),
                        0,
                        InternalBoundarySide::Left,
                    ),
                ],
                FaceBoundaryCondition::SecondOrderOutgoing,
            )
            .is_err()
    );
    assert_eq!(editor.document, before);
    assert_eq!(editor.history_len(), history);
}

#[test]
fn heterogeneous_first_order_assignment_preserves_face_ratio() {
    let mut editor = Editor::default();
    editor
        .set_boundary_face_conditions(
            &[
                BoundaryFaceTarget::Outer(OuterSide::Left),
                BoundaryFaceTarget::Hole(ObstacleId(1), 0),
            ],
            FaceBoundaryCondition::Impedance { ratio: 2.5 },
        )
        .unwrap();
    assert_eq!(
        editor.document.draft.outer_boundaries.get(OuterSide::Left),
        OuterBoundaryCondition::FirstOrderOutgoing
    );
    assert_eq!(
        editor.obstacle(ObstacleId(1)).unwrap().span_conditions[0],
        FaceBoundaryCondition::Impedance { ratio: 2.5 }
    );
    assert_eq!(
        editor
            .boundary_face_condition(BoundaryFaceTarget::Outer(OuterSide::Left))
            .unwrap(),
        FaceBoundaryCondition::Impedance { ratio: 1.0 }
    );
}

#[test]
fn validated_example_replacement_is_one_undoable_action() {
    let mut editor = Editor::default();
    let before = editor.document.clone();
    let scene = Scene::default();
    let example = Document {
        draft: scene.clone(),
        accepted: scene,
        probes: vec![],
        source: SourceSettings::default(),
        far_field: Default::default(),
    };

    editor.replace_validated_with_history(example.clone());
    assert_eq!(editor.document, example);
    assert_eq!(editor.history_len(), (1, 0));
    editor.undo();
    assert_eq!(editor.document, before);
}

#[test]
fn point_probes_are_undoable_and_persist_without_affecting_geometry_acceptance() {
    let mut editor = Editor::default();
    let revision = editor.revision;
    let id = editor.create_point_probe(Point2::new(0.25, -0.4)).unwrap();
    assert_eq!(editor.revision, revision);
    assert_eq!(editor.acceptance, Acceptance::Valid);
    assert_eq!(editor.history_len(), (1, 0));

    let mut probe = editor.document.probes[0].clone();
    probe.name = "Receiver".into();
    probe.target = ProbeTarget::Point(Point2::new(-0.2, 0.3));
    editor.update_probe(probe.clone()).unwrap();
    assert_eq!(editor.history_len(), (2, 0));
    editor.undo();
    assert_eq!(editor.document.probes[0].name, format!("Probe {}", id.0));
    editor.redo();
    assert_eq!(editor.document.probes[0], probe);

    let json = save(&editor.document).unwrap();
    let decoded = decode(json.as_bytes()).unwrap();
    assert_eq!(decoded.probes, [probe]);
    assert_eq!(decoded.draft, editor.document.draft);
    assert_eq!(decoded.accepted, editor.document.accepted);

    editor.delete_probe(id).unwrap();
    assert!(editor.document.probes.is_empty());
    editor.undo();
    assert_eq!(editor.document.probes.len(), 1);
}

#[test]
fn malformed_or_duplicate_point_probes_are_rejected() {
    let mut editor = Editor::default();
    editor.create_point_probe(Point2::new(0.2, 0.3)).unwrap();
    let json = save(&editor.document).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["probes"][0]["id"] = 0.into();
    assert!(parse_document(serde_json::to_string(&value).unwrap().as_bytes()).is_err());

    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let duplicate = value["probes"][0].clone();
    value["probes"].as_array_mut().unwrap().push(duplicate);
    assert!(parse_document(serde_json::to_string(&value).unwrap().as_bytes()).is_err());
}

#[test]
fn malformed_or_oversized_line_probes_are_rejected() {
    let mut editor = Editor::default();
    editor
        .create_segment_probe(Point2::new(-0.5, 0.0), Point2::new(0.5, 0.0))
        .unwrap();
    let json = save(&editor.document).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["probes"][0]["target"]["end"] = value["probes"][0]["target"]["start"].clone();
    assert!(parse_document(serde_json::to_string(&value).unwrap().as_bytes()).is_err());

    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["probes"][0]["target"]["preset"] = "high".into();
    let template = value["probes"][0].clone();
    for id in 2..=5 {
        let mut probe = template.clone();
        probe["id"] = id.into();
        value["probes"].as_array_mut().unwrap().push(probe);
    }
    assert!(parse_document(serde_json::to_string(&value).unwrap().as_bytes()).is_err());
}

#[test]
fn boundary_probe_round_trips_and_tracks_periodic_insertion() {
    let mut editor = Editor::default();
    let id = editor
        .create_boundary_probe(BoundaryProbeTarget {
            feature: BoundaryProbeFeature::Loop(ObstacleId(1)),
            start_span: 7,
            span_count: 2,
            whole: false,
            side: BoundaryProbeSide::Domain,
            reversed: false,
            preset: ProbeSamplingPreset::Medium,
        })
        .unwrap();
    editor.insert(ObstacleId(1), 7.5).unwrap();
    let ProbeTarget::Boundary(target) = editor.document.probes[0].target else {
        panic!("expected boundary probe")
    };
    assert_eq!(target.spans(9), vec![7, 8, 0]);

    let json = save(&editor.document).unwrap();
    assert!(json.contains("\"version\": 14"));
    let decoded = decode(json.as_bytes()).unwrap();
    assert_eq!(decoded.probes, editor.document.probes);

    editor.delete_obstacle(ObstacleId(1));
    assert!(editor.document.probes.is_empty());
    editor.undo();
    assert_eq!(editor.document.probes[0].id, id);
}

#[test]
fn baffle_split_keeps_largest_attached_piece() {
    let mut editor = Editor::default();
    let spline = OpenCubicSpline::uniform(vec![
        Point2::new(-0.8, 0.0),
        Point2::new(-0.55, 0.0),
        Point2::new(-0.25, 0.0),
        Point2::new(0.1, 0.0),
        Point2::new(0.45, 0.0),
        Point2::new(0.8, 0.0),
    ])
    .unwrap();
    let baffle = editor
        .create_internal_boundary(spline, BACKGROUND_REGION)
        .unwrap();
    editor
        .create_boundary_probe(BoundaryProbeTarget {
            feature: BoundaryProbeFeature::Baffle(baffle),
            start_span: 1,
            span_count: 2,
            whole: false,
            side: BoundaryProbeSide::Right,
            reversed: false,
            preset: ProbeSamplingPreset::Low,
        })
        .unwrap();
    let right = editor.split_internal_boundary(baffle, 1).unwrap();
    let ProbeTarget::Boundary(target) = editor.document.probes[0].target else {
        panic!("expected boundary probe")
    };
    assert_eq!(target.feature, BoundaryProbeFeature::Baffle(right));
    assert_eq!(target.spans(2), vec![0, 1]);
    assert_eq!(target.side, BoundaryProbeSide::Right);
}

#[test]
fn baffle_merge_moves_attachment_into_retained_curve() {
    let mut editor = Editor::default();
    let make = |start: f64, end: f64| {
        OpenCubicSpline::uniform(
            (0..4)
                .map(|index| Point2::new(start + (end - start) * index as f64 / 3.0, 0.55))
                .collect(),
        )
        .unwrap()
    };
    let first = editor
        .create_internal_boundary(make(-0.8, 0.0), BACKGROUND_REGION)
        .unwrap();
    let second = editor
        .create_internal_boundary(make(0.0, 0.8), BACKGROUND_REGION)
        .unwrap();
    editor
        .create_boundary_probe(BoundaryProbeTarget {
            feature: BoundaryProbeFeature::Baffle(second),
            start_span: 0,
            span_count: 1,
            whole: true,
            side: BoundaryProbeSide::Left,
            reversed: false,
            preset: ProbeSamplingPreset::Low,
        })
        .unwrap();
    editor
        .merge_internal_boundaries(first, second, 1.0e-12)
        .unwrap();
    let ProbeTarget::Boundary(target) = editor.document.probes[0].target else {
        panic!("expected boundary probe")
    };
    assert_eq!(target.feature, BoundaryProbeFeature::Baffle(first));
    assert_eq!(target.start_span, 1);
    assert_eq!(target.span_count, 1);
    assert_eq!(target.side, BoundaryProbeSide::Left);
}

#[test]
fn malformed_boundary_probe_is_rejected_without_replacement() {
    let mut editor = Editor::default();
    editor
        .create_boundary_probe(BoundaryProbeTarget {
            feature: BoundaryProbeFeature::Outer,
            start_span: 0,
            span_count: 1,
            whole: false,
            side: BoundaryProbeSide::Domain,
            reversed: false,
            preset: ProbeSamplingPreset::Low,
        })
        .unwrap();
    let json = save(&editor.document).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["probes"][0]["target"]["start_span"] = 9.into();
    assert!(parse_document(serde_json::to_string(&value).unwrap().as_bytes()).is_err());
}

#[test]
fn material_frame_drag_is_one_history_entry_and_can_be_undone() {
    let mut editor = Editor::default();
    let original = editor.document.clone();
    editor.begin();
    for index in 1..=12 {
        editor
            .set_region_frame_during_edit(
                BACKGROUND_REGION,
                MaterialFrame {
                    origin: Point2::new(index as f64 * 0.01, -0.2),
                    angle_radians: index as f64 * 0.02,
                    attachment: MaterialFrameAttachment::World,
                },
            )
            .unwrap();
    }
    editor.commit();
    assert_eq!(editor.history_len(), (1, 0));
    editor.undo();
    assert_eq!(editor.document, original);
}
