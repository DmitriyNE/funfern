# Single-boundary scattering validation — 19 September 2026

**Adopted reference:** the user selected this corrected boundary update together
with direct Q,b state. The [consolidated specification](funfern-material-laws-plan.md)
and [detailed stages](funfern-material-laws-review.md#5-implementation-stages)
are the implementation handoff; this report retains the measured justification.

## Decision

**The passive auxiliary boundary converges to its predicted planar reflection,
including grazing incidence.** Its earlier square-packet results did not measure
isolated incidence/reflection at a single face. We can proceed to implementation
review rather than rejecting the boundary on that aggregate measurement.

The test also found and corrected a timestep issue: separating a boundary-only
midpoint update from the interior KDK force leaves a small, nonvanishing
reflection bias under fixed-CFL mesh refinement. The corrected update includes
the held interior force in each boundary-aware midpoint kick. It keeps the
interior drift explicit and uses the same small boundary solve.

The corrected candidate passes **248 checks**. The original split has **211
passes and 21 failures**, retained separately, with no relaxed thresholds.
This is numerical correctness evidence, not a GPU performance result or a
production deployment. The update correction is part of the implementation
contract; the old separate-boundary split must not be copied from the previous
prototype merely because its stability tests passed.

## 1. Reproducible fixture and measurement

Artifacts:

- [Criteria and commands](../../experiments/material-laws-spike/scattering-README.md)
- [Bloch-strip and timestep implementation](../../experiments/material-laws-spike/scattering.py)
- [Corrected results](../../experiments/material-laws-spike/scattering-results.json)
- [Rejected split results](../../experiments/material-laws-spike/scattering-original-split-results.json)

Run `bash experiments/material-laws-spike/run-scattering.sh`. Add
`--original-split` to reproduce the rejected scheme (expected exit status 1).
The runner compiles the same clean `beca47e` core and exports the actual enriched
triangular basis/quadrature; no production code or existing material-law edits
are changed. Requires Rust and Python with NumPy, as in the prior spikes.

This is an isolated **harmonic scattering measurement**, not another transient
packet run. A phase-periodic (Bloch-periodic) transverse cell represents a plane
wave with prescribed tangential wavenumber `ky=ω sin θ`, where θ is measured
from the absorbing face's normal. Thus 85° really is grazing incidence at the
tested face. There are no transverse walls or corners to reflect the wave.

The two enriched triangles of a square cell are taken from the exported FEM.
Identifying the top and bottom degrees of freedom with phase `exp(i ky h)`
reduces that cell to eight unknowns. Neighboring cells share two face unknowns,
giving `6Nx+2` unknowns in a strip. This is an exact Bloch reduction of that
uniform 2D discretization, not a different 1D element approximation.

The right face carries the absorber. A prescribed harmonic left trace drives
the strip. We do not infer reflection from the drive amplitude or residual
energy. Instead, static condensation of the cell interiors gives its discrete
normal-direction transfer matrix. Its two propagating eigenmodes determine
the forward/backward phase and trace polarization. A complex least-squares fit
to interior samples separates their amplitudes, extrapolated to the right face:

```text
u_j = Ainc v+ λ+^(j-Nx) + Aref v- λ-^(j-Nx)
Rmeasured = Aref/Ainc.
```

Using the **discrete** wavenumbers prevents interior numerical dispersion from
being mistaken for boundary reflection. Samples exclude four or more cells
near either end, removing localized evanescent branches. The left drive can
alter Ainc, but it does not alter the ratio produced by the right boundary.
Changing the strip length and fitting window verifies this separation.

Sweep: angles 0°,30°,60°,75°,85°; wavelengths 0.75,1,1.5; nominal strip length
2; approximately 8,16,32 cells per wavelength. Mesh cell counts are rounded up.
The main sweep uses `dt/h=0.015` to separate the spatial and temporal effects.
Time refinement uses `dt/h=0.12,0.06,0.03,0.015` at 16 cells/wavelength and
angles 30° and 75°. All three laws use the same FEM and update convention:
first order, the legacy second-order admittance, and the passive candidate.
“Legacy” here means its boundary law, not the old production scalar timestep.

## 2. Reflection results

For time dependence `exp(-iωt)` and unit exterior coefficients,

```text
Ranalytic = [cos θ - Y(-iω, ω sin θ)] / [cos θ + Y(-iω, ω sin θ)].
```

The new candidate's formula is unchanged from the
[auxiliary derivation](funfern-boundary-auxiliary-spike.md). The finite element
trace evaluates its matrix function using the actual tangential operator, not
the continuum ky substituted into a scalar boundary condition by hand.

Measured amplitude reflection at wavelength 1 and approximately 32 cells per
wavelength, with the corrected timestep:

| Incidence from normal | First order | Legacy second order | Passive candidate |
| --- | --- | --- | --- |
| 0° | 0.00113% | 0.00114% | 0.00113% |
| 30° | 7.17951% | 0.51540% | 1.95964% |
| 60° | 33.33298% | 11.11104% | 21.38739% |
| 75° | 58.87877% | 34.66737% | 47.98699% |
| 85° | 83.96631% | 70.50360% | 78.55573% |

Across all finest-mesh cases, the maximum **complex** reflection error, including
phase, is `1.14e-5` in amplitude, against the declared `0.01` limit. The complex
fit residual is below `2.7e-13`. Coarse-to-fine reflection error decreases by at
least a factor 9.82. Fixed-mesh temporal errors decrease by at least a factor
4.000 on halving dt, consistent with second-order stepping.

Changing the strip length from 2 to 3 changes R by less than `2.9e-12` in the
tested cases; narrowing the fitting window changes it by less than `2e-14`.
The local stiffness reconstruction agrees with the exported core operator to
`8.84e-17` relative; boundary line stiffness/damping agree to roundoff. The
Bloch matrices are Hermitian to the declared tolerance.

Consequently:

- Grazing improvement over first order is now **measured in the FEM**, not just
  predicted analytically.
- The passive candidate does **not** beat legacy second order on this clean
  planar lossless fixture. Its benefit is the demonstrated passive formulation
  and robust coupling to states that broke the legacy law, at a known accuracy
  and nonlocal-cost tradeoff.
- Large grazing reflection remains: roughly 79% amplitude at 85°. This is not
  an exact or high-accuracy DtN replacement.
- The square-packet table remains valid for its geometry and metric. It cannot
  establish the candidate's single-face reflection coefficient, and this test
  does not prove which corner/packet contribution dominated each old result.

## 3. New finding: fixed-CFL error in the original boundary split

The previous reference stepped boundary memory/Q separately before and after
the interior KDK update. At fixed spatial mesh, that Strang/midpoint composition
converges quadratically in dt, explaining why the preceding temporal test passed.
However, the boundary primary mass scales as `h²`, while boundary impedance
weights scale as `h`. The boundary rate therefore scales as `1/h`; fixed-CFL
refinement holds its dimensionless step size roughly constant.

For a first-order boundary trace with rate `g=D/MΓ`, each half-boundary step
multiplies Q by `(1-r)/(1+r)`, `r=dt*g/4`. Eliminating the two half-steps from
the discrete harmonic equations shows that the leading effective admittance
is `D/(1+r²)` as h→0 with dt/h fixed, rather than D. Higher-order boundary
states inherit the splitting issue; it is not a constitutive inverse error.

At normal incidence and wavelength 1, first-order reflection error in the old
split is:

| Cells/wavelength | Original split | Corrected update |
| --- | --- | --- |
| 8 | `7.66e-4` | `2.52e-4` |
| 16 | `5.31e-4` | `2.74e-5` |
| 32 | `5.16e-4` | `1.13e-5` |

The error tends toward a floor instead of converging to the correct boundary.
The old split fails 21 mesh-reduction checks across the sweep, despite meeting
the deliberately loose 0.01 absolute-error threshold. Retaining a refinement
requirement prevented that threshold from hiding the issue.

This does not mean every fixed-CFL boundary splitting is invalid, nor does it
revoke the continuous energy proof. It identifies a specific flaw in this
particular time-discrete coupling.

## 4. Corrected boundary-aware kicks

Let X contain primary boundary Q and its auxiliary state, and G be the linear
boundary generator in those coordinates. During a kick, b is held fixed and
the interior force `F=CᵀWb` is known. For kick duration `τ=dt/2`, update

```text
[I-(τ/2)G] Xnew = [I+(τ/2)G] Xold - τ [FΓ, 0aux].
```

Non-boundary Q nodes receive their ordinary explicit kick. Then drift
`bnew=b+dt C uhalf`, and perform the second boundary-aware kick with the new b.
The bulk remains explicit. **The solve must act on the interior-force term as
well as on the boundary state**; applying an independent boundary damping step
before/after the force is the rejected method.

The existing auxiliary elimination/reduced trace solve still applies with a
changed right-hand side. This is not a request for a global implicit interior
solve or an extra live CPU field round trip. No claim about its production GPU
cost is made here.

### Independent harmonic-stage verification

For `ζ=exp(-iωdt)`, bulk KDK uses
`ωh=2 sin(ωdt/2)/dt`. At the drift stage,

```text
b_n = dt/(ζ-1) C uhalf,
Qhalf = M uhalf.
```

Eliminating the boundary-aware kicks gives

```text
J = (ζ-1)I - (dt/2)(ζ+1)G + (dt²/16)(ζ-1)G²,
J [MΓ uhalf, zhalf] + ζ dt²/(ζ-1) [(K uhalf)Γ, 0aux] = 0.
```

The strip solver uses this equation to obtain the frequency response. A separate
check reconstructs the states and executes both forced midpoint kicks and the
drift; the complete harmonic-stage residual is below `1.2e-14`. It does not
merely compare the continuum formula with itself. The prescribed left trace is
excluded from the unforced equation check because it is the driving source.

### Stability regression after the correction

The small full 2D FEM—not just the Bloch strip—is retested with the same passive
boundary generator. At 0.2,0.7,0.95 times the existing timestep bound, spectral
radius excess is below `1.3e-14` for lossless, both-side loss, complementary
loss 4, and spatially varying loss. Runs to t=100 (1691 steps) show no peak
total-energy increase for the existing smooth initial fixture. Final fractions
are approximately `1.15e-3` lossless, `1.19e-28` both-side loss, `2.35e-29`
for the old loss counterexample, and `5.74e-4` with spatial loss.

These are numerical linear regressions, not a new theorem of unconditional
stability or exact physical-energy monotonicity for nonlinear KDK. The earlier
standalone nonlinear boundary discrete-gradient proof remains useful, but its
full force-coupled nonlinear implementation needs verification in the real core.

## 5. Handoff to implementation

This closes the requested isolated planar reflection question. Use the passive
auxiliary equations **with the corrected force-coupled boundary update** as the
current reference. Preserve the source/skin, history-transfer, failure, f32,
nonlinear-composition, curved-boundary and nonlocal-cost gates already recorded.

Measure performance in the production-intended solver core while implementing
it, including the boundary solve, acceptance boundary and live handoff. No new
GPU benchmark or further broad boundary spike is required by this result.
The absence of a universal reflection improvement must remain explicit in
documentation and UI claims. More accurate rational/CRBC/DtN variants can later
be compared through the same physical boundary interface.

Production code, user scenes and autosaves were not modified. The artifacts and
planning updates are the only changes made for this follow-up.
