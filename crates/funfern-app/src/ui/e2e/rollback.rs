//! RT-05: the device refuses, in the app, and a retry lands.
//!
//! `rollback` runs the switchable cavity driven by its point source and, at
//! each of its stops, arms a fault on the running generation, as the device
//! examples do (`inject_failure`), then fires a pulse, the Switch or a move
//! of the source through the app's own handlers. The device refuses the
//! event: the stored state, the Switch bank, the clock and the event serials
//! must be the same bits after as before, and the app must say so in its
//! diagnostics and show what the device runs. The fault then goes, the same
//! act again must land, and the reference takes it. The run holds at each
//! stop: the fault refuses every step after the event too.
//!
//! `rollback-handoff` runs the struck cavity and edits its medium with the
//! next candidate made to fail its validation on the device
//! ([`E2eSteps::fail_handoff`]): once with the run held, the stored state
//! then the same bits, and once with the run going on, the running
//! generation then standing with a reference that never saw the edit. Each
//! time Remesh, the app's way to ask again for a revision it could not take,
//! must hand the edit off at the step held, and the reference hands off with
//! it.
//!
//! Natively `FUNFERN_E2E_SEED` draws the stops, the order of the acts and the
//! state word the fault is armed at.

use super::*;
use crate::canonical_gpu::CANONICAL_FAILURE_NON_FINITE;
use crate::topology_runtime::TopologyToken;
use bevy::render::storage::ShaderBuffer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RollbackFixture {
    /// Live events refused and fired again.
    Events,
    /// Handoffs refused and asked for again.
    Handoff,
}

impl RollbackFixture {
    pub(super) fn named(name: &str) -> Option<Self> {
        match name {
            "rollback" => Some(RollbackFixture::Events),
            "rollback-handoff" => Some(RollbackFixture::Handoff),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            RollbackFixture::Events => "rollback",
            RollbackFixture::Handoff => "rollback-handoff",
        }
    }

    /// The acts in order, each with the steps after the last stop it is
    /// taken at, the steps from the last stop to the endpoint, and the draw
    /// the fault's state word is taken from: [`EVENT_ACTS`] and
    /// [`HANDOFF_ACTS`], or drawn from `seed`.
    fn acts(self, seed: Option<u64>) -> (VecDeque<(u64, Act)>, u64, u64) {
        let Some(seed) = seed else {
            let acts = match self {
                RollbackFixture::Events => EVENT_ACTS.as_slice(),
                RollbackFixture::Handoff => HANDOFF_ACTS.as_slice(),
            };
            return (acts.iter().copied().collect(), self.end(), 0);
        };
        let mut state = seed;
        let mut next = |bound: u64| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) % bound
        };
        let mut kinds = match self {
            RollbackFixture::Events => vec![Act::Pulse, Act::Switch, Act::Source],
            RollbackFixture::Handoff => vec![Act::Held, Act::Running],
        };
        for index in (1..kinds.len()).rev() {
            kinds.swap(index, next(index as u64 + 1) as usize);
        }
        let acts = kinds
            .into_iter()
            .map(|act| match self {
                RollbackFixture::Events => (16 + next(81), act),
                RollbackFixture::Handoff => (32 + next(97), act),
            })
            .collect();
        (acts, self.end(), next(1 << 31))
    }

    /// The steps from the last stop to the endpoint.
    fn end(self) -> u64 {
        match self {
            RollbackFixture::Events => 64,
            RollbackFixture::Handoff => 128,
        }
    }

    /// The document, and the material its Switch throws.
    fn document(self) -> (TopologyDocument, Option<MaterialId>) {
        match self {
            RollbackFixture::Events => {
                let (mut document, material) = switch_document();
                let domain = document.model.accepted.geometry.domain;
                document.model.source = PointSource {
                    enabled: true,
                    position: Point2::new(
                        domain.min_x + 0.25 * domain.width(),
                        domain.min_y + 0.75 * domain.height(),
                    ),
                    signal: SOURCE_DRIVE,
                    ..document.model.source
                };
                (document, Some(material))
            }
            RollbackFixture::Handoff => (cavity_document(), None),
        }
    }

    fn mesh(self) -> &'static str {
        match self {
            RollbackFixture::Events => DISC_MESH,
            RollbackFixture::Handoff => CAVITY_MESH,
        }
    }

    /// The fixture of the same scene whose tolerance the endpoint is read by.
    fn compared_as(self) -> Fixture {
        match self {
            RollbackFixture::Events => Fixture::Switch,
            RollbackFixture::Handoff => Fixture::Handoff,
        }
    }
}

/// What a stop has the device refuse, and then fires again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    /// A pulse placed.
    Pulse,
    /// The Switch pressed.
    Switch,
    /// The point source moved by [`SOURCE_MOVE`]: a patch of its weights.
    Source,
    /// The medium's density raised, its handoff refused with the run held.
    Held,
    /// The medium's density raised and the run let go on, its handoff
    /// refused as it runs; the run holds again [`PAST_REFUSAL`] steps after
    /// the refusal for the retry.
    Running,
}

impl Act {
    fn what(self) -> &'static str {
        match self {
            Act::Pulse => "pulse",
            Act::Switch => "Switch",
            Act::Source => "source's move",
            Act::Held => "edit's handoff with the run held",
            Act::Running => "edit's handoff with the run going on",
        }
    }

    /// What the app's diagnostics say of the refusal.
    fn said(self) -> &'static str {
        match self {
            Act::Pulse | Act::Switch => "was rejected without changing the accepted state",
            Act::Source => "Canonical source update was rejected",
            Act::Held | Act::Running => "Canonical GPU handoff rejected",
        }
    }

    /// Whether the fault is armed on the running generation, as against on
    /// the edit's candidate.
    fn on_the_running(self) -> bool {
        matches!(self, Act::Pulse | Act::Switch | Act::Source)
    }
}

/// `rollback`'s acts: a pulse at step 64, the Switch at 128 and the source
/// moved at 192, the endpoint at 256.
const EVENT_ACTS: [(u64, Act); 3] = [(64, Act::Pulse), (64, Act::Switch), (64, Act::Source)];

/// `rollback-handoff`'s: the edit held at step 128, and the next at 192 with
/// the run going on, retried [`PAST_REFUSAL`] steps after its refusal and
/// run 128 further.
const HANDOFF_ACTS: [(u64, Act); 2] = [(128, Act::Held), (64, Act::Running)];

/// `rollback`'s source: a harmonic of amplitude 1 at 0.8 Hz from rest, at a
/// quarter of the cavity's width and three quarters of its height, away from
/// the disc.
const SOURCE_DRIVE: TimeSignal = TimeSignal::harmonic(0.0, 1.0, 0.8, 0.0);

/// How far each try moves the source, along x.
const SOURCE_MOVE: f64 = 0.05;

/// Where the refused pulse strikes, as fractions of the domain.
const PULSE_AT: (f64, f64) = (0.7, 0.65);

/// How much each edit raises the medium's density.
const DENSITY_STEP: f64 = 0.5;

/// How far a running edit's run may go before the device's refusal of its
/// handoff is in: the event fixtures' edit arrives within 224 steps on
/// lavapipe.
const REFUSAL_CEILING: u64 = 224;

/// Steps the run goes on past a running refusal, past those already asked
/// for, before it holds for the retry.
const PAST_REFUSAL: u64 = 32;

#[derive(Resource)]
pub(super) struct RollbackDriver {
    fixture: RollbackFixture,
    stage: Stage,
    /// The acts still to come, each with the steps after the last stop it
    /// is taken at, and the steps from the last stop to the endpoint.
    acts: VecDeque<(u64, Act)>,
    end: u64,
    /// The act under way, from its firing until its retry has landed.
    act: Option<Act>,
    /// The stop the run holds at, counted from `base`, and whether it is
    /// the endpoint.
    stop: u64,
    ending: bool,
    /// The draw the fault's state word is taken from.
    word: u64,
    /// The edits made to the medium so far.
    edits: u32,
    /// The device's completed-step count when the run's generation was
    /// installed.
    base: u64,
    /// The material the Switch throws.
    material: Option<MaterialId>,
    /// What each act came to, for the summary.
    refused: Vec<String>,
    deadline: Instant,
}

enum Stage {
    /// The app is starting; the document opens once it has.
    Open,
    /// The document's generation is being prepared and installed.
    Install,
    /// The run's own pulse waits for the device to take it.
    Pulse {
        reference: Reference,
        processed: u32,
    },
    /// The run steps to its stop.
    Run {
        reference: Reference,
    },
    /// Held at the stop, the run waits for any stationary flux a handoff
    /// left to be taken away, then for a full snapshot of the stop, `seen`
    /// once asked for.
    Hold {
        reference: Reference,
        seen: Option<u64>,
    },
    /// The act was fired with the fault armed, and the device's refusal is
    /// waited for: the serial last processed, the revision the act made and
    /// the generation running before it say when it is in. `before` is the
    /// stop's stored state where the run holds through the refusal.
    Refusing {
        reference: Reference,
        before: Option<Held>,
        processed: u32,
        revision: u64,
        generation: u64,
    },
    /// Refused with the run held: a full snapshot after the refusal, `seen`
    /// when asked for.
    Refused {
        reference: Reference,
        before: Held,
        seen: u64,
    },
    /// The act again, its fault gone, waits to land: `fired` is what the
    /// reference takes for a pulse, and `carried` the retried edit's maps.
    Retrying {
        reference: Reference,
        processed: u32,
        generation: u64,
        fired: Option<Fired>,
        carried: Option<Carried>,
    },
    Done,
}

impl Stage {
    fn name(&self) -> &'static str {
        match self {
            Stage::Open => "opening the document",
            Stage::Install => "installing its generation",
            Stage::Pulse { .. } => "waiting for the device to take the pulse",
            Stage::Run { .. } => "running to the next stop",
            Stage::Hold { .. } => "waiting for the stop's snapshot",
            Stage::Refusing { .. } => "waiting for the device's refusal",
            Stage::Refused { .. } => "waiting for the snapshot after the refusal",
            Stage::Retrying { .. } => "waiting for the retry to land",
            Stage::Done => "done",
        }
    }
}

/// The stop's state as the device stores it, and what runs it.
struct Held {
    state: Vec<u32>,
    runtime: Vec<u32>,
    serials: [u32; 4],
    /// The last event the clock took, and its time's bits.
    clock: (u32, u64),
    generation: u64,
    active: TopologyToken,
}

impl Held {
    fn read(
        display: &CanonicalGpuDisplay,
        request: &CanonicalGpuRequest,
        state: &Playground,
    ) -> Result<Self, String> {
        let clock = display
            .clock
            .ok_or_else(|| "the display has no clock".to_owned())?;
        let active = state
            .runtime
            .active()
            .map(|active| active.bundle.token)
            .ok_or_else(|| "no topology is active".to_owned())?;
        Ok(Held {
            state: display.accepted_storage_bits(),
            runtime: display.material_runtime_bits(),
            serials: display.runtime_serials,
            clock: (clock.event_serial, clock.absolute_seconds.to_bits()),
            generation: request.generation(),
            active,
        })
    }

    /// What moved from `self` to `now`.
    fn moved(&self, now: &Held) -> Vec<&'static str> {
        [
            (self.state != now.state, "the state"),
            (self.runtime != now.runtime, "the Switch bank"),
            (self.serials != now.serials, "the runtime serials"),
            (self.clock != now.clock, "the clock"),
            (self.generation != now.generation, "the generation"),
            (self.active != now.active, "the active topology"),
        ]
        .into_iter()
        .filter_map(|(moved, what)| moved.then_some(what))
        .collect()
    }
}

/// Adds the driver of the rollback fixture named.
pub(super) fn add(app: &mut App, fixture: RollbackFixture, seconds: Option<u64>) {
    #[cfg(not(target_arch = "wasm32"))]
    let seed = std::env::var("FUNFERN_E2E_SEED")
        .ok()
        .and_then(|seed| seed.parse::<u64>().ok());
    #[cfg(target_arch = "wasm32")]
    let seed = None;
    let (acts, end, word) = fixture.acts(seed);
    mark("running", "");
    report(&format!("e2e build {}", crate::BUILD_ID));
    if let Some(seed) = seed {
        report(&format!(
            "e2e {}: seed {seed}: {acts:?}, the endpoint {end} after, word draw {word}",
            fixture.name()
        ));
    }
    app.insert_resource(RollbackDriver {
        fixture,
        stage: Stage::Open,
        acts,
        end,
        act: None,
        stop: 0,
        ending: false,
        word,
        edits: 0,
        base: 0,
        material: None,
        refused: Vec::new(),
        deadline: Instant::now() + std::time::Duration::from_secs(seconds.unwrap_or(120)),
    })
    .add_systems(EguiPrimaryContextPass, drive_rollback.before(frame));
}

#[allow(clippy::too_many_arguments)]
fn drive_rollback(
    mut driver: ResMut<RollbackDriver>,
    mut state: ResMut<Playground>,
    mut request: ResMut<CanonicalGpuRequest>,
    display: Res<CanonicalGpuDisplay>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    if matches!(driver.stage, Stage::Done) {
        return;
    }
    let name = driver.fixture.name();
    let run = match advance(
        &mut driver,
        &mut state,
        &mut request,
        &display,
        &mut assets,
        &mut commands,
    ) {
        Some(run) => run,
        None if Instant::now() >= driver.deadline => Err(format!(
            "did not finish in time, at {}",
            driver.stage.name()
        )),
        None => return,
    };
    driver.stage = Stage::Done;
    match run {
        Ok(run) => {
            report(&format!(
                "e2e {name}: {}, endpoint hash {:016x}",
                run.summary, run.hash
            ));
            mark("pass", &run.summary);
            exit.write(AppExit::Success);
        }
        Err(failure) => {
            report(&format!("e2e {name} failed: {failure}"));
            mark("fail", &failure);
            exit.write(AppExit::error());
        }
    }
}

fn advance(
    driver: &mut RollbackDriver,
    state: &mut Playground,
    request: &mut CanonicalGpuRequest,
    display: &CanonicalGpuDisplay,
    assets: &mut Assets<ShaderBuffer>,
    commands: &mut Commands,
) -> Option<Result<Run, String>> {
    match std::mem::replace(&mut driver.stage, Stage::Done) {
        Stage::Open => {
            if !state.startup_done {
                driver.stage = Stage::Open;
                return None;
            }
            // Nothing steps until the run starts: it waits at step 0.
            state.e2e_steps.limit = Some(0);
            state.e2e_steps.batch = None;
            state.wave_running = true;
            *FRESH_CLOCK.lock().unwrap() = None;
            let (document, material) = driver.fixture.document();
            driver.material = material;
            if let Err(error) = state.set_document(document, false, true) {
                return Some(Err(format!("the document did not open: {error}")));
            }
            driver.stage = Stage::Install;
            None
        }
        Stage::Install => {
            let installed = state.runtime.active().cloned().filter(|active| {
                active.bundle.token.document_revision == state.editor.revision
                    && *active.bundle.authored == state.editor.document.model.accepted
                    && state.coordinator.uploading.is_none()
                    && request.generation() != 0
                    && display.generation == request.generation()
            });
            let Some(active) = installed else {
                driver.stage = Stage::Install;
                return None;
            };
            if let Err(failure) = same_mesh(&active.mesh, driver.fixture.mesh()) {
                return Some(Err(failure));
            }
            let temporal = active.canonical_temporal_operator.is_some();
            if temporal != (driver.fixture == RollbackFixture::Events) {
                return Some(Err(format!(
                    "the document prepared a {} generation",
                    if temporal { "time-varying" } else { "fixed" }
                )));
            }
            let time_step = state.coordinator.uploaded_time_step;
            let zero = match State::zero(&active, time_step) {
                Ok(zero) => zero,
                Err(error) => return Some(Err(format!("the reference did not start: {error}"))),
            };
            report(&format!(
                "e2e {}: {} DOFs, step {time_step:.4e}, mesh {}",
                driver.fixture.name(),
                active.canonical_operator.degrees_of_freedom(),
                mesh_hashes(&active.mesh)
            ));
            driver.base = request.stats().completed_steps();
            let mut reference = Reference::starting(active, zero, request.generation());
            match driver.fixture {
                // Driven by its source from rest: the run starts at once.
                RollbackFixture::Events => {
                    next_stop(driver, state);
                    driver.stage = Stage::Run { reference };
                }
                // Struck off its middle at step 0.
                RollbackFixture::Handoff => {
                    let fired = place_pulse(state, (0.37, 0.58)).and_then(|fired| {
                        reference.take(fired)?;
                        Ok(fired)
                    });
                    if let Err(failure) = fired {
                        return Some(Err(failure));
                    }
                    driver.stage = Stage::Pulse {
                        reference,
                        processed: request.stats().processed_event(),
                    };
                }
            }
            None
        }
        Stage::Pulse {
            reference,
            processed,
        } => {
            let untouched = request.stats().processed_event() == processed;
            let queued =
                !state.coordinator.pending_pulses.is_empty() || request.live_event_pending();
            if untouched && !queued {
                return Some(Err(format!("the app dropped the pulse: {}", state.message)));
            }
            if untouched || queued {
                driver.stage = Stage::Pulse {
                    reference,
                    processed,
                };
                return None;
            }
            if let Err(failure) = taken(request, "the pulse", driver.base) {
                return Some(Err(failure));
            }
            next_stop(driver, state);
            driver.stage = Stage::Run { reference };
            None
        }
        Stage::Run { mut reference } => {
            let completed = request
                .stats()
                .completed_steps()
                .saturating_sub(driver.base);
            let code = request.stats().failure();
            if code != 0 {
                return Some(Err(format!(
                    "the device failed after step {completed} (failure code {code})"
                )));
            }
            if completed < driver.stop {
                driver.stage = Stage::Run { reference };
                return None;
            }
            if completed > driver.stop {
                return Some(Err(format!(
                    "the run went past its stop at {}, to {completed}",
                    driver.stop
                )));
            }
            if let Err(error) = reference.advance_to(driver.stop) {
                return Some(Err(format!("the reference did not step: {error}")));
            }
            driver.stage = Stage::Hold {
                reference,
                seen: None,
            };
            None
        }
        Stage::Hold {
            mut reference,
            seen,
        } => {
            let generation = request.generation();
            let unsettled = state.flux_cleanup.as_ref().is_some_and(|cleanup| {
                cleanup.generation == generation
                    && !matches!(
                        cleanup.stage,
                        super::super::flux_cleanup::FluxCleanupStage::Landed(_)
                    )
            });
            let Some(seen) = seen.filter(|_| !unsettled) else {
                let seen = (!unsettled).then(|| {
                    request.request_full_state_readback();
                    display.full_readbacks
                });
                driver.stage = Stage::Hold { reference, seen };
                return None;
            };
            let arrived = display.full_readbacks > seen
                && display.generation == generation
                && display.full_snapshot_completed_steps() == driver.base + driver.stop;
            if !arrived {
                request.request_full_state_readback();
                driver.stage = Stage::Hold {
                    reference,
                    seen: Some(seen),
                };
                return None;
            }
            if let Err(error) = reference.follow_flux_cleanup(display) {
                return Some(Err(error));
            }
            if driver.ending {
                let compared = compare(
                    &reference,
                    display,
                    driver.fixture.compared_as(),
                    driver.stop,
                );
                return Some(compared.map(|summary| Run {
                    summary: format!("{summary}; {}", driver.refused.join("; ")),
                    hash: hash(&display.accepted_storage_bits()),
                }));
            }
            if driver.act == Some(Act::Running) {
                if let Err(failure) = ran_on(driver, state, &reference, display) {
                    return Some(Err(failure));
                }
                return retry(driver, state, request, reference);
            }
            let Some((_, act)) = driver.acts.pop_front() else {
                return Some(Err(format!("no act at the stop {}", driver.stop)));
            };
            driver.act = Some(act);
            // The run without its acts, for the endpoint to stand far from.
            if reference.control.is_none() {
                reference.control = Some((reference.active.clone(), reference.state.clone()));
            }
            let before = match Held::read(display, request, state) {
                Ok(before) => before,
                Err(failure) => return Some(Err(failure)),
            };
            let processed = request.stats().processed_event();
            let word = (driver.word
                % (reference.state.primary().len() + reference.state.complementary().len()) as u64)
                as u32;
            if act.on_the_running() {
                if let Err(error) =
                    request.inject_failure(assets, commands, CANONICAL_FAILURE_NON_FINITE, word)
                {
                    return Some(Err(format!("the fault was not armed: {error}")));
                }
            } else {
                state.e2e_steps.fail_handoff = Some(word);
            }
            if let Err(failure) = fire(driver, state, act) {
                return Some(Err(failure));
            }
            if act == Act::Running {
                state.e2e_steps.limit = Some(driver.base + driver.stop + REFUSAL_CEILING);
            }
            driver.stage = Stage::Refusing {
                reference,
                before: (act != Act::Running).then_some(before),
                processed,
                revision: state.editor.revision,
                generation: request.generation(),
            };
            None
        }
        Stage::Refusing {
            reference,
            before,
            processed,
            revision,
            generation,
        } => {
            let act = driver.act.expect("an act under way");
            let refused = match refusal(state, request, act, processed, revision, generation) {
                Ok(refused) => refused,
                Err(failure) => return Some(Err(failure)),
            };
            let completed = request
                .stats()
                .completed_steps()
                .saturating_sub(driver.base);
            let Some(code) = refused else {
                if act == Act::Running && completed >= driver.stop + REFUSAL_CEILING {
                    return Some(Err(format!(
                        "the run reached its ceiling at step {completed} before the device \
                         refused the edit's handoff"
                    )));
                }
                driver.stage = Stage::Refusing {
                    reference,
                    before,
                    processed,
                    revision,
                    generation,
                };
                return None;
            };
            if code != CANONICAL_FAILURE_NON_FINITE {
                return Some(Err(format!(
                    "the device took the {} it was to refuse (failure code {code})",
                    act.what()
                )));
            }
            let Some(before) = before else {
                // Refused as it ran: the run goes on a little, past what it
                // had asked for, and holds for the retry.
                if completed >= driver.stop + REFUSAL_CEILING {
                    return Some(Err(format!(
                        "the run was held at its ceiling, step {completed}, when the device \
                         refused the edit's handoff: the refusal never met the run going on"
                    )));
                }
                driver.refused.push(format!(
                    "the {} made at step {} refused by step {completed}",
                    act.what(),
                    driver.stop
                ));
                let asked = request.requested_steps().saturating_sub(driver.base);
                driver.stop = completed.max(asked) + PAST_REFUSAL;
                state.e2e_steps.limit = Some(driver.base + driver.stop);
                driver.stage = Stage::Run { reference };
                return None;
            };
            request.request_full_state_readback();
            driver.stage = Stage::Refused {
                reference,
                before,
                seen: display.full_readbacks,
            };
            None
        }
        Stage::Refused {
            reference,
            before,
            seen,
        } => {
            let arrived = display.full_readbacks > seen
                && display.generation == request.generation()
                && display.full_snapshot_completed_steps() == driver.base + driver.stop;
            if !arrived {
                request.request_full_state_readback();
                driver.stage = Stage::Refused {
                    reference,
                    before,
                    seen,
                };
                return None;
            }
            let act = driver.act.expect("an act under way");
            let after = match Held::read(display, request, state) {
                Ok(after) => after,
                Err(failure) => return Some(Err(failure)),
            };
            let moved = before.moved(&after);
            if !moved.is_empty() {
                return Some(Err(format!(
                    "the refused {} moved {}",
                    act.what(),
                    moved.join(", ")
                )));
            }
            let completed = request
                .stats()
                .completed_steps()
                .saturating_sub(driver.base);
            if completed != driver.stop {
                return Some(Err(format!(
                    "the refused {} left the run at step {completed}, not {}",
                    act.what(),
                    driver.stop
                )));
            }
            if let Err(failure) = told(state, act) {
                return Some(Err(failure));
            }
            if act == Act::Switch
                && let Err(failure) = shows_the_switch(driver, state)
            {
                return Some(Err(failure));
            }
            // The fault goes, as a device's would with the next state.
            if act.on_the_running()
                && let Err(error) = request.clear_failure(assets, commands)
            {
                return Some(Err(format!("the fault was not cleared: {error}")));
            }
            driver.refused.push(format!(
                "the {} refused at step {}, the stored state the same bits",
                act.what(),
                driver.stop
            ));
            retry(driver, state, request, reference)
        }
        Stage::Retrying {
            mut reference,
            processed,
            generation,
            fired,
            carried,
        } => {
            let act = driver.act.expect("an act under way");
            match act {
                Act::Pulse | Act::Switch => {
                    let sent = request.stats().processed_event() != processed
                        && state.coordinator.pending_pulses.is_empty()
                        && state.coordinator.pending_switches.is_empty()
                        && !request.live_event_pending();
                    if !sent {
                        driver.stage = Stage::Retrying {
                            reference,
                            processed,
                            generation,
                            fired,
                            carried,
                        };
                        return None;
                    }
                    let what = format!("the {} fired again", act.what());
                    if let Err(failure) = taken(request, &what, driver.base + driver.stop) {
                        return Some(Err(failure));
                    }
                    let fired = match fired {
                        Some(fired) => Ok(fired),
                        None => sent_switch(driver, state),
                    };
                    if let Err(failure) = fired.and_then(|fired| reference.take(fired)) {
                        return Some(Err(failure));
                    }
                }
                Act::Source => {
                    let revision = state.editor.revision;
                    if let Some(error) = state
                        .runtime
                        .last_error()
                        .filter(|error| error.token.document_revision == revision)
                    {
                        return Some(Err(format!(
                            "the source moved again was not taken: {error}"
                        )));
                    }
                    let committed = state.runtime.active().cloned().filter(|active| {
                        active.bundle.token.document_revision == revision
                            && state.coordinator.source_commit.is_none()
                    });
                    let Some(active) = committed else {
                        driver.stage = Stage::Retrying {
                            reference,
                            processed,
                            generation,
                            fired,
                            carried,
                        };
                        return None;
                    };
                    if request.generation() != generation
                        || request.stats().processed_event() == processed
                    {
                        return Some(Err(
                            "the source moved again was not taken as a patch of the running \
                             generation"
                                .into(),
                        ));
                    }
                    if let Err(failure) =
                        taken(request, "the source moved again", driver.base + driver.stop)
                    {
                        return Some(Err(failure));
                    }
                    reference.active = active;
                }
                Act::Held | Act::Running => {
                    let carried = carried.or_else(|| carried_now(state));
                    let Some(target) = published(state, request, display, generation) else {
                        driver.stage = Stage::Retrying {
                            reference,
                            processed,
                            generation,
                            fired,
                            carried,
                        };
                        return None;
                    };
                    if request.handoff_outcome() != CanonicalGpuHandoffOutcome::Accepted
                        || request.handed_over_at() != Some(driver.base + driver.stop)
                    {
                        return Some(Err(format!(
                            "the edit asked for again came by {:?} from step {:?}, not a handoff \
                             from the stop at {}",
                            request.handoff_outcome(),
                            request
                                .handed_over_at()
                                .map(|at| at.saturating_sub(driver.base)),
                            driver.stop
                        )));
                    }
                    let Some(carried) = carried else {
                        return Some(Err(
                            "the edit's candidate was never seen with its maps".into()
                        ));
                    };
                    match hand_off(
                        &reference,
                        &target,
                        &carried,
                        state.coordinator.uploaded_time_step,
                        display.runtime_serials,
                    ) {
                        Ok((handed, carry, target)) => {
                            reference.state = handed;
                            reference.carry = carry;
                            reference.active = target;
                            reference.generation = request.generation();
                        }
                        Err(error) => {
                            return Some(Err(format!("the reference was not handed off: {error}")));
                        }
                    }
                }
            }
            if let Some(last) = driver.refused.last_mut() {
                last.push_str(", then landed");
            }
            driver.act = None;
            next_stop(driver, state);
            driver.stage = Stage::Run { reference };
            None
        }
        Stage::Done => None,
    }
}

/// Moves the stop on to the next act's, or past the last to the endpoint.
fn next_stop(driver: &mut RollbackDriver, state: &mut Playground) {
    match driver.acts.front() {
        Some(&(after, _)) => driver.stop += after,
        None => {
            driver.stop += driver.end;
            driver.ending = true;
        }
    }
    state.e2e_steps.limit = Some(driver.base + driver.stop);
}

/// Places a pulse at `at`, as fractions of the domain, as a click with the
/// pulse tool does: what the reference takes for it, once the app has
/// queued it.
fn place_pulse(state: &mut Playground, at: (f64, f64)) -> Result<Fired, String> {
    let domain = state.editor.document.model.accepted.geometry.domain;
    let point = Point2::new(
        domain.min_x + at.0 * domain.width(),
        domain.min_y + at.1 * domain.height(),
    );
    let queued = state.coordinator.pending_pulses.len();
    state.place_pulse(point);
    let Some(&(position, region)) = state
        .coordinator
        .pending_pulses
        .back()
        .filter(|_| state.coordinator.pending_pulses.len() == queued + 1)
    else {
        return Err(format!("the pulse was not queued: {}", state.message));
    };
    Ok(Fired::Pulse {
        position,
        region,
        amplitude: f64::from(state.pulse_amplitude),
        width: f64::from(state.pulse_width),
    })
}

/// Fires `act` through the app's own handlers: what the reference takes
/// for a pulse.
fn fire(
    driver: &mut RollbackDriver,
    state: &mut Playground,
    act: Act,
) -> Result<Option<Fired>, String> {
    match act {
        Act::Pulse => place_pulse(state, PULSE_AT).map(Some),
        Act::Switch => {
            let queued = state.coordinator.pending_switches.len();
            state.request_material_switch();
            if state.coordinator.pending_switches.len() != queued + 1 {
                return Err(format!("the Switch was not queued: {}", state.message));
            }
            Ok(None)
        }
        Act::Source => {
            let mut source = state.editor.document.model.source;
            source.position.x += SOURCE_MOVE;
            state
                .editor
                .set_point_source(source)
                .map_err(|error| format!("the source did not move: {error}"))?;
            Ok(None)
        }
        Act::Held | Act::Running => {
            driver.edits += 1;
            let density = 1.0 + DENSITY_STEP * f64::from(driver.edits);
            state
                .editor
                .document
                .model
                .draft
                .material(DEFAULT_MATERIAL)
                .cloned()
                .ok_or_else(|| "the default medium is gone".to_owned())
                .and_then(|mut medium| {
                    medium.mass_density = ScalarField::constant(density);
                    state.editor.update_material(medium)
                })
                .map_err(|error| format!("the edit did not apply: {error}"))?;
            Ok(None)
        }
    }
}

/// Fires the act again with its fault gone, Remesh for an edit, the app's
/// way to ask again for a revision it could not take.
fn retry(
    driver: &mut RollbackDriver,
    state: &mut Playground,
    request: &CanonicalGpuRequest,
    reference: Reference,
) -> Option<Result<Run, String>> {
    let act = driver.act.expect("an act under way");
    let processed = request.stats().processed_event();
    let generation = request.generation();
    let fired = match act {
        Act::Held | Act::Running => {
            state.coordinator.remesh_requested = true;
            None
        }
        _ => match fire(driver, state, act) {
            Ok(fired) => fired,
            Err(failure) => return Some(Err(failure)),
        },
    };
    driver.stage = Stage::Retrying {
        reference,
        processed,
        generation,
        fired,
        carried: None,
    };
    None
}

/// The device's code for the act's refusal once it is in, None while it is
/// not: the event the act sent processed, or the edit's candidate turned
/// away, with nothing of it left in flight.
fn refusal(
    state: &Playground,
    request: &CanonicalGpuRequest,
    act: Act,
    processed: u32,
    revision: u64,
    generation: u64,
) -> Result<Option<u32>, String> {
    let failed = state
        .runtime
        .last_error()
        .filter(|error| error.token.document_revision == revision);
    match act {
        Act::Pulse | Act::Switch => {
            let sent = request.stats().processed_event() != processed
                && state.coordinator.pending_pulses.is_empty()
                && state.coordinator.pending_switches.is_empty()
                && !request.live_event_pending();
            Ok(sent.then(|| request.stats().event_rejection()))
        }
        Act::Source => {
            if state
                .runtime
                .active()
                .is_some_and(|active| active.bundle.token.document_revision == revision)
            {
                return Err("the app committed the source's move the device refused".into());
            }
            let Some(failed) = failed.filter(|_| state.coordinator.source_commit.is_none()) else {
                return Ok(None);
            };
            if request.stats().processed_event() == processed {
                return Err(format!(
                    "the app turned the source's move away without the device: {failed}"
                ));
            }
            Ok(Some(request.stats().event_rejection()))
        }
        Act::Held | Act::Running => {
            if request.generation() != generation {
                return Err("the refused edit's generation was installed".into());
            }
            let Some(failed) = failed.filter(|_| {
                state.coordinator.uploading.is_none()
                    && state.coordinator.gpu_upload_preparation.is_none()
            }) else {
                return Ok(None);
            };
            match request.handoff_outcome() {
                CanonicalGpuHandoffOutcome::Rejected(code) => Ok(Some(code)),
                outcome => Err(format!(
                    "the edit was turned away without the device's refusal ({outcome:?}): {failed}"
                )),
            }
        }
    }
}

/// Whether the app said the device refused the act: the diagnostics hold
/// its line, and the warning is lit.
fn told(state: &Playground, act: Act) -> Result<(), String> {
    let said = act.said();
    if state.events.iter().any(|entry| entry.text.contains(said)) && state.diagnostics_warning() {
        return Ok(());
    }
    Err(format!(
        "the app did not say the device refused the {}: its last word {:?}",
        act.what(),
        state.message
    ))
}

/// Whether the Switch the app shows heads where the device's does.
fn shows_the_switch(driver: &RollbackDriver, state: &Playground) -> Result<(), String> {
    let material = driver
        .material
        .ok_or_else(|| "a Switch with no material to throw".to_owned())?;
    let device = state
        .switch_states
        .iter()
        .find(|(id, _, _)| *id == material)
        .map(|(_, target, _)| *target >= 0.5);
    let shown = state
        .coordinator
        .switch_targets
        .get(&material)
        .copied()
        .or(device);
    let heading = |heading: Option<bool>| match heading {
        Some(true) => "for its alternate",
        Some(false) => "for its own law",
        None => "nowhere",
    };
    if device.is_none() || shown != device {
        return Err(format!(
            "after the refused Switch the app shows the material headed {} where the device \
             holds it headed {}",
            heading(shown),
            heading(device)
        ));
    }
    Ok(())
}

/// The Switch the app sent: the direction it records, and the material's
/// authored ramp.
fn sent_switch(driver: &RollbackDriver, state: &Playground) -> Result<Fired, String> {
    let material = driver
        .material
        .ok_or_else(|| "a Switch with no material to throw".to_owned())?;
    let target = *state
        .coordinator
        .switch_targets
        .get(&material)
        .ok_or_else(|| "the app recorded no Switch it sent".to_owned())?;
    let ramp = state
        .editor
        .document
        .model
        .accepted
        .materials
        .iter()
        .find(|found| found.id == material)
        .map(|found| found.switch_ramp)
        .ok_or_else(|| "the switched material is gone".to_owned())?;
    Ok(Fired::Switch {
        material,
        target,
        ramp,
    })
}

/// At the retry stop of an edit refused as the run went on: the running
/// generation, which the edit was to replace, must stand with the
/// reference, which never saw it, and the app must have said so.
fn ran_on(
    driver: &mut RollbackDriver,
    state: &Playground,
    reference: &Reference,
    display: &CanonicalGpuDisplay,
) -> Result<(), String> {
    told(state, Act::Running)?;
    let (q, b, r) = field_errors(reference, display, None)?;
    let worst = worse(worse(q, b), r);
    let tolerance = driver.fixture.compared_as().tolerance();
    if !within(worst, tolerance) {
        return Err(format!(
            "after the refused handoff the running generation stands Q {q:.3e}, b {b:.3e}, r \
             {r:.3e} from a reference that never saw the edit, beyond {tolerance:.0e}"
        ));
    }
    if let Some(last) = driver.refused.last_mut() {
        last.push_str(&format!(
            ", the running generation at step {} then Q {q:.1e} and b {b:.1e} from the run \
             without it",
            driver.stop
        ));
    }
    Ok(())
}
