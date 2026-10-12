//! The app with its end-to-end driver (`ui::e2e`): the binary the device
//! suite, the lavapipe job and the mutation runner run, natively with
//! `FUNFERN_E2E=<fixture>` and in the browser with `?e2e=<fixture>`. The
//! shipped `funfern-app` carries no driver.
use funfern_app::Build;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> bevy::app::AppExit {
    funfern_app::native_main(env!("CARGO_BIN_NAME"), Build::Fixtures)
}

#[cfg(target_arch = "wasm32")]
fn main() {
    funfern_app::browser_main(Build::Fixtures);
}
