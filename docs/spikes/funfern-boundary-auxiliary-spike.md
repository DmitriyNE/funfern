# Passive boundary auxiliary formulation — 19 September 2026

**Adopted for implementation:** the user accepted this boundary law's stated
accuracy/nonlocal-cost tradeoff. Follow the [consolidated plan](funfern-material-laws-plan.md)
for enablement gates and history/performance work. Recommendations below retain
the investigation's chronology; the corrected timestep in the next paragraph
is mandatory, not optional.

**Scattering follow-up:** the [single-boundary study](funfern-boundary-scattering-spike.md)
now confirms FEM convergence to the predicted planar reflection, including
grazing improvement over first order. It also finds a fixed-CFL accuracy defect
in this report's separate boundary/interior split. The force-coupled midpoint
kicks derived there supersede that timestep recommendation; this report and its
64-check artifact remain the historical auxiliary/passivity investigation.
The auxiliary equations themselves are unchanged.

## Result and recommendation

The bounded follow-up produced a **passive, three-state-per-boundary-mode
candidate** that retains the legacy second-order tangential expansion and
fourth-order small-angle reflection scaling. It removes the earlier linear-loss
instability in the tested continuous and fully discrete systems. No prohibition
on nonlinear materials is needed for the continuous energy argument. A Kerr
boundary-only nonlinear update also satisfies the discrete energy balance.

This is a newly derived rational approximation, **not a claimed implementation
of CRBC, exact DtN, or a universally superior absorber**. It trades some angular
accuracy for unconditional semidiscrete passivity and introduces a nonlocal
boundary operator. Its finite-box packet results are slightly worse than first
order in these fixtures, though within the declared tolerances. Curved-geometry
energy stability is demonstrated on an annulus, not exact curved radiation or
the original ignored topology reproducer.

Recommend carrying this formulation into review as the concrete correctness
reference for a retained second-order option. Do not port the old double
integrator unchanged, silently downgrade scenes to first order, or claim the
performance question is settled. Measure the production-intended solver core,
including this boundary work, before production cutover. A cheaper CRBC/local
realization or a more accurate DtN approximation remains possible behind the
same power-conjugate boundary interface.

Artifacts: [criteria](../../experiments/material-laws-spike/boundary-README.md),
[implementation](../../experiments/material-laws-spike/boundary.py),
[captured results](../../experiments/material-laws-spike/boundary-results.json).
Run `bash experiments/material-laws-spike/run-boundary.sh` from the repo root.
It compiles the pinned clean core in a temporary directory and uses its actual
enriched FEM operators, without modifying production or the unfinished NL work.
The recorded run has **64 checks passing, none failing**. Earlier material-spike
failures remain in their original results; they have not been reclassified.

## 1. Physical exterior response

Take a homogeneous isotropic half-space outside a planar boundary, initially
unexcited. Let `u` be the scalar physical field, `j` its outward power-conjugate
flux, and let the primary/complementary linear coefficients be `α,β>0`.
Set `c=1/sqrt(αβ)` and `Y0=sqrt(α/β)`. Laplace transform in time and Fourier
transform along the boundary, with tangential wavenumber `k` and `Re(s)>0`.
The decaying exterior solution has normal exponent

```text
κ = sqrt(k² + s²/c²),        ĵ = [κ/(βs)] û.
Yexact(s,k) = Y0 sqrt(1 + (ck/s)²).
```

The square-root branch is chosen by spatial decay/analytic continuation from
positive real s, not independently at each real frequency. With exterior
primary/complementary flux-decay rates `γp,γc`, the corresponding response is

```text
κ = sqrt(k² + αβ(s+γp)(s+γc)),
Yexact = κ/[β(s+γc)].
```

**Exterior loss is not interior loss.** This follow-up approximates a lossless,
linear exterior; interior loss may meet it as a material interface. The tested
`γc=4` bulk case therefore establishes stable coupling, not reflectionless
matching to an exterior that also has `γc=4`. Exterior dispersion, nonlinearity,
modulation, anisotropy or incoming fields need additional response contracts.

The uniform-half-space radiation setup and auxiliary rational approach are
consistent with [Hagstrom–Lagrone's Maxwell CRBC work](https://journals.riverpublishers.com/index.php/ACES/article/view/7485).
That paper's implementation uses different auxiliary fields and explicit corner
relations; its stability/accuracy results are not transferred to this candidate.
For future shape-specific exact kernels, see
[Alpert–Greengard–Hagstrom](https://math.nist.gov/~BAlpert/nrbc2.pdf).

## 2. Why the old truncation fails, and the proposed replacement

Normalize by Y0 and write `a=c|k|`. The old response truncates

```text
sqrt(1+a²/s²) = 1 + a²/(2s²) - a⁴/(8s⁴) + ...
```

after the quadratic term. On the imaginary axis its real part becomes
`1-a²/(2ω²)`, which is negative at low enough frequency. It is not a passive
load on arbitrary interior states. Banning nonlinear coefficients does not
change this fact: the original counterexample used constant linear laws.

A first two-state candidate used `1+d²/[s(s+d)]`, `d=a/sqrt(2)`. Its simple
energy proof and numerical checks passed, but its cubic angular error would
downgrade the old small-angle order. It was refined rather than adopted.

The implemented candidate is

```text
d = sqrt(7/8) a
Y(s) = 1 + (8/7)d²/[s(s+d)] - (1/7)4d²/[s(s+2d)]
     = 1 + (6d/7)/s - (8d/7)/(s+d) + (2d/7)/(s+2d).
```

Its expansion is `1+a²/(2s²)-7a⁴/(8s⁴)+O((a/s)^5)`: the cubic term cancels.
Thus the quadratic angular correction matches, and reflection at small angle
is O(θ⁴), as for the old second-order condition. **The leading reflection error
coefficient is not the same**: it is six times the old truncation's coefficient
in this asymptotic limit. This is an explicit accuracy tradeoff, not a drop-in
claim that every old reflection measurement improves.

For real nonzero ω,

```text
Re Y(iω) = ω² [ω²+(31/7)d²] / [(ω²+d²)(ω²+4d²)] >= 0.
```

The zero pole has positive residue `6d/7`, the other poles are `-d,-2d`.
The storage realization below proves passivity directly, including the static
state. At `d=0`, the response is exactly first-order `Y=1`; unused auxiliary
states can be omitted. Do not approximate the zero mode by a small arbitrary d.

## 3. Auxiliary equations and energy

For each normalized mode input `w`, define three real states `x=(x0,x1,x2)`:

```text
ẋ = -d diag(0,1,2) x + sqrt(d) [1,1,1]ᵀ w
j = w + sqrt(d) [6/7,-8/7,2/7] x.
```

Let `b=sqrt(31/7)`, `ℓ=(0,1-b,2b-4)`. Define symmetric H by

```text
H00 = 6/7; H01 = H02 = 0
Hij = 2 ℓi ℓj/(i+j), for i,j in {1,2}.
```

Its eigenvalues are approximately `0.00238490, 0.857143, 1.239158`: H is
positive definite. With `EΓ=xᵀHx/2`, direct substitution gives

```text
ĖΓ = w j - (w + sqrt(d) ℓᵀx)².
```

The negative residue in the transfer function is therefore **not** a negative
energy mode. The prototype stores `z=H^(1/2)x`, so its auxiliary energy is simply
`zᵀz/2` and does not depend on d. This also avoids carrying H's conditioning in
the norm used for energy diagnostics. f32 implementation still needs tests.

For the FEM, extract positive trace damping D and symmetric positive-semidefinite
auxiliary stiffness A. With trace restriction R, diagonalize

```text
D^(-1/2) A D^(-1/2) = V diag(λ) Vᵀ
d_j = sqrt(7λ_j/4)
T = Vᵀ D^(1/2) R;       w = T u;       Q̇boundary = -Tᵀ j.
```

For the unit flat exterior, `λ=a²/2`; the high-frequency physical response is
`D+A/s²+...`, matching the old tangential correction. Most importantly,
`uᵀQ̇boundary=-wᵀj` exactly. Therefore, at fixed passive material laws,

```text
d/dt (Ebulk + Σ EΓ) = -bulk_loss - Σ(w+sqrt(d)ℓᵀx)² <= 0.
```

This argument uses `u=∂Ebulk/∂Q`, not `u=M⁻¹Q`. It does **not** require linear
interior constitutive maps. Temporal material work and sources must be added
to the energy ledger rather than hidden under a passivity claim.

Spatial modal diagonalization is a real nonlocal operation, not just a test
notation that magically becomes an element-local kernel. At square corners we
use the existing assembled closed trace graph. It preserves power/energy but
is not the exact exterior corner DtN. Prepare disconnected trace components
separately; no physical coupling across independent boundaries is intended.

## 4. Actual time stepping, including nonlinear boundary nodes

The linear fixture uses symmetric splitting:

```text
half exact constant-rate loss
half implicit-midpoint boundary step
full explicit interior KDK
half implicit-midpoint boundary step
half exact constant-rate loss.
```

For a boundary subsystem generator G in normalized Q and z coordinates,
`G+Gᵀ<=0`. Its midpoint map `(I-hG/2)^(-1)(I+hG/2)` is contractive in the
stored energy for any h. The implicit solve is **only on boundary state**, not
the whole interior. KDK is still CFL-limited; an implicit boundary step does
not authorize an arbitrary global timestep.

For fixed linear interior coefficients, KDK conserves a positive modified
energy below its CFL limit:

```text
Etilde = 1/2 QᵀM⁻¹Q + 1/2 bᵀWb
         - Δt²/8 (CᵀWb)ᵀM⁻¹(CᵀWb) + 1/2 zᵀz.
```

Boundary substeps leave b fixed and therefore also dissipate this modified
energy. Uniform loss contracts the respective quadratic blocks. Spatially
varying complementary loss does not commute with that modified norm, so its
fully discrete stability is numerically checked here, not asserted as a general
theorem. Nonlinear/time-dependent full-system stepping requires its own tests.

### Smaller linear boundary solve

Eliminating auxiliary midpoints leaves an Nb-by-Nb positive-definite system
for the primary trace, rather than the prototype's full 4Nb-by-4Nb map.
In modal coordinates let `Yh=Y(2/h)>0` and `jhistory` be the old-memory term:

```text
[MΓ+(h/2)TᵀYhT] u1
  = [MΓ-(h/2)TᵀYhT] u0 - h Tᵀ jhistory.
```

Recover each mode's auxiliary midpoint with a 3×3 solve. The trace matrix can
be factored when the generation, boundary primary mass or timestep changes.
Reference parity against the full midpoint map is below `3.4e-14` over tested
step sizes 0.001, 0.1, 1 and 10. This derivation is a reusable implementation
route, not a proposal for a live CPU field solve.

### Nonlinear primary trace

Ordinary linear midpoint using `Q/m` is wrong when boundary nodes have nonlinear
primary laws. Use a discrete energy gradient

```text
ubar_i = [Ei(Q1_i)-Ei(Q0_i)]/[Q1_i-Q0_i]
```

with its continuous limit for equal states. Couple this ubar to auxiliary
midpoints and to the same transposed boundary force. The resulting boundary
step has an exact discrete energy balance up to nonlinear solve tolerance.

For the tested Kerr map `Q=m(u+χu³)`, `E=m(u²/2+3χu⁴/4)`, a cancellation-free
formula is

```text
ubar = (u1+u0)[1/2+(3χ/4)(u1²+u0²)]
       / [1+χ(u1²+u1*u0+u0²)].
```

After eliminating auxiliary midpoints, damped Newton solves only for the
primary trace. The fixture uses spatially varying χ between 0.8 and 1.6,
nonzero initial memory, and steps 0.001, 0.1, 1 and 10. It takes 3–7 Newton
iterations, with equation residual <=`1.1e-14` and relative energy-balance
residual <=`2.2e-15`. This is a boundary-substep oracle, not a finished generic
nonlinear solver or measured GPU iteration budget.

#### Force-coupled counterpart (Stage 8, 24 September)

The substep above stood alone. The production kick is the force-coupled
implicit midpoint rule of `Ẋ = G(u, z) + [s − F, 0]` for `X = (Q_Γ, z)`
([scattering spike](funfern-boundary-scattering-spike.md), sections 3–4). The
nonlinear counterpart replaces the trace midpoint `u_mid = (Q0 + Q1)/2m` with
the per-node discrete gradient `ū`, everywhere that midpoint appears. The
auxiliaries keep `z_mid`, because their energy is quadratic.

```text
Q1 − Q0 = τ [G_uu ū + G_uz z_mid + s − F]
z1 − z0 = τ [G_zu ū + G_zz z_mid]
```

Since `ΔT = ū·ΔQ` and `Δ(½|z|²) = z_mid·Δz` hold exactly, the kick's energy
change is `τ (ū, z_mid)·G(ū, z_mid) + τ ū·(s − F)`. That is the linear balance
term for term, so the wall stays passive and its loss is
`τ Σ (w(ū) + memory(z_mid))² + τ Σ d ū²`.

**Solve.** Newton on `Q1`, with `g = dū/dQ1 = (U(Q1) − ū)/(Q1 − Q0) > 0`.
Linearizing `ū ≈ ū_k + g_k (Q1 − Q_k)` makes each iteration exactly the linear
kick at the per-node mass `m_k = 1/(2 g_k)`. The constant
`c_k = ū_k − g_k Q_k` enters its right-hand side as `τ G(c_k, 0)`. The existing
mass-free trace factor, sweeps included, is therefore the whole inner solve.

- A linear node has `g = 1/2m` and `c = Q0/2m`, and reproduces the linear kick
  in one iteration.
- A first-order wall is the scalar case, `Q1 − Q0 = τ(s − F − d ū)`, whose
  residual rises with slope at least one. Any trial point `x` and `x − f(x)`
  bracket its root.
- `ū` is the energy quotient when `|Q1 − Q0| > 1e-4 |Q|`. Below that it is the
  four-point Gauss–Legendre mean of `U` over the interval, which avoids the
  quotient's cancellation.

**Measured** in the canonical core (`canonical_temporal` tests; Kerr mass
`χ = 0.8`, saturable stiffness row):
- A kick converges in 3 linear trace solves (1134 kicks), sometimes 4 (83),
  and needs no damping.
- The per-kick balance is checked at `2e-10` relative.
- Over a run, both wall orders balance at order 2.00–2.02, including under a
  pump.
- `χ = 0` reproduces the linear wall to `1e-12`.
- The wall's departure from its linear control is cubic in the amplitude
  (ratio 3.99 on doubling).
- A Gaussian at `u = 1.2` leaves through the second-order wall with 1.0% of
  its energy remaining, and the energy budget closes to `7.4e-5`.

A future cheap-path restriction, if needed, should say “linear primary
constitutive map at these trace nodes,” not ban all nonlinear material in an
entire boundary-touching domain. Complementary nonlinearity alone does not
invalidate the linear primary boundary solve. No such restriction is imposed
by this experiment.

## 5. Numerical results and accuracy limits

The core fixtures are unchanged: 113 enriched nodes for spectral/long-time
checks, 1601 nodes for packets. Results include:

- Coupled semidiscrete spectral growth <=`6.2e-15` for no loss, both-side loss,
  complementary loss 4 (the old counterexample), and spatially varying loss.
- Actual split-step spectral radius excess <=`8.3e-15` at 0.2, 0.7 and 0.95
  times the exported timestep bound; constant/unused auxiliary modes explain
  unit eigenvalues.
- Boundary midpoint energy excess <=`2.9e-15`, including large boundary steps.
- Actual split-step temporal error ratio `3.99954` on halving dt.
- Runs to t=100 (1691 steps) have no observed increase above initial total
  energy; final fractions are `5.09e-6` lossless, `2.59e-29` for the old
  complementary-loss counterexample, and `2.09e-6` for spatial loss.
- Small-angle reflection halving ratio `15.96`, confirming fourth-order scaling.

Planar continuum amplitude reflection, using the candidate analytic admittance:

| Incidence | First order | Candidate |
| --- | --- | --- |
| 15° | 1.73% | 0.156% |
| 30° | 7.18% | 1.96% |
| 45° | 17.16% | 7.94% |
| 60° | 33.33% | 21.39% |
| 75° | 58.88% | 47.99% |
| 85° | 83.97% | 78.56% |

Grazing reflection remains large; “second order” is not “transparent.” In the
low-frequency evanescent limit the coefficient is `6d/(7s)` rather than `a/s`,
a **19.8% underestimate**. These limits motivate a higher-accuracy family later.

Finite-box Gaussian packet residual bulk-energy amplitude at t=2.6, using the
actual split timestep (560 steps), is a different measurement:

| Direction | First order | Candidate |
| --- | --- | --- |
| 0° | 2.52% | 3.03% |
| 30° | 5.06% | 5.87% |
| 60° | 5.26% | 5.71% |
| 75° | 3.63% | 3.69% |
| 45°, aimed at corner | 5.83% | 7.37% |

These pass the predeclared normal limit 15% and first-order+2 percentage-point
limits for other directions, but show **no measured advantage over first order
on this finite box**. Auxiliary energy is reported separately and is at most
0.0374% of initial energy at this endpoint. Incident packets have transverse
spread, encounter different faces, and feel the corner approximation; the table
must not be presented as pure single-face reflection or evidence of improvement
over the legacy second-order boundary.

The optional annular fixture has 24 sectors, three radial layers, 480 nodes,
144 triangles and 96 boundary nodes. Both the concave inner and convex outer
polygonal circular traces absorb. At t=20 (1189 steps) no energy increase is
observed; final total energy fractions are `6.93e-5` lossless and `8.71e-10`
with complementary loss 4. Unit isotropic coefficients make the exporter's
outer-side-label normal convention irrelevant to the impedance. This is a
**curved-geometry passivity/decay smoke test**, not the original hole-and-baffle
topology reproducer, a curvature-convergence study, or a curved DtN accuracy test.

## 6. Cost, lifetime and remaining gates

At Nb trace degrees of freedom, three f32 auxiliaries need `12Nb` accepted bytes
or `24Nb` accepted+candidate bytes. The modal transform is dense (`4Nb²` bytes).
The correctness oracle's full midpoint map is `64Nb²` bytes, but the reduced
solve needs only an Nb-square factor (`4Nb²` as a full-storage upper bound)
plus the transform and per-mode data. For Nb=128 this is 3072 bytes of doubled
auxiliary state, 65,536 bytes for each dense transform/factor, versus 1,048,576
bytes for the oracle's full map. Workspace and preparation buffers are extra.

Even the reduced route has O(Nb²) boundary work and an O(Nb³) dense preparation
factorization; it is not the old local auxiliary cost. Mesh/material/dt edits
must prepare the appropriate factor without reading live fields back to CPU.
The boundary factor/transform tables need inclusion in portable binding and
peak-generation budgets. Measure the actual production-intended core before
choosing a dense GPU solve, cached inverse, iterative solve, or compressed/local
realization. No isolated performance benchmark was used to declare success.

All auxiliary state is accepted/candidate state. A failed trace solve rejects
the whole timestep/event, including fields, memory, clock and accounting. For
AMR and geometry edits, transform modal memory to physical weighted trace
coordinates before transferring it; never interpolate mode indices across two
different eigenbases. New-boundary initialization and exterior edits require an
explicit energy/work policy. Those transfer/failure paths are not implemented
in this reference.

Before production enablement:

1. Review the angular/evanescent accuracy tradeoff and the candidate's nonlocal
   cost; retaining the second-order option does not require retaining its old
   unstable auxiliary equation.
2. Port the equations into the real CPU/GPU core and validate f32, actual trace
   solve tolerance, full nonlinear composition, primary material mixtures,
   boundary-history handoff and failure rollback.
3. Preserve the current curved-boundary gate until the original topology case
   and geometry-specific radiation accuracy have been checked. Do not claim
   that graph passivity alone implements exact exterior propagation on curves.
4. Measure wall time per simulated second at matched accuracy, memory and edit
   latency on that core, including AMR and boundaries. Kernel timings may later
   diagnose a bottleneck; they do not replace this acceptance measurement.
5. Extend toward better rational kernels/CRBC or shape-specific DtN behind the
   same energy-conjugate trace interface. Do not assume coefficients or corner
   rules can be swapped without new validation.

Production code, scenes and autosaves remain unchanged. The result closes a
bounded auxiliary-system correctness investigation, not the full production
boundary migration or all material-overhaul gates.
