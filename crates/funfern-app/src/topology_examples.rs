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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topology_catalog_is_valid_version_22_data_with_complete_semantics() {
        assert_eq!(catalog().len(), 8);
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
}
