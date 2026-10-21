//! Automatic mesh adaptation: draining the background indicator, deciding
//! whether to refine or coarsen, and driving the adaptation transaction.

use crate::canonical_gpu::{CanonicalGpuDisplay, CanonicalGpuRequest};
use crate::wave_gpu::WaveDisplay;
use bevy::platform::time::Instant;
use bevy::prelude::*;
use funfern_core::*;
use std::sync::Arc;

use super::*;

impl Playground {
    pub(super) fn drain_background_amr(&mut self) {
        let events = self
            .background_amr
            .as_mut()
            .map_or_else(Vec::new, BackgroundAmrWorker::drain);
        for event in events {
            #[cfg(all(target_arch = "wasm32", feature = "browser-threads"))]
            match &event {
                BackgroundAmrEvent::WorkerStarted => {
                    crate::set_browser_amr_worker_status("active");
                }
                BackgroundAmrEvent::Progress { .. } => {
                    crate::set_browser_amr_job_status("progress");
                }
                BackgroundAmrEvent::Finished { .. } => {
                    crate::set_browser_amr_job_status("finished");
                }
                BackgroundAmrEvent::Stopped => {
                    crate::set_browser_amr_worker_status("stopped");
                }
            }
            if matches!(event, BackgroundAmrEvent::Stopped) {
                self.replace_stopped_amr_worker();
                continue;
            }
            if let BackgroundAmrEvent::Finished { result, .. } = event {
                match *result {
                    BackgroundAmrResult::Indicator(result) => {
                        self.amr_indicator_completed = Some(result);
                    }
                    BackgroundAmrResult::Adaptation(result) => {
                        self.amr_adaptation_completed = Some(result);
                    }
                }
            }
        }
    }

    /// An AMR worker that stopped, which only a defect does, fails the
    /// estimate or adaptation it held and is replaced, with the diagnostics
    /// warning lit. Adaptation goes on with the new worker.
    fn replace_stopped_amr_worker(&mut self) {
        let Some(worker) = self.background_amr.take() else {
            return;
        };
        match worker.kind {
            Some(BackgroundAmrKind::Indicator) => {
                self.amr_indicator_source = None;
                self.amr_status = "estimate failed".into();
            }
            Some(BackgroundAmrKind::Adaptation) => {
                self.amr_adaptation_source = None;
                self.amr_status = "adaptation failed".into();
            }
            None => {}
        }
        self.amr_error = Some("the adaptation worker stopped and was restarted".into());
        self.unseen_error = true;
        self.background_amr = BackgroundAmrWorker::spawn();
    }

    /// Cancels every estimate and adaptation in flight, here and on the
    /// worker, and forgets their results.
    pub(super) fn stop_adaptation_work(&mut self) {
        self.cancel_background_amr();
        self.amr_indicator_job = None;
        self.amr_indicator_completed = None;
        self.amr_indicator_source = None;
        self.amr_adaptation_job = None;
        self.amr_adaptation_completed = None;
        self.amr_adaptation_source = None;
        self.amr_pending_state = None;
        self.amr_indicator_result = None;
        self.amr_shown_report = None;
        self.amr_coarsen_streak = 0;
    }

    /// Adaptation as at launch, for a scene whose mesh has no history: its
    /// work stopped, and what it knew of the outgoing mesh forgotten too.
    pub(super) fn reset_adaptation(&mut self) {
        self.stop_adaptation_work();
        self.amr_adaptation_state = None;
        self.amr_report = None;
        self.amr_error = None;
        self.amr_last_started = None;
        self.amr_last_analyzed_step = None;
        self.amr_energy_peak = 0.0;
        self.amr_status = "waiting for solution".into();
    }

    /// Whether an adaptation is computing a new mesh, here or on the worker,
    /// or holds one not yet handed to preparation.
    pub(super) fn adaptation_in_flight(&self) -> bool {
        self.amr_adaptation_job.is_some()
            || self.amr_adaptation_completed.is_some()
            || self.background_amr_kind() == Some(BackgroundAmrKind::Adaptation)
    }

    /// Whether an error estimate is running, here or on the worker.
    pub(super) fn estimate_in_flight(&self) -> bool {
        self.amr_indicator_job.is_some()
            || self.background_amr_kind() == Some(BackgroundAmrKind::Indicator)
    }

    pub(super) fn background_amr_kind(&self) -> Option<BackgroundAmrKind> {
        self.background_amr.as_ref().and_then(|worker| worker.kind)
    }

    pub(super) fn background_amr_phase(&self, kind: BackgroundAmrKind) -> Option<&'static str> {
        self.background_amr
            .as_ref()
            .filter(|worker| worker.kind == Some(kind))
            .and_then(|worker| worker.phase)
    }

    pub(super) fn start_indicator_job(&mut self, job: AmrIndicatorJob) {
        if let Some(mut worker) = self.background_amr.take() {
            match worker.submit(BackgroundAmrJob::Indicator(Box::new(job))) {
                Ok(()) => {
                    self.background_amr = Some(worker);
                    return;
                }
                Err(BackgroundAmrJob::Indicator(job)) => {
                    self.amr_indicator_job = Some(*job);
                    return;
                }
                Err(BackgroundAmrJob::Adaptation(_)) => unreachable!(),
            }
        }
        self.amr_indicator_job = Some(job);
    }

    pub(super) fn start_adaptation_job(&mut self, job: MeshAdaptationJob) {
        if let Some(mut worker) = self.background_amr.take() {
            match worker.submit(BackgroundAmrJob::Adaptation(Box::new(job))) {
                Ok(()) => {
                    self.background_amr = Some(worker);
                    return;
                }
                Err(BackgroundAmrJob::Adaptation(job)) => {
                    self.amr_adaptation_job = Some(*job);
                    return;
                }
                Err(BackgroundAmrJob::Indicator(_)) => unreachable!(),
            }
        }
        self.amr_adaptation_job = Some(job);
    }

    pub(super) fn cancel_background_amr(&mut self) {
        if let Some(worker) = &mut self.background_amr {
            worker.cancel();
        }
    }

    pub(super) fn refresh_amr(
        &mut self,
        request: &CanonicalGpuRequest,
        canonical: &CanonicalGpuDisplay,
        display: &WaveDisplay,
    ) {
        self.drain_background_amr();
        if !self.editor.document.presentation.adaptation.enabled {
            self.stop_adaptation_work();
            self.amr_status = "off".into();
            return;
        }
        // No candidate could be packed any more, and each one asked for
        // would hold its memory while it waited.
        if browser_background_failure().is_some() {
            self.stop_adaptation_work();
            self.amr_status = "stopped".into();
            return;
        }
        // An adaptation spans many frames while the user may remesh underneath
        // it. Its result is only meaningful against the mesh it started from,
        // so once another mesh is active the job is dropped here instead of
        // finishing and being rejected at the handoff as an error.
        let adaptation_in_progress = self.adaptation_in_flight();
        if adaptation_in_progress
            && self.amr_adaptation_source.is_some_and(|source| {
                self.runtime
                    .active()
                    .is_none_or(|active| active.mesh.mesh_revision != source)
            })
        {
            if let Some(worker) = &mut self.background_amr {
                worker.cancel_kind(BackgroundAmrKind::Adaptation);
            }
            self.amr_adaptation_job = None;
            self.amr_adaptation_completed = None;
            self.amr_adaptation_source = None;
            self.amr_pending_state = None;
            self.amr_status = "adaptation discarded: the mesh changed underneath it".into();
            return;
        }
        // A pause holds adaptation where it is: no estimate starts, no job
        // advances and no mesh is handed off, and whatever finished in the
        // background waits for Run. Each handoff clears the step cadence, so
        // without this a paused field was estimated, adapted and handed off
        // again and again, 14 times in 39 s on the parametric fiber.
        if !self.wave_running {
            self.amr_status = "paused".into();
            return;
        }
        if self.coordinator.uploading.is_some()
            || self.preparation_in_progress()
            || self.editor.editing()
        {
            self.amr_status = if adaptation_in_progress {
                "adapting mesh"
            } else {
                "geometry has priority"
            }
            .into();
            return;
        }

        if let Some(job) = &mut self.amr_adaptation_job {
            self.amr_status = job.phase().into();
            let started = Instant::now();
            let mut result = None;
            while result.is_none() && started.elapsed().as_secs_f64() < 0.002 {
                result = job.advance(64);
            }
            if let Some(result) = result {
                self.amr_adaptation_completed = Some(result);
                self.amr_adaptation_job = None;
            } else {
                return;
            }
        }
        if let Some(result) = self.amr_adaptation_completed.take() {
            self.amr_adaptation_source = None;
            match result {
                Ok(mut result) => {
                    if result.report.topology_changes == 0 {
                        // A scan that cannot apply any requested change must
                        // not manufacture a new mesh revision and force a GPU
                        // handoff. Still advance the adaptation generation so
                        // cooldown can expire on an otherwise unchanged mesh.
                        if let Some(active) = self.runtime.active() {
                            result.state.mesh_revision = active.mesh.mesh_revision;
                            self.amr_adaptation_state = Some(result.state);
                        }
                        self.amr_report = Some(result.report);
                        self.amr_pending_state = None;
                        self.amr_coarsen_streak = 0;
                        self.amr_status = "mesh unchanged; monitoring solution".into();
                        self.amr_error = None;
                        return;
                    }
                    self.amr_pending_state = Some(result.state);
                    self.amr_report = Some(result.report);
                    match self.runtime.request_adapted(
                        self.editor.revision,
                        &self.editor.document,
                        result.mesh,
                    ) {
                        Ok(_) => {
                            self.amr_status = "preparing adaptive handoff".into();
                            self.amr_error = None;
                            self.begin_handoff_timeline();
                        }
                        Err(error) => {
                            self.amr_pending_state = None;
                            self.amr_status = "adaptation discarded".into();
                            self.amr_error = Some(error);
                        }
                    }
                }
                Err(error) => {
                    self.amr_status = "adaptation failed".into();
                    self.amr_error = Some(error.to_string());
                }
            }
            return;
        }
        if self.background_amr_kind() == Some(BackgroundAmrKind::Adaptation) {
            self.amr_status = self
                .background_amr_phase(BackgroundAmrKind::Adaptation)
                .unwrap_or("Adapting mesh")
                .into();
            return;
        }

        if let Some(job) = &mut self.amr_indicator_job {
            self.amr_status = job.phase().into();
            let started = Instant::now();
            let mut result = None;
            while result.is_none() && started.elapsed().as_secs_f64() < 0.002 {
                result = job.advance(64);
            }
            if let Some(result) = result {
                self.amr_indicator_completed = Some(result);
                self.amr_indicator_job = None;
            } else {
                return;
            }
        }
        if let Some(result) = self.amr_indicator_completed.take() {
            let source = self.amr_indicator_source.take();
            let Some(source) = source else {
                self.amr_status = "discarded stale estimate".into();
                return;
            };
            let Some(active) = self.runtime.active().cloned() else {
                return;
            };
            if !source.is_current(active.bundle.token, request.generation()) {
                self.amr_status = "discarded stale estimate".into();
                return;
            }
            self.amr_last_analyzed_step = Some(source.accepted_step);
            let result = match result {
                Ok(result) => result,
                Err(error) => {
                    self.amr_status = "estimate failed".into();
                    self.amr_error = Some(error.to_string());
                    return;
                }
            };
            if result.report.total_energy.is_finite() && result.report.total_energy > 0.0 {
                self.amr_energy_peak = self.amr_energy_peak.max(result.report.total_energy);
            }
            let decision = adaptation_decision(&result.report, self.amr_target_accuracy());
            self.amr_coarsen_streak = if decision == AmrDecision::Coarsen {
                self.amr_coarsen_streak.saturating_add(1)
            } else {
                0
            };
            self.amr_shown_report = Some(result.report.clone());
            self.amr_indicator_result = Some(result.clone());
            if decision == AmrDecision::Hold {
                self.amr_status = "mesh matches solution".into();
                self.amr_error = None;
                return;
            }
            if decision == AmrDecision::Coarsen && self.amr_coarsen_streak < 2 {
                self.amr_status = "confirming coarsening".into();
                return;
            }
            let Some(state) = self
                .amr_adaptation_state
                .clone()
                .filter(|state| state.mesh_revision == active.mesh.mesh_revision)
            else {
                self.amr_adaptation_state = Some(MeshAdaptationState::from_mesh(&active.mesh));
                self.amr_status = "initializing adaptation".into();
                return;
            };
            let target_revision = self.runtime.reserve_mesh_revision();
            let options = MeshAdaptationOptions {
                meshing: MeshingOptions {
                    curve_tolerance: (self.editor.document.presentation.adaptation.minimum_edge
                        * 0.02)
                        .min(1.5e-3),
                    target_edge_length: self.editor.document.presentation.adaptation.maximum_edge
                        / 1.05,
                    minimum_angle_degrees: 12.0,
                    max_vertices: 50_000,
                    max_triangles: 100_000,
                    max_refinement_steps: 50_000,
                },
                minimum_target_edge_length: self
                    .editor
                    .document
                    .presentation
                    .adaptation
                    .minimum_edge,
                maximum_target_edge_length: self
                    .editor
                    .document
                    .presentation
                    .adaptation
                    .maximum_edge,
                collapse_ratio: AMR_COARSEN_EDGE_RATIO,
                max_topology_changes: 512,
                max_refinement_changes: if decision == AmrDecision::Refine {
                    usize::MAX
                } else {
                    0
                },
                max_coarsening_changes: if decision == AmrDecision::Coarsen {
                    256
                } else {
                    0
                },
                max_work_units: 5_000_000,
                cooldown_generations: AMR_TOPOLOGY_COOLDOWN_GENERATIONS,
                ..Default::default()
            };
            let field: Arc<dyn MeshSizeField> =
                adaptation_field(&result, self.amr_target_accuracy());
            let job = MeshAdaptationJob::new_topology(
                active.mesh.clone(),
                &active.bundle.plan,
                state,
                target_revision,
                field,
                options,
            );
            self.amr_adaptation_source = Some(active.mesh.mesh_revision);
            self.start_adaptation_job(job);
            self.amr_status = "adapting mesh".into();
            return;
        }
        if self.background_amr_kind() == Some(BackgroundAmrKind::Indicator) {
            self.amr_status = self
                .background_amr_phase(BackgroundAmrKind::Indicator)
                .unwrap_or("Preparing AMR estimate")
                .into();
            return;
        }

        let Some(active) = self.runtime.active() else {
            self.amr_status = "waiting for solution".into();
            return;
        };
        let dofs = active.operator.degrees_of_freedom();
        if !request.ready()
            || display.generation != request.generation()
            || display.snapshot_current.len() != dofs
            || canonical.previous_primary_flux.len() != dofs
            || canonical.previous_complementary_flux.len()
                != active.canonical_operator.complementary_degrees_of_freedom()
            || canonical.previous_auxiliary.len() != canonical.auxiliary.len()
            || canonical.full_readback_at != canonical.readbacks
        {
            self.amr_status = "waiting for aligned readback".into();
            return;
        }
        let step = display.snapshot_completed_steps;
        // A mesh that has not stepped since its handoff has no step behind
        // its state: the other state slot is not the field a step earlier on
        // this mesh, and estimates of that pair read 24.7% on every mesh of a
        // paused run, flagging nearly every element.
        if !stepped_since_handoff(&mut self.amr_generation_start, request.generation(), step) {
            self.amr_status = "waiting for the new mesh to step".into();
            return;
        }
        if resident_filter_boundary(self.grid_filter_running(), step) {
            // The resident filter is accepted at this same solver step and
            // flips the state lanes once more. At that instant the other lane
            // is the pre-filter state, not the endpoint one `dt` earlier.
            // Let the next ordinary step restore the endpoint contract instead
            // of reporting the deliberate damping correction as wave error.
            self.amr_status = "waiting for post-filter endpoint".into();
            return;
        }
        if self
            .amr_last_analyzed_step
            .is_some_and(|previous| step < previous.saturating_add(8))
            || self
                .amr_last_started
                .is_some_and(|started| started.elapsed().as_secs_f64() < 0.75)
        {
            self.amr_status = "monitoring solution".into();
            return;
        }
        let snapshots = match indicator_snapshots(
            active,
            canonical,
            display,
            step,
            self.solver_time_step(),
            self.simulated_time(),
        ) {
            Ok(Some(snapshots)) => snapshots,
            Ok(None) => {
                self.amr_status = "waiting for aligned readback".into();
                return;
            }
            Err(error) => {
                self.amr_status = "canonical estimate failed".into();
                self.amr_error = Some(error.to_string());
                return;
            }
        };
        let IndicatorSnapshots {
            scalar: snapshot,
            canonical: canonical_snapshot,
            temporal,
        } = snapshots;
        let patterns = scene_resolution_demand(&active.bundle.authored);
        let job = SolutionIndicatorJob::new_topology(
            active.mesh.clone(),
            active.operator.clone(),
            &active.bundle.plan,
            active.bundle.model(),
            snapshot,
            SolutionIndicatorOptions {
                minimum_edge_length: self.editor.document.presentation.adaptation.minimum_edge,
                maximum_edge_length: self.editor.document.presentation.adaptation.maximum_edge,
                relative_tolerance: self.amr_target_accuracy(),
                elements_per_wavelength: self
                    .editor
                    .document
                    .presentation
                    .adaptation
                    .elements_per_wavelength,
                // What the field oscillates at: the estimator's spectral
                // scale, and a frequency the size rule resolves.
                forcing_frequency_hz: highest_forcing(
                    &active.bundle.authored,
                    active.point_source,
                    TimeSignal::carrier_frequency_hz,
                ),
                // How far a pulse's envelope widens that band, which the size
                // rule resolves too.
                band_edge_hz: highest_forcing(
                    &active.bundle.authored,
                    active.point_source,
                    TimeSignal::frequency_ceiling_hz,
                ),
                coarsen_ratio: AMR_COARSEN_EDGE_RATIO,
                dormant_below_energy: self.amr_energy_peak * DORMANT_ENERGY_RATIO,
                ..Default::default()
            },
        )
        .with_coefficient_patterns(patterns);
        let job = AmrIndicatorJob::with_canonical(
            job,
            active.mesh.clone(),
            active.canonical_operator.clone(),
            temporal,
            active.canonical_forcing.clone(),
            canonical_snapshot,
        );
        self.amr_indicator_source = Some(AmrIndicatorSource {
            topology: active.bundle.token,
            gpu_generation: request.generation(),
            accepted_step: step,
        });
        self.start_indicator_job(job);
        self.amr_last_started = Some(Instant::now());
        self.amr_status = "preparing estimate".into();
    }
}

/// The estimator's two snapshots of the generation's latest full readback,
/// and the runtime a driven one was stepped under.
pub(super) struct IndicatorSnapshots {
    pub(super) scalar: QuadraticSolutionSnapshot,
    pub(super) canonical: CanonicalIndicatorSnapshot,
    pub(super) temporal: Option<(
        Arc<CanonicalTemporalWaveOperator>,
        CanonicalMaterialRuntimeState,
    )>,
}

/// Builds the estimator's snapshots from what the displays hold of the latest
/// full readback, of the solver's `step`: the canonical one from its state
/// and the state a step earlier, the scalar one from its field and the rate
/// the generation's own operator gives that pair. `None` while the snapshot
/// cannot be read yet: a driven generation's runtime before a snapshot and a
/// clock agree on its epoch, or a step that is not positive. The caller has
/// checked the lanes belong to one generation and one snapshot.
pub(super) fn indicator_snapshots(
    active: &PreparedTopology,
    canonical: &CanonicalGpuDisplay,
    display: &WaveDisplay,
    step: u64,
    time_step: f64,
    simulated_time: f64,
) -> Result<Option<IndicatorSnapshots>, WaveError> {
    let dofs = active.operator.degrees_of_freedom();
    // A driven estimate is taken under the runtime the solver stepped the
    // field with, decoded from the same snapshot. Until a snapshot and a
    // clock agree on its epoch there is nothing safe to decode it against.
    let temporal = match &active.canonical_temporal_operator {
        Some(operator) => match canonical.accepted_material_runtime(&operator.initial_runtime()) {
            Some(runtime) => Some((operator.clone(), runtime)),
            None => return Ok(None),
        },
        None => None,
    };
    let dt = time_step;
    if !dt.is_finite() || dt <= 0.0 {
        return Ok(None);
    }
    let time = canonical.clock.map_or(simulated_time, |clock| {
        clock.absolute_seconds
            + (step as f64 - clock.accepted_steps as f64) * f64::from(clock.time_step)
    });
    let canonical_snapshot = CanonicalIndicatorSnapshot {
        mesh_revision: active.mesh.mesh_revision,
        primary_flux: canonical
            .primary_flux
            .iter()
            .map(|value| f64::from(*value))
            .collect(),
        previous_primary_flux: canonical
            .previous_primary_flux
            .iter()
            .map(|value| f64::from(*value))
            .collect(),
        complementary_flux: canonical
            .complementary_flux
            .iter()
            .map(|value| Point2::new(f64::from(value[0]), f64::from(value[1])))
            .collect(),
        previous_complementary_flux: canonical
            .previous_complementary_flux
            .iter()
            .map(|value| Point2::new(f64::from(value[0]), f64::from(value[1])))
            .collect(),
        auxiliary: canonical
            .history_auxiliary()
            .iter()
            .map(|value| f64::from(*value))
            .collect(),
        previous_auxiliary: canonical
            .previous_history_auxiliary()
            .iter()
            .map(|value| f64::from(*value))
            .collect(),
        // Gate O: the integrated field from the same snapshot, empty
        // without a restoring law.
        integrated_field: canonical
            .integrated_field()
            .iter()
            .map(|value| f64::from(*value))
            .collect(),
        previous_integrated_field: canonical
            .previous_integrated_field()
            .iter()
            .map(|value| f64::from(*value))
            .collect(),
        time,
        time_step: dt,
    };
    let displacement = display
        .snapshot_current
        .iter()
        .map(|value| f64::from(*value))
        .collect::<Vec<_>>();
    // A driven generation's rate is its own: the fixed operator's reads
    // the authored mass and force, and knows no loss law or restoring
    // force, which is not the field the solver stepped.
    let velocity = match &temporal {
        Some((operator, runtime)) => canonical_temporal_primary_rate(
            operator,
            &active.canonical_forcing,
            &canonical_snapshot,
            runtime,
        ),
        None => canonical_primary_rate(
            &active.canonical_operator,
            &active.canonical_forcing,
            &canonical_snapshot,
        ),
    }?;
    // The canonical estimator deliberately excludes the scalar strong cell
    // residual: applying the semidiscrete stiffness and feeding it back as
    // a pointwise acceleration produced a nonconvergent residual floor.
    // Keep shape-valid placeholders for the shared resumable job instead
    // of paying for an unused sparse multiply at every estimate.
    let scalar = QuadraticSolutionSnapshot {
        mesh_revision: active.mesh.mesh_revision,
        displacement,
        velocity,
        acceleration: vec![0.0; dofs],
        volume_acceleration: vec![0.0; dofs],
        time,
        time_step: dt,
    };
    Ok(Some(IndicatorSnapshots {
        scalar,
        canonical: canonical_snapshot,
        temporal,
    }))
}

/// Whether `generation` has taken a step since the estimator first saw it at
/// some step, recording that first sighting. The step count runs on across
/// handoffs, so a new generation shows the step its predecessor reached until
/// it takes one of its own.
fn stepped_since_handoff(start: &mut Option<(u64, u64)>, generation: u64, step: u64) -> bool {
    match start {
        Some((seen, first)) if *seen == generation => step > *first,
        _ => {
            *start = Some((generation, step));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topology_editor::TopologyEditor;

    /// An adaptation spans many frames while the user may remesh underneath
    /// it. Once the active mesh is no longer the one the job started from, the
    /// job is dropped quietly instead of finishing and being rejected at the
    /// handoff as "Adapted mesh does not match the active topology".
    #[test]
    fn an_adaptation_of_a_replaced_mesh_is_discarded_without_an_error() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        let first = activate(&mut state);
        state.editor.document.presentation.adaptation.enabled = true;
        state.amr_adaptation_job = Some(MeshAdaptationJob::new_topology(
            first.mesh.clone(),
            &first.bundle.plan,
            MeshAdaptationState::from_mesh(&first.mesh),
            state.runtime.reserve_mesh_revision(),
            Arc::new(|_, _| 0.09),
            MeshAdaptationOptions::default(),
        ));
        state.amr_adaptation_source = Some(first.mesh.mesh_revision);

        state
            .editor
            .set_domain(DomainRect {
                max_x: 1.4,
                ..DomainRect::UNIT
            })
            .unwrap();
        settle(&mut state.editor);
        let second = activate(&mut state);
        assert_ne!(second.mesh.mesh_revision, first.mesh.mesh_revision);

        state.refresh_amr(
            &CanonicalGpuRequest::default(),
            &CanonicalGpuDisplay::default(),
            &WaveDisplay::default(),
        );
        assert!(state.amr_adaptation_job.is_none());
        assert!(state.amr_adaptation_source.is_none());
        assert_eq!(state.amr_error, None);
        assert!(
            state.amr_status.contains("discarded"),
            "status was {:?}",
            state.amr_status
        );
    }

    /// A pause holds an adaptation where it is: the job does not advance and
    /// nothing is handed off, however many frames pass, and Run picks it up
    /// where it stopped. Paused on the parametric fiber, adaptation used to
    /// run on, 14 handoffs in 39 s.
    #[test]
    fn a_pause_holds_adaptation_until_run() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        let active = activate(&mut state);
        state.editor.document.presentation.adaptation.enabled = true;
        state.amr_adaptation_job = Some(MeshAdaptationJob::new_topology(
            active.mesh.clone(),
            &active.bundle.plan,
            MeshAdaptationState::from_mesh(&active.mesh),
            state.runtime.reserve_mesh_revision(),
            Arc::new(|_, _| 0.1),
            MeshAdaptationOptions {
                minimum_target_edge_length: 0.01,
                maximum_target_edge_length: 1.0,
                max_refinement_changes: 0,
                max_coarsening_changes: 0,
                ..Default::default()
            },
        ));
        state.amr_adaptation_source = Some(active.mesh.mesh_revision);
        let refresh = |state: &mut Playground| {
            state.refresh_amr(
                &CanonicalGpuRequest::default(),
                &CanonicalGpuDisplay::default(),
                &WaveDisplay::default(),
            )
        };

        state.wave_running = false;
        for _ in 0..1_000 {
            refresh(&mut state);
        }
        assert!(state.amr_adaptation_job.is_some(), "a paused job advanced");
        assert!(state.amr_report.is_none());
        assert_eq!(state.amr_status, "paused");

        state.wave_running = true;
        for _ in 0..10_000 {
            refresh(&mut state);
            if state.amr_adaptation_job.is_none() {
                break;
            }
        }
        assert!(
            state.amr_adaptation_job.is_none(),
            "Run did not resume the job"
        );
        assert!(state.amr_report.is_some());
    }

    /// A generation is estimated only once it has stepped: the step count
    /// runs on across a handoff, so the new mesh first shows its
    /// predecessor's step, and an estimate there pairs its state with a slot
    /// that is not the step before it.
    #[test]
    fn a_new_mesh_is_estimated_only_once_it_has_stepped() {
        let mut start = None;
        assert!(!stepped_since_handoff(&mut start, 1, 0));
        assert!(!stepped_since_handoff(&mut start, 1, 0));
        assert!(stepped_since_handoff(&mut start, 1, 8));
        // A handoff at step 687; paused, it stays there.
        assert!(!stepped_since_handoff(&mut start, 2, 687));
        for _ in 0..10 {
            assert!(!stepped_since_handoff(&mut start, 2, 687));
        }
        assert!(stepped_since_handoff(&mut start, 2, 688));
        // And the next handoff waits again.
        assert!(!stepped_since_handoff(&mut start, 3, 700));
    }

    #[test]
    fn an_unchanged_adaptation_does_not_request_a_handoff() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        let active = activate(&mut state);
        state.editor.document.presentation.adaptation.enabled = true;
        state.amr_adaptation_job = Some(MeshAdaptationJob::new_topology(
            active.mesh.clone(),
            &active.bundle.plan,
            MeshAdaptationState::from_mesh(&active.mesh),
            state.runtime.reserve_mesh_revision(),
            Arc::new(|_, _| 0.1),
            MeshAdaptationOptions {
                minimum_target_edge_length: 0.01,
                maximum_target_edge_length: 1.0,
                max_refinement_changes: 0,
                max_coarsening_changes: 0,
                ..Default::default()
            },
        ));
        state.amr_adaptation_source = Some(active.mesh.mesh_revision);

        for _ in 0..10_000 {
            state.refresh_amr(
                &CanonicalGpuRequest::default(),
                &CanonicalGpuDisplay::default(),
                &WaveDisplay::default(),
            );
            if state.amr_adaptation_job.is_none() {
                break;
            }
        }

        assert!(state.amr_adaptation_job.is_none());
        assert!(state.runtime.ready().is_none());
        assert_eq!(
            state.runtime.active().unwrap().mesh.mesh_revision,
            active.mesh.mesh_revision
        );
        assert_eq!(state.amr_report.as_ref().unwrap().topology_changes, 0);
        assert_eq!(
            state.amr_adaptation_state.as_ref().unwrap().mesh_revision,
            active.mesh.mesh_revision
        );
        assert_eq!(state.amr_error, None);
    }

    /// The estimate has no notion of enough on its own. Its step down is
    /// clamped, so an element it cannot satisfy - a boundary the field
    /// disagrees with, the grid-scale leftovers of a wave that has passed -
    /// asks for the same refinement however loose the target is, and walks to
    /// the smallest element allowed. The accuracy target is what answers that,
    /// and it answers for the whole field at once; the floors it does not
    /// answer for at all.
    #[test]
    fn the_accuracy_target_and_deadband_choose_one_adaptation_direction() {
        let report = |error, limit, coarsen, global| SolutionIndicatorReport {
            refine_candidates: error + limit,
            error_refine_candidates: error,
            limit_refine_candidates: limit,
            coarsen_candidates: coarsen,
            global_indicator: global,
            ..Default::default()
        };
        assert_eq!(
            adaptation_decision(&report(2000, 0, 0, 0.2), 0.12),
            AmrDecision::Refine
        );
        assert_eq!(
            adaptation_decision(&report(2000, 0, 0, 0.05), 0.12),
            AmrDecision::Hold
        );
        // The same estimate, asked for more: still running.
        assert_eq!(
            adaptation_decision(&report(2000, 0, 0, 0.05), 0.04),
            AmrDecision::Refine
        );
        // A forced wavelength is carried whatever the error reads.
        assert_eq!(
            adaptation_decision(&report(0, 2000, 2000, 0.0), 0.12),
            AmrDecision::Refine
        );
        // A handful of elements is noise, as it always was.
        assert_eq!(
            adaptation_decision(&report(3, 0, 0, 0.9), 0.12),
            AmrDecision::Hold
        );
        // Coarsening waits below a broad deadband, even if local edges ask.
        assert_eq!(
            adaptation_decision(&report(0, 0, 2000, 0.10), 0.12),
            AmrDecision::Hold
        );
        assert_eq!(
            adaptation_decision(&report(0, 0, 2000, 0.05), 0.12),
            AmrDecision::Coarsen
        );
        // Refinement wins when both local candidate sets are populated; one
        // transaction never yanks the topology in both directions.
        assert_eq!(
            adaptation_decision(&report(2000, 0, 2000, 0.2), 0.12),
            AmrDecision::Refine
        );
    }

    /// The estimate line names the size rule when it is what refines, so an
    /// error under target beside a mesh still refining explains itself: the
    /// parametric fiber read 2% against 12% while its pattern floor refined.
    /// An error refinement, or a hold, is not the size rule's.
    #[test]
    fn the_size_rule_is_told_apart_from_the_accuracy_target() {
        let report = |error, limit, global| SolutionIndicatorReport {
            refine_candidates: error + limit,
            error_refine_candidates: error,
            limit_refine_candidates: limit,
            global_indicator: global,
            ..Default::default()
        };
        assert!(size_rule_refines(&report(0, 2000, 0.02), 0.12));
        assert!(size_rule_refines(&report(2000, 2000, 0.05), 0.12));
        assert!(!size_rule_refines(&report(2000, 2000, 0.2), 0.12));
        assert!(!size_rule_refines(&report(2000, 0, 0.2), 0.12));
        assert!(!size_rule_refines(&report(0, 3, 0.02), 0.12));
    }

    /// A refinement the size rule asks for alone applies the limits' field,
    /// so nothing the estimate would merely like finer rides along with it;
    /// one the accuracy target asks for applies the whole field, limits and
    /// estimate together. A quiet field reads no error, and a largest element
    /// under the mesh's edges binds every element as a limit.
    #[test]
    fn a_size_rule_refinement_applies_only_the_limits() {
        let scene = Scene::initial();
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
        let dofs = operator.degrees_of_freedom();
        let mut job = SolutionIndicatorJob::new(
            mesh,
            operator,
            scene,
            QuadraticSolutionSnapshot {
                mesh_revision: 1,
                displacement: vec![0.0; dofs],
                velocity: vec![0.0; dofs],
                acceleration: vec![0.0; dofs],
                volume_acceleration: vec![0.0; dofs],
                time: 0.01,
                time_step: 0.01,
            },
            SolutionIndicatorOptions {
                maximum_edge_length: 0.1,
                ..Default::default()
            },
        );
        let mut result = loop {
            if let Some(result) = job.advance(1 << 16) {
                break result.unwrap();
            }
        };
        assert!(result.report.limit_refine_candidates >= 4);
        assert!(Arc::ptr_eq(
            &adaptation_field(&result, 0.12),
            &result.limit_field
        ));
        result.report.error_refine_candidates = 2000;
        result.report.global_indicator = 0.2;
        assert!(Arc::ptr_eq(&adaptation_field(&result, 0.12), &result.field));
    }

    /// An estimate owns a copied solution snapshot. Accepted-state maintenance
    /// may continue while the CPU walks that snapshot; only a
    /// topology/generation handoff makes it stale.
    #[test]
    fn a_live_gpu_event_does_not_disown_an_amr_snapshot() {
        let token = TopologyToken {
            document_revision: 4,
            topology_revision: 3,
            mesh_generation: 2,
        };
        let source = AmrIndicatorSource {
            topology: token,
            gpu_generation: 7,
            accepted_step: 120,
        };
        assert!(source.is_current(token, 7));
        assert!(!source.is_current(token, 8));
        assert!(!source.is_current(
            TopologyToken {
                mesh_generation: 3,
                ..token
            },
            7
        ));
    }

    #[test]
    fn the_accuracy_control_starts_on_the_medium_preset() {
        let state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        assert_eq!(
            amr_accuracy_preset_name(
                state
                    .editor
                    .document
                    .presentation
                    .adaptation
                    .accuracy_percent
            ),
            "Medium"
        );
        assert!((state.amr_target_accuracy() - 0.12).abs() < 1.0e-12);
        for (percent, name) in AMR_ACCURACY_PRESETS {
            assert_eq!(amr_accuracy_preset_name(percent), name);
        }
        assert_eq!(amr_accuracy_preset_name(9.0), "Custom");
    }

    /// Committing a mesh drops the estimate, and every adaptation commits one,
    /// so a reading that only exists while an estimate is in hand blinks out of
    /// the panel on every cycle and takes everything below it down a line.
    #[test]
    fn the_estimate_reading_holds_its_place_between_estimates() {
        let state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        assert!(state.amr_indicator_result.is_none());
        let line = state.amr_estimate_line();
        assert!(line.contains("target 12%"), "the line read {line:?}");
    }

    /// The size limits and the wavelength live in the fold at the bottom of the
    /// panel now, which is drawn after the adaptation section rather than
    /// inside it. Every one of them still has to drop an estimate in flight, or
    /// a change there is not felt until the next estimate happens to start.
    #[test]
    fn every_adaptation_setting_drops_an_estimate_in_flight() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        const WATCHED: [&str; 5] = [
            "adaptation itself",
            "the accuracy target",
            "elements per wavelength",
            "the smallest element",
            "the largest element",
        ];
        for (step, name) in WATCHED.into_iter().enumerate() {
            let before = state.amr_settings();
            match step {
                0 => {
                    state.editor.document.presentation.adaptation.enabled =
                        !state.editor.document.presentation.adaptation.enabled
                }
                1 => {
                    state
                        .editor
                        .document
                        .presentation
                        .adaptation
                        .accuracy_percent = 24.0
                }
                2 => {
                    state
                        .editor
                        .document
                        .presentation
                        .adaptation
                        .elements_per_wavelength = 9.0
                }
                3 => state.editor.document.presentation.adaptation.minimum_edge = 0.01,
                _ => state.editor.document.presentation.adaptation.maximum_edge = 0.2,
            }
            assert_ne!(state.amr_settings(), before, "{name} is not watched");
        }
    }

    /// The estimator's snapshots, built from the displays the way the app
    /// builds them, are the state the solver stepped. The CPU reference's two
    /// latest states, held on the canonical display as a readback decodes
    /// them with the clock run on past them, refresh the wave display, and the
    /// builder reads back the pair as held, the snapshot's time and the
    /// runtime the generation stepped under, and the field and its rate to
    /// 6e-8 and 1e-6, the f32 rounding; the estimate made from them is the
    /// reference's own pair's to 3e-8. On the default scene, whose walls are
    /// second-order outgoing so the pair carries their memory, and with the
    /// parametric pump on its medium, which drives the generation and moves
    /// its field's maps between the snapshot and the clock.
    #[test]
    fn the_estimators_snapshots_are_the_pair_the_displays_hold() {
        for pumped in [false, true] {
            let mut state = Playground::default();
            if pumped {
                let material = state.editor.document.model.draft.materials[0].clone();
                let pump = law_presets()
                    .iter()
                    .find(|preset| {
                        preset.name == "Parametric pump" && preset.row == LawPresetRow::Mass
                    })
                    .unwrap();
                state
                    .editor
                    .update_material(apply_law_preset(pump, &material).unwrap())
                    .unwrap();
                settle(&mut state.editor);
            }
            let active = activate(&mut state);
            assert!(active.canonical_operator.outgoing_boundary().is_some());
            assert_eq!(active.canonical_temporal_operator.is_some(), pumped);
            // A step the display's f32 clock holds exactly, so the
            // snapshot's time is the reference's.
            let dt = f64::from(active.recommended_time_step() as f32);
            let steps = 60_u32;
            let bump = active
                .canonical_operator
                .node_points()
                .iter()
                .map(|point| {
                    let offset = *point - Point2::new(0.0, -0.6);
                    (-offset.dot(offset) / (2.0 * 0.15 * 0.15)).exp()
                })
                .collect::<Vec<_>>();
            let forcing = &active.canonical_forcing;
            // Each state as the lanes hold it: primary, complementary, and the
            // gap and wall histories, then the reference's field, its time
            // and, for a driven generation, its runtime.
            type Lanes = (Vec<f64>, Vec<Point2>, Vec<f64>);
            let (current, previous, field, time, runtime): (
                Lanes,
                Lanes,
                Vec<f64>,
                f64,
                Option<CanonicalMaterialRuntimeState>,
            ) = match &active.canonical_temporal_operator {
                None => {
                    let operator = &active.canonical_operator;
                    let lanes = |state: &CanonicalWaveState| {
                        let CanonicalAuxiliaryState::Linear(auxiliaries) = state.auxiliaries()
                        else {
                            panic!("a linear generation's auxiliaries")
                        };
                        (
                            state.primary_flux().to_vec(),
                            state.complementary_flux().to_vec(),
                            auxiliaries
                                .thin_gap_jump()
                                .iter()
                                .chain(auxiliaries.outgoing_z())
                                .copied()
                                .collect::<Vec<_>>(),
                        )
                    };
                    let mut state = CanonicalWaveState::zero(operator, dt).unwrap();
                    state.apply_primary_pulse(operator, forcing, &bump).unwrap();
                    let mut previous = state.clone();
                    for _ in 0..steps {
                        previous = state.clone();
                        state.step_with_forcing(operator, forcing).unwrap();
                    }
                    (
                        lanes(&state),
                        lanes(&previous),
                        state.primary_field(operator).unwrap(),
                        state.time(),
                        None,
                    )
                }
                Some(temporal) => {
                    let lanes = |state: &CanonicalTemporalWaveState| {
                        (
                            state.primary_flux().to_vec(),
                            state.complementary_flux().to_vec(),
                            state
                                .thin_gap_jump()
                                .iter()
                                .chain(state.outgoing_pole_currents())
                                .chain(state.integrated_field())
                                .copied()
                                .collect::<Vec<_>>(),
                        )
                    };
                    let mut state = CanonicalTemporalWaveState::zero(temporal, dt).unwrap();
                    state.apply_primary_pulse(temporal, forcing, &bump).unwrap();
                    let mut previous = state.clone();
                    for _ in 0..steps {
                        previous = state.clone();
                        state.step_with_forcing(temporal, forcing).unwrap();
                    }
                    assert!(state.integrated_field().is_empty());
                    (
                        lanes(&state),
                        lanes(&previous),
                        temporal
                            .primary_field_at(state.primary_flux(), state.time(), state.runtime())
                            .unwrap(),
                        state.time(),
                        Some(state.runtime().clone()),
                    )
                }
            };
            let wall = active.canonical_operator.thin_gap_samples().len();
            assert!(
                current.2[wall..].iter().any(|z| z.abs() > 1.0e-6),
                "the pulse reached no wall"
            );

            let narrow = |lanes: &Lanes| {
                (
                    lanes
                        .0
                        .iter()
                        .map(|value| *value as f32)
                        .collect::<Vec<_>>(),
                    lanes
                        .1
                        .iter()
                        .map(|value| [value.x as f32, value.y as f32])
                        .collect::<Vec<_>>(),
                    lanes
                        .2
                        .iter()
                        .map(|value| *value as f32)
                        .collect::<Vec<_>>(),
                )
            };
            let (now, before) = (narrow(&current), narrow(&previous));
            let request = CanonicalGpuRequest::default();
            let mut canonical = CanonicalGpuDisplay::default();
            canonical.generation = request.generation();
            let records = active
                .canonical_temporal_operator
                .as_ref()
                .map_or(0, |temporal| temporal.initial_runtime().records().len());
            canonical.hold_full_snapshot_lanes(
                steps,
                dt as f32,
                (&now.0, &now.1, &now.2),
                (&before.0, &before.1, &before.2),
                0,
                records,
            );
            // The clock has run on past the snapshot, as it has by the time a
            // full readback lands; a pumped medium's maps move with it.
            let mut clock = canonical.clock.unwrap();
            clock.accepted_steps += 37;
            clock.step_in_epoch += 37;
            clock.absolute_seconds += 37.0 * dt;
            canonical.clock = Some(clock);
            let mut display = WaveDisplay::default();
            super::super::refresh_canonical_wave_display(
                Some(&active),
                &canonical,
                &request,
                &mut display,
            );
            assert_eq!(display.snapshot_completed_steps, u64::from(steps));
            let built = indicator_snapshots(
                &active,
                &canonical,
                &display,
                display.snapshot_completed_steps,
                dt,
                0.0,
            )
            .unwrap()
            .expect("a full snapshot at its clock reads");

            // The pair as held, and the snapshot's time.
            let widen = |values: &[f32]| -> Vec<f64> {
                values.iter().map(|value| f64::from(*value)).collect()
            };
            let held = |lanes: &(Vec<f32>, Vec<[f32; 2]>, Vec<f32>)| {
                (
                    widen(&lanes.0),
                    lanes
                        .1
                        .iter()
                        .map(|value| Point2::new(f64::from(value[0]), f64::from(value[1])))
                        .collect::<Vec<_>>(),
                    widen(&lanes.2),
                )
            };
            let (held_now, held_before) = (held(&now), held(&before));
            let snapshot = &built.canonical;
            assert_eq!(snapshot.mesh_revision, active.mesh.mesh_revision);
            assert_eq!(snapshot.primary_flux, held_now.0);
            assert_eq!(snapshot.previous_primary_flux, held_before.0);
            assert_eq!(snapshot.complementary_flux, held_now.1);
            assert_eq!(snapshot.previous_complementary_flux, held_before.1);
            assert_eq!(snapshot.auxiliary, held_now.2);
            assert_eq!(snapshot.previous_auxiliary, held_before.2);
            assert!(snapshot.integrated_field.is_empty());
            assert_eq!(snapshot.time_step, dt);
            assert!(
                (snapshot.time - time).abs() < 1.0e-12,
                "{} for {time}",
                snapshot.time
            );
            assert_eq!(built.scalar.time, snapshot.time);
            assert_eq!(
                built.temporal.as_ref().map(|(_, runtime)| runtime),
                runtime.as_ref()
            );

            // The field and its rate, against the reference's own.
            let reference = CanonicalIndicatorSnapshot {
                mesh_revision: active.mesh.mesh_revision,
                primary_flux: current.0.clone(),
                previous_primary_flux: previous.0.clone(),
                complementary_flux: current.1.clone(),
                previous_complementary_flux: previous.1.clone(),
                auxiliary: current.2.clone(),
                previous_auxiliary: previous.2.clone(),
                integrated_field: Vec::new(),
                previous_integrated_field: Vec::new(),
                time,
                time_step: dt,
            };
            let rate = match (&active.canonical_temporal_operator, &runtime) {
                (Some(temporal), Some(runtime)) => {
                    canonical_temporal_primary_rate(temporal, forcing, &reference, runtime)
                }
                _ => canonical_primary_rate(&active.canonical_operator, forcing, &reference),
            }
            .unwrap();
            let relative = |read: &[f64], expected: &[f64]| {
                let scale = expected
                    .iter()
                    .fold(0.0_f64, |most, value| most.max(value.abs()));
                read.iter()
                    .zip(expected)
                    .map(|(read, expected)| (read - expected).abs())
                    .fold(0.0_f64, f64::max)
                    / scale
            };
            let field_error = relative(&built.scalar.displacement, &field);
            let rate_error = relative(&built.scalar.velocity, &rate);

            // The estimate, from the built snapshots and from the reference's.
            let estimate = |scalar: QuadraticSolutionSnapshot,
                            canonical: CanonicalIndicatorSnapshot| {
                let mut job = AmrIndicatorJob::with_canonical(
                    SolutionIndicatorJob::new_topology(
                        active.mesh.clone(),
                        active.operator.clone(),
                        &active.bundle.plan,
                        active.bundle.model(),
                        scalar,
                        SolutionIndicatorOptions::default(),
                    ),
                    active.mesh.clone(),
                    active.canonical_operator.clone(),
                    built.temporal.clone(),
                    active.canonical_forcing.clone(),
                    canonical,
                );
                loop {
                    if let Some(result) = job.advance(1 << 16) {
                        break result.unwrap().report.global_indicator;
                    }
                }
            };
            let from_displays = estimate(built.scalar.clone(), built.canonical.clone());
            let from_reference = estimate(
                QuadraticSolutionSnapshot {
                    mesh_revision: active.mesh.mesh_revision,
                    displacement: field.clone(),
                    velocity: rate.clone(),
                    acceleration: vec![0.0; field.len()],
                    volume_acceleration: vec![0.0; field.len()],
                    time,
                    time_step: dt,
                },
                reference,
            );
            let estimate_error = (from_displays - from_reference).abs() / from_reference;
            eprintln!(
                "pumped {pumped}: field {field_error:.2e}, rate {rate_error:.2e}, \
                 estimate {from_displays:.6e} for {from_reference:.6e} ({estimate_error:.2e})"
            );
            assert!(field_error < 5.0e-7, "field {field_error:e}");
            assert!(rate_error < 5.0e-6, "rate {rate_error:e}");
            assert!(estimate_error < 1.0e-6, "estimate {estimate_error:e}");
        }
    }
}
