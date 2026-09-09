use femfun_core::*;
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
                spline,
                region,
                law: InternalBoundaryLaw::Reflecting,
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

    pub fn insert_internal_boundary(
        &mut self,
        id: InternalBoundaryId,
        parameter: f64,
    ) -> Result<usize, String> {
        let mut spline = self
            .internal_boundary(id)
            .ok_or("Missing internal boundary")?
            .spline
            .clone();
        match spline
            .insert(parameter)
            .map_err(|error| error.to_string())?
        {
            Insertion::Existing(index) => Ok(index),
            Insertion::Inserted(index) => {
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
                Ok(index)
            }
        }
    }

    pub fn remove_internal_boundary_point(
        &mut self,
        id: InternalBoundaryId,
        index: usize,
    ) -> Result<(), String> {
        let mut spline = self
            .internal_boundary(id)
            .ok_or("Missing internal boundary")?
            .spline
            .clone();
        spline.remove(index).map_err(|error| error.to_string())?;
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
            .push(Obstacle { id, spline, role });
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
        let mut spline = self.obstacle(id).ok_or("Missing obstacle")?.spline.clone();
        spline.remove(index).map_err(|e| e.to_string())?;
        self.begin();
        self.document
            .draft
            .obstacles
            .iter_mut()
            .find(|o| o.id == id)
            .unwrap()
            .spline = spline;
        self.changed();
        self.commit();
        Ok(())
    }
    pub fn insert(&mut self, id: ObstacleId, t: f64) -> Result<usize, String> {
        let mut spline = self.obstacle(id).ok_or("Missing obstacle")?.spline.clone();
        match spline.insert(t).map_err(|e| e.to_string())? {
            Insertion::Existing(i) => Ok(i),
            Insertion::Inserted(i) => {
                self.begin();
                self.document
                    .draft
                    .obstacles
                    .iter_mut()
                    .find(|o| o.id == id)
                    .unwrap()
                    .spline = spline;
                self.changed();
                self.commit();
                Ok(i)
            }
        }
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
