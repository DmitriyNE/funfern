//! Built-in examples authored directly in the unified topology model.

use crate::document::{
    FarFieldSettings, LineProbeQuantity, LineProbeRepresentation, MaterialOverlay,
    MaterialProperty, PresentationSettings, ProbeId, ProbeReadout, ProbeReadouts,
    ProbeSamplingPreset, TransferReference, VectorOverlay, VectorOverlayStyle,
};
use crate::topology_editor::{
    TopologyBoundaryProbeTarget, TopologyDocument, TopologyDocumentModel, TopologyProbeDefinition,
    TopologyProbeTarget,
};
use funfern_core::*;
use std::sync::OnceLock;

pub struct TopologyExample {
    pub group: ExampleGroup,
    pub name: &'static str,
    pub description: &'static str,
    pub document: TopologyDocument,
}

/// The gallery's sections. The catalog runs through them in this order, one
/// section after another, so stepping from one example to the next walks the
/// gallery as it is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExampleGroup {
    Basics,
    InterfacesAndMedia,
    LensesAndImaging,
    GuidesAndCrystals,
    Resonators,
    TimeVaryingMedia,
    NonlinearAndSelfOrganizing,
}

impl ExampleGroup {
    pub const ALL: [Self; 7] = [
        Self::Basics,
        Self::InterfacesAndMedia,
        Self::LensesAndImaging,
        Self::GuidesAndCrystals,
        Self::Resonators,
        Self::TimeVaryingMedia,
        Self::NonlinearAndSelfOrganizing,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Basics => "Basics",
            Self::InterfacesAndMedia => "Interfaces and media",
            Self::LensesAndImaging => "Lenses and imaging",
            Self::GuidesAndCrystals => "Guides and crystals",
            Self::Resonators => "Resonators",
            Self::TimeVaryingMedia => "Time-varying media",
            Self::NonlinearAndSelfOrganizing => "Nonlinear and self-organizing",
        }
    }
}

pub fn catalog() -> &'static [TopologyExample] {
    static CATALOG: OnceLock<Vec<TopologyExample>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        vec![
            example(
                ExampleGroup::Basics,
                "Obstacle over a mirror",
                "A point source scatters from a rounded obstacle above a reflecting floor.",
                obstacle_over_a_mirror(),
            ),
            example(
                ExampleGroup::Basics,
                "Echo comb",
                "A plane-wave pulse every 16 s, flat from 0.5 to 3.5 Hz, runs down the upper of \
                 two arms to a mirror half a unit past a probe, which hears it again a second \
                 later. Divided by the probe in the open lower arm, it reads the pulse and its \
                 echo together, 2|cos(πfτ)| with τ = 1 s: a comb whose teeth, twice the field, \
                 stand at every whole hertz and whose gaps, none of it, fall halfway between. \
                 Any reflector behind a probe combs what it reads this way.",
                echo_comb(),
            ),
            example(
                ExampleGroup::Basics,
                "Double slit",
                "A source boxed in black walls lights two slits; the screen and the far field show \
                 the fringes.",
                double_slit(),
            ),
            example(
                ExampleGroup::Basics,
                "Obstacle array",
                "A point source drives multiple scattering through eight reflecting obstacles.",
                obstacle_array(),
            ),
            example(
                ExampleGroup::Basics,
                "Phased array",
                "Five compact region sources use a phase ramp to steer a radiated beam.",
                phased_array(),
            ),
            example(
                ExampleGroup::Basics,
                "Talbot carpet",
                "A plane wave through a grating of period 0.4 at 4 Hz: behind it the grating's \
                 image comes back at the Talbot distance, 1.14 rather than the paraxial 1.28, \
                 bright behind the slits, and halfway there shifted by half a period, bright \
                 behind the bars.",
                talbot(),
            ),
            example(
                ExampleGroup::InterfacesAndMedia,
                "Anisotropic crystal",
                "A rotated directional inclusion turns circular wavefronts into ellipses.",
                anisotropic_crystal(),
            ),
            example(
                ExampleGroup::InterfacesAndMedia,
                "Brewster angle",
                "A point source in front of glass, ε = 2.25, in the H_z skin: each ray meets the \
                 glass at its own angle, and the one meeting it at the Brewster angle, atan 1.5 = \
                 56°, reflects nothing, so the fringes the reflection makes with the direct wave \
                 fade out along its direction. Switch to E_z and every ray reflects, at 56° seven \
                 times as strongly.",
                brewster(),
            ),
            example(
                ExampleGroup::InterfacesAndMedia,
                "Etalon",
                "A ceramic slab, ε = 9, a sixth thick across the upper of two arms, lit by a \
                 plane-wave pulse every 16 s flat from 0.5 to 3.5 Hz. Each face reflects half \
                 the field, yet the slab passes all of it wherever a round trip inside is a \
                 whole number of periods, at every whole hertz, and 0.6 of it halfway between: \
                 a Fabry-Pérot etalon. The probe behind it, divided by the one at the same place \
                 in the empty lower arm, reads the fringes.",
                etalon(),
            ),
            example(
                ExampleGroup::InterfacesAndMedia,
                "Skin depth",
                "A plane wave at 3 Hz meets a slab whose electric loss is twice its angular \
                 frequency: inside, the wave falls by e every 0.068 while its crests stand 0.26 \
                 apart, exactly as k = (ω/c)√(1 − iγ/ω) says, not the good conductor's shortcut, \
                 and about a hundredth of the field gets through.",
                skin_depth(),
            ),
            example(
                ExampleGroup::InterfacesAndMedia,
                "Plasma skin depth",
                "The skin-depth slab as a cold plasma whose cutoff, 4 Hz, lies above the 3 Hz \
                 wave: the field reaching into it falls by e every 0.060, κ = √(ωp² − ω²)/c, \
                 but it stands there without a travelling phase and takes no power, so the \
                 wave in front stands with nodes near zero, all of it reflected. Compare the \
                 lossy slab, which absorbs.",
                plasma_skin_depth(),
            ),
            example(
                ExampleGroup::InterfacesAndMedia,
                "Plasma group delay",
                "A Gaussian pulse at 3 Hz crosses 1.3 of plasma cutting off at 2 Hz in the upper \
                 of two arms while its twin crosses vacuum in the lower. The pulse's energy runs \
                 at the group velocity, c√(1 − (2/3)²) = 0.75c, and arrives 0.45 s after its \
                 twin, while its crests run through it at the phase velocity, 1.34c, faster \
                 than light: the two multiply to c², and the crests slide forward through the \
                 envelope as it crawls.",
                plasma_delay(),
            ),
            example(
                ExampleGroup::InterfacesAndMedia,
                "Plasma mirror",
                "A plane wave climbs a plasma whose cutoff rises along the channel, stands in \
                 front of the point where the cutoff meets its frequency, and never passes it.",
                plasma_mirror(),
            ),
            example(
                ExampleGroup::LensesAndImaging,
                "Material lens",
                "A TM electric-field source illuminates a slower dielectric region with absorbing edges.",
                material_lens(),
            ),
            example(
                ExampleGroup::LensesAndImaging,
                "GRIN collimator",
                "A quarter-pitch graded-index glass rod in the E_z skin, index 1.6 on its axis \
                 falling to 1 at its sides, turns a point source on one face into a collimated \
                 beam leaving the other.",
                grin_rod(),
            ),
            example(
                ExampleGroup::LensesAndImaging,
                "Luneburg lens",
                "A TE plane wave arriving at 30° focuses on the far rim of a radial-index lens, \
                 wherever it comes from.",
                luneburg_lens(),
            ),
            example(
                ExampleGroup::LensesAndImaging,
                "Maxwell's fisheye",
                "A disk whose index falls from 2 at its centre to 1 at its rim, n = 2/(1 + (r/R)²): \
                 every ray a source on the rim sends inward curves round to the opposite point, \
                 so the far rim lights up there five times brighter than 45° either side. Set \
                 the profile to 1 and the far rim is no brighter anywhere.",
                fisheye(),
            ),
            example(
                ExampleGroup::LensesAndImaging,
                "Ellipse flash",
                "A room inside a reflecting ellipse, flashed at one focus every 10 s. Every path \
                 from one focus to the wall and on to the other is the same length, twice the \
                 semi-major axis, so all the flash's echoes reach the far focus together, 1.9 s \
                 after it left, and gather there into a peak nine times the direct flash that \
                 passed 0.9 s before. A quarter beside the focus they arrive spread out, at a \
                 seventh of that peak. A lithotripter's reflector focuses a shock wave on a \
                 kidney stone this way.",
                ellipse_flash(),
            ),
            example(
                ExampleGroup::LensesAndImaging,
                "Fresnel zone plate",
                "A plane wave meets a screen of reflecting strips open over the odd Fresnel zones \
                 for a focus 0.4 behind it: the waves from the open zones arrive there in step \
                 and gather into a spot 1.7 times the plane wave's amplitude, nearly three times \
                 its energy, and over three times the field 0.4 to either side.",
                zone_plate(),
            ),
            example(
                ExampleGroup::GuidesAndCrystals,
                "Bent fiber",
                "A glass fiber, single-mode at 4 Hz, carries its mode round a quarter turn of \
                 radius 0.5 and delivers about three quarters of it to the top. Round the bend \
                 the mode's outer flank would have to outrun the light outside, so it sheds a \
                 beam off tangentially; drag the bend tighter and it sheds more.",
                bent_fiber(),
            ),
            example(
                ExampleGroup::GuidesAndCrystals,
                "Frustrated total internal reflection",
                "A glass fiber holds its light by total internal reflection, and outside the core \
                 the field dies away within a few hundredths. Lower a glass block to 0.05 above it \
                 and the reflection is frustrated: the light tunnels across the gap and leaves \
                 into the block as a tilted beam, draining 40% of the fiber's power. Raise the \
                 block to 0.15 and under a hundredth tunnels.",
                tunnelling(),
            ),
            example(
                ExampleGroup::GuidesAndCrystals,
                "Photonic crystal",
                "A plane-wave pulse every 20 s, flat from 0.8 to 3 Hz, meets five columns of \
                 ceramic rods, ε = 9, in a square lattice filling the upper of two arms; the \
                 lower arm is empty, the reference, as in a double-beam spectrometer. The probe \
                 behind the crystal, divided by the one at the same place in the reference arm, \
                 reads the crystal's transmission: under a tenth from 1.5 to 2.1 Hz, inside the \
                 band gap its lattice puts between 1.375 and 2.225 Hz, against 1.0 at 1 Hz and \
                 0.82 at 2.5 Hz either side. The readout averages 20 s segments, so it fills \
                 after the first.",
                photonic_crystal(),
            ),
            example(
                ExampleGroup::GuidesAndCrystals,
                "Crystal bend",
                "The photonic crystal with a channel of missing rods that turns a right angle. \
                 At 1.85 Hz, in the band gap, the wave cannot enter the crystal, so it follows \
                 the channel round the corner and out through the top, delivering about nine \
                 tenths of what a straight channel does.",
                crystal_bend(),
            ),
            example(
                ExampleGroup::GuidesAndCrystals,
                "Disordered crystal",
                "The photonic crystal's fifty rods scattered at random, a fixed seed, and lit at \
                 2.5 Hz, where the ordered crystal lets 80% of the power through: scattered from \
                 rod to rod, about an eighth gets through, nearly all of it thrown off the \
                 straight path. A slab this thin cannot show Anderson localization itself; this \
                 is the scattering that leads to it.",
                disordered_crystal(),
            ),
            example(
                ExampleGroup::Resonators,
                "Drum modes",
                "A clamped round membrane driven at 1.36 Hz, j₂₁/(2πa), its (2,1) mode: it \
                 stands in four lobes, rising and falling in turn, with two still diameters \
                 between them that keep under a tenth of the lobes' motion. Drive it 0.01 Hz \
                 off and the lobes fall.",
                drum(),
            ),
            example(
                ExampleGroup::Resonators,
                "Struck drum",
                "The clamped drum knocked once every 20 s at its centre: it rings at all its \
                 round modes at once, and a probe beside the centre hears them at 0.64, 1.46, \
                 2.30 and 3.13 Hz, the zeros of J₀ over 2πa, in the ratios 1 : 2.30 : 3.60 : \
                 4.90. A string's overtones would stand at whole multiples of its fundamental; \
                 at 2, 3 and 4 times it the drum is all but silent.",
                struck_drum(),
            ),
            example(
                ExampleGroup::Resonators,
                "Acoustic whispering gallery",
                "A 4 Hz source just inside a round room's reflecting wall, open on the left: \
                 the sound clings to the wall all the way round, so the far wall, half a turn \
                 away and twice as far as the centre, is several times louder than the centre. \
                 Delete the wall and the centre is the louder.",
                acoustic_gallery(),
            ),
            example(
                ExampleGroup::Resonators,
                "Dielectric whispering gallery",
                "A dielectric disk, ε = 4, with a 2.55 Hz source just inside its rim: at this, \
                 one of its whispering-gallery resonances, total internal reflection holds the \
                 wave running round inside the rim, and over half a minute it builds to fourteen \
                 lobes eight times the field at 2.7 Hz, between resonances.",
                dielectric_gallery(),
            ),
            example(
                ExampleGroup::Resonators,
                "Plasma whispering gallery",
                "A vacuum disk walled by a plasma, driven at 3.1 Hz below the plasma's cutoff: \
                 ten lobes of a whispering-gallery mode build up around the rim, and outside it \
                 the field dies within a few hundredths. Drive it at 5 Hz, above the cutoff, and \
                 the wall turns transparent.",
                whispering_gallery(),
            ),
            example(
                ExampleGroup::Resonators,
                "Cavity filter",
                "Two ceramic plates, ε = 9, each a quarter wave thick at 2 Hz and half a wave \
                 apart, across the upper of two arms, lit by a plane-wave pulse every 24 s flat \
                 from 1 to 3 Hz. Each plate alone reflects 0.8 of the field, yet together they \
                 pass all of it at 2 Hz, where the space between them resonates, over a band \
                 0.22 Hz wide, and 0.29 of it at 1.5 and 2.5 Hz. The probe behind them, divided \
                 by the one in the empty lower arm, reads the filter.",
                cavity_filter(),
            ),
            example(
                ExampleGroup::Resonators,
                "Ring resonator",
                "A glass ring beside a glass fiber, driven at 3.975 Hz, one of the ring's \
                 resonances: over half a minute the ring fills to ten times its field between \
                 resonances, and past it the fiber keeps under a fifth of its power, the rest \
                 shed from the ring's bend. Tune the source to 3.885 Hz, between resonances, \
                 and the wave runs past.",
                ring_resonator(),
            ),
            example(
                ExampleGroup::TimeVaryingMedia,
                "Parametric pump",
                "A slab pumped at twice the source frequency amplifies what it transmits, by an \
                 amount the pump's phase sets.",
                pumped_slab(),
            ),
            example(
                ExampleGroup::TimeVaryingMedia,
                "Pumped drum",
                "The struck drum with its membrane's density pumped by 40% at twice its \
                 fundamental, 1.28 Hz, for 8 s after each knock. The pump feeds the fundamental \
                 alone: it grows at dω/4 less its loss, 0.35 per second, where unpumped it rings \
                 down at 0.05, and its line comes to stand over twice any overtone's. A probe \
                 on the first overtone's still circle hears the fundamental swell without it.",
                pumped_drum(),
            ),
            example(
                ExampleGroup::TimeVaryingMedia,
                "Temporal slab",
                "A 1.5 Hz pulse runs into glass, ε = 4, and slows to half the wave speed. Once it \
                 is inside, the glass's permittivity drops to 1 at once, for 2 s, every 6 s. \
                 Across a sudden change in time the fields D and B carry over, so the pulse \
                 keeps its wavelength and doubles its frequency, and it splits: one pulse runs \
                 on with three times its field, and one, time-reflected, runs back with as much \
                 field as it had, over half the time. With the glass held, nothing comes back.",
                temporal_slab(),
            ),
            example(
                ExampleGroup::TimeVaryingMedia,
                "Chopper",
                "A lossy slab across a channel shuts a 3 Hz wave out, passing under 1% of it, \
                 until a gated drive cuts its loss a hundredfold for 1 s in every 2 s. Behind \
                 it the wave comes in bursts, and its spectrum is the carrier with sidebands \
                 every half hertz: each first sideband carries a third of the open wave's \
                 field, 1/π, as the Fourier series of a shutter open half the time says, and \
                 the carrier keeps a little under half, the slab refilling at the wave's speed \
                 after each opening.",
                chopper(),
            ),
            example(
                ExampleGroup::TimeVaryingMedia,
                "Time crystal",
                "A slab whose permittivity steps up and down once a second splits the wave into \
                 sidebands; its sharp edges reach three steps out.",
                time_crystal_slab(),
            ),
            example(
                ExampleGroup::TimeVaryingMedia,
                "Travelling modulation",
                "A modulation running with the wave converts it to higher frequencies; mirrored, \
                 against the wave, it barely does.",
                travelling_slab(),
            ),
            example(
                ExampleGroup::TimeVaryingMedia,
                "Doppler mirror",
                "A 1 Hz plane wave meets a slab whose permittivity is a grating running \
                 toward it at half the wave speed. The grating reflects it as a moving mirror \
                 would, at (c + v)/(c − v) = 3 times its frequency: the probe in front hears \
                 3 Hz about as loud as the wave itself. The reflection carries more power than \
                 the wave lost, since the grating does work on it, but it returns one wave \
                 quantum for each one it takes. At rest the grating reflects nothing.",
                doppler_mirror(),
            ),
            example(
                ExampleGroup::TimeVaryingMedia,
                "Parametric fiber amplifier",
                "A graded-index fiber whose permittivity is pumped at twice the signal's \
                 frequency by a wave running with it amplifies the signal several times over by \
                 the far end; shift the pump's phase by half a turn and the same signal is squeezed. \
                 Stop the pump's wave (wavenumber 0) and the fiber oscillates on its own. The \
                 fiber is slightly dispersive, as real ones are, which keeps the pump from also \
                 driving the signal's higher harmonics, and a short-wave loss, a weak viscosity \
                 on the field's gradient, damps what of them the mesh is too coarse to carry.",
                fiber_amplifier(),
            ),
            example(
                ExampleGroup::NonlinearAndSelfOrganizing,
                "Kerr slab",
                "A strong source drives a Kerr slab: the wave slows where it is strong, and the \
                 receiver hears the source's third harmonic. A short-wave loss, a weak viscosity \
                 on the field's gradient, damps the higher harmonics the mesh is too coarse to \
                 carry.",
                kerr_slab(),
            ),
            example(
                ExampleGroup::NonlinearAndSelfOrganizing,
                "Spatial soliton",
                "A 4 Hz beam enters a slab of saturable Kerr medium whose index at 4 Hz is 1, \
                 so a weak wave does not see it. Weak, the beam spreads to three times its \
                 width by the far face; strong, it raises the index where it is brightest, and \
                 that holds it together, breathing a little, at half the weak beam's width: a \
                 spatial soliton. Turn the launcher down to watch it spread. The slab is \
                 slightly dispersive, as real media are, which keeps its third harmonic, the \
                 fine ripple on the strong beam, small.",
                spatial_soliton(),
            ),
            example(
                ExampleGroup::NonlinearAndSelfOrganizing,
                "Josephson line",
                "A junction line held at a constant voltage V on one end sheds one fluxon per \
                 turn of its phase; each runs down the line as a kink in the integrated field, \
                 and a probe down the line hears them as a tone at V/2π, the Josephson \
                 frequency.",
                josephson_line(),
            ),
            example(
                ExampleGroup::NonlinearAndSelfOrganizing,
                "Symmetry breaking",
                "A medium resting on the top of a double well is tipped by faint frozen noise: \
                 it falls into both wells in patches, then the walls between them move until \
                 one well holds everything. Shown in the integrated field.",
                symmetry_breaking(),
            ),
            example(
                ExampleGroup::NonlinearAndSelfOrganizing,
                "Pinned domain wall",
                "A double-well medium falls into opposite wells either side of a wall that forms \
                 off-centre. Two round bumps narrow the channel: the wall slides to their waist \
                 and stays, and dragging the bumps drags it along.",
                pinned_domain_wall(),
            ),
            example(
                ExampleGroup::NonlinearAndSelfOrganizing,
                "Self-sustained emitter",
                "A disk of van der Pol oscillators starts from a faint seed and rings at its own \
                 3 Hz cutoff, sending rings through a plasma, where a probe hears the one tone. \
                 Raise the plasma's cutoff above 3 Hz and the rings stop: the disk still rings, \
                 but its tone cannot leave.",
                emitter(),
            ),
        ]
    })
}

fn example(
    group: ExampleGroup,
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
        group,
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
    /// It switches on as a cosine: started on a sine, its plane wave carries
    /// a static part as large as itself, which a channel keeps (see
    /// `SWITCH_ON_PHASE`).
    fn launcher(&mut self, x: f64, frequency: f64, amplitude: f64) -> RegionId {
        let background = self.scene.regions[0].material;
        let region = self.strip(x - 0.03, x + 0.03, background);
        self.scene.volume_sources.push(VolumeSource {
            region,
            enabled: true,
            profile: ScalarField::constant(1.0),
            parameters: vec![],
            signal: TimeSignal::harmonic(0.0, amplitude, frequency, SWITCH_ON_PHASE),
        });
        region
    }

    fn junction(&mut self, point: Point2) -> TopologyVertexId {
        let id = TopologyVertexId(self.next_vertex);
        self.next_vertex += 1;
        self.scene.geometry.vertices.push(TopologyVertex {
            id,
            location: TopologyVertexLocation::Interior(point),
        });
        id
    }

    /// A polyline through `points`, one span of `behavior` per piece, whose
    /// breakpoints sit on `vertices` where given.
    fn pinned_polyline(
        &mut self,
        points: Vec<Point2>,
        vertices: &[Option<TopologyVertexId>],
        behavior: SpanBehavior,
    ) -> CurveId {
        assert_eq!(points.len(), vertices.len());
        let behaviors = vec![behavior; points.len() - 1];
        let curve = self.open_curve(OpenCubicSpline::polyline(points).unwrap(), &behaviors);
        let authored = self
            .scene
            .geometry
            .curves
            .iter_mut()
            .find(|candidate| candidate.id == curve)
            .unwrap();
        for (node, vertex) in authored.nodes.iter_mut().zip(vertices) {
            node.vertex = *vertex;
        }
        curve
    }

    /// A region of `material` for the face the outer anchor `(side, x)`
    /// names, `x` along the floor or the ceiling.
    fn face_on(&mut self, side: OuterSide, x: f64, material: MaterialId) -> RegionId {
        let domain = self.scene.geometry.domain;
        let fraction = match side {
            OuterSide::Top => (domain.max_x - x) / domain.width(),
            _ => (x - domain.min_x) / domain.width(),
        };
        let region = RegionId(self.next_region);
        self.next_region += 1;
        self.scene.regions.push(Region {
            id: region,
            material,
            frame: MaterialFrame::world(),
        });
        self.scene.face_assignments.push(AuthoredFaceAssignment {
            anchor: FaceAnchor::Outer { side, fraction },
            region: Some(region),
        });
        region
    }

    /// The channel as two arms, one above the other, split along `y = 0` by
    /// a reflecting wall welded to both ends of the domain, which a wave
    /// uniform in `y` does not see. One launcher at `x` lights both: its two
    /// halves, a strip 0.06 wide across each arm, carry `signal`. The upper
    /// arm is cut from the split to the ceiling at the edges of each of
    /// `layers`, `(x0, x1, material)`, which fill it between them; the lower
    /// arm is empty, the reference. With a `mirror`, a reflecting wall closes
    /// the upper arm there, and what lies past it is cut away. Every face is
    /// named from the floor or the ceiling, so the background's own anchor,
    /// on the floor at `x = 0`, names the lower arm's face past the launcher.
    fn arms(
        &mut self,
        x: f64,
        signal: TimeSignal,
        layers: &[(f64, f64, MaterialId)],
        mirror: Option<f64>,
    ) {
        let domain = self.scene.geometry.domain;
        let background = self.scene.regions[0].material;
        let (left, right) = (x - 0.03, x + 0.03);
        assert!(right < 0.0, "a launcher left of the centre");
        let mut cuts = layers
            .iter()
            .flat_map(|(x0, x1, _)| [*x0, *x1])
            .chain(mirror)
            .collect::<Vec<_>>();
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        assert!(cuts.iter().all(|cut| *cut > right && *cut < domain.max_x));
        let mut crossings = vec![left, right];
        crossings.extend(&cuts);
        let junctions = crossings
            .iter()
            .map(|cut| (*cut, self.junction(Point2::new(*cut, 0.0))))
            .collect::<Vec<_>>();
        let start = self.outer_vertex(OuterSide::Left, 0.5);
        let end = self.outer_vertex(OuterSide::Right, 0.5);
        let mut points = vec![Point2::new(domain.min_x, 0.0)];
        let mut vertices = vec![Some(start)];
        for (cut, junction) in &junctions {
            points.push(Point2::new(*cut, 0.0));
            vertices.push(Some(*junction));
        }
        points.push(Point2::new(domain.max_x, 0.0));
        vertices.push(Some(end));
        self.pinned_polyline(points, &vertices, SpanBehavior::REFLECTING);
        for (cut, junction) in &junctions {
            let top = self.outer_vertex(OuterSide::Top, (domain.max_x - cut) / domain.width());
            if *cut == left || *cut == right {
                let bottom =
                    self.outer_vertex(OuterSide::Bottom, (cut - domain.min_x) / domain.width());
                self.pinned_polyline(
                    vec![
                        Point2::new(*cut, domain.min_y),
                        Point2::new(*cut, 0.0),
                        Point2::new(*cut, domain.max_y),
                    ],
                    &[Some(bottom), Some(*junction), Some(top)],
                    SpanBehavior::Transmitting,
                );
            } else {
                self.pinned_polyline(
                    vec![Point2::new(*cut, 0.0), Point2::new(*cut, domain.max_y)],
                    &[Some(*junction), Some(top)],
                    if Some(*cut) == mirror {
                        SpanBehavior::REFLECTING
                    } else {
                        SpanBehavior::Transmitting
                    },
                );
            }
        }
        // The lower arm: behind the launcher, the launcher, and the face past
        // it, which is the background's own.
        let behind = 0.5 * (domain.min_x + left);
        self.face_on(OuterSide::Bottom, behind, background);
        let lower = self.face_on(OuterSide::Bottom, x, background);
        let upper = self.face_on(OuterSide::Top, x, background);
        for region in [upper, lower] {
            self.scene.volume_sources.push(VolumeSource {
                region,
                enabled: true,
                profile: ScalarField::constant(1.0),
                parameters: vec![],
                signal,
            });
        }
        let mut edges = vec![domain.min_x, left, right];
        edges.extend(&cuts);
        edges.push(domain.max_x);
        for pair in edges.windows(2) {
            if pair[0] == left {
                continue;
            }
            let middle = 0.5 * (pair[0] + pair[1]);
            if mirror.is_some_and(|mirror| pair[0] >= mirror) {
                self.scene.face_assignments.push(AuthoredFaceAssignment {
                    anchor: FaceAnchor::Outer {
                        side: OuterSide::Top,
                        fraction: (domain.max_x - middle) / domain.width(),
                    },
                    region: None,
                });
                continue;
            }
            let material = layers
                .iter()
                .find(|(x0, x1, _)| *x0 <= pair[0] && pair[1] <= *x1)
                .map_or(background, |layer| layer.2);
            self.face_on(OuterSide::Top, middle, material);
        }
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
                region_names: Default::default(),
            },
            presentation: PresentationSettings::default(),
            readouts: ProbeReadouts::default(),
        }
    }
}

fn source(position: Point2, frequency: f64, amplitude: f64, width: f64) -> PointSource {
    PointSource {
        enabled: true,
        position,
        width,
        region: BACKGROUND_REGION,
        signal: TimeSignal::harmonic(0.0, amplitude, frequency, SWITCH_ON_PHASE),
    }
}

/// The gain of a gallery scene's arrows. Their scale is the 90th percentile
/// of what is on screen, and with a source in view most of a domain sits well
/// under it: at the default gain of 1 most arrows were a few pixels long.
const ARROW_GAIN: f32 = 2.0;

/// Arrows of the power flow: where it goes is what the scene is about.
fn power_flow(presentation: &mut PresentationSettings) {
    presentation.vector_overlay = VectorOverlay::RelativeEnergyFlow;
    presentation.vector_overlay_gain = ARROW_GAIN;
}

/// The distance between a gallery scene's streamlines, in screen pixels. The
/// application's default spacing is on the sparse side for a scene judged at
/// its own launch; the defaults are due a retuning of their own.
const LINE_SPACING: f32 = 40.0;

/// Streamlines of the power flow: the route it takes, through a lens, round a
/// corner or across a gap, is what the scene is about.
fn streamlines(presentation: &mut PresentationSettings) {
    power_flow(presentation);
    presentation.vector_overlay_style = VectorOverlayStyle::Streamlines;
    presentation.vector_overlay_density = LINE_SPACING;
}

/// The seconds a readout keeps: a probe on a mode that takes that long to
/// build up shows all of it.
const WHOLE_HISTORY: f64 = 10.0;

/// A gallery probe opens on what its claim reads and nothing else. A point
/// probe: its field, over `span` seconds.
fn field_readout(span: f64) -> ProbeReadout {
    ProbeReadout {
        span,
        field: true,
        ..ProbeReadout::blank()
    }
}

/// A point probe whose claim is about lines: its field over `span` seconds,
/// with the spectrum of that span under it, linear, up to `max_hz`.
fn spectrum_readout(span: f64, max_hz: f64) -> ProbeReadout {
    ProbeReadout {
        field_spectrum: true,
        spectrum_max_hz: max_hz,
        ..field_readout(span)
    }
}

/// A point probe whose claim is a frequency response: its field, with its
/// transfer from the probe `reference`, linear up to `max_hz`, averaged
/// over segments `segment` seconds long.
fn transfer_readout(reference: ProbeId, max_hz: f64, segment: f64) -> ProbeReadout {
    ProbeReadout {
        spectrum_max_hz: max_hz,
        transfer_from: Some(TransferReference::Probe(reference)),
        transfer_segment: segment,
        ..field_readout(4.0)
    }
}

/// A launcher's sinc pulse, flat from `low` to `high` Hz, every `repeat`
/// seconds or once for zero.
fn sinc_pulse(low: f64, high: f64, repeat: f64) -> TimeSignal {
    TimeSignal::pulsed(
        [0.0, 40.0, 0.5 * (low + high), 0.0],
        PulseEnvelope::Sinc {
            bandwidth_hz: 0.5 * (high - low),
            lobes: 4,
        },
        0.1,
        repeat,
    )
}

/// Where a scene in arms (`Builder::arms`) listens: behind its sample in the
/// upper arm, and at the same place in the empty lower one.
const ARM_BEHIND: Point2 = Point2::new(0.75, 0.5);
const ARM_REFERENCE: Point2 = Point2::new(0.75, -0.5);

/// The two probes of a scene in arms: `behind`, which opens on `readout`,
/// and "Reference", on its field.
fn arm_probes(document: &mut TopologyDocument, behind: &str, readout: ProbeReadout) {
    arm_probes_at(document, behind, ARM_BEHIND.x, readout);
}

/// `arm_probes` at `x` along the arms.
fn arm_probes_at(document: &mut TopologyDocument, behind: &str, x: f64, readout: ProbeReadout) {
    for (id, name, color, point) in [
        (1, behind, [91, 220, 194], Point2::new(x, ARM_BEHIND.y)),
        (
            2,
            "Reference",
            [248, 196, 112],
            Point2::new(x, ARM_REFERENCE.y),
        ),
    ] {
        document.model.probes.push(TopologyProbeDefinition {
            id: ProbeId(id),
            name: name.into(),
            color,
            enabled: true,
            target: TopologyProbeTarget::Point(point),
        });
    }
    document.readouts.set_probe(ProbeId(1), readout);
    document.readouts.set_probe(ProbeId(2), field_readout(4.0));
}

/// A line or boundary probe: the average energy density along it, the profile
/// a fringe, a focus, a decay or a beam's width is read from.
fn profile_readout() -> ProbeReadout {
    ProbeReadout::blank().with_line_plot(
        LineProbeQuantity::MeanEnergy,
        LineProbeRepresentation::Arclength,
    )
}

/// An area probe: the total energy it holds, or its mean energy density.
fn area_readout(total: bool, span: f64) -> ProbeReadout {
    ProbeReadout {
        span,
        area_total_energy: total,
        area_mean_energy: !total,
        ..ProbeReadout::blank()
    }
}

/// The far field: its polar pattern, where the lobes and zeros are read.
fn polar_readout() -> ProbeReadout {
    ProbeReadout {
        far_polar: true,
        ..ProbeReadout::blank()
    }
}

/// Outgoing on every side but the floor.
fn reflecting_floor() -> OuterBoundaryConditions {
    let mut boundaries = OuterBoundaryConditions::default();
    boundaries.sides[OuterSide::Bottom.index()] = OuterBoundaryCondition::Reflecting;
    boundaries
}

fn obstacle_over_a_mirror() -> TopologyDocument {
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
    document.readouts.set_probe(ProbeId(1), field_readout(2.0));
    document
}

/// The guide scene's obstacle to move, its region to repaint, the curve
/// round that region, and the material waiting for it.
pub const GUIDE_OBSTACLE: CurveId = CurveId(1);
pub const GUIDE_REGION: RegionId = RegionId(2);
pub const GUIDE_REGION_CURVE: CurveId = CurveId(2);
pub const GUIDE_GLASS: MaterialId = MaterialId(2);

/// The scene the guided tour opens on, not a gallery entry: a source, a
/// right-hand wall that reflects until the tour has it made outgoing, a
/// round obstacle to select and transform, and a round region in the
/// background material with glass in the library for it. In the TM skin,
/// where glass is an optical thing: ε 2.25 and μ 1, an index of 1.5. Quick
/// to mesh, so the tour runs on any machine.
pub fn guide_scene() -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.outer_boundaries.sides[OuterSide::Right.index()] =
        OuterBoundaryCondition::Reflecting;
    builder.scene.materials.push(Material {
        id: GUIDE_GLASS,
        name: "Glass".into(),
        mass_density: ScalarField::constant(2.25),
        stiffness: ScalarField::constant(1.0),
        damping: ScalarField::constant(0.0),
        axis_ratio: ScalarField::constant(1.0),
        parameters: vec![],
        color: [61, 116, 139],
        ..Material::default_medium()
    });
    let obstacle = builder.hole(PeriodicCubicSpline::rounded(Point2::new(0.25, 0.3), 0.18));
    debug_assert_eq!(obstacle, GUIDE_OBSTACLE);
    let (curve, region) = builder.closed(
        PeriodicCubicSpline::rounded(Point2::new(0.1, -0.45), 0.26),
        SpanBehavior::Transmitting,
        Some((
            DEFAULT_MATERIAL,
            MaterialFrame {
                attachment: MaterialFrameAttachment::FollowRegion,
                ..MaterialFrame::world()
            },
        )),
    );
    debug_assert_eq!(curve, GUIDE_REGION_CURVE);
    debug_assert_eq!(region, Some(GUIDE_REGION));
    let mut document = builder.document();
    document.model.source = source(Point2::new(-0.6, 0.15), 2.0, 18.0, 0.06);
    // The tour's words for it.
    document
        .model
        .region_names
        .insert(GUIDE_REGION, "Round region".into());
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
    document.readouts.set_probe(ProbeId(1), profile_readout());
    document.readouts.far_field = polar_readout();
    // The energy bunching into the bright fringes and skirting the dark.
    streamlines(&mut document.presentation);
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
    // The power converging behind the lens.
    streamlines(&mut document.presentation);
    document
}

/// Quarter-pitch GRIN collimator: `n = 1 + dn (1 − (y/H)²)` has paraxial
/// pitch `2πH √((1 + dn)/(2 dn))`, 2.18 for these values, so a rod a quarter
/// of that long turns a point on its entrance face into a plane wave at its
/// exit face. It is glass in the E_z skin, `ε = n²` and `μ = 1`, where the
/// field obeys the Helmholtz equation in `n` itself, so the pitch holds as
/// written; its faces reflect as glass does, about 5% on the axis.
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
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.materials.push(Material {
        id: MaterialId(2),
        name: "GRIN profile".into(),
        mass_density: ScalarField::formula("(1 + dn * max(0, 1 - (y / H)^2))^2").unwrap(),
        stiffness: ScalarField::constant(1.0),
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
    // The rod's permittivity, 1 in the vacuum: the wave speed puts the vacuum
    // at the top of the palette and paints the domain over. And the power,
    // leaving the rod straight.
    document.presentation.material_overlay = MaterialOverlay::Property(MaterialProperty::Density);
    document.presentation.material_overlay_opacity = 0.45;
    streamlines(&mut document.presentation);
    document.readouts.set_probe(ProbeId(1), profile_readout());
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
    document.presentation.material_overlay_opacity = 0.22;
    document.presentation.material_overlay_logarithmic = true;
    // Inside, the energy walks off: it runs along the arrows, not normal to
    // the elliptic wavefronts.
    power_flow(&mut document.presentation);
    document
}

/// The Luneburg lens's centre and radius, and the direction its light comes
/// from, 30° above the x axis.
const LUNEBURG_CENTRE: Point2 = Point2 { x: 0.08, y: 0.0 };
const LUNEBURG_RADIUS: f64 = 0.43;
const LUNEBURG_DEGREES: f64 = 30.0;

fn luneburg_direction() -> Point2 {
    let angle = LUNEBURG_DEGREES.to_radians();
    direction(angle)
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
    // The permittivity, 1 in the vacuum, and the rays' power bending onto
    // the focus.
    document.presentation.material_overlay = MaterialOverlay::Property(MaterialProperty::Density);
    streamlines(&mut document.presentation);
    document
        .readouts
        .set_probe(ProbeId(1), area_readout(false, 2.0));
    document
        .readouts
        .set_probe(ProbeId(2), area_readout(true, 2.0));
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
    // The beam the phase ramp steers.
    streamlines(&mut document.presentation);
    document.readouts.far_field = polar_readout();
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

/// The Kerr slab's short-wave loss α, a viscosity on the field's gradient
/// that damps the odd harmonics the mesh cannot carry. Measured on 3 October
/// 2026 at the receiver over 4 s on a fixed mesh, against α = 0: at edge
/// 0.08, where the bare 12.5 Hz line (9.1e-3) stands above the third
/// harmonic (6.7e-3) and so is mostly the mesh's, it takes 12.5 Hz to 0.13
/// and keeps 0.93 of the 2.5 Hz signal and 0.65 of the third harmonic; at
/// 0.04 it keeps 0.97 of the signal and 0.80 of the third harmonic and takes
/// 17.5 Hz to 0.44 and 22.5 Hz to 0.16. Chosen on how the scene looks.
const KERR_SLAB_SHORT_WAVE: f64 = 0.25;

fn kerr_slab_with(chi: f64, amplitude: f64) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.outer_boundaries =
        OuterBoundaryConditions::uniform(OuterBoundaryCondition::FirstOrderOutgoing);
    let mut medium = preset_material(
        2,
        "Kerr slab",
        [178, 102, 62],
        "Kerr medium",
        LawPresetRow::Mass,
        &[("kerr_chi", chi)],
    );
    medium.short_wave_loss = KERR_SLAB_SHORT_WAVE;
    builder.scene.materials.push(medium);
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
        .readouts
        .set_probe(ProbeId(1), spectrum_readout(2.0, 10.0));
    document
}

/// The pump phase the scene opens at, the top of its gain against phase;
/// tied to the source's cosine start as the fiber's is.
const PUMP_PHASE: f64 = 2.0 * SWITCH_ON_PHASE + std::f64::consts::FRAC_PI_4;

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
    document.readouts.set_probe(ProbeId(1), field_readout(2.0));
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
    // Four seconds, so lines a hertz apart stand clear of each other.
    document
        .readouts
        .set_probe(ProbeId(1), spectrum_readout(4.0, 8.0));
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

const SOLITON_HZ: f64 = 4.0;
/// The launcher's strength that makes the soliton; a hundredth of it is the
/// weak beam, which the slab does not see.
const SOLITON_AMPLITUDE: f64 = 1000.0;
/// The slab's Klein-Gordon cutoff. Without dispersion the Kerr slab's third
/// harmonic runs in step with the beam and builds up, to 35% of it on the
/// axis by 20 s with the stored energy still climbing; this cutoff, with the
/// permittivity that keeps the index 1 at 4 Hz, puts 12 Hz out of step
/// within about 0.6 and holds it to 5-13%.
const SOLITON_CUTOFF_HZ: f64 = 1.5;

fn spatial_soliton() -> TopologyDocument {
    spatial_soliton_with(SOLITON_AMPLITUDE)
}

/// A 4 Hz Gaussian beam, waist 0.2, launched at `x = −0.85` into a slab of
/// the saturable medium at the preset's own χ = 0.8 and saturation 1, made
/// slightly dispersive by a Klein-Gordon term: `n² = ε(1 − f₀²/f²)` is 1 at
/// 4 Hz, so the slab is the vacuum to a weak beam. The slab stops short of
/// the walls, where the outgoing conditions stay linear.
fn spatial_soliton_with(amplitude: f64) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.outer_boundaries =
        OuterBoundaryConditions::uniform(OuterBoundaryCondition::SecondOrderOutgoing);
    builder.launcher(-0.85, SOLITON_HZ, amplitude);
    let launcher = builder.scene.volume_sources.last_mut().unwrap();
    launcher.profile = ScalarField::formula("exp(-(y / w)^2)").unwrap();
    launcher.parameters = vec![MaterialParameter {
        name: "w".into(),
        value: 0.2,
    }];
    builder.scene.materials.push(preset_material(
        2,
        "Saturable slab",
        [178, 102, 62],
        "Saturable medium",
        LawPresetRow::Mass,
        &[("kerr_chi", 0.8), ("saturation", 1.0)],
    ));
    let klein_gordon = restoring_presets()
        .iter()
        .find(|preset| preset.id == "R1")
        .expect("the Klein-Gordon preset");
    let slab_material = builder.scene.materials.last_mut().unwrap();
    *slab_material = apply_restoring_preset(klein_gordon, slab_material)
        .expect("a restoring preset applies beside a response");
    slab_material
        .parameters
        .iter_mut()
        .find(|parameter| parameter.name == "omega0")
        .expect("the preset names its cutoff")
        .value = std::f64::consts::TAU * SOLITON_CUTOFF_HZ;
    slab_material.mass_density =
        ScalarField::constant(1.0 / (1.0 - (SOLITON_CUTOFF_HZ / SOLITON_HZ).powi(2)));
    builder.subdomain(
        slab(-0.6, 0.6, 0.9),
        MaterialId(2),
        MaterialFrame {
            attachment: MaterialFrameAttachment::FollowRegion,
            ..MaterialFrame::world()
        },
    );
    let mut document = builder.document();
    document.model.source.enabled = false;
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Along the beam".into(),
        color: [248, 196, 112],
        enabled: true,
        // To just inside the slab's back face, short of the line across the
        // beam behind it.
        target: TopologyProbeTarget::Segment {
            start: Point2::new(-0.8, 0.0),
            end: Point2::new(0.55, 0.0),
            preset: ProbeSamplingPreset::High,
        },
    });
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(2),
        name: "Behind the slab".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Segment {
            start: Point2::new(0.65, -0.9),
            end: Point2::new(0.65, 0.9),
            preset: ProbeSamplingPreset::High,
        },
    });
    document.readouts.set_probe(ProbeId(1), profile_readout());
    document.readouts.set_probe(ProbeId(2), profile_readout());
    // The beam's energy held in the channel it makes for itself.
    streamlines(&mut document.presentation);
    document
}

const DOPPLER_HZ: f64 = 1.0;
const DOPPLER_DEPTH: f64 = 0.2;

fn doppler_mirror() -> TopologyDocument {
    doppler_mirror_with(2.0 * DOPPLER_HZ)
}

/// A TM channel lit at 1 Hz from `x = −0.85`, with a slab from −0.55 to 0.75
/// whose permittivity carries a travelling modulation of depth 0.2 at
/// `pump_hz` and wavenumber `4k`, running toward the source: at 2 Hz a
/// grating moving at `c/2`, at 0 Hz the same grating at rest.
fn doppler_mirror_with(pump_hz: f64) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.outer_boundaries = channel();
    builder.launcher(-0.85, DOPPLER_HZ, 40.0);
    // The background's floor anchor moves beyond the slab, which crosses
    // the centre where it sat.
    builder.scene.face_assignments[0].anchor = FaceAnchor::Outer {
        side: OuterSide::Bottom,
        fraction: 0.95,
    };
    builder.scene.materials.push(preset_material(
        2,
        "Moving grating",
        [120, 92, 178],
        "Travelling modulation",
        LawPresetRow::Mass,
        &[
            ("depth", DOPPLER_DEPTH),
            ("pump_hz", pump_hz),
            ("pump_phase", 0.0),
            ("wavenumber", 4.0 * std::f64::consts::TAU * DOPPLER_HZ),
            ("wave_angle", std::f64::consts::PI),
        ],
    ));
    let (front, span) = builder.divider(-0.55);
    builder.divider(0.75);
    for (material, side) in [
        (MaterialId(2), CurveTraceSide::Right),
        (DEFAULT_MATERIAL, CurveTraceSide::Left),
    ] {
        let region = RegionId(builder.next_region);
        builder.next_region += 1;
        builder.scene.regions.push(Region {
            id: region,
            material,
            frame: MaterialFrame::world(),
        });
        // Running upwards, a divider's right is towards +x.
        builder.scene.face_assignments.push(AuthoredFaceAssignment {
            anchor: FaceAnchor::Curve {
                curve: front,
                span,
                side,
                parameter: 0.5,
            },
            region: Some(region),
        });
    }
    let mut document = builder.document();
    document.model.source.enabled = false;
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "In front of the slab".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Point(DOPPLER_FRONT),
    });
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(2),
        name: "Behind the slab".into(),
        color: [248, 196, 112],
        enabled: true,
        target: TopologyProbeTarget::Point(DOPPLER_BEHIND),
    });
    document
        .readouts
        .set_probe(ProbeId(1), spectrum_readout(4.0, 5.0));
    document.readouts.set_probe(ProbeId(2), field_readout(2.0));
    document
}

const DOPPLER_FRONT: Point2 = Point2::new(-0.7, 0.3);
const DOPPLER_BEHIND: Point2 = Point2::new(0.9, 0.3);

fn temporal_slab() -> TopologyDocument {
    temporal_slab_with(true)
}

/// The temporal slab's glass, `ε = 4` from `x = −0.6` to 0.9, and its
/// pulse: a Gaussian 0.2 s wide at 1.5 Hz, which inside the slab runs at
/// half the wave speed and is 0.8 long. It leaves the launcher at 0.9 s,
/// enters the slab at 1.15 s and is centred at `x = 0.1` at 2.55 s, when the
/// permittivity drops to 1 for 2 s. Pulse and drop repeat every 6 s.
const TEMPORAL_PERMITTIVITY: f64 = 4.0;
const TEMPORAL_FRONT: f64 = -0.6;
const TEMPORAL_BACK: f64 = 0.9;
const TEMPORAL_HZ: f64 = 1.5;
const TEMPORAL_WIDTH: f64 = 0.2;
const TEMPORAL_DROP: f64 = 2.55;
const TEMPORAL_HOLD: f64 = 2.0;
const TEMPORAL_REPEAT: f64 = 6.0;
/// Probes either side of where the pulse is when the permittivity drops.
const TEMPORAL_UPSTREAM: Point2 = Point2::new(-0.45, 0.0);
const TEMPORAL_DOWNSTREAM: Point2 = Point2::new(0.65, 0.0);

/// A TM channel lit by a Gaussian pulse from `x = −0.85`, with a glass slab
/// wall to wall whose permittivity, with `drop`, falls from 4 to 1 at once
/// while the pulse is inside it: a gated 0 Hz pump of depth 0.75 at phase π,
/// `1 − 0.75`, on the mass row. Across a change in time the canonical state
/// carries over, `D` and `B`, so the wavenumber stays and the frequency
/// doubles, and the pulse splits into `E_f = ½(n₁/n₂)(1 + n₁/n₂)` running on
/// and `E_b = ½(n₁/n₂)(n₁/n₂ − 1)` running back: 3 and 1 of its field. While
/// the permittivity is down the slab is vacuum, and its faces reflect
/// nothing.
fn temporal_slab_with(drop: bool) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.outer_boundaries = channel();
    let pulse = TimeSignal::pulsed(
        [0.0, 40.0, TEMPORAL_HZ, 0.0],
        PulseEnvelope::Gaussian {
            width: TEMPORAL_WIDTH,
        },
        0.1,
        TEMPORAL_REPEAT,
    );
    let region = builder.launcher(-0.85, TEMPORAL_HZ, 40.0);
    builder
        .scene
        .volume_sources
        .iter_mut()
        .find(|source| source.region == region)
        .unwrap()
        .signal = pulse;
    // The background's floor anchor moves beyond the slab, which crosses
    // the centre where it sat.
    builder.scene.face_assignments[0].anchor = FaceAnchor::Outer {
        side: OuterSide::Bottom,
        fraction: 0.975,
    };
    let mut glass = preset_material(
        2,
        "Temporal slab",
        [120, 92, 178],
        "Parametric pump",
        LawPresetRow::Mass,
        &[
            ("depth", 0.75),
            ("pump_hz", 0.0),
            ("pump_phase", std::f64::consts::PI),
        ],
    );
    glass.mass_density = ScalarField::constant(TEMPORAL_PERMITTIVITY);
    if drop {
        glass.mass_law.gate = Some(PulseTrain {
            envelope: PulseEnvelope::FlatTop {
                duration: TEMPORAL_HOLD,
                edge: 1e-3,
            },
            start: TEMPORAL_DROP,
            repeat: TEMPORAL_REPEAT,
        });
    } else {
        glass.mass_law.drive = TimeDrive::None;
    }
    builder.scene.materials.push(glass);
    let (front, span) = builder.divider(TEMPORAL_FRONT);
    builder.divider(TEMPORAL_BACK);
    for (material, side) in [
        (MaterialId(2), CurveTraceSide::Right),
        (DEFAULT_MATERIAL, CurveTraceSide::Left),
    ] {
        let region = RegionId(builder.next_region);
        builder.next_region += 1;
        builder.scene.regions.push(Region {
            id: region,
            material,
            frame: MaterialFrame::world(),
        });
        // Running upwards, a divider's right is towards +x.
        builder.scene.face_assignments.push(AuthoredFaceAssignment {
            anchor: FaceAnchor::Curve {
                curve: front,
                span,
                side,
                parameter: 0.5,
            },
            region: Some(region),
        });
    }
    let mut document = builder.document();
    document.model.source.enabled = false;
    for (id, name, color, point) in [
        (1, "Upstream", [91, 220, 194], TEMPORAL_UPSTREAM),
        (2, "Downstream", [248, 196, 112], TEMPORAL_DOWNSTREAM),
    ] {
        document.model.probes.push(TopologyProbeDefinition {
            id: ProbeId(id),
            name: name.into(),
            color,
            enabled: true,
            target: TopologyProbeTarget::Point(point),
        });
        document.readouts.set_probe(ProbeId(id), field_readout(4.0));
    }
    document
}

fn chopper() -> TopologyDocument {
    chopper_with(Shutter::Chopping, CHOPPER_RATE, CHOPPER_FRONT)
}

/// How the chopper's shutter runs: opening and shutting, or held either way,
/// which only the claim's controls do.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
enum Shutter {
    Chopping,
    Open,
    Shut,
}

/// The carrier, and the shutter: open 1 s in every 2 s, with 50 ms edges.
const CHOPPER_HZ: f64 = 3.0;
const CHOPPER_PERIOD: f64 = 2.0;
const CHOPPER_OPEN: f64 = 1.0;
const CHOPPER_EDGE: f64 = 0.05;
const CHOPPER_START: f64 = 2.0;
/// The shutter's loss when shut, eight times the carrier's angular
/// frequency, over 0.15: it passes 0.6% of the field. Thicker at a lower
/// loss it shuts as well but opens slowly, refilling at the wave's speed,
/// and the bursts lose their shape; thinner it leaks.
const CHOPPER_RATE: f64 = 8.0 * std::f64::consts::TAU * CHOPPER_HZ;
const CHOPPER_FRONT: f64 = -0.2;
const CHOPPER_BACK: f64 = -0.05;
const CHOPPER_BEHIND: Point2 = Point2::new(0.5, 0.0);

/// A TM channel lit by a 3 Hz launcher, with a lossy slab from `front` to
/// `x = −0.05` whose electric loss, `rate` when shut, a gated 0 Hz drive of
/// depth 0.99 at phase π opens to a hundredth of itself: `1 − 0.99`.
fn chopper_with(shutter: Shutter, rate: f64, front: f64) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.outer_boundaries = channel();
    builder.launcher(-0.85, CHOPPER_HZ, 40.0);
    let opening = TimeDrive::ParametricPump {
        depth: ScalarField::constant(0.99),
        frequency_hz: ScalarField::constant(0.0),
        phase_radians: ScalarField::constant(std::f64::consts::PI),
    };
    let law = match shutter {
        Shutter::Chopping => DampingLaw {
            drive: opening,
            gate: Some(PulseTrain {
                envelope: PulseEnvelope::FlatTop {
                    duration: CHOPPER_OPEN,
                    edge: CHOPPER_EDGE,
                },
                start: CHOPPER_START,
                repeat: CHOPPER_PERIOD,
            }),
            ..DampingLaw::constant()
        },
        Shutter::Open => DampingLaw {
            drive: opening,
            ..DampingLaw::constant()
        },
        Shutter::Shut => DampingLaw::constant(),
    };
    let shutter = Material {
        id: MaterialId(2),
        name: "Shutter".into(),
        electric_loss: Some(LossChannel {
            base_rate: ScalarField::constant(rate),
            law,
        }),
        color: [139, 92, 66],
        ..Material::default_medium()
    };
    builder.scene.materials.push(shutter);
    let (divider, span) = builder.divider(front);
    builder.divider(CHOPPER_BACK);
    for (material, side) in [
        (MaterialId(2), CurveTraceSide::Right),
        (DEFAULT_MATERIAL, CurveTraceSide::Left),
    ] {
        let region = RegionId(builder.next_region);
        builder.next_region += 1;
        builder.scene.regions.push(Region {
            id: region,
            material,
            frame: MaterialFrame::world(),
        });
        // Running upwards, a divider's right is towards +x.
        builder.scene.face_assignments.push(AuthoredFaceAssignment {
            anchor: FaceAnchor::Curve {
                curve: divider,
                span,
                side,
                parameter: 0.5,
            },
            region: Some(region),
        });
    }
    let mut document = builder.document();
    document.model.source.enabled = false;
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Behind the shutter".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Point(CHOPPER_BEHIND),
    });
    // Four chopping periods, so lines half a hertz apart stand clear.
    document
        .readouts
        .set_probe(ProbeId(1), spectrum_readout(4.0 * CHOPPER_PERIOD, 6.0));
    document
}

fn kerr_slab() -> TopologyDocument {
    kerr_slab_with(40.0, 60.0)
}

fn pumped_slab() -> TopologyDocument {
    pumped_slab_with(0.4, 5.0, PUMP_PHASE)
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
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Down the line".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Point(JOSEPHSON_PROBE),
    });
    document.presentation.integrated_field = true;
    document
        .readouts
        .set_probe(ProbeId(1), spectrum_readout(WHOLE_HISTORY, 3.0));
    document
}

const JOSEPHSON_PROBE: Point2 = Point2::new(0.5, 0.0);

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
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "In the plasma".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Point(EMITTER_PROBE),
    });
    document
        .readouts
        .set_probe(ProbeId(1), spectrum_readout(4.0, 5.0));
    document
}

const EMITTER_PROBE: Point2 = Point2::new(0.0, -0.7);

/// The unit direction at `angle`, from the portable sine and cosine, so a
/// scene built here has the same geometry, and meshes the same, on every
/// platform.
fn direction(angle: f64) -> Point2 {
    let (sin, cos) = portable_sin_cos(angle);
    Point2::new(cos, sin)
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
                center + direction(angle) * reach
            })
            .collect(),
    )
    .unwrap()
}

/// The circle's spline stretched to semi-axes `a` along `x` and `b` along
/// `y`: an affine image of a spline is the spline of the moved controls.
fn ellipse(center: Point2, a: f64, b: f64) -> PeriodicCubicSpline {
    let reach = 1.0 / 0.974_6;
    PeriodicCubicSpline::uniform(
        (0..16)
            .map(|index| {
                let unit = direction(index as f64 * std::f64::consts::TAU / 16.0);
                center + Point2::new(a * unit.x, b * unit.y) * reach
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
    // The mode building up over the run.
    document
        .readouts
        .set_probe(ProbeId(1), field_readout(WHOLE_HISTORY));
    document
}

const FIBER_H: f64 = 0.15;
const FIBER_DN: f64 = 0.6;
const FIBER_SIGNAL_HZ: f64 = 2.5;
const FIBER_DEPTH: f64 = 0.2;
/// The fiber's Klein-Gordon cutoff, which makes it slightly dispersive.
///
/// Without it the guided mode is nearly dispersionless, and a pump running
/// at its phase velocity also phase-matches the signal's sum frequencies
/// with itself, 7.5, 12.5, 17.5 Hz and on, which it climbs to the mesh's
/// scale: at edge 0.04 the three rungs held 0.9 of the signal, and the gain
/// fell with every refinement (4.6, 3.4 and 2.7 at edges 0.08, 0.04 and
/// 0.02). This cutoff lowers the mode's index at 2.5 Hz from 1.38 to 1.18
/// and much less at 7.5 Hz, so the first rung runs out of step within 0.2
/// of fiber instead of 0.44, the rungs stay under a third of the signal,
/// and the gain holds across meshes.
const FIBER_CUTOFF_HZ: f64 = 1.25;
/// The fiber's short-wave loss α, a viscosity on the field's gradient that
/// damps the rungs the mesh cannot carry. Measured on 3 October 2026 over 8 s
/// with the grid filter, at mid fiber against the signal there: at edge 0.04
/// the 12.5 and 17.5 Hz rungs, about 2.5 and 1.8 nodes a wavelength, fell
/// from 0.198 and 0.095 to 0.034 and 0.007, while the resolved 7.5 Hz one
/// held (0.205, 0.195). It costs 37% of the far-end signal there and 58% at
/// edge 0.08, and the gain moved from 8.48× to 7.57× (8.53× to 7.19× at
/// 0.08). α = 0.05 halved the two rungs for 4.5% of the signal; 0.5 was
/// chosen on how the scene looks, since this is a toy model of an amplifier,
/// not a design for one.
const FIBER_SHORT_WAVE: f64 = 0.5;
/// Where the gain peaks, about 3% under twice the unpumped mode's
/// propagation constant (β = 18.58 at 2.5 Hz, `n_eff` 1.183, measured from
/// the phase along the axis at edges 0.08, 0.04 and 0.02): the pump shifts
/// the signal's own propagation constant a little, and it runs with that.
/// The gain is flat to 2% over ±0.4 of it.
const FIBER_PUMP_WAVENUMBER: f64 = 36.2;
/// The pump's phase that amplifies the signal most. A degenerate pump's
/// gain goes with `pump_phase − 2 × signal phase`, so it is tied to the
/// source's cosine start (`SWITCH_ON_PHASE`) and holds if that moves.
const FIBER_PUMP_PHASE: f64 = 2.0 * SWITCH_ON_PHASE + 0.625 * std::f64::consts::PI;

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
/// above threshold and the fiber oscillates on its own. The fiber carries
/// the Klein-Gordon preset at [`FIBER_CUTOFF_HZ`], so the same pump does not
/// also climb the signal's sum frequencies.
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
    let klein_gordon = restoring_presets()
        .iter()
        .find(|preset| preset.id == "R1")
        .expect("the Klein-Gordon preset");
    let mut fiber = apply_restoring_preset(klein_gordon, &fiber)
        .expect("a restoring preset applies beside a drive");
    fiber
        .parameters
        .iter_mut()
        .find(|parameter| parameter.name == "omega0")
        .expect("the preset names its cutoff")
        .value = std::f64::consts::TAU * FIBER_CUTOFF_HZ;
    fiber.short_wave_loss = FIBER_SHORT_WAVE;
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
    // to short of the output probe: its energy density shows the signal
    // growing along the fiber.
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(2),
        name: "Along the fiber".into(),
        color: [248, 196, 112],
        enabled: true,
        target: TopologyProbeTarget::Segment {
            start: Point2::new(-0.75, 0.0),
            end: Point2::new(0.75, 0.0),
            preset: ProbeSamplingPreset::Medium,
        },
    });
    document.readouts.set_probe(ProbeId(1), field_readout(2.0));
    document.readouts.set_probe(ProbeId(2), profile_readout());
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
    let at = |angle: f64| BEND_CENTRE + direction(angle) * radius;
    let tangent = |angle: f64| {
        let along = direction(angle);
        Point2::new(-along.y, along.x) * radius
    };
    // The control reach of a cubic Bézier eighth of a circle.
    let reach = 4.0 / 3.0 * portable_tan(std::f64::consts::PI / 16.0);
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
    // The beam the bend sheds, leaving tangentially.
    power_flow(&mut document.presentation);
    document.readouts.set_probe(ProbeId(1), field_readout(2.0));
    document.readouts.set_probe(ProbeId(2), profile_readout());
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
    let reach = radius * (std::f64::consts::PI / (4.0 * portable_sin_cos(eighth).0)).sqrt();
    PeriodicCubicSpline::polygon(
        (0..8)
            .map(|index| {
                let angle = (index as f64 + 0.5) * eighth;
                center + direction(angle) * reach
            })
            .collect(),
    )
    .unwrap()
}

fn photonic_crystal() -> TopologyDocument {
    photonic_crystal_with(CRYSTAL_PULSE_REPEAT, true)
}

/// The crystal's sinc pulse repeats once a segment of the transfer readout:
/// the crystal rings for seconds at its band edges, so a segment must be
/// long, and a train that put several pulses in one segment would hand the
/// average only its harmonics, `k/repeat` Hz, whose phases the ringing turns
/// apart.
const CRYSTAL_PULSE_REPEAT: f64 = CRYSTAL_SEGMENT;

/// A TM channel in two arms (`Builder::arms`) lit from the left by a sinc
/// pulse flat from 0.8 to 3 Hz, across the gap and either side of it, every
/// `repeat` seconds, zero for one, with a square lattice of
/// ceramic rods, `ε = 9` and 0.2 of the pitch in radius, five columns deep
/// and five rows filling the upper arm. The walls either side of that arm
/// sit on the lattice's mirror planes, so the arm is the infinite crystal at
/// normal incidence, whose TM gap runs from 0.275 to 0.445 of the pitch over
/// the wavelength: 1.375 to 2.225 Hz. A probe behind the crystal reads its
/// transfer from a probe at the same place in the empty arm.
fn photonic_crystal_with(repeat: f64, rods: bool) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.outer_boundaries = channel();
    builder.arms(-0.85, sinc_pulse(0.8, 3.0, repeat), &[], None);
    builder.scene.materials.push(Material {
        id: MaterialId(2),
        name: "Ceramic".into(),
        mass_density: ScalarField::constant(CRYSTAL_ROD_PERMITTIVITY),
        color: [66, 105, 151],
        ..Material::default_medium()
    });
    if rods {
        for column in 0..5 {
            for row in 0..5 {
                let centre = Point2::new(
                    (column as f64 - 2.0) * CRYSTAL_PITCH,
                    (row as f64 + 0.5) * CRYSTAL_PITCH,
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
    arm_probes(
        &mut document,
        "Behind the crystal",
        transfer_readout(ProbeId(2), 3.5, CRYSTAL_SEGMENT),
    );
    // Twenty-five rods' control polygons and handles would hide the crystal.
    document.presentation.control_polygons = false;
    document.presentation.handles = false;
    document
}

const CRYSTAL_SEGMENT: f64 = 20.0;

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
    // The power following the channel round the corner, over rods without
    // their control polygons and handles.
    streamlines(&mut document.presentation);
    document.presentation.control_polygons = false;
    document.presentation.handles = false;
    document.readouts.set_probe(ProbeId(1), profile_readout());
    document.readouts.set_probe(ProbeId(2), field_readout(2.0));
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
        target: TopologyProbeTarget::Point(Point2::new(0.65, -0.55)),
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
    // The power circulating in the ring.
    power_flow(&mut document.presentation);
    // The ring filling over the run.
    document.readouts.set_probe(ProbeId(1), field_readout(2.0));
    document
        .readouts
        .set_probe(ProbeId(2), area_readout(true, WHOLE_HISTORY));
    document
}

/// A circular arc about `center` from angle `from` to `to`, in radians and
/// counterclockwise, as `pieces` cubic Bézier pieces joined at C0 knots.
fn arc(center: Point2, radius: f64, from: f64, to: f64, pieces: usize) -> OpenCubicSpline {
    let step = (to - from) / pieces as f64;
    let reach = 4.0 / 3.0 * portable_tan(step / 4.0) * radius;
    let at = |angle: f64| center + direction(angle) * radius;
    let tangent = |angle: f64| {
        let along = direction(angle);
        Point2::new(-along.y, along.x)
    };
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
        .readouts
        .set_probe(ProbeId(1), area_readout(false, 2.0));
    document
        .readouts
        .set_probe(ProbeId(2), area_readout(false, 2.0));
    // The sound travelling round the wall.
    streamlines(&mut document.presentation);
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
    document.readouts.set_probe(ProbeId(1), profile_readout());
    document
        .readouts
        .set_probe(ProbeId(2), field_readout(WHOLE_HISTORY));
    document
}

const FISHEYE_RADIUS: f64 = 0.45;
const FISHEYE_HZ: f64 = 3.0;

fn ellipse_flash() -> TopologyDocument {
    ellipse_flash_with(ELLIPSE_REPEAT, ELLIPSE_LOSS)
}

/// The room's semi-axes, its foci `±√(a² − b²)` = ±0.512 on `x`, and its
/// flash: a Gaussian 0.1 s wide at 2 Hz, so its peak leaves the near focus
/// 0.5 s after it starts, every `ELLIPSE_REPEAT` seconds.
const ELLIPSE_A: f64 = 0.95;
const ELLIPSE_B: f64 = 0.8;
const ELLIPSE_HZ: f64 = 2.0;
const ELLIPSE_WIDTH: f64 = 0.1;
const ELLIPSE_REPEAT: f64 = 10.0;
const ELLIPSE_LOSS: f64 = 0.2;

fn ellipse_focus() -> f64 {
    (ELLIPSE_A * ELLIPSE_A - ELLIPSE_B * ELLIPSE_B).sqrt()
}

/// A Mechanical room inside a reflecting ellipse, everything outside cut
/// away, with a velocity loss `loss` so one flash has faded before the next,
/// flashed at one focus by a point source every `repeat` seconds, zero for
/// once. Every path from one focus to the wall and on to the other is `2a`
/// long, so all the flash's echoes reach the far focus together, `2a/c`
/// after it left, where the direct wave took `2√(a² − b²)/c`. The gap
/// between the two, `2(a − √(a² − b²))/c` = 0.88 s, is why the room is this
/// round: at `b = 0.6` it is 0.43 s, and the direct pulse runs into the
/// echoes.
fn ellipse_flash_with(repeat: f64, loss: f64) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.materials[0].name = "Air".into();
    builder.scene.materials[0].magnetic_loss = Some(LossChannel {
        base_rate: ScalarField::constant(loss),
        law: DampingLaw::constant(),
    });
    builder.closed(
        ellipse(Point2::default(), ELLIPSE_A, ELLIPSE_B),
        SpanBehavior::REFLECTING,
        None,
    );
    // The room is the background region inside the wall; outside is cut away.
    builder.scene.face_assignments[0].region = None;
    let last = builder.scene.face_assignments.len() - 1;
    builder.scene.face_assignments[last].region = Some(BACKGROUND_REGION);
    let mut document = builder.document();
    let focus = ellipse_focus();
    document.model.source = PointSource {
        enabled: true,
        position: Point2::new(-focus, 0.0),
        width: 0.03,
        region: BACKGROUND_REGION,
        signal: TimeSignal::pulsed(
            [0.0, 10.0, ELLIPSE_HZ, 0.0],
            PulseEnvelope::Gaussian {
                width: ELLIPSE_WIDTH,
            },
            0.1,
            repeat,
        ),
    };
    for (id, name, color, point) in [
        (1, "Far focus", [91, 220, 194], Point2::new(focus, 0.0)),
        (2, "Beside it", [248, 196, 112], ELLIPSE_BESIDE),
    ] {
        document.model.probes.push(TopologyProbeDefinition {
            id: ProbeId(id),
            name: name.into(),
            color,
            enabled: true,
            target: TopologyProbeTarget::Point(point),
        });
        document.readouts.set_probe(ProbeId(id), field_readout(4.0));
    }
    document
}

/// A quarter beside the far focus.
const ELLIPSE_BESIDE: Point2 = Point2::new(0.512, 0.25);

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
    // The permittivity, 1 at the rim and 4 at the centre.
    document.presentation.material_overlay = MaterialOverlay::Property(MaterialProperty::Density);
    document.presentation.material_overlay_opacity = 0.36;
    document.readouts.set_probe(ProbeId(1), profile_readout());
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
    // Half the focal plane, from beside the focus out to the wall, so it
    // stays clear of the focus probe: the plate is symmetric about the axis,
    // and this half shows the spot falling away to the claim's 0.4 aside.
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(2),
        name: "Focal plane".into(),
        color: [248, 196, 112],
        enabled: true,
        target: TopologyProbeTarget::Segment {
            start: Point2::new(ZONE_FOCUS, 0.08),
            end: Point2::new(ZONE_FOCUS, 0.9),
            preset: ProbeSamplingPreset::Medium,
        },
    });
    document.readouts.set_probe(ProbeId(1), field_readout(2.0));
    document.readouts.set_probe(ProbeId(2), profile_readout());
    // The energy through the open zones converging on the focus.
    streamlines(&mut document.presentation);
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
    // The in-plane electric field, whose direction is what Brewster is about.
    document.presentation.vector_overlay = VectorOverlay::ComplementaryField;
    document.presentation.vector_overlay_gain = ARROW_GAIN;
    document.readouts.set_probe(ProbeId(1), profile_readout());
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
        (1, "Fiber output", [91, 220, 194], Point2::new(0.7, -0.5)),
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
    // The power tunnelling across the gap and leaving as a tilted beam.
    streamlines(&mut document.presentation);
    document.readouts.set_probe(ProbeId(1), field_readout(2.0));
    document.readouts.set_probe(ProbeId(2), field_readout(2.0));
    document
}

const TALBOT_HZ: f64 = 4.0;
const TALBOT_PERIOD: f64 = 0.4;
const TALBOT_SCREEN: f64 = -0.5;

/// The Talbot distance of a grating of `TALBOT_PERIOD` at `TALBOT_HZ`, not
/// paraxial: `λ/(1 − √(1 − (λ/a)²))`.
fn talbot_distance() -> f64 {
    let ratio = 1.0 / TALBOT_HZ / TALBOT_PERIOD;
    (1.0 / TALBOT_HZ) / (1.0 - (1.0 - ratio * ratio).sqrt())
}

fn talbot() -> TopologyDocument {
    talbot_with(true)
}

/// A Mechanical channel lit by a 4 Hz launcher at the left, with a grating
/// of reflecting bars at `x = −0.5`: period 0.4, half open, its slits
/// centred at `y = 0.2 + 0.4k`, so the reflecting walls sit on slit centres
/// and the channel holds the infinite grating. Behind it the grating's image
/// comes back at the Talbot distance and, halfway there, shifted by half a
/// period. Only the orders 0 and ±1 propagate, `λ/a = 0.625`, so that
/// distance is 1.14, not the paraxial `2a²/λ = 1.28`. Line probes run across
/// the channel on both images. Without `grating` the channel is open.
fn talbot_with(grating: bool) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.outer_boundaries = channel();
    builder.launcher(-0.85, TALBOT_HZ, 40.0);
    if grating {
        for bar in -2..=2 {
            let centre = TALBOT_PERIOD * bar as f64;
            builder.baffle(
                OpenCubicSpline::polyline(vec![
                    Point2::new(TALBOT_SCREEN, centre - 0.25 * TALBOT_PERIOD),
                    Point2::new(TALBOT_SCREEN, centre + 0.25 * TALBOT_PERIOD),
                ])
                .unwrap(),
            );
        }
    }
    let mut document = builder.document();
    document.model.source.enabled = false;
    for (id, name, color, distance) in [
        (1, "Half the Talbot distance", [248, 196, 112], 0.5),
        (2, "Talbot distance", [91, 220, 194], 1.0),
    ] {
        let x = TALBOT_SCREEN + distance * talbot_distance();
        document.model.probes.push(TopologyProbeDefinition {
            id: ProbeId(id),
            name: name.into(),
            color,
            enabled: true,
            target: TopologyProbeTarget::Segment {
                start: Point2::new(x, -0.95),
                end: Point2::new(x, 0.95),
                preset: ProbeSamplingPreset::Medium,
            },
        });
    }
    document.readouts.set_probe(ProbeId(1), profile_readout());
    document.readouts.set_probe(ProbeId(2), profile_readout());
    document
}

const DRUM_RADIUS: f64 = 0.6;
/// The first zero of `J₂`.
const BESSEL_J21: f64 = 5.135_622;
/// A tenth of the plan's 0.3: with that the neighbouring modes, the source
/// excites too, kept 14% of the lobes' field on the nodal diameters.
const DRUM_LOSS: f64 = 0.1;

/// The clamped drum's `(2,1)` mode, `j₂₁/(2πa) = 1.362 Hz`.
fn drum_frequency() -> f64 {
    BESSEL_J21 / (std::f64::consts::TAU * DRUM_RADIUS)
}

fn drum() -> TopologyDocument {
    drum_with(drum_frequency())
}

/// A Mechanical drum: the inside of a circle of radius 0.6 clamped at its
/// rim, everything outside cut away, with a velocity loss of 0.1 per second
/// so a steady state exists, driven by a point source on one lobe of its
/// `(2,1)` mode. At that mode's frequency the membrane stands in four lobes
/// with two still nodal diameters between them. A point probe sits on
/// another lobe and one on a nodal diameter.
fn drum_with(frequency: f64) -> TopologyDocument {
    let mut document = membrane();
    document.model.source = source(Point2::new(0.3, 0.0), frequency, 10.0, 0.03);
    let diagonal = std::f64::consts::FRAC_PI_4;
    for (id, name, color, point) in [
        (1, "Lobe", [91, 220, 194], Point2::new(0.0, 0.36)),
        (
            2,
            "Nodal diameter",
            [248, 196, 112],
            Point2::new(diagonal.cos(), diagonal.sin()) * 0.36,
        ),
    ] {
        document.model.probes.push(TopologyProbeDefinition {
            id: ProbeId(id),
            name: name.into(),
            color,
            enabled: true,
            target: TopologyProbeTarget::Point(point),
        });
    }
    // The lobes building up, and the diameter staying still.
    document
        .readouts
        .set_probe(ProbeId(1), field_readout(WHOLE_HISTORY));
    document
        .readouts
        .set_probe(ProbeId(2), field_readout(WHOLE_HISTORY));
    document
}

fn struck_drum() -> TopologyDocument {
    struck_drum_with(Point2::default())
}

fn pumped_drum() -> TopologyDocument {
    pumped_drum_with(DRUM_PUMP_DEPTH, 2.0 * drum_fundamental())
}

/// The first zero of `J₀`, where the drum's fundamental stands.
const BESSEL_J01: f64 = 2.404_826;
/// The pump: the membrane's density swinging by 40% for 8 s from 2 s after
/// each knock, at twice the fundamental. At 30% the fundamental's line only
/// draws level with the first overtone's.
const DRUM_PUMP_DEPTH: f64 = 0.4;
/// The first overtone's still circle, `r = a j₀₁/j₀₂`, where a probe hears
/// the fundamental without it.
const DRUM_STILL_CIRCLE: Point2 = Point2::new(0.0, 0.6 * 2.404_826 / 5.520_078);
const DRUM_PUMP_START: f64 = 2.0;
const DRUM_PUMP_HOLD: f64 = 8.0;

/// The drum's fundamental, `j₀₁/(2πa)` = 0.638 Hz.
fn drum_fundamental() -> f64 {
    BESSEL_J01 / (std::f64::consts::TAU * DRUM_RADIUS)
}

/// The struck drum whose membrane's density is pumped by `depth` at
/// `pump_hz` for `DRUM_PUMP_HOLD` seconds after each knock: a parametric
/// pump, gated, on the mass row. At twice a mode's frequency `ω` it grows
/// that mode at `dω/4`, and a pump uniform over the membrane couples round
/// modes to round modes only, so at twice the fundamental only the
/// fundamental pairs with itself.
fn pumped_drum_with(depth: f64, pump_hz: f64) -> TopologyDocument {
    let drive = TimeDrive::ParametricPump {
        depth: ScalarField::constant(depth),
        frequency_hz: ScalarField::constant(pump_hz),
        phase_radians: ScalarField::constant(0.0),
    };
    let gate = PulseTrain {
        envelope: PulseEnvelope::FlatTop {
            duration: DRUM_PUMP_HOLD,
            edge: 0.5,
        },
        start: DRUM_PUMP_START,
        repeat: DRUM_KNOCK_REPEAT,
    };
    let mut document = struck(membrane_driven(drive, Some(gate)), Point2::default());
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(2),
        name: "On the overtone's still circle".into(),
        color: [248, 196, 112],
        enabled: true,
        target: TopologyProbeTarget::Point(DRUM_STILL_CIRCLE),
    });
    document
        .readouts
        .set_probe(ProbeId(2), field_readout(DRUM_SPAN));
    document
}

/// The drum struck at `at` every 20 s: a point source whose rate is a
/// Gaussian 0.05 s wide, a velocity impulse whose spectrum is still 60% of
/// its peak at 3 Hz. A probe sits just beside the centre, where every round
/// mode moves nearly as much as at the centre itself.
fn struck_drum_with(at: Point2) -> TopologyDocument {
    struck(membrane(), at)
}

/// `membrane` knocked at `at` every 20 s, with the probe beside the centre.
fn struck(mut document: TopologyDocument, at: Point2) -> TopologyDocument {
    document.model.source = PointSource {
        enabled: true,
        position: at,
        width: 0.03,
        region: BACKGROUND_REGION,
        signal: TimeSignal::pulsed(
            [10.0, 0.0, 0.0, 0.0],
            PulseEnvelope::Gaussian {
                width: DRUM_KNOCK_WIDTH,
            },
            0.1,
            DRUM_KNOCK_REPEAT,
        ),
    };
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Beside the centre".into(),
        color: [91, 220, 194],
        enabled: true,
        target: TopologyProbeTarget::Point(DRUM_LISTENER),
    });
    document
        .readouts
        .set_probe(ProbeId(1), spectrum_readout(DRUM_SPAN, 4.0));
    document
}

const DRUM_SPAN: f64 = 20.0;
const DRUM_KNOCK_WIDTH: f64 = 0.05;
const DRUM_KNOCK_REPEAT: f64 = 20.0;
const DRUM_LISTENER: Point2 = Point2::new(0.0, 0.08);

/// The clamped drum's membrane, radius 0.6, with nothing driving it.
fn membrane() -> TopologyDocument {
    membrane_driven(TimeDrive::None, None)
}

/// The membrane with `drive` on its density, in `gate`'s pulses if given.
fn membrane_driven(drive: TimeDrive, gate: Option<PulseTrain>) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.materials[0].name = "Membrane".into();
    builder.scene.materials[0].mass_law.drive = drive;
    builder.scene.materials[0].mass_law.gate = gate;
    builder.scene.materials[0].magnetic_loss = Some(LossChannel {
        base_rate: ScalarField::constant(DRUM_LOSS),
        law: DampingLaw::constant(),
    });
    let clamped = SpanBehavior::Separated {
        left: FaceBoundaryCondition::Dirichlet {
            signal: TimeSignal::harmonic(0.0, 0.0, 1.0, 0.0),
        },
        right: FaceBoundaryCondition::Reflecting,
        coupling: InternalBoundaryCoupling::Independent,
    };
    builder.closed(circle(Point2::default(), DRUM_RADIUS), clamped, None);
    // The membrane is the background region inside the rim; outside is cut
    // away.
    builder.scene.face_assignments[0].region = None;
    let last = builder.scene.face_assignments.len() - 1;
    builder.scene.face_assignments[last].region = Some(BACKGROUND_REGION);
    let mut document = builder.document();
    // The membrane's elements.
    document.presentation.mesh = true;
    document
}

/// The seed of the disordered crystal's rods.
const DISORDER_SEED: u64 = 0x5eed_2026_0926;
/// How close two rods' centres may come: nearer, the gap between them
/// forces elements, and a time step, far below the rest of the scene's.
const DISORDER_SPACING: f64 = 0.14;

/// Fifty rod centres drawn at random, with a fixed seed, over the photonic
/// crystal's slab, `x` from −0.5 to 0.5, kept `DISORDER_SPACING` apart and
/// clear of the channel's walls.
fn disordered_sites() -> Vec<Point2> {
    let mut state = DISORDER_SEED;
    let mut next = move || {
        // xorshift64*
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        (state.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut sites: Vec<Point2> = vec![];
    while sites.len() < 50 {
        let candidate = Point2::new(-0.45 + 0.9 * next(), -0.9 + 1.8 * next());
        if sites
            .iter()
            .all(|site| (*site - candidate).norm() >= DISORDER_SPACING)
        {
            sites.push(candidate);
        }
    }
    sites
}

/// The photonic crystal's pass frequency above its gap, where the ordered
/// crystal lets 80% of the power through.
const DISORDER_HZ: f64 = 2.5;

fn disordered_crystal() -> TopologyDocument {
    rods_with(&disordered_sites())
}

/// The photonic crystal's channel and launcher at 2.5 Hz with its ceramic
/// rods at `sites`. Scattered from rod to rod, a wave through fifty rods at
/// random mostly turns back, and what gets through has been thrown off the
/// straight path. A point probe and a line probe across the channel read
/// what gets through.
fn rods_with(sites: &[Point2]) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.outer_boundaries = channel();
    builder.launcher(-0.85, DISORDER_HZ, 40.0);
    builder.scene.materials.push(Material {
        id: MaterialId(2),
        name: "Ceramic".into(),
        mass_density: ScalarField::constant(CRYSTAL_ROD_PERMITTIVITY),
        color: [66, 105, 151],
        ..Material::default_medium()
    });
    for site in sites {
        builder.subdomain(
            octagonal_rod(*site, CRYSTAL_ROD_FRACTION * CRYSTAL_PITCH),
            MaterialId(2),
            MaterialFrame::world(),
        );
    }
    let mut document = builder.document();
    document.model.source.enabled = false;
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Across the channel".into(),
        color: [248, 196, 112],
        enabled: true,
        target: TopologyProbeTarget::Segment {
            start: Point2::new(0.75, -0.95),
            end: Point2::new(0.75, 0.95),
            preset: ProbeSamplingPreset::Medium,
        },
    });
    // Fifty rods' control polygons and handles would hide the scatterers.
    document.presentation.control_polygons = false;
    document.presentation.handles = false;
    // The profile, and the power through the cut: the average flux across it.
    document.readouts.set_probe(
        ProbeId(1),
        profile_readout().with_line_plot(
            LineProbeQuantity::MeanFlux,
            LineProbeRepresentation::Integral,
        ),
    );
    document
}

const SKIN_HZ: f64 = 3.0;
/// The slab's faces.
const SKIN_FRONT: f64 = -0.35;
const SKIN_BACK: f64 = -0.05;

/// The loss rate, `γ = 2ω`.
fn echo_comb() -> TopologyDocument {
    echo_comb_with(ECHO_REPEAT)
}

/// The echo's mirror, half a unit behind the probe, so that there the echo
/// trails the pulse by a second.
const ECHO_PROBE_X: f64 = 0.45;
const ECHO_MIRROR: f64 = 0.95;
const ECHO_REPEAT: f64 = ECHO_SEGMENT;
const ECHO_SEGMENT: f64 = 16.0;

/// A TM channel in two arms lit by a sinc pulse flat from 0.5 to 3.5 Hz every
/// `repeat` seconds, zero for once, with the upper arm closed by a mirror at
/// `x = 0.95`. A probe half a unit before it hears the pulse and then, `τ =
/// 2d/c` = 1 s later, its echo, which a reflecting face in the E_z skin
/// returns in phase: over the reference's pulse alone the probe reads `1 +
/// e^{−iωτ}`, `2|cos(πfτ)|` in magnitude.
fn echo_comb_with(repeat: f64) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.outer_boundaries = channel();
    builder.arms(-0.85, sinc_pulse(0.5, 3.5, repeat), &[], Some(ECHO_MIRROR));
    let mut document = builder.document();
    document.model.source.enabled = false;
    arm_probes_at(
        &mut document,
        "Before the mirror",
        ECHO_PROBE_X,
        transfer_readout(ProbeId(2), 4.0, ECHO_SEGMENT),
    );
    document
}

fn etalon() -> TopologyDocument {
    etalon_with(ETALON_PERMITTIVITY, ETALON_REPEAT)
}

/// The etalon's ceramic and thickness: index 3 over a sixth, so a round trip
/// inside is one second and the slab passes everything at every whole hertz.
const ETALON_PERMITTIVITY: f64 = 9.0;
const ETALON_THICKNESS: f64 = 1.0 / 6.0;
const ETALON_FRONT: f64 = -0.1;
/// A pulse a segment, as the photonic crystal's. At 8 s the slab's echoes,
/// a second apart, read low by the segment's overlap with itself shifted by
/// them, and the fringes' peaks read 0.93; at 16 s, 0.98.
const ETALON_REPEAT: f64 = ETALON_SEGMENT;
const ETALON_SEGMENT: f64 = 16.0;

/// A TM channel in two arms lit by a sinc pulse flat from 0.5 to 3.5 Hz
/// every `repeat` seconds, zero for one, with a slab of `permittivity`
/// `ETALON_THICKNESS` thick across the upper arm. Each face reflects
/// `(n − 1)/(n + 1)` of the field, a half at `n = 3`, and the slab passes
/// `1/√(cos²δ + ¼(n + 1/n)² sin²δ)` of it, `δ = 2πfnd/c`: everything where
/// a round trip inside is a whole number of periods, 0.6 halfway between.
fn etalon_with(permittivity: f64, repeat: f64) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.outer_boundaries = channel();
    builder.scene.materials.push(Material {
        id: MaterialId(2),
        name: "Ceramic".into(),
        mass_density: ScalarField::constant(permittivity),
        color: [66, 105, 151],
        ..Material::default_medium()
    });
    builder.arms(
        -0.85,
        sinc_pulse(0.5, 3.5, repeat),
        &[(ETALON_FRONT, ETALON_FRONT + ETALON_THICKNESS, MaterialId(2))],
        None,
    );
    let mut document = builder.document();
    document.model.source.enabled = false;
    arm_probes(
        &mut document,
        "Behind the etalon",
        transfer_readout(ProbeId(2), 4.0, ETALON_SEGMENT),
    );
    document
}

fn cavity_filter() -> TopologyDocument {
    cavity_filter_with(CAVITY_REPEAT)
}

/// The cavity filter's design frequency, and its two ceramic plates, each a
/// quarter wave thick there, half a wave apart.
const CAVITY_HZ: f64 = 2.0;
const CAVITY_PERMITTIVITY: f64 = 9.0;
const CAVITY_PLATE: f64 = 1.0 / (4.0 * 3.0 * CAVITY_HZ);
const CAVITY_GAP: f64 = 1.0 / (2.0 * CAVITY_HZ);
const CAVITY_FRONT: f64 = -0.15;
/// A pulse a segment. The passband is 0.22 Hz wide, and a Hann window's
/// main lobe smooths it: 16 s segments read its peak at 0.93, 24 s at 0.965.
const CAVITY_REPEAT: f64 = CAVITY_SEGMENT;
const CAVITY_SEGMENT: f64 = 24.0;

/// A TM channel in two arms lit by a sinc pulse flat from 1 to 3 Hz every
/// `repeat` seconds, zero for one, with two ceramic plates, `ε = 9`, across
/// the upper arm. At `CAVITY_HZ` each plate is a quarter wave thick and
/// reflects `(n² − 1)/(n² + 1)`, 0.8, of the field, and the vacuum between
/// them is half a wave: a cavity whose two mirrors pass everything at its
/// resonance and little either side of it.
fn cavity_filter_with(repeat: f64) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.outer_boundaries = channel();
    builder.scene.materials.push(Material {
        id: MaterialId(2),
        name: "Ceramic".into(),
        mass_density: ScalarField::constant(CAVITY_PERMITTIVITY),
        color: [66, 105, 151],
        ..Material::default_medium()
    });
    let back = CAVITY_FRONT + CAVITY_PLATE + CAVITY_GAP;
    builder.arms(
        -0.85,
        sinc_pulse(1.0, 3.0, repeat),
        &[
            (CAVITY_FRONT, CAVITY_FRONT + CAVITY_PLATE, MaterialId(2)),
            (back, back + CAVITY_PLATE, MaterialId(2)),
        ],
        None,
    );
    let mut document = builder.document();
    document.model.source.enabled = false;
    arm_probes(
        &mut document,
        "Behind the filter",
        transfer_readout(ProbeId(2), 3.5, CAVITY_SEGMENT),
    );
    document
}

fn skin_loss() -> f64 {
    2.0 * std::f64::consts::TAU * SKIN_HZ
}

fn skin_depth() -> TopologyDocument {
    skin_with(skin_loss())
}

/// A TM channel lit by a 3 Hz launcher at the left, with a slab of an
/// otherwise vacuum medium carrying an electric loss `rate` from
/// `x = −0.35` to `−0.05`. Inside, the plane wave runs as `e^{ikx}` with
/// `k = (ω/c)√(1 − iγ/ω)`: at `γ = 2ω` it falls by `e` every 0.068 while its
/// crests stand 0.26 apart, where the good conductor's `√(ωγ/2)`, the usual
/// shortcut, would put the depth at 0.053. A line probe runs along the
/// channel through the slab and a point probe sits behind it.
fn skin_with(rate: f64) -> TopologyDocument {
    slab_channel(Material {
        id: MaterialId(2),
        name: "Lossy medium".into(),
        electric_loss: Some(LossChannel {
            base_rate: ScalarField::constant(rate),
            law: DampingLaw::constant(),
        }),
        color: [139, 92, 66],
        ..Material::default_medium()
    })
}

/// A TM channel lit by a 3 Hz launcher at the left, with a slab of
/// `material` from `x = −0.35` to `−0.05`, a line probe along the channel
/// through it and a point probe behind it.
fn slab_channel(material: Material) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.outer_boundaries = channel();
    builder.launcher(-0.85, SKIN_HZ, 40.0);
    let slab = material.id;
    builder.scene.materials.push(material);
    let (front, span) = builder.divider(SKIN_FRONT);
    builder.divider(SKIN_BACK);
    for (material, side) in [
        (slab, CurveTraceSide::Right),
        (DEFAULT_MATERIAL, CurveTraceSide::Left),
    ] {
        let region = RegionId(builder.next_region);
        builder.next_region += 1;
        builder.scene.regions.push(Region {
            id: region,
            material,
            frame: MaterialFrame::world(),
        });
        // Running upwards, a divider's right is towards +x.
        builder.scene.face_assignments.push(AuthoredFaceAssignment {
            anchor: FaceAnchor::Curve {
                curve: front,
                span,
                side,
                parameter: 0.5,
            },
            region: Some(region),
        });
    }
    let mut document = builder.document();
    document.model.source.enabled = false;
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Through the slab".into(),
        color: [248, 196, 112],
        enabled: true,
        target: TopologyProbeTarget::Segment {
            start: Point2::new(-0.8, 0.3),
            end: Point2::new(0.9, 0.3),
            preset: ProbeSamplingPreset::High,
        },
    });
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(2),
        name: "Behind the slab".into(),
        color: [91, 220, 194],
        enabled: true,
        // Off the line, on the axis: the plane wave is the same across the
        // channel.
        target: TopologyProbeTarget::Point(Point2::new(0.5, 0.0)),
    });
    // The lossy slab takes power, so its flow runs forward and fades; the
    // plasma takes none, so its flow swings back and forth about nothing.
    power_flow(&mut document.presentation);
    document.readouts.set_probe(ProbeId(1), profile_readout());
    document.readouts.set_probe(ProbeId(2), field_readout(2.0));
    document
}

/// The Klein-Gordon plasma's cutoff, above the 3 Hz drive.
const PLASMA_SKIN_CUTOFF_HZ: f64 = 4.0;

fn plasma_skin_depth() -> TopologyDocument {
    plasma_skin_with(PLASMA_SKIN_CUTOFF_HZ)
}

/// The skin-depth scene's slab as a collisionless plasma, the Klein-Gordon
/// medium with a cutoff of `cutoff` Hz above the 3 Hz drive: below its
/// cutoff a cold plasma is a mirror, and the field reaching into it falls as
/// `e^{−κx}` with `κ = √(ω_p² − ω²)/c`, 16.6 for a 4 Hz cutoff, a depth of
/// 0.060, without the travelling phase a lossy slab has and without taking
/// power: it all comes back. `c/ω_p`, the depth far below the cutoff, would
/// put it at 0.040.
fn plasma_skin_with(cutoff: f64) -> TopologyDocument {
    slab_channel(tm_plasma(cutoff))
}

/// The TM skin's Klein-Gordon medium, a collisionless plasma, as material 2,
/// cutting off at `cutoff` Hz: a wave at `f` runs there with `k =
/// 2π√(f² − cutoff²)/c`.
fn tm_plasma(cutoff: f64) -> Material {
    let physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    let preset = medium_presets()
        .iter()
        .find(|preset| !preset.self_oscillating && preset.restoring().id == "R1")
        .expect("the Klein-Gordon medium");
    let mut plasma = apply_medium_preset(
        preset,
        &Material {
            id: MaterialId(2),
            ..Material::default_medium()
        },
        physics,
    )
    .expect("a medium applies to a fresh material");
    plasma.name = "Plasma".into();
    plasma
        .parameters
        .iter_mut()
        .find(|parameter| parameter.name == "omega0")
        .expect("the medium names its cutoff")
        .value = std::f64::consts::TAU * cutoff;
    plasma
}

fn plasma_delay() -> TopologyDocument {
    plasma_delay_with(PLASMA_DELAY_REPEAT)
}

/// The plasma's cutoff under the pulse's carrier, and the pulse: a Gaussian
/// 0.5 s wide, whose spectrum is down to 1% by 0.95 Hz either side of its
/// carrier, so all of it runs above the cutoff.
const PLASMA_DELAY_CUTOFF_HZ: f64 = 2.0;
const PLASMA_DELAY_HZ: f64 = 3.0;
const PLASMA_DELAY_WIDTH: f64 = 0.5;
const PLASMA_DELAY_REPEAT: f64 = 8.0;
const PLASMA_DELAY_FRONT: f64 = -0.7;
const PLASMA_DELAY_BACK: f64 = 0.6;

/// A TM channel in two arms lit by a Gaussian pulse at 3 Hz every `repeat`
/// seconds, zero for one, with a plasma cutting off at 2 Hz across the upper
/// arm from `x = −0.7` to 0.6. There the pulse's energy runs at the group
/// velocity `c√(1 − (f_c/f)²)` and its crests at the phase velocity
/// `c/√(1 − (f_c/f)²)`; its twin in the lower arm runs through vacuum.
fn plasma_delay_with(repeat: f64) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.outer_boundaries = channel();
    builder
        .scene
        .materials
        .push(tm_plasma(PLASMA_DELAY_CUTOFF_HZ));
    builder.arms(
        -0.85,
        TimeSignal::pulsed(
            [0.0, 40.0, PLASMA_DELAY_HZ, 0.0],
            PulseEnvelope::Gaussian {
                width: PLASMA_DELAY_WIDTH,
            },
            0.1,
            repeat,
        ),
        &[(PLASMA_DELAY_FRONT, PLASMA_DELAY_BACK, MaterialId(2))],
        None,
    );
    let mut document = builder.document();
    document.model.source.enabled = false;
    arm_probes(&mut document, "Behind the plasma", field_readout(4.0));
    document
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scene near the topology limits: 63 rods of 42 corners, 126 controls
    /// each against a curve's 128 and 63 curves against 64.
    fn scene_at_the_limits() -> TopologyDocument {
        let mut builder = Builder::new();
        builder.scene.materials.push(Material {
            id: MaterialId(2),
            name: "Rod".into(),
            mass_density: ScalarField::constant(2.0),
            ..Material::default_medium()
        });
        for index in 0..63 {
            let center = Point2::new(
                -0.84 + 0.24 * (index % 8) as f64,
                -0.84 + 0.24 * (index / 8) as f64,
            );
            let corners = (0..42)
                .map(|corner| {
                    center + direction(corner as f64 * std::f64::consts::TAU / 42.0) * 0.08
                })
                .collect();
            builder.subdomain(
                PeriodicCubicSpline::polygon(corners).unwrap(),
                MaterialId(2),
                MaterialFrame::world(),
            );
        }
        builder.document()
    }

    /// Opening refused a scene file past 2 MiB, which this one passes at
    /// about 2.7 MiB: it saved, and then neither the file nor the autosave
    /// opened. The limit has room for the topology limits now.
    #[test]
    fn a_scene_at_the_topology_limits_saves_and_opens() {
        let document = scene_at_the_limits();
        let saved = crate::topology_persistence::save(&document).unwrap();
        assert!(saved.len() > 2 * 1024 * 1024, "{} bytes", saved.len());
        assert!(
            saved.len() < crate::topology_persistence::MAX_FILE_BYTES / 4,
            "{} bytes",
            saved.len()
        );
        assert_eq!(
            crate::topology_persistence::parse_document(saved.as_bytes()),
            Ok(document)
        );
    }

    #[test]
    fn topology_catalog_is_valid_version_22_data_with_complete_semantics() {
        assert_eq!(catalog().len(), 45);
        assert_eq!(catalog()[0].name, "Obstacle over a mirror");
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

    /// The catalog runs section by section in the gallery's order, every
    /// section has something in it, and no two scenes share a name.
    #[test]
    fn the_catalog_runs_section_by_section() {
        let groups = catalog()
            .iter()
            .map(|example| example.group)
            .collect::<Vec<_>>();
        assert!(
            ExampleGroup::ALL.is_sorted(),
            "ALL is not the declared order"
        );
        assert!(
            groups.is_sorted(),
            "a section is split or out of order: {groups:?}"
        );
        for group in ExampleGroup::ALL {
            assert!(groups.contains(&group), "{} is empty", group.label());
        }
        let names = catalog()
            .iter()
            .map(|example| example.name)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(names.len(), catalog().len());
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

    /// The guide scene compiles and holds what its steps ask for: the round
    /// region in the background material, the glass in the library, the
    /// obstacle, and a source that is on.
    #[test]
    fn the_guide_scene_holds_what_its_steps_ask_for() {
        let document = guide_scene();
        let draft = &document.model.draft;
        let region = draft.region(GUIDE_REGION).expect("the region to repaint");
        assert_eq!(region.material, DEFAULT_MATERIAL);
        assert_eq!(draft.geometry.curves[0].id, GUIDE_OBSTACLE);
        assert_eq!(
            draft.outer_boundaries.sides[OuterSide::Right.index()],
            OuterBoundaryCondition::Reflecting,
            "the right wall echoes until the tour has it made outgoing"
        );
        assert!(
            draft
                .materials
                .iter()
                .any(|material| material.id == GUIDE_GLASS)
        );
        assert_eq!(
            draft.geometry.curves.len(),
            2,
            "the obstacle and the region"
        );
        assert!(document.model.source.enabled);
        let prepared = prepare(&document, 0.16);
        assert!(prepared.mesh.triangles.len() > 100);
    }

    /// A scene drawn as streamlines shows the energy flow, the one field the
    /// lines are of; a style without its mode would be a choice that draws
    /// nothing different.
    #[test]
    fn every_streamline_scene_shows_the_energy_flow() {
        let mut lined = Vec::new();
        for example in catalog() {
            let presentation = &example.document.presentation;
            if presentation.vector_overlay_style == VectorOverlayStyle::Streamlines {
                assert_eq!(
                    presentation.vector_overlay,
                    VectorOverlay::RelativeEnergyFlow,
                    "{}",
                    example.name
                );
                lined.push(example.name);
            }
        }
        assert_eq!(
            lined,
            [
                "Double slit",
                "Phased array",
                "Material lens",
                "GRIN collimator",
                "Luneburg lens",
                "Fresnel zone plate",
                "Frustrated total internal reflection",
                "Crystal bend",
                "Acoustic whispering gallery",
                "Spatial soliton",
            ]
        );
    }

    /// Every gallery probe opens on what its claim reads: one or two plots of
    /// its own kind, chosen by the scene rather than the defaults, and the far
    /// field on its polar pattern alone. No readout names a probe the scene
    /// does not have.
    #[test]
    fn every_gallery_probe_opens_on_one_or_two_plots() {
        for example in catalog() {
            let document = &example.document;
            for id in document.readouts.probes.keys() {
                assert!(
                    document.model.probes.iter().any(|probe| probe.id == *id),
                    "{}: a readout for probe {id:?}, which it does not have",
                    example.name
                );
            }
            for probe in &document.model.probes {
                let readout = document.readouts.probes.get(&probe.id).unwrap_or_else(|| {
                    panic!("{}: {} opens on the defaults", example.name, probe.name)
                });
                let plots = match probe.target {
                    TopologyProbeTarget::Point(_) => [
                        readout.field,
                        readout.secondary_field,
                        readout.transverse_field,
                        readout.poynting,
                        readout.energy,
                    ]
                    .iter()
                    .filter(|shown| **shown)
                    .count(),
                    TopologyProbeTarget::Segment { .. } | TopologyProbeTarget::Boundary(_) => {
                        readout.line_plots.iter().filter(|shown| **shown).count()
                    }
                    TopologyProbeTarget::AreaDisk { .. } | TopologyProbeTarget::AreaRegion(_) => [
                        readout.area_mean_field,
                        readout.area_rms_field,
                        readout.area_rms_transverse,
                        readout.area_mean_energy,
                        readout.area_total_energy,
                    ]
                    .iter()
                    .filter(|shown| **shown)
                    .count(),
                };
                assert!(
                    (1..=2).contains(&plots),
                    "{}: {} opens on {plots} plots",
                    example.name,
                    probe.name
                );
            }
            if document.model.far_field.enabled {
                assert_eq!(
                    document.readouts.far_field,
                    polar_readout(),
                    "{}",
                    example.name
                );
            }
        }
    }

    /// No gallery probe sits on another. A point probe keeps clear of every
    /// line and boundary probe, and of every other point, by more than its
    /// marker: 0.04 is 15 pixels in the fitted view of a 2 × 2 domain. No two
    /// line probes cross. Each marker then stays one to click and to read.
    #[test]
    fn no_gallery_probe_sits_on_another() {
        use crate::topology_viewport::{
            SampledTopologyGeometry, ScreenPoint, TopologySpanTarget, ViewportTransform,
        };
        const CLEARANCE: f64 = 0.04;
        let distance = |point: Point2, [a, b]: [Point2; 2]| {
            let along = b - a;
            let t = ((point - a).dot(along) / along.dot(along)).clamp(0.0, 1.0);
            (point - a.lerp(b, t)).norm()
        };
        let crosses = |a: [Point2; 2], b: [Point2; 2]| {
            let side = |point: Point2, [p, q]: [Point2; 2]| (q - p).cross(point - p);
            side(b[0], a) * side(b[1], a) < 0.0 && side(a[0], b) * side(a[1], b) < 0.0
        };
        for example in catalog() {
            let geometry = &example.document.model.draft.geometry;
            let transform = ViewportTransform {
                screen_center: ScreenPoint::new(0.0, 0.0),
                world_center: geometry.domain.center(),
                pixels_per_world: 400.0,
            };
            let sampled = SampledTopologyGeometry::new(geometry, transform, 0.5).unwrap();
            let mut points = vec![];
            let mut lines = vec![];
            for probe in &example.document.model.probes {
                let name = probe.name.as_str();
                match &probe.target {
                    TopologyProbeTarget::Point(point) => points.push((name, *point)),
                    TopologyProbeTarget::Segment { start, end, .. } => {
                        lines.push((name, vec![[*start, *end]]))
                    }
                    TopologyProbeTarget::Boundary(target) => {
                        let path = target
                            .spans
                            .iter()
                            .flat_map(|span| {
                                sampled
                                    .spans
                                    .iter()
                                    .filter(|sampled| {
                                        sampled.target == TopologySpanTarget::Curve(*span)
                                    })
                                    .flat_map(|sampled| {
                                        sampled.samples.iter().map(|sample| sample.point)
                                    })
                            })
                            .collect::<Vec<_>>();
                        assert!(path.len() > 1, "{}: {name} has no path", example.name);
                        lines.push((
                            name,
                            path.windows(2).map(|pair| [pair[0], pair[1]]).collect(),
                        ));
                    }
                    TopologyProbeTarget::AreaDisk { .. } | TopologyProbeTarget::AreaRegion(_) => {}
                }
            }
            for (index, (name, point)) in points.iter().enumerate() {
                for (other, at) in &points[index + 1..] {
                    assert!(
                        (*point - *at).norm() > CLEARANCE,
                        "{}: {name} sits on {other}",
                        example.name
                    );
                }
                for (other, segments) in &lines {
                    let gap = segments
                        .iter()
                        .map(|segment| distance(*point, *segment))
                        .fold(f64::INFINITY, f64::min);
                    assert!(
                        gap > CLEARANCE,
                        "{}: {name} is {gap:.3} from {other}",
                        example.name
                    );
                }
            }
            for (index, (name, segments)) in lines.iter().enumerate() {
                for (other, others) in &lines[index + 1..] {
                    assert!(
                        !segments
                            .iter()
                            .any(|a| others.iter().any(|b| crosses(*a, *b))),
                        "{}: {name} crosses {other}",
                        example.name
                    );
                }
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
        let (mut series, dt, strongest) = traces(document, edge, seconds, &[point]);
        (series.pop().unwrap(), dt, strongest)
    }

    /// `trace` at each of `points`, from one run.
    fn traces(
        document: &TopologyDocument,
        edge: f64,
        seconds: f64,
        points: &[Point2],
    ) -> (Vec<Vec<f64>>, f64, f64) {
        let prepared = prepare(document, edge);
        let operator = prepared
            .canonical_temporal_operator
            .clone()
            .expect("a law-carrying document prepares a temporal operator");
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
        let steps = (seconds / dt).ceil() as usize;
        let mut series = vec![Vec::with_capacity(steps); points.len()];
        let mut strongest = 0.0_f64;
        for step in 0..steps {
            state.step_with_forcing(&operator, &forcing).unwrap();
            let field = operator
                .primary_field_at(state.primary_flux(), state.time(), state.runtime())
                .unwrap();
            for (series, node) in series.iter_mut().zip(&nodes) {
                series.push(field[*node]);
            }
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

    /// What a point probe's readout shows of its field's spectrum.
    struct Lines {
        spectrum: Spectrum,
        span: f64,
    }

    impl Lines {
        /// The spectrum `probe`'s readout draws at the end of `series`,
        /// recorded every `dt` at the probe's point: its own span, through the
        /// transform the readout uses, the span clamped to what was recorded
        /// as the readout clamps it. The readout must show the field's
        /// spectrum up to at least `top_hz`.
        fn read(
            document: &TopologyDocument,
            probe: ProbeId,
            series: &[f64],
            dt: f64,
            top_hz: f64,
        ) -> Self {
            let readout = document.readouts.probe(probe);
            assert!(readout.field && readout.field_spectrum);
            assert!(readout.spectrum_max_hz >= top_hz);
            let count = ((readout.span / dt).round() as usize).min(series.len());
            Self {
                spectrum: amplitude_spectrum(&series[series.len() - count..], dt).unwrap(),
                span: readout.span,
            }
        }

        /// The height of the line at `hz`: the largest magnitude within a
        /// quarter of the Hann window's main lobe of it.
        fn at(&self, hz: f64) -> f64 {
            self.strongest(hz - 1.0 / self.span, hz + 1.0 / self.span).1
        }

        /// The strongest line between `low` and `high` Hz, and its height.
        fn strongest(&self, low: f64, high: f64) -> (f64, f64) {
            (0..self.spectrum.magnitudes.len())
                .map(|index| {
                    (
                        self.spectrum.frequency_hz(index),
                        self.spectrum.magnitudes[index],
                    )
                })
                .filter(|(frequency, _)| (low..=high).contains(frequency))
                .fold(
                    (low, 0.0),
                    |best, next| if next.1 > best.1 { next } else { best },
                )
        }
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

    /// The RMS width of the time-averaged intensity across `x`.
    fn beam_width(scene: &Harmonic, x: f64) -> f64 {
        let (mut total, mut moment) = (0.0, 0.0);
        for index in 0..=190 {
            let y = -0.95 + index as f64 * 0.01;
            let (re, im) = scene.interpolated(Point2::new(x, y));
            total += re * re + im * im;
            moment += y * y * (re * re + im * im);
        }
        (moment / total).sqrt()
    }

    /// The spatial soliton's claims, against the same beam at a hundredth of
    /// the strength, which the slab does not see: at the far face the strong
    /// beam is under 0.6 of the weak one's width (0.47); and across the
    /// slab's second half it widens by under a fifth (2%), where the weak
    /// beam widens by over 40% (53%), so it is held, not focused.
    #[test]
    fn a_strong_beam_holds_its_width_where_a_weak_one_spreads() {
        let widths = |amplitude| {
            let scene = Harmonic::run(&spatial_soliton_with(amplitude), 0.08, 3.0, SOLITON_HZ, 4.0);
            [0.0, 0.6].map(|x| beam_width(&scene, x))
        };
        let strong = widths(SOLITON_AMPLITUDE);
        let weak = widths(SOLITON_AMPLITUDE / 100.0);
        let report = format!("strong {strong:.4?}, weak {weak:.4?}");
        assert!(strong[1] / weak[1] < 0.6, "{report}");
        assert!(strong[1] / strong[0] < 1.2, "{report}");
        assert!(weak[1] / weak[0] > 1.4, "{report}");
    }

    /// The Doppler mirror's claims, from the lines at 1 to 5 Hz in front of
    /// the slab and behind it. In front, the reflection at 3 Hz is over half
    /// the carrier (1.04) and over five times any other line (7.3 times), and
    /// with the grating at rest under a hundredth of it (4e-4). Wave quanta
    /// are conserved where energy is not: the transmitted power and a third
    /// of the reflected make the incident power within 5% (0.99), while the
    /// two powers make over 1.5 times it (1.71), the grating's work.
    #[test]
    fn a_grating_running_at_half_the_wave_speed_reflects_at_three_times_the_frequency() {
        let lines = |pump_hz| {
            let (series, dt, _) = traces(
                &doppler_mirror_with(pump_hz),
                0.08,
                8.0,
                &[DOPPLER_FRONT, DOPPLER_BEHIND],
            );
            let spectrum = |series: &Vec<f64>| {
                // A Hann-weighted mean: the plain one over the 3.997 s that
                // whole steps of the 1.2× step make of 4 s leaked 1.3e-3 of
                // the carrier and its 3 Hz reflection into the static field.
                let window = &series[series.len() - (4.0 / dt).round() as usize..];
                let weights = window.iter().enumerate().map(|(index, _)| {
                    (std::f64::consts::PI * index as f64 / (window.len() - 1) as f64)
                        .sin()
                        .powi(2)
                });
                let mean = window
                    .iter()
                    .zip(weights.clone())
                    .map(|(x, w)| x * w)
                    .sum::<f64>()
                    / weights.sum::<f64>();
                let lines = [1.0, 2.0, 3.0, 4.0, 5.0]
                    .map(|n| amplitude_at(series, dt, n * DOPPLER_HZ, 4.0 * n));
                // The launcher switches on as a cosine, so the channel holds
                // no static field; on a sine it held one as large as the wave
                // (0.191 against 0.19), which the grating made into 2 and 4 Hz
                // lines.
                std::assert!(
                    mean.abs() < 1e-3 * lines[0],
                    "a static field of {mean:.4} beside a carrier of {:.4}",
                    lines[0]
                );
                lines
            };
            (
                [spectrum(&series[0]), spectrum(&series[1])],
                Lines::read(
                    &doppler_mirror_with(pump_hz),
                    ProbeId(1),
                    &series[0],
                    dt,
                    3.0,
                ),
            )
        };
        let ([front, behind], readout) = lines(2.0 * DOPPLER_HZ);
        let ([rest, _], _) = lines(0.0);
        // The front probe's readout shows it as the front probe's lines do.
        assert!(readout.at(3.0) > 0.5 * readout.at(1.0));
        for other in [2.0, 4.0, 5.0] {
            assert!(readout.at(3.0) > 5.0 * readout.at(other), "{other} Hz");
        }
        let report = format!("front {front:.4?}, behind {behind:.4?}, at rest {rest:.4?}");
        let reflected = front[2] / front[0];
        assert!(reflected > 0.5, "{report}");
        for other in [1, 3, 4] {
            assert!(front[2] > 5.0 * front[other], "{report}");
        }
        assert!(rest[2] < 0.01 * rest[0], "{report}");
        let transmitted = (behind[0] / front[0]).powi(2);
        let returned = reflected.powi(2);
        assert!(
            (transmitted + returned / 3.0 - 1.0).abs() < 0.05,
            "{report}: quanta {:.4}",
            transmitted + returned / 3.0
        );
        assert!(transmitted + returned > 1.5, "{report}");
    }

    /// The Kerr gallery claim: behind the slab, the receiver hears the
    /// source's third harmonic, which a medium with the same geometry and no
    /// response does not make; and the slab's coefficient moves by about 15%,
    /// which is what the material readout will show. At the gallery's edge,
    /// 0.08, with its short-wave loss: the third harmonic at 0.16 of the
    /// fundamental (0.21 without the loss), the linear slab at 4e-4, and the
    /// coefficient's swing 0.149 (0.232 without it, the harmonics the loss
    /// trims having added to the field's peaks). At edge 0.15 the
    /// fundamental itself, at under three elements a wavelength, sits near
    /// the ceiling the loss acts on, and lost 13% to it.
    #[test]
    fn the_kerr_slab_generates_its_third_harmonic() {
        let receiver = Point2::new(0.55, 0.0);
        let run = |chi: f64| {
            let (series, dt, strongest) = trace(&kerr_slab_with(chi, 60.0), 0.08, 4.0, receiver);
            let ratio = amplitude_at(&series, dt, 7.5, 9.0) / amplitude_at(&series, dt, 2.5, 3.0);
            (ratio, strongest, series, dt)
        };
        let (kerr, strongest, series, dt) = run(40.0);
        let (linear, ..) = run(0.0);
        assert!(kerr > 0.1, "third harmonic {kerr:.3e} of the fundamental");
        assert!(linear < 0.01, "a linear slab made {linear:.3e}");
        assert!(strongest > 0.1, "the slab moved only {strongest:.3}");
        // The receiver's readout shows it at 0.16 of the fundamental, and
        // no second harmonic: 28 times the level at 5 Hz.
        let lines = Lines::read(&kerr_slab(), ProbeId(1), &series, dt, 7.5);
        assert!(lines.at(7.5) > 0.1 * lines.at(2.5));
        assert!(lines.at(7.5) > 5.0 * lines.at(5.0));
    }

    /// The slab's short-wave loss trims what the mesh makes of its harmonics
    /// and keeps the signal. At the gallery's edge, 0.08, the bare 12.5 Hz
    /// line stands above the third harmonic, so it is mostly the mesh's; the
    /// loss takes it to 0.13 of itself and keeps 0.93 of the 2.5 Hz signal.
    #[test]
    fn the_kerr_slabs_short_wave_loss_trims_what_the_mesh_makes() {
        let receiver = Point2::new(0.55, 0.0);
        let read = |document: &TopologyDocument| {
            let (series, dt, _) = trace(document, 0.08, 4.0, receiver);
            let at = |hz: f64| amplitude_at(&series, dt, hz, 1.5 * hz);
            (at(2.5), at(12.5))
        };
        let trimmed = kerr_slab();
        let mut bare = trimmed.clone();
        for scene in [&mut bare.model.draft, &mut bare.model.accepted] {
            scene
                .materials
                .iter_mut()
                .for_each(|material| material.short_wave_loss = 0.0);
        }
        let ((signal, rung), (bare_signal, bare_rung)) = (read(&trimmed), read(&bare));
        assert!(
            rung < 0.25 * bare_rung,
            "the 12.5 Hz line reads {rung:.3e} against {bare_rung:.3e} without the loss"
        );
        assert!(
            signal > 0.9 * bare_signal,
            "the signal keeps {:.3} of itself",
            signal / bare_signal
        );
    }

    /// The size rule's report on `document` after `seconds` from rest at
    /// `edge`, estimated the way the app estimates it: the field, its rate
    /// and its acceleration from the last three steps, with the instantaneous
    /// materials attached and the app's adaptation defaults.
    fn size_rule_report(
        document: &TopologyDocument,
        edge: f64,
        seconds: f64,
        frequency: f64,
    ) -> SolutionIndicatorReport {
        let prepared = prepare(document, edge);
        let operator = prepared
            .canonical_temporal_operator
            .clone()
            .expect("a field law prepares a temporal operator");
        let forcing = prepared.canonical_forcing.clone();
        let dt = prepared.recommended_time_step();
        let mut state = CanonicalTemporalWaveState::zero(&operator, dt)
            .unwrap()
            .pinned(&operator, &forcing)
            .unwrap();
        let mut fields = std::collections::VecDeque::new();
        for _ in 0..(seconds / dt).ceil() as usize {
            state.step_with_forcing(&operator, &forcing).unwrap();
            fields.push_back(
                operator
                    .primary_field_at(state.primary_flux(), state.time(), state.runtime())
                    .unwrap(),
            );
            if fields.len() > 3 {
                fields.pop_front();
            }
        }
        let (before, now, after) = (&fields[0], &fields[1], &fields[2]);
        let count = now.len();
        let snapshot = QuadraticSolutionSnapshot {
            mesh_revision: prepared.mesh.mesh_revision,
            displacement: now.clone(),
            velocity: (0..count)
                .map(|node| (after[node] - before[node]) / (2.0 * dt))
                .collect(),
            acceleration: (0..count)
                .map(|node| (after[node] - 2.0 * now[node] + before[node]) / (dt * dt))
                .collect(),
            volume_acceleration: vec![0.0; count],
            auxiliary: vec![0.0; count],
            time: state.time() - dt,
            time_step: dt,
        };
        let defaults = crate::document::AdaptationSettings::default();
        let mut job = SolutionIndicatorJob::new_topology(
            prepared.mesh.clone(),
            prepared.operator.clone(),
            &prepared.bundle.plan,
            prepared.bundle.model(),
            snapshot,
            SolutionIndicatorOptions {
                minimum_edge_length: defaults.minimum_edge,
                maximum_edge_length: defaults.maximum_edge,
                relative_tolerance: defaults.accuracy_percent / 100.0,
                elements_per_wavelength: defaults.elements_per_wavelength,
                forcing_frequency_hz: frequency,
                ..Default::default()
            },
        )
        .with_instantaneous_materials(state.runtime().clone());
        loop {
            if let Some(result) = job.advance(1 << 16) {
                break result.unwrap().report;
            }
        }
    }

    /// Steps a law-carrying generation 40 steps on the CPU reference with its
    /// own forcing and estimates it along the adaptation worker's path.
    fn estimate_as_the_worker_does(
        name: &str,
        prepared: &crate::topology_runtime::PreparedTopology,
    ) {
        let operator = prepared.canonical_temporal_operator.clone().unwrap();
        let forcing = prepared.canonical_forcing.clone();
        let dt = prepared.recommended_time_step();
        let mut state = CanonicalTemporalWaveState::zero(&operator, dt)
            .unwrap()
            .pinned(&operator, &forcing)
            .unwrap();
        let auxiliary_of = |state: &CanonicalTemporalWaveState| {
            state
                .thin_gap_jump()
                .iter()
                .chain(state.outgoing_pole_currents())
                .copied()
                .collect::<Vec<_>>()
        };
        let mut previous = state.clone();
        for _ in 0..40 {
            previous = state.clone();
            state
                .step_with_forcing(&operator, &forcing)
                .unwrap_or_else(|error| panic!("{name}: step: {error}"));
        }
        let snapshot = funfern_core::CanonicalIndicatorSnapshot {
            mesh_revision: prepared.mesh.mesh_revision,
            primary_flux: state.primary_flux().to_vec(),
            previous_primary_flux: previous.primary_flux().to_vec(),
            complementary_flux: state.complementary_flux().to_vec(),
            previous_complementary_flux: previous.complementary_flux().to_vec(),
            auxiliary: auxiliary_of(&state),
            previous_auxiliary: auxiliary_of(&previous),
            integrated_field: state.integrated_field().to_vec(),
            previous_integrated_field: previous.integrated_field().to_vec(),
            time: state.time(),
            time_step: dt,
        };
        let supplement = funfern_core::canonical_temporal_indicator_supplement(
            &prepared.mesh,
            &operator,
            &forcing,
            &snapshot,
            state.runtime(),
            0.0,
        )
        .unwrap_or_else(|error| panic!("{}: supplement: {error}", name));
        let velocity = funfern_core::canonical_temporal_primary_rate(
            &operator,
            &forcing,
            &snapshot,
            state.runtime(),
        )
        .unwrap_or_else(|error| panic!("{}: rate: {error}", name));
        let displacement = operator
            .primary_field_at(state.primary_flux(), state.time(), state.runtime())
            .unwrap();
        let count = displacement.len();
        let defaults = crate::document::AdaptationSettings::default();
        let mut job = SolutionIndicatorJob::new_topology(
            prepared.mesh.clone(),
            prepared.operator.clone(),
            &prepared.bundle.plan,
            prepared.bundle.model(),
            QuadraticSolutionSnapshot {
                mesh_revision: prepared.mesh.mesh_revision,
                displacement,
                velocity,
                acceleration: vec![0.0; count],
                auxiliary: vec![0.0; count],
                volume_acceleration: vec![0.0; count],
                time: state.time(),
                time_step: dt,
            },
            SolutionIndicatorOptions {
                minimum_edge_length: defaults.minimum_edge,
                maximum_edge_length: defaults.maximum_edge,
                relative_tolerance: defaults.accuracy_percent / 100.0,
                elements_per_wavelength: defaults.elements_per_wavelength,
                ..Default::default()
            },
        )
        .with_canonical_supplement(supplement)
        .with_instantaneous_materials(state.runtime().clone());
        let report = loop {
            if let Some(result) = job.advance(1 << 16) {
                break result
                    .unwrap_or_else(|error| panic!("{}: estimate: {error}", name))
                    .report;
            }
        };
        assert!(
            report.global_indicator.is_finite(),
            "{}: the estimate is {}",
            name,
            report.global_indicator
        );
    }

    /// A pinned or electric-wall side beside a second-order outgoing one pins
    /// the corner nodes they share, which sit on the outgoing wall's trace.
    /// Every law-carrying gallery scene behind such a wall used to be refused
    /// whole; with one side pinned, each steps on the reference, packs for the
    /// device and is estimated as the adaptation worker estimates it.
    #[test]
    fn every_driven_gallery_scene_runs_with_a_side_pinned_beside_an_outgoing_wall() {
        let outgoing = OuterBoundaryCondition::SecondOrderOutgoing;
        let mut ran = Vec::new();
        for example in catalog() {
            let mut document = example.document.clone();
            let sides = document.model.accepted.outer_boundaries.sides;
            // A side with an outgoing neighbour and no pinned one, whose own
            // signal would have to agree at their corner.
            let pinned_side =
                |side: usize| matches!(sides[side], OuterBoundaryCondition::Dirichlet { .. });
            let Some(side) = (0..4).find(|side| {
                let neighbours = [(side + 1) % 4, (side + 3) % 4];
                neighbours.iter().any(|next| sides[*next] == outgoing)
                    && !neighbours.iter().any(|next| pinned_side(*next))
            }) else {
                continue;
            };
            let pinned = OuterBoundaryCondition::Dirichlet {
                signal: TimeSignal::ZERO,
            };
            for scene in [&mut document.model.draft, &mut document.model.accepted] {
                scene.outer_boundaries.sides[side] = pinned;
            }
            let prepared = prepare(&document, 0.16);
            let Some(operator) = prepared.canonical_temporal_operator.clone() else {
                continue;
            };
            // A scene cut away from the domain's walls, as the pumped drum
            // is, has no outgoing trace and no such corner.
            let Some(boundary) = operator.base().outgoing_boundary() else {
                continue;
            };
            assert!(
                boundary
                    .trace_nodes()
                    .iter()
                    .any(|node| prepared.canonical_forcing.prescribed()[*node as usize].is_some()),
                "{}: no pin on the trace",
                example.name
            );
            let time_step = prepared.recommended_time_step();
            let state = CanonicalTemporalWaveState::zero(&operator, time_step)
                .unwrap()
                .pinned(&operator, &prepared.canonical_forcing)
                .unwrap();
            crate::canonical_gpu::CanonicalGpuPlan::compile_temporal(
                &operator,
                &state,
                &prepared.canonical_forcing,
                crate::canonical_gpu::CanonicalGpuClock::initial(time_step).unwrap(),
            )
            .unwrap_or_else(|error| panic!("{}: device plan: {error:?}", example.name));
            estimate_as_the_worker_does(example.name, &prepared);
            ran.push(example.name);
        }
        assert!(ran.len() >= 9, "{ran:?}");
    }

    /// Every gallery scene whose medium carries a law is estimated the way the
    /// application's adaptation worker estimates it: production's rate, the
    /// temporal supplement, the scene's own walls and the instantaneous
    /// materials. Eight of them - behind absorbing walls, with a prescribed
    /// side, or with loss - used to be refused.
    #[test]
    fn every_driven_gallery_scene_is_estimated() {
        let mut estimated = Vec::new();
        for example in catalog() {
            let prepared = prepare(&example.document, 0.16);
            if prepared.canonical_temporal_operator.is_some() {
                estimate_as_the_worker_does(example.name, &prepared);
                estimated.push(example.name);
            }
        }
        for name in [
            "Parametric pump",
            "Time crystal",
            "Travelling modulation",
            "Kerr slab",
            "Josephson line",
            "Symmetry breaking",
            "Pinned domain wall",
            "Self-sustained emitter",
        ] {
            assert!(estimated.contains(&name), "{name} was not estimated");
        }
    }

    /// The Kerr gallery scene's field is strong enough in the slab to make
    /// odd harmonics of the source, and the size rule asks for them there;
    /// the same slab lit ten times more weakly makes none worth resolving.
    #[test]
    fn the_kerr_slab_asks_the_mesh_for_the_harmonics_it_makes() {
        let strong = size_rule_report(&kerr_slab(), 0.15, 3.0, 2.5);
        assert!(
            strong.field_law_harmonics >= 1,
            "tangent {:.3}, harmonics {}",
            strong.largest_field_tangent,
            strong.field_law_harmonics
        );
        let weak = size_rule_report(&kerr_slab_with(40.0, 6.0), 0.15, 3.0, 2.5);
        assert_eq!(weak.field_law_harmonics, 0);
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
    /// scan, which sit at the same phases on a finer mesh (see the log);
    /// they moved by half a turn when the source's start moved by a quarter.
    #[test]
    fn a_pump_at_twice_the_source_frequency_amplifies_by_phase() {
        let (best, worst) = (pump_gain(5.0, 5.0), pump_gain(5.0, 1.0));
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
        sidebands_and_lines(document, receiver).0
    }

    /// `sidebands`, and what the receiver's readout shows.
    fn sidebands_and_lines(document: &TopologyDocument, receiver: Point2) -> ([f64; 3], Lines) {
        let (series, dt, _) = trace(document, 0.15, 8.0, receiver);
        let at = |hz: f64| amplitude_at(&series, dt, hz, 4.0 * hz);
        let carrier = at(2.5);
        (
            [1.5, 3.5, 5.5].map(|hz| at(hz) / carrier),
            Lines::read(document, ProbeId(1), &series, dt, 5.5),
        )
    }

    /// The time-crystal gallery claim: the slab splits the wave into
    /// sidebands, and its sharp edges put several times more into the third
    /// one than a sinusoidal pump of the same depth and frequency does.
    #[test]
    fn a_time_crystal_reaches_further_sidebands_than_a_pump() {
        let receiver = Point2::new(0.5, 0.0);
        let (crystal, lines) = sidebands_and_lines(&time_crystal_slab(), receiver);
        // The receiver's readout shows the three at 0.20, 0.88 and 0.20 of
        // the carrier.
        for (hz, least) in [(1.5, 0.1), (3.5, 0.3), (5.5, 0.1)] {
            assert!(lines.at(hz) > least * lines.at(2.5), "{hz} Hz");
        }
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
        let (with, lines) = sidebands_and_lines(&travelling_slab(), Point2::new(0.75, 0.0));
        // The receiver's readout shows the carrier outdone by the lines above
        // it: 3.5 Hz at 1.7 times it, 5.5 Hz at 7.5 times.
        assert!(lines.strongest(0.2, 8.0).0 > 3.0);
        assert!(lines.at(3.5) > lines.at(2.5));
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
                    gate: None,
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
    /// the near zone, so its fringes sit a little inside those angles. Nothing
    /// leaves the box but through the slits: straight behind it the far field
    /// holds 0.023 of the centre, where it held 0.25 while the source's
    /// Gaussian reached through the back wall, 1.4 widths away.
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
        assert!(behind < 0.05, "{behind:.3} went back round the box");
        let screen = scene.along(Point2::new(0.85, 0.0), Point2::new(0.85, 0.85), 18);
        let dark = screen[5..9].iter().cloned().fold(f64::MAX, f64::min) / screen[0];
        let bright = screen[12..16].iter().cloned().fold(0.0, f64::max) / screen[0];
        assert!(
            dark < 0.4,
            "the first dark fringe holds {dark:.3} of the centre"
        );
        assert!(bright > 0.6, "the first bright fringe holds {bright:.3}");
    }

    /// The share of a cut's power across the domain at `x` that lies within
    /// the rod's aperture, `|y| < 0.3`.
    fn aperture_share(scene: &Harmonic, x: f64) -> f64 {
        let line = scene.along(Point2::new(x, -0.9), Point2::new(x, 0.9), 37);
        let total: f64 = line.iter().map(|value| value * value).sum();
        let inside: f64 = line[12..=24].iter().map(|value| value * value).sum();
        inside / total
    }

    /// The collimator gallery claim: behind the rod the beam keeps most of its
    /// power within the aperture, and no less further on, where the bare
    /// source's spreads across the domain and keeps less the further it goes.
    /// No half-amplitude width is claimed: the profile has a shoulder either
    /// side near half the peak, and which side of half it falls on moves the
    /// width between 0.45 and 0.75 along the beam.
    #[test]
    fn the_grin_collimator_sends_out_a_beam_that_does_not_spread() {
        let rod = Harmonic::run(&grin_rod(), 0.08, 6.0, 4.0, 3.0);
        let bare = Harmonic::run(&grin_rod_with(0.0), 0.08, 6.0, 4.0, 3.0);
        let (near, far) = (aperture_share(&rod, 0.2), aperture_share(&rod, 0.75));
        let (bare_near, bare_far) = (aperture_share(&bare, 0.2), aperture_share(&bare, 0.75));
        assert!(far > 0.7, "the rod's beam keeps {far:.2} in the aperture");
        assert!(
            far >= near,
            "the rod's beam went from {near:.3} to {far:.3}"
        );
        assert!(bare_far < 0.5, "the bare source keeps {bare_far:.2}");
        assert!(
            bare_far < bare_near,
            "the bare source went from {bare_near:.3} to {bare_far:.3}"
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
        assert_eq!(points[2], JOSEPHSON_PROBE);
        let field = rows.iter().map(|row| row.2[2]).collect::<Vec<_>>();
        let lines = Lines::read(
            &josephson_line(),
            ProbeId(1),
            &field,
            rows[1].0 - rows[0].0,
            JOSEPHSON_BIAS / tau,
        );
        // Its readout hears the fluxons as a tone at V/2π, the Josephson
        // frequency, 0.46 Hz in view against 0.477, and its harmonics.
        let (strongest, _) = lines.strongest(0.1, 3.0);
        assert!((strongest - JOSEPHSON_BIAS / tau).abs() < 1.0 / lines.span);
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
            let editor = crate::topology_editor::TopologyEditor::from_document(document).unwrap();
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
                    // As prepared: the commit releases the handoff maps the
                    // drag below carries the state through.
                    let prepared = Arc::new(runtime.ready().unwrap().clone());
                    runtime.commit_ready(token).unwrap();
                    return prepared;
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
        assert_eq!(points[1], EMITTER_PROBE);
        // The probe there shows its strongest line within a quarter of the
        // window's main lobe of the cutoff, at 2.99 Hz.
        let lines = Lines::read(&emitter(), ProbeId(1), &far, dt, EMITTER_HZ);
        let (strongest, _) = lines.strongest(0.2, 5.0);
        assert!((strongest - EMITTER_HZ).abs() < 1.0 / lines.span);
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
    /// 2.5 Hz over 2 s windows. It is a toy model, so the claims are its
    /// physics rather than a gain figure. The pump running with the signal
    /// amplifies it more than 5× against the pump off (7.19×), and steadily:
    /// 6 s later the gain is the same within 10% (7.18×). Advanced by half a
    /// turn, the same pump squeezes it below half (0.15). The same pump
    /// uniform in space makes the fiber an oscillator: its far-end field
    /// grows more than 2× from 8 s to 12 s (2.37×; the short-wave loss slows
    /// the growth, which was 3× and more without it).
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
        assert!(gain > 5.0, "the pump amplifies {gain:.3}×");
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
            standing[1] > 2.0 * standing[0],
            "a standing pump's far end went from {:.4} to {:.4}",
            standing[0],
            standing[1]
        );
    }

    /// The fiber's short-wave loss trims the sum-frequency rungs the mesh
    /// cannot carry: at edge 0.08, read at mid fiber over 4 s to 8 s against
    /// the signal there, the 12.5 Hz rung falls to under a third of itself
    /// (0.046 against 0.242), while the pumped far end keeps more than a
    /// quarter of what it reaches without it (35%; 56% at edge 0.04, where
    /// less of the signal sits near the ceiling).
    #[test]
    fn the_fibers_short_wave_loss_trims_its_unresolved_rungs() {
        let points = [Point2::new(0.0, 0.0), Point2::new(0.85, 0.0)];
        let read = |document: &TopologyDocument| {
            let rows = integrated_traces(document, 0.08, 8.0, &points, 0.0);
            let dt = rows[1].0 - rows[0].0;
            let series = |index: usize| rows.iter().map(|row| row.2[index]).collect::<Vec<_>>();
            let amplitude = |series: &[f64], hz: f64, window: f64| {
                let (re, im) = phasor(series, dt, hz, window);
                re.hypot(im)
            };
            let (middle, far) = (series(0), series(1));
            (
                amplitude(&middle, 12.5, 4.0) / amplitude(&middle, FIBER_SIGNAL_HZ, 4.0),
                amplitude(&far, FIBER_SIGNAL_HZ, 2.0),
            )
        };
        let trimmed = fiber_amplifier();
        let mut bare = trimmed.clone();
        for scene in [&mut bare.model.draft, &mut bare.model.accepted] {
            scene
                .materials
                .iter_mut()
                .for_each(|material| material.short_wave_loss = 0.0);
        }
        let ((rung, far), (bare_rung, bare_far)) = (read(&trimmed), read(&bare));
        assert!(
            rung < 0.33 * bare_rung,
            "the 12.5 Hz rung reads {rung:.3} against {bare_rung:.3} without the loss"
        );
        assert!(
            far > 0.25 * bare_far,
            "the far end keeps {:.3} of its signal",
            far / bare_far
        );
    }

    /// The pump's travelling pattern sizes the fiber and nothing else. The
    /// estimate at the scene's own settings, edge 0.08, on a quiet field holds
    /// every fiber element to the pattern's floor, `2π/36.2` over six
    /// elements, 0.029, and leaves the vacuum to the 2.5 Hz wave, 0.067. Held
    /// to the shortest pattern anywhere, every one of the 2,448 vacuum
    /// elements of the user's run targeted under 0.035 and the whole domain
    /// refined towards the fiber's floor while the error read 2% of a 12%
    /// target.
    #[test]
    fn the_fibers_pump_pattern_sizes_only_the_fiber() {
        let document = fiber_amplifier();
        let settings = document.presentation.adaptation;
        let prepared = prepare(&document, document.presentation.mesh_edge);
        let patterns = funfern_core::CanonicalTemporalResolution::of_each_material(
            &prepared.bundle.authored.materials,
        )
        .unwrap();
        assert_eq!(patterns.len(), 1);
        let floor =
            std::f64::consts::TAU / FIBER_PUMP_WAVENUMBER / settings.elements_per_wavelength;
        let count = prepared.operator.degrees_of_freedom();
        let mut job = SolutionIndicatorJob::new_topology(
            prepared.mesh.clone(),
            prepared.operator.clone(),
            &prepared.bundle.plan,
            prepared.bundle.model(),
            QuadraticSolutionSnapshot {
                mesh_revision: prepared.mesh.mesh_revision,
                displacement: vec![0.0; count],
                velocity: vec![0.0; count],
                acceleration: vec![0.0; count],
                auxiliary: vec![0.0; count],
                volume_acceleration: vec![0.0; count],
                time: 0.0,
                time_step: prepared.recommended_time_step(),
            },
            SolutionIndicatorOptions {
                minimum_edge_length: settings.minimum_edge,
                maximum_edge_length: settings.maximum_edge,
                relative_tolerance: settings.accuracy_percent / 100.0,
                elements_per_wavelength: settings.elements_per_wavelength,
                forcing_frequency_hz: FIBER_SIGNAL_HZ,
                band_edge_hz: FIBER_SIGNAL_HZ,
                ..Default::default()
            },
        )
        .with_instantaneous_materials(
            prepared
                .canonical_temporal_operator
                .as_ref()
                .expect("the pump prepares a temporal operator")
                .initial_runtime(),
        )
        .with_coefficient_patterns(patterns);
        let result = loop {
            if let Some(result) = job.advance(1 << 16) {
                break result.unwrap();
            }
        };
        let (mut vacuum, mut held) = (0, 0);
        for (triangle, target) in prepared.mesh.triangles.iter().zip(&result.element_targets) {
            let centroid = triangle
                .vertices
                .iter()
                .fold(Point2::default(), |sum, vertex| {
                    sum + prepared.mesh.vertices[*vertex].point * (1.0 / 3.0)
                });
            if centroid.y.abs() < FIBER_H {
                assert!(
                    *target <= floor * (1.0 + 1.0e-9),
                    "a fiber element was left at {target:.4}, above the pattern's {floor:.4}"
                );
            } else {
                vacuum += 1;
                held += usize::from(*target < 0.035);
            }
        }
        assert!(
            held * 5 < vacuum,
            "{held} of {vacuum} vacuum elements are held near the fiber's floor"
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
    /// 0.3 loses more than twice what radius 0.7 loses (46% against 13%).
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

    /// What a live transfer readout of `response` from `reference`, records
    /// sampled every `dt`, has averaged by their end: segments `segment`
    /// seconds long ending every `every` seconds from the first whole one.
    fn welch(
        reference: &[f64],
        response: &[f64],
        dt: f64,
        segment: f64,
        every: f64,
    ) -> TransferSpectrum {
        let count = (segment / dt).floor() as usize + 1;
        let stride = ((every / dt).round() as usize).max(1);
        let mut average = TransferAverage::default();
        let mut end = count;
        while end <= response.len() {
            average
                .add(
                    &reference[end - count..end],
                    &response[end - count..end],
                    dt,
                )
                .unwrap();
            end += stride;
        }
        average.transfer().unwrap()
    }

    /// `transfer`'s gain at the bin nearest `hz`.
    fn gain_at(transfer: &TransferSpectrum, hz: f64) -> Option<f64> {
        transfer.magnitude((hz / transfer.frequency_step_hz).round() as usize)
    }

    /// The photonic-crystal claims, at edge 0.08, from one pulse's whole
    /// records behind the crystal and in the reference arm, unwindowed, 30 s
    /// from rest. With both arms empty they read the same to within 1%
    /// (0.25%). Across 1.5 to 2.1 Hz, inside the gap, the crystal passes
    /// under a tenth of the field (0.068 at most, 0.024 at 1.85 Hz: 0.06% of
    /// the power, as the continuous wave measured 0.08%); at the band
    /// structure's edges, 1.375 and 2.225 Hz, under a third (0.22 and 0.23);
    /// below the gap, at 1 Hz, over 0.95 (0.999), and above it, at 2.5 Hz,
    /// over 0.7 (0.825). The readout, a pulse a segment for 80 s, reads the
    /// gap as the whole pulse does, every frequency from 1.45 to 2.1 Hz
    /// within 12% (6.7%), and 1 and 2.5 Hz within 10% (3.0% and 5.9% low). Its
    /// 20 s segments cannot resolve the crystal's narrowest transmission
    /// peaks, 0.1 Hz wide at 1.25 and 2.3 Hz, which it reads 14% and 25% low.
    #[test]
    fn a_rod_crystal_turns_back_its_gap_and_passes_either_side() {
        let points = [ARM_BEHIND, ARM_REFERENCE];
        let whole = |rods: bool| {
            let (times, series) = records(&photonic_crystal_with(0.0, rods), 0.08, 30.0, &points);
            transfer_spectrum(&series[1], &series[0], times[1] - times[0]).unwrap()
        };
        let band = |transfer: &TransferSpectrum, low: f64, high: f64| {
            (0..transfer.ratios.len())
                .filter(|index| (low..=high).contains(&transfer.frequency_hz(*index)))
                .map(|index| {
                    (
                        transfer.frequency_hz(index),
                        transfer.magnitude(index).unwrap(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let empty = band(&whole(false), 0.8, 3.0)
            .into_iter()
            .fold(0.0_f64, |worst, (_, gain)| worst.max((gain - 1.0).abs()));
        assert!(empty < 0.01, "the empty arms differ by {empty:.3e}");
        let truth = whole(true);
        let gap = band(&truth, 1.5, 2.1)
            .into_iter()
            .fold(0.0_f64, |worst, (_, gain)| worst.max(gain));
        assert!(gap < 0.1, "the gap passes {gap:.4}");
        let gain = |hz: f64| gain_at(&truth, hz).unwrap();
        for (hz, bound) in [(1.375, 1.0 / 3.0), (2.225, 1.0 / 3.0)] {
            assert!(gain(hz) < bound, "{hz} Hz passes {:.3}", gain(hz));
        }
        assert!(gain(1.0) > 0.95, "1 Hz passes {:.3}", gain(1.0));
        assert!(gain(2.5) > 0.7, "2.5 Hz passes {:.3}", gain(2.5));

        let document = photonic_crystal();
        let readout = document.readouts.probe(ProbeId(1));
        assert_eq!(
            readout.transfer_from,
            Some(TransferReference::Probe(ProbeId(2)))
        );
        let (times, series) = records(&document, 0.08, 4.0 * CRYSTAL_SEGMENT, &points);
        let dt = times[1] - times[0];
        let average = welch(&series[1], &series[0], dt, readout.transfer_segment, 0.25);
        let off = |hz: f64| gain_at(&average, hz).unwrap() / gain_at(&truth, hz).unwrap() - 1.0;
        let inside = band(&average, 1.45, 2.1)
            .into_iter()
            .fold(0.0_f64, |worst, (hz, _)| worst.max(off(hz).abs()));
        assert!(inside < 0.12, "the readout's gap is {inside:.3} off");
        for hz in [1.0, 2.5] {
            assert!(
                off(hz).abs() < 0.1,
                "the readout at {hz} Hz is {:.3} off",
                off(hz)
            );
        }
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

    /// The Talbot claims, at edge 0.08, 8 s from rest at 4 Hz, with `|U|` at
    /// the slit centres (±0.2, ±0.6) and the bar centres (0, ±0.4). Halfway to
    /// the Talbot distance the bars are more than 2.5× the slits (3.9×); at
    /// the Talbot distance the slits are more than 2.5× the bars (3.5×), and
    /// that image is more than 1.5× as sharp as at the paraxial `2a²/λ`
    /// (1.7×). At edge 0.05: 4.0×, 3.5×, 1.7×.
    #[test]
    fn a_grating_reimages_itself_at_the_talbot_distance() {
        let scene = Harmonic::run(&talbot_with(true), 0.08, 8.0, TALBOT_HZ, 4.0);
        let at = |x: f64, y: f64| {
            let (a, b) = scene.interpolated(Point2::new(x, y));
            a.hypot(b)
        };
        let contrast = |distance: f64| {
            let x = TALBOT_SCREEN + distance;
            let slits = [0.2, -0.2, 0.6, -0.6]
                .iter()
                .map(|y| at(x, *y))
                .sum::<f64>()
                / 4.0;
            let bars = [0.0, 0.4, -0.4].iter().map(|y| at(x, *y)).sum::<f64>() / 3.0;
            slits / bars
        };
        let talbot = talbot_distance();
        let half = 1.0 / contrast(0.5 * talbot);
        assert!(half > 2.5, "halfway the bars are {half:.2}× the slits");
        let full = contrast(talbot);
        assert!(
            full > 2.5,
            "at the Talbot distance the slits are {full:.2}× the bars"
        );
        let paraxial = contrast(2.0 * TALBOT_PERIOD * TALBOT_PERIOD * TALBOT_HZ);
        assert!(
            full > 1.5 * paraxial,
            "the image is {full:.2}× at the Talbot distance, {paraxial:.2}× at the paraxial one"
        );
    }

    /// The drum claims, at edge 0.08, 45 s from rest, the lobes 95% of their
    /// 60 s size. Driven at `j₂₁/(2πa)` the RMS along the two nodal diameters,
    /// r from 0.05 to 0.55, is under a tenth of the RMS along the four lobes'
    /// radii (5.4%; the same at edge 0.05), and `|U|` round the circle through
    /// the lobes has four maxima. Driven 0.01 Hz either side the lobes are
    /// weaker (0.086 and 0.114 against 0.128): the mesh's resonance is within
    /// 0.1% of the Bessel zero.
    #[test]
    fn a_clamped_drum_stands_in_its_two_one_mode() {
        let lobes_and_nodes = |frequency: f64| {
            let scene = Harmonic::run(&drum_with(frequency), 0.08, 45.0, frequency, 4.0);
            let power = |r: f64, degrees: f64| {
                let angle = degrees.to_radians();
                let (a, b) = scene.interpolated(Point2::new(angle.cos(), angle.sin()) * r);
                a * a + b * b
            };
            let rms = |angles: [f64; 4]| {
                let radii = (0..=10).map(|index| 0.05 + 0.05 * index as f64);
                let values = radii
                    .flat_map(|r| angles.map(|degrees| power(r, degrees)))
                    .collect::<Vec<_>>();
                (values.iter().sum::<f64>() / values.len() as f64).sqrt()
            };
            let ring = (0..72)
                .map(|index| power(0.36, 5.0 * index as f64).sqrt())
                .collect::<Vec<_>>();
            (
                rms([0.0, 90.0, 180.0, 270.0]),
                rms([45.0, 135.0, 225.0, 315.0]),
                ring,
            )
        };
        let frequency = drum_frequency();
        let (lobes, nodes, ring) = lobes_and_nodes(frequency);
        assert!(
            nodes < 0.1 * lobes,
            "the nodal diameters keep {:.3}",
            nodes / lobes
        );
        let peak = ring.iter().cloned().fold(0.0, f64::max);
        let maxima = (0..ring.len())
            .filter(|&index| {
                let (before, here, after) = (
                    ring[(index + ring.len() - 1) % ring.len()],
                    ring[index],
                    ring[(index + 1) % ring.len()],
                );
                here > before && here >= after && here > 0.5 * peak
            })
            .count();
        assert_eq!(maxima, 4);
        for offset in [-0.01, 0.01] {
            let (off, _, _) = lobes_and_nodes(frequency + offset);
            assert!(
                off < lobes,
                "{offset:+} Hz off the lobes are {off:.4} against {lobes:.4}"
            );
        }
    }

    /// The disordered-crystal claims, at edge 0.08, 12 s from rest at 2.5 Hz,
    /// against the empty channel: the power through a cut across the channel
    /// behind the slab, and the plane wave's share of it, from the phasor
    /// averaged across the channel. The ordered crystal passes 80%; the same
    /// rods at random pass under a quarter of that (0.118, 15% of it; 13.5%
    /// at edge 0.05, 12.3% at 24 s), and of what they pass under a quarter is
    /// still the plane wave (6%; 7% and 14%).
    #[test]
    fn a_disordered_crystal_scatters_away_what_its_order_passes() {
        let through = |sites: &[Point2]| {
            let scene = Harmonic::run(&rods_with(sites), 0.08, 12.0, DISORDER_HZ, 4.0);
            let (re, im) = (0..80)
                .map(|index| {
                    scene.interpolated(Point2::new(0.75, -1.0 + 2.0 * (index as f64 + 0.5) / 80.0))
                })
                .fold((0.0, 0.0), |sum, (a, b)| (sum.0 + a, sum.1 + b));
            let power = scene.power_through(
                Point2::new(0.75, -0.995),
                Point2::new(0.75, 0.995),
                Point2::new(1.0, 0.0),
                400,
            );
            (power, (re * re + im * im) / 6400.0)
        };
        let lattice = (0..50)
            .map(|index| {
                Point2::new(
                    ((index / 10) as f64 - 2.0) * CRYSTAL_PITCH,
                    -1.0 + ((index % 10) as f64 + 0.5) * CRYSTAL_PITCH,
                )
            })
            .collect::<Vec<_>>();
        let (empty, empty_plane) = through(&[]);
        let (ordered, _) = through(&lattice);
        let (random, random_plane) = through(&disordered_sites());
        assert!(
            random < 0.25 * ordered,
            "at random the rods pass {:.3} of the empty channel, in order {:.3}",
            random / empty,
            ordered / empty
        );
        // The plane wave carries power in proportion to its squared
        // amplitude; the empty channel's is the plane wave's own.
        let plane_share = (random_plane / empty_plane) / (random / empty);
        assert!(
            plane_share < 0.25,
            "the plane wave is {plane_share:.3} of what passes"
        );
    }

    /// The first 0.6 of the disordered slab, its 35 rods west of `x = 0.1`,
    /// meshes at edges 0.16, 0.08 and 0.05. Bridging the channel's face to its holes gives
    /// one rod vertex two bridges there, and the second must leave through
    /// the copy of the vertex whose wedge it runs into; spliced at the other
    /// copy, the polygon crossed itself and ear clipping stalled.
    #[test]
    fn the_disordered_slab_meshes_where_a_rod_takes_two_bridges() {
        let west = disordered_sites()
            .into_iter()
            .filter(|site| site.x < 0.1)
            .collect::<Vec<_>>();
        assert_eq!(west.len(), 35);
        for edge in [0.16, 0.08, 0.05] {
            prepare(&rods_with(&west), edge);
        }
    }

    /// The skin-depth claims, at edge 0.08, 8 s from rest at 3 Hz, from the
    /// phasor along the slab's front, 0.02 to 0.18 in, before its back face's
    /// reflection counts: the slope of `ln|U|` is `Im k` of
    /// `k = (ω/c)√(1 − iγ/ω)` within 3% (14.81 against 14.82), and that of the
    /// phase `Re k` within 3% (23.88 against 23.98); edges 0.05 and 0.04 agree
    /// within 0.4%. The good conductor's `√(ωγ/2) = 18.85` is more than 15%
    /// off the measured decay (21%).
    #[test]
    fn a_lossy_slab_damps_the_wave_at_its_exact_skin_depth() {
        let scene = Harmonic::run(&skin_depth(), 0.08, 8.0, SKIN_HZ, 4.0);
        let omega = std::f64::consts::TAU * SKIN_HZ;
        let ratio = skin_loss() / omega;
        let (magnitude, angle) = ((1.0 + ratio * ratio).sqrt().sqrt(), -ratio.atan() / 2.0);
        let (real, imaginary) = (
            omega * magnitude * angle.cos(),
            -omega * magnitude * angle.sin(),
        );
        let mut samples = (0..=40)
            .map(|index| {
                let x = SKIN_FRONT + 0.02 + 0.16 * index as f64 / 40.0;
                let (u, v) = scene.interpolated(Point2::new(x, 0.3));
                (x, u.hypot(v).ln(), v.atan2(u))
            })
            .collect::<Vec<_>>();
        // The phase, unwrapped.
        for index in 1..samples.len() {
            let mut step = samples[index].2 - samples[index - 1].2;
            step -= (step / std::f64::consts::TAU).round() * std::f64::consts::TAU;
            samples[index].2 = samples[index - 1].2 + step;
        }
        let slope = |value: fn(&(f64, f64, f64)) -> f64| {
            let count = samples.len() as f64;
            let mean_x = samples.iter().map(|sample| sample.0).sum::<f64>() / count;
            let mean = samples.iter().map(value).sum::<f64>() / count;
            samples
                .iter()
                .map(|sample| (sample.0 - mean_x) * (value(sample) - mean))
                .sum::<f64>()
                / samples
                    .iter()
                    .map(|sample| (sample.0 - mean_x).powi(2))
                    .sum::<f64>()
        };
        let decay = -slope(|sample| sample.1);
        let wavenumber = slope(|sample| sample.2).abs();
        assert!(
            (decay / imaginary - 1.0).abs() < 0.03,
            "the field decays at {decay:.2} against {imaginary:.2}"
        );
        assert!(
            (wavenumber / real - 1.0).abs() < 0.03,
            "its phase runs at {wavenumber:.2} against {real:.2}"
        );
        let conductor = (omega * skin_loss() / 2.0).sqrt();
        assert!(
            (decay / conductor - 1.0).abs() > 0.15,
            "the good conductor's {conductor:.2} is within 15% of {decay:.2}"
        );
    }

    /// The plasma skin-depth claims, at edge 0.08, 8 s from rest at 3 Hz,
    /// from the phasor along the slab's front, 0.02 to 0.18 in, and in front
    /// of it from x = −0.8 to −0.4. The slope of `ln|U|` is
    /// `κ = √(ω_p² − ω²)/c` within 3% (16.63 against 16.62; 16.64 at edge
    /// 0.05), and `c/ω_p` is more than 15% off it (51%). The phase turns under
    /// 0.1 rad across that stretch (0.018), where the lossy slab's turns 3.8.
    /// In front the standing wave's least `|U|` is under a tenth of its
    /// greatest (0.014), where the lossy slab's is 0.49, its
    /// `(1 − |r|)/(1 + |r|)`.
    #[test]
    fn a_plasma_below_its_cutoff_turns_the_wave_back_at_its_skin_depth() {
        let scene = Harmonic::run(&plasma_skin_depth(), 0.08, 8.0, SKIN_HZ, 4.0);
        let at = |x: f64| scene.interpolated(Point2::new(x, 0.3));
        let inside = (0..=40)
            .map(|index| SKIN_FRONT + 0.02 + 0.16 * index as f64 / 40.0)
            .collect::<Vec<_>>();
        let logs = inside.iter().map(|x| {
            let (u, v) = at(*x);
            u.hypot(v).ln()
        });
        let count = inside.len() as f64;
        let mean_x = inside.iter().sum::<f64>() / count;
        let logs = logs.collect::<Vec<_>>();
        let mean_log = logs.iter().sum::<f64>() / count;
        let decay = -inside
            .iter()
            .zip(&logs)
            .map(|(x, log)| (x - mean_x) * (log - mean_log))
            .sum::<f64>()
            / inside.iter().map(|x| (x - mean_x).powi(2)).sum::<f64>();
        let omega = std::f64::consts::TAU * SKIN_HZ;
        let cutoff = std::f64::consts::TAU * PLASMA_SKIN_CUTOFF_HZ;
        let kappa = (cutoff * cutoff - omega * omega).sqrt();
        assert!(
            (decay / kappa - 1.0).abs() < 0.03,
            "the field decays at {decay:.2} against {kappa:.2}"
        );
        assert!(
            (decay / cutoff - 1.0).abs() > 0.15,
            "c/ω_p's {cutoff:.2} is within 15% of {decay:.2}"
        );
        let mut phases = inside
            .iter()
            .map(|x| {
                let (u, v) = at(*x);
                v.atan2(u)
            })
            .collect::<Vec<_>>();
        for index in 1..phases.len() {
            let mut step = phases[index] - phases[index - 1];
            step -= (step / std::f64::consts::TAU).round() * std::f64::consts::TAU;
            phases[index] = phases[index - 1] + step;
        }
        let turn = phases.iter().cloned().fold(f64::MIN, f64::max)
            - phases.iter().cloned().fold(f64::MAX, f64::min);
        assert!(turn < 0.1, "the phase turns {turn:.3} rad inside");
        let front = (0..=90)
            .map(|index| {
                let (u, v) = at(-0.8 + 0.4 * index as f64 / 90.0);
                u.hypot(v)
            })
            .collect::<Vec<_>>();
        let (least, greatest) = (
            front.iter().cloned().fold(f64::MAX, f64::min),
            front.iter().cloned().fold(0.0, f64::max),
        );
        assert!(
            least < 0.1 * greatest,
            "the standing wave's nodes keep {:.3}",
            least / greatest
        );
    }

    /// The step times, and the primary field at the node nearest each of
    /// `points` after every step, from one run from rest.
    fn records(
        document: &TopologyDocument,
        edge: f64,
        seconds: f64,
        points: &[Point2],
    ) -> (Vec<f64>, Vec<Vec<f64>>) {
        let prepared = prepare(document, edge);
        let forcing = prepared.canonical_forcing.clone();
        let dt = prepared.recommended_time_step();
        let steps = (seconds / dt).ceil() as usize;
        let mut times = Vec::with_capacity(steps);
        let mut series = vec![Vec::with_capacity(steps); points.len()];
        let mut nodes = Vec::new();
        let mut record = |time: f64, field: &[f64], positions: &[Point2]| {
            if nodes.is_empty() {
                nodes = points
                    .iter()
                    .map(|point| {
                        positions
                            .iter()
                            .enumerate()
                            .min_by(|a, b| {
                                (*a.1 - *point).norm().total_cmp(&(*b.1 - *point).norm())
                            })
                            .unwrap()
                            .0
                    })
                    .collect();
            }
            times.push(time);
            for (series, node) in series.iter_mut().zip(&nodes) {
                series.push(field[*node]);
            }
        };
        match prepared.canonical_temporal_operator.clone() {
            Some(operator) => {
                let mut state = CanonicalTemporalWaveState::zero(&operator, dt)
                    .unwrap()
                    .pinned(&operator, &forcing)
                    .unwrap();
                for _ in 0..steps {
                    state.step_with_forcing(&operator, &forcing).unwrap();
                    let field = operator
                        .primary_field_at(state.primary_flux(), state.time(), state.runtime())
                        .unwrap();
                    record(state.time(), &field, operator.base().node_points());
                }
            }
            None => {
                let operator = prepared.canonical_operator.clone();
                let mut state = CanonicalWaveState::zero(&operator, dt).unwrap();
                for _ in 0..steps {
                    state.step_with_forcing(&operator, &forcing).unwrap();
                    let field = state.primary_field(&operator).unwrap();
                    record(state.time(), &field, operator.node_points());
                }
            }
        }
        (times, series)
    }

    /// A sinc pulse, flat from 1 to 4 Hz, from a launcher down an empty
    /// channel, with the launcher's own signal.
    fn launched_pulse() -> (TopologyDocument, TimeSignal) {
        let mut builder = Builder::new();
        builder.scene.outer_boundaries = channel();
        builder.launcher(-0.6, 2.5, 1.0);
        let signal = TimeSignal::pulsed(
            [0.0, 1.0, 2.5, 0.0],
            PulseEnvelope::Sinc {
                bandwidth_hz: 1.5,
                lobes: 3,
            },
            0.1,
            0.0,
        );
        builder.scene.volume_sources[0].signal = signal;
        (builder.document(), signal)
    }

    /// Down an empty channel a pulse reaches one probe from another whole:
    /// the transfer between them is flat at one across the pulse's band. From
    /// the launcher's own signal, the field rate it drives, it is flat too:
    /// a strip `w` wide driving the rate `d(t)` sends `(1/2c) ∫ d(t − |x −
    /// x'|/c) dx'` each way, so the transfer is `(w/2c) sinc(πfw/c)`.
    #[test]
    fn a_pulse_down_an_empty_channel_transfers_flat_between_probes_and_from_its_launcher() {
        let (document, signal) = launched_pulse();
        let (times, series) = records(
            &document,
            0.05,
            3.6,
            &[Point2::new(-0.3, 0.0), Point2::new(0.5, 0.0)],
        );
        let interval = times[1] - times[0];
        let between = transfer_spectrum(&series[0], &series[1], interval).unwrap();
        let drive = CanonicalRateDrive::authored(signal, SOURCE_ANCHOR_TIME).unwrap();
        let imposed = times
            .iter()
            .map(|time| drive.value(*time).unwrap())
            .collect::<Vec<_>>();
        let launched = transfer_spectrum(&imposed, &series[1], interval).unwrap();
        let width = 0.06;
        let (mut between_worst, mut launched_worst) = (0.0_f64, 0.0_f64);
        for index in 0..between.ratios.len() {
            let frequency = between.frequency_hz(index);
            if !(1.2..=3.8).contains(&frequency) {
                continue;
            }
            let gain = between.magnitude(index).expect("the band is held");
            between_worst = between_worst.max((gain - 1.0).abs());
            let x = std::f64::consts::PI * frequency * width;
            let expected = 0.5 * width * x.sin() / x;
            let gain = launched.magnitude(index).expect("the band is held");
            launched_worst = launched_worst.max((gain / expected - 1.0).abs());
        }
        // 3.4e-3 and 3.7e-3 at edge 0.05 on the first run.
        assert!(
            between_worst < 1.0e-2,
            "between the probes {between_worst:.3e}"
        );
        assert!(
            launched_worst < 1.0e-2,
            "from the launcher {launched_worst:.3e}"
        );
        let at = |frequency: f64| (frequency / between.frequency_step_hz).round() as usize;
        for frequency in [0.0, 8.0] {
            assert!(between.ratios[at(frequency)].is_none());
            assert!(launched.ratios[at(frequency)].is_none());
        }
    }

    /// The first four zeros of `J₀`, where the round modes of a clamped drum
    /// stand: `j₀ₙ/(2πa)`.
    const BESSEL_J0: [f64; 4] = [2.404_826, 5.520_078, 8.653_728, 11.791_534];

    /// The struck drum's claims, at edge 0.08, 20 s from one knock at the
    /// centre. The probe beside it hears its four strongest lines at the
    /// round modes' `j₀ₙ/(2πa)`, within 0.5% (0.17%, 0.05%, 0.05% and 0.05%;
    /// the same at edge 0.05), each over a third of the strongest (0.53); its
    /// readout, over its own 20 s, finds each within a quarter of its window's
    /// main lobe. At 2, 3 and 4 times the fundamental, where a string's
    /// overtones would stand, the readout shows under 5% of the fundamental's
    /// line (1.9%, 0.26% and 0.70%).
    #[test]
    fn a_struck_drum_rings_at_its_round_modes_and_not_at_a_strings_overtones() {
        let document = struck_drum();
        let (times, series) = records(&document, 0.08, DRUM_SPAN, &[DRUM_LISTENER]);
        let dt = times[1] - times[0];
        let series = &series[0];
        let lines = Lines::read(&document, ProbeId(1), series, dt, 3.2);
        let modes = BESSEL_J0.map(|zero| zero / (std::f64::consts::TAU * DRUM_RADIUS));
        let heights = modes.map(|expected| {
            let found = tone(
                series,
                dt,
                0.95 * expected,
                1.05 * expected,
                DRUM_SPAN - 1.0,
            );
            assert!(
                (found / expected - 1.0).abs() < 5e-3,
                "{found:.4} Hz for {expected:.4}"
            );
            let (shown, _) = lines.strongest(0.9 * expected, 1.1 * expected);
            assert!(
                (shown - expected).abs() < 1.0 / lines.span,
                "the readout's line at {shown:.4} Hz for {expected:.4}"
            );
            let (re, im) = phasor(series, dt, found, DRUM_SPAN - 1.0);
            re.hypot(im)
        });
        let strongest = heights.iter().copied().fold(0.0, f64::max);
        assert!(
            heights.iter().all(|height| *height > strongest / 3.0),
            "{heights:.4?}"
        );
        for multiple in [2.0, 3.0, 4.0] {
            let overtone = lines.at(multiple * modes[0]);
            assert!(
                overtone < 0.05 * lines.at(modes[0]),
                "{multiple} times the fundamental: {overtone:.3e}"
            );
        }
    }

    /// The field a plane wave at `hz` keeps through `layers`, `(index,
    /// thickness)` in order, between vacuum either side at normal incidence:
    /// the product of the layers' characteristic matrices, for a field and
    /// its normal derivative that stay continuous across every face.
    fn layered_transmission(layers: &[(f64, f64)], hz: f64) -> f64 {
        let (re, im) = layered_transfer(layers, hz);
        re.hypot(im)
    }

    /// `layered_transmission` with its phase.
    fn layered_transfer(layers: &[(f64, f64)], hz: f64) -> (f64, f64) {
        type Complex = (f64, f64);
        let mul = |a: Complex, b: Complex| (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0);
        let add = |a: Complex, b: Complex| (a.0 + b.0, a.1 + b.1);
        let mut total = [[(1.0, 0.0), (0.0, 0.0)], [(0.0, 0.0), (1.0, 0.0)]];
        for (index, thickness) in layers {
            let delta = std::f64::consts::TAU * hz * index * thickness;
            let (sin, cos) = delta.sin_cos();
            let layer = [
                [(cos, 0.0), (0.0, sin / index)],
                [(0.0, index * sin), (cos, 0.0)],
            ];
            let mut next = [[(0.0, 0.0); 2]; 2];
            for row in 0..2 {
                for column in 0..2 {
                    next[row][column] = add(
                        mul(total[row][0], layer[0][column]),
                        mul(total[row][1], layer[1][column]),
                    );
                }
            }
            total = next;
        }
        let sum = add(add(total[0][0], total[0][1]), add(total[1][0], total[1][1]));
        let size = sum.0 * sum.0 + sum.1 * sum.1;
        (2.0 * sum.0 / size, -2.0 * sum.1 / size)
    }

    /// The etalon's claims, at edge 0.04, where the slab's shortest
    /// wavelength, 0.095 at 3.5 Hz, has over two elements; at 0.08 the 3 Hz
    /// fringe reads 0.55. One pulse's whole records, 20 s, follow the slab's
    /// `|t(f)|` within 3% from 0.5 to 3.5 Hz (2.5%): one at 1, 2 and 3 Hz,
    /// 0.6 at 1.5 and 2.5. The readout, a pulse a segment for two of them,
    /// reads the whole pulse within 5% from 0.6 to 3.4 Hz (2.0%).
    #[test]
    fn an_etalon_passes_every_whole_hertz_and_six_tenths_between() {
        let layers = [(ETALON_PERMITTIVITY.sqrt(), ETALON_THICKNESS)];
        let points = [ARM_BEHIND, ARM_REFERENCE];
        let (times, series) = records(&etalon_with(ETALON_PERMITTIVITY, 0.0), 0.04, 20.0, &points);
        let truth = transfer_spectrum(&series[1], &series[0], times[1] - times[0]).unwrap();
        let worst =
            |transfer: &TransferSpectrum, low: f64, high: f64, against: &dyn Fn(f64) -> f64| {
                (0..transfer.ratios.len())
                    .filter(|index| (low..=high).contains(&transfer.frequency_hz(*index)))
                    .map(|index| {
                        let hz = transfer.frequency_hz(index);
                        (transfer.magnitude(index).unwrap() / against(hz) - 1.0).abs()
                    })
                    .fold(0.0_f64, f64::max)
            };
        let slab = worst(&truth, 0.5, 3.5, &|hz| layered_transmission(&layers, hz));
        assert!(slab < 0.03, "the whole pulse is {slab:.4} off the slab's");

        let document = etalon();
        let readout = document.readouts.probe(ProbeId(1));
        let (times, series) = records(&document, 0.04, 2.0 * ETALON_SEGMENT, &points);
        let dt = times[1] - times[0];
        let average = welch(&series[1], &series[0], dt, readout.transfer_segment, 0.25);
        let read = worst(&average, 0.6, 3.4, &|hz| gain_at(&truth, hz).unwrap());
        assert!(read < 0.05, "the readout is {read:.4} off the whole pulse");
    }

    /// The cavity filter's claims, at edge 0.04. One pulse's whole records,
    /// 20 s, follow the two plates' `|t(f)|` within 2% from 1 to 3 Hz (0.5%):
    /// one at 2 Hz, over a half-power band 0.22 Hz wide, and 0.29 at 1.5 and
    /// 2.5 Hz. The readout, a pulse a segment for two of them, reads the whole
    /// pulse within 5% from 1.1 to 2.9 Hz (3.5%, at the peak).
    #[test]
    fn a_cavity_between_two_plates_passes_only_its_resonance() {
        let n = CAVITY_PERMITTIVITY.sqrt();
        let layers = [(n, CAVITY_PLATE), (1.0, CAVITY_GAP), (n, CAVITY_PLATE)];
        let points = [ARM_BEHIND, ARM_REFERENCE];
        let (times, series) = records(&cavity_filter_with(0.0), 0.04, 20.0, &points);
        let truth = transfer_spectrum(&series[1], &series[0], times[1] - times[0]).unwrap();
        let worst =
            |transfer: &TransferSpectrum, low: f64, high: f64, against: &dyn Fn(f64) -> f64| {
                (0..transfer.ratios.len())
                    .filter(|index| (low..=high).contains(&transfer.frequency_hz(*index)))
                    .map(|index| {
                        let hz = transfer.frequency_hz(index);
                        (transfer.magnitude(index).unwrap() / against(hz) - 1.0).abs()
                    })
                    .fold(0.0_f64, f64::max)
            };
        let plates = worst(&truth, 1.0, 3.0, &|hz| layered_transmission(&layers, hz));
        assert!(
            plates < 0.02,
            "the whole pulse is {plates:.4} off the plates'"
        );

        let document = cavity_filter();
        let readout = document.readouts.probe(ProbeId(1));
        let (times, series) = records(&document, 0.04, 2.0 * CAVITY_SEGMENT, &points);
        let dt = times[1] - times[0];
        let average = welch(&series[1], &series[0], dt, readout.transfer_segment, 0.25);
        let read = worst(&average, 1.1, 2.9, &|hz| gain_at(&truth, hz).unwrap());
        assert!(read < 0.05, "the readout is {read:.4} off the whole pulse");
    }

    /// The plasma delay's claims, at edge 0.05, from one pulse's records, 12 s
    /// from rest. The pulse behind the plasma trails its twin's energy by
    /// `L(1/v_g − 1/c)` at the carrier within 10% (0.467 s against 0.444).
    /// Frequency by frequency, the transfer's group delay, `−dφ/dω`, follows
    /// the slab's own within 5% from 2.6 to 3.4 Hz (3.2%), ripple from its
    /// faces' reflections and all: 0.61 s at 2.6 Hz, 0.31 at 3.2. Along 0.9
    /// of the plasma the pulse's 3 Hz crests run at `c/√(1 − (f_c/f)²)`,
    /// 1.34c, within 2% (1.2% fast).
    #[test]
    fn a_pulse_through_a_plasma_lags_its_twin_while_its_crests_run_ahead() {
        let tau = std::f64::consts::TAU;
        let length = PLASMA_DELAY_BACK - PLASMA_DELAY_FRONT;
        let index = |hz: f64| (1.0 - (PLASMA_DELAY_CUTOFF_HZ / hz).powi(2)).sqrt();
        let wrapped = |mut angle: f64| {
            while angle > std::f64::consts::PI {
                angle -= tau;
            }
            while angle < -std::f64::consts::PI {
                angle += tau;
            }
            angle
        };
        // The slab's transfer against the same length of vacuum, and its
        // group delay by a central difference of its phase.
        let slab_phase = |hz: f64| {
            let (a, b) = layered_transfer(&[(index(hz), length)], hz);
            let (c, d) = layered_transfer(&[(1.0, length)], hz);
            (b * c - a * d).atan2(a * c + b * d)
        };
        let slab_delay =
            |hz: f64| -wrapped(slab_phase(hz + 1e-3) - slab_phase(hz - 1e-3)) / (2e-3 * tau);

        let inside = (0..=18)
            .map(|step| Point2::new(-0.5 + 0.05 * step as f64, 0.5))
            .collect::<Vec<_>>();
        let mut points = vec![ARM_BEHIND, ARM_REFERENCE];
        points.extend(&inside);
        let (times, series) = records(&plasma_delay_with(0.0), 0.05, 12.0, &points);
        let dt = times[1] - times[0];

        let centroid = |series: &Vec<f64>| {
            let energy = series.iter().map(|u| u * u).sum::<f64>();
            series
                .iter()
                .zip(&times)
                .map(|(u, t)| t * u * u)
                .sum::<f64>()
                / energy
        };
        let lag = centroid(&series[0]) - centroid(&series[1]);
        let expected = length * (1.0 / index(PLASMA_DELAY_HZ) - 1.0);
        assert!(
            (lag / expected - 1.0).abs() < 0.1,
            "the pulse trails by {lag:.4} s against {expected:.4}"
        );

        let transfer = transfer_spectrum(&series[1], &series[0], dt).unwrap();
        let phase = |bin: usize| {
            let [re, im] = transfer.ratios[bin].unwrap();
            im.atan2(re)
        };
        for hz in [2.6, 2.8, 3.0, 3.2, 3.4] {
            let bin = (hz / transfer.frequency_step_hz).round() as usize;
            let delay = -wrapped(phase(bin + 1) - phase(bin - 1))
                / (2.0 * transfer.frequency_step_hz * tau);
            let expected = slab_delay(transfer.frequency_hz(bin));
            assert!(
                (delay / expected - 1.0).abs() < 0.05,
                "at {hz} Hz a group delay of {delay:.4} s against {expected:.4}"
            );
        }

        let mut phases: Vec<f64> = Vec::new();
        for series in &series[2..] {
            let (re, im) = phasor(series, dt, PLASMA_DELAY_HZ, 12.0 - dt);
            let mut value = im.atan2(re);
            if let Some(last) = phases.last() {
                value = last + wrapped(value - last);
            }
            phases.push(value);
        }
        let xs = inside.iter().map(|point| point.x).collect::<Vec<_>>();
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
        let crests = tau * PLASMA_DELAY_HZ / slope.abs();
        let expected = 1.0 / index(PLASMA_DELAY_HZ);
        assert!(
            (crests / expected - 1.0).abs() < 0.02,
            "the crests run at {crests:.4} against {expected:.4}"
        );
    }

    /// The temporal slab's claims, at edge 0.08, 4.5 s from rest, from
    /// `∫u² dt` at the two probes either side of the pulse when the
    /// permittivity drops. Upstream the time-reflected pulse carries
    /// `E_b² n₂/n₁` = 0.5 of what the incident one did within 3% (0.498): as
    /// much field, over half the time. Downstream the pulse running on carries
    /// `(E_f/E_b)²` = 9 times the reflected one within 3% (9.10). Both come at
    /// twice the incident's spectral centroid within 3% (3.03 and 3.08 Hz
    /// against 2 × 1.546). With the permittivity held, under a thousandth
    /// comes back (3e-4).
    #[test]
    fn a_sudden_drop_in_permittivity_splits_a_pulse_and_doubles_its_frequency() {
        let points = [TEMPORAL_UPSTREAM, TEMPORAL_DOWNSTREAM];
        let run = |drop: bool| records(&temporal_slab_with(drop), 0.08, 4.5, &points);
        let (times, series) = run(true);
        let dt = times[1] - times[0];
        let window = |series: &Vec<f64>, from: f64, to: f64| {
            series
                .iter()
                .zip(&times)
                .filter(|(_, time)| (from..to).contains(*time))
                .map(|(value, _)| *value)
                .collect::<Vec<_>>()
        };
        let energy = |values: &[f64]| values.iter().map(|value| value * value * dt).sum::<f64>();
        let centroid = |values: &[f64]| {
            let spectrum = amplitude_spectrum(values, dt).unwrap();
            let (weighted, total) = spectrum.magnitudes.iter().enumerate().fold(
                (0.0, 0.0),
                |(weighted, total), (index, magnitude)| {
                    let power = magnitude * magnitude;
                    (
                        weighted + spectrum.frequency_hz(index) * power,
                        total + power,
                    )
                },
            );
            weighted / total
        };
        let (before, after) = (TEMPORAL_DROP - 0.1, TEMPORAL_DROP + 0.05);
        let incident = window(&series[0], 0.0, before);
        let back = window(&series[0], after, 4.5);
        let forward = window(&series[1], after, 4.5);
        let reflected = energy(&back) / energy(&incident);
        assert!(
            (reflected / 0.5 - 1.0).abs() < 0.03,
            "the reflection carries {reflected:.4}"
        );
        let onward = energy(&forward) / energy(&back);
        assert!(
            (onward / 9.0 - 1.0).abs() < 0.03,
            "the pulse running on carries {onward:.4} times the reflection"
        );
        let doubled = 2.0 * centroid(&incident);
        for (name, pulse) in [("reflected", &back), ("onward", &forward)] {
            let frequency = centroid(pulse);
            assert!(
                (frequency / doubled - 1.0).abs() < 0.03,
                "the {name} pulse at {frequency:.4} Hz against {doubled:.4}"
            );
        }

        let (_, held) = run(false);
        let returned = energy(&window(&held[0], after, 4.5)) / energy(&incident);
        assert!(returned < 1e-3, "held, {returned:.3e} comes back");
    }

    /// The ellipse flash's claims, at edge 0.05, 5 s from one flash, from the
    /// field's largest swing within 0.3 s of each arrival. At the far focus
    /// the echoes peak `2a/c` after the flash's centre within 1% (1.898 s
    /// against 1.9) and over six times the direct pulse there (8.8; 8.9 at
    /// edge 0.04). A quarter beside the focus the echoes arrive spread out,
    /// under a fifth of the focus's peak (0.14).
    #[test]
    fn an_ellipse_gathers_a_flash_from_one_focus_onto_the_other() {
        let focus = ellipse_focus();
        let centre = 0.1 + 4.0 * ELLIPSE_WIDTH;
        let (direct, echoes) = (centre + 2.0 * focus, centre + 2.0 * ELLIPSE_A);
        let points = [Point2::new(focus, 0.0), ELLIPSE_BESIDE];
        let (times, series) = records(&ellipse_flash_with(0.0, ELLIPSE_LOSS), 0.05, 5.0, &points);
        let peak = |series: &Vec<f64>, around: f64| {
            series
                .iter()
                .zip(&times)
                .filter(|(_, time)| (around - 0.3..around + 0.3).contains(*time))
                .fold((0.0_f64, 0.0), |best, (value, time)| {
                    if value.abs() > best.0 {
                        (value.abs(), *time)
                    } else {
                        best
                    }
                })
        };
        let (gathered, when) = peak(&series[0], echoes);
        let travelled = when - centre;
        assert!(
            (travelled / (2.0 * ELLIPSE_A) - 1.0).abs() < 0.01,
            "the echoes peak {travelled:.4} s after the flash"
        );
        let (straight, _) = peak(&series[0], direct);
        assert!(
            gathered > 6.0 * straight,
            "the echoes peak at {:.3} times the direct pulse",
            gathered / straight
        );
        let (beside, _) = peak(&series[1], echoes);
        assert!(
            beside < 0.2 * gathered,
            "beside the focus they reach {:.3} of it",
            beside / gathered
        );
    }

    /// The echo comb's claims, at edge 0.05, where the comb's gaps, the
    /// phase of a round trip a second long, hold to the mesh's dispersion (at
    /// 0.08 they read 0.12 and 0.26). One pulse's whole records, 20 s, follow
    /// `2|cos(πfτ)|`, `τ` = 1 s, within 0.06 from 0.6 to 3.4 Hz (0.048): two
    /// at 1, 2 and 3 Hz, 0.01 and 0.05 at 1.5 and 2.5. The readout, a pulse a
    /// segment for two of them, reads the echo low by the segment's overlap
    /// with itself a second on, `R(1/16)` = 0.975, so its teeth stand at
    /// `1 + R` within 2% (1.977 to 1.982 against 1.975) and its gaps under
    /// 0.1 (0.03 and 0.05).
    #[test]
    fn a_mirror_behind_a_probe_combs_its_transfer() {
        let pi = std::f64::consts::PI;
        let tau = 2.0 * (ECHO_MIRROR - ECHO_PROBE_X);
        let comb = |hz: f64| 2.0 * (pi * hz * tau).cos().abs();
        let points = [
            Point2::new(ECHO_PROBE_X, ARM_BEHIND.y),
            Point2::new(ECHO_PROBE_X, ARM_REFERENCE.y),
        ];
        let (times, series) = records(&echo_comb_with(0.0), 0.05, 20.0, &points);
        let truth = transfer_spectrum(&series[1], &series[0], times[1] - times[0]).unwrap();
        let off = (0..truth.ratios.len())
            .filter(|index| (0.6..=3.4).contains(&truth.frequency_hz(*index)))
            .map(|index| (truth.magnitude(index).unwrap() - comb(truth.frequency_hz(index))).abs())
            .fold(0.0_f64, f64::max);
        assert!(off < 0.06, "the whole pulse is {off:.4} off the comb");

        let document = echo_comb();
        let segment = document.readouts.probe(ProbeId(1)).transfer_segment;
        let (times, series) = records(&document, 0.05, 2.0 * segment, &points);
        let dt = times[1] - times[0];
        let average = welch(&series[1], &series[0], dt, segment, 0.25);
        let u = tau / segment;
        let overlap = ((1.0 - u) * (2.0 + (2.0 * pi * u).cos())
            + 3.0 * (2.0 * pi * u).sin() / (2.0 * pi))
            / 3.0;
        for tooth in [1.0, 2.0, 3.0] {
            let gain = gain_at(&average, tooth).unwrap();
            assert!(
                (gain / (1.0 + overlap) - 1.0).abs() < 0.02,
                "the tooth at {tooth} Hz reads {gain:.4} against {:.4}",
                1.0 + overlap
            );
        }
        for gap in [1.5, 2.5] {
            let gain = gain_at(&average, gap).unwrap();
            assert!(gain < 0.1, "the gap at {gap} Hz reads {gain:.4}");
        }
    }

    /// The pumped drum's claims, at edge 0.08, 20 s from one knock, from the
    /// fundamental's amplitude by a Hann-weighted phasor over two of its
    /// periods, on the first overtone's still circle, where no (0,2) mode
    /// leaks in. Unpumped it rings down at `γ/2`, 0.050 per second (0.0502).
    /// Pumped, over windows inside the gate's second half it grows at `dω/4 −
    /// γ/2` within 10% (0.3506 against 0.3508), and once the pump stops it
    /// rings down at `γ/2` again within 5% (0.0500). The probe beside the
    /// centre reads its line, over its 20 s, at over twice every overtone's
    /// (2.75 times the first), where unpumped it is under the (0,2) mode's
    /// (0.56).
    #[test]
    fn a_pump_at_twice_a_drums_fundamental_grows_it_alone() {
        let fundamental = drum_fundamental();
        let period = 1.0 / fundamental;
        let points = [DRUM_STILL_CIRCLE, DRUM_LISTENER];
        let run = |document: &TopologyDocument| {
            let (times, series) = records(document, 0.08, DRUM_SPAN, &points);
            (times[1] - times[0], series)
        };
        let amplitude = |series: &[f64], dt: f64, end: f64| {
            let count = (2.0 * period / dt) as usize;
            let last = (end / dt) as usize;
            let (mut re, mut im, mut weight) = (0.0, 0.0, 0.0);
            for (index, value) in series[last - count..last].iter().enumerate() {
                let taper = (std::f64::consts::PI * (index as f64 + 0.5) / count as f64)
                    .sin()
                    .powi(2);
                let phase = std::f64::consts::TAU * fundamental * index as f64 * dt;
                re += taper * value * phase.cos();
                im -= taper * value * phase.sin();
                weight += taper;
            }
            2.0 * re.hypot(im) / weight
        };
        // The slope of the amplitude's logarithm over windows ending from
        // `from` to `to`, by least squares.
        let rate = |series: &[f64], dt: f64, from: f64, to: f64| {
            let ends = (0..)
                .map(|step| from + 0.25 * step as f64)
                .take_while(|end| *end <= to)
                .collect::<Vec<_>>();
            let logs = ends
                .iter()
                .map(|end| amplitude(series, dt, *end).ln())
                .collect::<Vec<_>>();
            let n = ends.len() as f64;
            let (mean_end, mean_log) = (ends.iter().sum::<f64>() / n, logs.iter().sum::<f64>() / n);
            ends.iter()
                .zip(&logs)
                .map(|(end, log)| (end - mean_end) * (log - mean_log))
                .sum::<f64>()
                / ends.iter().map(|end| (end - mean_end).powi(2)).sum::<f64>()
        };
        let lines = |series: &[f64], dt: f64| {
            let spectrum = amplitude_spectrum(series, dt).unwrap();
            let at = |hz: f64| {
                (0..spectrum.magnitudes.len())
                    .filter(|index| (spectrum.frequency_hz(*index) - hz).abs() < 0.05)
                    .map(|index| spectrum.magnitudes[index])
                    .fold(0.0, f64::max)
            };
            BESSEL_J0.map(|zero| at(zero / (std::f64::consts::TAU * DRUM_RADIUS)))
        };

        let (dt, control) = run(&struck_drum());
        let loss = -rate(&control[0], dt, 5.0, DRUM_SPAN);
        assert!(
            (loss / (0.5 * DRUM_LOSS) - 1.0).abs() < 0.05,
            "unpumped it rings down at {loss:.4}"
        );
        let control_lines = lines(&control[1], dt);
        assert!(control_lines[0] < control_lines[1], "{control_lines:.4?}");

        let (dt, pumped) = run(&pumped_drum());
        let expected = DRUM_PUMP_DEPTH * std::f64::consts::TAU * fundamental / 4.0 - loss;
        let growth = rate(&pumped[0], dt, 7.0, 9.5);
        assert!(
            (growth / expected - 1.0).abs() < 0.1,
            "pumped it grows at {growth:.4} against {expected:.4}"
        );
        let after = -rate(&pumped[0], dt, 13.5, DRUM_SPAN);
        assert!(
            (after / loss - 1.0).abs() < 0.05,
            "after the pump it rings down at {after:.4}"
        );
        let pumped_lines = lines(&pumped[1], dt);
        for overtone in &pumped_lines[1..] {
            assert!(pumped_lines[0] > 2.0 * overtone, "{pumped_lines:.4?}");
        }
    }

    /// The chopper's claims, at edge 0.08, from the lines over the last 8 s,
    /// four chopping periods, of 12 s from rest behind the shutter. Held
    /// shut, it passes under 1% of the field it passes held open (0.6%).
    /// Chopping, the carrier's first sidebands, half a hertz either side,
    /// carry the gate's first Fourier coefficient of the open field within
    /// 10% (0.316 and 0.301 against 0.317, a square wave's `1/π`), and the
    /// carrier keeps the gate's mean within 15% (0.419 against 0.475): the
    /// slab refills at the wave's speed after each opening, shortening the
    /// bursts. The readout, over its own 8 s, shows both first sidebands at
    /// over half its carrier line.
    #[test]
    fn a_shutter_opening_half_the_time_puts_a_third_of_the_wave_in_each_first_sideband() {
        let window = 4.0 * CHOPPER_PERIOD;
        let run = |shutter: Shutter| {
            let (times, mut series) = records(
                &chopper_with(shutter, CHOPPER_RATE, CHOPPER_FRONT),
                0.08,
                12.0,
                &[CHOPPER_BEHIND],
            );
            (series.pop().unwrap(), times[1] - times[0])
        };
        let line = |series: &[f64], dt: f64, hz: f64| {
            let (re, im) = phasor(series, dt, hz, window);
            re.hypot(im)
        };
        let (open, dt) = run(Shutter::Open);
        let open = line(&open, dt, CHOPPER_HZ);
        let (shut, dt) = run(Shutter::Shut);
        let shut = line(&shut, dt, CHOPPER_HZ) / open;
        assert!(shut < 0.01, "held shut it passes {shut:.4}");

        // The gate's own Fourier coefficients over one period.
        let gate = PulseTrain {
            envelope: PulseEnvelope::FlatTop {
                duration: CHOPPER_OPEN,
                edge: CHOPPER_EDGE,
            },
            start: 0.0,
            repeat: CHOPPER_PERIOD,
        };
        let coefficient = |k: f64| {
            let samples = 20_000;
            let (mut re, mut im) = (0.0, 0.0);
            for index in 0..samples {
                let time = CHOPPER_PERIOD * (index as f64 + 0.5) / samples as f64;
                let open = gate.seconds_from_centre(time).map_or(0.0, |from_centre| {
                    gate.envelope.value_and_rate(from_centre).0
                });
                let phase = std::f64::consts::TAU * k * time / CHOPPER_PERIOD;
                re += open * phase.cos() / samples as f64;
                im -= open * phase.sin() / samples as f64;
            }
            re.hypot(im)
        };
        let document = chopper();
        let (times, series) = records(&document, 0.08, 12.0, &[CHOPPER_BEHIND]);
        let (series, dt) = (&series[0], times[1] - times[0]);
        for side in [-1.0, 1.0] {
            let sideband = line(series, dt, CHOPPER_HZ + side / CHOPPER_PERIOD) / open;
            assert!(
                (sideband / coefficient(1.0) - 1.0).abs() < 0.1,
                "the sideband at {side:+} carries {sideband:.4} against {:.4}",
                coefficient(1.0)
            );
        }
        let carrier = line(series, dt, CHOPPER_HZ) / open;
        assert!(
            (carrier / coefficient(0.0) - 1.0).abs() < 0.15,
            "the carrier keeps {carrier:.4} against {:.4}",
            coefficient(0.0)
        );
        let lines = Lines::read(&document, ProbeId(1), series, dt, CHOPPER_HZ + 0.5);
        for side in [-0.5, 0.5] {
            assert!(lines.at(CHOPPER_HZ + side) > 0.5 * lines.at(CHOPPER_HZ));
        }
    }
}
