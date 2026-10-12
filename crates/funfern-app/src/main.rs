//! The shipped app.
use funfern_app::Build;

/// The `e2e` feature, deprecated on 2026-10-12, still makes this binary the
/// fixtures build, as it did before `funfern-fixtures`, until the browser
/// fixtures run that binary too; then the feature goes.
fn build() -> Build {
    if cfg!(feature = "e2e") {
        Build::Fixtures
    } else {
        Build::Shipped
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> bevy::app::AppExit {
    funfern_app::native_main(env!("CARGO_BIN_NAME"), build())
}

#[cfg(target_arch = "wasm32")]
fn main() {
    funfern_app::browser_main(build());
}
