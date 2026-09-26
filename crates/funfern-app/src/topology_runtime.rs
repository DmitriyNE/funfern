//! Atomic CPU-side preparation for the unified topology runtime.
//!
//! A candidate owns one document revision and one immutable authored/compiled
//! topology bundle through meshing, operator assembly, source compilation,
//! transfer, probes, and far field. The visible application may upload a ready
//! candidate and publish it only after the GPU acknowledges that same token.

use crate::document::{FarFieldSettings, ProbeId};
use crate::topology_editor::{
    TopologyBoundaryProbeTarget, TopologyDocument, TopologyProbeDefinition, TopologyProbeTarget,
};
use bevy::platform::time::Instant;
use funfern_core::*;
use std::sync::Arc;
use std::time::Duration;

pub const FAR_FIELD_CONTOUR_POINTS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TopologyToken {
    pub document_revision: u64,
    pub topology_revision: u64,
    /// Changes for every mesh-bearing transaction, including fixed-topology AMR.
    pub mesh_generation: u64,
}

#[derive(Clone, Debug)]
pub struct AcceptedTopology {
    pub token: TopologyToken,
    pub authored: Arc<TopologyScene>,
    pub snapshot: Arc<TopologySnapshot>,
    pub plan: Arc<TopologyMeshPlan>,
}

impl AcceptedTopology {
    pub fn new(
        document_revision: u64,
        mesh_generation: u64,
        authored: TopologyScene,
        compiled: CompiledTopologyScene,
        coarsening: AtomCoarsening,
    ) -> Result<Self, String> {
        let topology_revision = compiled.topology.revision;
        let assignments = authored
            .resolve_face_assignments(&compiled.topology)
            .map_err(|issue| {
                format!("Compiled topology does not match its authored scene: {issue}")
            })?;
        let expected_plan =
            TopologyMeshPlan::new(&compiled.topology, &assignments).map_err(|issue| {
                format!("Compiled topology does not match its authored scene: {issue}")
            })?;
        if compiled.plan.geometry_revision != topology_revision
            || compiled.geometry != authored.geometry
            || compiled.assignments != assignments
            || compiled.plan != expected_plan
            || compiled.plan.domain != authored.geometry.domain
            || !TopologyWaveModel::from_topology_scene(&authored).valid_for(&compiled.plan)
        {
            return Err("Compiled topology does not match its authored scene".into());
        }
        // The compiled plan carries one atom per arrangement segment; the mesh
        // is built from atoms merged to the meshing tolerance.
        let plan = compiled
            .plan
            .coarsened(&compiled.topology, coarsening)
            .map_err(|issue| {
                format!("Compiled topology could not be prepared for meshing: {issue}")
            })?;
        Ok(Self {
            token: TopologyToken {
                document_revision,
                topology_revision,
                mesh_generation,
            },
            authored: Arc::new(authored),
            snapshot: Arc::new(compiled.topology),
            plan: Arc::new(plan),
        })
    }

    pub fn model(&self) -> TopologyWaveModel<'_> {
        TopologyWaveModel::from_topology_scene(&self.authored)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TopologyProbeStencil {
    Point(QuadraticPointStencil),
    Segment(Vec<QuadraticPointStencil>),
    Boundary(Vec<QuadraticBoundaryStencil>),
    Area(QuadraticAreaStencil),
}

#[derive(Clone, Debug, PartialEq)]
pub enum TopologyProbeCompilation {
    Disabled,
    Ready(Box<TopologyProbeStencil>),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledTopologyProbe {
    pub id: ProbeId,
    pub result: TopologyProbeCompilation,
}

/// Wall-clock breakdown of one candidate's CPU preparation. Meshing, assembly,
/// the transfer map, and volume sources are cooperative and yield between
/// slices; probes and the far field still run to completion inside the slice
/// that reaches them, so the measurements bucket shows that synchronous tail
/// directly.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TopologyPreparationTiming {
    /// Wall time actually spent advancing preparation work. This excludes the
    /// idle time between UI frames but includes validation and phase-transition
    /// work which may not belong to one of the narrower buckets below.
    pub work_ms: f64,
    pub meshing_ms: f64,
    pub assembly_ms: f64,
    pub transfer_ms: f64,
    pub sources_ms: f64,
    pub measurements_ms: f64,
    pub slices: u32,
    pub longest_slice_ms: f64,
}

impl TopologyPreparationTiming {
    pub fn total_ms(self) -> f64 {
        self.work_ms
    }

    pub fn categorized_ms(self) -> f64 {
        self.meshing_ms
            + self.assembly_ms
            + self.transfer_ms
            + self.sources_ms
            + self.measurements_ms
    }
}

/// How a request relates to the active topology: whether the field starts
/// from zero and whether an unchanged plan must be remeshed anyway.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreparationIntent {
    pub fresh: bool,
    pub force_rebuild: bool,
    /// Whether a repair refills its band at the sizes adaptation requested
    /// there rather than at the meshing target.
    pub preserve_adaptation: bool,
    /// The solver timestep or another generation-owned value changed even if
    /// the authored solver inputs did not.
    pub require_solver_handoff: bool,
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

impl PreparedTopology {
    /// The step this generation should run at. A driven one is bounded over
    /// its whole coefficient trajectory rather than one set of coefficients,
    /// so its ceiling is the tighter of the two and every caller must read it
    /// from here rather than from the fixed operator.
    pub fn recommended_time_step(&self) -> f64 {
        match &self.canonical_temporal_operator {
            Some(temporal) => temporal.recommended_time_step(),
            None => self.canonical_operator.recommended_time_step(),
        }
    }

    /// Whether this generation carries a material drive.
    pub fn driven(&self) -> bool {
        self.canonical_temporal_operator.is_some()
    }

    /// The model this generation's operator and probe stencils were compiled
    /// against: the authored one with its temporal laws removed. Anything that
    /// evaluates a coefficient for this mesh has to read it here, because the
    /// authored model refuses a law-carrying material and the drive is applied
    /// on top of these base values by the temporal tables.
    pub fn fixed_model(&self) -> TopologyWaveModel<'_> {
        match &self.stripped_model {
            Some(model) => model.as_model(),
            None => self.bundle.model(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PreparedTopology {
    pub bundle: Arc<AcceptedTopology>,
    pub mesh: Arc<TriMesh>,
    pub operator: Arc<QuadraticWaveOperator>,
    pub canonical_operator: Arc<CanonicalWaveOperator>,
    /// The time-driven operator over that same base, when the document carries
    /// a material law. Absent means the generation is inert and the fixed path
    /// runs it exactly as before.
    pub canonical_temporal_operator: Option<Arc<CanonicalTemporalWaveOperator>>,
    /// The authored model with its temporal laws removed, on a driven
    /// generation. Read through [`Self::fixed_model`].
    stripped_model: Option<Arc<OwnedTopologyWaveModel>>,

    pub canonical_forcing: Arc<CanonicalForcing>,
    pub canonical_transfer: Option<Arc<PreparedCanonicalTransfer>>,
    pub volume_sources: Arc<CompiledVolumeSources>,
    pub probes: Arc<[CompiledTopologyProbe]>,
    pub far_field: Option<Result<Arc<QuadraticFarFieldStencil>, String>>,
    pub point_source: PointSource,
    pub probe_definitions: Arc<[TopologyProbeDefinition]>,
    pub far_field_settings: FarFieldSettings,
    pub transfer: Option<Arc<QuadraticTransferMap>>,
    /// Smallest transaction that can publish this candidate. Measurement-only
    /// edits do not touch solver buffers; drive-only edits use the staged GPU
    /// source event when the timestep is unchanged.
    pub solver_update: PreparedSolverUpdate,
    pub fresh: bool,
    pub mesh_action: TopologyMeshUpdateAction,
    pub operator_reused: bool,
    pub adapted: bool,
    /// What a repair kept, removed and inserted, when the mesh was carved.
    pub carve: Option<CarveReport>,
    /// Why a repair was abandoned for a full rebuild, when it was.
    pub repair_fallback: Option<String>,
    pub timing: TopologyPreparationTiming,
    /// Options the mesh was built with. A request with different options is
    /// a full rebuild even when the plan is unchanged.
    pub meshing: MeshingOptions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparedSolverUpdate {
    FullHandoff,
    SourceWeightsOnly,
    SourceDrivesOnly,
    MeasurementsOnly,
}

#[derive(Clone, Debug)]
pub struct PreparedCanonicalTransfer {
    pub primary: Arc<CanonicalPrimaryTransferMap>,
    pub complementary: Arc<CanonicalVectorTransferMap>,
    pub thin_gap: Arc<CanonicalThinGapHistoryTransferMap>,
    pub outgoing: Arc<CanonicalOutgoingNormalizedTransfer>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopologyPreparationPhase {
    Repairing,
    Meshing,
    Assembling,
    AssemblingCanonical,
    Transferring,
    TransferringCanonical,
    CompilingSources,
    CompilingMeasurements,
    Ready,
    Failed,
}

impl TopologyPreparationPhase {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Repairing => "Repairing the mesh",
            Self::Meshing => "Mesh rebuilding",
            Self::Assembling => "Assembling wave operator",
            Self::AssemblingCanonical => "Compiling canonical constitutive state",
            Self::Transferring => "Carrying the field across",
            Self::TransferringCanonical => "Preparing canonical field transfer",
            Self::CompilingSources => "Compiling sources",
            Self::CompilingMeasurements => "Compiling probes",
            Self::Ready => "Ready for GPU upload",
            Self::Failed => "Preparation failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopologyPreparationError {
    pub token: TopologyToken,
    pub phase: TopologyPreparationPhase,
    pub message: String,
}

impl std::fmt::Display for TopologyPreparationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.phase.label(), self.message)
    }
}

impl std::error::Error for TopologyPreparationError {}

/// Copies the active mesh onto a plan that holds the same boundaries in the
/// same places under a fresh trace numbering. The arrangement hands out one
/// trace id per sector a vertex has, so switching a span between transmitting
/// and separated renumbers the traces after it even when the plan is otherwise
/// identical - which it is when the far side is an excluded face, since the
/// span is walled either way. A carve derives its traces from the plan it is
/// handed, but a reused mesh keeps the numbering it was built with, and an
/// adaptation checks every one of them against the plan.
///
/// `None` when the two numberings cannot be matched up, which the caller
/// answers with a rebuild.
fn retraced(
    mesh: &TriMesh,
    previous: &TopologyMeshPlan,
    next: &TopologyMeshPlan,
) -> Option<TriMesh> {
    let remap = topology_trace_remap(previous, next)?;
    let mut mesh = mesh.clone();
    mesh.geometry_revision = next.geometry_revision;
    for vertex in &mut mesh.vertices {
        if let Some(trace) = vertex.trace {
            vertex.trace = Some(remap.get(&trace).copied()?);
        }
    }
    Some(mesh)
}

pub struct TopologyPreparationJob {
    bundle: Arc<AcceptedTopology>,
    probes: Arc<[TopologyProbeDefinition]>,
    point_source: PointSource,
    far_field: FarFieldSettings,
    previous: Option<Arc<PreparedTopology>>,
    mesh_action: TopologyMeshUpdateAction,
    fresh: bool,
    phase: TopologyPreparationPhase,
    carve_job: Option<TopologyCarveJob>,
    carve: Option<CarveReport>,
    repair_fallback: Option<String>,
    mesh_job: Option<TopologyMeshingJob>,
    mesh: Option<Arc<TriMesh>>,
    assembly_job: Option<QuadraticAssemblyJob>,
    operator: Option<Arc<QuadraticWaveOperator>>,
    point_source_validated: bool,
    #[cfg(test)]
    point_source_validation_count: u32,
    canonical_assembly_job: Option<CanonicalAssemblyJob>,
    canonical_operator: Option<Arc<CanonicalWaveOperator>>,
    /// The time-driven operator over the same base, when the document carries
    /// a material law. Absent means the generation is inert and the fixed
    /// path runs it, which is every document that existed before drives.
    canonical_temporal_operator: Option<Arc<CanonicalTemporalWaveOperator>>,
    /// The authored model with its temporal laws removed, which is what both
    /// assemblies must be given. The fixed compilers refuse a law-carrying
    /// material outright, so this is not an optimization: it is the only thing
    /// that compiles. Built once because both assemblies want it.
    stripped_model: Option<OwnedTopologyWaveModel>,
    transfer_job: Option<QuadraticTransferJob>,
    transfer: Option<Arc<QuadraticTransferMap>>,
    source_job: Option<VolumeSourceCompileJob>,
    volume_sources: Option<Arc<CompiledVolumeSources>>,
    canonical_forcing: Option<Arc<CanonicalForcing>>,
    canonical_primary_transfer: Option<Arc<CanonicalPrimaryTransferMap>>,
    canonical_vector_job: Option<CanonicalVectorTransferJob>,
    canonical_vector_transfer: Option<Arc<CanonicalVectorTransferMap>>,
    canonical_gap_transfer: Option<Arc<CanonicalThinGapHistoryTransferMap>>,
    canonical_outgoing_job: Option<CanonicalOutgoingNormalizedTransferJob>,
    canonical_outgoing_transfer: Option<Arc<CanonicalOutgoingNormalizedTransfer>>,
    canonical_transfer: Option<Arc<PreparedCanonicalTransfer>>,
    operator_reused: bool,
    require_solver_handoff: bool,
    adapted: bool,
    done: bool,
    timing: TopologyPreparationTiming,
    meshing: MeshingOptions,
}

impl TopologyPreparationJob {
    /// Whether this document carries a material law at all. Every document
    /// that existed before drives answers no and takes the fixed path
    /// unchanged.
    fn driven(&self) -> bool {
        !self
            .bundle
            .model()
            .materials
            .iter()
            .all(funfern_core::Material::time_invariant)
    }

    /// The authored model with its temporal laws removed, built once and kept.
    ///
    /// Both assemblies need this rather than the authored model: the fixed
    /// compilers refuse a law-carrying material outright, by the gate that
    /// stops a law being executed as a static medium. The laws are kept for
    /// the temporal operator, which is built over the base these produce.
    fn ensure_stripped_model(&mut self) {
        if self.stripped_model.is_none() {
            self.stripped_model = Some(self.bundle.model().to_owned().without_temporal_laws());
        }
    }

    fn stripped_model(&self) -> TopologyWaveModel<'_> {
        self.stripped_model
            .as_ref()
            .expect("the stripped model is built before either assembly")
            .as_model()
    }

    pub fn new(
        document_revision: u64,
        document: &TopologyDocument,
        compiled: CompiledTopologyScene,
        previous: Option<Arc<PreparedTopology>>,
        mesh_revision: u64,
        options: MeshingOptions,
        intent: PreparationIntent,
    ) -> Result<Self, String> {
        let PreparationIntent {
            fresh,
            force_rebuild,
            preserve_adaptation,
            require_solver_handoff,
        } = intent;
        let same_authored_scene = previous
            .as_ref()
            .is_some_and(|previous| *previous.bundle.authored == document.model.accepted);
        let same_operator_scene = previous.as_ref().is_some_and(|previous| {
            operator_scene_eq(&previous.bundle.authored, &document.model.accepted)
        });
        let same_mesh_scene = previous.as_ref().is_some_and(|previous| {
            mesh_scene_eq(&previous.bundle.authored, &document.model.accepted)
        });
        let coarsening = AtomCoarsening::from_meshing(options);
        let bundle = if same_mesh_scene {
            let previous = previous.as_ref().unwrap();
            // The plan's atoms depend on the meshing options, so a changed
            // resolution re-derives them from the same compiled scene.
            let plan = if previous.meshing == options {
                previous.bundle.plan.clone()
            } else {
                Arc::new(
                    compiled
                        .plan
                        .coarsened(&compiled.topology, coarsening)
                        .map_err(|issue| {
                            format!("Compiled topology could not be prepared for meshing: {issue}")
                        })?,
                )
            };
            Arc::new(AcceptedTopology {
                token: TopologyToken {
                    document_revision,
                    topology_revision: previous.bundle.token.topology_revision,
                    mesh_generation: if previous.meshing == options && !force_rebuild {
                        previous.bundle.token.mesh_generation
                    } else {
                        mesh_revision
                    },
                },
                authored: if same_authored_scene {
                    previous.bundle.authored.clone()
                } else {
                    Arc::new(document.model.accepted.clone())
                },
                snapshot: previous.bundle.snapshot.clone(),
                plan,
            })
        } else {
            Arc::new(AcceptedTopology::new(
                document_revision,
                mesh_revision,
                document.model.accepted.clone(),
                compiled,
                coarsening,
            )?)
        };
        // Changed options re-derive the plan's atoms, so they are decided
        // before the plans are compared; otherwise the comparison would read
        // the re-atomised boundaries as moved geometry.
        let mut mesh_action = match previous.as_ref() {
            None => TopologyMeshUpdateAction::FullRebuild(TopologyFullRebuildReason::DomainChanged),
            Some(previous) if previous.meshing != options => TopologyMeshUpdateAction::FullRebuild(
                TopologyFullRebuildReason::MeshingOptionsChanged,
            ),
            Some(previous) => {
                match topology_mesh_update_action(&previous.bundle.plan, &bundle.plan) {
                    TopologyMeshUpdateAction::Reuse if force_rebuild => {
                        TopologyMeshUpdateAction::FullRebuild(TopologyFullRebuildReason::Requested)
                    }
                    action => action,
                }
            }
        };
        // The reused mesh is prepared alongside the action, because bringing
        // its traces onto the new plan can fail and a rebuild is the answer
        // when it does.
        let mut reused = None;
        if matches!(mesh_action, TopologyMeshUpdateAction::Reuse)
            && let Some(active) = previous.as_ref()
        {
            reused = if same_mesh_scene {
                Some(active.mesh.clone())
            } else {
                retraced(&active.mesh, &active.bundle.plan, &bundle.plan).map(Arc::new)
            };
            if reused.is_none() {
                mesh_action = TopologyMeshUpdateAction::FullRebuild(
                    TopologyFullRebuildReason::TraceIdentityChanged,
                );
            }
        }
        let (carve_job, mesh_job, mesh, phase) = match mesh_action {
            TopologyMeshUpdateAction::Reuse => {
                (None, None, reused, TopologyPreparationPhase::Assembling)
            }
            TopologyMeshUpdateAction::Repair(_) => {
                let previous = previous.as_ref().unwrap();
                (
                    Some(TopologyCarveJob::new(
                        previous.mesh.clone(),
                        &previous.bundle.plan,
                        bundle.plan.as_ref().clone(),
                        bundle.snapshot.clone(),
                        mesh_revision,
                        options,
                        preserve_adaptation,
                    )),
                    None,
                    None,
                    TopologyPreparationPhase::Repairing,
                )
            }
            TopologyMeshUpdateAction::FullRebuild(_) => (
                None,
                Some(TopologyMeshingJob::new(
                    bundle.plan.as_ref().clone(),
                    mesh_revision,
                    options,
                )),
                None,
                TopologyPreparationPhase::Meshing,
            ),
        };
        // A rebuilt mesh needs a fresh operator even when the scene is unchanged.
        let operator_reused =
            same_operator_scene && matches!(mesh_action, TopologyMeshUpdateAction::Reuse);
        let operator = operator_reused.then(|| previous.as_ref().unwrap().operator.clone());
        let canonical_operator =
            operator_reused.then(|| previous.as_ref().unwrap().canonical_operator.clone());
        // The laws travel with the operator. `operator_scene_eq` compares whole
        // materials, so a reused operator means the laws are the ones it was
        // compiled against and the temporal operator built over it is still the
        // right one. Dropping it here made every preparation after the first
        // read as inert: the medium stopped being driven a moment after it
        // started, and the two generations then shared no state to transfer, so
        // the field was reset on the way through as well.
        let canonical_temporal_operator = operator_reused
            .then(|| {
                previous
                    .as_ref()
                    .unwrap()
                    .canonical_temporal_operator
                    .clone()
            })
            .flatten();
        let point_source_validated = operator_reused
            && previous.as_ref().is_some_and(|previous| {
                previous.point_source.enabled == document.model.source.enabled
                    && previous.point_source.spatial_eq(document.model.source)
            });
        let canonical_forcing = (operator_reused
            && previous.as_ref().is_some_and(|previous| {
                previous.point_source == document.model.source
                    && previous.bundle.authored.volume_sources
                        == document.model.accepted.volume_sources
            }))
        .then(|| previous.as_ref().unwrap().canonical_forcing.clone());
        let volume_sources = operator_reused
            .then(|| {
                let previous = previous.as_ref().unwrap();
                volume_source_layout_eq(
                    &previous.bundle.authored.volume_sources,
                    &document.model.accepted.volume_sources,
                )
                .then(|| {
                    if previous.bundle.authored.volume_sources
                        == document.model.accepted.volume_sources
                    {
                        previous.volume_sources.clone()
                    } else {
                        let mut compiled = previous.volume_sources.as_ref().clone();
                        compiled.signals = document
                            .model
                            .accepted
                            .volume_sources
                            .iter()
                            .filter(|source| source.enabled)
                            .map(|source| source.signal)
                            .collect();
                        Arc::new(compiled)
                    }
                })
            })
            .flatten();
        Ok(Self {
            bundle,
            probes: document.model.probes.clone().into(),
            point_source: document.model.source,
            far_field: document.model.far_field,
            previous,
            mesh_action,
            fresh,
            phase,
            carve_job,
            carve: None,
            repair_fallback: None,
            mesh_job,
            mesh,
            operator,
            point_source_validated,
            #[cfg(test)]
            point_source_validation_count: 0,
            assembly_job: None,
            canonical_assembly_job: None,
            canonical_temporal_operator,
            stripped_model: None,
            canonical_operator,
            transfer_job: None,
            transfer: None,
            source_job: None,
            volume_sources,
            canonical_forcing,
            canonical_primary_transfer: None,
            canonical_vector_job: None,
            canonical_vector_transfer: None,
            canonical_gap_transfer: None,
            canonical_outgoing_job: None,
            canonical_outgoing_transfer: None,
            canonical_transfer: None,
            operator_reused,
            require_solver_handoff,
            adapted: false,
            done: false,
            timing: TopologyPreparationTiming::default(),
            meshing: options,
        })
    }

    fn new_adapted(
        document_revision: u64,
        document: &TopologyDocument,
        previous: Arc<PreparedTopology>,
        mesh: TriMesh,
    ) -> Result<Self, String> {
        if document.model.accepted != *previous.bundle.authored
            || mesh.geometry_revision != previous.bundle.plan.geometry_revision
            || mesh.mesh_revision == previous.mesh.mesh_revision
        {
            return Err("Adapted mesh does not match the active topology".into());
        }
        let bundle = Arc::new(AcceptedTopology {
            token: TopologyToken {
                document_revision,
                topology_revision: previous.bundle.token.topology_revision,
                mesh_generation: mesh.mesh_revision,
            },
            authored: previous.bundle.authored.clone(),
            snapshot: previous.bundle.snapshot.clone(),
            plan: previous.bundle.plan.clone(),
        });
        Ok(Self {
            bundle,
            probes: document.model.probes.clone().into(),
            point_source: document.model.source,
            far_field: document.model.far_field,
            meshing: previous.meshing,
            previous: Some(previous),
            mesh_action: TopologyMeshUpdateAction::Reuse,
            fresh: false,
            phase: TopologyPreparationPhase::Assembling,
            carve_job: None,
            carve: None,
            repair_fallback: None,
            mesh_job: None,
            mesh: Some(Arc::new(mesh)),
            operator: None,
            point_source_validated: false,
            #[cfg(test)]
            point_source_validation_count: 0,
            assembly_job: None,
            canonical_assembly_job: None,
            canonical_temporal_operator: None,
            stripped_model: None,
            canonical_operator: None,
            transfer_job: None,
            transfer: None,
            source_job: None,
            volume_sources: None,
            canonical_forcing: None,
            canonical_primary_transfer: None,
            canonical_vector_job: None,
            canonical_vector_transfer: None,
            canonical_gap_transfer: None,
            canonical_outgoing_job: None,
            canonical_outgoing_transfer: None,
            canonical_transfer: None,
            operator_reused: false,
            require_solver_handoff: true,
            adapted: true,
            done: false,
            timing: TopologyPreparationTiming::default(),
        })
    }

    pub fn token(&self) -> TopologyToken {
        self.bundle.token
    }

    pub fn phase(&self) -> TopologyPreparationPhase {
        self.phase
    }

    pub fn detail(&self) -> &'static str {
        if let Some(job) = &self.carve_job {
            job.phase()
        } else if let Some(job) = &self.mesh_job {
            job.phase()
        } else if let Some(job) = &self.assembly_job {
            job.phase()
        } else if let Some(job) = &self.canonical_assembly_job {
            job.phase()
        } else if let Some(job) = &self.transfer_job {
            job.phase()
        } else if let Some(job) = &self.canonical_vector_job {
            job.phase()
        } else if let Some(job) = &self.canonical_outgoing_job {
            job.phase()
        } else {
            self.phase.label()
        }
    }

    pub fn timing(&self) -> TopologyPreparationTiming {
        self.timing
    }

    pub fn advance(
        &mut self,
        budget: usize,
    ) -> Option<Result<PreparedTopology, TopologyPreparationError>> {
        if self.done || budget == 0 {
            return None;
        }
        let started = Instant::now();
        let outcome = self.advance_slice(budget);
        self.finish_slice(started, outcome)
    }

    /// Runs indivisible work units until the job finishes or `budget` of wall
    /// time has passed, and counts the whole call as one slice. Checking the
    /// deadline after every unit matters: formula evaluation makes a canonical
    /// element much more expensive than the other phases' usual units, so a
    /// batch here would turn a time budget back into a frame-sized stall.
    pub fn advance_for(
        &mut self,
        budget: Duration,
    ) -> Option<Result<PreparedTopology, TopologyPreparationError>> {
        if self.done {
            return None;
        }
        let started = Instant::now();
        let mut outcome = None;
        while outcome.is_none() && !self.done {
            outcome = self.advance_slice(1);
            if started.elapsed() >= budget {
                break;
            }
        }
        self.finish_slice(started, outcome)
    }

    fn finish_slice(
        &mut self,
        started: Instant,
        mut outcome: Option<Result<PreparedTopology, TopologyPreparationError>>,
    ) -> Option<Result<PreparedTopology, TopologyPreparationError>> {
        let elapsed = elapsed_ms(started);
        self.timing.work_ms += elapsed;
        self.timing.slices = self.timing.slices.saturating_add(1);
        self.timing.longest_slice_ms = self.timing.longest_slice_ms.max(elapsed);
        // A finished handoff copied the timing before this slice was counted, so
        // without this it omits its own last and usually longest slice, which is
        // exactly the tail the diagnostics exist to show.
        if let Some(Ok(prepared)) = &mut outcome {
            prepared.timing = self.timing;
        }
        outcome
    }

    fn advance_slice(
        &mut self,
        budget: usize,
    ) -> Option<Result<PreparedTopology, TopologyPreparationError>> {
        if let Some(job) = &mut self.carve_job {
            let started = Instant::now();
            let result = job.advance(budget);
            self.timing.meshing_ms += elapsed_ms(started);
            let result = result?;
            let report = job.report();
            self.carve_job = None;
            match result {
                Ok(mesh) => {
                    self.carve = Some(report);
                    self.mesh = Some(Arc::new(mesh));
                    self.phase = TopologyPreparationPhase::Assembling;
                }
                // A carve that cannot finish is not an error of the request:
                // the plan is sound, so the full rebuild takes over and the
                // reason travels with the candidate for the diagnostics.
                Err(error) => {
                    self.repair_fallback = Some(error.to_string());
                    self.mesh_action = TopologyMeshUpdateAction::FullRebuild(
                        TopologyFullRebuildReason::RepairFailed,
                    );
                    self.mesh_job = Some(TopologyMeshingJob::new(
                        self.bundle.plan.as_ref().clone(),
                        self.bundle.token.mesh_generation,
                        self.meshing,
                    ));
                    self.phase = TopologyPreparationPhase::Meshing;
                }
            }
            return None;
        }
        if let Some(job) = &mut self.mesh_job {
            let started = Instant::now();
            let result = job.advance(budget);
            self.timing.meshing_ms += elapsed_ms(started);
            let result = result?;
            self.mesh_job = None;
            match result {
                Ok(mesh) => {
                    self.mesh = Some(Arc::new(mesh));
                    self.phase = TopologyPreparationPhase::Assembling;
                }
                Err(error) => return Some(Err(self.fail(error.to_string()))),
            }
            return None;
        }
        // Assembly and the transfer map used to run to completion inside the
        // slice that finished meshing, which cost every handover two to three
        // frames at once. Both are cooperative jobs now and yield like the mesh.
        if self.operator.is_none() {
            let mesh = self.mesh.as_ref().unwrap().clone();
            if self.assembly_job.is_none() {
                self.phase = TopologyPreparationPhase::Assembling;
                self.ensure_stripped_model();
                let plan = self.bundle.plan.clone();
                match QuadraticAssemblyJob::new_topology(mesh.clone(), plan, self.stripped_model())
                {
                    Ok(job) => self.assembly_job = Some(job),
                    Err(error) => return Some(Err(self.fail(error.to_string()))),
                }
            }
            let started = Instant::now();
            let result = self.assembly_job.as_mut().unwrap().advance(budget);
            self.timing.assembly_ms += elapsed_ms(started);
            let result = result?;
            self.assembly_job = None;
            let operator = match result {
                Ok(operator) => Arc::new(operator),
                Err(error) => return Some(Err(self.fail(error.to_string()))),
            };
            self.operator = Some(operator);
            if let Err(error) = self.validate_point_source_once() {
                return Some(Err(self.fail(error)));
            }
            return None;
        } else if let Err(error) = self.validate_point_source_once() {
            return Some(Err(self.fail(error)));
        }

        if self.canonical_operator.is_none() {
            let mesh = self.mesh.as_ref().unwrap().clone();
            let operator = self.operator.as_ref().unwrap().clone();
            if self.canonical_assembly_job.is_none() {
                self.phase = TopologyPreparationPhase::AssemblingCanonical;
                let previous_outgoing = self
                    .previous
                    .as_ref()
                    .and_then(|previous| previous.canonical_operator.outgoing_boundary_handle());
                self.ensure_stripped_model();
                let revision = self.bundle.token.document_revision;
                match CanonicalAssemblyJob::new_with_outgoing_reuse(
                    mesh,
                    operator,
                    self.stripped_model(),
                    revision,
                    previous_outgoing,
                ) {
                    Ok(job) => self.canonical_assembly_job = Some(job),
                    Err(error) => return Some(Err(self.fail(error.to_string()))),
                }
            }
            let started = Instant::now();
            let result = self
                .canonical_assembly_job
                .as_mut()
                .unwrap()
                .advance(budget);
            self.timing.assembly_ms += elapsed_ms(started);
            let result = result?;
            self.canonical_assembly_job = None;
            match result {
                Ok(operator) => {
                    let operator = Arc::new(operator);
                    // The laws the assembly was not given. Everything this adds
                    // is per-sample law evaluation over the base just built, so
                    // it is cheap next to the assembly it reuses - but it does
                    // not yield, so it is measured rather than assumed.
                    if self.driven() {
                        let started = Instant::now();
                        let temporal = CanonicalTemporalWaveOperator::from_base(
                            operator.clone(),
                            self.mesh.as_ref().unwrap(),
                            self.operator.as_ref().unwrap(),
                            self.bundle.model(),
                        );
                        self.timing.assembly_ms += elapsed_ms(started);
                        match temporal {
                            Ok(temporal) => {
                                self.canonical_temporal_operator = Some(Arc::new(temporal))
                            }
                            Err(error) => return Some(Err(self.fail(error.to_string()))),
                        }
                    }
                    self.canonical_operator = Some(operator);
                }
                Err(error) => return Some(Err(self.fail(error.to_string()))),
            }
            return None;
        }

        if !self.fresh
            && !self.operator_reused
            && self.transfer.is_none()
            && self.transfer_job.is_none()
        {
            let previous = self.previous.as_ref().unwrap();
            self.phase = TopologyPreparationPhase::Transferring;
            if Arc::ptr_eq(&previous.mesh, self.mesh.as_ref().unwrap()) {
                let started = Instant::now();
                match QuadraticTransferMap::identity_on_mesh(
                    self.mesh.as_ref().unwrap(),
                    &previous.operator,
                    self.operator.as_ref().unwrap(),
                ) {
                    Ok(transfer) => {
                        self.transfer = Some(Arc::new(transfer));
                        self.timing.transfer_ms += elapsed_ms(started);
                    }
                    Err(error) => return Some(Err(self.fail(error.to_string()))),
                }
            } else {
                self.transfer_job = Some(QuadraticTransferJob::new(
                    previous.mesh.clone(),
                    previous.operator.clone(),
                    self.mesh.as_ref().unwrap().clone(),
                    self.operator.as_ref().unwrap().clone(),
                ));
            }
        }
        if let Some(job) = &mut self.transfer_job {
            let started = Instant::now();
            let result = job.advance(budget);
            self.timing.transfer_ms += elapsed_ms(started);
            let result = result?;
            self.transfer_job = None;
            match result {
                Ok(transfer) => self.transfer = Some(Arc::new(transfer)),
                Err(error) => return Some(Err(self.fail(error.to_string()))),
            }
            return None;
        }

        if self.volume_sources.is_none() && self.source_job.is_none() {
            let mesh = self.mesh.as_ref().unwrap().clone();
            let operator = self.operator.as_ref().unwrap().clone();
            if let Err(error) = self.start_sources(mesh, operator) {
                return Some(Err(self.fail(error)));
            }
        }
        if let Some(job) = &mut self.source_job {
            let started = Instant::now();
            let result = job.advance(budget);
            self.timing.sources_ms += elapsed_ms(started);
            let result = result?;
            self.source_job = None;
            match result {
                Ok(sources) => self.volume_sources = Some(Arc::new(sources)),
                Err(error) => return Some(Err(self.fail(error.to_string()))),
            }
            return None;
        }

        if self.canonical_forcing.is_none() {
            let forcing = match compile_canonical_forcing(
                self.mesh.as_ref().unwrap(),
                self.operator.as_ref().unwrap(),
                self.canonical_operator.as_ref().unwrap(),
                self.volume_sources.as_ref().unwrap(),
                &self.bundle.authored.volume_sources,
                self.point_source,
            ) {
                Ok(forcing) => forcing,
                Err(error) => return Some(Err(self.fail(error))),
            };
            self.canonical_forcing = Some(Arc::new(forcing));
            return None;
        }

        let solver_update = self.prepared_solver_update();
        if !self.fresh
            && solver_update == PreparedSolverUpdate::FullHandoff
            && self.canonical_transfer.is_none()
        {
            self.phase = TopologyPreparationPhase::TransferringCanonical;
            let interpolation = if let Some(transfer) = &self.transfer {
                transfer.clone()
            } else {
                let previous = self.previous.as_ref().unwrap();
                match QuadraticTransferMap::identity_on_mesh(
                    self.mesh.as_ref().unwrap(),
                    &previous.operator,
                    self.operator.as_ref().unwrap(),
                ) {
                    Ok(map) => Arc::new(map),
                    Err(error) => return Some(Err(self.fail(error.to_string()))),
                }
            };
            let previous = self.previous.as_ref().unwrap();
            if self.canonical_primary_transfer.is_none() {
                let primary = CanonicalPrimaryTransferMap::prepare_with_meshes(
                    &interpolation,
                    &previous.mesh,
                    &previous.canonical_operator,
                    self.mesh.as_ref().unwrap(),
                    self.canonical_operator.as_ref().unwrap(),
                );
                let primary = match primary {
                    Ok(primary) => Arc::new(primary),
                    Err(error) => return Some(Err(self.fail(error.to_string()))),
                };
                self.canonical_primary_transfer = Some(primary);
                self.canonical_vector_job = Some(CanonicalVectorTransferJob::new(
                    previous.mesh.clone(),
                    previous.canonical_operator.clone(),
                    self.mesh.as_ref().unwrap().clone(),
                    self.canonical_operator.as_ref().unwrap().clone(),
                ));
                self.canonical_gap_transfer = match CanonicalThinGapHistoryTransferMap::prepare(
                    previous.canonical_operator.thin_gap_samples(),
                    self.canonical_operator.as_ref().unwrap().thin_gap_samples(),
                ) {
                    Ok(map) => Some(Arc::new(map)),
                    Err(error) => return Some(Err(self.fail(error.to_string()))),
                };
                let outgoing = match CanonicalOutgoingHistoryTransferMap::prepare(
                    &interpolation,
                    &previous.canonical_operator,
                    self.canonical_operator.as_ref().unwrap(),
                ) {
                    Ok(map) => Arc::new(map),
                    Err(error) => return Some(Err(self.fail(error.to_string()))),
                };
                self.canonical_outgoing_job = Some(CanonicalOutgoingNormalizedTransferJob::new(
                    outgoing,
                    previous.canonical_operator.clone(),
                    self.canonical_operator.as_ref().unwrap().clone(),
                ));
                return None;
            }
            if let Some(job) = &mut self.canonical_vector_job {
                let started = Instant::now();
                let result = job.advance(budget);
                self.timing.transfer_ms += elapsed_ms(started);
                if let Some(result) = result {
                    self.canonical_vector_job = None;
                    match result {
                        Ok(map) => self.canonical_vector_transfer = Some(Arc::new(map)),
                        Err(error) => return Some(Err(self.fail(error.to_string()))),
                    }
                }
                return None;
            }
            if let Some(job) = &mut self.canonical_outgoing_job {
                let started = Instant::now();
                let result = job.advance(budget);
                self.timing.transfer_ms += elapsed_ms(started);
                if let Some(result) = result {
                    self.canonical_outgoing_job = None;
                    match result {
                        Ok(map) => self.canonical_outgoing_transfer = Some(Arc::new(map)),
                        Err(error) => return Some(Err(self.fail(error.to_string()))),
                    }
                }
                return None;
            }
            if let (Some(primary), Some(complementary), Some(thin_gap), Some(outgoing)) = (
                self.canonical_primary_transfer.clone(),
                self.canonical_vector_transfer.clone(),
                self.canonical_gap_transfer.clone(),
                self.canonical_outgoing_transfer.clone(),
            ) {
                self.canonical_transfer = Some(Arc::new(PreparedCanonicalTransfer {
                    primary,
                    complementary,
                    thin_gap,
                    outgoing,
                }));
                return None;
            } else {
                return None;
            }
        }

        self.phase = TopologyPreparationPhase::CompilingMeasurements;
        let measurements_started = Instant::now();
        let mesh = self.mesh.as_ref().unwrap().clone();
        let operator = self.operator.as_ref().unwrap().clone();
        // Every stencil below places itself against the *base* coefficients,
        // the same ones both assemblies were built from. Reading the authored
        // model instead would refuse a law-carrying material outright - the
        // probe's own error says the mesh is unusable, which it is not - so a
        // document with a drive and an enabled probe or source could not be
        // prepared at all.
        self.ensure_stripped_model();
        let probes = compile_probes_reusing(
            &self.probes,
            &mesh,
            &operator,
            &self.bundle,
            self.stripped_model(),
            self.operator_reused
                .then_some(self.previous.as_deref())
                .flatten(),
        );
        let far_field = self
            .previous
            .as_ref()
            .filter(|previous| {
                self.operator_reused
                    && previous.far_field_settings == self.far_field
                    && previous.point_source.enabled == self.point_source.enabled
                    && previous.point_source.spatial_eq(self.point_source)
                    && volume_source_layout_eq(
                        &previous.bundle.authored.volume_sources,
                        &self.bundle.authored.volume_sources,
                    )
            })
            .map(|previous| previous.far_field.clone())
            .unwrap_or_else(|| {
                self.far_field.enabled.then(|| {
                    // The authored model, not the stripped one the fixed
                    // assembly reads: the projection has to see a drive on
                    // the exterior to refuse it, and the stripped model has
                    // none by construction.
                    QuadraticFarFieldStencil::build_topology(
                        &mesh,
                        &operator,
                        &self.bundle.plan,
                        self.bundle.model(),
                        FarFieldCompileOptions {
                            inset: self.far_field.inset,
                            sample_count: FAR_FIELD_CONTOUR_POINTS,
                            point_source: Some(self.point_source),
                            volume_sources: &self.bundle.authored.volume_sources,
                        },
                    )
                    .map(Arc::new)
                    .map_err(|error| error.to_string())
                })
            });
        self.timing.measurements_ms += elapsed_ms(measurements_started);
        self.done = true;
        self.phase = TopologyPreparationPhase::Ready;
        let stripped_model = self.driven().then(|| {
            self.ensure_stripped_model();
            Arc::new(self.stripped_model.clone().expect("built just above"))
        });
        Some(Ok(PreparedTopology {
            bundle: self.bundle.clone(),
            mesh,
            operator,
            canonical_operator: self.canonical_operator.as_ref().unwrap().clone(),
            canonical_temporal_operator: self.canonical_temporal_operator.clone(),
            stripped_model,
            canonical_forcing: self.canonical_forcing.as_ref().unwrap().clone(),
            canonical_transfer: self.canonical_transfer.take(),
            volume_sources: self.volume_sources.take().unwrap(),
            probes: probes.into(),
            far_field,
            point_source: self.point_source,
            probe_definitions: self.probes.clone(),
            far_field_settings: self.far_field,
            transfer: self.transfer.take(),
            solver_update,
            fresh: self.fresh,
            mesh_action: self.mesh_action,
            operator_reused: self.operator_reused,
            adapted: self.adapted,
            carve: self.carve,
            repair_fallback: self.repair_fallback.take(),
            timing: self.timing,
            meshing: self.meshing,
        }))
    }

    /// Starts the cooperative volume-source compilation once the operator and,
    /// when one is needed, the transfer map exist.
    fn start_sources(
        &mut self,
        mesh: Arc<TriMesh>,
        operator: Arc<QuadraticWaveOperator>,
    ) -> Result<(), String> {
        self.phase = TopologyPreparationPhase::CompilingSources;
        self.ensure_stripped_model();
        let job = VolumeSourceCompileJob::new_topology(
            mesh,
            operator,
            &self.bundle.plan,
            self.stripped_model(),
            &self.bundle.authored.volume_sources,
        )
        .map_err(|error| error.to_string())?;
        self.source_job = Some(job);
        Ok(())
    }

    fn validate_point_source(
        &self,
        mesh: &TriMesh,
        operator: &QuadraticWaveOperator,
    ) -> Result<(), String> {
        let active = self
            .bundle
            .plan
            .domains
            .iter()
            .any(|domain| domain.region == self.point_source.region);
        if !self.point_source.valid() || !active {
            return Err("Point source references an inactive region".into());
        }
        if !self.point_source.enabled {
            return Ok(());
        }
        let stencil = QuadraticPointStencil::build_topology(
            mesh,
            operator,
            &self.bundle.plan,
            self.stripped_model(),
            self.point_source.position,
        )
        .map_err(|error| format!("Point source placement is invalid: {error}"))?;
        if stencil.region != self.point_source.region {
            return Err("Point source position does not lie in its assigned region".into());
        }
        Ok(())
    }

    /// Point-source placement depends on the candidate mesh/operator but not on
    /// any later assembly or transfer phase. In particular, do not repeat its
    /// linear mesh search for every cooperative work unit after assembly.
    fn validate_point_source_once(&mut self) -> Result<(), String> {
        if self.point_source_validated {
            return Ok(());
        }
        let mesh = self.mesh.as_ref().unwrap().clone();
        let operator = self.operator.as_ref().unwrap().clone();
        #[cfg(test)]
        {
            self.point_source_validation_count =
                self.point_source_validation_count.saturating_add(1);
        }
        self.ensure_stripped_model();
        self.validate_point_source(&mesh, &operator)?;
        self.point_source_validated = true;
        Ok(())
    }

    fn prepared_solver_update(&self) -> PreparedSolverUpdate {
        if self.fresh || !self.operator_reused || self.require_solver_handoff {
            return PreparedSolverUpdate::FullHandoff;
        }
        let Some(previous) = &self.previous else {
            return PreparedSolverUpdate::FullHandoff;
        };
        let Some(forcing) = &self.canonical_forcing else {
            return PreparedSolverUpdate::FullHandoff;
        };
        if forcing.as_ref() == previous.canonical_forcing.as_ref() {
            PreparedSolverUpdate::MeasurementsOnly
        } else if forcing_layout_eq(forcing, &previous.canonical_forcing) {
            PreparedSolverUpdate::SourceDrivesOnly
        } else if forcing_sparse_layout_eq(forcing, &previous.canonical_forcing) {
            PreparedSolverUpdate::SourceWeightsOnly
        } else {
            PreparedSolverUpdate::FullHandoff
        }
    }

    fn fail(&mut self, message: String) -> TopologyPreparationError {
        let phase = self.phase;
        self.done = true;
        self.phase = TopologyPreparationPhase::Failed;
        TopologyPreparationError {
            token: self.bundle.token,
            phase,
            message,
        }
    }
}

fn operator_scene_eq(left: &TopologyScene, right: &TopologyScene) -> bool {
    left.geometry == right.geometry
        && left.physics == right.physics
        && left.materials == right.materials
        && left.regions == right.regions
        && left.face_assignments == right.face_assignments
        && left.outer_boundaries == right.outer_boundaries
}

fn mesh_scene_eq(left: &TopologyScene, right: &TopologyScene) -> bool {
    left.geometry == right.geometry
        && left.face_assignments == right.face_assignments
        && left.regions.len() == right.regions.len()
        && left
            .regions
            .iter()
            .zip(&right.regions)
            .all(|(left, right)| left.id == right.id)
}

fn volume_source_layout_eq(left: &[VolumeSource], right: &[VolumeSource]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.region == right.region
                && left.enabled == right.enabled
                && left.profile == right.profile
                && left.parameters == right.parameters
        })
}

fn forcing_layout_eq(left: &CanonicalForcing, right: &CanonicalForcing) -> bool {
    left.prescribed() == right.prescribed()
        && left.sources().len() == right.sources().len()
        && left
            .sources()
            .iter()
            .zip(right.sources())
            .all(|(left, right)| {
                left.support() == right.support() && left.weights() == right.weights()
            })
}

fn forcing_sparse_layout_eq(left: &CanonicalForcing, right: &CanonicalForcing) -> bool {
    left.prescribed() == right.prescribed()
        && left.sources().len() == right.sources().len()
        && left
            .sources()
            .iter()
            .zip(right.sources())
            .all(|(left, right)| left.support() == right.support())
}

pub struct TopologyRuntime {
    active: Option<Arc<PreparedTopology>>,
    preparing: Option<TopologyPreparationJob>,
    ready: Option<PreparedTopology>,
    requested: Option<TopologyToken>,
    /// Whether `requested` came from adaptation rather than an edit. Any
    /// preparation or upload in flight belongs to it, since an edit's request
    /// replaces an adaptation's.
    requested_adaptation: bool,
    next_mesh_revision: u64,
    last_error: Option<TopologyPreparationError>,
    /// Consumed by the next `request`: rebuild the mesh even for an unchanged
    /// plan with unchanged options.
    force_rebuild: bool,
    /// Whether repairs refill a carved band at the sizes adaptation requested
    /// there; see `set_preserve_adaptation`.
    preserve_adaptation: bool,
}

impl Default for TopologyRuntime {
    fn default() -> Self {
        Self {
            active: None,
            preparing: None,
            ready: None,
            requested: None,
            requested_adaptation: false,
            next_mesh_revision: 1,
            last_error: None,
            force_rebuild: false,
            preserve_adaptation: true,
        }
    }
}

impl TopologyRuntime {
    /// Makes the next `request` rebuild the mesh even when the plan and the
    /// meshing options are unchanged, for instance to leave an adapted mesh.
    pub fn request_full_rebuild(&mut self) {
        self.force_rebuild = true;
    }

    /// Whether repairs refill a carved band at the sizes adaptation requested
    /// there. Off, the band comes back at the meshing target, so once
    /// adaptation is disabled an adapted mesh coarsens wherever it is
    /// repaired. A mesh no adaptation has touched refills at the target
    /// either way.
    pub fn set_preserve_adaptation(&mut self, preserve: bool) {
        self.preserve_adaptation = preserve;
    }

    pub fn reserve_mesh_revision(&mut self) -> u64 {
        let revision = self.next_mesh_revision;
        self.next_mesh_revision = self.next_mesh_revision.wrapping_add(1).max(1);
        revision
    }

    pub fn request(
        &mut self,
        document_revision: u64,
        document: &TopologyDocument,
        compiled: CompiledTopologyScene,
        options: MeshingOptions,
        fresh: bool,
    ) -> Result<TopologyToken, String> {
        self.request_with_handoff(document_revision, document, compiled, options, fresh, false)
    }

    pub fn request_with_handoff(
        &mut self,
        document_revision: u64,
        document: &TopologyDocument,
        compiled: CompiledTopologyScene,
        options: MeshingOptions,
        fresh: bool,
        require_solver_handoff: bool,
    ) -> Result<TopologyToken, String> {
        let mesh_revision = self.reserve_mesh_revision();
        let job = TopologyPreparationJob::new(
            document_revision,
            document,
            compiled,
            self.active.clone(),
            mesh_revision,
            options,
            PreparationIntent {
                fresh,
                force_rebuild: std::mem::take(&mut self.force_rebuild),
                preserve_adaptation: self.preserve_adaptation,
                require_solver_handoff,
            },
        )?;
        let token = job.token();
        self.preparing = Some(job);
        self.ready = None;
        self.requested = Some(token);
        self.requested_adaptation = false;
        self.last_error = None;
        Ok(token)
    }

    /// Continues the normal preparation and GPU-acknowledged publication path
    /// from a fixed-topology AMR result.
    pub fn request_adapted(
        &mut self,
        document_revision: u64,
        document: &TopologyDocument,
        mesh: TriMesh,
    ) -> Result<TopologyToken, String> {
        let previous = self
            .active
            .clone()
            .ok_or_else(|| "Cannot adapt before a topology is active".to_owned())?;
        let job = TopologyPreparationJob::new_adapted(document_revision, document, previous, mesh)?;
        let token = job.token();
        self.preparing = Some(job);
        self.ready = None;
        self.requested = Some(token);
        self.requested_adaptation = true;
        self.last_error = None;
        Ok(token)
    }

    /// Whether the latest request came from adaptation rather than an edit,
    /// so whether what is preparing or uploading is the app's own work.
    pub fn requested_adaptation(&self) -> bool {
        self.requested_adaptation
    }

    pub fn phase(&self) -> Option<TopologyPreparationPhase> {
        self.preparing
            .as_ref()
            .map(TopologyPreparationJob::phase)
            .or_else(|| self.ready.as_ref().map(|_| TopologyPreparationPhase::Ready))
    }

    pub fn detail(&self) -> Option<&'static str> {
        self.preparing.as_ref().map(TopologyPreparationJob::detail)
    }

    /// Live breakdown of the candidate still being prepared.
    pub fn preparing_timing(&self) -> Option<TopologyPreparationTiming> {
        self.preparing.as_ref().map(TopologyPreparationJob::timing)
    }

    /// Moves the current immutable preparation job to an external executor.
    /// The requested token remains owned here, so a result superseded by a
    /// newer request is still rejected by [`Self::finish_external_preparation`].
    pub fn take_preparing_job(&mut self) -> Option<TopologyPreparationJob> {
        self.preparing.take()
    }

    /// Restores a job when an optional external executor is unavailable.
    pub fn restore_preparing_job(&mut self, job: TopologyPreparationJob) {
        if self.requested == Some(job.token()) && self.preparing.is_none() {
            self.preparing = Some(job);
        }
    }

    /// Settles a result produced outside the calling thread. Token ownership is
    /// checked exactly like the cooperative path, and a stale result cannot
    /// clear or replace a newer in-process candidate.
    pub fn finish_external_preparation(
        &mut self,
        result: Result<PreparedTopology, TopologyPreparationError>,
    ) -> Option<Result<TopologyToken, TopologyPreparationError>> {
        self.settle(result)
    }

    pub fn active(&self) -> Option<&Arc<PreparedTopology>> {
        self.active.as_ref()
    }

    /// Forgets the active topology and whatever was prepared or preparing,
    /// as at launch. A document replacement drops the outgoing scene rather
    /// than keep it running under the new one, and the new scene's first
    /// preparation is then a fresh full build. Mesh revisions keep counting.
    pub fn clear_active(&mut self) {
        self.active = None;
        self.preparing = None;
        self.ready = None;
        self.requested = None;
        self.requested_adaptation = false;
        self.last_error = None;
    }

    pub fn ready(&self) -> Option<&PreparedTopology> {
        self.ready.as_ref()
    }

    pub fn last_error(&self) -> Option<&TopologyPreparationError> {
        self.last_error.as_ref()
    }

    pub fn advance(
        &mut self,
        budget: usize,
    ) -> Option<Result<TopologyToken, TopologyPreparationError>> {
        let result = self.preparing.as_mut()?.advance(budget)?;
        self.preparing = None;
        self.settle(result)
    }

    /// Time-budgeted counterpart of `advance`; see
    /// `TopologyPreparationJob::advance_for`.
    pub fn advance_for(
        &mut self,
        budget: Duration,
    ) -> Option<Result<TopologyToken, TopologyPreparationError>> {
        let result = self.preparing.as_mut()?.advance_for(budget)?;
        self.preparing = None;
        self.settle(result)
    }

    fn settle(
        &mut self,
        result: Result<PreparedTopology, TopologyPreparationError>,
    ) -> Option<Result<TopologyToken, TopologyPreparationError>> {
        match result {
            Ok(candidate) if Some(candidate.bundle.token) == self.requested => {
                let token = candidate.bundle.token;
                self.ready = Some(candidate);
                Some(Ok(token))
            }
            Ok(_) => None,
            Err(error) if Some(error.token) == self.requested => {
                self.last_error = Some(error.clone());
                Some(Err(error))
            }
            Err(_) => None,
        }
    }

    /// Publishes the CPU candidate only after the caller has completed GPU upload.
    pub fn commit_ready(&mut self, token: TopologyToken) -> Result<Arc<PreparedTopology>, String> {
        if self.requested != Some(token)
            || self
                .ready
                .as_ref()
                .is_none_or(|candidate| candidate.bundle.token != token)
        {
            return Err("Candidate is stale or not ready".into());
        }
        let committed = Arc::new(self.ready.take().unwrap());
        self.active = Some(committed.clone());
        Ok(committed)
    }

    pub fn reject_ready(&mut self, token: TopologyToken, message: impl Into<String>) {
        if self
            .ready
            .as_ref()
            .is_some_and(|candidate| candidate.bundle.token == token)
        {
            self.ready = None;
            self.last_error = Some(TopologyPreparationError {
                token,
                phase: TopologyPreparationPhase::Failed,
                message: message.into(),
            });
        }
    }
}

fn compile_canonical_forcing(
    mesh: &TriMesh,
    quadratic: &QuadraticWaveOperator,
    canonical: &CanonicalWaveOperator,
    volume: &CompiledVolumeSources,
    authored_volume: &[VolumeSource],
    point: PointSource,
) -> Result<CanonicalForcing, String> {
    let mut forcing = CanonicalForcing::from_legacy_boundaries(canonical, quadratic, 0.0)
        .map_err(|error| error.to_string())?;
    let mut membership = vec![false; canonical.degrees_of_freedom()];
    for (triangle, nodes) in mesh.triangles.iter().zip(canonical.element_nodes()) {
        if triangle.region == point.region {
            for node in nodes {
                membership[*node as usize] = true;
            }
        }
    }
    forcing
        .push_source(
            CanonicalForcing::legacy_point_source(canonical, point, &membership, 0.0)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
    forcing
        .extend_legacy_volume_slots(canonical, volume, authored_volume, 0.0)
        .map_err(|error| error.to_string())?;
    Ok(forcing)
}

fn compile_probes(
    probes: &[TopologyProbeDefinition],
    mesh: &TriMesh,
    operator: &QuadraticWaveOperator,
    bundle: &AcceptedTopology,
    model: TopologyWaveModel<'_>,
) -> Vec<CompiledTopologyProbe> {
    probes
        .iter()
        .map(|probe| {
            let result = if !probe.enabled {
                TopologyProbeCompilation::Disabled
            } else {
                compile_probe(probe, mesh, operator, bundle, model)
                    .map(Box::new)
                    .map(TopologyProbeCompilation::Ready)
                    .unwrap_or_else(TopologyProbeCompilation::Failed)
            };
            CompiledTopologyProbe {
                id: probe.id,
                result,
            }
        })
        .collect()
}

fn compile_probes_reusing(
    probes: &[TopologyProbeDefinition],
    mesh: &TriMesh,
    operator: &QuadraticWaveOperator,
    bundle: &AcceptedTopology,
    model: TopologyWaveModel<'_>,
    previous: Option<&PreparedTopology>,
) -> Vec<CompiledTopologyProbe> {
    probes
        .iter()
        .map(|probe| {
            if let Some(previous) = previous
                && previous
                    .probe_definitions
                    .iter()
                    .any(|definition| definition == probe)
                && let Some(compiled) = previous.probes.iter().find(|entry| entry.id == probe.id)
            {
                return compiled.clone();
            }
            compile_probes(std::slice::from_ref(probe), mesh, operator, bundle, model)
                .pop()
                .expect("one probe compiles to one result")
        })
        .collect()
}

fn compile_probe(
    probe: &TopologyProbeDefinition,
    mesh: &TriMesh,
    operator: &QuadraticWaveOperator,
    bundle: &AcceptedTopology,
    model: TopologyWaveModel<'_>,
) -> Result<TopologyProbeStencil, String> {
    match &probe.target {
        TopologyProbeTarget::Point(point) => {
            QuadraticPointStencil::build_topology(mesh, operator, &bundle.plan, model, *point)
                .map(TopologyProbeStencil::Point)
                .map_err(|error| error.to_string())
        }
        TopologyProbeTarget::Segment { start, end, preset } => {
            let count = preset.spatial_points();
            (0..count)
                .map(|index| {
                    let fraction = index as f64 / (count - 1) as f64;
                    QuadraticPointStencil::build_topology(
                        mesh,
                        operator,
                        &bundle.plan,
                        model,
                        start.lerp(*end, fraction),
                    )
                    .map_err(|error| error.to_string())
                })
                .collect::<Result<Vec<_>, _>>()
                .map(TopologyProbeStencil::Segment)
        }
        TopologyProbeTarget::Boundary(target) => {
            compile_boundary_probe(target, mesh, operator, bundle)
                .map(TopologyProbeStencil::Boundary)
        }
        TopologyProbeTarget::AreaDisk { center, radius } => QuadraticAreaStencil::build_topology(
            mesh,
            operator,
            &bundle.plan,
            model,
            AreaProbeShape::Disk {
                center: *center,
                radius: *radius,
            },
        )
        .map(TopologyProbeStencil::Area)
        .map_err(|error| error.to_string()),
        TopologyProbeTarget::AreaRegion(region) => QuadraticAreaStencil::build_topology(
            mesh,
            operator,
            &bundle.plan,
            model,
            AreaProbeShape::Region(*region),
        )
        .map(TopologyProbeStencil::Area)
        .map_err(|error| error.to_string()),
    }
}

fn compile_boundary_probe(
    target: &TopologyBoundaryProbeTarget,
    mesh: &TriMesh,
    operator: &QuadraticWaveOperator,
    bundle: &AcceptedTopology,
) -> Result<Vec<QuadraticBoundaryStencil>, String> {
    let curve = bundle
        .authored
        .geometry
        .curves
        .iter()
        .find(|curve| curve.id == target.curve)
        .ok_or("Boundary probe curve is missing")?;
    let mut pieces = Vec::new();
    for span in &target.spans {
        let mut span_pieces = bundle
            .plan
            .boundaries
            .iter()
            .filter(|boundary| {
                boundary.source
                    == PlannedBoundarySource::Curve {
                        curve: target.curve,
                        span: *span,
                        side: target.side,
                    }
            })
            .copied()
            .collect::<Vec<_>>();
        span_pieces.sort_by(|left, right| left.parameter[0].total_cmp(&right.parameter[0]));
        if span_pieces.is_empty() {
            return Err("Boundary probe span has no active trace".into());
        }
        pieces.extend(span_pieces);
    }
    if target.reversed {
        pieces.reverse();
    }
    let lengths = pieces
        .iter()
        .map(|piece| (piece.points[1] - piece.points[0]).norm())
        .collect::<Vec<_>>();
    let total = lengths.iter().sum::<f64>();
    if !total.is_finite() || total <= 0.0 {
        return Err("Boundary probe path is degenerate".into());
    }
    let period = curve
        .spline
        .span_bounds(curve.spans.len().saturating_sub(1))
        .map(|bounds| bounds[1])
        .filter(|period| period.is_finite() && *period > 0.0)
        .ok_or("Boundary probe curve has an invalid period")?;
    let count = target.preset.spatial_points();
    (0..count)
        .map(|index| {
            let mut distance = total * index as f64 / (count - 1) as f64;
            let mut selected = pieces.len() - 1;
            for (piece_index, length) in lengths.iter().copied().enumerate() {
                if distance <= length || piece_index + 1 == pieces.len() {
                    selected = piece_index;
                    break;
                }
                distance -= length;
            }
            let piece = pieces[selected];
            let fraction = if lengths[selected] > 0.0 {
                (distance / lengths[selected]).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let parameter = if target.reversed {
                piece.parameter[1] + (piece.parameter[0] - piece.parameter[1]) * fraction
            } else {
                piece.parameter[0] + (piece.parameter[1] - piece.parameter[0]) * fraction
            };
            QuadraticBoundaryStencil::build_topology(
                mesh,
                operator,
                &bundle.plan,
                bundle.model(),
                BoundaryStencilTarget {
                    label: BoundaryLabel::Curve {
                        curve: target.curve,
                        span: match piece.source {
                            PlannedBoundarySource::Curve { span, .. } => span,
                            PlannedBoundarySource::Outer(_) => unreachable!(),
                        },
                        side: target.side,
                        separated: matches!(piece.behavior, Some(SpanBehavior::Separated { .. })),
                    },
                    parameter,
                    period,
                    region: piece.region,
                },
            )
            .map_err(|error| error.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topology_editor::{
        ClosedCurvePurpose, OpenCurvePurpose, TopologyAcceptance, TopologyAttachment,
        TopologyEditor,
    };
    use crate::topology_viewport::{
        SampledTopologyGeometry, ScreenPoint, TopologySpanTarget, ViewportTransform,
    };

    fn options() -> MeshingOptions {
        MeshingOptions {
            curve_tolerance: 1.0e-3,
            target_edge_length: 0.18,
            minimum_angle_degrees: 10.0,
            max_vertices: 20_000,
            max_triangles: 40_000,
            max_refinement_steps: 20_000,
        }
    }

    fn settle(editor: &mut TopologyEditor) {
        for _ in 0..100_000 {
            editor.validate_frame(64);
            if editor.acceptance != TopologyAcceptance::Pending {
                return;
            }
        }
        panic!("topology validation did not finish");
    }

    fn prepare(runtime: &mut TopologyRuntime) -> Result<TopologyToken, TopologyPreparationError> {
        for _ in 0..2_000_000 {
            if let Some(result) = runtime.advance(1) {
                return result;
            }
        }
        panic!("topology runtime did not finish");
    }

    fn prepare_job(
        mut job: TopologyPreparationJob,
    ) -> Result<PreparedTopology, TopologyPreparationError> {
        for _ in 0..2_000_000 {
            if let Some(result) = job.advance(64) {
                return result;
            }
        }
        panic!("detached topology job did not finish");
    }

    /// A document carrying a material drive must prepare at all.
    ///
    /// Before both assemblies were given the law-stripped model this failed
    /// outright: the fixed compilers refuse a law-carrying material, by the
    /// gate that stops a law being executed as a static medium. So a pump was
    /// not merely unsupported in the application - a document with one could
    /// not be loaded.
    #[test]
    fn a_document_with_a_material_drive_prepares_and_carries_its_temporal_operator() {
        let inert = {
            let editor = TopologyEditor::default();
            let mut runtime = TopologyRuntime::default();
            let token = runtime
                .request(
                    editor.revision,
                    &editor.document,
                    editor.compiled_accepted.clone(),
                    options(),
                    true,
                )
                .unwrap();
            prepare(&mut runtime).unwrap();
            runtime.commit_ready(token).unwrap()
        };
        assert!(
            inert.canonical_temporal_operator.is_none(),
            "an inert document must take the fixed path unchanged"
        );

        let mut document = TopologyEditor::default().document;
        let drive = funfern_core::TimeDrive::ParametricPump {
            depth: funfern_core::ScalarField::constant(0.2),
            frequency_hz: funfern_core::ScalarField::constant(1.0),
            phase_radians: funfern_core::ScalarField::constant(0.25),
        };
        document.model.draft.materials[0].mass_law.drive = drive.clone();
        document.model.accepted.materials[0].mass_law.drive = drive;
        let editor = TopologyEditor::from_document(document).unwrap();
        let mut runtime = TopologyRuntime::default();
        let token = runtime
            .request(
                editor.revision,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                true,
            )
            .unwrap();
        prepare(&mut runtime).unwrap();
        let driven = runtime.commit_ready(token).unwrap();

        let temporal = driven
            .canonical_temporal_operator
            .as_ref()
            .expect("a driven document must carry a temporal operator");
        // The base is shared rather than copied: one assembly, held twice.
        assert!(std::ptr::eq(
            Arc::as_ptr(&driven.canonical_operator),
            temporal.base() as *const CanonicalWaveOperator
        ));
        // And the drive is really in it, with the tighter trajectory bound.
        assert!(temporal.maximum_time_step() < driven.canonical_operator.maximum_time_step());
    }

    #[test]
    fn complete_candidate_publishes_only_after_explicit_gpu_acknowledgement() {
        let editor = TopologyEditor::default();
        let mut runtime = TopologyRuntime::default();
        let token = runtime
            .request(
                editor.revision,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                true,
            )
            .unwrap();
        assert!(runtime.active().is_none());
        assert_eq!(prepare(&mut runtime).unwrap(), token);
        assert!(runtime.active().is_none());
        assert_eq!(runtime.ready().unwrap().bundle.token, token);
        let committed = runtime.commit_ready(token).unwrap();
        assert_eq!(committed.bundle.token, token);
        assert!(committed.transfer.is_none());
        assert_eq!(committed.mesh.geometry_revision, token.topology_revision);
        assert_eq!(
            committed.operator.mesh_revision(),
            committed.mesh.mesh_revision
        );
    }

    #[test]
    fn externally_prepared_candidate_obeys_the_runtime_transaction() {
        let editor = TopologyEditor::default();
        let mut runtime = TopologyRuntime::default();
        let token = runtime
            .request(
                editor.revision,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                true,
            )
            .unwrap();
        let job = runtime.take_preparing_job().unwrap();
        assert_eq!(job.token(), token);
        assert!(runtime.phase().is_none());

        let result = prepare_job(job);
        assert_eq!(
            runtime
                .finish_external_preparation(result)
                .unwrap()
                .unwrap(),
            token
        );
        assert_eq!(runtime.ready().unwrap().bundle.token, token);
        assert_eq!(runtime.commit_ready(token).unwrap().bundle.token, token);
    }

    #[test]
    fn externally_prepared_stale_candidate_cannot_replace_a_newer_request() {
        let editor = TopologyEditor::default();
        let mut runtime = TopologyRuntime::default();
        let stale = runtime
            .request(
                1,
                &editor.document,
                editor.document.model.accepted.compile(1).unwrap(),
                options(),
                true,
            )
            .unwrap();
        let stale_job = runtime.take_preparing_job().unwrap();
        let newest = runtime
            .request(
                2,
                &editor.document,
                editor.document.model.accepted.compile(2).unwrap(),
                options(),
                true,
            )
            .unwrap();
        assert_ne!(stale, newest);

        assert!(
            runtime
                .finish_external_preparation(prepare_job(stale_job))
                .is_none()
        );
        assert!(runtime.ready().is_none());
        assert_eq!(runtime.take_preparing_job().unwrap().token(), newest);
    }

    /// Assembly and the transfer map yield like the mesh: a stepped preparation
    /// passes through both phases across many slices, and what it produces
    /// equals the one-shot operator and map.
    #[test]
    fn assembly_and_transfer_yield_between_slices() {
        let editor = TopologyEditor::default();
        let mut runtime = TopologyRuntime::default();
        let request = |runtime: &mut TopologyRuntime, fresh| {
            runtime
                .request(
                    editor.revision,
                    &editor.document,
                    editor.compiled_accepted.clone(),
                    options(),
                    fresh,
                )
                .unwrap()
        };
        let token = request(&mut runtime, true);
        assert_eq!(prepare(&mut runtime).unwrap(), token);
        let first = runtime.commit_ready(token).unwrap();

        runtime.request_full_rebuild();
        let token = request(&mut runtime, false);
        let mut seen = std::collections::BTreeMap::<&'static str, u32>::new();
        let finished = loop {
            if let Some(phase) = runtime.phase() {
                *seen.entry(phase.label()).or_default() += 1;
            }
            if let Some(result) = runtime.advance(64) {
                break result.unwrap();
            }
            assert!(
                seen.values().sum::<u32>() < 1_000_000,
                "preparation did not finish"
            );
        };
        assert_eq!(finished, token);
        let second = runtime.commit_ready(token).unwrap();
        assert!(seen["Assembling wave operator"] > 1, "{seen:?}");
        assert!(seen["Carrying the field across"] > 1, "{seen:?}");

        let expected = QuadraticWaveOperator::assemble_topology(
            &second.mesh,
            &second.bundle.plan,
            second.bundle.model(),
        )
        .unwrap();
        assert_eq!(*second.operator, expected);
        let map = QuadraticTransferMap::build(
            &first.mesh,
            &first.operator,
            &second.mesh,
            &second.operator,
        )
        .unwrap();
        assert_eq!(**second.transfer.as_ref().unwrap(), map);
    }

    /// A frame lends the preparation wall time, not a step count. Without a
    /// deadline one call finishes a fresh preparation and reports one slice,
    /// with the same mesh the step-counted path produces; with a zero budget
    /// every call still makes one slice of progress.
    #[test]
    fn a_time_budget_finishes_a_preparation_in_one_call_without_a_deadline() {
        let editor = TopologyEditor::default();
        let request = |runtime: &mut TopologyRuntime| {
            runtime
                .request(
                    editor.revision,
                    &editor.document,
                    editor.compiled_accepted.clone(),
                    options(),
                    true,
                )
                .unwrap()
        };

        let mut unbounded = TopologyRuntime::default();
        let token = request(&mut unbounded);
        let finished = unbounded
            .advance_for(Duration::from_secs(600))
            .expect("one unbounded call finishes the preparation")
            .unwrap();
        assert_eq!(finished, token);
        let committed = unbounded.commit_ready(token).unwrap();
        assert_eq!(committed.timing.slices, 1);

        let mut stepped = TopologyRuntime::default();
        let token = request(&mut stepped);
        assert_eq!(prepare(&mut stepped).unwrap(), token);
        let reference = stepped.commit_ready(token).unwrap();
        assert_eq!(*committed.mesh, *reference.mesh);

        let mut bounded = TopologyRuntime::default();
        let token = request(&mut bounded);
        let mut calls = 0u32;
        let finished = loop {
            calls += 1;
            if let Some(result) = bounded.advance_for(Duration::ZERO) {
                break result.unwrap();
            }
            assert!(calls < 1_000_000, "preparation did not finish");
        };
        assert_eq!(finished, token);
        assert!(calls > 1, "a zero budget still yields after one slice");
        assert_eq!(bounded.commit_ready(token).unwrap().timing.slices, calls);
    }

    #[test]
    fn point_source_placement_is_validated_once_per_candidate() {
        let mut editor = TopologyEditor::default();
        editor.document.model.source.enabled = true;
        let mut runtime = TopologyRuntime::default();
        let token = runtime
            .request(
                editor.revision,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                true,
            )
            .unwrap();
        let mut observed = 0;
        let finished = loop {
            if let Some(job) = runtime.preparing.as_ref() {
                observed = observed.max(job.point_source_validation_count);
                assert!(
                    job.point_source_validation_count <= 1,
                    "point-source placement was revalidated during later work"
                );
            }
            if let Some(result) = runtime.advance(1) {
                break result.unwrap();
            }
        };
        assert_eq!(finished, token);
        assert_eq!(observed, 1);
        let timing = runtime.commit_ready(token).unwrap().timing;
        assert!(timing.work_ms > 0.0);
        assert!(timing.work_ms >= timing.categorized_ms() * 0.99);
    }

    /// The accepted bundle carries the plan the mesh is built from, whose atoms
    /// merge arrangement segments within the meshing tolerance; coarser options
    /// give fewer atoms, and the editor's own compiled plan stays fine.
    #[test]
    fn the_accepted_plan_is_coarsened_to_the_meshing_options() {
        let mut editor = TopologyEditor::default();
        editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.3),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        settle(&mut editor);
        let atoms = |curve_tolerance: f64| {
            let mut runtime = TopologyRuntime::default();
            let token = runtime
                .request(
                    editor.revision,
                    &editor.document,
                    editor.compiled_accepted.clone(),
                    MeshingOptions {
                        curve_tolerance,
                        ..options()
                    },
                    true,
                )
                .unwrap();
            assert_eq!(prepare(&mut runtime).unwrap(), token);
            runtime
                .commit_ready(token)
                .unwrap()
                .bundle
                .plan
                .boundaries
                .len()
        };
        let fine = editor.compiled_accepted.plan.boundaries.len();
        let (tight, loose) = (atoms(1.0e-6), atoms(1.0e-3));
        assert!(tight <= fine, "{tight} atoms from {fine} segments");
        assert!(
            loose * 2 < tight,
            "{loose} loose against {tight} tight atoms"
        );
    }

    /// A resolution change rebuilds the mesh although the plan is unchanged:
    /// the operator follows the new mesh and the running field crosses through
    /// a transfer map. The same options again are an ordinary reuse.
    #[test]
    fn changing_the_meshing_options_rebuilds_the_mesh() {
        let editor = TopologyEditor::default();
        let mut runtime = TopologyRuntime::default();
        let coarse = options();
        let request = |runtime: &mut TopologyRuntime, revision, options, fresh| {
            runtime
                .request(
                    revision,
                    &editor.document,
                    editor.compiled_accepted.clone(),
                    options,
                    fresh,
                )
                .unwrap()
        };
        let token = request(&mut runtime, editor.revision, coarse, true);
        assert_eq!(prepare(&mut runtime).unwrap(), token);
        let first = runtime.commit_ready(token).unwrap();
        assert_eq!(first.meshing, coarse);

        let fine = MeshingOptions {
            target_edge_length: 0.09,
            ..coarse
        };
        let token = request(&mut runtime, editor.revision, fine, false);
        assert_eq!(prepare(&mut runtime).unwrap(), token);
        let second = runtime.commit_ready(token).unwrap();
        assert_eq!(
            second.mesh_action,
            TopologyMeshUpdateAction::FullRebuild(TopologyFullRebuildReason::MeshingOptionsChanged)
        );
        assert!(!second.operator_reused);
        assert!(second.transfer.is_some());
        assert!(second.mesh.triangles.len() > first.mesh.triangles.len());
        assert_eq!(second.meshing, fine);

        let token = request(&mut runtime, editor.revision + 1, fine, false);
        assert_eq!(prepare(&mut runtime).unwrap(), token);
        let third = runtime.commit_ready(token).unwrap();
        assert_eq!(third.mesh_action, TopologyMeshUpdateAction::Reuse);
        assert!(third.operator_reused);
    }

    /// A requested rebuild remeshes an unchanged scene once, which is how the
    /// user returns from an adapted mesh to the base resolution.
    #[test]
    fn a_requested_rebuild_remeshes_an_unchanged_scene_once() {
        let editor = TopologyEditor::default();
        let mut runtime = TopologyRuntime::default();
        let request = |runtime: &mut TopologyRuntime, revision, fresh| {
            runtime
                .request(
                    revision,
                    &editor.document,
                    editor.compiled_accepted.clone(),
                    options(),
                    fresh,
                )
                .unwrap()
        };
        let token = request(&mut runtime, editor.revision, true);
        assert_eq!(prepare(&mut runtime).unwrap(), token);
        let first = runtime.commit_ready(token).unwrap();

        runtime.request_full_rebuild();
        let token = request(&mut runtime, editor.revision, false);
        assert_eq!(prepare(&mut runtime).unwrap(), token);
        let second = runtime.commit_ready(token).unwrap();
        assert_eq!(
            second.mesh_action,
            TopologyMeshUpdateAction::FullRebuild(TopologyFullRebuildReason::Requested)
        );
        assert_ne!(second.mesh.mesh_revision, first.mesh.mesh_revision);
        assert!(!second.operator_reused);
        assert!(second.transfer.is_some());

        let token = request(&mut runtime, editor.revision + 1, false);
        assert_eq!(prepare(&mut runtime).unwrap(), token);
        assert_eq!(
            runtime.commit_ready(token).unwrap().mesh_action,
            TopologyMeshUpdateAction::Reuse
        );
    }

    /// The timing a handoff carries has to include the slice that finished it,
    /// which is usually the longest one and the whole point of the measurement.
    #[test]
    fn a_finished_preparation_counts_the_slice_that_finished_it() {
        let editor = TopologyEditor::default();
        let mut runtime = TopologyRuntime::default();
        let token = runtime
            .request(
                editor.revision,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                true,
            )
            .unwrap();
        let mut slices = 0u32;
        let finished = loop {
            slices += 1;
            if let Some(result) = runtime.advance(1) {
                break result.unwrap();
            }
            assert!(slices < 2_000_000, "preparation did not finish");
        };
        assert_eq!(finished, token);
        let committed = runtime.commit_ready(token).unwrap();
        assert!(slices > 1, "the preparation took more than one slice");
        assert_eq!(
            committed.timing.slices, slices,
            "the handoff dropped its own last slice"
        );
        assert!(committed.timing.longest_slice_ms >= 0.0);
    }

    #[test]
    fn accepted_bundle_rejects_a_compiled_snapshot_from_another_scene() {
        let authored = TopologyScene::default();
        let mut other = TopologyScene::default();
        other.geometry.domain.max_x = 1.5;
        let compiled = other.compile(4).unwrap();
        assert!(
            AcceptedTopology::new(
                7,
                1,
                authored,
                compiled,
                AtomCoarsening::from_meshing(options())
            )
            .is_err()
        );
    }

    #[test]
    fn material_only_revision_reuses_mesh_and_builds_an_exact_transfer() {
        let editor = TopologyEditor::default();
        let mut runtime = TopologyRuntime::default();
        let first = runtime
            .request(
                1,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                true,
            )
            .unwrap();
        prepare(&mut runtime).unwrap();
        let original = runtime.commit_ready(first).unwrap();
        let original_outgoing = original
            .canonical_operator
            .outgoing_boundary_handle()
            .unwrap();

        let mut document = editor.document.clone();
        document.model.accepted.materials[0].damping = ScalarField::constant(0.02);
        document.model.draft = document.model.accepted.clone();
        let compiled = document.model.accepted.compile(7).unwrap();
        let second = runtime
            .request(2, &document, compiled, options(), false)
            .unwrap();
        assert_eq!(prepare(&mut runtime).unwrap(), second);
        let candidate = runtime.ready().unwrap();
        assert_eq!(candidate.mesh_action, TopologyMeshUpdateAction::Reuse);
        assert_eq!(candidate.mesh.mesh_revision, original.mesh.mesh_revision);
        assert!(Arc::ptr_eq(&candidate.mesh, &original.mesh));
        assert!(!candidate.operator_reused);
        assert!(Arc::ptr_eq(
            &original_outgoing,
            &candidate
                .canonical_operator
                .outgoing_boundary_handle()
                .unwrap()
        ));
        assert!(candidate.transfer.is_some());
        assert!(
            candidate
                .canonical_transfer
                .as_ref()
                .unwrap()
                .complementary
                .is_identity()
        );
        runtime.commit_ready(second).unwrap();
    }

    #[test]
    fn adapted_mesh_gets_a_distinct_token_and_waits_for_gpu_acknowledgement() {
        let editor = TopologyEditor::default();
        let mut runtime = TopologyRuntime::default();
        let initial = runtime
            .request(
                editor.revision,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                true,
            )
            .unwrap();
        prepare(&mut runtime).unwrap();
        let active = runtime.commit_ready(initial).unwrap();
        let mut adapted_mesh = active.mesh.as_ref().clone();
        adapted_mesh.mesh_revision = runtime.reserve_mesh_revision();
        assert!(!runtime.requested_adaptation());
        let adapted = runtime
            .request_adapted(editor.revision, &editor.document, adapted_mesh)
            .unwrap();
        assert!(runtime.requested_adaptation());
        assert_eq!(adapted.document_revision, initial.document_revision);
        assert_eq!(adapted.topology_revision, initial.topology_revision);
        assert_ne!(adapted.mesh_generation, initial.mesh_generation);
        assert_eq!(prepare(&mut runtime).unwrap(), adapted);
        assert_eq!(runtime.active().unwrap().bundle.token, initial);
        assert!(runtime.ready().unwrap().adapted);
        assert_eq!(runtime.commit_ready(adapted).unwrap().bundle.token, adapted);
    }

    /// Cleared, the runtime is as at launch: nothing active, and the same
    /// document's next preparation is a full build, where without the clear
    /// it reuses the mesh.
    #[test]
    fn a_cleared_runtime_prepares_the_next_document_from_scratch() {
        let editor = TopologyEditor::default();
        let mut runtime = TopologyRuntime::default();
        let run = |runtime: &mut TopologyRuntime| {
            let token = runtime
                .request(
                    editor.revision,
                    &editor.document,
                    editor.compiled_accepted.clone(),
                    options(),
                    true,
                )
                .unwrap();
            prepare(runtime).unwrap();
            runtime.commit_ready(token).unwrap()
        };
        run(&mut runtime);
        assert!(matches!(
            run(&mut runtime).mesh_action,
            TopologyMeshUpdateAction::Reuse
        ));
        runtime.clear_active();
        assert!(runtime.active().is_none() && runtime.phase().is_none());
        assert!(matches!(
            run(&mut runtime).mesh_action,
            TopologyMeshUpdateAction::FullRebuild(_)
        ));
    }

    #[test]
    fn source_and_probe_only_request_keeps_topology_mesh_and_operator() {
        let editor = TopologyEditor::default();
        let mut runtime = TopologyRuntime::default();
        let first = runtime
            .request(
                1,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                true,
            )
            .unwrap();
        prepare(&mut runtime).unwrap();
        let active = runtime.commit_ready(first).unwrap();

        let mut document = editor.document.clone();
        document.model.source.signal = TimeSignal::harmonic(0.0, 2.0, 3.0, 0.0);
        document.model.probes.push(TopologyProbeDefinition {
            id: ProbeId(1),
            name: "receiver".into(),
            color: [91, 220, 194],
            enabled: true,
            target: TopologyProbeTarget::Point(Point2::new(0.25, 0.0)),
        });
        let second = runtime
            .request(
                2,
                &document,
                document.model.accepted.compile(99).unwrap(),
                options(),
                false,
            )
            .unwrap();
        assert_eq!(second.topology_revision, first.topology_revision);
        prepare(&mut runtime).unwrap();
        let candidate = runtime.ready().unwrap();
        assert!(candidate.operator_reused);
        assert!(Arc::ptr_eq(&candidate.mesh, &active.mesh));
        assert!(Arc::ptr_eq(&candidate.operator, &active.operator));
        assert!(candidate.transfer.is_none());
        assert!(candidate.canonical_transfer.is_none());
        assert_eq!(
            candidate.solver_update,
            PreparedSolverUpdate::SourceDrivesOnly
        );
    }

    #[test]
    fn probe_only_request_never_prepares_a_solver_transfer() {
        let editor = TopologyEditor::default();
        let mut runtime = TopologyRuntime::default();
        let first = runtime
            .request(
                1,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                true,
            )
            .unwrap();
        prepare(&mut runtime).unwrap();
        runtime.commit_ready(first).unwrap();

        let mut document = editor.document.clone();
        document.model.probes.push(TopologyProbeDefinition {
            id: ProbeId(1),
            name: "receiver".into(),
            color: [91, 220, 194],
            enabled: true,
            target: TopologyProbeTarget::Point(Point2::new(0.25, 0.0)),
        });
        let second = runtime
            .request(
                2,
                &document,
                document.model.accepted.compile(2).unwrap(),
                options(),
                false,
            )
            .unwrap();
        assert_eq!(prepare(&mut runtime).unwrap(), second);
        let candidate = runtime.ready().unwrap();
        assert_eq!(
            candidate.solver_update,
            PreparedSolverUpdate::MeasurementsOnly
        );
        assert!(candidate.canonical_transfer.is_none());
        assert_eq!(candidate.probes.len(), 1);
    }

    #[test]
    fn timestep_republish_keeps_the_full_identity_handoff() {
        let editor = TopologyEditor::default();
        let mut runtime = TopologyRuntime::default();
        let first = runtime
            .request(
                1,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                true,
            )
            .unwrap();
        prepare(&mut runtime).unwrap();
        runtime.commit_ready(first).unwrap();

        runtime
            .request_with_handoff(
                1,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                false,
                true,
            )
            .unwrap();
        prepare(&mut runtime).unwrap();
        let candidate = runtime.ready().unwrap();
        assert_eq!(candidate.solver_update, PreparedSolverUpdate::FullHandoff);
        assert!(candidate.canonical_transfer.is_some());
    }

    #[test]
    fn volume_source_signal_edit_reuses_operator_and_spatial_weights() {
        let editor = TopologyEditor::default();
        let mut document = editor.document.clone();
        let source = VolumeSource {
            region: BACKGROUND_REGION,
            enabled: true,
            profile: ScalarField::constant(1.0),
            parameters: vec![],
            signal: TimeSignal::harmonic(0.0, 1.0, 2.0, 0.0),
        };
        document.model.draft.volume_sources.push(source.clone());
        document.model.accepted.volume_sources.push(source);
        let mut runtime = TopologyRuntime::default();
        let first = runtime
            .request(
                1,
                &document,
                document.model.accepted.compile(1).unwrap(),
                options(),
                true,
            )
            .unwrap();
        prepare(&mut runtime).unwrap();
        let active = runtime.commit_ready(first).unwrap();

        document.model.draft.volume_sources[0].signal = TimeSignal::harmonic(0.0, 0.5, 4.0, 0.2);
        document.model.accepted.volume_sources[0].signal =
            document.model.draft.volume_sources[0].signal;
        runtime
            .request(
                2,
                &document,
                document.model.accepted.compile(2).unwrap(),
                options(),
                false,
            )
            .unwrap();
        prepare(&mut runtime).unwrap();
        let candidate = runtime.ready().unwrap();
        assert!(candidate.operator_reused);
        assert!(Arc::ptr_eq(&candidate.operator, &active.operator));
        assert_eq!(
            candidate.solver_update,
            PreparedSolverUpdate::SourceDrivesOnly
        );
        assert!(candidate.canonical_transfer.is_none());
    }

    #[test]
    fn point_source_motion_and_toggle_keep_the_structural_weight_layout() {
        point_source_edits_patch_the_running_generation(false);
    }

    /// A driven medium takes the same source patches: they rewrite the drive
    /// table and weights only, and the weights are normalized by the fixed
    /// reference mass (`canonical_gpu_temporal_live_source`).
    #[test]
    fn a_driven_medium_takes_source_edits_as_patches() {
        point_source_edits_patch_the_running_generation(true);
    }

    fn pumped(material: &mut funfern_core::Material) {
        material.mass_law.drive = funfern_core::TimeDrive::ParametricPump {
            depth: funfern_core::ScalarField::constant(0.2),
            frequency_hz: funfern_core::ScalarField::constant(1.0),
            phase_radians: funfern_core::ScalarField::constant(0.25),
        };
    }

    /// Prepares and commits `document`, returning whether the generation is
    /// driven and what became of its far field.
    fn far_field_of(
        runtime: &mut TopologyRuntime,
        revision: u64,
        document: &TopologyDocument,
        fresh: bool,
    ) -> (bool, Result<(), String>) {
        let token = runtime
            .request(
                revision,
                document,
                document.model.accepted.compile(revision).unwrap(),
                options(),
                fresh,
            )
            .unwrap();
        prepare(runtime).unwrap();
        let active = runtime.commit_ready(token).unwrap();
        let far_field = active
            .far_field
            .clone()
            .expect("the far field was asked for");
        (active.driven(), far_field.map(|_| ()))
    }

    /// A Kerr document prepares a field-dependent generation: the device
    /// executes its maps since Stage 9, so nothing between the document and
    /// the plan may drop the law or refuse it.
    #[test]
    fn a_field_dependent_document_prepares_a_field_dependent_generation() {
        let mut document = TopologyEditor::default().document;
        let kerr = funfern_core::FieldLaw::Polynomial {
            chi1: funfern_core::ScalarField::constant(0.0),
            chi2: funfern_core::ScalarField::constant(0.8),
            amplitude_bound: None,
        };
        document.model.draft.materials[0].stiffness_law.field = kerr.clone();
        document.model.accepted.materials[0].stiffness_law.field = kerr;
        let mut runtime = TopologyRuntime::default();
        let token = runtime
            .request(
                1,
                &document,
                document.model.accepted.compile(1).unwrap(),
                options(),
                true,
            )
            .unwrap();
        prepare(&mut runtime).unwrap();
        let active = runtime.commit_ready(token).unwrap();
        assert!(active.driven());
        assert!(
            active
                .canonical_temporal_operator
                .as_ref()
                .is_some_and(|operator| operator.has_field_laws())
        );
    }

    /// The projection integrates a time-invariant exterior, so a drive out
    /// there is refused - including one that arrives by editing a static
    /// document - while a driven subdomain inside the contour is not.
    #[test]
    fn the_far_field_refuses_a_driven_exterior_but_not_a_driven_interior() {
        let mut editor = TopologyEditor::default();
        editor.document.model.far_field.enabled = true;
        editor.document.model.far_field.inset = 0.08;

        let mut exterior = editor.document.clone();
        pumped(&mut exterior.model.draft.materials[0]);
        pumped(&mut exterior.model.accepted.materials[0]);
        let (driven, refused) = far_field_of(&mut TopologyRuntime::default(), 1, &exterior, true);
        assert!(driven);
        assert!(
            refused
                .as_ref()
                .is_err_and(|error| error.contains("time-invariant")),
            "{refused:?}"
        );

        let mut runtime = TopologyRuntime::default();
        let mut edited = editor.document.clone();
        far_field_of(&mut runtime, 1, &edited, true).1.unwrap();
        pumped(&mut edited.model.draft.materials[0]);
        pumped(&mut edited.model.accepted.materials[0]);
        assert!(far_field_of(&mut runtime, 2, &edited, false).1.is_err());

        let material = editor.add_material().unwrap();
        editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(0.0, 0.0), 0.3),
                ClosedCurvePurpose::Subdomain { material },
            )
            .unwrap();
        settle(&mut editor);
        let mut interior = editor.document.clone();
        for materials in [
            &mut interior.model.draft.materials,
            &mut interior.model.accepted.materials,
        ] {
            pumped(
                materials
                    .iter_mut()
                    .find(|entry| entry.id == material)
                    .unwrap(),
            );
        }
        let (driven, built) = far_field_of(&mut TopologyRuntime::default(), 1, &interior, true);
        assert!(driven, "the subdomain's drive must reach the generation");
        built.unwrap();
    }

    fn point_source_edits_patch_the_running_generation(driven: bool) {
        let editor = TopologyEditor::default();
        let mut document = editor.document.clone();
        document.model.source.enabled = true;
        if driven {
            let drive = funfern_core::TimeDrive::ParametricPump {
                depth: funfern_core::ScalarField::constant(0.2),
                frequency_hz: funfern_core::ScalarField::constant(1.0),
                phase_radians: funfern_core::ScalarField::constant(0.25),
            };
            document.model.draft.materials[0].mass_law.drive = drive.clone();
            document.model.accepted.materials[0].mass_law.drive = drive;
        }
        let mut runtime = TopologyRuntime::default();
        let first = runtime
            .request(
                1,
                &document,
                document.model.accepted.compile(1).unwrap(),
                options(),
                true,
            )
            .unwrap();
        prepare(&mut runtime).unwrap();
        assert_eq!(runtime.commit_ready(first).unwrap().driven(), driven);

        document.model.source.position.x += 0.05;
        let second = runtime
            .request(
                2,
                &document,
                document.model.accepted.compile(2).unwrap(),
                options(),
                false,
            )
            .unwrap();
        prepare(&mut runtime).unwrap();
        let candidate = runtime.ready().unwrap();
        assert_eq!(
            candidate.solver_update,
            PreparedSolverUpdate::SourceWeightsOnly
        );
        assert!(candidate.canonical_transfer.is_none());
        runtime.commit_ready(second).unwrap();

        document.model.source.enabled = false;
        runtime
            .request(
                3,
                &document,
                document.model.accepted.compile(3).unwrap(),
                options(),
                false,
            )
            .unwrap();
        prepare(&mut runtime).unwrap();
        let candidate = runtime.ready().unwrap();
        assert_eq!(
            candidate.solver_update,
            PreparedSolverUpdate::SourceWeightsOnly
        );
        assert!(candidate.canonical_transfer.is_none());
    }

    /// A hole with the running field's runtime around it, ready for edits.
    fn hole_runtime() -> (TopologyEditor, CurveId, TopologyRuntime) {
        let mut editor = TopologyEditor::default();
        let curve = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.2),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        settle(&mut editor);
        let mut runtime = TopologyRuntime::default();
        let first = runtime
            .request(
                editor.revision,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                true,
            )
            .unwrap();
        prepare(&mut runtime).unwrap();
        runtime.commit_ready(first).unwrap();
        (editor, curve, runtime)
    }

    /// Switching a hole's boundary to transmit leaves every atom where it
    /// was - the far side is excluded, so the span is walled either way - and
    /// the mesh is reused. The arrangement renumbers the traces across that
    /// switch, though, and the reused mesh has to come over with them or the
    /// next adaptation refuses the pair. Both directions, and back again.
    #[test]
    fn switching_a_walled_boundary_renumbers_the_reused_mesh() {
        let (mut editor, curve, mut runtime) = hole_runtime();
        let spans = editor
            .document
            .model
            .draft
            .geometry
            .curve(curve)
            .unwrap()
            .spans
            .iter()
            .map(|span| span.id)
            .collect::<std::collections::BTreeSet<_>>();
        for behavior in [
            SpanBehavior::Transmitting,
            SpanBehavior::REFLECTING,
            SpanBehavior::Transmitting,
        ] {
            editor.set_span_behavior(&spans, behavior).unwrap();
            settle(&mut editor);
            let token = runtime
                .request(
                    editor.revision,
                    &editor.document,
                    editor.compiled_accepted.clone(),
                    options(),
                    false,
                )
                .unwrap();
            assert_eq!(prepare(&mut runtime).unwrap(), token);
            let reused = runtime.commit_ready(token).unwrap();
            assert_eq!(
                reused.mesh_action,
                TopologyMeshUpdateAction::Reuse,
                "a walled span is walled either way, so nothing is remeshed"
            );
            let planned = reused
                .bundle
                .plan
                .vertices
                .iter()
                .map(|vertex| vertex.id)
                .collect::<std::collections::BTreeSet<_>>();
            let carried = reused
                .mesh
                .vertices
                .iter()
                .filter_map(|vertex| vertex.trace)
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(planned, carried, "the reused mesh names the plan's traces");

            let fine = options().target_edge_length * 0.4;
            let mut adaptation = MeshAdaptationJob::new_topology(
                reused.mesh.clone(),
                &reused.bundle.plan,
                MeshAdaptationState::from_mesh(&reused.mesh),
                runtime.reserve_mesh_revision(),
                Arc::new(move |point: Point2, _| {
                    if point.x > 0.0 {
                        fine
                    } else {
                        options().target_edge_length
                    }
                }),
                MeshAdaptationOptions {
                    meshing: options(),
                    minimum_target_edge_length: fine,
                    maximum_target_edge_length: options().target_edge_length,
                    max_topology_changes: 8_000,
                    max_work_units: 50_000_000,
                    ..MeshAdaptationOptions::default()
                },
            );
            let adapted = loop {
                if let Some(result) = adaptation.advance(4096) {
                    break result.unwrap_or_else(|error| {
                        panic!("adaptation refused the reused mesh: {error}")
                    });
                }
            };
            assert!(adapted.mesh.triangles.len() > reused.mesh.triangles.len());
        }
    }

    fn nudge(editor: &mut TopologyEditor, curve: CurveId) {
        editor
            .set_control(curve, 0, Point2::new(0.24, 0.0))
            .unwrap();
        settle(editor);
    }

    /// Moving a control is a repair: the band around the hole is carved and
    /// refilled, the rest of the mesh and its nodes survive, the operator is
    /// rebuilt on the new mesh and the field crosses through a transfer that
    /// copies the untouched nodes exactly.
    #[test]
    fn a_coordinate_edit_repairs_the_mesh_by_carving() {
        let (mut editor, curve, mut runtime) = hole_runtime();
        let before = runtime.active().unwrap().clone();
        nudge(&mut editor, curve);
        let token = runtime
            .request(
                editor.revision,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                false,
            )
            .unwrap();
        assert_eq!(
            runtime.preparing.as_ref().unwrap().mesh_action,
            TopologyMeshUpdateAction::Repair(TopologyRepairReason::CurveOrJunctionMoved)
        );
        assert_eq!(runtime.phase(), Some(TopologyPreparationPhase::Repairing));
        assert_eq!(prepare(&mut runtime).unwrap(), token);
        let repaired = runtime.commit_ready(token).unwrap();
        assert_eq!(
            repaired.mesh_action,
            TopologyMeshUpdateAction::Repair(TopologyRepairReason::CurveOrJunctionMoved)
        );
        let report = repaired.carve.expect("a repair reports its carve");
        assert!(report.kept_triangles > 0, "{report:?}");
        assert!(
            report.removed_triangles * 2 < before.mesh.triangles.len(),
            "{report:?}"
        );
        assert_eq!(
            repaired.mesh.triangles.len(),
            report.kept_triangles + report.inserted_triangles
        );
        assert!(repaired.repair_fallback.is_none());
        assert!(!repaired.operator_reused);
        assert!(!repaired.adapted);
        assert_ne!(
            repaired.bundle.token.mesh_generation,
            before.bundle.token.mesh_generation
        );
        let transfer = repaired.transfer.as_ref().expect("the field crosses over");
        assert!(transfer.exact_nodes() * 2 > repaired.operator.degrees_of_freedom());
        assert!(repaired.timing.meshing_ms >= 0.0);
    }

    /// A carve that cannot finish is not the request's failure: the candidate
    /// falls back to the full rebuild and says why.
    #[test]
    fn a_failed_repair_falls_back_to_a_full_rebuild() {
        let (mut editor, curve, mut runtime) = hole_runtime();
        {
            // A mesh the carve cannot read: one boundary edge with a legacy label.
            let active = Arc::make_mut(runtime.active.as_mut().unwrap());
            let mesh = Arc::make_mut(&mut active.mesh);
            mesh.boundary_edges[0].label = BoundaryLabel::Obstacle(ObstacleId(7));
        }
        nudge(&mut editor, curve);
        let token = runtime
            .request(
                editor.revision,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                false,
            )
            .unwrap();
        assert_eq!(prepare(&mut runtime).unwrap(), token);
        let rebuilt = runtime.commit_ready(token).unwrap();
        assert_eq!(
            rebuilt.mesh_action,
            TopologyMeshUpdateAction::FullRebuild(TopologyFullRebuildReason::RepairFailed)
        );
        assert!(rebuilt.carve.is_none());
        assert!(
            rebuilt
                .repair_fallback
                .as_deref()
                .is_some_and(|message| message.contains("topology plan")),
            "{:?}",
            rebuilt.repair_fallback
        );
        assert!(rebuilt.transfer.is_some());
        assert_eq!(rebuilt.mesh.mesh_revision, token.mesh_generation);
    }

    /// Refinement an adaptation added survives an edit elsewhere: only the
    /// band around the moved curve is refilled, at the density it had.
    #[test]
    fn a_repair_keeps_an_adapted_mesh_outside_the_band() {
        let (mut editor, curve, mut runtime) = hole_runtime();
        let base = runtime.active().unwrap().clone();
        let fine = options().target_edge_length * 0.4;
        let mut adaptation = MeshAdaptationJob::new_topology(
            base.mesh.clone(),
            &base.bundle.plan,
            MeshAdaptationState::from_mesh(&base.mesh),
            runtime.reserve_mesh_revision(),
            Arc::new(move |point: Point2, _| {
                if point.x > 0.3 {
                    fine
                } else {
                    options().target_edge_length
                }
            }),
            MeshAdaptationOptions {
                meshing: options(),
                minimum_target_edge_length: fine,
                maximum_target_edge_length: options().target_edge_length,
                max_topology_changes: 8_000,
                max_work_units: 50_000_000,
                ..MeshAdaptationOptions::default()
            },
        );
        let adapted = loop {
            if let Some(result) = adaptation.advance(4096) {
                break result.unwrap().mesh;
            }
        };
        assert!(adapted.triangles.len() > base.mesh.triangles.len() * 3 / 2);
        let token = runtime
            .request_adapted(editor.revision, &editor.document, adapted.clone())
            .unwrap();
        assert_eq!(prepare(&mut runtime).unwrap(), token);
        runtime.commit_ready(token).unwrap();

        nudge(&mut editor, curve);
        let token = runtime
            .request(
                editor.revision,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                false,
            )
            .unwrap();
        assert_eq!(prepare(&mut runtime).unwrap(), token);
        let repaired = runtime.commit_ready(token).unwrap();
        let report = repaired.carve.expect("a repair reports its carve");
        assert!(
            report.kept_triangles * 2 > adapted.triangles.len(),
            "{report:?}"
        );
        // The refined half is far from the hole and comes through untouched.
        let refined_far = |mesh: &TriMesh| {
            mesh.triangles
                .iter()
                .filter(|triangle| {
                    triangle
                        .vertices
                        .iter()
                        .all(|vertex| mesh.vertices[*vertex].point.x > 0.6)
                })
                .count()
        };
        assert_eq!(refined_far(&repaired.mesh), refined_far(&adapted));
        assert!(repaired.mesh.triangles.len() > base.mesh.triangles.len() * 3 / 2);
    }

    /// The runtime hands the adaptation switch to the carve. On, the refill
    /// follows the sizes adaptation requested for the removed band; off, it
    /// returns to the meshing target and carries no request.
    #[test]
    fn a_repair_follows_the_adaptation_switch() {
        let (mut editor, curve, mut runtime) = hole_runtime();
        let base = runtime.active().unwrap().clone();
        let fine = options().target_edge_length * 0.4;
        let mut adaptation = MeshAdaptationJob::new_topology(
            base.mesh.clone(),
            &base.bundle.plan,
            MeshAdaptationState::from_mesh(&base.mesh),
            runtime.reserve_mesh_revision(),
            Arc::new(move |point: Point2, _| {
                if point.x > 0.0 {
                    fine
                } else {
                    options().target_edge_length
                }
            }),
            MeshAdaptationOptions {
                meshing: options(),
                minimum_target_edge_length: fine,
                maximum_target_edge_length: options().target_edge_length,
                max_topology_changes: 8_000,
                max_work_units: 50_000_000,
                ..MeshAdaptationOptions::default()
            },
        );
        let adapted = loop {
            if let Some(result) = adaptation.advance(4096) {
                break result.unwrap().mesh;
            }
        };
        let token = runtime
            .request_adapted(editor.revision, &editor.document, adapted)
            .unwrap();
        assert_eq!(prepare(&mut runtime).unwrap(), token);
        runtime.commit_ready(token).unwrap();

        let repair = |editor: &mut TopologyEditor, runtime: &mut TopologyRuntime, x: f64| {
            editor.set_control(curve, 0, Point2::new(x, 0.0)).unwrap();
            settle(editor);
            let token = runtime
                .request(
                    editor.revision,
                    &editor.document,
                    editor.compiled_accepted.clone(),
                    options(),
                    false,
                )
                .unwrap();
            assert_eq!(prepare(runtime).unwrap(), token);
            let repaired = runtime.commit_ready(token).unwrap();
            let carve = repaired.carve.expect("a repair reports its carve");
            assert_eq!(
                repaired.mesh.requested_sizes.len(),
                repaired.mesh.triangles.len()
            );
            let band = repaired.mesh.requested_sizes[carve.kept_triangles..].to_vec();
            assert!(!band.is_empty());
            band
        };

        let band = repair(&mut editor, &mut runtime, 0.24);
        let fine_requests = band
            .iter()
            .flatten()
            .filter(|size| (**size - fine).abs() < 1.0e-9)
            .count();
        assert!(
            fine_requests * 2 > band.len(),
            "{fine_requests} of {}",
            band.len()
        );

        runtime.set_preserve_adaptation(false);
        let band = repair(&mut editor, &mut runtime, 0.22);
        assert!(band.iter().all(Option::is_none));
    }

    /// A topology change is a repair too: a new baffle carves the band it
    /// passes through and the field crosses over with most nodes copied
    /// exactly.
    #[test]
    fn adding_a_baffle_repairs_the_active_mesh() {
        let (mut editor, _, mut runtime) = hole_runtime();
        let before = runtime.active().unwrap().clone();
        editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![Point2::new(0.5, -0.5), Point2::new(0.6, 0.5)])
                    .unwrap(),
                OpenCurvePurpose::BoundaryBaffle,
                None,
                None,
            )
            .unwrap();
        settle(&mut editor);
        let token = runtime
            .request(
                editor.revision,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                false,
            )
            .unwrap();
        assert_eq!(
            runtime.preparing.as_ref().unwrap().mesh_action,
            TopologyMeshUpdateAction::Repair(TopologyRepairReason::CurveOrSpanTopologyChanged)
        );
        assert_eq!(prepare(&mut runtime).unwrap(), token);
        let repaired = runtime.commit_ready(token).unwrap();
        let report = repaired.carve.expect("a repair reports its carve");
        assert!(
            report.kept_triangles * 2 > before.mesh.triangles.len(),
            "{report:?}"
        );
        assert_eq!(report.rebuilt_curves, 1, "{report:?}");
        let transfer = repaired.transfer.as_ref().expect("the field crosses over");
        assert!(transfer.exact_nodes() * 2 > repaired.operator.degrees_of_freedom());
    }

    /// A separator with free ends divides nothing, so it is a transmitting
    /// chain dangling inside one face. Topologically that is a baffle without
    /// the two sides being separated, and it carves like one: the face's
    /// boundary walks out along the chain and back, and the plan carries an
    /// atom for each direction so the cavity rim can follow it round the tip.
    #[test]
    fn adding_a_free_separator_repairs_the_active_mesh() {
        let (mut editor, _, mut runtime) = hole_runtime();
        let before = runtime.active().unwrap().clone();
        let separator = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![Point2::new(0.45, -0.55), Point2::new(0.62, 0.48)])
                    .unwrap(),
                OpenCurvePurpose::SubdomainSeparator {
                    material: DEFAULT_MATERIAL,
                },
                None,
                None,
            )
            .unwrap()
            .curve;
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert!(
            editor
                .document
                .model
                .draft
                .geometry
                .curve(separator)
                .unwrap()
                .nodes
                .iter()
                .all(|node| node.vertex.is_none()),
            "both ends stay free"
        );
        assert_eq!(
            editor.document.model.draft.regions.len(),
            before.bundle.authored.regions.len(),
            "dividing nothing creates no region"
        );

        let token = runtime
            .request(
                editor.revision,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                false,
            )
            .unwrap();
        assert_eq!(
            runtime.preparing.as_ref().unwrap().mesh_action,
            TopologyMeshUpdateAction::Repair(TopologyRepairReason::CurveOrSpanTopologyChanged)
        );
        assert_eq!(prepare(&mut runtime).unwrap(), token);
        let repaired = runtime.commit_ready(token).unwrap();
        let report = repaired.carve.unwrap_or_else(|| {
            panic!(
                "the carve should follow the chain: {:?}",
                repaired.repair_fallback
            )
        });
        assert!(
            report.kept_triangles * 2 > before.mesh.triangles.len(),
            "{report:?}"
        );
        let transfer = repaired.transfer.as_ref().expect("the field crosses over");
        assert!(transfer.exact_nodes() * 2 > repaired.operator.degrees_of_freedom());
        // The chain is in the mesh, and both of its sides name the one face it
        // lies in, so nothing along it bounds anything.
        assert!(
            repaired.mesh.boundary_edges.iter().any(|edge| matches!(
                edge.label,
                BoundaryLabel::Curve {
                    curve: candidate,
                    separated: false,
                    ..
                } if candidate == separator
            )),
            "the separator is traced into the mesh"
        );
    }

    /// Moving one has to repair too, not only adding it. A transmitting chain
    /// the face walks out along and back encloses nothing, so the cavity cycle
    /// that follows it has no area and no orientation to sort it by; it is a
    /// slit, and it is pulled out of the cavity polygon and recovered into the
    /// triangulation exactly as a baffle is.
    #[test]
    fn moving_a_free_separator_repairs_like_a_baffle() {
        let carve_every_move = |purpose: OpenCurvePurpose| {
            let mut editor = TopologyEditor::default();
            let curve = editor
                .create_open_curve(
                    OpenCubicSpline::polyline(vec![
                        Point2::new(0.45, -0.55),
                        Point2::new(0.62, 0.48),
                    ])
                    .unwrap(),
                    purpose,
                    None,
                    None,
                )
                .unwrap()
                .curve;
            settle(&mut editor);
            let mut runtime = TopologyRuntime::default();
            let token = runtime
                .request(
                    editor.revision,
                    &editor.document,
                    editor.compiled_accepted.clone(),
                    options(),
                    true,
                )
                .unwrap();
            prepare(&mut runtime).unwrap();
            runtime.commit_ready(token).unwrap();

            for step in 1..=6 {
                let shift = 0.02 * step as f64;
                editor
                    .set_control(curve, 0, Point2::new(0.45 - shift, -0.55 + shift))
                    .unwrap();
                settle(&mut editor);
                let token = runtime
                    .request(
                        editor.revision,
                        &editor.document,
                        editor.compiled_accepted.clone(),
                        options(),
                        false,
                    )
                    .unwrap();
                assert_eq!(prepare(&mut runtime).unwrap(), token);
                let moved = runtime.commit_ready(token).unwrap();
                assert!(
                    moved.carve.is_some(),
                    "{purpose:?} move {step} fell back: {:?}",
                    moved.repair_fallback
                );
            }
        };
        carve_every_move(OpenCurvePurpose::BoundaryBaffle);
        carve_every_move(OpenCurvePurpose::SubdomainSeparator {
            material: DEFAULT_MATERIAL,
        });
    }

    /// A baffle welded to the wall at one end moves by carving, with a second
    /// welded baffle in the same face or without one: its foot is split into
    /// sectors again rather than leaving the cavity walk a dead end, which
    /// used to send every such move to a full rebuild.
    #[test]
    fn moving_a_welded_baffle_repairs_by_carving() {
        for count in [1, 2] {
            let mut editor = TopologyEditor::default();
            let mut curves = vec![];
            for (index, (from, to, side)) in [
                (0.9, 0.25, OuterSide::Top),
                (-0.9, -0.25, OuterSide::Bottom),
            ]
            .into_iter()
            .take(count)
            .enumerate()
            {
                let curve = editor
                    .create_open_curve(
                        OpenCubicSpline::polyline(vec![
                            Point2::new(0.0, from),
                            Point2::new(0.0, to),
                        ])
                        .unwrap(),
                        OpenCurvePurpose::BoundaryBaffle,
                        None,
                        None,
                    )
                    .unwrap()
                    .curve;
                settle(&mut editor);
                editor
                    .attach_endpoint(
                        curve,
                        0,
                        TopologyAttachment::Boundary(FaceAnchor::Outer {
                            side,
                            fraction: 0.5,
                        }),
                    )
                    .unwrap();
                settle(&mut editor);
                curves.push((index, curve));
            }
            let mut runtime = TopologyRuntime::default();
            let token = runtime
                .request(
                    editor.revision,
                    &editor.document,
                    editor.compiled_accepted.clone(),
                    options(),
                    true,
                )
                .unwrap();
            prepare(&mut runtime).unwrap();
            runtime.commit_ready(token).unwrap();
            let (_, curve) = curves[0];
            for step in 1..=6 {
                editor
                    .set_control(curve, 3, Point2::new(0.03 * step as f64, 0.25))
                    .unwrap();
                settle(&mut editor);
                let token = runtime
                    .request(
                        editor.revision,
                        &editor.document,
                        editor.compiled_accepted.clone(),
                        options(),
                        false,
                    )
                    .unwrap();
                assert_eq!(prepare(&mut runtime).unwrap(), token);
                let moved = runtime.commit_ready(token).unwrap();
                assert!(
                    moved.carve.is_some(),
                    "{count} welded, move {step} fell back: {:?}",
                    moved.repair_fallback
                );
            }
        }
    }

    /// The moving welded baffles' first and last meshes, with one baffle and
    /// with two.
    const PINNED_DIGESTS: [u64; 4] = [
        0x5131_04f0_bb40_dd41,
        0xaf1d_5f7e_65d6_4fe2,
        0xc9c7_76eb_cd0e_b1ef,
        0x8f78_cc60_a87e_9280,
    ];

    /// Welds one or two baffles to the walls, meshes, then moves the first
    /// baffle's free end `spacing` further right per step for `steps` steps,
    /// handing each prepared generation to `visit` with its step (0 for the
    /// first mesh).
    fn move_welded_baffles(
        count: usize,
        spacing: f64,
        steps: usize,
        mut visit: impl FnMut(usize, &PreparedTopology),
    ) {
        let mut editor = TopologyEditor::default();
        let mut curves = vec![];
        for (from, to, side) in [
            (0.9, 0.25, OuterSide::Top),
            (-0.9, -0.25, OuterSide::Bottom),
        ]
        .into_iter()
        .take(count)
        {
            let curve = editor
                .create_open_curve(
                    OpenCubicSpline::polyline(vec![Point2::new(0.0, from), Point2::new(0.0, to)])
                        .unwrap(),
                    OpenCurvePurpose::BoundaryBaffle,
                    None,
                    None,
                )
                .unwrap()
                .curve;
            settle(&mut editor);
            editor
                .attach_endpoint(
                    curve,
                    0,
                    TopologyAttachment::Boundary(FaceAnchor::Outer {
                        side,
                        fraction: 0.5,
                    }),
                )
                .unwrap();
            settle(&mut editor);
            curves.push(curve);
        }
        let mut runtime = TopologyRuntime::default();
        let token = runtime
            .request(
                editor.revision,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                true,
            )
            .unwrap();
        prepare(&mut runtime).unwrap();
        visit(0, &runtime.commit_ready(token).unwrap());
        for step in 1..=steps {
            editor
                .set_control(curves[0], 3, Point2::new(spacing * step as f64, 0.25))
                .unwrap();
            settle(&mut editor);
            let token = runtime
                .request(
                    editor.revision,
                    &editor.document,
                    editor.compiled_accepted.clone(),
                    options(),
                    false,
                )
                .unwrap();
            prepare(&mut runtime).unwrap();
            visit(step, &runtime.commit_ready(token).unwrap());
        }
    }

    /// FNV-1a over a mesh's vertex bits and triangles.
    fn mesh_digest(mesh: &TriMesh) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |value: u64| {
            for byte in value.to_le_bytes() {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
        };
        for vertex in &mesh.vertices {
            mix(vertex.point.x.to_bits());
            mix(vertex.point.y.to_bits());
        }
        for triangle in &mesh.triangles {
            for index in triangle.vertices {
                mix(index as u64);
            }
        }
        hash
    }

    /// Meshing is the same, bit for bit, on every platform: the moving
    /// welded baffles' first and last meshes hash to pinned values. Their
    /// failing twin on Linux CI meshed the same geometry differently from
    /// the first mesh on, because the refinement ranked triangles by the
    /// platform's `acos`, a last bit apart; the mesher now forms every
    /// decision from the four operations and `sqrt`. When the mesher changes
    /// on purpose these pins move with it; a mismatch on one platform alone
    /// means a platform-dependent operation has crept back in.
    #[test]
    fn welded_baffles_mesh_the_same_on_every_platform() {
        let mut digests = vec![];
        for count in [1, 2] {
            move_welded_baffles(count, 0.03, 6, |step, prepared| {
                if step == 0 || step == 6 {
                    digests.push(mesh_digest(&prepared.mesh));
                }
            });
        }
        assert_eq!(digests, PINNED_DIGESTS, "{digests:016x?}");
    }

    /// A refill refused for an element under the quality floor is carved
    /// again with the cavity a ring wider, as many times as it takes up to
    /// the limit, rather than falling back to a full rebuild. Moving a
    /// welded baffle 0.025 and 0.038 a step, with one baffle and two, puts
    /// first refills under the floor that one to five extra rings carve.
    #[test]
    fn a_refill_under_the_floor_carves_again_with_a_wider_cavity() {
        let mut retried = 0;
        for spacing in [0.025, 0.038] {
            for count in [1, 2] {
                move_welded_baffles(count, spacing, 8, |step, prepared| {
                    if step == 0 {
                        return;
                    }
                    let carve = prepared.carve.unwrap_or_else(|| {
                        panic!(
                            "{count} welded at {spacing}, move {step} fell back: {:?}",
                            prepared.repair_fallback
                        )
                    });
                    retried += usize::from(carve.quality_retries > 0);
                });
            }
        }
        assert!(retried >= 3, "only {retried} carves needed a wider cavity");
    }

    /// Adaptation checks every constrained edge against the plan it came from,
    /// so a chain the carve recovered has to carry the same lineage a rebuild
    /// would have given it: its interval endpoints hold their traces, and the
    /// vertices between them the label and parameter they sit at.
    #[test]
    fn an_adaptation_follows_a_free_separator_the_carve_recovered() {
        let mut editor = TopologyEditor::default();
        let separator = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    Point2::new(-0.41, 0.13),
                    Point2::new(0.02, -0.07),
                    Point2::new(0.37, -0.19),
                ])
                .unwrap(),
                OpenCurvePurpose::SubdomainSeparator {
                    material: DEFAULT_MATERIAL,
                },
                None,
                None,
            )
            .unwrap()
            .curve;
        settle(&mut editor);
        let mut runtime = TopologyRuntime::default();
        let token = runtime
            .request(
                editor.revision,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                true,
            )
            .unwrap();
        prepare(&mut runtime).unwrap();
        runtime.commit_ready(token).unwrap();

        editor
            .set_control(separator, 0, Point2::new(-0.45, 0.17))
            .unwrap();
        settle(&mut editor);
        let token = runtime
            .request(
                editor.revision,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                false,
            )
            .unwrap();
        assert_eq!(prepare(&mut runtime).unwrap(), token);
        let carved = runtime.commit_ready(token).unwrap();
        assert!(
            carved.carve.is_some(),
            "the move repaired: {:?}",
            carved.repair_fallback
        );

        let fine = options().target_edge_length * 0.4;
        let mut adaptation = MeshAdaptationJob::new_topology(
            carved.mesh.clone(),
            &carved.bundle.plan,
            MeshAdaptationState::from_mesh(&carved.mesh),
            runtime.reserve_mesh_revision(),
            Arc::new(move |point: Point2, _| {
                if point.x > 0.0 {
                    fine
                } else {
                    options().target_edge_length
                }
            }),
            MeshAdaptationOptions {
                meshing: options(),
                minimum_target_edge_length: fine,
                maximum_target_edge_length: options().target_edge_length,
                max_topology_changes: 8_000,
                max_work_units: 50_000_000,
                ..MeshAdaptationOptions::default()
            },
        );
        let adapted = loop {
            if let Some(result) = adaptation.advance(4096) {
                break result
                    .unwrap_or_else(|error| panic!("adaptation refused the carve: {error}"));
            }
        };
        assert!(adapted.mesh.triangles.len() > carved.mesh.triangles.len());
        assert!(
            adapted.mesh.boundary_edges.iter().any(|edge| matches!(
                edge.label,
                BoundaryLabel::Curve {
                    curve: candidate,
                    separated: false,
                    ..
                } if candidate == separator
            )),
            "the separator survives the adaptation"
        );
    }

    /// Carrying a probe is the point of a dangling separator, and a probe reads
    /// one side of a boundary. Both sides of this one name the same face, so
    /// both have to compile - they sample the same field with opposite normals,
    /// which is what makes the flux sign meaningful.
    #[test]
    fn a_probe_reads_both_sides_of_a_free_separator() {
        let mut editor = TopologyEditor::default();
        let separator = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    Point2::new(-0.41, 0.13),
                    Point2::new(0.02, -0.07),
                    Point2::new(0.37, -0.19),
                ])
                .unwrap(),
                OpenCurvePurpose::SubdomainSeparator {
                    material: DEFAULT_MATERIAL,
                },
                None,
                None,
            )
            .unwrap()
            .curve;
        settle(&mut editor);
        let spans = editor
            .document
            .model
            .draft
            .geometry
            .curve(separator)
            .unwrap()
            .spans
            .iter()
            .map(|span| span.id)
            .collect::<Vec<_>>();
        let probes = [CurveTraceSide::Left, CurveTraceSide::Right].map(|side| {
            editor
                .create_probe(
                    format!("{side:?}"),
                    [200, 160, 90],
                    TopologyProbeTarget::Boundary(TopologyBoundaryProbeTarget {
                        curve: separator,
                        spans: spans.clone(),
                        side,
                        reversed: false,
                        preset: crate::document::ProbeSamplingPreset::default(),
                    }),
                )
                .unwrap()
        });
        settle(&mut editor);

        let mut runtime = TopologyRuntime::default();
        let token = runtime
            .request(
                editor.revision,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                true,
            )
            .unwrap();
        assert_eq!(prepare(&mut runtime).unwrap(), token);
        let active = runtime.commit_ready(token).unwrap();

        let normals = probes.map(|id| {
            let compiled = active
                .probes
                .iter()
                .find(|compiled| compiled.id == id)
                .unwrap_or_else(|| panic!("probe {id:?} was compiled"));
            match &compiled.result {
                TopologyProbeCompilation::Ready(stencil) => match stencil.as_ref() {
                    TopologyProbeStencil::Boundary(points) => {
                        assert!(!points.is_empty(), "{id:?} sampled nothing");
                        points[0].outward_normal
                    }
                    other => panic!("{id:?} compiled to {other:?}"),
                },
                other => panic!("{id:?} did not compile: {other:?}"),
            }
        });
        let [left, right] = normals;
        assert!(
            (left.x + right.x).abs() < 1.0e-9 && (left.y + right.y).abs() < 1.0e-9,
            "the two sides face opposite ways: {left:?} {right:?}"
        );
    }

    /// The scene draws a boundary probe's band and flux arrow from the sampled
    /// polyline and the trace side alone, so that pair has to agree with the
    /// normal the stencil derives from the mesh. `Left` names the face on the
    /// left of increasing parameter - which is the side the band is drawn - and
    /// the normal leaving it points right of the drawn path.
    #[test]
    fn a_left_traces_normal_points_right_of_the_drawn_path() {
        let mut editor = TopologyEditor::default();
        let loop_id = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(0.0, 0.0), 0.4),
                ClosedCurvePurpose::Subdomain {
                    material: DEFAULT_MATERIAL,
                },
            )
            .unwrap();
        settle(&mut editor);
        let spans = editor
            .document
            .model
            .draft
            .geometry
            .curve(loop_id)
            .unwrap()
            .spans
            .iter()
            .map(|span| span.id)
            .collect::<Vec<_>>();
        let probe = editor
            .create_probe(
                "Left".into(),
                [200, 160, 90],
                TopologyProbeTarget::Boundary(TopologyBoundaryProbeTarget {
                    curve: loop_id,
                    spans: spans.clone(),
                    side: CurveTraceSide::Left,
                    reversed: false,
                    preset: crate::document::ProbeSamplingPreset::default(),
                }),
            )
            .unwrap();
        settle(&mut editor);
        let compiled = editor.compiled_accepted.clone();
        let mut runtime = TopologyRuntime::default();
        let token = runtime
            .request(
                editor.revision,
                &editor.document,
                compiled.clone(),
                options(),
                true,
            )
            .unwrap();
        assert_eq!(prepare(&mut runtime).unwrap(), token);
        let active = runtime.commit_ready(token).unwrap();

        // The plan puts a left trace on the face that lies to the left of the
        // curve's own direction, so an offset band names the face it covers.
        let mut checked = 0;
        for boundary in &active.bundle.plan.boundaries {
            let PlannedBoundarySource::Curve {
                curve,
                side: CurveTraceSide::Left,
                ..
            } = boundary.source
            else {
                continue;
            };
            if curve != loop_id {
                continue;
            }
            let tangent = boundary.points[1] - boundary.points[0];
            let tangent = tangent / tangent.norm();
            let left = Point2::new(-tangent.y, tangent.x);
            let middle = (boundary.points[0] + boundary.points[1]) * 0.5;
            assert_eq!(
                compiled.topology.face_at(middle + left * 0.02),
                Some(boundary.face),
                "a left trace at {middle:?} covers the face on its left"
            );
            checked += 1;
        }
        assert!(checked > 0, "the loop planned left traces");

        // The polyline the scene draws, in the order `boundary_probe_polyline`
        // assembles it.
        let sampled = SampledTopologyGeometry::new(
            &editor.document.model.draft.geometry,
            ViewportTransform {
                screen_center: ScreenPoint::new(100.0, 100.0),
                world_center: Point2::default(),
                pixels_per_world: 100.0,
            },
            0.25,
        )
        .unwrap();
        let mut drawn = Vec::new();
        for span in &spans {
            let piece = sampled
                .spans
                .iter()
                .find(|candidate| candidate.target == TopologySpanTarget::Curve(*span))
                .expect("the span is drawn");
            for sample in &piece.samples {
                if drawn.last() != Some(&sample.point) {
                    drawn.push(sample.point);
                }
            }
        }
        let tangent_at = |point: Point2| {
            drawn
                .windows(2)
                .filter_map(|pair| {
                    let segment = pair[1] - pair[0];
                    let length = segment.norm();
                    if length <= 0.0 {
                        return None;
                    }
                    let fraction =
                        ((point - pair[0]).dot(segment) / (length * length)).clamp(0.0, 1.0);
                    let closest = pair[0].lerp(pair[1], fraction);
                    Some(((point - closest).norm(), segment / length))
                })
                .min_by(|left, right| left.0.total_cmp(&right.0))
                .map(|(_, tangent)| tangent)
                .expect("the path has a segment")
        };

        let compiled_probe = active
            .probes
            .iter()
            .find(|candidate| candidate.id == probe)
            .expect("the probe compiled");
        let TopologyProbeCompilation::Ready(stencil) = &compiled_probe.result else {
            panic!("the probe did not compile: {:?}", compiled_probe.result);
        };
        let TopologyProbeStencil::Boundary(points) = stencil.as_ref() else {
            panic!("the probe compiled to {stencil:?}");
        };
        assert!(!points.is_empty(), "the probe sampled the trace");
        for sample in points.iter() {
            let tangent = tangent_at(sample.point);
            let right = Point2::new(tangent.y, -tangent.x);
            assert!(
                sample.outward_normal.dot(right) > 0.9,
                "the left trace's normal {:?} at {:?} leaves the path running {tangent:?}",
                sample.outward_normal,
                sample.point
            );
        }
    }

    #[test]
    fn failed_or_superseded_candidate_never_replaces_active_state() {
        let editor = TopologyEditor::default();
        let mut runtime = TopologyRuntime::default();
        let first = runtime
            .request(
                1,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                true,
            )
            .unwrap();
        prepare(&mut runtime).unwrap();
        let active = runtime.commit_ready(first).unwrap();

        let mut bad = editor.document.clone();
        bad.model.source.enabled = true;
        bad.model.source.position = Point2::new(4.0, 4.0);
        runtime
            .request(
                2,
                &bad,
                bad.model.accepted.compile(2).unwrap(),
                options(),
                false,
            )
            .unwrap();
        assert!(prepare(&mut runtime).is_err());
        assert_eq!(runtime.active().unwrap().bundle.token, active.bundle.token);

        runtime
            .request(
                3,
                &editor.document,
                editor.document.model.accepted.compile(3).unwrap(),
                options(),
                false,
            )
            .unwrap();
        let newest = runtime
            .request(
                4,
                &editor.document,
                editor.document.model.accepted.compile(4).unwrap(),
                options(),
                false,
            )
            .unwrap();
        assert_eq!(prepare(&mut runtime).unwrap(), newest);
        assert!(
            runtime
                .commit_ready(TopologyToken {
                    document_revision: 3,
                    topology_revision: 3,
                    mesh_generation: 3,
                })
                .is_err()
        );
        assert_eq!(runtime.active().unwrap().bundle.token, active.bundle.token);
    }

    #[test]
    fn attached_separator_probes_and_far_field_use_the_same_token() {
        let mut editor = TopologyEditor::default();
        editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    Point2::new(0.0, -1.0),
                    Point2::new(0.0, 0.0),
                    Point2::new(0.0, 1.0),
                ])
                .unwrap(),
                OpenCurvePurpose::SubdomainSeparator {
                    material: DEFAULT_MATERIAL,
                },
                Some(TopologyAttachment::Boundary(FaceAnchor::Outer {
                    side: OuterSide::Bottom,
                    fraction: 0.5,
                })),
                Some(TopologyAttachment::Boundary(FaceAnchor::Outer {
                    side: OuterSide::Top,
                    fraction: 0.5,
                })),
            )
            .unwrap();
        settle(&mut editor);
        let curve = &editor.document.model.accepted.geometry.curves[0];
        editor.document.model.probes = vec![
            TopologyProbeDefinition {
                id: ProbeId(1),
                name: "point".into(),
                color: [91, 220, 194],
                enabled: true,
                target: TopologyProbeTarget::Point(Point2::new(-0.5, 0.0)),
            },
            TopologyProbeDefinition {
                id: ProbeId(2),
                name: "divider".into(),
                color: [248, 196, 112],
                enabled: true,
                target: TopologyProbeTarget::Boundary(TopologyBoundaryProbeTarget {
                    curve: curve.id,
                    spans: vec![curve.spans[0].id],
                    side: CurveTraceSide::Left,
                    reversed: false,
                    preset: crate::document::ProbeSamplingPreset::Low,
                }),
            },
        ];
        editor.document.model.far_field.enabled = true;
        editor.document.model.far_field.inset = 0.08;
        let mut runtime = TopologyRuntime::default();
        let token = runtime
            .request(
                editor.revision,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                true,
            )
            .unwrap();
        prepare(&mut runtime).unwrap();
        let candidate = runtime.ready().unwrap();
        assert_eq!(candidate.bundle.token, token);
        assert_eq!(candidate.probes.len(), 2);
        assert!(
            candidate
                .probes
                .iter()
                .all(|probe| matches!(probe.result, TopologyProbeCompilation::Ready(_)))
        );
        assert!(candidate.far_field.as_ref().unwrap().is_err());
    }
}
