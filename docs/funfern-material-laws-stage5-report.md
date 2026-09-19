# Material-law Stage 5: GPU handoff and live-event report

**Date:** 19 September 2026

**Target:** Apple M1 Max, Metal/WebGPU

**Scope:** latest-state generation transfer, physical histories, runtime records,
paused events and atomic rollback

Stage 5 extends the dormant canonical `Q,b` GPU core from the
[Stage 4 report](funfern-material-laws-stage4-report.md). The currently connected
scalar solver is still the application path; rendering, probes, diagnostics, AMR
consumers and the production skin cutover remain Stage 6.

## Implementation

- Backend-neutral transfer exports now describe support-aware primary rows, local
  six-sample vector rows, thin-gap correspondence/energy weights and outgoing
  physical-trace correspondence. Complementary lookup and changed outgoing-basis
  composition have resumable jobs. Synchronous reference constructors drive the
  same work to completion, so the cooperative and oracle results compare exactly.
- A versioned packed GPU transfer has eight storage bindings per pipeline. Metal
  exposes only four read-only storage buffers in this configuration, so transfer
  inputs use logically read-only `read_write` bindings while retaining the portable
  eight-buffer ceiling. Cross-file tests and native compilation exercise every
  pipeline rather than validating WGSL text alone.
- Fourteen ordered dispatches copy/map latest accepted `Q,b`, thin-gap jump and
  normalized outgoing state; reduce desired component totals; enforce the 5%
  bounded correction; apply prescribed ownership and edit exchange; transfer
  clock, source phase/runtime and serials; rebuild the constitutive force cache;
  account changed auxiliary/main energy; validate; and commit one generation.
- Exact same-index `Q,b` and unchanged outgoing modal storage have compact identity
  representations. The identity predicates verify stable donor indices, unchanged
  geometric support and the complete outgoing operator/basis before omitting map
  rows. A changed operator or modal basis still goes through the physical-trace
  invariant dense map.
- The source generation remains installed and readable until the target status
  reports a complete successful transaction. Acceptance replaces generation,
  buffers, clock and readback ownership together. Rejection deletes the target and
  retains byte-identical accepted source storage. Candidate force caches contain
  constitutive force only; thin-gap force remains its separate evolution stage.
- Live source, fixed-law, pulse, paired-filter and maintenance payloads are staged
  against the current generation. They commit at complete-step boundaries even
  while paused, do not advance the clock, retain revision ordering, and use the
  same global validator. Source frequency edits preserve instantaneous carrier
  phase and use an independent accepted runtime-table slot.

There is no CPU field readback, field calculation or re-upload in either path.
The CPU prepares geometry, correspondence and immutable candidate data; all
field-dependent transfer and admission use the latest accepted GPU state.

## Correctness acceptance

The native harness warms the source generation, begins the handoff, advances the
accepted target, and compares its readback with the Stage 3 f64 transfer/evolution
oracle. Errors below are after 24 target steps, not immediately after a trivial
copy. `Aux` is RMS difference in physical auxiliary storage.

| Fixture | Request-to-visible | Q rel. L2 | b rel. L2 | Aux RMS | Clock error |
| --- | ---: | ---: | ---: | ---: | ---: |
| Standard 8,938/17,472 same mesh, first order | 32.82 ms | 2.131e-7 | 1.202e-6 | 0 | 1.369e-8 s |
| Irregular 2,692→4,200 / 5,112→8,112 | 33.67 ms | 2.164e-7 | 1.307e-6 | 0 | 4.664e-9 s |
| Standard same mesh, 825 outgoing auxiliaries | 49.06–104.50 ms | 2.149e-7 | 1.212e-6 | 5.679e-10 | 1.369e-8 s |
| Irregular second order, 429→477 auxiliaries | 47.74 ms | 2.221e-7 | 1.276e-6 | 8.616e-10 | 4.664e-9 s |
| Thin gap, 28→31 physical jump samples | 48.40 ms | 2.465e-7 | 7.949e-7 | 1.987e-9 | 1.930e-8 s |
| Same mesh with nonzero prescribed primary | 49.36 ms | 2.293e-7 | 1.381e-6 | 0 | 4.135e-9 s |
| Same mesh with mapped harmonic source | 33.14 ms | 2.198e-7 | 1.380e-6 | 0 | 4.135e-9 s |

The first-order standard result clears the 35 ms same-mesh bound. The timer starts
after CPU preparation and `begin_handoff`, then includes render extraction, GPU
transfer plus validation, status readback and the main-world visible switch; it is
therefore a conservative bound on the device commit rather than an isolated shader
timestamp. The standard second-order range includes its nonlocal validation and is
below the separate 250 ms second-order request limit. It is reported as a range
rather than selecting the fastest run.

Injected non-finite rejection took 35.62 ms and left the source generation,
accepted storage and clock byte-exact. Five separately queued paused events (zero
loss patch, phase-preserving source edit, nonzero pulse, nonzero filter and
maintenance) completed in 217.00 ms total; the slowest event was 32.91 ms. The
clock did not move, runtime serial ownership was `[source=2, law=1, pulse=3,
maintenance=5]`, and 48 resumed steps ended at `Q=3.027e-7`, `b=1.096e-6` versus
the CPU event oracle.

The Stage 3 f64 fixtures still cover reversed thin-gap orientation, new/deleted
histories, basis sign/order/degenerate rotation, component split/merge order,
bounded extension and twelve irregular refine/coarsen cycles. Stage 5 adds f32 GPU
application, global acceptance and continued evolution to those map contracts.

## Preparation and memory

Preparation is measured separately from the live GPU transaction. The resumable
jobs below used 256 complementary targets and 128 outgoing mode pairs per slice.

| Fixture | Total transfer prep | Largest complementary slice | Largest outgoing slice | Final GPU packing |
| --- | ---: | ---: | ---: | ---: |
| Standard same mesh, first order | 9.42 ms | 4.10 ms | 0 | 0.36 ms |
| Irregular first order | 17.55 ms | 1.17 ms | 0 | 0.55 ms |
| Standard same mesh, second order | 9.67 ms | 4.09 ms | 0 | 0.44 ms |
| Irregular changed second order | 34.73 ms | 1.19 ms | 0.19 ms | 0.80 ms |

The remaining interpolation, primary and history slices were at most 4.64 ms, so
every measured cooperative unit is below the 12 ms responsiveness bound. The
changed second-order outgoing composition takes 17.43 ms in total but yields in
0.19 ms units; an unchanged boundary is an exact identity and performs no dense
composition.

The standard first-order source and target generations are each 7.64 MiB. Their
full-resource overlap is 15.28 MiB and the compact identity map is eight 16-byte
words. The two packed main-state buffers total about 0.806 MiB, below twice the
0.427 MiB legacy 9,326-record state baseline; accepted physical `Q,b` alone totals
about 0.335 MiB. The standard second-order overlap is 17.23 MiB, including the
separately reported 0.93 MiB boundary factors in each generation, again with an
eight-word identity map. An irregular first-order transaction is 2.25 + 3.56 +
0.77 = 6.58 MiB; the changed second-order case is 2.54 + 3.91 + 1.56 = 8.01 MiB,
where the last term includes its dense invariant history map.

## Verification

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check -p funfern-app --target wasm32-unknown-unknown
cargo build --release -p funfern-app \
  --example canonical_gpu_handoff --example canonical_gpu_live_events
target/release/examples/canonical_gpu_handoff [fixture flags]
target/release/examples/canonical_gpu_live_events
```

All 602 workspace tests pass; one pre-existing legacy curved second-order
reproducer remains ignored. Shader parsing/validation, strict Clippy and wasm32
compilation pass. Native Metal ran the actual bind groups, command ordering,
buffers and readbacks for every result above.

## Stage boundary

Stage 5 closes latest-state GPU transfer, physical-history/runtime handoff,
cooperative transfer preparation, paused live events and rejected-generation
rollback. It does not connect the canonical core to production rendering or AMR,
nor claim migrated probes, thin-gap diagnostics, energy/flow/far-field consumers,
curved-boundary accuracy, time-driven media or nonlinear execution. Stage 6 owns
those consumers and the shared-skin linear production cutover.
