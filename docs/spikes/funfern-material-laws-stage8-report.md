# Material-law Stage 8: nonlinear CPU maps, bounds and composition

**Date:** 24 September 2026

**Target:** the f64 CPU reference (`canonical_temporal.rs`)

**Scope:** Kerr and saturable field response on either constitutive row,
composed with everything the time-driven reference steps: drives, the Switch,
sources, prescribed data, loss, thin gaps, both outgoing walls, the grid
filter, the estimator, probes, maintenance and skin conversion.

Stage 8 makes a coefficient depend on its own field, on the CPU reference only.
The application and the device refuse a field-dependent generation. The core
now compiles one, and the device would otherwise run it as a linear medium.
Stage 9 ports the maps against this oracle.

The stage closes its exit gate for the enabled combinations:
- scalar and vector maps;
- the TM/TE duality and Mechanical ↔ TE contracts;
- near-bound and domain failures;
- full nonlinear boundary composition;
- declared accuracy and passivity.

Signed χ₁, reciprocal (inverted) field responses and nonlinear anisotropy stay
refused. They close gate C separately.

Four defects in earlier stages were found while doing this work: three Stage 7
defects and one harness defect. All four are fixed; see
[defects](#defects-found-and-fixed).

## Maps, inverses and bounds

Constitutive kernels (`material_law.rs`, ebec99c):

- **Executed subset.** `FieldLawValues::executable` admits linear, Kerr
  (`χ₁ = 0`) and saturable laws on the direct coefficient, with a positive
  tangent over every admitted amplitude. Refusals name their gate:
  `SignedPolynomial`, `Reciprocal` or `NotMonotone`.
- **Scalar and vector.** Every executed law is even, so one map serves the
  nodal scalar and the quadrature vector through `r = |field|`. The radial
  tangent `ḡ + rḡ′` bounds the tangential `ḡ` wherever either falls below one,
  so the scalar `tangent_range` is also the vector bound. This is checked over
  the range, not assumed.
- **Junctions.** `ConstitutiveSite` holds every material's `m·ḡ(r)·r` term at a
  site, so a junction node inverts its assembled sum. It also provides the
  co-energy `∫P`, in closed form, with the saturable log on its series below
  `x = 1e-3`, and the stored energy `r·Q − ∫P`.
- **Inverse.** Safeguarded Newton inside the analytic bracket
  `[0, Q/Σ m·ḡ_min]`, cut to the tightest declared amplitude bound.
  - A target beyond the bound is `OutsideDomain`, not clipped.
  - The iteration cap is `NotConverged`.
  - Neither returns a best guess.
- **Tolerances.** f64 `8ε`. The device criterion is fixed now, at `4ε₃₂`.
- **Timestep.** The trajectory bound already took the minimum tangent.
  Self-focusing χ > 0 therefore never tightens the step. A defocusing
  saturable law with `a = χu_s² < 0` scales it by `√(1 + 9a/8)`.
- **Tests:**
  - both sides of the Kerr bound `B² < 1/(3|χ|)` and of saturable `a = −8/9`;
  - residuals over 19 decades from four initial guesses, including NaN and 1e30;
  - a three-material junction;
  - domain refusal at a junction's tightest bound;
  - convex stored energy whose gradient is the field.

## Bulk evolution

032c7c4.

- **Split.** The kick–drift–kick split is unchanged, because the Hamiltonian
  stays separable. Only the observables become inverses: `U = P⁻¹(Q)` per node
  and `v = r·b/|b|` per sample, with an explicit zero case.
- **Energy.** Energy and its explicit rate come from the co-energy, so
  temporal work composes as before.

| measure | result |
| --- | --- |
| energy deviation, 0.5·dt_max / 0.25·dt_max | 1.88e-4 → 4.64e-5 (ratio 4.06) |
| pumped Kerr, `ΔH − W` on halving | 1.82e-5 → 4.49e-6; reversible to 1e-13 |
| departure from the linear control, A → 2A | ratio 3.81 against the cubic law's 4 |
| saturated far above `u_s` against its `1 + χσ²` limit | within 2e-3 |
| TM ε-Kerr against TE μ-Kerr under `b ↦ −b` | identical to 1e-12; the swapped slot is not |
| χ = 0 through every nonlinear path | the linear medium to 1e-13 |

A flux past a declared bound is refused at state construction and by a pulse,
and a refused pulse leaves the state byte-identical. A step already commits
only at its end.

## Composition

418838c and 7406c9a. A strong field, with the maps tens of percent from linear,
balances at order > 1.7 in every case below:

| combination | notes |
| --- | --- |
| volume source + prescribed wall, under a pump | source and pinned work through the discrete gradient `ΔT/ΔQ` |
| prescribed field | written through the forward map, `Q = P(g)`; a held field in a fixed medium exchanges nothing |
| constant loss, both channels | `Q, b` scale exactly, which is passive because `T` rises with `|Q|`; the rate is the relative decay of stored flux |
| thin gap | unchanged spring |
| nonlinear stiffness row against both outgoing walls | the trace solve stays linear |

## The nonlinear primary trace

01fe8d3; derivation in the
[auxiliary spike](funfern-boundary-auxiliary-spike.md), section 4,
"Force-coupled counterpart".

- **Scheme.** The force-coupled implicit midpoint with the trace midpoint
  replaced by the discrete gradient `ū`. The balance is exact to the solve,
  and the wall stays passive.
- **Second-order wall.** `nonlinear_outgoing_kick_with` is Newton on `Q_Γ`.
  Each iteration is the linear kick at the mass `1/(2 dū/dQ)`, so the existing
  sweep factor is the whole inner solve.
- **First-order wall.** `damped_nonlinear_kick` is a scalar Newton, bracketed
  because its residual rises with slope at least one.

| measure | result |
| --- | --- |
| χ = 0 against the linear wall, both orders | 1e-12 |
| balance order, both orders and a pumped second-order wall | 2.00–2.02 |
| departure from the linear wall, A → 2A | ratio 3.99 on both orders |
| Gaussian at `u = 1.2` through the second-order wall | 1.0% of the energy stays; the budget closes to 7.4e-5 |
| Newton cost | 3 linear trace solves per kick in 1134 of 1217 kicks, 4 in the rest |

The admission refusal "a field response on an open or absorbing wall awaits
its boundary kick" is lifted.

## Gate F: the grid filter

5545f43. The linear polynomial frozen at its tangent at the event state:
- `M⁻¹ → 1/P′(U)` and `J → J_b`;
- the outer operators act on the actual `U(Q)` and `F(b)`;
- `Λ` comes from the tangent envelope.

Its first-order energy change is `−α/Λ²[(K_tU)ᵀA(K_tU) + FᵀAK_tAF] ≤ 0`.
Higher orders are unsigned, so the commit rule stays: the nonlinear energy may
not rise, and the new state must invert.

| measure | result |
| --- | --- |
| χ = 0 against the linear filter | 1e-12 |
| held field | untouched exactly |
| component total / compatibility | 1e-13 / stationary part below 1e-10 |
| departure of the correction from linear, A → 2A | 2.47e-3 → 9.80e-3 (ratio 3.96) |
| 20 steps each followed by a full-strength filter | every one commits and removes |

## Estimator, probe, maintenance and transfer

d3e8158.

- **Driven supplement.** It reads nonlinear observables and weights every
  defect by the tangent at the snapshot. It splits each node's store exactly by
  term, `Σ mᵢ(ḡᵢU² − Gᵢ(U))`, and reads the outgoing defect at the trace's
  discrete gradient. The element energies sum to the solver's energy to 1e-12,
  while the linear maps misread the same state by over 5%.
- **Calibration.** In `temporal_amr_calibration` the efficiency index keeps its
  flatness:

  | row | spread over 15× the unknowns | index at h = 0.2 / 0.14 / 0.1 |
  | --- | --- | --- |
  | inert | 1.18× | 1.509 / 1.278 / – |
  | mass Kerr | 1.17× | 1.558 / 1.332 / 1.424 |
  | stiffness Kerr | 1.18× | 1.606 / 1.360 / 1.450 |
  | both saturable | 1.18× | 1.704 / 1.448 / 1.535 |
  | pumped mass Kerr | 1.17× | 1.702 / 1.452 / 1.548 |

  Up to 10% above the linear rows, so the driven accuracy target carries over.
- **Point probe.** It inverts at the solver's own samples and interpolates only
  physical fields.
- **Maintenance.** The correction is spread by the tangent `P′(U)` and then
  reinverted; a correction past a bound is refused.
- **Transfer.** Conservative in `Q` and `b`. The target state reinverts, and
  refuses a flux outside its domain.

## Skin conversion

990b95b.

- **Laws.** Direct Kerr and saturable laws carry to the same physical
  coefficient, per gate C's contract: `ξ = s₀(1 + χ|e|²)e ↔ D = ε(1 + χ|E|²)E`
  and `ρ(u) ↔ μ(H)`. Signed and reciprocal field responses are refused by name.
- **Transactional.** One unconvertible material leaves the whole document
  unchanged, and its name is in the error.
- **Evolution.** A pumped stiffness row with a Switch alternate, and a pumped
  Kerr medium, each step identically as Mechanical and as TE, to 1e-12.

## Defects found and fixed

1. **Prescribed data was first order on the time-driven path** (7406c9a). Found
   as a first-order balance in the 8.4 composition test, and reproduced on a
   linear medium.
   - Cause: the drift read a pinned node's field back from its endpoint flux.
   - Fix: the drift reads `g(t_n+½)`, and the kicks charge a pin's work at its
     stage field. The same change was made on the device.
   - Trajectory ratios went from 2.2 and 2.1 to 4.08 and 4.05, the fixed
     path's.
2. **A driven thin gap drifted on the authored mass on the device** (772186f).
   A new `FORCED_GAP=1` mode reproduced it: Q 7.3e-3. After the fix the
   device's errors are Q 2.46e-7, b 4.91e-7.
3. **The device examples could not fail** (772186f). Every `main` discarded
   `AppExit`, so a failed gate exited 0.
   - All fifteen now return it; the unfixed gap run exits 1.
   - `FORCED_BARE=1`, which could not start, was repaired alongside.
   - Earlier "all pass" statements rested on the printed figures, which were
     read.
4. **A skin change ran stiffness-row drives backwards** (990b95b).
   `convert_material` reciprocated the law on its way to ε, although the solver
   multiplies `s₀ = ε` directly.
   - At a pump crest: `b/1.3` in Mechanical against `1.3 b` in TE.
   - Its test pinned the wrong algebra. Both now assert the solver's maps.

## Cost

`canonical_temporal_timing --nonlinear`: Kerr mass and saturable stiffness at
an amplitude where both maps depart from linear, 5,485 dofs, µs per step. The
fixed column is the same operator stepped linearly.

| composition | fixed | pumped (Stage 7) | nonlinear |
| --- | --- | --- | --- |
| bulk | 275 | 3,239 | 13,815 |
| bulk + source | 300 | 4,277 | 27,966 |
| thin gap | 282 | 3,293 | 14,281 |
| first-order wall | 286 | 4,187 | 32,609 |
| second-order wall | 730 | 5,810 | 34,370 |

- The oracle pays a full bracketed solve at every node and sample in each of a
  dozen helper walks per step. The Stage 7 stage-coefficient caching item
  applies here with more force.
- Forced kicks double the bulk cost, because the discrete gradient reinverts
  each node's endpoints.
- None of this is a device budget; that is Stage 9's.

## Refused by design (carried)

| refused | where it is refused |
| --- | --- |
| signed χ₁, reciprocal field response, nonlinear anisotropy | compile and skin conversion (gate C) |
| field-dependent loss (saturable absorption, polynomial rate) | compile; needs its own passivity derivation |
| prescribed data on an outgoing trace | compile, as in Stage 7 |
| area readout on a field-dependent medium | `CanonicalTemporalAreaContribution`, `sample_temporal_canonical_area` |
| every field-dependent generation in the app and on the device | `FIELD_LAWS_AWAIT_THE_DEVICE`, `attach_temporal_bulk` |

## Carried to Stage 9 and beyond

- The device port of the maps, the discrete-gradient walls, the tangent filter
  and the supplement, at `F32_INVERSE_TOLERANCE`, with cached guesses and
  stage-validated failure.
- An amplitude-aware wavelength for the size rule. Kerr χ > 0 slows the wave
  where the field is strong, and the rule is handed the small-signal medium.
- Stage-coefficient caching in the oracle.
- The self-focusing-beam and defocusing physics fixtures of plan §13.1, beyond
  the amplitude-scaling and saturation-limit ones here.
- The area consumer on field-dependent media.
- The Stage 7 carried items, which are unchanged.

## Verification

- Workspace tests: 386 core, 197 app, 203 app-bin, plus the smaller targets,
  all passing.
- clippy `-D warnings`, rustfmt, the release build and the wasm32 check pass.
- The fifteen device examples, and both `canonical_gpu_temporal_forced` modes,
  exit 0. Since 772186f that exit code is the gate.
