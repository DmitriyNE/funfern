//! Version 22 is the last break: files, links and autosaves written from it on
//! must keep opening. `tests/fixtures/scenes-v22` holds every gallery scene as
//! version 22 wrote it on 2026-10-03, frozen, unlike `examples/`, which follows
//! the catalog. Never regenerate them; see the folder's README.

use funfern_app::topology_persistence;
use serde_json::Value;
use std::path::PathBuf;

fn directory() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scenes-v22")
}

fn frozen_scenes() -> Vec<(String, Vec<u8>)> {
    let mut scenes: Vec<_> = std::fs::read_dir(directory())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read(path).unwrap())
        })
        .collect();
    scenes.sort();
    scenes
}

/// Where `read` does not hold what `held` did: every key `held` has, with the
/// same value. A key `read` adds, a later version's default, is not counted.
fn changed(held: &Value, read: &Value, path: &str, out: &mut Vec<String>) {
    match (held, read) {
        (Value::Object(held), Value::Object(read)) => {
            for (key, value) in held {
                match read.get(key) {
                    Some(read) => changed(value, read, &format!("{path}/{key}"), out),
                    None => out.push(format!("{path}/{key} is no longer written")),
                }
            }
        }
        (Value::Array(held), Value::Array(read)) if held.len() == read.len() => {
            for (index, (held, read)) in held.iter().zip(read).enumerate() {
                changed(held, read, &format!("{path}[{index}]"), out);
            }
        }
        _ if held == read => {}
        _ => out.push(format!("{path}: {held} reads back as {read}")),
    }
}

#[test]
fn every_frozen_scene_is_there() {
    assert_eq!(frozen_scenes().len(), 45);
}

#[test]
fn every_frozen_version_22_scene_opens_with_what_it_held() {
    let mut failures = Vec::new();
    for (name, bytes) in frozen_scenes() {
        let document = match topology_persistence::parse_document(&bytes) {
            Ok(document) => document,
            Err(error) => {
                failures.push(format!("{name} does not open: {error}"));
                continue;
            }
        };
        let held: Value = serde_json::from_slice(&bytes).unwrap();
        let read: Value =
            serde_json::from_slice(&topology_persistence::save_compact(&document).unwrap())
                .unwrap();
        let mut differences = Vec::new();
        changed(&held, &read, "", &mut differences);
        failures.extend(
            differences
                .into_iter()
                .take(5)
                .map(|difference| format!("{name}: {difference}")),
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The comparison itself: it finds a dropped key and a changed value, and
/// lets an added key pass.
#[test]
fn a_read_back_scene_is_compared_by_what_the_file_held() {
    let held = serde_json::json!({"a": 1, "b": [1.5, {"c": "x"}], "d": true});
    let mut out = Vec::new();
    let mut added = held.clone();
    added["e"] = 2.into();
    changed(&held, &added, "", &mut out);
    assert!(out.is_empty(), "{out:?}");
    let mut moved = held.clone();
    moved["b"][1]["c"] = "y".into();
    moved.as_object_mut().unwrap().remove("d");
    changed(&held, &moved, "", &mut out);
    assert_eq!(
        out,
        [
            "/b[1]/c: \"x\" reads back as \"y\"".to_string(),
            "/d is no longer written".to_string()
        ]
    );
}
