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
    next_id: u64,
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
            next_id: 2,
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
        if self.document.draft.obstacles.len() >= MAX_OBSTACLES {
            return Err("Maximum 32 obstacles".into());
        }
        let id = ObstacleId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or("Obstacle IDs exhausted")?;
        self.begin();
        self.document.draft.obstacles.push(Obstacle { id, spline });
        self.changed();
        self.commit();
        Ok(id)
    }
    pub fn delete_obstacle(&mut self, id: ObstacleId) {
        self.begin();
        self.document.draft.obstacles.retain(|o| o.id != id);
        self.changed();
        self.commit();
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
        self.next_id = document
            .draft
            .obstacles
            .iter()
            .chain(&document.accepted.obstacles)
            .map(|o| o.id.0)
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
