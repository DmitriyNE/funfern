//! Built-in examples authored directly in the unified topology model.

use crate::document::{
    FarFieldSettings, MaterialOverlay, MaterialProperty, PresentationSettings, ProbeId,
    ProbeSamplingPreset, VectorOverlay,
};
use crate::topology_editor::{
    TopologyDocument, TopologyDocumentModel, TopologyProbeDefinition, TopologyProbeTarget,
};
use funfern_core::*;
use std::sync::OnceLock;

pub struct TopologyExample {
    pub name: &'static str,
    pub description: &'static str,
    pub document: TopologyDocument,
}

pub fn catalog() -> &'static [TopologyExample] {
    static CATALOG: OnceLock<Vec<TopologyExample>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        vec![
            example(
                "Starter obstacle",
                "A point source scatters from a rounded obstacle above a reflecting floor.",
                starter_obstacle(),
            ),
            example(
                "Double slit",
                "A point source illuminates two apertures in a reflecting waveguide.",
                double_slit(),
            ),
            example(
                "Material lens",
                "A TM electric-field source illuminates a slower dielectric region with absorbing edges.",
                material_lens(),
            ),
            example(
                "GRIN rod",
                "An off-axis source is guided by a smooth transverse index profile.",
                grin_rod(),
            ),
            example(
                "Anisotropic crystal",
                "A rotated directional inclusion turns circular wavefronts into ellipses.",
                anisotropic_crystal(),
            ),
            example(
                "Luneburg lens",
                "A TE magnetic-field wave focuses at the far rim of a radial-index lens.",
                luneburg_lens(),
            ),
            example(
                "Phased array",
                "Five compact region sources use a phase ramp to steer a radiated beam.",
                phased_array(),
            ),
            example(
                "Obstacle array",
                "A point source drives multiple scattering through eight reflecting obstacles.",
                obstacle_array(),
            ),
            example(
                "Kerr slab",
                "A strong source drives a Kerr slab: the wave slows where it is strong, and the \
                 receiver hears the source's third harmonic.",
                kerr_slab(),
            ),
            example(
                "Parametric pump",
                "A slab pumped at twice the source frequency amplifies what it transmits, by an \
                 amount the pump's phase sets.",
                pumped_slab(),
            ),
            example(
                "Time crystal",
                "A slab whose permittivity steps up and down once a second splits the wave into \
                 sidebands; its sharp edges reach three steps out.",
                time_crystal_slab(),
            ),
            example(
                "Travelling modulation",
                "A modulation running with the wave converts it to higher frequencies; mirrored, \
                 against the wave, it barely does.",
                travelling_slab(),
            ),
        ]
    })
}

fn example(
    name: &'static str,
    description: &'static str,
    mut document: TopologyDocument,
) -> TopologyExample {
    document.presentation.boundary_conditions = true;
    document
        .model
        .accepted
        .compile(0)
        .expect("built-in topology example must compile");
    TopologyExample {
        name,
        description,
        document,
    }
}

struct Builder {
    scene: TopologyScene,
    next_curve: u64,
    next_span: u64,
    next_region: u64,
}

impl Builder {
    fn new() -> Self {
        Self {
            scene: TopologyScene::default(),
            next_curve: 1,
            next_span: 1,
            next_region: 2,
        }
    }

    fn closed(
        &mut self,
        spline: PeriodicCubicSpline,
        behavior: SpanBehavior,
        region: Option<(MaterialId, MaterialFrame)>,
    ) -> (CurveId, Option<RegionId>) {
        let curve = CurveId(self.next_curve);
        self.next_curve += 1;
        let spans = (0..spline.intervals().len())
            .map(|_| {
                let span = CurveSpan {
                    id: CurveSpanId(self.next_span),
                    behavior,
                };
                self.next_span += 1;
                span
            })
            .collect::<Vec<_>>();
        let anchor = FaceAnchor::Curve {
            curve,
            span: spans[0].id,
            side: CurveTraceSide::Left,
            parameter: spline.span_bounds(0).map(|[a, b]| (a + b) * 0.5).unwrap(),
        };
        let region = region.map(|(material, frame)| {
            let id = RegionId(self.next_region);
            self.next_region += 1;
            self.scene.regions.push(Region {
                id,
                material,
                frame,
            });
            id
        });
        self.scene
            .geometry
            .curves
            .push(TopologyCurve::new(curve, CurveSpline::Closed(spline), spans).unwrap());
        self.scene
            .face_assignments
            .push(AuthoredFaceAssignment { anchor, region });
        (curve, region)
    }

    fn hole(&mut self, spline: PeriodicCubicSpline) -> CurveId {
        self.closed(spline, SpanBehavior::REFLECTING, None).0
    }

    fn subdomain(
        &mut self,
        spline: PeriodicCubicSpline,
        material: MaterialId,
        frame: MaterialFrame,
    ) -> RegionId {
        self.closed(spline, SpanBehavior::Transmitting, Some((material, frame)))
            .1
            .unwrap()
    }

    fn baffle(&mut self, spline: OpenCubicSpline) -> CurveId {
        let curve = CurveId(self.next_curve);
        self.next_curve += 1;
        let spans = (0..spline.intervals().len())
            .map(|_| {
                let span = CurveSpan {
                    id: CurveSpanId(self.next_span),
                    behavior: SpanBehavior::REFLECTING,
                };
                self.next_span += 1;
                span
            })
            .collect();
        self.scene
            .geometry
            .curves
            .push(TopologyCurve::new(curve, CurveSpline::Open(spline), spans).unwrap());
        curve
    }

    fn document(self) -> TopologyDocument {
        let scene = self.scene;
        scene.compile(0).unwrap();
        TopologyDocument {
            model: TopologyDocumentModel {
                draft: scene.clone(),
                accepted: scene,
                probes: vec![],
                source: PointSource::default(),
                far_field: FarFieldSettings::default(),
            },
            presentation: PresentationSettings::default(),
        }
    }
}

fn source(position: Point2, frequency: f64, amplitude: f64, width: f64) -> PointSource {
    PointSource {
        enabled: true,
        position,
        width,
        region: BACKGROUND_REGION,
        signal: TimeSignal::harmonic(0.0, amplitude, frequency, 0.0),
    }
}

fn reflecting_channel() -> OuterBoundaryConditions {
    let mut boundaries = OuterBoundaryConditions::default();
    boundaries.sides[OuterSide::Bottom.index()] = OuterBoundaryCondition::Reflecting;
    boundaries.sides[OuterSide::Top.index()] = OuterBoundaryCondition::Reflecting;
    boundaries
}

fn starter_obstacle() -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.outer_boundaries = reflecting_channel();
    builder.hole(PeriodicCubicSpline::rounded(Point2::new(0.1, 0.05), 0.22));
    let mut document = builder.document();
    document.model.source = source(Point2::new(-0.55, 0.05), 2.5, 18.0, 0.06);
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Receiver".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Point(Point2::new(0.5, 0.15)),
    });
    document
}

fn straight_baffle(y0: f64, y1: f64) -> OpenCubicSpline {
    OpenCubicSpline::polyline(vec![Point2::new(0.0, y0), Point2::new(0.0, y1)]).unwrap()
}

fn double_slit() -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.outer_boundaries = reflecting_channel();
    builder.baffle(straight_baffle(-0.9, -0.36));
    builder.baffle(straight_baffle(-0.14, 0.14));
    builder.baffle(straight_baffle(0.36, 0.9));
    let mut document = builder.document();
    document.model.source = source(Point2::new(-0.62, 0.0), 3.0, 20.0, 0.05);
    document
}

fn material_lens() -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.outer_boundaries =
        OuterBoundaryConditions::uniform(OuterBoundaryCondition::FirstOrderOutgoing);
    builder.scene.materials.push(Material {
        id: MaterialId(2),
        name: "Slow lens".into(),
        mass_density: ScalarField::constant(1.0 / 0.36),
        stiffness: ScalarField::constant(1.0),
        damping: ScalarField::constant(0.0),
        axis_ratio: ScalarField::constant(1.0),
        parameters: vec![],
        color: [61, 116, 139],
        ..Material::default_medium()
    });
    builder.subdomain(
        PeriodicCubicSpline::rounded(Point2::default(), 0.45),
        MaterialId(2),
        MaterialFrame {
            attachment: MaterialFrameAttachment::FollowRegion,
            ..MaterialFrame::world()
        },
    );
    let mut document = builder.document();
    document.model.source = source(Point2::new(-0.72, 0.0), 3.5, 16.0, 0.045);
    document.presentation.vector_overlay = VectorOverlay::ComplementaryField;
    document
}

fn grin_rod() -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.materials.push(Material {
        id: MaterialId(2),
        name: "GRIN profile".into(),
        mass_density: ScalarField::formula("1 + dn * smoothstep(0, 1, 1 - (y / H)^2)").unwrap(),
        stiffness: ScalarField::formula("1 / (1 + dn * smoothstep(0, 1, 1 - (y / H)^2))").unwrap(),
        damping: ScalarField::constant(0.0),
        axis_ratio: ScalarField::constant(1.0),
        parameters: vec![
            MaterialParameter {
                name: "H".into(),
                value: 0.30,
            },
            MaterialParameter {
                name: "dn".into(),
                value: 0.60,
            },
        ],
        color: [46, 120, 139],
        ..Material::default_medium()
    });
    let region = builder.subdomain(
        PeriodicCubicSpline::polygon(vec![
            Point2::new(-0.70, -0.24),
            Point2::new(0.70, -0.24),
            Point2::new(0.74, 0.0),
            Point2::new(0.70, 0.24),
            Point2::new(-0.70, 0.24),
            Point2::new(-0.74, 0.0),
        ])
        .unwrap(),
        MaterialId(2),
        MaterialFrame {
            attachment: MaterialFrameAttachment::FollowRegion,
            ..MaterialFrame::world()
        },
    );
    let mut document = builder.document();
    document.model.source = PointSource {
        region,
        ..source(Point2::new(-0.56, 0.11), 4.0, 16.0, 0.04)
    };
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Rod output profile".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Segment {
            start: Point2::new(0.50, -0.38),
            end: Point2::new(0.50, 0.38),
            preset: ProbeSamplingPreset::High,
        },
    });
    document.presentation.material_overlay = MaterialOverlay::Property(MaterialProperty::WaveSpeed);
    document.presentation.material_overlay_opacity = 0.55;
    document
}

fn anisotropic_crystal() -> TopologyDocument {
    let mut builder = Builder::new();
    let center = Point2::new(0.08, 0.0);
    builder.scene.materials.push(Material {
        id: MaterialId(2),
        name: "Rotated crystal".into(),
        mass_density: ScalarField::constant(1.0),
        stiffness: ScalarField::constant(1.0),
        damping: ScalarField::constant(0.0),
        axis_ratio: ScalarField::constant(2.4),
        parameters: vec![],
        color: [54, 125, 126],
        ..Material::default_medium()
    });
    builder.subdomain(
        PeriodicCubicSpline::rounded(center, 0.48),
        MaterialId(2),
        MaterialFrame {
            origin: center,
            angle_radians: 32.0_f64.to_radians(),
            attachment: MaterialFrameAttachment::FollowRegion,
        },
    );
    let mut document = builder.document();
    document.model.source = source(Point2::new(-0.72, 0.0), 3.2, 15.0, 0.045);
    document.presentation.material_overlay =
        MaterialOverlay::Property(MaterialProperty::Anisotropy);
    document.presentation.material_overlay_opacity = 0.56;
    document.presentation.material_overlay_logarithmic = true;
    document
}

fn luneburg_lens() -> TopologyDocument {
    let mut builder = Builder::new();
    let center = Point2::new(0.08, 0.0);
    let radius = 0.43;
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Te,
    };
    builder.scene.outer_boundaries.sides[OuterSide::Left.index()] =
        OuterBoundaryCondition::Dirichlet {
            signal: TimeSignal::harmonic(0.0, 0.7, 3.5, 0.0),
        };
    builder.scene.materials.push(Material {
        id: MaterialId(2),
        name: "Luneburg profile".into(),
        mass_density: ScalarField::formula("max(2 - (r / R)^2, 1)").unwrap(),
        stiffness: ScalarField::constant(1.0),
        damping: ScalarField::constant(0.0),
        axis_ratio: ScalarField::constant(1.0),
        parameters: vec![MaterialParameter {
            name: "R".into(),
            value: radius,
        }],
        color: [66, 105, 151],
        ..Material::default_medium()
    });
    let region = builder.subdomain(
        PeriodicCubicSpline::rounded(center, radius),
        MaterialId(2),
        MaterialFrame {
            origin: center,
            attachment: MaterialFrameAttachment::FollowRegion,
            ..MaterialFrame::world()
        },
    );
    let mut document = builder.document();
    document.model.source.enabled = false;
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Focus energy".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::AreaDisk {
            center: Point2::new(center.x + radius - 0.025, 0.0),
            radius: 0.065,
        },
    });
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(2),
        name: "Lens energy".into(),
        color: [248, 196, 112],
        enabled: true,
        target: TopologyProbeTarget::AreaRegion(region),
    });
    document.presentation.material_overlay = MaterialOverlay::Property(MaterialProperty::WaveSpeed);
    document.presentation.vector_overlay = VectorOverlay::ComplementaryField;
    document
}

fn phased_array() -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.outer_boundaries =
        OuterBoundaryConditions::uniform(OuterBoundaryCondition::FirstOrderOutgoing);
    for index in 0..5 {
        let center = Point2::new(-0.48, (index as f64 - 2.0) * 0.20);
        let region = builder.subdomain(
            PeriodicCubicSpline::rounded(center, 0.075),
            DEFAULT_MATERIAL,
            MaterialFrame {
                origin: center,
                attachment: MaterialFrameAttachment::FollowRegion,
                ..MaterialFrame::world()
            },
        );
        builder.scene.volume_sources.push(VolumeSource {
            region,
            enabled: true,
            profile: ScalarField::formula("smoothstep(0, 1, 1 - (r / R)^2)").unwrap(),
            parameters: vec![MaterialParameter {
                name: "R".into(),
                value: 0.075,
            }],
            signal: TimeSignal::harmonic(0.0, 18.0, 3.0, -(index as f64 - 2.0) * 0.55),
        });
    }
    let mut document = builder.document();
    document.model.far_field = FarFieldSettings {
        enabled: true,
        inset: 0.08,
    };
    document.presentation.material_overlay =
        MaterialOverlay::Property(MaterialProperty::VolumeSource);
    document.presentation.material_overlay_opacity = 0.62;
    document
}

fn obstacle_array() -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.outer_boundaries =
        OuterBoundaryConditions::uniform(OuterBoundaryCondition::FirstOrderOutgoing);
    for center in [
        [-0.55, -0.48],
        [-0.15, -0.55],
        [0.30, -0.45],
        [0.58, -0.10],
        [0.25, 0.05],
        [-0.25, -0.02],
        [-0.52, 0.38],
        [0.12, 0.48],
    ] {
        builder.hole(PeriodicCubicSpline::rounded(
            Point2::new(center[0], center[1]),
            0.105,
        ));
    }
    let mut document = builder.document();
    document.model.source = source(Point2::new(-0.90, 0.0), 3.0, 20.0, 0.045);
    document
}

/// A material from a catalogue preset, with its parameters set by name.
fn preset_material(
    id: u64,
    name: &str,
    color: [u8; 3],
    preset: &str,
    row: LawPresetRow,
    values: &[(&str, f64)],
) -> Material {
    let base = Material {
        id: MaterialId(id),
        name: name.into(),
        color,
        ..Material::default_medium()
    };
    let preset = law_presets()
        .iter()
        .find(|candidate| candidate.name == preset && candidate.row == row)
        .expect("the catalogue offers this preset");
    let mut material = apply_law_preset(preset, &base).expect("a preset applies to a fresh medium");
    for (parameter, value) in values {
        material
            .parameters
            .iter_mut()
            .find(|candidate| candidate.name == *parameter)
            .expect("the preset names this parameter")
            .value = *value;
    }
    material
}

fn slab(x0: f64, x1: f64, half_height: f64) -> PeriodicCubicSpline {
    PeriodicCubicSpline::polygon(vec![
        Point2::new(x0, -half_height),
        Point2::new(x1, -half_height),
        Point2::new(x1, half_height),
        Point2::new(x0, half_height),
    ])
    .unwrap()
}

fn kerr_slab_with(chi: f64, amplitude: f64) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.outer_boundaries =
        OuterBoundaryConditions::uniform(OuterBoundaryCondition::FirstOrderOutgoing);
    builder.scene.materials.push(preset_material(
        2,
        "Kerr slab",
        [178, 102, 62],
        "Kerr medium",
        LawPresetRow::Mass,
        &[("kerr_chi", chi)],
    ));
    builder.subdomain(
        slab(-0.3, 0.3, 0.55),
        MaterialId(2),
        MaterialFrame {
            attachment: MaterialFrameAttachment::FollowRegion,
            ..MaterialFrame::world()
        },
    );
    let mut document = builder.document();
    document.model.source = source(Point2::new(-0.6, 0.0), 2.5, amplitude, 0.05);
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Receiver".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Point(Point2::new(0.55, 0.0)),
    });
    document
}

fn pumped_slab_with(depth: f64, pump_hz: f64, phase: f64) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.outer_boundaries =
        OuterBoundaryConditions::uniform(OuterBoundaryCondition::FirstOrderOutgoing);
    builder.scene.materials.push(preset_material(
        2,
        "Pumped slab",
        [120, 92, 178],
        "Parametric pump",
        LawPresetRow::Mass,
        &[
            ("depth", depth),
            ("pump_hz", pump_hz),
            ("pump_phase", phase),
        ],
    ));
    builder.subdomain(
        slab(-0.35, 0.35, 0.55),
        MaterialId(2),
        MaterialFrame {
            attachment: MaterialFrameAttachment::FollowRegion,
            ..MaterialFrame::world()
        },
    );
    let mut document = builder.document();
    document.model.source = source(Point2::new(-0.65, 0.0), 2.5, 20.0, 0.05);
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Receiver".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Point(Point2::new(0.6, 0.0)),
    });
    document
}

/// A slab carrying one preset between a source and a receiver, both of which
/// can be mirrored through the slab's centre.
fn modulated_slab(
    preset: &str,
    values: &[(&str, f64)],
    half_width: f64,
    mirrored: bool,
) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.outer_boundaries =
        OuterBoundaryConditions::uniform(OuterBoundaryCondition::FirstOrderOutgoing);
    builder.scene.materials.push(preset_material(
        2,
        "Modulated slab",
        [92, 150, 178],
        preset,
        LawPresetRow::Mass,
        values,
    ));
    builder.subdomain(
        slab(-half_width, half_width, 0.55),
        MaterialId(2),
        MaterialFrame {
            attachment: MaterialFrameAttachment::FollowRegion,
            ..MaterialFrame::world()
        },
    );
    let side = if mirrored { -1.0 } else { 1.0 };
    let mut document = builder.document();
    document.model.source = source(
        Point2::new(-side * (half_width + 0.2), 0.0),
        2.5,
        20.0,
        0.05,
    );
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Receiver".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Point(Point2::new(side * (half_width + 0.15), 0.0)),
    });
    document
}

const CRYSTAL: [(&str, f64); 4] = [
    ("depth", 0.3),
    ("pump_hz", 1.0),
    ("pump_phase", 0.0),
    ("edge", 6.0),
];
const TRAVELLING: [(&str, f64); 5] = [
    ("depth", 0.3),
    ("pump_hz", 1.0),
    ("pump_phase", 0.0),
    // 2π per unit length at 1 Hz: the modulation runs at the wave speed, so
    // each step up in frequency is phase-matched for a wave running with it.
    ("wavenumber", std::f64::consts::TAU),
    ("wave_angle", 0.0),
];

fn kerr_slab() -> TopologyDocument {
    kerr_slab_with(40.0, 60.0)
}

fn pumped_slab() -> TopologyDocument {
    pumped_slab_with(0.4, 5.0, std::f64::consts::FRAC_PI_4)
}

fn time_crystal_slab() -> TopologyDocument {
    modulated_slab("Time crystal", &CRYSTAL, 0.35, false)
}

fn travelling_slab() -> TopologyDocument {
    modulated_slab("Travelling modulation", &TRAVELLING, 0.6, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topology_catalog_is_valid_version_22_data_with_complete_semantics() {
        assert_eq!(catalog().len(), 12);
        assert_eq!(catalog()[0].name, "Starter obstacle");
        for example in catalog() {
            example.document.model.accepted.compile(1).unwrap();
            assert_eq!(
                example.document.model.draft,
                example.document.model.accepted
            );
            let bytes = crate::topology_persistence::save_compact(&example.document).unwrap();
            assert_eq!(
                crate::topology_persistence::parse_document(&bytes).unwrap(),
                example.document
            );
        }
        assert!(catalog().iter().any(|example| {
            !example.document.model.accepted.volume_sources.is_empty()
                && example.document.model.far_field.enabled
        }));
        assert!(catalog().iter().any(|example| {
            matches!(
                example.document.model.accepted.physics,
                PhysicsModel::Electromagnetic { .. }
            )
        }));
    }

    use crate::topology_editor::TopologyEditor;
    use crate::topology_runtime::TopologyRuntime;
    use std::sync::Arc;

    /// Prepares a document as the application does, at a given mesh edge.
    fn prepare(
        document: &TopologyDocument,
        edge: f64,
    ) -> Arc<crate::topology_runtime::PreparedTopology> {
        let editor = TopologyEditor::from_document(document.clone()).unwrap();
        let mut runtime = TopologyRuntime::default();
        let token = runtime
            .request(
                editor.revision,
                &editor.document,
                editor.compiled_accepted.clone(),
                MeshingOptions {
                    target_edge_length: edge,
                    ..MeshingOptions::default()
                },
                true,
            )
            .unwrap();
        loop {
            if let Some(result) = runtime.advance(1 << 16) {
                result.unwrap();
                return runtime.commit_ready(token).unwrap();
            }
        }
    }

    /// The primary field at the node nearest `point` after every step, and the
    /// largest nonlinear strength seen, stepping the CPU reference from rest.
    fn trace(
        document: &TopologyDocument,
        edge: f64,
        seconds: f64,
        point: Point2,
    ) -> (Vec<f64>, f64, f64) {
        let prepared = prepare(document, edge);
        let operator = prepared
            .canonical_temporal_operator
            .clone()
            .expect("a law-carrying document prepares a temporal operator");
        let forcing = prepared.canonical_forcing.clone();
        let dt = prepared.recommended_time_step();
        let node = operator
            .base()
            .node_points()
            .iter()
            .enumerate()
            .min_by(|a, b| (*a.1 - point).norm().total_cmp(&(*b.1 - point).norm()))
            .unwrap()
            .0;
        let mut state = CanonicalTemporalWaveState::zero(&operator, dt)
            .unwrap()
            .pinned(&operator, &forcing)
            .unwrap();
        let steps = (seconds / dt).ceil() as usize;
        let mut series = Vec::with_capacity(steps);
        let mut strongest = 0.0_f64;
        for step in 0..steps {
            state.step_with_forcing(&operator, &forcing).unwrap();
            let field = operator
                .primary_field_at(state.primary_flux(), state.time(), state.runtime())
                .unwrap();
            series.push(field[node]);
            if step % 50 == 0 {
                for strength in operator
                    .nonlinear_strength(
                        state.primary_flux(),
                        state.complementary_flux(),
                        state.time(),
                        state.runtime(),
                    )
                    .unwrap()
                {
                    strongest = strongest.max(strength.primary.max(strength.complementary));
                }
            }
        }
        (series, dt, strongest)
    }

    /// `|X(f)|` over the last `periods` whole periods of `f`.
    fn amplitude_at(series: &[f64], dt: f64, frequency: f64, periods: f64) -> f64 {
        let count = ((periods / frequency) / dt).round() as usize;
        let window = &series[series.len() - count..];
        let (mut re, mut im) = (0.0, 0.0);
        for (index, value) in window.iter().enumerate() {
            let phase = std::f64::consts::TAU * frequency * index as f64 * dt;
            re += value * phase.cos();
            im -= value * phase.sin();
        }
        2.0 * (re * re + im * im).sqrt() / count as f64
    }

    /// The Kerr gallery claim: behind the slab, the receiver hears the
    /// source's third harmonic, which a medium with the same geometry and no
    /// response does not make; and the slab's coefficient moves by tens of
    /// percent, which is what the material readout will show. Measured at a
    /// coarse mesh so the suite can afford it; the gallery's own mesh is finer.
    #[test]
    fn the_kerr_slab_generates_its_third_harmonic() {
        let receiver = Point2::new(0.55, 0.0);
        let run = |chi: f64| {
            let (series, dt, strongest) = trace(&kerr_slab_with(chi, 60.0), 0.15, 4.0, receiver);
            let ratio = amplitude_at(&series, dt, 7.5, 9.0) / amplitude_at(&series, dt, 2.5, 3.0);
            (ratio, strongest)
        };
        let (kerr, strongest) = run(40.0);
        let (linear, _) = run(0.0);
        assert!(kerr > 0.1, "third harmonic {kerr:.3e} of the fundamental");
        assert!(linear < 0.01, "a linear slab made {linear:.3e}");
        assert!(strongest > 0.2, "the slab moved only {strongest:.3}");
    }

    /// Amplitude at the source frequency behind the pumped slab, against the
    /// unpumped slab's, at a pump phase of `eighths` × π/4.
    fn pump_gain(pump_hz: f64, eighths: f64) -> f64 {
        let receiver = Point2::new(0.6, 0.0);
        let transmitted = |depth: f64, phase: f64| {
            let (series, dt, _) = trace(
                &pumped_slab_with(depth, pump_hz, phase),
                0.15,
                6.0,
                receiver,
            );
            amplitude_at(&series, dt, 2.5, 4.0)
        };
        transmitted(0.4, eighths * std::f64::consts::FRAC_PI_4) / transmitted(0.0, 0.0)
    }

    /// The pump gallery claim: pumped at twice the source's frequency the slab
    /// amplifies what it transmits, by an amount set by the pump's phase
    /// against the source. That phase sensitivity is what marks degenerate
    /// parametric amplification rather than a slab that merely changed its
    /// average impedance. The two phases are the extremes of an eight-phase
    /// scan, which sit at the same phases on a finer mesh (see the log).
    #[test]
    fn a_pump_at_twice_the_source_frequency_amplifies_by_phase() {
        let (best, worst) = (pump_gain(5.0, 1.0), pump_gain(5.0, 5.0));
        assert!(best > 1.8, "amplified to {best:.3}");
        assert!(
            best / worst > 1.5,
            "phase moved the gain {best:.3} / {worst:.3}"
        );
    }

    /// And a pump at an unrelated frequency neither amplifies nor cares for
    /// its phase.
    #[test]
    fn a_detuned_pump_neither_amplifies_nor_depends_on_phase() {
        let (one, other) = (pump_gain(3.6, 1.0), pump_gain(3.6, 5.0));
        assert!(one < 1.05 && other < 1.05, "{one:.3}, {other:.3}");
        assert!(
            (one / other - 1.0).abs() < 0.15,
            "{one:.3} against {other:.3}"
        );
    }

    /// Each sideband's amplitude at the receiver against the carrier's:
    /// `f − f_m`, `f + f_m` and `f + 3f_m` for a 2.5 Hz source and 1 Hz
    /// modulation.
    fn sidebands(document: &TopologyDocument, receiver: Point2) -> [f64; 3] {
        let (series, dt, _) = trace(document, 0.15, 8.0, receiver);
        let at = |hz: f64| amplitude_at(&series, dt, hz, 4.0 * hz);
        let carrier = at(2.5);
        [1.5, 3.5, 5.5].map(|hz| at(hz) / carrier)
    }

    /// The time-crystal gallery claim: the slab splits the wave into
    /// sidebands, and its sharp edges put several times more into the third
    /// one than a sinusoidal pump of the same depth and frequency does.
    #[test]
    fn a_time_crystal_reaches_further_sidebands_than_a_pump() {
        let receiver = Point2::new(0.5, 0.0);
        let crystal = sidebands(&time_crystal_slab(), receiver);
        let pump = sidebands(
            &modulated_slab("Parametric pump", &CRYSTAL[..3], 0.35, false),
            receiver,
        );
        assert!(crystal[0] > 0.1 && crystal[1] > 0.3, "{crystal:.3?}");
        assert!(
            crystal[2] > 2.5 * pump[2],
            "{crystal:.3?} against {pump:.3?}"
        );
    }

    /// The travelling-modulation gallery claim: running with the wave, the
    /// modulation converts it up in frequency; mirrored, so the wave runs
    /// against it, the same slab barely does.
    #[test]
    fn a_travelling_modulation_converts_only_the_wave_running_with_it() {
        let with = sidebands(&travelling_slab(), Point2::new(0.75, 0.0));
        let against = sidebands(
            &modulated_slab("Travelling modulation", &TRAVELLING, 0.6, true),
            Point2::new(-0.75, 0.0),
        );
        assert!(
            with[1] > 4.0 * against[1],
            "{with:.3?} against {against:.3?}"
        );
    }

    /// Stage 10's exit: every catalogue preset, on a fresh material in every
    /// skin, survives the file and prepares the way the application prepares
    /// it. So does a material carrying both named loss channels, and since
    /// Gate O every restoring preset, sine-Gordon beside Kerr, and van der Pol
    /// beside Klein-Gordon on the skin's primary channel.
    #[test]
    fn every_preset_round_trips_and_prepares_in_every_skin() {
        for physics in [
            PhysicsModel::Mechanical,
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            },
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            },
        ] {
            let mut materials = law_presets()
                .iter()
                .map(|preset| {
                    (
                        format!("{} ({:?})", preset.name, preset.row),
                        apply_law_preset(preset, &Material::default_medium()).unwrap(),
                    )
                })
                .collect::<Vec<_>>();
            let mut lossy = Material::default_medium();
            lossy.electric_loss = Some(LossChannel {
                base_rate: ScalarField::constant(0.2),
                law: DampingLaw::constant(),
            });
            lossy.magnetic_loss = Some(LossChannel {
                base_rate: ScalarField::constant(0.1),
                law: DampingLaw::constant(),
            });
            materials.push(("both loss channels".into(), lossy));
            for preset in restoring_presets() {
                materials.push((
                    format!("restoring {}", preset.name),
                    apply_restoring_preset(preset, &Material::default_medium()).unwrap(),
                ));
            }
            let kerr = law_presets()
                .iter()
                .find(|preset| preset.name == "Kerr medium")
                .unwrap();
            let sine_gordon = restoring_presets()
                .iter()
                .find(|preset| preset.id == "R2")
                .unwrap();
            materials.push((
                "Kerr sine-Gordon".into(),
                apply_restoring_preset(
                    sine_gordon,
                    &apply_law_preset(kerr, &Material::default_medium()).unwrap(),
                )
                .unwrap(),
            ));
            let klein_gordon = restoring_presets()
                .iter()
                .find(|preset| preset.id == "R1")
                .unwrap();
            let mut oscillator =
                apply_restoring_preset(klein_gordon, &Material::default_medium()).unwrap();
            let van_der_pol = Some(LossChannel {
                base_rate: ScalarField::constant(0.5),
                law: DampingLaw {
                    rate: RateLaw::VanDerPol {
                        threshold: ScalarField::constant(0.4),
                        amplitude_bound: ScalarField::constant(1.0e3),
                    },
                    drive: TimeDrive::None,
                },
            });
            // The primary row's own channel: E in TM, H in TE, and the
            // displacement's in Mechanical.
            if physics
                == (PhysicsModel::Electromagnetic {
                    polarization: ElectromagneticPolarization::Tm,
                })
            {
                oscillator.electric_loss = van_der_pol;
            } else {
                oscillator.magnetic_loss = van_der_pol;
            }
            materials.push(("van der Pol beside Klein-Gordon".into(), oscillator));
            for (label, material) in materials {
                let mut builder = Builder::new();
                builder.scene.physics = physics;
                builder.scene.outer_boundaries =
                    OuterBoundaryConditions::uniform(OuterBoundaryCondition::FirstOrderOutgoing);
                builder.scene.materials[0] = Material {
                    id: builder.scene.materials[0].id,
                    ..material
                };
                let mut document = builder.document();
                document.model.source = source(Point2::new(-0.5, 0.0), 2.0, 5.0, 0.06);
                let bytes = crate::topology_persistence::save_compact(&document).unwrap();
                let loaded = crate::topology_persistence::parse_document(&bytes).unwrap();
                assert_eq!(loaded, document, "{physics:?}: {label} changed in the file");
                let prepared = prepare(&loaded, 0.3);
                assert!(
                    prepared.canonical_operator.degrees_of_freedom() > 0,
                    "{physics:?}: {label}"
                );
                // A preset that writes a law prepares the operator that runs
                // it; linear and constant loss are the fixed path's.
                let carries_law = !loaded.model.accepted.materials[0].time_invariant();
                assert_eq!(
                    prepared.canonical_temporal_operator.is_some(),
                    carries_law,
                    "{physics:?}: {label}"
                );
                assert_eq!(
                    prepared
                        .canonical_temporal_operator
                        .as_ref()
                        .is_some_and(|operator| operator.has_restoring()),
                    !loaded.model.accepted.materials[0].restoring.is_none(),
                    "{physics:?}: {label}"
                );
            }
        }
    }
}
