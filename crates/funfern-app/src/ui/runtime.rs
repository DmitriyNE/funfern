//! Driving the solver runtime: preparation phases, the handoff transaction,
//! and the per-frame refresh that commits a candidate and advances the wave.

use crate::canonical_gpu::{
    CanonicalGpuClock, CanonicalGpuDisplay, CanonicalGpuHandoffOutcome, CanonicalGpuLiveEvent,
    CanonicalGpuPlan, CanonicalGpuRequest, canonical_failure_description,
};
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
        let options = MeshingOptions {
            target_edge_length: self.mesh_edge,
            curve_tolerance: (self.mesh_edge * 0.02).min(5e-4),
            ..MeshingOptions::default()
        };
        if !self.remesh_requested
            && self.requested_revision == Some(self.editor.revision)
            && self.requested_edge == self.mesh_edge
            && (self.preparation_in_progress()
                || self.runtime.active().is_some_and(|active| {
                    active.bundle.token.document_revision == self.editor.revision
                }))
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
        self.runtime.set_preserve_adaptation(self.amr_enabled);
        let require_solver_handoff = self.uploaded_time_step > 0.0
            && self.runtime.active().is_some_and(|active| {
                let wanted = paced_time_step(
                    active.canonical_operator.recommended_time_step(),
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
                self.requested_edge = self.mesh_edge;
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
            transferred: active.transfer.is_some(),
            exact_nodes: active
                .transfer
                .as_ref()
                .map_or(0, |transfer| transfer.exact_nodes()),
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
            candidate.canonical_operator.recommended_time_step(),
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
        request.set_grid_scale_filter(self.grid_scale_filter);
        self.finish_source_commit(request);
        // Starting another preparation mid-upload clears `runtime.ready`, and
        // would make the accepted GPU generation impossible to publish under
        // its immutable topology token. A later frame picks the edit up.
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
                candidate.canonical_operator.recommended_time_step(),
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
                                Err(error)
                                    if error == "another canonical transaction is pending" => {}
                                Err(error) => {
                                    self.runtime.reject_ready(token, error.clone());
                                    self.message = error;
                                }
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
            let upload = job.result.take().unwrap().and_then(|prepared| {
                if let Some(transfer) = prepared.transfer {
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
                    self.uploaded_time_step = dt;
                    self.handoff_upload = Some(Instant::now());
                    self.uploading = Some(Uploading {
                        token,
                        generation: if candidate.fresh {
                            request.generation()
                        } else {
                            request.generation().wrapping_add(1).max(1)
                        },
                        fresh: candidate.fresh,
                        degrees_of_freedom: candidate.canonical_operator.degrees_of_freedom(),
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
                match self.runtime.commit_ready(upload.token) {
                    Ok(active) => {
                        if upload.fresh {
                            self.accumulator = 0.0;
                            self.restart_probe_traces();
                            self.canonical_event_serial = 0;
                            self.canonical_event_observed = 0;
                            self.wave_energy = None;
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
                    active.canonical_operator.recommended_time_step(),
                    self.editor.document.presentation.simulation_speed,
                );
                let reset = CanonicalWaveState::zero(&active.canonical_operator, dt)
                    .map_err(|error| error.to_string())
                    .and_then(|state| {
                        CanonicalGpuPlan::compile_with_quadratic(
                            &active.canonical_operator,
                            &active.operator,
                            &state,
                            &active.canonical_forcing,
                            CanonicalGpuClock::initial(dt).map_err(|error| format!("{error:?}"))?,
                        )
                        .map_err(|error| format!("{error:?}"))
                    });
                if let Ok(plan) = reset {
                    request.install(assets, commands, plan);
                    self.reset_requested = false;
                    self.uploaded_time_step = dt;
                    self.sim_time_offset = 0.0;
                    self.canonical_event_serial = 0;
                    self.canonical_event_observed = 0;
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
            overlay_active.as_ref(),
            request.generation(),
        );
        if self.uploading.is_none()
            && let Some(active) = self.runtime.active().cloned()
            && probes_need_upload(self.probe_upload, active.bundle.token, request.generation())
        {
            self.configure_probes(recorders, assets, commands, &active);
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
            if !canonical_steps_withheld(packed_candidate_waiting, fresh_upload) {
                if self.wave_running {
                    let steps = steps_for_frame(
                        &mut self.accumulator,
                        delta,
                        self.editor.document.presentation.simulation_speed,
                        dt,
                    );
                    let admitted = steps_with_gpu_backpressure(
                        request.stats().completed_steps(),
                        request.requested_steps(),
                        steps,
                    );
                    if admitted > 0 {
                        request.request_steps(admitted);
                    }
                } else if self.wave_step {
                    request.request_steps(1);
                    self.wave_step = false;
                }
                self.speed_reached =
                    hold_rate(self.speed_reached, self.steps_per_second * dt, delta);
            }
        }
        self.completed_steps = request.stats().completed_steps();
        self.gpu_status = request.stats().status();
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
        // vector overlay has its own compact display-rate GPU sampler.
        let full_snapshot_interval = 0.25;
        if self.full_snapshot_requested.elapsed().as_secs_f64() >= full_snapshot_interval
            && request.request_full_state_readback(commands)
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
                let auxiliary = display
                    .auxiliary
                    .iter()
                    .map(|value| f64::from(*value))
                    .collect::<Vec<_>>();
                let energy = canonical_energy_breakdown(
                    &active.canonical_operator,
                    &primary,
                    &complementary,
                    &auxiliary,
                )
                .ok()
                .map(CanonicalEnergyBreakdown::total);
                if let Some(energy) = energy.filter(|energy| energy.is_finite() && *energy > 0.0) {
                    // Full snapshots run throughout the simulation, whether
                    // AMR is currently enabled or not. Preserve that history
                    // so AMR enabled after a pulse does not establish its
                    // "run" peak from the numerical tail it is meant to ignore.
                    self.amr_energy_peak = self.amr_energy_peak.max(energy);
                }
                self.wave_energy = energy;
                self.energy_readback = display.full_readbacks;
                self.energy_updated = Instant::now();
            }
            if let Some(clock) = display.clock {
                self.sim_time_offset = clock.absolute_seconds
                    - self.completed_steps as f64 * f64::from(clock.time_step);
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
