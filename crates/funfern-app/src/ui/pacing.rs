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
    budget: f64,
) -> u64 {
    if !time_step.is_finite() || time_step <= 0.0 || !speed.is_finite() || speed <= 0.0 {
        return 0;
    }
    const DISPLAY_INTERVAL: f64 = 1.0 / 60.0;
    let ceiling = budget.clamp(1.0, MAX_STEPS_PER_FRAME as f64);
    *accumulator += delta.clamp(0.0, DISPLAY_INTERVAL) * speed;
    let steps = (*accumulator / time_step).floor().clamp(0.0, ceiling) as u64;
    *accumulator -= steps as f64 * time_step;
    // Backlog is held only up to what a frame may actually spend. Holding more
    // would saturate every following frame trying to catch up, which is the
    // opposite of the contract: the shortfall is what gives, and it is
    // reported rather than queued.
    *accumulator = accumulator.min(ceiling * time_step);
    steps
}

/// How much longer than the display's own cadence a frame may run before the
/// solver's batch is held responsible.
///
/// The controller settles on the edge of this band, so the band is the frame
/// rate given away: at `1.25` a 120 Hz display holds 96 fps. Tight enough to
/// keep the cadence, loose enough that a frame landing exactly on it is not
/// read as an overrun. Under vsync a frame is quantized to the refresh period
/// or to twice it, and anything inside this band is the former.
const FRAME_BUDGET_TOLERANCE: f64 = 1.05;

/// What an overrunning frame multiplies the batch ceiling by.
///
/// This also has to be sharp enough to overshoot, because the cut is the only
/// thing that ever produces a frame faster than the last one and so the only
/// way [`hold_cadence`] learns what the display can do. Backing off gently
/// enough to merely stop overrunning leaves a saturated solver defining its own
/// slowness as the cadence: at `0.9` and this scene's numbers the estimate
/// stalls at 9.02 ms and the display holds 111 fps, where `0.75` finds 8.33 ms
/// and holds 116.
const FRAME_BUDGET_BACKOFF: f64 = 0.75;

/// What a frame inside the cadence adds back to it.
const FRAME_BUDGET_RECOVERY: f64 = 0.5;

/// How fast the observed cadence gives up a better reading, as a factor per
/// second. The mirror of [`SPEED_HOLD_PER_SECOND`]: a faster frame is believed
/// at once, a slower one only after the display has stayed slow for a while.
const CADENCE_RELAX_PER_SECOND: f64 = 1.15;

/// The frame interval the display is actually achieving: instant to accept a
/// faster one, slow to accept a slower one.
///
/// This is measured rather than assumed because the target depends on hardware
/// nobody tells us about - 60, 120 and 144 Hz all want different budgets, and
/// an unthrottled window wants whatever it can reach. Taking the best reading
/// lately and letting it relax means a saturated solver cannot quietly define
/// a slow cadence as normal: cutting the batch makes frames faster, which
/// tightens the target, which is the feedback that finds the display's own
/// rate.
pub(super) fn hold_cadence(held: f64, measured: f64, elapsed: f64) -> f64 {
    if !measured.is_finite() || measured <= 0.0 {
        return held;
    }
    measured.min(held * CADENCE_RELAX_PER_SECOND.powf(elapsed.clamp(0.0, 1.0)))
}

/// The batch ceiling for the next frame, given what this one cost.
///
/// Frame rate must not be a function of solver throughput. A frame that cannot
/// advance the simulation as far as the speed setting asks should still ship on
/// time and draw the latest state - at any batch worth pacing the solver has
/// advanced many steps, so there is always a new configuration to show, and the
/// simulated-speed shortfall is the thing that gives. It is already measured
/// and reported.
///
/// Nothing else bounds this. The step count is capped by accumulated simulated
/// time, by `MAX_STEPS_PER_FRAME` and by outstanding encoded lead, but never by
/// how long the batch will take to execute, and the compute shares the frame's
/// queue with drawing. So the batch sets the frame time, and a driven medium -
/// which runs at a tighter step and therefore asks for proportionally more
/// steps a frame - took the display down with it.
///
/// Multiplicative backoff and additive recovery rather than a cost model: it
/// needs no estimate of what a step costs, and it converges on the largest
/// batch that still ships frames at the cadence.
pub(super) fn frame_step_budget(budget: f64, frame_seconds: f64, cadence: f64) -> f64 {
    if !frame_seconds.is_finite() || frame_seconds <= 0.0 || !cadence.is_finite() || cadence <= 0.0
    {
        return budget;
    }
    let next = if frame_seconds > cadence * FRAME_BUDGET_TOLERANCE {
        budget * FRAME_BUDGET_BACKOFF
    } else {
        budget + FRAME_BUDGET_RECOVERY
    };
    next.clamp(1.0, MAX_STEPS_PER_FRAME as f64)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The reported defect: selecting a driven material took the frame rate from
    /// 120 to 65. A driven generation runs at a tighter step, so at the same
    /// requested speed it asks for proportionally more steps a frame, and the
    /// batch shares the frame's queue with drawing. Nothing bounded the batch by
    /// what it would cost, so the solver set the frame time.
    ///
    /// Reproduced from the reported numbers: 8827 dofs, `dt` 1.34e-3 linear
    /// against 6.27e-4 driven, one per-step cost of 0.93 ms fitted to both
    /// operating points, and 2.5 ms of everything else.
    #[test]
    fn a_driven_step_no_longer_takes_the_display_down_with_it() {
        const PER_STEP: f64 = 0.93e-3;
        const OVERHEAD: f64 = 2.5e-3;
        let display = 1.0 / 120.0;

        // One second of frames at a given step, returning the frame rate reached
        // and the simulated seconds advanced.
        let run = |time_step: f64, budgeted: bool| {
            let mut accumulator = 0.0;
            let mut budget = MAX_STEPS_PER_FRAME as f64;
            let mut cadence = 1.0 / 60.0;
            let mut frame = display;
            // A second of warm-up, then two seconds measured: the claim is
            // about the rate the controller settles on, and it starts from the
            // ceiling because nothing has told it what a step costs yet.
            let (mut frames, mut steps, mut elapsed) = (0u32, 0u64, 0.0);
            let mut warm = 0.0;
            while elapsed < 2.0 {
                if budgeted {
                    cadence = hold_cadence(cadence, frame, frame);
                    budget = frame_step_budget(budget, frame, cadence);
                }
                let asked = steps_for_frame(
                    &mut accumulator,
                    frame,
                    1.0,
                    time_step,
                    if budgeted {
                        budget
                    } else {
                        MAX_STEPS_PER_FRAME as f64
                    },
                );
                // The batch and the drawing share one queue, so the frame is as
                // long as the work it was given, never shorter than the display.
                frame = (OVERHEAD + asked as f64 * PER_STEP).max(display);
                if warm < 1.0 {
                    warm += frame;
                    continue;
                }
                elapsed += frame;
                frames += 1;
                steps += asked;
            }
            (
                f64::from(frames) / elapsed,
                steps as f64 * time_step / elapsed,
            )
        };

        let (linear_fps, linear_speed) = run(1.34e-3, false);
        assert!(
            linear_fps > 115.0,
            "linear was not display-bound: {linear_fps}"
        );
        assert!(
            linear_speed > 0.95,
            "linear did not keep up: {linear_speed}"
        );

        // The reported regression, with nothing bounding the batch.
        let (driven_fps, driven_speed) = run(6.27e-4, false);
        assert!(
            driven_fps < 80.0,
            "the reported drop did not reproduce: {driven_fps}"
        );
        assert!(driven_speed < 0.7, "it also fell behind: {driven_speed}");

        // Budgeted, the display is served and the shortfall is what gives.
        let (budgeted_fps, budgeted_speed) = run(6.27e-4, true);
        assert!(
            budgeted_fps > 110.0,
            "the display is still being held behind the solver: {budgeted_fps}"
        );
        assert!(
            budgeted_speed > 0.0,
            "the solver stopped advancing entirely: {budgeted_speed}"
        );
        // It advances many steps a frame, so every frame still has a new
        // configuration to draw - the display is never waiting on the solver
        // for something to show.
        assert!(
            budgeted_speed / budgeted_fps / 6.27e-4 > 1.0,
            "fewer than one step a frame: {budgeted_speed}"
        );
    }

    /// The budget is an outcome, not a guess: an overrunning frame cuts it and
    /// frames inside the cadence give it back.
    #[test]
    fn the_batch_ceiling_follows_what_frames_actually_cost() {
        let cadence = 1.0 / 120.0;
        let budget = MAX_STEPS_PER_FRAME as f64;

        let overran = frame_step_budget(budget, cadence * 2.0, cadence);
        assert!(overran < budget, "an overrunning frame kept its batch");
        let recovered = frame_step_budget(overran, cadence, cadence);
        assert!(recovered > overran, "a cheap frame did not give any back");

        // It bottoms out at one rather than at zero: a solver that cannot fit
        // a step inside a frame still advances, one step at a time.
        let mut starved = budget;
        for _ in 0..200 {
            starved = frame_step_budget(starved, cadence * 10.0, cadence);
        }
        assert_eq!(starved, 1.0);

        // And it climbs back to the ceiling rather than staying shy of it.
        let mut recovering = starved;
        for _ in 0..200 {
            recovering = frame_step_budget(recovering, cadence * 0.5, cadence);
        }
        assert_eq!(recovering, MAX_STEPS_PER_FRAME as f64);

        // A frame just inside the tolerance is not held responsible.
        assert!(frame_step_budget(budget, cadence * 1.02, cadence) >= budget);

        // Nonsense leaves it alone.
        assert_eq!(frame_step_budget(budget, f64::NAN, cadence), budget);
        assert_eq!(frame_step_budget(budget, cadence, 0.0), budget);
    }

    /// The target is measured because the hardware is not announced. A faster
    /// display is believed at once; a slower one only after it stays slow, so a
    /// saturated solver cannot define its own slowness as the cadence.
    #[test]
    fn the_cadence_is_the_best_frame_seen_lately() {
        let held = 1.0 / 60.0;
        let faster = hold_cadence(held, 1.0 / 144.0, 1.0 / 144.0);
        assert!((faster - 1.0 / 144.0).abs() < 1.0e-12);

        // One slow frame barely moves it.
        let nudged = hold_cadence(faster, 1.0, 1.0 / 144.0);
        assert!(
            nudged < faster * 1.01,
            "one slow frame relaxed it: {nudged}"
        );

        // A second of slow frames does.
        let mut relaxed = faster;
        for _ in 0..120 {
            relaxed = hold_cadence(relaxed, 1.0, 1.0 / 120.0);
        }
        assert!(relaxed > faster * 1.1, "it never relaxed: {relaxed}");

        assert_eq!(hold_cadence(held, f64::NAN, 0.01), held);
        assert_eq!(hold_cadence(held, -1.0, 0.01), held);
    }

    /// Backlog is held only up to what a frame may spend. Holding a full ceiling
    /// of it while the budget is small would saturate every following frame
    /// trying to catch up, which is the opposite of the contract.
    #[test]
    fn a_small_budget_does_not_queue_a_backlog_it_will_never_spend() {
        let step = 1.0e-3;
        let mut accumulator = 0.0;
        for _ in 0..100 {
            steps_for_frame(&mut accumulator, 1.0, 8.0, step, 4.0);
        }
        assert!(
            accumulator <= 4.0 * step + 1.0e-12,
            "backlog beyond the budget: {accumulator}"
        );
    }

    #[test]
    fn handoff_withholds_steps_only_when_the_source_cannot_advance() {
        assert!(!canonical_steps_withheld(false, false));
        // A packed candidate drains the requests already published before the
        // UI calls begin_handoff.
        assert!(canonical_steps_withheld(true, false));
        // A fresh install has no accepted source generation to advance.
        assert!(canonical_steps_withheld(false, true));
        // Ordinary target upload and validation are not solver pauses. Later
        // requests become a target catch-up backlog after admission.
        assert!(!canonical_steps_withheld(false, false));
    }

    /// Below a ceiling of `recommended / PACING_FRAME_SECONDS`, pacing by step
    /// count alone leaves whole frames without one. The step shrinks there
    /// instead — always downward, since the mesh's figure is a stability limit.
    #[test]
    fn a_low_ceiling_shrinks_the_step_rather_than_skipping_frames() {
        // The GRIN rod's step at the default mesh, whose threshold is most of
        // the slider.
        let recommended = 6.6e-3;
        let threshold = recommended / PACING_FRAME_SECONDS;
        assert!(
            (0.7..0.85).contains(&threshold),
            "threshold moved: {threshold}"
        );

        // At and above it the mesh keeps its own step and the count does the work.
        assert_eq!(paced_time_step(recommended, 1.0), recommended);
        assert_eq!(paced_time_step(recommended, 2.0), recommended);
        assert!(
            paced_time_step(recommended, 1.0e6) <= recommended,
            "went above the limit"
        );

        // Below it the step follows the ceiling down.
        assert_eq!(
            paced_time_step(recommended, 0.1),
            PACING_FRAME_SECONDS * 0.1
        );
        assert_eq!(
            paced_time_step(recommended, 0.02),
            PACING_FRAME_SECONDS * 0.02
        );

        // Nonsense leaves the mesh's own step alone.
        assert_eq!(paced_time_step(recommended, 0.0), recommended);
        assert_eq!(paced_time_step(recommended, f64::NAN), recommended);
        assert_eq!(paced_time_step(recommended, -1.0), recommended);
    }

    /// What the cap is for: a frame's budget buys at least one step at every
    /// ceiling, on coarse meshes and fine. Without it a coarse mesh leaves half
    /// the frames unadvanced below 0.8x and the picture judders.
    #[test]
    fn a_frame_advances_at_every_ceiling() {
        for recommended in [6.6e-3, 8.9e-4, 6.2e-4] {
            for speed in [2.0, 1.0, 0.5, 0.2, 0.05, 0.02] {
                let step = paced_time_step(recommended, speed);
                assert!(step <= recommended, "{recommended:e} {speed}");
                let mut accumulator = 0.0;
                let idle = (0..600)
                    .filter(|_| {
                        steps_for_frame(
                            &mut accumulator,
                            PACING_FRAME_SECONDS,
                            speed,
                            step,
                            MAX_STEPS_PER_FRAME as f64,
                        ) == 0
                    })
                    .count();
                assert_eq!(
                    idle, 0,
                    "recommended {recommended:e} at {speed}x left {idle} frames unadvanced"
                );
            }
        }
    }

    /// The ceiling is on simulated seconds per wall second, so half the speed
    /// asks for half the steps out of the same frame.
    #[test]
    fn speed_scales_the_steps_a_frame_asks_for() {
        let step = 1.0e-3;
        let frame = 16.0e-3;
        let mut full = 0.0;
        let mut half = 0.0;
        let mut quiet = 0.0;
        let (mut full_total, mut half_total, mut quiet_total) = (0, 0, 0);
        for _ in 0..60 {
            full_total += steps_for_frame(&mut full, frame, 1.0, step, MAX_STEPS_PER_FRAME as f64);
            half_total += steps_for_frame(&mut half, frame, 0.5, step, MAX_STEPS_PER_FRAME as f64);
            quiet_total +=
                steps_for_frame(&mut quiet, frame, 0.02, step, MAX_STEPS_PER_FRAME as f64);
        }
        // A second of frames at one millisecond a step.
        assert_eq!(full_total, 960);
        assert_eq!(half_total, 480);
        assert_eq!(quiet_total, 19);
    }

    /// A late display frame drops missed wall time instead of asking the GPU to
    /// catch up and making the next frame later still. Very high requested
    /// speeds retain the independent solver-batch ceiling.
    #[test]
    fn late_frames_preserve_the_display_budget_and_cap_the_leftover() {
        let step = 1.0e-3;
        let mut on_time = 0.0;
        let mut late = 0.0;
        let expected = steps_for_frame(
            &mut on_time,
            1.0 / 60.0,
            1.0,
            step,
            MAX_STEPS_PER_FRAME as f64,
        );
        assert_eq!(
            steps_for_frame(&mut late, 1.0, 1.0, step, MAX_STEPS_PER_FRAME as f64),
            expected
        );
        assert!((late - on_time).abs() < 1.0e-12);

        let mut accumulator = 0.0;
        let steps = steps_for_frame(&mut accumulator, 1.0, 8.0, step, MAX_STEPS_PER_FRAME as f64);
        assert_eq!(steps, MAX_STEPS_PER_FRAME);
        assert!(
            accumulator <= MAX_STEPS_PER_FRAME as f64 * step + 1.0e-12,
            "the leftover built a backlog: {accumulator}"
        );
        // And it stays capped however long the solver is behind.
        for _ in 0..100 {
            steps_for_frame(&mut accumulator, 1.0, 8.0, step, MAX_STEPS_PER_FRAME as f64);
        }
        assert!(accumulator <= MAX_STEPS_PER_FRAME as f64 * step + 1.0e-12);

        // Nonsense asks for nothing rather than panicking or racing.
        let mut idle = 0.0;
        assert_eq!(
            steps_for_frame(&mut idle, 0.016, 1.0, 0.0, MAX_STEPS_PER_FRAME as f64),
            0
        );
        assert_eq!(
            steps_for_frame(&mut idle, 0.016, 0.0, 1.0e-3, MAX_STEPS_PER_FRAME as f64),
            0
        );
        assert_eq!(
            steps_for_frame(&mut idle, -1.0, 1.0, 1.0e-3, MAX_STEPS_PER_FRAME as f64),
            0
        );
    }

    #[test]
    fn gpu_backpressure_drops_requests_beyond_the_completed_lead() {
        assert_eq!(steps_with_gpu_backpressure(100, 100, 12), 12);
        assert_eq!(
            steps_with_gpu_backpressure(100, 150, 20),
            MAX_STEPS_PER_FRAME - 50
        );
        assert_eq!(steps_with_gpu_backpressure(100, 164, 20), 0);
        assert_eq!(steps_with_gpu_backpressure(100, 200, 20), 0);
    }

    /// The windowed rate dips whenever a handoff withholds stepping inside its
    /// window, at every speed and including ones the solver reaches easily, so
    /// the note reads a held best rather than the raw measurement.
    #[test]
    fn a_held_rate_rides_over_a_dip_but_not_a_slowdown() {
        let frame = 1.0 / 60.0;
        let mut held = 0.0;
        for _ in 0..120 {
            held = hold_rate(held, 1.0, frame);
        }
        assert_eq!(held, 1.0);

        // A second of the worst dip measured still reads as keeping up.
        let mut dipped = held;
        for _ in 0..60 {
            dipped = hold_rate(dipped, 0.74, frame);
        }
        assert!(
            speed_shortfall(dipped, 1.0, true).is_none(),
            "a dip was reported as a shortfall: {dipped}"
        );

        // A real slowdown gets through inside two seconds.
        let mut slow = held;
        for _ in 0..120 {
            slow = hold_rate(slow, 0.5, frame);
        }
        assert_eq!(speed_shortfall(slow, 1.0, true), Some(slow));

        // A better reading is taken at once, and nonsense is ignored.
        assert_eq!(hold_rate(0.5, 2.0, frame), 2.0);
        assert_eq!(hold_rate(0.5, f64::NAN, frame), 0.5);
        assert_eq!(hold_rate(0.5, -1.0, frame), 0.5);
    }

    /// A rate below the one asked for is worth saying, but only when the solver
    /// is actually trying to reach it.
    #[test]
    fn a_shortfall_is_only_reported_while_the_solver_is_trying() {
        assert_eq!(speed_shortfall(0.34, 1.0, true), Some(0.34));
        assert_eq!(
            speed_shortfall(0.98, 1.0, true),
            None,
            "jitter is not a shortfall"
        );
        assert_eq!(speed_shortfall(0.19, 0.2, true), None);
        assert_eq!(speed_shortfall(0.09, 0.2, true), Some(0.09));
        // Paused, mid-handoff, or before anything has been measured.
        assert_eq!(speed_shortfall(0.34, 1.0, false), None);
        assert_eq!(speed_shortfall(0.0, 1.0, true), None);
        assert_eq!(speed_shortfall(f64::NAN, 1.0, true), None);
    }
}
