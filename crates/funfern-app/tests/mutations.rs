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

/// The runner itself, on a throwaway repository of two mutations: one its
/// test catches, and one whose test prints the failure it is meant to find,
/// then hangs in a process it started. Until 2026-11-25 the runner read the
/// output alone and counted the second as detected, and its timeout stopped
/// the test's own process alone, leaving the hung one running.
#[cfg(unix)]
#[test]
fn the_runner_counts_a_timed_out_mutation_as_not_detected() {
    // Removed when the test ends, and the hung process with it should an
    // older runner have left it running.
    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            if let Ok(hung) = std::fs::read_to_string(self.0.join("hung.pid")) {
                let _ = Command::new("kill")
                    .args(["-9", hung.trim()])
                    .stderr(std::process::Stdio::null())
                    .status();
            }
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let scratch = Scratch(
        std::env::temp_dir().join(format!("funfern-mutation-runner-{}", std::process::id())),
    );
    let root = &scratch.0;
    let _ = std::fs::remove_dir_all(root);
    std::fs::create_dir_all(root.join("scripts")).unwrap();
    std::fs::copy(
        repository().join("scripts/mutations.sh"),
        root.join("scripts/mutations.sh"),
    )
    .unwrap();
    let git = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(root)
        .status()
        .expect("git runs");
    assert!(git.success());
    // Both raise a flag the test looks for; the second, finding it, starts
    // a process that outlives the limit and records its id.
    let patch = "diff --git a/flag b/flag\nnew file mode 100644\nindex 0000000..d00491f\n\
                 --- /dev/null\n+++ b/flag\n@@ -0,0 +1 @@\n+1\n";
    let cases = [
        (
            "a-caught",
            "if [ -f flag ]; then echo \"assertion failed: the flag is up\"; exit 1; fi",
        ),
        (
            "b-hangs",
            "if [ -f flag ]; then echo \"assertion failed: the flag is up\"; \
             sleep 300 & echo $! > hung.pid; wait; fi",
        ),
    ];
    for (name, test) in cases {
        let folder = root.join("tests/mutations").join(name);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("patch.diff"), patch).unwrap();
        std::fs::write(
            folder.join("manifest"),
            format!(
                "test: {test}\nexpect: assertion failed: the flag is up\nneeds: cpu\nabout: {name}\n"
            ),
        )
        .unwrap();
    }
    let run = Command::new("bash")
        .arg("scripts/mutations.sh")
        .current_dir(root)
        .env("MUTATION_TIMEOUT", "2")
        .env_remove("MUTATION_SHARD")
        .output()
        .expect("bash runs");
    let report = String::from_utf8_lossy(&run.stdout);
    let verdict = |name: &str| {
        report
            .lines()
            .rev()
            .find(|line| line.starts_with(name))
            .map(|line| line[name.len()..].trim().to_owned())
            .unwrap_or_default()
    };
    assert_eq!(verdict("a-caught"), "detected (cpu)", "{report}");
    assert_eq!(verdict("b-hangs"), "not detected: timed out", "{report}");
    assert_eq!(run.status.code(), Some(1), "{report}");

    let hung = std::fs::read_to_string(root.join("hung.pid")).expect("the hung test ran");
    let alive = || {
        Command::new("kill")
            .args(["-0", hung.trim()])
            .stderr(std::process::Stdio::null())
            .status()
            .expect("kill runs")
            .success()
    };
    // Gone by the time the runner moved on; a moment for the system to reap
    // what the stopped group left.
    let gone = (0..50).any(|_| {
        let gone = !alive();
        if !gone {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        gone
    });
    assert!(
        gone,
        "the hung test's process {} outlived the runner",
        hung.trim()
    );
}
