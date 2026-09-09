use std::ops::{Add, Div, Mul, Sub};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point2 {
    pub x: f64,
    pub y: f64,
}
impl Point2 {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    pub fn dot(self, b: Self) -> f64 {
        self.x * b.x + self.y * b.y
    }
    pub fn cross(self, b: Self) -> f64 {
        self.x * b.y - self.y * b.x
    }
    pub fn norm(self) -> f64 {
        self.x.hypot(self.y)
    }
    pub fn finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
    pub fn lerp(self, b: Self, t: f64) -> Self {
        self * (1.0 - t) + b * t
    }
}
impl Add for Point2 {
    type Output = Self;
    fn add(self, b: Self) -> Self {
        Self::new(self.x + b.x, self.y + b.y)
    }
}
impl Sub for Point2 {
    type Output = Self;
    fn sub(self, b: Self) -> Self {
        Self::new(self.x - b.x, self.y - b.y)
    }
}
impl Mul<f64> for Point2 {
    type Output = Self;
    fn mul(self, b: f64) -> Self {
        Self::new(self.x * b, self.y * b)
    }
}
impl Div<f64> for Point2 {
    type Output = Self;
    fn div(self, b: f64) -> Self {
        self * (1.0 / b)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SplineError {
    ControlCount,
    IntervalCount,
    NonFinite,
    InvalidInterval,
    IllConditionedKnots,
    Index,
}
impl std::fmt::Display for SplineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SplineError {}

/// P_i is associated with knot t_i. Knots and controls extend periodically.
#[derive(Clone, Debug, PartialEq)]
pub struct PeriodicCubicSpline {
    controls: Vec<Point2>,
    intervals: Vec<f64>,
    knots: Vec<f64>,
}

/// A clamped, nonuniform cubic B-spline. Unlike [`PeriodicCubicSpline`], its
/// parameter interval has two distinct endpoints and its first and last control
/// points lie on the curve.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenCubicSpline {
    controls: Vec<Point2>,
    intervals: Vec<f64>,
    knots: Vec<f64>,
}

impl OpenCubicSpline {
    /// `intervals` stores the positive lengths of the `controls.len() - 3`
    /// nonempty knot spans. Endpoint knots have cubic multiplicity four.
    pub fn new(controls: Vec<Point2>, intervals: Vec<f64>) -> Result<Self, SplineError> {
        if controls.len() < 4 || controls.len() > 128 {
            return Err(SplineError::ControlCount);
        }
        if intervals.len() != controls.len() - 3 {
            return Err(SplineError::IntervalCount);
        }
        if controls.iter().any(|point| !point.finite()) {
            return Err(SplineError::NonFinite);
        }
        if intervals
            .iter()
            .any(|interval| !interval.is_finite() || *interval <= 0.0)
        {
            return Err(SplineError::InvalidInterval);
        }
        let period: f64 = intervals.iter().sum();
        if !period.is_finite() || intervals.iter().any(|value| value / period < 1.0e-12) {
            return Err(SplineError::IllConditionedKnots);
        }
        let mut knots = vec![0.0; 4];
        let mut knot = 0.0;
        for interval in intervals.iter().take(intervals.len() - 1) {
            knot += interval;
            knots.push(knot);
        }
        knots.extend([period; 4]);
        Ok(Self {
            controls,
            intervals,
            knots,
        })
    }

    pub fn uniform(controls: Vec<Point2>) -> Result<Self, SplineError> {
        let intervals = vec![1.0; controls.len().saturating_sub(3)];
        Self::new(controls, intervals)
    }

    pub fn controls(&self) -> &[Point2] {
        &self.controls
    }

    pub fn intervals(&self) -> &[f64] {
        &self.intervals
    }

    pub fn knots(&self) -> &[f64] {
        &self.knots
    }

    pub fn period(&self) -> f64 {
        *self.knots.last().unwrap()
    }

    pub fn set_control(&mut self, index: usize, point: Point2) -> Result<(), SplineError> {
        if !point.finite() {
            return Err(SplineError::NonFinite);
        }
        *self.controls.get_mut(index).ok_or(SplineError::Index)? = point;
        Ok(())
    }

    pub fn evaluate(&self, parameter: f64) -> Point2 {
        self.derivative(parameter, 0)
    }

    pub fn derivative(&self, parameter: f64, order: usize) -> Point2 {
        assert!(order <= 2 && parameter.is_finite());
        let mut controls = self.controls.clone();
        let mut knots = self.knots.clone();
        let mut degree = 3;
        for _ in 0..order {
            controls = controls
                .windows(2)
                .enumerate()
                .map(|(index, pair)| {
                    (pair[1] - pair[0])
                        * (degree as f64 / (knots[index + degree + 1] - knots[index + 1]))
                })
                .collect();
            knots = knots[1..knots.len() - 1].to_vec();
            degree -= 1;
        }
        de_boor_open(
            &controls,
            &knots,
            degree,
            parameter.clamp(0.0, self.period()),
        )
    }

    /// Inserts one interior knot without changing the curve.
    pub fn insert(&mut self, parameter: f64) -> Result<Insertion, SplineError> {
        if !parameter.is_finite() {
            return Err(SplineError::NonFinite);
        }
        let end = self.period();
        let tolerance = end * 1.0e-10;
        if parameter <= tolerance {
            return Ok(Insertion::Existing(0));
        }
        if parameter >= end - tolerance {
            return Ok(Insertion::Existing(self.controls.len() - 1));
        }
        if self.controls.len() == 128 {
            return Err(SplineError::ControlCount);
        }
        if let Some(knot) = self
            .knots
            .iter()
            .position(|knot| (parameter - knot).abs() <= tolerance)
        {
            return Ok(Insertion::Existing(
                knot.saturating_sub(2).min(self.controls.len() - 1),
            ));
        }

        let degree = 3;
        let last_control = self.controls.len() - 1;
        let span = self
            .knots
            .windows(2)
            .position(|pair| pair[0] <= parameter && parameter < pair[1])
            .ok_or(SplineError::InvalidInterval)?;
        let mut controls = vec![Point2::default(); self.controls.len() + 1];
        controls[..=span - degree].copy_from_slice(&self.controls[..=span - degree]);
        controls[span + 1..=last_control + 1].copy_from_slice(&self.controls[span..=last_control]);
        for (index, control) in controls
            .iter_mut()
            .enumerate()
            .take(span + 1)
            .skip(span - degree + 1)
        {
            let alpha =
                (parameter - self.knots[index]) / (self.knots[index + degree] - self.knots[index]);
            *control = self.controls[index - 1].lerp(self.controls[index], alpha);
        }
        let mut intervals = self.intervals.clone();
        let breakpoint = self
            .knots
            .iter()
            .take(span + 1)
            .copied()
            .fold(0.0, f64::max);
        let interval = self
            .knots
            .iter()
            .skip(span + 1)
            .copied()
            .find(|knot| *knot > breakpoint)
            .ok_or(SplineError::InvalidInterval)?;
        let interval_index = self
            .intervals
            .iter()
            .scan(0.0, |sum, value| {
                let start = *sum;
                *sum += value;
                Some((start, *sum))
            })
            .position(|(start, finish)| start <= parameter && parameter < finish)
            .ok_or(SplineError::InvalidInterval)?;
        intervals[interval_index] = parameter - breakpoint;
        intervals.insert(interval_index + 1, interval - parameter);
        *self = Self::new(controls, intervals)?;
        Ok(Insertion::Inserted(span - degree + 1))
    }

    /// Deletes one control and one nonempty knot span. This is an editing
    /// operation and may reshape the curve.
    pub fn remove(&mut self, index: usize) -> Result<(), SplineError> {
        if self.controls.len() <= 4 {
            return Err(SplineError::ControlCount);
        }
        if index >= self.controls.len() {
            return Err(SplineError::Index);
        }
        let mut controls = self.controls.clone();
        controls.remove(index);
        let mut intervals = self.intervals.clone();
        if index <= 1 {
            intervals.remove(0);
        } else if index + 2 >= self.controls.len() {
            intervals.pop();
        } else {
            let left = (index - 2).min(intervals.len() - 2);
            intervals[left] += intervals[left + 1];
            intervals.remove(left + 1);
        }
        *self = Self::new(controls, intervals)?;
        Ok(())
    }
}

fn de_boor_open(controls: &[Point2], knots: &[f64], degree: usize, parameter: f64) -> Point2 {
    let last_control = controls.len() - 1;
    let span = if parameter == *knots.last().unwrap() {
        last_control
    } else {
        knots
            .windows(2)
            .position(|pair| pair[0] <= parameter && parameter < pair[1])
            .unwrap()
            .clamp(degree, last_control)
    };
    let mut values = vec![Point2::default(); degree + 1];
    for (local, value) in values.iter_mut().enumerate() {
        *value = controls[span - degree + local];
    }
    for level in 1..=degree {
        for local in (level..=degree).rev() {
            let index = span - degree + local;
            let alpha =
                (parameter - knots[index]) / (knots[index + degree + 1 - level] - knots[index]);
            values[local] = values[local - 1].lerp(values[local], alpha);
        }
    }
    values[degree]
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Insertion {
    Inserted(usize),
    Existing(usize),
}
impl PeriodicCubicSpline {
    pub fn new(controls: Vec<Point2>, intervals: Vec<f64>) -> Result<Self, SplineError> {
        if controls.len() < 4 || controls.len() > 128 {
            return Err(SplineError::ControlCount);
        }
        if intervals.len() != controls.len() {
            return Err(SplineError::IntervalCount);
        }
        if controls.iter().any(|p| !p.finite()) {
            return Err(SplineError::NonFinite);
        }
        if intervals.iter().any(|v| !v.is_finite() || *v <= 0.0) {
            return Err(SplineError::InvalidInterval);
        }
        let period: f64 = intervals.iter().sum();
        if !period.is_finite() || intervals.iter().any(|v| v / period < 1e-12) {
            return Err(SplineError::IllConditionedKnots);
        }
        let mut knots = vec![0.0];
        for v in &intervals {
            knots.push(knots.last().unwrap() + v);
        }
        Ok(Self {
            controls,
            intervals,
            knots,
        })
    }
    pub fn uniform(controls: Vec<Point2>) -> Result<Self, SplineError> {
        let n = controls.len();
        Self::new(controls, vec![1.0; n])
    }
    pub fn rounded(center: Point2, radius: f64) -> Self {
        Self::uniform(
            (0..8)
                .map(|i| {
                    let a = i as f64 * std::f64::consts::TAU / 8.0;
                    center + Point2::new(a.cos(), a.sin()) * radius
                })
                .collect(),
        )
        .unwrap()
    }
    pub fn controls(&self) -> &[Point2] {
        &self.controls
    }
    pub fn intervals(&self) -> &[f64] {
        &self.intervals
    }
    pub fn knots(&self) -> &[f64] {
        &self.knots
    }
    pub fn period(&self) -> f64 {
        *self.knots.last().unwrap()
    }
    fn knot(&self, i: isize) -> f64 {
        let n = self.controls.len() as isize;
        self.knots[i.rem_euclid(n) as usize] + i.div_euclid(n) as f64 * self.period()
    }
    fn control(&self, i: isize) -> Point2 {
        self.controls[i.rem_euclid(self.controls.len() as isize) as usize]
    }
    pub fn set_control(&mut self, i: usize, p: Point2) -> Result<(), SplineError> {
        if !p.finite() {
            return Err(SplineError::NonFinite);
        }
        *self.controls.get_mut(i).ok_or(SplineError::Index)? = p;
        Ok(())
    }
    fn derivative_control(&self, i: isize, order: usize) -> Point2 {
        if order == 0 {
            return self.control(i);
        }
        (self.derivative_control(i + 1, order - 1) - self.derivative_control(i, order - 1))
            * ((4 - order) as f64 / (self.knot(i + 4) - self.knot(i + order as isize)))
    }
    /// Periodic de Boor, including the differentiated knot/control sequences.
    pub fn derivative(&self, t: f64, order: usize) -> Point2 {
        assert!(order <= 2 && t.is_finite());
        let t = t.rem_euclid(self.period());
        let span = self.knots.partition_point(|k| *k <= t).saturating_sub(1) as isize;
        let degree = 3 - order;
        let k = span - order as isize;
        let mut d = [Point2::default(); 4];
        for (j, item) in d.iter_mut().enumerate().take(degree + 1) {
            *item = self.derivative_control(k - degree as isize + j as isize, order);
        }
        for r in 1..=degree {
            for j in (r..=degree).rev() {
                let i = k - degree as isize + j as isize;
                let a = self.knot(i + order as isize);
                let b = self.knot(i + degree as isize + 1 - r as isize + order as isize);
                d[j] = d[j - 1].lerp(d[j], (t - a) / (b - a));
            }
        }
        d[degree]
    }
    pub fn evaluate(&self, t: f64) -> Point2 {
        self.derivative(t, 0)
    }
    pub fn insert(&mut self, t: f64) -> Result<Insertion, SplineError> {
        if !t.is_finite() {
            return Err(SplineError::NonFinite);
        }
        let t = t.rem_euclid(self.period());
        for (i, k) in self.knots.iter().enumerate() {
            if (t - k).abs() <= self.period() * 1e-10 {
                return Ok(Insertion::Existing(i % self.controls.len()));
            }
        }
        if self.controls.len() == 128 {
            return Err(SplineError::ControlCount);
        }
        let k = self.knots.partition_point(|v| *v < t) - 1;
        let n = self.controls.len() + 1;
        let mut controls = vec![Point2::default(); n];
        // Choose a full period whose first three controls straddle the insertion.
        // This also updates wrapped controls when the insertion is near t=0.
        for j in (k as isize - 2)..(k as isize - 2 + n as isize) {
            controls[j.rem_euclid(n as isize) as usize] = if j <= k as isize {
                let alpha = (t - self.knot(j)) / (self.knot(j + 3) - self.knot(j));
                self.control(j - 1).lerp(self.control(j), alpha)
            } else {
                self.control(j - 1)
            };
        }
        let mut intervals = self.intervals.clone();
        intervals[k] = t - self.knots[k];
        intervals.insert(k + 1, self.knots[k + 1] - t);
        *self = Self::new(controls, intervals)?;
        Ok(Insertion::Inserted(k + 1))
    }
    /// Deletes P_i and t_i, merging the intervals on either side. Reshapes the curve.
    pub fn remove(&mut self, i: usize) -> Result<(), SplineError> {
        if self.controls.len() <= 4 {
            return Err(SplineError::ControlCount);
        }
        if i >= self.controls.len() {
            return Err(SplineError::Index);
        }
        let mut controls = self.controls.clone();
        let mut intervals = self.intervals.clone();
        let prev = (i + intervals.len() - 1) % intervals.len();
        intervals[prev] += intervals[i];
        controls.remove(i);
        intervals.remove(i);
        *self = Self::new(controls, intervals)?;
        Ok(())
    }
}

pub fn point_segment_distance(p: Point2, a: Point2, b: Point2) -> f64 {
    let v = b - a;
    let t = ((p - a).dot(v) / v.dot(v)).clamp(0.0, 1.0);
    if v.dot(v) == 0.0 {
        (p - a).norm()
    } else {
        (p - a - v * t).norm()
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub t: f64,
    pub point: Point2,
}
#[derive(Clone, Copy, Debug)]
pub struct SamplingOptions {
    pub tolerance: f64,
    pub max_depth: u8,
    pub max_points: usize,
}
impl Default for SamplingOptions {
    fn default() -> Self {
        Self {
            tolerance: 0.000025,
            max_depth: 16,
            max_points: 4096,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SamplingError {
    Exhausted,
    NonFinite,
}
#[derive(Clone)]
struct Span {
    a: f64,
    b: f64,
    depth: u8,
}
/// Resumable subdivision. Each step examines one cubic span using its exact
/// Bezier control hull, avoiding the missed-inflection failure of midpoint tests.
pub struct Sampler {
    spline: PeriodicCubicSpline,
    options: SamplingOptions,
    stack: Vec<Span>,
    samples: Vec<Sample>,
    error: Option<SamplingError>,
}
impl Sampler {
    pub fn new(spline: &PeriodicCubicSpline, options: SamplingOptions) -> Self {
        let stack = spline
            .knots
            .windows(2)
            .rev()
            .map(|k| Span {
                a: k[0],
                b: k[1],
                depth: 0,
            })
            .collect();
        Self {
            spline: spline.clone(),
            options,
            stack,
            samples: vec![Sample {
                t: 0.0,
                point: spline.evaluate(0.0),
            }],
            error: None,
        }
    }
    pub fn step(&mut self) -> bool {
        if self.error.is_some() {
            return true;
        }
        let Some(s) = self.stack.pop() else {
            return true;
        };
        let a = self.spline.evaluate(s.a);
        let b = self.spline.evaluate(s.b);
        let c = a + self.spline.derivative(s.a, 1) * ((s.b - s.a) / 3.0);
        let d = b - self.spline.derivative(s.b, 1) * ((s.b - s.a) / 3.0);
        if ![a, b, c, d].iter().all(|p| p.finite())
            || !self.options.tolerance.is_finite()
            || self.options.tolerance <= 0.0
        {
            self.error = Some(SamplingError::NonFinite);
            return true;
        }
        let chord = b - a;
        let roundoff = 32.0 * f64::EPSILON * chord.dot(chord);
        // A narrow hull alone can hide a tiny reversal or loop. Require
        // monotone projection of the Bezier controls onto the chord as well.
        let monotone = (c - a).dot(chord) >= -roundoff
            && (d - c).dot(chord) >= -roundoff
            && (b - d).dot(chord) >= -roundoff;
        let flat = monotone
            && point_segment_distance(c, a, b).max(point_segment_distance(d, a, b))
                <= self.options.tolerance;
        if flat {
            if self.samples.len() >= self.options.max_points {
                self.error = Some(SamplingError::Exhausted);
                return true;
            }
            self.samples.push(Sample { t: s.b, point: b });
        } else if s.depth >= self.options.max_depth
            || self.samples.len() + self.stack.len() + 2 > self.options.max_points
        {
            self.error = Some(SamplingError::Exhausted);
            return true;
        } else {
            let m = (s.a + s.b) * 0.5;
            self.stack.push(Span {
                a: m,
                b: s.b,
                depth: s.depth + 1,
            });
            self.stack.push(Span {
                a: s.a,
                b: m,
                depth: s.depth + 1,
            });
        }
        self.stack.is_empty()
    }
    pub fn finish(self) -> Result<Vec<Sample>, SamplingError> {
        if let Some(e) = self.error {
            Err(e)
        } else if !self.stack.is_empty() {
            Err(SamplingError::Exhausted)
        } else {
            Ok(self.samples)
        }
    }
}
pub fn sample(
    spline: &PeriodicCubicSpline,
    options: SamplingOptions,
) -> Result<Vec<Sample>, SamplingError> {
    let mut s = Sampler::new(spline, options);
    while !s.step() {}
    s.finish()
}

/// Resumable adaptive subdivision for a clamped open cubic spline.
pub struct OpenSampler {
    spline: OpenCubicSpline,
    options: SamplingOptions,
    stack: Vec<Span>,
    samples: Vec<Sample>,
    error: Option<SamplingError>,
}

impl OpenSampler {
    pub fn new(spline: &OpenCubicSpline, options: SamplingOptions) -> Self {
        let mut start = 0.0;
        let mut spans = Vec::with_capacity(spline.intervals.len());
        for interval in &spline.intervals {
            spans.push(Span {
                a: start,
                b: start + interval,
                depth: 0,
            });
            start += interval;
        }
        spans.reverse();
        Self {
            spline: spline.clone(),
            options,
            stack: spans,
            samples: vec![Sample {
                t: 0.0,
                point: spline.evaluate(0.0),
            }],
            error: None,
        }
    }

    pub fn step(&mut self) -> bool {
        if self.error.is_some() {
            return true;
        }
        let Some(span) = self.stack.pop() else {
            return true;
        };
        let a = self.spline.evaluate(span.a);
        let b = self.spline.evaluate(span.b);
        let c = a + self.spline.derivative(span.a, 1) * ((span.b - span.a) / 3.0);
        let d = b - self.spline.derivative(span.b, 1) * ((span.b - span.a) / 3.0);
        if ![a, b, c, d].iter().all(|point| point.finite())
            || !self.options.tolerance.is_finite()
            || self.options.tolerance <= 0.0
        {
            self.error = Some(SamplingError::NonFinite);
            return true;
        }
        let chord = b - a;
        let roundoff = 32.0 * f64::EPSILON * chord.dot(chord);
        let monotone = (c - a).dot(chord) >= -roundoff
            && (d - c).dot(chord) >= -roundoff
            && (b - d).dot(chord) >= -roundoff;
        let flat = monotone
            && point_segment_distance(c, a, b).max(point_segment_distance(d, a, b))
                <= self.options.tolerance;
        if flat {
            if self.samples.len() >= self.options.max_points {
                self.error = Some(SamplingError::Exhausted);
                return true;
            }
            self.samples.push(Sample {
                t: span.b,
                point: b,
            });
        } else if span.depth >= self.options.max_depth
            || self.samples.len() + self.stack.len() + 2 > self.options.max_points
        {
            self.error = Some(SamplingError::Exhausted);
            return true;
        } else {
            let middle = (span.a + span.b) * 0.5;
            self.stack.push(Span {
                a: middle,
                b: span.b,
                depth: span.depth + 1,
            });
            self.stack.push(Span {
                a: span.a,
                b: middle,
                depth: span.depth + 1,
            });
        }
        self.stack.is_empty()
    }

    pub fn finish(self) -> Result<Vec<Sample>, SamplingError> {
        if let Some(error) = self.error {
            Err(error)
        } else if !self.stack.is_empty() {
            Err(SamplingError::Exhausted)
        } else {
            Ok(self.samples)
        }
    }
}

pub fn sample_open(
    spline: &OpenCubicSpline,
    options: SamplingOptions,
) -> Result<Vec<Sample>, SamplingError> {
    let mut sampler = OpenSampler::new(spline, options);
    while !sampler.step() {}
    sampler.finish()
}

pub fn closest_open_parameter(spline: &OpenCubicSpline, samples: &[Sample], point: Point2) -> f64 {
    let Some(pair) = samples.windows(2).min_by(|a, b| {
        point_segment_distance(point, a[0].point, a[1].point)
            .total_cmp(&point_segment_distance(point, b[0].point, b[1].point))
    }) else {
        return 0.0;
    };
    let mut a = pair[0].t;
    let mut b = pair[1].t;
    for _ in 0..28 {
        let left = (2.0 * a + b) / 3.0;
        let right = (a + 2.0 * b) / 3.0;
        if (spline.evaluate(left) - point).norm() < (spline.evaluate(right) - point).norm() {
            b = right;
        } else {
            a = left;
        }
    }
    (a + b) * 0.5
}

/// Closest sampled segment followed by bounded local minimization in parameter space.
pub fn closest_parameter(spline: &PeriodicCubicSpline, samples: &[Sample], point: Point2) -> f64 {
    let Some(pair) = samples.windows(2).min_by(|a, b| {
        point_segment_distance(point, a[0].point, a[1].point)
            .total_cmp(&point_segment_distance(point, b[0].point, b[1].point))
    }) else {
        return 0.0;
    };
    let mut a = pair[0].t;
    let mut b = pair[1].t;
    for _ in 0..28 {
        let l = (2.0 * a + b) / 3.0;
        let r = (a + 2.0 * b) / 3.0;
        if (spline.evaluate(l) - point).norm() < (spline.evaluate(r) - point).norm() {
            b = r
        } else {
            a = l
        }
    }
    (a + b) * 0.5
}
