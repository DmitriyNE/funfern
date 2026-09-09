use crate::editor::Document;
use funfern_core::*;
use serde::{Deserialize, Serialize};

pub const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
const DOMAIN: [f64; 4] = [-1.0, 1.0, -1.0, 1.0];

#[derive(Deserialize)]
struct Header {
    version: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileV1 {
    version: u32,
    domain: [f64; 4],
    draft: Vec<StoredLoopV1>,
    accepted: Vec<StoredLoopV1>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredLoopV1 {
    id: u64,
    controls: Vec<[f64; 2]>,
    intervals: Vec<f64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileV2 {
    version: u32,
    domain: [f64; 4],
    draft: StoredScene,
    accepted: StoredScene,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredScene {
    materials: Vec<StoredMaterial>,
    regions: Vec<StoredRegion>,
    loops: Vec<StoredLoop>,
    #[serde(default)]
    internal_boundaries: Vec<StoredInternalBoundary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    outer_boundaries: Option<[StoredOuterBoundaryCondition; 4]>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredMaterial {
    id: u64,
    name: String,
    mass_density: f64,
    stiffness: f64,
    damping: f64,
    color: [u8; 3],
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredRegion {
    id: u64,
    material: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredLoop {
    id: u64,
    role: StoredRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    span_conditions: Option<Vec<StoredFaceCondition>>,
    controls: Vec<[f64; 2]>,
    intervals: Vec<f64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredInternalBoundary {
    id: u64,
    region: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    law: Option<StoredInternalBoundaryLaw>,
    #[serde(default)]
    span_laws: Vec<StoredSpanLaw>,
    controls: Vec<[f64; 2]>,
    intervals: Vec<f64>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredInternalBoundaryLaw {
    Reflecting,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredSpanLaw {
    left: StoredFaceCondition,
    right: StoredFaceCondition,
    coupling: StoredCoupling,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredFaceCondition {
    Reflecting,
    Impedance { ratio: f64 },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredOuterBoundaryCondition {
    Reflecting,
    FirstOrderOutgoing,
    SecondOrderOutgoing,
    Neumann { signal: StoredBoundarySignal },
    Dirichlet { signal: StoredBoundarySignal },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredBoundarySignal {
    offset: f64,
    amplitude: f64,
    frequency_hz: f64,
    phase_radians: f64,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredCoupling {
    Independent,
    ThinGap { stiffness_ratio: f64 },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredRole {
    Hole { exterior: u64 },
    MaterialInterface { exterior: u64, interior: u64 },
    Wall { exterior: u64, interior: u64 },
}

fn encode_scene(scene: &Scene) -> StoredScene {
    StoredScene {
        materials: scene
            .materials
            .iter()
            .map(|material| StoredMaterial {
                id: material.id.0,
                name: material.name.clone(),
                mass_density: material.mass_density,
                stiffness: material.stiffness,
                damping: material.damping,
                color: material.color,
            })
            .collect(),
        regions: scene
            .regions
            .iter()
            .map(|region| StoredRegion {
                id: region.id.0,
                material: region.material.0,
            })
            .collect(),
        loops: scene
            .obstacles
            .iter()
            .map(|loop_| StoredLoop {
                id: loop_.id.0,
                role: match loop_.role {
                    LoopRole::Hole { exterior } => StoredRole::Hole {
                        exterior: exterior.0,
                    },
                    LoopRole::MaterialInterface { exterior, interior } => {
                        StoredRole::MaterialInterface {
                            exterior: exterior.0,
                            interior: interior.0,
                        }
                    }
                    LoopRole::Wall { exterior, interior } => StoredRole::Wall {
                        exterior: exterior.0,
                        interior: interior.0,
                    },
                },
                span_conditions: Some(
                    loop_
                        .span_conditions
                        .iter()
                        .copied()
                        .map(encode_face_condition)
                        .collect(),
                ),
                controls: loop_
                    .spline
                    .controls()
                    .iter()
                    .map(|point| [point.x, point.y])
                    .collect(),
                intervals: loop_.spline.intervals().to_vec(),
            })
            .collect(),
        internal_boundaries: scene
            .internal_boundaries
            .iter()
            .map(|boundary| StoredInternalBoundary {
                id: boundary.id.0,
                region: boundary.region.0,
                law: None,
                span_laws: boundary
                    .span_laws
                    .iter()
                    .map(|law| StoredSpanLaw {
                        left: encode_face_condition(law.left),
                        right: encode_face_condition(law.right),
                        coupling: match law.coupling {
                            InternalBoundaryCoupling::Independent => StoredCoupling::Independent,
                            InternalBoundaryCoupling::ThinGap { stiffness_ratio } => {
                                StoredCoupling::ThinGap { stiffness_ratio }
                            }
                        },
                    })
                    .collect(),
                controls: boundary
                    .spline
                    .controls()
                    .iter()
                    .map(|point| [point.x, point.y])
                    .collect(),
                intervals: boundary.spline.intervals().to_vec(),
            })
            .collect(),
        outer_boundaries: Some(scene.outer_boundaries.sides.map(encode_outer_condition)),
    }
}

fn encode_signal(signal: BoundarySignal) -> StoredBoundarySignal {
    StoredBoundarySignal {
        offset: signal.offset,
        amplitude: signal.amplitude,
        frequency_hz: signal.frequency_hz,
        phase_radians: signal.phase_radians,
    }
}

fn decode_signal(signal: StoredBoundarySignal) -> BoundarySignal {
    BoundarySignal {
        offset: signal.offset,
        amplitude: signal.amplitude,
        frequency_hz: signal.frequency_hz,
        phase_radians: signal.phase_radians,
    }
}

fn encode_outer_condition(condition: OuterBoundaryCondition) -> StoredOuterBoundaryCondition {
    match condition {
        OuterBoundaryCondition::Reflecting => StoredOuterBoundaryCondition::Reflecting,
        OuterBoundaryCondition::FirstOrderOutgoing => {
            StoredOuterBoundaryCondition::FirstOrderOutgoing
        }
        OuterBoundaryCondition::SecondOrderOutgoing => {
            StoredOuterBoundaryCondition::SecondOrderOutgoing
        }
        OuterBoundaryCondition::Neumann { signal } => StoredOuterBoundaryCondition::Neumann {
            signal: encode_signal(signal),
        },
        OuterBoundaryCondition::Dirichlet { signal } => StoredOuterBoundaryCondition::Dirichlet {
            signal: encode_signal(signal),
        },
    }
}

fn decode_outer_condition(condition: StoredOuterBoundaryCondition) -> OuterBoundaryCondition {
    match condition {
        StoredOuterBoundaryCondition::Reflecting => OuterBoundaryCondition::Reflecting,
        StoredOuterBoundaryCondition::FirstOrderOutgoing => {
            OuterBoundaryCondition::FirstOrderOutgoing
        }
        StoredOuterBoundaryCondition::SecondOrderOutgoing => {
            OuterBoundaryCondition::SecondOrderOutgoing
        }
        StoredOuterBoundaryCondition::Neumann { signal } => OuterBoundaryCondition::Neumann {
            signal: decode_signal(signal),
        },
        StoredOuterBoundaryCondition::Dirichlet { signal } => OuterBoundaryCondition::Dirichlet {
            signal: decode_signal(signal),
        },
    }
}

fn encode_face_condition(condition: FaceBoundaryCondition) -> StoredFaceCondition {
    match condition {
        FaceBoundaryCondition::Reflecting => StoredFaceCondition::Reflecting,
        FaceBoundaryCondition::Impedance { ratio } => StoredFaceCondition::Impedance { ratio },
    }
}

fn decode_face_condition(condition: StoredFaceCondition) -> FaceBoundaryCondition {
    match condition {
        StoredFaceCondition::Reflecting => FaceBoundaryCondition::Reflecting,
        StoredFaceCondition::Impedance { ratio } => FaceBoundaryCondition::Impedance { ratio },
    }
}

fn decode_spline(
    controls: Vec<[f64; 2]>,
    intervals: Vec<f64>,
) -> Result<PeriodicCubicSpline, String> {
    if controls.iter().flatten().any(|value| !value.is_finite()) {
        return Err("Coordinates must be finite".into());
    }
    PeriodicCubicSpline::new(
        controls
            .into_iter()
            .map(|[x, y]| Point2::new(x, y))
            .collect(),
        intervals,
    )
    .map_err(|error| error.to_string())
}

fn decode_open_spline(
    controls: Vec<[f64; 2]>,
    intervals: Vec<f64>,
) -> Result<OpenCubicSpline, String> {
    if controls.iter().flatten().any(|value| !value.is_finite()) {
        return Err("Coordinates must be finite".into());
    }
    OpenCubicSpline::new(
        controls
            .into_iter()
            .map(|[x, y]| Point2::new(x, y))
            .collect(),
        intervals,
    )
    .map_err(|error| error.to_string())
}

fn decode_v1(loops: Vec<StoredLoopV1>) -> Result<Scene, String> {
    if loops.len() > MAX_OBSTACLES {
        return Err("Maximum 32 loops".into());
    }
    let obstacles = loops
        .into_iter()
        .map(|loop_| {
            if loop_.id == 0 || loop_.id == u64::MAX {
                return Err("Loop ID is outside the supported range".into());
            }
            Ok(Obstacle::hole(
                ObstacleId(loop_.id),
                decode_spline(loop_.controls, loop_.intervals)?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let scene = Scene {
        obstacles,
        ..Scene::default()
    };
    if !scene.structure_valid() {
        return Err("Duplicate or invalid loop IDs".into());
    }
    Ok(scene)
}

fn decode_scene(
    stored: StoredScene,
    require_loop_conditions: bool,
    require_outer_boundaries: bool,
) -> Result<Scene, String> {
    if stored.loops.len() > MAX_OBSTACLES
        || stored.internal_boundaries.len() > MAX_INTERNAL_BOUNDARIES
        || stored.loops.len() + stored.internal_boundaries.len() > MAX_OBSTACLES
        || stored.materials.len() > MAX_MATERIALS
    {
        return Err("Scene exceeds the loop or material limit".into());
    }
    let materials = stored
        .materials
        .into_iter()
        .map(|material| Material {
            id: MaterialId(material.id),
            name: material.name,
            mass_density: material.mass_density,
            stiffness: material.stiffness,
            damping: material.damping,
            color: material.color,
        })
        .collect();
    let regions = stored
        .regions
        .into_iter()
        .map(|region| Region {
            id: RegionId(region.id),
            material: MaterialId(region.material),
        })
        .collect();
    let obstacles = stored
        .loops
        .into_iter()
        .map(|loop_| {
            if loop_.id == 0 || loop_.id == u64::MAX {
                return Err("Loop ID is outside the supported range".into());
            }
            let role = match loop_.role {
                StoredRole::Hole { exterior } => LoopRole::Hole {
                    exterior: RegionId(exterior),
                },
                StoredRole::MaterialInterface { exterior, interior } => {
                    LoopRole::MaterialInterface {
                        exterior: RegionId(exterior),
                        interior: RegionId(interior),
                    }
                }
                StoredRole::Wall { exterior, interior } => LoopRole::Wall {
                    exterior: RegionId(exterior),
                    interior: RegionId(interior),
                },
            };
            let spline = decode_spline(loop_.controls, loop_.intervals)?;
            let span_conditions = match loop_.span_conditions {
                Some(conditions) => conditions.into_iter().map(decode_face_condition).collect(),
                None if !require_loop_conditions => {
                    vec![FaceBoundaryCondition::Reflecting; spline.intervals().len()]
                }
                None => return Err("Loop has no span boundary conditions".into()),
            };
            Ok(Obstacle {
                id: ObstacleId(loop_.id),
                spline,
                role,
                span_conditions,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let internal_boundaries = stored
        .internal_boundaries
        .into_iter()
        .map(|boundary| {
            if boundary.id == 0 || boundary.id == u64::MAX {
                return Err("Internal-boundary ID is outside the supported range".into());
            }
            if boundary.law.is_some() && !boundary.span_laws.is_empty() {
                return Err("Internal boundary mixes legacy and span laws".into());
            }
            let spline = decode_open_spline(boundary.controls, boundary.intervals)?;
            let span_laws = if boundary.span_laws.is_empty() {
                match boundary.law {
                    Some(StoredInternalBoundaryLaw::Reflecting) => {
                        vec![InternalBoundaryLaw::REFLECTING; spline.intervals().len()]
                    }
                    None => return Err("Internal boundary has no span laws".into()),
                }
            } else {
                boundary
                    .span_laws
                    .into_iter()
                    .map(|law| InternalBoundaryLaw {
                        left: decode_face_condition(law.left),
                        right: decode_face_condition(law.right),
                        coupling: match law.coupling {
                            StoredCoupling::Independent => InternalBoundaryCoupling::Independent,
                            StoredCoupling::ThinGap { stiffness_ratio } => {
                                InternalBoundaryCoupling::ThinGap { stiffness_ratio }
                            }
                        },
                    })
                    .collect()
            };
            Ok(InternalBoundary {
                id: InternalBoundaryId(boundary.id),
                spline,
                region: RegionId(boundary.region),
                span_laws,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let outer_boundaries = match stored.outer_boundaries {
        Some(conditions) => OuterBoundaryConditions {
            sides: conditions.map(decode_outer_condition),
        },
        None if !require_outer_boundaries => OuterBoundaryConditions::default(),
        None => return Err("Scene has no outer boundary conditions".into()),
    };
    let scene = Scene {
        obstacles,
        internal_boundaries,
        materials,
        regions,
        outer_boundaries,
    };
    if !scene.structure_valid() {
        return Err("Scene contains invalid IDs, materials, or region references".into());
    }
    Ok(scene)
}

pub fn save(document: &Document) -> Result<String, String> {
    serde_json::to_string_pretty(&FileV2 {
        version: 6,
        domain: DOMAIN,
        draft: encode_scene(&document.draft),
        accepted: encode_scene(&document.accepted),
    })
    .map_err(|error| error.to_string())
}

/// Structural parsing only. UI advances LoadCandidate across frames before replacing.
pub fn parse(bytes: &[u8]) -> Result<LoadCandidate, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("File exceeds 2 MiB".into());
    }
    let header: Header = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let document = match header.version {
        1 => {
            let file: FileV1 = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
            if file.domain != DOMAIN {
                return Err("Unsupported scene domain".into());
            }
            Document {
                draft: decode_v1(file.draft)?,
                accepted: decode_v1(file.accepted)?,
            }
        }
        2..=6 => {
            let file: FileV2 = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
            if file.domain != DOMAIN {
                return Err("Unsupported scene domain".into());
            }
            Document {
                draft: decode_scene(file.draft, header.version >= 5, header.version >= 6)?,
                accepted: decode_scene(file.accepted, header.version >= 5, header.version >= 6)?,
            }
        }
        _ => return Err("Unsupported scene version".into()),
    };
    let job = ValidationJob::new(document.accepted.clone(), 0);
    Ok(LoadCandidate { document, job })
}

pub struct LoadCandidate {
    document: Document,
    job: ValidationJob,
}

impl LoadCandidate {
    pub fn advance(&mut self, budget: usize) -> Option<Result<Document, String>> {
        self.job.advance(budget).map(|result| match result.issue {
            Some(issue) => Err(format!("Invalid accepted scene: {issue}")),
            None => Ok(self.document.clone()),
        })
    }
}
