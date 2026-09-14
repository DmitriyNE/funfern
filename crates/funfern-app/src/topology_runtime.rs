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
use funfern_core::*;
use std::sync::Arc;

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
        Ok(Self {
            token: TopologyToken {
                document_revision,
                topology_revision,
                mesh_generation,
            },
            authored: Arc::new(authored),
            snapshot: Arc::new(compiled.topology),
            plan: Arc::new(compiled.plan),
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

#[derive(Clone, Debug)]
pub struct PreparedTopology {
    pub bundle: Arc<AcceptedTopology>,
    pub mesh: Arc<TriMesh>,
    pub operator: Arc<QuadraticWaveOperator>,
    pub volume_sources: Arc<CompiledVolumeSources>,
    pub probes: Arc<[CompiledTopologyProbe]>,
    pub far_field: Option<Result<Arc<QuadraticFarFieldStencil>, String>>,
    pub point_source: PointSource,
    pub transfer: Option<Arc<QuadraticTransferMap>>,
    pub fresh: bool,
    pub mesh_action: TopologyMeshUpdateAction,
    pub operator_reused: bool,
    pub adapted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopologyPreparationPhase {
    Meshing,
    Assembling,
    CompilingSources,
    CompilingMeasurements,
    Ready,
    Failed,
}

impl TopologyPreparationPhase {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Meshing => "Mesh rebuilding",
            Self::Assembling => "Assembling wave operator",
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

pub struct TopologyPreparationJob {
    bundle: Arc<AcceptedTopology>,
    probes: Arc<[TopologyProbeDefinition]>,
    point_source: PointSource,
    far_field: FarFieldSettings,
    previous: Option<Arc<PreparedTopology>>,
    mesh_action: TopologyMeshUpdateAction,
    fresh: bool,
    phase: TopologyPreparationPhase,
    mesh_job: Option<TopologyMeshingJob>,
    mesh: Option<Arc<TriMesh>>,
    operator: Option<Arc<QuadraticWaveOperator>>,
    transfer: Option<Arc<QuadraticTransferMap>>,
    source_job: Option<VolumeSourceCompileJob>,
    volume_sources: Option<Arc<CompiledVolumeSources>>,
    operator_reused: bool,
    adapted: bool,
    done: bool,
}

impl TopologyPreparationJob {
    pub fn new(
        document_revision: u64,
        document: &TopologyDocument,
        compiled: CompiledTopologyScene,
        previous: Option<Arc<PreparedTopology>>,
        mesh_revision: u64,
        options: MeshingOptions,
        fresh: bool,
    ) -> Result<Self, String> {
        let same_authored_scene = previous
            .as_ref()
            .is_some_and(|previous| *previous.bundle.authored == document.model.accepted);
        let bundle = if same_authored_scene {
            let previous = previous.as_ref().unwrap();
            Arc::new(AcceptedTopology {
                token: TopologyToken {
                    document_revision,
                    topology_revision: previous.bundle.token.topology_revision,
                    mesh_generation: mesh_revision,
                },
                authored: previous.bundle.authored.clone(),
                snapshot: previous.bundle.snapshot.clone(),
                plan: previous.bundle.plan.clone(),
            })
        } else {
            Arc::new(AcceptedTopology::new(
                document_revision,
                mesh_revision,
                document.model.accepted.clone(),
                compiled,
            )?)
        };
        let mesh_action = previous.as_ref().map_or(
            TopologyMeshUpdateAction::FullRebuild(TopologyFullRebuildReason::DomainChanged),
            |previous| topology_mesh_update_action(&previous.bundle.plan, &bundle.plan),
        );
        let (mesh_job, mesh, phase) = match mesh_action {
            TopologyMeshUpdateAction::Reuse => {
                let mesh = if same_authored_scene {
                    previous.as_ref().unwrap().mesh.clone()
                } else {
                    let mut mesh = previous.as_ref().unwrap().mesh.as_ref().clone();
                    mesh.geometry_revision = bundle.plan.geometry_revision;
                    Arc::new(mesh)
                };
                (None, Some(mesh), TopologyPreparationPhase::Assembling)
            }
            TopologyMeshUpdateAction::FullRebuild(_) => (
                Some(TopologyMeshingJob::new(
                    bundle.plan.as_ref().clone(),
                    mesh_revision,
                    options,
                )),
                None,
                TopologyPreparationPhase::Meshing,
            ),
        };
        let operator_reused = same_authored_scene;
        let operator = operator_reused.then(|| previous.as_ref().unwrap().operator.clone());
        let volume_sources =
            operator_reused.then(|| previous.as_ref().unwrap().volume_sources.clone());
        Ok(Self {
            bundle,
            probes: document.model.probes.clone().into(),
            point_source: document.model.source,
            far_field: document.model.far_field,
            previous,
            mesh_action,
            fresh,
            phase,
            mesh_job,
            mesh,
            operator,
            transfer: None,
            source_job: None,
            volume_sources,
            operator_reused,
            adapted: false,
            done: false,
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
            previous: Some(previous),
            mesh_action: TopologyMeshUpdateAction::Reuse,
            fresh: false,
            phase: TopologyPreparationPhase::Assembling,
            mesh_job: None,
            mesh: Some(Arc::new(mesh)),
            operator: None,
            transfer: None,
            source_job: None,
            volume_sources: None,
            operator_reused: false,
            adapted: true,
            done: false,
        })
    }

    pub fn token(&self) -> TopologyToken {
        self.bundle.token
    }

    pub fn phase(&self) -> TopologyPreparationPhase {
        self.phase
    }

    pub fn detail(&self) -> &'static str {
        self.mesh_job
            .as_ref()
            .map_or_else(|| self.phase.label(), TopologyMeshingJob::phase)
    }

    pub fn advance(
        &mut self,
        budget: usize,
    ) -> Option<Result<PreparedTopology, TopologyPreparationError>> {
        if self.done || budget == 0 {
            return None;
        }
        if let Some(job) = &mut self.mesh_job {
            let result = job.advance(budget)?;
            self.mesh_job = None;
            match result {
                Ok(mesh) => {
                    self.mesh = Some(Arc::new(mesh));
                    self.phase = TopologyPreparationPhase::Assembling;
                }
                Err(error) => return Some(Err(self.fail(error.to_string()))),
            }
        }
        if self.operator.is_none() {
            let mesh = self.mesh.as_ref().unwrap().clone();
            let operator = match QuadraticWaveOperator::assemble_topology(
                &mesh,
                &self.bundle.plan,
                self.bundle.model(),
            ) {
                Ok(operator) => Arc::new(operator),
                Err(error) => return Some(Err(self.fail(error.to_string()))),
            };
            if !self.fresh
                && let Some(previous) = &self.previous
            {
                match QuadraticTransferMap::build(
                    &previous.mesh,
                    &previous.operator,
                    &mesh,
                    &operator,
                ) {
                    Ok(transfer) => self.transfer = Some(Arc::new(transfer)),
                    Err(error) => return Some(Err(self.fail(error.to_string()))),
                }
            }
            match self.validate_point_source(&mesh, &operator) {
                Ok(()) => {}
                Err(error) => return Some(Err(self.fail(error))),
            }
            self.operator = Some(operator.clone());
            self.phase = TopologyPreparationPhase::CompilingSources;
            match VolumeSourceCompileJob::new_topology(
                mesh,
                operator,
                &self.bundle.plan,
                self.bundle.model(),
                &self.bundle.authored.volume_sources,
            ) {
                Ok(job) => self.source_job = Some(job),
                Err(error) => return Some(Err(self.fail(error.to_string()))),
            }
        } else if let Err(error) =
            self.validate_point_source(self.mesh.as_ref().unwrap(), self.operator.as_ref().unwrap())
        {
            return Some(Err(self.fail(error)));
        }
        if let Some(job) = &mut self.source_job {
            let result = job.advance(budget)?;
            self.source_job = None;
            match result {
                Ok(sources) => self.volume_sources = Some(Arc::new(sources)),
                Err(error) => return Some(Err(self.fail(error.to_string()))),
            }
        }
        self.phase = TopologyPreparationPhase::CompilingMeasurements;
        let mesh = self.mesh.as_ref().unwrap().clone();
        let operator = self.operator.as_ref().unwrap().clone();
        let probes = compile_probes(&self.probes, &mesh, &operator, &self.bundle);
        let far_field = self.far_field.enabled.then(|| {
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
        });
        self.done = true;
        self.phase = TopologyPreparationPhase::Ready;
        Some(Ok(PreparedTopology {
            bundle: self.bundle.clone(),
            mesh,
            operator,
            volume_sources: self.volume_sources.take().unwrap(),
            probes: probes.into(),
            far_field,
            point_source: self.point_source,
            transfer: self.transfer.take(),
            fresh: self.fresh,
            mesh_action: self.mesh_action,
            operator_reused: self.operator_reused,
            adapted: self.adapted,
        }))
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
            self.bundle.model(),
            self.point_source.position,
        )
        .map_err(|error| format!("Point source placement is invalid: {error}"))?;
        if stencil.region != self.point_source.region {
            return Err("Point source position does not lie in its assigned region".into());
        }
        Ok(())
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

pub struct TopologyRuntime {
    active: Option<Arc<PreparedTopology>>,
    preparing: Option<TopologyPreparationJob>,
    ready: Option<PreparedTopology>,
    requested: Option<TopologyToken>,
    next_mesh_revision: u64,
    last_error: Option<TopologyPreparationError>,
}

impl Default for TopologyRuntime {
    fn default() -> Self {
        Self {
            active: None,
            preparing: None,
            ready: None,
            requested: None,
            next_mesh_revision: 1,
            last_error: None,
        }
    }
}

impl TopologyRuntime {
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
        let mesh_revision = self.reserve_mesh_revision();
        let job = TopologyPreparationJob::new(
            document_revision,
            document,
            compiled,
            self.active.clone(),
            mesh_revision,
            options,
            fresh,
        )?;
        let token = job.token();
        self.preparing = Some(job);
        self.ready = None;
        self.requested = Some(token);
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
        self.last_error = None;
        Ok(token)
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

    pub fn active(&self) -> Option<&Arc<PreparedTopology>> {
        self.active.as_ref()
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

fn compile_probes(
    probes: &[TopologyProbeDefinition],
    mesh: &TriMesh,
    operator: &QuadraticWaveOperator,
    bundle: &AcceptedTopology,
) -> Vec<CompiledTopologyProbe> {
    probes
        .iter()
        .map(|probe| {
            let result = if !probe.enabled {
                TopologyProbeCompilation::Disabled
            } else {
                compile_probe(probe, mesh, operator, bundle)
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

fn compile_probe(
    probe: &TopologyProbeDefinition,
    mesh: &TriMesh,
    operator: &QuadraticWaveOperator,
    bundle: &AcceptedTopology,
) -> Result<TopologyProbeStencil, String> {
    let model = bundle.model();
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
    fn accepted_bundle_rejects_a_compiled_snapshot_from_another_scene() {
        let authored = TopologyScene::default();
        let mut other = TopologyScene::default();
        other.geometry.domain.max_x = 1.5;
        let compiled = other.compile(4).unwrap();
        assert!(AcceptedTopology::new(7, 1, authored, compiled).is_err());
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

        let mut document = editor.document.clone();
        document.model.accepted.materials[0].damping = ScalarField::constant(0.02);
        document.model.draft = document.model.accepted.clone();
        let compiled = document.model.accepted.compile(7).unwrap();
        let second = runtime
            .request(2, &document, compiled, options(), false)
            .unwrap();
        prepare(&mut runtime).unwrap();
        let candidate = runtime.ready().unwrap();
        assert_eq!(candidate.mesh_action, TopologyMeshUpdateAction::Reuse);
        assert_eq!(candidate.mesh.mesh_revision, original.mesh.mesh_revision);
        assert!(!candidate.operator_reused);
        assert!(candidate.transfer.is_some());
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
        let adapted = runtime
            .request_adapted(editor.revision, &editor.document, adapted_mesh)
            .unwrap();
        assert_eq!(adapted.document_revision, initial.document_revision);
        assert_eq!(adapted.topology_revision, initial.topology_revision);
        assert_ne!(adapted.mesh_generation, initial.mesh_generation);
        assert_eq!(prepare(&mut runtime).unwrap(), adapted);
        assert_eq!(runtime.active().unwrap().bundle.token, initial);
        assert!(runtime.ready().unwrap().adapted);
        assert_eq!(runtime.commit_ready(adapted).unwrap().bundle.token, adapted);
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
    }

    #[test]
    fn coordinate_edit_is_a_typed_full_rebuild() {
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

        editor
            .set_control(curve, 0, Point2::new(0.24, 0.0))
            .unwrap();
        settle(&mut editor);
        runtime
            .request(
                editor.revision,
                &editor.document,
                editor.compiled_accepted.clone(),
                options(),
                false,
            )
            .unwrap();
        assert!(matches!(
            runtime.preparing.as_ref().unwrap().mesh_action,
            TopologyMeshUpdateAction::FullRebuild(
                TopologyFullRebuildReason::CoordinateRepairDeferred
            )
        ));
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
