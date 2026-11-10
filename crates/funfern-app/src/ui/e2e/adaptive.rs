//! The adaptive loop on the device (campaign plan, section 6l, C7).
//!
//! AMR-01 to AMR-04 closed the loop on runs stepped on the CPU, where a mesh
//! changes at the instant of its estimate. Here the app runs it as it ships:
//! the estimate reads the device's f32 readbacks no sooner than its
//! wall-clock gap, the transaction runs on the worker, the mesh it makes is
//! prepared, packed and handed off while the run goes on, at its own step,
//! and the stationary flux each handoff leaves is taken away after it. Two
//! runs part where the mesh changes, so the run is held to what the loop
//! must do rather than to bits.
//!
//! `adaptive` clicks the app's own pulse at AMR-01's ring's place in the
//! default box, adaptation as a new document has it, at 0.04 wide, which the
//! authored mesh does not hold, and runs 8 s at the default speed: the mesh
//! refines as the ring reaches the walls, each handoff at its own step, and
//! coarsens as it drains. `adaptive-slow` clicks the default 0.06 at a tenth
//! of the speed, where the reach restores elements ahead of the ring the
//! coarsening took behind it. An f64 reference follows every generation the
//! app installs on another thread, so that the frames keep the app's own
//! pace: handed off at the step each new generation's clock names through
//! the maps its candidate was prepared with, its stationary flux taken away
//! at the step the device took it, the grid filter on the device's cadence.
//! It must stand with every full snapshot it reaches; it estimates each
//! state the app estimated, which must agree with the app's estimate of the
//! device's copy; and it watches where its field is against the mesh it
//! runs on, which may never be coarser than the authored one. Each mesh
//! must land within the cycle the field's reach was drawn for, and the loop
//! keep the bounds [`AdaptiveFixture::loop_bounds`] gives: the narrow ring
//! refined, the slow one dormant and coarsened back past the authored size.

use super::*;
use crate::adaptation::{
    AMR_FIELD_PRESENCE_RATIO, AMR_REACH_MARGIN, AdaptationController, AmrDecision,
    IndicatorSnapshots, adaptation_decision, size_rule_refines, target_accuracy,
};
use crate::canonical_gpu::CanonicalGpuStats;
use crate::document::PresentationSettings;
use crate::topology_runtime::TopologyToken;
use funfern_core::{
    CANONICAL_GRID_FILTER_STRENGTH, CanonicalPointStencil, GRID_SCALE_FILTER_CADENCE,
    QuadraticPointStencil, SolutionIndicatorReport,
};
use std::sync::mpsc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AdaptiveFixture {
    /// The ring at the default speed.
    Full,
    /// The ring at a tenth of it.
    Slow,
}

impl AdaptiveFixture {
    pub(super) fn named(name: &str) -> Option<Self> {
        match name {
            "adaptive" => Some(AdaptiveFixture::Full),
            "adaptive-slow" => Some(AdaptiveFixture::Slow),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            AdaptiveFixture::Full => "adaptive",
            AdaptiveFixture::Slow => "adaptive-slow",
        }
    }

    /// The speed ceiling the run is paced under.
    fn speed(self) -> f64 {
        match self {
            AdaptiveFixture::Full => 1.0,
            AdaptiveFixture::Slow => 0.1,
        }
    }

    /// The pulse's width: narrower than the ring's at full speed, which the
    /// authored mesh does not hold, so that the device hands off onto
    /// refined meshes at their own step while the field is loud; natively
    /// `FUNFERN_E2E_PULSE_WIDTH` sets it for a study.
    fn pulse_width(self) -> f32 {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(width) = std::env::var("FUNFERN_E2E_PULSE_WIDTH")
            .ok()
            .and_then(|width| width.parse().ok())
        {
            return width;
        }
        match self {
            AdaptiveFixture::Full => 0.04,
            AdaptiveFixture::Slow => 0.06,
        }
    }

    /// The most of the fine mesh the run may hold, and how far a handoff
    /// while the field holds its error floor's share of its peak may move
    /// its energy. The slow ring keeps AMR-01's quarter and its 1% (it loses
    /// 2.9e-5 at most), and may gain 1e-5: a coarsening while the field holds
    /// 1 to 2% of its peak gained 3e-7 to 3.3e-6 in 6 of 21 runs on lavapipe,
    /// as plan.md says a dormant field's can. The narrower ring is past what
    /// AMR-01 was set on: it
    /// loses 1.5% on the CPU too, and on the device peaks at 14,330
    /// triangles on lavapipe, which estimates more often per simulated
    /// second than a device at the speed ceiling, and loses up to 2.5% on
    /// Metal (2026-11-09). Neither may gain energy.
    fn loop_bounds(self) -> (f64, [f64; 2]) {
        match self {
            AdaptiveFixture::Full => (0.35, [-3.0e-2, 1.0e-6]),
            AdaptiveFixture::Slow => (0.25, [-1.0e-2, 1.0e-5]),
        }
    }

    /// The default box as a new document has it, at the fixture's speed.
    fn document(self) -> TopologyDocument {
        let mut document = TopologyEditor::default().document;
        document.presentation.simulation_speed = self.speed();
        document
    }
}

/// Where the ring is struck: AMR-01's.
const RING_AT: Point2 = Point2 { x: -0.2, y: -0.1 };

/// How long the run goes, simulated: past the ring's leaving, the mesh's
/// dormancy and its coarsening back (AMR-01 goes dormant between 4.1 and
/// 4.5 s).
const DURATION: f64 = 8.0;

/// The default box meshed at the smallest element adaptation allows, 0.02:
/// AMR-01's fine reference.
const FINE_TRIANGLES: usize = 45_203;

/// Cells across the lattice the field's whereabouts are read at, about two
/// to the smallest element allowed, as AMR-01 reads them.
const LATTICE: usize = 64;

/// An element longer than this many authored edges is coarser than the
/// authored mesh, which no field may stand on (AMR-01's).
const EXPOSED_EDGE: f64 = 1.25;

/// How many times the presence floor a lattice point must read for the field
/// to count as standing on such an element. The reach holds an element by its
/// own average density, the lattice reads points, and right at the floor the
/// two part: after the reach restored however few (2026-11-09), 3 slow runs
/// of 10 still read a leftover at 1.00 to 1.03 of the floor on elements 1.25
/// to 1.42 authored edges. A lapse of the reach reads far above it.
const EXPOSURE_MARGIN: f64 = 2.0;

/// Steps between the reference's readings of where its field is.
const WATCH_CADENCE: u64 = 8;

/// How far the device's state may stand from the reference's at any full
/// snapshot the reference reaches: Q, b and the wall's pole states over the
/// most each held in the run, and Q and b against themselves while the
/// field holds its error floor's share of its peak. The study's 20 runs on
/// lavapipe read at most 1.3e-6 and 9.8e-6, the slow case on Metal 1.2e-5;
/// the wall's memory against itself is only read, its worst as it first
/// gathers, at 0.7 s, where f32 rounds a quantity just begun (5.9e-5).
const REPLAY_OVER_PEAK: f64 = 1.0e-5;
const REPLAY_HELD: f64 = 3.0e-5;

/// How far the app's estimate of the device's copy may read from the
/// reference's of its own state, relative, while the field is not dormant:
/// 8.2e-7 at most on lavapipe, 4.2e-6 on Metal.
const ESTIMATE_AGREEMENT: f64 = 1.0e-4;

/// The full-speed ring's peak at least this many authored meshes, and the
/// slow ring's end at most this share of its peak, as AMR-01 asks.
const REFINED_PEAK: f64 = 1.3;
const COARSENED_BACK: f64 = 0.8;

/// A run's readings on the lattice, the field and the complementary field
/// at each point.
type Lattice = Vec<(f64, Point2)>;

/// The truth's readings at one of its times: its index, the fine run's and
/// the coarse run's.
type TruthReading = (usize, Lattice, Lattice);

/// Each part's worst error and the time of it.
pub(super) type Worst = [(f64, f64); 3];

/// What the driver tells the follower, in the device's order.
enum Note {
    Start {
        active: Arc<PreparedTopology>,
        time_step: f64,
        base: u64,
        generation: u64,
        pulse: Fired,
        filtered: bool,
        authored_edge: f64,
        /// Natively under `FUNFERN_E2E_TRUTH`, a study's: the document,
        /// whose box the truth's fine run meshes at the smallest element
        /// allowed.
        truth: Option<Box<TopologyDocument>>,
    },
    /// A full snapshot of the device, to stand with where the reference
    /// reaches it.
    Snapshot(Snapshot),
    /// The device took away the stationary flux of `generation` at `step`.
    Cleanup { generation: u64, step: u64 },
    /// A new generation took the field at `step`, its clock starting at
    /// `origin`.
    Handoff {
        generation: u64,
        step: u64,
        origin: f64,
        target: Arc<PreparedTopology>,
        carried: Carried,
        time_step: f64,
        runtime_serials: [u32; 4],
    },
    /// The app estimated the state of `step`, with its controller as it
    /// stood before.
    Estimate {
        generation: u64,
        step: u64,
        controller: Box<AdaptationController>,
        presentation: Box<PresentationSettings>,
    },
    /// The endpoint's snapshot.
    End(Snapshot),
}

/// A full snapshot as the device copied it.
pub(super) struct Snapshot {
    pub(super) generation: u64,
    pub(super) step: u64,
    pub(super) primary: Vec<f32>,
    pub(super) complementary: Vec<[f32; 2]>,
    pub(super) history: Vec<f32>,
    /// The device's time at the snapshot's step, where its clock shares the
    /// snapshot's epoch.
    pub(super) seconds: Option<f64>,
}

impl Snapshot {
    pub(super) fn of(display: &CanonicalGpuDisplay, epoch: Epoch) -> Self {
        let step = display.full_snapshot_completed_steps();
        Snapshot {
            generation: display.generation,
            step,
            primary: display.primary_flux.clone(),
            complementary: display.complementary_flux.clone(),
            history: display.history_auxiliary().to_vec(),
            seconds: display
                .clock
                .filter(|clock| {
                    clock
                        .accepted_steps
                        .saturating_sub(u64::from(clock.step_in_epoch))
                        == epoch.step
                })
                .map(|clock| {
                    clock.epoch_origin_seconds
                        + step.saturating_sub(epoch.step) as f64 * f64::from(clock.time_step)
                }),
        }
    }
}

/// What the follower answers.
enum Heard {
    Estimated {
        step: u64,
        report: Box<SolutionIndicatorReport>,
    },
    Failed(String),
    Ended(Box<Followed>),
}

/// What the reference found over the run.
#[derive(Clone, Debug, Default)]
struct Followed {
    compared: usize,
    /// Snapshots the reference had already stepped past when they came.
    skipped: usize,
    /// Each comparison: Q's, b's and the wall z's distance from the
    /// device's and the reference's norm.
    comparisons: Vec<Compared>,
    drift: f64,
    /// The most lattice points where the field stood on an element coarser
    /// than the authored mesh, and when.
    exposed: usize,
    exposed_at: f64,
    /// The first readings that found it, each its time, where, its density
    /// over the floor and its element's longest edge over the authored one.
    exposures: Vec<(f64, Point2, f64, f64)>,
    /// Each handoff: its time, the carry's change of the energy, the
    /// cleanup's after it, and the energy over its peak.
    handoffs: Vec<Carry>,
    endpoint: [f64; 3],
    end_time: f64,
    /// A study's: at each of the truth's times, the fine run's norm on the
    /// lattice and the adapted and the coarse run's errors against it.
    truth: Vec<Truthful>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Compared {
    pub(super) time: f64,
    /// Whether the field held its error floor's share of its peak energy.
    pub(super) held: bool,
    pub(super) parts: [[f64; 2]; 3],
}

/// The comparisons read two ways: each part's worst distance over the most
/// it held at any comparison, which a drained field's residue reads its
/// rounding against; and, while the field held and the part a hundredth
/// of its most, against itself. Each with its time.
fn replay_errors(comparisons: &[Compared]) -> (Worst, Worst, usize) {
    let mut largest = [0.0_f64; 3];
    for compared in comparisons {
        for (index, [_, norm]) in compared.parts.iter().enumerate() {
            largest[index] = largest[index].max(*norm);
        }
    }
    let mut over_peak = [(0.0, 0.0); 3];
    let mut held = [(0.0, 0.0); 3];
    let mut counted = 0;
    for compared in comparisons {
        counted += usize::from(compared.held);
        for (index, [difference, norm]) in compared.parts.iter().enumerate() {
            let peaked = difference / largest[index];
            if past(peaked, over_peak[index].0) {
                over_peak[index] = (peaked, compared.time);
            }
            let itself = difference / norm;
            if compared.held && *norm >= 1.0e-2 * largest[index] && past(itself, held[index].0) {
                held[index] = (itself, compared.time);
            }
        }
    }
    (over_peak, held, counted)
}

/// Whether `error` is worse than `worst`: past it, or a NaN, the worst there
/// is.
pub(super) fn past(error: f64, worst: f64) -> bool {
    !matches!(
        error.partial_cmp(&worst),
        Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
    )
}

#[derive(Clone, Copy, Debug)]
struct Truthful {
    time: f64,
    norm: f64,
    adapted: f64,
    coarse: f64,
}

#[derive(Clone, Copy, Debug)]
struct Carry {
    time: f64,
    carried: f64,
    cleaned: f64,
    share: f64,
}

/// The step a generation's clock starts at, the time it starts at, and its
/// step.
#[derive(Clone, Copy, Debug)]
pub(super) struct Epoch {
    pub(super) step: u64,
    pub(super) origin: f64,
    pub(super) time_step: f64,
}

impl Epoch {
    fn time(self, step: u64) -> f64 {
        self.origin + step.saturating_sub(self.step) as f64 * self.time_step
    }

    /// The first step at which the run has reached `time`.
    fn step_at(self, time: f64) -> u64 {
        self.step
            + ((time - self.origin) / self.time_step - 1.0e-9)
                .ceil()
                .max(0.0) as u64
    }
}

/// One estimate the app took: its step and time, the cycle the field's
/// reach was drawn for, the app's report and the reference's.
struct Estimate {
    step: u64,
    time: f64,
    cycle: f64,
    app: Option<SolutionIndicatorReport>,
    followed: Option<SolutionIndicatorReport>,
}

/// One generation the run went through: when it took the field, its
/// triangles and step, and for an adapted one the estimate it came from.
struct Generation {
    time: f64,
    triangles: usize,
    time_step: f64,
    /// The estimate's time, and the cycle its reach was drawn for.
    asked: Option<(f64, f64)>,
}

#[derive(Resource)]
pub(super) struct AdaptiveDriver {
    fixture: AdaptiveFixture,
    stage: Stage,
    following: Option<Following>,
    base: u64,
    generation: u64,
    epoch: Epoch,
    /// The running generation's stats, whose log names the steps its events
    /// landed at, and the flux cleanups already told.
    stats: Option<Arc<CanonicalGpuStats>>,
    cleanups: BTreeSet<(u64, u32)>,
    /// The steps of the running generation a cleanup landed at, whose
    /// snapshots could be of either side of it.
    cleanup_steps: BTreeSet<u64>,
    /// The candidate last seen ready, with its maps.
    candidate: Option<(TopologyToken, Carried)>,
    /// The app's controller as the frame before left it.
    controller: Option<AdaptationController>,
    estimated: Option<(u64, u64)>,
    analyzed: Option<u64>,
    estimates: Vec<Estimate>,
    /// The estimate the transaction under way came from: its time and cycle.
    adapting: Option<(f64, f64)>,
    unchanged: usize,
    generations: Vec<Generation>,
    /// The running mesh's elements past [`EXPOSED_EDGE`] authored edges.
    coarser_at_end: usize,
    forwarded: u64,
    authored_triangles: usize,
    authored_edge: f64,
    started: Instant,
    /// When the run's progress was last told.
    told: Instant,
    deadline: Instant,
}

enum Stage {
    Open,
    Install,
    Pulse {
        processed: u32,
    },
    Run,
    /// Held at the endpoint, at rest: its snapshot is asked for.
    Ending {
        seen: u64,
        step: u64,
    },
    /// The endpoint's snapshot went to the follower.
    Awaiting,
    Done,
}

impl Stage {
    fn name(&self) -> &'static str {
        match self {
            Stage::Open => "opening the document",
            Stage::Install => "installing its generation",
            Stage::Pulse { .. } => "waiting for the device to take the pulse",
            Stage::Run => "running and adapting",
            Stage::Ending { .. } => "waiting for the endpoint's snapshot",
            Stage::Awaiting => "waiting for the reference to reach the endpoint",
            Stage::Done => "done",
        }
    }
}

/// Adds the driver of the adaptive fixture named.
pub(super) fn add(app: &mut App, fixture: AdaptiveFixture, seconds: Option<u64>) {
    mark("running", "");
    report(&format!("e2e build {}", crate::BUILD_ID));
    app.insert_resource(AdaptiveDriver {
        fixture,
        stage: Stage::Open,
        following: None,
        base: 0,
        generation: 0,
        epoch: Epoch {
            step: 0,
            origin: 0.0,
            time_step: 1.0,
        },
        stats: None,
        cleanups: BTreeSet::new(),
        cleanup_steps: BTreeSet::new(),
        candidate: None,
        controller: None,
        estimated: None,
        analyzed: None,
        estimates: Vec::new(),
        adapting: None,
        unchanged: 0,
        generations: Vec::new(),
        coarser_at_end: 0,
        forwarded: 0,
        authored_triangles: 0,
        authored_edge: 0.0,
        started: Instant::now(),
        told: Instant::now(),
        deadline: Instant::now() + std::time::Duration::from_secs(seconds.unwrap_or(300)),
    })
    .add_systems(EguiPrimaryContextPass, drive_adaptive.before(frame));
}

fn drive_adaptive(
    mut driver: ResMut<AdaptiveDriver>,
    mut state: ResMut<Playground>,
    mut request: ResMut<CanonicalGpuRequest>,
    display: Res<CanonicalGpuDisplay>,
    mut exit: MessageWriter<AppExit>,
) {
    if matches!(driver.stage, Stage::Done) {
        return;
    }
    let name = driver.fixture.name();
    let run = match advance(&mut driver, &mut state, &mut request, &display) {
        Some(run) => run,
        None if Instant::now() >= driver.deadline => Err(format!(
            "did not finish in time, at {}",
            driver.stage.name()
        )),
        None => {
            // The controller as this frame leaves it, before the app's own
            // frame runs: an estimate it starts is taken against it.
            driver.controller = Some(state.amr.clone());
            return;
        }
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
    driver: &mut AdaptiveDriver,
    state: &mut Playground,
    request: &mut CanonicalGpuRequest,
    display: &CanonicalGpuDisplay,
) -> Option<Result<Run, String>> {
    match std::mem::replace(&mut driver.stage, Stage::Done) {
        Stage::Open => {
            if !state.startup_done {
                driver.stage = Stage::Open;
                return None;
            }
            // Nothing steps until the ring is struck.
            state.e2e_steps.limit = Some(0);
            state.e2e_steps.batch = None;
            state.wave_running = true;
            *FRESH_CLOCK.lock().unwrap() = None;
            if let Err(error) = state.set_document(driver.fixture.document(), false, true) {
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
            if active.canonical_temporal_operator.is_some() {
                return Some(Err("the box prepared a time-varying generation".into()));
            }
            let presentation = state.editor.document.presentation;
            if !presentation.adaptation.enabled || !presentation.grid_scale_filter {
                return Some(Err(
                    "a new document has adaptation and the grid filter on".into()
                ));
            }
            let time_step = state.coordinator.uploaded_time_step;
            driver.base = request.stats().completed_steps();
            driver.generation = request.generation();
            driver.epoch = Epoch {
                step: driver.base,
                origin: 0.0,
                time_step,
            };
            driver.stats = Some(Arc::clone(request.stats()));
            driver.authored_triangles = active.mesh.triangles.len();
            driver.authored_edge = presentation.mesh_edge;
            driver.generations.push(Generation {
                time: 0.0,
                triangles: active.mesh.triangles.len(),
                time_step,
                asked: None,
            });
            report(&format!(
                "e2e {}: {} DOFs, {} triangles, step {time_step:.4e} at speed {}, mesh {}",
                driver.fixture.name(),
                active.canonical_operator.degrees_of_freedom(),
                active.mesh.triangles.len(),
                presentation.simulation_speed,
                mesh_hashes(&active.mesh)
            ));
            state.pulse_width = driver.fixture.pulse_width();
            let pulse = match strike(state) {
                Ok(pulse) => pulse,
                Err(failure) => return Some(Err(failure)),
            };
            let mut following = Following::start();
            #[cfg(not(target_arch = "wasm32"))]
            let truth = std::env::var("FUNFERN_E2E_TRUTH")
                .is_ok()
                .then(|| Box::new(state.editor.document.clone()));
            #[cfg(target_arch = "wasm32")]
            let truth = None;
            following.tell(Note::Start {
                active,
                time_step,
                base: driver.base,
                generation: driver.generation,
                pulse,
                filtered: true,
                authored_edge: driver.authored_edge,
                truth,
            });
            driver.following = Some(following);
            driver.stage = Stage::Pulse {
                processed: request.stats().processed_event(),
            };
            None
        }
        Stage::Pulse { processed } => {
            let untouched = request.stats().processed_event() == processed;
            let queued =
                !state.coordinator.pending_pulses.is_empty() || request.live_event_pending();
            if untouched && !queued {
                return Some(Err(format!("the app dropped the pulse: {}", state.message)));
            }
            if untouched || queued {
                driver.stage = Stage::Pulse { processed };
                return None;
            }
            if let Err(failure) = taken(request, "the pulse", driver.base) {
                return Some(Err(failure));
            }
            state.e2e_steps.limit = Some(driver.epoch.step_at(DURATION));
            driver.stage = Stage::Run;
            None
        }
        Stage::Run => {
            if let Err(failure) = follow(driver, state, request, display) {
                return Some(Err(failure));
            }
            let limit = driver.epoch.step_at(DURATION);
            state.e2e_steps.limit = Some(limit);
            let completed = request.stats().completed_steps();
            if completed < limit || !at_rest(state) {
                driver.stage = Stage::Run;
                return None;
            }
            request.request_full_state_readback();
            driver.stage = Stage::Ending {
                seen: display.full_readbacks,
                step: completed,
            };
            None
        }
        Stage::Ending { seen, step } => {
            if let Err(failure) = follow(driver, state, request, display) {
                return Some(Err(failure));
            }
            // A transaction that finished at the endpoint hands off there,
            // and its step may ask for more of the run.
            let completed = request.stats().completed_steps();
            if completed != step || completed < driver.epoch.step_at(DURATION) || !at_rest(state) {
                driver.stage = Stage::Run;
                return None;
            }
            let arrived = display.full_readbacks > seen
                && display.full_readback_at == display.readbacks
                && display.generation == request.generation()
                && display.generation == driver.generation
                && display.full_snapshot_completed_steps() == step;
            if !arrived {
                request.request_full_state_readback();
                driver.stage = Stage::Ending { seen, step };
                return None;
            }
            let snapshot = Snapshot::of(display, driver.epoch);
            if let Some(active) = state.runtime.active() {
                let coarser = EXPOSED_EDGE * driver.authored_edge;
                driver.coarser_at_end = active
                    .mesh
                    .triangles
                    .iter()
                    .filter(|triangle| {
                        let [a, b, c] = triangle
                            .vertices
                            .map(|vertex| active.mesh.vertices[vertex].point);
                        (b - a).norm().max((c - b).norm()).max((a - c).norm()) > coarser
                    })
                    .count();
            }
            driver
                .following
                .as_mut()
                .expect("following since the install")
                .tell(Note::End(snapshot));
            driver.stage = Stage::Awaiting;
            None
        }
        Stage::Awaiting => {
            if let Err(failure) = follow(driver, state, request, display) {
                return Some(Err(failure));
            }
            let Some(followed) = driver
                .following
                .as_mut()
                .expect("following since the install")
                .ended
                .take()
            else {
                driver.stage = Stage::Awaiting;
                return None;
            };
            Some(judge(driver, &followed).map(|summary| Run {
                summary,
                hash: hash(&display.accepted_storage_bits()),
            }))
        }
        Stage::Done => None,
    }
}

/// Clicks the pulse tool at [`RING_AT`], the app's pulse as it stands: what
/// the reference takes for it.
fn strike(state: &mut Playground) -> Result<Fired, String> {
    let queued = state.coordinator.pending_pulses.len();
    state.place_pulse(RING_AT);
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

/// Whether nothing of adaptation or of the runtime is under way: no
/// estimate, transaction, preparation, upload or cleanup.
fn at_rest(state: &Playground) -> bool {
    !state.estimate_in_flight()
        && !state.adaptation_in_flight()
        && state.amr_indicator_completed.is_none()
        && state.coordinator.uploading.is_none()
        && !state.preparation_in_progress()
        && state.runtime.ready().is_none()
        && state.flux_cleanup.as_ref().is_none_or(|cleanup| {
            matches!(
                cleanup.stage,
                super::super::flux_cleanup::FluxCleanupStage::Landed(_)
            )
        })
}

/// Each frame: what the device and the app did since the last one, told to
/// the follower in the device's order, and what the follower answered.
fn follow(
    driver: &mut AdaptiveDriver,
    state: &Playground,
    request: &CanonicalGpuRequest,
    display: &CanonicalGpuDisplay,
) -> Result<(), String> {
    let code = request.stats().failure();
    if code != 0 {
        return Err(format!("the device failed (failure code {code})"));
    }
    let rejection = request.stats().event_rejection();
    if rejection != 0 {
        return Err(format!(
            "the device refused an event (failure code {rejection})"
        ));
    }
    if driver.told.elapsed().as_secs_f64() >= 10.0 {
        driver.told = Instant::now();
        let completed = request.stats().completed_steps();
        report(&format!(
            "e2e {}: at step {completed}, {:.3} s, the app {:?}, {:.1} s in",
            driver.fixture.name(),
            driver.epoch.time(completed),
            state.amr_status,
            driver.started.elapsed().as_secs_f64()
        ));
    }
    // The candidate's maps, while it is ready: the runtime drops them once
    // its generation is published.
    if let Some(candidate) = state.runtime.ready()
        && driver
            .candidate
            .as_ref()
            .is_none_or(|(token, _)| *token != candidate.bundle.token)
        && let Some(maps) = candidate.canonical_transfer.clone()
    {
        driver.candidate = Some((
            candidate.bundle.token,
            Carried {
                maps,
                interpolation: candidate.transfer.clone(),
            },
        ));
    }
    tell_cleanups(driver);
    // An estimate the app started last frame, at the step of the snapshot
    // it read, against the controller as that frame found it.
    if let Some(source) = state.amr_indicator_source
        && driver.estimated != Some((source.gpu_generation, source.accepted_step))
    {
        driver.estimated = Some((source.gpu_generation, source.accepted_step));
        if source.gpu_generation == driver.generation {
            // The controller as the frame that started the estimate left
            // it, which took the energy of that frame's snapshot first, but
            // for the time of the last estimate, which the estimate moves.
            let Some(before) = &driver.controller else {
                return Err("an estimate began before the fixture saw the controller".into());
            };
            let mut controller = state.amr.clone();
            controller.last_estimate_time = before.last_estimate_time;
            let time = driver.epoch.time(source.accepted_step);
            let presentation = state.editor.document.presentation;
            let cycle =
                controller.reach_horizon(time, presentation.simulation_speed) / AMR_REACH_MARGIN;
            driver.estimates.push(Estimate {
                step: source.accepted_step,
                time,
                cycle,
                app: None,
                followed: None,
            });
            report(&format!(
                "e2e {}: estimate at {time:.3} s, step {}, cycle {cycle:.3}, {:.1} s in",
                driver.fixture.name(),
                source.accepted_step,
                driver.started.elapsed().as_secs_f64()
            ));
            // An estimate starts only once the transaction before it has
            // ended; one that made no mesh ended without a handoff.
            if driver.adapting.take().is_some() {
                driver.unchanged += 1;
            }
            driver.tell(Note::Estimate {
                generation: source.gpu_generation,
                step: source.accepted_step,
                controller: Box::new(controller),
                presentation: Box::new(presentation),
            });
        }
    }
    // The app's answer to an estimate, once it has taken it.
    if state.amr_last_analyzed_step != driver.analyzed {
        driver.analyzed = state.amr_last_analyzed_step;
        if let (Some(step), Some(result)) = (driver.analyzed, &state.amr_indicator_result)
            && state.amr_status != "estimate failed"
            && let Some(estimate) = driver
                .estimates
                .iter_mut()
                .rev()
                .find(|estimate| estimate.step == step)
        {
            estimate.app = Some(result.report.clone());
        }
    }
    // A transaction under way, from the last estimate the app took.
    if state.amr_adaptation_source.is_some() && driver.adapting.is_none() {
        let Some(estimate) = driver.estimates.last() else {
            return Err("a transaction began with no estimate before it".into());
        };
        driver.adapting = Some((estimate.time, estimate.cycle));
    }
    // A new generation, once its clock is on the display.
    let generation = request.generation();
    if generation != driver.generation
        && display.generation == generation
        && state.coordinator.uploading.is_none()
        && let Some(clock) = display.clock
    {
        let Some(active) = state.runtime.active().cloned() else {
            return Err("a generation published with no topology active".into());
        };
        if request.handoff_outcome() != CanonicalGpuHandoffOutcome::Accepted {
            return Err(format!(
                "generation {generation} came by {:?}, not an admitted handoff",
                request.handoff_outcome()
            ));
        }
        let Some((token, carried)) = driver.candidate.take() else {
            return Err(format!("generation {generation} was never seen ready"));
        };
        if token != active.bundle.token {
            return Err(format!(
                "generation {generation} is not the candidate seen ready with its maps"
            ));
        }
        if !active.adapted {
            return Err(format!("generation {generation} is not an adapted mesh"));
        }
        // The last of the old generation's events, then the new one's.
        tell_cleanups(driver);
        let step = clock.accepted_steps - u64::from(clock.step_in_epoch);
        let time_step = state.coordinator.uploaded_time_step;
        driver.epoch = Epoch {
            step,
            origin: clock.epoch_origin_seconds,
            time_step,
        };
        driver.generation = generation;
        driver.stats = Some(Arc::clone(request.stats()));
        driver.cleanup_steps.clear();
        driver.generations.push(Generation {
            time: clock.epoch_origin_seconds,
            triangles: active.mesh.triangles.len(),
            time_step,
            asked: driver.adapting.take(),
        });
        report(&format!(
            "e2e {}: generation {generation} at {:.3} s, step {step}, {} triangles at {time_step:.3e}, \
             {:.1} s in",
            driver.fixture.name(),
            clock.epoch_origin_seconds,
            active.mesh.triangles.len(),
            driver.started.elapsed().as_secs_f64()
        ));
        driver.tell(Note::Handoff {
            generation,
            step,
            origin: clock.epoch_origin_seconds,
            target: active,
            carried,
            time_step,
            runtime_serials: display.runtime_serials,
        });
    }
    tell_cleanups(driver);
    // A full snapshot of the running generation, as it lands, but one of a
    // step a cleanup landed at, which could be of either side of it.
    if display.full_readbacks != driver.forwarded
        && display.full_readback_at == display.readbacks
        && display.generation == driver.generation
    {
        driver.forwarded = display.full_readbacks;
        if !driver
            .cleanup_steps
            .contains(&display.full_snapshot_completed_steps())
        {
            let snapshot = Snapshot::of(display, driver.epoch);
            driver.tell(Note::Snapshot(snapshot));
        }
    }
    let following = driver
        .following
        .as_mut()
        .expect("following since the install");
    while let Some(heard) = following.heard() {
        match heard {
            Heard::Failed(failure) => return Err(failure),
            Heard::Estimated { step, report } => {
                if let Some(estimate) = driver
                    .estimates
                    .iter_mut()
                    .rev()
                    .find(|estimate| estimate.step == step)
                {
                    estimate.followed = Some(*report);
                }
            }
            Heard::Ended(followed) => following.ended = Some(followed),
        }
    }
    Ok(())
}

/// Tells the follower of each flux cleanup the running generation's device
/// encoded since the last frame, at the step it lands at.
fn tell_cleanups(driver: &mut AdaptiveDriver) {
    let Some(stats) = driver.stats.clone() else {
        return;
    };
    for event in stats.encoded_events() {
        if event.is_flux_cleanup() && driver.cleanups.insert((driver.generation, event.serial)) {
            driver.cleanup_steps.insert(event.step);
            driver.tell(Note::Cleanup {
                generation: driver.generation,
                step: event.step,
            });
        }
    }
}

impl AdaptiveDriver {
    fn tell(&mut self, note: Note) {
        self.following
            .as_mut()
            .expect("following since the install")
            .tell(note);
    }
}

/// The run against AMR-01's loop and the reference, with every reading in
/// the summary.
fn judge(driver: &AdaptiveDriver, followed: &Followed) -> Result<String, String> {
    let target = target_accuracy(driver.fixture.document().presentation.adaptation);
    let peak = driver
        .generations
        .iter()
        .map(|generation| generation.triangles)
        .max()
        .unwrap_or(0);
    let last = driver.generations.last().map_or(0, |last| last.triangles);
    // The estimates: the app's of the device's copy against the
    // reference's of its own state.
    let mut agreement: f64 = 0.0;
    let mut decided_apart = Vec::new();
    // An estimate the app dropped, a newer mesh having published while it
    // ran, answers to nothing; one it took, the reference must have read.
    let mut unanswered = 0;
    let mut dropped = 0;
    let mut dormant_at = None;
    let mut refined_after_dormancy = Vec::new();
    for estimate in &driver.estimates {
        let Some(app) = &estimate.app else {
            dropped += 1;
            continue;
        };
        let Some(followed) = &estimate.followed else {
            unanswered += 1;
            continue;
        };
        if !app.dormant && !followed.dormant {
            agreement = agreement.max(
                (app.global_indicator - followed.global_indicator).abs()
                    / followed.global_indicator,
            );
        }
        if adaptation_decision(app, target) != adaptation_decision(followed, target)
            || app.dormant != followed.dormant
        {
            decided_apart.push(format!(
                "{:.3} s: app {:.4}{} {:?}, reference {:.4}{} {:?}",
                estimate.time,
                app.global_indicator,
                if app.dormant { " dormant" } else { "" },
                adaptation_decision(app, target),
                followed.global_indicator,
                if followed.dormant { " dormant" } else { "" },
                adaptation_decision(followed, target),
            ));
        }
        if app.dormant && dormant_at.is_none() {
            dormant_at = Some(estimate.time);
        }
        if dormant_at.is_some()
            && adaptation_decision(app, target) == AmrDecision::Refine
            && !size_rule_refines(app, target)
        {
            refined_after_dormancy.push(estimate.time);
        }
    }
    // Each adapted mesh against the cycle its estimate's reach was drawn
    // for.
    let lags = driver
        .generations
        .iter()
        .filter_map(|generation| {
            generation
                .asked
                .map(|(asked, cycle)| (generation.time - asked, cycle))
        })
        .collect::<Vec<_>>();
    let late = lags
        .iter()
        .map(|(lag, cycle)| lag / cycle)
        .fold(0.0_f64, f64::max);
    let timeline = driver
        .generations
        .iter()
        .map(|generation| {
            format!(
                "{:.3} s {} at {:.3e}{}",
                generation.time,
                generation.triangles,
                generation.time_step,
                generation
                    .asked
                    .map_or(String::new(), |(asked, cycle)| format!(
                        " ({:.3} s after its estimate, cycle {cycle:.3})",
                        generation.time - asked
                    ))
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let carries = followed
        .handoffs
        .iter()
        .filter(|carry| carry.share >= 1.0e-2)
        .map(|carry| carry.carried + carry.cleaned)
        .collect::<Vec<_>>();
    let carry_range = carries
        .iter()
        .fold([f64::INFINITY, f64::NEG_INFINITY], |range, value| {
            [range[0].min(*value), range[1].max(*value)]
        });
    let dormant_estimates = driver
        .estimates
        .iter()
        .filter(|estimate| estimate.app.as_ref().is_some_and(|app| app.dormant))
        .count();
    let (over_peak, held, counted) = replay_errors(&followed.comparisons);
    let summary = format!(
        "Q {:.3e}, b {:.3e}, z {:.3e} at {:.3} s; over {} snapshots ({} passed) the worst over \
         the most each held Q {:.3e} at {:.3} s, b {:.3e} at {:.3} s, z {:.3e} at {:.3} s, and \
         over {} while the field held, against itself, Q {:.3e} at {:.3} s, b {:.3e} at {:.3} s, \
         z {:.3e} at {:.3} s; clock {:.1e}; {} estimates ({} dormant, {} dropped, {} unanswered), \
         agreeing within {agreement:.2e}{}; {} triangles authored, peak {peak}, end {last}; \
         dormant from {}; {} handoffs, {} made nothing, lag over cycle at most {late:.2}; \
         handoff energy {:.2e} to {:.2e} over {} while held; field on coarser elements, from \
         twice the floor, at \
         most {} lattice points{}; {} elements past the authored size at the end; timeline \
         {timeline}",
        followed.endpoint[0],
        followed.endpoint[1],
        followed.endpoint[2],
        followed.end_time,
        followed.compared,
        followed.skipped,
        over_peak[0].0,
        over_peak[0].1,
        over_peak[1].0,
        over_peak[1].1,
        over_peak[2].0,
        over_peak[2].1,
        counted,
        held[0].0,
        held[0].1,
        held[1].0,
        held[1].1,
        held[2].0,
        held[2].1,
        followed.drift,
        driver.estimates.len(),
        dormant_estimates,
        dropped,
        unanswered,
        if decided_apart.is_empty() {
            String::new()
        } else {
            format!(", deciding apart at {}", decided_apart.join("; "))
        },
        driver.authored_triangles,
        dormant_at.map_or("never".to_owned(), |time| format!("{time:.3} s")),
        driver.generations.len() - 1,
        driver.unchanged,
        carry_range[0],
        carry_range[1],
        carries.len(),
        followed.exposed,
        if followed.exposed > 0 {
            format!(" at {:.3} s", followed.exposed_at)
        } else {
            String::new()
        },
        driver.coarser_at_end,
    );
    report(&format!("e2e {}: STUDY {summary}", driver.fixture.name()));
    report(&format!(
        "e2e {}: STUDY estimates (time, app's estimate, d for dormant, decision) {}",
        driver.fixture.name(),
        driver
            .estimates
            .iter()
            .filter_map(|estimate| estimate.app.as_ref().map(|app| (estimate.time, app)))
            .map(|(time, app)| format!(
                "{time:.3} {:.4}{}{} {}",
                app.global_indicator,
                if app.dormant { " d" } else { "" },
                if app.limit_refine_candidates > 0 {
                    format!(" l{}", app.limit_refine_candidates)
                } else {
                    String::new()
                },
                match adaptation_decision(app, target) {
                    AmrDecision::Hold => "hold",
                    AmrDecision::Refine if size_rule_refines(app, target) => "limits",
                    AmrDecision::Refine => "refine",
                    AmrDecision::Coarsen => "coarsen",
                }
            ))
            .collect::<Vec<_>>()
            .join("; ")
    ));
    if !followed.exposures.is_empty() {
        report(&format!(
            "e2e {}: STUDY exposures (time, where, density over the floor, edge over the \
             authored) {}",
            driver.fixture.name(),
            followed
                .exposures
                .iter()
                .map(|(time, point, density, edge)| format!(
                    "{time:.3} ({:.3}, {:.3}) {density:.2} {edge:.2}",
                    point.x, point.y
                ))
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    report(&format!(
        "e2e {}: STUDY handoffs (time, carry, cleanup, energy over its peak) {}",
        driver.fixture.name(),
        followed
            .handoffs
            .iter()
            .map(|carry| format!(
                "{:.3} {:+.2e} {:+.2e} {:.2e}",
                carry.time, carry.carried, carry.cleaned, carry.share
            ))
            .collect::<Vec<_>>()
            .join("; ")
    ));
    if !followed.truth.is_empty() {
        // AMR-01's window: while the fine run's norm on the lattice is
        // above nine tenths of its largest.
        let largest = followed
            .truth
            .iter()
            .map(|truth| truth.norm)
            .fold(0.0_f64, f64::max);
        let inside = followed
            .truth
            .iter()
            .filter(|truth| truth.norm >= 0.9 * largest)
            .collect::<Vec<_>>();
        let worst = inside
            .iter()
            .map(|truth| truth.adapted - truth.coarse)
            .fold(f64::NEG_INFINITY, f64::max);
        report(&format!(
            "e2e {}: STUDY truth: inside to {:.2} s, adapted less coarse at worst {worst:+.4}; \
             (time, norm over largest, adapted, coarse) {}",
            driver.fixture.name(),
            inside.last().map_or(0.0, |truth| truth.time),
            followed
                .truth
                .iter()
                .map(|truth| format!(
                    "{:.2} {:.2} {:.4} {:.4}",
                    truth.time,
                    truth.norm / largest,
                    truth.adapted,
                    truth.coarse
                ))
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    let fail = |what: String| Err(format!("{summary}: {what}"));
    let over = over_peak
        .iter()
        .map(|(error, _)| *error)
        .fold(0.0_f64, worse);
    let itself = worse(held[0].0, held[1].0);
    if !within(over, REPLAY_OVER_PEAK) || !within(itself, REPLAY_HELD) {
        return fail(format!(
            "the device stands from the reference beyond {REPLAY_OVER_PEAK:.0e} of the most \
             it held or {REPLAY_HELD:.0e} of itself while the field held"
        ));
    }
    if !within(followed.drift, CLOCK_TOLERANCE) {
        return fail(format!("the clock drifts beyond {CLOCK_TOLERANCE:.0e}"));
    }
    if unanswered > 0 {
        return fail("the reference did not read an estimate the app took".into());
    }
    if !within(agreement, ESTIMATE_AGREEMENT) || !decided_apart.is_empty() {
        return fail(format!(
            "the app's estimates of the device's copies part from the reference's beyond \
             {ESTIMATE_AGREEMENT:.0e}, or decide otherwise"
        ));
    }
    match driver.fixture {
        AdaptiveFixture::Full
            if (peak as f64) < REFINED_PEAK * driver.authored_triangles as f64 =>
        {
            return fail(format!(
                "the mesh never refined to {REFINED_PEAK} of the authored one"
            ));
        }
        AdaptiveFixture::Slow if driver.coarser_at_end == 0 => {
            return fail("no quiet region grew past the authored mesh".into());
        }
        _ => {}
    }
    let (share, handoff) = driver.fixture.loop_bounds();
    if peak as f64 > share * FINE_TRIANGLES as f64 {
        return fail(format!(
            "the mesh passed {share} of the fine one's {FINE_TRIANGLES}"
        ));
    }
    if !refined_after_dormancy.is_empty() {
        return fail(format!(
            "the mesh refined for its error after it went dormant, at {refined_after_dormancy:?}"
        ));
    }
    // The narrower ring's leftover drains past the run on Metal, whose
    // estimates come at the cycle the speed ceiling sets; the slow ring's
    // goes dormant at 4.3 s everywhere.
    if driver.fixture == AdaptiveFixture::Slow {
        if dormant_at.is_none() {
            return fail("the field never went dormant".into());
        }
        if last as f64 > COARSENED_BACK * peak as f64 {
            return fail(format!(
                "the mesh did not come back to {COARSENED_BACK} of its peak"
            ));
        }
    }
    if followed.exposed > 0 {
        return fail("the field stood on elements coarser than the authored mesh".into());
    }
    if late > 1.0 {
        return fail(
            "a mesh landed later than the cycle its estimate's reach was drawn for".into(),
        );
    }
    if !carries.is_empty() && (carry_range[0] < handoff[0] || carry_range[1] > handoff[1]) {
        return fail(format!(
            "a handoff moved the energy outside {:.0e} to {:.0e}",
            handoff[0], handoff[1]
        ));
    }
    Ok(summary)
}

/// The follower, on its own thread where there are threads, so that the
/// reference's steps and estimates leave the app's frames their pace.
struct Following {
    #[cfg(not(target_arch = "wasm32"))]
    notes: mpsc::Sender<Note>,
    #[cfg(not(target_arch = "wasm32"))]
    heard: std::sync::Mutex<mpsc::Receiver<Heard>>,
    #[cfg(target_arch = "wasm32")]
    follower: Follower,
    #[cfg(target_arch = "wasm32")]
    heard: VecDeque<Heard>,
    ended: Option<Box<Followed>>,
}

impl Following {
    fn start() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (notes, inbox) = mpsc::channel::<Note>();
            let (outbox, heard) = mpsc::channel();
            std::thread::Builder::new()
                .name("e2e-follower".into())
                .spawn(move || {
                    let mut follower = Follower::default();
                    for note in inbox {
                        for heard in follower.hear(note) {
                            if outbox.send(heard).is_err() {
                                return;
                            }
                        }
                    }
                })
                .expect("the follower's thread");
            Following {
                notes,
                heard: std::sync::Mutex::new(heard),
                ended: None,
            }
        }
        #[cfg(target_arch = "wasm32")]
        Following {
            follower: Follower::default(),
            heard: VecDeque::new(),
            ended: None,
        }
    }

    fn tell(&mut self, note: Note) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            // A follower that stopped has said why.
            let _ = self.notes.send(note);
        }
        #[cfg(target_arch = "wasm32")]
        self.heard.extend(self.follower.hear(note));
    }

    fn heard(&mut self) -> Option<Heard> {
        #[cfg(not(target_arch = "wasm32"))]
        return self.heard.get_mut().ok()?.try_recv().ok();
        #[cfg(target_arch = "wasm32")]
        return self.heard.pop_front();
    }
}

/// The reference and what it reads, once started; stopped for good at its
/// first failure.
#[derive(Default)]
struct Follower {
    track: Option<Track>,
    failed: bool,
}

struct Track {
    reference: Reference,
    /// The state a step before the reference's, on its generation.
    previous: Option<State>,
    base: u64,
    filtered: bool,
    authored_edge: f64,
    points: Vec<Point2>,
    /// The lattice's probes on the running generation, each with the
    /// longest edge of its element.
    probes: Vec<(CanonicalPointStencil, f64)>,
    largest_mean: f64,
    peak_energy: f64,
    /// The energy before the last handoff, which its cleanup is read
    /// against.
    handed_from: f64,
    followed: Followed,
    /// A study's: the adapted run's lattice readings at the truth's times,
    /// and the truth's, each until the other's comes.
    sampler: Option<Sampler>,
    adapted: BTreeMap<usize, Lattice>,
    truth: Option<std::sync::Mutex<mpsc::Receiver<TruthReading>>>,
    truths: BTreeMap<usize, (Lattice, Lattice)>,
}

impl Follower {
    fn hear(&mut self, note: Note) -> Vec<Heard> {
        if self.failed {
            return Vec::new();
        }
        match self.answer(note) {
            Ok(heard) => heard.into_iter().collect(),
            Err(failure) => {
                self.failed = true;
                vec![Heard::Failed(format!("the reference: {failure}"))]
            }
        }
    }

    fn answer(&mut self, note: Note) -> Result<Option<Heard>, String> {
        if let Note::Start {
            active,
            time_step,
            base,
            generation,
            pulse,
            filtered,
            authored_edge,
            truth,
        } = note
        {
            let domain = active.bundle.authored.geometry.domain;
            let points = (0..LATTICE * LATTICE)
                .map(|index| {
                    Point2::new(
                        domain.min_x
                            + domain.width() * ((index % LATTICE) as f64 + 0.5) / LATTICE as f64,
                        domain.min_y
                            + domain.height() * ((index / LATTICE) as f64 + 0.5) / LATTICE as f64,
                    )
                })
                .collect::<Vec<_>>();
            let probes = probes(&active, &points)?;
            let zero = State::zero(&active, time_step)?;
            let truth = truth.map(|document| {
                let (outbox, inbox) = mpsc::channel();
                let (coarse, points) = (active.clone(), points.clone());
                std::thread::Builder::new()
                    .name("e2e-truth".into())
                    .spawn(move || {
                        let run =
                            run_truth(&document, pulse, coarse, time_step, base, &points, &outbox);
                        if let Err(failure) = run {
                            report(&format!("e2e: the truth failed: {failure}"));
                        }
                    })
                    .expect("the truth's thread");
                std::sync::Mutex::new(inbox)
            });
            let mut reference = Reference::starting(active, zero, generation);
            reference.take(pulse)?;
            self.track = Some(Track {
                reference,
                previous: None,
                base,
                filtered,
                authored_edge,
                points,
                probes,
                largest_mean: 0.0,
                peak_energy: 0.0,
                handed_from: 0.0,
                followed: Followed::default(),
                sampler: truth.is_some().then(Sampler::default),
                adapted: BTreeMap::new(),
                truth,
                truths: BTreeMap::new(),
            });
            if let Some(track) = &mut self.track {
                track.sample()?;
            }
            return Ok(None);
        }
        let Some(track) = &mut self.track else {
            return Err("a note before the start".into());
        };
        match note {
            Note::Start { .. } => unreachable!("taken above"),
            Note::Snapshot(snapshot) => {
                if snapshot.generation != track.reference.generation {
                    return Err(format!(
                        "a snapshot of generation {} while the reference stands on {}",
                        snapshot.generation, track.reference.generation
                    ));
                }
                if snapshot.step < track.base + track.reference.steps {
                    track.followed.skipped += 1;
                    return Ok(None);
                }
                track.step_to(snapshot.step)?;
                track.compare(&snapshot)?;
                Ok(None)
            }
            Note::Cleanup { generation, step } => {
                if generation != track.reference.generation {
                    return Err(format!(
                        "a cleanup on generation {generation} while the reference stands on {}",
                        track.reference.generation
                    ));
                }
                track.step_to(step)?;
                track.clean()?;
                Ok(None)
            }
            Note::Handoff {
                generation,
                step,
                origin,
                target,
                carried,
                time_step,
                runtime_serials,
            } => {
                track.step_to(step)?;
                let reached = track.reference.state.time();
                let drift = (reached - origin).abs();
                if !within(drift, CLOCK_TOLERANCE) {
                    return Err(format!(
                        "generation {generation} starts its clock at {origin:.6} s, {drift:.1e} \
                         from the {reached:.6} s the run reached at step {step}"
                    ));
                }
                let before = track.energy()?;
                let (handed, _, target) = hand_off(
                    &track.reference,
                    &target,
                    &carried,
                    time_step,
                    runtime_serials,
                    None,
                )
                .map_err(|error| format!("the handoff at step {step}: {error}"))?;
                track.reference.state = handed;
                track.reference.active = target;
                track.reference.generation = generation;
                track.previous = None;
                track.probes = probes(&track.reference.active, &track.points)?;
                let after = track.energy()?;
                track.handed_from = before;
                track.followed.handoffs.push(Carry {
                    time: reached,
                    carried: after / before - 1.0,
                    cleaned: 0.0,
                    share: before / track.peak_energy,
                });
                Ok(None)
            }
            Note::Estimate {
                generation,
                step,
                controller,
                presentation,
            } => {
                if generation != track.reference.generation {
                    return Err(format!(
                        "an estimate of generation {generation} while the reference stands on {}",
                        track.reference.generation
                    ));
                }
                track.step_to(step)?;
                if track.base + track.reference.steps != step {
                    return Err(format!(
                        "the estimate of step {step} came with the reference at {}",
                        track.base + track.reference.steps
                    ));
                }
                let (State::Fixed(now), Some(State::Fixed(before))) =
                    (&track.reference.state, &track.previous)
                else {
                    return Err(format!(
                        "the app estimated step {step}, which has no step before it on its \
                         generation"
                    ));
                };
                let active = &track.reference.active;
                let snapshots = IndicatorSnapshots::of_fixed_run(active, now, before)
                    .map_err(|error| error.to_string())?;
                let mut controller = *controller;
                let mut job = controller.estimate(active, snapshots, &presentation);
                let result = loop {
                    if let Some(result) = job.advance(1 << 16) {
                        break result.map_err(|error| error.to_string())?;
                    }
                };
                Ok(Some(Heard::Estimated {
                    step,
                    report: Box::new(result.report),
                }))
            }
            Note::End(snapshot) => {
                if snapshot.generation != track.reference.generation {
                    return Err(format!(
                        "the endpoint is of generation {} while the reference stands on {}",
                        snapshot.generation, track.reference.generation
                    ));
                }
                track.step_to(snapshot.step)?;
                let errors = track.compare(&snapshot)?;
                track.followed.endpoint = errors;
                track.followed.end_time = track.reference.state.time();
                track.read_truth(true)?;
                Ok(Some(Heard::Ended(Box::new(track.followed.clone()))))
            }
        }
    }
}

impl Track {
    /// Steps the reference to the device's accepted step `step`, filtering
    /// on the device's cadence and reading where its field is.
    fn step_to(&mut self, step: u64) -> Result<(), String> {
        let Some(target) = step.checked_sub(self.base) else {
            return Err(format!("step {step} is before the run's start"));
        };
        if target < self.reference.steps {
            return Err(format!(
                "step {step} is behind the reference at {}",
                self.base + self.reference.steps
            ));
        }
        while self.reference.steps < target {
            self.previous = Some(self.reference.state.clone());
            self.reference.advance_to(self.reference.steps + 1)?;
            let absolute = self.base + self.reference.steps;
            if self.filtered && absolute.is_multiple_of(GRID_SCALE_FILTER_CADENCE) {
                self.reference.state.filter(&self.reference.active)?;
            }
            if absolute.is_multiple_of(WATCH_CADENCE) {
                self.watch()?;
            }
            self.sample()?;
        }
        Ok(())
    }

    /// A study's: the adapted run's lattice readings where the truth reads
    /// its own, and the errors of each time both have.
    fn sample(&mut self) -> Result<(), String> {
        let Some(sampler) = &mut self.sampler else {
            return Ok(());
        };
        let state = &self.reference.state;
        let read = self
            .probes
            .iter()
            .map(|(probe, _)| {
                probe
                    .sample(state.primary(), state.primary(), state.complementary())
                    .map(|sample| (sample.primary, sample.complementary))
                    .map_err(|error| format!("{error:?}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        for (index, read) in sampler.push(state.time(), read) {
            self.adapted.insert(index, read);
        }
        self.read_truth(false)
    }

    /// Takes in the truth's readings; at the endpoint, waits for those the
    /// adapted run's still lack, or for the truth to stop.
    fn read_truth(&mut self, waiting: bool) -> Result<(), String> {
        let Some(inbox) = &self.truth else {
            return Ok(());
        };
        let inbox = inbox
            .lock()
            .map_err(|_| "the truth's channel was poisoned".to_owned())?;
        loop {
            let lacking = waiting && !self.adapted.is_empty();
            let next = if lacking {
                inbox.recv().ok()
            } else {
                inbox.try_recv().ok()
            };
            let Some((index, fine, coarse)) = next else {
                break;
            };
            self.truths.insert(index, (fine, coarse));
            if lacking
                && self
                    .adapted
                    .keys()
                    .all(|index| self.truths.contains_key(index))
            {
                break;
            }
        }
        let both = self
            .adapted
            .keys()
            .copied()
            .filter(|index| self.truths.contains_key(index))
            .collect::<Vec<_>>();
        for index in both {
            let adapted = self.adapted.remove(&index).expect("listed");
            let (fine, coarse) = self.truths.remove(&index).expect("listed");
            let norm = fine
                .iter()
                .map(|(field, flux)| field * field + flux.dot(*flux))
                .sum::<f64>();
            let apart = |read: &[(f64, Point2)]| {
                (read
                    .iter()
                    .zip(&fine)
                    .map(|((field, flux), (expected, expected_flux))| {
                        let along = *flux - *expected_flux;
                        (field - expected).powi(2) + along.dot(along)
                    })
                    .sum::<f64>()
                    / norm)
                    .sqrt()
            };
            self.followed.truth.push(Truthful {
                time: index as f64 * TRUTH_INTERVAL,
                norm: norm.sqrt(),
                adapted: apart(&adapted),
                coarse: apart(&coarse),
            });
        }
        Ok(())
    }

    fn energy(&self) -> Result<f64, String> {
        match &self.reference.state {
            State::Fixed(state) => state
                .energy(&self.reference.active.canonical_operator)
                .map_err(|error| error.to_string()),
            State::Temporal(_) => Err("the box runs fixed".into()),
        }
    }

    /// Where the field is against the mesh it runs on: the lattice's
    /// densities, as AMR-01 reads them, against 1% of their largest mean,
    /// on elements past the authored size.
    fn watch(&mut self) -> Result<(), String> {
        let state = &self.reference.state;
        let densities = self
            .probes
            .iter()
            .map(|(probe, _)| {
                probe
                    .sample(state.primary(), state.primary(), state.complementary())
                    .map(|sample| {
                        sample.primary * sample.primary
                            + sample.complementary.dot(sample.complementary)
                    })
                    .map_err(|error| format!("{error:?}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mean = densities.iter().sum::<f64>() / densities.len() as f64;
        self.largest_mean = self.largest_mean.max(mean);
        self.peak_energy = self.peak_energy.max(self.energy()?);
        let floor = AMR_FIELD_PRESENCE_RATIO * self.largest_mean;
        let coarser = EXPOSED_EDGE * self.authored_edge;
        let exposed = densities
            .iter()
            .zip(&self.probes)
            .zip(&self.points)
            .filter(|((density, (_, longest)), _)| **density > floor && *longest > coarser)
            .inspect(|((density, (_, longest)), point)| {
                if self.followed.exposures.len() < 24 {
                    self.followed.exposures.push((
                        self.reference.state.time(),
                        **point,
                        *density / floor,
                        *longest / self.authored_edge,
                    ));
                }
            })
            .filter(|((density, _), _)| **density > EXPOSURE_MARGIN * floor)
            .count();
        if exposed > self.followed.exposed {
            self.followed.exposed = exposed;
            self.followed.exposed_at = self.reference.state.time();
        }
        Ok(())
    }

    /// Takes away the stationary flux the device did, read off the
    /// reference's own state: the part is the same at every step of the
    /// fixed lossless generation, and the filter leaves it alone.
    fn clean(&mut self) -> Result<(), String> {
        let before = self.energy()?;
        let State::Fixed(state) = &mut self.reference.state else {
            return Err("the device took a driven generation's stationary flux away".into());
        };
        let operator = &self.reference.active.canonical_operator;
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
        let after = self.energy()?;
        if let Some(carry) = self.followed.handoffs.last_mut()
            && self.handed_from > 0.0
        {
            carry.cleaned += (after - before) / self.handed_from;
        }
        Ok(())
    }

    /// The device's snapshot against the reference where it stands: Q, b
    /// and the wall's pole states, relative, and the clock.
    fn compare(&mut self, snapshot: &Snapshot) -> Result<[f64; 3], String> {
        let state = &self.reference.state;
        if snapshot.primary.len() != state.primary().len()
            || snapshot.complementary.len() != state.complementary().len()
        {
            return Err(format!(
                "the snapshot of step {} holds {} and {} values where the generation has {} \
                 and {}",
                snapshot.step,
                snapshot.primary.len(),
                snapshot.complementary.len(),
                state.primary().len(),
                state.complementary().len()
            ));
        }
        // Each part's distance and the reference's norm.
        let apart = |device: &mut dyn Iterator<Item = f64>,
                     expected: &mut dyn Iterator<Item = f64>| {
            device
                .zip(expected)
                .fold([0.0, 0.0], |[difference, norm], (device, expected)| {
                    [
                        difference + (device - expected).powi(2),
                        norm + expected * expected,
                    ]
                })
                .map(f64::sqrt)
        };
        let parts = [
            apart(
                &mut snapshot.primary.iter().map(|value| f64::from(*value)),
                &mut state.primary().iter().copied(),
            ),
            apart(
                &mut snapshot
                    .complementary
                    .iter()
                    .flat_map(|value| value.iter().map(|lane| f64::from(*lane))),
                &mut state
                    .complementary()
                    .iter()
                    .flat_map(|value| [value.x, value.y]),
            ),
            apart(
                &mut snapshot.history.iter().map(|value| f64::from(*value)),
                &mut state.outgoing_z().iter().copied(),
            ),
        ];
        // While the field holds its error floor's share of its peak, each
        // part against itself; at any time, against the most it has held,
        // as a drained field's residue reads its rounding against itself.
        let held = self.energy()? >= AMR_FIELD_PRESENCE_RATIO * self.peak_energy;
        let errors = parts.map(|[difference, norm]| difference / norm);
        self.followed.compared += 1;
        self.followed.comparisons.push(Compared {
            time: state.time(),
            held,
            parts,
        });
        if let Some(seconds) = snapshot.seconds {
            self.followed.drift = self.followed.drift.max((seconds - state.time()).abs());
        }
        Ok(errors)
    }
}

impl State {
    /// The grid filter the device runs every few steps, at its strength.
    fn filter(&mut self, active: &PreparedTopology) -> Result<(), String> {
        match self {
            State::Fixed(state) => state
                .apply_grid_filter(
                    &active.canonical_operator,
                    &active.canonical_forcing,
                    CANONICAL_GRID_FILTER_STRENGTH,
                )
                .map(|_| ())
                .map_err(|error| error.to_string()),
            State::Temporal(_) => Err("the box runs fixed".into()),
        }
    }
}

/// The lattice's probes on `active`, each with its element's longest edge.
fn probes(
    active: &PreparedTopology,
    points: &[Point2],
) -> Result<Vec<(CanonicalPointStencil, f64)>, String> {
    let mesh = &active.mesh;
    points
        .iter()
        .map(|point| {
            let stencil = QuadraticPointStencil::build_topology(
                mesh,
                &active.operator,
                &active.bundle.plan,
                active.fixed_model(),
                *point,
            )
            .map_err(|error| format!("no element takes the lattice point {point:?}: {error:?}"))?;
            let [a, b, c] = mesh.triangles[stencil.element as usize]
                .vertices
                .map(|vertex| mesh.vertices[vertex].point);
            let longest = (b - a).norm().max((c - b).norm()).max((a - c).norm());
            let probe = CanonicalPointStencil::from_quadratic(stencil, &active.canonical_operator)
                .map_err(|error| format!("{error:?}"))?;
            Ok((probe, longest))
        })
        .collect()
}

/// A study's truth: the times it reads the runs at, the interval apart,
/// and the last, past which the field has left the box (AMR-01 stops its
/// fine run at 2.5 s).
const TRUTH_INTERVAL: f64 = 0.02;
const TRUTH_SAMPLES: usize = 125;

/// A run's lattice readings at [`TRUTH_INTERVAL`]'s times, each read off
/// the parabola through the three last steps around it.
#[derive(Default)]
struct Sampler {
    last: VecDeque<(f64, Lattice)>,
    next: usize,
}

impl Sampler {
    fn push(&mut self, time: f64, read: Lattice) -> Vec<(usize, Lattice)> {
        self.last.push_back((time, read));
        if self.last.len() > 3 {
            self.last.pop_front();
        }
        let mut emitted = Vec::new();
        if self.next == 0 {
            self.next = 1;
        }
        while self.last.len() == 3 && self.next <= TRUTH_SAMPLES {
            let at = self.next as f64 * TRUTH_INTERVAL;
            if at > time + 1.0e-12 {
                break;
            }
            let [(t0, r0), (t1, r1), (t2, r2)] = [&self.last[0], &self.last[1], &self.last[2]];
            let weights = [
                (at - t1) * (at - t2) / ((t0 - t1) * (t0 - t2)),
                (at - t0) * (at - t2) / ((t1 - t0) * (t1 - t2)),
                (at - t0) * (at - t1) / ((t2 - t0) * (t2 - t1)),
            ];
            let read = r0
                .iter()
                .zip(r1)
                .zip(r2)
                .map(|((a, b), c)| {
                    (
                        weights[0] * a.0 + weights[1] * b.0 + weights[2] * c.0,
                        a.1 * weights[0] + b.1 * weights[1] + c.1 * weights[2],
                    )
                })
                .collect();
            emitted.push((self.next, read));
            self.next += 1;
        }
        emitted
    }
}

/// A study's truth, on its own thread: the box meshed at the smallest
/// element adaptation allows, stepped at half its own step, and the
/// authored mesh at the app's step, as the app runs it with adaptation off,
/// both struck as the device was and filtered on their cadence, read on the
/// lattice at the truth's times.
fn run_truth(
    document: &TopologyDocument,
    pulse: Fired,
    coarse: Arc<PreparedTopology>,
    coarse_step: f64,
    base: u64,
    points: &[Point2],
    outbox: &mpsc::Sender<TruthReading>,
) -> Result<(), String> {
    let started = Instant::now();
    let mut editor = TopologyEditor::from_document(document.clone())
        .map_err(|error| format!("the document does not load: {error}"))?;
    for _ in 0..100_000 {
        editor.validate_frame(64);
        if editor.acceptance != crate::topology_editor::TopologyAcceptance::Pending {
            break;
        }
    }
    let domain = editor.document.model.accepted.geometry.domain;
    let edge = editor.document.presentation.adaptation.minimum_edge;
    let options = MeshingOptions {
        target_edge_length: edge,
        curve_tolerance: (edge * 0.02).min(5e-4),
        ..MeshingOptions::default()
    }
    .sized_for_area(domain.width() * domain.height());
    let mut runtime = crate::topology_runtime::TopologyRuntime::default();
    runtime.request(
        editor.revision,
        &editor.document,
        editor.compiled_accepted.clone(),
        options,
        true,
    )?;
    let token = loop {
        if let Some(outcome) = runtime.advance(1 << 12) {
            break outcome.map_err(|error| error.to_string())?;
        }
    };
    let fine = runtime.commit_ready(token)?;
    let fine_step = 0.5 * fine.recommended_time_step();
    report(&format!(
        "e2e: the truth meshed {} triangles in {:.1} s, steps at {fine_step:.3e}",
        fine.mesh.triangles.len(),
        started.elapsed().as_secs_f64()
    ));
    let mut runs = [(fine, fine_step, 0_u64), (coarse, coarse_step, base)].map(
        |(active, time_step, base)| -> Result<_, String> {
            let Fired::Pulse {
                position,
                region,
                amplitude,
                width,
            } = pulse
            else {
                return Err("the run is struck by a pulse".into());
            };
            let mut state = State::zero(&active, time_step)?;
            state.pulse(
                &active,
                &pulse_increment(&active, position, region, amplitude, width)?,
            )?;
            let probes = probes(&active, points)?;
            Ok((active, state, probes, base, Sampler::default()))
        },
    );
    let mut readings: [BTreeMap<usize, Lattice>; 2] = Default::default();
    for (index, run) in runs.iter_mut().enumerate() {
        let (active, state, probes, base, sampler) = run.as_mut().map_err(|error| error.clone())?;
        let mut steps = 0_u64;
        while sampler.next <= TRUTH_SAMPLES {
            state.step(active)?;
            steps += 1;
            if (*base + steps).is_multiple_of(GRID_SCALE_FILTER_CADENCE) {
                state.filter(active)?;
            }
            let read = probes
                .iter()
                .map(|(probe, _)| {
                    probe
                        .sample(state.primary(), state.primary(), state.complementary())
                        .map(|sample| (sample.primary, sample.complementary))
                        .map_err(|error| format!("{error:?}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            for (at, read) in sampler.push(state.time(), read) {
                readings[index].insert(at, read);
            }
            // The fine run's readings go out with the coarse's, which
            // comes second and quickly.
            if index == 1 {
                let ready = readings[1]
                    .keys()
                    .copied()
                    .filter(|at| readings[0].contains_key(at))
                    .collect::<Vec<_>>();
                for at in ready {
                    let fine = readings[0].remove(&at).expect("listed");
                    let coarse = readings[1].remove(&at).expect("listed");
                    if outbox.send((at, fine, coarse)).is_err() {
                        return Ok(());
                    }
                }
            }
        }
    }
    report(&format!(
        "e2e: the truth ran in {:.1} s",
        started.elapsed().as_secs_f64()
    ));
    Ok(())
}
