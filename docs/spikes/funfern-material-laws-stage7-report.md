# Material-law Stage 7: time-driven media

**Date:** 24 September 2026

**Target:** Apple M1 Max, Metal/WebGPU

**Scope:** time drives, Switch, reciprocal ramp semantics, modulation-aware
AMR/filter/boundaries, and application enablement of the tested combinations

Stage 7 makes the material coefficients functions of time. A drive (harmonic,
smoothed-square or travelling modulation) or a Switch ramp moves the mass row,
the stiffness row or both. The accepted `Q,b` generation evolves through that
motion on the GPU, and the application can author, run, edit and hand off
driven scenes. Field-dependent laws remain Stage 8.

The stage closes with its exit gate met for the enabled drives: phase, event,
energy and handoff tests pass on the device, and the incremental cost is
recorded. It does **not** close every item its task list names. The
temporal-reflection physics fixtures and a calibrated frozen-impedance curve
are still open, and three defects were found while writing this report. Both
lists are below and carried forward explicitly, rather than folded into the
closed gates.

## Runtime and bulk semantics

- **One runtime contract** (20 September; 36a2265, 390e4f4). Coefficients are
  evaluated at each stage's own time on both sides.
  - Reciprocal factors follow the reciprocal of the live trajectory, not a
    re-interpolated endpoint.
  - A Switch is stamped on the device clock, never from host time. Its ramp is
    a C² smootherstep, and reversing it mid-ramp is value-continuous.
  - Records are mapped by stable material identity.
  - Tests: `reciprocal_switch_uses_the_reciprocal_of_the_live_trajectory`,
    `switch_reversal_is_value_continuous_and_zero_duration_is_immediate`,
    `shared_nodes_gather_independently_driven_material_contributions`.
- **Kick-drift-kick with `(t, p_t)` stages** (af2b33b). Verified:
  - inert parity with the fixed step;
  - reversibility;
  - CFL contraction under the trajectory bound;
  - a second-order temporal-work residual.

  The GPU layout followed in 2769818.
- **Real-core bulk** (83a5ba1): 96 steps across a clock rebase, Q 3.72e-7,
  b 2.62e-6 against the host reference. The re-run is
  `canonical_gpu_temporal`: Q 3.882e-7, b 4.172e-5.

## Events, handoffs and live edits

- **Live Switch and law patch** (71c1d91): Q 3.63e-7, b 2.41e-6.
- **Rejected temporal event** rolls back byte-exactly and resumes (7fa5fcf).
  `canonical_gpu_temporal_rollback`: Q 2.645e-7, b 2.426e-6.
- **The runtime bank travels in the state snapshot** (layout version 4,
  2de3a30), so a carrier's phase and a Switch mid-ramp survive a handoff
  instead of restarting from their anchors.
  - `canonical_gpu_temporal_handoff`: Q 2.986e-7, b 2.090e-6.
  - Static↔driven handoffs carry the field (0b3c2dd, 4dab9fc).
- **Source edits on a driven medium are live patches**, re-anchoring the
  carrier on the device clock (3b93a2e). A pulse is scaled on the device
  (10aef6a). Tests: `canonical_gpu_temporal_live_source` (Q 4.817e-7,
  b 2.214e-6, where an unedited run differs by 1.7e-1), and
  `canonical_gpu_temporal_pulse` (Q 2.505e-7, b 6.313e-6).

## Diagnostics

Every consumer reads the stage coefficients the step used:

| consumer | test | error against host |
| --- | --- | --- |
| point probe | `canonical_gpu_temporal_consumer` (c73e74b, 871f0e5) | 8.836e-8 |
| line probe | same example (ef2c61b) | ≤ 1.855e-6 |
| area energy | same (2104668) | 2.339e-7 |
| arrows | same (fb7b517) | 1.354e-6 |
| pump power / temporal work | `canonical_gpu_temporal_work` (ef34776, 2de3a30) | Switch 2.004e-7, pump 1.428e-8 |

- The far-field projection has an explicit time-invariant-exterior policy
  (e54b2a8). The application does not currently enforce it; see
  [defects](#defects-found-at-closeout).
- In the app itself, probes and arrows on a driven medium were blank until
  41bf525 and d1f517c (23 September).
- A filter-boundary rate defect was fixed (9f9ec98, f0c8b0f).
  `canonical_gpu_filter_boundary`: u 4.140e-7, rate 1.094e-6.

## Modulation-aware AMR, filter and boundaries

- **AMR estimator** (22 September, 168b26e…bd53c5b).
  - It reads the solver's own flux. The efficiency-index spread is at most
    1.18× across 15× the unknowns, where travelling mass read 7.5–17.4 before.
  - `DRIVEN_INDICATOR_CALIBRATION = 1.88` (734de2d). Open boundaries and gaps
    were added in e08da4a.
  - Interface calibration (d282369): driven path index 1.01–1.42.
  - Device gate `canonical_gpu_temporal_amr`: indicator 4.6e-7, refined
    transfer terms ≤ 3.4e-7.
- **Grid filter:** the frozen-time filter (Λ = 4/dt_max²) is closed for the
  conservative bulk (5e96b07). One filter: Q 3.88e-7, b 4.17e-5. Six resident
  filters: Q 7.25e-7, b 2.02e-4, against a bound of 3e-4.
- **Boundaries, CPU** (58988e7, 0b23031, 440d40f, fc004fa, 232873d, b823af8,
  38d7490): composed with inert parity at 1e-12, and constant loss keeps
  second order (orders 2.11 and 2.03).
- **Boundaries, device.**
  - `canonical_gpu_temporal_forced` (pump, source and absorbing wall) found the
    authored-mass kick defect: 1.439e-2 → 5.978e-6 (6d87be6), now Q 2.415e-7,
    b 4.825e-7.
  - A second-order wall under a drive solves its mass-free trace system by
    sweeps (60f7cb7, 83ccb27). `canonical_gpu_driven_document`: Q 2.457e-7,
    b 5.067e-7.
- **Frozen reference impedance** is admitted as a documented approximation
  (Gate B). It has been measured only with an uncalibrated Switch probe
  (440d40f): excess 0.003 at mass ×2.2, 0.085 at ×6. The calibrated curve is
  carried forward.

## Application enablement

**Enabled** (22–23 September: b56810a, 5993aaf, fb6e53f, e7d3e2e, 69b31a7,
cfbf92e):
- Presets on the mass or stiffness row: switchable medium, parametric pump,
  travelling modulation, time crystal, and the reflectionless time interface.
- Drive authoring with a response selector, Switch ramp and law summary.
- Composition with sources, prescribed data, constant loss, thin gaps and both
  outgoing orders.
- AMR, the resident grid filter, and probes and arrows.
- Live source, weight and pulse patches.
- Driven↔driven and static↔driven handoffs, and skin changes.

**Refused by design:**

| refused combination | why |
| --- | --- |
| prescribed data on an outgoing trace under a drive | needs its own trace composition |
| driven or field-dependent loss | Stage 8 |
| field, restoring and van der Pol laws | Gates C and O |
| spatially varying drive parameters | by authoring design |
| nonzero initial gap or pole history in a new temporal generation | needs an explicit initializer |
| fixed recorder stencils on temporal plans | a fixed stencil would read the wrong coefficients |
| a time-varying far-field exterior | policy e54b2a8, not currently enforced in the app (defect 1) |
| AMR on a driven medium with loss, a damped boundary or prescribed data | not yet calibrated for those |

**Not built:** there is no UI trigger for a live Switch or a live temporal law
patch, so an authored alternate never begins switching in the app. The
plan §11 "Switch ▸" button with its hotkey and the Advanced view belong to
Stage 10.

## Performance

- **Device, driven against fixed** (`canonical_gpu_temporal_timing`, 15,270
  dofs): 516.9 against 516.5 µs a step. A driven step costs what a fixed one
  costs. The difference is the tighter trajectory step (8.93e-4 against
  1.097e-3), about 1.23× wall time per simulated second.
- **CPU oracle:** 18.7× the fixed cost in the bulk and 37.5× behind a
  second-order wall before the sweep route, which brought the latter from
  27,147 to 6,373 µs a step. Stage-coefficient caching is carried forward.
- **Application.** The reported frame-rate drop on a pumped medium was frame
  pacing, not the drive, and is fixed.
  - Both step fences now pace on steps the queue has retired (089ce90).
  - Fixed generations apply a packed trace inverse instead of sweeping
    (6778148), taking second-order steps from 0.35–0.51 to 0.21–0.31 ms.
  - Driven walls keep the sweep, since their mass moves. On 24 September,
    `canonical_gpu_temporal_timing --outgoing` at 15,270 dofs ran 800 µs a
    step driven, and 699 µs for its fixed comparison. That fixture packs a
    backend state, so its fixed run sweeps too.

  See the [throughput spike](funfern-step-throughput-spike.md).
- **Driven pack:** 1,273 → 118 ms at 41k dofs.

## Defects found at closeout

Found by reading the code while assembling this report, and confirmed there.
None has been reproduced in a running scene yet.

1. **The far-field exterior refusal cannot fire in the application.**
   `topology_runtime.rs` builds the far-field stencil from the law-stripped
   model, whose materials are all time-invariant, so
   `FarFieldCompileError::TimeVaryingExterior` never triggers. A driven
   exterior would be projected with the static Kirchhoff formula and the
   authored inverse mass: plausible and wrong.
2. **The device grid filter is not gated on the conservative bulk.** The CPU
   reference refuses the time-driven filter outside it
   (`apply_grid_filter`), but the device encodes the filter on any plan when
   the setting is on. A driven scene with a source, loss or an outgoing wall
   and the filter enabled runs a composition no test covers.
3. **Application AMR reads the authored runtime,** not the accepted bank. The
   comment in `ui/workers.rs` says nothing in the application re-anchors a
   carrier, which live source patches and handoffs now do. The estimate is
   then taken against a mis-phased coefficient. This affects the adaptation
   target, not the field.

## Carried forward

- Temporal-reflection, parametric-growth and travelling-asymmetry physics
  fixtures (plan §13.1). Only the structural
  `the_reflectionless_pair_divides_on_one_row` exists.
- The calibrated frozen-impedance reflection curve (Gate B).
- A device gate for a handoff that changes whether the medium is driven.
- Device gates for loss, thin gaps and prescribed data under a drive; these
  are CPU-gated only.
- Stage-coefficient caching in the CPU oracle.
- The estimator cell residual still differentiates Q/M. A driven accuracy
  target has to be set against the ~0.7 index scaled by 1.88.
- The wavelength size limit still uses the authored speed, not the trajectory
  minimum.
- The live Switch trigger, the Advanced view and Stage 10 presets and
  demonstrations.
- Engineering-log TODOs that touch driven runs: simulated time running
  backwards at a handoff, no host view of drawn-state cadence, source ease-out.

## Verification

- **All fifteen canonical GPU examples pass** on 6778148 and 1178378: the
  thirteen validation examples and both timing harnesses.
  `canonical_gpu_timing --failure` passes from 1178378.
- **Workspace tests:** 351 core, 200 and 191 app, plus the smaller targets,
  all passing.
- **Gate:** clippy `-D warnings`, rustfmt, the release build and the wasm32
  check pass.

## Stage boundary

Time-driven media are enabled for the combinations above, and each rests on a
device gate against the host reference. Stage 8 starts the CPU nonlinear maps,
inverses and tangent bounds, Kerr and saturable first. The three
closeout defects should be fixed before it does. The first two let the
application run a composition its own policy refuses, and Stage 8 will lean on
exactly those refusals when it adds field-dependent laws.
