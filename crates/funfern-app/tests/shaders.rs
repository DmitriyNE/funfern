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

/// The float literals in `text` that WebKit's WGSL compiler cannot read: a run
/// of digits and a suffix, with no point or exponent, past `i64::MAX`. The
/// grammar allows them; Safari reads the digits as a 64-bit integer first and
/// rejects the shader when they overflow.
fn literals_webkit_rejects(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut rejected = vec![];
    let mut start = 0;
    while start < bytes.len() {
        if !bytes[start].is_ascii_digit()
            || (start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'.'))
        {
            start += 1;
            continue;
        }
        let end = start
            + bytes[start..]
                .iter()
                .take_while(|b| b.is_ascii_digit())
                .count();
        let digits = &text[start..end];
        if matches!(bytes.get(end), Some(b'f' | b'h'))
            && !bytes.get(end + 1).is_some_and(u8::is_ascii_alphanumeric)
            && digits.parse::<i64>().is_err()
        {
            rejected.push(format!("{digits}{}", bytes[end] as char));
        }
        start = end;
    }
    rejected
}

/// A browser is not given the source: Bevy composes each shader into naga's
/// module and wgpu writes that back out as WGSL, which is what Safari compiles.
/// The writer prints an `f32` constant in full, digits only, so `3.0e38` goes
/// out as a 39-digit literal Safari rejects, and with it every pipeline of the
/// shader. Chrome reads it, so the browser shader test cannot see it.
#[test]
fn every_shader_reaches_a_browser_in_a_form_safari_reads() {
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    let mut failures = vec![];
    for path in shaders() {
        let name = path.file_name().and_then(|value| value.to_str()).unwrap();
        let source = std::fs::read_to_string(&path).expect("a readable shader");
        let module = naga::front::wgsl::parse_str(&source).expect("a shader that parses");
        let info = validator
            .validate(&module)
            .expect("a shader that validates");
        let written =
            naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())
                .unwrap_or_else(|error| panic!("{name} cannot be written back: {error}"));
        let rejected = literals_webkit_rejects(&written);
        if !rejected.is_empty() {
            failures.push(format!("{name}: {}", rejected.join(", ")));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_literal_scan_finds_what_safari_rejects_and_nothing_else() {
    assert_eq!(
        literals_webkit_rejects("const A: f32 = -300000000000000000000000000000000000000f;"),
        ["300000000000000000000000000000000000000f"]
    );
    assert_eq!(
        literals_webkit_rejects("let a = 9223372036854775808f + 1f;"),
        ["9223372036854775808f"]
    );
    for accepted in [
        "let a = 9223372036854775807f;",
        "let a = 3e38f;",
        "let a = 300000000000000000000000000000000000000.0f;",
        "let a = 0.000000000000000000000000000000000000001f;",
        "let a = x12345678901234567890f;",
        "let a = 12345678901234567890u;",
    ] {
        assert!(literals_webkit_rejects(accepted).is_empty(), "{accepted}");
    }
}
