# The outgoing wall under the step, 10 October 2026

**Status: stage A done, 10 October 2026.** A spike in
`experiments/material-laws-spike/` with no production change. Its follow-up
the same day split the update as the trapezoid rule (see "The update") and
took in the kick form. Stages B (both CPU paths) and C (the device) are
planned separately.

## The question

Fixture `an_outgoing_wall_returns_what_its_law_says_between_free_and_pinned_sides`
measures a channel's second transverse mode, 1.6 to 2.2 Hz, meeting an
outgoing wall at 27° to 39°. At the app's step, 1.2 of the leapfrog bound, the
second-order wall returns 1.098 of its passive law's energy; at 0.6 of the
step 1.028, at 0.3 1.015, at 0.15 1.012, so about 1.01 is the mesh's and the
rest is the step's. The leapfrog at 0.9 reads 0.911 instead. The first-order
wall reads 1.002 to 1.005 throughout. The fourth-order step's spike left the
wall kicks as they were, the scattering spike's force-coupled Cayley update,
and its own table already showed the walls an order of magnitude behind the
bulk.

## The harness

`wall_step.py` takes the scattering spike's Bloch strip - one row of the
enriched triangles, Bloch-periodic across, driven at one end, the wall at the
other - and steps it with an explicit one-step map assembled from its stages:
the kicks, the drift with its corrected field, the wall update. A harmonic
solution of that map, fitted to the strip's discrete propagating modes, gives
the reflection of any candidate without deriving its harmonic equation by
hand. It reproduces the scattering spike's closed-form leapfrog response to
1e-13. The coarse 2D mesh of the boundary spike, behind second-order walls,
checks stability, energy and a pulse in the time domain.

A structured strip of cell 0.025 is about as stiff as the fixture's mesh: its
leapfrog bound is 5.46e-3 s against the fixture's 4.517e-3 s. At the
fixture's own steps it reproduces the fixture:

| | strip, 1.6 / 1.9 / 2.2 Hz | strip mean | fixture |
| --- | --- | --- | --- |
| production, 1.2 of the bound | 1.025 / 1.066 / 1.138 | 1.077 | 1.098 |
| leapfrog, 0.9 of the bound | 0.972 / 0.928 / 0.854 | 0.918 | 0.911 |

## What it is

**A floor every outgoing wall has.** The step adds a reflection of its own,
1e-3 to 2e-3 of the incident amplitude at the fixture's step, about in
proportion to `ω·dt`. It is the same at every angle from 0° to 85° and the
same for both orders of wall. Against the second-order wall's law, 0.014 to
0.047 in this band, it reads as 7% to 14%; against the first-order wall's,
four times larger, it hardly shows in a magnitude. At normal incidence, where
either law reflects about 1e-5, it is the whole reflection: the fixture's
normal-incidence check reads 5.9e-6 of the incident energy, which is this
floor, 2.4e-3 in amplitude.

**Its order comes from the kick, its size from the wall's stiffness.** The
order is two because each half-kick integrates the wall by a Cayley step: a
Padé(2,2) kick, nothing else changed, makes the wall fourth order (measured
4.0 to 4.1). But the wall's rates are of the grid's order - the trace node's
damping is its line mass over its share of the area, and the second-order
wall's tangential poles reach `√(7/4)` times the trace operator's largest
root - so the wall's rate times the step, `w`, is about 1.1 to 1.3 at the
app's step. At `w` near one a higher order buys little: the Padé kick leaves
0.51 to 0.67 of the floor. An exact exponential kick does no better than the
Padé, so the rest is not how the kick integrates the wall. Under refinement
at a fixed CFL the floor falls only as `h`, both ways.

## The exact target

The fourth-order step's drift reads `ũ = u − dt²/12 · M⁻¹K u`, and a harmonic
solution of its interior satisfies `K ũ = ω_e² M u`, `ω_e = 2 sin(ω dt/2)/dt`.
Let `λ₁` be the smaller root of `λ − dt²λ²/12 = ω_e²`, the frequency the step
propagates, `ω²` to fourth order. Then exactly

```text
K − dt²/12 · K M⁻¹ K − ω_e² M  =  L (K − λ₁ M),     L = I − dt²/12 · (K + λ₁ M) M⁻¹
```

and `L` is invertible at every step the app takes, its zero being the other
root, above the spectrum. So the semidiscrete system at the propagated
frequency, `(K − λ₁M + W(s′)) u = 0` with the wall's admittance `W`, has the
same solutions as `L (K − λ₁M + W(s′)) u = 0`, whose interior rows *are* the
stepped interior rows. A step whose rows at and next to the wall are also
those rows reflects as the semidiscrete wall does, at any step. Expanding the
rows `L` touches:

- **Next to the wall** the drift must read `ũ* = u − dt²/12 · M⁻¹(K u + W u)`,
  which is `u + dt²/12 · ü` with the wall's own share of the acceleration.
  Production leaves the wall out of the trace nodes' `ũ`.
- **At the wall** the row must be `(K ũ*)_b − ω_e² M_b u_b + (ω_e²/λ₁) W(s′) u_b`.

For a wall update that takes `X_h = A X_n + B f_n` in the first half-kick and
`X_{n+1} = A′ X_h + B′ f_{n+1}` in the second (`X` the trace's field and the
wall's poles, `f` the trace's force), the harmonic wall row depends only on
the full-step propagator `AA′` and the total weight `B + AB′`. For one rate
`g`, `w = g·dt`, `z = s·dt`, production's two force-coupled half-step Cayleys
give

```text
𝒥 dt² = 4 (1 + w²/16) sinh²(z/2) − w sinh z
```

where the target needs `4 sinh²(z/2) − w z (1 + z²/12 + …)`: the damping is
right, the trace node's mass is off by `w²/16`, and since the damping term is
`w/z` times the mass term, that is an error of order `z·w` in the wall's
admittance - first order in `ω·dt` at a fixed CFL, as measured. Matching the
target's damping and mass for every `w` leaves one choice: the Cayley map
over the whole step, `AA′ = (I − dt G/2)⁻¹(I + dt G/2)`, with total weight
`−dt (I − dt G/2)⁻¹ E`. Then `𝒥 dt² = 4 sinh²(z/2) − w sinh z`: the
admittance is off by `z²/12`, `R` by half that, at every `w`. No one-step
rational map does better for every `w`: a higher-order propagator, Padé
included, puts an error back into the damping term.

## The update

Since only `AA′` and `B + AB′` matter, the update needs no square root of a
Cayley map, and there is more than one way to place it in the step. The
first found, the *wall-after* split, kicks the trace as the bulk in both
halves and takes the whole Cayley, `(I − dt G/2) X′ = (I + dt G/2) X`, after
the second kick. It reflects right, but what it stores at a step's end is
off from the field by a factor of order `w` at the trace (see "One caveat"
below), which every reader of the state at a step's end would see.

The adopted split is the trapezoid rule over the whole step, cut into its
two halves:

1. **First kick:** the trace takes the bulk's plain kick and the explicit
   half, `X_h = (I + dt G/2) X_n + dt/2 · E(s − F)`.
2. **Drift:** the drift form's `ũ` at the trace gains the wall's acceleration,
   `+ dt²/12 · M⁻¹ (G (G X_h + E(s − F_mid)))_Q`, with `F_mid = F + dt/2 · K u`.
3. **Second kick:** the plain kick, then the implicit half,
   `(I − dt G/2) X_{n+1} = X_h + dt/2 · E(s − F)`.

Its `AA′` and `B + AB′` are the wall-after split's, and the harness's
responses agree to 4e-10. What it stores is `X_n = (X_{h,n−1/2} + X_{h,n+1/2})/2`,
the mean of the two drift instants around it, exactly as the interior's `Q`
is, so the field at the trace reads as the field anywhere else. It is one
wall solve a step instead of two, today's solve at twice the duration, plus
one application of the generator in the first kick and two in the drift. It
is symmetric in time but for the drift's estimate of `F_mid`. The leapfrog
needs only steps 1 and 3: it has no `ũ`, and the full-step Cayley alone
leaves it `z²/8` in the admittance.

**The kick form.** A generation with a stiffness-side field law corrects the
kicks' force instead of the drift's field: `F̃ = F + dt²/12 · L u̇`, with
`u̇ = M⁻¹(s − F)` at every free node. Its linear limit, on the strip, needs
the same two things: the trapezoid split, and the wall's rate in the trace's
`u̇`, `M⁻¹(s − F + (G X)_Q)`, with `X` as each kick finds it - `X_n` for the
first, `X_h` for the second. Without the wall's rate it is 2.9 times worse
than production; with it, 0.37 to 0.42 of production's error at the app's
step and `z²/24` (to 5%) at small steps. That is about twice the drift form's
error at the app's step: the second kick reads the wall's rate half a step
early. Under production's half-step Cayleys the kick form reflects as the
drift form does, to 3e-7.

## Measurements

The temporal error, against the strip's own semidiscrete reflection, of the
second-order wall at 2.2 Hz, 27°, steps in units of the fixture's bound:

| update | 1.2 | 0.6 | 0.3 | 0.15 | order |
| --- | --- | --- | --- | --- | --- |
| production | 1.92e-3 | 3.42e-4 | 7.83e-5 | 1.91e-5 | 2.0 |
| Padé(2,2) half-kicks | 9.86e-4 | 5.69e-5 | 3.50e-6 | 2.18e-7 | 4.0 |
| **full-step Cayley, wall acceleration** | **3.51e-4** | **5.53e-5** | **1.17e-5** | **2.80e-6** | 2.1 |
| leapfrog, production | 3.70e-3 | 9.21e-4 | 2.30e-4 | 5.75e-5 | 2.0 |
| leapfrog, full-step Cayley | 2.62e-4 | 6.56e-5 | 1.64e-5 | 4.10e-6 | 2.0 |

At small steps the adopted update reads `z²/24` at normal incidence and the
leapfrog's `z²/16`, both as derived, to 1.3% and 0.002%. Under refinement at
1.2 of each mesh's own bound its error falls as `h²`, production's and the
Padé's as `h`. Over wavelengths 0.45, 0.625 and 1 and angles 0° to 85°, at the
fixture's step and at 1.2 of the strip's bound, both walls, its error is at
most 0.36 of production's and typically 0.08 to 0.2. The fixture's band:

| update | second order, 1.6 / 1.9 / 2.2 Hz | mean |
| --- | --- | --- |
| production | 1.025 / 1.066 / 1.138 | 1.077 |
| Padé(2,2) half-kicks | 1.013 / 1.034 / 1.070 | 1.039 |
| **full-step Cayley, wall acceleration** | **0.998 / 0.999 / 1.005** | **1.0006** |

The first-order wall's band reads 0.9955 under the adopted update against
production's 1.0006, though its complex error is five to ten times smaller:
a magnitude ratio does not see the phase.

Candidates that failed, kept in the harness: the wall's acceleration alone,
under the half-step kicks, stays second order with a larger constant; the
full-step Cayley alone, under the fourth-order drift, likewise; a pair of
Taylor-derived corrections (a kick force `(I − dt²G²/24)E F` and a drift term
`dt²/24 · (G² X)_Q`, from the step's error commutators) is fourth order but
no better than the Padé at the app's step, and worse with Cayley kicks.

## Stability and energy

On the coarse 2D mesh behind second-order walls, under the fourth-order drift:
spectral excess at most 1.4e-14 from 0.2 to 1.22 of the bound, with the four
loss cases of the scattering spike, and an energy peak of 1.0000 to t = 100,
as production's. A Gaussian pulse from the interior, nothing at the walls:
the energy still in the box at t = 10 is 5% above the same update's small
step, production's 23%; at t = 30 2% below, production's 60% above.

The trapezoid split matches these: spectral excess 9.5e-15, the pulse 6%
above its small step at t = 10 and 2% below at t = 30.

**One caveat.** Data that starts with a normal field already *at* the wall
is out of equilibrium there, and the second-order wall's static memory - its
law's pole at zero frequency - keeps a share of the start that depends on
the step. The energy left at t = 100, against 1.15e-3 at 0.075 of the bound:

| update | 1.2 | 0.6 | 0.3 |
| --- | --- | --- | --- |
| production | 1.21e-3 | 1.15e-3 | 1.15e-3 |
| wall-after split | 4.95e-3 | 1.65e-3 | 1.25e-3 |
| trapezoid split | 3.01e-3 | 1.18e-3 | 1.15e-3 |

The wall-after split's share is mostly its stored state meaning something
else at the trace; the trapezoid split, whose stored state is the interior's
kind, keeps 0.61 of it. The rest is the start itself: the wall's stiff modes
answer within a step, and the bilinear map, which tends to −1 at stiff rates
where the exact flow tends to 0, rings on them instead of letting them die.
That limit is the price of the damping term being exact for every `w`, so
it is not to be traded away. A field that arrives from the interior is
unaffected. A handoff or a remesh that hands a generation a field already at
the trace is such data, so stage B measures repeated handoffs and, if the
static memory drifts, starts the wall's state at its equilibrium with the
carried field.

## What is left

The adopted update is second order with a small constant, uniform in the
wall's stiffness. A fourth-order wall would need the wall row's `z³` term,
`dt²/12 · g · Q⃛_b`, which takes one more gather of the neighbours' `M⁻¹F`;
not tried. The midpoint force `F + dt/2 · K u` carries a `z²/8` error of its
own, which the wall's stiffness amplifies at large steps; its curvature
correction, `− dt²/8 · K u̇`, took the normal-incidence error at the app's step
only from 4.4e-4 to 3.7e-4, not worth its gather.

## For stages B and C

Production carries the wall through more than the fixed linear path:

- first-order walls, which damp node by node in the kicks' local loop rather
  than on the trace, take the same halves as scalars;
- the driven path, whose mass moves within a step: the explicit half at the
  start's mass and the implicit half at the end's, the trapezoid rule for a
  moving generator, and one preparation a step instead of two;
- the kick form, with the wall's rate in `u̇`;
- the leapfrog, which takes the halves alone;
- pinned trace nodes, as held rows of both halves;
- the energy ledger: the wall's loss becomes the trapezoid's own dissipation
  at the step's mean state, `dt Σ (D û² + (w + memory)²)`, and what the
  split leaves goes to the splitting residual, which converges at second
  order as the bulk's does;
- nonlinear wall sites, whose discrete-gradient kick does not split into
  halves as it stands;
- the AMR indicator and the primary rate, which rebuild the wall from the
  state at a step's ends with today's update;
- handoffs and remeshes, per the caveat;
- the solve's contraction, about `(1/7)(h · decay)²` with `h` its duration:
  doubling the duration roughly quadruples it, which the gallery's meshes
  must show to stay well inside the sweep's limits.

The fixture's second-order band would then be held to about 1%.

**B1, done 10 October 2026: the fixed path.** `CanonicalWallStep::Trapezoid`
on `CanonicalWaveState`, beside the half kicks, which stay the default; the
device refuses it until stage C. On the fixture the second-order band reads
1.0098 between free sides and 1.0105 between pinned ones, against 1.0979 and
1.0957, and the first-order band 0.998. At normal incidence the channel's
near-less-far reading, 6.5e-6 against 5.9e-6, is the two meshes' difference
more than the wall's step - both converge to 3.9e-6 at a quarter of the step
- and on the same mesh the trapezoid step's run is ten times closer to its
quarter step than the half kicks'. Across the gallery at the app's step the
wall solve's contraction is at most 0.14 (0.04 under the half kicks), its
sweeps at most 22 in f64 and 12 in f32 (14 and 8): one solve of at most 12
device sweeps a step where there were two of at most 8.

**B2, done 10 October 2026: the driven path.**
`CanonicalTemporalWaveState::with_wall_step`. The explicit half reads the
state the step starts from at the mass in force there and the implicit half
the end's - the trapezoid rule for an admittance that moves with the mass -
and the wall solve is prepared once a step instead of once a kick. A
nonlinear site reads the field the half ends with: along the trace by Newton
on its linearization, each iteration the linear solve at the per-node mass
`1/U′`, and at a first-order wall's node by a scalar Newton. The drift form's
`ũ` and the kick form's `u̇` carry the wall as on the fixed path. With the
trapezoid step made every state's default for one run, 514 of the core's 515
tests passed - nonlinear walls' passivity and second-order balance, pumps,
pins and a reversal among them; the one that did not reads the AMR
indicator, which rebuilds the wall with the half kicks (B3). Twelve
driven-path wall tests now run under both steps. On a kick-form generation,
a stiffness-side Kerr law of `χ = 1e-9`, at its own step of 0.9 of the
bound, the channel's second-order band reads 1.0195 of its law against the
half kicks' 1.0532: twice the drift form's residue, as the strip said.

**B3, done 11 October 2026: what reads the wall.** The primary rate reads
the state at a step's end, which under the trapezoid step is the bulk's
kind, and its order test holds under both steps. The adaptation estimate's
outgoing-history term checks the wall's state over the last step against
the trapezoid relation, which the trapezoid step satisfies: the term reads
roundoff where the half kicks leave a defect, the trace's law holding
exactly in discrete time. Across the gallery's 34 scenes behind a
second-order wall, three seconds from rest, that term was at most 0.04% of
the estimate's total; the global indicator moved by at most 0.6% and the
refinement candidates by at most four of hundreds. The estimate keeps its
formula; the tests that need the term to read something step by the half
kicks on purpose. Handoffs carry a state near the wall's equilibrium, not
the caveat's: a pulse crossing the walls with the step alternating between
1.2 and 0.9 of the bound every 25 steps leaves 7% more behind than without
a handoff under the trapezoid step, 25% under the half kicks. With the
trapezoid step every state's default, the core's 517 tests pass, as they do
with the half kicks'.

**C1, done 11 October 2026: the device's fixed path.** Plan flag 1024, under
either integrator. Before the first kick the trace's prepare and reduce leave
the walls' rate of the state the step starts from, which the nodal kick adds;
after the second, prepare, reduce, one solve over the whole step and finalize
take the implicit half. Between them the drift form's `ũ` takes the wall's
acceleration along the trace in four passes, and a prescribed trace's pins
return their shares in one more. A first-order wall's halves are scalars in
the nodal kicks. The ledger books the trapezoid of the walls' dissipation
rate at the step's two ends, each formed by the pass that holds that end's
state, and the reference books the same; it is not an identity, and its
remainder, 7.7e-4 of the energy over the timing fixture's 144 steps, the
reference leaves too. Against the f64 reference the state reads as the half
kicks' does, `Q` and `b` within 1e-6 there with loss, forcing, pins, a gap,
obstacles, events and the leapfrog, handoffs within 2.6e-7 and 1.4e-6, a
thousand steps of Luneburg lens 3.5e-6 and 4.0e-6; the device's ledger holds
to the reference's within 1.6e-7. A step costs one solve where the half kicks
took two, plus six passes: with a fixed generation's inverse, two dispatches
more a step, 0.229 against 0.212 ms on the timing fixture; where the solve
sweeps, as beside a pinned trace, four fewer, 0.409 against 0.445 ms. At the
app's plain-scene edge of 0.06 it is 0.271 against 0.259 ms, 5% more. A
driven generation's walls stay refused on the device until C2.

**C2a, done 11 October 2026: the device's driven path behind linear
walls.** A driven generation's plan takes the state's wall step, its wall's
tables from a factor prepared for the whole step; the trace is swept at
each stage's mass as under the half kicks, and the shader's stages already
read the start's mass for the explicit half and the end's for the implicit
one, as B2 does. A pinned trace beside a loss found B2 reading a pin's
field from the flux the loss stage had decayed, where its half kicks read
the signal; the reference now reads the signal there too, and that case
steps at second order again (2.02 to 4.07 in the order test's ratio).
Against the f64 reference behind a pump the forced fixture
reads `Q` and `b` within 2.5e-7 and 4.6e-7 behind either wall order, its
wall-loss and source lanes within 1.6e-7 of the reference's; the driven
document behind second-order walls within 7.1e-7 (a pump, van der Pol,
sine-Gordon); a thousand steps of Parametric pump behind second-order walls
4.7e-6 and 4.5e-6. On the driven timing fixture, whose trace takes six
sweeps either way at its step, a step takes 21 wall dispatches where the
half kicks take 30, 1.076 against 1.139 ms; behind a first-order wall the
two read alike, 0.439 against 0.433 ms. A field law beside a wall stays
refused until C2b: its nonlinear sites, and the kick form, which only a
stiffness-side law brings.

**C2b, done 11 October 2026: the device's nonlinear walls.** A nonlinear
wall's halves read the field `U(Q)`, as B2's do. Before the first kick a
pass caches each trace node's field at the step's start, from which prepare
and reduce take the explicit half; a nonlinear node's plain kick adds it,
and a first-order wall's node `−D U` of its own. After the second kick the
implicit half is Newton on `U ≈ U_k + U′_k (Q − Q_k)` within the half
kicks' budget of four linearizations, each solving at the per-node mass
`1/U′_k` with the offset `U_k − U′_k Q_k` carried through the generator's
input column; a first-order wall's node solves `Q′ + dt/2 · D U(Q′) = Q`
by the safeguarded scalar Newton. The drift form's `ũ` reads `A = U′` at
the drift's field. The kick form takes the walls' rate in `u̇`: the first
kick's prepare and reduce run ahead of its rate pass, and before the second
a field pass and two reductions form it at the step's end. Against the f64
reference over 200 steps of a Kerr and saturable medium, which brings the
kick form, the state reads within 1.2e-6 behind either wall order, a pinned
trace with a source, loss and a pump among them, and the wall's loss lane
within 1.7e-7 of the reference's; a Kerr oscillator in the drift form
within 1.2e-6 behind either order and a pinned trace, beside van der Pol
too. Behind the driven timing fixture's nonlinear second-order wall, six
sweeps a solve, a step takes 67 wall dispatches where the half kicks take
122; its time a step is still to be read on a quiet host.
