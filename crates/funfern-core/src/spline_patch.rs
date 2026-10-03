//! Spike-grade spline patch discretization for the IGA feasibility study
//! (`docs/spikes/funfern-iga-feasibility-spike.md`).
//!
//! Uniform B-spline bases of degree one to three, Gauss rules, the periodic
//! symbols a dispersion analysis needs, and an affine tensor-product patch on
//! a box stepped by the same kick-drift `(Q, b)` scheme as the triangle
//! solver: `Q` lives on the control coefficients, `b` at the quadrature
//! samples, divergence is the exact transpose of gradient through one shared
//! table, and the field is recovered from `Q` by a mass treatment that stays
//! symmetric positive definite and matrix-free. The treatments are the
//! row-sum lumped diagonal, that diagonal followed by Jacobi sweeps toward
//! the consistent mass, and the consistent mass itself as the reference.
//! Nothing here is reached by the application.

use crate::{PeriodicCubicSpline, Point2};

/// Highest spline degree the fixed-size local arrays hold.
pub const MAX_DEGREE: usize = 3;
const MAX_LOCAL: usize = MAX_DEGREE + 1;
const MAX_LOCAL_2D: usize = MAX_LOCAL * MAX_LOCAL;

/// Gauss-Legendre points on `(0, 1)` with weights summing to one, for one
/// to five points.
pub fn gauss_rule(points: usize) -> Option<Vec<(f64, f64)>> {
    let symmetric: &[(f64, f64)] = match points {
        1 => &[(0.0, 2.0)],
        2 => &[(0.577_350_269_189_625_8, 1.0)],
        3 => &[(0.0, 8.0 / 9.0), (0.774_596_669_241_483_4, 5.0 / 9.0)],
        4 => &[
            (0.339_981_043_584_856_3, 0.652_145_154_862_546_1),
            (0.861_136_311_594_052_6, 0.347_854_845_137_453_8),
        ],
        5 => &[
            (0.0, 0.568_888_888_888_888_9),
            (0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
            (0.906_179_845_938_664, 0.236_926_885_056_189_1),
        ],
        _ => return None,
    };
    let mut rule = Vec::with_capacity(points);
    for &(abscissa, weight) in symmetric {
        if abscissa == 0.0 {
            rule.push((0.5, weight / 2.0));
        } else {
            rule.push(((1.0 - abscissa) / 2.0, weight / 2.0));
            rule.push(((1.0 + abscissa) / 2.0, weight / 2.0));
        }
    }
    rule.sort_by(|a, b| a.0.total_cmp(&b.0));
    Some(rule)
}

/// A uniform B-spline basis over `spans` spans of length `spacing`, either
/// clamped at both ends (open knot vector, `spans + degree` functions) or
/// periodic (`spans` functions). Span `s` carries the functions `s..=s+degree`,
/// taken modulo the function count when periodic.
#[derive(Clone, Debug)]
pub struct UniformBasis {
    degree: usize,
    spans: usize,
    spacing: f64,
    periodic: bool,
    knots: Vec<f64>,
    /// The knot interval each nonempty span is: `span + degree` unless an
    /// interior knot is repeated.
    intervals: Vec<usize>,
}

impl UniformBasis {
    pub fn open(degree: usize, spans: usize, spacing: f64) -> Self {
        assert!((1..=MAX_DEGREE).contains(&degree) && spans >= 1 && spacing > 0.0);
        let mut knots = vec![0.0; degree + 1];
        knots.extend((1..spans).map(|index| index as f64 * spacing));
        knots.extend(std::iter::repeat_n(spans as f64 * spacing, degree + 1));
        Self {
            degree,
            spans,
            spacing,
            periodic: false,
            knots,
            intervals: (0..spans).map(|span| span + degree).collect(),
        }
    }

    /// A clamped or unclamped uniform basis whose knot at the start of span
    /// `at_span` is repeated to multiplicity `degree`, so the splines are
    /// only continuous there: a line a field may kink along. Adds
    /// `degree - 1` functions.
    pub fn with_c0_knot(
        degree: usize,
        spans: usize,
        spacing: f64,
        at_span: usize,
        unclamped: bool,
    ) -> Self {
        assert!((1..=MAX_DEGREE).contains(&degree) && at_span >= 1 && at_span < spans);
        let base = if unclamped {
            Self::unclamped(degree, spans, spacing)
        } else {
            Self::open(degree, spans, spacing)
        };
        let knot = at_span as f64 * spacing;
        let position = base.intervals[at_span];
        let mut knots = base.knots.clone();
        for _ in 1..degree {
            knots.insert(position, knot);
        }
        let intervals = (0..spans)
            .map(|span| span + degree + if span >= at_span { degree - 1 } else { 0 })
            .collect();
        Self {
            degree,
            spans,
            spacing,
            periodic: false,
            knots,
            intervals,
        }
    }

    pub fn periodic(degree: usize, spans: usize, spacing: f64) -> Self {
        assert!((1..=MAX_DEGREE).contains(&degree) && spans > 2 * degree && spacing > 0.0);
        let knots = (0..=spans + 2 * degree)
            .map(|index| (index as f64 - degree as f64) * spacing)
            .collect();
        Self {
            degree,
            spans,
            spacing,
            periodic: true,
            knots,
            intervals: (0..spans).map(|span| span + degree).collect(),
        }
    }

    /// Uniform knots running `degree` spans past each end of the domain with
    /// no repeated knots: the same spline space as the clamped basis and the
    /// same `spans + degree` functions, but the end functions are truncated
    /// translates of the interior one rather than collapsed onto the wall.
    pub fn unclamped(degree: usize, spans: usize, spacing: f64) -> Self {
        assert!((1..=MAX_DEGREE).contains(&degree) && spans >= 1 && spacing > 0.0);
        let knots = (0..=spans + 2 * degree)
            .map(|index| (index as f64 - degree as f64) * spacing)
            .collect();
        Self {
            degree,
            spans,
            spacing,
            periodic: false,
            knots,
            intervals: (0..spans).map(|span| span + degree).collect(),
        }
    }

    pub fn degree(&self) -> usize {
        self.degree
    }

    pub fn spans(&self) -> usize {
        self.spans
    }

    pub fn spacing(&self) -> f64 {
        self.spacing
    }

    pub fn length(&self) -> f64 {
        self.spans as f64 * self.spacing
    }

    pub fn is_periodic(&self) -> bool {
        self.periodic
    }

    /// Whether the end knots are repeated (an open knot vector).
    pub fn is_clamped(&self) -> bool {
        !self.periodic && self.knots[0] == self.knots[self.degree]
    }

    pub fn functions(&self) -> usize {
        if self.periodic {
            self.spans
        } else {
            self.knots.len() - self.degree - 1
        }
    }

    /// The global index of local function `local` on span `span`.
    pub fn function(&self, span: usize, local: usize) -> usize {
        (self.intervals[span] - self.degree + local) % self.functions()
    }

    /// The span holding `parameter`, the last one at the right end.
    pub fn span_of(&self, parameter: f64) -> usize {
        ((parameter / self.spacing).floor().max(0.0) as usize).min(self.spans - 1)
    }

    /// Values and first derivatives of the `degree + 1` functions live on
    /// `span` at `parameter`, which lies in that span; the local arrays are
    /// filled up to `degree`.
    pub fn evaluate(&self, span: usize, parameter: f64) -> ([f64; MAX_LOCAL], [f64; MAX_LOCAL]) {
        let p = self.degree;
        let k = self.intervals[span];
        let values = self.cox_de_boor(k, p, parameter);
        let mut derivatives = [0.0; MAX_LOCAL];
        // The lower-degree functions live on `k - p + 1 ..= k`.
        let lower = self.cox_de_boor(k, p - 1, parameter);
        for (a, derivative) in derivatives.iter_mut().enumerate().take(p + 1) {
            let i = k - p + a;
            let mut value = 0.0;
            if a >= 1 {
                let width = self.knots[i + p] - self.knots[i];
                if width > 0.0 {
                    value += p as f64 * lower[a - 1] / width;
                }
            }
            if a < p {
                let width = self.knots[i + p + 1] - self.knots[i + 1];
                if width > 0.0 {
                    value -= p as f64 * lower[a] / width;
                }
            }
            *derivative = value;
        }
        (values, derivatives)
    }

    /// The nonzero functions of degree `degree` on knot interval `k`, which
    /// holds `parameter`: local `r` is function `k - degree + r`.
    fn cox_de_boor(&self, k: usize, degree: usize, parameter: f64) -> [f64; MAX_LOCAL] {
        let mut values = [0.0; MAX_LOCAL];
        values[0] = 1.0;
        let mut left = [0.0; MAX_LOCAL];
        let mut right = [0.0; MAX_LOCAL];
        for j in 1..=degree {
            left[j] = parameter - self.knots[k + 1 - j];
            right[j] = self.knots[k + j] - parameter;
            let mut saved = 0.0;
            for r in 0..j {
                let denominator = right[r + 1] + left[j - r];
                let temp = if denominator == 0.0 {
                    0.0
                } else {
                    values[r] / denominator
                };
                values[r] = saved + right[r + 1] * temp;
                saved = left[j - r] * temp;
            }
            values[j] = saved;
        }
        values
    }

    /// Greville abscissae, where a coefficient is best read as a point value.
    pub fn greville(&self) -> Vec<f64> {
        (0..self.functions())
            .map(|i| (1..=self.degree).map(|j| self.knots[i + j]).sum::<f64>() / self.degree as f64)
            .collect()
    }
}

/// The circulant symbols of a uniform periodic basis under a quadrature
/// rule: the eigenvalue of each operator on the plane wave `exp(i j θ)`
/// over the coefficients, `θ` in radians per span. Products of a cubic
/// basis are degree six, so a rule of fewer than four points integrates the
/// mass inexactly and the stiffness exactly only from three points; the
/// symbols are those of the rule as applied, not of exact integration.
#[derive(Clone, Debug)]
pub struct PeriodicSymbols {
    degree: usize,
    spacing: f64,
    /// `M_{0,d}` for `d = 0..=degree`.
    mass: Vec<f64>,
    /// `K_{0,d}` for `d = 0..=degree`.
    stiffness: Vec<f64>,
    /// The row sum of the mass, `∫ N_0` under the rule.
    lumped: f64,
}

impl PeriodicSymbols {
    pub fn new(degree: usize, spacing: f64, rule: &[(f64, f64)]) -> Self {
        let spans = 2 * degree + 2;
        let basis = UniformBasis::periodic(degree, spans, spacing);
        let n = basis.functions();
        let mut mass = vec![0.0; n * n];
        let mut stiffness = vec![0.0; n * n];
        for span in 0..spans {
            for &(abscissa, weight) in rule {
                let parameter = (span as f64 + abscissa) * spacing;
                let (values, derivatives) = basis.evaluate(span, parameter);
                let weight = weight * spacing;
                for a in 0..=degree {
                    let i = basis.function(span, a);
                    for b in 0..=degree {
                        let j = basis.function(span, b);
                        mass[i * n + j] += weight * values[a] * values[b];
                        stiffness[i * n + j] += weight * derivatives[a] * derivatives[b];
                    }
                }
            }
        }
        Self {
            degree,
            spacing,
            mass: (0..=degree).map(|d| mass[d]).collect(),
            stiffness: (0..=degree).map(|d| stiffness[d]).collect(),
            lumped: (0..n).map(|j| mass[j]).sum(),
        }
    }

    fn symbol(row: &[f64], theta: f64) -> f64 {
        row[0]
            + 2.0
                * row
                    .iter()
                    .enumerate()
                    .skip(1)
                    .map(|(d, value)| value * (d as f64 * theta).cos())
                    .sum::<f64>()
    }

    /// The consistent mass symbol.
    pub fn mass(&self, theta: f64) -> f64 {
        Self::symbol(&self.mass, theta)
    }

    pub fn stiffness(&self, theta: f64) -> f64 {
        Self::symbol(&self.stiffness, theta)
    }

    /// The row-sum lumped mass, the same at every wavenumber.
    pub fn lumped(&self) -> f64 {
        self.lumped
    }

    pub fn degree(&self) -> usize {
        self.degree
    }

    pub fn spacing(&self) -> f64 {
        self.spacing
    }

    /// The symbol of the field map `P` that turns `Q` into `u` on a tensor
    /// patch of square spans with unit density: the lumped inverse followed
    /// by `sweeps` Jacobi sweeps toward the consistent inverse, or the
    /// consistent inverse itself for `None`. `theta` and `phi` are the two
    /// parametric wavenumbers in radians per span.
    pub fn field_symbol(&self, theta: f64, phi: f64, sweeps: Option<usize>) -> f64 {
        let consistent = self.mass(theta) * self.mass(phi);
        let lumped = self.lumped * self.lumped;
        match sweeps {
            None => 1.0 / consistent,
            Some(sweeps) => {
                let ratio = 1.0 - consistent / lumped;
                (0..=sweeps).map(|k| ratio.powi(k as i32)).sum::<f64>() / lumped
            }
        }
    }

    /// The discrete frequency squared of the plane wave `(theta, phi)` on a
    /// tensor patch with unit material, under the field map.
    pub fn frequency_squared(&self, theta: f64, phi: f64, sweeps: Option<usize>) -> f64 {
        let stiffness =
            self.stiffness(theta) * self.mass(phi) + self.mass(theta) * self.stiffness(phi);
        stiffness * self.field_symbol(theta, phi, sweeps)
    }

    /// The discrete over the exact frequency of the plane wave.
    pub fn frequency_ratio(&self, theta: f64, phi: f64, sweeps: Option<usize>) -> f64 {
        let exact = (theta * theta + phi * phi).sqrt() / self.spacing;
        self.frequency_squared(theta, phi, sweeps).max(0.0).sqrt() / exact
    }

    /// The largest discrete frequency over the periodic band, searched on a
    /// grid; the stable leapfrog step is twice its reciprocal.
    pub fn maximum_frequency(&self, sweeps: Option<usize>) -> f64 {
        let steps = 256;
        let mut maximum: f64 = 0.0;
        for i in 0..=steps {
            let theta = std::f64::consts::PI * i as f64 / steps as f64;
            for j in 0..=steps {
                let phi = std::f64::consts::PI * j as f64 / steps as f64;
                maximum = maximum.max(self.frequency_squared(theta, phi, sweeps));
            }
        }
        maximum.sqrt()
    }
}

/// The time integrator a patch is stepped with. Both are Störmer schemes on
/// `Q̈ = -K P Q` written as a kick on `Q` and a drift on `b`, and both
/// conserve a quadratic energy exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Integrator {
    /// Second order: `b` drifts on the field `u = P Q`. Stable while
    /// `ω dt < 2`.
    Leapfrog,
    /// Fourth order by the modified equation (Dablain 1986): `b` drifts on
    /// `ũ = P (Q − dt²/12 · K u)`, so the kick applies the modified stiffness
    /// `K̃ = K (I − dt²/12 · P K)`, whose Taylor series matches
    /// `2 (I − cos(dt √(KP))) / dt²` through `dt⁴`. One extra gradient,
    /// divergence and field pass a step. Stable, and `K̃` positive definite,
    /// while `ω dt < 2√3`.
    FourthOrder,
}

impl Integrator {
    /// The stable `ω dt` over leapfrog's two.
    pub fn stability_factor(self) -> f64 {
        match self {
            Self::Leapfrog => 1.0,
            Self::FourthOrder => 3.0_f64.sqrt(),
        }
    }

    /// The frequency this integrator with step `dt` advances a mode of
    /// discrete frequency `omega` at.
    pub fn stepped_frequency(self, omega: f64, dt: f64) -> f64 {
        let x = (omega * dt).powi(2);
        let cosine = match self {
            Self::Leapfrog => 1.0 - 0.5 * x,
            Self::FourthOrder => 1.0 - 0.5 * x + x * x / 24.0,
        };
        cosine.clamp(-1.0, 1.0).acos() / dt
    }

    /// The inverse of [`Self::stepped_frequency`]: the discrete spatial
    /// frequency of a mode seen advancing at `stepped`.
    pub fn spatial_frequency(self, stepped: f64, dt: f64) -> f64 {
        let cosine = (stepped * dt).cos();
        let x = match self {
            Self::Leapfrog => 2.0 - 2.0 * cosine,
            Self::FourthOrder => 6.0 - (12.0 + 24.0 * cosine).max(0.0).sqrt(),
        };
        x.max(0.0).sqrt() / dt
    }
}

/// The frequency leapfrog with step `dt` advances a mode of discrete
/// frequency `omega` at.
pub fn leapfrog_frequency(omega: f64, dt: f64) -> f64 {
    Integrator::Leapfrog.stepped_frequency(omega, dt)
}

/// How `Q` is turned into the field `u` on a patch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MassTreatment {
    /// The row-sum lumped diagonal, then `sweeps` Jacobi sweeps toward the
    /// consistent mass; zero sweeps is plain lumping.
    Lumped { sweeps: usize },
    /// The consistent mass, solved by conjugate gradients to roundoff.
    Consistent,
}

impl MassTreatment {
    /// The sweep count the periodic symbols take: `None` for consistent.
    pub fn sweeps(self) -> Option<usize> {
        match self {
            Self::Lumped { sweeps } => Some(sweeps),
            Self::Consistent => None,
        }
    }
}

/// One side of a Coons patch: a curve from the parametric side's start to
/// its end, as a function of the normalized position `s` in `[0, 1]`.
#[derive(Clone, Debug)]
pub enum CoonsSide {
    /// A straight segment.
    Line { from: Point2, to: Point2 },
    /// An exact elliptic arc, `center + (a cos φ, b sin φ)` for `φ` from
    /// `from_angle` to `to_angle`.
    Ellipse {
        center: Point2,
        radii: Point2,
        from_angle: f64,
        to_angle: f64,
    },
    /// A piece of a closed spline from parameter `from` to `to`, taken
    /// forward in the parameter (`to` may pass the period).
    Spline {
        curve: PeriodicCubicSpline,
        from: f64,
        to: f64,
    },
}

impl CoonsSide {
    /// The point and its derivative with respect to `s`.
    pub fn evaluate(&self, s: f64) -> (Point2, Point2) {
        match self {
            Self::Line { from, to } => (from.lerp(*to, s), *to - *from),
            Self::Ellipse {
                center,
                radii,
                from_angle,
                to_angle,
            } => {
                let angle = from_angle + (to_angle - from_angle) * s;
                let (sin, cos) = angle.sin_cos();
                (
                    *center + Point2::new(radii.x * cos, radii.y * sin),
                    Point2::new(-radii.x * sin, radii.y * cos) * (to_angle - from_angle),
                )
            }
            Self::Spline { curve, from, to } => {
                let t = (from + (to - from) * s).rem_euclid(curve.period());
                (curve.evaluate(t), curve.derivative(t, 1) * (to - from))
            }
        }
    }

    /// The arc traversed the other way.
    pub fn reversed(self) -> Self {
        match self {
            Self::Line { from, to } => Self::Line { from: to, to: from },
            Self::Ellipse {
                center,
                radii,
                from_angle,
                to_angle,
            } => Self::Ellipse {
                center,
                radii,
                from_angle: to_angle,
                to_angle: from_angle,
            },
            Self::Spline { curve, from, to } => Self::Spline {
                curve,
                from: to,
                to: from,
            },
        }
    }
}

/// How the parametric box `[0, Lx] × [0, Ly]` maps onto the physical patch.
#[derive(Clone, Debug)]
pub enum GeometryMap {
    /// `x = origin + (ξ, η)`.
    Affine { origin: Point2 },
    /// A tensor-product B-spline surface on clamped uniform knots over the
    /// parametric box, with its control net in `node` order of its bases.
    Surface {
        basis_x: UniformBasis,
        basis_y: UniformBasis,
        controls: Vec<Point2>,
    },
    /// A bilinearly blended Coons patch between four sides: `bottom` and
    /// `top` run with `ξ`, `left` and `right` run with `η`.
    Coons {
        bottom: Box<CoonsSide>,
        right: Box<CoonsSide>,
        top: Box<CoonsSide>,
        left: Box<CoonsSide>,
    },
}

impl GeometryMap {
    /// The surface whose control net is the Greville points of its bases
    /// shifted to `origin`: the identity map, to be deformed.
    pub fn flat_surface(origin: Point2, size: Point2, spans: [usize; 2]) -> Self {
        let basis_x = UniformBasis::open(3, spans[0], size.x / spans[0] as f64);
        let basis_y = UniformBasis::open(3, spans[1], size.y / spans[1] as f64);
        let gx = basis_x.greville();
        let gy = basis_y.greville();
        let mut controls = Vec::with_capacity(gx.len() * gy.len());
        for x in &gx {
            for y in &gy {
                controls.push(origin + Point2::new(*x, *y));
            }
        }
        Self::Surface {
            basis_x,
            basis_y,
            controls,
        }
    }

    /// A Coons patch over the inside of a closed spline, with its corners at
    /// the four parameters, taken counterclockwise along the curve.
    pub fn coons_from_curve(curve: &PeriodicCubicSpline, corners: [f64; 4]) -> Self {
        let period = curve.period();
        let [t0, t1, t2, t3] = corners;
        let arc = |from: f64, to: f64| {
            let to = if to <= from { to + period } else { to };
            CoonsSide::Spline {
                curve: curve.clone(),
                from,
                to,
            }
        };
        Self::Coons {
            bottom: Box::new(arc(t0, t1)),
            right: Box::new(arc(t1, t2)),
            top: Box::new(arc(t2, t3).reversed()),
            left: Box::new(arc(t3, t0).reversed()),
        }
    }

    /// A Coons patch over an exact ellipse with its corners on the
    /// diagonals.
    pub fn coons_ellipse(center: Point2, radii: Point2) -> Self {
        let quarter = std::f64::consts::FRAC_PI_4;
        let arc = |from: f64, to: f64| CoonsSide::Ellipse {
            center,
            radii,
            from_angle: from,
            to_angle: to,
        };
        Self::Coons {
            bottom: Box::new(arc(-3.0 * quarter, -quarter)),
            right: Box::new(arc(-quarter, quarter)),
            top: Box::new(arc(3.0 * quarter, quarter)),
            left: Box::new(arc(5.0 * quarter, 3.0 * quarter)),
        }
    }

    /// The physical point and the two parametric tangents `∂S/∂ξ`, `∂S/∂η`
    /// at a parametric point of a box of the given size.
    pub fn evaluate(&self, parameter: Point2, size: Point2) -> (Point2, Point2, Point2) {
        match self {
            Self::Affine { origin } => (
                *origin + parameter,
                Point2::new(1.0, 0.0),
                Point2::new(0.0, 1.0),
            ),
            Self::Surface {
                basis_x,
                basis_y,
                controls,
            } => {
                let sx = basis_x.span_of(parameter.x);
                let sy = basis_y.span_of(parameter.y);
                let (nx, dnx) = basis_x.evaluate(sx, parameter.x);
                let (ny, dny) = basis_y.evaluate(sy, parameter.y);
                let mut point = Point2::default();
                let mut d_xi = Point2::default();
                let mut d_eta = Point2::default();
                for a in 0..=basis_x.degree() {
                    let ix = basis_x.function(sx, a);
                    for b in 0..=basis_y.degree() {
                        let iy = basis_y.function(sy, b);
                        let control = controls[ix * basis_y.functions() + iy];
                        point = point + control * (nx[a] * ny[b]);
                        d_xi = d_xi + control * (dnx[a] * ny[b]);
                        d_eta = d_eta + control * (nx[a] * dny[b]);
                    }
                }
                (point, d_xi, d_eta)
            }
            Self::Coons {
                bottom,
                right,
                top,
                left,
            } => {
                let s = parameter.x / size.x;
                let r = parameter.y / size.y;
                let (cb, dcb) = bottom.evaluate(s);
                let (ct, dct) = top.evaluate(s);
                let (cl, dcl) = left.evaluate(r);
                let (cr, dcr) = right.evaluate(r);
                let (p00, _) = bottom.evaluate(0.0);
                let (p10, _) = bottom.evaluate(1.0);
                let (p01, _) = top.evaluate(0.0);
                let (p11, _) = top.evaluate(1.0);
                let bilinear = p00 * ((1.0 - s) * (1.0 - r))
                    + p10 * (s * (1.0 - r))
                    + p01 * ((1.0 - s) * r)
                    + p11 * (s * r);
                let point = cb * (1.0 - r) + ct * r + cl * (1.0 - s) + cr * s - bilinear;
                let d_s =
                    dcb * (1.0 - r) + dct * r - cl + cr - (p10 - p00) * (1.0 - r) - (p11 - p01) * r;
                let d_r =
                    ct - cb + dcl * (1.0 - s) + dcr * s - (p01 - p00) * (1.0 - s) - (p11 - p10) * s;
                (point, d_s / size.x, d_r / size.y)
            }
        }
    }
}

/// One sample's Jacobian data on a curved patch: the inverse transpose,
/// row-major, and the determinant.
#[derive(Clone, Copy, Debug)]
struct SampleJacobian {
    /// `∂S/∂ξ` then `∂S/∂η`, each `(x, y)`.
    forward: [f64; 4],
    inverse_transpose: [f64; 4],
    determinant: f64,
}

impl SampleJacobian {
    fn new(d_xi: Point2, d_eta: Point2) -> Option<Self> {
        let determinant = d_xi.cross(d_eta);
        if !determinant.is_finite() || determinant <= 0.0 {
            return None;
        }
        Some(Self {
            forward: [d_xi.x, d_xi.y, d_eta.x, d_eta.y],
            inverse_transpose: [
                d_eta.y / determinant,
                -d_xi.y / determinant,
                -d_eta.x / determinant,
                d_xi.x / determinant,
            ],
            determinant,
        })
    }

    /// The covariant (parametric) components `Jᵀ b` of a physical vector.
    fn covariant(&self, b: Point2) -> Point2 {
        let f = self.forward;
        Point2::new(f[0] * b.x + f[1] * b.y, f[2] * b.x + f[3] * b.y)
    }

    /// The physical vector `J⁻ᵀ b̂` of covariant components.
    fn physical(&self, covariant: Point2) -> Point2 {
        let m = self.inverse_transpose;
        Point2::new(
            m[0] * covariant.x + m[1] * covariant.y,
            m[2] * covariant.x + m[3] * covariant.y,
        )
    }
}

/// What a control-point move touched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditReport {
    pub affected_samples: usize,
    pub samples: usize,
    pub affected_dofs: usize,
    pub degrees_of_freedom: usize,
}

/// A tensor-product spline patch over a parametric box mapped by a
/// [`GeometryMap`], with the quadrature samples the `(Q, b)` scheme keeps
/// `b` at. `b` and gradients are in physical components.
#[derive(Clone, Debug)]
pub struct SplinePatch {
    basis_x: UniformBasis,
    basis_y: UniformBasis,
    rule: Vec<(f64, f64)>,
    geometry: GeometryMap,
    /// Per `(span, point)` in each direction: values and derivatives.
    table_x: Vec<([f64; MAX_LOCAL], [f64; MAX_LOCAL])>,
    table_y: Vec<([f64; MAX_LOCAL], [f64; MAX_LOCAL])>,
    /// Per sample, in sample order; empty under the affine map.
    jacobians: Vec<SampleJacobian>,
    points: Vec<Point2>,
    /// Each tensor-product function's degree of freedom, when functions
    /// around the corners are condensed into one each; identity otherwise.
    condensation: Option<Vec<usize>>,
    degrees_of_freedom: usize,
    /// Mass added to each degree of freedom's lumped diagonal and to the
    /// consistent mass's diagonal alike; zeros when none.
    mass_scaling: Vec<f64>,
    /// Per sample: the density, which weights the mass, and the stiffness,
    /// which weights `b` in the kick and the energy; ones by default.
    density: Vec<f64>,
    stiffness: Vec<f64>,
    lumped: Vec<f64>,
    weights: Vec<f64>,
}

/// One quadrature sample's stencil: the first `count` entries are live.
pub struct SampleStencil {
    count: usize,
    pub weight: f64,
    nodes: [usize; MAX_LOCAL_2D],
    values: [f64; MAX_LOCAL_2D],
    gradients: [Point2; MAX_LOCAL_2D],
}

impl SampleStencil {
    pub fn nodes(&self) -> &[usize] {
        &self.nodes[..self.count]
    }

    pub fn values(&self) -> &[f64] {
        &self.values[..self.count]
    }

    pub fn gradients(&self) -> &[Point2] {
        &self.gradients[..self.count]
    }
}

impl SplinePatch {
    /// A patch of `degree` over the box from `origin` with the given side
    /// lengths, `spans` spans per direction and `points` Gauss points per
    /// span per direction, on clamped bases.
    pub fn new(
        degree: usize,
        origin: Point2,
        size: Point2,
        spans: [usize; 2],
        points: usize,
    ) -> Option<Self> {
        let basis_x = UniformBasis::open(degree, spans[0], size.x / spans[0] as f64);
        let basis_y = UniformBasis::open(degree, spans[1], size.y / spans[1] as f64);
        Self::from_bases(basis_x, basis_y, GeometryMap::Affine { origin }, points)
    }

    /// A patch of `degree` over the parametric box of the given size mapped
    /// by `geometry`, clamped or unclamped.
    pub fn curved(
        degree: usize,
        geometry: GeometryMap,
        size: Point2,
        spans: [usize; 2],
        points: usize,
        unclamped: bool,
    ) -> Option<Self> {
        let basis = |spans: usize, length: f64| {
            if unclamped {
                UniformBasis::unclamped(degree, spans, length / spans as f64)
            } else {
                UniformBasis::open(degree, spans, length / spans as f64)
            }
        };
        Self::from_bases(
            basis(spans[0], size.x),
            basis(spans[1], size.y),
            geometry,
            points,
        )
    }

    /// The same patch on unclamped bases: the same space, with end
    /// functions that are truncated translates instead of collapsed onto
    /// the wall, which changes the lumped diagonal and nothing else.
    pub fn unclamped(
        degree: usize,
        origin: Point2,
        size: Point2,
        spans: [usize; 2],
        points: usize,
    ) -> Option<Self> {
        let basis_x = UniformBasis::unclamped(degree, spans[0], size.x / spans[0] as f64);
        let basis_y = UniformBasis::unclamped(degree, spans[1], size.y / spans[1] as f64);
        Self::from_bases(basis_x, basis_y, GeometryMap::Affine { origin }, points)
    }

    /// A patch from two bases of its own, such as one with a C⁰ knot.
    pub fn from_bases_public(
        basis_x: UniformBasis,
        basis_y: UniformBasis,
        geometry: GeometryMap,
        points: usize,
    ) -> Option<Self> {
        Self::from_bases(basis_x, basis_y, geometry, points)
    }

    fn from_bases(
        basis_x: UniformBasis,
        basis_y: UniformBasis,
        geometry: GeometryMap,
        points: usize,
    ) -> Option<Self> {
        let rule = gauss_rule(points)?;
        let table = |basis: &UniformBasis| {
            let mut table = Vec::with_capacity(basis.spans() * rule.len());
            for span in 0..basis.spans() {
                for &(abscissa, _) in &rule {
                    let parameter = (span as f64 + abscissa) * basis.spacing();
                    table.push(basis.evaluate(span, parameter));
                }
            }
            table
        };
        let table_x = table(&basis_x);
        let table_y = table(&basis_y);
        let mut patch = Self {
            basis_x,
            basis_y,
            rule,
            geometry,
            table_x,
            table_y,
            jacobians: Vec::new(),
            points: Vec::new(),
            condensation: None,
            degrees_of_freedom: 0,
            mass_scaling: Vec::new(),
            density: Vec::new(),
            stiffness: Vec::new(),
            lumped: Vec::new(),
            weights: Vec::new(),
        };
        patch.degrees_of_freedom = patch.basis_x.functions() * patch.basis_y.functions();
        // The Jacobian at every sample, in sample order; the identity is
        // left implicit under the affine map.
        let size = Point2::new(patch.basis_x.length(), patch.basis_y.length());
        let affine = matches!(patch.geometry, GeometryMap::Affine { .. });
        let mut points = Vec::with_capacity(patch.samples());
        let mut jacobians = Vec::with_capacity(if affine { 0 } else { patch.samples() });
        patch
            .for_each_parametric_sample(|parameter| {
                let (point, d_xi, d_eta) = patch.geometry.evaluate(parameter, size);
                points.push(point);
                if !affine {
                    jacobians.push(SampleJacobian::new(d_xi, d_eta).ok_or(())?);
                }
                Ok::<(), ()>(())
            })
            .ok()?;
        patch.points = points;
        patch.jacobians = jacobians;
        patch.density = vec![1.0; patch.samples()];
        patch.stiffness = vec![1.0; patch.samples()];
        patch.accumulate_lumped();
        Some(patch)
    }

    /// The same patch with the density and stiffness sampled at every
    /// quadrature point: an immersed material, with the interface wherever
    /// the samples put it.
    pub fn with_material(
        mut self,
        density: impl Fn(Point2) -> f64,
        stiffness: impl Fn(Point2) -> f64,
    ) -> Self {
        self.density = self.points.iter().map(|p| density(*p)).collect();
        self.stiffness = self.points.iter().map(|p| stiffness(*p)).collect();
        assert!(self.density.iter().chain(&self.stiffness).all(|v| *v > 0.0));
        self.accumulate_lumped();
        self
    }

    /// Every sample's stiffness, in sample order.
    pub fn sample_stiffness(&self) -> &[f64] {
        &self.stiffness
    }

    /// Every sample's density, in sample order.
    pub fn sample_density(&self) -> &[f64] {
        &self.density
    }

    fn accumulate_lumped(&mut self) {
        let mut lumped = vec![0.0; self.degrees_of_freedom()];
        let mut weights = Vec::with_capacity(self.samples());
        self.for_each_sample(|index, stencil| {
            weights.push(stencil.weight);
            let mass = stencil.weight * self.density[index];
            for (node, value) in stencil.nodes().iter().zip(stencil.values()) {
                lumped[*node] += mass * value;
            }
        });
        self.lumped = lumped;
        self.weights = weights;
        self.mass_scaling = vec![0.0; self.degrees_of_freedom()];
    }

    /// The diagonal of the stiffness, `Σ_q w_q |∇N_i(q)|²`, over the lumped
    /// mass: a bound on each degree of freedom's own frequency squared.
    pub fn frequency_bounds(&self) -> Vec<f64> {
        let mut stiffness = vec![0.0; self.degrees_of_freedom()];
        self.for_each_sample(|index, stencil| {
            let weight = stencil.weight * self.stiffness[index];
            for (node, g) in stencil.nodes().iter().zip(stencil.gradients()) {
                stiffness[*node] += weight * g.dot(*g);
            }
        });
        stiffness
            .iter()
            .zip(&self.lumped)
            .map(|(k, d)| k / d)
            .collect()
    }

    /// Selective mass scaling: every degree of freedom whose own frequency
    /// bound exceeds `cap` times the median gets mass added to its lumped
    /// diagonal, and to the consistent mass's diagonal, until it meets the
    /// cap. The added mass sits on the functions at a singular corner,
    /// whose physical footprint is small, and nowhere else. The field map
    /// stays symmetric positive definite and the sweeps still converge,
    /// since the scaled mass keeps its row sums equal to the scaled
    /// diagonal.
    pub fn with_mass_scaling(mut self, cap: f64) -> Self {
        let bounds = self.frequency_bounds();
        let mut sorted = bounds.clone();
        sorted.sort_by(f64::total_cmp);
        let limit = cap * sorted[sorted.len() / 2];
        let mut scaled = 0;
        for ((bound, lumped), scaling) in bounds
            .iter()
            .zip(self.lumped.iter_mut())
            .zip(self.mass_scaling.iter_mut())
        {
            if *bound > limit {
                let added = *lumped * (bound / limit - 1.0);
                *lumped += added;
                *scaling += added;
                scaled += 1;
            }
        }
        let _ = scaled;
        self
    }

    /// The degrees of freedom that carry added mass.
    pub fn scaled_count(&self) -> usize {
        self.mass_scaling.iter().filter(|m| **m > 0.0).count()
    }

    /// The added mass over the lumped mass it joined, at the most scaled
    /// degree of freedom.
    pub fn largest_scaling_factor(&self) -> f64 {
        self.mass_scaling
            .iter()
            .zip(&self.lumped)
            .map(|(added, total)| total / (total - added))
            .fold(1.0, f64::max)
    }

    /// The same patch with the `reach × reach` functions nearest each
    /// corner condensed into one function per corner, their sum. On a
    /// Coons patch of a smooth domain the corners are singular, the
    /// functions there have almost no mass and ring at outlier
    /// frequencies, and their parametric neighbourhood is a tiny physical
    /// region, so the condensed space loses nothing visible. `reach` of
    /// zero or one leaves the patch as it is.
    pub fn with_condensed_corners(mut self, reach: usize) -> Self {
        let nx = self.basis_x.functions();
        let ny = self.basis_y.functions();
        if reach < 2 || 2 * reach > nx.min(ny) {
            return self;
        }
        let corner = |ix: usize, iy: usize| -> Option<usize> {
            let left = ix < reach;
            let right = ix >= nx - reach;
            let bottom = iy < reach;
            let top = iy >= ny - reach;
            match (left, right, bottom, top) {
                (true, _, true, _) => Some(0),
                (_, true, true, _) => Some(1),
                (true, _, _, true) => Some(2),
                (_, true, _, true) => Some(3),
                _ => None,
            }
        };
        let mut map = vec![usize::MAX; nx * ny];
        let mut next = 0;
        let mut corners = [usize::MAX; 4];
        for ix in 0..nx {
            for iy in 0..ny {
                let full = ix * ny + iy;
                map[full] = match corner(ix, iy) {
                    Some(c) => {
                        if corners[c] == usize::MAX {
                            corners[c] = next;
                            next += 1;
                        }
                        corners[c]
                    }
                    None => {
                        next += 1;
                        next - 1
                    }
                };
            }
        }
        self.condensation = Some(map);
        self.degrees_of_freedom = next;
        self.accumulate_lumped();
        self
    }

    /// Whether the corners are condensed.
    pub fn is_condensed(&self) -> bool {
        self.condensation.is_some()
    }

    pub fn degree(&self) -> usize {
        self.basis_x.degree()
    }

    pub fn basis_x(&self) -> &UniformBasis {
        &self.basis_x
    }

    pub fn basis_y(&self) -> &UniformBasis {
        &self.basis_y
    }

    pub fn degrees_of_freedom(&self) -> usize {
        self.degrees_of_freedom
    }

    pub fn samples(&self) -> usize {
        self.basis_x.spans() * self.basis_y.spans() * self.rule.len() * self.rule.len()
    }

    pub fn points_per_span(&self) -> usize {
        self.rule.len()
    }

    /// The row-sum lumped mass with unit density.
    pub fn lumped(&self) -> &[f64] {
        &self.lumped
    }

    /// Every sample's integration weight, in sample order.
    pub fn sample_weights(&self) -> &[f64] {
        &self.weights
    }

    /// The degree of freedom of tensor-product function `(ix, iy)`.
    pub fn node(&self, ix: usize, iy: usize) -> usize {
        let full = ix * self.basis_y.functions() + iy;
        match &self.condensation {
            Some(map) => map[full],
            None => full,
        }
    }

    pub fn geometry(&self) -> &GeometryMap {
        &self.geometry
    }

    /// Every sample's physical point, in sample order.
    pub fn sample_points(&self) -> &[Point2] {
        &self.points
    }

    /// The parametric box's size.
    pub fn parametric_size(&self) -> Point2 {
        Point2::new(self.basis_x.length(), self.basis_y.length())
    }

    /// The field's value at a parametric point.
    pub fn evaluate_field(&self, u: &[f64], parameter: Point2) -> f64 {
        let sx = self.basis_x.span_of(parameter.x);
        let sy = self.basis_y.span_of(parameter.y);
        let (nx, _) = self.basis_x.evaluate(sx, parameter.x);
        let (ny, _) = self.basis_y.evaluate(sy, parameter.y);
        let p = self.degree();
        let mut value = 0.0;
        for (a, nx) in nx.iter().enumerate().take(p + 1) {
            let ix = self.basis_x.function(sx, a);
            for (b, ny) in ny.iter().enumerate().take(p + 1) {
                let iy = self.basis_y.function(sy, b);
                value += u[self.node(ix, iy)] * nx * ny;
            }
        }
        value
    }

    /// Every sample's parametric point, in sample order.
    pub fn parametric_points(&self) -> Vec<Point2> {
        let mut out = Vec::with_capacity(self.samples());
        let _ = self.for_each_parametric_sample(|parameter| {
            out.push(parameter);
            Ok::<(), ()>(())
        });
        out
    }

    /// The same patch with one control point of its surface map moved by
    /// `delta`, rebuilding only the samples in the spans that point
    /// touches, and the count of what it touched. `None` for a map without
    /// a control net, or if the move folds the map.
    pub fn with_moved_control(
        &self,
        ix: usize,
        iy: usize,
        delta: Point2,
    ) -> Option<(Self, EditReport)> {
        let mut patch = self.clone();
        let GeometryMap::Surface {
            basis_x,
            basis_y,
            controls,
        } = &mut patch.geometry
        else {
            return None;
        };
        let degree = basis_x.degree();
        let control = ix * basis_y.functions() + iy;
        controls[control] = controls[control] + delta;
        // The control point's support: spans `ix - degree ..= ix`, clipped.
        let low = |index: usize| index.saturating_sub(degree) as f64;
        let high = |index: usize, spans: usize| (index.min(spans - 1) + 1) as f64;
        let touched_x = low(ix) * basis_x.spacing()..=high(ix, basis_x.spans()) * basis_x.spacing();
        let touched_y = low(iy) * basis_y.spacing()..=high(iy, basis_y.spans()) * basis_y.spacing();
        let size = patch.parametric_size();
        let geometry = patch.geometry.clone();
        let mut affected = Vec::new();
        let mut index = 0;
        let rebuilt = patch.for_each_parametric_sample(|parameter| {
            if touched_x.contains(&parameter.x) && touched_y.contains(&parameter.y) {
                let (point, d_xi, d_eta) = geometry.evaluate(parameter, size);
                affected.push((index, point, SampleJacobian::new(d_xi, d_eta).ok_or(())?));
            }
            index += 1;
            Ok::<(), ()>(())
        });
        rebuilt.ok()?;
        for (index, point, jacobian) in &affected {
            patch.points[*index] = *point;
            patch.jacobians[*index] = *jacobian;
        }
        let affected_set: std::collections::HashSet<usize> =
            affected.iter().map(|(index, _, _)| *index).collect();
        let mut touched_dofs = vec![false; patch.degrees_of_freedom()];
        patch.for_each_sample(|index, stencil| {
            if affected_set.contains(&index) {
                for node in stencil.nodes() {
                    touched_dofs[*node] = true;
                }
            }
        });
        patch.accumulate_lumped();
        let report = EditReport {
            affected_samples: affected.len(),
            samples: patch.samples(),
            affected_dofs: touched_dofs.iter().filter(|t| **t).count(),
            degrees_of_freedom: patch.degrees_of_freedom(),
        };
        Some((patch, report))
    }

    /// The flux `b` carried from this patch to `target`, which has the same
    /// samples at the same parametric points but a different map: the
    /// covariant components `Jᵀ b` are kept, as the gradient of a field that
    /// moves with the geometry.
    pub fn carry_flux(&self, target: &Self, b: &[Point2]) -> Vec<Point2> {
        assert_eq!(self.samples(), target.samples());
        (0..self.samples())
            .map(|index| {
                let covariant = match self.jacobians.get(index) {
                    Some(j) => j.covariant(b[index]),
                    None => b[index],
                };
                match target.jacobians.get(index) {
                    Some(j) => j.physical(covariant),
                    None => covariant,
                }
            })
            .collect()
    }

    /// The same patch with every span split in two per direction, on the
    /// same map, with unit material.
    pub fn refined(&self) -> Option<Self> {
        let refine = |basis: &UniformBasis| {
            let spans = 2 * basis.spans();
            let spacing = 0.5 * basis.spacing();
            if basis.is_periodic() {
                UniformBasis::periodic(basis.degree(), spans, spacing)
            } else if basis.is_clamped() {
                UniformBasis::open(basis.degree(), spans, spacing)
            } else {
                UniformBasis::unclamped(basis.degree(), spans, spacing)
            }
        };
        Self::from_bases(
            refine(&self.basis_x),
            refine(&self.basis_y),
            self.geometry.clone(),
            self.rule.len(),
        )
    }

    /// The coefficients whose spline is the L2 projection over the patch of
    /// a function of the parametric point.
    pub fn project_parametric(&self, f: impl Fn(Point2) -> f64) -> Vec<f64> {
        let parameters = self.parametric_points();
        let mut right = vec![0.0; self.degrees_of_freedom()];
        self.for_each_sample(|index, stencil| {
            let value = f(parameters[index]) * stencil.weight * self.density[index];
            for (node, basis) in stencil.nodes().iter().zip(stencil.values()) {
                right[*node] += value * basis;
            }
        });
        self.conjugate_gradients(
            |v| self.apply_mass(v),
            &right,
            &self.lumped.iter().map(|d| 1.0 / d).collect::<Vec<_>>(),
        )
    }

    /// The field `u` of this patch as coefficients of `fine`, a refinement
    /// of it: exact up to the projection's tolerance, since the coarse
    /// space lies in the fine one.
    pub fn refine_field(&self, fine: &Self, u: &[f64]) -> Vec<f64> {
        fine.project_parametric(|parameter| self.evaluate_field(u, parameter))
    }

    /// The spline `w` whose gradient the flux is: `b` evolves only by
    /// gradients of fields, so with a gradient start it stays one, and
    /// `K w = Gᵀ W κ b` recovers `w` up to a constant, here with zero
    /// weighted mean. The residual says how far `b` is from a gradient.
    pub fn flux_potential(&self, b: &[Point2]) -> (Vec<f64>, f64) {
        let right = self.divergence(b);
        let diagonal: Vec<f64> = {
            let mut k = vec![0.0; self.degrees_of_freedom()];
            self.for_each_sample(|index, stencil| {
                let weight = stencil.weight * self.stiffness[index];
                for (node, g) in stencil.nodes().iter().zip(stencil.gradients()) {
                    k[*node] += weight * g.dot(*g);
                }
            });
            k.iter().map(|k| 1.0 / k.max(f64::MIN_POSITIVE)).collect()
        };
        let mut w =
            self.conjugate_gradients(|v| self.divergence(&self.gradient(v)), &right, &diagonal);
        let total: f64 = self.lumped.iter().sum();
        let mean = dot(&w, &self.lumped) / total;
        for value in &mut w {
            *value -= mean;
        }
        let gradient = self.gradient(&w);
        let residual: f64 = gradient
            .iter()
            .zip(b)
            .zip(self.sample_weights())
            .map(|((g, b), weight)| weight * (*g - *b).dot(*g - *b))
            .sum();
        let norm: f64 = b
            .iter()
            .zip(self.sample_weights())
            .map(|(b, weight)| weight * b.dot(*b))
            .sum();
        (w, (residual / norm.max(f64::MIN_POSITIVE)).sqrt())
    }

    /// The flux `b` of this patch at the samples of `fine`, a refinement of
    /// it: the flux potential is carried as a field, exactly, and
    /// differentiated on the fine patch, so a flux that is a gradient is
    /// carried exactly.
    pub fn refine_flux(&self, fine: &Self, b: &[Point2]) -> Vec<Point2> {
        let (potential, _) = self.flux_potential(b);
        fine.gradient(&self.refine_field(fine, &potential))
    }

    /// Every sample's Jacobian determinant, one under the affine map.
    pub fn sample_determinants(&self) -> Vec<f64> {
        if self.jacobians.is_empty() {
            vec![1.0; self.samples()]
        } else {
            self.jacobians.iter().map(|j| j.determinant).collect()
        }
    }

    /// Visits every sample's parametric point in sample order, stopping at
    /// the first error.
    fn for_each_parametric_sample<E>(
        &self,
        mut visit: impl FnMut(Point2) -> Result<(), E>,
    ) -> Result<(), E> {
        let q = self.rule.len();
        for sx in 0..self.basis_x.spans() {
            for gx in 0..q {
                let xi = (sx as f64 + self.rule[gx].0) * self.basis_x.spacing();
                for sy in 0..self.basis_y.spans() {
                    for gy in 0..q {
                        let eta = (sy as f64 + self.rule[gy].0) * self.basis_y.spacing();
                        visit(Point2::new(xi, eta))?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Visits every sample in index order with its stencil, in physical
    /// components.
    pub fn for_each_sample(&self, mut visit: impl FnMut(usize, &SampleStencil)) {
        let p = self.degree();
        let q = self.rule.len();
        let area = self.basis_x.spacing() * self.basis_y.spacing();
        let mut stencil = SampleStencil {
            count: (p + 1) * (p + 1),
            weight: 0.0,
            nodes: [0; MAX_LOCAL_2D],
            values: [0.0; MAX_LOCAL_2D],
            gradients: [Point2::default(); MAX_LOCAL_2D],
        };
        let mut index = 0;
        for sx in 0..self.basis_x.spans() {
            for gx in 0..q {
                let (nx, dnx) = &self.table_x[sx * q + gx];
                for sy in 0..self.basis_y.spans() {
                    for gy in 0..q {
                        let (ny, dny) = &self.table_y[sy * q + gy];
                        let jacobian = self.jacobians.get(index).copied();
                        stencil.weight = self.rule[gx].1
                            * self.rule[gy].1
                            * area
                            * jacobian.map_or(1.0, |j| j.determinant);
                        let mut local = 0;
                        for a in 0..=p {
                            let ix = self.basis_x.function(sx, a);
                            for b in 0..=p {
                                let iy = self.basis_y.function(sy, b);
                                stencil.nodes[local] = self.node(ix, iy);
                                stencil.values[local] = nx[a] * ny[b];
                                let parametric = Point2::new(dnx[a] * ny[b], nx[a] * dny[b]);
                                stencil.gradients[local] = match jacobian {
                                    None => parametric,
                                    Some(j) => {
                                        let m = j.inverse_transpose;
                                        Point2::new(
                                            m[0] * parametric.x + m[1] * parametric.y,
                                            m[2] * parametric.x + m[3] * parametric.y,
                                        )
                                    }
                                };
                                local += 1;
                            }
                        }
                        visit(index, &stencil);
                        index += 1;
                    }
                }
            }
        }
    }

    /// `M u` with the sampled density, through the samples, plus any mass
    /// scaling.
    pub fn apply_mass(&self, u: &[f64]) -> Vec<f64> {
        let mut out: Vec<f64> = self
            .mass_scaling
            .iter()
            .zip(u)
            .map(|(m, u)| m * u)
            .collect();
        self.for_each_sample(|index, stencil| {
            let value = stencil
                .nodes()
                .iter()
                .zip(stencil.values())
                .map(|(node, value)| value * u[*node])
                .sum::<f64>()
                * stencil.weight
                * self.density[index];
            for (node, basis) in stencil.nodes().iter().zip(stencil.values()) {
                out[*node] += value * basis;
            }
        });
        out
    }

    /// The field's value at every sample.
    pub fn sample_values(&self, u: &[f64]) -> Vec<f64> {
        let mut out = vec![0.0; self.samples()];
        self.for_each_sample(|index, stencil| {
            out[index] = stencil
                .nodes()
                .iter()
                .zip(stencil.values())
                .map(|(node, value)| value * u[*node])
                .sum();
        });
        out
    }

    /// The field's gradient at every sample.
    pub fn gradient(&self, u: &[f64]) -> Vec<Point2> {
        let mut out = vec![Point2::default(); self.samples()];
        self.for_each_sample(|index, stencil| {
            let mut gradient = Point2::default();
            for (node, basis) in stencil.nodes().iter().zip(stencil.gradients()) {
                gradient = gradient + *basis * u[*node];
            }
            out[index] = gradient;
        });
        out
    }

    /// `Gᵀ W κ b`: the weighted gather of sample fluxes to the
    /// coefficients, each through its sample's stiffness; with unit
    /// stiffness the exact transpose of [`Self::gradient`].
    pub fn divergence(&self, b: &[Point2]) -> Vec<f64> {
        let mut out = vec![0.0; self.degrees_of_freedom()];
        self.for_each_sample(|index, stencil| {
            let flux = b[index] * (stencil.weight * self.stiffness[index]);
            for (node, basis) in stencil.nodes().iter().zip(stencil.gradients()) {
                out[*node] += basis.dot(flux);
            }
        });
        out
    }

    /// The field `u = P Q` under a mass treatment.
    pub fn field(&self, q: &[f64], treatment: MassTreatment) -> Vec<f64> {
        match treatment {
            MassTreatment::Lumped { sweeps } => {
                let mut u: Vec<f64> = q.iter().zip(&self.lumped).map(|(q, d)| q / d).collect();
                for _ in 0..sweeps {
                    let residual = self.apply_mass(&u);
                    for ((u, d), (q, mu)) in
                        u.iter_mut().zip(&self.lumped).zip(q.iter().zip(&residual))
                    {
                        *u += (q - mu) / d;
                    }
                }
                u
            }
            MassTreatment::Consistent => self.conjugate_gradients(
                |v| self.apply_mass(v),
                q,
                &self.lumped.iter().map(|d| 1.0 / d).collect::<Vec<_>>(),
            ),
        }
    }

    /// `Q = P⁻¹ u`: the flux whose field is `u`.
    pub fn flux_of_field(&self, u: &[f64], treatment: MassTreatment) -> Vec<f64> {
        match treatment {
            MassTreatment::Lumped { sweeps: 0 } => {
                u.iter().zip(&self.lumped).map(|(u, d)| u * d).collect()
            }
            MassTreatment::Lumped { .. } => {
                self.conjugate_gradients(|v| self.field(v, treatment), u, &self.lumped)
            }
            MassTreatment::Consistent => self.apply_mass(u),
        }
    }

    /// Preconditioned conjugate gradients on a symmetric positive-definite
    /// operator, to roundoff.
    fn conjugate_gradients(
        &self,
        apply: impl Fn(&[f64]) -> Vec<f64>,
        right: &[f64],
        preconditioner: &[f64],
    ) -> Vec<f64> {
        let n = right.len();
        let mut x = vec![0.0; n];
        let mut r = right.to_vec();
        let mut z: Vec<f64> = r.iter().zip(preconditioner).map(|(r, p)| r * p).collect();
        let mut p = z.clone();
        let mut rz = dot(&r, &z);
        let norm = dot(right, right).sqrt();
        if norm == 0.0 {
            return x;
        }
        for _ in 0..(4 * n + 10) {
            let ap = apply(&p);
            let pap = dot(&p, &ap);
            if pap <= 0.0 {
                break;
            }
            let alpha = rz / pap;
            for (x, p) in x.iter_mut().zip(&p) {
                *x += alpha * p;
            }
            for (r, ap) in r.iter_mut().zip(&ap) {
                *r -= alpha * ap;
            }
            if dot(&r, &r).sqrt() <= 1.0e-14 * norm {
                break;
            }
            z = r.iter().zip(preconditioner).map(|(r, p)| r * p).collect();
            let next = dot(&r, &z);
            let beta = next / rz;
            rz = next;
            for (p, z) in p.iter_mut().zip(&z) {
                *p = z + beta * *p;
            }
        }
        x
    }

    /// The coefficients whose spline is the L2 projection of
    /// `f(x) g(y)`, one one-dimensional projection per direction; on the
    /// affine map only, where `x` and `y` follow `ξ` and `η`.
    pub fn project_separable(&self, f: impl Fn(f64) -> f64, g: impl Fn(f64) -> f64) -> Vec<f64> {
        let GeometryMap::Affine { origin } = self.geometry else {
            panic!("a separable projection needs the affine map");
        };
        assert!(
            !self.is_condensed(),
            "a separable projection needs the full space"
        );
        let cx = project_1d(&self.basis_x, |t| f(origin.x + t));
        let cy = project_1d(&self.basis_y, |t| g(origin.y + t));
        self.separable(&cx, &cy)
    }

    /// The coefficients whose spline is the L2 projection of `f` over the
    /// patch, by conjugate gradients on the consistent mass.
    pub fn project(&self, f: impl Fn(Point2) -> f64) -> Vec<f64> {
        let mut right = vec![0.0; self.degrees_of_freedom()];
        self.for_each_sample(|index, stencil| {
            let value = f(self.points[index]) * stencil.weight;
            for (node, basis) in stencil.nodes().iter().zip(stencil.values()) {
                right[*node] += value * basis;
            }
        });
        self.conjugate_gradients(
            |v| self.apply_mass(v),
            &right,
            &self.lumped.iter().map(|d| 1.0 / d).collect::<Vec<_>>(),
        )
    }

    /// The coefficients that read `f(x) g(y)` at the Greville points: nodal
    /// interpolation for degree one, a quasi-interpolant above.
    pub fn interpolate_separable(
        &self,
        f: impl Fn(f64) -> f64,
        g: impl Fn(f64) -> f64,
    ) -> Vec<f64> {
        let GeometryMap::Affine { origin } = self.geometry else {
            panic!("a separable interpolation needs the affine map");
        };
        assert!(
            !self.is_condensed(),
            "a separable interpolation needs the full space"
        );
        let cx: Vec<f64> = self
            .basis_x
            .greville()
            .into_iter()
            .map(|t| f(origin.x + t))
            .collect();
        let cy: Vec<f64> = self
            .basis_y
            .greville()
            .into_iter()
            .map(|t| g(origin.y + t))
            .collect();
        self.separable(&cx, &cy)
    }

    fn separable(&self, cx: &[f64], cy: &[f64]) -> Vec<f64> {
        let mut out = vec![0.0; self.degrees_of_freedom()];
        for (ix, cx) in cx.iter().enumerate() {
            for (iy, cy) in cy.iter().enumerate() {
                out[self.node(ix, iy)] = cx * cy;
            }
        }
        out
    }

    /// The largest eigenvalue of `P K`, by power iteration from a fixed
    /// pseudo-random start, and whether it converged to `1e-7` relative.
    /// The estimate approaches the eigenvalue from below.
    pub fn largest_eigenvalue(&self, treatment: MassTreatment, iterations: usize) -> (f64, bool) {
        let n = self.degrees_of_freedom();
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut v: Vec<f64> = (0..n)
            .map(|_| {
                seed = seed
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
            })
            .collect();
        normalize(&mut v);
        let mut lambda = 0.0;
        let mut converged = false;
        for _ in 0..iterations {
            let b = self.gradient(&v);
            let kv = self.divergence(&b);
            let mut next = self.field(&kv, treatment);
            let estimate = dot(&next, &next).sqrt();
            normalize(&mut next);
            v = next;
            if (estimate - lambda).abs() <= 1.0e-7 * estimate {
                lambda = estimate;
                converged = true;
                break;
            }
            lambda = estimate;
        }
        (lambda, converged)
    }

    /// The `(Q, b)` energy of a field at rest, `½ uᵀ M u` with the
    /// consistent mass: the L2 inner product of the spline with itself.
    pub fn inner_product(&self, u: &[f64], v: &[f64]) -> f64 {
        dot(u, &self.apply_mass(v))
    }
}

/// What a standing mode's two latest field levels say about its phase and
/// amplitude, as the triangle convergence example measures them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModeMeasurement {
    /// The numerical phase less the exact `ω t`, wrapped to `(-π, π]`.
    pub phase_error: f64,
    /// The mode's amplitude less one.
    pub amplitude_error: f64,
    /// The L2 distance from the exact `cos(ω t)` mode, over the mode's norm.
    pub relative_l2: f64,
}

/// When a mode is read: its exact frequency, the frequency the readout
/// separates the sine component with, the step and the time.
///
/// Two field levels fix a mode's phase only once its frequency is assumed.
/// The triangle convergence example assumes the exact one, which offsets
/// the phase by about the relative frequency error, in radians; passing the
/// discrete frequency as `readout` makes the separation exact for it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModeClock {
    pub omega: f64,
    pub readout: f64,
    pub dt: f64,
    pub time: f64,
}

/// Measures a mode from the field at `clock.time` and the field one step
/// earlier, projecting with the consistent mass.
pub fn measure_mode(
    patch: &SplinePatch,
    mode: &[f64],
    current: &[f64],
    previous: &[f64],
    clock: ModeClock,
) -> ModeMeasurement {
    let mass_mode = patch.apply_mass(mode);
    let norm = dot(mode, &mass_mode);
    let current_mode = dot(current, &mass_mode) / norm;
    let previous_mode = dot(previous, &mass_mode) / norm;
    let angle = clock.readout * clock.dt;
    let sine = if angle.sin().abs() > 1.0e-12 {
        (previous_mode - current_mode * angle.cos()) / angle.sin()
    } else {
        0.0
    };
    let numerical_phase = sine.atan2(current_mode);
    let exact_phase = clock.omega * clock.time;
    let phase_error = wrap_angle(numerical_phase - exact_phase);
    let amplitude_error = current_mode.hypot(sine) - 1.0;
    let exact_factor = exact_phase.cos();
    let difference: Vec<f64> = current
        .iter()
        .zip(mode)
        .map(|(actual, mode)| actual - exact_factor * mode)
        .collect();
    let relative_l2 = (patch.inner_product(&difference, &difference) / norm).sqrt();
    ModeMeasurement {
        phase_error,
        amplitude_error,
        relative_l2,
    }
}

pub fn wrap_angle(angle: f64) -> f64 {
    (angle + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

fn normalize(v: &mut [f64]) {
    let norm = dot(v, v).sqrt();
    if norm > 0.0 {
        for value in v {
            *value /= norm;
        }
    }
}

/// The coefficients of the L2 projection of `f` onto a one-dimensional basis,
/// by a dense Cholesky solve of the consistent mass under a five-point rule.
pub fn project_1d(basis: &UniformBasis, f: impl Fn(f64) -> f64) -> Vec<f64> {
    let n = basis.functions();
    let p = basis.degree();
    let rule = gauss_rule(5).unwrap();
    let mut mass = vec![0.0; n * n];
    let mut right = vec![0.0; n];
    for span in 0..basis.spans() {
        for &(abscissa, weight) in &rule {
            let parameter = (span as f64 + abscissa) * basis.spacing();
            let (values, _) = basis.evaluate(span, parameter);
            let weight = weight * basis.spacing();
            let value = f(parameter);
            for a in 0..=p {
                let i = basis.function(span, a);
                right[i] += weight * values[a] * value;
                for b in 0..=p {
                    let j = basis.function(span, b);
                    mass[i * n + j] += weight * values[a] * values[b];
                }
            }
        }
    }
    cholesky_solve(&mass, &right)
}

fn cholesky_solve(matrix: &[f64], right: &[f64]) -> Vec<f64> {
    let n = right.len();
    let mut l = vec![0.0; n * n];
    for i in 0..n {
        for j in 0..=i {
            let mut sum = matrix[i * n + j];
            for k in 0..j {
                sum -= l[i * n + k] * l[j * n + k];
            }
            if i == j {
                assert!(sum > 0.0, "the projection mass is not positive definite");
                l[i * n + i] = sum.sqrt();
            } else {
                l[i * n + j] = sum / l[j * n + j];
            }
        }
    }
    let mut y = vec![0.0; n];
    for i in 0..n {
        let mut sum = right[i];
        for k in 0..i {
            sum -= l[i * n + k] * y[k];
        }
        y[i] = sum / l[i * n + i];
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let mut sum = y[i];
        for k in i + 1..n {
            sum -= l[k * n + i] * x[k];
        }
        x[i] = sum / l[i * n + i];
    }
    x
}

/// A Störmer step on a patch with unit material and natural reflecting
/// walls: `Q̇ = -Gᵀ W b`, `ḃ = G ũ`, `u = P Q`, with `b` half a step ahead of
/// `Q`; `ũ` is `u` under leapfrog and the modified field under fourth order.
pub struct PatchStepper<'a> {
    patch: &'a SplinePatch,
    treatment: MassTreatment,
    integrator: Integrator,
    dt: f64,
    q: Vec<f64>,
    q_previous: Vec<f64>,
    b: Vec<Point2>,
    u: Vec<f64>,
    u_previous: Vec<f64>,
    /// The field the last drift used.
    drift_field: Vec<f64>,
    /// The field the drift before that used.
    drift_field_previous: Vec<f64>,
    steps: u64,
}

impl<'a> PatchStepper<'a> {
    /// Leapfrog from the field `u` at rest.
    pub fn new(patch: &'a SplinePatch, treatment: MassTreatment, dt: f64, u: Vec<f64>) -> Self {
        Self::with_integrator(patch, treatment, Integrator::Leapfrog, dt, u)
    }

    /// Starts from the field `u` at rest under the given integrator.
    pub fn with_integrator(
        patch: &'a SplinePatch,
        treatment: MassTreatment,
        integrator: Integrator,
        dt: f64,
        u: Vec<f64>,
    ) -> Self {
        Self::with_state(patch, treatment, integrator, dt, u, None)
    }

    /// Starts from the field `u` and, if given, the flux `b` at the field's
    /// own instant, which the stepper staggers half a step ahead itself; at
    /// rest otherwise. A handoff between steppers passes
    /// [`Self::centered_flux`] here, whatever the steps on either side.
    pub fn with_state(
        patch: &'a SplinePatch,
        treatment: MassTreatment,
        integrator: Integrator,
        dt: f64,
        u: Vec<f64>,
        b_centered: Option<Vec<Point2>>,
    ) -> Self {
        let q = patch.flux_of_field(&u, treatment);
        let u = patch.field(&q, treatment);
        let mut stepper = Self {
            patch,
            treatment,
            integrator,
            dt,
            q_previous: q.clone(),
            q,
            b: Vec::new(),
            u_previous: u.clone(),
            u,
            drift_field: Vec::new(),
            drift_field_previous: Vec::new(),
            steps: 0,
        };
        let drift_field = stepper.drift_field();
        let half_kick = stepper.patch.gradient(&drift_field);
        stepper.b = match b_centered {
            Some(centered) => centered
                .into_iter()
                .zip(&half_kick)
                .map(|(b, g)| b + *g * (0.5 * dt))
                .collect(),
            None => half_kick.iter().map(|g| *g * (0.5 * dt)).collect(),
        };
        stepper.drift_field_previous = drift_field.clone();
        stepper.drift_field = drift_field;
        stepper
    }

    /// The field `b` drifts on at the current level.
    fn drift_field(&self) -> Vec<f64> {
        match self.integrator {
            Integrator::Leapfrog => self.u.clone(),
            Integrator::FourthOrder => {
                let force = self.patch.divergence(&self.patch.gradient(&self.u));
                let modified: Vec<f64> = self
                    .q
                    .iter()
                    .zip(&force)
                    .map(|(q, f)| q - self.dt * self.dt / 12.0 * f)
                    .collect();
                self.patch.field(&modified, self.treatment)
            }
        }
    }

    pub fn step(&mut self) {
        let force = self.patch.divergence(&self.b);
        std::mem::swap(&mut self.q, &mut self.q_previous);
        for ((q, previous), f) in self.q.iter_mut().zip(&self.q_previous).zip(&force) {
            *q = previous - self.dt * f;
        }
        std::mem::swap(&mut self.u, &mut self.u_previous);
        self.u = self.patch.field(&self.q, self.treatment);
        let drift_field = self.drift_field();
        let gradient = self.patch.gradient(&drift_field);
        for (b, g) in self.b.iter_mut().zip(&gradient) {
            *b = *b + *g * self.dt;
        }
        std::mem::swap(&mut self.drift_field, &mut self.drift_field_previous);
        self.drift_field = drift_field;
        self.steps += 1;
    }

    pub fn integrator(&self) -> Integrator {
        self.integrator
    }

    pub fn time(&self) -> f64 {
        self.steps as f64 * self.dt
    }

    pub fn steps(&self) -> u64 {
        self.steps
    }

    pub fn time_step(&self) -> f64 {
        self.dt
    }

    pub fn field(&self) -> &[f64] {
        &self.u
    }

    pub fn previous_field(&self) -> &[f64] {
        &self.u_previous
    }

    /// The flux half a step ahead of the field.
    pub fn flux(&self) -> &[Point2] {
        &self.b
    }

    /// The flux at the field's own instant, `b⁺ − dt/2 · G ũ`, the mean of
    /// the two half-step fluxes around it: what a handoff carries.
    pub fn centered_flux(&self) -> Vec<Point2> {
        self.b
            .iter()
            .zip(self.patch.gradient(&self.drift_field))
            .map(|(b, g)| *b - g * (0.5 * self.dt))
            .collect()
    }

    /// The staggered energy `½ Qᵀ P Q + ½ b⁻ · W b⁺`, which leapfrog
    /// conserves exactly: `b⁻` is the flux half a step before the current
    /// `Q` and `b⁺` the one half a step after. Fourth order conserves
    /// [`Self::conserved_energy`] instead.
    pub fn staggered_energy(&self) -> f64 {
        let kinetic = 0.5 * dot(&self.q, &self.u);
        // `b` holds `b⁺`; `b⁻ = b⁺ - dt G ũ`.
        let gradient = self.patch.gradient(&self.drift_field);
        let potential = self
            .b
            .iter()
            .zip(&gradient)
            .zip(
                self.patch
                    .sample_weights()
                    .iter()
                    .zip(self.patch.sample_stiffness()),
            )
            .map(|((plus, g), (weight, stiffness))| {
                let minus = *plus - *g * self.dt;
                0.5 * weight * stiffness * plus.dot(minus)
            })
            .sum::<f64>();
        kinetic + potential
    }

    /// The energy a Störmer scheme conserves exactly, across the last step:
    /// `½ ΔQ · P ΔQ / dt² + ½ (G uⁿ⁺¹) · W (G ũⁿ)`, where `ũⁿ` is the field the
    /// previous level's drift used, so the potential term carries the
    /// modified stiffness under fourth order. Needs one step taken.
    pub fn conserved_energy(&self) -> f64 {
        let kinetic = self
            .q
            .iter()
            .zip(&self.q_previous)
            .zip(self.u.iter().zip(&self.u_previous))
            .map(|((q, q0), (u, u0))| (q - q0) * (u - u0))
            .sum::<f64>()
            * 0.5
            / (self.dt * self.dt);
        let current = self.patch.gradient(&self.u);
        let previous = self.patch.gradient(&self.drift_field_previous);
        let potential = current
            .iter()
            .zip(&previous)
            .zip(
                self.patch
                    .sample_weights()
                    .iter()
                    .zip(self.patch.sample_stiffness()),
            )
            .map(|((a, b), (w, stiffness))| 0.5 * w * stiffness * a.dot(*b))
            .sum::<f64>();
        kinetic + potential
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TREATMENTS: [MassTreatment; 3] = [
        MassTreatment::Lumped { sweeps: 0 },
        MassTreatment::Lumped { sweeps: 1 },
        MassTreatment::Consistent,
    ];

    fn bases() -> Vec<UniformBasis> {
        let mut out = Vec::new();
        for degree in 1..=MAX_DEGREE {
            out.push(UniformBasis::open(degree, 7, 0.3));
            out.push(UniformBasis::unclamped(degree, 7, 0.3));
            out.push(UniformBasis::periodic(degree, 9, 0.3));
        }
        out
    }

    #[test]
    fn gauss_rules_integrate_their_degree() {
        for points in 1..=5 {
            let rule = gauss_rule(points).unwrap();
            assert_eq!(rule.len(), points);
            for power in 0..2 * points {
                let integral: f64 = rule.iter().map(|(x, w)| w * x.powi(power as i32)).sum();
                let exact = 1.0 / (power as f64 + 1.0);
                assert!(
                    (integral - exact).abs() < 1.0e-14,
                    "{points} points, x^{power}"
                );
            }
        }
    }

    #[test]
    fn the_basis_sums_to_one_and_its_derivatives_to_zero() {
        for basis in bases() {
            for step in 0..200 {
                let t = basis.length() * (step as f64 + 0.37) / 200.0;
                let span = basis.span_of(t);
                let (values, derivatives) = basis.evaluate(span, t);
                let p = basis.degree();
                let sum: f64 = values[..=p].iter().sum();
                let slope: f64 = derivatives[..=p].iter().sum();
                assert!((sum - 1.0).abs() < 1.0e-13, "degree {p}: {sum}");
                assert!(slope.abs() < 1.0e-11, "degree {p}: {slope}");
                assert!(values[..=p].iter().all(|v| *v >= -1.0e-15));
            }
        }
    }

    #[test]
    fn greville_coefficients_reproduce_a_line() {
        for basis in bases() {
            let greville = basis.greville();
            let p = basis.degree();
            let line = |t: f64| 2.0 * t - 1.0;
            for step in 0..50 {
                let t = basis.length() * (step as f64 + 0.5) / 50.0;
                if basis.is_periodic()
                    && (t < p as f64 * 0.3 || t > basis.length() - p as f64 * 0.3)
                {
                    // A periodic basis wraps its line around.
                    continue;
                }
                let span = basis.span_of(t);
                let (values, derivatives) = basis.evaluate(span, t);
                let value: f64 = (0..=p)
                    .map(|a| values[a] * line(greville[basis.function(span, a)]))
                    .sum();
                let slope: f64 = (0..=p)
                    .map(|a| derivatives[a] * line(greville[basis.function(span, a)]))
                    .sum();
                assert!((value - line(t)).abs() < 1.0e-12, "degree {p}");
                assert!((slope - 2.0).abs() < 1.0e-10, "degree {p}");
            }
        }
    }

    #[test]
    fn the_periodic_symbols_match_the_closed_forms() {
        let rule = gauss_rule(4).unwrap();
        let cubic = PeriodicSymbols::new(3, 1.0, &rule);
        assert!((cubic.mass[0] - 151.0 / 315.0).abs() < 1.0e-13);
        assert!((cubic.mass[1] - 397.0 / 1680.0).abs() < 1.0e-13);
        assert!((cubic.mass[2] - 1.0 / 42.0).abs() < 1.0e-13);
        assert!((cubic.mass[3] - 1.0 / 5040.0).abs() < 1.0e-13);
        assert!((cubic.stiffness[0] - 2.0 / 3.0).abs() < 1.0e-13);
        assert!((cubic.stiffness[1] + 1.0 / 8.0).abs() < 1.0e-13);
        assert!((cubic.stiffness[2] + 1.0 / 5.0).abs() < 1.0e-13);
        assert!((cubic.stiffness[3] + 1.0 / 120.0).abs() < 1.0e-13);
        assert!((cubic.lumped() - 1.0).abs() < 1.0e-13);
        assert!((cubic.mass(0.0) - 1.0).abs() < 1.0e-13);
        assert!(cubic.stiffness(0.0).abs() < 1.0e-13);
        let linear = PeriodicSymbols::new(1, 0.5, &gauss_rule(2).unwrap());
        let theta = 0.7;
        assert!((linear.mass(theta) - 0.5 * (2.0 / 3.0 + theta.cos() / 3.0)).abs() < 1.0e-13);
        assert!((linear.stiffness(theta) - (2.0 - 2.0 * theta.cos()) / 0.5).abs() < 1.0e-13);
        // Lumped linear elements run slow by θ²/24, consistent ones fast.
        let lumped = linear.frequency_ratio(theta, 0.0, Some(0));
        let consistent = linear.frequency_ratio(theta, 0.0, None);
        assert!((lumped - 2.0 * (theta / 2.0).sin() / theta).abs() < 1.0e-12);
        assert!(consistent > 1.0 && lumped < 1.0);
    }

    #[test]
    fn jacobi_sweeps_converge_to_the_consistent_field() {
        let rule = gauss_rule(4).unwrap();
        let cubic = PeriodicSymbols::new(3, 1.0, &rule);
        let consistent = cubic.field_symbol(0.4, 0.2, None);
        let mut previous = f64::INFINITY;
        for sweeps in 0..6 {
            let error = (cubic.field_symbol(0.4, 0.2, Some(sweeps)) - consistent).abs();
            assert!(error < previous, "sweep {sweeps}: {error} after {previous}");
            previous = error;
        }
        // Each sweep buys two orders in the wavenumber.
        for sweeps in 0..3 {
            let error = |theta: f64| {
                (cubic.field_symbol(theta, 0.0, Some(sweeps))
                    - cubic.field_symbol(theta, 0.0, None))
                .abs()
            };
            let order = (error(0.4) / error(0.2)).log2();
            let expected = 2.0 * (sweeps as f64 + 1.0);
            assert!(
                (order - expected).abs() < 0.35,
                "{sweeps} sweeps: order {order}"
            );
        }
    }

    fn small_patch(degree: usize, points: usize) -> SplinePatch {
        SplinePatch::new(
            degree,
            Point2::new(-1.0, -0.5),
            Point2::new(2.0, 1.0),
            [9, 6],
            points,
        )
        .unwrap()
    }

    #[test]
    fn the_lumped_mass_is_the_area_and_the_consistent_mass_agrees() {
        for degree in 1..=3 {
            let patch = small_patch(degree, 4);
            let total: f64 = patch.lumped().iter().sum();
            assert!((total - 2.0).abs() < 1.0e-12, "degree {degree}: {total}");
            let ones = vec![1.0; patch.degrees_of_freedom()];
            let row_sums = patch.apply_mass(&ones);
            for (row, lumped) in row_sums.iter().zip(patch.lumped()) {
                assert!((row - lumped).abs() < 1.0e-13);
            }
        }
    }

    /// The clamped and unclamped bases span the same space, so their
    /// consistent-mass spectra agree while their lumped ones differ.
    #[test]
    fn the_unclamped_basis_spans_the_clamped_space() {
        for degree in 1..=3 {
            let clamped = small_patch(degree, 3);
            let unclamped = SplinePatch::unclamped(
                degree,
                Point2::new(-1.0, -0.5),
                Point2::new(2.0, 1.0),
                [9, 6],
                3,
            )
            .unwrap();
            assert_eq!(clamped.degrees_of_freedom(), unclamped.degrees_of_freedom());
            let total: f64 = unclamped.lumped().iter().sum();
            assert!((total - 2.0).abs() < 1.0e-12);
            let (a, _) = clamped.largest_eigenvalue(MassTreatment::Consistent, 4000);
            let (b, _) = unclamped.largest_eigenvalue(MassTreatment::Consistent, 4000);
            assert!(
                (a - b).abs() < 1.0e-5 * a,
                "degree {degree}: {a} against {b}"
            );
            // A projected line is the same spline in either basis.
            let line = |x: f64| 3.0 * x + 1.0;
            for g in unclamped.gradient(&unclamped.project_separable(line, |_| 1.0)) {
                assert!((g.x - 3.0).abs() < 1.0e-9 && g.y.abs() < 1.0e-9, "{g:?}");
            }
        }
    }

    fn curved_small(geometry: GeometryMap, size: Point2) -> SplinePatch {
        SplinePatch::curved(2, geometry, size, [9, 6], 3, false).unwrap()
    }

    /// The flat surface and a Coons patch of straight sides are the affine
    /// box in other clothes: same samples, weights and lumped mass.
    #[test]
    fn a_flat_surface_and_a_square_coons_patch_are_the_affine_box() {
        let origin = Point2::new(-1.0, -0.5);
        let size = Point2::new(2.0, 1.0);
        let affine =
            SplinePatch::curved(2, GeometryMap::Affine { origin }, size, [9, 6], 3, false).unwrap();
        let corner = |x: f64, y: f64| origin + Point2::new(x, y);
        let line = |a: Point2, b: Point2| Box::new(CoonsSide::Line { from: a, to: b });
        let others = [
            curved_small(GeometryMap::flat_surface(origin, size, [4, 3]), size),
            curved_small(
                GeometryMap::Coons {
                    bottom: line(corner(0.0, 0.0), corner(2.0, 0.0)),
                    right: line(corner(2.0, 0.0), corner(2.0, 1.0)),
                    top: line(corner(0.0, 1.0), corner(2.0, 1.0)),
                    left: line(corner(0.0, 0.0), corner(0.0, 1.0)),
                },
                size,
            ),
        ];
        for other in &others {
            for (a, b) in affine.sample_points().iter().zip(other.sample_points()) {
                assert!((*a - *b).norm() < 1.0e-12);
            }
            for (a, b) in affine.sample_weights().iter().zip(other.sample_weights()) {
                assert!((a - b).abs() < 1.0e-12);
            }
            for (a, b) in affine.lumped().iter().zip(other.lumped()) {
                assert!((a - b).abs() < 1.0e-12);
            }
            let u: Vec<f64> = (0..affine.degrees_of_freedom())
                .map(|i| ((i * 13) % 7) as f64)
                .collect();
            for (a, b) in affine.gradient(&u).iter().zip(other.gradient(&u)) {
                assert!((*a - b).norm() < 1.0e-10);
            }
        }
    }

    /// A Coons disk has the disk's area, a positive Jacobian at every sample
    /// that vanishes toward the corners, and keeps the transpose structure.
    #[test]
    fn a_coons_disk_has_the_right_area_and_singular_corners() {
        let size = Point2::new(1.0, 1.0);
        let disk = SplinePatch::curved(
            2,
            GeometryMap::coons_ellipse(Point2::new(0.3, -0.2), Point2::new(1.0, 1.0)),
            size,
            [12, 12],
            3,
            false,
        )
        .unwrap();
        let area: f64 = disk.sample_weights().iter().sum();
        assert!((area - std::f64::consts::PI).abs() < 2.0e-4, "{area}");
        let total: f64 = disk.lumped().iter().sum();
        assert!((total - area).abs() < 1.0e-12);
        let determinants = disk.sample_determinants();
        let least = determinants.iter().cloned().fold(f64::INFINITY, f64::min);
        let most = determinants.iter().cloned().fold(0.0, f64::max);
        assert!(least > 0.0 && least < 0.2 * most, "{least} {most}");
        for p in disk.sample_points() {
            assert!((*p - Point2::new(0.3, -0.2)).norm() < 1.0 + 1.0e-12);
        }
        // Transpose, with the Jacobians in the weights and gradients.
        let n = disk.degrees_of_freedom();
        let u: Vec<f64> = (0..n).map(|i| ((i * 7919) % 101) as f64 / 101.0).collect();
        let b: Vec<Point2> = (0..disk.samples())
            .map(|i| Point2::new(((i * 31) % 17) as f64 / 17.0, ((i * 13) % 23) as f64 / 23.0))
            .collect();
        let left: f64 = disk
            .gradient(&u)
            .iter()
            .zip(&b)
            .zip(disk.sample_weights())
            .map(|((g, b), w)| w * g.dot(*b))
            .sum();
        let right = dot(&u, &disk.divergence(&b));
        assert!((left - right).abs() < 1.0e-12 * left.abs().max(1.0));
        // A projected linear function has the linear function's gradient up
        // to the projection's own error, which the mapped space makes small
        // but not zero.
        let u = disk.project(|p| 3.0 * p.x + 1.0);
        let worst = disk
            .gradient(&u)
            .iter()
            .map(|g| (*g - Point2::new(3.0, 0.0)).norm())
            .fold(0.0, f64::max);
        assert!(worst < 0.3, "{worst}");
    }

    #[test]
    fn condensed_corners_keep_the_area_and_the_structure() {
        let size = Point2::new(1.0, 1.0);
        let full = SplinePatch::curved(
            2,
            GeometryMap::coons_ellipse(Point2::default(), Point2::new(1.0, 1.0)),
            size,
            [12, 12],
            3,
            false,
        )
        .unwrap();
        let condensed = full.clone().with_condensed_corners(3);
        assert_eq!(
            condensed.degrees_of_freedom(),
            full.degrees_of_freedom() - 4 * (9 - 1)
        );
        let total: f64 = condensed.lumped().iter().sum();
        assert!((total - std::f64::consts::PI).abs() < 2.0e-4);
        // A constant is still in the space and its gradient is still zero.
        let ones = condensed.project(|_| 1.0);
        assert!(ones.iter().all(|c| (c - 1.0).abs() < 1.0e-8));
        assert!(condensed.gradient(&ones).iter().all(|g| g.norm() < 1.0e-9));
        // The lumped mass of a corner's function is the sum of its parts.
        let least_full = full.lumped().iter().cloned().fold(f64::INFINITY, f64::min);
        let least_condensed = condensed
            .lumped()
            .iter()
            .cloned()
            .fold(f64::INFINITY, f64::min);
        assert!(
            least_condensed > 5.0 * least_full,
            "{least_full} {least_condensed}"
        );
        let n = condensed.degrees_of_freedom();
        let u: Vec<f64> = (0..n).map(|i| ((i * 7919) % 101) as f64 / 101.0).collect();
        let b: Vec<Point2> = (0..condensed.samples())
            .map(|i| Point2::new(((i * 31) % 17) as f64 / 17.0, ((i * 13) % 23) as f64 / 23.0))
            .collect();
        let left: f64 = condensed
            .gradient(&u)
            .iter()
            .zip(&b)
            .zip(condensed.sample_weights())
            .map(|((g, b), w)| w * g.dot(*b))
            .sum();
        let right = dot(&u, &condensed.divergence(&b));
        assert!((left - right).abs() < 1.0e-12 * left.abs().max(1.0));
    }

    #[test]
    fn mass_scaling_caps_the_corner_frequencies_and_keeps_the_row_sums() {
        let size = Point2::new(1.0, 1.0);
        let disk = SplinePatch::curved(
            2,
            GeometryMap::coons_ellipse(Point2::default(), Point2::new(1.0, 1.0)),
            size,
            [12, 12],
            3,
            true,
        )
        .unwrap();
        let before = disk.frequency_bounds();
        let scaled = disk.clone().with_mass_scaling(2.0);
        let after = scaled.frequency_bounds();
        let median = {
            let mut sorted = before.clone();
            sorted.sort_by(f64::total_cmp);
            sorted[sorted.len() / 2]
        };
        assert!(before.iter().cloned().fold(0.0, f64::max) > 4.0 * median);
        assert!(after.iter().all(|b| *b <= 2.0 * median * (1.0 + 1.0e-9)));
        let count = scaled.scaled_count();
        assert!(
            count > 0 && count < scaled.degrees_of_freedom() / 2,
            "{count}"
        );
        let ones = vec![1.0; scaled.degrees_of_freedom()];
        for (row, lumped) in scaled.apply_mass(&ones).iter().zip(scaled.lumped()) {
            assert!((row - lumped).abs() < 1.0e-12);
        }
        let (plain, _) = disk.largest_eigenvalue(MassTreatment::Lumped { sweeps: 2 }, 3000);
        let (capped, _) = scaled.largest_eigenvalue(MassTreatment::Lumped { sweeps: 2 }, 3000);
        assert!(capped < 0.25 * plain, "{plain} {capped}");
    }

    #[test]
    fn a_c0_knot_adds_functions_and_lets_a_kink_through() {
        for unclamped in [false, true] {
            let smooth = if unclamped {
                UniformBasis::unclamped(2, 8, 0.25)
            } else {
                UniformBasis::open(2, 8, 0.25)
            };
            let kinked = UniformBasis::with_c0_knot(2, 8, 0.25, 4, unclamped);
            assert_eq!(kinked.functions(), smooth.functions() + 1);
            for step in 0..100 {
                let t = 2.0 * (step as f64 + 0.5) / 100.0;
                let span = kinked.span_of(t);
                let (values, derivatives) = kinked.evaluate(span, t);
                assert!((values[..3].iter().sum::<f64>() - 1.0).abs() < 1.0e-13);
                assert!(derivatives[..3].iter().sum::<f64>().abs() < 1.0e-11);
            }
            // The Greville coefficients of |t - 1| reproduce it: a kink at
            // the repeated knot is in the space.
            let greville = kinked.greville();
            for step in 0..100 {
                let t = 2.0 * (step as f64 + 0.5) / 100.0;
                let span = kinked.span_of(t);
                let (values, _) = kinked.evaluate(span, t);
                let value: f64 = (0..3)
                    .map(|a| values[a] * (greville[kinked.function(span, a)] - 1.0).abs())
                    .sum();
                assert!((value - (t - 1.0).abs()).abs() < 1.0e-12, "{t}: {value}");
            }
        }
    }

    #[test]
    fn a_sampled_material_weights_the_mass_and_the_kick() {
        let patch = small_patch(2, 2).with_material(|p| if p.x < 0.0 { 1.0 } else { 4.0 }, |_| 0.5);
        let total: f64 = patch.lumped().iter().sum();
        // Half the box at density one, half at four: 2.5 times the area 2.
        assert!((total - 5.0).abs() < 1.0e-12, "{total}");
        let ones = vec![1.0; patch.degrees_of_freedom()];
        for (row, lumped) in patch.apply_mass(&ones).iter().zip(patch.lumped()) {
            assert!((row - lumped).abs() < 1.0e-13);
        }
        let b: Vec<Point2> = (0..patch.samples())
            .map(|i| Point2::new(((i * 31) % 17) as f64 / 17.0, 0.3))
            .collect();
        let plain = small_patch(2, 2).divergence(&b);
        for (half, full) in patch.divergence(&b).iter().zip(&plain) {
            assert!((2.0 * half - full).abs() < 1.0e-12);
        }
        let values = patch.sample_values(&ones);
        assert!(values.iter().all(|v| (v - 1.0).abs() < 1.0e-13));
    }

    #[test]
    fn a_moved_control_point_rebuilds_locally_and_carries_the_flux_back_and_forth() {
        let size = Point2::new(2.0, 2.0);
        let patch = SplinePatch::curved(
            2,
            GeometryMap::flat_surface(Point2::new(-1.0, -1.0), size, [8, 8]),
            size,
            [24, 24],
            2,
            true,
        )
        .unwrap();
        let (moved, report) = patch
            .with_moved_control(5, 4, Point2::new(0.07, -0.05))
            .unwrap();
        // Control point 5 of a cubic net on 8 spans touches spans 2..=5:
        // half the width, so a quarter of the samples.
        assert_eq!(report.affected_samples, report.samples / 4);
        assert!(report.affected_dofs < report.degrees_of_freedom / 2);
        let (back, _) = moved
            .with_moved_control(5, 4, Point2::new(-0.07, 0.05))
            .unwrap();
        for (a, b) in patch.sample_points().iter().zip(back.sample_points()) {
            assert!((*a - *b).norm() < 1.0e-12);
        }
        let b: Vec<Point2> = (0..patch.samples())
            .map(|i| Point2::new(((i * 31) % 17) as f64 / 17.0, ((i * 13) % 23) as f64 / 23.0))
            .collect();
        let carried = patch.carry_flux(&moved, &b);
        let returned = moved.carry_flux(&back, &carried);
        for (a, b) in b.iter().zip(&returned) {
            assert!((*a - *b).norm() < 1.0e-12);
        }
        // A sample the move did not touch keeps its flux.
        let far = patch
            .sample_points()
            .iter()
            .position(|p| p.x < -0.6)
            .unwrap();
        assert!((carried[far] - b[far]).norm() < 1.0e-14);
        // The gradient of a field moves as its covariant components say.
        let u = patch.project_parametric(|p| (p.x * 1.3).sin() + p.y * p.y);
        let moved_gradient = moved.gradient(&u);
        let carried_gradient = patch.carry_flux(&moved, &patch.gradient(&u));
        for (a, b) in moved_gradient.iter().zip(&carried_gradient) {
            assert!((*a - *b).norm() < 1.0e-10);
        }
    }

    #[test]
    fn refinement_carries_the_field_exactly_and_the_flux_closely() {
        let size = Point2::new(2.0, 2.0);
        let coarse = SplinePatch::curved(
            2,
            GeometryMap::flat_surface(Point2::new(-1.0, -1.0), size, [6, 6]),
            size,
            [12, 12],
            2,
            true,
        )
        .unwrap();
        let fine = coarse.refined().unwrap();
        assert_eq!(fine.samples(), 4 * coarse.samples());
        let u = coarse.project_parametric(|p| (p.x * 1.7).cos() * (p.y - 0.4));
        let u_fine = coarse.refine_field(&fine, &u);
        for step in 0..40 {
            let parameter = Point2::new(
                2.0 * (step as f64 + 0.3) / 40.0,
                2.0 * ((step * 7 % 40) as f64 + 0.6) / 40.0,
            );
            let a = coarse.evaluate_field(&u, parameter);
            let b = fine.evaluate_field(&u_fine, parameter);
            assert!((a - b).abs() < 1.0e-9, "{a} {b}");
        }
        // A flux that is a gradient is recovered as one and carried exactly.
        let (potential, residual) = coarse.flux_potential(&coarse.gradient(&u));
        assert!(residual < 1.0e-9, "{residual}");
        let mean: f64 = u
            .iter()
            .zip(coarse.lumped())
            .map(|(u, m)| u * m)
            .sum::<f64>()
            / coarse.lumped().iter().sum::<f64>();
        for (w, u) in potential.iter().zip(&u) {
            assert!((w - (u - mean)).abs() < 1.0e-8, "{w} {}", u - mean);
        }
        let b_fine = coarse.refine_flux(&fine, &coarse.gradient(&u));
        for (a, b) in fine.gradient(&u_fine).iter().zip(&b_fine) {
            assert!((*a - *b).norm() < 1.0e-8, "{a:?} {b:?}");
        }
        // A flux that is not a gradient shows in the residual.
        let twisted: Vec<Point2> = coarse
            .sample_points()
            .iter()
            .map(|p| Point2::new(-p.y, p.x))
            .collect();
        let (_, residual) = coarse.flux_potential(&twisted);
        assert!(residual > 0.5, "{residual}");
    }

    #[test]
    fn divergence_is_the_transpose_of_gradient() {
        let patch = small_patch(3, 3);
        let n = patch.degrees_of_freedom();
        let u: Vec<f64> = (0..n)
            .map(|i| ((i * 7919) % 101) as f64 / 101.0 - 0.5)
            .collect();
        let b: Vec<Point2> = (0..patch.samples())
            .map(|i| {
                Point2::new(
                    ((i * 31) % 17) as f64 / 17.0 - 0.5,
                    ((i * 13) % 23) as f64 / 23.0 - 0.5,
                )
            })
            .collect();
        let gu = patch.gradient(&u);
        let left: f64 = gu
            .iter()
            .zip(&b)
            .zip(patch.sample_weights())
            .map(|((g, b), w)| w * g.dot(*b))
            .sum();
        let right = dot(&u, &patch.divergence(&b));
        assert!((left - right).abs() < 1.0e-12 * left.abs().max(1.0));
    }

    #[test]
    fn the_gradient_of_a_projected_line_is_exact() {
        let patch = small_patch(2, 2);
        let u = patch.project_separable(|x| 3.0 * x + 1.0, |_| 1.0);
        for g in patch.gradient(&u) {
            assert!((g.x - 3.0).abs() < 1.0e-10 && g.y.abs() < 1.0e-10, "{g:?}");
        }
        let v = patch.interpolate_separable(|x| 3.0 * x + 1.0, |y| 2.0 - y);
        for (u, v) in patch
            .project_separable(|x| 3.0 * x + 1.0, |y| 2.0 - y)
            .iter()
            .zip(&v)
        {
            assert!((u - v).abs() < 1.0e-10);
        }
    }

    #[test]
    fn the_field_maps_are_symmetric_and_invert_each_other() {
        let patch = small_patch(3, 4);
        let n = patch.degrees_of_freedom();
        for treatment in [
            MassTreatment::Lumped { sweeps: 0 },
            MassTreatment::Lumped { sweeps: 2 },
            MassTreatment::Consistent,
        ] {
            let mut columns = Vec::new();
            for i in [3, 17, n / 2, n - 5] {
                let mut e = vec![0.0; n];
                e[i] = 1.0;
                columns.push((i, patch.field(&e, treatment)));
            }
            // The lumped maps are exact arithmetic; the consistent one is a
            // conjugate-gradient solve, symmetric to its own accuracy.
            for (i, column_i) in &columns {
                for (j, column_j) in &columns {
                    assert!(
                        (column_i[*j] - column_j[*i]).abs() < 1.0e-9,
                        "{treatment:?}"
                    );
                }
            }
            let u: Vec<f64> = (0..n).map(|i| ((i * 37) % 11) as f64 / 11.0).collect();
            let back = patch.field(&patch.flux_of_field(&u, treatment), treatment);
            for (a, b) in u.iter().zip(&back) {
                assert!((a - b).abs() < 1.0e-10, "{treatment:?}");
            }
        }
    }

    #[test]
    fn both_integrators_conserve_their_energies_at_their_stable_steps() {
        for treatment in TREATMENTS {
            for integrator in [Integrator::Leapfrog, Integrator::FourthOrder] {
                let patch = small_patch(3, 4);
                let (lambda, _) = patch.largest_eigenvalue(treatment, 2000);
                let dt = 0.95 * integrator.stability_factor() * 2.0 / lambda.sqrt();
                let u = patch.project_separable(
                    |x| (std::f64::consts::PI * x).cos(),
                    |y| (2.0 * std::f64::consts::PI * y).cos(),
                );
                let mut stepper =
                    PatchStepper::with_integrator(&patch, treatment, integrator, dt, u);
                let staggered = stepper.staggered_energy();
                stepper.step();
                let conserved = stepper.conserved_energy();
                for _ in 0..400 {
                    stepper.step();
                }
                let drift = (stepper.conserved_energy() - conserved).abs() / conserved;
                assert!(drift < 1.0e-10, "{treatment:?} {integrator:?}: {drift}");
                if integrator == Integrator::Leapfrog {
                    let drift = (stepper.staggered_energy() - staggered).abs() / staggered;
                    assert!(drift < 1.0e-10, "{treatment:?}: staggered {drift}");
                }
                assert!(
                    stepper.field().iter().all(|v| v.abs() < 10.0),
                    "{integrator:?}"
                );
            }
        }
    }

    #[test]
    fn the_stepped_frequency_inverts_and_is_fourth_order_under_the_modified_equation() {
        let omega = 7.3;
        for integrator in [Integrator::Leapfrog, Integrator::FourthOrder] {
            for dt in [0.01, 0.05, 0.2] {
                let stepped = integrator.stepped_frequency(omega, dt);
                let back = integrator.spatial_frequency(stepped, dt);
                assert!(
                    (back - omega).abs() < 1.0e-9,
                    "{integrator:?} dt {dt}: {back}"
                );
            }
        }
        let error = |integrator: Integrator, dt: f64| {
            (integrator.stepped_frequency(omega, dt) - omega).abs()
        };
        let leapfrog =
            (error(Integrator::Leapfrog, 0.04) / error(Integrator::Leapfrog, 0.02)).log2();
        let fourth =
            (error(Integrator::FourthOrder, 0.04) / error(Integrator::FourthOrder, 0.02)).log2();
        assert!((leapfrog - 2.0).abs() < 0.05, "leapfrog order {leapfrog}");
        assert!((fourth - 4.0).abs() < 0.05, "fourth order {fourth}");
        // At leapfrog's stable step the fourth-order scheme's temporal error
        // is below leapfrog's by (ω dt)²/30 and change.
        let dt = 1.6 / omega;
        assert!(error(Integrator::FourthOrder, dt) < 0.1 * error(Integrator::Leapfrog, dt));
    }

    /// On a linear basis the nodal cosine is an exact Neumann eigenvector of
    /// every treatment, with the periodic symbol's eigenvalue, so the
    /// stepped phase must be the integrator's frequency to roundoff.
    #[test]
    fn a_linear_box_mode_drifts_exactly_as_the_symbol_predicts() {
        let patch = SplinePatch::new(
            1,
            Point2::new(-1.0, -1.0),
            Point2::new(2.0, 2.0),
            [20, 4],
            2,
        )
        .unwrap();
        let k = 2.0 * std::f64::consts::PI / 0.8;
        let theta = k * patch.basis_x().spacing();
        let symbols = PeriodicSymbols::new(1, patch.basis_x().spacing(), &gauss_rule(2).unwrap());
        for (treatment, integrator) in TREATMENTS.into_iter().flat_map(|treatment| {
            [Integrator::Leapfrog, Integrator::FourthOrder]
                .into_iter()
                .map(move |integrator| (treatment, integrator))
        }) {
            let omega = symbols
                .frequency_squared(theta, 0.0, treatment.sweeps())
                .sqrt();
            let dt = 0.02;
            let predicted = integrator.stepped_frequency(omega, dt);
            let mode = patch.interpolate_separable(|x| (k * (x + 1.0)).cos(), |_| 1.0);
            let mut stepper =
                PatchStepper::with_integrator(&patch, treatment, integrator, dt, mode.clone());
            for _ in 0..150 {
                stepper.step();
            }
            let measured = measure_mode(
                &patch,
                &mode,
                stepper.field(),
                stepper.previous_field(),
                ModeClock {
                    omega: predicted,
                    readout: predicted,
                    dt,
                    time: stepper.time(),
                },
            );
            assert!(
                measured.phase_error.abs() < 1.0e-9,
                "{treatment:?} {integrator:?}: {measured:?}"
            );
            assert!(
                measured.amplitude_error.abs() < 1.0e-9,
                "{treatment:?} {integrator:?}"
            );
            // Against the exact frequency the drift is what dispersion says.
            let against_exact = measure_mode(
                &patch,
                &mode,
                stepper.field(),
                stepper.previous_field(),
                ModeClock {
                    omega: k,
                    readout: predicted,
                    dt,
                    time: stepper.time(),
                },
            );
            let expected = wrap_angle((predicted - k) * stepper.time());
            assert!(
                (against_exact.phase_error - expected).abs() < 1.0e-9,
                "{treatment:?}: {} against {expected}",
                against_exact.phase_error
            );
            // Reading out with the exact angle instead, as the triangle
            // example does, offsets the phase by about the relative
            // frequency error.
            let exact_readout = measure_mode(
                &patch,
                &mode,
                stepper.field(),
                stepper.previous_field(),
                ModeClock {
                    omega: k,
                    readout: k,
                    dt,
                    time: stepper.time(),
                },
            );
            let offset = (exact_readout.phase_error - expected).abs();
            let relative = ((predicted - k) / k).abs();
            assert!(offset < 2.0 * relative, "{treatment:?}: offset {offset}");
        }
    }
}
