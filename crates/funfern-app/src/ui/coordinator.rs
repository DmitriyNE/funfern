//! The runtime's bookkeeping between the host and the device: what was asked
//! for, what is in flight, what waits for the device, and what the running
//! generation runs at.
//!
//! A transition of the runtime moves several of these at once - dropping a
//! replaced scene's generation, a Reset, a published upload - and each time
//! one of them was missed it showed: a pulse sent into the field Reset had
//! cleared, a Switch direction kept across a Reset, a candidate's step paced
//! after its handoff was refused. So each such transition is a method here
//! that names everything it moves, and `refresh_runtime` calls it rather than
//! moving them one by one. `funfern-protocol` models these transitions, and
//! the runtime tests hold this code to that model.

use super::*;
use std::collections::VecDeque;

pub(super) struct Coordinator {
    // What was asked for.
    /// The Remesh button: rebuild at the current resolution even though
    /// nothing changed, which also leaves an adapted mesh.
    pub(super) remesh_requested: bool,
    pub(super) requested_edge: f64,
    pub(super) requested_revision: Option<u64>,
    /// What the latest request read, which accounts for it while the draft
    /// is invalid and its revision moves with every edit.
    pub(super) requested_inputs: Option<PreparedInputs>,
    pub(super) reset_requested: bool,
    /// Set when a whole document is replaced: the next preparation must start
    /// the field from zero rather than transfer the outgoing scene's into it.
    /// Separate from `reset_requested` because that one is spent by the GPU
    /// reset, which runs earlier in the frame and against the topology still
    /// active — the scene being replaced. Sharing one flag let the load reset
    /// the outgoing scene, which then ran on for the seconds its replacement
    /// took to prepare and handed over a full-amplitude field.
    pub(super) fresh_requested: bool,
    /// Set when a whole document is replaced, spent by the next runtime
    /// update: the outgoing scene's generation is dropped on the device and
    /// the host, as at launch, so nothing of it runs or shows while the new
    /// one prepares. See `drop_generation`.
    pub(super) drop_requested: bool,

    // What is in flight.
    pub(super) gpu_upload_preparation: Option<GpuUploadPreparation>,
    pub(super) uploading: Option<Uploading>,
    pub(super) source_commit: Option<PendingSourceCommit>,

    // What waits for the device.
    /// Pulses placed and not yet taken by the device, oldest first. One
    /// waits while the generation is busy with another event; a single slot
    /// lost the second of two quick clicks.
    pub(super) pending_pulses: VecDeque<(Point2, RegionId)>,
    /// Material Switches asked for by button or hotkey and not yet taken by
    /// the device, oldest first, each material once. A single slot kept only
    /// the last of two materials switched while the generation was busy.
    pub(super) pending_switches: VecDeque<MaterialId>,

    // What the running generation runs at.
    /// The step the device's generation runs at. Not the active operator's
    /// recommendation: the speed ceiling can ask for a smaller one, and between
    /// a speed change and the republish that carries it the two differ. A
    /// handoff changes it when the device publishes the candidate, not when the
    /// upload starts: until then the accepted generation is the one stepping,
    /// and a rejected handoff keeps it.
    pub(super) uploaded_time_step: f64,
    /// The Switch direction last sent for each material. The accepted runtime
    /// only arrives with a full snapshot, so a second press before then would
    /// otherwise read the old direction and send the same one again.
    pub(super) switch_targets: BTreeMap<MaterialId, bool>,
    pub(super) canonical_event_serial: u32,
    pub(super) canonical_event_observed: u32,
    /// The device failure the run is paused on, if any. Run, Step or an edit
    /// resumes from the last accepted step; the failure itself never commits.
    pub(super) solver_fault: Option<u32>,
}

impl Default for Coordinator {
    fn default() -> Self {
        Self {
            remesh_requested: false,
            requested_edge: f64::NAN,
            requested_revision: None,
            requested_inputs: None,
            reset_requested: false,
            fresh_requested: false,
            drop_requested: false,
            gpu_upload_preparation: None,
            uploading: None,
            source_commit: None,
            pending_pulses: VecDeque::new(),
            pending_switches: VecDeque::new(),
            uploaded_time_step: 0.0,
            switch_targets: BTreeMap::new(),
            canonical_event_serial: 0,
            canonical_event_observed: 0,
            solver_fault: None,
        }
    }
}

impl Coordinator {
    /// A request was made for the document at `revision`, reading `inputs`,
    /// at mesh edge `edge`; it starts from zero if a Reset or a replacement
    /// asked for that, so both are spent.
    pub(super) fn requested(&mut self, revision: u64, inputs: PreparedInputs, edge: f64) {
        self.requested_revision = Some(revision);
        self.requested_inputs = Some(inputs);
        self.requested_edge = edge;
        self.reset_requested = false;
        self.fresh_requested = false;
    }

    /// A whole scene came in: its first request starts from zero, and the
    /// outgoing scene's generation is dropped at the next runtime update.
    pub(super) fn scene_replaced(&mut self, fresh: bool) {
        self.requested_revision = None;
        self.fresh_requested = fresh;
        self.drop_requested = true;
    }

    /// The outgoing scene's generation dropped: nothing in flight, nothing
    /// waiting for the device, no step running, and the event serials and the
    /// fault latch back where launch has them.
    pub(super) fn dropped(&mut self) {
        self.uploading = None;
        self.source_commit = None;
        self.gpu_upload_preparation = None;
        self.uploaded_time_step = 0.0;
        self.canonical_event_serial = 0;
        self.canonical_event_observed = 0;
        self.solver_fault = None;
        self.pending_pulses.clear();
        self.pending_switches.clear();
    }

    /// Reset installed the active topology from zero at step `time_step`.
    /// The reset medium starts at its authored laws, wherever a Switch had
    /// sent it, and a pulse or a Switch press still waiting was meant for the
    /// run Reset discarded: sent on, it went into the cleared field in the
    /// frame Reset installed it.
    pub(super) fn reset_installed(&mut self, time_step: f64) {
        self.reset_requested = false;
        self.uploaded_time_step = time_step;
        self.canonical_event_serial = 0;
        self.canonical_event_observed = 0;
        self.switch_targets.clear();
        self.pending_pulses.clear();
        self.pending_switches.clear();
    }

    /// The device published the upload in flight: its step is the one
    /// running from now. Answers the upload.
    pub(super) fn published(&mut self) -> Uploading {
        let upload = self.uploading.take().expect("an upload to publish");
        self.uploaded_time_step = upload.time_step;
        upload
    }

    /// A published generation that started from zero: its events count from
    /// the start again, and its medium starts from the authored runtime.
    pub(super) fn started_afresh(&mut self) {
        self.canonical_event_serial = 0;
        self.canonical_event_observed = 0;
        self.switch_targets.clear();
    }

    /// Queues a Switch press for the next frame that can send it. A material
    /// already waiting is not queued again: its button still shows the
    /// direction it had, so a second press is the same request.
    pub(super) fn queue_switch(&mut self, material: MaterialId) {
        if !self.pending_switches.contains(&material) {
            self.pending_switches.push_back(material);
        }
    }
}
