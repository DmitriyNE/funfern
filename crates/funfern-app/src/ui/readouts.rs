//! The floating readouts: probe plots and profiles, the curve waterfalls,
//! and the far-field direction/time views.

use crate::wave_gpu::{
    AreaProbeRecord, CurveProbeRecord, FAR_FIELD_DIRECTIONS, FarFieldRecord, PointProbeRecord,
};
use bevy::prelude::*;
use bevy_egui::egui::{self, Color32, Rect, Stroke};
use funfern_app::topology_editor::TopologyProbeTarget;
use funfern_core::*;

use super::*;

impl Playground {
    /// One pannable, zoomable time trace. Every trace in a readout shares the
    /// window held by `view`, so they stay aligned while the user navigates.
    pub(super) fn probe_plot(
        ui: &mut egui::Ui,
        label: &str,
        samples: &[PointProbeRecord],
        value: impl Fn(&PointProbeRecord) -> f64,
        color: Color32,
        view: &mut ProbeViewState,
        maximum_span: f64,
    ) {
        ui.small(label);
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().max(120.0), 92.0),
            egui::Sense::drag(),
        );
        ui.painter()
            .rect_filled(rect, 2.0, Color32::from_rgb(12, 18, 24));
        ui.painter().rect_stroke(
            rect,
            2.0,
            Stroke::new(1.0, Color32::from_rgb(55, 69, 80)),
            egui::StrokeKind::Inside,
        );
        let waiting = |painter: &egui::Painter| {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Waiting for samples",
                egui::FontId::monospace(11.0),
                Color32::from_rgb(112, 130, 143),
            );
        };
        let Some((mut minimum_time, mut maximum_time)) = Self::probe_time_window(samples, view)
        else {
            waiting(ui.painter());
            return;
        };
        let visible_span = maximum_time - minimum_time;
        if response.drag_started() {
            view.live = false;
        }
        if response.dragged() {
            let delta = response.drag_delta().x as f64;
            view.end_time -= delta / rect.width() as f64 * visible_span;
        }
        if response.hovered() {
            let wheel = ui.ctx().input(|input| input.smooth_scroll_delta.y);
            if wheel != 0.0 {
                let fraction = response.hover_pos().map_or(0.5, |position| {
                    ((position.x - rect.left()) / rect.width()).clamp(0.0, 1.0)
                }) as f64;
                let anchored_time = minimum_time + fraction * visible_span;
                view.live = false;
                view.readout.span =
                    (visible_span * (-wheel as f64 * 0.01).exp()).clamp(0.02, maximum_span);
                let first = samples.first().unwrap().time;
                let last = samples.last().unwrap().time;
                let zoomed_span = view.readout.span.min(last - first);
                let fullest_span = maximum_span.min(last - first);
                if view.readout.span >= fullest_span * (1.0 - 1.0e-9) {
                    view.live = true;
                    view.end_time = last;
                } else {
                    view.end_time = anchored_time + (1.0 - fraction) * zoomed_span;
                }
            }
        }
        (minimum_time, maximum_time) = Self::probe_time_window(samples, view).unwrap();
        let visible = samples
            .iter()
            .filter(|sample| sample.time >= minimum_time && sample.time <= maximum_time)
            .collect::<Vec<_>>();
        if visible.len() < 2 {
            waiting(ui.painter());
            return;
        }
        let mut minimum = f64::INFINITY;
        let mut maximum = f64::NEG_INFINITY;
        for sample in &visible {
            let sample = value(sample);
            minimum = minimum.min(sample);
            maximum = maximum.max(sample);
        }
        if !minimum.is_finite() || !maximum.is_finite() {
            return;
        }
        if (maximum - minimum).abs() < 1.0e-15 {
            let padding = maximum.abs().max(1.0) * 0.05;
            minimum -= padding;
            maximum += padding;
        }
        let time_span = (maximum_time - minimum_time).max(f64::MIN_POSITIVE);
        let value_span = maximum - minimum;
        let points = visible
            .iter()
            .map(|sample| {
                egui::pos2(
                    egui::lerp(
                        rect.left()..=rect.right(),
                        ((sample.time - minimum_time) / time_span) as f32,
                    ),
                    egui::lerp(
                        rect.bottom()..=rect.top(),
                        ((value(sample) - minimum) / value_span) as f32,
                    ),
                )
            })
            .collect();
        ui.painter()
            .add(egui::Shape::line(points, Stroke::new(1.4, color)));
        ui.painter().text(
            rect.left_top() + egui::vec2(4.0, 3.0),
            egui::Align2::LEFT_TOP,
            format!("{maximum:+.3e}"),
            egui::FontId::monospace(9.0),
            Color32::from_rgb(142, 161, 175),
        );
        ui.painter().text(
            rect.left_bottom() + egui::vec2(4.0, -3.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{minimum:+.3e}"),
            egui::FontId::monospace(9.0),
            Color32::from_rgb(142, 161, 175),
        );
        response.on_hover_text(format!(
            "Simulation time {minimum_time:.4}–{maximum_time:.4}"
        ));
    }
    pub(super) fn curve_probe_values(
        frame: &CurveProbeRecord,
        quantity: LineProbeQuantity,
    ) -> &[f32] {
        match quantity {
            LineProbeQuantity::Field => &frame.displacement,
            LineProbeQuantity::Transverse => &frame.transverse_magnitude,
            LineProbeQuantity::Flux | LineProbeQuantity::MeanFlux => &frame.normal_flux,
            LineProbeQuantity::Energy | LineProbeQuantity::MeanEnergy => &frame.energy_density,
        }
    }
    /// A causal trailing mean of the recorded normal flux and energy density:
    /// every frame carries the average of the `window` seconds ending at its
    /// own time, sample point by sample point. The instantaneous flux of a
    /// standing wave swings symmetrically about zero at twice the driven
    /// frequency, so only this average says how much power a path actually
    /// carries; the energy density swings with it, and its average says how
    /// much a wave holds along the path, which is what shows it gaining or
    /// losing power over the distance.
    ///
    /// The window is fixed rather than taken from the visible one, so panning
    /// and zooming move over the same numbers instead of rewriting them. A
    /// frame whose window the record does not cover in full yields a row of
    /// NaN, which keeps the series one row per recorded frame: every
    /// representation then holds the raw series' time alignment, and the
    /// renderers already skip what is not finite. The second return is how much
    /// of the window the newest frame holds, for the readout to report while it
    /// is still filling.
    /// The longest trailing window the trace can ever cover.
    ///
    /// The ring holds a fixed number of frames, so how much time it spans
    /// depends on the interval between them - and that is not the preset's
    /// nominal rate. The recorder strides the solver's own steps,
    /// `round(1 / (rate * dt))` of them, so a coarse enough time step rounds
    /// the stride down and oversamples: at the default mesh a 120 Hz preset
    /// records every step, about 149 Hz, and 512 frames reach back 3.4 seconds
    /// where the nominal rate promises 4.3.
    ///
    /// The interval is measured as the smallest gap in the trace. A dropped
    /// readback inflates an average and would put the limit back out of reach,
    /// while the smallest gap is still the true stride; if the time step
    /// changed inside the ring it takes the shorter of the two, which errs
    /// short. One interval is held back so the newest frame's window begins at
    /// or before a frame the ring holds rather than exactly on one.
    pub(super) fn curve_mean_window_limit(frames: &[CurveProbeRecord], nominal: f64) -> f64 {
        let interval = frames
            .windows(2)
            .map(|pair| pair[1].time - pair[0].time)
            .filter(|gap| gap.is_finite() && *gap > 0.0)
            .fold(f64::INFINITY, f64::min);
        let interval = if interval.is_finite() {
            interval
        } else {
            nominal
        };
        interval * (CURVE_TRACE_FRAMES - 2) as f64
    }
    pub(super) fn curve_probe_running_mean(
        frames: &[CurveProbeRecord],
        window: f64,
    ) -> (Vec<CurveProbeRecord>, f64) {
        if frames.is_empty() || !window.is_finite() || window <= 0.0 {
            return (Vec::new(), 0.0);
        }
        let mut means = Vec::with_capacity(frames.len());
        let mut flux = RunningRow::default();
        let mut energy = RunningRow::default();
        // The first frame of the run of equal-width records the accumulator was
        // built for, and the oldest frame still inside the window.
        let mut run = 0;
        let mut oldest = 0;
        let mut filled = 0.0;
        for (index, frame) in frames.iter().enumerate() {
            if frame.normal_flux.len() != flux.sums.len()
                || frame.energy_density.len() != energy.sums.len()
            {
                // A sampling-preset change or a boundary remesh leaves rows of
                // another length in the same trace, and two layouts have no
                // common average. The window starts again here.
                flux = RunningRow::sized(frame.normal_flux.len());
                energy = RunningRow::sized(frame.energy_density.len());
                run = index;
                oldest = index;
            }
            flux.add(&frame.normal_flux, 1.0);
            energy.add(&frame.energy_density, 1.0);
            let begin = frame.time - window;
            while oldest < index && frames[oldest].time < begin {
                flux.add(&frames[oldest].normal_flux, -1.0);
                energy.add(&frames[oldest].energy_density, -1.0);
                oldest += 1;
            }
            // Either a frame has already left the window, or the run itself
            // reaches back past its start. Anything else is a partial average
            // of whatever happens to be recorded, which is not what the row
            // claims to show.
            let covered = oldest > run || frames[run].time <= begin;
            filled = if covered {
                1.0
            } else {
                ((frame.time - frames[run].time) / window).clamp(0.0, 1.0)
            };
            means.push(CurveProbeRecord {
                probe_id: frame.probe_id,
                time: frame.time,
                normal_flux: flux.mean(covered),
                energy_density: energy.mean(covered),
                ..Default::default()
            });
        }
        (means, filled)
    }
    /// Trapezoidal integral along the sampled path, plus the fraction of the
    /// intervals that carried finite values.
    pub(super) fn curve_probe_integral(
        frame: &CurveProbeRecord,
        length: f64,
        quantity: LineProbeQuantity,
        closed: bool,
    ) -> (f64, f64) {
        let samples = Self::curve_probe_values(frame, quantity);
        let intervals = if closed {
            samples.len()
        } else {
            samples.len().saturating_sub(1)
        };
        if intervals == 0 || !length.is_finite() {
            return (f64::NAN, 0.0);
        }
        let mut integral = 0.0;
        let mut valid = 0usize;
        for index in 0..intervals {
            let a = samples[index] as f64;
            let b = samples[(index + 1) % samples.len()] as f64;
            if a.is_finite() && b.is_finite() {
                integral += 0.5 * (a + b);
                valid += 1;
            }
        }
        if valid == 0 {
            return (f64::NAN, 0.0);
        }
        (
            integral * length / intervals as f64,
            valid as f64 / intervals as f64,
        )
    }
    pub(super) fn curve_probe_profile(
        ui: &mut egui::Ui,
        frames: &[CurveProbeRecord],
        view: &ProbeViewState,
        quantity: LineProbeQuantity,
        length: f64,
        physics: PhysicsModel,
    ) {
        ui.small(format!("{} vs arclength", quantity.label_for(physics)));
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().max(120.0), 110.0),
            egui::Sense::hover(),
        );
        ui.painter()
            .rect_filled(rect, 2.0, Color32::from_rgb(12, 18, 24));
        let waiting = |painter: &egui::Painter, message: &str| {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                message,
                egui::FontId::monospace(11.0),
                Color32::from_rgb(112, 130, 143),
            );
        };
        let Some(frame) = frames.iter().min_by(|a, b| {
            (a.time - view.end_time)
                .abs()
                .total_cmp(&(b.time - view.end_time).abs())
        }) else {
            waiting(ui.painter(), "Waiting for samples");
            return;
        };
        let samples = Self::curve_probe_values(frame, quantity);
        let mut minimum = samples
            .iter()
            .copied()
            .filter(|value| value.is_finite())
            .fold(f32::INFINITY, f32::min);
        let mut maximum = samples
            .iter()
            .copied()
            .filter(|value| value.is_finite())
            .fold(f32::NEG_INFINITY, f32::max);
        if !minimum.is_finite() || !maximum.is_finite() {
            // Every sample at this time is invalid: outside the domain, on a
            // two-trace boundary, or - for the averaged row - earlier than the
            // first full window.
            waiting(ui.painter(), "No valid samples at this time");
            return;
        }
        if (maximum - minimum).abs() < 1.0e-12 {
            let padding = maximum.abs().max(1.0) * 0.05;
            minimum -= padding;
            maximum += padding;
        }
        if minimum <= 0.0 && maximum >= 0.0 {
            let zero = egui::lerp(
                rect.bottom()..=rect.top(),
                (0.0 - minimum) / (maximum - minimum),
            );
            ui.painter().line_segment(
                [
                    egui::pos2(rect.left(), zero),
                    egui::pos2(rect.right(), zero),
                ],
                Stroke::new(1.0, Color32::from_rgb(45, 57, 67)),
            );
        }
        let count = samples.len().max(2);
        let mut run = Vec::new();
        for (index, value) in samples.iter().copied().enumerate() {
            if value.is_finite() {
                run.push(egui::pos2(
                    egui::lerp(
                        rect.left()..=rect.right(),
                        index as f32 / (count - 1) as f32,
                    ),
                    egui::lerp(
                        rect.bottom()..=rect.top(),
                        (value - minimum) / (maximum - minimum),
                    ),
                ));
            } else if run.len() >= 2 {
                ui.painter().add(egui::Shape::line(
                    std::mem::take(&mut run),
                    Stroke::new(1.5, quantity.color()),
                ));
            } else {
                run.clear();
            }
        }
        if run.len() >= 2 {
            ui.painter()
                .add(egui::Shape::line(run, Stroke::new(1.5, quantity.color())));
        }
        ui.painter().text(
            rect.left_top() + egui::vec2(4.0, 3.0),
            egui::Align2::LEFT_TOP,
            format!("{maximum:+.3e}"),
            egui::FontId::monospace(9.0),
            Color32::from_rgb(142, 161, 175),
        );
        ui.painter().text(
            rect.left_bottom() + egui::vec2(4.0, -3.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{minimum:+.3e} · s=0"),
            egui::FontId::monospace(9.0),
            Color32::from_rgb(142, 161, 175),
        );
        ui.painter().text(
            rect.right_bottom() + egui::vec2(-4.0, -3.0),
            egui::Align2::RIGHT_BOTTOM,
            format!("s={length:.3}"),
            egui::FontId::monospace(9.0),
            Color32::from_rgb(142, 161, 175),
        );
    }
    pub(super) fn curve_probe_waterfall(
        ui: &mut egui::Ui,
        frames: &[CurveProbeRecord],
        times: &[PointProbeRecord],
        view: &mut ProbeViewState,
        maximum_span: f64,
        quantity: LineProbeQuantity,
        physics: PhysicsModel,
    ) {
        ui.small(format!("{} waterfall", quantity.label_for(physics)));
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().max(120.0), 170.0),
            egui::Sense::drag(),
        );
        ui.painter()
            .rect_filled(rect, 2.0, Color32::from_rgb(12, 18, 24));
        let Some((minimum_time, maximum_time)) = Self::probe_time_window(times, view) else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Waiting for samples",
                egui::FontId::monospace(11.0),
                Color32::from_rgb(112, 130, 143),
            );
            return;
        };
        let span = maximum_time - minimum_time;
        if response.drag_started() {
            view.live = false;
        }
        if response.dragged() {
            view.end_time += response.drag_delta().y as f64 / rect.height() as f64 * span;
        }
        if response.hovered() {
            let wheel = ui.ctx().input(|input| input.smooth_scroll_delta.y);
            if wheel != 0.0 {
                view.live = false;
                view.readout.span = (span * (-wheel as f64 * 0.01).exp()).clamp(0.02, maximum_span);
            }
        }
        let Some((minimum_time, maximum_time)) = Self::probe_time_window(times, view) else {
            return;
        };
        let visible = frames
            .iter()
            .filter(|frame| frame.time >= minimum_time && frame.time <= maximum_time)
            .collect::<Vec<_>>();
        let maximum = visible
            .iter()
            .flat_map(|frame| Self::curve_probe_values(frame, quantity).iter())
            .copied()
            .filter(|value| value.is_finite())
            .map(f32::abs)
            .fold(0.0, f32::max)
            .max(1.0e-12)
            / view.readout.waterfall_gain;
        for (row, frame) in visible.iter().enumerate() {
            let samples = Self::curve_probe_values(frame, quantity);
            let count = samples.len();
            if count == 0 {
                continue;
            }
            let top = egui::lerp(
                rect.bottom()..=rect.top(),
                (row + 1) as f32 / visible.len() as f32,
            );
            let bottom = egui::lerp(
                rect.bottom()..=rect.top(),
                row as f32 / visible.len() as f32,
            );
            for (column, value) in samples.iter().copied().enumerate() {
                if !value.is_finite() {
                    continue;
                }
                let normalized = (value / maximum).clamp(-1.0, 1.0);
                let color = waterfall_color(normalized, quantity == LineProbeQuantity::Energy);
                let left = egui::lerp(rect.left()..=rect.right(), column as f32 / count as f32);
                let right = egui::lerp(
                    rect.left()..=rect.right(),
                    (column + 1) as f32 / count as f32,
                );
                ui.painter().rect_filled(
                    Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, bottom)),
                    0.0,
                    color,
                );
            }
        }
        ui.painter().text(
            rect.left_top() + egui::vec2(4.0, 3.0),
            egui::Align2::LEFT_TOP,
            format!("t={maximum_time:.3}"),
            egui::FontId::monospace(9.0),
            Color32::WHITE,
        );
        ui.painter().text(
            rect.left_bottom() + egui::vec2(4.0, -3.0),
            egui::Align2::LEFT_BOTTOM,
            format!("t={minimum_time:.3} · s=0"),
            egui::FontId::monospace(9.0),
            Color32::WHITE,
        );
        ui.painter().text(
            rect.right_bottom() + egui::vec2(-4.0, -3.0),
            egui::Align2::RIGHT_BOTTOM,
            "s=L",
            egui::FontId::monospace(9.0),
            Color32::WHITE,
        );
    }
    pub(super) fn far_field_waterfall(
        ui: &mut egui::Ui,
        frames: &[FarFieldRecord],
        times: &[PointProbeRecord],
        view: &mut ProbeViewState,
        maximum_span: f64,
    ) {
        ui.small("Direction × time");
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().max(120.0), 170.0),
            egui::Sense::drag(),
        );
        ui.painter()
            .rect_filled(rect, 2.0, Color32::from_rgb(12, 18, 24));
        let Some((minimum_time, maximum_time)) = Self::probe_time_window(times, view) else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Warming up the propagation delay",
                egui::FontId::monospace(11.0),
                Color32::from_rgb(112, 130, 143),
            );
            return;
        };
        let span = maximum_time - minimum_time;
        if response.drag_started() {
            view.live = false;
        }
        if response.dragged() {
            view.end_time += response.drag_delta().y as f64 / rect.height() as f64 * span;
        }
        if response.hovered() {
            let wheel = ui.ctx().input(|input| input.smooth_scroll_delta.y);
            if wheel != 0.0 {
                view.live = false;
                view.readout.span = (span * (-wheel as f64 * 0.01).exp()).clamp(0.02, maximum_span);
                if view.readout.span >= maximum_span * (1.0 - 1.0e-9) {
                    view.live = true;
                }
            }
        }
        let Some((minimum_time, maximum_time)) = Self::probe_time_window(times, view) else {
            return;
        };
        let visible = frames
            .iter()
            .filter(|frame| frame.time >= minimum_time && frame.time <= maximum_time)
            .collect::<Vec<_>>();
        let maximum = visible
            .iter()
            .flat_map(|frame| frame.amplitude.iter())
            .copied()
            .filter(|value| value.is_finite())
            .map(f32::abs)
            .fold(0.0, f32::max)
            .max(1.0e-12)
            / view.readout.waterfall_gain;
        for (row, frame) in visible.iter().enumerate() {
            let count = frame.amplitude.len();
            if count == 0 {
                continue;
            }
            let top = egui::lerp(
                rect.bottom()..=rect.top(),
                (row + 1) as f32 / visible.len() as f32,
            );
            let bottom = egui::lerp(
                rect.bottom()..=rect.top(),
                row as f32 / visible.len() as f32,
            );
            for (column, value) in frame.amplitude.iter().copied().enumerate() {
                if !value.is_finite() {
                    continue;
                }
                let color = waterfall_color((value / maximum).clamp(-1.0, 1.0), false);
                let left = egui::lerp(rect.left()..=rect.right(), column as f32 / count as f32);
                let right = egui::lerp(
                    rect.left()..=rect.right(),
                    (column + 1) as f32 / count as f32,
                );
                ui.painter().rect_filled(
                    Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, bottom)),
                    0.0,
                    color,
                );
            }
        }
        ui.painter().text(
            rect.left_top() + egui::vec2(4.0, 3.0),
            egui::Align2::LEFT_TOP,
            format!("t={maximum_time:.3}"),
            egui::FontId::monospace(9.0),
            Color32::WHITE,
        );
        ui.painter().text(
            rect.left_bottom() + egui::vec2(4.0, -3.0),
            egui::Align2::LEFT_BOTTOM,
            format!("t={minimum_time:.3} · 0°"),
            egui::FontId::monospace(9.0),
            Color32::WHITE,
        );
        ui.painter().text(
            rect.right_bottom() + egui::vec2(-4.0, -3.0),
            egui::Align2::RIGHT_BOTTOM,
            "360°",
            egui::FontId::monospace(9.0),
            Color32::WHITE,
        );
    }
    /// Mean intensity over exactly the window the other far-field traces show.
    pub(super) fn far_field_average(
        frames: &[FarFieldRecord],
        times: &[PointProbeRecord],
        view: &ProbeViewState,
    ) -> Vec<f32> {
        let mut window = view.clone();
        let Some((minimum_time, maximum_time)) = Self::probe_time_window(times, &mut window) else {
            return vec![];
        };
        let mut average = vec![0.0_f32; FAR_FIELD_DIRECTIONS];
        let mut count = 0_u32;
        for frame in frames
            .iter()
            .filter(|frame| frame.time >= minimum_time && frame.time <= maximum_time)
        {
            if frame.intensity.len() != FAR_FIELD_DIRECTIONS {
                continue;
            }
            for (sum, value) in average.iter_mut().zip(&frame.intensity) {
                *sum += *value;
            }
            count += 1;
        }
        if count > 0 {
            for value in &mut average {
                *value /= count as f32;
            }
            average
        } else {
            vec![]
        }
    }
    pub(super) fn far_field_polar(ui: &mut egui::Ui, label: &str, intensity: &[f32]) {
        ui.small(label);
        let size = ui.available_width().max(100.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
        let center = rect.center();
        let radius = 0.44 * size;
        ui.painter()
            .rect_filled(rect, 2.0, Color32::from_rgb(12, 18, 24));
        for fraction in [0.25, 0.5, 0.75, 1.0] {
            ui.painter().circle_stroke(
                center,
                radius * fraction,
                Stroke::new(0.7, Color32::from_rgb(55, 69, 80)),
            );
        }
        ui.painter().line_segment(
            [
                egui::pos2(center.x - radius, center.y),
                egui::pos2(center.x + radius, center.y),
            ],
            Stroke::new(0.7, Color32::from_rgb(55, 69, 80)),
        );
        ui.painter().line_segment(
            [
                egui::pos2(center.x, center.y - radius),
                egui::pos2(center.x, center.y + radius),
            ],
            Stroke::new(0.7, Color32::from_rgb(55, 69, 80)),
        );
        let maximum = intensity
            .iter()
            .copied()
            .filter(|value| value.is_finite())
            .fold(0.0_f32, f32::max);
        if maximum <= 1.0e-30 {
            ui.painter().text(
                center,
                egui::Align2::CENTER_CENTER,
                "Waiting for signal",
                egui::FontId::monospace(10.0),
                Color32::from_rgb(112, 130, 143),
            );
            return;
        }
        let mut points = intensity
            .iter()
            .copied()
            .enumerate()
            .map(|(index, value)| {
                let db = 10.0 * (value.max(1.0e-30) / maximum).log10();
                let radial = (1.0 + db / 40.0).clamp(0.0, 1.0);
                let angle = std::f32::consts::TAU * index as f32 / FAR_FIELD_DIRECTIONS as f32;
                center + egui::vec2(angle.cos(), -angle.sin()) * radius * radial
            })
            .collect::<Vec<_>>();
        if let Some(first) = points.first().copied() {
            points.push(first);
        }
        if points.len() > 2 {
            ui.painter()
                .add(egui::Shape::line(points, Stroke::new(2.0, TEAL)));
        }
        for (offset, align, label) in [
            (egui::vec2(radius, 0.0), egui::Align2::RIGHT_BOTTOM, "0°"),
            (egui::vec2(0.0, -radius), egui::Align2::LEFT_TOP, "90°"),
            (egui::vec2(-radius, 0.0), egui::Align2::LEFT_BOTTOM, "180°"),
            (egui::vec2(0.0, radius), egui::Align2::LEFT_BOTTOM, "270°"),
        ] {
            ui.painter().text(
                center + offset,
                align,
                label,
                egui::FontId::monospace(9.0),
                Color32::from_rgb(142, 161, 175),
            );
        }
    }
    pub(super) fn probe_windows(&mut self, ctx: &egui::Context) {
        let physics = self.editor.document.model.accepted.physics;
        let history = self.probe_history_seconds;
        let ids = self.probe_windows.iter().copied().collect::<Vec<_>>();
        for id in ids {
            let Some(probe) = self
                .editor
                .document
                .model
                .probes
                .iter()
                .find(|probe| probe.id == id)
                .cloned()
            else {
                self.probe_windows.remove(&id);
                continue;
            };
            let point_samples = self
                .probe_traces
                .get(&id)
                .map(|trace| trace.samples.iter().copied().collect::<Vec<_>>())
                .unwrap_or_default();
            let curve_frames = self
                .curve_probe_traces
                .get(&id)
                .map(|trace| trace.records.iter().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            let area_samples = self
                .area_probe_traces
                .get(&id)
                .map(|trace| trace.records.iter().copied().collect::<Vec<_>>())
                .unwrap_or_default();
            let is_curve = matches!(
                probe.target,
                TopologyProbeTarget::Segment { .. } | TopologyProbeTarget::Boundary(_)
            );
            let is_area = matches!(
                probe.target,
                TopologyProbeTarget::AreaDisk { .. } | TopologyProbeTarget::AreaRegion(_)
            );
            let (length, closed) = self.probe_metrics.get(&id).copied().unwrap_or((0.0, false));
            // The averaged flux row can only look back as far as the trace
            // reaches, which at the faster presets is well short of the history
            // slider. Offering more than that would be a setting that shows
            // nothing however long the run continues.
            let mean_limit = match &probe.target {
                TopologyProbeTarget::Segment { preset, .. } => Some(*preset),
                TopologyProbeTarget::Boundary(target) => Some(target.preset),
                _ => None,
            }
            .map_or(history, |preset| {
                history.min(Self::curve_mean_window_limit(
                    &curve_frames,
                    1.0 / preset.sample_rate(),
                ))
            })
            .max(0.2);
            let curve_times = curve_frames
                .iter()
                .map(|frame| PointProbeRecord {
                    probe_id: frame.probe_id,
                    time: frame.time,
                    ..Default::default()
                })
                .collect::<Vec<_>>();
            let status = self.probe_status.get(&id).cloned();
            // The document holds what the readout shows; the view only where
            // its window sits.
            let stored = self.editor.document.readouts.probe(id);
            let mut view = self
                .probe_views
                .remove(&id)
                .unwrap_or_else(|| ProbeViewState::new(stored));
            view.readout = stored;
            let newest_time = point_samples
                .last()
                .map(|sample| sample.time)
                .or_else(|| curve_frames.last().map(|frame| frame.time))
                .or_else(|| area_samples.last().map(|sample| sample.time));
            if view.live
                && let Some(time) = newest_time
            {
                view.end_time = time;
            }
            view.readout.span = view.readout.span.clamp(0.02, history);
            view.readout.mean_window = view.readout.mean_window.clamp(0.05, mean_limit);
            let shown = view.readout;
            let kind = match probe.target {
                TopologyProbeTarget::Point(_) => "point probe",
                TopologyProbeTarget::Segment { .. } => "line probe",
                TopologyProbeTarget::Boundary(_) => "boundary probe",
                TopologyProbeTarget::AreaDisk { .. } | TopologyProbeTarget::AreaRegion(_) => {
                    "area probe"
                }
            };
            let mut open = true;
            let mut clear = false;
            egui::Window::new(format!("{} · {kind}", probe.name))
                .id(egui::Id::new(("probe_readout", id.0)))
                .open(&mut open)
                .default_width(430.0)
                .resizable(true)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .add(egui::Button::new("Live").selected(view.live))
                            .on_hover_text("Follow the newest sample")
                            .clicked()
                        {
                            view.live = true;
                            if let Some(time) = newest_time {
                                view.end_time = time;
                            }
                        }
                        if is_curve {
                            let active = LineProbeQuantity::ALL
                                .into_iter()
                                .filter(|quantity| quantity.applies(physics))
                                .flat_map(|quantity| {
                                    LineProbeRepresentation::ALL.into_iter().map(
                                        move |representation| {
                                            quantity.offset() + representation.offset()
                                        },
                                    )
                                })
                                .filter(|index| view.readout.line_plots[*index])
                                .count();
                            egui::containers::menu::MenuButton::new(format!("Plots ({active})"))
                                .config(
                                    egui::containers::menu::MenuConfig::new().close_behavior(
                                        egui::PopupCloseBehavior::CloseOnClickOutside,
                                    ),
                                )
                                .ui(ui, |ui| {
                                    egui::Grid::new(("line_probe_plots", id.0))
                                        .num_columns(4)
                                        .spacing(egui::vec2(12.0, 4.0))
                                        .show(ui, |ui| {
                                            ui.label("");
                                            for representation in LineProbeRepresentation::ALL {
                                                ui.small(representation.label());
                                            }
                                            ui.end_row();
                                            for quantity in LineProbeQuantity::ALL {
                                                if !quantity.applies(physics) {
                                                    continue;
                                                }
                                                ui.label(quantity.label_for(physics));
                                                for representation in LineProbeRepresentation::ALL {
                                                    let index =
                                                        quantity.offset() + representation.offset();
                                                    ui.checkbox(
                                                        &mut view.readout.line_plots[index],
                                                        "",
                                                    )
                                                    .on_hover_text(format!(
                                                        "{} {}",
                                                        quantity.label_for(physics),
                                                        representation.label()
                                                    ));
                                                }
                                                ui.end_row();
                                            }
                                        });
                                    ui.separator();
                                    ui.add(
                                        egui::Slider::new(
                                            &mut view.readout.waterfall_gain,
                                            0.1..=10.0,
                                        )
                                        .logarithmic(true)
                                        .text("Waterfall gain"),
                                    );
                                    ui.add(
                                        egui::Slider::new(
                                            &mut view.readout.mean_window,
                                            0.05..=mean_limit,
                                        )
                                        .logarithmic(true)
                                        .text("Mean window"),
                                    )
                                    .on_hover_text(
                                        "Seconds the averaged flux and energy rows look back \
                                         over. Cover several periods of the flux, which swings \
                                         at twice the driven frequency. It stops at what the \
                                         recorded trace can reach back over.",
                                    );
                                });
                        } else if is_area {
                            let active = [
                                view.readout.area_mean_field,
                                view.readout.area_rms_field,
                                view.readout.area_rms_transverse,
                                view.readout.area_mean_energy,
                                view.readout.area_total_energy,
                            ]
                            .into_iter()
                            .filter(|enabled| *enabled)
                            .count();
                            egui::containers::menu::MenuButton::new(format!("Plots ({active})"))
                                .config(
                                    egui::containers::menu::MenuConfig::new().close_behavior(
                                        egui::PopupCloseBehavior::CloseOnClickOutside,
                                    ),
                                )
                                .ui(ui, |ui| {
                                    ui.checkbox(
                                        &mut view.readout.area_mean_field,
                                        format!("Mean {}", primary_field_label(physics)),
                                    );
                                    ui.checkbox(
                                        &mut view.readout.area_rms_field,
                                        format!("RMS {}", primary_field_label(physics)),
                                    );
                                    ui.checkbox(
                                        &mut view.readout.area_rms_transverse,
                                        format!(
                                            "RMS {}",
                                            transverse_field_magnitude_label(physics)
                                        ),
                                    );
                                    ui.checkbox(
                                        &mut view.readout.area_mean_energy,
                                        "Mean energy density",
                                    );
                                    ui.checkbox(
                                        &mut view.readout.area_total_energy,
                                        total_energy_label(physics),
                                    );
                                });
                        } else {
                            let active = [
                                view.readout.field,
                                view.readout.secondary_field,
                                view.readout.transverse_field,
                                view.readout.poynting,
                                view.readout.energy,
                            ]
                            .into_iter()
                            .filter(|enabled| *enabled)
                            .count();
                            egui::containers::menu::MenuButton::new(format!("Plots ({active})"))
                                .config(
                                    egui::containers::menu::MenuConfig::new().close_behavior(
                                        egui::PopupCloseBehavior::CloseOnClickOutside,
                                    ),
                                )
                                .ui(ui, |ui| {
                                    ui.checkbox(
                                        &mut view.readout.field,
                                        primary_field_label(physics),
                                    );
                                    ui.checkbox(
                                        &mut view.readout.secondary_field,
                                        primary_field_rate_label(physics),
                                    );
                                    ui.checkbox(
                                        &mut view.readout.transverse_field,
                                        transverse_field_magnitude_label(physics),
                                    );
                                    ui.checkbox(
                                        &mut view.readout.poynting,
                                        energy_flow_magnitude_label(physics),
                                    );
                                    ui.checkbox(&mut view.readout.energy, "Energy density");
                                });
                        }
                        if ui.small_button("Clear").clicked() {
                            clear = true;
                        }
                    });
                    if let Some(status) = &status {
                        ui.colored_label(GOLD, status);
                    }
                    if is_curve {
                        if let Some(frame) = curve_frames.last() {
                            let (_, coverage) = Self::curve_probe_integral(
                                frame,
                                length,
                                LineProbeQuantity::Field,
                                closed,
                            );
                            if coverage < 0.999 {
                                ui.small(format!("Valid coverage {:.0}%", coverage * 100.0));
                            }
                        }
                        // One pass over the trace serves every averaged view,
                        // and none of them asks for it unless drawn.
                        let averaged = LineProbeQuantity::ALL
                            .into_iter()
                            .filter(|quantity| quantity.averaged())
                            .any(|quantity| {
                                LineProbeRepresentation::ALL
                                    .into_iter()
                                    .any(|representation| {
                                        view.readout.line_plots
                                            [quantity.offset() + representation.offset()]
                                    })
                            });
                        let (mean_frames, mean_filled) = if averaged {
                            Self::curve_probe_running_mean(&curve_frames, view.readout.mean_window)
                        } else {
                            (Vec::new(), 1.0)
                        };
                        if averaged && !curve_frames.is_empty() && mean_filled < 0.999 {
                            ui.colored_label(
                                GOLD,
                                format!(
                                    "Filling the {:.2} s mean window · {:.0}%",
                                    view.readout.mean_window,
                                    mean_filled * 100.0
                                ),
                            );
                        }
                        for quantity in LineProbeQuantity::ALL {
                            if !quantity.applies(physics) {
                                continue;
                            }
                            let frames = if quantity.averaged() {
                                &mean_frames
                            } else {
                                &curve_frames
                            };
                            if view.readout.line_plots
                                [quantity.offset() + LineProbeRepresentation::Arclength.offset()]
                            {
                                Self::curve_probe_profile(
                                    ui, frames, &view, quantity, length, physics,
                                );
                            }
                            if view.readout.line_plots
                                [quantity.offset() + LineProbeRepresentation::Waterfall.offset()]
                            {
                                Self::curve_probe_waterfall(
                                    ui,
                                    frames,
                                    &curve_times,
                                    &mut view,
                                    history,
                                    quantity,
                                    physics,
                                );
                            }
                            if view.readout.line_plots
                                [quantity.offset() + LineProbeRepresentation::Integral.offset()]
                            {
                                // A frame the averaging window does not cover
                                // integrates to NaN, and so does one whose
                                // samples are all invalid. Leaving those out
                                // shortens the trace rather than breaking it.
                                let integral = frames
                                    .iter()
                                    .map(|frame| PointProbeRecord {
                                        probe_id: frame.probe_id,
                                        time: frame.time,
                                        displacement: Self::curve_probe_integral(
                                            frame, length, quantity, closed,
                                        )
                                        .0,
                                        ..Default::default()
                                    })
                                    .filter(|sample| sample.displacement.is_finite())
                                    .collect::<Vec<_>>();
                                Self::probe_plot(
                                    ui,
                                    &format!("{} ∫ ds", quantity.label_for(physics)),
                                    &integral,
                                    |sample| sample.displacement,
                                    quantity.color(),
                                    &mut view,
                                    history,
                                );
                            }
                        }
                    } else if is_area {
                        if let Some(sample) = area_samples.last() {
                            ui.small(format!(
                                "area {:.4} · covered {:.0}%",
                                sample.covered_area,
                                sample.coverage * 100.0
                            ));
                        }
                        let history_of = |value: fn(&AreaProbeRecord) -> f64| {
                            area_samples
                                .iter()
                                .map(|sample| PointProbeRecord {
                                    probe_id: sample.probe_id,
                                    time: sample.time,
                                    displacement: value(sample),
                                    ..Default::default()
                                })
                                .collect::<Vec<_>>()
                        };
                        for (enabled, label, values, color) in [
                            (
                                view.readout.area_mean_field,
                                format!("Mean {}", primary_field_label(physics)),
                                history_of(|s| s.mean_displacement),
                                SELECT,
                            ),
                            (
                                view.readout.area_rms_field,
                                format!("RMS {}", primary_field_label(physics)),
                                history_of(|s| s.rms_displacement),
                                TEAL,
                            ),
                            (
                                view.readout.area_rms_transverse,
                                format!("RMS {}", transverse_field_magnitude_label(physics)),
                                history_of(|s| s.rms_transverse_magnitude),
                                Color32::from_rgb(188, 139, 255),
                            ),
                            (
                                view.readout.area_mean_energy,
                                "Mean energy density".to_owned(),
                                history_of(|s| s.mean_energy_density),
                                GOLD,
                            ),
                            (
                                view.readout.area_total_energy,
                                total_energy_label(physics).to_owned(),
                                history_of(|s| s.total_energy),
                                RED,
                            ),
                        ] {
                            if enabled {
                                Self::probe_plot(
                                    ui,
                                    &label,
                                    &values,
                                    |sample| sample.displacement,
                                    color,
                                    &mut view,
                                    history,
                                );
                            }
                        }
                    } else {
                        if view.readout.field {
                            Self::probe_plot(
                                ui,
                                primary_field_label(physics),
                                &point_samples,
                                |sample| sample.displacement,
                                SELECT,
                                &mut view,
                                history,
                            );
                        }
                        if view.readout.secondary_field {
                            Self::probe_plot(
                                ui,
                                primary_field_rate_label(physics),
                                &point_samples,
                                |sample| sample.velocity,
                                TEAL,
                                &mut view,
                                history,
                            );
                        }
                        if view.readout.transverse_field {
                            Self::probe_plot(
                                ui,
                                transverse_field_magnitude_label(physics),
                                &point_samples,
                                |sample| sample.transverse_magnitude,
                                Color32::from_rgb(188, 139, 255),
                                &mut view,
                                history,
                            );
                        }
                        if view.readout.poynting {
                            Self::probe_plot(
                                ui,
                                energy_flow_magnitude_label(physics),
                                &point_samples,
                                |sample| sample.poynting_magnitude,
                                RED,
                                &mut view,
                                history,
                            );
                        }
                        if view.readout.energy {
                            Self::probe_plot(
                                ui,
                                "Local energy density",
                                &point_samples,
                                |sample| sample.energy_density,
                                GOLD,
                                &mut view,
                                history,
                            );
                        }
                    }
                    ui.small(if is_curve {
                        "Drag traces horizontally or waterfalls vertically · wheel to zoom"
                    } else {
                        "Drag right for earlier time · wheel to zoom"
                    });
                });
            if clear {
                self.clear_probe_trace(id);
            }
            if !open {
                self.probe_windows.remove(&id);
            }
            if let Some(edited) = edited_readout(stored, shown, view.readout) {
                self.editor.document.readouts.set_probe(id, edited);
            }
            self.probe_views.insert(id, view);
        }
        self.far_field_readout_window(ctx);
    }
    pub(super) fn far_field_readout_window(&mut self, ctx: &egui::Context) {
        if !self.far_field_window {
            return;
        }
        let history = self.probe_history_seconds;
        let frames = self
            .far_field_trace
            .records
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let times = frames
            .iter()
            .map(|frame| PointProbeRecord {
                time: frame.time,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let power = frames
            .iter()
            .map(|frame| PointProbeRecord {
                time: frame.time,
                displacement: frame
                    .intensity
                    .iter()
                    .map(|value| *value as f64)
                    .sum::<f64>()
                    * std::f64::consts::TAU
                    / FAR_FIELD_DIRECTIONS as f64,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let newest_time = frames.last().map(|frame| frame.time);
        let stored = self.editor.document.readouts.far_field;
        self.far_field_view.readout = stored;
        if self.far_field_view.live
            && let Some(time) = newest_time
        {
            self.far_field_view.end_time = time;
        }
        self.far_field_view.readout.span = self.far_field_view.readout.span.clamp(0.02, history);
        let shown = self.far_field_view.readout;
        let status = self
            .runtime
            .active()
            .and_then(|active| active.far_field.as_ref())
            .and_then(|result| result.as_ref().err().cloned());
        let recording = self.far_field_recording();
        let mut open = true;
        let mut clear = false;
        egui::Window::new("Outer-domain far field")
            .id(egui::Id::new("far_field_readout"))
            .open(&mut open)
            .default_width(470.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Button::new("Live").selected(self.far_field_view.live))
                        .clicked()
                    {
                        self.far_field_view.live = true;
                        if let Some(time) = newest_time {
                            self.far_field_view.end_time = time;
                        }
                    }
                    let active = [
                        self.far_field_view.readout.far_waterfall,
                        self.far_field_view.readout.far_polar,
                        self.far_field_view.readout.far_power,
                    ]
                    .into_iter()
                    .filter(|enabled| *enabled)
                    .count();
                    egui::containers::menu::MenuButton::new(format!("Plots ({active})"))
                        .config(
                            egui::containers::menu::MenuConfig::new()
                                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
                        )
                        .ui(ui, |ui| {
                            ui.checkbox(
                                &mut self.far_field_view.readout.far_waterfall,
                                "Waterfall",
                            );
                            ui.checkbox(
                                &mut self.far_field_view.readout.far_polar,
                                "Polar patterns",
                            );
                            ui.checkbox(
                                &mut self.far_field_view.readout.far_power,
                                "Radiated power",
                            )
                            .on_hover_text(
                                "Directional intensity integrated over observation angle",
                            );
                        });
                    if self.far_field_view.readout.far_waterfall {
                        ui.add(
                            egui::Slider::new(
                                &mut self.far_field_view.readout.waterfall_gain,
                                0.2..=5.0,
                            )
                            .logarithmic(true)
                            .text("gain"),
                        );
                    }
                    if ui.small_button("Clear").clicked() {
                        clear = true;
                    }
                });
                if let Some(status) = &status {
                    ui.colored_label(RED, status);
                    return;
                }
                // A projection needs the whole delay window before it can report
                // anything, so an empty plot here is a recorder still filling
                // rather than a silence in the field.
                if let Some(recorded) = recording {
                    ui.colored_label(
                        GOLD,
                        format!("Recording the delay window · {:.0}%", recorded * 100.0),
                    );
                }
                if self.far_field_view.readout.far_waterfall {
                    Self::far_field_waterfall(
                        ui,
                        &frames,
                        &times,
                        &mut self.far_field_view,
                        history,
                    );
                }
                if self.far_field_view.readout.far_polar {
                    let instantaneous = frames
                        .iter()
                        .min_by(|a, b| {
                            (a.time - self.far_field_view.end_time)
                                .abs()
                                .total_cmp(&(b.time - self.far_field_view.end_time).abs())
                        })
                        .map(|frame| frame.intensity.clone())
                        .unwrap_or_default();
                    let averaged = Self::far_field_average(&frames, &times, &self.far_field_view);
                    ui.small("Relative radiation pattern · 40 dB");
                    ui.columns(2, |columns| {
                        Self::far_field_polar(&mut columns[0], "Instantaneous", &instantaneous);
                        Self::far_field_polar(&mut columns[1], "Time-averaged", &averaged);
                    });
                }
                if self.far_field_view.readout.far_power {
                    Self::probe_plot(
                        ui,
                        "Radiated power",
                        &power,
                        |sample| sample.displacement,
                        GOLD,
                        &mut self.far_field_view,
                        history,
                    );
                }
                ui.small(
                    "Drag through time · wheel to zoom · the average follows the visible window",
                );
            });
        if clear {
            self.far_field_trace = FarFieldTrace::default();
        }
        if let Some(edited) = edited_readout(stored, shown, self.far_field_view.readout) {
            self.editor.document.readouts.far_field = edited;
        }
        self.far_field_window = open;
    }
}

/// The sums and counts of one recorded row over a trailing window, point by
/// point; a non-finite sample counts for nothing.
#[derive(Default)]
struct RunningRow {
    sums: Vec<f64>,
    counts: Vec<u32>,
}

impl RunningRow {
    fn sized(length: usize) -> Self {
        Self {
            sums: vec![0.0; length],
            counts: vec![0; length],
        }
    }

    /// Adds a row (`sign` 1) or takes one out (`sign` −1).
    fn add(&mut self, row: &[f32], sign: f64) {
        for (point, value) in row.iter().enumerate() {
            if value.is_finite() {
                self.sums[point] += sign * *value as f64;
                if sign > 0.0 {
                    self.counts[point] += 1;
                } else {
                    self.counts[point] -= 1;
                }
            }
        }
    }

    /// The mean at every point, or NaN where the window is not yet covered
    /// or held nothing finite.
    fn mean(&self, covered: bool) -> Vec<f32> {
        self.counts
            .iter()
            .zip(&self.sums)
            .map(|(count, sum)| {
                if covered && *count > 0 {
                    (sum / *count as f64) as f32
                } else {
                    f32::NAN
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wave_gpu::CurveProbeRecord;

    /// Early in a run a readout shows its span and mean window clamped to what
    /// has been recorded. Those clamps are the display's: only what the user
    /// changes reaches the document, so a look does not shorten a window for
    /// good.
    #[test]
    fn only_what_the_user_changes_reaches_the_document() {
        use funfern_app::document::ProbeReadout;
        let stored = ProbeReadout {
            span: 4.0,
            ..ProbeReadout::default()
        };
        let shown = ProbeReadout {
            span: 0.5,
            mean_window: 0.3,
            ..stored
        };
        assert_eq!(edited_readout(stored, shown, shown), None);
        let toggled = ProbeReadout {
            energy: false,
            ..shown
        };
        assert_eq!(
            edited_readout(stored, shown, toggled),
            Some(ProbeReadout {
                energy: false,
                ..stored
            })
        );
        let zoomed = ProbeReadout {
            span: 0.25,
            ..shown
        };
        assert_eq!(
            edited_readout(stored, shown, zoomed),
            Some(ProbeReadout {
                span: 0.25,
                ..stored
            })
        );
    }

    #[test]
    fn the_plot_matrix_addresses_every_cell_exactly_once() {
        let view = ProbeViewState::new(funfern_app::document::ProbeReadout::default());
        let mut seen = BTreeSet::new();
        for quantity in LineProbeQuantity::ALL {
            for representation in LineProbeRepresentation::ALL {
                let index = quantity.offset() + representation.offset();
                assert!(
                    index < view.readout.line_plots.len(),
                    "{quantity:?} {representation:?}"
                );
                assert!(
                    seen.insert(index),
                    "{quantity:?} {representation:?} collides"
                );
            }
        }
        assert_eq!(seen.len(), view.readout.line_plots.len());

        // A new readout opens on the field's profile and waterfall, the mean
        // flux and mean energy profiles, and the two integrals.
        let enabled = |quantity: LineProbeQuantity, representation: LineProbeRepresentation| {
            view.readout.line_plots[quantity.offset() + representation.offset()]
        };
        assert!(enabled(
            LineProbeQuantity::Field,
            LineProbeRepresentation::Arclength
        ));
        assert!(enabled(
            LineProbeQuantity::Field,
            LineProbeRepresentation::Waterfall
        ));
        assert!(enabled(
            LineProbeQuantity::MeanFlux,
            LineProbeRepresentation::Arclength
        ));
        assert!(enabled(
            LineProbeQuantity::Flux,
            LineProbeRepresentation::Integral
        ));
        assert!(enabled(
            LineProbeQuantity::Energy,
            LineProbeRepresentation::Integral
        ));
        assert!(enabled(
            LineProbeQuantity::MeanEnergy,
            LineProbeRepresentation::Arclength
        ));
        assert_eq!(
            view.readout.line_plots.iter().filter(|plot| **plot).count(),
            6
        );
    }

    /// The average energy density of a wave running past a probe is the mean
    /// of its `sin²` swing, half its peak, once the window is full; before
    /// that the row is NaN, as the average flux's is.
    #[test]
    fn the_mean_energy_row_averages_a_swinging_density() {
        let frequency = 2.0;
        let frames = (0..400)
            .map(|index| {
                let time = index as f64 / 200.0;
                let density = (std::f64::consts::TAU * frequency * time).sin().powi(2);
                CurveProbeRecord {
                    probe_id: 1,
                    time,
                    energy_density: vec![density as f32, 2.0 * density as f32],
                    normal_flux: vec![0.0; 2],
                    ..Default::default()
                }
            })
            .collect::<Vec<_>>();
        let (means, filled) = Playground::curve_probe_running_mean(&frames, 1.0);
        assert_eq!(filled, 1.0);
        let last = &means[means.len() - 1];
        assert!(
            (last.energy_density[0] - 0.5).abs() < 0.01,
            "{:?}",
            last.energy_density
        );
        assert!(
            (last.energy_density[1] - 1.0).abs() < 0.02,
            "{:?}",
            last.energy_density
        );
        assert!(means[10].energy_density.iter().all(|value| value.is_nan()));
    }

    /// The window slider stopped at the preset's nominal rate, which the
    /// recorder does not sample at: it strides the solver's steps, so a coarse
    /// enough step rounds the stride down and the ring reaches back a quarter
    /// less than the slider offered. The top of the slider filled nothing,
    /// however long the run went on.
    #[test]
    fn the_mean_window_stops_at_what_the_trace_can_reach() {
        // The default mesh's time step, which rounds a 120 Hz preset to every
        // step: 149 Hz recorded, not 120.
        let interval = 6.692_306e-3;
        let nominal = 1.0 / 120.0;
        let frames = flux_trace(CURVE_TRACE_FRAMES, interval, 4, |time, _| time as f32);

        let old = CURVE_TRACE_FRAMES as f64 * nominal;
        let (_, old_filled) = Playground::curve_probe_running_mean(&frames, old);
        assert!(
            old_filled < 1.0,
            "the nominal ceiling used to fill: {old_filled}"
        );

        let limit = Playground::curve_mean_window_limit(&frames, nominal);
        assert!(limit < old, "{limit} is not below the nominal {old}");
        let (means, filled) = Playground::curve_probe_running_mean(&frames, limit);
        assert_eq!(filled, 1.0, "the top of the slider never filled");
        assert!(
            means
                .last()
                .unwrap()
                .normal_flux
                .iter()
                .all(|v| v.is_finite()),
            "the newest frame is still waiting at the limit"
        );

        // A dropped readback leaves a gap. The smallest interval is still the
        // stride, so the limit stays reachable rather than following the gap up.
        let mut gapped = frames.clone();
        gapped.remove(CURVE_TRACE_FRAMES / 2);
        let gapped_limit = Playground::curve_mean_window_limit(&gapped, nominal);
        assert!((gapped_limit - limit).abs() < 1.0e-9);
        let (_, gapped_filled) = Playground::curve_probe_running_mean(&gapped, gapped_limit);
        assert_eq!(gapped_filled, 1.0);

        // Nothing recorded yet: the nominal rate is all there is to go on, and
        // the slider still has a range.
        assert_eq!(
            Playground::curve_mean_window_limit(&[], nominal),
            nominal * (CURVE_TRACE_FRAMES - 2) as f64
        );
    }

    /// Frames of `points` samples at `dt`, each value from the frame's time and
    /// the point's index.
    fn flux_trace(
        count: usize,
        dt: f64,
        points: usize,
        value: impl Fn(f64, usize) -> f32,
    ) -> Vec<CurveProbeRecord> {
        (0..count)
            .map(|frame| {
                let time = frame as f64 * dt;
                CurveProbeRecord {
                    probe_id: 1,
                    time,
                    normal_flux: (0..points).map(|point| value(time, point)).collect(),
                    ..Default::default()
                }
            })
            .collect()
    }

    #[test]
    fn the_averaged_flux_row_withholds_a_window_it_cannot_fill() {
        let frames = flux_trace(40, 0.1, 3, |_, point| 2.0 + point as f32);
        let (means, filled) = Playground::curve_probe_running_mean(&frames, 1.0);

        // One row per recorded frame, so every representation keeps the raw
        // series' time alignment.
        assert_eq!(means.len(), frames.len());
        assert!(
            means
                .iter()
                .zip(&frames)
                .all(|(mean, frame)| mean.time == frame.time)
        );

        // Nothing before the record reaches a full second back: a mean over
        // whatever happens to be recorded is not what the row claims to show.
        assert!(
            means[..10]
                .iter()
                .all(|mean| mean.normal_flux.iter().all(|value| value.is_nan()))
        );
        for mean in &means[10..] {
            assert_eq!(mean.normal_flux, vec![2.0, 3.0, 4.0]);
        }
        assert_eq!(filled, 1.0);

        // Reported while it fills, so an empty plot reads as a recorder still
        // filling rather than a silence in the field.
        let (_, partial) = Playground::curve_probe_running_mean(&frames[..5], 1.0);
        assert!((partial - 0.4).abs() < 1.0e-9, "{partial}");
    }

    #[test]
    fn the_averaged_flux_row_cancels_a_standing_wave_and_keeps_a_net_flow() {
        // A standing wave's flux swings symmetrically about zero at twice the
        // driven frequency; a travelling one carries a constant across it. The
        // instantaneous row cannot tell them apart at an arbitrary instant.
        let dt = 1.0 / 120.0;
        let flux = |time: f64, point: usize| {
            let offset = if point == 0 { 0.0 } else { 0.3 };
            (offset + (std::f64::consts::TAU * 5.0 * time).sin()) as f32
        };
        let frames = flux_trace(600, dt, 2, flux);
        let (means, _) = Playground::curve_probe_running_mean(&frames, 1.0);
        let newest = means.last().unwrap();

        // Five whole periods of the oscillation land in the window, to within
        // the one sample the window's edge is ambiguous by.
        assert!(
            newest.normal_flux[0].abs() < 0.02,
            "{}",
            newest.normal_flux[0]
        );
        assert!(
            (newest.normal_flux[1] - 0.3).abs() < 0.02,
            "{}",
            newest.normal_flux[1]
        );
    }

    #[test]
    fn the_averaged_flux_row_skips_gaps_point_by_point() {
        // Portions outside the domain or on a two-trace boundary arrive as NaN,
        // and one bad point must not discard the rest of the path.
        let frames = flux_trace(40, 0.1, 3, |time, point| match point {
            0 => f32::NAN,
            1 if time < 1.5 => f32::NAN,
            _ => 4.0,
        });
        let (means, _) = Playground::curve_probe_running_mean(&frames, 1.0);
        let newest = means.last().unwrap();

        assert!(newest.normal_flux[0].is_nan());
        assert_eq!(newest.normal_flux[1], 4.0);
        assert_eq!(newest.normal_flux[2], 4.0);

        // Halfway through its gap the point averages only what it has, not a
        // zero for every frame it was missing.
        let straddling = means
            .iter()
            .find(|mean| (mean.time - 2.0).abs() < 1.0e-9)
            .unwrap();
        assert_eq!(straddling.normal_flux[1], 4.0);
    }

    #[test]
    fn the_averaged_flux_row_restarts_when_the_sample_layout_changes() {
        // A sampling preset change or a boundary remesh leaves rows of another
        // length in the same trace. Two layouts have no common average.
        let mut frames = flux_trace(20, 0.1, 2, |_, _| 1.0);
        frames.extend(flux_trace(20, 0.1, 4, |_, _| 5.0).into_iter().map(|frame| {
            CurveProbeRecord {
                time: frame.time + 2.0,
                ..frame
            }
        }));
        let (means, filled) = Playground::curve_probe_running_mean(&frames, 1.0);

        assert_eq!(means[19].normal_flux, vec![1.0, 1.0]);
        // The first second after the change is withheld, then the mean is the
        // new layout's alone.
        assert!(means[25].normal_flux.iter().all(|value| value.is_nan()));
        assert_eq!(means[35].normal_flux, vec![5.0; 4]);
        assert_eq!(filled, 1.0);
    }
}
