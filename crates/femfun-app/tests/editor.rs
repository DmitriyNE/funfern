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
    for mutation in 0..8 {
        let mut value = base.clone();
        match mutation {
            0 => value["version"] = 2.into(),
            1 => value["domain"][0] = 0.into(),
            2 => value["draft"][0]["intervals"][0] = 0.into(),
            3 => value["draft"][0]["controls"] = serde_json::json!([[0, 0], [0, 0], [0, 0]]),
            4 => value["accepted"][0]["controls"][0][0] = 100.into(),
            5 => value["draft"][0]["id"] = 0.into(),
            6 => {
                let o = value["draft"][0].clone();
                value["draft"].as_array_mut().unwrap().push(o);
            }
            _ => value["draft"][0]["unknown"] = true.into(),
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
    value["draft"][0]["controls"][0][0] = serde_json::json!(1e100);
    let loaded = decode(serde_json::to_string(&value).unwrap().as_bytes()).unwrap();
    assert_eq!(loaded.accepted, document.accepted);
    assert_eq!(loaded.draft.obstacles[0].spline.controls()[0].x, 1e100);
}
