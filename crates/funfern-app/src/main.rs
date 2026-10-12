//! The shipped app, without the end-to-end driver: that is
//! `funfern-fixtures` (`bin/funfern-fixtures.rs`).
use funfern_app::Build;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> bevy::app::AppExit {
    funfern_app::native_main(env!("CARGO_BIN_NAME"), Build::Shipped)
}

#[cfg(target_arch = "wasm32")]
fn main() {
    funfern_app::browser_main(Build::Shipped);
}
