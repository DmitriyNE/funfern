# Material-law Stage 9: nonlinear media on the device

**Date:** 24 September 2026

**Target:** Apple M1 Max, Metal/WebGPU, f32

**Scope:** the Stage 8 CPU reference's Kerr and saturable maps and
compositions, ported to the production GPU solver and enabled in the
application

Stage 9 runs field-dependent media on the device. Each stage inverts the
assembled nodal map and the radial quadrature map by a safeguarded f32 Newton,
at the tolerance Stage 8 fixed (`4ε₃₂`), and fails detectably rather than
accepting a guess. The application prepares, runs, reads out, adapts and hands
off these media, and offers two presets. One composition stays refused: the
grid filter.

Four earlier defects were found and fixed while doing this work; see
[defects](#defects-found-and-fixed).

## Parity against the f64 reference

`canonical_gpu_nonlinear`: Kerr on the mass row and saturation on the
stiffness row, 5,485 dofs, 200 steps, with the field 16–34% from its linear
read.

| mode | Q | b |
| --- | --- | --- |
| free bulk | 8.595e-7 | 8.789e-7 |
| `NONLINEAR_PUMPED=1` (drive on the Kerr row) | 9.990e-7 | 1.157e-6 |
| `NONLINEAR_FORCED=1` (volume source, harmonic prescribed wall, both loss channels) | 2.255e-6 | 1.250e-6 |
| `NONLINEAR_GAP=1` (stiff thin gap) | 5.484e-7 | 9.219e-7 |
| `NONLINEAR_WALL=1` (first-order wall) | 1.090e-6 | 8.466e-7 |
| `NONLINEAR_WALL=2` (second-order wall) | 1.136e-6 | 8.639e-7 |

The Stage 0 limit is 3e-5.

The other device gates:
- **Consumers** (`canonical_gpu_temporal_consumer`, `CONSUMER_NONLINEAR=1`):
  point u 1.79e-7, rate 2.11e-6, energy 1.33e-7. Line and arrows: at most
  1.93e-6.
- **Handoff** (`canonical_gpu_temporal_handoff`):
  - `HANDOFF_NONLINEAR=1`, driven linear source into a Kerr/saturable target:
    Q 2.917e-7, b 1.897e-6, clock 1.254e-8.
  - `HANDOFF_REJECT=1`: rejected with the inverse-domain status; the source
    steps on.
- **Failure** (`canonical_gpu_nonlinear_failure`): a bounded Kerr field driven
  past its bound.
  - The device refuses the same step the reference refuses (255), with
    `STATUS_INVERSE_DOMAIN`.
  - Its accepted state is 1.191e-6 from the reference's step 254.
  - A retry fails again with every stored bit unchanged.

## What the device does

- **Records.** Temporal coefficient records grow from 3 words to 4. The law
  kind sits in the flag bits, and the fourth word holds
  `(χ, saturation, bound, ḡ_min)`.
- **Kernels.** `field_response` gives `ḡ`, `ḡ + rḡ′` and the co-energy;
  `primary_site` sums a junction's records. Two bracketed solves:
  - `solve_primary_radius`, the nodal solve;
  - `solve_complementary_radius`, the radial solve at a sample.

  A solve past a bound raises `STATUS_INVERSE_DOMAIN`. A capped solve raises
  the new `STATUS_INVERSE_CONVERGENCE`.
- **Stage caches.** Each site is solved once per stage: the nodal field before
  the drift, and each sample's secant `r/(j|b|)` before each kick. The secant
  turns the folded linear force entry into `W curlᵀ v(b)`.
- **Kicks.**
  - A pin is written through the forward map, `Q = P(g)`.
  - An absorbing wall solves the discrete-gradient kick by bracketed scalar
    Newton, with `ū` taken as the two-point Gauss–Legendre mean of `U`.
  - A second-order wall runs four Newton linearizations around the unchanged
    linear trace solve. Each iteration is the linear kick at the mass
    `1/(2g_k)`, with the old field `2c_k`. The finalize rejects a kick whose
    last step exceeds `1e-5` of the largest trace flux; a one-solve budget
    measurably fails that check.
- **Handoff.** A field-dependent target inverts every transferred site in
  `handoff_finalize`. A domain violation rejects the handoff and keeps the
  running generation.
- **Refused on field-dependent plans:** pulses, maintenance and temporal law
  patches. Each is a mass-scaled or positive-factor event and takes a new
  generation instead. The grid filter is refused too.

## Application

- **Presets:** "Kerr medium" (M-F1) and "Saturable medium" (M-F2), both
  self-focusing. A defocusing law needs an authored bound that a slider
  cannot promise to keep valid.
- **Readouts:**
  - The painted field is `P⁻¹(Q)`, read on the CPU through the operator.
  - The energy readout is the nonlinear store.
  - Point, line and arrow probes invert on the device.
  - The area readout refuses these media, on both sides.
- **Adaptive refinement:** the Stage 8 supplement, and the size rule at the
  law's small-signal limit.
- **Failure.** A mid-run device failure pauses at the last accepted step, is
  logged with its reason and lights the badge. Run or Step retries from that
  step. An edit hands off from it. Before this, such a failure latched
  silently, and the next edit's upload was rejected as though it had faulted.
- **Run** (a pumped scene with second-order walls and adaptive refinement from
  about 20k to 48k dofs, 40 s): 11,169 steps with Kerr, against 19,777 with a
  linear law.

## Cost

Measured with `canonical_gpu_temporal_timing`, 15,270 dofs, µs a step,
unfenced:

| wall | driven linear | nonlinear | ratio |
| --- | --- | --- | --- |
| first-order | 350 | 850 | 2.4× |
| second-order | 792 | 1,872 | 2.4× |

The stage caches took the nonlinear figures down from 1,241 and 3,095. The
plan flag took the driven linear figure from 417 to 350, below its pre-Stage 9
cost.

## Defects found and fixed

1. **Pins with loss under a drive disagreed with the reference** (9afebba).
   The device's loss stages re-pinned a prescribed node at the authored mass;
   the reference decays it and pins only in its kicks. The new
   `FORCED_PINNED_LOSS=1` mode measured Q 1.276e-2 before the fix and
   2.702e-7 after.
2. **A 9.1 regression: the probe record stride** (1e779ea). Two probe
   shaders hard-coded the 3-word stride in `probe_temporal_factor`, so a
   travelling drive on the stiffness row read its probe-point phase from the
   wrong words. The consumer gate drives the mass row only, which is why it
   missed this.
3. **A mid-run device failure could not be recovered in the app** (e6f55bf).
4. **The first cut's record scan cost linear driven media about 10%.** It was
   removed by the plan flag (b19f905).

## Carried forward

- The grid filter on a field-dependent medium on the device (gate F is
  CPU-only).
- The area readout on these media, on both sides.
- An amplitude-aware wavelength for the size rule.
- Pulses and live law patches on field-dependent plans, which currently
  prepare a new generation.
- Defocusing presets with an authored bound.
- The Stage 8 physics fixtures (self-focusing beam, defocusing control) as
  device demonstrations, and the rest of Stage 10's UX.
- The Stage 7 and Stage 8 carried items.

## Verification

- Every device example and mode exits 0; since Stage 8 the exit code is the
  gate.
- Workspace tests, clippy `-D warnings`, rustfmt, the release build and the
  wasm32 check pass.
