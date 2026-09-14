//! Headless document and command layer for the unified topology application.
//!
//! This module is intentionally not wired into the visible editor until its
//! semantic commands and persistence contract are complete.

use crate::editor::{FarFieldSettings, PresentationSettings, ProbeId, ProbeSamplingPreset};
use funfern_core::*;
use std::collections::BTreeSet;

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

#[derive(Clone, Debug, PartialEq)]
pub struct TopologyCurveRemoval {
    pub curve: CurveId,
    pub removed_regions: Vec<RegionId>,
    pub region_remaps: Vec<(RegionId, RegionId)>,
    pub removed_probes: Vec<ProbeId>,
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
        let mut material = Material::default_medium();
        material.id = id;
        material.name = format!("Material {}", id.0);
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
        if !source.valid() || self.document.model.draft.region(source.region).is_none() {
            return Err("Point source settings are invalid".into());
        }
        if self.document.model.source == source {
            return Ok(());
        }
        self.begin();
        self.document.model.source = source;
        self.changed();
        self.commit();
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
        if self.document.model.probes.len() >= crate::editor::MAX_PROBES {
            return Err(format!("Maximum {} probes", crate::editor::MAX_PROBES));
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
        self.begin();
        self.document.model.probes[index] = probe;
        self.changed();
        self.commit();
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
                    frame: MaterialFrame::world(),
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
        for (node, target) in [(0, start), (curve.nodes.len() - 1, end)] {
            let Some(target) = target else { continue };
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

        let topology = compile_topology(&candidate.draft.geometry, self.revision.wrapping_add(1))
            .map_err(|issue| issue.to_string())?;
        self.assign_new_faces(
            &mut candidate.draft,
            &topology,
            curve_id,
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
            curve: curve_id,
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
        prune_unused_vertex(&mut candidate.draft.geometry, vertex);
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
        let compiled = self
            .compiled_draft
            .as_ref()
            .unwrap_or(&self.compiled_accepted)
            .clone();
        let source_face = attachment_face(target, &compiled)?;
        let source_region = compiled
            .assignments
            .iter()
            .find(|assignment| assignment.face == source_face)
            .map(|assignment| assignment.region)
            .ok_or("Attachment face no longer exists")?;
        let mut candidate = self.document.model.clone();
        let mut span_splits = vec![];
        let (vertex, point) = self.materialize_attachment(
            &mut candidate.draft,
            &mut candidate.probes,
            target,
            &mut span_splits,
        )?;
        let attached = candidate
            .draft
            .geometry
            .curves
            .iter_mut()
            .find(|candidate| candidate.id == curve)
            .ok_or("Curve no longer exists")?;
        if !attached.spline.is_open() || !matches!(endpoint, 0 | 1) {
            return Err("Choose the start or end of an open curve".into());
        }
        let node = if endpoint == 0 {
            0
        } else {
            attached.nodes.len() - 1
        };
        if attached.nodes[node].vertex.is_some() {
            return Err("Endpoint is already attached".into());
        }
        attached
            .spline
            .set_node_point(node, point)
            .map_err(|error| error.to_string())?;
        attached.nodes[node].vertex = Some(vertex);

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

    /// Removes a curve and deterministically resolves any face merge. When two
    /// active regions meet across the removed curve, `keep_region` is required.
    pub fn remove_curve(
        &mut self,
        curve: CurveId,
        keep_region: Option<RegionId>,
    ) -> Result<TopologyCurveRemoval, String> {
        let compiled = self
            .compiled_draft
            .clone()
            .ok_or("Resolve the invalid draft before removing a topology curve")?;
        if !self
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .any(|candidate| candidate.id == curve)
        {
            return Err("Curve no longer exists".into());
        }
        let assignment_by_face = compiled
            .assignments
            .iter()
            .map(|assignment| (assignment.face, assignment.region))
            .collect::<std::collections::BTreeMap<_, _>>();
        let affected_faces = compiled
            .topology
            .edges
            .iter()
            .filter(|edge| edge.curve == Some(curve))
            .flat_map(|edge| [edge.left, edge.right])
            .filter(|face| assignment_by_face.contains_key(face))
            .collect::<BTreeSet<_>>();
        let active = affected_faces
            .iter()
            .filter_map(|face| assignment_by_face[face])
            .collect::<BTreeSet<_>>();
        let chosen_survivor = match active.len() {
            0 => {
                if keep_region.is_some() {
                    return Err("An inactive curve has no material region to keep".into());
                }
                None
            }
            1 => {
                let only = active.iter().next().copied().unwrap();
                if keep_region.is_some_and(|selected| selected != only) {
                    return Err("Selected surviving material is not adjacent to this curve".into());
                }
                Some(only)
            }
            _ => {
                let selected =
                    keep_region.ok_or("Choose which adjacent material survives divider removal")?;
                if !active.contains(&selected) {
                    return Err("Selected surviving material is not adjacent to this curve".into());
                }
                Some(selected)
            }
        };
        // Region 1 is the stable exterior identity used by document defaults.
        // Choosing the other material across an exterior merge transfers that
        // region's semantic state into Region 1 instead of deleting Region 1.
        let survivor = if active.contains(&BACKGROUND_REGION) {
            Some(BACKGROUND_REGION)
        } else {
            chosen_survivor
        };
        let region_remaps = match (chosen_survivor, survivor) {
            (Some(from), Some(to)) if from != to => vec![(from, to)],
            _ => vec![],
        };
        let dropped_dependencies = active
            .iter()
            .copied()
            .filter(|region| Some(*region) != chosen_survivor)
            .collect::<BTreeSet<_>>();

        let old_assignments = self
            .document
            .model
            .draft
            .face_assignments
            .iter()
            .copied()
            .map(|assignment| {
                assignment
                    .anchor
                    .resolve(&compiled.topology)
                    .map(|face| (face, assignment))
                    .map_err(|issue| issue.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut candidate = self.document.model.clone();
        candidate
            .draft
            .geometry
            .curves
            .retain(|candidate| candidate.id != curve);
        let used_vertices = candidate
            .draft
            .geometry
            .curves
            .iter()
            .flat_map(|curve| &curve.nodes)
            .filter_map(|node| node.vertex)
            .collect::<BTreeSet<_>>();
        candidate
            .draft
            .geometry
            .vertices
            .retain(|vertex| used_vertices.contains(&vertex.id));
        let topology = compile_topology(&candidate.draft.geometry, self.revision.wrapping_add(1))
            .map_err(|issue| issue.to_string())?;

        let mut by_new_face =
            std::collections::BTreeMap::<FaceId, Vec<(FaceId, AuthoredFaceAssignment)>>::new();
        for (old_face, assignment) in old_assignments {
            if matches!(
                assignment.anchor,
                FaceAnchor::Curve {
                    curve: anchor_curve,
                    ..
                } if anchor_curve == curve
            ) {
                continue;
            }
            if let Ok(face) = assignment.anchor.resolve(&topology) {
                by_new_face
                    .entry(face)
                    .or_default()
                    .push((old_face, assignment));
            }
        }
        let mut rebuilt = vec![];
        for face in &topology.faces {
            let candidates = by_new_face.remove(&face.id).unwrap_or_default();
            let merged = candidates
                .iter()
                .any(|(old_face, _)| affected_faces.contains(old_face));
            if merged || candidates.is_empty() && !affected_faces.is_empty() {
                let anchor = candidates
                    .first()
                    .map(|(_, assignment)| assignment.anchor)
                    .or_else(|| any_face_anchor(&topology, face.id))
                    .ok_or("Merged face has no stable boundary anchor")?;
                rebuilt.push(AuthoredFaceAssignment {
                    anchor,
                    region: survivor,
                });
            } else if candidates.len() == 1 {
                rebuilt.push(candidates[0].1);
            } else {
                return Err("Curve removal merged unrelated face assignments".into());
            }
        }
        candidate.draft.face_assignments = rebuilt;

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
        let removed_regions = active
            .into_iter()
            .filter(|region| Some(*region) != survivor)
            .collect::<Vec<_>>();
        candidate
            .draft
            .regions
            .retain(|region| !removed_regions.contains(&region.id));
        candidate.draft.volume_sources.retain_mut(|source| {
            if region_remaps.iter().any(|(from, _)| source.region == *from) {
                source.region = survivor.unwrap();
                true
            } else {
                !dropped_dependencies.contains(&source.region)
            }
        });
        let mut removed_probes = vec![];
        candidate.probes.retain_mut(|probe| {
            let mut retargeted_region = false;
            if let TopologyProbeTarget::AreaRegion(region) = &mut probe.target
                && let Some((_, to)) = region_remaps.iter().find(|(from, _)| *from == *region)
            {
                *region = *to;
                retargeted_region = true;
            }
            let remove = matches!(
                &probe.target,
                TopologyProbeTarget::Boundary(target) if target.curve == curve
            ) || !retargeted_region && matches!(
                probe.target,
                TopologyProbeTarget::AreaRegion(region) if dropped_dependencies.contains(&region)
            );
            if remove {
                removed_probes.push(probe.id);
            }
            !remove
        });
        candidate
            .draft
            .compile(self.revision.wrapping_add(1))
            .map_err(|issue| issue.to_string())?;

        self.begin();
        self.document.model = candidate;
        self.changed();
        self.commit();
        Ok(TopologyCurveRemoval {
            curve,
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
                        frame: if separator_material.is_some() {
                            MaterialFrame::world()
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
    }
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

fn existing_curve_vertex(
    curve: &TopologyCurve,
    geometry: &TopologyGeometry,
    parameter: f64,
) -> Option<(TopologyVertexId, Point2)> {
    let tolerance = match &curve.spline {
        CurveSpline::Closed(spline) => spline.period(),
        CurveSpline::Open(spline) => spline.period(),
    }
    .max(1.0)
        * 1.0e-10;
    curve.nodes.iter().enumerate().find_map(|(index, node)| {
        let node_parameter = curve.spline.node_parameter(index)?;
        if (node_parameter - parameter).abs() > tolerance {
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

fn prune_unused_vertex(geometry: &mut TopologyGeometry, vertex: TopologyVertexId) {
    if geometry
        .curves
        .iter()
        .flat_map(|curve| &curve.nodes)
        .any(|node| node.vertex == Some(vertex))
    {
        return;
    }
    geometry.vertices.retain(|candidate| candidate.id != vertex);
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
}
