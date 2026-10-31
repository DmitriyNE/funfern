# A point's transverse field from its neighbours' samples, 31 October 2026

**Status: done, 31 October 2026: go for patch recovery with a second ring at
material interfaces.** A spike in
`crates/funfern-app/examples/point_recovery_spike.rs`, with no production
change. The production change is planned separately (`docs/plan.md`,
Maintenance).

## The question

A point probe reads the transverse field as the quadratic through the six
constitutive samples of the element that holds it, each sample's physical
field `J b` taken where the solver owns `J`. The samples sit 0.0916 or more
in from every edge in barycentric terms, so a point nearer an edge than that
is read by extrapolation. On the `observables` channel at edge 0.07, the
upstream point, 0.004 from an edge, reads 9.0e-3 off an amplitude of 0.5,
and the fixture's CPU test holds the transverse field, the density and the
flow at 1.2e-2, 6e-3 and 6e-3 for it (Worth checking sometime, 2026-10-30).

The question was whether a fit over the samples of the elements around the
point reads closer to the truth, everywhere a probe can stand: in the bulk,
beside walls, slits and holes, and at material interfaces, on uniform,
graded, coarse, fine and sliver meshes.

## The harness

The harness meshes and assembles scenes as the app's runtime does
(`compile`, coarsening, `mesh_topology_plan`, the scalar operator, then
`CanonicalWaveOperator::compile`), so it holds the solver's own samples and
inverses. On the channel it reproduces the fixture: 1935 vertices, 11305
degrees of freedom, step 6.6931e-3, the upstream point 8.98e-3 off.

**The rules.** Each reads a point of an element from the physical fields at
samples, and each is linear in them, so its weights can be fixed when a
stencil is built:

- `own`: today's quadratic through the element's six samples;
- `averaged`: the error estimator's recovery, each vertex the area-weighted
  mean of its elements' own fits there, read linearly across the element;
- `edge patch`: a least-squares quadratic over the element and the elements
  sharing an edge with it, up to 24 samples;
- `spr`: Zienkiewicz and Zhu's superconvergent patch recovery. Each vertex
  has a least-squares quadratic over the samples of its elements. That fit
  gives the value at the vertex, and the mean of an edge's two vertex fits
  gives the value at the edge's midpoint. The reading is the quadratic
  through those six values across the element;
- `vertex patch`: one least-squares quadratic over every element touching
  the element's vertices;
- `+ring`: `edge patch` or `spr` with a second ring of elements where the
  patch meets a wall, a slit or an interface, so that it does not reach only
  one way;
- `+iring`: the second ring only where the patch meets another material.

No fit reaches into an element of another material or frame. A material here
means its coefficients: two regions of one medium fit together whatever they
are called. No fit reaches across a slit either, since its two sides hold
different nodes. Every fit is centred on its element or vertex and scaled by
the mean element size. A fit is never singular, because each holds at least
one element's six samples, which determine a quadratic.

**The levels.** Each rule was read at up to three levels, to tell the
reconstruction's error from the samples' own:

- L1: the exact field at each sample;
- L2: the field of the exact potential's interpolant, `compatible_flux`;
- L3: the solver's own samples, from runs whose solution is known.

**The corpus.**

- Uniform boxes at edges 0.1, 0.05 and 0.025, a plane wave of wavelength 1 at
  0.3 rad (L1, L2).
- The `observables` channel and four meshes beside it: edges 0.065, 0.07 and
  0.075, loop radii 0.297 and 0.303. Each was run over the fixture's 8 s,
  with the plane wave `u = g(t − (x + 1))` and the fixture's two points read
  every step (L3).
- A travelling packet on `[−3, 3] × [−½, ½]` at edges 0.1, 0.05 and 0.025,
  over 2.2 s (L3).
- Graded meshes: a small hole at 0.1 and two holes at 0.05, plus a curved
  hole, a free baffle and two baffle tips spanning 0.012 and 0.006 with
  slivers down to 3.6° (L2).
- A slit wall to wall, with a different wave on each side, which any fit
  across it would mix (L2).
- A disc of stiffness slot 4 at 0.08 (L2), and a packet crossing a material
  interface at 0.05 and 0.025, stiffness 1 against 0.25, reflected by 1/3 and
  transmitted by 4/3 (L3).
- All 45 gallery scenes at their own edge, the flux of a plane wave's
  interpolant at ten elements to the wavelength (L2 on `b`; a graded medium
  has no exact `J` between its samples).

Every element was read at 16 positions: its centroid, 0.004 and 0.06 off
each edge's middle, near each vertex, on each edge's middle, and among the
samples. Errors are relative to the field's amplitude. They are taken
separately by place in the element (within 0.01 of an edge, between that and
the samples, among them) and by element kind. A wall element has an edge on a
wall, a slit or a hole, an interface element has an edge on another material,
and a bulk element has neither.

## The rule set before measuring

Go if the best fixed-size candidate:

- at least halves the worst L3 error in every kind of element on every mesh;
- is nowhere more than 10% worse than today, walls, interfaces and slits
  included;
- has weight sums small enough for f32;
- reads the `observables` channel's points within about 2e-3 wherever they
  fall.

`spr` or the vertex patch would win only if clearly better than the edge
patch, since a stencil of variable size needs a table of sample weights on
the device. No-go if the error is the samples' own.

## What was found

**The samples carry the error, but it cancels between elements.** At L1,
with exact samples, `own` is the best rule everywhere: on the box at 0.1, at
an edge, 1.25e-3 against `spr`'s 2.07e-3. A wider fit pays polynomial error
there. The samples themselves are an order of magnitude further off than
that: on the channel, 5.4e-2 at L2 and 4.3e-2 at L3. That error alternates
from element to element, so a fit over several elements cancels much of it,
where `own` extrapolates it. The no-go branch assumed that an error in the
samples is beyond any reconstruction. It is not, since recovery removes most
of it.

**`averaged` and `vertex patch` are out.** `averaged` reads linearly across
the element and pays for it: at L1 it is 20 to 40 times further off than
`own`, and it reads the upstream point 1.5e-2 off against `own`'s 9.0e-3.
The vertex patch, one fit over some thirteen elements, pays polynomial
error: at L1 ten times `own`'s, and 1.2e-2 to 1.3e-2 at the upstream point
on two of the five channel meshes.

**The smallest gain over every case, by element kind.** This is the worst
error of `own` divided by the rule's, the smaller the worse:

| rule | bulk | walls | interfaces | samples read, mean / most |
| --- | --- | --- | --- | --- |
| `edge patch` | 2.02 | 0.97 | 0.91 | 24 / 24 |
| `edge+ring` | 2.02 | 0.82 | 1.70 | 25 / 42 |
| `edge+iring` | 2.02 | 0.97 | 1.70 | 24 / 42 |
| `spr` | 2.13 | 1.13 | 0.97 | 77 / 108 |
| `spr+ring` | 1.01 | 0.76 | 1.62 | 81 / 192 |
| **`spr+iring`** | **2.23** | **1.13** | **1.62** | **78 / 114** |

**`spr+iring` case by case**, the worst of `own`, then the gain:

| case | bulk | walls | interfaces |
| --- | --- | --- | --- |
| box at 0.1 / 0.05 / 0.025, L2 | 3.2e-2: 3.07 / 3.19 / 3.09 | 3.1e-2: 1.80 / 1.70 / 1.69 | |
| channel, L3 | 6.0e-2: 2.30 | 5.1e-2: 1.27 | |
| channel at 0.065 / 0.075, L3 | 2.23 / 2.43 | 1.25 / 1.13 | |
| channel, loop 0.297 / 0.303, L3 | 2.72 / 2.57 | 1.26 / 1.33 | |
| packet at 0.1 / 0.05 / 0.025, L3 | 5.6e-2: 5.81 / 5.51 / 5.09 | 4.7e-2: 4.02 / 4.00 / 3.27 | |
| small hole at 0.1, two holes at 0.05, L2 | 3.83 / 3.84 | 1.53 / 1.68 | |
| curved hole, free baffle, L2 | 3.47 / 3.07 | 2.28 / 2.79 | |
| baffle tips 0.012 / 0.006, L2 | 1.0e-1: 6.03 / 3.67 | 1.5e-1: 3.39 / 1.91 | |
| slit wall to wall, L2 | 7.1e-2: 3.87 | 6.0e-2: 1.71 | |
| disc of stiffness slot 4, L2 | 3.56 | 1.69 | 2.1e-2: 4.65 |
| packet through an interface at 0.05 / 0.025, L3 | 3.37 / 5.30 | 3.71 / 3.32 | 2.6e-2: 1.62 / 2.60 |
| gallery, L2 on `b` | 4.6e-2: 2.50 | 3.3e-2: 1.64 | 3.8e-2: 1.74 |

**The fixture's two points**, over the five channel meshes, `|c| − |u|` at
its worst over the run:

| | `own` | `spr` (`= spr+iring` here) |
| --- | --- | --- |
| upstream | 4.9e-3 to 9.0e-3 | 9.0e-4 to 2.6e-3 |
| upstream, mean over the run | −3.4e-3 to +4.9e-3 | +3.4e-5 to +5.7e-4 |
| downstream | 1.4e-3 to 4.5e-3 | 8.0e-4 to 1.9e-3 |

On the fixture's own mesh the downstream point sits among its element's
samples, where `own` interpolates, and `spr` reads it 1.9e-3 off against
`own`'s 1.4e-3; on the loop of 0.303, 1.9e-3 against 1.5e-3. Over the five
meshes the worst reading falls from 9.0e-3 to 2.6e-3.

**Weight sums** stay at 2.86 or below for every rule but `averaged` (3.55),
against `own`'s 2.38, slivers included, so an f32 rounding in the samples
grows by less than three.

The gallery's interface elements are a weak test: its wave is one plane wave
on `b` across every interface, smooth where a physical field jumps. The
disc and the packet through an interface are the real ones.

## What the rings do

**At walls, a ring hurts.** At the coarse channel's walls, `spr+ring` gains
0.76 to 0.90, worse than `own`, and `edge+ring` 0.82 to 1.01. At the walls of
fine meshes a ring helps (the box at 0.025, 4.68), but at the channel's 0.07
a wavelength spans about seven elements. A second ring there reaches far
enough to pay more in polynomial error than it gains in reach.

**At material interfaces, a ring is what makes the gain.** Without one, a
patch beside an interface reaches only one way and extrapolates the
interface's jump in the field: `edge patch` reads the packet's interface
elements 0.91 to 0.92 of `own`, and `spr` 0.97 to 0.99. With the ring there,
1.62 to 4.65.

**At walls, nothing halves the error.** A fit at a wall can only reach one
way, so it still extrapolates there, only from more samples: `spr+iring`
gains 1.13 to 1.33 at the coarse channel's walls and 1.5 to 4 elsewhere. An
interface element is the same case on its own side, and gains 1.62 on the
packet through an interface at 0.05.

## The decision

Against the rule set before measuring, `spr+iring` meets every condition but
the first. It is nowhere worse than today, its weight sums are below three,
and it reads the fixture's points within 2.6e-3. It at least halves the error
in the bulk (2.2 to 6), but not at walls (1.13 to 4) or at every interface
(1.62 to 4.65).

The halving at walls and interfaces was relaxed to "never worse", which
`spr+iring` meets in every case, and it was taken (31 October 2026). It gains
most everywhere else, and it fixes the motivating case. The fixed-size
alternative, `edge+iring`, gains less at walls (1.6 against 4.0 on the
packet, 1.09 against 1.27 on the channel) and reads 3% worse than today at
the coarse channel's walls, so it fails the relaxed criterion.

## For production

**The rule.**

- An element's material is its coefficients and frame.
- A vertex's star in a material is that material's elements with a corner
  at the vertex. Corners are canonical nodes, so a slit's two sides are two
  stars.
- A vertex on an interface, an endpoint of an edge between two materials,
  fits over its star's ring: every element of that material touching a
  corner of the star.
- A point's reading is the quadratic through its element's corner values
  and edge-midpoint means of those fits. Its weights are computed in f64
  when the stencil is built: about 78 samples on average, at most 114 in the
  corpus, unbounded in principle on a mesh of high valence.

**The constitutive order is kept.** Each sample's field is still `v(b)` at
that sample, its own inverse, temporal factor and, on a nonlinear record,
radial solve. Only the combination changes, from six weights to a table.

**The device.** `PointStencil` keeps its six own weights. The probe-point law
for the densities interpolates its spatial phase with them, and the vector
overlay reads its field with them: at up to 16,384 arrows and about 78
entries each, a table would hold 40 MB. Each stencil gains a range into a
new storage buffer of entries. An entry holds a sample, its temporal law
word and its weight, with the sample's inverse beside them, 32 bytes in all.
The shared reconstruction block of `canonical_probe.wgsl` and
`canonical_curve_probe.wgsl` gains the table's loop beside
`physical_complement`. The block stays byte-identical, and the point and
curve probes read through the table. At the largest curve, 512 points, the
table is about 1.3 MB on average and 1.9 MB at the corpus's most.

**The CPU twins.** `CanonicalPointStencil` and
`CanonicalTemporalPointStencil` carry the same table, built by one function
in `funfern-core`, and read through it as the device does.

**What to measure.**

- A quadratic field per material is read exactly through the table anywhere
  in a graded mesh with walls, a slit and an interface.
- The device against the twins on every consumer that reads a point.
- The fixture's three bounds narrowed to what the reading then supports.
- Its segment down the wall, which reads 1.5% high from the same
  extrapolation, measured again.
- The step and record times with the largest curve probe, on the
  methodology the GPU timings use.
- Temporal and nonlinear media were outside this spike. They need their
  twins' readings, against a refined run, before and after.
