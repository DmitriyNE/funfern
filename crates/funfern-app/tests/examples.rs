//! `examples/` holds every gallery scene as a file, exported from the catalog.
//! The catalog's builders stay the source; these tests catch a file that has
//! drifted from its scene, a scene without a file and a file without a scene.
//!
//! To regenerate after changing a scene:
//!
//! ```sh
//! FUNFERN_BLESS_EXAMPLES=1 cargo test -p funfern-app --test examples
//! ```

use funfern_app::{topology_examples, topology_persistence};
use std::path::PathBuf;

fn directory() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

/// A scene's file name: its name in lower case, apostrophes dropped and every
/// other run of non-alphanumerics a single hyphen.
fn file_name(scene: &str) -> String {
    let mut name = String::new();
    for character in scene.chars().filter(|character| *character != '\'') {
        if character.is_ascii_alphanumeric() {
            name.push(character.to_ascii_lowercase());
        } else if !name.is_empty() && !name.ends_with('-') {
            name.push('-');
        }
    }
    let trimmed = name.trim_end_matches('-');
    format!("{trimmed}.json")
}

fn blessing() -> bool {
    std::env::var_os("FUNFERN_BLESS_EXAMPLES").is_some()
}

#[test]
fn scene_file_names_are_distinct_and_plain() {
    assert_eq!(file_name("Maxwell's fisheye"), "maxwells-fisheye.json");
    assert_eq!(
        file_name("Frustrated total internal reflection"),
        "frustrated-total-internal-reflection.json"
    );
    let mut names: Vec<String> = topology_examples::catalog()
        .iter()
        .map(|example| file_name(example.name))
        .collect();
    let count = names.len();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), count, "two scenes share a file name");
}

#[test]
fn every_gallery_scene_ships_as_its_own_file() {
    let directory = directory();
    let mut stale = Vec::new();
    for example in topology_examples::catalog() {
        let path = directory.join(file_name(example.name));
        let expected = topology_persistence::save(&example.document)
            .map(|json| json + "\n")
            .unwrap_or_else(|error| panic!("{} does not save: {error}", example.name));
        if blessing() {
            std::fs::write(&path, &expected)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(shipped) if shipped == expected => {}
            Ok(_) => stale.push(format!(
                "{} has drifted from {}",
                path.display(),
                example.name
            )),
            Err(error) => stale.push(format!("{}: {error}", path.display())),
        }
    }
    assert!(
        stale.is_empty(),
        "{}\nregenerate with FUNFERN_BLESS_EXAMPLES=1 cargo test -p funfern-app --test examples",
        stale.join("\n")
    );
}

#[test]
fn every_shipped_file_is_a_gallery_scene_that_loads() {
    let scenes: Vec<String> = topology_examples::catalog()
        .iter()
        .map(|example| file_name(example.name))
        .collect();
    let mut seen = 0usize;
    for entry in std::fs::read_dir(directory()).expect("an examples directory") {
        let path = entry.expect("a directory entry").path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let name = path.file_name().and_then(|value| value.to_str()).unwrap();
        assert!(
            scenes.iter().any(|scene| scene == name),
            "{} matches no gallery scene",
            path.display()
        );
        let bytes = std::fs::read(&path).expect("a readable example");
        topology_persistence::parse_document(&bytes)
            .unwrap_or_else(|error| panic!("{} does not load: {error}", path.display()));
        seen += 1;
    }
    assert!(
        blessing() || seen == scenes.len(),
        "{seen} files for {} scenes",
        scenes.len()
    );
}
