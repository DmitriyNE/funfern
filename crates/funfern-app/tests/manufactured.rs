//! PH-19: manufactured solutions. A field `u*` chosen in closed form, the
//! forcing its equations need derived by hand, and the production path
//! (document, meshing, assembly, the canonical compile and the step) run
//! under that forcing from `u*`'s own state: the error against `u*` must
//! fall with the mesh as the element predicts.
//!
//! In the canonical form the state is the lumped nodal flux `Q = M u` and
//! the complementary flux `b` at the elements' samples, `Q̇ = −F(b) + S`
//! and `ḃ = η C u`. So `ψ`, the field's time integral with `b = η C ψ`,
//! solves `M ψ̈ + K ψ = S`: the source drives `ψ`'s equation, whose
//! manufactured right-hand side `s* = m ψ*_tt − ∇·(A ∇ψ*)` is written here
//! from the physics and lumped as the mass is.

mod support;
use support::*;

use funfern_app::topology_editor::{ClosedCurvePurpose, TopologyDocument, TopologyEditor};
use funfern_app::topology_runtime::PreparedTopology;
use funfern_core::{
    BACKGROUND_REGION, CanonicalForcing, CanonicalIntegrator, CanonicalSource,
    CanonicalTemporalWaveState, CanonicalWaveOperator, CanonicalWaveState, CoefficientLaw,
    DampingLaw, DomainRect, ElectromagneticPolarization, FieldLaw, LossChannel, MaterialFrame,
    MaterialFrameAttachment, MaterialParameter, OuterBoundaryCondition, OuterBoundaryConditions,
    PeriodicCubicSpline, PhysicsModel, Point2, RateLaw, ScalarField, SymmetricTensor2, TimeDrive,
    TimeSignal, VolumeSource,
};
use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// The element's lumping weights: a vertex, an edge's midpoint, the bubble.
const MASS_WEIGHTS: [f64; 7] = [
    1.0 / 20.0,
    1.0 / 20.0,
    1.0 / 20.0,
    2.0 / 15.0,
    2.0 / 15.0,
    2.0 / 15.0,
    9.0 / 20.0,
];

/// A smooth function of the world plane, its gradient and its Hessian
/// `[xx, xy, yy]`, each written out by hand.
#[derive(Clone, Copy)]
struct Smooth {
    value: fn(Point2) -> f64,
    gradient: fn(Point2) -> Point2,
    hessian: fn(Point2) -> [f64; 3],
}

/// A material slot's field, a formula of the material frame's coordinates
/// `(x, y)`, its value and gradient there written out by hand.
#[derive(Clone, Copy)]
struct FrameField {
    formula: &'static str,
    value: fn(f64, f64) -> f64,
    gradient: fn(f64, f64) -> (f64, f64),
}

impl FrameField {
    const fn constant(formula: &'static str, value: fn(f64, f64) -> f64) -> Self {
        Self {
            formula,
            value,
            gradient: |_, _| (0.0, 0.0),
        }
    }
}

/// The box the cases run in, `[−1, 1] × [−0.7, 0.7]`.
const BOX: DomainRect = DomainRect {
    min_x: -1.0,
    max_x: 1.0,
    min_y: -0.7,
    max_y: 0.7,
};
const KX: f64 = PI / 2.0;
const KY: f64 = PI / 1.4;

/// The box's mode `(1, 1)` of a reflecting wall, `cos(π(x + 1)/2)
/// cos(π(y + 0.7)/1.4)`, whose normal derivative vanishes on every side.
const COSINE_SHAPE: Smooth = Smooth {
    value: |p| (KX * (p.x + 1.0)).cos() * (KY * (p.y + 0.7)).cos(),
    gradient: |p| {
        let (sx, cx) = (KX * (p.x + 1.0)).sin_cos();
        let (sy, cy) = (KY * (p.y + 0.7)).sin_cos();
        Point2::new(-KX * sx * cy, -KY * cx * sy)
    },
    hessian: |p| {
        let (sx, cx) = (KX * (p.x + 1.0)).sin_cos();
        let (sy, cy) = (KY * (p.y + 0.7)).sin_cos();
        [-KX * KX * cx * cy, KX * KY * sx * sy, -KY * KY * cx * cy]
    },
};

/// The box's mode `(1, 1)` of a pinned wall, `sin(π(x + 1)/2)
/// sin(π(y + 0.7)/1.4)`, which vanishes on every side.
const SINE_SHAPE: Smooth = Smooth {
    value: |p| (KX * (p.x + 1.0)).sin() * (KY * (p.y + 0.7)).sin(),
    gradient: |p| {
        let (sx, cx) = (KX * (p.x + 1.0)).sin_cos();
        let (sy, cy) = (KY * (p.y + 0.7)).sin_cos();
        Point2::new(KX * cx * sy, KY * sx * cy)
    },
    hessian: |p| {
        let (sx, cx) = (KX * (p.x + 1.0)).sin_cos();
        let (sy, cy) = (KY * (p.y + 0.7)).sin_cos();
        [-KX * KX * sx * sy, KX * KY * cx * cy, -KY * KY * sx * sy]
    },
};

/// A shape with no law on any side, `cos(1.3x + 0.4) cos(1.7y − 0.3)`.
const GENERIC_SHAPE: Smooth = Smooth {
    value: |p| (1.3 * p.x + 0.4).cos() * (1.7 * p.y - 0.3).cos(),
    gradient: |p| {
        let (sx, cx) = (1.3 * p.x + 0.4).sin_cos();
        let (sy, cy) = (1.7 * p.y - 0.3).sin_cos();
        Point2::new(-1.3 * sx * cy, -1.7 * cx * sy)
    },
    hessian: |p| {
        let (sx, cx) = (1.3 * p.x + 0.4).sin_cos();
        let (sy, cy) = (1.7 * p.y - 0.3).sin_cos();
        [-1.69 * cx * cy, 2.21 * sx * sy, -2.89 * cx * cy]
    },
};

/// What a case does at one of the box's sides.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Side {
    /// A reflecting wall, whose law the shape keeps: no flux out.
    Natural,
    /// A Dirichlet wall, prescribed `u*` node by node.
    Pinned,
    /// A reflecting wall loaded with `u*`'s outward flux `∫ (A ∇ψ*·n) N_i`:
    /// a Neumann side whose data vary along it.
    Flux,
    /// A first-order outgoing wall, `A ∇u·n = −√(m a) u_t`, loaded with what
    /// `u*` leaves of its law, `h = A ∇ψ*·n + √(m a) u*`.
    Outgoing,
}

impl Side {
    fn wall(self) -> OuterBoundaryCondition {
        match self {
            Self::Natural | Self::Flux => OuterBoundaryCondition::Reflecting,
            Self::Pinned => OuterBoundaryCondition::Dirichlet {
                signal: TimeSignal::harmonic(0.0, 0.0, 1.0, 0.0),
            },
            Self::Outgoing => OuterBoundaryCondition::FirstOrderOutgoing,
        }
    }
}

/// Whether a point lies on a side.
type OnSide = fn(Point2) -> bool;

/// The box's sides in `OuterSide`'s order, each with its outward normal and
/// whether a point lies on it.
const SIDES: [(Point2, OnSide); 4] = [
    (Point2::new(0.0, -1.0), |p| (p.y - BOX.min_y).abs() < 1.0e-9),
    (Point2::new(1.0, 0.0), |p| (p.x - BOX.max_x).abs() < 1.0e-9),
    (Point2::new(0.0, 1.0), |p| (p.y - BOX.max_y).abs() < 1.0e-9),
    (Point2::new(-1.0, 0.0), |p| (p.x - BOX.min_x).abs() < 1.0e-9),
];

/// A medium: its frame, its density and stiffness slots and its axis ratio,
/// the ratio of the wave speeds along the frame's x and y axes.
#[derive(Clone, Copy)]
struct Medium {
    frame: MaterialFrame,
    density: FrameField,
    stiffness: FrameField,
    axis_ratio: FrameField,
    /// The electric and magnetic flux-rate losses, `Ḋ = … − γ_e D` and
    /// `Ḃ = … − γ_m B`.
    electric_loss: Option<FrameField>,
    magnetic_loss: Option<FrameField>,
}

/// Case 1's medium: isotropic, in the world frame, density
/// `1 + 0.3 sin(x) cos(2y)` and stiffness `1 + 0.25 cos(x + y)`.
const HETEROGENEOUS: Medium = Medium {
    frame: MaterialFrame::world(),
    density: FrameField {
        formula: "1 + 0.3 * sin(x) * cos(2 * y)",
        value: |x, y| 1.0 + 0.3 * x.sin() * (2.0 * y).cos(),
        gradient: |x, y| {
            (
                0.3 * x.cos() * (2.0 * y).cos(),
                -0.6 * x.sin() * (2.0 * y).sin(),
            )
        },
    },
    stiffness: FrameField {
        formula: "1 + 0.25 * cos(x + y)",
        value: |x, y| 1.0 + 0.25 * (x + y).cos(),
        gradient: |x, y| {
            let s = -0.25 * (x + y).sin();
            (s, s)
        },
    },
    axis_ratio: FrameField::constant("1", |_, _| 1.0),
    electric_loss: None,
    magnetic_loss: None,
};

/// Case 4's medium: case 1's with an electric loss `0.3 + 0.2 sin(x + y/2)`
/// and a magnetic loss `0.25 + 0.15 cos(2x − y)`.
const LOSSY: Medium = Medium {
    electric_loss: Some(FrameField {
        formula: "0.3 + 0.2 * sin(x + 0.5 * y)",
        value: |x, y| 0.3 + 0.2 * (x + 0.5 * y).sin(),
        gradient: |x, y| {
            let c = 0.2 * (x + 0.5 * y).cos();
            (c, 0.5 * c)
        },
    }),
    magnetic_loss: Some(FrameField {
        formula: "0.25 + 0.15 * cos(2 * x - y)",
        value: |x, y| 0.25 + 0.15 * (2.0 * x - y).cos(),
        gradient: |x, y| {
            let s = -0.15 * (2.0 * x - y).sin();
            (2.0 * s, -s)
        },
    }),
    ..HETEROGENEOUS
};

/// Case 2's medium: a frame turned 0.5 rad about `(0.1, −0.05)`, density
/// `1 + 0.2 cos(x − y)`, stiffness `1 + 0.2 x y` and axis ratio
/// `1.5 + 0.3 x²` in its coordinates, so the tensor's off-diagonal term
/// varies over the box.
const ANISOTROPIC: Medium = Medium {
    frame: MaterialFrame {
        origin: Point2::new(0.1, -0.05),
        angle_radians: 0.5,
        attachment: MaterialFrameAttachment::World,
    },
    density: FrameField {
        formula: "1 + 0.2 * cos(x - y)",
        value: |x, y| 1.0 + 0.2 * (x - y).cos(),
        gradient: |x, y| {
            let s = -0.2 * (x - y).sin();
            (s, -s)
        },
    },
    stiffness: FrameField {
        formula: "1 + 0.2 * x * y",
        value: |x, y| 1.0 + 0.2 * x * y,
        gradient: |x, y| (0.2 * y, 0.2 * x),
    },
    axis_ratio: FrameField {
        formula: "1.5 + 0.3 * x^2",
        value: |x, _| 1.5 + 0.3 * x * x,
        gradient: |x, _| (0.6 * x, 0.0),
    },
    electric_loss: None,
    magnetic_loss: None,
};

impl Medium {
    /// `field` and its world gradient at `p`.
    fn at(&self, field: FrameField, p: Point2) -> (f64, Point2) {
        let c = self.frame.coordinates(p);
        let (gx, gy) = (field.gradient)(c.x, c.y);
        let (sin, cos) = self.frame.angle_radians.sin_cos();
        (
            (field.value)(c.x, c.y),
            Point2::new(gx * cos - gy * sin, gx * sin + gy * cos),
        )
    }
}

/// What a skin makes of the material's slots in the scalar equation
/// `m u_tt − ∇·(A ∇u) = f`, from each model's own law, and the orientation
/// `η` of `ḃ = η R∇u`. Mechanical is `ρ u_tt = ∇·(k ∇u)`, density and
/// stiffness as written; TM is `ε ∂_t E_z = ∇×H`, `μ ∂_t H = −∇×E`, so
/// `m = ε`, the density slot, and `A` scales as `1/μ`, the stiffness slot's
/// reciprocal; TE is `μ ∂_t H_z = −∇×E`, `ε ∂_t E = ∇×H`, so `m = μ`, the
/// stiffness slot, and `A` scales as `1/ε`, the density slot's reciprocal.
/// In each, the axis ratio `r` makes `A` that scale times `r` along the
/// frame's x axis and over `r` along its y.
#[derive(Clone, Copy, Debug)]
struct Skin(PhysicsModel);

impl Skin {
    fn te(self) -> bool {
        matches!(
            self.0,
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            }
        )
    }

    fn mass(self, medium: &Medium, p: Point2) -> f64 {
        let slot = if self.te() {
            medium.stiffness
        } else {
            medium.density
        };
        medium.at(slot, p).0
    }

    /// `A`'s scale and its gradient at `p`.
    fn scale(self, medium: &Medium, p: Point2) -> (f64, Point2) {
        let reciprocal =
            |(value, gradient): (f64, Point2)| (1.0 / value, gradient * (-1.0 / (value * value)));
        match self.0 {
            PhysicsModel::Mechanical => medium.at(medium.stiffness, p),
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            } => reciprocal(medium.at(medium.stiffness, p)),
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Te,
            } => reciprocal(medium.at(medium.density, p)),
        }
    }

    /// `A` at `p` in world axes, and its divergence `(∂_x A_xx + ∂_y A_xy,
    /// ∂_x A_xy + ∂_y A_yy)`.
    fn tensor(self, medium: &Medium, p: Point2) -> (SymmetricTensor2, Point2) {
        let (g, grad_g) = self.scale(medium, p);
        let (r, grad_r) = medium.at(medium.axis_ratio, p);
        let (s, c) = medium.frame.angle_radians.sin_cos();
        // `T(r)`, `r` along the frame's x and `1/r` along its y, and `T′(r)`.
        let t = [
            r * c * c + s * s / r,
            (r - 1.0 / r) * s * c,
            r * s * s + c * c / r,
        ];
        let dt = [
            c * c - s * s / (r * r),
            (1.0 + 1.0 / (r * r)) * s * c,
            s * s - c * c / (r * r),
        ];
        let tensor = SymmetricTensor2::new(g * t[0], g * t[1], g * t[2]);
        // `∂_l A_ij = ∂_l g T_ij + g T′_ij ∂_l r`.
        let d =
            |ij: usize, grad: fn(Point2) -> f64| grad(grad_g) * t[ij] + g * dt[ij] * grad(grad_r);
        let x = |v: Point2| v.x;
        let y = |v: Point2| v.y;
        let divergence = Point2::new(d(0, x) + d(1, y), d(1, x) + d(2, y));
        (tensor, divergence)
    }

    fn orientation(self) -> f64 {
        match self.0 {
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            } => 1.0,
            _ => -1.0,
        }
    }

    /// The primary and the complementary row's loss rates and gradients at
    /// `p`: in TM the primary field is `E_z` and the complementary `H⊥`, so
    /// the electric loss is the primary row's; in TE the primary is `H_z`,
    /// and Mechanical is TE's.
    fn losses(self, medium: &Medium, p: Point2) -> [(f64, Point2); 2] {
        let rate = |field: Option<FrameField>| {
            field.map_or((0.0, Point2::default()), |field| medium.at(field, p))
        };
        let (electric, magnetic) = (rate(medium.electric_loss), rate(medium.magnetic_loss));
        match self.0 {
            PhysicsModel::Electromagnetic {
                polarization: ElectromagneticPolarization::Tm,
            } => [electric, magnetic],
            _ => [magnetic, electric],
        }
    }
}

/// A harmonic manufactured field `u* = φ sin(ωt + θ)` in `skin` and
/// `medium`: its time integral `ψ* = −φ cos(ωt + θ)/ω`, and the right-hand
/// side of `ψ`'s equation, `s* = m ψ*_tt − ∇·(A ∇ψ*) = (ω m φ +
/// ∇·(A ∇φ)/ω) cos(ωt + θ)`.
#[derive(Clone, Copy)]
struct Harmonic {
    skin: Skin,
    medium: Medium,
    shape: Smooth,
    omega: f64,
    phase: f64,
    /// The box's sides in `OuterSide`'s order.
    sides: [Side; 4],
    /// A parametric pump on the primary row's coefficient, the mass:
    /// `m (1 + d cos(Ω t + φ))`.
    pump: Option<Pump>,
    /// A travelling modulation of the scalar stiffness, `A (1 + d cos(Ω t +
    /// φ − q·x))`: the complementary row's coefficient divided by it.
    travelling: Option<Travelling>,
    /// A Kerr law on the primary row, its flux `m (1 + χ u²) u`, `χ` a field
    /// of the world frame.
    kerr: Option<FrameField>,
    /// A reflecting hole, `rounded`'s spline about a centre at a radius,
    /// loaded with `u*`'s outward flux along the mesh's own boundary.
    hole: Option<(Point2, f64)>,
    /// The source authored as the app's own volume source, its profile a
    /// formula of the acceleration `S/m` and its harmonic integrated by the
    /// app from zero, rather than built here: only for case 1's medium in
    /// Mechanical at `θ = π/2`, where `S(0) = 0`.
    authored: Option<&'static str>,
}

#[derive(Clone, Copy)]
struct Pump {
    depth: f64,
    frequency_hz: f64,
    phase: f64,
}

#[derive(Clone, Copy)]
struct Travelling {
    depth: f64,
    frequency_hz: f64,
    phase: f64,
    wavenumber: f64,
    angle: f64,
}

impl Travelling {
    /// `q` in world axes, the frame the cases modulate in being the world's.
    fn vector(self) -> Point2 {
        Point2::new(self.angle.cos(), self.angle.sin()) * self.wavenumber
    }
}

/// `cos(ν t + χ)`.
#[derive(Clone, Copy, Debug)]
struct Wave {
    nu: f64,
    chi: f64,
}

impl Wave {
    fn sine(nu: f64, chi: f64) -> Self {
        Self {
            nu,
            chi: chi - FRAC_PI_2,
        }
    }

    /// `cos a cos b = ½ cos(a + b) + ½ cos(a − b)`.
    fn times(self, other: Self) -> [Self; 2] {
        [
            Self {
                nu: self.nu + other.nu,
                chi: self.chi + other.chi,
            },
            Self {
                nu: self.nu - other.nu,
                chi: self.chi - other.chi,
            },
        ]
    }

    fn signal(self) -> TimeSignal {
        // `cos(−ν t + χ) = cos(ν t − χ)`, and a harmonic signal is a sine.
        let (nu, chi) = if self.nu < 0.0 {
            (-self.nu, -self.chi)
        } else {
            (self.nu, self.chi)
        };
        TimeSignal::harmonic(0.0, 1.0, nu / TAU, chi + FRAC_PI_2)
    }
}

/// A term of a source rate: a weight over space times a product of waves.
struct Term<'a> {
    weight: Box<dyn Fn(Point2) -> f64 + 'a>,
    waves: Vec<Wave>,
}

impl Term<'_> {
    /// The term as waves of time alone, each with its share of the weight.
    fn expanded(&self) -> Vec<(f64, Wave)> {
        let mut product = vec![(1.0, self.waves[0])];
        for wave in &self.waves[1..] {
            product = product
                .into_iter()
                .flat_map(|(share, factor)| factor.times(*wave).map(|w| (0.5 * share, w)))
                .collect();
        }
        product
    }
}

impl Harmonic {
    fn angle(&self, t: f64) -> f64 {
        self.omega * t + self.phase
    }

    fn field(&self, p: Point2, t: f64) -> f64 {
        (self.shape.value)(p) * self.angle(t).sin()
    }

    fn mass(&self, p: Point2) -> f64 {
        self.skin.mass(&self.medium, p)
    }

    /// `b* = η R∇φ T`, the periodic solution of `ḃ + γ_c b = η R∇u*` at
    /// each point: `T = α sin(ωt + θ) + β cos(ωt + θ)` with
    /// `α = γ_c/(γ_c² + ω²)`, `β = −ω/(γ_c² + ω²)`. Without loss
    /// `b* = η R∇ψ*`, `ψ* = −φ cos(ωt + θ)/ω`. Answers `(α, β)` and their
    /// gradients.
    fn flux_factor(&self, p: Point2) -> ((f64, f64), (Point2, Point2)) {
        let [_, (gamma, grad_gamma)] = self.skin.losses(&self.medium, p);
        let w = self.omega;
        let d = gamma * gamma + w * w;
        let (alpha, beta) = (gamma / d, -w / d);
        let (d_alpha, d_beta) = ((w * w - gamma * gamma) / (d * d), 2.0 * gamma * w / (d * d));
        ((alpha, beta), (grad_gamma * d_alpha, grad_gamma * d_beta))
    }

    fn rotated(&self, v: Point2) -> Point2 {
        Point2::new(-v.y, v.x) * self.skin.orientation()
    }

    /// Whether `b*` is a gradient, `η R∇` of a potential: so unless the
    /// complementary row's loss varies in space, which gives `T` a gradient
    /// and `b*` a curl.
    fn potential(&self, points: &[Point2]) -> bool {
        points
            .iter()
            .all(|p| self.skin.losses(&self.medium, *p)[1].1 == Point2::default())
    }

    /// `b*` at `p` and `t`.
    fn complementary(&self, p: Point2, t: f64) -> Point2 {
        let ((alpha, beta), _) = self.flux_factor(p);
        let (sin, cos) = self.angle(t).sin_cos();
        self.rotated((self.shape.gradient)(p)) * (alpha * sin + beta * cos)
    }

    /// `b*`'s amplitude at `p`.
    fn complementary_amplitude(&self, p: Point2) -> f64 {
        let ((alpha, beta), _) = self.flux_factor(p);
        (self.shape.gradient)(p).norm() * alpha.hypot(beta)
    }

    /// `∇·(A ∇φ) = A : ∇∇φ + (∇·A)·∇φ`.
    fn divergence(&self, p: Point2) -> f64 {
        let (a, div_a) = self.skin.tensor(&self.medium, p);
        let [xx, xy, yy] = (self.shape.hessian)(p);
        a.xx * xx + 2.0 * a.xy * xy + a.yy * yy + div_a.dot((self.shape.gradient)(p))
    }

    /// The primary row's coefficient `p(x, t)`, the mass under the pump.
    fn mass_at(&self, p: Point2, t: f64) -> f64 {
        let pump = self.pump.map_or(1.0, |pump| {
            1.0 + pump.depth * (TAU * pump.frequency_hz * t + pump.phase).cos()
        });
        self.mass(p) * pump
    }

    /// The primary flux density `P(x, u, t) = p(x, t) (1 + χ u²) u`.
    fn primary_flux(&self, p: Point2, u: f64, t: f64) -> f64 {
        self.mass_at(p, t) * (1.0 + self.chi(p) * u * u) * u
    }

    /// The Kerr law's `χ` at `p`, none without one.
    fn chi(&self, p: Point2) -> f64 {
        self.kerr.map_or(0.0, |chi| (chi.value)(p.x, p.y))
    }

    /// The scalar stiffness `A(x, t)` under the travelling modulation.
    fn tensor_at(&self, p: Point2, t: f64) -> SymmetricTensor2 {
        let a = self.skin.tensor(&self.medium, p).0;
        let g = self.travelling.map_or(1.0, |wave| {
            1.0 + wave.depth
                * (TAU * wave.frequency_hz * t + wave.phase - wave.vector().dot(p)).cos()
        });
        SymmetricTensor2::new(a.xx * g, a.xy * g, a.yy * g)
    }

    /// The source rate `S = ∂_t(p u*) + γ_p p u* − η ∇·(Rᵀ J b*)` as terms,
    /// with `η Rᵀ J b* = A ∇φ T`, `∇·(A ∇φ T) = T ∇·(A ∇φ) + A ∇φ·∇T`.
    /// Under a pump `∂_t(p u*) = m φ (ω (1 + d cos P) cos W − d Ω sin P
    /// sin W)`, `W = ωt + θ`, `P = Ωt + φ`; under a travelling modulation
    /// `−∇·(A g ∇ψ*) = (cos W/ω) (g ∇·(A ∇φ) + A ∇φ·∇g)` with `g = 1 +
    /// d cos Z`, `Z = Z₀ − q·x`, `Z₀ = Ωt + φ`, `∇g = d sin Z q`, and
    /// `cos Z = cos(q·x) cos Z₀ + sin(q·x) sin Z₀`, `sin Z = cos(q·x) sin Z₀
    /// − sin(q·x) cos Z₀`.
    fn terms(&self) -> Vec<Term<'_>> {
        let w = self.omega;
        let cos_w = Wave {
            nu: w,
            chi: self.phase,
        };
        let sin_w = Wave::sine(w, self.phase);
        let m = move |p: Point2| self.mass(p);
        let phi = move |p: Point2| (self.shape.value)(p);
        let div = move |p: Point2| self.divergence(p);
        fn term<'a>(weight: Box<dyn Fn(Point2) -> f64 + 'a>, waves: Vec<Wave>) -> Term<'a> {
            Term { weight, waves }
        }
        let mut kerr = Vec::new();
        if self.kerr.is_some() {
            // `∂_t(m χ u*³) = 3 m χ φ³ ω sin²W cos W`, `χ` pointwise.
            kerr.push(term(
                Box::new(move |p| 3.0 * self.chi(p) * w * m(p) * phi(p).powi(3)),
                vec![sin_w, sin_w, cos_w],
            ));
        }
        if self.pump.is_none() && self.travelling.is_none() {
            let mut terms = vec![
                term(Box::new(move |p| self.loss_source(p).0), vec![cos_w]),
                term(Box::new(move |p| self.loss_source(p).1), vec![sin_w]),
            ];
            terms.extend(kerr);
            return terms;
        }
        let mut terms = vec![
            term(Box::new(move |p| w * m(p) * phi(p)), vec![cos_w]),
            term(Box::new(move |p| div(p) / w), vec![cos_w]),
        ];
        if let Some(pump) = self.pump {
            let big = TAU * pump.frequency_hz;
            let cos_p = Wave {
                nu: big,
                chi: pump.phase,
            };
            let sin_p = Wave::sine(big, pump.phase);
            terms.push(term(
                Box::new(move |p| w * pump.depth * m(p) * phi(p)),
                vec![cos_p, cos_w],
            ));
            terms.push(term(
                Box::new(move |p| -big * pump.depth * m(p) * phi(p)),
                vec![sin_p, sin_w],
            ));
        }
        if let Some(wave) = self.travelling {
            let big = TAU * wave.frequency_hz;
            let cos_z = Wave {
                nu: big,
                chi: wave.phase,
            };
            let sin_z = Wave::sine(big, wave.phase);
            let q = wave.vector();
            let d = wave.depth;
            let along = move |p: Point2| q.dot(p);
            let flux = move |p: Point2| {
                self.skin
                    .tensor(&self.medium, p)
                    .0
                    .apply((self.shape.gradient)(p))
                    .dot(q)
            };
            terms.push(term(
                Box::new(move |p| d * div(p) / w * along(p).cos()),
                vec![cos_z, cos_w],
            ));
            terms.push(term(
                Box::new(move |p| d * div(p) / w * along(p).sin()),
                vec![sin_z, cos_w],
            ));
            terms.push(term(
                Box::new(move |p| d * flux(p) / w * along(p).cos()),
                vec![sin_z, cos_w],
            ));
            terms.push(term(
                Box::new(move |p| -d * flux(p) / w * along(p).sin()),
                vec![cos_z, cos_w],
            ));
        }
        terms
    }

    /// The source rate at `p` and `t` from its terms.
    fn source_at(&self, p: Point2, t: f64) -> f64 {
        self.terms()
            .iter()
            .map(|term| {
                term.waves.iter().fold((term.weight)(p), |value, wave| {
                    value * (wave.nu * t + wave.chi).cos()
                })
            })
            .sum()
    }

    /// Without drives, `S`'s factors of `cos(ωt + θ)` and `sin(ωt + θ)`.
    fn loss_source(&self, p: Point2) -> (f64, f64) {
        let [(gamma, _), _] = self.skin.losses(&self.medium, p);
        let ((alpha, beta), (grad_alpha, grad_beta)) = self.flux_factor(p);
        let (m, phi) = (self.mass(p), (self.shape.value)(p));
        let div = self.divergence(p);
        let flux = self
            .skin
            .tensor(&self.medium, p)
            .0
            .apply((self.shape.gradient)(p));
        (
            self.omega * m * phi - (beta * div + flux.dot(grad_beta)),
            gamma * m * phi - (alpha * div + flux.dot(grad_alpha)),
        )
    }

    /// What the system leaves of `u*` and `b*` at `p` and `t` by finite
    /// differences of the closed forms, the equations as written and
    /// nothing of the derivation above: `S` from `∂_t(p u) + γ_p p u −
    /// η ∇·(A Rᵀ b)`, `A` the scalar stiffness at `t`, and `ḃ + γ_c b −
    /// η R∇u`, which must vanish.
    fn residual(&self, p: Point2, t: f64) -> (f64, Point2) {
        let h = 1.0e-3;
        let d1 = |f: &dyn Fn(f64) -> f64| {
            (-f(2.0 * h) + 8.0 * f(h) - 8.0 * f(-h) + f(-2.0 * h)) / (12.0 * h)
        };
        let [(gamma_p, _), (gamma_c, _)] = self.skin.losses(&self.medium, p);
        let q_t = d1(&|s| self.primary_flux(p, self.field(p, t + s), t + s));
        let eta = self.skin.orientation();
        let flux = |q: Point2| {
            let b = self.complementary(q, t);
            self.tensor_at(q, t).apply(Point2::new(b.y, -b.x)) * eta
        };
        let divergence =
            d1(&|s| flux(p + Point2::new(s, 0.0)).x) + d1(&|s| flux(p + Point2::new(0.0, s)).y);
        let rate = q_t + gamma_p * self.mass_at(p, t) * self.field(p, t) - divergence;
        let grad_u = Point2::new(
            d1(&|s| self.field(p + Point2::new(s, 0.0), t)),
            d1(&|s| self.field(p + Point2::new(0.0, s), t)),
        );
        let b_t = Point2::new(
            d1(&|s| self.complementary(p, t + s).x),
            d1(&|s| self.complementary(p, t + s).y),
        );
        let left = b_t + self.complementary(p, t) * gamma_c - self.rotated(grad_u);
        (rate, left)
    }

    /// The signal `cos(ωt + θ)`.
    fn signal(&self) -> TimeSignal {
        TimeSignal::harmonic(0.0, 1.0, self.omega / TAU, self.phase + FRAC_PI_2)
    }

    /// `u*`'s own signal, `sin(ωt + θ)`, scaled by `amplitude`.
    fn field_signal(&self, amplitude: f64) -> TimeSignal {
        TimeSignal::harmonic(0.0, amplitude, self.omega / TAU, self.phase)
    }

    /// The impedance `√(m a)` of a first-order outgoing wall at `p`, the
    /// medium isotropic there.
    fn impedance(&self, p: Point2) -> f64 {
        (self.mass(p) * self.skin.tensor(&self.medium, p).0.xx).sqrt()
    }

    /// `A ∇φ·n` at `p`.
    fn normal_flux(&self, p: Point2, normal: Point2) -> f64 {
        self.skin
            .tensor(&self.medium, p)
            .0
            .apply((self.shape.gradient)(p))
            .dot(normal)
    }
}

/// The box in `physics` of `medium`, its sides `sides`, the point source
/// off.
fn medium_box(case: &Harmonic) -> Result<TopologyDocument, String> {
    let (physics, medium, sides) = (case.skin.0, &case.medium, case.sides);
    let mut editor = TopologyEditor::default();
    editor.set_physics(physics)?;
    editor.set_domain(BOX)?;
    let mut material = editor.document.model.draft.materials[0].clone();
    let formula =
        |field: FrameField| ScalarField::formula(field.formula).map_err(|e| e.to_string());
    material.mass_density = formula(medium.density)?;
    material.stiffness = formula(medium.stiffness)?;
    material.axis_ratio = formula(medium.axis_ratio)?;
    let channel = |field: Option<FrameField>| -> Result<Option<LossChannel>, String> {
        field
            .map(|field| {
                Ok(LossChannel {
                    base_rate: formula(field)?,
                    law: DampingLaw {
                        rate: RateLaw::Constant,
                        drive: TimeDrive::None,
                        gate: None,
                    },
                })
            })
            .transpose()
    };
    material.electric_loss = channel(medium.electric_loss)?;
    material.magnetic_loss = channel(medium.magnetic_loss)?;
    // The pump on the primary row's coefficient, the travelling modulation
    // dividing the complementary row's: the density and the stiffness slots
    // in Mechanical and TM, the other way round in TE.
    let constant = ScalarField::constant;
    let pump = case.pump.map(|pump| CoefficientLaw {
        drive: TimeDrive::ParametricPump {
            depth: constant(pump.depth),
            frequency_hz: constant(pump.frequency_hz),
            phase_radians: constant(pump.phase),
        },
        ..CoefficientLaw::linear()
    });
    let travelling = case.travelling.map(|wave| CoefficientLaw {
        drive: TimeDrive::TravellingModulation {
            depth: constant(wave.depth),
            frequency_hz: constant(wave.frequency_hz),
            phase_radians: constant(wave.phase),
            wavenumber: constant(wave.wavenumber),
            angle_radians: constant(wave.angle),
        },
        inverted: true,
        ..CoefficientLaw::linear()
    });
    let (primary, complementary) = if case.skin.te() {
        (&mut material.stiffness_law, &mut material.mass_law)
    } else {
        (&mut material.mass_law, &mut material.stiffness_law)
    };
    if let Some(pump) = pump {
        *primary = pump;
    }
    if let Some(travelling) = travelling {
        *complementary = travelling;
    }
    if let Some(chi) = case.kerr {
        primary.field = FieldLaw::Polynomial {
            chi1: constant(0.0),
            chi2: ScalarField::formula(chi.formula).map_err(|e| e.to_string())?,
            amplitude_bound: Some(constant(1.2)),
        };
    }
    editor.update_material(material)?;
    editor.set_region_frame(BACKGROUND_REGION, medium.frame)?;
    if let Some(profile) = case.authored {
        editor.set_volume_source(
            BACKGROUND_REGION,
            Some(VolumeSource {
                region: BACKGROUND_REGION,
                enabled: true,
                profile: ScalarField::formula(profile).map_err(|e| e.to_string())?,
                parameters: [("kx", KX), ("ky", KY), ("kk", -(KX * KX + KY * KY))]
                    .map(|(name, value)| MaterialParameter {
                        name: name.into(),
                        value,
                    })
                    .to_vec(),
                signal: TimeSignal::harmonic(0.0, 1.0, case.omega / TAU, -FRAC_PI_2),
            }),
        )?;
    }
    if let Some((centre, radius)) = case.hole {
        editor.create_closed_curve(
            PeriodicCubicSpline::rounded(centre, radius),
            ClosedCurvePurpose::Hole,
        )?;
    }
    settle(&mut editor);
    let mut document = editor.document;
    let walls = OuterBoundaryConditions {
        sides: sides.map(Side::wall),
    };
    document.model.draft.outer_boundaries = walls;
    document.model.accepted.outer_boundaries = walls;
    document.model.source.enabled = false;
    Ok(document)
}

/// Each element's area and its seven nodes.
fn elements(operator: &CanonicalWaveOperator) -> Vec<(f64, [u32; 7])> {
    let points = operator.node_points();
    operator
        .element_nodes()
        .iter()
        .map(|nodes| {
            let [a, b, c] = [0, 1, 2].map(|k| points[nodes[k] as usize]);
            (0.5 * (b - a).cross(c - a).abs(), *nodes)
        })
        .collect()
}

/// `f` lumped as the mass is: each node's share of each element's area
/// times `f` at the node.
fn lumped(operator: &CanonicalWaveOperator, f: impl Fn(Point2) -> f64) -> Vec<f64> {
    let points = operator.node_points();
    let mut lumped = vec![0.0; points.len()];
    for (area, nodes) in elements(operator) {
        for (local, node) in nodes.iter().enumerate() {
            lumped[*node as usize] += area * MASS_WEIGHTS[local] * f(points[*node as usize]);
        }
    }
    lumped
}

/// `g` along each side whose `side` it is, Simpson's rule on each boundary
/// edge, its two vertices and its midpoint in order along the side.
fn boundary_load(
    operator: &CanonicalWaveOperator,
    sides: [Side; 4],
    wanted: Side,
    g: impl Fn(Point2, Point2) -> f64,
) -> Vec<f64> {
    let points = operator.node_points();
    let mut load = vec![0.0; points.len()];
    for ((normal, on), side) in SIDES.iter().zip(sides) {
        if side != wanted {
            continue;
        }
        let along = Point2::new(-normal.y, normal.x);
        let mut nodes = (0..points.len())
            .filter(|node| on(points[*node]))
            .collect::<Vec<_>>();
        nodes.sort_by(|a, b| points[*a].dot(along).total_cmp(&points[*b].dot(along)));
        for edge in nodes.windows(3).step_by(2) {
            let [a, m, b] = [edge[0], edge[1], edge[2]];
            let length = (points[b] - points[a]).norm();
            // `∫ g N_i` on the edge, the trace's quadratic basis at five
            // Gauss-Legendre points. Simpson's rule on `g`'s nodal values is
            // not this: on `g N_i`, `N_i` quadratic, it errs by `h³` an edge,
            // `h²` along a side, and the field fell to order 2.5.
            const GAUSS: [(f64, f64); 5] = [
                (0.046_910_077_030_668, 0.118_463_442_528_095),
                (0.230_765_344_947_158, 0.239_314_335_249_683),
                (0.5, 0.284_444_444_444_444),
                (0.769_234_655_052_842, 0.239_314_335_249_683),
                (0.953_089_922_969_332, 0.118_463_442_528_095),
            ];
            for (s, w) in GAUSS {
                let q = points[a] + (points[b] - points[a]) * s;
                let basis = [
                    (1.0 - s) * (1.0 - 2.0 * s),
                    4.0 * s * (1.0 - s),
                    s * (2.0 * s - 1.0),
                ];
                for (node, n) in [a, m, b].into_iter().zip(basis) {
                    load[node] += length * w * n * g(q, *normal);
                }
            }
        }
    }
    load
}

/// `g` along the mesh's curve boundaries, the hole's: each boundary edge a
/// chord of the element it bounds, its two vertices and its midpoint node,
/// the trace's quadratic basis at five Gauss-Legendre points, `g` given the
/// point and the domain's outward normal there, away from the element.
fn curve_load(prepared: &PreparedTopology, g: impl Fn(Point2, Point2) -> f64) -> Vec<f64> {
    let operator = &prepared.canonical_operator;
    let points = operator.node_points();
    let mesh = &prepared.mesh;
    let mut load = vec![0.0; points.len()];
    const GAUSS: [(f64, f64); 5] = [
        (0.046_910_077_030_668, 0.118_463_442_528_095),
        (0.230_765_344_947_158, 0.239_314_335_249_683),
        (0.5, 0.284_444_444_444_444),
        (0.769_234_655_052_842, 0.239_314_335_249_683),
        (0.953_089_922_969_332, 0.118_463_442_528_095),
    ];
    for edge in &mesh.boundary_edges {
        if !matches!(edge.label, funfern_core::BoundaryLabel::Curve { .. }) {
            continue;
        }
        let [va, vb] = edge.vertices;
        assert!(
            points[va] == mesh.vertices[va].point && points[vb] == mesh.vertices[vb].point,
            "a mesh vertex is not its canonical node"
        );
        let (nodes, [a, b, c, m]) = operator
            .element_nodes()
            .iter()
            .find_map(|nodes| {
                let local = |v: usize| nodes[..3].iter().position(|n| *n as usize == v);
                let (i, j) = (local(va)?, local(vb)?);
                let mid = match (i.min(j), i.max(j)) {
                    (0, 1) => 3,
                    (1, 2) => 4,
                    _ => 5,
                };
                Some((nodes, [i, j, 3 - i - j, mid]))
            })
            .expect("a boundary edge bounds an element");
        let [pa, pb, pc] = [a, b, c].map(|k| points[nodes[k] as usize]);
        let tangent = pb - pa;
        let mut normal = Point2::new(tangent.y, -tangent.x) * (1.0 / tangent.norm());
        if normal.dot(pc - pa) > 0.0 {
            normal = normal * -1.0;
        }
        let ends = [nodes[a] as usize, nodes[m] as usize, nodes[b] as usize];
        for (t, w) in GAUSS {
            let q = pa + tangent * t;
            let basis = [
                (1.0 - t) * (1.0 - 2.0 * t),
                4.0 * t * (1.0 - t),
                t * (2.0 * t - 1.0),
            ];
            for (node, n) in ends.into_iter().zip(basis) {
                load[node] += tangent.norm() * w * n * g(q, normal);
            }
        }
    }
    load
}

/// A run's end: the field at the nodes and the complementary flux, and what
/// they read against `u*` there, the field's error weighted by the mass and
/// the flux's by the samples' integration weights, each over its closed
/// form's amplitude.
struct Ended {
    field: Vec<f64>,
    flux: Vec<Point2>,
    field_error: f64,
    flux_error: f64,
    largest: f64,
    /// Where the largest nodal error is.
    worst: Point2,
}

/// A run's state, on a fixed or a time-driven generation.
enum Run {
    Fixed(CanonicalWaveState),
    Temporal(CanonicalTemporalWaveState),
}

impl Run {
    fn complementary(&self) -> &[Point2] {
        match self {
            Self::Fixed(state) => state.complementary_flux(),
            Self::Temporal(state) => state.complementary_flux(),
        }
    }
}

/// `case` on `prepared`, from its state at 0 to `duration` at `share` of
/// the recommended step under `integrator`.
fn run(
    prepared: &PreparedTopology,
    case: &Harmonic,
    duration: f64,
    share: f64,
    integrator: CanonicalIntegrator,
) -> Result<Ended, String> {
    let operator = &prepared.canonical_operator;
    let points = operator.node_points();
    let mass = lumped(operator, |p| case.mass(p));
    let temporal = prepared.canonical_temporal_operator.as_ref();
    let theirs = match temporal {
        Some(temporal) => temporal
            .primary_mass_at(0.0, &temporal.initial_runtime())
            .map_err(|error| error.to_string())?,
        None => operator.primary_mass().to_vec(),
    };
    let starting = lumped(operator, |p| case.mass_at(p, 0.0));
    let worst = starting
        .iter()
        .zip(&theirs)
        .map(|(mine, its)| (mine / its - 1.0).abs())
        .fold(0.0, f64::max);
    if worst > 1.0e-12 {
        return Err(format!("the lumped mass is off by {worst:.3e}"));
    }
    // What of a gradient's samples lies in the coupling's kernel is an
    // interpolation's remainder, which never moves, and is left out; a `b*`
    // with a curl has an order-one part there, which a loss that varies
    // couples back to the field, and is kept whole.
    let potential = case.potential(points);
    let flux_at = |t: f64| -> Result<Vec<Point2>, String> {
        let sampled = operator
            .constitutive_samples()
            .iter()
            .map(|sample| case.complementary(sample.point, t))
            .collect::<Vec<_>>();
        if !potential {
            return Ok(sampled);
        }
        let stationary = operator
            .stationary_complementary_component(&sampled)
            .map_err(|error| error.to_string())?;
        Ok(sampled
            .iter()
            .zip(stationary)
            .map(|(sampled, stationary)| *sampled - stationary)
            .collect())
    };
    let primary = points
        .iter()
        .zip(&starting)
        .map(|(p, m)| {
            let u = case.field(*p, 0.0);
            m * (1.0 + case.chi(*p) * u * u) * u
        })
        .collect::<Vec<_>>();
    let steps = (duration / (share * prepared.recommended_time_step())).ceil() as u64;
    let dt = duration / steps as f64;
    let mut state = match temporal {
        Some(temporal) => Run::Temporal(
            CanonicalTemporalWaveState::new(temporal, dt, primary, flux_at(0.0)?)
                .map_err(|error| error.to_string())?
                .with_integrator(integrator),
        ),
        None => Run::Fixed(
            CanonicalWaveState::new(operator, dt, primary, flux_at(0.0)?)
                .map_err(|error| error.to_string())?
                .with_integrator(integrator),
        ),
    };
    let prescribed = prepared
        .canonical_forcing
        .prescribed()
        .iter()
        .zip(points)
        .map(|(pinned, p)| pinned.map(|_| case.field_signal((case.shape.value)(*p))))
        .collect();
    let mut forcing = CanonicalForcing::from_prescribed(operator, prescribed)
        .map_err(|error| error.to_string())?;
    if case.authored.is_some() {
        forcing = (*prepared.canonical_forcing).clone();
    }
    // What drives `cos(ωt + θ)`: the bulk, a Neumann side's flux and the
    // flux part of an outgoing wall's remainder; what drives
    // `sin(ωt + θ)`: the remainder's `√(m a) u*`.
    let flux_load = |side| {
        boundary_load(operator, case.sides, side, |p, n| {
            -case.normal_flux(p, n) / case.omega
        })
    };
    let hole = if case.hole.is_some() {
        curve_load(prepared, |p, n| -case.normal_flux(p, n) / case.omega)
    } else {
        vec![0.0; points.len()]
    };
    let cosine = flux_load(Side::Flux)
        .iter()
        .zip(flux_load(Side::Outgoing))
        .zip(hole)
        .map(|((flux, outgoing), hole)| flux + outgoing + hole)
        .collect::<Vec<_>>();
    let sine = boundary_load(operator, case.sides, Side::Outgoing, |p, _| {
        case.impedance(p) * (case.shape.value)(p)
    });
    let mut channels = vec![(cosine, case.signal()), (sine, case.field_signal(1.0))];
    for term in case.terms().into_iter().filter(|_| case.authored.is_none()) {
        let weights = lumped(operator, &term.weight);
        for (share, wave) in term.expanded() {
            channels.push((
                weights.iter().map(|weight| share * weight).collect(),
                wave.signal(),
            ));
        }
    }
    for (weights, signal) in channels {
        if weights.iter().all(|weight| *weight == 0.0) {
            continue;
        }
        forcing
            .push_source(
                CanonicalSource::direct(operator, weights, signal)
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
    }
    for _ in 0..steps {
        match (&mut state, temporal) {
            (Run::Fixed(state), _) => state.step_with_forcing(operator, &forcing).map(|_| ()),
            (Run::Temporal(state), Some(temporal)) => {
                state.step_with_forcing(temporal, &forcing).map(|_| ())
            }
            (Run::Temporal(_), None) => unreachable!("a driven state on a fixed generation"),
        }
        .map_err(|error| error.to_string())?;
    }
    let (t, field) = match (&state, temporal) {
        (Run::Fixed(state), _) => (
            state.time(),
            state.primary_field(operator).map_err(|e| e.to_string())?,
        ),
        (Run::Temporal(state), Some(temporal)) => (
            state.time(),
            temporal
                .primary_field_at(state.primary_flux(), state.time(), state.runtime())
                .map_err(|e| e.to_string())?,
        ),
        (Run::Temporal(_), None) => unreachable!("a driven state on a fixed generation"),
    };
    let shape = points
        .iter()
        .map(|p| (case.shape.value)(*p))
        .collect::<Vec<_>>();
    let norm = shape
        .iter()
        .zip(&mass)
        .map(|(s, m)| m * s * s)
        .sum::<f64>()
        .sqrt();
    let error = points
        .iter()
        .zip(&field)
        .zip(&mass)
        .map(|((p, u), m)| m * (u - case.field(*p, t)).powi(2))
        .sum::<f64>()
        .sqrt();
    let (largest, worst) = points
        .iter()
        .zip(&field)
        .map(|(p, u)| ((u - case.field(*p, t)).abs(), *p))
        .fold((0.0, Point2::default()), |best, next| {
            if next.0 > best.0 { next } else { best }
        });
    let largest = largest / shape.iter().fold(0.0_f64, |m, s| m.max(s.abs()));
    let exact = flux_at(t)?;
    let samples = operator.constitutive_samples();
    let flux_norm = samples
        .iter()
        .map(|sample| {
            sample.integration_weight * case.complementary_amplitude(sample.point).powi(2)
        })
        .sum::<f64>()
        .sqrt();
    let flux = state.complementary().to_vec();
    let flux_error = samples
        .iter()
        .zip(&flux)
        .zip(&exact)
        .map(|((sample, b), e)| {
            let d = *b - *e;
            sample.integration_weight * d.dot(d)
        })
        .sum::<f64>()
        .sqrt();
    Ok(Ended {
        field,
        flux,
        field_error: error / norm,
        flux_error: flux_error / flux_norm,
        largest,
        worst,
    })
}

/// The hand-derived source against the finite-difference residual of the
/// closed forms at points over the box and times over a period, over the
/// source's size, and what `b*` leaves of its own equation, over `η R∇u*`'s.
fn derivation_error(case: &Harmonic) -> (f64, f64) {
    let (mut worst, mut size, mut left, mut gradient): (f64, f64, f64, f64) = (0.0, 0.0, 0.0, 0.0);
    for i in 0..7 {
        for j in 0..5 {
            let p = Point2::new(-0.93 + 0.31 * i as f64, -0.61 + 0.3 * j as f64);
            for k in 0..4 {
                let t = 0.37 + 1.3 * k as f64;
                let derived = case.source_at(p, t);
                let (rate, remainder) = case.residual(p, t);
                worst = worst.max((derived - rate).abs());
                size = size.max(derived.abs());
                left = left.max(remainder.norm());
                gradient = gradient.max((case.shape.gradient)(p).norm());
            }
        }
    }
    (worst / size, left / gradient)
}

/// Case 7's Kerr coefficient, `χ = 0.3 + 0.1 sin(x) cos(y)`.
const KERR: FrameField = FrameField {
    formula: "0.3 + 0.1 * sin(x) * cos(y)",
    value: |x, y| 0.3 + 0.1 * x.sin() * y.cos(),
    gradient: |x, y| (0.1 * x.cos() * y.cos(), -0.1 * x.sin() * y.sin()),
};

/// Case 5's drives: a pump at 0.25 Hz, and a modulation at 0.18 Hz
/// travelling along 0.6 rad at a wavenumber of 1.2.
const PUMP: Pump = Pump {
    depth: 0.2,
    frequency_hz: 0.25,
    phase: 0.3,
};
const TRAVELLING: Travelling = Travelling {
    depth: 0.15,
    frequency_hz: 0.18,
    phase: -0.4,
    wavenumber: 1.2,
    angle: 0.6,
};

/// Case 1's acceleration `S/m` in Mechanical at `ω = 1`, `φ + ∇·(k ∇φ)/ρ`,
/// as the app's formulas write it: `∇·(k ∇φ) = k Δφ + ∇k·∇φ`, `Δφ =
/// −(k_x² + k_y²) φ`, the wavenumbers and their squares' sum parameters.
const AUTHORED_PROFILE: &str = "cos(kx*(x+1))*cos(ky*(y+0.7))*(1+(1+0.25*cos(x+y))*kk/(1+0.3*sin(x)*cos(2*y)))\
    +0.25*sin(x+y)*(kx*sin(kx*(x+1))*cos(ky*(y+0.7))+ky*cos(kx*(x+1))*sin(ky*(y+0.7)))\
    /(1+0.3*sin(x)*cos(2*y))";

const SKINS: [PhysicsModel; 3] = [
    PhysicsModel::Mechanical,
    PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Tm,
    },
    PhysicsModel::Electromagnetic {
        polarization: ElectromagneticPolarization::Te,
    },
];

/// The cases the study runs, by name.
fn cases(skin: PhysicsModel) -> Vec<(&'static str, Harmonic)> {
    let harmonic = |medium, shape, sides| Harmonic {
        skin: Skin(skin),
        medium,
        shape,
        omega: 1.0,
        phase: PI / 3.0,
        sides,
        pump: None,
        travelling: None,
        kerr: None,
        hole: None,
        authored: None,
    };
    vec![
        (
            "heterogeneous",
            harmonic(HETEROGENEOUS, COSINE_SHAPE, [Side::Natural; 4]),
        ),
        (
            "anisotropic",
            harmonic(ANISOTROPIC, SINE_SHAPE, [Side::Pinned; 4]),
        ),
        (
            "mixed",
            harmonic(
                HETEROGENEOUS,
                GENERIC_SHAPE,
                [Side::Pinned, Side::Flux, Side::Outgoing, Side::Pinned],
            ),
        ),
        ("lossy", harmonic(LOSSY, COSINE_SHAPE, [Side::Natural; 4])),
        (
            "modulated",
            Harmonic {
                pump: Some(PUMP),
                travelling: Some(TRAVELLING),
                ..harmonic(HETEROGENEOUS, COSINE_SHAPE, [Side::Natural; 4])
            },
        ),
        (
            "pumped",
            Harmonic {
                pump: Some(PUMP),
                ..harmonic(HETEROGENEOUS, COSINE_SHAPE, [Side::Natural; 4])
            },
        ),
        (
            "travelling",
            Harmonic {
                travelling: Some(TRAVELLING),
                ..harmonic(HETEROGENEOUS, COSINE_SHAPE, [Side::Natural; 4])
            },
        ),
        (
            "authored",
            Harmonic {
                phase: FRAC_PI_2,
                authored: Some(AUTHORED_PROFILE),
                ..harmonic(HETEROGENEOUS, COSINE_SHAPE, [Side::Natural; 4])
            },
        ),
        (
            "hole",
            Harmonic {
                hole: Some((Point2::new(0.2, 0.05), 0.35)),
                ..harmonic(HETEROGENEOUS, GENERIC_SHAPE, [Side::Pinned; 4])
            },
        ),
        (
            "kerr",
            Harmonic {
                kerr: Some(KERR),
                ..harmonic(HETEROGENEOUS, COSINE_SHAPE, [Side::Natural; 4])
            },
        ),
        (
            "electric",
            harmonic(
                Medium {
                    magnetic_loss: None,
                    ..LOSSY
                },
                COSINE_SHAPE,
                [Side::Natural; 4],
            ),
        ),
        (
            "magnetic",
            harmonic(
                Medium {
                    electric_loss: None,
                    ..LOSSY
                },
                COSINE_SHAPE,
                [Side::Natural; 4],
            ),
        ),
        (
            "constant-loss",
            harmonic(
                Medium {
                    electric_loss: Some(FrameField::constant("0.3", |_, _| 0.3)),
                    magnetic_loss: Some(FrameField::constant("0.25", |_, _| 0.25)),
                    ..LOSSY
                },
                COSINE_SHAPE,
                [Side::Natural; 4],
            ),
        ),
        (
            "pinned",
            harmonic(HETEROGENEOUS, GENERIC_SHAPE, [Side::Pinned; 4]),
        ),
        (
            "flux",
            harmonic(HETEROGENEOUS, GENERIC_SHAPE, [Side::Flux; 4]),
        ),
        (
            "outgoing",
            harmonic(HETEROGENEOUS, GENERIC_SHAPE, [Side::Outgoing; 4]),
        ),
    ]
}

/// PH-19's study: each case at three mesh edges with the step a small share
/// of the recommended one; and on the middle mesh each integrator at three
/// steps against a step 32 times smaller, the semidiscrete reference.
#[test]
#[ignore = "PH-19's study: cargo test --release -p funfern-app --test manufactured -- --ignored --nocapture"]
fn study_manufactured_solutions() {
    let variable = |name: &str| std::env::var(name).ok();
    let duration: f64 = variable("PH19_DURATION")
        .and_then(|v| v.parse().ok())
        .unwrap_or(4.0);
    let wanted = variable("PH19_CASE");
    let share: f64 = variable("PH19_SHARE")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.25);
    for physics in SKINS {
        for (name, case) in cases(physics) {
            if wanted.as_deref().is_some_and(|wanted| wanted != name) {
                continue;
            }
            // The authored profile is Mechanical's formula.
            if case.authored.is_some() && physics != PhysicsModel::Mechanical {
                continue;
            }
            let (source, flux) = derivation_error(&case);
            eprintln!(
                "{name} {physics:?}: the derivation against the residual, {source:.3e}, b* against its law, {flux:.3e}"
            );
            let document = medium_box(&case).unwrap();
            for edge in [0.2, 0.1, 0.05] {
                let started = std::time::Instant::now();
                let (_, _, prepared) = prepare_at(&document, edge).unwrap();
                match run(
                    &prepared,
                    &case,
                    duration,
                    share,
                    CanonicalIntegrator::FourthOrder,
                ) {
                    Ok(ended) => eprintln!(
                        "  edge {edge}: {} nodes in {:.1} s: field {:.3e}, flux {:.3e}, largest {:.3e} at ({:.3}, {:.3})",
                        prepared.canonical_operator.degrees_of_freedom(),
                        started.elapsed().as_secs_f64(),
                        ended.field_error,
                        ended.flux_error,
                        ended.largest,
                        ended.worst.x,
                        ended.worst.y
                    ),
                    Err(error) => eprintln!("  edge {edge}: FAILED {error}"),
                }
            }
            if variable("PH19_TIME").is_none() {
                continue;
            }
            let (_, _, prepared) = prepare_at(&document, 0.1).unwrap();
            for integrator in [
                CanonicalIntegrator::FourthOrder,
                CanonicalIntegrator::Leapfrog,
            ] {
                let reference = run(&prepared, &case, duration, 1.0 / 32.0, integrator).unwrap();
                let mass = prepared.canonical_operator.primary_mass();
                let norm = reference
                    .field
                    .iter()
                    .zip(mass)
                    .map(|(u, m)| m * u * u)
                    .sum::<f64>()
                    .sqrt();
                // The recommended step is the fourth-order step's, past the
                // leapfrog's bound; the app's leapfrog took 0.9 of its own.
                let scale = match integrator {
                    CanonicalIntegrator::Leapfrog => 0.75,
                    CanonicalIntegrator::FourthOrder => 1.0,
                };
                for share in [1.0, 0.5, 0.25].map(|share| share * scale) {
                    let ended = run(&prepared, &case, duration, share, integrator).unwrap();
                    let apart = ended
                        .field
                        .iter()
                        .zip(&reference.field)
                        .zip(mass)
                        .map(|((u, r), m)| m * (u - r).powi(2))
                        .sum::<f64>()
                        .sqrt();
                    let flux_apart = ended
                        .flux
                        .iter()
                        .zip(&reference.flux)
                        .map(|(b, r)| (*b - *r).norm())
                        .fold(0.0, f64::max);
                    eprintln!(
                        "  time, {integrator:?} at share {share}: field {:.3e} from the reference, flux {:.3e} at most",
                        apart / norm,
                        flux_apart
                    );
                }
            }
        }
    }
}

/// The edges the gate runs each case at, and the finest the nightly adds.
const GATE: [f64; 2] = [0.2, 0.1];
const NIGHTLY: [f64; 3] = [0.2, 0.1, 0.05];

/// How long each run runs, about two thirds of `u*`'s period.
const DURATION: f64 = 4.0;

/// The skins `name` runs in: every one, but the authored profile, which is
/// Mechanical's formula.
fn skins(name: &str) -> &'static [PhysicsModel] {
    if name == "authored" {
        &SKINS[..1]
    } else {
        &SKINS
    }
}

/// `name` in `physics`.
fn case(name: &str, physics: PhysicsModel) -> Harmonic {
    cases(physics)
        .into_iter()
        .find(|(candidate, _)| *candidate == name)
        .unwrap_or_else(|| panic!("no case {name}"))
        .1
}

/// PH-19's bounds on case `name` in each skin: its source derived by hand
/// within 1e-9 of the residual of the closed forms, and `b*` of its law;
/// over `edges`, each halving, the field's error at the nodes falling at
/// least `2^2.6` times and the flux's `2^flux_order`; and the field's at
/// the last edge at most `bound`.
fn converges(name: &str, edges: &[f64], flux_order: f64, bound: f64) {
    for &physics in skins(name) {
        let case = case(name, physics);
        let (source, law) = derivation_error(&case);
        assert!(
            source < 1.0e-9 && law < 1.0e-9,
            "{name} {physics:?}: the derivation {source:.3e} and b* {law:.3e} from the residual"
        );
        let document = medium_box(&case).unwrap();
        let ended = edges
            .iter()
            .map(|edge| {
                let (_, _, prepared) = prepare_at(&document, *edge).unwrap();
                run(
                    &prepared,
                    &case,
                    DURATION,
                    0.25,
                    CanonicalIntegrator::FourthOrder,
                )
                .unwrap_or_else(|error| panic!("{name} {physics:?} at {edge}: {error}"))
            })
            .collect::<Vec<_>>();
        let errors = ended
            .iter()
            .map(|ended| (ended.field_error, ended.flux_error))
            .collect::<Vec<_>>();
        for pair in errors.windows(2) {
            let field = (pair[0].0 / pair[1].0).log2();
            let flux = (pair[0].1 / pair[1].1).log2();
            assert!(
                field >= 2.6 && flux >= flux_order,
                "{name} {physics:?}: orders {field:.2} and {flux:.2} over {errors:?}"
            );
        }
        let last = errors.last().unwrap().0;
        assert!(
            last <= bound,
            "{name} {physics:?}: the field's error {last:.3e} at {} over {bound:.1e}",
            edges.last().unwrap()
        );
    }
}

/// PH-19 1: a medium whose density and stiffness vary smoothly over the box,
/// reflecting walls. 6.0e-5 at edge 0.1 at most, order 2.9 to 3.0 in the
/// field and 3.0 in the flux.
#[test]
fn a_heterogeneous_medium_converges_at_third_order() {
    converges("heterogeneous", &GATE, 2.2, 9.0e-5);
}

/// PH-19 2: a frame turned half a radian, an axis ratio and a stiffness that
/// vary in it, pinned walls: the tensor's off-diagonal term varies over the
/// box. 4.9e-5 at edge 0.1, order 2.85; the flux's 2.5, which the pinned
/// sides take from the samples' superconvergence.
#[test]
fn an_anisotropic_medium_in_a_turned_frame_converges() {
    converges("anisotropic", &GATE, 2.2, 7.5e-5);
}

/// PH-19 3: two walls pinned to `u*` node by node, a Neumann side whose
/// flux varies along it and a first-order outgoing wall `u*` does not keep,
/// each loaded with what `u*` asks of it. 1.75e-5 at edge 0.1, order 2.85.
#[test]
fn mixed_boundaries_converge() {
    converges("mixed", &GATE, 2.2, 2.7e-5);
}

/// PH-19 4: electric and magnetic losses varying over the box, the
/// complementary flux periodic under its own loss and so with a curl. 1.9e-5
/// at edge 0.1, order 2.85; the flux's 1.95, compared whole.
#[test]
fn losses_varying_in_space_converge() {
    converges("lossy", &GATE, 1.8, 2.8e-5);
}

/// PH-19 5: a parametric pump on the mass and a travelling modulation of
/// the stiffness, the forcing at `ω` and `ω ± Ω` of each. 6.0e-5 at edge
/// 0.1, order 2.9.
#[test]
fn modulated_coefficients_converge() {
    converges("modulated", &GATE, 2.2, 9.0e-5);
}

/// PH-19 6: a reflecting hole on the app's spline, the mesh graded along
/// it, loaded with `u*`'s flux along the mesh's own chords; pinned outer
/// walls. 2.2e-5 at edge 0.1, order 2.9.
#[test]
fn a_curved_hole_converges() {
    converges("hole", &GATE, 2.2, 3.3e-5);
}

/// PH-19 7: a Kerr law on the primary row, `χ = 0.3 + 0.1 sin(x) cos(y)` at
/// unit amplitude, the forcing at `ω` and `3ω`. 5.6e-5 at edge 0.1, order 2.9.
#[test]
fn a_kerr_law_converges() {
    converges("kerr", &GATE, 2.2, 8.5e-5);
}

/// PH-19: case 1's source authored as the app's own volume source, a
/// formula profile and a harmonic the app integrates from zero: it drives
/// the field as the forcing built here does, to 1e-9, and converges.
#[test]
fn an_authored_volume_source_drives_the_manufactured_field() {
    converges("authored", &GATE, 2.2, 2.0e-5);
    let authored = case("authored", PhysicsModel::Mechanical);
    let built = Harmonic {
        authored: None,
        ..authored
    };
    let (_, _, prepared) = prepare_at(&medium_box(&authored).unwrap(), 0.1).unwrap();
    let [a, b] = [authored, built].map(|case| {
        run(
            &prepared,
            &case,
            DURATION,
            0.25,
            CanonicalIntegrator::FourthOrder,
        )
        .unwrap()
        .field
    });
    let mass = prepared.canonical_operator.primary_mass();
    let norm = |v: &[f64]| {
        v.iter()
            .zip(mass)
            .map(|(u, m)| m * u * u)
            .sum::<f64>()
            .sqrt()
    };
    let apart = a.iter().zip(&b).map(|(x, y)| x - y).collect::<Vec<_>>();
    assert!(
        norm(&apart) <= 1.0e-9 * norm(&b),
        "the authored source drives {:.3e} apart",
        norm(&apart) / norm(&b)
    );
}

/// PH-19 at the finest mesh, nightly: every case at edges 0.2, 0.1 and
/// 0.05, both halvings held to the gate's orders, the field's error at 0.05
/// to 1.5 times the study's largest in any skin.
#[test]
#[ignore = "nightly: PH-19 at the finest mesh, about 20 minutes"]
fn manufactured_solutions_converge_on_the_finest_mesh() {
    for (name, flux_order, bound) in [
        ("heterogeneous", 2.2, 1.2e-5),
        ("anisotropic", 2.2, 9.0e-6),
        ("mixed", 2.2, 3.5e-6),
        ("lossy", 1.8, 3.5e-6),
        ("modulated", 2.2, 1.1e-5),
        ("hole", 2.2, 4.0e-6),
        ("kerr", 2.2, 1.05e-5),
        ("authored", 2.2, 2.5e-6),
    ] {
        converges(name, &NIGHTLY, flux_order, bound);
    }
}
