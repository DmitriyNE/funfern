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
- The GPU vector sampler reads the independent complementary field directly. All
  skins offer the in-plane field and energy-flow views; no skin reconstructs a
  transverse field from a scalar history. The complementary-field arrow view may
  subtract a slow presentation-only baseline, explicitly labelled AC coupling;
  its lazy history follows physical mesh samples rather than screen bins, and newly
  visible samples start silent rather than flashing an unknown DC baseline. The
  sampler, physical consumers and energy-flow view retain the full field. Shared
  quiet-tail visibility is applied after arrow saturation so sparse residuals cannot
  remain full-length after the bulk field has faded.
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

AMR retains the resumable scalar estimator infrastructure only for static linear
material generations, but its phase-paired field terms now come directly from
synchronized canonical state:

- primary-field gradient recovery and ordinary interface jumps remain valid;
- the phase-paired recovery reconstructs physical `Jb` at region-local vertices
  and measures quadrature defects in the `J^-1` norm, scaled by the forcing
  frequency;
- the cancellation-prone scalar `u_dot` recovery and semidiscrete-acceleration
  strong cell residual are not evaluated for a canonical snapshot;
- element normalization includes direct primary and complementary physical energy;
- complementary endpoint defects compare accepted `b` against the curl drift and
  exact half-loss composition;
- thin-gap and outgoing endpoint defects are distributed back to adjacent
  elements, and their stored energy participates in normalization;
- interface jumps stay in the scalar estimator, while retired scalar gap and
  second-order auxiliary residuals are suppressed when the canonical supplement
  is present.

The supplement reports complementary recovery, ordinary drift, thin-gap and
outgoing contributions separately. The Performance panel exposes the primary,
complementary, cell, jump and boundary split. Dynamic and nonlinear material AMR
stays gated: this adapter is not being renamed into a general nonlinear residual.

The post-cutover production autosave exposed why this distinction is numerical,
not cosmetic. At about 50k DOFs, deriving `u_dot` from adjacent f32 `Q/M`
endpoints or from the cancellation-heavy nodal balance made rate recovery
`0.09–0.22`, while primary recovery was only `5e-4–1e-3`; the scalar strong cell
term independently stayed near `0.05–0.10`. The reported error consequently
hovered around 40–90% and drove refinement despite a resolved primary field.
With canonical complementary recovery, the same run crossed the 12% target at
50,480 DOFs and then measured about 7–10%; the decreasing interior jump was the
dominant remaining term.

The controller treats the paired filter's exact cadence boundary as a
zero-duration maintenance event, not as an ordinary pair of time endpoints. At
that boundary the spare lane contains the pre-filter state, so AMR waits for the
next solver step before taking its synchronized snapshot. It also retains the
run's peak canonical energy across ordinary handoffs. Below `10⁻⁴` of that peak,
relative error is dormant and error-driven targets coarsen instead of chasing
floating-point tail; forced-wavelength and maximum-element limits remain active.
Refinement stops at the requested global error, while coarsening requires two
successive estimates below 65% of it; the interval is a hold band. Transactions
are directional: refinement disables collapses, and coarsening disables splits,
so a local size field cannot bypass the global accuracy decision. The spatial
collapse threshold is 0.45 of target versus refinement above 1.05, with a
two-generation modified-vertex cooldown. An adaptation scan that changes no
topology advances cooldown state without publishing a new solver mesh.

Full-state readback is self-describing. One state-buffer metadata word records
the accepted lane and exact accepted step at every commit, including
zero-duration events and handoffs. Static-linear AMR therefore never combines a
state copy with a control readback that arrived from another solver batch.

### Post-cutover AMR and preparation correction

The first production acceptance run exposed two integration defects that the
isolated Stage 6 fixtures did not exercise.

- An AMR estimate was owned by the canonical request's mutable buffer revision.
  The then-host-scheduled periodic paired grid filter advanced that revision, so
  after the first adaptive handoff every later CPU estimate was discarded as
  stale. Estimate ownership now consists of the immutable topology token,
  canonical GPU generation and sampled accepted step. Live events may run while
  the owned CPU snapshot is evaluated; a generation handoff still invalidates
  it. A native Metal trace then completed repeated adaptive handoffs from 9,653
  to 12,669 and 15,683 degrees of freedom.
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

### Interactive-performance correction

A second production run separated four effects that the original long-batch GPU
gate did not expose:

- Stage 5 preserves the absolute accepted-step count at handoff, but the UI rate
  accumulator still treated every generation as a counter reset. It therefore
  counted the complete run again and could suppress a real speed-shortfall notice
  for many seconds. A generation's first observation is now only its baseline.
- Complementary transfer located each of six target quadrature samples by scanning
  every source triangle. It now builds a cooperative uniform source index and
  recognizes an identical mesh even when its transaction revision changed. The
  standard 8,938-DOF handoff's vector-map time fell from 102.0 ms to 1.3 ms, and
  total transfer preparation from 109.9 ms to 5.7 ms.
- The display mapped all primary, complementary and auxiliary state every frame,
  expanded it into fresh host vectors, recomputed total energy, and applied the
  scalar stiffness solely to prepare a possible AMR sample. The continuous stream
  is now the primary state prefix. Full state is sampled at 4 Hz for energy/AMR;
  AMR accepts only an aligned full snapshot. The vector overlay independently
  selects one stencil per visible screen bin and reads back compact GPU-sampled
  complementary/flow records after rendered solver batches, rather than forcing
  a 15 Hz full snapshot and whole-mesh CPU reconstruction. Energy and acceleration
  run at their consumer cadence, and host vectors retain their allocations.
- Dense presentation no longer asks egui to flatten topology-sized geometry each
  frame. One clipped callback retains quadratic field positions/indices, linear
  categorical triangles and a unique mesh-edge list in GPU buffers; frames update
  field values, optional per-triangle colors and view/style uniforms. With the
  production autosave's field, Regions, mesh and boundary layers all enabled, a
  release run remained around 54–60 FPS through 52,660–67,956 DOFs and stabilized
  at 60 FPS on the settled 67,956-DOF mesh.
- Refining beyond that run exposed a solver/display feedback loop rather than a
  further presentation copy. Once one real-time solver batch missed 60 Hz, the
  next frame spent the whole late wall interval and submitted a larger batch to
  the same GPU queue as rendering and AMR readback. At the observed ~15 FPS this
  reached the 64-step frame ceiling. Pacing now spends at most one 60 Hz interval
  per frame; an overloaded scene falls short of requested simulated speed instead
  of starving display service in an attempt to catch up.
- Error estimation and mesh adaptation no longer receive a two-millisecond slice
  per rendered frame. A dedicated long-lived worker advances both jobs, separate
  from the candidate-assembly worker, and publishes only serialized results that
  still pass the topology/generation gate. Native and threaded WebAssembly use
  the worker; static WebAssembly retains the cooperative runner. A release profile
  of the production autosave showed AMR traversals on `funfern-amr` while assembly
  independently occupied `funfern-cpu-prepare`.
- Solver/display contention had one more independent feedback path. Continuous
  Bevy readbacks schedule a fresh staging copy every rendered frame; if Metal
  completion lags, copies and command buffers accumulate even though each frame's
  solver batch is bounded. State, control/status, vector, probe and far-field
  streams now allow one copy in flight per entity. Requested full snapshots run
  once; handoff admission polls its pending marker through the same bounded gate.
  Both the host request clock and render-world encoding stay within 64 steps of
  the last GPU-completed boundary. In the reported locked process, graphics
  mappings had reached 8.9 GiB/~39,800 regions with 4,414 command buffers. The
  corrected release soak held them to 41.1 MiB/268 regions and 30 command buffers
  after more than three minutes; physical footprint was about 0.88 GiB with no
  process compression. This closes the structural unbounded-queue route; exact
  foreground recovery remains a hands-on application check.
- Preparing a 93,144-primary-DOF GPU generation took about 155 ms synchronously,
  followed by a redundant second copy while turning roughly 80.7 MiB of typed
  buffers into Bevy assets. Reusing the already validated scalar CSR reduces plan
  compilation to about 54 ms, compact adjacency reduces canonical assembly to
  about 73 ms, native builds perform plan/transfer packing on a background worker,
  and owned serialization removes the duplicate copy. The remaining measured
  main-thread handoff call is 11.5 ms. The accepted generation continues through
  CPU packing, upload and admission: the transfer records the exact source clock,
  and the admitted target consumes requests queued during the admission readback
  from that transferred boundary.

The full production render-graph harness at 93,144 `Q` plus 185,310 independent
`b` samples sustains 1,025 steps/s with continuous full-state validation readback
and 1,164 steps/s with the production primary-only stream. Those runs correspond
to 2.04 and 2.25 simulated seconds per wall second on the M1 Max fixture and
exclude periodic filtering. Follow-up UI isolation found that the remaining
~220-step/s cap came from representing the every-16-step filter as a host live
event: each occurrence forced an upload, request revision, binding rebuild and
host acknowledgement. The filter now runs as validated resident maintenance in
the solver command stream. With it enabled, the same 93,144 / 185,310 fixture
runs 512 measured steps in 521 ms (about 982 steps/s and 1.90 simulated
seconds/wall second) while retaining the CPU-oracle and energy tolerances.
With 1,024 compact vector samples enabled, a later run completed the same measured
step count in 386 ms (2.56 simulated seconds/wall second) with `1.19e-6` vector
error; this is the production solver/readback path, not an isolated sampler bench.

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

The periodic paired filter is encoded at an exact accepted-step boundary with
its canonical admissible strength. It uses the ordinary accepted/candidate
validation boundary inside the same command stream: success flips the accepted
lane before consumers and the next step, while failure latches the global solver
status. It does not create a host event, request revision, upload or readback.
Consumers must nevertheless recognize that zero-time lane flip: complementary
arrow AC presentation rebases without treating the filter correction as a wave,
and the static-linear AMR adapter defers its endpoint sample. The filter preserves
constants and stationary force-free complementary flux; terminal quieting is a
separate run-relative presentation/estimator policy.

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

The workspace suite completes with 641 passes; one historical legacy
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
