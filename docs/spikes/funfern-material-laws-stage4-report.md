# Material-law Stage 4: f32 GPU core report

**Date:** 19 September 2026

**Target:** Apple M1 Max, Metal/WebGPU

**Scope:** linear canonical evolution, clock, events and global acceptance

Stage 4 implements the production-intended f32 `Q,b` solver as a dormant path
beside the currently connected scalar GPU solver. It uses the real Bevy render
graph and application plugin, but is not yet an application cutover: latest-state
generation transfer is Stage 5 and consumer migration is Stage 6.

## Implementation

- One versioned Rust/WGSL manifest has eight storage bindings: control, status,
  state, nodes, samples, packed tables, scratch and boundary data. Integer
  offsets, counts, clocks, flags and serials stay in integer lanes.
- State words carry accepted/candidate `Q`, independent two-component `b`, thin-
  gap jump and outgoing auxiliary state. Force caches, loss fractions, accounting
  and the clock have corresponding accepted/candidate ownership.
- The common force-cache KDK path uses four dispatches per step. First-order adds
  accounting; frozen two-sided loss/prescribed ownership adds two stages; the
  second-order trace adds eight parallel boundary dispatches. There is no serial
  GPU factorization.
- The passive second-order boundary exports the CPU Schur solve as a dense inverse
  plus modal three-state eliminations and physical energy transforms. Prescribed
  trace rows prepare one cached constrained factor. This replaced a dense CPU
  solve on every prescribed kick and matches its f64 oracle within `2e-11`.
- Pulses, the paired grid filter, fixed-linear loss patches and invariant
  maintenance stage candidate data and pass through the same global validator and
  commit. These are installation-time Stage 4 fixtures; scheduling edits against
  the latest live generation, including paused service, belongs to Stage 5.
- The clock rebases before `2^16` local steps or 256 local seconds, retaining a
  split `u64` epoch and retiming source/prescribed phase anchors. Failure priority
  is non-finite, inverse domain, timestep/envelope, layout/reference, success.
  Once latched, later encoded work cannot commit.

## Numerical and performance acceptance

The standard mesh has 8,938 primary DOFs and 17,472 complementary samples. All
rows below use 1,000 accepted steps after warmup and compare the GPU result with
the Stage 3 f64 CPU equations at the same accepted time.

| Fixture | Dispatches/step | Simulated s / wall s | Q rel. L2 | b rel. L2 | Auxiliary RMS | Energy residual |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Reflecting | 4 | 18.46 | 1.140e-6 | 1.832e-6 | 0 | 3.155e-7 |
| First-order outgoing | 5 | 18.50 | 6.243e-6 | 6.227e-6 | 0 | 1.890e-6 |
| Electric + magnetic fixed loss | 7 | 16.51 | 1.309e-6 | 2.292e-6 | 0 | 4.600e-7 |
| Passive second-order outgoing | 13 | 10.18 | 1.145e-5 | 1.878e-5 | 3.900e-8 | 3.036e-6 |
| Prescribed/source + second order | 15 | 9.73 | 1.462e-5 | 1.297e-5 | 5.613e-8 | 2.269e-6 |
| Second order + fixed loss | 15 | 9.86 | 1.541e-5 | 1.695e-5 | 6.134e-8 | 9.018e-6 |
| Thin gap | 4 | 17.32 | 1.012e-6 | 2.061e-6 | 6.536e-9 | 3.777e-8 |

The declared 1,000-step limits are `3e-5` for `Q,b`, `2e-5` absolute/RMS for
physical history and `2e-4` for the accounted energy residual. The thin-gap
history has a less useful relative error of `5.047e-5` only because the physical
state is near zero; its `6.536e-9` RMS is the applicable physical-history gate.
Lossless and passive fixtures show no undeclared growth.

The shipped eight-obstacle geometry, with first-order outer boundary, has 11,975
primary DOFs and 22,680 complementary samples. It prepared in 153.54 ms, used
9.99 MiB steady, ran at 10.35 simulated seconds/wall second, and produced
`Q=1.653e-6`, `b=3.550e-6`, and energy residual `6.151e-6`.

The standard reflecting allocation is 7.64 MiB total steady GPU data and 0.17
MiB accepted physical `Q,b,auxiliary` state. Its packed accepted/candidate main
state buffer is 0.403 MiB, versus 0.427 MiB for the baseline's 9,326 48-byte
`GpuState` records; operator tables and scratch are included in the reported
steady total rather than hidden in that comparison. The standard second-order
case is 8.62 MiB total, including 0.93 MiB of separately reported boundary
transforms and factors, with 825 auxiliary scalars. Standard second-order CPU
mesh/operator/factor/export/packing preparation measured 302–311 ms. A 128-step
request then completed in 50.21 ms at 6.29 simulated seconds/wall second; the 1,000-step rate
above separates steady GPU evolution from preparation and startup.

The reflecting and first-order steady rates exceed the 12.48 target derived from
75% of the 16.64 baseline. Second order exceeds its 5.0 steady target. Its
end-to-end CPU preparation is still above 250 ms on these runs, but the declared
250 ms request-to-commit review point is met by the measured 128-step request;
Stage 5 must keep preparation cooperative and measure the actual live handoff.

## Transactions and fixtures

- Injected non-finite failure leaves accepted physical/cache storage, accounting
  and clock bits exactly unchanged except for status; clearing it and requesting
  one step resumes normally.
- Individual pulse, filter, maintenance and fixed-law-patch candidates pass CPU/
  GPU parity and energy gates. The filter uses the actual paired linear operator,
  not a placeholder copy.
- A rebase crossing local step 65,535 preserves the accepted trajectory and
  monotonic serial. Unit fixtures separately verify time and harmonic phase-anchor
  continuity when changing the timestep.
- Cross-file tests compare all shared sizes, offsets, lanes, flags and binding
  count. WGSL validation runs in the workspace shader test.

Verification commands included:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check -p funfern-app --target wasm32-unknown-unknown
cargo build -p funfern-app --example canonical_gpu_timing --release
target/release/examples/canonical_gpu_timing [fixture flags]
```

All 598 workspace tests pass; one pre-existing legacy curved second-order
reproducer remains ignored. Native Metal exercises the actual GPU path. The WASM
gate here is compilation and shader validation, not a browser timing claim.

## Stage boundary

Stage 4 closes the linear f32 evolution, layout, clock, staged-writer, failure
and steady-performance gates. It does not claim latest-state topology/material
handoff, physical-history remap across generations, consumer correctness, curved
radiation accuracy, or nonlinear execution. Those remain Stages 5, 6 and 8–9,
and the old production solver stays connected until their dependent gates pass.
