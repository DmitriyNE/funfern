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
fn set_file_version(value: &mut serde_json::Value, version: u32) {
    value["version"] = version.into();
    if version < 16 {
        value.as_object_mut().unwrap().remove("presentation");
    }
}

#[test]
fn domain_bounds_are_undoable_persisted_and_keep_invalid_drafts() {
    let mut editor = Editor::default();
    let original = editor.document.model.accepted.clone();
    let resized = DomainRect::new(-1.5, 2.0, -0.8, 1.2);
    editor.set_domain(resized);
    settle(&mut editor);
    assert_eq!(editor.document.model.accepted.domain, resized);
    assert_eq!(editor.history_len(), (1, 0));

    let json = save(&editor.document).unwrap();
    let decoded = decode(json.as_bytes()).unwrap();
    assert_eq!(decoded.model.draft.domain, resized);
    assert_eq!(decoded.model.accepted.domain, resized);

    editor.undo();
    assert_eq!(editor.document.model.draft.domain, original.domain);
    assert_eq!(editor.document.model.accepted.domain, original.domain);
    editor.redo();
    settle(&mut editor);
    assert_eq!(editor.document.model.accepted.domain, resized);

    editor.set_domain(DomainRect::new(0.2, 0.25, -0.8, 1.2));
    settle(&mut editor);
    assert!(matches!(
        editor.acceptance,
        Acceptance::Invalid(ValidationIssue::Domain)
    ));
    assert_eq!(editor.document.model.accepted.domain, resized);
    assert_ne!(editor.document.model.draft.domain, resized);
}

#[test]
fn version_eighteen_migrates_the_fixed_domain() {
    let document = Document::default();
    let mut value: serde_json::Value = serde_json::from_str(&save(&document).unwrap()).unwrap();
    set_file_version(&mut value, 18);
    value["draft"].as_object_mut().unwrap().remove("domain");
    value["accepted"].as_object_mut().unwrap().remove("domain");
    let decoded = decode(serde_json::to_string(&value).unwrap().as_bytes()).unwrap();
    assert_eq!(decoded.model.draft.domain, DomainRect::default());
    assert_eq!(decoded.model.accepted.domain, DomainRect::default());
}

fn downgrade_time_signals_to_v16(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                downgrade_time_signals_to_v16(value);
            }
        }
        serde_json::Value::Object(object) => {
            if object.get("kind").and_then(serde_json::Value::as_str) == Some("harmonic") {
                object.remove("kind");
            }
            for value in object.values_mut() {
                downgrade_time_signals_to_v16(value);
            }
        }
        _ => {}
    }
}

fn downgrade_document_to_v16(value: &mut serde_json::Value) {
    set_file_version(value, 16);
    let source = value["source"].as_object_mut().unwrap();
    let signal = source.remove("signal").unwrap();
    source.insert("amplitude".into(), signal["amplitude"].clone());
    source.insert("frequency_hz".into(), signal["frequency_hz"].clone());
    downgrade_time_signals_to_v16(value);
}
#[test]
fn invalid_draft_persists_and_recovers() {
    let mut e = Editor::default();
    let accepted = e.document.model.accepted.clone();
    move_point(&mut e, Point2::new(8.0, 0.0));
    assert!(matches!(e.acceptance, Acceptance::Invalid(_)));
    assert_eq!(e.document.model.accepted, accepted);
    assert_ne!(e.document.model.draft, accepted);
    for _ in 0..10 {
        e.validate_frame(1000);
    }
    assert_ne!(e.document.model.draft, accepted);
    move_point(&mut e, Point2::new(0.15, 0.0));
    assert_eq!(e.acceptance, Acceptance::Valid);
    assert_eq!(e.document.model.accepted, e.document.model.draft);
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
    let mut job = ValidationJob::new(e.document.model.draft.clone(), old);
    e.set_point(ObstacleId(1), 0, Point2::new(8.0, 0.0))
        .unwrap();
    let result = loop {
        if let Some(result) = job.advance(1000) {
            break result;
        }
    };
    let accepted = e.document.model.accepted.clone();
    e.apply_validation(result);
    assert_eq!(e.acceptance, Acceptance::Pending);
    assert_eq!(e.document.model.accepted, accepted);
    settle(&mut e);
    assert!(matches!(e.acceptance, Acceptance::Invalid(_)));
}

#[test]
fn outer_side_conditions_are_undoable_and_round_trip_with_time_signals() {
    let mut editor = Editor::default();
    let original = editor.document.clone();
    let signal = TimeSignal::Harmonic {
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
            .model
            .accepted
            .outer_boundaries
            .get(OuterSide::Bottom),
        OuterBoundaryCondition::Dirichlet { signal }
    );
    let json = save(&editor.document).unwrap();
    assert_eq!(decode(json.as_bytes()).unwrap(), editor.document);

    let mut version_five: serde_json::Value = serde_json::from_str(&json).unwrap();
    set_file_version(&mut version_five, 5);
    version_five.as_object_mut().unwrap().remove("far_field");
    for scene in ["draft", "accepted"] {
        version_five[scene]
            .as_object_mut()
            .unwrap()
            .remove("outer_boundaries");
    }
    let migrated = decode(serde_json::to_string(&version_five).unwrap().as_bytes()).unwrap();
    assert_eq!(
        migrated.model.accepted.outer_boundaries,
        OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting)
    );

    editor.undo();
    settle(&mut editor);
    assert_eq!(
        editor
            .document
            .model
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
fn presentation_round_trips_and_older_scenes_receive_defaults() {
    let document = Document {
        presentation: PresentationSettings {
            grid: false,
            control_polygons: false,
            handles: false,
            accepted_reference: false,
            boundary_conditions: true,
            mesh: true,
            mesh_boundaries: false,
            adaptation_target: true,
            point_probes: false,
            line_probes: false,
            boundary_probes: false,
            area_probes: false,
            far_field_contour: false,
            probe_labels: false,
            field: false,
            field_gain: 7.5,
            vector_overlay: VectorOverlay::RelativeEnergyFlow,
            vector_overlay_smoothed: false,
            vector_overlay_density: 72.0,
            vector_overlay_gain: 1.7,
            material_overlay: MaterialOverlay::Property(MaterialProperty::Impedance),
            material_overlay_opacity: 0.73,
            material_overlay_auto_range: false,
            material_overlay_logarithmic: true,
            material_overlay_manual_min: 0.2,
            material_overlay_manual_max: 4.8,
        },
        ..Default::default()
    };

    let json = save(&document).unwrap();
    assert_eq!(decode(json.as_bytes()).unwrap(), document);

    let mut vector_document = document.clone();
    vector_document.presentation.vector_overlay = VectorOverlay::ComplementaryField;
    let current = save(&vector_document).unwrap();
    assert!(current.contains("\"complementary_field_rate\""));
    let alternate_name = current.replace("\"complementary_field_rate\"", "\"complementary_field\"");
    assert_eq!(decode(alternate_name.as_bytes()).unwrap(), vector_document);

    let mut legacy: serde_json::Value = serde_json::from_str(&json).unwrap();
    set_file_version(&mut legacy, 15);
    let decoded = decode(serde_json::to_string(&legacy).unwrap().as_bytes()).unwrap();
    assert_eq!(decoded.model, document.model);
    assert_eq!(decoded.presentation, PresentationSettings::default());

    let mut malformed: serde_json::Value = serde_json::from_str(&json).unwrap();
    malformed["presentation"]["field_gain"] = 0.into();
    assert!(decode(serde_json::to_string(&malformed).unwrap().as_bytes()).is_err());
}

#[test]
fn physics_is_undoable_and_version_seventeen_migrates_exactly_to_mechanical() {
    let mut editor = Editor::default();
    let original = editor.document.model.clone();
    editor
        .set_physics(PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        })
        .unwrap();
    settle(&mut editor);
    assert_eq!(editor.history_len(), (1, 0));
    assert!(matches!(
        editor.document.model.accepted.physics,
        PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm
        }
    ));
    let json = save(&editor.document).unwrap();
    assert!(json.contains("\"permittivity\""));
    assert!(!json.contains("\"mass_density\""));
    assert_eq!(decode(json.as_bytes()).unwrap(), editor.document);
    editor.undo();
    settle(&mut editor);
    assert_eq!(editor.document.model, original);

    let legacy = Document::default();
    let mut value: serde_json::Value = serde_json::from_str(&save(&legacy).unwrap()).unwrap();
    value["version"] = 17.into();
    for scene_name in ["draft", "accepted"] {
        let scene = value[scene_name].as_object_mut().unwrap();
        scene.remove("physics");
        for material in scene["materials"].as_array_mut().unwrap() {
            let material = material.as_object_mut().unwrap();
            let law = material.remove("law").unwrap();
            let law = law.as_object().unwrap();
            material.insert("mass_density".into(), law["density"].clone());
            material.insert("stiffness".into(), law["stiffness"].clone());
            material.insert("damping".into(), law["damping"].clone());
        }
    }
    let migrated = decode(serde_json::to_string(&value).unwrap().as_bytes()).unwrap();
    assert_eq!(migrated, legacy);
}

#[test]
fn physics_switch_converts_spatial_materials_and_undo_restores_the_exact_law() {
    let mut editor = Editor::default();
    for scene in [
        &mut editor.document.model.draft,
        &mut editor.document.model.accepted,
    ] {
        let material = scene.materials.first_mut().unwrap();
        material.mass_density = ScalarField::formula("2 + 0.2*x*x").unwrap();
        material.stiffness = ScalarField::formula("5 - 0.3*y").unwrap();
        material.damping = ScalarField::formula("0.1 + 0.02*r").unwrap();
    }
    let original = editor.document.clone();
    let mechanical = PhysicsModel::Mechanical;
    let tm = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };

    editor.set_physics(tm).unwrap();
    settle(&mut editor);
    assert_eq!(editor.history_len(), (1, 0));
    assert_eq!(editor.document.model.draft, editor.document.model.accepted);
    for point in [Point2::new(-0.5, 0.25), Point2::new(0.4, -0.7)] {
        let frame = MaterialFrame::world();
        let before = original.model.draft.materials[0]
            .evaluate(frame, point)
            .unwrap();
        let after = editor.document.model.draft.materials[0]
            .evaluate(frame, point)
            .unwrap();
        let before = WaveCoefficients {
            mass_density: before.mass_density,
            stiffness: before.stiffness,
            damping: before.damping,
        };
        let after = WaveCoefficients {
            mass_density: after.mass_density,
            stiffness: after.stiffness,
            damping: after.damping,
        };
        assert!((mechanical.wave_speed(before) - tm.wave_speed(after)).abs() < 1.0e-14);
        assert!((mechanical.impedance(before) - tm.impedance(after)).abs() < 1.0e-14);
    }

    let converted = editor.document.clone();
    let json = save(&converted).unwrap();
    assert_eq!(decode(json.as_bytes()).unwrap(), converted);
    editor.undo();
    assert_eq!(editor.document, original);
    editor.redo();
    settle(&mut editor);
    assert_eq!(editor.document, converted);
}

#[test]
fn failed_physics_formula_conversion_is_atomic() {
    let mut editor = Editor::default();
    let complex_formula =
        ScalarField::formula(std::iter::repeat_n("x", 64).collect::<Vec<_>>().join("+")).unwrap();
    for scene in [
        &mut editor.document.model.draft,
        &mut editor.document.model.accepted,
    ] {
        scene.materials[0].stiffness = complex_formula.clone();
    }
    let original = editor.document.clone();

    let error = editor
        .set_physics(PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        })
        .unwrap_err();
    assert!(error.contains("formula exceeds 256 bytes"), "{error}");
    assert_eq!(editor.document, original);
    assert_eq!(editor.history_len(), (0, 0));
}

#[test]
fn presentation_is_outside_model_history() {
    let mut editor = Editor::default();
    move_point(&mut editor, Point2::new(0.17, 0.01));
    let changed_model = editor.document.model.clone();
    editor.document.presentation.grid = false;
    editor.document.presentation.field_gain = 6.0;
    let presentation = editor.document.presentation;
    let revision = editor.revision;

    editor.undo();
    assert_ne!(editor.document.model, changed_model);
    assert_eq!(editor.document.presentation, presentation);
    assert!(editor.revision > revision);

    editor.redo();
    assert_eq!(editor.document.model, changed_model);
    assert_eq!(editor.document.presentation, presentation);
}

#[test]
fn line_probes_are_undoable_bounded_and_round_trip() {
    let mut editor = Editor::default();
    let id = editor
        .create_segment_probe(Point2::new(-0.5, 0.2), Point2::new(0.5, 0.2))
        .unwrap();
    assert_eq!(editor.history_len(), (1, 0));
    let mut probe = editor.document.model.probes[0].clone();
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
    assert_eq!(editor.document.model.probes[0].id, id);
    editor.undo();
    assert!(editor.document.model.probes.is_empty());

    let mut invalid = document.clone();
    invalid.model.probes[0].target = ProbeTarget::Segment {
        start: Point2::default(),
        end: Point2::default(),
        preset: ProbeSamplingPreset::Medium,
    };
    assert!(
        Editor::default()
            .update_probe(invalid.model.probes[0].clone())
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
    assert!(json.contains("\"version\": 21"));
    let decoded = decode(json.as_bytes()).unwrap();
    assert_eq!(decoded, editor.document);
    assert_eq!(decoded.model.far_field.inset, 0.17);
    assert!(matches!(
        decoded
            .model
            .probes
            .iter()
            .find(|probe| probe.id == disk)
            .unwrap()
            .target,
        ProbeTarget::AreaDisk { radius, .. } if radius == 0.18
    ));

    editor.delete_obstacle(region);
    assert!(
        editor
            .document
            .model
            .probes
            .iter()
            .any(|probe| probe.id == disk)
    );
    assert!(
        !editor
            .document
            .model
            .probes
            .iter()
            .any(|probe| probe.id == attached)
    );
    editor.undo();
    assert!(
        editor
            .document
            .model
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
    set_file_version(&mut legacy, 12);
    legacy.as_object_mut().unwrap().remove("far_field");
    assert!(decode(serde_json::to_string(&legacy).unwrap().as_bytes()).is_err());
}

#[test]
fn version_nine_point_probes_remain_loadable() {
    let mut editor = Editor::default();
    editor.create_point_probe(Point2::new(0.1, -0.2)).unwrap();
    let json = save(&editor.document).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    set_file_version(&mut value, 9);
    value.as_object_mut().unwrap().remove("source");
    value.as_object_mut().unwrap().remove("far_field");
    assert_eq!(
        decode(serde_json::to_string(&value).unwrap().as_bytes()).unwrap(),
        editor.document
    );
}

#[test]
fn point_source_round_trips_and_version_ten_uses_the_default() {
    let mut document = Document::default();
    document.model.source = PointSource {
        enabled: true,
        position: Point2::new(-0.37, 0.28),
        width: 0.045,
        region: BACKGROUND_REGION,
        signal: TimeSignal::harmonic(0.0, 23.0, 3.25, 0.0),
    };
    let json = save(&document).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["version"], 21);
    assert_eq!(decode(json.as_bytes()).unwrap(), document);

    set_file_version(&mut value, 10);
    value.as_object_mut().unwrap().remove("source");
    value.as_object_mut().unwrap().remove("far_field");
    let legacy = decode(serde_json::to_string(&value).unwrap().as_bytes()).unwrap();
    assert_eq!(legacy.model.source, PointSource::default());

    let mut malformed: serde_json::Value = serde_json::from_str(&json).unwrap();
    malformed["source"]["width"] = 0.into();
    assert!(decode(serde_json::to_string(&malformed).unwrap().as_bytes()).is_err());
    malformed = serde_json::from_str(&json).unwrap();
    malformed["source"]["region"] = 999.into();
    assert!(decode(serde_json::to_string(&malformed).unwrap().as_bytes()).is_err());
}

#[test]
fn version_sixteen_migrates_point_volume_and_boundary_signals() {
    let point_signal = TimeSignal::harmonic(0.0, 24.0, 3.5, 0.0);
    let volume_signal = TimeSignal::harmonic(0.125, 4.5, 2.25, -0.375);
    let boundary_signal = TimeSignal::harmonic(-0.25, 1.75, 1.5, 0.625);
    let mut document = Document::default();
    document.model.source = PointSource {
        enabled: true,
        position: Point2::new(-0.25, 0.5),
        width: 0.0625,
        region: BACKGROUND_REGION,
        signal: point_signal,
    };
    for scene in [&mut document.model.draft, &mut document.model.accepted] {
        scene.volume_sources.push(VolumeSource {
            region: BACKGROUND_REGION,
            enabled: true,
            profile: ScalarField::constant(0.75),
            parameters: Vec::new(),
            signal: volume_signal,
        });
        scene.outer_boundaries.sides[OuterSide::Left.index()] = OuterBoundaryCondition::Neumann {
            signal: boundary_signal,
        };
    }

    let json = save(&document).unwrap();
    let current: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(current["source"]["signal"]["kind"], "harmonic");
    assert_eq!(
        current["accepted"]["volume_sources"][0]["signal"]["kind"],
        "harmonic"
    );

    let mut legacy = current;
    downgrade_document_to_v16(&mut legacy);
    let migrated = decode(serde_json::to_string(&legacy).unwrap().as_bytes()).unwrap();
    assert_eq!(migrated, document);
    assert_eq!(migrated.model.source.signal, point_signal);
    assert_eq!(
        migrated.model.accepted.volume_sources[0].signal,
        volume_signal
    );
    assert_eq!(
        migrated
            .model
            .accepted
            .outer_boundaries
            .get(OuterSide::Left)
            .signal(),
        Some(boundary_signal)
    );

    let mut malformed: serde_json::Value = serde_json::from_str(&json).unwrap();
    malformed["source"]["signal"]["kind"] = "chirp".into();
    assert!(decode(serde_json::to_string(&malformed).unwrap().as_bytes()).is_err());
}

#[test]
fn volume_source_round_trips_and_is_one_undoable_region_edit() {
    let mut editor = Editor::default();
    let source = VolumeSource {
        region: BACKGROUND_REGION,
        enabled: true,
        profile: ScalarField::formula("gain * cos(theta)").unwrap(),
        parameters: vec![MaterialParameter {
            name: "gain".into(),
            value: 0.75,
        }],
        signal: TimeSignal::Harmonic {
            offset: 0.1,
            amplitude: 4.0,
            frequency_hz: 2.5,
            phase_radians: 0.3,
        },
    };
    editor
        .set_volume_source(BACKGROUND_REGION, Some(source.clone()))
        .unwrap();
    settle(&mut editor);
    assert_eq!(
        editor
            .document
            .model
            .accepted
            .volume_source(BACKGROUND_REGION),
        Some(&source)
    );
    editor.undo();
    assert!(editor.document.model.draft.volume_sources.is_empty());
    editor.redo();
    settle(&mut editor);

    let json = save(&editor.document).unwrap();
    assert!(json.contains("\"version\": 21"));
    assert_eq!(decode(json.as_bytes()).unwrap(), editor.document);

    let mut legacy: serde_json::Value = serde_json::from_str(&json).unwrap();
    set_file_version(&mut legacy, 14);
    assert!(decode(serde_json::to_string(&legacy).unwrap().as_bytes()).is_err());
}

#[test]
fn temporal_volume_source_edits_reuse_the_spatial_carrier() {
    let mut before = Scene::default();
    before.volume_sources.push(VolumeSource {
        region: BACKGROUND_REGION,
        enabled: true,
        profile: ScalarField::formula("1 - 0.25 * r").unwrap(),
        parameters: Vec::new(),
        signal: TimeSignal::harmonic(0.0, 2.0, 3.0, 0.0),
    });
    let mut after = before.clone();
    after.volume_sources[0].signal = TimeSignal::harmonic(0.5, 4.0, 6.0, 0.25);
    assert!(!before.volume_sources_eq(&after));
    assert!(before.volume_source_carriers_eq(&after));

    after.volume_sources[0].profile = ScalarField::constant(1.0);
    assert!(!before.volume_source_carriers_eq(&after));
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
            0 => value["version"] = 22.into(),
            1 => value["accepted"]["domain"][0] = 1.into(),
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
    assert!(editor.document.model.draft.internal_boundaries.is_empty());
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
        21
    );
    let decoded = decode(json.as_bytes()).unwrap();
    assert_eq!(decoded, editor.document);
    assert_eq!(decoded.model.draft.internal_boundaries[0].id, boundary);
    assert_eq!(decoded.model.draft.internal_boundaries[0].span_laws, [law]);

    let mut parallel_gap: serde_json::Value = serde_json::from_str(&json).unwrap();
    set_file_version(&mut parallel_gap, 6);
    parallel_gap.as_object_mut().unwrap().remove("far_field");
    for scene in ["draft", "accepted"] {
        parallel_gap[scene]["internal_boundaries"][0]["span_laws"][0]["left"] =
            serde_json::json!({ "kind": "impedance", "ratio": 1.25 });
    }
    let migrated_gap = decode(serde_json::to_string(&parallel_gap).unwrap().as_bytes()).unwrap();
    assert_eq!(
        migrated_gap.model.draft.internal_boundaries[0].span_laws,
        [law]
    );

    let mut legacy: serde_json::Value = serde_json::from_str(&json).unwrap();
    set_file_version(&mut legacy, 3);
    legacy.as_object_mut().unwrap().remove("far_field");
    for scene in ["draft", "accepted"] {
        let stored = &mut legacy[scene]["internal_boundaries"][0];
        stored.as_object_mut().unwrap().remove("span_laws");
        stored["law"] = serde_json::json!({ "kind": "reflecting" });
    }
    let migrated = decode(serde_json::to_string(&legacy).unwrap().as_bytes()).unwrap();
    assert_eq!(
        migrated.model.draft.internal_boundaries[0].span_laws,
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
            signal: TimeSignal::ZERO,
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
        &editor.document.model.draft,
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
        &editor.document.model.draft,
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
        decoded.model.draft.internal_boundaries[0]
            .spline
            .multiplicities(),
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
        editor
            .document
            .model
            .draft
            .region(interior)
            .unwrap()
            .material,
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
    assert!(editor.document.model.draft.region(interior).is_none());
    settle(&mut editor);
    assert_eq!(editor.acceptance, Acceptance::Valid);
    editor.undo();
    assert_eq!(editor.loop_kind(id), Some(LoopKind::Wall));
    assert!(editor.document.model.draft.region(interior).is_some());

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
        signal: TimeSignal::Harmonic {
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
        editor.document.model.accepted.obstacles[0].span_conditions[7],
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
    set_file_version(&mut legacy, 4);
    legacy.as_object_mut().unwrap().remove("far_field");
    for scene in ["draft", "accepted"] {
        legacy[scene]["loops"][0]
            .as_object_mut()
            .unwrap()
            .remove("span_conditions");
    }
    let migrated = decode(serde_json::to_string(&legacy).unwrap().as_bytes()).unwrap();
    assert!(
        migrated.model.draft.obstacles[0]
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

/// A version-one document still migrates. The fixture lives here rather than in
/// `examples/`, which ships a current-version scene for readers to open.
#[test]
fn representative_example_loads_and_extreme_finite_draft_remains_editable() {
    let document = decode(include_bytes!("fixtures/eight-obstacles-v1.json")).unwrap();
    assert_eq!(document.model.draft.obstacles.len(), 8);
    let mut value: serde_json::Value = serde_json::from_str(&save(&document).unwrap()).unwrap();
    value["draft"]["loops"][0]["controls"][0][0] = serde_json::json!(1e100);
    let loaded = decode(serde_json::to_string(&value).unwrap().as_bytes()).unwrap();
    assert_eq!(loaded.model.accepted, document.model.accepted);
    assert_eq!(
        loaded.model.draft.obstacles[0].spline.controls()[0].x,
        1e100
    );
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
    let mut values = editor
        .document
        .model
        .draft
        .material(material)
        .unwrap()
        .clone();
    values.mass_density = ScalarField::constant(2.5);
    values.stiffness = ScalarField::constant(6.0);
    values.damping = ScalarField::constant(0.1);
    editor.update_material(values).unwrap();
    settle(&mut editor);
    let document = decode(save(&editor.document).unwrap().as_bytes()).unwrap();
    assert_eq!(document, editor.document);
    assert!(matches!(
        document
            .model
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
    assert_eq!(migrated.model.draft.materials, Scene::default().materials);
    assert!(matches!(
        migrated.model.draft.obstacles[0].role,
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
    let mut material = editor
        .document
        .model
        .draft
        .material(material_id)
        .unwrap()
        .clone();
    material.parameters = vec![MaterialParameter {
        name: "R".into(),
        value: 0.35,
    }];
    material.mass_density = ScalarField::formula("1 + r / R").unwrap();
    material.stiffness = ScalarField::formula("2 - clamp(0, 1, r / R)").unwrap();
    material.axis_ratio = ScalarField::formula("1 + 2 * clamp(0, 1, r / R)").unwrap();
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
    editor.document.presentation.material_overlay =
        MaterialOverlay::Property(MaterialProperty::Anisotropy);
    editor.document.presentation.material_overlay_logarithmic = true;

    let json = save(&editor.document).unwrap();
    assert!(json.contains("\"version\": 21"));
    assert_eq!(decode(json.as_bytes()).unwrap(), editor.document);

    let mut malformed: serde_json::Value = serde_json::from_str(&json).unwrap();
    malformed["accepted"]["materials"][1]["mass_density"]["source"] = "sqrt(".into();
    assert!(decode(serde_json::to_string(&malformed).unwrap().as_bytes()).is_err());
    malformed = serde_json::from_str(&json).unwrap();
    malformed["accepted"]["regions"][1]["frame"]["angle_radians"] = "sideways".into();
    assert!(decode(serde_json::to_string(&malformed).unwrap().as_bytes()).is_err());

    let mut legacy: serde_json::Value = serde_json::from_str(&json).unwrap();
    legacy["version"] = 19.into();
    for scene in ["draft", "accepted"] {
        for material in legacy[scene]["materials"].as_array_mut().unwrap() {
            material.as_object_mut().unwrap().remove("axis_ratio");
        }
    }
    let migrated = decode(serde_json::to_string(&legacy).unwrap().as_bytes()).unwrap();
    assert!(
        migrated
            .model
            .draft
            .materials
            .iter()
            .all(|material| { material.axis_ratio == ScalarField::constant(1.0) })
    );
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
    set_file_version(&mut legacy, 13);
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
        migrated
            .model
            .draft
            .region(BACKGROUND_REGION)
            .unwrap()
            .frame,
        MaterialFrame::world()
    );
    let frame = migrated.model.draft.region(region_id).unwrap().frame;
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
    let frame = editor.document.model.draft.region(region_id).unwrap().frame;
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
        editor.document.model.draft.region(region_id).unwrap().frame,
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
            .model
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
            .model
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
    editor.document.model.draft.obstacles[0].span_conditions[2] =
        FaceBoundaryCondition::SecondOrderOutgoing;
    editor.document.model.accepted = editor.document.model.draft.clone();
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
        editor
            .document
            .model
            .draft
            .region(copy_region)
            .unwrap()
            .material,
        material
    );
    editor.undo();
    assert!(editor.obstacle(copy).is_none());
    assert!(editor.document.model.draft.region(copy_region).is_none());
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
        signal: TimeSignal::Harmonic {
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
        editor
            .document
            .model
            .draft
            .outer_boundaries
            .get(OuterSide::Top),
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
        editor
            .document
            .model
            .draft
            .outer_boundaries
            .get(OuterSide::Left),
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
    let mut example = Document {
        model: DocumentModel {
            draft: scene.clone(),
            accepted: scene,
            probes: vec![],
            source: PointSource::default(),
            far_field: Default::default(),
        },
        presentation: Default::default(),
    };
    example.presentation.mesh = true;
    example.presentation.field_gain = 5.0;

    editor.replace_validated_with_history(example.clone());
    assert_eq!(editor.document, example);
    assert_eq!(editor.history_len(), (1, 0));
    editor.undo();
    assert_eq!(editor.document.model, before.model);
    assert_eq!(editor.document.presentation, example.presentation);
}

#[test]
fn point_probes_are_undoable_and_persist_without_affecting_geometry_acceptance() {
    let mut editor = Editor::default();
    let revision = editor.revision;
    let id = editor.create_point_probe(Point2::new(0.25, -0.4)).unwrap();
    assert_eq!(editor.revision, revision);
    assert_eq!(editor.acceptance, Acceptance::Valid);
    assert_eq!(editor.history_len(), (1, 0));

    let mut probe = editor.document.model.probes[0].clone();
    probe.name = "Receiver".into();
    probe.target = ProbeTarget::Point(Point2::new(-0.2, 0.3));
    editor.update_probe(probe.clone()).unwrap();
    assert_eq!(editor.history_len(), (2, 0));
    editor.undo();
    assert_eq!(
        editor.document.model.probes[0].name,
        format!("Probe {}", id.0)
    );
    editor.redo();
    assert_eq!(editor.document.model.probes[0], probe);

    let json = save(&editor.document).unwrap();
    let decoded = decode(json.as_bytes()).unwrap();
    assert_eq!(decoded.model.probes, [probe]);
    assert_eq!(decoded.model.draft, editor.document.model.draft);
    assert_eq!(decoded.model.accepted, editor.document.model.accepted);

    editor.delete_probe(id).unwrap();
    assert!(editor.document.model.probes.is_empty());
    editor.undo();
    assert_eq!(editor.document.model.probes.len(), 1);
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
    let ProbeTarget::Boundary(target) = editor.document.model.probes[0].target else {
        panic!("expected boundary probe")
    };
    assert_eq!(target.spans(9), vec![7, 8, 0]);

    let json = save(&editor.document).unwrap();
    assert!(json.contains("\"version\": 21"));
    let decoded = decode(json.as_bytes()).unwrap();
    assert_eq!(decoded.model.probes, editor.document.model.probes);

    editor.delete_obstacle(ObstacleId(1));
    assert!(editor.document.model.probes.is_empty());
    editor.undo();
    assert_eq!(editor.document.model.probes[0].id, id);
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
    let ProbeTarget::Boundary(target) = editor.document.model.probes[0].target else {
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
    let ProbeTarget::Boundary(target) = editor.document.model.probes[0].target else {
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

#[test]
fn outer_to_outer_divider_is_one_undoable_region_split() {
    let mut editor = Editor::default();
    let before = editor.document.model.clone();
    let divider = editor
        .create_material_divider(
            OpenCubicSpline::polyline(vec![Point2::new(-0.75, -1.0), Point2::new(-0.75, 1.0)])
                .unwrap(),
            DividerEndpoint::Outer {
                side: OuterSide::Bottom,
                fraction: 0.125,
            },
            DividerEndpoint::Outer {
                side: OuterSide::Top,
                fraction: 0.875,
            },
            vec![BACKGROUND_REGION],
            DEFAULT_MATERIAL,
        )
        .unwrap();
    assert_eq!(editor.history_len(), (1, 0));
    assert_eq!(editor.document.model.draft.material_interfaces.len(), 1);
    assert_eq!(editor.document.model.draft.regions.len(), 2);
    editor.undo();
    assert_eq!(editor.document.model, before);
    editor.redo();
    editor.delete_material_divider(divider, 0).unwrap();
    assert!(editor.document.model.draft.material_interfaces.is_empty());
    assert_eq!(editor.document.model.draft.regions.len(), 1);
}

#[test]
fn divider_can_branch_from_a_c0_node_into_a_t_junction() {
    let mut editor = Editor::default();
    editor.document.model.draft.obstacles.clear();
    editor.document.model.accepted.obstacles.clear();
    let first = editor
        .create_material_divider(
            OpenCubicSpline::polyline(vec![
                Point2::new(0.0, -1.0),
                Point2::default(),
                Point2::new(0.0, 1.0),
            ])
            .unwrap(),
            DividerEndpoint::Outer {
                side: OuterSide::Bottom,
                fraction: 0.5,
            },
            DividerEndpoint::Outer {
                side: OuterSide::Top,
                fraction: 0.5,
            },
            vec![BACKGROUND_REGION, BACKGROUND_REGION],
            DEFAULT_MATERIAL,
        )
        .unwrap();
    let node = editor.document.model.draft.material_interfaces[0].nodes[1].id;
    let branch = editor
        .create_material_divider(
            OpenCubicSpline::polyline(vec![Point2::default(), Point2::new(1.0, 0.0)]).unwrap(),
            DividerEndpoint::InterfaceNode {
                interface: first,
                node,
            },
            DividerEndpoint::Outer {
                side: OuterSide::Right,
                fraction: 0.5,
            },
            vec![BACKGROUND_REGION],
            DEFAULT_MATERIAL,
        )
        .unwrap();
    assert_eq!(editor.document.model.draft.material_interfaces.len(), 2);
    assert_eq!(editor.document.model.draft.junctions.len(), 4);
    assert!(validate(&editor.document.model.draft).valid());
    let vertical = &editor.document.model.draft.material_interfaces[0];
    assert_ne!(vertical.span_sides[0].right, vertical.span_sides[1].right);
    editor.delete_material_divider(branch, 0).unwrap();
    assert!(validate(&editor.document.model.draft).valid());
    assert_eq!(editor.document.model.draft.material_interfaces.len(), 1);
    assert_eq!(editor.document.model.draft.junctions.len(), 2);
}

#[test]
fn divider_curve_attachment_creates_a_shape_preserving_c0_junction() {
    let mut editor = Editor::default();
    editor.document.model.draft.obstacles.clear();
    editor.document.model.accepted.obstacles.clear();
    let first = editor
        .create_material_divider(
            OpenCubicSpline::polyline(vec![Point2::new(0.0, -1.0), Point2::new(0.0, 1.0)]).unwrap(),
            DividerEndpoint::Outer {
                side: OuterSide::Bottom,
                fraction: 0.5,
            },
            DividerEndpoint::Outer {
                side: OuterSide::Top,
                fraction: 0.5,
            },
            vec![BACKGROUND_REGION],
            DEFAULT_MATERIAL,
        )
        .unwrap();
    let before = match &editor.document.model.draft.material_interfaces[0].spline {
        InterfaceSpline::Open(spline) => spline.clone(),
        InterfaceSpline::Closed(_) => panic!("expected an open divider"),
    };
    editor
        .create_material_divider(
            OpenCubicSpline::polyline(vec![Point2::default(), Point2::new(1.0, 0.0)]).unwrap(),
            DividerEndpoint::InterfaceCurve {
                interface: first,
                parameter: 1.0,
            },
            DividerEndpoint::Outer {
                side: OuterSide::Right,
                fraction: 0.5,
            },
            vec![BACKGROUND_REGION],
            DEFAULT_MATERIAL,
        )
        .unwrap();

    let vertical = &editor.document.model.draft.material_interfaces[0];
    let InterfaceSpline::Open(spline) = &vertical.spline else {
        panic!("expected an open divider")
    };
    assert_eq!(spline.intervals().len(), 2);
    assert_eq!(spline.continuity(1), Some(0));
    assert!((spline.evaluate(spline.breakpoint(1).unwrap()) - Point2::default()).norm() < 1.0e-12);
    for sample in 0..=64 {
        let parameter = before.period() * sample as f64 / 64.0;
        assert!((spline.evaluate(parameter) - before.evaluate(parameter)).norm() < 1.0e-12);
    }
    assert!(vertical.nodes[1].junction.is_some());
    assert!(validate(&editor.document.model.draft).valid());
}

#[test]
fn divider_crossing_a_c0_node_relabels_each_entered_sector() {
    let mut editor = Editor::default();
    editor.document.model.draft.obstacles.clear();
    editor.document.model.accepted.obstacles.clear();
    editor
        .create_material_divider(
            OpenCubicSpline::polyline(vec![
                Point2::new(0.0, -1.0),
                Point2::default(),
                Point2::new(0.0, 1.0),
            ])
            .unwrap(),
            DividerEndpoint::Outer {
                side: OuterSide::Bottom,
                fraction: 0.5,
            },
            DividerEndpoint::Outer {
                side: OuterSide::Top,
                fraction: 0.5,
            },
            vec![BACKGROUND_REGION, BACKGROUND_REGION],
            DEFAULT_MATERIAL,
        )
        .unwrap();
    editor
        .create_material_divider(
            OpenCubicSpline::polyline(vec![
                Point2::new(-1.0, 0.0),
                Point2::default(),
                Point2::new(1.0, 0.0),
            ])
            .unwrap(),
            DividerEndpoint::Outer {
                side: OuterSide::Left,
                fraction: 0.5,
            },
            DividerEndpoint::Outer {
                side: OuterSide::Right,
                fraction: 0.5,
            },
            vec![RegionId(2), BACKGROUND_REGION],
            DEFAULT_MATERIAL,
        )
        .unwrap();
    assert!(validate(&editor.document.model.draft).valid());
    assert_eq!(editor.document.model.draft.regions.len(), 3);
    assert_eq!(editor.document.model.draft.junctions.len(), 4);
    let mesh = mesh_scene(
        &editor.document.model.draft,
        9,
        MeshingOptions {
            target_edge_length: 0.25,
            minimum_angle_degrees: 10.0,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        mesh.triangles
            .iter()
            .map(|triangle| triangle.region)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
    let encoded = save(&editor.document).unwrap();
    assert_eq!(parse_document(encoded.as_bytes()).unwrap(), editor.document);
    editor
        .delete_material_divider(MaterialInterfaceId(2), 0)
        .unwrap();
    assert!(validate(&editor.document.model.draft).valid());
    assert_eq!(editor.document.model.draft.regions.len(), 2);
}

#[test]
fn junction_drag_moves_every_attached_c0_node_as_one_edit() {
    let mut editor = Editor::default();
    editor.document.model.draft.obstacles.clear();
    editor.document.model.accepted.obstacles.clear();
    let first = editor
        .create_material_divider(
            OpenCubicSpline::polyline(vec![
                Point2::new(0.0, -1.0),
                Point2::default(),
                Point2::new(0.0, 1.0),
            ])
            .unwrap(),
            DividerEndpoint::Outer {
                side: OuterSide::Bottom,
                fraction: 0.5,
            },
            DividerEndpoint::Outer {
                side: OuterSide::Top,
                fraction: 0.5,
            },
            vec![BACKGROUND_REGION, BACKGROUND_REGION],
            DEFAULT_MATERIAL,
        )
        .unwrap();
    let node = editor.document.model.draft.material_interfaces[0].nodes[1].id;
    editor
        .create_material_divider(
            OpenCubicSpline::polyline(vec![Point2::default(), Point2::new(1.0, 0.0)]).unwrap(),
            DividerEndpoint::InterfaceNode {
                interface: first,
                node,
            },
            DividerEndpoint::Outer {
                side: OuterSide::Right,
                fraction: 0.5,
            },
            vec![BACKGROUND_REGION],
            DEFAULT_MATERIAL,
        )
        .unwrap();
    let junction = editor
        .document
        .model
        .draft
        .junctions
        .iter()
        .find(|junction| matches!(junction.location, JunctionLocation::Interior))
        .unwrap()
        .id;
    let history = editor.history_len().0;
    let target = Point2::new(0.12, 0.08);
    editor.begin();
    editor
        .set_junction_point_during_edit(junction, target)
        .unwrap();
    editor.commit();
    assert_eq!(editor.history_len().0, history + 1);
    let attached = editor
        .document
        .model
        .draft
        .material_interfaces
        .iter()
        .flat_map(|interface| {
            interface
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, node)| node.junction == Some(junction))
                .map(|(index, _)| interface.spline.node_point(index).unwrap())
        })
        .collect::<Vec<_>>();
    assert_eq!(attached.len(), 2);
    assert!(
        attached
            .iter()
            .all(|point| (*point - target).norm() < 1.0e-12)
    );
}
