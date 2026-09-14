//! Headless document and command layer for the unified topology application.
//!
//! The visible editor consumes this layer through stable topology viewport
//! commands; no legacy object-role adapter belongs here.

use crate::document::{
    FarFieldSettings, MAX_PROBES, PresentationSettings, ProbeId, ProbeSamplingPreset,
};
use crate::topology_viewport::TopologyTransformUpdate;
use funfern_core::*;
use std::collections::{BTreeMap, BTreeSet};

const HISTORY_LIMIT: usize = 100;

#[derive(Clone, Debug, PartialEq)]
pub struct TopologyBoundaryProbeTarget {
    pub curve: CurveId,
    pub spans: Vec<CurveSpanId>,
    pub side: CurveTraceSide,
    pub reversed: bool,
    pub preset: ProbeSamplingPreset,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TopologyProbeTarget {
    Point(Point2),
    Segment {
        start: Point2,
        end: Point2,
        preset: ProbeSamplingPreset,
    },
    Boundary(TopologyBoundaryProbeTarget),
    AreaDisk {
        center: Point2,
        radius: f64,
    },
    AreaRegion(RegionId),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopologyProbeDefinition {
    pub id: ProbeId,
    pub name: String,
    pub color: [u8; 3],
    pub enabled: bool,
    pub target: TopologyProbeTarget,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopologyDocumentModel {
    pub draft: TopologyScene,
    pub accepted: TopologyScene,
    pub probes: Vec<TopologyProbeDefinition>,
    pub source: PointSource,
    pub far_field: FarFieldSettings,
}

impl Default for TopologyDocumentModel {
    fn default() -> Self {
        let scene = TopologyScene::default();
        Self {
            draft: scene.clone(),
            accepted: scene,
            probes: vec![],
            source: PointSource::default(),
            far_field: FarFieldSettings::default(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TopologyDocument {
    pub model: TopologyDocumentModel,
    pub presentation: PresentationSettings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopologyAcceptance {
    Pending,
    Valid,
    Invalid(TopologySceneIssue),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClosedCurvePurpose {
    Subdomain { material: MaterialId },
    Hole,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenCurvePurpose {
    SubdomainSeparator { material: MaterialId },
    BoundaryBaffle,
}

/// A viewport hit resolved against the editor's current compiled draft.
/// Snapshot-local face IDs are command input only and never enter the document.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TopologyAttachment {
    Boundary(FaceAnchor),
    Junction {
        vertex: TopologyVertexId,
        face: FaceId,
    },
    /// A free endpoint of an open curve: `endpoint` is 0 for its start and 1
    /// for its end. Attaching here welds the two curves into one rather than
    /// materialising a vertex.
    LooseEnd {
        curve: CurveId,
        endpoint: usize,
    },
    /// An interior breakpoint that owns no vertex yet, such as the seam an
    /// earlier weld left. Attaching here turns the breakpoint into a junction
    /// without splitting a span; `side` says which face the arm comes from.
    Breakpoint {
        curve: CurveId,
        node: usize,
        side: CurveTraceSide,
    },
}

/// Two open curves fused end to end into one. `survivor` kept its identity and
/// direction; `absorbed` is gone, reversed first when the ends demanded it. The
/// seam is node `seam_node` of the survivor and `seam_control` is its on-curve
/// control, so the UI can select it. `absorbed_far_node` is where the absorbed
/// curve's other end landed on the survivor. `survivor == absorbed` records a
/// curve whose two loose ends met and closed into a loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JoinRecord {
    pub survivor: CurveId,
    pub absorbed: CurveId,
    pub seam_node: usize,
    pub seam_control: usize,
    pub absorbed_far_node: usize,
}

/// What welding a loose end did. `curve` is the curve the dragged end now
/// belongs to; `seam_control` is set when two curves fused or a loop closed.
#[derive(Clone, Debug, PartialEq)]
pub struct TopologyWeld {
    pub curve: CurveId,
    pub seam_control: Option<usize>,
    pub promoted: bool,
    pub span_splits: Vec<TopologySpanSplit>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TopologySpanSplit {
    pub curve: CurveId,
    pub original: CurveSpanId,
    pub inserted: CurveSpanId,
    pub parameter: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TopologyCurveEdit {
    pub curve: CurveId,
    pub span_splits: Vec<TopologySpanSplit>,
}

/// What a partial span deletion changed. `pieces` are the surviving curves in
/// order, the first keeping the original identity; `promoted` names curves the
/// deletion demoted to baffles because it freed one of their endpoints.
#[derive(Clone, Debug, PartialEq)]
pub struct TopologySpanRemoval {
    pub pieces: Vec<CurveId>,
    pub promoted: Vec<CurveId>,
    pub joined: Vec<JoinRecord>,
    pub removed_regions: Vec<RegionId>,
    pub region_remaps: Vec<(RegionId, RegionId)>,
    pub removed_probes: Vec<ProbeId>,
}

/// What removing a whole curve changed. `promoted` names curves demoted to
/// baffles because the removal freed one of their ends; `joined` records the
/// pairs of loose ends the removal left at a two-arm junction and fused.
#[derive(Clone, Debug, PartialEq)]
pub struct TopologyCurveRemoval {
    pub curve: CurveId,
    pub promoted: Vec<CurveId>,
    pub joined: Vec<JoinRecord>,
    pub removed_regions: Vec<RegionId>,
    pub region_remaps: Vec<(RegionId, RegionId)>,
    pub removed_probes: Vec<ProbeId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ControlRemovalTarget {
    Closed { node: usize },
    OpenStart { control: usize },
    OpenInterior { node: usize },
    OpenEnd { from_end: usize },
}

pub struct TopologyEditor {
    pub document: TopologyDocument,
    pub revision: u64,
    pub acceptance: TopologyAcceptance,
    pub compiled_draft: Option<CompiledTopologyScene>,
    pub compiled_accepted: CompiledTopologyScene,
    undo: Vec<TopologyDocumentModel>,
    redo: Vec<TopologyDocumentModel>,
    before: Option<TopologyDocumentModel>,
    job: Option<TopologySceneJob>,
    next_curve: u64,
    next_span: u64,
    next_region: u64,
    next_vertex: u64,
    next_material: u64,
    next_probe: u64,
}

impl Default for TopologyEditor {
    fn default() -> Self {
        let document = TopologyDocument::default();
        let compiled = document.model.accepted.compile(0).unwrap();
        Self {
            document,
            revision: 0,
            acceptance: TopologyAcceptance::Valid,
            compiled_draft: Some(compiled.clone()),
            compiled_accepted: compiled,
            undo: vec![],
            redo: vec![],
            before: None,
            job: None,
            next_curve: 1,
            next_span: 1,
            next_region: BACKGROUND_REGION.0 + 1,
            next_vertex: 1,
            next_material: DEFAULT_MATERIAL.0 + 1,
            next_probe: 1,
        }
    }
}

impl TopologyEditor {
    pub fn from_document(document: TopologyDocument) -> Result<Self, String> {
        let compiled_accepted = document
            .model
            .accepted
            .compile(0)
            .map_err(|issue| format!("Accepted topology is invalid: {issue}"))?;
        let next_curve = next_id(
            document
                .model
                .draft
                .geometry
                .curves
                .iter()
                .chain(&document.model.accepted.geometry.curves)
                .map(|curve| curve.id.0),
        )?;
        let next_span = next_id(
            document
                .model
                .draft
                .geometry
                .curves
                .iter()
                .chain(&document.model.accepted.geometry.curves)
                .flat_map(|curve| curve.spans.iter().map(|span| span.id.0)),
        )?;
        let next_region = next_id(
            document
                .model
                .draft
                .regions
                .iter()
                .chain(&document.model.accepted.regions)
                .map(|region| region.id.0),
        )?;
        let next_vertex = next_id(
            document
                .model
                .draft
                .geometry
                .vertices
                .iter()
                .chain(&document.model.accepted.geometry.vertices)
                .map(|vertex| vertex.id.0),
        )?;
        let next_material = next_id(
            document
                .model
                .draft
                .materials
                .iter()
                .chain(&document.model.accepted.materials)
                .map(|material| material.id.0),
        )?;
        let next_probe = next_id(document.model.probes.iter().map(|probe| probe.id.0))?;
        let mut editor = Self {
            document,
            revision: 0,
            acceptance: TopologyAcceptance::Pending,
            compiled_draft: None,
            compiled_accepted,
            undo: vec![],
            redo: vec![],
            before: None,
            job: None,
            next_curve,
            next_span,
            next_region,
            next_vertex,
            next_material,
            next_probe,
        };
        editor.changed();
        Ok(editor)
    }

    pub fn begin(&mut self) {
        if self.before.is_none() {
            self.before = Some(self.document.model.clone());
        }
    }

    pub fn commit(&mut self) {
        let Some(before) = self.before.take() else {
            return;
        };
        if before == self.document.model {
            return;
        }
        self.undo.push(before);
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn cancel(&mut self) {
        if let Some(before) = self.before.take() {
            self.document.model = before;
            self.changed();
        }
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.before = None;
        self.redo
            .push(std::mem::replace(&mut self.document.model, previous));
        self.changed();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.before = None;
        self.undo
            .push(std::mem::replace(&mut self.document.model, next));
        self.changed();
        true
    }

    pub fn history_len(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }

    pub fn editing(&self) -> bool {
        self.before.is_some()
    }

    pub fn replace_validated(&mut self, document: TopologyDocument) -> Result<(), String> {
        *self = Self::from_document(document)?;
        Ok(())
    }

    pub fn replace_validated_with_history(
        &mut self,
        document: TopologyDocument,
    ) -> Result<(), String> {
        let mut loaded = Self::from_document(document)?;
        let previous = self.document.model.clone();
        let mut undo = std::mem::take(&mut self.undo);
        undo.push(previous);
        if undo.len() > HISTORY_LIMIT {
            undo.remove(0);
        }
        loaded.undo = undo;
        *self = loaded;
        Ok(())
    }

    pub fn set_domain_during_edit(&mut self, domain: DomainRect) -> Result<(), String> {
        if !domain.valid() {
            return Err("Domain extents are invalid".into());
        }
        if self.document.model.draft.geometry.domain != domain {
            self.document.model.draft.geometry.domain = domain;
            self.changed();
        }
        Ok(())
    }

    pub fn set_domain(&mut self, domain: DomainRect) -> Result<(), String> {
        if self.document.model.draft.geometry.domain == domain {
            return Ok(());
        }
        self.begin();
        if let Err(error) = self.set_domain_during_edit(domain) {
            self.cancel();
            return Err(error);
        }
        self.commit();
        Ok(())
    }

    pub fn set_physics(&mut self, physics: PhysicsModel) -> Result<(), String> {
        let previous = self.document.model.draft.physics;
        if previous == physics {
            return Ok(());
        }
        let materials = self
            .document
            .model
            .draft
            .materials
            .iter()
            .map(|material| {
                previous
                    .convert_material(physics, material)
                    .map_err(|error| {
                        format!("Could not convert material `{}`: {error}", material.name)
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.begin();
        self.document.model.draft.physics = physics;
        self.document.model.draft.materials = materials;
        self.changed();
        self.commit();
        Ok(())
    }

    pub fn add_material(&mut self) -> Result<MaterialId, String> {
        if self.document.model.draft.materials.len() >= MAX_MATERIALS {
            return Err(format!("Maximum {MAX_MATERIALS} materials"));
        }
        let id = MaterialId(self.next_material);
        self.next_material = self
            .next_material
            .checked_add(1)
            .ok_or("Material IDs exhausted")?;
        // A new material that kept `default_medium`'s slate would be invisible
        // against the background it shares, so cycle a distinguishable palette.
        const COLORS: [[u8; 3]; 6] = [
            [77, 121, 164],
            [129, 98, 168],
            [67, 139, 112],
            [174, 113, 72],
            [153, 86, 111],
            [102, 130, 67],
        ];
        let mut material = Material::default_medium();
        material.id = id;
        material.name = format!("Material {}", id.0);
        material.color =
            COLORS[(id.0.saturating_sub(DEFAULT_MATERIAL.0 + 1) as usize) % COLORS.len()];
        self.begin();
        self.document.model.draft.materials.push(material);
        self.changed();
        self.commit();
        Ok(id)
    }

    pub fn update_material(&mut self, material: Material) -> Result<(), String> {
        if !material.valid() {
            return Err("Material settings are invalid".into());
        }
        let id = material.id;
        let candidate = self
            .document
            .model
            .draft
            .materials
            .iter_mut()
            .find(|candidate| candidate.id == id)
            .ok_or("Material no longer exists")?;
        if *candidate == material {
            return Ok(());
        }
        self.begin();
        *self
            .document
            .model
            .draft
            .materials
            .iter_mut()
            .find(|candidate| candidate.id == id)
            .unwrap() = material;
        self.changed();
        self.commit();
        Ok(())
    }

    pub fn delete_material(&mut self, id: MaterialId) -> Result<(), String> {
        if id == DEFAULT_MATERIAL {
            return Err("The default material cannot be deleted".into());
        }
        if self
            .document
            .model
            .draft
            .regions
            .iter()
            .any(|region| region.material == id)
        {
            return Err("Material is assigned to a subdomain".into());
        }
        let index = self
            .document
            .model
            .draft
            .materials
            .iter()
            .position(|material| material.id == id)
            .ok_or("Material no longer exists")?;
        self.begin();
        self.document.model.draft.materials.remove(index);
        self.changed();
        self.commit();
        Ok(())
    }

    pub fn set_region_material(
        &mut self,
        region: RegionId,
        material: MaterialId,
    ) -> Result<(), String> {
        self.require_material(material)?;
        let candidate = self
            .document
            .model
            .draft
            .regions
            .iter_mut()
            .find(|candidate| candidate.id == region)
            .ok_or("Region no longer exists")?;
        if candidate.material == material {
            return Ok(());
        }
        self.begin();
        self.document
            .model
            .draft
            .regions
            .iter_mut()
            .find(|candidate| candidate.id == region)
            .unwrap()
            .material = material;
        self.changed();
        self.commit();
        Ok(())
    }

    pub fn set_region_frame_during_edit(
        &mut self,
        region: RegionId,
        frame: MaterialFrame,
    ) -> Result<(), String> {
        if !frame.valid() {
            return Err("Material frame is invalid".into());
        }
        let candidate = self
            .document
            .model
            .draft
            .regions
            .iter_mut()
            .find(|candidate| candidate.id == region)
            .ok_or("Region no longer exists")?;
        if candidate.frame != frame {
            candidate.frame = frame;
            self.changed();
        }
        Ok(())
    }

    pub fn set_region_frame(
        &mut self,
        region: RegionId,
        frame: MaterialFrame,
    ) -> Result<(), String> {
        self.begin();
        if let Err(error) = self.set_region_frame_during_edit(region, frame) {
            self.cancel();
            return Err(error);
        }
        self.commit();
        Ok(())
    }

    pub fn set_volume_source(
        &mut self,
        region: RegionId,
        source: Option<VolumeSource>,
    ) -> Result<(), String> {
        if self.document.model.draft.region(region).is_none() {
            return Err("Region no longer exists".into());
        }
        if source
            .as_ref()
            .is_some_and(|source| !source.valid() || source.region != region)
        {
            return Err("Volume source settings are invalid".into());
        }
        let mut sources = self.document.model.draft.volume_sources.clone();
        sources.retain(|candidate| candidate.region != region);
        if let Some(source) = source {
            if sources.len() >= MAX_VOLUME_SOURCES {
                return Err(format!("Maximum {MAX_VOLUME_SOURCES} volume sources"));
            }
            sources.push(source);
        }
        if sources == self.document.model.draft.volume_sources {
            return Ok(());
        }
        self.begin();
        self.document.model.draft.volume_sources = sources;
        self.changed();
        self.commit();
        Ok(())
    }

    pub fn set_point_source(&mut self, source: PointSource) -> Result<(), String> {
        self.begin();
        if let Err(error) = self.set_point_source_during_edit(source) {
            self.cancel();
            return Err(error);
        }
        self.commit();
        Ok(())
    }

    pub fn set_point_source_during_edit(&mut self, source: PointSource) -> Result<(), String> {
        if !source.valid() || self.document.model.draft.region(source.region).is_none() {
            return Err("Point source settings are invalid".into());
        }
        if self.document.model.source == source {
            return Ok(());
        }
        self.document.model.source = source;
        self.changed();
        Ok(())
    }

    pub fn set_far_field(&mut self, settings: FarFieldSettings) -> Result<(), String> {
        if !settings.valid() {
            return Err("Far-field settings are invalid".into());
        }
        if self.document.model.far_field == settings {
            return Ok(());
        }
        self.begin();
        self.document.model.far_field = settings;
        self.changed();
        self.commit();
        Ok(())
    }

    pub fn create_probe(
        &mut self,
        name: String,
        color: [u8; 3],
        target: TopologyProbeTarget,
    ) -> Result<ProbeId, String> {
        if self.document.model.probes.len() >= MAX_PROBES {
            return Err(format!("Maximum {MAX_PROBES} probes"));
        }
        let id = ProbeId(self.next_probe);
        self.next_probe = self
            .next_probe
            .checked_add(1)
            .ok_or("Probe IDs exhausted")?;
        let probe = TopologyProbeDefinition {
            id,
            name,
            color,
            enabled: true,
            target,
        };
        if !probe_definition_valid(&probe, &self.document.model.draft) {
            return Err("Probe settings are invalid".into());
        }
        self.begin();
        self.document.model.probes.push(probe);
        self.changed();
        self.commit();
        Ok(id)
    }

    pub fn update_probe(&mut self, probe: TopologyProbeDefinition) -> Result<(), String> {
        self.begin();
        if let Err(error) = self.update_probe_during_edit(probe) {
            self.cancel();
            return Err(error);
        }
        self.commit();
        Ok(())
    }

    pub fn update_probe_during_edit(
        &mut self,
        probe: TopologyProbeDefinition,
    ) -> Result<(), String> {
        if !probe_definition_valid(&probe, &self.document.model.draft) {
            return Err("Probe settings are invalid".into());
        }
        let index = self
            .document
            .model
            .probes
            .iter()
            .position(|candidate| candidate.id == probe.id)
            .ok_or("Probe no longer exists")?;
        if self.document.model.probes[index] == probe {
            return Ok(());
        }
        self.document.model.probes[index] = probe;
        self.changed();
        Ok(())
    }

    pub fn delete_probe(&mut self, id: ProbeId) -> Result<(), String> {
        let index = self
            .document
            .model
            .probes
            .iter()
            .position(|probe| probe.id == id)
            .ok_or("Probe no longer exists")?;
        self.begin();
        self.document.model.probes.remove(index);
        self.changed();
        self.commit();
        Ok(())
    }

    pub fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.acceptance = TopologyAcceptance::Pending;
        self.compiled_draft = None;
        self.job = Some(TopologySceneJob::new(
            self.document.model.draft.clone(),
            self.revision,
        ));
    }

    pub fn validate_frame(&mut self, budget: usize) {
        let Some(result) = self.job.as_mut().and_then(|job| job.advance(budget)) else {
            return;
        };
        self.job = None;
        match result {
            Ok(compiled) if compiled.topology.revision == self.revision => {
                self.document.model.accepted = self.document.model.draft.clone();
                self.compiled_accepted = compiled.clone();
                self.compiled_draft = Some(compiled);
                self.acceptance = TopologyAcceptance::Valid;
            }
            Err(issue) => self.acceptance = TopologyAcceptance::Invalid(issue),
            Ok(_) => {}
        }
    }

    pub fn create_closed_curve(
        &mut self,
        spline: PeriodicCubicSpline,
        purpose: ClosedCurvePurpose,
    ) -> Result<CurveId, String> {
        let curve_id = self.allocate_curve()?;
        let behavior = match purpose {
            ClosedCurvePurpose::Subdomain { material } => {
                self.require_material(material)?;
                SpanBehavior::Transmitting
            }
            ClosedCurvePurpose::Hole => SpanBehavior::REFLECTING,
        };
        let spans = self.allocate_spans(spline.intervals().len(), behavior)?;
        let curve = TopologyCurve::new(curve_id, CurveSpline::Closed(spline), spans)
            .map_err(|issue| issue.to_string())?;

        let mut candidate = self.document.model.draft.clone();
        candidate.geometry.curves.push(curve);
        let topology = compile_topology(&candidate.geometry, self.revision.wrapping_add(1))
            .map_err(|issue| issue.to_string())?;
        let assigned = candidate
            .face_assignments
            .iter()
            .map(|assignment| assignment.anchor.resolve(&topology))
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(|issue| issue.to_string())?;
        let curve = candidate.geometry.curves.last().unwrap();
        let bounds = curve.spline.span_bounds(0).unwrap();
        let parameter = (bounds[0] + bounds[1]) * 0.5;
        let mut anchors =
            [CurveTraceSide::Left, CurveTraceSide::Right].map(|side| FaceAnchor::Curve {
                curve: curve_id,
                span: curve.spans[0].id,
                side,
                parameter,
            });
        let anchor = anchors
            .iter_mut()
            .find(|anchor| {
                anchor
                    .resolve(&topology)
                    .is_ok_and(|face| !assigned.contains(&face))
            })
            .copied()
            .ok_or("Closed curve does not create one new interior face")?;
        let region = match purpose {
            ClosedCurvePurpose::Subdomain { material } => {
                let id = self.allocate_region()?;
                candidate.regions.push(Region {
                    id,
                    material,
                    frame: face_frame(&topology, anchor.resolve(&topology).ok()),
                });
                Some(id)
            }
            ClosedCurvePurpose::Hole => None,
        };
        candidate
            .face_assignments
            .push(AuthoredFaceAssignment { anchor, region });
        candidate
            .compile(self.revision.wrapping_add(1))
            .map_err(|issue| issue.to_string())?;
        self.begin();
        self.document.model.draft = candidate;
        self.changed();
        self.commit();
        Ok(curve_id)
    }

    pub fn create_boundary_baffle(&mut self, spline: OpenCubicSpline) -> Result<CurveId, String> {
        self.create_open_curve(spline, OpenCurvePurpose::BoundaryBaffle, None, None)
            .map(|edit| edit.curve)
    }

    pub fn create_open_curve(
        &mut self,
        spline: OpenCubicSpline,
        purpose: OpenCurvePurpose,
        start: Option<TopologyAttachment>,
        end: Option<TopologyAttachment>,
    ) -> Result<TopologyCurveEdit, String> {
        let compiled = self
            .compiled_draft
            .clone()
            .ok_or("Finish the current invalid topology edit before drawing a new open curve")?;
        let start_face = start
            .map(|target| attachment_face(target, &compiled))
            .transpose()?;
        let end_face = end
            .map(|target| attachment_face(target, &compiled))
            .transpose()?;
        if let (Some(start_face), Some(end_face)) = (start_face, end_face)
            && start_face != end_face
        {
            return Err("Open-curve endpoints must attach to the same face".into());
        }

        let source_face = start_face
            .or(end_face)
            .or_else(|| compiled.topology.face_at(open_spline_midpoint(&spline)))
            .ok_or("Open curve must lie in a bounded face")?;
        let source_region = compiled
            .assignments
            .iter()
            .find(|assignment| assignment.face == source_face)
            .map(|assignment| assignment.region)
            .ok_or("Attachment face no longer exists")?;
        let new_material = match purpose {
            OpenCurvePurpose::SubdomainSeparator { material } => {
                self.require_material(material)?;
                if start.is_none() || end.is_none() {
                    return Err("A subdomain separator needs two attached endpoints".into());
                }
                if source_region.is_none() {
                    return Err("A subdomain separator cannot split an excluded face".into());
                }
                Some(material)
            }
            OpenCurvePurpose::BoundaryBaffle => None,
        };

        let curve_id = self.allocate_curve()?;
        let behavior = match purpose {
            OpenCurvePurpose::SubdomainSeparator { .. } => SpanBehavior::Transmitting,
            OpenCurvePurpose::BoundaryBaffle => SpanBehavior::REFLECTING,
        };
        let spans = self.allocate_spans(spline.intervals().len(), behavior)?;
        let mut curve = TopologyCurve::new(curve_id, CurveSpline::Open(spline), spans)
            .map_err(|issue| issue.to_string())?;
        let mut candidate = self.document.model.clone();
        let mut span_splits = vec![];
        // Loose ends are welded after the curve exists; everything else pins
        // the new end onto a materialised vertex.
        let mut loose = vec![];
        let last = curve.nodes.len() - 1;
        for (node, target) in [(0, start), (last, end)] {
            let Some(target) = target else { continue };
            if let TopologyAttachment::LooseEnd {
                curve: other,
                endpoint,
            } = target
            {
                let other_curve = candidate
                    .draft
                    .geometry
                    .curve(other)
                    .ok_or("Attachment curve no longer exists")?;
                let other_node = loose_end_node(other_curve, endpoint)?;
                let tip = other_curve
                    .spline
                    .node_point(other_node)
                    .ok_or("Attachment curve end no longer exists")?;
                curve
                    .spline
                    .set_node_point(node, tip)
                    .map_err(|error| error.to_string())?;
                loose.push((node, other, other_node));
                continue;
            }
            let (vertex, point) = self.materialize_attachment(
                &mut candidate.draft,
                &mut candidate.probes,
                target,
                &mut span_splits,
            )?;
            curve
                .spline
                .set_node_point(node, point)
                .map_err(|error| error.to_string())?;
            curve.nodes[node].vertex = Some(vertex);
        }
        candidate.draft.geometry.curves.push(curve);

        // The start-side existing curve keeps its identity: the drawn curve
        // dissolves into it, and an end-side curve is then absorbed in turn.
        let mut surviving = curve_id;
        let mut far_node = last;
        let mut joined = vec![];
        for (node, other, other_node) in loose {
            if other == surviving {
                refine_curve_to_spans(&mut candidate, &mut self.next_span, surviving, 2)?;
                close_curve_geometry(&mut candidate, surviving)?;
                break;
            }
            let record = if node == 0 {
                join_curves(&mut candidate, (other, other_node), (surviving, 0))?
            } else {
                join_curves(&mut candidate, (surviving, far_node), (other, other_node))?
            };
            surviving = record.survivor;
            far_node = record.absorbed_far_node;
            joined.push(record);
        }

        let topology = compile_topology(&candidate.draft.geometry, self.revision.wrapping_add(1))
            .map_err(|issue| issue.to_string())?;
        self.assign_new_faces(
            &mut candidate.draft,
            &topology,
            surviving,
            source_region,
            new_material,
        )?;
        candidate
            .draft
            .compile(self.revision.wrapping_add(1))
            .map_err(|issue| issue.to_string())?;

        self.begin();
        self.document.model = candidate;
        self.changed();
        self.commit();
        Ok(TopologyCurveEdit {
            curve: surviving,
            span_splits,
        })
    }

    pub fn detach_endpoint(&mut self, curve: CurveId, endpoint: usize) -> Result<(), String> {
        let mut candidate = self.document.model.clone();
        let target = candidate
            .draft
            .geometry
            .curves
            .iter_mut()
            .find(|candidate| candidate.id == curve)
            .ok_or("Curve no longer exists")?;
        if !target.spline.is_open() || !matches!(endpoint, 0 | 1) {
            return Err("Choose the start or end of an open curve".into());
        }
        let node = if endpoint == 0 {
            0
        } else {
            target.nodes.len() - 1
        };
        let Some(vertex) = target.nodes[node].vertex.take() else {
            return Err("Endpoint is already free".into());
        };
        let _ = vertex;
        prune_unreferenced_vertices(&mut candidate.draft.geometry);
        self.begin();
        self.document.model = candidate;
        self.changed();
        self.commit();
        Ok(())
    }

    pub fn attach_endpoint(
        &mut self,
        curve: CurveId,
        endpoint: usize,
        target: TopologyAttachment,
    ) -> Result<Vec<TopologySpanSplit>, String> {
        if matches!(target, TopologyAttachment::LooseEnd { .. }) {
            return Err("Weld loose ends with weld_endpoint".into());
        }
        let compiled = self
            .compiled_draft
            .as_ref()
            .unwrap_or(&self.compiled_accepted)
            .clone();
        let mut candidate = self.document.model.clone();
        let (source_region, span_splits) =
            self.attach_end_to_vertex(&mut candidate, &compiled, curve, endpoint, target)?;
        if let Ok(topology) =
            compile_topology(&candidate.draft.geometry, self.revision.wrapping_add(1))
        {
            self.assign_new_faces(&mut candidate.draft, &topology, curve, source_region, None)?;
        }
        self.begin();
        self.document.model = candidate;
        self.changed();
        self.commit();
        Ok(span_splits)
    }

    /// Materialises `target` as a vertex and pins one free end of `curve` onto
    /// it. Shared by [`Self::attach_endpoint`] and [`Self::weld_endpoint`].
    fn attach_end_to_vertex(
        &mut self,
        candidate: &mut TopologyDocumentModel,
        compiled: &CompiledTopologyScene,
        curve: CurveId,
        endpoint: usize,
        target: TopologyAttachment,
    ) -> Result<(Option<RegionId>, Vec<TopologySpanSplit>), String> {
        let source_face = attachment_face(target, compiled)?;
        let source_region = compiled
            .assignments
            .iter()
            .find(|assignment| assignment.face == source_face)
            .map(|assignment| assignment.region)
            .ok_or("Attachment face no longer exists")?;
        loose_end_node(
            candidate
                .draft
                .geometry
                .curve(curve)
                .ok_or("Curve no longer exists")?,
            endpoint,
        )?;
        let mut span_splits = vec![];
        let (vertex, point) = self.materialize_attachment(
            &mut candidate.draft,
            &mut candidate.probes,
            target,
            &mut span_splits,
        )?;
        // A curve may attach to itself, and the split that made the junction
        // then inserted a node ahead of the tip, so locate the tip again.
        let node = loose_end_node(
            candidate
                .draft
                .geometry
                .curve(curve)
                .ok_or("Curve no longer exists")?,
            endpoint,
        )?;
        let attached = candidate
            .draft
            .geometry
            .curves
            .iter_mut()
            .find(|candidate| candidate.id == curve)
            .ok_or("Curve no longer exists")?;
        attached
            .spline
            .set_node_point(node, point)
            .map_err(|error| error.to_string())?;
        attached.nodes[node].vertex = Some(vertex);
        Ok((source_region, span_splits))
    }

    /// Welds a loose end onto whatever the user dropped it on. Two loose ends
    /// fuse into one curve (or close a loop when they belong to the same
    /// curve); a junction, an outer side, a curve interior, or a vertex-less
    /// breakpoint gains an arm. The arrangement must compile afterwards or the
    /// weld is refused and only the drag remains.
    pub fn weld_endpoint(
        &mut self,
        curve: CurveId,
        endpoint: usize,
        target: TopologyAttachment,
    ) -> Result<TopologyWeld, String> {
        let compiled = self
            .compiled_draft
            .as_ref()
            .unwrap_or(&self.compiled_accepted)
            .clone();
        let mut candidate = self.document.model.clone();
        let dragged = candidate
            .draft
            .geometry
            .curve(curve)
            .ok_or("Curve no longer exists")?
            .clone();
        let node = loose_end_node(&dragged, endpoint)?;
        let region_at = |point: Point2| {
            compiled.topology.face_at(point).and_then(|face| {
                compiled
                    .assignments
                    .iter()
                    .find(|assignment| assignment.face == face)
                    .and_then(|assignment| assignment.region)
            })
        };
        let mut span_splits = vec![];
        let mut seam_control = None;
        let mut face_curve = curve;
        let source_region;
        match target {
            TopologyAttachment::LooseEnd {
                curve: other,
                endpoint: other_end,
            } if other == curve => {
                if other_end == endpoint {
                    return Err("Choose a different end to weld to".into());
                }
                let CurveSpline::Open(spline) = &dragged.spline else {
                    return Err("Curve is already closed".into());
                };
                source_region = region_at(open_spline_midpoint(spline));
                // A single span cannot become a loop: a periodic cubic needs
                // four controls. Refine first, exactly, so the weld still lands.
                refine_curve_to_spans(&mut candidate, &mut self.next_span, curve, 2)?;
                close_curve_geometry(&mut candidate, curve)?;
                seam_control =
                    candidate
                        .draft
                        .geometry
                        .curve(curve)
                        .and_then(|closed| match &closed.spline {
                            CurveSpline::Closed(spline) => Some(spline.controls().len() - 1),
                            CurveSpline::Open(_) => None,
                        });
            }
            TopologyAttachment::LooseEnd {
                curve: other,
                endpoint: other_end,
            } => {
                let other_curve = candidate
                    .draft
                    .geometry
                    .curve(other)
                    .ok_or("Attachment curve no longer exists")?;
                let other_node = loose_end_node(other_curve, other_end)?;
                let tip = other_curve
                    .spline
                    .node_point(other_node)
                    .ok_or("Attachment curve end no longer exists")?;
                source_region = region_at(tip);
                let record = join_curves(&mut candidate, (other, other_node), (curve, node))?;
                face_curve = record.survivor;
                seam_control = Some(record.seam_control);
            }
            _ => {
                let (region, splits) =
                    self.attach_end_to_vertex(&mut candidate, &compiled, curve, endpoint, target)?;
                source_region = region;
                span_splits = splits;
            }
        }
        // A join can hand transmitting spans a free tip; the result is a baffle.
        let promoted = promote_curve_if_freed(&mut candidate.draft.geometry, face_curve);
        let topology = compile_topology(&candidate.draft.geometry, self.revision.wrapping_add(1))
            .map_err(|issue| issue.to_string())?;
        self.assign_new_faces(
            &mut candidate.draft,
            &topology,
            face_curve,
            source_region,
            None,
        )?;
        candidate
            .draft
            .compile(self.revision.wrapping_add(1))
            .map_err(|issue| issue.to_string())?;
        self.begin();
        self.document.model = candidate;
        self.changed();
        self.commit();
        Ok(TopologyWeld {
            curve: face_curve,
            seam_control,
            promoted,
            span_splits,
        })
    }

    /// Active regions a curve removal would merge into one. More than one means
    /// removing the curve merges subdomains and the caller must say which
    /// survives. The answer comes from compiling the removal, so it already
    /// accounts for curves the removal promotes or welds.
    pub fn curve_removal_choices(&self, curve: CurveId) -> Result<Vec<RegionId>, String> {
        Ok(self
            .plan_removal(RemovalTarget::Curve(curve))?
            .merge
            .map(|merge| merge.regions.into_iter().collect())
            .unwrap_or_default())
    }
    /// The face a closed curve encloses, identified through the authored anchor
    /// that curve owns rather than through snapshot traversal order.
    /// The face assignment anchored on `curve`, by document index. A curve that
    /// bounds more than one face owns more than one; this returns the first, so
    /// callers that must be exact should pick the face themselves.
    pub fn enclosed_assignment(&self, curve: CurveId) -> Option<usize> {
        self.document
            .model
            .draft
            .face_assignments
            .iter()
            .position(|assignment| {
                matches!(
                    assignment.anchor,
                    FaceAnchor::Curve { curve: owner, .. } if owner == curve
                )
            })
    }

    /// The region an assignment carries, or `None` when that face is a hole.
    pub fn assignment_region(&self, assignment: usize) -> Option<RegionId> {
        self.document
            .model
            .draft
            .face_assignments
            .get(assignment)
            .and_then(|assignment| assignment.region)
    }

    /// Turns one assigned face into a hole, or back into a subdomain carrying
    /// `material`. Only that face's own boundary is re-walled, and only where it
    /// genuinely divides two faces: such a span transmits exactly when the faces
    /// on both of its sides are active subdomains. A slit inside the face keeps
    /// whatever the user gave it, and nothing outside the face is touched.
    pub fn set_face_disposition(
        &mut self,
        assignment: usize,
        material: Option<MaterialId>,
    ) -> Result<(), String> {
        if let Some(material) = material {
            self.require_material(material)?;
        }
        let compiled = self
            .compiled_draft
            .clone()
            .ok_or("Resolve the invalid draft before changing this subdomain".to_owned())?;
        let current = self
            .document
            .model
            .draft
            .face_assignments
            .get(assignment)
            .copied()
            .ok_or("That subdomain no longer exists")?;
        if current.region.is_some() == material.is_some() {
            if let (Some(region), Some(material)) = (current.region, material) {
                return self.set_region_material(region, material);
            }
            return Ok(());
        }
        let face = current
            .anchor
            .resolve(&compiled.topology)
            .map_err(|issue| issue.to_string())?;

        // Which faces carry a material once this change lands.
        let mut active = compiled
            .assignments
            .iter()
            .filter(|assignment| assignment.region.is_some())
            .map(|assignment| assignment.face)
            .collect::<BTreeSet<_>>();
        if material.is_some() {
            active.insert(face);
        } else {
            active.remove(&face);
        }
        let mut walls = BTreeSet::new();
        let mut openings = BTreeSet::new();
        for edge in &compiled.topology.edges {
            let CompiledEdgeSource::Curve(span) = edge.source else {
                continue;
            };
            if edge.left == edge.right || (edge.left != face && edge.right != face) {
                continue;
            }
            if active.contains(&edge.left) && active.contains(&edge.right) {
                openings.insert(span);
            } else {
                walls.insert(span);
            }
        }

        let mut candidate = self.document.model.clone();
        for curve in &mut candidate.draft.geometry.curves {
            for span in &mut curve.spans {
                if walls.contains(&span.id) {
                    span.behavior = SpanBehavior::REFLECTING;
                } else if openings.contains(&span.id) {
                    span.behavior = SpanBehavior::Transmitting;
                }
            }
        }
        let slot = candidate
            .draft
            .face_assignments
            .get_mut(assignment)
            .ok_or("That subdomain no longer exists")?;
        match material {
            Some(material) => {
                let id = self.allocate_region()?;
                slot.region = Some(id);
                let frame = face_frame(&compiled.topology, Some(face));
                candidate.draft.regions.push(Region {
                    id,
                    material,
                    frame,
                });
            }
            None => {
                slot.region = None;
                if let Some(region) = current.region {
                    drop_region_dependents(&mut candidate, region);
                }
            }
        }
        candidate
            .draft
            .compile(self.revision.wrapping_add(1))
            .map_err(|issue| issue.to_string())?;
        self.begin();
        self.document.model = candidate;
        self.changed();
        self.commit();
        Ok(())
    }

    /// The curve and contiguous run a span removal acts on. `whole` marks a
    /// selection that covers the complete curve, which is ordinary curve removal.
    fn span_removal_target(
        &self,
        selected: &BTreeSet<CurveSpanId>,
    ) -> Result<(CurveId, Vec<usize>, bool), String> {
        if selected.is_empty() {
            return Err("Select curve spans to delete".into());
        }
        let mut touched = self
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .filter(|curve| curve.spans.iter().any(|span| selected.contains(&span.id)));
        let curve = touched.next().ok_or("Selected spans no longer exist")?;
        if touched.next().is_some() {
            return Err("Select one contiguous run of spans on one curve".into());
        }
        if curve.spans.iter().all(|span| selected.contains(&span.id)) {
            return Ok((curve.id, (0..curve.spans.len()).collect(), true));
        }
        let run = contiguous_run(curve, selected)?.ok_or("Selected spans no longer exist")?;
        Ok((curve.id, run, false))
    }

    /// Active regions that a span removal would merge into one. More than one
    /// means the caller must say which survives, exactly as for whole-curve
    /// removal. The answer comes from compiling the removal, so it already
    /// accounts for curves the deletion promotes or welds.
    pub fn span_removal_choices(
        &self,
        selected: &BTreeSet<CurveSpanId>,
    ) -> Result<Vec<RegionId>, String> {
        let (curve, run, whole) = self.span_removal_target(selected)?;
        if whole {
            return self.curve_removal_choices(curve);
        }
        Ok(self
            .plan_removal(RemovalTarget::Spans { curve, run })?
            .merge
            .map(|merge| merge.regions.into_iter().collect())
            .unwrap_or_default())
    }

    /// Deletes one contiguous run of spans, leaving the rest of the curve as open
    /// pieces. Every surviving piece becomes a baffle with the default wall,
    /// because an open curve that kept a transmitting end span with a free tip is
    /// rejected by the arrangement compiler. Any curve the deletion frees from a
    /// junction is promoted the same way, and two loose ends left at a junction
    /// fuse into one curve.
    pub fn remove_spans(
        &mut self,
        selected: &BTreeSet<CurveSpanId>,
        keep_region: Option<RegionId>,
    ) -> Result<TopologySpanRemoval, String> {
        let (curve, run, whole) = self.span_removal_target(selected)?;
        if whole {
            let removal = self.remove_curve(curve, keep_region)?;
            return Ok(TopologySpanRemoval {
                pieces: vec![],
                promoted: removal.promoted,
                joined: removal.joined,
                removed_regions: removal.removed_regions,
                region_remaps: removal.region_remaps,
                removed_probes: removal.removed_probes,
            });
        }
        let plan = self.plan_removal(RemovalTarget::Spans { curve, run })?;
        let outcome = self.apply_removal(plan, keep_region)?;
        Ok(TopologySpanRemoval {
            pieces: outcome.pieces,
            promoted: outcome.promoted,
            joined: outcome.joined,
            removed_regions: outcome.removed_regions,
            region_remaps: outcome.region_remaps,
            removed_probes: outcome.removed_probes,
        })
    }

    /// Removes a curve and deterministically resolves any face merge. When two
    /// or more active regions meet across the removed curve, `keep_region` is
    /// required. Curves the removal frees are promoted to baffles; two loose
    /// ends left at a junction fuse into one curve.
    pub fn remove_curve(
        &mut self,
        curve: CurveId,
        keep_region: Option<RegionId>,
    ) -> Result<TopologyCurveRemoval, String> {
        let plan = self.plan_removal(RemovalTarget::Curve(curve))?;
        let outcome = self.apply_removal(plan, keep_region)?;
        Ok(TopologyCurveRemoval {
            curve,
            promoted: outcome.promoted,
            joined: outcome.joined,
            removed_regions: outcome.removed_regions,
            region_remaps: outcome.region_remaps,
            removed_probes: outcome.removed_probes,
        })
    }

    /// Works a removal out on a copy of the document: cuts or removes the
    /// geometry, prunes and promotes, fuses loose ends the removal leaves at a
    /// two-arm junction, compiles, and finds where every old face landed. The
    /// editor is untouched and no id is allocated — a second cut piece takes the
    /// next curve id provisionally and `apply_removal` claims it.
    fn plan_removal(&self, target: RemovalTarget) -> Result<RemovalPlan, String> {
        let compiled = self.compiled_draft.as_ref().ok_or(match target {
            RemovalTarget::Curve(_) => "Resolve the invalid draft before removing a topology curve",
            RemovalTarget::Spans { .. } => "Resolve the invalid draft before deleting spans",
        })?;
        let mut candidate = self.document.model.clone();
        let mut removed_probes = vec![];
        let mut pieces = vec![];
        let mut provisional_curve = false;
        let (dead_spans, touched, mut reparameterised) = match &target {
            RemovalTarget::Curve(curve_id) => {
                let curve = candidate
                    .draft
                    .geometry
                    .curve(*curve_id)
                    .ok_or("Curve no longer exists")?
                    .clone();
                let touched = curve
                    .nodes
                    .iter()
                    .filter_map(|node| node.vertex)
                    .collect::<BTreeSet<_>>();
                let dead = curve
                    .spans
                    .iter()
                    .map(|span| span.id)
                    .collect::<BTreeSet<_>>();
                candidate
                    .draft
                    .geometry
                    .curves
                    .retain(|candidate| candidate.id != *curve_id);
                candidate.probes.retain(|probe| {
                    let gone = matches!(
                        &probe.target,
                        TopologyProbeTarget::Boundary(target) if target.curve == *curve_id
                    );
                    if gone {
                        removed_probes.push(probe.id);
                    }
                    !gone
                });
                (dead, touched, BTreeSet::from([*curve_id]))
            }
            RemovalTarget::Spans {
                curve: curve_id,
                run,
            } => {
                let curve = candidate
                    .draft
                    .geometry
                    .curve(*curve_id)
                    .ok_or("Curve no longer exists")?
                    .clone();
                let cut = CurveCut::plan(&curve, run)?;
                let dead = run
                    .iter()
                    .map(|index| curve.spans[*index].id)
                    .collect::<BTreeSet<_>>();
                let touched = curve
                    .nodes
                    .iter()
                    .filter_map(|node| node.vertex)
                    .collect::<BTreeSet<_>>();
                let mut built = Vec::new();
                let mut rebased = Vec::new();
                for (index, piece) in cut.pieces.iter().enumerate() {
                    let id = if index == 0 {
                        curve.id
                    } else {
                        provisional_curve = true;
                        CurveId(self.next_curve)
                    };
                    let spans = piece
                        .spans
                        .iter()
                        .map(|span| CurveSpan {
                            id: curve.spans[*span].id,
                            behavior: SpanBehavior::REFLECTING,
                        })
                        .collect::<Vec<_>>();
                    let mut built_piece = TopologyCurve::new(id, piece.spline.clone(), spans)
                        .map_err(|issue| issue.to_string())?;
                    for (node, original) in piece.nodes.iter().enumerate() {
                        built_piece.nodes[node] = curve.nodes[*original];
                    }
                    for (position, span) in piece.spans.iter().enumerate() {
                        let old = curve
                            .spline
                            .span_bounds(*span)
                            .ok_or("Missing curve span")?;
                        let new = built_piece
                            .spline
                            .span_bounds(position)
                            .ok_or("Missing curve span")?;
                        rebased.push((curve.spans[*span].id, id, old, new));
                    }
                    built.push(built_piece);
                }
                let position = candidate
                    .draft
                    .geometry
                    .curves
                    .iter()
                    .position(|candidate| candidate.id == curve.id)
                    .ok_or("Curve no longer exists")?;
                candidate.draft.geometry.curves.remove(position);
                for (offset, piece) in built.iter().enumerate() {
                    candidate
                        .draft
                        .geometry
                        .curves
                        .insert(position + offset, piece.clone());
                }
                // Anchors on kept spans follow their span onto its piece, with
                // the parameter rebased into the piece's own domain.
                for assignment in &mut candidate.draft.face_assignments {
                    let FaceAnchor::Curve {
                        curve: anchor_curve,
                        span,
                        parameter,
                        ..
                    } = &mut assignment.anchor
                    else {
                        continue;
                    };
                    if *anchor_curve != curve.id {
                        continue;
                    }
                    let Some((_, piece, old, new)) = rebased
                        .iter()
                        .find(|(candidate, _, _, _)| candidate == span)
                    else {
                        continue;
                    };
                    let width = old[1] - old[0];
                    let fraction = if width.abs() > f64::MIN_POSITIVE {
                        ((*parameter - old[0]) / width).clamp(0.0, 1.0)
                    } else {
                        0.5
                    };
                    *anchor_curve = *piece;
                    *parameter = new[0] + (new[1] - new[0]) * fraction;
                }
                removed_probes.extend(remap_cut_boundary_probes(
                    &mut candidate.probes,
                    curve.id,
                    &curve,
                    &cut,
                    &built,
                ));
                pieces = built.iter().map(|piece| piece.id).collect();
                (dead, touched, BTreeSet::from([curve.id]))
            }
        };
        prune_dangling_junctions(&mut candidate.draft.geometry);
        let mut promoted = promote_freed_curves(&mut candidate.draft.geometry);
        let joined = join_valence_two_ends(&mut candidate, &touched);
        if !joined.is_empty() {
            for record in &joined {
                reparameterised.insert(record.survivor);
                reparameterised.insert(record.absorbed);
            }
            // A fusion can hand transmitting spans a free tip.
            for curve in promote_freed_curves(&mut candidate.draft.geometry) {
                if !promoted.contains(&curve) {
                    promoted.push(curve);
                }
            }
        }
        let existing = candidate
            .draft
            .geometry
            .curves
            .iter()
            .map(|curve| curve.id)
            .collect::<BTreeSet<_>>();
        pieces.retain(|piece| existing.contains(piece));

        // Every old face paired with its anchor as the cut and the fusions left
        // it: the face comes from the original anchor against the old snapshot,
        // the anchor from the candidate, which the loops above rewrote in place.
        let old_assignments = self
            .document
            .model
            .draft
            .face_assignments
            .iter()
            .zip(&candidate.draft.face_assignments)
            .map(|(original, rebased)| {
                original
                    .anchor
                    .resolve(&compiled.topology)
                    .map(|face| (face, *rebased))
                    .map_err(|issue| issue.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let topology = compile_topology(&candidate.draft.geometry, self.revision.wrapping_add(1))
            .map_err(|issue| issue.to_string())?;
        let dead = |anchor: &FaceAnchor| matches!(anchor, FaceAnchor::Curve { span, .. } if dead_spans.contains(span));
        let mut landings = BTreeMap::<FaceId, Landing>::new();
        let mut live = vec![];
        let mut vanishing = BTreeSet::new();
        for (old_face, assignment) in &old_assignments {
            if dead(&assignment.anchor) {
                if let Some(region) = assignment.region {
                    vanishing.insert(region);
                }
                let Some(new_face) = attribute_dead_face(
                    compiled,
                    &topology,
                    *old_face,
                    &dead_spans,
                    &reparameterised,
                ) else {
                    continue;
                };
                let landing = landings.entry(new_face).or_default();
                match assignment.region {
                    Some(region) => {
                        landing.dead_regions.insert(region);
                    }
                    None => landing.dead_hole = true,
                }
            } else {
                let new_face = assignment.anchor.resolve(&topology).map_err(|issue| {
                    format!("Face anchor no longer resolves after removal: {issue}")
                })?;
                landings.entry(new_face).or_default().live.push(*assignment);
                live.push((new_face, *assignment));
            }
        }
        let mut merges = landings
            .iter()
            .filter_map(|(face, landing)| {
                let regions = landing.regions();
                (regions.len() >= 2).then_some(Merge {
                    face: *face,
                    regions,
                })
            })
            .collect::<Vec<_>>();
        if merges.len() > 1 {
            return Err(
                "This deletion would merge subdomains in more than one place; delete a smaller part"
                    .into(),
            );
        }
        Ok(RemovalPlan {
            candidate,
            topology,
            live,
            landings,
            merge: merges.pop(),
            vanishing,
            pieces,
            promoted,
            joined,
            removed_probes,
            provisional_curve,
        })
    }

    /// Settles a planned removal with the caller's survivor choice and commits
    /// it as one history entry.
    fn apply_removal(
        &mut self,
        plan: RemovalPlan,
        keep_region: Option<RegionId>,
    ) -> Result<RemovalOutcome, String> {
        let RemovalPlan {
            mut candidate,
            topology,
            live,
            landings,
            merge,
            vanishing,
            pieces,
            promoted,
            joined,
            mut removed_probes,
            provisional_curve,
        } = plan;
        let choices = merge
            .as_ref()
            .map(|merge| merge.regions.clone())
            .unwrap_or_default();
        let (chosen_survivor, survivor) = resolve_survivor(&choices, keep_region)?;
        let region_remaps = match (chosen_survivor, survivor) {
            (Some(from), Some(to)) if from != to => vec![(from, to)],
            _ => vec![],
        };

        // Faces whose region the removal decides: the merged face takes the
        // survivor; a face that lost its only anchor but absorbed nothing keeps
        // what it had under a fresh anchor.
        let mut forced = BTreeMap::<FaceId, Option<RegionId>>::new();
        let mut kept = BTreeSet::new();
        for (face, landing) in &landings {
            if merge.as_ref().is_some_and(|merge| merge.face == *face) {
                forced.insert(*face, survivor);
                continue;
            }
            if landing.live.is_empty() {
                let mut regions = landing.dead_regions.iter().copied();
                match (regions.next(), regions.next()) {
                    (Some(region), None) => {
                        forced.insert(*face, Some(region));
                        kept.insert(region);
                    }
                    (None, None) if landing.dead_hole => {
                        forced.insert(*face, None);
                    }
                    _ => {}
                }
            }
        }
        candidate.draft.face_assignments = rebuild_face_assignments(&live, &topology, &forced)?;

        if let Some((from, to)) = region_remaps.first().copied() {
            let replacement = candidate
                .draft
                .region(from)
                .copied()
                .ok_or("Selected surviving region no longer exists")?;
            let exterior = candidate
                .draft
                .regions
                .iter_mut()
                .find(|region| region.id == to)
                .ok_or("Exterior region no longer exists")?;
            exterior.material = replacement.material;
            exterior.frame = MaterialFrame::world();
        }
        // Regions leave the document relative to the surviving identity, but
        // their dependents follow the user's choice: keeping the other material
        // across an exterior merge moves its sources and probes onto region 1
        // and drops region 1's own, whose material just got replaced.
        let removed_regions = choices
            .iter()
            .chain(&vanishing)
            .copied()
            .filter(|region| Some(*region) != survivor && !kept.contains(region))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let dropped = choices
            .iter()
            .chain(&vanishing)
            .copied()
            .filter(|region| Some(*region) != chosen_survivor && !kept.contains(region))
            .collect::<BTreeSet<_>>();
        candidate
            .draft
            .regions
            .retain(|region| !removed_regions.contains(&region.id));
        candidate.draft.volume_sources.retain_mut(|source| {
            if let Some((_, to)) = region_remaps
                .iter()
                .find(|(from, _)| source.region == *from)
            {
                source.region = *to;
                true
            } else {
                !dropped.contains(&source.region)
            }
        });
        retarget_point_source(&mut candidate, &region_remaps, &dropped, survivor);
        removed_probes.extend(settle_region_probes(
            &mut candidate.probes,
            &region_remaps,
            &dropped,
        ));
        candidate
            .draft
            .compile(self.revision.wrapping_add(1))
            .map_err(|issue| issue.to_string())?;
        if provisional_curve {
            self.allocate_curve()?;
        }
        self.begin();
        self.document.model = candidate;
        self.changed();
        self.commit();
        Ok(RemovalOutcome {
            pieces,
            promoted,
            joined,
            removed_regions,
            region_remaps,
            removed_probes,
        })
    }

    pub fn set_control(
        &mut self,
        curve: CurveId,
        control: usize,
        point: Point2,
    ) -> Result<(), String> {
        let curve = self
            .document
            .model
            .draft
            .geometry
            .curves
            .iter_mut()
            .find(|candidate| candidate.id == curve)
            .ok_or("Curve no longer exists")?;
        match &mut curve.spline {
            CurveSpline::Closed(spline) => spline.set_control(control, point),
            CurveSpline::Open(spline) => spline.set_control(control, point),
        }
        .map_err(|error| error.to_string())?;
        self.changed();
        Ok(())
    }

    /// Inserts a simple knot without changing the curve and splits the stable
    /// span identity. Boundary probes and face anchors follow both pieces.
    pub fn insert_control(
        &mut self,
        curve_id: CurveId,
        parameter: f64,
    ) -> Result<(usize, Option<CurveSpanId>), String> {
        if !parameter.is_finite() {
            return Err("Insertion parameter must be finite".into());
        }
        let inserted_span = self.allocate_span_id()?;
        let mut candidate = self.document.model.clone();
        let outcome = insert_curve_control(&mut candidate, curve_id, parameter, inserted_span)?;
        self.begin();
        self.document.model = candidate;
        self.changed();
        self.commit();
        Ok(outcome)
    }

    /// Removes one control and its associated knot span. This is a reshaping
    /// edit; attached topology vertices and incompatible adjacent span laws are
    /// protected from an ambiguous merge.
    pub fn remove_control(&mut self, curve_id: CurveId, control: usize) -> Result<(), String> {
        let mut candidate = self.document.model.clone();
        remove_curve_control(&mut candidate, curve_id, control)?;
        self.begin();
        self.document.model = candidate;
        self.changed();
        self.commit();
        Ok(())
    }

    /// Why deleting this control would be refused, or `None` when it would
    /// succeed. The answer runs the real removal on a copy, so it cannot drift
    /// from the command, and it lets the UI disable an action rather than let
    /// the user discover the refusal by clicking.
    pub fn control_removal_error(&self, curve_id: CurveId, control: usize) -> Option<String> {
        let mut candidate = self.document.model.clone();
        remove_curve_control(&mut candidate, curve_id, control).err()
    }

    /// Changes the continuity at one authored curve node. Sharpening is exact;
    /// smoothing uses an exact knot removal when possible and otherwise the
    /// spline's bounded least-squares projection. Topology junctions remain C0.
    pub fn set_curve_continuity(
        &mut self,
        curve_id: CurveId,
        breakpoint: usize,
        target: u8,
    ) -> Result<f64, String> {
        if target > 2 {
            return Err("Cubic continuity must be C0, C1, or C2".into());
        }
        let mut candidate = self.document.model.clone();
        let curve = candidate
            .draft
            .geometry
            .curve(curve_id)
            .ok_or("Curve no longer exists")?;
        if target > 0
            && curve
                .nodes
                .get(breakpoint)
                .is_some_and(|node| node.vertex.is_some())
        {
            return Err("A topology junction must remain a C0 corner".into());
        }
        // A closed curve's control count is the sum of its multiplicities and
        // may not fall below four, so promoting a knot on a small loop has to
        // buy room first. Knot insertion is exact, so nothing moves, and the
        // refinement rides inside this command's single history entry.
        let deficit = promotion_control_deficit(curve, breakpoint, target);
        let breakpoint = if deficit == 0 {
            breakpoint
        } else {
            let parameter = curve
                .spline
                .node_parameter(breakpoint)
                .ok_or("Choose an existing curve knot")?;
            let spans = curve.spans.len() + deficit;
            refine_curve_to_spans(&mut candidate, &mut self.next_span, curve_id, spans)?;
            let refined = candidate
                .draft
                .geometry
                .curve(curve_id)
                .ok_or("Curve no longer exists")?;
            node_index_at_parameter(refined, parameter).ok_or("Refined curve lost its knot")?
        };
        let curve = candidate
            .draft
            .geometry
            .curves
            .iter_mut()
            .find(|curve| curve.id == curve_id)
            .ok_or("Curve no longer exists")?;
        let mut displacement = 0.0;
        match &mut curve.spline {
            CurveSpline::Closed(spline) => {
                let current = spline
                    .continuity(breakpoint)
                    .ok_or("Choose an existing loop knot")?;
                if current == target {
                    return Ok(0.0);
                }
                while spline.continuity(breakpoint).unwrap() > target {
                    spline
                        .increase_multiplicity(breakpoint)
                        .map_err(|error| error.to_string())?;
                }
                while spline.continuity(breakpoint).unwrap() < target {
                    match spline.decrease_multiplicity(breakpoint, 1.0e-10) {
                        Ok(()) => {}
                        Err(SplineError::NotRemovable) => {
                            displacement += spline
                                .decrease_multiplicity_approximate(breakpoint)
                                .map_err(|error| error.to_string())?;
                        }
                        Err(error) => return Err(error.to_string()),
                    }
                }
            }
            CurveSpline::Open(spline) => {
                let current = spline
                    .continuity(breakpoint)
                    .ok_or("Choose an interior curve knot")?;
                if current == target {
                    return Ok(0.0);
                }
                while spline.continuity(breakpoint).unwrap() > target {
                    spline
                        .increase_multiplicity(breakpoint)
                        .map_err(|error| error.to_string())?;
                }
                while spline.continuity(breakpoint).unwrap() < target {
                    match spline.decrease_multiplicity(breakpoint, 1.0e-10) {
                        Ok(()) => {}
                        Err(SplineError::NotRemovable) => {
                            displacement += spline
                                .decrease_multiplicity_approximate(breakpoint)
                                .map_err(|error| error.to_string())?;
                        }
                        Err(error) => return Err(error.to_string()),
                    }
                }
            }
        }
        candidate
            .draft
            .geometry
            .synchronize_vertices()
            .map_err(|issue| issue.to_string())?;
        self.begin();
        self.document.model = candidate;
        self.changed();
        self.commit();
        Ok(displacement)
    }

    /// Refines every selected/unselected transition to C0 without changing the
    /// represented curves. Stable span IDs and attached document data survive.
    pub fn isolate_span_boundaries(
        &mut self,
        selected: &BTreeSet<CurveSpanId>,
    ) -> Result<(), String> {
        if selected.is_empty() {
            return Err("Select curve spans to isolate".into());
        }
        let mut candidate = self.document.model.clone();
        let mut found = false;
        let mut changed = false;
        for curve in &mut candidate.draft.geometry.curves {
            let chosen = curve
                .spans
                .iter()
                .map(|span| selected.contains(&span.id))
                .collect::<Vec<_>>();
            if !chosen.iter().any(|value| *value) {
                continue;
            }
            found = true;
            if chosen.iter().all(|value| *value) {
                continue;
            }
            let count = chosen.len();
            let breakpoints = match &curve.spline {
                CurveSpline::Closed(_) => (0..count)
                    .filter(|node| chosen[*node] != chosen[(*node + count - 1) % count])
                    .collect::<Vec<_>>(),
                CurveSpline::Open(_) => (1..count)
                    .filter(|node| chosen[*node - 1] != chosen[*node])
                    .collect::<Vec<_>>(),
            };
            for breakpoint in breakpoints {
                match &mut curve.spline {
                    CurveSpline::Closed(spline) => {
                        while spline.continuity(breakpoint).unwrap() > 0 {
                            spline
                                .increase_multiplicity(breakpoint)
                                .map_err(|error| error.to_string())?;
                            changed = true;
                        }
                    }
                    CurveSpline::Open(spline) => {
                        while spline.continuity(breakpoint).unwrap() > 0 {
                            spline
                                .increase_multiplicity(breakpoint)
                                .map_err(|error| error.to_string())?;
                            changed = true;
                        }
                    }
                }
            }
        }
        if !found {
            return Err("Selected spans no longer exist".into());
        }
        if !changed {
            return Ok(());
        }
        self.begin();
        self.document.model = candidate;
        self.changed();
        self.commit();
        Ok(())
    }

    /// Makes each selected logical span a straight cubic between its own
    /// endpoints. The span ends are first isolated at C0 and the complete edit
    /// is recorded as one history action.
    pub fn straighten_spans(&mut self, selected: &BTreeSet<CurveSpanId>) -> Result<(), String> {
        if selected.is_empty() {
            return Err("Select curve spans to straighten".into());
        }
        let mut candidate = self.document.model.clone();
        let mut found = false;
        for curve in &mut candidate.draft.geometry.curves {
            let chosen = curve
                .spans
                .iter()
                .enumerate()
                .filter_map(|(index, span)| selected.contains(&span.id).then_some(index))
                .collect::<Vec<_>>();
            if chosen.is_empty() {
                continue;
            }
            found = true;
            let count = curve.spans.len();
            let breakpoints = chosen
                .iter()
                .flat_map(|span| match &curve.spline {
                    CurveSpline::Closed(_) => vec![*span, (*span + 1) % count],
                    CurveSpline::Open(_) => vec![*span, *span + 1],
                })
                .filter(|node| match &curve.spline {
                    CurveSpline::Closed(_) => true,
                    CurveSpline::Open(_) => *node > 0 && *node < count,
                })
                .collect::<BTreeSet<_>>();
            for breakpoint in breakpoints {
                match &mut curve.spline {
                    CurveSpline::Closed(spline) => {
                        while spline.continuity(breakpoint).unwrap() > 0 {
                            spline
                                .increase_multiplicity(breakpoint)
                                .map_err(|e| e.to_string())?;
                        }
                    }
                    CurveSpline::Open(spline) => {
                        while spline.continuity(breakpoint).unwrap() > 0 {
                            spline
                                .increase_multiplicity(breakpoint)
                                .map_err(|e| e.to_string())?;
                        }
                    }
                }
            }
            for span in chosen {
                match &mut curve.spline {
                    CurveSpline::Closed(spline) => {
                        let [start_t, end_t] =
                            spline.span_bounds(span).ok_or("Missing curve span")?;
                        let start = spline.evaluate(start_t);
                        let end = spline.evaluate(end_t);
                        if (end - start).norm() <= f64::EPSILON {
                            return Err("Cannot straighten a span with coincident endpoints".into());
                        }
                        for (offset, control) in spline
                            .span_control_indices(span)
                            .ok_or("Missing curve span")?
                            .into_iter()
                            .enumerate()
                        {
                            spline
                                .set_control(control, start.lerp(end, offset as f64 / 3.0))
                                .map_err(|e| e.to_string())?;
                        }
                    }
                    CurveSpline::Open(spline) => {
                        let [start_t, end_t] =
                            spline.span_bounds(span).ok_or("Missing curve span")?;
                        let start = spline.evaluate(start_t);
                        let end = spline.evaluate(end_t);
                        if (end - start).norm() <= f64::EPSILON {
                            return Err("Cannot straighten a span with coincident endpoints".into());
                        }
                        for (offset, control) in spline
                            .span_control_indices(span)
                            .ok_or("Missing curve span")?
                            .into_iter()
                            .enumerate()
                        {
                            spline
                                .set_control(control, start.lerp(end, offset as f64 / 3.0))
                                .map_err(|e| e.to_string())?;
                        }
                    }
                }
            }
        }
        if !found {
            return Err("Selected spans no longer exist".into());
        }
        self.begin();
        self.document.model = candidate;
        self.changed();
        self.commit();
        Ok(())
    }

    /// Whether every curve touched by the selection contributes exactly one
    /// contiguous run, which is what [`Self::straighten_span_sections`] needs.
    pub fn selection_is_contiguous(&self, selected: &BTreeSet<CurveSpanId>) -> bool {
        if selected.is_empty() {
            return false;
        }
        let mut found = false;
        for curve in &self.document.model.draft.geometry.curves {
            match contiguous_run(curve, selected) {
                Ok(Some(_)) => found = true,
                Ok(None) => {}
                Err(_) => return false,
            }
        }
        found
    }

    /// Collapses one contiguous selected run onto a single straight chord between
    /// its outer breakpoints. Every span inside the run keeps its stable identity,
    /// laws, and attached probes; they simply become collinear pieces of one
    /// segment. Interior knots are raised to C0 first so the straight result is
    /// exact rather than a least-squares fit.
    pub fn straighten_span_sections(
        &mut self,
        selected: &BTreeSet<CurveSpanId>,
    ) -> Result<(), String> {
        if selected.is_empty() {
            return Err("Select curve spans to straighten".into());
        }
        let mut candidate = self.document.model.clone();
        let mut found = false;
        for curve in &mut candidate.draft.geometry.curves {
            let Some(run) = contiguous_run(curve, selected)? else {
                continue;
            };
            found = true;
            let count = curve.spans.len();
            let last_node = match &curve.spline {
                CurveSpline::Closed(_) => (run[run.len() - 1] + 1) % count,
                CurveSpline::Open(_) => run[run.len() - 1] + 1,
            };
            // A junction strictly inside the run would have to move off the
            // curves that share it, so refuse instead of tearing it loose.
            if run
                .iter()
                .skip(1)
                .any(|span| curve.nodes[*span].vertex.is_some())
            {
                return Err(
                    "A junction inside the selection cannot move; straighten each side".into(),
                );
            }
            let mut breakpoints = run.clone();
            breakpoints.push(last_node);
            for node in breakpoints {
                match &mut curve.spline {
                    CurveSpline::Closed(spline) => {
                        while spline.continuity(node).unwrap_or(0) > 0 {
                            spline
                                .increase_multiplicity(node)
                                .map_err(|error| error.to_string())?;
                        }
                    }
                    CurveSpline::Open(spline) => {
                        if node == 0 || node == count {
                            continue;
                        }
                        while spline.continuity(node).unwrap_or(0) > 0 {
                            spline
                                .increase_multiplicity(node)
                                .map_err(|error| error.to_string())?;
                        }
                    }
                }
            }
            let mut controls: Vec<usize> = Vec::new();
            for span in &run {
                let active = match &curve.spline {
                    CurveSpline::Closed(spline) => spline.span_control_indices(*span),
                    CurveSpline::Open(spline) => spline.span_control_indices(*span),
                }
                .ok_or("Missing curve span")?;
                if controls.is_empty() {
                    controls.extend(active);
                } else if controls.last() == Some(&active[0]) {
                    controls.extend_from_slice(&active[1..]);
                } else {
                    return Err("Selected curve section is not one continuous chain".into());
                }
            }
            let control_points = match &curve.spline {
                CurveSpline::Closed(spline) => spline.controls(),
                CurveSpline::Open(spline) => spline.controls(),
            };
            let start = control_points[controls[0]];
            let end = control_points[*controls.last().unwrap()];
            if (end - start).norm() <= f64::EPSILON {
                return Err("Cannot straighten a section with coincident endpoints".into());
            }
            let denominator = (controls.len() - 1) as f64;
            for (offset, control) in controls.into_iter().enumerate() {
                let point = start.lerp(end, offset as f64 / denominator);
                match &mut curve.spline {
                    CurveSpline::Closed(spline) => spline.set_control(control, point),
                    CurveSpline::Open(spline) => spline.set_control(control, point),
                }
                .map_err(|error| error.to_string())?;
            }
        }
        if !found {
            return Err("Selected spans no longer exist".into());
        }
        candidate
            .draft
            .geometry
            .synchronize_vertices()
            .map_err(|issue| issue.to_string())?;
        self.begin();
        self.document.model = candidate;
        self.changed();
        self.commit();
        Ok(())
    }

    /// Applies a topology-native viewport transform without opening or closing
    /// a history transaction. Drag gestures call this repeatedly between
    /// [`Self::begin`] and [`Self::commit`], so one complete drag remains one
    /// undo entry.
    pub fn apply_transform_updates_during_edit(
        &mut self,
        updates: &[TopologyTransformUpdate],
    ) -> Result<(), String> {
        if updates.is_empty() {
            return Err("Transform has no geometry to update".into());
        }
        let mut geometry = self.document.model.draft.geometry.clone();
        for update in updates {
            match *update {
                TopologyTransformUpdate::Control {
                    curve,
                    control,
                    point,
                } => {
                    let curve = geometry
                        .curves
                        .iter_mut()
                        .find(|candidate| candidate.id == curve)
                        .ok_or("Curve no longer exists")?;
                    match &mut curve.spline {
                        CurveSpline::Closed(spline) => spline.set_control(control, point),
                        CurveSpline::Open(spline) => spline.set_control(control, point),
                    }
                    .map_err(|error| error.to_string())?;
                }
                TopologyTransformUpdate::Vertex { vertex, point } => {
                    if !point.finite() {
                        return Err("Junction position must be finite".into());
                    }
                    let domain = geometry.domain;
                    let vertex = geometry
                        .vertices
                        .iter_mut()
                        .find(|candidate| candidate.id == vertex)
                        .ok_or("Junction no longer exists")?;
                    vertex.location = match vertex.location {
                        TopologyVertexLocation::Free(_) => TopologyVertexLocation::Free(point),
                        TopologyVertexLocation::Interior(_) => {
                            TopologyVertexLocation::Interior(point)
                        }
                        TopologyVertexLocation::Outer { side, .. } => {
                            TopologyVertexLocation::Outer {
                                side,
                                fraction: outer_fraction(domain, side, point),
                            }
                        }
                    };
                }
            }
        }
        geometry
            .synchronize_vertices()
            .map_err(|issue| issue.to_string())?;
        if geometry == self.document.model.draft.geometry {
            return Ok(());
        }
        self.document.model.draft.geometry = geometry;
        self.changed();
        Ok(())
    }

    pub fn apply_transform_updates(
        &mut self,
        updates: &[TopologyTransformUpdate],
    ) -> Result<(), String> {
        self.begin();
        if let Err(error) = self.apply_transform_updates_during_edit(updates) {
            self.cancel();
            return Err(error);
        }
        self.commit();
        Ok(())
    }

    pub fn set_span_behavior(
        &mut self,
        spans: &BTreeSet<CurveSpanId>,
        behavior: SpanBehavior,
    ) -> Result<(), String> {
        if !behavior.valid() {
            return Err("Boundary behavior is invalid".into());
        }
        self.begin();
        let mut changed = false;
        for curve in &mut self.document.model.draft.geometry.curves {
            for span in &mut curve.spans {
                if spans.contains(&span.id) && span.behavior != behavior {
                    span.behavior = behavior;
                    changed = true;
                }
            }
        }
        if !changed {
            self.before = None;
            return Err("Select existing curve spans".into());
        }
        self.changed();
        self.commit();
        Ok(())
    }

    pub fn set_span_face_condition(
        &mut self,
        spans: &BTreeSet<CurveSpanId>,
        side: CurveTraceSide,
        condition: FaceBoundaryCondition,
    ) -> Result<(), String> {
        if !condition.valid() {
            return Err("Boundary condition is invalid".into());
        }
        self.begin();
        let mut changed = false;
        for curve in &mut self.document.model.draft.geometry.curves {
            for span in &mut curve.spans {
                if !spans.contains(&span.id) {
                    continue;
                }
                let (mut left, mut right, mut coupling) = match span.behavior {
                    SpanBehavior::Transmitting => (
                        FaceBoundaryCondition::Reflecting,
                        FaceBoundaryCondition::Reflecting,
                        InternalBoundaryCoupling::Independent,
                    ),
                    SpanBehavior::Separated {
                        left,
                        right,
                        coupling,
                    } => (left, right, coupling),
                };
                match side {
                    CurveTraceSide::Left => left = condition,
                    CurveTraceSide::Right => right = condition,
                }
                if condition != FaceBoundaryCondition::Reflecting {
                    coupling = InternalBoundaryCoupling::Independent;
                }
                let behavior = SpanBehavior::Separated {
                    left,
                    right,
                    coupling,
                };
                if span.behavior != behavior {
                    span.behavior = behavior;
                    changed = true;
                }
            }
        }
        if !changed {
            self.before = None;
            return Err("Select existing curve spans".into());
        }
        self.changed();
        self.commit();
        Ok(())
    }

    pub fn set_span_coupling(
        &mut self,
        spans: &BTreeSet<CurveSpanId>,
        coupling: InternalBoundaryCoupling,
    ) -> Result<(), String> {
        if !coupling.valid() {
            return Err("Boundary coupling is invalid".into());
        }
        self.begin();
        let mut changed = false;
        for curve in &mut self.document.model.draft.geometry.curves {
            for span in &mut curve.spans {
                if !spans.contains(&span.id) {
                    continue;
                }
                let (left, right) = match span.behavior {
                    SpanBehavior::Transmitting => (
                        FaceBoundaryCondition::Reflecting,
                        FaceBoundaryCondition::Reflecting,
                    ),
                    SpanBehavior::Separated { left, right, .. } => (left, right),
                };
                let behavior = SpanBehavior::Separated {
                    left: if matches!(coupling, InternalBoundaryCoupling::ThinGap { .. }) {
                        FaceBoundaryCondition::Reflecting
                    } else {
                        left
                    },
                    right: if matches!(coupling, InternalBoundaryCoupling::ThinGap { .. }) {
                        FaceBoundaryCondition::Reflecting
                    } else {
                        right
                    },
                    coupling,
                };
                if span.behavior != behavior {
                    span.behavior = behavior;
                    changed = true;
                }
            }
        }
        if !changed {
            self.before = None;
            return Err("Select existing curve spans".into());
        }
        self.changed();
        self.commit();
        Ok(())
    }

    pub fn set_outer_condition(
        &mut self,
        sides: &BTreeSet<OuterSide>,
        condition: OuterBoundaryCondition,
    ) -> Result<(), String> {
        if !condition.valid() || sides.is_empty() {
            return Err("Outer boundary condition is invalid".into());
        }
        self.begin();
        let mut changed = false;
        for side in sides {
            let slot = &mut self.document.model.draft.outer_boundaries.sides[side.index()];
            if *slot != condition {
                *slot = condition;
                changed = true;
            }
        }
        if !changed {
            self.before = None;
            return Ok(());
        }
        self.changed();
        self.commit();
        Ok(())
    }

    fn materialize_attachment(
        &mut self,
        scene: &mut TopologyScene,
        probes: &mut [TopologyProbeDefinition],
        target: TopologyAttachment,
        splits: &mut Vec<TopologySpanSplit>,
    ) -> Result<(TopologyVertexId, Point2), String> {
        match target {
            TopologyAttachment::Junction { vertex, .. } => {
                if !scene
                    .geometry
                    .vertices
                    .iter()
                    .any(|candidate| candidate.id == vertex)
                {
                    let restored = self
                        .document
                        .model
                        .accepted
                        .geometry
                        .vertices
                        .iter()
                        .find(|candidate| candidate.id == vertex)
                        .copied()
                        .ok_or("Junction no longer exists")?;
                    scene.geometry.vertices.push(restored);
                    scene.geometry.vertices.sort_by_key(|vertex| vertex.id);
                }
                let point = scene
                    .geometry
                    .vertices
                    .iter()
                    .find(|candidate| candidate.id == vertex)
                    .and_then(|vertex| vertex.point(scene.geometry.domain))
                    .ok_or("Junction no longer exists")?;
                Ok((vertex, point))
            }
            TopologyAttachment::Boundary(FaceAnchor::Outer { side, fraction }) => {
                let point = outer_attachment_point(scene.geometry.domain, side, fraction)?;
                if let Some(vertex) = scene.geometry.vertices.iter().find(|vertex| {
                    matches!(
                        vertex.location,
                        TopologyVertexLocation::Outer {
                            side: candidate_side,
                            fraction: candidate_fraction,
                        } if candidate_side == side
                            && (candidate_fraction - fraction).abs() <= 1.0e-12
                    )
                }) {
                    return Ok((vertex.id, point));
                }
                let vertex = self.allocate_vertex()?;
                scene.geometry.vertices.push(TopologyVertex {
                    id: vertex,
                    location: TopologyVertexLocation::Outer { side, fraction },
                });
                remap_outer_anchor_dependencies(scene, side, fraction);
                Ok((vertex, point))
            }
            TopologyAttachment::LooseEnd { .. } => {
                Err("Loose ends are welded, not attached to a vertex".into())
            }
            TopologyAttachment::Breakpoint { curve, node, .. } => {
                let curve_index = scene
                    .geometry
                    .curves
                    .iter()
                    .position(|candidate| candidate.id == curve)
                    .ok_or("Attachment curve no longer exists")?;
                let target = &scene.geometry.curves[curve_index];
                breakpoint_span(target, node)?;
                if let Some(vertex) = target.nodes[node].vertex {
                    let point = scene
                        .geometry
                        .vertices
                        .iter()
                        .find(|candidate| candidate.id == vertex)
                        .and_then(|vertex| vertex.point(scene.geometry.domain))
                        .ok_or("Junction no longer exists")?;
                    return Ok((vertex, point));
                }
                self.attach_vertex_to_node(scene, curve_index, node)
            }
            TopologyAttachment::Boundary(FaceAnchor::Curve {
                curve, parameter, ..
            }) => {
                let curve_index = scene
                    .geometry
                    .curves
                    .iter()
                    .position(|candidate| candidate.id == curve)
                    .ok_or("Attachment curve no longer exists")?;
                if let Some((vertex, point)) = existing_curve_vertex(
                    &scene.geometry.curves[curve_index],
                    &scene.geometry,
                    parameter,
                ) {
                    return Ok((vertex, point));
                }
                // A breakpoint that owns no vertex yet — an earlier weld's seam,
                // say — becomes the junction itself: raised to a corner, no span
                // inserted. A free tip is a loose end and welds instead.
                if let Some(node) =
                    vertexless_node_at(&scene.geometry.curves[curve_index], parameter)
                {
                    let target = &scene.geometry.curves[curve_index];
                    if target.spline.is_open() && (node == 0 || node + 1 == target.nodes.len()) {
                        return Err("That is a loose end; weld to it instead".into());
                    }
                    return self.attach_vertex_to_node(scene, curve_index, node);
                }
                let point = evaluate_curve(&scene.geometry.curves[curve_index], parameter);
                let span_index = containing_span(&scene.geometry.curves[curve_index], parameter)
                    .ok_or("Attachment point is no longer inside a curve span")?;
                let original = scene.geometry.curves[curve_index].spans[span_index].id;
                let vertex = self.allocate_vertex()?;
                let inserted = self.allocate_span_id()?;
                let insertion = scene.geometry.curves[curve_index]
                    .insert_topology_vertex(parameter, vertex, inserted)
                    .map_err(|error| error.to_string())?;
                if !insertion.inserted_span {
                    return Err("Attachment point must be inside a curve span".into());
                }
                scene.geometry.vertices.push(TopologyVertex {
                    id: vertex,
                    location: TopologyVertexLocation::Interior(point),
                });
                remap_split_dependencies(scene, probes, curve, original, inserted, parameter);
                splits.push(TopologySpanSplit {
                    curve,
                    original,
                    inserted,
                    parameter,
                });
                Ok((vertex, point))
            }
        }
    }

    /// Makes an interior breakpoint that owns no vertex into a junction: raised
    /// to a corner without moving the curve, then bound to a fresh vertex.
    fn attach_vertex_to_node(
        &mut self,
        scene: &mut TopologyScene,
        curve_index: usize,
        node: usize,
    ) -> Result<(TopologyVertexId, Point2), String> {
        let target = &mut scene.geometry.curves[curve_index];
        let point = target
            .spline
            .node_point(node)
            .ok_or("Attachment breakpoint no longer exists")?;
        let vertex = self.allocate_vertex()?;
        match &mut target.spline {
            CurveSpline::Open(spline) => {
                while spline
                    .continuity(node)
                    .is_some_and(|continuity| continuity > 0)
                {
                    spline
                        .increase_multiplicity(node)
                        .map_err(|error| error.to_string())?;
                }
            }
            CurveSpline::Closed(spline) => {
                while spline
                    .continuity(node)
                    .is_some_and(|continuity| continuity > 0)
                {
                    spline
                        .increase_multiplicity(node)
                        .map_err(|error| error.to_string())?;
                }
            }
        }
        target.nodes[node].vertex = Some(vertex);
        scene.geometry.vertices.push(TopologyVertex {
            id: vertex,
            location: TopologyVertexLocation::Interior(point),
        });
        Ok((vertex, point))
    }

    fn assign_new_faces(
        &mut self,
        scene: &mut TopologyScene,
        topology: &TopologySnapshot,
        new_curve: CurveId,
        source_region: Option<RegionId>,
        separator_material: Option<MaterialId>,
    ) -> Result<(), String> {
        let mut assigned = BTreeSet::new();
        for assignment in &scene.face_assignments {
            assigned.insert(
                assignment
                    .anchor
                    .resolve(topology)
                    .map_err(|issue| issue.to_string())?,
            );
        }
        let new_faces = topology
            .faces
            .iter()
            .map(|face| face.id)
            .filter(|face| !assigned.contains(face))
            .collect::<Vec<_>>();
        if separator_material.is_some() && new_faces.len() != 1 {
            return Err("Subdomain separator must create exactly one new face".into());
        }
        let source = source_region
            .map(|region| {
                scene
                    .region(region)
                    .copied()
                    .ok_or("Source region no longer exists")
            })
            .transpose()?;
        for face in new_faces {
            let anchor = curve_face_anchor(topology, new_curve, face)
                .ok_or("New face has no stable anchor on the drawn curve")?;
            let region = match source {
                Some(source) => {
                    let id = self.allocate_region()?;
                    scene.regions.push(Region {
                        id,
                        material: separator_material.unwrap_or(source.material),
                        // A daughter that keeps the old material keeps the old
                        // frame; a genuinely new subdomain starts centred on the
                        // face it owns instead of at the world origin.
                        frame: if separator_material.is_some() {
                            face_frame(topology, Some(face))
                        } else {
                            source.frame
                        },
                    });
                    Some(id)
                }
                None => None,
            };
            scene
                .face_assignments
                .push(AuthoredFaceAssignment { anchor, region });
        }
        Ok(())
    }

    fn require_material(&self, material: MaterialId) -> Result<(), String> {
        self.document
            .model
            .draft
            .material(material)
            .map(|_| ())
            .ok_or("Choose an existing material".into())
    }

    fn allocate_curve(&mut self) -> Result<CurveId, String> {
        let id = CurveId(self.next_curve);
        self.next_curve = self
            .next_curve
            .checked_add(1)
            .ok_or("Curve IDs exhausted")?;
        Ok(id)
    }

    fn allocate_spans(
        &mut self,
        count: usize,
        behavior: SpanBehavior,
    ) -> Result<Vec<CurveSpan>, String> {
        (0..count)
            .map(|_| {
                let id = CurveSpanId(self.next_span);
                self.next_span = self.next_span.checked_add(1).ok_or("Span IDs exhausted")?;
                Ok(CurveSpan { id, behavior })
            })
            .collect()
    }

    fn allocate_span_id(&mut self) -> Result<CurveSpanId, String> {
        let id = CurveSpanId(self.next_span);
        self.next_span = self.next_span.checked_add(1).ok_or("Span IDs exhausted")?;
        Ok(id)
    }

    fn allocate_region(&mut self) -> Result<RegionId, String> {
        let id = RegionId(self.next_region);
        self.next_region = self
            .next_region
            .checked_add(1)
            .ok_or("Region IDs exhausted")?;
        Ok(id)
    }

    fn allocate_vertex(&mut self) -> Result<TopologyVertexId, String> {
        let id = TopologyVertexId(self.next_vertex);
        self.next_vertex = self
            .next_vertex
            .checked_add(1)
            .ok_or("Topology vertex IDs exhausted")?;
        Ok(id)
    }
}

fn probe_definition_valid(probe: &TopologyProbeDefinition, scene: &TopologyScene) -> bool {
    if probe.id.0 == 0
        || probe.name.trim().is_empty()
        || probe.name.len() > 64
        || scene
            .regions
            .iter()
            .all(|region| region.id != BACKGROUND_REGION)
    {
        return false;
    }
    match &probe.target {
        TopologyProbeTarget::Point(point) => point.finite(),
        TopologyProbeTarget::Segment { start, end, .. } => {
            start.finite() && end.finite() && (*end - *start).norm() >= 1.0e-6
        }
        TopologyProbeTarget::Boundary(target) => scene
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == target.curve)
            .is_some_and(|curve| {
                !target.spans.is_empty()
                    && target
                        .spans
                        .iter()
                        .all(|span| curve.spans.iter().any(|candidate| candidate.id == *span))
            }),
        TopologyProbeTarget::AreaDisk { center, radius } => {
            center.finite()
                && radius.is_finite()
                && *radius > 0.0
                && (std::f64::consts::PI * radius * radius).is_finite()
        }
        TopologyProbeTarget::AreaRegion(region) => scene.region(*region).is_some(),
    }
}

fn next_id(ids: impl Iterator<Item = u64>) -> Result<u64, String> {
    ids.max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or("Document IDs exhausted".into())
}

fn attachment_face(
    target: TopologyAttachment,
    compiled: &CompiledTopologyScene,
) -> Result<FaceId, String> {
    match target {
        TopologyAttachment::Boundary(anchor) => anchor
            .resolve(&compiled.topology)
            .map_err(|issue| issue.to_string()),
        TopologyAttachment::Junction { vertex, face } => {
            let incident = compiled.topology.vertices.iter().any(|candidate| {
                candidate.authored == Some(vertex)
                    && candidate.traces.iter().any(|trace| trace.face == face)
            });
            if incident
                && compiled
                    .assignments
                    .iter()
                    .any(|assignment| assignment.face == face)
            {
                Ok(face)
            } else {
                Err("Selected junction sector no longer exists".into())
            }
        }
        TopologyAttachment::Breakpoint { curve, node, side } => {
            let target = compiled
                .geometry
                .curve(curve)
                .ok_or("Attachment curve no longer exists")?;
            let span = breakpoint_span(target, node)?;
            let [a, b] = target
                .spline
                .span_bounds(span)
                .ok_or("Attachment breakpoint no longer exists")?;
            // The node owns no vertex, so the face beside the span starting at
            // it is the face beside the node.
            FaceAnchor::Curve {
                curve,
                span: target.spans[span].id,
                side,
                parameter: (a + b) * 0.5,
            }
            .resolve(&compiled.topology)
            .map_err(|issue| issue.to_string())
        }
        TopologyAttachment::LooseEnd { curve, endpoint } => {
            let target = compiled
                .geometry
                .curve(curve)
                .ok_or("Attachment curve no longer exists")?;
            let node = loose_end_node(target, endpoint)?;
            // A free tip is a slit inside one face: the end span sees that face
            // on both sides, so either side of its compiled edge names it.
            let span = if node == 0 {
                target.spans[0].id
            } else {
                target.spans[target.spans.len() - 1].id
            };
            compiled
                .topology
                .edges
                .iter()
                .find(|edge| edge.source == CompiledEdgeSource::Curve(span))
                .map(|edge| edge.left)
                .or_else(|| {
                    let point = target.spline.node_point(node)?;
                    compiled.topology.face_at(point)
                })
                .ok_or("Loose end no longer lies in a face".into())
        }
    }
}

/// The span that starts at an interior breakpoint, or why the node is not one.
/// Every node of a closed curve qualifies; an open curve's ends do not.
fn breakpoint_span(curve: &TopologyCurve, node: usize) -> Result<usize, String> {
    if node >= curve.nodes.len()
        || (curve.spline.is_open() && (node == 0 || node + 1 == curve.nodes.len()))
    {
        return Err("Choose an interior breakpoint".into());
    }
    Ok(node)
}

/// The node index of a free end of an open curve, or why it is not one.
fn loose_end_node(curve: &TopologyCurve, endpoint: usize) -> Result<usize, String> {
    if !curve.spline.is_open() || !matches!(endpoint, 0 | 1) {
        return Err("Choose the start or end of an open curve".into());
    }
    let node = if endpoint == 0 {
        0
    } else {
        curve.nodes.len() - 1
    };
    if curve.nodes[node].vertex.is_some() {
        return Err("Endpoint is already attached".into());
    }
    Ok(node)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CurveEnd {
    Start,
    End,
}

fn curve_end(curve: &TopologyCurve, node: usize) -> Result<CurveEnd, String> {
    if node == 0 {
        Ok(CurveEnd::Start)
    } else if node + 1 == curve.nodes.len() {
        Ok(CurveEnd::End)
    } else {
        Err("Weld a curve at one of its ends".into())
    }
}

fn open_period(curve: &TopologyCurve) -> Result<f64, String> {
    match &curve.spline {
        CurveSpline::Open(spline) => Ok(spline.period()),
        CurveSpline::Closed(_) => Err("Only open curves can be welded".into()),
    }
}

/// Fuses two open curves whose named ends are both free into one curve. The
/// stationary curve keeps its identity and direction; the absorbed curve is
/// reversed when the ends demand it and disappears. Span ids survive, every
/// face anchor and boundary probe on either curve is carried onto the result,
/// and the seam is an ordinary C0 breakpoint with no vertex. Nothing is
/// compiled here: callers validate the arrangement afterwards.
fn join_curves(
    model: &mut TopologyDocumentModel,
    stationary: (CurveId, usize),
    absorbed: (CurveId, usize),
) -> Result<JoinRecord, String> {
    let (s_id, s_node) = stationary;
    let (a_id, a_node) = absorbed;
    if s_id == a_id {
        return Err("Choose two different curves to weld".into());
    }
    let geometry = &model.draft.geometry;
    let s = geometry
        .curve(s_id)
        .ok_or("Curve no longer exists")?
        .clone();
    let mut a = geometry
        .curve(a_id)
        .ok_or("Curve no longer exists")?
        .clone();
    let s_end = curve_end(&s, s_node)?;
    let a_end = curve_end(&a, a_node)?;
    if s.nodes[s_node].vertex.is_some() || a.nodes[a_node].vertex.is_some() {
        return Err("Both curve ends must be free to weld them".into());
    }
    open_period(&s)?;
    open_period(&a)?;
    // Pin the absorbed end onto the stationary tip so the seam control `join`
    // averages is exact.
    let tip = s
        .spline
        .node_point(s_node)
        .ok_or("Curve end no longer exists")?;
    a.spline
        .set_node_point(a_node, tip)
        .map_err(|error| error.to_string())?;
    // The joined spline runs head -> tail; the stationary curve is never
    // reversed, so the absorbed one turns around when the ends demand it.
    let (head, tail, a_reversed) = match (s_end, a_end) {
        (CurveEnd::End, CurveEnd::Start) => (s.clone(), a, false),
        (CurveEnd::End, CurveEnd::End) => (
            s.clone(),
            a.reversed().map_err(|issue| issue.to_string())?,
            true,
        ),
        (CurveEnd::Start, CurveEnd::End) => (a, s.clone(), false),
        (CurveEnd::Start, CurveEnd::Start) => (
            a.reversed().map_err(|issue| issue.to_string())?,
            s.clone(),
            true,
        ),
    };
    let head_is_s = s_end == CurveEnd::End;
    let (CurveSpline::Open(head_spline), CurveSpline::Open(tail_spline)) =
        (&head.spline, &tail.spline)
    else {
        return Err("Only open curves can be welded".into());
    };
    let head_period = head_spline.period();
    let tail_period = tail_spline.period();
    // (reversed, offset, period) for every parameter that lived on S or on A.
    let (s_map, a_map) = if head_is_s {
        (
            (false, 0.0, head_period),
            (a_reversed, head_period, tail_period),
        )
    } else {
        (
            (false, head_period, tail_period),
            (a_reversed, 0.0, head_period),
        )
    };
    let spline = head_spline
        .clone()
        .join(tail_spline.clone(), 1.0e-9)
        .map_err(|error| match error {
            SplineError::ControlCount => {
                "The welded curve would have too many control points".to_owned()
            }
            other => other.to_string(),
        })?;
    let seam_node = head.spans.len();
    let seam_control = spline
        .span_control_indices(seam_node - 1)
        .ok_or("Welded curve lost its seam")?[3];
    let spans = head
        .spans
        .iter()
        .chain(&tail.spans)
        .copied()
        .collect::<Vec<_>>();
    // The two seam nodes collapse into one, which is free by construction.
    let nodes = head.nodes[..seam_node]
        .iter()
        .chain(&tail.nodes)
        .copied()
        .collect::<Vec<_>>();
    let mut joined = TopologyCurve::new(s_id, CurveSpline::Open(spline), spans)
        .map_err(|issue| issue.to_string())?;
    if nodes.len() != joined.nodes.len() {
        return Err("Welded curve has inconsistent nodes".into());
    }
    joined.nodes = nodes;
    let absorbed_far_node = if head_is_s { joined.nodes.len() - 1 } else { 0 };

    let remap = |curve: CurveId, parameter: f64| -> Option<(f64, bool)> {
        let (reversed, offset, period) = if curve == s_id {
            s_map
        } else if curve == a_id {
            a_map
        } else {
            return None;
        };
        let local = if reversed {
            period - parameter
        } else {
            parameter
        };
        Some((offset + local, reversed))
    };
    for assignment in &mut model.draft.face_assignments {
        let FaceAnchor::Curve {
            curve,
            side,
            parameter,
            ..
        } = &mut assignment.anchor
        else {
            continue;
        };
        let Some((mapped, reversed)) = remap(*curve, *parameter) else {
            continue;
        };
        if reversed {
            *side = side.opposite();
        }
        *parameter = mapped;
        *curve = s_id;
    }
    for probe in &mut model.probes {
        let TopologyProbeTarget::Boundary(target) = &mut probe.target else {
            continue;
        };
        if target.curve == a_id && a_reversed {
            target.spans.reverse();
            target.reversed = !target.reversed;
            target.side = target.side.opposite();
        }
        if target.curve == a_id || target.curve == s_id {
            target.curve = s_id;
        }
    }
    let curves = &mut model.draft.geometry.curves;
    let position = curves
        .iter()
        .position(|curve| curve.id == s_id)
        .ok_or("Curve no longer exists")?;
    curves[position] = joined;
    curves.retain(|curve| curve.id != a_id);
    Ok(JoinRecord {
        survivor: s_id,
        absorbed: a_id,
        seam_node,
        seam_control,
        absorbed_far_node,
    })
}

/// Closes an open curve whose two free ends meet into a loop. Node 0 becomes
/// the seam, spans and every parameter on the curve are unchanged, so no
/// dependency moves. Faces are not assigned here: inside a removal no new face
/// can arise, and `weld_endpoint` runs the face step itself.
/// Deletes one control and its knot span, merging the spans on either side.
/// Attached junctions and mismatched span laws block the merge, and the spline
/// keeps its four-control floor.
fn remove_curve_control(
    model: &mut TopologyDocumentModel,
    curve_id: CurveId,
    control: usize,
) -> Result<(), String> {
    let curve = model
        .draft
        .geometry
        .curves
        .iter_mut()
        .find(|curve| curve.id == curve_id)
        .ok_or("Curve no longer exists")?;
    let target = control_removal_target(&curve.spline, control)?;
    let (removed_span_index, retained_span_index, removed_node) =
        control_removal_indices(target, curve.spans.len())?;
    if curve
        .nodes
        .get(removed_node)
        .is_some_and(|node| node.vertex.is_some())
    {
        return Err("Detach this topology junction before deleting its control".into());
    }
    let removed = curve.spans[removed_span_index];
    let retained = curve.spans[retained_span_index];
    if removed.behavior != retained.behavior {
        return Err("Adjacent spans have different boundary settings".into());
    }
    let parameter_shift = leading_interval_shift(&curve.spline, target);
    remove_attributed_control(&mut curve.spline, target)?;
    curve.spans.remove(removed_span_index);
    curve.nodes.remove(removed_node);
    if curve.spans.len() + usize::from(curve.spline.is_open()) != curve.nodes.len() {
        return Err("Removed curve topology is inconsistent".into());
    }
    model
        .draft
        .geometry
        .synchronize_vertices()
        .map_err(|issue| issue.to_string())?;
    remap_removed_span_dependencies(
        &mut model.draft,
        &mut model.probes,
        curve_id,
        removed.id,
        retained.id,
        parameter_shift,
    );
    Ok(())
}

/// Splits one span by exact knot insertion. The curve keeps its shape exactly;
/// only its control and span structure is refined. Anchors and probes follow
/// the split, so this is safe to run inside another command's candidate.
fn insert_curve_control(
    model: &mut TopologyDocumentModel,
    curve_id: CurveId,
    parameter: f64,
    inserted_span: CurveSpanId,
) -> Result<(usize, Option<CurveSpanId>), String> {
    let curve = model
        .draft
        .geometry
        .curves
        .iter_mut()
        .find(|curve| curve.id == curve_id)
        .ok_or("Curve no longer exists")?;
    let old_span_count = curve.spans.len();
    let old_span_index = (0..old_span_count)
        .find(|index| {
            curve
                .spline
                .span_bounds(*index)
                .is_some_and(|[start, end]| parameter >= start && parameter <= end)
        })
        .ok_or("Insertion parameter is outside the curve")?;
    let original = curve.spans[old_span_index];
    let insertion = match &mut curve.spline {
        CurveSpline::Closed(spline) => spline.insert(parameter),
        CurveSpline::Open(spline) => spline.insert(parameter),
    }
    .map_err(control_count_message)?;
    let control = match insertion {
        Insertion::Existing(control) => return Ok((control, None)),
        Insertion::Inserted(control) => control,
    };
    let node = match &curve.spline {
        CurveSpline::Closed(spline) => spline
            .knots()
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| {
                (**left - parameter)
                    .abs()
                    .total_cmp(&(**right - parameter).abs())
            })
            .map(|(index, _)| index),
        CurveSpline::Open(spline) => (0..=spline.intervals().len())
            .filter_map(|index| spline.breakpoint(index).map(|value| (index, value)))
            .min_by(|(_, left), (_, right)| {
                (*left - parameter)
                    .abs()
                    .total_cmp(&(*right - parameter).abs())
            })
            .map(|(index, _)| index),
    }
    .ok_or("Inserted knot could not be located")?;
    curve.nodes.insert(node, CurveNode::default());
    curve.spans.insert(
        node,
        CurveSpan {
            id: inserted_span,
            behavior: original.behavior,
        },
    );
    if curve.spans.len() != old_span_count + 1 || curve.nodes.len() != curve.spline.node_count() {
        return Err("Inserted curve topology is inconsistent".into());
    }
    remap_split_dependencies(
        &mut model.draft,
        &mut model.probes,
        curve_id,
        original.id,
        inserted_span,
        parameter,
    );
    Ok((control, Some(inserted_span)))
}

/// A spline carries a hard ceiling on its control points, which no amount of
/// refinement can buy past. Say so instead of leaking the error name.
fn control_count_message(error: SplineError) -> String {
    match error {
        SplineError::ControlCount => {
            "This curve already carries as many control points as a spline allows".to_owned()
        }
        other => other.to_string(),
    }
}

/// Refines a curve by exact knot insertion until it carries at least `spans`
/// spans, splitting the widest span each time. Knot insertion does not move the
/// curve, so the refinement is invisible; it only buys the control points that
/// an operation needs to stay inside the spline's four-control floor.
fn refine_curve_to_spans(
    model: &mut TopologyDocumentModel,
    next_span: &mut u64,
    curve_id: CurveId,
    spans: usize,
) -> Result<(), String> {
    loop {
        let curve = model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == curve_id)
            .ok_or("Curve no longer exists")?;
        if curve.spans.len() >= spans {
            return Ok(());
        }
        let parameter = widest_span_midpoint(curve).ok_or("Curve has no span to refine")?;
        let id = CurveSpanId(*next_span);
        *next_span = next_span.checked_add(1).ok_or("Span IDs exhausted")?;
        let (_, inserted) = insert_curve_control(model, curve_id, parameter, id)?;
        if inserted.is_none() {
            return Err("Curve could not be refined".into());
        }
    }
}

fn widest_span_midpoint(curve: &TopologyCurve) -> Option<f64> {
    (0..curve.spans.len())
        .filter_map(|index| {
            let [start, end] = curve.spline.span_bounds(index)?;
            Some((end - start, (start + end) * 0.5))
        })
        .max_by(|(left, _), (right, _)| left.total_cmp(right))
        .map(|(_, middle)| middle)
}

/// How many extra control points a promotion needs before the periodic spline's
/// four-control floor allows it. An open spline has no floor, and sharpening a
/// knot only ever adds controls.
fn promotion_control_deficit(curve: &TopologyCurve, breakpoint: usize, target: u8) -> usize {
    let CurveSpline::Closed(spline) = &curve.spline else {
        return 0;
    };
    let Some(current) = spline.continuity(breakpoint) else {
        return 0;
    };
    if current >= target {
        return 0;
    }
    let spent = usize::from(current.abs_diff(target));
    let remaining = spline.controls().len().saturating_sub(spent);
    4usize.saturating_sub(remaining)
}

/// Locates a node by its parameter. Refinement preserves every existing knot,
/// so a breakpoint index taken before a refinement is recovered through this.
fn node_index_at_parameter(curve: &TopologyCurve, parameter: f64) -> Option<usize> {
    (0..curve.nodes.len()).find(|index| {
        curve
            .spline
            .node_parameter(*index)
            .is_some_and(|node| (node - parameter).abs() <= 1.0e-9)
    })
}

fn close_curve_geometry(
    model: &mut TopologyDocumentModel,
    curve_id: CurveId,
) -> Result<(), String> {
    let curve = model
        .draft
        .geometry
        .curves
        .iter_mut()
        .find(|curve| curve.id == curve_id)
        .ok_or("Curve no longer exists")?;
    let CurveSpline::Open(spline) = &curve.spline else {
        return Err("Curve is already closed".into());
    };
    let last = curve.nodes.len() - 1;
    if curve.nodes[0].vertex.is_some() || curve.nodes[last].vertex.is_some() {
        return Err("Both curve ends must be free to close the loop".into());
    }
    if curve.spans.len() < 2 {
        return Err("Add a control point before closing a single-span curve".into());
    }
    let mut spline = spline.clone();
    let start = spline.evaluate(0.0);
    spline
        .set_breakpoint_point(last, start)
        .map_err(|error| error.to_string())?;
    let closed = spline.close(1.0e-9).map_err(|error| error.to_string())?;
    curve.spline = CurveSpline::Closed(closed);
    curve.nodes.pop();
    Ok(())
}

/// Demotes one open curve to a baffle when it holds a transmitting span and a
/// free tip, which the arrangement compiler would otherwise reject.
fn promote_curve_if_freed(geometry: &mut TopologyGeometry, curve_id: CurveId) -> bool {
    let Some(curve) = geometry
        .curves
        .iter_mut()
        .find(|curve| curve.id == curve_id)
    else {
        return false;
    };
    if !curve.spline.is_open() || !free_transmitting_end(curve) {
        return false;
    }
    for span in &mut curve.spans {
        span.behavior = SpanBehavior::REFLECTING;
    }
    true
}

fn open_spline_midpoint(spline: &OpenCubicSpline) -> Point2 {
    spline.evaluate(spline.period() * 0.5)
}

fn evaluate_curve(curve: &TopologyCurve, parameter: f64) -> Point2 {
    match &curve.spline {
        CurveSpline::Closed(spline) => spline.evaluate(parameter),
        CurveSpline::Open(spline) => spline.evaluate(parameter),
    }
}

fn containing_span(curve: &TopologyCurve, parameter: f64) -> Option<usize> {
    curve.spans.iter().enumerate().find_map(|(index, _)| {
        let [a, b] = curve.spline.span_bounds(index)?;
        let tolerance = (b - a).abs().max(1.0) * 1.0e-10;
        (parameter > a.min(b) + tolerance && parameter < a.max(b) - tolerance).then_some(index)
    })
}

/// The node sitting at `parameter` that owns no vertex yet, if any.
fn vertexless_node_at(curve: &TopologyCurve, parameter: f64) -> Option<usize> {
    let tolerance = curve_period(&curve.spline).max(1.0) * 1.0e-10;
    curve.nodes.iter().enumerate().find_map(|(index, node)| {
        let node_parameter = curve.spline.node_parameter(index)?;
        (node.vertex.is_none() && parameter_distance(curve, node_parameter, parameter) <= tolerance)
            .then_some(index)
    })
}

/// Distance between two parameters on one curve. A closed curve's seam is node
/// zero, which the last span reaches again at the period, so the comparison has
/// to wrap or every lookup at the seam misses.
fn parameter_distance(curve: &TopologyCurve, left: f64, right: f64) -> f64 {
    let period = curve_period(&curve.spline);
    let distance = (left - right).abs();
    if curve.spline.is_open() {
        distance
    } else {
        distance.min(period - distance)
    }
}

fn existing_curve_vertex(
    curve: &TopologyCurve,
    geometry: &TopologyGeometry,
    parameter: f64,
) -> Option<(TopologyVertexId, Point2)> {
    let tolerance = curve_period(&curve.spline).max(1.0) * 1.0e-10;
    curve.nodes.iter().enumerate().find_map(|(index, node)| {
        let node_parameter = curve.spline.node_parameter(index)?;
        if parameter_distance(curve, node_parameter, parameter) > tolerance {
            return None;
        }
        let vertex = node.vertex?;
        let point = geometry
            .vertices
            .iter()
            .find(|candidate| candidate.id == vertex)?
            .point(geometry.domain)?;
        Some((vertex, point))
    })
}

fn outer_attachment_point(
    domain: DomainRect,
    side: OuterSide,
    fraction: f64,
) -> Result<Point2, String> {
    if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
        return Err("Outer attachment fraction is invalid".into());
    }
    Ok(match side {
        OuterSide::Bottom => Point2::new(domain.min_x + domain.width() * fraction, domain.min_y),
        OuterSide::Right => Point2::new(domain.max_x, domain.min_y + domain.height() * fraction),
        OuterSide::Top => Point2::new(domain.max_x - domain.width() * fraction, domain.max_y),
        OuterSide::Left => Point2::new(domain.min_x, domain.max_y - domain.height() * fraction),
    })
}

fn outer_fraction(domain: DomainRect, side: OuterSide, point: Point2) -> f64 {
    match side {
        OuterSide::Bottom => (point.x - domain.min_x) / domain.width(),
        OuterSide::Right => (point.y - domain.min_y) / domain.height(),
        OuterSide::Top => (domain.max_x - point.x) / domain.width(),
        OuterSide::Left => (domain.max_y - point.y) / domain.height(),
    }
    .clamp(0.0, 1.0)
}

fn curve_period(spline: &CurveSpline) -> f64 {
    match spline {
        CurveSpline::Closed(spline) => spline.period(),
        CurveSpline::Open(spline) => spline.period(),
    }
}

/// Reports how far the curve's parameter origin moves when a removal consumes
/// the first knot interval. Every other removal merges into the preceding
/// interval and leaves existing parameters untouched.
fn leading_interval_shift(spline: &CurveSpline, target: ControlRemovalTarget) -> f64 {
    if !matches!(
        target,
        ControlRemovalTarget::Closed { node: 0 } | ControlRemovalTarget::OpenStart { .. }
    ) {
        return 0.0;
    }
    match spline {
        CurveSpline::Closed(spline) => spline.intervals().first().copied(),
        CurveSpline::Open(spline) => spline.intervals().first().copied(),
    }
    .unwrap_or(0.0)
}

fn control_removal_target(
    spline: &CurveSpline,
    control: usize,
) -> Result<ControlRemovalTarget, String> {
    match spline {
        CurveSpline::Closed(spline) => {
            if control >= spline.controls().len() {
                return Err("Control no longer exists".into());
            }
            let mut end = 0usize;
            let node = spline
                .multiplicities()
                .iter()
                .position(|multiplicity| {
                    end += *multiplicity as usize;
                    control < end
                })
                .ok_or("Control has no associated curve span")?;
            Ok(ControlRemovalTarget::Closed { node })
        }
        CurveSpline::Open(spline) => {
            let count = spline.controls().len();
            if control >= count {
                return Err("Control no longer exists".into());
            }
            if control <= 1 {
                return Ok(ControlRemovalTarget::OpenStart { control });
            }
            if control + 2 >= count {
                return Ok(ControlRemovalTarget::OpenEnd {
                    from_end: count - 1 - control,
                });
            }
            let mut end = 2usize;
            let slot = spline
                .multiplicities()
                .iter()
                .position(|multiplicity| {
                    end += *multiplicity as usize;
                    control < end
                })
                .ok_or("Control has no associated curve span")?;
            Ok(ControlRemovalTarget::OpenInterior { node: slot + 1 })
        }
    }
}

fn control_removal_indices(
    target: ControlRemovalTarget,
    span_count: usize,
) -> Result<(usize, usize, usize), String> {
    if span_count < 2 {
        return Err("At least four spline controls must remain".into());
    }
    match target {
        ControlRemovalTarget::Closed { node } if node < span_count => {
            Ok((node, (node + span_count - 1) % span_count, node))
        }
        ControlRemovalTarget::OpenStart { .. } => Ok((0, 1, 0)),
        ControlRemovalTarget::OpenInterior { node } if node < span_count => {
            Ok((node, node - 1, node))
        }
        ControlRemovalTarget::OpenEnd { .. } => Ok((span_count - 1, span_count - 2, span_count)),
        _ => Err("Control has no associated curve span".into()),
    }
}

fn smooth_periodic_breakpoint(
    spline: &mut PeriodicCubicSpline,
    breakpoint: usize,
) -> Result<(), String> {
    while spline.continuity(breakpoint).unwrap_or(2) < 2 {
        match spline.decrease_multiplicity(breakpoint, 1.0e-10) {
            Ok(()) => {}
            Err(SplineError::NotRemovable) => {
                spline
                    .decrease_multiplicity_approximate(breakpoint)
                    .map_err(|error| error.to_string())?;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

fn smooth_open_breakpoint(spline: &mut OpenCubicSpline, breakpoint: usize) -> Result<(), String> {
    while spline.continuity(breakpoint).unwrap_or(2) < 2 {
        match spline.decrease_multiplicity(breakpoint, 1.0e-10) {
            Ok(()) => {}
            Err(SplineError::NotRemovable) => {
                spline
                    .decrease_multiplicity_approximate(breakpoint)
                    .map_err(|error| error.to_string())?;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

fn remove_attributed_control(
    spline: &mut CurveSpline,
    target: ControlRemovalTarget,
) -> Result<(), String> {
    match (spline, target) {
        (CurveSpline::Closed(spline), ControlRemovalTarget::Closed { node }) => {
            smooth_periodic_breakpoint(spline, node)?;
            let control = spline.multiplicities()[..node]
                .iter()
                .map(|value| *value as usize)
                .sum::<usize>();
            let mut controls = spline.controls().to_vec();
            controls.remove(control);
            let mut intervals = spline.intervals().to_vec();
            let previous = (node + intervals.len() - 1) % intervals.len();
            intervals[previous] += intervals[node];
            intervals.remove(node);
            let mut multiplicities = spline.multiplicities().to_vec();
            multiplicities.remove(node);
            *spline =
                PeriodicCubicSpline::new_with_multiplicities(controls, intervals, multiplicities)
                    .map_err(|error| error.to_string())?;
        }
        (CurveSpline::Open(spline), ControlRemovalTarget::OpenStart { control }) => {
            if spline.intervals().len() > 1 {
                smooth_open_breakpoint(spline, 1)?;
            }
            let mut controls = spline.controls().to_vec();
            controls.remove(control);
            let mut intervals = spline.intervals().to_vec();
            intervals.remove(0);
            let mut multiplicities = spline.multiplicities().to_vec();
            if !multiplicities.is_empty() {
                multiplicities.remove(0);
            }
            *spline = OpenCubicSpline::new_with_multiplicities(controls, intervals, multiplicities)
                .map_err(|error| error.to_string())?;
        }
        (CurveSpline::Open(spline), ControlRemovalTarget::OpenInterior { node }) => {
            smooth_open_breakpoint(spline, node)?;
            let control = 2 + spline.multiplicities()[..node - 1]
                .iter()
                .map(|value| *value as usize)
                .sum::<usize>();
            let mut controls = spline.controls().to_vec();
            controls.remove(control);
            let mut intervals = spline.intervals().to_vec();
            intervals[node - 1] += intervals[node];
            intervals.remove(node);
            let mut multiplicities = spline.multiplicities().to_vec();
            multiplicities.remove(node - 1);
            *spline = OpenCubicSpline::new_with_multiplicities(controls, intervals, multiplicities)
                .map_err(|error| error.to_string())?;
        }
        (CurveSpline::Open(spline), ControlRemovalTarget::OpenEnd { from_end }) => {
            let spans = spline.intervals().len();
            if spans > 1 {
                smooth_open_breakpoint(spline, spans - 1)?;
            }
            let mut controls = spline.controls().to_vec();
            let control = controls.len() - 1 - from_end;
            controls.remove(control);
            let mut intervals = spline.intervals().to_vec();
            intervals.pop();
            let mut multiplicities = spline.multiplicities().to_vec();
            multiplicities.pop();
            *spline = OpenCubicSpline::new_with_multiplicities(controls, intervals, multiplicities)
                .map_err(|error| error.to_string())?;
        }
        _ => return Err("Control attribution does not match the curve".into()),
    }
    Ok(())
}

fn remap_split_dependencies(
    scene: &mut TopologyScene,
    probes: &mut [TopologyProbeDefinition],
    curve_id: CurveId,
    original: CurveSpanId,
    inserted: CurveSpanId,
    parameter: f64,
) {
    let Some(curve) = scene
        .geometry
        .curves
        .iter()
        .find(|curve| curve.id == curve_id)
    else {
        return;
    };
    for assignment in &mut scene.face_assignments {
        let FaceAnchor::Curve {
            curve: anchor_curve,
            span,
            parameter: anchor_parameter,
            ..
        } = &mut assignment.anchor
        else {
            continue;
        };
        if *anchor_curve != curve_id || *span != original {
            continue;
        }
        if let Some(index) = containing_span(curve, *anchor_parameter) {
            *span = curve.spans[index].id;
        } else if (*anchor_parameter - parameter).abs() <= 1.0e-10
            && let Some(index) = curve.spans.iter().position(|span| span.id == original)
            && let Some([a, b]) = curve.spline.span_bounds(index)
        {
            *anchor_parameter = (a + b) * 0.5;
        }
    }

    let mut pieces = [original, inserted];
    pieces.sort_by_key(|span| {
        curve
            .spans
            .iter()
            .position(|candidate| candidate.id == *span)
            .unwrap_or(usize::MAX)
    });
    for probe in probes {
        let TopologyProbeTarget::Boundary(target) = &mut probe.target else {
            continue;
        };
        if target.curve != curve_id || !target.spans.contains(&original) {
            continue;
        }
        let mut remapped = Vec::with_capacity(target.spans.len() + 1);
        for span in &target.spans {
            if *span == original {
                remapped.extend(pieces);
            } else {
                remapped.push(*span);
            }
        }
        target.spans = remapped;
    }
}

fn remap_removed_span_dependencies(
    scene: &mut TopologyScene,
    probes: &mut [TopologyProbeDefinition],
    curve_id: CurveId,
    removed: CurveSpanId,
    retained: CurveSpanId,
    parameter_shift: f64,
) {
    let curve = scene
        .geometry
        .curves
        .iter()
        .find(|curve| curve.id == curve_id);
    for assignment in &mut scene.face_assignments {
        let FaceAnchor::Curve {
            curve: anchor_curve,
            span,
            parameter,
            ..
        } = &mut assignment.anchor
        else {
            continue;
        };
        if *anchor_curve != curve_id {
            continue;
        }
        let Some(curve) = curve else {
            if *span == removed {
                *span = retained;
            }
            continue;
        };
        if parameter_shift != 0.0 {
            *parameter -= parameter_shift;
            if !curve.spline.is_open() {
                let period = curve_period(&curve.spline);
                if *parameter < 0.0 {
                    *parameter += period;
                }
            }
        }
        if let Some(index) = containing_span(curve, *parameter) {
            *span = curve.spans[index].id;
            continue;
        }
        if *span == removed {
            *span = retained;
        }
        if let Some(index) = curve
            .spans
            .iter()
            .position(|candidate| candidate.id == *span)
            && let Some([start, end]) = curve.spline.span_bounds(index)
        {
            *parameter = (start + end) * 0.5;
        }
    }
    for probe in probes {
        let TopologyProbeTarget::Boundary(target) = &mut probe.target else {
            continue;
        };
        if target.curve != curve_id || !target.spans.contains(&removed) {
            continue;
        }
        for span in &mut target.spans {
            if *span == removed {
                *span = retained;
            }
        }
        target.spans.dedup();
    }
}

fn remap_outer_anchor_dependencies(scene: &mut TopologyScene, side: OuterSide, fraction: f64) {
    let mut breakpoints = scene
        .geometry
        .vertices
        .iter()
        .filter_map(|vertex| match vertex.location {
            TopologyVertexLocation::Outer {
                side: candidate,
                fraction,
            } if candidate == side => Some(fraction),
            _ => None,
        })
        .chain([0.0, 1.0])
        .collect::<Vec<_>>();
    breakpoints.sort_by(f64::total_cmp);
    breakpoints.dedup_by(|left, right| (*left - *right).abs() <= 1.0e-12);
    let previous = breakpoints
        .iter()
        .copied()
        .rfind(|candidate| *candidate < fraction - 1.0e-12);
    let next = breakpoints
        .iter()
        .copied()
        .find(|candidate| *candidate > fraction + 1.0e-12);
    let replacement = previous
        .map(|previous| (previous + fraction) * 0.5)
        .or_else(|| next.map(|next| (fraction + next) * 0.5));
    let Some(replacement) = replacement else {
        return;
    };
    for assignment in &mut scene.face_assignments {
        if let FaceAnchor::Outer {
            side: anchor_side,
            fraction: anchor_fraction,
        } = &mut assignment.anchor
            && *anchor_side == side
            && (*anchor_fraction - fraction).abs() <= 1.0e-12
        {
            *anchor_fraction = replacement;
        }
    }
}

fn curve_face_anchor(
    topology: &TopologySnapshot,
    curve: CurveId,
    face: FaceId,
) -> Option<FaceAnchor> {
    topology.edges.iter().find_map(|edge| {
        let CompiledEdgeSource::Curve(span) = edge.source else {
            return None;
        };
        if edge.curve != Some(curve) {
            return None;
        }
        let side = if edge.left == face {
            CurveTraceSide::Left
        } else if edge.right == face {
            CurveTraceSide::Right
        } else {
            return None;
        };
        Some(FaceAnchor::Curve {
            curve,
            span,
            side,
            parameter: (edge.parameter[0] + edge.parameter[1]) * 0.5,
        })
    })
}

fn any_face_anchor(topology: &TopologySnapshot, face: FaceId) -> Option<FaceAnchor> {
    topology.edges.iter().find_map(|edge| {
        let parameter = (edge.parameter[0] + edge.parameter[1]) * 0.5;
        match edge.source {
            CompiledEdgeSource::Outer(side) if edge.left == face => Some(FaceAnchor::Outer {
                side,
                fraction: parameter,
            }),
            CompiledEdgeSource::Curve(span) => {
                let curve = edge.curve?;
                let side = if edge.left == face {
                    CurveTraceSide::Left
                } else if edge.right == face {
                    CurveTraceSide::Right
                } else {
                    return None;
                };
                Some(FaceAnchor::Curve {
                    curve,
                    span,
                    side,
                    parameter,
                })
            }
            _ => None,
        }
    })
}

/// The single contiguous run of selected spans on one curve, in curve order.
/// A closed curve may wrap through its seam. Returns `None` when the curve has
/// no selected span and an error when the selection breaks into several runs.
/// One surviving piece of a cut curve, described in the original curve's index
/// space so span identities, node vertices, and anchors can be carried across.
struct CutPiece {
    spline: CurveSpline,
    spans: Vec<usize>,
    nodes: Vec<usize>,
}

/// How a contiguous run of spans divides a curve into surviving pieces.
struct CurveCut {
    pieces: Vec<CutPiece>,
}

impl CurveCut {
    /// Plans the cut without mutating anything. `run` is a contiguous run of span
    /// indices in curve order, wrapping the seam on a closed curve, and must not
    /// cover the whole curve.
    fn plan(curve: &TopologyCurve, run: &[usize]) -> Result<Self, String> {
        let count = curve.spans.len();
        if run.is_empty() || run.len() >= count {
            return Err("Select part of a curve to delete".into());
        }
        let first = run[0];
        let last = run[run.len() - 1];
        let mut pieces = Vec::new();
        match &curve.spline {
            CurveSpline::Closed(spline) => {
                let start = (last + 1) % count;
                let kept = count - run.len();
                let opened = spline
                    .clone()
                    .open_at(start)
                    .map_err(|error| error.to_string())?;
                let (piece, _) = opened.split(kept).map_err(|error| error.to_string())?;
                let spans = (0..kept).map(|offset| (start + offset) % count).collect();
                pieces.push(CutPiece {
                    spline: CurveSpline::Open(piece),
                    spans,
                    nodes: vec![],
                });
            }
            CurveSpline::Open(spline) => {
                let keeps_prefix = first > 0;
                let keeps_suffix = last + 1 < count;
                match (keeps_prefix, keeps_suffix) {
                    (true, false) => {
                        let (prefix, _) = spline.clone().split(first).map_err(|e| e.to_string())?;
                        pieces.push(CutPiece {
                            spline: CurveSpline::Open(prefix),
                            spans: (0..first).collect(),
                            nodes: vec![],
                        });
                    }
                    (false, true) => {
                        let (_, suffix) =
                            spline.clone().split(last + 1).map_err(|e| e.to_string())?;
                        pieces.push(CutPiece {
                            spline: CurveSpline::Open(suffix),
                            spans: (last + 1..count).collect(),
                            nodes: vec![],
                        });
                    }
                    (true, true) => {
                        let (prefix, rest) =
                            spline.clone().split(first).map_err(|e| e.to_string())?;
                        let (_, suffix) = rest
                            .split(last + 1 - first)
                            .map_err(|error| error.to_string())?;
                        pieces.push(CutPiece {
                            spline: CurveSpline::Open(prefix),
                            spans: (0..first).collect(),
                            nodes: vec![],
                        });
                        pieces.push(CutPiece {
                            spline: CurveSpline::Open(suffix),
                            spans: (last + 1..count).collect(),
                            nodes: vec![],
                        });
                    }
                    (false, false) => return Err("Select part of a curve to delete".into()),
                }
            }
        }
        let closed = !curve.spline.is_open();
        for piece in &mut pieces {
            let mut nodes = vec![piece.spans[0]];
            for span in &piece.spans {
                nodes.push(if closed { (span + 1) % count } else { span + 1 });
            }
            piece.nodes = nodes;
        }
        Ok(Self { pieces })
    }
}

/// Demotes an open curve to a baffle when one of its free tips ends in a
/// transmitting span, which the arrangement compiler rejects. A mixed curve
/// whose free tip is already a wall keeps its transmitting spans: they still
/// separate two faces. Promotion frees no further endpoints, so one pass settles.
fn promote_freed_curves(geometry: &mut TopologyGeometry) -> Vec<CurveId> {
    let mut promoted = vec![];
    for curve in &mut geometry.curves {
        if curve.spline.is_open() && free_transmitting_end(curve) {
            for span in &mut curve.spans {
                span.behavior = SpanBehavior::REFLECTING;
            }
            promoted.push(curve.id);
        }
    }
    promoted
}

/// Whether an open curve has a free tip whose adjacent span transmits.
fn free_transmitting_end(curve: &TopologyCurve) -> bool {
    let last = curve.nodes.len() - 1;
    (curve.nodes[0].vertex.is_none() && curve.spans[0].behavior == SpanBehavior::Transmitting)
        || (curve.nodes[last].vertex.is_none()
            && curve.spans[curve.spans.len() - 1].behavior == SpanBehavior::Transmitting)
}

/// Moves boundary probes onto the surviving piece that keeps more of their path,
/// mirroring the pre-topology baffle split, and drops the probes that keep
/// nothing or would no longer describe one contiguous run.
fn remap_cut_boundary_probes(
    probes: &mut Vec<TopologyProbeDefinition>,
    cut: CurveId,
    original: &TopologyCurve,
    plan: &CurveCut,
    pieces: &[TopologyCurve],
) -> Vec<ProbeId> {
    let span_weight = |index: usize| {
        original
            .spline
            .span_bounds(index)
            .map_or(0.0, |[start, end]| (end - start).abs())
    };
    let mut removed = vec![];
    probes.retain_mut(|probe| {
        let TopologyProbeTarget::Boundary(target) = &mut probe.target else {
            return true;
        };
        if target.curve != cut {
            return true;
        }
        let mut best: Option<(f64, usize, Vec<usize>)> = None;
        for (index, piece) in plan.pieces.iter().enumerate() {
            let mut positions = vec![];
            let mut weight = 0.0;
            for (position, span) in piece.spans.iter().enumerate() {
                if target.spans.contains(&original.spans[*span].id) {
                    positions.push(position);
                    weight += span_weight(*span);
                }
            }
            if positions.is_empty() {
                continue;
            }
            if best
                .as_ref()
                .is_none_or(|(current, _, _)| weight > *current)
            {
                best = Some((weight, index, positions));
            }
        }
        let Some((_, index, positions)) = best else {
            removed.push(probe.id);
            return false;
        };
        // The remaining path must still be one contiguous run on that piece.
        if positions.windows(2).any(|pair| pair[1] != pair[0] + 1) {
            removed.push(probe.id);
            return false;
        }
        target.curve = pieces[index].id;
        target.spans = positions
            .into_iter()
            .map(|position| pieces[index].spans[position].id)
            .collect();
        true
    });
    removed
}

/// Follows area probes through a region merge: a probe on a region that folded
/// into the exterior identity moves with it, a probe on a region that ceased to
/// exist is dropped and reported.
fn settle_region_probes(
    probes: &mut Vec<TopologyProbeDefinition>,
    remaps: &[(RegionId, RegionId)],
    dropped: &BTreeSet<RegionId>,
) -> Vec<ProbeId> {
    let mut removed = vec![];
    probes.retain_mut(|probe| {
        let TopologyProbeTarget::AreaRegion(region) = &mut probe.target else {
            return true;
        };
        if let Some((_, to)) = remaps.iter().find(|(from, _)| *from == *region) {
            *region = *to;
            return true;
        }
        if dropped.contains(region) {
            removed.push(probe.id);
            return false;
        }
        true
    });
    removed
}

/// Rebuilds the authored face assignments after a removal compiled. `live`
/// pairs every surviving anchor with the face it now resolves to; `forced`
/// names the faces whose region the removal decided — the merged face takes the
/// survivor, a face that lost its only anchor keeps its region under a fresh
/// one. Every other face must have inherited exactly one anchor.
fn rebuild_face_assignments(
    live: &[(FaceId, AuthoredFaceAssignment)],
    topology: &TopologySnapshot,
    forced: &BTreeMap<FaceId, Option<RegionId>>,
) -> Result<Vec<AuthoredFaceAssignment>, String> {
    let mut rebuilt = vec![];
    for face in &topology.faces {
        let candidates = live
            .iter()
            .filter(|(candidate, _)| *candidate == face.id)
            .map(|(_, assignment)| *assignment)
            .collect::<Vec<_>>();
        if let Some(region) = forced.get(&face.id) {
            let anchor = candidates
                .first()
                .map(|assignment| assignment.anchor)
                .or_else(|| any_face_anchor(topology, face.id))
                .ok_or("Merged face has no stable boundary anchor")?;
            rebuilt.push(AuthoredFaceAssignment {
                anchor,
                region: *region,
            });
        } else if candidates.len() == 1 {
            rebuilt.push(candidates[0]);
        } else if candidates.is_empty() {
            return Err("Removal left a face without an anchor".into());
        } else {
            return Err("Removal merged unrelated face assignments".into());
        }
    }
    Ok(rebuilt)
}

/// Which old face contributions land on one face of the compiled removal.
#[derive(Default)]
struct Landing {
    live: Vec<AuthoredFaceAssignment>,
    dead_regions: BTreeSet<RegionId>,
    dead_hole: bool,
}

impl Landing {
    fn regions(&self) -> BTreeSet<RegionId> {
        self.live
            .iter()
            .filter_map(|assignment| assignment.region)
            .chain(self.dead_regions.iter().copied())
            .collect()
    }
}

/// A face of the compiled removal that absorbed two or more active regions.
struct Merge {
    face: FaceId,
    regions: BTreeSet<RegionId>,
}

enum RemovalTarget {
    Curve(CurveId),
    Spans { curve: CurveId, run: Vec<usize> },
}

/// A removal worked out to the point where only the survivor choice is missing.
/// Nothing in it has touched the editor: `candidate` is the geometry after the
/// cut, pruning, promotion, and welding, compiled into `topology`.
struct RemovalPlan {
    candidate: TopologyDocumentModel,
    topology: TopologySnapshot,
    live: Vec<(FaceId, AuthoredFaceAssignment)>,
    landings: BTreeMap<FaceId, Landing>,
    merge: Option<Merge>,
    vanishing: BTreeSet<RegionId>,
    pieces: Vec<CurveId>,
    promoted: Vec<CurveId>,
    joined: Vec<JoinRecord>,
    removed_probes: Vec<ProbeId>,
    provisional_curve: bool,
}

struct RemovalOutcome {
    pieces: Vec<CurveId>,
    promoted: Vec<CurveId>,
    joined: Vec<JoinRecord>,
    removed_regions: Vec<RegionId>,
    region_remaps: Vec<(RegionId, RegionId)>,
    removed_probes: Vec<ProbeId>,
}

/// Resolves the caller's survivor choice against the regions a removal merges.
/// Region 1 is the stable exterior identity: an exterior merge always keeps it
/// and transfers the chosen region's material into it instead.
fn resolve_survivor(
    choices: &BTreeSet<RegionId>,
    keep_region: Option<RegionId>,
) -> Result<(Option<RegionId>, Option<RegionId>), String> {
    let chosen = match choices.len() {
        0 => {
            if keep_region.is_some() {
                return Err("An inactive curve has no material region to keep".into());
            }
            None
        }
        1 => {
            let only = *choices.iter().next().unwrap();
            if keep_region.is_some_and(|selected| selected != only) {
                return Err("Selected surviving material is not adjacent to this curve".into());
            }
            Some(only)
        }
        _ => {
            let selected =
                keep_region.ok_or("Choose which adjacent material survives this deletion")?;
            if !choices.contains(&selected) {
                return Err("Selected surviving material is not adjacent to this curve".into());
            }
            Some(selected)
        }
    };
    let survivor = if choices.contains(&BACKGROUND_REGION) {
        Some(BACKGROUND_REGION)
    } else {
        chosen
    };
    Ok((chosen, survivor))
}

/// The face of the compiled removal that an old face whose anchor died now
/// belongs to. Prefers an edge of the old face on geometry the removal left
/// alone, and falls back to the old face's centroid.
fn attribute_dead_face(
    compiled: &CompiledTopologyScene,
    topology: &TopologySnapshot,
    old_face: FaceId,
    dead_spans: &BTreeSet<CurveSpanId>,
    reparameterised: &BTreeSet<CurveId>,
) -> Option<FaceId> {
    compiled
        .topology
        .edges
        .iter()
        .find_map(|edge| {
            let side = if edge.left == old_face {
                CurveTraceSide::Left
            } else if edge.right == old_face {
                CurveTraceSide::Right
            } else {
                return None;
            };
            let parameter = (edge.parameter[0] + edge.parameter[1]) * 0.5;
            let anchor = match edge.source {
                CompiledEdgeSource::Outer(outer) => {
                    if side != CurveTraceSide::Left {
                        return None;
                    }
                    FaceAnchor::Outer {
                        side: outer,
                        fraction: parameter,
                    }
                }
                CompiledEdgeSource::Curve(span) => {
                    let curve = edge.curve?;
                    if dead_spans.contains(&span) || reparameterised.contains(&curve) {
                        return None;
                    }
                    FaceAnchor::Curve {
                        curve,
                        span,
                        side,
                        parameter,
                    }
                }
            };
            anchor.resolve(topology).ok()
        })
        .or_else(|| {
            compiled
                .topology
                .face(old_face)
                .and_then(CompiledFace::centroid)
                .and_then(|centroid| topology.face_at(centroid))
        })
}

/// Fuses the two loose ends a removal left at a junction that dropped to
/// valence two: two open curves become one, or one curve closes into a loop.
/// Only vertices the removal touched are considered, and a fusion the geometry
/// refuses (a single-span loop, too many controls) leaves that junction as it
/// was. A record with `survivor == absorbed` is a curve that closed.
fn join_valence_two_ends(
    model: &mut TopologyDocumentModel,
    touched: &BTreeSet<TopologyVertexId>,
) -> Vec<JoinRecord> {
    let mut records = vec![];
    for vertex_id in touched {
        let geometry = &model.draft.geometry;
        if !geometry.vertices.iter().any(|vertex| {
            vertex.id == *vertex_id
                && matches!(vertex.location, TopologyVertexLocation::Interior(_))
        }) {
            continue;
        }
        let references = geometry
            .curves
            .iter()
            .flat_map(|curve| {
                curve
                    .nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, node)| node.vertex == Some(*vertex_id))
                    .map(move |(node, _)| {
                        let endpoint =
                            curve.spline.is_open() && (node == 0 || node + 1 == curve.nodes.len());
                        (curve.id, node, endpoint)
                    })
            })
            .collect::<Vec<_>>();
        let [
            (first, first_node, first_end),
            (second, second_node, second_end),
        ] = references[..]
        else {
            continue;
        };
        if !first_end || !second_end {
            continue;
        }
        let before = model.clone();
        for (curve, node) in [(first, first_node), (second, second_node)] {
            if let Some(curve) = model
                .draft
                .geometry
                .curves
                .iter_mut()
                .find(|candidate| candidate.id == curve)
            {
                curve.nodes[node].vertex = None;
            }
        }
        model
            .draft
            .geometry
            .vertices
            .retain(|vertex| vertex.id != *vertex_id);
        let fused = if first == second {
            close_curve_geometry(model, first).map(|()| {
                let seam_control = model
                    .draft
                    .geometry
                    .curve(first)
                    .and_then(|curve| match &curve.spline {
                        CurveSpline::Closed(spline) => Some(spline.controls().len() - 1),
                        CurveSpline::Open(_) => None,
                    })
                    .unwrap_or_default();
                JoinRecord {
                    survivor: first,
                    absorbed: first,
                    seam_node: 0,
                    seam_control,
                    absorbed_far_node: 0,
                }
            })
        } else {
            join_curves(model, (first, first_node), (second, second_node))
        };
        match fused {
            Ok(record) => records.push(record),
            Err(_) => *model = before,
        }
    }
    records
}
/// Follows the point source through a region merge. A distributed source dies
/// with its region because it is a profile over that face, but the point source
/// has a position that is still meshed once the faces merge, so it moves to the
/// surviving identity instead. Without this it names a region that no longer
/// exists: the scene still compiles, and then every candidate is rejected with
/// "references an inactive region" and the solver stops for no visible reason.
fn retarget_point_source(
    model: &mut TopologyDocumentModel,
    remaps: &[(RegionId, RegionId)],
    dropped: &BTreeSet<RegionId>,
    survivor: Option<RegionId>,
) {
    if let Some((_, to)) = remaps.iter().find(|(from, _)| *from == model.source.region) {
        model.source.region = *to;
        return;
    }
    if !dropped.contains(&model.source.region) {
        return;
    }
    match survivor {
        Some(region) => model.source.region = region,
        None => {
            model.source.region = BACKGROUND_REGION;
            model.source.enabled = false;
        }
    }
}

/// Removes everything that only existed because a region did: its distributed
/// source and any probe integrating over it.
fn drop_region_dependents(model: &mut TopologyDocumentModel, region: RegionId) {
    // The point source names a region too. Left behind it points at nothing, and
    // preparation rejects the whole candidate with "references an inactive
    // region", so the simulation stops with no visible cause.
    if model.source.region == region {
        model.source.region = BACKGROUND_REGION;
        model.source.enabled = false;
    }
    model
        .draft
        .regions
        .retain(|candidate| candidate.id != region);
    model
        .draft
        .volume_sources
        .retain(|source| source.region != region);
    model.probes.retain(|probe| {
        !matches!(probe.target, TopologyProbeTarget::AreaRegion(candidate) if candidate == region)
    });
}

/// Seeds a new subdomain's local frame at the centre of the face it owns, so a
/// region-local profile or source starts somewhere inside the region rather than
/// at the world origin.
fn face_frame(topology: &TopologySnapshot, face: Option<FaceId>) -> MaterialFrame {
    let mut frame = MaterialFrame::world();
    if let Some(origin) = face
        .and_then(|face| topology.face(face))
        .and_then(CompiledFace::centroid)
    {
        frame.origin = origin;
    }
    frame
}

fn contiguous_run(
    curve: &TopologyCurve,
    selected: &BTreeSet<CurveSpanId>,
) -> Result<Option<Vec<usize>>, String> {
    let chosen = curve
        .spans
        .iter()
        .map(|span| selected.contains(&span.id))
        .collect::<Vec<_>>();
    if !chosen.iter().any(|value| *value) {
        return Ok(None);
    }
    let count = chosen.len();
    let closed = !curve.spline.is_open();
    if closed && chosen.iter().all(|value| *value) {
        return Err("A complete closed curve cannot become one chord".into());
    }
    let starts = (0..count)
        .filter(|span| {
            chosen[*span]
                && if closed {
                    !chosen[(*span + count - 1) % count]
                } else {
                    *span == 0 || !chosen[*span - 1]
                }
        })
        .collect::<Vec<_>>();
    let [start] = starts.as_slice() else {
        return Err("Select one contiguous run of spans on each curve".into());
    };
    let mut run = vec![*start];
    loop {
        let next = if closed {
            (run[run.len() - 1] + 1) % count
        } else {
            run[run.len() - 1] + 1
        };
        if next >= count || next == *start || !chosen[next] {
            break;
        }
        run.push(next);
    }
    Ok(Some(run))
}

/// Drops interior topology vertices that no longer join anything and clears the
/// breakpoints still pointing at them. Attaching a separator to a curve
/// materialises a shared vertex on that curve's new C0 breakpoint; detaching or
/// deleting the separator used to leave the vertex behind, so an ordinary corner
/// stayed permanently locked out of a continuity change. Outer attachments and
/// free tips are kept: they still constrain their breakpoint.
fn prune_dangling_junctions(geometry: &mut TopologyGeometry) {
    prune_vertices(geometry, |location, references| {
        // A vertex nothing references at all is dead whatever its location: an
        // outer attachment with no curve on it still subdivides its side and
        // still draws a junction handle. An interior vertex additionally needs
        // two incident breakpoints to be joining anything.
        references == 0
            || (references < 2 && matches!(location, TopologyVertexLocation::Interior(_)))
    });
}

/// Drops only the vertices nothing references at all. Detaching one arm of a
/// shared junction uses this rather than [`prune_dangling_junctions`]: the other
/// arm keeps its breakpoint, so re-attaching to the same junction restores the
/// scene exactly instead of materialising a second vertex beside the first.
fn prune_unreferenced_vertices(geometry: &mut TopologyGeometry) {
    prune_vertices(geometry, |_, references| references == 0);
}

fn prune_vertices(
    geometry: &mut TopologyGeometry,
    dead: impl Fn(TopologyVertexLocation, usize) -> bool,
) {
    let orphans = geometry
        .vertices
        .iter()
        .filter(|vertex| {
            let references = geometry
                .curves
                .iter()
                .flat_map(|curve| &curve.nodes)
                .filter(|node| node.vertex == Some(vertex.id))
                .count();
            dead(vertex.location, references)
        })
        .map(|vertex| vertex.id)
        .collect::<BTreeSet<_>>();
    if orphans.is_empty() {
        return;
    }
    for curve in &mut geometry.curves {
        for node in &mut curve.nodes {
            if node.vertex.is_some_and(|vertex| orphans.contains(&vertex)) {
                node.vertex = None;
            }
        }
    }
    geometry
        .vertices
        .retain(|candidate| !orphans.contains(&candidate.id));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(editor: &mut TopologyEditor) {
        for _ in 0..100_000 {
            editor.validate_frame(64);
            if editor.acceptance != TopologyAcceptance::Pending {
                return;
            }
        }
        panic!("topology editor validation did not finish");
    }

    #[test]
    fn closed_subdomain_and_hole_are_atomic_history_commands() {
        let mut editor = TopologyEditor::default();
        let original = editor.document.model.clone();
        editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(-0.35, 0.0), 0.2),
                ClosedCurvePurpose::Subdomain {
                    material: DEFAULT_MATERIAL,
                },
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(editor.document.model.draft.regions.len(), 2);
        assert_eq!(editor.history_len(), (1, 0));

        editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(0.35, 0.0), 0.2),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.compiled_accepted.plan.domains.len(), 2);
        assert_eq!(editor.history_len(), (2, 0));
        assert!(editor.undo());
        settle(&mut editor);
        assert_eq!(editor.document.model.draft.geometry.curves.len(), 1);
        assert!(editor.undo());
        settle(&mut editor);
        assert_eq!(editor.document.model, original);
    }

    #[test]
    fn loaded_document_clears_history_and_reseeds_stable_ids() {
        let mut original = TopologyEditor::default();
        let first_curve = original
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(-0.35, 0.0), 0.15),
                ClosedCurvePurpose::Subdomain {
                    material: DEFAULT_MATERIAL,
                },
            )
            .unwrap();
        settle(&mut original);
        original
            .create_open_curve(
                OpenCubicSpline::polyline(vec![Point2::new(-0.6, -1.0), Point2::new(-0.45, -0.65)])
                    .unwrap(),
                OpenCurvePurpose::BoundaryBaffle,
                Some(outer(OuterSide::Bottom, 0.2)),
                None,
            )
            .unwrap();
        settle(&mut original);
        assert_eq!(original.history_len(), (2, 0));

        let old_curve_ids = original
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .map(|curve| curve.id)
            .collect::<BTreeSet<_>>();
        let old_span_ids = original
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .flat_map(|curve| curve.spans.iter().map(|span| span.id))
            .collect::<BTreeSet<_>>();
        let old_vertex_ids = original
            .document
            .model
            .draft
            .geometry
            .vertices
            .iter()
            .map(|vertex| vertex.id)
            .collect::<BTreeSet<_>>();
        let old_region_ids = original
            .document
            .model
            .draft
            .regions
            .iter()
            .map(|region| region.id)
            .collect::<BTreeSet<_>>();

        let mut loaded = TopologyEditor::from_document(original.document.clone()).unwrap();
        settle(&mut loaded);
        assert_eq!(loaded.history_len(), (0, 0));
        assert_eq!(loaded.acceptance, TopologyAcceptance::Valid);

        let second_curve = loaded
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(0.35, 0.0), 0.15),
                ClosedCurvePurpose::Subdomain {
                    material: DEFAULT_MATERIAL,
                },
            )
            .unwrap();
        settle(&mut loaded);
        assert_ne!(second_curve, first_curve);
        assert!(!old_curve_ids.contains(&second_curve));
        let second = loaded
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == second_curve)
            .unwrap();
        assert!(
            second
                .spans
                .iter()
                .all(|span| !old_span_ids.contains(&span.id))
        );
        assert!(
            loaded
                .document
                .model
                .draft
                .regions
                .iter()
                .any(|region| !old_region_ids.contains(&region.id))
        );

        loaded
            .create_open_curve(
                OpenCubicSpline::polyline(vec![Point2::new(0.6, -1.0), Point2::new(0.45, -0.65)])
                    .unwrap(),
                OpenCurvePurpose::BoundaryBaffle,
                Some(outer(OuterSide::Bottom, 0.8)),
                None,
            )
            .unwrap();
        settle(&mut loaded);
        assert!(
            loaded
                .document
                .model
                .draft
                .geometry
                .vertices
                .iter()
                .any(|vertex| !old_vertex_ids.contains(&vertex.id))
        );
        assert_eq!(loaded.history_len(), (2, 0));
    }

    #[test]
    fn loaded_material_and_probe_ids_are_reseeded_and_document_edits_are_atomic() {
        let document = crate::topology_examples::catalog()[3].document.clone();
        let maximum_material = document
            .model
            .draft
            .materials
            .iter()
            .map(|material| material.id.0)
            .max()
            .unwrap();
        let maximum_probe = document
            .model
            .probes
            .iter()
            .map(|probe| probe.id.0)
            .max()
            .unwrap();
        let mut editor = TopologyEditor::from_document(document).unwrap();
        settle(&mut editor);

        let material = editor.add_material().unwrap();
        settle(&mut editor);
        assert!(material.0 > maximum_material);
        let probe = editor
            .create_probe(
                "new receiver".into(),
                [91, 220, 194],
                TopologyProbeTarget::Point(Point2::new(0.8, 0.0)),
            )
            .unwrap();
        settle(&mut editor);
        assert!(probe.0 > maximum_probe);
        assert_eq!(editor.history_len(), (2, 0));

        let mut source = editor.document.model.source;
        source.enabled = false;
        editor.set_point_source(source).unwrap();
        settle(&mut editor);
        assert_eq!(editor.history_len(), (3, 0));
        assert!(editor.undo());
        settle(&mut editor);
        assert_ne!(editor.document.model.source, source);
        assert!(editor.redo());
        settle(&mut editor);
        assert_eq!(editor.document.model.source, source);
    }

    #[test]
    fn invalid_drag_preserves_accepted_scene_and_cancels_exactly() {
        let mut editor = TopologyEditor::default();
        let curve = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.2),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        settle(&mut editor);
        let accepted = editor.document.model.accepted.clone();
        editor.begin();
        editor.set_control(curve, 0, Point2::new(4.0, 0.0)).unwrap();
        settle(&mut editor);
        assert!(matches!(editor.acceptance, TopologyAcceptance::Invalid(_)));
        assert_eq!(editor.document.model.accepted, accepted);
        editor.cancel();
        settle(&mut editor);
        assert_eq!(editor.document.model.draft, accepted);
        assert_eq!(editor.history_len(), (1, 0));
    }

    #[test]
    fn control_insertion_preserves_shape_and_splits_one_stable_span() {
        let mut editor = TopologyEditor::default();
        let curve = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.2),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        settle(&mut editor);
        let before = match &editor.document.model.draft.geometry.curves[0].spline {
            CurveSpline::Closed(spline) => (0..=128)
                .map(|index| spline.evaluate(spline.period() * index as f64 / 128.0))
                .collect::<Vec<_>>(),
            CurveSpline::Open(_) => unreachable!(),
        };
        let old_spans = editor.document.model.draft.geometry.curves[0].spans.len();
        let old_history = editor.history_len().0;
        let (control, inserted) = editor.insert_control(curve, 0.5).unwrap();
        settle(&mut editor);
        let authored = &editor.document.model.draft.geometry.curves[0];
        let after = match &authored.spline {
            CurveSpline::Closed(spline) => (0..=128)
                .map(|index| spline.evaluate(spline.period() * index as f64 / 128.0))
                .collect::<Vec<_>>(),
            CurveSpline::Open(_) => unreachable!(),
        };
        assert!(inserted.is_some());
        assert!(
            control
                < match &authored.spline {
                    CurveSpline::Closed(spline) => spline.controls().len(),
                    CurveSpline::Open(spline) => spline.controls().len(),
                }
        );
        assert_eq!(authored.spans.len(), old_spans + 1);
        assert_eq!(editor.history_len().0, old_history + 1);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert!(
            before
                .iter()
                .zip(after)
                .all(|(left, right)| (*left - right).norm() <= 1.0e-11)
        );
        let history = editor.history_len().0;
        editor.remove_control(curve, control).unwrap();
        settle(&mut editor);
        assert_eq!(
            editor.document.model.draft.geometry.curves[0].spans.len(),
            old_spans
        );
        assert_eq!(editor.history_len().0, history + 1);
    }

    #[test]
    fn repeated_knot_control_deletion_smooths_only_its_attributed_corner() {
        let mut editor = TopologyEditor::default();
        let curve = editor
            .create_closed_curve(
                PeriodicCubicSpline::polygon(vec![
                    Point2::new(-0.3, -0.3),
                    Point2::new(0.3, -0.3),
                    Point2::new(0.3, 0.3),
                    Point2::new(-0.3, 0.3),
                ])
                .unwrap(),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        settle(&mut editor);
        let history = editor.history_len().0;
        let last_control = match &editor.document.model.draft.geometry.curves[0].spline {
            CurveSpline::Closed(spline) => spline.controls().len() - 1,
            CurveSpline::Open(_) => unreachable!(),
        };

        editor.remove_control(curve, last_control).unwrap();
        let curve = &editor.document.model.draft.geometry.curves[0];
        assert_eq!(curve.spans.len(), 3);
        assert_eq!(curve.nodes.len(), 3);
        let CurveSpline::Closed(spline) = &curve.spline else {
            unreachable!()
        };
        assert_eq!(spline.controls().len(), 9);
        assert_eq!(spline.multiplicities(), &[3, 3, 3]);
        assert_eq!(editor.history_len().0, history + 1);
    }

    #[test]
    fn control_deletion_retains_unrelated_repeated_knots() {
        let mut editor = TopologyEditor::default();
        let curve_id = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.3),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        settle(&mut editor);
        editor.set_curve_continuity(curve_id, 1, 0).unwrap();
        let control_for_node_four = match &editor.document.model.draft.geometry.curves[0].spline {
            CurveSpline::Closed(spline) => spline.multiplicities()[..4]
                .iter()
                .map(|value| *value as usize)
                .sum(),
            CurveSpline::Open(_) => unreachable!(),
        };

        editor
            .remove_control(curve_id, control_for_node_four)
            .unwrap();
        let CurveSpline::Closed(spline) = &editor.document.model.draft.geometry.curves[0].spline
        else {
            unreachable!()
        };
        assert_eq!(spline.multiplicities()[1], 3);
        assert_eq!(spline.intervals().len(), 7);
    }

    #[test]
    fn open_curve_end_control_deletion_smooths_the_adjacent_corner() {
        let mut editor = TopologyEditor::default();
        let curve_id = editor
            .create_boundary_baffle(
                OpenCubicSpline::polyline(vec![
                    Point2::new(-0.6, -0.2),
                    Point2::new(-0.2, 0.2),
                    Point2::new(0.2, -0.2),
                    Point2::new(0.6, 0.2),
                ])
                .unwrap(),
            )
            .unwrap();
        settle(&mut editor);
        let last_control = match &editor.document.model.draft.geometry.curves[0].spline {
            CurveSpline::Open(spline) => spline.controls().len() - 1,
            CurveSpline::Closed(_) => unreachable!(),
        };

        editor.remove_control(curve_id, last_control).unwrap();
        let curve = &editor.document.model.draft.geometry.curves[0];
        assert_eq!(curve.spans.len(), 2);
        assert_eq!(curve.nodes.len(), 3);
        let CurveSpline::Open(spline) = &curve.spline else {
            unreachable!()
        };
        assert_eq!(spline.multiplicities(), &[3]);
    }

    #[test]
    fn bulk_span_behavior_is_one_undoable_action() {
        let mut editor = TopologyEditor::default();
        let _curve = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.2),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        settle(&mut editor);
        let spans = editor.document.model.draft.geometry.curves[0]
            .spans
            .iter()
            .take(3)
            .map(|span| span.id)
            .collect();
        let coupled = SpanBehavior::Separated {
            left: FaceBoundaryCondition::Reflecting,
            right: FaceBoundaryCondition::Reflecting,
            coupling: InternalBoundaryCoupling::ThinGap {
                stiffness_ratio: 1.0,
            },
        };
        editor.set_span_behavior(&spans, coupled).unwrap();
        settle(&mut editor);
        assert!(matches!(editor.acceptance, TopologyAcceptance::Invalid(_)));
        assert_eq!(editor.history_len(), (2, 0));
        assert!(editor.undo());
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert!(
            editor.document.model.draft.geometry.curves[0]
                .spans
                .iter()
                .all(|span| span.behavior == SpanBehavior::REFLECTING)
        );
    }

    #[test]
    fn bulk_curve_side_and_outer_conditions_are_atomic() {
        let mut editor = TopologyEditor::default();
        editor
            .create_boundary_baffle(
                OpenCubicSpline::polyline(vec![
                    Point2::new(-0.5, 0.0),
                    Point2::new(0.0, 0.2),
                    Point2::new(0.5, 0.0),
                ])
                .unwrap(),
            )
            .unwrap();
        settle(&mut editor);
        let spans = editor.document.model.draft.geometry.curves[0]
            .spans
            .iter()
            .map(|span| span.id)
            .collect::<BTreeSet<_>>();
        let before = editor.history_len().0;
        editor
            .set_span_face_condition(
                &spans,
                CurveTraceSide::Right,
                FaceBoundaryCondition::Dirichlet {
                    signal: TimeSignal::harmonic(0.0, 2.0, 3.0, 0.4),
                },
            )
            .unwrap();
        assert_eq!(editor.history_len().0, before + 1);
        assert!(
            editor.document.model.draft.geometry.curves[0]
                .spans
                .iter()
                .all(|span| matches!(
                    span.behavior,
                    SpanBehavior::Separated {
                        right: FaceBoundaryCondition::Dirichlet { .. },
                        coupling: InternalBoundaryCoupling::Independent,
                        ..
                    }
                ))
        );

        let sides = BTreeSet::from([OuterSide::Left, OuterSide::Top]);
        let before = editor.history_len().0;
        editor
            .set_outer_condition(&sides, OuterBoundaryCondition::FirstOrderOutgoing)
            .unwrap();
        assert_eq!(editor.history_len().0, before + 1);
        assert_eq!(
            editor.document.model.draft.outer_boundaries.sides[OuterSide::Left.index()],
            OuterBoundaryCondition::FirstOrderOutgoing
        );
        assert_eq!(
            editor.document.model.draft.outer_boundaries.sides[OuterSide::Top.index()],
            OuterBoundaryCondition::FirstOrderOutgoing
        );
    }

    #[test]
    fn free_baffle_is_valid_and_keeps_both_trace_sides() {
        let mut editor = TopologyEditor::default();
        let curve = editor
            .create_boundary_baffle(
                OpenCubicSpline::polyline(vec![
                    Point2::new(-0.5, 0.0),
                    Point2::new(0.0, 0.2),
                    Point2::new(0.5, 0.0),
                ])
                .unwrap(),
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        let sides = editor
            .compiled_accepted
            .plan
            .boundaries
            .iter()
            .filter_map(|boundary| match boundary.source {
                PlannedBoundarySource::Curve {
                    curve: candidate,
                    side,
                    ..
                } if candidate == curve => Some(side),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            sides,
            BTreeSet::from([CurveTraceSide::Left, CurveTraceSide::Right])
        );
        assert_eq!(editor.history_len(), (1, 0));
    }

    fn outer(side: OuterSide, fraction: f64) -> TopologyAttachment {
        TopologyAttachment::Boundary(FaceAnchor::Outer { side, fraction })
    }

    #[test]
    fn outer_to_outer_separator_splits_and_assigns_one_new_region_atomically() {
        let mut editor = TopologyEditor::default();
        let edit = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    Point2::new(0.0, -0.8),
                    Point2::new(0.0, 0.0),
                    Point2::new(0.0, 0.8),
                ])
                .unwrap(),
                OpenCurvePurpose::SubdomainSeparator {
                    material: DEFAULT_MATERIAL,
                },
                Some(outer(OuterSide::Bottom, 0.5)),
                Some(outer(OuterSide::Top, 0.5)),
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(editor.document.model.draft.regions.len(), 2);
        assert_eq!(editor.compiled_accepted.plan.domains.len(), 2);
        let curve = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == edit.curve)
            .unwrap();
        assert!(curve.nodes.first().unwrap().vertex.is_some());
        assert!(curve.nodes.last().unwrap().vertex.is_some());
        assert!(
            curve
                .spans
                .iter()
                .all(|span| span.behavior == SpanBehavior::Transmitting)
        );
        assert_eq!(editor.history_len(), (1, 0));
    }

    #[test]
    fn curve_interior_attachment_remaps_face_anchor_and_boundary_probe() {
        let mut editor = TopologyEditor::default();
        let loop_id = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.25),
                ClosedCurvePurpose::Subdomain {
                    material: DEFAULT_MATERIAL,
                },
            )
            .unwrap();
        settle(&mut editor);
        let curve = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == loop_id)
            .unwrap();
        let original = curve.spans[0].id;
        let [a, b] = curve.spline.span_bounds(0).unwrap();
        let parameter = (a + b) * 0.5;
        let point = evaluate_curve(curve, parameter);
        let background_face = FaceAnchor::Outer {
            side: OuterSide::Right,
            fraction: 0.5,
        }
        .resolve(&editor.compiled_accepted.topology)
        .unwrap();
        let side = [CurveTraceSide::Left, CurveTraceSide::Right]
            .into_iter()
            .find(|side| {
                FaceAnchor::Curve {
                    curve: loop_id,
                    span: original,
                    side: *side,
                    parameter,
                }
                .resolve(&editor.compiled_accepted.topology)
                .is_ok_and(|face| face == background_face)
            })
            .unwrap();
        editor.document.model.probes.push(TopologyProbeDefinition {
            id: ProbeId(1),
            name: "loop trace".into(),
            color: [91, 220, 194],
            enabled: true,
            target: TopologyProbeTarget::Boundary(TopologyBoundaryProbeTarget {
                curve: loop_id,
                spans: vec![original],
                side,
                reversed: false,
                preset: ProbeSamplingPreset::Medium,
            }),
        });
        let edit = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![point, Point2::new(0.65, point.y + 0.1)]).unwrap(),
                OpenCurvePurpose::BoundaryBaffle,
                Some(TopologyAttachment::Boundary(FaceAnchor::Curve {
                    curve: loop_id,
                    span: original,
                    side,
                    parameter,
                })),
                None,
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(edit.span_splits.len(), 1);
        let target = match &editor.document.model.probes[0].target {
            TopologyProbeTarget::Boundary(target) => target,
            _ => unreachable!(),
        };
        assert_eq!(target.spans.len(), 2);
        assert!(target.spans.contains(&original));
        assert!(target.spans.contains(&edit.span_splits[0].inserted));
        for assignment in &editor.document.model.draft.face_assignments {
            assignment
                .anchor
                .resolve(&editor.compiled_accepted.topology)
                .unwrap();
        }
    }

    #[test]
    fn existing_junction_accepts_a_new_baffle_arm() {
        let mut editor = TopologyEditor::default();
        let separator = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    Point2::new(0.0, -0.8),
                    Point2::new(0.0, 0.0),
                    Point2::new(0.0, 0.8),
                ])
                .unwrap(),
                OpenCurvePurpose::SubdomainSeparator {
                    material: DEFAULT_MATERIAL,
                },
                Some(outer(OuterSide::Bottom, 0.5)),
                Some(outer(OuterSide::Top, 0.5)),
            )
            .unwrap();
        settle(&mut editor);
        let divider = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == separator.curve)
            .unwrap();
        let vertex = divider.nodes[0].vertex.unwrap();
        let face = editor
            .compiled_accepted
            .topology
            .face_at(Point2::new(-0.1, -0.9))
            .unwrap();
        let baffle = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![Point2::new(0.0, -1.0), Point2::new(-0.35, -0.65)])
                    .unwrap(),
                OpenCurvePurpose::BoundaryBaffle,
                Some(TopologyAttachment::Junction { vertex, face }),
                None,
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        let curve = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == baffle.curve)
            .unwrap();
        assert_eq!(curve.nodes[0].vertex, Some(vertex));
    }

    #[test]
    fn separator_can_split_an_inner_face_between_two_curve_attachments() {
        let mut editor = TopologyEditor::default();
        let loop_id = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.55),
                ClosedCurvePurpose::Subdomain {
                    material: DEFAULT_MATERIAL,
                },
            )
            .unwrap();
        settle(&mut editor);
        let loop_curve = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == loop_id)
            .unwrap();
        let make_target = |span_index: usize| {
            let span = loop_curve.spans[span_index].id;
            let [a, b] = loop_curve.spline.span_bounds(span_index).unwrap();
            let parameter = (a + b) * 0.5;
            let side = [CurveTraceSide::Left, CurveTraceSide::Right]
                .into_iter()
                .find(|side| {
                    FaceAnchor::Curve {
                        curve: loop_id,
                        span,
                        side: *side,
                        parameter,
                    }
                    .resolve(&editor.compiled_accepted.topology)
                    .is_ok_and(|face| {
                        editor
                            .compiled_accepted
                            .assignments
                            .iter()
                            .any(|assignment| {
                                assignment.face == face
                                    && assignment.region != Some(BACKGROUND_REGION)
                            })
                    })
                })
                .unwrap();
            (
                evaluate_curve(loop_curve, parameter),
                TopologyAttachment::Boundary(FaceAnchor::Curve {
                    curve: loop_id,
                    span,
                    side,
                    parameter,
                }),
            )
        };
        let (start_point, start) = make_target(0);
        let (end_point, end) = make_target(4);
        let edit = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![start_point, Point2::default(), end_point]).unwrap(),
                OpenCurvePurpose::SubdomainSeparator {
                    material: DEFAULT_MATERIAL,
                },
                Some(start),
                Some(end),
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(edit.span_splits.len(), 2);
        assert_eq!(editor.document.model.draft.regions.len(), 3);
        assert_eq!(editor.compiled_accepted.plan.domains.len(), 3);
    }

    /// A reshaping control deletion must not tear an incident junction off its
    /// authoritative topology vertex, and must not let a face anchor drift
    /// across a junction into a face that another assignment already owns.
    #[test]
    fn seam_control_deletion_keeps_junctions_pinned_and_anchors_on_their_face() {
        let mut editor = TopologyEditor::default();
        let loop_id = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.5),
                ClosedCurvePurpose::Subdomain {
                    material: DEFAULT_MATERIAL,
                },
            )
            .unwrap();
        settle(&mut editor);
        let loop_curve = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == loop_id)
            .unwrap();
        let inner_target = |span_index: usize, fraction: f64| {
            let span = loop_curve.spans[span_index].id;
            let [a, b] = loop_curve.spline.span_bounds(span_index).unwrap();
            let parameter = a + (b - a) * fraction;
            let side = [CurveTraceSide::Left, CurveTraceSide::Right]
                .into_iter()
                .find(|side| {
                    FaceAnchor::Curve {
                        curve: loop_id,
                        span,
                        side: *side,
                        parameter,
                    }
                    .resolve(&editor.compiled_accepted.topology)
                    .is_ok_and(|face| {
                        editor
                            .compiled_accepted
                            .assignments
                            .iter()
                            .any(|assignment| {
                                assignment.face == face
                                    && assignment.region != Some(BACKGROUND_REGION)
                            })
                    })
                })
                .unwrap();
            (
                evaluate_curve(loop_curve, parameter),
                TopologyAttachment::Boundary(FaceAnchor::Curve {
                    curve: loop_id,
                    span,
                    side,
                    parameter,
                }),
            )
        };
        // The first attachment sits early inside span 1, so the seam removal
        // below moves the loop's own anchor backwards past that junction.
        let (start_point, start) = inner_target(1, 0.3);
        let (end_point, end) = inner_target(5, 0.5);
        editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![start_point, Point2::default(), end_point]).unwrap(),
                OpenCurvePurpose::SubdomainSeparator {
                    material: DEFAULT_MATERIAL,
                },
                Some(start),
                Some(end),
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        let regions = editor
            .document
            .model
            .draft
            .regions
            .iter()
            .map(|region| region.id)
            .collect::<BTreeSet<_>>();
        assert_eq!(editor.compiled_accepted.plan.domains.len(), 3);

        for control in [0usize, 1] {
            let mut candidate = TopologyEditor::from_document(editor.document.clone()).unwrap();
            settle(&mut candidate);
            candidate.remove_control(loop_id, control).unwrap();
            settle(&mut candidate);
            assert_eq!(
                candidate.acceptance,
                TopologyAcceptance::Valid,
                "control {control} left the document invalid"
            );
            let geometry = &candidate.document.model.draft.geometry;
            for curve in &geometry.curves {
                for (index, node) in curve.nodes.iter().enumerate() {
                    let Some(vertex) = node.vertex else {
                        continue;
                    };
                    let target = geometry
                        .vertices
                        .iter()
                        .find(|candidate| candidate.id == vertex)
                        .and_then(|candidate| candidate.point(geometry.domain))
                        .unwrap();
                    let point = curve.spline.node_point(index).unwrap();
                    assert!(
                        (point - target).norm() <= 1.0e-12,
                        "control {control} moved junction {vertex:?} off its vertex"
                    );
                }
            }
            assert_eq!(candidate.compiled_accepted.plan.domains.len(), 3);
            assert_eq!(
                candidate
                    .document
                    .model
                    .draft
                    .regions
                    .iter()
                    .map(|region| region.id)
                    .collect::<BTreeSet<_>>(),
                regions
            );
        }
    }

    #[test]
    fn detaching_a_separator_is_an_invalid_draft_that_undo_restores() {
        let mut editor = TopologyEditor::default();
        let separator = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    Point2::new(0.0, -0.8),
                    Point2::new(0.0, 0.0),
                    Point2::new(0.0, 0.8),
                ])
                .unwrap(),
                OpenCurvePurpose::SubdomainSeparator {
                    material: DEFAULT_MATERIAL,
                },
                Some(outer(OuterSide::Bottom, 0.5)),
                Some(outer(OuterSide::Top, 0.5)),
            )
            .unwrap();
        settle(&mut editor);
        let accepted = editor.document.model.accepted.clone();
        let vertex = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == separator.curve)
            .unwrap()
            .nodes[0]
            .vertex
            .unwrap();
        let face = editor
            .compiled_accepted
            .topology
            .vertices
            .iter()
            .find(|candidate| candidate.authored == Some(vertex))
            .unwrap()
            .traces[0]
            .face;
        editor.detach_endpoint(separator.curve, 0).unwrap();
        settle(&mut editor);
        assert!(matches!(editor.acceptance, TopologyAcceptance::Invalid(_)));
        assert_eq!(editor.document.model.accepted, accepted);
        editor
            .attach_endpoint(
                separator.curve,
                0,
                TopologyAttachment::Junction { vertex, face },
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(editor.document.model.draft, accepted);
        assert!(editor.undo());
        settle(&mut editor);
        assert!(matches!(editor.acceptance, TopologyAcceptance::Invalid(_)));
        assert!(editor.undo());
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(editor.document.model.draft, accepted);
    }

    #[test]
    fn divider_removal_requires_ownership_and_removes_dropped_dependents() {
        let mut editor = TopologyEditor::default();
        let separator = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    Point2::new(0.0, -0.8),
                    Point2::new(0.0, 0.0),
                    Point2::new(0.0, 0.8),
                ])
                .unwrap(),
                OpenCurvePurpose::SubdomainSeparator {
                    material: DEFAULT_MATERIAL,
                },
                Some(outer(OuterSide::Bottom, 0.5)),
                Some(outer(OuterSide::Top, 0.5)),
            )
            .unwrap();
        settle(&mut editor);
        let created_region = editor
            .document
            .model
            .draft
            .regions
            .iter()
            .map(|region| region.id)
            .find(|region| *region != BACKGROUND_REGION)
            .unwrap();
        let span = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == separator.curve)
            .unwrap()
            .spans[0]
            .id;
        editor.document.model.probes.extend([
            TopologyProbeDefinition {
                id: ProbeId(1),
                name: "divider".into(),
                color: [91, 220, 194],
                enabled: true,
                target: TopologyProbeTarget::Boundary(TopologyBoundaryProbeTarget {
                    curve: separator.curve,
                    spans: vec![span],
                    side: CurveTraceSide::Left,
                    reversed: false,
                    preset: ProbeSamplingPreset::Medium,
                }),
            },
            TopologyProbeDefinition {
                id: ProbeId(2),
                name: "new face".into(),
                color: [248, 196, 112],
                enabled: true,
                target: TopologyProbeTarget::AreaRegion(created_region),
            },
        ]);
        let before = editor.document.model.clone();
        assert!(editor.remove_curve(separator.curve, None).is_err());
        assert_eq!(editor.document.model, before);
        assert_eq!(editor.history_len(), (1, 0));

        let removal = editor
            .remove_curve(separator.curve, Some(BACKGROUND_REGION))
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(removal.removed_regions, vec![created_region]);
        assert_eq!(removal.removed_probes, vec![ProbeId(1), ProbeId(2)]);
        assert!(editor.document.model.draft.geometry.curves.is_empty());
        assert_eq!(editor.document.model.draft.regions.len(), 1);
        assert!(editor.document.model.probes.is_empty());
        assert_eq!(editor.history_len(), (2, 0));
        assert!(editor.undo());
        settle(&mut editor);
        assert_eq!(editor.document.model.probes.len(), 2);
        assert_eq!(editor.document.model.draft.regions.len(), 2);
    }

    #[test]
    fn hole_removal_keeps_its_only_active_neighbor_without_a_choice() {
        let mut editor = TopologyEditor::default();
        let hole = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.25),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        settle(&mut editor);
        editor.remove_curve(hole, None).unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert!(editor.document.model.draft.geometry.curves.is_empty());
        assert_eq!(editor.document.model.draft.regions.len(), 1);
        assert_eq!(editor.compiled_accepted.plan.domains.len(), 1);
    }

    #[test]
    fn exterior_merge_can_keep_the_other_material_via_background_identity() {
        let mut editor = TopologyEditor::default();
        let mut glass = Material::default_medium();
        glass.id = MaterialId(2);
        glass.name = "Glass".into();
        glass.color = [80, 120, 180];
        editor.document.model.draft.materials.push(glass.clone());
        editor.document.model.accepted.materials.push(glass);
        let separator = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    Point2::new(0.0, -0.8),
                    Point2::new(0.0, 0.0),
                    Point2::new(0.0, 0.8),
                ])
                .unwrap(),
                OpenCurvePurpose::SubdomainSeparator {
                    material: MaterialId(2),
                },
                Some(outer(OuterSide::Bottom, 0.5)),
                Some(outer(OuterSide::Top, 0.5)),
            )
            .unwrap();
        settle(&mut editor);
        let glass_region = editor
            .document
            .model
            .draft
            .regions
            .iter()
            .find(|region| region.material == MaterialId(2))
            .unwrap()
            .id;
        editor.document.model.probes.extend([
            TopologyProbeDefinition {
                id: ProbeId(10),
                name: "old exterior".into(),
                color: [91, 220, 194],
                enabled: true,
                target: TopologyProbeTarget::AreaRegion(BACKGROUND_REGION),
            },
            TopologyProbeDefinition {
                id: ProbeId(11),
                name: "glass side".into(),
                color: [248, 196, 112],
                enabled: true,
                target: TopologyProbeTarget::AreaRegion(glass_region),
            },
        ]);
        let removal = editor
            .remove_curve(separator.curve, Some(glass_region))
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(
            removal.region_remaps,
            vec![(glass_region, BACKGROUND_REGION)]
        );
        assert_eq!(removal.removed_probes, vec![ProbeId(10)]);
        assert_eq!(editor.document.model.draft.regions.len(), 1);
        assert_eq!(
            editor.document.model.draft.regions[0].material,
            MaterialId(2)
        );
        assert_eq!(editor.document.model.probes.len(), 1);
        assert!(matches!(
            editor.document.model.probes[0].target,
            TopologyProbeTarget::AreaRegion(BACKGROUND_REGION)
        ));
    }

    #[test]
    fn viewport_drag_is_one_history_entry_and_cancel_restores_geometry() {
        use crate::topology_viewport::{RigidTransform, TopologySpanTarget, plan_rigid_transform};

        let mut editor = TopologyEditor::default();
        editor
            .create_boundary_baffle(
                OpenCubicSpline::polyline(vec![Point2::new(-0.4, -0.2), Point2::new(0.4, -0.2)])
                    .unwrap(),
            )
            .unwrap();
        settle(&mut editor);
        let original = editor.document.model.draft.geometry.clone();
        let span = original.curves[0].spans[0].id;
        let selected = BTreeSet::from([TopologySpanTarget::Curve(span)]);

        editor.begin();
        for _ in 0..2 {
            let updates = plan_rigid_transform(
                &editor.document.model.draft.geometry,
                &selected,
                RigidTransform {
                    pivot: Point2::default(),
                    translation: Point2::new(0.05, 0.0),
                    rotation_radians: 0.0,
                    scale: 1.0,
                },
            )
            .unwrap();
            editor
                .apply_transform_updates_during_edit(&updates)
                .unwrap();
        }
        editor.commit();
        assert_eq!(editor.history_len(), (2, 0));
        assert_ne!(editor.document.model.draft.geometry, original);
        assert!(editor.undo());
        assert_eq!(editor.document.model.draft.geometry, original);

        let updates = plan_rigid_transform(
            &editor.document.model.draft.geometry,
            &selected,
            RigidTransform {
                pivot: Point2::default(),
                translation: Point2::new(0.0, 0.2),
                rotation_radians: 0.0,
                scale: 1.0,
            },
        )
        .unwrap();
        editor.begin();
        editor
            .apply_transform_updates_during_edit(&updates)
            .unwrap();
        editor.cancel();
        assert_eq!(editor.document.model.draft.geometry, original);
    }

    #[test]
    fn span_isolation_is_exact_stable_and_one_history_action() {
        let mut editor = TopologyEditor::default();
        let curve_id = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.3),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        settle(&mut editor);
        let curve = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == curve_id)
            .unwrap();
        let span_ids = curve.spans.iter().map(|span| span.id).collect::<Vec<_>>();
        let before = (0..257)
            .map(|sample| {
                let spline = match &curve.spline {
                    CurveSpline::Closed(spline) => spline,
                    _ => unreachable!(),
                };
                spline.evaluate(spline.period() * sample as f64 / 257.0)
            })
            .collect::<Vec<_>>();
        let history = editor.history_len().0;

        editor
            .isolate_span_boundaries(&BTreeSet::from([span_ids[1], span_ids[2]]))
            .unwrap();

        let curve = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == curve_id)
            .unwrap();
        let spline = match &curve.spline {
            CurveSpline::Closed(spline) => spline,
            _ => unreachable!(),
        };
        assert_eq!(spline.continuity(1), Some(0));
        assert_eq!(spline.continuity(3), Some(0));
        assert_eq!(
            curve.spans.iter().map(|span| span.id).collect::<Vec<_>>(),
            span_ids
        );
        for (sample, expected) in before.into_iter().enumerate() {
            let actual = spline.evaluate(spline.period() * sample as f64 / 257.0);
            assert!((actual - expected).norm() < 1.0e-11);
        }
        assert_eq!(editor.history_len().0, history + 1);
        assert!(editor.undo());
        let restored = &editor.document.model.draft.geometry.curves[0];
        let restored = match &restored.spline {
            CurveSpline::Closed(spline) => spline,
            _ => unreachable!(),
        };
        assert_eq!(restored.continuity(1), Some(2));
    }

    #[test]
    fn continuity_upgrade_reshapes_and_topology_nodes_stay_c0() {
        let mut editor = TopologyEditor::default();
        let curve_id = editor
            .create_closed_curve(
                PeriodicCubicSpline::polygon(vec![
                    Point2::new(-0.3, -0.3),
                    Point2::new(0.3, -0.3),
                    Point2::new(0.3, 0.3),
                    Point2::new(-0.3, 0.3),
                ])
                .unwrap(),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        settle(&mut editor);
        assert!(editor.set_curve_continuity(curve_id, 1, 2).unwrap() > 0.0);
        let curve = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == curve_id)
            .unwrap();
        assert_eq!(
            match &curve.spline {
                CurveSpline::Closed(spline) => spline.continuity(1),
                _ => unreachable!(),
            },
            Some(2)
        );

        editor.document.model.draft.geometry.curves[0].nodes[2].vertex =
            Some(TopologyVertexId(999));
        assert_eq!(
            editor.set_curve_continuity(curve_id, 2, 1).unwrap_err(),
            "A topology junction must remain a C0 corner"
        );
    }

    #[test]
    fn straighten_selected_spans_preserves_ids_and_makes_chords() {
        let mut editor = TopologyEditor::default();
        let curve_id = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.3),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        settle(&mut editor);
        let curve = &editor.document.model.draft.geometry.curves[0];
        let selected = BTreeSet::from([curve.spans[0].id, curve.spans[2].id]);
        let ids = curve.spans.iter().map(|span| span.id).collect::<Vec<_>>();
        let history = editor.history_len().0;

        editor.straighten_spans(&selected).unwrap();

        let curve = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == curve_id)
            .unwrap();
        assert_eq!(
            curve.spans.iter().map(|span| span.id).collect::<Vec<_>>(),
            ids
        );
        let spline = match &curve.spline {
            CurveSpline::Closed(spline) => spline,
            _ => unreachable!(),
        };
        for span in [0, 2] {
            let [start_t, end_t] = spline.span_bounds(span).unwrap();
            let start = spline.evaluate(start_t);
            let end = spline.evaluate(end_t);
            let middle = spline.evaluate((start_t + end_t) * 0.5);
            assert!((middle - start.lerp(end, 0.5)).norm() < 1.0e-12);
        }
        assert_eq!(editor.history_len().0, history + 1);
    }

    fn span_ids(editor: &TopologyEditor, curve: CurveId) -> Vec<CurveSpanId> {
        editor
            .document
            .model
            .draft
            .geometry
            .curve(curve)
            .unwrap()
            .spans
            .iter()
            .map(|span| span.id)
            .collect()
    }

    fn all_separated(editor: &TopologyEditor, curve: CurveId) -> bool {
        editor
            .document
            .model
            .draft
            .geometry
            .curve(curve)
            .unwrap()
            .spans
            .iter()
            .all(|span| matches!(span.behavior, SpanBehavior::Separated { .. }))
    }

    /// Detaching one arm of a shared junction must leave the other arm's
    /// breakpoint bound, so re-attaching to that junction restores the scene
    /// rather than materialising a second vertex beside the first.
    #[test]
    fn detaching_one_arm_keeps_the_junction_for_the_other() {
        let mut editor = TopologyEditor::default();
        let loop_id = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.4),
                ClosedCurvePurpose::Subdomain {
                    material: DEFAULT_MATERIAL,
                },
            )
            .unwrap();
        settle(&mut editor);
        let interior = editor
            .compiled_accepted
            .topology
            .face_at(Point2::default())
            .unwrap();
        let target = |editor: &TopologyEditor, index: usize| {
            let curve = editor.document.model.draft.geometry.curve(loop_id).unwrap();
            let span = curve.spans[index].id;
            let [a, b] = curve.spline.span_bounds(index).unwrap();
            let parameter = (a + b) * 0.5;
            let side = [CurveTraceSide::Left, CurveTraceSide::Right]
                .into_iter()
                .find(|side| {
                    FaceAnchor::Curve {
                        curve: loop_id,
                        span,
                        side: *side,
                        parameter,
                    }
                    .resolve(&editor.compiled_accepted.topology)
                    .is_ok_and(|face| face == interior)
                })
                .unwrap();
            (
                TopologyAttachment::Boundary(FaceAnchor::Curve {
                    curve: loop_id,
                    span,
                    side,
                    parameter,
                }),
                evaluate_curve(curve, parameter),
            )
        };
        let (start, start_point) = target(&editor, 0);
        let (end, end_point) = target(&editor, 4);
        let separator = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![start_point, Point2::default(), end_point]).unwrap(),
                OpenCurvePurpose::SubdomainSeparator {
                    material: DEFAULT_MATERIAL,
                },
                Some(start),
                Some(end),
            )
            .unwrap();
        settle(&mut editor);
        let host_links = |editor: &TopologyEditor| {
            editor
                .document
                .model
                .draft
                .geometry
                .curve(loop_id)
                .unwrap()
                .nodes
                .iter()
                .filter_map(|node| node.vertex)
                .collect::<Vec<_>>()
        };
        let before = host_links(&editor);
        assert_eq!(before.len(), 2, "both separator ends sit on the loop");
        let vertex = editor
            .document
            .model
            .draft
            .geometry
            .curve(separator.curve)
            .unwrap()
            .nodes[0]
            .vertex
            .unwrap();

        editor.detach_endpoint(separator.curve, 0).unwrap();
        settle(&mut editor);
        assert_eq!(
            host_links(&editor),
            before,
            "the loop keeps its breakpoint while the separator tip is free"
        );
        assert!(
            editor
                .document
                .model
                .draft
                .geometry
                .vertices
                .iter()
                .any(|candidate| candidate.id == vertex),
            "the junction survives for re-attachment"
        );

        editor
            .attach_endpoint(
                separator.curve,
                0,
                TopologyAttachment::Junction {
                    vertex,
                    face: interior,
                },
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(host_links(&editor), before);
        assert_eq!(
            editor.document.model.draft.geometry.vertices.len(),
            2,
            "re-attaching must not add a second vertex at the same junction"
        );
    }

    /// Removing several curves in one gesture must remove all of them.    /// Removing several curves in one gesture must remove all of them. Each
    /// command invalidates the compiled draft, which the next one needs, so the
    /// caller has to revalidate between them.
    #[test]
    fn removing_several_curves_needs_revalidation_between_commands() {
        let mut editor = TopologyEditor::default();
        let first = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(-0.6, 0.0), 0.2),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        let second = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(0.6, 0.0), 0.2),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        settle(&mut editor);

        editor.remove_curve(first, None).unwrap();
        assert!(
            editor.curve_removal_choices(second).is_err(),
            "the compiled draft is stale until validation runs again"
        );
        settle(&mut editor);
        assert!(editor.curve_removal_choices(second).is_ok());
        editor.remove_curve(second, None).unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert!(editor.document.model.draft.geometry.curves.is_empty());
    }

    /// Detaching an outer-attached endpoint must take its vertex with it.    /// Detaching an outer-attached endpoint must take its vertex with it. An
    /// unreferenced outer vertex still subdivides its domain side and still
    /// draws a junction handle, so it reads as a ghost the user cannot remove.
    #[test]
    fn detaching_an_outer_endpoint_leaves_no_orphan_vertex() {
        let mut editor = TopologyEditor::default();
        let domain = editor.document.model.draft.geometry.domain;
        let edit = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    Point2::new(0.0, domain.min_y),
                    Point2::new(0.0, 0.0),
                    Point2::new(0.0, domain.max_y),
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
        assert_eq!(editor.document.model.draft.geometry.vertices.len(), 2);

        editor.detach_endpoint(edit.curve, 0).unwrap();
        settle(&mut editor);
        let orphans = |editor: &TopologyEditor| {
            let referenced = editor
                .document
                .model
                .draft
                .geometry
                .curves
                .iter()
                .flat_map(|curve| &curve.nodes)
                .filter_map(|node| node.vertex)
                .collect::<BTreeSet<_>>();
            editor
                .document
                .model
                .draft
                .geometry
                .vertices
                .iter()
                .filter(|vertex| !referenced.contains(&vertex.id))
                .count()
        };
        assert_eq!(orphans(&editor), 0, "the detached outer vertex must go");
        assert_eq!(editor.document.model.draft.geometry.vertices.len(), 1);

        // Re-attaching somewhere else must not accumulate a second ghost.
        editor
            .set_control(edit.curve, 0, Point2::new(0.35, domain.min_y))
            .unwrap();
        editor
            .attach_endpoint(
                edit.curve,
                0,
                TopologyAttachment::Boundary(FaceAnchor::Outer {
                    side: OuterSide::Bottom,
                    fraction: 0.75,
                }),
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(orphans(&editor), 0);
        assert_eq!(editor.document.model.draft.geometry.vertices.len(), 2);
    }

    /// Deleting part of a closed subdomain opens the loop into a baffle and    /// Deleting part of a closed subdomain opens the loop into a baffle and
    /// merges the interior into its neighbour.
    #[test]
    fn deleting_part_of_a_closed_subdomain_leaves_one_baffle() {
        let mut editor = TopologyEditor::default();
        let curve = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.4),
                ClosedCurvePurpose::Subdomain {
                    material: DEFAULT_MATERIAL,
                },
            )
            .unwrap();
        settle(&mut editor);
        let spans = span_ids(&editor, curve);
        let selected = [spans[2]].into_iter().collect::<BTreeSet<_>>();
        let choices = editor.span_removal_choices(&selected).unwrap();
        assert_eq!(choices.len(), 2, "background and interior both qualify");
        assert!(editor.remove_spans(&selected, None).is_err());

        let removal = editor
            .remove_spans(&selected, Some(BACKGROUND_REGION))
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(removal.pieces, vec![curve]);
        assert!(removal.promoted.is_empty());

        let remaining = editor.document.model.draft.geometry.curve(curve).unwrap();
        assert!(remaining.spline.is_open(), "the loop became a baffle");
        assert_eq!(remaining.spans.len(), spans.len() - 1);
        assert!(all_separated(&editor, curve));
        let kept = span_ids(&editor, curve)
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert!(!kept.contains(&spans[2]), "the deleted span is gone");
        assert_eq!(
            kept,
            spans
                .iter()
                .copied()
                .filter(|span| *span != spans[2])
                .collect::<BTreeSet<_>>(),
            "every other span keeps its identity"
        );
        assert_eq!(
            editor.document.model.draft.regions.len(),
            1,
            "the interior merged away"
        );
        assert_eq!(editor.history_len().0, 2);
    }

    /// Deleting the middle of an attached separator leaves two pieces, each still
    /// attached at its outer end.
    #[test]
    fn deleting_the_middle_of_a_separator_leaves_two_attached_baffles() {
        let mut editor = TopologyEditor::default();
        let domain = editor.document.model.draft.geometry.domain;
        let edit = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    Point2::new(0.0, domain.min_y),
                    Point2::new(0.0, -0.3),
                    Point2::new(0.0, 0.3),
                    Point2::new(0.0, domain.max_y),
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
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        let spans = span_ids(&editor, edit.curve);
        assert_eq!(spans.len(), 3);

        let selected = [spans[1]].into_iter().collect::<BTreeSet<_>>();
        let choices = editor.span_removal_choices(&selected).unwrap();
        let removal = editor
            .remove_spans(&selected, choices.first().copied())
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(removal.pieces.len(), 2, "a prefix and a suffix survive");
        assert_eq!(
            removal.pieces[0], edit.curve,
            "the first piece keeps the id"
        );
        assert_ne!(removal.pieces[1], edit.curve, "the second takes a fresh id");

        for piece in &removal.pieces {
            let curve = editor.document.model.draft.geometry.curve(*piece).unwrap();
            assert!(curve.spline.is_open());
            assert_eq!(curve.spans.len(), 1);
            assert!(all_separated(&editor, *piece));
            let attached = [curve.nodes.first(), curve.nodes.last()]
                .into_iter()
                .flatten()
                .filter(|node| node.vertex.is_some())
                .count();
            assert_eq!(attached, 1, "each piece keeps its outer attachment only");
        }
        assert_eq!(
            editor.document.model.draft.regions.len(),
            1,
            "the split faces merged back"
        );
    }

    fn three_span_baffle(editor: &mut TopologyEditor) -> CurveId {
        let domain = editor.document.model.draft.geometry.domain;
        let curve = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    Point2::new(0.0, domain.min_y),
                    Point2::new(0.0, -0.3),
                    Point2::new(0.0, 0.1),
                    Point2::new(0.0, 0.4),
                ])
                .unwrap(),
                OpenCurvePurpose::BoundaryBaffle,
                Some(TopologyAttachment::Boundary(FaceAnchor::Outer {
                    side: OuterSide::Bottom,
                    fraction: 0.5,
                })),
                None,
            )
            .unwrap()
            .curve;
        settle(editor);
        curve
    }

    /// Deleting a run at one end leaves a single piece, and a whole-curve
    /// selection still behaves as ordinary curve removal.
    #[test]
    fn deleting_an_end_run_and_a_whole_curve() {
        let mut editor = TopologyEditor::default();
        let curve = three_span_baffle(&mut editor);
        let spans = span_ids(&editor, curve);
        let selected = [spans[0]].into_iter().collect::<BTreeSet<_>>();
        let removal = editor.remove_spans(&selected, None).unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(removal.pieces, vec![curve]);
        assert_eq!(span_ids(&editor, curve), spans[1..].to_vec());

        let remaining = span_ids(&editor, curve)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let removal = editor.remove_spans(&remaining, None).unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert!(
            removal.pieces.is_empty(),
            "a whole selection removes the curve"
        );
        assert!(editor.document.model.draft.geometry.curves.is_empty());
    }

    /// A boundary probe follows the piece that keeps more of its path, and one
    /// left with nothing is removed and reported.
    #[test]
    fn boundary_probes_follow_the_surviving_piece() {
        let mut editor = TopologyEditor::default();
        let curve = three_span_baffle(&mut editor);
        let spans = span_ids(&editor, curve);
        let probe = |editor: &mut TopologyEditor, name: &str, covered: Vec<CurveSpanId>| {
            editor
                .create_probe(
                    name.to_owned(),
                    [248, 196, 112],
                    TopologyProbeTarget::Boundary(TopologyBoundaryProbeTarget {
                        curve,
                        spans: covered,
                        side: CurveTraceSide::Left,
                        reversed: false,
                        preset: ProbeSamplingPreset::Medium,
                    }),
                )
                .unwrap()
        };
        let crossing = probe(&mut editor, "Crossing", vec![spans[0], spans[1]]);
        let doomed = probe(&mut editor, "Doomed", vec![spans[0]]);
        settle(&mut editor);

        let selected = [spans[0]].into_iter().collect::<BTreeSet<_>>();
        let removal = editor.remove_spans(&selected, None).unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(removal.removed_probes, vec![doomed]);

        let kept = editor
            .document
            .model
            .probes
            .iter()
            .find(|candidate| candidate.id == crossing)
            .expect("the crossing probe survives on the remaining piece");
        let TopologyProbeTarget::Boundary(target) = &kept.target else {
            panic!("expected a boundary probe")
        };
        assert_eq!(target.curve, curve);
        assert_eq!(target.spans, vec![spans[1]], "the deleted span is trimmed");
        assert!(
            editor
                .document
                .model
                .probes
                .iter()
                .all(|candidate| candidate.id != doomed)
        );
    }

    /// One deletion is one history entry, and undo restores the geometry it cut.
    #[test]
    fn partial_deletion_is_one_undoable_action() {
        let mut editor = TopologyEditor::default();
        let curve = three_span_baffle(&mut editor);
        let before = editor.document.model.clone();
        let history = editor.history_len().0;
        let spans = span_ids(&editor, curve);
        let selected = [spans[1]].into_iter().collect::<BTreeSet<_>>();
        editor.remove_spans(&selected, None).unwrap();
        settle(&mut editor);
        assert_eq!(editor.history_len().0, history + 1);
        assert_ne!(editor.document.model, before);

        assert!(editor.undo());
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(
            editor.document.model, before,
            "undo restores the cut geometry exactly"
        );
    }

    /// A separator attached inside the deleted run would be left transmitting    /// A separator attached inside the deleted run would be left transmitting
    /// with a free tip, which the compiler rejects, so it is promoted too. The
    /// host is a baffle with a free end, so only the separator's own two sides
    /// merge and the two-region cap is not what is under test here.
    #[test]
    fn deleting_a_run_promotes_the_curve_it_frees() {
        let mut editor = TopologyEditor::default();
        let domain = editor.document.model.draft.geometry.domain;
        let host = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    Point2::new(0.0, domain.min_y),
                    Point2::new(0.0, -0.3),
                    Point2::new(0.0, 0.1),
                    Point2::new(0.0, 0.4),
                ])
                .unwrap(),
                OpenCurvePurpose::BoundaryBaffle,
                Some(TopologyAttachment::Boundary(FaceAnchor::Outer {
                    side: OuterSide::Bottom,
                    fraction: 0.5,
                })),
                None,
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(
            editor.document.model.draft.regions.len(),
            1,
            "a slit with a free tip splits nothing"
        );

        let middle = span_ids(&editor, host.curve)[1];
        let curve = editor
            .document
            .model
            .draft
            .geometry
            .curve(host.curve)
            .unwrap();
        let [low, high] = curve.spline.span_bounds(1).unwrap();
        let parameter = (low + high) * 0.5;
        let attachment_point = evaluate_curve(curve, parameter);
        let side = [CurveTraceSide::Left, CurveTraceSide::Right]
            .into_iter()
            .find(|side| {
                FaceAnchor::Curve {
                    curve: host.curve,
                    span: middle,
                    side: *side,
                    parameter,
                }
                .resolve(&editor.compiled_accepted.topology)
                .is_ok()
            })
            .unwrap();
        let separator = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    attachment_point,
                    Point2::new(-0.4, -0.1),
                    Point2::new(domain.min_x, -0.1),
                ])
                .unwrap(),
                OpenCurvePurpose::SubdomainSeparator {
                    material: DEFAULT_MATERIAL,
                },
                Some(TopologyAttachment::Boundary(FaceAnchor::Curve {
                    curve: host.curve,
                    span: middle,
                    side,
                    parameter,
                })),
                Some(TopologyAttachment::Boundary(FaceAnchor::Outer {
                    side: OuterSide::Left,
                    fraction: 0.5,
                })),
            )
            .expect("a separator attached to the slit");
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(editor.document.model.draft.regions.len(), 2);

        // The attachment split the host's middle span, so delete the run that
        // still contains the junction between them.
        let host_spans = span_ids(&editor, host.curve);
        let selected = host_spans[1..3].iter().copied().collect::<BTreeSet<_>>();
        let choices = editor.span_removal_choices(&selected).unwrap();
        assert!(choices.len() <= 2, "expected at most two merging regions");
        let removal = match editor.remove_spans(&selected, choices.last().copied()) {
            Ok(removal) => removal,
            Err(error) => panic!("partial deletion refused: {error}"),
        };
        settle(&mut editor);
        assert_eq!(
            editor.acceptance,
            TopologyAcceptance::Valid,
            "a freed transmitting end would have made this invalid"
        );
        assert!(
            removal.promoted.contains(&separator.curve),
            "the freed separator must be reported as promoted, got {:?}",
            removal.promoted
        );
        assert!(all_separated(&editor, separator.curve));
    }

    /// A merge keeps the point source driving: its position is still meshed, so
    /// it follows the surviving region rather than being disabled.
    #[test]
    fn merging_regions_carries_the_point_source_across() {
        let build = || {
            let mut editor = TopologyEditor::default();
            let curve = editor
                .create_closed_curve(
                    PeriodicCubicSpline::rounded(Point2::default(), 0.4),
                    ClosedCurvePurpose::Subdomain {
                        material: DEFAULT_MATERIAL,
                    },
                )
                .unwrap();
            settle(&mut editor);
            let region = editor
                .enclosed_assignment(curve)
                .and_then(|face| editor.assignment_region(face))
                .unwrap();
            let mut source = editor.document.model.source;
            source.enabled = true;
            source.position = Point2::default();
            source.region = region;
            editor.set_point_source(source).unwrap();
            settle(&mut editor);
            (editor, curve)
        };

        let (mut editor, curve) = build();
        editor.remove_curve(curve, Some(BACKGROUND_REGION)).unwrap();
        settle(&mut editor);
        assert_eq!(editor.document.model.source.region, BACKGROUND_REGION);
        assert!(
            editor.document.model.source.enabled,
            "the merge keeps it driving"
        );

        let (mut editor, curve) = build();
        let span = editor
            .document
            .model
            .draft
            .geometry
            .curve(curve)
            .unwrap()
            .spans[1]
            .id;
        let selected = [span].into_iter().collect::<BTreeSet<_>>();
        editor
            .remove_spans(&selected, Some(BACKGROUND_REGION))
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(editor.document.model.source.region, BACKGROUND_REGION);
        assert!(editor.document.model.source.enabled);
    }

    /// A region that goes away must take the point source with it.    /// A region that goes away must take the point source with it. The scene
    /// still compiles without this, but preparation rejects the candidate with
    /// "references an inactive region" and the simulation stops for no visible
    /// reason.
    #[test]
    fn making_a_hole_does_not_strand_the_point_source() {
        let mut editor = TopologyEditor::default();
        let curve = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.4),
                ClosedCurvePurpose::Subdomain {
                    material: DEFAULT_MATERIAL,
                },
            )
            .unwrap();
        settle(&mut editor);
        let face = editor.enclosed_assignment(curve).expect("an enclosed face");
        let region = editor.assignment_region(face).expect("a subdomain");
        let mut source = editor.document.model.source;
        source.enabled = true;
        source.position = Point2::default();
        source.region = region;
        editor.set_point_source(source).unwrap();
        settle(&mut editor);

        editor.set_face_disposition(face, None).unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(editor.document.model.source.region, BACKGROUND_REGION);
        assert!(
            !editor.document.model.source.enabled,
            "a source left inside a hole cannot keep driving"
        );
    }

    /// A span with an excluded face on either side bounds nothing the simulation
    /// solves, which is what the Edit panel calls inactive. Reachable now that a
    /// split subdomain's halves are emptied one at a time.
    #[test]
    fn a_span_between_two_holes_reports_itself_inactive() {
        use crate::topology_viewport::span_context;
        let mut editor = TopologyEditor::default();
        let material = editor.document.model.draft.materials[0].id;
        let loop_id = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.4),
                ClosedCurvePurpose::Subdomain { material },
            )
            .unwrap();
        settle(&mut editor);
        let (start_point, start) = curve_target(&editor, loop_id, 0);
        let (end_point, end) = curve_target(&editor, loop_id, 4);
        let separator = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![start_point, Point2::default(), end_point]).unwrap(),
                OpenCurvePurpose::SubdomainSeparator { material },
                Some(start),
                Some(end),
            )
            .unwrap()
            .curve;
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);

        let inactive = |editor: &TopologyEditor, curve: CurveId| {
            let compiled = editor.compiled_draft.as_ref().unwrap();
            editor
                .document
                .model
                .draft
                .geometry
                .curve(curve)
                .unwrap()
                .spans
                .iter()
                .map(|span| {
                    span_context(compiled, span.id)
                        .is_some_and(|context| !context.left.active && !context.right.active)
                })
                .collect::<Vec<_>>()
        };
        assert!(
            inactive(&editor, separator).iter().all(|dead| !*dead),
            "the separator divides two live subdomains"
        );

        // Empty both halves; the separator then sits between two holes.
        while let Some(index) = editor
            .document
            .model
            .draft
            .face_assignments
            .iter()
            .position(|assignment| {
                assignment
                    .region
                    .is_some_and(|region| region != BACKGROUND_REGION)
            })
        {
            editor.set_face_disposition(index, None).unwrap();
            settle(&mut editor);
        }
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert!(
            inactive(&editor, separator).iter().all(|dead| *dead),
            "every separator span is now excluded on both sides"
        );
        assert!(
            inactive(&editor, loop_id).iter().all(|dead| !*dead),
            "the loop still divides the holes from the live background"
        );
    }

    /// Emptying a face walls only that face's own boundary. A span dividing two
    /// faces that both stay active is left alone, and a round trip reopens the
    /// spans whose far side is still a subdomain.
    #[test]
    fn a_face_disposition_rewalls_only_that_face() {
        let mut editor = TopologyEditor::default();
        let material = editor.document.model.draft.materials[0].id;
        let left = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(-0.45, 0.0), 0.25),
                ClosedCurvePurpose::Subdomain { material },
            )
            .unwrap();
        settle(&mut editor);
        let right = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(0.45, 0.0), 0.25),
                ClosedCurvePurpose::Subdomain { material },
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        let face = editor.enclosed_assignment(left).expect("an enclosed face");
        let behavior = |editor: &TopologyEditor, curve: CurveId| {
            editor
                .document
                .model
                .draft
                .geometry
                .curve(curve)
                .unwrap()
                .spans
                .iter()
                .map(|span| matches!(span.behavior, SpanBehavior::Transmitting))
                .collect::<Vec<_>>()
        };
        let untouched = behavior(&editor, right);
        assert!(
            untouched.iter().all(|open| *open),
            "both start transmitting"
        );

        editor.set_face_disposition(face, None).unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(editor.assignment_region(face), None);
        assert!(
            behavior(&editor, left).iter().all(|open| !*open),
            "the emptied face is walled all the way round"
        );
        assert_eq!(
            behavior(&editor, right),
            untouched,
            "the other subdomain is not touched"
        );

        editor.set_face_disposition(face, Some(material)).unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert!(
            behavior(&editor, left).iter().all(|open| *open),
            "its boundary reopens onto the background, which is active"
        );
        assert!(editor.undo(), "one entry per command");
        settle(&mut editor);
        assert_eq!(editor.assignment_region(face), None, "back to the hole");
        assert!(editor.undo());
        settle(&mut editor);
        assert!(
            editor.assignment_region(face).is_some(),
            "back to the original subdomain"
        );
        assert!(
            behavior(&editor, left).iter().all(|open| *open),
            "and to its original transmitting boundary"
        );
    }

    /// A subdomain split by a separator is two faces, and each is disposed of
    /// on its own: emptying one leaves the other alive behind a new wall.
    #[test]
    fn half_of_a_split_subdomain_becomes_a_hole_on_its_own() {
        let mut editor = TopologyEditor::default();
        let loop_id = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.4),
                ClosedCurvePurpose::Subdomain {
                    material: DEFAULT_MATERIAL,
                },
            )
            .unwrap();
        settle(&mut editor);
        let interior = editor
            .compiled_accepted
            .topology
            .face_at(Point2::default())
            .unwrap();
        let target = |editor: &TopologyEditor, index: usize| {
            let curve = editor.document.model.draft.geometry.curve(loop_id).unwrap();
            let span = curve.spans[index].id;
            let [a, b] = curve.spline.span_bounds(index).unwrap();
            let parameter = (a + b) * 0.5;
            let side = [CurveTraceSide::Left, CurveTraceSide::Right]
                .into_iter()
                .find(|side| {
                    FaceAnchor::Curve {
                        curve: loop_id,
                        span,
                        side: *side,
                        parameter,
                    }
                    .resolve(&editor.compiled_accepted.topology)
                    .is_ok_and(|face| face == interior)
                })
                .unwrap();
            (
                TopologyAttachment::Boundary(FaceAnchor::Curve {
                    curve: loop_id,
                    span,
                    side,
                    parameter,
                }),
                evaluate_curve(curve, parameter),
            )
        };
        let (start, start_point) = target(&editor, 0);
        let (end, end_point) = target(&editor, 4);
        editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![start_point, Point2::default(), end_point]).unwrap(),
                OpenCurvePurpose::SubdomainSeparator {
                    material: DEFAULT_MATERIAL,
                },
                Some(start),
                Some(end),
            )
            .unwrap();
        settle(&mut editor);
        // Each half is its own face, so one of them becomes a hole on its own
        // and the separator between them turns into the wall that divides them.
        let faces = editor.document.model.draft.face_assignments.len();
        let half = editor
            .document
            .model
            .draft
            .face_assignments
            .iter()
            .position(|assignment| {
                assignment
                    .region
                    .is_some_and(|region| region != BACKGROUND_REGION)
            })
            .expect("a split half");
        editor.set_face_disposition(half, None).unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(
            editor.document.model.draft.face_assignments.len(),
            faces,
            "the other half keeps its own assignment"
        );
        assert_eq!(editor.assignment_region(half), None, "that half is a hole");
        assert!(
            editor
                .document
                .model
                .draft
                .face_assignments
                .iter()
                .filter(|assignment| assignment.region.is_some())
                .count()
                >= 2,
            "the background and the surviving half stay active"
        );
        let separator = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id != loop_id)
            .expect("the separator");
        assert!(
            separator
                .spans
                .iter()
                .all(|span| !matches!(span.behavior, SpanBehavior::Transmitting)),
            "the separator now walls the hole off from its neighbour"
        );
    }

    /// A closed curve must be able to change between enclosing a subdomain and
    /// enclosing a hole without being deleted and redrawn.
    #[test]
    fn a_closed_curve_switches_between_subdomain_and_hole() {
        let mut editor = TopologyEditor::default();
        let curve = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(0.1, -0.2), 0.3),
                ClosedCurvePurpose::Subdomain {
                    material: DEFAULT_MATERIAL,
                },
            )
            .unwrap();
        settle(&mut editor);
        let face = editor.enclosed_assignment(curve).expect("an enclosed face");
        let region = editor.assignment_region(face).expect("a subdomain");
        editor
            .set_volume_source(
                region,
                Some(VolumeSource {
                    region,
                    enabled: true,
                    profile: ScalarField::constant(1.0),
                    parameters: vec![],
                    signal: TimeSignal::harmonic(0.0, 1.0, 2.0, 0.0),
                }),
            )
            .unwrap();
        settle(&mut editor);

        editor.set_face_disposition(face, None).unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(editor.assignment_region(face), None, "now a hole");
        assert!(
            editor.document.model.draft.region(region).is_none(),
            "the region and its dependents go with the interior"
        );
        assert!(editor.document.model.draft.volume_sources.is_empty());
        let curve_spans = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|candidate| candidate.id == curve)
            .unwrap();
        assert!(
            curve_spans
                .spans
                .iter()
                .all(|span| matches!(span.behavior, SpanBehavior::Separated { .. })),
            "a hole separates its traces"
        );

        editor
            .set_face_disposition(face, Some(DEFAULT_MATERIAL))
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        let restored = editor.assignment_region(face).expect("a subdomain");
        assert_ne!(
            restored, region,
            "the interior takes a fresh stable identity"
        );
        let curve_spans = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|candidate| candidate.id == curve)
            .unwrap();
        assert!(
            curve_spans
                .spans
                .iter()
                .all(|span| span.behavior == SpanBehavior::Transmitting)
        );
        let frame = editor.document.model.draft.region(restored).unwrap().frame;
        assert!((frame.origin - Point2::new(0.1, -0.2)).norm() < 0.05);
    }

    /// Removing a curve between two assigned subdomains must offer both, so the
    /// caller can answer the question the command asks.
    #[test]
    fn divider_removal_reports_its_survivor_choices() {
        let mut editor = TopologyEditor::default();
        let material = editor.add_material().unwrap();
        settle(&mut editor);
        let hole = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(0.5, 0.5), 0.15),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(
            editor.curve_removal_choices(hole).unwrap(),
            vec![],
            "a hole merges no active regions and removes without a question"
        );
        assert!(editor.remove_curve(hole, None).is_ok());
        settle(&mut editor);

        let subdomain = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.4),
                ClosedCurvePurpose::Subdomain { material },
            )
            .unwrap();
        settle(&mut editor);
        let choices = editor.curve_removal_choices(subdomain).unwrap();
        assert_eq!(choices.len(), 2, "background and interior both qualify");
        assert!(editor.remove_curve(subdomain, None).is_err());
        assert!(editor.remove_curve(subdomain, Some(choices[0])).is_ok());
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
    }

    /// A region-local profile is unusable if its frame starts at the world    /// A region-local profile is unusable if its frame starts at the world
    /// origin while the region sits somewhere else.
    #[test]
    fn a_new_subdomain_frame_starts_inside_its_own_face() {
        let mut editor = TopologyEditor::default();
        let centre = Point2::new(-0.42, 0.31);
        editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(centre, 0.18),
                ClosedCurvePurpose::Subdomain {
                    material: DEFAULT_MATERIAL,
                },
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        let region = editor
            .document
            .model
            .draft
            .regions
            .iter()
            .find(|region| region.id != BACKGROUND_REGION)
            .expect("the subdomain allocated a region");
        assert!(
            (region.frame.origin - centre).norm() < 0.02,
            "frame started at {:?} instead of the face centre {centre:?}",
            region.frame.origin
        );
        assert_eq!(region.frame.angle_radians, 0.0);
        assert_eq!(region.frame.attachment, MaterialFrameAttachment::World);

        // The background keeps the frame it was authored with.
        let background = editor
            .document
            .model
            .draft
            .region(BACKGROUND_REGION)
            .unwrap();
        assert_eq!(background.frame.origin, Point2::default());
    }

    /// A separator's new daughter should centre on the face it actually owns,
    /// not on the whole parent.
    #[test]
    fn a_separator_daughter_frame_starts_inside_its_own_face() {
        let mut editor = TopologyEditor::default();
        let start = TopologyAttachment::Boundary(FaceAnchor::Outer {
            side: OuterSide::Bottom,
            fraction: 0.5,
        });
        let end = TopologyAttachment::Boundary(FaceAnchor::Outer {
            side: OuterSide::Top,
            fraction: 0.5,
        });
        let domain = editor.document.model.draft.geometry.domain;
        editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    Point2::new(0.0, domain.min_y),
                    Point2::new(0.0, 0.0),
                    Point2::new(0.0, domain.max_y),
                ])
                .unwrap(),
                OpenCurvePurpose::SubdomainSeparator {
                    material: DEFAULT_MATERIAL,
                },
                Some(start),
                Some(end),
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        let daughter = editor
            .document
            .model
            .draft
            .regions
            .iter()
            .find(|region| region.id != BACKGROUND_REGION)
            .expect("the separator allocated a region");
        assert!(
            daughter.frame.origin.x.abs() > 0.2,
            "the daughter frame stayed on the separator at {:?}",
            daughter.frame.origin
        );
        assert!(daughter.frame.origin.y.abs() < 0.05);
    }

    /// A new material that reused the background's own colour would be invisible
    /// in the Materials overlay, which is what the overlay is for.
    #[test]
    fn added_materials_take_distinguishable_colours() {
        let mut editor = TopologyEditor::default();
        let background = editor.document.model.draft.materials[0].color;
        let mut seen = vec![background];
        for _ in 0..4 {
            let id = editor.add_material().unwrap();
            settle(&mut editor);
            let color = editor
                .document
                .model
                .draft
                .materials
                .iter()
                .find(|material| material.id == id)
                .unwrap()
                .color;
            assert!(
                !seen.contains(&color),
                "material {id:?} reused an existing colour {color:?}"
            );
            seen.push(color);
        }
    }

    /// The whole selected run must end up on one straight line, not on a chain
    /// of per-span chords the way `straighten_spans` leaves it.
    #[test]
    fn straighten_selection_lays_the_whole_run_on_one_line() {
        let mut editor = TopologyEditor::default();
        let curve_id = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.5),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        settle(&mut editor);
        let spans = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == curve_id)
            .unwrap()
            .spans
            .iter()
            .map(|span| span.id)
            .collect::<Vec<_>>();
        let selected = spans[1..4].iter().copied().collect::<BTreeSet<_>>();
        editor.straighten_span_sections(&selected).unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(editor.history_len(), (2, 0));

        let curve = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == curve_id)
            .unwrap();
        assert_eq!(
            curve.spans.iter().map(|span| span.id).collect::<Vec<_>>(),
            spans,
            "span identities must survive"
        );
        let [start, _] = curve.spline.span_bounds(1).unwrap();
        let [_, end] = curve.spline.span_bounds(3).unwrap();
        let first = evaluate_curve(curve, start);
        let last = evaluate_curve(curve, end);
        let direction = last - first;
        let length = direction.norm();
        assert!(length > 0.1);
        for step in 0..=60 {
            let parameter = start + (end - start) * step as f64 / 60.0;
            let point = evaluate_curve(curve, parameter) - first;
            let offset = (direction.x * point.y - direction.y * point.x).abs() / length;
            assert!(
                offset < 1.0e-9,
                "run left the chord by {offset:.3e} at step {step}"
            );
        }
    }

    #[test]
    fn straighten_selection_needs_one_contiguous_run() {
        let mut editor = TopologyEditor::default();
        let curve_id = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.5),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        settle(&mut editor);
        let spans = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == curve_id)
            .unwrap()
            .spans
            .iter()
            .map(|span| span.id)
            .collect::<Vec<_>>();
        let split = [spans[0], spans[1], spans[4]]
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert!(!editor.selection_is_contiguous(&split));
        assert!(editor.straighten_span_sections(&split).is_err());
        let wrapping = [spans[7], spans[0], spans[1]]
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert!(
            editor.selection_is_contiguous(&wrapping),
            "a run through the seam is still contiguous"
        );
        let whole = spans.iter().copied().collect::<BTreeSet<_>>();
        assert!(!editor.selection_is_contiguous(&whole));
    }

    /// Removing the curve that created a junction must release the breakpoint it
    /// materialised, so the corner can be smoothed again.
    #[test]
    fn removing_a_separator_releases_its_host_junction() {
        let mut editor = TopologyEditor::default();
        let loop_id = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.55),
                ClosedCurvePurpose::Subdomain {
                    material: DEFAULT_MATERIAL,
                },
            )
            .unwrap();
        settle(&mut editor);
        let loop_curve = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == loop_id)
            .unwrap();
        let make_target = |span_index: usize| {
            let span = loop_curve.spans[span_index].id;
            let [a, b] = loop_curve.spline.span_bounds(span_index).unwrap();
            let parameter = (a + b) * 0.5;
            let side = [CurveTraceSide::Left, CurveTraceSide::Right]
                .into_iter()
                .find(|side| {
                    FaceAnchor::Curve {
                        curve: loop_id,
                        span,
                        side: *side,
                        parameter,
                    }
                    .resolve(&editor.compiled_accepted.topology)
                    .is_ok_and(|face| {
                        editor
                            .compiled_accepted
                            .assignments
                            .iter()
                            .any(|assignment| {
                                assignment.face == face
                                    && assignment.region != Some(BACKGROUND_REGION)
                            })
                    })
                })
                .unwrap();
            (
                evaluate_curve(loop_curve, parameter),
                TopologyAttachment::Boundary(FaceAnchor::Curve {
                    curve: loop_id,
                    span,
                    side,
                    parameter,
                }),
            )
        };
        let (start_point, start) = make_target(0);
        let (end_point, end) = make_target(4);
        let edit = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![start_point, Point2::default(), end_point]).unwrap(),
                OpenCurvePurpose::SubdomainSeparator {
                    material: DEFAULT_MATERIAL,
                },
                Some(start),
                Some(end),
            )
            .unwrap();
        settle(&mut editor);
        let junctions = |editor: &TopologyEditor| {
            editor
                .document
                .model
                .draft
                .geometry
                .curves
                .iter()
                .flat_map(|curve| &curve.nodes)
                .filter(|node| node.vertex.is_some())
                .count()
        };
        assert_eq!(junctions(&editor), 4, "two shared vertices, two arms each");

        let survivor = editor
            .document
            .model
            .draft
            .regions
            .iter()
            .map(|region| region.id)
            .find(|region| *region != BACKGROUND_REGION)
            .unwrap();
        editor.remove_curve(edit.curve, Some(survivor)).unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(
            junctions(&editor),
            0,
            "the host curve kept ghost junctions after the separator was removed"
        );
        assert!(editor.document.model.draft.geometry.vertices.is_empty());

        // The released corners can be smoothed again, which is what the ghosts blocked.
        let curve = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == loop_id)
            .unwrap();
        let CurveSpline::Closed(spline) = &curve.spline else {
            panic!("expected a closed curve")
        };
        let corner = (0..spline.intervals().len())
            .find(|node| spline.continuity(*node) == Some(0))
            .expect("the inserted C0 corners remain");
        editor.set_curve_continuity(loop_id, corner, 2).unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
    }

    #[test]
    fn straighten_selection_replaces_a_c0_run_with_one_chord() {
        let mut editor = TopologyEditor::default();
        editor
            .create_closed_curve(
                PeriodicCubicSpline::polygon(vec![
                    Point2::new(-0.3, -0.3),
                    Point2::new(0.3, -0.3),
                    Point2::new(0.3, 0.3),
                    Point2::new(-0.3, 0.3),
                ])
                .unwrap(),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        settle(&mut editor);
        let curve = &editor.document.model.draft.geometry.curves[0];
        let selected = BTreeSet::from([curve.spans[0].id, curve.spans[1].id]);
        let start = curve.spline.node_point(0).unwrap();
        let old_corner = curve.spline.node_point(1).unwrap();
        let end = curve.spline.node_point(2).unwrap();

        editor.straighten_span_sections(&selected).unwrap();

        let curve = &editor.document.model.draft.geometry.curves[0];
        let new_corner = curve.spline.node_point(1).unwrap();
        assert!((new_corner - old_corner).norm() > 0.1);
        let chord = end - start;
        for span in 0..=1 {
            let [a, b] = curve.spline.span_bounds(span).unwrap();
            for step in 0..=8 {
                let point = match &curve.spline {
                    CurveSpline::Closed(spline) => spline.evaluate(a + (b - a) * step as f64 / 8.0),
                    _ => unreachable!(),
                };
                assert!((point - start).cross(chord).abs() < 1.0e-12);
            }
        }
    }

    /// A boundary attachment on the interior (non-background) side of one span
    /// of a closed subdomain, at that span's midpoint.
    /// Samples a curve evenly over its whole parameter range, for asserting
    /// that a structural edit left the geometry where it was.
    fn sample_curve(editor: &TopologyEditor, curve: CurveId, count: usize) -> Vec<Point2> {
        let curve = editor.document.model.draft.geometry.curve(curve).unwrap();
        let period = match &curve.spline {
            CurveSpline::Closed(spline) => spline.period(),
            CurveSpline::Open(spline) => spline.period(),
        };
        (0..count)
            .map(|index| {
                let parameter = period * index as f64 / (count - 1) as f64;
                match &curve.spline {
                    CurveSpline::Closed(spline) => spline.evaluate(parameter),
                    CurveSpline::Open(spline) => spline.evaluate(parameter),
                }
            })
            .collect()
    }

    fn curve_target(
        editor: &TopologyEditor,
        loop_id: CurveId,
        span_index: usize,
    ) -> (Point2, TopologyAttachment) {
        let loop_curve = editor.document.model.draft.geometry.curve(loop_id).unwrap();
        let span = loop_curve.spans[span_index].id;
        let [a, b] = loop_curve.spline.span_bounds(span_index).unwrap();
        let parameter = (a + b) * 0.5;
        let side = [CurveTraceSide::Left, CurveTraceSide::Right]
            .into_iter()
            .find(|side| {
                FaceAnchor::Curve {
                    curve: loop_id,
                    span,
                    side: *side,
                    parameter,
                }
                .resolve(&editor.compiled_accepted.topology)
                .is_ok_and(|face| {
                    editor
                        .compiled_accepted
                        .assignments
                        .iter()
                        .any(|assignment| {
                            assignment.face == face && assignment.region != Some(BACKGROUND_REGION)
                        })
                })
            })
            .unwrap();
        (
            evaluate_curve(loop_curve, parameter),
            TopologyAttachment::Boundary(FaceAnchor::Curve {
                curve: loop_id,
                span,
                side,
                parameter,
            }),
        )
    }

    /// A closed subdomain split by a chord into two lobes. The lobe that
    /// existed first stays anchored on the loop; the new one is anchored on the
    /// chord. Returns `(loop, chord)`.
    fn figure_eight(editor: &mut TopologyEditor) -> (CurveId, CurveId) {
        let loop_id = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::default(), 0.55),
                ClosedCurvePurpose::Subdomain {
                    material: DEFAULT_MATERIAL,
                },
            )
            .unwrap();
        settle(editor);
        let (start_point, start) = curve_target(editor, loop_id, 0);
        let (end_point, end) = curve_target(editor, loop_id, 4);
        let chord = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![start_point, Point2::default(), end_point]).unwrap(),
                OpenCurvePurpose::SubdomainSeparator {
                    material: DEFAULT_MATERIAL,
                },
                Some(start),
                Some(end),
            )
            .unwrap()
            .curve;
        settle(editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        (loop_id, chord)
    }

    fn region_anchored_on(editor: &TopologyEditor, curve: CurveId) -> Option<RegionId> {
        editor
            .document
            .model
            .draft
            .face_assignments
            .iter()
            .find(|assignment| {
                matches!(assignment.anchor, FaceAnchor::Curve { curve: owner, .. } if owner == curve)
            })
            .and_then(|assignment| assignment.region)
    }

    /// Loop spans bordering `region`, in curve order.
    fn loop_spans_beside(
        editor: &TopologyEditor,
        loop_id: CurveId,
        region: RegionId,
    ) -> Vec<CurveSpanId> {
        let compiled = editor.compiled_draft.as_ref().unwrap();
        let faces = compiled
            .assignments
            .iter()
            .filter(|assignment| assignment.region == Some(region))
            .map(|assignment| assignment.face)
            .collect::<BTreeSet<_>>();
        let beside = compiled
            .topology
            .edges
            .iter()
            .filter(|edge| {
                edge.curve == Some(loop_id)
                    && (faces.contains(&edge.left) || faces.contains(&edge.right))
            })
            .filter_map(|edge| match edge.source {
                CompiledEdgeSource::Curve(span) => Some(span),
                CompiledEdgeSource::Outer(_) => None,
            })
            .collect::<BTreeSet<_>>();
        span_ids(editor, loop_id)
            .into_iter()
            .filter(|span| beside.contains(span))
            .collect()
    }

    /// The loop span starting at the first junction node, and the one before.
    fn junction_run(editor: &TopologyEditor, loop_id: CurveId) -> BTreeSet<CurveSpanId> {
        let loop_curve = editor.document.model.draft.geometry.curve(loop_id).unwrap();
        let junction = loop_curve
            .nodes
            .iter()
            .position(|node| node.vertex.is_some())
            .unwrap();
        let count = loop_curve.spans.len();
        [
            loop_curve.spans[(junction + count - 1) % count].id,
            loop_curve.spans[junction].id,
        ]
        .into_iter()
        .collect()
    }

    /// Deleting a span beside the lobe anchored through the chord must leave
    /// the other lobe — anchored on the loop, whose parameters the cut rebased —
    /// with its region and material intact.
    #[test]
    fn deleting_a_span_beside_a_loop_anchored_lobe_keeps_that_lobe() {
        let mut editor = TopologyEditor::default();
        let (loop_id, chord) = figure_eight(&mut editor);
        let chord_lobe =
            region_anchored_on(&editor, chord).expect("the new lobe anchors on the chord");
        let loop_lobe =
            region_anchored_on(&editor, loop_id).expect("the old lobe anchors on the loop");
        let glass = editor.add_material().unwrap();
        settle(&mut editor);
        editor.set_region_material(loop_lobe, glass).unwrap();
        settle(&mut editor);
        let history = editor.history_len().0;
        let target = loop_spans_beside(&editor, loop_id, chord_lobe)[0];
        let selected = [target].into_iter().collect::<BTreeSet<_>>();
        assert_eq!(
            editor.span_removal_choices(&selected).unwrap(),
            vec![BACKGROUND_REGION, chord_lobe]
        );
        editor
            .remove_spans(&selected, Some(BACKGROUND_REGION))
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        let regions = editor
            .document
            .model
            .draft
            .regions
            .iter()
            .map(|region| region.id)
            .collect::<Vec<_>>();
        assert!(regions.contains(&loop_lobe) && !regions.contains(&chord_lobe));
        assert_eq!(
            editor
                .document
                .model
                .draft
                .region(loop_lobe)
                .unwrap()
                .material,
            glass
        );
        assert_eq!(editor.history_len().0, history + 1);
        assert!(editor.undo());
        settle(&mut editor);
        assert_eq!(editor.document.model.draft.regions.len(), 3);
    }

    /// One contiguous run through a junction node frees the chord and merges
    /// all three faces; every survivor is a legal answer and the chord becomes
    /// a baffle.
    #[test]
    fn deleting_through_a_junction_offers_three_survivors_and_promotes_the_chord() {
        let mut editor = TopologyEditor::default();
        let (loop_id, _) = figure_eight(&mut editor);
        let selected = junction_run(&editor, loop_id);
        let choices = editor.span_removal_choices(&selected).unwrap();
        assert_eq!(choices.len(), 3);
        assert!(editor.remove_spans(&selected, None).is_err());
        for index in 0..3 {
            let mut editor = TopologyEditor::default();
            let (loop_id, chord) = figure_eight(&mut editor);
            let selected = junction_run(&editor, loop_id);
            let choices = editor.span_removal_choices(&selected).unwrap();
            let removal = editor
                .remove_spans(&selected, Some(choices[index]))
                .unwrap();
            settle(&mut editor);
            assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
            assert_eq!(removal.promoted, vec![chord]);
            assert!(all_separated(&editor, chord));
            assert_eq!(editor.document.model.draft.regions.len(), 1);
            assert_eq!(removal.removed_regions.len(), 2);
        }
    }

    /// Removing a baffle merges nothing and asks nothing.
    #[test]
    fn baffle_removal_asks_no_question() {
        let mut editor = TopologyEditor::default();
        let baffle = three_span_baffle(&mut editor);
        assert!(editor.curve_removal_choices(baffle).unwrap().is_empty());
        editor.remove_curve(baffle, None).unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
    }

    /// A curve bordering two separate pairs of subdomains cannot be removed in
    /// one go: the merge would happen in two places with no single survivor.
    #[test]
    fn removal_merging_in_two_places_is_refused() {
        let mut editor = TopologyEditor::default();
        let glass = editor.add_material().unwrap();
        settle(&mut editor);
        let bar = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    Point2::new(-1.0, 0.0),
                    Point2::new(0.0, 0.0),
                    Point2::new(1.0, 0.0),
                ])
                .unwrap(),
                OpenCurvePurpose::SubdomainSeparator { material: glass },
                Some(outer(OuterSide::Left, 0.5)),
                Some(outer(OuterSide::Right, 0.5)),
            )
            .unwrap()
            .curve;
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        // The bar runs left to right, so its left is up: the new glass region.
        let up = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![Point2::new(0.0, 0.0), Point2::new(0.0, 1.0)])
                    .unwrap(),
                OpenCurvePurpose::SubdomainSeparator { material: glass },
                Some(TopologyAttachment::Breakpoint {
                    curve: bar,
                    node: 1,
                    side: CurveTraceSide::Left,
                }),
                Some(outer(OuterSide::Top, 0.5)),
            )
            .unwrap()
            .curve;
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        let vertex = editor
            .document
            .model
            .draft
            .geometry
            .curve(bar)
            .unwrap()
            .nodes[1]
            .vertex
            .expect("the bar's corner became a junction");
        let compiled = editor.compiled_draft.as_ref().unwrap();
        let bottom = compiled
            .topology
            .vertices
            .iter()
            .find(|candidate| candidate.authored == Some(vertex))
            .unwrap()
            .traces
            .iter()
            .map(|trace| trace.face)
            .find(|face| {
                compiled.assignments.iter().any(|assignment| {
                    assignment.face == *face && assignment.region == Some(BACKGROUND_REGION)
                })
            })
            .unwrap();
        editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![Point2::new(0.0, 0.0), Point2::new(0.0, -1.0)])
                    .unwrap(),
                OpenCurvePurpose::SubdomainSeparator { material: glass },
                Some(TopologyAttachment::Junction {
                    vertex,
                    face: bottom,
                }),
                Some(outer(OuterSide::Bottom, 0.5)),
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(editor.document.model.draft.regions.len(), 4);

        let before = editor.document.model.clone();
        let history = editor.history_len();
        let error = editor.curve_removal_choices(bar).unwrap_err();
        assert!(error.contains("more than one place"), "{error}");
        assert!(editor.remove_curve(bar, Some(BACKGROUND_REGION)).is_err());
        assert_eq!(editor.document.model, before);
        assert_eq!(editor.history_len(), history);
        assert_eq!(editor.curve_removal_choices(up).unwrap().len(), 2);
    }

    /// Removing the loop frees both chord ends: the chord becomes a baffle
    /// instead of leaving the draft invalid.
    #[test]
    fn removing_the_loop_promotes_the_chord_to_a_baffle() {
        let mut editor = TopologyEditor::default();
        let (loop_id, chord) = figure_eight(&mut editor);
        assert_eq!(editor.curve_removal_choices(loop_id).unwrap().len(), 3);
        let removal = editor
            .remove_curve(loop_id, Some(BACKGROUND_REGION))
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(removal.promoted, vec![chord]);
        assert!(all_separated(&editor, chord));
        assert_eq!(editor.document.model.draft.geometry.curves.len(), 1);
        assert_eq!(editor.document.model.draft.regions.len(), 1);
        assert!(editor.undo());
        settle(&mut editor);
        assert_eq!(editor.document.model.draft.geometry.curves.len(), 2);
        assert_eq!(editor.document.model.draft.regions.len(), 3);
    }

    /// Two two-span baffles with all ends loose: `a` along y = 0 on the left,
    /// `b` along y = 0.3 on the right.
    fn two_baffles(editor: &mut TopologyEditor) -> (CurveId, CurveId) {
        let mut baffle = |points: Vec<Point2>| {
            let curve = editor
                .create_open_curve(
                    OpenCubicSpline::polyline(points).unwrap(),
                    OpenCurvePurpose::BoundaryBaffle,
                    None,
                    None,
                )
                .unwrap()
                .curve;
            settle(editor);
            curve
        };
        let a = baffle(vec![
            Point2::new(-0.5, 0.0),
            Point2::new(-0.3, 0.0),
            Point2::new(-0.1, 0.0),
        ]);
        let b = baffle(vec![
            Point2::new(0.1, 0.3),
            Point2::new(0.3, 0.3),
            Point2::new(0.5, 0.3),
        ]);
        (a, b)
    }

    /// Every way two loose ends can meet fuses the curves into one: the
    /// stationary curve keeps its identity and direction, the dragged one turns
    /// around when it must, and the seam is a plain corner that can be smoothed.
    #[test]
    fn welding_loose_ends_fuses_curves_in_every_orientation() {
        let wall = SpanBehavior::Separated {
            left: FaceBoundaryCondition::Reflecting,
            right: FaceBoundaryCondition::Impedance { ratio: 1.0 },
            coupling: InternalBoundaryCoupling::Independent,
        };
        // (dragged end of a, target end of b, span order as (curve, index), a reversed)
        for (dragged, target, order, reversed) in [
            (1, 0, [(0, 0), (0, 1), (1, 0), (1, 1)], false),
            (1, 1, [(1, 0), (1, 1), (0, 1), (0, 0)], true),
            (0, 1, [(1, 0), (1, 1), (0, 0), (0, 1)], false),
            (0, 0, [(0, 1), (0, 0), (1, 0), (1, 1)], true),
        ] {
            let mut editor = TopologyEditor::default();
            let (a, b) = two_baffles(&mut editor);
            for span in &mut editor
                .document
                .model
                .draft
                .geometry
                .curves
                .iter_mut()
                .find(|curve| curve.id == a)
                .unwrap()
                .spans
            {
                span.behavior = wall;
            }
            let a_spans = span_ids(&editor, a);
            let b_spans = span_ids(&editor, b);
            let history = editor.history_len().0;
            let weld = editor
                .weld_endpoint(
                    a,
                    dragged,
                    TopologyAttachment::LooseEnd {
                        curve: b,
                        endpoint: target,
                    },
                )
                .unwrap();
            settle(&mut editor);
            assert_eq!(
                editor.acceptance,
                TopologyAcceptance::Valid,
                "{dragged}->{target}"
            );
            assert_eq!(weld.curve, b);
            assert!(editor.document.model.draft.geometry.curve(a).is_none());
            let expected = order
                .iter()
                .map(|(which, index)| {
                    if *which == 0 {
                        a_spans[*index]
                    } else {
                        b_spans[*index]
                    }
                })
                .collect::<Vec<_>>();
            assert_eq!(span_ids(&editor, b), expected, "{dragged}->{target}");
            let joined = editor.document.model.draft.geometry.curve(b).unwrap();
            let CurveSpline::Open(spline) = &joined.spline else {
                panic!("welding two open curves keeps the result open");
            };
            let a_behavior = joined
                .spans
                .iter()
                .find(|span| span.id == a_spans[0])
                .unwrap()
                .behavior;
            assert_eq!(
                a_behavior,
                if reversed { wall.mirrored() } else { wall },
                "{dragged}->{target}"
            );
            assert_eq!(spline.continuity(2), Some(0), "the seam is a corner");
            assert_eq!(
                weld.seam_control,
                Some(spline.span_control_indices(1).unwrap()[3])
            );
            assert!(joined.nodes[2].vertex.is_none(), "no vertex at valence two");
            assert_eq!(editor.history_len().0, history + 1);
            editor.set_curve_continuity(b, 2, 2).unwrap();
            settle(&mut editor);
            assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        }
    }

    /// A curve's two loose ends meeting close it into a loop that owns a face;
    /// a single span has too few controls for that, and an end cannot meet
    /// itself.
    /// A loop small enough to sit on the periodic spline's four-control floor
    /// cannot spend a control on smoothing. Promotion refines it first by exact
    /// knot insertion, inside the one history entry the gesture earns.
    #[test]
    fn promoting_a_floor_bound_seam_refines_the_loop_first() {
        for (target, expected_spans) in [(1u8, 3usize), (2, 4)] {
            let mut editor = TopologyEditor::default();
            let curve = editor
                .create_open_curve(
                    OpenCubicSpline::new(
                        vec![
                            Point2::new(-0.2, 0.0),
                            Point2::new(-0.45, 0.55),
                            Point2::new(0.45, 0.55),
                            Point2::new(0.3, 0.1),
                            Point2::new(0.2, 0.0),
                        ],
                        vec![1.0, 1.0],
                    )
                    .unwrap(),
                    OpenCurvePurpose::BoundaryBaffle,
                    None,
                    None,
                )
                .unwrap()
                .curve;
            settle(&mut editor);
            editor
                .weld_endpoint(
                    curve,
                    1,
                    TopologyAttachment::LooseEnd { curve, endpoint: 0 },
                )
                .unwrap();
            settle(&mut editor);
            let loop_curve = editor.document.model.draft.geometry.curve(curve).unwrap();
            let CurveSpline::Closed(spline) = &loop_curve.spline else {
                panic!("the weld closed the curve")
            };
            assert_eq!(spline.controls().len(), 4, "the loop sits on the floor");
            assert_eq!(spline.continuity(0), Some(0), "the seam is a corner");

            let before = editor.document.model.clone();
            let history = editor.history_len().0;
            let sampled = sample_curve(&editor, curve, 401);
            let bound = editor.set_curve_continuity(curve, 0, target).unwrap();
            settle(&mut editor);
            assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
            assert_eq!(
                editor.history_len().0,
                history + 1,
                "the refinement rides inside the promotion"
            );
            let promoted = editor.document.model.draft.geometry.curve(curve).unwrap();
            assert_eq!(promoted.spans.len(), expected_spans);
            let CurveSpline::Closed(spline) = &promoted.spline else {
                panic!("still closed")
            };
            assert_eq!(
                spline.continuity(0),
                Some(target),
                "the seam reached C{target}"
            );
            assert!(
                spline.controls().len() >= 4,
                "the refined loop stays above the floor"
            );
            let moved = sampled
                .iter()
                .zip(sample_curve(&editor, curve, 401))
                .map(|(before, after)| (*before - after).norm())
                .fold(0.0, f64::max);
            assert!(
                moved <= bound + 1.0e-9,
                "the curve moves no further than the reported bound: {moved} > {bound}"
            );
            assert!(editor.undo());
            settle(&mut editor);
            assert_eq!(
                editor.document.model, before,
                "one undo restores the unrefined loop"
            );
        }
    }

    /// Refinement buys control points; it can never buy a junction the right to
    /// stop being a corner.
    #[test]
    fn a_junction_still_refuses_promotion_after_refinement_is_available() {
        let mut editor = TopologyEditor::default();
        let ring = editor
            .create_closed_curve(
                PeriodicCubicSpline::polygon(vec![
                    Point2::new(-0.4, -0.4),
                    Point2::new(0.4, -0.4),
                    Point2::new(0.0, 0.4),
                ])
                .unwrap(),
                ClosedCurvePurpose::Hole,
            )
            .unwrap();
        settle(&mut editor);
        // Hang a baffle off the ring so one of its nodes becomes a junction.
        let (point, attachment) = curve_target(&editor, ring, 0);
        editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![point, Point2::new(point.x, point.y - 0.35)])
                    .unwrap(),
                OpenCurvePurpose::BoundaryBaffle,
                Some(attachment),
                None,
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        let junction = editor
            .document
            .model
            .draft
            .geometry
            .curve(ring)
            .unwrap()
            .nodes
            .iter()
            .position(|node| node.vertex.is_some())
            .expect("the baffle planted a junction on the ring");
        let before = editor.document.model.clone();
        for target in [1u8, 2] {
            assert_eq!(
                editor.set_curve_continuity(ring, junction, target),
                Err("A topology junction must remain a C0 corner".to_owned())
            );
        }
        assert_eq!(
            editor.document.model, before,
            "a refused promotion refines nothing"
        );
    }

    /// A welded loop's seam is node zero, which the last span reaches at the
    /// period rather than at zero. Attaching there is a junction like any other.
    #[test]
    fn a_curve_attaches_to_a_closed_curves_seam() {
        let mut editor = TopologyEditor::default();
        let ring = editor
            .create_open_curve(
                OpenCubicSpline::new(
                    vec![
                        Point2::new(-0.2, 0.0),
                        Point2::new(-0.5, 0.6),
                        Point2::new(0.5, 0.6),
                        Point2::new(0.3, 0.1),
                        Point2::new(0.2, 0.0),
                    ],
                    vec![1.0, 1.0],
                )
                .unwrap(),
                OpenCurvePurpose::BoundaryBaffle,
                None,
                None,
            )
            .unwrap()
            .curve;
        settle(&mut editor);
        editor
            .weld_endpoint(
                ring,
                1,
                TopologyAttachment::LooseEnd {
                    curve: ring,
                    endpoint: 0,
                },
            )
            .unwrap();
        settle(&mut editor);
        let closed = editor.document.model.draft.geometry.curve(ring).unwrap();
        assert!(!closed.spline.is_open());
        assert!(
            closed.nodes[0].vertex.is_none(),
            "the seam carries no junction yet"
        );
        let seam = closed.spline.node_point(0).unwrap();
        let span = closed.spans[0].id;

        // Reach the seam from the span that ends there, at the period.
        let period = match &closed.spline {
            CurveSpline::Closed(spline) => spline.period(),
            CurveSpline::Open(spline) => spline.period(),
        };
        let last = closed.spans[closed.spans.len() - 1].id;
        for (name, target_span, parameter) in [
            ("from the first span", span, 0.0),
            ("from the last span", last, period),
        ] {
            let mut trial = TopologyEditor::from_document(editor.document.clone()).unwrap();
            settle(&mut trial);
            let side = [CurveTraceSide::Left, CurveTraceSide::Right]
                .into_iter()
                .find(|side| {
                    FaceAnchor::Curve {
                        curve: ring,
                        span: target_span,
                        side: *side,
                        parameter,
                    }
                    .resolve(&trial.compiled_accepted.topology)
                    .is_ok()
                })
                .unwrap_or_else(|| panic!("{name}: neither side resolves"));
            trial
                .create_open_curve(
                    OpenCubicSpline::polyline(vec![seam, Point2::new(seam.x, seam.y - 0.4)])
                        .unwrap(),
                    OpenCurvePurpose::BoundaryBaffle,
                    Some(TopologyAttachment::Boundary(FaceAnchor::Curve {
                        curve: ring,
                        span: target_span,
                        side,
                        parameter,
                    })),
                    None,
                )
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            settle(&mut trial);
            assert_eq!(trial.acceptance, TopologyAcceptance::Valid, "{name}");
            let ringed = trial.document.model.draft.geometry.curve(ring).unwrap();
            assert!(
                ringed.nodes[0].vertex.is_some(),
                "{name}: the seam became the junction"
            );
            assert_eq!(
                ringed.spans.len(),
                closed.spans.len(),
                "{name}: attaching at a node splits nothing"
            );
        }
    }

    /// The delete action reports its own refusal ahead of the click, so the UI
    /// can disable it rather than let the user discover the wall.
    #[test]
    fn control_deletion_reports_its_refusal_before_the_click() {
        let mut editor = TopologyEditor::default();
        let minimal = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![Point2::new(-0.3, 0.2), Point2::new(0.3, 0.2)])
                    .unwrap(),
                OpenCurvePurpose::BoundaryBaffle,
                None,
                None,
            )
            .unwrap()
            .curve;
        settle(&mut editor);
        for control in 0..4 {
            let refusal = editor
                .control_removal_error(minimal, control)
                .unwrap_or_else(|| panic!("control {control} of a minimal curve is not deletable"));
            assert!(refusal.contains("four"), "{refusal}");
        }

        let roomy = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    Point2::new(-0.3, -0.2),
                    Point2::new(0.0, -0.3),
                    Point2::new(0.3, -0.2),
                ])
                .unwrap(),
                OpenCurvePurpose::BoundaryBaffle,
                None,
                None,
            )
            .unwrap()
            .curve;
        settle(&mut editor);
        assert_eq!(
            editor.control_removal_error(roomy, 2),
            None,
            "an interior control of a two-span curve deletes"
        );
        assert!(
            editor.control_removal_error(roomy, 99).is_some(),
            "a control that does not exist is refused"
        );
        // The prediction is the command: what it allows, the command performs.
        assert!(editor.remove_control(roomy, 2).is_ok());
    }

    #[test]
    fn welding_a_curve_onto_itself_closes_a_loop() {
        let mut editor = TopologyEditor::default();
        let curve = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    Point2::new(-0.3, -0.3),
                    Point2::new(0.3, -0.3),
                    Point2::new(0.3, 0.3),
                ])
                .unwrap(),
                OpenCurvePurpose::BoundaryBaffle,
                None,
                None,
            )
            .unwrap()
            .curve;
        settle(&mut editor);
        let spans = span_ids(&editor, curve);
        let history = editor.history_len().0;
        let weld = editor
            .weld_endpoint(
                curve,
                1,
                TopologyAttachment::LooseEnd { curve, endpoint: 0 },
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(weld.curve, curve);
        assert!(weld.seam_control.is_some());
        assert!(
            !editor
                .document
                .model
                .draft
                .geometry
                .curve(curve)
                .unwrap()
                .spline
                .is_open()
        );
        assert_eq!(span_ids(&editor, curve), spans);
        assert_eq!(
            editor.document.model.draft.regions.len(),
            2,
            "the loop encloses a face"
        );
        assert_eq!(editor.history_len().0, history + 1);

        let mut editor = TopologyEditor::default();
        let single = editor
            .create_open_curve(
                OpenCubicSpline::new(
                    vec![
                        Point2::new(-0.2, 0.0),
                        Point2::new(-0.45, 0.55),
                        Point2::new(0.45, 0.55),
                        Point2::new(0.2, 0.0),
                    ],
                    vec![1.0],
                )
                .unwrap(),
                OpenCurvePurpose::BoundaryBaffle,
                None,
                None,
            )
            .unwrap()
            .curve;
        settle(&mut editor);
        let before = editor.document.model.clone();
        assert_eq!(
            editor
                .weld_endpoint(
                    single,
                    1,
                    TopologyAttachment::LooseEnd {
                        curve: single,
                        endpoint: 1,
                    },
                )
                .unwrap_err(),
            "Choose a different end to weld to"
        );
        assert_eq!(editor.document.model, before);

        // One span cannot span a loop, so the weld refines the curve first.
        // Knot insertion is exact, so the closed curve runs where the open one
        // did, and the whole thing is still a single history entry.
        let history = editor.history_len().0;
        editor
            .weld_endpoint(
                single,
                1,
                TopologyAttachment::LooseEnd {
                    curve: single,
                    endpoint: 0,
                },
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        let closed = editor.document.model.draft.geometry.curve(single).unwrap();
        assert!(!closed.spline.is_open(), "the single span closed");
        assert_eq!(closed.spans.len(), 2, "refined to the smallest legal loop");
        assert_eq!(editor.history_len().0, history + 1);
        assert!(editor.undo());
        settle(&mut editor);
        assert_eq!(editor.document.model, before, "one undo restores one span");
    }

    /// A loose end dropped on a junction, a curve interior, or a vertex-less
    /// seam becomes an arm of that point; a curve cannot attach to itself.
    #[test]
    fn welding_a_loose_end_onto_junctions_interiors_and_seams() {
        let mut editor = TopologyEditor::default();
        let host = three_span_baffle(&mut editor);
        let free = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![Point2::new(0.4, 0.0), Point2::new(0.6, 0.0)])
                    .unwrap(),
                OpenCurvePurpose::BoundaryBaffle,
                None,
                None,
            )
            .unwrap()
            .curve;
        settle(&mut editor);
        let geometry = |editor: &TopologyEditor| editor.document.model.draft.geometry.clone();

        // Onto the outer junction the host hangs from.
        let vertex = geometry(&editor).curve(host).unwrap().nodes[0]
            .vertex
            .unwrap();
        let face = editor
            .compiled_draft
            .as_ref()
            .unwrap()
            .topology
            .vertices
            .iter()
            .find(|candidate| candidate.authored == Some(vertex))
            .unwrap()
            .traces[0]
            .face;
        let weld = editor
            .weld_endpoint(free, 0, TopologyAttachment::Junction { vertex, face })
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert!(weld.seam_control.is_none() && weld.span_splits.is_empty());
        assert_eq!(
            geometry(&editor).curve(free).unwrap().nodes[0].vertex,
            Some(vertex)
        );
        assert!(editor.undo());
        settle(&mut editor);

        // Onto the host's interior: a new junction splits the span.
        let host_curve = geometry(&editor).curve(host).unwrap().clone();
        let [a, b] = host_curve.spline.span_bounds(1).unwrap();
        let weld = editor
            .weld_endpoint(
                free,
                0,
                TopologyAttachment::Boundary(FaceAnchor::Curve {
                    curve: host,
                    span: host_curve.spans[1].id,
                    side: CurveTraceSide::Right,
                    parameter: (a + b) * 0.5,
                }),
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(weld.span_splits.len(), 1);
        assert_eq!(span_ids(&editor, host).len(), 4);
        let attached = geometry(&editor).curve(free).unwrap().nodes[0].vertex;
        assert!(attached.is_some());
        assert!(editor.undo());
        settle(&mut editor);

        // Onto the seam an earlier weld left: the breakpoint becomes the
        // junction, no span is split. Both baffles sit clear of the host.
        let baffle = |editor: &mut TopologyEditor, points: Vec<Point2>| {
            let curve = editor
                .create_open_curve(
                    OpenCubicSpline::polyline(points).unwrap(),
                    OpenCurvePurpose::BoundaryBaffle,
                    None,
                    None,
                )
                .unwrap()
                .curve;
            settle(editor);
            curve
        };
        let first = baffle(
            &mut editor,
            vec![
                Point2::new(0.2, 0.6),
                Point2::new(0.3, 0.6),
                Point2::new(0.4, 0.6),
            ],
        );
        let second = baffle(
            &mut editor,
            vec![
                Point2::new(0.6, 0.6),
                Point2::new(0.7, 0.6),
                Point2::new(0.8, 0.6),
            ],
        );
        editor
            .weld_endpoint(
                first,
                1,
                TopologyAttachment::LooseEnd {
                    curve: second,
                    endpoint: 0,
                },
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert!(
            geometry(&editor).curve(second).unwrap().nodes[2]
                .vertex
                .is_none()
        );
        let weld = editor
            .weld_endpoint(
                free,
                0,
                TopologyAttachment::Breakpoint {
                    curve: second,
                    node: 2,
                    side: CurveTraceSide::Left,
                },
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert!(weld.span_splits.is_empty());
        assert_eq!(span_ids(&editor, second).len(), 4);
        let seam_vertex = geometry(&editor).curve(second).unwrap().nodes[2].vertex;
        assert!(seam_vertex.is_some());
        assert_eq!(
            geometry(&editor).curve(free).unwrap().nodes[0].vertex,
            seam_vertex
        );
        let seam = geometry(&editor)
            .curve(second)
            .unwrap()
            .spline
            .node_point(2)
            .unwrap();
        assert!(
            (geometry(&editor)
                .curve(free)
                .unwrap()
                .spline
                .node_point(0)
                .unwrap()
                - seam)
                .norm()
                < 1.0e-9
        );
    }

    /// A curve is as good a target as any other, including for its own loose
    /// end: the tip may land on one of its own breakpoints or inside one of its
    /// own spans, and the result is a loop hanging off a stem.
    #[test]
    fn a_loose_end_welds_onto_its_own_curve() {
        let hook = |editor: &mut TopologyEditor| {
            let id = editor
                .create_open_curve(
                    OpenCubicSpline::polyline(vec![
                        Point2::new(0.0, -0.4),
                        Point2::new(0.0, 0.0),
                        Point2::new(0.0, 0.3),
                        Point2::new(0.25, 0.15),
                        Point2::new(0.06, 0.03),
                    ])
                    .unwrap(),
                    OpenCurvePurpose::BoundaryBaffle,
                    None,
                    None,
                )
                .unwrap()
                .curve;
            settle(editor);
            id
        };

        // Onto one of its own vertex-less breakpoints: no span is split.
        let mut editor = TopologyEditor::default();
        settle(&mut editor);
        let id = hook(&mut editor);
        let spans_before = editor
            .document
            .model
            .draft
            .geometry
            .curve(id)
            .unwrap()
            .spans
            .len();
        let history = editor.history_len().0;
        editor
            .weld_endpoint(
                id,
                1,
                TopologyAttachment::Breakpoint {
                    curve: id,
                    node: 1,
                    side: CurveTraceSide::Left,
                },
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        let curve = editor.document.model.draft.geometry.curve(id).unwrap();
        assert_eq!(curve.spans.len(), spans_before, "a node needs no split");
        let vertex = curve.nodes[1]
            .vertex
            .expect("the breakpoint became a junction");
        assert_eq!(
            curve.nodes.last().unwrap().vertex,
            Some(vertex),
            "the tip joined the same junction"
        );
        assert_eq!(editor.history_len().0, history + 1);
        assert!(editor.undo());
        settle(&mut editor);
        assert!(
            editor
                .document
                .model
                .draft
                .geometry
                .curve(id)
                .unwrap()
                .nodes
                .iter()
                .all(|node| node.vertex.is_none()),
            "one undo unpicks the whole weld"
        );

        // Onto the inside of one of its own spans: that span splits, and the
        // tip is found again on the far side of the inserted node.
        let mut editor = TopologyEditor::default();
        settle(&mut editor);
        let id = hook(&mut editor);
        let curve = editor.document.model.draft.geometry.curve(id).unwrap();
        let span = curve.spans[0].id;
        let [a, b] = curve.spline.span_bounds(0).unwrap();
        let spans_before = curve.spans.len();
        editor
            .weld_endpoint(
                id,
                1,
                TopologyAttachment::Boundary(FaceAnchor::Curve {
                    curve: id,
                    span,
                    side: CurveTraceSide::Left,
                    parameter: (a + b) * 0.5,
                }),
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        let curve = editor.document.model.draft.geometry.curve(id).unwrap();
        assert_eq!(curve.spans.len(), spans_before + 1, "the span split");
        let vertex = curve.nodes[1]
            .vertex
            .expect("the split node is the junction");
        assert_eq!(
            curve.nodes.last().unwrap().vertex,
            Some(vertex),
            "the tip landed on it, not on a stale index"
        );
    }

    /// Reversing the absorbed curve keeps every dependency on the geometry it
    /// described: anchors flip side and map their parameter, boundary probes
    /// reverse their path, flip side, and toggle their direction.
    #[test]
    fn join_maps_anchors_and_probes_of_a_reversed_curve() {
        let mut editor = TopologyEditor::default();
        let (a, b) = two_baffles(&mut editor);
        let mut model = editor.document.model.clone();
        let a_spans = span_ids(&editor, a);
        let a_period = open_period(model.draft.geometry.curve(a).unwrap()).unwrap();
        let b_period = open_period(model.draft.geometry.curve(b).unwrap()).unwrap();
        let probed = 0.25 * a_period;
        model.draft.face_assignments.push(AuthoredFaceAssignment {
            anchor: FaceAnchor::Curve {
                curve: a,
                span: a_spans[0],
                side: CurveTraceSide::Left,
                parameter: probed,
            },
            region: Some(BACKGROUND_REGION),
        });
        model.probes.push(TopologyProbeDefinition {
            id: ProbeId(7),
            name: "wall".into(),
            color: [1, 2, 3],
            enabled: true,
            target: TopologyProbeTarget::Boundary(TopologyBoundaryProbeTarget {
                curve: a,
                spans: a_spans.clone(),
                side: CurveTraceSide::Left,
                reversed: false,
                preset: ProbeSamplingPreset::Medium,
            }),
        });
        let expected_point = evaluate_curve(model.draft.geometry.curve(a).unwrap(), probed);
        let a_last = model.draft.geometry.curve(a).unwrap().nodes.len() - 1;
        let b_last = model.draft.geometry.curve(b).unwrap().nodes.len() - 1;
        // b's end meets a's end: b stays put, a turns around and follows.
        let record = join_curves(&mut model, (b, b_last), (a, a_last)).unwrap();
        assert_eq!((record.survivor, record.absorbed), (b, a));
        let FaceAnchor::Curve {
            curve,
            span,
            side,
            parameter,
        } = model.draft.face_assignments.last().unwrap().anchor
        else {
            panic!("the anchor stays a curve anchor");
        };
        assert_eq!((curve, span, side), (b, a_spans[0], CurveTraceSide::Right));
        assert!((parameter - (b_period + a_period - probed)).abs() < 1.0e-9);
        let joined = model.draft.geometry.curve(b).unwrap();
        assert!((evaluate_curve(joined, parameter) - expected_point).norm() < 1.0e-9);
        let TopologyProbeTarget::Boundary(target) = &model.probes.last().unwrap().target else {
            panic!("the probe stays a boundary probe");
        };
        assert_eq!(target.curve, b);
        assert_eq!(
            target.spans,
            a_spans.iter().rev().copied().collect::<Vec<_>>()
        );
        assert_eq!(target.side, CurveTraceSide::Right);
        assert!(target.reversed);
        assert!(probe_definition_valid(
            model.probes.last().unwrap(),
            &model.draft
        ));
    }

    /// A weld the arrangement rejects leaves the document as the drag left it.
    #[test]
    fn weld_that_breaks_the_arrangement_is_refused() {
        let mut editor = TopologyEditor::default();
        let mut baffle = |points: Vec<Point2>| {
            let curve = editor
                .create_open_curve(
                    OpenCubicSpline::polyline(points).unwrap(),
                    OpenCurvePurpose::BoundaryBaffle,
                    None,
                    None,
                )
                .unwrap()
                .curve;
            settle(&mut editor);
            curve
        };
        let a = baffle(vec![Point2::new(-0.5, 0.0), Point2::new(-0.3, 0.0)]);
        baffle(vec![Point2::new(0.0, -0.3), Point2::new(0.0, 0.3)]);
        let b = baffle(vec![Point2::new(0.3, 0.0), Point2::new(0.5, 0.0)]);
        let before = editor.document.model.clone();
        let history = editor.history_len();
        let error = editor
            .weld_endpoint(
                a,
                1,
                TopologyAttachment::LooseEnd {
                    curve: b,
                    endpoint: 0,
                },
            )
            .unwrap_err();
        assert!(!error.is_empty());
        assert_eq!(editor.document.model, before);
        assert_eq!(editor.history_len(), history);
    }

    /// Cutting the loop right beside a junction leaves the arc's new end and
    /// the chord's end alone at that point: valence two, so they become one
    /// curve — and the chord keeps transmitting, because the free tip is on the
    /// wall side. One undo restores both curves and the junction.
    #[test]
    fn cutting_beside_a_junction_welds_the_arc_and_the_chord() {
        let mut editor = TopologyEditor::default();
        let (loop_id, chord) = figure_eight(&mut editor);
        let loop_curve = editor
            .document
            .model
            .draft
            .geometry
            .curve(loop_id)
            .unwrap()
            .clone();
        let junction = loop_curve
            .nodes
            .iter()
            .position(|node| node.vertex.is_some())
            .unwrap();
        let vertex = loop_curve.nodes[junction].vertex.unwrap();
        let chord_spans = span_ids(&editor, chord);
        let selected = [loop_curve.spans[junction].id]
            .into_iter()
            .collect::<BTreeSet<_>>();
        let history = editor.history_len().0;
        let removal = editor
            .remove_spans(&selected, Some(BACKGROUND_REGION))
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(removal.joined.len(), 1);
        let record = removal.joined[0];
        assert_eq!((record.survivor, record.absorbed), (loop_id, chord));
        assert!(removal.promoted.is_empty());
        let geometry = &editor.document.model.draft.geometry;
        assert!(geometry.curve(chord).is_none());
        assert!(
            !geometry
                .vertices
                .iter()
                .any(|candidate| candidate.id == vertex)
        );
        let joined = geometry.curve(loop_id).unwrap();
        assert!(joined.spline.is_open());
        assert!(
            joined.spans.iter().any(
                |span| span.id == chord_spans[0] && span.behavior == SpanBehavior::Transmitting
            ),
            "the chord still separates the surviving lobe"
        );
        assert!(joined.nodes[record.seam_node].vertex.is_none());
        assert_eq!(editor.document.model.draft.regions.len(), 2);
        assert_eq!(editor.history_len().0, history + 1);
        editor
            .set_curve_continuity(loop_id, record.seam_node, 1)
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert!(editor.undo() && editor.undo());
        settle(&mut editor);
        let geometry = &editor.document.model.draft.geometry;
        assert_eq!(geometry.curves.len(), 2);
        assert!(
            geometry
                .vertices
                .iter()
                .any(|candidate| candidate.id == vertex)
        );
        assert_eq!(editor.document.model.draft.regions.len(), 3);
    }

    /// A two-arm junction the removal never touched keeps its arms, valence two
    /// or not: only junctions the command released are fused.
    #[test]
    fn auto_join_only_touches_junctions_the_removal_released() {
        let template = TopologyEditor::default();
        let mut model = template.document.model.clone();
        let vertex = TopologyVertexId(50);
        model.draft.geometry.vertices.push(TopologyVertex {
            id: vertex,
            location: TopologyVertexLocation::Interior(Point2::new(0.5, 0.5)),
        });
        let mut first = TopologyCurve::new(
            CurveId(50),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![Point2::new(0.2, 0.5), Point2::new(0.5, 0.5)])
                    .unwrap(),
            ),
            vec![CurveSpan {
                id: CurveSpanId(50),
                behavior: SpanBehavior::REFLECTING,
            }],
        )
        .unwrap();
        first.nodes[1].vertex = Some(vertex);
        let mut second = TopologyCurve::new(
            CurveId(51),
            CurveSpline::Open(
                OpenCubicSpline::polyline(vec![Point2::new(0.5, 0.5), Point2::new(0.5, 0.8)])
                    .unwrap(),
            ),
            vec![CurveSpan {
                id: CurveSpanId(51),
                behavior: SpanBehavior::REFLECTING,
            }],
        )
        .unwrap();
        second.nodes[0].vertex = Some(vertex);
        model.draft.geometry.curves.extend([first, second]);
        model.accepted = model.draft.clone();
        let mut editor = TopologyEditor::from_document(TopologyDocument {
            model,
            ..template.document.clone()
        })
        .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        let loop_id = editor
            .create_closed_curve(
                PeriodicCubicSpline::rounded(Point2::new(-0.4, -0.4), 0.2),
                ClosedCurvePurpose::Subdomain {
                    material: DEFAULT_MATERIAL,
                },
            )
            .unwrap();
        settle(&mut editor);
        let removal = editor
            .remove_curve(loop_id, Some(BACKGROUND_REGION))
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert!(removal.joined.is_empty());
        let geometry = &editor.document.model.draft.geometry;
        assert!(
            geometry
                .vertices
                .iter()
                .any(|candidate| candidate.id == vertex)
        );
        assert_eq!(geometry.curves.len(), 2);
    }

    /// Drawing from one loose end to another dissolves the new curve into the
    /// existing ones and the start-side curve keeps its identity; both ends on
    /// the same curve close it into a loop.
    #[test]
    fn drawing_between_loose_ends_welds_the_curves() {
        let mut editor = TopologyEditor::default();
        let (a, b) = two_baffles(&mut editor);
        let a_spans = span_ids(&editor, a);
        let b_spans = span_ids(&editor, b);
        let history = editor.history_len().0;
        let edit = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![Point2::new(-0.1, 0.0), Point2::new(0.1, 0.3)])
                    .unwrap(),
                OpenCurvePurpose::BoundaryBaffle,
                Some(TopologyAttachment::LooseEnd {
                    curve: a,
                    endpoint: 1,
                }),
                Some(TopologyAttachment::LooseEnd {
                    curve: b,
                    endpoint: 0,
                }),
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(edit.curve, a);
        assert_eq!(editor.document.model.draft.geometry.curves.len(), 1);
        let spans = span_ids(&editor, a);
        assert_eq!(spans.len(), 5);
        assert_eq!(&spans[..2], &a_spans[..]);
        assert_eq!(&spans[3..], &b_spans[..]);
        assert_eq!(editor.history_len().0, history + 1);

        let mut editor = TopologyEditor::default();
        let angle = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![
                    Point2::new(-0.3, -0.3),
                    Point2::new(0.3, -0.3),
                    Point2::new(0.3, 0.3),
                ])
                .unwrap(),
                OpenCurvePurpose::BoundaryBaffle,
                None,
                None,
            )
            .unwrap()
            .curve;
        settle(&mut editor);
        let edit = editor
            .create_open_curve(
                OpenCubicSpline::polyline(vec![Point2::new(0.3, 0.3), Point2::new(-0.3, -0.3)])
                    .unwrap(),
                OpenCurvePurpose::BoundaryBaffle,
                Some(TopologyAttachment::LooseEnd {
                    curve: angle,
                    endpoint: 1,
                }),
                Some(TopologyAttachment::LooseEnd {
                    curve: angle,
                    endpoint: 0,
                }),
            )
            .unwrap();
        settle(&mut editor);
        assert_eq!(editor.acceptance, TopologyAcceptance::Valid);
        assert_eq!(edit.curve, angle);
        let geometry = &editor.document.model.draft.geometry;
        assert_eq!(geometry.curves.len(), 1);
        assert!(!geometry.curve(angle).unwrap().spline.is_open());
        assert_eq!(span_ids(&editor, angle).len(), 3);
        assert_eq!(editor.document.model.draft.regions.len(), 2);
    }
}
