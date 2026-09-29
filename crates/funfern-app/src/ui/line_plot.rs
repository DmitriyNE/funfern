//! A small plot of one curve through given points, in the probe plots'
//! style: the pulse shape window's trace and spectrum, and a probe's
//! spectrum and transfer, which the user can zoom and pan.
use super::*;

/// A vertical line across the plot at `x`, with its label beside it.
pub(super) struct PlotMarker {
    pub(super) x: f64,
    pub(super) label: String,
}

const AXIS_TEXT: Color32 = Color32::from_rgb(142, 161, 175);
const MARKER: Color32 = Color32::from_rgb(112, 130, 143);

/// The `x` extent of `points`: from the first point with a finite `x` to the
/// last, so a gap at either end keeps its place on the axis.
fn extent(points: &[[f64; 2]]) -> Option<[f64; 2]> {
    let first = points.iter().find(|[x, _]| x.is_finite())?[0];
    let last = points.iter().rev().find(|[x, _]| x.is_finite())?[0];
    (last > first).then_some([first, last])
}

/// The `x` range and the value range a plot of `points` spans, or `None`
/// for fewer than two finite points in it. The `x` range is `band`, or the
/// points' whole extent without one; the value range runs over the finite
/// points inside it and `y_from`, where there is one.
fn plot_range(
    points: &[[f64; 2]],
    band: Option<[f64; 2]>,
    y_from: Option<f64>,
) -> Option<([f64; 2], [f64; 2])> {
    let [first, last] = band.or_else(|| extent(points))?;
    let finite = points
        .iter()
        .filter(|[x, y]| y.is_finite() && (first..=last).contains(x))
        .collect::<Vec<_>>();
    if finite.len() < 2 || last <= first {
        return None;
    }
    let (mut minimum, mut maximum) = finite.iter().fold(
        (f64::INFINITY, f64::NEG_INFINITY),
        |(minimum, maximum), [_, y]| (minimum.min(*y), maximum.max(*y)),
    );
    if let Some(from) = y_from.filter(|from| from.is_finite()) {
        minimum = minimum.min(from);
        maximum = maximum.max(from);
    }
    Some(([first, last], [minimum, maximum]))
}

/// The band `points` have values in, from the first drawn to the last, as
/// Fit zooms to it; or the whole extent, `None`, where that is all of it.
pub(super) fn fitted_band(points: &[[f64; 2]]) -> Option<[f64; 2]> {
    let drawn = |[x, y]: &&[f64; 2]| x.is_finite() && y.is_finite();
    let first = points.iter().find(drawn)?[0];
    let last = points.iter().rev().find(drawn)?[0];
    (last > first && Some([first, last]) != extent(points)).then_some([first, last])
}

/// `band` of `extent` zoomed by `factor`, under one zooming in, about the
/// point `anchor` of the way across it, as a probe's time trace zooms about
/// the pointer. It never narrows below `narrowest`, and once it would cover
/// the whole extent it is the whole extent again: `None`.
fn zoomed(
    band: Option<[f64; 2]>,
    extent: [f64; 2],
    anchor: f64,
    factor: f64,
    narrowest: f64,
) -> Option<[f64; 2]> {
    let [from, to] = band.unwrap_or(extent);
    let width = ((to - from) * factor).max(narrowest);
    if width >= extent[1] - extent[0] {
        return None;
    }
    let at = from + anchor * (to - from);
    let start = (at - anchor * width).clamp(extent[0], extent[1] - width);
    Some([start, start + width])
}

/// `band` of `extent` moved `shift` of its own width along, stopping at the
/// extent's ends. The whole extent has nowhere to move.
fn panned(band: Option<[f64; 2]>, extent: [f64; 2], shift: f64) -> Option<[f64; 2]> {
    let [from, to] = band?;
    let width = to - from;
    let start = (from + shift * width).clamp(extent[0], extent[1] - width);
    Some([start, start + width])
}

/// Draws `points`, `x` increasing, scaled to fill a plot `height` tall: the
/// value range at the left, reaching `y_from` where there is one, the `x`
/// range underneath in `x_unit`, and `markers` inside the range as vertical
/// lines. A point that is not finite breaks the curve. Nothing is drawn for
/// fewer than two finite points but the frame and `empty`.
#[allow(clippy::too_many_arguments)]
pub(super) fn line_plot(
    ui: &mut egui::Ui,
    points: &[[f64; 2]],
    y_from: Option<f64>,
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
    draw(
        ui, rect, points, None, y_from, color, x_unit, markers, empty,
    );
    response
}

/// `line_plot` over `band` of its points' extent, or all of it without one,
/// which the plot zooms and pans as a probe's time trace does: the wheel
/// zooms about the pointer, a drag pans, and zooming all the way out shows
/// everything again.
#[allow(clippy::too_many_arguments)]
pub(super) fn zoomable_line_plot(
    ui: &mut egui::Ui,
    points: &[[f64; 2]],
    y_from: Option<f64>,
    color: Color32,
    x_unit: &str,
    band: &mut Option<[f64; 2]>,
    height: f32,
    empty: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(120.0), height),
        egui::Sense::drag(),
    );
    if let Some(whole) = extent(points) {
        // A band from another extent, a ceiling since lowered, is kept to
        // this one.
        *band = band.and_then(|[from, to]| {
            let width = (to - from).min(whole[1] - whole[0]);
            let start = from.clamp(whole[0], whole[1] - width);
            (width < whole[1] - whole[0]).then_some([start, start + width])
        });
        if response.dragged() {
            let shift = -f64::from(response.drag_delta().x / rect.width());
            *band = panned(*band, whole, shift);
        }
        if response.hovered() {
            let wheel = ui.ctx().input(|input| input.smooth_scroll_delta.y);
            if wheel != 0.0 {
                let anchor = response.hover_pos().map_or(0.5, |position| {
                    ((position.x - rect.left()) / rect.width()).clamp(0.0, 1.0)
                });
                // Four of the points' own spacings, the narrowest a band can
                // still draw a curve in.
                let narrowest = 4.0 * (whole[1] - whole[0]) / points.len().max(2) as f64;
                *band = zoomed(
                    *band,
                    whole,
                    f64::from(anchor),
                    (-f64::from(wheel) * 0.01).exp(),
                    narrowest,
                );
            }
        }
    }
    draw(ui, rect, points, *band, y_from, color, x_unit, &[], empty);
    response
}

#[allow(clippy::too_many_arguments)]
fn draw(
    ui: &egui::Ui,
    rect: Rect,
    points: &[[f64; 2]],
    band: Option<[f64; 2]>,
    y_from: Option<f64>,
    color: Color32,
    x_unit: &str,
    markers: &[PlotMarker],
    empty: &str,
) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, Color32::from_rgb(12, 18, 24));
    painter.rect_stroke(
        rect,
        2.0,
        Stroke::new(1.0, Color32::from_rgb(55, 69, 80)),
        egui::StrokeKind::Inside,
    );
    let Some(([minimum_x, maximum_x], [mut minimum, mut maximum])) =
        plot_range(points, band, y_from)
    else {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            empty,
            egui::FontId::monospace(11.0),
            MARKER,
        );
        return;
    };
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
    // again after it rather than bridging what has no value. Outside a zoomed
    // band only the neighbours of its ends are drawn, which the painter
    // clips at the frame, so a curve leaves the plot rather than stopping
    // short of its edge.
    let first = points
        .iter()
        .position(|[x, _]| *x >= minimum_x)
        .map_or(0, |index| index.saturating_sub(1));
    let last = points
        .iter()
        .rposition(|[x, _]| *x <= maximum_x)
        .map_or(points.len(), |index| (index + 2).min(points.len()));
    for run in points[first..last].split(|[x, y]| !x.is_finite() || !y.is_finite()) {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plot spans every point's `x`, gaps at either end included, so a
    /// transfer lines up under the spectrum above it; its values reach down
    /// to where it is told to start.
    #[test]
    fn a_plot_spans_its_gaps_and_starts_where_it_is_told() {
        let points = [
            [0.0, f64::NAN],
            [1.0, 0.9],
            [2.0, 1.1],
            [3.0, f64::NAN],
            [4.0, 1.0],
            [5.0, f64::NAN],
        ];
        assert_eq!(
            plot_range(&points, None, None),
            Some(([0.0, 5.0], [0.9, 1.1]))
        );
        assert_eq!(
            plot_range(&points, None, Some(0.0)),
            Some(([0.0, 5.0], [0.0, 1.1]))
        );
        assert_eq!(
            plot_range(&points[..3], None, Some(f64::NAN)).unwrap().1,
            [0.9, 1.1]
        );
        assert_eq!(
            plot_range(&[[0.0, 1.0], [1.0, f64::NAN]], None, Some(0.0)),
            None
        );
    }

    /// Zoomed to a band, a plot spans the band and ranges its values over
    /// the points inside it alone.
    #[test]
    fn a_zoomed_plot_ranges_what_its_band_holds() {
        let points = (0..=10)
            .map(|index| [f64::from(index), f64::from(index * index)])
            .collect::<Vec<_>>();
        assert_eq!(
            plot_range(&points, Some([2.0, 4.0]), None),
            Some(([2.0, 4.0], [4.0, 16.0]))
        );
        assert_eq!(
            plot_range(&points, Some([2.0, 4.0]), Some(0.0)),
            Some(([2.0, 4.0], [0.0, 16.0]))
        );
        assert_eq!(plot_range(&points, Some([2.2, 2.8]), None), None);
    }

    /// Fit takes the band from the first value drawn to the last, and the
    /// whole extent where the values reach both its ends.
    #[test]
    fn fit_takes_the_band_the_values_cover() {
        let gappy = [
            [0.0, f64::NAN],
            [1.0, f64::NAN],
            [2.0, 0.5],
            [3.0, f64::NAN],
            [4.0, 0.7],
            [5.0, f64::NAN],
        ];
        assert_eq!(fitted_band(&gappy), Some([2.0, 4.0]));
        let whole = [[0.0, 1.0], [1.0, 2.0], [2.0, 3.0]];
        assert_eq!(fitted_band(&whole), None);
        assert_eq!(fitted_band(&gappy[..3]), None);
        assert_eq!(fitted_band(&[]), None);
    }

    /// The wheel zooms about the pointer, keeping the frequency under it
    /// where it was; zooming out past the whole extent shows it all again;
    /// and a band never narrows below the narrowest asked for. A drag moves
    /// the band along and stops at the extent's ends.
    #[test]
    fn a_band_zooms_about_the_pointer_and_pans_within_its_extent() {
        let extent = [0.0, 10.0];
        let band = zoomed(None, extent, 0.25, 0.5, 0.1).unwrap();
        assert_eq!(band, [1.25, 6.25]);
        assert_eq!(band[0] + 0.25 * (band[1] - band[0]), 2.5);
        assert_eq!(zoomed(Some(band), extent, 0.5, 3.0, 0.1), None);
        let [from, to] = zoomed(Some([4.0, 4.2]), extent, 0.5, 0.01, 0.1).unwrap();
        assert!((from - 4.05).abs() < 1.0e-12 && (to - from - 0.1).abs() < 1.0e-12);
        assert_eq!(zoomed(None, extent, 0.0, 0.5, 0.1), Some([0.0, 5.0]));
        assert_eq!(panned(Some([2.0, 4.0]), extent, 0.5), Some([3.0, 5.0]));
        assert_eq!(panned(Some([2.0, 4.0]), extent, 10.0), Some([8.0, 10.0]));
        assert_eq!(panned(Some([2.0, 4.0]), extent, -10.0), Some([0.0, 2.0]));
        assert_eq!(panned(None, extent, 0.5), None);
    }
}
