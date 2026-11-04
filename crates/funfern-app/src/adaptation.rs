//! Automatic mesh adaptation's decisions, apart from when they are made:
//! what an estimate of the running field is asked, what its answer decides,
//! what an adaptation transaction is asked to do, and the history those
//! decisions keep from one estimate to the next. The frame loop (`ui::amr`)
//! schedules this work across frames and a worker thread, and keeps it off
//! a mesh that changed underneath it; a test drives it by hand, estimate by
//! estimate, on a run stepped on the CPU (AMR-01 to AMR-04 of
//! `docs/spikes/funfern-testing-campaign-plan.md`, section 6l).

use crate::document::{AdaptationSettings, PresentationSettings};
use crate::topology_runtime::{PreparedTopology, TopologyRuntime};
use funfern_core::*;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Below one percent of the experiment's meaningful amplitude, automatic exposure is
/// more likely to reveal the slowly decaying numerical tail than useful wave
/// content. The saved-scene drain fixture plateaus at roughly 0.2--0.3% even
/// with resident damping, so the former 0.1% floor still normalized that tail.
pub const PRESENTATION_QUIET_AMPLITUDE_RATIO: f64 = 1.0e-2;
/// Energy is quadratic in amplitude. AMR's relative residual becomes dormant
/// below the energy counterpart of the presentation floor.
pub const DORMANT_ENERGY_RATIO: f64 =
    PRESENTATION_QUIET_AMPLITUDE_RATIO * PRESENTATION_QUIET_AMPLITUDE_RATIO;
/// The share of the experiment's peak energy a field's error is measured
/// against once it holds less. Below it the error a target allows stops
/// shrinking with the field: what a draining field leaves, divided by its
/// vanishing energy, read up to 0.27 against the 0.12 target as the ring at
/// a tenth of the speed faded from 1e-3 to 1e-4 of its peak, and refined 512
/// splits at every estimate until the field went dormant (AMR-02,
/// 2026-11-05).
pub const AMR_ERROR_FLOOR_RATIO: f64 = 1.0e-2;
/// Coarsening starts only well below the requested global error. The interval
/// between this threshold and the target is the controller's global deadband:
/// an estimate wobbling near the target cannot alternate the mesh direction.
pub const AMR_COARSEN_GLOBAL_RATIO: f64 = 0.65;
/// An edge must be substantially shorter than its requested size before it may
/// be collapsed. This is below half the 1.05 refinement threshold, so an edge
/// split just above that threshold is not immediately eligible for reversal.
pub const AMR_COARSEN_EDGE_RATIO: f64 = 0.45;
/// A vertex changed by one transaction must survive the immediately following
/// generation. Together with the size deadband this prevents split/collapse
/// ping-pong when a moving wave nudges a target across a threshold.
pub const AMR_TOPOLOGY_COOLDOWN_GENERATIONS: u64 = 2;
/// Estimates in a row that must ask to coarsen before a coarsening runs: one
/// taken just after a wave has passed may read a field the next one no
/// longer sees.
pub const AMR_COARSEN_CONFIRMATIONS: u8 = 2;
/// The fewest solver steps between the snapshots of two estimates.
pub const AMR_ESTIMATE_STEPS: u64 = 8;
/// The fewest wall-clock seconds between the starts of two estimates, which
/// at a simulation speed of one is as many simulated seconds.
pub const AMR_ESTIMATE_SECONDS: f64 = 0.75;
/// How many times over the field's reach ([`FieldReach`]) covers the time
/// until adaptation can next act: an element the field may get to is
/// refined back a cycle before it can, and a cycle up to this much longer
/// than the last still finds it refined.
pub const AMR_REACH_MARGIN: f64 = 2.0;
/// An element holds the field when its energy density exceeds this share of
/// the experiment's peak energy spread evenly over the domain. Tied to the
/// peak, as dormancy is, a leftover that has spread thin stops holding the
/// mesh at its authored size once it fades; a weak field that stays
/// localized still holds it.
pub const AMR_FIELD_PRESENCE_RATIO: f64 = 1.0e-2;
/// The authored mesh's largest element over its target edge: the mesher
/// splits every edge past this of its target, as adaptation does, so a limit
/// at it never refines the authored mesh.
pub const AMR_AUTHORED_EDGE_SLACK: f64 = 1.05;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AmrDecision {
    Hold,
    Refine,
    Coarsen,
}

impl AmrDecision {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Hold => "settled",
            Self::Refine => "refining",
            Self::Coarsen => "coarsening",
        }
    }
}

/// The estimated error of the whole field `settings` aim for, as a fraction
/// rather than as the control shows it.
pub fn target_accuracy(settings: AdaptationSettings) -> f64 {
    settings.accuracy_percent / 100.0
}

/// Chooses one topology direction for the next adaptation transaction.
///
/// A limit is a floor rather than a judgement: too few elements across a forced
/// wavelength, or an element larger than the largest allowed, is wrong however
/// small the error reads, so those refine on their own account. What the error
/// estimate asks for answers to the accuracy target instead, and it answers to
/// it for the field as a whole rather than element by element. Without that
/// second clause anything the estimate cannot satisfy - a boundary the field
/// disagrees with, the grid-scale leftovers of a wave that has passed - shrinks
/// by the estimator's step every cycle until it reaches the smallest element
/// allowed, and no accuracy setting reaches far enough to stop it: the step is
/// clamped, so it is the same step whatever the target says.
///
/// The cost is that error concentrated in a small part of a domain the estimate
/// is otherwise happy with stops being chased once the whole field is inside
/// the target. That is the trade a single number for the whole field makes.
///
/// Refinement stops at the requested error, but coarsening does not begin until
/// the estimate is substantially below it. That deadband, plus choosing only
/// one direction per transaction, prevents a moving wave from making the same
/// patch alternate between splitting and collapsing.
pub fn adaptation_decision(report: &SolutionIndicatorReport, target_accuracy: f64) -> AmrDecision {
    if report.limit_refine_candidates >= 4
        || (report.error_refine_candidates >= 4 && report.global_indicator > target_accuracy)
    {
        AmrDecision::Refine
    } else if report.coarsen_candidates >= 4
        && report.global_indicator < target_accuracy * AMR_COARSEN_GLOBAL_RATIO
    {
        AmrDecision::Coarsen
    } else {
        AmrDecision::Hold
    }
}

/// Whether a refinement is the size rule's rather than the accuracy
/// target's: elements past their wavelength floor, with the error itself
/// under target or too sparse to refine for. The estimate line says so, or
/// an error well under its target beside a mesh still refining reads as a
/// contradiction.
pub fn size_rule_refines(report: &SolutionIndicatorReport, target_accuracy: f64) -> bool {
    adaptation_decision(report, target_accuracy) == AmrDecision::Refine
        && !(report.error_refine_candidates >= 4 && report.global_indicator > target_accuracy)
}

/// The size field a refinement applies. One the size rule asks for alone
/// applies the limits alone: handed the full field it also splits whatever
/// the estimate would like finer, which the accuracy target has just said is
/// not needed, and the parametric fiber climbed to 19,000 triangles at 2% of
/// a 12% target before coarsening back what the floor never asked for.
pub fn adaptation_field(
    result: &SolutionIndicatorResult,
    target_accuracy: f64,
) -> Arc<AdaptiveSizeField> {
    if size_rule_refines(&result.report, target_accuracy) {
        result.limit_field.clone()
    } else {
        result.field.clone()
    }
}

/// What each material's own drives demand of the mesh beyond the sources.
///
/// A travelling drive patterns the coefficients in space whether or not a
/// wave is present, and only where its material is; the sidebands any drive
/// mixes into the field are left to the error estimate. The scene is
/// authored, so this holds for materials whose drives are not yet
/// executable: it is the mesh rule, not the solver path.
pub fn scene_resolution_demand(scene: &TopologyScene) -> BTreeMap<MaterialId, f64> {
    CanonicalTemporalResolution::of_each_material(&scene.materials).unwrap_or_default()
}

/// The most `measure` gives of any signal the scene drives with: the point
/// source, the driven walls and faces, and the region sources.
pub fn highest_forcing(
    scene: &TopologyScene,
    source: PointSource,
    measure: impl Fn(TimeSignal) -> f64,
) -> f64 {
    let mut frequency = if source.enabled {
        measure(source.signal)
    } else {
        0.0
    };
    for condition in scene.outer_boundaries.sides {
        if let Some(signal) = condition.signal() {
            frequency = frequency.max(measure(signal));
        }
    }
    for curve in &scene.geometry.curves {
        for span in &curve.spans {
            if let SpanBehavior::Separated { left, right, .. } = span.behavior {
                for condition in [left, right] {
                    if let Some(signal) = condition.signal() {
                        frequency = frequency.max(measure(signal));
                    }
                }
            }
        }
    }
    for source in &scene.volume_sources {
        if source.enabled {
            frequency = frequency.max(measure(source.signal));
        }
    }
    frequency
}

/// The estimator's two snapshots of one accepted state, and the runtime a
/// driven one was stepped under.
pub struct IndicatorSnapshots {
    pub scalar: QuadraticSolutionSnapshot,
    pub canonical: CanonicalIndicatorSnapshot,
    pub temporal: Option<(
        Arc<CanonicalTemporalWaveOperator>,
        CanonicalMaterialRuntimeState,
    )>,
}

impl IndicatorSnapshots {
    /// The snapshots of `canonical`, a state of `active` and the state a step
    /// earlier, whose primary field at the state's time is `displacement`:
    /// the scalar one carries that field and the rate the generation's own
    /// operator gives the pair, under `temporal`'s runtime for a driven one.
    pub fn new(
        active: &PreparedTopology,
        canonical: CanonicalIndicatorSnapshot,
        displacement: Vec<f64>,
        temporal: Option<(
            Arc<CanonicalTemporalWaveOperator>,
            CanonicalMaterialRuntimeState,
        )>,
    ) -> Result<Self, WaveError> {
        let dofs = active.operator.degrees_of_freedom();
        // A driven generation's rate is its own: the fixed operator's reads
        // the authored mass and force, and knows no loss law or restoring
        // force, which is not the field the solver stepped.
        let velocity = match &temporal {
            Some((operator, runtime)) => canonical_temporal_primary_rate(
                operator,
                &active.canonical_forcing,
                &canonical,
                runtime,
            ),
            None => canonical_primary_rate(
                &active.canonical_operator,
                &active.canonical_forcing,
                &canonical,
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
            time: canonical.time,
            time_step: canonical.time_step,
        };
        Ok(Self {
            scalar,
            canonical,
            temporal,
        })
    }

    /// The snapshots of a fixed generation's run stepped on the CPU, from its
    /// `state` and the state a step `previous`, its lanes as a full readback
    /// of the device's holds them.
    pub fn of_fixed_run(
        active: &PreparedTopology,
        state: &CanonicalWaveState,
        previous: &CanonicalWaveState,
    ) -> Result<Self, WaveError> {
        let history = |state: &CanonicalWaveState| match state.auxiliaries() {
            CanonicalAuxiliaryState::Linear(auxiliaries) => auxiliaries
                .thin_gap_jump()
                .iter()
                .chain(auxiliaries.outgoing_z())
                .copied()
                .collect(),
            _ => Vec::new(),
        };
        let canonical = CanonicalIndicatorSnapshot {
            mesh_revision: active.mesh.mesh_revision,
            primary_flux: state.primary_flux().to_vec(),
            previous_primary_flux: previous.primary_flux().to_vec(),
            complementary_flux: state.complementary_flux().to_vec(),
            previous_complementary_flux: previous.complementary_flux().to_vec(),
            auxiliary: history(state),
            previous_auxiliary: history(previous),
            integrated_field: Vec::new(),
            previous_integrated_field: Vec::new(),
            time: state.time(),
            time_step: state.time_step(),
        };
        let displacement = state.primary_field(&active.canonical_operator)?;
        Self::new(active, canonical, displacement, None)
    }

    /// The snapshots of a driven generation's run stepped on the CPU under
    /// `operator`, as [`Self::of_fixed_run`] reads a fixed one's, taken
    /// under the runtime `state` was stepped to.
    pub fn of_temporal_run(
        active: &PreparedTopology,
        operator: &Arc<CanonicalTemporalWaveOperator>,
        state: &CanonicalTemporalWaveState,
        previous: &CanonicalTemporalWaveState,
    ) -> Result<Self, WaveError> {
        let history = |state: &CanonicalTemporalWaveState| {
            state
                .thin_gap_jump()
                .iter()
                .chain(state.outgoing_pole_currents())
                .copied()
                .collect()
        };
        let canonical = CanonicalIndicatorSnapshot {
            mesh_revision: active.mesh.mesh_revision,
            primary_flux: state.primary_flux().to_vec(),
            previous_primary_flux: previous.primary_flux().to_vec(),
            complementary_flux: state.complementary_flux().to_vec(),
            previous_complementary_flux: previous.complementary_flux().to_vec(),
            auxiliary: history(state),
            previous_auxiliary: history(previous),
            integrated_field: state.integrated_field().to_vec(),
            previous_integrated_field: previous.integrated_field().to_vec(),
            time: state.time(),
            time_step: state.time_step(),
        };
        let displacement =
            operator.primary_field_at(state.primary_flux(), state.time(), state.runtime())?;
        Self::new(
            active,
            canonical,
            displacement,
            Some((operator.clone(), state.runtime().clone())),
        )
    }
}

/// What the estimate needs beside the scalar job: the solver's own state, of
/// which the canonical supplement is made on the thread the job runs on.
struct CanonicalAmrPreparation {
    mesh: Arc<TriMesh>,
    operator: Arc<CanonicalWaveOperator>,
    /// Present when the generation is driven, in which case the estimate is
    /// built from it instead: the fixed supplement's residual, energy and
    /// recovery all read authored coefficients, and on a driven medium that
    /// charges the estimator for the medium's own modulation.
    ///
    /// It travels with the runtime the accepted state was stepped under -
    /// carrier phases and Switch trajectories as the solver holds them - not
    /// the operator's authored one. A handoff carries a live carrier's phase
    /// into the new generation and re-anchors it, so after a drive edit the
    /// authored anchors describe a coefficient the field never saw.
    temporal: Option<(
        Arc<CanonicalTemporalWaveOperator>,
        CanonicalMaterialRuntimeState,
    )>,
    forcing: Arc<CanonicalForcing>,
    snapshot: CanonicalIndicatorSnapshot,
}

/// An error estimate of one accepted state, resumable in slices of work.
pub struct AmrIndicatorJob {
    job: Option<SolutionIndicatorJob>,
    canonical: Option<CanonicalAmrPreparation>,
}

impl AmrIndicatorJob {
    pub fn with_canonical(
        job: SolutionIndicatorJob,
        mesh: Arc<TriMesh>,
        operator: Arc<CanonicalWaveOperator>,
        temporal: Option<(
            Arc<CanonicalTemporalWaveOperator>,
            CanonicalMaterialRuntimeState,
        )>,
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

    pub fn phase(&self) -> &'static str {
        if self.canonical.is_some() {
            "Preparing canonical AMR estimate"
        } else {
            self.job
                .as_ref()
                .map_or("AMR estimate ready", SolutionIndicatorJob::phase)
        }
    }

    pub fn advance(
        &mut self,
        budget: usize,
    ) -> Option<Result<SolutionIndicatorResult, AmrIndicatorError>> {
        if let Some(canonical) = self.canonical.take() {
            let runtime = canonical
                .temporal
                .as_ref()
                .map(|(_, runtime)| runtime.clone());
            let supplement = match (&canonical.temporal, &runtime) {
                (Some((temporal, _)), Some(runtime)) => canonical_temporal_indicator_supplement(
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
pub enum AmrIndicatorError {
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

/// What an estimate decided.
pub enum AdaptationStep {
    /// The mesh matches the solution.
    Hold,
    /// The estimate asked to coarsen, which waits for the next ones to ask
    /// again.
    Confirming,
    /// The mesh had no adaptation history, and has one now: the next
    /// estimate that asks for a change may make it.
    Initialized,
    /// The transaction to run on the mesh estimated.
    Adapt(Box<MeshAdaptationJob>),
}

/// What adaptation keeps from one estimate to the next.
#[derive(Clone, Debug, Default)]
pub struct AdaptationController {
    /// The running mesh's vertex lineage and cooldowns, once a decision has
    /// needed them.
    pub state: Option<MeshAdaptationState>,
    /// An adapted mesh's, until its handoff publishes.
    pub pending: Option<MeshAdaptationState>,
    /// Estimates in a row that asked to coarsen.
    pub coarsen_streak: u8,
    /// The most energy the field has held since the experiment began, which
    /// sets the floor under which its error is no longer chased.
    pub energy_peak: f64,
    /// The simulated time of the last estimate's snapshot: the next one's,
    /// less it, is how long adaptation last took to come round.
    pub last_estimate_time: Option<f64>,
    /// Whether the transaction [`Self::decide`] last started was asked for by
    /// a dormant estimate, whose handoff carries a leftover and keeps no
    /// totals ([`crate::topology_runtime::PreparedTopology::carries_totals`]).
    pub dormant_transaction: bool,
}

impl AdaptationController {
    /// A new experiment: the user put energy in, by a pulse, a Switch or an
    /// edit of a source, and the device has it. What the run held before is
    /// another experiment's, and a weaker one after a loud one would sit
    /// under its floor, neither refined nor painted; the peak starts again
    /// from the field as it next reads, which holds whatever still rings.
    pub fn mark_experiment(&mut self) {
        self.energy_peak = 0.0;
    }

    /// Raises the experiment's energy peak to `energy`, a stored energy of
    /// the field the solver accepted.
    pub fn observe_energy(&mut self, energy: f64) {
        if energy.is_finite() && energy > 0.0 {
            self.energy_peak = self.energy_peak.max(energy);
        }
    }

    /// How far, in simulated seconds, the field may run before a transaction
    /// an estimate at `time` leads to could be followed by another: the
    /// longer of the last cycle, snapshot to snapshot, and the estimates'
    /// cadence at the speed ceiling `simulation_speed`, which covers a
    /// ceiling raised since, [`AMR_REACH_MARGIN`] times over. A first
    /// estimate, or one after the clock went back, has the cadence alone.
    pub fn reach_horizon(&self, time: f64, simulation_speed: f64) -> f64 {
        let cadence = simulation_speed * AMR_ESTIMATE_SECONDS;
        let cycle = self
            .last_estimate_time
            .map(|last| time - last)
            .filter(|cycle| cycle.is_finite() && *cycle > 0.0)
            .unwrap_or(0.0);
        AMR_REACH_MARGIN * cycle.max(cadence)
    }

    /// The estimate of `snapshots`, a state of `active`, under `presentation`'s
    /// adaptation settings, its authored mesh edge and its speed ceiling.
    pub fn estimate(
        &mut self,
        active: &PreparedTopology,
        snapshots: IndicatorSnapshots,
        presentation: &PresentationSettings,
    ) -> AmrIndicatorJob {
        let settings = presentation.adaptation;
        let IndicatorSnapshots {
            scalar: snapshot,
            canonical: canonical_snapshot,
            temporal,
        } = snapshots;
        let time = canonical_snapshot.time;
        let horizon = self.reach_horizon(time, presentation.simulation_speed);
        self.last_estimate_time = Some(time);
        // The drives' and field laws' floors bound how far under the authored
        // coefficients the solver's divisors fall, which is how much faster
        // than authored any wave of the generation runs. Never under one: the
        // authored speed the estimate samples is the fixed medium's.
        let speed_factor = active
            .canonical_temporal_operator
            .as_ref()
            .map_or(1.0, |operator| {
                let bound = operator.time_step_bound();
                (1.0 / (bound.primary_floor * bound.complementary_floor)).sqrt()
            })
            .max(1.0);
        let patterns = scene_resolution_demand(&active.bundle.authored);
        let job = SolutionIndicatorJob::new_topology(
            active.mesh.clone(),
            active.operator.clone(),
            &active.bundle.plan,
            active.bundle.model(),
            snapshot,
            SolutionIndicatorOptions {
                minimum_edge_length: settings.minimum_edge,
                maximum_edge_length: settings.maximum_edge,
                relative_tolerance: target_accuracy(settings),
                elements_per_wavelength: settings.elements_per_wavelength,
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
                dormant_below_energy: self.energy_peak * DORMANT_ENERGY_RATIO,
                error_energy_floor: self.energy_peak * AMR_ERROR_FLOOR_RATIO,
                field_reach: Some(FieldReach {
                    horizon,
                    speed_factor,
                    authored_edge: AMR_AUTHORED_EDGE_SLACK * presentation.mesh_edge,
                    holding_energy: self.energy_peak * AMR_FIELD_PRESENCE_RATIO,
                }),
                ..Default::default()
            },
        )
        .with_coefficient_patterns(patterns);
        AmrIndicatorJob::with_canonical(
            job,
            active.mesh.clone(),
            active.canonical_operator.clone(),
            temporal,
            active.canonical_forcing.clone(),
            canonical_snapshot,
        )
    }

    /// What `result`, an estimate of `active`, decides under `settings`; a
    /// transaction takes its target revision from `runtime`.
    pub fn decide(
        &mut self,
        result: &SolutionIndicatorResult,
        active: &PreparedTopology,
        settings: AdaptationSettings,
        runtime: &mut TopologyRuntime,
    ) -> AdaptationStep {
        self.observe_energy(result.report.total_energy);
        let target = target_accuracy(settings);
        let decision = adaptation_decision(&result.report, target);
        self.coarsen_streak = if decision == AmrDecision::Coarsen {
            self.coarsen_streak.saturating_add(1)
        } else {
            0
        };
        if decision == AmrDecision::Hold {
            return AdaptationStep::Hold;
        }
        if decision == AmrDecision::Coarsen && self.coarsen_streak < AMR_COARSEN_CONFIRMATIONS {
            return AdaptationStep::Confirming;
        }
        let Some(state) = self
            .state
            .clone()
            .filter(|state| state.mesh_revision == active.mesh.mesh_revision)
        else {
            self.state = Some(MeshAdaptationState::from_mesh(&active.mesh));
            return AdaptationStep::Initialized;
        };
        let target_revision = runtime.reserve_mesh_revision();
        let options = MeshAdaptationOptions {
            meshing: MeshingOptions {
                curve_tolerance: (settings.minimum_edge * 0.02).min(1.5e-3),
                target_edge_length: settings.maximum_edge / 1.05,
                minimum_angle_degrees: 12.0,
                max_vertices: 50_000,
                max_triangles: 100_000,
                max_refinement_steps: 50_000,
            },
            minimum_target_edge_length: settings.minimum_edge,
            maximum_target_edge_length: settings.maximum_edge,
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
        let field: Arc<dyn MeshSizeField> = adaptation_field(result, target);
        self.dormant_transaction = result.report.dormant;
        AdaptationStep::Adapt(Box::new(MeshAdaptationJob::new_topology(
            active.mesh.clone(),
            &active.bundle.plan,
            state,
            target_revision,
            field,
            options,
        )))
    }

    /// Takes in a finished transaction, and answers its report and the mesh
    /// to hand off. A transaction that could apply none of its changes hands
    /// off nothing: it must not manufacture a new mesh revision and force a
    /// GPU handoff. Its history still moves on, onto the `active` mesh, so
    /// that cooldowns expire on an otherwise unchanged mesh.
    pub fn adapted(
        &mut self,
        result: MeshAdaptationResult,
        active: Option<&TriMesh>,
    ) -> (MeshAdaptationReport, Option<TriMesh>) {
        let MeshAdaptationResult {
            mesh,
            mut state,
            report,
        } = result;
        if report.topology_changes == 0 {
            if let Some(active) = active {
                state.mesh_revision = active.mesh_revision;
                self.state = Some(state);
            }
            self.pending = None;
            self.coarsen_streak = 0;
            return (report, None);
        }
        self.pending = Some(state);
        (report, Some(mesh))
    }

    /// A generation has published, `active` now running. An adapted one
    /// takes the history its transaction left; any other starts one.
    pub fn published(&mut self, active: &PreparedTopology) {
        self.state = if active.adapted {
            self.pending
                .take()
                .filter(|state| state.mesh_revision == active.mesh.mesh_revision)
                .or_else(|| Some(MeshAdaptationState::from_mesh(&active.mesh)))
        } else {
            self.pending = None;
            Some(MeshAdaptationState::from_mesh(&active.mesh))
        };
        // A streak of estimates asking to coarsen was of the old mesh:
        // carried over, the new one was coarsened on a single estimate taken
        // right after its transfer (MESH-4 of the 2026-10-20 review).
        self.coarsen_streak = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_estimate_reaches_as_far_as_the_cadence_at_the_ceiling() {
        let controller = AdaptationController::default();
        let horizon = controller.reach_horizon(3.0, 0.5);
        assert_eq!(horizon, AMR_REACH_MARGIN * 0.5 * AMR_ESTIMATE_SECONDS);
    }

    #[test]
    fn a_slow_cycle_reaches_as_far_as_it_took() {
        let controller = AdaptationController {
            last_estimate_time: Some(1.0),
            ..Default::default()
        };
        // A transaction and its handoff stretched the last cycle past the
        // cadence: the next may take as long.
        assert_eq!(controller.reach_horizon(2.5, 1.0), AMR_REACH_MARGIN * 1.5);
    }

    #[test]
    fn a_ceiling_raised_since_the_last_cycle_reaches_at_once() {
        let controller = AdaptationController {
            last_estimate_time: Some(1.0),
            ..Default::default()
        };
        // The last cycle ran at a tenth of the speed; the ceiling is two now.
        let horizon = controller.reach_horizon(1.075, 2.0);
        assert_eq!(horizon, AMR_REACH_MARGIN * 2.0 * AMR_ESTIMATE_SECONDS);
    }

    #[test]
    fn a_clock_gone_back_reaches_as_far_as_the_cadence() {
        let controller = AdaptationController {
            last_estimate_time: Some(40.0),
            ..Default::default()
        };
        // A Reset restarts the clock at zero.
        assert_eq!(
            controller.reach_horizon(0.2, 1.0),
            AMR_REACH_MARGIN * AMR_ESTIMATE_SECONDS
        );
    }
}
