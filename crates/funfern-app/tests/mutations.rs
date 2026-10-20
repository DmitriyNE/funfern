//! `tests/mutations/` holds one deliberate defect per folder, a patch and a
//! manifest, which `scripts/mutations.sh` applies, runs the named test
//! against and reverts, nightly. A patch whose context the code has moved
//! past no longer applies, and the nightly reports it as not detected while
//! the check it guards goes unguarded: on 2026-10-19 five had gone that way
//! since the clock and far-field work two days before. These tests catch it
//! in the gate, the day the code moves.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn mutations() -> Vec<PathBuf> {
    let mut folders = std::fs::read_dir(repository().join("tests/mutations"))
        .expect("tests/mutations is readable")
        .map(|entry| entry.expect("a readable entry").path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    folders.sort();
    folders
}

fn name(folder: &Path) -> String {
    folder.file_name().unwrap().to_string_lossy().into_owned()
}

#[test]
fn every_mutation_patch_still_applies() {
    let folders = mutations();
    assert!(!folders.is_empty(), "no mutations found");
    let stale = folders
        .iter()
        .filter(|folder| {
            let status = Command::new("git")
                .arg("-C")
                .arg(repository())
                .args(["apply", "--check"])
                .arg(folder.join("patch.diff"))
                .output()
                .expect("git runs");
            !status.status.success()
        })
        .map(|folder| name(folder))
        .collect::<Vec<_>>();
    assert!(
        stale.is_empty(),
        "these mutation patches no longer apply to the tree; rewrite them against it: {stale:?}"
    );
}

/// The fields `scripts/mutations.sh` reads, each present once and not empty,
/// and `needs` one it knows.
#[test]
fn every_mutation_manifest_names_its_test_and_failure() {
    for folder in mutations() {
        let manifest = std::fs::read_to_string(folder.join("manifest"))
            .unwrap_or_else(|_| panic!("{} has no manifest", name(&folder)));
        for field in ["test", "expect", "needs", "about"] {
            let values = manifest
                .lines()
                .filter_map(|line| line.strip_prefix(&format!("{field}: ")))
                .collect::<Vec<_>>();
            assert!(
                values.len() == 1 && !values[0].trim().is_empty(),
                "{}'s manifest needs one `{field}:` line",
                name(&folder)
            );
            if field == "needs" {
                assert!(
                    ["cpu", "device"].contains(&values[0].trim()),
                    "{}'s manifest needs `cpu` or `device`, not {:?}",
                    name(&folder),
                    values[0]
                );
            }
        }
    }
}
