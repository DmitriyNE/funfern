# IGA feasibility spikes — 3 October 2026

**Status: spikes 1 and 2 measured; gate 1 passed, see the verdict.** Spikes 3
to 6 are planned, not started. Everything here is CPU f64 in
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
- `BoxPatch`: an affine tensor-product patch over the 2 × 2 box with
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
spatial accuracy reaches the screen.

The spikes continue: 3 (parameterization), 4 (materials), 5 (edits) and 6
(cost on a curved patch), plus the time integrator as a new item.

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
- Curved patches, outliers from a non-affine map, and the Jacobian's effect
  on the lumped diagonal are spike 3.

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
