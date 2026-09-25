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
                "A source boxed in black walls lights two slits; the screen and the far field show \
                 the fringes.",
                double_slit(),
            ),
            example(
                "Material lens",
                "A TM electric-field source illuminates a slower dielectric region with absorbing edges.",
                material_lens(),
            ),
            example(
                "GRIN collimator",
                "A quarter-pitch graded-index rod turns a point source on one face into a \
                 collimated beam leaving the other.",
                grin_rod(),
            ),
            example(
                "Anisotropic crystal",
                "A rotated directional inclusion turns circular wavefronts into ellipses.",
                anisotropic_crystal(),
            ),
            example(
                "Luneburg lens",
                "A TE plane wave arriving at 30° focuses on the far rim of a radial-index lens, \
                 wherever it comes from.",
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
            example(
                "Plasma mirror",
                "A plane wave climbs a plasma whose cutoff rises along the channel, stands in \
                 front of the point where the cutoff meets its frequency, and never passes it.",
                plasma_mirror(),
            ),
            example(
                "Josephson line",
                "A junction line held at a constant voltage on one end sheds one fluxon per \
                 turn of its phase; each runs down the line as a kink in the integrated field.",
                josephson_line(),
            ),
            example(
                "Symmetry breaking",
                "A medium resting on the top of a double well is tipped by faint frozen noise: \
                 it falls into both wells in patches, then the walls between them move until \
                 one well holds everything. Shown in the integrated field.",
                symmetry_breaking(),
            ),
            example(
                "Pinned domain wall",
                "A double-well medium falls into opposite wells either side of a wall that forms \
                 off-centre. Two round bumps narrow the channel: the wall slides to their waist \
                 and stays, and dragging the bumps drags it along.",
                pinned_domain_wall(),
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
    document.presentation.advanced_materials = advanced_materials(&document.model.draft);
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

/// The materials the simple view cannot name as one medium: those open in the
/// Advanced view, where their laws are shown as authored.
fn advanced_materials(scene: &TopologyScene) -> crate::document::AdvancedMaterials {
    scene
        .materials
        .iter()
        .filter(|material| identify_medium_preset(material, scene.physics).is_none())
        .map(|material| material.id)
        .collect()
}

struct Builder {
    scene: TopologyScene,
    next_curve: u64,
    next_span: u64,
    next_region: u64,
    next_vertex: u64,
}

impl Builder {
    fn new() -> Self {
        Self {
            scene: TopologyScene::default(),
            next_curve: 1,
            next_span: 1,
            next_region: 2,
            next_vertex: 1,
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
        let behaviors = vec![SpanBehavior::REFLECTING; spline.intervals().len()];
        self.open_curve(spline, &behaviors)
    }

    /// An open curve whose spans carry the given behaviours, in order.
    fn open_curve(&mut self, spline: OpenCubicSpline, behaviors: &[SpanBehavior]) -> CurveId {
        assert_eq!(behaviors.len(), spline.intervals().len());
        let curve = CurveId(self.next_curve);
        self.next_curve += 1;
        let spans = behaviors
            .iter()
            .map(|behavior| {
                let span = CurveSpan {
                    id: CurveSpanId(self.next_span),
                    behavior: *behavior,
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

    fn outer_vertex(&mut self, side: OuterSide, fraction: f64) -> TopologyVertexId {
        let id = TopologyVertexId(self.next_vertex);
        self.next_vertex += 1;
        self.scene.geometry.vertices.push(TopologyVertex {
            id,
            location: TopologyVertexLocation::Outer { side, fraction },
        });
        id
    }

    /// A transmitting line across the domain at `x`, from the floor to the
    /// ceiling and attached to both.
    fn divider(&mut self, x: f64) -> (CurveId, CurveSpanId) {
        let domain = self.scene.geometry.domain;
        let bottom = self.outer_vertex(OuterSide::Bottom, (x - domain.min_x) / domain.width());
        let top = self.outer_vertex(OuterSide::Top, (domain.max_x - x) / domain.width());
        let span = CurveSpanId(self.next_span);
        let curve = self.open_curve(
            OpenCubicSpline::polyline(vec![
                Point2::new(x, domain.min_y),
                Point2::new(x, domain.max_y),
            ])
            .unwrap(),
            &[SpanBehavior::Transmitting],
        );
        let authored = self
            .scene
            .geometry
            .curves
            .iter_mut()
            .find(|candidate| candidate.id == curve)
            .unwrap();
        authored.nodes[0].vertex = Some(bottom);
        authored.nodes[1].vertex = Some(top);
        (curve, span)
    }

    /// A reflecting baffle through `points`, from −x to +x, welded to the
    /// floor or the ceiling at both ends; the pocket between it and the wall
    /// is left out of the domain. The ends are moved onto the wall vertices
    /// exactly, as the topology resolves them.
    fn wall_bump(&mut self, points: &[Point2]) {
        let domain = self.scene.geometry.domain;
        let ceiling = points[points.len() / 2].y > 0.0;
        let side = if ceiling {
            OuterSide::Top
        } else {
            OuterSide::Bottom
        };
        let fraction = |x: f64| {
            if ceiling {
                (domain.max_x - x) / domain.width()
            } else {
                (x - domain.min_x) / domain.width()
            }
        };
        let resolved = |x: f64| {
            if ceiling {
                Point2::new(domain.max_x - domain.width() * fraction(x), domain.max_y)
            } else {
                Point2::new(domain.min_x + domain.width() * fraction(x), domain.min_y)
            }
        };
        let mut points = points.to_vec();
        let last = points.len() - 1;
        let (left, right) = (points[0].x, points[last].x);
        let start = self.outer_vertex(side, fraction(left));
        let end = self.outer_vertex(side, fraction(right));
        points[0] = resolved(left);
        points[last] = resolved(right);
        let spline = OpenCubicSpline::polyline(points).unwrap();
        let parameter = spline.span_bounds(0).map(|[a, b]| (a + b) * 0.5).unwrap();
        let first_span = CurveSpanId(self.next_span);
        let curve = self.baffle(spline);
        let authored = self
            .scene
            .geometry
            .curves
            .iter_mut()
            .find(|candidate| candidate.id == curve)
            .unwrap();
        let last = authored.nodes.len() - 1;
        authored.nodes[0].vertex = Some(start);
        authored.nodes[last].vertex = Some(end);
        // Running from −x to +x, the ceiling's pocket is on the curve's left
        // and the floor's on its right.
        self.scene.face_assignments.push(AuthoredFaceAssignment {
            anchor: FaceAnchor::Curve {
                curve,
                span: first_span,
                side: if ceiling {
                    CurveTraceSide::Left
                } else {
                    CurveTraceSide::Right
                },
                parameter,
            },
            region: None,
        });
    }

    /// The band between two dividers, from wall to wall, as a region of
    /// `material`.
    fn strip(&mut self, x0: f64, x1: f64, material: MaterialId) -> RegionId {
        let (curve, span) = self.divider(x0);
        self.divider(x1);
        // The background's own anchor sits on the floor at x = 0, so it
        // names the face right of the strip; the face left of it is a
        // region of its own, of the same material.
        assert!(x1 < 0.0, "a strip left of the centre");
        let behind = RegionId(self.next_region);
        self.next_region += 1;
        let background = self.scene.regions[0].material;
        self.scene.regions.push(Region {
            id: behind,
            material: background,
            frame: MaterialFrame::world(),
        });
        self.scene.face_assignments.push(AuthoredFaceAssignment {
            anchor: FaceAnchor::Outer {
                side: OuterSide::Left,
                fraction: 0.5,
            },
            region: Some(behind),
        });
        let region = RegionId(self.next_region);
        self.next_region += 1;
        self.scene.regions.push(Region {
            id: region,
            material,
            frame: MaterialFrame::world(),
        });
        // Running upwards, the divider's right is towards +x.
        self.scene.face_assignments.push(AuthoredFaceAssignment {
            anchor: FaceAnchor::Curve {
                curve,
                span,
                side: CurveTraceSide::Right,
                parameter: 0.5,
            },
            region: Some(region),
        });
        region
    }

    /// A plane-wave launcher: a thin strip at `x` across the whole height,
    /// radiating both ways, of the background material so it is transparent.
    fn launcher(&mut self, x: f64, frequency: f64, amplitude: f64) -> RegionId {
        let background = self.scene.regions[0].material;
        let region = self.strip(x - 0.03, x + 0.03, background);
        self.scene.volume_sources.push(VolumeSource {
            region,
            enabled: true,
            profile: ScalarField::constant(1.0),
            parameters: vec![],
            signal: TimeSignal::harmonic(0.0, amplitude, frequency, 0.0),
        });
        region
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

/// Outgoing on every side but the floor.
fn reflecting_floor() -> OuterBoundaryConditions {
    let mut boundaries = OuterBoundaryConditions::default();
    boundaries.sides[OuterSide::Bottom.index()] = OuterBoundaryCondition::Reflecting;
    boundaries
}

fn starter_obstacle() -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.outer_boundaries = reflecting_floor();
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

/// A wall that absorbs on its left face and reflects on its right.
const ABSORBING_LEFT: SpanBehavior = SpanBehavior::Separated {
    left: FaceBoundaryCondition::SecondOrderOutgoing,
    right: FaceBoundaryCondition::Reflecting,
    coupling: InternalBoundaryCoupling::Independent,
};

/// The source sits in a box that is black inside and reflecting outside, so
/// nothing leaves it except through the two slits in its right side, and the
/// far field sees the two-slit pattern alone.
fn double_slit() -> TopologyDocument {
    double_slit_with(0.11, -0.85, -0.78)
}

/// Slits of half-width `half_width` centred at ±0.25 in the box's right side,
/// the box's back wall at `back` and the source at `source_x`.
fn double_slit_with(half_width: f64, back: f64, source_x: f64) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.outer_boundaries =
        OuterBoundaryConditions::uniform(OuterBoundaryCondition::SecondOrderOutgoing);
    // Anticlockwise round the box, so its inside is on the curve's left.
    builder.open_curve(
        OpenCubicSpline::polyline(vec![
            Point2::new(0.0, 0.25 + half_width),
            Point2::new(0.0, 0.6),
            Point2::new(back, 0.6),
            Point2::new(back, -0.6),
            Point2::new(0.0, -0.6),
            Point2::new(0.0, -0.25 - half_width),
        ])
        .unwrap(),
        &[
            SpanBehavior::REFLECTING,
            ABSORBING_LEFT,
            ABSORBING_LEFT,
            ABSORBING_LEFT,
            SpanBehavior::REFLECTING,
        ],
    );
    builder.baffle(straight_baffle(-0.25 + half_width, 0.25 - half_width));
    let mut document = builder.document();
    document.model.source = source(Point2::new(source_x, 0.0), 3.0, 20.0, 0.05);
    document.model.far_field = FarFieldSettings {
        enabled: true,
        inset: 0.08,
    };
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Screen".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Segment {
            start: Point2::new(0.85, -0.85),
            end: Point2::new(0.85, 0.85),
            preset: ProbeSamplingPreset::High,
        },
    });
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

/// Quarter-pitch GRIN collimator: `n = 1 + dn (1 − (y/H)²)` has paraxial
/// pitch `2πH √((1 + dn)/(2 dn))`, 2.18 for these values, so a rod a quarter
/// of that long turns a point on its entrance face into a plane wave at its
/// exit face.
const GRIN_H: f64 = 0.3;
const GRIN_DN: f64 = 0.6;
const GRIN_ENTRANCE: f64 = -0.75;

fn grin_quarter_pitch() -> f64 {
    0.25 * std::f64::consts::TAU * GRIN_H * ((1.0 + GRIN_DN) / (2.0 * GRIN_DN)).sqrt()
}

fn grin_rod() -> TopologyDocument {
    grin_rod_with(GRIN_DN)
}

fn grin_rod_with(dn: f64) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.materials.push(Material {
        id: MaterialId(2),
        name: "GRIN profile".into(),
        mass_density: ScalarField::formula("1 + dn * max(0, 1 - (y / H)^2)").unwrap(),
        stiffness: ScalarField::formula("1 / (1 + dn * max(0, 1 - (y / H)^2))").unwrap(),
        damping: ScalarField::constant(0.0),
        axis_ratio: ScalarField::constant(1.0),
        parameters: vec![
            MaterialParameter {
                name: "H".into(),
                value: GRIN_H,
            },
            MaterialParameter {
                name: "dn".into(),
                value: dn,
            },
        ],
        color: [46, 120, 139],
        ..Material::default_medium()
    });
    let exit = GRIN_ENTRANCE + grin_quarter_pitch();
    let region = builder.subdomain(
        slab(GRIN_ENTRANCE, exit, GRIN_H),
        MaterialId(2),
        MaterialFrame {
            attachment: MaterialFrameAttachment::FollowRegion,
            ..MaterialFrame::world()
        },
    );
    let mut document = builder.document();
    document.model.source = PointSource {
        region,
        ..source(Point2::new(GRIN_ENTRANCE + 0.01, 0.0), 4.0, 16.0, 0.04)
    };
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Beam profile".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Segment {
            start: Point2::new(0.75, -0.9),
            end: Point2::new(0.75, 0.9),
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

/// The Luneburg lens's centre and radius, and the direction its light comes
/// from, 30° above the x axis.
const LUNEBURG_CENTRE: Point2 = Point2 { x: 0.08, y: 0.0 };
const LUNEBURG_RADIUS: f64 = 0.43;
const LUNEBURG_DEGREES: f64 = 30.0;

fn luneburg_direction() -> Point2 {
    let angle = LUNEBURG_DEGREES.to_radians();
    Point2::new(angle.cos(), angle.sin())
}

/// Where the lens focuses its plane wave: the rim point it runs towards.
fn luneburg_focus() -> Point2 {
    LUNEBURG_CENTRE + luneburg_direction() * LUNEBURG_RADIUS
}

fn luneburg_lens() -> TopologyDocument {
    luneburg_lens_with(true)
}

/// A plane wave from a tilted radiator: an open line whose face towards the
/// lens carries a prescribed flux and whose back face absorbs.
fn luneburg_lens_with(lens: bool) -> TopologyDocument {
    let mut builder = Builder::new();
    let (center, radius) = (LUNEBURG_CENTRE, LUNEBURG_RADIUS);
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Te,
    };
    let direction = luneburg_direction();
    let across = Point2::new(-direction.y, direction.x);
    let middle = center - direction * 0.85;
    // Running from `+across` to `−across`, the curve's left is `direction`.
    builder.open_curve(
        OpenCubicSpline::polyline(vec![middle + across * 0.55, middle - across * 0.55]).unwrap(),
        &[SpanBehavior::Separated {
            left: FaceBoundaryCondition::Neumann {
                signal: TimeSignal::harmonic(0.0, 15.0, 3.5, 0.0),
            },
            right: FaceBoundaryCondition::SecondOrderOutgoing,
            coupling: InternalBoundaryCoupling::Independent,
        }],
    );
    builder.scene.materials.push(Material {
        id: MaterialId(2),
        name: "Luneburg profile".into(),
        mass_density: ScalarField::formula(if lens { "max(2 - (r / R)^2, 1)" } else { "1" })
            .unwrap(),
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
            center: luneburg_focus() - direction * 0.025,
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

/// Top and bottom reflect, so a wave uniform in y stays uniform; the ends
/// are outgoing.
fn channel() -> OuterBoundaryConditions {
    let mut boundaries = OuterBoundaryConditions::default();
    boundaries.sides[OuterSide::Bottom.index()] = OuterBoundaryCondition::Reflecting;
    boundaries.sides[OuterSide::Top.index()] = OuterBoundaryCondition::Reflecting;
    boundaries
}

const PLASMA_HZ: f64 = 3.0;

fn plasma_mirror() -> TopologyDocument {
    plasma_mirror_with(
        "W * smoothstep(0, 1, (x - x0) / L)",
        std::f64::consts::TAU * 4.0,
    )
}

/// A TM channel of Klein-Gordon plasma whose cutoff is `omega0`, a formula in
/// `x` over `W`, `x0 = −0.2` and `L = 0.8`, lit by a plane wave at 3 Hz.
fn plasma_mirror_with(omega0: &str, top: f64) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.outer_boundaries = channel();
    let background = &mut builder.scene.materials[0];
    background.name = "Plasma".into();
    background.restoring = RestoringLaw::KleinGordon {
        omega0: ScalarField::formula(omega0).unwrap(),
    };
    background.parameters = [("W", top), ("x0", -0.2), ("L", 0.8)]
        .into_iter()
        .map(|(name, value)| MaterialParameter {
            name: name.into(),
            value,
        })
        .collect();
    builder.launcher(-0.82, PLASMA_HZ, 40.0);
    let mut document = builder.document();
    document.model.source.enabled = false;
    document
}

/// A Josephson transmission line: sine-Gordon in the integrated field, which
/// in TM is the phase difference across the junction, with its left end held
/// at a constant voltage. The phase there winds at that rate, and every turn
/// of 2π leaves the end as a fluxon, a kink in `r` and a voltage pulse in
/// `E_z`, which runs down the line into the matched right-hand wall.
const JOSEPHSON_OMEGA0: f64 = 12.0;
const JOSEPHSON_BIAS: f64 = 3.0;

fn josephson_line() -> TopologyDocument {
    josephson_line_with("R2", JOSEPHSON_BIAS)
}

/// The line with restoring preset `restoring` (R1 Klein-Gordon, R2
/// sine-Gordon) at `omega0 = 12`, biased at `bias` volts on its left end.
fn josephson_line_with(restoring: &str, bias: f64) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    let mut boundaries = channel();
    boundaries.sides[OuterSide::Left.index()] = OuterBoundaryCondition::Dirichlet {
        signal: TimeSignal::harmonic(bias, 0.0, 0.0, 0.0),
    };
    builder.scene.outer_boundaries = boundaries;
    let preset = restoring_presets()
        .iter()
        .find(|preset| preset.id == restoring)
        .expect("a restoring preset");
    let mut line = apply_restoring_preset(preset, &builder.scene.materials[0])
        .expect("a restoring preset applies to the background");
    line.name = "Junction".into();
    line.parameters
        .iter_mut()
        .find(|parameter| parameter.name == "omega0")
        .expect("the preset names its cutoff")
        .value = JOSEPHSON_OMEGA0;
    builder.scene.materials[0] = line;
    let mut document = builder.document();
    document.model.source.enabled = false;
    document.presentation.integrated_field = true;
    document
}

/// φ⁴ at rest sits on the unstable top of its double well. A weak source
/// tips it, the medium falls into the wells at `r = ±1` in patches, and the
/// walls between them straighten and meet under a constant loss.
const PHI4_LAMBDA: f64 = 60.0;

/// A fixed pattern of both signs at wavenumbers inside φ⁴'s unstable band
/// (`k < √λ = 7.7`), standing in for the noise a real quench starts from.
const FROZEN_NOISE: &str =
    "sin(6.1*x + 2.3*y) + sin(5.7*y - 3.4*x + 1.3) + sin(4.4*x - 4.1*y + 2.9)";

fn symmetry_breaking() -> TopologyDocument {
    symmetry_breaking_with(PHI4_LAMBDA, 1.0)
}

fn symmetry_breaking_with(lambda: f64, amplitude: f64) -> TopologyDocument {
    phi4_document(phi4_builder(lambda, FROZEN_NOISE, amplitude))
}

/// The "φ⁴ double well" medium over the whole background, `λ` and a bound of
/// 3, with a constant electric loss of 1/s so it settles, seeded by a faint
/// 3 Hz volume source over the background whose profile is `seed`.
fn phi4_builder(lambda: f64, seed: &str, amplitude: f64) -> Builder {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    let preset = restoring_presets()
        .iter()
        .find(|preset| preset.id == "R3")
        .expect("the φ⁴ preset");
    let mut medium = apply_restoring_preset(preset, &builder.scene.materials[0])
        .expect("a restoring preset applies to the background");
    medium.name = "Double well".into();
    for (name, value) in [("lambda", lambda), ("phi4_bound", 3.0)] {
        medium
            .parameters
            .iter_mut()
            .find(|parameter| parameter.name == name)
            .expect("the preset names its parameter")
            .value = value;
    }
    medium.electric_loss = Some(LossChannel {
        base_rate: ScalarField::constant(1.0),
        law: DampingLaw::constant(),
    });
    builder.scene.materials[0] = medium;
    builder.scene.volume_sources.push(VolumeSource {
        region: BACKGROUND_REGION,
        enabled: true,
        profile: ScalarField::formula(seed).unwrap(),
        parameters: vec![],
        signal: TimeSignal::harmonic(0.0, amplitude, 3.0, 0.0),
    });
    builder
}

/// A φ⁴ scene opens on the integrated field, where its wells and walls are.
fn phi4_document(builder: Builder) -> TopologyDocument {
    let mut document = builder.document();
    document.model.source.enabled = false;
    document.presentation.integrated_field = true;
    document
}

/// A seed odd about x = 0.3, so the medium falls into opposite wells either
/// side of there and a wall forms 0.3 away from the waist.
const PINNING_SEED: &str = "sin(1.5 * (x - 0.3))";
/// Half the waist's width: a quarter of the channel's height.
const WAIST: f64 = 0.25;
/// The bumps are arcs of this radius.
const BUMP_RADIUS: f64 = 0.6;

fn pinned_domain_wall() -> TopologyDocument {
    pinned_domain_wall_with(PINNING_SEED, true)
}

/// A φ⁴ wall pinned at a waist. Two round bumps, baffles welded to the floor
/// and the ceiling with the pockets behind them left out, narrow the channel
/// to a quarter of its height at x = 0. A wall's energy is its tension times
/// its length, and anywhere over the bumps a wall is shorter the nearer it
/// is to the waist, so it is pushed there; moving the bumps drags it along.
/// Without them a straight wall costs the same anywhere and stays roughly
/// where it formed.
fn pinned_domain_wall_with(seed: &str, bumps: bool) -> TopologyDocument {
    let mut builder = phi4_builder(PHI4_LAMBDA, seed, 1.0);
    builder.scene.outer_boundaries = channel();
    // The background's own anchor, on the floor at x = 0, would sit in the
    // floor's pocket.
    builder.scene.face_assignments[0].anchor = FaceAnchor::Outer {
        side: OuterSide::Left,
        fraction: 0.5,
    };
    if bumps {
        for sign in [1.0, -1.0] {
            let centre = sign * (WAIST + BUMP_RADIUS);
            let reach = (BUMP_RADIUS * BUMP_RADIUS - (1.0 - WAIST - BUMP_RADIUS).powi(2)).sqrt();
            let half_angle = (reach / BUMP_RADIUS).asin();
            let points = (0..=12)
                .map(|index| {
                    let angle = -half_angle + 2.0 * half_angle * index as f64 / 12.0;
                    Point2::new(
                        BUMP_RADIUS * angle.sin(),
                        centre - sign * BUMP_RADIUS * angle.cos(),
                    )
                })
                .collect::<Vec<_>>();
            builder.wall_bump(&points);
        }
    }
    phi4_document(builder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topology_catalog_is_valid_version_22_data_with_complete_semantics() {
        assert_eq!(catalog().len(), 16);
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

    /// Every gallery material the simple view cannot name opens in Advanced,
    /// and no other does. The plasma's cutoff is a formula in x, which no
    /// medium writes.
    #[test]
    fn a_gallery_material_opens_in_the_view_that_can_show_it() {
        for example in catalog() {
            let scene = &example.document.model.draft;
            for material in &scene.materials {
                assert_eq!(
                    example
                        .document
                        .presentation
                        .advanced_materials
                        .contains(material.id),
                    identify_medium_preset(material, scene.physics).is_none(),
                    "{}: {}",
                    example.name,
                    material.name
                );
            }
        }
        let plasma = catalog()
            .iter()
            .find(|example| example.name == "Plasma mirror")
            .unwrap();
        assert!(
            plasma
                .document
                .presentation
                .advanced_materials
                .contains(DEFAULT_MATERIAL)
        );
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

    /// The steady complex amplitude at one frequency on every node, from the
    /// last whole periods of a run from rest.
    struct Harmonic {
        prepared: Arc<crate::topology_runtime::PreparedTopology>,
        nodes: Vec<Point2>,
        amplitude: Vec<(f64, f64)>,
        wavenumber: f64,
    }

    impl Harmonic {
        fn run(
            document: &TopologyDocument,
            edge: f64,
            seconds: f64,
            frequency: f64,
            periods: f64,
        ) -> Self {
            let prepared = prepare(document, edge);
            let forcing = prepared.canonical_forcing.clone();
            let dt = prepared.recommended_time_step();
            let steps = (seconds / dt).ceil() as usize;
            let window = ((periods / frequency) / dt).round() as usize;
            let omega = std::f64::consts::TAU * frequency;
            let mut amplitude = Vec::new();
            let mut accumulate = |step: usize, time: f64, field: &[f64]| {
                if step + window < steps {
                    return;
                }
                amplitude.resize(field.len(), (0.0, 0.0));
                let (sin, cos) = (omega * time).sin_cos();
                let scale = 2.0 / window as f64;
                for (sum, value) in amplitude.iter_mut().zip(field) {
                    sum.0 += scale * value * cos;
                    sum.1 -= scale * value * sin;
                }
            };
            let nodes = match prepared.canonical_temporal_operator.clone() {
                Some(operator) => {
                    let mut state = CanonicalTemporalWaveState::zero(&operator, dt)
                        .unwrap()
                        .pinned(&operator, &forcing)
                        .unwrap();
                    for step in 0..steps {
                        state.step_with_forcing(&operator, &forcing).unwrap();
                        let field = operator
                            .primary_field_at(state.primary_flux(), state.time(), state.runtime())
                            .unwrap();
                        accumulate(step, state.time(), &field);
                    }
                    operator.base().node_points().to_vec()
                }
                None => {
                    let operator = prepared.canonical_operator.clone();
                    let mut state = CanonicalWaveState::zero(&operator, dt).unwrap();
                    for step in 0..steps {
                        state.step_with_forcing(&operator, &forcing).unwrap();
                        accumulate(step, state.time(), &state.primary_field(&operator).unwrap());
                    }
                    operator.node_points().to_vec()
                }
            };
            // The exterior's speed: the far field needs it, and nothing else
            // here does.
            let wavenumber = omega
                / prepared
                    .far_field
                    .as_ref()
                    .and_then(|far| far.as_ref().ok())
                    .map_or(1.0, |far| far.wave_speed);
            Self {
                prepared,
                nodes,
                amplitude,
                wavenumber,
            }
        }

        /// `U` at the node nearest `point`.
        fn complex(&self, point: Point2) -> (f64, f64) {
            let node = self
                .nodes
                .iter()
                .enumerate()
                .min_by(|a, b| (*a.1 - point).norm().total_cmp(&(*b.1 - point).norm()))
                .unwrap()
                .0;
            self.amplitude[node]
        }

        /// `|U|` at the node nearest `point`.
        fn at(&self, point: Point2) -> f64 {
            let (re, im) = self.complex(point);
            re.hypot(im)
        }

        /// `|U|` at `count` evenly spaced points from `start` to `end`.
        fn along(&self, start: Point2, end: Point2, count: usize) -> Vec<f64> {
            (0..count)
                .map(|index| self.at(start + (end - start) * (index as f64 / (count - 1) as f64)))
                .collect()
        }

        /// The far-field magnitude towards `degrees`, projected from the
        /// document's own Huygens contour by the frequency-domain Kirchhoff
        /// integral `∮ (ik n·ŝ U − ∂ₙU) e^{ik ŝ·y} ds`.
        fn far_field(&self, degrees: f64) -> f64 {
            let stencil = self
                .prepared
                .far_field
                .as_ref()
                .expect("the document enables its far field")
                .as_ref()
                .expect("the far field compiles");
            assert_eq!(
                self.nodes.len(),
                self.prepared.operator.degrees_of_freedom()
            );
            let k = self.wavenumber;
            let direction = Point2::new(degrees.to_radians().cos(), degrees.to_radians().sin());
            let (mut re, mut im) = (0.0, 0.0);
            for (point, position, normal) in &stencil.samples {
                let (mut value, mut gradient) =
                    ((0.0, 0.0), (Point2::default(), Point2::default()));
                for ((node, weight), slope) in point
                    .nodes
                    .iter()
                    .zip(point.value_weights)
                    .zip(point.gradient_weights)
                {
                    let (a, b) = self.amplitude[*node as usize];
                    value.0 += weight * a;
                    value.1 += weight * b;
                    gradient.0 = gradient.0 + slope * a;
                    gradient.1 = gradient.1 + slope * b;
                }
                let normal_derivative = (normal.dot(gradient.0), normal.dot(gradient.1));
                let obliquity = k * normal.dot(direction);
                // ik(n·ŝ)U − ∂ₙU
                let term = (
                    -obliquity * value.1 - normal_derivative.0,
                    obliquity * value.0 - normal_derivative.1,
                );
                let (sin, cos) = (k * direction.dot(*position)).sin_cos();
                re += term.0 * cos - term.1 * sin;
                im += term.0 * sin + term.1 * cos;
            }
            re.hypot(im)
        }
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

    /// The double-slit gallery claim. Slits 0.5 apart at wavelength 1/3 put
    /// the far field's zeros at sin θ = 1/3 and its side lobes at
    /// sin θ = 2/3: 19.5° and 41.8°. The screen 0.85 behind the slits is in
    /// the near zone, so its fringes sit a little inside those angles.
    #[test]
    fn the_double_slit_draws_its_fringes_on_the_screen_and_in_the_far_field() {
        let scene = Harmonic::run(&double_slit(), 0.08, 6.0, 3.0, 3.0);
        let centre = scene.far_field(0.0);
        for side in [1.0, -1.0] {
            let zero = scene.far_field(side * 19.47) / centre;
            let lobe = scene.far_field(side * 41.81) / centre;
            assert!(
                zero < 0.15,
                "{side}: the zero holds {zero:.3} of the centre"
            );
            assert!(lobe > 0.5, "{side}: the side lobe holds {lobe:.3}");
        }
        let behind = scene.far_field(180.0) / centre;
        assert!(behind < 0.35, "{behind:.3} went back round the box");
        let screen = scene.along(Point2::new(0.85, 0.0), Point2::new(0.85, 0.85), 18);
        let dark = screen[5..9].iter().cloned().fold(f64::MAX, f64::min) / screen[0];
        let bright = screen[12..16].iter().cloned().fold(0.0, f64::max) / screen[0];
        assert!(
            dark < 0.4,
            "the first dark fringe holds {dark:.3} of the centre"
        );
        assert!(bright > 0.6, "the first bright fringe holds {bright:.3}");
    }

    /// A beam's cut across the domain at `x`: the share of its power within
    /// `|y| < 0.3`, and its half-amplitude width about the axis.
    fn beam(scene: &Harmonic, x: f64) -> (f64, f64) {
        let line = scene.along(Point2::new(x, -0.9), Point2::new(x, 0.9), 37);
        let total: f64 = line.iter().map(|value| value * value).sum();
        let inside: f64 = line[12..=24].iter().map(|value| value * value).sum();
        let half = 0.5 * line[18];
        let reach = |step: isize| {
            (1..=18)
                .take_while(|offset| line[(18 + step * offset) as usize] >= half)
                .count() as f64
        };
        (inside / total, 0.05 * (reach(1) + reach(-1) + 1.0))
    }

    /// The collimator gallery claim: behind the rod the beam keeps its width
    /// and most of its power within the aperture, where the bare source's
    /// spreads across the domain.
    #[test]
    fn the_grin_collimator_sends_out_a_beam_that_does_not_spread() {
        let rod = Harmonic::run(&grin_rod(), 0.08, 6.0, 4.0, 3.0);
        let bare = Harmonic::run(&grin_rod_with(0.0), 0.08, 6.0, 4.0, 3.0);
        let (near, far) = (beam(&rod, 0.2), beam(&rod, 0.75));
        let spread = beam(&bare, 0.75);
        assert!(
            far.0 > 0.7,
            "the rod's beam keeps {:.2} in the aperture",
            far.0
        );
        assert!(spread.0 < 0.5, "the bare source keeps {:.2}", spread.0);
        assert!(
            (far.1 / near.1 - 1.0).abs() < 0.25,
            "the beam went from {:.2} to {:.2} wide",
            near.1,
            far.1
        );
    }

    /// The Luneburg gallery claim: a plane wave arriving at 30° focuses on
    /// the rim point it runs towards, not where normal incidence focused, and
    /// into a spot the same wave without the lens does not make.
    #[test]
    fn the_luneburg_lens_focuses_an_angled_wave_on_its_far_rim() {
        let lens = Harmonic::run(&luneburg_lens(), 0.08, 6.0, 3.5, 3.0);
        let bare = Harmonic::run(&luneburg_lens_with(false), 0.08, 6.0, 3.5, 3.0);
        let focus = luneburg_focus();
        let normal_focus = LUNEBURG_CENTRE + Point2::new(LUNEBURG_RADIUS, 0.0);
        let peak = lens.at(focus);
        let elsewhere = lens.at(normal_focus);
        let unfocused = bare.at(focus);
        assert!(
            peak > 2.5 * elsewhere,
            "{peak:.3} at the focus, {elsewhere:.3} at 0°"
        );
        assert!(
            peak > 1.5 * unfocused,
            "{peak:.3} with the lens, {unfocused:.3} without"
        );
        let direction = luneburg_direction();
        let across = Point2::new(-direction.y, direction.x);
        for side in [1.0, -1.0] {
            let flank = lens.at(focus + across * (0.15 * side));
            assert!(
                flank < 0.5 * peak,
                "the spot's flank holds {flank:.3} of {peak:.3}"
            );
        }
    }

    /// The reflection `|B/A|` of `U = A e^{−ikx} + B e^{ikx}` fitted by least
    /// squares on the axis from `x0` to `x1`, at the `k` that fits best.
    fn reflection(scene: &Harmonic, x0: f64, x1: f64) -> (f64, f64) {
        let samples = (0..=80)
            .map(|index| {
                let x = x0 + (x1 - x0) * index as f64 / 80.0;
                (x, scene.complex(Point2::new(x, 0.0)))
            })
            .collect::<Vec<_>>();
        let fit = |k: f64| {
            // Normal equations for two complex unknowns, written out.
            type C = (f64, f64);
            let mul = |a: C, b: C| (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0);
            let conj = |a: C| (a.0, -a.1);
            let add = |a: C, b: C| (a.0 + b.0, a.1 + b.1);
            let (mut g12, mut r1, mut r2) = ((0.0, 0.0), (0.0, 0.0), (0.0, 0.0));
            let n = samples.len() as f64;
            for (x, u) in &samples {
                let forward = ((k * x).cos(), -(k * x).sin());
                let backward = conj(forward);
                g12 = add(g12, mul(conj(forward), backward));
                r1 = add(r1, mul(conj(forward), *u));
                r2 = add(r2, mul(conj(backward), *u));
            }
            // [n g12; conj(g12) n] [a; b] = [r1; r2]
            let det = n * n - (g12.0 * g12.0 + g12.1 * g12.1);
            let a = mul(
                (1.0 / det, 0.0),
                add(mul((n, 0.0), r1), mul((-g12.0, -g12.1), r2)),
            );
            let b = mul(
                (1.0 / det, 0.0),
                add(mul((n, 0.0), r2), mul((-g12.0, g12.1), r1)),
            );
            let residual: f64 = samples
                .iter()
                .map(|(x, u)| {
                    let forward = ((k * x).cos(), -(k * x).sin());
                    let model = add(mul(a, forward), mul(b, conj(forward)));
                    (u.0 - model.0).powi(2) + (u.1 - model.1).powi(2)
                })
                .sum();
            (residual, b.0.hypot(b.1) / a.0.hypot(a.1))
        };
        (0..=400)
            .map(|index| 5.0 + 15.0 * index as f64 / 400.0)
            .map(|k| (k, fit(k)))
            .min_by(|a, b| a.1.0.total_cmp(&b.1.0))
            .map(|(k, (_, r))| (k, r))
            .unwrap()
    }

    /// The plasma-mirror gallery claim. Where the cutoff reaches the drive
    /// the wave turns back whole: in front of the turning point the field
    /// stands with nodes near zero, and beyond it nothing arrives, where the
    /// same channel without the plasma carries one travelling wave end to end.
    /// A flat plasma below the drive measures the outgoing wall, which is
    /// tuned for `k = ω/c` and so reflects `(ω − ck)/(ω + ck)` of a wave with
    /// `ck = √(ω² − ω₀²)`: 0.288 for 3 Hz over a 2.5 Hz cutoff.
    #[test]
    fn a_plasma_ramp_turns_the_wave_back_and_the_wall_reflects_as_derived() {
        let ramp = Harmonic::run(&plasma_mirror(), 0.08, 10.0, PLASMA_HZ, 3.0);
        let vacuum = Harmonic::run(
            &plasma_mirror_with("W * smoothstep(0, 1, (x - x0) / L)", 0.0),
            0.08,
            10.0,
            PLASMA_HZ,
            3.0,
        );
        let front = |scene: &Harmonic| {
            let line = scene.along(Point2::new(-0.75, 0.0), Point2::new(-0.3, 0.0), 46);
            let peak = line.iter().cloned().fold(0.0, f64::max);
            let node = line.iter().cloned().fold(f64::MAX, f64::min);
            (peak, node / peak, scene.at(Point2::new(0.8, 0.0)) / peak)
        };
        let (_, node, beyond) = front(&ramp);
        assert!(
            node < 0.15,
            "the plasma's standing wave keeps {node:.3} at a node"
        );
        assert!(
            beyond < 0.02,
            "{beyond:.3} of the wave got past the turning point"
        );
        let (_, node, beyond) = front(&vacuum);
        assert!(node > 0.8 && beyond > 0.8, "vacuum: {node:.3}, {beyond:.3}");

        let omega = std::f64::consts::TAU * PLASMA_HZ;
        let cutoff = std::f64::consts::TAU * 2.5;
        let flat = Harmonic::run(&plasma_mirror_with("W", cutoff), 0.08, 10.0, PLASMA_HZ, 3.0);
        let (k, measured) = reflection(&flat, -0.7, 0.9);
        let expected_k = (omega * omega - cutoff * cutoff).sqrt();
        let expected = (omega - expected_k) / (omega + expected_k);
        assert!(
            (k / expected_k - 1.0).abs() < 0.02,
            "k {k:.3} against {expected_k:.3}"
        );
        assert!(
            (measured - expected).abs() < 0.03,
            "the wall reflects {measured:.3} against {expected:.3}"
        );
    }

    /// `r` and `u` at the nodes nearest `points`, every `every` seconds, on a
    /// document carrying a restoring law, stepped from rest.
    fn integrated_traces(
        document: &TopologyDocument,
        edge: f64,
        seconds: f64,
        points: &[Point2],
        every: f64,
    ) -> Vec<(f64, Vec<f64>, Vec<f64>)> {
        let prepared = prepare(document, edge);
        let operator = prepared
            .canonical_temporal_operator
            .clone()
            .expect("a restoring law prepares a temporal operator");
        let forcing = prepared.canonical_forcing.clone();
        let dt = prepared.recommended_time_step();
        let nodes = points
            .iter()
            .map(|point| {
                operator
                    .base()
                    .node_points()
                    .iter()
                    .enumerate()
                    .min_by(|a, b| (*a.1 - *point).norm().total_cmp(&(*b.1 - *point).norm()))
                    .unwrap()
                    .0
            })
            .collect::<Vec<_>>();
        let mut state = CanonicalTemporalWaveState::zero(&operator, dt)
            .unwrap()
            .pinned(&operator, &forcing)
            .unwrap();
        let stride = ((every / dt).round() as usize).max(1);
        let steps = (seconds / dt).ceil() as usize;
        let mut rows = Vec::new();
        for step in 1..=steps {
            state.step_with_forcing(&operator, &forcing).unwrap();
            if step % stride == 0 {
                let field = operator
                    .primary_field_at(state.primary_flux(), state.time(), state.runtime())
                    .unwrap();
                rows.push((
                    state.time(),
                    nodes
                        .iter()
                        .map(|node| state.integrated_field()[*node])
                        .collect(),
                    nodes.iter().map(|node| field[*node]).collect(),
                ));
            }
        }
        rows
    }

    /// The times `r` at the `index`th recorded point crosses each odd
    /// multiple of π, by linear interpolation: when each fluxon's centre
    /// passes.
    fn fluxon_arrivals(rows: &[(f64, Vec<f64>, Vec<f64>)], index: usize) -> Vec<f64> {
        let mut arrivals = Vec::new();
        let mut next = std::f64::consts::PI;
        for pair in rows.windows(2) {
            let (t0, r0) = (pair[0].0, pair[0].1[index]);
            let (t1, r1) = (pair[1].0, pair[1].1[index]);
            while r0 < next && r1 >= next {
                arrivals.push(t0 + (t1 - t0) * (next - r0) / (r1 - r0));
                next += std::f64::consts::TAU;
            }
        }
        arrivals
    }

    /// The Josephson gallery claim. Holding the end at a constant voltage V
    /// winds its phase at V, and the line sheds one fluxon per turn: every
    /// 2π/V seconds a kink of 2π passes each point, at one steady speed below
    /// the wave speed, and between kinks `r` rests on a multiple of 2π. A
    /// Klein-Gordon line under the same bias screens it: DC is below its
    /// cutoff, so the winding end leaves the line beyond a few `c/ω₀` at rest.
    #[test]
    fn a_biased_josephson_line_sheds_one_fluxon_per_turn_of_its_phase() {
        let tau = std::f64::consts::TAU;
        let points = [-0.5, 0.0, 0.5].map(|x| Point2::new(x, 0.0));
        let rows = integrated_traces(&josephson_line(), 0.08, 10.0, &points, 0.02);
        let arrivals = (0..points.len())
            .map(|index| fluxon_arrivals(&rows, index))
            .collect::<Vec<_>>();
        let period = tau / JOSEPHSON_BIAS;
        for gap in arrivals[0].windows(2).take(3) {
            let gap = gap[1] - gap[0];
            assert!(
                (gap / period - 1.0).abs() < 0.01,
                "a fluxon every {gap:.3} s"
            );
        }
        let speeds = arrivals[0]
            .iter()
            .zip(&arrivals[2])
            .map(|(first, last)| 1.0 / (last - first))
            .collect::<Vec<_>>();
        assert!(
            speeds.len() >= 3,
            "{} fluxons crossed the line",
            speeds.len()
        );
        for speed in &speeds {
            assert!(
                *speed < 0.9 && (speed / speeds[0] - 1.0).abs() < 0.02,
                "fluxons at {speeds:.3?}"
            );
        }
        // Halfway between two fluxons, r rests on a whole number of turns.
        for pair in arrivals[1].windows(2) {
            let middle = 0.5 * (pair[0] + pair[1]);
            let (_, r, _) = rows
                .iter()
                .min_by(|a, b| (a.0 - middle).abs().total_cmp(&(b.0 - middle).abs()))
                .unwrap();
            let turns = r[1] / tau;
            assert!(
                (turns - turns.round()).abs() < 0.03,
                "r rests at {turns:.3} turns"
            );
        }

        let screened = integrated_traces(
            &josephson_line_with("R1", JOSEPHSON_BIAS),
            0.08,
            10.0,
            &[Point2::new(-0.5, 0.0)],
            0.1,
        );
        let furthest = screened
            .iter()
            .map(|(_, r, _)| r[0].abs())
            .fold(0.0, f64::max);
        assert!(
            furthest < 0.2,
            "the Klein-Gordon line moved to {furthest:.3}"
        );
    }

    /// The symmetry-breaking gallery claim. From the unstable top, the
    /// frozen noise tips the medium into both wells at once: at 2 s each sign
    /// holds a tenth of the grid or more. The walls between the domains then
    /// move under the loss until one well holds everything, at `|r| = 1`
    /// within 2%. With no double well the same source leaves `r` near zero.
    #[test]
    fn phi4_breaks_into_domains_of_both_signs_that_then_coarsen() {
        let grid = (0..9)
            .flat_map(|j| {
                (0..9).map(move |i| Point2::new(-0.9 + 0.225 * i as f64, 0.9 - 0.225 * j as f64))
            })
            .collect::<Vec<_>>();
        let at = |rows: &[(f64, Vec<f64>, Vec<f64>)], seconds: f64| {
            rows.iter()
                .min_by(|a, b| (a.0 - seconds).abs().total_cmp(&(b.0 - seconds).abs()))
                .unwrap()
                .1
                .clone()
        };
        let rows = integrated_traces(&symmetry_breaking(), 0.08, 8.0, &grid, 0.5);
        let early = at(&rows, 2.0);
        let (up, down) = (
            early.iter().filter(|r| **r > 0.9).count(),
            early.iter().filter(|r| **r < -0.9).count(),
        );
        assert!(
            up >= 8 && down >= 8,
            "at 2 s, {up} up and {down} down of 81"
        );
        let late = at(&rows, 8.0);
        let well = late[0].signum();
        assert!(
            late.iter().all(|r| (r * well - 1.0).abs() < 0.02),
            "at 8 s, r spans {:.3?}",
            late.iter()
                .fold((f64::MAX, f64::MIN), |(lo, hi), r| (lo.min(*r), hi.max(*r)))
        );
        let flat = integrated_traces(&symmetry_breaking_with(0.0, 1.0), 0.08, 8.0, &grid, 2.0);
        let furthest = flat
            .iter()
            .flat_map(|(_, r, _)| r.iter().map(|v| v.abs()))
            .fold(0.0, f64::max);
        assert!(furthest < 0.2, "without the well r reached {furthest:.3}");
    }

    /// Where `r` changes sign along a sampled line, by interpolation.
    fn sign_changes(line: &[f64], start: f64, spacing: f64) -> Vec<f64> {
        line.windows(2)
            .enumerate()
            .filter(|(_, pair)| pair[0].signum() != pair[1].signum())
            .map(|(index, pair)| start + spacing * (index as f64 + pair[0] / (pair[0] - pair[1])))
            .collect()
    }

    /// The CPU reference stepping a document's oscillator medium across edits,
    /// handing its state from one generation to the next the way the app
    /// does: `Q` interpolated without restoring component totals across a
    /// change of geometry, `b` reconstructed, `r` interpolated and, where a
    /// moved boundary opened new ground, extended from the same neighbours
    /// as `Q`.
    struct Session {
        editor: TopologyEditor,
        runtime: TopologyRuntime,
        meshing: MeshingOptions,
        operator: Arc<CanonicalTemporalWaveOperator>,
        forcing: Arc<CanonicalForcing>,
        state: CanonicalTemporalWaveState,
    }

    impl Session {
        fn start(document: TopologyDocument, edge: f64) -> Self {
            let editor = TopologyEditor::from_document(document).unwrap();
            let meshing = MeshingOptions {
                target_edge_length: edge,
                ..MeshingOptions::default()
            };
            let mut runtime = TopologyRuntime::default();
            let prepared = Self::prepare(&mut runtime, &editor, meshing, true);
            let operator = prepared.canonical_temporal_operator.clone().unwrap();
            let forcing = prepared.canonical_forcing.clone();
            let state =
                CanonicalTemporalWaveState::zero(&operator, prepared.recommended_time_step())
                    .unwrap()
                    .pinned(&operator, &forcing)
                    .unwrap();
            Self {
                editor,
                runtime,
                meshing,
                operator,
                forcing,
                state,
            }
        }

        fn prepare(
            runtime: &mut TopologyRuntime,
            editor: &TopologyEditor,
            meshing: MeshingOptions,
            fresh: bool,
        ) -> Arc<crate::topology_runtime::PreparedTopology> {
            let token = runtime
                .request(
                    editor.revision,
                    &editor.document,
                    editor.compiled_accepted.clone(),
                    meshing,
                    fresh,
                )
                .unwrap();
            loop {
                if let Some(result) = runtime.advance(1 << 16) {
                    result.unwrap();
                    return runtime.commit_ready(token).unwrap();
                }
            }
        }

        fn run(&mut self, seconds: f64) {
            for _ in 0..(seconds / self.state.time_step()).round() as usize {
                self.state
                    .step_with_forcing(&self.operator, &self.forcing)
                    .unwrap();
            }
        }

        /// Moves every curve by `shift`, as dragging the selection does, and
        /// hands the state to the generation the edit prepares.
        fn drag(&mut self, shift: Point2) {
            use crate::topology_viewport::{
                RigidTransform, TopologySpanTarget, plan_rigid_transform,
            };
            let geometry = self.editor.document.model.draft.geometry.clone();
            let spans = geometry
                .curves
                .iter()
                .flat_map(|curve| {
                    curve
                        .spans
                        .iter()
                        .map(|span| TopologySpanTarget::Curve(span.id))
                })
                .collect();
            let updates = plan_rigid_transform(
                &geometry,
                &spans,
                RigidTransform {
                    pivot: Point2::default(),
                    translation: shift,
                    rotation_radians: 0.0,
                    scale: 1.0,
                },
            )
            .unwrap();
            self.editor.begin();
            self.editor
                .apply_transform_updates_during_edit(&updates)
                .unwrap();
            self.editor.commit();
            while self.editor.acceptance == crate::topology_editor::TopologyAcceptance::Pending {
                self.editor.validate_frame(64);
            }
            let next = Self::prepare(&mut self.runtime, &self.editor, self.meshing, false);
            assert!(next.carve.is_some(), "{:?}", next.repair_fallback);
            let transfer = next
                .canonical_transfer
                .clone()
                .expect("a canonical transfer");
            let target = next.canonical_temporal_operator.clone().unwrap();
            let primary = transfer
                .primary
                .transfer(
                    self.state.primary_flux(),
                    &vec![None; transfer.primary.target_component_count()],
                    &vec![false; target.base().degrees_of_freedom()],
                )
                .unwrap()
                .0;
            let complementary = transfer
                .complementary
                .transfer(self.state.complementary_flux())
                .unwrap()
                .0;
            let integrated = transfer_integrated_field(
                next.transfer.as_ref().unwrap(),
                &transfer.primary,
                self.state.integrated_field(),
                &target,
            )
            .unwrap();
            self.state = CanonicalTemporalWaveState::new_at(
                &target,
                next.recommended_time_step(),
                primary,
                complementary,
                self.state.time(),
            )
            .unwrap()
            .with_integrated_field(&target, integrated.field)
            .unwrap();
            self.forcing = next.canonical_forcing.clone();
            self.operator = target;
        }

        /// `r` at the node nearest `point`.
        fn r_at(&self, point: Point2) -> f64 {
            let node = self
                .operator
                .base()
                .node_points()
                .iter()
                .enumerate()
                .min_by(|a, b| (*a.1 - point).norm().total_cmp(&(*b.1 - point).norm()))
                .unwrap()
                .0;
            self.state.integrated_field()[node]
        }

        /// Where `r` changes sign along the axis.
        fn walls(&self) -> Vec<f64> {
            let line = (0..=40)
                .map(|index| self.r_at(Point2::new(-1.0 + 0.05 * index as f64, 0.0)))
                .collect::<Vec<_>>();
            sign_changes(&line, -1.0, 0.05)
        }
    }

    /// The pinned-wall gallery claim. The wall forms 0.3 to the right of the
    /// waist, slides into it and stays within 0.03 of it, with the two wells
    /// either side. Dragging the bumps 0.3 in six steps half a second apart,
    /// as a drag in the app arrives, takes the wall with them: 4 s after the
    /// last step it is within 0.05 of the new waist.
    #[test]
    fn a_domain_wall_settles_at_the_waist_and_follows_it_when_dragged() {
        let mut session = Session::start(pinned_domain_wall(), 0.08);
        session.run(6.0);
        for _ in 0..4 {
            session.run(0.5);
            let walls = session.walls();
            assert!(
                walls.len() == 1 && walls[0].abs() < 0.03,
                "at {:.1} s the wall is at {walls:.3?}",
                session.state.time()
            );
        }
        let (left, right) = (
            session.r_at(Point2::new(-0.7, 0.0)),
            session.r_at(Point2::new(0.7, 0.0)),
        );
        assert!(
            left < -0.9 && right > 0.9,
            "the wells read {left:.3} and {right:.3}"
        );
        for _ in 0..6 {
            session.drag(Point2::new(0.05, 0.0));
            session.run(0.5);
        }
        session.run(4.0);
        let walls = session.walls();
        assert!(
            walls.len() == 1 && (walls[0] - 0.3).abs() < 0.05,
            "after the drag the wall is at {walls:.3?}"
        );
    }

    /// The bumps pull on a wall well away from the waist: one formed 0.5 off
    /// settles there, which the straight baffles tried first did not manage.
    /// In a channel without them the same wall drifts away.
    #[test]
    fn the_bumps_reach_a_wall_a_free_channel_leaves_alone() {
        let far = "sin(1.5 * (x - 0.5))";
        let mut pinned = Session::start(pinned_domain_wall_with(far, true), 0.08);
        pinned.run(10.0);
        let walls = pinned.walls();
        assert!(
            walls.len() == 1 && walls[0].abs() < 0.05,
            "with the bumps the wall is at {walls:.3?}"
        );
        let mut free = Session::start(pinned_domain_wall_with(far, false), 0.08);
        free.run(10.0);
        let walls = free.walls();
        assert!(
            walls.iter().all(|wall| wall.abs() > 0.25),
            "without them the wall is at {walls:.3?}"
        );
    }
}
