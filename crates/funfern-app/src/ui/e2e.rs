//! End-to-end fixtures: the app as it ships, driven through its own handlers
//! on this machine's GPU and judged against the f64 reference stepped on the
//! generation the app accepted. Added by the `funfern-fixtures` binary
//! (`crate::Build::Fixtures`) and chosen by `FUNFERN_E2E=<fixture>`; the
//! process exits 0 when the fixture holds, and 1 when it does not or does not
//! finish in time. In the browser the fixture is `?e2e=<fixture>`, and the verdict is the page's: `data-funfern-e2e` on its
//! root element reads `pass` or `fail`, with `data-funfern-e2e-summary` beside
//! it, and the driver's lines go to the console.
//!
//! The driver acts as the user's handlers do - it opens a document, places a
//! pulse, presses Run and Switch, edits a material - and reads only what the
//! device accepted:
//! its accepted-step counter, the serial of the last event it processed, and
//! a full snapshot stamped with the step it holds. A fixture holds the run at
//! the steps where it acts, so each action lands at a step both sides know,
//! and the reference takes it there. What the driver adds to the app is where
//! a run stops and, for a fixture that asks, how many steps a running frame
//! asks for ([`E2eSteps`]): the app asks by the wall clock, and a comparison
//! needs an exact endpoint.

mod batches;
#[path = "../../examples/support/check.rs"]
mod check;
pub(crate) mod far_field;
mod observables;

use super::runtime::pulse_increment;
use super::*;
use crate::canonical_gpu::{
    CANONICAL_FAILURE_INVERSE_DOMAIN, CanonicalGpuHandoffOutcome, CanonicalGpuRuntimeTransfer,
    canonical_failure_description,
};
use crate::topology_editor::{
    ClosedCurvePurpose, OpenCurvePurpose, TopologyAttachment, TopologyDocument, TopologyEditor,
};
use crate::topology_runtime::PreparedCanonicalTransfer;
use bevy::render::{
    Render, RenderApp, RenderSystems,
    extract_resource::{ExtractResource, ExtractResourcePlugin},
    renderer::RenderDevice,
};
use bevy_egui::EguiPrimaryContextPass;
use check::{within, worse};
use funfern_core::{
    CanonicalHandoffMaps, CanonicalStationaryFluxJob, CanonicalTemporalWaveState,
    CanonicalWaveOperator, CanonicalWaveState, FaceAnchor, OpenCubicSpline, OuterBoundaryCondition,
    OuterBoundaryConditions, SpanBehavior,
};
use std::collections::{BTreeSet, VecDeque};

/// Where an end-to-end fixture holds the run: the steps a running frame asks
/// for stop at `limit`, counted on the running generation, and are `batch` a
/// frame where it names a number, in place of the frame's pacing. Under
/// `burst` one frame asks for every step to the stop at once, past the
/// frame's ceiling and its backpressure, as the device examples ask. `most`
/// is the most a frame has asked for since the fixture last cleared it.
#[derive(Default)]
pub(super) struct E2eSteps {
    pub(super) limit: Option<u64>,
    pub(super) batch: Option<u64>,
    pub(super) burst: bool,
    pub(super) most: u64,
}

/// Where a fixture starts a fresh generation's clock, if it says: the epoch
/// origin, and how many steps before the epoch's rebase. The app starts every
/// fresh generation at zero; `far-field-late` starts 36 hours in, and
/// `batches` at zero, with the origin as far before it as the epoch has run.
static FRESH_CLOCK: std::sync::Mutex<Option<(FreshOrigin, u32)>> = std::sync::Mutex::new(None);

#[derive(Clone, Copy)]
enum FreshOrigin {
    At(f64),
    StartingAtZero,
}

/// The clock a fresh generation starts at: the fixture's, or zero.
pub(crate) fn fresh_clock(
    time_step: f64,
) -> Result<crate::canonical_gpu::CanonicalGpuClock, String> {
    let clock = crate::canonical_gpu::CanonicalGpuClock::initial(time_step)
        .map_err(|error| format!("{error:?}"))?;
    let Some((origin, before)) = *FRESH_CLOCK.lock().unwrap() else {
        return Ok(clock);
    };
    let clock = crate::canonical_gpu::CanonicalGpuClock {
        step_in_epoch: rebase_step_limit(time_step) - before,
        ..clock
    };
    Ok(crate::canonical_gpu::CanonicalGpuClock {
        epoch_origin_seconds: match origin {
            FreshOrigin::At(origin) => origin,
            FreshOrigin::StartingAtZero => -clock.local_seconds(),
        },
        ..clock
    })
}

/// The steps an epoch holds before the device's encoder rebases it, as the
/// encoder counts them from the step in f32.
pub(crate) fn rebase_step_limit(time_step: f64) -> u32 {
    ((256.0 / f64::from(time_step as f32)).ceil() as u32).min(1 << 16)
}

/// Adds the driver of the fixture asked for, if one is.
pub(crate) fn add(app: &mut App) {
    let Some((name, seconds)) = requested() else {
        return;
    };
    if name == "batches" {
        batches::add(app, seconds);
        return;
    }
    let (fixture, batches) = match name.as_str() {
        "cavity" => (Fixture::Cavity, vec![None]),
        "cavity-batches" => (
            Fixture::CavityBatches,
            BATCHES.iter().copied().map(Some).collect(),
        ),
        "switch" => (Fixture::Switch, vec![None]),
        "handoff" => (Fixture::Handoff, vec![None]),
        "handoff-driven" => (Fixture::HandoffDriven, vec![None]),
        "handoff-driven-pins" => (Fixture::HandoffDrivenPins, vec![None]),
        "remesh" => (Fixture::Remesh, vec![None]),
        "neutral-remesh" => (Fixture::NeutralRemesh, vec![None]),
        "baffle-move" => (Fixture::BaffleMove, vec![None]),
        "separator-mid-run" => (Fixture::SeparatorMidRun, vec![None]),
        "observers" => (Fixture::Observers, vec![None, None]),
        "superposition" => (Fixture::Superposition, vec![None; SUPERPOSED.len()]),
        "similarity" => (Fixture::Similarity, vec![None; 2]),
        "handoff-memory-rebuild" => (Fixture::HandoffMemory(MemoryHandoff::Rebuild), vec![None]),
        "handoff-memory-remesh" => (Fixture::HandoffMemory(MemoryHandoff::Remesh), vec![None]),
        "handoff-memory-edit" => (Fixture::HandoffMemory(MemoryHandoff::Edit), vec![None]),
        "handoff-memory-timestep" => (Fixture::HandoffMemory(MemoryHandoff::Timestep), vec![None]),
        "handoff-memory-flux" => (Fixture::HandoffMemory(MemoryHandoff::Flux), vec![None]),
        "handoff-memory-running" => (Fixture::HandoffMemory(MemoryHandoff::Running), vec![None]),
        "handoff-memory-running-late" => (
            Fixture::HandoffMemory(MemoryHandoff::RunningLate),
            vec![None],
        ),
        "rejection" => (Fixture::Rejection, vec![None]),
        "device-loss" => (Fixture::DeviceLoss, vec![None]),
        "observables" => (Fixture::Observables, vec![None, None]),
        "far-field" => (Fixture::FarField, vec![None, None]),
        "far-field-late" => (Fixture::FarFieldLate, vec![None]),
        "events" => (Fixture::Events, vec![None]),
        "events-handoff" => (Fixture::EventsHandoff, vec![None]),
        _ => {
            let failure = format!("no fixture named {name:?}");
            report(&format!("e2e: {failure}"));
            mark("fail", &failure);
            #[cfg(not(target_arch = "wasm32"))]
            std::process::exit(2);
            #[cfg(target_arch = "wasm32")]
            return;
        }
    };
    mark("running", "");
    report(&format!("e2e build {}", crate::BUILD_ID));
    app.insert_resource(Driver {
        fixture,
        phase: Phase::Open,
        material: None,
        batches: batches.into(),
        hashes: Vec::new(),
        base: 0,
        start_time: 0.0,
        observing: Observing::default(),
        endpoints: Vec::new(),
        time_steps: Vec::new(),
        events: Events {
            bursts: fixture.bursts().into(),
            ..Events::default()
        },
        deadline: Instant::now() + std::time::Duration::from_secs(seconds.unwrap_or(120)),
    })
    .add_systems(EguiPrimaryContextPass, drive.before(frame))
    .init_resource::<LoseDevice>()
    .add_plugins(ExtractResourcePlugin::<LoseDevice>::default());
    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app.add_systems(Render, lose_device.in_set(RenderSystems::Cleanup));
    }
}

/// Asks the render world to destroy the application's own device, as
/// [`Action::LoseDevice`] does: the loss then arrives as a real one, through
/// wgpu's callbacks and Bevy's error handler, to the application's answer
/// (`stopped`).
#[derive(Resource, ExtractResource, Clone, Default)]
struct LoseDevice(bool);

fn lose_device(request: Res<LoseDevice>, device: Res<RenderDevice>, mut done: Local<bool>) {
    if request.0 && !*done {
        *done = true;
        device.wgpu_device().destroy();
    }
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
    /// The cavity, its medium's permittivity raised at step 128 of 256: a
    /// whole new generation, handed the running field on the same mesh.
    Handoff,
    /// `Handoff` with a disc of a parametric pump, a medium driven from the
    /// run's start: a driven generation handed to another, its candidate
    /// packed against the time the run had reached and the pump's phase
    /// carried, read against the core's driven handoff. Until 2026-10-20
    /// every driven handoff was refused once a run had advanced (RT-1 of
    /// that day's review).
    HandoffDriven,
    /// The cavity's whole medium the parametric pump and its top side
    /// driven, the same edit at step 128: the driven generation's pinned
    /// nodes lie on a medium whose mass at the handoff is not the authored
    /// one. The handed state is read at the handoff's own step, pins
    /// included, against the core's driven handoff, which pins them at the
    /// mass in force there; the device pinned them at the authored mass
    /// until 2026-10-22 (GPU-3 of the 2026-10-20 review).
    HandoffDrivenPins,
    /// The cavity remeshed finer at step 128 of 256, nothing else changed:
    /// the field is interpolated onto another mesh of the same geometry, its
    /// component totals kept.
    Remesh,
    /// IV-10 of the testing campaign on the device: the cavity parted by a
    /// reflecting baffle from wall to wall and an island inside a reflecting
    /// loop, three components, each struck by a pulse, remeshed from edge
    /// 0.04 to 0.028 at step 128 of 256. The device's own state is read just
    /// before the remesh and at the handoff, and each part's total, counted
    /// over the nodes of the elements whose centres lie in it, which the
    /// driver tells from where the baffle and the loop are, must come through
    /// to the device's precision: the conservation of the device's own carry,
    /// which an agreement with the reference would not show where both carried
    /// alike.
    NeutralRemesh,
    /// IV-12 of the testing campaign on the device: the cavity at edge 0.04
    /// with a free-ended reflecting baffle, an arc across its middle, struck
    /// below it and run until the pulse has come back off it, then the baffle
    /// dragged up an edge at step [`BAFFLE_MOVE_STEP`]. The device's own state
    /// is read just before the move and at the handoff: at the moved baffle
    /// the jump must stand within [`MOVED_SIDES`] of each side's own values,
    /// and no more than [`LEFT_BEHIND`] of it may be left where the baffle
    /// was, which an agreement with the reference would not show where both
    /// carried alike.
    BaffleMove,
    /// IV-08 edited mid-run on the device: the cavity at
    /// [`SEPARATED_EDGE`] in IV-08's medium, anisotropic in a turned frame
    /// and graded along it, struck off its middle; IV-08's loop, a subdomain
    /// of the same medium, drawn through the editor at step 128 and deleted
    /// at 192, its face given back to the background; the run taken to 256.
    /// Each handed state is read against the reference at its handoff, and
    /// the device's endpoint, read on its own state at a grid of points,
    /// must stand from the unedited run within [`UNEDITED_APART`]: a
    /// separator within one medium is none, which an agreement with the
    /// reference would not show where both carried alike.
    SeparatorMidRun,
    /// IV-05 of the testing campaign on the device: the cavity with a point
    /// probe, struck off its middle and run to step 256 twice, once with
    /// every observer the app has off and once with all of them on. The
    /// vector overlay, which samples the solver's state inside the solver's
    /// own pass, auto exposure and the probe's readout window are on from
    /// the start, and a recording runs from the start; at step [`PAUSE_AT`]
    /// the run is paused for [`PAUSE_FOR`], and at [`RESIZE_AT`] the
    /// recording stops and the window is resized and zoomed, which uploads
    /// the overlay's lattice again. The readout window, which a recording
    /// hides, is drawn from then on. The two endpoints must be the same
    /// bits, and each observer must show it took.
    Observers,
    /// IV-01 of the testing campaign on the device: `HandoffMemory`'s cavity,
    /// its bottom side a second-order outgoing wall and its top driven, run
    /// to step 384 four times, as [`SUPERPOSED`] says: struck at two points
    /// and driven, struck at the first and driven, struck at the second, near
    /// the wall, with the drive at zero, and all of it negated. Each run's
    /// waves fill the wall's memory. At the endpoints the first must be the
    /// sum of the next two within [`SUPERPOSED_RESIDUAL`], and the last the
    /// first negated, every value of Q, b and the wall's pole states:
    /// rounding is symmetric, so only an operation of the device's that is
    /// not odd can part them.
    Superposition,
    /// IV-06 of the testing campaign on the device: `HandoffMemory`'s cavity,
    /// its bottom side a second-order outgoing wall and its top driven, run
    /// to step 384, and run again [`SOONER`] times sooner: its medium's
    /// density divided by the square, so that its waves run that much
    /// faster, its drive that much faster and stronger and its pulse that
    /// much stronger, at the step the app takes for it, the first's divided
    /// by [`SOONER`]. At the endpoints the second's Q times [`SOONER`], and
    /// its b and the wall's pole states as they are, must be the first's,
    /// every value: a power of four moves no rounding, so only an operation
    /// of the device's that is not homogeneous in time can part them.
    Similarity,
    /// The cavity with its bottom side a second-order outgoing wall and its
    /// top side driven, the pulse run into the wall until its pole states
    /// hold memory, then handed off at step 192 of 384 the way named: the
    /// memory must come through the handoff, the drive keep its phase, and
    /// the clock its origin, read against a reference that carries the memory
    /// the physical way and runs on from the time reached.
    HandoffMemory(MemoryHandoff),
    /// The cavity's medium made defocusing Kerr with an amplitude bound and
    /// driven at its resonance until the field reaches the bound: the device
    /// must refuse the step the reference refuses, the app pause on it with
    /// the last accepted state kept, and Run retry it to the same refusal
    /// without moving a stored bit.
    Rejection,
    /// The cavity, its own GPU device destroyed at step 128 of 256, after a
    /// change to the document the autosave had not yet written: rendering
    /// must stop and the app say so within two seconds, with the document,
    /// change included, in the autosave, and nothing left pending.
    DeviceLoss,
    /// PH-18: a TM channel its left side drives into a matched wall, past
    /// two points, two segments, a boundary probe, a region and a disk
    /// (`observables::observables_document`), run 8 s at the app's own
    /// pacing. Every trace the readouts draw is held to the recorders' f64
    /// twins at the steps it recorded, none missing, with the readouts'
    /// spectrum and transfer and Poynting's theorem on what the device
    /// recorded. Then the same run without its probes must end on the same
    /// bits: a probe only reads.
    Observables,
    /// PH-18: a TM box of outgoing walls with a hole and a source beside it,
    /// its far field on (`far_field::far_field_document`), run 6.5 s at the
    /// app's own pacing. Every record the far-field readout draws is held to
    /// the recorder's f64 twin at the frame it was projected from. Then the
    /// same run with the far field off must end on the same bits.
    FarField,
    /// The far field 36 hours into a run, its clock started 400 steps before
    /// an epoch's rebase: every stamp and every difference of two the device
    /// takes at that age, and across the new origin the rebase adds on, held
    /// to the twin, which forms them as the shader's text does. A fast-math
    /// compiler that folded a two-part difference back into one f32 would
    /// read times f32's spacing at 36 hours off, 1.6e-2 s.
    FarFieldLate,
    /// RT-04: the cavity with the switchable disc, struck at step 0 and run
    /// to step 384 held nowhere else, [`Fixture::bursts`] fired into it as it
    /// runs, several pulses and Switches in one frame. The app sends them
    /// one at a time; every one must land once, in the order fired, and the
    /// reference, which replays the timeline the device was given, must
    /// stand with the device at the endpoint.
    Events,
    /// `Events` with the medium's permittivity raised at step 160 and the
    /// run going on, bursts fired beside it: each event lands once, on the
    /// generation before the handoff or the one after.
    EventsHandoff,
}

/// How [`Fixture::HandoffMemory`] hands the running field over at step 192.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MemoryHandoff {
    /// Remesh pressed with nothing changed: the same mesh built again, the
    /// transfer an identity in effect, and the device must stand where the
    /// uninterrupted run stands, pole states included.
    Rebuild,
    /// The mesh edge set finer: the memory carried onto another trace, read
    /// in its physical representation within the carry's budget.
    Remesh,
    /// The medium's mass density raised: a new operator and a new wall basis
    /// on the same mesh, the reference changed the same way at the same step.
    Edit,
    /// The simulation speed halved: the same state stepped on at a shorter
    /// time step from the accepted step, the reference re-stepped at it.
    Timestep,
    /// The top side's law set to a harmonic Neumann flux: a drive the new
    /// generation starts, which the device integrates from rest at the
    /// handoff, the stage-6 rule, and the reference with it
    /// (`CanonicalForcing::handed_on`). Packed against the running time, the
    /// device started it at the rate its authored integral had reached by
    /// the candidate's preparation, a steady flux the walls kept (GPU-2 of
    /// the 2026-10-20 review).
    Flux,
    /// The top side's drive switched on, with the run not held: the device
    /// steps on while the candidate is prepared, as it does for a user, and
    /// the handoff lands at whatever step it has reached, read from the new
    /// generation's clock origin; the reference steps to that step and
    /// carries from there. A drive the new generation starts takes its
    /// authored phase moved on by the time between the candidate's
    /// preparation and its arrival, which the runtime transfer computes; a
    /// held run has none of that time, and a drive carried from the old
    /// generation takes its phase from the old clock.
    Running,
    /// `Running` 36 hours into a run, its clock started 300 steps before an
    /// epoch's rebase: the new generation's origin and the delay that moves a
    /// drive it starts on are compensated sums at that age, which a fast-math
    /// compiler that folded them would leave f32's spacing at 36 hours off,
    /// 1.6e-2 s.
    RunningLate,
}

/// The meshes the fixtures run on, as [`mesh_hashes`] reads them: the cavity,
/// the cavity with the switchable disc, and the cavity remeshed finer. The
/// mesher decides through portable arithmetic, so every platform must build
/// them to the bit: the M1 Max natively, Chrome's wasm and Linux on arm64 do,
/// and CI's x86_64 runner holds them each run. A change to the mesher made on
/// purpose records them again here.
const CAVITY_MESH: &str = "fdbd8736b4d960ea/c57811e22d49eb75";
const DISC_MESH: &str = "b7183a42450e64cb/70e06866ae57b36a";
const FINER_CAVITY_MESH: &str = "2a08c7884de88562/d105b9149fd88df1";
const CHANNEL_MESH: &str = "87169bdb09e61b78/b63cee07f5ecf758";
const HOLE_MESH: &str = "f84998df08704d66/34b4ee47d28f53ae";
const PARTED_MESH: &str = "6f134ce035236366/db143b236f3be3fb";
const FINER_PARTED_MESH: &str = "345ff654d5d190eb/7b78d839d7290095";
const BAFFLED_MESH: &str = "cb876407d8c1eb75/63a98d72da510426";
const SEPARATED_MESH: &str = "8a6e5526c63c5d41/5927bf38c673576e";

/// The batches [`Fixture::CavityBatches`] asks for its steps in: one at a
/// time, a number that divides nothing, half the most a frame may ask for
/// (`MAX_STEPS_PER_FRAME`), and the most. Half the run and the whole run, as
/// it was, both ran 64 a frame.
const BATCHES: [u64; 4] = [1, 7, 32, 64];

/// The event fixtures' endpoint, and the step `events-handoff` edits at.
const EVENTS_ENDPOINT: u64 = 384;
const EVENTS_EDIT: u64 = 160;

/// What an event fixture fires as the run goes on: a pulse at one of
/// [`BURST_POINTS`], or the scene's Switch pressed.
#[derive(Clone, Copy, Debug)]
enum Burst {
    Pulse(usize),
    Switch,
}

/// Where a burst's pulses strike, as fractions of the domain.
const BURST_POINTS: [(f64, f64); 4] = [(0.25, 0.3), (0.7, 0.65), (0.5, 0.2), (0.35, 0.75)];

/// The event fixtures' bursts: three pulses and a Switch in one frame; a
/// Switch, two pulses and a Switch in another; where the fixture edits, a
/// pulse, a Switch and a pulse in the frame the edit's step is reached; and
/// a pulse and a Switch after.
const EVENT_BURSTS: &[(u64, &[Burst])] = &[
    (
        48,
        &[
            Burst::Pulse(0),
            Burst::Pulse(1),
            Burst::Switch,
            Burst::Pulse(2),
        ],
    ),
    (
        112,
        &[
            Burst::Switch,
            Burst::Pulse(3),
            Burst::Pulse(0),
            Burst::Switch,
        ],
    ),
    (
        EVENTS_EDIT,
        &[Burst::Pulse(1), Burst::Switch, Burst::Pulse(2)],
    ),
    (200, &[Burst::Pulse(3), Burst::Switch]),
];

/// Bursts drawn from `seed`, for the event fixtures' study: two to four, at
/// steps from 16 to the endpoint's last 64, of one to five events each, two
/// pulses to every Switch.
#[cfg(not(target_arch = "wasm32"))]
fn seeded_bursts(seed: u64) -> Vec<(u64, Vec<Burst>)> {
    let mut state = seed;
    let mut next = |bound: u64| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) % bound
    };
    let mut bursts = Vec::new();
    for _ in 0..2 + next(3) {
        let step = 16 + next(EVENTS_ENDPOINT - 80);
        let mut events = Vec::new();
        for _ in 0..1 + next(5) {
            events.push(if next(3) == 0 {
                Burst::Switch
            } else {
                Burst::Pulse(next(BURST_POINTS.len() as u64) as usize)
            });
        }
        bursts.push((step, events));
    }
    bursts.sort_by_key(|(step, _)| *step);
    bursts
}

/// What an event the fixture fired was, as the reference replays it.
#[derive(Clone, Copy, Debug)]
enum Fired {
    Pulse {
        position: Point2,
        region: RegionId,
        amplitude: f64,
        width: f64,
    },
    Switch {
        material: MaterialId,
        target: bool,
        ramp: f64,
    },
}

/// An event of the timeline the device was given: the generation and the
/// accepted step it landed at, what it was, and where it stood in the order
/// the fixture fired them, pulses and Switches together.
#[derive(Clone, Copy, Debug)]
struct Landed {
    generation: u64,
    step: u64,
    fired: Fired,
    order: usize,
}

/// What a fixture does at a step it holds the run at.
#[derive(Clone, Copy, Debug)]
enum Action {
    /// Switch's hotkey, which throws the scene's only Switch.
    Switch,
    /// The default medium's permittivity raised by half, as its row in the
    /// material panel does on Apply.
    Edit,
    /// The mesh's target edge set finer, as its preset or slider does.
    Remesh,
    /// Remesh pressed, which builds the mesh again at the edge it has.
    Rebuild,
    /// The simulation speed halved, as its slider does, which shortens the
    /// time step and hands the running field to a generation stepping at it.
    Timestep,
    /// The top side's law set to the driven Dirichlet, as the inspector's
    /// picker does: a drive the new generation starts.
    Drive,
    /// The top side's law set to a harmonic Neumann flux, as the inspector's
    /// picker does: an integrated drive the new generation starts.
    Flux,
    /// The view's grid toggled, a change for the autosave to carry, and the
    /// device destroyed.
    LoseDevice,
    /// The cavity's baffle selected and dragged up [`BAFFLE_SHIFT`] edges, as
    /// the Move tool does.
    Move,
    /// [`SEPARATOR_LOOP`] drawn round part of the cavity, a subdomain of the
    /// medium it encloses, as the closed-curve tool draws one.
    Draw,
    /// The loop deleted, its face merged with the background's, whose region
    /// is kept, as the merge picker keeps it.
    Delete,
}

impl Fixture {
    /// The step each action is taken at, in order, and the endpoint.
    fn script(self) -> (&'static [(u64, Action)], u64) {
        match self {
            Fixture::Cavity | Fixture::CavityBatches | Fixture::Observers => (&[], 256),
            Fixture::Switch => (&[(128, Action::Switch)], 256),
            Fixture::Handoff | Fixture::HandoffDriven | Fixture::HandoffDrivenPins => {
                (&[(128, Action::Edit)], 256)
            }
            Fixture::Remesh | Fixture::NeutralRemesh => (&[(128, Action::Remesh)], 256),
            Fixture::BaffleMove => (&[(BAFFLE_MOVE_STEP, Action::Move)], BAFFLE_MOVE_STEP + 128),
            // The pulse's front reaches the wall near step 150, and the wall
            // holds memory for the rest of the run.
            Fixture::Superposition | Fixture::Similarity => (&[], 384),
            Fixture::SeparatorMidRun => (&[(128, Action::Draw), (192, Action::Delete)], 256),
            // The pulse's front reaches the wall near step 150: the handoff
            // comes once the wall holds memory, and the run goes on for as
            // long again, so that what the wall gives back is in the field.
            Fixture::HandoffMemory(MemoryHandoff::Rebuild) => (&[(192, Action::Rebuild)], 384),
            Fixture::HandoffMemory(MemoryHandoff::Remesh) => (&[(192, Action::Remesh)], 384),
            Fixture::HandoffMemory(MemoryHandoff::Edit) => (&[(192, Action::Edit)], 384),
            Fixture::HandoffMemory(MemoryHandoff::Timestep) => (&[(192, Action::Timestep)], 384),
            Fixture::HandoffMemory(MemoryHandoff::Flux) => (&[(192, Action::Flux)], 384),
            // The handoff lands wherever the device has got to while the
            // candidate was prepared, and the endpoint must leave it room.
            Fixture::HandoffMemory(MemoryHandoff::Running | MemoryHandoff::RunningLate) => {
                (&[(192, Action::Drive)], 768)
            }
            // The endpoint is never reached: the device is gone at 128.
            Fixture::DeviceLoss => (&[(128, Action::LoseDevice)], 256),
            // The endpoint is a ceiling the run must not reach.
            Fixture::Rejection => (&[], 4096),
            Fixture::Observables => (&[], observables::OBSERVED_STEPS),
            Fixture::FarField | Fixture::FarFieldLate => (&[], far_field::FAR_FIELD_STEPS),
            Fixture::Events => (&[], EVENTS_ENDPOINT),
            Fixture::EventsHandoff => (&[(EVENTS_EDIT, Action::Edit)], EVENTS_ENDPOINT),
        }
    }

    /// Whether the run holds at the step an action is taken at until the
    /// device has taken it. A fixture that does not lets the device step on
    /// while an edit's candidate is prepared, and reads where the handoff
    /// landed from the new generation's clock.
    fn holds(self) -> bool {
        !matches!(
            self,
            Fixture::HandoffMemory(MemoryHandoff::Running | MemoryHandoff::RunningLate)
                | Fixture::EventsHandoff
        )
    }

    /// The bursts the fixture fires as the run goes on, each at the first
    /// frame its step has completed by: none but for the event fixtures,
    /// whose bursts are [`EVENT_BURSTS`] or, natively, drawn from
    /// `FUNFERN_E2E_SEED`.
    fn bursts(self) -> Vec<(u64, Vec<Burst>)> {
        let edit = match self {
            Fixture::Events => None,
            Fixture::EventsHandoff => Some(EVENTS_EDIT),
            _ => return Vec::new(),
        };
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(seed) = std::env::var("FUNFERN_E2E_SEED")
            .ok()
            .and_then(|seed| seed.parse::<u64>().ok())
        {
            return seeded_bursts(seed);
        }
        EVENT_BURSTS
            .iter()
            .filter(|(step, _)| edit.is_some() || *step != EVENTS_EDIT)
            .map(|(step, events)| (*step, events.to_vec()))
            .collect()
    }

    /// Whether the handed state is read at the handoff's own step, before
    /// the new generation steps, where no wall's memory calls for it: a
    /// driven generation's pins, which its first step sets again.
    fn reads_the_handoff(self) -> bool {
        matches!(
            self,
            Fixture::HandoffDrivenPins
                | Fixture::NeutralRemesh
                | Fixture::BaffleMove
                | Fixture::SeparatorMidRun
        )
    }

    /// Whether each part's total is read off the device's own state before
    /// the remesh and at the handoff.
    fn counts_the_parts(self) -> bool {
        matches!(self, Fixture::NeutralRemesh)
    }

    /// Whether the jump across the baffle is read off the device's own state
    /// before the move and at the handoff.
    fn reads_the_sides(self) -> bool {
        matches!(self, Fixture::BaffleMove)
    }

    /// Whether the endpoint is read against the run without the fixture's
    /// actions, which the device must stand near rather than far from.
    fn reads_unedited(self) -> bool {
        matches!(self, Fixture::SeparatorMidRun)
    }

    /// Whether the fixture's second run turns every observer on.
    fn observes(self) -> bool {
        matches!(self, Fixture::Observers)
    }

    /// The width the fixture sets its pulse to, as the pulse's slider does,
    /// where it does not take the app's.
    fn pulse_width(self) -> Option<f32> {
        match self {
            // As wide as IV-12's in `tests/invariants.rs`, so that the field
            // beside the baffle is resolved.
            Fixture::BaffleMove => Some(0.2),
            // As wide, so that the unedited run's own move over a halving of
            // the mesh, which [`UNEDITED_APART`] is, is the field's and not
            // the pulse's.
            Fixture::SeparatorMidRun => Some(0.2),
            // The slider's widest, which reaches every node from the start,
            // 1e-11 of its peak at the farthest. A node the pulse leaves for
            // the stencil to reach holds values down to where f32 flushes
            // them to zero, and the sooner run's Q, a quarter of the first's,
            // flushes nodes the first's keeps: one bit apart, every rounding
            // after it parts, and the runs stood 9e-7 apart by step 384.
            Fixture::Similarity => Some(0.25),
            _ => None,
        }
    }

    /// Where the fixture's pulses strike, in `domain`: one off the middle, or
    /// one in each part of the parted cavity, which a pulse does not leave.
    fn pulses_at(self, domain: DomainRect) -> Vec<Point2> {
        match self {
            Fixture::NeutralRemesh => vec![
                Point2::new(0.0, -0.6),
                Point2::new(0.7, 0.0),
                PARTED_ISLAND.0,
            ],
            Fixture::BaffleMove => vec![Point2::new(
                domain.min_x + 0.5 * domain.width(),
                domain.min_y + 0.38 * domain.height(),
            )],
            // The usual point, and one near the wall.
            Fixture::Superposition => vec![
                Point2::new(
                    domain.min_x + 0.37 * domain.width(),
                    domain.min_y + 0.58 * (domain.max_y - domain.min_y),
                ),
                Point2::new(
                    domain.min_x + 0.7 * domain.width(),
                    domain.min_y + 0.3 * domain.height(),
                ),
            ],
            _ => vec![Point2::new(
                domain.min_x + 0.37 * domain.width(),
                domain.min_y + 0.58 * (domain.max_y - domain.min_y),
            )],
        }
    }

    /// The edge a remesh sets, and the mesh it must build.
    fn finer(self) -> (f64, &'static str) {
        match self {
            Fixture::NeutralRemesh => (0.028, FINER_PARTED_MESH),
            _ => (0.06, FINER_CAVITY_MESH),
        }
    }

    /// Whether a pulse at step 0 sets the field going. A field-dependent medium
    /// takes none - a pulse's increment is not derived through its nonlinear
    /// map, so the app refuses one - and its fixture is driven by its source.
    fn pulses(self) -> bool {
        !matches!(
            self,
            Fixture::Rejection | Fixture::Observables | Fixture::FarField | Fixture::FarFieldLate
        )
    }

    /// The relative L2 error Q, b and r may reach. Measured on the M1 Max the
    /// fixtures read 6e-7 to 1.2e-6 after their 256 steps; 5e-6 leaves four
    /// times that, where the device examples' 3e-5 let a remesh that lost its
    /// component totals through at 2.5e-5. A device that reads more is to be
    /// looked at, not given more.
    fn tolerance(self) -> f64 {
        match self {
            // 1195 and 1150 steps, not 256: the M1 Max reads 2.1e-6 on the
            // first.
            Fixture::Observables | Fixture::FarField => 2.0e-5,
            // Late in an epoch the local time is one f32 near 256 s, 3e-5 s
            // apart, and the drives' phases carry 1e-4 rad from it, which a
            // run early in its epoch never reaches: the M1 Max reads 1.25e-4
            // 254 s into the epoch, and 2.1e-5 to 3.9e-5 where a handoff starts
            // a new one, by the step it lands at.
            Fixture::FarFieldLate => 5.0e-4,
            Fixture::HandoffMemory(MemoryHandoff::RunningLate) => 2.0e-4,
            _ => 5.0e-6,
        }
    }

    /// The mesh the fixture's document must be built on.
    fn mesh(self) -> &'static str {
        match self {
            Fixture::Switch | Fixture::HandoffDriven | Fixture::Events | Fixture::EventsHandoff => {
                DISC_MESH
            }
            Fixture::Observables => CHANNEL_MESH,
            Fixture::FarField | Fixture::FarFieldLate => HOLE_MESH,
            Fixture::NeutralRemesh => PARTED_MESH,
            Fixture::BaffleMove => BAFFLED_MESH,
            Fixture::SeparatorMidRun => SEPARATED_MESH,
            _ => CAVITY_MESH,
        }
    }

    /// The fixture's document, and the material its Switch throws.
    fn document(self) -> (TopologyDocument, Option<MaterialId>) {
        match self {
            Fixture::Cavity
            | Fixture::CavityBatches
            | Fixture::Handoff
            | Fixture::Remesh
            | Fixture::DeviceLoss => (cavity_document(), None),
            Fixture::Rejection => (rejection_document(), None),
            Fixture::NeutralRemesh => (parted_document(), None),
            Fixture::BaffleMove => (baffled_document(), None),
            Fixture::SeparatorMidRun => (separated_document(), None),
            Fixture::Observers => (observers_document(), None),
            Fixture::Superposition | Fixture::Similarity => (wall_document(true), None),
            Fixture::HandoffDriven => (pump_document(), None),
            Fixture::HandoffDrivenPins => (pumped_driven_document(), None),
            Fixture::Observables => (observables::observables_document(), None),
            Fixture::FarField => (far_field::far_field_document(0.0), None),
            Fixture::FarFieldLate => (
                far_field::far_field_document(far_field::LATE_ORIGIN + far_field::LATE_DRIVE),
                None,
            ),
            Fixture::HandoffMemory(
                MemoryHandoff::Running | MemoryHandoff::RunningLate | MemoryHandoff::Flux,
            ) => (wall_document(false), None),
            Fixture::HandoffMemory(_) => (wall_document(true), None),
            Fixture::Switch | Fixture::Events | Fixture::EventsHandoff => {
                let (document, material) = switch_document();
                (document, Some(material))
            }
        }
    }
}

#[derive(Resource)]
pub(crate) struct Driver {
    fixture: Fixture,
    phase: Phase,
    /// The material the fixture's Switch throws.
    material: Option<MaterialId>,
    /// The batch of each run still to come, the current one first; none, the
    /// frame's own pacing.
    batches: VecDeque<Option<u64>>,
    /// Each finished run's batch and the hash of its accepted endpoint.
    hashes: Vec<(Option<u64>, u64)>,
    /// The device's completed-step count when the run's generation was
    /// installed, which its clock's step in the epoch seeds: zero but where
    /// the fixture starts its clock late. The script's steps count from it.
    base: u64,
    /// The time the run starts at: zero, but where the fixture starts its
    /// clock late.
    start_time: f64,
    /// What the observed run's observers have done so far.
    observing: Observing,
    /// Each finished run's endpoint where the fixture reads the runs against
    /// each other: Q, b and the wall's pole states.
    endpoints: Vec<[Vec<f32>; 3]>,
    /// The step each of those runs took.
    time_steps: Vec<f64>,
    /// The event fixtures' timeline: what they fire and what the device
    /// was given.
    events: Events,
    deadline: Instant,
}

/// What an event fixture has fired and what the device was given of it.
#[derive(Default)]
struct Events {
    /// The bursts still to fire, the next first.
    bursts: VecDeque<(u64, Vec<Burst>)>,
    /// The pulses fired, in the order fired, with where each stood among
    /// all the events fired; where each Switch that queued stood; and how
    /// many presses were the same request as one still waiting.
    pulses: Vec<(Fired, usize)>,
    switches: Vec<usize>,
    merged: usize,
    /// How many of each have landed, and where the last Switch headed.
    pulses_landed: usize,
    switches_landed: usize,
    heading: bool,
    /// The serial the timeline starts after: the run's own pulse is the
    /// reference's from the start. None until the run starts.
    from: Option<u32>,
    /// Each generation's stats, whose encoded events make the timeline.
    generations: Vec<(u64, Arc<crate::canonical_gpu::CanonicalGpuStats>)>,
    /// The timeline so far, by serial.
    timeline: BTreeMap<u32, Landed>,
}

impl Events {
    /// Whether everything has been fired and nothing waits to be sent or
    /// to be taken: what landed then is all that will, and the endpoint
    /// counts it.
    fn settled(&self, state: &Playground, request: &CanonicalGpuRequest) -> bool {
        self.bursts.is_empty()
            && state.coordinator.pending_pulses.is_empty()
            && state.coordinator.pending_switches.is_empty()
            && !request.live_event_pending()
    }
}

/// Whether every event the fixture fired landed once, the serials the app
/// gave them without a gap, and the reference replayed them all: the
/// timeline, for the summary.
fn landed_once(events: &Events, reference: &Reference, from: u32) -> Result<String, String> {
    if events.pulses_landed != events.pulses.len()
        || events.switches_landed != events.switches.len()
    {
        return Err(format!(
            "{} of {} pulses and {} of {} Switches landed",
            events.pulses_landed,
            events.pulses.len(),
            events.switches_landed,
            events.switches.len()
        ));
    }
    // Pulses and Switch presses go in the order pressed, kinds together:
    // offered the pulses first, a click landed before a Switch pressed
    // before it (2026-11-07).
    let mut latest: Option<(u32, usize)> = None;
    for (&serial, landed) in &events.timeline {
        if let Some((earlier, order)) = latest
            && order > landed.order
        {
            return Err(format!(
                "event {serial}, fired {}th, landed after event {earlier}, fired {}th",
                landed.order + 1,
                order + 1
            ));
        }
        latest = Some((serial, landed.order));
    }
    let serials = events.timeline.keys().copied().collect::<Vec<_>>();
    if serials
        .iter()
        .copied()
        .ne(from + 1..from + 1 + serials.len() as u32)
    {
        return Err(format!(
            "the events' serials run {serials:?}, not on from {}",
            from + 1
        ));
    }
    if serials
        .last()
        .is_some_and(|last| reference.replayed != *last)
    {
        return Err(format!(
            "the reference replayed the events to serial {}, not {:?}",
            reference.replayed,
            serials.last()
        ));
    }
    let first = events
        .generations
        .first()
        .map_or(0, |(generation, _)| *generation);
    let steps = |pulse: bool| {
        events
            .timeline
            .values()
            .filter(|landed| matches!(landed.fired, Fired::Pulse { .. }) == pulse)
            .map(|landed| {
                format!(
                    "{}{}",
                    landed.step,
                    if landed.generation == first { "" } else { "'" }
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    Ok(format!(
        "; {} events landed once each, in the order fired: pulses at {}, Switches at {}{}{}",
        serials.len(),
        steps(true),
        steps(false),
        if events.merged > 0 {
            format!(
                ", {} presses the same request as one waiting",
                events.merged
            )
        } else {
            String::new()
        },
        if events.generations.len() > 1 {
            " (' on the edited generation)"
        } else {
            ""
        }
    ))
}

/// Fires the event fixture's bursts that are due, and reads what the device
/// was given into the timeline, which the reference replays: each event the
/// render world encoded, by serial, matched to what was fired in the order
/// fired, a pulse to the next pulse and a Switch to the next Switch.
fn follow_events(
    driver: &mut Driver,
    state: &mut Playground,
    request: &CanonicalGpuRequest,
) -> Result<(), String> {
    let Some(from) = driver.events.from else {
        return Ok(());
    };
    let events = &mut driver.events;
    let completed = request
        .stats()
        .completed_steps()
        .saturating_sub(driver.base);
    while events
        .bursts
        .front()
        .is_some_and(|(step, _)| completed >= *step)
    {
        let (_, burst) = events.bursts.pop_front().expect("a burst is due");
        let domain = state.editor.document.model.accepted.geometry.domain;
        for fired in burst {
            match fired {
                Burst::Pulse(index) => {
                    let (x, y) = BURST_POINTS[index];
                    let point = Point2::new(
                        domain.min_x + x * domain.width(),
                        domain.min_y + y * domain.height(),
                    );
                    let queued = state.coordinator.pending_pulses.len();
                    state.place_pulse(point);
                    let Some(&(position, region)) = state
                        .coordinator
                        .pending_pulses
                        .back()
                        .filter(|_| state.coordinator.pending_pulses.len() == queued + 1)
                    else {
                        return Err(format!("a burst's pulse was not queued: {}", state.message));
                    };
                    let order = events.pulses.len() + events.switches.len();
                    events.pulses.push((
                        Fired::Pulse {
                            position,
                            region,
                            amplitude: f64::from(state.pulse_amplitude),
                            width: f64::from(state.pulse_width),
                        },
                        order,
                    ));
                }
                // A press while the material's last waits to be sent is the
                // same request (2026-10-07): its button still shows the old
                // direction.
                Burst::Switch => {
                    let waiting = driver.material.is_some_and(|material| {
                        state.coordinator.pending_switches.contains(&material)
                    });
                    let queued = state.coordinator.pending_switches.len();
                    state.request_material_switch();
                    if waiting {
                        if state.coordinator.pending_switches.len() != queued {
                            return Err("a second press queued a second Switch".into());
                        }
                        events.merged += 1;
                    } else if state.coordinator.pending_switches.len() != queued + 1 {
                        return Err(format!(
                            "a burst's Switch was not queued: {}",
                            state.message
                        ));
                    } else {
                        events
                            .switches
                            .push(events.pulses.len() + events.switches.len());
                    }
                }
            }
        }
    }
    let generation = request.generation();
    if !events
        .generations
        .iter()
        .any(|(seen, _)| *seen == generation)
    {
        events
            .generations
            .push((generation, Arc::clone(request.stats())));
    }
    let mut encoded = Vec::new();
    for (generation, stats) in &events.generations {
        let log = stats.encoded_events();
        for event in &log {
            if log
                .iter()
                .filter(|other| other.serial == event.serial)
                .count()
                > 1
            {
                return Err(format!(
                    "event {} was encoded twice on generation {generation}",
                    event.serial
                ));
            }
            if event.serial > from {
                encoded.push((*generation, *event));
            }
        }
    }
    encoded.sort_by_key(|(_, event)| event.serial);
    for (generation, event) in encoded {
        if let Some(landed) = events.timeline.get(&event.serial) {
            if (landed.generation, landed.step) != (generation, event.step) {
                return Err(format!(
                    "event {} landed twice, on generation {} at step {} and on {generation} \
                     at step {}",
                    event.serial, landed.generation, landed.step, event.step
                ));
            }
            continue;
        }
        let (fired, order) = if event.is_pulse() {
            let Some(fired) = events.pulses.get(events.pulses_landed).copied() else {
                return Err(format!(
                    "a pulse the fixture never fired landed at step {}",
                    event.step
                ));
            };
            events.pulses_landed += 1;
            fired
        } else if event.is_switch() {
            let Some(&order) = events.switches.get(events.switches_landed) else {
                return Err(format!(
                    "a Switch the fixture never pressed landed at step {}",
                    event.step
                ));
            };
            let Some(material) = driver.material else {
                return Err("a Switch with no material to throw".into());
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
                return Err("the switched material is gone".into());
            };
            events.switches_landed += 1;
            events.heading = !events.heading;
            (
                Fired::Switch {
                    material,
                    target: events.heading,
                    ramp,
                },
                order,
            )
        } else {
            return Err(format!(
                "an event of kind {} the fixture never fired landed at step {}",
                event.kind, event.step
            ));
        };
        if let Some((&last, landed)) = events.timeline.last_key_value()
            && (landed.generation, landed.step) > (generation, event.step)
        {
            return Err(format!(
                "event {} landed at step {} on generation {generation}, before event {last} \
                 at step {} on {}",
                event.serial, event.step, landed.step, landed.generation
            ));
        }
        events.timeline.insert(
            event.serial,
            Landed {
                generation,
                step: event.step,
                fired,
                order,
            },
        );
    }
    if let Some(reference) = driver.phase.reference_mut() {
        reference.timeline.clone_from(&driver.events.timeline);
    }
    Ok(())
}

/// What [`Fixture::Observers`]' observed run has seen of its observers.
#[derive(Default)]
struct Observing {
    /// Whether the recording was asked for.
    recording_asked: bool,
    /// The steps asked for when the pause began, and when the device had
    /// completed them.
    pause: Option<(u64, Option<Instant>)>,
    /// The step the device held at through the pause, once the run went on.
    paused_at: Option<u64>,
    /// When the endpoint began waiting for the vector overlay's first
    /// readback.
    overlay_wait: Option<Instant>,
    /// How long the recording ran and, natively, the frame slots it asked
    /// for, once stopped.
    recorded: Option<(std::time::Duration, Option<u64>)>,
    /// The viewport's size before the resize, once taken.
    resized_from: Option<egui::Vec2>,
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
    /// The `next` action waits for the device to take it: a live event by
    /// its serial, an edit by the generation the device publishes for it.
    Act {
        reference: Reference,
        next: usize,
        processed: u32,
        generation: u64,
        /// What the edit's candidate was prepared with, taken while it waits:
        /// the runtime drops it once the generation is published.
        carried: Option<Carried>,
    },
    /// The device's loss was asked for at `asked`, with the document as it
    /// was then; the app is to stop and say so.
    Losing {
        asked: Instant,
        document: Box<TopologyDocument>,
    },
    /// The run holds at the `next` action's step, and a full snapshot of the
    /// device's state is asked for, to count the parts before the action.
    Departing {
        reference: Reference,
        next: usize,
        seen: u64,
    },
    /// The handoff's generation holds at the step it took the field, and a
    /// full snapshot of it is asked for, to read the carry on its own before
    /// the run goes on.
    Carried {
        reference: Reference,
        next: usize,
        seen: u64,
    },
    /// Held at the `next` action's step, or the endpoint, the run waits
    /// for the running generation's stationary flux to be taken away or found
    /// to be none, and then for a full snapshot, from which the reference
    /// takes it too; `seen` once the snapshot is asked for.
    Settling {
        reference: Reference,
        next: usize,
        seen: Option<u64>,
    },
    /// A full snapshot of the endpoint is asked for.
    Snapshot {
        reference: Reference,
        seen: u64,
    },
    /// The endpoint held, and the recording the observed run stopped is
    /// still being written.
    Recorded {
        run: Run,
        since: Instant,
    },
    /// The endpoint held, and its probes' last records are on their way.
    Observe {
        reference: Reference,
        run: Run,
        since: Instant,
    },
    /// The device refused the step after `step`, with `code`; the app is to
    /// pause on it. `first` is the refused state's hash and summary once its
    /// snapshot is in, and `retried` whether Run has cleared the failure since.
    Refused {
        reference: Reference,
        step: u64,
        code: u32,
        seen: u64,
        first: Option<(u64, String)>,
        retried: bool,
    },
    Done,
}

impl Phase {
    /// The reference the phase carries, if it carries one.
    fn reference_mut(&mut self) -> Option<&mut Reference> {
        match self {
            Phase::Pulse { reference, .. }
            | Phase::Run { reference, .. }
            | Phase::Act { reference, .. }
            | Phase::Departing { reference, .. }
            | Phase::Carried { reference, .. }
            | Phase::Settling { reference, .. }
            | Phase::Snapshot { reference, .. }
            | Phase::Observe { reference, .. }
            | Phase::Refused { reference, .. } => Some(reference),
            Phase::Open
            | Phase::Install
            | Phase::Losing { .. }
            | Phase::Recorded { .. }
            | Phase::Done => None,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Phase::Open => "opening the document",
            Phase::Install => "installing its generation",
            Phase::Pulse { .. } => "waiting for the device to take the pulse",
            Phase::Run { .. } => "running to the next stop",
            Phase::Act { .. } => "waiting for the device to take the action",
            Phase::Losing { .. } => "waiting for the app to stop after the device was lost",
            Phase::Departing { .. } => "waiting for the snapshot before the action",
            Phase::Carried { .. } => "waiting for the handoff's snapshot",
            Phase::Settling { .. } => "waiting for the handoff's stationary flux to settle",
            Phase::Snapshot { .. } => "waiting for the endpoint's snapshot",
            Phase::Recorded { .. } => "waiting for the recording to be written",
            Phase::Observe { .. } => "waiting for the probes' last records",
            Phase::Refused { .. } => "waiting for the app to pause on the refusal",
            Phase::Done => "done",
        }
    }
}

/// What a candidate generation was prepared with, for the reference's own
/// handoff: the maps the device's transfer compiles from, and the field's
/// interpolation between the meshes, which the wall's memory goes through
/// the physical way; none where the mesh is reused and the maps are
/// identities.
struct Carried {
    maps: Arc<PreparedCanonicalTransfer>,
    interpolation: Option<Arc<QuadraticTransferMap>>,
}

/// What the reference's wall memory went through at a handoff: the energy it
/// held before and after, its share of the field's energy before, the
/// carry's residual, what the new basis could not hold of the old memory
/// interpolated onto its trace relative to that memory, both measured along
/// the trace, and the
/// device's carry read against the reference's at the handoff itself, before
/// a step is taken: Q, b, the pole states and the memory they represent.
/// The device carries the memory through the normalized matrix its
/// generation was prepared with, the same map in another form, so the two
/// must agree to the fixture's tolerance there; the residual and the
/// energy's change are the carry's own.
#[derive(Clone, Copy, Debug, Default)]
struct MemoryCarry {
    source_energy: f64,
    target_energy: f64,
    share: f64,
    residual: f64,
    at_handoff: [f64; 4],
}

/// The least share of the field's energy the wall must hold at the handoff:
/// a carry of nothing would prove nothing. At step 192 the wall holds
/// 6.5e-4 of it, the pulse's front absorbed over the last forty steps; see
/// the log of 2026-10-13.
const MEMORY_SHARE_FLOOR: f64 = 1.0e-4;
/// What the carry may leave behind of the memory, relative: the new basis
/// holds the memory interpolated onto its trace up to this. The remesh from
/// 0.07 to 0.06 reads 4.8e-4 on the pulse's front, the same-mesh carries
/// roundoff; see the log of 2026-10-13.
const CARRY_RESIDUAL: f64 = 2.0e-3;
/// How far the carry may move the wall's energy, relative. The memory goes
/// as a density along the trace and an edit holds the flux it puts through
/// the wall, so a carry keeps the energy but for what a finer trace resolves
/// differently of the pulse's front: the remesh reads -2.4e-3, the mass
/// density's edit 1e-13, the rest roundoff. Carried as nodal values, the
/// remesh read +0.20 and the edit +0.22; see the log of 2026-10-13.
const CARRY_ENERGY: f64 = 1.0e-2;
/// The relative L2 error the wall's pole states, and the memory they
/// represent, may reach at the endpoint, 192 steps after the carry: the
/// device integrates them in f32 beside a field a thousand times their
/// size, and reads 2e-6 to 1.9e-5 where the carry itself reads under the
/// tolerance; see the log of 2026-10-13.
const WALL_TOLERANCE: f64 = 1.0e-4;

/// The f64 reference, on the generation the app accepted, and the same run
/// without the fixture's actions.
struct Reference {
    active: Arc<PreparedTopology>,
    state: State,
    /// The device must stand far from it: an action that changed nothing
    /// would prove nothing. None until the first action; it keeps the
    /// generation it started on.
    control: Option<(Arc<PreparedTopology>, State)>,
    steps: u64,
    /// What the wall's memory went through at the last handoff, where the
    /// generation has a wall.
    carry: Option<MemoryCarry>,
    /// Each part's total of the device's `Q` and of its magnitude just before
    /// the action, where the fixture counts the parts.
    departed: Option<([f64; 3], [f64; 3])>,
    /// The baffle's jump on the device's state just before the move, where
    /// the fixture reads the sides.
    sides: Option<SidesBefore>,
    /// The run without the fixture's actions, held at the first action's
    /// step on the generation it ran on, where the fixture reads its
    /// endpoint against it.
    unedited: Option<(Arc<PreparedTopology>, State)>,
    /// The recorders' twins, read after every step, where the run's probes
    /// are to be judged.
    twin: Option<observables::Twin>,
    /// The far-field recorder's twin, where its records are to be judged.
    far: Option<far_field::FarFieldTwin>,
    /// The maintenance serial the device last showed: the app's flux
    /// maintenance moves it when it takes away the stationary flux a
    /// handoff left, and the reference follows.
    maintenance_seen: u32,
    /// The stop, an action's step or the endpoint, at which the run last
    /// waited for that to settle.
    settled: Option<u64>,
    /// The generation the reference stands on, and the events the device
    /// was given that it replays, by serial, the last it replayed.
    generation: u64,
    timeline: BTreeMap<u32, Landed>,
    replayed: u32,
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

    /// The state at rest at `time`, where the fixture starts its clock.
    fn starting_at(self, time: f64) -> Result<Self, String> {
        match self {
            State::Fixed(state) => state
                .with_time_origin(time)
                .map(State::Fixed)
                .map_err(|error| error.to_string()),
            State::Temporal(_) if time == 0.0 => Ok(self),
            State::Temporal(_) => Err("a time-driven reference starts at zero".into()),
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

    /// The wall's memory, its normalized pole states; empty where the
    /// generation has no wall, or the state is time-varying, which no fixture
    /// with a wall is.
    fn outgoing_z(&self) -> &[f64] {
        match self {
            State::Fixed(state) => match state.auxiliaries() {
                CanonicalAuxiliaryState::Linear(auxiliaries) => auxiliaries.outgoing_z(),
                _ => &[],
            },
            State::Temporal(_) => &[],
        }
    }

    fn time_step(&self) -> f64 {
        match self {
            State::Fixed(state) => state.time_step(),
            State::Temporal(state) => state.time_step(),
        }
    }

    /// The time the state has reached, on the clock its generation keeps:
    /// its origin, the time the run had reached when the generation took
    /// the field, and its steps since.
    fn time(&self) -> f64 {
        match self {
            State::Fixed(state) => state.time(),
            State::Temporal(state) => state.time(),
        }
    }
}

impl Reference {
    /// Steps the reference, and the control, to the accepted step `step`.
    /// Takes away the stationary flux the device's cleanup took after the
    /// last handoff, once a full readback shows the maintenance serial it
    /// moved. The part is the same at every step of a fixed lossless
    /// generation, so taking it here, at whatever step the reference stands,
    /// is what the device's subtraction comes to; a comparison is then of
    /// the same field whichever side of the cleanup its snapshot fell.
    fn follow_flux_cleanup(&mut self, display: &CanonicalGpuDisplay) -> Result<(), String> {
        if display.full_readback_at != display.readbacks
            || display.runtime_serials[3] <= self.maintenance_seen
        {
            return Ok(());
        }
        self.maintenance_seen = display.runtime_serials[3];
        let State::Fixed(state) = &mut self.state else {
            return Err("the device took a driven generation's stationary flux away".into());
        };
        let operator = &self.active.canonical_operator;
        let mut job =
            CanonicalStationaryFluxJob::new(operator.clone(), state.complementary_flux().to_vec())
                .map_err(|error| error.to_string())?;
        let found = loop {
            if let Some(found) = job.advance(1 << 16) {
                break found.map_err(|error| error.to_string())?;
            }
        };
        let increment = found
            .stationary
            .iter()
            .map(|value| *value * -1.0)
            .collect::<Vec<_>>();
        state
            .apply_complementary_increment(operator, &increment)
            .map_err(|error| error.to_string())?;
        report(&format!(
            "e2e: the reference took away the handoff's stationary flux, {:.2e} of the flux's energy",
            found.energy / found.flux_energy
        ));
        Ok(())
    }

    /// Applies the events of the timeline that land where the reference
    /// stands, on its generation at its step. An event of its generation it
    /// has stepped past, or of one it has left, was never replayed; one of a
    /// later generation waits for the handoff.
    fn replay_events(&mut self) -> Result<(), String> {
        while let Some((&serial, &landed)) = self.timeline.range(self.replayed + 1..).next() {
            if landed.generation != self.generation {
                if landed.generation < self.generation {
                    return Err(format!(
                        "event {serial} landed on generation {} at step {}, which the \
                         reference had left",
                        landed.generation, landed.step
                    ));
                }
                return Ok(());
            }
            if landed.step > self.steps {
                return Ok(());
            }
            if landed.step < self.steps {
                return Err(format!(
                    "event {serial} landed at step {}, behind the reference at {}",
                    landed.step, self.steps
                ));
            }
            match landed.fired {
                Fired::Pulse {
                    position,
                    region,
                    amplitude,
                    width,
                } => {
                    let increment =
                        pulse_increment(&self.active, position, region, amplitude, width)?;
                    self.state.pulse(&self.active, &increment)?;
                }
                Fired::Switch {
                    material,
                    target,
                    ramp,
                } => {
                    let State::Temporal(temporal) = &mut self.state else {
                        return Err("a Switch on a fixed generation".into());
                    };
                    let time = temporal.time();
                    temporal
                        .runtime_mut()
                        .begin_switch(material, target, time, ramp)
                        .map_err(|error| format!("the reference did not switch: {error}"))?;
                }
            }
            self.replayed = serial;
        }
        Ok(())
    }

    fn advance_to(&mut self, step: u64) -> Result<(), String> {
        self.replay_events()?;
        while self.steps < step {
            self.state.step(&self.active)?;
            if let Some((active, control)) = &mut self.control {
                control.step(active)?;
            }
            if let Some(twin) = &mut self.twin {
                twin.observe(
                    self.state.time(),
                    self.state.primary(),
                    self.state.complementary(),
                )?;
            }
            if let Some(far) = &mut self.far {
                far.observe(self.steps + 1, self.state.time(), self.state.primary());
            }
            self.steps += 1;
            self.replay_events()?;
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut driver: ResMut<Driver>,
    mut state: ResMut<Playground>,
    mut request: ResMut<CanonicalGpuRequest>,
    display: Res<CanonicalGpuDisplay>,
    overlay: Res<VectorOverlayDisplay>,
    mut windows: Query<&mut Window, With<bevy::window::PrimaryWindow>>,
    mut contexts: bevy_egui::EguiContexts,
    stopped: Option<Res<crate::stopped::RenderStopped>>,
    mut lose: ResMut<LoseDevice>,
    mut exit: MessageWriter<AppExit>,
) {
    if matches!(driver.phase, Phase::Done) {
        return;
    }
    let fixture = driver.fixture;
    let context = contexts.ctx_mut().ok().cloned();
    let followed = follow_events(&mut driver, &mut state, &request);
    let advanced = match followed {
        Ok(()) => advance(
            &mut driver,
            &mut state,
            &mut request,
            &display,
            &overlay,
            &mut windows,
            context.as_ref(),
            stopped.as_deref(),
            &mut lose,
        ),
        Err(failure) => Some(Err(failure)),
    };
    let run = match advanced {
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
        report(&format!(
            "e2e {fixture:?}{label}: {}, endpoint hash {:016x}",
            run.summary, run.hash
        ));
        driver.hashes.push((batch, run.hash));
        run.summary
    });
    if verdict.is_ok() && !driver.batches.is_empty() {
        driver.phase = Phase::Open;
        return;
    }
    let verdict = verdict.and_then(|summary| match driver.hashes.as_slice() {
        _ if matches!(fixture, Fixture::Superposition) => {
            superposed(&driver.endpoints).map(|read| format!("{summary}; {read}"))
        }
        _ if matches!(fixture, Fixture::Similarity) => {
            similar(&driver.endpoints, &driver.time_steps).map(|read| format!("{summary}; {read}"))
        }
        [(_, first), rest @ ..] if rest.iter().any(|(_, hash)| hash != first) => Err(format!(
            "the endpoint differs {}: {}",
            match fixture {
                Fixture::Observables => "with the probes off",
                Fixture::FarField => "with the far field off",
                Fixture::Observers => "with every observer on",
                _ => "with the batching",
            },
            driver
                .hashes
                .iter()
                .map(|(batch, hash)| format!("{}: {hash:016x}", batch.unwrap_or(0)))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        [_, _, ..] if matches!(fixture, Fixture::Observables) => Ok(format!(
            "{summary}; the endpoint is the same bits with the probes off"
        )),
        [_, _, ..] if matches!(fixture, Fixture::FarField) => Ok(format!(
            "{summary}; the endpoint is the same bits with the far field off"
        )),
        [_, _, ..] if matches!(fixture, Fixture::Observers) => Ok(format!(
            "{summary}; the endpoint is the same bits with every observer on"
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
            report(&format!("e2e {fixture:?}: {summary}"));
            mark("pass", &summary);
            exit.write(AppExit::Success);
        }
        Err(failure) => {
            report(&format!("e2e {fixture:?} failed: {failure}"));
            mark("fail", &failure);
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

/// The cavity with its bottom side a second-order outgoing wall and, where
/// `driven`, its top side driven by [`WALL_DRIVE`]: the pulse's wave reaches
/// the wall before the handoff and leaves memory in its pole states, and the
/// drive has turned more than a period by step 192, a phase a restarted
/// clock would lose.
fn wall_document(driven: bool) -> TopologyDocument {
    let mut document = cavity_document();
    let mut walls = OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting);
    walls.sides[OuterSide::Bottom.index()] = OuterBoundaryCondition::SecondOrderOutgoing;
    if driven {
        walls.sides[OuterSide::Top.index()] =
            OuterBoundaryCondition::Dirichlet { signal: WALL_DRIVE };
    }
    document.model.draft.outer_boundaries = walls;
    document.model.accepted.outer_boundaries = walls;
    document
}

/// The driven side's signal: a harmonic at 0.8 Hz from rest, amplitude 0.3.
const WALL_DRIVE: TimeSignal = TimeSignal::harmonic(0.0, 0.3, 0.8, 0.0);

/// The flux side's signal, the rate of the flux it holds: a harmonic at 0.8 Hz,
/// amplitude 1.5, a flux of about 0.3.
const WALL_FLUX: TimeSignal = TimeSignal::harmonic(0.0, 1.5, 0.8, 0.0);

/// The cavity's medium made defocusing Kerr, as the Advanced editor's field
/// law row authors it - a negative cubic term needs a declared amplitude
/// bound - and its point source switched on at the (1, 1) mode's frequency,
/// which pumps the closed box until the field reaches the bound.
fn rejection_document() -> TopologyDocument {
    let mut document = cavity_document();
    for scene in [&mut document.model.draft, &mut document.model.accepted] {
        for material in &mut scene.materials {
            if material.id == DEFAULT_MATERIAL {
                material.mass_law.field = FieldLaw::Polynomial {
                    chi1: ScalarField::constant(0.0),
                    chi2: ScalarField::constant(-0.2),
                    amplitude_bound: Some(ScalarField::constant(REJECTION_BOUND)),
                };
            }
        }
    }
    document.model.source.enabled = true;
    document.model.source.signal = TimeSignal::harmonic(0.0, REJECTION_DRIVE, 0.354, 0.0);
    document
}

/// The rejection fixture's drive amplitude and its medium's amplitude bound:
/// the drive reaches the bound well into the run, not at its start. At this
/// drive the field peaks at 0.45 by step 4096, the Kerr term detuning the
/// resonance as it grows.
const REJECTION_DRIVE: f64 = 10.0;
const REJECTION_BOUND: f64 = 0.2;

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

/// The cavity with the switch fixture's disc a parametric pump on its mass
/// row: a medium driven from the run's start.
fn pump_document() -> TopologyDocument {
    let mut editor = TopologyEditor::from_document(cavity_document()).expect("the cavity opens");
    let material = editor.add_material().expect("a material to pump");
    let medium = editor
        .document
        .model
        .draft
        .material(material)
        .expect("the material added")
        .clone();
    let preset = law_presets()
        .iter()
        .find(|preset| preset.name == "Parametric pump" && preset.row == LawPresetRow::Mass)
        .expect("the parametric pump preset");
    editor
        .update_material(apply_law_preset(preset, &medium).expect("the preset applies"))
        .expect("the pumped medium");
    editor
        .create_closed_curve(
            PeriodicCubicSpline::rounded(Point2::new(0.15, -0.1), 0.3),
            ClosedCurvePurpose::Subdomain { material },
        )
        .expect("the disc");
    let mut document = editor.document;
    document.model.accepted = document.model.draft.clone();
    document
}

/// The cavity's whole medium the parametric pump on its mass row and its top
/// side driven by [`WALL_DRIVE`]: the pinned nodes lie on the pumped medium.
fn pumped_driven_document() -> TopologyDocument {
    let mut document = cavity_document();
    let mut walls = OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting);
    walls.sides[OuterSide::Top.index()] = OuterBoundaryCondition::Dirichlet { signal: WALL_DRIVE };
    document.model.draft.outer_boundaries = walls;
    document.model.accepted.outer_boundaries = walls;
    let mut editor = TopologyEditor::from_document(document).expect("the driven cavity opens");
    let medium = editor
        .document
        .model
        .draft
        .material(DEFAULT_MATERIAL)
        .expect("the default medium")
        .clone();
    let preset = law_presets()
        .iter()
        .find(|preset| preset.name == "Parametric pump" && preset.row == LawPresetRow::Mass)
        .expect("the parametric pump preset");
    editor
        .update_material(apply_law_preset(preset, &medium).expect("the preset applies"))
        .expect("the pumped medium");
    let mut document = editor.document;
    document.model.accepted = document.model.draft.clone();
    document
}

/// The parted cavity's baffle, from its bottom side to its top, and its
/// island's centre and radius.
const PARTED_BAFFLE: [Point2; 2] = [Point2 { x: 0.24, y: -1.0 }, Point2 { x: 0.4, y: 1.0 }];
const PARTED_ISLAND: (Point2, f64) = (Point2 { x: -0.4, y: 0.1 }, 0.44);

/// How far the device's handoff may move a part's total, relative to the
/// part's magnitude: f32's own reach. On 2026-11-01 the M1 Max read 5.5e-8;
/// with each component's share its area's ratio, as until that day, 1.1e-4,
/// with a wall's nodes taken from its other side 1.1e-3, and with every node
/// given the first component's correction 8.0e-4, where the reference, which
/// carries the field the device's way, read the first two within 5e-6.
const PARTS_TOLERANCE: f64 = 1.0e-6;

/// The cavity at edge 0.04 parted by a reflecting baffle along
/// [`PARTED_BAFFLE`] and an island of its own medium inside a reflecting loop
/// round [`PARTED_ISLAND`], drawn through the editor: three components, as
/// IV-10's box in `tests/invariants.rs`.
fn parted_document() -> TopologyDocument {
    let mut document = cavity_document();
    document.presentation.mesh_edge = 0.04;
    let mut editor = TopologyEditor::from_document(document).expect("the cavity opens");
    let settle = |editor: &mut TopologyEditor| {
        while editor.acceptance == crate::topology_editor::TopologyAcceptance::Pending {
            editor.validate_frame(64);
        }
    };
    settle(&mut editor);
    let domain = editor.document.model.draft.geometry.domain;
    let [bottom, top] = PARTED_BAFFLE;
    let anchor = |side, fraction| {
        Some(TopologyAttachment::Boundary(FaceAnchor::Outer {
            side,
            fraction,
        }))
    };
    editor
        .create_open_curve(
            OpenCubicSpline::polyline(PARTED_BAFFLE.to_vec()).expect("the baffle's line"),
            OpenCurvePurpose::BoundaryBaffle,
            anchor(
                OuterSide::Bottom,
                (bottom.x - domain.min_x) / domain.width(),
            ),
            // The top side runs right to left.
            anchor(OuterSide::Top, (domain.max_x - top.x) / domain.width()),
        )
        .expect("the baffle");
    settle(&mut editor);
    let (centre, radius) = PARTED_ISLAND;
    let island = editor
        .create_closed_curve(
            PeriodicCubicSpline::rounded(centre, radius),
            ClosedCurvePurpose::Subdomain {
                material: DEFAULT_MATERIAL,
            },
        )
        .expect("the island");
    settle(&mut editor);
    let spans = editor
        .document
        .model
        .draft
        .geometry
        .curves
        .iter()
        .find(|curve| curve.id == island)
        .expect("the island's loop")
        .spans
        .iter()
        .map(|span| span.id)
        .collect();
    editor
        .set_span_behavior(&spans, SpanBehavior::REFLECTING)
        .expect("the island's wall");
    let mut document = editor.document;
    document.model.accepted = document.model.draft.clone();
    document
}

/// [`Fixture::BaffleMove`]'s baffle, as fractions of the domain: IV-12's
/// arc in `tests/invariants.rs`, bulging up across the middle, its ends
/// free.
const BAFFLE_ARC: [[f64; 2]; 5] = [
    [0.3, 0.45],
    [0.4, 0.62],
    [0.5, 0.64],
    [0.6, 0.62],
    [0.7, 0.45],
];

/// The baffled cavity's edge, and how far its baffle is dragged up, in
/// edges.
const BAFFLED_EDGE: f64 = 0.04;
const BAFFLE_SHIFT: f64 = 1.0;

/// The step the baffle is dragged at: the pulse below it has come back off
/// it, the field jumping across it.
const BAFFLE_MOVE_STEP: u64 = 384;

/// How far from the baffle each side is read, in edges.
const BESIDE: f64 = 0.3;

/// How far the jump at the moved baffle may stand from each side's own
/// values on the device, relatively: IV-12's bound in `tests/invariants.rs`.
const MOVED_SIDES: f64 = 0.3;

/// How much of the jump may be left where the baffle was on the device,
/// relatively: IV-12's bound in `tests/invariants.rs`.
const LEFT_BEHIND: f64 = 0.1;

/// The cavity at [`BAFFLED_EDGE`] with a free-ended reflecting baffle along
/// [`BAFFLE_ARC`], drawn through the editor, as IV-12's box.
fn baffled_document() -> TopologyDocument {
    let mut document = cavity_document();
    document.presentation.mesh_edge = BAFFLED_EDGE;
    let mut editor = TopologyEditor::from_document(document).expect("the cavity opens");
    let settle = |editor: &mut TopologyEditor| {
        while editor.acceptance == crate::topology_editor::TopologyAcceptance::Pending {
            editor.validate_frame(64);
        }
    };
    settle(&mut editor);
    let domain = editor.document.model.draft.geometry.domain;
    let controls = BAFFLE_ARC
        .iter()
        .map(|[x, y]| {
            Point2::new(
                domain.min_x + x * domain.width(),
                domain.min_y + y * domain.height(),
            )
        })
        .collect();
    editor
        .create_boundary_baffle(
            OpenCubicSpline::new(controls, vec![1.0, 1.0]).expect("the baffle's arc"),
        )
        .expect("the baffle");
    settle(&mut editor);
    let mut document = editor.document;
    document.model.accepted = document.model.draft.clone();
    document
}

/// The baffle's curve: the document's only open one.
fn the_baffle(document: &TopologyDocument) -> Result<&funfern_core::TopologyCurve, String> {
    let mut open = document
        .model
        .draft
        .geometry
        .curves
        .iter()
        .filter(|curve| matches!(curve.spline, funfern_core::CurveSpline::Open(_)));
    match (open.next(), open.next()) {
        (Some(curve), None) => Ok(curve),
        _ => Err("the document has no one baffle".into()),
    }
}

/// [`Action::Move`]: the baffle's spans selected and dragged up
/// [`BAFFLE_SHIFT`] edges in one transaction, as the Move tool does.
fn move_the_baffle(editor: &mut TopologyEditor) -> Result<(), String> {
    let geometry = editor.document.model.draft.geometry.clone();
    let selected = the_baffle(&editor.document)?
        .spans
        .iter()
        .map(|span| crate::topology_viewport::TopologySpanTarget::Curve(span.id))
        .collect();
    let updates = crate::topology_viewport::plan_rigid_transform(
        &geometry,
        &selected,
        crate::topology_viewport::RigidTransform {
            pivot: Point2::default(),
            translation: Point2::new(0.0, BAFFLE_SHIFT * BAFFLED_EDGE),
            rotation_radians: 0.0,
            scale: 1.0,
        },
    )
    .map_err(|issue| issue.to_string())?;
    editor.apply_transform_updates(&updates)
}

/// 33 points along the baffle's middle, from a fifth of the way along it to
/// four fifths, each with the normal to its convex side, the curve's left.
fn along_the_baffle(document: &TopologyDocument) -> Result<Vec<(Point2, Point2)>, String> {
    let funfern_core::CurveSpline::Open(spline) = &the_baffle(document)?.spline else {
        return Err("the baffle is not open".into());
    };
    let end = spline
        .span_bounds(spline.controls().len() - 4)
        .ok_or("the baffle has no last span")?[1];
    Ok((0..33)
        .map(|index| {
            let t = end * (0.2 + 0.6 * index as f64 / 32.0);
            let tangent = spline.derivative(t, 1);
            let normal = Point2::new(-tangent.y, tangent.x) / tangent.norm();
            (spline.evaluate(t), normal)
        })
        .collect())
}

/// The display's full snapshot as `prepared`'s `Q` and `b`.
fn device_state(
    prepared: &PreparedTopology,
    display: &CanonicalGpuDisplay,
) -> Result<(Vec<f64>, Vec<Point2>), String> {
    let operator = &prepared.canonical_operator;
    if display.primary_flux.len() != operator.degrees_of_freedom()
        || display.complementary_flux.len() != operator.complementary_degrees_of_freedom()
    {
        return Err("the snapshot does not match the generation".into());
    }
    let primary = display
        .primary_flux
        .iter()
        .map(|value| f64::from(*value))
        .collect();
    let complementary = display
        .complementary_flux
        .iter()
        .map(|[x, y]| Point2::new(f64::from(*x), f64::from(*y)))
        .collect();
    Ok((primary, complementary))
}

/// What a point probe on `prepared` reads at `point` of the state `primary`
/// and `complementary`: the field and the complementary field.
fn read_at(
    prepared: &PreparedTopology,
    primary: &[f64],
    complementary: &[Point2],
    point: Point2,
) -> Result<(f64, Point2), String> {
    let stencil = funfern_core::QuadraticPointStencil::build_topology(
        &prepared.mesh,
        &prepared.operator,
        &prepared.bundle.plan,
        prepared.fixed_model(),
        point,
    )
    .map_err(|error| format!("no stencil at {point:?}: {error:?}"))?;
    funfern_core::CanonicalPointStencil::from_quadratic(stencil, &prepared.canonical_operator)
        .and_then(|stencil| stencil.sample(primary, primary, complementary))
        .map(|reading| (reading.primary, reading.complementary))
        .map_err(|error| format!("no reading at {point:?}: {error}"))
}

/// The device's `Q` and `b` jumps from the second point of each pair to the
/// first, read on `prepared` from the display's full snapshot.
fn device_jumps(
    prepared: &PreparedTopology,
    display: &CanonicalGpuDisplay,
    pairs: impl Iterator<Item = (Point2, Point2)>,
) -> Result<Vec<(f64, Point2)>, String> {
    let (primary, complementary) = device_state(prepared, display)?;
    let read = |point| read_at(prepared, &primary, &complementary, point);
    pairs
        .map(|(above, below)| {
            let (above, below) = (read(above)?, read(below)?);
            Ok((above.0 - below.0, above.1 - below.1))
        })
        .collect()
}

/// The RMS of `jumps`' differences from `meant`'s over the RMS of `against`,
/// in `Q` and in `b`; from none where `meant` is `None`.
fn jumps_apart(
    jumps: &[(f64, Point2)],
    meant: Option<&[(f64, Point2)]>,
    against: &[(f64, Point2)],
) -> [f64; 2] {
    let sum = |pairs: &mut dyn Iterator<Item = ((f64, Point2), (f64, Point2))>| {
        pairs.fold([0.0, 0.0], |[q, b], (one, other)| {
            [
                q + (one.0 - other.0).powi(2),
                b + (one.1 - other.1).dot(one.1 - other.1),
            ]
        })
    };
    let zero = vec![(0.0, Point2::default()); jumps.len().max(against.len())];
    let meant = meant.unwrap_or(&zero);
    let apart = sum(&mut jumps.iter().copied().zip(meant.iter().copied()));
    let size = sum(&mut against.iter().copied().zip(zero.iter().copied()));
    [0, 1].map(|index| (apart[index] / size[index]).sqrt())
}

/// What [`Fixture::BaffleMove`] reads off the device's state just before the
/// move: the baffle's points and normals, the jump at each, and the jump
/// each side's own values make at the moved baffle: the old field where it
/// stands above, the side the baffle moves away from, and beside the old
/// baffle below, the side that gains ground.
struct SidesBefore {
    points: Vec<(Point2, Point2)>,
    before: Vec<(f64, Point2)>,
    meant: Vec<(f64, Point2)>,
}

impl SidesBefore {
    fn read(
        active: &PreparedTopology,
        document: &TopologyDocument,
        display: &CanonicalGpuDisplay,
    ) -> Result<Self, String> {
        let points = along_the_baffle(document)?;
        let apart = BESIDE * BAFFLED_EDGE;
        let up = Point2::new(0.0, BAFFLE_SHIFT * BAFFLED_EDGE);
        let before = device_jumps(
            active,
            display,
            points
                .iter()
                .map(|(at, normal)| (*at + *normal * apart, *at - *normal * apart)),
        )?;
        let meant = device_jumps(
            active,
            display,
            points
                .iter()
                .map(|(at, normal)| (*at + up + *normal * apart, *at - *normal * apart)),
        )?;
        Ok(Self {
            points,
            before,
            meant,
        })
    }

    /// At the handoff, on the new generation: the jump at the moved baffle
    /// against each side's own values, and the jump left where the baffle
    /// was against the jump before; each in `Q` and in `b`.
    fn after(
        &self,
        target: &PreparedTopology,
        display: &CanonicalGpuDisplay,
    ) -> Result<([f64; 2], [f64; 2]), String> {
        let apart = BESIDE * BAFFLED_EDGE;
        let up = Point2::new(0.0, BAFFLE_SHIFT * BAFFLED_EDGE);
        let after = device_jumps(
            target,
            display,
            self.points
                .iter()
                .map(|(at, normal)| (*at + up + *normal * apart, *at + up - *normal * apart)),
        )?;
        // Read within the ground the baffle swept.
        let near = apart.min(BESIDE * up.y);
        let behind = device_jumps(
            target,
            display,
            self.points
                .iter()
                .map(|(at, normal)| (*at + *normal * near, *at - *normal * near)),
        )?;
        Ok((
            jumps_apart(&after, Some(&self.meant), &self.meant),
            jumps_apart(&behind, None, &self.before),
        ))
    }
}

/// [`Fixture::Observers`]' point probe, where the pulse's wave passes.
const OBSERVED_PROBE: ProbeId = ProbeId(1);
const OBSERVED_POINT: Point2 = Point2 { x: 0.3, y: -0.2 };

/// The step the observed run is paused at, once it has reached it running,
/// and for how long.
const PAUSE_AT: u64 = 96;
const PAUSE_FOR: std::time::Duration = std::time::Duration::from_secs(1);

/// How long the observed run's endpoint waits for the vector overlay's first
/// readback before it reads the overlay as having taken nothing.
const OVERLAY_WAIT: std::time::Duration = std::time::Duration::from_secs(60);

/// The step the observed run's recording stops and its window is resized
/// and zoomed at, once it has reached it running, the window's new logical
/// size, and the zoom. A window manager may refuse the resize, as a tiling
/// one does; the zoom, as Cmd+= sets it, changes the viewport whatever it
/// does.
const RESIZE_AT: u64 = 160;
const RESIZED: [f32; 2] = [1040.0, 680.0];
const ZOOMED: f32 = 1.25;

/// How long the recording must have run when it stops, and, natively, how
/// many frame slots it must have asked for by then: half its rate's worth.
const RECORDED_FOR: std::time::Duration = std::time::Duration::from_secs(1);
const RECORDED_SLOTS: u64 = 30;

/// How long the recording may take to be written once stopped.
const RECORDING_FINISH: std::time::Duration = std::time::Duration::from_secs(15);

/// The cavity with [`OBSERVED_PROBE`], a point probe, whose readout window
/// the observed run opens; probes only read, as `observables` holds.
fn observers_document() -> TopologyDocument {
    let mut document = cavity_document();
    document.model.probes = vec![crate::topology_editor::TopologyProbeDefinition {
        id: OBSERVED_PROBE,
        name: "Observed".into(),
        color: [91, 220, 194],
        enabled: true,
        target: crate::topology_editor::TopologyProbeTarget::Point(OBSERVED_POINT),
    }];
    document
}

/// Where the observed run records to natively: the run's own home, which
/// the fixture's runner makes for it. The encoder adds its extension.
#[cfg(not(target_arch = "wasm32"))]
fn recording_path() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("funfern-observers")
}

/// Whether the observed run's recording was written natively: a file of
/// [`recording_path`]'s name, whatever its extension, that is not empty.
#[cfg(not(target_arch = "wasm32"))]
fn recording_written() -> Result<(), String> {
    let path = recording_path();
    let folder = path.parent().ok_or("the recording has no folder")?;
    let written = std::fs::read_dir(folder)
        .map_err(|error| format!("the recording's folder does not open: {error}"))?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().file_stem() == path.file_name())
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| metadata.len())
        .max();
    match written {
        Some(bytes) if bytes > 0 => Ok(()),
        _ => Err(format!("the recording wrote nothing at {}", path.display())),
    }
}

/// The observed run's pause, the recording's stop and the resize and zoom,
/// each taken once the run has reached its step running. The pause must hold
/// the device's step count through its second half, and all three must
/// come before the endpoint.
#[allow(clippy::too_many_arguments)]
fn observe_mid_run(
    observing: &mut Observing,
    state: &mut Playground,
    windows: &mut Query<&mut Window, With<bevy::window::PrimaryWindow>>,
    context: Option<&egui::Context>,
    completed: u64,
    asked: u64,
    endpoint: u64,
) -> Result<(), String> {
    match observing.pause {
        None if completed >= PAUSE_AT => {
            state.wave_running = false;
            observing.pause = Some((asked, None));
        }
        // The steps asked for before the pause may still be completing: on
        // SwiftShader in CI they were half a second into it, which read as
        // stepping on (2026-11-07). Paused, the app asks for none, the device
        // does none past them, and the second is timed from when it has done
        // them.
        Some((before, done)) if observing.paused_at.is_none() => {
            if asked != before {
                return Err(format!(
                    "paused, the app asked for steps from {before} to {asked}"
                ));
            }
            if completed > before {
                return Err(format!(
                    "paused, the device stepped on from {before} to {completed}"
                ));
            }
            match done {
                None if completed == before => {
                    observing.pause = Some((before, Some(Instant::now())));
                }
                Some(done) if done.elapsed() >= PAUSE_FOR => {
                    observing.paused_at = Some(completed);
                    state.wave_running = true;
                }
                _ => {}
            }
        }
        _ => {}
    }
    if observing.paused_at.is_some() && observing.resized_from.is_none() && completed >= RESIZE_AT {
        let Some(started) = state
            .recording_started
            .filter(|_| state.recording_state == RecordingState::Recording)
        else {
            return Err(format!(
                "the recording was {:?} at step {completed}",
                state.recording_state
            ));
        };
        #[cfg(not(target_arch = "wasm32"))]
        let slots = state.recording_last_requested_slot;
        #[cfg(target_arch = "wasm32")]
        let slots = None;
        observing.recorded = Some((started.elapsed(), slots));
        state.stop_video_recording();
        observing.resized_from = Some(state.viewport_rect.size());
        resize(windows)?;
        let context = context.ok_or("no egui context to zoom")?;
        context.set_zoom_factor(context.zoom_factor() * ZOOMED);
    }
    if completed >= endpoint && observing.resized_from.is_none() {
        return Err(format!(
            "the run reached step {completed} before its {}",
            if observing.paused_at.is_none() {
                "pause was over"
            } else {
                "resize"
            }
        ));
    }
    Ok(())
}

/// Resizes the window, as dragging its corner does; in the browser, where
/// the canvas follows its page, the page's body.
fn resize(
    windows: &mut Query<&mut Window, With<bevy::window::PrimaryWindow>>,
) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut window = windows
            .single_mut()
            .map_err(|error| format!("no window to resize: {error}"))?;
        window.resolution.set(RESIZED[0], RESIZED[1]);
        Ok(())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = windows;
        let body = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.body())
            .ok_or("the page has no body to resize")?;
        body.set_attribute(
            "style",
            &format!("width: {}px; height: {}px", RESIZED[0], RESIZED[1]),
        )
        .map_err(|error| format!("the page did not resize: {error:?}"))
    }
}

/// What the observed run's observers show they did, for the summary, or
/// the first that did not: an observer that never took would prove
/// nothing.
fn observed(
    state: &Playground,
    overlay: &VectorOverlayDisplay,
    observing: &Observing,
    generation: u64,
) -> Result<String, String> {
    if overlay.generation != generation || overlay.samples.is_empty() || overlay.readbacks == 0 {
        return Err("the vector overlay read nothing back".into());
    }
    let Some(level) = state
        .field_exposure
        .reference()
        .filter(|_| state.editor.document.presentation.field_auto_exposure)
    else {
        return Err("auto exposure set no level".into());
    };
    let Some(view) = state
        .probe_views
        .get(&OBSERVED_PROBE)
        .filter(|view| view.end_time > 0.0)
    else {
        return Err("the probe's readout window was never drawn with a sample".into());
    };
    let Some((recorded, slots)) = observing.recorded else {
        return Err("the recording was never stopped".into());
    };
    if recorded < RECORDED_FOR || slots.is_some_and(|slots| slots < RECORDED_SLOTS) {
        return Err(format!(
            "the recording ran {:.2} s and asked for {slots:?} frames",
            recorded.as_secs_f64()
        ));
    }
    let (Some(paused), Some(before)) = (observing.paused_at, observing.resized_from) else {
        return Err("the run was not paused and resized".into());
    };
    let after = state.viewport_rect.size();
    if after == before {
        return Err(format!("the viewport stayed {before:?} through the resize"));
    }
    Ok(format!(
        "; the overlay read {} samples, auto exposure a level of {level:.3e}, the readout \
         window its probe to {:.3} s; recorded {:.1} s{}, paused at step {paused}, the \
         viewport {:.0}x{:.0} resized and zoomed to {:.0}x{:.0}",
        overlay.samples.len(),
        view.end_time,
        recorded.as_secs_f64(),
        slots.map_or(String::new(), |slots| format!(" in {slots} frame slots")),
        before.x,
        before.y,
        after.x,
        after.y
    ))
}

/// How many times sooner [`Fixture::Similarity`]'s second run is: a power of
/// four, so that every quantity, and the square root of the wall's damping
/// its modal transform takes, scales by a power of two.
const SOONER: f64 = 4.0;

/// [`Fixture::Similarity`]'s second run: the cavity [`SOONER`] times sooner,
/// its medium's density divided by the square, and its drive that much
/// faster and, the field it holds coming that much stronger, stronger. The
/// cavity carries no other rate.
fn sooner(document: &mut TopologyDocument) {
    let TimeSignal::Harmonic {
        offset,
        amplitude,
        frequency_hz,
        phase_radians,
    } = WALL_DRIVE
    else {
        unreachable!("the wall's drive is harmonic");
    };
    for scene in [&mut document.model.draft, &mut document.model.accepted] {
        assert_eq!(
            scene.physics,
            PhysicsModel::Mechanical,
            "the density is the primary row"
        );
        for material in &mut scene.materials {
            let ScalarField::Constant(density) = material.mass_density else {
                unreachable!("the cavity's density is constant");
            };
            material.mass_density = ScalarField::Constant(density / (SOONER * SOONER));
        }
        scene.outer_boundaries.sides[OuterSide::Top.index()] = OuterBoundaryCondition::Dirichlet {
            signal: TimeSignal::Harmonic {
                offset: offset * SOONER,
                amplitude: amplitude * SOONER,
                frequency_hz: frequency_hz * SOONER,
                phase_radians,
            },
        };
    }
}

/// [`Fixture::Similarity`]'s two endpoints: the sooner run's step the
/// first's divided by [`SOONER`], and its Q times [`SOONER`], b and the
/// wall's pole states as they are, the first's, value for value.
fn similar(endpoints: &[[Vec<f32>; 3]], time_steps: &[f64]) -> Result<String, String> {
    let ([first, sooner], [step, sooner_step]) = (endpoints, time_steps) else {
        return Err(format!(
            "{} endpoints and {} steps, where the runs are two",
            endpoints.len(),
            time_steps.len()
        ));
    };
    if *sooner_step * SOONER != *step {
        return Err(format!(
            "the sooner run steps {sooner_step:e}, not the first's {step:e} over {SOONER}"
        ));
    }
    let mut read = Vec::new();
    for (index, (name, factor)) in [
        ("Q", SOONER as f32),
        ("b", 1.0),
        ("the wall's pole states", 1.0),
    ]
    .into_iter()
    .enumerate()
    {
        let (first, sooner) = (&first[index], &sooner[index]);
        if first.len() != sooner.len() || first.is_empty() {
            return Err(format!(
                "the runs hold {} and {} values of {name}",
                first.len(),
                sooner.len()
            ));
        }
        // A zero of either sign is itself scaled.
        let parted = first
            .iter()
            .zip(sooner)
            .filter(|(first, sooner)| **sooner * factor != **first)
            .count();
        if parted != 0 {
            return Err(format!(
                "{parted} of {} values of {name} in the sooner run are not the first run's{}: \
                 the device is not homogeneous in time",
                first.len(),
                if factor == 1.0 {
                    String::new()
                } else {
                    format!(" over {SOONER}")
                }
            ));
        }
        read.push(format!("{} of {name}", first.len()));
    }
    Ok(format!(
        "the sooner run is the first, its step and every value: {}",
        read.join(", ")
    ))
}

/// [`Fixture::Superposition`]'s four runs: which of its two points are
/// struck, the pulses' amplitude, which the pulse slider takes to -5, and
/// the factor on [`WALL_DRIVE`]'s amplitude. The first run is the sum of the
/// next two, and the last the first negated.
const SUPERPOSED: [(&[usize], f32, f64); 4] = [
    (&[0, 1], 1.0, 1.0),
    (&[0], 1.0, 1.0),
    (&[1], 1.0, 0.0),
    (&[0, 1], -1.0, -1.0),
];

/// How far the first of [`SUPERPOSED`]'s runs may stand from the sum of the
/// next two at the endpoint, relative to the largest of the three, in each
/// of Q, b and the wall's pole states: the device's own rounding, which is
/// not linear.
const SUPERPOSED_RESIDUAL: f64 = 1.0e-5;

/// [`Fixture::Superposition`]'s four endpoints against each other: the
/// first less the next two, relative to the largest of the three, within
/// [`SUPERPOSED_RESIDUAL`], and the last the first negated, value for value,
/// in Q, b and the wall's pole states.
fn superposed(endpoints: &[[Vec<f32>; 3]]) -> Result<String, String> {
    let [sum, struck, driven, negated] = endpoints else {
        return Err(format!(
            "{} endpoints, where the runs are four",
            endpoints.len()
        ));
    };
    let norm = |values: &[f32]| {
        values
            .iter()
            .map(|value| f64::from(*value).powi(2))
            .sum::<f64>()
            .sqrt()
    };
    let mut read = Vec::new();
    for (index, name) in ["Q", "b", "the wall's pole states"].into_iter().enumerate() {
        let (sum, struck, driven, negated) =
            (&sum[index], &struck[index], &driven[index], &negated[index]);
        if struck.len() != sum.len() || driven.len() != sum.len() || negated.len() != sum.len() {
            return Err(format!("the runs hold {name} of different lengths"));
        }
        if sum.is_empty() {
            return Err(format!("the runs hold no {name}"));
        }
        let residual = sum
            .iter()
            .zip(struck)
            .zip(driven)
            .map(|((sum, struck), driven)| {
                (f64::from(*sum) - f64::from(*struck) - f64::from(*driven)).powi(2)
            })
            .sum::<f64>()
            .sqrt()
            / norm(sum).max(norm(struck)).max(norm(driven));
        // A zero of either sign is its negation.
        let parted = sum
            .iter()
            .zip(negated)
            .filter(|(sum, negated)| **negated != -**sum)
            .count();
        if !within(residual, SUPERPOSED_RESIDUAL) {
            return Err(format!(
                "{name} stands {residual:.3e} from the sum of its parts' runs, beyond \
                 {SUPERPOSED_RESIDUAL:.0e}"
            ));
        }
        if parted != 0 {
            return Err(format!(
                "{parted} of {} values of {name} in the negated run are not the first run's \
                 negated: the device is not odd",
                sum.len()
            ));
        }
        read.push(format!("{name} {residual:.3e}"));
    }
    Ok(format!(
        "the sum of its parts' runs stands {}; the negated run is the first negated, every \
         value",
        read.join(", ")
    ))
}

/// [`Fixture::SeparatorMidRun`]'s cavity edge.
const SEPARATED_EDGE: f64 = 0.04;

/// IV-08's loop in `tests/invariants.rs`, its centre and radius: round part
/// of the cavity beside the pulse, which has spread through it by step 128.
const SEPARATOR_LOOP: (Point2, f64) = (Point2 { x: 0.21, y: 0.13 }, 0.33);

/// How far the device's endpoint may stand from the unedited run, relatively,
/// in `Q` and in `b`, read at a grid of points: IV-08's bound in
/// `tests/invariants.rs`, one halving's move, here the unedited run's own
/// move from twice the edge, 4.6e-4 and 8.9e-4 in f64 on 2026-11-02. The
/// edited run read 1.5e-4 and 7.1e-5 from the unedited in f64 that day; with
/// a new face's complementary field taken only from its own region, as
/// until 2026-10-06, 0.30 and 0.36.
const UNEDITED_APART: [f64; 2] = [4.6e-4, 8.9e-4];

/// The points across the cavity the endpoint is read at, a side's count:
/// the centres of a grid of cells.
const UNEDITED_GRID: usize = 24;

/// The cavity at [`SEPARATED_EDGE`] in IV-08's medium, as IV-08's box: a
/// density graded along the frame's first axis and an axis ratio of two, the
/// frame turned by 0.4, which a face the loop makes must carry over as it
/// is.
fn separated_document() -> TopologyDocument {
    let mut document = cavity_document();
    document.presentation.mesh_edge = SEPARATED_EDGE;
    for scene in [&mut document.model.draft, &mut document.model.accepted] {
        for material in &mut scene.materials {
            material.mass_density = ScalarField::formula("1.7 + 0.3 * x").expect("IV-08's density");
            material.axis_ratio = ScalarField::constant(2.0);
        }
        for region in &mut scene.regions {
            region.frame.angle_radians = 0.4;
        }
    }
    document
}

/// The loop: the document's only closed curve.
fn the_loop(document: &TopologyDocument) -> Result<CurveId, String> {
    let mut closed = document
        .model
        .draft
        .geometry
        .curves
        .iter()
        .filter(|curve| matches!(curve.spline, funfern_core::CurveSpline::Closed(_)));
    match (closed.next(), closed.next()) {
        (Some(curve), None) => Ok(curve.id),
        _ => Err("the document has no one loop".into()),
    }
}

/// [`Action::Draw`]: [`SEPARATOR_LOOP`] drawn as a subdomain of the
/// cavity's medium.
fn draw_the_loop(editor: &mut TopologyEditor) -> Result<(), String> {
    let (centre, radius) = SEPARATOR_LOOP;
    editor
        .create_closed_curve(
            PeriodicCubicSpline::rounded(centre, radius),
            ClosedCurvePurpose::Subdomain {
                material: DEFAULT_MATERIAL,
            },
        )
        .map(|_| ())
}

/// [`Action::Delete`]: the loop removed, the face it merges with keeping
/// the background's region, the one the loop only divided.
fn delete_the_loop(editor: &mut TopologyEditor) -> Result<(), String> {
    let curve = the_loop(&editor.document)?;
    let choices = editor.curve_removal_choices(curve)?;
    let keep = match choices.as_slice() {
        [] => None,
        [only] => Some(*only),
        _ if choices.contains(&BACKGROUND_REGION) => Some(BACKGROUND_REGION),
        _ => {
            return Err(format!(
                "the deletion merges {choices:?}, not the background"
            ));
        }
    };
    editor.remove_curve(curve, keep).map(|_| ())
}

/// How far the device's state at `time`, on `target`, stands from the
/// unedited run `held` on `plain` stepped on to `time` in whole steps no
/// longer than its own, relatively, at a grid of [`UNEDITED_GRID`] points a
/// side: in `Q` and in `b`.
fn unedited_apart(
    plain: &PreparedTopology,
    held: &State,
    target: &PreparedTopology,
    display: &CanonicalGpuDisplay,
    time: f64,
) -> Result<[f64; 2], String> {
    let State::Fixed(held) = held else {
        return Err("a time-varying unedited run".into());
    };
    let rest = time - held.time();
    let steps = (rest / held.time_step()).ceil().max(1.0);
    let operator = &plain.canonical_operator;
    let mut unedited = CanonicalWaveState::new(
        operator,
        rest / steps,
        held.primary_flux().to_vec(),
        held.complementary_flux().to_vec(),
    )
    .and_then(|state| state.with_time_origin(held.time()))
    .map_err(|error| error.to_string())?;
    for _ in 0..steps as u64 {
        unedited
            .step_with_forcing(operator, &plain.canonical_forcing)
            .map_err(|error| format!("the unedited run did not step: {error}"))?;
    }
    let (primary, complementary) = device_state(target, display)?;
    let domain = plain.bundle.authored.geometry.domain;
    let mut sums = [0.0; 4];
    for index in 0..UNEDITED_GRID * UNEDITED_GRID {
        let point = Point2::new(
            domain.min_x
                + domain.width() * ((index % UNEDITED_GRID) as f64 + 0.5) / UNEDITED_GRID as f64,
            domain.min_y
                + domain.height() * ((index / UNEDITED_GRID) as f64 + 0.5) / UNEDITED_GRID as f64,
        );
        let device = read_at(target, &primary, &complementary, point)?;
        let meant = read_at(
            plain,
            unedited.primary_flux(),
            unedited.complementary_flux(),
            point,
        )?;
        sums[0] += (device.0 - meant.0).powi(2);
        sums[1] += meant.0 * meant.0;
        sums[2] += (device.1 - meant.1).dot(device.1 - meant.1);
        sums[3] += meant.1.dot(meant.1);
    }
    Ok([(sums[0] / sums[1]).sqrt(), (sums[2] / sums[3]).sqrt()])
}

/// Each of the parted cavity's parts' totals of `primary` and of its
/// magnitude: 0 left of [`PARTED_BAFFLE`], 1 right of it, 2 inside the
/// island's loop. A node takes the part the centres of its elements lie in,
/// which tells the two copies of a wall's node apart.
fn part_totals(
    operator: &CanonicalWaveOperator,
    primary: &[f32],
) -> Result<([f64; 3], [f64; 3]), String> {
    if primary.len() != operator.degrees_of_freedom() {
        return Err(format!(
            "the snapshot holds {} values where the generation has {}",
            primary.len(),
            operator.degrees_of_freedom()
        ));
    }
    let (centre, radius) = PARTED_ISLAND;
    let spline = PeriodicCubicSpline::rounded(centre, radius);
    let island = (0..4096)
        .map(|index| spline.evaluate(spline.period() * index as f64 / 4096.0))
        .collect::<Vec<_>>();
    let [from, to] = PARTED_BAFFLE;
    let part = |point: Point2| {
        let mut inside = false;
        for (index, a) in island.iter().enumerate() {
            let b = island[(index + 1) % island.len()];
            if (a.y > point.y) != (b.y > point.y)
                && point.x < a.x + (point.y - a.y) * (b.x - a.x) / (b.y - a.y)
            {
                inside = !inside;
            }
        }
        if inside {
            2
        } else if (to - from).cross(point - from) > 0.0 {
            0
        } else {
            1
        }
    };
    let mut owner = vec![usize::MAX; operator.degrees_of_freedom()];
    for nodes in operator.element_nodes() {
        let centre = nodes.iter().fold(Point2::default(), |sum, node| {
            sum + operator.node_points()[*node as usize]
        }) / nodes.len() as f64;
        let part = part(centre);
        for node in nodes {
            let slot = &mut owner[*node as usize];
            if *slot != usize::MAX && *slot != part {
                return Err(format!("node {node} lies in parts {} and {part}", *slot));
            }
            *slot = part;
        }
    }
    let (mut totals, mut sizes) = ([0.0; 3], [0.0; 3]);
    for (part, value) in owner.iter().zip(primary) {
        totals[*part] += f64::from(*value);
        sizes[*part] += f64::from(*value).abs();
    }
    Ok((totals, sizes))
}

/// Advances the fixture a phase; answers a run's verdict once it has one.
#[allow(clippy::too_many_arguments)]
fn advance(
    driver: &mut Driver,
    state: &mut Playground,
    request: &mut CanonicalGpuRequest,
    display: &CanonicalGpuDisplay,
    overlay: &VectorOverlayDisplay,
    windows: &mut Query<&mut Window, With<bevy::window::PrimaryWindow>>,
    context: Option<&egui::Context>,
    stopped: Option<&crate::stopped::RenderStopped>,
    lose: &mut LoseDevice,
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
            // Every fresh generation starts at zero but this one's.
            *FRESH_CLOCK.lock().unwrap() = match driver.fixture {
                Fixture::FarFieldLate => Some((
                    FreshOrigin::At(far_field::LATE_ORIGIN),
                    far_field::LATE_STEPS_BEFORE_REBASE,
                )),
                Fixture::HandoffMemory(MemoryHandoff::RunningLate) => {
                    Some((FreshOrigin::At(far_field::LATE_ORIGIN), 300))
                }
                _ => None,
            };
            let (mut document, material) = driver.fixture.document();
            if !driver.hashes.is_empty() {
                // The run again with nothing recording: a recorder only reads.
                match driver.fixture {
                    Fixture::Observables => document.model.probes.clear(),
                    Fixture::FarField => document.model.far_field.enabled = false,
                    _ => {}
                }
            }
            if matches!(driver.fixture, Fixture::Superposition) {
                let factor = SUPERPOSED[driver.hashes.len()].2;
                let signal = TimeSignal::harmonic(0.0, 0.3 * factor, 0.8, 0.0);
                for scene in [&mut document.model.draft, &mut document.model.accepted] {
                    scene.outer_boundaries.sides[OuterSide::Top.index()] =
                        OuterBoundaryCondition::Dirichlet { signal };
                }
            }
            if matches!(driver.fixture, Fixture::Similarity) && !driver.hashes.is_empty() {
                sooner(&mut document);
            }
            // Auto exposure is the app's default; the quiet run turns it off.
            let observed = driver.fixture.observes() && !driver.hashes.is_empty();
            if driver.fixture.observes() {
                let physics = document.model.accepted.physics;
                let presentation = &mut document.presentation;
                presentation.field_auto_exposure = observed;
                presentation.vector_overlay = if observed {
                    crate::document::VectorOverlay::choices(physics)[1]
                } else {
                    crate::document::VectorOverlay::Off
                };
            }
            driver.material = material;
            if let Err(error) = state.set_document(document, false, true) {
                return Some(Err(format!("the document did not open: {error}")));
            }
            driver.observing = Observing::default();
            if observed {
                state.probe_windows.insert(OBSERVED_PROBE);
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
            if let Err(failure) = same_mesh(&active.mesh, driver.fixture.mesh()) {
                return Some(Err(failure));
            }
            let temporal = active.canonical_temporal_operator.is_some();
            let expected = driver.material.is_some()
                || matches!(
                    driver.fixture,
                    Fixture::Rejection | Fixture::HandoffDriven | Fixture::HandoffDrivenPins
                );
            if temporal != expected {
                return Some(Err(format!(
                    "the document prepared a {} generation",
                    if temporal { "time-varying" } else { "fixed" }
                )));
            }
            // The observed run records from its start: the recording is asked
            // for, and running, before the pulse is placed.
            if driver.fixture.observes()
                && !driver.hashes.is_empty()
                && state.recording_state != RecordingState::Recording
            {
                if !driver.observing.recording_asked {
                    driver.observing.recording_asked = true;
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        *crate::recording::FIXTURE_DESTINATION.lock().unwrap() =
                            Some(recording_path());
                    }
                    state.request_video_recording();
                }
                if state.recording_state == RecordingState::Idle {
                    return Some(Err(format!(
                        "the recording did not start: {}",
                        state.message
                    )));
                }
                driver.phase = Phase::Install;
                return None;
            }
            let time_step = state.coordinator.uploaded_time_step;
            if !driver.fixture.pulses() {
                // Driven by its source from rest: the run starts at once.
                let reference = fresh_clock(time_step)
                    .and_then(|clock| State::zero(&active, time_step)?.starting_at(clock.time()));
                let reference = match reference {
                    Ok(reference) => reference,
                    Err(error) => {
                        return Some(Err(format!("the reference did not start: {error}")));
                    }
                };
                report(&format!(
                    "e2e {:?}: {} DOFs, step {time_step:.4e}, driven by its source, mesh {}",
                    driver.fixture,
                    active.canonical_operator.degrees_of_freedom(),
                    mesh_hashes(&active.mesh),
                ));
                let twin = if matches!(driver.fixture, Fixture::Observables)
                    && !state.editor.document.model.probes.is_empty()
                {
                    match observables::Twin::new(
                        &active,
                        &state.editor.document.model.probes,
                        time_step,
                    ) {
                        Ok(twin) => Some(twin),
                        Err(error) => {
                            return Some(Err(format!("the probes' twins did not start: {error}")));
                        }
                    }
                } else {
                    None
                };
                let far = if matches!(driver.fixture, Fixture::FarField | Fixture::FarFieldLate)
                    && state.editor.document.model.far_field.enabled
                {
                    let clock = match fresh_clock(time_step) {
                        Ok(clock) => clock,
                        Err(error) => return Some(Err(error)),
                    };
                    match far_field::FarFieldTwin::with_clock(
                        &active,
                        time_step,
                        clock.epoch_origin_seconds,
                        clock.step_in_epoch,
                        rebase_step_limit(time_step),
                    ) {
                        Ok(far) => Some(far),
                        Err(error) => {
                            return Some(Err(format!(
                                "the far field's twin did not start: {error}"
                            )));
                        }
                    }
                } else {
                    None
                };
                driver.base = request.stats().completed_steps();
                driver.start_time = reference.time();
                state.e2e_steps.limit = Some(driver.base + stop(0));
                state.e2e_steps.batch = driver.batches.front().copied().flatten();
                state.e2e_steps.most = 0;
                driver.phase = Phase::Run {
                    reference: Reference {
                        active,
                        state: reference,
                        control: None,
                        steps: 0,
                        carry: None,
                        departed: None,
                        sides: None,
                        unedited: None,
                        twin,
                        far,
                        maintenance_seen: 0,
                        settled: None,
                        generation: request.generation(),
                        timeline: BTreeMap::new(),
                        replayed: 0,
                    },
                    next: 0,
                };
                return None;
            }
            let domain = state.editor.document.model.accepted.geometry.domain;
            if let Some(width) = driver.fixture.pulse_width() {
                state.pulse_width = width;
            }
            let mut points = driver.fixture.pulses_at(domain);
            if matches!(driver.fixture, Fixture::Superposition) {
                let (struck, amplitude, _) = SUPERPOSED[driver.hashes.len()];
                points = struck.iter().map(|index| points[*index]).collect();
                state.pulse_amplitude = amplitude;
            }
            if matches!(driver.fixture, Fixture::Similarity) && !driver.hashes.is_empty() {
                state.pulse_amplitude = SOONER as f32;
            }
            let queued_before = state.coordinator.pending_pulses.len();
            for point in &points {
                state.place_pulse(*point);
            }
            if state.coordinator.pending_pulses.len() != queued_before + points.len() {
                return Some(Err(format!("a pulse was not queued: {}", state.message)));
            }
            let queued = state
                .coordinator
                .pending_pulses
                .iter()
                .skip(queued_before)
                .copied()
                .collect::<Vec<_>>();
            let reference = fresh_clock(time_step).and_then(|clock| {
                let mut reference = State::zero(&active, time_step)?.starting_at(clock.time())?;
                for (position, region) in &queued {
                    let increment = pulse_increment(
                        &active,
                        *position,
                        *region,
                        f64::from(state.pulse_amplitude),
                        f64::from(state.pulse_width),
                    )?;
                    reference.pulse(&active, &increment)?;
                }
                Ok(reference)
            });
            let reference = match reference {
                Ok(reference) => reference,
                Err(error) => return Some(Err(format!("the reference did not start: {error}"))),
            };
            driver.base = request.stats().completed_steps();
            driver.start_time = reference.time();
            report(&format!(
                "e2e {:?}: {} DOFs, step {time_step:.4e}, pulse at {}, mesh {}",
                driver.fixture,
                active.canonical_operator.degrees_of_freedom(),
                queued
                    .iter()
                    .map(|(position, _)| format!("({:.3}, {:.3})", position.x, position.y))
                    .collect::<Vec<_>>()
                    .join(" and "),
                mesh_hashes(&active.mesh)
            ));
            driver.phase = Phase::Pulse {
                reference: Reference {
                    active,
                    state: reference,
                    control: None,
                    steps: 0,
                    carry: None,
                    departed: None,
                    sides: None,
                    unedited: None,
                    twin: None,
                    far: None,
                    maintenance_seen: 0,
                    settled: None,
                    generation: request.generation(),
                    timeline: BTreeMap::new(),
                    replayed: 0,
                },
                processed: request.stats().processed_event(),
            };
            None
        }
        Phase::Pulse {
            reference,
            processed,
        } => {
            // Neither waiting, nor on its way, nor taken: the app dropped it.
            if request.stats().processed_event() == processed
                && state.coordinator.pending_pulses.is_empty()
                && !request.live_event_pending()
            {
                return Some(Err(format!("the app dropped the pulse: {}", state.message)));
            }
            if request.stats().processed_event() == processed
                || !state.coordinator.pending_pulses.is_empty()
                || request.live_event_pending()
            {
                driver.phase = Phase::Pulse {
                    reference,
                    processed,
                };
                return None;
            }
            if let Err(failure) = taken(request, "the pulse", driver.base) {
                return Some(Err(failure));
            }
            state.e2e_steps.limit = Some(driver.base + stop(0));
            state.e2e_steps.batch = driver.batches.front().copied().flatten();
            state.e2e_steps.most = 0;
            let mut reference = reference;
            // The events fired from here on are the timeline's; the run's
            // own pulse the reference took at the start.
            if !driver.events.bursts.is_empty() {
                let from = request.stats().processed_event();
                driver.events.from = Some(from);
                reference.replayed = from;
            }
            driver.phase = Phase::Run { reference, next: 0 };
            None
        }
        Phase::Run {
            mut reference,
            next,
        } => {
            let target = stop(next);
            let completed = request
                .stats()
                .completed_steps()
                .saturating_sub(driver.base);
            let code = request.stats().failure();
            if code != 0 {
                if !matches!(driver.fixture, Fixture::Rejection) {
                    return Some(Err(format!(
                        "the device failed after step {completed} (failure code {code})"
                    )));
                }
                driver.phase = Phase::Refused {
                    reference,
                    step: completed,
                    code,
                    seen: display.full_readbacks,
                    first: None,
                    retried: false,
                };
                return None;
            }
            if driver.fixture.observes()
                && !driver.hashes.is_empty()
                && let Err(failure) = observe_mid_run(
                    &mut driver.observing,
                    state,
                    windows,
                    context,
                    completed,
                    request.requested_steps().saturating_sub(driver.base),
                    target,
                )
            {
                return Some(Err(failure));
            }
            if completed < target {
                driver.phase = Phase::Run { reference, next };
                return None;
            }
            if completed > target {
                return Some(Err(format!(
                    "the run went past its stop at {target}, to {completed}"
                )));
            }
            // A batching the frames never asked for compares one schedule
            // with itself: past what a frame may ask, 128 and 256 both ran 64
            // a frame (VER-40 of the 2026-10-20 review).
            if let Some(batch) = state.e2e_steps.batch
                && state.e2e_steps.most != batch
            {
                return Some(Err(format!(
                    "asked for batches of {batch}, the frames asked for at most {}",
                    state.e2e_steps.most
                )));
            }
            // The rejection's endpoint is a ceiling: reaching it with nothing
            // refused is its failure, where it passed as a plain comparison
            // when neither side refused (VER-9 of the 2026-10-20 review).
            if matches!(driver.fixture, Fixture::Rejection) {
                return Some(Err(format!("reached step {target} without a refusal")));
            }
            if let Err(error) = reference.advance_to(target) {
                return Some(Err(format!("the reference did not step: {error}")));
            }
            // The app takes a handoff's stationary flux away whenever its
            // solve finishes, which on a slow device was mid-run: the
            // reference then took it after its own next handoff, from the
            // next generation (2026-11-07). Held at the stop, the run waits
            // for it, so both sides take it from the generation it was found
            // on before either hands its field on.
            if reference.settled != Some(target) {
                driver.phase = Phase::Settling {
                    reference,
                    next,
                    seen: None,
                };
                return None;
            }
            let Some(&(_, action)) = actions.get(next) else {
                let seen = display.full_readbacks;
                request.request_full_state_readback();
                driver.phase = Phase::Snapshot { reference, seen };
                return None;
            };
            if (driver.fixture.counts_the_parts() && reference.departed.is_none())
                || (driver.fixture.reads_the_sides() && reference.sides.is_none())
            {
                let seen = display.full_readbacks;
                request.request_full_state_readback();
                driver.phase = Phase::Departing {
                    reference,
                    next,
                    seen,
                };
                return None;
            }
            let processed = request.stats().processed_event();
            let generation = request.generation();
            match action {
                Action::Switch => state.request_material_switch(),
                Action::Edit => {
                    let edited = state
                        .editor
                        .document
                        .model
                        .draft
                        .material(DEFAULT_MATERIAL)
                        .cloned()
                        .map(|mut medium| {
                            medium.mass_density = ScalarField::constant(1.5);
                            medium
                        });
                    let applied = edited
                        .ok_or_else(|| "the default medium is gone".to_owned())
                        .and_then(|medium| state.editor.update_material(medium));
                    if let Err(error) = applied {
                        return Some(Err(format!("the edit did not apply: {error}")));
                    }
                }
                Action::Remesh => {
                    state.editor.document.presentation.mesh_edge = driver.fixture.finer().0;
                }
                Action::Rebuild => state.coordinator.remesh_requested = true,
                Action::Move => {
                    if let Err(error) = move_the_baffle(&mut state.editor) {
                        return Some(Err(format!("the baffle did not move: {error}")));
                    }
                }
                Action::Draw => {
                    if let Err(error) = draw_the_loop(&mut state.editor) {
                        return Some(Err(format!("the loop was not drawn: {error}")));
                    }
                }
                Action::Delete => {
                    if let Err(error) = delete_the_loop(&mut state.editor) {
                        return Some(Err(format!("the loop was not deleted: {error}")));
                    }
                }
                Action::Timestep => state.editor.document.presentation.simulation_speed = 0.5,
                Action::Drive => {
                    let driven = state.editor.set_outer_condition(
                        &BTreeSet::from([OuterSide::Top]),
                        OuterBoundaryCondition::Dirichlet { signal: WALL_DRIVE },
                    );
                    if let Err(error) = driven {
                        return Some(Err(format!("the drive did not apply: {error}")));
                    }
                }
                Action::Flux => {
                    let driven = state.editor.set_outer_condition(
                        &BTreeSet::from([OuterSide::Top]),
                        OuterBoundaryCondition::Neumann { signal: WALL_FLUX },
                    );
                    if let Err(error) = driven {
                        return Some(Err(format!("the flux did not apply: {error}")));
                    }
                }
                Action::LoseDevice => {
                    state.editor.document.presentation.grid =
                        !state.editor.document.presentation.grid;
                    lose.0 = true;
                    driver.phase = Phase::Losing {
                        asked: Instant::now(),
                        document: Box::new(state.editor.document.clone()),
                    };
                    return None;
                }
            }
            if !driver.fixture.holds() {
                state.e2e_steps.limit = Some(driver.base + stop(next + 1));
            }
            driver.phase = Phase::Act {
                reference,
                next,
                processed,
                generation,
                carried: None,
            };
            None
        }
        Phase::Act {
            mut reference,
            next,
            processed,
            generation,
            carried,
        } => {
            let (step, action) = actions[next];
            let carried = carried.or_else(|| {
                state
                    .runtime
                    .ready()
                    .filter(|candidate| {
                        candidate.bundle.token.document_revision == state.editor.revision
                    })
                    .and_then(|candidate| {
                        Some(Carried {
                            maps: candidate.canonical_transfer.clone()?,
                            interpolation: candidate.transfer.clone(),
                        })
                    })
            });
            // The generation published for the edit: of the document's
            // revision, nothing uploading, and on the display.
            let published = state.runtime.active().cloned().filter(|active| {
                active.bundle.token.document_revision == state.editor.revision
                    && state.coordinator.uploading.is_none()
                    && request.generation() != generation
                    && display.generation == request.generation()
            });
            let waiting = match action {
                Action::Switch => {
                    request.stats().processed_event() == processed
                        || !state.coordinator.pending_switches.is_empty()
                }
                Action::Edit
                | Action::Remesh
                | Action::Rebuild
                | Action::Timestep
                | Action::Drive
                | Action::Flux
                | Action::Move
                | Action::Draw
                | Action::Delete => published.is_none(),
                // The loss takes its own phase and never waits here.
                Action::LoseDevice => {
                    return Some(Err("the device's loss is not an action to wait for".into()));
                }
            };
            if waiting {
                if !driver.fixture.holds()
                    && request
                        .stats()
                        .completed_steps()
                        .saturating_sub(driver.base)
                        >= endpoint
                {
                    return Some(Err(format!(
                        "the device reached the endpoint at {endpoint} before the {action:?} \
                         arrived"
                    )));
                }
                driver.phase = Phase::Act {
                    reference,
                    next,
                    processed,
                    generation,
                    carried,
                };
                return None;
            }
            // The new generation's clock arrives with its first control
            // readback, a frame or so after the display takes the generation.
            if !driver.fixture.holds() && display.clock.is_none() {
                driver.phase = Phase::Act {
                    reference,
                    next,
                    processed,
                    generation,
                    carried,
                };
                return None;
            }
            // Where the action landed: at the step the run holds at, or, with
            // the run going on, at the step the new generation's clock
            // origin names, which the reference then steps to.
            let landed = if driver.fixture.holds() {
                if let Err(failure) = taken(request, &format!("the {action:?}"), driver.base + step)
                {
                    return Some(Err(failure));
                }
                step
            } else {
                let rejection = request.stats().event_rejection();
                if rejection != 0 {
                    return Some(Err(format!(
                        "the device refused the {action:?} (failure code {rejection})"
                    )));
                }
                let Some(clock) = display.clock else {
                    return Some(Err(
                        "the display has no clock to read the handoff from".into()
                    ));
                };
                let time_step = reference.state.time_step();
                let since = clock.epoch_origin_seconds - driver.start_time;
                let landed = (since / time_step).round() as u64;
                let off = (landed as f64 * time_step - since).abs();
                if !within(off, CLOCK_TOLERANCE) || landed < step || landed >= endpoint {
                    return Some(Err(format!(
                        "the new generation's origin {:.6} s is {off:.1e} s from step {landed} \
                         of the old clock, which must lie in {step}..{endpoint}",
                        clock.epoch_origin_seconds
                    )));
                }
                if let Err(error) = reference.advance_to(landed) {
                    return Some(Err(format!("the reference did not step: {error}")));
                }
                landed
            };
            if driver.fixture.reads_unedited() {
                if reference.unedited.is_none() {
                    reference.unedited = Some((reference.active.clone(), reference.state.clone()));
                }
            } else if reference.control.is_none() {
                reference.control = Some((reference.active.clone(), reference.state.clone()));
            }
            match action {
                Action::LoseDevice => {
                    return Some(Err("the device's loss is not an action to take here".into()));
                }
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
                Action::Edit
                | Action::Remesh
                | Action::Rebuild
                | Action::Timestep
                | Action::Drive
                | Action::Flux
                | Action::Move
                | Action::Draw
                | Action::Delete => {
                    let target = published.expect("an edit waits for its generation");
                    let (from, to) = (
                        reference.active.canonical_operator.degrees_of_freedom(),
                        target.canonical_operator.degrees_of_freedom(),
                    );
                    // A remesh goes to the finer mesh recorded; a rebuild of
                    // the same scene builds the same mesh to the bit.
                    let recorded = match action {
                        Action::Remesh => Some(driver.fixture.finer().1),
                        Action::Rebuild => Some(driver.fixture.mesh()),
                        _ => None,
                    };
                    if let Some(recorded) = recorded
                        && let Err(failure) = same_mesh(&target.mesh, recorded)
                    {
                        return Some(Err(failure));
                    }
                    // A move, and a curve drawn or deleted, mesh the ground
                    // they change again, its nodes as many or not.
                    if !matches!(action, Action::Move | Action::Draw | Action::Delete)
                        && matches!(action, Action::Remesh) == (from == to)
                    {
                        return Some(Err(format!(
                            "the {action:?} went from {from} nodes to {to}"
                        )));
                    }
                    report(&format!(
                        "e2e {:?}: handed off at step {landed}{}, {from} nodes to {to}, mesh {}",
                        driver.fixture,
                        if driver.fixture.holds() {
                            ""
                        } else {
                            ", the run going on"
                        },
                        mesh_hashes(&target.mesh)
                    ));
                    if request.handoff_outcome() != CanonicalGpuHandoffOutcome::Accepted {
                        return Some(Err(format!(
                            "the edit's generation came by {:?}, not an admitted handoff",
                            request.handoff_outcome()
                        )));
                    }
                    let Some(carried) = carried else {
                        return Some(Err(
                            "the edit's candidate was never seen with its maps".into()
                        ));
                    };
                    let handed = hand_off(
                        &reference,
                        &target,
                        &carried,
                        state.coordinator.uploaded_time_step,
                        display.runtime_serials,
                    );
                    match handed {
                        Ok((handed, carry, target)) => {
                            reference.state = handed;
                            reference.carry = carry;
                            reference.active = target;
                            reference.generation = request.generation();
                            if let Some(carry) = carry {
                                report(&format!(
                                    "e2e {:?}: the wall's memory carried at {:.4} s, energy {:.3e} \
                                     to {:.3e}, {:.1e} of the field's, residual {:.1e}",
                                    driver.fixture,
                                    reference.state.time(),
                                    carry.source_energy,
                                    carry.target_energy,
                                    carry.share,
                                    carry.residual
                                ));
                            }
                        }
                        Err(error) => {
                            return Some(Err(format!("the reference was not handed off: {error}")));
                        }
                    }
                }
            }
            if reference.carry.is_some() || driver.fixture.reads_the_handoff() {
                let seen = display.full_readbacks;
                request.request_full_state_readback();
                driver.phase = Phase::Carried {
                    reference,
                    next,
                    seen,
                };
                return None;
            }
            state.e2e_steps.limit = Some(driver.base + stop(next + 1));
            driver.phase = Phase::Run {
                reference,
                next: next + 1,
            };
            None
        }
        Phase::Settling {
            mut reference,
            next,
            seen,
        } => {
            let generation = request.generation();
            // A cleanup that landed stays recorded until a readback of a
            // later step, which a held run never takes. At the endpoint an
            // event fixture's events land, all of them, first.
            let unsettled = state.flux_cleanup.as_ref().is_some_and(|cleanup| {
                cleanup.generation == generation
                    && !matches!(
                        cleanup.stage,
                        super::flux_cleanup::FluxCleanupStage::Landed(_)
                    )
            }) || (driver.events.from.is_some()
                && next == actions.len()
                && !driver.events.settled(state, request));
            let Some(seen) = seen.filter(|_| !unsettled) else {
                let seen = (!unsettled).then(|| {
                    request.request_full_state_readback();
                    display.full_readbacks
                });
                driver.phase = Phase::Settling {
                    reference,
                    next,
                    seen,
                };
                return None;
            };
            let stop = stop(next);
            let arrived = display.full_readbacks > seen
                && display.generation == generation
                && display
                    .full_snapshot_completed_steps()
                    .saturating_sub(driver.base)
                    == stop;
            if !arrived {
                request.request_full_state_readback();
                driver.phase = Phase::Settling {
                    reference,
                    next,
                    seen: Some(seen),
                };
                return None;
            }
            if let Err(error) = reference.follow_flux_cleanup(display) {
                return Some(Err(error));
            }
            // Events that landed at the stop itself, while the run held.
            reference.timeline.clone_from(&driver.events.timeline);
            if let Err(error) = reference.advance_to(stop) {
                return Some(Err(format!("the reference did not step: {error}")));
            }
            reference.settled = Some(stop);
            driver.phase = Phase::Run { reference, next };
            None
        }
        Phase::Departing {
            mut reference,
            next,
            seen,
        } => {
            let (step, _) = actions[next];
            let arrived = display.full_readbacks > seen
                && display.generation == request.generation()
                && display
                    .full_snapshot_completed_steps()
                    .saturating_sub(driver.base)
                    == step;
            if !arrived {
                request.request_full_state_readback();
                driver.phase = Phase::Departing {
                    reference,
                    next,
                    seen,
                };
                return None;
            }
            if driver.fixture.counts_the_parts() {
                match part_totals(&reference.active.canonical_operator, &display.primary_flux) {
                    Ok(totals) => reference.departed = Some(totals),
                    Err(failure) => return Some(Err(failure)),
                }
            }
            if driver.fixture.reads_the_sides() {
                match SidesBefore::read(&reference.active, &state.editor.document, display) {
                    Ok(sides) => reference.sides = Some(sides),
                    Err(failure) => return Some(Err(failure)),
                }
            }
            driver.phase = Phase::Run { reference, next };
            None
        }
        Phase::Carried {
            mut reference,
            next,
            seen,
        } => {
            let (step, _) = actions[next];
            // A held run's snapshot is the handoff's own step; a run going
            // on is read at whatever step its snapshot stamps, the
            // reference stepped to it.
            let holds = driver.fixture.holds();
            let stamped = display
                .full_snapshot_completed_steps()
                .saturating_sub(driver.base);
            let arrived = display.full_readbacks > seen
                && display.generation == request.generation()
                && if holds {
                    stamped == step
                } else {
                    stamped >= reference.steps && stamped <= endpoint
                };
            if !arrived {
                request.request_full_state_readback();
                driver.phase = Phase::Carried {
                    reference,
                    next,
                    seen,
                };
                return None;
            }
            if let Err(error) = reference.follow_flux_cleanup(display) {
                return Some(Err(error));
            }
            if !holds {
                if let Err(error) = reference.advance_to(stamped) {
                    return Some(Err(format!("the reference did not step: {error}")));
                }
                // Read, not judged: the pinned nodes and the rest apart, to
                // tell a drive's phase from the field's carry.
                let pinned = reference
                    .active
                    .canonical_forcing
                    .prescribed()
                    .iter()
                    .map(Option::is_some)
                    .collect::<Vec<_>>();
                let unpinned = pinned.iter().map(|pinned| !pinned).collect::<Vec<_>>();
                let on_pins = field_errors(&reference, display, Some(&pinned)).map(|e| e.0);
                let off_pins = field_errors(&reference, display, Some(&unpinned));
                let wall = wall_errors(&reference, display);
                match (on_pins, off_pins, wall) {
                    (Ok(on), Ok((off, b, _)), Ok(Some(wall))) => report(&format!(
                        "e2e {:?}: read at step {stamped}, {} after the request: Q {on:.3e} on the \
                         pinned nodes, {off:.3e} off them, b {b:.3e}, wall z {:.3e}, memory {:.3e}",
                        driver.fixture,
                        stamped - step,
                        wall.z,
                        wall.memory
                    )),
                    (Err(failure), _, _) | (_, Err(failure), _) | (_, _, Err(failure)) => {
                        return Some(Err(failure));
                    }
                    (_, _, Ok(None)) => return Some(Err("no wall to read".into())),
                }
                driver.phase = Phase::Run {
                    reference,
                    next: next + 1,
                };
                return None;
            }
            if driver.fixture.counts_the_parts() {
                let Some((departed, sizes)) = reference.departed else {
                    return Some(Err("the parts were not counted before the remesh".into()));
                };
                let arrived = match part_totals(
                    &reference.active.canonical_operator,
                    &display.primary_flux,
                ) {
                    Ok((arrived, _)) => arrived,
                    Err(failure) => return Some(Err(failure)),
                };
                let moved = (0..3)
                    .map(|part| (arrived[part] - departed[part]).abs() / sizes[part])
                    .fold(0.0, worse);
                let summary = format!(
                    "the parts' totals {:.4e}, {:.4e} and {:.4e} came through within {moved:.1e} \
                     of their magnitudes",
                    departed[0], departed[1], departed[2]
                );
                report(&format!("e2e {:?}: {summary}", driver.fixture));
                if !within(moved, PARTS_TOLERANCE) {
                    return Some(Err(format!(
                        "{summary}: the device's carry moved a part's total beyond \
                         {PARTS_TOLERANCE:.0e}"
                    )));
                }
            }
            let tolerance = driver.fixture.tolerance();
            // Q off the prescribed nodes, and on them where the reference
            // pins them at the handoff. The device pins them to the signal on
            // the new generation's clock as it takes the field; the fixed
            // reference pins them as it steps, and on an edited medium the
            // two values differ until that step, but the driven reference
            // pins them as it takes the field, at the mass in force then.
            let pinned = reference
                .active
                .canonical_forcing
                .prescribed()
                .iter()
                .map(Option::is_some)
                .collect::<Vec<_>>();
            let unpinned = pinned.iter().map(|pinned| !pinned).collect::<Vec<_>>();
            let (q, b, _) = match field_errors(&reference, display, Some(&unpinned)) {
                Ok(errors) => errors,
                Err(failure) => return Some(Err(failure)),
            };
            let pins = if matches!(reference.state, State::Temporal(_)) && pinned.contains(&true) {
                match field_errors(&reference, display, Some(&pinned)) {
                    Ok((pins, _, _)) => Some(pins),
                    Err(failure) => return Some(Err(failure)),
                }
            } else {
                None
            };
            let wall = match wall_errors(&reference, display) {
                Ok(None) if reference.carry.is_some() => {
                    return Some(Err("the carry's snapshot has no wall to read".into()));
                }
                Ok(wall) => wall,
                Err(failure) => return Some(Err(failure)),
            };
            let mut summary = format!("at the handoff Q {q:.3e}");
            let mut largest = q;
            if let Some(pins) = pins {
                summary += &format!(", {pins:.3e} on the pins");
                largest = worse(largest, pins);
            }
            summary += &format!(", b {b:.3e}");
            largest = worse(largest, b);
            if let Some(wall) = &wall {
                summary += &format!(
                    ", wall z {:.3e}, memory {:.3e}, energy {:.3e}",
                    wall.z, wall.memory, wall.energy
                );
                largest = worse(largest, worse(wall.z, worse(wall.memory, wall.energy)));
            }
            report(&format!("e2e {:?}: {summary}", driver.fixture));
            if !within(largest, tolerance) {
                return Some(Err(format!(
                    "{summary}: the device's handed state stands from the reference's beyond \
                     {tolerance:.0e}"
                )));
            }
            if let (Some(carry), Some(wall)) = (&mut reference.carry, &wall) {
                carry.at_handoff = [q, b, wall.z, wall.memory];
            }
            // Read once the device stands with the reference: both carry the
            // field the same way, so only the device's own state shows where
            // the jump went.
            if driver.fixture.reads_the_sides() {
                let Some(sides) = &reference.sides else {
                    return Some(Err("the baffle was not read before the move".into()));
                };
                let ([q, b], [q_left, b_left]) = match sides.after(&reference.active, display) {
                    Ok(read) => read,
                    Err(failure) => return Some(Err(failure)),
                };
                let summary = format!(
                    "the jump stands within {q:.3} of each side's own in Q and {b:.3} in b, \
                     {q_left:.3} and {b_left:.3} of it left behind"
                );
                report(&format!("e2e {:?}: {summary}", driver.fixture));
                if !within(q, MOVED_SIDES)
                    || !within(b, MOVED_SIDES)
                    || !within(q_left, LEFT_BEHIND)
                    || !within(b_left, LEFT_BEHIND)
                {
                    return Some(Err(format!(
                        "{summary}: the device's carry did not move the jump with the baffle"
                    )));
                }
            }
            state.e2e_steps.limit = Some(driver.base + stop(next + 1));
            driver.phase = Phase::Run {
                reference,
                next: next + 1,
            };
            None
        }
        Phase::Losing { asked, document } => {
            // The handler records the stop and the app acts on it once; both
            // must have happened, within two seconds of the loss.
            let Some(stopped) = stopped.filter(|stopped| stopped.announced()) else {
                driver.phase = Phase::Losing { asked, document };
                return None;
            };
            let elapsed = asked.elapsed();
            if elapsed > std::time::Duration::from_secs(2) {
                return Some(Err(format!(
                    "the app stopped {:.0} ms after the device was lost",
                    elapsed.as_secs_f64() * 1000.0
                )));
            }
            // The document, the change the autosave had not written included.
            let saved = crate::recovery::load()
                .and_then(|bytes| bytes.ok_or_else(|| "no autosave".to_owned()))
                .and_then(|bytes| crate::topology_persistence::parse_document(&bytes));
            let saved = match saved {
                Ok(saved) => saved,
                Err(error) => return Some(Err(format!("the autosave does not open: {error}"))),
            };
            if saved != *document {
                return Some(Err(
                    "the autosave does not hold the document as it was at the loss".to_owned(),
                ));
            }
            Some(Ok(Run {
                summary: format!(
                    "the app stopped {:.0} ms after the device was lost ({}): {}; the autosave holds the document",
                    elapsed.as_secs_f64() * 1000.0,
                    stopped.attribute(),
                    stopped.message()
                ),
                hash: 0,
            }))
        }
        Phase::Snapshot {
            mut reference,
            seen,
        } => {
            // Only a snapshot of this generation, stamped with the endpoint
            // inside the state it copied, is the endpoint's.
            let arrived = display.full_readbacks > seen
                && display.generation == request.generation()
                && display.full_snapshot_completed_steps() == driver.base + endpoint;
            if !arrived {
                request.request_full_state_readback();
                driver.phase = Phase::Snapshot { reference, seen };
                return None;
            }
            // The overlay's first readback waits for its pipeline, which
            // SwiftShader in CI had not compiled by the endpoint (2026-11-07).
            let overlay_read = overlay.generation == request.generation()
                && !overlay.samples.is_empty()
                && overlay.readbacks != 0;
            if driver.fixture.observes()
                && !driver.hashes.is_empty()
                && !overlay_read
                && driver
                    .observing
                    .overlay_wait
                    .get_or_insert_with(Instant::now)
                    .elapsed()
                    < OVERLAY_WAIT
            {
                driver.phase = Phase::Snapshot { reference, seen };
                return None;
            }
            if let Err(error) = reference.follow_flux_cleanup(display) {
                return Some(Err(error));
            }
            let timeline = match driver.events.from {
                Some(from) => {
                    // What the device was given, whatever the verdict, and
                    // where the device itself took the handoff's snapshot: a
                    // failure seen once in 63 runs (2026-11-07) stood 0.65
                    // off with every event landed once and in order.
                    report(&format!(
                        "e2e {:?}: generations {:?}, the device's handoff snapshot at {:?}, \
                         its outcome {:?}",
                        driver.fixture,
                        driver
                            .events
                            .generations
                            .iter()
                            .map(|(generation, _)| *generation)
                            .collect::<Vec<_>>(),
                        request
                            .handed_over_at()
                            .map(|step| step.saturating_sub(driver.base)),
                        request.handoff_outcome()
                    ));
                    report(&format!(
                        "e2e {:?}: timeline {}",
                        driver.fixture,
                        driver
                            .events
                            .timeline
                            .iter()
                            .map(|(serial, landed)| format!(
                                "{serial}:{}@{}g{}#{}",
                                if matches!(landed.fired, Fired::Pulse { .. }) {
                                    "P"
                                } else {
                                    "S"
                                },
                                landed.step,
                                landed.generation,
                                landed.order + 1
                            ))
                            .collect::<Vec<_>>()
                            .join(" ")
                    ));
                    match landed_once(&driver.events, &reference, from) {
                        Ok(timeline) => timeline,
                        Err(failure) => return Some(Err(failure)),
                    }
                }
                None => String::new(),
            };
            let run = compare(&reference, display, driver.fixture, endpoint).map(|summary| Run {
                summary: summary + &timeline,
                hash: hash(&display.accepted_storage_bits()),
            });
            if matches!(driver.fixture, Fixture::Superposition | Fixture::Similarity) {
                driver.time_steps.push(state.coordinator.uploaded_time_step);
                driver.endpoints.push([
                    display.primary_flux.clone(),
                    display
                        .complementary_flux
                        .iter()
                        .flatten()
                        .copied()
                        .collect(),
                    display.history_auxiliary().to_vec(),
                ]);
            }
            if driver.fixture.observes() && !driver.hashes.is_empty() {
                return match run.and_then(|mut run| {
                    run.summary +=
                        &observed(state, overlay, &driver.observing, request.generation())?;
                    Ok(run)
                }) {
                    Ok(run) => {
                        driver.phase = Phase::Recorded {
                            run,
                            since: Instant::now(),
                        };
                        None
                    }
                    failed => Some(failed),
                };
            }
            match run {
                Ok(run) if reference.twin.is_some() || reference.far.is_some() => {
                    driver.phase = Phase::Observe {
                        reference,
                        run,
                        since: Instant::now(),
                    };
                    None
                }
                run => Some(run),
            }
        }
        Phase::Recorded { run, since } => {
            if state.recording_state != RecordingState::Idle {
                if since.elapsed() > RECORDING_FINISH {
                    return Some(Err(format!(
                        "the recording was not written within {} s of its stop",
                        RECORDING_FINISH.as_secs()
                    )));
                }
                driver.phase = Phase::Recorded { run, since };
                return None;
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                *crate::recording::FIXTURE_DESTINATION.lock().unwrap() = None;
                if let Err(failure) = recording_written() {
                    return Some(Err(failure));
                }
            }
            Some(Ok(run))
        }
        Phase::Observe {
            reference,
            run,
            since,
        } => {
            // The readbacks that carry the run's last records arrive frames
            // after its endpoint; a second is many of them.
            let time = reference.state.time();
            let last =
                |times: &mut dyn Iterator<Item = f64>| times.fold(f64::NEG_INFINITY, f64::max);
            let behind = [
                last(&mut state.probe_traces.values().map(|trace| trace.last_time)),
                last(
                    &mut state
                        .curve_probe_traces
                        .values()
                        .map(|trace| trace.last_time),
                ),
                last(
                    &mut state
                        .area_probe_traces
                        .values()
                        .map(|trace| trace.last_time),
                ),
            ]
            .iter()
            .any(|seen| *seen < time - 4.0 * reference.state.time_step());
            let behind = match &reference.far {
                // The far field's last record is the twin's last projection,
                // which the judge holds it to.
                Some(far) => far
                    .span()
                    .is_some_and(|(_, last)| state.far_field_trace.last_time < last - 1.0e-3),
                None => behind,
            };
            // Many frames natively; served as Pages a frame can take a second.
            if behind && since.elapsed() < std::time::Duration::from_secs(30) {
                driver.phase = Phase::Observe {
                    reference,
                    run,
                    since,
                };
                return None;
            }
            let judged = match (&reference.twin, &reference.far) {
                (Some(twin), _) => observables::judge(state, twin, endpoint),
                (None, Some(far)) => far_field::judge(state, far, driver.fixture),
                (None, None) => unreachable!("an observed run has its twins"),
            };
            Some(judged.map(|recorded| Run {
                summary: format!("{}; {recorded}", run.summary),
                hash: run.hash,
            }))
        }
        Phase::Refused {
            mut reference,
            step,
            code,
            seen,
            first,
            mut retried,
        } => {
            let paused = state.coordinator.solver_fault.is_some() && !state.wave_running;
            if first.is_some() && !retried && state.coordinator.solver_fault.is_none() {
                // Run cleared the failure: what follows is the retry's.
                retried = true;
            }
            let waiting_for_retry = first.is_some() && !retried;
            let snapshot_in = display.full_readbacks > seen
                && display.generation == request.generation()
                && display.full_snapshot_completed_steps() == step;
            if !paused || waiting_for_retry || !snapshot_in {
                if paused && !waiting_for_retry {
                    request.request_full_state_readback();
                }
                driver.phase = Phase::Refused {
                    reference,
                    step,
                    code,
                    seen,
                    first,
                    retried,
                };
                return None;
            }
            let completed = request.stats().completed_steps();
            let again = request.stats().failure();
            if completed != step || again != code {
                return Some(Err(format!(
                    "refused after step {step} with code {code}, then after {completed} with {again}"
                )));
            }
            let hash = hash(&display.accepted_storage_bits());
            match first {
                None => {
                    if code != CANONICAL_FAILURE_INVERSE_DOMAIN {
                        // The reference's verdict on the same step, the one
                        // after `step`: taken from where it stood, step 0, it
                        // judged step 1 (VER-41 of the 2026-10-20 review).
                        let verdict = match reference.advance_to(step) {
                            Err(error) => format!(
                                "refuses earlier, before step {}: {error}",
                                reference.steps + 1
                            ),
                            Ok(()) => match reference.state.clone().step(&reference.active) {
                                Ok(()) => "takes that step".to_owned(),
                                Err(error) => format!("refuses it: {error}"),
                            },
                        };
                        return Some(Err(format!(
                            "the device refused the step after {step} with code {code} ({}), not \
                             the inverse domain's; the reference {verdict}",
                            canonical_failure_description(code)
                        )));
                    }
                    // The reference takes every step the device accepted and
                    // refuses the one after, as the device did.
                    if let Err(error) = reference.advance_to(step) {
                        return Some(Err(format!(
                            "the reference refused earlier than the device, before step {}: {error}",
                            reference.steps + 1
                        )));
                    }
                    if reference.state.clone().step(&reference.active).is_ok() {
                        return Some(Err(format!(
                            "the device refused the step after {step}, which the reference takes"
                        )));
                    }
                    if let Err(error) = reference.follow_flux_cleanup(display) {
                        return Some(Err(error));
                    }
                    let summary = match compare(&reference, display, driver.fixture, step) {
                        Ok(summary) => summary,
                        Err(failure) => return Some(Err(failure)),
                    };
                    report(&format!(
                        "e2e Rejection: refused after step {step} as the reference is, the app \
                         paused; {summary}"
                    ));
                    // Run, as the toolbar's button.
                    state.wave_running = true;
                    driver.phase = Phase::Refused {
                        reference,
                        step,
                        code,
                        seen: display.full_readbacks,
                        first: Some((hash, summary)),
                        retried: false,
                    };
                    None
                }
                Some((first, summary)) => Some(if hash == first {
                    Ok(Run {
                        summary: format!(
                            "{summary}; Run retried the step to the same refusal, the stored \
                             state the same bits"
                        ),
                        hash,
                    })
                } else {
                    Err(format!(
                        "the retry moved the stored state: {first:016x} to {hash:016x}"
                    ))
                }),
            }
        }
        Phase::Done => None,
    }
}

/// The reference's state carried onto `target` as the device carries the
/// running field: through the maps the candidate was prepared with, each
/// component's total kept where the app keeps it - on the same geometry, the
/// shares of the source components the runtime transfer names - and stepped
/// on at the step the app uploaded for it. Where the generation has an
/// outgoing wall, the wall's memory goes the physical way, its pole currents
/// interpolated onto the new trace and projected onto the new basis, apart
/// from the device's normalized matrix, and the carry says how much that way
/// could not hold. The state's clock runs on from the time the run reached,
/// as the device's clock origin does, so the generation's forcing reads the
/// phase the signals had got to.
///
/// A driven generation, or one going between fixed and driven, is handed by
/// the core's handoffs, which the device's transfer table is built from;
/// no fixture with a wall is driven. Either way the target runs its forcing
/// as the device runs it from the handoff (`CanonicalForcing::handed_on`):
/// a harmonic channel the handoff starts begins at rest there, and one it
/// carries runs on as it ran. The generation returned carries that forcing,
/// for the reference to step on.
fn hand_off(
    reference: &Reference,
    target: &PreparedTopology,
    carried: &Carried,
    time_step: f64,
    runtime_serials: [u32; 4],
) -> Result<(State, Option<MemoryCarry>, Arc<PreparedTopology>), String> {
    let source = &reference.active;
    let mut handed_target = target.clone();
    handed_target.canonical_forcing = Arc::new(
        target
            .canonical_forcing
            .handed_on(&source.canonical_forcing, reference.state.time())
            .map_err(|error| error.to_string())?,
    );
    let target = Arc::new(handed_target);
    let maps = &carried.maps;
    let runtime = CanonicalGpuRuntimeTransfer::from_primary_transfer(
        &source.canonical_operator,
        &target.canonical_operator,
        &source.canonical_forcing,
        &target.canonical_forcing,
        &maps.primary,
        runtime_serials,
    )
    .map_err(|error| format!("{error:?}"))?;
    let runtime = if crate::topology_runtime::corrects_component_totals(&target, source) {
        runtime
    } else {
        runtime.without_total_correction()
    };
    let desired = runtime.desired_totals(&source.canonical_operator, reference.state.primary());
    let handoff_maps = CanonicalHandoffMaps {
        primary: &maps.primary,
        complementary: &maps.complementary,
        thin_gap: &maps.thin_gap,
        outgoing: &maps.outgoing,
    };
    let forcing = &target.canonical_forcing;
    let driven = match (
        &reference.state,
        &source.canonical_temporal_operator,
        &target.canonical_temporal_operator,
    ) {
        (State::Fixed(_), None, None) => None,
        (State::Temporal(running), Some(from), Some(to)) => Some(
            running
                .handed_over(from, to, forcing, handoff_maps, &desired, time_step)
                .map(|handed| State::Temporal(handed.state)),
        ),
        (State::Fixed(running), None, Some(to)) => Some(
            CanonicalTemporalWaveState::handed_from_fixed(
                running,
                &source.canonical_operator,
                to,
                forcing,
                handoff_maps,
                &desired,
                time_step,
            )
            .map(|handed| State::Temporal(handed.state)),
        ),
        (State::Temporal(running), Some(from), None) => Some(
            CanonicalWaveState::handed_from_driven(
                running,
                from,
                &target.canonical_operator,
                forcing,
                handoff_maps,
                &desired,
                time_step,
            )
            .map(|(handed, _, _)| State::Fixed(handed)),
        ),
        _ => return Err("the reference's state and its generation disagree on a drive".into()),
    };
    if let Some(handed) = driven {
        let handed = handed.map_err(|error| error.to_string())?;
        return Ok((handed, None, target));
    }
    let State::Fixed(running) = &reference.state else {
        unreachable!("a fixed generation's reference is fixed");
    };
    let prescribed = target
        .canonical_forcing
        .prescribed()
        .iter()
        .map(Option::is_some)
        .collect::<Vec<_>>();
    let (primary, _) = maps
        .primary
        .transfer(running.primary_flux(), &desired, &prescribed)
        .map_err(|error| error.to_string())?;
    let (complementary, _) = maps
        .complementary
        .transfer(running.complementary_flux())
        .map_err(|error| error.to_string())?;
    let mut handed = CanonicalWaveState::new(
        &target.canonical_operator,
        time_step,
        primary,
        complementary,
    )
    .and_then(|handed| handed.with_time_origin(running.time()))
    .map_err(|error| error.to_string())?;
    let memory = running
        .outgoing_physical_memory(&source.canonical_operator)
        .map_err(|error| error.to_string())?;
    let carry = match (memory, target.canonical_operator.outgoing_boundary()) {
        (None, None) => None,
        (Some(_), None) => return Err("the wall's memory has no wall to go to".into()),
        (None, Some(_)) => return Err("the new generation has a wall the old had not".into()),
        (Some(memory), Some(_)) => {
            let interpolation = match &carried.interpolation {
                Some(interpolation) => interpolation.clone(),
                None => Arc::new(
                    QuadraticTransferMap::identity_on_mesh(
                        &target.mesh,
                        &source.operator,
                        &target.operator,
                    )
                    .map_err(|error| error.to_string())?,
                ),
            };
            let map = CanonicalOutgoingHistoryTransferMap::prepare(
                &interpolation,
                &source.canonical_operator,
                &target.canonical_operator,
            )
            .map_err(|error| error.to_string())?;
            let (interpolated, _) = map
                .transfer(
                    &source.canonical_operator,
                    &target.canonical_operator,
                    Some(&memory),
                )
                .map_err(|error| error.to_string())?;
            let interpolated =
                interpolated.ok_or_else(|| "the wall's memory was lost in the carry".to_owned())?;
            let projection = handed
                .set_outgoing_physical_memory(&target.canonical_operator, &interpolated)
                .map_err(|error| error.to_string())?;
            let before = energies(&source.canonical_operator, running)?;
            let after = energies(&target.canonical_operator, &handed)?;
            Some(MemoryCarry {
                source_energy: before.outgoing,
                target_energy: after.outgoing,
                share: before.outgoing / before.total(),
                residual: if projection.physical_norm > 0.0 {
                    projection.physical_residual_norm / projection.physical_norm
                } else {
                    0.0
                },
                at_handoff: [0.0; 4],
            })
        }
    };
    Ok((State::Fixed(handed), carry, target))
}

/// The energy a state holds, by component.
fn energies(
    operator: &CanonicalWaveOperator,
    state: &CanonicalWaveState,
) -> Result<CanonicalEnergyBreakdown, String> {
    let auxiliary = match state.auxiliaries() {
        CanonicalAuxiliaryState::None => Vec::new(),
        CanonicalAuxiliaryState::Linear(auxiliaries) => auxiliaries
            .thin_gap_jump()
            .iter()
            .chain(auxiliaries.outgoing_z())
            .copied()
            .collect(),
        _ => return Err("an auxiliary state this fixture does not read".into()),
    };
    canonical_energy_breakdown(
        operator,
        state.primary_flux(),
        state.complementary_flux(),
        &auxiliary,
    )
    .map_err(|error| error.to_string())
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

/// The clock the device reports may stand this far, in seconds, from the
/// time the reference has reached: the origin is carried as a pair of f32
/// and the local time as one, at a few seconds into a run.
const CLOCK_TOLERANCE: f64 = 2.0e-5;

/// The relative L2 errors of Q, b and r, where the medium keeps one, of the
/// display's snapshot against the reference's state; Q over the nodes
/// `included` names, where it names any.
fn field_errors(
    reference: &Reference,
    display: &CanonicalGpuDisplay,
    included: Option<&[bool]>,
) -> Result<(f64, f64, f64), String> {
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
    let q = match included {
        Some(included) => relative_l2(
            display
                .primary_flux
                .iter()
                .zip(included)
                .filter(|(_, included)| **included)
                .map(|(value, _)| f64::from(*value)),
            expected
                .primary()
                .iter()
                .zip(included)
                .filter(|(_, included)| **included)
                .map(|(value, _)| *value),
        ),
        None => primary_error(display, expected),
    };
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
    Ok((q, b, r))
}

/// The relative L2 error of the display's Q against a state's.
fn primary_error(display: &CanonicalGpuDisplay, state: &State) -> f64 {
    relative_l2(
        display.primary_flux.iter().map(|value| f64::from(*value)),
        state.primary().iter().copied(),
    )
}

/// The wall's pole states in the display's snapshot against the reference's:
/// coefficient by coefficient, as the memory they represent, and as the
/// energy they hold, relative to the reference's.
struct WallErrors {
    z: f64,
    memory: f64,
    energy: f64,
    energy_expected: f64,
}

/// The wall's errors, where the generation has a wall.
fn wall_errors(
    reference: &Reference,
    display: &CanonicalGpuDisplay,
) -> Result<Option<WallErrors>, String> {
    let operator = &reference.active.canonical_operator;
    let Some(boundary) = operator.outgoing_boundary() else {
        return Ok(None);
    };
    if !operator.thin_gap_samples().is_empty() {
        return Err("a thin gap beside the wall, which this comparison does not read".into());
    }
    let expected = &reference.state;
    let device = display
        .history_auxiliary()
        .iter()
        .map(|value| f64::from(*value))
        .collect::<Vec<_>>();
    let z_expected = expected.outgoing_z();
    if device.len() != boundary.auxiliary_count() || z_expected.len() != boundary.auxiliary_count()
    {
        return Err(format!(
            "the wall has {} pole states where the snapshot holds {} and the reference {}",
            boundary.auxiliary_count(),
            device.len(),
            z_expected.len()
        ));
    }
    let z = relative_l2(device.iter().copied(), z_expected.iter().copied());
    let physical = |lanes: &[f64]| {
        boundary
            .physical_memory(lanes)
            .map(|memory| {
                memory
                    .pole_currents
                    .into_iter()
                    .flatten()
                    .collect::<Vec<f64>>()
            })
            .map_err(|error| error.to_string())
    };
    let memory = relative_l2(
        physical(&device)?.into_iter(),
        physical(z_expected)?.into_iter(),
    );
    let energy_device = canonical_energy_breakdown(
        operator,
        &display
            .primary_flux
            .iter()
            .map(|value| f64::from(*value))
            .collect::<Vec<_>>(),
        &display
            .complementary_flux
            .iter()
            .map(|value| Point2::new(f64::from(value[0]), f64::from(value[1])))
            .collect::<Vec<_>>(),
        &device,
    )
    .map_err(|error| error.to_string())?
    .outgoing;
    let energy_expected = canonical_energy_breakdown(
        operator,
        expected.primary(),
        expected.complementary(),
        z_expected,
    )
    .map_err(|error| error.to_string())?
    .outgoing;
    Ok(Some(WallErrors {
        z,
        memory,
        energy: (energy_device - energy_expected).abs() / energy_expected,
        energy_expected,
    }))
}

/// The endpoint's snapshot against the reference: the relative L2 error of Q,
/// of b, and of r where the medium keeps one, each within the fixture's
/// tolerance; where the generation has a wall, its pole states coefficient by
/// coefficient and in the memory they represent within [`WALL_TOLERANCE`],
/// their energy within the tolerance, and the last carry's own share and
/// residual within their bounds; the device's clock at the time the
/// reference has reached; and, where the fixture acted, the device far from
/// the run without its actions, or, for a rebuild that changed nothing,
/// where that run is, or, where the fixture reads it, within
/// [`UNEDITED_APART`] of that run stepped on to the same time.
fn compare(
    reference: &Reference,
    display: &CanonicalGpuDisplay,
    fixture: Fixture,
    endpoint: u64,
) -> Result<String, String> {
    let tolerance = fixture.tolerance();
    let expected = &reference.state;
    let (q, b, r) = field_errors(reference, display, None)?;
    let mut summary = format!("Q {q:.3e}, b {b:.3e}, r {r:.3e} after {endpoint} steps");
    let mut worst = worse(worse(q, b), r);
    if let Some(wall) = wall_errors(reference, display)? {
        summary.push_str(&format!(
            "; wall z {:.3e}, memory {:.3e}, energy {:.3e} of {:.3e}",
            wall.z, wall.memory, wall.energy, wall.energy_expected
        ));
        if !within(worse(wall.z, wall.memory), WALL_TOLERANCE) {
            return Err(format!(
                "{summary}: the pole states stand from the reference's beyond {WALL_TOLERANCE:.0e}"
            ));
        }
        worst = worse(worst, wall.energy);
        if let Some(carry) = reference.carry {
            let change = (carry.target_energy - carry.source_energy) / carry.source_energy;
            summary.push_str(&format!(
                "; carried {:.1e} of the field's energy, {:.3e} to {:.3e} ({change:+.1e}), \
                 residual {:.1e}; at the handoff Q {:.1e}, b {:.1e}, z {:.1e}, memory {:.1e}",
                carry.share,
                carry.source_energy,
                carry.target_energy,
                carry.residual,
                carry.at_handoff[0],
                carry.at_handoff[1],
                carry.at_handoff[2],
                carry.at_handoff[3]
            ));
            if carry.share < MEMORY_SHARE_FLOOR {
                return Err(format!(
                    "{summary}: the wall held too little at the handoff to carry, under \
                     {MEMORY_SHARE_FLOOR:.0e} of the field's energy"
                ));
            }
            if !within(carry.residual, CARRY_RESIDUAL) {
                return Err(format!(
                    "{summary}: the carry left {:.3e} of the memory behind, beyond \
                     {CARRY_RESIDUAL:.0e}",
                    carry.residual
                ));
            }
            if !within(change.abs(), CARRY_ENERGY) {
                return Err(format!(
                    "{summary}: the carry moved the wall's energy by {change:+.3e}, beyond \
                     {CARRY_ENERGY:.0e}"
                ));
            }
        }
    }
    if !within(worst, tolerance) {
        return Err(format!("{summary}, beyond {tolerance:.0e}"));
    }
    if let Some(clock) = display.clock {
        let reached = expected.time();
        let drift = (clock.absolute_seconds - reached).abs();
        summary.push_str(&format!(
            "; clock {:.4} s, {drift:.1e} from the run's",
            clock.absolute_seconds
        ));
        if !within(drift, CLOCK_TOLERANCE) {
            return Err(format!(
                "{summary} {reached:.6} s, beyond {CLOCK_TOLERANCE:.0e}"
            ));
        }
    }
    if fixture.reads_unedited() {
        let Some((plain, held)) = &reference.unedited else {
            return Err(format!("{summary}: the unedited run was never held"));
        };
        let [q, b] = unedited_apart(plain, held, &reference.active, display, expected.time())?;
        summary.push_str(&format!(
            "; the unedited run stands {q:.3e} off in Q and {b:.3e} in b"
        ));
        if !within(q, UNEDITED_APART[0]) || !within(b, UNEDITED_APART[1]) {
            return Err(format!(
                "{summary}, beyond a halving's move of it, {:.1e} and {:.1e}: the device \
                 does not read the separator as none",
                UNEDITED_APART[0], UNEDITED_APART[1]
            ));
        }
    }
    if let Some((_, control)) = reference
        .control
        .as_ref()
        .filter(|(_, control)| control.primary().len() == expected.primary().len())
    {
        let apart = primary_error(display, control);
        if matches!(fixture, Fixture::HandoffMemory(MemoryHandoff::Rebuild)) {
            // A rebuild of the same mesh is to leave the device where the
            // uninterrupted run is.
            summary.push_str(&format!("; the uninterrupted run stands {apart:.3e} off"));
            if !within(apart, tolerance) {
                return Err(format!(
                    "{summary}, where a rebuild of the same mesh should leave the device"
                ));
            }
        } else {
            // An action the device ignored would leave it where the control
            // is; a hundred tolerances away, it did not. A remesh the device
            // ignored would leave it on the other mesh, which the lengths
            // already catch.
            summary.push_str(&format!(
                "; the run without its actions stands {apart:.3e} off"
            ));
            if !apart.is_finite() || apart <= 100.0 * tolerance {
                return Err(format!("{summary}, too near to tell the action took"));
            }
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

/// The fixture asked for and its time limit in seconds, if one is:
/// `FUNFERN_E2E` and `FUNFERN_E2E_DEADLINE` natively, `?e2e=` and
/// `e2e-deadline=` in the page's address.
fn requested() -> Option<(String, Option<u64>)> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let name = std::env::var("FUNFERN_E2E").ok()?;
        let seconds = std::env::var("FUNFERN_E2E_DEADLINE")
            .ok()
            .and_then(|seconds| seconds.parse().ok());
        Some((name, seconds))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let search = web_sys::window()?.location().search().ok()?;
        let value = |key: &str| {
            search
                .trim_start_matches('?')
                .split('&')
                .find_map(|pair| pair.strip_prefix(key)?.strip_prefix('='))
                .map(str::to_owned)
        };
        Some((
            value("e2e")?,
            value("e2e-deadline").and_then(|s| s.parse().ok()),
        ))
    }
}

/// A line of the driver's account: standard output natively, the console in
/// the browser, where standard output goes nowhere.
fn report(line: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    println!("{line}");
    #[cfg(target_arch = "wasm32")]
    web_sys::console::log_1(&line.into());
}

/// The fixture's state where a browser test reads it: `running`, `pass` or
/// `fail` on the page's root element, its summary written first. Natively the
/// exit status says it.
fn mark(verdict: &str, summary: &str) {
    #[cfg(target_arch = "wasm32")]
    if let Some(root) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
    {
        let _ = root.set_attribute("data-funfern-e2e-summary", summary);
        let _ = root.set_attribute("data-funfern-e2e", verdict);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (verdict, summary);
}

/// Whether `mesh` is the one recorded, to the bit.
fn same_mesh(mesh: &funfern_core::TriMesh, recorded: &str) -> Result<(), String> {
    let built = mesh_hashes(mesh);
    if built == recorded {
        Ok(())
    } else {
        Err(format!(
            "the mesh is {built} where {recorded} is recorded: every platform must build \
             the same mesh to the bit, and a change to the mesher made on purpose records \
             it again in src/ui/e2e.rs"
        ))
    }
}

/// A mesh as two hashes, `topology/positions`: its triangles' vertices and
/// regions, and every vertex's coordinates to the bit.
fn mesh_hashes(mesh: &funfern_core::TriMesh) -> String {
    let topology = mesh
        .triangles
        .iter()
        .flat_map(|triangle| {
            triangle
                .vertices
                .iter()
                .map(|vertex| *vertex as u64)
                .chain([triangle.region.0])
        })
        .chain([mesh.vertices.len() as u64]);
    let positions = mesh
        .vertices
        .iter()
        .flat_map(|vertex| [vertex.point.x.to_bits(), vertex.point.y.to_bits()]);
    let fnv = |words: &mut dyn Iterator<Item = u64>| {
        words
            .flat_map(u64::to_le_bytes)
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
            })
    };
    format!(
        "{:016x}/{:016x}",
        fnv(&mut topology.into_iter()),
        fnv(&mut positions.into_iter())
    )
}
