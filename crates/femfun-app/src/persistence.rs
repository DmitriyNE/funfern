use crate::editor::Document;
use femfun_core::*;
use serde::{Deserialize, Serialize};
pub const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    version: u32,
    domain: [f64; 4],
    draft: Vec<Loop>,
    accepted: Vec<Loop>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Loop {
    id: u64,
    controls: Vec<[f64; 2]>,
    intervals: Vec<f64>,
}
fn encode_scene(scene: &Scene) -> Vec<Loop> {
    scene
        .obstacles
        .iter()
        .map(|o| Loop {
            id: o.id.0,
            controls: o.spline.controls().iter().map(|p| [p.x, p.y]).collect(),
            intervals: o.spline.intervals().to_vec(),
        })
        .collect()
}
fn decode_scene(loops: Vec<Loop>) -> Result<Scene, String> {
    if loops.len() > MAX_OBSTACLES {
        return Err("Maximum 32 obstacles".into());
    }
    let mut obstacles = vec![];
    for o in loops {
        if o.controls.iter().flatten().any(|v| !v.is_finite()) {
            return Err("Coordinates must be finite".into());
        }
        if o.id == u64::MAX {
            return Err("Obstacle ID cannot be u64::MAX".into());
        }
        obstacles.push(Obstacle {
            id: ObstacleId(o.id),
            spline: PeriodicCubicSpline::new(
                o.controls
                    .into_iter()
                    .map(|[x, y]| Point2::new(x, y))
                    .collect(),
                o.intervals,
            )
            .map_err(|e| e.to_string())?,
        });
    }
    let scene = Scene { obstacles };
    if !scene.structure_valid() {
        return Err("Duplicate or invalid obstacle IDs".into());
    }
    Ok(scene)
}
pub fn save(document: &Document) -> Result<String, String> {
    serde_json::to_string_pretty(&File {
        version: 1,
        domain: [-1.0, 1.0, -1.0, 1.0],
        draft: encode_scene(&document.draft),
        accepted: encode_scene(&document.accepted),
    })
    .map_err(|e| e.to_string())
}
/// Structural parsing only. UI advances LoadCandidate across frames before replacing.
pub fn parse(bytes: &[u8]) -> Result<LoadCandidate, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("File exceeds 2 MiB".into());
    }
    let file: File = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if file.version != 1 || file.domain != [-1.0, 1.0, -1.0, 1.0] {
        return Err("Unsupported scene version or domain".into());
    }
    let document = Document {
        draft: decode_scene(file.draft)?,
        accepted: decode_scene(file.accepted)?,
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
