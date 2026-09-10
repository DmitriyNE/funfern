use funfern_core::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryControl {
    Loop(ObstacleId, usize),
    Baffle(InternalBoundaryId, usize),
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
}
impl Default for Document {
    fn default() -> Self {
        let scene = Scene::initial();
        Self {
            draft: scene.clone(),
            accepted: scene,
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
        }
    }
}
impl Editor {
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
            self.document = before;
            self.changed();
        }
    }
    pub fn undo(&mut self) {
        if self.editing() {
            self.cancel();
            return;
        }
        if let Some(doc) = self.undo.pop() {
            self.redo.push(self.document.clone());
            self.document = doc;
            self.changed();
        }
    }
    pub fn redo(&mut self) {
        if self.editing() {
            return;
        }
        if let Some(doc) = self.redo.pop() {
            self.undo.push(self.document.clone());
            self.document = doc;
            self.changed();
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

    /// Refines an existing baffle breakpoint to the requested continuity. This
    /// only inserts knots, so the represented curve and span laws are unchanged.
    pub fn set_internal_boundary_continuity(
        &mut self,
        id: InternalBoundaryId,
        breakpoint: usize,
        continuity: u8,
    ) -> Result<(), String> {
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
        if continuity > current {
            return Err("Smoothing an edited corner is not shape preserving; use Undo".into());
        }
        if continuity == current {
            return Ok(());
        }
        let mut spline = boundary.spline.clone();
        while spline.continuity(breakpoint).unwrap() > continuity {
            spline
                .increase_multiplicity(breakpoint)
                .map_err(|error| error.to_string())?;
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
        let spline = a
            .spline
            .join(b.spline, tolerance)
            .map_err(|error| error.to_string())?;
        let mut laws = a.span_laws;
        laws.extend(b.span_laws);
        self.begin();
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
        if index <= 1 {
            span_laws.remove(0);
        } else if index + 2 >= old_control_count {
            span_laws.pop();
        } else {
            let left = (index - 2).min(span_laws.len() - 2);
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
        match spline.insert(t).map_err(|e| e.to_string())? {
            Insertion::Existing(i) => Ok(i),
            Insertion::Inserted(i) => {
                self.begin();
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

    /// Refines a loop breakpoint to C1 or C0 without changing its shape or its
    /// per-span assignments. Breakpoint zero is the editable periodic seam.
    pub fn set_obstacle_continuity(
        &mut self,
        id: ObstacleId,
        breakpoint: usize,
        continuity: u8,
    ) -> Result<(), String> {
        if continuity > 2 {
            return Err("Cubic continuity must be C0, C1, or C2".into());
        }
        let obstacle = self.obstacle(id).ok_or("Missing obstacle")?;
        let current = obstacle
            .spline
            .continuity(breakpoint)
            .ok_or("Missing loop knot")?;
        if continuity > current {
            return Err("Smoothing an edited corner is not shape preserving; use Undo".into());
        }
        if continuity == current {
            return Ok(());
        }
        let mut spline = obstacle.spline.clone();
        while spline.continuity(breakpoint).unwrap() > continuity {
            spline
                .increase_multiplicity(breakpoint)
                .map_err(|error| error.to_string())?;
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
    /// Caller must validate the accepted scene before replacement.
    pub fn replace_validated(&mut self, document: Document) {
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
        self.document = document;
        self.undo.clear();
        self.redo.clear();
        self.before = None;
        self.changed();
    }
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
