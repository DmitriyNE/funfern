# Where a solver step goes — 23 September 2026

**Status: measured, not yet acted on.** Two levers are identified and costed
below; neither is implemented. Frame pacing, which is a separate defect found
in the same investigation, is being fixed first and is described at the end.

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

## Lever 2 - a static-mass generation should not sweep at all

The CPU already splits this. `direct_trace` applies a precomputed dense inverse
when the trace mass matches what the factor was inverted for, and falls through
to the sweep when the mass has moved. The device has only the sweep.

Packing the inverse for undriven generations turns `3 + 2 x sweeps` dispatches
per half-kick into 2, so the boundary nearly disappears from a linear scene's
step. Cost is `trace^2` floats: 332 KB at 288 trace nodes.

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
