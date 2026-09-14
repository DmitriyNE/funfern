use crate::editor::{
    BoundaryProbeFeature, BoundaryProbeSide, BoundaryProbeTarget, Document, DocumentModel,
    FarFieldSettings, MAX_PROBES, MAX_SEGMENT_PROBE_POINTS, MaterialOverlay, MaterialProperty,
    PresentationSettings, ProbeDefinition, ProbeId, ProbeSamplingPreset, ProbeTarget,
    VectorOverlay,
};
use funfern_core::*;
use serde::{Deserialize, Serialize};

pub const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    probes: Vec<StoredProbe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<StoredSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    far_field: Option<StoredFarField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    presentation: Option<StoredPresentation>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredPresentation {
    grid: bool,
    control_polygons: bool,
    handles: bool,
    accepted_reference: bool,
    boundary_conditions: bool,
    mesh: bool,
    mesh_boundaries: bool,
    adaptation_target: bool,
    point_probes: bool,
    line_probes: bool,
    boundary_probes: bool,
    area_probes: bool,
    far_field_contour: bool,
    field: bool,
    field_gain: f32,
    #[serde(default)]
    vector_overlay: StoredVectorOverlay,
    #[serde(default = "default_vector_overlay_smoothed")]
    vector_overlay_smoothed: bool,
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
#[serde(deny_unknown_fields)]
struct StoredFarField {
    enabled: bool,
    inset: f64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
enum StoredSource {
    Current(StoredPointSourceV17),
    Legacy(StoredPointSourceV16),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPointSourceV17 {
    enabled: bool,
    position: [f64; 2],
    width: f64,
    region: u64,
    signal: StoredTimeSignal,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPointSourceV16 {
    enabled: bool,
    position: [f64; 2],
    amplitude: f32,
    width: f32,
    frequency_hz: f32,
    region: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredProbe {
    id: u64,
    name: String,
    color: [u8; 3],
    enabled: bool,
    target: StoredProbeTarget,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredProbeTarget {
    Point {
        position: [f64; 2],
    },
    Segment {
        start: [f64; 2],
        end: [f64; 2],
        preset: StoredProbeSamplingPreset,
    },
    Boundary {
        feature: StoredBoundaryProbeFeature,
        start_span: usize,
        span_count: usize,
        whole: bool,
        side: StoredBoundaryProbeSide,
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
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredBoundaryProbeFeature {
    Outer,
    Loop { id: u64 },
    Baffle { id: u64 },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredBoundaryProbeSide {
    Domain,
    Exterior,
    Interior,
    Left,
    Right,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredProbeSamplingPreset {
    Low,
    Medium,
    High,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredScene {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    domain: Option<[f64; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    physics: Option<StoredPhysicsModel>,
    materials: Vec<StoredMaterial>,
    regions: Vec<StoredRegion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    volume_sources: Vec<StoredVolumeSource>,
    loops: Vec<StoredLoop>,
    #[serde(default)]
    internal_boundaries: Vec<StoredInternalBoundary>,
    #[serde(default)]
    material_interfaces: Vec<StoredMaterialInterface>,
    #[serde(default)]
    junctions: Vec<StoredJunction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    outer_boundaries: Option<[StoredOuterBoundaryCondition; 4]>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredVolumeSource {
    region: u64,
    enabled: bool,
    profile: StoredScalarField,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    parameters: Vec<StoredMaterialParameter>,
    signal: StoredTimeSignal,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredMaterial {
    id: u64,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    law: Option<StoredMaterialLaw>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mass_density: Option<StoredScalarField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stiffness: Option<StoredScalarField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    damping: Option<StoredScalarField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    axis_ratio: Option<StoredScalarField>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    parameters: Vec<StoredMaterialParameter>,
    color: [u8; 3],
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StoredPhysicsModel {
    Mechanical,
    Electromagnetic { polarization: StoredPolarization },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredPolarization {
    Tm,
    Te,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredMaterialLaw {
    Mechanical {
        density: StoredScalarField,
        stiffness: StoredScalarField,
        damping: StoredScalarField,
    },
    Electromagnetic {
        permittivity: StoredScalarField,
        permeability: StoredScalarField,
        loss_rate: StoredScalarField,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum StoredScalarField {
    Legacy(f64),
    Field(StoredScalarFieldV14),
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StoredScalarFieldV14 {
    Constant { value: f64 },
    Formula { source: String },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredMaterialParameter {
    name: String,
    value: f64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredRegion {
    id: u64,
    material: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    frame: Option<StoredMaterialFrame>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredMaterialFrame {
    origin: [f64; 2],
    angle_radians: f64,
    attachment: StoredMaterialFrameAttachment,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredMaterialFrameAttachment {
    World,
    FollowRegion,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    multiplicities: Vec<u8>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    multiplicities: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredMaterialInterface {
    id: u64,
    closed: bool,
    controls: Vec<[f64; 2]>,
    intervals: Vec<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    multiplicities: Vec<u8>,
    nodes: Vec<StoredInterfaceNode>,
    span_sides: Vec<StoredInterfaceSpanSides>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredInterfaceNode {
    id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    junction: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredInterfaceSpanSides {
    left: u64,
    right: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredJunction {
    id: u64,
    location: StoredJunctionLocation,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StoredJunctionLocation {
    Interior,
    Outer {
        side: StoredOuterSide,
        fraction: f64,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredOuterSide {
    Bottom,
    Right,
    Top,
    Left,
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
pub(crate) enum StoredFaceCondition {
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
pub(crate) enum StoredOuterBoundaryCondition {
    Reflecting,
    FirstOrderOutgoing,
    SecondOrderOutgoing,
    ElectricWall,
    MagneticWall,
    Neumann { signal: StoredTimeSignal },
    Dirichlet { signal: StoredTimeSignal },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
pub(crate) enum StoredTimeSignal {
    Current(StoredTimeSignalV17),
    Legacy(StoredHarmonicSignalV16),
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StoredTimeSignalV17 {
    Harmonic {
        offset: f64,
        amplitude: f64,
        frequency_hz: f64,
        phase_radians: f64,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredHarmonicSignalV16 {
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
        domain: Some([
            scene.domain.min_x,
            scene.domain.max_x,
            scene.domain.min_y,
            scene.domain.max_y,
        ]),
        physics: Some(encode_physics(scene.physics)),
        materials: scene
            .materials
            .iter()
            .map(|material| StoredMaterial {
                id: material.id.0,
                name: material.name.clone(),
                law: Some(match scene.physics {
                    PhysicsModel::Mechanical => StoredMaterialLaw::Mechanical {
                        density: encode_scalar_field(&material.mass_density),
                        stiffness: encode_scalar_field(&material.stiffness),
                        damping: encode_scalar_field(&material.damping),
                    },
                    PhysicsModel::Electromagnetic { .. } => StoredMaterialLaw::Electromagnetic {
                        permittivity: encode_scalar_field(&material.mass_density),
                        permeability: encode_scalar_field(&material.stiffness),
                        loss_rate: encode_scalar_field(&material.damping),
                    },
                }),
                mass_density: None,
                stiffness: None,
                damping: None,
                axis_ratio: (material.axis_ratio.constant_value() != Some(1.0))
                    .then(|| encode_scalar_field(&material.axis_ratio)),
                parameters: material
                    .parameters
                    .iter()
                    .map(|parameter| StoredMaterialParameter {
                        name: parameter.name.clone(),
                        value: parameter.value,
                    })
                    .collect(),
                color: material.color,
            })
            .collect(),
        regions: scene
            .regions
            .iter()
            .map(|region| StoredRegion {
                id: region.id.0,
                material: region.material.0,
                frame: Some(StoredMaterialFrame {
                    origin: [region.frame.origin.x, region.frame.origin.y],
                    angle_radians: region.frame.angle_radians,
                    attachment: match region.frame.attachment {
                        MaterialFrameAttachment::World => StoredMaterialFrameAttachment::World,
                        MaterialFrameAttachment::FollowRegion => {
                            StoredMaterialFrameAttachment::FollowRegion
                        }
                    },
                }),
            })
            .collect(),
        volume_sources: scene
            .volume_sources
            .iter()
            .map(|source| StoredVolumeSource {
                region: source.region.0,
                enabled: source.enabled,
                profile: encode_scalar_field(&source.profile),
                parameters: source
                    .parameters
                    .iter()
                    .map(|parameter| StoredMaterialParameter {
                        name: parameter.name.clone(),
                        value: parameter.value,
                    })
                    .collect(),
                signal: encode_signal(source.signal),
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
                multiplicities: loop_.spline.multiplicities().to_vec(),
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
                multiplicities: boundary.spline.multiplicities().to_vec(),
            })
            .collect(),
        material_interfaces: scene
            .material_interfaces
            .iter()
            .map(|interface| {
                let (closed, controls, intervals, multiplicities) = match &interface.spline {
                    InterfaceSpline::Closed(spline) => (
                        true,
                        spline.controls(),
                        spline.intervals(),
                        spline.multiplicities(),
                    ),
                    InterfaceSpline::Open(spline) => (
                        false,
                        spline.controls(),
                        spline.intervals(),
                        spline.multiplicities(),
                    ),
                };
                StoredMaterialInterface {
                    id: interface.id.0,
                    closed,
                    controls: controls.iter().map(|point| [point.x, point.y]).collect(),
                    intervals: intervals.to_vec(),
                    multiplicities: multiplicities.to_vec(),
                    nodes: interface
                        .nodes
                        .iter()
                        .map(|node| StoredInterfaceNode {
                            id: node.id.0,
                            junction: node.junction.map(|id| id.0),
                        })
                        .collect(),
                    span_sides: interface
                        .span_sides
                        .iter()
                        .map(|sides| StoredInterfaceSpanSides {
                            left: sides.left.0,
                            right: sides.right.0,
                        })
                        .collect(),
                }
            })
            .collect(),
        junctions: scene
            .junctions
            .iter()
            .map(|junction| StoredJunction {
                id: junction.id.0,
                location: match junction.location {
                    JunctionLocation::Interior => StoredJunctionLocation::Interior,
                    JunctionLocation::Outer { side, fraction } => StoredJunctionLocation::Outer {
                        side: match side {
                            OuterSide::Bottom => StoredOuterSide::Bottom,
                            OuterSide::Right => StoredOuterSide::Right,
                            OuterSide::Top => StoredOuterSide::Top,
                            OuterSide::Left => StoredOuterSide::Left,
                        },
                        fraction,
                    },
                },
            })
            .collect(),
        outer_boundaries: Some(scene.outer_boundaries.sides.map(encode_outer_condition)),
    }
}

pub(crate) fn encode_physics(physics: PhysicsModel) -> StoredPhysicsModel {
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

pub(crate) fn decode_physics(physics: StoredPhysicsModel) -> PhysicsModel {
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

pub(crate) fn encode_scalar_field(field: &ScalarField) -> StoredScalarField {
    StoredScalarField::Field(match field {
        ScalarField::Constant(value) => StoredScalarFieldV14::Constant { value: *value },
        ScalarField::Formula(formula) => StoredScalarFieldV14::Formula {
            source: formula.source().into(),
        },
    })
}

pub(crate) fn decode_scalar_field(
    field: StoredScalarField,
    require_tagged: bool,
) -> Result<ScalarField, String> {
    match field {
        StoredScalarField::Legacy(_) if require_tagged => {
            Err("Version 14 material coefficients require an explicit kind".into())
        }
        StoredScalarField::Legacy(value) => Ok(ScalarField::constant(value)),
        StoredScalarField::Field(StoredScalarFieldV14::Constant { value }) => {
            Ok(ScalarField::constant(value))
        }
        StoredScalarField::Field(StoredScalarFieldV14::Formula { source }) => {
            ScalarField::formula(source).map_err(|error| error.to_string())
        }
    }
}

pub(crate) fn encode_signal(signal: TimeSignal) -> StoredTimeSignal {
    let [offset, amplitude, frequency_hz, phase_radians] = signal.harmonic_parameters();
    StoredTimeSignal::Current(StoredTimeSignalV17::Harmonic {
        offset,
        amplitude,
        frequency_hz,
        phase_radians,
    })
}

pub(crate) fn decode_signal(signal: StoredTimeSignal) -> TimeSignal {
    let (offset, amplitude, frequency_hz, phase_radians) = match signal {
        StoredTimeSignal::Current(StoredTimeSignalV17::Harmonic {
            offset,
            amplitude,
            frequency_hz,
            phase_radians,
        }) => (offset, amplitude, frequency_hz, phase_radians),
        StoredTimeSignal::Legacy(StoredHarmonicSignalV16 {
            offset,
            amplitude,
            frequency_hz,
            phase_radians,
        }) => (offset, amplitude, frequency_hz, phase_radians),
    };
    TimeSignal::Harmonic {
        offset,
        amplitude,
        frequency_hz,
        phase_radians,
    }
}

pub(crate) fn encode_outer_condition(
    condition: OuterBoundaryCondition,
) -> StoredOuterBoundaryCondition {
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

pub(crate) fn decode_outer_condition(
    condition: StoredOuterBoundaryCondition,
) -> OuterBoundaryCondition {
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

pub(crate) fn encode_face_condition(condition: FaceBoundaryCondition) -> StoredFaceCondition {
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

pub(crate) fn decode_face_condition(condition: StoredFaceCondition) -> FaceBoundaryCondition {
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

pub(crate) fn decode_spline(
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

pub(crate) fn decode_open_spline(
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

fn decode_domain(values: [f64; 4]) -> Result<DomainRect, String> {
    let domain = DomainRect::new(values[0], values[1], values[2], values[3]);
    domain
        .valid()
        .then_some(domain)
        .ok_or_else(|| "Scene contains invalid domain extents".into())
}

fn decode_v1(loops: Vec<StoredLoopV1>, domain: DomainRect) -> Result<Scene, String> {
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
                decode_spline(loop_.controls, loop_.intervals, vec![])?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let scene = Scene {
        domain,
        obstacles,
        ..Scene::default()
    };
    if !scene.structure_valid() {
        return Err("Duplicate or invalid loop IDs".into());
    }
    Ok(scene)
}

#[derive(Clone, Copy)]
struct SceneDecodeOptions {
    legacy_domain: DomainRect,
    require_domain: bool,
    require_loop_conditions: bool,
    require_outer_boundaries: bool,
    normalize_legacy_parallel_gap: bool,
    require_material_frames: bool,
    require_volume_sources: bool,
    require_physics: bool,
}

fn decode_scene(stored: StoredScene, options: SceneDecodeOptions) -> Result<Scene, String> {
    let SceneDecodeOptions {
        legacy_domain,
        require_domain,
        require_loop_conditions,
        require_outer_boundaries,
        normalize_legacy_parallel_gap,
        require_material_frames,
        require_volume_sources,
        require_physics,
    } = options;
    if stored.loops.len() > MAX_OBSTACLES
        || stored.internal_boundaries.len() > MAX_INTERNAL_BOUNDARIES
        || stored.material_interfaces.len() > MAX_MATERIAL_INTERFACES
        || stored.junctions.len() > MAX_JUNCTIONS
        || stored.loops.len() + stored.internal_boundaries.len() + stored.material_interfaces.len()
            > MAX_OBSTACLES
        || stored.materials.len() > MAX_MATERIALS
        || stored.volume_sources.len() > MAX_VOLUME_SOURCES
    {
        return Err("Scene exceeds the loop or material limit".into());
    }
    let physics = match stored.physics {
        Some(physics) => decode_physics(physics),
        None if require_physics => return Err("Scene has no physics model".into()),
        None => PhysicsModel::Mechanical,
    };
    let materials = stored
        .materials
        .into_iter()
        .map(|material| {
            let (mass_density, stiffness, damping) = match material.law {
                Some(StoredMaterialLaw::Mechanical {
                    density,
                    stiffness,
                    damping,
                }) if physics == PhysicsModel::Mechanical => {
                    if material.mass_density.is_some()
                        || material.stiffness.is_some()
                        || material.damping.is_some()
                    {
                        return Err("Material mixes version 18 and legacy coefficients".into());
                    }
                    (density, stiffness, damping)
                }
                Some(StoredMaterialLaw::Electromagnetic {
                    permittivity,
                    permeability,
                    loss_rate,
                }) if matches!(physics, PhysicsModel::Electromagnetic { .. }) => {
                    if material.mass_density.is_some()
                        || material.stiffness.is_some()
                        || material.damping.is_some()
                    {
                        return Err("Material mixes version 18 and legacy coefficients".into());
                    }
                    (permittivity, permeability, loss_rate)
                }
                Some(_) => return Err("Material law does not match the scene physics".into()),
                None if require_physics => return Err("Material has no version 18 law".into()),
                None => (
                    material
                        .mass_density
                        .ok_or("Legacy material has no density")?,
                    material
                        .stiffness
                        .ok_or("Legacy material has no stiffness")?,
                    material.damping.ok_or("Legacy material has no damping")?,
                ),
            };
            Ok(Material {
                id: MaterialId(material.id),
                name: material.name,
                mass_density: decode_scalar_field(mass_density, require_material_frames)?,
                stiffness: decode_scalar_field(stiffness, require_material_frames)?,
                damping: decode_scalar_field(damping, require_material_frames)?,
                axis_ratio: material
                    .axis_ratio
                    .map(|field| decode_scalar_field(field, require_material_frames))
                    .transpose()?
                    .unwrap_or_else(|| ScalarField::constant(1.0)),
                parameters: material
                    .parameters
                    .into_iter()
                    .map(|parameter| MaterialParameter {
                        name: parameter.name,
                        value: parameter.value,
                    })
                    .collect(),
                color: material.color,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut regions = stored
        .regions
        .into_iter()
        .map(|region| {
            let frame = match region.frame {
                Some(frame) => MaterialFrame {
                    origin: Point2::new(frame.origin[0], frame.origin[1]),
                    angle_radians: frame.angle_radians,
                    attachment: match frame.attachment {
                        StoredMaterialFrameAttachment::World => MaterialFrameAttachment::World,
                        StoredMaterialFrameAttachment::FollowRegion => {
                            MaterialFrameAttachment::FollowRegion
                        }
                    },
                },
                None if !require_material_frames => MaterialFrame::world(),
                None => return Err("Scene region has no material frame".into()),
            };
            Ok(Region {
                id: RegionId(region.id),
                material: MaterialId(region.material),
                frame,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if !require_volume_sources && !stored.volume_sources.is_empty() {
        return Err("Volume sources require scene version 15".into());
    }
    let volume_sources = stored
        .volume_sources
        .into_iter()
        .map(|source| {
            Ok(VolumeSource {
                region: RegionId(source.region),
                enabled: source.enabled,
                profile: decode_scalar_field(source.profile, true)?,
                parameters: source
                    .parameters
                    .into_iter()
                    .map(|parameter| MaterialParameter {
                        name: parameter.name,
                        value: parameter.value,
                    })
                    .collect(),
                signal: decode_signal(source.signal),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
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
            let spline = decode_spline(loop_.controls, loop_.intervals, loop_.multiplicities)?;
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
            let spline = decode_open_spline(
                boundary.controls,
                boundary.intervals,
                boundary.multiplicities,
            )?;
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
                    .map(|law| {
                        let coupling = match law.coupling {
                            StoredCoupling::Independent => InternalBoundaryCoupling::Independent,
                            StoredCoupling::ThinGap { stiffness_ratio } => {
                                InternalBoundaryCoupling::ThinGap { stiffness_ratio }
                            }
                        };
                        let mut decoded = InternalBoundaryLaw {
                            left: decode_face_condition(law.left),
                            right: decode_face_condition(law.right),
                            coupling,
                        };
                        if normalize_legacy_parallel_gap
                            && matches!(coupling, InternalBoundaryCoupling::ThinGap { .. })
                        {
                            decoded.left = FaceBoundaryCondition::Reflecting;
                            decoded.right = FaceBoundaryCondition::Reflecting;
                        }
                        decoded
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
    let material_interfaces = stored
        .material_interfaces
        .into_iter()
        .map(|interface| {
            let spline = if interface.closed {
                InterfaceSpline::Closed(decode_spline(
                    interface.controls,
                    interface.intervals,
                    interface.multiplicities,
                )?)
            } else {
                InterfaceSpline::Open(decode_open_spline(
                    interface.controls,
                    interface.intervals,
                    interface.multiplicities,
                )?)
            };
            Ok(MaterialInterface {
                id: MaterialInterfaceId(interface.id),
                spline,
                nodes: interface
                    .nodes
                    .into_iter()
                    .map(|node| InterfaceNode {
                        id: InterfaceNodeId(node.id),
                        junction: node.junction.map(JunctionId),
                    })
                    .collect(),
                span_sides: interface
                    .span_sides
                    .into_iter()
                    .map(|sides| InterfaceSpanSides {
                        left: RegionId(sides.left),
                        right: RegionId(sides.right),
                    })
                    .collect(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let junctions = stored
        .junctions
        .into_iter()
        .map(|junction| Junction {
            id: JunctionId(junction.id),
            location: match junction.location {
                StoredJunctionLocation::Interior => JunctionLocation::Interior,
                StoredJunctionLocation::Outer { side, fraction } => JunctionLocation::Outer {
                    side: match side {
                        StoredOuterSide::Bottom => OuterSide::Bottom,
                        StoredOuterSide::Right => OuterSide::Right,
                        StoredOuterSide::Top => OuterSide::Top,
                        StoredOuterSide::Left => OuterSide::Left,
                    },
                    fraction,
                },
            },
        })
        .collect();
    let outer_boundaries = match stored.outer_boundaries {
        Some(conditions) => OuterBoundaryConditions {
            sides: conditions.map(decode_outer_condition),
        },
        // Versions before 6 had implicit reflecting outer walls. Keep their
        // physical meaning even though new documents now default to absorption.
        None if !require_outer_boundaries => {
            OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting)
        }
        None => return Err("Scene has no outer boundary conditions".into()),
    };
    if !require_material_frames {
        for region in &mut regions {
            if region.id == BACKGROUND_REGION {
                region.frame = MaterialFrame::world();
                continue;
            }
            let Some(owner) = obstacles
                .iter()
                .find(|obstacle| obstacle.role.interior() == Some(region.id))
            else {
                continue;
            };
            region.frame = MaterialFrame {
                origin: sampled_loop_bounds_center(&owner.spline)?,
                angle_radians: 0.0,
                attachment: MaterialFrameAttachment::FollowRegion,
            };
        }
    }
    let domain = match stored.domain {
        Some(domain) if require_domain => decode_domain(domain)?,
        Some(_) | None if !require_domain => legacy_domain,
        None => return Err("Scene has no domain extents".into()),
        Some(_) => unreachable!(),
    };
    let scene = Scene {
        domain,
        physics,
        obstacles,
        internal_boundaries,
        material_interfaces,
        junctions,
        materials,
        regions,
        volume_sources,
        outer_boundaries,
    };
    if !scene.structure_valid() {
        return Err("Scene contains invalid IDs, materials, or region references".into());
    }
    Ok(scene)
}

fn sampled_loop_bounds_center(spline: &PeriodicCubicSpline) -> Result<Point2, String> {
    let samples = sample(
        spline,
        SamplingOptions {
            tolerance: 1.0e-4,
            max_depth: 14,
            max_points: 4096,
        },
    )
    .map_err(|_| "Could not fit a material frame to its region")?;
    let first = samples
        .first()
        .ok_or("Could not fit a material frame to an empty region")?
        .point;
    let (minimum, maximum) = samples
        .iter()
        .fold((first, first), |(minimum, maximum), sample| {
            (
                Point2::new(minimum.x.min(sample.point.x), minimum.y.min(sample.point.y)),
                Point2::new(maximum.x.max(sample.point.x), maximum.y.max(sample.point.y)),
            )
        });
    Ok((minimum + maximum) / 2.0)
}

pub fn save(document: &Document) -> Result<String, String> {
    serde_json::to_string_pretty(&encode_document(document)).map_err(|error| error.to_string())
}

#[doc(hidden)]
pub fn save_compact(document: &Document) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&encode_document(document)).map_err(|error| error.to_string())
}

fn encode_document(document: &Document) -> FileV2 {
    FileV2 {
        version: 21,
        domain: [
            document.model.accepted.domain.min_x,
            document.model.accepted.domain.max_x,
            document.model.accepted.domain.min_y,
            document.model.accepted.domain.max_y,
        ],
        draft: encode_scene(&document.model.draft),
        accepted: encode_scene(&document.model.accepted),
        probes: document
            .model
            .probes
            .iter()
            .map(|probe| StoredProbe {
                id: probe.id.0,
                name: probe.name.clone(),
                color: probe.color,
                enabled: probe.enabled,
                target: match probe.target {
                    ProbeTarget::Point(position) => StoredProbeTarget::Point {
                        position: [position.x, position.y],
                    },
                    ProbeTarget::Segment { start, end, preset } => StoredProbeTarget::Segment {
                        start: [start.x, start.y],
                        end: [end.x, end.y],
                        preset: encode_probe_preset(preset),
                    },
                    ProbeTarget::Boundary(target) => StoredProbeTarget::Boundary {
                        feature: match target.feature {
                            BoundaryProbeFeature::Outer => StoredBoundaryProbeFeature::Outer,
                            BoundaryProbeFeature::Loop(id) => {
                                StoredBoundaryProbeFeature::Loop { id: id.0 }
                            }
                            BoundaryProbeFeature::Baffle(id) => {
                                StoredBoundaryProbeFeature::Baffle { id: id.0 }
                            }
                        },
                        start_span: target.start_span,
                        span_count: target.span_count,
                        whole: target.whole,
                        side: match target.side {
                            BoundaryProbeSide::Domain => StoredBoundaryProbeSide::Domain,
                            BoundaryProbeSide::Exterior => StoredBoundaryProbeSide::Exterior,
                            BoundaryProbeSide::Interior => StoredBoundaryProbeSide::Interior,
                            BoundaryProbeSide::Left => StoredBoundaryProbeSide::Left,
                            BoundaryProbeSide::Right => StoredBoundaryProbeSide::Right,
                        },
                        reversed: target.reversed,
                        preset: encode_probe_preset(target.preset),
                    },
                    ProbeTarget::AreaDisk { center, radius } => StoredProbeTarget::AreaDisk {
                        center: [center.x, center.y],
                        radius,
                    },
                    ProbeTarget::AreaRegion { region } => {
                        StoredProbeTarget::AreaRegion { region: region.0 }
                    }
                },
            })
            .collect(),
        source: Some(StoredSource::Current(StoredPointSourceV17 {
            enabled: document.model.source.enabled,
            position: [
                document.model.source.position.x,
                document.model.source.position.y,
            ],
            width: document.model.source.width,
            region: document.model.source.region.0,
            signal: encode_signal(document.model.source.signal),
        })),
        far_field: Some(StoredFarField {
            enabled: document.model.far_field.enabled,
            inset: document.model.far_field.inset,
        }),
        presentation: Some(encode_presentation(document.presentation)),
    }
}

pub(crate) fn encode_presentation(settings: PresentationSettings) -> StoredPresentation {
    StoredPresentation {
        grid: settings.grid,
        control_polygons: settings.control_polygons,
        handles: settings.handles,
        accepted_reference: settings.accepted_reference,
        boundary_conditions: settings.boundary_conditions,
        mesh: settings.mesh,
        mesh_boundaries: settings.mesh_boundaries,
        adaptation_target: settings.adaptation_target,
        point_probes: settings.point_probes,
        line_probes: settings.line_probes,
        boundary_probes: settings.boundary_probes,
        area_probes: settings.area_probes,
        far_field_contour: settings.far_field_contour,
        field: settings.field,
        field_gain: settings.field_gain,
        vector_overlay: match settings.vector_overlay {
            VectorOverlay::Off => StoredVectorOverlay::Off,
            VectorOverlay::ComplementaryField => StoredVectorOverlay::ComplementaryField,
            VectorOverlay::RelativeEnergyFlow => StoredVectorOverlay::RelativeEnergyFlow,
        },
        vector_overlay_smoothed: settings.vector_overlay_smoothed,
        vector_overlay_density: settings.vector_overlay_density,
        vector_overlay_gain: settings.vector_overlay_gain,
        material_overlay: match settings.material_overlay {
            MaterialOverlay::Off => StoredMaterialOverlay::Off,
            MaterialOverlay::Regions => StoredMaterialOverlay::Regions,
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

pub(crate) fn decode_presentation(
    stored: StoredPresentation,
) -> Result<PresentationSettings, String> {
    let settings = PresentationSettings {
        grid: stored.grid,
        control_polygons: stored.control_polygons,
        handles: stored.handles,
        accepted_reference: stored.accepted_reference,
        boundary_conditions: stored.boundary_conditions,
        mesh: stored.mesh,
        mesh_boundaries: stored.mesh_boundaries,
        adaptation_target: stored.adaptation_target,
        point_probes: stored.point_probes,
        line_probes: stored.line_probes,
        boundary_probes: stored.boundary_probes,
        area_probes: stored.area_probes,
        far_field_contour: stored.far_field_contour,
        field: stored.field,
        field_gain: stored.field_gain,
        vector_overlay: match stored.vector_overlay {
            StoredVectorOverlay::Off => VectorOverlay::Off,
            StoredVectorOverlay::ComplementaryField => VectorOverlay::ComplementaryField,
            StoredVectorOverlay::RelativeEnergyFlow => VectorOverlay::RelativeEnergyFlow,
        },
        vector_overlay_smoothed: stored.vector_overlay_smoothed,
        vector_overlay_density: stored.vector_overlay_density,
        vector_overlay_gain: stored.vector_overlay_gain,
        material_overlay: match stored.material_overlay {
            StoredMaterialOverlay::Off => MaterialOverlay::Off,
            StoredMaterialOverlay::Regions => MaterialOverlay::Regions,
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

pub(crate) fn encode_probe_preset(preset: ProbeSamplingPreset) -> StoredProbeSamplingPreset {
    match preset {
        ProbeSamplingPreset::Low => StoredProbeSamplingPreset::Low,
        ProbeSamplingPreset::Medium => StoredProbeSamplingPreset::Medium,
        ProbeSamplingPreset::High => StoredProbeSamplingPreset::High,
    }
}

pub(crate) fn decode_probe_preset(preset: StoredProbeSamplingPreset) -> ProbeSamplingPreset {
    match preset {
        StoredProbeSamplingPreset::Low => ProbeSamplingPreset::Low,
        StoredProbeSamplingPreset::Medium => ProbeSamplingPreset::Medium,
        StoredProbeSamplingPreset::High => ProbeSamplingPreset::High,
    }
}

fn decode_source(stored: Option<StoredSource>, accepted: &Scene) -> Result<PointSource, String> {
    let Some(stored) = stored else {
        return Ok(PointSource::default());
    };
    let source = match stored {
        StoredSource::Current(stored) => PointSource {
            enabled: stored.enabled,
            position: Point2::new(stored.position[0], stored.position[1]),
            width: stored.width,
            region: RegionId(stored.region),
            signal: decode_signal(stored.signal),
        },
        StoredSource::Legacy(stored) => PointSource {
            enabled: stored.enabled,
            position: Point2::new(stored.position[0], stored.position[1]),
            width: stored.width as f64,
            region: RegionId(stored.region),
            signal: TimeSignal::harmonic(
                0.0,
                stored.amplitude as f64,
                stored.frequency_hz as f64,
                0.0,
            ),
        },
    };
    if !source.valid() {
        return Err("Scene contains invalid point-source settings".into());
    }
    if accepted.region(source.region).is_none() {
        return Err("Point source references a missing region".into());
    }
    Ok(source)
}

fn decode_probes(
    stored: Vec<StoredProbe>,
    draft: &Scene,
    allow_boundary: bool,
    allow_area: bool,
) -> Result<Vec<ProbeDefinition>, String> {
    if stored.len() > MAX_PROBES {
        return Err(format!("Scene contains more than {MAX_PROBES} probes"));
    }
    let probes = stored
        .into_iter()
        .map(|probe| -> Result<ProbeDefinition, String> {
            Ok(ProbeDefinition {
                id: ProbeId(probe.id),
                name: probe.name,
                color: probe.color,
                enabled: probe.enabled,
                target: match probe.target {
                    StoredProbeTarget::Point { position } => {
                        ProbeTarget::Point(Point2::new(position[0], position[1]))
                    }
                    StoredProbeTarget::Segment { start, end, preset } => ProbeTarget::Segment {
                        start: Point2::new(start[0], start[1]),
                        end: Point2::new(end[0], end[1]),
                        preset: decode_probe_preset(preset),
                    },
                    StoredProbeTarget::Boundary {
                        feature,
                        start_span,
                        span_count,
                        whole,
                        side,
                        reversed,
                        preset,
                    } => ProbeTarget::Boundary(BoundaryProbeTarget {
                        feature: match feature {
                            StoredBoundaryProbeFeature::Outer => BoundaryProbeFeature::Outer,
                            StoredBoundaryProbeFeature::Loop { id } => {
                                BoundaryProbeFeature::Loop(ObstacleId(id))
                            }
                            StoredBoundaryProbeFeature::Baffle { id } => {
                                BoundaryProbeFeature::Baffle(InternalBoundaryId(id))
                            }
                        },
                        start_span,
                        span_count,
                        whole,
                        side: match side {
                            StoredBoundaryProbeSide::Domain => BoundaryProbeSide::Domain,
                            StoredBoundaryProbeSide::Exterior => BoundaryProbeSide::Exterior,
                            StoredBoundaryProbeSide::Interior => BoundaryProbeSide::Interior,
                            StoredBoundaryProbeSide::Left => BoundaryProbeSide::Left,
                            StoredBoundaryProbeSide::Right => BoundaryProbeSide::Right,
                        },
                        reversed,
                        preset: decode_probe_preset(preset),
                    }),
                    StoredProbeTarget::AreaDisk { center, radius } => {
                        if !allow_area {
                            return Err("Area probes require scene version 13".into());
                        }
                        ProbeTarget::AreaDisk {
                            center: Point2::new(center[0], center[1]),
                            radius,
                        }
                    }
                    StoredProbeTarget::AreaRegion { region } => {
                        if !allow_area {
                            return Err("Area probes require scene version 13".into());
                        }
                        ProbeTarget::AreaRegion {
                            region: RegionId(region),
                        }
                    }
                },
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if probes.iter().any(|probe| !probe.valid()) {
        return Err("Scene contains an invalid probe".into());
    }
    for probe in &probes {
        if let ProbeTarget::AreaRegion { region } = probe.target
            && draft.region(region).is_none()
        {
            return Err("Area probe references a missing region".into());
        }
        let ProbeTarget::Boundary(target) = probe.target else {
            continue;
        };
        if !allow_boundary {
            return Err("Boundary probes require scene version 12".into());
        }
        let (total, side_valid) = match target.feature {
            BoundaryProbeFeature::Outer => (Some(4), target.side == BoundaryProbeSide::Domain),
            BoundaryProbeFeature::Loop(id) => draft
                .obstacles
                .iter()
                .find(|loop_| loop_.id == id)
                .map_or((None, false), |loop_| {
                    let valid = match loop_.role {
                        LoopRole::Hole { .. } => target.side == BoundaryProbeSide::Domain,
                        LoopRole::MaterialInterface { .. } | LoopRole::Wall { .. } => matches!(
                            target.side,
                            BoundaryProbeSide::Exterior | BoundaryProbeSide::Interior
                        ),
                    };
                    (Some(loop_.spline.intervals().len()), valid)
                }),
            BoundaryProbeFeature::Baffle(id) => (
                draft
                    .internal_boundaries
                    .iter()
                    .find(|boundary| boundary.id == id)
                    .map(|boundary| boundary.spline.intervals().len()),
                matches!(
                    target.side,
                    BoundaryProbeSide::Left | BoundaryProbeSide::Right
                ),
            ),
        };
        let Some(total) = total else {
            return Err("Boundary probe references a missing feature".into());
        };
        if !side_valid
            || target.start_span >= total
            || target.span_count == 0
            || target.span_count > total
            || (matches!(target.feature, BoundaryProbeFeature::Baffle(_))
                && target.start_span + target.span_count > total)
        {
            return Err("Scene contains an invalid boundary probe".into());
        }
    }
    let segment_points = probes
        .iter()
        .map(|probe| match probe.target {
            ProbeTarget::Segment { preset, .. }
            | ProbeTarget::Boundary(BoundaryProbeTarget { preset, .. }) => preset.spatial_points(),
            ProbeTarget::Point(_) => 0,
            ProbeTarget::AreaDisk { .. } | ProbeTarget::AreaRegion { .. } => 0,
        })
        .sum::<usize>();
    if segment_points > MAX_SEGMENT_PROBE_POINTS {
        return Err(format!(
            "Line probes exceed the {MAX_SEGMENT_PROBE_POINTS}-point sampling budget"
        ));
    }
    let mut ids = std::collections::BTreeSet::new();
    if probes.iter().any(|probe| !ids.insert(probe.id)) {
        return Err("Scene contains duplicate probe IDs".into());
    }
    Ok(probes)
}

/// Structural parsing only. UI advances LoadCandidate across frames before replacing.
pub fn parse(bytes: &[u8]) -> Result<LoadCandidate, String> {
    Ok(candidate(parse_document(bytes)?))
}

#[doc(hidden)]
pub fn parse_document(bytes: &[u8]) -> Result<Document, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("File exceeds 2 MiB".into());
    }
    let header: Header = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let document = match header.version {
        1 => {
            let file: FileV1 = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
            let domain = decode_domain(file.domain)?;
            Document {
                model: DocumentModel {
                    draft: decode_v1(file.draft, domain)?,
                    accepted: decode_v1(file.accepted, domain)?,
                    probes: vec![],
                    source: PointSource::default(),
                    far_field: FarFieldSettings::default(),
                },
                presentation: PresentationSettings::default(),
            }
        }
        2..=21 => {
            let file: FileV2 = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
            let legacy_domain = decode_domain(file.domain)?;
            let options = SceneDecodeOptions {
                legacy_domain,
                require_domain: header.version >= 19,
                require_loop_conditions: header.version >= 5,
                require_outer_boundaries: header.version >= 6,
                normalize_legacy_parallel_gap: header.version < 7,
                require_material_frames: header.version >= 14,
                require_volume_sources: header.version >= 15,
                require_physics: header.version >= 18,
            };
            let draft = decode_scene(file.draft, options)?;
            let accepted = decode_scene(file.accepted, options)?;
            let source = decode_source(file.source, &accepted)?;
            let probes = decode_probes(
                file.probes,
                &draft,
                header.version >= 12,
                header.version >= 13,
            )?;
            let far_field = match file.far_field {
                Some(stored) if header.version >= 13 => FarFieldSettings {
                    enabled: stored.enabled,
                    inset: stored.inset,
                },
                Some(_) => return Err("Far-field settings require scene version 13".into()),
                None => FarFieldSettings::default(),
            };
            if !far_field.valid() {
                return Err("Scene contains invalid far-field settings".into());
            }
            if 2.0 * far_field.inset >= accepted.domain.minimum_extent() {
                return Err("Far-field inset leaves no contour inside the accepted domain".into());
            }
            let presentation = match file.presentation {
                Some(stored) if header.version >= 16 => decode_presentation(stored)?,
                Some(_) => return Err("Presentation settings require scene version 16".into()),
                None => PresentationSettings::default(),
            };
            Document {
                model: DocumentModel {
                    draft,
                    accepted,
                    probes,
                    source,
                    far_field,
                },
                presentation,
            }
        }
        _ => return Err("Unsupported scene version".into()),
    };
    Ok(document)
}

#[doc(hidden)]
pub fn candidate(document: Document) -> LoadCandidate {
    let job = ValidationJob::new(document.model.accepted.clone(), 0);
    LoadCandidate { document, job }
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
