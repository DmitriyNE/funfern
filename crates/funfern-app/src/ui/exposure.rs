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

    /// Starts the scale again for a field that has been replaced with zeros.
    ///
    /// How loud the session has been is kept. The first frames of a new field
    /// are numerical dust — measured at 3.5e-10 — and an instant attack onto a
    /// scale with nothing behind it latches straight onto that and paints it at
    /// full colour. The remembered peak holds the quiet floor above the dust
    /// until the field is really there.
    pub(super) fn restart(&mut self) {
        self.reference = 0.0;
    }

    /// Starts a genuinely different displayed quantity. Unlike a fresh field
    /// in the same run, it must not inherit a peak measured in different units.
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
