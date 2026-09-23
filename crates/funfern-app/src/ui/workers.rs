//! Work that runs off the UI thread: solver preparation, the AMR indicator,
//! and the GPU upload compile, with the thread and worker-message plumbing
//! each target needs.
#[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
use super::BROWSER_BACKGROUND_POOL_READY;
use super::PreparedGpuUpload;
use crate::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuPlan, CanonicalGpuRuntimeTransfer, CanonicalGpuTransferPlan,
};
#[cfg(any(
    not(target_arch = "wasm32"),
    all(target_arch = "wasm32", feature = "browser-threads")
))]
use bevy::platform::time::Instant;
use funfern_app::topology_runtime::{
    PreparedTopology, TopologyPreparationError, TopologyPreparationJob, TopologyPreparationPhase,
    TopologyPreparationTiming, TopologyToken,
};
use funfern_core::*;
#[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
use std::sync::atomic::Ordering;
use std::sync::{
    Arc, Mutex,
    mpsc::{self, Receiver, Sender},
};

#[cfg_attr(
    all(target_arch = "wasm32", not(feature = "browser-threads")),
    allow(dead_code)
)]
pub(super) enum BackgroundPreparationEvent {
    WorkerStarted,
    Progress {
        token: TopologyToken,
        phase: TopologyPreparationPhase,
        detail: &'static str,
        timing: TopologyPreparationTiming,
    },
    Finished {
        token: TopologyToken,
        result: Box<Result<PreparedTopology, TopologyPreparationError>>,
    },
}

/// One long-lived worker owns CPU candidate preparation. New jobs queue through
/// one channel; between bounded quanta the worker drains that queue to its newest
/// member, so rapid edits cannot create a growing set of competing assembly
/// threads. Native uses an OS thread and an isolated browser uses one shared-
/// memory Web Worker in the threaded browser bundle. The static-host bundle and
/// browser worker bootstrap failure retain the cooperative main-thread runner.
pub(super) struct BackgroundPreparationWorker {
    pub(super) sender: Sender<TopologyPreparationJob>,
    pub(super) receiver: Mutex<Receiver<BackgroundPreparationEvent>>,
    pub(super) token: Option<TopologyToken>,
    pub(super) phase: Option<TopologyPreparationPhase>,
    pub(super) detail: Option<&'static str>,
    pub(super) timing: Option<TopologyPreparationTiming>,
}

impl BackgroundPreparationWorker {
    #[cfg(any(
        not(target_arch = "wasm32"),
        all(target_arch = "wasm32", feature = "browser-threads")
    ))]
    const QUANTUM: std::time::Duration = std::time::Duration::from_millis(8);

    pub(super) fn spawn() -> Option<Self> {
        let (job_sender, job_receiver) = mpsc::channel::<TopologyPreparationJob>();
        let (event_sender, event_receiver) = mpsc::channel::<BackgroundPreparationEvent>();
        if !spawn_preparation_worker(job_receiver, event_sender) {
            return None;
        }
        Some(Self {
            sender: job_sender,
            receiver: Mutex::new(event_receiver),
            token: None,
            phase: None,
            detail: None,
            timing: None,
        })
    }

    pub(super) fn submit(
        &mut self,
        job: TopologyPreparationJob,
    ) -> Option<Box<TopologyPreparationJob>> {
        self.token = Some(job.token());
        self.phase = Some(job.phase());
        self.detail = Some(job.detail());
        self.timing = Some(job.timing());
        self.sender.send(job).err().map(|error| Box::new(error.0))
    }

    pub(super) fn drain(&self) -> Vec<BackgroundPreparationEvent> {
        let receiver = self.receiver.lock().unwrap();
        std::iter::from_fn(|| receiver.try_recv().ok()).collect()
    }

    pub(super) fn observe(&mut self, event: &BackgroundPreparationEvent) {
        match event {
            BackgroundPreparationEvent::WorkerStarted => {}
            BackgroundPreparationEvent::Progress {
                token,
                phase,
                detail,
                timing,
            } if self.token == Some(*token) => {
                self.phase = Some(*phase);
                self.detail = Some(*detail);
                self.timing = Some(*timing);
            }
            BackgroundPreparationEvent::Finished { token, .. } if self.token == Some(*token) => {
                self.token = None;
                self.phase = None;
                self.detail = None;
                self.timing = None;
            }
            _ => {}
        }
    }
}

#[cfg(any(
    not(target_arch = "wasm32"),
    all(target_arch = "wasm32", feature = "browser-threads")
))]
pub(super) fn run_preparation_worker(
    job_receiver: Receiver<TopologyPreparationJob>,
    event_sender: Sender<BackgroundPreparationEvent>,
) {
    if event_sender
        .send(BackgroundPreparationEvent::WorkerStarted)
        .is_err()
    {
        return;
    }
    while let Ok(mut job) = job_receiver.recv() {
        loop {
            loop {
                match job_receiver.try_recv() {
                    Ok(newer) => job = newer,
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                }
            }
            let token = job.token();
            let result = job.advance_for(BackgroundPreparationWorker::QUANTUM);
            if let Some(result) = result {
                if event_sender
                    .send(BackgroundPreparationEvent::Finished {
                        token,
                        result: Box::new(result),
                    })
                    .is_err()
                {
                    return;
                }
                break;
            }
            if event_sender
                .send(BackgroundPreparationEvent::Progress {
                    token,
                    phase: job.phase(),
                    detail: job.detail(),
                    timing: job.timing(),
                })
                .is_err()
            {
                return;
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn spawn_preparation_worker(
    job_receiver: Receiver<TopologyPreparationJob>,
    event_sender: Sender<BackgroundPreparationEvent>,
) -> bool {
    std::thread::Builder::new()
        .name("funfern-cpu-prepare".into())
        .spawn(move || run_preparation_worker(job_receiver, event_sender))
        .is_ok()
}

#[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
pub(super) fn spawn_preparation_worker(
    job_receiver: Receiver<TopologyPreparationJob>,
    event_sender: Sender<BackgroundPreparationEvent>,
) -> bool {
    if !BROWSER_BACKGROUND_POOL_READY.load(Ordering::Acquire) {
        return false;
    }
    crate::set_browser_preparation_worker_status("scheduled");
    rayon::spawn(move || run_preparation_worker(job_receiver, event_sender));
    true
}

#[cfg(all(target_arch = "wasm32", not(feature = "browser-threads")))]
pub(super) fn spawn_preparation_worker(
    _job_receiver: Receiver<TopologyPreparationJob>,
    _event_sender: Sender<BackgroundPreparationEvent>,
) -> bool {
    false
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BackgroundAmrKind {
    Indicator,
    Adaptation,
}

pub(super) struct CanonicalAmrPreparation {
    pub(super) mesh: Arc<TriMesh>,
    pub(super) operator: Arc<CanonicalWaveOperator>,
    /// Present when the generation is driven, in which case the estimate is
    /// built from it instead: the fixed supplement's residual, energy and
    /// recovery all read authored coefficients, and on a driven medium that
    /// charges the estimator for the medium's own modulation.
    pub(super) temporal: Option<Arc<CanonicalTemporalWaveOperator>>,
    pub(super) forcing: Arc<CanonicalForcing>,
    pub(super) snapshot: CanonicalIndicatorSnapshot,
}

pub(super) struct AmrIndicatorJob {
    pub(super) job: Option<SolutionIndicatorJob>,
    pub(super) canonical: Option<CanonicalAmrPreparation>,
}

impl AmrIndicatorJob {
    pub(super) fn with_canonical(
        job: SolutionIndicatorJob,
        mesh: Arc<TriMesh>,
        operator: Arc<CanonicalWaveOperator>,
        temporal: Option<Arc<CanonicalTemporalWaveOperator>>,
        forcing: Arc<CanonicalForcing>,
        snapshot: CanonicalIndicatorSnapshot,
    ) -> Self {
        Self {
            job: Some(job),
            canonical: Some(CanonicalAmrPreparation {
                mesh,
                operator,
                temporal,
                forcing,
                snapshot,
            }),
        }
    }

    pub(super) fn phase(&self) -> &'static str {
        if self.canonical.is_some() {
            "Preparing canonical AMR estimate"
        } else {
            self.job
                .as_ref()
                .map_or("AMR estimate ready", SolutionIndicatorJob::phase)
        }
    }

    pub(super) fn advance(
        &mut self,
        budget: usize,
    ) -> Option<Result<SolutionIndicatorResult, AmrIndicatorError>> {
        if let Some(canonical) = self.canonical.take() {
            // The runtime is the operator's authored one. That is correct
            // while nothing has stamped a Switch or re-anchored a carrier,
            // which nothing in the application can do yet; when drive
            // authoring lands this has to become the bank decoded from the
            // accepted state, against the live epoch origin.
            let runtime = canonical
                .temporal
                .as_ref()
                .map(|temporal| temporal.initial_runtime());
            let supplement = match (&canonical.temporal, &runtime) {
                (Some(temporal), Some(runtime)) => canonical_temporal_indicator_supplement(
                    &canonical.mesh,
                    temporal,
                    &canonical.forcing,
                    &canonical.snapshot,
                    runtime,
                    // The spectral scale the supplement's own recovery uses.
                    // Zero is what the calibration sweep validated; the size
                    // rule's frequency is a separate option on the job.
                    0.0,
                ),
                _ => canonical_indicator_supplement(
                    &canonical.mesh,
                    &canonical.operator,
                    &canonical.forcing,
                    &canonical.snapshot,
                ),
            };
            let supplement = match supplement {
                Ok(supplement) => supplement,
                Err(error) => {
                    self.job = None;
                    return Some(Err(AmrIndicatorError::Canonical(error)));
                }
            };
            self.job = self.job.take().map(|job| {
                let job = job.with_canonical_supplement(supplement);
                match runtime {
                    Some(runtime) => job.with_instantaneous_materials(runtime),
                    None => job,
                }
            });
        }
        self.job
            .as_mut()
            .and_then(|job| job.advance(budget))
            .map(|result| result.map_err(AmrIndicatorError::Indicator))
    }
}

#[derive(Debug)]
pub(super) enum AmrIndicatorError {
    Canonical(WaveError),
    Indicator(SolutionIndicatorError),
}

impl std::fmt::Display for AmrIndicatorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Canonical(error) => write!(formatter, "Canonical AMR estimate failed: {error}"),
            Self::Indicator(error) => error.fmt(formatter),
        }
    }
}

pub(super) enum BackgroundAmrJob {
    Indicator(Box<AmrIndicatorJob>),
    Adaptation(Box<MeshAdaptationJob>),
}

impl BackgroundAmrJob {
    pub(super) fn kind(&self) -> BackgroundAmrKind {
        match self {
            Self::Indicator(_) => BackgroundAmrKind::Indicator,
            Self::Adaptation(_) => BackgroundAmrKind::Adaptation,
        }
    }

    pub(super) fn phase(&self) -> &'static str {
        match self {
            Self::Indicator(job) => job.phase(),
            Self::Adaptation(job) => job.phase(),
        }
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", not(feature = "browser-threads")),
    allow(dead_code)
)]
pub(super) enum BackgroundAmrCommand {
    Run { serial: u64, job: BackgroundAmrJob },
    Cancel,
}

#[cfg_attr(
    all(target_arch = "wasm32", not(feature = "browser-threads")),
    allow(dead_code)
)]
pub(super) enum BackgroundAmrResult {
    Indicator(Result<SolutionIndicatorResult, AmrIndicatorError>),
    Adaptation(Result<MeshAdaptationResult, MeshAdaptationError>),
}

#[cfg_attr(
    all(target_arch = "wasm32", not(feature = "browser-threads")),
    allow(dead_code)
)]
pub(super) enum BackgroundAmrEvent {
    WorkerStarted,
    Progress {
        serial: u64,
        kind: BackgroundAmrKind,
        phase: &'static str,
    },
    Finished {
        serial: u64,
        result: Box<BackgroundAmrResult>,
    },
}

impl BackgroundAmrEvent {
    pub(super) fn serial(&self) -> Option<u64> {
        match self {
            Self::WorkerStarted => None,
            Self::Progress { serial, .. } | Self::Finished { serial, .. } => Some(*serial),
        }
    }
}

/// One independent worker advances the immutable solution estimate and the
/// subsequent mesh transaction. Their old two-millisecond UI slices made total
/// AMR throughput proportional to display FPS: precisely when the GPU was
/// overloaded, adaptation also appeared to stop. Assembly has its own worker,
/// so accepting an adapted mesh can start candidate preparation without either
/// job sharing a queue or a core with the other.
pub(super) struct BackgroundAmrWorker {
    pub(super) sender: Sender<BackgroundAmrCommand>,
    pub(super) receiver: Mutex<Receiver<BackgroundAmrEvent>>,
    pub(super) next_serial: u64,
    pub(super) active_serial: Option<u64>,
    pub(super) kind: Option<BackgroundAmrKind>,
    pub(super) phase: Option<&'static str>,
}

impl BackgroundAmrWorker {
    #[cfg(any(
        not(target_arch = "wasm32"),
        all(target_arch = "wasm32", feature = "browser-threads")
    ))]
    const QUANTUM: std::time::Duration = std::time::Duration::from_millis(8);

    pub(super) fn spawn() -> Option<Self> {
        let (job_sender, job_receiver) = mpsc::channel::<BackgroundAmrCommand>();
        let (event_sender, event_receiver) = mpsc::channel::<BackgroundAmrEvent>();
        if !spawn_amr_worker(job_receiver, event_sender) {
            return None;
        }
        Some(Self {
            sender: job_sender,
            receiver: Mutex::new(event_receiver),
            next_serial: 0,
            active_serial: None,
            kind: None,
            phase: None,
        })
    }

    pub(super) fn submit(&mut self, job: BackgroundAmrJob) -> Result<(), BackgroundAmrJob> {
        let kind = job.kind();
        let phase = job.phase();
        let serial = self.next_serial.wrapping_add(1).max(1);
        self.next_serial = serial;
        match self.sender.send(BackgroundAmrCommand::Run { serial, job }) {
            Ok(()) => {
                self.active_serial = Some(serial);
                self.kind = Some(kind);
                self.phase = Some(phase);
                Ok(())
            }
            Err(error) => match error.0 {
                BackgroundAmrCommand::Run { job, .. } => Err(job),
                BackgroundAmrCommand::Cancel => unreachable!(),
            },
        }
    }

    pub(super) fn cancel(&mut self) {
        self.active_serial = None;
        self.kind = None;
        self.phase = None;
        let _ = self.sender.send(BackgroundAmrCommand::Cancel);
    }

    pub(super) fn cancel_kind(&mut self, kind: BackgroundAmrKind) {
        if self.kind == Some(kind) {
            self.cancel();
        }
    }

    pub(super) fn drain(&mut self) -> Vec<BackgroundAmrEvent> {
        let events = {
            let receiver = self.receiver.lock().unwrap();
            std::iter::from_fn(|| receiver.try_recv().ok()).collect::<Vec<_>>()
        };
        let mut current = Vec::new();
        for event in events {
            let Some(serial) = event.serial() else {
                current.push(event);
                continue;
            };
            if self.active_serial != Some(serial) {
                continue;
            }
            match &event {
                BackgroundAmrEvent::WorkerStarted => unreachable!("handled above"),
                BackgroundAmrEvent::Progress { kind, phase, .. } => {
                    self.kind = Some(*kind);
                    self.phase = Some(*phase);
                }
                BackgroundAmrEvent::Finished { .. } => {
                    self.active_serial = None;
                    self.kind = None;
                    self.phase = None;
                }
            }
            current.push(event);
        }
        current
    }
}

#[cfg(any(
    not(target_arch = "wasm32"),
    all(target_arch = "wasm32", feature = "browser-threads")
))]
pub(super) fn run_amr_worker(
    job_receiver: Receiver<BackgroundAmrCommand>,
    event_sender: Sender<BackgroundAmrEvent>,
) {
    if event_sender
        .send(BackgroundAmrEvent::WorkerStarted)
        .is_err()
    {
        return;
    }
    while let Ok(command) = job_receiver.recv() {
        let BackgroundAmrCommand::Run {
            mut serial,
            mut job,
        } = command
        else {
            continue;
        };
        loop {
            let mut cancelled = false;
            loop {
                match job_receiver.try_recv() {
                    Ok(BackgroundAmrCommand::Run {
                        serial: newer_serial,
                        job: newer,
                    }) => {
                        serial = newer_serial;
                        job = newer;
                        cancelled = false;
                    }
                    Ok(BackgroundAmrCommand::Cancel) => cancelled = true,
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                }
            }
            if cancelled {
                break;
            }

            let started = Instant::now();
            let result = loop {
                let result = match &mut job {
                    BackgroundAmrJob::Indicator(job) => {
                        job.advance(64).map(BackgroundAmrResult::Indicator)
                    }
                    BackgroundAmrJob::Adaptation(job) => {
                        job.advance(64).map(BackgroundAmrResult::Adaptation)
                    }
                };
                if result.is_some() || started.elapsed() >= BackgroundAmrWorker::QUANTUM {
                    break result;
                }
            };
            if let Some(result) = result {
                if event_sender
                    .send(BackgroundAmrEvent::Finished {
                        serial,
                        result: Box::new(result),
                    })
                    .is_err()
                {
                    return;
                }
                break;
            }
            if event_sender
                .send(BackgroundAmrEvent::Progress {
                    serial,
                    kind: job.kind(),
                    phase: job.phase(),
                })
                .is_err()
            {
                return;
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn spawn_amr_worker(
    job_receiver: Receiver<BackgroundAmrCommand>,
    event_sender: Sender<BackgroundAmrEvent>,
) -> bool {
    std::thread::Builder::new()
        .name("funfern-amr".into())
        .spawn(move || run_amr_worker(job_receiver, event_sender))
        .is_ok()
}

#[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
pub(super) fn spawn_amr_worker(
    job_receiver: Receiver<BackgroundAmrCommand>,
    event_sender: Sender<BackgroundAmrEvent>,
) -> bool {
    if !BROWSER_BACKGROUND_POOL_READY.load(Ordering::Acquire) {
        return false;
    }
    crate::set_browser_amr_worker_status("scheduled");
    rayon::spawn(move || run_amr_worker(job_receiver, event_sender));
    true
}

#[cfg(all(target_arch = "wasm32", not(feature = "browser-threads")))]
pub(super) fn spawn_amr_worker(
    _job_receiver: Receiver<BackgroundAmrCommand>,
    _event_sender: Sender<BackgroundAmrEvent>,
) -> bool {
    false
}

pub(super) fn compile_gpu_upload(
    candidate: PreparedTopology,
    active: Option<Arc<PreparedTopology>>,
    time_step: f64,
    runtime_serials: [u32; 4],
) -> Result<PreparedGpuUpload, String> {
    let clock = CanonicalGpuClock::initial(time_step).map_err(|error| format!("{error:?}"))?;
    let plan = compile_generation_plan(&candidate, time_step, clock)?;
    if candidate.fresh || active.is_none() {
        return Ok(PreparedGpuUpload {
            plan,
            transfer: None,
        });
    }
    let active = active.unwrap();
    let transfer = candidate
        .canonical_transfer
        .as_ref()
        .ok_or_else(|| "Canonical handoff maps are not prepared".to_owned())?;
    let runtime = CanonicalGpuRuntimeTransfer::from_primary_transfer(
        &active.canonical_operator,
        &candidate.canonical_operator,
        &active.canonical_forcing,
        &candidate.canonical_forcing,
        &transfer.primary,
        runtime_serials,
    )
    .map_err(|error| format!("{error:?}"))?;
    let gpu_transfer = CanonicalGpuTransferPlan::compile_prepared(
        &active.canonical_operator,
        &candidate.canonical_operator,
        &active.canonical_forcing,
        &candidate.canonical_forcing,
        &transfer.primary,
        &transfer.complementary,
        &transfer.thin_gap,
        &transfer.outgoing,
        &runtime,
    )
    .map_err(|error| format!("{error:?}"))?;
    // Two driven generations also carry a material runtime bank - each drive's
    // carrier phase and each material's Switch trajectory - and the handoff
    // checks that the transfer describes as many records as the plans hold. It
    // is also what keeps a pump's phase and a Switch mid-ramp across an edit
    // rather than restarting them from their authored anchors.
    //
    // The source plan is rebuilt rather than kept, because the request holds
    // device buffers rather than the plan they came from. Only its layout and
    // its drives' identities are read, and those follow from the operator and
    // the forcing, so rebuilding recovers them exactly. It is packing work on a
    // worker thread, not frame work.
    //
    // It is rebuilt at the *source's* own step, not the candidate's. Nothing
    // read back from it depends on a step, but a driven generation runs at the
    // tighter step its coefficient trajectory demands rather than the one its
    // authored coefficients allow, so the two rarely agree - and undriving a
    // medium loosens the step, which leaves the candidate's above what the
    // source will hold a state at. Packing the source at the candidate's step
    // is what refused every switch from a pump back to a linear material.
    let gpu_transfer = if active.driven() || candidate.driven() {
        let source_step = active.recommended_time_step();
        let source_clock =
            CanonicalGpuClock::initial(source_step).map_err(|error| format!("{error:?}"))?;
        let source = compile_generation_plan(&active, source_step, source_clock)?;
        gpu_transfer
            .with_temporal_material_runtime(&source, &plan)
            .map_err(|error| format!("{error:?}"))?
    } else {
        gpu_transfer
    };
    Ok(PreparedGpuUpload {
        plan,
        transfer: Some(gpu_transfer),
    })
}

/// The device plan for one prepared generation, driven or not.
///
/// The source plan is rebuilt rather than kept, because the request holds
/// device buffers rather than the plan they came from. Only its layout and its
/// drives' identities are read back out of it, and both follow from the
/// operator and the forcing, so rebuilding recovers them exactly.
fn compile_generation_plan(
    prepared: &PreparedTopology,
    time_step: f64,
    clock: CanonicalGpuClock,
) -> Result<CanonicalGpuPlan, String> {
    match &prepared.canonical_temporal_operator {
        Some(temporal) => {
            let state = CanonicalTemporalWaveState::zero(temporal, time_step)
                .map_err(|error| error.to_string())?;
            CanonicalGpuPlan::compile_temporal(temporal, &state, &prepared.canonical_forcing, clock)
        }
        None => {
            let state =
                CanonicalWaveState::zero_for_backend(&prepared.canonical_operator, time_step)
                    .map_err(|error| error.to_string())?;
            CanonicalGpuPlan::compile_with_quadratic(
                &prepared.canonical_operator,
                &prepared.operator,
                &state,
                &prepared.canonical_forcing,
                clock,
            )
        }
    }
    .map_err(|error| format!("{error:?}"))
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn dispatch_gpu_upload_preparation(
    sender: Sender<Result<PreparedGpuUpload, String>>,
    candidate: PreparedTopology,
    active: Option<Arc<PreparedTopology>>,
    time_step: f64,
    runtime_serials: [u32; 4],
) {
    let failure_sender = sender.clone();
    if std::thread::Builder::new()
        .name("funfern-gpu-pack".into())
        .spawn(move || {
            let _ = sender.send(compile_gpu_upload(
                candidate,
                active,
                time_step,
                runtime_serials,
            ));
        })
        .is_err()
    {
        let _ = failure_sender.send(Err("Canonical GPU packing worker did not start".into()));
    }
}

#[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
pub(super) fn dispatch_gpu_upload_preparation(
    sender: Sender<Result<PreparedGpuUpload, String>>,
    candidate: PreparedTopology,
    active: Option<Arc<PreparedTopology>>,
    time_step: f64,
    runtime_serials: [u32; 4],
) {
    if BROWSER_BACKGROUND_POOL_READY.load(Ordering::Acquire) {
        crate::set_browser_gpu_pack_status("scheduled");
        rayon::spawn(move || {
            let _ = sender.send(compile_gpu_upload(
                candidate,
                active,
                time_step,
                runtime_serials,
            ));
        });
    } else {
        let _ = sender.send(compile_gpu_upload(
            candidate,
            active,
            time_step,
            runtime_serials,
        ));
    }
}

#[cfg(all(target_arch = "wasm32", not(feature = "browser-threads")))]
pub(super) fn dispatch_gpu_upload_preparation(
    sender: Sender<Result<PreparedGpuUpload, String>>,
    candidate: PreparedTopology,
    active: Option<Arc<PreparedTopology>>,
    time_step: f64,
    runtime_serials: [u32; 4],
) {
    let _ = sender.send(compile_gpu_upload(
        candidate,
        active,
        time_step,
        runtime_serials,
    ));
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;
    use funfern_app::topology_editor::TopologyAcceptance;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn background_worker_returns_a_candidate_to_the_runtime_transaction() {
        let mut state = Playground::default();
        assert!(state.background_preparation.is_some());
        for _ in 0..100_000 {
            state.editor.validate_frame(64);
            if state.editor.acceptance != TopologyAcceptance::Pending {
                break;
            }
        }
        assert_eq!(state.editor.acceptance, TopologyAcceptance::Valid);
        state.request_runtime();
        assert!(state.runtime.phase().is_some());

        state.advance_runtime_preparation();
        assert!(state.runtime.phase().is_none());
        assert!(state.preparation_in_progress());

        let started = std::time::Instant::now();
        while state.runtime.ready().is_none()
            && state.runtime.last_error().is_none()
            && started.elapsed() < std::time::Duration::from_secs(5)
        {
            std::thread::yield_now();
            state.advance_runtime_preparation();
        }
        assert!(state.runtime.last_error().is_none());
        assert!(
            state.runtime.ready().is_some(),
            "native preparation timed out"
        );
        assert!(state.handoff_ready.is_some());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn background_amr_worker_finishes_a_mesh_transaction() {
        let scene = Scene::default();
        let mesh = Arc::new(
            mesh_scene(
                &scene,
                1,
                MeshingOptions {
                    target_edge_length: 0.2,
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        let job = MeshAdaptationJob::new(
            mesh.clone(),
            scene,
            MeshAdaptationState::from_mesh(&mesh),
            2,
            Arc::new(|_, _| 0.2),
            MeshAdaptationOptions {
                minimum_target_edge_length: 0.01,
                maximum_target_edge_length: 0.3,
                max_refinement_changes: 0,
                max_coarsening_changes: 0,
                ..Default::default()
            },
        );
        let mut worker = BackgroundAmrWorker::spawn().expect("native AMR worker");
        worker
            .submit(BackgroundAmrJob::Adaptation(Box::new(job)))
            .map_err(|_| ())
            .unwrap();

        let started = std::time::Instant::now();
        loop {
            for event in worker.drain() {
                if let BackgroundAmrEvent::Finished { result, .. } = event {
                    let BackgroundAmrResult::Adaptation(result) = *result else {
                        panic!("wrong AMR result kind");
                    };
                    let result = result.unwrap();
                    assert_eq!(result.report.topology_changes, 0);
                    return;
                }
            }
            assert!(
                started.elapsed() < std::time::Duration::from_secs(5),
                "native AMR worker timed out"
            );
            std::thread::yield_now();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn background_amr_worker_prepares_the_canonical_supplement() {
        let mut scene = Scene::initial();
        scene.outer_boundaries =
            OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting);
        let mesh = Arc::new(
            mesh_scene(
                &scene,
                1,
                MeshingOptions {
                    target_edge_length: 0.2,
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        let operator = Arc::new(
            QuadraticWaveOperator::assemble_scene(
                &mesh,
                &scene,
                OuterBoundaryCondition::Reflecting,
            )
            .unwrap(),
        );
        let canonical =
            Arc::new(CanonicalWaveOperator::compile_scene(&mesh, &operator, &scene, 1).unwrap());
        let forcing = Arc::new(CanonicalForcing::none(&canonical));
        let dofs = operator.degrees_of_freedom();
        let snapshot = QuadraticSolutionSnapshot {
            mesh_revision: mesh.mesh_revision,
            displacement: vec![0.0; dofs],
            velocity: vec![0.0; dofs],
            acceleration: vec![0.0; dofs],
            auxiliary: vec![0.0; dofs],
            volume_acceleration: vec![0.0; dofs],
            time: 0.01,
            time_step: 0.01,
        };
        let canonical_snapshot = CanonicalIndicatorSnapshot {
            mesh_revision: mesh.mesh_revision,
            primary_flux: vec![0.0; canonical.degrees_of_freedom()],
            previous_primary_flux: vec![0.0; canonical.degrees_of_freedom()],
            complementary_flux: vec![
                Point2::default();
                canonical.complementary_degrees_of_freedom()
            ],
            previous_complementary_flux: vec![
                Point2::default();
                canonical.complementary_degrees_of_freedom()
            ],
            auxiliary: Vec::new(),
            previous_auxiliary: Vec::new(),
            time: 0.01,
            time_step: 0.01,
        };
        let job = AmrIndicatorJob::with_canonical(
            SolutionIndicatorJob::new(
                mesh.clone(),
                operator,
                scene,
                snapshot,
                SolutionIndicatorOptions::default(),
            ),
            mesh,
            canonical,
            None,
            forcing,
            canonical_snapshot,
        );
        assert_eq!(job.phase(), "Preparing canonical AMR estimate");

        let mut worker = BackgroundAmrWorker::spawn().expect("native AMR worker");
        worker
            .submit(BackgroundAmrJob::Indicator(Box::new(job)))
            .map_err(|_| ())
            .unwrap();

        let started = std::time::Instant::now();
        loop {
            for event in worker.drain() {
                if let BackgroundAmrEvent::Finished { result, .. } = event {
                    let BackgroundAmrResult::Indicator(result) = *result else {
                        panic!("wrong AMR result kind");
                    };
                    let result = result.unwrap();
                    assert_eq!(result.report.canonical_drift_contribution, 0.0);
                    assert_eq!(result.report.complementary_recovery_contribution, 0.0);
                    return;
                }
            }
            assert!(
                started.elapsed() < std::time::Duration::from_secs(5),
                "native canonical AMR worker timed out"
            );
            std::thread::yield_now();
        }
    }
}
