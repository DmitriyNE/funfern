use crate::{
    PeriodicCubicSpline, Point2, Sample, Sampler, SamplingOptions, point_segment_distance,
};
pub const MAX_OBSTACLES: usize = 32;
pub const MAX_MATERIALS: usize = 32;
pub const WORLD_TOLERANCE: f64 = 2.0e-4;
pub const BACKGROUND_REGION: RegionId = RegionId(1);
pub const DEFAULT_MATERIAL: MaterialId = MaterialId(1);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObstacleId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegionId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MaterialId(pub u64);

#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub id: MaterialId,
    pub name: String,
    pub mass_density: f64,
    pub stiffness: f64,
    pub damping: f64,
    pub color: [u8; 3],
}

impl Material {
    pub fn default_medium() -> Self {
        Self {
            id: DEFAULT_MATERIAL,
            name: "Background".into(),
            mass_density: 1.0,
            stiffness: 1.0,
            damping: 0.0,
            color: [47, 73, 88],
        }
    }

    pub fn valid(&self) -> bool {
        self.id.0 > 0
            && !self.name.trim().is_empty()
            && self.name.len() <= 64
            && self.mass_density.is_finite()
            && self.mass_density > 0.0
            && self.stiffness.is_finite()
            && self.stiffness > 0.0
            && self.damping.is_finite()
            && self.damping >= 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub id: RegionId,
    pub material: MaterialId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopRole {
    Hole {
        exterior: RegionId,
    },
    MaterialInterface {
        exterior: RegionId,
        interior: RegionId,
    },
    Wall {
        exterior: RegionId,
        interior: RegionId,
    },
}

impl LoopRole {
    pub fn exterior(self) -> RegionId {
        match self {
            Self::Hole { exterior }
            | Self::MaterialInterface { exterior, .. }
            | Self::Wall { exterior, .. } => exterior,
        }
    }

    pub fn interior(self) -> Option<RegionId> {
        match self {
            Self::Hole { .. } => None,
            Self::MaterialInterface { interior, .. } | Self::Wall { interior, .. } => {
                Some(interior)
            }
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Hole { .. } => "Hole",
            Self::MaterialInterface { .. } => "Material interface",
            Self::Wall { .. } => "Closed wall",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Obstacle {
    pub id: ObstacleId,
    pub spline: PeriodicCubicSpline,
    pub role: LoopRole,
}
impl Obstacle {
    pub fn hole(id: ObstacleId, spline: PeriodicCubicSpline) -> Self {
        Self {
            id,
            spline,
            role: LoopRole::Hole {
                exterior: BACKGROUND_REGION,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub obstacles: Vec<Obstacle>,
    pub materials: Vec<Material>,
    pub regions: Vec<Region>,
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            obstacles: vec![],
            materials: vec![Material::default_medium()],
            regions: vec![Region {
                id: BACKGROUND_REGION,
                material: DEFAULT_MATERIAL,
            }],
        }
    }
}
impl Scene {
    pub fn initial() -> Self {
        Self {
            obstacles: vec![Obstacle::hole(
                ObstacleId(1),
                PeriodicCubicSpline::rounded(Point2::default(), 0.15),
            )],
            ..Self::default()
        }
    }
    pub fn structure_valid(&self) -> bool {
        if self.obstacles.len() > MAX_OBSTACLES
            || self.materials.is_empty()
            || self.materials.len() > MAX_MATERIALS
            || self.regions.is_empty()
            || self.regions.len() > MAX_OBSTACLES + 1
        {
            return false;
        }
        let unique_obstacles = self.obstacles.iter().enumerate().all(|(i, o)| {
            o.id.0 > 0
                && !self.obstacles[..i]
                    .iter()
                    .any(|previous| previous.id == o.id)
        });
        let unique_materials = self.materials.iter().enumerate().all(|(i, material)| {
            material.valid()
                && !self.materials[..i]
                    .iter()
                    .any(|previous| previous.id == material.id)
        });
        let unique_regions = self.regions.iter().enumerate().all(|(i, region)| {
            region.id.0 > 0
                && self.material(region.material).is_some()
                && !self.regions[..i]
                    .iter()
                    .any(|previous| previous.id == region.id)
        });
        if !unique_obstacles
            || !unique_materials
            || !unique_regions
            || self.region(BACKGROUND_REGION).is_none()
        {
            return false;
        }
        let mut interiors = Vec::new();
        for obstacle in &self.obstacles {
            if self.region(obstacle.role.exterior()).is_none() {
                return false;
            }
            if let Some(interior) = obstacle.role.interior() {
                if interior == BACKGROUND_REGION
                    || interior == obstacle.role.exterior()
                    || self.region(interior).is_none()
                    || interiors.contains(&interior)
                {
                    return false;
                }
                interiors.push(interior);
            }
        }
        self.regions
            .iter()
            .all(|region| region.id == BACKGROUND_REGION || interiors.contains(&region.id))
    }

    pub fn material(&self, id: MaterialId) -> Option<&Material> {
        self.materials.iter().find(|material| material.id == id)
    }

    pub fn region(&self, id: RegionId) -> Option<&Region> {
        self.regions.iter().find(|region| region.id == id)
    }

    pub fn region_material(&self, id: RegionId) -> Option<&Material> {
        self.region(id)
            .and_then(|region| self.material(region.material))
    }

    /// Geometry and topology equality excludes names, colors, coefficients, and
    /// region-to-material assignments so those edits can reuse the mesh.
    pub fn geometry_eq(&self, other: &Self) -> bool {
        self.obstacles == other.obstacles
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum ValidationIssue {
    Structure,
    Subdivision(ObstacleId),
    Outside(ObstacleId),
    Degenerate(ObstacleId),
    SelfContact(ObstacleId),
    ObstacleContact(ObstacleId, ObstacleId),
    Nested(ObstacleId, ObstacleId),
    RegionTopology(ObstacleId),
    WorkLimit,
}
impl std::fmt::Display for ValidationIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Structure => write!(
                f,
                "Scene has invalid IDs, materials, regions, or too many loops"
            ),
            Self::Subdivision(id) => write!(
                f,
                "Loop {}: sampling is exhausted or numerically ambiguous",
                id.0
            ),
            Self::Outside(id) => write!(f, "Loop {} leaves or nearly touches the outer box", id.0),
            Self::Degenerate(id) => write!(f, "Loop {} is degenerate or too small", id.0),
            Self::SelfContact(id) => {
                write!(f, "Loop {} crosses or nearly touches itself", id.0)
            }
            Self::ObstacleContact(a, b) => {
                write!(f, "Loops {} and {} intersect or nearly touch", a.0, b.0)
            }
            Self::Nested(a, b) => write!(
                f,
                "Loops {} and {} have incompatible nesting roles",
                a.0, b.0
            ),
            Self::RegionTopology(id) => write!(
                f,
                "Loop {} has a region assignment inconsistent with its containment",
                id.0
            ),
            Self::WorkLimit => write!(f, "Validation work limit reached; simplify the scene"),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ValidationResult {
    pub revision: u64,
    pub issue: Option<ValidationIssue>,
}
impl ValidationResult {
    pub fn valid(&self) -> bool {
        self.issue.is_none()
    }
}
#[derive(Clone, Copy)]
struct Segment {
    a: Point2,
    b: Point2,
    obstacle: usize,
    index: usize,
    arc_start: f64,
    arc_end: f64,
}
/// At most `budget` elementary operations per advance; no camera inputs.
pub struct ValidationJob {
    revision: u64,
    scene: Scene,
    options: SamplingOptions,
    sampler: Option<Sampler>,
    loops: Vec<Vec<Sample>>,
    segments: Vec<Segment>,
    bounds: Vec<(Point2, Point2)>,
    perimeters: Vec<f64>,
    pending_points: Option<Vec<Sample>>,
    build_index: usize,
    area: f64,
    perimeter: f64,
    lower: Point2,
    upper: Point2,
    i: usize,
    j: usize,
    nest_a: usize,
    nest_b: usize,
    nest_edge: usize,
    inside: bool,
    containment: Vec<Vec<bool>>,
    work: usize,
    result: Option<ValidationResult>,
}
impl ValidationJob {
    pub fn new(scene: Scene, revision: u64) -> Self {
        Self::with_options(scene, revision, SamplingOptions::default())
    }
    pub fn with_options(scene: Scene, revision: u64, options: SamplingOptions) -> Self {
        let loop_count = scene.obstacles.len();
        let issue = if scene.structure_valid() {
            None
        } else {
            Some(ValidationIssue::Structure)
        };
        Self {
            revision,
            scene,
            options,
            sampler: None,
            loops: vec![],
            segments: vec![],
            bounds: vec![],
            perimeters: vec![],
            pending_points: None,
            build_index: 0,
            area: 0.0,
            perimeter: 0.0,
            lower: Point2::new(f64::INFINITY, f64::INFINITY),
            upper: Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY),
            i: 0,
            j: 1,
            nest_a: 0,
            nest_b: 1,
            nest_edge: 0,
            inside: false,
            containment: vec![vec![false; loop_count]; loop_count],
            work: 0,
            result: issue.map(|v| ValidationResult {
                revision,
                issue: Some(v),
            }),
        }
    }
    fn finish(&mut self, issue: Option<ValidationIssue>) {
        self.result = Some(ValidationResult {
            revision: self.revision,
            issue,
        });
    }
    pub fn advance(&mut self, budget: usize) -> Option<ValidationResult> {
        for _ in 0..budget {
            if self.result.is_some() {
                break;
            }
            self.work += 1;
            if self.work > 50_000_000 {
                self.finish(Some(ValidationIssue::WorkLimit));
                break;
            }
            if let Some(points) = &self.pending_points {
                let obstacle = self.loops.len();
                let id = self.scene.obstacles[obstacle].id;
                if self.build_index + 1 == points.len() {
                    if self.area.abs() * 0.5 <= WORLD_TOLERANCE * WORLD_TOLERANCE {
                        self.finish(Some(ValidationIssue::Degenerate(id)));
                        continue;
                    }
                    self.loops.push(self.pending_points.take().unwrap());
                    self.bounds.push((self.lower, self.upper));
                    self.perimeters.push(self.perimeter);
                    self.build_index = 0;
                    self.area = 0.0;
                    self.perimeter = 0.0;
                    self.lower = Point2::new(f64::INFINITY, f64::INFINITY);
                    self.upper = Point2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
                    continue;
                }
                let a = points[self.build_index].point;
                let b = points[self.build_index + 1].point;
                let margin = WORLD_TOLERANCE + self.options.tolerance;
                if a.x.abs() >= 1.0 - margin || a.y.abs() >= 1.0 - margin {
                    self.finish(Some(ValidationIssue::Outside(id)));
                    continue;
                }
                self.lower.x = self.lower.x.min(a.x);
                self.lower.y = self.lower.y.min(a.y);
                self.upper.x = self.upper.x.max(a.x);
                self.upper.y = self.upper.y.max(a.y);
                self.area += a.cross(b);
                let arc_start = self.perimeter;
                self.perimeter += (b - a).norm();
                self.segments.push(Segment {
                    a,
                    b,
                    obstacle,
                    index: self.build_index,
                    arc_start,
                    arc_end: self.perimeter,
                });
                self.build_index += 1;
            } else if self.loops.len() < self.scene.obstacles.len() {
                let o = &self.scene.obstacles[self.loops.len()];
                let id = o.id;
                let sampler = self
                    .sampler
                    .get_or_insert_with(|| Sampler::new(&o.spline, self.options));
                if sampler.step() {
                    match self.sampler.take().unwrap().finish() {
                        Err(_) => self.finish(Some(ValidationIssue::Subdivision(id))),
                        Ok(points) => self.pending_points = Some(points),
                    }
                }
            } else if self.i < self.segments.len() {
                if self.j >= self.segments.len() {
                    self.i += 1;
                    self.j = self.i + 1;
                    continue;
                }
                let a = self.segments[self.i];
                let b = self.segments[self.j];
                self.j += 1;
                let margin = WORLD_TOLERANCE + 2.0 * self.options.tolerance;
                let mut local_neighbors = false;
                if a.obstacle == b.obstacle {
                    let n = self.loops[a.obstacle].len() - 1;
                    if a.index.abs_diff(b.index) == 1 || a.index.abs_diff(b.index) == n - 1 {
                        continue;
                    }
                    // Knot insertion can introduce arbitrarily short spans on an
                    // unchanged smooth arc. Suppress only local proximity, never
                    // a crossing, using arc distance instead of segment indices.
                    let gap = (b.arc_start - a.arc_end)
                        .min(self.perimeters[a.obstacle] - b.arc_end + a.arc_start);
                    local_neighbors = gap <= 4.0 * margin;
                } else {
                    let (amin, amax) = self.bounds[a.obstacle];
                    let (bmin, bmax) = self.bounds[b.obstacle];
                    if amax.x + margin < bmin.x
                        || bmax.x + margin < amin.x
                        || amax.y + margin < bmin.y
                        || bmax.y + margin < amin.y
                    {
                        // Segments are grouped by obstacle: skip the entire loop.
                        self.j += self.loops[b.obstacle].len() - 2 - b.index;
                        continue;
                    }
                }
                if segments_close(
                    a.a,
                    a.b,
                    b.a,
                    b.b,
                    if local_neighbors { 0.0 } else { margin },
                ) {
                    let id = self.scene.obstacles[a.obstacle].id;
                    self.finish(Some(if a.obstacle == b.obstacle {
                        ValidationIssue::SelfContact(id)
                    } else {
                        ValidationIssue::ObstacleContact(id, self.scene.obstacles[b.obstacle].id)
                    }));
                }
            } else if self.nest_a < self.loops.len() {
                if self.nest_b >= self.loops.len() {
                    self.nest_a += 1;
                    self.nest_b = 0;
                    continue;
                }
                if self.nest_a == self.nest_b {
                    self.nest_b += 1;
                    continue;
                }
                let p = self.loops[self.nest_a][0].point;
                let (lower, upper) = self.bounds[self.nest_b];
                if p.x < lower.x || p.y < lower.y || p.x > upper.x || p.y > upper.y {
                    self.nest_b += 1;
                    continue;
                }
                let polygon = &self.loops[self.nest_b];
                if self.nest_edge + 1 >= polygon.len() {
                    if self.inside {
                        self.containment[self.nest_a][self.nest_b] = true;
                    }
                    self.nest_b += 1;
                    self.nest_edge = 0;
                    self.inside = false;
                    continue;
                }
                let a = polygon[self.nest_edge].point;
                let b = polygon[self.nest_edge + 1].point;
                self.nest_edge += 1;
                if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x
                {
                    self.inside = !self.inside;
                }
            } else {
                self.finish(self.region_topology_issue())
            }
        }
        self.result.clone()
    }

    fn region_topology_issue(&self) -> Option<ValidationIssue> {
        for (index, obstacle) in self.scene.obstacles.iter().enumerate() {
            let direct_container = (0..self.scene.obstacles.len())
                .filter(|container| self.containment[index][*container])
                .min_by(|a, b| self.perimeters[*a].total_cmp(&self.perimeters[*b]));
            let expected_exterior = match direct_container {
                None => BACKGROUND_REGION,
                Some(container) => match self.scene.obstacles[container].role.interior() {
                    Some(region) => region,
                    None => {
                        return Some(ValidationIssue::Nested(
                            obstacle.id,
                            self.scene.obstacles[container].id,
                        ));
                    }
                },
            };
            if obstacle.role.exterior() != expected_exterior {
                return Some(ValidationIssue::RegionTopology(obstacle.id));
            }
        }
        None
    }
}
fn segments_close(a: Point2, b: Point2, c: Point2, d: Point2, tol: f64) -> bool {
    if a.x.max(b.x) + tol < c.x.min(d.x)
        || c.x.max(d.x) + tol < a.x.min(b.x)
        || a.y.max(b.y) + tol < c.y.min(d.y)
        || c.y.max(d.y) + tol < a.y.min(b.y)
    {
        return false;
    }
    let ab = b - a;
    let cd = d - c;
    let x1 = ab.cross(c - a);
    let x2 = ab.cross(d - a);
    let y1 = cd.cross(a - c);
    let y2 = cd.cross(b - c);
    if x1.signum() != x2.signum() && y1.signum() != y2.signum() {
        return true;
    }
    [
        point_segment_distance(a, c, d),
        point_segment_distance(b, c, d),
        point_segment_distance(c, a, b),
        point_segment_distance(d, a, b),
    ]
    .into_iter()
    .fold(f64::INFINITY, f64::min)
        <= tol
}
pub fn validate(scene: &Scene) -> ValidationResult {
    let mut job = ValidationJob::new(scene.clone(), 0);
    loop {
        if let Some(result) = job.advance(10000) {
            return result;
        }
    }
}
