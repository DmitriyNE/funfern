//! Version-22 persistence for the unified topology document.
//!
//! This is deliberately a hard schema boundary. The live version-21 loader stays
//! in `persistence` until the atomic application cutover; this module accepts only
//! version 22 and has no legacy geometry adapter.

use crate::editor::{MAX_PROBES, MAX_SEGMENT_PROBE_POINTS, ProbeId};
use crate::persistence::{
    MAX_FILE_BYTES, StoredFaceCondition, StoredOuterBoundaryCondition, StoredPhysicsModel,
    StoredPresentation, StoredProbeSamplingPreset, StoredScalarField, StoredTimeSignal,
    decode_face_condition, decode_open_spline, decode_outer_condition, decode_physics,
    decode_presentation, decode_probe_preset, decode_scalar_field, decode_signal, decode_spline,
    encode_face_condition, encode_outer_condition, encode_physics, encode_presentation,
    encode_probe_preset, encode_scalar_field, encode_signal,
};
use crate::topology_editor::{
    TopologyBoundaryProbeTarget, TopologyDocument, TopologyDocumentModel, TopologyProbeDefinition,
    TopologyProbeTarget,
};
use funfern_core::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const TOPOLOGY_FILE_VERSION: u32 = 22;

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
            far_field: crate::editor::FarFieldSettings {
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
                    mass_density: decode_scalar_field(material.mass_density, true)?,
                    stiffness: decode_scalar_field(material.stiffness, true)?,
                    damping: decode_scalar_field(material.damping, true)?,
                    axis_ratio: decode_scalar_field(material.axis_ratio, true)?,
                    parameters: decode_parameters(material.parameters),
                    color: material.color,
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
                    profile: decode_scalar_field(source.profile, true)?,
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
                    preset: crate::editor::ProbeSamplingPreset::Low,
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
                    preset: crate::editor::ProbeSamplingPreset::Medium,
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
}
