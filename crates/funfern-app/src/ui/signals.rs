//! The signal editor every source and driven wall shares: a continuous
//! harmonic, or a pulse under one of the core's envelopes, and the window
//! that shows a pulse's shape and spectrum.
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

    /// What a pulse here imposes, for the shape window's trace.
    const fn pulse_quantity(self) -> &'static str {
        match self {
            Self::Source => "Field rate",
            Self::Flux => "Flux",
            Self::Field => "Field",
        }
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

/// Wall seconds ahead of the queued steps that "Fire now" starts a pulse:
/// the frames in flight, and the handoff a wall's edit takes, so the pulse
/// starts from zero rather than partway in.
const FIRE_LEAD_SECONDS: f64 = 0.25;

impl Playground {
    /// When a pulse fired now starts, on the simulated clock: past every step
    /// already asked of the device, and a quarter second of wall time more.
    /// Paused, nothing is queued and the pulse starts where the run stands.
    pub(super) fn pulse_fire_time(&self) -> f64 {
        let now = self.simulated_time();
        if !self.wave_running {
            return now;
        }
        now + self.step_backlog as f64 * self.solver_time_step()
            + FIRE_LEAD_SECONDS * self.editor.document.presentation.simulation_speed
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
    let duration = if frequency_hz > 0.0 {
        3.0 / frequency_hz
    } else {
        0.5
    };
    TimeSignal::pulsed(
        [offset, amplitude, frequency_hz, 0.0],
        PulseEnvelope::FlatTop {
            duration,
            edge: 0.5 * duration,
        },
        fire_at,
        0.0,
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
    signal: Option<(TimeSignal, SignalUse)>,
}

/// Edits `signal` in place. `fire_at` is when "Fire now" starts a pulse, and
/// `preview` the shape window a pulse's "Shape" opens.
pub(super) fn edit_time_signal(
    ui: &mut egui::Ui,
    signal: &mut TimeSignal,
    role: SignalUse,
    fire_at: f64,
    preview: &mut PulsePreview,
) {
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
        edit_pulse_envelope(ui, envelope);
        let duration = envelope.duration();
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(start)
                    .speed(0.02)
                    .prefix("Start ")
                    .suffix(" s"),
            );
            if ui.button("Fire now").clicked() {
                *start = fire_at;
            }
            if ui.button("Shape").clicked() {
                preview.open = true;
                preview.editor = Some(editor);
            }
        });
        ui.horizontal(|ui| {
            let mut repeats = *repeat > 0.0;
            if ui.checkbox(&mut repeats, "Repeat").changed() {
                *repeat = if repeats { 2.0 * duration } else { 0.0 };
            }
            if repeats {
                ui.add(
                    egui::DragValue::new(repeat)
                        .speed(0.02)
                        .range(duration..=1.0e6)
                        .prefix("every ")
                        .suffix(" s"),
                );
            }
        });
        // A pulse lengthened past its repeat would overlap the next one.
        if *repeat > 0.0 && *repeat < duration {
            *repeat = duration;
        }
        ui.small(pulse_summary(
            duration,
            *start,
            *repeat,
            (*amplitude != 0.0).then_some(*frequency_hz),
        ));
    }
    ui.small(role.units(signal.is_pulsed()));
    if preview.editor == Some(editor) {
        preview.signal = Some((*signal, role));
    }
}

impl Playground {
    pub(super) fn pulse_shape_window(&mut self, ctx: &egui::Context) {
        if !self.pulse_preview.open {
            return;
        }
        let mut open = true;
        let shown = self.pulse_preview.signal;
        egui::Window::new("Pulse shape")
            .open(&mut open)
            .default_width(380.0)
            .show(ctx, |ui| match shown {
                Some((signal, role)) if signal.is_pulsed() => pulse_shape(ui, signal, role),
                _ => {
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

/// One pulse's spectrum up to twice the frequency adaptation resolves,
/// relative to its peak.
fn relative_spectrum(signal: TimeSignal) -> Vec<[f64; 2]> {
    let TimeSignal::Pulsed {
        envelope, start, ..
    } = signal
    else {
        return Vec::new();
    };
    let ceiling = signal.frequency_ceiling_hz();
    if ceiling <= 0.0 {
        return Vec::new();
    }
    let duration = envelope.duration();
    let count = ((duration * SPECTRUM_SAMPLES_PER_PERIOD * ceiling).ceil() as usize + 1)
        .clamp(2, MOST_SPECTRUM_SAMPLES);
    let interval = duration / (count - 1) as f64;
    let samples = (0..count)
        .map(|index| signal.value(start + index as f64 * interval))
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

/// One pulse, or two periods of a train, over time, and one pulse's
/// spectrum relative to its peak with the frequency adaptation resolves
/// marked.
fn pulse_shape(ui: &mut egui::Ui, signal: TimeSignal, role: SignalUse) {
    let TimeSignal::Pulsed {
        envelope,
        start,
        repeat,
        ..
    } = signal
    else {
        return;
    };
    let duration = envelope.duration();
    let (from, to) = if repeat > 0.0 {
        (start, start + 2.0 * repeat)
    } else {
        (start - 0.1 * duration, start + 1.1 * duration)
    };
    let trace = (0..TRACE_POINTS)
        .map(|index| {
            let time = from + (to - from) * index as f64 / (TRACE_POINTS - 1) as f64;
            [time, signal.value(time)]
        })
        .collect::<Vec<_>>();
    ui.small(role.pulse_quantity());
    line_plot(ui, &trace, SELECT, "s", &[], 110.0, "No pulse");
    let ceiling = signal.frequency_ceiling_hz();
    let points = relative_spectrum(signal);
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
            edit_time_signal(ui, signal, SignalUse::Source, 12.5, preview);
        });
        laid_out(&output)
    }

    fn click(widget: &LaidOut) -> Vec<egui::Event> {
        let at = widget.rect.center();
        [true, false]
            .map(|pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            })
            .into_iter()
            .chain([egui::Event::PointerMoved(at)])
            .collect()
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
        assert!(!preview.open && preview.signal.is_none());
        press(&context, &mut signal, &mut preview, "Shape");
        assert!(preview.open);
        assert_eq!(preview.signal, Some((signal, SignalUse::Source)));
        press(&context, &mut signal, &mut preview, "Continuous");
        assert!(!signal.is_pulsed());
        assert!((signal.carrier()[1] - 6.0).abs() < 1.0e-12);
        assert_eq!(preview.signal, Some((signal, SignalUse::Source)));
    }

    /// Paused, a pulse fires where the run stands. Running, it starts past
    /// the steps already queued and a quarter second of wall time on, at the
    /// speed the run goes.
    #[test]
    fn fire_now_starts_past_the_queued_steps_while_running() {
        let mut state = Playground {
            sim_time_offset: 3.0,
            sim_time_step: 1.0e-3,
            completed_steps: 500,
            uploaded_time_step: 2.0e-3,
            step_backlog: 40,
            wave_running: false,
            ..Playground::default()
        };
        assert_eq!(state.pulse_fire_time(), 3.5);
        state.wave_running = true;
        state.editor.document.presentation.simulation_speed = 2.0;
        assert!((state.pulse_fire_time() - (3.5 + 40.0 * 2.0e-3 + 0.5)).abs() < 1.0e-12);
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
        assert!((peak(&relative_spectrum(burst)) - 3.0).abs() < 0.05);
        let sinc = TimeSignal::pulsed(
            [0.0, 1.0, 5.0, 0.0],
            PulseEnvelope::Sinc {
                bandwidth_hz: 2.0,
                lobes: 8,
            },
            0.0,
            0.0,
        );
        let points = relative_spectrum(sinc);
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
        assert_eq!(peak(&relative_spectrum(flash)), 0.0);
        assert!(relative_spectrum(TimeSignal::ZERO).is_empty());
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
