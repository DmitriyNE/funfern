//! What the application does when rendering stops: the GPU device lost, or a
//! rendering error wgpu caught. Bevy's own answer quits the application,
//! which natively is an error exit and in the browser a canvas that freezes
//! with no word on the page. Here the render world stops and the main world
//! keeps running: the session writes its autosave if the document moved since
//! the last one; the page is told in the words of its own startup overlay,
//! that the document is saved, the running simulation is not, and a reload
//! starts again; and natively the same goes to standard error and the process
//! exits with an error. The field itself is not promised: nothing holds a copy
//! of it the application could resume from.

use bevy::prelude::*;
use bevy::render::error_handler::{ErrorType, RenderError, RenderErrorHandler, RenderErrorPolicy};

/// Rendering has stopped, and why. Recorded once by [`policy`] in the main
/// world; [`announce`] acts on it once.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenderStopped {
    pub(crate) kind: StopKind,
    pub(crate) description: String,
    announced: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StopKind {
    DeviceLost,
    RenderError,
}

impl RenderStopped {
    /// The word the page's root element carries for it.
    pub(crate) fn attribute(&self) -> &'static str {
        match self.kind {
            StopKind::DeviceLost => "device-lost",
            StopKind::RenderError => "render-error",
        }
    }

    /// What the page and the terminal say. A lost device may arrive as the
    /// validation errors of the operations that failed on it before the loss
    /// itself is reported - on Metal it did - so a rendering error names the
    /// GPU too; its description's first line, since wgpu's can run to many.
    pub(crate) fn message(&self) -> String {
        let kept = "The document is saved; the running simulation is not.";
        match self.kind {
            StopKind::DeviceLost => format!("The GPU device was lost. {kept}"),
            StopKind::RenderError => {
                let description = self.description.lines().next().unwrap_or("").trim();
                if description.is_empty() {
                    format!("The GPU stopped with a rendering error. {kept}")
                } else {
                    format!("The GPU stopped with a rendering error: {description}. {kept}")
                }
            }
        }
    }
}

/// The application's render error policy, installed in place of Bevy's.
pub(crate) fn handler() -> RenderErrorHandler {
    RenderErrorHandler(policy)
}

/// Records the error in the main world, the first one only - Bevy asks every
/// frame while rendering is stopped - and stops rendering. Bevy has already
/// logged the error.
fn policy(error: &RenderError, main_world: &mut World, _: &mut World) -> RenderErrorPolicy {
    if !main_world.contains_resource::<RenderStopped>() {
        main_world.insert_resource(RenderStopped {
            kind: if error.ty == ErrorType::DeviceLost {
                StopKind::DeviceLost
            } else {
                StopKind::RenderError
            },
            description: error.description.clone(),
            announced: false,
        });
    }
    RenderErrorPolicy::StopRendering
}

/// Acts once on a stop: the autosave written, the page or the terminal told,
/// and natively the exit. An end-to-end fixture that is running reports the
/// stop itself and exits by its verdict, so with the driver present this only
/// saves and tells the page.
pub(crate) fn announce(
    stopped: Option<ResMut<RenderStopped>>,
    mut playground: ResMut<crate::ui::Playground>,
    #[cfg(feature = "e2e")] driver: Option<Res<crate::ui::e2e::Driver>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(mut stopped) = stopped else {
        return;
    };
    if stopped.announced {
        return;
    }
    stopped.announced = true;
    playground.autosave_now();
    let message = stopped.message();
    tell_page(stopped.attribute(), &message);
    #[cfg(feature = "e2e")]
    if driver.is_some() {
        return;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        eprintln!("{message}");
        exit.write(AppExit::error());
    }
    #[cfg(target_arch = "wasm32")]
    let _ = &mut exit;
}

/// Writes the stop on the page's root element as `data-funfern-stopped` and
/// shows it through the page's own overlay, `window.funfernStopped`.
#[cfg(target_arch = "wasm32")]
fn tell_page(attribute: &str, message: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    if let Some(root) = window
        .document()
        .and_then(|document| document.document_element())
    {
        let _ = root.set_attribute("data-funfern-stopped", attribute);
    }
    if let Ok(show) = js_sys::Reflect::get(&window, &"funfernStopped".into())
        && let Ok(show) = show.dyn_into::<js_sys::Function>()
    {
        let _ = show.call1(&window, &message.into());
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn tell_page(_: &str, _: &str) {}

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;

#[cfg(test)]
mod tests {
    use super::*;

    /// Bevy asks every frame while rendering is stopped; the first error is
    /// the one recorded, as the device lost it was, and rendering stays
    /// stopped. The page's word and the message follow the kind.
    #[test]
    fn the_first_error_is_recorded_once_and_rendering_stays_stopped() {
        let mut main_world = World::new();
        let mut render_world = World::new();
        let lost = RenderError {
            ty: ErrorType::DeviceLost,
            description: "Destroyed".to_owned(),
            source: None,
        };
        let validation = RenderError {
            ty: ErrorType::Validation,
            description: "a buffer too small".to_owned(),
            source: None,
        };
        for error in [&lost, &validation, &lost] {
            assert!(matches!(
                policy(error, &mut main_world, &mut render_world),
                RenderErrorPolicy::StopRendering
            ));
        }
        let stopped = main_world.resource::<RenderStopped>();
        assert_eq!(stopped.kind, StopKind::DeviceLost);
        assert_eq!(stopped.attribute(), "device-lost");
        assert!(stopped.message().contains("The GPU device was lost"));
        assert!(!stopped.announced);

        let mut other = World::new();
        policy(&validation, &mut other, &mut render_world);
        let stopped = other.resource::<RenderStopped>();
        assert_eq!(stopped.kind, StopKind::RenderError);
        assert_eq!(stopped.attribute(), "render-error");
        assert!(stopped.message().contains("a buffer too small"));
        let two_lines = RenderError {
            ty: ErrorType::Validation,
            description: "first line\n  second line of wgpu's".to_owned(),
            source: None,
        };
        let mut third = World::new();
        policy(&two_lines, &mut third, &mut render_world);
        let message = third.resource::<RenderStopped>().message();
        assert!(message.contains("first line") && !message.contains("second line"));
    }
}
