# Material-law Stage 6: production consumer cutover

**Date:** 19 September 2026

**Target:** Apple M1 Max, Metal/WebGPU

**Scope:** canonical consumers, static-linear AMR and shared-skin production cutover

Stage 6 connects the accepted canonical `Q,b` generation from Stages 4–5 to the
application. Mechanical, TM and TE now evolve the same direct state; changing the
skin changes presentation rather than selecting a separate evolution formula. The
legacy scalar GPU path remains in the source tree as a comparison/recorder owner,
but is no longer installed as the application's accepted wave generation.

This is the first linear release boundary. Authored time-driven and nonlinear
laws remain explicitly unavailable until their later stages.

## Direct-state consumers

The core now owns one f64 consumer oracle shared by tests and GPU stencil
compilation. At a point it reconstructs

```text
u = sum_i N_i Q_i / M_i
c = B^-1 b
e = (m u^2 + b dot c) / 2
F = orientation * u * R c
```

where the six independent complementary samples are interpolated in the physical
element and `R` is the quarter-turn used by the canonical equations. Accepted
endpoint rate is computed from consecutive accepted `Q` states. There is no bulk
potential, inverse-derivative filter or component-mean subtraction.

- Point and line recorder shaders expose the signed primary field, accepted rate,
  complementary-field magnitude, canonical energy density and directed energy
  flow. Mechanical and electromagnetic skins differ only in names and units.
- Area probes use a shared degree-six physical quadrature to integrate primary
  mean/RMS, complementary RMS and canonical energy. Their compact output rings
  remain independent of mesh density.
- Far-field sampling reads direct `u`, its accepted backward rate and `grad(u)` at
  the existing contour. Its uniform, isotropic, lossless exterior eligibility
  contract is unchanged.
- The vector overlay reads the independent complementary field directly. All
  skins offer the in-plane field and energy-flow views; no skin reconstructs a
  transverse field from a scalar history.
- Total energy is the sum of primary, complementary, thin-gap and outgoing
  physical storage. UI terminology now says canonical/discrete energy and energy
  flow rather than implying the retired potential formulation.

Recorder rings keep samples across a mesh-generation handoff only when recorder
identity, physical skin and canonical semantics still match. A skin change clears
host traces instead of joining histories with different physical meanings.

The solver panel reports the canonical manifest's measured steady allocation,
not the retired scalar operator estimate. The standard core is 7.64 MiB; the
second-order fixture is 8.62 MiB including 0.93 MiB of boundary transforms and
factors. Recorder rings are optional consumer allocations: the fixed maximum
point, line, area and far-field output rings are approximately 1.00, 1.00, 1.50
and 2.75 MiB respectively, before their configuration-dependent stencils and
area scratch. They are not hidden in the solver-core figure.

## Static-linear AMR

AMR retains the existing scalar-equivalent spatial estimator only for static
linear material generations, and augments it from synchronized canonical
readback:

- the scalar acceleration adapter includes primary loss;
- element normalization uses direct primary and complementary physical energy;
- complementary endpoint defects compare accepted `b` against the curl drift and
  exact half-loss composition;
- thin-gap and outgoing endpoint defects are distributed back to adjacent
  elements, and their stored energy participates in normalization;
- interface jumps stay in the scalar estimator, while retired scalar gap and
  second-order auxiliary residuals are suppressed when the canonical supplement
  is present.

The supplement reports ordinary drift, thin-gap and outgoing contributions
separately. Dynamic and nonlinear material AMR stays gated: this adapter is not
being renamed into a general nonlinear residual.

### Post-cutover AMR and preparation correction

The first production acceptance run exposed two integration defects that the
isolated Stage 6 fixtures did not exercise.

- An AMR estimate was owned by the canonical request's mutable buffer revision.
  The periodic paired grid filter legitimately advances that revision, so after
  the first adaptive handoff every later CPU estimate was discarded as stale.
  Estimate ownership now consists of the immutable topology token, canonical GPU
  generation and sampled accepted step. Live events may run while the owned CPU
  snapshot is evaluated; a generation handoff still invalidates it. A native
  Metal trace then completed repeated adaptive handoffs from 9,653 to 12,669 and
  15,683 degrees of freedom.
- A wavelength or maximum-edge floor that bound the same element as the error
  target was classified as error-only. A satisfied global accuracy target could
  therefore suppress a mandatory resolution floor. Error and limit binding are
  now tracked independently, with the limit taking precedence when both apply.

The same run showed that preparation was resumable in name but not yet in frame
latency. The wall-time loop checked its deadline only after batches of 256
canonical elements, and the outgoing-boundary Jacobi eigensolve still occupied
one final work unit. On successively adapted second-order meshes the longest
observed slices were 222.5, 364.7 and 630.7 ms. Preparation now checks its
deadline after every indivisible unit, and the trace eigensolve yields between
small rotation blocks. Per-row validation releases its trees incrementally;
volume-source results move rather than deep-clone; complementary extension uses
an edge index instead of an all-triangle-pairs search; outgoing-history result
validation is blockwise; and completing one preparation phase yields before the
next phase starts. The application lends preparation 4 ms per frame. Total
second-order preparation can still span many frames, but the accepted solver is
scheduled throughout it rather than waiting behind a monolithic CPU tail.

## Generation, events and source continuity

Every topology candidate now compiles the canonical operator, forcing and
transfer maps. Assembly and transfer remain resumable. Source/probe-only edits
reuse the operator and avoid an unnecessary legacy scalar transfer while still
preparing the canonical identity handoff.

The production request owns stepping, reset, pulse, the paired grid filter,
latest-state transfer and global acceptance. Consumer dispatches run only after a
successful accepted commit. Event rejection is surfaced through the ordinary
status/log path without changing accepted state.

Stage 6 also closes a handoff defect exposed by production source editing. A
mapped edited source now installs the target waveform parameters while using the
old instantaneous integrated-rate anchor and carrier phase. Unchanged sources
retain their exact runtime record; a newly added legacy source starts with zero
integrated rate at the handoff time. Prescribed edits use the target parameters
with the same instantaneous carrier-phase rule. Disabled authored volume-source
slots remain present with zero weights, so enabling a source cannot shift runtime
slot ownership.

The periodic paired filter is scheduled at an exact accepted-step boundary with
its canonical admissible strength. Request batching stops at that boundary, so
filtering cannot land halfway through a multi-step encode.

## Verification

The completed checks are:

```text
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p funfern-app --target wasm32-unknown-unknown
cargo build --release -p funfern-app --bin funfern-app --example canonical_gpu_timing
target/release/examples/canonical_gpu_timing [reflecting/first/second-order]
target/release/funfern-app
```

The workspace suite completes with 639 passes; one historical legacy
curved-second-order reproducer remains ignored. The shader suite parses and validates the four new
consumer shaders as well as the evolution and transfer shaders. Strict Clippy,
all-target native checks and the wasm32 application check pass. A release native
application smoke run created the canonical Metal pipelines and evolved the
default scene without validation or pipeline errors.

The standard 1,000-step accuracy fixtures remain inside the Stage 0 bounds:

| Fixture | Dispatches/step | Q rel. L2 | b rel. L2 | Auxiliary RMS | Energy residual |
| --- | ---: | ---: | ---: | ---: | ---: |
| Reflecting | 4 | 1.140e-6 | 1.832e-6 | 0 | 3.155e-7 |
| First-order outgoing | 5 | 6.243e-6 | 6.227e-6 | 0 | 1.890e-6 |
| Passive second-order outgoing | 13 | 1.145e-5 | 1.878e-5 | 3.900e-8 | 3.036e-6 |

Short 1,000-step wall timings showed substantial fixed-overhead and desktop-load
sensitivity and did not consistently represent the sustained common-core target.
The longer run is therefore used for the declared *steady* throughput gate;
preparation and accuracy remain reported from their appropriate fixtures rather
than being folded into it:

| Fixture | Preparation | Simulated s / wall s | Required |
| --- | ---: | ---: | ---: |
| Reflecting, 5,000 steps | 63.59 ms | 16.13 | 12.48 |
| First-order outgoing, 5,000 steps | 62.51 ms | 13.74 | 12.48 |
| Passive second order, 5,000 steps | 299.84 ms | 8.71 | 5.0 |

The 5,000-step first/second-order comparisons exceed the deliberately
1,000-step f32 trajectory tolerance, so they are performance measurements, not a
replacement for the passing accuracy rows above. The standard second-order
preparation remains near 300 ms; as in Stages 4–5 it is cooperative preparation,
not request-to-commit latency, and does not block the UI.

## Stage boundary

Stage 6 closes the linear application cutover and consumer boundary. Stage 7 may
add time-driven coefficients and switching against this production state. It must
derive dynamic AMR residuals, temporal-work diagnostics and exterior/boundary
policy before enabling those combinations. Nonlinear constitutive inverses,
nonlinear skin contracts and nonlinear GPU recovery remain Stages 8–9.

The adopted second-order condition remains the passive nonlocal rational
candidate justified by the [auxiliary derivation](funfern-boundary-auxiliary-spike.md)
and [corrected scattering spike](funfern-boundary-scattering-spike.md). This
cutover does not turn it into an exact curved DtN map; the historical curved
legacy reproducer remains visible rather than being silently reclassified.
