//! RT-08: preparation scheduling. `preparation` opens [`SCENES`] in turn and
//! works each through two parts; `FUNFERN_E2E_PREPARATION` names one,
//! `replace` or `frames`, or both run.
//!
//! The replacing part asks a scene for the Fine mesh and, while it prepares,
//! edits it three times, opens the next scene, hands it to a worker that
//! stops a few quanta in, or to one that hands its result back only after a
//! newer request. After each, the app must come to rest running the document
//! as it stands: its revision, on the mesh a fresh preparation of it makes,
//! shown by the device, with the two workers alive. The active revision must
//! never go back, and no candidate may be ready but the newest request's.
//! The worker must leave a superseded job for the newer one: where a
//! preparation outlasts the edits' spacing three times, at most one of the
//! three edits' superseded jobs may come back finished. A late result must
//! be dropped; and a stopped worker's job must fail with its reason, light
//! the warning, and leave a new worker to take the remesh. Each edit lands
//! once the one before is being prepared.
//!
//! The frames part asks for a larger mesh and edits it, with the workers and
//! then without them, as the static-host page runs, and reports what each
//! frame cost the main thread while the app waited on each part of the work.
//! Natively on a hardware adapter, every frame past a scene's opening must
//! stay under [`FRAME_BOUND_MS`], and the cooperative runner's slices under
//! [`SLICE_BOUND_MS`]. A software adapter's frames take seconds, and a page
//! paces its own, so there the part is left out.

use super::lifecycle::{at_rest, edit};
use super::*;
use crate::topology_runtime::TopologyRuntime;
use crate::ui::workers::{BackgroundAmrWorker, BackgroundPreparationWorker};
use bevy::render::renderer::RenderAdapterInfo;

const SCENES: &[&str] = &["Double slit", "Plasma mirror"];

/// The replacing part's mesh, the Fine preset's: about 34,000 nodes on the
/// gallery's 2 x 2 domains, a preparation of a second or two on a worker,
/// which the part's acts land inside.
const FINE_EDGE: f64 = 0.04;

/// The frames part's: about 87,000 nodes, where the study behind C8.5 found
/// every frame and slice it bounds.
const LARGE_EDGE: f64 = 0.025;

/// The most a frame of the frames part may cost the main thread, from the
/// start of `First` to the end of `Last`. On the M1 Max the worst frame read
/// 37.5 ms in one run and 73 ms once in another, where before C8.5 a large
/// generation's upload took 46 to 128 ms and the cooperative runner's
/// unyielding steps up to 450.
const FRAME_BOUND_MS: f64 = 100.0;

/// The most a slice of the cooperative runner's may take. On the M1 Max the
/// longest read 23 to 31 ms, where the time-driven operator's laws, sampled
/// in one step before C8.5, took 115 to 446. The worker's quanta are left
/// out: they only delay its turn to the newest job, and stretched to 49 ms
/// beside the pack's thread.
const SLICE_BOUND_MS: f64 = 60.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Runner {
    Workers,
    MainThread,
}

#[derive(Clone, Copy, Debug)]
enum Act {
    Open,
    Frames(u32),
    Runner(Runner),
    Large,
    Edit,
    Small,
    /// The Fine mesh asked for, and three edits made while it prepares.
    Supersede,
    /// The Fine mesh asked for, and the next scene opened while it
    /// prepares.
    Switch,
    /// The Fine mesh asked for of a worker that stops a few quanta into it,
    /// then asked for again.
    StopWorker,
    /// The Fine mesh asked for of a worker that hands its result back only
    /// after a newer request, and an edit made while it prepares.
    LateResult,
}

const REPLACE_WORK: &[Act] = &[
    Act::Open,
    Act::Supersede,
    Act::Small,
    Act::Switch,
    Act::Open,
    Act::StopWorker,
    Act::Small,
    Act::LateResult,
    Act::Small,
];

const FRAMES_WORK: &[Act] = &[
    Act::Open,
    Act::Frames(60),
    Act::Large,
    Act::Frames(30),
    Act::Edit,
    Act::Frames(30),
    Act::Small,
    Act::Runner(Runner::MainThread),
    Act::Large,
    Act::Frames(30),
    Act::Edit,
    Act::Frames(30),
    Act::Small,
    Act::Runner(Runner::Workers),
];

/// What one act may take.
const ACT_SECONDS: u64 = 600;

#[derive(Clone, Copy, Debug)]
enum Stage {
    Take,
    Rest,
    Frames(u32),
    /// Acting each time the app's newest request is being prepared:
    /// `left` acts to go.
    Preparing {
        left: u32,
    },
    /// Waiting for the stopped worker's job to fail.
    Stopping,
}

/// Each frame's main-thread time, from the start of `First` to the end of
/// `Last`, and the time since the frame before began, with what the app
/// waited on.
#[derive(Resource, Default)]
struct FrameTimes {
    started: Option<Instant>,
    interval: f64,
    tag: &'static str,
    rows: Vec<(&'static str, f64, f64)>,
}

fn frame_started(mut times: ResMut<FrameTimes>) {
    let now = Instant::now();
    times.interval = times
        .started
        .map_or(0.0, |started| (now - started).as_secs_f64() * 1e3);
    times.started = Some(now);
}

fn frame_ended(mut times: ResMut<FrameTimes>) {
    let Some(started) = times.started else {
        return;
    };
    let busy = started.elapsed().as_secs_f64() * 1e3;
    let (tag, interval) = (times.tag, times.interval);
    times.rows.push((tag, busy, interval));
}

#[derive(Resource)]
pub(super) struct PreparationDriver {
    work: Vec<Act>,
    scene: usize,
    act: usize,
    stage: Stage,
    /// The scene's own edge, which `Small` returns to.
    edge: f64,
    runner: Runner,
    /// Whether the frames part's bounds hold here: natively, on a hardware
    /// adapter. Unknown until the first frame.
    judged: Option<bool>,
    act_started: Instant,
    act_deadline: Instant,
    deadline: Instant,
    done: bool,
    /// The revision of the topology last seen active, which must never go
    /// back.
    active_revision: u64,
    /// Preparations the act's worker finished after a newer request.
    discarded: usize,
    /// Whether the act has found a candidate ready for a request other
    /// than the app's newest.
    stale_ready: bool,
    /// When the act asked for a preparation, and each time it superseded
    /// one.
    asked: Vec<Instant>,
    findings: Vec<String>,
}

impl PreparationDriver {
    fn act(&self) -> Act {
        self.work[self.act]
    }

    fn find(&mut self, finding: String) {
        let finding = format!("{}, {:?}: {finding}", SCENES[self.scene], self.act());
        report(&format!("e2e preparation: {finding}"));
        self.findings.push(finding);
    }

    /// Moves on to the next act or scene; `false` once every scene is done.
    fn next(&mut self) -> bool {
        self.stage = Stage::Take;
        self.act_started = Instant::now();
        self.act_deadline = self.act_started + std::time::Duration::from_secs(ACT_SECONDS);
        self.discarded = 0;
        self.stale_ready = false;
        self.asked.clear();
        self.act += 1;
        if self.act < self.work.len() {
            return true;
        }
        self.act = 0;
        self.scene += 1;
        self.scene < SCENES.len()
    }
}

/// Adds the preparation fixture's driver and its frame clock.
pub(super) fn add(app: &mut App, seconds: Option<u64>) {
    mark("running", "");
    report(&format!("e2e build {}", crate::BUILD_ID));
    app.insert_resource(FrameTimes::default())
        .insert_resource(PreparationDriver {
            work: Vec::new(),
            scene: 0,
            act: 0,
            stage: Stage::Take,
            edge: 0.0,
            runner: Runner::Workers,
            judged: None,
            act_started: Instant::now(),
            act_deadline: Instant::now() + std::time::Duration::from_secs(ACT_SECONDS),
            deadline: Instant::now() + std::time::Duration::from_secs(seconds.unwrap_or(3600)),
            done: false,
            active_revision: 0,
            discarded: 0,
            stale_ready: false,
            asked: Vec::new(),
            findings: Vec::new(),
        })
        .add_systems(First, frame_started)
        .add_systems(Last, frame_ended)
        .add_systems(EguiPrimaryContextPass, drive_preparation.before(frame));
}

fn drive_preparation(
    mut driver: ResMut<PreparationDriver>,
    mut state: ResMut<Playground>,
    request: Res<CanonicalGpuRequest>,
    display: Res<CanonicalGpuDisplay>,
    adapter: Option<Res<RenderAdapterInfo>>,
    mut times: ResMut<FrameTimes>,
    mut exit: MessageWriter<AppExit>,
) {
    if driver.done || !state.startup_done {
        return;
    }
    if driver.judged.is_none() {
        let hardware = adapter.is_some_and(|adapter| adapter.device_type != wgpu::DeviceType::Cpu);
        let judged = cfg!(not(target_arch = "wasm32")) && hardware;
        let part = std::env::var("FUNFERN_E2E_PREPARATION").ok();
        let replace = part.as_deref() != Some("frames");
        let frames = part.as_deref() != Some("replace");
        if frames && !judged {
            report(
                "e2e preparation: the frames part judges a hardware adapter's frames natively, and is left out here",
            );
        }
        driver.judged = Some(judged);
        driver.work = [
            if replace { REPLACE_WORK } else { &[] },
            if frames && judged { FRAMES_WORK } else { &[] },
        ]
        .concat();
        if driver.work.is_empty() {
            driver.work.push(Act::Open);
            driver.find("only the frames part asked for, where it is left out".into());
        }
        state.protocol_log = ProtocolLog::kept();
    }
    times.tag = waiting(&state, &request).unwrap_or(match driver.stage {
        Stage::Frames(_) => "running",
        _ => "at rest",
    });
    driver.discarded += state
        .protocol_log
        .take()
        .iter()
        .filter(|note| **note == ProtocolNote::Discarded)
        .count();
    // A new request drops what was ready, and only the request's own
    // preparation is taken, so a candidate is the newest request's or none.
    if let Some(ready) = state.runtime.ready()
        && Some(ready.bundle.token) != state.runtime.requested()
        && !driver.stale_ready
    {
        driver.stale_ready = true;
        let (ready, requested) = (ready.bundle.token, state.runtime.requested());
        driver.find(format!(
            "{ready:?} is ready where {requested:?} is asked for"
        ));
    }
    if let Some(active) = state.runtime.active() {
        let revision = active.bundle.token.document_revision;
        if revision < driver.active_revision {
            let previous = driver.active_revision;
            driver.find(format!("revision {revision} active after {previous}"));
        }
        driver.active_revision = revision;
    }
    let outcome = advance(&mut driver, &mut state, &request, &display, &mut times);
    let now = Instant::now();
    let verdict = match outcome {
        Ok(true) if now < driver.act_deadline && now < driver.deadline => return,
        Ok(true) => Err(format!(
            "did not finish in time: {}, {:?} at {:?}",
            SCENES[driver.scene],
            driver.act(),
            driver.stage
        )),
        Ok(false) if driver.findings.is_empty() => Ok(format!("{} scenes", SCENES.len())),
        Ok(false) => Err(driver.findings.join("; ")),
        Err(failure) => Err(failure),
    };
    driver.done = true;
    state.wave_running = false;
    match verdict {
        Ok(summary) => {
            report(&format!("e2e preparation: {summary}"));
            mark("pass", &summary);
            exit.write(AppExit::Success);
        }
        Err(failure) => {
            report(&format!("e2e preparation FAILED: {failure}"));
            mark("fail", &failure);
            exit.write(AppExit::error());
        }
    }
}

/// Opens the gallery scene named `name`, adaptation off so that only the
/// acts change its mesh.
fn open(state: &mut Playground, name: &str) -> Result<(), String> {
    let catalog = crate::topology_examples::catalog();
    let index = catalog
        .iter()
        .position(|example| example.name == name)
        .ok_or_else(|| format!("no gallery scene named {name:?}"))?;
    state.open_example(index);
    if state.example_opened != Some(index) {
        return Err(format!("{name} did not open"));
    }
    state.editor.document.presentation.adaptation.enabled = false;
    Ok(())
}

/// Asks for a mesh at `edge`.
fn remesh(state: &mut Playground, edge: f64) {
    state.editor.document.presentation.mesh_edge = edge;
    state.coordinator.remesh_requested = true;
}

/// Takes the current act a frame further: `Ok(false)` once every scene is
/// done.
fn advance(
    driver: &mut PreparationDriver,
    state: &mut Playground,
    request: &CanonicalGpuRequest,
    display: &CanonicalGpuDisplay,
    times: &mut FrameTimes,
) -> Result<bool, String> {
    let act = driver.act();
    let stage = match (act, driver.stage) {
        (Act::Open, Stage::Take) => {
            open(state, SCENES[driver.scene])?;
            // A scene's revisions count from its own start.
            driver.active_revision = 0;
            driver.edge = state.editor.document.presentation.mesh_edge;
            state.wave_running = true;
            Stage::Rest
        }
        (Act::Runner(runner), Stage::Take) => {
            driver.runner = runner;
            match runner {
                Runner::MainThread => {
                    state.background_preparation = None;
                    state.background_amr = None;
                }
                Runner::Workers => {
                    if state.background_preparation.is_none() {
                        state.background_preparation = BackgroundPreparationWorker::spawn();
                    }
                    if state.background_amr.is_none() {
                        state.background_amr = BackgroundAmrWorker::spawn();
                    }
                }
            }
            return Ok(driver.next());
        }
        (Act::Large, Stage::Take) => {
            remesh(state, LARGE_EDGE);
            times.rows.clear();
            Stage::Rest
        }
        (Act::Edit, Stage::Take) => {
            edit(state)?;
            times.rows.clear();
            Stage::Rest
        }
        (Act::Small, Stage::Take) => {
            remesh(state, driver.edge);
            times.rows.clear();
            Stage::Rest
        }
        (Act::Supersede, Stage::Take) => {
            remesh(state, FINE_EDGE);
            driver.asked.push(Instant::now());
            Stage::Preparing { left: 3 }
        }
        (Act::Switch, Stage::Take) => {
            remesh(state, FINE_EDGE);
            Stage::Preparing { left: 1 }
        }
        (Act::Supersede | Act::Switch | Act::LateResult, Stage::Preparing { left }) => {
            // Each act lands while the newest request is in hand, a frame
            // or so into its preparation.
            let in_hand = state.preparation_in_progress()
                && state
                    .runtime
                    .requested()
                    .is_some_and(|token| token.document_revision == state.editor.revision);
            if !in_hand {
                return Ok(true);
            }
            driver.asked.push(Instant::now());
            if matches!(act, Act::Switch) {
                open(state, SCENES[(driver.scene + 1) % SCENES.len()])?;
                driver.active_revision = 0;
            } else {
                edit(state)?;
            }
            match left.checked_sub(1) {
                Some(left) if left > 0 => Stage::Preparing { left },
                _ => Stage::Rest,
            }
        }
        (Act::StopWorker | Act::LateResult, Stage::Take) => {
            let stopping = matches!(act, Act::StopWorker);
            let Some(worker) = stand_in_worker(stopping) else {
                report(&format!(
                    "e2e preparation: {act:?} stands a thread in for the worker, and a page's workers are its pool's"
                ));
                return Ok(driver.next());
            };
            state.background_preparation = Some(worker);
            remesh(state, FINE_EDGE);
            if stopping {
                Stage::Stopping
            } else {
                Stage::Preparing { left: 1 }
            }
        }
        (Act::StopWorker, Stage::Stopping) => {
            if state.preparation_in_progress() || state.runtime.last_error().is_none() {
                return Ok(true);
            }
            let error = state.runtime.last_error().unwrap().message.clone();
            if error != "the preparation worker stopped" {
                driver.find(format!("the stopped worker's job failed with {error:?}"));
            }
            if !state.diagnostics_warning() {
                driver.find("the stopped worker lit no warning".into());
            }
            if state
                .background_preparation
                .as_ref()
                .is_none_or(|worker| worker.token.is_some())
            {
                driver.find("no new worker took the stopped one's place".into());
            }
            state.unseen_error = false;
            state.coordinator.remesh_requested = true;
            Stage::Rest
        }
        (Act::Frames(frames), Stage::Take) => {
            times.rows.clear();
            Stage::Frames(frames)
        }
        (Act::Frames(_), Stage::Frames(left)) => match left.checked_sub(1) {
            Some(left) if left > 0 => Stage::Frames(left),
            _ => {
                summarize(driver, state, times);
                return Ok(driver.next());
            }
        },
        (_, Stage::Rest) => {
            if !at_rest(state, request, display) {
                return Ok(true);
            }
            summarize(driver, state, times);
            judge(driver, state, display);
            // The late worker keeps to every job it is sent; the acts after
            // take the app's own, which leaves a superseded one.
            if matches!(act, Act::LateResult) {
                state.background_preparation = BackgroundPreparationWorker::spawn();
            }
            return Ok(driver.next());
        }
        (act, stage) => return Err(format!("no step for {act:?} at {stage:?}")),
    };
    driver.stage = stage;
    Ok(true)
}

/// Reports what the act's frames cost, by what the app waited on, and holds
/// the frames part's frames and slices to their bounds where they are
/// judged.
fn summarize(driver: &mut PreparationDriver, state: &Playground, times: &mut FrameTimes) {
    let rows = std::mem::take(&mut times.rows);
    let mut by_tag: BTreeMap<&'static str, (u32, f64, f64, f64)> = BTreeMap::new();
    for (tag, busy, interval) in &rows {
        let entry = by_tag.entry(tag).or_default();
        entry.0 += 1;
        entry.1 += busy;
        entry.2 = entry.2.max(*busy);
        entry.3 = entry.3.max(*interval);
    }
    let stages = by_tag
        .iter()
        .map(|(tag, (count, total, busy, interval))| {
            format!(
                "{tag} {count} frames, busy {:.1} mean {busy:.1} max, interval {interval:.1} max",
                total / f64::from(*count)
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let nodes = state
        .runtime
        .active()
        .map_or(0, |active| active.canonical_operator.degrees_of_freedom());
    let timing = state
        .runtime
        .active()
        .map(|active| active.timing)
        .unwrap_or_default();
    report(&format!(
        "e2e preparation: {}, {:?}, {:?}: {:.1} s, {nodes} nodes, preparation {:.0} ms in {} slices, longest {:.1} ms; {stages}",
        SCENES[driver.scene],
        driver.runner,
        driver.act(),
        driver.act_started.elapsed().as_secs_f64(),
        timing.work_ms,
        timing.slices,
        timing.longest_slice_ms,
    ));
    let framed = matches!(
        driver.act(),
        Act::Large | Act::Edit | Act::Small | Act::Frames(_)
    );
    if !framed || driver.judged != Some(true) {
        return;
    }
    if let Some((tag, busy, _)) = rows
        .iter()
        .copied()
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .filter(|(_, busy, _)| *busy > FRAME_BOUND_MS)
    {
        let runner = driver.runner;
        driver.find(format!(
            "{runner:?}: a frame took the main thread {busy:.1} ms, waiting on {tag}, past {FRAME_BOUND_MS} ms"
        ));
    }
    if driver.runner == Runner::MainThread
        && !matches!(driver.act(), Act::Frames(_))
        && timing.longest_slice_ms > SLICE_BOUND_MS
    {
        driver.find(format!(
            "a slice of the cooperative runner's took {:.1} ms, past {SLICE_BOUND_MS} ms",
            timing.longest_slice_ms
        ));
    }
}

/// The replacing part's judgement of an act come to rest.
fn judge(driver: &mut PreparationDriver, state: &Playground, display: &CanonicalGpuDisplay) {
    let act = driver.act();
    if !matches!(
        act,
        Act::Supersede | Act::Switch | Act::StopWorker | Act::LateResult
    ) {
        return;
    }
    if let Err(finding) = runs_the_document(state, display) {
        driver.find(finding);
    }
    let discarded = driver.discarded;
    match act {
        Act::LateResult if discarded != 1 => driver.find(format!(
            "{discarded} results dropped as late, where the worker handed back one"
        )),
        // Each edit lands soon after the job before it was asked for, which
        // then finishes only if it is that short or the worker keeps to it.
        // The plasma mirror's Fine mesh takes 0.5 s on lavapipe, three of
        // its frames, and two of three came back so when the edits were
        // three frames apart.
        Act::Supersede => {
            let work = state
                .runtime
                .active()
                .map_or(0.0, |active| active.timing.work_ms);
            let spacing = driver
                .asked
                .windows(2)
                .map(|pair| (pair[1] - pair[0]).as_secs_f64() * 1e3)
                .fold(0.0, f64::max);
            if work < 3.0 * spacing {
                report(&format!(
                    "e2e preparation: {}, {act:?}: {discarded} back finished, not judged: a preparation of {work:.0} ms against edits up to {spacing:.0} ms apart",
                    SCENES[driver.scene]
                ));
            } else if discarded > 1 {
                driver.find(format!(
                    "{discarded} superseded preparations came back finished: the worker kept to them"
                ));
            }
        }
        _ => {}
    }
}

/// Whether the app runs the document as it stands: its revision, on the mesh
/// a fresh preparation of it makes, on the device, with the two workers
/// alive and no more.
fn runs_the_document(state: &Playground, display: &CanonicalGpuDisplay) -> Result<(), String> {
    let active = state.runtime.active().ok_or("nothing active")?;
    let document = &state.editor.document;
    let edge = document.presentation.mesh_edge;
    let domain = document.model.accepted.geometry.domain;
    let options = MeshingOptions {
        target_edge_length: edge,
        curve_tolerance: (edge * 0.02).min(5e-4),
        ..MeshingOptions::default()
    }
    .sized_for_area(domain.width() * domain.height());
    let mut runtime = TopologyRuntime::default();
    let token = runtime.request(
        state.editor.revision,
        document,
        state.editor.compiled_accepted.clone(),
        options,
        true,
    )?;
    runtime
        .advance_for(std::time::Duration::from_secs(600))
        .ok_or("the fresh preparation did not finish")?
        .map_err(|error| error.message)?;
    let fresh = runtime.commit_ready(token)?;
    let mut findings = Vec::new();
    if active.bundle.token.document_revision != state.editor.revision {
        findings.push(format!(
            "revision {} runs where the document is at {}",
            active.bundle.token.document_revision, state.editor.revision
        ));
    }
    if active.mesh.vertices != fresh.mesh.vertices || active.mesh.triangles != fresh.mesh.triangles
    {
        findings.push(format!(
            "the running mesh's {} triangles are not a fresh preparation's {}",
            active.mesh.triangles.len(),
            fresh.mesh.triangles.len()
        ));
    }
    let nodes = active.canonical_operator.degrees_of_freedom();
    if display.primary_flux.len() != nodes {
        findings.push(format!(
            "the device shows {} nodes of the active {nodes}",
            display.primary_flux.len()
        ));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let threads = crate::live_resources::live_threads();
        if threads != 2 {
            findings.push(format!("{threads} worker threads alive"));
        }
    }
    if findings.is_empty() {
        Ok(())
    } else {
        Err(findings.join(", "))
    }
}

/// The worker that stops, or the one that is late; none in the browser.
#[cfg(not(target_arch = "wasm32"))]
fn stand_in_worker(stopping: bool) -> Option<BackgroundPreparationWorker> {
    Some(if stopping {
        dying_worker()
    } else {
        late_worker()
    })
}

#[cfg(target_arch = "wasm32")]
fn stand_in_worker(_stopping: bool) -> Option<BackgroundPreparationWorker> {
    None
}

/// A worker that takes one job, works a few quanta of it and stops, as a
/// panic in a job stops the real one.
#[cfg(not(target_arch = "wasm32"))]
fn dying_worker() -> BackgroundPreparationWorker {
    use crate::topology_runtime::TopologyPreparationJob;
    use crate::ui::workers::BackgroundPreparationEvent;
    let (sender, jobs) = std::sync::mpsc::channel::<TopologyPreparationJob>();
    let (events, receiver) = std::sync::mpsc::channel::<BackgroundPreparationEvent>();
    std::thread::spawn(move || {
        let _live = crate::live_resources::live_thread();
        let _ = events.send(BackgroundPreparationEvent::WorkerStarted);
        if let Ok(mut job) = jobs.recv() {
            for _ in 0..4 {
                if job
                    .advance_for(std::time::Duration::from_millis(8))
                    .is_some()
                {
                    break;
                }
            }
        }
    });
    worker(sender, receiver)
}

/// A worker that works its first job to the end and hands the result back
/// only once a newer job has come, as a worker finishing in the frame a
/// newer request is made does; then each job in turn, to the end.
#[cfg(not(target_arch = "wasm32"))]
fn late_worker() -> BackgroundPreparationWorker {
    use crate::topology_runtime::TopologyPreparationJob;
    use crate::ui::workers::BackgroundPreparationEvent;
    let (sender, jobs) = std::sync::mpsc::channel::<TopologyPreparationJob>();
    let (events, receiver) = std::sync::mpsc::channel::<BackgroundPreparationEvent>();
    std::thread::spawn(move || {
        let _live = crate::live_resources::live_thread();
        let finish = |mut job: TopologyPreparationJob| {
            let token = job.token();
            loop {
                if let Some(result) = job.advance_for(std::time::Duration::from_millis(8)) {
                    break BackgroundPreparationEvent::Finished {
                        token,
                        result: Box::new(result),
                    };
                }
            }
        };
        if events
            .send(BackgroundPreparationEvent::WorkerStarted)
            .is_err()
        {
            return;
        }
        let Ok(first) = jobs.recv() else {
            return;
        };
        let late = finish(first);
        let Ok(mut job) = jobs.recv() else {
            return;
        };
        if events.send(late).is_err() {
            return;
        }
        loop {
            if events.send(finish(job)).is_err() {
                return;
            }
            match jobs.recv() {
                Ok(next) => job = next,
                Err(_) => return,
            }
        }
    });
    worker(sender, receiver)
}

#[cfg(not(target_arch = "wasm32"))]
fn worker(
    sender: std::sync::mpsc::Sender<crate::topology_runtime::TopologyPreparationJob>,
    receiver: std::sync::mpsc::Receiver<crate::ui::workers::BackgroundPreparationEvent>,
) -> BackgroundPreparationWorker {
    BackgroundPreparationWorker {
        sender,
        receiver: std::sync::Mutex::new(receiver),
        token: None,
        phase: None,
        detail: None,
        timing: None,
    }
}
