//! Built-in examples authored directly in the unified topology model.

use crate::document::{
    FarFieldSettings, MaterialOverlay, MaterialProperty, PresentationSettings, ProbeId,
    ProbeSamplingPreset, VectorOverlay,
};
use crate::topology_editor::{
    TopologyBoundaryProbeTarget, TopologyDocument, TopologyDocumentModel, TopologyProbeDefinition,
    TopologyProbeTarget,
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
            example(
                "Self-sustained emitter",
                "A disk of van der Pol oscillators starts from a faint seed and rings at its own \
                 3 Hz cutoff, sending rings through a plasma. Raise the plasma's cutoff above \
                 3 Hz and the rings stop: the disk still rings, but its tone cannot leave.",
                emitter(),
            ),
            example(
                "Plasma whispering gallery",
                "A vacuum disk walled by a plasma, driven at 3.1 Hz below the plasma's cutoff: \
                 ten lobes of a whispering-gallery mode build up around the rim, and outside it \
                 the field dies within a few hundredths. Drive it at 5 Hz, above the cutoff, and \
                 the wall turns transparent.",
                whispering_gallery(),
            ),
            example(
                "Parametric fiber amplifier",
                "A graded-index fiber whose permittivity is pumped at twice the signal's \
                 frequency by a wave running with it amplifies the signal about fivefold by the \
                 far end; shift the pump's phase by half a turn and the same signal is squeezed. \
                 Stop the pump's wave (wavenumber 0) and the fiber oscillates on its own.",
                fiber_amplifier(),
            ),
            example(
                "Bent fiber",
                "A glass fiber, single-mode at 4 Hz, carries its mode round a quarter turn of \
                 radius 0.5 and delivers about three quarters of it to the top. Round the bend \
                 the mode's outer flank would have to outrun the light outside, so it sheds a \
                 beam off tangentially; drag the bend tighter and it sheds more.",
                bent_fiber(),
            ),
            example(
                "Photonic crystal",
                "A plane wave at 1.85 Hz meets five columns of ceramic rods, ε = 9, in a square \
                 lattice: the frequency is in the crystal's band gap, the wave turns back, and \
                 under a thousandth of its power gets through. Tune the launcher to 1 Hz, below \
                 the gap, or 2.5 Hz, above it, and most of it passes.",
                photonic_crystal(),
            ),
            example(
                "Crystal bend",
                "The photonic crystal with a channel of missing rods that turns a right angle. \
                 At 1.85 Hz, in the band gap, the wave cannot enter the crystal, so it follows \
                 the channel round the corner and out through the top, delivering about nine \
                 tenths of what a straight channel does.",
                crystal_bend(),
            ),
            example(
                "Ring resonator",
                "A glass ring beside a glass fiber, driven at 3.975 Hz, one of the ring's \
                 resonances: over half a minute the ring fills to ten times its field between \
                 resonances, and past it the fiber keeps under a fifth of its power, the rest \
                 shed from the ring's bend. Tune the source to 3.885 Hz, between resonances, \
                 and the wave runs past.",
                ring_resonator(),
            ),
            example(
                "Acoustic whispering gallery",
                "A 4 Hz source just inside a round room's reflecting wall, open on the left: \
                 the sound clings to the wall all the way round, so the far wall, half a turn \
                 away and twice as far as the centre, is several times louder than the centre. \
                 Delete the wall and the centre is the louder.",
                acoustic_gallery(),
            ),
            example(
                "Dielectric whispering gallery",
                "A dielectric disk, ε = 4, with a 2.55 Hz source just inside its rim: at this, \
                 one of its whispering-gallery resonances, total internal reflection holds the \
                 wave running round inside the rim, and over half a minute it builds to fourteen \
                 lobes eight times the field at 2.7 Hz, between resonances.",
                dielectric_gallery(),
            ),
            example(
                "Maxwell's fisheye",
                "A disk whose index falls from 2 at its centre to 1 at its rim, n = 2/(1 + (r/R)²): \
                 every ray a source on the rim sends inward curves round to the opposite point, \
                 so the far rim lights up there five times brighter than 45° either side. Set \
                 the profile to 1 and the far rim is no brighter anywhere.",
                fisheye(),
            ),
            example(
                "Fresnel zone plate",
                "A plane wave meets a screen of reflecting strips open over the odd Fresnel zones \
                 for a focus 0.4 behind it: the waves from the open zones arrive there in step \
                 and gather into a spot 1.7 times the plane wave's amplitude, nearly three times \
                 its energy, and over three times the field 0.4 to either side.",
                zone_plate(),
            ),
            example(
                "Brewster angle",
                "A point source in front of glass, ε = 2.25, in the H_z skin: each ray meets the \
                 glass at its own angle, and the one meeting it at the Brewster angle, atan 1.5 = \
                 56°, reflects nothing, so the fringes the reflection makes with the direct wave \
                 fade out along its direction. Switch to E_z and every ray reflects, at 56° seven \
                 times as strongly.",
                brewster(),
            ),
            example(
                "Frustrated total internal reflection",
                "A glass fiber holds its light by total internal reflection, and outside the core \
                 the field dies away within a few hundredths. Lower a glass block to 0.05 above it \
                 and the reflection is frustrated: the light tunnels across the gap and leaves \
                 into the block as a tilted beam, draining 40% of the fiber's power. Raise the \
                 block to 0.15 and under a hundredth tunnels.",
                tunnelling(),
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

    /// A reflecting baffle, the clamped cubic with `controls` and one knot
    /// span per interval joined at `multiplicities`, running from −x to +x
    /// and welded to the floor or the ceiling at both ends; the pocket
    /// between it and the wall is left out of the domain. Its end controls
    /// are moved onto the wall vertices exactly, as the topology resolves
    /// them.
    fn wall_bump(&mut self, controls: Vec<Point2>, multiplicities: Vec<u8>) {
        let domain = self.scene.geometry.domain;
        let ceiling = controls[controls.len() / 2].y > 0.0;
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
        let mut points = controls;
        let last = points.len() - 1;
        let (left, right) = (points[0].x, points[last].x);
        let start = self.outer_vertex(side, fraction(left));
        let end = self.outer_vertex(side, fraction(right));
        points[0] = resolved(left);
        points[last] = resolved(right);
        let intervals = vec![1.0; multiplicities.len() + 1];
        let spline =
            OpenCubicSpline::new_with_multiplicities(points, intervals, multiplicities).unwrap();
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

    /// A wall-attached transmitting divider across the whole width at `y`,
    /// running from the left wall to the right one.
    fn level(&mut self, y: f64) -> (CurveId, CurveSpanId) {
        let domain = self.scene.geometry.domain;
        let left = self.outer_vertex(OuterSide::Left, (domain.max_y - y) / domain.height());
        let right = self.outer_vertex(OuterSide::Right, (y - domain.min_y) / domain.height());
        let span = CurveSpanId(self.next_span);
        let curve = self.open_curve(
            OpenCubicSpline::polyline(vec![
                Point2::new(domain.min_x, y),
                Point2::new(domain.max_x, y),
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
        authored.nodes[0].vertex = Some(left);
        authored.nodes[1].vertex = Some(right);
        (curve, span)
    }

    /// The band between two levels, from wall to wall, as a region of
    /// `material`. The background's own anchor, on the floor, names the face
    /// below it; the face above is a region of its own of the same material.
    fn band(&mut self, y0: f64, y1: f64, material: MaterialId) -> RegionId {
        self.level(y0);
        let (curve, span) = self.level(y1);
        let above = RegionId(self.next_region);
        self.next_region += 1;
        let background = self.scene.regions[0].material;
        self.scene.regions.push(Region {
            id: above,
            material: background,
            frame: MaterialFrame::world(),
        });
        self.scene.face_assignments.push(AuthoredFaceAssignment {
            anchor: FaceAnchor::Outer {
                side: OuterSide::Top,
                fraction: 0.5,
            },
            region: Some(above),
        });
        let region = RegionId(self.next_region);
        self.next_region += 1;
        self.scene.regions.push(Region {
            id: region,
            material,
            frame: MaterialFrame::world(),
        });
        // Running towards +x, the upper level's right is below it.
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
/// Each bump is half an ellipse this wide either side of x = 0 along the
/// wall, and deep enough to leave the waist.
const BUMP_HALF_WIDTH: f64 = 0.35;

fn pinned_domain_wall() -> TopologyDocument {
    pinned_domain_wall_with(PINNING_SEED, true)
}

/// A φ⁴ wall pinned at a waist. Two half-elliptic bumps, baffles welded to
/// the floor and the ceiling with the pockets behind them left out, narrow
/// the channel to a quarter of its height at x = 0. A wall's energy is its
/// tension times its length, and anywhere over the bumps a wall is shorter
/// the nearer it is to the waist, so it is pushed there; moving the bumps
/// drags it along. Without them a straight wall costs the same anywhere and
/// stays roughly where it formed.
fn pinned_domain_wall_with(seed: &str, bumps: bool) -> TopologyDocument {
    // A tenth of the symmetry-breaking seed: it still decides the fall, and
    // its 3 Hz shimmer in the field is ten times fainter.
    let mut builder = phi4_builder(PHI4_LAMBDA, seed, 0.1);
    builder.scene.outer_boundaries = channel();
    // The background's own anchor, on the floor at x = 0, would sit in the
    // floor's pocket.
    builder.scene.face_assignments[0].anchor = FaceAnchor::Outer {
        side: OuterSide::Left,
        fraction: 0.5,
    };
    if bumps {
        // Two cubic Bézier quarters of the ellipse, joined where they meet
        // below the wall: seven controls, smooth at the waist and square to
        // the wall at the feet.
        let (a, b) = (BUMP_HALF_WIDTH, 1.0 - WAIST);
        let k = 4.0 / 3.0 * (std::f64::consts::SQRT_2 - 1.0);
        for sign in [1.0, -1.0] {
            let at = |x: f64, depth: f64| Point2::new(x, sign * (1.0 - depth));
            builder.wall_bump(
                vec![
                    at(-a, 0.0),
                    at(-a, k * b),
                    at(-k * a, b),
                    at(0.0, b),
                    at(k * a, b),
                    at(a, k * b),
                    at(a, 0.0),
                ],
                vec![3],
            );
        }
    }
    phi4_document(builder)
}

/// The plasma's cutoff and the emitter's own, in Hz.
const PLASMA_CUTOFF_HZ: f64 = 2.0;
const EMITTER_HZ: f64 = 3.0;
/// The oscillators' gain, and the magnetic loss that limits it to their long
/// waves.
const EMITTER_GAIN: f64 = 15.0;
const EMITTER_MAGNETIC_LOSS: f64 = 25.0;

fn emitter() -> TopologyDocument {
    emitter_with(PLASMA_CUTOFF_HZ)
}

/// A disk of van der Pol oscillators with a 3 Hz cutoff in a Klein-Gordon
/// plasma whose cutoff is `plasma_hz`, seeded by a faint 2.5 Hz source at its
/// centre since a field exactly at rest never grows. The oscillators' gain
/// acts on the electric field and their loss on the magnetic one: a short
/// wave keeps half its energy in the magnetic field and is damped, while the
/// disk's near-uniform oscillation keeps almost none there and grows. So the
/// disk rings as one oscillator, at its own cutoff and its limit-cycle
/// amplitude, and radiates wherever the plasma lets that frequency through.
fn emitter_with(plasma_hz: f64) -> TopologyDocument {
    let mut builder = Builder::new();
    let physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.physics = physics;
    let medium = |base: &Material, oscillating: bool, name: &str, values: &[(&str, f64)]| {
        let preset = medium_presets()
            .iter()
            .find(|preset| preset.self_oscillating == oscillating && preset.restoring().id == "R1")
            .expect("a Klein-Gordon medium");
        let mut medium = apply_medium_preset(preset, base, physics)
            .expect("a medium applies to a fresh material");
        medium.name = name.into();
        for (parameter, value) in values {
            medium
                .parameters
                .iter_mut()
                .find(|candidate| candidate.name == *parameter)
                .expect("the medium names its parameter")
                .value = *value;
        }
        medium
    };
    let tau = std::f64::consts::TAU;
    builder.scene.materials[0] = medium(
        &builder.scene.materials[0],
        false,
        "Plasma",
        &[("omega0", tau * plasma_hz)],
    );
    let disk = Material {
        id: MaterialId(2),
        color: [214, 120, 88],
        magnetic_loss: Some(LossChannel {
            base_rate: ScalarField::constant(EMITTER_MAGNETIC_LOSS),
            law: DampingLaw::constant(),
        }),
        ..Material::default_medium()
    };
    builder.scene.materials.push(medium(
        &disk,
        true,
        "Oscillators",
        &[
            ("omega0", tau * EMITTER_HZ),
            ("gain", EMITTER_GAIN),
            ("threshold", 1.0),
        ],
    ));
    let region = builder.subdomain(
        PeriodicCubicSpline::rounded(Point2::new(0.0, 0.0), 0.25),
        MaterialId(2),
        MaterialFrame::world(),
    );
    let mut document = builder.document();
    document.model.source = PointSource {
        region,
        ..source(Point2::new(0.0, 0.0), 2.5, 0.01, 0.06)
    };
    document
}

/// A circle of `radius` about `center`: sixteen controls, whose uniform
/// B-spline sits 0.9746 of their radius in and departs from a circle by under
/// 1e-4 of it.
fn circle(center: Point2, radius: f64) -> PeriodicCubicSpline {
    let reach = radius / 0.974_6;
    PeriodicCubicSpline::uniform(
        (0..16)
            .map(|index| {
                let angle = index as f64 * std::f64::consts::TAU / 16.0;
                center + Point2::new(angle.cos(), angle.sin()) * reach
            })
            .collect(),
    )
    .unwrap()
}

const GALLERY_CENTRE: Point2 = Point2 { x: 0.05, y: 0.0 };
const GALLERY_RADIUS: f64 = 0.4;
const GALLERY_CUTOFF_HZ: f64 = 3.5;
/// The clad disk's `m = 5` whispering-gallery resonance, 3.098-3.099 Hz by
/// ringdown on meshes from edge 0.04 to 0.16.
const GALLERY_HZ: f64 = 3.1;

fn whispering_gallery() -> TopologyDocument {
    whispering_gallery_with(GALLERY_HZ)
}

/// A TM vacuum disk walled by a Klein-Gordon plasma with a 3.5 Hz cutoff,
/// driven at `frequency` by a point source just inside its rim. Below the
/// cutoff the plasma is a Drude metal: every wave meeting the rim turns
/// back, and the disk's whispering-gallery modes ring with nothing to leak
/// into, their field outside falling as `K_m(κr)` with
/// `κ = √(ω₀² − ω²)/c`. A probe sits opposite the source, on an antinode of
/// the `m = 5` mode, so its build-up shows in the probe plot.
fn whispering_gallery_with(frequency: f64) -> TopologyDocument {
    let mut builder = Builder::new();
    let physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.physics = physics;
    let preset = medium_presets()
        .iter()
        .find(|preset| !preset.self_oscillating && preset.restoring().id == "R1")
        .expect("the Klein-Gordon medium");
    let mut plasma = apply_medium_preset(preset, &builder.scene.materials[0], physics)
        .expect("a medium applies to a fresh material");
    plasma.name = "Plasma".into();
    plasma
        .parameters
        .iter_mut()
        .find(|parameter| parameter.name == "omega0")
        .expect("the medium names its cutoff")
        .value = std::f64::consts::TAU * GALLERY_CUTOFF_HZ;
    builder.scene.materials[0] = plasma;
    builder.scene.materials.push(Material {
        id: MaterialId(2),
        name: "Vacuum".into(),
        color: [40, 58, 72],
        ..Material::default_medium()
    });
    let region = builder.subdomain(
        circle(GALLERY_CENTRE, GALLERY_RADIUS),
        MaterialId(2),
        MaterialFrame::world(),
    );
    let mut document = builder.document();
    document.model.source = PointSource {
        region,
        ..source(
            GALLERY_CENTRE + Point2::new(GALLERY_RADIUS - 0.06, 0.0),
            frequency,
            10.0,
            0.03,
        )
    };
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Opposite rim".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Point(
            GALLERY_CENTRE - Point2::new(GALLERY_RADIUS - 0.06, 0.0),
        ),
    });
    document
}

const FIBER_H: f64 = 0.15;
const FIBER_DN: f64 = 0.6;
const FIBER_SIGNAL_HZ: f64 = 2.5;
const FIBER_DEPTH: f64 = 0.2;
/// Twice the fundamental mode's propagation constant at 2.5 Hz, measured on
/// the unpumped fiber (β = 21.94, `n_eff` 1.397; a 1D mode solve gives
/// 1.391), so the pump runs with the signal.
const FIBER_PUMP_WAVENUMBER: f64 = 43.88;
/// The pump phase that amplifies the source's quadrature most.
const FIBER_PUMP_PHASE: f64 = 0.75 * std::f64::consts::PI;

fn fiber_amplifier() -> TopologyDocument {
    fiber_amplifier_with(FIBER_DEPTH, FIBER_PUMP_WAVENUMBER, FIBER_PUMP_PHASE)
}

/// A TM graded-index fiber across the whole width, the band between two
/// levels at `±H`, its permittivity carrying the "Travelling modulation"
/// preset over the graded base at twice the signal's frequency, with
/// `depth`, `wavenumber` and `phase`. A weak signal source sits on its axis
/// at the left end; a probe reads the right end.
///
/// A pump that runs with the signal at `2β` keeps its phase against the
/// signal's all the way along, so it amplifies one quadrature of it and
/// squeezes the other, steadily, as the signal passes. A pump uniform in
/// space conserves the wavenumber instead of the frequency and couples the
/// forward wave to a backward one; over this length that closes a loop
/// above threshold and the fiber oscillates on its own.
fn fiber_amplifier_with(depth: f64, wavenumber: f64, phase: f64) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    let graded = Material {
        id: MaterialId(2),
        name: "Pumped fiber".into(),
        // In TM the rows are ε and μ, so both carry the index: `n = √(εμ)` is
        // graded and the impedance `√(μ/ε)` stays one.
        mass_density: ScalarField::formula("1 + dn * max(0, 1 - (y / H)^2)").unwrap(),
        stiffness: ScalarField::formula("1 + dn * max(0, 1 - (y / H)^2)").unwrap(),
        parameters: vec![
            MaterialParameter {
                name: "H".into(),
                value: FIBER_H,
            },
            MaterialParameter {
                name: "dn".into(),
                value: FIBER_DN,
            },
        ],
        color: [46, 120, 139],
        ..Material::default_medium()
    };
    let preset = law_presets()
        .iter()
        .find(|candidate| {
            candidate.name == "Travelling modulation" && candidate.row == LawPresetRow::Mass
        })
        .expect("the catalogue offers the travelling modulation");
    let mut fiber = apply_law_preset(preset, &graded).expect("a preset applies to the fiber");
    for (name, value) in [
        ("depth", depth),
        ("pump_hz", 2.0 * FIBER_SIGNAL_HZ),
        ("pump_phase", phase),
        ("wavenumber", wavenumber),
        ("wave_angle", 0.0),
    ] {
        fiber
            .parameters
            .iter_mut()
            .find(|parameter| parameter.name == name)
            .expect("the preset names its parameter")
            .value = value;
    }
    builder.scene.materials.push(fiber);
    let region = builder.band(-FIBER_H, FIBER_H, MaterialId(2));
    let mut document = builder.document();
    document.model.source = PointSource {
        region,
        ..source(Point2::new(-0.85, 0.0), FIBER_SIGNAL_HZ, 4.0, 0.04)
    };
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Output".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Point(Point2::new(0.85, 0.0)),
    });
    // From past the source's near field, which would set the plot's scale,
    // to the far end: its energy density shows the signal growing along the
    // fiber.
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(2),
        name: "Along the fiber".into(),
        color: [248, 196, 112],
        enabled: true,
        target: TopologyProbeTarget::Segment {
            start: Point2::new(-0.75, 0.0),
            end: Point2::new(0.9, 0.0),
            preset: ProbeSamplingPreset::Medium,
        },
    });
    document
}

const BEND_CENTRE: Point2 = Point2 { x: -0.4, y: 0.0 };
const BEND_RADIUS: f64 = 0.5;
const CORE_WIDTH: f64 = 0.1;
const CORE_PERMITTIVITY: f64 = 2.25;
const BEND_HZ: f64 = 4.0;

/// A path `radius` from the bend's centre: from `entry` along `y = −radius`,
/// a quarter circle about the centre, and straight up to `exit`. Cubic Bézier
/// pieces joined at C0 knots, the quarter in two eighths whose departure from
/// the circle is under 5e-6 of its radius.
fn bend_path(radius: f64, entry: Point2, exit: Point2) -> OpenCubicSpline {
    let at = |angle: f64| BEND_CENTRE + Point2::new(angle.cos(), angle.sin()) * radius;
    let tangent = |angle: f64| Point2::new(-angle.sin(), angle.cos()) * radius;
    // The control reach of a cubic Bézier eighth of a circle.
    let reach = 4.0 / 3.0 * (std::f64::consts::PI / 16.0).tan();
    let quarter = std::f64::consts::FRAC_PI_2;
    let (arc_start, arc_end) = (at(-quarter), at(0.0));
    let mut controls = vec![
        entry,
        entry.lerp(arc_start, 1.0 / 3.0),
        entry.lerp(arc_start, 2.0 / 3.0),
    ];
    for (from, to) in [(-quarter, -quarter / 2.0), (-quarter / 2.0, 0.0)] {
        controls.extend([
            at(from),
            at(from) + tangent(from) * reach,
            at(to) - tangent(to) * reach,
        ]);
    }
    controls.extend([
        arc_end,
        arc_end.lerp(exit, 1.0 / 3.0),
        arc_end.lerp(exit, 2.0 / 3.0),
        exit,
    ]);
    let intervals = vec![
        (arc_start - entry).norm(),
        radius * quarter / 2.0,
        radius * quarter / 2.0,
        (exit - arc_end).norm(),
    ];
    OpenCubicSpline::new_with_multiplicities(controls, intervals, vec![3; 3]).unwrap()
}

/// One edge of a bent fiber, `radius` from the bend's centre, from the left
/// wall to the top wall. Its ends are on the walls exactly, as the topology
/// resolves its vertices.
fn bend_edge(builder: &mut Builder, radius: f64) -> (CurveId, CurveSpanId, f64) {
    let domain = builder.scene.geometry.domain;
    let left = (domain.max_y + radius) / domain.height();
    let top = (domain.max_x - BEND_CENTRE.x - radius) / domain.width();
    let start = builder.outer_vertex(OuterSide::Left, left);
    let end = builder.outer_vertex(OuterSide::Top, top);
    let entry = Point2::new(domain.min_x, domain.max_y - domain.height() * left);
    let exit = Point2::new(domain.max_x - domain.width() * top, domain.max_y);
    let spline = bend_path(radius, entry, exit);
    let parameter = spline.span_bounds(0).map(|[a, b]| (a + b) * 0.5).unwrap();
    let span = CurveSpanId(builder.next_span);
    let curve = builder.open_curve(spline, &[SpanBehavior::Transmitting; 4]);
    let authored = builder
        .scene
        .geometry
        .curves
        .iter_mut()
        .find(|candidate| candidate.id == curve)
        .unwrap();
    let last = authored.nodes.len() - 1;
    authored.nodes[0].vertex = Some(start);
    authored.nodes[last].vertex = Some(end);
    (curve, span, parameter)
}

/// A TM scene whose second material is glass, `ε = 2.25` and `μ = 1`.
fn glass_builder() -> Builder {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.materials.push(Material {
        id: MaterialId(2),
        name: "Glass".into(),
        mass_density: ScalarField::constant(CORE_PERMITTIVITY),
        color: [66, 105, 151],
        ..Material::default_medium()
    });
    builder
}

fn bent_fiber() -> TopologyDocument {
    bent_fiber_with(BEND_RADIUS)
}

/// A TM step-index fiber of glass, `ε = 2.25` and `μ = 1`, 0.1 wide, which is
/// single-mode at 4 Hz: in from the left wall along `y = −radius`, a quarter
/// turn of `radius` about (−0.4, 0), and out to the top wall. A source in the
/// core at its left end launches the mode; round the bend the mode's outer
/// flank would have to outrun the light in the cladding, and there it leaks
/// off tangentially.
///
/// A free transmitting curve along the core's axis, from just past the
/// source to 0.2 short of the top wall, carries a probe whose energy density
/// follows the power the mode carries along its length. Its samples are close
/// enough to resolve that density's ripple at half the guided wavelength,
/// 0.094. A point probe past its end reads the output.
fn bent_fiber_with(radius: f64) -> TopologyDocument {
    let mut builder = glass_builder();
    bend_edge(&mut builder, radius - 0.5 * CORE_WIDTH);
    let (outer, span, parameter) = bend_edge(&mut builder, radius + 0.5 * CORE_WIDTH);
    // The background's own anchor, on the floor, names the face outside the
    // bend; the face inside it is a region of its own of the same material.
    let inside = RegionId(builder.next_region);
    builder.next_region += 1;
    builder.scene.regions.push(Region {
        id: inside,
        material: DEFAULT_MATERIAL,
        frame: MaterialFrame::world(),
    });
    builder.scene.face_assignments.push(AuthoredFaceAssignment {
        anchor: FaceAnchor::Outer {
            side: OuterSide::Top,
            fraction: 0.9,
        },
        region: Some(inside),
    });
    let core = RegionId(builder.next_region);
    builder.next_region += 1;
    builder.scene.regions.push(Region {
        id: core,
        material: MaterialId(2),
        frame: MaterialFrame::world(),
    });
    // Running towards +x, the outer edge's left is the core above it.
    builder.scene.face_assignments.push(AuthoredFaceAssignment {
        anchor: FaceAnchor::Curve {
            curve: outer,
            span,
            side: CurveTraceSide::Left,
            parameter,
        },
        region: Some(core),
    });
    let first = builder.next_span;
    let axis = builder.open_curve(
        bend_path(
            radius,
            Point2::new(-0.8, -radius),
            Point2::new(BEND_CENTRE.x + radius, 0.8),
        ),
        &[SpanBehavior::Transmitting; 4],
    );
    let mut document = builder.document();
    document.model.source = PointSource {
        region: core,
        ..source(Point2::new(-0.9, -radius), BEND_HZ, 10.0, 0.03)
    };
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Output".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Point(Point2::new(BEND_CENTRE.x + radius, 0.9)),
    });
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(2),
        name: "Along the core".into(),
        color: [248, 196, 112],
        enabled: true,
        target: TopologyProbeTarget::Boundary(TopologyBoundaryProbeTarget {
            curve: axis,
            spans: (first..first + 4).map(CurveSpanId).collect(),
            side: CurveTraceSide::Left,
            reversed: false,
            preset: ProbeSamplingPreset::High,
        }),
    });
    document
}

const CRYSTAL_PITCH: f64 = 0.2;
const CRYSTAL_ROD_FRACTION: f64 = 0.2;
const CRYSTAL_ROD_PERMITTIVITY: f64 = 9.0;
const CRYSTAL_GAP_HZ: f64 = 1.85;

/// A regular octagon about `center` with the area of a circle of `radius`,
/// its flats facing the axes. Meshed as spline circles of radius 0.04, the
/// crystal's rods brought its shortest edge down to 0.004 and its time step
/// to a sixth of the octagons', at four times the unknowns.
fn octagonal_rod(center: Point2, radius: f64) -> PeriodicCubicSpline {
    let eighth = std::f64::consts::TAU / 8.0;
    let reach = radius * (std::f64::consts::PI / (4.0 * eighth.sin())).sqrt();
    PeriodicCubicSpline::polygon(
        (0..8)
            .map(|index| {
                let angle = (index as f64 + 0.5) * eighth;
                center + Point2::new(angle.cos(), angle.sin()) * reach
            })
            .collect(),
    )
    .unwrap()
}

fn photonic_crystal() -> TopologyDocument {
    photonic_crystal_with(CRYSTAL_GAP_HZ, true)
}

/// A TM channel lit by a launcher at `frequency` at the left, with a square
/// lattice of ceramic rods, `ε = 9` and 0.2 of the pitch in radius, five
/// columns deep and ten rows filling the height. The reflecting walls sit on
/// the lattice's mirror planes, so the channel is the infinite crystal at
/// normal incidence, whose TM gap runs from 0.275 to 0.445 of the pitch over
/// the wavelength. A line probe runs along the midline, between two rows,
/// and a point probe reads the transmitted wave.
fn photonic_crystal_with(frequency: f64, rods: bool) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.outer_boundaries = channel();
    builder.launcher(-0.85, frequency, 40.0);
    builder.scene.materials.push(Material {
        id: MaterialId(2),
        name: "Ceramic".into(),
        mass_density: ScalarField::constant(CRYSTAL_ROD_PERMITTIVITY),
        color: [66, 105, 151],
        ..Material::default_medium()
    });
    if rods {
        for column in 0..5 {
            for row in 0..10 {
                let centre = Point2::new(
                    (column as f64 - 2.0) * CRYSTAL_PITCH,
                    -1.0 + (row as f64 + 0.5) * CRYSTAL_PITCH,
                );
                builder.subdomain(
                    octagonal_rod(centre, CRYSTAL_ROD_FRACTION * CRYSTAL_PITCH),
                    MaterialId(2),
                    MaterialFrame::world(),
                );
            }
        }
    }
    let mut document = builder.document();
    document.model.source.enabled = false;
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Along the channel".into(),
        color: [248, 196, 112],
        enabled: true,
        target: TopologyProbeTarget::Segment {
            start: Point2::new(-0.75, 0.0),
            end: Point2::new(0.9, 0.0),
            preset: ProbeSamplingPreset::Medium,
        },
    });
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(2),
        name: "Behind the crystal".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Point(Point2::new(0.75, 0.0)),
    });
    document
}

/// The lattice sites, three either side of the centre, a crystal bend's
/// channel runs through: in along the middle row from the left, round the
/// centre, and out up the middle column.
fn bend_channel(column: i32, row: i32) -> bool {
    (row == 0 && column <= 0) || (column == 0 && row >= 0)
}

/// A TM block of the photonic crystal's rods, seven by seven about the
/// origin, with the sites `open` names left empty. Every wall is outgoing.
fn crystal_block(open: fn(i32, i32) -> bool) -> Builder {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.materials.push(Material {
        id: MaterialId(2),
        name: "Ceramic".into(),
        mass_density: ScalarField::constant(CRYSTAL_ROD_PERMITTIVITY),
        color: [66, 105, 151],
        ..Material::default_medium()
    });
    for column in -3..=3 {
        for row in -3..=3 {
            if !open(column, row) {
                builder.subdomain(
                    octagonal_rod(
                        Point2::new(column as f64, row as f64) * CRYSTAL_PITCH,
                        CRYSTAL_ROD_FRACTION * CRYSTAL_PITCH,
                    ),
                    MaterialId(2),
                    MaterialFrame::world(),
                );
            }
        }
    }
    builder
}

/// The crystal's own gap frequency, from inside a channel's entrance.
fn channel_source() -> PointSource {
    source(Point2::new(-0.5, 0.0), CRYSTAL_GAP_HZ, 10.0, 0.03)
}

/// The photonic crystal as a block with a channel of missing rods that turns
/// a right angle at the centre, lit from inside its entrance at the gap
/// frequency. The wave cannot enter the crystal, so the channel guides it
/// round the corner and out through the top. A free transmitting path along
/// the channel, from past the source round the corner to 0.2 short of the
/// top wall, carries a probe whose energy density follows the wave along
/// it; a point probe past its end reads what leaves.
fn crystal_bend() -> TopologyDocument {
    let mut builder = crystal_block(bend_channel);
    let first = builder.next_span;
    let path = builder.open_curve(
        OpenCubicSpline::polyline(vec![
            Point2::new(-0.4, 0.0),
            Point2::new(0.0, 0.0),
            Point2::new(0.0, 0.8),
        ])
        .unwrap(),
        &[SpanBehavior::Transmitting; 2],
    );
    let mut document = builder.document();
    document.model.source = channel_source();
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Along the channel".into(),
        color: [248, 196, 112],
        enabled: true,
        target: TopologyProbeTarget::Boundary(TopologyBoundaryProbeTarget {
            curve: path,
            spans: (first..first + 2).map(CurveSpanId).collect(),
            side: CurveTraceSide::Left,
            reversed: false,
            preset: ProbeSamplingPreset::Medium,
        }),
    });
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(2),
        name: "Out of the corner".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Point(Point2::new(0.0, 0.9)),
    });
    document
}

const RING_RADIUS: f64 = 0.6;
/// Close to critical coupling: on the ring's resonance the bus keeps 0.2% of
/// its power at edge 0.08, where 0.05 left 46%.
const RING_GAP: f64 = 0.02;
/// Between the ring's resonance at edge 0.08, 3.968 Hz, and at edges 0.05 and
/// 0.04, 3.98 Hz, so the scene stays on it as adaptation refines the mesh.
const RING_HZ: f64 = 3.975;

fn ring_resonator() -> TopologyDocument {
    ring_resonator_with(RING_HZ, true)
}

/// A TM bus fiber of the bent fiber's glass along the floor, `y` from −0.6
/// to −0.5 from wall to wall, and, with `ring`, a glass ring of the same
/// width, mean radius 0.6, 0.02 above it, lit by a source in the bus at its
/// left end. At one of the ring's resonances the ring fills up and its
/// output cancels the wave running past it in the bus, the power going out
/// as the radiation its bend sheds. A probe reads the ring's energy and
/// another the bus past the ring.
fn ring_resonator_with(frequency: f64, ring: bool) -> TopologyDocument {
    let mut builder = glass_builder();
    let bus = builder.band(-0.6, -0.5, MaterialId(2));
    let mut region = None;
    if ring {
        let centre = Point2::new(0.0, -0.5 + RING_GAP + 0.5 * CORE_WIDTH + RING_RADIUS);
        region = Some(builder.subdomain(
            circle(centre, RING_RADIUS + 0.5 * CORE_WIDTH),
            MaterialId(2),
            MaterialFrame::world(),
        ));
        builder.subdomain(
            circle(centre, RING_RADIUS - 0.5 * CORE_WIDTH),
            DEFAULT_MATERIAL,
            MaterialFrame::world(),
        );
    }
    let mut document = builder.document();
    document.model.source = PointSource {
        region: bus,
        ..source(Point2::new(-0.9, -0.55), frequency, 10.0, 0.03)
    };
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Past the ring".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Point(Point2::new(0.8, -0.55)),
    });
    if let Some(region) = region {
        document.model.probes.push(TopologyProbeDefinition {
            id: ProbeId(2),
            name: "Ring".into(),
            color: [248, 196, 112],
            enabled: true,
            target: TopologyProbeTarget::AreaRegion(region),
        });
    }
    document
}

/// A circular arc about `center` from angle `from` to `to`, in radians and
/// counterclockwise, as `pieces` cubic Bézier pieces joined at C0 knots.
fn arc(center: Point2, radius: f64, from: f64, to: f64, pieces: usize) -> OpenCubicSpline {
    let step = (to - from) / pieces as f64;
    let reach = 4.0 / 3.0 * (step / 4.0).tan() * radius;
    let at = |angle: f64| center + Point2::new(angle.cos(), angle.sin()) * radius;
    let tangent = |angle: f64| Point2::new(-angle.sin(), angle.cos());
    let mut controls = vec![at(from)];
    for piece in 0..pieces {
        let (a, b) = (from + step * piece as f64, from + step * (piece + 1) as f64);
        controls.extend([
            at(a) + tangent(a) * reach,
            at(b) - tangent(b) * reach,
            at(b),
        ]);
    }
    OpenCubicSpline::new_with_multiplicities(
        controls,
        vec![radius * step; pieces],
        vec![3; pieces - 1],
    )
    .unwrap()
}

const GALLERY_WALL: f64 = 0.85;
const ACOUSTIC_HZ: f64 = 4.0;
/// The far wall's receiver and the centre's, as the scene's area probes read
/// them: a disk 0.07 inside the wall opposite the source, and a wide one at
/// the centre, wide enough to average over the room's standing pattern.
const FAR_WALL: (Point2, f64) = (Point2 { x: 0.0, y: -0.78 }, 0.05);
const ROOM_CENTRE: (Point2, f64) = (Point2 { x: 0.0, y: 0.0 }, 0.25);

fn acoustic_gallery() -> TopologyDocument {
    acoustic_gallery_with(true)
}

/// A Mechanical room walled by a reflecting arc of radius 0.85 about the
/// origin, 300° of it, open over the 60° on the left so the sound can leave,
/// and a 4 Hz source 0.05 inside the wall at the top. Sound launched along
/// the wall clings to it all the way round, so the far wall, half a turn
/// away, is louder than the centre. Outside, every wall is outgoing.
fn acoustic_gallery_with(wall: bool) -> TopologyDocument {
    let mut builder = Builder::new();
    if wall {
        let open = std::f64::consts::PI / 6.0;
        builder.baffle(arc(
            Point2::default(),
            GALLERY_WALL,
            -std::f64::consts::PI + open,
            std::f64::consts::PI - open,
            8,
        ));
    }
    let mut document = builder.document();
    document.model.source = source(
        Point2::new(0.0, GALLERY_WALL - 0.05),
        ACOUSTIC_HZ,
        10.0,
        0.03,
    );
    for (id, name, color, (center, radius)) in [
        (1, "Far wall", [91, 220, 194], FAR_WALL),
        (2, "Centre", [248, 196, 112], ROOM_CENTRE),
    ] {
        document.model.probes.push(TopologyProbeDefinition {
            id: ProbeId(id),
            name: name.into(),
            color,
            enabled: true,
            target: TopologyProbeTarget::AreaDisk { center, radius },
        });
    }
    document
}

const DISK_RADIUS: f64 = 0.3;
const DISK_PERMITTIVITY: f64 = 4.0;
/// The disk's `m = 7` whispering-gallery resonance, 2.55 Hz at edges 0.08
/// and 0.05. It is 88% full at 30 s; `m = 9`, at 3.17 Hz, was still growing
/// at a minute.
const DISK_HZ: f64 = 2.55;

fn dielectric_gallery() -> TopologyDocument {
    dielectric_gallery_with(DISK_HZ)
}

/// A TM dielectric disk, `ε = 4` and `μ = 1`, radius 0.3 about the origin,
/// with a point source 0.03 inside its rim. On a whispering-gallery
/// resonance the field runs round just inside the rim, held there by total
/// internal reflection, and stands in `2m` lobes. A probe runs round the
/// rim itself, and a point probe sits opposite the source. Every wall is
/// outgoing.
fn dielectric_gallery_with(frequency: f64) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.materials.push(Material {
        id: MaterialId(2),
        name: "Dielectric".into(),
        mass_density: ScalarField::constant(DISK_PERMITTIVITY),
        color: [66, 105, 151],
        ..Material::default_medium()
    });
    let (rim, first) = (CurveId(builder.next_curve), builder.next_span);
    let spline = circle(Point2::default(), DISK_RADIUS);
    let spans = spline.intervals().len() as u64;
    let region = builder.subdomain(spline, MaterialId(2), MaterialFrame::world());
    let mut document = builder.document();
    document.model.source = PointSource {
        region,
        ..source(Point2::new(DISK_RADIUS - 0.03, 0.0), frequency, 10.0, 0.02)
    };
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Round the rim".into(),
        color: [248, 196, 112],
        enabled: true,
        target: TopologyProbeTarget::Boundary(TopologyBoundaryProbeTarget {
            curve: rim,
            spans: (first..first + spans).map(CurveSpanId).collect(),
            side: CurveTraceSide::Left,
            reversed: false,
            preset: ProbeSamplingPreset::High,
        }),
    });
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(2),
        name: "Opposite rim".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Point(Point2::new(0.05 - DISK_RADIUS, 0.0)),
    });
    document
}

const FISHEYE_RADIUS: f64 = 0.45;
const FISHEYE_HZ: f64 = 3.0;

fn fisheye() -> TopologyDocument {
    fisheye_with(true)
}

/// A TM Maxwell fisheye, `n = 2/(1 + (r/R)²)` as `ε = n²` with `μ = 1`, in a
/// disk of radius 0.45 about the origin: the index falls from 2 at the centre
/// to 1 at the rim, where it meets the vacuum outside. Every ray a point on
/// the rim sends inward runs on a circular arc to the opposite point, so a
/// 3 Hz source 0.03 inside the rim on the left images onto the right. A
/// probe runs round the rim, and a point probe sits on the image. Without the
/// `lens` the disk is vacuum. Every wall is outgoing.
fn fisheye_with(lens: bool) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.materials.push(Material {
        id: MaterialId(2),
        name: "Maxwell fisheye".into(),
        mass_density: ScalarField::formula(if lens { "(2 / (1 + (r / R)^2))^2" } else { "1" })
            .unwrap(),
        parameters: vec![MaterialParameter {
            name: "R".into(),
            value: FISHEYE_RADIUS,
        }],
        color: [66, 105, 151],
        ..Material::default_medium()
    });
    let (rim, first) = (CurveId(builder.next_curve), builder.next_span);
    let spline = circle(Point2::default(), FISHEYE_RADIUS);
    let spans = spline.intervals().len() as u64;
    let region = builder.subdomain(
        spline,
        MaterialId(2),
        MaterialFrame {
            attachment: MaterialFrameAttachment::FollowRegion,
            ..MaterialFrame::world()
        },
    );
    let mut document = builder.document();
    document.model.source = PointSource {
        region,
        ..source(
            Point2::new(0.03 - FISHEYE_RADIUS, 0.0),
            FISHEYE_HZ,
            10.0,
            0.03,
        )
    };
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Round the rim".into(),
        color: [248, 196, 112],
        enabled: true,
        target: TopologyProbeTarget::Boundary(TopologyBoundaryProbeTarget {
            curve: rim,
            spans: (first..first + spans).map(CurveSpanId).collect(),
            side: CurveTraceSide::Left,
            reversed: false,
            preset: ProbeSamplingPreset::Medium,
        }),
    });
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(2),
        name: "Image".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Point(Point2::new(FISHEYE_RADIUS - 0.03, 0.0)),
    });
    document.presentation.material_overlay = MaterialOverlay::Property(MaterialProperty::WaveSpeed);
    document
}

const ZONE_HZ: f64 = 4.0;
/// Short enough that three open zones fit inside the walls; in 2D more zones
/// barely raise the focus, 1.64× to 1.68× the plane wave for 0.3 to 0.5.
const ZONE_FOCUS: f64 = 0.4;

/// The edges inside the domain of the zones of a plate focusing a plane wave
/// of `frequency` at `focus` behind it, `y_n = √(nλF + (nλ/2)²)`.
fn zone_edges(frequency: f64, focus: f64, reach: f64) -> Vec<f64> {
    let wavelength = 1.0 / frequency;
    (1..)
        .map(|n| {
            let n = n as f64;
            (n * wavelength * focus + (0.5 * n * wavelength).powi(2)).sqrt()
        })
        .take_while(|edge| *edge < reach)
        .collect()
}

fn zone_plate() -> TopologyDocument {
    zone_plate_with(true)
}

/// A Mechanical plane wave from a 4 Hz launcher at the left, and, with
/// `plate`, a screen of reflecting baffles at `x = 0` over the even Fresnel
/// zones for a focus 0.4 behind it, open over the odd ones, whose waves
/// arrive there in step; a last zone cut by the wall is closed out to it when
/// it is even. A point probe sits on the focus and a line probe runs along
/// the axis behind the screen. Every wall is outgoing.
fn zone_plate_with(plate: bool) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.launcher(-0.85, ZONE_HZ, 40.0);
    if plate {
        let domain = builder.scene.geometry.domain;
        let edges = zone_edges(ZONE_HZ, ZONE_FOCUS, domain.max_y);
        for sign in [1.0, -1.0] {
            for (index, pair) in edges.windows(2).enumerate() {
                // Zone `index + 2` runs from `pair[0]` to `pair[1]`.
                if index % 2 == 0 {
                    builder.baffle(straight_baffle(sign * pair[0], sign * pair[1]));
                }
            }
            if edges.len() % 2 == 1 {
                // The zone the wall cuts is even: closed out to the wall.
                let (side, fraction) = if sign > 0.0 {
                    (OuterSide::Top, (domain.max_x - 0.0) / domain.width())
                } else {
                    (OuterSide::Bottom, (0.0 - domain.min_x) / domain.width())
                };
                let wall = builder.outer_vertex(side, fraction);
                let end = if sign > 0.0 {
                    domain.max_y
                } else {
                    domain.min_y
                };
                let curve = builder.baffle(straight_baffle(sign * edges[edges.len() - 1], end));
                let authored = builder
                    .scene
                    .geometry
                    .curves
                    .iter_mut()
                    .find(|candidate| candidate.id == curve)
                    .unwrap();
                let last = authored.nodes.len() - 1;
                authored.nodes[last].vertex = Some(wall);
            }
        }
    }
    let mut document = builder.document();
    document.model.source.enabled = false;
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Focus".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Point(Point2::new(ZONE_FOCUS, 0.0)),
    });
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(2),
        name: "Along the axis".into(),
        color: [248, 196, 112],
        enabled: true,
        target: TopologyProbeTarget::Segment {
            start: Point2::new(0.05, 0.0),
            end: Point2::new(0.95, 0.0),
            preset: ProbeSamplingPreset::Medium,
        },
    });
    document
}

const BREWSTER_HZ: f64 = 4.0;
/// The glass's face, and the source 0.2 in front of it.
const BREWSTER_FACE: f64 = 0.2;
const BREWSTER_SOURCE: Point2 = Point2 { x: 0.0, y: 0.0 };
/// The radius, about the source's mirror image in the face, of the arc the
/// reflection is read on and the probe runs along.
const BREWSTER_ARC: f64 = 0.8;

/// The source's mirror image in the glass's face, from which every reflected
/// ray leaves.
fn brewster_image() -> Point2 {
    Point2::new(2.0 * BREWSTER_FACE - BREWSTER_SOURCE.x, BREWSTER_SOURCE.y)
}

fn brewster() -> TopologyDocument {
    brewster_with(ElectromagneticPolarization::Te, true)
}

/// A point source at 4 Hz 0.2 in front of glass, `ε = 2.25` and `μ = 1`,
/// filling everything beyond `x = 0.2`, in the skin whose out-of-plane field
/// `polarization` names. Each ray meets the face at its own angle, and in
/// the `H_z` skin (TE) the one meeting it at the Brewster angle, `atan 1.5`,
/// reflects nothing, where in the `E_z` skin (TM) every ray reflects. A free
/// transmitting arc about the source's mirror image, through the directions
/// the rays meeting the face from 20° to 72° reflect into, carries a probe:
/// the fringes the reflection makes with the direct wave fade out along it
/// at 56°. Without `glass` the half-plane is vacuum. Every wall is outgoing.
fn brewster_with(polarization: ElectromagneticPolarization, glass: bool) -> TopologyDocument {
    let mut builder = glass_builder();
    builder.scene.physics = PhysicsModel::Electromagnetic { polarization };
    if !glass {
        builder.scene.materials[1].mass_density = ScalarField::constant(1.0);
    }
    let (curve, span) = builder.divider(BREWSTER_FACE);
    let region = RegionId(builder.next_region);
    builder.next_region += 1;
    builder.scene.regions.push(Region {
        id: region,
        material: MaterialId(2),
        frame: MaterialFrame::world(),
    });
    // Running upwards, the divider's right is the glass.
    builder.scene.face_assignments.push(AuthoredFaceAssignment {
        anchor: FaceAnchor::Curve {
            curve,
            span,
            side: CurveTraceSide::Right,
            parameter: 0.5,
        },
        region: Some(region),
    });
    let first = builder.next_span;
    let path = builder.open_curve(
        arc(
            brewster_image(),
            BREWSTER_ARC,
            std::f64::consts::PI - 72f64.to_radians(),
            std::f64::consts::PI - 20f64.to_radians(),
            2,
        ),
        &[SpanBehavior::Transmitting; 2],
    );
    let mut document = builder.document();
    document.model.source = source(BREWSTER_SOURCE, BREWSTER_HZ, 10.0, 0.03);
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Reflected 20° to 72°".into(),
        color: [248, 196, 112],
        enabled: true,
        target: TopologyProbeTarget::Boundary(TopologyBoundaryProbeTarget {
            curve: path,
            spans: (first..first + 2).map(CurveSpanId).collect(),
            side: CurveTraceSide::Left,
            reversed: false,
            preset: ProbeSamplingPreset::Medium,
        }),
    });
    document
}

const TUNNEL_HZ: f64 = 4.0;
/// Where the glass block over the fiber begins.
const TUNNEL_BLOCK: f64 = -0.3;
/// The gallery's gap: 40% of the guided power tunnels into the block over
/// its length, where 0.1 lets 5% through and 0.15 half a percent.
const TUNNEL_GAP: f64 = 0.05;

fn tunnelling() -> TopologyDocument {
    tunnelling_with(TUNNEL_GAP, true)
}

/// The bent fiber's glass as a straight fiber along `y = −0.5` from wall to
/// wall, lit by a 4 Hz source at its left end, and a glass block over it
/// from `x = −0.3` to the top and right walls, `gap` above the fiber. The
/// fiber's mode is light held by total internal reflection at 61.9° inside
/// the core, its field outside falling as `e^{−γy}` with
/// `γ = k₀√(n_eff² − 1) = 21.8`; where the block reaches into that tail the
/// reflection is frustrated, and the mode leaks into the block as a beam at
/// that same angle, draining at a rate that falls with the gap as `e^{−2γd}`.
/// The block's edge is a wall-attached curve, so the beam leaves through the
/// walls rather than being turned back by a free face. Without `glass` the
/// block is vacuum, on the same mesh. Every wall is outgoing.
fn tunnelling_with(gap: f64, glass: bool) -> TopologyDocument {
    let mut builder = glass_builder();
    builder.scene.materials.push(Material {
        id: MaterialId(3),
        name: "Block".into(),
        mass_density: ScalarField::constant(if glass { CORE_PERMITTIVITY } else { 1.0 }),
        color: [66, 105, 151],
        ..Material::default_medium()
    });
    let domain = builder.scene.geometry.domain;
    builder.level(-0.55);
    let (top, top_span) = builder.level(-0.45);
    let fiber = RegionId(builder.next_region);
    builder.next_region += 1;
    builder.scene.regions.push(Region {
        id: fiber,
        material: MaterialId(2),
        frame: MaterialFrame::world(),
    });
    // Running towards +x, the upper level's right is the fiber below it.
    builder.scene.face_assignments.push(AuthoredFaceAssignment {
        anchor: FaceAnchor::Curve {
            curve: top,
            span: top_span,
            side: CurveTraceSide::Right,
            parameter: 0.5,
        },
        region: Some(fiber),
    });
    let above = RegionId(builder.next_region);
    builder.next_region += 1;
    builder.scene.regions.push(Region {
        id: above,
        material: DEFAULT_MATERIAL,
        frame: MaterialFrame::world(),
    });
    builder.scene.face_assignments.push(AuthoredFaceAssignment {
        anchor: FaceAnchor::Outer {
            side: OuterSide::Top,
            fraction: 0.9,
        },
        region: Some(above),
    });
    let face = -0.45 + gap;
    let start = builder.outer_vertex(
        OuterSide::Top,
        (domain.max_x - TUNNEL_BLOCK) / domain.width(),
    );
    let end = builder.outer_vertex(OuterSide::Right, (face - domain.min_y) / domain.height());
    let span = CurveSpanId(builder.next_span);
    let edge = builder.open_curve(
        OpenCubicSpline::polyline(vec![
            Point2::new(TUNNEL_BLOCK, domain.max_y),
            Point2::new(TUNNEL_BLOCK, face),
            Point2::new(domain.max_x, face),
        ])
        .unwrap(),
        &[SpanBehavior::Transmitting; 2],
    );
    let authored = builder
        .scene
        .geometry
        .curves
        .iter_mut()
        .find(|candidate| candidate.id == edge)
        .unwrap();
    authored.nodes[0].vertex = Some(start);
    authored.nodes[2].vertex = Some(end);
    let block = RegionId(builder.next_region);
    builder.next_region += 1;
    builder.scene.regions.push(Region {
        id: block,
        material: MaterialId(3),
        frame: MaterialFrame::world(),
    });
    // Running down its left edge, the block is on the curve's left.
    builder.scene.face_assignments.push(AuthoredFaceAssignment {
        anchor: FaceAnchor::Curve {
            curve: edge,
            span,
            side: CurveTraceSide::Left,
            parameter: 0.5 * (domain.max_y - face),
        },
        region: Some(block),
    });
    let mut document = builder.document();
    document.model.source = PointSource {
        region: fiber,
        ..source(Point2::new(-0.9, -0.5), TUNNEL_HZ, 10.0, 0.03)
    };
    for (id, name, color, point) in [
        (1, "Fiber output", [91, 220, 194], Point2::new(0.85, -0.5)),
        (2, "In the block", [248, 196, 112], Point2::new(0.6, 0.0)),
    ] {
        document.model.probes.push(TopologyProbeDefinition {
            id: ProbeId(id),
            name: name.into(),
            color,
            enabled: true,
            target: TopologyProbeTarget::Point(point),
        });
    }
    document
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topology_catalog_is_valid_version_22_data_with_complete_semantics() {
        assert_eq!(catalog().len(), 29);
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

    /// Every gallery probe compiles on its scene's mesh. A point probe on a
    /// curve, or a boundary probe on a span with no trace to read, fails only
    /// in its readout otherwise.
    #[test]
    fn every_gallery_probe_compiles_on_its_scene() {
        use crate::topology_runtime::TopologyProbeCompilation;
        for example in catalog() {
            let prepared = prepare(&example.document, 0.16);
            assert_eq!(
                prepared.probes.len(),
                example.document.model.probes.len(),
                "{}",
                example.name
            );
            for probe in prepared.probes.iter() {
                if let TopologyProbeCompilation::Failed(error) = &probe.result {
                    panic!("{}: probe {:?}: {error}", example.name, probe.id);
                }
                assert!(
                    matches!(probe.result, TopologyProbeCompilation::Ready(_)),
                    "{}: probe {:?} is disabled",
                    example.name,
                    probe.id
                );
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
        omega: f64,
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
                omega,
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

        /// The element holding `point`, its barycentric coordinates there,
        /// and their gradients.
        fn locate(&self, point: Point2) -> (usize, [f64; 3], [Point2; 3]) {
            let mesh = &self.prepared.mesh;
            mesh.triangles
                .iter()
                .enumerate()
                .find_map(|(index, triangle)| {
                    let [a, b, c] = triangle.vertices.map(|vertex| mesh.vertices[vertex].point);
                    let twice = (b - a).cross(c - a);
                    let weights = [
                        (b - point).cross(c - point) / twice,
                        (c - point).cross(a - point) / twice,
                        (a - point).cross(b - point) / twice,
                    ];
                    let gradients = [
                        Point2::new(b.y - c.y, c.x - b.x) / twice,
                        Point2::new(c.y - a.y, a.x - c.x) / twice,
                        Point2::new(a.y - b.y, b.x - a.x) / twice,
                    ];
                    weights
                        .iter()
                        .all(|weight| *weight >= -1e-10)
                        .then_some((index, weights, gradients))
                })
                .expect("the point is in the domain")
        }

        /// `U` at `point`, through the element's own basis.
        fn interpolated(&self, point: Point2) -> (f64, f64) {
            let (element, barycentric, _) = self.locate(point);
            let nodes = self.prepared.operator.element_nodes()[element];
            enriched_quadratic_basis(barycentric)
                .iter()
                .zip(nodes)
                .fold((0.0, 0.0), |sum, (weight, node)| {
                    let (re, im) = self.amplitude[node as usize];
                    (sum.0 + weight * re, sum.1 + weight * im)
                })
        }

        /// The time-averaged power a TM wave with `μ = 1` carries through the
        /// segment from `start` to `end` towards `normal`, by the midpoint
        /// rule over `count` pieces: `S = Im(U ∇U*) / 2ω` for
        /// `u = Re(U e^{iωt})`.
        fn power_through(&self, start: Point2, end: Point2, normal: Point2, count: usize) -> f64 {
            let length = (end - start).norm() / count as f64;
            (0..count)
                .map(|index| {
                    let point = start.lerp(end, (index as f64 + 0.5) / count as f64);
                    let (element, barycentric, gradients) = self.locate(point);
                    let nodes = self.prepared.operator.element_nodes()[element];
                    let values = enriched_quadratic_basis(barycentric);
                    let slopes = enriched_quadratic_basis_gradients(barycentric, gradients);
                    let (mut u, mut du) = ((0.0, 0.0), (0.0, 0.0));
                    for local in 0..7 {
                        let (re, im) = self.amplitude[nodes[local] as usize];
                        let slope = normal.dot(slopes[local]);
                        u = (u.0 + values[local] * re, u.1 + values[local] * im);
                        du = (du.0 + slope * re, du.1 + slope * im);
                    }
                    (u.1 * du.0 - u.0 * du.1) / (2.0 * self.omega) * length
                })
                .sum()
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
    /// document carrying a temporal law, stepped from rest. Without a
    /// restoring law there is no `r`, and it reads zero.
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
            .expect("a temporal law prepares a temporal operator");
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
                        .map(|node| state.integrated_field().get(*node).copied().unwrap_or(0.0))
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
    /// change of geometry, `r` interpolated and, where a moved boundary opened
    /// new ground, extended from the same neighbours as `Q`, and `b` rebuilt
    /// about the new `r` with the invariant `b − ηC r` carried.
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

            let integrated = transfer_integrated_field(
                next.transfer.as_ref().unwrap(),
                &transfer.primary,
                self.state.integrated_field(),
                &target,
            )
            .unwrap();
            let complementary = transfer_oscillator_flux(
                &transfer.complementary,
                self.operator.base(),
                self.state.complementary_flux(),
                self.state.integrated_field(),
                target.base(),
                &integrated.field,
            )
            .unwrap()
            .0;
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
        // Each handoff rebuilt `b` about the new `r`, so nothing was left for
        // the field to settle against: `b − ηC r` is still zero.
        let r = session.state.integrated_field();
        let b = session.state.complementary_flux();
        let potential = session.operator.base().compatible_flux(r).unwrap();
        let parted = b
            .iter()
            .zip(&potential)
            .map(|(b, p)| (*b - *p).norm().powi(2))
            .sum::<f64>()
            .sqrt()
            / b.iter().map(|b| b.norm().powi(2)).sum::<f64>().sqrt();
        assert!(parted < 1.0e-9, "b − ηC r holds {parted:.3e} of b");
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
    /// The complex amplitude at `frequency` over the last `seconds` of a
    /// series sampled every `dt`.
    fn phasor(series: &[f64], dt: f64, frequency: f64, seconds: f64) -> (f64, f64) {
        let count = (seconds / dt).round() as usize;
        let window = &series[series.len() - count..];
        let (mut re, mut im) = (0.0, 0.0);
        for (index, value) in window.iter().enumerate() {
            let phase = std::f64::consts::TAU * frequency * index as f64 * dt;
            re += value * phase.cos();
            im -= value * phase.sin();
        }
        (2.0 * re / count as f64, 2.0 * im / count as f64)
    }

    /// The strongest frequency between `low` and `high` over the last
    /// `seconds`, to a thousandth of a hertz.
    fn tone(series: &[f64], dt: f64, low: f64, high: f64, seconds: f64) -> f64 {
        let steps = ((high - low) * 1000.0).round() as usize;
        (0..=steps)
            .map(|index| low + index as f64 * 0.001)
            .map(|frequency| {
                let (re, im) = phasor(series, dt, frequency, seconds);
                (frequency, re.hypot(im))
            })
            .fold(
                (low, 0.0),
                |best, next| if next.1 > best.1 { next } else { best },
            )
            .0
    }

    /// The emitter gallery claim. From a faint 2.5 Hz seed the disk grows to
    /// one oscillation at its own cutoff, 3 Hz within 1%, with the Rayleigh
    /// limit-cycle amplitude `2a/√3` at its centre within 5%. The plasma
    /// carries it off as rings whose phase runs along a ray at
    /// `k = 2π√(f² − f_c²)` within 5%, and 0.7 away the seed's own frequency
    /// is under 2% of the tone. With the plasma's cutoff at 3.6 Hz, above the
    /// tone, nothing drains the disk and it rings more strongly, while its
    /// tone 0.7 away is under 5% of what the radiating disk sends there.
    #[test]
    fn a_self_sustained_emitter_rings_at_its_cutoff_and_radiates_only_through_a_lower_one() {
        let seconds = 10.0;
        let window = 4.0;
        let ray = (0..=10)
            .map(|index| Point2::new(0.4 + 0.05 * index as f64, 0.0))
            .collect::<Vec<_>>();
        let mut points = vec![Point2::new(0.0, 0.0), Point2::new(0.0, -0.7)];
        points.extend(&ray);
        let rows = integrated_traces(&emitter(), 0.08, seconds, &points, 0.0);
        let dt = rows[1].0 - rows[0].0;
        let series = |rows: &[(f64, Vec<f64>, Vec<f64>)], index: usize| {
            rows.iter().map(|row| row.2[index]).collect::<Vec<_>>()
        };
        let far = series(&rows, 1);
        let frequency = tone(&far, dt, 2.0, 4.0, window);
        assert!(
            (frequency / EMITTER_HZ - 1.0).abs() < 0.01,
            "the disk rings at {frequency:.3} Hz"
        );
        let magnitude = |series: &[f64], frequency: f64| {
            let (re, im) = phasor(series, dt, frequency, window);
            re.hypot(im)
        };
        let centre = magnitude(&series(&rows, 0), frequency);
        let limit_cycle = 2.0 / 3.0_f64.sqrt();
        assert!(
            (centre / limit_cycle - 1.0).abs() < 0.05,
            "the centre oscillates at {centre:.3}"
        );
        let radiated = magnitude(&far, frequency);
        let seed = magnitude(&far, 2.5);
        assert!(
            seed < 0.02 * radiated,
            "the seed {seed:.4} against {radiated:.3}"
        );

        // The phase along the ray, unwrapped, against the plasma's
        // dispersion at the measured tone.
        let mut phases = Vec::new();
        for index in 0..ray.len() {
            let (re, im) = phasor(&series(&rows, 2 + index), dt, frequency, window);
            let mut phase = im.atan2(re);
            if let Some(last) = phases.last() {
                while phase - last > std::f64::consts::PI {
                    phase -= std::f64::consts::TAU;
                }
                while phase - last < -std::f64::consts::PI {
                    phase += std::f64::consts::TAU;
                }
            }
            phases.push(phase);
        }
        let xs = ray.iter().map(|point| point.x).collect::<Vec<_>>();
        let (mean_x, mean_phase) = (
            xs.iter().sum::<f64>() / xs.len() as f64,
            phases.iter().sum::<f64>() / phases.len() as f64,
        );
        let slope = xs
            .iter()
            .zip(&phases)
            .map(|(x, phase)| (x - mean_x) * (phase - mean_phase))
            .sum::<f64>()
            / xs.iter().map(|x| (x - mean_x).powi(2)).sum::<f64>();
        let expected = std::f64::consts::TAU
            * (frequency * frequency - PLASMA_CUTOFF_HZ * PLASMA_CUTOFF_HZ).sqrt();
        assert!(
            (slope.abs() / expected - 1.0).abs() < 0.05,
            "the rings advance at {:.3} rad per unit against {expected:.3}",
            slope.abs()
        );

        let trapped = integrated_traces(
            &emitter_with(3.6),
            0.08,
            seconds,
            &[Point2::new(0.0, 0.0), Point2::new(0.0, -0.7)],
            0.0,
        );
        let held_series = series(&trapped, 0);
        let trapped_tone = tone(&held_series, dt, 2.0, 4.0, window);
        let held = magnitude(&held_series, trapped_tone);
        let leaked = magnitude(&series(&trapped, 1), trapped_tone);
        assert!(
            held > centre,
            "the trapped disk rings at {held:.3} against {centre:.3}"
        );
        assert!(
            leaked < 0.05 * radiated,
            "0.7 away the trapped tone is {leaked:.4} against {radiated:.3}"
        );
    }
    /// The mesh-scale lasing a self-oscillating region used to show. Without
    /// its magnetic loss, at gain 5, the emitter's rim grew an 11.5 Hz
    /// pattern three nodes to a wavelength that replaced the tone by 21 s.
    /// The short-wave viscosity holds the rim to its tone: from 12 s to 27 s
    /// the tone there moves under 5%, and nothing between 6 and 100 Hz
    /// passes 0.05 (the tone's own harmonics sit near 0.02).
    #[test]
    fn a_self_oscillating_rim_keeps_to_its_tone_without_a_magnetic_loss() {
        let mut document = emitter_with(PLASMA_CUTOFF_HZ);
        for scene in [&mut document.model.draft, &mut document.model.accepted] {
            let disk = &mut scene.materials[1];
            disk.magnetic_loss = None;
            disk.parameters
                .iter_mut()
                .find(|parameter| parameter.name == "gain")
                .expect("the oscillators name their gain")
                .value = 5.0;
        }
        let rim = [Point2::new(0.1669, -0.004), Point2::new(0.0, 0.17)];
        let rows = integrated_traces(&document, 0.08, 27.0, &rim, 0.0);
        let dt = rows[1].0 - rows[0].0;
        for (index, point) in rim.iter().enumerate() {
            let series = rows.iter().map(|row| row.2[index]).collect::<Vec<_>>();
            let window = |start: f64| {
                let from = (start / dt) as usize;
                &series[from..from + (3.0 / dt) as usize]
            };
            let magnitude = |chunk: &[f64], frequency: f64| {
                let (re, im) = phasor(chunk, dt, frequency, 2.99);
                re.hypot(im)
            };
            let early = window(12.0);
            let frequency = tone(early, dt, 2.0, 4.0, 2.99);
            let (before, after) = (
                magnitude(early, frequency),
                magnitude(window(24.0), frequency),
            );
            assert!(
                (after / before - 1.0).abs() < 0.05,
                "{point:?}: the tone went from {before:.3} to {after:.3}"
            );
            let strongest = (12..=200)
                .map(|half| magnitude(window(24.0), 0.5 * half as f64))
                .fold(0.0, f64::max);
            assert!(strongest < 0.05, "{point:?}: {strongest:.3} above 6 Hz");
        }
    }
    /// The plasma whispering-gallery claims, at edge 0.08 over the last 2 s
    /// of 12. Driven on its `m = 5` resonance, the rim away from the source
    /// rings more than 3× as strongly as when driven between resonances at
    /// 3.225 Hz, with `2m = 10` lobes around it. Outside the rim, opposite
    /// the source, the field falls as `K_5(κr)` with `κ = √(ω₀² − ω²)/c`:
    /// from 0.02 to 0.11 out its ratio is K_5's within 25%, and nearer that
    /// than the plasma's own skin `e^{−κ·0.09}`, since the mode's angular
    /// order steepens the fall. Driven at 5 Hz, above the cutoff, the wall is
    /// transparent: 0.35 out the field keeps more than half its strength
    /// 0.02 out, where on resonance it keeps under 5%.
    #[test]
    fn a_plasma_walled_disk_rings_in_a_whispering_gallery_mode_below_the_cutoff() {
        // The angular order of the resonance `GALLERY_HZ` drives.
        const GALLERY_ORDER: u32 = 5;
        let seconds = 12.0;
        let angles = 32;
        let ring = (0..angles)
            .map(|index| {
                let angle = index as f64 * std::f64::consts::TAU / angles as f64;
                GALLERY_CENTRE + Point2::new(angle.cos(), angle.sin()) * (GALLERY_RADIUS - 0.04)
            })
            .collect::<Vec<_>>();
        let outside = |depth: f64| GALLERY_CENTRE - Point2::new(GALLERY_RADIUS + depth, 0.0);
        let mut points = ring.clone();
        points.extend([outside(0.02), outside(0.11), outside(0.35)]);
        let run = |frequency: f64| {
            let rows = integrated_traces(
                &whispering_gallery_with(frequency),
                0.08,
                seconds,
                &points,
                0.0,
            );
            let tail = &rows[rows.len() * 5 / 6..];
            (0..points.len())
                .map(|index| {
                    (tail.iter().map(|row| row.2[index].powi(2)).sum::<f64>() / tail.len() as f64)
                        .sqrt()
                })
                .collect::<Vec<_>>()
        };
        let resonant = run(GALLERY_HZ);
        let detuned = run(3.225);
        let transparent = run(5.0);
        // The half of the ring away from the source.
        let far_rim = |rms: &[f64]| {
            (angles / 4..3 * angles / 4)
                .map(|index| rms[index].powi(2))
                .sum::<f64>()
                .sqrt()
        };
        assert!(
            far_rim(&resonant) > 3.0 * far_rim(&detuned),
            "the rim rings at {:.4} on resonance and {:.4} off it",
            far_rim(&resonant),
            far_rim(&detuned)
        );
        let lobes = &resonant[..angles];
        let mean = lobes.iter().sum::<f64>() / angles as f64;
        let crossings = (0..angles)
            .filter(|index| (lobes[*index] - mean) * (lobes[(index + 1) % angles] - mean) < 0.0)
            .count();
        assert_eq!(
            crossings,
            4 * GALLERY_ORDER as usize,
            "{crossings} crossings"
        );

        // `K_m(x) = ∫₀^∞ e^{−x cosh t} cosh(mt) dt`.
        let bessel_k = |order: f64, x: f64| {
            let step = 1.0e-3;
            (0..20_000)
                .map(|index| {
                    let t = (index as f64 + 0.5) * step;
                    (-x * t.cosh()).exp() * (order * t).cosh() * step
                })
                .sum::<f64>()
        };
        let kappa = std::f64::consts::TAU
            * (GALLERY_CUTOFF_HZ * GALLERY_CUTOFF_HZ - GALLERY_HZ * GALLERY_HZ).sqrt();
        let order = f64::from(GALLERY_ORDER);
        let expected = bessel_k(order, kappa * (GALLERY_RADIUS + 0.11))
            / bessel_k(order, kappa * (GALLERY_RADIUS + 0.02));
        let measured = resonant[angles + 1] / resonant[angles];
        assert!(
            (measured / expected - 1.0).abs() < 0.25,
            "the field falls to {measured:.3} of itself against K_5's {expected:.3}"
        );
        let skin = (-kappa * 0.09).exp();
        assert!(
            (measured - expected).abs() < (measured - skin).abs(),
            "the fall {measured:.3} is nearer the plasma's skin {skin:.3} than K_5's {expected:.3}"
        );
        let reach = |rms: &[f64]| rms[angles + 2] / rms[angles];
        assert!(
            reach(&resonant) < 0.05 && reach(&transparent) > 0.5,
            "0.35 out the field keeps {:.3} of itself on resonance and {:.3} at 5 Hz",
            reach(&resonant),
            reach(&transparent)
        );
    }
    /// The parametric fiber claims, at edge 0.08, read at the far end at
    /// 2.5 Hz over 2 s windows. The pump running with the signal amplifies it
    /// more than 4× against the pump off, and steadily: 6 s later the gain
    /// is the same within 10%. Advanced by half a turn, the same pump
    /// squeezes it below half. The same pump uniform in space makes the fiber
    /// an oscillator: its far-end field grows more than 3× from 8 s to 12 s.
    #[test]
    fn a_pump_running_with_the_signal_amplifies_it_where_a_standing_one_oscillates() {
        let output = Point2::new(0.85, 0.0);
        let far_end = |document: &TopologyDocument, seconds: f64, windows: &[f64]| {
            let rows = integrated_traces(document, 0.08, seconds, &[output], 0.0);
            let dt = rows[1].0 - rows[0].0;
            let series = rows.iter().map(|row| row.2[0]).collect::<Vec<_>>();
            windows
                .iter()
                .map(|end| {
                    let (re, im) = phasor(&series[..(end / dt) as usize], dt, FIBER_SIGNAL_HZ, 2.0);
                    re.hypot(im)
                })
                .collect::<Vec<_>>()
        };
        let off = far_end(&fiber_amplifier_with(0.0, 0.0, 0.0), 8.0, &[8.0])[0];
        let pumped = far_end(&fiber_amplifier(), 14.0, &[8.0, 14.0]);
        let squeezed = far_end(
            &fiber_amplifier_with(
                FIBER_DEPTH,
                FIBER_PUMP_WAVENUMBER,
                FIBER_PUMP_PHASE + std::f64::consts::PI,
            ),
            8.0,
            &[8.0],
        )[0];
        let standing = far_end(
            &fiber_amplifier_with(FIBER_DEPTH, 0.0, FIBER_PUMP_PHASE),
            12.0,
            &[8.0, 12.0],
        );
        let (gain, later) = (pumped[0] / off, pumped[1] / off);
        assert!(gain > 4.0, "the pump amplifies {gain:.3}×");
        assert!(
            (later / gain - 1.0).abs() < 0.1,
            "the gain moved from {gain:.3} to {later:.3}"
        );
        assert!(
            squeezed / off < 0.5,
            "advanced by π it passes {:.3}×",
            squeezed / off
        );
        assert!(
            standing[1] > 3.0 * standing[0],
            "a standing pump's far end went from {:.4} to {:.4}",
            standing[0],
            standing[1]
        );
    }

    /// The fundamental mode of the glass core as a slab, `cos(uy/a)` inside
    /// and `cos(u)·e^{−w(|y|−a)/a}` outside, at `offset` from its axis, with
    /// `u tan u = w` and `u² + w² = V²`.
    fn core_mode(offset: f64, frequency: f64) -> f64 {
        let half = 0.5 * CORE_WIDTH;
        let v = std::f64::consts::TAU * frequency * half * (CORE_PERMITTIVITY - 1.0).sqrt();
        let (mut low, mut high) = (0.0, std::f64::consts::FRAC_PI_2);
        for _ in 0..60 {
            let u = 0.5 * (low + high);
            if u * u.tan() > (v * v - u * u).sqrt() {
                high = u;
            } else {
                low = u;
            }
        }
        let u = 0.5 * (low + high);
        let w = (v * v - u * u).sqrt();
        if offset.abs() <= half {
            (u * offset / half).cos()
        } else {
            u.cos() * (-w * (offset.abs() - half) / half).exp()
        }
    }

    /// The power the core's mode carries through a cut `across` it at
    /// `centre`, up to a constant: `|∫ U φ ds / ∫ φ² ds|²` over 0.25 either
    /// side. The cladding's radiation is orthogonal to the mode and drops out.
    fn guided_power(scene: &Harmonic, centre: Point2, across: Point2) -> f64 {
        let frequency = scene.omega / std::f64::consts::TAU;
        let (mut re, mut im, mut norm) = (0.0, 0.0, 0.0);
        for index in 0..100 {
            let offset = 0.5 * ((index as f64 + 0.5) / 100.0 - 0.5);
            let mode = core_mode(offset, frequency);
            let (a, b) = scene.interpolated(centre + across * offset);
            re += a * mode;
            im += b * mode;
            norm += mode * mode;
        }
        (re * re + im * im) / (norm * norm)
    }

    /// The bent-fiber claims, at edge 0.08, from the power in the core's mode
    /// 0.2 short of the top wall, against a straight fiber from the same
    /// source 0.2 short of the right wall, whose power is all the source
    /// launches into the mode. (On that straight fiber the mode runs at
    /// `n_eff` 1.329 against a slab solve's 1.323, and 1.3238 at edge 0.04.)
    /// The gallery's radius 0.5 delivers more than 60% of it (72%); radius
    /// 0.3 loses more than twice what radius 0.7 loses (45% against 12%).
    #[test]
    fn a_bent_fiber_leaks_more_the_sharper_its_bend() {
        let mut builder = glass_builder();
        let core = builder.band(-0.55, -0.45, MaterialId(2));
        let mut straight = builder.document();
        straight.model.source = PointSource {
            region: core,
            ..source(Point2::new(-0.9, -0.5), BEND_HZ, 10.0, 0.03)
        };
        let straight = Harmonic::run(&straight, 0.08, 8.0, BEND_HZ, 4.0);
        let launched = guided_power(&straight, Point2::new(0.8, -0.5), Point2::new(0.0, 1.0));
        let delivered = |radius: f64| {
            let bent = Harmonic::run(&bent_fiber_with(radius), 0.08, 8.0, BEND_HZ, 4.0);
            let exit = Point2::new(BEND_CENTRE.x + radius, 0.8);
            guided_power(&bent, exit, Point2::new(1.0, 0.0)) / launched
        };
        let gallery = delivered(BEND_RADIUS);
        assert!(gallery > 0.6, "radius 0.5 delivers {gallery:.3}");
        let (sharp, gentle) = (1.0 - delivered(0.3), 1.0 - delivered(0.7));
        assert!(
            sharp > 2.0 * gentle,
            "radius 0.3 loses {sharp:.3}, radius 0.7 {gentle:.3}"
        );
    }

    /// The photonic-crystal claims, at edge 0.08, from the transmitted plane
    /// wave: the phasor averaged across the channel 0.25 behind the last
    /// column, where every other diffraction order has died, against the
    /// empty channel. In the gap, at 1.85 Hz, five columns pass under 1% of
    /// the power (0.07%); below it, at 1 Hz, more than 90% (99.9%); above
    /// it, at 2.5 Hz, more than half (79%). At edge 0.05 the three are 0.08%,
    /// 99.5% and 79%.
    #[test]
    fn a_rod_crystal_turns_back_its_gap_and_passes_either_side() {
        let transmission = |frequency: f64| {
            let behind = |rods: bool| {
                let scene = Harmonic::run(
                    &photonic_crystal_with(frequency, rods),
                    0.08,
                    10.0,
                    frequency,
                    3.0,
                );
                let (re, im) = (0..40)
                    .map(|index| {
                        let y = -1.0 + 2.0 * (index as f64 + 0.5) / 40.0;
                        scene.interpolated(Point2::new(0.75, y))
                    })
                    .fold((0.0, 0.0), |sum, (a, b)| (sum.0 + a, sum.1 + b));
                re.hypot(im)
            };
            (behind(true) / behind(false)).powi(2)
        };
        let gap = transmission(CRYSTAL_GAP_HZ);
        assert!(gap < 0.01, "the gap passes {gap:.4}");
        let below = transmission(1.0);
        assert!(below > 0.9, "1 Hz passes {below:.3}");
        let above = transmission(2.5);
        assert!(above > 0.5, "2.5 Hz passes {above:.3}");
    }

    /// The crystal-bend claims, at edge 0.08, 12 s from rest at 1.85 Hz, from
    /// the time-averaged power through a cut across the channel 0.14 inside
    /// the crystal's exit face, against the same cut across a straight
    /// channel's exit from the same source. The bend delivers more than 70% of
    /// it (91%; 91% at edge 0.05). The bend's own small reflection returns to
    /// the source and moves what it emits, so across the channel's band, 1.65
    /// to 2.15 Hz, the ratio runs from 0.90 to 1.10. With the channel filled,
    /// under 1% gets out (under 1e-5).
    #[test]
    fn a_channel_through_the_crystal_turns_a_right_angle() {
        let run =
            |document: &TopologyDocument| Harmonic::run(document, 0.08, 12.0, CRYSTAL_GAP_HZ, 3.0);
        let control = |open: fn(i32, i32) -> bool| {
            let mut document = crystal_block(open).document();
            document.model.source = channel_source();
            run(&document)
        };
        let across = |scene: &Harmonic, centre: Point2, normal: Point2| {
            let side = Point2::new(-normal.y, normal.x) * 0.3;
            scene.power_through(centre - side, centre + side, normal, 120)
        };
        let up = Point2::new(0.0, 1.0);
        let straight = across(
            &control(|_, row| row == 0),
            Point2::new(0.5, 0.0),
            Point2::new(1.0, 0.0),
        );
        let bend = across(&run(&crystal_bend()), Point2::new(0.0, 0.5), up);
        let filled = across(&control(|_, _| false), Point2::new(0.0, 0.5), up);
        assert!(straight > 0.0);
        assert!(
            bend > 0.7 * straight,
            "the bend delivers {:.3}",
            bend / straight
        );
        assert!(
            filled.abs() < 0.01 * straight,
            "the filled crystal lets out {:.4}",
            filled / straight
        );
    }

    /// The ring-resonator claims, at edge 0.08, 30 s from rest, with the
    /// power in the fiber's mode past the ring against the bus alone at the
    /// same frequency. At 3.975 Hz, on the ring's resonance, the bus keeps
    /// under half of what it keeps at 3.885 Hz between resonances (0.16
    /// against 0.92), and the field round the ring's mean circle holds more
    /// than five times the energy (11×).
    #[test]
    fn a_ring_on_its_resonance_fills_and_empties_the_fiber_past_it() {
        let measure = |frequency: f64| {
            let ring = Harmonic::run(
                &ring_resonator_with(frequency, true),
                0.08,
                30.0,
                frequency,
                4.0,
            );
            let bus = Harmonic::run(
                &ring_resonator_with(frequency, false),
                0.08,
                10.0,
                frequency,
                4.0,
            );
            let past = |scene: &Harmonic| {
                guided_power(scene, Point2::new(0.8, -0.55), Point2::new(0.0, 1.0))
            };
            let centre = Point2::new(0.0, -0.5 + RING_GAP + 0.5 * CORE_WIDTH + RING_RADIUS);
            let stored = (0..200)
                .map(|index| {
                    let angle = std::f64::consts::TAU * (index as f64 + 0.5) / 200.0;
                    let (a, b) = ring
                        .interpolated(centre + Point2::new(angle.cos(), angle.sin()) * RING_RADIUS);
                    a * a + b * b
                })
                .sum::<f64>();
            (past(&ring) / past(&bus), stored)
        };
        let (on, filled) = measure(RING_HZ);
        let (off, idle) = measure(3.885);
        assert!(on < 0.5 * off, "on resonance {on:.3}, between {off:.3}");
        assert!(filled > 5.0 * idle, "the ring holds {:.2}×", filled / idle);
    }

    /// The acoustic whispering-gallery claims, at edge 0.08, 25 s from rest at
    /// 4 Hz, with the RMS over the scene's two probe disks. The far wall, half
    /// a turn from the source, is more than twice as loud as the centre (3.9×;
    /// 3.6× at edge 0.05). The centre's disk averages over the room's standing
    /// pattern, and a slow beat moves it: 3.9× to 7.2× from 25 to 50 s, and
    /// 2.8× to 4.1× from 3.8 to 4.2 Hz at 15 s. Without the wall, the centre is
    /// the louder (the far point hears 0.70 of it, near free space's 1/√2).
    #[test]
    fn a_curved_wall_carries_sound_round_to_its_far_side() {
        let loudness = |wall: bool| {
            let scene = Harmonic::run(&acoustic_gallery_with(wall), 0.08, 25.0, ACOUSTIC_HZ, 4.0);
            let rms = |(center, radius): (Point2, f64)| {
                let points = (0..81)
                    .map(|index| {
                        let step = Point2::new((index % 9) as f64 - 4.0, (index / 9) as f64 - 4.0);
                        center + step * (radius / 4.0)
                    })
                    .filter(|point| (*point - center).norm() <= radius)
                    .collect::<Vec<_>>();
                let sum = points
                    .iter()
                    .map(|point| {
                        let (a, b) = scene.interpolated(*point);
                        a * a + b * b
                    })
                    .sum::<f64>();
                (sum / points.len() as f64).sqrt()
            };
            rms(FAR_WALL) / rms(ROOM_CENTRE)
        };
        let walled = loudness(true);
        assert!(walled > 2.0, "the far wall hears {walled:.2}× the centre");
        let open = loudness(false);
        assert!(
            open < 1.0,
            "without the wall the far point hears {open:.2}×"
        );
    }

    /// The dielectric whispering-gallery claims, at edge 0.08, 30 s from rest,
    /// on the circle 0.05 inside the rim. On the `m = 7` resonance the RMS
    /// there is more than three times that at 2.7 Hz, between resonances (8.0×;
    /// 8.1× at edge 0.05), and `|U|` has `2m = 14` maxima round it.
    #[test]
    fn a_dielectric_disk_rings_in_fourteen_lobes_round_its_rim() {
        let round = |frequency: f64| {
            let scene = Harmonic::run(
                &dielectric_gallery_with(frequency),
                0.08,
                30.0,
                frequency,
                4.0,
            );
            (0..360)
                .map(|index| {
                    let angle = std::f64::consts::TAU * index as f64 / 360.0;
                    let (a, b) = scene
                        .interpolated(Point2::new(angle.cos(), angle.sin()) * (DISK_RADIUS - 0.05));
                    a.hypot(b)
                })
                .collect::<Vec<_>>()
        };
        let rms =
            |line: &[f64]| (line.iter().map(|v| v * v).sum::<f64>() / line.len() as f64).sqrt();
        let on = round(DISK_HZ);
        let off = round(2.7);
        assert!(
            rms(&on) > 3.0 * rms(&off),
            "the rim rings {:.2}× off resonance",
            rms(&on) / rms(&off)
        );
        let peak = on.iter().cloned().fold(0.0, f64::max);
        let maxima = (0..on.len())
            .filter(|&index| {
                let (before, here, after) = (
                    on[(index + on.len() - 1) % on.len()],
                    on[index],
                    on[(index + 1) % on.len()],
                );
                here > before && here >= after && here > 0.2 * peak
            })
            .count();
        assert_eq!(maxima, 14);
    }

    /// The fisheye claims, at edge 0.08, 10 s from rest at 3 Hz, with `|U|` on
    /// the circle 0.03 inside the rim. With the lens the rim is brightest,
    /// away from the source, at the antipode, and there more than three times
    /// what it is 45° either side (5.0×; 5.1× at edge 0.05, 4.9× at 2.5 Hz and
    /// 3.7× at 3.5 Hz). With the disk vacuum the antipode is within a fifth of
    /// its neighbours 45° away (1.04×).
    #[test]
    fn a_fisheye_images_a_rim_source_onto_the_opposite_rim() {
        let rim = |lens: bool| {
            let scene = Harmonic::run(&fisheye_with(lens), 0.08, 10.0, FISHEYE_HZ, 4.0);
            move |degrees: f64| {
                let angle = degrees.to_radians();
                let (a, b) = scene
                    .interpolated(Point2::new(angle.cos(), angle.sin()) * (FISHEYE_RADIUS - 0.03));
                a.hypot(b)
            }
        };
        let lens = rim(true);
        let focus = lens(0.0) / lens(45.0).max(lens(-45.0));
        assert!(focus > 3.0, "the image is {focus:.2}× its flanks");
        let brightest = (-29..=29)
            .map(|step| 5.0 * step as f64)
            .max_by(|a, b| lens(*a).total_cmp(&lens(*b)))
            .unwrap();
        assert_eq!(brightest, 0.0, "the far rim peaks at {brightest}°");
        let vacuum = rim(false);
        let flat = vacuum(0.0) / vacuum(45.0).max(vacuum(-45.0));
        assert!(flat < 1.2, "without the lens the antipode is {flat:.2}×");
    }

    /// The zone-plate claims, at edge 0.08, 10 s from rest at 4 Hz, with `|U|`
    /// at the focus. It exceeds 1.5× the bare plane wave there (1.68×; 1.67×
    /// at edge 0.05), and twice the field 0.4 to either side (3.4×). The
    /// axis peaks a little past the focus, 1.76× at 0.46. The plan's "twice
    /// the plane wave" is a 3D ring plate's: in 2D the zones are slits, whose
    /// contributions fall off, and more of them barely raise the focus.
    #[test]
    fn a_zone_plate_gathers_a_plane_wave_into_its_focus() {
        let field = |plate: bool| {
            let scene = Harmonic::run(&zone_plate_with(plate), 0.08, 10.0, ZONE_HZ, 4.0);
            move |y: f64| {
                let (a, b) = scene.interpolated(Point2::new(ZONE_FOCUS, y));
                a.hypot(b)
            }
        };
        let (plate, bare) = (field(true), field(false));
        let gain = plate(0.0) / bare(0.0);
        assert!(gain > 1.5, "the focus holds {gain:.2}× the plane wave");
        let flank = plate(0.4).max(plate(-0.4));
        assert!(
            plate(0.0) > 2.0 * flank,
            "the focus is {:.2}× its flanks",
            plate(0.0) / flank
        );
    }

    /// The Brewster angle of air on glass, `atan(1.5)`.
    fn brewster_angle() -> f64 {
        CORE_PERMITTIVITY.sqrt().atan()
    }

    /// The point on the reading arc a ray meeting the face at `incidence`
    /// from the normal reflects through.
    fn brewster_arc_point(incidence: f64) -> Point2 {
        brewster_image() + Point2::new(-incidence.cos(), incidence.sin()) * BREWSTER_ARC
    }

    /// The Brewster claims, at edge 0.08, 8 s from rest at 4 Hz. The reflected
    /// field is the field with the glass less the field without it, on the arc
    /// about the source's mirror image, over the direct field there. In `H_z`
    /// it is least, over rays meeting the glass from 44° to 70°, within 3° of
    /// `atan 1.5` (58°; 56° at edge 0.05). There `E_z` reflects more than five
    /// times as strongly (7.2×), and `H_z` itself more than three times as
    /// strongly at 72° (5.5×). `E_z` follows Fresnel's `|r_s|` within 15% from
    /// 20° to 72°; `H_z` ripples by about 0.06 about `|r_p|`. The plan's tilted
    /// beam could not show it: a beam this domain holds spreads over ±15°, and
    /// a plane wave across it at 56° is not clean enough to null.
    #[test]
    fn glass_reflects_nothing_at_the_brewster_angle_in_one_skin() {
        let reflection = |polarization: ElectromagneticPolarization| {
            let with = Harmonic::run(
                &brewster_with(polarization, true),
                0.08,
                8.0,
                BREWSTER_HZ,
                4.0,
            );
            let without = Harmonic::run(
                &brewster_with(polarization, false),
                0.08,
                8.0,
                BREWSTER_HZ,
                4.0,
            );
            move |degrees: f64| {
                let point = brewster_arc_point(degrees.to_radians());
                let (a, b) = with.interpolated(point);
                let (c, d) = without.interpolated(point);
                (a - c).hypot(b - d) / c.hypot(d)
            }
        };
        let (te, tm) = (
            reflection(ElectromagneticPolarization::Te),
            reflection(ElectromagneticPolarization::Tm),
        );
        let brewster = brewster_angle().to_degrees();
        let least = (44..=70)
            .map(f64::from)
            .min_by(|a, b| te(*a).total_cmp(&te(*b)))
            .unwrap();
        assert!(
            (least - brewster).abs() < 3.0,
            "H_z reflects least at {least}°, the Brewster angle is {brewster:.1}°"
        );
        assert!(
            tm(brewster) > 5.0 * te(brewster),
            "at the Brewster angle E_z reflects {:.3}, H_z {:.3}",
            tm(brewster),
            te(brewster)
        );
        assert!(
            te(72.0) > 3.0 * te(brewster),
            "H_z reflects {:.3} at 72° and {:.3} at the Brewster angle",
            te(72.0),
            te(brewster)
        );
    }

    /// The frustrated-TIR claims, at edge 0.08, 8 s from rest at 4 Hz, with the
    /// power in the fiber's mode from x = 0.6 to 0.9, under the block's far
    /// end, against the same run with the block vacuum on the same mesh.
    /// Over the gallery's gap of 0.05 the fiber keeps under 70% (60%). The
    /// leak's rate, `−ln` of what it keeps, falls from a gap of 0.05 to 0.1 by
    /// `e^{−2γ·0.05} = 0.113`, `γ = 21.8`, within 30% (0.096; 0.105 at edge
    /// 0.05). Over 0.15 the fiber keeps more than 99% (99.65%).
    #[test]
    fn a_nearby_block_frustrates_a_fibers_total_internal_reflection() {
        let kept = |gap: f64| {
            let glass = Harmonic::run(&tunnelling_with(gap, true), 0.08, 8.0, TUNNEL_HZ, 4.0);
            let vacuum = Harmonic::run(&tunnelling_with(gap, false), 0.08, 8.0, TUNNEL_HZ, 4.0);
            (0..=6)
                .map(|index| {
                    let point = Point2::new(0.6 + 0.05 * index as f64, -0.5);
                    let across = Point2::new(0.0, 1.0);
                    guided_power(&glass, point, across) / guided_power(&vacuum, point, across)
                })
                .sum::<f64>()
                / 7.0
        };
        let (close, farther, far) = (kept(TUNNEL_GAP), kept(0.1), kept(0.15));
        assert!(close < 0.7, "over 0.05 the fiber keeps {close:.3}");
        let gamma = std::f64::consts::TAU * TUNNEL_HZ * (1.3234_f64.powi(2) - 1.0).sqrt();
        let expected = (-2.0 * gamma * 0.05).exp();
        let measured = farther.ln() / close.ln();
        assert!(
            (measured / expected - 1.0).abs() < 0.3,
            "the leak fell by {measured:.3}, e^(-2γ·0.05) = {expected:.3}"
        );
        assert!(far > 0.99, "over 0.15 the fiber keeps {far:.4}");
    }
}
