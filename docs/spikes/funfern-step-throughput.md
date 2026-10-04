# Step throughput: where a canonical GPU step spends its time

Done on 2026-10-04 on `step-throughput`, after the short-wave split cost
1.16 to 1.62× a step. The request was to merge passes and take any other
lossless gain. Everything below is on an M1 Max with
`canonical_gpu_temporal_timing`: 15,264 dofs, 30,072 samples, the
fourth-order drift form unless the fixture is field-dependent.

## Merging passes buys little

A dispatch that returns at once costs 2 to 3 µs within the step's one compute
pass, whether it has 120 workgroups or one: ten such dispatches a step added
32 µs to the driven fixture, forty added 80. Merging the split's eight passes
into four or five would have saved 10 to 15 µs of the 460 it cost.

## Per-pass profile

Each pipeline was dispatched four extra times wherever the step dispatches it,
and the slope read as its cost. A pass that is not idempotent still costs the
same to repeat, and a run that failed would have read faster, not slower.
Driven fixture, steady round (step 417 µs):

| pass | µs |
| --- | --- |
| `kick_first` | 138 |
| `kick_second` | 140 |
| `fourth_order_nodes` | 51 |
| `drift` | 12 |
| `fourth_order_fields`, `fourth_order_samples`, `stage_accounting` | 6 to 8 each |

On the short-wave fixture the split's three gathering passes cost about 36 µs
each, the two loss stages it forced on about 50 each, its field and stress
passes 3 to 16.

## What the kicks spent

- **The drive's factor, once an entry.** A time-driven kick's gather divided
  each entry by its sample's factor, evaluated there: a cosine, a phase
  reduction and dependent table reads, about seven times per sample per
  kick. With that evaluation skipped (timing only) the driven step fell from
  417 to 260 µs.
- **Not the `b` reads** (about 7 µs a kick) and not the empty gap-spring walk
  on the fixed fixture (nothing measurable).
- **The table's memory pattern.** After the change below, the two kick
  gathers are 124 µs of a 380 µs step (removing them reads 255). A dummy
  gather over the same addresses, kept alive by an impossible compare, reads
  the same 380, so the gather's arithmetic is negligible and its cost is the
  reads. The same dummy over interleaved table addresses, entry `k` of node
  `n` at `k·N + n`, reads 318: half the gathers' cost. Its `b` indices were
  arbitrary, so a real interleaving, which keeps the mesh's locality, should
  do no worse.

## What changed

- **Each sample's factor once a kick.** The secant passes that a
  field-dependent generation already ran before each kick now run on every
  time-driven one, and the gather reads their word. For a linear record
  `temporal_complementary_secant` is `1/factor` at the same instant, the
  value the gather computed, so nothing moves. Two dispatches on time-driven
  generations without field laws, no new words.
- **Node masses: nothing to do.** The production step evaluates a node's
  mass once per node per pass; the per-entry evaluations sit on the fixed
  force cache's rows, where the mass does not move, and on the leapfrog's
  drift, which no longer runs in production.
- **The short-wave split's first half applied by the first kick.** The kick
  reads the flux the first half starts from and applies it before its own
  update, one gather pass fewer. Nothing between the two passes reads `Q`:
  the secants and the kick form's rates and tangents read `b`, the gathered
  force and the rates. With that, a generation whose only loss is the
  short-wave one no longer runs the loss stages, which at zero loss left
  every value as it was. Seven passes a step.
- **Merging the field passes into the stress passes** was not done: each
  sample would evaluate seven nodal masses where the pass evaluates one a
  node, the pattern the first change removes.

## Held to the same bits

`canonical_gpu_temporal_timing --checksum` runs exactly 2,000 steps, reads the
whole state back and hashes the bits of `Q`, `b`, `r` and the accounting
lanes. Two runs of one build agree on every fixture. Every fixture gives the
baseline's hash after both changes: fixed, driven, second-order wall, Kerr and
saturable with and without that wall, sine-Gordon, van der Pol with and
without Kerr, constant loss, gated, the resident filter, the short-wave loss
alone, on the second-order wall and with Kerr. The device suite (23 default
runs and 107 mode runs) passes.

## Throughput

`--steps=20000`, three rounds with the order of the three builds rotated,
medians, µs a step (two outliers, 645 and 767, each the first run of a round
after the heaviest fixture):

| fixture | before | factor once | and the split | ratio |
| --- | --- | --- | --- | --- |
| fixed | 475 | 377 | 380 | 0.80 |
| driven | 468 | 378 | 373 | 0.80 |
| van der Pol | 1035 | 942 | 931 | 0.90 |
| short-wave loss | 914 | 809 | 692 | 0.76 |
| Kerr and saturable, kick form | 1794 | 1650 | 1652 | 0.92 |
| short-wave loss with Kerr | 4848 | 4818 | 3270 | 0.67 |

The fixed fixture gains as much as the driven one though none of its passes
changed: it never evaluated a factor. The likely reason is the kick's
register use, which the inlined drive evaluation raised for every
generation whether it ran or not; this was not measured. The last row's gain
is the loss stages it no longer runs, whose energies take a field law's
solves.

## Interleaved entry tables

Agreed and done the same day. Every gather walked its node's entries from a
contiguous range, so the threads of a SIMD group read 32 separate stretches
of the table an iteration. Both per-node tables, the force entries
(`ranges.xy`) and the stiffness rows the fixed plan's force cache walks
(`stiffness.xy`), are now packed by `pack_entry_slices` in slices of 32
nodes, entry `k` of each node side by side; a range is the node's first word
and its count, and a loop steps `ENTRY_SLICE` words. Each node still visits
its entries in order, so every sum keeps its bits. Nine loops walk the force
entries and three the rows; the source entries stay contiguous.

The app's plain scenes run a different plan from the timing fixture above:
`compile_with_quadratic`, with the force cache, whose drift and
`fourth_order_nodes` walk the stiffness rows. Profiled the same way through
`canonical_gpu_timing` at the same mesh (step 211 µs): the drift 55 µs,
`fourth_order_nodes` 61, each kick 40, the two small passes 5 each. The
kick's 40 was the gap-spring walk over every force entry of a scene with no
gap springs, which found nothing and returned zero; it is now skipped when
the generation has none, the same zero.

Held to the same bits: both timing examples' hashes match the commit before
on all 14 temporal fixtures and on 14 plain ones (`canonical_gpu_timing`
prints the hash on every run now: the default box, both walls, loss, thin
gap, obstacles, forcing with and without the second-order wall, the filter,
pulse, law patch and maintenance events, the periodic filter and a clock
rebase). At 2,000 steps the second-order wall exceeds that example's
accuracy gate on the commit before as after (`b` 4.5e-5 against 3e-5, a gate
set for its default 128 steps, where it reads 7e-7).

The padding grows the tables from 5.75 to 7.09 MiB on the 15,264-dof mesh,
the generation's buffers from 14.5 to 15.9 MiB.

`--steps=20000`, three rounds alternating which build runs first, medians,
µs a step:

| fixture | before | after | ratio |
| --- | --- | --- | --- |
| plain box | 211 | 147 | 0.70 |
| plain, thin gap | 246 | 201 | 0.82 |
| plain, second-order wall | 377 | 309 | 0.82 |
| plain, forcing | 298 | 241 | 0.81 |
| temporal, no drive | 384 | 338 | 0.88 |
| driven | 383 | 336 | 0.88 |
| van der Pol | 955 | 878 | 0.92 |
| short-wave loss | 692 | 618 | 0.89 |
| Kerr and saturable, kick form | 1672 | 1626 | 0.97 |
| short-wave loss with Kerr | 3328 | 3298 | 0.99 |

The thin gap still walks its springs, so its 0.82 is about what the layout
alone gives a plain scene; the box's 0.70 adds the skipped walk. The
temporal fixtures gained about 47 µs, short of the 60 the dummy gather
predicted for the kicks alone; why was not measured. The field-dependent
fixtures gain least, likely because their steps are mostly the field laws'
solves, which the layout does not touch; also not measured.
