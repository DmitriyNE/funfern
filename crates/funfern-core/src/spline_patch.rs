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

use crate::Point2;

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

    pub fn functions(&self) -> usize {
        if self.periodic {
            self.spans
        } else {
            self.spans + self.degree
        }
    }

    /// The global index of local function `local` on span `span`.
    pub fn function(&self, span: usize, local: usize) -> usize {
        (span + local) % self.functions()
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
        let k = span + p;
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

/// The frequency leapfrog with step `dt` advances a mode of discrete
/// frequency `omega` at.
pub fn leapfrog_frequency(omega: f64, dt: f64) -> f64 {
    (2.0 / dt) * (0.5 * omega * dt).min(1.0).asin()
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

/// An affine tensor-product patch over a box, with the quadrature samples
/// the `(Q, b)` scheme keeps `b` at.
#[derive(Clone, Debug)]
pub struct BoxPatch {
    basis_x: UniformBasis,
    basis_y: UniformBasis,
    rule: Vec<(f64, f64)>,
    origin: Point2,
    /// Per `(span, point)` in each direction: values and derivatives.
    table_x: Vec<([f64; MAX_LOCAL], [f64; MAX_LOCAL])>,
    table_y: Vec<([f64; MAX_LOCAL], [f64; MAX_LOCAL])>,
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

impl BoxPatch {
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
        Self::from_bases(basis_x, basis_y, origin, points)
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
        Self::from_bases(basis_x, basis_y, origin, points)
    }

    fn from_bases(
        basis_x: UniformBasis,
        basis_y: UniformBasis,
        origin: Point2,
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
            origin,
            table_x,
            table_y,
            lumped: Vec::new(),
            weights: Vec::new(),
        };
        let mut lumped = vec![0.0; patch.degrees_of_freedom()];
        let mut weights = Vec::with_capacity(patch.samples());
        patch.for_each_sample(|_, stencil| {
            weights.push(stencil.weight);
            for (node, value) in stencil.nodes().iter().zip(stencil.values()) {
                lumped[*node] += stencil.weight * value;
            }
        });
        patch.lumped = lumped;
        patch.weights = weights;
        Some(patch)
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
        self.basis_x.functions() * self.basis_y.functions()
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

    pub fn node(&self, ix: usize, iy: usize) -> usize {
        ix * self.basis_y.functions() + iy
    }

    /// Visits every sample in index order with its stencil.
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
                        stencil.weight = self.rule[gx].1 * self.rule[gy].1 * area;
                        let mut local = 0;
                        for a in 0..=p {
                            let ix = self.basis_x.function(sx, a);
                            for b in 0..=p {
                                let iy = self.basis_y.function(sy, b);
                                stencil.nodes[local] = self.node(ix, iy);
                                stencil.values[local] = nx[a] * ny[b];
                                stencil.gradients[local] =
                                    Point2::new(dnx[a] * ny[b], nx[a] * dny[b]);
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

    /// `M u` with unit density, through the samples.
    pub fn apply_mass(&self, u: &[f64]) -> Vec<f64> {
        let mut out = vec![0.0; u.len()];
        self.for_each_sample(|_, stencil| {
            let value = stencil
                .nodes()
                .iter()
                .zip(stencil.values())
                .map(|(node, value)| value * u[*node])
                .sum::<f64>()
                * stencil.weight;
            for (node, basis) in stencil.nodes().iter().zip(stencil.values()) {
                out[*node] += value * basis;
            }
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

    /// `Gᵀ W b`: the weighted gather of sample fluxes to the coefficients,
    /// the exact transpose of [`Self::gradient`].
    pub fn divergence(&self, b: &[Point2]) -> Vec<f64> {
        let mut out = vec![0.0; self.degrees_of_freedom()];
        self.for_each_sample(|index, stencil| {
            let flux = b[index] * stencil.weight;
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
    /// `f(x) g(y)`, one one-dimensional projection per direction.
    pub fn project_separable(&self, f: impl Fn(f64) -> f64, g: impl Fn(f64) -> f64) -> Vec<f64> {
        let cx = project_1d(&self.basis_x, |t| f(self.origin.x + t));
        let cy = project_1d(&self.basis_y, |t| g(self.origin.y + t));
        self.separable(&cx, &cy)
    }

    /// The coefficients that read `f(x) g(y)` at the Greville points: nodal
    /// interpolation for degree one, a quasi-interpolant above.
    pub fn interpolate_separable(
        &self,
        f: impl Fn(f64) -> f64,
        g: impl Fn(f64) -> f64,
    ) -> Vec<f64> {
        let cx: Vec<f64> = self
            .basis_x
            .greville()
            .into_iter()
            .map(|t| f(self.origin.x + t))
            .collect();
        let cy: Vec<f64> = self
            .basis_y
            .greville()
            .into_iter()
            .map(|t| g(self.origin.y + t))
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
    patch: &BoxPatch,
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

/// Leapfrog on a patch with unit material and natural reflecting walls:
/// `Q̇ = -Gᵀ W b`, `ḃ = G u`, `u = P Q`, with `b` half a step ahead of `Q`.
pub struct PatchStepper<'a> {
    patch: &'a BoxPatch,
    treatment: MassTreatment,
    dt: f64,
    q: Vec<f64>,
    b: Vec<Point2>,
    u: Vec<f64>,
    u_previous: Vec<f64>,
    steps: u64,
}

impl<'a> PatchStepper<'a> {
    /// Starts from the field `u` at rest.
    pub fn new(patch: &'a BoxPatch, treatment: MassTreatment, dt: f64, u: Vec<f64>) -> Self {
        let q = patch.flux_of_field(&u, treatment);
        let u = patch.field(&q, treatment);
        let gradient = patch.gradient(&u);
        let b = gradient.into_iter().map(|g| g * (0.5 * dt)).collect();
        Self {
            patch,
            treatment,
            dt,
            q,
            u_previous: u.clone(),
            u,
            b,
            steps: 0,
        }
    }

    pub fn step(&mut self) {
        let force = self.patch.divergence(&self.b);
        for (q, f) in self.q.iter_mut().zip(&force) {
            *q -= self.dt * f;
        }
        std::mem::swap(&mut self.u, &mut self.u_previous);
        self.u = self.patch.field(&self.q, self.treatment);
        let gradient = self.patch.gradient(&self.u);
        for (b, g) in self.b.iter_mut().zip(&gradient) {
            *b = *b + *g * self.dt;
        }
        self.steps += 1;
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

    /// The staggered energy `½ Qᵀ P Q + ½ b⁻ · W b⁺`, which leapfrog
    /// conserves exactly: `b⁻` is the flux half a step before the current
    /// `Q` and `b⁺` the one half a step after.
    pub fn staggered_energy(&self) -> f64 {
        let kinetic = 0.5 * dot(&self.q, &self.u);
        // `b` holds `b⁺`; `b⁻ = b⁺ - dt G u`.
        let gradient = self.patch.gradient(&self.u);
        let potential = self
            .b
            .iter()
            .zip(&gradient)
            .zip(self.patch.sample_weights())
            .map(|((plus, g), weight)| {
                let minus = *plus - *g * self.dt;
                0.5 * weight * plus.dot(minus)
            })
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

    fn small_patch(degree: usize, points: usize) -> BoxPatch {
        BoxPatch::new(
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
            let unclamped = BoxPatch::unclamped(
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
    fn leapfrog_conserves_the_staggered_energy() {
        for treatment in TREATMENTS {
            let patch = small_patch(3, 4);
            let (lambda, _) = patch.largest_eigenvalue(treatment, 2000);
            let dt = 0.8 * 2.0 / lambda.sqrt();
            let u = patch.project_separable(
                |x| (std::f64::consts::PI * x).cos(),
                |y| (2.0 * std::f64::consts::PI * y).cos(),
            );
            let mut stepper = PatchStepper::new(&patch, treatment, dt, u);
            let initial = stepper.staggered_energy();
            for _ in 0..200 {
                stepper.step();
            }
            let drift = (stepper.staggered_energy() - initial).abs() / initial;
            assert!(drift < 1.0e-11, "{treatment:?}: {drift}");
        }
    }

    /// On a linear basis the nodal cosine is an exact Neumann eigenvector of
    /// every treatment, with the periodic symbol's eigenvalue, so the
    /// stepped phase must be the leapfrog frequency's to roundoff.
    #[test]
    fn a_linear_box_mode_drifts_exactly_as_the_symbol_predicts() {
        let patch = BoxPatch::new(
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
        for treatment in TREATMENTS {
            let omega = symbols
                .frequency_squared(theta, 0.0, treatment.sweeps())
                .sqrt();
            let dt = 0.02;
            let predicted = leapfrog_frequency(omega, dt);
            let mode = patch.interpolate_separable(|x| (k * (x + 1.0)).cos(), |_| 1.0);
            let mut stepper = PatchStepper::new(&patch, treatment, dt, mode.clone());
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
                "{treatment:?}: {measured:?}"
            );
            assert!(measured.amplitude_error.abs() < 1.0e-9, "{treatment:?}");
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
