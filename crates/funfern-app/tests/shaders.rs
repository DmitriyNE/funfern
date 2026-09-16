//! The shaders are the one part of this app nothing in the workspace read.
//!
//! Every wave kernel edit was checked by hand against a throwaway crate, which
//! caught a reserved keyword once and depended on remembering to do it. naga is
//! pinned to the version wgpu links, so this is the parse and validation the
//! app's own device performs, minus the backend translation.

use std::path::{Path, PathBuf};

fn shaders() -> Vec<PathBuf> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut paths = std::fs::read_dir(&directory)
        .expect("a source directory")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("wgsl"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

#[test]
fn every_shader_parses_and_validates() {
    let paths = shaders();
    assert!(
        paths.len() >= 7,
        "only {} shaders found; this test is looking in the wrong place",
        paths.len()
    );
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    for path in paths {
        let name = path.file_name().and_then(|value| value.to_str()).unwrap();
        let source = std::fs::read_to_string(&path).expect("a readable shader");
        let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|error| {
            panic!("{name} does not parse: {}", error.emit_to_string(&source))
        });
        validator
            .validate(&module)
            .unwrap_or_else(|error| panic!("{name} does not validate: {error:?}"));
    }
}
