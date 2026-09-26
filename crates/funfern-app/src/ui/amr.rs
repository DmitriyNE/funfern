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
            self.cancel_background_amr();
            self.amr_indicator_job = None;
            self.amr_indicator_completed = None;
            self.amr_indicator_source = None;
            self.amr_adaptation_job = None;
            self.amr_adaptation_completed = None;
            self.amr_adaptation_source = None;
            self.amr_pending_state = None;
            self.amr_indicator_result = None;
            self.amr_coarsen_streak = 0;
            self.amr_status = "off".into();
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
        if self.uploading.is_some() || self.preparation_in_progress() || self.editor.editing() {
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
            let field: Arc<dyn MeshSizeField> = result.field;
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
            || display.auxiliary.len() != dofs
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
        // A driven estimate is taken under the runtime the solver stepped the
        // field with, decoded from the same snapshot. Until a snapshot and a
        // clock agree on its epoch there is nothing safe to decode it against.
        let temporal = match &active.canonical_temporal_operator {
            Some(operator) => {
                match canonical.accepted_material_runtime(&operator.initial_runtime()) {
                    Some(runtime) => Some((operator.clone(), runtime)),
                    None => {
                        self.amr_status = "waiting for aligned readback".into();
                        return;
                    }
                }
            }
            None => None,
        };
        let dt = self.solver_time_step();
        let time = canonical.clock.map_or(self.simulated_time(), |clock| {
            clock.absolute_seconds + (step as f64 - f64::from(clock.accepted_steps)) * dt
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
        let velocity = match canonical_primary_rate(
            &active.canonical_operator,
            &active.canonical_forcing,
            &canonical_snapshot,
        ) {
            Ok(velocity) => velocity,
            Err(error) => {
                self.amr_status = "canonical estimate failed".into();
                self.amr_error = Some(error.to_string());
                return;
            }
        };
        // The canonical estimator deliberately excludes the scalar strong cell
        // residual: applying the semidiscrete stiffness and feeding it back as
        // a pointwise acceleration produced a nonconvergent residual floor.
        // Keep shape-valid placeholders for the shared resumable job instead
        // of paying for an unused sparse multiply at every estimate.
        let acceleration = vec![0.0; dofs];
        let volume_acceleration = vec![0.0; dofs];
        let Some(auxiliary) = aligned_indicator_auxiliary(display, &active.operator, dt, step)
        else {
            self.amr_status = "waiting for aligned readback".into();
            return;
        };
        let snapshot = QuadraticSolutionSnapshot {
            mesh_revision: active.mesh.mesh_revision,
            displacement,
            velocity,
            acceleration,
            auxiliary,
            volume_acceleration,
            time,
            time_step: dt,
        };
        let demand = scene_resolution_demand(&active.bundle.authored, active.point_source);
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
                // The estimator's spectral scale is what the field oscillates
                // at; only the size rule takes what the medium generates.
                forcing_frequency_hz: highest_forcing_frequency(
                    &active.bundle.authored,
                    active.point_source,
                ),
                resolved_frequency_hz: demand.frequency_hz,
                coefficient_wavelength: demand.coefficient_wavelength,
                coarsen_ratio: AMR_COARSEN_EDGE_RATIO,
                dormant_below_energy: self.amr_energy_peak * DORMANT_ENERGY_RATIO,
                ..Default::default()
            },
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use funfern_app::topology_editor::TopologyEditor;

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
}
