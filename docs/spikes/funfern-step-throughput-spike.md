# Where a solver step goes — 23 September 2026

**Status: lever 1 implemented and measured (23 September 2026); lever 2 not
implemented, and its estimate below is revised down by that measurement.** Frame
pacing, a separate defect found in the same investigation, is described at the
end and has since been fixed.

## What prompted it

A reported frame-rate drop when a non-stationary material is selected. Reported
at 8827 dofs:

| material | fps | steps/s | dt |
| --- | --- | --- | --- |
| linear | 120 | 750 | 1.34e-3 |
| parametric pump | 65 | 900 | 6.27e-4 |

## The drive costs nothing per step

Steps per frame are `750/120 = 6.25` and `900/65 = 13.85`. Fitting both points
to `frame = overhead + steps x per_step`:

```
 8.33 ms = R + 6.25 S
15.38 ms = R + 13.85 S
-> S = 0.93 ms/step, R = 2.5 ms
```

One per-step cost explains both operating points, so a driven step costs what a
fixed one costs. The whole difference is that the driven generation runs at its
coefficient-trajectory CFL bound and therefore takes 2.2x as many steps for the
same simulated second. The simulated rate confirms it: `750 x 1.34e-3 = 1.00`
against `900 x 6.27e-4 = 0.56`, so the driven case is delivering 56% of the
requested speed.

Caveat: if 120 fps is vsync, the linear point is a ceiling and `S` from the fit
is an upper bound. The driven point is not vsync-capped, and alone it gives
`S <= 15.38/13.85 = 1.11 ms`, so the figure stands either way.

## The trajectory bound is not over-conservative

`maximum_time_step = base x sqrt(min_primary x min_complementary)`, over the
tangent range of every sample. Two ways that could be tighter than the physics
requires, neither of which applies here:

- the two slot minima are taken independently, which over-tightens when two
  slots modulate out of phase - but the pump preset sits on a single row
  (`LawPresetRow::Mass` or `Stiffness`), so the other factor is exactly `1.0`;
- the minima are global over the mesh rather than per element, which
  over-tightens when only part of the domain is driven - but the reported scene
  pumps the material covering the domain.

A dt ratio of 2.137 squares to 4.57, so the minimum tangent factor is 0.219:
the coefficient reaches 22% of nominal at the bottom of the cycle, a modulation
depth near 0.78. The step is what the scheme needs.

## Where the 0.93 ms actually goes

Per step, with a second-order outgoing boundary, the encoder emits:

```
kick_first
  boundary_prepare_first        (trace_count workgroups)
  boundary_reduce_first
  for 0..trace_sweeps:          two dispatches per sweep
      boundary_sweep_modal
      boundary_sweep_trace
  boundary_finalize_first
drift
kick_second
  ...the same boundary block...
commit_step
```

At the sweep ceiling of 32 that is `7 + 2 x (3 + 2 x 32) = 141` dispatches per
step. At roughly 6.6 us per pipeline switch and barrier this is 0.93 ms, which
is the figure the frame-time fit produced independently. The arithmetic is
negligible - each sweep dispatch is a trace x trace reduction, about 17k
multiply-adds at the reported size. The step is bound by launch latency, not by
work, which is why it barely moves with the mesh at these sizes.

## Lever 1 - the sweep count targets the wrong precision

`canonical_wave.rs`, in `CanonicalOutgoingMidpointFactor::prepare`:

```rust
let needed = (f64::EPSILON.ln() / contraction.ln()).ceil().max(1.0);
```

The device stores f32. `ln(f32::EPSILON) / ln(f64::EPSILON) = 0.44`, so
converging to the precision the device actually holds needs 44% of the sweeps.
Everything past f32 rounding buys nothing and costs two barriers a sweep.

Roughly halves the per-step cost. The CPU factor keeps the f64 count; only the
backend export changes, and `sweeps` is already a field on it.

### Measured

Implemented as `device_sweeps` on the factor and its export: the same bound at
`f32::EPSILON`, the same two spare sweeps, even. The device reads it; the host
solve and every host oracle keep the f64 count.

The premise above was wrong in one respect: the solve was never at the ceiling
of 32. Measured counts are 8 on the reported driven autosave (contraction
2.2e-3, f32 needs 6) and 10 on the timing harness's second-order scene
(contraction 2.8e-3, f32 needs 6). A smaller contraction than the ceiling
assumed is what keeps the f64 count low already.

Production render graph, `canonical_gpu_timing`, 128 steps, baseline and new
builds interleaved on the same machine:

| scene | sweeps | dispatches/step | baseline | device count |
| --- | --- | --- | --- | --- |
| 8.9k dofs, second order | 10 -> 6 | 51 -> 35 | 116.3-117.0 ms | 82.6-83.4 ms (1.40x) |
| same, with forcing / loss | 10 -> 6 | 53 -> 37 | 116.5-117.0 ms | 83.2-83.4 ms |
| 21.9k dofs, second order | 10 -> 6 | 51 -> 35 | 132.7-150.1 ms | 115.8-116.3 ms |
| 8.9k dofs, first order | - | 5 | 49.1-49.4 ms | 50.0-50.1 ms |

Every error lane is bit-identical to four digits between the two counts, as the
f32 argument predicts. On `canonical_gpu_temporal_timing` (15.3k dofs, driven,
outgoing) the gain is about 5%, 1131-1213 against 1192-1247 us/step: at that
size a step costs more than a millisecond and the sweeps are a smaller part of
it. In the app on the driven autosave at 2x speed the new build averaged 793
steps/s against 697 over three interleaved pairs, one of them tied, under a
load average near 20; indicative only.

### The step is not bound by dispatch count

Forcing the device count on the 8.9k scene, 3000 steps: 10 sweeps 0.78 ms/step,
8 sweeps 0.58, and 6, 4 and 2 sweeps all 0.527 ms/step to within 0.1%. The
first-order boundary, at 5 dispatches a step, costs 0.39 ms. So there is a floor
of roughly 0.4-0.5 ms a step that dispatch count does not explain, and the
6.6 us-per-dispatch model above does not hold below about 35 dispatches.

## Lever 2 - a static-mass generation should not sweep at all

The CPU already splits this. `direct_trace` applies a precomputed dense inverse
when the trace mass matches what the factor was inverted for, and falls through
to the sweep when the mass has moved. The device has only the sweep.

Packing the inverse for undriven generations turns `3 + 2 x sweeps` dispatches
per half-kick into 2, so the boundary nearly disappears from a linear scene's
step. **Revised after lever 1:** removing the remaining 6 sweeps
did not move the step at all when tried by forcing the count down to 2, so on
the measured scenes the direct lane can at most close the gap to the
first-order floor - about 0.14 ms of 0.53 - and may close less. Find what sets
the floor before building it. Cost is `trace^2` floats: 332 KB at 288 trace nodes.

This partly reverses `CanonicalWaveState::for_backend`, which skips building the
inverse because the export does not read it - correct as of today, and what took
the pack from 613 ms to 0.4 ms. If the inverse is packed for fixed generations,
`for_backend` becomes the driven-only path and the undriven pack pays the
inversion again, now at `O(trace^3)`: about 30 ms rather than the old 613.

## The separate defect: no frame budget

Frame rate should not be a function of solver throughput. A frame that cannot
advance the simulation as far as asked should still ship on time and draw the
latest state; the simulated-speed shortfall is the thing that gives, and it is
already measured and reported. At 13.85 steps a frame the solver is advancing
far more than one step per frame, so every frame has a new configuration to
show and there is no reason for the display to wait.

`steps_for_frame` caps by accumulated simulated time and by
`MAX_STEPS_PER_FRAME`; `steps_with_gpu_backpressure` caps by outstanding encoded
lead. Nothing caps by how long the batch will take to execute. The contract in
`pacing.rs` already states the intent - "preserve display service and report the
simulation-speed shortfall" - but the implementation only clamps the credited
delta to one 60 Hz interval, and at a small dt that interval is many steps. The
batch therefore sets the frame time.
