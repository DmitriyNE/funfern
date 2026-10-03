# IGA feasibility spikes — 3 October 2026

**Status: all spikes measured.** Gate 1 passed; gate 3 failed for smooth
boundaries and passed for a deformed box; gate 4 passed for knot-aligned
regions and failed for free curves; gate 5 passed, with spike 6 folded in. Everything here is CPU f64 in
`crates/funfern-core/src/spline_patch.rs` and the two examples
`iga_dispersion` and `iga_box_mode`; nothing is reached by the application.

## The question

Plan §10 names single-patch isogeometric analysis (IGA) as the next big piece,
and the user's standing concern is whether a spline discretization can keep
the triangle solver's live-reassembly philosophy, above all its mass
treatment and its quadrature. The spikes are ordered so the two concerns are
answered first and cheapest, and the rest only if they pass:

1. **Mass treatment** on an affine box patch: row-sum lumped, lumped with
   Jacobi sweeps toward the consistent mass, and consistent as the reference.
   Gate: a variant must match the seven-node triangle's phase error at no
   more than its per-dof cost, sweeps included, or IGA stops here.
2. **Quadrature point sets**: Gauss 1 to 4 points per span per direction.
   Spurious modes, dispersion shift, samples per dof.
3. Interior parameterization of a patch from a closed boundary spline.
4. Materials inside a patch: immersed sampling against knot-aligned regions.
5. Edits without remeshing: local rebuild, mass rescale, Piola map of `b`.
6. Cost at equal phase error against the triangle solver.
7. (T, added after spike 1) A fourth-order time integrator, since
   leapfrog's temporal error dominates at the stable step.

## What the triangle solver fixes for any IGA variant

The canonical `(Q, b)` scheme never assembles a matrix. The kick gathers each
node's force from its samples through one curl table; the drift builds each
sample's gradient from the same table, so divergence is the exact transpose
of gradient and the energy accounting rests on that. The stored primary state
is the integrated flux `Q`, the field is `Q` divided by a per-node mass, and
the constitutive law is pointwise at the `b` samples. Edits reassemble
incrementally and land through a transaction with a field transfer.

An IGA variant therefore needs:

- one shared point set with one weight per point, so that `Gᵀ W` is the
  transpose of `G`. That rules out weighted quadrature (Calabrò, Sangalli,
  Tani), whose weights differ per test function;
- a field map `u = P Q` that is symmetric positive definite and matrix-free.
  The energy `½ Qᵀ P Q + ½ bᵀ W b` is then conserved by the leapfrog for any
  symmetric `P`, diagonal or not. That rules out the Petrov-Galerkin form of
  dual-basis lumping, where the test space differs from the trial space; it
  admits its Galerkin reading (below);
- `b` living at the quadrature points, so fewer points mean fewer
  complementary unknowns and coarser sampling of a nonlinear law;
- local rebuilding of a span's table when a control point moves. On a tensor
  patch a control point touches `(p+1)²` spans, so an edit is local without a
  mesher.

### The mass treatments that fit

Row-sum lumping `D = diag(M 1)` is always positive for B-splines (nonnegative
basis, partition of unity), but the literature is unambiguous that it is
second order whatever the degree, with a constant that grows with the degree
(Cottrell et al. 2006, restated in Nguyen et al. 2023). The variant measured
here keeps the lumped diagonal and adds `K` Jacobi sweeps toward the
consistent mass:

```text
P_K = D⁻¹ Σ_{k=0}^{K} (I − M D⁻¹)^k       u = P_K Q
```

applied as `x₀ = D⁻¹Q`, `x_{k+1} = x_k + D⁻¹(Q − M x_k)`, each sweep one mass
application through the sample tables, which is a gather and a scatter like
the drift and the kick together. `D⁻¹M` is row-stochastic for B-splines, so
`D^{-½} M D^{-½}` has its spectrum in `(0, 1]`, the series converges, and
`P_K = D^{-½} Σ (I − S)^k D^{-½}` is symmetric positive definite. It is the
Taylor series of `1/s` at `s = 1`, so each sweep buys two orders in the
wavenumber on the resolved modes and barely moves the unresolved ones, which
keeps the stable step near the lumped one. Voet, Sande and Buffa (2023) prove
the monotone ordering of the lumped, intermediate and consistent spectra that
this rides on, and their banded generalized lumping is a cousin of it. The
approximate-dual lumping of Nguyen, Hiemstra, Eisenträger and Schillinger
(2023), which the user pointed at via Anitescu et al. (2019), reads in
Galerkin form as a fixed banded SPD `P = S`; it fits the same slot and is
precomputed, where the sweep form follows a time-driven density for free. It
was not measured here.

The consistent mass is applied by conjugate gradients to roundoff and serves
as the reference only.

## Method

- `UniformBasis`: open and periodic uniform B-splines of degree 1 to 3 by
  Cox-de Boor, values and first derivatives.
- `PeriodicSymbols`: the circulant mass and stiffness symbols of a uniform
  periodic basis under a given rule, built numerically from one row of the
  periodic matrices. For a tensor patch of square spans with unit material the
  2D symbols are `σ_K(θ)σ_M(φ) + σ_M(θ)σ_K(φ)` and `σ_M(θ)σ_M(φ)`, the lumped
  one `h²`, and the sweep symbol follows from them, so a whole dispersion
  study is arithmetic. The cubic symbols are checked against the closed
  forms `151/315, 397/1680, 1/42, 1/5040` and `2/3, −1/8, −1/5, −1/120`.
- `SplinePatch`: an affine tensor-product patch over the 2 × 2 box with
  clamped walls, Gauss samples per span, sum-factorized 1D tables, `apply_mass`,
  `gradient`, `divergence`, `field`, L2 projection, and the top eigenvalue of
  `P K` by power iteration.
- `PatchStepper`: the leapfrog `Q̇ = −Gᵀ W b`, `ḃ = G u`, `u = P Q`, with the
  staggered energy `½ Qᵀ P Q + ½ b⁻ · W b⁺` checked to roundoff.
- The time-domain test is the protocol of `wave_convergence`: the reflecting
  box mode `cos(5π(x+1))`, wavelength 0.4, wave speed 1, phase and amplitude
  read from two field levels at `t = 2` and `t = 10`. One difference: that
  example separates the sine component with the exact angle `ω dt`, which
  offsets its phase by about the relative frequency error in radians; the patch
  readout separates it with the predicted discrete frequency, which the
  linear-basis test shows to be exact. On a linear basis the nodal cosine is
  an exact eigenvector of every treatment with the periodic symbol's
  eigenvalue, and the stepped phase matches the leapfrog frequency to 1e-9.

The baseline, rerun today on this machine (`wave_convergence`, f64 CPU):

| element | parent h | DOFs | dt_max (Gershgorin) | phase at t=10, 0.225 dt_max | at 0.1125 dt_max | sim s / wall s |
| --- | --- | --- | --- | --- | --- | --- |
| P2e | 0.08 | 9,215 | 6.98e-3 | −5.14e-3 rad | −8.13e-3 rad | 4.4 (at 0.225) |
| P2e | 0.04 | 37,395 | 3.45e-3 | +5.07e-4 rad | −2.23e-4 rad | 0.51 |
| P1 | 0.04 | 6,320 | 1.01e-2 | −0.960 rad | −0.966 rad | 35 |
| P1 | 0.02 | 25,217 | 5.08e-3 | −0.238 rad | −0.240 rad | 4.2 |

Extrapolating the two P2e steps to `dt → 0`, its spatial phase error at
parent h 0.08 is about −9.1e-3 rad over 157 rad, a relative frequency error of
−5.8e-5 at a dof spacing of 19 per wavelength. That is the number to beat at
equal dofs: a 96 × 96 patch has 9,216 coefficients and 18.6 (cubic) or 18.8
(quadratic) spans per wavelength.

## Spike 1: mass treatment, from the symbols

Relative frequency error `ω_h/ω − 1` of a plane wave along a parametric axis,
by spans per wavelength; the diagonal direction differs by less than a factor
of two except for the consistent cubic, which is ten times better on the
diagonal. Three-point Gauss; `dt_max` is the leapfrog limit over the
periodic band in units of `h/c`. The full table, with every rule and the
diagonal direction, is what `iga_dispersion` prints.

| basis / mass | 8 ppw | 10 ppw | 15 ppw | 20 ppw | 30 ppw | dt_max |
| --- | --- | --- | --- | --- | --- | --- |
| p=2 lumped | −7.4e-2 | −4.8e-2 | −2.2e-2 | −1.2e-2 | −5.5e-3 | 1.63 |
| p=2 lumped+1 | −1.0e-2 | −4.3e-3 | −9.0e-4 | −2.9e-4 | −5.8e-5 | 1.24 |
| p=2 lumped+2 | −1.2e-3 | −3.0e-4 | −1.8e-5 | −3.5e-7 | +7.0e-7 | 1.07 |
| p=2 lumped+3 | +8.7e-5 | +7.8e-5 | +2.0e-5 | +6.7e-6 | +1.3e-6 | 0.96 |
| p=2 consistent | +3.0e-4 | +1.2e-4 | +2.2e-5 | +6.9e-6 | +1.3e-6 | 0.45 |
| p=3 lumped | −9.8e-2 | −6.4e-2 | −2.9e-2 | −1.6e-2 | −7.3e-3 | 1.92 |
| p=3 lumped+1 | −1.8e-2 | −7.7e-3 | −1.6e-3 | −5.2e-4 | −1.1e-4 | 1.48 |
| p=3 lumped+2 | −3.3e-3 | −9.4e-4 | −9.2e-5 | −1.7e-5 | −1.5e-6 | 1.30 |
| p=3 lumped+3 | −6.0e-4 | −1.2e-4 | −5.1e-6 | −5.3e-7 | −2.0e-8 | 1.19 |
| p=3 consistent | +6.4e-6 | +1.5e-6 | +1.2e-7 | +2.1e-8 | +1.8e-9 | 0.45 |
| p=1 lumped (bilinear) | −2.6e-2 | −1.6e-2 | −7.3e-3 | −4.1e-3 | −1.8e-3 | 1.00 |
| P2e triangle, measured | | | | −5.8e-5 at 19 | | 0.34 |

Findings:

- **Row-sum lumping is disqualifying**, as the literature says. Lumped cubic
  splines are four times worse than lumped bilinear elements at the same
  spacing, lumped quadratics three times worse, and both are second order
  with a large constant. At 19 spans per wavelength the lumped cubic is 280
  times worse than the triangle solver at equal dofs.
- **The sweeps restore the order.** Each sweep adds two orders in `θ`: lumped
  is `O(θ²)`, one sweep `O(θ⁴)`, two sweeps `O(θ⁶)` until the consistent error
  takes over. For quadratics the lumping error and the consistent error have
  opposite signs, and two sweeps land within a factor of a few of zero across
  the resolved band. The sweep error is isotropic to a few percent where the
  consistent cubic's is not.
- **At equal dofs**, two sweeps beat the triangle solver's frequency error
  by 2.4 times for cubics and 11 times for quadratics; three sweeps on
  cubics beat it by 60 times.
- **The stable step stays generous.** Lumped cubics take 1.92 `h/c`, two
  sweeps 1.30, the consistent mass 0.45. The triangle solver's Gershgorin
  bound is 0.34 in units of its dof spacing. The sweeps sit between lumped
  and consistent, as the monotonicity theorem says they must.

## Spike 2: quadrature, from the symbols

`K rule/4` is the least ratio over the band, Nyquist included, of the rule's
stiffness symbol to the four-point rule's: zero means a spurious zero-energy
mode, one means the stiffness is integrated exactly.

| rule | samples per span | p=2: K rule/4 | p=3: K rule/4 | p=3 lumped+2 at 20 ppw | p=3 consistent at 20 ppw |
| --- | --- | --- | --- | --- | --- |
| Gauss 1 | 1 | 0.000 | 0.000 | spurious | spurious |
| Gauss 2 | 4 | 0.833 | 0.833 | −2.6e-5 | −9.1e-6 |
| Gauss 3 | 9 | 1.000 | 0.988 | −1.7e-5 | +2.1e-8 |
| Gauss 4 | 16 | 1.000 | 1.000 | −1.7e-5 | +1.7e-8 |

- **One point per span has spurious modes** at every degree: the sawtooth's
  gradient vanishes at the midpoints.
- **Two points per span has none.** Its stiffness is under-integrated by 17%
  only at the band edge, which lowers the top frequency slightly and leaves
  the resolved band alone. Its mass is under-integrated (the integrand is
  degree 6 for cubics), which caps the attainable accuracy near −1e-5 at 20
  ppw. That is invisible under two sweeps and binding under three or with the
  consistent mass.
- **Three points equals four** to two digits in every dispersion entry.

Verdict: Gauss 2 × 2 for quadratics and for cubics with up to two sweeps,
Gauss 3 × 3 for cubics with three. The optimal and reduced spline rules of
Hiemstra et al. (2017) were not implemented: the `(Q, b)` structure needs a
common point set in any case, two Gauss points per span already sit at the
half-point density those rules reach for cubics, and nothing above says a
better rule is needed.

## Cost per dof

Drift multiply-adds per dof per step relative to the seven-node triangle,
which has about two samples per dof of seven nodes each, counting each sweep
as one more pass over the samples. Steps per simulated second from `dt_max`
at a 96 × 96 patch against the triangle solver's 0.9 × 6.98e-3.

| configuration | work per step | steps relative to P2e | work per simulated second | frequency error at 19 ppw |
| --- | --- | --- | --- | --- |
| p=2, Gauss 2, lumped+2 | 7.7× | 0.31× | 2.4× | +5e-6 (11× better) |
| p=3, Gauss 2, lumped+2 | 13.7× | 0.25× | 3.4× | −3.5e-5 (1.7× better) |
| p=3, Gauss 3, lumped+2 | 30.9× | 0.25× | 7.7× | −2.4e-5 (2.4× better) |
| p=3, Gauss 3, lumped+3 | 41.1× | 0.27× | 11× | −9e-7 (60× better) |

At equal accuracy a fourth-order variant may coarsen: to match the triangle's
−5.8e-5, quadratics with two sweeps need about 54% of the resolution, so 29%
of the dofs and 54% of the steps, about 0.4 times the triangle solver's work.
Cubics with two sweeps and two Gauss points come to about 2.3 times. These
are multiply-add counts on one CPU thread; on the device the same ratios
govern memory traffic, and a lumped+K step is `2 + 2K` dispatches against
the triangle solver's five.

## Spike 1, continued: the clamped box in the time domain

`iga_box_mode` on the 96 × 96 patch, clamped bases, `dt = 0.8` of the
measured stable step, the mode read at `t = 10`. `dt_max` is from a power
iteration on the clamped box; `ratio` is that over the periodic band's. The
symbol column is the periodic prediction of the whole drift, spatial plus
leapfrog temporal.

| basis / rule / mass | dt_max | ratio | phase at t=10 | symbol | steps to t=10 | sim s / wall s |
| --- | --- | --- | --- | --- | --- | --- |
| p=2 Gauss 2 lumped | 1.83e-2 | 0.53 | −1.842 | −1.842 | 681 | 11.5 |
| p=2 Gauss 2 lumped+1 | 1.42e-2 | 0.54 | +0.1513 | +0.1518 | 880 | 5.4 |
| p=2 Gauss 2 lumped+2 | 1.25e-2 | 0.56 | +0.1618 | +0.1621 | 1003 | 3.6 |
| p=2 Gauss 2 lumped+3 | 1.14e-2 | 0.57 | +0.1382 | +0.1384 | 1092 | 2.6 |
| p=2 Gauss 2 consistent | 5.70e-3 | 0.66 | +0.0363 | +0.0363 | 2194 | 0.15 |
| p=2 Gauss 3 lumped+2 | 1.25e-2 | 0.55 | +0.1612 | +0.1615 | 1001 | 1.6 |
| p=3 Gauss 2 lumped | 1.43e-2 | 0.35 | −2.761 | −2.763 | 872 | 5.6 |
| p=3 Gauss 2 lumped+1 | 1.07e-2 | 0.34 | +0.0067 | +0.0078 | 1163 | 2.6 |
| p=3 Gauss 2 lumped+2 | 9.18e-3 | 0.33 | +0.0805 | +0.0812 | 1362 | 1.7 |
| p=3 Gauss 2 lumped+3 | 8.25e-3 | 0.32 | +0.0679 | +0.0683 | 1516 | 1.2 |
| p=3 Gauss 2 consistent | 2.35e-3 | 0.20 | +0.0038 | +0.0038 | 5323 | 0.03 |
| p=3 Gauss 3 lumped+2 | 9.61e-3 | 0.34 | +0.0909 | +0.0916 | 1301 | 0.75 |
| p=3 Gauss 3 lumped+3 | 8.78e-3 | 0.34 | +0.0793 | +0.0796 | 1424 | 0.54 |
| p=3 Gauss 3 consistent | 4.34e-3 | 0.45 | +0.0195 | +0.0195 | 2880 | 0.03 |
| P2e, parent h 0.08, at 0.9 dt_max | 6.98e-3 | | ≈ +0.055 | | 1591 | ≈ 17.6 |

Amplitude errors are below 2e-6 everywhere and the staggered energy drifts by
less than 1e-13: the scheme is symplectic as built. Three things stand out.

**The symbols predict the box.** Every measured drift is within 1.2e-3 rad of
the periodic prediction, most within 3e-4, over a total phase of 157 rad. The
walls move the mode's frequency by less than 8e-6 relative, so the symbol
tables above are the spatial accuracy of the clamped box too, and the
sweeps lose nothing at the walls.

**At the stable step the leapfrog's temporal error dominates.** The quadratic
patch with two sweeps drifts +0.16 rad at `t = 10`, of which the spatial part
is about +1e-3 rad: the rest is the second-order leapfrog at `ω dt = 0.16`.
The triangle solver at its own operating point is in the same position, with
about +0.064 rad temporal against −0.009 spatial, only less so because its
step is smaller. A spline patch's larger stable step is therefore a speed
lever, not an accuracy lever, until the time integrator catches up: either
run at a fraction of the step, which costs steps, or compose a fourth-order
symplectic integrator from the kick-drift stages, which costs three stages a
step but turns `(ω dt)²/24` into a fourth power. That is the same lever for
both solvers and belongs on the spike list as its own item.

**The clamped walls add an outlier.** The box's stable step is 0.53 to 0.57
of the periodic band's for quadratics and 0.32 to 0.35 for cubics under every
lumped treatment, and 0.20 to 0.66 with the consistent mass. The culprit is
the open knot vector's corner function, whose support is one span and whose
lumped mass is `h²/(p+1)²`: it rings at about `(p+1)` times the interior's top
frequency in the corner. The same space has another basis, uniform knots
running `p` spans past each wall with no repeated knots, whose end functions
are truncated translates; its consistent spectrum is identical (the test
`the_unclamped_basis_spans_the_clamped_space`) and only its lumped diagonal
differs. Measured on the 96 × 96 patch with the two-point rule:

| basis / mass | clamped dt_max | ratio | unclamped dt_max | ratio | steps saved |
| --- | --- | --- | --- | --- | --- |
| p=2 lumped | 1.83e-2 | 0.53 | 2.85e-2 | 0.82 | 1.55× |
| p=2 lumped+1 | 1.42e-2 | 0.54 | 2.12e-2 | 0.80 | 1.49× |
| p=2 lumped+2 | 1.25e-2 | 0.56 | 1.79e-2 | 0.80 | 1.44× |
| p=2 lumped+3 | 1.14e-2 | 0.57 | 1.60e-2 | 0.80 | 1.40× |
| p=3 lumped | 1.43e-2 | 0.35 | 3.27e-2 | ≤ 0.79 | 2.28× |
| p=3 lumped+1 | 1.07e-2 | 0.34 | 2.44e-2 | ≤ 0.76 | 2.27× |
| p=3 lumped+2 | 9.18e-3 | 0.33 | 2.09e-2 | ≤ 0.74 | 2.27× |
| p=3 lumped+3 | 8.25e-3 | 0.32 | 1.87e-2 | ≤ 0.73 | 2.27× |

The cubic unclamped power iterations had not converged after 1,500 passes,
so their eigenvalue is a lower bound and the step an upper one; the runs at
0.8 of it were stable with the energy held to 1e-13, which puts the true
ratio between 0.6 and the figure shown. The unclamped basis takes back most
of the outlier, 1.4 to 1.5 times the step for quadratics and 2.3 for cubics,
and changes nothing else: its spatial drifts match the clamped ones to
2e-4 rad. The remaining 0.2 is the truncated end functions, which deflation
(Voet, Sande, Buffa 2024) or a boundary-adapted lumping would address; not
pursued here.

### Convergence of the spatial part

The spatial drift at `t = 10`, read by inverting the leapfrog relation, for
two sweeps and the two-point rule at three resolutions. The symbol column is
the periodic prediction.

| basis | spans per λ | DOFs | clamped | unclamped | symbol |
| --- | --- | --- | --- | --- | --- |
| p=2 | 9.2 | 2,304 | −6.7e-2 | −6.0e-2 | −5.6e-2 |
| p=2 | 18.8 | 9,216 | +8.3e-4 | +1.0e-3 | +1.1e-3 |
| p=2 | 38.0 | 36,864 | +1.30e-4 | +1.35e-4 | +1.39e-4 |
| p=3 | 9.0 | 2,304 | −3.28e-1 | −3.19e-1 | −3.04e-1 |
| p=3 | 18.6 | 9,216 | −6.7e-3 | −6.3e-3 | −6.0e-3 |
| p=3 | 37.8 | 36,864 | −1.90e-4 | −1.80e-4 | −1.72e-4 |
| P2e | 19 (parent 5) | 9,215 | −9.1e-3 | | |
| P2e | 38 (parent 10) | 37,395 | about −5e-4 | | |

The cubic's error falls 49 times from 9 to 19 spans per wavelength and 35
times from 19 to 38: order five to six, the `O(θ⁶)` remainder of two sweeps,
with the consistent cubic's own error far below it. The quadratic changes
sign between 9 and 19 spans: its negative sweep remainder and the positive
consistent-quadratic error cancel near 15 spans per wavelength, after which
the consistent fourth-order error is what remains (the 38-span figure is
within 5% of the consistent quadratic's symbol). The walls add a residual
that shrinks from 8% of the drift at 9 spans to 3% at 38; it is of the same
order as the drift itself, so it does not change the picture. At 9,216 dofs
the triangle solver's −9.1e-3 rad sits between the two spline variants'
figures at 2,304 dofs and their figures at 9,216: it is matched by a
quadratic patch of about 2,900 dofs or a cubic one of about 7,800.

### Cost at equal dofs and at equal spatial accuracy

Per simulated second on one CPU thread at each solver's operating point, 0.9
of the triangle's Gershgorin step against 0.8 of the patch's measured step
scaled to 0.9; 9,216 against 9,215 dofs. The clamped throughputs are the
uncontended matrix figures; the unclamped ones apply the measured step
gains, since those runs shared the machine with the test suite.

| configuration | spatial phase at t=10 | against P2e's −9.1e-3 | sim s / wall s, clamped | unclamped | against P2e's 17.6 | at equal spatial accuracy |
| --- | --- | --- | --- | --- | --- | --- |
| p=2 Gauss 2 lumped+2 | +1.0e-3 | 9× better | 4.0 | 5.8 | 3.0× slower | ≈ 0.5× P2e's time |
| p=3 Gauss 2 lumped+2 | −6.3e-3 | 1.4× better | 1.9 | 4.2 | 4.2× slower | ≈ 3.3× |
| p=3 Gauss 3 lumped+2 | −3.8e-3 | 2.4× better | 0.84 | 1.9 | 9.3× slower | ≈ 6× |
| p=3 Gauss 3 lumped+3 | −1.4e-4 | 60× better | 0.61 | 1.4 | 13× slower | ≈ 2.6× |

The equal-accuracy column coarsens each variant along its own error law
until its spatial error meets the triangle's, scaling dofs by the square and
steps by the first power of the factor. The measured throughputs agree with
the multiply-add model above to about 10%, so the model can be trusted for
the device layout as far as arithmetic and traffic go.

## Gate 1 verdict

**Passed, on quadratic splines with two Jacobi sweeps, the Gauss 2 × 2 rule
and the unclamped basis; cubics pass on accuracy and lag on cost.** Row-sum
lumping alone is out, by a factor of 280 against the triangle at equal dofs.
The sweep variant keeps every constraint of the triangle solver's
architecture: one point set, divergence the transpose of gradient, a
symmetric positive-definite matrix-free field map applied through the same
sample tables, `b` at the samples, the density read where it stands, and
energy conserved to roundoff. At equal dofs it is nine times more accurate in
space than the seven-node triangle and three times slower per simulated
second on the CPU; at equal spatial accuracy it takes about half the
triangle's time. Two levers remain untouched: the last 20% of the corner
outlier, and the time integrator, which both solvers need before any of this
spatial accuracy reaches the screen. Spike T below measures the integrator:
fourth order in time for 13% of the throughput, and the runs become
spatial-error-limited.

The spikes continue: 3 (parameterization), 4 (materials), 5 (edits) and 6
(cost on a curved patch).

## Spike T: a fourth-order time integrator

The box runs showed the leapfrog's temporal error dominating at the stable
step for both solvers. The remedy measured here is the modified-equation
Störmer scheme (Dablain 1986). The semi-discrete system is `Q̈ = −K P Q`;
leapfrog's two-step form is `Qⁿ⁺¹ − 2Qⁿ + Qⁿ⁻¹ = −dt² K P Qⁿ`, and the exact
propagator's is `2(cos(dt√(KP)) − I) Qⁿ = (−dt² KP + dt⁴ (KP)²/12 − …) Qⁿ`.
Keeping the next term gives a Störmer step under the modified stiffness
`K̃ = K (I − dt²/12 · P K)`, fourth order in time. In the kick-drift form
nothing moves but the field `b` drifts on:

```text
u  = P Q
ũ  = P (Q − dt²/12 · K u)        K u = Gᵀ W (G u), one more gradient, divergence and field pass
b  += dt G ũ                      the drift
Q  −= dt Gᵀ W b                   the kick, unchanged
```

So a step costs two gradient, two divergence and two field passes against
leapfrog's one each, through the same tables, and the sweeps double with
the field passes. For a mode of discrete frequency `ω` the amplification
has `cos(ω_num dt) = 1 − x/2 + x²/24`, `x = (ω dt)²`, so the scheme is
stable, and `K̃` positive definite, while `x < 12`: the stable step grows by
√3. The relative frequency error is `(ω dt)⁴/720` against leapfrog's
`(ω dt)²/24`. The scheme stays symplectic, since it is Störmer under a
symmetric positive-definite operator, and conserves
`½ ΔQ · P ΔQ / dt² + ½ (G uⁿ⁺¹) · W (G ũⁿ)` exactly
(`PatchStepper::conserved_energy`, held to 1e-13 in the tests). On a linear
basis the stepped phase matches the formula to 1e-9.

For a nonlinear constitutive law `K` is not fixed, and the dt⁴ term would
need the law's tangent at the current state; Chin's force-gradient
integrators are the systematic route. For a time-driven mass `P` moves with
the drive and the modified field reads it where it stands. Neither was
measured.

### Measured

`iga_box_mode` on the unclamped 96 × 96 patch, two sweeps, Gauss 2 × 2, each
integrator at 0.8 of its own stable step; the spatial part is read by
inverting each integrator's own relation, the temporal part is the rest.

| basis | integrator | dt | steps to t=10 | phase at t=10 | temporal part | spatial part | sim s / wall s |
| --- | --- | --- | --- | --- | --- | --- | --- |
| p=2 | leapfrog | 1.43e-2 | 697 | +0.335 | +0.334 | +1.0e-3 | 5.2 |
| p=2 | fourth order | 2.49e-2 | 402 | −4.2e-3 | −5.2e-3 | +1.0e-3 | 4.6 |
| p=2 | fourth order at 0.4 | 1.24e-2 | 805 | +6.8e-4 | −3.2e-4 | +1.0e-3 | 2.3 |
| p=2 | fourth order at 0.2 | 6.21e-3 | 1,610 | +9.8e-4 | −2.0e-5 | +1.0e-3 | 1.1 |
| p=2 | leapfrog at 0.46 | 8.25e-3 | 1,212 | +0.111 | +0.110 | +1.0e-3 | 3.0 |
| p=3 | leapfrog | 1.67e-2 | 599 | +0.446 | +0.452 | −6.3e-3 | 3.7 |
| p=3 | fourth order | 2.89e-2 | 346 | −1.58e-2 | −9.5e-3 | −6.3e-3 | 3.2 |
| p=2, 192² | leapfrog | 7.10e-3 | 1,409 | +8.16e-2 | +8.15e-2 | +1.35e-4 | 0.63 |
| p=2, 192² | fourth order | 1.23e-2 | 813 | −1.70e-4 | −3.05e-4 | +1.35e-4 | 0.55 |

- **The temporal error falls 50 to 60 times** at the stable step, from
  +0.33 to −0.005 rad on quadratics and from +0.45 to −0.0095 on cubics, and
  the runs are now limited by their spatial error. Halving the step halves
  the temporal part sixteen times over, twice: fourth order as built.
- **It costs 13% of the throughput**, two passes a step against √3 fewer
  steps, `2/√3 = 1.15`; leapfrog at the same cost (fraction 0.46) is still
  twenty times worse in time. The symbol predicts every row to 3%.
- **Energy is conserved** to 1e-13 under both integrators at 95% of their
  stable steps (the test), and no amplitude is lost.
- **The same lever fits the triangle solver.** At 0.9 of its Gershgorin step
  times √3, the seven-node triangle's temporal error at `t = 10` would fall
  from +0.064 to about 2e-4 rad for the same 15% of throughput, leaving its
  spatial −9.1e-3 as the whole error. The extra pass is one more gather,
  scatter and field recovery before the drift, through the existing tables.

## Spike 3: a single untrimmed patch over a curved domain

The patch now carries a geometry map: affine, a bicubic B-spline surface
with a control net, or a bilinearly blended Coons patch between four sides
(straight segments, exact elliptic arcs, or pieces of a closed gallery
spline). The Jacobian is evaluated at every sample; its determinant goes
into the weight and its inverse transpose onto the gradients, so `b` and the
gradients are in physical components and nothing else changes. Checks: the
flat surface and a Coons patch of straight sides reproduce the affine box to
1e-12; a Coons disk has the disk's area to 2e-4 and keeps the transpose
structure. `iga_curved_patch` measures each geometry at 62 × 62 spans
(4,096 dofs), quadratics, Gauss 2 × 2, two sweeps.

| geometry | det J min / median / max | lumped min / median | dt_max over an equal-area box's, clamped | unclamped |
| --- | --- | --- | --- | --- |
| affine 2 × 2 box | 1 / 1 / 1 | 7.2e-6 / 2.6e-4 | 1.00 | 1.00 |
| bulged box, right side pushed out 0.4 | 1.00 / 1.07 / 1.15 | 7.2e-6 / 2.8e-4 | 0.82 | 0.87 |
| exact unit disk, Coons, corners on the diagonals | 0.060 / 3.2 / 4.7 | 5.1e-7 / 8.2e-4 | 0.017 | 0.021 |
| exact 2:1 ellipse, Coons | 0.030 / 1.6 / 2.3 | 2.6e-7 / 4.1e-4 | 0.016 | 0.019 |
| gallery `rounded` circle (8 controls), Coons | 0.050 / 2.6 / 3.8 | 4.3e-7 / 6.6e-4 | 0.018 | 0.021 |
| gallery-like blob (smoothed hexagon), Coons | 0.032 / 3.5 / 4.4 | 2.7e-7 / 9.0e-4 | 0.009 | 0.010 |

On the exact disk two Neumann modes with known frequencies, `J_m(kr) cos mθ`
at a zero of `J_m'`, stepped by the fourth-order integrator at 0.8 of the
stable step to `t = 10`:

| mode | kR | spans per wavelength | relative frequency error, clamped | unclamped |
| --- | --- | --- | --- | --- |
| J1, first | 1.8412 | 106 | −3.4e-8 | −1.1e-7 |
| J3, third | 11.3459 | 17.2 | −2.7e-6 | −2.9e-6 |

**The accuracy is untouched by the curved map**: the J3 mode at 17 spans
per wavelength lands within 3e-6, where the box mode at 19 spans sits at
7e-6. **The stable step is destroyed**: one to two percent of an equal-area
box's on every smooth domain, fifty to a hundred times too small. A
deformed box with its four corners kept is fine at 0.82 to 0.87.

The cause is the map, not the domain. A single bijective patch of a smooth
domain must put the square's four corners on a boundary without corners, so
the two parametric tangents turn parallel there and the Jacobian vanishes:
the functions at the corners have almost no mass (5e-7 against a median of
8e-4) and ring at an outlier frequency. Two repairs that keep the
architecture were measured on the unclamped disk; both buy the step back
only in proportion to what they take from the accuracy.

| repair | dofs touched | dt_max over the box's | J3 error |
| --- | --- | --- | --- |
| none | 0 | 0.021 | −2.9e-6 |
| condense the 2 × 2 functions at each corner into one | 12 | 0.073 | +2.5e-5 |
| condense 3 × 3 | 32 | 0.159 | +1.4e-4 |
| condense 4 × 4 | 60 | 0.214 | +3.4e-4 |
| condense 6 × 6 | 140 | 0.314 | +1.5e-3 |
| cap each function's `K_ii / D_i` at 16 × the median, by added mass | 96 | 0.28 | −2.0e-3 |
| cap at 8 × | 172 | 0.39 | −7.4e-3 |
| cap at 4 × | 320 | 0.55 | +2.2e-2 |
| cap at 2 × | 684 | 0.78 | +9.2e-3 (J1: −6.1e-2) |

Condensation merges the functions around a corner into one (their
parametric neighbourhood maps to a small physical region); selective mass
scaling adds mass to each function whose own frequency bound exceeds a cap,
to the lumped diagonal and the consistent mass alike, so the sweeps still
converge. Neither is local enough: the region where the Jacobian is small
reaches many spans into the patch along the corner, so the step recovers
linearly with the reach while the mode, which is large at the ±45° corners,
loses accuracy at the same rate. Deflation (Voet, Sande, Buffa 2024) would
face the same count of outlier modes, since the elevated frequencies are
not a handful.

**Gate 3 verdict: failed for a smooth closed boundary as one untrimmed
patch; passed for a deformed box.** This matters less for the product than
it sounds, because the scene's own domain is a rectangle: the natural
single patch is the scene's box, and the curves inside it are materials,
which spike 4 is about. A curved outer wall, and the plan's wish to revisit
second-order outgoing conditions on curved spans with the patch's exact
curvature, would need a multipatch layout (a square surrounded by four
curved quadrilaterals is the standard disk), which is where that item now
sits.

## Spike 4: materials inside the box patch

The patch now samples a density and a stiffness at every quadrature point
(`with_material`): the density weights the mass and the lumped diagonal,
the stiffness weights `b` in the kick and in the energy, so `K = Gᵀ W κ G`
stays symmetric and the sweeps still converge. A basis may carry a C⁰ knot
(an interior knot repeated to the degree, `with_c0_knot`), which adds one
function per direction and lets the field kink along that line.
`iga_slab_reflection` sends a Gaussian pulse at unit speed against a planar
step at normal incidence and reads the energy left of the step at `t = 1.2`
against the initial energy, which Fresnel puts at `R = 1/9` for an impedance
of two either way: a density step 1 → 4 at unit stiffness, across which the
field stays C¹, and a stiffness step 1 → ¼ at unit density, across which it
kinks. Quadratics, Gauss 2 × 2, two sweeps, unclamped, fourth-order
integrator at 0.8 of its step, pulse width σ = 0.1.

Relative error of `R` by resolution, spans per σ in the header:

| variant | 3.1 | 6.3 | 12.7 | order |
| --- | --- | --- | --- | --- |
| density step on a knot line, C¹ basis | +2.0e-2 | −7.7e-5 | +1.0e-5 | at the floor |
| density step mid-span (immersed), C¹ basis | +3.4e-2 | +7.4e-3 | +1.9e-3 | 2 |
| density step on a knot line, C⁰ knot there | +9.9e-3 | −1.2e-3 | −4.9e-5 | ≥ 4 |
| stiffness step on a knot line, C¹ basis | +6.9e-2 | +6.4e-3 | +1.5e-3 | 2 |
| stiffness step mid-span (immersed), C¹ basis | +5.3e-2 | +4.2e-3 | +9.5e-4 | 2 |
| stiffness step on a knot line, C⁰ knot there | +3.9e-2 | +1.3e-3 | +6.2e-5 | ≈ 4 |

The total energy is accounted to 1e-6 at the finest resolution, which is
the floor the first and last rows sit at. A second series with σ = 0.2 and
a quarter of the step gives the same magnitudes at 6 and 13 spans per σ
(immersed density 7.2e-3 and 1.6e-3, C¹ stiffness 5.8e-3 and 1.1e-3, C⁰
stiffness 4.9e-4 and then the floor), with an unexplained floor of −2e-4 to
−5e-4 in every row, probably the reflected pulse's tail at the wall under
the half-step readout; not pursued.

- **An interface on a knot line with the right continuity is high order.**
  A density jump leaves the field C¹, which the plain basis holds, and the
  error is at the floor from 6 spans per σ. A stiffness jump kinks the
  field, and with a C⁰ knot on the line the error falls about sixteenfold
  per halving.
- **Anything else is second order.** An interface between knot lines, or a
  kink the basis cannot make, costs about 1% of `R` at 6 spans per σ and 0.2%
  at 13, and halving the span halves it four times. On a knot line without
  the C⁰ knot a stiffness jump is as bad as an immersed one.
- **Which jump is which depends on the skin.** TE puts ε in the mass, so an
  index step is a density step and the plain basis is right; TM puts ε in
  the stiffness and needs the C⁰ knot; Mechanical has both. The triangle
  solver conforms to every curve, so it has none of this.

**Gate 4 verdict: passed for material regions bounded by knot lines, with
C⁰ knots where the stiffness jumps; a free curve inside the patch is an
immersed, second-order interface and a downgrade from the conforming
triangle.** An IGA scene is therefore its own kind of document: a box
patch, deformed through its control net where the user wants curvature,
whose material regions are parametric rectangles; the geometry map curves
their boundaries while they stay knot lines. The present scene model, free
curves in a box, maps onto a single patch only by immersion. Trimming, local
refinement or an enriched basis at the interface are the ways past that,
none of them a single untrimmed patch.

## Spike 5: edits without a mesher, and the deformed box's cost

On the deformed box (right side bulged by 0.3 through a bicubic net on 16 ×
16 spans, Jacobian 1.00 to 1.73), quadratics on 62 × 62 spans, Gauss 2 × 2,
two sweeps, unclamped, the fourth-order integrator at 0.8 of its step
(`iga_patch_edits`). A handoff carries the field's coefficients, which move
with the geometry, and the flux at the field's own instant
(`PatchStepper::centered_flux`), which the new stepper staggers itself,
whatever its step.

- **A control-point move rebuilds 3% of the samples and touches 4% of the
  dofs** (`with_moved_control`: the cubic net point's 4 × 4 spans), and left
  the stable step unchanged. The flux is carried by its covariant
  components, `Jᵀ b` kept (`carry_flux`); the field at a sample moves with
  the geometry. With the pulse 1.9 away from the moved point the energy
  changed by 0 and the field at `t = 1` differed from a run on the edited
  box from the start by 4e-10: no artifact. Under the pulse the energy
  changes linearly with the move, 1.7e-3, 3.5e-3, 6.9e-3 for 0.004, 0.008,
  0.016, as stretching the medium should.
- **Knot refinement carries the state exactly.** The field is in the fine
  space and is projected onto it to 1e-9. The flux is a gradient to 6e-15
  throughout a run (`flux_potential`: `b` evolves only by gradients, so
  `K w = Gᵀ W κ b` recovers its potential), and the potential is carried as
  a field and differentiated on the fine patch (`refine_flux`), so the flux
  is exact too; the energy as the fine quadrature reads it differs by 4e-6.
  Refining 62 × 62 to 124 × 124 at `t = 0.29` and running to `t = 1` lands
  3.9e-3 from a fine run from the start, where the coarse run lands 6e-4:
  the handoff's own cost is the fourth-order scheme's step-dependent
  modified field, an `O(dt²)` inconsistency when the step changes, not the
  transfer.
- **The deformed box costs its step only.** Flat and deformed boxes of
  4,096 dofs step at 2.8 and 3.0 ms a step; the deformed one's stable step
  is 0.65 of the flat one's for this strong bulge (0.82 to 0.87 for spike
  3's milder one), so it runs at 8.2 against 14.0 simulated seconds per
  wall second. That folds spike 6 in: the cost tables of spike 1 scale by
  the Jacobian's effect on the step and by nothing else.

**Gate 5 verdict: passed.** Edits and refinement are local, exact and
artifact-free, with the one caveat that a handoff that changes the step
under the fourth-order integrator carries an `O(dt²)` inconsistency.

## What the spikes do not settle

- A field-dependent mass (Kerr or saturable in the mass row) makes the field
  recovery `u = P(Q)` nonlinear. The triangle solver solves a per-node map by
  Newton; with sweeps the map is no longer diagonal and the sweep would have
  to wrap it. A design question for the integration, not a numerical one.
- A time-driven mass (pump) changes the density at every sample each stage.
  The sweep form applies `M` through the samples with the density in force,
  so it follows the drive without recomputation; a precomputed banded `S`
  would not.
- The mass application needs the basis values at each sample beside the
  curls: another `(p+1)²` floats per sample, or a recomputation from the 1D
  tables that the sum-factorized layout allows.
- Reflecting or outgoing obstacles inside the patch: a free curve as a
  material is immersed (spike 4); a free curve as a wall has no single-patch
  treatment at all without trimming.
- A comparison of the slab reflection against the triangle solver on the
  same step; the Fresnel value stood in for it.

## References

- Cottrell, Reali, Bazilevs, Hughes (2006), *Isogeometric analysis of
  structural vibrations*: row-sum lumping of splines is second order at any
  degree.
- Voet, Sande, Buffa (2023), [A mathematical theory for mass lumping and its
  generalization with applications to isogeometric
  analysis](https://arxiv.org/abs/2212.03614): lumped eigenvalues lie below
  consistent ones; a monotone family of banded lumped masses between them.
- Voet, Sande, Buffa (2024), [Mass lumping and outlier removal strategies for
  complex geometries in isogeometric analysis](https://arxiv.org/abs/2402.14956):
  deflation of boundary outliers, trimmed and multipatch geometry; the plan's
  reference.
- Anitescu, Nguyen, Rabczuk, Zhuang (2019), *Isogeometric analysis for
  explicit elastodynamics using a dual-basis diagonal mass formulation*,
  CMAME 346, and Nguyen, Hiemstra, Eisenträger, Schillinger (2023), [Towards
  higher-order accurate mass lumping in explicit isogeometric analysis for
  structural dynamics](https://arxiv.org/abs/2310.13379): approximate duals
  as test functions; equivalently a Galerkin method with the SPD mass `S⁻¹`.
- Chan, Evans (2017), [Multi-patch discontinuous Galerkin isogeometric
  analysis for wave propagation](https://arxiv.org/abs/1708.02972):
  weight-adjusted mass inverses on curved patches with the tensor structure
  kept, and DG coupling between patches; relevant to spikes 3 and to
  multipatch later.
- Hiemstra, Calabrò, Schillinger, Hughes (2017), *Optimal and reduced
  quadrature rules for tensor product and hierarchically refined splines in
  isogeometric analysis*, CMAME 316.
