# A fourth-order step for the triangle solver (M0), 4 October 2026

**Status: done, 4 October 2026.** Stages A (the design check), B (both CPU
paths, with tests) and C (the device) are in, and the fourth-order step is the
production integrator on CPU and device alike. M0 comes out of the IGA spikes ("Spike T" in
[the feasibility report](funfern-iga-feasibility-spike.md)): the
modified-equation Störmer step cut the spline patch's temporal error 50 to
60 times, and the triangle solver at its own operating point is in the same
position, its leapfrog temporal error seven times its spatial one. Stage A
asked whether the correction composes with everything the production step
holds: implicit wall kicks, loss stages, prescribed nodes, sources, gaps,
time-driven coefficients, field laws, restoring laws and handoffs.

## The scheme

The production step is kick, drift, kick on `(Q, b)`, with Strang loss
halves around it. The fourth-order step keeps that composition and changes
one force or one field:

```text
drift form   the drift reads   ũ = u − dt²/12 · A (L u − ṡ)
kick form    each kick uses    F̃ = F + dt²/12 · L_b u̇,   u̇ = M⁻¹ (s − F)

A    ∂u/∂Q at the drift's instant: the inverse mass, or a mass law's inverse tangent
L    the stiffness of everything that drifts: the bulk K through each sample's
     tangent J_b, the gap springs, the restoring law's m₀ V″(r)
s    the source rate
```

For a linear generation both forms are Störmer under the same modified
stiffness `K (I − dt²/12 · M⁻¹ K)`: fourth order in time, stable while
`ω dt < 2√3`, and the step stays at the app's 0.9 of the leapfrog bound,
so the bound has √3 of headroom.

Each form is the exact gradient of a modified store when the *other* side is
quadratic, so each stays symplectic there:

- the drift form is the gradient of `E_Q − dt²/24 · uᵀ L u`, exact while `L`
  does not depend on what the drift moves; any mass-side law is fine;
- the kick form is the gradient of `U − dt²/24 · Fᵀ M⁻¹ F`, exact while the
  mass is linear; any stiffness-side or restoring law is fine.

The long runs below confirm this to the letter: a stiffness-side law leaks
energy secularly under the drift form and holds under the kick form, and a
mass-side law does better under the drift form.

## Measurements

Two harnesses. `examples/canonical_fourth_order.rs` runs the reflecting-box
mode of `wave_convergence` on the production fixed CPU path (P2e, parent
edge 0.08, 9,215 dofs), reading the phase by a least-squares fit over one
period that needs no prediction of the discrete frequency. Two ignored tests
in `canonical_temporal.rs`, `measure_fourth_order_against_each_composition`
and `measure_fourth_order_long_run_energy`, run every composition on the
driven CPU path (edge 0.2, about 1,400 dofs). The reference for each
composition is a Richardson extrapolation of the leapfrog at 1/64 and 1/128
of the step, which does not depend on the scheme under test.

### The box mode at the app's step

| integrator | phase at t = 10 | amplitude | simulated s per wall s |
| --- | --- | --- | --- |
| leapfrog, 0.9 dt_max | +5.47e-2 rad | +1.6e-4 | 13.0 |
| fourth order, 0.9 dt_max | −9.147e-3 rad | −3.3e-5 | 8.8 |
| fourth order, 0.225 dt_max | −9.126e-3 rad | −3.3e-5 | 2.2 |

The leapfrog's +5.47e-2 is +6.39e-2 temporal against −9.1e-3 spatial, the
spike's estimate exactly. The fourth-order step's temporal part at the app's
step is −2.1e-5 rad, the `(ω dt)⁴/720 · ωt` prediction to two digits: the
step is now spatial-error limited, six times more accurate overall. The CPU
fixed path pays 1.47× per simulated second, because there the correction
adds a gradient and a gather to a step that has one of each. The device
cost is stage C's question; its linear path already keeps an assembled
stiffness row for the force cache, which makes `L u` one node pass.

### Every composition

Relative error of the field at t = 1 against the reference, at the app's
step, smooth data (cosine modes of the box, nothing near the grid scale).
The drift and kick columns are the two forms.

| composition | leapfrog | drift form | kick form |
| --- | --- | --- | --- |
| bulk | 6.1e-3 | 4.4e-4 | 6.4e-4 |
| first-order wall | 3.1e-2 | 3.3e-3 | 3.3e-3 |
| second-order wall | 3.0e-2 | 3.1e-3 | 3.1e-3 |
| pinned trace behind a second-order wall | 3.2e-2 | 3.0e-3 | 3.1e-3 |
| prescribed signal only, from rest | 1.2e-2 | 1.2e-3 | 2.9e-3 |
| source only, from rest | 3.2e-3 | 2.4e-4 | 4.8e-4 |
| thin gap | 1.4e-3 | 2.1e-4 | 2.2e-4 |
| constant loss | 5.8e-3 | 4.2e-4 | 6.1e-4 |
| pumped mass | 4.7e-3 | 2.7e-4 | 3.7e-4 |
| pumped stiffness | 5.8e-3 | 4.4e-4 | 6.1e-4 |
| Kerr, mass side | 4.6e-3 | 5.9e-4 | 2.0e-3 |
| Kerr, stiffness side | 4.6e-3 | 5.1e-4 | 7.9e-4 |
| saturable, stiffness side | 5.0e-3 | 3.9e-4 | 6.5e-4 |
| Kerr mass and saturable stiffness | 2.4e-3 | 3.2e-4 | 1.0e-3 |
| Klein-Gordon | 3.1e-3 | 4.4e-4 | 8.2e-4 |
| sine-Gordon | 3.0e-3 | 3.6e-4 | 6.6e-4 |
| φ⁴ | 2.3e-3 | 4.9e-4 | 7.1e-4 |
| van der Pol with Klein-Gordon | 1.9e-3 | 6.1e-4 | 6.9e-4 |
| authored short-wave loss | 2.7e-3 | 1.7e-3 | 1.7e-3 |

Data with a Neumann mismatch at the walls, which fills the spectrum to the
grid scale, gives the same picture with larger numbers: 7 to 19 times better
than the leapfrog at the app's step everywhere except the two short-wave
rows. The energy ledgers' splitting residual stays at the leapfrog's size in
every row.

Three things the table does not show directly:

- **The state error at a fixed time converges at second order** for both
  forms once the step is small. Any kick-drift-kick step carries a bounded
  `O(dt²)` offset between its kicked and drifted variables, a processing
  error that does not grow; the phase, which does grow, is fourth order.
  The box mode separates the two: its amplitude does not move with the step
  and its phase is fourth order.
- **The short-wave rows are first order under every integrator.** Van der
  Pol's automatic short-wave viscosity and the authored short-wave loss are
  applied once after the second kick on the drift's field, forward Euler in
  their own subflow, as plan.md "Worth checking sometime" already records.
  They bound what any integrator gains there to about 1.6 to 3 times. Fixed
  after stage D: see "The short-wave split" below.
- **Forcing needs its own quadrature.** The kicks integrate a source rate by
  the trapezoid at the step's ends, an `h³/12 · s″` error per step that holds
  every integrator at second order with the leapfrog's constant. Applying
  `s − (s⁺ − 2s + s⁻)/12` at each kick and keeping `ṡ` in `ũ` takes the
  source-only error from 2.65e-3 to 2.4e-4; either alone gives 7.8e-4 or
  2.4e-3.
- **A prescribed node reads its signal expanded as a free node's field is.**
  A free node's `ũ` is the predictor `u(t½) − h²/8 · ü` plus the correction
  `h²/12 · ü`, so it stands `h²/24 · ü` below the midpoint field; under the
  kick form the drift reads the predictor itself, `h²/8 · ü` below. A pin
  reading `g − h²/24 · g″` (drift form) or `g − h²/8 · g″` (kick form), the
  second derivative as the second difference over the step, is consistent
  with its neighbours. Stage A tried the opposite signs, `+h²/12` and
  `+h²/24`, which were worse than the plain `g`, and kept the plain value;
  stage B derived the sign and measured it on the test mesh (edge 0.3):

  | pinned node reads | drift form | kick form |
  | --- | --- | --- |
  | `g` | 2.5e-3 | 6.2e-3 |
  | `g − h²/24 · g″` | **1.9e-3** | |
  | `g − h²/8 · g″` | | **3.6e-3** |
  | `g + h²/24 · g″` | 3.9e-3 | |
  | `g + h²/8 · g″` | | 1.1e-2 |

  against the leapfrog's 1.8e-2: 9.5 and 5 times better. Still second order:
  a continuously generated wave carries the bounded offset. The leapfrog's
  own pin has the same `h²/8` mismatch, and would gain from it too.

### Long runs

Relative energy change at the app's step, no loss, from data with grid-scale
content, which is where a non-symplectic step leaks first.

| composition | leapfrog t = 160 | drift form t = 40 / 160 | kick form t = 40 / 160 |
| --- | --- | --- | --- |
| bulk | −1.3e-3 | −7.8e-4 / −7.3e-4 | −1.5e-3 / −1.4e-3 |
| Kerr, mass side | +2.8e-1 | −1.2e-3 / +5.4e-2 | +1.0e-3 / +9.6e-2 |
| Kerr, stiffness side | +6.1e-2 | **−4.4e-2 / −3.8e-1** | −1.7e-4 / +9.9e-3 |
| saturable, stiffness side | +2.0e-2 | **−1.8e-2 / −2.1e-1** | −7.5e-4 / +1.4e-2 |
| Kerr mass and saturable stiffness | +3.8e-2 | **−2.9e-2 / −3.8e-1** | −1.1e-4 / +1.9e-2 |
| sine-Gordon | −3.4e-4 | −4.7e-4 / −5.5e-4 | −6.7e-4 / −3.5e-4 |
| φ⁴ | −7.3e-4 | −4.1e-4 / +8.2e-5 | −5.5e-4 / −3.8e-4 |

The drift form leaks under any stiffness-side law, steadily, as a frozen
tangent inside the drift predicts. The kick form holds, below the leapfrog,
even with laws on both sides, where neither form is exact. Sine-Gordon and
φ⁴ freeze `V″(r)` in the drift form too and do not leak over this run; the
kick form is the exact one for them. The pumped mass was also run: its energy
grows by parametric gain, identically under all three.

### Handoffs

The box mode handed to a fresh stepper every 25 steps, alternating the step
between 0.9 and 0.675 of the bound as a live drag's generations might. The
fourth-order step's amplitude moves by 1.1e-4 and its phase by 2e-4 rad, the
leapfrog's amplitude by about 1.9e-4. The offset does not accumulate (t = 2
and t = 10 agree), so no handoff processing is needed.

The same harness at an alternation every 4 steps grows the leapfrog's energy
by a factor of 1e9 by t = 10, and the fourth-order step's by about 4, in
modes other than the box mode: two maps with different invariant ellipses
alternated quickly pump the grid-scale modes parametrically. Pre-existing,
and not reachable in the app as far as the pacing goes: a generation's step
is the smaller of its recommended step and the frame's, which changes only
when the stiffest row does. Filed under "Worth checking sometime".

## The design for stages B to D

- **Which form.** The kick form for a generation with any stiffness-side
  field law (in either skin's sense: a magnetic law in TM, an electric one in
  TE, a stiffness law in Mechanical), alone or beside a mass-side law. The
  drift form for everything else, linear generations included: it is the
  more accurate of the two there, and the cheaper on the device. Restoring
  laws stay in whichever form the field laws pick.
- **Sources.** The fourth-order kick quadrature
  `s − (s⁺ − 2s + s⁻)/12` in both forms, and `ṡ` in the drift form's `ũ`
  (the kick form's `u̇` carries `s` already).
- **Prescribed nodes.** The drift reads `g − h²/24 · g″` in the drift form
  and `g − h²/8 · g″` in the kick form; the kick form's `u̇` there is the
  signal's central difference.
- **Unchanged.** Loss halves, wall kicks (the kick form passes `F̃` where
  they take `F`), short-wave viscosity on the plain midpoint field, the step
  size, handoffs, the energy ledger's terms.
- **No switch.** The fourth-order step replaces the leapfrog in production
  on every generation; `CanonicalIntegrator::Leapfrog` stays for tests and
  comparison only. Nothing in the document or the UI changes.
- **Stage B, done.** `CanonicalIntegrator::{Leapfrog, FourthOrder}` on both
  CPU states; `FourthOrderForm::{Drift, Kick}`, picked by
  `CanonicalTemporalWaveOperator::fourth_order_form` from whether any
  complementary sample carries a field law (the fixed path has none, so it
  is always the drift form). Tests:
  - every composition test that checks its energy ledger converges at
    second order now checks it under both integrators (the helper behind
    ten of them, and the temporal-work residual);
  - an inert driven generation steps exactly as the fixed path does under
    both integrators, with walls, a pinned wall and a source;
  - the signed step is the exact inverse under both forms;
  - from rest, both forms converge at order 4.2 then 4.0, 25 times below the
    leapfrog at the app's step, and on a linear generation they are the same
    step to 1e-9 relative;
  - a source and a prescribed signal from rest: at least 8 and 7 times below
    the leapfrog under the drift form, 5 and 4 under the kick form;
  - the form follows the field laws in every skin (a TE electric law sits on
    the complementary row and takes the kick form);
  - a stiffness-side Kerr medium over t = 40 at the app's step: the kick form
    keeps its energy to 7e-4, the drift form loses 3.5e-2.
- **Stage C, done.** `canonical_wave.wgsl` and `canonical_gpu.rs`. The device
  plan takes its integrator from the CPU state it is compiled from, and its
  form from the driven state, so a device and its reference always step
  alike. Two control flags (256 the fourth-order step, 512 its kick form)
  and one scratch word per node and per sample after the secant words, which
  a linear-loss patch's flag reset now keeps.
  - The drift form is three passes before the drift: each node's midpoint
    field, cached once so a drive's mass or a field law's inverse is not
    evaluated at every sample; each sample's `J η C u`; each node's `ũ`, with
    the gap springs, the restoring curvature at the step's `r`, `ṡ` and the
    pin's `h²/24` expansion. A fixed generation with a force cache takes
    `K u` from its assembled row and returns from the first two at once. The
    drift, `r`, the gap jumps and the force cache read `ũ`; the short-wave
    stress keeps the plain gradient.
  - The kick form is two passes before each kick, after the secants: each
    node's force and `u̇`, each sample's `J_b η C u̇` from the cached secant,
    whose `r/(j|b|)` gives the radius back without a second solve. Every
    kick, the walls' included, reads the force through `kick_force` and the
    source through `step_source`, which carries the Simpson correction in
    both forms.
  - The suite: every canonical GPU example at its default (23 runs) and
    across its modes (107 runs: the nonlinear gate's nine, the oscillator's
    seven media under nine compositions and the filter, the long run, the
    driven document, both handoffs, the failure gate, the consumers, the
    forced compositions), all within their gates, worst Q 1.2e-5 and b
    1.5e-5 on the remeshing oscillator handoff, typically 1e-6. With the
    device forced to the leapfrog against the fourth-order reference the
    same examples fail at 1e-3 to 6e-2, so the gates see the integrator.
  - Throughput (`canonical_gpu_temporal_timing`, 15,264 dofs, M1 Max, 2,000
    steps):

    | generation | leapfrog | fourth order | cost |
    | --- | --- | --- | --- |
    | fixed | 400 µs | 426 µs | 1.07× |
    | driven | 395 µs | 425 µs | 1.08× |
    | sine-Gordon | 400 to 426 µs | 443 to 450 µs | 1.05 to 1.11× |
    | Kerr and saturable, kick form | 1007 µs | 1194 µs | 1.19× |

    A first cut cost 1.37 to 1.46×: it evaluated a driven mass at each of a
    node's samples, scanned every force entry for gaps on the force-cache
    path, always formed the drift's plain gradient, and solved each sample's
    radius twice.
- **Stage D.** The fourth-order step is the default on both CPU states. Tests
  that are about the leapfrog's own recurrence (the scalar-recurrence
  parities, velocity Verlet on the Klein-Gordon uniform mode) name it; the
  Klein-Gordon one has a fourth-order twin, exact to 1e-11 against the
  modified oscillator `ω̃² = (1 − (ω₀h)²/12) ω₀²`. Two magnitude bounds moved
  with the step's breathing about its conserved store: the Kerr bulk's
  energy, 2.3e-4 against the leapfrog's 1.9e-4 at the same second order, and
  the short-wave loss's closure, now held to the bare step's own breathing.

## The short-wave split

Done after stage D, on 2026-10-04, before the branch was merged. The
short-wave viscosity (van der Pol's automatic one and the authored
short-wave loss) was read once on the drift's midpoint field and applied
after the second kick: forward Euler in its own subflow, and a Lie step
against the core, so first order under either integrator.

Two repairs that do not work, both measured:

- **A predictor alone.** Reading the force at `u(Q − h/2 · F)` makes the
  viscosity second order against itself, but its whole impulse still lands
  after the second kick while the drift integrates a field without it, so
  `b` takes an `h²` error a step. The composition table's short-wave rows
  stayed at order 1.0 (1.65e-3 at the app's step). The rate-against-
  trajectory test cannot see this; it checks one step's consistency.
- **Half before the drift, the rest after the second kick.** Second order,
  but the viscosity then sits inside the kick-drift-kick core and breaks the
  core's energy identity: the ledger's residual grows as `dt²` and reaches
  30% of the short-wave loss on a mesh-ceiling mode at the app's step,
  against 1e-5 of it before. The same under both integrators.

What is in: the viscosity is split about the core as the loss stages are,
outside it, in two halves of different kinds:

- before the first kick, a forward Euler half on the field the core starts
  from, `Q − h/2 · F(Q)`;
- after the second kick, a half that predicts before it applies,
  `Q − h/2 · F(Q − h/2 · F(Q))`.

On the viscosity alone the pair is `1 − x + x²/2`, the exponential to second
order. With the core between them the composition matches the exact flow to
`h²`: writing `C` and `V` for the two generators, both halves have the same
first-order term `−V/2`, the second has `V²/4` as its own second-order term,
and the products collect to `(C − V)²/2`. Each half reads the
complementary map's drive at its own instant, the step's start and end.

| short-wave rows, at the app's step (0.9) | before | split |
| --- | --- | --- |
| short-wave loss, smooth, drift form | 1.7e-3 (order 1.0) | 1.5e-5 (order 2.0) |
| short-wave loss, rough, drift form | 1.6e-2 (order 1.0) | 6.3e-4 (order 2.0) |
| van der Pol, smooth, drift form | 6.1e-4 (order 1.1) | 4.6e-4 (order 2.0) |
| van der Pol, rough, drift form | 1.1e-2 (order 1.0) | 2.7e-3 (order 2.0) |

The leapfrog gains the same order: the smooth short-wave row goes from
2.7e-3 to 1.0e-3. Van der Pol's smooth row moves little because its Bernoulli
loss stages, not the viscosity, dominate there. The ledger closes as it did:
on the mesh-ceiling mode the residual is 6e-13 of a 4.8e-9 loss at 0.2 of
the bound and 8e-15 at 0.9.

On the device the stress no longer rides the drift and the second kick. The
first half is three passes after the loss stage (each node's field, each
sample's stress over its drive, each node's gather and update) and the
second five after the second kick and its wall (field, stresses, the
prediction, stresses, update), on a word a node after the fourth-order ones.
The first half writes the flux the first kick reads, which the kick takes
from the loss stage's copy, so a generation with a short-wave loss and no
other now runs the loss stages too, and the live loss patch keeps them on
there. The suite: 23 default runs and 107 mode runs, all within their gates,
the short-wave modes at 3e-7 to 2e-6.

Throughput (`canonical_gpu_temporal_timing`, 15,264 dofs, M1 Max, 2,000
steps, five rounds alternating the commit before and the split, medians):

| generation | before | split | cost |
| --- | --- | --- | --- |
| driven, no short wave | 457 µs | 446 µs | unchanged |
| van der Pol | 831 µs | 969 µs | 1.16 to 1.23× |
| short-wave loss 0.5 | 621 µs | 918 µs | 1.47 to 1.62× |

The short-wave work itself goes from about 170 µs a step over the driven
scene to about 470: the split reads the viscosity three times a step where
the old one read it once, in eight passes of their own, and the short-wave
loss generation now pays the loss stages too. Generations without a
short-wave term do not see it.

