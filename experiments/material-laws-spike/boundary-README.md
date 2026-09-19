# Boundary follow-up: passive auxiliary candidate

Historical auxiliary/passivity experiment. The subsequent `scattering-README.md`
confirms planar reflection but rejects this prototype's separate boundary split
under fixed-CFL refinement. Use its force-coupled midpoint-kick correction for
implementation; retain this runner/results to reproduce the earlier evidence.

Isolated correctness experiment. Production files remain unchanged. Uses the
same pinned baseline/exporter as the material spike. Run from the repo root:

```sh
bash experiments/material-laws-spike/run-boundary.sh
```

The candidate is a newly derived two-state, spatially nonlocal admittance,
**not an implementation of published CRBC**. For normalized tangential frequency
`a=c|k|`, put `d=a/sqrt(2)` and use
`Y(s)=1+d^2/[s(s+d)] = d/s+s/(s+d)` instead of `1+a^2/(2s^2)`.
For each mode: `p'=sqrt(d) w`, `r'=sqrt(d) w-d r`,
`j=w+sqrt(d)(p-r)`. Its energy identity is
`[(p^2+r^2)/2]'=w*j-(w-sqrt(d)*r)^2`.

With exported trace damping D and auxiliary tangential stiffness A, diagonalize
`D^-1/2 A D^-1/2=V diag(d^2) V^T` on active boundary nodes and use power-conjugate
input/output maps. A null tangential mode has d=0 and reduces to first order.
This matches the existing high-frequency second-order tangential correction,
but has O((a/s)^3) remainder, not the old truncation's O((a/s)^4).
The low-frequency evanescent coefficient is only 1/sqrt(2) of exact. Both are
explicit accuracy compromises to test, not claims of exact DtN.

After this two-state candidate passed its numerical checks, it was refined to
retain the old **quartic** small-angle reflection order rather than accept a
cubic downgrade. The implemented three-state version sets `d=sqrt(7/8)*a` and
uses `Y=1+(8/7)d^2/[s(s+d)]-(1/7)4d^2/[s(s+2d)]`.
It cancels the cubic expansion term and has a positive energy realization
derived in the follow-up report. This is a separately identified refinement,
not a relaxed criterion. Its small-angle halving ratio must be >=14 (order 4);
all other criteria below remain unchanged. Low-frequency evanescent bias is
`1-(6/7)*sqrt(7/8)`, about 19.8%, rather than zero.

## Criteria fixed before running this follow-up

- Analytic admittance: nonnegative real part (slack 1e-12) for sampled Re(s)>0;
  correct zero tangential mode; normal-incidence limit; angular reflection
  O(theta^3) with ratio >=7 on halving a sufficiently small angle. For angles
  15,30,45,60,75,85 degrees, reflection no worse than first order.
- Semidiscrete boundary energy identity: relative residual <=1e-11. Coupled
  linear spectrum max real part <=1e-8 for no loss, the prior (0,4) failure,
  both-side loss, and a spatial-loss fixture. Retain the old failing operator
  as a negative control, not as a passing acceptance case.
- Actual step: explicit interior KDK, half-step implicit-midpoint boundary
  updates, symmetric exact constant-rate loss. No global implicit interior
  solve. Boundary midpoint map contractive within 1e-11 in stored energy;
  full linear step non-growing to 1e-10 in spectral radius at declared CFLs.
  Temporal convergence ratio >=3.5. Long run at least t=100 on the coarse mesh,
  with finite state and no growth beyond 1% of initial physical total energy.
- Packet reference: same n=16 mesh/profile as original spike, directions
  0,30,60,75 degrees plus a corner-directed 45-degree packet; residual bulk
  amplitude <0.15 for normal incidence and no worse than first order+0.02 for
  each other case. Track auxiliary energy separately. These are finite-box
  packet proxies, not exact planar reflection coefficients.
- Curves: assess passivity separately from exact exterior radiation accuracy.
  A passive result alone does not certify a curved DtN approximation or fix the
  ignored topology reproducer. No production curved-support claim without
  the corresponding geometry/accuracy tests.
  An added 24-sector, three-ring annular smoke fixture tests both boundary
  orientations: to t=20, peak total energy <=1.01 of initial and final <=0.5.
- Nonlinear interior: continuous boundary energy proof must not rely on a
  linear interior constitutive map. Do not claim the linear midpoint update
  itself solves nonlinear boundary-touching nodes correctly.
  Added Kerr boundary-substep discrete-gradient tests require energy-balance
  residual <=1e-10 and nonlinear equation residual <=1e-11 at dt=.001,.1,1,10.
- Reduced linear trace solve: the boundary-only Schur update must agree with
  the full boundary midpoint oracle to <=1e-11 at the same four step sizes.
- Record spatial transform/implicit trace-solve cost and boundary-state memory.
  No GPU performance claim and no production enablement in this experiment.

All failures stay in generated JSON and cause exit status 1. A result that is
passive but misses reflection/order requirements is not silently relabeled a
replacement for the old second-order condition.
