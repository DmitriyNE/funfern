//! Pacing the solver against the display: how many steps a frame asks for,
//! the step it uses, and when a shortfall is worth reporting.

use crate::wave_gpu::MAX_STEPS_PER_FRAME;

/// Wall-clock seconds a frame is budgeted when deciding how small the solver's
/// step has to be.
///
/// Fixed rather than the measured frame time: a step that moved with the frame
/// rate would jitter, and each jitter costs a republish. A hundred and twentieth
/// keeps every frame of a fast display fed.
pub(super) const PACING_FRAME_SECONDS: f64 = 1.0 / 120.0;

/// How far the wanted step may drift from the one the solver is running before
/// it is worth republishing to change it.
pub(super) const TIME_STEP_HYSTERESIS: f64 = 0.1;

/// The step the solver runs at: the mesh's stability limit, or smaller when the
/// speed ceiling is low enough that pacing by step count alone would leave whole
/// frames without one.
///
/// Above a speed of `recommended / PACING_FRAME_SECONDS` this is the
/// recommendation unchanged and the rate is paced purely by how many steps a
/// frame asks for. Below it that count floors to zero on most frames and the
/// picture judders — measured at 0.53 steps a frame with 52 % of frames
/// advancing at 0.05x, and a coarse mesh crosses the threshold at 0.8x — so the
/// step shrinks instead and every frame gets one. Shrinking is always safe: the
/// stability limit is an upper bound, and this never goes above it.
pub(super) fn paced_time_step(recommended: f64, speed: f64) -> f64 {
    if !recommended.is_finite() || recommended <= 0.0 || !speed.is_finite() || speed <= 0.0 {
        return recommended;
    }
    recommended.min(PACING_FRAME_SECONDS * speed)
}

/// The two short boundaries at which it is unsafe to publish more work.
/// Preparing and uploading a replacement generation are deliberately absent:
/// the accepted generation can keep advancing through both. We drain just
/// before `begin_handoff`. Once the transfer is encoded, later source steps
/// remain visible while validation is in flight and are replayed by the target
/// from its exact transferred clock before its first visible readback.
pub(super) fn canonical_steps_withheld(packed_candidate_waiting: bool, fresh_upload: bool) -> bool {
    packed_candidate_waiting || fresh_upload
}

/// Steps to ask the solver for this frame, spending `accumulator` at
/// `time_step` a step.
///
/// `speed` is the ceiling on simulated seconds per wall second: the wall-clock
/// time a frame took is scaled by it before being spent, so half asks for half
/// the steps. A late frame may spend at most one 60 Hz display interval. Trying
/// to catch up the whole late interval creates a positive feedback loop on a
/// saturated GPU: a long solver batch delays drawing, the delayed frame asks
/// for a still larger batch, and rendering collapses to the step ceiling. The
/// interactive contract is instead to preserve display service and report the
/// simulation-speed shortfall. The fractional remainder is still retained, but
/// is capped at one solver batch so high requested speeds cannot queue an
/// unbounded backlog.
pub(super) fn steps_for_frame(
    accumulator: &mut f64,
    delta: f64,
    speed: f64,
    time_step: f64,
) -> u64 {
    if !time_step.is_finite() || time_step <= 0.0 || !speed.is_finite() || speed <= 0.0 {
        return 0;
    }
    const DISPLAY_INTERVAL: f64 = 1.0 / 60.0;
    *accumulator += delta.clamp(0.0, DISPLAY_INTERVAL) * speed;
    let steps = (*accumulator / time_step)
        .floor()
        .clamp(0.0, MAX_STEPS_PER_FRAME as f64) as u64;
    *accumulator -= steps as f64 * time_step;
    *accumulator = accumulator.min(MAX_STEPS_PER_FRAME as f64 * time_step);
    steps
}

/// Keeps the host request clock close to the last GPU-completed boundary. A
/// render thread can otherwise encode small batches faster than an overloaded
/// or background-throttled GPU executes them, accumulating minutes of stale
/// simulation work without ever violating the per-frame batch ceiling.
pub(super) fn steps_with_gpu_backpressure(completed: u64, requested: u64, proposed: u64) -> u64 {
    let outstanding = requested.saturating_sub(completed);
    proposed.min(MAX_STEPS_PER_FRAME.saturating_sub(outstanding))
}

/// How close the reached rate has to come to the one asked for before the
/// shortfall is worth mentioning.
pub(super) const SPEED_SHORTFALL_MARGIN: f64 = 0.8;

/// How fast the held rate gives up a better reading, as a factor per second.
///
/// The windowed measurement dips to about three quarters of the rate asked for
/// whenever a handoff withholds stepping inside its window — measured at every
/// speed, including ones the solver reaches comfortably — so comparing it
/// directly would flash the note at random. Holding the best reading rides over
/// a dip of a second while still letting a real slowdown through in under two.
pub(super) const SPEED_HOLD_PER_SECOND: f64 = 1.15;

/// The best rate seen lately: instant to a better reading, slow to give one up.
pub(super) fn hold_rate(held: f64, measured: f64, elapsed: f64) -> f64 {
    if !measured.is_finite() || measured < 0.0 {
        return held;
    }
    measured.max(held * SPEED_HOLD_PER_SECOND.powf(-elapsed.clamp(0.0, 1.0)))
}

/// The rate actually being reached, when it falls meaningfully short of `target`
/// and the solver is genuinely trying.
///
/// Both are simulated seconds per wall second. Nothing is said while the solver
/// is paused or before any steps have been measured — neither is the solver
/// failing to keep up.
pub(super) fn speed_shortfall(measured: f64, target: f64, stepping: bool) -> Option<f64> {
    if !stepping || !measured.is_finite() || measured <= 0.0 {
        return None;
    }
    (measured < target * SPEED_SHORTFALL_MARGIN).then_some(measured)
}
