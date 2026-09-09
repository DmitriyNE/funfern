use crate::{
    OpenCubicSpline, OpenSampler, PeriodicCubicSpline, Point2, Sample, Sampler, SamplingOptions,
    point_segment_distance,
};
pub const MAX_OBSTACLES: usize = 32;
pub const MAX_INTERNAL_BOUNDARIES: usize = 32;
pub const MAX_MATERIALS: usize = 32;
pub const WORLD_TOLERANCE: f64 = 2.0e-4;
pub const BACKGROUND_REGION: RegionId = RegionId(1);
pub const DEFAULT_MATERIAL: MaterialId = MaterialId(1);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObstacleId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InternalBoundaryId(pub u64);
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
            Self::Wall { .. } => "Two-sided wall",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Obstacle {
    pub id: ObstacleId,
    pub spline: PeriodicCubicSpline,
    pub role: LoopRole,
    /// One exterior-face condition for each periodic spline knot span.
    /// Conditions are currently assembled for holes; retaining the vector on
    /// every loop keeps spline edits and future closed-wall assignment uniform.
    pub span_conditions: Vec<FaceBoundaryCondition>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FaceBoundaryCondition {
    Reflecting,
    /// Local absorbing condition scaled by the adjacent material's characteristic
    /// impedance. A ratio of one is the matched first-order condition.
    Impedance {
        ratio: f64,
    },
}

impl FaceBoundaryCondition {
    pub fn valid(self) -> bool {
        match self {
            Self::Reflecting => true,
            Self::Impedance { ratio } => ratio.is_finite() && ratio > 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InternalBoundaryCoupling {
    Independent,
    /// Conservative zero-thickness compliant layer. The coefficient is scaled
    /// by the adjacent material stiffness and couples the two trace jumps.
    ThinGap {
        stiffness_ratio: f64,
    },
}

impl InternalBoundaryCoupling {
    pub fn valid(self) -> bool {
        match self {
            Self::Independent => true,
            Self::ThinGap { stiffness_ratio } => {
                stiffness_ratio.is_finite() && stiffness_ratio > 0.0
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InternalBoundaryLaw {
    pub left: FaceBoundaryCondition,
    pub right: FaceBoundaryCondition,
    pub coupling: InternalBoundaryCoupling,
}

impl InternalBoundaryLaw {
    pub const REFLECTING: Self = Self {
        left: FaceBoundaryCondition::Reflecting,
        right: FaceBoundaryCondition::Reflecting,
        coupling: InternalBoundaryCoupling::Independent,
    };

    pub fn valid(self) -> bool {
        self.left.valid() && self.right.valid() && self.coupling.valid()
    }
}

impl Default for InternalBoundaryLaw {
    fn default() -> Self {
        Self::REFLECTING
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InternalBoundary {
    pub id: InternalBoundaryId,
    pub spline: OpenCubicSpline,
    pub region: RegionId,
    /// One law for each nonempty spline knot span.
    pub span_laws: Vec<InternalBoundaryLaw>,
}
impl Obstacle {
    pub fn with_role(id: ObstacleId, spline: PeriodicCubicSpline, role: LoopRole) -> Self {
        let span_conditions = vec![FaceBoundaryCondition::Reflecting; spline.intervals().len()];
        Self {
            id,
            spline,
            role,
            span_conditions,
        }
    }

    pub fn hole(id: ObstacleId, spline: PeriodicCubicSpline) -> Self {
        Self::with_role(
            id,
            spline,
            LoopRole::Hole {
                exterior: BACKGROUND_REGION,
            },
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub obstacles: Vec<Obstacle>,
    pub internal_boundaries: Vec<InternalBoundary>,
    pub materials: Vec<Material>,
    pub regions: Vec<Region>,
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            obstacles: vec![],
            internal_boundaries: vec![],
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
            || self.internal_boundaries.len() > MAX_INTERNAL_BOUNDARIES
            || self.obstacles.len() + self.internal_boundaries.len() > MAX_OBSTACLES
            || self.materials.is_empty()
            || self.materials.len() > MAX_MATERIALS
            || self.regions.is_empty()
            || self.regions.len() > MAX_OBSTACLES + 1
        {
            return false;
        }
        let unique_obstacles = self.obstacles.iter().enumerate().all(|(i, o)| {
            o.id.0 > 0
                && o.span_conditions.len() == o.spline.intervals().len()
                && o.span_conditions.iter().all(|condition| condition.valid())
                && !self.obstacles[..i]
                    .iter()
                    .any(|previous| previous.id == o.id)
        });
        let unique_boundaries =
            self.internal_boundaries
                .iter()
                .enumerate()
                .all(|(index, boundary)| {
                    boundary.id.0 > 0
                        && self.region(boundary.region).is_some()
                        && boundary.span_laws.len() == boundary.spline.intervals().len()
                        && boundary.span_laws.iter().all(|law| law.valid())
                        && !self.internal_boundaries[..index]
                            .iter()
                            .any(|previous| previous.id == boundary.id)
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
            || !unique_boundaries
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
        self.obstacles.len() == other.obstacles.len()
            && self
                .obstacles
                .iter()
                .zip(&other.obstacles)
                .all(|(left, right)| {
                    left.id == right.id && left.spline == right.spline && left.role == right.role
                })
            && self.internal_boundaries.len() == other.internal_boundaries.len()
            && self
                .internal_boundaries
                .iter()
                .zip(&other.internal_boundaries)
                .all(|(left, right)| {
                    left.id == right.id
                        && left.spline == right.spline
                        && left.region == right.region
                })
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
    BoundarySubdivision(InternalBoundaryId),
    BoundaryOutside(InternalBoundaryId),
    BoundaryDegenerate(InternalBoundaryId),
    BoundarySelfContact(InternalBoundaryId),
    BoundaryContact(InternalBoundaryId),
    BoundaryRegionTopology(InternalBoundaryId),
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
            Self::BoundarySubdivision(id) => write!(
                f,
                "Internal boundary {}: sampling is exhausted or numerically ambiguous",
                id.0
            ),
            Self::BoundaryOutside(id) => write!(
                f,
                "Internal boundary {} leaves or nearly touches the outer box",
                id.0
            ),
            Self::BoundaryDegenerate(id) => {
                write!(f, "Internal boundary {} is degenerate or too short", id.0)
            }
            Self::BoundarySelfContact(id) => write!(
                f,
                "Internal boundary {} crosses or nearly touches itself",
                id.0
            ),
            Self::BoundaryContact(id) => write!(
                f,
                "Internal boundary {} intersects or nearly touches another boundary",
                id.0
            ),
            Self::BoundaryRegionTopology(id) => write!(
                f,
                "Internal boundary {} has a region assignment inconsistent with its location",
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
    curve: usize,
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
    open_sampler: Option<OpenSampler>,
    loops: Vec<Vec<Sample>>,
    open_boundaries: Vec<Vec<Sample>>,
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
            open_sampler: None,
            loops: vec![],
            open_boundaries: vec![],
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
                let is_open = self.loops.len() == self.scene.obstacles.len();
                let curve = if is_open {
                    self.scene.obstacles.len() + self.open_boundaries.len()
                } else {
                    self.loops.len()
                };
                if self.build_index + 1 == points.len() {
                    let issue = if is_open {
                        let boundary = &self.scene.internal_boundaries[self.open_boundaries.len()];
                        (self.perimeter <= WORLD_TOLERANCE
                            || (points.last().unwrap().point - points[0].point).norm()
                                <= WORLD_TOLERANCE)
                            .then_some(ValidationIssue::BoundaryDegenerate(boundary.id))
                    } else {
                        (self.area.abs() * 0.5 <= WORLD_TOLERANCE * WORLD_TOLERANCE).then_some(
                            ValidationIssue::Degenerate(self.scene.obstacles[self.loops.len()].id),
                        )
                    };
                    if issue.is_some() {
                        self.finish(issue);
                        continue;
                    }
                    if is_open {
                        self.open_boundaries
                            .push(self.pending_points.take().unwrap());
                    } else {
                        self.loops.push(self.pending_points.take().unwrap());
                    }
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
                if [a, b]
                    .iter()
                    .any(|point| point.x.abs() >= 1.0 - margin || point.y.abs() >= 1.0 - margin)
                {
                    let issue = if is_open {
                        ValidationIssue::BoundaryOutside(
                            self.scene.internal_boundaries[self.open_boundaries.len()].id,
                        )
                    } else {
                        ValidationIssue::Outside(self.scene.obstacles[self.loops.len()].id)
                    };
                    self.finish(Some(issue));
                    continue;
                }
                self.lower.x = self.lower.x.min(a.x);
                self.lower.y = self.lower.y.min(a.y);
                self.upper.x = self.upper.x.max(a.x);
                self.upper.y = self.upper.y.max(a.y);
                self.lower.x = self.lower.x.min(b.x);
                self.lower.y = self.lower.y.min(b.y);
                self.upper.x = self.upper.x.max(b.x);
                self.upper.y = self.upper.y.max(b.y);
                self.area += a.cross(b);
                let arc_start = self.perimeter;
                self.perimeter += (b - a).norm();
                self.segments.push(Segment {
                    a,
                    b,
                    curve,
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
            } else if self.open_boundaries.len() < self.scene.internal_boundaries.len() {
                let boundary = &self.scene.internal_boundaries[self.open_boundaries.len()];
                let sampler = self
                    .open_sampler
                    .get_or_insert_with(|| OpenSampler::new(&boundary.spline, self.options));
                if sampler.step() {
                    match self.open_sampler.take().unwrap().finish() {
                        Err(_) => {
                            self.finish(Some(ValidationIssue::BoundarySubdivision(boundary.id)))
                        }
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
                if a.curve == b.curve {
                    let closed = a.curve < self.loops.len();
                    let points = if closed {
                        &self.loops[a.curve]
                    } else {
                        &self.open_boundaries[a.curve - self.loops.len()]
                    };
                    let n = points.len() - 1;
                    if a.index.abs_diff(b.index) == 1
                        || (closed && a.index.abs_diff(b.index) == n - 1)
                    {
                        continue;
                    }
                    // Knot insertion can introduce arbitrarily short spans on an
                    // unchanged smooth arc. Suppress only local proximity, never
                    // a crossing, using arc distance instead of segment indices.
                    let direct_gap = b.arc_start - a.arc_end;
                    let gap = if closed {
                        direct_gap.min(self.perimeters[a.curve] - b.arc_end + a.arc_start)
                    } else {
                        direct_gap
                    };
                    local_neighbors = gap <= 4.0 * margin;
                } else {
                    let (amin, amax) = self.bounds[a.curve];
                    let (bmin, bmax) = self.bounds[b.curve];
                    if amax.x + margin < bmin.x
                        || bmax.x + margin < amin.x
                        || amax.y + margin < bmin.y
                        || bmax.y + margin < amin.y
                    {
                        // Segments are grouped by obstacle: skip the entire loop.
                        let point_count = if b.curve < self.loops.len() {
                            self.loops[b.curve].len()
                        } else {
                            self.open_boundaries[b.curve - self.loops.len()].len()
                        };
                        self.j += point_count - 2 - b.index;
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
                    let issue = if a.curve == b.curve {
                        if a.curve < self.scene.obstacles.len() {
                            ValidationIssue::SelfContact(self.scene.obstacles[a.curve].id)
                        } else {
                            ValidationIssue::BoundarySelfContact(
                                self.scene.internal_boundaries
                                    [a.curve - self.scene.obstacles.len()]
                                .id,
                            )
                        }
                    } else if a.curve >= self.scene.obstacles.len() {
                        ValidationIssue::BoundaryContact(
                            self.scene.internal_boundaries[a.curve - self.scene.obstacles.len()].id,
                        )
                    } else if b.curve >= self.scene.obstacles.len() {
                        ValidationIssue::BoundaryContact(
                            self.scene.internal_boundaries[b.curve - self.scene.obstacles.len()].id,
                        )
                    } else {
                        ValidationIssue::ObstacleContact(
                            self.scene.obstacles[a.curve].id,
                            self.scene.obstacles[b.curve].id,
                        )
                    };
                    self.finish(Some(issue));
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
        for (boundary, samples) in self
            .scene
            .internal_boundaries
            .iter()
            .zip(&self.open_boundaries)
        {
            let point = samples[0].point;
            let direct_container = self
                .loops
                .iter()
                .enumerate()
                .filter(|(_, polygon)| point_inside_samples(point, polygon))
                .min_by(|(a, _), (b, _)| self.perimeters[*a].total_cmp(&self.perimeters[*b]));
            let expected = match direct_container {
                None => Some(BACKGROUND_REGION),
                Some((index, _)) => self.scene.obstacles[index].role.interior(),
            };
            if expected != Some(boundary.region) {
                return Some(ValidationIssue::BoundaryRegionTopology(boundary.id));
            }
        }
        None
    }
}

fn point_inside_samples(point: Point2, polygon: &[Sample]) -> bool {
    let mut inside = false;
    for edge in polygon.windows(2) {
        let a = edge[0].point;
        let b = edge[1].point;
        if (a.y > point.y) != (b.y > point.y)
            && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
        {
            inside = !inside;
        }
    }
    inside
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
