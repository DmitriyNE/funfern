use funfern_core::*;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryControl {
    Loop(ObstacleId, usize),
    Baffle(InternalBoundaryId, usize),
}

pub const MAX_PROBES: usize = 16;
pub const MAX_SEGMENT_PROBE_POINTS: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceSettings {
    pub enabled: bool,
    pub position: Point2,
    pub amplitude: f32,
    pub width: f32,
    pub frequency_hz: f32,
    pub region: RegionId,
}

impl SourceSettings {
    pub fn valid(self) -> bool {
        self.position.finite()
            && self.amplitude.is_finite()
            && self.width.is_finite()
            && self.width > 0.0
            && self.frequency_hz.is_finite()
            && self.frequency_hz >= 0.0
    }
}

impl Default for SourceSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            position: Point2::new(-0.45, 0.0),
            amplitude: 18.0,
            width: 0.06,
            frequency_hz: 2.5,
            region: BACKGROUND_REGION,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProbeId(pub u64);

pub const DEFAULT_FAR_FIELD_INSET: f64 = 0.12;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FarFieldSettings {
    pub enabled: bool,
    pub inset: f64,
}

impl FarFieldSettings {
    pub fn valid(self) -> bool {
        self.inset.is_finite() && self.inset > 0.0 && self.inset < 1.0
    }
}

impl Default for FarFieldSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            inset: DEFAULT_FAR_FIELD_INSET,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryProbeFeature {
    Outer,
    Loop(ObstacleId),
    Baffle(InternalBoundaryId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryProbeSide {
    Domain,
    Exterior,
    Interior,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoundaryProbeTarget {
    pub feature: BoundaryProbeFeature,
    pub start_span: usize,
    pub span_count: usize,
    pub whole: bool,
    pub side: BoundaryProbeSide,
    pub reversed: bool,
    pub preset: ProbeSamplingPreset,
}

impl BoundaryProbeTarget {
    pub fn spans(self, total: usize) -> Vec<usize> {
        if total == 0 {
            return vec![];
        }
        let count = if self.whole {
            total
        } else {
            self.span_count.min(total)
        };
        (0..count)
            .map(|offset| match self.feature {
                BoundaryProbeFeature::Baffle(_) => self.start_span + offset,
                BoundaryProbeFeature::Outer | BoundaryProbeFeature::Loop(_) => {
                    (self.start_span + offset) % total
                }
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProbeTarget {
    Point(Point2),
    Segment {
        start: Point2,
        end: Point2,
        preset: ProbeSamplingPreset,
    },
    Boundary(BoundaryProbeTarget),
    AreaDisk {
        center: Point2,
        radius: f64,
    },
    AreaRegion {
        region: RegionId,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProbeSamplingPreset {
    Low,
    #[default]
    Medium,
    High,
}

impl ProbeSamplingPreset {
    pub const fn spatial_points(self) -> usize {
        match self {
            Self::Low => 32,
            Self::Medium => 64,
            Self::High => 128,
        }
    }

    pub const fn sample_rate(self) -> f64 {
        match self {
            Self::Low => 30.0,
            Self::Medium => 60.0,
            Self::High => 120.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProbeDefinition {
    pub id: ProbeId,
    pub name: String,
    pub color: [u8; 3],
    pub enabled: bool,
    pub target: ProbeTarget,
}

impl ProbeDefinition {
    pub fn valid(&self) -> bool {
        self.id.0 > 0
            && !self.name.trim().is_empty()
            && self.name.len() <= 64
            && match self.target {
                ProbeTarget::Point(point) => point.finite(),
                ProbeTarget::Segment { start, end, .. } => {
                    start.finite() && end.finite() && (end - start).norm() >= 1.0e-6
                }
                ProbeTarget::Boundary(target) => target.span_count > 0,
                ProbeTarget::AreaDisk { center, radius } => {
                    center.finite()
                        && radius.is_finite()
                        && radius > 0.0
                        && (std::f64::consts::PI * radius * radius).is_finite()
                }
                ProbeTarget::AreaRegion { region } => region.0 > 0,
            }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopKind {
    Hole,
    MaterialInterface,
    Wall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryFaceTarget {
    Outer(OuterSide),
    Hole(ObstacleId, usize),
    Baffle(InternalBoundaryId, usize, InternalBoundarySide),
}

fn outer_condition_from_face(condition: FaceBoundaryCondition) -> OuterBoundaryCondition {
    match condition {
        FaceBoundaryCondition::Reflecting => OuterBoundaryCondition::Reflecting,
        FaceBoundaryCondition::Impedance { .. } => OuterBoundaryCondition::FirstOrderOutgoing,
        FaceBoundaryCondition::SecondOrderOutgoing => OuterBoundaryCondition::SecondOrderOutgoing,
        FaceBoundaryCondition::Neumann { signal } => OuterBoundaryCondition::Neumann { signal },
        FaceBoundaryCondition::Dirichlet { signal } => OuterBoundaryCondition::Dirichlet { signal },
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    pub draft: Scene,
    pub accepted: Scene,
    pub probes: Vec<ProbeDefinition>,
    pub source: SourceSettings,
    pub far_field: FarFieldSettings,
}
impl Default for Document {
    fn default() -> Self {
        let scene = Scene::initial();
        Self {
            draft: scene.clone(),
            accepted: scene,
            probes: vec![],
            source: SourceSettings::default(),
            far_field: FarFieldSettings::default(),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Acceptance {
    Pending,
    Valid,
    Invalid(ValidationIssue),
}
pub struct Editor {
    pub document: Document,
    pub revision: u64,
    pub acceptance: Acceptance,
    undo: Vec<Document>,
    redo: Vec<Document>,
    before: Option<Document>,
    job: Option<ValidationJob>,
    next_obstacle_id: u64,
    next_internal_boundary_id: u64,
    next_region_id: u64,
    next_material_id: u64,
    next_probe_id: u64,
}
impl Default for Editor {
    fn default() -> Self {
        Self {
            document: Document::default(),
            revision: 0,
            acceptance: Acceptance::Valid,
            undo: vec![],
            redo: vec![],
            before: None,
            job: None,
            next_obstacle_id: 2,
            next_internal_boundary_id: 1,
            next_region_id: 2,
            next_material_id: 2,
            next_probe_id: 1,
        }
    }
}
impl Editor {
    pub fn loop_kind(&self, id: ObstacleId) -> Option<LoopKind> {
        self.obstacle(id).map(|obstacle| match obstacle.role {
            LoopRole::Hole { .. } => LoopKind::Hole,
            LoopRole::MaterialInterface { .. } => LoopKind::MaterialInterface,
            LoopRole::Wall { .. } => LoopKind::Wall,
        })
    }

    /// Converts a closed loop between a hole, material interface, and two-sided
    /// wall. Region creation/removal is atomic; a nonempty interior cannot be
    /// converted to a hole because its dependent geometry would become invalid.
    pub fn set_loop_kind(
        &mut self,
        id: ObstacleId,
        kind: LoopKind,
        new_region_material: MaterialId,
    ) -> Result<(), String> {
        let obstacle = self.obstacle(id).ok_or("Missing obstacle")?;
        let current = match obstacle.role {
            LoopRole::Hole { .. } => LoopKind::Hole,
            LoopRole::MaterialInterface { .. } => LoopKind::MaterialInterface,
            LoopRole::Wall { .. } => LoopKind::Wall,
        };
        if current == kind {
            return Ok(());
        }
        let old_role = obstacle.role;
        let exterior = old_role.exterior();
        if matches!(old_role, LoopRole::Hole { .. }) && kind != LoopKind::Hole {
            if self.document.draft.material(new_region_material).is_none() {
                return Err("Choose an existing interior material".into());
            }
            let interior = RegionId(self.next_region_id);
            self.next_region_id = self
                .next_region_id
                .checked_add(1)
                .ok_or("Region IDs exhausted")?;
            let role = match kind {
                LoopKind::MaterialInterface => LoopRole::MaterialInterface { exterior, interior },
                LoopKind::Wall => LoopRole::Wall { exterior, interior },
                LoopKind::Hole => unreachable!(),
            };
            self.begin();
            self.remap_loop_probe_role(id, old_role, role);
            self.document.draft.regions.push(Region {
                id: interior,
                material: new_region_material,
            });
            self.document
                .draft
                .obstacles
                .iter_mut()
                .find(|obstacle| obstacle.id == id)
                .unwrap()
                .role = role;
            self.changed();
            self.commit();
            return Ok(());
        }
        let interior = old_role.interior().ok_or("Loop has no interior region")?;
        if kind == LoopKind::Hole
            && (self
                .document
                .draft
                .obstacles
                .iter()
                .any(|child| child.id != id && child.role.exterior() == interior)
                || self
                    .document
                    .draft
                    .internal_boundaries
                    .iter()
                    .any(|boundary| boundary.region == interior))
        {
            return Err("Move or delete geometry inside this loop before making it a hole".into());
        }
        self.begin();
        let new_role = if kind == LoopKind::Hole {
            LoopRole::Hole { exterior }
        } else {
            match kind {
                LoopKind::MaterialInterface => LoopRole::MaterialInterface { exterior, interior },
                LoopKind::Wall => LoopRole::Wall { exterior, interior },
                LoopKind::Hole => unreachable!(),
            }
        };
        self.remap_loop_probe_role(id, old_role, new_role);
        if kind == LoopKind::Hole {
            self.delete_area_probes_for_region(interior);
            self.document
                .draft
                .regions
                .retain(|region| region.id != interior);
            self.document
                .draft
                .obstacles
                .iter_mut()
                .find(|obstacle| obstacle.id == id)
                .unwrap()
                .role = LoopRole::Hole { exterior };
        } else {
            self.document
                .draft
                .obstacles
                .iter_mut()
                .find(|obstacle| obstacle.id == id)
                .unwrap()
                .role = match kind {
                LoopKind::MaterialInterface => LoopRole::MaterialInterface { exterior, interior },
                LoopKind::Wall => LoopRole::Wall { exterior, interior },
                LoopKind::Hole => unreachable!(),
            };
        }
        self.changed();
        self.commit();
        Ok(())
    }

    pub fn boundary_face_condition(
        &self,
        target: BoundaryFaceTarget,
    ) -> Result<FaceBoundaryCondition, String> {
        match target {
            BoundaryFaceTarget::Outer(side) => {
                Ok(match self.document.draft.outer_boundaries.get(side) {
                    OuterBoundaryCondition::Reflecting => FaceBoundaryCondition::Reflecting,
                    OuterBoundaryCondition::FirstOrderOutgoing => {
                        FaceBoundaryCondition::Impedance { ratio: 1.0 }
                    }
                    OuterBoundaryCondition::SecondOrderOutgoing => {
                        FaceBoundaryCondition::SecondOrderOutgoing
                    }
                    OuterBoundaryCondition::Neumann { signal } => {
                        FaceBoundaryCondition::Neumann { signal }
                    }
                    OuterBoundaryCondition::Dirichlet { signal } => {
                        FaceBoundaryCondition::Dirichlet { signal }
                    }
                })
            }
            BoundaryFaceTarget::Hole(id, span) => {
                let obstacle = self.obstacle(id).ok_or("Missing obstacle")?;
                if !matches!(obstacle.role, LoopRole::Hole { .. }) {
                    return Err("The selected loop does not expose a boundary face".into());
                }
                obstacle
                    .span_conditions
                    .get(span)
                    .copied()
                    .ok_or_else(|| "Missing obstacle span".into())
            }
            BoundaryFaceTarget::Baffle(id, span, side) => {
                let law = self
                    .internal_boundary(id)
                    .ok_or("Missing internal boundary")?
                    .span_laws
                    .get(span)
                    .copied()
                    .ok_or("Missing internal-boundary span")?;
                Ok(match side {
                    InternalBoundarySide::Left => law.left,
                    InternalBoundarySide::Right => law.right,
                })
            }
        }
    }

    /// Applies one face condition to a validated set of heterogeneous boundary
    /// targets as one document revision and history action.
    pub fn set_boundary_face_conditions(
        &mut self,
        targets: &[BoundaryFaceTarget],
        condition: FaceBoundaryCondition,
    ) -> Result<(), String> {
        if !condition.valid() {
            return Err("Boundary parameters must be valid and finite".into());
        }
        let mut unique_targets = Vec::with_capacity(targets.len());
        for target in targets {
            if !unique_targets.contains(target) {
                unique_targets.push(*target);
            }
        }
        let targets = unique_targets;
        for target in &targets {
            self.boundary_face_condition(*target)?;
        }
        let changed = targets.iter().any(|target| match target {
            BoundaryFaceTarget::Outer(side) => {
                self.document.draft.outer_boundaries.get(*side)
                    != outer_condition_from_face(condition)
            }
            BoundaryFaceTarget::Hole(id, span) => {
                self.obstacle(*id).unwrap().span_conditions[*span] != condition
            }
            BoundaryFaceTarget::Baffle(id, span, side) => {
                let law = self.internal_boundary(*id).unwrap().span_laws[*span];
                !matches!(law.coupling, InternalBoundaryCoupling::Independent)
                    || match side {
                        InternalBoundarySide::Left => law.left,
                        InternalBoundarySide::Right => law.right,
                    } != condition
            }
        });
        if !changed {
            return Ok(());
        }
        self.begin();
        for target in targets {
            match target {
                BoundaryFaceTarget::Outer(side) => {
                    self.document.draft.outer_boundaries.sides[side.index()] =
                        outer_condition_from_face(condition);
                }
                BoundaryFaceTarget::Hole(id, span) => {
                    self.document
                        .draft
                        .obstacles
                        .iter_mut()
                        .find(|obstacle| obstacle.id == id)
                        .unwrap()
                        .span_conditions[span] = condition;
                }
                BoundaryFaceTarget::Baffle(id, span, side) => {
                    let law = &mut self
                        .document
                        .draft
                        .internal_boundaries
                        .iter_mut()
                        .find(|boundary| boundary.id == id)
                        .unwrap()
                        .span_laws[span];
                    if !matches!(law.coupling, InternalBoundaryCoupling::Independent) {
                        law.left = FaceBoundaryCondition::Reflecting;
                        law.right = FaceBoundaryCondition::Reflecting;
                        law.coupling = InternalBoundaryCoupling::Independent;
                    }
                    match side {
                        InternalBoundarySide::Left => law.left = condition,
                        InternalBoundarySide::Right => law.right = condition,
                    }
                }
            }
        }
        self.changed();
        self.commit();
        Ok(())
    }

    pub fn set_internal_boundary_couplings(
        &mut self,
        spans: &[(InternalBoundaryId, usize)],
        coupling: InternalBoundaryCoupling,
    ) -> Result<(), String> {
        if !coupling.valid() {
            return Err("Thin-gap stiffness must be finite and positive".into());
        }
        let mut unique_spans = Vec::with_capacity(spans.len());
        for span in spans {
            if !unique_spans.contains(span) {
                unique_spans.push(*span);
            }
        }
        let spans = unique_spans;
        for (id, span) in &spans {
            self.internal_boundary(*id)
                .ok_or("Missing internal boundary")?
                .span_laws
                .get(*span)
                .ok_or("Missing internal-boundary span")?;
        }
        if spans.iter().all(|(id, span)| {
            self.internal_boundary(*id).unwrap().span_laws[*span].coupling == coupling
        }) {
            return Ok(());
        }
        self.begin();
        for (id, span) in spans {
            let law = &mut self
                .document
                .draft
                .internal_boundaries
                .iter_mut()
                .find(|boundary| boundary.id == id)
                .unwrap()
                .span_laws[span];
            law.coupling = coupling;
            if !matches!(coupling, InternalBoundaryCoupling::Independent) {
                law.left = FaceBoundaryCondition::Reflecting;
                law.right = FaceBoundaryCondition::Reflecting;
            }
        }
        self.changed();
        self.commit();
        Ok(())
    }

    pub fn control_point(&self, control: GeometryControl) -> Option<Point2> {
        match control {
            GeometryControl::Loop(id, index) => self
                .obstacle(id)
                .and_then(|obstacle| obstacle.spline.controls().get(index))
                .copied(),
            GeometryControl::Baffle(id, index) => self
                .internal_boundary(id)
                .and_then(|boundary| boundary.spline.controls().get(index))
                .copied(),
        }
    }

    /// Updates any mixture of loop and baffle controls as one document revision.
    /// The caller owns the surrounding history transaction.
    pub fn set_control_points(
        &mut self,
        points: &[(GeometryControl, Point2)],
    ) -> Result<(), String> {
        if points.iter().any(|(_, point)| !point.finite()) {
            return Err("Control coordinates must be finite".into());
        }
        for (control, _) in points {
            if self.control_point(*control).is_none() {
                return Err("Missing selected control".into());
            }
        }
        let changed = points
            .iter()
            .any(|(control, point)| self.control_point(*control) != Some(*point));
        if !changed {
            return Ok(());
        }
        for (control, point) in points {
            match *control {
                GeometryControl::Loop(id, index) => self
                    .document
                    .draft
                    .obstacles
                    .iter_mut()
                    .find(|obstacle| obstacle.id == id)
                    .unwrap()
                    .spline
                    .set_control(index, *point)
                    .map_err(|error| error.to_string())?,
                GeometryControl::Baffle(id, index) => self
                    .document
                    .draft
                    .internal_boundaries
                    .iter_mut()
                    .find(|boundary| boundary.id == id)
                    .unwrap()
                    .spline
                    .set_control(index, *point)
                    .map_err(|error| error.to_string())?,
            }
        }
        self.changed();
        Ok(())
    }

    pub fn set_outer_boundary_condition(
        &mut self,
        side: OuterSide,
        condition: OuterBoundaryCondition,
    ) -> Result<(), String> {
        if !condition.valid() {
            return Err("Boundary signal values must be finite and frequency nonnegative".into());
        }
        if self.document.draft.outer_boundaries.get(side) == condition {
            return Ok(());
        }
        self.begin();
        self.document.draft.outer_boundaries.sides[side.index()] = condition;
        self.changed();
        self.commit();
        Ok(())
    }

    pub fn begin(&mut self) {
        if self.before.is_none() {
            self.before = Some(self.document.clone())
        }
    }
    pub fn editing(&self) -> bool {
        self.before.is_some()
    }
    pub fn changed(&mut self) {
        self.revision += 1;
        self.acceptance = Acceptance::Pending;
        self.job = None;
    }
    pub fn commit(&mut self) {
        if let Some(before) = self.before.take()
            && before != self.document
        {
            self.undo.push(before);
            if self.undo.len() > 100 {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }
    pub fn cancel(&mut self) {
        if let Some(before) = self.before.take() {
            let geometry_changed =
                before.draft != self.document.draft || before.accepted != self.document.accepted;
            self.document = before;
            if geometry_changed {
                self.changed();
            }
        }
    }
    pub fn undo(&mut self) {
        if self.editing() {
            self.cancel();
            return;
        }
        if let Some(doc) = self.undo.pop() {
            self.redo.push(self.document.clone());
            let geometry_changed =
                doc.draft != self.document.draft || doc.accepted != self.document.accepted;
            self.document = doc;
            if geometry_changed {
                self.changed();
            }
        }
    }
    pub fn redo(&mut self) {
        if self.editing() {
            return;
        }
        if let Some(doc) = self.redo.pop() {
            self.undo.push(self.document.clone());
            let geometry_changed =
                doc.draft != self.document.draft || doc.accepted != self.document.accepted;
            self.document = doc;
            if geometry_changed {
                self.changed();
            }
        }
    }
    pub fn history_len(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
    pub fn revert(&mut self) {
        self.begin();
        self.document.draft = self.document.accepted.clone();
        self.changed();
        self.commit();
    }
    pub fn validate_frame(&mut self, budget: usize) {
        if self.acceptance != Acceptance::Pending {
            return;
        }
        let job = self
            .job
            .get_or_insert_with(|| ValidationJob::new(self.document.draft.clone(), self.revision));
        if let Some(result) = job.advance(budget) {
            self.apply_validation(result);
            self.job = None;
        }
    }
    pub fn apply_validation(&mut self, result: ValidationResult) {
        if result.revision != self.revision {
            return;
        }
        if let Some(issue) = result.issue {
            self.acceptance = Acceptance::Invalid(issue)
        } else {
            self.document.accepted = self.document.draft.clone();
            if self
                .document
                .accepted
                .region(self.document.source.region)
                .is_none()
            {
                self.document.source.region = BACKGROUND_REGION;
            }
            self.acceptance = Acceptance::Valid;
        }
    }
    pub fn obstacle(&self, id: ObstacleId) -> Option<&Obstacle> {
        self.document.draft.obstacles.iter().find(|o| o.id == id)
    }

    pub fn internal_boundary(&self, id: InternalBoundaryId) -> Option<&InternalBoundary> {
        self.document
            .draft
            .internal_boundaries
            .iter()
            .find(|boundary| boundary.id == id)
    }

    pub fn create_internal_boundary(
        &mut self,
        spline: OpenCubicSpline,
        region: RegionId,
    ) -> Result<InternalBoundaryId, String> {
        if self.document.draft.region(region).is_none() {
            return Err("Missing containing region".into());
        }
        if self.document.draft.obstacles.len() + self.document.draft.internal_boundaries.len()
            >= MAX_OBSTACLES
        {
            return Err("Maximum 32 geometric features".into());
        }
        let id = InternalBoundaryId(self.next_internal_boundary_id);
        self.next_internal_boundary_id = self
            .next_internal_boundary_id
            .checked_add(1)
            .ok_or("Internal-boundary IDs exhausted")?;
        self.begin();
        self.document
            .draft
            .internal_boundaries
            .push(InternalBoundary {
                id,
                span_laws: vec![InternalBoundaryLaw::REFLECTING; spline.intervals().len()],
                spline,
                region,
            });
        self.changed();
        self.commit();
        Ok(id)
    }

    pub fn set_internal_boundary_point(
        &mut self,
        id: InternalBoundaryId,
        index: usize,
        point: Point2,
    ) -> Result<(), String> {
        let boundary = self
            .document
            .draft
            .internal_boundaries
            .iter_mut()
            .find(|boundary| boundary.id == id)
            .ok_or("Missing internal boundary")?;
        if boundary.spline.controls().get(index) == Some(&point) {
            return Ok(());
        }
        boundary
            .spline
            .set_control(index, point)
            .map_err(|error| error.to_string())?;
        self.changed();
        Ok(())
    }

    pub fn delete_internal_boundary(&mut self, id: InternalBoundaryId) {
        self.begin();
        self.delete_boundary_probes_for(BoundaryProbeFeature::Baffle(id));
        self.document
            .draft
            .internal_boundaries
            .retain(|boundary| boundary.id != id);
        self.changed();
        self.commit();
    }

    pub fn duplicate_internal_boundary(
        &mut self,
        id: InternalBoundaryId,
        offset: Point2,
    ) -> Result<InternalBoundaryId, String> {
        if self.document.draft.obstacles.len() + self.document.draft.internal_boundaries.len()
            >= MAX_OBSTACLES
        {
            return Err("Maximum 32 geometric features".into());
        }
        let source = self
            .internal_boundary(id)
            .cloned()
            .ok_or("Missing internal boundary")?;
        let controls = source
            .spline
            .controls()
            .iter()
            .map(|point| *point + offset)
            .collect();
        let spline = OpenCubicSpline::new_with_multiplicities(
            controls,
            source.spline.intervals().to_vec(),
            source.spline.multiplicities().to_vec(),
        )
        .map_err(|error| error.to_string())?;
        let new_id = InternalBoundaryId(self.next_internal_boundary_id);
        self.next_internal_boundary_id = self
            .next_internal_boundary_id
            .checked_add(1)
            .ok_or("Internal-boundary IDs exhausted")?;
        self.begin();
        self.document
            .draft
            .internal_boundaries
            .push(InternalBoundary {
                id: new_id,
                spline,
                region: source.region,
                span_laws: source.span_laws,
            });
        self.changed();
        self.commit();
        Ok(new_id)
    }

    pub fn straighten_internal_boundary(&mut self, id: InternalBoundaryId) -> Result<(), String> {
        let boundary = self
            .internal_boundary(id)
            .ok_or("Missing internal boundary")?;
        let count = boundary.spline.controls().len();
        let start = boundary.spline.controls()[0];
        let end = boundary.spline.controls()[count - 1];
        if (end - start).norm() <= f64::EPSILON {
            return Err("A straight baffle needs distinct endpoints".into());
        }
        let spline = OpenCubicSpline::new_with_multiplicities(
            (0..count)
                .map(|index| start.lerp(end, index as f64 / (count - 1) as f64))
                .collect(),
            boundary.spline.intervals().to_vec(),
            boundary.spline.multiplicities().to_vec(),
        )
        .map_err(|error| error.to_string())?;
        if spline == boundary.spline {
            return Ok(());
        }
        self.begin();
        self.document
            .draft
            .internal_boundaries
            .iter_mut()
            .find(|boundary| boundary.id == id)
            .unwrap()
            .spline = spline;
        self.changed();
        self.commit();
        Ok(())
    }

    pub fn insert_internal_boundary(
        &mut self,
        id: InternalBoundaryId,
        parameter: f64,
    ) -> Result<usize, String> {
        let boundary = self
            .internal_boundary(id)
            .ok_or("Missing internal boundary")?;
        let mut spline = boundary.spline.clone();
        let span = spline.span_index(parameter);
        let mut span_laws = boundary.span_laws.clone();
        match spline
            .insert(parameter)
            .map_err(|error| error.to_string())?
        {
            Insertion::Existing(index) => Ok(index),
            Insertion::Inserted(index) => {
                let span = span.ok_or("Inserted knot is outside the open spline")?;
                let inherited = span_laws[span];
                span_laws.insert(span + 1, inherited);
                self.begin();
                let old_total = span_laws.len() - 1;
                self.remap_boundary_probe_spans(
                    BoundaryProbeFeature::Baffle(id),
                    old_total,
                    old_total + 1,
                    |old| {
                        if old < span {
                            vec![old]
                        } else if old == span {
                            vec![span, span + 1]
                        } else {
                            vec![old + 1]
                        }
                    },
                );
                let boundary = self
                    .document
                    .draft
                    .internal_boundaries
                    .iter_mut()
                    .find(|boundary| boundary.id == id)
                    .unwrap();
                boundary.spline = spline;
                boundary.span_laws = span_laws;
                self.changed();
                self.commit();
                Ok(index)
            }
        }
    }

    /// Changes the continuity at a baffle breakpoint. Sharpening is exact;
    /// smoothing falls back to a least-squares reshape when exact knot removal
    /// is impossible. The returned value bounds the resulting displacement.
    pub fn set_internal_boundary_continuity(
        &mut self,
        id: InternalBoundaryId,
        breakpoint: usize,
        continuity: u8,
    ) -> Result<f64, String> {
        if continuity > 2 {
            return Err("Cubic continuity must be C0, C1, or C2".into());
        }
        let boundary = self
            .internal_boundary(id)
            .ok_or("Missing internal boundary")?;
        let current = boundary
            .spline
            .continuity(breakpoint)
            .ok_or("Choose an interior baffle knot")?;
        if continuity == current {
            return Ok(0.0);
        }
        let mut spline = boundary.spline.clone();
        let mut displacement_bound = 0.0;
        while spline.continuity(breakpoint).unwrap() > continuity {
            spline
                .increase_multiplicity(breakpoint)
                .map_err(|error| error.to_string())?;
        }
        while spline.continuity(breakpoint).unwrap() < continuity {
            match spline.decrease_multiplicity(breakpoint, 1.0e-10) {
                Ok(()) => {}
                Err(SplineError::NotRemovable) => {
                    displacement_bound += spline
                        .decrease_multiplicity_approximate(breakpoint)
                        .map_err(|error| error.to_string())?;
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        self.begin();
        self.document
            .draft
            .internal_boundaries
            .iter_mut()
            .find(|boundary| boundary.id == id)
            .unwrap()
            .spline = spline;
        self.changed();
        self.commit();
        Ok(displacement_bound)
    }

    /// Splits a baffle at an existing interior breakpoint. The original ID is
    /// retained by the start half and the end half receives a fresh stable ID.
    pub fn split_internal_boundary(
        &mut self,
        id: InternalBoundaryId,
        breakpoint: usize,
    ) -> Result<InternalBoundaryId, String> {
        if self.document.draft.obstacles.len() + self.document.draft.internal_boundaries.len()
            >= MAX_OBSTACLES
        {
            return Err("Maximum 32 geometric features".into());
        }
        let source = self
            .internal_boundary(id)
            .cloned()
            .ok_or("Missing internal boundary")?;
        if breakpoint == 0 || breakpoint >= source.spline.intervals().len() {
            return Err("Choose an interior baffle knot".into());
        }
        let source_spline = source.spline.clone();
        let (left, right) = source
            .spline
            .split(breakpoint)
            .map_err(|error| error.to_string())?;
        let left_laws = source.span_laws[..breakpoint].to_vec();
        let right_laws = source.span_laws[breakpoint..].to_vec();
        let new_id = InternalBoundaryId(self.next_internal_boundary_id);
        self.next_internal_boundary_id = self
            .next_internal_boundary_id
            .checked_add(1)
            .ok_or("Internal-boundary IDs exhausted")?;
        self.begin();
        let old_total = source_spline.intervals().len();
        self.document.probes.retain_mut(|probe| {
            let ProbeTarget::Boundary(mut target) = probe.target else {
                return true;
            };
            if target.feature != BoundaryProbeFeature::Baffle(id) {
                return true;
            }
            let selected = target.spans(old_total);
            let left_score = selected
                .iter()
                .filter(|span| **span < breakpoint)
                .map(|span| open_span_arc_length(&source_spline, *span))
                .sum::<f64>();
            let right_score = selected
                .iter()
                .filter(|span| **span >= breakpoint)
                .map(|span| open_span_arc_length(&source_spline, *span))
                .sum::<f64>();
            let (feature, total, indices) = if left_score >= right_score && left_score > 0.0 {
                (
                    BoundaryProbeFeature::Baffle(id),
                    breakpoint,
                    selected
                        .into_iter()
                        .filter(|span| *span < breakpoint)
                        .collect::<BTreeSet<_>>(),
                )
            } else if right_score > 0.0 {
                (
                    BoundaryProbeFeature::Baffle(new_id),
                    old_total - breakpoint,
                    selected
                        .into_iter()
                        .filter(|span| *span >= breakpoint)
                        .map(|span| span - breakpoint)
                        .collect::<BTreeSet<_>>(),
                )
            } else {
                return false;
            };
            target.feature = feature;
            if !set_boundary_probe_run(&mut target, total, &indices, false) {
                return false;
            }
            probe.target = ProbeTarget::Boundary(target);
            true
        });
        let boundary = self
            .document
            .draft
            .internal_boundaries
            .iter_mut()
            .find(|boundary| boundary.id == id)
            .unwrap();
        boundary.spline = left;
        boundary.span_laws = left_laws;
        self.document
            .draft
            .internal_boundaries
            .push(InternalBoundary {
                id: new_id,
                spline: right,
                region: source.region,
                span_laws: right_laws,
            });
        self.changed();
        self.commit();
        Ok(new_id)
    }

    /// Joins the nearest endpoints of two baffles. Reversing a curve also
    /// reverses span order and exchanges its geometrical left/right faces.
    pub fn merge_internal_boundaries(
        &mut self,
        first: InternalBoundaryId,
        second: InternalBoundaryId,
        tolerance: f64,
    ) -> Result<InternalBoundaryId, String> {
        if first == second {
            return Err("Select two different baffles".into());
        }
        let mut a = self
            .internal_boundary(first)
            .cloned()
            .ok_or("Missing first baffle")?;
        let mut b = self
            .internal_boundary(second)
            .cloned()
            .ok_or("Missing second baffle")?;
        if a.region != b.region {
            return Err("Baffles in different regions cannot be merged".into());
        }
        fn reverse(boundary: &mut InternalBoundary) {
            boundary.spline = boundary.spline.reversed();
            boundary.span_laws.reverse();
            for law in &mut boundary.span_laws {
                std::mem::swap(&mut law.left, &mut law.right);
            }
        }
        let a_start = a.spline.evaluate(0.0);
        let a_end = a.spline.evaluate(a.spline.period());
        let b_start = b.spline.evaluate(0.0);
        let b_end = b.spline.evaluate(b.spline.period());
        let choices = [
            ((a_end - b_start).norm(), false, false),
            ((a_end - b_end).norm(), false, true),
            ((a_start - b_start).norm(), true, false),
            ((a_start - b_end).norm(), true, true),
        ];
        let &(_, reverse_a, reverse_b) = choices
            .iter()
            .min_by(|left, right| left.0.total_cmp(&right.0))
            .unwrap();
        if reverse_a {
            reverse(&mut a);
        }
        if reverse_b {
            reverse(&mut b);
        }
        if !tolerance.is_finite() || tolerance < 0.0 {
            return Err("Merge tolerance must be finite and nonnegative".into());
        }
        if a.spline.controls().len() + b.spline.controls().len() - 1 > 128 {
            return Err("Merged baffle would exceed 128 controls".into());
        }
        if (a.spline.evaluate(a.spline.period()) - b.spline.evaluate(0.0)).norm() > tolerance {
            return Err("Nearest endpoints are too far apart to merge".into());
        }
        let a_count = a.spline.intervals().len();
        let b_count = b.spline.intervals().len();
        let spline = a
            .spline
            .join(b.spline, tolerance)
            .map_err(|error| error.to_string())?;
        let mut laws = a.span_laws;
        laws.extend(b.span_laws);
        self.begin();
        self.document.probes.retain_mut(|probe| {
            let ProbeTarget::Boundary(mut target) = probe.target else {
                return true;
            };
            let (old_total, offset, reversed) =
                if target.feature == BoundaryProbeFeature::Baffle(first) {
                    (a_count, 0, reverse_a)
                } else if target.feature == BoundaryProbeFeature::Baffle(second) {
                    (b_count, a_count, reverse_b)
                } else {
                    return true;
                };
            let indices = target
                .spans(old_total)
                .into_iter()
                .map(|span| {
                    let span = if reversed { old_total - 1 - span } else { span };
                    span + offset
                })
                .collect::<BTreeSet<_>>();
            target.feature = BoundaryProbeFeature::Baffle(first);
            if reversed {
                target.reversed = !target.reversed;
                target.side = match target.side {
                    BoundaryProbeSide::Left => BoundaryProbeSide::Right,
                    BoundaryProbeSide::Right => BoundaryProbeSide::Left,
                    side => side,
                };
            }
            if !set_boundary_probe_run(&mut target, a_count + b_count, &indices, false) {
                return false;
            }
            probe.target = ProbeTarget::Boundary(target);
            true
        });
        let kept = self
            .document
            .draft
            .internal_boundaries
            .iter_mut()
            .find(|boundary| boundary.id == first)
            .unwrap();
        kept.spline = spline;
        kept.span_laws = laws;
        self.document
            .draft
            .internal_boundaries
            .retain(|boundary| boundary.id != second);
        self.changed();
        self.commit();
        Ok(first)
    }

    pub fn remove_internal_boundary_point(
        &mut self,
        id: InternalBoundaryId,
        index: usize,
    ) -> Result<(), String> {
        let boundary = self
            .internal_boundary(id)
            .ok_or("Missing internal boundary")?;
        if boundary
            .spline
            .multiplicities()
            .iter()
            .any(|multiplicity| *multiplicity != 1)
        {
            return Err("Use split/merge or Undo to edit a repeated-knot baffle".into());
        }
        let mut spline = boundary.spline.clone();
        let old_control_count = spline.controls().len();
        if index >= old_control_count {
            return Err("Missing internal-boundary control".into());
        }
        let mut span_laws = boundary.span_laws.clone();
        let old_span_count = span_laws.len();
        let merge_left;
        if index <= 1 {
            merge_left = 0;
            span_laws.remove(0);
        } else if index + 2 >= old_control_count {
            merge_left = old_span_count - 2;
            span_laws.pop();
        } else {
            let left = (index - 2).min(span_laws.len() - 2);
            merge_left = left;
            if span_laws[left] != span_laws[left + 1] {
                return Err(
                    "Removal would merge spans with different boundary laws; make them equal first"
                        .into(),
                );
            }
            span_laws.remove(left + 1);
        }
        spline.remove(index).map_err(|error| error.to_string())?;
        self.begin();
        self.remap_boundary_probe_spans(
            BoundaryProbeFeature::Baffle(id),
            old_span_count,
            old_span_count - 1,
            |old| {
                if old < merge_left {
                    vec![old]
                } else if old <= merge_left + 1 {
                    vec![merge_left]
                } else {
                    vec![old - 1]
                }
            },
        );
        let boundary = self
            .document
            .draft
            .internal_boundaries
            .iter_mut()
            .find(|boundary| boundary.id == id)
            .unwrap();
        boundary.spline = spline;
        boundary.span_laws = span_laws;
        self.changed();
        self.commit();
        Ok(())
    }

    pub fn set_internal_boundary_law(
        &mut self,
        id: InternalBoundaryId,
        span: usize,
        law: InternalBoundaryLaw,
    ) -> Result<(), String> {
        if !law.valid() {
            if matches!(law.coupling, InternalBoundaryCoupling::ThinGap { .. })
                && (law.left != FaceBoundaryCondition::Reflecting
                    || law.right != FaceBoundaryCondition::Reflecting)
            {
                return Err("A thin-gap law replaces both independent face conditions".into());
            }
            return Err("Boundary-law parameters must be valid and finite".into());
        }
        let boundary = self
            .document
            .draft
            .internal_boundaries
            .iter()
            .find(|boundary| boundary.id == id)
            .ok_or("Missing internal boundary")?;
        let current = *boundary
            .span_laws
            .get(span)
            .ok_or("Missing internal-boundary span")?;
        if current == law {
            return Ok(());
        }
        self.begin();
        let boundary = self
            .document
            .draft
            .internal_boundaries
            .iter_mut()
            .find(|boundary| boundary.id == id)
            .unwrap();
        boundary.span_laws[span] = law;
        self.changed();
        self.commit();
        Ok(())
    }
    pub fn set_point(&mut self, id: ObstacleId, index: usize, p: Point2) -> Result<(), String> {
        let o = self
            .document
            .draft
            .obstacles
            .iter_mut()
            .find(|o| o.id == id)
            .ok_or("Missing obstacle")?;
        if o.spline.controls().get(index) == Some(&p) {
            return Ok(());
        }
        o.spline.set_control(index, p).map_err(|e| e.to_string())?;
        self.changed();
        Ok(())
    }
    pub fn create(&mut self, spline: PeriodicCubicSpline) -> Result<ObstacleId, String> {
        self.create_loop(
            spline,
            LoopRole::Hole {
                exterior: BACKGROUND_REGION,
            },
        )
    }

    pub fn create_region_loop(
        &mut self,
        spline: PeriodicCubicSpline,
        exterior: RegionId,
        material: MaterialId,
        wall: bool,
    ) -> Result<ObstacleId, String> {
        if self.document.draft.region(exterior).is_none() {
            return Err("Missing exterior region".into());
        }
        if self.document.draft.material(material).is_none() {
            return Err("Missing material".into());
        }
        let interior = RegionId(self.next_region_id);
        self.next_region_id = self
            .next_region_id
            .checked_add(1)
            .ok_or("Region IDs exhausted")?;
        let role = if wall {
            LoopRole::Wall { exterior, interior }
        } else {
            LoopRole::MaterialInterface { exterior, interior }
        };
        self.begin();
        self.document.draft.regions.push(Region {
            id: interior,
            material,
        });
        match self.create_loop_inner(spline, role) {
            Ok(id) => {
                self.changed();
                self.commit();
                Ok(id)
            }
            Err(error) => {
                self.cancel();
                Err(error)
            }
        }
    }

    pub fn create_loop(
        &mut self,
        spline: PeriodicCubicSpline,
        role: LoopRole,
    ) -> Result<ObstacleId, String> {
        self.begin();
        match self.create_loop_inner(spline, role) {
            Ok(id) => {
                self.changed();
                self.commit();
                Ok(id)
            }
            Err(error) => {
                self.cancel();
                Err(error)
            }
        }
    }

    fn create_loop_inner(
        &mut self,
        spline: PeriodicCubicSpline,
        role: LoopRole,
    ) -> Result<ObstacleId, String> {
        if self.document.draft.obstacles.len() >= MAX_OBSTACLES {
            return Err("Maximum 32 obstacles".into());
        }
        if self.document.draft.region(role.exterior()).is_none()
            || role
                .interior()
                .is_some_and(|id| self.document.draft.region(id).is_none())
        {
            return Err("Loop references a missing region".into());
        }
        let id = ObstacleId(self.next_obstacle_id);
        self.next_obstacle_id = self
            .next_obstacle_id
            .checked_add(1)
            .ok_or("Obstacle IDs exhausted")?;
        self.document
            .draft
            .obstacles
            .push(Obstacle::with_role(id, spline, role));
        Ok(id)
    }
    pub fn delete_obstacle(&mut self, id: ObstacleId) {
        self.begin();
        self.delete_boundary_probes_for(BoundaryProbeFeature::Loop(id));
        let removed = self
            .document
            .draft
            .obstacles
            .iter()
            .find(|loop_| loop_.id == id)
            .map(|loop_| loop_.role);
        self.document.draft.obstacles.retain(|o| o.id != id);
        if let Some(role) = removed
            && let Some(interior) = role.interior()
        {
            self.delete_area_probes_for_region(interior);
            let exterior = role.exterior();
            for child in &mut self.document.draft.obstacles {
                child.role = replace_exterior(child.role, interior, exterior);
            }
            for boundary in &mut self.document.draft.internal_boundaries {
                if boundary.region == interior {
                    boundary.region = exterior;
                }
            }
            self.document
                .draft
                .regions
                .retain(|region| region.id != interior);
        }
        self.changed();
        self.commit();
    }

    pub fn duplicate_obstacle(
        &mut self,
        id: ObstacleId,
        offset: Point2,
    ) -> Result<ObstacleId, String> {
        if self.document.draft.obstacles.len() + self.document.draft.internal_boundaries.len()
            >= MAX_OBSTACLES
        {
            return Err("Maximum 32 geometric features".into());
        }
        let source = self.obstacle(id).cloned().ok_or("Missing obstacle")?;
        let spline = PeriodicCubicSpline::new_with_multiplicities(
            source
                .spline
                .controls()
                .iter()
                .map(|point| *point + offset)
                .collect(),
            source.spline.intervals().to_vec(),
            source.spline.multiplicities().to_vec(),
        )
        .map_err(|error| error.to_string())?;
        let new_id = ObstacleId(self.next_obstacle_id);
        self.next_obstacle_id = self
            .next_obstacle_id
            .checked_add(1)
            .ok_or("Obstacle IDs exhausted")?;
        let role = if let Some(interior) = source.role.interior() {
            let old_region = self
                .document
                .draft
                .region(interior)
                .cloned()
                .ok_or("Loop references a missing interior region")?;
            let new_region = RegionId(self.next_region_id);
            self.next_region_id = self
                .next_region_id
                .checked_add(1)
                .ok_or("Region IDs exhausted")?;
            self.begin();
            self.document.draft.regions.push(Region {
                id: new_region,
                material: old_region.material,
            });
            match source.role {
                LoopRole::MaterialInterface { exterior, .. } => LoopRole::MaterialInterface {
                    exterior,
                    interior: new_region,
                },
                LoopRole::Wall { exterior, .. } => LoopRole::Wall {
                    exterior,
                    interior: new_region,
                },
                LoopRole::Hole { .. } => unreachable!(),
            }
        } else {
            self.begin();
            source.role
        };
        self.document.draft.obstacles.push(Obstacle {
            id: new_id,
            spline,
            role,
            span_conditions: source.span_conditions,
        });
        self.changed();
        self.commit();
        Ok(new_id)
    }

    pub fn add_material(&mut self) -> Result<MaterialId, String> {
        if self.document.draft.materials.len() >= MAX_MATERIALS {
            return Err("Maximum 32 materials".into());
        }
        let id = MaterialId(self.next_material_id);
        self.next_material_id = self
            .next_material_id
            .checked_add(1)
            .ok_or("Material IDs exhausted")?;
        self.begin();
        const COLORS: [[u8; 3]; 6] = [
            [77, 121, 164],
            [129, 98, 168],
            [67, 139, 112],
            [174, 113, 72],
            [153, 86, 111],
            [102, 130, 67],
        ];
        self.document.draft.materials.push(Material {
            id,
            name: format!("Material {}", id.0),
            mass_density: 1.0,
            stiffness: 1.0,
            damping: 0.0,
            color: COLORS[(id.0.saturating_sub(2) as usize) % COLORS.len()],
        });
        self.changed();
        self.commit();
        Ok(id)
    }

    pub fn delete_material(&mut self, id: MaterialId) -> Result<(), String> {
        if id == DEFAULT_MATERIAL {
            return Err("The default material cannot be deleted".into());
        }
        if self
            .document
            .draft
            .regions
            .iter()
            .any(|region| region.material == id)
        {
            return Err("Material is assigned to a region".into());
        }
        self.begin();
        self.document
            .draft
            .materials
            .retain(|material| material.id != id);
        self.changed();
        self.commit();
        Ok(())
    }

    pub fn set_region_material(
        &mut self,
        region_id: RegionId,
        material_id: MaterialId,
    ) -> Result<(), String> {
        if self.document.draft.material(material_id).is_none() {
            return Err("Missing material".into());
        }
        let current = self
            .document
            .draft
            .regions
            .iter()
            .find(|region| region.id == region_id)
            .ok_or("Missing region")?;
        if current.material == material_id {
            return Ok(());
        }
        self.begin();
        self.document
            .draft
            .regions
            .iter_mut()
            .find(|region| region.id == region_id)
            .unwrap()
            .material = material_id;
        self.changed();
        self.commit();
        Ok(())
    }

    pub fn update_material(&mut self, material: Material) -> Result<(), String> {
        if !material.valid() {
            return Err(
                "Material values must be finite; density and stiffness must be positive".into(),
            );
        }
        let material_id = material.id;
        let current = self
            .document
            .draft
            .material(material_id)
            .ok_or("Missing material")?;
        if current == &material {
            return Ok(());
        }
        self.begin();
        *self
            .document
            .draft
            .materials
            .iter_mut()
            .find(|candidate| candidate.id == material_id)
            .unwrap() = material;
        self.changed();
        self.commit();
        Ok(())
    }
    pub fn remove_point(&mut self, id: ObstacleId, index: usize) -> Result<(), String> {
        let obstacle = self.obstacle(id).ok_or("Missing obstacle")?;
        if obstacle
            .spline
            .multiplicities()
            .iter()
            .any(|multiplicity| *multiplicity != 1)
        {
            return Err("Use Undo to remove a repeated loop knot".into());
        }
        let mut spline = obstacle.spline.clone();
        let mut span_conditions = obstacle.span_conditions.clone();
        if index >= span_conditions.len() {
            return Err("Missing obstacle span".into());
        }
        let previous = (index + span_conditions.len() - 1) % span_conditions.len();
        if span_conditions[previous] != span_conditions[index] {
            return Err(
                "Removal would merge spans with different boundary conditions; make them equal first"
                    .into(),
            );
        }
        spline.remove(index).map_err(|e| e.to_string())?;
        span_conditions.remove(index);
        self.begin();
        let old_span_count = span_conditions.len() + 1;
        let previous = (index + old_span_count - 1) % old_span_count;
        let merged = if index == 0 {
            old_span_count - 2
        } else {
            index - 1
        };
        self.remap_boundary_probe_spans(
            BoundaryProbeFeature::Loop(id),
            old_span_count,
            old_span_count - 1,
            |old| {
                if old == index || old == previous {
                    vec![merged]
                } else if old > index {
                    vec![old - 1]
                } else {
                    vec![old]
                }
            },
        );
        let obstacle = self
            .document
            .draft
            .obstacles
            .iter_mut()
            .find(|o| o.id == id)
            .unwrap();
        obstacle.spline = spline;
        obstacle.span_conditions = span_conditions;
        self.changed();
        self.commit();
        Ok(())
    }
    pub fn insert(&mut self, id: ObstacleId, t: f64) -> Result<usize, String> {
        let obstacle = self.obstacle(id).ok_or("Missing obstacle")?;
        let mut spline = obstacle.spline.clone();
        let span = spline.span_index(t).ok_or("Invalid spline parameter")?;
        let inherited = obstacle.span_conditions[span];
        let old_total = obstacle.span_conditions.len();
        match spline.insert(t).map_err(|e| e.to_string())? {
            Insertion::Existing(i) => Ok(i),
            Insertion::Inserted(i) => {
                self.begin();
                self.remap_boundary_probe_spans(
                    BoundaryProbeFeature::Loop(id),
                    old_total,
                    old_total + 1,
                    |old| {
                        if old < span {
                            vec![old]
                        } else if old == span {
                            vec![span, span + 1]
                        } else {
                            vec![old + 1]
                        }
                    },
                );
                let obstacle = self
                    .document
                    .draft
                    .obstacles
                    .iter_mut()
                    .find(|o| o.id == id)
                    .unwrap();
                obstacle.spline = spline;
                obstacle.span_conditions.insert(span + 1, inherited);
                self.changed();
                self.commit();
                Ok(i)
            }
        }
    }

    /// Changes a loop breakpoint's continuity. Sharpening is exact; smoothing
    /// falls back to a least-squares reshape when exact removal is impossible.
    /// Breakpoint zero is the editable periodic seam. The returned value bounds
    /// the resulting displacement.
    pub fn set_obstacle_continuity(
        &mut self,
        id: ObstacleId,
        breakpoint: usize,
        continuity: u8,
    ) -> Result<f64, String> {
        if continuity > 2 {
            return Err("Cubic continuity must be C0, C1, or C2".into());
        }
        let obstacle = self.obstacle(id).ok_or("Missing obstacle")?;
        let current = obstacle
            .spline
            .continuity(breakpoint)
            .ok_or("Missing loop knot")?;
        if continuity == current {
            return Ok(0.0);
        }
        let mut spline = obstacle.spline.clone();
        let mut displacement_bound = 0.0;
        while spline.continuity(breakpoint).unwrap() > continuity {
            spline
                .increase_multiplicity(breakpoint)
                .map_err(|error| error.to_string())?;
        }
        while spline.continuity(breakpoint).unwrap() < continuity {
            match spline.decrease_multiplicity(breakpoint, 1.0e-10) {
                Ok(()) => {}
                Err(SplineError::NotRemovable) => {
                    displacement_bound += spline
                        .decrease_multiplicity_approximate(breakpoint)
                        .map_err(|error| error.to_string())?;
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        self.begin();
        self.document
            .draft
            .obstacles
            .iter_mut()
            .find(|obstacle| obstacle.id == id)
            .unwrap()
            .spline = spline;
        self.changed();
        self.commit();
        Ok(displacement_bound)
    }

    /// Refines several loop and baffle breakpoints to C0 as one exact history
    /// action. All splines are prepared before the document is mutated.
    pub fn isolate_span_boundaries(
        &mut self,
        loop_breakpoints: &[(ObstacleId, usize)],
        baffle_breakpoints: &[(InternalBoundaryId, usize)],
    ) -> Result<(), String> {
        let mut loop_updates = Vec::<(ObstacleId, PeriodicCubicSpline)>::new();
        for &(id, breakpoint) in loop_breakpoints {
            if let Some((_, spline)) = loop_updates
                .iter_mut()
                .find(|(candidate, _)| *candidate == id)
            {
                while spline.continuity(breakpoint).ok_or("Missing loop knot")? > 0 {
                    spline
                        .increase_multiplicity(breakpoint)
                        .map_err(|error| error.to_string())?;
                }
            } else {
                let mut spline = self.obstacle(id).ok_or("Missing obstacle")?.spline.clone();
                while spline.continuity(breakpoint).ok_or("Missing loop knot")? > 0 {
                    spline
                        .increase_multiplicity(breakpoint)
                        .map_err(|error| error.to_string())?;
                }
                loop_updates.push((id, spline));
            }
        }
        let mut baffle_updates = Vec::<(InternalBoundaryId, OpenCubicSpline)>::new();
        for &(id, breakpoint) in baffle_breakpoints {
            if let Some((_, spline)) = baffle_updates
                .iter_mut()
                .find(|(candidate, _)| *candidate == id)
            {
                while spline
                    .continuity(breakpoint)
                    .ok_or("Choose an interior baffle knot")?
                    > 0
                {
                    spline
                        .increase_multiplicity(breakpoint)
                        .map_err(|error| error.to_string())?;
                }
            } else {
                let mut spline = self
                    .internal_boundary(id)
                    .ok_or("Missing internal boundary")?
                    .spline
                    .clone();
                while spline
                    .continuity(breakpoint)
                    .ok_or("Choose an interior baffle knot")?
                    > 0
                {
                    spline
                        .increase_multiplicity(breakpoint)
                        .map_err(|error| error.to_string())?;
                }
                baffle_updates.push((id, spline));
            }
        }
        let changed = loop_updates.iter().any(|(id, spline)| {
            self.obstacle(*id)
                .is_some_and(|obstacle| obstacle.spline != *spline)
        }) || baffle_updates.iter().any(|(id, spline)| {
            self.internal_boundary(*id)
                .is_some_and(|boundary| boundary.spline != *spline)
        });
        if !changed {
            return Ok(());
        }
        self.begin();
        for (id, spline) in loop_updates {
            self.document
                .draft
                .obstacles
                .iter_mut()
                .find(|obstacle| obstacle.id == id)
                .unwrap()
                .spline = spline;
        }
        for (id, spline) in baffle_updates {
            self.document
                .draft
                .internal_boundaries
                .iter_mut()
                .find(|boundary| boundary.id == id)
                .unwrap()
                .spline = spline;
        }
        self.changed();
        self.commit();
        Ok(())
    }

    pub fn set_obstacle_boundary_condition(
        &mut self,
        id: ObstacleId,
        span: usize,
        condition: FaceBoundaryCondition,
    ) -> Result<(), String> {
        if !condition.valid() {
            return Err("Boundary parameters must be valid and finite".into());
        }
        let obstacle = self.obstacle(id).ok_or("Missing obstacle")?;
        if !matches!(obstacle.role, LoopRole::Hole { .. }) {
            return Err("Boundary conditions can currently be assigned only to holes".into());
        }
        let current = *obstacle
            .span_conditions
            .get(span)
            .ok_or("Missing obstacle span")?;
        if current == condition {
            return Ok(());
        }
        self.begin();
        self.document
            .draft
            .obstacles
            .iter_mut()
            .find(|obstacle| obstacle.id == id)
            .unwrap()
            .span_conditions[span] = condition;
        self.changed();
        self.commit();
        Ok(())
    }

    pub fn create_point_probe(&mut self, position: Point2) -> Result<ProbeId, String> {
        if !position.finite() {
            return Err("Probe position must be finite".into());
        }
        if self.document.probes.len() >= MAX_PROBES {
            return Err(format!("Maximum {MAX_PROBES} probes"));
        }
        let id = ProbeId(self.next_probe_id);
        self.next_probe_id = self
            .next_probe_id
            .checked_add(1)
            .ok_or("Probe IDs exhausted")?;
        const COLORS: [[u8; 3]; 8] = [
            [63, 144, 239],
            [244, 105, 122],
            [78, 201, 176],
            [245, 183, 69],
            [164, 126, 232],
            [70, 190, 232],
            [230, 125, 67],
            [153, 203, 103],
        ];
        self.begin();
        self.document.probes.push(ProbeDefinition {
            id,
            name: format!("Probe {}", id.0),
            color: COLORS[(id.0.saturating_sub(1) as usize) % COLORS.len()],
            enabled: true,
            target: ProbeTarget::Point(position),
        });
        self.commit();
        Ok(id)
    }

    pub fn create_segment_probe(&mut self, start: Point2, end: Point2) -> Result<ProbeId, String> {
        let target = ProbeTarget::Segment {
            start,
            end,
            preset: ProbeSamplingPreset::Medium,
        };
        if !start.finite() || !end.finite() || (end - start).norm() < 1.0e-6 {
            return Err("Line probe endpoints must be distinct and finite".into());
        }
        if self.document.probes.len() >= MAX_PROBES {
            return Err(format!("Maximum {MAX_PROBES} probes"));
        }
        if self.segment_probe_points() + ProbeSamplingPreset::Medium.spatial_points()
            > MAX_SEGMENT_PROBE_POINTS
        {
            return Err(format!(
                "Line probes are limited to {MAX_SEGMENT_PROBE_POINTS} sample points"
            ));
        }
        let id = ProbeId(self.next_probe_id);
        self.next_probe_id = self
            .next_probe_id
            .checked_add(1)
            .ok_or("Probe IDs exhausted")?;
        const COLORS: [[u8; 3]; 8] = [
            [63, 144, 239],
            [244, 105, 122],
            [78, 201, 176],
            [245, 183, 69],
            [164, 126, 232],
            [70, 190, 232],
            [230, 125, 67],
            [153, 203, 103],
        ];
        self.begin();
        self.document.probes.push(ProbeDefinition {
            id,
            name: format!("Line probe {}", id.0),
            color: COLORS[(id.0.saturating_sub(1) as usize) % COLORS.len()],
            enabled: true,
            target,
        });
        self.commit();
        Ok(id)
    }

    pub fn create_boundary_probe(
        &mut self,
        mut target: BoundaryProbeTarget,
    ) -> Result<ProbeId, String> {
        self.validate_boundary_probe(target)?;
        if self.document.probes.len() >= MAX_PROBES {
            return Err(format!("Maximum {MAX_PROBES} probes"));
        }
        if self.segment_probe_points() + target.preset.spatial_points() > MAX_SEGMENT_PROBE_POINTS {
            return Err(format!(
                "Line probes are limited to {MAX_SEGMENT_PROBE_POINTS} sample points"
            ));
        }
        let total = self.boundary_feature_span_count(target.feature).unwrap();
        target.start_span %= total;
        target.span_count = if target.whole {
            total
        } else {
            target.span_count.min(total)
        };
        let id = ProbeId(self.next_probe_id);
        self.next_probe_id = self
            .next_probe_id
            .checked_add(1)
            .ok_or("Probe IDs exhausted")?;
        const COLORS: [[u8; 3]; 8] = [
            [63, 144, 239],
            [244, 105, 122],
            [78, 201, 176],
            [245, 183, 69],
            [164, 126, 232],
            [70, 190, 232],
            [230, 125, 67],
            [153, 203, 103],
        ];
        self.begin();
        self.document.probes.push(ProbeDefinition {
            id,
            name: format!("Boundary probe {}", id.0),
            color: COLORS[(id.0.saturating_sub(1) as usize) % COLORS.len()],
            enabled: true,
            target: ProbeTarget::Boundary(target),
        });
        self.commit();
        Ok(id)
    }

    pub fn create_area_disk_probe(
        &mut self,
        center: Point2,
        radius: f64,
    ) -> Result<ProbeId, String> {
        self.create_area_probe(ProbeTarget::AreaDisk { center, radius })
    }

    pub fn create_area_region_probe(&mut self, region: RegionId) -> Result<ProbeId, String> {
        if self.document.draft.region(region).is_none() {
            return Err("Area probe references a missing region".into());
        }
        self.create_area_probe(ProbeTarget::AreaRegion { region })
    }

    fn create_area_probe(&mut self, target: ProbeTarget) -> Result<ProbeId, String> {
        let valid = match target {
            ProbeTarget::AreaDisk { center, radius } => {
                center.finite()
                    && radius.is_finite()
                    && radius > 0.0
                    && (std::f64::consts::PI * radius * radius).is_finite()
            }
            ProbeTarget::AreaRegion { region } => region.0 > 0,
            _ => false,
        };
        if !valid {
            return Err("Area probe target must be valid".into());
        }
        if self.document.probes.len() >= MAX_PROBES {
            return Err(format!("Maximum {MAX_PROBES} probes"));
        }
        let id = ProbeId(self.next_probe_id);
        self.next_probe_id = self
            .next_probe_id
            .checked_add(1)
            .ok_or("Probe IDs exhausted")?;
        const COLORS: [[u8; 3]; 8] = [
            [63, 144, 239],
            [244, 105, 122],
            [78, 201, 176],
            [245, 183, 69],
            [164, 126, 232],
            [70, 190, 232],
            [230, 125, 67],
            [153, 203, 103],
        ];
        self.begin();
        self.document.probes.push(ProbeDefinition {
            id,
            name: format!("Area probe {}", id.0),
            color: COLORS[(id.0.saturating_sub(1) as usize) % COLORS.len()],
            enabled: true,
            target,
        });
        self.commit();
        Ok(id)
    }

    pub fn set_far_field(&mut self, settings: FarFieldSettings) -> Result<(), String> {
        if !settings.valid() {
            return Err("Far-field inset must be finite and between 0 and 1".into());
        }
        if self.document.far_field == settings {
            return Ok(());
        }
        self.begin();
        self.document.far_field = settings;
        self.commit();
        Ok(())
    }

    pub fn boundary_feature_span_count(&self, feature: BoundaryProbeFeature) -> Option<usize> {
        match feature {
            BoundaryProbeFeature::Outer => Some(4),
            BoundaryProbeFeature::Loop(id) => self
                .document
                .draft
                .obstacles
                .iter()
                .find(|obstacle| obstacle.id == id)
                .map(|obstacle| obstacle.spline.intervals().len()),
            BoundaryProbeFeature::Baffle(id) => self
                .document
                .draft
                .internal_boundaries
                .iter()
                .find(|boundary| boundary.id == id)
                .map(|boundary| boundary.spline.intervals().len()),
        }
    }

    pub fn validate_boundary_probe(&self, target: BoundaryProbeTarget) -> Result<(), String> {
        let total = self
            .boundary_feature_span_count(target.feature)
            .ok_or("Boundary probe references a missing feature")?;
        if total == 0
            || target.start_span >= total
            || target.span_count == 0
            || target.span_count > total
            || (matches!(target.feature, BoundaryProbeFeature::Baffle(_))
                && target.start_span + target.span_count > total)
        {
            return Err("Boundary probe has an invalid span run".into());
        }
        let side_valid = match target.feature {
            BoundaryProbeFeature::Outer => target.side == BoundaryProbeSide::Domain,
            BoundaryProbeFeature::Baffle(_) => {
                matches!(
                    target.side,
                    BoundaryProbeSide::Left | BoundaryProbeSide::Right
                )
            }
            BoundaryProbeFeature::Loop(id) => {
                match self
                    .document
                    .draft
                    .obstacles
                    .iter()
                    .find(|obstacle| obstacle.id == id)
                    .unwrap()
                    .role
                {
                    LoopRole::Hole { .. } => target.side == BoundaryProbeSide::Domain,
                    LoopRole::MaterialInterface { .. } | LoopRole::Wall { .. } => matches!(
                        target.side,
                        BoundaryProbeSide::Exterior | BoundaryProbeSide::Interior
                    ),
                }
            }
        };
        if !side_valid {
            return Err("Boundary probe side is incompatible with its feature".into());
        }
        Ok(())
    }

    fn delete_boundary_probes_for(&mut self, feature: BoundaryProbeFeature) {
        self.document.probes.retain(|probe| {
            !matches!(probe.target, ProbeTarget::Boundary(target) if target.feature == feature)
        });
    }

    fn delete_area_probes_for_region(&mut self, region: RegionId) {
        self.document.probes.retain(
            |probe| !matches!(probe.target, ProbeTarget::AreaRegion { region: id } if id == region),
        );
    }

    fn remap_loop_probe_role(&mut self, id: ObstacleId, old: LoopRole, new: LoopRole) {
        self.document.probes.retain_mut(|probe| {
            let ProbeTarget::Boundary(mut target) = probe.target else {
                return true;
            };
            if target.feature != BoundaryProbeFeature::Loop(id) {
                return true;
            }
            match (old, new, target.side) {
                (
                    LoopRole::Hole { .. },
                    LoopRole::MaterialInterface { .. } | LoopRole::Wall { .. },
                    BoundaryProbeSide::Domain,
                ) => {
                    target.side = BoundaryProbeSide::Exterior;
                }
                (
                    LoopRole::MaterialInterface { .. } | LoopRole::Wall { .. },
                    LoopRole::Hole { .. },
                    BoundaryProbeSide::Exterior,
                ) => {
                    target.side = BoundaryProbeSide::Domain;
                }
                (
                    LoopRole::MaterialInterface { .. } | LoopRole::Wall { .. },
                    LoopRole::Hole { .. },
                    BoundaryProbeSide::Interior,
                ) => {
                    return false;
                }
                _ => {}
            }
            probe.target = ProbeTarget::Boundary(target);
            true
        });
    }

    fn remap_boundary_probe_spans(
        &mut self,
        feature: BoundaryProbeFeature,
        old_total: usize,
        new_total: usize,
        mut map: impl FnMut(usize) -> Vec<usize>,
    ) {
        self.document.probes.retain_mut(|probe| {
            let ProbeTarget::Boundary(mut target) = probe.target else {
                return true;
            };
            if target.feature != feature {
                return true;
            }
            let mut selected = BTreeSet::new();
            for old in target.spans(old_total) {
                selected.extend(map(old).into_iter().filter(|index| *index < new_total));
            }
            let closed = !matches!(feature, BoundaryProbeFeature::Baffle(_));
            if !set_boundary_probe_run(&mut target, new_total, &selected, closed) {
                return false;
            }
            probe.target = ProbeTarget::Boundary(target);
            true
        });
    }

    fn segment_probe_points(&self) -> usize {
        self.document
            .probes
            .iter()
            .map(|probe| match probe.target {
                ProbeTarget::Segment { preset, .. }
                | ProbeTarget::Boundary(BoundaryProbeTarget { preset, .. }) => {
                    preset.spatial_points()
                }
                ProbeTarget::Point(_) => 0,
                ProbeTarget::AreaDisk { .. } | ProbeTarget::AreaRegion { .. } => 0,
            })
            .sum()
    }

    pub fn update_probe(&mut self, probe: ProbeDefinition) -> Result<(), String> {
        if !probe.valid() {
            return Err("Probe name and target must be valid".into());
        }
        if let ProbeTarget::Boundary(target) = probe.target {
            self.validate_boundary_probe(target)?;
        }
        if let ProbeTarget::AreaRegion { region } = probe.target
            && self.document.draft.region(region).is_none()
        {
            return Err("Area probe references a missing region".into());
        }
        let id = probe.id;
        let current = self
            .document
            .probes
            .iter()
            .find(|candidate| candidate.id == id)
            .ok_or("Missing probe")?;
        let points_without_current = self.segment_probe_points()
            - match current.target {
                ProbeTarget::Segment { preset, .. }
                | ProbeTarget::Boundary(BoundaryProbeTarget { preset, .. }) => {
                    preset.spatial_points()
                }
                ProbeTarget::Point(_) => 0,
                ProbeTarget::AreaDisk { .. } | ProbeTarget::AreaRegion { .. } => 0,
            };
        let replacement_points = match probe.target {
            ProbeTarget::Segment { preset, .. }
            | ProbeTarget::Boundary(BoundaryProbeTarget { preset, .. }) => preset.spatial_points(),
            ProbeTarget::Point(_) => 0,
            ProbeTarget::AreaDisk { .. } | ProbeTarget::AreaRegion { .. } => 0,
        };
        if points_without_current + replacement_points > MAX_SEGMENT_PROBE_POINTS {
            return Err(format!(
                "Line probes are limited to {MAX_SEGMENT_PROBE_POINTS} sample points"
            ));
        }
        if current == &probe {
            return Ok(());
        }
        self.begin();
        *self
            .document
            .probes
            .iter_mut()
            .find(|candidate| candidate.id == id)
            .unwrap() = probe;
        self.commit();
        Ok(())
    }

    pub fn delete_probe(&mut self, id: ProbeId) -> Result<(), String> {
        if !self.document.probes.iter().any(|probe| probe.id == id) {
            return Err("Missing probe".into());
        }
        self.begin();
        self.document.probes.retain(|probe| probe.id != id);
        self.commit();
        Ok(())
    }

    /// Caller must validate the accepted scene before replacement.
    pub fn replace_validated(&mut self, document: Document) {
        self.install_document(document);
        self.undo.clear();
        self.redo.clear();
        self.before = None;
        self.changed();
    }

    /// Installs a validated example as one undoable document action.
    pub fn replace_validated_with_history(&mut self, document: Document) {
        if self.document == document {
            return;
        }
        self.commit();
        self.undo.push(self.document.clone());
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.before = None;
        self.install_document(document);
        self.changed();
    }

    fn install_document(&mut self, document: Document) {
        self.next_obstacle_id = document
            .draft
            .obstacles
            .iter()
            .chain(&document.accepted.obstacles)
            .map(|o| o.id.0)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.next_region_id = document
            .draft
            .regions
            .iter()
            .chain(&document.accepted.regions)
            .map(|region| region.id.0)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.next_internal_boundary_id = document
            .draft
            .internal_boundaries
            .iter()
            .chain(&document.accepted.internal_boundaries)
            .map(|boundary| boundary.id.0)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.next_material_id = document
            .draft
            .materials
            .iter()
            .chain(&document.accepted.materials)
            .map(|material| material.id.0)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.next_probe_id = document
            .probes
            .iter()
            .map(|probe| probe.id.0)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.document = document;
    }
}

fn set_boundary_probe_run(
    target: &mut BoundaryProbeTarget,
    total: usize,
    selected: &BTreeSet<usize>,
    closed: bool,
) -> bool {
    if total == 0 || selected.is_empty() {
        return false;
    }
    if selected.len() == total {
        target.start_span = 0;
        target.span_count = total;
        target.whole = true;
        return true;
    }
    let mut runs = Vec::new();
    if closed {
        for start in 0..total {
            if !selected.contains(&start) || selected.contains(&((start + total - 1) % total)) {
                continue;
            }
            let mut count = 0;
            while count < total && selected.contains(&((start + count) % total)) {
                count += 1;
            }
            runs.push((count, start));
        }
    } else {
        let mut index = 0;
        while index < total {
            if !selected.contains(&index) {
                index += 1;
                continue;
            }
            let start = index;
            while index < total && selected.contains(&index) {
                index += 1;
            }
            runs.push((index - start, start));
        }
    }
    let Some((count, start)) = runs
        .into_iter()
        .max_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1)))
    else {
        return false;
    };
    target.start_span = start;
    target.span_count = count;
    target.whole = false;
    true
}

fn open_span_arc_length(spline: &OpenCubicSpline, span: usize) -> f64 {
    let Some(start) = spline.breakpoint(span) else {
        return 0.0;
    };
    let Some(end) = spline.breakpoint(span + 1) else {
        return 0.0;
    };
    let mut length = 0.0;
    let mut previous = spline.evaluate(start);
    for piece in 1..=16 {
        let parameter = start + (end - start) * piece as f64 / 16.0;
        let point = spline.evaluate(parameter);
        length += (point - previous).norm();
        previous = point;
    }
    length
}

fn replace_exterior(role: LoopRole, from: RegionId, to: RegionId) -> LoopRole {
    if role.exterior() != from {
        return role;
    }
    match role {
        LoopRole::Hole { .. } => LoopRole::Hole { exterior: to },
        LoopRole::MaterialInterface { interior, .. } => LoopRole::MaterialInterface {
            exterior: to,
            interior,
        },
        LoopRole::Wall { interior, .. } => LoopRole::Wall {
            exterior: to,
            interior,
        },
    }
}
