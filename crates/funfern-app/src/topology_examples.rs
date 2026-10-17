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
use std::collections::BTreeMap;
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
                "A TM point source and a biconvex glass lens of index 2: the light it gathers \
                 converges to a spot behind it, with the power flow drawn in.",
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
                 turn of its phase; each runs down the line as a voltage pulse, a kink of 2π \
                 in the phase the integrated field shows, and a probe down the line hears them \
                 as a tone at V/2π, the Josephson frequency.",
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
    /// What the scene calls its regions, which the Materials panel lists.
    names: BTreeMap<RegionId, String>,
    /// How many rods `rod` has built.
    rods: usize,
}

impl Builder {
    fn new() -> Self {
        Self {
            scene: TopologyScene::default(),
            next_curve: 1,
            next_span: 1,
            next_region: 2,
            next_vertex: 1,
            names: BTreeMap::new(),
            rods: 0,
        }
    }

    /// Names a region, which must be one the scene holds by the time it is
    /// a document.
    fn name(&mut self, region: RegionId, name: &str) {
        self.names.insert(region, name.to_owned());
    }

    /// A crystal's rod of `material`, named Rod n in the order built.
    fn rod(&mut self, spline: PeriodicCubicSpline, material: MaterialId) -> RegionId {
        let region = self.subdomain(spline, material, MaterialFrame::world());
        self.rods += 1;
        let name = format!("Rod {}", self.rods);
        self.name(region, &name);
        region
    }

    /// Names regions in order, one name each.
    fn name_all(&mut self, regions: &[RegionId], names: &[&str]) {
        assert_eq!(regions.len(), names.len(), "a name for each of {regions:?}");
        for (region, name) in regions.iter().zip(names) {
            self.name(*region, name);
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
        self.divider_as(x, SpanBehavior::Transmitting, false)
    }

    /// A divider at `x` from the floor to the ceiling, or from the ceiling
    /// down with `downward`, its one span `behavior`, welded to the outer
    /// vertices at both ends.
    fn divider_as(
        &mut self,
        x: f64,
        behavior: SpanBehavior,
        downward: bool,
    ) -> (CurveId, CurveSpanId) {
        let domain = self.scene.geometry.domain;
        let bottom = self.outer_vertex(OuterSide::Bottom, (x - domain.min_x) / domain.width());
        let top = self.outer_vertex(OuterSide::Top, (domain.max_x - x) / domain.width());
        let span = CurveSpanId(self.next_span);
        let mut ends = [
            (Point2::new(x, domain.min_y), bottom),
            (Point2::new(x, domain.max_y), top),
        ];
        if downward {
            ends.reverse();
        }
        let curve = self.open_curve(
            OpenCubicSpline::polyline(ends.iter().map(|(point, _)| *point).collect()).unwrap(),
            &[behavior],
        );
        let authored = self
            .scene
            .geometry
            .curves
            .iter_mut()
            .find(|candidate| candidate.id == curve)
            .unwrap();
        authored.nodes[0].vertex = Some(ends[0].1);
        authored.nodes[1].vertex = Some(ends[1].1);
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
        // The strip names the face behind it just before itself.
        self.name(RegionId(region.0 - 1), "Behind the launcher");
        self.name(region, "Launcher");
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
    /// Names the launcher's halves, what is behind them and the lower arm;
    /// returns the upper arm's pieces past the launcher, left to right.
    fn arms(
        &mut self,
        x: f64,
        signal: TimeSignal,
        layers: &[(f64, f64, MaterialId)],
        mirror: Option<f64>,
    ) -> Vec<RegionId> {
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
        let behind = self.face_on(OuterSide::Bottom, behind, background);
        let lower = self.face_on(OuterSide::Bottom, x, background);
        let upper = self.face_on(OuterSide::Top, x, background);
        self.name(BACKGROUND_REGION, "Reference arm");
        self.name(behind, "Behind the reference launcher");
        self.name(lower, "Reference launcher");
        self.name(upper, "Sample launcher");
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
        // The upper arm's pieces past the launcher, left to right, for the
        // scene to name.
        let mut past = vec![];
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
            let piece = self.face_on(OuterSide::Top, middle, material);
            if pair[0] == domain.min_x {
                self.name(piece, "Behind the sample launcher");
            } else {
                past.push(piece);
            }
        }
        past
    }

    fn document(self) -> TopologyDocument {
        let scene = self.scene;
        scene.compile(0).unwrap();
        for (region, name) in &self.names {
            assert!(
                scene.region(*region).is_some(),
                "{name:?} names region {}, which the scene does not hold",
                region.0
            );
        }
        TopologyDocument {
            model: TopologyDocumentModel {
                draft: scene.clone(),
                accepted: scene,
                probes: vec![],
                source: PointSource::default(),
                far_field: FarFieldSettings::default(),
                region_names: self.names,
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

/// Streamlines of the power flow: the route it takes, through a lens, round a
/// corner or across a gap, is what the scene is about.
fn streamlines(presentation: &mut PresentationSettings) {
    power_flow(presentation);
    presentation.vector_overlay_style = VectorOverlayStyle::Streamlines;
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

/// A line probe across a beam: the power crossing it, averaged over the mean
/// window, along its length. Its ends run so that the positive normal, to the
/// left of the run, points away from the source.
fn flux_profile_readout() -> ProbeReadout {
    ProbeReadout::blank().with_line_plot(
        LineProbeQuantity::MeanFlux,
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
        // Run downward, so the power leaving the slits crosses it positive.
        target: TopologyProbeTarget::Segment {
            start: Point2::new(0.85, 0.85),
            end: Point2::new(0.85, -0.85),
            preset: ProbeSamplingPreset::High,
        },
    });
    document
        .readouts
        .set_probe(ProbeId(1), flux_profile_readout());
    document.readouts.far_field = polar_readout();
    // The energy bunching into the bright fringes and skirting the dark.
    streamlines(&mut document.presentation);
    document
}

/// The biconvex lens: two circular surfaces of `LENS_RADIUS` meeting at sharp
/// edges `LENS_HALF_APERTURE` above and below the axis, 0.35 thick. Glass of
/// index 2 in the E_z skin, `ε = 4` and `μ = 1`. The thick-lens formula puts
/// its focal length near 0.33, so a source 0.8 in front images about 0.55
/// behind; with an aperture under three wavelengths the focus forms nearer,
/// at 0.43-0.47 on meshes of edge 0.08 and 0.05. Index 1.67 left a lens this
/// thin focusing too weakly to read inside the domain.
const LENS_RADIUS: f64 = 0.55;
const LENS_HALF_APERTURE: f64 = 0.4;
const LENS_SOURCE: Point2 = Point2 { x: -0.8, y: 0.0 };

fn material_lens() -> TopologyDocument {
    material_lens_with(true)
}

/// The material lens, or with `lens` false the same scene whose lens is
/// vacuum.
fn material_lens_with(lens: bool) -> TopologyDocument {
    let mut builder = Builder::new();
    builder.scene.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    builder.scene.outer_boundaries =
        OuterBoundaryConditions::uniform(OuterBoundaryCondition::FirstOrderOutgoing);
    builder.scene.materials.push(Material {
        id: MaterialId(2),
        name: "Glass".into(),
        mass_density: ScalarField::constant(if lens { 4.0 } else { 1.0 }),
        stiffness: ScalarField::constant(1.0),
        damping: ScalarField::constant(0.0),
        axis_ratio: ScalarField::constant(1.0),
        parameters: vec![],
        color: [61, 116, 139],
        ..Material::default_medium()
    });
    let lens = builder.subdomain(
        biconvex(LENS_RADIUS, LENS_HALF_APERTURE),
        MaterialId(2),
        MaterialFrame {
            attachment: MaterialFrameAttachment::FollowRegion,
            ..MaterialFrame::world()
        },
    );
    builder.name(lens, "Lens");
    let mut document = builder.document();
    document.model.source = source(LENS_SOURCE, 3.5, 16.0, 0.045);
    // The power converging behind the lens.
    streamlines(&mut document.presentation);
    document
}

/// A symmetric biconvex lens about the origin, its axis along `x`: two
/// circular surfaces of `radius` meeting at sharp edges `half_aperture` above
/// and below the axis. Each surface is three cubic Bézier pieces, as `arc`
/// builds one, and every knot is C0.
fn biconvex(radius: f64, half_aperture: f64) -> PeriodicCubicSpline {
    const PIECES: usize = 3;
    let offset = (radius * radius - half_aperture * half_aperture).sqrt();
    let half_angle = half_aperture.atan2(offset);
    let step = 2.0 * half_angle / PIECES as f64;
    let reach = 4.0 / 3.0 * portable_tan(step / 4.0) * radius;
    let mut controls = Vec::with_capacity(6 * PIECES);
    // The right surface, centred behind the axis' origin, from the lower edge
    // up; then the left, from the upper edge down.
    for (center, from) in [
        (Point2::new(-offset, 0.0), -half_angle),
        (Point2::new(offset, 0.0), std::f64::consts::PI - half_angle),
    ] {
        let at = |angle: f64| center + direction(angle) * radius;
        let tangent = |angle: f64| {
            let along = direction(angle);
            Point2::new(-along.y, along.x)
        };
        for piece in 0..PIECES {
            let (a, b) = (from + step * piece as f64, from + step * (piece + 1) as f64);
            controls.extend([
                at(a) + tangent(a) * reach,
                at(b) - tangent(b) * reach,
                at(b),
            ]);
        }
    }
    PeriodicCubicSpline::new_with_multiplicities(
        controls,
        vec![radius * step; 2 * PIECES],
        vec![3; 2 * PIECES],
    )
    .unwrap()
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
    builder.name(region, "GRIN rod");
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
        // Run downward, so the power leaving the rod crosses it positive.
        target: TopologyProbeTarget::Segment {
            start: Point2::new(0.75, 0.9),
            end: Point2::new(0.75, -0.9),
            preset: ProbeSamplingPreset::High,
        },
    });
    // The rod's permittivity, 1 in the vacuum: the wave speed puts the vacuum
    // at the top of the palette and paints the domain over. And the power,
    // leaving the rod straight.
    document.presentation.material_overlay = MaterialOverlay::Property(MaterialProperty::Density);
    document.presentation.material_overlay_opacity = 0.45;
    streamlines(&mut document.presentation);
    document
        .readouts
        .set_probe(ProbeId(1), flux_profile_readout());
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
    let crystal = builder.subdomain(
        PeriodicCubicSpline::rounded(center, 0.48),
        MaterialId(2),
        MaterialFrame {
            origin: center,
            angle_radians: 32.0_f64.to_radians(),
            attachment: MaterialFrameAttachment::FollowRegion,
        },
    );
    builder.name(crystal, "Crystal");
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
    builder.name(region, "Lens");
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
        builder.name(region, &format!("Element {}", index + 1));
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
    let slab = builder.subdomain(
        slab(-0.3, 0.3, 0.55),
        MaterialId(2),
        MaterialFrame {
            attachment: MaterialFrameAttachment::FollowRegion,
            ..MaterialFrame::world()
        },
    );
    builder.name(slab, "Slab");
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
    let slab = builder.subdomain(
        slab(-0.35, 0.35, 0.55),
        MaterialId(2),
        MaterialFrame {
            attachment: MaterialFrameAttachment::FollowRegion,
            ..MaterialFrame::world()
        },
    );
    builder.name(slab, "Slab");
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
    let slab = builder.subdomain(
        slab(-half_width, half_width, 0.55),
        MaterialId(2),
        MaterialFrame {
            attachment: MaterialFrameAttachment::FollowRegion,
            ..MaterialFrame::world()
        },
    );
    builder.name(slab, "Slab");
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
    let slab = builder.subdomain(
        slab(-0.6, 0.6, 0.9),
        MaterialId(2),
        MaterialFrame {
            attachment: MaterialFrameAttachment::FollowRegion,
            ..MaterialFrame::world()
        },
    );
    builder.name(slab, "Slab");
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
        // Run downward, so the beam leaving the slab crosses it positive.
        target: TopologyProbeTarget::Segment {
            start: Point2::new(0.65, 0.9),
            end: Point2::new(0.65, -0.9),
            preset: ProbeSamplingPreset::High,
        },
    });
    document.readouts.set_probe(ProbeId(1), profile_readout());
    document
        .readouts
        .set_probe(ProbeId(2), flux_profile_readout());
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
        builder.name(
            region,
            if side == CurveTraceSide::Right {
                "Grating"
            } else {
                "Before the grating"
            },
        );
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
    builder.name(BACKGROUND_REGION, "Past the grating");
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
        builder.name(
            region,
            if side == CurveTraceSide::Right {
                "Slab"
            } else {
                "Before the slab"
            },
        );
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
    builder.name(BACKGROUND_REGION, "Past the slab");
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
        builder.name(
            region,
            if side == CurveTraceSide::Right {
                "Shutter"
            } else {
                "Before the shutter"
            },
        );
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
    builder.name(BACKGROUND_REGION, "Past the shutter");
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
    // Shown in E_z, the voltage: each fluxon a pulse of one height. The
    // integrated field is the phase, which the biased end winds without
    // bound, so the scale follows that and the kinks shrink into it.
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
    builder.name(region, "Emitter");
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
    builder.name(region, "Vacuum disk");
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
    // The band names the face above it just before itself.
    builder.name(BACKGROUND_REGION, "Below the fiber");
    builder.name(RegionId(region.0 - 1), "Above the fiber");
    builder.name(region, "Fiber");
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
    builder.name(BACKGROUND_REGION, "Outside the bend");
    builder.name(inside, "Inside the bend");
    builder.name(core, "Fiber");
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
    let past = builder.arms(-0.85, sinc_pulse(0.8, 3.0, repeat), &[], None);
    builder.name_all(&past, &["Sample arm"]);
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
                builder.rod(
                    octagonal_rod(centre, CRYSTAL_ROD_FRACTION * CRYSTAL_PITCH),
                    MaterialId(2),
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
                builder.rod(
                    octagonal_rod(
                        Point2::new(column as f64, row as f64) * CRYSTAL_PITCH,
                        CRYSTAL_ROD_FRACTION * CRYSTAL_PITCH,
                    ),
                    MaterialId(2),
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
        let inside = builder.subdomain(
            circle(centre, RING_RADIUS - 0.5 * CORE_WIDTH),
            DEFAULT_MATERIAL,
            MaterialFrame::world(),
        );
        builder.name(inside, "Inside the ring");
    }
    // The band names the face above it just before itself.
    builder.name(BACKGROUND_REGION, "Below the bus");
    builder.name(RegionId(bus.0 - 1), "Above the bus");
    builder.name(bus, "Bus waveguide");
    if let Some(ring) = region {
        builder.name(ring, "Ring");
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
    builder.name(region, "Disk");
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
    builder.name(region, "Fisheye lens");
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
    // The power leaving the disk, which leaves at the image.
    document
        .readouts
        .set_probe(ProbeId(1), flux_profile_readout());
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
/// it is even. A point probe sits on the focus and a line probe across the
/// axis reads half the focal plane. Every wall is outgoing.
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
    // Half the focal plane, from the wall in to beside the focus, so it stays
    // clear of the focus probe: the plate is symmetric about the axis, and
    // this half shows the spot rising from the claim's 0.4 aside. Run
    // downward, the power arriving through the plate crosses it positive.
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(2),
        name: "Focal plane".into(),
        color: [248, 196, 112],
        enabled: true,
        target: TopologyProbeTarget::Segment {
            start: Point2::new(ZONE_FOCUS, 0.9),
            end: Point2::new(ZONE_FOCUS, 0.08),
            preset: ProbeSamplingPreset::Medium,
        },
    });
    document.readouts.set_probe(ProbeId(1), field_readout(2.0));
    document
        .readouts
        .set_probe(ProbeId(2), flux_profile_readout());
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
    builder.name(BACKGROUND_REGION, "Incident side");
    builder.name(region, "Glass");
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
    builder.name(BACKGROUND_REGION, "Below the guide");
    builder.name(fiber, "Glass guide");
    builder.name(above, "Above the guide");
    builder.name(block, "Block");
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
            // Run downward, so the power through the grating crosses it
            // positive.
            target: TopologyProbeTarget::Segment {
                start: Point2::new(x, 0.95),
                end: Point2::new(x, -0.95),
                preset: ProbeSamplingPreset::Medium,
            },
        });
    }
    document
        .readouts
        .set_probe(ProbeId(1), flux_profile_readout());
    document
        .readouts
        .set_probe(ProbeId(2), flux_profile_readout());
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
        builder.rod(
            octagonal_rod(*site, CRYSTAL_ROD_FRACTION * CRYSTAL_PITCH),
            MaterialId(2),
        );
    }
    let mut document = builder.document();
    document.model.source.enabled = false;
    document.model.probes.push(TopologyProbeDefinition {
        id: ProbeId(1),
        name: "Across the channel".into(),
        color: [248, 196, 112],
        enabled: true,
        // Run downward, so the power through the rods crosses it positive.
        target: TopologyProbeTarget::Segment {
            start: Point2::new(0.75, 0.95),
            end: Point2::new(0.75, -0.95),
            preset: ProbeSamplingPreset::Medium,
        },
    });
    // Fifty rods' control polygons and handles would hide the scatterers.
    document.presentation.control_polygons = false;
    document.presentation.handles = false;
    // The power through the cut, along it and in all: the average flux.
    document.readouts.set_probe(
        ProbeId(1),
        flux_profile_readout().with_line_plot(
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
    let past = builder.arms(-0.85, sinc_pulse(0.5, 3.5, repeat), &[], Some(ECHO_MIRROR));
    builder.name_all(&past, &["Sample arm"]);
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
    let past = builder.arms(
        -0.85,
        sinc_pulse(0.5, 3.5, repeat),
        &[(ETALON_FRONT, ETALON_FRONT + ETALON_THICKNESS, MaterialId(2))],
        None,
    );
    builder.name_all(&past, &["Before the etalon", "Etalon", "Past the etalon"]);
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
    let past = builder.arms(
        -0.85,
        sinc_pulse(1.0, 3.0, repeat),
        &[
            (CAVITY_FRONT, CAVITY_FRONT + CAVITY_PLATE, MaterialId(2)),
            (back, back + CAVITY_PLATE, MaterialId(2)),
        ],
        None,
    );
    builder.name_all(
        &past,
        &[
            "Before the filter",
            "First mirror",
            "Cavity",
            "Second mirror",
            "Past the filter",
        ],
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
    slab_channel(
        Material {
            id: MaterialId(2),
            name: "Lossy medium".into(),
            electric_loss: Some(LossChannel {
                base_rate: ScalarField::constant(rate),
                law: DampingLaw::constant(),
            }),
            color: [139, 92, 66],
            ..Material::default_medium()
        },
        "Lossy slab",
    )
}

/// A TM channel lit by a 3 Hz launcher at the left, with a slab of
/// `material`, called `slab_name`, from `x = −0.35` to `−0.05`, a line
/// probe along the channel through it and a point probe behind it.
fn slab_channel(material: Material, slab_name: &str) -> TopologyDocument {
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
        builder.name(
            region,
            if side == CurveTraceSide::Right {
                slab_name
            } else {
                "Before the slab"
            },
        );
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
    builder.name(BACKGROUND_REGION, "Past the slab");
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
    slab_channel(tm_plasma(cutoff), "Plasma slab")
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
    let past = builder.arms(
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
    builder.name_all(
        &past,
        &["Before the plasma", "Plasma slab", "Past the plasma"],
    );
    let mut document = builder.document();
    document.model.source.enabled = false;
    arm_probes(&mut document, "Behind the plasma", field_readout(4.0));
    document
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every region of every gallery scene is named, but the background,
    /// which a scene names only when it is one piece among others, and no
    /// two regions of a scene share a name: the Materials panel lists them
    /// by these names.
    #[test]
    fn every_gallery_region_but_the_background_is_named() {
        for example in catalog() {
            let model = &example.document.model;
            for region in &model.draft.regions {
                assert!(
                    region.id == BACKGROUND_REGION || model.region_names.contains_key(&region.id),
                    "{}: region {} has no name",
                    example.name,
                    region.id.0
                );
            }
            let names = model
                .region_names
                .values()
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(
                names.len(),
                model.region_names.len(),
                "{}: two regions share a name",
                example.name
            );
        }
    }

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

    /// Every gallery probe that plots flux reads the power its claim is about
    /// as positive. The cuts across a beam are all lit from the left, by a
    /// launcher or a point source left of them, so their positive normal - to
    /// the left of the run from start to end - points right; a probe round a
    /// closed rim reads the power leaving what it encloses. The rods' cut ran
    /// upward and read its transmitted power negative.
    #[test]
    fn every_flux_probe_reads_its_claimed_power_positive() {
        use crate::topology_runtime::{TopologyProbeCompilation, TopologyProbeStencil};
        let mut checked = 0;
        for example in catalog() {
            let document = &example.document;
            let plots_flux = |probe: &TopologyProbeDefinition| {
                let readout = document.readouts.probe(probe.id);
                LineProbeRepresentation::ALL.into_iter().any(|view| {
                    readout.line_plot(LineProbeQuantity::Flux, view)
                        || readout.line_plot(LineProbeQuantity::MeanFlux, view)
                })
            };
            let probes = document
                .model
                .probes
                .iter()
                .filter(|probe| plots_flux(probe));
            for probe in probes.clone() {
                let what = format!("{} / {}", example.name, probe.name);
                match &probe.target {
                    TopologyProbeTarget::Segment { start, end, .. } => {
                        let run = *end - *start;
                        let normal = Point2::new(-run.y, run.x) / run.norm();
                        assert!(normal.x > 0.99, "{what}: positive normal {normal:?}");
                        if document.model.source.enabled {
                            assert!(document.model.source.position.x < start.x, "{what}");
                        }
                    }
                    TopologyProbeTarget::Boundary(_) => {
                        let prepared = prepare(document, 0.08);
                        let compiled = prepared
                            .probes
                            .iter()
                            .find(|compiled| compiled.id == probe.id)
                            .unwrap();
                        let TopologyProbeCompilation::Ready(stencil) = &compiled.result else {
                            panic!("{what}: does not compile");
                        };
                        let TopologyProbeStencil::Boundary(samples) = stencil.as_ref() else {
                            panic!("{what}: not a boundary stencil");
                        };
                        let centre = samples
                            .iter()
                            .fold(Point2::new(0.0, 0.0), |sum, sample| sum + sample.point)
                            / samples.len() as f64;
                        for sample in samples {
                            let away = sample.point - centre;
                            assert!(
                                sample.outward_normal.dot(away) > 0.9 * away.norm(),
                                "{what}: inward at {:?}",
                                sample.point
                            );
                        }
                    }
                    _ => panic!("{what}: only line probes plot flux"),
                }
                checked += 1;
            }
        }
        // Screen, beam profile, behind the slab, focal plane, both Talbot
        // planes, the rods' cut and the fisheye's rim.
        assert_eq!(checked, 8, "flux probes");
    }

    /// Prepares a document as the application does, at a given mesh edge.
    fn prepare(
        document: &TopologyDocument,
        edge: f64,
    ) -> Arc<crate::topology_runtime::PreparedTopology> {
        prepare_on(&mut TopologyRuntime::default(), document, 1, edge).0
    }

    /// Prepares `document` as the application does, on `runtime`, where it
    /// follows whatever that runtime holds: the first generation is fresh,
    /// a later one is an edit of the active one and comes with the maps
    /// that carry a state over, which the commit lets go of.
    fn prepare_on(
        runtime: &mut TopologyRuntime,
        document: &TopologyDocument,
        revision: u64,
        edge: f64,
    ) -> (
        Arc<crate::topology_runtime::PreparedTopology>,
        Option<Arc<crate::topology_runtime::PreparedCanonicalTransfer>>,
    ) {
        let editor = TopologyEditor::from_document(document.clone()).unwrap();
        // Capped for the domain's area, as the application meshes it.
        let domain = document.model.accepted.geometry.domain;
        let token = runtime
            .request(
                revision,
                &editor.document,
                editor.compiled_accepted.clone(),
                MeshingOptions {
                    target_edge_length: edge,
                    ..MeshingOptions::default()
                }
                .sized_for_area(domain.width() * domain.height()),
                runtime.active().is_none(),
            )
            .unwrap();
        loop {
            if let Some(result) = runtime.advance(1 << 16) {
                result.unwrap();
                break;
            }
        }
        let maps = runtime
            .ready()
            .and_then(|candidate| candidate.canonical_transfer.clone());
        (runtime.commit_ready(token).unwrap(), maps)
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

    /// The adaptation estimate with the walls stepped `wall_step`'s way, as
    /// the worker estimates it, after `end` seconds from rest: the report and
    /// the per-element indicators.
    fn estimate_after(
        prepared: &crate::topology_runtime::PreparedTopology,
        wall_step: CanonicalWallStep,
        end: f64,
    ) -> (funfern_core::SolutionIndicatorReport, Vec<f64>) {
        let forcing = prepared.canonical_forcing.clone();
        let dt = prepared.recommended_time_step();
        let steps = (end / dt).ceil() as usize;
        let (snapshot, supplement, velocity, displacement, runtime) =
            if let Some(operator) = prepared.canonical_temporal_operator.clone() {
                let mut state = CanonicalTemporalWaveState::zero(&operator, dt)
                    .unwrap()
                    .pinned(&operator, &forcing)
                    .unwrap()
                    .with_wall_step(wall_step);
                let auxiliary_of = |state: &CanonicalTemporalWaveState| {
                    state
                        .thin_gap_jump()
                        .iter()
                        .chain(state.outgoing_pole_currents())
                        .copied()
                        .collect::<Vec<_>>()
                };
                let mut previous = state.clone();
                for _ in 0..steps {
                    previous = state.clone();
                    state.step_with_forcing(&operator, &forcing).unwrap();
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
                .unwrap();
                let velocity = funfern_core::canonical_temporal_primary_rate(
                    &operator,
                    &forcing,
                    &snapshot,
                    state.runtime(),
                )
                .unwrap();
                let displacement = operator
                    .primary_field_at(state.primary_flux(), state.time(), state.runtime())
                    .unwrap();
                (
                    snapshot,
                    supplement,
                    velocity,
                    displacement,
                    Some(state.runtime().clone()),
                )
            } else {
                let operator = &prepared.canonical_operator;
                let mut state = CanonicalWaveState::zero(operator, dt)
                    .unwrap()
                    .with_wall_step(operator, wall_step)
                    .unwrap();
                let auxiliary_of = |state: &CanonicalWaveState| match state.auxiliaries() {
                    CanonicalAuxiliaryState::Linear(auxiliaries) => auxiliaries
                        .thin_gap_jump()
                        .iter()
                        .chain(auxiliaries.outgoing_z())
                        .copied()
                        .collect::<Vec<_>>(),
                    _ => Vec::new(),
                };
                let mut previous = state.clone();
                for _ in 0..steps {
                    previous = state.clone();
                    state.step_with_forcing(operator, &forcing).unwrap();
                }
                let snapshot = funfern_core::CanonicalIndicatorSnapshot {
                    mesh_revision: prepared.mesh.mesh_revision,
                    primary_flux: state.primary_flux().to_vec(),
                    previous_primary_flux: previous.primary_flux().to_vec(),
                    complementary_flux: state.complementary_flux().to_vec(),
                    previous_complementary_flux: previous.complementary_flux().to_vec(),
                    auxiliary: auxiliary_of(&state),
                    previous_auxiliary: auxiliary_of(&previous),
                    integrated_field: Vec::new(),
                    previous_integrated_field: Vec::new(),
                    time: state.time(),
                    time_step: dt,
                };
                let supplement = funfern_core::canonical_indicator_supplement(
                    &prepared.mesh,
                    operator,
                    &forcing,
                    &snapshot,
                )
                .unwrap();
                let velocity =
                    funfern_core::canonical_primary_rate(operator, &forcing, &snapshot).unwrap();
                let displacement = state.primary_field(operator).unwrap();
                (snapshot, supplement, velocity, displacement, None)
            };
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
                volume_acceleration: vec![0.0; count],
                time: snapshot.time,
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
        .with_canonical_supplement(supplement);
        if let Some(runtime) = runtime {
            job = job.with_instantaneous_materials(runtime);
        }
        loop {
            if let Some(result) = job.advance(1 << 16) {
                let result = result.unwrap();
                break (result.report, result.element_indicators);
            }
        }
    }

    /// How much of the adaptation estimate the trapezoid wall step changes:
    /// its second-order trace satisfies the trapezoid relation the estimate's
    /// outgoing-history term checks, so that term falls to roundoff. Across
    /// the gallery's second-order walls after three seconds from rest.
    /// `cargo test -p funfern-app --release --lib measure_what_the_wall_step_does_to_the_estimate -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn measure_what_the_wall_step_does_to_the_estimate() {
        for example in catalog() {
            let prepared = prepare(&example.document, 0.16);
            if prepared.canonical_operator.outgoing_boundary().is_none() {
                continue;
            }
            let [(halves, halves_elements), (trapezoid, trapezoid_elements)] =
                [CanonicalWallStep::HalfKicks, CanonicalWallStep::Trapezoid]
                    .map(|wall_step| estimate_after(&prepared, wall_step, 3.0));
            let largest = halves_elements.iter().fold(0.0_f64, |m, v| m.max(*v));
            let moved = halves_elements
                .iter()
                .zip(&trapezoid_elements)
                .filter(|(a, b)| (*a - *b).abs() > 0.1 * a.abs().max(**b).max(1e-3 * largest))
                .count();
            eprintln!(
                "{}: outgoing {:.2e} -> {:.2e}, boundary {:.3e} -> {:.3e}, total {:.3e} -> {:.3e}, global {:.4} -> {:.4}, refine {} -> {}, elements moved >10%: {} of {}",
                example.name,
                halves.outgoing_history_contribution,
                trapezoid.outgoing_history_contribution,
                halves.boundary_residual_contribution,
                trapezoid.boundary_residual_contribution,
                halves.total_residual,
                trapezoid.total_residual,
                halves.global_indicator,
                trapezoid.global_indicator,
                halves.refine_candidates,
                trapezoid.refine_candidates,
                moved,
                halves_elements.len()
            );
        }
    }

    /// The kick form, which a generation with a stiffness-side field law
    /// steps with, against the second-order wall's law: the channel's second
    /// transverse mode as in
    /// `an_outgoing_wall_returns_what_its_law_says_between_free_and_pinned_sides`,
    /// in a medium whose stiffness-side Kerr law is too weak to be read
    /// (`χ = 1e-9`) but takes the generation to the kick form, at that
    /// generation's own step. Under the half kicks the wall returns what it
    /// does in the drift form; under the trapezoid wall step, whose kicks read
    /// the wall's own rate beside `s − F`, it returns its law to the mesh's
    /// percent: 1.0195 at 0.9 of the bound, the half kicks 1.0532. A field
    /// law, however weak, sends every node through its inverse, and the run
    /// takes about seventeen minutes, so it is a measurement:
    /// `cargo test -p funfern-app --release --lib measure_the_kick_forms_wall -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn measure_the_kick_forms_wall_against_its_law() {
        let edge = 0.05;
        let signal = TimeSignal::pulsed(
            [0.0, 1.0, 2.2, 0.0],
            PulseEnvelope::Gaussian { width: 0.35 },
            0.1,
            0.0,
        );
        let points = [0.3, 0.5, 0.7]
            .into_iter()
            .flat_map(|x| [0.125, -0.125, 0.375].map(|y| Point2::new(x, y)))
            .collect::<Vec<_>>();
        let weak = FieldLaw::Polynomial {
            chi1: ScalarField::constant(0.0),
            chi2: ScalarField::constant(1.0e-9),
            amplitude_bound: None,
        };
        let walls = [
            OuterBoundaryCondition::SecondOrderOutgoing,
            OuterBoundaryCondition::Reflecting,
            OuterBoundaryCondition::Dirichlet {
                signal: TimeSignal::ZERO,
            },
        ];
        let nears = walls.map(|wall| {
            wall_channel(
                -3.0,
                1.0,
                wall,
                edge,
                "cos(2 * pi * y)",
                signal,
                None,
                Some(weak.clone()),
            )
        });
        for near in &nears {
            let operator = near.canonical_temporal_operator.as_ref().unwrap();
            assert_eq!(operator.fourth_order_form(), FourthOrderForm::Kick);
        }
        let dt = nears
            .iter()
            .map(|near| near.recommended_time_step())
            .fold(f64::INFINITY, f64::min);
        let steps = (7.5 / dt).ceil() as usize;
        let references = [&nears[1], &nears[2]].map(|near| {
            temporal_wall_traces(near, dt, steps, &points, CanonicalWallStep::HalfKicks)
        });
        let walled = [CanonicalWallStep::HalfKicks, CanonicalWallStep::Trapezoid]
            .map(|wall_step| temporal_wall_traces(&nears[0], dt, steps, &points, wall_step));
        let spectrum = |trace: &[f64], hz: f64| {
            let (mut re, mut im) = (0.0, 0.0);
            for (step, u) in trace.iter().enumerate() {
                let phase = std::f64::consts::TAU * hz * (step + 1) as f64 * dt;
                re += u * phase.cos() * dt;
                im -= u * phase.sin() * dt;
            }
            (re, im)
        };
        let norm = |(re, im): (f64, f64)| re * re + im * im;
        let tangential = std::f64::consts::TAU;
        let mut returned = [0.0; 2];
        let mut predicted = 0.0;
        for tenth in 16..=22 {
            let hz = f64::from(tenth) / 10.0;
            let cos = (1.0 - (tangential / (std::f64::consts::TAU * hz)).powi(2)).sqrt();
            let law = second_order_reflection(cos, std::f64::consts::TAU * hz, tangential);
            for point in 0..points.len() {
                let (n, d) = (
                    spectrum(&references[0][point], hz),
                    spectrum(&references[1][point], hz),
                );
                let incident = (0.5 * (n.0 + d.0), 0.5 * (n.1 + d.1));
                predicted += law * norm((0.5 * (n.0 - d.0), 0.5 * (n.1 - d.1)));
                for (returned, traces) in returned.iter_mut().zip(&walled) {
                    let at = spectrum(&traces[point], hz);
                    *returned += norm((at.0 - incident.0, at.1 - incident.1));
                }
            }
        }
        let [halves, trapezoid] = returned.map(|returned| returned / predicted);
        eprintln!(
            "kick form at {:.3e} s: second order over its law, half kicks {halves:.4}, trapezoid wall step {trapezoid:.4}",
            dt
        );
        assert!((trapezoid - 1.0).abs() < 0.02, "{trapezoid}");
    }

    /// The trapezoid wall step solves the wall over the whole step, twice
    /// the half kick's duration, and the sweep's contraction grows about as
    /// the duration's square. Measured across the gallery at the app's step.
    /// `cargo test -p funfern-app --release --lib measure_the_wall_solves -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn measure_the_wall_solves_contraction_across_the_gallery() {
        for edge in [0.16, 0.08, 0.05] {
            let mut worst = (0.0, String::new());
            for example in catalog() {
                let prepared = prepare(&example.document, edge);
                let operator = &prepared.canonical_operator;
                if operator.outgoing_boundary().is_none() {
                    continue;
                }
                let dt = operator.recommended_time_step();
                let measure = |wall_step| {
                    let state = CanonicalWaveState::zero_for_backend(operator, dt)
                        .unwrap()
                        .with_wall_step(operator, wall_step)
                        .unwrap();
                    let factor = state.outgoing_midpoint_factor().unwrap();
                    (
                        factor.contraction(),
                        factor.sweeps(),
                        factor.device_sweeps(),
                    )
                };
                let halves = measure(CanonicalWallStep::HalfKicks);
                let trapezoid = measure(CanonicalWallStep::Trapezoid);
                eprintln!(
                    "edge {edge} {}: half kicks {:.3} ({} / {} sweeps), trapezoid {:.3} ({} / {} sweeps)",
                    example.name,
                    halves.0,
                    halves.1,
                    halves.2,
                    trapezoid.0,
                    trapezoid.1,
                    trapezoid.2
                );
                if trapezoid.0 > worst.0 {
                    worst = (trapezoid.0, example.name.to_string());
                }
            }
            eprintln!(
                "edge {edge}: worst trapezoid contraction {:.3} in {}",
                worst.0, worst.1
            );
        }
    }

    /// A pinned or electric-wall side beside a second-order outgoing one pins
    /// the corner nodes they share, which end the outgoing wall's trace: they
    /// carry its damping but are not on it, and nothing on it is pinned.
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
            let pinned_corners = prepared
                .operator
                .second_order_boundary_damping()
                .iter()
                .zip(prepared.canonical_forcing.prescribed())
                .filter(|(damping, pin)| **damping > 0.0 && pin.is_some())
                .count();
            assert!(pinned_corners >= 2, "{}: no pinned corner", example.name);
            assert!(
                boundary
                    .trace_nodes()
                    .iter()
                    .all(|node| prepared.canonical_forcing.prescribed()[*node as usize].is_none()),
                "{}: a pin on the trace",
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

    /// The material lens's claim: behind the lens, on its axis, the light
    /// converges to a spot - brighter than the bare source leaves there, and
    /// brighter than either side of it.
    #[test]
    fn the_material_lens_focuses_its_source_behind_it() {
        let lens = Harmonic::run(&material_lens(), 0.08, 6.0, 3.5, 3.0);
        let bare = Harmonic::run(&material_lens_with(false), 0.08, 6.0, 3.5, 3.0);
        let (focus, peak) = (0..=50)
            .map(|index| {
                let point = Point2::new(0.25 + 0.6 * f64::from(index) / 50.0, 0.0);
                (point, lens.at(point))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        assert!(
            (0.3..=0.6).contains(&focus.x),
            "the brightest point behind the lens is at {:.3}",
            focus.x
        );
        let unfocused = bare.at(focus);
        assert!(
            peak > 2.0 * unfocused,
            "{peak:.4} with the lens, {unfocused:.4} without"
        );
        for side in [1.0, -1.0] {
            let flank = lens.at(focus + Point2::new(0.0, 0.15 * side));
            assert!(
                flank < 0.5 * peak,
                "the spot's flank holds {flank:.4} of {peak:.4}"
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
        // The scene shows E_z, and the probe in the app reads it, not r: a
        // train of voltage pulses, one per fluxon, whose strongest line is
        // the same tone.
        let voltage = rows.iter().map(|row| row.2[2]).collect::<Vec<_>>();
        let lines = Lines::read(
            &josephson_line(),
            ProbeId(1),
            &voltage,
            rows[1].0 - rows[0].0,
            JOSEPHSON_BIAS / tau,
        );
        let (strongest, _) = lines.strongest(0.1, 3.0);
        assert!(
            (strongest - JOSEPHSON_BIAS / tau).abs() < 1.0 / lines.span,
            "E_z's strongest line is at {strongest:.3} Hz"
        );
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
    const INTERFACE_X: f64 = 0.1;
    const INTERFACE_HZ: f64 = 1.5;
    const INTERFACE_WIDTH: f64 = 0.1;
    const INTERFACE_START: f64 = 0.1;

    struct InterfaceRun {
        /// Upper arm left of the interface, upper arm right of it, lower arm.
        energies: [f64; 3],
        /// How far the upper probe's pulse arrives behind the lower one's.
        lag: f64,
        /// What that should be, from where the probe nodes are.
        expected_lag: f64,
    }

    /// A pulse down the two-arm channel at normal incidence on a planar
    /// interface: medium 2 fills the upper arm from `INTERFACE_X` on, and the
    /// launcher at `x = −0.85` lights both arms. The Gaussian, σ = 0.1 s on a
    /// 1.5 Hz carrier and cut at 4σ, is 0.8 long in medium 1 and 0.4 in
    /// medium 2. Four σ after its peak crosses the interface, the reflected
    /// pulse spans −0.7 to 0.1, the transmitted 0.1 to 0.5 and the reference
    /// 0.1 to 0.9, each clear of the interface, the launcher and the far wall:
    /// the energy then, `½Q·u` a node and `½w b·v` a sample, summed over each
    /// arm right of the launcher, the upper one either side of the interface.
    /// The probes at `(0.5, ±0.5)` time the pulse past them.
    fn interface_run(
        polarization: ElectromagneticPolarization,
        permittivity: f64,
        permeability: f64,
    ) -> InterfaceRun {
        let mut builder = Builder::new();
        builder.scene.physics = PhysicsModel::Electromagnetic { polarization };
        builder.scene.outer_boundaries = channel();
        builder.scene.materials.push(Material {
            id: MaterialId(2),
            name: "Medium 2".into(),
            mass_density: ScalarField::constant(permittivity),
            stiffness: ScalarField::constant(permeability),
            ..Material::default_medium()
        });
        let pulse = TimeSignal::pulsed(
            [0.0, 1.0, INTERFACE_HZ, 0.0],
            PulseEnvelope::Gaussian {
                width: INTERFACE_WIDTH,
            },
            INTERFACE_START,
            0.0,
        );
        builder.arms(-0.85, pulse, &[(INTERFACE_X, 0.95, MaterialId(2))], None);
        let mut document = builder.document();
        document.model.source.enabled = false;
        let prepared = prepare(&document, 0.035);
        assert!(prepared.canonical_temporal_operator.is_none());
        let operator = &prepared.canonical_operator;
        let peak = INTERFACE_START + GAUSSIAN_CUT_WIDTHS * INTERFACE_WIDTH;
        let hit = peak + (INTERFACE_X + 0.85);
        let snapshot = hit + GAUSSIAN_CUT_WIDTHS * INTERFACE_WIDTH;
        // The transmitted pulse passes the upper probe from 0.4 to 1.2 s after
        // the hit, and its echo from the layer's far end is not back before
        // 2.2 s. The reference passes the lower probe from 0 to 0.8 s, and its
        // front could be back from the far wall at 1 s.
        let ends = [hit + 1.4, hit + 1.0];
        let probes = [Point2::new(0.5, 0.5), Point2::new(0.5, -0.5)].map(|point| {
            operator
                .node_points()
                .iter()
                .enumerate()
                .min_by(|a, b| (*a.1 - point).norm().total_cmp(&(*b.1 - point).norm()))
                .unwrap()
                .0
        });
        // The wall along `y = 0` carries a node for each arm at one point:
        // which arm a node is in is the arm of the elements around it.
        let mut lower = vec![false; operator.node_points().len()];
        for nodes in operator.element_nodes() {
            let below = operator.node_points()[nodes[6] as usize].y < 0.0;
            for node in nodes {
                lower[*node as usize] = below;
            }
        }
        let mut state =
            CanonicalWaveState::zero(operator, prepared.recommended_time_step()).unwrap();
        let mut energies = None;
        let mut traces = [Vec::new(), Vec::new()];
        while state.time() < ends[0] {
            state
                .step_with_forcing(operator, &prepared.canonical_forcing)
                .unwrap();
            let field = state.primary_field(operator).unwrap();
            for ((trace, node), end) in traces.iter_mut().zip(probes).zip(ends) {
                if state.time() < end {
                    trace.push((state.time(), field[node]));
                }
            }
            if energies.is_some() || state.time() < snapshot {
                continue;
            }
            let mut shares = [0.0; 3];
            let mut add = |point: Point2, lower: bool, energy: f64| {
                if point.x < -0.8 {
                    return;
                }
                let lane = if lower {
                    2
                } else if point.x < INTERFACE_X {
                    0
                } else {
                    1
                };
                shares[lane] += energy;
            };
            for (((point, q), u), lower) in operator
                .node_points()
                .iter()
                .zip(state.primary_flux())
                .zip(&field)
                .zip(&lower)
            {
                add(*point, *lower, 0.5 * q * u);
            }
            let v = state.complementary_field(operator).unwrap();
            for ((sample, b), v) in operator
                .constitutive_samples()
                .iter()
                .zip(state.complementary_flux())
                .zip(&v)
            {
                add(
                    sample.point,
                    sample.point.y < 0.0,
                    0.5 * sample.integration_weight * b.dot(*v),
                );
            }
            energies = Some(shares);
        }
        // Where a pulse sits in time at a probe: the centroid of `u²`.
        let [upper, lower] = traces.map(|trace| {
            let weight: f64 = trace.iter().map(|(_, u)| u * u).sum();
            trace.iter().map(|(t, u)| t * u * u).sum::<f64>() / weight
        });
        let speed = (permittivity * permeability).sqrt().recip();
        let [upper_x, lower_x] = probes.map(|node| operator.node_points()[node].x);
        InterfaceRun {
            energies: energies.unwrap(),
            lag: upper - lower,
            expected_lag: (upper_x - INTERFACE_X) / speed + (INTERFACE_X - lower_x),
        }
    }

    /// A pulse at normal incidence on a planar interface, in TM and in TE,
    /// against the reference arm. Medium 2 with ε = 4, μ = 1 has half the
    /// impedance and half the speed: it reflects R = ((Z₂ − Z₁)/(Z₂ + Z₁))² =
    /// 1/9 of the energy, passes the rest, and the transmitted pulse lags by
    /// the time the slower medium adds. ε = μ = 2 is as slow and matched, and
    /// reflects nothing. At this mesh R misses 1/9 by ±9e-5, TM over and TE
    /// under, R + T misses 1 by 2e-6, the matched R is 1e-6 and the lag misses
    /// by 2e-5 s. An impedance 0.2% off at the right speed moves R by 6e-4.
    #[test]
    fn a_pulse_at_an_interface_splits_as_fresnel_says_and_lags_by_the_slower_medium() {
        for polarization in [
            ElectromagneticPolarization::Tm,
            ElectromagneticPolarization::Te,
        ] {
            for (permittivity, permeability, fresnel, tolerance) in
                [(4.0, 1.0, 1.0 / 9.0, 3.0e-4), (2.0, 2.0, 0.0, 1.0e-5)]
            {
                let run = interface_run(polarization, permittivity, permeability);
                let [reflected, transmitted, incident] = run.energies;
                let (r, t) = (reflected / incident, transmitted / incident);
                let case = format!("{polarization:?}, ε {permittivity}, μ {permeability}");
                eprintln!(
                    "{case}: R {r:.6}, off {:.2e}, R + T − 1 {:.2e}, lag {:.5} off {:.2e}",
                    r - fresnel,
                    r + t - 1.0,
                    run.lag,
                    run.lag - run.expected_lag
                );
                assert!((r - fresnel).abs() < tolerance, "{case}: R {r}");
                assert!((r + t - 1.0).abs() < 1.0e-5, "{case}: R + T {}", r + t);
                assert!(
                    (run.lag - run.expected_lag).abs() < 1.0e-4,
                    "{case}: lag {} against {}",
                    run.lag,
                    run.expected_lag
                );
            }
        }
    }

    /// A channel `[left, right] × [-0.5, 0.5]` of the unit medium with `wall`
    /// at its right end, reflecting sides unless `sides` names the physics
    /// and the condition they take, and a strip at `x = -0.7` across it
    /// radiating `signal` under `profile`. `stiffness` gives the medium a
    /// field law on its stiffness side.
    #[allow(clippy::too_many_arguments)]
    fn wall_channel(
        left: f64,
        right: f64,
        wall: OuterBoundaryCondition,
        edge: f64,
        profile: &str,
        signal: TimeSignal,
        sides: Option<(PhysicsModel, OuterBoundaryCondition)>,
        stiffness: Option<FieldLaw>,
    ) -> Arc<crate::topology_runtime::PreparedTopology> {
        let mut builder = Builder::new();
        if let Some(law) = stiffness {
            builder.scene.materials[0].stiffness_law.field = law;
        }
        builder.scene.geometry.domain = DomainRect {
            min_x: left,
            max_x: right,
            min_y: -0.5,
            max_y: 0.5,
        };
        // The background's anchor, on the floor at x = 0 as the builder
        // expects, right of the launcher.
        builder.scene.face_assignments[0].anchor = FaceAnchor::Outer {
            side: OuterSide::Bottom,
            fraction: -left / (right - left),
        };
        let mut boundaries = channel();
        boundaries.sides[OuterSide::Right.index()] = wall;
        if let Some((physics, transverse)) = sides {
            builder.scene.physics = physics;
            boundaries.sides[OuterSide::Top.index()] = transverse;
            boundaries.sides[OuterSide::Bottom.index()] = transverse;
        }
        builder.scene.outer_boundaries = boundaries;
        builder.launcher(-0.7, INTERFACE_HZ, 1.0);
        let launcher = builder.scene.volume_sources.last_mut().unwrap();
        launcher.signal = signal;
        launcher.profile = ScalarField::formula(profile).unwrap();
        let mut document = builder.document();
        document.model.source.enabled = false;
        prepare(&document, edge)
    }

    /// The primary field at `points` after each of `steps` steps of `dt`,
    /// the walls stepped `wall_step`'s way.
    fn wall_traces(
        prepared: &crate::topology_runtime::PreparedTopology,
        dt: f64,
        steps: usize,
        points: &[Point2],
        wall_step: CanonicalWallStep,
    ) -> Vec<Vec<f64>> {
        field_and_gap_traces(prepared, dt, steps, points, wall_step).0
    }

    /// `wall_traces`, and each thin-gap sample's jump memory after each step.
    fn field_and_gap_traces(
        prepared: &crate::topology_runtime::PreparedTopology,
        dt: f64,
        steps: usize,
        points: &[Point2],
        wall_step: CanonicalWallStep,
    ) -> (Vec<Vec<f64>>, Vec<Vec<f64>>) {
        let operator = &prepared.canonical_operator;
        let mut state = CanonicalWaveState::zero(operator, dt)
            .unwrap()
            .with_wall_step(operator, wall_step)
            .unwrap();
        record_steps(
            &mut state,
            prepared,
            steps,
            &point_stencils(prepared, points),
        )
    }

    /// The quadratic stencils that read the primary field at `points`.
    fn point_stencils(
        prepared: &crate::topology_runtime::PreparedTopology,
        points: &[Point2],
    ) -> Vec<QuadraticPointStencil> {
        points
            .iter()
            .map(|point| {
                QuadraticPointStencil::build_topology(
                    &prepared.mesh,
                    &prepared.operator,
                    &prepared.bundle.plan,
                    prepared.fixed_model(),
                    *point,
                )
                .unwrap()
            })
            .collect()
    }

    /// Steps `state` on `prepared` `steps` times, recording the primary field
    /// through `stencils` and each thin-gap sample's jump memory after each.
    fn record_steps(
        state: &mut CanonicalWaveState,
        prepared: &crate::topology_runtime::PreparedTopology,
        steps: usize,
        stencils: &[QuadraticPointStencil],
    ) -> (Vec<Vec<f64>>, Vec<Vec<f64>>) {
        let operator = &prepared.canonical_operator;
        let mut traces = vec![Vec::with_capacity(steps); stencils.len()];
        let mut gaps = vec![Vec::with_capacity(steps); operator.thin_gap_samples().len()];
        for _ in 0..steps {
            state
                .step_with_forcing(operator, &prepared.canonical_forcing)
                .unwrap();
            for (trace, memory) in gaps
                .iter_mut()
                .zip(state.thin_gap_memory(operator).unwrap())
            {
                trace.push(memory.jump);
            }
            let field = state.primary_field(operator).unwrap();
            for (trace, stencil) in traces.iter_mut().zip(stencils) {
                trace.push(
                    stencil
                        .nodes
                        .iter()
                        .zip(stencil.value_weights)
                        .map(|(node, weight)| weight * field[*node as usize])
                        .sum(),
                );
            }
        }
        (traces, gaps)
    }

    /// The primary field at `points` after each of `steps` steps of `dt` on a
    /// driven generation, the walls stepped `wall_step`'s way.
    fn temporal_wall_traces(
        prepared: &crate::topology_runtime::PreparedTopology,
        dt: f64,
        steps: usize,
        points: &[Point2],
        wall_step: CanonicalWallStep,
    ) -> Vec<Vec<f64>> {
        let stencils = points
            .iter()
            .map(|point| {
                QuadraticPointStencil::build_topology(
                    &prepared.mesh,
                    &prepared.operator,
                    &prepared.bundle.plan,
                    prepared.fixed_model(),
                    *point,
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let operator = prepared.canonical_temporal_operator.as_ref().unwrap();
        let mut state = CanonicalTemporalWaveState::zero(operator, dt)
            .unwrap()
            .with_wall_step(wall_step);
        let mut traces = vec![Vec::with_capacity(steps); points.len()];
        for _ in 0..steps {
            state
                .step_with_forcing(operator, &prepared.canonical_forcing)
                .unwrap();
            let field = operator
                .primary_field_at(state.primary_flux(), state.time(), state.runtime())
                .unwrap();
            for (trace, stencil) in traces.iter_mut().zip(&stencils) {
                trace.push(
                    stencil
                        .nodes
                        .iter()
                        .zip(stencil.value_weights)
                        .map(|(node, weight)| weight * field[*node as usize])
                        .sum(),
                );
            }
        }
        traces
    }

    /// The first-order wall's energy reflection at `cos θ` from the normal,
    /// a wall of coefficient one: `((1 − cos θ)/(1 + cos θ))²`.
    fn first_order_reflection(cos: f64) -> f64 {
        ((1.0 - cos) / (1.0 + cos)).powi(2)
    }

    /// The second-order wall's, at `cos θ` and angular frequency `w` with
    /// tangential wavenumber `k`: the passive law of the boundary spike,
    /// `Y(s) = 1 + (6d/7)/s − (8d/7)/(s + d) + (2d/7)/(s + 2d)` at `s = iw`,
    /// `d = √(7/8) k`, and `|(cos θ − Y)/(cos θ + Y)|²`.
    fn second_order_reflection(cos: f64, w: f64, k: f64) -> f64 {
        let d = (7.0f64 / 8.0).sqrt() * k;
        let inverse = |re: f64, im: f64| (re / (re * re + im * im), -im / (re * re + im * im));
        let poles = [inverse(0.0, w), inverse(d, w), inverse(2.0 * d, w)];
        let residues = [6.0 * d / 7.0, -8.0 * d / 7.0, 2.0 * d / 7.0];
        let (mut re, mut im) = (1.0, 0.0);
        for (pole, residue) in poles.iter().zip(residues) {
            re += residue * pole.0;
            im += residue * pole.1;
        }
        ((cos - re).powi(2) + im * im) / ((cos + re).powi(2) + im * im)
    }

    /// An outgoing wall against a far domain, then against its own law. At
    /// normal incidence a pulse down the channel `[-1, 1] × [-0.5, 0.5]` meets
    /// the right wall; the same run with that wall at `x = 3` can return
    /// nothing in the time, so the near run less the far run at nine points
    /// is what the wall sent back: a reflecting wall all of it, either
    /// outgoing wall under 2e-5 of it, over a floor of 3e-6 from the two
    /// meshes. Both orders are exact there, so the wall is then met by the
    /// channel's second transverse mode, `cos(2πy)`, whose angle at the wall
    /// falls from 39° at 1.6 Hz to 27° at 2.2 Hz, under a pulse 0.35 s wide
    /// on 2.2 Hz with the left wall at `x = -3`, and read on one mesh: a
    /// reflecting wall returns the incident wave and a zero-Dirichlet wall
    /// its negative, so their mean is the incident wave and their
    /// half-difference what a wall of coefficient one returns. Over the band
    /// the first-order wall's energy reflection matches its law to 1%; the
    /// second-order wall's, under the default trapezoid wall step
    /// (`docs/spikes/funfern-outgoing-wall-step-spike.md`), reads 1.01, the
    /// mesh's own, and is held to 2%. The half kicks it replaced read 1.10 of
    /// the passive law at the default step, 1.03 at half of it and 1.02 at a
    /// quarter, an excess in their stepping, not the law, and are held to 15%
    /// for comparison; at normal incidence, where the two meshes' difference
    /// hides it, the trapezoid step's near run is held to a fifth of the half
    /// kicks' distance from where both converge, a quarter of the step on the
    /// same mesh. Between electric walls in TM the field vanishes at the corners,
    /// the wall's trace ends there, and `sin(2πy)` must reflect as `cos(2πy)`
    /// does between reflecting sides, within 2%: with the corners kept on the
    /// trace it reflected 6.6 times its law. Electric walls in TM and zero
    /// Dirichlet sides in a mechanical medium resolve to one operator.
    #[test]
    fn an_outgoing_wall_returns_what_its_law_says_between_free_and_pinned_sides() {
        let edge = 0.05;
        let pulse = TimeSignal::pulsed(
            [0.0, 1.0, INTERFACE_HZ, 0.0],
            PulseEnvelope::Gaussian {
                width: INTERFACE_WIDTH,
            },
            INTERFACE_START,
            0.0,
        );
        let points = (1..=9)
            .flat_map(|i| [-0.3, 0.0, 0.3].map(|y| Point2::new(0.1 * f64::from(i), y)))
            .collect::<Vec<_>>();
        let far = wall_channel(
            -1.0,
            3.0,
            OuterBoundaryCondition::SecondOrderOutgoing,
            edge,
            "1",
            pulse,
            None,
            None,
        );
        let walls = [
            OuterBoundaryCondition::Reflecting,
            OuterBoundaryCondition::FirstOrderOutgoing,
            OuterBoundaryCondition::SecondOrderOutgoing,
        ];
        let nears = walls.map(|wall| wall_channel(-1.0, 1.0, wall, edge, "1", pulse, None, None));
        let dt = nears
            .iter()
            .map(|near| near.recommended_time_step())
            .fold(far.recommended_time_step(), f64::min);
        let steps = (3.6 / dt).ceil() as usize;
        let far_traces = wall_traces(&far, dt, steps, &points, CanonicalWallStep::HalfKicks);
        let incident: f64 = far_traces.iter().flatten().map(|f| f * f).sum();
        // The energy between two sets of traces, over the incident pulse's;
        // `every` reads the other at each `every`-th sample.
        let apart = |near: &[Vec<f64>], other: &[Vec<f64>], every: usize| {
            near.iter()
                .zip(other)
                .flat_map(|(near, other)| {
                    near.iter()
                        .enumerate()
                        .map(move |(step, n)| n - other[every * step + every - 1])
                })
                .map(|difference| difference * difference)
                .sum::<f64>()
                / incident
        };
        let returned = nears.each_ref().map(|near| {
            apart(
                &wall_traces(near, dt, steps, &points, CanonicalWallStep::HalfKicks),
                &far_traces,
                1,
            )
        });
        let second = [CanonicalWallStep::HalfKicks, CanonicalWallStep::Trapezoid]
            .map(|wall_step| wall_traces(&nears[2], dt, steps, &points, wall_step));
        let trapezoid = [
            apart(
                &wall_traces(&nears[1], dt, steps, &points, CanonicalWallStep::Trapezoid),
                &far_traces,
                1,
            ),
            apart(&second[1], &far_traces, 1),
        ];
        // Against the same mesh at a quarter of the step, the far mesh's own
        // difference left out: how far each way of stepping the wall is from
        // where both converge.
        let quarter = wall_traces(
            &nears[2],
            0.25 * dt,
            4 * steps,
            &points,
            CanonicalWallStep::Trapezoid,
        );
        let converged = second.each_ref().map(|traces| apart(traces, &quarter, 4));
        eprintln!(
            "normal incidence: reflecting {:.4}, first order {:.2e}, second order {:.2e}; trapezoid wall step first {:.2e} second {:.2e}; second order from its quarter step: half kicks {:.2e}, trapezoid {:.2e}",
            returned[0],
            returned[1],
            returned[2],
            trapezoid[0],
            trapezoid[1],
            converged[0],
            converged[1]
        );
        assert!((returned[0] - 1.0).abs() < 0.01, "{}", returned[0]);
        assert!(returned[1] < 2.0e-5, "{}", returned[1]);
        assert!(returned[2] < 2.0e-5, "{}", returned[2]);
        assert!(trapezoid[0] < 2.0e-5, "{}", trapezoid[0]);
        assert!(trapezoid[1] < 2.0e-5, "{}", trapezoid[1]);
        assert!(converged[1] < 0.2 * converged[0], "{converged:?}");

        let signal = TimeSignal::pulsed(
            [0.0, 1.0, 2.2, 0.0],
            PulseEnvelope::Gaussian { width: 0.35 },
            0.1,
            0.0,
        );
        // Where both `cos(2πy)` and `sin(2πy)` are 0.7 in size.
        let points = [0.3, 0.5, 0.7]
            .into_iter()
            .flat_map(|x| [0.125, -0.125, 0.375].map(|y| Point2::new(x, y)))
            .collect::<Vec<_>>();
        let tangential = std::f64::consts::TAU;
        // The band's energy reflection over its law, first and second order.
        let band = |profile: &str, sides: Option<(PhysicsModel, OuterBoundaryCondition)>| {
            let walls = [
                OuterBoundaryCondition::FirstOrderOutgoing,
                OuterBoundaryCondition::SecondOrderOutgoing,
                OuterBoundaryCondition::Reflecting,
                OuterBoundaryCondition::Dirichlet {
                    signal: TimeSignal::ZERO,
                },
            ];
            let nears =
                walls.map(|wall| wall_channel(-3.0, 1.0, wall, edge, profile, signal, sides, None));
            let dt = nears
                .iter()
                .map(|near| near.recommended_time_step())
                .fold(f64::INFINITY, f64::min);
            let steps = (7.5 / dt).ceil() as usize;
            let traces = nears
                .each_ref()
                .map(|near| wall_traces(near, dt, steps, &points, CanonicalWallStep::HalfKicks));
            let trapezoid = [&nears[0], &nears[1]]
                .map(|near| wall_traces(near, dt, steps, &points, CanonicalWallStep::Trapezoid));
            let spectrum = |trace: &[f64], hz: f64| {
                let (mut re, mut im) = (0.0, 0.0);
                for (step, u) in trace.iter().enumerate() {
                    let phase = std::f64::consts::TAU * hz * (step + 1) as f64 * dt;
                    re += u * phase.cos() * dt;
                    im -= u * phase.sin() * dt;
                }
                (re, im)
            };
            let norm = |(re, im): (f64, f64)| re * re + im * im;
            let mut returned = [0.0; 4];
            let mut predicted = [0.0; 2];
            for tenth in 16..=22 {
                let hz = f64::from(tenth) / 10.0;
                let cos = (1.0 - (tangential / (std::f64::consts::TAU * hz)).powi(2)).sqrt();
                let law = [
                    first_order_reflection(cos),
                    second_order_reflection(cos, std::f64::consts::TAU * hz, tangential),
                ];
                for point in 0..points.len() {
                    let at = traces.each_ref().map(|trace| spectrum(&trace[point], hz));
                    let stepped = trapezoid
                        .each_ref()
                        .map(|trace| spectrum(&trace[point], hz));
                    let (n, d) = (at[2], at[3]);
                    let incident = (0.5 * (n.0 + d.0), 0.5 * (n.1 + d.1));
                    let unit = norm((0.5 * (n.0 - d.0), 0.5 * (n.1 - d.1)));
                    for wall in 0..2 {
                        returned[wall] += norm((at[wall].0 - incident.0, at[wall].1 - incident.1));
                        returned[2 + wall] +=
                            norm((stepped[wall].0 - incident.0, stepped[wall].1 - incident.1));
                        predicted[wall] += law[wall] * unit;
                    }
                }
            }
            [
                returned[0] / predicted[0],
                returned[1] / predicted[1],
                returned[2] / predicted[0],
                returned[3] / predicted[1],
            ]
        };
        let free = band("cos(2 * pi * y)", None);
        let pinned = band(
            "sin(2 * pi * y)",
            Some((
                PhysicsModel::Electromagnetic {
                    polarization: ElectromagneticPolarization::Tm,
                },
                OuterBoundaryCondition::ElectricWall,
            )),
        );
        eprintln!(
            "oblique, over the law, half kicks: free sides first {:.4} second {:.4}; pinned sides first {:.4} second {:.4}",
            free[0], free[1], pinned[0], pinned[1]
        );
        eprintln!(
            "oblique, trapezoid wall step: free sides first {:.4} second {:.4}; pinned sides first {:.4} second {:.4}",
            free[2], free[3], pinned[2], pinned[3]
        );
        assert!((free[0] - 1.0).abs() < 0.01, "{}", free[0]);
        assert!(free[1] > 0.95 && free[1] < 1.15, "{}", free[1]);
        assert!((pinned[0] - 1.0).abs() < 0.01, "{}", pinned[0]);
        assert!(
            (pinned[1] / free[1] - 1.0).abs() < 0.02,
            "{} / {}",
            pinned[1],
            free[1]
        );
        assert!((free[2] - 1.0).abs() < 0.01, "{}", free[2]);
        assert!((free[3] - 1.0).abs() < 0.02, "{}", free[3]);
        assert!((pinned[2] - 1.0).abs() < 0.01, "{}", pinned[2]);
        assert!(
            (pinned[3] / free[3] - 1.0).abs() < 0.02,
            "{} / {}",
            pinned[3],
            free[3]
        );

        let resolved = [
            (
                PhysicsModel::Electromagnetic {
                    polarization: ElectromagneticPolarization::Tm,
                },
                OuterBoundaryCondition::ElectricWall,
            ),
            (
                PhysicsModel::Mechanical,
                OuterBoundaryCondition::Dirichlet {
                    signal: TimeSignal::ZERO,
                },
            ),
        ]
        .map(|sides| {
            wall_channel(
                -3.0,
                1.0,
                OuterBoundaryCondition::SecondOrderOutgoing,
                edge,
                "1",
                signal,
                Some(sides),
                None,
            )
        });
        let [tm, mechanical] = resolved
            .each_ref()
            .map(|prepared| &prepared.canonical_operator);
        assert_eq!(
            tm.outgoing_boundary().unwrap().trace_nodes(),
            mechanical.outgoing_boundary().unwrap().trace_nodes()
        );
        assert_eq!(tm.primary_mass(), mechanical.primary_mass());
    }

    /// A circle of radius `radius` about the origin as a periodic spline of
    /// `controls` uniform controls, reached out by the factor the uniform
    /// cubic B-spline draws a circle's controls in by, `(2 + cos(2π/n))/3`.
    fn round_circle(radius: f64, controls: usize) -> PeriodicCubicSpline {
        let step = std::f64::consts::TAU / controls as f64;
        let reach = radius / ((2.0 + step.cos()) / 3.0);
        PeriodicCubicSpline::uniform(
            (0..controls)
                .map(|index| Point2::default() + direction(index as f64 * step) * reach)
                .collect(),
        )
        .unwrap()
    }

    /// The eigenvalues of a generation's outgoing trace, ascending.
    fn trace_spectrum(prepared: &crate::topology_runtime::PreparedTopology) -> Vec<f64> {
        let mut eigenvalues = prepared
            .canonical_operator
            .outgoing_boundary()
            .unwrap()
            .modes()
            .iter()
            .map(|mode| mode.eigenvalue)
            .collect::<Vec<_>>();
        eigenvalues.sort_by(f64::total_cmp);
        eigenvalues
    }

    /// A second-order wall on a curve inside the domain solves its own
    /// trace's tangential operator, as an outer side's does
    /// (`a_zero_pinned_side_ends_the_outgoing_trace_and_a_driven_one_does_not`):
    /// on a circle of radius one, at unit speed and impedance, the modes
    /// pair as `cos mθ` and `sin mθ` at `λ_m = m²/2`; on a straight curve one
    /// long, floating in a box, they are its cosines, `λ_n = (nπ)²/2`. Each
    /// edge's nodes reach the face's assembly as endpoint, midpoint,
    /// endpoint, and the line stiffness takes its endpoints first; passed as
    /// they came, the circle read `2λ` 2.28, 9.09, 20.3 and 35.7 for
    /// `m²` 1, 4, 9 and 16, every second-order wall on a curve absorbing by a
    /// law it was not built to (found by PH-06's sizing, 2026-10-14).
    #[test]
    fn an_internal_second_order_wall_solves_its_own_traces_tangential_operator() {
        let wall = SpanBehavior::Separated {
            left: FaceBoundaryCondition::SecondOrderOutgoing,
            right: FaceBoundaryCondition::Reflecting,
            coupling: InternalBoundaryCoupling::Independent,
        };
        let reflecting_box = |half: f64| {
            let mut builder = Builder::new();
            builder.scene.geometry.domain = DomainRect {
                min_x: -half,
                max_x: half,
                min_y: -half,
                max_y: half,
            };
            builder.scene.outer_boundaries =
                OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting);
            builder
        };

        let mut circle = reflecting_box(1.2);
        let material = circle.scene.regions[0].material;
        circle.closed(
            round_circle(1.0, 64),
            wall,
            Some((material, MaterialFrame::world())),
        );
        let mut document = circle.document();
        document.model.source.enabled = false;
        let spectrum = trace_spectrum(&prepare(&document, 0.1));
        assert_eq!(spectrum[0], 0.0);
        for m in 1..=5 {
            let exact = f64::from(m * m) / 2.0;
            let pair = [spectrum[2 * m as usize - 1], spectrum[2 * m as usize]];
            eprintln!("circle m {m}: λ {pair:?} for {exact}");
            for eigenvalue in pair {
                assert!((eigenvalue / exact - 1.0).abs() < 1.0e-3, "m {m}: {pair:?}");
            }
        }

        let mut line = reflecting_box(1.0);
        line.open_curve(
            OpenCubicSpline::polyline(vec![
                Point2::new(0.0, -0.5),
                Point2::new(0.0, 0.0),
                Point2::new(0.0, 0.5),
            ])
            .unwrap(),
            &[wall, wall],
        );
        let mut document = line.document();
        document.model.source.enabled = false;
        let spectrum = trace_spectrum(&prepare(&document, 0.1));
        assert_eq!(spectrum[0], 0.0);
        for n in 1..=3 {
            let exact = (f64::from(n) * std::f64::consts::PI).powi(2) / 2.0;
            eprintln!("line n {n}: λ {} for {exact}", spectrum[n as usize]);
            assert!(
                (spectrum[n as usize] / exact - 1.0).abs() < 1.0e-3,
                "n {n}: {}",
                spectrum[n as usize]
            );
        }
    }

    /// PH-06 and PH-07's reference: for each case, the radii it fits at and
    /// the Hankel functions there and at the wall, written by
    /// `tests/physics/references/cylinder_hankel.py` and checked there by
    /// the Wronskian and at twice the precision.
    #[derive(serde::Deserialize)]
    struct HankelTable {
        cases: Vec<HankelCase>,
    }

    #[derive(Clone, Debug, serde::Deserialize)]
    struct HankelCase {
        kind: String,
        kr: f64,
        m: u32,
        radii: Vec<f64>,
        h1: Vec<[f64; 2]>,
        h2: Vec<[f64; 2]>,
        wall: HankelWall,
    }

    #[derive(Clone, Copy, Debug, serde::Deserialize)]
    struct HankelWall {
        h1: [f64; 2],
        h2: [f64; 2],
        dh1: [f64; 2],
        dh2: [f64; 2],
    }

    impl HankelCase {
        fn hole(&self) -> bool {
            self.kind == "hole"
        }
    }

    /// The table's case, `"outgoing"` or `"hole"`, at `kr` and order `m`.
    fn hankel_case(kind: &str, kr: f64, m: u32) -> HankelCase {
        static TABLE: std::sync::OnceLock<HankelTable> = std::sync::OnceLock::new();
        let table = TABLE.get_or_init(|| {
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/physics/references/cylinder-hankel.json"
            );
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
        });
        table
            .cases
            .iter()
            .find(|case| case.kind == kind && case.kr == kr && case.m == m)
            .cloned()
            .unwrap_or_else(|| panic!("no {kind} case at kR {kr}, m {m} in the table"))
    }

    /// PH-09's reference: `H1_m` and its derivative at the wall, on a uniform
    /// grid of kR for each order from `max(3/2, m)` to 14, written by the same
    /// generator and checked there by the Wronskian, at twice the precision
    /// and by `wall_hankel`'s interpolation at every midpoint.
    #[derive(serde::Deserialize)]
    struct DenseHankelTable {
        step: f64,
        orders: Vec<DenseHankelOrder>,
    }

    #[derive(serde::Deserialize)]
    struct DenseHankelOrder {
        m: u32,
        start: f64,
        count: usize,
        h1: Vec<[f64; 2]>,
        dh1: Vec<[f64; 2]>,
    }

    /// `H1_m(kR)` and `H1_m′(kR)` on the unit circle, by cubic Hermite between
    /// the dense table's points: the value from the value and the derivative
    /// there, the derivative from the derivative and the second, which
    /// Bessel's equation gives, `H″ = −H′/x − (1 − m²/x²) H`. `H2` is the
    /// conjugate.
    fn wall_hankel(m: u32, kr: f64) -> (Phasor, Phasor) {
        static TABLE: std::sync::OnceLock<DenseHankelTable> = std::sync::OnceLock::new();
        let table = TABLE.get_or_init(|| {
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/physics/references/cylinder-hankel-dense.json"
            );
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
        });
        let order = table
            .orders
            .iter()
            .find(|order| order.m == m)
            .unwrap_or_else(|| panic!("no order {m} in the dense table"));
        assert_eq!(order.h1.len(), order.count);
        let position = (kr - order.start) / table.step;
        assert!(
            (-1.0e-9..=(order.count - 1) as f64 + 1.0e-9).contains(&position),
            "kR {kr} lies outside order {m}'s grid"
        );
        let index = (position.max(0.0).floor() as usize).min(order.count - 2);
        let x0 = order.start + index as f64 * table.step;
        let x1 = x0 + table.step;
        let t = (kr - x0) / table.step;
        let hermite = |f0: Phasor, g0: Phasor, f1: Phasor, g1: Phasor| {
            let (t2, t3) = (t * t, t * t * t);
            f0.scale(2.0 * t3 - 3.0 * t2 + 1.0)
                + g0.scale((t3 - 2.0 * t2 + t) * table.step)
                + f1.scale(-2.0 * t3 + 3.0 * t2)
                + g1.scale((t3 - t2) * table.step)
        };
        let second = |x: f64, value: Phasor, slope: Phasor| {
            slope.scale(-1.0 / x) - value.scale(1.0 - f64::from(m * m) / (x * x))
        };
        let (h0, d0) = (tabled(order.h1[index]), tabled(order.dh1[index]));
        let (h1, d1) = (tabled(order.h1[index + 1]), tabled(order.dh1[index + 1]));
        (
            hermite(h0, d0, h1, d1),
            hermite(d0, second(x0, h0, d0), d1, second(x1, h1, d1)),
        )
    }

    /// The dense table, as `wall_hankel` reads it, against the case table's
    /// own evaluations - at the wall, value and derivative, and at every fit
    /// radius the cases hold, `kR` there being `k r`, all of which fall on its
    /// grid - and between its points by Bessel's recurrences across three
    /// orders, `H_{m−1} + H_{m+1} = (2m/x) H_m` and `H_m′ = H_{m−1} − (m/x)
    /// H_m`: each to 1e-9 of the value, the generator's own check.
    #[test]
    fn the_dense_hankel_table_meets_the_case_table() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/physics/references/cylinder-hankel.json"
        );
        let table: HankelTable =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut compared = 0;
        let mut worst = 0.0_f64;
        for case in table.cases.iter().filter(|case| case.m <= 4) {
            let start = f64::from(case.m).max(1.5);
            if case.kr >= start {
                let (value, slope) = wall_hankel(case.m, case.kr);
                for (read, tabled_value) in [(value, case.wall.h1), (slope, case.wall.dh1)] {
                    let error = (read - tabled(tabled_value)).norm() / tabled(case.wall.h1).norm();
                    worst = worst.max(error);
                    compared += 1;
                }
            }
            for (radius, h1) in case.radii.iter().zip(&case.h1) {
                let kr = case.kr * radius;
                if (start..=14.0).contains(&kr) {
                    let error =
                        (wall_hankel(case.m, kr).0 - tabled(*h1)).norm() / tabled(*h1).norm();
                    worst = worst.max(error);
                    compared += 1;
                }
            }
        }
        eprintln!("{compared} values on the grid, worst {worst:.2e}");
        assert!(compared > 100);
        assert!(worst < 1.0e-9, "{worst:e}");

        let mut between = 0.0_f64;
        for m in 1..=3_u32 {
            let start = f64::from(m + 1).max(1.5);
            for step in 0..400 {
                // Off the grid: an irrational stride.
                let x = start + 0.0313 * std::f64::consts::SQRT_2 * f64::from(step);
                if x > 14.0 {
                    break;
                }
                let (below, _) = wall_hankel(m - 1, x);
                let (value, slope) = wall_hankel(m, x);
                let (above, _) = wall_hankel(m + 1, x);
                let scale = value.norm();
                let recurrence = (below + above - value.scale(2.0 * f64::from(m) / x)).norm();
                let derivative = (slope - (below - value.scale(f64::from(m) / x))).norm();
                between = between.max(recurrence.max(derivative) / scale);
            }
        }
        eprintln!("between the points, the recurrences hold to {between:.2e}");
        assert!(between < 1.0e-9, "{between:e}");
    }

    /// The cases PH-06 and PH-07 run: each kR with the angular orders that
    /// propagate at the wall.
    const CIRCULAR_CASES: [(f64, &[u32]); 4] = [
        (0.8, &[0]),
        (2.0, &[0, 1, 2]),
        (5.0, &[0, 1, 2, 4]),
        (10.0, &[0, 1, 2, 4, 8]),
    ];

    /// A tabled complex value as a phasor.
    fn tabled([re, im]: [f64; 2]) -> Phasor {
        Phasor::new(re, im)
    }

    /// The implemented law's `B/A` on a circle of radius one at unit speed,
    /// in the `e^{−iωt}` convention of the 11 October plan. The wall
    /// imposes `∂_n u = iω Y u`, `Y` the law's admittance at `s = −iω` with
    /// the tangential wavenumber `m` of the order on the unit circle, one for
    /// the first-order law and `1 + (6d/7)/s − (8d/7)/(s + d) + (2d/7)/(s +
    /// 2d)`, `d = √(7/8) m`, for the second. Outside an outgoing circle the
    /// field is `A H1 + B H2` and the normal is `r̂`, so `B/A = −(H1′ − iY
    /// H1)/(H2′ − iY H2)`; outside a hole it is `A H2 + B H1` with the normal
    /// `−r̂`, so `B/A = −(H2′ + iY H2)/(H1′ + iY H1)`. The exact exterior
    /// DtN returns nothing, `B = 0`, so `|B/A|` here is the law's distance
    /// from it.
    fn circular_law(case: &HankelCase, second: bool) -> Phasor {
        let omega = case.kr;
        let mut admittance = Phasor::new(1.0, 0.0);
        if second && case.m > 0 {
            let s = Phasor::new(0.0, -omega);
            let d = (7.0f64 / 8.0).sqrt() * f64::from(case.m);
            for (pole, residue) in [(0.0, 6.0 / 7.0), (d, -8.0 / 7.0), (2.0 * d, 2.0 / 7.0)] {
                admittance =
                    admittance + Phasor::new(residue * d, 0.0) / (s + Phasor::new(pole, 0.0));
            }
        }
        let iy = Phasor::new(0.0, 1.0) * admittance;
        let wall = case.wall;
        let (h1, h2, dh1, dh2) = (
            tabled(wall.h1),
            tabled(wall.h2),
            tabled(wall.dh1),
            tabled(wall.dh2),
        );
        let minus = Phasor::new(-1.0, 0.0);
        if case.hole() {
            minus * (dh2 + iy * h2) / (dh1 + iy * h1)
        } else {
            minus * (dh1 - iy * h1) / (dh2 - iy * h2)
        }
    }

    /// `cos mθ` as the formula language writes it, `T_m(x/r)`.
    fn angular_profile(m: u32) -> &'static str {
        match m {
            0 => "1",
            1 => "x / sqrt(x^2 + y^2)",
            2 => "(x^2 - y^2) / (x^2 + y^2)",
            4 => "(x^4 - 6 * x^2 * y^2 + y^4) / (x^2 + y^2)^2",
            8 => "(x^8 - 28 * x^6 * y^2 + 70 * x^4 * y^4 - 28 * x^2 * y^6 + y^8) / (x^2 + y^2)^4",
            _ => panic!("no profile for order {m}"),
        }
    }

    /// The curve tolerance PH-06 and PH-07 mesh their circles at. At the
    /// default 5e-4 the circle becomes the same polygon at every mesh and
    /// its shape held `B/A` 5e-5 to 2e-4 off the law however fine the
    /// elements; at 5e-5 that floor fell under 4e-6, and tighter tolerances
    /// build the same mesh.
    const CIRCULAR_TOLERANCE: f64 = 5.0e-5;

    /// PH-06's or PH-07's scene for `case`, the wall under the first- or
    /// second-order law, meshed at `edge`, its source switched on over two
    /// periods and held for `held` seconds. PH-06: a circle of radius one
    /// whose inner side is the wall, the corners outside it meshed but cut
    /// off, and a source ring a twentieth thick where the order radiates,
    /// `k a` near `m`. PH-07: a hole of radius one whose outer side is the
    /// wall, in a square of side 5.2 whose own sides absorb to first order,
    /// so that it does not ring, and a source ring from 2 to 2.15. The ring
    /// drives `cos mθ`.
    fn circular_scene(
        case: &HankelCase,
        second: bool,
        edge: f64,
        held: f64,
    ) -> Arc<crate::topology_runtime::PreparedTopology> {
        let law = if second {
            FaceBoundaryCondition::SecondOrderOutgoing
        } else {
            FaceBoundaryCondition::Impedance { ratio: 1.0 }
        };
        let period = std::f64::consts::TAU / case.kr;
        let signal = TimeSignal::pulsed(
            [0.0, 1.0, case.kr / std::f64::consts::TAU, 0.0],
            PulseEnvelope::FlatTop {
                duration: 2.0 * held,
                edge: 2.0 * period,
            },
            0.0,
            0.0,
        );
        let inner = (0.75 * f64::from(case.m) / case.kr).clamp(0.25, 0.6);
        circle_scene(
            case.hole(),
            angular_profile(case.m),
            inner,
            law,
            signal,
            edge,
        )
    }

    /// The circular walls' scene: PH-06's circle of radius one whose inner
    /// side is `wall`, with a source ring from `inner` to `inner + 1/20`, or
    /// PH-07's hole of radius one whose outer side is `wall`, in a square of
    /// side 5.2 absorbing to first order, with a ring from 2 to 2.15. The
    /// ring drives `profile` under `signal`; meshed at `edge` and
    /// `CIRCULAR_TOLERANCE`.
    fn circle_scene(
        hole: bool,
        profile: &str,
        inner: f64,
        law: FaceBoundaryCondition,
        signal: TimeSignal,
        edge: f64,
    ) -> Arc<crate::topology_runtime::PreparedTopology> {
        let half = if hole { 2.6 } else { 1.2 };
        let mut builder = Builder::new();
        builder.scene.geometry.domain = DomainRect {
            min_x: -half,
            max_x: half,
            min_y: -half,
            max_y: half,
        };
        builder.scene.outer_boundaries = OuterBoundaryConditions::uniform(if hole {
            OuterBoundaryCondition::FirstOrderOutgoing
        } else {
            OuterBoundaryCondition::Reflecting
        });
        let material = Some((builder.scene.regions[0].material, MaterialFrame::world()));
        let ring = if hole {
            let (_, ring) =
                builder.closed(round_circle(2.15, 64), SpanBehavior::Transmitting, material);
            builder.closed(round_circle(2.0, 64), SpanBehavior::Transmitting, material);
            builder.closed(
                round_circle(1.0, 64),
                SpanBehavior::Separated {
                    left: FaceBoundaryCondition::Reflecting,
                    right: law,
                    coupling: InternalBoundaryCoupling::Independent,
                },
                None,
            );
            ring
        } else {
            builder.closed(
                round_circle(1.0, 64),
                SpanBehavior::Separated {
                    left: law,
                    right: FaceBoundaryCondition::Reflecting,
                    coupling: InternalBoundaryCoupling::Independent,
                },
                material,
            );
            let (_, ring) = builder.closed(
                round_circle(inner + 0.05, 64),
                SpanBehavior::Transmitting,
                material,
            );
            builder.closed(
                round_circle(inner, 64),
                SpanBehavior::Transmitting,
                material,
            );
            ring
        };
        builder.scene.volume_sources.push(VolumeSource {
            region: ring.unwrap(),
            enabled: true,
            profile: ScalarField::formula(profile).unwrap(),
            parameters: vec![],
            signal,
        });
        let mut document = builder.document();
        document.model.source.enabled = false;
        let editor = TopologyEditor::from_document(document.clone()).unwrap();
        let mut runtime = TopologyRuntime::default();
        let token = runtime
            .request(
                1,
                &editor.document,
                editor.compiled_accepted.clone(),
                MeshingOptions {
                    target_edge_length: edge,
                    curve_tolerance: CIRCULAR_TOLERANCE,
                    ..MeshingOptions::default()
                }
                .sized_for_area(4.0 * half * half),
                true,
            )
            .unwrap();
        loop {
            if let Some(result) = runtime.advance(1 << 16) {
                result.unwrap();
                break;
            }
        }
        runtime.commit_ready(token).unwrap()
    }

    /// What one run of a circular wall reads: `B/A` and the share of the
    /// phasors the fit leaves unexplained.
    #[derive(Clone, Copy, Debug)]
    struct CircularReading {
        ratio: Phasor,
        residual: f64,
    }

    /// One run of `case` on meshes of `edge` at `fraction` of the production
    /// step: switched on over two periods and held to steady state, three
    /// seconds and two periods for PH-06, fifteen and two for PH-07, whose
    /// square takes longer to settle; then a Blackman-Harris lock-in at the
    /// drive's frequency over four periods, eight for PH-07, at 64 points on
    /// each of the case's circles, the order's `cos mθ` projection on each,
    /// and those fitted in least squares to `A H + B H′` from the table.
    fn circular_run(case: &HankelCase, second: bool, edge: f64, fraction: f64) -> CircularReading {
        let tau = std::f64::consts::TAU;
        let omega = case.kr;
        let period = tau / omega;
        let (settle, window) = if case.hole() {
            (15.0 + 2.0 * period, 8.0 * period)
        } else {
            (3.0 + 2.0 * period, 4.0 * period)
        };
        let held = 2.0 * period + settle + window;
        let prepared = circular_scene(case, second, edge, held);
        let angles = 64usize;
        let stencils = case
            .radii
            .iter()
            .flat_map(|radius| {
                (0..angles).map(move |j| direction(j as f64 * tau / angles as f64) * *radius)
            })
            .map(|point| {
                QuadraticPointStencil::build_topology(
                    &prepared.mesh,
                    &prepared.operator,
                    &prepared.bundle.plan,
                    prepared.fixed_model(),
                    Point2::default() + point,
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let operator = &prepared.canonical_operator;
        let dt = fraction * prepared.recommended_time_step();
        let steps = (held / dt).ceil() as usize;
        let span = (window / dt).round() as usize;
        let first = steps - span;
        let mut state = CanonicalWaveState::zero(operator, dt).unwrap();
        let mut sums = vec![Phasor::default(); stencils.len()];
        let mut total = 0.0;
        for step in 0..steps {
            state
                .step_with_forcing(operator, &prepared.canonical_forcing)
                .unwrap();
            if step < first {
                continue;
            }
            let x = tau * (step - first) as f64 / span as f64;
            let weight =
                0.35875 - 0.48829 * x.cos() + 0.14128 * (2.0 * x).cos() - 0.01168 * (3.0 * x).cos();
            // e^{−iωt} phasors: u = Re(U e^{−iωt}) gives Σ u e^{iωt} = U/2.
            let (sin, cos) = (omega * (step + 1) as f64 * dt).sin_cos();
            let field = state.primary_field(operator).unwrap();
            for (sum, stencil) in sums.iter_mut().zip(&stencils) {
                let value = stencil
                    .nodes
                    .iter()
                    .zip(stencil.value_weights)
                    .map(|(node, weight)| weight * field[*node as usize])
                    .sum::<f64>();
                *sum = *sum + Phasor::new(cos, sin).scale(weight * value);
            }
            total += weight;
        }
        let projection = |ring: usize| {
            let scale = if case.m == 0 { 1.0 } else { 2.0 } / (angles as f64 * total);
            (0..angles).fold(Phasor::default(), |sum, j| {
                let angle = f64::from(case.m) * j as f64 * tau / angles as f64;
                sum + sums[ring * angles + j].scale(angle.cos() * scale)
            })
        };
        let measured = (0..case.radii.len()).map(projection).collect::<Vec<_>>();
        // Outside an outgoing circle the field is A H1 + B H2; outside a
        // hole A H2 + B H1: A comes from the source, B from the wall.
        let (from_source, from_wall) = if case.hole() {
            (&case.h2, &case.h1)
        } else {
            (&case.h1, &case.h2)
        };
        let (mut a, mut b, mut d) = (0.0, Phasor::default(), 0.0);
        let (mut r0, mut r1) = (Phasor::default(), Phasor::default());
        for ((source, wall), value) in from_source.iter().zip(from_wall).zip(&measured) {
            let (p, q) = (tabled(*source), tabled(*wall));
            a += p.norm().powi(2);
            d += q.norm().powi(2);
            b = b + p.conj() * q;
            r0 = r0 + p.conj() * *value;
            r1 = r1 + q.conj() * *value;
        }
        let determinant = a * d - b.norm().powi(2);
        let incoming = (r0.scale(d) - b * r1).scale(1.0 / determinant);
        let returned = (r1.scale(a) - b.conj() * r0).scale(1.0 / determinant);
        let (mut misfit, mut size) = (0.0, 0.0);
        for ((source, wall), value) in from_source.iter().zip(from_wall).zip(&measured) {
            let fitted = incoming * tabled(*source) + returned * tabled(*wall);
            misfit += (fitted - *value).norm().powi(2);
            size += value.norm().powi(2);
        }
        CircularReading {
            ratio: returned / incoming,
            residual: (misfit / size).sqrt(),
        }
    }

    /// Runs `work` over `tasks` on every core, in the tasks' order.
    fn in_parallel<T: Sync, R: Send>(tasks: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
        let next = std::sync::atomic::AtomicUsize::new(0);
        let results = std::sync::Mutex::new((0..tasks.len()).map(|_| None).collect::<Vec<_>>());
        let threads = std::thread::available_parallelism().map_or(4, |count| count.get());
        std::thread::scope(|scope| {
            for _ in 0..threads.min(tasks.len()) {
                scope.spawn(|| {
                    loop {
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(task) = tasks.get(index) else { break };
                        let result = work(task);
                        results.lock().unwrap()[index] = Some(result);
                    }
                });
            }
        });
        results
            .into_inner()
            .unwrap()
            .into_iter()
            .map(|result| result.unwrap())
            .collect()
    }

    /// One reading of PH-06 or PH-07 as the tests report it.
    #[derive(Clone, Copy, Debug)]
    struct CircularResult {
        kind: &'static str,
        kr: f64,
        m: u32,
        second: bool,
        edge: f64,
        /// `|B/A − law|`, the time step's share removed by extrapolating the
        /// production step and its half, second order, to zero.
        error: f64,
        /// `|B/A|` at the production step and its half, apart: the time
        /// step's share.
        time: f64,
        /// `|B/A|`, read: under one for a passive wall, which a sink must be.
        magnitude: f64,
        residual: f64,
        approximation: f64,
    }

    /// Reads `cases`, each `(kind, kR, m, second, edge)`, in parallel at the
    /// production step and, with `extrapolate`, half of it, which takes the
    /// time step's share out.
    fn circular_study(
        cases: &[(&'static str, f64, u32, bool, f64)],
        extrapolate: bool,
    ) -> Vec<CircularResult> {
        let fractions: &[f64] = if extrapolate { &[1.0, 0.5] } else { &[1.0] };
        let tasks = cases
            .iter()
            .flat_map(|case| fractions.iter().map(|fraction| (*case, *fraction)))
            .collect::<Vec<_>>();
        let readings = in_parallel(&tasks, |((kind, kr, m, second, edge), fraction)| {
            circular_run(&hankel_case(kind, *kr, *m), *second, *edge, *fraction)
        });
        cases
            .iter()
            .zip(readings.chunks(fractions.len()))
            .map(|(&(kind, kr, m, second, edge), pair)| {
                let case = hankel_case(kind, kr, m);
                let law = circular_law(&case, second);
                let (full, half) = (pair[0], *pair.last().unwrap());
                let extrapolated = half.ratio + (half.ratio - full.ratio).scale(1.0 / 3.0);
                let result = CircularResult {
                    kind,
                    kr,
                    m,
                    second,
                    edge,
                    error: (extrapolated - law).norm(),
                    time: (full.ratio - half.ratio).norm(),
                    magnitude: full.ratio.norm(),
                    residual: full.residual.max(half.residual),
                    approximation: law.norm(),
                };
                eprintln!(
                    "{kind} kR {kr} m {m} {} edge {edge}: B/A {:+.6}{:+.6}i, law {:+.6}{:+.6}i, E_discretization {:.3e} (time step's share {:.1e}), E_approximation {:.4}, fit {:.1e}",
                    if second { "second" } else { "first" },
                    extrapolated.re,
                    extrapolated.im,
                    law.re,
                    law.im,
                    result.error,
                    result.time,
                    result.approximation,
                    result.residual
                );
                result
            })
            .collect()
    }

    /// Holds a study's results to what was measured on 2026-10-15. Each case
    /// within `5e-5 + 2e-3 |R_law|` of its law on its finest mesh: the
    /// circle's shape, fixed by `CIRCULAR_TOLERANCE` and not by the mesh,
    /// moves `B/A` by more where the wall returns more, 6e-5 to 2.4e-4 at
    /// the near-grazing orders whose `|R_law|` reaches 0.2 and more, while
    /// the rest read 2e-6 to 4.6e-5. Falling fivefold or more from 0.1 to
    /// 0.05 at kR 10 for orders 0 to 2, measured 7.5 to 13.6: at kR 5 and
    /// under the error lies at the method's floor, a few parts in a
    /// million, on the coarsest mesh already, and order 4 and up come near
    /// it at kR 10, the hole's falling 2.9 and 3.2 times to 3.7e-5 and
    /// 1.6e-5. The fit within 5e-4,
    /// measured 3.6e-4 at worst. And every hole passive, `|B/A| < 1`, the
    /// power flowing into it.
    fn hold_circular(results: &[CircularResult]) {
        for result in results {
            let kind = result.kind;
            let case = format!(
                "{kind} kR {} m {} {} edge {}",
                result.kr,
                result.m,
                if result.second { "second" } else { "first" },
                result.edge
            );
            assert!(result.residual < 5.0e-4, "{case}: fit {}", result.residual);
            if kind == "hole" {
                assert!(result.magnitude < 1.0, "{case}: |B/A| {}", result.magnitude);
            }
            let same = results.iter().filter(|other| {
                other.kind == kind
                    && other.kr == result.kr
                    && other.m == result.m
                    && other.second == result.second
            });
            let finest = same
                .clone()
                .map(|other| other.edge)
                .fold(f64::INFINITY, f64::min);
            if result.edge == finest {
                let bound = 5.0e-5 + 2.0e-3 * result.approximation;
                assert!(
                    result.error < bound,
                    "{case}: E_discretization {} beyond {bound}",
                    result.error
                );
            }
            if result.kr == 10.0 && result.m <= 2 && result.edge == 0.05 {
                let coarse = same.clone().find(|other| other.edge == 0.1).unwrap();
                assert!(
                    coarse.error / result.error > 5.0,
                    "{case}: E_discretization {} at 0.1, {} at 0.05",
                    coarse.error,
                    result.error
                );
            }
        }
    }

    /// Every case of `kind`: kR 5 and 10 on edges 0.1, 0.07 and 0.05, kR 0.8
    /// and 2, whose error already lies under the method's floor on the
    /// coarsest mesh, on 0.1 alone.
    fn circular_sweep(kind: &'static str) -> Vec<CircularResult> {
        let mut cases = Vec::new();
        for (kr, orders) in CIRCULAR_CASES {
            let edges: &[f64] = if kr >= 5.0 {
                &[0.1, 0.07, 0.05]
            } else {
                &[0.1]
            };
            for m in orders {
                for second in [false, true] {
                    for edge in edges {
                        cases.push((kind, kr, *m, second, *edge));
                    }
                }
            }
        }
        circular_study(&cases, true)
    }

    /// PH-06 and PH-07 in the gate: order 2 at kR 5 on the coarsest mesh, a
    /// hole under either law and an outgoing circle under the first-order
    /// one, at the production step alone, the time step's share left in;
    /// see `circular_run` for the measurement.
    #[test]
    fn circular_walls_meet_their_law_on_the_coarse_mesh() {
        let results = circular_study(
            &[
                ("hole", 5.0, 2, false, 0.1),
                ("hole", 5.0, 2, true, 0.1),
                ("outgoing", 5.0, 2, false, 0.1),
            ],
            false,
        );
        hold_circular(&results);
    }

    /// PH-06, outgoing radiation through a circular outer boundary.
    #[test]
    #[ignore = "nightly: PH-06's sweep"]
    fn outgoing_circles_converge_to_their_law() {
        let results = circular_sweep("outgoing");
        hold_circular(&results);
    }

    /// PH-07, inward radiation into an absorbing circular hole.
    #[test]
    #[ignore = "nightly: PH-07's sweep"]
    fn absorbing_holes_converge_to_their_law() {
        let results = circular_sweep("hole");
        hold_circular(&results);
    }

    /// A complex amplitude, `e^{iωt}` in time, for PH-04's phasors.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    struct Phasor {
        re: f64,
        im: f64,
    }

    impl Phasor {
        const fn new(re: f64, im: f64) -> Self {
            Self { re, im }
        }

        fn norm(self) -> f64 {
            self.re.hypot(self.im)
        }

        fn conj(self) -> Self {
            Self::new(self.re, -self.im)
        }

        fn scale(self, factor: f64) -> Self {
            Self::new(self.re * factor, self.im * factor)
        }
    }

    impl std::ops::Add for Phasor {
        type Output = Self;
        fn add(self, other: Self) -> Self {
            Self::new(self.re + other.re, self.im + other.im)
        }
    }

    impl std::ops::Sub for Phasor {
        type Output = Self;
        fn sub(self, other: Self) -> Self {
            Self::new(self.re - other.re, self.im - other.im)
        }
    }

    impl std::ops::Mul for Phasor {
        type Output = Self;
        fn mul(self, other: Self) -> Self {
            Self::new(
                self.re * other.re - self.im * other.im,
                self.re * other.im + self.im * other.re,
            )
        }
    }

    impl std::ops::Div for Phasor {
        type Output = Self;
        fn div(self, other: Self) -> Self {
            let denominator = other.re * other.re + other.im * other.im;
            (self * other.conj()).scale(1.0 / denominator)
        }
    }

    /// The implemented law's reflection of the primary field, written from
    /// the boundary spike, never from the solver: a wall of admittance `Y`
    /// met at `cos θ` from its normal returns `R = (cos θ − Y)/(cos θ + Y)`.
    /// The first-order wall's `Y` is one; the second-order wall's is the
    /// passive law `Y(s) = 1 + (6d/7)/s − (8d/7)/(s + d) + (2d/7)/(s + 2d)`
    /// at `s = iω`, `d = √(7/8) k` for the tangential wavenumber `k`. The
    /// exact half-space admittance is `cos θ` itself, so the exact `R` is
    /// zero at every propagating angle, and the law's `|R|` is its distance
    /// from exact transparency.
    fn planar_law(cos: f64, omega: f64, tangential: f64, second: bool) -> Phasor {
        let mut admittance = Phasor::new(1.0, 0.0);
        if second && tangential > 0.0 {
            let d = (7.0f64 / 8.0).sqrt() * tangential;
            for (pole, residue) in [(0.0, 6.0 / 7.0), (d, -8.0 / 7.0), (2.0 * d, 2.0 / 7.0)] {
                admittance = admittance + Phasor::new(residue * d, 0.0) / Phasor::new(pole, omega);
            }
        }
        let cos = Phasor::new(cos, 0.0);
        (cos - admittance) / (cos + admittance)
    }

    /// One of PH-04's angles, from the normal of a wall at the right end of a
    /// reflecting channel one unit high: the channel mode `cos(nπ(y + ½))`
    /// that meets the wall at that angle, `sin θ = nπ/ω`, the constant for
    /// 0°; and what the angle asks of the measurement. A mode near its
    /// cutoff moves slowly and a drive switched on excites the cutoff
    /// itself, which lingers, so 75°, 3.5% above cutoff, takes a minute's
    /// switch-on and two minutes of lock-in, and a gentle sponge: a steep
    /// one reflects a slow mode, and a strength of 4 over 6.5 units read
    /// 75°'s first-order wall at −0.523 for its law's −0.589.
    #[derive(Clone, Copy, Debug)]
    struct PlanarAngle {
        degrees: f64,
        mode: u32,
        /// The sponge's damping rate at the channel's far end, and the
        /// channel's length behind the wall's side, past the launcher.
        sponge: f64,
        length: f64,
        /// Seconds of switch-on, of settling, and of lock-in.
        ramp: f64,
        settle: f64,
        window: f64,
    }

    const PLANAR_ANGLES: [PlanarAngle; 3] = [
        PlanarAngle {
            degrees: 0.0,
            mode: 0,
            sponge: 2.0,
            length: 8.0,
            ramp: 4.0,
            settle: 4.0,
            window: 10.0,
        },
        PlanarAngle {
            degrees: 45.0,
            mode: 2,
            sponge: 2.0,
            length: 8.0,
            ramp: 8.0,
            settle: 6.0,
            window: 20.0,
        },
        PlanarAngle {
            degrees: 75.0,
            mode: 2,
            sponge: 0.4,
            length: 14.0,
            ramp: 60.0,
            settle: 20.0,
            window: 120.0,
        },
    ];

    impl PlanarAngle {
        fn tangential(self) -> f64 {
            f64::from(self.mode) * std::f64::consts::PI
        }

        /// The drive's frequency: 1.5 Hz at normal incidence, and otherwise
        /// the one that puts the mode at the angle.
        fn hz(self) -> f64 {
            if self.mode == 0 {
                1.5
            } else {
                self.tangential() / (std::f64::consts::TAU * self.degrees.to_radians().sin())
            }
        }

        fn omega(self) -> f64 {
            std::f64::consts::TAU * self.hz()
        }

        fn cos(self) -> f64 {
            self.degrees.to_radians().cos()
        }

        /// A quarter of the wavelength along the channel: moving the sponge
        /// by it turns its reflection's round trip by half a period.
        fn quarter(self) -> f64 {
            0.5 * std::f64::consts::PI / (self.omega() * self.cos())
        }

        fn profile(self) -> String {
            if self.mode == 0 {
                "1".into()
            } else {
                format!("cos({} * pi * (y + 0.5))", self.mode)
            }
        }

        fn law(self, second: bool) -> Phasor {
            planar_law(self.cos(), self.omega(), self.tangential(), second)
        }
    }

    /// The reflecting channel `[−L − shift, 1] × [−½, ½]` with `wall` on its
    /// right end and its left end second-order outgoing, a launcher strip at
    /// `x = −0.7` driving `angle`'s mode at its frequency, switched on over
    /// `angle.ramp` seconds and held, and a sponge: the medium's damping
    /// rising smoothly from nothing at `x = −1.5 − shift` to `angle.sponge`
    /// at the far end, so what goes left is absorbed before it can return.
    fn sponge_channel(
        angle: PlanarAngle,
        wall: OuterBoundaryCondition,
        edge: f64,
        shift: f64,
        held: f64,
    ) -> Arc<crate::topology_runtime::PreparedTopology> {
        let left = angle.length + shift;
        let mut builder = Builder::new();
        builder.scene.geometry.domain = DomainRect {
            min_x: -left,
            max_x: 1.0,
            min_y: -0.5,
            max_y: 0.5,
        };
        builder.scene.face_assignments[0].anchor = FaceAnchor::Outer {
            side: OuterSide::Bottom,
            fraction: left / (left + 1.0),
        };
        builder.scene.materials[0].damping = ScalarField::formula(format!(
            "{} * smoothstep({}, {left}, -x)",
            angle.sponge,
            1.5 + shift
        ))
        .unwrap();
        let mut boundaries = channel();
        boundaries.sides[OuterSide::Right.index()] = wall;
        builder.scene.outer_boundaries = boundaries;
        builder.launcher(-0.7, INTERFACE_HZ, 1.0);
        let launcher = builder.scene.volume_sources.last_mut().unwrap();
        launcher.signal = TimeSignal::pulsed(
            [0.0, 1.0, angle.hz(), 0.0],
            PulseEnvelope::FlatTop {
                duration: 2.0 * held,
                edge: angle.ramp,
            },
            0.0,
            0.0,
        );
        launcher.profile = ScalarField::formula(angle.profile()).unwrap();
        let mut document = builder.document();
        document.model.source.enabled = false;
        prepare(&document, edge)
    }

    /// The walls' reflections at one angle, mesh and step, and how far the
    /// nine points' own estimates stray from them.
    #[derive(Clone, Copy, Debug)]
    struct PlanarReading {
        first: Phasor,
        second: Phasor,
        spread: f64,
    }

    /// PH-04's measurement at `angle` on meshes of `edge` and `fraction` of
    /// the production step. Four runs share the channel, the drive and the
    /// sponge, and differ in the right wall: first-order, second-order,
    /// reflecting and zero-pinned. Held to steady state, each run's field at
    /// nine points beside the wall is read by a Blackman-Harris lock-in at
    /// the drive's frequency over the last `angle.window` seconds. The
    /// reflecting and pinned runs return the mode whole, `R = ±1`, so their
    /// mean is the incident field and their half-difference the reflection
    /// of a unit wall, and a wall under test returns `R` where `U = U_inc +
    /// R U_unit`, fitted in least squares over the points. The sponge's own
    /// small reflection `C` would bias that by `C(1 − R²)`, so the whole
    /// measurement runs twice with the sponge a quarter wavelength further
    /// away, which turns `C` over, and the two are averaged.
    fn planar_reflection(angle: PlanarAngle, edge: f64, fraction: f64) -> PlanarReading {
        let walls = [
            OuterBoundaryCondition::FirstOrderOutgoing,
            OuterBoundaryCondition::SecondOrderOutgoing,
            OuterBoundaryCondition::Reflecting,
            OuterBoundaryCondition::Dirichlet {
                signal: TimeSignal::ZERO,
            },
        ];
        // A quarter of the mode's transverse period apart, where it is 0.7
        // of its peak; the constant reads alike anywhere.
        let quarter = 0.25 / f64::from(angle.mode.max(2));
        let points = [0.3, 0.5, 0.7]
            .into_iter()
            .flat_map(|x| [quarter, -quarter, 3.0 * quarter].map(|y| Point2::new(x, y)))
            .collect::<Vec<_>>();
        // Long enough to switch on, for the mode to reach the wall and come
        // back past the points, to settle and to be read.
        let held = angle.ramp + 2.4 / angle.cos() + angle.settle + angle.window;
        let omega = angle.omega();
        let readings = [0.0, angle.quarter()].map(|shift| {
            let prepared = walls.map(|wall| sponge_channel(angle, wall, edge, shift, held));
            let dt = fraction
                * prepared
                    .iter()
                    .map(|prepared| prepared.recommended_time_step())
                    .fold(f64::INFINITY, f64::min);
            let steps = (held / dt).ceil() as usize;
            let traces = std::thread::scope(|scope| {
                prepared
                    .iter()
                    .map(|prepared| {
                        let points = &points;
                        scope.spawn(move || {
                            wall_traces(prepared, dt, steps, points, CanonicalWallStep::default())
                        })
                    })
                    .collect::<Vec<_>>()
                    .into_iter()
                    .map(|handle| handle.join().unwrap())
                    .collect::<Vec<_>>()
            });
            let span = (angle.window / dt).round() as usize;
            let lock_in = |trace: &[f64]| {
                let first = steps - span;
                let (mut sum, mut total) = (Phasor::default(), 0.0);
                for (step, value) in trace.iter().enumerate().skip(first) {
                    let x = std::f64::consts::TAU * (step - first) as f64 / span as f64;
                    let weight = 0.35875 - 0.48829 * x.cos() + 0.14128 * (2.0 * x).cos()
                        - 0.01168 * (3.0 * x).cos();
                    let phase = omega * (step + 1) as f64 * dt;
                    sum = sum + Phasor::new(phase.cos(), -phase.sin()).scale(weight * value);
                    total += weight;
                }
                sum.scale(1.0 / total)
            };
            let phasors = traces
                .iter()
                .map(|run| run.iter().map(|trace| lock_in(trace)).collect::<Vec<_>>())
                .collect::<Vec<_>>();
            let reflection = |wall: usize| {
                let (mut numerator, mut denominator) = (Phasor::default(), 0.0);
                let mut estimates = Vec::new();
                for ((open, pinned), tested) in
                    phasors[2].iter().zip(&phasors[3]).zip(&phasors[wall])
                {
                    let incident = (*open + *pinned).scale(0.5);
                    let unit = (*open - *pinned).scale(0.5);
                    let returned = *tested - incident;
                    estimates.push(returned / unit);
                    numerator = numerator + returned * unit.conj();
                    denominator += unit.norm().powi(2);
                }
                let fitted = numerator.scale(1.0 / denominator);
                let spread = estimates
                    .iter()
                    .map(|estimate| (*estimate - fitted).norm())
                    .fold(0.0, f64::max);
                (fitted, spread)
            };
            let (first, first_spread) = reflection(0);
            let (second, second_spread) = reflection(1);
            PlanarReading {
                first,
                second,
                spread: first_spread.max(second_spread),
            }
        });
        PlanarReading {
            first: (readings[0].first + readings[1].first).scale(0.5),
            second: (readings[0].second + readings[1].second).scale(0.5),
            spread: readings[0].spread.max(readings[1].spread),
        }
    }

    /// Prints a reading against the law, and returns the two walls' errors.
    fn planar_report(
        angle: PlanarAngle,
        edge: f64,
        fraction: f64,
        reading: PlanarReading,
    ) -> [f64; 2] {
        [false, true].map(|second| {
            let read = if second { reading.second } else { reading.first };
            let law = angle.law(second);
            let error = (read - law).norm();
            eprintln!(
                "{}° {} order, edge {edge} at {fraction} of the step: R {:+.6}{:+.6}i, law {:+.6}{:+.6}i, \
                 E_discretization {error:.3e}, E_approximation {:.4}, points within {:.1e}",
                angle.degrees,
                if second { "second" } else { "first" },
                read.re,
                read.im,
                law.re,
                law.im,
                law.norm(),
                reading.spread
            );
            error
        })
    }

    /// PH-04 in the gate: the first- and second-order walls at 0° and 45°
    /// on the coarsest mesh of the nightly study, read against their law;
    /// see `planar_reflection` for the measurement and
    /// `planar_reflection_converges_to_its_law_at_three_angles` for the
    /// study.
    #[test]
    fn planar_reflection_meets_its_law_at_0_and_45_degrees() {
        for angle in &PLANAR_ANGLES[..2] {
            let reading = planar_reflection(*angle, 0.1, 1.0);
            let errors = planar_report(*angle, 0.1, 1.0, reading);
            for (error, order) in errors.into_iter().zip(["first", "second"]) {
                assert!(
                    error < 1.0e-3,
                    "{}° {order} order: E_discretization {error:e}",
                    angle.degrees
                );
            }
            assert!(
                reading.spread < 1.0e-3,
                "{}°: {}",
                angle.degrees,
                reading.spread
            );
        }
    }

    /// PH-04, planar reflection against the implemented law
    /// (`docs/spikes/funfern-testing-campaign-plan.md`): the first- and
    /// second-order walls at 0°, 45° and 75° on meshes of edge 0.1, 0.07 and
    /// 0.05 at the production step, and at 45° on 0.07 at 1, ½ and ¼ of it.
    ///
    /// Measured on the M1 Max, `|R − R_law|` at 0.1, 0.07 and 0.05: 0°
    /// 6.0e-4, 3.6e-4 and 1.8e-4 for both walls, which agree there; 45°
    /// 5.7e-4, 2.3e-4 and 1.4e-4 first-order and 4.0e-4, 1.7e-4 and 1.0e-4
    /// second-order; 75° 2.5e-4, 9.0e-5 and 4.9e-5, and 3.7e-4, 1.6e-4 and
    /// 8.0e-5. Each falls at every level, 3.3 to 5.1 times over the halving,
    /// the trapezoid wall step's `(ω dt)²/24` with the mesh's own error
    /// beside it, the meshes not nested; the nine points agree to 2.1e-5 at
    /// 0.05. In time, at 45° on 0.07, successive differences fall 5.6 and 5.4
    /// times a halving of the step. The law's own distance from exact
    /// transparency, `|R_law|`, is 0, 0.17 and 0.08, and 0.59 and 0.48.
    /// About 57 minutes, 75° at 0.05 alone 27; run nightly.
    #[test]
    #[ignore = "nightly: PH-04's three-angle study, about 50 minutes"]
    fn planar_reflection_converges_to_its_law_at_three_angles() {
        let edges = [0.1, 0.07, 0.05];
        for angle in PLANAR_ANGLES {
            let readings = edges.map(|edge| {
                let reading = planar_reflection(angle, edge, 1.0);
                (planar_report(angle, edge, 1.0, reading), reading.spread)
            });
            assert!(
                readings[2].1 < 5.0e-5,
                "{}°: the points stray {} at 0.05",
                angle.degrees,
                readings[2].1
            );
            let errors = readings.map(|(errors, _)| errors);
            for (wall, order) in ["first", "second"].into_iter().enumerate() {
                let series = errors.map(|errors| errors[wall]);
                let case = format!("{}° {order} order", angle.degrees);
                assert!(
                    series[1] < series[0] && series[2] < series[1],
                    "{case}: E_discretization {series:?}"
                );
                assert!(
                    series[0] / series[2] > 2.5 && series[2] < 2.5e-4,
                    "{case}: E_discretization {series:?}"
                );
                eprintln!(
                    "{}° {order} order: E_discretization {}",
                    angle.degrees,
                    series
                        .iter()
                        .map(|error| format!("{error:.3e}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
        let angle = PLANAR_ANGLES[1];
        let readings = [1.0, 0.5, 0.25].map(|fraction| {
            let reading = planar_reflection(angle, 0.07, fraction);
            planar_report(angle, 0.07, fraction, reading);
            reading
        });
        for second in [false, true] {
            let read = |reading: &PlanarReading| {
                if second {
                    reading.second
                } else {
                    reading.first
                }
            };
            let coarse = (read(&readings[0]) - read(&readings[1])).norm();
            let fine = (read(&readings[1]) - read(&readings[2])).norm();
            eprintln!(
                "45° {} order in time: successive differences {coarse:.3e} and {fine:.3e}, ratio {:.3}",
                if second { "second" } else { "first" },
                coarse / fine
            );
            assert!(coarse / fine > 3.5, "45° in time: {coarse:e} and {fine:e}");
        }
    }

    /// The default scene's box with every side reflecting, meshed at `edge`
    /// on the application's own path, the source off.
    fn reflecting_box(edge: f64) -> Arc<crate::topology_runtime::PreparedTopology> {
        let mut builder = Builder::new();
        builder.scene.outer_boundaries =
            OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting);
        let mut document = builder.document();
        document.model.source.enabled = false;
        prepare(&document, edge)
    }

    /// The angular frequency of a sampled sinusoid, from the recurrence
    /// `s(n+1) + s(n−1) = 2 cos(ω dt) s(n)` it satisfies exactly, fitted in
    /// least squares over the series.
    fn sampled_frequency(series: &[f64], dt: f64) -> f64 {
        let (mut numerator, mut denominator) = (0.0, 0.0);
        for window in series.windows(3) {
            numerator += window[1] * (window[0] + window[2]);
            denominator += 2.0 * window[1] * window[1];
        }
        (numerator / denominator).acos() / dt
    }

    /// What a box-mode run reads.
    struct CavityRun {
        /// The frequency the stepped mode has.
        stepped: f64,
        /// The mode's amplitude at the end over its start.
        envelope: f64,
        /// The energy's largest relative excursion from its start.
        excursion: f64,
    }

    /// Runs the box mode `cos(k(x+1)) cos(k(y+1))` from rest on `prepared`
    /// with `integrator` at `dt` for `periods` of the exact mode and reads
    /// its frequency from the mode's mass-weighted projection.
    fn cavity_run(
        prepared: &crate::topology_runtime::PreparedTopology,
        integrator: CanonicalIntegrator,
        dt: f64,
        k: f64,
        periods: f64,
    ) -> CavityRun {
        let operator = &prepared.canonical_operator;
        let shape = operator
            .node_points()
            .iter()
            .map(|point| (k * (point.x + 1.0)).cos() * (k * (point.y + 1.0)).cos())
            .collect::<Vec<_>>();
        let mut state = CanonicalWaveState::from_primary_and_potential(
            operator,
            dt,
            &shape,
            &vec![0.0; shape.len()],
        )
        .unwrap()
        .with_integrator(integrator);
        let weights = shape
            .iter()
            .zip(operator.primary_mass())
            .map(|(shape, mass)| shape * mass)
            .collect::<Vec<_>>();
        let norm = weights.iter().zip(&shape).map(|(w, s)| w * s).sum::<f64>();
        let project = |state: &CanonicalWaveState| {
            let field = state.primary_field(operator).unwrap();
            weights.iter().zip(&field).map(|(w, u)| w * u).sum::<f64>() / norm
        };
        let steps = (periods * std::f64::consts::TAU / (k * 2.0f64.sqrt()) / dt).ceil() as usize;
        let start = state.energy(operator).unwrap();
        let mut series = Vec::with_capacity(steps + 1);
        series.push(project(&state));
        let mut excursion: f64 = 0.0;
        for _ in 0..steps {
            state.step(operator).unwrap();
            series.push(project(&state));
            excursion = excursion.max((state.energy(operator).unwrap() / start - 1.0).abs());
        }
        let stepped = sampled_frequency(&series, dt);
        // A sinusoid's amplitude from three samples about one.
        let amplitude = |window: &[f64]| {
            (window[1].powi(2) + ((window[2] - window[0]) / (2.0 * (stepped * dt).sin())).powi(2))
                .sqrt()
        };
        CavityRun {
            stepped,
            envelope: amplitude(&series[series.len() - 3..]) / amplitude(&series[..3]),
            excursion,
        }
    }

    /// The semidiscrete frequency a stepped one came from, by the
    /// integrator's exact dispersion relation on a linear mode: leapfrog's
    /// `2 sin(ω dt/2)/dt`, and for the fourth-order step
    /// `cos(ω dt) = 1 − x²/2 + x⁴/24` at `x = ω_h dt`.
    fn semidiscrete_frequency(integrator: CanonicalIntegrator, stepped: f64, dt: f64) -> f64 {
        match integrator {
            CanonicalIntegrator::Leapfrog => 2.0 * (0.5 * stepped * dt).sin() / dt,
            CanonicalIntegrator::FourthOrder => {
                let x_squared = 6.0 - (36.0 - 24.0 * (1.0 - (stepped * dt).cos())).sqrt();
                x_squared.sqrt() / dt
            }
        }
    }

    /// The box mode `cos(3π(x+1)/2) cos(3π(y+1)/2)` on the production path -
    /// the application's mesh of the reflecting box, its fourth-order step
    /// at its recommended size - with space and time told apart exactly. On
    /// a linear mode each integrator has an exact dispersion relation,
    /// leapfrog's `2 sin(ω dt/2)/dt = ω_h` and the fourth-order step's
    /// `cos(ω dt) = 1 − x²/2 + x⁴/24` at `x = ω_h dt`, so the semidiscrete
    /// frequency `ω_h` is read from each run without extrapolation, and the
    /// two must agree: at edge 0.08 they do to 1e-8, over three step sizes
    /// each. Their stepped frequencies then miss `ω_h` as their laws say,
    /// leapfrog by `ω_h³dt²/24` to 0.1% and the fourth-order step by
    /// `−ω_h⁵dt⁴/720` to 0.2%, two orders of magnitude apart at the
    /// recommended step. In space `ω_h` misses the exact `3π/√2` by 3.7e-5
    /// at edge 0.16, 9.9e-7 at 0.08 and 1.1e-7 at 0.04: 339 times down over
    /// two halvings, the fourth order of the element. The mode's amplitude
    /// over ten periods holds to 1e-6 at the coarsest mesh and 2e-10 at the
    /// finest, and the energy wobbles by a quarter of `(ω dt)²` about its
    /// start, four times less each halving, and never drifts.
    #[test]
    fn a_box_mode_on_the_production_path_keeps_its_frequency_in_space_and_time() {
        let k = 1.5 * std::f64::consts::PI;
        let exact = k * 2.0f64.sqrt();
        let periods = 10.0;

        // Time, at one mesh.
        let prepared = reflecting_box(0.08);
        let largest = prepared.canonical_operator.maximum_time_step();
        let mut semidiscrete = Vec::new();
        for integrator in [
            CanonicalIntegrator::Leapfrog,
            CanonicalIntegrator::FourthOrder,
        ] {
            let mut excursions = Vec::new();
            for fraction in [1.0, 0.5, 0.25] {
                let dt = fraction * integrator.recommended() * largest;
                let run = cavity_run(&prepared, integrator, dt, k, periods);
                let inverted = semidiscrete_frequency(integrator, run.stepped, dt);
                let law = match integrator {
                    CanonicalIntegrator::Leapfrog => inverted.powi(3) * dt * dt / 24.0,
                    CanonicalIntegrator::FourthOrder => -inverted.powi(5) * dt.powi(4) / 720.0,
                };
                let case = format!("{integrator:?} at {fraction} of its step");
                eprintln!(
                    "{case}: ω_h {inverted:.10}, stepped − ω_h {:.3e} against {law:.3e}, envelope − 1 {:.1e}, energy excursion {:.2e} of (ω dt)² {:.2e}",
                    run.stepped - inverted,
                    run.envelope - 1.0,
                    run.excursion,
                    (run.stepped * dt).powi(2)
                );
                assert!(
                    (run.stepped - inverted - law).abs() < 1.0e-2 * law.abs() + 1.0e-10,
                    "{case}: {} against {law}",
                    run.stepped - inverted
                );
                assert!(
                    (run.envelope - 1.0).abs() < 1.0e-6,
                    "{case}: {}",
                    run.envelope
                );
                assert!(run.excursion > 0.0 && run.excursion < 0.5 * (run.stepped * dt).powi(2));
                excursions.push(run.excursion);
                semidiscrete.push(inverted);
            }
            assert!(excursions[0] / excursions[1] > 3.5 && excursions[1] / excursions[2] > 3.5);
        }
        let spread = semidiscrete.iter().copied().fold(f64::MIN, f64::max)
            - semidiscrete.iter().copied().fold(f64::MAX, f64::min);
        assert!(
            spread < 1.0e-8 * exact,
            "ω_h spread {spread:.3e} over {semidiscrete:?}"
        );

        // Space, at the production step.
        let mut errors = Vec::new();
        for edge in [0.16, 0.08, 0.04] {
            let prepared = reflecting_box(edge);
            let dt = prepared.recommended_time_step();
            let run = cavity_run(&prepared, CanonicalIntegrator::default(), dt, k, periods);
            let inverted = semidiscrete_frequency(CanonicalIntegrator::default(), run.stepped, dt);
            eprintln!(
                "edge {edge}: ω_h {inverted:.10} off {:.3e}, envelope − 1 {:.1e}",
                inverted / exact - 1.0,
                run.envelope - 1.0
            );
            assert!(
                (run.envelope - 1.0).abs() < 1.0e-5,
                "edge {edge}: {}",
                run.envelope
            );
            errors.push((inverted / exact - 1.0).abs());
        }
        assert!(
            errors[0] / errors[1] > 6.0 && errors[1] / errors[2] > 6.0,
            "{errors:?}"
        );
        assert!(errors[0] / errors[2] > 200.0, "{errors:?}");
        assert!(errors[2] < 1.0e-6, "{errors:?}");
    }

    /// A mode `cos(mπ(x + w/2)/w) cos(nπ(y + h/2)/h)` of a reflecting
    /// `w × h` box centred on the origin, a sum of the four plane waves
    /// `(±mπ/w, ±nπ/h)`. PH-01's box is 2 × 1.4, not square, so that no mode
    /// it runs shares its frequency with another: on the square (1,0) and
    /// (0,1), (1,2) and (2,1), (2,3) and (3,2) are pairs an unstructured mesh
    /// mixes.
    #[derive(Clone, Copy, Debug)]
    struct BoxMode {
        m: u32,
        n: u32,
        width: f64,
        height: f64,
    }

    impl BoxMode {
        const fn ph01(m: u32, n: u32) -> Self {
            Self {
                m,
                n,
                width: 2.0,
                height: 1.4,
            }
        }

        fn wavenumbers(self) -> Point2 {
            Point2::new(
                f64::from(self.m) * std::f64::consts::PI / self.width,
                f64::from(self.n) * std::f64::consts::PI / self.height,
            )
        }

        fn wavenumber(self) -> f64 {
            self.wavenumbers().norm()
        }

        fn shape(self, point: Point2) -> f64 {
            let k = self.wavenumbers();
            (k.x * (point.x + 0.5 * self.width)).cos() * (k.y * (point.y + 0.5 * self.height)).cos()
        }

        fn gradient(self, point: Point2) -> Point2 {
            let k = self.wavenumbers();
            let (x, y) = (
                k.x * (point.x + 0.5 * self.width),
                k.y * (point.y + 0.5 * self.height),
            );
            Point2::new(-k.x * x.sin() * y.cos(), -k.y * x.cos() * y.sin())
        }
    }

    /// The modes PH-01 runs one at a time.
    const BOX_MODES: [BoxMode; 4] = [
        BoxMode::ph01(1, 0),
        BoxMode::ph01(0, 1),
        BoxMode::ph01(1, 2),
        BoxMode::ph01(2, 3),
    ];

    /// The phase PH-01's modes start at, where Q and b are both nonzero.
    const BOX_PHASE: f64 = std::f64::consts::FRAC_PI_3;

    /// A box's uniform medium: the physics model, its material's mass and
    /// stiffness slots, and its axis ratio `r`, the stiffness `r` times
    /// itself along x and over `r` along y in the world frame.
    #[derive(Clone, Copy, Debug)]
    struct BoxMedium {
        physics: PhysicsModel,
        mass: f64,
        stiffness: f64,
        axis_ratio: f64,
    }

    impl BoxMedium {
        /// PH-01's medium: mass slot 2, stiffness slot 0.5.
        const fn ph01(physics: PhysicsModel) -> Self {
            Self {
                physics,
                mass: 2.0,
                stiffness: 0.5,
                axis_ratio: 1.0,
            }
        }

        /// The wave speed each model's own law gives the slots. Mechanical
        /// runs at `√(k/ρ)` and maps to the TE core; TM and TE at
        /// `1/√(εμ)`, TM reading ε from the mass slot and TE from the
        /// stiffness slot: PH-01's medium runs Mechanical at 0.5 and the
        /// electromagnetic models at 1.
        fn speed(self) -> f64 {
            match self.physics {
                PhysicsModel::Mechanical => (self.stiffness / self.mass).sqrt(),
                PhysicsModel::Electromagnetic { .. } => 1.0 / (self.mass * self.stiffness).sqrt(),
            }
        }

        /// The orientation `η` of `ḃ = η R∇u`, `R(x, y) = (−y, x)`: in TM b
        /// is `B⊥` and `∂B/∂t = −∇×E` is `R∇E_z`; in TE b is `D⊥` and
        /// `∂D/∂t = ∇×H` is `−R∇H_z`; Mechanical is TE's.
        fn orientation(self) -> f64 {
            match self.physics {
                PhysicsModel::Electromagnetic {
                    polarization: ElectromagneticPolarization::Tm,
                } => 1.0,
                _ => -1.0,
            }
        }

        /// The frequency of `mode`, `c √(r k_x² + k_y²/r)`. Written here from
        /// the physics, not read from the operator, so a model that took its
        /// slots or its orientation the other way would run its modes at
        /// another frequency or backwards.
        fn omega(self, mode: BoxMode) -> f64 {
            let k = mode.wavenumbers();
            let r = self.axis_ratio;
            self.speed() * (r * k.x * k.x + k.y * k.y / r).sqrt()
        }
    }

    /// A `width × height` box of `medium` centred on the origin, every side
    /// reflecting, meshed at `edge` on the application's own path, the
    /// source off.
    fn mode_box(
        medium: BoxMedium,
        width: f64,
        height: f64,
        edge: f64,
    ) -> Arc<crate::topology_runtime::PreparedTopology> {
        let mut builder = Builder::new();
        builder.scene.physics = medium.physics;
        builder.scene.geometry.domain = DomainRect {
            min_x: -0.5 * width,
            max_x: 0.5 * width,
            min_y: -0.5 * height,
            max_y: 0.5 * height,
        };
        builder.scene.materials[0].mass_density = ScalarField::constant(medium.mass);
        builder.scene.materials[0].stiffness = ScalarField::constant(medium.stiffness);
        builder.scene.materials[0].axis_ratio = ScalarField::constant(medium.axis_ratio);
        builder.scene.outer_boundaries =
            OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting);
        let mut document = builder.document();
        document.model.source.enabled = false;
        prepare(&document, edge)
    }

    /// The state of `modes`, each `(mode, amplitude)`, at `BOX_PHASE` of
    /// `u = φ cos(ωt + θ)`, from the closed forms alone: `Q = M u` at the
    /// nodes, and at the complementary samples `b = η R∇ψ` of its time
    /// integral `ψ = φ sin(ωt + θ)/ω`, less what of it lies in the coupling's
    /// kernel, which never moves. A constant is a mode of no frequency and
    /// no flux.
    fn mode_state(
        prepared: &crate::topology_runtime::PreparedTopology,
        medium: BoxMedium,
        modes: &[(BoxMode, f64)],
        constant: f64,
        integrator: CanonicalIntegrator,
        dt: f64,
    ) -> CanonicalWaveState {
        let operator = &prepared.canonical_operator;
        let orientation = medium.orientation();
        let primary = operator
            .node_points()
            .iter()
            .zip(operator.primary_mass())
            .map(|(point, mass)| {
                let field = modes
                    .iter()
                    .map(|(mode, amplitude)| amplitude * mode.shape(*point))
                    .sum::<f64>();
                mass * (constant + field * BOX_PHASE.cos())
            })
            .collect::<Vec<_>>();
        let sampled = operator
            .constitutive_samples()
            .iter()
            .map(|sample| {
                modes
                    .iter()
                    .fold(Point2::default(), |flux, (mode, amplitude)| {
                        let gradient = mode.gradient(sample.point);
                        let scale = orientation * amplitude * BOX_PHASE.sin() / medium.omega(*mode);
                        flux + Point2::new(-gradient.y, gradient.x) * scale
                    })
            })
            .collect::<Vec<_>>();
        let stationary = operator
            .stationary_complementary_component(&sampled)
            .unwrap();
        let flux = sampled
            .iter()
            .zip(stationary)
            .map(|(sampled, stationary)| *sampled - stationary)
            .collect();
        CanonicalWaveState::new(operator, dt, primary, flux)
            .unwrap()
            .with_integrator(integrator)
    }

    /// What a single-mode run reads off the mode's mass-weighted projection.
    struct ModeRun {
        /// The frequency the stepped mode has.
        stepped: f64,
        /// The phase the projection starts at, by a least-squares fit of
        /// `a cos(ω t) + c sin(ω t)` at the stepped frequency.
        phase: f64,
        /// The amplitude at the end over the start.
        envelope: f64,
        /// The energy's largest relative excursion from its start.
        excursion: f64,
    }

    /// Runs `mode` alone from `BOX_PHASE` for `periods` of the exact mode.
    fn mode_run(
        prepared: &crate::topology_runtime::PreparedTopology,
        medium: BoxMedium,
        mode: BoxMode,
        integrator: CanonicalIntegrator,
        dt: f64,
        periods: f64,
    ) -> ModeRun {
        let operator = &prepared.canonical_operator;
        let mut state = mode_state(prepared, medium, &[(mode, 1.0)], 0.0, integrator, dt);
        let shape = operator
            .node_points()
            .iter()
            .map(|point| mode.shape(*point))
            .collect::<Vec<_>>();
        let weights = shape
            .iter()
            .zip(operator.primary_mass())
            .map(|(shape, mass)| shape * mass)
            .collect::<Vec<_>>();
        let norm = weights.iter().zip(&shape).map(|(w, s)| w * s).sum::<f64>();
        let project = |state: &CanonicalWaveState| {
            let field = state.primary_field(operator).unwrap();
            weights.iter().zip(&field).map(|(w, u)| w * u).sum::<f64>() / norm
        };
        let omega = medium.omega(mode);
        let steps = (periods * std::f64::consts::TAU / omega / dt).ceil() as usize;
        let start = state.energy(operator).unwrap();
        let mut series = Vec::with_capacity(steps + 1);
        series.push(project(&state));
        let mut excursion: f64 = 0.0;
        for _ in 0..steps {
            state.step(operator).unwrap();
            series.push(project(&state));
            excursion = excursion.max((state.energy(operator).unwrap() / start - 1.0).abs());
        }
        let stepped = sampled_frequency(&series, dt);
        let (mut cc, mut cs, mut ss, mut yc, mut ys) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for (index, value) in series.iter().enumerate() {
            let (sin, cos) = (stepped * dt * index as f64).sin_cos();
            cc += cos * cos;
            cs += cos * sin;
            ss += sin * sin;
            yc += value * cos;
            ys += value * sin;
        }
        let determinant = cc * ss - cs * cs;
        let along_cos = (yc * ss - ys * cs) / determinant;
        let along_sin = (ys * cc - yc * cs) / determinant;
        let amplitude = |window: &[f64]| {
            (window[1].powi(2) + ((window[2] - window[0]) / (2.0 * (stepped * dt).sin())).powi(2))
                .sqrt()
        };
        ModeRun {
            stepped,
            phase: (-along_sin).atan2(along_cos),
            envelope: amplitude(&series[series.len() - 3..]) / amplitude(&series[1..4]),
            excursion,
        }
    }

    /// What a run of every mode at once reads at its end against the closed
    /// form `Σ a φ cos(ω t + θ) + c`.
    struct SuperpositionRun {
        /// The mass-weighted L2 error over the closed form's norm.
        l2: f64,
        /// The largest nodal error over the closed form's largest value.
        linf: f64,
        /// The energy's largest relative excursion from its start.
        excursion: f64,
        /// How far the total primary flux, which the constant carries, moved,
        /// over its size.
        total: f64,
        /// The complementary flux's L2 error, weighted by the samples'
        /// integration weights, over the closed form's norm: the field the
        /// model reconstructs beside its primary one, `B⊥` in TM and `D⊥` in
        /// TE.
        flux: f64,
    }

    /// The modes PH-01 runs together, each with its amplitude, and the
    /// constant beside them.
    const BOX_SUPERPOSITION: [(BoxMode, f64); 4] = [
        (BoxMode::ph01(1, 0), 1.0),
        (BoxMode::ph01(0, 1), 0.7),
        (BoxMode::ph01(1, 2), 0.5),
        (BoxMode::ph01(2, 3), 0.3),
    ];
    const BOX_CONSTANT: f64 = 0.4;

    /// Runs `modes` at once with `constant` for one period of the slowest,
    /// at the production step.
    fn superposition_run(
        prepared: &crate::topology_runtime::PreparedTopology,
        medium: BoxMedium,
        modes: &[(BoxMode, f64)],
        constant: f64,
    ) -> SuperpositionRun {
        let operator = &prepared.canonical_operator;
        let dt = prepared.recommended_time_step();
        let mut state = mode_state(
            prepared,
            medium,
            modes,
            constant,
            CanonicalIntegrator::default(),
            dt,
        );
        let slowest = modes
            .iter()
            .map(|(mode, _)| medium.omega(*mode))
            .fold(f64::INFINITY, f64::min);
        let steps = (std::f64::consts::TAU / slowest / dt).ceil() as usize;
        let total = |state: &CanonicalWaveState| state.primary_flux().iter().sum::<f64>();
        let (start, total_start) = (state.energy(operator).unwrap(), total(&state));
        let mut excursion: f64 = 0.0;
        for _ in 0..steps {
            state.step(operator).unwrap();
            excursion = excursion.max((state.energy(operator).unwrap() / start - 1.0).abs());
        }
        let time = steps as f64 * dt;
        let field = state.primary_field(operator).unwrap();
        let (mut error, mut norm, mut largest_error, mut largest) = (0.0, 0.0, 0.0_f64, 0.0_f64);
        for ((point, mass), read) in operator
            .node_points()
            .iter()
            .zip(operator.primary_mass())
            .zip(&field)
        {
            let exact = constant
                + modes
                    .iter()
                    .map(|(mode, amplitude)| {
                        let omega = medium.omega(*mode);
                        amplitude * mode.shape(*point) * (omega * time + BOX_PHASE).cos()
                    })
                    .sum::<f64>();
            error += mass * (read - exact).powi(2);
            norm += mass * exact * exact;
            largest_error = largest_error.max((read - exact).abs());
            largest = largest.max(exact.abs());
        }
        let magnitude = state
            .primary_flux()
            .iter()
            .map(|value| value.abs())
            .sum::<f64>();
        // The flux's closed form at the same instant, `η R∇ψ` of
        // `ψ = Σ a φ sin(ω t + θ)/ω`, less its part in the coupling's kernel
        // as the start was.
        let sampled = operator
            .constitutive_samples()
            .iter()
            .map(|sample| {
                modes
                    .iter()
                    .fold(Point2::default(), |flux, (mode, amplitude)| {
                        let omega = medium.omega(*mode);
                        let gradient = mode.gradient(sample.point);
                        let scale =
                            medium.orientation() * amplitude * (omega * time + BOX_PHASE).sin()
                                / omega;
                        flux + Point2::new(-gradient.y, gradient.x) * scale
                    })
            })
            .collect::<Vec<_>>();
        let stationary = operator
            .stationary_complementary_component(&sampled)
            .unwrap();
        let (mut flux_error, mut flux_norm) = (0.0, 0.0);
        for (((sample, sampled), stationary), read) in operator
            .constitutive_samples()
            .iter()
            .zip(&sampled)
            .zip(&stationary)
            .zip(state.complementary_flux())
        {
            let exact = *sampled - *stationary;
            let difference = *read - exact;
            flux_error += sample.integration_weight * difference.dot(difference);
            flux_norm += sample.integration_weight * exact.dot(exact);
        }
        SuperpositionRun {
            l2: (error / norm).sqrt(),
            linf: largest_error / largest,
            excursion,
            total: (total(&state) - total_start).abs() / magnitude,
            flux: (flux_error / flux_norm).sqrt(),
        }
    }

    /// PH-01, rectangular cavity modes (`docs/spikes/funfern-testing-campaign-plan.md`),
    /// in each physics model on the production path: the application's mesh
    /// of a reflecting 2 × 1.4 box, its medium mass slot 2 and stiffness slot
    /// 0.5, modes (1,0), (0,1), (1,2) and (2,3) one at a time and all four
    /// with a constant, each started at a third of a period, `θ = π/3`, its
    /// Q and b from the closed forms.
    ///
    /// The three models read alike to the last digit shown, Mechanical at
    /// half the speed and twice the step, as `BoxMedium::speed` says they must. In
    /// space the semidiscrete frequency misses the closed form by 6.4e-8,
    /// 9.1e-7, 2.1e-5 and 2.6e-5 at edge 0.16 and by 3.3e-10, 1.5e-9, 3.0e-8
    /// and 1.9e-7 at 0.04, 139 to 674 times down over the two halvings, the
    /// element's fourth order coming in; the phase starts within 7.6e-4 of
    /// θ at the coarsest and 3.8e-5 at the finest, the envelope holds to
    /// 2.7e-6 over three periods, and the energy wobbles by an eighth of
    /// `(ω dt)²`. All four together, after a period of (1,0), stand 7.5e-4,
    /// 1.1e-4 and 2.0e-5 from the closed form in L2, 1.5e-3, 2.3e-4 and
    /// 3.8e-5 at worst, and the constant's total flux holds to 3e-15. In
    /// time, on (2,3) at edge 0.08, leapfrog misses its semidiscrete
    /// frequency by `ω_h³dt²/24` to 0.03% and the fourth-order step by
    /// `−ω_h⁵dt⁴/720` to 0.07%, 1.4% at a quarter of its step where the
    /// miss is 7e-10, and the semidiscrete frequency read through either
    /// agrees over six steps to 1.2e-9.
    ///
    /// The closed-form b carries the law's orientation, so a model whose
    /// operator took it the other way would start its modes at −θ;
    /// `tests/mutations/tm-orientation-flipped` does that to TM.
    fn box_modes_in(physics: PhysicsModel) {
        let name = format!("{physics:?}");
        let medium = BoxMedium::ph01(physics);
        let speed = medium.speed();

        // Space, at the production step: the semidiscrete frequency, read
        // through the fourth-order step's dispersion relation, against the
        // closed form, with the phase, the envelope and the energy beside it.
        let mut errors = vec![Vec::new(); BOX_MODES.len()];
        let mut together = Vec::new();
        for edge in [0.16, 0.08, 0.04] {
            let prepared = mode_box(medium, 2.0, 1.4, edge);
            let dt = prepared.recommended_time_step();
            for (index, mode) in BOX_MODES.iter().enumerate() {
                let run = mode_run(
                    &prepared,
                    medium,
                    *mode,
                    CanonicalIntegrator::default(),
                    dt,
                    3.0,
                );
                let exact = speed * mode.wavenumber();
                let inverted =
                    semidiscrete_frequency(CanonicalIntegrator::default(), run.stepped, dt);
                let case = format!("{name} edge {edge} ({},{})", mode.m, mode.n);
                eprintln!(
                    "{case}: ω_h off {:.3e}, phase off {:.2e}, envelope − 1 {:.1e}, energy excursion {:.2e} of (ω dt)² {:.2e}",
                    inverted / exact - 1.0,
                    run.phase - BOX_PHASE,
                    run.envelope - 1.0,
                    run.excursion,
                    (exact * dt).powi(2)
                );
                assert!(
                    (run.phase - BOX_PHASE).abs() < 1.0e-3,
                    "{case}: phase {} for {BOX_PHASE}",
                    run.phase
                );
                assert!(
                    (run.envelope - 1.0).abs() < 1.0e-5,
                    "{case}: envelope {}",
                    run.envelope
                );
                assert!(
                    run.excursion > 0.0 && run.excursion < 0.25 * (exact * dt).powi(2),
                    "{case}: energy excursion {}",
                    run.excursion
                );
                errors[index].push((inverted / exact - 1.0).abs());
            }
            let run = superposition_run(&prepared, medium, &BOX_SUPERPOSITION, BOX_CONSTANT);
            eprintln!(
                "{name} edge {edge} together: L2 {:.3e}, L∞ {:.3e}, complementary flux {:.3e}, energy excursion {:.2e}, total flux {:.1e}",
                run.l2, run.linf, run.flux, run.excursion, run.total
            );
            assert!(
                run.total < 1.0e-13,
                "{name} edge {edge}: total flux {}",
                run.total
            );
            together.push(run);
        }
        for (mode, errors) in BOX_MODES.iter().zip(&errors) {
            assert!(
                errors[0] / errors[1] > 6.0 && errors[1] / errors[2] > 6.0,
                "{name} ({},{}): ω_h errors {errors:?}",
                mode.m,
                mode.n
            );
            assert!(
                errors[0] / errors[2] > 100.0 && errors[2] < 3.0e-7,
                "{name} ({},{}): ω_h errors {errors:?}",
                mode.m,
                mode.n
            );
        }
        for pair in together.windows(2) {
            // PH-02's field reconstruction in each polarization: the
            // complementary flux, `B⊥` in TM and `D⊥` in TE, against its
            // closed form.
            assert!(
                pair[0].flux / pair[1].flux > 5.0,
                "{name}: complementary flux errors {} and {}",
                pair[0].flux,
                pair[1].flux
            );
            assert!(
                pair[0].l2 / pair[1].l2 > 4.0 && pair[0].linf / pair[1].linf > 4.0,
                "{name}: field errors L2 {} and {}, L∞ {} and {}",
                pair[0].l2,
                pair[1].l2,
                pair[0].linf,
                pair[1].linf
            );
            assert!(
                pair[0].excursion / pair[1].excursion > 3.5,
                "{name}: energy excursions {} and {}",
                pair[0].excursion,
                pair[1].excursion
            );
        }
        assert!(together[2].l2 < 3.0e-5 && together[2].linf < 6.0e-5);
        assert!(together[2].flux < 3.0e-5, "{name}: {}", together[2].flux);

        // Time, at one mesh, on the fastest mode: each integrator's stepped
        // frequency misses the semidiscrete one by its own law, and the
        // semidiscrete frequency read through either agrees at every step.
        let prepared = mode_box(medium, 2.0, 1.4, 0.08);
        let largest = prepared.canonical_operator.maximum_time_step();
        let mode = BOX_MODES[3];
        let mut semidiscrete = Vec::new();
        for integrator in [
            CanonicalIntegrator::Leapfrog,
            CanonicalIntegrator::FourthOrder,
        ] {
            for fraction in [1.0, 0.5, 0.25] {
                let dt = fraction * integrator.recommended() * largest;
                let run = mode_run(&prepared, medium, mode, integrator, dt, 3.0);
                let inverted = semidiscrete_frequency(integrator, run.stepped, dt);
                let law = match integrator {
                    CanonicalIntegrator::Leapfrog => inverted.powi(3) * dt * dt / 24.0,
                    CanonicalIntegrator::FourthOrder => -inverted.powi(5) * dt.powi(4) / 720.0,
                };
                let case = format!("{name} {integrator:?} at {fraction} of its step");
                eprintln!(
                    "{case}: stepped − ω_h {:.3e} against {law:.3e}, phase off {:.2e}",
                    run.stepped - inverted,
                    run.phase - BOX_PHASE
                );
                assert!(
                    (run.stepped - inverted - law).abs() < 1.0e-2 * law.abs() + 1.0e-10,
                    "{case}: {} against {law}",
                    run.stepped - inverted
                );
                assert!(
                    (run.phase - BOX_PHASE).abs() < 1.0e-3,
                    "{case}: phase {} for {BOX_PHASE}",
                    run.phase
                );
                semidiscrete.push(inverted);
            }
        }
        let spread = semidiscrete.iter().copied().fold(f64::MIN, f64::max)
            - semidiscrete.iter().copied().fold(f64::MAX, f64::min);
        assert!(
            spread < 1.0e-8 * speed * mode.wavenumber(),
            "{name}: ω_h spread {spread:.3e} over {semidiscrete:?}"
        );
    }

    #[test]
    fn box_modes_in_mechanical() {
        box_modes_in(PhysicsModel::Mechanical);
    }

    #[test]
    fn box_modes_in_tm() {
        box_modes_in(PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Tm,
        });
    }

    #[test]
    fn box_modes_in_te() {
        box_modes_in(PhysicsModel::Electromagnetic {
            polarization: ElectromagneticPolarization::Te,
        });
    }

    /// PH-02's box, 2 × 1.06, and its modes, which together point from 0° to
    /// 90° at wavenumbers from 5.6 to 8.6: five to twenty elements a
    /// wavelength on the meshes PH-02 reads. The height is the one that keeps
    /// each of them 2.5% or more in frequency from every other mode of the
    /// box, so that the mesh mixes none of them into another.
    const PLANE_MODES: [(u32, u32); 7] = [(5, 0), (5, 1), (3, 1), (4, 2), (2, 2), (1, 2), (0, 2)];
    const PLANE_EDGES: [f64; 3] = [0.16, 0.08, 0.04];

    /// PH-02's medium: Mechanical, mass and stiffness one, so the speed is
    /// one, its stiffness `r` times itself along x and over `r` along y.
    const fn plane_medium(axis_ratio: f64) -> BoxMedium {
        BoxMedium {
            physics: PhysicsModel::Mechanical,
            mass: 1.0,
            stiffness: 1.0,
            axis_ratio,
        }
    }

    /// Mode `(m, n)` of PH-02's box for axis ratio `r`: the box `2√r ×
    /// 1.06/√r`. In the coordinates `X = x/√r`, `Y = y√r` the medium is
    /// isotropic and the box is the isotropic one, so its modes are the
    /// isotropic box's at the same frequencies; and since the elements are
    /// affine, the same discretization is the isotropic medium on the
    /// mesher's elements stretched `r` times along y against x. An axis
    /// ratio is how PH-02 reads elements of aspect `r` on meshes the
    /// application builds.
    fn plane_mode(m: u32, n: u32, axis_ratio: f64) -> BoxMode {
        let stretch = axis_ratio.sqrt();
        BoxMode {
            m,
            n,
            width: 2.0 * stretch,
            height: 1.06 / stretch,
        }
    }

    /// The direction of a mode's wavevector in the medium's isotropic
    /// coordinates, in degrees from x.
    fn plane_direction(m: u32, n: u32) -> f64 {
        let isotropic = plane_mode(m, n, 1.0).wavenumbers();
        isotropic.y.atan2(isotropic.x).to_degrees()
    }

    /// The semidiscrete frequency of `mode` on `prepared` over the closed
    /// form, less one: the element's dispersion, read through the
    /// fourth-order step's own dispersion relation at the production step.
    fn plane_dispersion(
        prepared: &crate::topology_runtime::PreparedTopology,
        medium: BoxMedium,
        mode: BoxMode,
    ) -> f64 {
        let dt = prepared.recommended_time_step();
        let run = mode_run(
            prepared,
            medium,
            mode,
            CanonicalIntegrator::default(),
            dt,
            3.0,
        );
        semidiscrete_frequency(CanonicalIntegrator::default(), run.stepped, dt) / medium.omega(mode)
            - 1.0
    }

    /// PH-02's modes at axis ratio `r` on PH-02's three meshes: each mode's
    /// dispersion at each edge, and the coefficient `|ε|/(k h)⁴` the
    /// element's fourth order makes of it, printed. Each mode's error must
    /// fall at least sixfold from 0.08 to 0.04 and thirtyfold from 0.16,
    /// fourth order coming in where a coarse error crosses zero, and stay
    /// under `finest` at 0.04.
    fn plane_dispersion_sweep(axis_ratio: f64, finest: f64) -> Vec<[f64; 3]> {
        let medium = plane_medium(axis_ratio);
        let probe = plane_mode(1, 0, axis_ratio);
        let mut errors = vec![[0.0; 3]; PLANE_MODES.len()];
        for (level, edge) in PLANE_EDGES.iter().enumerate() {
            let prepared = mode_box(medium, probe.width, probe.height, *edge);
            for (index, (m, n)) in PLANE_MODES.iter().enumerate() {
                let mode = plane_mode(*m, *n, axis_ratio);
                let error = plane_dispersion(&prepared, medium, mode);
                let kh = medium.omega(mode) * edge;
                eprintln!(
                    "r {axis_ratio}, edge {edge}, ({m},{n}) at {:.1}°: kh {kh:.3}, ε {error:.3e}, |ε|/(kh)⁴ {:.3e}",
                    plane_direction(*m, *n),
                    error.abs() / kh.powi(4)
                );
                errors[index][level] = error;
            }
        }
        for ((m, n), errors) in PLANE_MODES.iter().zip(&errors) {
            let [coarse, middle, fine] = errors.map(f64::abs);
            assert!(
                middle / fine > 6.0 && coarse / fine > 30.0 && fine < finest,
                "r {axis_ratio}, ({m},{n}): ε {errors:?}"
            );
        }
        errors
    }

    /// PH-02, plane-wave dispersion (`docs/spikes/funfern-testing-campaign-plan.md`),
    /// on the application's meshes of edge 0.16, 0.08 and 0.04: the
    /// semidiscrete frequency of seven modes pointing from 0° to 90°, read
    /// through the fourth-order step's own dispersion relation, against the
    /// closed form. Measured: fourth order, (4,2) at 1.0e-4, 4.0e-6 and
    /// 3.6e-7, every mode 121 to 282 times down over the two halvings; the
    /// coefficient `|ε|/(k h)⁴` at 0.04 is 2.1e-5 to 2.5e-5 in every
    /// direction, the mesher's unoriented elements dispersing alike whichever
    /// way the wave goes, held to 1.5 times its least.
    #[test]
    fn plane_waves_disperse_as_the_element_does_in_every_direction() {
        let errors = plane_dispersion_sweep(1.0, 5.0e-7);
        let coefficients = PLANE_MODES
            .iter()
            .zip(&errors)
            .map(|((m, n), errors)| {
                let kh = plane_medium(1.0).omega(plane_mode(*m, *n, 1.0)) * PLANE_EDGES[2];
                errors[2].abs() / kh.powi(4)
            })
            .collect::<Vec<_>>();
        let (least, most) = coefficients
            .iter()
            .fold((f64::INFINITY, 0.0_f64), |(least, most), c| {
                (least.min(*c), most.max(*c))
            });
        assert!(most / least < 1.5, "{coefficients:?}");
    }

    /// PH-02 on stretched elements: the same modes at axis ratio 2, which by
    /// `plane_mode`'s affine equivalence is the isotropic medium on the
    /// mesher's elements stretched twofold along y. Fourth order holds,
    /// every mode 47 to 1307 times down over the two halvings, but a wave
    /// along the stretch disperses ten times as much at 0.04 as on the
    /// unstretched elements, 7.6e-7 at 90° for 7.2e-8, where the coarser
    /// resolution alone would account for four.
    #[test]
    fn plane_waves_disperse_on_elements_stretched_twofold() {
        plane_dispersion_sweep(2.0, 1.5e-6);
    }

    /// The same at axis ratio 4: still fourth order, every mode 56 times
    /// down or more, and a wave along the stretch 175 times the unstretched
    /// dispersion at 0.04, 1.3e-5 at 90°, where resolution alone would
    /// account for sixteen.
    #[test]
    fn plane_waves_disperse_on_elements_stretched_fourfold() {
        plane_dispersion_sweep(4.0, 2.5e-5);
    }

    /// PH-02 in time: mode (4,2) of the isotropic box on the 0.08 mesh, under
    /// leapfrog and under the fourth-order step at ¼, ½ and 1 of each one's
    /// recommended step and just under its stability limit. Each misses the
    /// semidiscrete frequency by its own law to 0.1% at every step,
    /// `ω_h³dt²/24` and `−ω_h⁵dt⁴/720`, the limit included, where the energy
    /// still wobbles by an eighth of `(ω dt)²`; the semidiscrete frequency
    /// read through either agrees over the eight runs to 4e-9.
    #[test]
    fn plane_waves_keep_each_integrators_dispersion_up_to_its_stability_limit() {
        let medium = plane_medium(1.0);
        let mode = plane_mode(4, 2, 1.0);
        let prepared = mode_box(medium, mode.width, mode.height, 0.08);
        let largest = prepared.canonical_operator.maximum_time_step();
        let mut semidiscrete = Vec::new();
        for integrator in [
            CanonicalIntegrator::Leapfrog,
            CanonicalIntegrator::FourthOrder,
        ] {
            let limit = 0.999 * integrator.ceiling() / integrator.recommended();
            for fraction in [0.25, 0.5, 1.0, limit] {
                let dt = fraction * integrator.recommended() * largest;
                let run = mode_run(&prepared, medium, mode, integrator, dt, 3.0);
                let inverted = semidiscrete_frequency(integrator, run.stepped, dt);
                let law = match integrator {
                    CanonicalIntegrator::Leapfrog => inverted.powi(3) * dt * dt / 24.0,
                    CanonicalIntegrator::FourthOrder => -inverted.powi(5) * dt.powi(4) / 720.0,
                };
                let case = format!("{integrator:?} at {fraction:.4} of its step");
                eprintln!(
                    "{case}: ω_h off {:.3e}, stepped − ω_h {:.3e} against {law:.3e}, energy excursion {:.2e}",
                    inverted / medium.omega(mode) - 1.0,
                    run.stepped - inverted,
                    run.excursion
                );
                assert!(
                    (run.stepped - inverted - law).abs() < 1.0e-2 * law.abs() + 1.0e-10,
                    "{case}: {} against {law}",
                    run.stepped - inverted
                );
                assert!(
                    run.excursion < 0.25 * (run.stepped * dt).powi(2),
                    "{case}: energy excursion {}",
                    run.excursion
                );
                semidiscrete.push(inverted);
            }
        }
        let spread = semidiscrete.iter().copied().fold(f64::MIN, f64::max)
            - semidiscrete.iter().copied().fold(f64::MAX, f64::min);
        assert!(
            spread < 1.0e-8 * medium.omega(mode),
            "ω_h spread {spread:.3e} over {semidiscrete:?}"
        );
    }

    /// PH-02's group velocity: along 62° and along 90°, the semidiscrete
    /// frequency at three wavenumbers `k`, `2k` and `3k` on each mesh, and
    /// its slope across them against the medium's, one. Measured 2.5e-4,
    /// 1.9e-5 and 1.1e-6 off along 62° and 1.6e-4, 1.7e-5 and 6.9e-7 along
    /// 90°, fourth order; held to sixfold a halving and 2e-6 at 0.04.
    #[test]
    fn plane_waves_travel_at_their_group_velocity() {
        let medium = plane_medium(1.0);
        for steps in [[(1, 1), (2, 2), (3, 3)], [(0, 1), (0, 2), (0, 3)]] {
            let mut errors = Vec::new();
            for edge in PLANE_EDGES {
                let probe = plane_mode(1, 0, 1.0);
                let prepared = mode_box(medium, probe.width, probe.height, edge);
                let frequencies = steps.map(|(m, n)| {
                    let mode = plane_mode(m, n, 1.0);
                    let error = plane_dispersion(&prepared, medium, mode);
                    (medium.omega(mode) * (1.0 + error), medium.omega(mode))
                });
                let slope =
                    (frequencies[2].0 - frequencies[0].0) / (frequencies[2].1 - frequencies[0].1);
                eprintln!(
                    "({},{}) to ({},{}) at {:.1}°, edge {edge}: group velocity off {:.3e}",
                    steps[0].0,
                    steps[0].1,
                    steps[2].0,
                    steps[2].1,
                    plane_direction(steps[0].0, steps[0].1),
                    slope - 1.0
                );
                errors.push((slope - 1.0).abs());
            }
            assert!(
                errors[0] / errors[1] > 6.0 && errors[1] / errors[2] > 6.0 && errors[2] < 2.0e-6,
                "{steps:?}: group velocity errors {errors:?}"
            );
        }
    }

    /// Carries a fixed state from `source` to `target` through `maps`, as the
    /// application's handoff does on the reference: integrated flux through
    /// the primary map with its component totals kept when the geometry is
    /// the same, the complementary flux through its map.
    fn carry_over(
        state: &CanonicalWaveState,
        source: &crate::topology_runtime::PreparedTopology,
        target: &crate::topology_runtime::PreparedTopology,
        maps: &crate::topology_runtime::PreparedCanonicalTransfer,
    ) -> CanonicalWaveState {
        let runtime = crate::canonical_gpu::CanonicalGpuRuntimeTransfer::from_primary_transfer(
            &source.canonical_operator,
            &target.canonical_operator,
            &source.canonical_forcing,
            &target.canonical_forcing,
            &maps.primary,
            [0; 4],
        )
        .unwrap();
        let runtime = if target.mesh.geometry_revision == source.mesh.geometry_revision {
            runtime
        } else {
            runtime.without_total_correction()
        };
        let mut totals = vec![0.0; source.canonical_operator.component_count()];
        for (value, label) in state
            .primary_flux()
            .iter()
            .zip(source.canonical_operator.component_labels())
        {
            totals[*label as usize] += value;
        }
        let desired = runtime
            .components
            .iter()
            .map(|component| {
                (!component.sources.is_empty()).then(|| {
                    component
                        .sources
                        .iter()
                        .map(|(source, share)| share * totals[*source as usize])
                        .sum()
                })
            })
            .collect::<Vec<Option<f64>>>();
        let prescribed = target
            .canonical_forcing
            .prescribed()
            .iter()
            .map(Option::is_some)
            .collect::<Vec<_>>();
        let (primary, _) = maps
            .primary
            .transfer(state.primary_flux(), &desired, &prescribed)
            .unwrap();
        let (complementary, _) = maps
            .complementary
            .transfer(state.complementary_flux())
            .unwrap();
        CanonicalWaveState::new(
            &target.canonical_operator,
            state.time_step(),
            primary,
            complementary,
        )
        .unwrap()
    }

    /// The primary field of `state` at `points` on `prepared`'s mesh.
    fn field_at(
        state: &CanonicalWaveState,
        prepared: &crate::topology_runtime::PreparedTopology,
        points: &[Point2],
    ) -> Vec<f64> {
        let field = state.primary_field(&prepared.canonical_operator).unwrap();
        points
            .iter()
            .map(|point| {
                let stencil = QuadraticPointStencil::build_topology(
                    &prepared.mesh,
                    &prepared.operator,
                    &prepared.bundle.plan,
                    prepared.fixed_model(),
                    *point,
                )
                .unwrap();
                stencil
                    .nodes
                    .iter()
                    .zip(stencil.value_weights)
                    .map(|(node, weight)| weight * field[*node as usize])
                    .sum()
            })
            .collect()
    }

    /// The relative RMS distance between two readings.
    fn relative_distance(a: &[f64], b: &[f64]) -> f64 {
        let difference: f64 = a.iter().zip(b).map(|(a, b)| (a - b).powi(2)).sum();
        let scale: f64 = b.iter().map(|b| b * b).sum();
        (difference / scale).sqrt()
    }

    /// The reflecting box with a disc of its own medium drawn in it: the
    /// field cannot tell.
    fn reflecting_box_with_disc() -> TopologyDocument {
        let mut builder = Builder::new();
        builder.scene.outer_boundaries =
            OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting);
        let background = builder.scene.regions[0].material;
        builder.subdomain(
            circle(Point2::new(0.2, -0.1), 0.35),
            background,
            MaterialFrame::world(),
        );
        let mut document = builder.document();
        document.model.source.enabled = false;
        document
    }

    /// A curl-free flux, the gradient of `cos(πx/2) cos(πy/2)`: it pushes
    /// no field, so it is static, a stored field that never moves.
    fn static_flux(point: Point2) -> Point2 {
        let half = 0.5 * std::f64::consts::PI;
        Point2::new(
            -half * (half * point.x).sin() * (half * point.y).cos(),
            -half * (half * point.x).cos() * (half * point.y).sin(),
        )
    }

    /// Edits the field cannot tell, on the application's own path. The box
    /// mode `cos(3π(x+1)/2) cos(3π(y+1)/2)` runs three periods; a disc of the
    /// box's own medium is then drawn in, the state carried onto the new
    /// mesh through the maps the runtime prepares for the edit, run three
    /// periods more, and the disc erased the same way. Against the untouched
    /// run at twenty-five points the edited one is off by 7e-4 right after the
    /// disc is drawn and 9e-4 after it is erased and three more periods, at
    /// edge 0.16; at 0.08 by 5e-5 and 1e-4, nine to twelve times less. A
    /// remesh 1.4 times finer mid-run is off by 1e-3 at once and 3e-3 three
    /// periods on at edge 0.16, 2e-4 and 4e-4 at 0.08: the carried field's
    /// error falls with the mesh, as a transfer's must. And a flux that
    /// pushes no field - the gradient of `cos(πx/2) cos(πy/2)`, curl-free,
    /// which in TM is a magnetostatic field - is static: sampled on the mesh
    /// its compatible part is 1e-13 of it by energy, and less that part its
    /// field stays at 1e-14 over three periods; carried across that remesh it
    /// keeps its
    /// energy to 4e-7 and 5e-9, stays force-free to a compatible share of
    /// 3e-10 and 5e-12, and gives the field 1e-10 of its energy in three
    /// periods more.
    #[test]
    fn edits_the_field_cannot_tell_leave_it_as_the_mesh_allows_and_a_static_flux_static() {
        let k = 1.5 * std::f64::consts::PI;
        let period = std::f64::consts::TAU / (k * 2.0f64.sqrt());
        let shape = |operator: &CanonicalWaveOperator| {
            operator
                .node_points()
                .iter()
                .map(|point| (k * (point.x + 1.0)).cos() * (k * (point.y + 1.0)).cos())
                .collect::<Vec<_>>()
        };
        let points = (0..5)
            .flat_map(|i| {
                (0..5)
                    .map(move |j| Point2::new(-0.8 + 0.4 * f64::from(i), -0.8 + 0.4 * f64::from(j)))
            })
            .collect::<Vec<_>>();
        let mut plain = Builder::new();
        plain.scene.outer_boundaries =
            OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting);
        let mut plain = plain.document();
        plain.model.source.enabled = false;
        let with_disc = reflecting_box_with_disc();
        // Each run against the untouched one, right after the edit and
        // `stage` steps on, for every stage of the sequence of edits.
        let distances = |edits: &[(&TopologyDocument, f64)], edge: f64| -> Vec<f64> {
            let mut runtime = TopologyRuntime::default();
            let (first, _) = prepare_on(&mut runtime, &plain, 1, edge);
            let mut generations = vec![(first.clone(), None)];
            for (revision, (document, scale)) in edits.iter().enumerate() {
                let (prepared, maps) =
                    prepare_on(&mut runtime, document, revision as u64 + 2, edge * scale);
                generations.push((prepared, Some(maps.expect("maps for the edit"))));
            }
            // Every generation takes the smallest step among them.
            let dt = generations
                .iter()
                .map(|(prepared, _)| prepared.recommended_time_step())
                .fold(f64::INFINITY, f64::min);
            let stage = (3.0 * period / dt).ceil() as usize;
            let operator = &first.canonical_operator;
            let mut control = CanonicalWaveState::from_primary_and_potential(
                operator,
                dt,
                &shape(operator),
                &vec![0.0; operator.degrees_of_freedom()],
            )
            .unwrap();
            let mut edited = control.clone();
            for _ in 0..stage {
                control.step(operator).unwrap();
                edited.step(operator).unwrap();
            }
            let mut distances = Vec::new();
            for pair in generations.windows(2) {
                let (source, _) = &pair[0];
                let (target, maps) = &pair[1];
                edited = carry_over(&edited, source, target, maps.as_ref().unwrap());
                distances.push(relative_distance(
                    &field_at(&edited, target, &points),
                    &field_at(&control, &first, &points),
                ));
                for _ in 0..stage {
                    control.step(operator).unwrap();
                    edited.step(&target.canonical_operator).unwrap();
                }
                distances.push(relative_distance(
                    &field_at(&edited, target, &points),
                    &field_at(&control, &first, &points),
                ));
            }
            distances
        };
        let drawn_and_erased = [0.16, 0.08].map(|edge| {
            let distances = distances(&[(&with_disc, 1.0), (&plain, 1.0)], edge);
            eprintln!(
                "edge {edge}: disc drawn {:.2e}, run {:.2e}, erased {:.2e}, run {:.2e}",
                distances[0], distances[1], distances[2], distances[3]
            );
            distances
        });
        for distance in &drawn_and_erased[0] {
            assert!(*distance < 2.0e-3, "{drawn_and_erased:?}");
        }
        for (coarse, fine) in drawn_and_erased[0].iter().zip(&drawn_and_erased[1]) {
            assert!(coarse / fine > 5.0, "{drawn_and_erased:?}");
        }
        let remeshed = [0.16, 0.08].map(|edge| {
            let distances = distances(&[(&plain, 1.0 / 1.4)], edge);
            eprintln!(
                "edge {edge}: remeshed {:.2e}, run {:.2e}",
                distances[0], distances[1]
            );
            distances
        });
        assert!(
            remeshed[0][0] < 2.0e-3 && remeshed[0][1] < 5.0e-3,
            "{remeshed:?}"
        );
        for (coarse, fine) in remeshed[0].iter().zip(&remeshed[1]) {
            assert!(coarse / fine > 3.0, "{remeshed:?}");
        }

        // The static flux across that remesh.
        let mut carried_energy_changes = Vec::new();
        for edge in [0.16, 0.08] {
            let mut runtime = TopologyRuntime::default();
            let (coarse, _) = prepare_on(&mut runtime, &plain, 1, edge);
            let (fine, maps) = prepare_on(&mut runtime, &plain, 2, edge / 1.4);
            let maps = maps.unwrap();
            let dt = coarse
                .recommended_time_step()
                .min(fine.recommended_time_step());
            let stage = (3.0 * period / dt).ceil() as usize;
            let operator = &coarse.canonical_operator;
            let sampled = operator
                .constitutive_samples()
                .iter()
                .map(|sample| static_flux(sample.point))
                .collect::<Vec<_>>();
            // Sampled, the gradient is force-free to its discretization; less
            // its compatible part it is force-free exactly.
            assert!(compatible_share(operator, dt, &sampled) < 1.0e-10);
            let compatible = compatible_part(operator, dt, &sampled);
            let flux = sampled
                .iter()
                .zip(&compatible)
                .map(|(sampled, compatible)| *sampled - *compatible)
                .collect::<Vec<_>>();
            let mut state = CanonicalWaveState::new(
                operator,
                dt,
                vec![0.0; operator.degrees_of_freedom()],
                flux,
            )
            .unwrap();
            let energy = state.energy(operator).unwrap();
            for _ in 0..stage {
                state.step(operator).unwrap();
            }
            let moved = state
                .primary_flux()
                .iter()
                .fold(0.0f64, |m, q| m.max(q.abs()));
            assert!(moved < 1.0e-12, "{moved}");
            let mut carried = carry_over(&state, &coarse, &fine, &maps);
            let fine_operator = &fine.canonical_operator;
            let carried_energy = carried.energy(fine_operator).unwrap();
            let share = compatible_share(fine_operator, dt, carried.complementary_flux());
            for _ in 0..stage {
                carried.step(fine_operator).unwrap();
            }
            let pushed = carried
                .primary_flux()
                .iter()
                .zip(carried.primary_field(fine_operator).unwrap())
                .map(|(q, u)| 0.5 * q * u)
                .sum::<f64>()
                / carried_energy;
            eprintln!(
                "edge {edge}: static flux of energy {energy:.4} carried to {:.2e} of it, compatible share {share:.1e}, {pushed:.1e} of it in the field after {stage} steps",
                carried_energy / energy - 1.0
            );
            assert!((carried_energy / energy - 1.0).abs() < 1.0e-5);
            assert!(share < 1.0e-8, "{share}");
            assert!(pushed < 1.0e-8, "{pushed}");
            carried_energy_changes.push((carried_energy / energy - 1.0).abs());
        }
        assert!(carried_energy_changes[0] > 10.0 * carried_energy_changes[1]);
    }

    /// The compatible part of a flux, the one that pushes the field: the
    /// flux `ηCψ` of the potential the Poisson solve finds for its force.
    fn compatible_part(operator: &CanonicalWaveOperator, dt: f64, flux: &[Point2]) -> Vec<Point2> {
        let force = operator.force(flux).unwrap();
        let velocity = force
            .iter()
            .zip(operator.primary_mass())
            .map(|(force, mass)| -force / mass)
            .collect::<Vec<_>>();
        let zeros = vec![0.0; operator.degrees_of_freedom()];
        CanonicalWaveState::from_primary_velocity(operator, dt, &zeros, &velocity)
            .unwrap()
            .complementary_flux()
            .to_vec()
    }

    /// How much of a flux is compatible, by energy.
    fn compatible_share(operator: &CanonicalWaveOperator, dt: f64, flux: &[Point2]) -> f64 {
        let zeros = vec![0.0; operator.degrees_of_freedom()];
        let energy = |flux: Vec<Point2>| {
            CanonicalWaveState::new(operator, dt, zeros.clone(), flux)
                .unwrap()
                .energy(operator)
                .unwrap()
        };
        energy(compatible_part(operator, dt, flux)) / energy(flux.to_vec())
    }

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

    // -----------------------------------------------------------------------
    // PH-17: energy accounting. Each channel's power is written here from the
    // authored law and integrated over the run's own states; nothing is read
    // from the solver's ledger or its compiled rates, and the ledger is held
    // to what this finds.
    // -----------------------------------------------------------------------

    /// A loss channel's rate, as authored and as the oracle reads it: per
    /// unit time, on the magnitude `f` of the channel's own physical field.
    /// A passive rate that follows the field is not executable yet (gate C),
    /// so the field enters only through van der Pol.
    #[derive(Clone, Copy, Debug)]
    enum LossRate {
        /// `γ`.
        Constant(f64),
        /// `γ (1 + x/2)`, rising across the box.
        Rising(f64),
        /// `γ (1 + depth cos 2πνt)`.
        Driven { rate: f64, depth: f64, hz: f64 },
        /// `γ (f²/f₀² − 1)`: gain below the threshold `f₀`, loss above it.
        VanDerPol { rate: f64, threshold: f64 },
    }

    impl LossRate {
        fn channel(self) -> LossChannel {
            let constant = ScalarField::constant;
            let (base_rate, rate, drive) = match self {
                Self::Constant(rate) => (constant(rate), RateLaw::Constant, TimeDrive::None),
                Self::Rising(rate) => (
                    ScalarField::formula(format!("{rate} * (1 + x / 2)")).unwrap(),
                    RateLaw::Constant,
                    TimeDrive::None,
                ),
                Self::Driven { rate, depth, hz } => (
                    constant(rate),
                    RateLaw::Constant,
                    TimeDrive::ParametricPump {
                        depth: constant(depth),
                        frequency_hz: constant(hz),
                        phase_radians: constant(0.0),
                    },
                ),
                Self::VanDerPol { rate, threshold } => (
                    constant(rate),
                    RateLaw::VanDerPol {
                        threshold: constant(threshold),
                        amplitude_bound: constant(4.0 * threshold.max(1.0)),
                    },
                    TimeDrive::None,
                ),
            };
            LossChannel {
                base_rate,
                law: DampingLaw {
                    rate,
                    drive,
                    gate: None,
                },
            }
        }

        /// The rate at `point` on a field of magnitude `field` at `time`.
        fn at(self, point: Point2, field: f64, time: f64) -> f64 {
            match self {
                Self::Constant(rate) => rate,
                Self::Rising(rate) => rate * (1.0 + 0.5 * point.x),
                Self::Driven { rate, depth, hz } => {
                    rate * (1.0 + depth * (std::f64::consts::TAU * hz * time).cos())
                }
                Self::VanDerPol { rate, threshold } => rate * ((field / threshold).powi(2) - 1.0),
            }
        }

        /// Whether the channel can put energy in: what it moves is gain.
        fn active(self) -> bool {
            matches!(self, Self::VanDerPol { .. })
        }
    }

    /// A drive on a constitutive row, as authored and as the oracle reads it:
    /// the row's coefficient is multiplied by `m(x, t)`.
    #[derive(Clone, Copy, Debug)]
    enum RowDrive {
        /// `1 + depth cos θ`, `θ = 2πνt + φ`.
        Pump { depth: f64, hz: f64, phase: f64 },
        /// `1 + depth tanh(σ cos θ) / tanh σ`.
        Crystal {
            depth: f64,
            hz: f64,
            phase: f64,
            sharpness: f64,
        },
        /// `1 + depth cos(θ − q·x)`, `q` of magnitude `wavenumber` along
        /// `angle`.
        Travelling {
            depth: f64,
            hz: f64,
            phase: f64,
            wavenumber: f64,
            angle: f64,
        },
    }

    impl RowDrive {
        fn drive(self) -> TimeDrive {
            let constant = ScalarField::constant;
            match self {
                Self::Pump { depth, hz, phase } => TimeDrive::ParametricPump {
                    depth: constant(depth),
                    frequency_hz: constant(hz),
                    phase_radians: constant(phase),
                },
                Self::Crystal {
                    depth,
                    hz,
                    phase,
                    sharpness,
                } => TimeDrive::TimeCrystal {
                    depth: constant(depth),
                    frequency_hz: constant(hz),
                    phase_radians: constant(phase),
                    sharpness: constant(sharpness),
                },
                Self::Travelling {
                    depth,
                    hz,
                    phase,
                    wavenumber,
                    angle,
                } => TimeDrive::TravellingModulation {
                    depth: constant(depth),
                    frequency_hz: constant(hz),
                    phase_radians: constant(phase),
                    wavenumber: constant(wavenumber),
                    angle_radians: constant(angle),
                },
            }
        }

        /// The factor `m` and its rate `∂m/∂t` at `point` and `time`.
        fn factor_and_rate(self, point: Point2, time: f64) -> (f64, f64) {
            let omega = |hz: f64| std::f64::consts::TAU * hz;
            match self {
                Self::Pump { depth, hz, phase } => {
                    let angle = omega(hz) * time + phase;
                    (1.0 + depth * angle.cos(), -depth * omega(hz) * angle.sin())
                }
                Self::Crystal {
                    depth,
                    hz,
                    phase,
                    sharpness,
                } => {
                    let angle = omega(hz) * time + phase;
                    let level = (sharpness * angle.cos()).tanh();
                    let scale = depth / sharpness.tanh();
                    (
                        1.0 + scale * level,
                        -scale * (1.0 - level * level) * sharpness * angle.sin() * omega(hz),
                    )
                }
                Self::Travelling {
                    depth,
                    hz,
                    phase,
                    wavenumber,
                    angle,
                } => {
                    let along = point.x * angle.cos() + point.y * angle.sin();
                    let angle = omega(hz) * time - wavenumber * along + phase;
                    (1.0 + depth * angle.cos(), -depth * omega(hz) * angle.sin())
                }
            }
        }
    }

    /// One PH-17 configuration of PH-01's box: what its material and scene
    /// carry beyond the plain medium.
    #[derive(Clone, Copy, Debug)]
    struct Ledgered {
        name: &'static str,
        physics: PhysicsModel,
        electric: Option<LossRate>,
        magnetic: Option<LossRate>,
        mass: Option<RowDrive>,
        stiffness: Option<RowDrive>,
        /// A harmonic volume source over the box, `(amplitude, hz)`, on
        /// `profile`.
        source: Option<(f64, f64)>,
        profile: &'static str,
        /// A thin gap across the box, at this stiffness ratio.
        gap: Option<f64>,
        /// Kerr on the density row, `1 + χ₂ f²`.
        kerr: Option<f64>,
        /// Klein-Gordon's restoring force at this cutoff `ω₀`.
        restoring: Option<f64>,
        /// An outgoing wall on the box's right side, of this order.
        wall: Option<OuterBoundaryCondition>,
    }

    impl Ledgered {
        const fn plain(name: &'static str, physics: PhysicsModel) -> Self {
            Self {
                name,
                physics,
                electric: None,
                magnetic: None,
                mass: None,
                stiffness: None,
                source: None,
                profile: "1 - x * x / 4",
                gap: None,
                kerr: None,
                restoring: None,
                wall: None,
            }
        }
    }

    const LEDGER_TM: PhysicsModel = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    const LEDGER_TE: PhysicsModel = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Te,
    };

    /// Where each physical field lives on the canonical core, from the
    /// physics: TM's electric field is the nodal scalar `E_z` and its
    /// magnetic field the in-plane vector; TE is the other way round, and
    /// Mechanical runs on TE's core with its velocity in the magnetic place.
    /// The density row (`ε`, `ρ`) holds the electric energy in the EM skins
    /// and the kinetic energy in Mechanical.
    fn electric_is_primary(physics: PhysicsModel) -> bool {
        physics == LEDGER_TM
    }

    fn density_row_is_primary(physics: PhysicsModel) -> bool {
        physics != LEDGER_TE
    }

    /// PH-01's 2 × 1.4 box of PH-01's medium in `config`'s skin, every side
    /// reflecting, with what `config` adds, meshed at `edge` on the
    /// application's path. The scene's point source is off.
    fn ledger_box(config: &Ledgered, edge: f64) -> Arc<crate::topology_runtime::PreparedTopology> {
        prepare(&ledger_document(config), edge)
    }

    fn ledger_document(config: &Ledgered) -> TopologyDocument {
        let medium = BoxMedium::ph01(config.physics);
        let mut builder = Builder::new();
        builder.scene.physics = config.physics;
        builder.scene.geometry.domain = DomainRect {
            min_x: -1.0,
            max_x: 1.0,
            min_y: -0.7,
            max_y: 0.7,
        };
        builder.scene.outer_boundaries =
            OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting);
        if let Some(wall) = config.wall {
            builder.scene.outer_boundaries.sides[OuterSide::Right.index()] = wall;
        }
        let material = &mut builder.scene.materials[0];
        material.mass_density = ScalarField::constant(medium.mass);
        material.stiffness = ScalarField::constant(medium.stiffness);
        material.electric_loss = config.electric.map(LossRate::channel);
        material.magnetic_loss = config.magnetic.map(LossRate::channel);
        if let Some(drive) = config.mass {
            material.mass_law.drive = drive.drive();
        }
        if let Some(drive) = config.stiffness {
            material.stiffness_law.drive = drive.drive();
        }
        if let Some(chi2) = config.kerr {
            material.mass_law.field = FieldLaw::Polynomial {
                chi1: ScalarField::constant(0.0),
                chi2: ScalarField::constant(chi2),
                amplitude_bound: Some(ScalarField::constant(4.0)),
            };
        }
        if let Some(omega0) = config.restoring {
            material.restoring = RestoringLaw::KleinGordon {
                omega0: ScalarField::constant(omega0),
            };
        }
        if let Some((amplitude, hz)) = config.source {
            let region = builder.scene.regions[0].id;
            builder.scene.volume_sources.push(VolumeSource {
                region,
                enabled: true,
                profile: ScalarField::formula(config.profile).unwrap(),
                parameters: vec![],
                signal: TimeSignal::harmonic(0.0, amplitude, hz, 0.0),
            });
        }
        if let Some(stiffness_ratio) = config.gap {
            let spline =
                OpenCubicSpline::polyline(vec![Point2::new(-0.6, 0.1), Point2::new(0.6, 0.1)])
                    .unwrap();
            let gap = SpanBehavior::Separated {
                left: FaceBoundaryCondition::Reflecting,
                right: FaceBoundaryCondition::Reflecting,
                coupling: InternalBoundaryCoupling::ThinGap { stiffness_ratio },
            };
            let behaviors = vec![gap; spline.intervals().len()];
            builder.open_curve(spline, &behaviors);
        }
        let mut document = builder.document();
        document.model.source.enabled = false;
        document
    }

    /// The modes PH-17 starts from, with both `Q` and `b` nonzero.
    const LEDGER_MODES: [(BoxMode, f64); 3] = [
        (BoxMode::ph01(1, 0), 1.0),
        (BoxMode::ph01(0, 1), 0.6),
        (BoxMode::ph01(1, 2), 0.4),
    ];

    /// What a run's state holds at one instant, as the oracle reads it.
    struct LedgerInstant {
        time: f64,
        energy: f64,
        primary: Vec<f64>,
        field: Vec<f64>,
        flux: Vec<Point2>,
        vector: Vec<Point2>,
        jumps: Vec<f64>,
        /// The energy the solver's wall memory holds, `|z|²/2`.
        memory: f64,
    }

    /// Energy by channel over a run, or its rate at an instant: what the
    /// source, the pumps and a gain put in, and what each loss takes out,
    /// each positive in its own direction.
    #[derive(Clone, Copy, Debug, Default)]
    struct Ledger {
        source: f64,
        pump: f64,
        gain: f64,
        primary_loss: f64,
        complementary_loss: f64,
        /// What a wall dissipates, its memory's store apart.
        wall_loss: f64,
    }

    impl Ledger {
        fn net(self) -> f64 {
            self.source + self.pump + self.gain
                - self.primary_loss
                - self.complementary_loss
                - self.wall_loss
        }

        fn lanes(self) -> [(&'static str, f64); 6] {
            [
                ("source", self.source),
                ("pump", self.pump),
                ("gain", self.gain),
                ("primary loss", self.primary_loss),
                ("complementary loss", self.complementary_loss),
                ("wall loss", self.wall_loss),
            ]
        }

        fn add_scaled(&mut self, other: Self, scale: f64) {
            self.source += scale * other.source;
            self.pump += scale * other.pump;
            self.gain += scale * other.gain;
            self.primary_loss += scale * other.primary_loss;
            self.complementary_loss += scale * other.complementary_loss;
            self.wall_loss += scale * other.wall_loss;
        }
    }

    /// A PH-17 run on whichever path the application takes the generation
    /// on: the fixed one for a linear, time-invariant medium, the
    /// time-driven one otherwise.
    enum LedgerRun<'a> {
        Fixed(&'a CanonicalWaveOperator, CanonicalWaveState),
        Driven(
            &'a CanonicalTemporalWaveOperator,
            CanonicalTemporalWaveState,
        ),
    }

    impl<'a> LedgerRun<'a> {
        fn new(
            prepared: &'a crate::topology_runtime::PreparedTopology,
            physics: PhysicsModel,
            integrator: CanonicalIntegrator,
            dt: f64,
        ) -> Self {
            let fixed = mode_state(
                prepared,
                BoxMedium::ph01(physics),
                &LEDGER_MODES,
                0.0,
                integrator,
                dt,
            );
            match prepared.canonical_temporal_operator.as_deref() {
                None => Self::Fixed(&prepared.canonical_operator, fixed),
                Some(operator) => Self::Driven(
                    operator,
                    CanonicalTemporalWaveState::new(
                        operator,
                        dt,
                        fixed.primary_flux().to_vec(),
                        fixed.complementary_flux().to_vec(),
                    )
                    .unwrap()
                    .with_integrator(integrator),
                ),
            }
        }

        /// Steps once and returns the solver's own ledger for the step.
        fn step(&mut self, forcing: &CanonicalForcing) -> Ledger {
            match self {
                Self::Fixed(operator, state) => {
                    let step = state.step_with_forcing(operator, forcing).unwrap();
                    assert_eq!(step.prescribed_exchange, 0.0);
                    Ledger {
                        source: step.source_work,
                        primary_loss: step.primary_loss,
                        complementary_loss: step.complementary_loss,
                        wall_loss: step.boundary_loss,
                        ..Ledger::default()
                    }
                }
                Self::Driven(operator, state) => {
                    let step = state.step_with_forcing(operator, forcing).unwrap();
                    assert_eq!(step.prescribed_exchange, 0.0);
                    assert_eq!(step.short_wave_loss, 0.0);
                    Ledger {
                        source: step.source_work,
                        pump: step.temporal_work,
                        gain: step.active_gain,
                        primary_loss: step.primary_loss,
                        complementary_loss: step.complementary_loss,
                        wall_loss: step.boundary_loss,
                    }
                }
            }
        }

        fn instant(&self) -> LedgerInstant {
            match self {
                Self::Fixed(operator, state) => LedgerInstant {
                    time: state.time(),
                    energy: state.energy(operator).unwrap(),
                    primary: state.primary_flux().to_vec(),
                    field: state.primary_field(operator).unwrap(),
                    flux: state.complementary_flux().to_vec(),
                    vector: state.complementary_field(operator).unwrap(),
                    jumps: match state.auxiliaries() {
                        CanonicalAuxiliaryState::Linear(auxiliaries) => {
                            auxiliaries.thin_gap_jump().to_vec()
                        }
                        _ => Vec::new(),
                    },
                    memory: match state.auxiliaries() {
                        CanonicalAuxiliaryState::Linear(auxiliaries) => {
                            0.5 * auxiliaries.outgoing_z().iter().map(|z| z * z).sum::<f64>()
                        }
                        _ => 0.0,
                    },
                },
                Self::Driven(operator, state) => {
                    let time = state.time();
                    LedgerInstant {
                        time,
                        energy: state.energy(operator).unwrap(),
                        primary: state.primary_flux().to_vec(),
                        field: operator
                            .primary_field_at(state.primary_flux(), time, state.runtime())
                            .unwrap(),
                        flux: state.complementary_flux().to_vec(),
                        vector: operator
                            .complementary_field_at(
                                state.complementary_flux(),
                                time,
                                state.runtime(),
                            )
                            .unwrap(),
                        jumps: state.thin_gap_jump().to_vec(),
                        memory: 0.5
                            * state
                                .outgoing_pole_currents()
                                .iter()
                                .map(|z| z * z)
                                .sum::<f64>(),
                    }
                }
            }
        }
    }

    /// Each channel's power at `instant`, from `config`'s authored laws on
    /// the state's own fields: a flux-rate loss `γ` on a flux takes
    /// `γ · flux · field`, a drive `m(t)` on the coefficient an energy
    /// `flux · field / 2` sits over moves it at `−(∂m/∂t)/m` of itself, and a
    /// source puts its nodal rate times the field in. A harmonic source's
    /// signal is the version-22 acceleration: the rate it drives is its
    /// integral from the generation's start.
    fn ledger_powers(
        config: &Ledgered,
        prepared: &crate::topology_runtime::PreparedTopology,
        instant: &LedgerInstant,
    ) -> Ledger {
        let operator = &prepared.canonical_operator;
        let time = instant.time;
        let (primary_rate, complementary_rate) = if electric_is_primary(config.physics) {
            (config.electric, config.magnetic)
        } else {
            (config.magnetic, config.electric)
        };
        let (primary_drive, complementary_drive) = if density_row_is_primary(config.physics) {
            (config.mass, config.stiffness)
        } else {
            (config.stiffness, config.mass)
        };
        let mut powers = Ledger::default();
        let mut lose = |rate: LossRate, power: f64, primary: bool| {
            if rate.active() {
                powers.gain -= power;
            } else if primary {
                powers.primary_loss += power;
            } else {
                powers.complementary_loss += power;
            }
        };
        // A drive's power at fixed flux is `−(ṁ/m) M Φ(u)`, `Φ = ∫ u g(u) du`
        // under the row's own law `Q = m M g(u) u`: half of `Q u` on a linear
        // row, and on Kerr's density row `(½ + χu²/4)/(1 + χu²)` of it.
        let density_primary = density_row_is_primary(config.physics);
        let share = |density: bool, field: f64| match config.kerr.filter(|_| density) {
            Some(chi) => (0.5 + 0.25 * chi * field * field) / (1.0 + chi * field * field),
            None => 0.5,
        };
        let mut pump = 0.0;
        for (node, point) in operator.node_points().iter().enumerate() {
            let product = instant.primary[node] * instant.field[node];
            if let Some(rate) = primary_rate {
                let field = instant.field[node].abs();
                lose(rate, rate.at(*point, field, time) * product, true);
            }
            if let Some(drive) = primary_drive {
                let (factor, rate) = drive.factor_and_rate(*point, time);
                pump -= rate / factor * share(density_primary, instant.field[node].abs()) * product;
            }
        }
        for (sample, (flux, vector)) in operator
            .constitutive_samples()
            .iter()
            .zip(instant.flux.iter().zip(&instant.vector))
        {
            let product = sample.integration_weight * flux.dot(*vector);
            if let Some(rate) = complementary_rate {
                lose(
                    rate,
                    rate.at(sample.point, vector.norm(), time) * product,
                    false,
                );
            }
            if let Some(drive) = complementary_drive {
                let (factor, rate) = drive.factor_and_rate(sample.point, time);
                pump -= rate / factor * share(!density_primary, vector.norm()) * product;
            }
        }
        powers.pump = pump;
        if let Some((amplitude, hz)) = config.source {
            let omega = std::f64::consts::TAU * hz;
            let rate = amplitude / omega * (1.0 - (omega * time).cos());
            for source in prepared.canonical_forcing.sources() {
                powers.source += rate
                    * source
                        .weights()
                        .iter()
                        .zip(&instant.field)
                        .map(|(weight, field)| weight * field)
                        .sum::<f64>();
            }
        }
        powers
    }

    /// The eigenvalues and eigenvectors (as columns) of a small symmetric
    /// matrix, by cyclic Jacobi rotations.
    fn symmetric_eigen(mut matrix: Vec<Vec<f64>>) -> (Vec<f64>, Vec<Vec<f64>>) {
        let n = matrix.len();
        let mut vectors = (0..n)
            .map(|row| (0..n).map(|column| f64::from(row == column)).collect())
            .collect::<Vec<Vec<f64>>>();
        for _ in 0..100 {
            let off = (0..n)
                .flat_map(|row| {
                    (0..n)
                        .filter(move |column| *column != row)
                        .map(move |column| (row, column))
                })
                .map(|(row, column)| matrix[row][column].powi(2))
                .sum::<f64>();
            let scale = (0..n)
                .map(|index| matrix[index][index].powi(2))
                .sum::<f64>();
            if off <= 1.0e-30 * scale.max(f64::MIN_POSITIVE) {
                break;
            }
            for p in 0..n {
                for q in p + 1..n {
                    if matrix[p][q] == 0.0 {
                        continue;
                    }
                    let theta = (matrix[q][q] - matrix[p][p]) / (2.0 * matrix[p][q]);
                    let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                    let t = if theta == 0.0 { 1.0 } else { t };
                    let c = 1.0 / (t * t + 1.0).sqrt();
                    let s = t * c;
                    for row in matrix.iter_mut() {
                        let (a, b) = (row[p], row[q]);
                        row[p] = c * a - s * b;
                        row[q] = s * a + c * b;
                    }
                    let (upper, lower) = matrix.split_at_mut(q);
                    for (a, b) in upper[p].iter_mut().zip(lower[0].iter_mut()) {
                        let (x, y) = (*a, *b);
                        *a = c * x - s * y;
                        *b = s * x + c * y;
                    }
                    for row in vectors.iter_mut() {
                        let (a, b) = (row[p], row[q]);
                        row[p] = c * a - s * b;
                        row[q] = s * a + c * b;
                    }
                }
            }
        }
        ((0..n).map(|index| matrix[index][index]).collect(), vectors)
    }

    /// PH-05's wall, assembled here from the box's geometry and the authored
    /// medium alone, never from the solver's trace. On the right side
    /// `x = 1` the trace runs vertex, midpoint, vertex along each quadratic
    /// edge. The first-order law damps it by `D = Z ℓ` lumped by Simpson's
    /// weights, `Z = √(m k)` the medium's impedance from its mass slot `m`
    /// and wave stiffness `k`; the second order adds the tangential
    /// stiffness `A = k²/(2Z)` times the edge's line stiffness, so that
    /// `D^{-½} A D^{-½}` has the eigenvalues `λ = c² k_τ²/2`. Each mode `w`
    /// of `T = Vᵀ D^½` drives three pole states
    /// (`docs/spikes/funfern-boundary-auxiliary-spike.md`):
    /// `ẋ = −d diag(0,1,2) x + √d (1,1,1) w`, `d = √(7λ/4)`, which take
    /// `j = w + √d (6/7, −8/7, 2/7)·x` from the field, store `xᵀHx/2` and
    /// dissipate `(w + √d ℓ·x)²`. They are integrated here exactly, the trace
    /// field taken linear between the run's states.
    struct WallOracle {
        nodes: Vec<usize>,
        /// Per mode, `d` and its row of `T`.
        modes: Vec<(f64, Vec<f64>)>,
        poles: Vec<[f64; 3]>,
    }

    impl WallOracle {
        fn new(
            prepared: &crate::topology_runtime::PreparedTopology,
            physics: PhysicsModel,
            order: OuterBoundaryCondition,
        ) -> Self {
            let points = prepared.canonical_operator.node_points();
            let mut nodes = (0..points.len())
                .filter(|node| (points[*node].x - 1.0).abs() < 1.0e-9)
                .collect::<Vec<_>>();
            nodes.sort_by(|a, b| points[*a].y.total_cmp(&points[*b].y));
            assert!(
                nodes.len() % 2 == 1 && nodes.len() >= 3,
                "{} trace nodes",
                nodes.len()
            );
            let medium = BoxMedium::ph01(physics);
            // The mass slot the primary field's flux sits over, and the wave
            // stiffness its gradient is weighed by.
            let (mass, stiffness) = match physics {
                PhysicsModel::Mechanical => (medium.mass, medium.stiffness),
                physics if physics == LEDGER_TM => (medium.mass, 1.0 / medium.stiffness),
                _ => (medium.stiffness, 1.0 / medium.mass),
            };
            let impedance = (mass * stiffness).sqrt();
            let count = nodes.len();
            let mut damping = vec![0.0; count];
            let mut tangential = vec![vec![0.0; count]; count];
            for edge in 0..count / 2 {
                let [first, middle, last] = [2 * edge, 2 * edge + 1, 2 * edge + 2];
                let length = points[nodes[last]].y - points[nodes[first]].y;
                for (local, weight) in [(first, 1.0 / 6.0), (middle, 2.0 / 3.0), (last, 1.0 / 6.0)]
                {
                    damping[local] += impedance * length * weight;
                }
                if order == OuterBoundaryCondition::SecondOrderOutgoing {
                    let scale = stiffness * stiffness / (2.0 * impedance) / (3.0 * length);
                    let line = [[7.0, 1.0, -8.0], [1.0, 7.0, -8.0], [-8.0, -8.0, 16.0]];
                    let local = [first, last, middle];
                    for (row, entries) in line.iter().enumerate() {
                        for (column, entry) in entries.iter().enumerate() {
                            tangential[local[row]][local[column]] += scale * entry;
                        }
                    }
                }
            }
            let scaled = (0..count)
                .map(|row| {
                    (0..count)
                        .map(|column| {
                            tangential[row][column] / (damping[row] * damping[column]).sqrt()
                        })
                        .collect()
                })
                .collect();
            let (eigenvalues, vectors) = symmetric_eigen(scaled);
            let modes = eigenvalues
                .iter()
                .enumerate()
                .map(|(mode, eigenvalue)| {
                    let decay = (1.75 * eigenvalue.max(0.0)).sqrt();
                    let row = (0..count)
                        .map(|node| vectors[node][mode] * damping[node].sqrt())
                        .collect();
                    (decay, row)
                })
                .collect::<Vec<_>>();
            let poles = vec![[0.0; 3]; modes.len()];
            Self {
                nodes,
                modes,
                poles,
            }
        }

        /// Each mode's `w = T u` on the field `u`.
        fn modal(&self, field: &[f64]) -> Vec<f64> {
            self.modes
                .iter()
                .map(|(_, row)| {
                    row.iter()
                        .zip(&self.nodes)
                        .map(|(weight, node)| weight * field[*node])
                        .sum()
                })
                .collect()
        }

        /// The poles carried over `dt` from `before` to `after`, each mode's
        /// `w` linear between them: exact for the pole's linear equation.
        fn advance(&mut self, before: &[f64], after: &[f64], dt: f64) {
            for ((x, (decay, _)), (start, end)) in self
                .poles
                .iter_mut()
                .zip(&self.modes)
                .zip(before.iter().zip(after))
            {
                let root = decay.sqrt();
                for (pole, value) in x.iter_mut().enumerate() {
                    let rate = pole as f64 * decay;
                    let (keep, whole, ramp) = if rate * dt < 1.0e-8 {
                        (1.0, dt, 0.5 * dt)
                    } else {
                        let keep = (-rate * dt).exp();
                        let whole = (1.0 - keep) / rate;
                        (keep, whole, 1.0 / rate - whole / (rate * dt))
                    };
                    *value = keep * *value + root * (start * (whole - ramp) + end * ramp);
                }
            }
        }

        fn weights() -> ([f64; 3], [f64; 3]) {
            let b = (31.0_f64 / 7.0).sqrt();
            (
                [6.0 / 7.0, -8.0 / 7.0, 2.0 / 7.0],
                [0.0, 1.0 - b, 2.0 * b - 4.0],
            )
        }

        /// The poles' store, `Σ xᵀHx/2`.
        fn energy(&self) -> f64 {
            let (_, ell) = Self::weights();
            self.poles
                .iter()
                .map(|x| {
                    let mut energy = 6.0 / 7.0 * x[0] * x[0];
                    for i in 1..3 {
                        for j in 1..3 {
                            energy += 2.0 * ell[i] * ell[j] / (i + j) as f64 * x[i] * x[j];
                        }
                    }
                    0.5 * energy
                })
                .sum()
        }

        /// What the wall dissipates, `Σ (w + √d ℓ·x)²`, and what the field
        /// puts into it, `Σ w j`, at the modes' `w`.
        fn dissipation_and_power(&self, modal: &[f64]) -> (f64, f64) {
            let (current, ell) = Self::weights();
            self.poles.iter().zip(&self.modes).zip(modal).fold(
                (0.0, 0.0),
                |(dissipated, power), ((x, (decay, _)), w)| {
                    let root = decay.sqrt();
                    let dot = |weights: [f64; 3]| (0..3).map(|k| weights[k] * x[k]).sum::<f64>();
                    let leak = w + root * dot(ell);
                    (
                        dissipated + leak * leak,
                        power + w * (w + root * dot(current)),
                    )
                },
            )
        }
    }

    /// One run of a PH-17 configuration at one step.
    struct LedgerLevel {
        dt: f64,
        /// The stored energy at the start.
        start: f64,
        /// The stored energy's change less every channel's independent
        /// integral.
        residual: f64,
        /// The solver's ledger over the run.
        lanes: Ledger,
        /// The oracle's integral of each channel's power, by the trapezoid
        /// over the run's own states.
        oracle: Ledger,
        /// Each gap sample's jump against the trapezoid of the field's
        /// difference across it: the largest gap over the largest jump the
        /// run reached.
        jump_error: f64,
        /// The stored energy at the end.
        end: f64,
        /// The stored energy's largest departure from its start.
        excursion: f64,
        /// The solver's wall memory at the end less the oracle's poles', a
        /// share of the starting energy.
        memory_error: f64,
        /// The steps at which the field took energy back from the wall.
        returned: usize,
        /// The stored energy after each step.
        energies: Vec<f64>,
    }

    fn ledger_level(
        config: &Ledgered,
        prepared: &crate::topology_runtime::PreparedTopology,
        integrator: CanonicalIntegrator,
        dt: f64,
        duration: f64,
    ) -> LedgerLevel {
        let mut run = LedgerRun::new(prepared, config.physics, integrator, dt);
        let steps = (duration / dt).round() as usize;
        let forcing = &prepared.canonical_forcing;
        let gaps = prepared.canonical_operator.thin_gap_samples();
        let mut wall = config
            .wall
            .map(|order| WallOracle::new(prepared, config.physics, order));
        let mut before = run.instant();
        let start = before.energy;
        assert_eq!(before.memory, 0.0);
        let mut modal_before = wall.as_ref().map(|wall| wall.modal(&before.field));
        let wall_before = match (&wall, &modal_before) {
            (Some(wall), Some(modal)) => wall.dissipation_and_power(modal),
            _ => (0.0, 0.0),
        };
        let mut returned = 0;
        let mut energies = Vec::with_capacity(steps);
        let first_jumps = before.jumps.clone();
        let mut integrated_jumps = vec![0.0; gaps.len()];
        let mut powers_before = ledger_powers(config, prepared, &before);
        powers_before.wall_loss = wall_before.0;
        let mut lanes = Ledger::default();
        let mut oracle = Ledger::default();
        let mut excursion = 0.0_f64;
        let mut largest_jump = 0.0_f64;
        for _ in 0..steps {
            lanes.add_scaled(run.step(forcing), 1.0);
            let now = run.instant();
            let mut powers_now = ledger_powers(config, prepared, &now);
            if let (Some(wall), Some(modal)) = (wall.as_mut(), modal_before.as_mut()) {
                let modal_now = wall.modal(&now.field);
                wall.advance(modal, &modal_now, dt);
                let wall_now = wall.dissipation_and_power(&modal_now);
                powers_now.wall_loss = wall_now.0;
                if wall_now.1 < 0.0 {
                    returned += 1;
                }
                *modal = modal_now;
            }
            oracle.add_scaled(powers_before, 0.5 * dt);
            oracle.add_scaled(powers_now, 0.5 * dt);
            for (integral, gap) in integrated_jumps.iter_mut().zip(gaps) {
                let difference = |instant: &LedgerInstant| {
                    instant.field[gap.left_node as usize] - instant.field[gap.right_node as usize]
                };
                *integral += 0.5 * dt * (difference(&before) + difference(&now));
            }
            excursion = excursion.max((now.energy - start).abs());
            energies.push(now.energy);
            largest_jump = now
                .jumps
                .iter()
                .fold(largest_jump, |largest, jump| largest.max(jump.abs()));
            before = now;
            powers_before = powers_now;
        }
        let jump_error = before
            .jumps
            .iter()
            .zip(&first_jumps)
            .zip(&integrated_jumps)
            .fold(0.0_f64, |error, ((last, first), integral)| {
                error.max((last - first - integral).abs())
            })
            / largest_jump.max(f64::MIN_POSITIVE);
        // The oracle's poles hold the wall's store in place of the solver's.
        let poles = wall.as_ref().map_or(0.0, WallOracle::energy);
        LedgerLevel {
            dt,
            start,
            residual: before.energy - before.memory + poles - start - oracle.net(),
            lanes,
            oracle,
            jump_error,
            end: before.energy,
            excursion,
            memory_error: (before.memory - poles) / start,
            returned,
            energies,
        }
    }

    /// Values as `1.234e-5, ...`, for the reports.
    fn scientific(values: &[f64]) -> String {
        values
            .iter()
            .map(|value| format!("{value:.3e}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The order a quantity falls at between two steps a factor two apart.
    fn halving_order(coarse: f64, fine: f64) -> f64 {
        (coarse.abs() / fine.abs()).log2()
    }

    /// Runs `config` for `duration` at the production step, half and a
    /// quarter of it, and reports what each level's ledger and residual are.
    fn ledger_study(
        config: &Ledgered,
        integrator: CanonicalIntegrator,
        edge: f64,
        duration: f64,
    ) -> Vec<LedgerLevel> {
        let prepared = ledger_box(config, edge);
        // The production step is the default integrator's; another takes its
        // own share of the same ceiling.
        let production = prepared.recommended_time_step() * integrator.recommended()
            / CanonicalIntegrator::default().recommended();
        let levels = [1.0, 0.5, 0.25]
            .map(|refinement| {
                let steps = (duration / (production * refinement)).ceil();
                ledger_level(config, &prepared, integrator, duration / steps, duration)
            })
            .into_iter()
            .collect::<Vec<_>>();
        for level in &levels {
            let lanes = level
                .lanes
                .lanes()
                .iter()
                .zip(level.oracle.lanes())
                .filter(|((_, lane), (_, oracle))| *lane != 0.0 || *oracle != 0.0)
                .map(|((name, lane), (_, oracle))| {
                    format!(
                        "{name} {lane:.4e} off {:.2e}",
                        (lane - oracle) / level.start
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            eprintln!(
                "{:?} {} dt {:.3e}: residual {:.3e} of {:.4e}; {lanes}; jumps {:.2e}; memory {:.2e}, returned at {} steps",
                config.physics,
                config.name,
                level.dt,
                level.residual / level.start,
                level.start,
                level.jump_error,
                level.memory_error,
                level.returned,
            );
        }
        levels
    }

    /// The configurations every channel is read in, alone and together.
    fn ledger_configurations() -> Vec<Ledgered> {
        let mut configurations = Vec::new();
        for physics in [PhysicsModel::Mechanical, LEDGER_TM, LEDGER_TE] {
            let plain = |name| Ledgered::plain(name, physics);
            configurations.extend([
                Ledgered {
                    electric: Some(LossRate::Constant(0.3)),
                    ..plain("electric loss")
                },
                Ledgered {
                    magnetic: Some(LossRate::Constant(0.2)),
                    ..plain("magnetic loss")
                },
                Ledgered {
                    electric: Some(LossRate::Rising(0.3)),
                    magnetic: Some(LossRate::Rising(0.2)),
                    ..plain("rising loss")
                },
                Ledgered {
                    mass: Some(RowDrive::Pump {
                        depth: 0.2,
                        hz: 0.7,
                        phase: 0.3,
                    }),
                    ..plain("pumped density")
                },
                Ledgered {
                    stiffness: Some(RowDrive::Pump {
                        depth: 0.2,
                        hz: 0.9,
                        phase: 0.0,
                    }),
                    ..plain("pumped stiffness")
                },
                Ledgered {
                    source: Some((1.5, 0.6)),
                    ..plain("source")
                },
                Ledgered {
                    electric: Some(LossRate::Constant(0.3)),
                    magnetic: Some(LossRate::Constant(0.2)),
                    kerr: Some(0.5),
                    ..plain("lossy Kerr")
                },
                Ledgered {
                    mass: Some(RowDrive::Pump {
                        depth: 0.2,
                        hz: 0.7,
                        phase: 0.3,
                    }),
                    kerr: Some(0.5),
                    ..plain("pumped Kerr")
                },
                Ledgered {
                    magnetic: Some(LossRate::Driven {
                        rate: 0.3,
                        depth: 0.8,
                        hz: 0.8,
                    }),
                    ..plain("driven loss")
                },
                Ledgered {
                    mass: Some(RowDrive::Crystal {
                        depth: 0.15,
                        hz: 0.5,
                        phase: 0.4,
                        sharpness: 3.0,
                    }),
                    ..plain("time crystal")
                },
                Ledgered {
                    stiffness: Some(RowDrive::Travelling {
                        depth: 0.15,
                        hz: 0.6,
                        phase: 0.2,
                        wavenumber: 2.0,
                        angle: 0.5,
                    }),
                    ..plain("travelling modulation")
                },
                Ledgered {
                    gap: Some(40.0),
                    ..plain("gap")
                },
                Ledgered {
                    electric: Some(LossRate::Constant(0.3)),
                    magnetic: Some(LossRate::Rising(0.2)),
                    mass: Some(RowDrive::Pump {
                        depth: 0.2,
                        hz: 0.7,
                        phase: 0.3,
                    }),
                    source: Some((1.5, 0.6)),
                    gap: Some(40.0),
                    ..plain("everything")
                },
            ]);
            // Above the threshold the rate reaches `γ (u²/f₀² − 1)`, 24 times
            // the base on a unit field at 0.2: a base of 0.1 keeps it in the
            // step's asymptotic range at the production step.
            let primary_gain = |name, rate, threshold| {
                let gain = Some(LossRate::VanDerPol { rate, threshold });
                if electric_is_primary(physics) {
                    Ledgered {
                        electric: gain,
                        ..plain(name)
                    }
                } else {
                    Ledgered {
                        magnetic: gain,
                        ..plain(name)
                    }
                }
            };
            configurations.extend([
                primary_gain("van der Pol below", 0.3, 3.0),
                primary_gain("van der Pol above", 0.1, 0.2),
            ]);
        }
        configurations
    }

    /// The share of the starting energy each level's residual and each lane's
    /// distance from its channel may keep at the quarter step.
    const LEDGER_BOUND: f64 = 2.0e-4;

    /// The order the finest pair of levels must show, or the value it must
    /// stand under as roundoff: a lane the solver books by the oracle's own
    /// quadrature meets it there.
    fn second_order(what: &str, values: [f64; 3], start: f64) {
        second_order_under(what, values, start, LEDGER_BOUND);
    }

    fn second_order_under(what: &str, values: [f64; 3], start: f64, bound: f64) {
        let roundoff = 1.0e-12 * start;
        if values[2].abs() < roundoff {
            return;
        }
        let order = halving_order(values[1], values[2]);
        assert!(
            order > 1.7,
            "{what}: {:.3e}, {:.3e}, {:.3e} falls at order {order:.2}",
            values[0],
            values[1],
            values[2]
        );
        assert!(
            values[2].abs() < bound * start,
            "{what}: {:.3e} of {start:.3e} at the quarter step",
            values[2]
        );
    }

    /// A passive or driven configuration: its residual and each lane fall at
    /// second order under the bound.
    fn hold_ledger(config: &Ledgered, levels: &[LedgerLevel]) {
        let what = format!("{:?} {}", config.physics, config.name);
        let start = levels[0].start;
        second_order(
            &format!("{what} residual"),
            [0, 1, 2].map(|index| levels[index].residual),
            start,
        );
        for lane in 0..levels[0].lanes.lanes().len() {
            let (name, _) = levels[0].lanes.lanes()[lane];
            second_order(
                &format!("{what} {name}"),
                [0, 1, 2].map(|index| {
                    levels[index].lanes.lanes()[lane].1 - levels[index].oracle.lanes()[lane].1
                }),
                start,
            );
        }
    }

    /// Van der Pol: the gain lane has the law's sign - gain below the
    /// threshold, loss above it - and what the lane leaves of the oracle's
    /// gain converges to a dissipation, the self-oscillating elements'
    /// short-wave viscosity, which the oracle leaves out.
    fn hold_active(config: &Ledgered, levels: &[LedgerLevel], below: bool) {
        let what = format!("{:?} {}", config.physics, config.name);
        for level in levels {
            assert_eq!(level.lanes.gain > 0.0, below, "{what}: {:?}", level.lanes);
        }
        let residuals = [0, 1, 2].map(|index| levels[index].residual);
        let order = halving_order(residuals[1] - residuals[0], residuals[2] - residuals[1]);
        let limit = residuals[2] + (residuals[2] - residuals[1]) / 3.0;
        eprintln!(
            "{what}: the viscosity takes {:.3e} of the energy",
            limit / levels[0].start
        );
        assert!(
            order > 1.7,
            "{what}: residuals {} settle at order {order:.2}",
            scientific(&residuals)
        );
        assert!(limit < 0.0, "{what}: the viscosity cannot add {limit:.3e}");
    }

    /// A gap's jumps are the time integral of the field's difference across
    /// it, and its springs carry the authored ratio of the wave stiffness
    /// beside it - `k` in Mechanical, `1/μ` in TM and `1/ε` in TE - over the
    /// gap's length, lumped by Simpson's weights on each quadratic edge, less
    /// the lumps at its two tips, where the sides meet and nothing opens.
    fn hold_gap(config: &Ledgered, levels: &[LedgerLevel], ratio: f64) {
        let what = format!("{:?} {}", config.physics, config.name);
        let errors = [0, 1, 2].map(|index| levels[index].jump_error);
        let order = halving_order(errors[1], errors[2]);
        assert!(
            order > 1.7,
            "{what}: jumps {} fall at order {order:.2}",
            scientific(&errors)
        );
        let medium = BoxMedium::ph01(config.physics);
        let wave_stiffness = match config.physics {
            PhysicsModel::Mechanical => medium.stiffness,
            physics if physics == LEDGER_TM => 1.0 / medium.stiffness,
            _ => 1.0 / medium.mass,
        };
        let prepared = ledger_box(config, 0.25);
        let operator = &prepared.canonical_operator;
        let samples = operator.thin_gap_samples();
        let springs = samples.iter().map(|sample| sample.stiffness).sum::<f64>();
        // The gap runs straight from x = -0.6 to 0.6; the trace nodes nearest
        // its tips are the midpoints of its end edges.
        let along = samples
            .iter()
            .map(|sample| operator.node_points()[sample.left_node as usize].x);
        let first = 2.0 * (along.clone().fold(f64::INFINITY, f64::min) + 0.6);
        let last = 2.0 * (0.6 - along.fold(f64::NEG_INFINITY, f64::max));
        let expected = ratio * wave_stiffness * (1.2 - (first + last) / 6.0);
        assert!(
            (springs / expected - 1.0).abs() < 1.0e-9,
            "{what}: springs {springs} against {expected}"
        );
    }

    /// PH-17: every channel's lane against the power its authored law gives
    /// on the run's own states, alone and together, in each skin, at the
    /// production step, its half and its quarter.
    #[test]
    fn every_channel_books_what_its_law_puts_in_or_takes_out() {
        for config in ledger_configurations() {
            let levels = ledger_study(&config, CanonicalIntegrator::default(), 0.25, 3.0);
            match config.electric.or(config.magnetic) {
                Some(LossRate::VanDerPol { threshold, .. }) => {
                    hold_active(&config, &levels, threshold > 1.0)
                }
                _ => hold_ledger(&config, &levels),
            }
            if let Some(ratio) = config.gap {
                hold_gap(&config, &levels, ratio);
            }
        }
    }

    /// PH-17's constant loss in closed form: with the electric and magnetic
    /// rates equal, the losses are `−γ` times the identity in
    /// energy-normalized coordinates and the coupling is skew, so every
    /// field's energy decays as `e^{−2γt}`, whatever modes it holds.
    #[test]
    fn matched_loss_decays_every_field_as_its_closed_form() {
        let rate = 0.4;
        for physics in [PhysicsModel::Mechanical, LEDGER_TM, LEDGER_TE] {
            let config = Ledgered {
                electric: Some(LossRate::Constant(rate)),
                magnetic: Some(LossRate::Constant(rate)),
                ..Ledgered::plain("matched loss", physics)
            };
            let levels = ledger_study(&config, CanonicalIntegrator::default(), 0.25, 3.0);
            let exact = (-2.0 * rate * 3.0_f64).exp();
            let errors = [0, 1, 2].map(|index| levels[index].end / levels[index].start - exact);
            eprintln!(
                "{physics:?} matched loss: E(T)/E(0) off e^(-2γT) by {}",
                scientific(&errors)
            );
            second_order(&format!("{physics:?} matched loss"), errors, 1.0);
        }
    }

    /// PH-17's conservative rows: a linear medium, Kerr and Klein-Gordon keep
    /// their stored energy, its excursion falling four times over the
    /// halving under either integrator: the fourth-order step is fourth order
    /// in phase, and its energy breathes about its conserved store at the
    /// leapfrog's second order (`docs/spikes/funfern-fourth-order-step.md`).
    #[test]
    fn conservative_media_keep_their_energy_to_their_integrators_order() {
        let configurations = [
            Ledgered::plain("linear", PhysicsModel::Mechanical),
            Ledgered {
                kerr: Some(0.5),
                ..Ledgered::plain("Kerr", LEDGER_TM)
            },
            Ledgered {
                restoring: Some(1.5),
                ..Ledgered::plain("Klein-Gordon", LEDGER_TE)
            },
        ];
        for config in configurations {
            for integrator in [
                CanonicalIntegrator::Leapfrog,
                CanonicalIntegrator::FourthOrder,
            ] {
                let levels = ledger_study(&config, integrator, 0.25, 3.0);
                let excursions =
                    [0, 1, 2].map(|index| levels[index].excursion / levels[index].start);
                let order = halving_order(excursions[1], excursions[2]);
                eprintln!(
                    "{:?} {} {integrator:?}: excursion {}, order {order:.2}",
                    config.physics,
                    config.name,
                    scientific(&excursions)
                );
                assert!(
                    order > 1.7,
                    "{:?} {} {integrator:?}: {} at order {order:.2}",
                    config.physics,
                    config.name,
                    scientific(&excursions)
                );
            }
        }
    }

    /// The configurations PH-05 reads its walls in.
    fn wall_configurations() -> Vec<Ledgered> {
        let second = Some(OuterBoundaryCondition::SecondOrderOutgoing);
        let first = Some(OuterBoundaryCondition::FirstOrderOutgoing);
        let mut configurations = Vec::new();
        for physics in [PhysicsModel::Mechanical, LEDGER_TM, LEDGER_TE] {
            let walled = |name, wall| Ledgered {
                wall,
                ..Ledgered::plain(name, physics)
            };
            configurations.extend([
                walled("first-order wall", first),
                walled("second-order wall", second),
                Ledgered {
                    electric: Some(LossRate::Constant(0.3)),
                    ..walled("second-order wall, electric loss", second)
                },
                Ledgered {
                    magnetic: Some(LossRate::Constant(0.2)),
                    ..walled("second-order wall, magnetic loss", second)
                },
                Ledgered {
                    electric: Some(LossRate::Rising(0.3)),
                    magnetic: Some(LossRate::Rising(0.2)),
                    ..walled("second-order wall, loss rising to it", second)
                },
                Ledgered {
                    kerr: Some(0.5),
                    ..walled("second-order wall, Kerr", second)
                },
                Ledgered {
                    mass: Some(RowDrive::Pump {
                        depth: 0.2,
                        hz: 0.7,
                        phase: 0.3,
                    }),
                    ..walled("second-order wall, pumped density", second)
                },
            ]);
        }
        configurations
    }

    /// A slow source beside the second-order wall, varying along it as its
    /// third tangential mode: the field there is a near field at a tenth of
    /// that mode's `d` in Mechanical and a twentieth in TM, where the law's
    /// pole at zero makes the wall store and return rather than absorb.
    fn slow_near_field(physics: PhysicsModel) -> Ledgered {
        Ledgered {
            wall: Some(OuterBoundaryCondition::SecondOrderOutgoing),
            source: Some((0.3, 0.05)),
            profile: "exp(-(x - 0.85) * (x - 0.85) / 0.01) * cos(6.732 * y)",
            ..Ledgered::plain("slow near field", physics)
        }
    }

    /// The share of the starting energy a walled run's residual may keep at
    /// the quarter step: the trapezoid wall step's own split leaves a
    /// second-order remainder (`docs/checks.md`), measured to 2.2e-4.
    const WALL_BOUND: f64 = 5.0e-4;

    /// A walled run: its residual against the oracle - the wall's store taken
    /// as the oracle's poles - falls at second order under the walls' bound,
    /// every lane against its channel under the ledger's, and the solver's
    /// wall memory meets the oracle's poles at second order, to under 1e-5
    /// of the energy.
    fn hold_wall(config: &Ledgered, levels: &[LedgerLevel]) {
        let what = format!("{:?} {}", config.physics, config.name);
        let start = levels[0].start;
        second_order_under(
            &format!("{what} residual"),
            [0, 1, 2].map(|index| levels[index].residual),
            start,
            WALL_BOUND,
        );
        for lane in 0..levels[0].lanes.lanes().len() {
            let (name, _) = levels[0].lanes.lanes()[lane];
            second_order(
                &format!("{what} {name}"),
                [0, 1, 2].map(|index| {
                    levels[index].lanes.lanes()[lane].1 - levels[index].oracle.lanes()[lane].1
                }),
                start,
            );
        }
        second_order_under(
            &format!("{what} memory"),
            [0, 1, 2].map(|index| levels[index].memory_error),
            1.0,
            1.0e-5,
        );
        assert!(
            levels[2].lanes.wall_loss > 0.0,
            "{what}: the wall took nothing"
        );
    }

    /// PH-05: the walls' power, store and dissipation against their law's
    /// own equations, integrated from the field the solver's trace carries,
    /// beside every medium a wall can meet, in each skin, at the production
    /// step, its half and its quarter.
    #[test]
    fn walls_book_what_their_law_takes_beside_every_medium() {
        for config in wall_configurations() {
            let levels = ledger_study(&config, CanonicalIntegrator::default(), 0.25, 3.0);
            hold_wall(&config, &levels);
        }
    }

    /// PH-05 near the law's pole at zero: a slow source beside the
    /// second-order wall, under its third tangential mode's `d`, for one
    /// period of the drive. The wall stores and gives back - the field takes
    /// energy back at a share of the steps that holds as the step halves -
    /// and the ledger closes as anywhere else.
    #[test]
    fn a_slow_near_field_against_the_wall_keeps_its_ledger() {
        for physics in [PhysicsModel::Mechanical, LEDGER_TM] {
            let config = slow_near_field(physics);
            let levels = ledger_study(&config, CanonicalIntegrator::default(), 0.25, 20.0);
            hold_wall(&config, &levels);
            for level in &levels {
                assert!(level.returned > 0, "{physics:?}: the wall never gave back");
            }
        }
    }

    /// One long run of `config` at the production step for `duration`.
    fn long_ledger(
        config: &Ledgered,
        integrator: CanonicalIntegrator,
        duration: f64,
    ) -> LedgerLevel {
        let prepared = ledger_box(config, 0.25);
        let production = prepared.recommended_time_step() * integrator.recommended()
            / CanonicalIntegrator::default().recommended();
        let steps = (duration / production).ceil();
        ledger_level(config, &prepared, integrator, duration / steps, duration)
    }

    /// The largest rise from one window's peak energy to the next's, relative
    /// to the earlier: a passive run's envelope cannot rise. Within a window
    /// the stored energy breathes about its conserved store by the step's own
    /// second-order offset, which late in a run, when what is left is the
    /// mesh's fastest content, is a large share of a small energy; the peaks
    /// of windows longer than that breathing do not see it.
    fn envelope_rise(energies: &[f64], window: usize) -> f64 {
        let peaks = energies
            .chunks(window)
            .map(|chunk| chunk.iter().copied().fold(f64::NEG_INFINITY, f64::max))
            .collect::<Vec<_>>();
        peaks
            .windows(2)
            .map(|pair| (pair[1] - pair[0]) / pair[0])
            .fold(f64::NEG_INFINITY, f64::max)
    }

    /// The energy's largest departure from the start over a stretch of its
    /// record, relative to the start.
    fn excursion_over(level: &LedgerLevel, range: std::ops::Range<usize>) -> f64 {
        level.energies[range].iter().fold(0.0_f64, |band, energy| {
            band.max((energy - level.start).abs())
        }) / level.start
    }

    /// PH-17's long conservative runs: a linear medium in two skins and
    /// Klein-Gordon, a thousand seconds at the production step under either
    /// integrator, 17,000 to 47,000 steps. The energy breathes about its
    /// conserved store and goes nowhere: its excursion over the last quarter
    /// stands where it stood over the first (3.65e-4 under the leapfrog, 4.3e-4
    /// under the fourth-order step, the two within 3e-4 of each other), and
    /// it ends inside that band.
    #[test]
    #[ignore = "nightly: energy-long-runs"]
    fn long_run_conservative_media_hold_their_energy_over_a_thousand_seconds() {
        for config in [
            Ledgered::plain("linear", PhysicsModel::Mechanical),
            Ledgered::plain("linear", LEDGER_TM),
            Ledgered {
                restoring: Some(1.5),
                ..Ledgered::plain("Klein-Gordon", LEDGER_TE)
            },
        ] {
            for integrator in [
                CanonicalIntegrator::Leapfrog,
                CanonicalIntegrator::FourthOrder,
            ] {
                let level = long_ledger(&config, integrator, 1000.0);
                let quarter = level.energies.len() / 4;
                let first = excursion_over(&level, 0..quarter);
                let last = excursion_over(&level, 3 * quarter..level.energies.len());
                let end = level.end / level.start - 1.0;
                let what = format!("{:?} {} {integrator:?}", config.physics, config.name);
                eprintln!(
                    "{what}: excursion {first:.3e} first quarter, {last:.3e} last, end {end:.3e}"
                );
                assert!(
                    last < 1.05 * first,
                    "{what}: {first:.3e} grew to {last:.3e}"
                );
                assert!(
                    end.abs() < first,
                    "{what}: ends {end:.3e} outside {first:.3e}"
                );
            }
        }
    }

    /// A Kerr medium's known drift, held where it stands: a field-dependent
    /// medium gains energy under the splitting at the production step, which
    /// `docs/spikes/funfern-fourth-order-step.md` measured at +5.4e-2 by
    /// t = 160 for Kerr on the density row. PH-17's TM box, its Kerr strong
    /// at χ₂ = 0.5 on a unit field, gains 9.5e-2 by then, and by t = 1000 its
    /// field leaves the declared amplitude bound of 4 and the step is refused.
    /// Held to 0.15 so that a worse drift shows.
    #[test]
    #[ignore = "nightly: energy-long-runs"]
    fn long_run_kerr_gains_no_more_than_its_known_drift() {
        let config = Ledgered {
            kerr: Some(0.5),
            ..Ledgered::plain("Kerr", LEDGER_TM)
        };
        let level = long_ledger(&config, CanonicalIntegrator::FourthOrder, 160.0);
        let gain = level.end / level.start - 1.0;
        eprintln!("TM Kerr by t = 160: {gain:.3e}");
        assert!(gain < 0.15, "Kerr gained {gain:.3e}");
    }

    /// PH-17 and PH-05's long passive runs, 400 s at the production step in
    /// each skin: weak loss, either wall, and a wall beside a gap and a weak
    /// loss. The peak energy of each 20 s window never exceeds the last one's,
    /// measured falling by 7.8e-6 to 26% from one to the next, and the
    /// ledger's residual against the oracle stays at the switch-on's, 2.3e-3
    /// of the energy at most, against 5e-3.
    #[test]
    #[ignore = "nightly: energy-long-runs"]
    fn long_run_passive_media_and_walls_never_raise_their_envelope() {
        let second = Some(OuterBoundaryCondition::SecondOrderOutgoing);
        let first = Some(OuterBoundaryCondition::FirstOrderOutgoing);
        for physics in [PhysicsModel::Mechanical, LEDGER_TM, LEDGER_TE] {
            for config in [
                Ledgered {
                    electric: Some(LossRate::Constant(0.01)),
                    magnetic: Some(LossRate::Rising(0.005)),
                    ..Ledgered::plain("weak loss", physics)
                },
                Ledgered {
                    wall: first,
                    ..Ledgered::plain("first-order wall", physics)
                },
                Ledgered {
                    wall: second,
                    ..Ledgered::plain("second-order wall", physics)
                },
                Ledgered {
                    wall: second,
                    gap: Some(40.0),
                    electric: Some(LossRate::Constant(0.01)),
                    ..Ledgered::plain("wall, gap, weak loss", physics)
                },
            ] {
                let level = long_ledger(&config, CanonicalIntegrator::default(), 400.0);
                let window = (20.0 / level.dt).round() as usize;
                let rise = envelope_rise(&level.energies, window);
                let residual = level.residual / level.start;
                let what = format!("{physics:?} {}", config.name);
                eprintln!("{what}: envelope rise {rise:.3e}, residual {residual:.3e}");
                assert!(rise < 1.0e-4, "{what}: the envelope rose {rise:.3e}");
                assert!(residual.abs() < 5.0e-3, "{what}: residual {residual:.3e}");
            }
        }
    }

    /// PH-05's slow near field for twenty periods of its drive, 400 s: the
    /// wall keeps storing and giving back, its memory meets the oracle's
    /// poles, and the ledger's residual stays at the switch-on's, 2.2e-3.
    #[test]
    #[ignore = "nightly: energy-long-runs"]
    fn long_run_slow_near_field_keeps_its_ledger_over_twenty_periods() {
        for physics in [PhysicsModel::Mechanical, LEDGER_TM] {
            let level = long_ledger(
                &slow_near_field(physics),
                CanonicalIntegrator::default(),
                400.0,
            );
            let residual = level.residual / level.start;
            eprintln!(
                "{physics:?} slow near field: residual {residual:.3e}, memory {:.3e}, returned at {}",
                level.memory_error, level.returned
            );
            assert!(
                residual.abs() < 5.0e-3,
                "{physics:?}: residual {residual:.3e}"
            );
            assert!(
                level.memory_error.abs() < 1.0e-6,
                "{physics:?}: memory {:.3e}",
                level.memory_error
            );
            assert!(level.returned > 0, "{physics:?}: the wall never gave back");
        }
    }

    /// One remeshed run: `config` on edge 0.25 for half the run, then handed
    /// to `target` - the same composition, or one with a law added or taken
    /// away - meshed at 0.18, as the device hands it, component totals where
    /// the runtime transfer puts them, and on, on whichever path each
    /// generation runs. Returns the starting energy, the residual of the
    /// whole run against the oracle's channels and the handoff's booked edit,
    /// and that edit. A wall's loss is read from the run's own lane, which
    /// PH-05 holds to the wall's law.
    fn remesh_level(
        config: &Ledgered,
        target: &Ledgered,
        refinement: f64,
        duration: f64,
    ) -> (f64, f64, f64) {
        let mut runtime = TopologyRuntime::default();
        let (coarse, _) = prepare_on(&mut runtime, &ledger_document(config), 1, 0.25);
        let (fine, maps) = prepare_on(&mut runtime, &ledger_document(target), 2, 0.18);
        let maps = maps.expect("a remesh carries its maps");
        let production = coarse
            .recommended_time_step()
            .min(fine.recommended_time_step());
        let steps = (duration / (production * refinement)).ceil() as usize;
        let dt = duration / steps as f64;
        let mut lanes = Ledger::default();
        let mut oracle = Ledger::default();
        let mut stage = |run: &mut LedgerRun<'_>,
                         prepared: &crate::topology_runtime::PreparedTopology,
                         config: &Ledgered,
                         count: usize| {
            let mut before = run.instant();
            let mut powers_before = ledger_powers(config, prepared, &before);
            for _ in 0..count {
                let step = run.step(&prepared.canonical_forcing);
                oracle.wall_loss += step.wall_loss;
                lanes.add_scaled(step, 1.0);
                let now = run.instant();
                let powers_now = ledger_powers(config, prepared, &now);
                oracle.add_scaled(powers_before, 0.5 * dt);
                oracle.add_scaled(powers_now, 0.5 * dt);
                before = now;
                powers_before = powers_now;
            }
            before.energy
        };
        let mut run = LedgerRun::new(&coarse, config.physics, CanonicalIntegrator::default(), dt);
        let start = run.instant().energy;
        stage(&mut run, &coarse, config, steps / 2);
        let desired = |source: &CanonicalWaveOperator, primary: &[f64]| {
            crate::canonical_gpu::CanonicalGpuRuntimeTransfer::from_primary_transfer(
                source,
                &fine.canonical_operator,
                &coarse.canonical_forcing,
                &fine.canonical_forcing,
                &maps.primary,
                [0; 4],
            )
            .unwrap()
            .desired_totals(source, primary)
        };
        let handoff_maps = CanonicalHandoffMaps {
            primary: &maps.primary,
            complementary: &maps.complementary,
            thin_gap: &maps.thin_gap,
            outgoing: &maps.outgoing,
        };
        let forcing = &fine.canonical_forcing;
        let (mut run, edit, prescribed) = match (run, fine.canonical_temporal_operator.as_deref()) {
            (LedgerRun::Fixed(source, state), None) => {
                let (handed, accounting) = state
                    .handed_over(
                        source,
                        &fine.canonical_operator,
                        forcing,
                        handoff_maps,
                        &desired(source, state.primary_flux()),
                        dt,
                    )
                    .unwrap();
                (
                    LedgerRun::Fixed(&fine.canonical_operator, handed),
                    accounting.edit_exchange,
                    accounting.prescribed_exchange,
                )
            }
            (LedgerRun::Fixed(source, state), Some(target)) => {
                let handoff = CanonicalTemporalWaveState::handed_from_fixed(
                    &state,
                    source,
                    target,
                    forcing,
                    handoff_maps,
                    &desired(source, state.primary_flux()),
                    dt,
                )
                .unwrap();
                (
                    LedgerRun::Driven(target, handoff.state),
                    handoff.accounting.edit_exchange,
                    handoff.accounting.prescribed_exchange,
                )
            }
            (LedgerRun::Driven(source, state), Some(target)) => {
                let handoff = state
                    .handed_over(
                        source,
                        target,
                        forcing,
                        handoff_maps,
                        &desired(source.base(), state.primary_flux()),
                        dt,
                    )
                    .unwrap();
                (
                    LedgerRun::Driven(target, handoff.state),
                    handoff.accounting.edit_exchange,
                    handoff.accounting.prescribed_exchange,
                )
            }
            (LedgerRun::Driven(source, state), None) => {
                let (handed, accounting, _) = CanonicalWaveState::handed_from_driven(
                    &state,
                    source,
                    &fine.canonical_operator,
                    forcing,
                    handoff_maps,
                    &desired(source.base(), state.primary_flux()),
                    dt,
                )
                .unwrap();
                (
                    LedgerRun::Fixed(&fine.canonical_operator, handed),
                    accounting.edit_exchange,
                    accounting.prescribed_exchange,
                )
            }
        };
        assert_eq!(prescribed, 0.0);
        let end = stage(&mut run, &fine, target, steps - steps / 2);
        (start, end - start - oracle.net() - edit, edit)
    }

    /// PH-17's remesh row: a remesh's own change of the stored energy is
    /// booked as the edit lane, and with it the run's whole ledger closes at
    /// second order against the oracle's channels on either generation. The
    /// edit itself is the transfer's error, a small share of the energy.
    #[test]
    fn a_remesh_books_its_own_energy_change_and_the_ledger_closes_across_it() {
        let config = Ledgered {
            electric: Some(LossRate::Constant(0.3)),
            magnetic: Some(LossRate::Constant(0.2)),
            source: Some((1.5, 0.6)),
            gap: Some(40.0),
            ..Ledgered::plain("remeshed", PhysicsModel::Mechanical)
        };
        remeshes_close(&config, &config, true);
    }

    /// The remesh rows on the driven path, and across the handoffs that
    /// change what a generation runs: a pump, Klein-Gordon's `r`, a pump
    /// beside a second-order wall and a gap, each remeshed; a pumped medium
    /// handed a Kerr law; and a pump switched on, from the fixed path, and
    /// off, back to it. Every run's ledger closes at second order with the
    /// edit lane in it; a remesh's edit is its transfer's error, a small share
    /// of the energy, and a law's change books what it changes.
    #[test]
    fn driven_handoffs_book_their_edit_and_the_ledger_closes_across_them() {
        let lossy = Ledgered {
            electric: Some(LossRate::Constant(0.3)),
            magnetic: Some(LossRate::Constant(0.2)),
            source: Some((1.5, 0.6)),
            gap: Some(40.0),
            ..Ledgered::plain("lossy", PhysicsModel::Mechanical)
        };
        let pumped = Ledgered {
            name: "pumped",
            mass: Some(RowDrive::Pump {
                depth: 0.2,
                hz: 0.7,
                phase: 0.3,
            }),
            ..lossy
        };
        let oscillator = Ledgered {
            name: "Klein-Gordon",
            restoring: Some(2.0),
            ..lossy
        };
        let walled = Ledgered {
            name: "walled pump",
            wall: Some(OuterBoundaryCondition::SecondOrderOutgoing),
            ..pumped
        };
        let kerr = Ledgered {
            name: "pumped Kerr",
            kerr: Some(0.5),
            ..pumped
        };
        let rows = [
            (pumped, pumped, true),
            (oscillator, oscillator, true),
            (walled, walled, true),
            (pumped, kerr, false),
            (lossy, pumped, false),
            (pumped, lossy, false),
        ];
        std::thread::scope(|scope| {
            for (config, target, remesh) in &rows {
                scope.spawn(move || remeshes_close(config, target, *remesh));
            }
        });
    }

    /// `config` handed to `target` at three steps: the residual falls at
    /// second order, and the edit is booked - within a hundredth of the
    /// energy where the handoff only remeshes.
    fn remeshes_close(config: &Ledgered, target: &Ledgered, remesh: bool) {
        let levels =
            [1.0, 0.5, 0.25].map(|refinement| remesh_level(config, target, refinement, 3.0));
        let name = format!("{} to {}", config.name, target.name);
        for (start, residual, edit) in &levels {
            eprintln!(
                "{name}: residual {:.3e}, edit {:.3e}, of {start:.4e}",
                residual / start,
                edit / start
            );
        }
        let start = levels[0].0;
        second_order(
            &format!("{name} residual"),
            levels.map(|level| level.1),
            start,
        );
        for (_, _, edit) in levels {
            assert!(edit != 0.0, "{name}: no edit booked");
            if remesh {
                assert!(edit.abs() < 1.0e-2 * start, "{name}: edit {edit:e}");
            }
        }
    }

    // -----------------------------------------------------------------------
    // PH-09: the walls' transient response against their law.
    // -----------------------------------------------------------------------

    /// A wall on PH-06's circle, by its admittance `Y` in the `e^{−iωt}`
    /// convention: a constant impedance wall, or the second-order law at
    /// `s = −iω` with `d = √(7/8) m` for angular order `m` on the unit
    /// circle.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum CircleWall {
        Impedance(f64),
        Second,
    }

    impl CircleWall {
        fn face(self) -> FaceBoundaryCondition {
            match self {
                Self::Impedance(ratio) => FaceBoundaryCondition::Impedance { ratio },
                Self::Second => FaceBoundaryCondition::SecondOrderOutgoing,
            }
        }

        fn admittance(self, m: u32, omega: f64) -> Phasor {
            match self {
                Self::Impedance(ratio) => Phasor::new(ratio, 0.0),
                Self::Second => {
                    let mut admittance = Phasor::new(1.0, 0.0);
                    if m > 0 {
                        let d = (7.0f64 / 8.0).sqrt() * f64::from(m);
                        let s = Phasor::new(0.0, -omega);
                        for (pole, residue) in
                            [(0.0, 6.0 / 7.0), (d, -8.0 / 7.0), (2.0 * d, 2.0 / 7.0)]
                        {
                            admittance = admittance
                                + Phasor::new(residue * d, 0.0) / (s + Phasor::new(pole, 0.0));
                        }
                    }
                    admittance
                }
            }
        }
    }

    /// What `tested`'s record of order `m` on the disk's wall is to
    /// `reference`'s at angular frequency `omega`, in the FFT's `e^{+iωt}`
    /// bins. Inside a disk driven from a ring, order `m` meets the wall of
    /// admittance `Y` - `∂_r u = iωY u` in `e^{−iωt}` - as `u = C J_m(kr)`
    /// outside the ring's reach of `J`, and the ring's jump fixes `C` against
    /// `k J′ − iωY J` at the wall: `u(R) ∝ J(kR) / (J′(kR) − iY J(kR))`, the
    /// factor every wall shares left out. So two walls' records stand in the
    /// ratio `(J′ − iY_r J) / (J′ − iY_t J)` at `kR = ω`, whatever the ring and
    /// however long the disk rings; `J` is the dense table's real part. The
    /// FFT's bins are the conjugate convention.
    fn disk_ratio(m: u32, omega: f64, tested: CircleWall, reference: CircleWall) -> Phasor {
        let (h, dh) = wall_hankel(m, omega);
        let (j, dj) = (Phasor::new(h.re, 0.0), Phasor::new(dh.re, 0.0));
        let i = Phasor::new(0.0, 1.0);
        let stands = |wall: CircleWall| dj - i * wall.admittance(m, omega) * j;
        (stands(reference) / stands(tested)).conj()
    }

    /// The disk's wall nodes on its own side, by angle, each with half the
    /// angle to its two neighbours: the trapezoid's weights for `∫ u cos mθ
    /// dθ` around the circle.
    fn disk_wall_nodes(
        prepared: &crate::topology_runtime::PreparedTopology,
    ) -> Vec<(usize, f64, f64)> {
        let operator = &prepared.canonical_operator;
        let points = operator.node_points();
        let centre = (0..points.len())
            .min_by(|a, b| points[*a].norm().total_cmp(&points[*b].norm()))
            .unwrap();
        let disk = operator.component_labels()[centre];
        let mut nodes = (0..points.len())
            .filter(|node| {
                (points[*node].norm() - 1.0).abs() < 1.0e-3
                    && operator.component_labels()[*node] == disk
            })
            .map(|node| (node, points[node].y.atan2(points[node].x)))
            .collect::<Vec<_>>();
        nodes.sort_by(|a, b| a.1.total_cmp(&b.1));
        let count = nodes.len();
        assert!(count > 64, "{count} wall nodes");
        let tau = std::f64::consts::TAU;
        (0..count)
            .map(|index| {
                let previous = nodes[(index + count - 1) % count].1;
                let next = nodes[(index + 1) % count].1;
                let spread = (next - previous).rem_euclid(tau);
                (nodes[index].0, nodes[index].1, 0.5 * spread)
            })
            .collect()
    }

    /// Each order's `cos mθ` projection of the disk's wall field after each
    /// of `steps` steps of `dt`.
    fn disk_wall_records(
        prepared: &crate::topology_runtime::PreparedTopology,
        orders: &[u32],
        dt: f64,
        steps: usize,
    ) -> Vec<Vec<f64>> {
        let nodes = disk_wall_nodes(prepared);
        let operator = &prepared.canonical_operator;
        let mut state = CanonicalWaveState::zero(operator, dt).unwrap();
        let mut records = vec![Vec::with_capacity(steps); orders.len()];
        for _ in 0..steps {
            state
                .step_with_forcing(operator, &prepared.canonical_forcing)
                .unwrap();
            let field = state.primary_field(operator).unwrap();
            for (record, m) in records.iter_mut().zip(orders) {
                record.push(
                    nodes
                        .iter()
                        .map(|(node, angle, weight)| {
                            weight * field[*node] * (f64::from(*m) * angle).cos()
                        })
                        .sum(),
                );
            }
        }
        records
    }

    /// A record's spectrum, zero-padded to `length`, in the FFT's bins:
    /// `X_k = Σ x_n e^{−iω_k t_n}`.
    fn record_spectrum(record: &[f64], length: usize) -> Vec<Phasor> {
        let mut re = record.to_vec();
        re.resize(length, 0.0);
        let mut im = vec![0.0; length];
        funfern_core::fft(&mut re, &mut im).unwrap();
        re.into_iter()
            .zip(im)
            .map(|(re, im)| Phasor::new(re, im))
            .collect()
    }

    /// One disk comparison: how far `tested`'s spectrum stands from
    /// `reference`'s times their ratio, relative, over the band the pulse
    /// carries, and how far the records had decayed by their end.
    #[derive(Clone, Copy, Debug)]
    struct DiskReading {
        error: f64,
        decayed: f64,
        band: (f64, f64),
    }

    fn disk_compare(
        m: u32,
        tested: (&[f64], CircleWall),
        reference: (&[f64], CircleWall),
        dt: f64,
    ) -> DiskReading {
        let length = (4 * tested.0.len()).next_power_of_two();
        let (spectrum_t, spectrum_r) = (
            record_spectrum(tested.0, length),
            record_spectrum(reference.0, length),
        );
        let resolution = std::f64::consts::TAU / (length as f64 * dt);
        let lowest = f64::from(m).max(1.5);
        let peak = spectrum_r[..length / 2]
            .iter()
            .map(|value| value.norm())
            .fold(0.0, f64::max);
        let (mut difference, mut total) = (0.0, 0.0);
        let mut band = (f64::INFINITY, 0.0_f64);
        for bin in 1..length / 2 {
            let omega = resolution * bin as f64;
            if omega < lowest || omega > 14.0 || spectrum_r[bin].norm() < 1.0e-3 * peak {
                continue;
            }
            band = (band.0.min(omega), band.1.max(omega));
            let predicted = disk_ratio(m, omega, tested.1, reference.1) * spectrum_r[bin];
            difference += (spectrum_t[bin] - predicted).norm().powi(2);
            total += spectrum_t[bin].norm().powi(2);
        }
        let scale = |record: &[f64]| record.iter().fold(0.0_f64, |s, v| s.max(v.abs()));
        let tail = |record: &[f64]| {
            record[record.len() - record.len() / 20..]
                .iter()
                .fold(0.0_f64, |s, v| s.max(v.abs()))
        };
        DiskReading {
            error: (difference / total).sqrt(),
            decayed: (tail(tested.0) / scale(tested.0)).max(tail(reference.0) / scale(reference.0)),
            band,
        }
    }

    /// The disk's floors. The drawn circle is a polygon whose wall nodes sit
    /// 9.5e-6 inside the unit circle on average and 1.9e-5 at most, the same
    /// on every mesh at PH-06's curve tolerance, and the readings stop there:
    /// the second-order wall 3.7e-6 to 1.3e-5 off its law, the `Y = 2` wall
    /// 2.8e-5 to 6.4e-5 off the method's ratio, alike on edges 0.1 and 0.07
    /// and at half the step. Read on a circle of radius `1 − 1e-5` the method
    /// comes 3.2e-5 to 4.9e-5 off, so the floor is the circle's, and the
    /// circle cannot show convergence; the channel does.
    const DISK_LAW_BOUND: f64 = 3.0e-5;
    const DISK_METHOD_BOUND: f64 = 1.5e-4;

    /// Orders 0 and 2 driven at once.
    const DISK_TWO_ORDERS: &str = "1 + (x^2 - y^2) / (x^2 + y^2)";

    /// One disk study: the second-order wall's records against the `Y = 1`
    /// wall's - its law - and, with `method`, the `Y = 2` wall's - the
    /// method's own check, which wants twice the record, since the weaker wall
    /// rings longer.
    #[derive(Clone, Copy, Debug)]
    struct DiskStudy {
        law: DiskReading,
        method: Option<DiskReading>,
    }

    /// The disk rung by a Gaussian pulse at `k0` of `width` seconds driving
    /// `profile` on the ring at `r = 0.35`, on `edge` for `duration` seconds,
    /// read in each of `orders`.
    fn disk_study(
        profile: &str,
        orders: &[u32],
        (k0, width): (f64, f64),
        edge: f64,
        duration: f64,
        method: bool,
    ) -> Vec<DiskStudy> {
        let mut walls = vec![CircleWall::Impedance(1.0), CircleWall::Second];
        if method {
            walls.push(CircleWall::Impedance(2.0));
        }
        let signal = TimeSignal::pulsed(
            [0.0, 1.0, k0 / std::f64::consts::TAU, 0.0],
            PulseEnvelope::Gaussian { width },
            0.0,
            0.0,
        );
        let prepared = walls
            .iter()
            .map(|wall| circle_scene(false, profile, 0.35, wall.face(), signal, edge))
            .collect::<Vec<_>>();
        let dt = prepared
            .iter()
            .map(|prepared| prepared.recommended_time_step())
            .fold(f64::INFINITY, f64::min);
        let steps = (duration / dt).ceil() as usize;
        let records = std::thread::scope(|scope| {
            prepared
                .iter()
                .map(|prepared| scope.spawn(move || disk_wall_records(prepared, orders, dt, steps)))
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        orders
            .iter()
            .enumerate()
            .map(|(index, m)| {
                let record = |wall: usize| (&records[wall][index][..], walls[wall]);
                DiskStudy {
                    law: disk_compare(*m, record(1), record(0), dt),
                    method: method.then(|| disk_compare(*m, record(2), record(0), dt)),
                }
            })
            .collect()
    }

    fn disk_holds(name: &str, orders: &[u32], studies: &[DiskStudy]) {
        for (m, study) in orders.iter().zip(studies) {
            for (what, reading, bound) in [
                ("law", Some(study.law), DISK_LAW_BOUND),
                ("method", study.method, DISK_METHOD_BOUND),
            ] {
                let Some(reading) = reading else {
                    continue;
                };
                eprintln!(
                    "{name}, order {m}: {what} {:.3e} over {:.2}..{:.2}, decayed to {:.1e}",
                    reading.error, reading.band.0, reading.band.1, reading.decayed
                );
                assert!(
                    reading.decayed < 1.0e-4,
                    "{name}, order {m}: the records end at {:e} of their peak",
                    reading.decayed
                );
                assert!(
                    reading.error < bound,
                    "{name}, order {m}: {what} {:e} off",
                    reading.error
                );
            }
        }
    }

    /// PH-09 in the gate: a disk rung in orders 0 and 2 at once, its
    /// second-order wall's records against a `Y = 1` wall's on the coarsest
    /// mesh, 15 s. Order 2 carries the wall's memory; a zero pole leaking a
    /// twentieth of its rate reads 2.7e-3.
    #[test]
    fn a_disk_rung_in_two_orders_at_once_meets_its_wall_law() {
        let studies = disk_study(DISK_TWO_ORDERS, &[0, 2], (6.5, 0.6), 0.1, 15.0, false);
        disk_holds("two orders on 0.1", &[0, 2], &studies);
    }

    /// PH-09's circle (`docs/spikes/funfern-testing-campaign-plan.md`): a
    /// broadband pulse in order 0, a low one (`k0 = 3`, down to the table's
    /// `kR = 1.5`), a narrow one in order 2, and orders 0 and 2 at once, on
    /// edge 0.1, and the two orders on 0.07 as well; the walls `Y = 1`,
    /// `Y = 2` and second order, 30 s, 40 s for the longer pulses. About 15
    /// minutes on an M1 Max.
    #[test]
    #[ignore = "PH-09's circle, about 15 minutes; run in the nightly"]
    fn transient_circle_walls_meet_their_law_in_every_case() {
        for (name, profile, orders, pulse, edge, duration) in [
            (
                "broadband",
                angular_profile(0),
                &[0][..],
                (6.0, 0.6),
                0.1,
                30.0,
            ),
            ("low", angular_profile(0), &[0], (3.0, 1.0), 0.1, 40.0),
            ("narrow", angular_profile(2), &[2], (7.0, 2.5), 0.1, 40.0),
            (
                "two orders",
                DISK_TWO_ORDERS,
                &[0, 2],
                (6.5, 0.6),
                0.1,
                30.0,
            ),
            (
                "two orders",
                DISK_TWO_ORDERS,
                &[0, 2],
                (6.5, 0.6),
                0.07,
                30.0,
            ),
        ] {
            let studies = disk_study(profile, orders, pulse, edge, duration, true);
            disk_holds(&format!("{name} on {edge}"), orders, &studies);
        }
    }

    /// PH-09's channel: PH-04's reflecting channel one unit high, from
    /// `x = −length` to the wall at `x = 1`, its far end second-order
    /// outgoing, launcher strips from `x = −0.7` leftward 0.2 apart, each
    /// driving `profile` under one of `signals` (a region holds one source),
    /// and a sponge in the damping rising smoothly from nothing at
    /// `x = −sponge` to 1 at the far end, slowly enough to return little.
    fn pulse_channel(
        wall: OuterBoundaryCondition,
        profile: &str,
        signals: &[TimeSignal],
        edge: f64,
        length: f64,
        sponge: f64,
    ) -> Arc<crate::topology_runtime::PreparedTopology> {
        let mut builder = Builder::new();
        builder.scene.geometry.domain = DomainRect {
            min_x: -length,
            max_x: 1.0,
            min_y: -0.5,
            max_y: 0.5,
        };
        builder.scene.face_assignments[0].anchor = FaceAnchor::Outer {
            side: OuterSide::Bottom,
            fraction: length / (length + 1.0),
        };
        builder.scene.materials[0].damping =
            ScalarField::formula(format!("smoothstep({sponge}, {length}, -x)")).unwrap();
        let mut boundaries = channel();
        boundaries.sides[OuterSide::Right.index()] = wall;
        builder.scene.outer_boundaries = boundaries;
        for (index, signal) in signals.iter().enumerate() {
            let assigned = builder.scene.face_assignments.len();
            builder.launcher(-0.7 - 0.2 * index as f64, INTERFACE_HZ, 1.0);
            let launcher = builder.scene.volume_sources.last_mut().unwrap();
            launcher.profile = ScalarField::formula(profile).unwrap();
            launcher.signal = *signal;
            // A strip names the outer left face as the one behind it, so
            // the strip before this one takes the face just left of its own
            // divider instead.
            if index > 0 {
                let FaceAnchor::Curve { curve, span, .. } =
                    builder.scene.face_assignments[assigned - 1].anchor
                else {
                    unreachable!("a strip's own face hangs on its divider");
                };
                builder.scene.face_assignments[assigned - 2].anchor = FaceAnchor::Curve {
                    curve,
                    span,
                    side: CurveTraceSide::Left,
                    parameter: 0.5,
                };
            }
        }
        let mut document = builder.document();
        document.model.source.enabled = false;
        prepare(&document, edge)
    }

    /// The implemented law's reflection of channel mode `n` at angular
    /// frequency `omega` (PH-04's `planar_law`, in the FFT's `e^{+iωt}`
    /// bins): `cos θ = √(1 − (nπ/ω)²)` above the mode's cutoff.
    fn channel_law(n: u32, omega: f64, second: bool) -> Phasor {
        let tangential = f64::from(n) * std::f64::consts::PI;
        let cos = (1.0 - (tangential / omega).powi(2)).sqrt();
        planar_law(cos, omega, tangential, second)
    }

    /// One planar transient: each wall's spectrum at the points against
    /// `U_inc + R U_unit` from the reflecting and pinned runs, in units of
    /// the unit reflection - an RMS over the band the pulse carries of
    /// `|R_read − R_law|`, as PH-04 reads one frequency - for the first-
    /// and second-order walls, and how far the records had fallen by their
    /// end.
    #[derive(Clone, Copy, Debug)]
    struct ChannelReading {
        first: f64,
        second: f64,
        quiet: f64,
        band: (f64, f64),
    }

    fn channel_transient(
        n: u32,
        signals: &[TimeSignal],
        edge: f64,
        fraction: f64,
        duration: f64,
        sponge: f64,
    ) -> ChannelReading {
        let walls = [
            OuterBoundaryCondition::FirstOrderOutgoing,
            OuterBoundaryCondition::SecondOrderOutgoing,
            OuterBoundaryCondition::Reflecting,
            OuterBoundaryCondition::Dirichlet {
                signal: TimeSignal::ZERO,
            },
        ];
        let profile = if n == 0 {
            "1".to_owned()
        } else {
            format!("cos({n} * pi * (y + 0.5))")
        };
        let length = 2.0 * sponge;
        let prepared =
            walls.map(|wall| pulse_channel(wall, &profile, signals, edge, length, sponge));
        let dt = fraction
            * prepared
                .iter()
                .map(|prepared| prepared.recommended_time_step())
                .fold(f64::INFINITY, f64::min);
        let steps = (duration / dt).ceil() as usize;
        let quarter = 0.25 / f64::from(n.max(2));
        let points = [0.3, 0.5, 0.7]
            .into_iter()
            .flat_map(|x| [quarter, -quarter, 3.0 * quarter].map(|y| Point2::new(x, y)))
            .collect::<Vec<_>>();
        let traces = std::thread::scope(|scope| {
            prepared
                .iter()
                .map(|prepared| {
                    let points = &points;
                    scope.spawn(move || {
                        wall_traces(prepared, dt, steps, points, CanonicalWallStep::default())
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        let length = (4 * steps).next_power_of_two();
        let resolution = std::f64::consts::TAU / (length as f64 * dt);
        let spectra = traces
            .iter()
            .map(|run| {
                run.iter()
                    .map(|trace| record_spectrum(trace, length))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let lowest = 1.02 * f64::from(n) * std::f64::consts::PI;
        let peak = spectra[2]
            .iter()
            .flat_map(|spectrum| spectrum[..length / 2].iter().map(|value| value.norm()))
            .fold(0.0, f64::max);
        let (mut errors, mut unit) = ([0.0; 2], 0.0);
        let mut band = (f64::INFINITY, 0.0_f64);
        for bin in 1..length / 2 {
            let omega = resolution * bin as f64;
            if omega < lowest
                || spectra[2]
                    .iter()
                    .all(|spectrum| spectrum[bin].norm() < 1.0e-3 * peak)
            {
                continue;
            }
            band = (band.0.min(omega), band.1.max(omega));
            for (point, (open, pinned)) in spectra[2].iter().zip(&spectra[3]).enumerate() {
                let (open, pinned) = (open[bin], pinned[bin]);
                let inc = (open + pinned).scale(0.5);
                let unit_part = (open - pinned).scale(0.5);
                unit += unit_part.norm().powi(2);
                for (index, second) in [false, true].into_iter().enumerate() {
                    let tested = spectra[index][point][bin];
                    let predicted = inc + channel_law(n, omega, second) * unit_part;
                    errors[index] += (tested - predicted).norm().powi(2);
                }
            }
        }
        let quiet = traces
            .iter()
            .flat_map(|run| run.iter())
            .map(|trace| {
                let scale = trace.iter().fold(0.0_f64, |s, v| s.max(v.abs()));
                trace[trace.len() - trace.len() / 20..]
                    .iter()
                    .fold(0.0_f64, |s, v| s.max(v.abs()))
                    / scale
            })
            .fold(0.0, f64::max);
        ChannelReading {
            first: (errors[0] / unit).sqrt(),
            second: (errors[1] / unit).sqrt(),
            quiet,
            band,
        }
    }

    /// The walls on edge 0.1 at the production step: 5.3e-4 to 6.8e-4 in
    /// every case.
    const CHANNEL_BOUND: f64 = 1.0e-3;

    /// A Gaussian pulse of `width` seconds carrying `omega`, `amplitude`
    /// high, starting at `start`.
    fn channel_pulse(omega: f64, width: f64, start: f64, amplitude: f64) -> TimeSignal {
        TimeSignal::pulsed(
            [0.0, amplitude, omega / std::f64::consts::TAU, 0.0],
            PulseEnvelope::Gaussian { width },
            start,
            0.0,
        )
    }

    /// Two broadband pulses in mode 1, the second launched 2 s after the
    /// first has ended, while the first's slow part is still reaching the
    /// wall: a wall that loses or resets its memory between them is caught.
    fn separated_pulses() -> [TimeSignal; 2] {
        [
            channel_pulse(8.0, 1.0, 0.0, 1.0),
            channel_pulse(8.0, 1.0, 10.0, 1.0),
        ]
    }

    fn channel_holds(name: &str, reading: ChannelReading, bound: f64) {
        eprintln!(
            "{name}: first order {:.3e}, second order {:.3e} over {:.2}..{:.2}, quiet to {:.1e}",
            reading.first, reading.second, reading.band.0, reading.band.1, reading.quiet
        );
        assert!(
            reading.quiet < 2.0e-3,
            "{name}: the records end at {:e} of their peak",
            reading.quiet
        );
        for (order, error) in [("first", reading.first), ("second", reading.second)] {
            assert!(
                error < bound,
                "{name}: {order}-order wall {error:e} off its law"
            );
        }
    }

    /// Successive meshes' errors falling at `least` order or faster.
    fn channel_converges(name: &str, edges: &[f64], errors: &[f64], least: f64) {
        for pair in 0..edges.len() - 1 {
            let order =
                (errors[pair] / errors[pair + 1]).ln() / (edges[pair] / edges[pair + 1]).ln();
            assert!(
                order > least,
                "{name}: {errors:?} on {edges:?}, order {order:.2} from edge {}",
                edges[pair]
            );
        }
    }

    /// PH-09 in the gate: separated pulses against the planar law on the
    /// coarsest mesh, 34 s. A zero pole leaking a twentieth of its rate
    /// reads 2.9e-3 (the mutation `wall-memory-leaks`).
    #[test]
    fn separated_pulses_meet_the_planar_law_through_the_walls_memory() {
        let reading = channel_transient(1, &separated_pulses(), 0.1, 1.0, 34.0, 8.0);
        channel_holds("separated pulses on 0.1", reading, CHANNEL_BOUND);
    }

    /// PH-09's channel (`docs/spikes/funfern-testing-campaign-plan.md`):
    /// separated pulses on edges 0.1, 0.07 and 0.05 at the production step
    /// and on 0.07 at half of it; a narrow pulse in mode 2 at PH-04's 45°,
    /// `2π√2`, 2.5 s wide, 40 s; and the late tail, a pulse in mode 1 just
    /// above its cutoff `π`, as copies weighted 1, −2 and 1 two seconds
    /// apart, whose sum has a double zero at `π`: what sits at the cutoff
    /// never leaves the launcher and would ring past any record, and what is
    /// just above crawls to the wall, a fifth of the wave speed at `1.02π`;
    /// in a channel twice as long, so what its sponge returns of that comes
    /// after the 60 s record. The tail's errors, 1.2e-4 and 7.0e-5, fall at
    /// order 1.5 to 1.9 against the others' 2 to 3: its records end at 3.9e-4
    /// of their peak. About 15 minutes on an M1 Max.
    #[test]
    #[ignore = "PH-09's channel, about 15 minutes; run in the nightly"]
    fn transient_planar_walls_converge_to_their_law() {
        let edges = [0.1, 0.07, 0.05];
        let readings = edges.map(|edge| {
            let reading = channel_transient(1, &separated_pulses(), edge, 1.0, 34.0, 8.0);
            channel_holds(
                &format!("separated pulses on {edge}"),
                reading,
                CHANNEL_BOUND,
            );
            reading
        });
        let half = channel_transient(1, &separated_pulses(), 0.07, 0.5, 34.0, 8.0);
        channel_holds(
            "separated pulses on 0.07 at half the step",
            half,
            CHANNEL_BOUND,
        );
        for (order, errors, halved) in [
            ("first", readings.map(|reading| reading.first), half.first),
            (
                "second",
                readings.map(|reading| reading.second),
                half.second,
            ),
        ] {
            channel_converges(
                &format!("separated pulses, {order} order"),
                &edges,
                &errors,
                1.7,
            );
            assert!(
                errors[1] > 2.0 * halved,
                "separated pulses, {order} order: {:.3e} on 0.07, {halved:.3e} at half the step",
                errors[1]
            );
        }
        let narrow = [channel_pulse(
            2.0 * std::f64::consts::PI / std::f64::consts::FRAC_1_SQRT_2,
            2.5,
            0.0,
            1.0,
        )];
        let tail = [
            channel_pulse(4.5, 1.5, 0.0, 1.0),
            channel_pulse(4.5, 1.5, 2.0, -2.0),
            channel_pulse(4.5, 1.5, 4.0, 1.0),
        ];
        for (name, n, signals, duration, sponge, least) in [
            ("narrow", 2, &narrow[..], 40.0, 8.0, 1.7),
            ("late tail", 1, &tail[..], 60.0, 16.0, 1.3),
        ] {
            let edges = [0.1, 0.07];
            let readings = edges.map(|edge| {
                let reading = channel_transient(n, signals, edge, 1.0, duration, sponge);
                channel_holds(&format!("{name} on {edge}"), reading, CHANNEL_BOUND);
                reading
            });
            channel_converges(
                &format!("{name}, first order"),
                &edges,
                &readings.map(|reading| reading.first),
                least,
            );
            channel_converges(
                &format!("{name}, second order"),
                &edges,
                &readings.map(|reading| reading.second),
                least,
            );
        }
    }

    // -----------------------------------------------------------------------
    // PH-08: the rectangle's walls against exact multipoles, a one-off
    // experiment (`experiments/rectangular-radiation`).
    // -----------------------------------------------------------------------

    /// PH-08's box: `[-1, 1]²` of a unit medium, `c = Z = 1`, every side
    /// `wall`, with `sources` over it.
    fn radiation_box(wall: OuterBoundaryCondition, sources: Vec<VolumeSource>) -> TopologyDocument {
        let mut builder = Builder::new();
        builder.scene.volume_sources = sources;
        builder.scene.physics = PhysicsModel::Mechanical;
        builder.scene.geometry.domain = DomainRect {
            min_x: -1.0,
            max_x: 1.0,
            min_y: -1.0,
            max_y: 1.0,
        };
        builder.scene.outer_boundaries = OuterBoundaryConditions::uniform(wall);
        let material = &mut builder.scene.materials[0];
        material.mass_density = ScalarField::constant(1.0);
        material.stiffness = ScalarField::constant(1.0);
        let mut document = builder.document();
        document.model.source.enabled = false;
        document
    }

    /// PH-08, the boundary operator: each wall's trace as production
    /// assembles it on the box, for `boundary.py` - the trace nodes, their
    /// first-order damping, and the second-order wall's modes, each its decay
    /// and its row of `T`.
    #[test]
    #[ignore = "PH-08's experiment: experiments/rectangular-radiation/run.sh"]
    fn export_the_rectangles_wall_traces() {
        let out = std::path::PathBuf::from(
            std::env::var("FUNFERN_PH08_OUT").expect("FUNFERN_PH08_OUT names the output directory"),
        );
        std::fs::create_dir_all(&out).unwrap();
        for (name, wall) in [
            ("first", OuterBoundaryCondition::FirstOrderOutgoing),
            ("second", OuterBoundaryCondition::SecondOrderOutgoing),
        ] {
            for edge in [0.1, 0.07, 0.05] {
                let prepared = prepare(&radiation_box(wall, Vec::new()), edge);
                let operator = &prepared.canonical_operator;
                let points = operator.node_points();
                let damping = operator.first_order_boundary_damping();
                let (trace, modes) = match operator.outgoing_boundary() {
                    Some(boundary) => (
                        boundary
                            .trace_nodes()
                            .iter()
                            .map(|node| *node as usize)
                            .collect::<Vec<_>>(),
                        boundary
                            .modes()
                            .iter()
                            .map(|mode| serde_json::json!({ "decay": mode.decay, "trace": mode.trace() }))
                            .collect::<Vec<_>>(),
                    ),
                    None => (
                        (0..points.len()).filter(|node| damping[*node] > 0.0).collect(),
                        Vec::new(),
                    ),
                };
                let record = serde_json::json!({
                    "wall": name,
                    "edge": edge,
                    "nodes": trace.iter().map(|node| [points[*node].x, points[*node].y]).collect::<Vec<_>>(),
                    "first_order_damping": trace.iter().map(|node| damping[*node]).collect::<Vec<_>>(),
                    "modes": modes,
                });
                std::fs::write(
                    out.join(format!("trace-{name}-{edge}.json")),
                    serde_json::to_string(&record).unwrap(),
                )
                .unwrap();
                eprintln!("{name} on {edge}: {} trace nodes", trace.len());
            }
        }
    }

    /// One source of PH-08's harmonic runs: order `m` about `centre`, of
    /// `amplitude`.
    #[derive(Clone, Copy, Debug)]
    struct Multipole {
        order: u32,
        centre: (f64, f64),
        amplitude: f64,
    }

    /// PH-08's sources' width: their profile is `(r/σ)ᵐ e^{−r²/σ²} cos mθ`
    /// about each centre, so that orders of one amplitude radiate alike.
    const RADIATION_SIGMA: f64 = 0.1;

    /// The profile of `sources`, summed, as a formula in `x` and `y`.
    fn multipole_profile(sources: &[Multipole]) -> String {
        sources
            .iter()
            .map(|source| {
                let (x0, y0) = source.centre;
                let (x, y) = (format!("(x - ({x0}))"), format!("(y - ({y0}))"));
                let sigma2 = RADIATION_SIGMA * RADIATION_SIGMA;
                let angular = match source.order {
                    0 => "1".to_owned(),
                    1 => format!("{x} / {RADIATION_SIGMA}"),
                    2 => format!("({x}^2 - {y}^2) / {sigma2}"),
                    order => panic!("no profile for order {order}"),
                };
                format!(
                    "{} * {angular} * exp(-({x}^2 + {y}^2) / {sigma2})",
                    source.amplitude
                )
            })
            .collect::<Vec<_>>()
            .join(" + ")
    }

    /// PH-08, the full solver: the box walled by `wall`, driven at `k` by
    /// `sources` switched on over two periods, held to steady state for 15 s
    /// and two periods, then every node's `e^{−iωt}` phasor of the primary
    /// field by a Blackman-Harris lock-in over six periods, with the wall's
    /// and the source's time-averaged power over the same window.
    fn harmonic_box_run(
        wall: OuterBoundaryCondition,
        edge: f64,
        k: f64,
        sources: &[Multipole],
    ) -> serde_json::Value {
        let tau = std::f64::consts::TAU;
        let (omega, period) = (k, tau / k);
        let (settle, window) = (15.0 + 2.0 * period, 6.0 * period);
        let held = 2.0 * period + settle + window;
        let source = VolumeSource {
            region: BACKGROUND_REGION,
            enabled: true,
            profile: ScalarField::formula(multipole_profile(sources)).unwrap(),
            parameters: vec![],
            signal: TimeSignal::pulsed(
                [0.0, 1.0, omega / tau, 0.0],
                PulseEnvelope::FlatTop {
                    duration: 2.0 * held,
                    edge: 2.0 * period,
                },
                0.0,
                0.0,
            ),
        };
        let document = radiation_box(wall, vec![source]);
        let prepared = prepare(&document, edge);
        let operator = &prepared.canonical_operator;
        let dt = prepared.recommended_time_step();
        let steps = (held / dt).ceil() as usize;
        let span = (window / dt).round() as usize;
        let first = steps - span;
        let mut state = CanonicalWaveState::zero(operator, dt).unwrap();
        let nodes = operator.degrees_of_freedom();
        let (mut re, mut im) = (vec![0.0; nodes], vec![0.0; nodes]);
        let (mut total, mut wall_energy, mut source_energy) = (0.0, 0.0, 0.0);
        for step in 0..steps {
            let accounting = state
                .step_with_forcing(operator, &prepared.canonical_forcing)
                .unwrap();
            if step < first {
                continue;
            }
            wall_energy += accounting.boundary_loss;
            source_energy += accounting.source_work;
            let x = tau * (step - first) as f64 / span as f64;
            let weight =
                0.35875 - 0.48829 * x.cos() + 0.14128 * (2.0 * x).cos() - 0.01168 * (3.0 * x).cos();
            let (sin, cos) = (omega * (step + 1) as f64 * dt).sin_cos();
            let field = state.primary_field(operator).unwrap();
            for (node, value) in field.iter().enumerate() {
                re[node] += weight * cos * value;
                im[node] += weight * sin * value;
            }
            total += weight;
        }
        // `u = Re(U e^{−iωt})` gives `Σ w u e^{iωt} = U/2 Σ w`.
        let scale = 2.0 / total;
        let duration = span as f64 * dt;
        let points = operator.node_points();
        serde_json::json!({
            "wall": if wall == OuterBoundaryCondition::FirstOrderOutgoing { "first" } else { "second" },
            "edge": edge,
            "k": k,
            "sigma": RADIATION_SIGMA,
            "sources": sources.iter().map(|source| [f64::from(source.order), source.centre.0, source.centre.1, source.amplitude]).collect::<Vec<_>>(),
            // The carrier is `sin ω(t − t_c)`, counted from the envelope's centre.
            "carrier_centre": held,
            "dt": dt,
            "window": duration,
            "nodes": points.iter().map(|point| [point.x, point.y]).collect::<Vec<_>>(),
            "mass": operator.primary_mass(),
            "phasor_re": re.iter().map(|value| value * scale).collect::<Vec<_>>(),
            "phasor_im": im.iter().map(|value| value * scale).collect::<Vec<_>>(),
            "wall_power": wall_energy / duration,
            "source_power": source_energy / duration,
        })
    }

    /// PH-08, the full solver: the harmonic runs `harmonic.py` reads, each
    /// wall at edges 0.1 and 0.07, and 0.05 for the centred and the corner
    /// monopoles; `k` 2, 5 and 10; orders 0 to 2 at the centre, off it, 0.3
    /// from an edge and 0.35 from two, and two sources at once, radiating
    /// alike.
    #[test]
    #[ignore = "PH-08's experiment: experiments/rectangular-radiation/run.sh"]
    fn export_the_rectangles_harmonic_fields() {
        let out = std::path::PathBuf::from(
            std::env::var("FUNFERN_PH08_OUT").expect("FUNFERN_PH08_OUT names the output directory"),
        );
        std::fs::create_dir_all(&out).unwrap();
        let positions = [
            ("centre", (0.0, 0.0)),
            ("off-centre", (0.4, 0.2)),
            ("edge", (0.7, 0.0)),
            ("corner", (0.65, 0.65)),
        ];
        let mut runs = Vec::new();
        for wall in [
            OuterBoundaryCondition::FirstOrderOutgoing,
            OuterBoundaryCondition::SecondOrderOutgoing,
        ] {
            for k in [2.0, 5.0, 10.0] {
                for (name, centre) in positions {
                    for order in 0..=2_u32 {
                        let source = Multipole {
                            order,
                            centre,
                            amplitude: 1.0,
                        };
                        let edges: &[f64] = if order == 0 && (name == "centre" || name == "corner")
                        {
                            &[0.1, 0.07, 0.05]
                        } else {
                            &[0.1, 0.07]
                        };
                        for edge in edges {
                            runs.push((wall, *edge, k, format!("m{order}-{name}"), vec![source]));
                        }
                    }
                }
                let both = vec![
                    Multipole {
                        order: 0,
                        centre: (0.4, 0.2),
                        amplitude: 1.0,
                    },
                    // A Gaussian multipole radiates as `∫ J_m g r dr =
                    // (σ²/2)(kσ/2)ᵐ e^{−(kσ)²/4}`, so the second source is
                    // scaled to send out as much as the first.
                    Multipole {
                        order: 2,
                        centre: (-0.3, -0.5),
                        amplitude: 0.7 * (2.0 / (k * RADIATION_SIGMA)).powi(2),
                    },
                ];
                for edge in [0.1, 0.07] {
                    runs.push((wall, edge, k, "two".to_owned(), both.clone()));
                }
            }
        }
        let wall_name = |wall: &OuterBoundaryCondition| {
            if *wall == OuterBoundaryCondition::FirstOrderOutgoing {
                "first"
            } else {
                "second"
            }
        };
        let file = |(wall, edge, k, name, _): &(
            OuterBoundaryCondition,
            f64,
            f64,
            String,
            Vec<Multipole>,
        )| { format!("field-{}-{edge}-k{k}-{name}.json", wall_name(wall)) };
        // `FUNFERN_PH08_ONLY` keeps the runs whose file name holds it.
        if let Ok(only) = std::env::var("FUNFERN_PH08_ONLY") {
            runs.retain(|run| file(run).contains(&only));
        }
        let next = std::sync::atomic::AtomicUsize::new(0);
        let workers = std::thread::available_parallelism().map_or(4, |count| count.get());
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    loop {
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(run) = runs.get(index) else {
                            break;
                        };
                        let (wall, edge, k, _, sources) = run;
                        let clock = std::time::Instant::now();
                        let record = harmonic_box_run(*wall, *edge, *k, sources);
                        std::fs::write(
                            out.join(file(run)),
                            serde_json::to_string(&record).unwrap(),
                        )
                        .unwrap();
                        eprintln!(
                            "{}: {:.0} s, wall {:.4e}, source {:.4e}",
                            file(run),
                            clock.elapsed().as_secs_f64(),
                            record["wall_power"].as_f64().unwrap(),
                            record["source_power"].as_f64().unwrap()
                        );
                    }
                });
            }
        });
    }

    // -----------------------------------------------------------------------
    // PH-11: planar material interfaces against Fresnel.
    // -----------------------------------------------------------------------

    /// One layer of PH-11's channel, as authored: its mass and stiffness
    /// slots - `ρ` and `k` in Mechanical, `ε` and `μ` in TM and TE - and its
    /// axis ratio, along the channel or, with `across`, its frame turned a
    /// quarter turn.
    #[derive(Clone, Copy, Debug)]
    struct Layer {
        mass: f64,
        stiffness: f64,
        axis_ratio: f64,
        across: bool,
    }

    impl Layer {
        const fn of(mass: f64, stiffness: f64) -> Self {
            Self {
                mass,
                stiffness,
                axis_ratio: 1.0,
                across: false,
            }
        }

        /// The layer's mass and its stiffness along `x` and `y` for the
        /// skin's nodal field, from the physics, not the solver's slots:
        /// Mechanical's displacement, `ρ ü = ∇·(k ∇u)`; TM's `E_z`, `ε` over
        /// `1/μ`; TE's `H_z`, `μ` over `1/ε`. An axis ratio `a` is the ratio
        /// of the wave speeds along its frame's axes, the stiffness `a` times
        /// along it and `1/a` across.
        fn physical(self, physics: PhysicsModel) -> (f64, f64, f64) {
            let (mass, stiffness) = match physics {
                PhysicsModel::Mechanical => (self.mass, self.stiffness),
                physics if physics == LEDGER_TM => (self.mass, 1.0 / self.stiffness),
                _ => (self.stiffness, 1.0 / self.mass),
            };
            let (along, across) = (self.axis_ratio, 1.0 / self.axis_ratio);
            if self.across {
                (mass, stiffness * across, stiffness * along)
            } else {
                (mass, stiffness * along, stiffness * across)
            }
        }
    }

    /// The `x` wavenumber of the channel mode of tangential wavenumber `kt`
    /// in a layer of `mass` and stiffnesses `sx`, `sy` at `omega`: real for a
    /// wave that runs, `−iκ` for one that decays, so that `e^{−ik(x − x₀)}`
    /// runs or falls towards `+x`, `e^{iωt}` in time.
    fn layer_wavenumber(mass: f64, sx: f64, sy: f64, omega: f64, kt: f64) -> Phasor {
        let square = (mass * omega * omega - sy * kt * kt) / sx;
        if square >= 0.0 {
            Phasor::new(square.sqrt(), 0.0)
        } else {
            Phasor::new(0.0, -(-square).sqrt())
        }
    }

    /// `e^{∓ik d}`, the forward wave with `−` and the backward with `+`.
    fn wave(k: Phasor, distance: f64, forward: bool) -> Phasor {
        let sign = if forward { -1.0 } else { 1.0 };
        // ±i k d = ±(i k_re d − k_im d).
        let exponent = Phasor::new(-sign * k.im * distance, sign * k.re * distance);
        let size = exponent.re.exp();
        Phasor::new(size * exponent.im.cos(), size * exponent.im.sin())
    }

    /// Solves `matrix x = rhs` by elimination with partial pivoting.
    fn solve_phasors(mut matrix: Vec<Vec<Phasor>>, mut rhs: Vec<Phasor>) -> Vec<Phasor> {
        let n = rhs.len();
        for column in 0..n {
            let pivot = (column..n)
                .max_by(|a, b| {
                    matrix[*a][column]
                        .norm()
                        .total_cmp(&matrix[*b][column].norm())
                })
                .unwrap();
            matrix.swap(column, pivot);
            rhs.swap(column, pivot);
            for row in column + 1..n {
                let factor = matrix[row][column] / matrix[column][column];
                let pivot_row = matrix[column].clone();
                for (entry, value) in matrix[row].iter_mut().zip(&pivot_row).skip(column) {
                    *entry = *entry - factor * *value;
                }
                let value = rhs[column];
                rhs[row] = rhs[row] - factor * value;
            }
        }
        let mut solution = vec![Phasor::default(); n];
        for row in (0..n).rev() {
            let mut sum = rhs[row];
            for column in row + 1..n {
                sum = sum - matrix[row][column] * solution[column];
            }
            solution[row] = sum / matrix[row][row];
        }
        solution
    }

    /// How two of the channel's layers meet: welded, the field continuous;
    /// across a thin gap of `ratio` (PH-13), its divider drawn upwards or,
    /// with `downward`, from the ceiling down; or cut, two reflecting faces
    /// with nothing between them, the gap's limit as its spring vanishes.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Joint {
        Welded,
        Gap { ratio: f64, downward: bool },
        Cut,
    }

    impl Joint {
        /// The spring per unit length between layers of nodal stiffness
        /// tensors `(sx, sy)` either side: a gap's ratio times the geometric
        /// mean of the two sides' geometric means, as the gap's law scales
        /// it; none for a weld, and nothing for a cut.
        fn spring(self, left: (f64, f64), right: (f64, f64)) -> Option<f64> {
            match self {
                Self::Welded => None,
                Self::Gap { ratio, .. } => {
                    Some(ratio * ((left.0 * left.1).sqrt() * (right.0 * right.1).sqrt()).sqrt())
                }
                Self::Cut => Some(0.0),
            }
        }
    }

    /// The exact channel mode `kt` through `layers` meeting at `interfaces`
    /// by `joints`: in each layer `a e^{−ik(x − x₀)} + b e^{ik(x − x₀)}`, `x₀`
    /// the layer's left interface (the first one for the first layer), the
    /// flux `s_x ∂_x u` continuous at every interface and the field too where
    /// it is welded; across a spring `K` the flux is its tension,
    /// `K (u_right − u_left) = s_x ∂_x u`, which a cut's `K = 0` makes free.
    /// A unit wave in and nothing coming back from the far side; returns each
    /// layer's `(a, b)` and each interface's jump `u_right − u_left`.
    fn layered_amplitudes(
        physics: PhysicsModel,
        layers: &[Layer],
        interfaces: &[f64],
        joints: &[Joint],
        omega: f64,
        kt: f64,
    ) -> (Vec<(Phasor, Phasor)>, Vec<Phasor>) {
        let count = interfaces.len();
        assert_eq!(layers.len(), count + 1);
        assert_eq!(joints.len(), count);
        let properties = layers
            .iter()
            .map(|layer| {
                let (mass, sx, sy) = layer.physical(physics);
                (layer_wavenumber(mass, sx, sy, omega, kt), sx, sy)
            })
            .collect::<Vec<_>>();
        let springs = (0..count)
            .map(|interface| {
                let (_, lx, ly) = properties[interface];
                let (_, rx, ry) = properties[interface + 1];
                joints[interface].spring((lx, ly), (rx, ry))
            })
            .collect::<Vec<_>>();
        let reference = |layer: usize| interfaces[layer.max(1) - 1];
        // Unknowns: R, then each inner layer's a and b, then T.
        let unknowns = 2 * count;
        let index_a = |layer: usize| (layer > 0).then(|| 2 * layer - 1);
        let index_b = |layer: usize| {
            if layer == 0 {
                Some(0)
            } else if layer == count {
                None
            } else {
                Some(2 * layer)
            }
        };
        let mut matrix = vec![vec![Phasor::default(); unknowns]; unknowns];
        let mut rhs = vec![Phasor::default(); unknowns];
        let one = Phasor::new(1.0, 0.0);
        // The flux of a e^{∓ik(x − x₀)} is ∓i k s_x times it.
        let flux_factor = |layer: usize| {
            let (k, sx, _) = properties[layer];
            (Phasor::new(0.0, -1.0) * k).scale(sx)
        };
        for interface in 1..=count {
            let x = interfaces[interface - 1];
            // The field row reads `u_left − u_right` where welded, and
            // `K (u_left − u_right) + s_x ∂_x u_left` across a spring.
            let (weight, tension) =
                springs[interface - 1].map_or((1.0, 0.0), |spring| (spring, 1.0));
            for (side, layer) in [(1.0, interface - 1), (-1.0, interface)] {
                let (k, _, _) = properties[layer];
                let distance = x - reference(layer);
                let forward = wave(k, distance, true);
                let backward = wave(k, distance, false);
                let flux = flux_factor(layer);
                let (field, field_backward) = if side > 0.0 {
                    (
                        Phasor::new(weight, 0.0) + flux.scale(tension),
                        Phasor::new(weight, 0.0) - flux.scale(tension),
                    )
                } else {
                    (Phasor::new(weight, 0.0), Phasor::new(weight, 0.0))
                };
                let row = 2 * (interface - 1);
                let terms = [
                    (
                        row,
                        (field * forward).scale(side),
                        (field_backward * backward).scale(side),
                    ),
                    (
                        row + 1,
                        (flux * forward).scale(side),
                        (flux * backward).scale(-side),
                    ),
                ];
                for (row, forward_term, backward_term) in terms {
                    match index_a(layer) {
                        Some(column) => matrix[row][column] = matrix[row][column] + forward_term,
                        None => rhs[row] = rhs[row] - forward_term,
                    }
                    if let Some(column) = index_b(layer) {
                        matrix[row][column] = matrix[row][column] + backward_term;
                    }
                }
            }
        }
        let solution = solve_phasors(matrix, rhs);
        let amplitudes = (0..=count)
            .map(|layer| {
                let a = index_a(layer).map_or(one, |column| solution[column]);
                let b = index_b(layer).map_or(Phasor::default(), |column| solution[column]);
                (a, b)
            })
            .collect::<Vec<_>>();
        let field = |layer: usize, x: f64| {
            let (k, _, _) = properties[layer];
            let (a, b) = amplitudes[layer];
            a * wave(k, x - reference(layer), true) + b * wave(k, x - reference(layer), false)
        };
        let jumps = (1..=count)
            .map(|interface| {
                let x = interfaces[interface - 1];
                field(interface, x) - field(interface - 1, x)
            })
            .collect();
        (amplitudes, jumps)
    }

    /// The exact reflection and transmission of the channel mode `kt` through
    /// `layers` meeting at `interfaces` by `joints`: the first layer's `b`
    /// and the last layer's `a` of `layered_amplitudes`.
    fn layered_response(
        physics: PhysicsModel,
        layers: &[Layer],
        interfaces: &[f64],
        joints: &[Joint],
        omega: f64,
        kt: f64,
    ) -> (Phasor, Phasor) {
        let (amplitudes, _) = layered_amplitudes(physics, layers, interfaces, joints, omega, kt);
        (amplitudes[0].1, amplitudes.last().unwrap().0)
    }

    /// PH-11's channel for `case`: one unit high from `x = −8` to 8,
    /// reflecting floor and ceiling, second-order walls at both ends behind
    /// sponges of equal electric and magnetic loss rising smoothly from
    /// `|x| = 4` to 2 at the ends, which absorb without an impedance step;
    /// the first layer up to the first interface, each next layer from its
    /// interface on, a welded one transmitting and a gap's divider separated
    /// with its reflecting faces coupled; the launcher at `x = −3.5` driving
    /// the case's mode at its frequency, switched on over `ramp` and held to
    /// `held`.
    fn interface_document(case: &FresnelCase, ramp: f64, held: f64) -> TopologyDocument {
        let FresnelCase {
            physics,
            layers,
            interfaces,
            joints,
            mode,
            omega,
            ..
        } = case;
        let (physics, mode, omega) = (*physics, *mode, *omega);
        let mut builder = Builder::new();
        builder.scene.physics = physics;
        builder.scene.geometry.domain = DomainRect {
            min_x: -8.0,
            max_x: 8.0,
            min_y: -0.5,
            max_y: 0.5,
        };
        // The background, the first layer, between the launcher and the first
        // interface.
        builder.scene.face_assignments[0].anchor = FaceAnchor::Outer {
            side: OuterSide::Bottom,
            fraction: (8.0 - 1.5) / 16.0,
        };
        let mut boundaries = channel();
        boundaries.sides[OuterSide::Left.index()] = OuterBoundaryCondition::SecondOrderOutgoing;
        boundaries.sides[OuterSide::Right.index()] = OuterBoundaryCondition::SecondOrderOutgoing;
        builder.scene.outer_boundaries = boundaries;
        // A loss rate is read in its region's frame: a quarter turn puts the
        // channel's x on the frame's y.
        let sponge = |across: bool| LossChannel {
            base_rate: ScalarField::formula(if across {
                "2 * smoothstep(4, 8, abs(y))"
            } else {
                "2 * smoothstep(4, 8, abs(x))"
            })
            .unwrap(),
            law: DampingLaw {
                rate: RateLaw::Constant,
                drive: TimeDrive::None,
                gate: None,
            },
        };
        let author = |material: &mut Material, layer: Layer| {
            material.mass_density = ScalarField::constant(layer.mass);
            material.stiffness = ScalarField::constant(layer.stiffness);
            material.axis_ratio = ScalarField::constant(layer.axis_ratio);
            material.electric_loss = Some(sponge(layer.across));
            material.magnetic_loss = Some(sponge(layer.across));
        };
        assert!(!layers[0].across && layers[0].axis_ratio == 1.0);
        author(&mut builder.scene.materials[0], layers[0]);
        for (index, ((layer, x), joint)) in
            layers[1..].iter().zip(interfaces).zip(joints).enumerate()
        {
            let material = MaterialId(10 + index as u64);
            let mut authored = Material {
                id: material,
                name: format!("Layer {}", index + 2),
                ..Material::default_medium()
            };
            author(&mut authored, *layer);
            builder.scene.materials.push(authored);
            let (curve, span) = match *joint {
                Joint::Welded => builder.divider(*x),
                Joint::Gap { ratio, downward } => builder.divider_as(
                    *x,
                    SpanBehavior::Separated {
                        left: FaceBoundaryCondition::Reflecting,
                        right: FaceBoundaryCondition::Reflecting,
                        coupling: InternalBoundaryCoupling::ThinGap {
                            stiffness_ratio: ratio,
                        },
                    },
                    downward,
                ),
                Joint::Cut => builder.divider_as(
                    *x,
                    SpanBehavior::Separated {
                        left: FaceBoundaryCondition::Reflecting,
                        right: FaceBoundaryCondition::Reflecting,
                        coupling: InternalBoundaryCoupling::Independent,
                    },
                    false,
                ),
            };
            let downward = matches!(joint, Joint::Gap { downward: true, .. });
            let region = RegionId(builder.next_region);
            builder.next_region += 1;
            builder.scene.regions.push(Region {
                id: region,
                material,
                frame: MaterialFrame {
                    angle_radians: if layer.across {
                        std::f64::consts::FRAC_PI_2
                    } else {
                        0.0
                    },
                    ..MaterialFrame::world()
                },
            });
            // Running upwards, the divider's right is towards +x; running
            // down, its left.
            builder.scene.face_assignments.push(AuthoredFaceAssignment {
                anchor: FaceAnchor::Curve {
                    curve,
                    span,
                    side: if downward {
                        CurveTraceSide::Left
                    } else {
                        CurveTraceSide::Right
                    },
                    parameter: 0.5,
                },
                region: Some(region),
            });
        }
        builder.launcher(-3.5, INTERFACE_HZ, 1.0);
        let launcher = builder.scene.volume_sources.last_mut().unwrap();
        launcher.signal = TimeSignal::pulsed(
            [0.0, 1.0, omega / std::f64::consts::TAU, 0.0],
            PulseEnvelope::FlatTop {
                duration: 2.0 * held,
                edge: ramp,
            },
            0.0,
            0.0,
        );
        launcher.profile = ScalarField::formula(if mode == 0 {
            "1".to_owned()
        } else {
            format!("cos({mode} * pi * (y + 0.5))")
        })
        .unwrap();
        let mut document = builder.document();
        document.model.source.enabled = false;
        document
    }

    /// What one Fresnel run reads: the first layer's forward and backward
    /// waves `A` and `B`, phases at the first interface, the last layer's
    /// `C` and `D` at the last, and the largest share of the mode's amplitude
    /// either side's two-wave fit leaves unexplained; at each gap, what its
    /// memory reads.
    #[derive(Clone, Debug)]
    struct InterfaceReading {
        incident: Phasor,
        reflected: Phasor,
        forward: Phasor,
        returned: Phasor,
        misfit: f64,
        gaps: Vec<GapReading>,
    }

    /// A gap's memory, the jump `ψ_left − ψ_right` of the field's time
    /// integral at each of its samples, read as the projections are: its
    /// lock-in phasors fitted to the mode's profile along the gap, weighted
    /// by the samples' springs; and the springs' energy `½ Σ K_i j_i²`
    /// averaged over the lock-in's window by its weights.
    #[derive(Clone, Copy, Debug)]
    struct GapReading {
        jump: Phasor,
        energy: f64,
    }

    /// The timing of a run of `case`: a switch-on rings at each layer's
    /// cutoff `kt √(s_y/m)`, where the mode stands still, so the drive's own
    /// switch-on is made slow beside that frequency, and the lock-in long
    /// enough to keep it four Blackman-Harris bins away; a slow mode takes
    /// long to cross. Returns the switch-on, the settle and the window.
    fn fresnel_timing(case: &FresnelCase) -> (f64, f64, f64) {
        let tau = std::f64::consts::TAU;
        let (omega, kt) = (case.omega, case.kt());
        let properties = case
            .layers
            .iter()
            .map(|layer| layer.physical(case.physics))
            .collect::<Vec<_>>();
        let beside = properties
            .iter()
            .map(|(mass, _, sy)| (omega - kt * (sy / mass).sqrt()).abs())
            .filter(|gap| kt > 0.0 && *gap > 0.0)
            .fold(f64::INFINITY, f64::min);
        let slowest = [properties[0], *properties.last().unwrap()]
            .iter()
            .filter_map(|(mass, sx, sy)| {
                let k = layer_wavenumber(*mass, *sx, *sy, omega, kt);
                (k.im == 0.0).then(|| sx * k.re / (mass * omega))
            })
            .fold(f64::INFINITY, f64::min);
        let ramp = (16.0 / beside).max(4.0);
        let window = (90.0 / beside).max(8.0 * tau / omega);
        let settle = 10.0 + 14.0 / slowest;
        (ramp, settle, window)
    }

    /// Where a run of `case` reads: every 0.11, which no wavenumber here
    /// turns into a multiple of π, where a forward and a backward wave would
    /// sample alike, from 0.25 off each outer interface to 3 away, at eight
    /// heights. Returns the positions before the first interface and after
    /// the last, and the points.
    fn fresnel_points(case: &FresnelCase) -> (Vec<f64>, Vec<f64>, Vec<Point2>) {
        let heights = 8;
        let first = case.interfaces[0];
        let last = *case.interfaces.last().unwrap();
        let before = (0..)
            .map(|i| -3.0 + 0.11 * f64::from(i))
            .take_while(|x| *x <= first - 0.25 + 1.0e-9)
            .collect::<Vec<_>>();
        let after = (0..)
            .map(|i| last + 0.25 + 0.11 * f64::from(i))
            .take_while(|x| *x <= 3.0 + 1.0e-9)
            .collect::<Vec<_>>();
        let points = before
            .iter()
            .chain(&after)
            .flat_map(|x| {
                (0..heights)
                    .map(move |j| Point2::new(*x, -0.5 + (f64::from(j) + 0.5) / f64::from(heights)))
            })
            .collect::<Vec<_>>();
        (before, after, points)
    }

    /// One PH-11 run on `edge` at `fraction` of the production step: held to
    /// steady state and read by `fresnel_read`.
    fn fresnel_run(case: &FresnelCase, edge: f64, fraction: f64) -> InterfaceReading {
        let (ramp, settle, window) = fresnel_timing(case);
        let held = ramp + settle + window;
        let prepared = prepare(&interface_document(case, ramp, held), edge);
        let (_, _, points) = fresnel_points(case);
        let dt = fraction * prepared.recommended_time_step();
        let steps = (held / dt).ceil() as usize;
        let (traces, gap_traces) =
            field_and_gap_traces(&prepared, dt, steps, &points, CanonicalWallStep::default());
        fresnel_read(
            case,
            &prepared.canonical_operator,
            &traces,
            &gap_traces,
            dt,
            dt,
            window,
        )
    }

    /// Reads `case` from `traces` at its points and `gap_traces` at
    /// `operator`'s gap samples, recorded every `dt` from `first`: the last
    /// `window` of each read by a Blackman-Harris lock-in and the field's
    /// projected on the mode; on the first and the last layer the
    /// projections are fitted in least squares to a forward and a backward
    /// wave of the layer's exact wavenumber.
    fn fresnel_read(
        case: &FresnelCase,
        operator: &CanonicalWaveOperator,
        traces: &[Vec<f64>],
        gap_traces: &[Vec<f64>],
        dt: f64,
        first: f64,
        window: f64,
    ) -> InterfaceReading {
        let FresnelCase {
            physics,
            layers,
            interfaces,
            mode,
            omega,
            ..
        } = case;
        let (physics, mode, omega) = (*physics, *mode, *omega);
        let tau = std::f64::consts::TAU;
        let kt = case.kt();
        let heights = 8;
        let (before, after, _) = fresnel_points(case);
        let (first_interface, last_interface) = (interfaces[0], *interfaces.last().unwrap());
        let steps = traces[0].len();
        let span = (window / dt).round() as usize;
        let start = steps - span;
        let window_weight = |step: usize| {
            let x = tau * (step - start) as f64 / span as f64;
            0.35875 - 0.48829 * x.cos() + 0.14128 * (2.0 * x).cos() - 0.01168 * (3.0 * x).cos()
        };
        let lock_in = |trace: &[f64]| {
            let (mut sum, mut total) = (Phasor::default(), 0.0);
            for (step, value) in trace.iter().enumerate().skip(start) {
                let weight = window_weight(step);
                let phase = omega * (first + step as f64 * dt);
                sum = sum + Phasor::new(phase.cos(), -phase.sin()).scale(weight * value);
                total += weight;
            }
            sum.scale(1.0 / total)
        };
        let gaps = interfaces
            .iter()
            .zip(&case.joints)
            .filter(|(_, joint)| matches!(joint, Joint::Gap { .. }))
            .map(|(x, _)| {
                let samples = operator
                    .thin_gap_samples()
                    .iter()
                    .enumerate()
                    .filter(|(_, sample)| {
                        (operator.node_points()[sample.left_node as usize].x - x).abs() < 1.0e-9
                    })
                    .collect::<Vec<_>>();
                assert!(!samples.is_empty(), "no gap samples at {x}");
                let (mut along, mut norm) = (Phasor::default(), 0.0);
                for (index, sample) in &samples {
                    let y = operator.node_points()[sample.left_node as usize].y;
                    let profile = (kt * (y + 0.5)).cos();
                    along = along + lock_in(&gap_traces[*index]).scale(sample.stiffness * profile);
                    norm += sample.stiffness * profile * profile;
                }
                let (energy, total) = (start..steps).fold((0.0, 0.0), |(energy, total), step| {
                    let weight = window_weight(step);
                    let stored = samples
                        .iter()
                        .map(|(index, sample)| {
                            0.5 * sample.stiffness * gap_traces[*index][step].powi(2)
                        })
                        .sum::<f64>();
                    (energy + weight * stored, total + weight)
                });
                GapReading {
                    jump: along.scale(1.0 / norm),
                    energy: energy / total,
                }
            })
            .collect();
        let weight = if mode == 0 { 1.0 } else { 2.0 } / f64::from(heights);
        let projections = traces
            .chunks(heights as usize)
            .map(|column| {
                column
                    .iter()
                    .zip(0..heights)
                    .fold(Phasor::default(), |sum, (trace, j)| {
                        let y = -0.5 + (f64::from(j) + 0.5) / f64::from(heights);
                        sum + lock_in(trace).scale(weight * (kt * (y + 0.5)).cos())
                    })
            })
            .collect::<Vec<_>>();
        let fit = |xs: &[f64], values: &[Phasor], layer: Layer, reference: f64| {
            let (mass, sx, sy) = layer.physical(physics);
            let k = layer_wavenumber(mass, sx, sy, omega, kt);
            let basis = |x: f64| [wave(k, x - reference, true), wave(k, x - reference, false)];
            let mut gram = vec![vec![Phasor::default(); 2]; 2];
            let mut rhs = vec![Phasor::default(); 2];
            for (x, value) in xs.iter().zip(values) {
                let functions = basis(*x);
                for row in 0..2 {
                    for column in 0..2 {
                        gram[row][column] =
                            gram[row][column] + functions[row].conj() * functions[column];
                    }
                    rhs[row] = rhs[row] + functions[row].conj() * *value;
                }
            }
            let amplitudes = solve_phasors(gram, rhs);
            let (mut residual, mut size) = (0.0_f64, 0.0_f64);
            for (x, value) in xs.iter().zip(values) {
                let functions = basis(*x);
                let model = amplitudes[0] * functions[0] + amplitudes[1] * functions[1];
                residual = residual.max((model - *value).norm());
                size = size.max(value.norm());
            }
            (amplitudes[0], amplitudes[1], residual / size)
        };
        let (incident, reflected, misfit_before) = fit(
            &before,
            &projections[..before.len()],
            layers[0],
            first_interface,
        );
        let (forward, backward, misfit_after) = fit(
            &after,
            &projections[before.len()..],
            *layers.last().unwrap(),
            last_interface,
        );
        InterfaceReading {
            incident,
            reflected,
            forward,
            returned: backward,
            misfit: misfit_before.max(misfit_after),
            gaps,
        }
    }

    /// One of PH-11's or PH-13's cases: the skin, the layers, their
    /// interfaces and how each joins its layers, the channel mode and the
    /// frequency.
    #[derive(Clone, Debug)]
    struct FresnelCase {
        name: String,
        physics: PhysicsModel,
        layers: Vec<Layer>,
        interfaces: Vec<f64>,
        joints: Vec<Joint>,
        mode: u32,
        omega: f64,
    }

    /// How far a reading stands from the exact scattering: `B` against
    /// `R A + T′ D` and `C` against `T A + R′ D`, relative to `A`; the power
    /// balance's defect; across a single interface, how far the field and
    /// its flux `s_x ∂_x u` jump there beside the joint's law, the field's
    /// jump the flux over the spring across a gap; the reading's own misfit,
    /// which also carries the discrete wave's small error in wavenumber over
    /// the three units read; and at the worst gap, its memory's jump against
    /// the exact one the measured `A` and `D` drive, relative to `A/ω`, and
    /// its springs' energy against that jump's, relative.
    #[derive(Clone, Copy, Debug)]
    struct FresnelErrors {
        reflection: f64,
        transmission: f64,
        power: f64,
        field_jump: Option<f64>,
        flux_jump: Option<f64>,
        misfit: f64,
        gap_jump: Option<f64>,
        gap_energy: Option<f64>,
    }

    impl FresnelCase {
        fn kt(&self) -> f64 {
            f64::from(self.mode) * std::f64::consts::PI
        }

        fn exact(&self) -> (Phasor, Phasor) {
            layered_response(
                self.physics,
                &self.layers,
                &self.interfaces,
                &self.joints,
                self.omega,
                self.kt(),
            )
        }

        /// The stack mirrored, `x` to `−x`: met from the left, it is the stack
        /// met from the right, its phases at the same interfaces.
        fn mirrored(&self) -> (Vec<Layer>, Vec<f64>, Vec<Joint>) {
            (
                self.layers.iter().rev().copied().collect(),
                self.interfaces.iter().rev().map(|x| -x).collect(),
                self.joints.iter().rev().copied().collect(),
            )
        }

        /// `R′` and `T′`, the stack met from the right.
        fn exact_from_the_right(&self) -> (Phasor, Phasor) {
            let (layers, interfaces, joints) = self.mirrored();
            layered_response(
                self.physics,
                &layers,
                &interfaces,
                &joints,
                self.omega,
                self.kt(),
            )
        }

        /// A layer's wavenumber and flux stiffness `s_x`.
        fn layer(&self, layer: Layer) -> (Phasor, f64) {
            let (mass, sx, sy) = layer.physical(self.physics);
            (layer_wavenumber(mass, sx, sy, self.omega, self.kt()), sx)
        }

        /// The exact jumps `u_right − u_left` at every interface that the
        /// waves `a` in from the left and `d` in from the right drive.
        fn exact_jumps(&self, a: Phasor, d: Phasor) -> Vec<Phasor> {
            let kt = self.kt();
            let (_, from_the_left) = layered_amplitudes(
                self.physics,
                &self.layers,
                &self.interfaces,
                &self.joints,
                self.omega,
                kt,
            );
            let (layers, interfaces, joints) = self.mirrored();
            let (_, mirrored) =
                layered_amplitudes(self.physics, &layers, &interfaces, &joints, self.omega, kt);
            // Mirrored, right and left swap and so does the jump's sign.
            from_the_left
                .iter()
                .zip(mirrored.iter().rev())
                .map(|(left, right)| *left * a - *right * d)
                .collect()
        }

        fn errors(&self, reading: InterfaceReading) -> FresnelErrors {
            let (r, t) = self.exact();
            let (r_back, t_back) = self.exact_from_the_right();
            let a = reading.incident;
            let scale = a.norm();
            let reflection = (reading.reflected - r * a - t_back * reading.returned).norm() / scale;
            let transmission = (reading.forward - t * a - r_back * reading.returned).norm() / scale;
            let (k_first, s_first) = self.layer(self.layers[0]);
            let (k_last, s_last) = self.layer(*self.layers.last().unwrap());
            let into = s_first * k_first.re * (a.norm().powi(2) - reading.reflected.norm().powi(2));
            let out = if k_last.im == 0.0 {
                s_last
                    * k_last.re
                    * (reading.forward.norm().powi(2) - reading.returned.norm().powi(2))
            } else {
                0.0
            };
            let power = (into - out).abs() / (s_first * k_first.re * scale * scale);
            let single = self.interfaces.len() == 1;
            let flux = |k: Phasor, s: f64, forward: Phasor, backward: Phasor| {
                (Phasor::new(0.0, -1.0) * k * (forward - backward)).scale(s)
            };
            let spring = |interface: usize| {
                let (_, lx, ly) = self.layers[interface].physical(self.physics);
                let (_, rx, ry) = self.layers[interface + 1].physical(self.physics);
                self.joints[interface].spring((lx, ly), (rx, ry))
            };
            // The memory reads `ψ_left − ψ_right` of the divider, its left
            // towards −x running upwards, and `ψ = u/(iω)`.
            let jumps = self.exact_jumps(a, reading.returned);
            let (mut gap_jump, mut gap_energy) = (None::<f64>, None::<f64>);
            let profile = if self.mode == 0 { 1.0 } else { 0.5 };
            let gaps =
                (0..self.interfaces.len()).filter_map(|interface| match self.joints[interface] {
                    Joint::Gap { downward, .. } => Some((interface, downward)),
                    Joint::Welded | Joint::Cut => None,
                });
            for ((interface, downward), read) in gaps.zip(&reading.gaps) {
                let sign = if downward { 1.0 } else { -1.0 };
                let exact = (jumps[interface] / Phasor::new(0.0, self.omega)).scale(sign);
                let error = (read.jump - exact).norm() / (scale / self.omega);
                gap_jump = Some(gap_jump.map_or(error, |worst| worst.max(error)));
                let stored = spring(interface).unwrap() * exact.norm().powi(2) * profile;
                let error = (read.energy - stored).abs() / stored;
                gap_energy = Some(gap_energy.map_or(error, |worst| worst.max(error)));
            }
            FresnelErrors {
                reflection,
                transmission,
                power,
                field_jump: (single && self.joints[0] != Joint::Cut).then(|| {
                    let law = spring(0).map_or(Phasor::default(), |spring| {
                        flux(k_first, s_first, a, reading.reflected).scale(1.0 / spring)
                    });
                    ((reading.forward + reading.returned) - (a + reading.reflected) - law).norm()
                        / scale
                }),
                flux_jump: single.then(|| {
                    (flux(k_first, s_first, a, reading.reflected)
                        - flux(k_last, s_last, reading.forward, reading.returned))
                    .norm()
                        / (s_first * k_first.norm() * scale)
                }),
                misfit: reading.misfit,
                gap_jump,
                gap_energy,
            }
        }

        fn run(&self, edge: f64, fraction: f64) -> InterfaceReading {
            fresnel_run(self, edge, fraction)
        }
    }

    /// PH-11's cases, in each skin.
    fn interface_cases() -> Vec<FresnelCase> {
        let pi = std::f64::consts::PI;
        let tau = std::f64::consts::TAU;
        // The mode at `degrees` in a layer of index `n`: `sin θ = nπ/(n ω)`.
        let at = |degrees: f64, index: f64| pi / (index * degrees.to_radians().sin());
        let mut cases = Vec::new();
        for (skin, physics) in [
            ("Mechanical", PhysicsModel::Mechanical),
            ("TM", LEDGER_TM),
            ("TE", LEDGER_TE),
        ] {
            let mechanical = physics == PhysicsModel::Mechanical;
            // A matched layer twice as slow: `ε = μ = 2`, or `ρ = 2, k = 1/2`.
            let matched = if mechanical {
                Layer::of(2.0, 0.5)
            } else {
                Layer::of(2.0, 2.0)
            };
            let one = Layer::of(1.0, 1.0);
            let mut add = |name: &str, layers: Vec<Layer>, interfaces: Vec<f64>, mode, omega| {
                cases.push(FresnelCase {
                    name: format!("{skin} {name}"),
                    physics,
                    joints: vec![Joint::Welded; interfaces.len()],
                    layers,
                    interfaces,
                    mode,
                    omega,
                });
            };
            add(
                "normal, 4 and 1",
                vec![one, Layer::of(4.0, 1.0)],
                vec![0.0],
                0,
                tau,
            );
            add(
                "normal, 2 and 3",
                vec![one, Layer::of(2.0, 3.0)],
                vec![0.0],
                0,
                tau,
            );
            add(
                "30°, 2 and 3",
                vec![one, Layer::of(2.0, 3.0)],
                vec![0.0],
                1,
                at(30.0, 1.0),
            );
            add(
                "60°, 2 and 3",
                vec![one, Layer::of(2.0, 3.0)],
                vec![0.0],
                1,
                at(60.0, 1.0),
            );
            add("matched, normal", vec![one, matched], vec![0.0], 0, tau);
            add(
                "matched, 30°",
                vec![one, matched],
                vec![0.0],
                1,
                at(30.0, 1.0),
            );
            add(
                "contrast 25",
                vec![one, Layer::of(25.0, 1.0)],
                vec![0.0],
                0,
                pi,
            );
            let dense = Layer::of(4.0, 1.0);
            // Index 2 in every skin: `ε = 4` over `μ = 1`, or `ρ = 4` over `k = 1`.
            let dense_index = 2.0;
            add(
                "total reflection, 45°",
                vec![dense, one],
                vec![0.0],
                1,
                at(45.0, dense_index),
            );
            add(
                "total reflection, 60°",
                vec![dense, one],
                vec![0.0],
                1,
                at(60.0, dense_index),
            );
            add(
                "slab, resonant",
                vec![one, Layer::of(4.0, 1.0), one],
                vec![0.0, 0.5],
                0,
                tau,
            );
            add(
                "slab, off resonance",
                vec![one, Layer::of(4.0, 1.0), one],
                vec![0.0, 0.375],
                0,
                tau,
            );
            if mechanical {
                for across in [false, true] {
                    let crystal = Layer {
                        axis_ratio: 2.0,
                        across,
                        ..Layer::of(1.0, 1.0)
                    };
                    let name = if across {
                        "axis across, 30°"
                    } else {
                        "axis along, 30°"
                    };
                    add(name, vec![one, crystal], vec![0.0], 1, at(30.0, 1.0));
                }
            }
        }
        cases
    }

    // -----------------------------------------------------------------------
    // PH-13: thin gaps against their spring law.
    // -----------------------------------------------------------------------

    /// The ratio at which two gaps `apart` in a unit medium transmit the
    /// mode at `omega` whole: where `|R|` vanishes, found by golden section
    /// on the transfer matrix between `low` and `high`, a bracket that holds
    /// one minimum: `|R|` falls again as the gaps stiffen towards a weld.
    fn resonant_ratio(apart: f64, omega: f64, low: f64, high: f64) -> f64 {
        let one = Layer::of(1.0, 1.0);
        let reflection = |ratio: f64| {
            let gap = Joint::Gap {
                ratio,
                downward: false,
            };
            layered_response(LEDGER_TM, &[one; 3], &[0.0, apart], &[gap; 2], omega, 0.0)
                .0
                .norm()
        };
        let golden = (5.0_f64.sqrt() - 1.0) / 2.0;
        let (mut low, mut high) = (low, high);
        while high - low > 1.0e-12 * high {
            let a = high - golden * (high - low);
            let b = low + golden * (high - low);
            if reflection(a) < reflection(b) {
                high = b;
            } else {
                low = a;
            }
        }
        let ratio = 0.5 * (low + high);
        assert!(
            reflection(ratio) < 1.0e-8,
            "no resonance between the bounds"
        );
        ratio
    }

    /// PH-13's cases. A gap of ratio `r` in a unit medium passes
    /// `|T|² = 1/(1 + (k_x/2r)²)` of the mode, the same in every skin: in
    /// TM at normal incidence a weak gap passing 1% (`r = 0.3`), one passing
    /// half (`r = π`) and a stiff one 99% (`r = 30`); the half gap at 30°; the
    /// cut, the limit as the spring vanishes; and two gaps 0.15 apart at the
    /// ratio that makes the pair transmit everything, and the same pair 0.25
    /// apart. Between a unit medium and another the spring is the ratio
    /// times the two sides' geometric mean: `ε = 4` in TM and TE, `K = r`
    /// and `r/2`, and `k = 4` in Mechanical, `2r`; the last two drawn either
    /// way.
    fn gap_cases() -> Vec<FresnelCase> {
        let pi = std::f64::consts::PI;
        let tau = std::f64::consts::TAU;
        let one = Layer::of(1.0, 1.0);
        let gap = |ratio: f64| Joint::Gap {
            ratio,
            downward: false,
        };
        let mut cases = Vec::new();
        let mut add = |name: &str,
                       physics,
                       layers: Vec<Layer>,
                       interfaces: Vec<f64>,
                       joints: Vec<Joint>,
                       mode,
                       omega| {
            cases.push(FresnelCase {
                name: name.to_owned(),
                physics,
                layers,
                interfaces,
                joints,
                mode,
                omega,
            });
        };
        for (name, ratio) in [("weak", 0.3), ("half", pi), ("stiff", 30.0)] {
            add(
                &format!("TM gap, {name}"),
                LEDGER_TM,
                vec![one, one],
                vec![0.0],
                vec![gap(ratio)],
                0,
                tau,
            );
        }
        // Mode 1 at 30° in a unit medium: `sin θ = π/ω`.
        add(
            "TM gap, half, 30°",
            LEDGER_TM,
            vec![one, one],
            vec![0.0],
            vec![gap(pi)],
            1,
            2.0 * pi,
        );
        add(
            "TM cut",
            LEDGER_TM,
            vec![one, one],
            vec![0.0],
            vec![Joint::Cut],
            0,
            tau,
        );
        let resonant = resonant_ratio(0.15, tau, 1.0, 8.0);
        for (name, apart) in [("resonant", 0.15), ("off resonance", 0.25)] {
            add(
                &format!("TM gap pair, {name}"),
                LEDGER_TM,
                vec![one; 3],
                vec![0.0, apart],
                vec![gap(resonant); 2],
                0,
                tau,
            );
        }
        for (name, physics, other, downward) in [
            ("TM gap, 1 and ε 4", LEDGER_TM, Layer::of(4.0, 1.0), false),
            ("TE gap, 1 and ε 4", LEDGER_TE, Layer::of(4.0, 1.0), false),
            (
                "TE gap, 1 and ε 4, drawn down",
                LEDGER_TE,
                Layer::of(4.0, 1.0),
                true,
            ),
            (
                "Mechanical gap, 1 and k 4",
                PhysicsModel::Mechanical,
                Layer::of(1.0, 4.0),
                false,
            ),
            (
                "Mechanical gap, 1 and k 4, drawn down",
                PhysicsModel::Mechanical,
                Layer::of(1.0, 4.0),
                true,
            ),
        ] {
            add(
                name,
                physics,
                vec![one, other],
                vec![0.0],
                vec![Joint::Gap {
                    ratio: pi,
                    downward,
                }],
                0,
                tau,
            );
        }
        cases
    }

    /// What PH-13's handoff reads: the case read on the coarse mesh before
    /// the handoff and, if asked, on the fine one after it; and the gap's
    /// memory right after it, sample by sample, against the jump the coarse
    /// run's lock-in reads at that instant and against the exact jump the
    /// measured waves drive, the worst of each relative to the jump's
    /// amplitude.
    #[derive(Clone, Debug)]
    struct GapHandoff {
        before: InterfaceReading,
        after: Option<InterfaceReading>,
        carried: f64,
        exact: f64,
    }

    /// PH-13's handoff: `case`, with one gap, held to steady state on edge
    /// `coarse` and read; remeshed at `fine` through the maps the runtime
    /// prepares and handed over by `CanonicalWaveState::handed_over`, the
    /// gap's memory carried by physical position; then, with `after`,
    /// settled again and read on the fine mesh. Both run at the finer
    /// mesh's step.
    fn gap_handoff(case: &FresnelCase, coarse: f64, fine: f64, after: bool) -> GapHandoff {
        let (ramp, settle, window) = fresnel_timing(case);
        let first_leg = ramp + settle + window;
        let document = interface_document(case, ramp, first_leg + settle + window);
        let mut runtime = TopologyRuntime::default();
        let (source, _) = prepare_on(&mut runtime, &document, 1, coarse);
        let (target, maps) = prepare_on(&mut runtime, &document, 2, fine);
        let maps = maps.expect("a remesh carries its maps");
        let dt = source
            .recommended_time_step()
            .min(target.recommended_time_step());
        let (_, _, points) = fresnel_points(case);
        let (from, to) = (&source.canonical_operator, &target.canonical_operator);
        let mut state = CanonicalWaveState::zero(from, dt)
            .unwrap()
            .with_wall_step(from, CanonicalWallStep::default())
            .unwrap();
        let steps = (first_leg / dt).ceil() as usize;
        let (traces, gaps) = record_steps(
            &mut state,
            &source,
            steps,
            &point_stencils(&source, &points),
        );
        let before = fresnel_read(case, from, &traces, &gaps, dt, dt, window);
        let desired = crate::canonical_gpu::CanonicalGpuRuntimeTransfer::from_primary_transfer(
            from,
            to,
            &source.canonical_forcing,
            &target.canonical_forcing,
            &maps.primary,
            [0; 4],
        )
        .unwrap()
        .desired_totals(from, state.primary_flux());
        let (mut state, _) = state
            .handed_over(
                from,
                to,
                &target.canonical_forcing,
                CanonicalHandoffMaps {
                    primary: &maps.primary,
                    complementary: &maps.complementary,
                    thin_gap: &maps.thin_gap,
                    outgoing: &maps.outgoing,
                },
                &desired,
                dt,
            )
            .unwrap();
        // The memory reads `ψ_left − ψ_right`, `ψ = u/(iω)`, and a lock-in
        // phasor `P` stands for `Re(2 P e^{iωt})`.
        let time = steps as f64 * dt;
        let interface = case
            .joints
            .iter()
            .position(|joint| matches!(joint, Joint::Gap { .. }))
            .unwrap();
        let Joint::Gap { downward, .. } = case.joints[interface] else {
            unreachable!()
        };
        let sign = if downward { 1.0 } else { -1.0 };
        let exact = (case.exact_jumps(before.incident, before.returned)[interface]
            / Phasor::new(0.0, case.omega))
        .scale(sign);
        let read = before.gaps[0].jump;
        let at = |phasor: Phasor, y: f64| {
            let turn = Phasor::new((case.omega * time).cos(), (case.omega * time).sin());
            2.0 * (phasor * turn).re * (case.kt() * (y + 0.5)).cos()
        };
        let x = case.interfaces[interface];
        let (mut carried, mut against_exact) = (0.0_f64, 0.0_f64);
        for (sample, memory) in to
            .thin_gap_samples()
            .iter()
            .zip(state.thin_gap_memory(to).unwrap())
        {
            let point = to.node_points()[sample.left_node as usize];
            if (point.x - x).abs() > 1.0e-9 {
                continue;
            }
            carried = carried.max((memory.jump - at(read, point.y)).abs() / (2.0 * read.norm()));
            against_exact =
                against_exact.max((memory.jump - at(exact, point.y)).abs() / (2.0 * exact.norm()));
        }
        let after = after.then(|| {
            let steps = ((settle + window) / dt).ceil() as usize;
            let (traces, gaps) = record_steps(
                &mut state,
                &target,
                steps,
                &point_stencils(&target, &points),
            );
            fresnel_read(case, to, &traces, &gaps, dt, time + dt, window)
        });
        GapHandoff {
            before,
            after,
            carried,
            exact: against_exact,
        }
    }

    /// The transfer matrix meets the gap's closed forms: a gap of ratio `r`
    /// in a unit medium passes `|T|² = 1/(1 + (k_x/2r)²)` with no loss, its
    /// jump the flux over the spring; a stiff gap tends to the weld and a
    /// weak one to the cut; and two gaps `d` apart transmit everything where
    /// `r = (k/2) tan(k d)`, which the search finds.
    #[test]
    fn the_gap_transfer_meets_its_closed_forms() {
        let tau = std::f64::consts::TAU;
        let one = Layer::of(1.0, 1.0);
        let gap = |ratio: f64| Joint::Gap {
            ratio,
            downward: false,
        };
        for (ratio, kt) in [
            (0.3, 0.0),
            (std::f64::consts::PI, 0.0),
            (30.0, 0.0),
            (2.0, 3.0),
        ] {
            let kx = (tau * tau - kt * kt).sqrt();
            let (amplitudes, jumps) =
                layered_amplitudes(LEDGER_TM, &[one, one], &[0.0], &[gap(ratio)], tau, kt);
            let (r, t) = (amplitudes[0].1, amplitudes[1].0);
            let passed = 1.0 / (1.0 + (kx / (2.0 * ratio)).powi(2));
            assert!(
                (t.norm().powi(2) - passed).abs() < 1.0e-12,
                "{ratio}: {t:?}"
            );
            assert!((r.norm().powi(2) + t.norm().powi(2) - 1.0).abs() < 1.0e-12);
            // The flux of the transmitted wave over the spring.
            let flux = Phasor::new(0.0, -kx) * t;
            assert!((jumps[0] - flux.scale(1.0 / ratio)).norm() < 1.0e-12);
        }
        let welded = layered_response(
            LEDGER_TM,
            &[one, Layer::of(4.0, 1.0)],
            &[0.0],
            &[Joint::Welded],
            tau,
            0.0,
        );
        let stiff = layered_response(
            LEDGER_TM,
            &[one, Layer::of(4.0, 1.0)],
            &[0.0],
            &[gap(1.0e9)],
            tau,
            0.0,
        );
        assert!((welded.0 - stiff.0).norm() < 1.0e-8 && (welded.1 - stiff.1).norm() < 1.0e-8);
        let cut = layered_response(LEDGER_TM, &[one, one], &[0.0], &[Joint::Cut], tau, 0.0);
        let weak = layered_response(LEDGER_TM, &[one, one], &[0.0], &[gap(1.0e-9)], tau, 0.0);
        assert!((cut.0 - weak.0).norm() < 1.0e-8 && cut.1.norm() < 1.0e-12);
        assert!((cut.0 - Phasor::new(1.0, 0.0)).norm() < 1.0e-12);
        let apart = 0.15;
        let closed = 0.5 * tau * (tau * apart).tan();
        assert!((resonant_ratio(apart, tau, 1.0, 8.0) - closed).abs() < 1.0e-6 * closed);
    }

    /// PH-13's bounds, every reading of a level under one, the misfit under
    /// twice it. At the production step on edge 0.1 the worst is 2.4e-3, the
    /// weak gap's field jump read off the fitted waves, and the misfit
    /// 3.5e-3 beside `ε = 4` on five elements to its wavelength; on 0.07,
    /// 9.7e-4 and 1.8e-3; on 0.05, 2.4e-4, the gap's stored energy, and
    /// 3.0e-4. The gap's memory is a drifted variable of the step and
    /// carries its bounded second-order offset, which sets its readings
    /// from 0.07 down: 1e-3, 4e-4 and 2.4e-4 of the stored energy. Right
    /// after a handoff the memory stands 8.8e-4 of the jump from the coarse
    /// run's harmonic at that instant, with the gap carried, and 0.69
    /// without, the jump's own value at that instant.
    const GAP_COARSE: f64 = 5.0e-3;
    const GAP_MIDDLE: f64 = 2.0e-3;
    const GAP_FINE: f64 = 5.0e-4;
    const GAP_CARRIED: f64 = 3.0e-3;

    /// Holds a handoff's memory right after it to `GAP_CARRIED`.
    fn gap_handoff_holds(case: &FresnelCase, handoff: &GapHandoff) {
        eprintln!(
            "{} handed over: carried {:.2e}, against the exact jump {:.2e}",
            case.name, handoff.carried, handoff.exact
        );
        for (what, error) in [("carried", handoff.carried), ("exact", handoff.exact)] {
            assert!(
                error < GAP_CARRIED,
                "{} handed over: the memory {error:e} off the {what} jump",
                case.name
            );
        }
    }

    /// PH-13 in the gate, on edge 0.1 at the production step: the gap that
    /// passes half the mode, the pair at its resonance, and the gap between
    /// a unit medium and `ε = 4` in TE drawn down, its spring the two sides'
    /// geometric mean; and the half gap held to steady state and remeshed
    /// at 0.07, its memory read right after the handoff.
    #[test]
    fn thin_gaps_scatter_as_their_springs_say() {
        let names = [
            "TM gap, half",
            "TM gap pair, resonant",
            "TE gap, 1 and ε 4, drawn down",
        ];
        let cases = gap_cases()
            .into_iter()
            .filter(|case| names.contains(&case.name.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(cases.len(), names.len());
        let (levels, handoff) = std::thread::scope(|scope| {
            let handoff = scope.spawn(|| gap_handoff(&cases[0], 0.1, 0.07, false));
            (
                fresnel_levels(&cases, &[(0.1, 1.0)]),
                handoff.join().unwrap(),
            )
        });
        for (case, errors) in cases.iter().zip(levels) {
            fresnel_holds(case, 0.1, 1.0, errors[0], GAP_COARSE);
        }
        gap_handoff_holds(&cases[0], &handoff);
    }

    /// PH-13 (`docs/spikes/funfern-testing-campaign-plan.md`, section 6e):
    /// every case on edge 0.1 and 0.07; a case of each kind on 0.05, `R` and
    /// `T` there under 0.6 of their readings on 0.07 wherever those stand
    /// above 2e-5; the half and the stiff gap at half the step, `R` and `T`
    /// within 5% of the full step's and the stored energy's offset under
    /// 0.4 of it; and the half gap at 30°, whose jump varies along the gap,
    /// remeshed from 0.1 to 0.07 and read on both.
    #[test]
    #[ignore = "PH-13, about twelve minutes; run in the nightly"]
    fn thin_gaps_converge_to_their_springs() {
        let cases = gap_cases();
        let fine = [
            "TM gap, weak",
            "TM gap, half, 30°",
            "TM cut",
            "TM gap pair, resonant",
            "TE gap, 1 and ε 4",
            "Mechanical gap, 1 and k 4",
        ];
        let stepped = ["TM gap, half", "TM gap, stiff"];
        let named = |name: &str| cases.iter().find(|case| case.name == name).unwrap().clone();
        let handed = named("TM gap, half, 30°");
        let subset = fine.map(named);
        let halved = stepped.map(named);
        let (levels, finer, steps, handoff) = std::thread::scope(|scope| {
            let handoff = scope.spawn(|| gap_handoff(&handed, 0.1, 0.07, true));
            (
                fresnel_levels(&cases, &[(0.1, 1.0), (0.07, 1.0)]),
                fresnel_levels(&subset, &[(0.05, 1.0)]),
                fresnel_levels(&halved, &[(0.1, 1.0), (0.1, 0.5)]),
                handoff.join().unwrap(),
            )
        });
        for (case, errors) in cases.iter().zip(&levels) {
            fresnel_holds(case, 0.1, 1.0, errors[0], GAP_COARSE);
            fresnel_holds(case, 0.07, 1.0, errors[1], GAP_MIDDLE);
        }
        for (case, errors) in subset.iter().zip(finer) {
            let errors = errors[0];
            fresnel_holds(case, 0.05, 1.0, errors, GAP_FINE);
            let coarser = levels[cases
                .iter()
                .position(|other| other.name == case.name)
                .unwrap()][1];
            for (what, at, before) in [
                ("R", errors.reflection, coarser.reflection),
                ("T", errors.transmission, coarser.transmission),
            ] {
                assert!(
                    before < 2.0e-5 || at < 0.6 * before,
                    "{}: {what} {before:.2e} on 0.07, {at:.2e} on 0.05",
                    case.name
                );
            }
        }
        for (case, errors) in halved.iter().zip(steps) {
            let (full, half) = (errors[0], errors[1]);
            fresnel_holds(case, 0.1, 0.5, half, GAP_COARSE);
            for (what, at, before) in [
                ("R", half.reflection, full.reflection),
                ("T", half.transmission, full.transmission),
            ] {
                assert!(
                    (at - before).abs() < 0.05 * before,
                    "{}: {what} {before:.2e} at the step, {at:.2e} at half",
                    case.name
                );
            }
            let (at, before) = (half.gap_energy.unwrap(), full.gap_energy.unwrap());
            assert!(
                at < 0.4 * before,
                "{}: stored energy {before:.2e} off at the step, {at:.2e} at half",
                case.name
            );
        }
        gap_handoff_holds(&handed, &handoff);
        fresnel_holds(&handed, 0.1, 1.0, handed.errors(handoff.before), GAP_COARSE);
        fresnel_holds(
            &handed,
            0.07,
            1.0,
            handed.errors(handoff.after.unwrap()),
            GAP_MIDDLE,
        );
    }

    /// Prints a PH-11 reading and holds it: the scattering, the power and,
    /// across a single interface, the field's and the flux's jumps, all
    /// under `bound`.
    fn fresnel_holds(
        case: &FresnelCase,
        edge: f64,
        fraction: f64,
        errors: FresnelErrors,
        bound: f64,
    ) {
        let gap = errors.gap_jump.map_or(String::new(), |jump| {
            format!(
                ", gap {jump:.1e}, stored {:.1e}",
                errors.gap_energy.unwrap_or(0.0)
            )
        });
        eprintln!(
            "{} on {edge} at {fraction}: R {:.2e}, T {:.2e}, power {:.1e}, jumps {:.1e} {:.1e}, misfit {:.1e}{gap}",
            case.name,
            errors.reflection,
            errors.transmission,
            errors.power,
            errors.field_jump.unwrap_or(0.0),
            errors.flux_jump.unwrap_or(0.0),
            errors.misfit
        );
        assert!(
            errors.misfit < 2.0 * bound,
            "{} on {edge} at {fraction}: misfit {:e}",
            case.name,
            errors.misfit
        );
        for (what, error) in [
            ("R", errors.reflection),
            ("T", errors.transmission),
            ("power", errors.power),
            ("field jump", errors.field_jump.unwrap_or(0.0)),
            ("flux jump", errors.flux_jump.unwrap_or(0.0)),
            ("gap's jump", errors.gap_jump.unwrap_or(0.0)),
            ("gap's energy", errors.gap_energy.unwrap_or(0.0)),
        ] {
            assert!(
                error < bound,
                "{} on {edge} at {fraction}: {what} {error:e} off",
                case.name
            );
        }
    }

    /// Runs `cases` on `levels` of edge and step, side by side, and returns
    /// each case's errors at each level.
    fn fresnel_levels(cases: &[FresnelCase], levels: &[(f64, f64)]) -> Vec<Vec<FresnelErrors>> {
        let next = std::sync::atomic::AtomicUsize::new(0);
        let workers = std::thread::available_parallelism().map_or(4, |count| count.get());
        let jobs = cases.len() * levels.len();
        let results = std::sync::Mutex::new(vec![None; jobs]);
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    loop {
                        let job = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if job >= jobs {
                            break;
                        }
                        let (case, (edge, fraction)) =
                            (&cases[job / levels.len()], levels[job % levels.len()]);
                        let errors = case.errors(case.run(edge, fraction));
                        results.lock().unwrap()[job] = Some(errors);
                    }
                });
            }
        });
        let results = results.into_inner().unwrap();
        results
            .chunks(levels.len())
            .map(|chunk| chunk.iter().map(|errors| errors.unwrap()).collect())
            .collect()
    }

    /// PH-11's bounds. On edge 0.1 the gate's cases read 4.2e-3 at worst, a
    /// field jump at 30° in TM, and every case 7.5e-3, contrast 25 in TE on
    /// four elements to its wavelength; on 0.07 every case 1.9e-3; on 0.05
    /// the subset 3.8e-4.
    const FRESNEL_GATE: f64 = 8.0e-3;
    const FRESNEL_COARSE: f64 = 1.5e-2;
    const FRESNEL_MIDDLE: f64 = 3.0e-3;
    const FRESNEL_FINE: f64 = 1.0e-3;

    /// PH-11 in the gate: normal incidence on `ε = 2, μ = 3` and 30° in TM
    /// and TE, total internal reflection at 45°, an anisotropic layer with
    /// its axis along and across the channel, and a resonant slab, on edge
    /// 0.1 at the production step.
    #[test]
    fn planar_interfaces_scatter_as_fresnel_says() {
        let names = [
            "TM normal, 2 and 3",
            "TE normal, 2 and 3",
            "TM 30°, 2 and 3",
            "TE 30°, 2 and 3",
            "TM total reflection, 45°",
            "Mechanical axis along, 30°",
            "Mechanical axis across, 30°",
            "TM slab, resonant",
        ];
        let cases = interface_cases()
            .into_iter()
            .filter(|case| names.contains(&case.name.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(cases.len(), names.len());
        let levels = fresnel_levels(&cases, &[(0.1, 1.0)]);
        for (case, errors) in cases.iter().zip(levels) {
            fresnel_holds(case, 0.1, 1.0, errors[0], FRESNEL_GATE);
        }
    }

    /// PH-11 (`docs/spikes/funfern-testing-campaign-plan.md`): every case on
    /// edge 0.1; on 0.07 every case but the six at 60°, which sit near their
    /// mode's cutoff, take the longest lock-in and read alike on every mesh;
    /// a case of each kind on 0.05 as well, each error there under
    /// `FRESNEL_FINE` and, where it stands above 1e-4, the measurement's own
    /// floor, under half its reading on 0.07; and one at half the step,
    /// where the step's own error does not show.
    #[test]
    #[ignore = "PH-11, about half an hour; run in the nightly"]
    fn planar_interfaces_converge_to_fresnel() {
        let cases = interface_cases();
        let coarse = fresnel_levels(&cases, &[(0.1, 1.0)]);
        for (case, errors) in cases.iter().zip(&coarse) {
            fresnel_holds(case, 0.1, 1.0, errors[0], FRESNEL_COARSE);
        }
        let cases = cases
            .into_iter()
            .filter(|case| !case.name.contains("60°"))
            .collect::<Vec<_>>();
        let levels = fresnel_levels(&cases, &[(0.07, 1.0)]);
        for (case, errors) in cases.iter().zip(&levels) {
            fresnel_holds(case, 0.07, 1.0, errors[0], FRESNEL_MIDDLE);
        }
        let fine = [
            "TM normal, 2 and 3",
            "TE 30°, 2 and 3",
            "TM contrast 25",
            "TE contrast 25",
            "TM matched, 30°",
            "TM slab, off resonance",
            "TM total reflection, 45°",
            "Mechanical axis across, 30°",
        ];
        let subset = cases
            .iter()
            .zip(&levels)
            .filter(|(case, _)| fine.contains(&case.name.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(subset.len(), fine.len());
        let finer = fresnel_levels(
            &subset
                .iter()
                .map(|(case, _)| (*case).clone())
                .collect::<Vec<_>>(),
            &[(0.05, 1.0)],
        );
        for ((case, coarser), errors) in subset.iter().zip(finer) {
            let errors = errors[0];
            fresnel_holds(case, 0.05, 1.0, errors, FRESNEL_FINE);
            for (what, at, before) in [
                ("R", errors.reflection, coarser[0].reflection),
                ("T", errors.transmission, coarser[0].transmission),
            ] {
                assert!(
                    at < 1.0e-4 || at < 0.5 * before,
                    "{}: {what} {before:.2e} on 0.07, {at:.2e} on 0.05",
                    case.name
                );
            }
        }
        let halved = cases
            .iter()
            .find(|case| case.name == "TM 30°, 2 and 3")
            .unwrap()
            .clone();
        let errors = fresnel_levels(std::slice::from_ref(&halved), &[(0.07, 0.5)])[0][0];
        fresnel_holds(&halved, 0.07, 0.5, errors, FRESNEL_MIDDLE);
    }

    // -----------------------------------------------------------------------
    // PH-12: cylinder scattering against the partial-wave series.
    // -----------------------------------------------------------------------

    /// PH-12's reference (`tests/physics/references/cylinder-scattering.json`,
    /// from `cylinder_scattering.py`): for each case the scattering ratio
    /// `S_n` of every order it scatters measurably, its slope `dS_n/d(ka)`
    /// at a fixed index and stiffness ratio, and `H1_n(k r)` at the radii
    /// the test fits at, 1.2 to 2 times the cylinder's.
    #[derive(serde::Deserialize)]
    struct ScatteringTable {
        radius: f64,
        cases: Vec<ScatteringEntry>,
    }

    #[derive(Clone, serde::Deserialize)]
    struct ScatteringEntry {
        ka: f64,
        kind: String,
        index: Option<f64>,
        stiffness_ratio: Option<f64>,
        orders: Vec<ScatteringOrder>,
    }

    #[derive(Clone, serde::Deserialize)]
    struct ScatteringOrder {
        ratio: [f64; 2],
        slope: [f64; 2],
        radii: Vec<ScatteringRadius>,
    }

    #[derive(Clone, serde::Deserialize)]
    struct ScatteringRadius {
        radius: f64,
        h: [f64; 2],
    }

    fn scattering_table() -> &'static ScatteringTable {
        static TABLE: std::sync::OnceLock<ScatteringTable> = std::sync::OnceLock::new();
        TABLE.get_or_init(|| {
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/physics/references/cylinder-scattering.json"
            );
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
        })
    }

    /// A PH-12 cylinder: a hole whose rim reflects, one whose rim is pinned,
    /// or an inclusion of a layer.
    #[derive(Clone, Copy, Debug)]
    enum Cylinder {
        Hole,
        Pinned,
        Inclusion(Layer),
    }

    /// One PH-12 case: the skin, the cylinder, its size `k a` in the unit
    /// medium around it, its centre, and the direction the band source's
    /// wave runs, from the `x` axis.
    #[derive(Clone, Debug)]
    struct ScatteringCase {
        name: String,
        physics: PhysicsModel,
        cylinder: Cylinder,
        ka: f64,
        centre: Point2,
        incidence: f64,
    }

    impl ScatteringCase {
        fn k(&self) -> f64 {
            self.ka / scattering_table().radius
        }

        /// The table's entry: by kind and size, and an inclusion by the
        /// index and the stiffness ratio its skin gives its layer.
        fn entry(&self) -> &'static ScatteringEntry {
            let close = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() < 1.0e-9);
            scattering_table()
                .cases
                .iter()
                .find(|entry| {
                    entry.ka == self.ka
                        && match self.cylinder {
                            Cylinder::Hole => entry.kind == "hole",
                            Cylinder::Pinned => entry.kind == "pinned",
                            Cylinder::Inclusion(layer) => {
                                let (mass, sx, _) = layer.physical(self.physics);
                                entry.kind == "penetrable"
                                    && close(entry.index, (mass / sx).sqrt())
                                    && close(entry.stiffness_ratio, sx)
                            }
                        }
                })
                .unwrap_or_else(|| panic!("no table entry for {}", self.name))
        }
    }

    /// `round_circle`'s circle of `radius`, about `centre`.
    fn circle_at(centre: Point2, radius: f64) -> PeriodicCubicSpline {
        let around = round_circle(radius, 64);
        PeriodicCubicSpline::uniform(
            around
                .controls()
                .iter()
                .map(|point| centre + (*point - Point2::default()))
                .collect(),
        )
        .unwrap()
    }

    /// PH-12's scene: the box `[-2, 2]²` of a unit medium, every side a
    /// second-order wall, the cylinder meshed at `tolerance`, and a band
    /// source `e^{-((x cos φ + y sin φ + 1.6)/0.08)²}` across the
    /// background, whose wave runs along `φ`; switched on over `ramp` and
    /// held to `held`. What the walls return meets the cylinder too, and
    /// changes nothing the test reads.
    fn scattering_scene(
        case: &ScatteringCase,
        ramp: f64,
        held: f64,
        edge: f64,
        tolerance: f64,
    ) -> Arc<crate::topology_runtime::PreparedTopology> {
        let half = 2.0;
        let mut builder = Builder::new();
        builder.scene.physics = case.physics;
        builder.scene.geometry.domain = DomainRect {
            min_x: -half,
            max_x: half,
            min_y: -half,
            max_y: half,
        };
        builder.scene.outer_boundaries =
            OuterBoundaryConditions::uniform(OuterBoundaryCondition::SecondOrderOutgoing);
        let background = &mut builder.scene.materials[0];
        background.mass_density = ScalarField::constant(1.0);
        background.stiffness = ScalarField::constant(1.0);
        let circle = circle_at(case.centre, scattering_table().radius);
        let rim = |law| SpanBehavior::Separated {
            left: FaceBoundaryCondition::Reflecting,
            right: law,
            coupling: InternalBoundaryCoupling::Independent,
        };
        match case.cylinder {
            Cylinder::Hole => {
                builder.closed(circle, rim(FaceBoundaryCondition::Reflecting), None);
            }
            Cylinder::Pinned => {
                builder.closed(
                    circle,
                    rim(FaceBoundaryCondition::Dirichlet {
                        signal: TimeSignal::ZERO,
                    }),
                    None,
                );
            }
            Cylinder::Inclusion(layer) => {
                assert!(layer.axis_ratio == 1.0);
                builder.scene.materials.push(Material {
                    id: MaterialId(2),
                    name: "Cylinder".into(),
                    mass_density: ScalarField::constant(layer.mass),
                    stiffness: ScalarField::constant(layer.stiffness),
                    ..Material::default_medium()
                });
                builder.closed(
                    circle,
                    SpanBehavior::Transmitting,
                    Some((MaterialId(2), MaterialFrame::world())),
                );
            }
        }
        let omega = case.k();
        let (cos, sin) = (case.incidence.cos(), case.incidence.sin());
        builder.scene.volume_sources.push(VolumeSource {
            region: BACKGROUND_REGION,
            enabled: true,
            profile: ScalarField::formula(format!(
                "exp(-((x * ({cos}) + y * ({sin}) + 1.6) / 0.08)^2)"
            ))
            .unwrap(),
            parameters: vec![],
            signal: TimeSignal::pulsed(
                [0.0, 1.0, omega / std::f64::consts::TAU, 0.0],
                PulseEnvelope::FlatTop {
                    duration: 2.0 * held,
                    edge: ramp,
                },
                0.0,
                0.0,
            ),
        });
        let mut document = builder.document();
        document.model.source.enabled = false;
        let editor = TopologyEditor::from_document(document.clone()).unwrap();
        let mut runtime = TopologyRuntime::default();
        let token = runtime
            .request(
                1,
                &editor.document,
                editor.compiled_accepted.clone(),
                MeshingOptions {
                    target_edge_length: edge,
                    curve_tolerance: tolerance,
                    ..MeshingOptions::default()
                }
                .sized_for_area(4.0 * half * half),
                true,
            )
            .unwrap();
        loop {
            if let Some(result) = runtime.advance(1 << 16) {
                result.unwrap();
                break;
            }
        }
        runtime.commit_ready(token).unwrap()
    }

    /// What a PH-12 run reads. The mesh's rim is a polygon inscribed in the
    /// circle, which scatters, to first order, as the circle of its area:
    /// `deficit` is that circle's radius short of the cylinder's, relative,
    /// and `S_n − k a · deficit · S_n'` its ratios. Against them, the
    /// solver's own error: the scattered field's against the ratio times the
    /// regular field each order met, on the inner fit circle, relative to
    /// it; each order's `|S_read − S_n|` where the order met a measurable
    /// field, the worst of them; the worst `| |1 + 2 S_read| − 1 |` there,
    /// energy's balance order by order; and the radial fits' largest
    /// misfit. Against the circle's own ratios, `circle`, the field's error
    /// with the polygon's.
    #[derive(Clone, Copy, Debug)]
    struct ScatteringReading {
        circle: f64,
        field: f64,
        order: f64,
        unitarity: f64,
        misfit: f64,
        deficit: f64,
    }

    /// The relative deficit of the radius of the circle with the area of
    /// the polygon the mesh draws the cylinder's rim with, its vertices those
    /// on the rim, ordered by angle about `centre`.
    fn rim_deficit(mesh: &funfern_core::TriMesh, centre: Point2, radius: f64) -> f64 {
        let mut rim = mesh
            .vertices
            .iter()
            .map(|vertex| vertex.point - centre)
            .filter(|offset| (offset.norm() - radius).abs() < 1.0e-5 * radius)
            .collect::<Vec<_>>();
        assert!(rim.len() >= 64, "{} vertices on the rim", rim.len());
        rim.sort_by(|a, b| a.y.atan2(a.x).total_cmp(&b.y.atan2(b.x)));
        let area = (0..rim.len())
            .map(|index| {
                let (a, b) = (rim[index], rim[(index + 1) % rim.len()]);
                0.5 * (a.x * b.y - a.y * b.x)
            })
            .sum::<f64>();
        1.0 - (area / std::f64::consts::PI).sqrt() / radius
    }

    /// One PH-12 run on `edge` at the curve `tolerance` and `fraction` of
    /// the production step. The source switches on over 20 s, or two periods
    /// where longer: a switch-on rings the cylinder's resonances, and over
    /// two periods `ε = 4`'s third order, 0.8 rad/s from the drive and inside
    /// the lock-in's main lobe, still read 5e-5 after the settle. Held 25 s
    /// more, the field at 64 angles on each fit circle about the cylinder is
    /// read by a Blackman-Harris lock-in over eight periods, and each order's
    /// Fourier coefficient at every radius fitted in least squares to
    /// `α_n J_n(k r) + c_n H_n(k r)`, `e^{−iωt}`.
    fn scattering_run(
        case: &ScatteringCase,
        edge: f64,
        tolerance: f64,
        fraction: f64,
    ) -> ScatteringReading {
        let tau = std::f64::consts::TAU;
        let omega = case.k();
        let period = tau / omega;
        let (ramp, settle, window) = (f64::max(20.0, 2.0 * period), 25.0, 8.0 * period);
        let held = ramp + settle + window;
        let prepared = scattering_scene(case, ramp, held, edge, tolerance);
        let entry = case.entry();
        let deficit = rim_deficit(&prepared.mesh, case.centre, scattering_table().radius);
        let radii = entry.orders[0]
            .radii
            .iter()
            .map(|radius| radius.radius)
            .collect::<Vec<_>>();
        let angles = 64_usize;
        let points = radii
            .iter()
            .flat_map(|radius| {
                (0..angles)
                    .map(move |j| case.centre + direction(j as f64 * tau / angles as f64) * *radius)
            })
            .collect::<Vec<_>>();
        let dt = fraction * prepared.recommended_time_step();
        let steps = (held / dt).ceil() as usize;
        let traces = wall_traces(&prepared, dt, steps, &points, CanonicalWallStep::default());
        let span = (window / dt).round() as usize;
        // e^{−iωt} phasors: u = Re(U e^{−iωt}) gives Σ w u e^{iωt} = U/2 Σ w.
        let lock_in = |trace: &[f64]| {
            let start = steps - span;
            let (mut sum, mut total) = (Phasor::default(), 0.0);
            for (step, value) in trace.iter().enumerate().skip(start) {
                let x = tau * (step - start) as f64 / span as f64;
                let weight = 0.35875 - 0.48829 * x.cos() + 0.14128 * (2.0 * x).cos()
                    - 0.01168 * (3.0 * x).cos();
                let phase = omega * (step + 1) as f64 * dt;
                sum = sum + Phasor::new(phase.cos(), phase.sin()).scale(weight * value);
                total += weight;
            }
            sum.scale(2.0 / total)
        };
        let phasors = traces
            .iter()
            .map(|trace| lock_in(trace))
            .collect::<Vec<_>>();
        let largest = entry.orders.len() as i32 - 1;
        let mut fits = Vec::new();
        for n in -largest..=largest {
            let order = &entry.orders[n.unsigned_abs() as usize];
            let sign = if n < 0 && n % 2 != 0 { -1.0 } else { 1.0 };
            let coefficients = (0..radii.len())
                .map(|ring| {
                    (0..angles)
                        .fold(Phasor::default(), |sum, j| {
                            let angle = -f64::from(n) * j as f64 * tau / angles as f64;
                            sum + phasors[ring * angles + j] * Phasor::new(angle.cos(), angle.sin())
                        })
                        .scale(1.0 / angles as f64)
                })
                .collect::<Vec<_>>();
            let basis = |ring: usize| {
                let [j, y] = order.radii[ring].h;
                [Phasor::new(sign * j, 0.0), Phasor::new(sign * j, sign * y)]
            };
            let mut gram = vec![vec![Phasor::default(); 2]; 2];
            let mut rhs = vec![Phasor::default(); 2];
            for (ring, value) in coefficients.iter().enumerate() {
                let functions = basis(ring);
                for row in 0..2 {
                    for column in 0..2 {
                        gram[row][column] =
                            gram[row][column] + functions[row].conj() * functions[column];
                    }
                    rhs[row] = rhs[row] + functions[row].conj() * *value;
                }
            }
            let fitted = solve_phasors(gram, rhs);
            let (mut residual, mut size) = (0.0_f64, 0.0_f64);
            for (ring, value) in coefficients.iter().enumerate() {
                let functions = basis(ring);
                let model = fitted[0] * functions[0] + fitted[1] * functions[1];
                residual = residual.max((model - *value).norm());
                size = size.max(value.norm());
            }
            let misfit = residual / size.max(f64::MIN_POSITIVE);
            let ratio = Phasor::new(order.ratio[0], order.ratio[1]);
            let polygon =
                ratio - Phasor::new(order.slope[0], order.slope[1]).scale(case.ka * deficit);
            let inner = basis(0)[1].norm();
            fits.push((fitted[0], fitted[1], ratio, polygon, inner, misfit));
        }
        let (mut circle, mut error, mut scattered) = (0.0, 0.0, 0.0);
        let met = fits
            .iter()
            .map(|(regular, _, _, _, inner, _)| regular.norm() * inner)
            .fold(0.0, f64::max);
        let (mut order_error, mut unitarity, mut misfit) = (0.0_f64, 0.0_f64, 0.0_f64);
        for (regular, outgoing, ratio, polygon, inner, fit) in &fits {
            circle += ((*outgoing - *ratio * *regular).norm() * inner).powi(2);
            let predicted = *polygon * *regular;
            error += ((*outgoing - predicted).norm() * inner).powi(2);
            scattered += (predicted.norm() * inner).powi(2);
            // An order that met a field a hundredth of the strongest's has
            // its own ratio and fit read.
            if regular.norm() * inner > 1.0e-2 * met {
                misfit = misfit.max(*fit);
                let read = *outgoing / *regular;
                order_error = order_error.max((read - *polygon).norm());
                unitarity =
                    unitarity.max(((Phasor::new(1.0, 0.0) + read.scale(2.0)).norm() - 1.0).abs());
            }
        }
        ScatteringReading {
            circle: (circle / scattered).sqrt(),
            field: (error / scattered).sqrt(),
            order: order_error,
            unitarity,
            misfit,
            deficit,
        }
    }

    /// PH-12's cases: holes at `k a` 0.5, 2 and 5 and pinned cylinders at
    /// 2 and 5 in TM; in TM and TE, `ε = 4` at 0.5 and 2, `ε = 2` at 2 and
    /// 5, `ε = 10` at 0.5 and `μ = 2` at 2; in Mechanical `ρ = 4` with `k`
    /// 1 and 2, and a hole, at 2; and three at `(0.4, −0.3)`, lit from below
    /// and at 30°. Index 2 at 5 and `√10` at 2 keep whispering-gallery
    /// resonances that ring for 25 to 400 s, longer than a run can wait.
    fn scattering_cases() -> Vec<ScatteringCase> {
        let origin = Point2::default();
        let off = Point2::new(0.4, -0.3);
        let mut cases = Vec::new();
        let mut add = |name: &str, physics, cylinder, ka, centre, incidence: f64| {
            cases.push(ScatteringCase {
                name: name.to_owned(),
                physics,
                cylinder,
                ka,
                centre,
                incidence: incidence.to_radians(),
            });
        };
        for ka in [0.5, 2.0, 5.0] {
            add(
                &format!("TM hole, ka {ka}"),
                LEDGER_TM,
                Cylinder::Hole,
                ka,
                origin,
                0.0,
            );
        }
        for ka in [2.0, 5.0] {
            add(
                &format!("TM pinned, ka {ka}"),
                LEDGER_TM,
                Cylinder::Pinned,
                ka,
                origin,
                0.0,
            );
        }
        for (skin, physics) in [("TM", LEDGER_TM), ("TE", LEDGER_TE)] {
            for ka in [0.5, 2.0] {
                add(
                    &format!("{skin} ε 4, ka {ka}"),
                    physics,
                    Cylinder::Inclusion(Layer::of(4.0, 1.0)),
                    ka,
                    origin,
                    0.0,
                );
            }
            for ka in [2.0, 5.0] {
                add(
                    &format!("{skin} ε 2, ka {ka}"),
                    physics,
                    Cylinder::Inclusion(Layer::of(2.0, 1.0)),
                    ka,
                    origin,
                    0.0,
                );
            }
            add(
                &format!("{skin} ε 10, ka 0.5"),
                physics,
                Cylinder::Inclusion(Layer::of(10.0, 1.0)),
                0.5,
                origin,
                0.0,
            );
            add(
                &format!("{skin} μ 2, ka 2"),
                physics,
                Cylinder::Inclusion(Layer::of(1.0, 2.0)),
                2.0,
                origin,
                0.0,
            );
        }
        add(
            "Mechanical ρ 4, ka 2",
            PhysicsModel::Mechanical,
            Cylinder::Inclusion(Layer::of(4.0, 1.0)),
            2.0,
            origin,
            0.0,
        );
        add(
            "Mechanical ρ 4, k 2, ka 2",
            PhysicsModel::Mechanical,
            Cylinder::Inclusion(Layer::of(4.0, 2.0)),
            2.0,
            origin,
            0.0,
        );
        add(
            "Mechanical hole, ka 2",
            PhysicsModel::Mechanical,
            Cylinder::Hole,
            2.0,
            origin,
            0.0,
        );
        add(
            "TM ε 4, ka 2, off centre from below",
            LEDGER_TM,
            Cylinder::Inclusion(Layer::of(4.0, 1.0)),
            2.0,
            off,
            90.0,
        );
        add(
            "TE ε 4, ka 2, off centre at 30°",
            LEDGER_TE,
            Cylinder::Inclusion(Layer::of(4.0, 1.0)),
            2.0,
            off,
            30.0,
        );
        add(
            "TM hole, ka 2, off centre at 30°",
            LEDGER_TM,
            Cylinder::Hole,
            2.0,
            off,
            30.0,
        );
        cases
    }

    /// What a PH-12 reading is held to: against the polygon's ratios, the
    /// solver's error, field, orders and balance alike; the radial fits'
    /// misfit, which reads the point stencil at single points, an error the
    /// fit and the angular sum average away; and against the circle's own
    /// ratios, the field's error with the polygon's.
    #[derive(Clone, Copy, Debug)]
    struct ScatteringBounds {
        solver: f64,
        misfit: f64,
        circle: f64,
    }

    /// A PH-12 run's edge, curve tolerance and fraction of the production
    /// step.
    type ScatteringLevel = (f64, f64, f64);

    /// Prints a PH-12 reading and holds it to `bounds`.
    fn scattering_holds(
        case: &ScatteringCase,
        (edge, tolerance, fraction): ScatteringLevel,
        reading: ScatteringReading,
        bounds: ScatteringBounds,
    ) {
        let at = format!("{} on {edge} at {tolerance:e} and {fraction}", case.name);
        eprintln!(
            "{at}: field {:.2e}, order {:.2e}, unitarity {:.1e}, misfit {:.1e}; circle {:.2e}, deficit {:.2e}",
            reading.field,
            reading.order,
            reading.unitarity,
            reading.misfit,
            reading.circle,
            reading.deficit
        );
        for (what, error) in [
            ("field", reading.field),
            ("order", reading.order),
            ("unitarity", reading.unitarity),
        ] {
            assert!(error < bounds.solver, "{at}: {what} {error:e} off");
        }
        assert!(
            reading.misfit < bounds.misfit,
            "{at}: misfit {:e}",
            reading.misfit
        );
        assert!(
            reading.circle < bounds.circle,
            "{at}: {:e} off the circle",
            reading.circle
        );
    }

    /// Runs each case at its level, side by side, and returns the readings in
    /// order.
    fn scattering_runs(jobs: &[(&ScatteringCase, ScatteringLevel)]) -> Vec<ScatteringReading> {
        let next = std::sync::atomic::AtomicUsize::new(0);
        let workers = std::thread::available_parallelism().map_or(4, |count| count.get());
        let results = std::sync::Mutex::new(vec![None; jobs.len()]);
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    loop {
                        let job = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some((case, (edge, tolerance, fraction))) = jobs.get(job) else {
                            break;
                        };
                        let reading = scattering_run(case, *edge, *tolerance, *fraction);
                        results.lock().unwrap()[job] = Some(reading);
                    }
                });
            }
        });
        results
            .into_inner()
            .unwrap()
            .into_iter()
            .map(|reading| reading.unwrap())
            .collect()
    }

    /// PH-12's bounds for a case of size `ka` on `edge`, at the default
    /// curve tolerance or, `rim`, at PH-06's. Measured up to `k a = 2` on
    /// 0.1, 0.07 and 0.05: the solver's error 1.2e-4, 4.6e-5 and 1.2e-5 at
    /// worst, the misfit 2.3e-3, 3.6e-4 and 2.0e-4, the circle's 8.3e-4
    /// throughout, the rim's polygon short 2.0e-4; at PH-06's tolerance on
    /// 0.1, 1.4e-4, 1.2e-3 and 2.2e-4, short 5.0e-5. At `k a = 5`, four to
    /// six elements to the wavelength on 0.1, 2.6e-3, 1.1e-3 and 2.0e-4, the
    /// misfit 1.6e-3, 4.4e-4 and 1.9e-4, the circle's 2.2e-3, 1.3e-3 and
    /// 1.2e-3.
    fn scattering_bounds(ka: f64, edge: f64, rim: bool) -> ScatteringBounds {
        let large = ka > 2.0;
        let (solver, misfit, circle) = if rim {
            assert!(!large && edge == 0.1);
            (2.5e-4, 2.5e-3, 4.0e-4)
        } else if edge >= 0.1 {
            if large {
                (5.0e-3, 3.0e-3, 4.0e-3)
            } else {
                (2.0e-4, 4.0e-3, 1.2e-3)
            }
        } else if edge >= 0.07 {
            if large {
                (2.0e-3, 8.0e-4, 2.5e-3)
            } else {
                (8.0e-5, 8.0e-4, 1.2e-3)
            }
        } else if large {
            (4.0e-4, 4.0e-4, 2.5e-3)
        } else {
            (2.5e-5, 4.0e-4, 1.2e-3)
        };
        ScatteringBounds {
            solver,
            misfit,
            circle,
        }
    }

    /// PH-12 in the gate, at `k a = 2` on edge 0.1: a reflecting hole and a
    /// pinned cylinder in TM, `ε = 4` in TM and off centre at 30° in TE,
    /// and Mechanical's stiffer inclusion at the default curve tolerance;
    /// `ε = 4` in TE at PH-06's, where the circle's own ratios hold the rim
    /// to its 256 vertices.
    #[test]
    fn cylinders_scatter_as_the_partial_waves_say() {
        let default = MeshingOptions::default().curve_tolerance;
        let cases = scattering_cases();
        let named = |name: &str| cases.iter().find(|case| case.name == name).unwrap();
        let jobs = [
            (named("TM hole, ka 2"), (0.1, default, 1.0)),
            (named("TM pinned, ka 2"), (0.1, default, 1.0)),
            (named("TM ε 4, ka 2"), (0.1, default, 1.0)),
            (
                named("TE ε 4, ka 2, off centre at 30°"),
                (0.1, default, 1.0),
            ),
            (named("Mechanical ρ 4, k 2, ka 2"), (0.1, default, 1.0)),
            (named("TE ε 4, ka 2"), (0.1, CIRCULAR_TOLERANCE, 1.0)),
        ];
        for ((case, level), reading) in jobs.iter().zip(scattering_runs(&jobs)) {
            let bounds = scattering_bounds(case.ka, level.0, level.1 < default);
            scattering_holds(case, *level, reading, bounds);
        }
    }

    /// PH-12 (`docs/spikes/funfern-testing-campaign-plan.md`, section 6d):
    /// every case on edge 0.1 at the default curve tolerance; on 0.07 every
    /// case but the four whose twin in another skin reads alike to the
    /// digit; those up to `k a = 2` on 0.1 at PH-06's tolerance as well,
    /// where the rim's polygon has twice the vertices and the field's error
    /// against the circle falls under half; a subset on 0.05, the solver's
    /// error there under half its reading on 0.07 wherever that stands above
    /// 1e-5; and one at half the step, which moves it under 5%.
    #[test]
    #[ignore = "PH-12, about twenty minutes; run in the nightly"]
    fn cylinders_converge_to_the_partial_waves() {
        let default = MeshingOptions::default().curve_tolerance;
        let cases = scattering_cases();
        let twins = [
            "Mechanical hole, ka 2",
            "Mechanical ρ 4, ka 2",
            "TE μ 2, ka 2",
            "TM μ 2, ka 2",
        ];
        let fine = [
            "TM pinned, ka 2",
            "TM ε 4, ka 2",
            "TE ε 4, ka 2",
            "TE ε 4, ka 2, off centre at 30°",
            "TM ε 4, ka 0.5",
            "TM hole, ka 5",
            "TM ε 2, ka 5",
            "TE ε 2, ka 5",
        ];
        let mut jobs = Vec::new();
        for case in &cases {
            jobs.push((case, (0.1, default, 1.0)));
            if !twins.contains(&case.name.as_str()) {
                jobs.push((case, (0.07, default, 1.0)));
                if case.ka <= 2.0 {
                    jobs.push((case, (0.1, CIRCULAR_TOLERANCE, 1.0)));
                }
            }
            if fine.contains(&case.name.as_str()) {
                jobs.push((case, (0.05, default, 1.0)));
            }
        }
        let halved = cases
            .iter()
            .find(|case| case.name == "TM ε 4, ka 2")
            .unwrap();
        jobs.push((halved, (0.07, default, 0.5)));
        assert_eq!(
            jobs.iter().filter(|(_, level)| level.0 == 0.05).count(),
            fine.len()
        );
        let readings = scattering_runs(&jobs);
        let reading_of = |name: &str, level: ScatteringLevel| {
            jobs.iter()
                .zip(&readings)
                .find(|((case, at), _)| case.name == name && *at == level)
                .map(|(_, reading)| *reading)
                .unwrap()
        };
        for ((case, level), reading) in jobs.iter().zip(&readings) {
            let (edge, tolerance, _) = *level;
            let bounds = scattering_bounds(case.ka, edge, tolerance < default);
            scattering_holds(case, *level, *reading, bounds);
            if tolerance < default {
                let coarser = reading_of(&case.name, (0.1, default, 1.0));
                assert!(
                    reading.circle < 0.5 * coarser.circle,
                    "{}: {:.2e} off the circle at {default:e}, {:.2e} at {tolerance:e}",
                    case.name,
                    coarser.circle,
                    reading.circle
                );
            }
            if edge == 0.05 {
                let coarser = reading_of(&case.name, (0.07, default, 1.0));
                assert!(
                    coarser.field < 1.0e-5 || reading.field < 0.5 * coarser.field,
                    "{}: field {:.2e} on 0.07, {:.2e} on 0.05",
                    case.name,
                    coarser.field,
                    reading.field
                );
            }
        }
        let full = reading_of(&halved.name, (0.07, default, 1.0));
        let half = reading_of(&halved.name, (0.07, default, 0.5));
        assert!(
            (half.field - full.field).abs() < 0.05 * full.field,
            "{}: field {:.2e} at the step, {:.2e} at half",
            halved.name,
            full.field,
            half.field
        );
    }
}
