//! Per-probe presentation state: what each readout is showing, over what time
//! span, and the sample rings behind it.
use super::transfer::TransferView;
use super::{GOLD, RED, SELECT, TEAL, primary_field_label, transverse_field_magnitude_label};
use crate::wave_gpu::{AreaProbeRecord, CurveProbeRecord, FarFieldRecord, PointProbeRecord};
use bevy::platform::time::Instant;
use bevy_egui::egui::Color32;
use funfern_core::*;
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Debug)]
pub(super) struct ProbeTrace {
    pub(super) samples: VecDeque<PointProbeRecord>,
    pub(super) last_time: f64,
}

pub(super) use funfern_app::document::{LineProbeQuantity, LineProbeRepresentation};
use funfern_app::document::{ProbeReadout, TransferReference};

/// What the readout draws for each line-probe quantity: its name in this skin,
/// its colour, and whether the skin has it.
pub(super) trait LineProbeQuantityView {
    fn label_for(self, physics: PhysicsModel) -> &'static str;
    fn color(self) -> Color32;
    fn applies(self, physics: PhysicsModel) -> bool;
}

impl LineProbeQuantityView for LineProbeQuantity {
    fn label_for(self, physics: PhysicsModel) -> &'static str {
        match self {
            Self::Field => primary_field_label(physics),
            Self::Transverse => transverse_field_magnitude_label(physics),
            Self::Flux => match physics {
                PhysicsModel::Mechanical => "Normal energy flux",
                PhysicsModel::Electromagnetic { .. } => "Normal Poynting flux",
            },
            // Named plainly rather than with angle brackets: egui's default
            // font has no glyph for those and drew them as tofu.
            Self::MeanFlux => "Average flux",
            Self::Energy => "Energy density",
            Self::MeanEnergy => "Average energy density",
        }
    }

    fn color(self) -> Color32 {
        match self {
            Self::Field => SELECT,
            Self::Transverse => Color32::from_rgb(188, 139, 255),
            Self::Flux => TEAL,
            Self::MeanFlux => RED,
            Self::Energy => GOLD,
            Self::MeanEnergy => Color32::from_rgb(240, 150, 90),
        }
    }

    fn applies(self, _physics: PhysicsModel) -> bool {
        true
    }
}

/// Per-readout presentation: where the shared time window sits, and the
/// document's `ProbeReadout` for which traces are drawn over how long. Every
/// trace in a readout pans and zooms together.
#[derive(Clone, Debug)]
pub(super) struct ProbeViewState {
    pub(super) live: bool,
    pub(super) end_time: f64,
    pub(super) readout: ProbeReadout,
    pub(super) spectra: SpectrumCache,
    /// A point readout's transfer, averaged over the windows it has shown.
    pub(super) transfer: TransferView,
    /// The frequencies the readout's spectra and transfer show, all of them
    /// unless zoomed, shared as their traces share a time window.
    pub(super) band: Option<[f64; 2]>,
}

impl ProbeViewState {
    pub(super) fn new(readout: ProbeReadout) -> Self {
        Self {
            live: true,
            end_time: 0.0,
            readout,
            spectra: SpectrumCache::default(),
            transfer: TransferView::default(),
            band: None,
        }
    }
}

/// Wall seconds between a live readout's spectra. Each is an FFT of up to
/// the whole history, which a frame need not repeat for a plot that moves
/// this little.
pub(super) const SPECTRUM_REFRESH_SECONDS: f64 = 0.25;

/// What a readout's spectra were drawn for: the settings and the span in
/// view, which a live view does not fix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SpectrumKey {
    pub(super) decibels: bool,
    pub(super) top_hz: f64,
    pub(super) span: f64,
    pub(super) end_time: Option<f64>,
    /// What a transfer divides by, over what segment.
    pub(super) transfer: Option<(TransferReference, f64)>,
}

/// A readout's spectra as last drawn, by plot label.
#[derive(Clone, Debug, Default)]
pub(super) struct SpectrumCache {
    pub(super) plots: BTreeMap<String, Vec<[f64; 2]>>,
    drawn_for: Option<SpectrumKey>,
    drawn_at: Option<Instant>,
    drawn_newest: f64,
}

impl SpectrumCache {
    /// Forgets the drawn spectra when what they were drawn for changed, or
    /// when a live view has new samples and a refresh has passed.
    pub(super) fn refresh(&mut self, key: SpectrumKey, newest: f64, now: Instant) {
        let due = self
            .drawn_at
            .is_none_or(|at| now.duration_since(at).as_secs_f64() >= SPECTRUM_REFRESH_SECONDS);
        let moved = key.end_time.is_none() && newest != self.drawn_newest && due;
        if self.drawn_for != Some(key) || moved {
            self.plots.clear();
            self.drawn_for = Some(key);
            self.drawn_at = Some(now);
            self.drawn_newest = newest;
        }
    }
}

/// How far under its loudest a readout's plot in decibels reaches, where it
/// floors: 100 dB, an amplitude a hundred thousand times smaller.
pub(super) const DECIBEL_RANGE: f64 = 100.0;

/// The amplitude spectrum of `value` over the records from `from` to `to`, as
/// plot points up to `top_hz` or to half the sample rate, whichever is lower,
/// and in decibels re one unit if asked, floored 100 dB under the peak.
/// Records are resampled at their median spacing, which a handoff that
/// changes the step changes; a record that is not finite is left out.
/// Nothing for fewer than eight records.
pub(super) fn trace_spectrum(
    records: &[PointProbeRecord],
    value: impl Fn(&PointProbeRecord) -> f64,
    (from, to): (f64, f64),
    top_hz: f64,
    decibels: bool,
) -> Vec<[f64; 2]> {
    let (times, values): (Vec<_>, Vec<_>) = records
        .iter()
        .filter(|record| record.time >= from && record.time <= to)
        .map(|record| (record.time, value(record)))
        .filter(|(_, value)| value.is_finite())
        .unzip();
    if times.len() < 8 {
        return Vec::new();
    }
    let mut spacings = times
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect::<Vec<_>>();
    spacings.sort_by(f64::total_cmp);
    let interval = spacings[spacings.len() / 2];
    let Ok(spectrum) = resample_evenly(&times, &values, interval)
        .and_then(|even| amplitude_spectrum(&even, interval))
    else {
        return Vec::new();
    };
    let nyquist = 0.5 / interval;
    let top = if top_hz > 0.0 {
        top_hz.min(nyquist)
    } else {
        nyquist
    };
    let peak = spectrum.magnitudes.iter().copied().fold(0.0, f64::max);
    let floor = peak * 10.0_f64.powf(-DECIBEL_RANGE / 20.0);
    (0..spectrum.magnitudes.len())
        .map(|index| (spectrum.frequency_hz(index), spectrum.magnitudes[index]))
        .take_while(|(frequency, _)| *frequency <= top)
        .map(|(frequency, magnitude)| {
            let shown = if decibels {
                20.0 * magnitude.max(floor).max(f64::MIN_POSITIVE).log10()
            } else {
                magnitude
            };
            [frequency, shown]
        })
        .collect()
}

/// What a readout's window changed of its document readout, if anything. The
/// window shows `shown`, which is `stored` with its span and mean window
/// clamped to the history recorded so far; those clamps are the display's, so
/// a span or window the user left alone keeps its stored value rather than
/// being shortened for good by a look early in a run.
pub(super) fn edited_readout(
    stored: ProbeReadout,
    shown: ProbeReadout,
    after: ProbeReadout,
) -> Option<ProbeReadout> {
    if after == shown {
        return None;
    }
    let mut edited = after;
    if after.span == shown.span {
        edited.span = stored.span;
    }
    if after.mean_window == shown.mean_window {
        edited.mean_window = stored.mean_window;
    }
    Some(edited)
}

pub(super) struct CurveTrace {
    pub(super) records: VecDeque<CurveProbeRecord>,
    pub(super) last_time: f64,
}
impl Default for CurveTrace {
    fn default() -> Self {
        Self {
            records: VecDeque::new(),
            last_time: -1.0,
        }
    }
}

pub(super) struct AreaTrace {
    pub(super) records: VecDeque<AreaProbeRecord>,
    pub(super) last_time: f64,
}
impl Default for AreaTrace {
    fn default() -> Self {
        Self {
            records: VecDeque::new(),
            last_time: -1.0,
        }
    }
}

pub(super) struct FarFieldTrace {
    pub(super) records: VecDeque<FarFieldRecord>,
    pub(super) last_time: f64,
}
impl Default for FarFieldTrace {
    fn default() -> Self {
        Self {
            records: VecDeque::new(),
            last_time: -1.0,
        }
    }
}
impl Default for ProbeTrace {
    fn default() -> Self {
        Self {
            samples: VecDeque::new(),
            last_time: -1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;
    use std::time::Duration;

    /// A tone recorded at 60 Hz and then, after a handoff, at 48 Hz, with a
    /// record the device could not form in between.
    fn tone(amplitude: f64, frequency: f64) -> Vec<PointProbeRecord> {
        let mut records = Vec::new();
        let mut time = 0.0;
        while time < 10.0 {
            let displacement = if records.len() == 200 {
                f64::NAN
            } else {
                0.3 + amplitude * (TAU * frequency * time).sin()
            };
            records.push(PointProbeRecord {
                time,
                displacement,
                ..Default::default()
            });
            time += if time < 5.0 { 1.0 / 60.0 } else { 1.0 / 48.0 };
        }
        records
    }

    #[test]
    fn a_probes_spectrum_reads_its_tone_across_a_change_of_spacing() {
        let records = tone(0.8, 2.0);
        let points = trace_spectrum(
            &records,
            |record| record.displacement,
            (0.0, 10.0),
            5.0,
            false,
        );
        let [frequency, amplitude] = points
            .iter()
            .copied()
            .max_by(|a, b| a[1].total_cmp(&b[1]))
            .unwrap();
        assert!((frequency - 2.0).abs() < 0.05, "peak at {frequency} Hz");
        assert!((amplitude / 0.8 - 1.0).abs() < 0.03, "read {amplitude}");
        assert!(points.last().unwrap()[0] <= 5.0);
        let decibels = trace_spectrum(
            &records,
            |record| record.displacement,
            (0.0, 10.0),
            0.0,
            true,
        );
        let loudest = decibels
            .iter()
            .map(|point| point[1])
            .fold(f64::MIN, f64::max);
        assert!((loudest - 20.0 * 0.8_f64.log10()).abs() < 0.3);
        // Half the median spacing's rate caps an automatic plot.
        assert!(decibels.last().unwrap()[0] <= 30.0);
        assert!(
            decibels
                .iter()
                .all(|point| point[1] >= loudest - 100.0 - 1.0e-9)
        );
        assert!(
            trace_spectrum(
                &records[..7],
                |record| record.displacement,
                (0.0, 1.0),
                0.0,
                false
            )
            .is_empty()
        );
    }

    /// Spectra are drawn again when what they show changes, and a live view
    /// with new samples a refresh after the last; a view the user parked is
    /// left as it is.
    #[test]
    fn a_readouts_spectra_redraw_on_change_and_on_a_live_refresh() {
        let mut cache = SpectrumCache::default();
        let live = SpectrumKey {
            decibels: false,
            top_hz: 12.0,
            span: 2.0,
            end_time: None,
            transfer: None,
        };
        let start = Instant::now();
        let redraws = |cache: &mut SpectrumCache, key, newest, at| {
            cache.plots.insert("drawn".into(), Vec::new());
            cache.refresh(key, newest, at);
            cache.plots.is_empty()
        };
        assert!(redraws(&mut cache, live, 1.0, start));
        let soon = start + Duration::from_millis(100);
        let later = start + Duration::from_millis(300);
        assert!(!redraws(&mut cache, live, 1.1, soon));
        assert!(redraws(&mut cache, live, 1.2, later));
        assert!(!redraws(
            &mut cache,
            live,
            1.2,
            later + Duration::from_secs(1)
        ));
        let decibels = SpectrumKey {
            decibels: true,
            ..live
        };
        assert!(redraws(&mut cache, decibels, 1.2, later));
        let parked = SpectrumKey {
            end_time: Some(0.9),
            ..decibels
        };
        assert!(redraws(&mut cache, parked, 1.2, later));
        assert!(!redraws(
            &mut cache,
            parked,
            1.9,
            later + Duration::from_secs(5)
        ));
    }
}
