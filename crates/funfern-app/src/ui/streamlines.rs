//! Streamlines of the vector overlay. The overlay's lattice, filtered as the
//! arrows are, is read bilinearly between its cell centres; a line is
//! integrated through it with RK4 both ways from a seed, stopped where the
//! field falls below the exposure's floor, where the lattice ends, where it
//! comes within half a separation of another line or of itself, or at a
//! length cap. Lines are placed an even separation apart after Jobard and
//! Lefer: candidate seeds one separation to either side of every line,
//! accepted when no line already passes within a separation. The previous
//! frame's seeds are placed first, in their order, so a line that still has
//! room stays where it was as the field evolves.
//!
//! Everything here is in world coordinates. The view only chooses where
//! seeds may start, so a pan or a zoom moves the picture and nothing else.

use funfern_core::Point2;
use std::collections::HashMap;

/// A line alone has no direction, so it is drawn as dashes that drift along
/// the flow: this many screen pixels a simulated second, so they stand when
/// the simulation is paused and quicken with its speed.
pub(super) const STREAMLINE_DASH_SPEED: f64 = 40.0;
/// The dash and its period, in screen pixels.
pub(super) const STREAMLINE_DASH_ON: f64 = 8.0;
pub(super) const STREAMLINE_DASH_PERIOD: f64 = 14.0;
/// The share of a dash's alpha every line keeps however weak its flow, so
/// a line the floor let through is legible; the rest follows the exposed
/// strength as the arrows' length does.
pub(super) const STREAMLINE_ALPHA_FLOOR: f64 = 0.3;

/// The overlay's lattice values at the cell centres, by world cell.
pub(super) struct VectorLattice {
    spacing: f64,
    column: i64,
    row: i64,
    columns: usize,
    rows: usize,
    values: Vec<Option<Point2>>,
}

/// More cells than any viewport asks for; a lattice past it is refused
/// rather than allocated.
const MAX_LATTICE_CELLS: usize = 1 << 22;

impl VectorLattice {
    /// From samples at the cell centres of a lattice of `spacing`, each the
    /// centre's world position and the field there. A position is placed by
    /// the cell it falls in, so a centre read back through f32 still lands.
    pub(super) fn new(
        spacing: f64,
        samples: impl IntoIterator<Item = (Point2, Point2)>,
    ) -> Option<Self> {
        if !spacing.is_finite() || spacing <= 0.0 {
            return None;
        }
        let samples = samples
            .into_iter()
            .filter(|(point, value)| point.finite() && value.finite())
            .map(|(point, value)| {
                (
                    (point.x / spacing).floor() as i64,
                    (point.y / spacing).floor() as i64,
                    value,
                )
            })
            .collect::<Vec<_>>();
        let (first, rest) = samples.split_first()?;
        let mut bounds = [first.0, first.0, first.1, first.1];
        for (column, row, _) in rest {
            bounds[0] = bounds[0].min(*column);
            bounds[1] = bounds[1].max(*column);
            bounds[2] = bounds[2].min(*row);
            bounds[3] = bounds[3].max(*row);
        }
        let columns = usize::try_from(bounds[1] - bounds[0] + 1).ok()?;
        let rows = usize::try_from(bounds[3] - bounds[2] + 1).ok()?;
        if columns.checked_mul(rows)? > MAX_LATTICE_CELLS {
            return None;
        }
        let mut values = vec![None; columns * rows];
        for (column, row, value) in samples {
            let index = (row - bounds[2]) as usize * columns + (column - bounds[0]) as usize;
            values[index] = Some(value);
        }
        Some(Self {
            spacing,
            column: bounds[0],
            row: bounds[2],
            columns,
            rows,
            values,
        })
    }

    fn cell(&self, column: i64, row: i64) -> Option<Point2> {
        let column = usize::try_from(column - self.column).ok()?;
        let row = usize::try_from(row - self.row).ok()?;
        if column >= self.columns || row >= self.rows {
            return None;
        }
        self.values[row * self.columns + column]
    }

    /// The field at a world point, bilinear between the four cell centres
    /// around it. A centre without a sample drops out and the others are
    /// reweighted, so a line reads up to half a cell from the lattice's
    /// edge; with less than half the weight present there is nothing to
    /// read.
    pub(super) fn sample(&self, point: Point2) -> Option<Point2> {
        if !point.finite() {
            return None;
        }
        let u = point.x / self.spacing - 0.5;
        let v = point.y / self.spacing - 0.5;
        let (column, row) = (u.floor(), v.floor());
        let (fu, fv) = (u - column, v - row);
        let (column, row) = (column as i64, row as i64);
        let corners = [
            ((column, row), (1.0 - fu) * (1.0 - fv)),
            ((column + 1, row), fu * (1.0 - fv)),
            ((column, row + 1), (1.0 - fu) * fv),
            ((column + 1, row + 1), fu * fv),
        ];
        let mut sum = Point2::default();
        let mut weight = 0.0;
        for ((column, row), corner_weight) in corners {
            if let Some(value) = self.cell(column, row) {
                sum = sum + value * corner_weight;
                weight += corner_weight;
            }
        }
        (weight >= 0.5).then(|| sum / weight)
    }

    /// The centre of the cell with the strongest field inside `view`, where
    /// the first line of a picture starts.
    pub(super) fn strongest(&self, view: &WorldRect) -> Option<Point2> {
        let mut best: Option<(f64, Point2)> = None;
        for (index, value) in self.values.iter().enumerate() {
            let Some(value) = value else {
                continue;
            };
            let center = Point2::new(
                ((index % self.columns) as i64 + self.column) as f64 + 0.5,
                ((index / self.columns) as i64 + self.row) as f64 + 0.5,
            ) * self.spacing;
            let magnitude = value.norm();
            if view.contains(center)
                && magnitude.is_finite()
                && best.is_none_or(|(strongest, _)| magnitude > strongest)
            {
                best = Some((magnitude, center));
            }
        }
        best.map(|(_, center)| center)
    }
}

/// An axis-aligned world rectangle, the view's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct WorldRect {
    pub min: Point2,
    pub max: Point2,
}

impl WorldRect {
    pub(super) fn contains(&self, point: Point2) -> bool {
        point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct StreamlineParameters {
    /// The distance between lines, in world units.
    pub separation: f64,
    /// The integration step; an eighth of the separation draws smooth lines.
    pub step: f64,
    /// The magnitude below which the field has no direction worth following.
    pub floor: f64,
    /// A line longer than this is cut, each way from its seed taking half.
    pub maximum_length: f64,
}

impl StreamlineParameters {
    fn valid(&self) -> bool {
        self.separation.is_finite()
            && self.separation > 0.0
            && self.step.is_finite()
            && self.step > 0.0
            && self.floor.is_finite()
            && self.floor >= 0.0
            && self.maximum_length.is_finite()
            && self.maximum_length > 0.0
    }

    /// A line stops this close to another line, or to itself.
    fn test_distance(&self) -> f64 {
        self.separation * 0.5
    }

    /// A fresh candidate, placed one separation beside its parent line, is
    /// accepted when no line passes closer than this to it. The parent's own
    /// point is exactly a separation away, closer by rounding or where the
    /// parent curves toward the candidate, so the test leaves a margin.
    fn acceptance_distance(&self) -> f64 {
        self.separation * 0.9
    }

    /// A seed carried from the previous frame is accepted with more room to
    /// spare: its neighbours were placed exactly a separation from it, and
    /// the field has moved them since. A line that has drifted closer than
    /// this to another dies and a fresh candidate takes the room.
    fn carried_distance(&self) -> f64 {
        self.separation * 0.75
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Streamline {
    /// Where the line was started, kept so the next frame starts it there.
    pub seed: Point2,
    /// The line's points in the order of the flow; `points[seed_index]` is
    /// the seed.
    pub points: Vec<Point2>,
    pub seed_index: usize,
}

impl Streamline {
    /// The arc length from the first point to each point, and so the
    /// line's length as the last entry.
    pub(super) fn cumulative_arcs(&self) -> Vec<f64> {
        let mut arcs = Vec::with_capacity(self.points.len());
        let mut total = 0.0;
        for (index, point) in self.points.iter().enumerate() {
            if index > 0 {
                total += (*point - self.points[index - 1]).norm();
            }
            arcs.push(total);
        }
        arcs
    }
}

/// The picture's lines, kept between frames for their seeds.
#[derive(Default)]
pub(super) struct Streamlines {
    pub lines: Vec<Streamline>,
}

impl Streamlines {
    /// Places the lines anew on `lattice`: the previous lines' seeds first,
    /// in their order, then the strongest cell if nothing survived, then
    /// Jobard and Lefer's candidates beside every line until no room is
    /// left. Seeds start inside `view`; lines run to the lattice's edge.
    pub(super) fn place(
        &mut self,
        lattice: &VectorLattice,
        parameters: &StreamlineParameters,
        view: &WorldRect,
    ) {
        let carried = self.lines.iter().map(|line| line.seed).collect::<Vec<_>>();
        self.lines.clear();
        if !parameters.valid() {
            return;
        }
        let mut occupancy = Occupancy::new(parameters.separation);
        let carried_room = parameters.carried_distance();
        for seed in carried {
            grow(
                lattice,
                parameters,
                view,
                &mut occupancy,
                &mut self.lines,
                seed,
                carried_room,
            );
        }
        let room = parameters.acceptance_distance();
        if self.lines.is_empty()
            && let Some(seed) = lattice.strongest(view)
        {
            grow(
                lattice,
                parameters,
                view,
                &mut occupancy,
                &mut self.lines,
                seed,
                room,
            );
        }
        let mut index = 0;
        while index < self.lines.len() {
            for candidate in candidates_beside(&self.lines[index], parameters.separation) {
                grow(
                    lattice,
                    parameters,
                    view,
                    &mut occupancy,
                    &mut self.lines,
                    candidate,
                    room,
                );
            }
            index += 1;
        }
    }
}

/// Seeds one separation to either side of the line, every half separation
/// along it.
fn candidates_beside(line: &Streamline, separation: f64) -> Vec<Point2> {
    let mut candidates = Vec::new();
    let mut since_last = f64::INFINITY;
    for index in 1..line.points.len().saturating_sub(1) {
        let (previous, point, next) = (
            line.points[index - 1],
            line.points[index],
            line.points[index + 1],
        );
        since_last += (point - previous).norm();
        if since_last < separation * 0.5 {
            continue;
        }
        let tangent = next - previous;
        let length = tangent.norm();
        if length.is_nan() || length <= 0.0 {
            continue;
        }
        let normal = Point2::new(-tangent.y, tangent.x) / length;
        candidates.push(point + normal * separation);
        candidates.push(point - normal * separation);
        since_last = 0.0;
    }
    candidates
}

/// Starts a line at `seed` if no line passes within `room` of it,
/// integrates it both ways, and keeps it when it has any length. Its points
/// join the occupancy either way, since a spot too small for a line is too
/// small for the next one.
fn grow(
    lattice: &VectorLattice,
    parameters: &StreamlineParameters,
    view: &WorldRect,
    occupancy: &mut Occupancy,
    lines: &mut Vec<Streamline>,
    seed: Point2,
    room: f64,
) -> bool {
    if !view.contains(seed) || occupancy.within(seed, room, None) {
        return false;
    }
    if direction(lattice, seed, parameters.floor).is_none() {
        return false;
    }
    let line = lines.len();
    occupancy.insert(seed, line, 0.0);
    let backward = integrate(lattice, parameters, seed, -1.0, occupancy, line);
    let forward = integrate(lattice, parameters, seed, 1.0, occupancy, line);
    if backward.len() + forward.len() < 2 {
        return false;
    }
    let mut points = backward;
    points.reverse();
    let seed_index = points.len();
    points.push(seed);
    points.extend(forward);
    lines.push(Streamline {
        seed,
        points,
        seed_index,
    });
    true
}

/// The points after the seed in one direction, `sign` along or against the
/// flow, each added to the occupancy as it is reached.
fn integrate(
    lattice: &VectorLattice,
    parameters: &StreamlineParameters,
    seed: Point2,
    sign: f64,
    occupancy: &mut Occupancy,
    line: usize,
) -> Vec<Point2> {
    let mut points = Vec::new();
    let mut point = seed;
    let mut arc = 0.0;
    let half_length = parameters.maximum_length * 0.5;
    let steps = (half_length / parameters.step).ceil() as usize + 2;
    for _ in 0..steps {
        let Some(next) = rk4(lattice, point, parameters.step, sign, parameters.floor) else {
            break;
        };
        let moved = (next - point).norm();
        if moved.is_nan() || moved <= parameters.step * 0.1 {
            break;
        }
        arc += moved;
        if arc > half_length {
            break;
        }
        if occupancy.within(
            next,
            parameters.test_distance(),
            Some((line, sign * arc, parameters.separation)),
        ) {
            break;
        }
        occupancy.insert(next, line, sign * arc);
        points.push(next);
        point = next;
    }
    points
}

/// The unit direction of the field at `point`, when the field is there and
/// at least `floor` strong.
fn direction(lattice: &VectorLattice, point: Point2, floor: f64) -> Option<Point2> {
    let value = lattice.sample(point)?;
    let magnitude = value.norm();
    (magnitude.is_finite() && magnitude > 0.0 && magnitude >= floor).then(|| value / magnitude)
}

/// One Runge–Kutta step of `h` along the unit direction field, `sign` with
/// or against it.
fn rk4(lattice: &VectorLattice, point: Point2, h: f64, sign: f64, floor: f64) -> Option<Point2> {
    let at = |at: Point2| direction(lattice, at, floor).map(|unit| unit * sign);
    let k1 = at(point)?;
    let k2 = at(point + k1 * (h * 0.5))?;
    let k3 = at(point + k2 * (h * 0.5))?;
    let k4 = at(point + k3 * h)?;
    Some(point + (k1 + k2 * 2.0 + k3 * 2.0 + k4) * (h / 6.0))
}

/// Every placed point, bucketed by cells of one separation, so a distance
/// test looks at nine cells. Each point remembers its line and its signed
/// arc from the seed, so a line's own recent points do not stop it.
struct Occupancy {
    cell: f64,
    points: HashMap<(i64, i64), Vec<Placed>>,
}

/// A placed point, its line, and its signed arc from the line's seed.
type Placed = (Point2, usize, f64);

impl Occupancy {
    fn new(cell: f64) -> Self {
        Self {
            cell,
            points: HashMap::new(),
        }
    }

    fn bucket(&self, point: Point2) -> (i64, i64) {
        (
            (point.x / self.cell).floor() as i64,
            (point.y / self.cell).floor() as i64,
        )
    }

    fn insert(&mut self, point: Point2, line: usize, arc: f64) {
        let bucket = self.bucket(point);
        self.points
            .entry(bucket)
            .or_default()
            .push((point, line, arc));
    }

    /// Whether a placed point lies within `radius` of `point`. With `own`,
    /// the line's points within `exclusion` of arc from `arc` do not count,
    /// since the line's own last steps are always that close.
    fn within(&self, point: Point2, radius: f64, own: Option<(usize, f64, f64)>) -> bool {
        let (column, row) = self.bucket(point);
        let radius_squared = radius * radius;
        for column in column - 1..=column + 1 {
            for row in row - 1..=row + 1 {
                let Some(points) = self.points.get(&(column, row)) else {
                    continue;
                };
                for (placed, line, placed_arc) in points {
                    if let Some((own_line, arc, exclusion)) = own
                        && *line == own_line
                        && (placed_arc - arc).abs() < exclusion
                    {
                        continue;
                    }
                    let offset = *placed - point;
                    if offset.dot(offset) < radius_squared {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// The arc intervals of a line's dashes: a pattern of `on` every `period`,
/// anchored at `anchor` (the seed's arc, so a line whose extent changes
/// keeps its dashes where they were) and shifted along the flow by `phase`,
/// clipped to the line's `length`.
pub(super) fn dash_intervals(
    length: f64,
    anchor: f64,
    phase: f64,
    on: f64,
    period: f64,
) -> Vec<(f64, f64)> {
    if !(length > 0.0 && on > 0.0 && period > 0.0 && anchor.is_finite() && phase.is_finite()) {
        return vec![];
    }
    let start = anchor + phase.rem_euclid(period);
    let first = ((-on - start) / period).ceil() as i64;
    let last = ((length - start) / period).floor() as i64;
    (first..=last)
        .filter_map(|index| {
            let from = start + index as f64 * period;
            let (from, to) = (from.max(0.0), (from + on).min(length));
            (to > from).then_some((from, to))
        })
        .collect()
}

/// The polyline between two arcs of a line, the vertices between them kept.
pub(super) fn section(points: &[Point2], arcs: &[f64], from: f64, to: f64) -> Vec<Point2> {
    let mut out = Vec::new();
    if points.len() < 2
        || arcs.len() != points.len()
        || !to.is_finite()
        || !from.is_finite()
        || to <= from
    {
        return out;
    }
    let at = |arc: f64| -> Point2 {
        let index = arcs
            .partition_point(|value| *value <= arc)
            .clamp(1, arcs.len() - 1);
        let span = arcs[index] - arcs[index - 1];
        let t = if span > 0.0 {
            ((arc - arcs[index - 1]) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        points[index - 1].lerp(points[index], t)
    };
    out.push(at(from));
    for (point, arc) in points.iter().zip(arcs) {
        if *arc > from && *arc < to {
            out.push(*point);
        }
    }
    out.push(at(to));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lattice(
        spacing: f64,
        columns: i64,
        rows: i64,
        field: impl Fn(Point2) -> Option<Point2>,
    ) -> VectorLattice {
        let field = &field;
        let samples = (0..columns).flat_map(|column| {
            (0..rows).filter_map(move |row| {
                let center = Point2::new(column as f64 + 0.5, row as f64 + 0.5) * spacing;
                field(center).map(|value| (center, value))
            })
        });
        VectorLattice::new(spacing, samples).unwrap()
    }

    fn parameters(separation: f64) -> StreamlineParameters {
        StreamlineParameters {
            separation,
            step: separation / 8.0,
            floor: 0.0,
            maximum_length: 1.0e3,
        }
    }

    fn view(columns: i64, rows: i64, spacing: f64) -> WorldRect {
        WorldRect {
            min: Point2::default(),
            max: Point2::new(columns as f64, rows as f64) * spacing,
        }
    }

    fn all_points(lines: &Streamlines) -> Vec<(usize, Point2)> {
        lines
            .lines
            .iter()
            .enumerate()
            .flat_map(|(index, line)| line.points.iter().map(move |point| (index, *point)))
            .collect()
    }

    #[test]
    fn the_lattice_reads_bilinearly_and_ends_where_its_samples_do() {
        let lattice = lattice(1.0, 4, 4, |center| {
            Some(Point2::new(center.x, center.y * 2.0))
        });
        // A linear field is reproduced exactly between the centres.
        let at = lattice.sample(Point2::new(1.7, 2.2)).unwrap();
        assert!((at.x - 1.7).abs() < 1.0e-12 && (at.y - 4.4).abs() < 1.0e-12);
        // Up to half a cell past the last centre the nearest corners carry
        // the weight; further out there is nothing to read.
        assert!(lattice.sample(Point2::new(3.9, 2.0)).is_some());
        assert!(lattice.sample(Point2::new(4.1, 2.0)).is_none());
        assert!(lattice.sample(Point2::new(-0.1, 2.0)).is_none());
        assert!(lattice.sample(Point2::new(f64::NAN, 2.0)).is_none());
    }

    #[test]
    fn a_uniform_field_draws_straight_lines_an_even_separation_apart() {
        let spacing = 1.0;
        let (columns, rows) = (40, 30);
        let lattice = lattice(spacing, columns, rows, |_| Some(Point2::new(1.0, 0.0)));
        let parameters = parameters(3.0);
        let view = view(columns, rows, spacing);
        let mut lines = Streamlines::default();
        lines.place(&lattice, &parameters, &view);
        assert!(lines.lines.len() >= 8, "{} lines", lines.lines.len());
        let mut heights = Vec::new();
        for line in &lines.lines {
            let y = line.points[0].y;
            assert!(line.points.iter().all(|point| (point.y - y).abs() < 1.0e-9));
            assert!(
                line.points.windows(2).all(|pair| pair[1].x > pair[0].x),
                "ordered along the flow"
            );
            let length = line.points.last().unwrap().x - line.points[0].x;
            assert!(
                length > (columns as f64 - 2.0) * spacing,
                "spans the lattice: {length}"
            );
            assert_eq!(line.points[line.seed_index], line.seed);
            heights.push(y);
        }
        heights.sort_by(f64::total_cmp);
        for pair in heights.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(gap >= parameters.separation - 1.0e-9, "lines {gap} apart");
            assert!(
                gap <= 2.0 * parameters.separation + 1.0e-9,
                "a gap of {gap}"
            );
        }
    }

    #[test]
    fn a_rotating_field_draws_circles_that_close_on_themselves() {
        let spacing = 0.5;
        let (columns, rows) = (80, 80);
        let center = Point2::new(20.0, 20.0);
        let lattice = lattice(spacing, columns, rows, |at| {
            let offset = at - center;
            Some(Point2::new(-offset.y, offset.x))
        });
        let parameters = parameters(2.0);
        let mut lines = Streamlines::default();
        lines.place(&lattice, &parameters, &view(columns, rows, spacing));
        assert!(lines.lines.len() >= 5, "{} lines", lines.lines.len());
        let mut closed = 0;
        for line in &lines.lines {
            let radius = (line.seed - center).norm();
            if radius < parameters.separation {
                continue;
            }
            for point in &line.points {
                let error = ((*point - center).norm() - radius).abs() / radius;
                assert!(
                    error < 2.0e-3,
                    "radius error {error} on a circle of {radius}"
                );
            }
            // A circle that fits the lattice closes: the two halves meet
            // across from the seed, so the line is as long as the circle
            // less the test distance, and no longer. One that leaves the
            // lattice is an arc.
            if radius > 20.0 - 2.0 * spacing {
                continue;
            }
            let arcs = line.cumulative_arcs();
            let length = *arcs.last().unwrap();
            let circumference = std::f64::consts::TAU * radius;
            assert!(
                length > circumference - 2.0 * parameters.separation
                    && length < circumference + 1.0e-6,
                "length {length} on a circumference of {circumference}"
            );
            closed += 1;
        }
        assert!(closed >= 4, "{closed} closed circles");
    }

    #[test]
    fn no_two_lines_come_closer_than_the_test_distance() {
        let spacing = 1.0;
        let (columns, rows) = (40, 40);
        let lattice = lattice(spacing, columns, rows, |at| {
            Some(Point2::new(1.0, (at.x * 0.3).sin() * 0.8))
        });
        let parameters = parameters(3.0);
        let mut lines = Streamlines::default();
        lines.place(&lattice, &parameters, &view(columns, rows, spacing));
        let points = all_points(&lines);
        assert!(points.len() > 100);
        let allowed = parameters.test_distance() - parameters.step;
        for (i, (line, point)) in points.iter().enumerate() {
            for (other_line, other) in &points[i + 1..] {
                if line == other_line {
                    continue;
                }
                let distance = (*point - *other).norm();
                assert!(
                    distance >= allowed,
                    "lines {line} and {other_line} are {distance} apart"
                );
            }
        }
    }

    #[test]
    fn lines_stop_where_the_field_is_quiet_or_missing() {
        let spacing = 1.0;
        let (columns, rows) = (40, 20);
        let hole = WorldRect {
            min: Point2::new(10.0, 5.0),
            max: Point2::new(15.0, 12.0),
        };
        let lattice = lattice(spacing, columns, rows, |at| {
            if hole.contains(at) {
                None
            } else if at.x > 30.0 {
                Some(Point2::new(1.0e-6, 0.0))
            } else {
                Some(Point2::new(1.0, 0.0))
            }
        });
        let parameters = StreamlineParameters {
            floor: 1.0e-2,
            ..parameters(2.0)
        };
        let mut lines = Streamlines::default();
        lines.place(&lattice, &parameters, &view(columns, rows, spacing));
        assert!(lines.lines.len() >= 5);
        for (_, point) in all_points(&lines) {
            assert!(
                point.x < 31.0,
                "a line ran into the quiet half at {point:?}"
            );
            let inside = point.x > hole.min.x + spacing
                && point.x < hole.max.x - spacing
                && point.y > hole.min.y + spacing
                && point.y < hole.max.y - spacing;
            assert!(!inside, "a line crossed the hole at {point:?}");
        }
    }

    #[test]
    fn the_same_field_places_the_same_lines_again_from_the_carried_seeds() {
        let spacing = 1.0;
        let (columns, rows) = (40, 30);
        let lattice = lattice(spacing, columns, rows, |at| {
            Some(Point2::new(1.0, (at.y * 0.2).cos() * 0.5))
        });
        let parameters = parameters(3.0);
        let view = view(columns, rows, spacing);
        let mut lines = Streamlines::default();
        lines.place(&lattice, &parameters, &view);
        let first = lines.lines.clone();
        lines.place(&lattice, &parameters, &view);
        assert_eq!(lines.lines, first);

        // A slightly changed field keeps the seeds, and so the lines'
        // identity: each line still starts where it did.
        let drifted = self::lattice(spacing, columns, rows, |at| {
            Some(Point2::new(1.0, (at.y * 0.2 + 0.05).cos() * 0.5))
        });
        lines.place(&drifted, &parameters, &view);
        let kept = lines
            .lines
            .iter()
            .filter(|line| first.iter().any(|before| before.seed == line.seed))
            .count();
        assert!(
            kept * 10 >= first.len() * 9,
            "{kept} of {} seeds kept",
            first.len()
        );
    }

    #[test]
    fn dashes_are_anchored_at_the_seed_and_move_with_the_phase() {
        let length = 10.0;
        let (on, period) = (1.0, 2.0);
        let still = dash_intervals(length, 3.0, 0.0, on, period);
        assert_eq!(
            still,
            vec![(1.0, 2.0), (3.0, 4.0), (5.0, 6.0), (7.0, 8.0), (9.0, 10.0)]
        );
        let moved = dash_intervals(length, 3.0, 0.5, on, period);
        assert_eq!(moved[0], (0.0, 0.5));
        assert_eq!(moved[1], (1.5, 2.5));
        // The anchor shifts the whole pattern, the phase wraps.
        assert_eq!(
            dash_intervals(length, 4.0, 0.0, on, period),
            dash_intervals(length, 2.0, 0.0, on, period)
        );
        assert_eq!(dash_intervals(length, 3.0, 2.5, on, period), moved);
        assert!(dash_intervals(0.0, 3.0, 0.0, on, period).is_empty());
    }

    #[test]
    fn a_section_keeps_the_vertices_between_its_ends() {
        let points = vec![
            Point2::new(0.0, 0.0),
            Point2::new(1.0, 0.0),
            Point2::new(1.0, 1.0),
            Point2::new(2.0, 1.0),
        ];
        let arcs = vec![0.0, 1.0, 2.0, 3.0];
        let piece = section(&points, &arcs, 0.5, 2.5);
        assert_eq!(
            piece,
            vec![
                Point2::new(0.5, 0.0),
                Point2::new(1.0, 0.0),
                Point2::new(1.0, 1.0),
                Point2::new(1.5, 1.0),
            ]
        );
        assert_eq!(
            section(&points, &arcs, 2.5, 3.0),
            vec![Point2::new(1.5, 1.0), Point2::new(2.0, 1.0)]
        );
        assert!(section(&points, &arcs, 2.0, 2.0).is_empty());
    }
}
