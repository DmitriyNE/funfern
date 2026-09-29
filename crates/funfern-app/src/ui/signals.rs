//! The signal editor every source and driven wall shares: a continuous
//! harmonic, or a pulse under one of the core's envelopes, and the window
//! that shows a pulse's shape and spectrum. A material drive's gate takes the
//! same pulse controls and the same window.
use super::line_plot::{PlotMarker, line_plot};
use super::*;

/// What a signal drives, which is what its numbers mean.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum SignalUse {
    /// A point or region source. A continuous signal is the version-22
    /// acceleration it integrates; a pulse is the field rate it drives.
    Source,
    /// A Neumann side. A continuous signal is the rate of the flux it holds;
    /// a pulse is the flux.
    Flux,
    /// A Dirichlet side: the field it pins, either way.
    Field,
}

impl SignalUse {
    /// Whether a continuous signal here is integrated, so that its amplitude
    /// and a pulse's differ by the carrier's angular frequency.
    const fn integrates(self) -> bool {
        matches!(self, Self::Source | Self::Flux)
    }

    /// What a signal here imposes at `time` as the solver runs it: a
    /// continuous source or Neumann signal integrated from the anchor the
    /// application compiles against, and a pulse or a pinned field as it is.
    pub(super) fn imposed(self, signal: TimeSignal, time: f64) -> Option<f64> {
        if self.integrates() {
            CanonicalRateDrive::authored(signal, SOURCE_ANCHOR_TIME)
                .and_then(|drive| drive.value(time))
                .ok()
        } else {
            Some(signal.value(time)).filter(|value| value.is_finite())
        }
    }

    /// What a pulse here imposes, for the shape window's trace.
    const fn pulse_quantity(self) -> &'static str {
        match self {
            Self::Source => "Field rate",
            Self::Flux => "Flux",
            Self::Field => "Field",
        }
    }

    /// Whether an edit of a signal here takes a new generation rather than a
    /// live patch: a pinned field is part of the generation's layout.
    const fn takes_handoff(self) -> bool {
        matches!(self, Self::Field)
    }

    fn units(self, pulsed: bool) -> &'static str {
        match (self, pulsed) {
            (Self::Source, false) => {
                "An acceleration: the solver integrates it into the field rate the source drives."
            }
            (Self::Source, true) => {
                "The field rate the source drives, as authored: nothing is left driving once the \
                 pulse is over."
            }
            (Self::Flux, false) => "The rate of the flux: the wall holds its time integral.",
            (Self::Flux, true) => "The flux the wall holds, as authored.",
            (Self::Field, _) => "The field the wall pins.",
        }
    }
}

/// Wall seconds ahead of the queued steps that "Fire now" starts a pulse: the
/// frames in flight, so the pulse starts from zero rather than partway in.
const FIRE_LEAD_SECONDS: f64 = 0.25;

/// When "Fire now" starts a pulse, on the simulated clock, for an edit the
/// running generation takes as a live patch and for one that takes a new
/// generation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FireTimes {
    pub(super) live: f64,
    pub(super) handoff: f64,
}

impl Playground {
    /// Past every step already asked of the device and a quarter second of
    /// wall time more; an edit that takes a new generation waits as long
    /// again as the last handoff took. Paused, nothing is queued, a handoff
    /// lands before the next step, and a pulse starts where the run stands.
    pub(super) fn fire_times(&self) -> FireTimes {
        let now = self.simulated_time();
        if !self.wave_running {
            return FireTimes {
                live: now,
                handoff: now,
            };
        }
        let speed = self.editor.document.presentation.simulation_speed;
        let live =
            now + self.step_backlog as f64 * self.solver_time_step() + FIRE_LEAD_SECONDS * speed;
        let handoff = self
            .last_handoff
            .as_ref()
            .map_or(0.0, HandoffRecord::seconds);
        FireTimes {
            live,
            handoff: live + handoff * speed,
        }
    }
}

/// The first pulse a carrier at `frequency_hz` gets: a Hann burst three
/// cycles long, or half a second with no carrier, fired at `fire_at`.
fn burst(frequency_hz: f64, fire_at: f64) -> PulseTrain {
    let duration = if frequency_hz > 0.0 {
        3.0 / frequency_hz
    } else {
        0.5
    };
    PulseTrain {
        envelope: PulseEnvelope::FlatTop {
            duration,
            edge: 0.5 * duration,
        },
        start: fire_at,
        repeat: 0.0,
    }
}

/// A pulse made from a continuous signal: the same carrier, and for an
/// integrated use its rate amplitude, so the strength does not jump. A sine
/// counted from the pulse's centre has no area. The envelope is a Hann burst
/// three carrier cycles long, fired at `fire_at`.
pub(super) fn pulse_from(signal: TimeSignal, role: SignalUse, fire_at: f64) -> TimeSignal {
    let [offset, amplitude, frequency_hz, _] = signal.carrier();
    let omega = std::f64::consts::TAU * frequency_hz;
    let amplitude = if role.integrates() && omega > 0.0 {
        amplitude / omega
    } else {
        amplitude
    };
    let train = burst(frequency_hz, fire_at);
    TimeSignal::pulsed(
        [offset, amplitude, frequency_hz, 0.0],
        train.envelope,
        train.start,
        train.repeat,
    )
}

/// The continuous signal a pulse's carrier makes, back in the units a
/// continuous signal of this use has. A source starts on a cosine, as a new
/// one does.
pub(super) fn continuous_from(signal: TimeSignal, role: SignalUse) -> TimeSignal {
    let [offset, amplitude, frequency_hz, _] = signal.carrier();
    let omega = std::f64::consts::TAU * frequency_hz;
    let amplitude = if role.integrates() && omega > 0.0 {
        amplitude * omega
    } else {
        amplitude
    };
    let phase = if role == SignalUse::Source {
        SWITCH_ON_PHASE
    } else {
        0.0
    };
    TimeSignal::harmonic(offset, amplitude, frequency_hz, phase)
}

/// The shape window: whether it is open, which editor opened it, and what
/// that editor showed last. It follows that editor while the editor is drawn,
/// and keeps the last pulse once it is not.
#[derive(Default)]
pub(super) struct PulsePreview {
    pub(super) open: bool,
    editor: Option<egui::Id>,
    shown: Option<Previewed>,
}

/// What the shape window draws.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Previewed {
    Signal(TimeSignal, SignalUse),
    /// A material drive under its gate, with `None` for a drive whose
    /// numbers are formulas the window cannot evaluate on its own.
    Drive(Option<TimeDriveValues>, PulseTrain),
}

/// Edits `signal` in place. `fire` is when "Fire now" starts a pulse, and
/// `preview` the shape window a pulse's "Shape" opens.
pub(super) fn edit_time_signal(
    ui: &mut egui::Ui,
    signal: &mut TimeSignal,
    role: SignalUse,
    fire: FireTimes,
    preview: &mut PulsePreview,
) {
    let fire_at = if role.takes_handoff() {
        fire.handoff
    } else {
        fire.live
    };
    ui.push_id(("time-signal", role), |ui| {
        edit_time_signal_in(ui, signal, role, fire_at, preview);
    });
}

fn edit_time_signal_in(
    ui: &mut egui::Ui,
    signal: &mut TimeSignal,
    role: SignalUse,
    fire_at: f64,
    preview: &mut PulsePreview,
) {
    let editor = ui.id();
    let mut pulsed = signal.is_pulsed();
    ui.horizontal(|ui| {
        ui.selectable_value(&mut pulsed, false, "Continuous");
        ui.selectable_value(&mut pulsed, true, "Pulse");
    });
    if pulsed != signal.is_pulsed() {
        *signal = if pulsed {
            pulse_from(*signal, role, fire_at)
        } else {
            continuous_from(*signal, role)
        };
    }
    let (offset, amplitude, frequency, phase) = signal.carrier_mut();
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(offset).speed(0.02).prefix("Offset "));
        ui.add(
            egui::DragValue::new(amplitude)
                .speed(0.02)
                .prefix("Amplitude "),
        );
    });
    ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(frequency)
                .speed(0.05)
                .range(0.0..=1.0e6)
                .prefix("Hz "),
        );
        ui.add(egui::DragValue::new(phase).speed(0.05).prefix("Phase "));
    });
    if let TimeSignal::Pulsed {
        amplitude,
        frequency_hz,
        envelope,
        start,
        repeat,
        ..
    } = signal
    {
        let mut train = PulseTrain {
            envelope: *envelope,
            start: *start,
            repeat: *repeat,
        };
        let carrier = (*amplitude != 0.0).then_some(*frequency_hz);
        if edit_pulse_train(ui, &mut train, fire_at, carrier).shape {
            preview.open = true;
            preview.editor = Some(editor);
        }
        (*envelope, *start, *repeat) = (train.envelope, train.start, train.repeat);
    }
    ui.small(role.units(signal.is_pulsed()));
    if preview.editor == Some(editor) {
        preview.shown = Some(Previewed::Signal(*signal, role));
    }
}

/// What a press in the pulse controls asks of their owner.
#[derive(Clone, Copy, Default)]
struct PulseButtons {
    fired: bool,
    shape: bool,
}

/// A pulse's envelope, start and repeat, with "Fire now", which starts it at
/// `fire_at`, and "Shape". The summary counts the cycles of `carrier_hz`.
fn edit_pulse_train(
    ui: &mut egui::Ui,
    train: &mut PulseTrain,
    fire_at: f64,
    carrier_hz: Option<f64>,
) -> PulseButtons {
    edit_pulse_envelope(ui, &mut train.envelope);
    let duration = train.envelope.duration();
    let mut pressed = PulseButtons::default();
    ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(&mut train.start)
                .speed(0.02)
                .prefix("Start ")
                .suffix(" s"),
        );
        if ui.button("Fire now").clicked() {
            train.start = fire_at;
            pressed.fired = true;
        }
        pressed.shape = ui.button("Shape").clicked();
    });
    ui.horizontal(|ui| {
        let mut repeats = train.repeat > 0.0;
        if ui.checkbox(&mut repeats, "Repeat").changed() {
            train.repeat = if repeats { 2.0 * duration } else { 0.0 };
        }
        if repeats {
            ui.add(
                egui::DragValue::new(&mut train.repeat)
                    .speed(0.02)
                    .range(duration..=1.0e6)
                    .prefix("every ")
                    .suffix(" s"),
            );
        }
    });
    // A pulse lengthened past its repeat would overlap the next one.
    if train.repeat > 0.0 && train.repeat < duration {
        train.repeat = duration;
    }
    ui.small(pulse_summary(
        duration,
        train.start,
        train.repeat,
        carrier_hz,
    ));
    pressed
}

/// A material drive's timing: running throughout, or in pulses under one of
/// the envelopes. `drive` is the drive's numbers where they are constants,
/// for the cycle count and the shape window; a gate first set on a drive
/// is a Hann burst three of its cycles long, fired at `fire_at`. True when
/// "Fire now" was pressed, which its owner commits at once.
pub(super) fn edit_drive_gate(
    ui: &mut egui::Ui,
    drive: Option<TimeDriveValues>,
    gate: &mut Option<PulseTrain>,
    fire_at: f64,
    preview: &mut PulsePreview,
) -> bool {
    let editor = ui.id();
    let frequency = drive.map(TimeDriveValues::frequency_hz);
    let mut pulsed = gate.is_some();
    ui.horizontal(|ui| {
        ui.label("Timing");
        ui.selectable_value(&mut pulsed, false, "Continuous");
        ui.selectable_value(&mut pulsed, true, "Pulsed");
    });
    if pulsed != gate.is_some() {
        *gate = pulsed.then(|| burst(frequency.unwrap_or(0.0), fire_at));
    }
    let mut fired = false;
    if let Some(train) = gate {
        let pressed = edit_pulse_train(ui, train, fire_at, frequency);
        fired = pressed.fired;
        if pressed.shape {
            preview.open = true;
            preview.editor = Some(editor);
        }
        ui.small(
            "The drive's swing follows the envelope, its carrier counted from each pulse's \
             centre. Between pulses the coefficient is its base.",
        );
    }
    if preview.editor == Some(editor) {
        preview.shown = gate.map(|train| Previewed::Drive(drive, train));
    }
    fired
}

impl Playground {
    pub(super) fn pulse_shape_window(&mut self, ctx: &egui::Context) {
        if !self.pulse_preview.open {
            return;
        }
        let mut open = true;
        let shown = self.pulse_preview.shown;
        egui::Window::new("Pulse shape")
            .open(&mut open)
            .default_width(380.0)
            .show(ctx, |ui| match shown {
                Some(Previewed::Signal(signal, role)) => match signal.train() {
                    Some(train) => shape_plots(
                        ui,
                        train,
                        signal.frequency_ceiling_hz(),
                        role.pulse_quantity(),
                        |time| signal.value(time),
                        |time| signal.value(time),
                    ),
                    None => {
                        ui.label("The editor that opened this no longer shows a pulse.");
                    }
                },
                Some(Previewed::Drive(Some(drive), train)) => drive_shape(ui, drive, train),
                Some(Previewed::Drive(None, _)) => {
                    ui.label(
                        "This drive's numbers are formulas; the window draws a drive whose \
                         numbers are constants.",
                    );
                }
                None => {
                    ui.label("The editor that opened this no longer shows a pulse.");
                }
            });
        self.pulse_preview.open = open;
    }
}

/// Samples the spectrum takes a period of the highest frequency the pulse
/// holds, and the most it takes of one pulse.
const SPECTRUM_SAMPLES_PER_PERIOD: f64 = 16.0;
const MOST_SPECTRUM_SAMPLES: usize = 1 << 14;
const TRACE_POINTS: usize = 1024;

/// The spectrum of `value` over the first pulse of `train`, up to twice the
/// `ceiling` adaptation resolves, relative to its peak.
fn relative_spectrum(value: impl Fn(f64) -> f64, train: PulseTrain, ceiling: f64) -> Vec<[f64; 2]> {
    if ceiling <= 0.0 {
        return Vec::new();
    }
    let duration = train.envelope.duration();
    let count = ((duration * SPECTRUM_SAMPLES_PER_PERIOD * ceiling).ceil() as usize + 1)
        .clamp(2, MOST_SPECTRUM_SAMPLES);
    let interval = duration / (count - 1) as f64;
    let samples = (0..count)
        .map(|index| value(train.start + index as f64 * interval))
        .collect::<Vec<_>>();
    let Ok(spectrum) = transient_spectrum(&samples, interval) else {
        return Vec::new();
    };
    let peak = spectrum.magnitudes.iter().copied().fold(0.0, f64::max);
    (0..spectrum.magnitudes.len())
        .map(|index| [spectrum.frequency_hz(index), spectrum.magnitudes[index]])
        .take_while(|[frequency, _]| *frequency <= 2.0 * ceiling)
        .map(|[frequency, magnitude]| [frequency, magnitude / peak.max(f64::MIN_POSITIVE)])
        .collect()
}

/// A gated drive's coefficient factor over time, at the material frame's
/// origin, which is where a travelling modulation's is drawn.
fn gated_factor(drive: TimeDriveValues, train: PulseTrain) -> Option<impl Fn(f64) -> f64> {
    let origin = MaterialCoordinates {
        x: 0.0,
        y: 0.0,
        r: 0.0,
        theta: 0.0,
    };
    let runtime = TimeDriveRuntime::authored(drive).ok()?;
    Some(move |time: f64| {
        drive
            .gated_multiplier_and_rate(Some(train), time, origin, runtime)
            .map_or(f64::NAN, |(factor, _)| factor)
    })
}

/// A gated drive's coefficient factor over its pulses, and the spectrum of
/// its swing.
fn drive_shape(ui: &mut egui::Ui, drive: TimeDriveValues, train: PulseTrain) {
    let Some(factor) = gated_factor(drive, train) else {
        return;
    };
    shape_plots(
        ui,
        train,
        drive.frequency_hz() + train.envelope.bandwidth_hz(),
        "Coefficient factor, at the material frame's origin",
        &factor,
        |time| factor(time) - 1.0,
    );
}

/// One pulse, or two periods of a train, of `value` over time, and the
/// spectrum of `swing` over one pulse, relative to its peak, with the
/// frequency adaptation resolves marked.
fn shape_plots(
    ui: &mut egui::Ui,
    train: PulseTrain,
    ceiling: f64,
    quantity: &str,
    value: impl Fn(f64) -> f64,
    swing: impl Fn(f64) -> f64,
) {
    let PulseTrain {
        envelope,
        start,
        repeat,
    } = train;
    let duration = envelope.duration();
    let (from, to) = if repeat > 0.0 {
        (start, start + 2.0 * repeat)
    } else {
        (start - 0.1 * duration, start + 1.1 * duration)
    };
    let trace = (0..TRACE_POINTS)
        .map(|index| {
            let time = from + (to - from) * index as f64 / (TRACE_POINTS - 1) as f64;
            [time, value(time)]
        })
        .collect::<Vec<_>>();
    ui.small(quantity);
    line_plot(ui, &trace, SELECT, "s", &[], 110.0, "No pulse");
    let points = relative_spectrum(swing, train, ceiling);
    ui.small("Spectrum, relative to its peak");
    line_plot(
        ui,
        &points,
        TEAL,
        "Hz",
        &[PlotMarker {
            x: ceiling,
            label: format!("{ceiling:.2} Hz"),
        }],
        110.0,
        "No spectrum",
    );
    ui.small("Adaptation sizes the mesh for waves up to the marked frequency.");
}

/// How long the pulse lasts, when it peaks, and how many carrier cycles it
/// holds, for the numbers the envelope's own parameters do not show.
fn pulse_summary(duration: f64, start: f64, repeat: f64, carrier_hz: Option<f64>) -> String {
    let mut summary = format!(
        "Lasts {duration:.3} s and peaks at {:.3} s",
        start + 0.5 * duration
    );
    if let Some(hz) = carrier_hz.filter(|hz| *hz > 0.0) {
        summary += &format!(", {:.1} cycles", hz * duration);
    }
    if repeat > 0.0 {
        summary += &format!(", every {repeat:.3} s");
    }
    summary
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EnvelopeShape {
    FlatTop,
    Gaussian,
    Sinc,
}

impl EnvelopeShape {
    const ALL: [Self; 3] = [Self::FlatTop, Self::Gaussian, Self::Sinc];

    fn of(envelope: PulseEnvelope) -> Self {
        match envelope {
            PulseEnvelope::FlatTop { .. } => Self::FlatTop,
            PulseEnvelope::Gaussian { .. } => Self::Gaussian,
            PulseEnvelope::Sinc { .. } => Self::Sinc,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::FlatTop => "Flat top",
            Self::Gaussian => "Gaussian",
            Self::Sinc => "Sinc",
        }
    }

    /// This shape over the same duration: a Hann burst, a Gaussian cut at
    /// its ends, or a three-lobe sinc.
    fn lasting(self, duration: f64) -> PulseEnvelope {
        match self {
            Self::FlatTop => PulseEnvelope::FlatTop {
                duration,
                edge: 0.5 * duration,
            },
            Self::Gaussian => PulseEnvelope::Gaussian {
                width: duration / (2.0 * GAUSSIAN_CUT_WIDTHS),
            },
            Self::Sinc => PulseEnvelope::Sinc {
                bandwidth_hz: 3.0 / duration,
                lobes: 3,
            },
        }
    }
}

/// The smallest duration, edge, width or reciprocal bandwidth the editor
/// offers, well above any step a scene runs at being a concern of the mesh
/// rather than of the envelope.
const SHORTEST_PULSE_SECONDS: f64 = 1.0e-3;

fn edit_pulse_envelope(ui: &mut egui::Ui, envelope: &mut PulseEnvelope) {
    let mut shape = EnvelopeShape::of(*envelope);
    // Wrapped: a menu and two fields are wider than a phone's panel.
    ui.horizontal_wrapped(|ui| {
        egui::ComboBox::from_id_salt(ui.id().with("pulse-envelope"))
            .selected_text(shape.label())
            .show_ui(ui, |ui| {
                for choice in EnvelopeShape::ALL {
                    ui.selectable_value(&mut shape, choice, choice.label());
                }
            });
        if shape != EnvelopeShape::of(*envelope) {
            *envelope = shape.lasting(envelope.duration());
        }
        match envelope {
            PulseEnvelope::FlatTop { duration, edge } => {
                ui.add(
                    egui::DragValue::new(duration)
                        .speed(0.01)
                        .range(SHORTEST_PULSE_SECONDS..=1.0e4)
                        .prefix("Lasts ")
                        .suffix(" s"),
                );
                let longest = 0.5 * *duration;
                ui.add(
                    egui::DragValue::new(edge)
                        .speed(0.005)
                        .range(0.5 * SHORTEST_PULSE_SECONDS..=longest)
                        .prefix("Edge ")
                        .suffix(" s"),
                );
                *edge = edge.min(longest);
            }
            PulseEnvelope::Gaussian { width } => {
                ui.add(
                    egui::DragValue::new(width)
                        .speed(0.002)
                        .range(SHORTEST_PULSE_SECONDS..=1.0e3)
                        .prefix("Width ")
                        .suffix(" s"),
                );
            }
            PulseEnvelope::Sinc {
                bandwidth_hz,
                lobes,
            } => {
                ui.add(
                    egui::DragValue::new(bandwidth_hz)
                        .speed(0.05)
                        .range(1.0e-3..=1.0 / SHORTEST_PULSE_SECONDS)
                        .prefix("Band ")
                        .suffix(" Hz"),
                );
                ui.add(
                    egui::DragValue::new(lobes)
                        .range(1..=MAX_SINC_LOBES)
                        .prefix("Lobes "),
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source turned into a pulse keeps the rate it drove, so its strength
    /// does not jump, and turned back keeps its acceleration. A pinned wall's
    /// field is the same number either way.
    #[test]
    fn a_waveform_change_keeps_what_the_consumer_imposes() {
        let continuous = TimeSignal::harmonic(0.0, 6.0, 3.0, SWITCH_ON_PHASE);
        let omega = std::f64::consts::TAU * 3.0;
        let pulse = pulse_from(continuous, SignalUse::Source, 4.5);
        let TimeSignal::Pulsed {
            envelope,
            start,
            repeat,
            ..
        } = pulse
        else {
            panic!("not a pulse: {pulse:?}");
        };
        assert_eq!(pulse.carrier(), [0.0, 6.0 / omega, 3.0, 0.0]);
        assert_eq!(
            envelope,
            PulseEnvelope::FlatTop {
                duration: 1.0,
                edge: 0.5
            }
        );
        assert_eq!((start, repeat), (4.5, 0.0));
        assert!(pulse.valid());
        let back = continuous_from(pulse, SignalUse::Source);
        assert!((back.carrier()[1] - 6.0).abs() < 1.0e-12);
        assert_eq!(back.carrier()[3], SWITCH_ON_PHASE);

        let wall = TimeSignal::harmonic(0.2, 0.5, 2.0, 0.3);
        let pinned = pulse_from(wall, SignalUse::Field, 1.0);
        assert_eq!(pinned.carrier(), [0.2, 0.5, 2.0, 0.0]);
        assert_eq!(
            continuous_from(pinned, SignalUse::Field),
            TimeSignal::harmonic(0.2, 0.5, 2.0, 0.0)
        );
        // No carrier: a flash of the offset, half a second long.
        let flash = pulse_from(
            TimeSignal::harmonic(0.4, 0.0, 0.0, 0.0),
            SignalUse::Flux,
            0.0,
        );
        assert!(flash.valid());
        assert_eq!(flash.carrier()[0], 0.4);
    }

    /// Every shape converts into every other over the same duration, and each
    /// result is a valid envelope.
    #[test]
    fn a_shape_change_keeps_the_duration() {
        for from in EnvelopeShape::ALL {
            for to in EnvelopeShape::ALL {
                let envelope = to.lasting(from.lasting(0.8).duration());
                assert!(envelope.valid());
                assert!((envelope.duration() - 0.8).abs() < 1.0e-12);
                assert!(EnvelopeShape::of(envelope) == to);
            }
        }
    }

    /// One pass of the editor over `signal`, with `events`.
    fn editor_pass(
        context: &egui::Context,
        signal: &mut TimeSignal,
        preview: &mut PulsePreview,
        events: Vec<egui::Event>,
    ) -> Vec<LaidOut> {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(480.0, 640.0),
            )),
            events,
            ..egui::RawInput::default()
        };
        let output = context.run_ui(input, |ui| {
            // A source's edit patches the running generation, so it fires
            // at the live time.
            let fire = FireTimes {
                live: 12.5,
                handoff: 13.0,
            };
            edit_time_signal(ui, signal, SignalUse::Source, fire, preview);
        });
        laid_out(&output)
    }

    /// "Pulse" turns a continuous source into a pulse fired at the time it
    /// is handed, and "Fire now" moves an existing pulse's start there.
    #[test]
    fn the_editor_turns_a_source_into_a_pulse_and_fires_it() {
        let context = egui::Context::default();
        theme::apply(&context);
        context.enable_accesskit();
        let mut signal = TimeSignal::harmonic(0.0, 6.0, 3.0, SWITCH_ON_PHASE);
        let mut preview = PulsePreview::default();
        let press = |context: &egui::Context,
                     signal: &mut TimeSignal,
                     preview: &mut PulsePreview,
                     label: &str| {
            let widgets = editor_pass(context, signal, preview, vec![]);
            let widget = widgets
                .iter()
                .find(|widget| widget.label == label)
                .unwrap_or_else(|| panic!("no {label} among {:?}", widgets));
            let events = click(widget);
            editor_pass(context, signal, preview, events);
        };
        press(&context, &mut signal, &mut preview, "Pulse");
        assert!(signal.is_pulsed(), "{signal:?}");
        assert!(matches!(signal, TimeSignal::Pulsed { start: 12.5, .. }));
        if let TimeSignal::Pulsed { start, .. } = &mut signal {
            *start = 1.0;
        }
        press(&context, &mut signal, &mut preview, "Fire now");
        assert!(matches!(signal, TimeSignal::Pulsed { start: 12.5, .. }));
        // "Shape" opens the window on this pulse, which then follows it.
        assert!(!preview.open && preview.shown.is_none());
        press(&context, &mut signal, &mut preview, "Shape");
        assert!(preview.open);
        assert_eq!(
            preview.shown,
            Some(Previewed::Signal(signal, SignalUse::Source))
        );
        press(&context, &mut signal, &mut preview, "Continuous");
        assert!(!signal.is_pulsed());
        assert!((signal.carrier()[1] - 6.0).abs() < 1.0e-12);
        assert_eq!(
            preview.shown,
            Some(Previewed::Signal(signal, SignalUse::Source))
        );
    }

    /// One pass of the gate editor over `gate`, with `events`.
    fn gate_pass(
        context: &egui::Context,
        drive: TimeDriveValues,
        gate: &mut Option<PulseTrain>,
        preview: &mut PulsePreview,
        events: Vec<egui::Event>,
    ) -> (Vec<LaidOut>, bool) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(480.0, 640.0),
            )),
            events,
            ..egui::RawInput::default()
        };
        let mut fired = false;
        let output = context.run_ui(input, |ui| {
            fired = edit_drive_gate(ui, Some(drive), gate, 7.25, preview);
        });
        (laid_out(&output), fired)
    }

    /// "Pulsed" gates a drive with a burst three of its cycles long fired at
    /// the time it is handed, "Fire now" moves the start there, "Shape"
    /// shows the drive under its gate, and "Continuous" removes the gate.
    #[test]
    fn the_gate_editor_pulses_a_drive_and_fires_it() {
        let context = egui::Context::default();
        theme::apply(&context);
        context.enable_accesskit();
        let drive = TimeDriveValues::ParametricPump {
            depth: 0.2,
            frequency_hz: 2.0,
            phase_radians: 0.0,
        };
        let mut gate = None;
        let mut preview = PulsePreview::default();
        let press = |context: &egui::Context,
                     gate: &mut Option<PulseTrain>,
                     preview: &mut PulsePreview,
                     label: &str| {
            let (widgets, _) = gate_pass(context, drive, gate, preview, vec![]);
            let widget = widgets
                .iter()
                .find(|widget| widget.label == label)
                .unwrap_or_else(|| panic!("no {label} among {:?}", widgets));
            let events = click(widget);
            gate_pass(context, drive, gate, preview, events).1
        };
        assert!(!press(&context, &mut gate, &mut preview, "Pulsed"));
        assert_eq!(
            gate,
            Some(PulseTrain {
                envelope: PulseEnvelope::FlatTop {
                    duration: 1.5,
                    edge: 0.75,
                },
                start: 7.25,
                repeat: 0.0,
            })
        );
        gate.as_mut().unwrap().start = 1.0;
        // "Fire now" tells the panel, which commits the drive at once.
        assert!(press(&context, &mut gate, &mut preview, "Fire now"));
        assert_eq!(gate.unwrap().start, 7.25);
        press(&context, &mut gate, &mut preview, "Shape");
        assert!(preview.open);
        assert_eq!(
            preview.shown,
            Some(Previewed::Drive(Some(drive), gate.unwrap()))
        );
        press(&context, &mut gate, &mut preview, "Continuous");
        assert_eq!(gate, None);
        assert_eq!(preview.shown, None);
    }

    /// Paused, a pulse fires where the run stands. Running, it starts past
    /// the steps already queued and a quarter second of wall time on, at the
    /// speed the run goes, and an edit that takes a new generation later
    /// again by as long as the last handoff took.
    #[test]
    fn fire_now_starts_past_the_queued_steps_and_the_handoff_while_running() {
        let mut state = Playground {
            sim_time_offset: 3.0,
            sim_time_step: 1.0e-3,
            completed_steps: 500,
            uploaded_time_step: 2.0e-3,
            step_backlog: 40,
            wave_running: false,
            last_handoff: Some(HandoffRecord {
                prepare_ms: 250.0,
                pack_ms: 60.0,
                drain_ms: 30.0,
                upload_ms: 60.0,
                timing: TopologyPreparationTiming::default(),
                action: TopologyMeshUpdateAction::Reuse,
                operator_reused: false,
                adapted: false,
                transferred: true,
                exact_nodes: 0,
                fresh: false,
                degrees_of_freedom: 0,
                triangles: 0,
                carve: None,
                repair_fallback: None,
            }),
            ..Playground::default()
        };
        assert_eq!(
            state.fire_times(),
            FireTimes {
                live: 3.5,
                handoff: 3.5
            }
        );
        state.wave_running = true;
        state.editor.document.presentation.simulation_speed = 2.0;
        let fire = state.fire_times();
        let live = 3.5 + 40.0 * 2.0e-3 + 0.5;
        assert!((fire.live - live).abs() < 1.0e-12);
        assert!((fire.handoff - (live + 0.8)).abs() < 1.0e-12);
    }

    /// The shape window's spectrum of a pulsed signal, or nothing for one
    /// that is not a pulse.
    fn spectrum_of(signal: TimeSignal) -> Vec<[f64; 2]> {
        signal.train().map_or_else(Vec::new, |train| {
            relative_spectrum(
                |time| signal.value(time),
                train,
                signal.frequency_ceiling_hz(),
            )
        })
    }

    /// A gated pump's swing peaks at its carrier. At 0 Hz, a temporal slab,
    /// it peaks at zero, and between pulses the factor is exactly one.
    #[test]
    fn the_shape_window_draws_a_gated_drives_swing() {
        let train = burst(2.0, 0.5);
        for (frequency_hz, expected) in [(2.0, 2.0), (0.0, 0.0)] {
            let drive = TimeDriveValues::ParametricPump {
                depth: 0.3,
                frequency_hz,
                phase_radians: 0.0,
            };
            let factor = gated_factor(drive, train).unwrap();
            assert_eq!(factor(0.2), 1.0);
            assert_eq!(factor(2.5), 1.0);
            let points = relative_spectrum(
                |time| factor(time) - 1.0,
                train,
                frequency_hz + train.envelope.bandwidth_hz(),
            );
            let peak = points.iter().max_by(|a, b| a[1].total_cmp(&b[1])).unwrap()[0];
            assert!(
                (peak - expected).abs() < 0.1,
                "{frequency_hz} Hz peaks at {peak}"
            );
        }
    }

    /// A burst peaks at its carrier, a sinc is flat across its band and quiet
    /// past it, and a flash, having no carrier, peaks at 0 Hz.
    #[test]
    fn the_shape_window_draws_each_pulse_its_own_spectrum() {
        let at = |points: &[[f64; 2]], frequency: f64| {
            points
                .iter()
                .min_by(|a, b| {
                    (a[0] - frequency)
                        .abs()
                        .total_cmp(&(b[0] - frequency).abs())
                })
                .unwrap()[1]
        };
        let peak =
            |points: &[[f64; 2]]| points.iter().max_by(|a, b| a[1].total_cmp(&b[1])).unwrap()[0];
        let burst = TimeSignal::pulsed(
            [0.0, 1.0, 3.0, 0.0],
            PulseEnvelope::FlatTop {
                duration: 4.0,
                edge: 2.0,
            },
            1.0,
            0.0,
        );
        assert!((peak(&spectrum_of(burst)) - 3.0).abs() < 0.05);
        let sinc = TimeSignal::pulsed(
            [0.0, 1.0, 5.0, 0.0],
            PulseEnvelope::Sinc {
                bandwidth_hz: 2.0,
                lobes: 8,
            },
            0.0,
            0.0,
        );
        let points = spectrum_of(sinc);
        for frequency in [3.8, 4.5, 5.0, 5.5, 6.2] {
            assert!(at(&points, frequency) > 0.85, "{frequency} Hz");
        }
        assert!(at(&points, 8.5) < 0.05);
        let flash = TimeSignal::pulsed(
            [1.0, 0.0, 0.0, 0.0],
            PulseEnvelope::Gaussian { width: 0.05 },
            0.0,
            0.0,
        );
        assert_eq!(peak(&spectrum_of(flash)), 0.0);
        assert!(spectrum_of(TimeSignal::ZERO).is_empty());
    }

    #[test]
    fn the_summary_names_duration_peak_cycles_and_repeat() {
        assert_eq!(
            pulse_summary(1.0, 2.0, 0.0, Some(3.0)),
            "Lasts 1.000 s and peaks at 2.500 s, 3.0 cycles"
        );
        assert_eq!(
            pulse_summary(0.4, 0.0, 1.5, None),
            "Lasts 0.400 s and peaks at 0.200 s, every 1.500 s"
        );
    }
}
