//! RT-07 and section 5.4 of the 11 October testing plan: the same work
//! leaves the app holding the same. `lifecycle` takes the gallery's
//! [`SCENES`] in turn, [`CYCLES`] times over, each opened as the gallery
//! opens it, its adaptation on and refining to half the scene's own edge at
//! most, and through the app's own handlers puts each through the work that
//! makes and drops what the device holds ([`WORK`]): a pulse, a medium
//! edited, the overlays on and off, a coarser mesh, a full snapshot, Reset, a
//! fault and the run resumed past it, and a handoff the device refuses,
//! asked for again. After each scene's work, adaptation off, back on the
//! scene's own mesh and the run paused and drained, it counts what the app
//! holds (`live_resources`): the shader buffers in the main world and on the
//! device, the readbacks and their staging, the snapshot's staging, the
//! field paint's buffers, the bind groups each consumer keeps, the
//! pipelines, the entities of both worlds, the worker threads alive, and of
//! the meshes and operators the generations were built on, those alive.
//!
//! Between them the scenes must make every bind group the app keeps. Each
//! count must hold what any count must ([`held`]) - the device's shader
//! buffers the main world's, no more staging than its readbacks can have in
//! flight, no handoff's or live event's bind groups kept, one generation
//! alive, no work in hand - and be the one the same scene left on the cycle
//! before: a bounded workload, a bounded steady state.

use super::*;
use crate::document::{MaterialOverlay, MaterialProperty, VectorOverlay};
use crate::live_resources::{RenderCount, RenderCounts, Tally};
use crate::paced_readback::PacedReadback;
use crate::topology_runtime::PreparedTopology;
use bevy::ecs::archetype::Archetypes;
use bevy::render::storage::ShaderBuffer;
use funfern_core::{CanonicalTemporalWaveOperator, TriMesh};
use std::sync::Weak;

/// The gallery's scenes the fixture cycles, `batches`'s: between them every
/// recorder - point, line and area probes and the far field - and
/// adaptation on in each.
const SCENES: &[&str] = &[
    "Spatial soliton",
    "Josephson line",
    "Self-sustained emitter",
    "Ring resonator",
    "Double slit",
];

/// Times the scenes are cycled: the second must find what the app holds as
/// the first left it. The study's third, on lavapipe, read every count as
/// the second had.
const CYCLES: usize = 2;

/// What each scene is put through, in order.
const WORK: &[Act] = &[
    Act::Open,
    Act::Frames(60),
    Act::Pulse,
    Act::Edit,
    Act::Overlays(true),
    Act::Frames(20),
    Act::Coarser,
    Act::Frames(20),
    Act::Snapshot,
    Act::Reset,
    Act::Fault,
    Act::RefusedHandoff,
    Act::Overlays(false),
    Act::Frames(60),
    Act::Count,
];

/// Every bind group the app keeps, by its holder, which the scenes must
/// make between them.
const HOLDERS: &[&str] = &[
    "step",
    "handoff",
    "live event",
    "point probes",
    "line probes",
    "area probes",
    "far field",
    "vector overlay",
    "field paint",
];

/// Frames a count waits once the run is paused, past the readback pool's
/// ten unused frames, and the frames its counts must then hold still for.
const SETTLE_FRAMES: u32 = 15;
const STILL_FRAMES: u32 = 5;

/// The share of the run's time one act may take, 300 s of the native 1800
/// and more on the browser's slower pages, and how long one takes before it
/// is reported.
const ACT_SHARE: u32 = 6;
const SLOW_ACT_SECONDS: f64 = 10.0;

#[derive(Clone, Copy, Debug)]
enum Act {
    /// The scene, as the gallery opens it.
    Open,
    /// The run goes on for this many frames.
    Frames(u32),
    /// A pulse at the domain's centre.
    Pulse,
    /// The first material's density toggled.
    Edit,
    /// The vector overlay and the density overlay on, or both off.
    Overlays(bool),
    /// The mesh edge a quarter longer, and Remesh.
    Coarser,
    /// A full snapshot, asked for and waited for.
    Snapshot,
    Reset,
    /// A fault armed on the running generation, its notice dismissed, and
    /// Run pressed past it.
    Fault,
    /// An edit whose candidate the device refuses, then Remesh to ask again.
    RefusedHandoff,
    /// Adaptation off, the scene's own mesh, the run paused and drained,
    /// and what the app holds counted.
    Count,
}

/// Where an act has got to.
#[derive(Clone, Copy, Debug)]
enum Stage {
    /// Not yet taken.
    Take,
    /// Taken; waiting for the app to come to rest.
    Rest,
    /// Frames left to run.
    Frames(u32),
    /// The snapshot asked for, after this many had arrived.
    Snapshot(u64),
    /// The fault armed, its failure not yet seen.
    Failing,
    /// The refused edit sent, its refusal not yet seen.
    Refusing,
    /// The run paused for a count: frames waited, and frames the counts
    /// have held still.
    Settling { waited: u32, still: u32 },
}

/// What the main world holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct MainCounts {
    shader_buffers: usize,
    readbacks: usize,
    entities: u32,
    threads: usize,
    /// The generations' meshes and operators seen and still alive.
    generations: Generations,
    /// Work the app holds that rest does not wait for.
    jobs: Vec<&'static str>,
}

/// Of every mesh, operator and time-driven operator a generation the runtime
/// held active or ready was built on, how many are alive: whatever else the
/// app keeps of them keeps them alive too.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Generations {
    meshes: usize,
    operators: usize,
    temporal_operators: usize,
}

/// What the generations were built on, held weakly, each once.
#[derive(Default)]
struct Seen {
    meshes: Vec<Weak<TriMesh>>,
    operators: Vec<Weak<CanonicalWaveOperator>>,
    temporal_operators: Vec<Weak<CanonicalTemporalWaveOperator>>,
}

impl Seen {
    fn see(&mut self, generation: &PreparedTopology) {
        fn once<T>(seen: &mut Vec<Weak<T>>, held: &Arc<T>) {
            seen.retain(|weak| weak.strong_count() > 0);
            if !seen
                .iter()
                .any(|weak| std::ptr::eq(weak.as_ptr(), Arc::as_ptr(held)))
            {
                seen.push(Arc::downgrade(held));
            }
        }
        once(&mut self.meshes, &generation.mesh);
        once(&mut self.operators, &generation.canonical_operator);
        if let Some(temporal) = &generation.canonical_temporal_operator {
            once(&mut self.temporal_operators, temporal);
        }
    }

    fn alive(&self) -> Generations {
        fn alive<T>(seen: &[Weak<T>]) -> usize {
            seen.iter().filter(|weak| weak.strong_count() > 0).count()
        }
        Generations {
            meshes: alive(&self.meshes),
            operators: alive(&self.operators),
            temporal_operators: alive(&self.temporal_operators),
        }
    }
}

/// One count, after a scene's work on a cycle.
struct Counted {
    cycle: usize,
    scene: &'static str,
    nodes: usize,
    /// How long the scene's work took.
    seconds: f64,
    main: MainCounts,
    render: RenderCounts,
}

impl Counted {
    /// What must be the same on the next cycle: everything but the frame, the
    /// copies in flight, and the snapshot staging's bytes. That one buffer
    /// is sized to the state the last snapshot copied, taken while adaptation
    /// refines at the device's pace: on the M1 Max the emitter's read
    /// 566,112 B on one cycle and 369,888 B on the next.
    fn compared(&self) -> (MainCounts, RenderCounts) {
        let render = RenderCounts {
            frame: 0,
            copies_in_flight: 0,
            snapshot_staging: Tally {
                bytes: 0,
                ..self.render.snapshot_staging
            },
            ..self.render.clone()
        };
        (self.main.clone(), render)
    }

    fn line(&self) -> String {
        let (main, render) = (&self.main, &self.render);
        let groups = render
            .bind_groups
            .iter()
            .map(|(holder, count)| format!("{holder} {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "cycle {}, {} ({} nodes, {:.0} s): shader buffers {}, on the device {}, readbacks {}, \
             their staging {} with {} copies in flight, snapshot staging {}, paint {}, \
             bind groups {} ({groups}), pipelines {}, entities {} and {}, threads {}, \
             generations' meshes {}, operators {} and time-driven operators {} alive{}",
            self.cycle,
            self.scene,
            self.nodes,
            self.seconds,
            main.shader_buffers,
            render.device_buffers,
            main.readbacks,
            render.readback_staging,
            render.copies_in_flight,
            render.snapshot_staging,
            render.paint_buffers,
            render.bind_group_count(),
            render.pipelines,
            main.entities,
            render.entities,
            main.threads,
            main.generations.meshes,
            main.generations.operators,
            main.generations.temporal_operators,
            if main.jobs.is_empty() {
                String::new()
            } else {
                format!(", jobs {}", main.jobs.join(", "))
            },
        )
    }
}

#[derive(Resource)]
pub(super) struct LifecycleDriver {
    /// The scene and the act under way, by index.
    cycle: usize,
    scene: usize,
    act: usize,
    stage: Stage,
    /// The scene's own mesh edge, as the gallery opened it, and when it
    /// opened.
    edge: f64,
    opened: Instant,
    count: RenderCount,
    counted: Vec<Counted>,
    /// Every bind group holder seen holding one, on any frame.
    holders: BTreeSet<&'static str>,
    seen: Seen,
    /// The last counts read while settling, to see them hold still.
    last: Option<(MainCounts, RenderCounts)>,
    act_started: Instant,
    act_time: std::time::Duration,
    act_deadline: Instant,
    deadline: Instant,
    done: bool,
}

impl LifecycleDriver {
    fn scene(&self) -> &'static str {
        SCENES[self.scene]
    }

    fn act(&self) -> Act {
        WORK[self.act]
    }

    /// Moves on to the next act, the next scene's first, or the next cycle's;
    /// `false` once every cycle is done.
    fn next(&mut self, nodes: usize) -> bool {
        let took = self.act_started.elapsed().as_secs_f64();
        if took > SLOW_ACT_SECONDS {
            report(&format!(
                "e2e lifecycle: cycle {}, {}: {:?} took {took:.0} s, {nodes} nodes",
                self.cycle,
                self.scene(),
                self.act()
            ));
        }
        self.stage = Stage::Take;
        self.act_started = Instant::now();
        self.act_deadline = self.act_started + self.act_time;
        self.act += 1;
        if self.act < WORK.len() {
            return true;
        }
        self.act = 0;
        self.scene += 1;
        if self.scene < SCENES.len() {
            return true;
        }
        self.scene = 0;
        self.cycle += 1;
        self.cycle <= CYCLES
    }
}

/// Adds the lifecycle fixture's driver, and the render world's count.
pub(super) fn add(app: &mut App, seconds: Option<u64>) {
    mark("running", "");
    report(&format!("e2e build {}", crate::BUILD_ID));
    let count = crate::live_resources::add_render_count(app);
    let run_time = std::time::Duration::from_secs(seconds.unwrap_or(1800));
    let act_time = run_time / ACT_SHARE;
    app.insert_resource(LifecycleDriver {
        cycle: 1,
        scene: 0,
        act: 0,
        stage: Stage::Take,
        edge: 0.0,
        opened: Instant::now(),
        count,
        counted: Vec::new(),
        holders: BTreeSet::new(),
        seen: Seen::default(),
        last: None,
        act_started: Instant::now(),
        act_time,
        act_deadline: Instant::now() + act_time,
        deadline: Instant::now() + run_time,
        done: false,
    })
    .add_systems(EguiPrimaryContextPass, drive_lifecycle.before(frame));
}

#[allow(clippy::too_many_arguments)]
fn drive_lifecycle(
    mut driver: ResMut<LifecycleDriver>,
    mut state: ResMut<Playground>,
    mut request: ResMut<CanonicalGpuRequest>,
    display: Res<CanonicalGpuDisplay>,
    mut assets: ResMut<Assets<ShaderBuffer>>,
    readbacks: Query<(), With<PacedReadback>>,
    archetypes: &Archetypes,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    if driver.done || !state.startup_done {
        return;
    }
    let render = driver.count.0.lock().unwrap().clone();
    for (holder, _) in &render.bind_groups {
        driver.holders.insert(holder);
    }
    if let Some(active) = state.runtime.active() {
        driver.seen.see(active);
    }
    if let Some(ready) = state.runtime.ready() {
        driver.seen.see(ready);
    }
    let main = main_counts(
        &state,
        &assets,
        readbacks.iter().count(),
        archetypes,
        driver.seen.alive(),
    );
    let outcome = advance(
        &mut driver,
        &mut state,
        &mut request,
        &display,
        &mut assets,
        &mut commands,
        main,
        render,
    );
    let now = Instant::now();
    let verdict = match outcome {
        Ok(true) if now < driver.act_deadline && now < driver.deadline => return,
        Ok(true) => Err(format!(
            "did not finish in time: cycle {}, {}, {:?} at {:?}",
            driver.cycle,
            driver.scene(),
            driver.act(),
            driver.stage
        )),
        Ok(false) => judge(&driver),
        Err(failure) => Err(failure),
    };
    driver.done = true;
    state.wave_running = false;
    match verdict {
        Ok(summary) => {
            report(&format!("e2e lifecycle: {summary}"));
            mark("pass", &summary);
            exit.write(AppExit::Success);
        }
        Err(failure) => {
            report(&format!("e2e lifecycle FAILED: {failure}"));
            mark("fail", &failure);
            exit.write(AppExit::error());
        }
    }
}

fn main_counts(
    state: &Playground,
    assets: &Assets<ShaderBuffer>,
    readbacks: usize,
    archetypes: &Archetypes,
    generations: Generations,
) -> MainCounts {
    let jobs = [
        ("a material overlay", state.material_overlay_job.is_some()),
        ("a handoff's flux solve", state.flux_cleanup_job.is_some()),
        ("an estimate", state.amr_indicator_job.is_some()),
        ("an adaptation", state.amr_adaptation_job.is_some()),
    ];
    MainCounts {
        shader_buffers: assets.len(),
        readbacks,
        entities: crate::live_resources::entities(archetypes),
        threads: crate::live_resources::live_threads(),
        generations,
        jobs: jobs
            .iter()
            .filter(|(_, held)| *held)
            .map(|(job, _)| *job)
            .collect(),
    }
}

/// Whether what the app runs is the document's accepted revision, with
/// nothing left in hand.
fn at_rest(
    state: &Playground,
    request: &CanonicalGpuRequest,
    display: &CanonicalGpuDisplay,
) -> bool {
    waiting(state, request).is_none()
        && !state.request_is_due()
        && !state.coordinator.remesh_requested
        && state.runtime.active().is_some_and(|active| {
            active.bundle.token.document_revision == state.editor.revision
                && *active.bundle.authored == state.editor.document.model.accepted
        })
        && request.ready()
        && request.generation() != 0
        && display.generation == request.generation()
}

/// Takes the current act a frame further: `Ok(false)` once every cycle is
/// done.
#[allow(clippy::too_many_arguments)]
fn advance(
    driver: &mut LifecycleDriver,
    state: &mut Playground,
    request: &mut CanonicalGpuRequest,
    display: &CanonicalGpuDisplay,
    assets: &mut Assets<ShaderBuffer>,
    commands: &mut Commands,
    main: MainCounts,
    render: RenderCounts,
) -> Result<bool, String> {
    let act = driver.act();
    let stage = match (act, driver.stage) {
        (Act::Open, Stage::Take) => {
            let catalog = crate::topology_examples::catalog();
            let index = catalog
                .iter()
                .position(|example| example.name == driver.scene())
                .ok_or_else(|| format!("no gallery scene named {:?}", driver.scene()))?;
            state.open_example(index);
            if state.example_opened != Some(index) {
                return Err(format!("{} did not open", driver.scene()));
            }
            // Adaptation refines to half the scene's own edge at most: its
            // jobs and handoffs run, and the mesh stays one a slow device
            // steps in a frame.
            let presentation = &mut state.editor.document.presentation;
            driver.edge = presentation.mesh_edge;
            presentation.adaptation.minimum_edge =
                presentation.adaptation.minimum_edge.max(0.5 * driver.edge);
            driver.opened = Instant::now();
            state.wave_running = true;
            Stage::Rest
        }
        (Act::Frames(frames), Stage::Take) => Stage::Frames(frames),
        (Act::Frames(_), Stage::Frames(left)) => match left.checked_sub(1) {
            Some(left) if left > 0 => Stage::Frames(left),
            _ => return Ok(driver.next(display.primary_flux.len())),
        },
        (Act::Pulse, Stage::Take) => {
            let domain = state.editor.document.model.draft.geometry.domain;
            state.place_pulse(Point2::new(
                domain.min_x + 0.5 * domain.width(),
                domain.min_y + 0.5 * domain.height(),
            ));
            Stage::Rest
        }
        (Act::Edit, Stage::Take) => {
            edit(state)?;
            Stage::Rest
        }
        (Act::Overlays(on), Stage::Take) => {
            let physics = state.editor.document.model.draft.physics;
            let presentation = &mut state.editor.document.presentation;
            presentation.vector_overlay = if on {
                VectorOverlay::choices(physics)
                    .iter()
                    .copied()
                    .find(|overlay| *overlay != VectorOverlay::Off)
                    .unwrap_or(VectorOverlay::Off)
            } else {
                VectorOverlay::Off
            };
            presentation.material_overlay = if on {
                MaterialOverlay::Property(MaterialProperty::Density)
            } else {
                MaterialOverlay::Off
            };
            Stage::Rest
        }
        (Act::Coarser, Stage::Take) => {
            state.editor.document.presentation.mesh_edge = 1.25 * driver.edge;
            state.coordinator.remesh_requested = true;
            Stage::Rest
        }
        (Act::Snapshot, Stage::Take) => {
            let seen = display.full_readbacks;
            request.request_full_state_readback();
            Stage::Snapshot(seen)
        }
        (Act::Snapshot, Stage::Snapshot(seen)) => {
            if display.full_readbacks > seen && display.generation == request.generation() {
                return Ok(driver.next(display.primary_flux.len()));
            }
            request.request_full_state_readback();
            Stage::Snapshot(seen)
        }
        (Act::Reset, Stage::Take) => {
            state.coordinator.reset_requested = true;
            Stage::Rest
        }
        (Act::Fault, Stage::Take) => {
            if !at_rest(state, request, display) {
                return Ok(true);
            }
            let nodes = request
                .running_node_count()
                .ok_or("no generation runs to fault")?;
            request
                .inject_failure(
                    assets,
                    commands,
                    crate::canonical_gpu::CANONICAL_FAILURE_NON_FINITE,
                    nodes as u32 / 2,
                )
                .map_err(|error| format!("the fault was not armed: {error}"))?;
            Stage::Failing
        }
        (Act::Fault, Stage::Failing) => {
            if !(request.failed() && state.coordinator.solver_fault.is_some()) {
                return Ok(true);
            }
            // The notice's OK, and Run.
            state.notices.clear();
            state.wave_running = true;
            Stage::Rest
        }
        (Act::RefusedHandoff, Stage::Take) => {
            if !at_rest(state, request, display) {
                return Ok(true);
            }
            state.e2e_steps.fail_handoff = Some(7);
            edit(state)?;
            Stage::Refusing
        }
        // The first handoff takes the failure, the edit's or, adaptation on,
        // one of its own.
        (Act::RefusedHandoff, Stage::Refusing) => {
            if state.e2e_steps.fail_handoff.is_some() && at_rest(state, request, display) {
                return Err("the edit landed with no handoff to refuse".into());
            }
            if state.e2e_steps.fail_handoff.is_some() || waiting(state, request).is_some() {
                return Ok(true);
            }
            state.coordinator.remesh_requested = true;
            Stage::Rest
        }
        (Act::Count, Stage::Take) => {
            let presentation = &mut state.editor.document.presentation;
            presentation.adaptation.enabled = false;
            presentation.mesh_edge = driver.edge;
            state.coordinator.remesh_requested = true;
            Stage::Rest
        }
        (Act::Count, Stage::Settling { waited, still }) => {
            if !at_rest(state, request, display) {
                return Err("the paused run took up work while it was counted".into());
            }
            let now = (
                main.clone(),
                RenderCounts {
                    frame: 0,
                    copies_in_flight: 0,
                    ..render.clone()
                },
            );
            let still = if driver.last.as_ref() == Some(&now) {
                still + 1
            } else {
                0
            };
            driver.last = Some(now);
            if waited < SETTLE_FRAMES || still < STILL_FRAMES {
                Stage::Settling {
                    waited: waited + 1,
                    still,
                }
            } else {
                let counted = Counted {
                    cycle: driver.cycle,
                    scene: driver.scene(),
                    nodes: display.primary_flux.len(),
                    seconds: driver.opened.elapsed().as_secs_f64(),
                    main,
                    render,
                };
                report(&format!("e2e lifecycle: {}", counted.line()));
                driver.counted.push(counted);
                driver.last = None;
                return Ok(driver.next(display.primary_flux.len()));
            }
        }
        (_, Stage::Rest) => {
            if !at_rest(state, request, display) {
                return Ok(true);
            }
            match act {
                Act::Count => {
                    state.wave_running = false;
                    Stage::Settling {
                        waited: 0,
                        still: 0,
                    }
                }
                _ => return Ok(driver.next(display.primary_flux.len())),
            }
        }
        (act, stage) => unreachable!("{act:?} has no {stage:?}"),
    };
    driver.stage = stage;
    Ok(true)
}

/// Toggles the first material's density, as its editor does.
fn edit(state: &mut Playground) -> Result<(), String> {
    let mut material = state
        .editor
        .document
        .model
        .draft
        .materials
        .first()
        .cloned()
        .ok_or("the scene has no material")?;
    let dense = material.mass_density == ScalarField::constant(1.0);
    material.mass_density = ScalarField::constant(if dense { 1.5 } else { 1.0 });
    state
        .editor
        .update_material(material)
        .map_err(|error| format!("the edit was refused: {error:?}"))
}

/// Every holder made, and every count as the cycle before left it.
fn judge(driver: &LifecycleDriver) -> Result<String, String> {
    let mut failures = Vec::new();
    let missing: Vec<&str> = HOLDERS
        .iter()
        .copied()
        .filter(|holder| !driver.holders.contains(holder))
        .collect();
    if !missing.is_empty() {
        failures.push(format!(
            "no scene made the {} bind groups",
            missing.join(", ")
        ));
    }
    for counted in &driver.counted {
        let broken = held(counted);
        if !broken.is_empty() {
            failures.push(format!(
                "cycle {}, {}: {}",
                counted.cycle,
                counted.scene,
                broken.join(", ")
            ));
        }
        let Some(before) = driver
            .counted
            .iter()
            .find(|before| before.cycle + 1 == counted.cycle && before.scene == counted.scene)
        else {
            continue;
        };
        let changed = differences(before, counted);
        if changed.is_empty() {
            continue;
        }
        failures.push(format!(
            "{}, cycle {} against {}: {}",
            counted.scene,
            counted.cycle,
            before.cycle,
            changed.join(", ")
        ));
    }
    if failures.is_empty() {
        Ok(format!(
            "{} scenes, {CYCLES} cycles, every bind group made, every count as the cycle \
             before left it",
            SCENES.len()
        ))
    } else {
        Err(failures.join("; "))
    }
}

/// What must hold of any count, whatever came before it: as many shader
/// buffers on the device as in the main world, no more staging than the
/// readbacks can have in flight, no handoff's or live event's bind groups
/// once both are done, one generation alive, and no work in hand.
fn held(counted: &Counted) -> Vec<String> {
    let (main, render) = (&counted.main, &counted.render);
    let mut broken = Vec::new();
    if render.device_buffers.count != main.shader_buffers {
        broken.push(format!(
            "{} shader buffers on the device for {} in the main world",
            render.device_buffers.count, main.shader_buffers
        ));
    }
    let depth = main.readbacks * usize::from(crate::paced_readback::IN_FLIGHT_DEPTH);
    if render.readback_staging.count > depth {
        broken.push(format!(
            "{} staging buffers, where {} readbacks have {depth} in flight at most",
            render.readback_staging.count, main.readbacks
        ));
    }
    for (holder, _) in &render.bind_groups {
        if matches!(*holder, "handoff" | "live event") {
            broken.push(format!("the {holder} bind groups kept"));
        }
    }
    let generations = main.generations;
    if generations.meshes != 1 || generations.operators != 1 || generations.temporal_operators > 1 {
        broken.push(format!(
            "{} meshes, {} operators and {} time-driven operators alive",
            generations.meshes, generations.operators, generations.temporal_operators
        ));
    }
    if !main.jobs.is_empty() {
        broken.push(format!("still held {}", main.jobs.join(", ")));
    }
    broken
}

/// What differs between two counts of a scene.
fn differences(before: &Counted, after: &Counted) -> Vec<String> {
    let (main_before, render_before) = before.compared();
    let (main_after, render_after) = after.compared();
    let mut changed = Vec::new();
    let mut count = |what: &str, before: String, after: String| {
        if before != after {
            changed.push(format!("{what} {before} to {after}"));
        }
    };
    let tally = |tally: Tally| tally.to_string();
    count(
        "shader buffers",
        main_before.shader_buffers.to_string(),
        main_after.shader_buffers.to_string(),
    );
    count(
        "readbacks",
        main_before.readbacks.to_string(),
        main_after.readbacks.to_string(),
    );
    count(
        "main entities",
        main_before.entities.to_string(),
        main_after.entities.to_string(),
    );
    count(
        "threads",
        main_before.threads.to_string(),
        main_after.threads.to_string(),
    );
    count(
        "generations alive",
        format!("{:?}", main_before.generations),
        format!("{:?}", main_after.generations),
    );
    count(
        "device buffers",
        tally(render_before.device_buffers),
        tally(render_after.device_buffers),
    );
    count(
        "readback staging",
        tally(render_before.readback_staging),
        tally(render_after.readback_staging),
    );
    count(
        "snapshot staging",
        tally(render_before.snapshot_staging),
        tally(render_after.snapshot_staging),
    );
    count(
        "paint buffers",
        tally(render_before.paint_buffers),
        tally(render_after.paint_buffers),
    );
    count(
        "bind groups",
        format!("{:?}", render_before.bind_groups),
        format!("{:?}", render_after.bind_groups),
    );
    count(
        "pipelines",
        render_before.pipelines.to_string(),
        render_after.pipelines.to_string(),
    );
    count(
        "render entities",
        render_before.entities.to_string(),
        render_after.entities.to_string(),
    );
    changed
}
