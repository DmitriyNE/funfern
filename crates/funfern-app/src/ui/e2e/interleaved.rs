//! RT-01: random actions through the app's handlers on a device, the
//! protocol model run beside it and the reference on the timeline it took.
//!
//! `interleaved` opens a small scene, the runtime machines' own - the default
//! box on a coarse mesh, a disc of its own medium, the point source on - and
//! fires actions drawn from a seed, the fixture's `interleaved-<seed>` or
//! `FUNFERN_E2E_SEED` beside `interleaved`, through the app's handlers
//! while the run goes on as the app paces it: material edits, a drag of the
//! disc, the mesh edge, Remesh, the speed, the source moved, switched or
//! retuned, Reset, Run and Step, pulses, the Switch, another scene opened,
//! Undo and Redo, and now and then a fault armed on the running generation or
//! the next handoff made to fail. A few frames lie between actions, or none,
//! so that each lands wherever the device and the workers have got to.
//! Adaptation is on in half the seeds.
//!
//! Three things judge the run:
//! - the protocol model (`funfern-protocol`) steps beside the app, told what
//!   the runtime logged it decided, what the actions changed and what the
//!   device's copies told the host, and after every step the two must stand
//!   in one state (`lockstep`); the runtime machines' own claims hold too,
//!   that the host's active topology is the one the device runs while
//!   nothing uploads, at the step it was packed for, and that nothing is
//!   prepared from a scene this one never held;
//! - an f64 reference, on a thread of its own, follows the timeline the
//!   device accepted - each generation from rest or handed the field at the
//!   step its clock names, each event the device took at the step it was
//!   encoded at, in the order encoded, the grid filter on the device's
//!   cadence - and every full snapshot must stand with it;
//! - after the last action the run must come to rest, running the
//!   document's accepted revision or with an error naming it, and the model
//!   with it.
//!
//! A failure writes the actions, the model's steps and the timeline to
//! `target/interleaved-<seed>.log`. Natively and on lavapipe only: in the
//! browser the reference would step inside the page's frames.

use super::adaptive::{Compared, Epoch, Snapshot, past};
use super::*;
use crate::canonical_gpu::{CANONICAL_FAILURE_NON_FINITE, CanonicalGpuStats, EncodedEvent};
use crate::topology_runtime::{PreparedSolverUpdate, TopologyToken};
use crate::topology_viewport::{TopologyHandle, plan_handle_drag};
use bevy::render::storage::ShaderBuffer;
use funfern_protocol as model;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc;

use super::super::lockstep::{Heard, Lockstep, Running, Told, Was};

/// Actions a seed fires at most; the model names 128 requests, and the run
/// stops acting before its requests come near that.
const ACTIONS: usize = 48;
const REQUESTS: usize = 100;
/// The frames before an action, drawn: none as often as one, a burst.
const GAPS: [u64; 12] = [0, 0, 0, 1, 1, 2, 3, 5, 8, 13, 21, 34];
/// How long the run may take to come to rest after its last action.
const REST_DEADLINE: std::time::Duration = std::time::Duration::from_secs(120);
/// The bounds the replay is held to, `adaptive`'s: over the most each part
/// held, and against itself while it holds a hundredth of that. The study's
/// 30 seeds on lavapipe read 9.4e-7 and 2.8e-6 at most, the device
/// suite's twelve on Metal 8.4e-7 and 1.4e-6.
const REPLAY_OVER_PEAK: f64 = 1.0e-5;
const REPLAY_HELD: f64 = 3.0e-5;
/// How far a cleanup's increment may stand from the reference's own, over
/// the flux it changes: the app solves from a single-precision readback, to
/// 1e-6 of the flux's force. The study read 5.6e-6 at most.
const CLEANUP_GAP: f64 = 5.0e-5;

/// One action, as a user takes it through the app's handlers. Ordinals wrap,
/// so that every one is legal in every state.
#[derive(Clone, Copy, Debug)]
enum Action {
    /// Edit the `nth` material the `kind`th of four ways: its density
    /// toggled, the Switchable medium or the Parametric pump preset, or its
    /// laws made linear.
    Material {
        nth: u8,
        kind: u8,
    },
    /// Drag the first curve's first control point by the `nth` offset.
    Nudge(u8),
    /// Set the mesh edge to the `nth` of three.
    Edge(u8),
    Remesh,
    /// Set the simulation speed to the `nth` of three.
    Speed(u8),
    /// Move the point source to the `cell`th of sixteen cells, on or off, at
    /// one of two frequencies.
    Source {
        cell: u8,
        on: bool,
        high: bool,
    },
    Reset,
    Run(bool),
    Step,
    /// Place a pulse in the `cell`th cell.
    Pulse(u8),
    /// Switch's hotkey.
    Switch,
    /// Open the other scene or this one again, its medium switchable or not.
    Open {
        other: bool,
        switchable: bool,
    },
    Undo,
    Redo,
    /// Arm a fault on the running generation, at the `word`th state word.
    Fault(u32),
    /// Make the next handoff's candidate fail its validation on the device.
    FailHandoff(u32),
}

impl Action {
    /// What the model is told of it, if anything: a fault is the device's,
    /// read from its copies.
    fn told(self) -> Option<Told> {
        Some(match self {
            Action::Material { .. } | Action::Nudge(_) | Action::Source { .. } => Told::Edit,
            Action::Edge(_) => Told::Edge,
            Action::Remesh => Told::Remesh,
            Action::Speed(_) => Told::Speed,
            Action::Reset => Told::Reset,
            Action::Run(on) => Told::Run(on),
            Action::Step => Told::Step,
            Action::Pulse(_) => Told::Pulse,
            Action::Switch => Told::Switch,
            Action::Open { .. } => Told::Open,
            Action::Undo => Told::Undo,
            Action::Redo => Told::Redo,
            Action::Fault(_) | Action::FailHandoff(_) => return None,
        })
    }
}

/// The seed's draws: a linear congruential generator, as `rollback`'s.
struct Draw(u64);

impl Draw {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % bound
    }

    fn byte(&mut self) -> u8 {
        self.next(256) as u8
    }

    fn flip(&mut self) -> bool {
        self.next(2) == 1
    }
}

/// The seed's run: whether it adapts, whether its scene opens switchable, and
/// its actions, each with the frames before it.
fn drawn(seed: u64) -> (bool, bool, VecDeque<(u64, Action)>) {
    let mut draw = Draw(seed ^ 0x5e_ed0f_0001);
    let adaptive = draw.flip();
    let switchable = draw.flip();
    // Weighted as the runtime machines weigh the user's actions, the faults
    // rare: a fault pauses the run until Run.
    const WEIGHTS: [(u64, u8); 16] = [
        (3, 0),  // Material
        (2, 1),  // Nudge
        (1, 2),  // Edge
        (1, 3),  // Remesh
        (1, 4),  // Speed
        (2, 5),  // Source
        (2, 6),  // Reset
        (2, 7),  // Run
        (1, 8),  // Step
        (3, 9),  // Pulse
        (2, 10), // Switch
        (1, 11), // Open
        (1, 12), // Undo
        (1, 13), // Redo
        (1, 14), // Fault
        (1, 15), // FailHandoff
    ];
    let total = WEIGHTS.iter().map(|(weight, _)| weight).sum::<u64>();
    let actions = (0..ACTIONS)
        .map(|_| {
            let gap = GAPS[draw.next(GAPS.len() as u64) as usize];
            let mut pick = draw.next(total);
            let kind = WEIGHTS
                .iter()
                .find(|(weight, _)| {
                    let found = pick < *weight;
                    pick = pick.saturating_sub(*weight);
                    found
                })
                .map_or(0, |(_, kind)| *kind);
            let action = match kind {
                0 => Action::Material {
                    nth: draw.byte(),
                    kind: draw.byte(),
                },
                1 => Action::Nudge(draw.byte()),
                2 => Action::Edge(draw.byte()),
                3 => Action::Remesh,
                4 => Action::Speed(draw.byte()),
                5 => Action::Source {
                    cell: draw.byte(),
                    on: draw.next(4) != 0,
                    high: draw.flip(),
                },
                6 => Action::Reset,
                // Run more often than Pause.
                7 => Action::Run(draw.next(3) != 0),
                8 => Action::Step,
                9 => Action::Pulse(draw.byte()),
                10 => Action::Switch,
                11 => Action::Open {
                    other: draw.flip(),
                    switchable: draw.flip(),
                },
                12 => Action::Undo,
                13 => Action::Redo,
                14 => Action::Fault(draw.next(1 << 16) as u32),
                _ => Action::FailHandoff(draw.next(1 << 16) as u32),
            };
            (gap, action)
        })
        .collect();
    (adaptive, switchable, actions)
}

/// The runtime machines' scene: the default box on a coarse mesh, a disc of
/// a material of its own, switchable or left linear, the point source on.
/// `other` moves the disc and coarsens the mesh.
fn scene(other: bool, switchable: bool, adaptive: bool) -> Result<TopologyDocument, String> {
    let mut editor = TopologyEditor::default();
    let subdomain = editor.add_material().map_err(|error| error.to_string())?;
    if switchable {
        let material = editor
            .document
            .model
            .draft
            .material(subdomain)
            .ok_or("the added material")?
            .clone();
        let switch = law_presets()
            .iter()
            .find(|preset| preset.name == "Switchable medium" && preset.row == LawPresetRow::Mass)
            .ok_or("the switchable medium preset")?;
        editor
            .update_material(
                apply_law_preset(switch, &material).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
    }
    let centre = if other {
        Point2::new(0.35, -0.25)
    } else {
        Point2::new(-0.4, 0.1)
    };
    editor
        .create_closed_curve(
            PeriodicCubicSpline::rounded(centre, 0.25),
            ClosedCurvePurpose::Subdomain {
                material: subdomain,
            },
        )
        .map_err(|error| error.to_string())?;
    let mut source = editor.document.model.source;
    source.enabled = true;
    source.position = Point2::new(0.3, 0.4);
    editor
        .set_point_source(source)
        .map_err(|error| error.to_string())?;
    while editor.acceptance == crate::topology_editor::TopologyAcceptance::Pending {
        editor.validate_frame(4096);
    }
    let mut document = editor.document.clone();
    document.presentation.mesh_edge = if other { 0.3 } else { 0.25 };
    document.presentation.adaptation.enabled = adaptive;
    Ok(document)
}

/// The middle of the `cell`th of sixteen cells of `domain`.
fn cell_point(domain: DomainRect, cell: u8) -> Point2 {
    let (column, row) = (f64::from(cell % 4), f64::from(cell / 4 % 4));
    Point2::new(
        domain.min_x + (column + 0.5) / 4.0 * domain.width(),
        domain.min_y + (row + 0.5) / 4.0 * domain.height(),
    )
}

/// What an event the app sent was, as the reference takes it.
#[derive(Clone)]
enum What {
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
    /// The sources patched to the candidate's, against the forcing the
    /// running topology was authored with: its weights alone, or its drives.
    Sources {
        target: Arc<PreparedTopology>,
        authored: Arc<CanonicalForcing>,
        weights: bool,
    },
    /// The stationary flux the last handoff left taken away: the increment
    /// the app solved for, as the device took it.
    Cleanup { increment: Arc<Vec<Point2>> },
}

impl What {
    fn name(&self) -> &'static str {
        match self {
            What::Pulse { .. } => "pulse",
            What::Switch { .. } => "Switch",
            What::Sources { weights: true, .. } => "source weights",
            What::Sources { weights: false, .. } => "source drives",
            What::Cleanup { .. } => "cleanup",
        }
    }
}

/// An event the app sent: what it was, and, once the device has settled it,
/// whether the device took it.
struct Sent {
    what: What,
    taken: Option<bool>,
}

/// How a generation took the field: from rest, or handed it by the one
/// before, through the maps its candidate was prepared with.
enum Begun {
    Install {
        active: Arc<PreparedTopology>,
        time_step: f64,
    },
    Handoff {
        target: Arc<PreparedTopology>,
        carried: Carried,
        time_step: f64,
        runtime_serials: [u32; 4],
        /// The forcing the running topology was authored with, which the
        /// runtime transfer reads each drive's change against.
        authored: Arc<CanonicalForcing>,
    },
}

/// A generation the device ran, as the reference follows it.
struct Run {
    generation: u64,
    begun: Option<Begun>,
    stats: Arc<CanonicalGpuStats>,
    /// Where its clock starts, from its first control copy.
    epoch: Option<Epoch>,
    /// Its start told the reference.
    started: bool,
    /// What its render world encoded, all of it, oldest first, and how much
    /// has gone to the reference.
    log: Vec<EncodedEvent>,
    forwarded: usize,
    /// The full snapshots read of it and not yet sent, and the last step one
    /// was read at.
    snapshots: VecDeque<Snapshot>,
    read: Option<u64>,
    /// The steps live events landed at, whose snapshots could be of either
    /// side of them.
    landed: BTreeSet<u64>,
}

impl Run {
    fn new(generation: u64, begun: Option<Begun>, stats: Arc<CanonicalGpuStats>) -> Self {
        Run {
            generation,
            begun,
            stats,
            epoch: None,
            started: false,
            log: Vec::new(),
            forwarded: 0,
            snapshots: VecDeque::new(),
            read: None,
            landed: BTreeSet::new(),
        }
    }

    /// Takes in what its render world's log holds now: the log keeps its
    /// last entries only, so the new ones are what follows the longest run
    /// of the log's head the copy already ends with.
    fn merge(&mut self) {
        let fresh = self.stats.encoded_events();
        let same = |a: &EncodedEvent, b: &EncodedEvent| {
            (a.serial, a.kind, a.step) == (b.serial, b.kind, b.step)
        };
        let overlap = (0..=fresh.len().min(self.log.len()))
            .rev()
            .find(|&count| {
                self.log[self.log.len() - count..]
                    .iter()
                    .zip(&fresh[..count])
                    .all(|(kept, new)| same(kept, new))
            })
            .unwrap_or(0);
        self.log.extend_from_slice(&fresh[overlap..]);
    }
}

/// What the host held before a frame, which the frame's changes are read
/// against.
struct Before {
    generation: u64,
    active: Option<TopologyToken>,
    uploading: Option<(TopologyToken, u64)>,
    reset: bool,
    drop: bool,
    serial: u32,
    /// Whether the device held a live event still to process.
    pending: bool,
    pulse: Option<(Point2, RegionId)>,
    switch: Option<MaterialId>,
    packing: Option<TopologyToken>,
    /// The stationary flux's increment found and not yet sent.
    cleanup: Option<Vec<Point2>>,
}

impl Before {
    fn read(state: &Playground, request: &CanonicalGpuRequest) -> Self {
        let coordinator = &state.coordinator;
        Before {
            generation: request.generation(),
            active: state.runtime.active().map(|active| active.bundle.token),
            uploading: coordinator
                .uploading
                .as_ref()
                .map(|upload| (upload.token, upload.generation)),
            reset: coordinator.reset_requested,
            drop: coordinator.drop_requested,
            serial: coordinator.canonical_event_serial,
            pending: request.live_event_pending(),
            pulse: coordinator.pending_pulses.front().copied(),
            switch: coordinator.pending_switches.front().copied(),
            packing: coordinator
                .gpu_upload_preparation
                .as_ref()
                .map(|pack| pack.token),
            cleanup: state
                .flux_cleanup
                .as_ref()
                .and_then(|cleanup| match &cleanup.stage {
                    super::super::flux_cleanup::FluxCleanupStage::Found(increment) => {
                        Some(increment.clone())
                    }
                    _ => None,
                }),
        }
    }
}

#[derive(Resource)]
pub(super) struct InterleavedDriver {
    seed: u64,
    adaptive: bool,
    switchable: bool,
    /// Whether the scene runs the grid filter, as a new document does.
    filtered: bool,
    stage: Stage,
    /// The actions still to fire, each with the frames before it, and the
    /// frames left before the next.
    actions: VecDeque<(u64, Action)>,
    wait: u64,
    /// What happened, frame by frame, for a failure's record.
    trace: Vec<String>,
    frame: u64,
    lockstep: Option<Lockstep>,
    heard: Option<Heard>,
    running: Running,
    /// What the host held before this frame, and what the model is told
    /// the frame changed.
    before: Option<Before>,
    frame_was: Option<Was>,
    /// The accepted scenes the editor has held since the scene was last
    /// replaced: all a topology may be prepared from.
    scenes: Vec<TopologyScene>,
    /// The revision whose draft was last validated.
    accepted_revision: u64,
    /// The pack each candidate went to the device with: its token, the
    /// runtime serials it was packed against.
    packed: Option<(TopologyToken, [u32; 4])>,
    /// The candidate an upload began for and how it starts its generation,
    /// with the generation it publishes as.
    begun: Option<(u64, Begun)>,
    /// The events the app sent, by generation and serial: a generation
    /// installed afresh counts its serials again.
    sent: BTreeMap<(u64, u32), Sent>,
    /// The generations run, the running one last.
    runs: Vec<Run>,
    /// The generation a fault was armed on that has not yet landed: a
    /// generation that follows starts with a clean status.
    armed: Option<u64>,
    following: Option<Following>,
    /// What the reference found, once it reached the endpoint.
    followed: Option<Box<Followed>>,
    counts: Counts,
    started: Instant,
    rested_from: Option<Instant>,
    told: Instant,
    deadline: Instant,
}

/// What the run went through, for its summary.
#[derive(Default, Debug)]
struct Counts {
    fired: usize,
    model_steps: usize,
    installs: usize,
    handoffs: usize,
    refused_handoffs: usize,
    faults: usize,
    taken: BTreeMap<&'static str, usize>,
    refused: BTreeMap<&'static str, usize>,
}

enum Stage {
    Open,
    /// Firing actions.
    Acting,
    /// The last action fired, Run pressed: waiting for the run to come to
    /// rest.
    Resting,
    /// At rest after `after` steps: the first full snapshot past it is the
    /// endpoint.
    Ending {
        after: u64,
    },
    /// The endpoint went to the reference.
    Awaiting,
    Done,
}

impl Stage {
    fn name(&self) -> &'static str {
        match self {
            Stage::Open => "opening the scene",
            Stage::Acting => "acting",
            Stage::Resting => "waiting for the run to come to rest",
            Stage::Ending { .. } => "waiting for the endpoint's snapshot",
            Stage::Awaiting => "waiting for the reference to reach the endpoint",
            Stage::Done => "done",
        }
    }
}

/// Adds the `interleaved` fixture's driver for `seed`, or the one
/// FUNFERN_E2E_SEED names.
pub(super) fn add(app: &mut App, seed: Option<u64>, seconds: Option<u64>) {
    #[cfg(not(target_arch = "wasm32"))]
    let seed = seed.or_else(|| {
        std::env::var("FUNFERN_E2E_SEED")
            .ok()
            .and_then(|seed| seed.parse::<u64>().ok())
    });
    let seed = seed.unwrap_or(1);
    let (adaptive, switchable, actions) = drawn(seed);
    mark("running", "");
    report(&format!("e2e build {}", crate::BUILD_ID));
    report(&format!(
        "e2e interleaved: seed {seed}, adaptation {}, the medium {}, {} actions",
        if adaptive { "on" } else { "off" },
        if switchable { "switchable" } else { "linear" },
        actions.len()
    ));
    app.insert_resource(InterleavedDriver {
        seed,
        adaptive,
        switchable,
        filtered: true,
        stage: Stage::Open,
        wait: actions.front().map_or(0, |(gap, _)| *gap),
        actions,
        trace: Vec::new(),
        frame: 0,
        lockstep: None,
        heard: None,
        running: None,
        before: None,
        frame_was: None,
        scenes: Vec::new(),
        accepted_revision: 0,
        packed: None,
        begun: None,
        sent: BTreeMap::new(),
        runs: Vec::new(),
        armed: None,
        following: None,
        followed: None,
        counts: Counts::default(),
        started: Instant::now(),
        rested_from: None,
        told: Instant::now(),
        deadline: Instant::now() + std::time::Duration::from_secs(seconds.unwrap_or(600)),
    })
    .add_systems(
        EguiPrimaryContextPass,
        (
            drive_interleaved.before(frame),
            conform_interleaved.after(frame),
        ),
    );
}

/// Before the app's frame: the device's news since the last, then the
/// actions due.
fn drive_interleaved(
    mut driver: ResMut<InterleavedDriver>,
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
    let outcome = before_frame(
        &mut driver,
        &mut state,
        &mut request,
        &display,
        &mut assets,
        &mut commands,
    );
    finish(&mut driver, outcome, &mut exit);
}

/// After the app's frame: the model's frame, the claims, and what the
/// frame sent the device.
fn conform_interleaved(
    mut driver: ResMut<InterleavedDriver>,
    mut state: ResMut<Playground>,
    request: Res<CanonicalGpuRequest>,
    display: Res<CanonicalGpuDisplay>,
    mut exit: MessageWriter<AppExit>,
) {
    if matches!(driver.stage, Stage::Done | Stage::Open) || driver.before.is_none() {
        return;
    }
    let outcome = after_frame(&mut driver, &mut state, &request, &display);
    finish(&mut driver, outcome, &mut exit);
}

/// Ends the run where `outcome` says it is over.
fn finish(
    driver: &mut InterleavedDriver,
    outcome: Option<Result<String, String>>,
    exit: &mut MessageWriter<AppExit>,
) {
    let Some(outcome) = outcome else {
        return;
    };
    driver.stage = Stage::Done;
    match outcome {
        Ok(summary) => {
            report(&format!("e2e interleaved: {summary}"));
            mark("pass", &summary);
            exit.write(AppExit::Success);
        }
        Err(failure) => {
            let record = driver.record(&failure);
            report(&format!("e2e interleaved failed: {failure}{record}"));
            mark("fail", &failure);
            exit.write(AppExit::error());
        }
    }
}

impl InterleavedDriver {
    /// Writes what the run did, natively, and says where.
    fn record(&self, failure: &str) -> String {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = std::path::PathBuf::from(
                std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into()),
            )
            .join(format!("interleaved-{}.log", self.seed));
            let mut text = format!(
                "seed {}, adaptation {}, medium {}\nfailure: {failure}\n\n",
                self.seed,
                self.adaptive,
                if self.switchable {
                    "switchable"
                } else {
                    "linear"
                }
            );
            for line in &self.trace {
                text.push_str(line);
                text.push('\n');
            }
            text.push_str("\nsent:\n");
            for ((generation, serial), sent) in &self.sent {
                text.push_str(&format!(
                    "  {serial} on {generation}: {} {:?}\n",
                    sent.what.name(),
                    sent.taken
                ));
            }
            text.push_str("\ngenerations:\n");
            for run in &self.runs {
                text.push_str(&format!(
                    "  {} {} from {:?}, {} encoded, {} forwarded\n",
                    run.generation,
                    match run.begun {
                        Some(Begun::Install { .. }) => "installed",
                        Some(Begun::Handoff { .. }) => "handed off",
                        None => "begun unknown",
                    },
                    run.epoch.map(|epoch| epoch.step),
                    run.log.len(),
                    run.forwarded
                ));
            }
            if let Some(followed) = &self.followed {
                text.push_str("\ncomparisons, each part's distance over its norm:\n");
                for (at, compared) in followed.comparisons.iter().enumerate() {
                    let [q, b, z] = compared.parts.map(|[difference, norm]| {
                        format!("{:.2e} of {norm:.2e}", difference / norm)
                    });
                    text.push_str(&format!("  {}: Q {q}, b {b}, z {z}\n", followed.place(at)));
                }
            }
            match std::fs::write(&path, text) {
                Ok(()) => format!("; the run is recorded in {}", path.display()),
                Err(error) => format!("; the record was not written: {error}"),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = failure;
            String::new()
        }
    }

    fn trace(&mut self, line: String) {
        self.trace.push(format!("{:6} {line}", self.frame));
    }

    fn lockstep(&mut self) -> Result<&mut Lockstep, String> {
        self.lockstep
            .as_mut()
            .ok_or_else(|| "no model runs beside the app".to_owned())
    }
}

fn before_frame(
    driver: &mut InterleavedDriver,
    state: &mut Playground,
    request: &mut CanonicalGpuRequest,
    display: &CanonicalGpuDisplay,
    assets: &mut Assets<ShaderBuffer>,
    commands: &mut Commands,
) -> Option<Result<String, String>> {
    driver.frame += 1;
    if Instant::now() >= driver.deadline {
        return Some(Err(format!(
            "did not finish in time, {}",
            driver.stage.name()
        )));
    }
    if driver.told.elapsed().as_secs() >= 10 {
        driver.told = Instant::now();
        report(&format!(
            "e2e interleaved: {} at {:.0} s, frame {}, {} actions left, generation {}, step {}",
            driver.stage.name(),
            driver.started.elapsed().as_secs_f64(),
            driver.frame,
            driver.actions.len(),
            request.generation(),
            request.stats().completed_steps()
        ));
    }
    // The reference answers once, failed or at the endpoint.
    if let Some(following) = &mut driver.following
        && let Some(heard) = following.heard()
    {
        match heard {
            Answer::Failed(failure) => return Some(Err(failure)),
            Answer::Ended(followed) => {
                driver.stage = Stage::Done;
                let verdict = judge(driver, &followed);
                driver.followed = Some(followed);
                return Some(verdict);
            }
        }
    }
    if matches!(driver.stage, Stage::Open) {
        if !state.startup_done {
            return None;
        }
        if let Err(failure) = open(driver, state, request, display) {
            return Some(Err(failure));
        }
    } else {
        if let Err(failure) = hear_device(driver, state, request, display) {
            return Some(Err(failure));
        }
        // A fault armed while the run was paused lands once it runs, after
        // the last Run as like as not, and the failure pauses the run again:
        // Run is pressed again, as the user would, and the run rests anew.
        if matches!(driver.stage, Stage::Resting | Stage::Ending { .. }) && !state.wave_running {
            if let Err(failure) = fire(
                driver,
                state,
                request,
                display,
                assets,
                commands,
                Action::Run(true),
            ) {
                return Some(Err(failure));
            }
            driver.stage = Stage::Resting;
        }
        if matches!(driver.stage, Stage::Acting)
            && let Err(failure) = act(driver, state, request, display, assets, commands)
        {
            return Some(Err(failure));
        }
    }
    driver.before = Some(Before::read(state, request));
    driver.frame_was = Some(Was::of(state));
    None
}

/// Opens the seed's scene in the app's, and starts the model beside it, at
/// the device's generation as it stands: the scene the app started with is
/// dropped by the frame, as a replacement's is.
fn open(
    driver: &mut InterleavedDriver,
    state: &mut Playground,
    request: &CanonicalGpuRequest,
    display: &CanonicalGpuDisplay,
) -> Result<(), String> {
    *FRESH_CLOCK.lock().unwrap() = None;
    state.e2e_steps = E2eSteps::default();
    state.wave_running = true;
    let document = scene(false, driver.switchable, driver.adaptive)?;
    state
        .set_document(document, false, true)
        .map_err(|error| format!("the scene did not open: {error}"))?;
    state.protocol_log = ProtocolLog::kept();
    let mut lockstep = Lockstep::opened(state);
    lockstep.model.generation = request.generation() as u8;
    lockstep.model.display = display.generation as u8;
    lockstep.model.running = state.wave_running;
    driver.lockstep = Some(lockstep);
    driver.heard = Some(Heard::now(request, display));
    driver.scenes = vec![state.editor.document.model.accepted.clone()];
    driver.filtered = state.editor.document.presentation.grid_scale_filter;
    driver.following = Some(Following::start());
    driver.stage = Stage::Acting;
    driver.trace("opened the scene".into());
    Ok(())
}

/// What the device's copies told the host since the last frame, as the
/// model's steps, and what the fixture follows of it: a handoff admitted
/// starts the generation the upload carried, a live event settled is taken
/// or refused.
fn hear_device(
    driver: &mut InterleavedDriver,
    state: &mut Playground,
    request: &CanonicalGpuRequest,
    display: &CanonicalGpuDisplay,
) -> Result<(), String> {
    let Some(heard) = driver.heard.take() else {
        return Err("nothing was heard of the device".into());
    };
    let log = state.protocol_log.take();
    if !log.is_empty() {
        return Err(format!("the runtime logged {log:?} outside a frame"));
    }
    let steps = heard.since(request, display);
    for step in &steps {
        match step {
            model::Step::Handoff { admit: true } => {
                let generation = request.generation();
                let Some((token, awaited)) = state
                    .coordinator
                    .uploading
                    .as_ref()
                    .map(|upload| (upload.token, upload.generation))
                else {
                    return Err("the device published a handoff no upload waits for".into());
                };
                if awaited != generation {
                    return Err(format!(
                        "a handoff published as generation {generation}, where the host awaits {awaited}"
                    ));
                }
                driver.running = Some((generation, token));
                let begun = match driver.begun.take() {
                    Some((published, begun)) if published == generation => Some(begun),
                    _ => None,
                };
                if begun.is_none() {
                    return Err(format!(
                        "generation {generation} was handed off with no candidate seen to begin it"
                    ));
                }
                driver.counts.handoffs += 1;
                driver
                    .runs
                    .push(Run::new(generation, begun, Arc::clone(request.stats())));
            }
            model::Step::Handoff { admit: false } => {
                driver.counts.refused_handoffs += 1;
                driver.begun = None;
            }
            model::Step::Settle { take } => {
                let generation = request.generation();
                let Some(sent) = driver
                    .sent
                    .iter_mut()
                    .rev()
                    .find(|((on, _), sent)| *on == generation && sent.taken.is_none())
                    .map(|(_, sent)| sent)
                else {
                    return Err("the device settled an event the app never sent".into());
                };
                sent.taken = Some(*take);
                let counts = if *take {
                    &mut driver.counts.taken
                } else {
                    &mut driver.counts.refused
                };
                *counts.entry(sent.what.name()).or_default() += 1;
            }
            model::Step::Fault => {
                driver.counts.faults += 1;
                driver.armed = None;
            }
            _ => {}
        }
    }
    if !steps.is_empty() {
        driver.trace(format!("device {steps:?}"));
        driver.counts.model_steps += steps.len();
        let running = driver.running;
        driver
            .lockstep()?
            .conform(&steps, &[], state, request, display, running)
            .map_err(|failure| format!("after the device's {steps:?}: {failure}"))?;
    }
    Ok(())
}

/// Fires the actions due this frame, and, the last one fired, presses Run
/// and stops adapting, so that the run comes to rest.
fn act(
    driver: &mut InterleavedDriver,
    state: &mut Playground,
    request: &mut CanonicalGpuRequest,
    display: &CanonicalGpuDisplay,
    assets: &mut Assets<ShaderBuffer>,
    commands: &mut Commands,
) -> Result<(), String> {
    if driver.wait > 0 {
        driver.wait -= 1;
        return Ok(());
    }
    while driver.wait == 0 {
        if driver.lockstep()?.requests() >= REQUESTS {
            driver.actions.clear();
        }
        let Some((_, action)) = driver.actions.pop_front() else {
            break;
        };
        fire(driver, state, request, display, assets, commands, action)?;
        driver.wait = driver.actions.front().map_or(0, |(gap, _)| *gap);
    }
    if driver.actions.is_empty() {
        fire(
            driver,
            state,
            request,
            display,
            assets,
            commands,
            Action::Run(true),
        )?;
        // Adaptation chases a field a source keeps driving for as long as
        // it runs; switched off, what it has under way still lands.
        state.editor.document.presentation.adaptation.enabled = false;
        driver.stage = Stage::Resting;
        driver.rested_from = Some(Instant::now());
        driver.trace("resting".into());
    }
    Ok(())
}

/// Takes `action` through the app's handlers, and tells the model.
#[allow(clippy::too_many_arguments)]
fn fire(
    driver: &mut InterleavedDriver,
    state: &mut Playground,
    request: &mut CanonicalGpuRequest,
    display: &CanonicalGpuDisplay,
    assets: &mut Assets<ShaderBuffer>,
    commands: &mut Commands,
    action: Action,
) -> Result<(), String> {
    let was = Was::of(state);
    let domain = state.editor.document.model.draft.geometry.domain;
    match action {
        Action::Material { nth, kind } => {
            let materials = &state.editor.document.model.draft.materials;
            let mut material = materials[nth as usize % materials.len()].clone();
            let preset = |name: &str| {
                law_presets()
                    .iter()
                    .find(|preset| preset.name == name && preset.row == LawPresetRow::Mass)
                    .ok_or_else(|| format!("the {name} preset"))
            };
            let edited = match kind % 4 {
                0 => {
                    let dense = material.mass_density == ScalarField::constant(1.0);
                    material.mass_density = ScalarField::constant(if dense { 1.5 } else { 1.0 });
                    Ok(material)
                }
                1 => apply_law_preset(preset("Switchable medium")?, &material),
                2 => apply_law_preset(preset("Parametric pump")?, &material),
                _ => {
                    material.mass_law = CoefficientLaw::linear();
                    material.stiffness_law = CoefficientLaw::linear();
                    Ok(material)
                }
            };
            if let Ok(edited) = edited {
                let _ = state.editor.update_material(edited);
            }
        }
        Action::Nudge(nth) => {
            let draft = &state.editor.document.model.draft;
            if let Some(curve) = draft.geometry.curves.first() {
                let controls = match &curve.spline {
                    CurveSpline::Closed(spline) => spline.controls(),
                    CurveSpline::Open(spline) => spline.controls(),
                };
                let (dx, dy) =
                    [(0.05, 0.0), (-0.05, 0.03), (0.4, 0.0), (0.0, 0.9)][nth as usize % 4];
                let point = Point2::new(controls[0].x + dx, controls[0].y + dy);
                let handle = TopologyHandle::Control {
                    curve: curve.id,
                    control: 0,
                };
                if let Ok(update) = plan_handle_drag(&draft.geometry, handle, point) {
                    let _ = state.editor.apply_transform_updates(&[update]);
                }
            }
        }
        Action::Edge(nth) => {
            state.editor.document.presentation.mesh_edge = [0.2, 0.25, 0.3][nth as usize % 3];
        }
        Action::Remesh => state.coordinator.remesh_requested = true,
        Action::Speed(nth) => {
            state.editor.document.presentation.simulation_speed =
                [0.25, 1.0, 2.0][nth as usize % 3];
        }
        Action::Source { cell, on, high } => {
            let mut source = state.editor.document.model.source;
            source.position = cell_point(domain, cell);
            source.enabled = on;
            source.signal = TimeSignal::harmonic(0.0, 1.0, if high { 3.5 } else { 2.5 }, 0.0);
            let _ = state.editor.set_point_source(source);
        }
        Action::Reset => state.coordinator.reset_requested = true,
        Action::Run(on) => state.wave_running = on,
        Action::Step => state.wave_step = true,
        Action::Pulse(cell) => state.place_pulse(cell_point(domain, cell)),
        Action::Switch => state.request_material_switch(),
        Action::Open { other, switchable } => {
            let document = scene(other, switchable, driver.adaptive)?;
            state
                .set_document(document, true, true)
                .map_err(|error| format!("the scene did not open: {error}"))?;
        }
        Action::Undo => state.undo(),
        Action::Redo => state.redo(),
        // The hook replaces the device's status record, so a fault armed on
        // a device that has failed, or beside one armed and not yet landed,
        // would wipe the failure it holds: no user's device clears its own
        // failure. One at a time, and none on a failed device.
        Action::Fault(word) => {
            let generation = request.generation();
            if driver.armed != Some(generation)
                && !request.failed()
                && let Some(nodes) = request.running_node_count()
                && request
                    .inject_failure(
                        assets,
                        commands,
                        CANONICAL_FAILURE_NON_FINITE,
                        word % nodes as u32,
                    )
                    .is_ok()
            {
                driver.armed = Some(generation);
            }
        }
        Action::FailHandoff(word) => state.e2e_steps.fail_handoff = Some(word % 64),
    }
    driver.counts.fired += 1;
    driver.trace(format!("{action:?}"));
    let lockstep = driver.lockstep()?;
    let mut steps = action
        .told()
        .and_then(|told| lockstep.told(told, state, &was))
        .into_iter()
        .collect::<Vec<_>>();
    // A handler that finishes the draft's validation itself, as a scene
    // opened may, has it finish outside a frame.
    if was.pending()
        && state.editor.acceptance != crate::topology_editor::TopologyAcceptance::Pending
    {
        steps.extend(lockstep.told(Told::Validate, state, &was));
    }
    if !steps.is_empty() {
        let running = driver.running;
        driver.trace(format!("model {steps:?}"));
        driver.counts.model_steps += steps.len();
        driver
            .lockstep()?
            .conform(&steps, &[], state, request, display, running)
            .map_err(|failure| format!("after {action:?}: {failure}"))?;
    }
    Ok(())
}

fn after_frame(
    driver: &mut InterleavedDriver,
    state: &mut Playground,
    request: &CanonicalGpuRequest,
    display: &CanonicalGpuDisplay,
) -> Option<Result<String, String>> {
    match conform_frame(driver, state, request, display) {
        Ok(()) => {}
        Err(failure) => return Some(Err(failure)),
    }
    if matches!(driver.stage, Stage::Resting) {
        if driver
            .rested_from
            .is_some_and(|from| from.elapsed() >= REST_DEADLINE)
        {
            let why = unsettled(driver, state, request).unwrap_or_default();
            return Some(Err(format!(
                "the run did not come to rest within {} s of its last action: {why}",
                REST_DEADLINE.as_secs()
            )));
        }
        let model_rests = driver
            .lockstep
            .as_ref()
            .is_some_and(|lockstep| lockstep.model.at_rest());
        if model_rests && unsettled(driver, state, request).is_none() {
            let after = request.stats().completed_steps();
            driver.trace(format!("at rest at step {after}"));
            driver.stage = Stage::Ending { after };
        }
    }
    None
}

/// The model's frame, what the frame did to the device and the host's
/// active topology, the claims, and what the frame sent the device.
fn conform_frame(
    driver: &mut InterleavedDriver,
    state: &mut Playground,
    request: &CanonicalGpuRequest,
    display: &CanonicalGpuDisplay,
) -> Result<(), String> {
    let log = state.protocol_log.take();
    let (Some(was), Some(before)) = (driver.frame_was.take(), driver.before.take()) else {
        return Err("a frame with nothing read before it".into());
    };
    let step = driver.lockstep()?.frame(state, request, &was, &log);
    observe(driver, state, request, display, &before, &log)?;
    let running = driver.running;
    driver.counts.model_steps += 1;
    if !log.is_empty() {
        driver.trace(format!("frame {log:?}"));
    }
    driver
        .lockstep()?
        .conform(&[step], &log, state, request, display, running)
        .map_err(|failure| format!("after the frame {step:?} with {log:?}: {failure}"))?;
    check(driver, state, request)?;
    sent(driver, state, request, &before, &log)?;
    follow(driver, request, display)?;
    driver.heard = Some(Heard::now(request, display));
    Ok(())
}

/// Follows what the frame did to the device and to the host's active
/// topology, and holds each change to what may cause it: a new generation
/// to an upload or a Reset, a topology committed with no new generation to
/// a change of its measurements or sources. The runtime machines'.
fn observe(
    driver: &mut InterleavedDriver,
    state: &Playground,
    request: &CanonicalGpuRequest,
    display: &CanonicalGpuDisplay,
    before: &Before,
    log: &[ProtocolNote],
) -> Result<(), String> {
    let generation = request.generation();
    let active = state.runtime.active().map(|active| active.bundle.token);
    // The pack each candidate went to the device with, and the runtime
    // serials it was packed against, as `refresh_runtime` read them: the
    // display changes between frames only.
    if let Some(pack) = &state.coordinator.gpu_upload_preparation
        && before.packing != Some(pack.token)
    {
        driver.packed = Some((pack.token, display.runtime_serials));
    }
    if log.contains(&ProtocolNote::Begin {
        install: false,
        ok: true,
    }) {
        let Some(upload) = &state.coordinator.uploading else {
            return Err("a handoff began with no upload".into());
        };
        let Some(candidate) = state
            .runtime
            .ready()
            .filter(|ready| ready.bundle.token == upload.token)
        else {
            return Err("a handoff began with its candidate not ready".into());
        };
        let Some(maps) = candidate.canonical_transfer.clone() else {
            return Err("a handoff began with no maps".into());
        };
        // Packed in an earlier frame, or in this one, against the same
        // display.
        let runtime_serials = match driver.packed {
            Some((token, serials)) if token == upload.token => serials,
            _ => display.runtime_serials,
        };
        // The running topology stays active until the handoff publishes.
        let Some(authored) = state
            .runtime
            .active()
            .map(|active| Arc::clone(&active.canonical_forcing))
        else {
            return Err("a handoff began with nothing active".into());
        };
        driver.begun = Some((
            upload.generation,
            Begun::Handoff {
                target: Arc::new(candidate.clone()),
                carried: Carried {
                    maps,
                    interpolation: candidate.transfer.clone(),
                },
                time_step: upload.time_step,
                runtime_serials,
                authored,
            },
        ));
    }
    if request.running_node_count().is_none() {
        driver.running = None;
    } else if generation != before.generation {
        let begun = if let Some(upload) = state
            .coordinator
            .uploading
            .as_ref()
            .filter(|upload| upload.generation == generation)
        {
            driver.running = Some((generation, upload.token));
            let Some(candidate) = state
                .runtime
                .ready()
                .filter(|ready| ready.bundle.token == upload.token)
            else {
                return Err("an install with its candidate not ready".into());
            };
            Begun::Install {
                active: Arc::new(candidate.clone()),
                time_step: upload.time_step,
            }
        } else if before.reset && !state.coordinator.reset_requested {
            let Some(active) = state.runtime.active() else {
                return Err("Reset installed a generation with none active".into());
            };
            driver.running = Some((generation, active.bundle.token));
            if !state.coordinator.pending_pulses.is_empty()
                || !state.coordinator.pending_switches.is_empty()
                || request.live_event_pending()
            {
                return Err("Reset left a pulse or a Switch for the run it cleared".into());
            }
            Begun::Install {
                active: Arc::clone(active),
                time_step: state.coordinator.uploaded_time_step,
            }
        } else {
            return Err(format!(
                "the device installed generation {generation} that no upload or Reset accounts for"
            ));
        };
        driver.counts.installs += 1;
        // The field starts from rest: nothing of the generations before is
        // followed any more.
        driver.runs.clear();
        driver.runs.push(Run::new(
            generation,
            Some(begun),
            Arc::clone(request.stats()),
        ));
        driver.trace(format!("installed generation {generation}"));
    } else if let Some(token) = active.filter(|token| Some(*token) != before.active) {
        if before
            .uploading
            .is_some_and(|(uploaded, _)| uploaded == token)
        {
            if driver.running.map(|(_, running)| running) != Some(token) {
                return Err("the host published an upload the device does not run".into());
            }
        } else {
            let update = state
                .runtime
                .active()
                .map(|active| active.solver_update)
                .ok_or("a commit with nothing active")?;
            if !matches!(
                update,
                PreparedSolverUpdate::MeasurementsOnly
                    | PreparedSolverUpdate::SourceWeightsOnly
                    | PreparedSolverUpdate::SourceDrivesOnly
            ) {
                return Err(format!(
                    "a {update:?} candidate was committed with no new generation"
                ));
            }
            if let Some((running, _)) = driver.running {
                driver.running = Some((running, token));
            }
        }
    }
    if before.drop
        && (!state.coordinator.pending_pulses.is_empty()
            || !state.coordinator.pending_switches.is_empty()
            || state.coordinator.source_commit.is_some())
    {
        return Err("work for the replaced scene survived its drop".into());
    }
    let accepted = &state.editor.document.model.accepted;
    if state.coordinator.drop_requested && !before.drop {
        driver.scenes = vec![accepted.clone()];
    } else if !driver.scenes.contains(accepted) {
        driver.scenes.push(accepted.clone());
    }
    if state.editor.acceptance == crate::topology_editor::TopologyAcceptance::Valid {
        driver.accepted_revision = state.editor.revision;
    }
    Ok(())
}

/// The runtime machines' claims: while nothing uploads, the host's active
/// topology is the one the device runs, on its mesh and at its step; and
/// nothing is prepared from a scene this one never held.
fn check(
    driver: &InterleavedDriver,
    state: &Playground,
    request: &CanonicalGpuRequest,
) -> Result<(), String> {
    if state.coordinator.drop_requested {
        return Ok(());
    }
    if let (Some((generation, token)), Some(active)) = (driver.running, state.runtime.active())
        && state.coordinator.uploading.is_none()
        && request.generation() == generation
    {
        if active.bundle.token != token {
            return Err("the host's active topology is not the one the device runs".into());
        }
        if request.running_node_count() != Some(active.canonical_operator.degrees_of_freedom()) {
            return Err("the device runs another mesh than the host's".into());
        }
        if request.running_time_step() != Some(state.coordinator.uploaded_time_step) {
            return Err("the host paces a step the device does not run".into());
        }
    }
    for prepared in state
        .runtime
        .active()
        .map(|active| &active.bundle)
        .into_iter()
        .chain(state.runtime.ready().map(|ready| &ready.bundle))
    {
        if !driver.scenes.contains(&prepared.authored) {
            return Err("a topology was prepared from a scene this one never held".into());
        }
    }
    Ok(())
}

/// The event the frame sent the device, if it sent one: at most one goes a
/// frame, since the device takes one at a time. Its serial is the one the
/// coordinator moved to, and what it is the queue it left or the
/// transaction it began.
fn sent(
    driver: &mut InterleavedDriver,
    state: &Playground,
    request: &CanonicalGpuRequest,
    before: &Before,
    log: &[ProtocolNote],
) -> Result<(), String> {
    // An event waits for the render world from the frame that queued it, and
    // none is queued beside one waiting: one waiting now, where none was or
    // under another serial, is this frame's. Its serial alone would not say:
    // a generation started afresh counts again from nothing, in the frame
    // that may send its first.
    let serial = state.coordinator.canonical_event_serial;
    if !request.live_event_pending() || (before.pending && serial == before.serial) {
        return Ok(());
    }
    let coordinator = &state.coordinator;
    let what = if let Some(commit) = coordinator
        .source_commit
        .as_ref()
        .filter(|commit| commit.serial == serial)
    {
        // The candidate stays ready, and the running topology active, until
        // the patch settles.
        let (Some(target), Some(authored)) = (
            state
                .runtime
                .ready()
                .filter(|ready| ready.bundle.token == commit.token)
                .map(|ready| Arc::new(ready.clone())),
            state
                .runtime
                .active()
                .map(|active| Arc::clone(&active.canonical_forcing)),
        ) else {
            return Err(format!(
                "source patch {serial} with no candidate or forcing"
            ));
        };
        let weights = target.solver_update == PreparedSolverUpdate::SourceWeightsOnly;
        What::Sources {
            target,
            authored,
            weights,
        }
    } else if state.flux_cleanup.as_ref().is_some_and(|cleanup| {
        cleanup.stage == super::super::flux_cleanup::FluxCleanupStage::Sent(serial)
    }) {
        let Some(increment) = &before.cleanup else {
            return Err(format!("cleanup {serial} with no increment found"));
        };
        // The device holds it in single precision.
        What::Cleanup {
            increment: Arc::new(
                increment
                    .iter()
                    .map(|value| Point2::new(f64::from(value.x as f32), f64::from(value.y as f32)))
                    .collect(),
            ),
        }
    } else if log.contains(&ProtocolNote::Switch(true))
        && coordinator.pending_switches.front().copied() != before.switch
    {
        let Some(material) = before.switch else {
            return Err(format!("Switch {serial} with no press waiting"));
        };
        let target = coordinator
            .switch_targets
            .get(&material)
            .copied()
            .ok_or_else(|| format!("Switch {serial} sent with no direction recorded"))?;
        let ramp = state
            .editor
            .document
            .model
            .accepted
            .materials
            .iter()
            .find(|found| found.id == material)
            .map(|found| found.switch_ramp)
            .ok_or("the switched material is gone")?;
        What::Switch {
            material,
            target,
            ramp,
        }
    } else if log.contains(&ProtocolNote::Pulse(true)) {
        let Some((position, region)) = before.pulse else {
            return Err(format!("pulse {serial} with none waiting"));
        };
        What::Pulse {
            position,
            region,
            amplitude: f64::from(state.pulse_amplitude),
            width: f64::from(state.pulse_width),
        }
    } else {
        return Err(format!(
            "event {serial} went for nothing the fixture follows"
        ));
    };
    driver.trace(format!("sent {serial}: {}", what.name()));
    driver
        .sent
        .insert((request.generation(), serial), Sent { what, taken: None });
    Ok(())
}

/// What the reference is told, in the order it takes it: a generation's
/// start, then its events and its snapshots in the order of the steps they
/// stand at, the encoder's order within a step.
enum Note {
    Install {
        generation: u64,
        active: Arc<PreparedTopology>,
        time_step: f64,
        epoch: Epoch,
        filtered: bool,
    },
    Handoff {
        generation: u64,
        epoch: Epoch,
        target: Arc<PreparedTopology>,
        carried: Carried,
        time_step: f64,
        runtime_serials: [u32; 4],
        authored: Arc<CanonicalForcing>,
    },
    Event {
        generation: u64,
        step: u64,
        what: What,
    },
    /// A full snapshot, compared unless a live event landed at its step,
    /// whose lanes it could be of either side of.
    Snapshot {
        snapshot: Snapshot,
        compare: bool,
    },
    End(Snapshot),
}

/// Reads the device's generations: where each one's clock starts, what its
/// render world encoded, its full snapshots; and tells the reference each in
/// turn, once nothing at or before it is still to be learned.
fn follow(
    driver: &mut InterleavedDriver,
    request: &CanonicalGpuRequest,
    display: &CanonicalGpuDisplay,
) -> Result<(), String> {
    let generation = request.generation();
    if let Some(run) = driver.runs.last_mut()
        && run.generation == generation
        && display.generation == generation
    {
        if run.epoch.is_none()
            && let Some(clock) = display.clock
        {
            let time_step = match &run.begun {
                Some(Begun::Install { time_step, .. } | Begun::Handoff { time_step, .. }) => {
                    *time_step
                }
                None => return Err(format!("generation {generation} began unseen")),
            };
            run.epoch = Some(Epoch {
                step: clock
                    .accepted_steps
                    .saturating_sub(u64::from(clock.step_in_epoch)),
                origin: clock.epoch_origin_seconds,
                time_step,
            });
        }
        if let Some(epoch) = run.epoch
            && display.full_readback_at == display.readbacks
        {
            let step = display.full_snapshot_completed_steps();
            if run.read.is_none_or(|read| step > read) {
                run.read = Some(step);
                run.snapshots.push_back(Snapshot::of(display, epoch));
            }
        }
    }
    for run in &mut driver.runs {
        run.merge();
    }
    forward(driver)
}

/// Tells the reference what it can take now.
fn forward(driver: &mut InterleavedDriver) -> Result<(), String> {
    let mut notes = Vec::new();
    let mut ending = None;
    let mut flushed = true;
    for index in 0..driver.runs.len() {
        // A generation handed the field waits for the one before to be told
        // in full.
        let run = &mut driver.runs[index];
        if !run.started {
            if !flushed {
                break;
            }
            let Some(epoch) = run.epoch else {
                break;
            };
            let Some(begun) = run.begun.take() else {
                return Err(format!("generation {} began unseen", run.generation));
            };
            notes.push(match begun {
                Begun::Install { active, time_step } => Note::Install {
                    generation: run.generation,
                    active,
                    time_step,
                    epoch,
                    filtered: driver.filtered,
                },
                Begun::Handoff {
                    target,
                    carried,
                    time_step,
                    runtime_serials,
                    authored,
                } => Note::Handoff {
                    generation: run.generation,
                    epoch,
                    target,
                    carried,
                    time_step,
                    runtime_serials,
                    authored,
                },
            });
            run.started = true;
        }
        while run.forwarded < run.log.len() {
            let entry = run.log[run.forwarded];
            let Some(sent) = driver.sent.get(&(run.generation, entry.serial)) else {
                return Err(format!(
                    "generation {} encoded event {} at step {}, which the app never sent it",
                    run.generation, entry.serial, entry.step
                ));
            };
            let Some(taken) = sent.taken else {
                break;
            };
            // An event the latch refused is encoded again where the failure
            // is cleared before its answer is read: the last encoding is the
            // one that ran.
            let again = run.log[run.forwarded + 1..]
                .iter()
                .any(|later| later.serial == entry.serial);
            if taken && !again {
                run.landed.insert(entry.step);
                notes.push(Note::Event {
                    generation: run.generation,
                    step: entry.step,
                    what: sent.what.clone(),
                });
            }
            run.forwarded += 1;
        }
        // A snapshot goes once everything encoded at or before its step has.
        let frontier = run
            .log
            .get(run.forwarded)
            .map_or(u64::MAX, |entry| entry.step);
        while run
            .snapshots
            .front()
            .is_some_and(|snapshot| snapshot.step < frontier)
        {
            let snapshot = run.snapshots.pop_front().expect("a snapshot due");
            if let Stage::Ending { after } = driver.stage
                && snapshot.step > after
                && ending.is_none()
            {
                ending = Some(notes.len());
                notes.push(Note::End(snapshot));
                break;
            }
            let compare = !run.landed.contains(&snapshot.step);
            notes.push(Note::Snapshot { snapshot, compare });
        }
        flushed = run.forwarded == run.log.len();
    }
    if ending.is_some() {
        driver.stage = Stage::Awaiting;
    }
    for note in &notes {
        let told = match note {
            Note::Install {
                generation, epoch, ..
            } => format!("generation {generation} from rest at step {}", epoch.step),
            Note::Handoff {
                generation, epoch, ..
            } => format!(
                "generation {generation} handed the field at step {}",
                epoch.step
            ),
            Note::Event {
                generation,
                step,
                what,
            } => format!(
                "the {} on generation {generation} at step {step}",
                what.name()
            ),
            Note::Snapshot { .. } => continue,
            Note::End(snapshot) => format!(
                "the endpoint, step {} of generation {}",
                snapshot.step, snapshot.generation
            ),
        };
        driver.trace(format!("reference: {told}"));
    }
    let Some(following) = &mut driver.following else {
        return Err("no reference follows the run".into());
    };
    for note in notes {
        following.tell(note);
    }
    Ok(())
}

/// Why the run is not at rest, if it is not: running the accepted revision
/// at the speed's step on a ready device, or with an error naming that
/// revision, and nothing waiting. The runtime machines'.
fn unsettled(
    driver: &InterleavedDriver,
    state: &Playground,
    request: &CanonicalGpuRequest,
) -> Option<String> {
    let waits = [
        ("a pause", !state.wave_running),
        ("a failure", request.failed()),
        ("a Reset", state.coordinator.reset_requested),
        ("a drop", state.coordinator.drop_requested),
        ("an upload", state.coordinator.uploading.is_some()),
        ("a pack", state.coordinator.gpu_upload_preparation.is_some()),
        (
            "a handoff's stationary flux",
            state.flux_cleanup.as_ref().is_some_and(|cleanup| {
                !matches!(
                    cleanup.stage,
                    super::super::flux_cleanup::FluxCleanupStage::Sent(_)
                        | super::super::flux_cleanup::FluxCleanupStage::Landed(_)
                )
            }),
        ),
        ("a source commit", state.coordinator.source_commit.is_some()),
        ("a preparation", state.preparation_in_progress()),
        ("a candidate", state.runtime.ready().is_some()),
        ("a pulse", !state.coordinator.pending_pulses.is_empty()),
        ("a Switch", !state.coordinator.pending_switches.is_empty()),
        ("a live event", request.live_event_pending()),
        (
            "a handoff",
            request.handoff_outcome() == CanonicalGpuHandoffOutcome::Pending,
        ),
        (
            "validation",
            state.editor.acceptance == crate::topology_editor::TopologyAcceptance::Pending,
        ),
        ("an adaptation", state.adaptation_in_flight()),
    ];
    if let Some((what, _)) = waits.iter().find(|(_, waiting)| *waiting) {
        return Some(format!("{what} still waits"));
    }
    if state.request_is_due() {
        return Some("a request is due".into());
    }
    let held = state.editor.acceptance != crate::topology_editor::TopologyAcceptance::Valid;
    let refused = if held {
        state
            .coordinator
            .requested_inputs
            .as_ref()
            .is_some_and(|inputs| inputs.read(&state.editor.document))
            && state.runtime.last_error().is_some_and(|error| {
                Some(error.token.document_revision) == state.coordinator.requested_revision
            })
    } else {
        state
            .runtime
            .last_error()
            .is_some_and(|error| error.token.document_revision == driver.accepted_revision)
    };
    if refused {
        return None;
    }
    let Some(active) = state.runtime.active() else {
        return Some("nothing runs, and no error says why".into());
    };
    if !held && active.bundle.token.document_revision != driver.accepted_revision {
        return Some(format!(
            "revision {} runs where {} was accepted, and no error says why",
            active.bundle.token.document_revision, driver.accepted_revision
        ));
    }
    if driver.running.map(|(_, token)| token) != Some(active.bundle.token) {
        return Some("the device runs another topology than the host's".into());
    }
    if !request.ready() {
        return Some("the device is not ready".into());
    }
    None
}

/// What the reference answers.
enum Answer {
    Failed(String),
    Ended(Box<Followed>),
}

/// What the reference found over the run.
#[derive(Clone, Debug, Default)]
struct Followed {
    comparisons: Vec<Compared>,
    /// The generation and step of each comparison.
    at: Vec<(u64, u64)>,
    /// Snapshots it had stepped past when they came, and those at a live
    /// event's step.
    skipped: usize,
    at_events: usize,
    drift: f64,
    /// The farthest a cleanup's increment stood from the reference's own,
    /// over the flux it changed.
    cleanup_gap: f64,
    generations: usize,
    events: usize,
    filters: usize,
    endpoint: [f64; 3],
    end_time: f64,
}

/// The reference's thread natively; in the browser it would step inside the
/// page's frames, and the fixture does not run there.
struct Following {
    #[cfg(not(target_arch = "wasm32"))]
    notes: mpsc::Sender<Note>,
    #[cfg(not(target_arch = "wasm32"))]
    heard: std::sync::Mutex<mpsc::Receiver<Answer>>,
    #[cfg(target_arch = "wasm32")]
    follower: Follower,
    #[cfg(target_arch = "wasm32")]
    heard: VecDeque<Answer>,
}

impl Following {
    fn start() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (notes, inbox) = mpsc::channel::<Note>();
            let (outbox, heard) = mpsc::channel();
            std::thread::Builder::new()
                .name("e2e-interleaved".into())
                .spawn(move || {
                    let mut follower = Follower::default();
                    for note in inbox {
                        if let Some(heard) = follower.hear(note)
                            && outbox.send(heard).is_err()
                        {
                            return;
                        }
                    }
                })
                .expect("the follower's thread");
            Following {
                notes,
                heard: std::sync::Mutex::new(heard),
            }
        }
        #[cfg(target_arch = "wasm32")]
        Following {
            follower: Follower::default(),
            heard: VecDeque::new(),
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

    fn heard(&mut self) -> Option<Answer> {
        #[cfg(not(target_arch = "wasm32"))]
        return self.heard.get_mut().ok()?.try_recv().ok();
        #[cfg(target_arch = "wasm32")]
        return self.heard.pop_front();
    }
}

/// The reference on the generation it stands on, and what it found; stopped
/// for good at its first failure.
#[derive(Default)]
struct Follower {
    /// The reference, the step its first generation started at, and whether
    /// the grid filter runs.
    reference: Option<(Reference, u64, bool)>,
    followed: Followed,
    failed: bool,
}

impl Follower {
    fn hear(&mut self, note: Note) -> Option<Answer> {
        if self.failed {
            return None;
        }
        match self.answer(note) {
            Ok(heard) => heard,
            Err(failure) => {
                self.failed = true;
                Some(Answer::Failed(format!("the reference: {failure}")))
            }
        }
    }

    fn answer(&mut self, note: Note) -> Result<Option<Answer>, String> {
        if let Note::Install {
            generation,
            active,
            time_step,
            epoch,
            filtered,
        } = note
        {
            let state = State::zero(&active, time_step)?.starting_at(epoch.origin)?;
            self.reference = Some((
                Reference::starting(active, state, generation),
                epoch.step,
                filtered,
            ));
            self.followed.generations += 1;
            return Ok(None);
        }
        let Some((reference, base, filtered)) = &mut self.reference else {
            return Err("a note before any generation started".into());
        };
        let (base, filtered) = (*base, *filtered);
        let followed = &mut self.followed;
        match note {
            Note::Install { .. } => unreachable!("taken above"),
            Note::Handoff {
                generation,
                epoch,
                target,
                carried,
                time_step,
                runtime_serials,
                authored,
            } => {
                step_to(reference, base, epoch.step, filtered, &mut followed.filters)?;
                let reached = reference.state.time();
                let drift = (reached - epoch.origin).abs();
                if !within(drift, CLOCK_TOLERANCE) {
                    return Err(format!(
                        "generation {generation} starts its clock at {:.6} s, {drift:.1e} from \
                         the {reached:.6} s the run reached at step {}",
                        epoch.origin, epoch.step
                    ));
                }
                let (handed, _, target) = hand_off(
                    reference,
                    &target,
                    &carried,
                    time_step,
                    runtime_serials,
                    Some(&authored),
                )
                .map_err(|error| format!("the handoff at step {}: {error}", epoch.step))?;
                reference.state = handed;
                reference.active = target;
                reference.generation = generation;
                followed.generations += 1;
            }
            Note::Event {
                generation,
                step,
                what,
            } => {
                on(reference, generation, "an event")?;
                step_to(reference, base, step, filtered, &mut followed.filters)?;
                let gap = take(reference, &what)
                    .map_err(|error| format!("the {} at step {step}: {error}", what.name()))?;
                followed.cleanup_gap = followed.cleanup_gap.max(gap);
                followed.events += 1;
            }
            Note::Snapshot { snapshot, compare } => {
                on(reference, snapshot.generation, "a snapshot")?;
                if snapshot.step < base + reference.steps {
                    followed.skipped += 1;
                    return Ok(None);
                }
                step_to(
                    reference,
                    base,
                    snapshot.step,
                    filtered,
                    &mut followed.filters,
                )?;
                if !compare {
                    followed.at_events += 1;
                    return Ok(None);
                }
                let parts = parts(reference, &snapshot)?;
                if let Some(seconds) = snapshot.seconds {
                    followed.drift = followed.drift.max((seconds - reference.state.time()).abs());
                }
                followed.at.push((snapshot.generation, snapshot.step));
                followed.comparisons.push(Compared {
                    time: reference.state.time(),
                    held: true,
                    parts,
                });
            }
            Note::End(snapshot) => {
                on(reference, snapshot.generation, "the endpoint")?;
                step_to(
                    reference,
                    base,
                    snapshot.step,
                    filtered,
                    &mut followed.filters,
                )?;
                let parts = parts(reference, &snapshot)?;
                // A field still at rest holds nothing to be a share of.
                followed.endpoint = parts.map(|[difference, norm]| {
                    if norm > 0.0 {
                        difference / norm
                    } else {
                        difference
                    }
                });
                followed.end_time = reference.state.time();
                followed.at.push((snapshot.generation, snapshot.step));
                followed.comparisons.push(Compared {
                    time: reference.state.time(),
                    held: true,
                    parts,
                });
                return Ok(Some(Answer::Ended(Box::new(followed.clone()))));
            }
        }
        Ok(None)
    }
}

/// Whether the reference stands on `generation`.
fn on(reference: &Reference, generation: u64, what: &str) -> Result<(), String> {
    if generation == reference.generation {
        return Ok(());
    }
    Err(format!(
        "{what} of generation {generation} while the reference stands on {}",
        reference.generation
    ))
}

/// Steps the reference to the device's accepted step `step`, its first
/// generation having started at `base`, filtering on the device's cadence
/// where the filter runs.
fn step_to(
    reference: &mut Reference,
    base: u64,
    step: u64,
    filtered: bool,
    filters: &mut usize,
) -> Result<(), String> {
    let Some(target) = step.checked_sub(base) else {
        return Err(format!("step {step} is before the run's start at {base}"));
    };
    if target < reference.steps {
        return Err(format!(
            "step {step} is behind the reference at {}",
            base + reference.steps
        ));
    }
    while reference.steps < target {
        reference.advance_to(reference.steps + 1)?;
        if filtered && (base + reference.steps).is_multiple_of(GRID_SCALE_FILTER_CADENCE) {
            filter(&mut reference.state, &reference.active)?;
            *filters += 1;
        }
    }
    Ok(())
}

/// Takes an event the device took, where the reference stands, and answers
/// how far a cleanup's increment stood from the reference's own, over the
/// flux it changed; nothing for the others.
fn take(reference: &mut Reference, what: &What) -> Result<f64, String> {
    match what {
        What::Pulse {
            position,
            region,
            amplitude,
            width,
        } => reference
            .take(Fired::Pulse {
                position: *position,
                region: *region,
                amplitude: *amplitude,
                width: *width,
            })
            .map(|()| 0.0),
        What::Switch {
            material,
            target,
            ramp,
        } => reference
            .take(Fired::Switch {
                material: *material,
                target: *target,
                ramp: *ramp,
            })
            .map(|()| 0.0),
        // The candidate runs the operator the generation runs, with its
        // sources. Either patch rewrites each drive as a handoff's runtime
        // transfer does, its carrier and rate running on, the phase stepped
        // as the authored one stepped: a weights patch steps it by nothing.
        What::Sources {
            target, authored, ..
        } => {
            let time = reference.state.time();
            let running = &reference.active.canonical_forcing;
            let forcing = target
                .canonical_forcing
                .replaced_on(running, authored, time)
                .map_err(|error| error.to_string())?;
            let mut patched = (**target).clone();
            patched.canonical_forcing = Arc::new(forcing);
            reference.active = Arc::new(patched);
            Ok(0.0)
        }
        // The increment the app found from a readback, as the device took
        // it; the reference's own solve, from where it stands, says how
        // far that stood from the stationary flux.
        What::Cleanup { increment } => {
            let State::Fixed(state) = &mut reference.state else {
                return Err("the device took a driven generation's stationary flux away".into());
            };
            let operator = &reference.active.canonical_operator;
            let mut job = CanonicalStationaryFluxJob::new(
                operator.clone(),
                state.complementary_flux().to_vec(),
            )
            .map_err(|error| error.to_string())?;
            let found = loop {
                if let Some(found) = job.advance(1 << 16) {
                    break found.map_err(|error| error.to_string())?;
                }
            };
            let [gap, flux] = increment
                .iter()
                .zip(&found.stationary)
                .zip(state.complementary_flux())
                .fold([0.0, 0.0], |[gap, flux], ((sent, stationary), held)| {
                    let apart = *sent + *stationary;
                    [gap + apart.dot(apart), flux + held.dot(*held)]
                })
                .map(f64::sqrt);
            state
                .apply_complementary_increment(operator, increment)
                .map_err(|error| error.to_string())?;
            Ok(if flux > 0.0 { gap / flux } else { gap })
        }
    }
}

/// The grid filter, where the device ran it, at its strength.
fn filter(state: &mut State, active: &PreparedTopology) -> Result<(), String> {
    let forcing = &active.canonical_forcing;
    match (state, &active.canonical_temporal_operator) {
        (State::Fixed(state), _) => state
            .apply_grid_filter(
                &active.canonical_operator,
                forcing,
                CANONICAL_GRID_FILTER_STRENGTH,
            )
            .map(|_| ()),
        (State::Temporal(state), Some(temporal)) => state
            .apply_grid_filter_with_forcing(temporal, forcing, CANONICAL_GRID_FILTER_STRENGTH)
            .map(|_| ()),
        (State::Temporal(_), None) => unreachable!("a temporal state on a fixed generation"),
    }
    .map_err(|error| error.to_string())
}

/// The snapshot against the reference where it stands: Q, b and the wall's
/// pole states where the generation runs fixed, each the distance and the
/// reference's norm.
fn parts(reference: &Reference, snapshot: &Snapshot) -> Result<[[f64; 2]; 3], String> {
    let state = &reference.state;
    if snapshot.primary.len() != state.primary().len()
        || snapshot.complementary.len() != state.complementary().len()
    {
        return Err(format!(
            "the snapshot of step {} holds {} and {} values where generation {} has {} and {}",
            snapshot.step,
            snapshot.primary.len(),
            snapshot.complementary.len(),
            reference.generation,
            state.primary().len(),
            state.complementary().len()
        ));
    }
    let apart = |device: &mut dyn Iterator<Item = f64>, expected: &mut dyn Iterator<Item = f64>| {
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
    let wall = state.outgoing_z();
    Ok([
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
        if wall.is_empty() || snapshot.history.len() != wall.len() {
            [0.0, 0.0]
        } else {
            apart(
                &mut snapshot.history.iter().map(|value| f64::from(*value)),
                &mut wall.iter().copied(),
            )
        },
    ])
}

/// Each part's worst distance and the comparison it was found at.
type Worst = [(f64, usize); 3];

/// The comparisons read two ways, as `adaptive` reads them: each part's
/// worst distance over the most it held at any comparison, and against
/// itself where it held a hundredth of that. A part never held is not read.
fn worst(comparisons: &[Compared]) -> (Worst, Worst) {
    let mut largest = [0.0_f64; 3];
    for compared in comparisons {
        for (index, [_, norm]) in compared.parts.iter().enumerate() {
            largest[index] = largest[index].max(*norm);
        }
    }
    let mut over_peak = [(0.0, 0); 3];
    let mut held = [(0.0, 0); 3];
    for (at, compared) in comparisons.iter().enumerate() {
        for (index, [difference, norm]) in compared.parts.iter().enumerate() {
            if largest[index] == 0.0 {
                continue;
            }
            let peaked = difference / largest[index];
            if past(peaked, over_peak[index].0) {
                over_peak[index] = (peaked, at);
            }
            let itself = difference / norm;
            if *norm >= 1.0e-2 * largest[index] && past(itself, held[index].0) {
                held[index] = (itself, at);
            }
        }
    }
    (over_peak, held)
}

impl Followed {
    /// Where the `at`th comparison was made.
    fn place(&self, at: usize) -> String {
        match (self.comparisons.get(at), self.at.get(at)) {
            (Some(compared), Some((generation, step))) => format!(
                "{:.2} s, step {step} of generation {generation}",
                compared.time
            ),
            _ => "no comparison".into(),
        }
    }

    fn time(&self, at: usize) -> f64 {
        self.comparisons
            .get(at)
            .map_or(0.0, |compared| compared.time)
    }
}

/// The verdict on what the reference found, with the study's lines.
fn judge(driver: &InterleavedDriver, followed: &Followed) -> Result<String, String> {
    let (over_peak, held) = worst(&followed.comparisons);
    let counts = &driver.counts;
    report(&format!(
        "STUDY interleaved seed {} adaptive {} switchable {}: {} actions, {} frames, {} model \
         steps, {} requests; {} installs, {} handoffs, {} refused, {} faults; taken {:?}, \
         refused {:?}; {:.1} s",
        driver.seed,
        driver.adaptive,
        driver.switchable,
        counts.fired,
        driver.frame,
        counts.model_steps,
        driver
            .lockstep
            .as_ref()
            .map_or(0, |lockstep| lockstep.requests()),
        counts.installs,
        counts.handoffs,
        counts.refused_handoffs,
        counts.faults,
        counts.taken,
        counts.refused,
        driver.started.elapsed().as_secs_f64()
    ));
    report(&format!(
        "STUDY interleaved seed {} replay: {} compared, {} skipped, {} at events, {} \
         generations, {} events, {} filters; over the peak Q {:.2e} at {:.2} s, b {:.2e} at \
         {:.2} s, z {:.2e} at {:.2} s; held Q {:.2e} at {:.2} s, b {:.2e} at {:.2} s, z {:.2e} \
         at {:.2} s; clock {:.1e}; cleanup {:.1e}; endpoint Q {:.2e} b {:.2e} at {:.2} s",
        driver.seed,
        followed.comparisons.len(),
        followed.skipped,
        followed.at_events,
        followed.generations,
        followed.events,
        followed.filters,
        over_peak[0].0,
        followed.time(over_peak[0].1),
        over_peak[1].0,
        followed.time(over_peak[1].1),
        over_peak[2].0,
        followed.time(over_peak[2].1),
        held[0].0,
        followed.time(held[0].1),
        held[1].0,
        followed.time(held[1].1),
        held[2].0,
        followed.time(held[2].1),
        followed.drift,
        followed.cleanup_gap,
        followed.endpoint[0],
        followed.endpoint[1],
        followed.end_time
    ));
    if followed.comparisons.is_empty() {
        return Err("no snapshot was compared".into());
    }
    for (index, name) in ["Q", "b", "the wall's poles"].iter().enumerate() {
        if past(over_peak[index].0, REPLAY_OVER_PEAK) {
            return Err(format!(
                "{name} stood {:.2e} of its peak from the reference at {}",
                over_peak[index].0,
                followed.place(over_peak[index].1)
            ));
        }
        if past(held[index].0, REPLAY_HELD) {
            return Err(format!(
                "{name} stood {:.2e} of itself from the reference at {}",
                held[index].0,
                followed.place(held[index].1)
            ));
        }
    }
    if !within(followed.drift, CLOCK_TOLERANCE) {
        return Err(format!("the clock drifted {:.1e} s", followed.drift));
    }
    if past(followed.cleanup_gap, CLEANUP_GAP) {
        return Err(format!(
            "a cleanup's increment stood {:.1e} of the flux from the reference's",
            followed.cleanup_gap
        ));
    }
    Ok(format!(
        "seed {}: {} actions, {} model steps in lockstep, {} generations, {} comparisons, \
         replay within {:.1e} of the peak",
        driver.seed,
        counts.fired,
        counts.model_steps,
        followed.generations,
        followed.comparisons.len(),
        over_peak
            .iter()
            .map(|(error, _)| *error)
            .fold(0.0, f64::max)
    ))
}
