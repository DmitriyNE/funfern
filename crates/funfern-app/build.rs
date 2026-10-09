//! Stamps the build with what it was built from, as `FUNFERN_BUILD_ID`: the
//! environment variable of that name when the build sets one - CI passes the
//! commit it checked out - else the checkout's own commit, else `unknown`.
//! The application writes it on the page and prints it for `--version`, and
//! the browser runs hold what they load to it, so a stale service worker or a
//! cached bundle serving an older application fails a run instead of passing
//! on old code. The commit alone: a dirty tree is not tracked, since a change
//! to a file this script does not watch would not rerun it.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=FUNFERN_BUILD_ID");
    let id = std::env::var("FUNFERN_BUILD_ID")
        .ok()
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
        .or_else(commit)
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=FUNFERN_BUILD_ID={id}");
}

/// The checkout's commit, twelve digits, and the files a new commit changes,
/// so the stamp follows the checkout.
fn commit() -> Option<String> {
    let git = |arguments: &[&str]| {
        let output = Command::new("git").args(arguments).output().ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    let git_dir = git(&["rev-parse", "--absolute-git-dir"])?;
    println!("cargo:rerun-if-changed={git_dir}/HEAD");
    if let Some(reference) = git(&["symbolic-ref", "--quiet", "HEAD"]) {
        println!("cargo:rerun-if-changed={git_dir}/{reference}");
        println!("cargo:rerun-if-changed={git_dir}/packed-refs");
    }
    git(&["rev-parse", "--short=12", "HEAD"]).filter(|id| !id.is_empty())
}
