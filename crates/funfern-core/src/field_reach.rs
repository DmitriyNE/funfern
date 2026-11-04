//! How far each element of a mesh lies from the elements that hold a field,
//! for an estimate that keeps the mesh at its authored size wherever the
//! field can get before adaptation next acts ([`crate::FieldReach`]).
//!
//! The distance is read off an exact Euclidean distance transform of a square
//! raster over the mesh. A field-holding triangle marks every cell whose
//! centre lies within half a cell diagonal of it, so the cell holding any of
//! its points is marked; a triangle reads the least distance over the cells it
//! marks in the same way, less one cell diagonal. That is never more than the
//! true distance between the two triangles, nor less than it by more than two
//! cell diagonals, so the reach it bounds can only err towards holding the
//! mesh. Walls are not seen: a field behind one counts as near, which errs the
//! same way.

use crate::{Point2, point_segment_distance};

/// Cells across the raster's longer side. In the default box of width two a
/// cell diagonal is `0.011`, against reaches from `0.03` at the slowest speed
/// ceiling.
pub(crate) const FIELD_REACH_RASTER_CELLS: usize = 256;

/// A raster over a mesh's bounding box: cells marked where the field is, then
/// each cell's distance to the nearest marked one.
#[derive(Clone, Debug)]
pub(crate) struct FieldReachRaster {
    origin: Point2,
    cell: f64,
    columns: usize,
    rows: usize,
    /// While marking, zero at a marked cell and infinity elsewhere; after
    /// every line is transformed, each cell's squared distance in cells to the
    /// nearest marked cell, row by row.
    squared: Vec<f64>,
    /// One line's values, and its envelope's sites, heights and starts.
    line: Vec<f64>,
    sites: Vec<usize>,
    heights: Vec<f64>,
    starts: Vec<f64>,
}

impl FieldReachRaster {
    /// A raster of square cells over the box from `low` to `high`,
    /// [`FIELD_REACH_RASTER_CELLS`] across its longer side.
    pub(crate) fn new(low: Point2, high: Point2) -> Self {
        let width = (high.x - low.x).max(0.0);
        let height = (high.y - low.y).max(0.0);
        let side = width.max(height);
        let cell = if side > 0.0 {
            side / FIELD_REACH_RASTER_CELLS as f64
        } else {
            1.0
        };
        let columns = ((width / cell).ceil() as usize).clamp(1, FIELD_REACH_RASTER_CELLS);
        let rows = ((height / cell).ceil() as usize).clamp(1, FIELD_REACH_RASTER_CELLS);
        Self {
            origin: low,
            cell,
            columns,
            rows,
            squared: vec![f64::INFINITY; columns * rows],
            line: Vec::with_capacity(columns.max(rows)),
            sites: Vec::with_capacity(columns.max(rows)),
            heights: Vec::with_capacity(columns.max(rows)),
            starts: Vec::with_capacity(columns.max(rows)),
        }
    }

    /// A cell's diagonal, in the mesh's units.
    pub(crate) fn diagonal(&self) -> f64 {
        self.cell * std::f64::consts::SQRT_2
    }

    /// The transform's lines: every column, then every row.
    pub(crate) fn lines(&self) -> usize {
        self.columns + self.rows
    }

    /// The cells whose centres lie within half a cell diagonal of `triangle`,
    /// which hold every point of it.
    fn cells_of(&self, triangle: [Point2; 3]) -> impl Iterator<Item = usize> + '_ {
        let reach = 0.5 * self.diagonal();
        let low_x = triangle
            .iter()
            .map(|point| point.x)
            .fold(f64::INFINITY, f64::min);
        let high_x = triangle
            .iter()
            .map(|point| point.x)
            .fold(f64::NEG_INFINITY, f64::max);
        let low_y = triangle
            .iter()
            .map(|point| point.y)
            .fold(f64::INFINITY, f64::min);
        let high_y = triangle
            .iter()
            .map(|point| point.y)
            .fold(f64::NEG_INFINITY, f64::max);
        let column = |x: f64| ((x - self.origin.x) / self.cell).floor();
        let row = |y: f64| ((y - self.origin.y) / self.cell).floor();
        let clamp = |index: f64, count: usize| index.clamp(0.0, count as f64 - 1.0) as usize;
        let columns = clamp(column(low_x - reach), self.columns)
            ..=clamp(column(high_x + reach), self.columns);
        let rows = clamp(row(low_y - reach), self.rows)..=clamp(row(high_y + reach), self.rows);
        rows.flat_map(move |row| columns.clone().map(move |column| (row, column)))
            .filter(move |(row, column)| {
                let centre = Point2::new(
                    self.origin.x + (*column as f64 + 0.5) * self.cell,
                    self.origin.y + (*row as f64 + 0.5) * self.cell,
                );
                point_triangle_distance(centre, triangle) <= reach
            })
            .map(|(row, column)| row * self.columns + column)
    }

    /// Marks the cells `triangle` holds: the field is there.
    pub(crate) fn mark(&mut self, triangle: [Point2; 3]) {
        let cells = self.cells_of(triangle).collect::<Vec<_>>();
        for cell in cells {
            self.squared[cell] = 0.0;
        }
    }

    /// Transforms one of [`Self::lines`]: the columns, along `y`, all before
    /// the rows, along `x`, as the separable transform takes them.
    pub(crate) fn transform_line(&mut self, line: usize) {
        let (count, stride, first) = if line < self.columns {
            (self.rows, self.columns, line)
        } else {
            (self.columns, 1, (line - self.columns) * self.columns)
        };
        self.line.clear();
        self.line
            .extend((0..count).map(|index| self.squared[first + index * stride]));
        squared_distance_transform(
            &mut self.line,
            &mut self.sites,
            &mut self.heights,
            &mut self.starts,
        );
        for (index, value) in self.line.iter().enumerate() {
            self.squared[first + index * stride] = *value;
        }
    }

    /// The distance from `triangle` to the nearest marked triangle once every
    /// line is transformed: never more than the true distance, nor less by
    /// more than two cell diagonals, and infinite when nothing is marked.
    pub(crate) fn distance(&self, triangle: [Point2; 3]) -> f64 {
        let nearest = self
            .cells_of(triangle)
            .map(|cell| self.squared[cell])
            .fold(f64::INFINITY, f64::min);
        if nearest.is_finite() {
            (nearest.sqrt() * self.cell - self.diagonal()).max(0.0)
        } else {
            f64::INFINITY
        }
    }
}

/// The distance from `point` to `triangle`, zero inside it.
pub(crate) fn point_triangle_distance(point: Point2, triangle: [Point2; 3]) -> f64 {
    let [a, b, c] = triangle;
    let side = |from: Point2, to: Point2| {
        let edge = to - from;
        let offset = point - from;
        edge.x * offset.y - edge.y * offset.x
    };
    let signs = [side(a, b), side(b, c), side(c, a)];
    if signs.iter().all(|sign| *sign >= 0.0) || signs.iter().all(|sign| *sign <= 0.0) {
        return 0.0;
    }
    point_segment_distance(point, a, b)
        .min(point_segment_distance(point, b, c))
        .min(point_segment_distance(point, c, a))
}

/// The squared distance transform of one line in place, Felzenszwalb and
/// Huttenlocher's lower envelope of the parabolas `(q − p)² + f(p)` over the
/// sites `p` where `f` is finite, sampled at every `q`. A line with no finite
/// value is left infinite. `sites`, `heights` and `starts` are scratch.
fn squared_distance_transform(
    values: &mut [f64],
    sites: &mut Vec<usize>,
    heights: &mut Vec<f64>,
    starts: &mut Vec<f64>,
) {
    sites.clear();
    heights.clear();
    starts.clear();
    for (site, height) in values.iter().copied().enumerate() {
        if !height.is_finite() {
            continue;
        }
        let p = site as f64;
        let mut start = f64::NEG_INFINITY;
        while let (Some(&last), Some(&last_height), Some(&last_start)) =
            (sites.last(), heights.last(), starts.last())
        {
            let q = last as f64;
            let crossing = ((height + p * p) - (last_height + q * q)) / (2.0 * (p - q));
            if crossing <= last_start {
                sites.pop();
                heights.pop();
                starts.pop();
            } else {
                start = crossing;
                break;
            }
        }
        sites.push(site);
        heights.push(height);
        starts.push(start);
    }
    if sites.is_empty() {
        return;
    }
    let mut parabola = 0;
    for (q, value) in values.iter_mut().enumerate() {
        while parabola + 1 < sites.len() && starts[parabola + 1] < q as f64 {
            parabola += 1;
        }
        let offset = q as f64 - sites[parabola] as f64;
        *value = offset * offset + heights[parabola];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic stream in `[0, 1)`.
    struct Stream(u64);

    impl Stream {
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    fn crosses(a: [Point2; 2], b: [Point2; 2]) -> bool {
        let side = |from: Point2, to: Point2, point: Point2| {
            let edge = to - from;
            let offset = point - from;
            edge.x * offset.y - edge.y * offset.x
        };
        side(a[0], a[1], b[0]) * side(a[0], a[1], b[1]) < 0.0
            && side(b[0], b[1], a[0]) * side(b[0], b[1], a[1]) < 0.0
    }

    /// The distance between two triangles: zero where they meet, otherwise
    /// the least from a vertex of either to the other.
    fn triangle_distance(a: [Point2; 3], b: [Point2; 3]) -> f64 {
        let edges = |t: [Point2; 3]| [[t[0], t[1]], [t[1], t[2]], [t[2], t[0]]];
        if edges(a)
            .iter()
            .any(|edge| edges(b).iter().any(|other| crosses(*edge, *other)))
        {
            return 0.0;
        }
        a.iter()
            .map(|point| point_triangle_distance(*point, b))
            .chain(b.iter().map(|point| point_triangle_distance(*point, a)))
            .fold(f64::INFINITY, f64::min)
    }

    #[test]
    fn a_lines_transform_is_its_nearest_finite_parabola() {
        let mut stream = Stream(7);
        let (mut sites, mut heights, mut starts) = (Vec::new(), Vec::new(), Vec::new());
        for length in [1, 2, 3, 17, 256] {
            for _ in 0..50 {
                let input = (0..length)
                    .map(|_| {
                        if stream.next() < 0.7 {
                            f64::INFINITY
                        } else {
                            (stream.next() * 40.0).floor()
                        }
                    })
                    .collect::<Vec<_>>();
                let mut output = input.clone();
                squared_distance_transform(&mut output, &mut sites, &mut heights, &mut starts);
                for (q, value) in output.iter().enumerate() {
                    let expected = input
                        .iter()
                        .enumerate()
                        .filter(|(_, height)| height.is_finite())
                        .map(|(p, height)| (q as f64 - p as f64).powi(2) + height)
                        .fold(f64::INFINITY, f64::min);
                    assert_eq!(*value, expected, "{input:?} at {q}");
                }
            }
        }
    }

    #[test]
    fn a_distance_is_never_above_the_truth_nor_two_diagonals_below_it() {
        let mut stream = Stream(11);
        let low = Point2::new(-1.0, -0.6);
        let high = Point2::new(1.0, 0.6);
        // Inside the raster's box, as a mesh's triangles are inside its own.
        let mut triangle = |size: f64| {
            let centre = Point2::new(
                low.x + 0.5 * size + (high.x - low.x - size) * stream.next(),
                low.y + 0.5 * size + (high.y - low.y - size) * stream.next(),
            );
            let mut vertex = || {
                Point2::new(
                    centre.x + size * (stream.next() - 0.5),
                    centre.y + size * (stream.next() - 0.5),
                )
            };
            [vertex(), vertex(), vertex()]
        };
        for (marked_count, size) in [(1, 0.05), (3, 0.2), (12, 0.02), (40, 0.1)] {
            let marked = (0..marked_count)
                .map(|_| triangle(size))
                .collect::<Vec<_>>();
            let queried = (0..200).map(|_| triangle(size)).collect::<Vec<_>>();
            let mut raster = FieldReachRaster::new(low, high);
            for held in &marked {
                raster.mark(*held);
            }
            for line in 0..raster.lines() {
                raster.transform_line(line);
            }
            for query in &queried {
                let truth = marked
                    .iter()
                    .map(|held| triangle_distance(*query, *held))
                    .fold(f64::INFINITY, f64::min);
                let read = raster.distance(*query);
                assert!(read <= truth + 1e-12, "{read} above {truth}");
                assert!(
                    read >= truth - 2.0 * raster.diagonal() - 1e-12,
                    "{read} more than two diagonals under {truth}"
                );
            }
        }
    }

    #[test]
    fn nothing_marked_is_out_of_every_reach() {
        let mut raster = FieldReachRaster::new(Point2::new(0.0, 0.0), Point2::new(1.0, 1.0));
        for line in 0..raster.lines() {
            raster.transform_line(line);
        }
        let triangle = [
            Point2::new(0.1, 0.1),
            Point2::new(0.2, 0.1),
            Point2::new(0.1, 0.2),
        ];
        assert_eq!(raster.distance(triangle), f64::INFINITY);
    }
}
