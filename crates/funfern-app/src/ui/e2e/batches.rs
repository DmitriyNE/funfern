//! RT-02: the same steps in any schedule are the same bits, on the gallery's
//! scenes. `batches` opens each scene as the gallery does, adaptation off,
//! and runs it [`BATCH_STEPS`] steps under each of [`SCHEDULES`]: the app's
//! frames at four batch sizes, and the whole run asked for in one frame, as
//! the device examples ask, which the render world encodes under its lead
//! fence or, unfenced, under its per-frame ceiling alone. Each run starts
//! [`STEPS_BEFORE_REBASE`] steps before its epoch rebases, so the rebase
//! falls inside a batch. At the endpoint the accepted state, the clock, the
//! accounting, the event words and every recorder's ring must be the same
//! bits in every schedule: the device keeps its clock and control on the GPU,
//! takes no host write while stepping, and books and samples by the step, so
//! how the steps were grouped into submissions has no arithmetic to change.
//! A schedule must also meet what it names - the frames asked for its batch,
//! the fence held a burst back, the ceiling split one - or it compares a
//! schedule with another.
//!
//! The scenes are `FUNFERN_E2E_SCENES`, gallery names separated by `;` or
//! `all`, and [`SCENES`] where it is not set; those must take every per-step
//! branch the encoder has between them. A name ending in [`DRIVEN`] is the
//! scene with its top side set to a driven Dirichlet law, as the inspector's
//! picker sets it: no gallery scene drives a side beside an outgoing one,
//! whose trace then holds prescribed nodes. A name may end in `@` and a
//! number of steps to run in place of [`BATCH_STEPS`].

use super::*;
use crate::wave_gpu::{
    AreaProbeDisplay, CurveProbeDisplay, FarFieldDisplay, ProbeDisplay, WaveGpuRequest,
};

/// Steps a run takes unless its scene names more: past one per-frame
/// ceiling, so that the unfenced burst encodes 256 steps and then the rest.
const BATCH_STEPS: u64 = 320;

/// Steps before the epoch rebases that every run starts at.
const STEPS_BEFORE_REBASE: u32 = 100;

/// The gallery scenes the fixture runs unless told otherwise, which between
/// them take every branch in [`BRANCHES`], measured on 2026-11-06: the
/// soliton the field laws, the nonlinear walls and the integrated field; the
/// Josephson line the fourth-order kick form and the losses; the emitter the
/// short-wave split; the ring resonator the direct wall solve and the area
/// probes; and the double slit, its top side driven, the prescribed walls
/// and the far field, whose first frames wait for its contour's history to
/// span the retarded times: it runs longer.
const SCENES: &[(&str, u64)] = &[
    ("Spatial soliton", BATCH_STEPS),
    ("Josephson line", BATCH_STEPS),
    ("Self-sustained emitter", BATCH_STEPS),
    ("Ring resonator", BATCH_STEPS),
    ("Double slit+driven", FAR_STEPS),
];

/// Steps the far field's scene runs.
const FAR_STEPS: u64 = 640;

/// The suffix that names a scene with its top side driven.
const DRIVEN: &str = "+driven";

/// How the steps are asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Schedule {
    /// This many steps a frame in place of the frame's pacing, under the
    /// frame's ceiling and its backpressure.
    Frames(u64),
    /// Every step asked for in one frame, encoded under the render world's
    /// lead fence where `fenced`, and under its per-frame ceiling alone where
    /// not.
    Burst { fenced: bool },
}

impl std::fmt::Display for Schedule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Schedule::Frames(steps) => write!(f, "{steps} a frame"),
            Schedule::Burst { fenced: true } => f.write_str("at once, fenced"),
            Schedule::Burst { fenced: false } => f.write_str("at once, unfenced"),
        }
    }
}

/// The schedules each scene runs under, the first the one the others are
/// read against.
const SCHEDULES: [Schedule; 6] = [
    Schedule::Frames(1),
    Schedule::Frames(7),
    Schedule::Frames(32),
    Schedule::Frames(64),
    Schedule::Burst { fenced: true },
    Schedule::Burst { fenced: false },
];

/// Readbacks of each recorder's ring to see past the endpoint's snapshot
/// before its ring is read: the copies in flight when it arrived, and one
/// taken after.
const RING_READBACKS: u64 = 4;

#[derive(Resource)]
pub(super) struct BatchDriver {
    /// Each scene still to run, the current one first.
    scenes: VecDeque<Scene>,
    /// Whether the scenes are the fixture's own, which must cover every
    /// branch.
    own_scenes: bool,
    /// The current run's schedule, by its index in [`SCHEDULES`].
    schedule: usize,
    phase: BatchPhase,
    /// The current scene's finished runs.
    endpoints: Vec<Endpoint>,
    /// The device's completed-step count when the run's generation was
    /// installed.
    base: u64,
    /// The branches the scenes run so far took, and the scenes that took
    /// each.
    covered: BTreeMap<&'static str, Vec<String>>,
    /// Each scene's verdict, a failure or what held.
    verdicts: Vec<Result<String, String>>,
    /// When the current scene's first run began.
    started: Instant,
    /// How long a scene's six runs may take, and when the current scene's
    /// time is up.
    per_scene: std::time::Duration,
    deadline: Instant,
}

/// A gallery scene, by its catalogue index, whether its top side is driven,
/// and the steps it runs.
#[derive(Clone)]
struct Scene {
    index: usize,
    name: String,
    driven: bool,
    steps: u64,
}

enum BatchPhase {
    /// The scene opens for the next run.
    Open,
    /// Its generation is being prepared and installed.
    Install,
    /// The run steps to its endpoint.
    Run,
    /// A full snapshot of the endpoint is asked for.
    Snapshot {
        seen: u64,
    },
    /// The recorders' rings are read once copies taken after the endpoint
    /// have arrived.
    Rings {
        endpoint: Box<Endpoint>,
        seen: [u64; 4],
    },
    Done,
}

impl BatchPhase {
    fn name(&self) -> &'static str {
        match self {
            BatchPhase::Open => "opening the scene",
            BatchPhase::Install => "installing its generation",
            BatchPhase::Run => "running to the endpoint",
            BatchPhase::Snapshot { .. } => "waiting for the endpoint's snapshot",
            BatchPhase::Rings { .. } => "waiting for the recorders' rings",
            BatchPhase::Done => "done",
        }
    }
}

/// What a run ended on, part by part, as hashes of the parts' bits, the
/// branches it took, and the records each recorder's ring held.
struct Endpoint {
    parts: Vec<(&'static str, u64)>,
    branches: Vec<&'static str>,
    records: Vec<(&'static str, usize)>,
}

/// Adds the batching fixture's driver.
pub(super) fn add(app: &mut App, seconds: Option<u64>) {
    let (scenes, own_scenes) = match scenes() {
        Ok(scenes) => scenes,
        Err(failure) => {
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
    app.insert_resource(BatchDriver {
        scenes,
        own_scenes,
        schedule: 0,
        phase: BatchPhase::Open,
        endpoints: Vec::new(),
        base: 0,
        covered: BTreeMap::new(),
        verdicts: Vec::new(),
        started: Instant::now(),
        per_scene: std::time::Duration::from_secs(seconds.unwrap_or(120)),
        deadline: Instant::now() + std::time::Duration::from_secs(seconds.unwrap_or(120)),
    })
    .add_systems(EguiPrimaryContextPass, drive_batches.before(frame));
}

/// The scenes asked for, and whether they are the fixture's own.
fn scenes() -> Result<(VecDeque<Scene>, bool), String> {
    #[cfg(not(target_arch = "wasm32"))]
    let asked = std::env::var("FUNFERN_E2E_SCENES").ok();
    #[cfg(target_arch = "wasm32")]
    let asked: Option<String> = None;
    let catalog = crate::topology_examples::catalog();
    let names: Vec<(&str, u64)> = match asked.as_deref() {
        None => SCENES.to_vec(),
        Some("all") => catalog
            .iter()
            .map(|example| (example.name, BATCH_STEPS))
            .collect(),
        Some(names) => names
            .split(';')
            .map(|name| match name.trim().rsplit_once('@') {
                Some((name, steps)) => (name, steps.parse().unwrap_or(0)),
                None => (name.trim(), BATCH_STEPS),
            })
            .collect(),
    };
    let scenes = names
        .into_iter()
        .map(|(name, steps)| {
            if steps <= crate::canonical_gpu::MAX_STEPS_PER_FRAME {
                return Err(format!(
                    "{name} runs {steps} steps, which one frame's ceiling holds"
                ));
            }
            let (shipped, driven) = match name.strip_suffix(DRIVEN) {
                Some(shipped) => (shipped, true),
                None => (name, false),
            };
            catalog
                .iter()
                .position(|example| example.name == shipped)
                .map(|index| Scene {
                    index,
                    name: name.to_owned(),
                    driven,
                    steps,
                })
                .ok_or_else(|| format!("no gallery scene named {shipped:?}"))
        })
        .collect::<Result<_, _>>()?;
    Ok((scenes, asked.is_none()))
}

#[allow(clippy::too_many_arguments)]
fn drive_batches(
    mut driver: ResMut<BatchDriver>,
    mut state: ResMut<Playground>,
    mut request: ResMut<CanonicalGpuRequest>,
    display: Res<CanonicalGpuDisplay>,
    recorders: Res<WaveGpuRequest>,
    rings: (
        Res<ProbeDisplay>,
        Res<CurveProbeDisplay>,
        Res<AreaProbeDisplay>,
        Res<FarFieldDisplay>,
    ),
    mut exit: MessageWriter<AppExit>,
) {
    if matches!(driver.phase, BatchPhase::Done) {
        return;
    }
    let run = match advance(
        &mut driver,
        &mut state,
        &mut request,
        &display,
        &recorders,
        &rings,
    ) {
        Some(run) => run,
        None if Instant::now() >= driver.deadline => Err(format!(
            "did not finish in time, at {}",
            driver.phase.name()
        )),
        None => return,
    };
    let scene = driver
        .scenes
        .front()
        .map_or_else(String::new, |scene| scene.name.clone());
    let schedule = SCHEDULES[driver.schedule];
    let verdict = match run {
        Ok(endpoint) => {
            report(&format!(
                "e2e batches: {scene}, {schedule}: {}",
                endpoint
                    .parts
                    .iter()
                    .map(|(name, hash)| format!("{name} {hash:016x}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            for branch in &endpoint.branches {
                let scenes = driver.covered.entry(branch).or_default();
                if !scenes.contains(&scene) {
                    scenes.push(scene.clone());
                }
            }
            driver.endpoints.push(endpoint);
            if driver.schedule + 1 < SCHEDULES.len() {
                driver.schedule += 1;
                driver.phase = BatchPhase::Open;
                return;
            }
            let nodes = display.primary_flux.len();
            let seconds = driver.started.elapsed().as_secs_f64();
            let records: String = driver.endpoints[0]
                .records
                .iter()
                .map(|(name, records)| format!(", {name} {records} records"))
                .collect();
            same_bits(&driver.endpoints)
                .map(|held| format!("{scene}: {nodes} nodes{records}, {seconds:.1} s, {held}"))
                .map_err(|failure| format!("{scene}: {failure}"))
        }
        // A run that failed, or ran out of its scene's time, ends its scene.
        Err(failure) => Err(format!("{scene}, {schedule}: {failure}")),
    };
    match &verdict {
        Ok(held) => report(&format!("e2e batches: {held}")),
        Err(failure) => report(&format!("e2e batches: FAIL {failure}")),
    }
    driver.verdicts.push(verdict);
    driver.scenes.pop_front();
    driver.endpoints.clear();
    driver.schedule = 0;
    driver.started = Instant::now();
    driver.deadline = driver.started + driver.per_scene;
    if !driver.scenes.is_empty() {
        driver.phase = BatchPhase::Open;
        return;
    }
    driver.phase = BatchPhase::Done;
    *FRESH_CLOCK.lock().unwrap() = None;
    for (branch, scenes) in &driver.covered {
        report(&format!("e2e batches: {branch}: {}", scenes.join(", ")));
    }
    let mut failures: Vec<String> = driver
        .verdicts
        .iter()
        .filter_map(|verdict| verdict.as_ref().err().cloned())
        .collect();
    if driver.own_scenes {
        let missing: Vec<&str> = BRANCHES
            .iter()
            .copied()
            .filter(|branch| !driver.covered.contains_key(branch))
            .collect();
        if !missing.is_empty() {
            failures.push(format!("no scene took {}", missing.join(", ")));
        }
    }
    let summary = if failures.is_empty() {
        format!(
            "{} scenes, {} schedules each, the same bits",
            driver.verdicts.len(),
            SCHEDULES.len()
        )
    } else {
        failures.join("; ")
    };
    if failures.is_empty() {
        report(&format!("e2e batches: {summary}"));
        mark("pass", &summary);
        exit.write(AppExit::Success);
    } else {
        report(&format!("e2e batches FAILED: {summary}"));
        mark("fail", &summary);
        exit.write(AppExit::error());
    }
}

/// Every branch the scenes must take between them: the encoder's, the
/// rebase, and each recorder. Walls stepped by half kicks are not among
/// them: the app steps every wall by the trapezoid rule, and the half kicks
/// stay for the examples' comparisons.
const BRANCHES: &[&str] = &[
    "losses",
    "accounting",
    "material runtime",
    "integrated field",
    "sample secants",
    "field laws",
    "short-wave split",
    "trapezoid walls",
    "direct wall solve",
    "swept wall solve",
    "nonlinear walls",
    "prescribed walls",
    "fourth-order drift",
    "fourth-order kick",
    "grid filter",
    "rebase",
    "point probes",
    "line probes",
    "area probes",
    "far field",
];

fn advance(
    driver: &mut BatchDriver,
    state: &mut Playground,
    request: &mut CanonicalGpuRequest,
    display: &CanonicalGpuDisplay,
    recorders: &WaveGpuRequest,
    rings: &(
        Res<ProbeDisplay>,
        Res<CurveProbeDisplay>,
        Res<AreaProbeDisplay>,
        Res<FarFieldDisplay>,
    ),
) -> Option<Result<Endpoint, String>> {
    let schedule = SCHEDULES[driver.schedule];
    match std::mem::replace(&mut driver.phase, BatchPhase::Done) {
        BatchPhase::Open => {
            if !state.startup_done {
                driver.phase = BatchPhase::Open;
                return None;
            }
            let scene = driver.scenes.front()?.clone();
            state.e2e_steps.limit = Some(0);
            state.e2e_steps.batch = None;
            state.e2e_steps.burst = false;
            state.wave_running = true;
            *FRESH_CLOCK.lock().unwrap() = Some((FreshOrigin::StartingAtZero, STEPS_BEFORE_REBASE));
            let mut document = crate::topology_examples::catalog()[scene.index]
                .document
                .clone();
            document.presentation.adaptation.enabled = false;
            if scene.driven {
                for model in [&mut document.model.draft, &mut document.model.accepted] {
                    model.outer_boundaries.sides[OuterSide::Top.index()] =
                        OuterBoundaryCondition::Dirichlet { signal: WALL_DRIVE };
                }
            }
            if let Err(error) = state.set_document(document, false, true) {
                return Some(Err(format!("the scene did not open: {error}")));
            }
            driver.phase = BatchPhase::Install;
            None
        }
        BatchPhase::Install => {
            let installed = state.runtime.active().is_some_and(|active| {
                active.bundle.token.document_revision == state.editor.revision
                    && *active.bundle.authored == state.editor.document.model.accepted
                    && state.coordinator.uploading.is_none()
                    && request.generation() != 0
                    && display.generation == request.generation()
            });
            if !installed {
                driver.phase = BatchPhase::Install;
                return None;
            }
            request.set_unfenced_stepping(matches!(schedule, Schedule::Burst { fenced: false }));
            driver.base = request.stats().completed_steps();
            state.wave_running = true;
            let steps = driver.scenes.front()?.steps;
            state.e2e_steps.limit = Some(driver.base + steps);
            state.e2e_steps.batch = match schedule {
                Schedule::Frames(steps) => Some(steps),
                Schedule::Burst { .. } => None,
            };
            state.e2e_steps.burst = matches!(schedule, Schedule::Burst { .. });
            state.e2e_steps.most = 0;
            driver.phase = BatchPhase::Run;
            None
        }
        BatchPhase::Run => {
            let completed = request
                .stats()
                .completed_steps()
                .saturating_sub(driver.base);
            let code = request.stats().failure();
            if code != 0 {
                return Some(Err(format!(
                    "the device failed after step {completed} (failure code {code}: {})",
                    crate::canonical_gpu::canonical_failure_description(code)
                )));
            }
            let steps = driver.scenes.front()?.steps;
            if completed < steps {
                driver.phase = BatchPhase::Run;
                return None;
            }
            if completed > steps {
                return Some(Err(format!(
                    "the run went past its endpoint at {steps}, to {completed}"
                )));
            }
            if let Err(failure) = met(schedule, state.e2e_steps.most, request) {
                return Some(Err(failure));
            }
            let seen = display.full_readbacks;
            request.request_full_state_readback();
            driver.phase = BatchPhase::Snapshot { seen };
            None
        }
        BatchPhase::Snapshot { seen } => {
            let endpoint = driver.base + driver.scenes.front()?.steps;
            let arrived = display.full_readbacks > seen
                && display.generation == request.generation()
                && display.full_snapshot_completed_steps() == endpoint
                && display
                    .clock
                    .is_some_and(|clock| clock.accepted_steps == endpoint);
            if !arrived {
                request.request_full_state_readback();
                driver.phase = BatchPhase::Snapshot { seen };
                return None;
            }
            let Some(clock) = display.clock else {
                unreachable!("the snapshot's arrival read the clock");
            };
            let mut branches = request.step_branches();
            if clock.epoch > 0 {
                branches.push("rebase");
            }
            let words = |values: &[f32]| values.iter().map(|value| value.to_bits()).collect();
            let clock_words = [
                clock.epoch,
                clock.epoch_origin_seconds.to_bits(),
                clock.absolute_seconds.to_bits(),
                u64::from(clock.step_in_epoch),
                clock.accepted_steps,
                u64::from(clock.local_seconds.to_bits()),
                u64::from(clock.time_step.to_bits()),
                u64::from(clock.event_serial),
            ]
            .iter()
            .flat_map(|word| [*word as u32, (*word >> 32) as u32])
            .collect::<Vec<_>>();
            let mut accounting: Vec<u32> = words(&display.accounting);
            accounting.push(display.active_gain.to_bits());
            let events: Vec<u32> = display
                .event_result
                .iter()
                .chain(&display.runtime_serials)
                .copied()
                .chain([request.stats().processed_event()])
                .collect();
            let endpoint = Endpoint {
                parts: vec![
                    ("state", hash(&display.accepted_storage_bits())),
                    ("material runtime", hash(&display.material_runtime_bits())),
                    ("force cache", hash(&words(&display.constitutive_force))),
                    ("clock", hash(&clock_words)),
                    ("accounting", hash(&accounting)),
                    ("events", hash(&events)),
                ],
                branches,
                records: Vec::new(),
            };
            driver.phase = BatchPhase::Rings {
                endpoint: Box::new(endpoint),
                seen: ring_readbacks(rings),
            };
            None
        }
        BatchPhase::Rings { mut endpoint, seen } => {
            let now = ring_readbacks(rings);
            let configured = [
                recorders.probes.is_some(),
                recorders.curve_probes.is_some(),
                recorders.area_probes.is_some(),
                recorders.far_field.is_some(),
            ];
            let generations = [
                rings.0.generation,
                rings.1.generation,
                rings.2.generation,
                rings.3.generation,
            ];
            let waiting = (0..4).any(|ring| {
                configured[ring]
                    && (now[ring] < seen[ring] + RING_READBACKS
                        || generations[ring] != request.generation())
            });
            if waiting {
                driver.phase = BatchPhase::Rings { endpoint, seen };
                return None;
            }
            let mut point = Vec::new();
            for record in &rings.0.records {
                point.extend(
                    [
                        record.probe_id,
                        record.time.to_bits(),
                        record.displacement.to_bits(),
                        record.velocity.to_bits(),
                        record.transverse_magnitude.to_bits(),
                        record.poynting_magnitude.to_bits(),
                        record.energy_density.to_bits(),
                    ]
                    .iter()
                    .flat_map(|word| [*word as u32, (*word >> 32) as u32]),
                );
            }
            let mut line = Vec::new();
            for record in &rings.1.records {
                line.extend(
                    [record.probe_id, record.time.to_bits()]
                        .iter()
                        .flat_map(|word| [*word as u32, (*word >> 32) as u32]),
                );
                for values in [
                    &record.displacement,
                    &record.transverse_magnitude,
                    &record.energy_density,
                    &record.normal_flux,
                ] {
                    line.extend(values.iter().map(|value| value.to_bits()));
                }
            }
            let mut area = Vec::new();
            for record in &rings.2.records {
                area.extend(
                    [
                        record.probe_id,
                        record.time.to_bits(),
                        record.mean_displacement.to_bits(),
                        record.rms_displacement.to_bits(),
                        record.rms_transverse_magnitude.to_bits(),
                        record.mean_energy_density.to_bits(),
                        record.total_energy.to_bits(),
                        record.covered_area.to_bits(),
                        record.coverage.to_bits(),
                    ]
                    .iter()
                    .flat_map(|word| [*word as u32, (*word >> 32) as u32]),
                );
            }
            let mut far = Vec::new();
            for record in &rings.3.records {
                let time = record.time.to_bits();
                far.extend([time as u32, (time >> 32) as u32]);
                far.extend(record.amplitude.iter().map(|value| value.to_bits()));
                far.extend(record.intensity.iter().map(|value| value.to_bits()));
            }
            // A recorder counts as a branch taken only where its ring holds
            // records to compare: the far field's first frames wait for its
            // contour's history to span the retarded times. A display keeps
            // the last recorder's records where the scene has none.
            for (name, words, records, configured) in [
                ("point probes", point, rings.0.records.len(), configured[0]),
                ("line probes", line, rings.1.records.len(), configured[1]),
                ("area probes", area, rings.2.records.len(), configured[2]),
                ("far field", far, rings.3.records.len(), configured[3]),
            ] {
                if configured {
                    endpoint.parts.push((name, hash(&words)));
                    endpoint.records.push((name, records));
                    if records > 0 {
                        endpoint.branches.push(name);
                    }
                }
            }
            Some(Ok(*endpoint))
        }
        BatchPhase::Done => None,
    }
}

/// Whether the run met the schedule it names: its frames asked for its
/// batch, the lead fence held its burst back, or the per-frame ceiling split
/// it. The app's own frames never reach the fence: their backpressure keeps
/// what is asked within the same lead of what the queue has retired.
fn met(schedule: Schedule, most: u64, request: &CanonicalGpuRequest) -> Result<(), String> {
    let (encoded, fenced) = request.stats().encoding_bounds();
    let ceiling = crate::canonical_gpu::MAX_STEPS_PER_FRAME;
    let lead = crate::canonical_gpu::MAX_ENCODED_STEP_LEAD;
    match schedule {
        Schedule::Frames(steps) if most != steps => Err(format!(
            "asked for {steps} a frame, the frames asked for at most {most}"
        )),
        Schedule::Frames(_) if fenced != 0 => Err(format!(
            "the lead fence held back {fenced} of the app's own frames"
        )),
        Schedule::Burst { fenced: true } if fenced == 0 || encoded > lead => Err(format!(
            "the fenced burst encoded up to {encoded} a frame, the fence held back {fenced}"
        )),
        Schedule::Burst { fenced: false } if encoded != ceiling || fenced != 0 => Err(format!(
            "the unfenced burst encoded up to {encoded} a frame, the fence held back {fenced}"
        )),
        _ => Ok(()),
    }
}

fn ring_readbacks(
    rings: &(
        Res<ProbeDisplay>,
        Res<CurveProbeDisplay>,
        Res<AreaProbeDisplay>,
        Res<FarFieldDisplay>,
    ),
) -> [u64; 4] {
    [
        rings.0.readbacks,
        rings.1.readbacks,
        rings.2.readbacks,
        rings.3.readbacks,
    ]
}

/// Whether every run ended on the first's bits, part by part.
fn same_bits(endpoints: &[Endpoint]) -> Result<String, String> {
    let [first, ..] = endpoints else {
        return Err("no run finished".to_owned());
    };
    let mut parted = Vec::new();
    for (schedule, endpoint) in SCHEDULES.iter().zip(endpoints).skip(1) {
        if endpoint.branches != first.branches {
            parted.push(format!("{schedule} took other branches"));
        }
        let names = |endpoint: &Endpoint| {
            endpoint
                .parts
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
        };
        if names(endpoint) != names(first) {
            parted.push(format!("{schedule} read other parts"));
            continue;
        }
        let apart: Vec<&str> = endpoint
            .parts
            .iter()
            .zip(&first.parts)
            .filter(|((_, hash), (_, first))| hash != first)
            .map(|((name, _), _)| *name)
            .collect();
        if !apart.is_empty() {
            parted.push(format!("{schedule}: {} apart", apart.join(", ")));
        }
    }
    if parted.is_empty() {
        Ok(format!(
            "the same bits in all {} schedules ({}; {})",
            endpoints.len(),
            first
                .parts
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(", "),
            first.branches.join(", ")
        ))
    } else {
        Err(format!("against {}: {}", SCHEDULES[0], parted.join("; ")))
    }
}
