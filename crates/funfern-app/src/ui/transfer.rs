//! A point probe's transfer readout: its field over a reference's, frequency
//! by frequency, where the reference is another point probe or what a source
//! imposes as the solver runs it.

use super::line_plot::line_plot;
use super::probe_view::ProbeViewState;
use super::signals::SignalUse;
use super::*;
use crate::wave_gpu::PointProbeRecord;
use funfern_app::document::TransferReference;
use funfern_app::topology_editor::TopologyProbeTarget;

/// A source's signal, what it drives, and since when the running generation
/// has driven it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TrackedSignal {
    pub(super) signal: TimeSignal,
    pub(super) drives: SignalUse,
    /// Simulated seconds from which this signal has been in force. Before
    /// then another signal ran, whose response a transfer must not divide by
    /// this one.
    pub(super) since: f64,
}

/// What a transfer divides by over a span.
#[derive(Clone, Copy, Debug)]
pub(super) enum TransferSource<'a> {
    Probe(&'a [PointProbeRecord]),
    Signal(TrackedSignal),
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

/// The transfer from `reference` to `response`'s field over the records
/// from `from` to `to`, as plot points up to `top_hz` or half the sample
/// rate, whichever is lower, and in decibels re one if asked. A frequency the
/// reference does not hold is a gap. Against a probe both records are
/// resampled over the span they share; against a source the response is
/// taken only from when that source's signal came into force. Nothing for
/// fewer than eight samples.
pub(super) fn transfer_points(
    response: &[PointProbeRecord],
    reference: TransferSource<'_>,
    (from, to): (f64, f64),
    top_hz: f64,
    decibels: bool,
) -> Vec<[f64; 2]> {
    let from = match reference {
        TransferSource::Probe(_) => from,
        TransferSource::Signal(tracked) => from.max(tracked.since),
    };
    let within = |records: &[PointProbeRecord]| -> (Vec<f64>, Vec<f64>) {
        records
            .iter()
            .filter(|record| record.time >= from && record.time <= to)
            .map(|record| (record.time, record.displacement))
            .filter(|(_, value)| value.is_finite())
            .unzip()
    };
    let (times, values) = within(response);
    if times.len() < 8 {
        return Vec::new();
    }
    let mut spacings = times
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect::<Vec<_>>();
    spacings.sort_by(f64::total_cmp);
    let interval = spacings[spacings.len() / 2];
    let series = match reference {
        TransferSource::Probe(records) => {
            let (reference_times, reference_values) = within(records);
            if reference_times.len() < 8 {
                return Vec::new();
            }
            let start = times[0].max(reference_times[0]);
            let end = times[times.len() - 1].min(reference_times[reference_times.len() - 1]);
            if end - start < 8.0 * interval {
                return Vec::new();
            }
            resample_between(&reference_times, &reference_values, start, end, interval).and_then(
                |reference| {
                    resample_between(&times, &values, start, end, interval)
                        .map(|response| (reference, response))
                },
            )
        }
        TransferSource::Signal(tracked) => {
            resample_evenly(&times, &values, interval).and_then(|response| {
                (0..response.len())
                    .map(|index| {
                        let time = times[0] + index as f64 * interval;
                        tracked.drives.imposed(tracked.signal, time)
                    })
                    .collect::<Option<Vec<_>>>()
                    .map(|reference| (reference, response))
                    .ok_or(SpectrumError::NonFinite)
            })
        }
    };
    let Ok(transfer) =
        series.and_then(|(reference, response)| transfer_spectrum(&reference, &response, interval))
    else {
        return Vec::new();
    };
    let nyquist = 0.5 / interval;
    let top = if top_hz > 0.0 {
        top_hz.min(nyquist)
    } else {
        nyquist
    };
    (0..transfer.ratios.len())
        .map(|index| (transfer.frequency_hz(index), transfer.magnitude(index)))
        .take_while(|(frequency, _)| *frequency <= top)
        .map(|(frequency, gain)| {
            let shown = gain.map_or(f64::NAN, |gain| {
                if decibels {
                    20.0 * gain.max(f64::MIN_POSITIVE).log10()
                } else {
                    gain
                }
            });
            [frequency, shown]
        })
        .collect()
}

impl Playground {
    /// Notes when each source's signal, as the running generation drives it,
    /// last changed. A transfer from a source divides by that signal only
    /// over the time it has been in force.
    pub(super) fn track_transfer_signals(&mut self) {
        let Some(active) = self.runtime.active() else {
            return;
        };
        let now = self.simulated_time();
        let current = signal_sources(&active.bundle.authored, active.point_source);
        self.transfer_signals
            .retain(|reference, _| current.iter().any(|(held, ..)| held == reference));
        for (reference, signal, drives) in current {
            let unchanged = self
                .transfer_signals
                .get(&reference)
                .is_some_and(|tracked| tracked.signal == signal && tracked.drives == drives);
            if !unchanged {
                self.transfer_signals.insert(
                    reference,
                    TrackedSignal {
                        signal,
                        drives,
                        since: now,
                    },
                );
            }
        }
    }

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
    /// reference probe's record or the source's tracked signal.
    pub(super) fn transfer_input(&self, reference: TransferReference) -> TransferInput {
        let data = match reference {
            TransferReference::Probe(id) => self
                .probe_traces
                .get(&id)
                .map(|trace| TransferData::Probe(trace.samples.iter().copied().collect())),
            _ => self
                .transfer_signals
                .get(&reference)
                .copied()
                .map(TransferData::Signal),
        };
        TransferInput {
            reference,
            label: self.transfer_reference_label(reference),
            data,
        }
    }

    /// A point probe's transfer from its reference over the span in view,
    /// drawn from the readout's cache and computed only when it has none; or
    /// why there is none to draw.
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
        ui.small(format!(
            "{field} ÷ {} · {}",
            input.label,
            if decibels { "dB re 1" } else { "ratio" }
        ));
        let source = match &input.data {
            Some(TransferData::Probe(records)) => TransferSource::Probe(records),
            Some(TransferData::Signal(tracked)) => TransferSource::Signal(*tracked),
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
        if let (TransferSource::Signal(tracked), Some((from, _))) = (source, window)
            && tracked.since > from
        {
            ui.small(format!(
                "From {:.2} s, when its signal last changed",
                tracked.since
            ));
        }
        let points = view
            .spectra
            .plots
            .entry("transfer".to_owned())
            .or_insert_with(|| {
                window.map_or_else(Vec::new, |window| {
                    transfer_points(samples, source, window, top_hz, decibels)
                })
            });
        line_plot(ui, points, color, "Hz", &[], 92.0, "Waiting for samples");
    }

    /// What a readout's spectra key on for its transfer: the reference, and
    /// when a source's signal came into force.
    pub(super) fn transfer_key(input: Option<&TransferInput>) -> Option<(TransferReference, f64)> {
        input.map(|input| {
            let since = match &input.data {
                Some(TransferData::Signal(tracked)) => tracked.since,
                _ => 0.0,
            };
            (input.reference, since)
        })
    }
}

/// A reference's own data: a probe's record or a source's signal.
#[derive(Clone, Debug)]
pub(super) enum TransferData {
    Probe(Vec<PointProbeRecord>),
    Signal(TrackedSignal),
}

/// What a readout draws a transfer from: the reference, what it is called,
/// and its data, if it recorded or imposes anything.
#[derive(Clone, Debug)]
pub(super) struct TransferInput {
    pub(super) reference: TransferReference,
    pub(super) label: String,
    pub(super) data: Option<TransferData>,
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{activate, click, laid_out, settle, with_baffles};
    use super::*;
    use std::collections::BTreeSet;
    use std::f64::consts::TAU;

    /// A Gaussian burst on a 3 Hz carrier, centred at `centre`.
    fn burst(time: f64, centre: f64) -> f64 {
        let from_centre = time - centre;
        (-0.5 * (from_centre / 0.08).powi(2)).exp() * (TAU * 3.0 * from_centre).sin()
    }

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

    /// The largest departure from `gain` over 2 to 4 Hz, where the burst is
    /// strong, and whether the plot leaves a gap where it is not.
    fn departure(points: &[[f64; 2]], gain: f64) -> (f64, bool) {
        let within = points
            .iter()
            .filter(|[frequency, _]| (2.0..=4.0).contains(frequency))
            .map(|[_, value]| (value - gain).abs())
            .fold(0.0, f64::max);
        let gap = points
            .iter()
            .any(|[frequency, value]| *frequency > 15.0 && value.is_nan());
        (within, gap)
    }

    /// Against another probe, a copy half as strong and 0.7 s late reads a
    /// half across the burst's band, though the reference's ring started a
    /// second after the response's; and nothing where the burst is silent.
    #[test]
    fn a_probe_transfer_reads_a_late_copy_at_its_scale() {
        let reference = recorded(120, 720, |time| burst(time, 2.0));
        let response = recorded(0, 720, |time| 0.5 * burst(time - 0.7, 2.0));
        let points = transfer_points(
            &response,
            TransferSource::Probe(&reference),
            (0.0, 6.0),
            0.0,
            false,
        );
        let (within, gap) = departure(&points, 0.5);
        assert!(within < 1.0e-6, "departs {within:.3e}");
        assert!(gap);
        let decibels = transfer_points(
            &response,
            TransferSource::Probe(&reference),
            (0.0, 6.0),
            0.0,
            true,
        );
        let (within, _) = departure(&decibels, 20.0 * 0.5_f64.log10());
        assert!(within < 1.0e-4, "departs {within:.3e} dB");
        assert!(
            transfer_points(
                &response,
                TransferSource::Probe(&reference[..4]),
                (0.0, 6.0),
                0.0,
                false
            )
            .is_empty()
        );
    }

    /// Against a source, only the time since its signal came into force
    /// counts: the response to a pulse before it is left out, and what is
    /// left transfers at its scale. Counted from the start, the earlier
    /// response would have been divided by a signal that did not make it.
    #[test]
    fn a_source_transfer_counts_only_from_when_its_signal_came_into_force() {
        let signal = TimeSignal::pulsed(
            [0.0, 1.0, 3.0, 0.0],
            PulseEnvelope::Gaussian { width: 0.08 },
            2.6,
            0.0,
        );
        let response = recorded(0, 720, |time| {
            burst(time, 1.0) + 0.25 * signal.value(time - 0.4)
        });
        let tracked = TrackedSignal {
            signal,
            drives: SignalUse::Source,
            since: 2.0,
        };
        let points = transfer_points(
            &response,
            TransferSource::Signal(tracked),
            (0.0, 6.0),
            0.0,
            false,
        );
        let (within, gap) = departure(&points, 0.25);
        assert!(within < 1.0e-6, "departs {within:.3e}");
        assert!(gap);
        let unclipped = transfer_points(
            &response,
            TransferSource::Signal(TrackedSignal {
                since: 0.0,
                ..tracked
            }),
            (0.0, 6.0),
            0.0,
            false,
        );
        assert!(departure(&unclipped, 0.25).0 > 0.1);
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

    /// A source's signal is in force from the frame the running generation
    /// first drives it. A new generation that keeps it keeps its time, one
    /// that changes it starts it again, and a restart counts it from zero.
    #[test]
    fn a_tracked_signal_counts_from_when_it_changed() {
        let mut state = Playground::default();
        let mut source = state.editor.document.model.source;
        source.enabled = true;
        state.editor.set_point_source(source).unwrap();
        settle(&mut state.editor);
        activate(&mut state);
        state.track_transfer_signals();
        let since =
            |state: &Playground| state.transfer_signals[&TransferReference::PointSource].since;
        assert_eq!(since(&state), 0.0);
        state.sim_time_offset = 5.0;
        activate(&mut state);
        state.track_transfer_signals();
        assert_eq!(since(&state), 0.0);
        source.signal = TimeSignal::harmonic(0.0, 1.0, 4.0, 0.0);
        state.editor.set_point_source(source).unwrap();
        settle(&mut state.editor);
        activate(&mut state);
        state.track_transfer_signals();
        assert_eq!(since(&state), 5.0);
        state.restart_probe_traces();
        assert_eq!(since(&state), 0.0);
        source.enabled = false;
        state.editor.set_point_source(source).unwrap();
        settle(&mut state.editor);
        activate(&mut state);
        state.track_transfer_signals();
        assert!(state.transfer_signals.is_empty());
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
