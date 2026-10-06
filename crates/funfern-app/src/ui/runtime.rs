//! Driving the solver runtime: preparation phases, the handoff transaction,
//! and the per-frame refresh that commits a candidate and advances the wave.

use crate::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuHandoffOutcome, CanonicalGpuLiveEvent,
    CanonicalGpuPlan, CanonicalGpuRequest, canonical_failure_description,
};
use crate::drawn_pacing::PacingNote;
use crate::gpu_frame_timer::GpuFrameReading;
use crate::wave_gpu::{VectorOverlayDisplay, WaveGpuRequest};
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use funfern_app::topology_editor::TopologyAcceptance;
use funfern_app::topology_runtime::{
    PreparedSolverUpdate, PreparedTopology, TopologyPreparationPhase, TopologyPreparationTiming,
    TopologyToken,
};
use funfern_core::*;
use std::sync::{
    Arc, Mutex,
    mpsc::{self},
};

use super::*;

impl Playground {
    /// Wall time a frame lends to topology preparation. The runtime checks the
    /// deadline between individual work units, including each formula-heavy
    /// canonical element, so this remains a latency bound rather than only an
    /// average throughput target.
    pub(super) const PREPARATION_FRAME_BUDGET: std::time::Duration =
        std::time::Duration::from_millis(4);
    /// Candidate state can live either in the cooperative runtime runner or in
    /// a background worker. Keep that placement detail out of UI/status policy.
    pub(super) fn preparation_phase(&self) -> Option<TopologyPreparationPhase> {
        self.runtime.phase().or_else(|| {
            self.background_preparation
                .as_ref()
                .and_then(|worker| worker.phase)
        })
    }

    pub(super) fn preparation_detail(&self) -> Option<&'static str> {
        self.runtime.detail().or_else(|| {
            self.background_preparation
                .as_ref()
                .and_then(|worker| worker.detail)
        })
    }

    pub(super) fn preparation_timing(&self) -> Option<TopologyPreparationTiming> {
        self.runtime.preparing_timing().or_else(|| {
            self.background_preparation
                .as_ref()
                .and_then(|worker| worker.timing)
        })
    }

    /// The host side of dropping a replaced scene's generation, beside the
    /// device's `CanonicalGpuRequest::clear`: no active topology, nothing
    /// uploading or committing, and the clock, the live-event serials, the
    /// probe traces, the exposures and adaptation back where launch has them.
    /// The new scene's first request then sees nothing active and prepares a
    /// fresh full build, which is the launch path.
    pub(super) fn drop_generation(&mut self) {
        self.runtime.clear_active();
        self.uploading = None;
        self.source_commit = None;
        self.gpu_upload_preparation = None;
        self.uploaded_time_step = 0.0;
        self.sim_time_offset = 0.0;
        self.sim_time_step = 0.0;
        self.accumulator = 0.0;
        self.step_backlog = 0;
        self.canonical_event_serial = 0;
        self.canonical_event_observed = 0;
        self.solver_fault = None;
        self.pending_pulse = None;
        self.pending_switch = None;
        self.probe_upload = None;
        self.restart_probe_traces();
        self.clear_exposures();
        self.reset_adaptation();
    }

    pub(super) fn preparation_in_progress(&self) -> bool {
        self.preparation_phase().is_some()
    }

    /// Advances candidate assembly without making its execution placement part
    /// of the topology transaction. Native and cross-origin-isolated browser
    /// builds hand the immutable job to one long-lived worker; failed worker
    /// bootstrap or channels use the existing bounded cooperative runner.
    pub(super) fn advance_runtime_preparation(&mut self) {
        let events = self
            .background_preparation
            .as_ref()
            .map_or_else(Vec::new, BackgroundPreparationWorker::drain);
        for event in events {
            #[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
            if matches!(event, BackgroundPreparationEvent::WorkerStarted) {
                crate::set_browser_preparation_worker_status("active");
            }
            if let Some(worker) = &mut self.background_preparation {
                worker.observe(&event);
            }
            if let BackgroundPreparationEvent::Finished { result, .. } = event
                && let Some(Ok(_)) = self.runtime.finish_external_preparation(*result)
            {
                self.handoff_ready = Some(Instant::now());
            }
        }

        if let Some(job) = self.runtime.take_preparing_job() {
            let rejected = match &mut self.background_preparation {
                Some(worker) => worker.submit(job),
                None => Some(Box::new(job)),
            };
            if let Some(job) = rejected {
                self.background_preparation = None;
                self.runtime.restore_preparing_job(*job);
            }
        }

        if self.background_preparation.is_some() {
            return;
        }

        if let Some(Ok(_)) = self.runtime.advance_for(Self::PREPARATION_FRAME_BUDGET) {
            self.handoff_ready = Some(Instant::now());
        }
    }

    pub(super) fn request_runtime(&mut self) {
        if self.editor.acceptance != TopologyAcceptance::Valid
            || self.editor.editing()
            || self.mesh_edge_dragging
            || self.source_commit.is_some()
        {
            return;
        }
        let domain = self.editor.document.model.accepted.geometry.domain;
        let options = MeshingOptions {
            target_edge_length: self.editor.document.presentation.mesh_edge,
            curve_tolerance: (self.editor.document.presentation.mesh_edge * 0.02).min(5e-4),
            ..MeshingOptions::default()
        }
        .sized_for_area(domain.width() * domain.height());
        // This revision is accounted for when a preparation is in flight for
        // it, when it is already the accepted generation, or when preparing it
        // has already failed. Without that last case a revision that cannot be
        // prepared is retried every frame forever: the whole preparation runs
        // again, the phase label churns, and the error it is reporting is
        // replaced before it can be read. The next edit, a different mesh edge
        // or an explicit remesh moves on; Reset publishes onto the accepted
        // generation and does not come through here.
        if !self.remesh_requested
            && self.requested_revision == Some(self.editor.revision)
            && self.requested_edge == self.editor.document.presentation.mesh_edge
            && (self.preparation_in_progress()
                || self.runtime.active().is_some_and(|active| {
                    active.bundle.token.document_revision == self.editor.revision
                })
                || self
                    .runtime
                    .last_error()
                    .is_some_and(|failure| failure.token.document_revision == self.editor.revision))
        {
            return;
        }
        let fresh = starts_from_zero(
            self.runtime.active().is_some(),
            self.reset_requested,
            self.fresh_requested,
        );
        if std::mem::take(&mut self.remesh_requested) {
            self.runtime.request_full_rebuild();
        }
        self.runtime
            .set_preserve_adaptation(self.editor.document.presentation.adaptation.enabled);
        let require_solver_handoff = self.uploaded_time_step > 0.0
            && self.runtime.active().is_some_and(|active| {
                let wanted = paced_time_step(
                    active.recommended_time_step(),
                    self.editor.document.presentation.simulation_speed,
                );
                (wanted - self.uploaded_time_step).abs()
                    > 1.0e-12 * wanted.abs().max(self.uploaded_time_step.abs()).max(1.0)
            });
        match self.runtime.request_with_handoff(
            self.editor.revision,
            &self.editor.document,
            self.editor.compiled_accepted.clone(),
            options,
            fresh,
            require_solver_handoff,
        ) {
            Ok(_) => {
                self.requested_revision = Some(self.editor.revision);
                self.requested_edge = self.editor.document.presentation.mesh_edge;
                self.reset_requested = false;
                self.fresh_requested = false;
                self.begin_handoff_timeline();
            }
            Err(error) => self.message = error,
        }
    }
    pub(super) fn begin_handoff_timeline(&mut self) {
        self.handoff_requested = Some(Instant::now());
        self.handoff_ready = None;
        self.handoff_packed = None;
        self.handoff_upload = None;
    }
    pub(super) fn record_handoff(&mut self, active: &Arc<PreparedTopology>) {
        let now = Instant::now();
        let millis = |from: Option<Instant>, to: Instant| {
            from.map_or(0.0, |from| (to - from).as_secs_f64() * 1000.0)
        };
        let ready = self.handoff_ready.unwrap_or(now);
        let packed = self.handoff_packed.unwrap_or(ready);
        let upload = self.handoff_upload.unwrap_or(packed);
        self.last_handoff = Some(HandoffRecord {
            prepare_ms: millis(self.handoff_requested, ready),
            pack_ms: millis(Some(ready), packed),
            drain_ms: millis(Some(packed), upload),
            upload_ms: millis(Some(upload), now),
            timing: active.timing,
            action: active.mesh_action,
            operator_reused: active.operator_reused,
            adapted: active.adapted,
            transferred: active.exact_transfer_nodes.is_some(),
            exact_nodes: active.exact_transfer_nodes.unwrap_or(0),
            fresh: active.fresh,
            degrees_of_freedom: active.operator.degrees_of_freedom(),
            triangles: active.mesh.triangles.len(),
            carve: active.carve,
            repair_fallback: active.repair_fallback.clone(),
        });
        // A fallback is only in the record until the next transaction replaces
        // it, and the record says nothing about how often one has happened.
        if let Some(fallback) = &active.repair_fallback {
            self.pending_repairs.push(fallback.clone());
        }
        self.handoff_requested = None;
        self.handoff_ready = None;
        self.handoff_packed = None;
        self.handoff_upload = None;
    }

    pub(super) fn commit_in_place(&mut self, token: TopologyToken, message: &'static str) {
        match self.runtime.commit_ready(token) {
            Ok(active) => {
                if let Some(worker) = &mut self.background_amr {
                    worker.cancel_kind(BackgroundAmrKind::Indicator);
                }
                self.amr_indicator_job = None;
                self.amr_indicator_completed = None;
                self.amr_indicator_source = None;
                self.amr_indicator_result = None;
                self.amr_last_analyzed_step = None;
                self.message = message.into();
                self.record_handoff(&active);
            }
            Err(error) => self.message = error,
        }
    }

    pub(super) fn finish_source_commit(&mut self, request: &CanonicalGpuRequest) {
        let Some(pending) = self.source_commit.as_ref() else {
            return;
        };
        let processed = request.stats().processed_event();
        if processed < pending.serial {
            return;
        }
        let pending = self.source_commit.take().unwrap();
        self.canonical_event_observed = processed;
        let rejection = request.stats().event_rejection();
        if rejection != 0 {
            self.runtime.reject_ready(
                pending.token,
                format!("Canonical source update failed (failure code {rejection})"),
            );
            self.message = format!(
                "Canonical source update was rejected without changing the accepted state (failure code {rejection})"
            );
            self.unseen_error = true;
        } else {
            self.commit_in_place(pending.token, "Simulation sources committed");
        }
    }

    pub(super) fn time_step_unchanged(&self, candidate: &PreparedTopology) -> bool {
        if self.uploaded_time_step <= 0.0 {
            return false;
        }
        let wanted = paced_time_step(
            candidate.recommended_time_step(),
            self.editor.document.presentation.simulation_speed,
        );
        (wanted - self.uploaded_time_step).abs()
            <= 1.0e-12 * wanted.abs().max(self.uploaded_time_step.abs()).max(1.0)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn refresh_runtime(
        &mut self,
        request: &mut CanonicalGpuRequest,
        display: &CanonicalGpuDisplay,
        recorders: &mut WaveGpuRequest,
        vector_display: &VectorOverlayDisplay,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
        delta: f64,
    ) {
        if std::mem::take(&mut self.drop_requested) {
            request.clear(assets, commands);
            recorders.clear_recorders(assets, commands);
            self.drop_generation();
        }
        request.set_grid_scale_filter(self.editor.document.presentation.grid_scale_filter);
        self.finish_source_commit(request);
        self.supervise_solver_fault(request, assets, commands);
        self.refresh_switch_states(display, request);
        // Starting another preparation mid-upload clears `runtime.ready`, and
        // would make the accepted GPU generation impossible to publish under
        // its immutable topology token. A later frame picks the edit up.
        // What the frame actually cost decides what the next one may spend on
        // the solver. Both readings are of this frame, before anything is asked
        // for, so a batch is always sized by an outcome rather than a guess.
        self.display_cadence.observe(delta);
        // Taken, so that a frame which asked for nothing - paused, or with a
        // handoff withholding steps - is not read as the solver's doing.
        let batch = core::mem::take(&mut self.last_batch);
        // The solver's steady share of the frame, from the GPU's own clock on
        // its passes, decides whether a late frame is its doing.
        let now = Instant::now();
        let readings = request.gpu_frames().recent();
        let fresh: Vec<&GpuFrameReading> = readings
            .iter()
            .filter(|reading| {
                now.saturating_duration_since(reading.taken_at)
                    .as_secs_f64()
                    <= SOLVER_SHARE_FRESH_SECONDS
            })
            .collect();
        let passes: Vec<f64> = fresh.iter().map(|reading| reading.pass_seconds()).collect();
        // A step's cost is this generation's: another mesh's steps cost
        // something else, so the first frames after a handoff go without.
        let costs: Vec<(f64, u64)> = fresh
            .iter()
            .filter(|reading| reading.generation == request.generation())
            .map(|reading| (reading.pass_seconds(), reading.steps))
            .collect();
        self.solver_share = solver_share(&passes, self.display_cadence.seconds());
        self.step_seconds = step_seconds(&costs);
        self.step_budget.update(
            delta,
            self.display_cadence.seconds(),
            batch,
            self.solver_share,
            self.step_seconds,
        );
        if self.uploading.is_none()
            && self.source_commit.is_none()
            && self.gpu_upload_preparation.is_none()
        {
            self.retime_for_speed();
            self.request_runtime();
        }
        self.advance_runtime_preparation();
        if self.gpu_upload_preparation.as_ref().is_some_and(|job| {
            self.runtime
                .ready()
                .is_none_or(|candidate| candidate.bundle.token != job.token)
        }) {
            self.gpu_upload_preparation = None;
        }
        if self.uploading.is_none()
            && self.source_commit.is_none()
            && self.gpu_upload_preparation.is_none()
            && let Some(candidate) = self.runtime.ready().cloned()
        {
            let dt = paced_time_step(
                candidate.recommended_time_step(),
                self.editor.document.presentation.simulation_speed,
            );
            let token = candidate.bundle.token;
            let mut needs_gpu_pack = true;
            if self.time_step_unchanged(&candidate) {
                match candidate.solver_update {
                    PreparedSolverUpdate::MeasurementsOnly => {
                        self.handoff_packed = self.handoff_ready;
                        self.handoff_upload = self.handoff_ready;
                        self.commit_in_place(token, "Simulation measurements committed");
                        needs_gpu_pack = false;
                    }
                    PreparedSolverUpdate::SourceWeightsOnly
                    | PreparedSolverUpdate::SourceDrivesOnly => {
                        needs_gpu_pack = false;
                        if !request.live_event_pending() {
                            let serial = self
                                .canonical_event_serial
                                .max(request.stats().processed_event())
                                .saturating_add(1)
                                .max(1);
                            let event = match candidate.solver_update {
                                PreparedSolverUpdate::SourceWeightsOnly => self
                                    .runtime
                                    .active()
                                    .ok_or_else(|| "Canonical source generation is missing".into())
                                    .and_then(|active| {
                                        CanonicalGpuLiveEvent::source_weight_patch(
                                            &active.canonical_forcing,
                                            &candidate.canonical_forcing,
                                            dt,
                                            serial,
                                        )
                                        .map_err(|error| format!("{error:?}"))
                                    }),
                                PreparedSolverUpdate::SourceDrivesOnly => {
                                    CanonicalGpuLiveEvent::source_patch(
                                        &candidate.canonical_forcing,
                                        dt,
                                        serial,
                                    )
                                    .map_err(|error| format!("{error:?}"))
                                }
                                _ => unreachable!("matched source-only update"),
                            };
                            match event.and_then(|event| {
                                request
                                    .queue_live_event(assets, event)
                                    .map_err(str::to_owned)
                            }) {
                                Ok(()) => {
                                    self.canonical_event_serial = serial;
                                    self.handoff_packed = self.handoff_ready;
                                    self.handoff_upload = Some(Instant::now());
                                    self.source_commit =
                                        Some(PendingSourceCommit { token, serial });
                                }
                                Err(error) => match live_event_fallback(
                                    &error,
                                    candidate.canonical_transfer.is_some(),
                                ) {
                                    LiveEventFallback::Retry => {}
                                    LiveEventFallback::Pack => needs_gpu_pack = true,
                                    LiveEventFallback::Refuse => {
                                        let refusal = format!(
                                            "{error}, and this edit was prepared without the \
                                             handoff maps a packed generation needs"
                                        );
                                        self.runtime.reject_ready(token, refusal.clone());
                                        self.message = refusal;
                                    }
                                },
                            }
                        }
                    }
                    PreparedSolverUpdate::FullHandoff => {}
                }
            }
            if needs_gpu_pack {
                let active = self.runtime.active().cloned();
                let runtime_serials = display.runtime_serials;
                let (sender, receiver) = mpsc::channel();
                dispatch_gpu_upload_preparation(sender, candidate, active, dt, runtime_serials);
                self.gpu_upload_preparation = Some(GpuUploadPreparation {
                    token,
                    time_step: dt,
                    receiver: Mutex::new(receiver),
                    result: None,
                });
            }
        }
        if let Some(job) = &mut self.gpu_upload_preparation
            && job.result.is_none()
        {
            let received = job.receiver.lock().unwrap().try_recv();
            match received {
                Ok(result) => {
                    job.result = Some(result);
                    self.handoff_packed = Some(Instant::now());
                    #[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
                    crate::set_browser_gpu_pack_status("finished");
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    job.result = Some(Err("Canonical GPU packing worker stopped".into()));
                    self.handoff_packed = Some(Instant::now());
                }
            }
        }
        if self.uploading.is_none()
            && request.caught_up()
            && self
                .gpu_upload_preparation
                .as_ref()
                .is_some_and(|job| job.result.is_some())
        {
            let mut job = self.gpu_upload_preparation.take().unwrap();
            let token = job.token;
            let dt = job.time_step;
            let Some(candidate) = self
                .runtime
                .ready()
                .filter(|candidate| candidate.bundle.token == token)
                .cloned()
            else {
                return;
            };
            // A corrective edit on a paused, failed run hands off from the
            // accepted state, which the failure never touched; the latched
            // status would otherwise reject the new generation as its own.
            if request.failed() && request.clear_failure(assets, commands).is_ok() {
                self.solver_fault = None;
            }
            let mut handed_off = false;
            let upload = job.result.take().unwrap().and_then(|prepared| {
                if let Some(transfer) = prepared.transfer {
                    handed_off = true;
                    request
                        .begin_handoff(assets, commands, prepared.plan, transfer)
                        .map_err(str::to_owned)
                } else {
                    request.install(assets, commands, prepared.plan);
                    Ok(())
                }
            });
            match upload {
                Ok(()) => {
                    // An install starts its clock at zero, and publishes at
                    // once: its first steps are the new generation's. A
                    // handoff's are not until it publishes, and one that is
                    // rejected leaves the accepted generation stepping at its
                    // own step - taking the candidate's here kept it after a
                    // rejection, mispaced the solver, and made the speed
                    // ceiling request the rejected revision again.
                    if !handed_off {
                        self.uploaded_time_step = dt;
                        self.sim_time_offset = 0.0;
                        self.sim_time_step = dt;
                    }
                    self.handoff_upload = Some(Instant::now());
                    self.uploading = Some(Uploading {
                        token,
                        // Which generation to wait for follows the path just
                        // taken, not whether the candidate starts from zero.
                        // An install publishes its generation immediately; a
                        // handoff publishes on completion, so the one to wait
                        // for is the next. Those agreed only while a fresh
                        // candidate meant an install - and a candidate whose
                        // medium starts or stops being driven now installs
                        // too, because the two generations share no state to
                        // transfer. Reading `fresh` here left such an edit
                        // waiting for a generation that never arrives, with
                        // Reset gated behind the upload it was stuck in.
                        generation: if handed_off {
                            request.generation().wrapping_add(1).max(1)
                        } else {
                            request.generation()
                        },
                        fresh: candidate.fresh,
                        degrees_of_freedom: candidate.canonical_operator.degrees_of_freedom(),
                        time_step: dt,
                    });
                }
                Err(error) => {
                    self.runtime.reject_ready(token, error.clone());
                    self.message = error;
                }
            }
        }
        if let Some(upload) = &self.uploading {
            let failure = if request.failed() {
                let reason = request.stats().failure();
                Some(format!(
                    "Accepted canonical GPU generation faulted: {} (failure code {reason}); candidate was not committed",
                    canonical_failure_description(reason),
                ))
            } else if let CanonicalGpuHandoffOutcome::Rejected(reason) = request.handoff_outcome() {
                Some(format!(
                    "Canonical GPU handoff rejected: {} (failure code {reason}); accepted generation was retained",
                    canonical_failure_description(reason),
                ))
            } else {
                None
            };
            if let Some(failure) = failure {
                let token = upload.token;
                self.runtime.reject_ready(token, failure.clone());
                self.message = failure;
                self.unseen_error = true;
                self.uploading = None;
            } else if request.ready()
                && display.generation == upload.generation
                && display.primary_flux.len() == upload.degrees_of_freedom
                && !matches!(
                    request.handoff_outcome(),
                    CanonicalGpuHandoffOutcome::Pending
                )
            {
                let upload = self.uploading.take().unwrap();
                // The device has published the candidate.
                self.uploaded_time_step = upload.time_step;
                match self.runtime.commit_ready(upload.token) {
                    Ok(active) => {
                        if upload.fresh {
                            self.accumulator = 0.0;
                            self.restart_probe_traces();
                            self.canonical_event_serial = 0;
                            self.canonical_event_observed = 0;
                            self.wave_energy = None;
                            self.nonlinear_strength.clear();
                            // A fresh start begins from the authored runtime.
                            self.switch_targets.clear();
                            self.amr_energy_peak = 0.0;
                        }
                        self.restart_exposures_after_handoff(upload.fresh);
                        self.amr_adaptation_state = if active.adapted {
                            self.amr_pending_state
                                .take()
                                .filter(|state| state.mesh_revision == active.mesh.mesh_revision)
                                .or_else(|| Some(MeshAdaptationState::from_mesh(&active.mesh)))
                        } else {
                            self.amr_pending_state = None;
                            Some(MeshAdaptationState::from_mesh(&active.mesh))
                        };
                        if let Some(worker) = &mut self.background_amr {
                            worker.cancel_kind(BackgroundAmrKind::Indicator);
                        }
                        self.amr_indicator_job = None;
                        self.amr_indicator_completed = None;
                        self.amr_indicator_source = None;
                        self.amr_indicator_result = None;
                        self.amr_last_analyzed_step = None;
                        self.message = "Simulation topology committed".into();
                        self.record_handoff(&active);
                    }
                    Err(error) => self.message = error,
                }
            }
        }
        if self.reset_requested && self.uploading.is_none() && self.source_commit.is_none() {
            if let Some(active) = self.runtime.active() {
                let dt = paced_time_step(
                    active.recommended_time_step(),
                    self.editor.document.presentation.simulation_speed,
                );
                let reset = CanonicalGpuClock::initial(dt)
                    .map_err(|error| format!("{error:?}"))
                    .and_then(|clock| match &active.canonical_temporal_operator {
                        Some(temporal) => CanonicalTemporalWaveState::zero(temporal, dt)
                            .map_err(|error| error.to_string())
                            .and_then(|state| {
                                CanonicalGpuPlan::compile_temporal(
                                    temporal,
                                    &state,
                                    &active.canonical_forcing,
                                    clock,
                                )
                                .map_err(|error| format!("{error:?}"))
                            }),
                        None => CanonicalWaveState::zero(&active.canonical_operator, dt)
                            .map_err(|error| error.to_string())
                            .and_then(|state| {
                                CanonicalGpuPlan::compile_with_quadratic(
                                    &active.canonical_operator,
                                    &active.operator,
                                    &state,
                                    &active.canonical_forcing,
                                    clock,
                                )
                                .map_err(|error| format!("{error:?}"))
                            }),
                    });
                if let Ok(plan) = reset {
                    request.install(assets, commands, plan);
                    self.reset_requested = false;
                    self.uploaded_time_step = dt;
                    self.sim_time_offset = 0.0;
                    self.sim_time_step = dt;
                    self.canonical_event_serial = 0;
                    self.canonical_event_observed = 0;
                    // The reset medium starts at its authored laws, wherever
                    // a Switch had sent it.
                    self.switch_targets.clear();
                    self.restart_probe_traces();
                    self.restart_exposures_after_handoff(true);
                }
            }
        }
        recorders.adopt_canonical_generation(request.generation());
        let overlay_active = self.runtime.active().cloned().filter(|active| {
            display.generation == request.generation()
                && display.primary_flux.len() == active.canonical_operator.degrees_of_freedom()
        });
        self.refresh_vector_overlay(
            recorders,
            vector_display,
            assets,
            commands,
            overlay_active
                .as_ref()
                .and_then(|active| Some((active, RecorderSource::of(active, request)?))),
            request.generation(),
        );
        if self.uploading.is_none()
            && let Some(active) = self.runtime.active().cloned()
            && probes_need_upload(self.probe_upload, active.bundle.token, request.generation())
            && let Some(source) = RecorderSource::of(&active, request)
        {
            self.configure_probes(recorders, assets, commands, &active, source);
        }
        if let Some(active) = self.runtime.active() {
            let dt = self.solver_time_step();
            if let Some((position, region)) = (self.uploading.is_none()
                && self.source_commit.is_none())
            .then(|| self.pending_pulse.take())
            .flatten()
            {
                let mut increment = vec![0.0; active.canonical_operator.degrees_of_freedom()];
                for (triangle, nodes) in active
                    .mesh
                    .triangles
                    .iter()
                    .zip(active.canonical_operator.element_nodes())
                {
                    if triangle.region != region {
                        continue;
                    }
                    for node in nodes {
                        let delta =
                            active.canonical_operator.node_points()[*node as usize] - position;
                        increment[*node as usize] = f64::from(self.pulse_amplitude)
                            * (-0.5 * delta.dot(delta) / f64::from(self.pulse_width).powi(2)).exp();
                    }
                }
                self.canonical_event_serial = self
                    .canonical_event_serial
                    .max(request.stats().processed_event())
                    .saturating_add(1)
                    .max(1);
                match CanonicalGpuLiveEvent::primary_pulse(
                    &active.canonical_operator,
                    &increment,
                    self.canonical_event_serial,
                )
                .map_err(|error| format!("{error:?}"))
                .and_then(|event| {
                    request
                        .queue_live_event(assets, event)
                        .map_err(str::to_owned)
                }) {
                    Ok(()) => {}
                    Err(error) => self.message = error,
                }
            }
            if let Some(material) = (self.uploading.is_none() && self.source_commit.is_none())
                .then(|| self.pending_switch.take())
                .flatten()
            {
                let active = active.clone();
                self.send_material_switch(&active, display, request, assets, material);
            }
            // Drain once the packed candidate is ready so begin_handoff gets a
            // complete requested-step boundary. After that, keep advancing the
            // accepted generation while the target assets upload; the render
            // graph snapshots one complete boundary while later requests keep
            // the source display live; the target consumes that short backlog
            // after admission. Fresh installs have no outgoing generation.
            let packed_candidate_waiting = self.uploading.is_none()
                && self
                    .gpu_upload_preparation
                    .as_ref()
                    .is_some_and(|job| job.result.is_some());
            let fresh_upload = self.uploading.as_ref().is_some_and(|upload| upload.fresh);
            let withheld = canonical_steps_withheld(packed_candidate_waiting, fresh_upload);
            let mut note = PacingNote {
                frame_seconds: delta,
                speed: self.editor.document.presentation.simulation_speed,
                time_step: dt,
                ceiling: self.step_budget.ceiling,
                running: self.wave_running,
                withheld,
                picture: display.picture().map_or(0, |copy| copy.serial),
                picture_step: display.picture().map_or(0, |copy| copy.step),
                picture_generation: display.generation,
                solver_share: self.solver_share.unwrap_or(f64::NAN),
                step_seconds: self.step_seconds.unwrap_or(f64::NAN),
                target: self.step_budget.target,
                ..PacingNote::default()
            };
            if !withheld {
                if self.wave_running {
                    let batch = steps_for_frame(
                        &mut self.accumulator,
                        delta,
                        self.editor.document.presentation.simulation_speed,
                        dt,
                        self.step_budget.ceiling,
                    );
                    // Spike (2026-10-05, `docs/spikes/funfern-solver-submission.md`):
                    // a fixed batch, so the pass spills past the interval
                    // whatever the controller would have done.
                    let batch = fixed_batch().map_or(batch, |steps| FrameBatch {
                        steps,
                        ceiling_bound: false,
                    });
                    let admitted = steps_with_gpu_backpressure(
                        request.stats().retired_steps(),
                        request.requested_steps(),
                        batch.steps,
                    );
                    if admitted > 0 {
                        request.request_steps(admitted);
                    }
                    note.asked = batch.steps;
                    note.admitted = admitted;
                    self.last_batch = FrameBatch {
                        steps: admitted,
                        ceiling_bound: batch.ceiling_bound,
                    };
                    self.display_cadence.record_batch(admitted);
                } else if self.wave_step {
                    request.request_steps(1);
                    self.display_cadence.record_batch(1);
                    self.wave_step = false;
                }
                self.speed_reached =
                    hold_rate(self.speed_reached, self.steps_per_second * dt, delta);
                self.speed_short = speed_short(
                    self.speed_reached,
                    self.editor.document.presentation.simulation_speed,
                    self.speed_short,
                );
            }
            request.set_pacing_note(note);
        } else {
            request.set_pacing_note(PacingNote::default());
        }
        self.completed_steps = request.stats().completed_steps();
        self.gpu_status = request.stats().status();
        self.grid_filter_refused = request.grid_scale_filter_refused();
        self.gpu_dispatches = request.stats().dispatches();
        let processed_event = request.stats().processed_event();
        if processed_event != self.canonical_event_observed {
            self.canonical_event_observed = processed_event;
            let rejection = request.stats().event_rejection();
            if rejection != 0 {
                self.unseen_error = true;
                self.notify(format!(
                    "Canonical event {processed_event} was rejected without changing the accepted state (failure code {rejection})"
                ));
            }
        }
        self.canonical_gpu_bytes = request
            .manifest()
            .map(|manifest| manifest.bytes.steady_bytes());
        self.step_backlog = request
            .requested_steps()
            .saturating_sub(self.completed_steps);
        // Full physical snapshots are for AMR and energy diagnostics. The
        // vector overlay has its own compact display-rate GPU sampler, and
        // the integrated-field view its own stream of `r` alone.
        request.set_integrated_display(commands, self.integrated_field_shown());
        if self.full_snapshot_requested.elapsed().as_secs_f64() >= 0.25
            && request.request_full_state_readback()
        {
            self.full_snapshot_requested = Instant::now();
        }
        if let Some(active) = self.runtime.active()
            && display.generation == request.generation()
            && display.primary_flux.len() == active.canonical_operator.degrees_of_freedom()
        {
            if display.full_readbacks != self.energy_readback
                && display.full_readback_at == display.readbacks
                && self.energy_updated.elapsed().as_secs_f64() >= 0.25
            {
                let primary = display
                    .primary_flux
                    .iter()
                    .map(|value| f64::from(*value))
                    .collect::<Vec<_>>();
                let complementary = display
                    .complementary_flux
                    .iter()
                    .map(|value| Point2::new(f64::from(value[0]), f64::from(value[1])))
                    .collect::<Vec<_>>();
                // The gap and pole lanes alone; an oscillator's `r` sits after
                // them and is stored energy of its own.
                let auxiliary = display
                    .history_auxiliary()
                    .iter()
                    .map(|value| f64::from(*value))
                    .collect::<Vec<_>>();
                let integrated = display
                    .integrated_field()
                    .iter()
                    .map(|value| f64::from(*value))
                    .collect::<Vec<_>>();
                let breakdown = canonical_energy_breakdown(
                    &active.canonical_operator,
                    &primary,
                    &complementary,
                    &auxiliary,
                )
                .ok();
                // A field-dependent medium stores the Legendre dual of its
                // co-energy, not the quadratic form, and an oscillator's
                // integrated field stores `Σ m₀V(r)`; the gap and pole stores
                // are unchanged by either.
                let energy = match (breakdown, super::stored_energy_view(active, display)) {
                    // A snapshot without its `r` has no energy to report,
                    // rather than one missing the restoring store.
                    (Some(breakdown), Some((temporal, runtime, time)))
                        if !temporal.has_restoring()
                            || integrated.len() == temporal.base().degrees_of_freedom() =>
                    {
                        temporal
                            .energy_at(&primary, &complementary, time, &runtime)
                            .ok()
                            .map(|bulk| {
                                bulk + breakdown.thin_gap
                                    + breakdown.outgoing
                                    + temporal.restoring_energy(&integrated)
                            })
                    }
                    (_, Some(_)) => None,
                    (breakdown, _) => breakdown.map(CanonicalEnergyBreakdown::total),
                };
                if let Some(energy) = energy.filter(|energy| energy.is_finite() && *energy > 0.0) {
                    // Full snapshots run throughout the simulation, whether
                    // AMR is currently enabled or not. Preserve that history
                    // so AMR enabled after a pulse does not establish its
                    // "run" peak from the numerical tail it is meant to ignore.
                    self.amr_energy_peak = self.amr_energy_peak.max(energy);
                }
                self.wave_energy = energy;
                self.nonlinear_strength = super::field_law_view(active, display)
                    .and_then(|(temporal, runtime, time)| {
                        temporal
                            .nonlinear_strength(&primary, &complementary, time, &runtime)
                            .ok()
                    })
                    .unwrap_or_default();
                self.energy_readback = display.full_readbacks;
                self.energy_updated = Instant::now();
            }
            if let Some(clock) = display.clock {
                self.read_device_clock(clock);
            }
        }
        self.accumulate_step_rate(
            request.generation(),
            self.completed_steps,
            self.rate_started.elapsed().as_secs_f64(),
        );
    }
    /// Banks the progress the solver made since the previous frame and closes
    /// the averaging window when it is full.
    ///
    /// Accepted-step totals survive a handover, while a fresh install/reset may
    /// start a new generation at zero. The first observation of any generation
    /// is therefore a baseline, not progress: counting its absolute total
    /// credited the whole run again after every adaptive handover and could
    /// leave the reported real-time rate falsely high for many seconds.
    /// Each Switch material's accepted ramp, read from the runtime bank the
    /// latest full snapshot carries, at that snapshot's clock.
    fn refresh_switch_states(
        &mut self,
        display: &CanonicalGpuDisplay,
        request: &CanonicalGpuRequest,
    ) {
        self.switch_states.clear();
        let Some(temporal) = self
            .runtime
            .active()
            .and_then(|active| active.canonical_temporal_operator.clone())
        else {
            return;
        };
        let runtime = running_material_runtime(display, request, temporal.initial_runtime());
        let time = display.clock.map_or(0.0, |clock| clock.absolute_seconds);
        for material in &self.editor.document.model.accepted.materials {
            if material.mass_law.alternate.is_none() && material.stiffness_law.alternate.is_none() {
                continue;
            }
            let Some(record) = runtime
                .records()
                .iter()
                .find(|record| record.material() == material.id)
            else {
                continue;
            };
            let switch = record.switch();
            let now = switch.blend(time).unwrap_or(switch.target_blend());
            self.switch_states
                .push((material.id, switch.target_blend(), now));
        }
    }

    /// Sends one material's Switch towards whichever state it is not headed
    /// for, over the material's authored ramp. The device stamps the start;
    /// nothing is written to the document.
    fn send_material_switch(
        &mut self,
        active: &PreparedTopology,
        display: &CanonicalGpuDisplay,
        request: &mut CanonicalGpuRequest,
        assets: &mut Assets<ShaderBuffer>,
        material: MaterialId,
    ) {
        let Some(temporal) = &active.canonical_temporal_operator else {
            self.message = "This medium has no Switch to throw".into();
            return;
        };
        let runtime = running_material_runtime(display, request, temporal.initial_runtime());
        let Some(ramp) = self
            .editor
            .document
            .model
            .accepted
            .materials
            .iter()
            .find(|candidate| candidate.id == material)
            .map(|found| found.switch_ramp)
        else {
            return;
        };
        let heading = self.switch_targets.get(&material).copied().or_else(|| {
            runtime
                .records()
                .iter()
                .find(|record| record.material() == material)
                .map(|record| record.switch().target_blend() >= 0.5)
        });
        let Some(heading) = heading else {
            self.message = "This material has no Switch in the running generation".into();
            return;
        };
        self.canonical_event_serial = self
            .canonical_event_serial
            .max(request.stats().processed_event())
            .saturating_add(1)
            .max(1);
        match CanonicalGpuLiveEvent::temporal_switch_in(
            &runtime,
            material,
            !heading,
            ramp,
            self.canonical_event_serial,
        )
        .map_err(|error| format!("{error:?}"))
        .and_then(|event| {
            request
                .queue_live_event(assets, event)
                .map_err(str::to_owned)
        }) {
            Ok(()) => {
                self.switch_targets.insert(material, !heading);
            }
            Err(error) => self.message = error,
        }
    }

    /// A device failure during a run pauses it at the last accepted step,
    /// names the reason in the log and lights the badge. Nothing is clipped
    /// or reset: Run or Step clears the latch and retries from that step, and
    /// an edit prepares a generation that hands off from it.
    fn supervise_solver_fault(
        &mut self,
        request: &mut CanonicalGpuRequest,
        assets: &mut Assets<ShaderBuffer>,
        commands: &mut Commands,
    ) {
        let failure = request.stats().failure();
        if failure == 0 || self.uploading.is_some() {
            if failure == 0 {
                self.solver_fault = None;
            }
            return;
        }
        if self.solver_fault != Some(failure) {
            self.solver_fault = Some(failure);
            self.wave_running = false;
            self.wave_step = false;
            self.message = format!(
                "Paused at the last accepted step: {} (failure code {failure}). Edit the scene, \
                 or press Run to retry from that step",
                canonical_failure_description(failure)
            );
            // The run stopped on its own, and the status line keeps the
            // reason only until the next message.
            let description = canonical_failure_description(failure);
            let mut reason = description[..1].to_uppercase();
            reason.push_str(&description[1..]);
            self.raise_notice(
                "Simulation paused",
                format!(
                    "{reason} (failure code {failure}).\n\nThe field is held at the last accepted step. \
                     Edit the scene, or press Run to retry from that step."
                ),
            );
            self.unseen_error = true;
            return;
        }
        if (self.wave_running || self.wave_step) && request.clear_failure(assets, commands).is_ok()
        {
            self.solver_fault = None;
            self.message = "Resumed from the last accepted step".into();
        }
    }

    pub(super) fn accumulate_step_rate(&mut self, generation: u64, completed: u64, elapsed: f64) {
        if self.rate_generation != generation {
            self.rate_generation = generation;
            self.rate_steps = completed;
        } else {
            self.rate_window_steps += completed.saturating_sub(self.rate_steps);
            self.rate_steps = completed;
        }
        if elapsed >= STEP_RATE_WINDOW {
            self.steps_per_second = self.rate_window_steps as f64 / elapsed;
            self.rate_window_steps = 0;
            self.rate_started = Instant::now();
        }
    }
}

/// What to do with a source edit whose live patch the accepted generation would
/// not take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LiveEventFallback {
    /// The generation is busy. The candidate stays ready and a later frame
    /// tries the same patch again.
    Retry,
    /// The generation cannot take this patch at all. The edit goes through a
    /// whole prepared generation instead, which is what it did before the patch
    /// existed.
    Pack,
    /// The generation cannot take the patch and the edit was not prepared in a
    /// form that can be packed either. Nothing can carry it, so say so rather
    /// than fail further on with a reason that names neither cause.
    Refuse,
}

/// A refused live patch is a reason to take the slow path, never a reason to
/// lose the edit.
///
/// Source moves and weight edits were routed onto the live-patch fast path in
/// "Avoid full handoffs for sources and measurements"; the day after, temporal
/// material events arrived and gated every patch kind whose composition with a
/// driven medium had not been tested. A driven scene therefore took the fast
/// path and was then refused, and the refusal rejected the prepared candidate -
/// so moving a source on a pumped medium reported a failed preparation and
/// dropped the edit. Neither change is wrong on its own.
///
/// Packing is the fallback because it is the path these edits took before the
/// patch existed - but only for a candidate prepared with the handoff maps a
/// pack needs. A source-only preparation deliberately builds none of them, on
/// the promise that a live patch will carry the edit; where that promise cannot
/// be kept, the preparation now makes a whole generation instead, so the last
/// case should not arise. It is kept truthful rather than trusted, because what
/// it replaced failed later on with a reason that named neither the refusal nor
/// the missing maps.
/// Spike (2026-10-05): `FUNFERN_FIXED_BATCH=N` asks N steps every running
/// frame in place of the controller's batch; the GPU backpressure still
/// bounds what is admitted.
fn fixed_batch() -> Option<u64> {
    static FIXED: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *FIXED.get_or_init(|| std::env::var("FUNFERN_FIXED_BATCH").ok()?.parse().ok())
}

fn live_event_fallback(error: &str, packable: bool) -> LiveEventFallback {
    if error == "another canonical transaction is pending" {
        LiveEventFallback::Retry
    } else if packable {
        LiveEventFallback::Pack
    } else {
        LiveEventFallback::Refuse
    }
}

/// The material runtime the running generation stepped with: the display's
/// bank once it belongs to that generation, and the authored runtime before.
/// A Reset installs a new generation while the display still holds the old
/// one's readbacks, whose Switches may sit at the alternate.
fn running_material_runtime(
    display: &CanonicalGpuDisplay,
    request: &CanonicalGpuRequest,
    authored: CanonicalMaterialRuntimeState,
) -> CanonicalMaterialRuntimeState {
    if display.generation != request.generation() {
        return authored;
    }
    display
        .accepted_material_runtime(&authored)
        .unwrap_or(authored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical_gpu::CANONICAL_FAILURE_NON_FINITE;

    /// The reported regression: moving a continuous source on a pumped medium
    /// reported "this event has not passed its Stage 7 temporal composition
    /// gate" as a failed preparation, and the edit was lost.
    #[test]
    fn a_refused_source_patch_packs_instead_of_losing_the_edit() {
        // The gate a driven generation puts on patch kinds it has not composed
        // with yet, verbatim from `queue_live_event`.
        assert_eq!(
            live_event_fallback(
                "this event has not passed its Stage 7 temporal composition gate",
                true
            ),
            LiveEventFallback::Pack
        );
        // Anything else the generation will not take is equally a reason to
        // take the slow path rather than to drop the edit.
        for refusal in [
            "live canonical event does not match the active generation",
            "a temporal material event requires a temporal generation",
            "live canonical event serial is stale or the solver has failed",
            "canonical GPU is not installed",
        ] {
            assert_eq!(
                live_event_fallback(refusal, true),
                LiveEventFallback::Pack,
                "{refusal}"
            );
            // The same refusal on an edit that was never prepared to be packed
            // says so, rather than failing later on missing handoff maps.
            assert_eq!(
                live_event_fallback(refusal, false),
                LiveEventFallback::Refuse,
                "{refusal}"
            );
        }

        // Except a generation that is merely busy: the same patch is worth
        // trying again next frame, and packing would throw away a fast path
        // that is about to be available.
        for packable in [true, false] {
            assert_eq!(
                live_event_fallback("another canonical transaction is pending", packable),
                LiveEventFallback::Retry
            );
        }
    }

    /// Handover generations preserve the accepted-step total. Their first
    /// observation establishes a baseline instead of re-crediting the run;
    /// genuinely new steps on either side remain in the same rate window.
    #[test]
    fn the_step_rate_survives_a_handover() {
        let mut state = Playground::default();
        // Four frames of a settled generation, then the window closes.
        state.accumulate_step_rate(7, 0, 0.0);
        for (frame, completed) in [(1, 300_u64), (2, 600), (3, 900), (4, 1200)] {
            state.accumulate_step_rate(7, completed, if frame == 4 { 0.5 } else { 0.1 });
        }
        assert_eq!(state.steps_per_second, 2400.0);
        assert_eq!(state.rate_window_steps, 0, "a closed window starts empty");

        // A handover mid-window preserves its total, and the steps already
        // banked stay without the preserved total being counted a second time.
        state.accumulate_step_rate(7, 1500, 0.1);
        assert_eq!(state.rate_window_steps, 300);
        state.accumulate_step_rate(8, 1500, 0.1);
        assert_eq!(
            state.rate_window_steps, 300,
            "the preserved total is a baseline and the old progress is kept"
        );
        state.accumulate_step_rate(8, 1700, 0.5);
        assert_eq!(state.steps_per_second, 1000.0);

        // A window holding only the frames either side of another handover.
        state.accumulate_step_rate(8, 5000, 0.1);
        state.accumulate_step_rate(9, 5000, 0.5);
        assert_eq!(state.steps_per_second, 3300.0 / 0.5);
        assert!(
            state.steps_per_second > 0.0,
            "a handover is not a stall in the solver"
        );

        // A fresh install may reset the counter; its first observation is also
        // only a baseline, and subsequent progress is measured normally.
        state.accumulate_step_rate(10, 0, 0.1);
        state.accumulate_step_rate(10, 250, 0.5);
        assert_eq!(state.steps_per_second, 500.0);
    }

    /// Reset starts the medium again at its authored laws, so the Switch
    /// direction last sent no longer says where it is headed: the next press
    /// heads for the alternate again rather than for the base it is at. Until
    /// the reset generation's first readback, the display still holds the old
    /// generation's bank, which says alternate too.
    #[test]
    fn reset_forgets_where_a_switch_was_sent() {
        let mut world = World::new();
        world.init_resource::<Assets<ShaderBuffer>>();
        let mut request = CanonicalGpuRequest::default();
        let mut display = CanonicalGpuDisplay::default();
        let mut recorders = WaveGpuRequest::default();
        let vector = VectorOverlayDisplay::default();
        let mut frame = |state: &mut Playground,
                         request: &mut CanonicalGpuRequest,
                         display: &CanonicalGpuDisplay| {
            world.resource_scope(|world, mut assets: Mut<Assets<ShaderBuffer>>| {
                let mut queue = bevy::ecs::world::CommandQueue::default();
                let mut commands = Commands::new(&mut queue, world);
                state.refresh_runtime(
                    request,
                    display,
                    &mut recorders,
                    &vector,
                    &mut assets,
                    &mut commands,
                    1.0 / 60.0,
                );
            });
        };

        let mut state = Playground::default();
        let material = state.editor.document.model.draft.materials[0].clone();
        let switch = law_presets()
            .iter()
            .find(|preset| preset.name == "Switchable medium" && preset.row == LawPresetRow::Mass)
            .unwrap();
        state
            .editor
            .update_material(apply_law_preset(switch, &material).unwrap())
            .unwrap();
        settle(&mut state.editor);
        activate(&mut state);
        state.requested_revision = Some(state.editor.revision);
        state.requested_edge = state.editor.document.presentation.mesh_edge;
        state.reset_requested = true;
        state.wave_running = false;
        frame(&mut state, &mut request, &display);
        let installed = request.generation();

        state.request_material_switch();
        frame(&mut state, &mut request, &display);
        assert_eq!(state.switch_targets.get(&material.id), Some(&true));
        let records = state
            .runtime
            .active()
            .and_then(|active| active.canonical_temporal_operator.as_ref())
            .unwrap()
            .initial_runtime()
            .records()
            .len();
        display.hold_switched_snapshot(installed, records);

        state.reset_requested = true;
        frame(&mut state, &mut request, &display);
        assert!(!state.reset_requested && request.generation() > installed);
        assert!(state.switch_targets.is_empty());

        state.request_material_switch();
        frame(&mut state, &mut request, &display);
        assert_eq!(
            state.switch_targets.get(&material.id),
            Some(&true),
            "the press after Reset heads for the alternate"
        );
        assert_eq!(
            state.switch_states.first().map(|(_, target, _)| *target),
            Some(0.0),
            "the reset medium reads as at its base"
        );
    }

    /// A medium switched to twice its mass, with the snapshot that says so.
    fn doubled_mass_medium() -> (Arc<PreparedTopology>, CanonicalGpuDisplay) {
        let mut state = Playground::default();
        let mut material = state.editor.document.model.draft.materials[0].clone();
        material.mass_law.alternate = Some(ScalarField::constant(2.0));
        state.editor.update_material(material).unwrap();
        settle(&mut state.editor);
        let active = activate(&mut state);
        let temporal = active.canonical_temporal_operator.as_ref().unwrap();
        assert!(!temporal.has_field_laws() && !temporal.has_restoring());
        let mut display = CanonicalGpuDisplay::default();
        display.hold_switched_snapshot(1, temporal.initial_runtime().records().len());
        (active, display)
    }

    /// Reported: a switched or pumped linear medium was painted through its
    /// authored mass, since only a field law sent the picture through the
    /// time-driven maps. At twice the mass, the authored unit field is 0.5.
    #[test]
    fn a_switched_medium_paints_the_field_at_its_current_mass() {
        let (active, display) = doubled_mass_medium();
        let flux = active
            .canonical_operator
            .primary_mass()
            .iter()
            .map(|mass| *mass as f32)
            .collect::<Vec<_>>();
        let field = super::super::primary_field(&active, &display, &flux);
        assert!(!field.is_empty());
        for value in field {
            assert!((value - 0.5).abs() < 1.0e-6, "painted {value}");
        }
    }

    /// Reported: the stored energy of a switched or pumped linear medium was
    /// the authored quadratic form, which also fed the AMR energy peak. The
    /// authored unit field at twice the mass is half the field, and half the
    /// energy.
    #[test]
    fn a_switched_medium_stores_the_energy_of_its_current_mass() {
        let (active, display) = doubled_mass_medium();
        let operator = &active.canonical_operator;
        let primary = operator.primary_mass().to_vec();
        let complementary = vec![Point2::default(); operator.complementary_degrees_of_freedom()];
        let auxiliary = operator.thin_gap_samples().len()
            + operator
                .outgoing_boundary()
                .map_or(0, |boundary| boundary.auxiliary_count());
        let authored =
            canonical_energy_breakdown(operator, &primary, &complementary, &vec![0.0; auxiliary])
                .unwrap()
                .total();
        let (temporal, runtime, time) = super::super::stored_energy_view(&active, &display)
            .expect("a switched medium's energy is read through its runtime");
        let energy = temporal
            .energy_at(&primary, &complementary, time, &runtime)
            .unwrap();
        assert!(
            (energy - 0.5 * authored).abs() <= 1.0e-9 * authored,
            "stored {energy}, authored {authored}"
        );
    }

    /// Reported: a rejected handoff kept the candidate's step. The upload took
    /// it when it started, and the rejection left it in place under the
    /// retained generation - mispacing the solver, and making the speed ceiling
    /// see a step it had not asked for and request the rejected revision again,
    /// past the guard that stops a failed revision being retried every frame.
    #[test]
    fn a_rejected_handoff_keeps_the_accepted_step() {
        let mut world = World::new();
        world.init_resource::<Assets<ShaderBuffer>>();
        let mut request = CanonicalGpuRequest::default();
        let display = CanonicalGpuDisplay::default();
        let mut recorders = WaveGpuRequest::default();
        let vector = VectorOverlayDisplay::default();
        let mut frame = |state: &mut Playground, request: &mut CanonicalGpuRequest| {
            world.resource_scope(|world, mut assets: Mut<Assets<ShaderBuffer>>| {
                let mut queue = bevy::ecs::world::CommandQueue::default();
                let mut commands = Commands::new(&mut queue, world);
                state.refresh_runtime(
                    request,
                    &display,
                    &mut recorders,
                    &vector,
                    &mut assets,
                    &mut commands,
                    1.0 / 60.0,
                );
            });
        };

        // A running generation, installed by Reset. Paused, so that no steps
        // are owed and the handoff finds the complete-step boundary it needs.
        let mut state = with_baffles(&[]);
        let accepted = activate(&mut state).bundle.token;
        state.requested_revision = Some(state.editor.revision);
        state.requested_edge = state.editor.document.presentation.mesh_edge;
        state.reset_requested = true;
        state.wave_running = false;
        frame(&mut state, &mut request);
        let step = state.uploaded_time_step;
        assert!(step > 0.0);

        // A finer mesh wants a smaller step, so its candidate hands off.
        state.editor.document.presentation.mesh_edge *= 0.5;
        for _ in 0..100_000 {
            if state.uploading.is_some() {
                break;
            }
            frame(&mut state, &mut request);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let candidate = state
            .uploading
            .as_ref()
            .expect("the handoff began")
            .time_step;
        assert_eq!(
            request.handoff_outcome(),
            CanonicalGpuHandoffOutcome::Pending
        );
        assert!(
            (candidate / step - 1.0).abs() > TIME_STEP_HYSTERESIS,
            "the candidate must want a different step: {candidate:e} against {step:e}"
        );
        assert_eq!(
            state.solver_time_step(),
            step,
            "until the device publishes it, the accepted generation is stepping"
        );

        request.reject_handoff(CANONICAL_FAILURE_NON_FINITE);
        frame(&mut state, &mut request);
        assert!(state.uploading.is_none());
        assert_eq!(state.runtime.active().unwrap().bundle.token, accepted);
        assert_eq!(state.solver_time_step(), step);

        frame(&mut state, &mut request);
        assert!(
            !state.preparation_in_progress()
                && state
                    .runtime
                    .last_error()
                    .is_some_and(|failure| failure.message.contains("handoff rejected")),
            "the rejected revision is not prepared again"
        );
    }
}
