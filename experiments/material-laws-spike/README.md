# Constitutive-state validation spike

Isolated numerical experiment; production code is not modified. `run.sh` extracts
the pinned clean `beca47e` core into a temporary directory, compiles it directly
with rustc (no external Rust dependencies), exports its actual P2e basis,
quadrature geometry, mass/stiffness and boundary operators, then runs a NumPy
reference. Requires Rust and Python 3 with NumPy. No application or autosave is
opened. Temporary builds are retained and their path is printed for inspection.

Run from the repository root:

```sh
bash experiments/material-laws-spike/run.sh
```

The captured run is in `results.json`; interpretation and the architecture
recommendation are in `../../docs/spikes/funfern-material-laws-spike-report.md`.
The subsequent passive-boundary follow-up has its own `boundary-README.md`,
`run-boundary.sh`, `boundary.py`, and `boundary-results.json`; its passing
candidate does not change the retained legacy failures in this first run.
The final single-boundary check is in `scattering-README.md`: it verifies the
reflection prediction and corrects the auxiliary prototype's timestep coupling.
The runner intentionally exits **1** when any numerical acceptance check fails.
This is currently expected: the spike found an unstable second-order outgoing
extension, redundant-memory spectral artifacts, and an under-resolved input
fixture. Failures are retained, not waived to make the experiment green.
Writing stdout to a file captures the complete JSON even on that exit status.

## Acceptance criteria established before numerical runs

- Assembled operator, potential/direct-state equivalence, sign/duality,
  constant-state and algebraic conservation checks: relative/absolute residual
  at most `1e-10` in f64 (energy derivative `1e-9`).
- Conservative temporal convergence: error reduction at least 3.5 on halving
  step on a fixed discrete eigenmode; energy excursion below 0.5% at tested dt.
- Kerr/saturable inverse residual below `1e-11`; passive loss substeps never
  increase constitutive energy beyond `1e-12` relative arithmetic slack.
- Frozen nonlinear-loss accuracy: below 2% at dt=0.01 for the declared unit-scale
  fixture, and error decreases at least 1.7 on halving dt.
- Boundary derivation must reproduce the old linear boundary transfer function.
  First-order normal packet residual amplitude below 0.15; second-order oblique
  residual no worse than first order plus 0.02 absolute on the resolved fixture.
  No growing homogeneous boundary eigenmode above `1e-8` real growth rate.
  A failure remains a failure; an alternative must be labeled separately.
- Thin-gap paired force cancels, energy derivative balances to `1e-9`, and
  compatible canonical/scalar force evolution agrees to `1e-10`.
- Remap: integrated primary total within `1e-11`; identity/constant/affine
  fixtures within `1e-10`; resolved smooth-field weighted RMS error below 5%.
  Track weak-divergence/interface artifacts and stationary energy separately:
  a scalar total check alone does not establish a valid vector transfer.
  For a smooth initially potential-derived field, remap-generated stationary
  energy must remain below 1% of transferred vector energy. This diagnostic
  threshold does not authorize projecting away physical charge created by loss.
- Filter: preserve constants and component totals to `1e-10`, reduce the top
  wave eigenmode by at least 50%, attenuate an eigenmode at <=0.1 of the
  frequency ceiling by less than 0.1% per application. Must expose its action
  on extra vector null modes, not silently erase all stationary fields.
- Source migration: analytic derivative/zero-limit checks within `1e-8`;
  phase-anchor/ramp event identities within `1e-10` in f64. The reference failure
  transaction must preserve all accepted state/clock/accounting on rejection.
- Resource screen: <=8 storage bindings/stage; candidate estimated main buffers
  <=2x measured baseline packing on the same mesh. CPU timings are indicative,
  not a GPU performance claim. A GPU benchmark is needed only if choosing the
  architecture depends on an unresolved hardware question.

Numeric tests, analytical derivations, resource estimates and untested production
behavior are reported separately. A critical failure triggers a bounded check of
the potential-plus-loss-memory alternative on the same failure, not a second full
implementation. Architecture acceptance requires review of the final report.
