//! A point probe's transfer readout: its field's amplitude spectrum over a
//! reference's, both over the span it shows and on its own sample times,
//! where the reference is another point probe or what a source imposes as
//! the solver runs it.

use super::line_plot::{fitted_band, zoomable_line_plot};
use super::probe_view::{DECIBEL_RANGE, ProbeViewState};
use super::signals::SignalUse;
use super::*;
use crate::wave_gpu::PointProbeRecord;
#[cfg(test)]
use funfern_app::document::DEFAULT_TRANSFER_SEGMENT;
use funfern_app::document::TransferReference;
use funfern_app::topology_editor::TopologyProbeTarget;

/// What a transfer divides by: a probe's record, or a source's signal and
/// what it drives.
#[derive(Clone, Copy, Debug)]
pub(super) enum TransferSource<'a> {
    Probe(&'a [PointProbeRecord]),
    Signal(TimeSignal, SignalUse),
}

/// Every signal `scene` imposes, by the reference a transfer names it with:
/// the point source and each volume source that is on, and each side and
/// curve-span side that prescribes a signal.
pub(super) fn signal_sources(
    scene: &TopologyScene,
    point: PointSource,
) -> Vec<(TransferReference, TimeSignal, SignalUse)> {
    let mut sources = Vec::new();
    if point.enabled {
        sources.push((
            TransferReference::PointSource,
            point.signal,
            SignalUse::Source,
        ));
    }
    for source in &scene.volume_sources {
        if source.enabled {
            sources.push((
                TransferReference::VolumeSource(source.region),
                source.signal,
                SignalUse::Source,
            ));
        }
    }
    for side in OuterSide::ALL {
        match scene.outer_boundaries.sides[side.index()] {
            OuterBoundaryCondition::Neumann { signal } => {
                sources.push((TransferReference::Wall(side), signal, SignalUse::Flux));
            }
            OuterBoundaryCondition::Dirichlet { signal } => {
                sources.push((TransferReference::Wall(side), signal, SignalUse::Field));
            }
            _ => {}
        }
    }
    for curve in &scene.geometry.curves {
        for span in &curve.spans {
            let SpanBehavior::Separated { left, right, .. } = span.behavior else {
                continue;
            };
            for (side, condition) in [(CurveTraceSide::Left, left), (CurveTraceSide::Right, right)]
            {
                let reference = TransferReference::Face {
                    span: span.id,
                    side,
                };
                match condition {
                    FaceBoundaryCondition::Neumann { signal } => {
                        sources.push((reference, signal, SignalUse::Flux));
                    }
                    FaceBoundaryCondition::Dirichlet { signal } => {
                        sources.push((reference, signal, SignalUse::Field));
                    }
                    _ => {}
                }
            }
        }
    }
    sources
}

/// Every source the running scene has: an average resets on any change to
/// one, since what the records hold from then on answers other sources.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct SourceSet {
    point: PointSource,
    volume: Vec<VolumeSource>,
    signals: Vec<(TransferReference, TimeSignal, SignalUse)>,
}

impl SourceSet {
    pub(super) fn of(scene: &TopologyScene, point: PointSource) -> Self {
        Self {
            point,
            volume: scene.volume_sources.clone(),
            signals: signal_sources(scene, point),
        }
    }
}

/// `count` samples every `interval` seconds of `response`'s field and of
/// the reference, both ending at `end`: a probe's record resampled on that
/// timing, or a source's signal evaluated on it. `None` until both records
/// cover it.
pub(super) fn transfer_window(
    response: &[PointProbeRecord],
    reference: TransferSource<'_>,
    end: f64,
    interval: f64,
    count: usize,
) -> Option<(Vec<f64>, Vec<f64>)> {
    let record = |records: &[PointProbeRecord]| -> (Vec<f64>, Vec<f64>) {
        records
            .iter()
            .map(|record| (record.time, record.displacement))
            .filter(|(_, value)| value.is_finite())
            .unzip()
    };
    let start = end - (count - 1) as f64 * interval;
    let (times, values) = record(response);
    let response = resample_from(&times, &values, start, interval, count).ok()?;
    let reference = match reference {
        TransferSource::Probe(records) => {
            let (times, values) = record(records);
            resample_from(&times, &values, start, interval, count).ok()?
        }
        TransferSource::Signal(signal, drives) => (0..count)
            .map(|index| drives.imposed(signal, start + index as f64 * interval))
            .collect::<Option<Vec<_>>>()?,
    };
    Some((reference, response))
}

/// A readout's running transfer average and what it was taken on.
#[derive(Clone, Debug, Default)]
pub(super) struct TransferView {
    average: TransferAverage,
    basis: Option<TransferBasis>,
    /// The readout's Reset asked for a fresh average.
    pub(super) reset: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct TransferBasis {
    reference: TransferReference,
    segment: f64,
    sources: Option<SourceSet>,
    interval: f64,
    count: usize,
    last_end: f64,
}

impl TransferView {
    /// How many windows the average holds.
    pub(super) fn windows(&self) -> usize {
        self.average.windows()
    }

    /// Takes the `segment` seconds of `response` ending at `end` against
    /// `input` into the average: Welch's estimate, over segments as long as
    /// the readout asks whatever span its plots show. A live view adds each
    /// segment that ends later than the last; a parked one only its first.
    /// The average starts over at the Reset, for another reference or
    /// segment length, when a source of the running scene changes, and when a
    /// live view's time goes back, as it does at a reset of the run. The first
    /// segment fixes the spacing, at the response's median, and the length
    /// every later one is resampled on.
    pub(super) fn update(
        &mut self,
        response: &[PointProbeRecord],
        input: &TransferInput,
        source: TransferSource<'_>,
        end: f64,
        segment: f64,
        live: bool,
    ) {
        let from = end - segment;
        let holds = self.basis.as_ref().is_some_and(|basis| {
            basis.reference == input.reference
                && basis.segment == segment
                && basis.sources == input.sources
                && !(live && end < basis.last_end)
        });
        if self.reset || !holds {
            *self = Self::default();
        }
        if self.basis.is_none() {
            let times = response
                .iter()
                .filter(|record| record.time >= from && record.time <= end)
                .filter(|record| record.displacement.is_finite())
                .map(|record| record.time)
                .collect::<Vec<_>>();
            if times.len() < 8 {
                return;
            }
            let mut spacings = times
                .windows(2)
                .map(|pair| pair[1] - pair[0])
                .collect::<Vec<_>>();
            spacings.sort_by(f64::total_cmp);
            let interval = spacings[spacings.len() / 2];
            self.basis = Some(TransferBasis {
                reference: input.reference,
                segment,
                sources: input.sources.clone(),
                interval,
                count: ((end - from) / interval).floor() as usize + 1,
                last_end: f64::NEG_INFINITY,
            });
        }
        let Some(basis) = self.basis.as_mut() else {
            return;
        };
        if (live && end > basis.last_end) || self.average.windows() == 0 {
            if let Some((reference, response)) =
                transfer_window(response, source, end, basis.interval, basis.count)
                && self
                    .average
                    .add(&reference, &response, basis.interval)
                    .is_ok()
            {
                basis.last_end = end;
            }
        }
    }

    /// The average as plot points up to `top_hz` or half the sample rate,
    /// whichever is lower, and in decibels re one if asked: a gap where the
    /// reference holds too little to divide by.
    pub(super) fn points(&self, top_hz: f64, decibels: bool) -> Vec<[f64; 2]> {
        let (Some(transfer), Some(basis)) = (self.average.transfer(), self.basis.as_ref()) else {
            return Vec::new();
        };
        let nyquist = 0.5 / basis.interval;
        let top = if top_hz > 0.0 {
            top_hz.min(nyquist)
        } else {
            nyquist
        };
        let gains = (0..transfer.ratios.len())
            .map(|index| (transfer.frequency_hz(index), transfer.magnitude(index)))
            .take_while(|(frequency, _)| *frequency <= top)
            .collect::<Vec<_>>();
        // Floored in decibels as a spectrum is, under the largest gain shown.
        let largest = gains
            .iter()
            .filter_map(|(_, gain)| *gain)
            .fold(0.0, f64::max);
        let floor = (largest * 10.0_f64.powf(-DECIBEL_RANGE / 20.0)).max(f64::MIN_POSITIVE);
        gains
            .into_iter()
            .map(|(frequency, gain)| {
                let shown = gain.map_or(f64::NAN, |gain| {
                    if decibels {
                        20.0 * gain.max(floor).log10()
                    } else {
                        gain
                    }
                });
                [frequency, shown]
            })
            .collect()
    }
}

/// Where a transfer's plot starts, as a spectrum's does: at zero, or in
/// decibels `DECIBEL_RANGE` under the largest gain shown.
pub(super) fn transfer_plot_floor(points: &[[f64; 2]], decibels: bool) -> Option<f64> {
    if !decibels {
        return Some(0.0);
    }
    points
        .iter()
        .map(|[_, value]| *value)
        .filter(|value| value.is_finite())
        .reduce(f64::max)
        .map(|largest| largest - DECIBEL_RANGE)
}

impl Playground {
    /// What a transfer reference is called in the readout.
    pub(super) fn transfer_reference_label(&self, reference: TransferReference) -> String {
        match reference {
            TransferReference::Probe(id) => self
                .editor
                .document
                .model
                .probes
                .iter()
                .find(|probe| probe.id == id)
                .map_or_else(|| "a deleted probe".to_owned(), |probe| probe.name.clone()),
            TransferReference::PointSource => "Point source".to_owned(),
            TransferReference::VolumeSource(region) => {
                format!("{} source, region {}", self.material_name(region), region.0)
            }
            TransferReference::Wall(side) => format!("{} wall", side.label()),
            TransferReference::Face { span, side } => format!(
                "Span {} · {} side",
                span.0,
                match side {
                    CurveTraceSide::Left => "left",
                    CurveTraceSide::Right => "right",
                }
            ),
        }
    }

    /// What probe `id`'s transfer can divide by, as its readout lists it:
    /// the scene's other point probes, then every signal the accepted scene
    /// imposes.
    pub(super) fn transfer_candidates(&self, id: ProbeId) -> Vec<(TransferReference, String)> {
        let model = &self.editor.document.model;
        let probes = model
            .probes
            .iter()
            .filter(|probe| probe.id != id && matches!(probe.target, TopologyProbeTarget::Point(_)))
            .map(|probe| TransferReference::Probe(probe.id));
        let sources = signal_sources(&model.accepted, model.source)
            .into_iter()
            .map(|(reference, ..)| reference);
        probes
            .chain(sources)
            .map(|reference| (reference, self.transfer_reference_label(reference)))
            .collect()
    }

    /// What a readout needs to draw a transfer from `reference`: the
    /// reference probe's record, or the source's signal in the running scene.
    pub(super) fn transfer_input(&self, reference: TransferReference) -> TransferInput {
        let data = match reference {
            TransferReference::Probe(id) => self
                .probe_traces
                .get(&id)
                .map(|trace| TransferData::Probe(trace.samples.iter().copied().collect())),
            _ => self.runtime.active().and_then(|active| {
                signal_sources(&active.bundle.authored, active.point_source)
                    .into_iter()
                    .find(|(held, ..)| *held == reference)
                    .map(|(_, signal, drives)| TransferData::Signal(signal, drives))
            }),
        };
        TransferInput {
            reference,
            label: self.transfer_reference_label(reference),
            data,
            sources: self
                .runtime
                .active()
                .map(|active| SourceSet::of(&active.bundle.authored, active.point_source)),
        }
    }

    /// A point probe's transfer from its reference, averaged over the windows
    /// the readout has shown, drawn from the readout's cache and taken again
    /// only when it has none; or why there is none to draw.
    pub(super) fn probe_transfer(
        ui: &mut egui::Ui,
        field: &str,
        samples: &[PointProbeRecord],
        input: &TransferInput,
        color: Color32,
        view: &mut ProbeViewState,
        top_hz: f64,
    ) {
        let decibels = view.readout.spectrum_decibels;
        let mut fit = false;
        ui.horizontal(|ui| {
            ui.small(format!(
                "{field} ÷ {} · {} · {} × {:.1} s",
                input.label,
                if decibels { "dB re 1" } else { "ratio" },
                view.transfer.windows(),
                view.readout.transfer_segment,
            ));
            if ui
                .small_button("Reset")
                .on_hover_text(
                    "Start the average again. Any change to a source of the running scene \
                     starts it again too.",
                )
                .clicked()
            {
                view.transfer.reset = true;
                view.spectra.plots.remove("transfer");
            }
            fit = ui
                .small_button("Fit")
                .on_hover_text(
                    "Zoom the spectra and the transfer to the band the transfer has values in; \
                     the wheel and a drag move it from there",
                )
                .clicked();
        });
        let source = match &input.data {
            Some(TransferData::Probe(records)) => TransferSource::Probe(records),
            Some(TransferData::Signal(signal, drives)) => TransferSource::Signal(*signal, *drives),
            None => {
                ui.colored_label(
                    GOLD,
                    match input.reference {
                        TransferReference::Probe(_) => {
                            format!("{} has recorded nothing", input.label)
                        }
                        _ => format!("{} imposes no signal in the running scene", input.label),
                    },
                );
                return;
            }
        };
        let window = Self::probe_time_window(samples, view);
        if !view.spectra.plots.contains_key("transfer") {
            if let Some((_, end)) = window {
                let (segment, live) = (view.readout.transfer_segment, view.live);
                view.transfer
                    .update(samples, input, source, end, segment, live);
            }
            let points = view.transfer.points(top_hz, decibels);
            view.spectra.plots.insert("transfer".to_owned(), points);
        }
        let points = &view.spectra.plots["transfer"];
        if fit {
            view.band = fitted_band(points);
        }
        let floor = transfer_plot_floor(points, decibels);
        zoomable_line_plot(
            ui,
            points,
            floor,
            color,
            "Hz",
            &mut view.band,
            92.0,
            "Waiting for samples",
        );
    }
}

/// A reference's own data: a probe's record, or a source's signal and what
/// it drives.
#[derive(Clone, Debug)]
pub(super) enum TransferData {
    Probe(Vec<PointProbeRecord>),
    Signal(TimeSignal, SignalUse),
}

/// What a readout draws a transfer from: the reference, what it is called,
/// and its data, if it recorded or imposes anything.
#[derive(Clone, Debug)]
pub(super) struct TransferInput {
    pub(super) reference: TransferReference,
    pub(super) label: String,
    pub(super) data: Option<TransferData>,
    /// The running scene's sources, whose change restarts the average.
    pub(super) sources: Option<SourceSet>,
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{click, laid_out, settle, with_baffles};
    use super::*;
    use std::collections::BTreeSet;
    use std::f64::consts::TAU;

    /// Records at 120 Hz, a probe's rate, from sample `first` to `last`.
    fn recorded(first: usize, last: usize, value: impl Fn(f64) -> f64) -> Vec<PointProbeRecord> {
        (first..last)
            .map(|index| {
                let time = index as f64 / 120.0;
                PointProbeRecord {
                    time,
                    displacement: value(time),
                    ..Default::default()
                }
            })
            .collect()
    }

    /// Hann bursts on a 3 Hz carrier, a second long and back to back, as a
    /// source repeating a flat-top pulse of edge half its length makes.
    fn bursts(time: f64) -> f64 {
        let phase = time.rem_euclid(1.0);
        (std::f64::consts::PI * phase).sin().powi(2) * (TAU * 3.0 * time).sin()
    }

    /// The largest departure from `gain` of every value drawn, how many
    /// there are, and whether the plot leaves a gap above 15 Hz.
    fn departure(points: &[[f64; 2]], gain: f64) -> (f64, usize, bool) {
        let drawn = points
            .iter()
            .filter(|[_, value]| value.is_finite())
            .collect::<Vec<_>>();
        let within = drawn
            .iter()
            .map(|[_, value]| (value - gain).abs())
            .fold(0.0, f64::max);
        let gap = points
            .iter()
            .any(|[frequency, value]| *frequency > 15.0 && value.is_nan());
        (within, drawn.len(), gap)
    }

    /// A reference probe's input, with no running scene.
    fn from_probe(id: u64) -> TransferInput {
        TransferInput {
            reference: TransferReference::Probe(ProbeId(id)),
            label: "There".into(),
            data: None,
            sources: None,
        }
    }

    /// The gain drawn at the frequency nearest `frequency`.
    fn gain_at(points: &[[f64; 2]], frequency: f64) -> f64 {
        points
            .iter()
            .min_by(|a, b| {
                (a[0] - frequency)
                    .abs()
                    .total_cmp(&(b[0] - frequency).abs())
            })
            .unwrap()[1]
    }

    /// A copy as strong as a half of its reference reads a half wherever the
    /// reference holds anything, and nothing until the reference's ring
    /// covers the whole span the response shows.
    #[test]
    fn a_simultaneous_copy_reads_its_scale() {
        let reference = recorded(120, 1200, bursts);
        let response = recorded(0, 1200, |time| 0.5 * bursts(time));
        let input = from_probe(2);
        let mut view = TransferView::default();
        view.update(
            &response,
            &input,
            TransferSource::Probe(&reference),
            9.5,
            8.0,
            false,
        );
        let (within, drawn, gap) = departure(&view.points(0.0, false), 0.5);
        assert!(within < 1.0e-9, "departs {within:.3e}");
        assert!(drawn > 10 && gap, "{drawn} drawn");
        let mut early = TransferView::default();
        early.update(
            &response,
            &input,
            TransferSource::Probe(&reference),
            8.0,
            8.0,
            false,
        );
        assert_eq!(early.windows(), 0);
        assert!(early.points(0.0, false).is_empty());
    }

    /// Live, a window a burst long against a copy 0.3 s late reads a
    /// different gain at every refresh; the readout's average of those
    /// windows holds still, from one stretch of refreshes to the next.
    #[test]
    fn a_live_average_holds_where_one_window_wobbles() {
        let reference = recorded(0, 3000, bursts);
        let response = recorded(0, 3000, |time| 0.4 * bursts(time - 0.3));
        let input = from_probe(2);
        let mut view = TransferView::default();
        let (mut single, mut settled) = (Vec::new(), Vec::new());
        for step in 0..80 {
            let end = 2.0 + 0.25 * f64::from(step);
            let mut one = TransferView::default();
            one.update(
                &response,
                &input,
                TransferSource::Probe(&reference),
                end,
                1.25,
                true,
            );
            single.push(gain_at(&one.points(0.0, false), 2.0));
            view.update(
                &response,
                &input,
                TransferSource::Probe(&reference),
                end,
                1.25,
                true,
            );
            if step == 39 || step == 79 {
                settled.push(gain_at(&view.points(0.0, false), 2.0));
            }
        }
        assert_eq!(view.windows(), 80);
        let spread = single.iter().copied().fold(0.0, f64::max)
            - single.iter().copied().fold(f64::MAX, f64::min);
        assert!(spread > 0.1, "single windows spread {spread:.3}");
        assert!(
            (settled[0] - settled[1]).abs() < 2.0e-3,
            "{} then {}",
            settled[0],
            settled[1]
        );
    }

    /// Through segments eight seconds long, the default, a copy 0.3 s late
    /// averages to within 1% of its scale, where one segment a burst long
    /// read 32% low: the Hann window's overlap with itself shifted by the
    /// delay is 0.991 at a delay of 1/27 of the segment.
    #[test]
    fn a_long_segment_reads_a_late_copy_at_its_scale() {
        let reference = recorded(0, 4000, bursts);
        let response = recorded(0, 4000, |time| 0.4 * bursts(time - 0.3));
        let input = from_probe(2);
        let mut view = TransferView::default();
        for step in 0..40 {
            view.update(
                &response,
                &input,
                TransferSource::Probe(&reference),
                10.0 + 0.5 * f64::from(step),
                DEFAULT_TRANSFER_SEGMENT,
                true,
            );
        }
        assert_eq!(view.windows(), 40);
        let points = view.points(0.0, false);
        for line in [2.0, 3.0, 4.0] {
            let gain = gain_at(&points, line);
            assert!((gain / 0.4 - 1.0).abs() < 1.0e-2, "{line} Hz reads {gain}");
        }
    }

    /// The average starts over at the Reset, for another reference or segment,
    /// when a source of the running scene changes, and when a live view's
    /// time goes back; a parked view adds only its first window.
    #[test]
    fn a_transfer_average_starts_over_when_what_it_divides_changes() {
        let reference = recorded(0, 1200, bursts);
        let response = recorded(0, 1200, |time| 0.5 * bursts(time));
        let source = TransferSource::Probe(&reference);
        let input = from_probe(2);
        let mut view = TransferView::default();
        let take = |view: &mut TransferView, input: &TransferInput, end: f64, span: f64, live| {
            view.update(&response, input, source, end, span, live);
            view.windows()
        };
        assert_eq!(take(&mut view, &input, 3.0, 2.0, true), 1);
        assert_eq!(take(&mut view, &input, 3.25, 2.0, true), 2);
        assert_eq!(take(&mut view, &input, 3.25, 2.0, true), 2);
        assert_eq!(take(&mut view, &input, 3.5, 2.0, false), 2);
        assert_eq!(take(&mut view, &input, 2.5, 2.0, true), 1);
        assert_eq!(take(&mut view, &input, 2.75, 2.0, true), 2);
        assert_eq!(take(&mut view, &input, 3.0, 3.0, true), 1);
        assert_eq!(take(&mut view, &input, 3.25, 3.0, true), 2);
        assert_eq!(take(&mut view, &from_probe(3), 3.5, 3.0, true), 1);
        let scene = TopologyScene::default();
        let point = PointSource::default();
        let running = TransferInput {
            sources: Some(SourceSet::of(&scene, point)),
            ..from_probe(3)
        };
        assert_eq!(take(&mut view, &running, 3.75, 3.0, true), 1);
        assert_eq!(take(&mut view, &running, 4.0, 3.0, true), 2);
        let moved = TransferInput {
            sources: Some(SourceSet::of(
                &scene,
                PointSource {
                    position: Point2::new(0.3, 0.1),
                    ..point
                },
            )),
            ..from_probe(3)
        };
        assert_eq!(take(&mut view, &moved, 4.25, 3.0, true), 1);
        view.reset = true;
        assert_eq!(take(&mut view, &moved, 4.5, 3.0, true), 1);
        assert!(!view.reset);
    }

    /// A source reference is its signal evaluated at the response's own
    /// sample times, through the same window.
    #[test]
    fn a_source_reference_is_its_signal_on_the_responses_timing() {
        let signal = TimeSignal::pulsed(
            [0.0, 1.0, 3.0, 0.0],
            PulseEnvelope::FlatTop {
                duration: 1.0,
                edge: 0.5,
            },
            0.0,
            1.0,
        );
        let response = recorded(0, 1200, |time| 0.25 * signal.value(time));
        let input = TransferInput {
            reference: TransferReference::PointSource,
            label: "Point source".into(),
            data: None,
            sources: None,
        };
        let mut view = TransferView::default();
        view.update(
            &response,
            &input,
            TransferSource::Signal(signal, SignalUse::Source),
            9.0,
            8.0,
            false,
        );
        let (within, drawn, gap) = departure(&view.points(0.0, false), 0.25);
        assert!(within < 1.0e-9, "departs {within:.3e}");
        assert!(drawn > 10 && gap, "{drawn} drawn");
    }

    /// A transfer is drawn over the whole band a spectrum is, from 0 Hz to
    /// its ceiling with gaps where it divides nothing, and from zero, or in
    /// decibels from 100 dB under its largest gain, which also floors it.
    #[test]
    fn a_transfer_is_ranged_as_a_spectrum_is() {
        let reference = recorded(0, 1200, bursts);
        let response = recorded(0, 1200, |time| 0.5 * bursts(time - 0.2));
        let mut view = TransferView::default();
        view.update(
            &response,
            &from_probe(2),
            TransferSource::Probe(&reference),
            9.0,
            9.0,
            false,
        );
        let linear = view.points(12.0, false);
        assert_eq!(linear[0][0], 0.0);
        assert!(linear[0][1].is_nan());
        let last = linear[linear.len() - 1][0];
        assert!(last <= 12.0 && last > 11.9, "ends at {last} Hz");
        assert_eq!(transfer_plot_floor(&linear, false), Some(0.0));
        let decibels = view.points(12.0, true);
        let largest = decibels
            .iter()
            .map(|[_, value]| *value)
            .filter(|value| value.is_finite())
            .fold(f64::MIN, f64::max);
        assert_eq!(
            transfer_plot_floor(&decibels, true),
            Some(largest - DECIBEL_RANGE)
        );
        assert!(
            decibels
                .iter()
                .filter(|[_, value]| value.is_finite())
                .all(|[_, value]| *value >= largest - DECIBEL_RANGE - 1.0e-9)
        );
        assert_eq!(transfer_plot_floor(&[[0.0, f64::NAN]], true), None);
    }

    /// A continuous source or Neumann signal imposes the rate it integrates
    /// from the anchor, a pulse imposes itself, and a pinned field is its
    /// signal either way.
    #[test]
    fn a_continuous_signal_imposes_what_it_integrates() {
        let signal = TimeSignal::harmonic(0.2, 2.0, 1.5, 0.3);
        let (steps, end) = (200_000, 2.0);
        let dt = end / f64::from(steps);
        let integral = (0..steps)
            .map(|index| signal.value((f64::from(index) + 0.5) * dt) * dt)
            .sum::<f64>();
        for drives in [SignalUse::Source, SignalUse::Flux] {
            let imposed = drives.imposed(signal, end).unwrap();
            assert!((imposed - integral).abs() < 1.0e-8, "{imposed} {integral}");
        }
        assert_eq!(
            SignalUse::Field.imposed(signal, end),
            Some(signal.value(end))
        );
        let pulse = TimeSignal::pulsed(
            [0.0, 1.0, 3.0, 0.0],
            PulseEnvelope::Gaussian { width: 0.08 },
            0.5,
            0.0,
        );
        assert_eq!(
            SignalUse::Source.imposed(pulse, 0.8),
            Some(pulse.value(0.8))
        );
    }

    /// Every signal a scene imposes is a candidate, named as the readout
    /// lists it, and nothing that imposes none.
    #[test]
    fn every_imposed_signal_is_a_candidate() {
        let mut state = with_baffles(&[]);
        let span = state.editor.document.model.draft.geometry.curves[0].spans[0].id;
        let pinned = TimeSignal::harmonic(0.0, 1.0, 2.0, 0.0);
        state
            .editor
            .set_span_face_condition(
                &BTreeSet::from([span]),
                CurveTraceSide::Left,
                FaceBoundaryCondition::Dirichlet { signal: pinned },
            )
            .unwrap();
        state
            .editor
            .set_outer_condition(
                &BTreeSet::from([OuterSide::Top]),
                OuterBoundaryCondition::Neumann { signal: pinned },
            )
            .unwrap();
        settle(&mut state.editor);
        let mut source = state.editor.document.model.source;
        source.enabled = true;
        state.editor.set_point_source(source).unwrap();
        let probe = state
            .editor
            .create_probe(
                "Here".into(),
                [255, 255, 255],
                TopologyProbeTarget::Point(Point2::new(0.5, 0.5)),
            )
            .unwrap();
        let other = state
            .editor
            .create_probe(
                "There".into(),
                [255, 255, 255],
                TopologyProbeTarget::Point(Point2::new(-0.5, 0.5)),
            )
            .unwrap();
        settle(&mut state.editor);
        let candidates = state.transfer_candidates(probe);
        assert_eq!(
            candidates,
            vec![
                (TransferReference::Probe(other), "There".to_owned()),
                (TransferReference::PointSource, "Point source".to_owned()),
                (
                    TransferReference::Wall(OuterSide::Top),
                    "Top wall".to_owned()
                ),
                (
                    TransferReference::Face {
                        span,
                        side: CurveTraceSide::Left
                    },
                    format!("Span {} · left side", span.0)
                ),
            ]
        );
        let model = &state.editor.document.model;
        let imposed = signal_sources(&model.accepted, model.source);
        assert_eq!(
            imposed[1],
            (
                TransferReference::Wall(OuterSide::Top),
                pinned,
                SignalUse::Flux
            )
        );
        assert_eq!(imposed[2].2, SignalUse::Field);
    }

    /// A point probe's readout lists what its transfer can divide by, and
    /// picking one keeps it with the scene and draws the transfer.
    #[test]
    fn a_point_readout_picks_what_its_transfer_divides_by() {
        let mut state = Playground::default();
        let probe = state
            .editor
            .create_probe(
                "Here".into(),
                [255, 255, 255],
                TopologyProbeTarget::Point(Point2::new(0.5, 0.5)),
            )
            .unwrap();
        let other = state
            .editor
            .create_probe(
                "There".into(),
                [255, 255, 255],
                TopologyProbeTarget::Point(Point2::new(-0.5, 0.5)),
            )
            .unwrap();
        settle(&mut state.editor);
        state.probe_windows.insert(probe);
        let context = egui::Context::default();
        theme::apply(&context);
        context.enable_accesskit();
        let pass = |state: &mut Playground, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 900.0),
                )),
                events,
                ..egui::RawInput::default()
            };
            laid_out(&context.run_ui(input, |ui| state.probe_windows(ui.ctx())))
        };
        let press = |state: &mut Playground, label: &str| {
            let widgets = pass(state, vec![]);
            let widget = widgets
                .iter()
                .find(|widget| widget.label.starts_with(label))
                .unwrap_or_else(|| panic!("no {label} among {widgets:?}"))
                .clone();
            pass(state, click(&widget));
        };
        pass(&mut state, vec![]);
        press(&mut state, "Plots");
        press(&mut state, "Transfer from: none");
        press(&mut state, "There");
        assert_eq!(
            state.editor.document.readouts.probe(probe).transfer_from,
            Some(TransferReference::Probe(other))
        );
        let widgets = pass(&mut state, vec![]);
        assert!(
            widgets
                .iter()
                .any(|widget| widget.label.contains("÷ There · ratio")),
            "{widgets:?}"
        );
        assert!(
            widgets
                .iter()
                .any(|widget| widget.label == "There has recorded nothing"),
            "{widgets:?}"
        );
    }
}
