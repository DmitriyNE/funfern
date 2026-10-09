//! End-to-end fixtures: the app as it ships, driven through its own handlers
//! on this machine's GPU and judged against the f64 reference stepped on the
//! generation the app accepted. Built with the `e2e` feature and chosen by
//! `FUNFERN_E2E=<fixture>`; the process exits 0 when the fixture holds, and 1
//! when it does not or does not finish in time.
//!
//! The driver acts as the user's handlers do - it opens a document, places a
//! pulse, presses Run - and reads only what the device accepted: its
//! accepted-step counter, the serial of the last event it processed, and a
//! full snapshot stamped with the step it holds. The one thing it adds to the
//! app is where the run stops ([`E2eSteps`]), since the app asks for steps by
//! the wall clock and a comparison needs an exact endpoint.

#[path = "../../examples/support/check.rs"]
mod check;

use super::runtime::pulse_increment;
use super::*;
use bevy_egui::EguiPrimaryContextPass;
use check::{within, worse};
use funfern_app::topology_editor::{TopologyDocument, TopologyEditor};
use funfern_core::{CanonicalWaveState, OuterBoundaryCondition, OuterBoundaryConditions};

/// Where an end-to-end fixture holds the run: the steps a running frame asks
/// for stop at `limit`, counted on the running generation.
#[derive(Default)]
pub(super) struct E2eSteps {
    pub(super) limit: Option<u64>,
}

/// Adds the driver of the fixture `FUNFERN_E2E` names, if it names one.
pub(crate) fn add(app: &mut App) {
    let Ok(name) = std::env::var("FUNFERN_E2E") else {
        return;
    };
    let fixture = match name.as_str() {
        "cavity" => Fixture::Cavity,
        _ => {
            eprintln!("e2e: no fixture named {name:?}");
            std::process::exit(2);
        }
    };
    app.insert_resource(Driver {
        fixture,
        phase: Phase::Open,
        deadline: Instant::now() + std::time::Duration::from_secs(120),
    })
    .add_systems(EguiPrimaryContextPass, drive.before(frame));
}

#[derive(Clone, Copy, Debug)]
enum Fixture {
    /// A closed reflecting box of a lossless linear medium, struck by one
    /// pulse at step 0 and run for [`CAVITY_STEPS`].
    Cavity,
}

/// Steps the cavity runs after its pulse.
const CAVITY_STEPS: u64 = 256;

/// The relative L2 error Q and b may reach, as the device examples allow
/// for f32 stepping over a few hundred steps.
const CAVITY_TOLERANCE: f64 = 3.0e-5;

#[derive(Resource)]
struct Driver {
    fixture: Fixture,
    phase: Phase,
    deadline: Instant,
}

enum Phase {
    /// The app is starting; the fixture's document opens once it has.
    Open,
    /// The document's generation is being prepared and installed.
    Install,
    /// The pulse is placed and waits for the device to take it.
    Pulse {
        reference: Reference,
        processed: u32,
    },
    /// The run steps to its endpoint.
    Run {
        reference: Reference,
    },
    /// A full snapshot of the endpoint is asked for.
    Snapshot {
        reference: Reference,
        seen: u64,
    },
    Done,
}

/// The f64 reference, on the generation the app accepted.
struct Reference {
    active: Arc<PreparedTopology>,
    state: CanonicalWaveState,
}

fn drive(
    mut driver: ResMut<Driver>,
    mut state: ResMut<Playground>,
    mut request: ResMut<CanonicalGpuRequest>,
    display: Res<CanonicalGpuDisplay>,
    mut exit: MessageWriter<AppExit>,
) {
    if matches!(driver.phase, Phase::Done) {
        return;
    }
    let fixture = driver.fixture;
    let verdict = match fixture {
        Fixture::Cavity => cavity(&mut driver, &mut state, &mut request, &display),
    };
    let verdict = match verdict {
        Some(verdict) => verdict,
        None if Instant::now() >= driver.deadline => Err(format!(
            "did not finish in time, at {}",
            driver.phase.name()
        )),
        None => return,
    };
    driver.phase = Phase::Done;
    match verdict {
        Ok(summary) => {
            println!("e2e {fixture:?}: {summary}");
            exit.write(AppExit::Success);
        }
        Err(failure) => {
            eprintln!("e2e {fixture:?} failed: {failure}");
            exit.write(AppExit::error());
        }
    }
}

impl Phase {
    fn name(&self) -> &'static str {
        match self {
            Phase::Open => "opening the document",
            Phase::Install => "installing its generation",
            Phase::Pulse { .. } => "waiting for the device to take the pulse",
            Phase::Run { .. } => "running to the endpoint",
            Phase::Snapshot { .. } => "waiting for the endpoint's snapshot",
            Phase::Done => "done",
        }
    }
}

/// The unit square with reflecting walls, the default medium alone, no
/// source, no grid filter, adaptation off: nothing but the pulse puts energy
/// in, and nothing takes it out.
fn cavity_document() -> TopologyDocument {
    let mut document = TopologyEditor::default().document;
    let walls = OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting);
    document.model.draft.outer_boundaries = walls;
    document.model.accepted.outer_boundaries = walls;
    document.model.source.enabled = false;
    document.presentation.mesh_edge = 0.07;
    document.presentation.adaptation.enabled = false;
    document.presentation.grid_scale_filter = false;
    document
}

/// Advances the cavity a phase; answers its verdict once it has one.
fn cavity(
    driver: &mut Driver,
    state: &mut Playground,
    request: &mut CanonicalGpuRequest,
    display: &CanonicalGpuDisplay,
) -> Option<Result<String, String>> {
    match std::mem::replace(&mut driver.phase, Phase::Done) {
        Phase::Open => {
            if !state.startup_done {
                driver.phase = Phase::Open;
                return None;
            }
            // Nothing steps until the pulse is in: the run waits at step 0.
            state.e2e_steps.limit = Some(0);
            state.wave_running = true;
            if let Err(error) = state.set_document(cavity_document(), false, true) {
                return Some(Err(format!("the document did not open: {error}")));
            }
            driver.phase = Phase::Install;
            None
        }
        Phase::Install => {
            let installed = state.runtime.active().cloned().filter(|active| {
                *active.bundle.authored == state.editor.document.model.accepted
                    && state.coordinator.uploading.is_none()
                    && request.generation() != 0
                    && display.generation == request.generation()
            });
            let Some(active) = installed else {
                driver.phase = Phase::Install;
                return None;
            };
            if active.canonical_temporal_operator.is_some() {
                return Some(Err("the cavity prepared a time-varying generation".into()));
            }
            let domain = state.editor.document.model.accepted.geometry.domain;
            let point = Point2::new(
                domain.min_x + 0.37 * domain.width(),
                domain.min_y + 0.58 * (domain.max_y - domain.min_y),
            );
            state.place_pulse(point);
            let Some(&(position, region)) = state.coordinator.pending_pulses.back() else {
                return Some(Err(format!("the pulse was not queued: {}", state.message)));
            };
            let time_step = state.coordinator.uploaded_time_step;
            let increment = match pulse_increment(
                &active,
                position,
                region,
                f64::from(state.pulse_amplitude),
                f64::from(state.pulse_width),
            ) {
                Ok(increment) => increment,
                Err(error) => return Some(Err(format!("the pulse has no profile: {error}"))),
            };
            let operator = &active.canonical_operator;
            let mut reference = match CanonicalWaveState::zero(operator, time_step) {
                Ok(reference) => reference,
                Err(error) => return Some(Err(format!("the reference did not start: {error}"))),
            };
            if let Err(error) =
                reference.apply_primary_pulse(operator, &active.canonical_forcing, &increment)
            {
                return Some(Err(format!("the reference took no pulse: {error}")));
            }
            println!(
                "e2e cavity: {} DOFs, step {time_step:.4e}, pulse at ({:.3}, {:.3})",
                operator.degrees_of_freedom(),
                position.x,
                position.y
            );
            driver.phase = Phase::Pulse {
                reference: Reference {
                    active,
                    state: reference,
                },
                processed: request.stats().processed_event(),
            };
            None
        }
        Phase::Pulse {
            mut reference,
            processed,
        } => {
            if request.stats().processed_event() == processed
                || !state.coordinator.pending_pulses.is_empty()
            {
                driver.phase = Phase::Pulse {
                    reference,
                    processed,
                };
                return None;
            }
            if request.stats().event_rejection() != 0 {
                return Some(Err(format!(
                    "the device refused the pulse (failure code {})",
                    request.stats().event_rejection()
                )));
            }
            if request.stats().completed_steps() != 0 {
                return Some(Err(format!(
                    "the pulse landed at step {}, not 0",
                    request.stats().completed_steps()
                )));
            }
            let operator = &reference.active.canonical_operator;
            for _ in 0..CAVITY_STEPS {
                if let Err(error) = reference
                    .state
                    .step_with_forcing(operator, &reference.active.canonical_forcing)
                {
                    return Some(Err(format!("the reference did not step: {error}")));
                }
            }
            state.e2e_steps.limit = Some(CAVITY_STEPS);
            driver.phase = Phase::Run { reference };
            None
        }
        Phase::Run { reference } => {
            let completed = request.stats().completed_steps();
            if completed < CAVITY_STEPS {
                driver.phase = Phase::Run { reference };
                return None;
            }
            if completed > CAVITY_STEPS {
                return Some(Err(format!(
                    "the run went past its endpoint, to {completed}"
                )));
            }
            let seen = display.full_readbacks;
            request.request_full_state_readback();
            driver.phase = Phase::Snapshot { reference, seen };
            None
        }
        Phase::Snapshot { reference, seen } => {
            // Only a snapshot of this generation, stamped with the endpoint
            // inside the state it copied, is the endpoint's.
            let arrived = display.full_readbacks > seen
                && display.generation == request.generation()
                && display.full_snapshot_completed_steps() == CAVITY_STEPS;
            if !arrived {
                request.request_full_state_readback();
                driver.phase = Phase::Snapshot { reference, seen };
                return None;
            }
            Some(compare(&reference, display))
        }
        Phase::Done => None,
    }
}

/// The endpoint's snapshot against the reference: the relative L2 error of Q
/// and of b, each within the cavity's tolerance.
fn compare(reference: &Reference, display: &CanonicalGpuDisplay) -> Result<String, String> {
    let primary = reference.state.primary_flux();
    let complementary = reference.state.complementary_flux();
    if display.primary_flux.len() != primary.len()
        || display.complementary_flux.len() != complementary.len()
    {
        return Err(format!(
            "the snapshot holds {} and {} values where the generation has {} and {}",
            display.primary_flux.len(),
            display.complementary_flux.len(),
            primary.len(),
            complementary.len()
        ));
    }
    let q = relative_l2(
        display.primary_flux.iter().map(|value| f64::from(*value)),
        primary.iter().copied(),
    );
    let b = relative_l2(
        display
            .complementary_flux
            .iter()
            .flat_map(|value| value.iter().map(|lane| f64::from(*lane))),
        complementary.iter().flat_map(|value| [value.x, value.y]),
    );
    let summary = format!("Q {q:.3e}, b {b:.3e} after {CAVITY_STEPS} steps");
    if within(worse(q, b), CAVITY_TOLERANCE) {
        Ok(summary)
    } else {
        Err(format!("{summary}, beyond {CAVITY_TOLERANCE:.0e}"))
    }
}

/// `‖actual − expected‖ / ‖expected‖`, a NaN on either side carried through.
fn relative_l2(actual: impl Iterator<Item = f64>, expected: impl Iterator<Item = f64>) -> f64 {
    let (difference, scale) = actual.zip(expected).fold((0.0, 0.0), |sum, pair| {
        (sum.0 + (pair.0 - pair.1).powi(2), sum.1 + pair.1 * pair.1)
    });
    (difference / scale).sqrt()
}
