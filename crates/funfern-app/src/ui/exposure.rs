//! Automatic field exposure: the scale the colours are relative to, how it
//! rises and releases, and the floor below which a decayed field fades.

use bevy::prelude::*;

use super::*;

/// A display reference level for one measured quantity.
///
/// Across the shipped examples the field's own amplitude spans a hundredfold,
/// which is wider than the intensity slider's whole range, so no fixed gain can
/// serve them: at the default, seven of the eight painted under a tenth of full
/// colour, and at the slider's maximum three of them still did. The level is
/// measured from the field each frame instead.
///
/// It rises the instant the field does, so a real transient is never clipped,
/// and falls back over about a second, so a placed pulse fades out of the scale
/// rather than darkening everything after it for the rest of the run — which is
/// what the monotone run peak this replaces used to do. It never falls below a
/// small fraction of the loudest level seen, and that is what keeps a field
/// which has decayed into numerical noise from being magnified back into view.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct AutoExposure {
    reference: f64,
    peak: f64,
}

impl AutoExposure {
    /// The most the reference may fall in a second, as a factor. A scale that
    /// moves by a factor rather than by a difference takes the same time to
    /// clear a spike whatever its size, which is the only behaviour that reads
    /// the same on a field of 6e-3 and one of 7e-1.
    ///
    /// This has to be *slower* than the field's own decay or the scale simply
    /// follows it down and a domain that has emptied still paints at full
    /// brightness. Measured on a recorded level series from a scene whose walls
    /// all radiate: after the sources stop the field drains 2000-fold in eight
    /// seconds, and at the 8.0 this started at that still painted 48 % — the
    /// wave looked like it never left. The rates trade against each other in one
    /// direction, the tail brightness a field settles at against how long a
    /// placed pulse holds the scale:
    ///
    /// | per second | drain tail | 20x spike clears |
    /// | --- | --- | --- |
    /// | 1.15 | 0.07-0.37 % | 21 s |
    /// | 1.4 | up to 3.8 % | 9 s |
    /// | 1.7 | up to 8.2 % | 6 s |
    /// | 8.0 | 100 % then 48 % | 2 s |
    ///
    /// Above about 1.25 the scale catches up with the slow late decay and the
    /// picture creeps back up — which is also what made the high-frequency modes
    /// the grid-scale filter is busy killing swim back into view. 1.15 never
    /// does; the cost is that a pulse holds the scale for some twenty seconds,
    /// which is honest, since the pulse really was that much brighter.
    pub(super) const RELEASE_PER_SECOND: f64 = 1.15;
    /// How far under the loudest level seen the reference may go.
    pub(super) const QUIET_FLOOR: f64 = PRESENTATION_QUIET_AMPLITUDE_RATIO;
    /// The longest step the release is allowed to take at once, so a stalled
    /// frame cannot drop the scale by an unbounded factor in one go.
    pub(super) const MAX_STEP_SECONDS: f32 = 1.0;

    /// Starts the scale again for a field of the same scene that has been
    /// replaced with zeros, by a reset or a fresh start.
    ///
    /// How loud the run has been is kept. The first frames of a new field
    /// are numerical dust — measured at 3.5e-10 — and an instant attack onto a
    /// scale with nothing behind it latches straight onto that and paints it at
    /// full colour. The remembered peak holds the quiet floor above the dust
    /// until the field is really there.
    pub(super) fn restart(&mut self) {
        self.reference = 0.0;
    }

    /// Starts a genuinely different displayed quantity, or another scene's
    /// field. Unlike a fresh field in the same run, it must not inherit a peak
    /// measured in different units or from a louder scene.
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(super) fn reference(self) -> Option<f64> {
        (self.reference > 0.0).then_some(self.reference)
    }

    /// Takes this frame's measured level and the wall-clock seconds since the
    /// previous one, and answers with the level to divide by.
    pub(super) fn update(&mut self, level: f64, elapsed: f32) -> Option<f64> {
        if level.is_finite() && level > 0.0 {
            self.peak = self.peak.max(level);
            let elapsed = f64::from(elapsed.clamp(0.0, Self::MAX_STEP_SECONDS));
            // The maximum rises to meet a louder field at once, so nothing is
            // ever clipped, and the release only ever slows the way back down.
            self.reference = level.max(self.reference * Self::RELEASE_PER_SECOND.powf(-elapsed));
            self.reference = self.reference.max(self.peak * Self::QUIET_FLOOR);
        }
        self.reference()
    }

    /// Smoothly turns off structure below the run-relative quiet floor. Merely
    /// flooring the denominator still paints late f32 residue at a few percent,
    /// which reads as a full-domain static pattern. Squaring the smoothstep
    /// makes that residue disappear without a visible threshold crossing.
    pub(super) fn visibility(self, level: f64) -> f64 {
        if !level.is_finite() || level <= 0.0 || self.peak <= 0.0 {
            return 0.0;
        }
        let fraction = (level / (self.peak * Self::QUIET_FLOOR)).clamp(0.0, 1.0);
        let smooth = fraction * fraction * (3.0 - 2.0 * fraction);
        smooth * smooth
    }
}

/// Where the field's reference level sits in its own distribution: above the
/// quiet bulk of the domain, below the few nodes right against a source.
pub(super) const FIELD_EXPOSURE_QUANTILE: f64 = 0.98;

/// The intensity slider is a trim on the automatic scale rather than the scale
/// itself. At its default of 2.0 the reference level lands on `tanh(1.0)`,
/// about three quarters of full colour, which leaves the brightest nodes
/// brighter still instead of clipping them flat.
pub(super) const FIELD_EXPOSURE_GAIN: f32 = 0.5;

/// Presentation-only complementary-field DC rejection. This is the old
/// reconstruction corner (0.5 rad/s, about 0.08 Hz), now applied only to arrow
/// samples and measured in simulated time. Ordinary 2.5--4 Hz waves therefore
/// retain more than 99.9% of their amplitude.
pub(super) const VECTOR_DC_REJECTION_RATE: f64 = 0.5;
/// A vector sampling revision is tiny and normally completes within a few
/// display frames. If its readback disappears during rapid resource churn,
/// retry instead of allowing the one-in-flight coalescer to deadlock.
pub(super) const VECTOR_OVERLAY_READBACK_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(750);
/// The longest arrow below this global visibility is less than about a tenth
/// of a pixel even at the coarsest supported density. Avoiding the draw also
/// avoids assigning a visible direction to near-zero floating-point residue.
pub(super) const VECTOR_OVERLAY_VISIBILITY_CUTOFF: f64 = 1.0 / 512.0;

/// Scales one arrow under a shared exposure. Saturation belongs before the
/// quiet-tail visibility: otherwise an arbitrarily large sparse outlier can
/// cancel an arbitrarily small global fade by hitting the length clamp.
pub(super) fn vector_arrow_length(
    magnitude: f64,
    reference: f64,
    gain: f32,
    maximum_length: f32,
    visibility: f64,
) -> f32 {
    let exposed = (magnitude * f64::from(gain) / reference).clamp(0.0, 1.0);
    (f64::from(maximum_length) * exposed * visibility.clamp(0.0, 1.0)) as f32
}

/// The `quantile` of `values` by magnitude, sampled rather than sorted.
///
/// Sorting every node each frame would spend milliseconds placing a number the
/// field itself moves by more than the estimate's error. Nodes are numbered in
/// meshing order, which bears no relation to the field, so a strided sample is
/// a fair one.
pub(super) fn exposure_level(values: &[f32], quantile: f64, scratch: &mut Vec<f64>) -> f64 {
    pub(super) const SAMPLES: usize = 4096;
    scratch.clear();
    let stride = values.len().div_ceil(SAMPLES).max(1);
    scratch.extend(
        values
            .iter()
            .step_by(stride)
            .map(|value| f64::from(value.abs()))
            .filter(|value| value.is_finite()),
    );
    if scratch.is_empty() {
        return 0.0;
    }
    let index = ((scratch.len() - 1) as f64 * quantile).round() as usize;
    *scratch
        .select_nth_unstable_by(index, |a, b| a.total_cmp(b))
        .1
}

/// The factor a node's value is multiplied by before it becomes colour.
///
/// Automatic, the reference level lands on `tanh(gain * FIELD_EXPOSURE_GAIN)`,
/// which at the default gain is about three quarters of full colour. Manual, the
/// slider is the whole scale — `tanh(value * gain)`, exactly what the field was
/// painted with before it measured its own. With nothing measured yet there is
/// no scale, and every node is zero anyway.
pub(super) fn field_scale(gain: f32, reference: Option<f64>, automatic: bool) -> f64 {
    if !automatic {
        return f64::from(gain);
    }
    reference.map_or(0.0, |reference| {
        f64::from(gain) * f64::from(FIELD_EXPOSURE_GAIN) / reference
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The point of the whole mechanism: the catalog's quietest example and its
    /// loudest sit a hundredfold apart, and both must paint the same picture.
    #[test]
    fn an_exposure_paints_the_same_picture_at_any_field_scale() {
        let scale = 118.0;
        let mut levels = vec![0.0, 1.0e-3, 6.0e-3, 4.0e-3, 6.2e-3, 5.9e-3, 2.0e-2];
        // Then all the way down past the exposure's own floor, so a scale that
        // is relative to the field is told apart from one pinned to a constant.
        let mut decaying = 2.0e-2;
        for _ in 0..40 {
            decaying *= 0.5;
            levels.push(decaying);
        }
        let mut quiet = AutoExposure::default();
        let mut loud = AutoExposure::default();
        let mut floored = false;
        for level in levels {
            let (Some(a), Some(b)) = (quiet.update(level, 0.1), loud.update(level * scale, 0.1))
            else {
                assert_eq!(level, 0.0, "a measured level produced no reference");
                continue;
            };
            floored |= a > level * 2.0;
            let probe = level * 0.7;
            assert!(
                (probe / a - probe * scale / b).abs() < 1.0e-9 * (probe / a).max(1.0e-9),
                "{level}: {a} against {b}"
            );
            assert!(
                (quiet.visibility(level) - loud.visibility(level * scale)).abs() < 1.0e-12,
                "the quiet-tail fade changed with absolute field scale"
            );
        }
        assert!(floored, "the run never reached the quiet floor");
    }

    /// The failure the monotone run peak had: one placed pulse set the scale for
    /// the rest of the run.
    #[test]
    fn an_exposure_recovers_after_a_transient_spike() {
        let mut exposure = AutoExposure::default();
        for _ in 0..120 {
            exposure.update(1.0, 1.0 / 60.0);
        }
        assert_eq!(exposure.update(20.0, 1.0 / 60.0), Some(20.0), "clipped");
        // The release gives up a factor of 1.15 a second, so a twentyfold spike
        // takes some twenty-one seconds to walk off. Bounded is the property
        // that matters — the run peak it replaced never gave it up at all.
        for _ in 0..1_500 {
            exposure.update(1.0, 1.0 / 60.0);
        }
        assert_eq!(exposure.reference(), Some(1.0));
    }

    /// The failure this release rate was chosen for. A scale that falls faster
    /// than the field does simply follows it down, so a domain that has emptied
    /// still paints at full brightness and the wave looks like it never left.
    /// The shape here is the recorded one: full amplitude, then three decades
    /// over eight seconds once the sources stop.
    #[test]
    fn a_field_that_drains_away_stops_being_painted() {
        let mut exposure = AutoExposure::default();
        for _ in 0..600 {
            exposure.update(3.0e-2, 1.0 / 60.0);
        }
        let mut level = 3.0e-2;
        for _ in 0..480 {
            level *= 0.985_7;
            exposure.update(level, 1.0 / 60.0);
        }
        let reference = exposure.reference().unwrap();
        let painted = level / reference;
        assert!(
            level < 3.0e-5,
            "the fixture did not actually drain: {level:e}"
        );
        assert!(
            painted < 0.05,
            "a drained domain still paints at {:.1}%",
            painted * 100.0
        );
    }

    /// And the failure the other way: once a field has decayed into rounding
    /// noise, renormalizing it would fill the view with structure that is not
    /// there.
    #[test]
    fn an_exposure_refuses_to_magnify_decayed_noise() {
        let mut exposure = AutoExposure::default();
        exposure.update(1.0, 1.0 / 60.0);
        // Two decades down to the floor at 1.15 a second is about thirty-three.
        for _ in 0..3_600 {
            exposure.update(1.0e-9, 1.0 / 60.0);
        }
        let floor = exposure.reference().unwrap();
        assert!(
            (floor - AutoExposure::QUIET_FLOOR).abs() < 1.0e-12,
            "{floor}"
        );
        assert!(1.0e-9 / floor < 1.0e-5, "noise would still be drawn");
        assert!(
            exposure.visibility(1.0e-9) < 1.0e-10,
            "late residue was not faded out"
        );
    }

    #[test]
    fn exposure_fades_smoothly_below_its_run_relative_floor() {
        let mut exposure = AutoExposure::default();
        exposure.update(1.0, 0.0);
        assert_eq!(exposure.visibility(AutoExposure::QUIET_FLOOR), 1.0);
        let tenth = exposure.visibility(AutoExposure::QUIET_FLOOR * 0.1);
        assert!((0.0..1.0e-3).contains(&tenth), "weak fade {tenth}");
        assert_eq!(exposure.visibility(0.0), 0.0);
        assert_eq!(exposure.visibility(f64::NAN), 0.0);
    }

    #[test]
    fn measured_damped_tail_is_not_renormalized_into_a_field() {
        let mut exposure = AutoExposure::default();
        exposure.update(1.0, 0.0);
        // The saved source-free TM drain fixture settles near 0.2--0.3% of
        // its propagated peak. Let the release reach its run-relative floor,
        // then verify that tail still paints below two percent of full scale.
        let tail = 3.0e-3;
        for _ in 0..6_000 {
            exposure.update(tail, 1.0 / 60.0);
        }
        let painted = tail / exposure.reference().unwrap() * exposure.visibility(tail);
        assert!(painted < 0.02, "tail still paints at {painted:.3}");
        assert_eq!(
            DORMANT_ENERGY_RATIO,
            AutoExposure::QUIET_FLOOR * AutoExposure::QUIET_FLOOR
        );
    }

    /// Release is a rate in seconds, so the same second of wall clock has to
    /// land in the same place whether it took two frames or two hundred.
    #[test]
    fn an_exposure_releases_by_wall_clock_not_by_frame_count() {
        let mut coarse = AutoExposure::default();
        let mut fine = AutoExposure::default();
        coarse.update(10.0, 0.016);
        fine.update(10.0, 0.016);
        coarse.update(1.0, 0.5);
        coarse.update(1.0, 0.5);
        for _ in 0..100 {
            fine.update(1.0, 0.01);
        }
        // A relative tolerance: the two differ only in how the same decay was
        // recomposed in floating point.
        let (a, b) = (coarse.reference().unwrap(), fine.reference().unwrap());
        assert!((a - b).abs() < a * 1.0e-6, "{a} against {b}");
    }

    #[test]
    fn a_sampled_quantile_matches_the_sorted_one() {
        let mut scratch = Vec::new();
        let values = (0..50_000)
            .map(|index| index as f32 / 50_000.0)
            .collect::<Vec<_>>();
        let level = exposure_level(&values, 0.98, &mut scratch);
        assert!((level - 0.98).abs() < 0.01, "{level}");
        assert_eq!(exposure_level(&[0.0; 32], 0.98, &mut scratch), 0.0);
        assert_eq!(exposure_level(&[], 0.98, &mut scratch), 0.0);
        let broken = [f32::NAN, f32::INFINITY, -3.0, 1.0];
        assert_eq!(exposure_level(&broken, 0.5, &mut scratch), 3.0);
    }

    /// The default gain has to land the reference level somewhere legible, and
    /// leave the nodes above it room to read brighter still.
    #[test]
    fn the_default_gain_paints_the_reference_level_in_the_readable_band() {
        let default = funfern_app::document::PresentationSettings::default().field_gain;
        let scale = default * FIELD_EXPOSURE_GAIN;
        let base = field_color(0.0, Color32::TRANSPARENT);
        let at_reference = field_color(scale, Color32::TRANSPARENT);
        let above = field_color(2.0 * scale, Color32::TRANSPARENT);
        let reach = |color: Color32| f32::from(color.r() - base.r()) / f32::from(244 - base.r());
        assert!(
            (0.7..0.85).contains(&reach(at_reference)),
            "{}",
            reach(at_reference)
        );
        assert!(reach(above) > reach(at_reference) + 0.1, "no room above");
        assert_eq!(
            field_color_over_overlay(scale).a(),
            (0.761_594_f32 * 220.0).round() as u8
        );
    }

    /// Adaptation hands the field to a new mesh every second or two. It is the
    /// same field, so its scale has to carry across: restarting it there dropped
    /// the reference onto the instantaneous level, and a decaying field fell in
    /// visible steps instead of easing down at the release rate.
    #[test]
    fn a_mesh_handoff_leaves_the_scale_alone() {
        let mut state = Playground::default();
        state.field_exposure.update(0.71, 0.016);
        state.vector_overlay_exposure.update(0.71, 0.016);
        state.vector_overlay_ac_owner = Some(VectorOverlayAcOwner {
            mesh_revision: 3,
            physics: PhysicsModel::Mechanical,
        });
        state.vector_overlay_ac_state.insert(
            4,
            VectorAcState {
                input: Point2::new(1.0, 0.0),
                output: Point2::new(0.2, 0.0),
                step: 10,
                time: 0.1,
                origin: Pos2::new(20.0, 30.0),
            },
        );
        state.restart_exposures_after_handoff(false);
        assert_eq!(state.field_exposure.reference(), Some(0.71));
        assert_eq!(state.vector_overlay_exposure.reference(), Some(0.71));
        assert_eq!(state.vector_overlay_ac_state.len(), 1);

        // A field replaced with zeros starts the scale again.
        state.restart_exposures_after_handoff(true);
        assert_eq!(state.field_exposure.reference(), None);
        assert_eq!(state.vector_overlay_exposure.reference(), None);
        assert!(state.vector_overlay_ac_state.is_empty());
        assert_eq!(state.vector_overlay_ac_owner, None);
    }

    /// Asking for a new field leaves the scale alone. A reset zeroes the field
    /// on the GPU, but the outgoing field stays on display until the
    /// replacement arrives, a few frames; a scale cleared at the request
    /// measures that residue and paints it at full brightness: measured at
    /// 0.09 % before, 100 % after, for a tenth of a second. A loaded scene
    /// clears the scales only where it drops the outgoing generation, when
    /// nothing of the old field is left on display.
    #[test]
    fn asking_for_a_new_field_does_not_magnify_the_outgoing_one() {
        let mut state = Playground::default();
        state.field_exposure.update(3.0e-2, 0.016);
        let residue = 3.8e-6;
        let held = state.field_exposure.update(residue, 0.016).unwrap();
        assert!(residue / held < 0.01, "the residue was not already dark");

        state.reset_requested = true;
        let document = state.editor.document.clone();
        state.set_document(document, false, true).unwrap();
        let after = state.field_exposure.update(residue, 0.016).unwrap();
        assert!(
            residue / after < 0.01,
            "the outgoing field was magnified to {:.0}%",
            100.0 * residue / after
        );
    }

    /// The integrated field and the field are measured apart: showing one
    /// after the other starts the scale again, and asking for the same one
    /// again, as every frame does, keeps it.
    #[test]
    fn switching_to_or_from_the_integrated_field_starts_its_scale_again() {
        let mut state = Playground::default();
        state.follow_field_quantity(true);
        state.field_exposure.update(34.8, 0.016);
        state.follow_field_quantity(true);
        assert_eq!(state.field_exposure.reference(), Some(34.8));
        state.follow_field_quantity(false);
        let rate = 0.3;
        assert_eq!(state.field_exposure.update(rate, 0.016), Some(rate));
        assert_eq!(state.field_exposure.visibility(rate), 1.0);
    }

    /// The first frames of a replaced field are numerical dust, and an instant
    /// attack onto a scale with nothing behind it paints that dust at full
    /// colour. Measured at 3.5e-10 arriving one frame before the real field.
    #[test]
    fn a_restarted_scale_does_not_latch_onto_the_first_dust() {
        let mut exposure = AutoExposure::default();
        exposure.update(3.0e-2, 0.016);
        exposure.restart();
        let dust = 3.5e-10;
        let reference = exposure.update(dust, 0.016).unwrap();
        assert!(
            dust / reference < 1.0e-4,
            "dust painted at {:.0}%",
            100.0 * dust / reference
        );
        // The real field, when it arrives, takes the scale straight over.
        assert_eq!(exposure.update(2.0e-2, 0.016), Some(2.0e-2));
    }

    /// The symptom the handoff bug showed as: a scale that falls faster than the
    /// release allows. Nothing `update` does may outrun that rate.
    #[test]
    fn the_scale_never_falls_faster_than_the_release_rate() {
        let mut exposure = AutoExposure::default();
        let mut level = 1.0_f64;
        let step = 1.0_f32 / 60.0;
        let mut previous = exposure.update(level, step).unwrap();
        for _ in 0..1_200 {
            level *= 0.98;
            let reference = exposure.update(level, step).unwrap();
            // The same widening `update` does, so the two agree to the bit.
            let allowed = previous * AutoExposure::RELEASE_PER_SECOND.powf(-f64::from(step));
            assert!(
                reference >= allowed * (1.0 - 1.0e-12),
                "the scale fell to {reference:e} when {allowed:e} was the floor"
            );
            previous = reference;
        }
    }

    /// Turning the automatic scale off has to put the field back exactly where it
    /// was before there was one: the slider as the whole scale.
    #[test]
    fn turning_auto_exposure_off_restores_the_plain_gain() {
        let default = funfern_app::document::PresentationSettings::default();
        assert!(default.field_auto_exposure, "it should start on");
        assert_eq!(field_scale(2.0, Some(0.02), false), 2.0);
        assert_eq!(field_scale(0.25, None, false), 0.25);
        // Automatic, the same field paints the same whatever its size.
        let quiet = field_scale(2.0, Some(6.0e-3), true) * 6.0e-3;
        let loud = field_scale(2.0, Some(7.1e-1), true) * 7.1e-1;
        assert!((quiet - loud).abs() < 1.0e-12);
        assert_eq!(field_scale(2.0, None, true), 0.0);
    }
}
