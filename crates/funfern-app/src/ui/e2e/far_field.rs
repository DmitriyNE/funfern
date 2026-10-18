//! PH-18's far field: an f64 twin of the far-field recorder, which the
//! fixture `far-field` holds the device's records to and the gallery's tests
//! (`topology_examples`) hold to the analytic patterns of a line source alone
//! and beside PH-12's cylinders.

use super::*;
use crate::document::FarFieldSettings;
use funfern_core::{
    ElectromagneticPolarization, PhysicsModel, PulseEnvelope, QuadraticFarFieldStencil,
};

/// The fixture's run: 6.5 s at the step the app paces its scene at, 5.66e-3.
pub(super) const FAR_FIELD_STEPS: u64 = 1150;

/// `far-field-late`'s clock: an epoch origin 36 hours in, the run starting
/// 400 steps before the epoch rebases, 253.7 s past the origin, and the drive
/// switched on 250 s past it.
pub(super) const LATE_ORIGIN: f64 = 129_600.0;
pub(super) const LATE_STEPS_BEFORE_REBASE: u32 = 400;
pub(super) const LATE_DRIVE: f64 = 250.0;

/// The cavity made a TM box whose every side is a first-order outgoing wall,
/// with a hole off its centre and the document's point source beside it, a
/// 2 Hz sine under a flat top switched on over half a second from `start`,
/// and its far field on at the default inset.
pub(super) fn far_field_document(start: f64) -> TopologyDocument {
    let mut editor = TopologyEditor::from_document(cavity_document()).expect("the cavity opens");
    editor
        .create_closed_curve(
            PeriodicCubicSpline::rounded(Point2::new(0.2, -0.15), 0.25),
            ClosedCurvePurpose::Hole,
        )
        .expect("the hole");
    let mut document = editor.document;
    document.model.draft.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    document.model.draft.outer_boundaries =
        OuterBoundaryConditions::uniform(OuterBoundaryCondition::FirstOrderOutgoing);
    document.model.accepted = document.model.draft.clone();
    document.model.source = funfern_core::PointSource {
        enabled: true,
        position: Point2::new(-0.4, 0.3),
        width: 0.05,
        region: funfern_core::BACKGROUND_REGION,
        signal: TimeSignal::Pulsed {
            offset: 0.0,
            amplitude: 1.0,
            frequency_hz: 2.0,
            phase_radians: 0.0,
            envelope: PulseEnvelope::FlatTop {
                duration: 20.0,
                edge: 0.5,
            },
            start,
            repeat: 0.0,
        },
    };
    document.model.far_field = FarFieldSettings {
        enabled: true,
        ..FarFieldSettings::default()
    };
    document
}

/// What the device's far field recorded, held to the twin: the host's
/// records, every one whose 96 directions all read, must stand a period and
/// at most a pass more apart from the first the delay window lets through to
/// the run's end; each
/// direction's amplitude within 4e-5 of the twin's at the same frame, relative
/// to the largest the twin reads, f32 against f64 (the M1 Max reads 5.3e-6);
/// each record's far time within 2e-6 s of the twin's (1.4e-7 s).
pub(super) fn judge(
    state: &Playground,
    twin: &FarFieldTwin,
    fixture: Fixture,
) -> Result<String, String> {
    // Late, the twin's f64 state and the device's stand 1.25e-4 apart (the
    // run's own tolerance says why), and the projections with them: the M1
    // Max reads 1.17e-4.
    let late = matches!(fixture, Fixture::FarFieldLate);
    let (tolerance, stamp_bound) = if late {
        (5.0e-4, 2.0e-6)
    } else {
        (4.0e-5, 2.0e-6)
    };
    let records = &state.far_field_trace.records;
    if records.len() < 60 {
        return Err(format!("the far field holds {} records", records.len()));
    }
    let (mut worst, mut largest, mut stamp) = (0.0_f64, 0.0_f64, 0.0_f64);
    for record in records {
        let now = record.time + twin.margin();
        let (far, expected) = twin
            .projection(now)
            .ok_or(format!("the twin holds no projection at {now} s"))?;
        grow(&mut stamp, (record.time - far).abs());
        if record.amplitude.len() != expected.len() {
            return Err(format!(
                "a record holds {} directions",
                record.amplitude.len()
            ));
        }
        for (read, exact) in record.amplitude.iter().zip(&expected) {
            grow(&mut worst, (f64::from(*read) - exact).abs());
            largest = largest.max(exact.abs());
        }
    }
    let (least, most) = (
        twin.period - stamp_bound,
        twin.period + twin.pass() + stamp_bound,
    );
    if let Some(pair) = records
        .iter()
        .collect::<Vec<_>>()
        .windows(2)
        .find(|pair| !(least..=most).contains(&(pair[1].time - pair[0].time)))
    {
        return Err(format!(
            "the far field's records skip from {} s to {} s",
            pair[0].time, pair[1].time
        ));
    }
    let worst = worst / largest;
    let summary = format!(
        "{} far-field records from {:.3} s, a period apart, amplitude {worst:.2e} of its largest, \
         stamps {stamp:.1e} s",
        records.len(),
        records[0].time
    );
    for (what, value, bound) in [
        ("an amplitude", worst, tolerance),
        ("a stamp", stamp, stamp_bound),
    ] {
        if !within(value, bound) {
            return Err(format!(
                "{summary}: {what} reads {value:.3e}, beyond {bound:.0e}"
            ));
        }
    }
    Ok(summary)
}

/// The largest of `worst` and `value`, a NaN carried through.
fn grow(worst: &mut f64, value: f64) {
    *worst = worse(*worst, value);
}

/// The recorder's samples per simulated second, `FAR_FIELD_SAMPLE_RATE`.
const SAMPLE_RATE: f64 = 60.0;
/// Its directions, `FAR_FIELD_DIRECTIONS`.
pub(crate) const DIRECTIONS: usize = 96;

/// One recorded contour frame: its stamp as the device forms it, the epoch
/// origin's coarse part and its fine part with the centred local time, the
/// time they make, and at each contour point the field, its rate and its
/// normal gradient.
struct Frame {
    stamp: (f32, f32),
    time: f64,
    values: Vec<[f64; 3]>,
}

/// The recorder in f64, as `canonical_far_field.wgsl` takes it. At every step
/// it reads each contour point's field `u = Σ wᵢQᵢ/Mᵢ` through the static
/// mass and its normal gradient, and centres a frame on the step: their means
/// over its two ends and the field's difference across it, at half a step
/// back. A pass, every `stride` steps, keeps a frame once its stamp stands a
/// period past the newest frame's, compared part by part in f32 as the shader
/// compares them. A projection at a frame sums `Δs (∂ₙu − (n·ŝ) u̇/c)` over
/// the contour, each point at its retarded time `now − margin + ŝ·y/c`, `now`
/// the frame's time, interpolated between the frames about it. The run starts
/// at rest, and the twin keeps the device's clock as the device keeps it: an
/// origin of whole seconds and a fraction, the epoch-local time `steps × dt`
/// in f32, and a rebase that adds the local time onto the origin by
/// `advance_origin`.
pub(crate) struct FarFieldTwin {
    stencil: Arc<QuadraticFarFieldStencil>,
    inverse_mass: Vec<f64>,
    time_step: f64,
    pub(crate) period: f64,
    stride: u64,
    previous: Option<Vec<(f64, f64)>>,
    frames: Vec<Frame>,
    origin: (f32, f32),
    epoch_step: u32,
    rebase_limit: u32,
    /// The device's completed-step count before the run's first step, which
    /// the clock's step in the epoch seeds, and which the host counts its
    /// passes on.
    first_serial: u64,
}

impl FarFieldTwin {
    /// The twin of a run that starts at rest at time zero, as the recorder's
    /// first pass differences its first step against.
    #[cfg(test)]
    pub(crate) fn new(active: &PreparedTopology, time_step: f64) -> Result<Self, String> {
        Self::with_clock(
            active,
            time_step,
            0.0,
            0,
            super::rebase_step_limit(time_step),
        )
    }

    /// The twin of a run that starts at rest on a clock at `origin`,
    /// `step_in_epoch` steps into the epoch, which rebases at `rebase_limit`.
    pub(crate) fn with_clock(
        active: &PreparedTopology,
        time_step: f64,
        origin: f64,
        step_in_epoch: u32,
        rebase_limit: u32,
    ) -> Result<Self, String> {
        let stencil = active
            .far_field
            .as_ref()
            .ok_or("the document's far field is off")?
            .as_ref()
            .map_err(|error| error.to_string())?
            .clone();
        let inverse_mass = active
            .canonical_operator
            .primary_mass()
            .iter()
            .map(|mass| 1.0 / mass)
            .collect();
        let mut period = 1.0 / SAMPLE_RATE;
        while period < time_step {
            period *= 2.0;
        }
        let stride = ((period / (4.0 * time_step)).floor() as u64).max(1);
        let rest = vec![(0.0, 0.0); stencil.samples.len()];
        Ok(Self {
            stencil,
            inverse_mass,
            time_step,
            period,
            stride,
            previous: Some(rest),
            frames: Vec::new(),
            origin: (origin.floor() as f32, (origin - origin.floor()) as f32),
            epoch_step: step_in_epoch,
            rebase_limit,
            first_serial: u64::from(step_in_epoch),
        })
    }

    pub(crate) fn margin(&self) -> f64 {
        self.stencil.delay_margin
    }

    /// Seconds between two recorder passes.
    pub(crate) fn pass(&self) -> f64 {
        self.stride as f64 * self.time_step
    }

    /// Moves the clock on a step, as the device does, and answers the stamp
    /// of a pass after it, as the shader forms it: the origin's coarse part,
    /// and its fine part with the epoch-local time centred half a step back.
    /// The encoder rebases before the step that would reach the limit, adding
    /// the local time onto the origin by `advance_origin`.
    fn step_clock(&mut self) -> (f32, f32) {
        let dt = self.time_step as f32;
        if self.epoch_step + 1 >= self.rebase_limit {
            let (high, low) = self.origin;
            let increment = self.epoch_step as f32 * dt;
            let whole = increment.floor();
            let fraction = low + (increment - whole);
            let carry = fraction.floor();
            self.origin = (high + (whole + carry), fraction - carry);
            self.epoch_step = 0;
        }
        self.epoch_step += 1;
        let local = self.epoch_step as f32 * dt;
        (self.origin.0, self.origin.1 + (local - 0.5 * dt))
    }

    /// `a − b` part by part, as the shader's `stamp_difference`.
    fn stamp_difference(a: (f32, f32), b: (f32, f32)) -> f32 {
        let coarse = if a.0.to_bits() == b.0.to_bits() {
            0.0
        } else {
            a.0 - b.0
        };
        coarse + (a.1 - b.1)
    }

    /// Reads the state after accepted step `step`, at `time`.
    pub(crate) fn observe(&mut self, step: u64, time: f64, primary: &[f64]) {
        let values = self
            .stencil
            .samples
            .iter()
            .map(|(point, _, normal)| {
                let (mut value, mut gradient) = (0.0, Point2::default());
                for ((node, weight), slope) in point
                    .nodes
                    .iter()
                    .zip(point.value_weights)
                    .zip(point.gradient_weights)
                {
                    let u = primary[*node as usize] * self.inverse_mass[*node as usize];
                    value += weight * u;
                    gradient = gradient + slope * u;
                }
                (value, gradient.dot(*normal))
            })
            .collect::<Vec<_>>();
        let stamp = self.step_clock();
        let previous = self.previous.replace(values.clone());
        let due = (self.first_serial + step).is_multiple_of(self.stride);
        let (Some(previous), true) = (previous, due) else {
            return;
        };
        if self
            .frames
            .last()
            .is_some_and(|frame| Self::stamp_difference(stamp, frame.stamp) < self.period as f32)
        {
            return;
        }
        debug_assert!(
            (f64::from(stamp.0) + f64::from(stamp.1) - (time - 0.5 * self.time_step)).abs()
                < 1.0e-3
        );
        self.frames.push(Frame {
            stamp,
            time: f64::from(stamp.0) + f64::from(stamp.1),
            values: values
                .iter()
                .zip(&previous)
                .map(|((u, normal), (before, normal_before))| {
                    [
                        0.5 * (u + before),
                        (u - before) / self.time_step,
                        0.5 * (normal + normal_before),
                    ]
                })
                .collect(),
        });
    }

    /// The projection the recorder took at the frame nearest `now`, from
    /// that frame's time, with its far time `now − margin`, by direction, or
    /// `None` where a retarded time falls outside the frames.
    pub(crate) fn projection(&self, now: f64) -> Option<(f64, Vec<f64>)> {
        let after = self.frames.partition_point(|frame| frame.time < now);
        let current = [after.checked_sub(1), Some(after)]
            .into_iter()
            .flatten()
            .filter(|index| *index < self.frames.len())
            .min_by(|a, b| {
                (self.frames[*a].time - now)
                    .abs()
                    .total_cmp(&(self.frames[*b].time - now).abs())
            })?;
        let now = self.frames[current].time;
        let frames = &self.frames[..=current];
        let speed = self.stencil.wave_speed;
        let margin = self.stencil.delay_margin;
        let mut amplitudes = Vec::with_capacity(DIRECTIONS);
        for direction in 0..DIRECTIONS {
            let angle = std::f64::consts::TAU * direction as f64 / DIRECTIONS as f64;
            let ray = Point2::new(angle.cos(), angle.sin());
            let mut amplitude = 0.0;
            for (index, (_, position, normal)) in self.stencil.samples.iter().enumerate() {
                let retarded = now - margin + ray.dot(*position) / speed;
                let newer = frames.partition_point(|frame| frame.time < retarded);
                if newer == 0 || newer == frames.len() {
                    return None;
                }
                let (newer, older) = (&frames[newer], &frames[newer - 1]);
                let fraction =
                    ((newer.time - retarded) / (newer.time - older.time)).clamp(0.0, 1.0);
                let sample = |k: usize| {
                    newer.values[index][k]
                        + (older.values[index][k] - newer.values[index][k]) * fraction
                };
                amplitude +=
                    self.stencil.sample_spacing * (sample(2) - normal.dot(ray) * sample(1) / speed);
            }
            amplitudes.push(amplitude);
        }
        Some((now - margin, amplitudes))
    }
}
