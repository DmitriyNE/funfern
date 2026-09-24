//! Version-22 persistence for the unified topology document.
//!
//! This is deliberately a hard schema boundary: production accepts only version
//! 22, and there is no legacy geometry adapter behind it. The whole schema
//! lives here, value codecs included, so a field that changes shape changes in
//! one file.

use crate::document::{
    AdaptationSettings, MAX_PROBES, MAX_SEGMENT_PROBE_POINTS, MaterialOverlay, MaterialProperty,
    PresentationSettings, ProbeId, ProbeSamplingPreset, VectorOverlay,
};
use crate::topology_editor::{
    TopologyBoundaryProbeTarget, TopologyDocument, TopologyDocumentModel, TopologyProbeDefinition,
    TopologyProbeTarget,
};
use funfern_core::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const TOPOLOGY_FILE_VERSION: u32 = 22;
pub const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Deserialize)]
struct Header {
    version: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileV22 {
    version: u32,
    model: StoredTopologyDocumentModel,
    presentation: StoredPresentation,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredTopologyDocumentModel {
    draft: StoredTopologyScene,
    accepted: StoredTopologyScene,
    probes: Vec<StoredTopologyProbe>,
    source: StoredPointSource,
    far_field: StoredFarField,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredTopologyScene {
    domain: [f64; 4],
    physics: StoredPhysicsModel,
    materials: Vec<StoredMaterial>,
    regions: Vec<StoredRegion>,
    face_assignments: Vec<StoredFaceAssignment>,
    volume_sources: Vec<StoredVolumeSource>,
    outer_boundaries: [StoredOuterBoundaryCondition; 4],
    curves: Vec<StoredCurve>,
    vertices: Vec<StoredVertex>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredMaterial {
    id: u64,
    name: String,
    mass_density: StoredScalarField,
    stiffness: StoredScalarField,
    damping: StoredScalarField,
    axis_ratio: StoredScalarField,
    parameters: Vec<StoredParameter>,
    color: [u8; 3],
    #[serde(default = "stored_linear_coefficient_law")]
    mass_law: StoredCoefficientLaw,
    #[serde(default = "stored_linear_coefficient_law")]
    stiffness_law: StoredCoefficientLaw,
    #[serde(default)]
    electric_loss: Option<StoredLossChannel>,
    #[serde(default)]
    magnetic_loss: Option<StoredLossChannel>,
    #[serde(default = "stored_no_restoring_law")]
    restoring: StoredRestoringLaw,
    #[serde(default)]
    switch_ramp: f64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredParameter {
    name: String,
    value: f64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredRegion {
    id: u64,
    material: u64,
    frame: StoredFrame,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredFrame {
    origin: [f64; 2],
    angle_radians: f64,
    attachment: StoredFrameAttachment,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredFrameAttachment {
    World,
    FollowRegion,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredVolumeSource {
    region: u64,
    enabled: bool,
    profile: StoredScalarField,
    parameters: Vec<StoredParameter>,
    signal: StoredTimeSignal,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredCurve {
    id: u64,
    spline: StoredSpline,
    nodes: Vec<Option<u64>>,
    spans: Vec<StoredSpan>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredSpline {
    Closed {
        controls: Vec<[f64; 2]>,
        intervals: Vec<f64>,
        multiplicities: Vec<u8>,
    },
    Open {
        controls: Vec<[f64; 2]>,
        intervals: Vec<f64>,
        multiplicities: Vec<u8>,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredSpan {
    id: u64,
    behavior: StoredSpanBehavior,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredSpanBehavior {
    Transmitting,
    Boundary {
        left: StoredFaceCondition,
        right: StoredFaceCondition,
        coupling: StoredCoupling,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredCoupling {
    Independent,
    ThinGap { stiffness_ratio: f64 },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredVertex {
    id: u64,
    location: StoredVertexLocation,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredVertexLocation {
    Free {
        point: [f64; 2],
    },
    Interior {
        point: [f64; 2],
    },
    Outer {
        side: StoredOuterSide,
        fraction: f64,
    },
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredOuterSide {
    Bottom,
    Right,
    Top,
    Left,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredFaceAssignment {
    anchor: StoredFaceAnchor,
    region: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredFaceAnchor {
    Outer {
        side: StoredOuterSide,
        fraction: f64,
    },
    Curve {
        curve: u64,
        span: u64,
        side: StoredCurveSide,
        parameter: f64,
    },
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredCurveSide {
    Left,
    Right,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredTopologyProbe {
    id: u64,
    name: String,
    color: [u8; 3],
    enabled: bool,
    target: StoredTopologyProbeTarget,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredTopologyProbeTarget {
    Point {
        position: [f64; 2],
    },
    Segment {
        start: [f64; 2],
        end: [f64; 2],
        preset: StoredProbeSamplingPreset,
    },
    Boundary {
        curve: u64,
        spans: Vec<u64>,
        side: StoredCurveSide,
        reversed: bool,
        preset: StoredProbeSamplingPreset,
    },
    AreaDisk {
        center: [f64; 2],
        radius: f64,
    },
    AreaRegion {
        region: u64,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPointSource {
    enabled: bool,
    position: [f64; 2],
    width: f64,
    region: u64,
    signal: StoredTimeSignal,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredFarField {
    enabled: bool,
    inset: f64,
}

pub fn save(document: &TopologyDocument) -> Result<String, String> {
    validate_document(document)?;
    serde_json::to_string_pretty(&encode_document(document)).map_err(|error| error.to_string())
}

pub fn save_compact(document: &TopologyDocument) -> Result<Vec<u8>, String> {
    validate_document(document)?;
    serde_json::to_vec(&encode_document(document)).map_err(|error| error.to_string())
}

pub fn parse_document(bytes: &[u8]) -> Result<TopologyDocument, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(format!("Scene file exceeds {MAX_FILE_BYTES} bytes"));
    }
    let header: Header = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if header.version != TOPOLOGY_FILE_VERSION {
        return Err(format!(
            "Unsupported scene version {}; Funfern now requires version {TOPOLOGY_FILE_VERSION}",
            header.version
        ));
    }
    let file: FileV22 = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let document = decode_document(file)?;
    validate_document(&document)?;
    Ok(document)
}

/// A bounded-load compatible wrapper used by the application event loop. JSON
/// decoding is already capacity bounded before construction; validation is
/// performed by `TopologyEditor::from_document` before replacement.
pub struct TopologyLoadCandidate {
    document: Option<TopologyDocument>,
}

impl TopologyLoadCandidate {
    pub fn advance(&mut self, _budget: usize) -> Option<Result<TopologyDocument, String>> {
        self.document.take().map(Ok)
    }
}

pub fn parse(bytes: &[u8]) -> Result<TopologyLoadCandidate, String> {
    parse_document(bytes).map(candidate)
}

pub fn candidate(document: TopologyDocument) -> TopologyLoadCandidate {
    TopologyLoadCandidate {
        document: Some(document),
    }
}

fn encode_document(document: &TopologyDocument) -> FileV22 {
    FileV22 {
        version: TOPOLOGY_FILE_VERSION,
        model: StoredTopologyDocumentModel {
            draft: encode_scene(&document.model.draft),
            accepted: encode_scene(&document.model.accepted),
            probes: document.model.probes.iter().map(encode_probe).collect(),
            source: StoredPointSource {
                enabled: document.model.source.enabled,
                position: [
                    document.model.source.position.x,
                    document.model.source.position.y,
                ],
                width: document.model.source.width,
                region: document.model.source.region.0,
                signal: encode_signal(document.model.source.signal),
            },
            far_field: StoredFarField {
                enabled: document.model.far_field.enabled,
                inset: document.model.far_field.inset,
            },
        },
        presentation: encode_presentation(document.presentation),
    }
}

fn decode_document(file: FileV22) -> Result<TopologyDocument, String> {
    Ok(TopologyDocument {
        model: TopologyDocumentModel {
            draft: decode_scene(file.model.draft)?,
            accepted: decode_scene(file.model.accepted)?,
            probes: file
                .model
                .probes
                .into_iter()
                .map(decode_probe)
                .collect::<Result<_, _>>()?,
            source: PointSource {
                enabled: file.model.source.enabled,
                position: Point2::new(file.model.source.position[0], file.model.source.position[1]),
                width: file.model.source.width,
                region: RegionId(file.model.source.region),
                signal: decode_signal(file.model.source.signal),
            },
            far_field: crate::document::FarFieldSettings {
                enabled: file.model.far_field.enabled,
                inset: file.model.far_field.inset,
            },
        },
        presentation: decode_presentation(file.presentation)?,
    })
}

fn encode_scene(scene: &TopologyScene) -> StoredTopologyScene {
    StoredTopologyScene {
        domain: [
            scene.geometry.domain.min_x,
            scene.geometry.domain.max_x,
            scene.geometry.domain.min_y,
            scene.geometry.domain.max_y,
        ],
        physics: encode_physics(scene.physics),
        materials: scene
            .materials
            .iter()
            .map(|material| StoredMaterial {
                id: material.id.0,
                name: material.name.clone(),
                mass_density: encode_scalar_field(&material.mass_density),
                stiffness: encode_scalar_field(&material.stiffness),
                damping: encode_scalar_field(&material.damping),
                axis_ratio: encode_scalar_field(&material.axis_ratio),
                parameters: encode_parameters(&material.parameters),
                color: material.color,
                mass_law: encode_coefficient_law(&material.mass_law),
                stiffness_law: encode_coefficient_law(&material.stiffness_law),
                electric_loss: material.electric_loss.as_ref().map(encode_loss_channel),
                magnetic_loss: material.magnetic_loss.as_ref().map(encode_loss_channel),
                restoring: encode_restoring_law(&material.restoring),
                switch_ramp: material.switch_ramp,
            })
            .collect(),
        regions: scene
            .regions
            .iter()
            .map(|region| StoredRegion {
                id: region.id.0,
                material: region.material.0,
                frame: encode_frame(region.frame),
            })
            .collect(),
        face_assignments: scene
            .face_assignments
            .iter()
            .map(|assignment| StoredFaceAssignment {
                anchor: encode_anchor(assignment.anchor),
                region: assignment.region.map(|region| region.0),
            })
            .collect(),
        volume_sources: scene
            .volume_sources
            .iter()
            .map(|source| StoredVolumeSource {
                region: source.region.0,
                enabled: source.enabled,
                profile: encode_scalar_field(&source.profile),
                parameters: encode_parameters(&source.parameters),
                signal: encode_signal(source.signal),
            })
            .collect(),
        outer_boundaries: scene.outer_boundaries.sides.map(encode_outer_condition),
        curves: scene.geometry.curves.iter().map(encode_curve).collect(),
        vertices: scene
            .geometry
            .vertices
            .iter()
            .map(|vertex| StoredVertex {
                id: vertex.id.0,
                location: match vertex.location {
                    TopologyVertexLocation::Free(point) => StoredVertexLocation::Free {
                        point: [point.x, point.y],
                    },
                    TopologyVertexLocation::Interior(point) => StoredVertexLocation::Interior {
                        point: [point.x, point.y],
                    },
                    TopologyVertexLocation::Outer { side, fraction } => {
                        StoredVertexLocation::Outer {
                            side: encode_outer_side(side),
                            fraction,
                        }
                    }
                },
            })
            .collect(),
    }
}

fn decode_scene(stored: StoredTopologyScene) -> Result<TopologyScene, String> {
    let domain = DomainRect {
        min_x: stored.domain[0],
        max_x: stored.domain[1],
        min_y: stored.domain[2],
        max_y: stored.domain[3],
    };
    if !domain.valid() {
        return Err("Scene contains an invalid domain".into());
    }
    let curves = stored
        .curves
        .into_iter()
        .map(decode_curve)
        .collect::<Result<Vec<_>, _>>()?;
    let scene = TopologyScene {
        geometry: TopologyGeometry {
            domain,
            curves,
            vertices: stored
                .vertices
                .into_iter()
                .map(|vertex| TopologyVertex {
                    id: TopologyVertexId(vertex.id),
                    location: match vertex.location {
                        StoredVertexLocation::Free { point } => {
                            TopologyVertexLocation::Free(Point2::new(point[0], point[1]))
                        }
                        StoredVertexLocation::Interior { point } => {
                            TopologyVertexLocation::Interior(Point2::new(point[0], point[1]))
                        }
                        StoredVertexLocation::Outer { side, fraction } => {
                            TopologyVertexLocation::Outer {
                                side: decode_outer_side(side),
                                fraction,
                            }
                        }
                    },
                })
                .collect(),
        },
        physics: decode_physics(stored.physics),
        materials: stored
            .materials
            .into_iter()
            .map(|material| {
                Ok(Material {
                    id: MaterialId(material.id),
                    name: material.name,
                    mass_density: decode_scalar_field(material.mass_density)?,
                    stiffness: decode_scalar_field(material.stiffness)?,
                    damping: decode_scalar_field(material.damping)?,
                    axis_ratio: decode_scalar_field(material.axis_ratio)?,
                    parameters: decode_parameters(material.parameters),
                    color: material.color,
                    mass_law: decode_coefficient_law(material.mass_law)?,
                    stiffness_law: decode_coefficient_law(material.stiffness_law)?,
                    electric_loss: material
                        .electric_loss
                        .map(decode_loss_channel)
                        .transpose()?,
                    magnetic_loss: material
                        .magnetic_loss
                        .map(decode_loss_channel)
                        .transpose()?,
                    restoring: decode_restoring_law(material.restoring)?,
                    switch_ramp: material.switch_ramp,
                })
            })
            .collect::<Result<_, String>>()?,
        regions: stored
            .regions
            .into_iter()
            .map(|region| Region {
                id: RegionId(region.id),
                material: MaterialId(region.material),
                frame: decode_frame(region.frame),
            })
            .collect(),
        face_assignments: stored
            .face_assignments
            .into_iter()
            .map(|assignment| AuthoredFaceAssignment {
                anchor: decode_anchor(assignment.anchor),
                region: assignment.region.map(RegionId),
            })
            .collect(),
        volume_sources: stored
            .volume_sources
            .into_iter()
            .map(|source| {
                Ok(VolumeSource {
                    region: RegionId(source.region),
                    enabled: source.enabled,
                    profile: decode_scalar_field(source.profile)?,
                    parameters: decode_parameters(source.parameters),
                    signal: decode_signal(source.signal),
                })
            })
            .collect::<Result<_, String>>()?,
        outer_boundaries: OuterBoundaryConditions {
            sides: stored.outer_boundaries.map(decode_outer_condition),
        },
    };
    validate_scene_structure(&scene)?;
    Ok(scene)
}

fn encode_curve(curve: &TopologyCurve) -> StoredCurve {
    let spline = match &curve.spline {
        CurveSpline::Closed(spline) => StoredSpline::Closed {
            controls: encode_points(spline.controls()),
            intervals: spline.intervals().to_vec(),
            multiplicities: spline.multiplicities().to_vec(),
        },
        CurveSpline::Open(spline) => StoredSpline::Open {
            controls: encode_points(spline.controls()),
            intervals: spline.intervals().to_vec(),
            multiplicities: spline.multiplicities().to_vec(),
        },
    };
    StoredCurve {
        id: curve.id.0,
        spline,
        nodes: curve
            .nodes
            .iter()
            .map(|node| node.vertex.map(|vertex| vertex.0))
            .collect(),
        spans: curve
            .spans
            .iter()
            .map(|span| StoredSpan {
                id: span.id.0,
                behavior: match span.behavior {
                    SpanBehavior::Transmitting => StoredSpanBehavior::Transmitting,
                    SpanBehavior::Separated {
                        left,
                        right,
                        coupling,
                    } => StoredSpanBehavior::Boundary {
                        left: encode_face_condition(left),
                        right: encode_face_condition(right),
                        coupling: match coupling {
                            InternalBoundaryCoupling::Independent => StoredCoupling::Independent,
                            InternalBoundaryCoupling::ThinGap { stiffness_ratio } => {
                                StoredCoupling::ThinGap { stiffness_ratio }
                            }
                        },
                    },
                },
            })
            .collect(),
    }
}

fn decode_curve(stored: StoredCurve) -> Result<TopologyCurve, String> {
    let spline = match stored.spline {
        StoredSpline::Closed {
            controls,
            intervals,
            multiplicities,
        } => CurveSpline::Closed(decode_spline(controls, intervals, multiplicities)?),
        StoredSpline::Open {
            controls,
            intervals,
            multiplicities,
        } => CurveSpline::Open(decode_open_spline(controls, intervals, multiplicities)?),
    };
    let spans = stored
        .spans
        .into_iter()
        .map(|span| CurveSpan {
            id: CurveSpanId(span.id),
            behavior: match span.behavior {
                StoredSpanBehavior::Transmitting => SpanBehavior::Transmitting,
                StoredSpanBehavior::Boundary {
                    left,
                    right,
                    coupling,
                } => SpanBehavior::Separated {
                    left: decode_face_condition(left),
                    right: decode_face_condition(right),
                    coupling: match coupling {
                        StoredCoupling::Independent => InternalBoundaryCoupling::Independent,
                        StoredCoupling::ThinGap { stiffness_ratio } => {
                            InternalBoundaryCoupling::ThinGap { stiffness_ratio }
                        }
                    },
                },
            },
        })
        .collect();
    let mut curve =
        TopologyCurve::new(CurveId(stored.id), spline, spans).map_err(|issue| issue.to_string())?;
    if stored.nodes.len() != curve.nodes.len() {
        return Err("Scene contains a malformed curve-node table".into());
    }
    curve.nodes = stored
        .nodes
        .into_iter()
        .map(|vertex| CurveNode {
            vertex: vertex.map(TopologyVertexId),
        })
        .collect();
    Ok(curve)
}

fn encode_probe(probe: &TopologyProbeDefinition) -> StoredTopologyProbe {
    StoredTopologyProbe {
        id: probe.id.0,
        name: probe.name.clone(),
        color: probe.color,
        enabled: probe.enabled,
        target: match &probe.target {
            TopologyProbeTarget::Point(position) => StoredTopologyProbeTarget::Point {
                position: [position.x, position.y],
            },
            TopologyProbeTarget::Segment { start, end, preset } => {
                StoredTopologyProbeTarget::Segment {
                    start: [start.x, start.y],
                    end: [end.x, end.y],
                    preset: encode_probe_preset(*preset),
                }
            }
            TopologyProbeTarget::Boundary(target) => StoredTopologyProbeTarget::Boundary {
                curve: target.curve.0,
                spans: target.spans.iter().map(|span| span.0).collect(),
                side: encode_curve_side(target.side),
                reversed: target.reversed,
                preset: encode_probe_preset(target.preset),
            },
            TopologyProbeTarget::AreaDisk { center, radius } => {
                StoredTopologyProbeTarget::AreaDisk {
                    center: [center.x, center.y],
                    radius: *radius,
                }
            }
            TopologyProbeTarget::AreaRegion(region) => {
                StoredTopologyProbeTarget::AreaRegion { region: region.0 }
            }
        },
    }
}

fn decode_probe(stored: StoredTopologyProbe) -> Result<TopologyProbeDefinition, String> {
    Ok(TopologyProbeDefinition {
        id: ProbeId(stored.id),
        name: stored.name,
        color: stored.color,
        enabled: stored.enabled,
        target: match stored.target {
            StoredTopologyProbeTarget::Point { position } => {
                TopologyProbeTarget::Point(Point2::new(position[0], position[1]))
            }
            StoredTopologyProbeTarget::Segment { start, end, preset } => {
                TopologyProbeTarget::Segment {
                    start: Point2::new(start[0], start[1]),
                    end: Point2::new(end[0], end[1]),
                    preset: decode_probe_preset(preset),
                }
            }
            StoredTopologyProbeTarget::Boundary {
                curve,
                spans,
                side,
                reversed,
                preset,
            } => TopologyProbeTarget::Boundary(TopologyBoundaryProbeTarget {
                curve: CurveId(curve),
                spans: spans.into_iter().map(CurveSpanId).collect(),
                side: decode_curve_side(side),
                reversed,
                preset: decode_probe_preset(preset),
            }),
            StoredTopologyProbeTarget::AreaDisk { center, radius } => {
                TopologyProbeTarget::AreaDisk {
                    center: Point2::new(center[0], center[1]),
                    radius,
                }
            }
            StoredTopologyProbeTarget::AreaRegion { region } => {
                TopologyProbeTarget::AreaRegion(RegionId(region))
            }
        },
    })
}

fn validate_document(document: &TopologyDocument) -> Result<(), String> {
    validate_scene_structure(&document.model.draft)?;
    document
        .model
        .accepted
        .compile(0)
        .map_err(|issue| format!("Accepted topology is invalid: {issue}"))?;
    if !document.model.source.valid()
        || document
            .model
            .accepted
            .region(document.model.source.region)
            .is_none()
    {
        return Err("Scene contains an invalid point source".into());
    }
    if !document.model.far_field.valid() || !document.presentation.valid() {
        return Err("Scene contains invalid presentation settings".into());
    }
    validate_probes(&document.model.probes, &document.model.draft)
}

fn validate_scene_structure(scene: &TopologyScene) -> Result<(), String> {
    scene
        .validate_structure()
        .map_err(|issue| issue.to_string())?;
    if scene.geometry.curves.len() > MAX_TOPOLOGY_CURVES
        || scene.geometry.vertices.len() > MAX_TOPOLOGY_VERTICES
    {
        return Err("Scene exceeds topology object limits".into());
    }
    let mut curve_ids = BTreeSet::new();
    let mut span_ids = BTreeSet::new();
    if scene.geometry.curves.iter().any(|curve| {
        curve.id.0 == 0
            || !curve_ids.insert(curve.id)
            || curve
                .spans
                .iter()
                .any(|span| span.id.0 == 0 || !span_ids.insert(span.id))
    }) {
        return Err("Scene contains duplicate or invalid curve IDs".into());
    }
    let mut vertex_ids = BTreeSet::new();
    if scene.geometry.vertices.iter().any(|vertex| {
        vertex.id.0 == 0
            || !vertex_ids.insert(vertex.id)
            || vertex.point(scene.geometry.domain).is_none()
    }) {
        return Err("Scene contains duplicate or invalid topology vertices".into());
    }
    if scene
        .geometry
        .curves
        .iter()
        .flat_map(|curve| &curve.nodes)
        .filter_map(|node| node.vertex)
        .any(|vertex| !vertex_ids.contains(&vertex))
    {
        return Err("Curve references a missing topology vertex".into());
    }
    let regions = scene
        .regions
        .iter()
        .map(|region| region.id)
        .collect::<BTreeSet<_>>();
    for assignment in &scene.face_assignments {
        if assignment
            .region
            .is_some_and(|region| !regions.contains(&region))
            || !anchor_structure_valid(assignment.anchor, scene)
        {
            return Err("Scene contains a malformed face assignment".into());
        }
    }
    Ok(())
}

fn validate_probes(
    probes: &[TopologyProbeDefinition],
    scene: &TopologyScene,
) -> Result<(), String> {
    if probes.len() > MAX_PROBES {
        return Err(format!("Scene contains more than {MAX_PROBES} probes"));
    }
    let mut ids = BTreeSet::new();
    let mut sampling_points = 0;
    for probe in probes {
        if probe.id.0 == 0
            || !ids.insert(probe.id)
            || probe.name.is_empty()
            || probe.name.len() > 64
            || probe.name.trim().is_empty()
        {
            return Err("Scene contains an invalid probe".into());
        }
        match &probe.target {
            TopologyProbeTarget::Point(point) if !point.finite() => {
                return Err("Point probe is not finite".into());
            }
            TopologyProbeTarget::Segment { start, end, preset } => {
                if !start.finite() || !end.finite() || start == end {
                    return Err("Scene contains an invalid segment probe".into());
                }
                sampling_points += preset.spatial_points();
            }
            TopologyProbeTarget::Boundary(target) => {
                let curve = scene
                    .geometry
                    .curves
                    .iter()
                    .find(|curve| curve.id == target.curve)
                    .ok_or("Boundary probe references a missing curve")?;
                if target.spans.is_empty()
                    || target
                        .spans
                        .iter()
                        .any(|span| !curve.spans.iter().any(|candidate| candidate.id == *span))
                    || !contiguous_span_path(curve, &target.spans)
                {
                    return Err("Scene contains an invalid boundary probe".into());
                }
                sampling_points += target.preset.spatial_points();
            }
            TopologyProbeTarget::AreaDisk { center, radius }
                if !center.finite() || !radius.is_finite() || *radius <= 0.0 =>
            {
                return Err("Scene contains an invalid disk probe".into());
            }
            TopologyProbeTarget::AreaRegion(region) if scene.region(*region).is_none() => {
                return Err("Area probe references a missing region".into());
            }
            _ => {}
        }
    }
    if sampling_points > MAX_SEGMENT_PROBE_POINTS {
        return Err(format!(
            "Line probes exceed the {MAX_SEGMENT_PROBE_POINTS}-point sampling budget"
        ));
    }
    Ok(())
}

fn contiguous_span_path(curve: &TopologyCurve, spans: &[CurveSpanId]) -> bool {
    let indices = spans
        .iter()
        .map(|span| {
            curve
                .spans
                .iter()
                .position(|candidate| candidate.id == *span)
        })
        .collect::<Option<Vec<_>>>();
    let Some(indices) = indices else { return false };
    let mut unique = BTreeSet::new();
    indices.iter().all(|index| unique.insert(*index))
        && indices.windows(2).all(|pair| {
            pair[1] == pair[0] + 1
                || matches!(curve.spline, CurveSpline::Closed(_))
                    && pair[0] + 1 == curve.spans.len()
                    && pair[1] == 0
        })
}

fn anchor_structure_valid(anchor: FaceAnchor, scene: &TopologyScene) -> bool {
    match anchor {
        FaceAnchor::Outer { fraction, .. } => {
            fraction.is_finite() && (0.0..=1.0).contains(&fraction)
        }
        FaceAnchor::Curve {
            curve,
            span,
            parameter,
            ..
        } => scene
            .geometry
            .curves
            .iter()
            .find(|candidate| candidate.id == curve)
            .and_then(|curve| {
                curve
                    .spans
                    .iter()
                    .position(|candidate| candidate.id == span)
                    .and_then(|index| curve.spline.span_bounds(index))
            })
            .is_some_and(|[a, b]| {
                parameter.is_finite() && parameter >= a.min(b) && parameter <= a.max(b)
            }),
    }
}

fn encode_parameters(parameters: &[MaterialParameter]) -> Vec<StoredParameter> {
    parameters
        .iter()
        .map(|parameter| StoredParameter {
            name: parameter.name.clone(),
            value: parameter.value,
        })
        .collect()
}

fn decode_parameters(parameters: Vec<StoredParameter>) -> Vec<MaterialParameter> {
    parameters
        .into_iter()
        .map(|parameter| MaterialParameter {
            name: parameter.name,
            value: parameter.value,
        })
        .collect()
}

fn encode_frame(frame: MaterialFrame) -> StoredFrame {
    StoredFrame {
        origin: [frame.origin.x, frame.origin.y],
        angle_radians: frame.angle_radians,
        attachment: match frame.attachment {
            MaterialFrameAttachment::World => StoredFrameAttachment::World,
            MaterialFrameAttachment::FollowRegion => StoredFrameAttachment::FollowRegion,
        },
    }
}

fn decode_frame(frame: StoredFrame) -> MaterialFrame {
    MaterialFrame {
        origin: Point2::new(frame.origin[0], frame.origin[1]),
        angle_radians: frame.angle_radians,
        attachment: match frame.attachment {
            StoredFrameAttachment::World => MaterialFrameAttachment::World,
            StoredFrameAttachment::FollowRegion => MaterialFrameAttachment::FollowRegion,
        },
    }
}

fn encode_anchor(anchor: FaceAnchor) -> StoredFaceAnchor {
    match anchor {
        FaceAnchor::Outer { side, fraction } => StoredFaceAnchor::Outer {
            side: encode_outer_side(side),
            fraction,
        },
        FaceAnchor::Curve {
            curve,
            span,
            side,
            parameter,
        } => StoredFaceAnchor::Curve {
            curve: curve.0,
            span: span.0,
            side: encode_curve_side(side),
            parameter,
        },
    }
}

fn decode_anchor(anchor: StoredFaceAnchor) -> FaceAnchor {
    match anchor {
        StoredFaceAnchor::Outer { side, fraction } => FaceAnchor::Outer {
            side: decode_outer_side(side),
            fraction,
        },
        StoredFaceAnchor::Curve {
            curve,
            span,
            side,
            parameter,
        } => FaceAnchor::Curve {
            curve: CurveId(curve),
            span: CurveSpanId(span),
            side: decode_curve_side(side),
            parameter,
        },
    }
}

fn encode_outer_side(side: OuterSide) -> StoredOuterSide {
    match side {
        OuterSide::Bottom => StoredOuterSide::Bottom,
        OuterSide::Right => StoredOuterSide::Right,
        OuterSide::Top => StoredOuterSide::Top,
        OuterSide::Left => StoredOuterSide::Left,
    }
}

fn decode_outer_side(side: StoredOuterSide) -> OuterSide {
    match side {
        StoredOuterSide::Bottom => OuterSide::Bottom,
        StoredOuterSide::Right => OuterSide::Right,
        StoredOuterSide::Top => OuterSide::Top,
        StoredOuterSide::Left => OuterSide::Left,
    }
}

fn encode_curve_side(side: CurveTraceSide) -> StoredCurveSide {
    match side {
        CurveTraceSide::Left => StoredCurveSide::Left,
        CurveTraceSide::Right => StoredCurveSide::Right,
    }
}

fn decode_curve_side(side: StoredCurveSide) -> CurveTraceSide {
    match side {
        StoredCurveSide::Left => CurveTraceSide::Left,
        StoredCurveSide::Right => CurveTraceSide::Right,
    }
}

fn encode_points(points: &[Point2]) -> Vec<[f64; 2]> {
    points.iter().map(|point| [point.x, point.y]).collect()
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPresentation {
    grid: bool,
    control_polygons: bool,
    handles: bool,
    accepted_reference: bool,
    boundary_conditions: bool,
    mesh: bool,
    mesh_boundaries: bool,
    /// Retired: the adaptation target is a `material_overlay` choice now. Still
    /// read, so an older scene that had it on is migrated, and still written as
    /// `false`, because a build from before the move requires the key.
    #[serde(default)]
    adaptation_target: bool,
    point_probes: bool,
    line_probes: bool,
    boundary_probes: bool,
    area_probes: bool,
    far_field_contour: bool,
    #[serde(default = "default_probe_labels")]
    probe_labels: bool,
    field: bool,
    #[serde(default = "default_simulation_speed")]
    simulation_speed: f64,
    field_gain: f32,
    #[serde(default = "default_field_auto_exposure")]
    field_auto_exposure: bool,
    #[serde(default)]
    vector_overlay: StoredVectorOverlay,
    /// Retired component-wise temporal smoothing. Kept in version-22 JSON so
    /// an older build sees the replacement as explicitly disabled.
    #[serde(default = "default_vector_overlay_smoothed")]
    vector_overlay_smoothed: bool,
    #[serde(default)]
    vector_overlay_ac_coupled: Option<bool>,
    #[serde(default = "default_vector_overlay_density")]
    vector_overlay_density: f32,
    #[serde(default = "default_vector_overlay_gain")]
    vector_overlay_gain: f32,
    material_overlay: StoredMaterialOverlay,
    material_overlay_opacity: f32,
    material_overlay_auto_range: bool,
    material_overlay_logarithmic: bool,
    material_overlay_manual_min: f64,
    material_overlay_manual_max: f64,
    /// The solver settings. Every one defaults, so a document written before
    /// they were kept loads with the values the application used to start from
    /// rather than being refused.
    #[serde(default = "default_mesh_edge")]
    mesh_edge: f64,
    #[serde(default = "default_adaptation_enabled")]
    adaptation_enabled: bool,
    #[serde(default = "default_adaptation_accuracy_percent")]
    adaptation_accuracy_percent: f64,
    #[serde(default = "default_adaptation_elements_per_wavelength")]
    adaptation_elements_per_wavelength: f64,
    #[serde(default = "default_adaptation_minimum_edge")]
    adaptation_minimum_edge: f64,
    #[serde(default = "default_adaptation_maximum_edge")]
    adaptation_maximum_edge: f64,
    #[serde(default = "default_grid_scale_filter")]
    grid_scale_filter: bool,
    #[serde(default)]
    advanced_materials: bool,
}

fn default_mesh_edge() -> f64 {
    PresentationSettings::default().mesh_edge
}
fn default_adaptation_enabled() -> bool {
    AdaptationSettings::default().enabled
}
fn default_adaptation_accuracy_percent() -> f64 {
    AdaptationSettings::default().accuracy_percent
}
fn default_adaptation_elements_per_wavelength() -> f64 {
    AdaptationSettings::default().elements_per_wavelength
}
fn default_adaptation_minimum_edge() -> f64 {
    AdaptationSettings::default().minimum_edge
}
fn default_adaptation_maximum_edge() -> f64 {
    AdaptationSettings::default().maximum_edge
}
fn default_grid_scale_filter() -> bool {
    PresentationSettings::default().grid_scale_filter
}

const fn default_probe_labels() -> bool {
    true
}
const fn default_field_auto_exposure() -> bool {
    true
}
const fn default_simulation_speed() -> f64 {
    1.0
}
const fn default_vector_overlay_smoothed() -> bool {
    true
}
const fn default_vector_overlay_density() -> f32 {
    54.0
}
const fn default_vector_overlay_gain() -> f32 {
    1.0
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum StoredVectorOverlay {
    #[default]
    Off,
    #[serde(rename = "complementary_field_rate", alias = "complementary_field")]
    ComplementaryField,
    RelativeEnergyFlow,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", content = "property", rename_all = "snake_case")]
enum StoredMaterialOverlay {
    Off,
    Regions,
    Subdomains,
    AdaptationTarget,
    Property(StoredMaterialProperty),
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredMaterialProperty {
    Density,
    Stiffness,
    Damping,
    WaveSpeed,
    Impedance,
    Anisotropy,
    VolumeSource,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredProbeSamplingPreset {
    Low,
    Medium,
    High,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredPhysicsModel {
    Mechanical,
    Electromagnetic { polarization: StoredPolarization },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredPolarization {
    Tm,
    Te,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredScalarField {
    Constant { value: f64 },
    Formula { source: String },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredCoefficientLaw {
    field: StoredFieldLaw,
    drive: StoredTimeDrive,
    #[serde(default)]
    alternate: Option<StoredScalarField>,
    #[serde(default)]
    inverted: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredFieldLaw {
    Linear,
    Polynomial {
        chi1: StoredScalarField,
        chi2: StoredScalarField,
        #[serde(default)]
        amplitude_bound: Option<StoredScalarField>,
    },
    Saturable {
        chi: StoredScalarField,
        saturation: StoredScalarField,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredTimeDrive {
    None,
    ParametricPump {
        depth: StoredScalarField,
        frequency_hz: StoredScalarField,
        phase_radians: StoredScalarField,
    },
    TimeCrystal {
        depth: StoredScalarField,
        frequency_hz: StoredScalarField,
        phase_radians: StoredScalarField,
        sharpness: StoredScalarField,
    },
    TravellingModulation {
        depth: StoredScalarField,
        frequency_hz: StoredScalarField,
        phase_radians: StoredScalarField,
        wavenumber: StoredScalarField,
        angle_radians: StoredScalarField,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDampingLaw {
    rate: StoredRateLaw,
    drive: StoredTimeDrive,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredLossChannel {
    base_rate: StoredScalarField,
    law: StoredDampingLaw,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredRateLaw {
    Constant,
    SaturableAbsorption {
        saturation: StoredScalarField,
    },
    Polynomial {
        beta1: StoredScalarField,
        beta2: StoredScalarField,
        #[serde(default)]
        amplitude_bound: Option<StoredScalarField>,
    },
    VanDerPol {
        threshold: StoredScalarField,
        amplitude_bound: StoredScalarField,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredRestoringLaw {
    None,
    KleinGordon {
        omega0: StoredScalarField,
    },
    SineGordon {
        omega0: StoredScalarField,
    },
    Phi4 {
        lambda: StoredScalarField,
        amplitude_bound: StoredScalarField,
    },
}

fn stored_linear_coefficient_law() -> StoredCoefficientLaw {
    StoredCoefficientLaw {
        field: StoredFieldLaw::Linear,
        drive: StoredTimeDrive::None,
        alternate: None,
        inverted: false,
    }
}

fn stored_no_restoring_law() -> StoredRestoringLaw {
    StoredRestoringLaw::None
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredFaceCondition {
    Reflecting,
    Impedance { ratio: f64 },
    SecondOrderOutgoing,
    ElectricWall,
    MagneticWall,
    Neumann { signal: StoredTimeSignal },
    Dirichlet { signal: StoredTimeSignal },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredOuterBoundaryCondition {
    Reflecting,
    FirstOrderOutgoing,
    SecondOrderOutgoing,
    ElectricWall,
    MagneticWall,
    Neumann { signal: StoredTimeSignal },
    Dirichlet { signal: StoredTimeSignal },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredTimeSignal {
    Harmonic {
        offset: f64,
        amplitude: f64,
        frequency_hz: f64,
        phase_radians: f64,
    },
}

fn encode_physics(physics: PhysicsModel) -> StoredPhysicsModel {
    match physics {
        PhysicsModel::Mechanical => StoredPhysicsModel::Mechanical,
        PhysicsModel::Electromagnetic { polarization } => StoredPhysicsModel::Electromagnetic {
            polarization: match polarization {
                ElectromagneticPolarization::Tm => StoredPolarization::Tm,
                ElectromagneticPolarization::Te => StoredPolarization::Te,
            },
        },
    }
}

fn decode_physics(physics: StoredPhysicsModel) -> PhysicsModel {
    match physics {
        StoredPhysicsModel::Mechanical => PhysicsModel::Mechanical,
        StoredPhysicsModel::Electromagnetic { polarization } => PhysicsModel::Electromagnetic {
            polarization: match polarization {
                StoredPolarization::Tm => ElectromagneticPolarization::Tm,
                StoredPolarization::Te => ElectromagneticPolarization::Te,
            },
        },
    }
}

fn encode_scalar_field(field: &ScalarField) -> StoredScalarField {
    match field {
        ScalarField::Constant(value) => StoredScalarField::Constant { value: *value },
        ScalarField::Formula(formula) => StoredScalarField::Formula {
            source: formula.source().into(),
        },
    }
}

fn decode_scalar_field(field: StoredScalarField) -> Result<ScalarField, String> {
    match field {
        StoredScalarField::Constant { value } => Ok(ScalarField::constant(value)),
        StoredScalarField::Formula { source } => {
            ScalarField::formula(source).map_err(|error| error.to_string())
        }
    }
}

fn encode_coefficient_law(law: &CoefficientLaw) -> StoredCoefficientLaw {
    StoredCoefficientLaw {
        field: match &law.field {
            FieldLaw::Linear => StoredFieldLaw::Linear,
            FieldLaw::Polynomial {
                chi1,
                chi2,
                amplitude_bound,
            } => StoredFieldLaw::Polynomial {
                chi1: encode_scalar_field(chi1),
                chi2: encode_scalar_field(chi2),
                amplitude_bound: amplitude_bound.as_ref().map(encode_scalar_field),
            },
            FieldLaw::Saturable { chi, saturation } => StoredFieldLaw::Saturable {
                chi: encode_scalar_field(chi),
                saturation: encode_scalar_field(saturation),
            },
        },
        drive: encode_time_drive(&law.drive),
        alternate: law.alternate.as_ref().map(encode_scalar_field),
        inverted: law.inverted,
    }
}

fn decode_coefficient_law(law: StoredCoefficientLaw) -> Result<CoefficientLaw, String> {
    Ok(CoefficientLaw {
        field: match law.field {
            StoredFieldLaw::Linear => FieldLaw::Linear,
            StoredFieldLaw::Polynomial {
                chi1,
                chi2,
                amplitude_bound,
            } => FieldLaw::Polynomial {
                chi1: decode_scalar_field(chi1)?,
                chi2: decode_scalar_field(chi2)?,
                amplitude_bound: amplitude_bound.map(decode_scalar_field).transpose()?,
            },
            StoredFieldLaw::Saturable { chi, saturation } => FieldLaw::Saturable {
                chi: decode_scalar_field(chi)?,
                saturation: decode_scalar_field(saturation)?,
            },
        },
        drive: decode_time_drive(law.drive)?,
        alternate: law.alternate.map(decode_scalar_field).transpose()?,
        inverted: law.inverted,
    }
    .normalized())
}

fn encode_time_drive(drive: &TimeDrive) -> StoredTimeDrive {
    match drive {
        TimeDrive::None => StoredTimeDrive::None,
        TimeDrive::ParametricPump {
            depth,
            frequency_hz,
            phase_radians,
        } => StoredTimeDrive::ParametricPump {
            depth: encode_scalar_field(depth),
            frequency_hz: encode_scalar_field(frequency_hz),
            phase_radians: encode_scalar_field(phase_radians),
        },
        TimeDrive::TimeCrystal {
            depth,
            frequency_hz,
            phase_radians,
            sharpness,
        } => StoredTimeDrive::TimeCrystal {
            depth: encode_scalar_field(depth),
            frequency_hz: encode_scalar_field(frequency_hz),
            phase_radians: encode_scalar_field(phase_radians),
            sharpness: encode_scalar_field(sharpness),
        },
        TimeDrive::TravellingModulation {
            depth,
            frequency_hz,
            phase_radians,
            wavenumber,
            angle_radians,
        } => StoredTimeDrive::TravellingModulation {
            depth: encode_scalar_field(depth),
            frequency_hz: encode_scalar_field(frequency_hz),
            phase_radians: encode_scalar_field(phase_radians),
            wavenumber: encode_scalar_field(wavenumber),
            angle_radians: encode_scalar_field(angle_radians),
        },
    }
}

fn decode_time_drive(drive: StoredTimeDrive) -> Result<TimeDrive, String> {
    Ok(match drive {
        StoredTimeDrive::None => TimeDrive::None,
        StoredTimeDrive::ParametricPump {
            depth,
            frequency_hz,
            phase_radians,
        } => TimeDrive::ParametricPump {
            depth: decode_scalar_field(depth)?,
            frequency_hz: decode_scalar_field(frequency_hz)?,
            phase_radians: decode_scalar_field(phase_radians)?,
        },
        StoredTimeDrive::TimeCrystal {
            depth,
            frequency_hz,
            phase_radians,
            sharpness,
        } => TimeDrive::TimeCrystal {
            depth: decode_scalar_field(depth)?,
            frequency_hz: decode_scalar_field(frequency_hz)?,
            phase_radians: decode_scalar_field(phase_radians)?,
            sharpness: decode_scalar_field(sharpness)?,
        },
        StoredTimeDrive::TravellingModulation {
            depth,
            frequency_hz,
            phase_radians,
            wavenumber,
            angle_radians,
        } => TimeDrive::TravellingModulation {
            depth: decode_scalar_field(depth)?,
            frequency_hz: decode_scalar_field(frequency_hz)?,
            phase_radians: decode_scalar_field(phase_radians)?,
            wavenumber: decode_scalar_field(wavenumber)?,
            angle_radians: decode_scalar_field(angle_radians)?,
        },
    })
}

fn encode_damping_law(law: &DampingLaw) -> StoredDampingLaw {
    StoredDampingLaw {
        rate: match &law.rate {
            RateLaw::Constant => StoredRateLaw::Constant,
            RateLaw::SaturableAbsorption { saturation } => StoredRateLaw::SaturableAbsorption {
                saturation: encode_scalar_field(saturation),
            },
            RateLaw::Polynomial {
                beta1,
                beta2,
                amplitude_bound,
            } => StoredRateLaw::Polynomial {
                beta1: encode_scalar_field(beta1),
                beta2: encode_scalar_field(beta2),
                amplitude_bound: amplitude_bound.as_ref().map(encode_scalar_field),
            },
            RateLaw::VanDerPol {
                threshold,
                amplitude_bound,
            } => StoredRateLaw::VanDerPol {
                threshold: encode_scalar_field(threshold),
                amplitude_bound: encode_scalar_field(amplitude_bound),
            },
        },
        drive: encode_time_drive(&law.drive),
    }
}

fn encode_loss_channel(loss: &LossChannel) -> StoredLossChannel {
    StoredLossChannel {
        base_rate: encode_scalar_field(&loss.base_rate),
        law: encode_damping_law(&loss.law),
    }
}

fn decode_damping_law(law: StoredDampingLaw) -> Result<DampingLaw, String> {
    Ok(DampingLaw {
        rate: match law.rate {
            StoredRateLaw::Constant => RateLaw::Constant,
            StoredRateLaw::SaturableAbsorption { saturation } => RateLaw::SaturableAbsorption {
                saturation: decode_scalar_field(saturation)?,
            },
            StoredRateLaw::Polynomial {
                beta1,
                beta2,
                amplitude_bound,
            } => RateLaw::Polynomial {
                beta1: decode_scalar_field(beta1)?,
                beta2: decode_scalar_field(beta2)?,
                amplitude_bound: amplitude_bound.map(decode_scalar_field).transpose()?,
            },
            StoredRateLaw::VanDerPol {
                threshold,
                amplitude_bound,
            } => RateLaw::VanDerPol {
                threshold: decode_scalar_field(threshold)?,
                amplitude_bound: decode_scalar_field(amplitude_bound)?,
            },
        },
        drive: decode_time_drive(law.drive)?,
    })
}

fn decode_loss_channel(loss: StoredLossChannel) -> Result<LossChannel, String> {
    Ok(LossChannel {
        base_rate: decode_scalar_field(loss.base_rate)?,
        law: decode_damping_law(loss.law)?,
    })
}

fn encode_restoring_law(law: &RestoringLaw) -> StoredRestoringLaw {
    match law {
        RestoringLaw::None => StoredRestoringLaw::None,
        RestoringLaw::KleinGordon { omega0 } => StoredRestoringLaw::KleinGordon {
            omega0: encode_scalar_field(omega0),
        },
        RestoringLaw::SineGordon { omega0 } => StoredRestoringLaw::SineGordon {
            omega0: encode_scalar_field(omega0),
        },
        RestoringLaw::Phi4 {
            lambda,
            amplitude_bound,
        } => StoredRestoringLaw::Phi4 {
            lambda: encode_scalar_field(lambda),
            amplitude_bound: encode_scalar_field(amplitude_bound),
        },
    }
}

fn decode_restoring_law(law: StoredRestoringLaw) -> Result<RestoringLaw, String> {
    Ok(match law {
        StoredRestoringLaw::None => RestoringLaw::None,
        StoredRestoringLaw::KleinGordon { omega0 } => RestoringLaw::KleinGordon {
            omega0: decode_scalar_field(omega0)?,
        },
        StoredRestoringLaw::SineGordon { omega0 } => RestoringLaw::SineGordon {
            omega0: decode_scalar_field(omega0)?,
        },
        StoredRestoringLaw::Phi4 {
            lambda,
            amplitude_bound,
        } => RestoringLaw::Phi4 {
            lambda: decode_scalar_field(lambda)?,
            amplitude_bound: decode_scalar_field(amplitude_bound)?,
        },
    })
}

fn encode_signal(signal: TimeSignal) -> StoredTimeSignal {
    let [offset, amplitude, frequency_hz, phase_radians] = signal.harmonic_parameters();
    StoredTimeSignal::Harmonic {
        offset,
        amplitude,
        frequency_hz,
        phase_radians,
    }
}

fn decode_signal(signal: StoredTimeSignal) -> TimeSignal {
    let StoredTimeSignal::Harmonic {
        offset,
        amplitude,
        frequency_hz,
        phase_radians,
    } = signal;
    TimeSignal::Harmonic {
        offset,
        amplitude,
        frequency_hz,
        phase_radians,
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
        OuterBoundaryCondition::ElectricWall => StoredOuterBoundaryCondition::ElectricWall,
        OuterBoundaryCondition::MagneticWall => StoredOuterBoundaryCondition::MagneticWall,
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
        StoredOuterBoundaryCondition::ElectricWall => OuterBoundaryCondition::ElectricWall,
        StoredOuterBoundaryCondition::MagneticWall => OuterBoundaryCondition::MagneticWall,
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
        FaceBoundaryCondition::SecondOrderOutgoing => StoredFaceCondition::SecondOrderOutgoing,
        FaceBoundaryCondition::ElectricWall => StoredFaceCondition::ElectricWall,
        FaceBoundaryCondition::MagneticWall => StoredFaceCondition::MagneticWall,
        FaceBoundaryCondition::Neumann { signal } => StoredFaceCondition::Neumann {
            signal: encode_signal(signal),
        },
        FaceBoundaryCondition::Dirichlet { signal } => StoredFaceCondition::Dirichlet {
            signal: encode_signal(signal),
        },
    }
}

fn decode_face_condition(condition: StoredFaceCondition) -> FaceBoundaryCondition {
    match condition {
        StoredFaceCondition::Reflecting => FaceBoundaryCondition::Reflecting,
        StoredFaceCondition::Impedance { ratio } => FaceBoundaryCondition::Impedance { ratio },
        StoredFaceCondition::SecondOrderOutgoing => FaceBoundaryCondition::SecondOrderOutgoing,
        StoredFaceCondition::ElectricWall => FaceBoundaryCondition::ElectricWall,
        StoredFaceCondition::MagneticWall => FaceBoundaryCondition::MagneticWall,
        StoredFaceCondition::Neumann { signal } => FaceBoundaryCondition::Neumann {
            signal: decode_signal(signal),
        },
        StoredFaceCondition::Dirichlet { signal } => FaceBoundaryCondition::Dirichlet {
            signal: decode_signal(signal),
        },
    }
}

fn decode_spline(
    controls: Vec<[f64; 2]>,
    intervals: Vec<f64>,
    multiplicities: Vec<u8>,
) -> Result<PeriodicCubicSpline, String> {
    if controls.iter().flatten().any(|value| !value.is_finite()) {
        return Err("Coordinates must be finite".into());
    }
    let controls = controls
        .into_iter()
        .map(|[x, y]| Point2::new(x, y))
        .collect();
    if multiplicities.is_empty() {
        PeriodicCubicSpline::new(controls, intervals)
    } else {
        PeriodicCubicSpline::new_with_multiplicities(controls, intervals, multiplicities)
    }
    .map_err(|error| error.to_string())
}

fn decode_open_spline(
    controls: Vec<[f64; 2]>,
    intervals: Vec<f64>,
    multiplicities: Vec<u8>,
) -> Result<OpenCubicSpline, String> {
    if controls.iter().flatten().any(|value| !value.is_finite()) {
        return Err("Coordinates must be finite".into());
    }
    let controls = controls
        .into_iter()
        .map(|[x, y]| Point2::new(x, y))
        .collect();
    if multiplicities.is_empty() {
        OpenCubicSpline::new(controls, intervals)
    } else {
        OpenCubicSpline::new_with_multiplicities(controls, intervals, multiplicities)
    }
    .map_err(|error| error.to_string())
}

fn encode_presentation(settings: PresentationSettings) -> StoredPresentation {
    StoredPresentation {
        grid: settings.grid,
        control_polygons: settings.control_polygons,
        handles: settings.handles,
        accepted_reference: settings.accepted_reference,
        boundary_conditions: settings.boundary_conditions,
        mesh: settings.mesh,
        mesh_boundaries: settings.mesh_boundaries,
        adaptation_target: false,
        point_probes: settings.point_probes,
        line_probes: settings.line_probes,
        boundary_probes: settings.boundary_probes,
        area_probes: settings.area_probes,
        far_field_contour: settings.far_field_contour,
        probe_labels: settings.probe_labels,
        field: settings.field,
        simulation_speed: settings.simulation_speed,
        field_gain: settings.field_gain,
        field_auto_exposure: settings.field_auto_exposure,
        mesh_edge: settings.mesh_edge,
        adaptation_enabled: settings.adaptation.enabled,
        adaptation_accuracy_percent: settings.adaptation.accuracy_percent,
        adaptation_elements_per_wavelength: settings.adaptation.elements_per_wavelength,
        adaptation_minimum_edge: settings.adaptation.minimum_edge,
        adaptation_maximum_edge: settings.adaptation.maximum_edge,
        grid_scale_filter: settings.grid_scale_filter,
        advanced_materials: settings.advanced_materials,
        vector_overlay: match settings.vector_overlay {
            VectorOverlay::Off => StoredVectorOverlay::Off,
            VectorOverlay::ComplementaryField => StoredVectorOverlay::ComplementaryField,
            VectorOverlay::RelativeEnergyFlow => StoredVectorOverlay::RelativeEnergyFlow,
        },
        vector_overlay_smoothed: false,
        vector_overlay_ac_coupled: Some(settings.vector_overlay_ac_coupled),
        vector_overlay_density: settings.vector_overlay_density,
        vector_overlay_gain: settings.vector_overlay_gain,
        material_overlay: match settings.material_overlay {
            MaterialOverlay::Off => StoredMaterialOverlay::Off,
            MaterialOverlay::Regions => StoredMaterialOverlay::Regions,
            MaterialOverlay::Subdomains => StoredMaterialOverlay::Subdomains,
            MaterialOverlay::AdaptationTarget => StoredMaterialOverlay::AdaptationTarget,
            MaterialOverlay::Property(property) => {
                StoredMaterialOverlay::Property(match property {
                    MaterialProperty::Density => StoredMaterialProperty::Density,
                    MaterialProperty::Stiffness => StoredMaterialProperty::Stiffness,
                    MaterialProperty::Damping => StoredMaterialProperty::Damping,
                    MaterialProperty::WaveSpeed => StoredMaterialProperty::WaveSpeed,
                    MaterialProperty::Impedance => StoredMaterialProperty::Impedance,
                    MaterialProperty::Anisotropy => StoredMaterialProperty::Anisotropy,
                    MaterialProperty::VolumeSource => StoredMaterialProperty::VolumeSource,
                })
            }
        },
        material_overlay_opacity: settings.material_overlay_opacity,
        material_overlay_auto_range: settings.material_overlay_auto_range,
        material_overlay_logarithmic: settings.material_overlay_logarithmic,
        material_overlay_manual_min: settings.material_overlay_manual_min,
        material_overlay_manual_max: settings.material_overlay_manual_max,
    }
}

fn decode_presentation(stored: StoredPresentation) -> Result<PresentationSettings, String> {
    let settings = PresentationSettings {
        grid: stored.grid,
        control_polygons: stored.control_polygons,
        handles: stored.handles,
        accepted_reference: stored.accepted_reference,
        boundary_conditions: stored.boundary_conditions,
        mesh: stored.mesh,
        mesh_boundaries: stored.mesh_boundaries,
        point_probes: stored.point_probes,
        line_probes: stored.line_probes,
        boundary_probes: stored.boundary_probes,
        area_probes: stored.area_probes,
        far_field_contour: stored.far_field_contour,
        probe_labels: stored.probe_labels,
        field: stored.field,
        simulation_speed: stored.simulation_speed,
        field_gain: stored.field_gain,
        field_auto_exposure: stored.field_auto_exposure,
        mesh_edge: stored.mesh_edge,
        adaptation: AdaptationSettings {
            enabled: stored.adaptation_enabled,
            accuracy_percent: stored.adaptation_accuracy_percent,
            elements_per_wavelength: stored.adaptation_elements_per_wavelength,
            minimum_edge: stored.adaptation_minimum_edge,
            maximum_edge: stored.adaptation_maximum_edge,
        },
        grid_scale_filter: stored.grid_scale_filter,
        advanced_materials: stored.advanced_materials,
        vector_overlay: match stored.vector_overlay {
            StoredVectorOverlay::Off => VectorOverlay::Off,
            StoredVectorOverlay::ComplementaryField => VectorOverlay::ComplementaryField,
            StoredVectorOverlay::RelativeEnergyFlow => VectorOverlay::RelativeEnergyFlow,
        },
        // Files from before AC coupling reuse the retired checkbox's value;
        // newly written files carry the separate setting explicitly.
        vector_overlay_ac_coupled: stored
            .vector_overlay_ac_coupled
            .unwrap_or(stored.vector_overlay_smoothed),
        vector_overlay_density: stored.vector_overlay_density,
        vector_overlay_gain: stored.vector_overlay_gain,
        // A scene from before the move carries the target as its own flag. It
        // becomes the overlay it now is, unless that slot already holds a
        // material overlay, which is the one that was actually visible.
        material_overlay: match stored.material_overlay {
            StoredMaterialOverlay::Off if stored.adaptation_target => {
                MaterialOverlay::AdaptationTarget
            }
            StoredMaterialOverlay::Off => MaterialOverlay::Off,
            StoredMaterialOverlay::Regions => MaterialOverlay::Regions,
            StoredMaterialOverlay::Subdomains => MaterialOverlay::Subdomains,
            StoredMaterialOverlay::AdaptationTarget => MaterialOverlay::AdaptationTarget,
            StoredMaterialOverlay::Property(property) => {
                MaterialOverlay::Property(match property {
                    StoredMaterialProperty::Density => MaterialProperty::Density,
                    StoredMaterialProperty::Stiffness => MaterialProperty::Stiffness,
                    StoredMaterialProperty::Damping => MaterialProperty::Damping,
                    StoredMaterialProperty::WaveSpeed => MaterialProperty::WaveSpeed,
                    StoredMaterialProperty::Impedance => MaterialProperty::Impedance,
                    StoredMaterialProperty::Anisotropy => MaterialProperty::Anisotropy,
                    StoredMaterialProperty::VolumeSource => MaterialProperty::VolumeSource,
                })
            }
        },
        material_overlay_opacity: stored.material_overlay_opacity,
        material_overlay_auto_range: stored.material_overlay_auto_range,
        material_overlay_logarithmic: stored.material_overlay_logarithmic,
        material_overlay_manual_min: stored.material_overlay_manual_min,
        material_overlay_manual_max: stored.material_overlay_manual_max,
    };
    if settings.valid() {
        Ok(settings)
    } else {
        Err("Scene contains invalid presentation settings".into())
    }
}

fn encode_probe_preset(preset: ProbeSamplingPreset) -> StoredProbeSamplingPreset {
    match preset {
        ProbeSamplingPreset::Low => StoredProbeSamplingPreset::Low,
        ProbeSamplingPreset::Medium => StoredProbeSamplingPreset::Medium,
        ProbeSamplingPreset::High => StoredProbeSamplingPreset::High,
    }
}

fn decode_probe_preset(preset: StoredProbeSamplingPreset) -> ProbeSamplingPreset {
    match preset {
        StoredProbeSamplingPreset::Low => ProbeSamplingPreset::Low,
        StoredProbeSamplingPreset::Medium => ProbeSamplingPreset::Medium,
        StoredProbeSamplingPreset::High => ProbeSamplingPreset::High,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topology_editor::{
        OpenCurvePurpose, TopologyAcceptance, TopologyAttachment, TopologyEditor,
    };

    fn settle(editor: &mut TopologyEditor) {
        for _ in 0..100_000 {
            editor.validate_frame(64);
            if editor.acceptance != TopologyAcceptance::Pending {
                return;
            }
        }
        panic!("topology validation did not finish");
    }

    fn outer(side: OuterSide, fraction: f64) -> TopologyAttachment {
        TopologyAttachment::Boundary(FaceAnchor::Outer { side, fraction })
    }

    fn divider_document() -> (TopologyEditor, CurveId) {
        let mut editor = TopologyEditor::default();
        let curve = editor
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
            .unwrap()
            .curve;
        settle(&mut editor);
        (editor, curve)
    }

    #[test]
    fn version_22_default_document_round_trips_exactly() {
        let document = TopologyDocument::default();
        let pretty = save(&document).unwrap();
        assert!(pretty.contains("\"version\": 22"));
        assert_eq!(parse_document(pretty.as_bytes()).unwrap(), document);
        let compact = save_compact(&document).unwrap();
        assert_eq!(parse_document(&compact).unwrap(), document);
    }

    #[test]
    fn authored_material_laws_round_trip_without_enabling_solver_behavior() {
        let mut document = TopologyDocument::default();
        for scene in [&mut document.model.draft, &mut document.model.accepted] {
            let material = &mut scene.materials[0];
            material.mass_law = CoefficientLaw {
                field: FieldLaw::Polynomial {
                    chi1: ScalarField::constant(0.1),
                    chi2: ScalarField::constant(0.2),
                    amplitude_bound: Some(ScalarField::constant(0.5)),
                },
                drive: TimeDrive::TravellingModulation {
                    depth: ScalarField::constant(0.2),
                    frequency_hz: ScalarField::constant(3.0),
                    phase_radians: ScalarField::constant(0.4),
                    wavenumber: ScalarField::constant(2.0),
                    angle_radians: ScalarField::constant(0.3),
                },
                alternate: Some(ScalarField::constant(1.4)),
                inverted: true,
            };
            material.stiffness_law = CoefficientLaw {
                field: FieldLaw::Saturable {
                    chi: ScalarField::constant(0.25),
                    saturation: ScalarField::constant(1.5),
                },
                drive: TimeDrive::TimeCrystal {
                    depth: ScalarField::constant(0.1),
                    frequency_hz: ScalarField::constant(2.0),
                    phase_radians: ScalarField::constant(0.2),
                    sharpness: ScalarField::constant(4.0),
                },
                alternate: None,
                inverted: false,
            };
            material.electric_loss = Some(LossChannel {
                base_rate: ScalarField::formula("0.02 + 0*x").unwrap(),
                law: DampingLaw {
                    rate: RateLaw::Polynomial {
                        beta1: ScalarField::constant(0.1),
                        beta2: ScalarField::constant(0.2),
                        amplitude_bound: None,
                    },
                    drive: TimeDrive::ParametricPump {
                        depth: ScalarField::constant(0.1),
                        frequency_hz: ScalarField::constant(1.0),
                        phase_radians: ScalarField::constant(0.0),
                    },
                },
            });
            material.magnetic_loss = Some(LossChannel {
                base_rate: ScalarField::constant(0.03),
                law: DampingLaw {
                    rate: RateLaw::SaturableAbsorption {
                        saturation: ScalarField::constant(1.2),
                    },
                    drive: TimeDrive::None,
                },
            });
            material.restoring = RestoringLaw::Phi4 {
                lambda: ScalarField::constant(0.5),
                amplitude_bound: ScalarField::constant(2.0),
            };
            material.switch_ramp = 0.25;
        }

        let encoded = save(&document).unwrap();
        let decoded = parse_document(encoded.as_bytes()).unwrap();
        assert_eq!(decoded, document);
        assert_eq!(
            decoded.model.accepted.materials[0].evaluate(MaterialFrame::world(), Point2::default()),
            Err(MaterialError::UnsupportedMaterialLaw)
        );
    }

    #[test]
    fn earlier_version_22_materials_receive_inert_law_defaults() {
        let document = TopologyDocument::default();
        let mut value: serde_json::Value = serde_json::from_str(&save(&document).unwrap()).unwrap();
        for scene_name in ["draft", "accepted"] {
            let materials = value["model"][scene_name]["materials"]
                .as_array_mut()
                .unwrap();
            for material in materials {
                let material = material.as_object_mut().unwrap();
                for key in [
                    "mass_law",
                    "stiffness_law",
                    "electric_loss",
                    "magnetic_loss",
                    "restoring",
                    "switch_ramp",
                ] {
                    assert!(material.remove(key).is_some(), "{key} was not written");
                }
            }
        }
        let decoded = parse_document(serde_json::to_string(&value).unwrap().as_bytes()).unwrap();
        assert_eq!(decoded, document);
    }

    #[test]
    fn attached_topology_materials_probes_and_presentation_round_trip() {
        let (mut editor, curve_id) = divider_document();
        let span = editor
            .document
            .model
            .draft
            .geometry
            .curves
            .iter()
            .find(|curve| curve.id == curve_id)
            .unwrap()
            .spans[0]
            .id;
        for scene in [
            &mut editor.document.model.draft,
            &mut editor.document.model.accepted,
        ] {
            scene.materials[0].mass_density = ScalarField::formula("1 + a*x*x").unwrap();
            scene.materials[0].parameters = vec![MaterialParameter {
                name: "a".into(),
                value: 0.25,
            }];
            scene.volume_sources.push(VolumeSource {
                region: BACKGROUND_REGION,
                enabled: true,
                profile: ScalarField::formula("exp(-r*r/w)").unwrap(),
                parameters: vec![MaterialParameter {
                    name: "w".into(),
                    value: 0.2,
                }],
                signal: TimeSignal::harmonic(0.1, 2.0, 1.5, 0.3),
            });
            scene.outer_boundaries.sides[0] = OuterBoundaryCondition::Neumann {
                signal: TimeSignal::harmonic(0.0, 1.0, 2.0, 0.0),
            };
        }
        editor.document.model.probes = vec![
            TopologyProbeDefinition {
                id: ProbeId(1),
                name: "point".into(),
                color: [91, 220, 194],
                enabled: true,
                target: TopologyProbeTarget::Point(Point2::new(-0.5, 0.0)),
            },
            TopologyProbeDefinition {
                id: ProbeId(2),
                name: "line".into(),
                color: [248, 196, 112],
                enabled: false,
                target: TopologyProbeTarget::Segment {
                    start: Point2::new(-0.8, -0.2),
                    end: Point2::new(-0.2, 0.3),
                    preset: crate::document::ProbeSamplingPreset::Low,
                },
            },
            TopologyProbeDefinition {
                id: ProbeId(3),
                name: "divider".into(),
                color: [72, 166, 255],
                enabled: true,
                target: TopologyProbeTarget::Boundary(TopologyBoundaryProbeTarget {
                    curve: curve_id,
                    spans: vec![span],
                    side: CurveTraceSide::Left,
                    reversed: true,
                    preset: crate::document::ProbeSamplingPreset::Medium,
                }),
            },
            TopologyProbeDefinition {
                id: ProbeId(4),
                name: "disk".into(),
                color: [255, 106, 123],
                enabled: true,
                target: TopologyProbeTarget::AreaDisk {
                    center: Point2::new(0.4, 0.0),
                    radius: 0.15,
                },
            },
            TopologyProbeDefinition {
                id: ProbeId(5),
                name: "region".into(),
                color: [180, 140, 255],
                enabled: true,
                target: TopologyProbeTarget::AreaRegion(BACKGROUND_REGION),
            },
        ];
        editor.document.model.source.signal = TimeSignal::harmonic(0.2, 4.0, 3.0, 0.4);
        editor.document.model.far_field.enabled = true;
        editor.document.model.far_field.inset = 0.18;
        editor.document.presentation.grid = false;
        editor.document.presentation.field_gain = 3.25;
        editor.document.presentation.simulation_speed = 0.35;
        editor.document.presentation.field_auto_exposure = false;

        let encoded = save(&editor.document).unwrap();
        assert_eq!(parse_document(encoded.as_bytes()).unwrap(), editor.document);
    }

    #[test]
    fn invalid_detached_separator_draft_round_trips_with_valid_accepted_scene() {
        let (mut editor, curve) = divider_document();
        editor.detach_endpoint(curve, 0).unwrap();
        settle(&mut editor);
        assert!(matches!(editor.acceptance, TopologyAcceptance::Invalid(_)));
        let encoded = save(&editor.document).unwrap();
        let decoded = parse_document(encoded.as_bytes()).unwrap();
        assert_eq!(decoded, editor.document);
        assert!(decoded.model.accepted.compile(1).is_ok());
        assert!(decoded.model.draft.compile(1).is_err());
    }

    #[test]
    fn obsolete_unknown_and_malformed_files_fail_without_legacy_fallback() {
        let document = TopologyDocument::default();
        let mut value: serde_json::Value = serde_json::from_str(&save(&document).unwrap()).unwrap();
        value["version"] = 21.into();
        let issue = parse_document(serde_json::to_string(&value).unwrap().as_bytes()).unwrap_err();
        assert!(issue.contains("requires version 22"));

        value["version"] = TOPOLOGY_FILE_VERSION.into();
        value["unexpected"] = true.into();
        assert!(parse_document(serde_json::to_string(&value).unwrap().as_bytes()).is_err());

        value.as_object_mut().unwrap().remove("unexpected");
        value["model"]["accepted"]["domain"] = serde_json::json!([1.0, -1.0, -1.0, 1.0]);
        assert!(parse_document(serde_json::to_string(&value).unwrap().as_bytes()).is_err());
    }

    #[test]
    fn every_physics_model_and_polarization_round_trips() {
        for physics in [
            PhysicsModel::Mechanical,
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            },
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            },
        ] {
            let mut document = TopologyDocument::default();
            document.model.draft.physics = physics;
            document.model.accepted.physics = physics;
            let encoded = save(&document).unwrap();
            assert_eq!(
                parse_document(encoded.as_bytes()).unwrap(),
                document,
                "{physics:?} did not survive the file"
            );
        }
    }

    /// Every boundary condition, on the outer walls and on a divider's spans,
    /// carries its own signal across the file. The rotation means each condition
    /// appears on every wall and on both sides of a span across the passes, so a
    /// swapped arm in either direction shows up.
    #[test]
    fn every_boundary_condition_round_trips_with_its_signal() {
        let signal = TimeSignal::harmonic(0.25, 1.5, 3.0, 0.75);
        let faces = [
            FaceBoundaryCondition::Reflecting,
            FaceBoundaryCondition::Impedance { ratio: 0.4 },
            FaceBoundaryCondition::SecondOrderOutgoing,
            FaceBoundaryCondition::ElectricWall,
            FaceBoundaryCondition::MagneticWall,
            FaceBoundaryCondition::Neumann { signal },
            FaceBoundaryCondition::Dirichlet { signal },
        ];
        let outers = [
            OuterBoundaryCondition::Reflecting,
            OuterBoundaryCondition::FirstOrderOutgoing,
            OuterBoundaryCondition::SecondOrderOutgoing,
            OuterBoundaryCondition::ElectricWall,
            OuterBoundaryCondition::MagneticWall,
            OuterBoundaryCondition::Neumann { signal },
            OuterBoundaryCondition::Dirichlet { signal },
        ];
        for pass in 0..faces.len() {
            let (mut editor, curve_id) = divider_document();
            let curve = editor
                .document
                .model
                .draft
                .geometry
                .curves
                .iter_mut()
                .find(|curve| curve.id == curve_id)
                .unwrap();
            curve.spans[0].behavior = SpanBehavior::Separated {
                left: faces[pass],
                right: faces[(pass + 1) % faces.len()],
                coupling: InternalBoundaryCoupling::Independent,
            };
            // A thin gap is only a law between two reflecting faces, so it
            // rides along on the other span rather than in the rotation.
            curve.spans[1].behavior = SpanBehavior::Separated {
                left: FaceBoundaryCondition::Reflecting,
                right: FaceBoundaryCondition::Reflecting,
                coupling: InternalBoundaryCoupling::ThinGap {
                    stiffness_ratio: 0.3,
                },
            };
            // Two walls meeting at a corner may not resolve to Dirichlet with
            // different signals, so a pass wears one condition all the way
            // round. The mixed set below pins which slot is which wall.
            for scene in [
                &mut editor.document.model.draft,
                &mut editor.document.model.accepted,
            ] {
                scene.outer_boundaries.sides = [outers[pass]; 4];
            }
            let encoded = save(&editor.document).unwrap();
            assert_eq!(
                parse_document(encoded.as_bytes()).unwrap(),
                editor.document,
                "pass {pass} did not survive the file"
            );
        }

        let (mut editor, _) = divider_document();
        let mixed = [
            OuterBoundaryCondition::Reflecting,
            OuterBoundaryCondition::FirstOrderOutgoing,
            OuterBoundaryCondition::SecondOrderOutgoing,
            OuterBoundaryCondition::Neumann { signal },
        ];
        for scene in [
            &mut editor.document.model.draft,
            &mut editor.document.model.accepted,
        ] {
            scene.outer_boundaries.sides = mixed;
        }
        let encoded = save(&editor.document).unwrap();
        assert_eq!(parse_document(encoded.as_bytes()).unwrap(), editor.document);
    }

    #[test]
    fn every_overlay_and_sampling_preset_round_trips() {
        let overlays = [
            MaterialOverlay::Off,
            MaterialOverlay::Regions,
            MaterialOverlay::Subdomains,
            MaterialOverlay::AdaptationTarget,
            MaterialOverlay::Property(MaterialProperty::Density),
            MaterialOverlay::Property(MaterialProperty::Stiffness),
            MaterialOverlay::Property(MaterialProperty::Damping),
            MaterialOverlay::Property(MaterialProperty::WaveSpeed),
            MaterialOverlay::Property(MaterialProperty::Impedance),
            MaterialOverlay::Property(MaterialProperty::Anisotropy),
            MaterialOverlay::Property(MaterialProperty::VolumeSource),
        ];
        let vectors = [
            VectorOverlay::Off,
            VectorOverlay::ComplementaryField,
            VectorOverlay::RelativeEnergyFlow,
        ];
        let presets = [
            ProbeSamplingPreset::Low,
            ProbeSamplingPreset::Medium,
            ProbeSamplingPreset::High,
        ];
        for (index, overlay) in overlays.into_iter().enumerate() {
            let mut document = TopologyDocument::default();
            let flag = index.is_multiple_of(2);
            document.presentation = PresentationSettings {
                grid: flag,
                control_polygons: !flag,
                handles: flag,
                accepted_reference: !flag,
                boundary_conditions: flag,
                mesh: !flag,
                mesh_boundaries: flag,
                point_probes: !flag,
                line_probes: flag,
                boundary_probes: !flag,
                area_probes: flag,
                far_field_contour: !flag,
                probe_labels: flag,
                field: !flag,
                field_gain: 3.25,
                simulation_speed: 0.35,
                field_auto_exposure: !flag,
                vector_overlay: vectors[index % vectors.len()],
                vector_overlay_ac_coupled: flag,
                vector_overlay_density: 71.5,
                vector_overlay_gain: 2.5,
                material_overlay: overlay,
                material_overlay_opacity: 0.75,
                material_overlay_auto_range: flag,
                material_overlay_logarithmic: !flag,
                material_overlay_manual_min: -2.5,
                material_overlay_manual_max: 4.5,
                mesh_edge: if flag { 0.07 } else { 0.11 },
                adaptation: AdaptationSettings {
                    enabled: !flag,
                    accuracy_percent: if flag { 0.5 } else { 2.0 },
                    elements_per_wavelength: if flag { 5.0 } else { 9.0 },
                    minimum_edge: if flag { 0.015 } else { 0.03 },
                    maximum_edge: if flag { 0.2 } else { 0.35 },
                },
                grid_scale_filter: flag,
                advanced_materials: flag,
            };
            document.model.probes = vec![TopologyProbeDefinition {
                id: ProbeId(1),
                name: "line".into(),
                color: [10, 20, 30],
                enabled: true,
                target: TopologyProbeTarget::Segment {
                    start: Point2::new(-0.4, -0.2),
                    end: Point2::new(0.4, 0.2),
                    preset: presets[index % presets.len()],
                },
            }];
            let encoded = save(&document).unwrap();
            assert_eq!(
                parse_document(encoded.as_bytes()).unwrap(),
                document,
                "{overlay:?} did not survive the file"
            );
        }
    }

    /// Several presentation keys arrived after version 22 froze and are written
    /// with a serde default, so a file from an earlier build of this version
    /// still loads. The retired adaptation-target flag is the one that migrates
    /// rather than defaults.
    #[test]
    fn a_version_22_file_from_an_earlier_build_takes_the_presentation_defaults() {
        let document = TopologyDocument::default();
        let mut value: serde_json::Value = serde_json::from_str(&save(&document).unwrap()).unwrap();
        let presentation = value["presentation"].as_object_mut().unwrap();
        for key in [
            "adaptation_target",
            "mesh_edge",
            "adaptation_enabled",
            "adaptation_accuracy_percent",
            "adaptation_elements_per_wavelength",
            "adaptation_minimum_edge",
            "adaptation_maximum_edge",
            "grid_scale_filter",
            "advanced_materials",
            "probe_labels",
            "field_auto_exposure",
            "simulation_speed",
            "vector_overlay",
            "vector_overlay_smoothed",
            "vector_overlay_ac_coupled",
            "vector_overlay_density",
            "vector_overlay_gain",
        ] {
            assert!(presentation.remove(key).is_some(), "{key} was not written");
        }
        let decoded = parse_document(serde_json::to_string(&value).unwrap().as_bytes()).unwrap();
        assert_eq!(decoded.presentation, PresentationSettings::default());

        value["presentation"]["adaptation_target"] = true.into();
        value["presentation"]["material_overlay"] = serde_json::json!({ "kind": "off" });
        let migrated = parse_document(serde_json::to_string(&value).unwrap().as_bytes()).unwrap();
        assert_eq!(
            migrated.presentation.material_overlay,
            MaterialOverlay::AdaptationTarget
        );
    }

    /// The settings that decide what the solver does, not what is drawn.
    ///
    /// Until these were kept, starting a session with adaptation off was not
    /// expressible: the control existed but reset to on at every launch, so a
    /// document could not be opened the way it was left.
    #[test]
    fn the_solver_settings_survive_a_round_trip() {
        let mut document = TopologyDocument::default();
        document.presentation.mesh_edge = 0.045;
        document.presentation.grid_scale_filter = false;
        document.presentation.adaptation = AdaptationSettings {
            enabled: false,
            accuracy_percent: 6.0,
            elements_per_wavelength: 9.0,
            minimum_edge: 0.011,
            maximum_edge: 0.29,
        };
        assert!(document.presentation.valid());
        let decoded = parse_document(save(&document).unwrap().as_bytes()).unwrap();
        assert_eq!(decoded.presentation, document.presentation);
        assert!(
            !decoded.presentation.adaptation.enabled,
            "a session left with adaptation off must open with it off"
        );
    }

    #[test]
    fn the_retired_smoothing_choice_migrates_to_arrow_ac_coupling() {
        for old_value in [false, true] {
            let mut value: serde_json::Value =
                serde_json::from_str(&save(&TopologyDocument::default()).unwrap()).unwrap();
            assert!(
                value["presentation"]
                    .as_object_mut()
                    .unwrap()
                    .remove("vector_overlay_ac_coupled")
                    .is_some()
            );
            value["presentation"]["vector_overlay_smoothed"] = old_value.into();
            let migrated =
                parse_document(serde_json::to_string(&value).unwrap().as_bytes()).unwrap();
            assert_eq!(migrated.presentation.vector_overlay_ac_coupled, old_value);
        }
    }

    /// The value codecs are the last guard before a bad number reaches the
    /// solver. Version 22 also never wrote a bare coefficient or an untagged
    /// signal, so neither is accepted back.
    #[test]
    fn malformed_values_are_rejected() {
        let document = TopologyDocument::default();
        let original: serde_json::Value = serde_json::from_str(&save(&document).unwrap()).unwrap();

        // Damping, because zero damping is otherwise a legal material: the
        // rejection has to come from the formula and not from a later guard.
        let mut value = original.clone();
        value["model"]["draft"]["materials"][0]["damping"] =
            serde_json::json!({ "kind": "formula", "source": "1 +" });
        let issue = parse_document(serde_json::to_string(&value).unwrap().as_bytes()).unwrap_err();
        assert!(issue.contains("unexpected token"), "{issue}");

        let mut value = original.clone();
        value["model"]["draft"]["materials"][0]["stiffness"] = serde_json::json!(1.5);
        assert!(parse_document(serde_json::to_string(&value).unwrap().as_bytes()).is_err());

        let mut value = original.clone();
        value["model"]["source"]["signal"] = serde_json::json!({
            "offset": 0.0,
            "amplitude": 1.0,
            "frequency_hz": 2.0,
            "phase_radians": 0.0,
        });
        assert!(parse_document(serde_json::to_string(&value).unwrap().as_bytes()).is_err());

        // JSON has no infinity, so the overflowing literal goes in as text and
        // is unquoted afterwards. It is the reader that turns it away, which is
        // why the codec's own guard is checked directly below.
        let (editor, _) = divider_document();
        let mut value: serde_json::Value =
            serde_json::from_str(&save(&editor.document).unwrap()).unwrap();
        value["model"]["draft"]["curves"][0]["spline"]["controls"][0][0] =
            serde_json::json!("overflow");
        let text = serde_json::to_string(&value)
            .unwrap()
            .replace("\"overflow\"", "1e400");
        assert!(parse_document(text.as_bytes()).is_err());

        for coordinate in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
            let controls = vec![[coordinate, 0.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]];
            assert_eq!(
                decode_open_spline(controls.clone(), vec![1.0], Vec::new()).unwrap_err(),
                "Coordinates must be finite"
            );
            let controls = [controls.clone(), controls].concat();
            assert_eq!(
                decode_spline(controls, vec![1.0; 8], Vec::new()).unwrap_err(),
                "Coordinates must be finite"
            );
        }
    }
}
