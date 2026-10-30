//! PH-18's observables: a TM channel driven from its left side into a matched
//! wall on its right, carrying the plane wave `u = g(t − (x + 1)/c₀)`, `g` the
//! left side's drive, past every kind of probe. [`Twin`] reads the f64
//! reference through the recorders' CPU twins at every step; the fixture
//! holds what the app's traces recorded on the device to it, and the tests
//! below hold the twin to the wave.

use super::super::transfer::{TransferInput, TransferSource, TransferView};
use super::*;
use crate::document::{LineProbeQuantity, ProbeSamplingPreset, TransferReference};
use crate::topology_editor::{
    TopologyBoundaryProbeTarget, TopologyProbeDefinition, TopologyProbeTarget,
};
use crate::topology_runtime::{TopologyProbeCompilation, TopologyProbeStencil};
use crate::wave_gpu::PointProbeRecord;
use funfern_core::{
    CanonicalAreaSample, CanonicalPointSample, CanonicalPointStencil, CanonicalWaveOperator,
    ElectromagneticPolarization, PhysicsModel, PulseEnvelope, QuadraticAreaStencil,
    sample_canonical_area,
};

/// The left side's drive: a 2 Hz sine of amplitude 0.5 under a flat top 8 s
/// long, its raised-cosine edges half a second each, from rest at 0.
pub(super) const OBSERVED_DRIVE: TimeSignal = TimeSignal::Pulsed {
    offset: 0.0,
    amplitude: 0.5,
    frequency_hz: 2.0,
    phase_radians: 0.0,
    envelope: PulseEnvelope::FlatTop {
        duration: 8.0,
        edge: 0.5,
    },
    start: 0.0,
    repeat: 0.0,
};
const AMPLITUDE: f64 = 0.5;
const FREQUENCY: f64 = 2.0;

/// The fixture's run: 8 s at the step the app paces the channel at, 6.69e-3.
pub(super) const OBSERVED_STEPS: u64 = 1195;

/// Where the readouts' spectrum and transfer read the wave: from its front
/// passing the far point to the drive's falling edge.
const STEADY: (f64, f64) = (3.0, 7.5);
/// The transfer readout's segment there.
const TRANSFER_SEGMENT: f64 = 2.0;
/// The transfer's phase against the wave's delay, in radians: the M1 Max
/// reads 1.1e-3, a reference resampled one sample out of line 0.14.
const PHASE: f64 = 1.0e-2;

pub(super) const UPSTREAM: ProbeId = ProbeId(1);
pub(super) const DOWNSTREAM: ProbeId = ProbeId(2);
pub(super) const ACROSS: ProbeId = ProbeId(3);
pub(super) const AT_THE_WALL: ProbeId = ProbeId(4);
pub(super) const ROUND_THE_LOOP: ProbeId = ProbeId(5);
pub(super) const INSIDE_THE_LOOP: ProbeId = ProbeId(6);
pub(super) const DISK: ProbeId = ProbeId(7);
/// The probes by kind, in the order [`TwinRecord`] holds them.
pub(super) const POINTS: [ProbeId; 2] = [UPSTREAM, DOWNSTREAM];
pub(super) const LINES: [ProbeId; 3] = [ACROSS, AT_THE_WALL, ROUND_THE_LOOP];
pub(super) const AREAS: [ProbeId; 2] = [INSIDE_THE_LOOP, DISK];

const UPSTREAM_POINT: Point2 = Point2 { x: -0.45, y: 0.61 };
const DOWNSTREAM_POINT: Point2 = Point2 { x: 0.35, y: 0.62 };
const ACROSS_X: f64 = -0.2;
const LOOP_CENTER: Point2 = Point2 { x: 0.3, y: -0.35 };
const LOOP_RADIUS: f64 = 0.3;
const DISK_CENTER: Point2 = Point2 { x: -0.5, y: -0.45 };
const DISK_RADIUS: f64 = 0.25;

/// The cavity made a TM channel, `ε = μ = 1`: its left side driven by
/// [`OBSERVED_DRIVE`], its right a first-order outgoing wall, which a plane
/// wave meets at normal incidence and leaves through, its top and bottom
/// reflecting, which a wave along x does not see. A circle of the same
/// medium, its spans transmitting, makes a region to probe round and
/// inside. Two points on the way, a segment across the channel bottom to
/// top, its left normal against the wave, a segment down the wall, its left
/// normal out, a boundary probe on the circle's inside trace, whose normal
/// points out of the circle, an area probe of the circle's region and a
/// disk.
pub(super) fn observables_document() -> TopologyDocument {
    let mut editor = TopologyEditor::from_document(cavity_document()).expect("the cavity opens");
    let curve = editor
        .create_closed_curve(
            PeriodicCubicSpline::rounded(LOOP_CENTER, LOOP_RADIUS),
            ClosedCurvePurpose::Subdomain {
                material: DEFAULT_MATERIAL,
            },
        )
        .expect("the loop");
    let mut document = editor.document;
    let mut walls = OuterBoundaryConditions::uniform(OuterBoundaryCondition::Reflecting);
    walls.sides[OuterSide::Left.index()] = OuterBoundaryCondition::Dirichlet {
        signal: OBSERVED_DRIVE,
    };
    walls.sides[OuterSide::Right.index()] = OuterBoundaryCondition::FirstOrderOutgoing;
    document.model.draft.physics = PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    };
    document.model.draft.outer_boundaries = walls;
    let draft = &document.model.draft;
    let (region, side) = draft
        .face_assignments
        .iter()
        .find_map(|assignment| match assignment.anchor {
            funfern_core::FaceAnchor::Curve {
                curve: on, side, ..
            } if on == curve => Some((assignment.region?, side)),
            _ => None,
        })
        .expect("the loop's region");
    let spans = draft
        .geometry
        .curve(curve)
        .expect("the loop's curve")
        .spans
        .iter()
        .map(|span| span.id)
        .collect();
    document.model.accepted = document.model.draft.clone();
    let domain = document.model.accepted.geometry.domain;
    let probe = |id: ProbeId, name: &str, target| TopologyProbeDefinition {
        id,
        name: name.into(),
        color: [91, 220, 194],
        enabled: true,
        target,
    };
    let segment = |start, end| TopologyProbeTarget::Segment {
        start,
        end,
        preset: ProbeSamplingPreset::Medium,
    };
    document.model.probes = vec![
        probe(
            UPSTREAM,
            "Upstream",
            TopologyProbeTarget::Point(UPSTREAM_POINT),
        ),
        probe(
            DOWNSTREAM,
            "Downstream",
            TopologyProbeTarget::Point(DOWNSTREAM_POINT),
        ),
        probe(
            ACROSS,
            "Across",
            segment(
                Point2::new(ACROSS_X, domain.min_y),
                Point2::new(ACROSS_X, domain.max_y),
            ),
        ),
        probe(
            AT_THE_WALL,
            "At the wall",
            segment(
                Point2::new(domain.max_x, domain.max_y),
                Point2::new(domain.max_x, domain.min_y),
            ),
        ),
        probe(
            ROUND_THE_LOOP,
            "Round the loop",
            TopologyProbeTarget::Boundary(TopologyBoundaryProbeTarget {
                curve,
                spans,
                side,
                reversed: false,
                preset: ProbeSamplingPreset::Medium,
            }),
        ),
        probe(
            INSIDE_THE_LOOP,
            "Inside the loop",
            TopologyProbeTarget::AreaRegion(region),
        ),
        probe(
            DISK,
            "Disk",
            TopologyProbeTarget::AreaDisk {
                center: DISK_CENTER,
                radius: DISK_RADIUS,
            },
        ),
    ];
    document
}

/// What a line's twin reads at one of its points, a device record's four.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct LinePoint {
    field: f64,
    transverse: f64,
    energy: f64,
    flux: f64,
}

/// Every probe's twin at one step, the probes in [`POINTS`], [`LINES`] and
/// [`AREAS`] order.
pub(super) struct TwinRecord {
    time: f64,
    points: Vec<CanonicalPointSample>,
    lines: Vec<Vec<LinePoint>>,
    areas: Vec<CanonicalAreaSample>,
}

/// The recorders' CPU twins on the channel: each point through
/// `CanonicalPointStencil`, its rate the backward difference of the flux the
/// recorder takes; each line's points the same, with the normal its
/// documentation gives - a segment's left normal, a boundary sample's
/// outward one - and the length its readout integrates over; each area
/// through `sample_canonical_area`.
pub(super) struct Twin {
    operator: Arc<CanonicalWaveOperator>,
    time_step: f64,
    points: Vec<CanonicalPointStencil>,
    lines: Vec<(Vec<(CanonicalPointStencil, Point2)>, f64)>,
    areas: Vec<QuadraticAreaStencil>,
    previous: Vec<f64>,
    records: Vec<TwinRecord>,
}

impl Twin {
    pub(super) fn new(
        active: &PreparedTopology,
        probes: &[TopologyProbeDefinition],
        time_step: f64,
    ) -> Result<Self, String> {
        let operator = active.canonical_operator.clone();
        let stencil = |id: ProbeId| {
            let compiled = active
                .probes
                .iter()
                .find(|probe| probe.id == id)
                .ok_or(format!("probe {id:?} was not compiled"))?;
            match &compiled.result {
                TopologyProbeCompilation::Ready(stencil) => Ok(stencil.as_ref().clone()),
                other => Err(format!("probe {id:?} did not compile: {other:?}")),
            }
        };
        let canonical = |stencil| {
            CanonicalPointStencil::from_quadratic(stencil, &operator).map_err(|e| e.to_string())
        };
        let mut points = Vec::new();
        for id in POINTS {
            let TopologyProbeStencil::Point(stencil) = stencil(id)? else {
                return Err(format!("probe {id:?} is not a point"));
            };
            points.push(canonical(stencil)?);
        }
        let mut lines = Vec::new();
        for id in LINES {
            let line = match (stencil(id)?, probes.iter().find(|probe| probe.id == id)) {
                (
                    TopologyProbeStencil::Segment(stencils),
                    Some(TopologyProbeDefinition {
                        target: TopologyProbeTarget::Segment { start, end, .. },
                        ..
                    }),
                ) => {
                    let along = *end - *start;
                    let normal = Point2::new(-along.y, along.x) * (1.0 / along.norm());
                    let samples = stencils
                        .into_iter()
                        .map(|stencil| canonical(stencil).map(|stencil| (stencil, normal)))
                        .collect::<Result<Vec<_>, _>>()?;
                    (samples, along.norm())
                }
                (TopologyProbeStencil::Boundary(stencils), _) => {
                    let length = stencils
                        .windows(2)
                        .map(|pair| (pair[1].point - pair[0].point).norm())
                        .sum::<f64>();
                    let samples = stencils
                        .into_iter()
                        .map(|sample| {
                            canonical(sample.stencil)
                                .map(|stencil| (stencil, sample.outward_normal))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    (samples, length)
                }
                _ => return Err(format!("probe {id:?} is not a line")),
            };
            lines.push(line);
        }
        let mut areas = Vec::new();
        for id in AREAS {
            let TopologyProbeStencil::Area(stencil) = stencil(id)? else {
                return Err(format!("probe {id:?} is not an area"));
            };
            areas.push(stencil);
        }
        let previous = vec![0.0; operator.degrees_of_freedom()];
        let mut twin = Self {
            operator,
            time_step,
            points,
            lines,
            areas,
            previous,
            records: Vec::new(),
        };
        let zero = twin.previous.clone();
        let rest = vec![Point2::default(); twin.operator.complementary_degrees_of_freedom()];
        twin.observe(0.0, &zero, &rest)?;
        Ok(twin)
    }

    /// Reads the state after the next step, at `time`.
    pub(super) fn observe(
        &mut self,
        time: f64,
        primary: &[f64],
        complementary: &[Point2],
    ) -> Result<(), String> {
        let rate = primary
            .iter()
            .zip(&self.previous)
            .map(|(now, before)| (now - before) / self.time_step)
            .collect::<Vec<_>>();
        let sample = |stencil: &CanonicalPointStencil| {
            stencil
                .sample(primary, &rate, complementary)
                .map_err(|error| error.to_string())
        };
        let points = self
            .points
            .iter()
            .map(sample)
            .collect::<Result<Vec<_>, _>>()?;
        let lines = self
            .lines
            .iter()
            .map(|(samples, _)| {
                samples
                    .iter()
                    .map(|(stencil, normal)| {
                        sample(stencil).map(|read| LinePoint {
                            field: read.primary,
                            transverse: read.complementary.norm(),
                            energy: read.energy_density,
                            flux: read.energy_flow.dot(*normal),
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let areas = self
            .areas
            .iter()
            .map(|stencil| {
                sample_canonical_area(stencil, &self.operator, primary, complementary)
                    .map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.records.push(TwinRecord {
            time,
            points,
            lines,
            areas,
        });
        self.previous = primary.to_vec();
        Ok(())
    }

    /// The readout's trapezoid over one of the line's frames, as
    /// `Playground::curve_probe_integral` takes it.
    fn integral(&self, line: usize, values: impl Iterator<Item = f64>) -> f64 {
        let values = values.collect::<Vec<_>>();
        let intervals = values.len() - 1;
        (values.iter().sum::<f64>() - 0.5 * (values[0] + values[intervals])) * self.lines[line].1
            / intervals as f64
    }
}

/// The largest of `worst` and `value`, a NaN carried through.
fn grow(worst: &mut f64, value: f64) {
    *worst = worse(*worst, value);
}

/// What the device recorded, held to the twin at the steps it recorded. The
/// record's step is its time over the step, since the recorder stamps the
/// solver's clock. Each kind's records must be a stride apart from the run's
/// start to its end, none missing; a line's may instead be the latest
/// `CURVE_TRACE_FRAMES`, all the app keeps, which at the channel's step and
/// 60 Hz is a stride of 2 and the last 6.85 s of the 8 s run. A value within
/// `TOLERANCE` of its scale of the twin's, f32 against f64 (the M1 Max reads
/// 2.8e-5 at worst, a point's); the stamp within `STAMP` of the twin's time,
/// two of f32's spacings at the run's end (3.0e-7). Poynting's theorem on the
/// loop, what left round it between two of its records against what the
/// region inside lost, which a trapezoid over the records' stride holds
/// within 1e-2 of the flow's scale (7.5e-3), within 1e-4 of the twin's at the
/// same records (7.9e-6). Then the
/// readouts' estimators on the device's records: the upstream point's
/// spectrum peaks at the drive's amplitude, and the transfer between the
/// points, a pure delay, reads one. The rate, a backward difference, is
/// read against `u̇` half a step before its stamp, and at the stamp.
pub(super) fn judge(state: &Playground, twin: &Twin, endpoint: u64) -> Result<String, String> {
    const TOLERANCE: f64 = 1.0e-4;
    const STAMP: f64 = 2.0e-6;
    let dt = twin.time_step;
    let step_of = |time: f64| -> Result<usize, String> {
        let step = (time / dt).round();
        if step < 0.0 || step as usize >= twin.records.len() {
            return Err(format!("a record at {time} s, past the run"));
        }
        Ok(step as usize)
    };
    // Every recorded step from the first stride to the endpoint, none twice,
    // or the latest `kept` of them where the trace keeps no more.
    let covered = |name: &str, steps: &[usize], kept: Option<usize>| -> Result<usize, String> {
        let stride = steps
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .min()
            .unwrap_or(0);
        if steps.len() < 2
            || stride == 0
            || steps.windows(2).any(|pair| pair[1] - pair[0] != stride)
            || (steps[0] > stride && Some(steps.len()) != kept)
            || steps[steps.len() - 1] + stride <= endpoint as usize
        {
            return Err(format!(
                "{name}'s records do not cover the run a stride apart: {} records, steps {:?}..{:?}",
                steps.len(),
                steps.first(),
                steps.last()
            ));
        }
        Ok(stride)
    };
    let (scale_u, scale_rate) = (AMPLITUDE, std::f64::consts::TAU * FREQUENCY * AMPLITUDE);
    let mut summary = Vec::new();
    let mut worst_stamp = 0.0_f64;

    let mut worst_point = 0.0_f64;
    let (mut rate_mid, mut rate_end) = (0.0_f64, 0.0_f64);
    let origin = state.editor.document.model.accepted.geometry.domain.min_x;
    for (index, id) in POINTS.iter().enumerate() {
        let x = [UPSTREAM_POINT.x, DOWNSTREAM_POINT.x][index];
        let derivative = |t: f64| OBSERVED_DRIVE.derivative(t - (x - origin));
        let trace = state
            .probe_traces
            .get(id)
            .ok_or(format!("no trace for probe {id:?}"))?;
        let steps = trace
            .samples
            .iter()
            .map(|record| step_of(record.time))
            .collect::<Result<Vec<_>, _>>()?;
        let stride = covered(&format!("point {id:?}"), &steps, None)?;
        for (record, step) in trace.samples.iter().zip(&steps) {
            let expected = &twin.records[*step];
            let read = expected.points[index];
            grow(&mut worst_stamp, (record.time - expected.time).abs());
            grow(
                &mut rate_mid,
                (record.velocity - derivative(expected.time - 0.5 * dt)).abs() / scale_rate,
            );
            grow(
                &mut rate_end,
                (record.velocity - derivative(expected.time)).abs() / scale_rate,
            );
            for error in [
                (record.displacement - read.primary).abs() / scale_u,
                (record.velocity - read.primary_rate).abs() / scale_rate,
                (record.transverse_magnitude - read.complementary.norm()).abs() / scale_u,
                (record.poynting_magnitude - read.energy_flow.norm()).abs() / (scale_u * scale_u),
                (record.energy_density - read.energy_density).abs() / (scale_u * scale_u),
            ] {
                grow(&mut worst_point, error);
            }
        }
        if index == 0 {
            summary.push(format!("points every {stride} steps"));
        }
    }
    summary.push(format!(
        "points {worst_point:.2e}, their rate against u̇ half a step before its stamp \
         {rate_mid:.2e}, at its stamp {rate_end:.2e}"
    ));

    let mut worst_line = 0.0_f64;
    let mut worst_integral = 0.0_f64;
    let mut worst_length = 0.0_f64;
    let mut loop_flux = Vec::new();
    for (index, id) in LINES.iter().enumerate() {
        let trace = state
            .curve_probe_traces
            .get(id)
            .ok_or(format!("no trace for probe {id:?}"))?;
        let steps = trace
            .records
            .iter()
            .map(|record| step_of(record.time))
            .collect::<Result<Vec<_>, _>>()?;
        covered(
            &format!("line {id:?}"),
            &steps,
            Some(super::super::CURVE_TRACE_FRAMES),
        )?;
        for (record, step) in trace.records.iter().zip(&steps) {
            let expected = &twin.records[*step];
            grow(&mut worst_stamp, (record.time - expected.time).abs());
            let line = &expected.lines[index];
            if record.displacement.len() != line.len() {
                return Err(format!(
                    "line {id:?} records {} points where it has {}",
                    record.displacement.len(),
                    line.len()
                ));
            }
            for (k, point) in line.iter().enumerate() {
                for error in [
                    (f64::from(record.displacement[k]) - point.field).abs() / scale_u,
                    (f64::from(record.transverse_magnitude[k]) - point.transverse).abs() / scale_u,
                    (f64::from(record.energy_density[k]) - point.energy).abs()
                        / (scale_u * scale_u),
                    (f64::from(record.normal_flux[k]) - point.flux).abs() / (scale_u * scale_u),
                ] {
                    grow(&mut worst_line, error);
                }
            }
            // Over the length the readout integrates with, held to the
            // twin's: the twin's own length let a readout that integrated
            // over a wrong one pass (VER-42 of the 2026-10-20 review).
            let length = Playground::curve_frame_length(
                record,
                state
                    .probe_metrics
                    .get(id)
                    .map_or(0.0, |(length, _)| *length),
            );
            grow(
                &mut worst_length,
                (length - twin.lines[index].1).abs() / twin.lines[index].1,
            );
            let (read, coverage) =
                Playground::curve_probe_integral(record, length, LineProbeQuantity::Flux);
            let exact = twin.integral(index, line.iter().map(|point| point.flux));
            grow(
                &mut worst_integral,
                (read - exact).abs() / (scale_u * scale_u),
            );
            if coverage != 1.0 {
                return Err(format!("line {id:?} covers {coverage} of itself"));
            }
            if *id == ROUND_THE_LOOP {
                loop_flux.push((*step, read));
            }
        }
    }
    summary.push(format!(
        "lines {worst_line:.2e}, their integrals {worst_integral:.2e} over lengths \
         {worst_length:.1e} off"
    ));

    let mut worst_area = 0.0_f64;
    let mut inside = Vec::new();
    for (index, id) in AREAS.iter().enumerate() {
        let trace = state
            .area_probe_traces
            .get(id)
            .ok_or(format!("no trace for probe {id:?}"))?;
        let steps = trace
            .records
            .iter()
            .map(|record| step_of(record.time))
            .collect::<Result<Vec<_>, _>>()?;
        covered(&format!("area {id:?}"), &steps, None)?;
        for (record, step) in trace.records.iter().zip(&steps) {
            let expected = &twin.records[*step];
            grow(&mut worst_stamp, (record.time - expected.time).abs());
            let read = expected.areas[index];
            let energy = (scale_u * scale_u) * read.covered_area;
            for error in [
                (record.mean_displacement - read.mean_primary).abs() / scale_u,
                (record.rms_displacement - read.rms_primary).abs() / scale_u,
                (record.rms_transverse_magnitude - read.rms_complementary).abs() / scale_u,
                (record.mean_energy_density - read.mean_energy_density).abs() / (scale_u * scale_u),
                (record.total_energy - read.total_energy).abs() / energy,
                (record.covered_area - read.covered_area).abs() / read.covered_area,
                (record.coverage - read.coverage).abs(),
            ] {
                grow(&mut worst_area, error);
            }
            if *id == INSIDE_THE_LOOP {
                inside.push((*step, record.total_energy));
            }
        }
    }
    summary.push(format!("areas {worst_area:.2e}"));

    // Poynting on the device: what leaves round the loop between two of its
    // records, by the trapezoid, against what the region inside lost.
    let (mut worst_poynting, mut poynting_apart) = (0.0_f64, 0.0_f64);
    let round = LINES
        .iter()
        .position(|id| *id == ROUND_THE_LOOP)
        .unwrap_or(0);
    let twin_flux = |step: usize| {
        twin.integral(
            round,
            twin.records[step].lines[round]
                .iter()
                .map(|point| point.flux),
        )
    };
    let kept_from = loop_flux.first().map_or(0, |(step, _)| *step);
    for pair in inside.windows(2).filter(|pair| pair[0].0 >= kept_from) {
        let [(from, before), (to, after)] = [pair[0], pair[1]];
        let flux = |step| {
            loop_flux
                .iter()
                .find(|(at, _)| *at == step)
                .map(|(_, f)| *f)
        };
        let (Some(left_then), Some(left_now)) = (flux(from), flux(to)) else {
            return Err(format!("the loop has no record at steps {from} and {to}"));
        };
        let span = (to - from) as f64 * dt;
        let residual = (after - before) / span + 0.5 * (left_then + left_now);
        let energy = |step: usize| twin.records[step].areas[0].total_energy;
        let expected = (energy(to) - energy(from)) / span + 0.5 * (twin_flux(from) + twin_flux(to));
        grow(&mut worst_poynting, residual.abs() / (scale_u * scale_u));
        grow(
            &mut poynting_apart,
            (residual - expected).abs() / (scale_u * scale_u),
        );
    }
    summary.push(format!(
        "Poynting round the loop {worst_poynting:.2e}, {poynting_apart:.2e} from the twin's"
    ));
    summary.push(format!("stamps {worst_stamp:.1e} s"));

    // The readouts' estimators on what the device recorded.
    let upstream = &state.probe_traces[&UPSTREAM].samples;
    let downstream = &state.probe_traces[&DOWNSTREAM].samples;
    let records = upstream.iter().copied().collect::<Vec<PointProbeRecord>>();
    let spectrum = super::super::probe_view::trace_spectrum(
        &records,
        |record| record.displacement,
        STEADY,
        0.0,
        false,
    );
    let [frequency, peak] =
        spectrum.iter().copied().fold(
            [0.0, 0.0],
            |best, line| if line[1] > best[1] { line } else { best },
        );
    summary.push(format!("spectrum {peak:.5} at {frequency:.3} Hz"));
    // The readout's own transfer, taken as the readout takes it, on the
    // downstream probe against the upstream one over two-second segments
    // across the steady span, and held in phase as well as in gain: one tone
    // through a pure delay reads |H| = 1 however the two records are aligned,
    // where its phase moves 0.1 rad a sample (VER-43 of the 2026-10-20
    // review). The wave travels the points' x apart at c₀ = 1.
    let (up, down) = (
        upstream.iter().copied().collect::<Vec<_>>(),
        downstream.iter().copied().collect::<Vec<_>>(),
    );
    let input = TransferInput {
        reference: TransferReference::Probe(UPSTREAM),
        label: String::new(),
        data: None,
        sources: None,
    };
    let mut view = TransferView::default();
    let mut end = STEADY.0 + TRANSFER_SEGMENT;
    while end <= STEADY.1 {
        view.update(
            &down,
            &input,
            TransferSource::Probe(&up),
            end,
            TRANSFER_SEGMENT,
            true,
        );
        end += TRANSFER_SEGMENT / 4.0;
    }
    let transfer = view.transfer().ok_or("no transfer window")?;
    let bin = (FREQUENCY / transfer.frequency_step_hz).round() as usize;
    let [re, im] = transfer
        .ratios
        .get(bin)
        .copied()
        .flatten()
        .ok_or("no transfer at the drive's frequency")?;
    let gain = re.hypot(im);
    let delay = -std::f64::consts::TAU * FREQUENCY * (DOWNSTREAM_POINT.x - UPSTREAM_POINT.x);
    let phase = (im.atan2(re) - delay + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
        - std::f64::consts::PI;
    summary.push(format!(
        "transfer |H| {gain:.5}, its phase {phase:+.1e} rad from the delay's"
    ));

    let summary = summary.join(", ");
    for (what, value, bound) in [
        ("a point", worst_point, TOLERANCE),
        ("a line", worst_line, TOLERANCE),
        ("a line's integral", worst_integral, TOLERANCE),
        ("a line's length", worst_length, 1.0e-6),
        ("an area", worst_area, TOLERANCE),
        ("a stamp", worst_stamp, STAMP),
        ("Poynting round the loop", worst_poynting, 1.0e-2),
        ("Poynting against the twin's", poynting_apart, TOLERANCE),
        ("the rate half a step before its stamp", rate_mid, 1.0e-2),
        (
            "the spectrum's peak",
            (peak / AMPLITUDE - 1.0).abs(),
            1.0e-2,
        ),
        ("the transfer", (gain - 1.0).abs(), 2.0e-3),
        ("the transfer's phase", phase.abs(), PHASE),
    ] {
        if !within(value, bound) {
            return Err(format!(
                "{summary}: {what} reads {value:.3e}, beyond {bound:.0e}"
            ));
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topology_runtime::TopologyRuntime;
    use funfern_core::{MeshingOptions, amplitude_spectrum};

    fn prepare(document: &TopologyDocument) -> Arc<PreparedTopology> {
        let mut runtime = TopologyRuntime::default();
        let editor = TopologyEditor::from_document(document.clone()).unwrap();
        let domain = document.model.accepted.geometry.domain;
        let token = runtime
            .request(
                1,
                &editor.document,
                editor.compiled_accepted.clone(),
                MeshingOptions {
                    target_edge_length: document.presentation.mesh_edge,
                    ..MeshingOptions::default()
                }
                .sized_for_area(domain.width() * domain.height()),
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

    /// PH-18: the recorders' twins on the fixture's channel, at its mesh and
    /// step, against the plane wave `u = g(t − (x + 1))` over the 8 s run:
    /// - the points' field within 2.5e-3 (6.9e-4, of an amplitude of 0.5),
    ///   their rate against `u̇` half a step before the sample within 4e-3
    ///   of `2πf` (2.3e-3), the transverse field `|c| = |u|` within 1.2e-2
    ///   (9.0e-3), the density `u²` within 6e-3 (4.6e-3) and the flow
    ///   `(u², 0)` within 6e-3 (4.5e-3);
    /// - the segment across, by the readout's trapezoid, `−2u²` under its
    ///   left normal within 1.5e-3 (8.4e-4);
    /// - round the loop, Poynting's theorem against the area inside within
    ///   4e-3 (2.1e-3);
    /// - the energy the wall's ledger books within 5e-4 of `2∫g²` (2.1e-4);
    /// - the segment down the wall carries 1.5% more than that, which is the
    ///   transverse field on the wall's elements: it reads 1.5% high at the
    ///   wall and 0.7% low half an element in, where the field reads exact,
    ///   falling four times as the edge halves. Held within 3%, and listed as
    ///   worth checking;
    /// - the transverse field at a point is the quadratic through its
    ///   element's six samples, and reads 0.4% to 1.8% of the amplitude off
    ///   by where the point falls in the element: the upstream point, 0.004
    ///   from an edge in barycentric terms, reads 9.0e-3, that element's
    ///   centroid about 5e-3, and both points read within 1.9e-3 on the mesh
    ///   the chain cut left before 2026-10-30. The density and the flow take
    ///   it in, so the three are held wide of the rest; listed as worth
    ///   checking;
    /// - the readout's spectrum of the upstream point peaks within 1e-2 of
    ///   the drive's amplitude (5.7e-4), and the transfer to the downstream
    ///   one, a pure delay, reads one within 2e-3 (4.6e-4).
    #[test]
    fn the_twins_meet_the_wave_on_the_observables_channel() {
        let document = observables_document();
        let prepared = prepare(&document);
        assert!(prepared.canonical_temporal_operator.is_none());
        let operator = prepared.canonical_operator.clone();
        let forcing = prepared.canonical_forcing.clone();
        let dt = prepared.recommended_time_step();
        let domain = document.model.accepted.geometry.domain;
        let mut twin = Twin::new(&prepared, &document.model.probes, dt).unwrap();
        let mut state = CanonicalWaveState::zero(&operator, dt).unwrap();
        let mut booked = 0.0;
        for _ in 0..OBSERVED_STEPS {
            booked += state
                .step_with_forcing(&operator, &forcing)
                .unwrap()
                .boundary_loss;
            twin.observe(
                state.time(),
                state.primary_flux(),
                state.complementary_flux(),
            )
            .unwrap();
        }
        let wave = |x: f64, t: f64| OBSERVED_DRIVE.value(t - (x - domain.min_x));
        let rate = |x: f64, t: f64| OBSERVED_DRIVE.derivative(t - (x - domain.min_x));
        let omega = std::f64::consts::TAU * FREQUENCY;
        let height = domain.max_y - domain.min_y;
        let flux = |line: usize, record: &TwinRecord| {
            twin.integral(line, record.lines[line].iter().map(|point| point.flux))
        };
        let mut worst = [0.0_f64; 7];
        let mut left = 0.0;
        for pair in twin.records.windows(2) {
            let (before, record) = (&pair[0], &pair[1]);
            let t = record.time;
            for (read, x) in record
                .points
                .iter()
                .zip([UPSTREAM_POINT.x, DOWNSTREAM_POINT.x])
            {
                let u = wave(x, t);
                for (slot, error) in [
                    (read.primary - u).abs(),
                    (read.primary_rate - rate(x, t - 0.5 * dt)).abs() / omega,
                    (read.complementary.norm() - u.abs()).abs(),
                    (read.energy_density - u * u).abs(),
                    (read.energy_flow - Point2::new(u * u, 0.0)).norm(),
                ]
                .into_iter()
                .enumerate()
                {
                    grow(&mut worst[slot], error);
                }
            }
            let crossing = wave(ACROSS_X, t);
            grow(
                &mut worst[5],
                (flux(0, record) + height * crossing * crossing).abs(),
            );
            left += flux(1, record) * dt;
            // Over the step that ends here: the loss inside against the
            // trapezoid of what left round the loop.
            let change = (record.areas[0].total_energy - before.areas[0].total_energy) / dt;
            grow(
                &mut worst[6],
                (change + 0.5 * (flux(2, record) + flux(2, before))).abs(),
            );
        }
        let samples = 80_000;
        let span = OBSERVED_STEPS as f64 * dt;
        let exact = (0..=samples)
            .map(|i| {
                let u = wave(domain.max_x, span * f64::from(i) / f64::from(samples));
                height * u * u * span / f64::from(samples)
            })
            .sum::<f64>();
        let bounds = [2.5e-3, 4.0e-3, 1.2e-2, 6.0e-3, 6.0e-3, 1.5e-3, 4.0e-3];
        for (slot, (read, bound)) in worst.iter().zip(bounds).enumerate() {
            assert!(*read <= bound, "reading {slot} at {read:.2e}: {worst:?}");
        }
        assert!(
            (booked / exact - 1.0).abs() < 5.0e-4,
            "the wall booked {booked} of {exact}"
        );
        assert!(
            (left / booked - 1.0).abs() < 3.0e-2,
            "the segment down the wall carried {left} of {booked}"
        );
        let steady = twin
            .records
            .iter()
            .filter(|record| record.time >= STEADY.0 && record.time <= STEADY.1)
            .collect::<Vec<_>>();
        let up = steady
            .iter()
            .map(|record| record.points[0].primary)
            .collect::<Vec<_>>();
        let down = steady
            .iter()
            .map(|record| record.points[1].primary)
            .collect::<Vec<_>>();
        let spectrum = amplitude_spectrum(&up, dt).unwrap();
        let peak = spectrum.magnitudes.iter().copied().fold(0.0, f64::max);
        assert!(
            (peak / AMPLITUDE - 1.0).abs() < 1.0e-2,
            "spectrum peak {peak}"
        );
        let window = (2.0 / dt).round() as usize;
        let mut average = TransferAverage::default();
        let mut start = 0;
        while start + window <= up.len() {
            average
                .add(&up[start..start + window], &down[start..start + window], dt)
                .unwrap();
            start += window / 4;
        }
        let transfer = average.transfer().unwrap();
        let gain = transfer
            .magnitude((FREQUENCY / transfer.frequency_step_hz).round() as usize)
            .unwrap();
        assert!((gain - 1.0).abs() < 2.0e-3, "transfer |H| {gain}");
    }
}
