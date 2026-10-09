//! End-to-end fixtures: the app as it ships, driven through its own handlers
//! on this machine's GPU and judged against the f64 reference stepped on the
//! generation the app accepted. Built with the `e2e` feature and chosen by
//! `FUNFERN_E2E=<fixture>`; the process exits 0 when the fixture holds, and 1
//! when it does not or does not finish in time.
//!
//! The driver acts as the user's handlers do - it opens a document, places a
//! pulse, presses Run and Switch - and reads only what the device accepted:
//! its accepted-step counter, the serial of the last event it processed, and
//! a full snapshot stamped with the step it holds. A fixture holds the run at
//! the steps where it acts, so each action lands at a step both sides know,
//! and the reference takes it there. What the driver adds to the app is where
//! a run stops and, for a fixture that asks, how many steps a running frame
//! asks for ([`E2eSteps`]): the app asks by the wall clock, and a comparison
//! needs an exact endpoint.

#[path = "../../examples/support/check.rs"]
mod check;

use super::runtime::pulse_increment;
use super::*;
use bevy_egui::EguiPrimaryContextPass;
use check::{within, worse};
use funfern_app::topology_editor::{ClosedCurvePurpose, TopologyDocument, TopologyEditor};
use funfern_core::{
    CanonicalTemporalWaveState, CanonicalWaveState, OuterBoundaryCondition, OuterBoundaryConditions,
};
use std::collections::VecDeque;

/// Where an end-to-end fixture holds the run: the steps a running frame asks
/// for stop at `limit`, counted on the running generation, and are `batch` a
/// frame where it names a number, in place of the frame's pacing.
#[derive(Default)]
pub(super) struct E2eSteps {
    pub(super) limit: Option<u64>,
    pub(super) batch: Option<u64>,
}

/// Adds the driver of the fixture `FUNFERN_E2E` names, if it names one.
pub(crate) fn add(app: &mut App) {
    let Ok(name) = std::env::var("FUNFERN_E2E") else {
        return;
    };
    let (fixture, batches) = match name.as_str() {
        "cavity" => (Fixture::Cavity, vec![None]),
        "cavity-batches" => (
            Fixture::CavityBatches,
            BATCHES.iter().copied().map(Some).collect(),
        ),
        "switch" => (Fixture::Switch, vec![None]),
        _ => {
            eprintln!("e2e: no fixture named {name:?}");
            std::process::exit(2);
        }
    };
    app.insert_resource(Driver {
        fixture,
        phase: Phase::Open,
        material: None,
        batches: batches.into(),
        hashes: Vec::new(),
        deadline: Instant::now() + std::time::Duration::from_secs(120),
    })
    .add_systems(EguiPrimaryContextPass, drive.before(frame));
}

#[derive(Clone, Copy, Debug)]
enum Fixture {
    /// A closed reflecting box of a lossless linear medium, struck by one
    /// pulse at step 0 and run for 256 steps.
    Cavity,
    /// The cavity once for each of [`BATCHES`], the same steps asked for in
    /// batches of that many: the accepted state at the endpoint must be the
    /// same bits each time. The device keeps its clock and control state on
    /// the GPU, takes no host write while stepping, and reduces in fixed-order
    /// workgroup trees, so how the steps were grouped into submissions has no
    /// arithmetic to change.
    CavityBatches,
    /// The cavity with a disc of a switchable medium, its Switch pressed at
    /// step 128 of 256, once the pulse's wave has reached it.
    Switch,
}

/// The batches [`Fixture::CavityBatches`] asks for its steps in: one at a
/// time, a number that divides nothing, half the run, and the whole run.
const BATCHES: [u64; 4] = [1, 7, 128, 256];

/// What a fixture does at a step it holds the run at.
#[derive(Clone, Copy, Debug)]
enum Action {
    /// Switch's hotkey, which throws the scene's only Switch.
    Switch,
}

impl Fixture {
    /// The step each action is taken at, in order, and the endpoint.
    fn script(self) -> (&'static [(u64, Action)], u64) {
        match self {
            Fixture::Cavity | Fixture::CavityBatches => (&[], 256),
            Fixture::Switch => (&[(128, Action::Switch)], 256),
        }
    }

    /// The relative L2 error Q, b and r may reach: what the device examples
    /// allow for f32 stepping over a few hundred steps.
    fn tolerance(self) -> f64 {
        3.0e-5
    }

    /// The fixture's document, and the material its Switch throws.
    fn document(self) -> (TopologyDocument, Option<MaterialId>) {
        match self {
            Fixture::Cavity | Fixture::CavityBatches => (cavity_document(), None),
            Fixture::Switch => {
                let (document, material) = switch_document();
                (document, Some(material))
            }
        }
    }
}

#[derive(Resource)]
struct Driver {
    fixture: Fixture,
    phase: Phase,
    /// The material the fixture's Switch throws.
    material: Option<MaterialId>,
    /// The batch of each run still to come, the current one first; none, the
    /// frame's own pacing.
    batches: VecDeque<Option<u64>>,
    /// Each finished run's batch and the hash of its accepted endpoint.
    hashes: Vec<(Option<u64>, u64)>,
    deadline: Instant,
}

/// A run that held: what it measured, and a hash of the accepted state at its
/// endpoint.
struct Run {
    summary: String,
    hash: u64,
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
    /// The run steps to the `next` action's step, or to the endpoint.
    Run {
        reference: Reference,
        next: usize,
    },
    /// The `next` action waits for the device to take it.
    Act {
        reference: Reference,
        next: usize,
        processed: u32,
    },
    /// A full snapshot of the endpoint is asked for.
    Snapshot {
        reference: Reference,
        seen: u64,
    },
    Done,
}

impl Phase {
    fn name(&self) -> &'static str {
        match self {
            Phase::Open => "opening the document",
            Phase::Install => "installing its generation",
            Phase::Pulse { .. } => "waiting for the device to take the pulse",
            Phase::Run { .. } => "running to the next stop",
            Phase::Act { .. } => "waiting for the device to take the action",
            Phase::Snapshot { .. } => "waiting for the endpoint's snapshot",
            Phase::Done => "done",
        }
    }
}

/// The f64 reference, on the generation the app accepted, and the same run
/// without the fixture's actions.
struct Reference {
    active: Arc<PreparedTopology>,
    state: State,
    /// The device must stand far from it: an action that changed nothing
    /// would prove nothing. None until the first action.
    control: Option<State>,
    steps: u64,
}

#[derive(Clone)]
enum State {
    Fixed(CanonicalWaveState),
    Temporal(CanonicalTemporalWaveState),
}

impl State {
    fn zero(active: &PreparedTopology, time_step: f64) -> Result<Self, String> {
        match &active.canonical_temporal_operator {
            Some(temporal) => CanonicalTemporalWaveState::zero(temporal, time_step)
                .map(State::Temporal)
                .map_err(|error| error.to_string()),
            None => CanonicalWaveState::zero(&active.canonical_operator, time_step)
                .map(State::Fixed)
                .map_err(|error| error.to_string()),
        }
    }

    fn pulse(&mut self, active: &PreparedTopology, increment: &[f64]) -> Result<(), String> {
        let forcing = &active.canonical_forcing;
        match (self, &active.canonical_temporal_operator) {
            (State::Fixed(state), _) => state
                .apply_primary_pulse(&active.canonical_operator, forcing, increment)
                .map(|_| ()),
            (State::Temporal(state), Some(temporal)) => {
                state.apply_primary_pulse(temporal, forcing, increment)
            }
            (State::Temporal(_), None) => unreachable!("a temporal state on a fixed generation"),
        }
        .map_err(|error| error.to_string())
    }

    fn step(&mut self, active: &PreparedTopology) -> Result<(), String> {
        let forcing = &active.canonical_forcing;
        match (self, &active.canonical_temporal_operator) {
            (State::Fixed(state), _) => state
                .step_with_forcing(&active.canonical_operator, forcing)
                .map(|_| ()),
            (State::Temporal(state), Some(temporal)) => {
                state.step_with_forcing(temporal, forcing).map(|_| ())
            }
            (State::Temporal(_), None) => unreachable!("a temporal state on a fixed generation"),
        }
        .map_err(|error| error.to_string())
    }

    fn primary(&self) -> &[f64] {
        match self {
            State::Fixed(state) => state.primary_flux(),
            State::Temporal(state) => state.primary_flux(),
        }
    }

    fn complementary(&self) -> &[Point2] {
        match self {
            State::Fixed(state) => state.complementary_flux(),
            State::Temporal(state) => state.complementary_flux(),
        }
    }

    /// The integrated field `r`, empty where the medium keeps none.
    fn integrated(&self) -> &[f64] {
        match self {
            State::Fixed(_) => &[],
            State::Temporal(state) => state.integrated_field(),
        }
    }
}

impl Reference {
    /// Steps the reference, and the control, to the accepted step `step`.
    fn advance_to(&mut self, step: u64) -> Result<(), String> {
        while self.steps < step {
            self.state.step(&self.active)?;
            if let Some(control) = &mut self.control {
                control.step(&self.active)?;
            }
            self.steps += 1;
        }
        Ok(())
    }
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
    let run = match advance(&mut driver, &mut state, &mut request, &display) {
        Some(run) => run,
        None if Instant::now() >= driver.deadline => Err(format!(
            "did not finish in time, at {}",
            driver.phase.name()
        )),
        None => return,
    };
    let verdict = run.map(|run| {
        let batch = driver.batches.pop_front().flatten();
        let label = batch.map_or(String::new(), |batch| format!(" in batches of {batch}"));
        println!(
            "e2e {fixture:?}{label}: {}, endpoint hash {:016x}",
            run.summary, run.hash
        );
        driver.hashes.push((batch, run.hash));
        run.summary
    });
    if verdict.is_ok() && !driver.batches.is_empty() {
        driver.phase = Phase::Open;
        return;
    }
    let verdict = verdict.and_then(|summary| match driver.hashes.as_slice() {
        [(_, first), rest @ ..] if rest.iter().any(|(_, hash)| hash != first) => Err(format!(
            "the endpoint differs with the batching: {}",
            driver
                .hashes
                .iter()
                .map(|(batch, hash)| format!("{}: {hash:016x}", batch.unwrap_or(0)))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        [_, _, ..] => Ok(format!(
            "the endpoint is the same bits in all {} batchings",
            driver.hashes.len()
        )),
        _ => Ok(summary),
    });
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

/// The cavity with a disc of the switchable medium preset on its mass row,
/// near enough the pulse for its wave to reach the disc by the Switch.
fn switch_document() -> (TopologyDocument, MaterialId) {
    let mut editor = TopologyEditor::from_document(cavity_document()).expect("the cavity opens");
    let material = editor.add_material().expect("a material to switch");
    let medium = editor
        .document
        .model
        .draft
        .material(material)
        .expect("the material added")
        .clone();
    let preset = law_presets()
        .iter()
        .find(|preset| preset.name == "Switchable medium" && preset.row == LawPresetRow::Mass)
        .expect("the switchable medium preset");
    editor
        .update_material(apply_law_preset(preset, &medium).expect("the preset applies"))
        .expect("the switchable medium");
    editor
        .create_closed_curve(
            PeriodicCubicSpline::rounded(Point2::new(0.15, -0.1), 0.3),
            ClosedCurvePurpose::Subdomain { material },
        )
        .expect("the disc");
    let mut document = editor.document;
    document.model.accepted = document.model.draft.clone();
    (document, material)
}

/// Advances the fixture a phase; answers a run's verdict once it has one.
fn advance(
    driver: &mut Driver,
    state: &mut Playground,
    request: &mut CanonicalGpuRequest,
    display: &CanonicalGpuDisplay,
) -> Option<Result<Run, String>> {
    let (actions, endpoint) = driver.fixture.script();
    // The step the run holds at before the `next` action, or the endpoint.
    let stop = |next: usize| actions.get(next).map_or(endpoint, |(step, _)| *step);
    match std::mem::replace(&mut driver.phase, Phase::Done) {
        Phase::Open => {
            if !state.startup_done {
                driver.phase = Phase::Open;
                return None;
            }
            // Nothing steps until the pulse is in: the run waits at step 0.
            state.e2e_steps.limit = Some(0);
            state.e2e_steps.batch = None;
            state.wave_running = true;
            let (document, material) = driver.fixture.document();
            driver.material = material;
            if let Err(error) = state.set_document(document, false, true) {
                return Some(Err(format!("the document did not open: {error}")));
            }
            driver.phase = Phase::Install;
            None
        }
        Phase::Install => {
            // This run's generation: a document opened again is the same
            // scene as the last run's, but not the same revision.
            let installed = state.runtime.active().cloned().filter(|active| {
                active.bundle.token.document_revision == state.editor.revision
                    && *active.bundle.authored == state.editor.document.model.accepted
                    && state.coordinator.uploading.is_none()
                    && request.generation() != 0
                    && display.generation == request.generation()
            });
            let Some(active) = installed else {
                driver.phase = Phase::Install;
                return None;
            };
            let temporal = active.canonical_temporal_operator.is_some();
            if temporal != driver.material.is_some() {
                return Some(Err(format!(
                    "the document prepared a {} generation",
                    if temporal { "time-varying" } else { "fixed" }
                )));
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
            let reference = pulse_increment(
                &active,
                position,
                region,
                f64::from(state.pulse_amplitude),
                f64::from(state.pulse_width),
            )
            .and_then(|increment| {
                let mut reference = State::zero(&active, time_step)?;
                reference.pulse(&active, &increment)?;
                Ok(reference)
            });
            let reference = match reference {
                Ok(reference) => reference,
                Err(error) => return Some(Err(format!("the reference did not start: {error}"))),
            };
            println!(
                "e2e {:?}: {} DOFs, step {time_step:.4e}, pulse at ({:.3}, {:.3})",
                driver.fixture,
                active.canonical_operator.degrees_of_freedom(),
                position.x,
                position.y
            );
            driver.phase = Phase::Pulse {
                reference: Reference {
                    active,
                    state: reference,
                    control: None,
                    steps: 0,
                },
                processed: request.stats().processed_event(),
            };
            None
        }
        Phase::Pulse {
            reference,
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
            if let Err(failure) = taken(request, "the pulse", 0) {
                return Some(Err(failure));
            }
            state.e2e_steps.limit = Some(stop(0));
            state.e2e_steps.batch = driver.batches.front().copied().flatten();
            driver.phase = Phase::Run { reference, next: 0 };
            None
        }
        Phase::Run {
            mut reference,
            next,
        } => {
            let target = stop(next);
            let completed = request.stats().completed_steps();
            if completed < target {
                driver.phase = Phase::Run { reference, next };
                return None;
            }
            if completed > target {
                return Some(Err(format!(
                    "the run went past its stop at {target}, to {completed}"
                )));
            }
            if let Err(error) = reference.advance_to(target) {
                return Some(Err(format!("the reference did not step: {error}")));
            }
            let Some(&(_, action)) = actions.get(next) else {
                let seen = display.full_readbacks;
                request.request_full_state_readback();
                driver.phase = Phase::Snapshot { reference, seen };
                return None;
            };
            let processed = request.stats().processed_event();
            match action {
                Action::Switch => state.request_material_switch(),
            }
            driver.phase = Phase::Act {
                reference,
                next,
                processed,
            };
            None
        }
        Phase::Act {
            mut reference,
            next,
            processed,
        } => {
            if request.stats().processed_event() == processed
                || !state.coordinator.pending_switches.is_empty()
            {
                driver.phase = Phase::Act {
                    reference,
                    next,
                    processed,
                };
                return None;
            }
            let (step, action) = actions[next];
            if let Err(failure) = taken(request, &format!("the {action:?}"), step) {
                return Some(Err(failure));
            }
            if reference.control.is_none() {
                reference.control = Some(reference.state.clone());
            }
            match action {
                Action::Switch => {
                    let Some(material) = driver.material else {
                        return Some(Err("a Switch with no material to throw".into()));
                    };
                    // What the app sent: the direction it records, and the
                    // material's authored ramp.
                    let Some(&target) = state.coordinator.switch_targets.get(&material) else {
                        return Some(Err("the app recorded no Switch it sent".into()));
                    };
                    let Some(ramp) = state
                        .editor
                        .document
                        .model
                        .accepted
                        .materials
                        .iter()
                        .find(|found| found.id == material)
                        .map(|found| found.switch_ramp)
                    else {
                        return Some(Err("the switched material is gone".into()));
                    };
                    let State::Temporal(temporal) = &mut reference.state else {
                        return Some(Err("a Switch on a fixed generation".into()));
                    };
                    let time = temporal.time();
                    if let Err(error) = temporal
                        .runtime_mut()
                        .begin_switch(material, target, time, ramp)
                    {
                        return Some(Err(format!("the reference did not switch: {error}")));
                    }
                }
            }
            state.e2e_steps.limit = Some(stop(next + 1));
            driver.phase = Phase::Run {
                reference,
                next: next + 1,
            };
            None
        }
        Phase::Snapshot { reference, seen } => {
            // Only a snapshot of this generation, stamped with the endpoint
            // inside the state it copied, is the endpoint's.
            let arrived = display.full_readbacks > seen
                && display.generation == request.generation()
                && display.full_snapshot_completed_steps() == endpoint;
            if !arrived {
                request.request_full_state_readback();
                driver.phase = Phase::Snapshot { reference, seen };
                return None;
            }
            Some(
                compare(&reference, display, driver.fixture.tolerance(), endpoint).map(|summary| {
                    Run {
                        summary,
                        hash: hash(&display.accepted_storage_bits()),
                    }
                }),
            )
        }
        Phase::Done => None,
    }
}

/// Whether the device took the event just processed cleanly, at `step`.
fn taken(request: &CanonicalGpuRequest, what: &str, step: u64) -> Result<(), String> {
    let rejection = request.stats().event_rejection();
    if rejection != 0 {
        return Err(format!(
            "the device refused {what} (failure code {rejection})"
        ));
    }
    let completed = request.stats().completed_steps();
    if completed != step {
        return Err(format!("{what} landed at step {completed}, not {step}"));
    }
    Ok(())
}

/// The endpoint's snapshot against the reference: the relative L2 error of Q,
/// of b, and of r where the medium keeps one, each within `tolerance`; and,
/// where the fixture acted, the device far from the run without its actions.
fn compare(
    reference: &Reference,
    display: &CanonicalGpuDisplay,
    tolerance: f64,
    endpoint: u64,
) -> Result<String, String> {
    let expected = &reference.state;
    let integrated = display.integrated_field();
    if display.primary_flux.len() != expected.primary().len()
        || display.complementary_flux.len() != expected.complementary().len()
        || integrated.len() != expected.integrated().len()
    {
        return Err(format!(
            "the snapshot holds {}, {} and {} values where the generation has {}, {} and {}",
            display.primary_flux.len(),
            display.complementary_flux.len(),
            integrated.len(),
            expected.primary().len(),
            expected.complementary().len(),
            expected.integrated().len()
        ));
    }
    let primary = |state: &State| {
        relative_l2(
            display.primary_flux.iter().map(|value| f64::from(*value)),
            state.primary().iter().copied(),
        )
    };
    let q = primary(expected);
    let b = relative_l2(
        display
            .complementary_flux
            .iter()
            .flat_map(|value| value.iter().map(|lane| f64::from(*lane))),
        expected
            .complementary()
            .iter()
            .flat_map(|value| [value.x, value.y]),
    );
    let r = if integrated.is_empty() {
        0.0
    } else {
        relative_l2(
            integrated.iter().map(|value| f64::from(*value)),
            expected.integrated().iter().copied(),
        )
    };
    let mut summary = format!("Q {q:.3e}, b {b:.3e}, r {r:.3e} after {endpoint} steps");
    if !within(worse(worse(q, b), r), tolerance) {
        return Err(format!("{summary}, beyond {tolerance:.0e}"));
    }
    if let Some(control) = &reference.control {
        // An action the device ignored would leave it where the control is;
        // a hundred tolerances away, it did not.
        let apart = primary(control);
        summary.push_str(&format!(
            "; the run without its actions stands {apart:.3e} off"
        ));
        if !apart.is_finite() || apart <= 100.0 * tolerance {
            return Err(format!("{summary}, too near to tell the action took"));
        }
    }
    Ok(summary)
}

/// `‖actual − expected‖ / ‖expected‖`, a NaN on either side carried through.
fn relative_l2(actual: impl Iterator<Item = f64>, expected: impl Iterator<Item = f64>) -> f64 {
    let (difference, scale) = actual.zip(expected).fold((0.0, 0.0), |sum, pair| {
        (sum.0 + (pair.0 - pair.1).powi(2), sum.1 + pair.1 * pair.1)
    });
    (difference / scale).sqrt()
}

/// FNV-1a over the accepted state's bits.
fn hash(words: &[u32]) -> u64 {
    words
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}
