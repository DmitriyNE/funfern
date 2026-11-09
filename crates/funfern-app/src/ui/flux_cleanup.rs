//! Taking away the stationary flux a handoff leaves.
//!
//! A handoff carries the complementary flux through its L2 projection, and on
//! a new element that straddles old ones the projection of a potential's flux
//! is partly the flux of no potential: a stationary part, which pushes no node,
//! never moves and never leaves. Measured on the default box, an adaptive
//! refinement left 0.13% of the energy so and a remesh 0.56%; added up over a
//! run it held a box's mesh fully refined after the wave had gone, the error
//! estimate reading it as error (2026-11-03). A local map that keeps a
//! potential's flux one grew back and forth between two meshes without
//! bound, so the projection stays and its stationary part is taken away
//! after: being inert, it is the same at every step of its generation, so it
//! is solved for from the first readback and subtracted whenever the event
//! lands, which changes nothing of how the field moves.

use crate::canonical_gpu::{
    CanonicalGpuDisplay, CanonicalGpuLiveEvent, CanonicalGpuRequest, LiveEventRefusal,
};
use crate::topology_runtime::PreparedTopology;
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use funfern_core::*;
use funfern_protocol::decisions::{CleanupSeen, cleanup_may_send};
use std::sync::Arc;

use super::*;

/// A stationary part under this share of the flux's energy is left: what the
/// solve's own tolerance leaves of a flux with none is some 1e-12 of it, and a
/// handoff's is 1e-3 and more.
pub(super) const FLUX_CLEANUP_FLOOR: f64 = 1.0e-9;

/// Where taking away a handoff's stationary flux stands, for the generation
/// the handoff published.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct FluxCleanup {
    pub(super) generation: u64,
    pub(super) stage: FluxCleanupStage,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum FluxCleanupStage {
    /// Waiting for a full readback of the generation and a free worker.
    Due,
    /// Solving from that readback, on the worker or here.
    Solving,
    /// Found: the increment that takes it away, waiting for the device.
    Found(Vec<Point2>),
    /// Sent with this serial, until a readback shows it landed.
    Sent(u32),
    /// Landed by this accepted step. A zero-duration event flips the state
    /// lanes, so at that step the other lane holds the flux before it, not
    /// the state a step earlier: estimates wait for a readback past it.
    Landed(u64),
}

impl FluxCleanup {
    /// What a handoff onto `active`, published as `generation`, leaves to
    /// do. A driven generation's flux moves with its runtime and holds no
    /// part that stays put, and one whose flux loses energy has its
    /// stationary part decay as the event waits; neither is cleaned.
    pub(super) fn after_handoff(active: &PreparedTopology, generation: u64) -> Option<Self> {
        let fixed = active.canonical_temporal_operator.is_none();
        let lossless = active
            .canonical_operator
            .complementary_loss_rate()
            .iter()
            .all(|rate| *rate == 0.0);
        (fixed && lossless).then_some(Self {
            generation,
            stage: FluxCleanupStage::Due,
        })
    }
}

/// The accepted step of the latest readback, where it is a full one of
/// `generation`.
fn full_readback_step(canonical: &CanonicalGpuDisplay, generation: u64) -> Option<u64> {
    (canonical.generation == generation && canonical.full_readback_at == canonical.readbacks)
        .then_some(canonical.clock?.accepted_steps)
}

impl Playground {
    /// Whether the running generation's handoff flux is still to be taken
    /// away. Estimates wait for it: what they read of it is error the next
    /// event removes.
    pub(super) fn flux_cleanup_pending(&self, generation: u64) -> bool {
        self.flux_cleanup
            .as_ref()
            .is_some_and(|cleanup| cleanup.generation == generation)
    }

    /// Advances taking away the stationary flux the last handoff left: the
    /// solve started from the generation's first full readback once the
    /// worker is free, its answer kept as the event to send, and the event
    /// seen to land. The event itself is sent with the other live events.
    pub(super) fn refresh_flux_cleanup(
        &mut self,
        request: &CanonicalGpuRequest,
        canonical: &CanonicalGpuDisplay,
    ) {
        self.drain_background_amr();
        let Some(cleanup) = self.flux_cleanup.clone() else {
            return;
        };
        if cleanup.generation != request.generation() {
            self.drop_flux_cleanup();
            return;
        }
        match cleanup.stage {
            FluxCleanupStage::Due => self.start_flux_cleanup(request, canonical),
            FluxCleanupStage::Solving => self.finish_flux_cleanup(),
            FluxCleanupStage::Found(_) => {}
            FluxCleanupStage::Sent(serial) => {
                let stats = request.stats();
                if stats.processed_event() < serial {
                    return;
                }
                if stats.processed_event() == serial && stats.event_rejection() != 0 {
                    self.flux_cleanup_failed(format!(
                        "the device refused it (failure code {})",
                        stats.event_rejection()
                    ));
                    return;
                }
                // A full readback the device took once it had processed the
                // event says whether it landed: the maintenance serial it
                // records, or a later event's with it refused.
                let Some(step) = full_readback_step(canonical, cleanup.generation) else {
                    return;
                };
                if canonical.runtime_serials[3] >= serial {
                    if let Some(cleanup) = &mut self.flux_cleanup {
                        cleanup.stage = FluxCleanupStage::Landed(step);
                    }
                } else if canonical.event_result[0] >= serial {
                    self.flux_cleanup_failed("the device refused it".into());
                }
            }
            FluxCleanupStage::Landed(landed) => {
                if full_readback_step(canonical, cleanup.generation)
                    .is_some_and(|step| step > landed)
                {
                    self.flux_cleanup = None;
                }
            }
        }
    }

    fn start_flux_cleanup(
        &mut self,
        request: &CanonicalGpuRequest,
        canonical: &CanonicalGpuDisplay,
    ) {
        // The worker runs one job; an estimate or adaptation on it finishes
        // first, and none starts while this waits.
        if self.background_amr_kind().is_some()
            || self.amr_indicator_job.is_some()
            || self.amr_adaptation_job.is_some()
        {
            return;
        }
        let Some(active) = self.runtime.active() else {
            return;
        };
        let samples = active.canonical_operator.complementary_degrees_of_freedom();
        if canonical.generation != request.generation()
            || canonical.complementary_flux.len() != samples
            || canonical.full_readback_at != canonical.readbacks
        {
            return;
        }
        let flux = canonical
            .complementary_flux
            .iter()
            .map(|[x, y]| Point2::new(f64::from(*x), f64::from(*y)))
            .collect();
        let job =
            match CanonicalStationaryFluxJob::new(Arc::clone(&active.canonical_operator), flux) {
                Ok(job) => job,
                Err(error) => {
                    self.protocol_log.push(ProtocolNote::CleanupSolved(false));
                    self.flux_cleanup_failed(error.to_string());
                    return;
                }
            };
        if let Some(cleanup) = &mut self.flux_cleanup {
            cleanup.stage = FluxCleanupStage::Solving;
        }
        // As an estimate is: a worker whose channel is gone is dropped and
        // the job runs here.
        if let Some(mut worker) = self.background_amr.take() {
            match worker.submit(BackgroundAmrJob::Stationary(Box::new(job))) {
                Ok(()) => {
                    self.background_amr = Some(worker);
                    return;
                }
                Err(BackgroundAmrJob::Stationary(job)) => {
                    self.flux_cleanup_job = Some(*job);
                    return;
                }
                Err(_) => unreachable!("the worker hands back the job it was given"),
            }
        }
        self.flux_cleanup_job = Some(job);
    }

    fn finish_flux_cleanup(&mut self) {
        if let Some(job) = &mut self.flux_cleanup_job {
            let started = Instant::now();
            let mut result = None;
            while result.is_none() && started.elapsed().as_secs_f64() < 0.002 {
                result = job.advance(64);
            }
            if result.is_some() {
                self.flux_cleanup_job = None;
            }
            self.flux_cleanup_completed = result;
        }
        let Some(result) = self.flux_cleanup_completed.take() else {
            return;
        };
        match result {
            Ok(found) if found.energy > FLUX_CLEANUP_FLOOR * found.flux_energy => {
                self.protocol_log.push(ProtocolNote::CleanupSolved(true));
                let increment = found.stationary.iter().map(|value| *value * -1.0).collect();
                if let Some(cleanup) = &mut self.flux_cleanup {
                    cleanup.stage = FluxCleanupStage::Found(increment);
                }
            }
            Ok(_) => {
                self.protocol_log.push(ProtocolNote::CleanupSolved(false));
                self.flux_cleanup = None;
            }
            Err(error) => {
                self.protocol_log.push(ProtocolNote::CleanupSolved(false));
                self.flux_cleanup_failed(error.to_string());
            }
        }
    }

    /// Sends the stationary flux's increment, found for the running
    /// generation, as a flux maintenance, once the device is free of the
    /// edits, pulses and Switch presses it waits behind
    /// ([`cleanup_may_send`]).
    pub(super) fn send_flux_cleanup(
        &mut self,
        active: &PreparedTopology,
        request: &mut CanonicalGpuRequest,
        assets: &mut Assets<ShaderBuffer>,
    ) {
        let Some(FluxCleanup {
            generation,
            stage: FluxCleanupStage::Found(increment),
        }) = &self.flux_cleanup
        else {
            return;
        };
        if !cleanup_may_send(CleanupSeen {
            current: *generation == request.generation(),
            transacting: self.coordinator.uploading.is_some()
                || self.coordinator.source_commit.is_some(),
            pressed: !self.coordinator.pending_pulses.is_empty()
                || !self.coordinator.pending_switches.is_empty(),
        }) {
            return;
        }
        let serial = self
            .coordinator
            .canonical_event_serial
            .max(request.stats().processed_event())
            .saturating_add(1)
            .max(1);
        let event =
            CanonicalGpuLiveEvent::flux_maintenance(&active.canonical_operator, increment, serial)
                .map_err(|error| LiveEventRefusal::Refused(format!("{error:?}")))
                .and_then(|event| request.queue_live_event(assets, event));
        self.protocol_log
            .push(ProtocolNote::FluxCleanup(event.is_ok()));
        match event {
            Ok(()) => {
                self.coordinator.canonical_event_serial = serial;
                if let Some(cleanup) = &mut self.flux_cleanup {
                    cleanup.stage = FluxCleanupStage::Sent(serial);
                }
            }
            Err(refusal) if refusal.busy() => {}
            Err(refusal) => self.flux_cleanup_failed(refusal.to_string()),
        }
    }

    /// Stops taking away the stationary flux, its solve too wherever it
    /// runs: its generation is gone, or the scene starts afresh.
    pub(super) fn drop_flux_cleanup(&mut self) {
        if let Some(worker) = &mut self.background_amr {
            worker.cancel_kind(BackgroundAmrKind::Stationary);
        }
        self.flux_cleanup = None;
        self.flux_cleanup_job = None;
        self.flux_cleanup_completed = None;
    }

    /// A cleanup that failed is a defect's: the flux stays as the handoff
    /// left it, which the run survives, and the diagnostics say so.
    fn flux_cleanup_failed(&mut self, reason: String) {
        self.drop_flux_cleanup();
        self.amr_error = Some(format!(
            "the stationary flux the last handoff left could not be taken away: {reason}"
        ));
        self.unseen_error = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topology_editor::TopologyEditor;
    use crate::wave_gpu::WaveDisplay;

    fn energy(operator: &CanonicalWaveOperator, flux: &[Point2]) -> f64 {
        operator
            .constitutive_samples()
            .iter()
            .zip(flux)
            .map(|(sample, value)| {
                0.5 * sample.integration_weight
                    * value.dot(sample.complementary_inverse.apply(*value))
            })
            .sum()
    }

    /// A pulse run on `active` twenty steps, `stationary` added to its flux
    /// as a handoff's carry leaves one, held on a canonical display as a full
    /// readback of it.
    fn held(
        active: &PreparedTopology,
        stationary: &[Point2],
    ) -> (CanonicalGpuRequest, CanonicalGpuDisplay) {
        let operator = &active.canonical_operator;
        let forcing = &active.canonical_forcing;
        let dt = f64::from(active.recommended_time_step() as f32);
        let bump = operator
            .node_points()
            .iter()
            .map(|point| {
                let offset = *point - Point2::new(0.1, -0.2);
                (-offset.dot(offset) / (2.0 * 0.15 * 0.15)).exp()
            })
            .collect::<Vec<_>>();
        let mut state = CanonicalWaveState::zero(operator, dt).unwrap();
        state.apply_primary_pulse(operator, forcing, &bump).unwrap();
        let mut previous = state.clone();
        for _ in 0..20 {
            previous = state.clone();
            state.step_with_forcing(operator, forcing).unwrap();
        }
        state
            .apply_complementary_increment(operator, stationary)
            .unwrap();
        previous
            .apply_complementary_increment(operator, stationary)
            .unwrap();
        let lanes = |state: &CanonicalWaveState| {
            let CanonicalAuxiliaryState::Linear(auxiliaries) = state.auxiliaries() else {
                panic!("a linear generation's auxiliaries")
            };
            (
                state
                    .primary_flux()
                    .iter()
                    .map(|value| *value as f32)
                    .collect::<Vec<_>>(),
                state
                    .complementary_flux()
                    .iter()
                    .map(|value| [value.x as f32, value.y as f32])
                    .collect::<Vec<_>>(),
                auxiliaries
                    .thin_gap_jump()
                    .iter()
                    .chain(auxiliaries.outgoing_z())
                    .map(|value| *value as f32)
                    .collect::<Vec<_>>(),
            )
        };
        let (now, before) = (lanes(&state), lanes(&previous));
        let request = CanonicalGpuRequest::default();
        let mut canonical = CanonicalGpuDisplay::default();
        canonical.generation = request.generation();
        canonical.hold_full_snapshot_lanes(
            20,
            dt as f32,
            (&now.0, &now.1, &now.2),
            (&before.0, &before.1, &before.2),
            0,
            0,
        );
        (request, canonical)
    }

    /// Refreshes until the solve has answered, on the worker or here.
    fn solved(
        state: &mut Playground,
        request: &CanonicalGpuRequest,
        canonical: &CanonicalGpuDisplay,
    ) {
        let started = std::time::Instant::now();
        while started.elapsed().as_secs() < 60 {
            state.refresh_flux_cleanup(request, canonical);
            if !state.flux_cleanup.as_ref().is_some_and(|cleanup| {
                matches!(
                    cleanup.stage,
                    FluxCleanupStage::Due | FluxCleanupStage::Solving
                )
            }) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("the stationary solve did not finish");
    }

    /// The stationary flux a handoff left is found from the generation's
    /// full readback, the increment that takes it away being what was added
    /// to the readback's f32 rounding, and no estimate starts until it is
    /// gone. Without a device to take it, the send is refused, which only a
    /// defect does, and the diagnostics say so.
    #[test]
    fn a_handoffs_stationary_flux_is_found_from_its_readback_and_holds_estimates_back() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            protocol_log: ProtocolLog::kept(),
            ..Playground::default()
        };
        let active = activate(&mut state);
        let operator = &active.canonical_operator;
        let smooth = operator
            .constitutive_samples()
            .iter()
            .map(|sample| {
                let p = sample.point;
                Point2::new((2.0 * p.x).sin(), (3.0 * p.y).cos() * p.x)
            })
            .collect::<Vec<_>>();
        let stationary = operator
            .stationary_complementary_component(&smooth)
            .unwrap()
            .into_iter()
            .map(|value| value * 0.05)
            .collect::<Vec<_>>();
        let (mut request, canonical) = held(&active, &stationary);
        state.flux_cleanup = FluxCleanup::after_handoff(&active, request.generation());
        assert_eq!(
            state.flux_cleanup.as_ref().map(|cleanup| &cleanup.stage),
            Some(&FluxCleanupStage::Due)
        );

        state.editor.document.presentation.adaptation.enabled = true;
        state.wave_running = true;
        state.refresh_amr(&request, &canonical, &WaveDisplay::default());
        assert_eq!(state.amr_status, "clearing the handoff's stationary flux");
        assert!(state.amr_indicator_job.is_none());

        solved(&mut state, &request, &canonical);
        let Some(FluxCleanup {
            stage: FluxCleanupStage::Found(increment),
            ..
        }) = &state.flux_cleanup
        else {
            panic!(
                "nothing found: {:?}",
                state.flux_cleanup.map(|cleanup| cleanup.generation)
            );
        };
        let left = increment
            .iter()
            .zip(&stationary)
            .map(|(increment, added)| *increment + *added)
            .collect::<Vec<_>>();
        let share = energy(operator, &left) / energy(operator, &stationary);
        assert!(
            share < 1.0e-8,
            "the increment leaves {share:e} of the added flux"
        );
        assert!(state.flux_cleanup_pending(request.generation()));

        let mut assets = Assets::<ShaderBuffer>::default();
        state.send_flux_cleanup(&active, &mut request, &mut assets);
        assert_eq!(
            state.protocol_log.take().last(),
            Some(&ProtocolNote::FluxCleanup(false))
        );
        assert!(state.flux_cleanup.is_none());
        assert!(
            state
                .amr_error
                .as_deref()
                .is_some_and(|error| error.contains("stationary flux")),
            "{:?}",
            state.amr_error
        );
        assert!(state.unseen_error);
    }

    /// A readback with no stationary flux beyond its rounding leaves nothing
    /// to send, and a cleanup for a generation since replaced is dropped.
    #[test]
    fn nothing_is_sent_for_a_clean_flux_or_a_replaced_generation() {
        let mut state = Playground {
            editor: TopologyEditor::default(),
            ..Playground::default()
        };
        let active = activate(&mut state);
        let samples = active.canonical_operator.complementary_degrees_of_freedom();
        let (request, canonical) = held(&active, &vec![Point2::default(); samples]);
        state.flux_cleanup = FluxCleanup::after_handoff(&active, request.generation());
        solved(&mut state, &request, &canonical);
        assert_eq!(state.flux_cleanup, None);
        assert_eq!(state.amr_error, None);

        state.flux_cleanup = Some(FluxCleanup {
            generation: request.generation() + 1,
            stage: FluxCleanupStage::Due,
        });
        state.refresh_flux_cleanup(&request, &canonical);
        assert_eq!(state.flux_cleanup, None);
    }

    /// A driven generation's flux moves with its runtime and holds no part
    /// that stays put: its handoff leaves nothing to take away.
    #[test]
    fn a_driven_generation_is_not_cleaned() {
        let mut state = Playground::default();
        let material = state.editor.document.model.draft.materials[0].clone();
        let pump = law_presets()
            .iter()
            .find(|preset| preset.name == "Parametric pump" && preset.row == LawPresetRow::Mass)
            .unwrap();
        state
            .editor
            .update_material(apply_law_preset(pump, &material).unwrap())
            .unwrap();
        settle(&mut state.editor);
        let active = activate(&mut state);
        assert!(active.canonical_temporal_operator.is_some());
        assert_eq!(FluxCleanup::after_handoff(&active, 3), None);
    }
}
