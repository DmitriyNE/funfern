//! A small plot of one curve through given points, in the probe plots'
//! style: the pulse shape window's trace and spectrum, and a probe's
//! spectrum and transfer.
use super::*;

/// A vertical line across the plot at `x`, with its label beside it.
pub(super) struct PlotMarker {
    pub(super) x: f64,
    pub(super) label: String,
}

const AXIS_TEXT: Color32 = Color32::from_rgb(142, 161, 175);
const MARKER: Color32 = Color32::from_rgb(112, 130, 143);

/// Draws `points`, `x` increasing, scaled to fill a plot `height` tall: the
/// value range at the left, the `x` range underneath in `x_unit`, and
/// `markers` inside the range as vertical lines. A point that is not finite
/// breaks the curve. Nothing is drawn for fewer than two finite points but
/// the frame and `empty`.
pub(super) fn line_plot(
    ui: &mut egui::Ui,
    points: &[[f64; 2]],
    color: Color32,
    x_unit: &str,
    markers: &[PlotMarker],
    height: f32,
    empty: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(120.0), height),
        egui::Sense::hover(),
    );
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, Color32::from_rgb(12, 18, 24));
    painter.rect_stroke(
        rect,
        2.0,
        Stroke::new(1.0, Color32::from_rgb(55, 69, 80)),
        egui::StrokeKind::Inside,
    );
    let finite = points
        .iter()
        .filter(|[x, y]| x.is_finite() && y.is_finite())
        .collect::<Vec<_>>();
    let (Some(first), Some(last)) = (finite.first(), finite.last()) else {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            empty,
            egui::FontId::monospace(11.0),
            MARKER,
        );
        return response;
    };
    let (minimum_x, maximum_x) = (first[0], last[0]);
    let (mut minimum, mut maximum) = finite.iter().fold(
        (f64::INFINITY, f64::NEG_INFINITY),
        |(minimum, maximum), [_, y]| (minimum.min(*y), maximum.max(*y)),
    );
    if finite.len() < 2 || maximum_x <= minimum_x {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            empty,
            egui::FontId::monospace(11.0),
            MARKER,
        );
        return response;
    }
    if maximum - minimum < 1.0e-15 * maximum.abs().max(1.0) {
        let padding = maximum.abs().max(1.0) * 0.05;
        minimum -= padding;
        maximum += padding;
    }
    let x_at = |x: f64| {
        egui::lerp(
            rect.left()..=rect.right(),
            ((x - minimum_x) / (maximum_x - minimum_x)) as f32,
        )
    };
    let y_at = |y: f64| {
        egui::lerp(
            rect.bottom()..=rect.top(),
            ((y - minimum) / (maximum - minimum)) as f32,
        )
    };
    for marker in markers {
        if marker.x < minimum_x || marker.x > maximum_x {
            continue;
        }
        let x = x_at(marker.x);
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            Stroke::new(1.0, MARKER),
        );
        painter.text(
            egui::pos2(x + 3.0, rect.top() + 14.0),
            egui::Align2::LEFT_TOP,
            &marker.label,
            egui::FontId::monospace(9.0),
            AXIS_TEXT,
        );
    }
    // A point that is not finite is a gap: the line stops there and starts
    // again after it rather than bridging what has no value.
    for run in points.split(|[x, y]| !x.is_finite() || !y.is_finite()) {
        let run = run
            .iter()
            .map(|[x, y]| egui::pos2(x_at(*x), y_at(*y)))
            .collect::<Vec<_>>();
        match run.as_slice() {
            [] => {}
            [alone] => {
                painter.circle_filled(*alone, 1.4, color);
            }
            _ => {
                painter.add(egui::Shape::line(run, Stroke::new(1.4, color)));
            }
        }
    }
    for (text, anchor, at) in [
        (
            format!("{maximum:+.3e}"),
            egui::Align2::LEFT_TOP,
            rect.left_top() + egui::vec2(4.0, 3.0),
        ),
        (
            format!("{minimum:+.3e}"),
            egui::Align2::LEFT_BOTTOM,
            rect.left_bottom() + egui::vec2(4.0, -3.0),
        ),
        (
            format!("{minimum_x:.3}–{maximum_x:.3} {x_unit}"),
            egui::Align2::RIGHT_BOTTOM,
            rect.right_bottom() + egui::vec2(-4.0, -3.0),
        ),
    ] {
        painter.text(at, anchor, text, egui::FontId::monospace(9.0), AXIS_TEXT);
    }
    response
}
