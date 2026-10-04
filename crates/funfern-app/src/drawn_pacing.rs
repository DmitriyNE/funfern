//! The drawn cadence: how far the simulation each rendered frame shows has
//! advanced, and when.
//!
//! A frame's compute pass and its drawing are encoded into one command buffer,
//! the drawing sampling the field the compute has just written, so the state a
//! frame draws is the render world's encoded step count after that frame's
//! encode. Neither counter the main world can see is that. The requested count
//! is a frame ahead under pipelined rendering and is clamped by the render
//! world's lead fence. The completed count arrives with a readback, several
//! frames late and in bursts: zero on most frames, then a few dozen steps at
//! once.
//!
//! Setting `FUNFERN_PACING_TRACE` to a path makes the render world write one
//! CSV row per rendered frame there; `examples/drawn_pacing_summary.rs` reads
//! one back and prints [`DrawnSummary`]. When a frame is shown is not
//! available: wgpu keeps the presented drawable private. Two clocks are
//! recorded instead, the render world's host time at encode and, where the
//! device has timestamp queries, the GPU's at the start and end of the
//! frame's solver pass. Both are exact up to the vsync that follows.

use std::{
    collections::VecDeque,
    fmt,
    fs::File,
    io::{BufWriter, Write},
    path::Path,
    time::Instant,
};

use bevy::prelude::*;

use crate::gpu_frame_timer::GpuFrameReadings;

/// The environment variable naming the trace file.
pub const TRACE_VARIABLE: &str = "FUNFERN_PACING_TRACE";

/// What the main world's pacing decided for a frame. Carried to the render
/// world with the solver request, for the trace alone.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PacingNote {
    /// The main world's frame delta, in seconds.
    pub frame_seconds: f64,
    /// The speed setting, simulated seconds per wall second.
    pub speed: f64,
    /// The solver's step, in simulated seconds.
    pub time_step: f64,
    /// The batch ceiling the frame was paced under.
    pub ceiling: f64,
    /// Steps the accumulator asked for.
    pub asked: u64,
    /// Steps admitted past the backpressure clamp, and requested.
    pub admitted: u64,
    /// Whether the simulation is running rather than paused.
    pub running: bool,
    /// Whether a handoff withheld stepping this frame.
    pub withheld: bool,
    /// The state readback the frame painted: its serial, and the step the
    /// control readback carried at the time. The field a frame shows is the
    /// latest readback's, uploaded to the painter, not the state the frame
    /// encoded.
    pub picture: u64,
    pub picture_step: u64,
    /// The generation the painted readback belongs to; it lags the drawn
    /// generation across a handoff.
    pub picture_generation: u64,
    /// The solver's steady share of the display interval as the controller
    /// saw it, NaN when unknown.
    pub solver_share: f64,
}

/// One rendered frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DrawnFrame {
    /// Render-world frames since the trace started.
    pub frame: u64,
    /// The render world's host clock, seconds since the trace started.
    pub host_seconds: f64,
    /// The generation whose state the frame draws.
    pub generation: u64,
    /// That generation's encoded step count after the frame's encode: the
    /// step the frame draws.
    pub drawn_step: u64,
    /// The main world's requested count, as extracted for the frame.
    pub requested: u64,
    /// The read-back completed count, as the frame saw it.
    pub completed: u64,
    /// The GPU clock at the start and end of the frame's solver pass, seconds
    /// since the timer's first reading. None where the frame had no pass or
    /// the device no timestamp queries.
    pub gpu: Option<(f64, f64)>,
    pub note: PacingNote,
}

const COLUMNS: &str = "frame,host_seconds,generation,drawn_step,requested,completed,\
gpu_begin,gpu_end,frame_seconds,speed,time_step,ceiling,asked,admitted,running,withheld,\
picture,picture_step,picture_generation,solver_share";

impl DrawnFrame {
    fn csv(&self) -> String {
        let [begin, end] = self.gpu.map_or(Default::default(), |(begin, end)| {
            [begin, end].map(|seconds| seconds.to_string())
        });
        let note = &self.note;
        format!(
            "{},{},{},{},{},{},{begin},{end},{},{},{},{},{},{},{},{},{},{},{},{}",
            self.frame,
            self.host_seconds,
            self.generation,
            self.drawn_step,
            self.requested,
            self.completed,
            note.frame_seconds,
            note.speed,
            note.time_step,
            note.ceiling,
            note.asked,
            note.admitted,
            u8::from(note.running),
            u8::from(note.withheld),
            note.picture,
            note.picture_step,
            note.picture_generation,
            note.solver_share,
        )
    }

    fn parse(line: &str) -> Result<Self, String> {
        let fields: Vec<&str> = line.split(',').collect();
        if fields.len() != COLUMNS.split(',').count() {
            return Err(format!(
                "expected {} fields: {line}",
                COLUMNS.split(',').count()
            ));
        }
        fn number<T: std::str::FromStr>(field: &str) -> Result<T, String> {
            field
                .parse()
                .map_err(|_| format!("not a number: {field:?}"))
        }
        let gpu = match (fields[6], fields[7]) {
            ("", "") => None,
            (begin, end) => Some((number(begin)?, number(end)?)),
        };
        Ok(Self {
            frame: number(fields[0])?,
            host_seconds: number(fields[1])?,
            generation: number(fields[2])?,
            drawn_step: number(fields[3])?,
            requested: number(fields[4])?,
            completed: number(fields[5])?,
            gpu,
            note: PacingNote {
                frame_seconds: number(fields[8])?,
                speed: number(fields[9])?,
                time_step: number(fields[10])?,
                ceiling: number(fields[11])?,
                asked: number(fields[12])?,
                admitted: number(fields[13])?,
                running: fields[14] == "1",
                withheld: fields[15] == "1",
                picture: number(fields[16])?,
                picture_step: number(fields[17])?,
                picture_generation: number(fields[18])?,
                solver_share: number(fields[19])?,
            },
        })
    }
}

/// Reads a trace the render world wrote. A last line cut short by the process
/// ending is dropped.
pub fn parse_trace(text: &str) -> Result<Vec<DrawnFrame>, String> {
    let mut lines = text.lines();
    if lines.next() != Some(COLUMNS) {
        return Err("not a drawn pacing trace".into());
    }
    let lines: Vec<&str> = lines.filter(|line| !line.is_empty()).collect();
    let mut frames = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        match DrawnFrame::parse(line) {
            Ok(frame) => frames.push(frame),
            Err(_) if index + 1 == lines.len() => {}
            Err(error) => return Err(error),
        }
    }
    Ok(frames)
}

/// Frames a window spans when measuring the clock's rate: about a fifteenth of
/// a second at 120 Hz, roughly what the eye integrates. The same window as
/// the pacing harness's `wobble`, so the figures compare.
pub const WOBBLE_WINDOW: usize = 8;

/// How evenly the drawn state advanced over a trace.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DrawnSummary {
    /// Frame intervals measured: both ends running, on one generation and
    /// step.
    pub intervals: usize,
    /// Wall seconds they span, on the host clock.
    pub seconds: f64,
    /// Frames a second, on the host clock.
    pub fps: f64,
    /// Intervals the main world measured at more than one and a half times
    /// its median frame delta: the frames that came in late.
    pub late: f64,
    /// The speed setting, the median over the intervals.
    pub speed: f64,
    /// Drawn simulated seconds per wall second.
    pub drawn_speed: f64,
    /// The relative spread of drawn simulated seconds per wall second over
    /// sliding windows of [`WOBBLE_WINDOW`] intervals, on the host clock.
    pub wobble: f64,
    /// The same over the windows no handoff withheld stepping in: what is
    /// left once the freezes the handoffs cost are set aside.
    pub steady_wobble: f64,
    /// The same against the GPU clock: the end of each solver pass, over the
    /// frames that had one.
    pub gpu_wobble: Option<f64>,
    /// The same statistic on the main world's series: steps it requested over
    /// its own frame deltas. The proxy pacing was tuned against.
    pub requested_wobble: f64,
    /// The same on the read-back completed count against the host clock.
    pub completed_wobble: f64,
    /// Each interval's drawn advance over `speed x interval`: p10, p50, p90.
    pub advance: [f64; 3],
    /// Intervals across which the drawn state did not move.
    pub frozen: f64,
    /// Of those, the ones a handoff withheld stepping on.
    pub frozen_withheld: f64,
    /// Intervals that advanced more than one and a half times the median
    /// advance: a frame that catches up a frozen one draws twice it.
    pub jumps: f64,
    /// Intervals across which the read-back completed count did not move.
    pub completed_still: f64,
    /// Intervals across which the picture did not change: the same readback
    /// painted twice.
    pub picture_still: f64,
    /// The wobble of the picture's own clock: the step the painted readback
    /// carried, over host time.
    pub picture_wobble: f64,
    /// How often the batch ceiling came down, per wall second.
    pub cuts_per_second: f64,
    /// Steps requested but not yet encoded at each frame: p50, p90.
    pub backlog: [f64; 2],
    /// Milliseconds of GPU time each solver pass took: p50, p90, max.
    pub gpu_milliseconds: Option<[f64; 3]>,
}

fn percentile(values: &mut [f64], fraction: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    values.sort_by(|left, right| left.total_cmp(right));
    let index = ((values.len() - 1) as f64 * fraction).round() as usize;
    values[index]
}

/// The relative spread of `advanced / wall` over sliding windows, each run of
/// consecutive intervals taken on its own.
fn wobble(runs: &[Vec<(f64, f64)>]) -> f64 {
    let rates: Vec<f64> = runs
        .iter()
        .flat_map(|run| run.windows(WOBBLE_WINDOW))
        .filter_map(|window| {
            let advanced: f64 = window.iter().map(|(advanced, _)| advanced).sum();
            let wall: f64 = window.iter().map(|(_, wall)| wall).sum();
            (wall > 0.0).then_some(advanced / wall)
        })
        .collect();
    if rates.is_empty() {
        return f64::NAN;
    }
    let mean = rates.iter().sum::<f64>() / rates.len() as f64;
    let variance = rates.iter().map(|rate| (rate - mean).powi(2)).sum::<f64>() / rates.len() as f64;
    variance.sqrt() / mean.max(f64::MIN_POSITIVE)
}

/// Whether the interval from `before` to `after` can be measured: both
/// running, drawing one generation at one step, and the clock not rewound.
fn comparable(before: &DrawnFrame, after: &DrawnFrame) -> bool {
    before.note.running
        && after.note.running
        && before.generation == after.generation
        && before.note.time_step == after.note.time_step
        && after.note.time_step > 0.0
        && after.drawn_step >= before.drawn_step
        && after.completed >= before.completed
        && after.host_seconds > before.host_seconds
}

/// Splits a series of measurements into runs, starting a new one wherever an
/// interval cannot be measured.
fn runs<T>(items: impl Iterator<Item = Option<T>>) -> Vec<Vec<T>> {
    let mut runs = vec![Vec::new()];
    for item in items {
        match item {
            Some(item) => runs.last_mut().expect("never empty").push(item),
            None if runs.last().is_some_and(|run| !run.is_empty()) => runs.push(Vec::new()),
            None => {}
        }
    }
    runs
}

/// Summarises a trace. The first `skip_seconds` of host time are left out,
/// for the warm-up.
pub fn summarize(frames: &[DrawnFrame], skip_seconds: f64) -> DrawnSummary {
    let start = frames.first().map_or(0.0, |frame| frame.host_seconds) + skip_seconds;
    let frames: Vec<&DrawnFrame> = frames
        .iter()
        .filter(|frame| frame.host_seconds >= start)
        .collect();
    let pairs: Vec<Option<(&DrawnFrame, &DrawnFrame)>> = frames
        .windows(2)
        .map(|pair| comparable(pair[0], pair[1]).then_some((pair[0], pair[1])))
        .collect();
    let measured: Vec<(&DrawnFrame, &DrawnFrame)> = pairs.iter().flatten().copied().collect();
    let drawn = |before: &DrawnFrame, after: &DrawnFrame| {
        (after.drawn_step - before.drawn_step) as f64 * after.note.time_step
    };
    let wall = |before: &DrawnFrame, after: &DrawnFrame| after.host_seconds - before.host_seconds;

    let seconds: f64 = measured
        .iter()
        .map(|(before, after)| wall(before, after))
        .sum();
    let advanced: f64 = measured
        .iter()
        .map(|(before, after)| drawn(before, after))
        .sum();
    let mut speeds: Vec<f64> = measured.iter().map(|(_, after)| after.note.speed).collect();
    let speed = percentile(&mut speeds, 0.5);
    let mut advance: Vec<f64> = measured
        .iter()
        .map(|(before, after)| drawn(before, after) / (after.note.speed * wall(before, after)))
        .collect();
    let mut steps: Vec<f64> = measured
        .iter()
        .map(|(before, after)| (after.drawn_step - before.drawn_step) as f64)
        .filter(|steps| *steps > 0.0)
        .collect();
    let median_steps = percentile(&mut steps, 0.5);
    let count = measured.len().max(1) as f64;
    let share = |predicate: &dyn Fn(&DrawnFrame, &DrawnFrame) -> bool| {
        measured
            .iter()
            .filter(|(before, after)| predicate(before, after))
            .count() as f64
            / count
    };
    let frozen = share(&|before, after| after.drawn_step == before.drawn_step);
    let frozen_withheld =
        share(&|before, after| after.drawn_step == before.drawn_step && after.note.withheld);
    let jumps =
        share(&|before, after| (after.drawn_step - before.drawn_step) as f64 > 1.5 * median_steps);
    let completed_still = share(&|before, after| after.completed == before.completed);
    let picture_still = share(&|before, after| after.note.picture == before.note.picture);
    let mut deltas: Vec<f64> = measured
        .iter()
        .map(|(_, after)| after.note.frame_seconds)
        .collect();
    let median_delta = percentile(&mut deltas, 0.5);
    let late = share(&|_, after| after.note.frame_seconds > 1.5 * median_delta);
    let cuts = measured
        .iter()
        .filter(|(before, after)| after.note.ceiling < before.note.ceiling)
        .count();

    let host = runs(
        pairs
            .iter()
            .map(|pair| pair.map(|(before, after)| (drawn(before, after), wall(before, after)))),
    );
    let steady = runs(pairs.iter().map(|pair| {
        pair.filter(|(_, after)| !after.note.withheld)
            .map(|(before, after)| (drawn(before, after), wall(before, after)))
    }));
    let requested = runs(pairs.iter().map(|pair| {
        pair.map(|(_, after)| {
            (
                after.note.admitted as f64 * after.note.time_step,
                after.note.frame_seconds,
            )
        })
    }));
    let picture = runs(pairs.iter().map(|pair| {
        // A readback's step is zero until its generation's first clock lands.
        pair.filter(|(before, after)| {
            after.note.picture_generation == before.note.picture_generation
                && before.note.picture_step > 0
                && after.note.picture_step >= before.note.picture_step
        })
        .map(|(before, after)| {
            (
                (after.note.picture_step - before.note.picture_step) as f64 * after.note.time_step,
                wall(before, after),
            )
        })
    }));
    let completed = runs(pairs.iter().map(|pair| {
        pair.map(|(before, after)| {
            (
                (after.completed - before.completed) as f64 * after.note.time_step,
                wall(before, after),
            )
        })
    }));
    // On the GPU clock a frame without a pass has no new state to time, so the
    // series runs over the frames that had one, and is broken wherever the
    // host series is.
    let mut gpu_series = Vec::new();
    let mut previous: Option<&DrawnFrame> = None;
    for (index, frame) in frames.iter().enumerate() {
        let measurable = index == 0 || pairs[index - 1].is_some();
        if !measurable {
            gpu_series.push(None);
            previous = None;
        }
        let Some((_, end)) = frame.gpu else {
            continue;
        };
        if let Some(before) = previous
            && let Some((_, before_end)) = before.gpu
            && end > before_end
        {
            gpu_series.push(Some((drawn(before, frame), end - before_end)));
        }
        previous = Some(frame);
    }
    let gpu_runs = runs(gpu_series.into_iter());
    let mut gpu_milliseconds: Vec<f64> = frames
        .iter()
        .filter(|frame| frame.note.running)
        .filter_map(|frame| frame.gpu.map(|(begin, end)| (end - begin) * 1.0e3))
        .collect();
    let gpu_milliseconds = (!gpu_milliseconds.is_empty()).then(|| {
        let max = gpu_milliseconds.iter().copied().fold(0.0, f64::max);
        [
            percentile(&mut gpu_milliseconds, 0.5),
            percentile(&mut gpu_milliseconds, 0.9),
            max,
        ]
    });
    let mut backlog: Vec<f64> = frames
        .iter()
        .filter(|frame| frame.note.running)
        .map(|frame| frame.requested.saturating_sub(frame.drawn_step) as f64)
        .collect();

    DrawnSummary {
        intervals: measured.len(),
        seconds,
        fps: measured.len() as f64 / seconds.max(f64::MIN_POSITIVE),
        late,
        speed,
        drawn_speed: advanced / seconds.max(f64::MIN_POSITIVE),
        wobble: wobble(&host),
        steady_wobble: wobble(&steady),
        gpu_wobble: gpu_runs
            .iter()
            .any(|run| run.len() >= WOBBLE_WINDOW)
            .then(|| wobble(&gpu_runs)),
        requested_wobble: wobble(&requested),
        completed_wobble: wobble(&completed),
        advance: [
            percentile(&mut advance, 0.1),
            percentile(&mut advance, 0.5),
            percentile(&mut advance, 0.9),
        ],
        frozen,
        frozen_withheld,
        jumps,
        completed_still,
        picture_still,
        picture_wobble: wobble(&picture),
        cuts_per_second: cuts as f64 / seconds.max(f64::MIN_POSITIVE),
        backlog: [percentile(&mut backlog, 0.5), percentile(&mut backlog, 0.9)],
        gpu_milliseconds,
    }
}

impl fmt::Display for DrawnSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let percent = |value: f64| format!("{:.1} %", value * 100.0);
        writeln!(
            f,
            "intervals {} over {:.2} s, {:.1} fps, late {}",
            self.intervals,
            self.seconds,
            self.fps,
            percent(self.late)
        )?;
        writeln!(
            f,
            "speed: set {:.4}, drawn {:.4} ({:.3} of set)",
            self.speed,
            self.drawn_speed,
            self.drawn_speed / self.speed
        )?;
        writeln!(
            f,
            "wobble over {WOBBLE_WINDOW} frames: drawn {}, away from handoffs {} (GPU clock {}), requested {}, completed {}",
            percent(self.wobble),
            percent(self.steady_wobble),
            self.gpu_wobble.map_or("n/a".into(), percent),
            percent(self.requested_wobble),
            percent(self.completed_wobble),
        )?;
        writeln!(
            f,
            "advance / (speed x interval): p10 {:.3}, p50 {:.3}, p90 {:.3}",
            self.advance[0], self.advance[1], self.advance[2]
        )?;
        writeln!(
            f,
            "frozen {} (withheld {}), jumps {}, completed still {}",
            percent(self.frozen),
            percent(self.frozen_withheld),
            percent(self.jumps),
            percent(self.completed_still),
        )?;
        writeln!(
            f,
            "picture: still {}, wobble {}",
            percent(self.picture_still),
            percent(self.picture_wobble),
        )?;
        write!(
            f,
            "ceiling cuts {:.2} a second; requested ahead of drawn: p50 {:.0}, p90 {:.0} steps",
            self.cuts_per_second, self.backlog[0], self.backlog[1]
        )?;
        if let Some([p50, p90, max]) = self.gpu_milliseconds {
            write!(
                f,
                "\nsolver pass on the GPU: p50 {p50:.3} ms, p90 {p90:.3} ms, max {max:.3} ms"
            )?;
        }
        Ok(())
    }
}

/// Frames a row waits for its GPU times before it is written without them.
const TIMESTAMP_PATIENCE: u64 = 64;

/// The render world's recorder, present only while tracing.
#[derive(Resource)]
pub struct DrawnPacingTrace {
    writer: BufWriter<File>,
    started: Instant,
    frames: u64,
    /// Rows not yet written, and the GPU frame each waits on.
    pending: VecDeque<(DrawnFrame, Option<u64>)>,
}

impl DrawnPacingTrace {
    /// Opens the trace named by [`TRACE_VARIABLE`], if it names one.
    pub fn from_environment() -> Option<Self> {
        let path = std::env::var_os(TRACE_VARIABLE)?;
        match Self::create(Path::new(&path)) {
            Ok(trace) => Some(trace),
            Err(error) => {
                warn!("cannot write the drawn pacing trace: {error}");
                None
            }
        }
    }

    fn create(path: &Path) -> std::io::Result<Self> {
        let mut writer = BufWriter::new(File::create(path)?);
        writeln!(writer, "{COLUMNS}")?;
        writer.flush()?;
        Ok(Self {
            writer,
            started: Instant::now(),
            frames: 0,
            pending: VecDeque::new(),
        })
    }

    /// Records the frame, called once a frame after the solver's encode.
    /// `gpu_frame` is the GPU frame timer's count for it, if its solver pass
    /// is being timed.
    pub fn record(&mut self, mut row: DrawnFrame, gpu_frame: Option<u64>) {
        row.frame = self.frames;
        row.host_seconds = self.started.elapsed().as_secs_f64();
        self.pending.push_back((row, gpu_frame));
        self.frames += 1;
    }

    /// Takes in the GPU times that have arrived and writes every row no
    /// longer waiting. Called after the frame's submission.
    pub fn collect(&mut self, readings: Option<&GpuFrameReadings>) {
        if let Some(readings) = readings {
            for reading in readings.recent() {
                if let Some((row, waiting)) = self
                    .pending
                    .iter_mut()
                    .find(|(_, waiting)| *waiting == Some(reading.frame))
                {
                    row.gpu = Some((reading.pass_begin, reading.pass_end));
                    *waiting = None;
                }
            }
        }
        let frames = self.frames;
        while let Some((row, waiting)) = self.pending.front() {
            if waiting.is_some() && frames - row.frame < TIMESTAMP_PATIENCE {
                break;
            }
            let line = row.csv();
            self.pending.pop_front();
            if writeln!(self.writer, "{line}").is_err() {
                break;
            }
        }
        let _ = self.writer.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(index: u64, seconds: f64, step: u64, completed: u64) -> DrawnFrame {
        DrawnFrame {
            frame: index,
            host_seconds: seconds,
            generation: 1,
            drawn_step: step,
            requested: step + 2,
            completed,
            gpu: Some((seconds, seconds + 0.001)),
            note: PacingNote {
                frame_seconds: 1.0 / 120.0,
                speed: 1.0,
                time_step: 1.0 / 1200.0,
                ceiling: 12.0,
                asked: 10,
                admitted: 10,
                running: true,
                withheld: false,
                picture: index,
                picture_step: step.saturating_sub(20),
                picture_generation: 1,
                solver_share: 0.3,
            },
        }
    }

    #[test]
    fn a_trace_reads_back_what_was_written() {
        let mut rows = vec![frame(0, 0.0, 0, 0), frame(1, 0.008, 10, 0)];
        rows[1].gpu = None;
        rows[1].note.withheld = true;
        let text = std::iter::once(COLUMNS.to_owned())
            .chain(rows.iter().map(DrawnFrame::csv))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(parse_trace(&text).unwrap(), rows);
        // A last line the process was killed while writing is dropped.
        let cut = format!("{text}\n2,0.01");
        assert_eq!(parse_trace(&cut).unwrap(), rows);
    }

    /// Steady drawing at the set speed reads as no wobble at all, while the
    /// read-back count, arriving every third frame in a lump, reads as a
    /// large one and as still on two frames in three.
    #[test]
    fn steady_drawing_is_told_apart_from_a_bursty_readback() {
        let rows: Vec<DrawnFrame> = (0..240u64)
            .map(|index| frame(index, index as f64 / 120.0, 10 * index, 30 * (index / 3)))
            .collect();
        let summary = summarize(&rows, 0.0);
        assert_eq!(summary.intervals, 239);
        assert!((summary.drawn_speed - 1.0).abs() < 1.0e-9);
        assert!(summary.wobble < 1.0e-9, "{}", summary.wobble);
        assert!(summary.gpu_wobble.unwrap() < 1.0e-9);
        assert!(summary.requested_wobble < 1.0e-9);
        assert!(
            summary.completed_wobble > 0.1,
            "{}",
            summary.completed_wobble
        );
        assert!((summary.completed_still - 2.0 / 3.0).abs() < 0.01);
        assert_eq!(summary.frozen, 0.0);
        assert_eq!(summary.jumps, 0.0);
        assert_eq!(summary.backlog, [2.0, 2.0]);
        assert!((summary.advance[1] - 1.0).abs() < 1.0e-9);
        assert!((summary.gpu_milliseconds.unwrap()[0] - 1.0).abs() < 1.0e-6);
    }

    /// A frame that draws nothing new, then one that catches up, is a freeze
    /// and a jump, and the wobble sees them.
    #[test]
    fn a_skipped_frame_is_frozen_then_a_jump() {
        let mut step = 0;
        let rows: Vec<DrawnFrame> = (0..240u64)
            .map(|index| {
                if index % 10 != 5 {
                    step += if index % 10 == 6 { 20 } else { 10 };
                }
                frame(index, index as f64 / 120.0, step, 0)
            })
            .collect();
        let summary = summarize(&rows, 0.0);
        assert!((summary.frozen - 0.1).abs() < 0.01, "{}", summary.frozen);
        assert!((summary.jumps - 0.1).abs() < 0.01, "{}", summary.jumps);
        assert!(summary.wobble > 0.02, "{}", summary.wobble);
        assert!((summary.advance[1] - 1.0).abs() < 1.0e-9);
    }

    /// A handoff's freeze is what the eye sees, so it counts in the wobble,
    /// but the steady figure sets it aside and reads the rest.
    #[test]
    fn a_handoff_freeze_is_set_aside_by_the_steady_wobble() {
        let mut step = 0;
        let rows: Vec<DrawnFrame> = (0..240u64)
            .map(|index| {
                let withheld = (100..103).contains(&index);
                if !withheld {
                    step += 10;
                }
                let mut row = frame(index, index as f64 / 120.0, step, 0);
                row.note.withheld = withheld;
                row
            })
            .collect();
        let summary = summarize(&rows, 0.0);
        assert!(summary.wobble > 0.01, "{}", summary.wobble);
        assert!(summary.steady_wobble < 1.0e-9, "{}", summary.steady_wobble);
        assert_eq!(summary.frozen, summary.frozen_withheld);
    }

    /// Paused frames, a new generation and a changed step each break the
    /// series instead of reading as a freeze or a jump.
    #[test]
    fn what_cannot_be_compared_is_left_out() {
        let mut rows: Vec<DrawnFrame> = (0..60u64)
            .map(|index| frame(index, index as f64 / 120.0, 10 * index, 0))
            .collect();
        for row in &mut rows[20..25] {
            row.note.running = false;
        }
        for row in &mut rows[40..] {
            row.generation = 2;
            row.drawn_step -= 300;
        }
        let summary = summarize(&rows, 0.0);
        // Six intervals touch the paused frames and one spans the two
        // generations.
        assert_eq!(summary.intervals, 59 - 6 - 1);
        assert_eq!(summary.frozen, 0.0);
        assert!(summary.wobble < 1.0e-9);
    }
}
