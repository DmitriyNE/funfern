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

/// The cavity made a TM box whose every side is a first-order outgoing wall,
/// with a hole off its centre and the document's point source beside it, a
/// 2 Hz sine under a flat top switched on over half a second, and its far
/// field on at the default inset.
pub(super) fn far_field_document() -> TopologyDocument {
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
            start: 0.0,
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
/// records, every one whose 96 directions all read, must stand a bucket
/// apart from the first the delay window lets through to the run's end; each
/// direction's amplitude within `TOLERANCE` of the twin's at the same frame,
/// relative to the largest the twin reads, f32 against f64 (the M1 Max reads
/// 8.1e-6); each record's far time within `STAMP` of the twin's (3.8e-7 s).
pub(super) fn judge(state: &Playground, twin: &FarFieldTwin) -> Result<String, String> {
    const TOLERANCE: f64 = 4.0e-5;
    const STAMP: f64 = 2.0e-6;
    let records = &state.far_field_trace.records;
    if records.len() < 60 {
        return Err(format!("the far field holds {} records", records.len()));
    }
    let (mut worst, mut largest, mut stamp) = (0.0_f64, 0.0_f64, 0.0_f64);
    let mut buckets = Vec::new();
    for record in records {
        let now = record.time + twin.margin();
        let (far, expected) = twin
            .projection(now)
            .ok_or(format!("the twin holds no projection at {now} s"))?;
        buckets.push(twin.bucket(far + twin.margin()));
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
    if let Some(pair) = buckets.windows(2).find(|pair| pair[1] - pair[0] != 1.0) {
        return Err(format!(
            "the far field's records skip from bucket {} to {}",
            pair[0], pair[1]
        ));
    }
    let worst = worst / largest;
    let summary = format!(
        "{} far-field records from {:.3} s, a bucket apart, amplitude {worst:.2e} of its largest, \
         stamps {stamp:.1e} s",
        records.len(),
        records[0].time
    );
    for (what, value, bound) in [
        ("an amplitude", worst, TOLERANCE),
        ("a stamp", stamp, STAMP),
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

/// One recorded contour frame: its time, and at each contour point the
/// field, its rate and its normal gradient.
struct Frame {
    time: f64,
    values: Vec<[f64; 3]>,
}

/// The recorder in f64, as `canonical_far_field.wgsl` takes it. At every step
/// it reads each contour point's field `u = Σ wᵢQᵢ/Mᵢ` through the static
/// mass and its normal gradient, and centres a frame on the step: their means
/// over its two ends and the field's difference across it, at half a step
/// back. It keeps the first frame a recorder pass takes in each bucket of
/// `period`, by the frame's own time, passes coming every `stride` steps. A
/// projection at a bucket's frame sums `Δs (∂ₙu − (n·ŝ) u̇/c)` over the
/// contour, each point at its retarded time `now − margin + ŝ·y/c`, `now` the
/// frame's time, interpolated between the frames about it.
pub(crate) struct FarFieldTwin {
    stencil: Arc<QuadraticFarFieldStencil>,
    inverse_mass: Vec<f64>,
    time_step: f64,
    period: f64,
    stride: u64,
    previous: Option<Vec<(f64, f64)>>,
    frames: Vec<Frame>,
}

impl FarFieldTwin {
    /// The twin of a run that starts at rest, as the recorder's first pass
    /// differences its first step against.
    pub(crate) fn new(active: &PreparedTopology, time_step: f64) -> Result<Self, String> {
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
        })
    }

    pub(crate) fn margin(&self) -> f64 {
        self.stencil.delay_margin
    }

    /// The bucket a frame's time falls in, as the shader takes it in f32.
    pub(crate) fn bucket(&self, time: f64) -> f32 {
        ((time as f32) / (self.period as f32)).floor()
    }

    /// The time a frame taken after the step ending at `time` stands at, as
    /// the shader takes it in f32: half a step back.
    fn centred(&self, time: f64) -> f64 {
        f64::from(time as f32 - 0.5 * self.time_step as f32)
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
        let previous = self.previous.replace(values.clone());
        let (Some(previous), true) = (previous, step.is_multiple_of(self.stride)) else {
            return;
        };
        let time = self.centred(time);
        let bucket = self.bucket(time);
        if self
            .frames
            .last()
            .is_some_and(|frame| self.bucket(frame.time) >= bucket)
        {
            return;
        }
        self.frames.push(Frame {
            time,
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
