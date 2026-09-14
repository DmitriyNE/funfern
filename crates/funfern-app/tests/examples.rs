//! The example files shipped beside the README must load in the app that ships
//! with them. One of them went stale across a schema break and sat unloadable
//! while both the README and the browser checklist pointed readers at it.

use funfern_app::{topology_examples, topology_persistence};

fn shipped(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn every_shipped_example_parses_at_the_current_version() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut seen = 0usize;
    for entry in std::fs::read_dir(&directory).expect("an examples directory") {
        let path = entry.expect("a directory entry").path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let bytes = std::fs::read(&path).expect("a readable example");
        topology_persistence::parse_document(&bytes)
            .unwrap_or_else(|error| panic!("{} does not load: {error}", path.display()));
        seen += 1;
    }
    assert!(seen > 0, "no examples were checked");
}

/// The shipped file is an export of the catalog entry of the same shape, so a
/// change to one without the other is caught here rather than by a reader. To
/// regenerate: save the Obstacle array example from the app over this file.
#[test]
fn the_obstacle_array_example_matches_the_catalog() {
    let expected = topology_examples::catalog()
        .iter()
        .find(|example| example.name == "Obstacle array")
        .expect("the catalog still carries the obstacle array");
    let document = topology_persistence::parse_document(&shipped("eight-obstacles.json"))
        .expect("the shipped example loads");
    assert_eq!(
        document.model, expected.document.model,
        "examples/eight-obstacles.json has drifted from the Obstacle array example"
    );
}
