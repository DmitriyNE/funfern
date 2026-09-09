use femfun_app::{editor::*, persistence::*};
use femfun_core::*;
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
fn malformed_files_and_invalid_accepted_scene_rejected_without_replacement() {
    let e = Editor::default();
    let original = e.document.clone();
    let json = save(&original).unwrap();
    let base: serde_json::Value = serde_json::from_str(&json).unwrap();
    for mutation in 0..9 {
        let mut value = base.clone();
        match mutation {
            0 => value["version"] = 6.into(),
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
        left: FaceBoundaryCondition::Impedance { ratio: 1.25 },
        right: FaceBoundaryCondition::Reflecting,
        coupling: InternalBoundaryCoupling::ThinGap {
            stiffness_ratio: 0.5,
        },
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
        5
    );
    let decoded = decode(json.as_bytes()).unwrap();
    assert_eq!(decoded, editor.document);
    assert_eq!(decoded.draft.internal_boundaries[0].id, boundary);
    assert_eq!(decoded.draft.internal_boundaries[0].span_laws, [law]);

    let mut legacy: serde_json::Value = serde_json::from_str(&json).unwrap();
    legacy["version"] = 3.into();
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
        right: FaceBoundaryCondition::Reflecting,
        coupling: InternalBoundaryCoupling::ThinGap {
            stiffness_ratio: 2.0,
        },
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
fn hole_span_conditions_round_trip_follow_seam_insertion_and_guard_removal() {
    let mut editor = Editor::default();
    let id = ObstacleId(1);
    let assigned = FaceBoundaryCondition::Impedance { ratio: 0.75 };
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
    values.mass_density = 2.5;
    values.stiffness = 6.0;
    values.damping = 0.1;
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
