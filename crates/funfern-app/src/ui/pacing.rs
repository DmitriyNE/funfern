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
) -> FrameBatch {
    if !time_step.is_finite() || time_step <= 0.0 || !speed.is_finite() || speed <= 0.0 {
        return FrameBatch::default();
    }
    const DISPLAY_INTERVAL: f64 = 1.0 / 60.0;
    let ceiling = budget.clamp(1.0, MAX_STEPS_PER_FRAME as f64);
    *accumulator += delta.clamp(0.0, DISPLAY_INTERVAL) * speed;
    let wanted = (*accumulator / time_step).floor().max(0.0);
    let steps = wanted.min(ceiling) as u64;
    *accumulator -= steps as f64 * time_step;
    // Backlog is held only up to what a frame may actually spend. Holding more
    // would saturate every following frame trying to catch up, which is the
    // opposite of the contract: the shortfall is what gives, and it is
    // reported rather than queued.
    *accumulator = accumulator.min(ceiling * time_step);
    FrameBatch {
        steps,
        ceiling_bound: wanted >= ceiling,
    }
}

/// What a frame asked the solver for, and whether the ceiling is what decided
/// it.
#[derive(Clone, Copy, Default)]
pub(super) struct FrameBatch {
    pub(super) steps: u64,
    /// True when the batch ceiling, rather than the simulated time the frame
    /// had accumulated, is what limited the batch.
    ///
    /// The distinction is the difference between a solver that wants more room
    /// and one that has all it needs. Raising a ceiling nothing is pressing
    /// against buys no steps at all - the accumulator still decides - but it
    /// does let a single late frame spend a much larger catch-up burst later,
    /// and that burst is what overruns and takes the cut. Left ungated, the
    /// budget climbed to four times the batch in use and the simulated rate
    /// swung threefold at about 2 Hz, which is visible as the picture speeding
    /// up and slowing down.
    pub(super) ceiling_bound: bool,
}

/// How much longer than the display's own cadence a frame may run before the
/// solver's batch is held responsible.
///
/// Under vsync a frame time is quantized: the batch either fits inside a
/// refresh period or pushes presentation to the next one, so there is no
/// reading between one period and two and a wide band gives nothing away. What
/// the band must clear is the display's own jitter. Measured over 4670 frames
/// with the batch pinned at a single step - so every overrun was the display's,
/// not the solver's - 15.5 % of frames exceeded 1.05x the refresh period, 4.2 %
/// exceeded 1.25x and 1.7 % exceeded 1.5x. At `1.05` that false accusation rate
/// alone pinned the batch near a tenth of what the frame could afford. Anything
/// below `2.0` still catches a genuinely doubled frame.
const FRAME_BUDGET_TOLERANCE: f64 = 1.5;

/// What an overrunning frame multiplies the batch ceiling by.
///
/// Multiplicative, because this is the direction that protects the display:
/// when the cost of a step jumps - a finer mesh, a driven medium - the batch
/// has to come down in a few frames rather than a few hundred.
///
/// Sharp, because whenever the ceiling is the constraint the batch *is* the
/// ceiling, and the display has to be given back its cadence within a few
/// frames rather than a few hundred.
///
/// That sharpness is also the amplitude of the sawtooth the controller leaves
/// in the animation's own clock, so it was measured against a gentler `0.9`.
/// In the harness below, `0.9` is clearly better - the spread of simulated
/// seconds per wall second over eight-frame windows falls from 18.3 % to 7.9 %
/// where the solver has headroom and from 16.6 % to 7.8 % where it has none,
/// at no cost in frame rate in the first case and about nine frames a second in
/// the second. On the machine it could not be told apart from noise: two runs
/// of each build on the reported scene gave a controller-attributable spread of
/// 15.0 % and 20.2 % at `0.75` against 16.6 % and 18.1 % at `0.9`, run to run
/// variation larger than the effect. Left at `0.75` until there is a
/// measurement that can resolve it.
///
/// An earlier version of this comment justified `0.75` on the grounds that the
/// cut was the only thing producing a frame faster than the last, and so the
/// only way the cadence estimate learned what the display could do. That
/// stopped being true when the cadence moved to measuring frames the solver was
/// not loading; see [`DisplayCadence`].
const FRAME_BUDGET_BACKOFF: f64 = 0.75;

/// What a frame inside the cadence adds back to it, while the ceiling is what
/// the batch is pressing against.
///
/// Smaller than the backoff, because the controller can only find the edge by
/// crossing it and every crossing costs a presented frame. But it no longer has
/// to be tiny: growth happens only while [`FrameBatch::ceiling_bound`], so the
/// budget stops climbing the moment it is no longer the constraint, and the
/// probing that used to continue past that point is gone. Raising it from the
/// `0.05` that needed costs nothing where the solver has headroom and recovers
/// the requested rate in full: at the measured operating point the simulated
/// rate goes from 0.76x of what was asked for to 1.00x, at the same 119 fps,
/// with the spread of steps across frames narrowing from 5..11 to 8..12.
///
/// Where the solver genuinely cannot keep up it is a trade rather than a gain -
/// about six frames a second bought for a fifth more simulated speed - and the
/// right answer there is to govern the rate smoothly rather than to clamp it,
/// which this does not yet do.
const FRAME_BUDGET_RECOVERY: f64 = 0.25;

/// How many display-only frames the cadence is the median of.
///
/// Half a second at 120 Hz. Long enough that the median is stable to a tenth of
/// a millisecond, short enough to refill while a handoff withholds stepping.
const CADENCE_WINDOW: usize = 60;

/// The largest batch a frame may carry and still be read as timing the display.
///
/// One, not zero, because the budget floors at one step and a running solver
/// would otherwise never offer a reading. One step is the least load the solver
/// can impose while running, so this is the closest honest look at the display
/// available. Two is already too many: on a mesh costing 9 ms a step the pair
/// overflows a refresh period and the display reads as 24.9 ms rather than
/// 16.7.
const CADENCE_QUIET_STEPS: u64 = 1;

/// The frame interval the display is actually achieving, taken from frames the
/// solver was not loading.
///
/// This is measured rather than assumed because the target depends on hardware
/// nobody announces: 60, 120 and 144 Hz all want different budgets, and an
/// unthrottled window wants whatever it can reach.
///
/// Two things make it hard to measure, and an earlier version of this got both
/// wrong by tracking the fastest frame seen. A stall is followed by a very
/// short frame - 99.9 ms then 2.56 ms, in the trace that exposed this - so the
/// fastest frame is noise, not the display: it read 1.3 ms where the truth was
/// 8.32, after which every real frame looked like a threefold overrun and the
/// batch collapsed to one step and stayed there. But the obvious repair, a
/// median of recent frames, fails the other way: once the batch is large enough
/// to slow every frame, the median is the solver's own slowness and the
/// controller settles for it, at 60 fps of a 120 Hz display.
///
/// Both are avoided by choosing *which* frames to measure rather than how to
/// average them. A frame carrying at most [`CADENCE_QUIET_STEPS`] cannot have
/// been slowed by the solver, so its timing is the display's; a median over a
/// window of those rejects the stalls. Such frames are plentiful: every paused
/// frame, every frame a handoff withholds stepping on, and every frame while
/// the budget is still climbing from its floor. If the display genuinely slows,
/// the batch collapses, the window refills from the floor and the estimate
/// follows.
pub(super) struct DisplayCadence {
    quiet: Vec<f64>,
    pending_batch: u64,
    seconds: f64,
}

impl DisplayCadence {
    /// Seeded at 60 Hz: the slowest display worth assuming, so nothing is
    /// accused of overrunning before anything has been measured.
    pub(super) fn new() -> Self {
        Self {
            quiet: Vec::new(),
            pending_batch: 0,
            seconds: 1.0 / 60.0,
        }
    }

    /// Take this frame's timing as a reading of the display, if the batch this
    /// frame carried was small enough that it cannot be responsible for it.
    ///
    /// The batch is consumed, so a frame that asked for nothing - paused, or
    /// mid-handoff - reads as the display alone, which is what it is.
    pub(super) fn observe(&mut self, frame_seconds: f64) {
        let batch = core::mem::take(&mut self.pending_batch);
        if !frame_seconds.is_finite() || frame_seconds <= 0.0 || batch > CADENCE_QUIET_STEPS {
            return;
        }
        if self.quiet.len() == CADENCE_WINDOW {
            self.quiet.remove(0);
        }
        self.quiet.push(frame_seconds);
        let mut sorted = self.quiet.clone();
        sorted.sort_by(|left, right| left.total_cmp(right));
        self.seconds = sorted[sorted.len() / 2];
    }

    /// Record what the frame in progress asked the solver for.
    pub(super) fn record_batch(&mut self, steps: u64) {
        self.pending_batch = self.pending_batch.saturating_add(steps);
    }

    /// The display's frame interval in seconds.
    pub(super) fn seconds(&self) -> f64 {
        self.seconds
    }
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
pub(super) fn frame_step_budget(
    budget: f64,
    frame_seconds: f64,
    cadence: f64,
    batch: FrameBatch,
) -> f64 {
    if !frame_seconds.is_finite() || frame_seconds <= 0.0 || !cadence.is_finite() || cadence <= 0.0
    {
        return budget;
    }
    let next = if batch.steps > 0 && frame_seconds > cadence * FRAME_BUDGET_TOLERANCE {
        // Cut what the frame actually ran, not a ceiling it never reached. A
        // ceiling well above the batch in use would take several cuts before it
        // began to bite, and the display waits through every one of them.
        // A frame that ran no steps is not the solver's to answer for.
        budget.min(batch.steps as f64) * FRAME_BUDGET_BACKOFF
    } else if batch.ceiling_bound {
        budget + FRAME_BUDGET_RECOVERY
    } else {
        // The simulated time the frame had accumulated, not the ceiling, is
        // what limited the batch. More ceiling buys no steps and costs the
        // oscillation described on [`FrameBatch::ceiling_bound`].
        budget
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

    const OVERHEAD: f64 = 2.5e-3;

    /// A frame under vsync, driven by frame times this app actually measured.
    ///
    /// Presentation waits for a refresh boundary, so work that overflows a
    /// period is paid for a whole period at a time; but the renderer runs a
    /// frame behind the app, so an isolated spike is absorbed and only a
    /// sustained overrun costs frames. That is the `carry` below - overflow is
    /// banked and spends a period whenever it has earned one. On top of it the
    /// display has a spread of its own, which is what
    /// `measured-frame-deltas.txt` holds.
    ///
    /// An earlier version of this test modelled the frame as a smooth
    /// `overhead + steps x cost`, and that model cannot express either defect
    /// below: with no granularity, probing for the edge looks free, and with no
    /// jitter a cadence estimate cannot be fooled. Both shipped.
    struct Display {
        refresh: f64,
        jitter: Vec<f64>,
        next: usize,
        carry: f64,
    }

    impl Display {
        /// The measured spread, re-centred on `refresh`. The samples are from a
        /// 120 Hz panel; using them at another refresh rate assumes the spread
        /// is the compositor's rather than the panel's, which is what its shape
        /// - a stall and a short frame in pairs - suggests.
        fn measured(refresh: f64) -> Self {
            let samples: Vec<f64> = include_str!("measured-frame-deltas.txt")
                .lines()
                .filter(|line| !line.starts_with('#'))
                .map(|line| line.trim().parse::<f64>().expect("frame delta") * 1.0e-3)
                .collect();
            let mut sorted = samples.clone();
            sorted.sort_by(|left, right| left.total_cmp(right));
            let median = sorted[sorted.len() / 2];
            Self {
                refresh,
                jitter: samples.iter().map(|sample| sample - median).collect(),
                next: 0,
                carry: 0.0,
            }
        }

        /// How long the frame carrying `work` seconds of solver batch takes.
        fn present(&mut self, work: f64) -> f64 {
            self.carry += (work - self.refresh).max(0.0);
            let extra = (self.carry / self.refresh).floor();
            self.carry -= extra * self.refresh;
            let jitter = self.jitter[self.next % self.jitter.len()];
            self.next += 1;
            ((1.0 + extra) * self.refresh + jitter).max(1.0e-3)
        }
    }

    /// What four seconds of paced frames came to.
    struct Paced {
        /// Frames a second.
        fps: f64,
        /// Simulated seconds a wall second, over the whole run.
        speed: f64,
        /// Steps a frame, averaged.
        per_frame: f64,
        /// How unevenly the simulated clock ran, as the relative spread of
        /// simulated seconds per wall second over eight-frame windows.
        ///
        /// This is the one the eye reads. A run can hold its frame rate and
        /// deliver the requested speed on average while still visibly speeding
        /// up and slowing down, which is exactly what was reported.
        wobble: f64,
    }

    /// Four seconds of paced frames after a second of warm-up.
    fn paced(display: &mut Display, time_step: f64, per_step: f64, budgeted: bool) -> Paced {
        let mut accumulator = 0.0;
        let mut cadence = DisplayCadence::new();
        let mut budget = 1.0;
        let mut frame = display.refresh;
        let mut asked = FrameBatch::default();
        let mut run: Vec<(u64, f64)> = Vec::new();
        let (mut frames, mut steps, mut elapsed, mut warm) = (0u32, 0u64, 0.0, 0.0);
        while elapsed < 4.0 {
            cadence.observe(frame);
            if budgeted {
                budget = frame_step_budget(budget, frame, cadence.seconds(), asked);
            }
            asked = steps_for_frame(
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
            cadence.record_batch(asked.steps);
            frame = display.present(OVERHEAD + asked.steps as f64 * per_step);
            if warm < 1.0 {
                warm += frame;
                continue;
            }
            elapsed += frame;
            frames += 1;
            steps += asked.steps;
            run.push((asked.steps, frame));
        }

        // The simulated clock's rate over each eight-frame window - about a
        // fifteenth of a second, which is roughly what the eye integrates.
        const WINDOW: usize = 8;
        let rates: Vec<f64> = run
            .windows(WINDOW)
            .map(|window| {
                let advanced = window.iter().map(|(steps, _)| *steps).sum::<u64>() as f64;
                let wall = window.iter().map(|(_, frame)| *frame).sum::<f64>();
                advanced * time_step / wall
            })
            .collect();
        let mean = rates.iter().sum::<f64>() / rates.len().max(1) as f64;
        let variance =
            rates.iter().map(|rate| (rate - mean).powi(2)).sum::<f64>() / rates.len().max(1) as f64;

        Paced {
            fps: f64::from(frames) / elapsed,
            speed: steps as f64 * time_step / elapsed,
            per_frame: steps as f64 / f64::from(frames).max(1.0),
            wobble: variance.sqrt() / mean.max(f64::MIN_POSITIVE),
        }
    }

    /// The first reported defect: selecting a driven material took the frame
    /// rate from 120 to 65. A driven generation runs at a tighter step, so at
    /// the same requested speed it asks for proportionally more steps a frame,
    /// and the batch shares the frame's queue with drawing. Nothing bounded the
    /// batch by what it would cost, so the solver set the frame time.
    ///
    /// From the reported numbers: 8827 dofs, `dt` 1.34e-3 linear against
    /// 6.27e-4 driven, 0.93 ms a step fitted to both operating points.
    #[test]
    fn a_driven_step_does_not_take_the_display_down_with_it() {
        let refresh = 1.0 / 120.0;

        // Unbudgeted and linear, the step is loose enough that the batch fits:
        // 110 fps and 739 steps a second here against 120 and 750 reported.
        let linear = paced(&mut Display::measured(refresh), 1.34e-3, 0.93e-3, false);
        assert!(
            linear.fps > 105.0,
            "linear was not display-bound: {}",
            linear.fps
        );
        assert!(
            linear.speed > 0.9,
            "linear did not keep up: {}",
            linear.speed
        );

        // Unbudgeted and driven, the reported regression. It lands below the
        // reported 65 fps because nothing here models the GPU backpressure that
        // caps outstanding lead; the direction and the cause are the point.
        let unbudgeted = paced(&mut Display::measured(refresh), 6.27e-4, 0.93e-3, false);
        assert!(
            unbudgeted.fps < 80.0,
            "the reported drop did not reproduce: {}",
            unbudgeted.fps
        );

        // Budgeted, the display is served and the shortfall is what gives.
        let driven = paced(&mut Display::measured(refresh), 6.27e-4, 0.93e-3, true);
        assert!(
            driven.fps > 110.0,
            "the display is still held behind the solver: {}",
            driven.fps
        );
        assert!(
            driven.speed > 0.3,
            "the solver barely advanced: {}",
            driven.speed
        );
        // And it stays a simulation: several steps between one frame and the
        // next, so every frame has a new configuration to draw.
        assert!(
            driven.per_frame > 2.0,
            "the batch collapsed to a step a frame: {}",
            driven.per_frame
        );
    }

    /// How evenly the simulated clock runs, which is not the same question as
    /// whether the frame rate holds or whether the requested speed is reached
    /// on average. A run can do both and still visibly speed up and slow down.
    ///
    /// Whenever the ceiling is the constraint the batch is the ceiling, so the
    /// controller's sawtooth lands directly in the animation's clock: the
    /// spread here is [`FRAME_BUDGET_BACKOFF`] transmitted, against about 1 %
    /// for a pacer with no ceiling at all on the same frames. These bounds are
    /// a ratchet on what is currently reached - 18.3 % with room to spare and
    /// 16.6 % without - not a statement that this is good enough. Reducing it
    /// means a gentler cut, which measured better here and could not be
    /// distinguished from run-to-run variation on the machine.
    #[test]
    fn the_simulated_clock_runs_evenly_in_both_regimes() {
        let refresh = 1.0 / 120.0;

        // Headroom: a step cheap enough that the ceiling sits above demand.
        let easy = paced(&mut Display::measured(refresh), 8.13e-4, 0.4e-3, true);
        assert!(easy.fps > 110.0, "{}", easy.fps);
        assert!(
            easy.speed > 0.9,
            "it did not reach the requested rate: {}",
            easy.speed
        );
        assert!(
            easy.wobble < 0.20,
            "the simulated clock got less even with room to spare: {:.1} %",
            easy.wobble * 100.0
        );

        // And where the solver cannot keep up, falling behind should look like
        // slow motion rather than stutter: a lower rate, but no less even.
        let hard = paced(&mut Display::measured(refresh), 8.13e-4, 0.8e-3, true);
        assert!(hard.fps > 105.0, "{}", hard.fps);
        assert!(
            hard.speed < 0.95,
            "this operating point is meant to fall short: {}",
            hard.speed
        );
        assert!(
            hard.wobble < 0.20,
            "falling behind got stuttery rather than slow: {:.1} %",
            hard.wobble * 100.0
        );
    }

    /// The second reported defect, and the reason the cadence is measured only
    /// from frames the solver was not loading. Tracking the fastest frame seen
    /// latches onto the short frame that follows a stall - 99.9 ms then 2.56 ms
    /// in the trace that exposed this - after which every real frame reads as a
    /// threefold overrun. Measured in the app: the estimate sat at 2.56 ms
    /// against a true 8.32, and 2638 of 2670 frames carried a batch of exactly
    /// one step. The frame rate held; the simulation ran at a tenth of the
    /// speed asked for, which is how it was reported.
    #[test]
    fn the_cadence_is_not_fooled_by_the_frame_that_follows_a_stall() {
        let refresh = 1.0 / 120.0;
        let mut display = Display::measured(refresh);
        let mut cadence = DisplayCadence::new();
        let mut fastest = f64::INFINITY;
        for _ in 0..600 {
            let frame = display.present(OVERHEAD);
            cadence.observe(frame);
            fastest = fastest.min(frame);
        }
        assert!(
            fastest < refresh * 0.6,
            "the recorded trace has no stall recovery in it to be fooled by: {fastest}"
        );
        assert!(
            (cadence.seconds() - refresh).abs() < refresh * 0.05,
            "the cadence is not the refresh period: {} ms",
            cadence.seconds() * 1.0e3
        );
    }

    /// The mirror of it: a solver slow enough to hold every frame must not have
    /// its own slowness taken for the display's, which is how a plain median of
    /// recent frames fails - it settles for 60 fps of a 120 Hz panel.
    #[test]
    fn a_loaded_frame_is_not_a_reading_of_the_display() {
        let mut cadence = DisplayCadence::new();
        for _ in 0..120 {
            cadence.record_batch(40);
            cadence.observe(1.0 / 15.0);
        }
        assert_eq!(
            cadence.seconds(),
            1.0 / 60.0,
            "a slow batch redefined what the display can do"
        );

        // A frame that asked for nothing is the display alone.
        for _ in 0..120 {
            cadence.observe(1.0 / 144.0);
        }
        assert!((cadence.seconds() - 1.0 / 144.0).abs() < 1.0e-12);

        // So is one carrying the single step the budget floors at.
        for _ in 0..120 {
            cadence.record_batch(1);
            cadence.observe(1.0 / 120.0);
        }
        assert!((cadence.seconds() - 1.0 / 120.0).abs() < 1.0e-12);

        // Nonsense leaves the reading alone.
        cadence.observe(f64::NAN);
        cadence.observe(-1.0);
        cadence.observe(0.0);
        assert!((cadence.seconds() - 1.0 / 120.0).abs() < 1.0e-12);
    }

    /// A slower display is believed rather than accused of overrunning a faster
    /// one, which is the failure the fixed estimate had to avoid.
    #[test]
    fn a_slower_display_is_believed_rather_than_accused() {
        let slow = paced(&mut Display::measured(1.0 / 60.0), 6.27e-4, 0.93e-3, true);
        assert!(
            slow.fps > 55.0,
            "a 60 Hz display did not hold its own rate: {}",
            slow.fps
        );
        assert!(
            slow.per_frame > 4.0,
            "the batch collapsed on a slower display: {}",
            slow.per_frame
        );
    }

    /// The budget is an outcome, not a guess: an overrunning frame cuts it and
    /// frames inside the cadence give it back.
    #[test]
    fn the_batch_ceiling_follows_what_frames_actually_cost() {
        let cadence = 1.0 / 120.0;
        let budget = MAX_STEPS_PER_FRAME as f64;
        let pressing = |steps| FrameBatch {
            steps,
            ceiling_bound: true,
        };

        let overran = frame_step_budget(budget, cadence * 2.0, cadence, pressing(64));
        assert!(overran < budget, "an overrunning frame kept its batch");
        let recovered = frame_step_budget(overran, cadence, cadence, pressing(48));
        assert!(recovered > overran, "a cheap frame did not give any back");

        // It bottoms out at one rather than at zero: a solver that cannot fit
        // a step inside a frame still advances, one step at a time.
        let mut starved = budget;
        for _ in 0..200 {
            starved = frame_step_budget(starved, cadence * 10.0, cadence, pressing(64));
        }
        assert_eq!(starved, 1.0);

        // And it climbs back to the ceiling rather than staying shy of it.
        let mut recovering = starved;
        for _ in 0..300 {
            recovering = frame_step_budget(recovering, cadence * 0.5, cadence, pressing(64));
        }
        assert_eq!(recovering, MAX_STEPS_PER_FRAME as f64);

        // A frame just inside the tolerance is not held responsible.
        assert!(frame_step_budget(budget, cadence * 1.02, cadence, pressing(64)) >= budget);

        // Nonsense leaves it alone.
        assert_eq!(
            frame_step_budget(budget, f64::NAN, cadence, pressing(64)),
            budget
        );
        assert_eq!(
            frame_step_budget(budget, cadence, 0.0, pressing(64)),
            budget
        );
    }

    /// The reported oscillation: with room to spare the batch is decided by the
    /// simulated time a frame accumulated, not by the ceiling, and a ceiling
    /// nothing is pressing against must stop climbing. Left to climb it banks a
    /// catch-up burst that one late frame then spends all at once, which
    /// overruns, takes the cut, starves the frames after it and bursts again -
    /// measured at a threefold swing in the simulated rate every 0.57 s.
    #[test]
    fn a_ceiling_nothing_is_pressing_against_stops_climbing() {
        let cadence = 1.0 / 120.0;
        let slack = FrameBatch {
            steps: 11,
            ceiling_bound: false,
        };
        let mut budget = 15.0;
        for _ in 0..600 {
            budget = frame_step_budget(budget, cadence, cadence, slack);
        }
        assert_eq!(
            budget, 15.0,
            "the ceiling climbed with nothing asking for it"
        );

        // A frame that is pressing on it still moves it.
        let pressed = frame_step_budget(
            budget,
            cadence,
            cadence,
            FrameBatch {
                steps: 15,
                ceiling_bound: true,
            },
        );
        assert!(pressed > budget);
    }

    /// An overrun is answered by cutting the batch that actually ran. Cutting a
    /// ceiling the batch never reached would take several frames to bite, and
    /// the display waits through all of them.
    #[test]
    fn an_overrun_cuts_the_batch_that_ran_not_the_ceiling_above_it() {
        let cadence = 1.0 / 120.0;
        let cut = frame_step_budget(
            60.0,
            cadence * 2.0,
            cadence,
            FrameBatch {
                steps: 12,
                ceiling_bound: false,
            },
        );
        assert!(
            cut < 12.0,
            "the cut did not reach the batch that overran: {cut}"
        );

        // A frame that ran nothing is not the solver's to answer for.
        assert_eq!(
            frame_step_budget(60.0, cadence * 4.0, cadence, FrameBatch::default()),
            60.0
        );
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
                        )
                        .steps
                            == 0
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
            full_total +=
                steps_for_frame(&mut full, frame, 1.0, step, MAX_STEPS_PER_FRAME as f64).steps;
            half_total +=
                steps_for_frame(&mut half, frame, 0.5, step, MAX_STEPS_PER_FRAME as f64).steps;
            quiet_total +=
                steps_for_frame(&mut quiet, frame, 0.02, step, MAX_STEPS_PER_FRAME as f64).steps;
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
        )
        .steps;
        assert_eq!(
            steps_for_frame(&mut late, 1.0, 1.0, step, MAX_STEPS_PER_FRAME as f64).steps,
            expected
        );
        assert!((late - on_time).abs() < 1.0e-12);

        let mut accumulator = 0.0;
        let steps = steps_for_frame(&mut accumulator, 1.0, 8.0, step, MAX_STEPS_PER_FRAME as f64);
        assert_eq!(steps.steps, MAX_STEPS_PER_FRAME);
        assert!(steps.ceiling_bound, "the ceiling was what limited it");
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
            steps_for_frame(&mut idle, 0.016, 1.0, 0.0, MAX_STEPS_PER_FRAME as f64).steps,
            0
        );
        assert_eq!(
            steps_for_frame(&mut idle, 0.016, 0.0, 1.0e-3, MAX_STEPS_PER_FRAME as f64).steps,
            0
        );
        assert_eq!(
            steps_for_frame(&mut idle, -1.0, 1.0, 1.0e-3, MAX_STEPS_PER_FRAME as f64).steps,
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
