# The outgoing wall under the step, 10 October 2026

**Status: stage A done, 10 October 2026.** A spike in
`experiments/material-laws-spike/` with no production change. Stages B (both
CPU paths) and C (the device) are planned separately.

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
Cayley map. It is:

1. **Kicks:** the trace nodes take the bulk's plain kick, `Q −= dt/2 · F`, in
   both halves.
2. **Drift:** the drift form's `ũ` at the trace gains the wall's acceleration,
   `+ dt²/12 · M⁻¹ (G (G X − E F_mid))_Q`, with `F_mid = F + dt/2 · K u`.
3. **Wall:** after the second kick, one Cayley over `(Q_b, Z)` for the whole
   step, `(I − dt G/2) X′ = (I + dt G/2) X`.

That is one wall solve a step instead of two, with today's structure at twice
the duration, and two applications of the generator in the drift. Stepped
this way and as square-root halves the harness's responses agree to 3e-10.
The leapfrog needs only step 3: it has no `ũ`, and the full-step Cayley alone
leaves it `z²/8` in the admittance.

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

**One caveat.** Data that starts with a normal field already *at* the wall
leaves the second-order wall's static memory - its law's pole at zero
frequency - holding four times the small step's energy at 1.2 of the bound,
where production's holds the right amount. The raw state at a trace node
means something different under the two updates; a field that arrives from
the interior is unaffected, but a handoff or a remesh that carries a field
onto the trace into a new generation is exactly such data. Stage B must
carry the wall's state in the update's own terms.

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

- the driven path, whose mass moves within a step, so the full-step Cayley
  must pick its stage's mass;
- the kick form, for stiffness-side field laws, whose `F̃` replaces `F` and
  whose `ũ` is the plain field;
- the leapfrog, which takes step 3 alone;
- pinned trace nodes, as held rows of the wall solve;
- the energy ledger's wall terms, now a plain kick and a separate wall work;
- handoffs, remeshes and the wall's physical memory, per the caveat;
- whatever reads the field at the trace nodes, which now holds the state
  after the wall's update;
- the step is no longer symmetric in time, for any test that assumes so.

The fixture's second-order band would then be held to about 1%.
