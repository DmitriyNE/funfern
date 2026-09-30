# Gallery plan: Stage 11.5 and the scenes after it

**Date:** 25 September 2026, closed 27 September 2026

**Status:** batches A to F done. Every scene of A to E is built, 8b and 25b
with them, each behind its measured claim and a device run, and the catalogue
went from 12 entries to 36; batch F, proposed and built on 30 September 2026,
took it to 42 and revised seven scenes (see its section). One scene is left, the plasmonic guide, which needs physics
the solver does not have; it is under "Still open". The scene sections below
are the proposals the scenes were built from, some with their measured
claims added; each scene's entry in the engineering log has what was built
and what it measured.

## Purpose

The gallery is where the solver's physics is seen. Every scene in it must
satisfy three things, as the four Stage 10 slabs already do:

1. A description that says what the scene shows, in one or two sentences.
2. A measured claim: a CPU test in `topology_examples.rs` that runs the scene's
   own document at a coarse mesh and asserts the number the description
   promises, against a control where one exists. The tolerance is set from the
   scene's first run and recorded in the engineering log.
3. A device run through `canonical_gpu_long_run` with the scene's name, inside
   the Stage 0 bound, before the commit.

Every scene must be authorable with the app's own tools: geometry, span and
wall conditions, regions, materials and their law slots, sources and their
signals, probes and view settings. A scene never carries state the UI
cannot make, such as a seeded initial field. The φ⁴ scene's seed is an
ordinary volume source with a formula profile for that reason.

A scene whose claim cannot be measured is not shipped. A scene whose physics
needs something the solver does not have is filed under "Blocked" at the end,
not approximated.

Scenes are built in `topology_examples.rs` with the existing `Builder`, which
gains what the new geometry needs (see "Groundwork"). Each scene is one
commit with the full gate and a dated log entry.

## Conventions used below

- The domain is the unit square `[−1, 1]²`. Frequencies are in Hz, the free
  wave speed is 1, so a source at 3 Hz has wavelength 1/3.
- "Launcher" is a plane-wave source: a thin volume-source strip spanning a
  channel, built as the face between two wall-attached transmitting dividers,
  with a uniform profile. It radiates both ways; the outer wall behind it is
  outgoing and takes the backward wave. A Dirichlet wall is not used as a
  launcher where anything reflects back to it, because a Dirichlet wall
  reflects fully and makes a cavity.
- "Channel" is a scene whose top and bottom walls reflect so that a y-uniform
  wave stays uniform. It is used only where the wave is y-uniform.
- "Outgoing" without qualification is the second-order condition.
- Claims are measured on the CPU reference at the mesh edge named in the test.
  Where a claim needs a phase, the trace helper returns the complex amplitude
  at the drive frequency over the last whole periods.

## Batch A: existing scenes and groundwork

### 1. Double slit

Today the top and bottom walls reflect, which traps channel modes, and the
baffles stop at y = ±0.90 while the walls sit at ±1.0, so each end leaves a
0.10 gap that is itself a slit.

New build: the source sits in a box made of baffles. One C-shaped open
polyline runs from (0, 0.36) up to (0, 0.60), left to (−0.75, 0.60), down to
(−0.75, −0.60), right to (0, −0.60) and up to (0, −0.36). Its first and last
spans are the outer screen pieces, reflecting on both faces. Its three box
spans absorb on the inside with the second-order condition and reflect on the
outside. The middle screen piece from (0, −0.14) to (0, 0.14) is a second
baffle, reflecting on both faces. The slits are the two gaps, width 0.22,
centres ±0.25. There is no region and no material: the box interior is the
background face, reached through the slits. The source stays at 3 Hz,
moved to (−0.50, 0). All four outer walls are outgoing. The far field is on
with inset 0.08, and every curve lies inside its contour. A segment probe at
x = 0.85 from y = −0.85 to 0.85 is the screen.

Claims, with the screen 0.85 behind the slits, spacing 0.5, wavelength 1/3:

| Where | Expected |
| --- | --- |
| screen, y = 0 | central maximum |
| screen, y = ±0.31 | minima, RMS well below the centre |
| screen, y = ±0.77 | side maxima, about half the central intensity |
| far field, 0° and ±41.8° | the three lobes; nothing on the source side |

Nothing leaves the box except through the slits, so the far-field pattern is
the two-slit pattern alone.

### 2. Obstacle over a mirror (was: Starter obstacle)

The description says "above a reflecting floor" but both walls reflect. The
top wall becomes outgoing; the floor stays.

### 3. GRIN collimator

The rod becomes a quarter-pitch collimator: a point source on its entrance
base leaves the other base as a collimated beam. The profile changes from
`smoothstep` to parabolic, `n = 1 + dn·max(0, 1 − (y/H)²)`, because the
smoothstep profile is flat to fourth order on the axis and has no paraxial
pitch. With `H = 0.3` and `dn = 0.6` the paraxial pitch is
`2πH·√((1 + dn)/(2dn)) = 2.18`, so the rod is 0.544 long: x from −0.75 to
−0.206, y from −0.3 to 0.3, rectangular. The source sits inside the rod on
the entrance base at (−0.74, 0), 4 Hz.

It was built in the Mechanical skin with reciprocal mass and stiffness, so
the impedance was matched. On 27 September it became glass in the E_z skin,
`ε = n²` and `μ = 1`, where the field obeys the Helmholtz equation in `n`
itself and the pitch holds as written; its faces reflect as glass does,
about 5% on the axis.

Claims, as built (edge 0.08, on a cut across the whole domain height): at
x = 0.75 the rod's beam keeps 0.77 of its power within the aperture, up from
0.71 at x = 0.2, where the bare source keeps 0.39, down from 0.43. Not
claimed: a flat phase at the exit face, where the near field still holds the
reflected and wide-angle waves, and a half-amplitude width, which the
profile's shoulders near half the peak move between 0.45 and 0.75 along the
beam.

### 4. Luneburg lens, angled illumination

The left-wall Dirichlet drive goes back to outgoing. A straight open curve
becomes the radiator: 1.1 long, tilted so its normal points along 30° above
the x axis through the lens centre, so its centre is at (−0.66, −0.43). Its
lens-facing face is a prescribed Neumann flux at 3.5 Hz; its back face is
second-order outgoing. The focus moves to the rim point in the propagation
direction, (0.45, 0.21).

Claims: the energy in a disk of radius 0.065 at the new focus exceeds twice
the energy in the same disk at the old focus (0.485, 0); the lens-energy
probe stays.

Fallback if the canonical operator refuses an internal Neumann face: a thin
tilted volume-source slab, 1.1 by 0.06, which the phased array already
proves. It radiates both ways and the backward wave leaves through the
outgoing walls.

### 5. Groundwork

- `Builder`: per-span behaviours on one curve; open curves whose ends attach
  to outer walls; faces bounded by walls and dividers assigned to a region.
- `PresentationSettings` gains the integrated-field view, persisted with a
  serde default so version 22 files still load. The φ⁴ and Josephson scenes
  open on it.
- Test helpers: the trace at several points with the complex amplitude at a
  frequency and the `r` channel; an RMS profile along a line; energy in a
  disk. A sweep helper that writes a transmission-versus-frequency table to
  the scratchpad, for the scenes whose frequency is found rather than chosen.
- The pulse todo, filed in `docs/plan.md` under "Later experiments". See
  "Blocked".

## Batch B: oscillator media

### 6. Plasma mirror

TM. A channel whose background is Klein-Gordon with a cutoff that ramps along
x: `omega0 = W·smoothstep(0, 1, (x − x0)/L)` with `W = 2π·4`, `x0 = −0.2`,
`L = 0.8`. A launcher at x = −0.82 drives 3 Hz. Left and right walls
outgoing. The wave stretches and slows as it climbs the ramp, turns where
`omega0(x)` reaches the drive frequency, near x = 0.30, and dies beyond it.
The material reads Custom in the simple view, as the GRIN rod does.

Claims:

- The RMS at x = 0.8 is under 2% of the largest RMS on the axis between
  x = −0.7 and −0.3. With `W = 0` it is comparable.
- The standing-wave node spacing grows along the ramp as
  `π/√(ω² − omega0(x)²)`, 0.167 in the flat part and about 0.21 where the
  cutoff is 0.6 ω; the node count between x = −0.7 and the turning point
  matches the WKB phase integral to one node.
- Flat variant, the measurement the log promised on 24 September: cutoff
  2.5 Hz everywhere, drive 3 Hz, right wall first-order outgoing. The
  standing-wave ratio on the axis gives the wall reflection
  `R = (ω − ck)/(ω + ck) = 0.29` within 0.03. The left wall's own reflection
  does not disturb this, since the ratio of the two travelling amplitudes is
  R whatever feeds them.

### 7. Josephson line

TM. A channel of sine-Gordon with `omega0 = 12`, so the kink width
`ℓ = c/omega0` is 0.083. The left wall is Dirichlet with a constant offset of
3 and no oscillation: the integrated field `r`, the Josephson phase, winds at
that rate, and every 2π launches one fluxon down the line, a kink in `r` and
a pulse in `E_z`. Right wall first-order outgoing. Opens on the integrated
field.

Claims:

- At x = 0.5, `r` advances in steps of 2π, one every 2π/3 = 2.09 s after the
  first arrives.
- Each pulse in `u` carries area 2π.
- A Klein-Gordon line under the same bias: DC is below its cutoff, so `r` at
  x = 0.5 grows at under 1% of the wall's rate; the static profile is
  `e^{−12x}`.

Exploratory. Emission from a biased end, the fluxon speed and the outgoing
wall's treatment of a kink are unmeasured; CPU exploration first. Fallback:
nonlinear transparency, a strong wave crossing a sine-Gordon slab below its
cutoff where a weak one cannot. `r` grows without bound at the wall, which
f32 carries for hours before its resolution matters; the scene does not run
that long.

### 8. Symmetry breaking

TM. Background φ⁴ with `lambda = 60` and `phi4_bound = 3`, plus a constant
electric loss of 1/s so the domains settle. Rest is the unstable top of the
double well, as the Gate O note says; a weak source seeds the fall, 3 Hz,
amplitude 0.5, at (−0.3, 0.2). All walls outgoing. Opens on the integrated
field: `u` goes dark once the field settles, `r` shows the domains and the
walls between them, width `√2·c/√λ = 0.18`, which then straighten and
annihilate.

Claims after 6 s at the test mesh:

- `|r| > 0.9` on more than 90% of the nodes.
- Both signs occur, so at least one wall formed.
- With `lambda = 0`, `|r|` stays under 0.2 everywhere.
- A seed three times stronger gives the first two claims again.

The bound of 3 clears the fall's overshoot of √2 with margin; it also sets the
curvature and with it the step, which the mesh bounds tighter anyway.

### 8b. Pinned domain wall

Added at the user's request after scene 8: a φ⁴ wall that stays. A straight
wall costs the same anywhere, so on its own it is only neutrally stable. Two
baffles welded to the floor and the ceiling at x = 0 leave a neck a quarter of
the channel's height, where a wall costs a quarter as much; a seed odd about
x = 0.3 forms the wall off-centre and it slides into the neck. Top and bottom
reflect. Building it found and fixed a mesher defect: two baffles welded at
one end in one face did not mesh.

Revised the same day: with straight baffles a settled wall did not follow
the neck when the baffles moved, because a straight wall away from the neck
feels only the exponential tail of its own profile. The neck is now two round
bumps, arcs of radius 0.6 welded to the floor and the ceiling with the
pockets behind them left out, so a wall anywhere over them is pushed to the
waist and follows it when the bumps are dragged. Narrowed again at the user's request:
each bump is now half an ellipse, ±0.35 along the wall and 0.75 deep, drawn as
two Bézier quarters with seven controls, and the seed is a tenth as strong.

### 9. Self-sustained emitter (was: Pacemaker)

The pacemaker was explored first and does not happen in this medium. A 3 Hz
disk of "Van der Pol oscillators" in a 2 Hz lattice of them, gain 2: the bulk
locks at 2.05 Hz and the disk is suppressed. With the disk's gain at 10 its
rings fill the domain by 6 s, but they thin with distance below what
suppresses the bulk's own mode, which takes over by 15 s; switched on in a
bulk already oscillating, the disk rings in itself and its waves never get
past. The gain lifts every mode alike, the modes are coupled by waves rather
than diffusion, and cross-saturation is twice self-saturation, so whichever
pattern saturates first holds; the pacemaker effect of heart tissue and the
BZ reaction needs diffusive coupling. The planned fallback, a uniform lattice,
failed its amplitude claim too: 0.25 to 1.05 with outgoing walls, competing
modes for 27 s with reflecting ones.

Shipped instead: a disk of radius 0.25 of "Van der Pol oscillators", cutoff
3 Hz, gain 15, threshold 1 and a magnetic loss of 25/s, in a Klein-Gordon
plasma with a 2 Hz cutoff, seeded by a 2.5 Hz point source of 0.01 at its
centre. Walls outgoing. The magnetic loss limits the gain to the disk's long
waves: a short wave keeps half its energy in the magnetic field, the disk's
near-uniform oscillation almost none. Without it the gain lased on the mesh's
shortest waves at the rim, which the short-wave viscosity added after this
scene now stops (`docs/spikes/funfern-gate-o.md`). The disk
rings as one oscillator at its cutoff and radiates rings of wavelength
`c/√(f² − f_c²) = 0.447`. Raising the plasma's cutoff to 3.6 Hz traps the
tone.

Claims at edge 0.08 over the last 4 s of 10 s, as measured with the
short-wave viscosity:

- The far field's tone is 3 Hz within 1% (2.998), not the seed's 2.5 Hz,
  which is under 2% of it (0.7%).
- The centre's amplitude is `2a/√3 = 1.155` within 5% (1.181).
- The rings' phase runs along a ray at `2π√(f² − f_c²)` within 5% (14.46
  against 14.03).
- With the plasma at 3.6 Hz the disk rings more strongly (1.245 at 3.26 Hz),
  and its tone 0.7 away is under 5% of the radiating disk's (1.9%).

### 10. Plasma whispering gallery (was: Plasma-clad whispering gallery)

TM. A vacuum disk of radius 0.4 at (0.05, 0), drawn as a sixteen-control
circle (flat to 1e-4), in a "Klein-Gordon" plasma with a 3.5 Hz cutoff. A
point source 0.06 inside the rim, amplitude 10, width 0.03, and a point probe
opposite it on an antinode. Walls outgoing; nothing reaches them. Lossless:
below the cutoff the plasma turns every wave back, so the disk's modes have
nowhere to leak.

Explored by ringdown (a bump near the rim, the spectrum over 32 rim points
and the angular order of each peak): the fundamental radial order runs m = 3
at 2.265 Hz, 4 at 2.690, 5 at 3.095, 6 at 3.495 (the cutoff) and 7 at 3.880
(above it, leaky). The m = 5 mode sits at 3.098-3.099 Hz on meshes from edge
0.04 to 0.16, so the lossless resonance survives every resolution preset; the
scene drives it at 3.1 Hz, where it builds for minutes.

The plan's skin `c/√(ω₀² − ω²)` holds only for a mode without angular
structure. Outside the rim a mode of order m falls as `K_m(κr)`, whose local
rate `√(κ² + m²/r²)` is steeper, so the claim is made against K_5. The plan's
4 Hz for the transparent cladding lands beside the m = 8 mode at 4.005 Hz,
which its own angular barrier holds near the rim, and the plasma above the
cutoff is optically thinner than vacuum (n = 0.48 at 4 Hz) and still turns
back every wave past 29°; 5 Hz shows the transparency plainly.

Claims at edge 0.08 over the last 2 s of 12 s:

- Driven at 3.1 Hz, the half of the rim away from the source rings more than
  3× as strongly as when driven between resonances at 3.225 Hz (0.169
  against 0.037), with `2m = 10` lobes around the rim.
- From 0.02 to 0.11 outside the rim, opposite the source, the field falls to
  K_5's ratio within 25% (0.288 against 0.250), nearer it than to the plain
  skin `e^{−κ·0.09}` (0.399).
- Driven at 5 Hz, above the cutoff, the field 0.35 outside the rim keeps more
  than half of its strength 0.02 outside (1.2), where on resonance it keeps
  under 5% (1.4%).

This is the nearest thing to a polariton the catalogue can make. A true
polariton needs a Lorentz law with a second auxiliary state, which Gate O did
not add; see "Blocked".

## Batch C: guides, resonators and amplifiers

### 11. Parametric fiber amplifier

TM. A graded-index fiber across the whole width, `n(y) = 1 + 0.6·max(0, 1 −
(y/0.15)²)`, the band between two wall-attached levels at `y = ±0.15`. In TM
the two rows are ε and μ, so both carry `n`: the index is `√(εμ)` and the
impedance stays one. (The GRIN rod's `1/n` stiffness is right in its
Mechanical skin; copied into TM it made `εμ = 1` and the fiber invisible,
which the first exploration ran into.) The permittivity carries the
"Travelling modulation" preset over the graded base: pump 5 Hz, depth 0.2,
wavenumber 36.2, phase `5π/8`, angle 0. The fiber also carries the
Klein-Gordon preset with a 1.25 Hz cutoff, which makes it slightly
dispersive. A 2.5 Hz point source of 4 on
the axis at x = −0.85, a probe at x = 0.85, and a line probe along the axis
from x = −0.75, past the source's near field, to 0.75, short of the probe,
whose energy density shows the signal growing along the fiber. Walls outgoing.

Explored. Unpumped, the fundamental mode runs at β = 21.94 (`n_eff` 1.397;
a 1D mode solve of the same profile gives 1.391). Pumped at `2β` the gain is
phase-sensitive, as degenerate parametric gain is: the best and worst pump
phases are half a turn apart (the plan's quarter turn was wrong), and the
gain grows as `e^{gx}` with `g ≈ dβ/4`, which put the depth at 0.2. The
plan expected a pump uniform in space to be merely mismatched; instead it
conserves wavenumber rather than frequency, couples the forward wave to a
backward one, and over this length drives an absolute instability: the far
end grew 5×, 21×, 77× and 411× of the unpumped level at 6, 9, 12 and 16 s.
That is the scene's contrast now. The plan's "signal off stays quiet" is
true of any deterministic run from rest and is dropped.

Dispersion, added on 2026-09-27. The dispersionless fiber filled with
mesh-scale excitations. A pump running at the mode's phase velocity also
phase-matches the signal's sum frequencies with itself (7.5, 12.5, 17.5 Hz
and on) and climbs them to the mesh's scale; only the mesh's own dispersion
stopped it. At edge 0.04 the three rungs held 0.9 of the signal, and the
gain fell with each refinement: 4.6×, 3.4× and 2.7× at edges 0.08, 0.04 and
0.02. The grid filter and a gradient viscosity were measured against it and
set aside (`docs/engineering-log.md`, 2026-09-27). The 1.25 Hz cutoff
lowers the mode's index at 2.5 Hz from 1.38 to 1.18 and much less at
7.5 Hz, so the first rung runs out of step within 0.2 of fiber instead of
0.44. The gain peaks at wavenumber 36.2, 3% under the dispersive mode's
`2β = 37.17`, because the pump shifts the signal's own propagation constant
a little; it is flat to 2% over ±0.4 of that.

Claims at edge 0.08, at the far end at 2.5 Hz over 2 s windows:

- The travelling pump amplifies the signal more than 7× against the pump
  off (8.58×), and steadily: at 14 s the gain is within 10% of 8 s (8.71×).
- Advanced by half a turn it squeezes the signal below half (0.12×).
- Uniform in space, the same pump makes the fiber oscillate: its far end
  grows more than 3× from 8 s to 12 s (6.0×).

### 12. Bent fiber

TM. A classic step-index core of glass, `ε = 2.25` and `μ = 1`, width 0.10,
which is single-mode at 4 Hz (`V = 1.40`). Lead-in along y = −0.5 from the
left wall to x = −0.4, a 90° arc of radius 0.5 centred at (−0.4, 0), lead-out
from (0.1, 0) straight up to the top wall. The core is the face between two
open curves, its inner and outer edges, each attached to the left and top
walls with transmitting spans: a straight Bézier piece, the quarter circle
in two eighths (under 5e-6 of the radius off the circle), and a straight
piece, joined at C0 knots. The face inside the bend is a background region
of its own. A point source of 10, width 0.03, sits in the core at
x = −0.9. A free transmitting curve along the core's axis, the same path
from x = −0.8 to 0.2 short of the top wall, carries a boundary probe at the
High preset: the flux along a curve is not a readout, so its energy density
stands in for the power the mode carries along the length, and 128 samples
resolve that density's ripple at half the guided wavelength, 0.094. A point
probe reads the output 0.1 short of the top wall, past the curve's end,
since a point probe on a curve is refused. Walls outgoing.

Explored. The straight fiber carries the slab's mode: `n_eff` 1.329 at
edge 0.08 and 1.3238 at edge 0.04 against a 1D solve's 1.3234, its profile
across the core within 4% of the slab's. The point source also radiates
into the cladding, rings that leave through the left wall and the floor;
the measurement projects each cut onto the slab mode, which the cladding's
radiation is orthogonal to. The share delivered to the top, against a
straight fiber at the same distance from its end wall, converges by edge
0.05 (within two points of 0.04): at edge 0.04 radii 0.3 to 0.7 in steps of
0.1 lose 45%, 33%, 23%, 15% and 12%. The loss falls with the radius as
Marcuse's exponent `e^{−2γ³R/3β²}` (`2γ³/3β² = 6.24`): −ln of the share
delivered, over `R·e^{−6.24R}`, stays between 11.3 and 12.9 from radius 0.3
to 0.6, and is 14.5 at 0.7. His asymptotic prefactor is about three times
the measured one at these sharp bends. Along the axis, the energy density
averaged over a period falls from 64 on the lead-in to 51 on the lead-out
at edge 0.05 (0.80, against 0.76 from the mode's projection), smoothly; at
edge 0.08 it swings up to 1.9× between neighbouring samples, a coarse
mesh's own reflection. The app's adaptation refines the fiber to six
elements per wavelength by default.

Claims at edge 0.08, with the power in the core's mode through a cut 0.2
short of the top wall, against a straight fiber's cut 0.2 short of the
right wall:

- The gallery's radius 0.5 delivers more than 60% of the launched power
  (72%).
- Radius 0.3 loses more than twice what radius 0.7 loses (45% against 12%,
  3.6×).

### 13. Photonic crystal

TM. A channel with a launcher at x = −0.85. A square lattice of ceramic
rods, `ε = 9`, radius 0.2 of the pitch, pitch 0.2, five columns deep
centred on the origin and ten rows filling the channel height. With the
pitch dividing the height the reflecting walls sit on the lattice's mirror
planes, so the channel is the infinite crystal at normal incidence. (The
plan's pitch 0.16 does not divide the height, and its radius 0.3 is not
the textbook crystal: Joannopoulos's rods of `ε = 8.9` and radius 0.2 open
a TM gap from 0.302 to 0.443 of the pitch over the wavelength.) Each rod
is a regular octagon with the circle's area, its flats facing the axes. A
line probe runs along the midline between two rows to 0.6, short of the
point probe that reads the transmitted wave 0.25 behind the crystal.

Explored. Meshed as spline circles the rods cost the scene 36.6k unknowns
and a time step of 1.0e-3 at edge 0.08, its shortest edge 0.004. As
octagons the scene takes 9.4k and 6.5e-3, cheaper than the fiber, and
12-gons 11.6k and 4.8e-3. The transmission sweeps of both, from 1 to 3 Hz,
agree within a point or two: five columns pass under 3.4% from 1.40 to
2.20 Hz (0.28 to 0.44 of the pitch over the wavelength), 0.04% to 0.2%
through the middle, 12% at 1.35 and 27% at 2.25. Below, from 1 to 1.3 Hz,
they pass 48% to 100%; above, from 2.3 to 2.85 Hz, 48% to 88%; the next
stop band begins by 2.9 Hz. The gap's top matches the textbook's; its
bottom, 0.275, is band 1's edge at X, below the complete gap's 0.302,
which band 1 sets at M. At edge 0.05 the claims' three frequencies move
by under 1.5 points. The scene runs at 1.85 Hz, mid-gap.

Claims at edge 0.08, from the phasor averaged across the channel 0.25
behind the last column, against the empty channel:

- In the gap, at 1.85 Hz, five columns pass under 1% of the power (0.08%).
- Below it, at 1 Hz, more than 90% (99.6%).
- Above it, at 2.5 Hz, more than half (79%).

The Bragg-stack fallback was not needed.

### 14. Crystal bend

TM. Scene 13's rods as a block, seven by seven about the origin, pitch 0.2,
with a channel of missing sites that runs in along the middle row from the
left, turns at the centre, and leaves up the middle column: 42 rods, within
the scene's 64 curves. Walls outgoing. A point source of 10 at the gap's
1.85 Hz sits inside the channel's entrance, at (−0.5, 0), so it feeds the
channel rather than radiating into the room. A free transmitting path
along the channel, from (−0.4, 0) round the corner to (0, 0.8), carries a
boundary probe; a point probe at (0, 0.9) reads what leaves.

Explored, with the time-averaged power through a cut 0.6 wide across the
channel 0.14 inside the exit face, against the same cut on a straight
channel through the block from the same source. The channel guides from
1.60 to 2.15 Hz; below 1.6 Hz the straight exit falls to a hundredth. Across
1.65 to 2.15 Hz the bend delivers 0.90 to 1.10 of the straight channel's
power, 0.64 at 2.2 Hz at the gap's top. Ratios over one are the source's
own: the bend's small reflection returns to it and moves what it emits.
With the channel filled the same cut sees under 0.2% through the band. At
edge 0.05 the ratio at 1.85 Hz moves from 0.915 to 0.913; at 18 s instead
of 12, to 0.897. The cuts across the far walls read a little high, because
the source's backward emission leaves the entrance and wraps round the
block.

Claims at edge 0.08, 12 s from rest:

- The bend delivers more than 70% of what the straight channel does (91%).
- With the channel filled, under 1% gets out (under 1e-5).

The plan's "30% of the power entering it" is not the measure: the net
power entering a lossless channel always leaves it, whatever the bend
reflects, so the straight channel is the reference.

### 15. Ring resonator

TM. The bent fiber's glass, `ε = 2.25` and `μ = 1`, width 0.1: a bus fiber
along the floor, `y` from −0.6 to −0.5 from wall to wall, and a ring of the
same width, mean radius 0.6, 0.02 above the bus. A point source of 10 in
the bus at x = −0.9, at 3.975 Hz. A point probe reads the bus past the
ring, an area probe the ring. Walls outgoing.

The plan's ring, mean radius 0.35, would shed most of its power every turn
at 4 Hz: the bent fiber lost 45% per quarter turn at radius 0.3. A
stronger core or a higher frequency needs a mesh too fine for the test and
for real time, so the ring is as large as the domain allows, where a
quarter turn loses 12% to 15%, at the bent fiber's accurate 4 Hz.

Explored, with the power in the fiber's mode past the ring against the bus
alone at the same frequency, and the field round the ring's mean circle.
The resonances sit 0.17 Hz apart, at 3.80, 3.96 and 4.13 Hz for a gap of
0.05. There the ring is under-coupled: the bus keeps 0.46 on resonance. At
0.03 it keeps 0.07; at 0.02, 0.002 at 3.968 Hz, near critical coupling,
with the ring full by 30 s (its field 1.71e-4 at 30 s, 1.73e-4 at 45 s,
where the gap 0.03 was still filling). The resonance moves with the mesh:
at edge 0.05 it sits at 3.980 Hz, and edge 0.04 agreed with 0.05 at gap
0.03. The dip is about 0.03 Hz wide, so 3.975 Hz, between the two, keeps
the bus under a fifth on either mesh. Between resonances, at 3.885 Hz, the
bus keeps 0.92 on both meshes and the ring's field is a tenth.

Claims at edge 0.08, 30 s from rest:

- On resonance, 3.975 Hz, the bus keeps under half of what it keeps at
  3.885 Hz (0.16 against 0.92).
- The ring's field holds more than five times the energy it does between
  resonances (11×).

Real-time cost is the bent fiber's, 8.5k unknowns at a time step of 5.2e-3.
The ring fills over about 30 s of simulated time.

### 16. Acoustic whispering gallery

Mechanical. A reflecting arc baffle of radius 0.85 about the origin spanning
300°, open over the 60° on the left, built by `arc` as eight C0-joined
Bézier pieces. The source at 4 Hz sits 0.05 inside the wall at the top.
Walls outgoing; the opening lets sound out. The plan opened the arc at the
bottom, which is where the point half a turn from the source lies, so the
opening moved to the side: the wall runs unbroken from the source round
the right to the far point. Two area probes: "Far wall", a disk of radius
0.05 at (0, −0.78), and "Centre", radius 0.25 at the origin.

Explored. At a single point the centre sits on a node of the room's
standing pattern, and the far wall read 37× it, which says nothing. Over
the disks, the far wall is 1.4× to 4.1× the centre from 3.8 to 4.2 Hz at
15 s, the least at 3.9 Hz; along the right half of the rim the RMS is two
to four times the centre's. The far wall is steady from 25 s on (0.0102 to
0.0108); the centre is not, beating slowly with some ringing mode (0.0027,
0.0014, 0.0023 at 25, 35, 50 s), so the ratio runs 3.9× to 7.2× at 4 Hz.
Edge 0.05 gives 3.6× at 25 s. Without the wall the far point hears 0.70 of
the centre, near free space's `1/√2`.

Claims at edge 0.08, 25 s from rest, over the two probe disks:

- With the wall, the far wall is more than twice as loud as the centre
  although twice as far from the source (3.9×).
- Without it, the centre is the louder (the far point 0.70×).

### 17. Dielectric whispering gallery

TM. A disk of `ε = 4` and `μ = 1`, radius 0.3 about the origin, with a
point source 0.03 inside the rim at angle 0, at 2.55 Hz. A boundary probe
runs round the rim itself at the High preset, about nine samples a lobe,
and a point probe sits 0.05 inside the rim opposite the source. Walls
outgoing.

Explored. A sweep from 2.0 to 3.6 Hz finds a whispering-gallery resonance
every 0.3 Hz, each counted by its `2m` maxima round the circle 0.05 inside
the rim: `m = 6` at 2.25 Hz, 7 at 2.55, 8 at 2.87, 9 at 3.17 and 10 at 3.47.
The plan's "near 2.5 Hz" and "about `m = 9`" disagree: `m = 9` sits at 3.17
Hz, as the first-order estimate `nkR ≈ m + 1.86 (m/2)^{1/3}` puts it. The
higher the order the longer it takes to fill: at the peak the rim's RMS
went 0.058, 0.080, 0.091 at 15, 30, 60 s for `m = 7`, and 0.061, 0.105,
0.152 for `m = 9`, still growing at a minute. So the scene rings `m = 7`,
at the plan's 2.5 Hz, 88% full by 30 s, the centre 17× quieter than the
rim. The mesh hardly moves it: the peak is 2.55 Hz at edges 0.08 and 0.05,
and `m = 9` moved 0.002 Hz.

Claims at edge 0.08, 30 s from rest, on the circle 0.05 inside the rim:

- On the resonance the RMS is more than three times that at 2.7 Hz,
  between resonances (8.0×; 8.1× at edge 0.05).
- `|U|` has `2m = 14` maxima round it.

## Batch D: diffraction and interfaces

### 18. Maxwell's fisheye

TM, not the plan's Mechanical: the user asked on 25 September for graded
optics to run in an EM skin, and for plain `μ = 1` media, so the profile is
`ε = n²` with `n = 2/(1 + (r/R)²)`, in a disk of radius `R = 0.45`: the index
runs from 2 at the centre to 1 at the rim, where it meets the vacuum. The
region's frame follows it, so a dragged lens keeps its profile. A 3 Hz
source 0.03 inside the rim on the left; a boundary probe on the rim itself,
whose peak is the image. (A point probe on the image, 0.03 inside the rim,
sat on the rim probe's line; the view pass dropped it.) The view shows the
permittivity. Walls outgoing.

Explored, with `|U|` on the circle 0.03 inside the rim. Away from the source
the rim is brightest at the antipode, which is as bright as the rim beside
the source (0.0116 against 0.013): the rays launched inward all arrive,
those launched outward leave. The antipode is 4.9×, 5.0× and 3.7× the rim
45° either side at 2.5, 3 and 3.5 Hz, 5.1× at edge 0.05, and steady by 10 s.
With the disk vacuum it is 1.04×, and the rim is brightest beside the
source.

Claims at edge 0.08, 10 s from rest:

- With the lens the rim is brightest away from the source at the antipode,
  more than 3× the rim 45° either side (5.0×).
- With the disk vacuum the antipode is within a fifth of those points
  (1.04×).

### 19. Fresnel zone plate

Mechanical. A launcher at the left, 4 Hz. A screen of reflecting baffles at
x = 0 closed over the even Fresnel zones for a focus at `F = 0.4`, zone
edges `y_n = √(nλF + (nλ/2)²)`: 0.34, 0.51, 0.66, 0.81, 0.94, so the
openings are |y| < 0.34, 0.51 < |y| < 0.66 and 0.81 < |y| < 0.94, and the
last, even zone is closed out to the walls, to which those baffles attach.
A point probe on the focus and a line probe across half the focal plane,
from 0.08 beside the focus out to the wall; the axis line it replaced ran
through the focus probe. Walls outgoing.

Explored. The plan's plate, `F = 0.6`, focuses at 1.48× the bare plane
wave's amplitude at (0.6, 0), and its axis peaks at 1.76× beyond it, at
0.715. The plan's "twice the plane wave" belongs to a 3D ring plate, whose
zones contribute equally: in 2D the zones are slits, whose contributions
fall off (a single first-zone slit peaks on the axis near 0.9), and more
zones barely help. Opening the odd zones out to the walls, `F` from 0.3
to 0.6 gives 1.57× to 1.68× at the focus and 1.72× to 1.76× at the axial
peak, which sits 10% to 20% past `F`. `F = 0.4` fits three open zones and
focuses tightest: 1.68× (1.67× at edge 0.05, the same at 15 s), a spot
about 0.1 wide, and 3.4× the field 0.4 to either side. An amplitude of
1.7× is nearly three times the energy; doubling it would take a phase
plate.

Claims at edge 0.08, 10 s from rest, at the focus (0.4, 0):

- The RMS exceeds 1.5× the bare plane wave's (1.68×).
- It exceeds twice the RMS 0.4 to either side (3.4×).

### 20. Brewster angle

Electromagnetic, one document per skin. Glass, `ε = 2.25` and `μ = 1`,
fills everything beyond a divider at `x = 0.2`, and a 4 Hz point source
sits 0.2 in front of it. Each ray meets the face at its own angle and
reflects as if from the source's mirror image, so the reflection is read
on an arc of radius 0.8 about that image, the rays meeting the face from
20° to 72° crossing it in turn; a free transmitting arc there carries a
boundary probe, along which the fringes the reflection makes with the
direct wave fade out at the Brewster angle. Walls outgoing. The gallery
ships the `H_z` (TE) scene; the test builds its `E_z` (TM) twin.

Explored. The plan's tilted beam cannot show the null: the radiator this
domain holds, 0.8 long, spreads its beam over about ±15°, and its ends
diffract into the reflected cut, which read 0.054 of the power in `H_z`
at every angle. A plane wave from a launcher meeting a tilted interface at
56.3° was better, but it grazes the outgoing walls on its way and is not
plane enough: `E_z` read 8% to 25% above Fresnel and `H_z` about 0.1 with no
minimum at 56°. The point source is clean. The reflected field, the field
with the glass less the field without it on the same mesh, over the direct
field, follows Fresnel's `|r_s|` in `E_z` within 15% from 20° to 72°. In
`H_z` it ripples by about 0.06 about `|r_p|` and falls to 0.013 at 58° (0.012
at 56° at edge 0.05).

Claims at edge 0.08, 8 s from rest, on the arc:

- In `H_z` the reflection is least, over rays meeting the glass from 44° to
  70°, within 3° of `atan 1.5 = 56.3°` (58°).
- There `E_z` reflects more than 5× as strongly (7.2×).
- `H_z` reflects more than 3× as strongly at 72° as at the Brewster angle
  (5.5×).

### 21. Frustrated total internal reflection

TM, not the plan's Mechanical: optics goes in an EM skin. The bent fiber's
glass as a straight fiber along `y = −0.5` from wall to wall, lit by a
4 Hz source at its left end, and a glass block over it from `x = −0.3` to
the top and right walls, 0.05 above the fiber. The fiber's mode is light
held by total internal reflection at 61.9° inside the core, its field
outside falling as `e^{−γy}` with `γ = k₀√(n_eff² − 1) = 21.8`, the plan's
`κ` at that angle. The block frustrates the reflection, and the mode leaks
into it as a beam at that same angle. The block's edge is a wall-attached
L, so the beam leaves through the walls; with a free top face it would be
turned back by total internal reflection there. Probes on the fiber's
output and in the block. Walls outgoing.

Explored. The plan's prisms need a beam at one angle, and the scenes
before this one showed a tilted beam here spreads over ±15°: at 45°, 3.2°
past the critical angle, much of it would cross the gap below it. A point
source in one of two glass half-planes, read on an arc or as a plane-wave
spectrum along a line above the gap, came close near 45° (0.39 against
the formula's 0.40) but moved with the mesh and the line: past the
critical angle the tunnelled field is weak beside the lateral wave, the
line's finite span and the walls. The guided mode is one angle by
construction. With the block's material vacuum in the control, on the
same mesh, the power the fiber keeps falls smoothly along the block: 0.604,
0.953 and 0.9965 of the control at gaps of 0.05, 0.1 and 0.15. The leak's
rate, `−ln` of that, falls by 0.096 and 0.072 per 0.05 of gap against
`e^{−2γ·0.05} = 0.113`, and by 0.105 and 0.099 at edge 0.05: the coarse mesh
under-resolves the gap's evanescent field. The gallery ships a gap of 0.05,
where the beam into the block is plain; at 0.1 only 5% tunnels.

Claims at edge 0.08, 8 s from rest, with the fiber's guided power from
x = 0.6 to 0.9 against the vacuum block:

- Over a gap of 0.05 the fiber keeps under 70% (60%).
- The leak's rate falls from 0.05 to 0.1 by `e^{−2γ·0.05}` within 30%
  (0.096 against 0.113).
- Over 0.15 it keeps more than 99% (99.65%).

### 22. Talbot carpet

Mechanical. A channel lit by a launcher at the left, 4 Hz. A grating of
reflecting bars at x = −0.5, period 0.4, half open, with its slits centred
at `y = 0.2 + 0.4k`: the reflecting walls sit on slit centres, where the
grating is mirror-symmetric, so the channel holds the infinite grating
rather than the plan's four periods with their edges. Line probes across
the channel at the half and full Talbot distances.

Explored. Only the orders 0 and ±1 propagate, `λ/a = 0.625`, so the image
comes back at `λ/(1 − √(1 − (λ/a)²)) = 1.14`, not the plan's paraxial
`2a²/λ = 1.28`: the slits-over-bars contrast along the axis peaks at
`z/z_T = 0.97`. At half that distance, x = 0.07, the profile peaks on the
bars (0.054 against 0.008 at the slits); at `z_T`, x = 0.64, on the slits.
Averaged over the slit and bar centres: 3.9× and 3.6× at edge 0.08, 4.0×
and 3.5× at 0.05; at the paraxial distance, 1.7×.

Claims at edge 0.08, 8 s from rest:

- Halfway to the Talbot distance the bars are more than 2.5× the slits
  (3.9×).
- At the Talbot distance the slits are more than 2.5× the bars (3.5×), and
  more than 1.5× the contrast at the paraxial distance (1.7×).

### 23. Drum modes

Mechanical. A membrane of radius 0.6, the inside of a circle whose inner
face is clamped, `u = 0` (a Dirichlet condition), with everything outside
cut away: the plan's "reflecting circle" is a free rim in this skin, whose
`(2,1)` mode sits at `j'₂₁`, not the `j₂₁/(2πa) = 1.362 Hz` it names. A
velocity loss of 0.1 per second on the membrane so a steady state exists,
and a point source on one lobe, at (0.3, 0), driven at exactly
`j₂₁/(2πa)`. Point probes on another lobe and on a nodal diameter.
Trapped modes on purpose.

Explored. The mesh's resonance is within 0.1% of the Bessel zero: the lobes
are 0.086, 0.128, 0.114 at 1.35, 1.36, 1.37 Hz. With the plan's loss of
0.3 the resonance is 0.05 Hz wide, and the modes the point source excites
too, the `(0,2)` 0.1 Hz above among them, keep 14% of the lobes' RMS on the
nodal diameters; the nodes themselves sit exactly at 45°, 135°, 225°, 315°.
That background stays near 0.007 while the lobes grow with the resonance's
sharpness: 7.7% at a loss of 0.15, 5.4% at 0.1, 5.0% at 60 s. At 0.1 the
lobes are 95% of their final size by 45 s; edge 0.05 gives the same.

Claims at edge 0.08, 45 s from rest:

- The RMS along the two nodal diameters is under 10% of the RMS along the
  four lobes' radii (5.4%).
- `|U|` round the circle through the lobes has four maxima.
- Driven 0.01 Hz either side, the lobes are weaker.

### 24. Disordered crystal (was: Anderson localization)

TM. The photonic crystal's channel, launcher and fifty octagonal rods, but
at centres drawn at random with a fixed seed (`disordered_sites`,
xorshift64*) over the same slab, kept 0.14 apart (nearer, the gap between
two rods forces elements and a time step far below the rest) and clear of
the walls. Lit at 2.5 Hz, where the ordered crystal passes 80%. A line
probe across the channel behind the slab, on the claim's cut. (A point
probe on the axis sat under the line's badge; the view pass dropped it.)

The name changed: a slab one unit deep with fifty rods cannot tell Anderson
localization from the scattering that leads to it, and the plan's claims,
transmission against the crystal's and the log-transmission against depth,
measure extinction, which falls exponentially in any disordered medium.

Explored, against the empty channel: at 2.5 Hz the ordered rods pass 80%,
the random ones 11.6% of the power, and the plane wave, from the phasor
averaged across the channel, 0.6% (13.5% and 1.0% at edge 0.05; 12.3% and
1.7% at 24 s). At 1 Hz, long waves beside the rods, the random slab passes
82%, 78% as the plane wave; in the crystal's gap at 2 Hz it passes 2.4%
where the crystal passes 0.1%. The depth series failed to mesh: the first
0.6 of the random slab, 35 rods, stalls the mesher ("topology ear clipping
stalled"), as the axis-aligned square rods did in scene 13. Fixed on
2026-09-27: a second bridge to a rod vertex was spliced at the wrong copy of
it; the slab cut every 0.05 from either side now meshes at edges 0.16, 0.08
and 0.05. The depth series itself has not been run.

Claims at edge 0.08, 12 s from rest, at 2.5 Hz:

- The random rods pass under a quarter of what the ordered ones do (15%).
- Of what they pass, under a quarter is still the plane wave (6%).

### 25. Skin depth

TM. A channel lit by a 3 Hz launcher at the left, and a slab 0.3 thick,
from x = −0.35 to −0.05, of an otherwise vacuum medium with a constant
electric loss of `γ = 2ω` (the primary row in TM). The solver's rate is
the damped wave equation's, `u_tt + γ u_t = c²∇²u`: each half step scales
the flux by `e^{−γh/2}`. So inside, `k = (ω/c)√(1 − iγ/ω)`: `Im k = 14.82`,
a depth of 0.068, and `Re k = 23.98`, crests 0.26 apart, where the good
conductor's `√(ωγ/2) = 18.85` would put the depth at 0.053. A line probe
along the channel through the slab, 0.3 off the axis, and a point probe
behind it on the axis, clear of the line.

Explored, from the phasor 0.02 to 0.18 inside the front face, before the
back face's reflection counts: the slope of `ln|U|` is 14.83 at edge 0.08,
14.76 at 0.05, 14.81 at 0.04, and the phase's 23.90, 23.98, 24.00. The
plan's 15% was far looser than the scene needs; about a hundredth of the
field gets through.

Claims at edge 0.08, 8 s from rest:

- The decay constant is `Im k` within 3% (14.81 against 14.82).
- The phase runs at `Re k` within 3% (23.88 against 23.98).
- The good conductor's `√(ωγ/2)` is more than 15% off the measured decay
  (21%).

### 25b. Plasma skin depth

Added at the user's request on 26 September, as scene 25's lossless twin.
TM. The same channel, launcher and slab, the slab a Klein-Gordon plasma
with a 4 Hz cutoff above the 3 Hz drive. Below its cutoff a cold plasma is
a mirror: the field reaching into it falls as `e^{−κx}` with
`κ = √(ω_p² − ω²)/c = 16.62`, a depth of 0.060 beside the lossy slab's
0.068, while `c/ω_p`, the depth far below the cutoff, would say 0.040. This
is the collisionless, or inertial, skin depth; a Debye length would need
the thermal charges this cold model has no pressure for. The slab and its
probes are the lossy slab's, through `slab_channel`.

Explored, at edges 0.08 and 0.05: the slope of `ln|U|` 0.02 to 0.18 inside
is 16.634 and 16.642; the phase turns 0.018 rad across that stretch, where
the lossy slab's turns 3.83; and in front the standing wave's least `|U|` is
0.014 of its greatest, 0.011 at edge 0.05, where the lossy slab's is 0.486,
its `(1 − |r|)/(1 + |r|)` exactly.

Claims at edge 0.08, 8 s from rest:

- The decay constant is `κ` within 3% (16.63), and `c/ω_p` is more than 15%
  off it (51%).
- The phase turns under 0.1 rad inside (0.018).
- In front, the standing wave's nodes keep under a tenth of its peak
  (0.014).

## Batch E: nonlinear extras

### 26. Spatial soliton

TM. A slab of the "Saturable medium" preset at its defaults (χ = 0.8,
saturation 1) from x = −0.6 to 0.6, half-height 0.9 so it stops short of
the outgoing walls. A 4 Hz Gaussian beam, `exp(−(y/w)²)` with w = 0.2, from
a wall-to-wall launcher at x = −0.85, strength 1000; the weak beam is the
same at 10. Walls second-order outgoing. Probes along the axis to 0.55,
inside the slab, and across the beam just behind it, at x = 0.65 (a probe on the face itself
would sit on a boundary with two traces).

The plan's risk came true: without dispersion the third harmonic is
phase-matched and builds, 17% of the fundamental on the axis at mid-slab
at 3 s and 35% by 20 s, with the stored energy still climbing at 20 s. At
the user's choice the slab is also a Klein-Gordon medium with a 1.5 Hz
cutoff and `ε = 1/(1 − (1.5/4)²) = 1.164`, so `n² = ε(1 − f₀²/f²)` is 1 at
4 Hz and the slab stays invisible to the weak beam, while 12 Hz runs out of
step within about 0.6: the harmonic is 5% at mid-slab and 13% at the exit
at 3 s, 7% and 11% at 12 s, and the energy levels off at 12 s.

Explored at edges 0.08, 0.07 and 0.05 (without the dispersion): the weak
beam's RMS width at the far face 0.3277, 0.3273, 0.3271 (Gaussian optics
0.305); the strong beam's 0.156, 0.157, 0.152. The strong beam is held, not
focused: from x = 0 to 0.6 its width stays at 0.16 while the weak beam's
grows by half. With the dispersion it breathes a little, 0.15 to 0.18 at
12 s. Its steady state is chaotic at this strength: two CPU runs whose
strengths differ by 1e-7 part at about 0.9 per second, so the device leaves
the reference after about 1500 steps as any two runs do (0.15 at 3000
steps), where the Kerr slab keeps 4.8e-5.

Claims at edge 0.08, 3 s from rest, against the weak beam:

- At the far face the strong beam is under 0.6 of the weak one's width
  (0.47).
- Across the slab's second half the strong beam widens by under a fifth
  (2%) and the weak one by over 40% (53%).

### 27. Doppler mirror

TM channel. A 1 Hz launcher at x = −0.85 and a wall-to-wall slab from
−0.55 to 0.75 carrying the "Travelling modulation" preset on ε: depth 0.2,
pump 2 Hz, wavenumber `4k = 8π`, angle π, so the grating runs toward the
source at `c/2`. Incident `(ω, k)` plus the grating's `(2ω, −4k)` is
`(3ω, −3k)`, on the light line: the Bragg condition off a mirror moving at
`c/2`, which returns `f(c + v)/(c − v) = 3f`. Nothing else is
phase-matched. At 1 Hz the grating's period 0.25 and the 3 Hz wavelength
0.33 are both resolved at edge 0.08. The background's floor anchor moves
beyond the slab, which crosses the centre. Probes: points in front of the
slab (−0.7, 0.3) and behind it (0.9, 0.3).

The launcher starts on a cosine, as every launcher now does. Started on a
sine, as they all did before this scene, its plane wave carries a static
part as large as the wave (0.191 at every probe), which a channel keeps
because a constant satisfies its outgoing walls, and the grating turned it
into lines at 2 and 4 Hz (0.19 and 0.014 of the carrier). A cosine start
leaves 4e-5 and removes both, with the odd lines unchanged. The claims of
the other launcher scenes above were measured again with the cosine start;
the numbers in their claim lists are those, and their exploration notes
keep the sine-start values.

Explored at depths 0.1, 0.2 and 0.3: the 3 Hz line in front is 0.58, 1.00
and 1.23 of the carrier; at rest 2e-4; with the grating running away 0.03.
The powers follow Manley-Rowe at every depth: the reflection is three times
the power the carrier lost, 0.605 predicted against 0.587 measured, 1.065
against 1.053, 1.36 against 1.35.

Claims at edge 0.08, 8 s from rest, the lines over the last 4 s:

- In front, the 3 Hz line is over half the carrier (1.04) and over five
  times any other line (7.4, the 5 Hz one); with the grating at rest it is
  under a hundredth (4e-4).
- The transmitted power and a third of the reflected make the incident
  power within 5% (0.988), while the two powers make over 1.5 of it
  (1.71): the grating's work.

## Batch F: pulses, spectra and transfers

Proposed on 30 September 2026, once signals took pulses and probes read
spectra and transfers, agreed with the user the same day, and built that day
in the order below. Where a scene departs from its proposal the section says
why: the struck drum's control, the crystal's pulse train, the temporal
slab's direction and the ellipse's shape. After F3 the user found the
spectra and the transfer read better linearly, and every one is linear. The
gallery's rows went from six tiles to seven with the cavity filter, so each
section still sits in one row. Where a scene
is about lines, its probe opens on a spectrum; where it is about a frequency
response, on a transfer; where it is about arrival or refocusing, on a pulse.

Rules the transfer scenes keep, from measuring the readout on the user's
scenes (`docs/engineering-log.md`, 30 September):

- A source reference includes the source's own radiation, and a point source
  in the plane radiates very differently across a band, so a transfer that
  should read the medium divides one probe by another.
- A probe in front of a sample hears the reflection too. The transmission
  scenes are built with a *reference arm*: a reflecting wall along `y = 0`
  splits the channel in two, which the channel's mirror symmetry allows, one
  launcher feeds both arms, the sample sits in the upper one, and a probe
  behind it is divided by a probe at the same place in the empty one: the
  transmission itself, as a double-beam spectrometer reads it.
- The readout's segments must outlast the delay between the two records and
  any ringing many times over, or the average reads low. The claims are
  measured on the whole pulse, unwindowed (`transfer_spectrum`).
- A scene that is pumped or nonlinear has no transfer function; it reads
  spectra.

### F1. Scenes open on their spectra

The scenes whose claims are lines open with the spectrum under the probe's
trace: the Kerr slab (to 10 Hz), the time crystal and the travelling
modulation (over 4 s, so lines a hertz apart stand clear), and the Doppler
mirror's front probe (to 5 Hz). The self-sustained emitter gains a probe in
the plasma at the point its claim reads, and the Josephson line one down the
line. Every gallery spectrum and transfer is linear: the first three opened
in decibels, and the user found them all read better linearly. Each claim test
now also reads the spectrum the readout draws, over its own span, and finds
the lines there (measured at the claims' own meshes):

- Kerr slab: the third harmonic at 0.29 of the fundamental, 15 times the
  level at 5 Hz.
- Time crystal: the sidebands at 1.5, 3.5 and 5.5 Hz at 0.20, 0.88 and 0.20
  of the carrier.
- Travelling modulation: 3.5 Hz at 1.7 times the carrier, 5.5 Hz at 7.5.
- Doppler mirror: 3 Hz at 1.04 of the carrier, 7 times any other line.
- Self-sustained emitter: the strongest line at 2.99 Hz.
- Josephson line: the strongest line at 0.46 Hz, against `V/2π` = 0.477, over
  a 10 s span whose resolution is 0.1 Hz.

### F2. Struck drum

Resonators, after the drum modes. The drum's membrane, radius 0.6, clamped,
velocity loss 0.1/s, knocked at its centre every 20 s: a point source 0.03
wide whose rate is a Gaussian 0.05 s wide with nothing but an offset under
it, a velocity impulse, whose spectrum is still 60% of its peak at 3 Hz.
Struck at the centre it excites only the round modes, which stand at
`j₀ₙ/(2πa)`: 0.638, 1.464, 2.296 and 3.128 Hz, in the ratios 1 : 2.30 : 3.60
: 4.90. A probe at (0, 0.08), where every one of them moves nearly as much
as at the centre, opens on its field and its spectrum over 20 s, linear to
4 Hz.

The control planned first, the knock moved off-centre so the modes with
diameters join in, cannot be heard beside the centre, where those modes
vanish. What the scene is about, a drum's overtones not being a string's, is
measured instead at the string's: whole multiples of the fundamental.

Claims at edge 0.08, 20 s from one knock (the lines the same at 0.05):

- The probe's four strongest lines sit at the round modes within 0.5%
  (0.17%, 0.05%, 0.05% and 0.05%), each over a third of the strongest
  (0.53), and its readout, over its own 20 s, finds each within a quarter of
  its window's main lobe.
- At 2, 3 and 4 times the fundamental the readout shows under 5% of the
  fundamental's line (1.9%, 0.26% and 0.70%).

### F3. Photonic crystal, in a reference arm

Scene 13 revised. `Builder::arms` splits the channel along `y = 0` with a
reflecting wall welded to both outgoing ends, crossed by the launcher's two
dividers at junctions, so the launcher is two strips, one per arm, carrying
one signal. Every face is named from the floor or the ceiling. The crystal
keeps its pitch, rods and columns and fills the upper arm with five rows,
whose walls still sit on the lattice's mirror planes. A sinc pulse flat from
0.8 to 3 Hz lights both arms; "Behind the crystal" at (0.75, 0.5) opens on
its field and its transfer from "Reference" at (0.75, −0.5), linear to
3.5 Hz. The continuous-wave text, "tune the launcher to 1 Hz or 2.5 Hz", is
what one pulse now shows at once.

Explored on the CPU, at edge 0.08:

- **The arms.** Both empty, the probe-to-probe transfer is 1 to 2.5e-3 across
  0.8 to 3 Hz.
- **The crystal, one whole pulse.** 0.024 at 1.85 Hz, 0.06% of the power
  where the continuous wave measured 0.08%; 0.999 at 1 Hz and 0.825 at
  2.5 Hz. At the band structure's edges, 1.375 and 2.225 Hz, 0.22 and 0.23;
  half is crossed at 1.32 and 2.26 Hz. The crystal rings at its band edges:
  5% of the energy behind it comes more than 8 s after the pulse, 0.1% after
  20 s.
- **The readout.** A train that puts several pulses in one segment carries
  power only at its harmonics, `k/repeat` Hz. With segments twice the
  repeat, the readout reads the whole pulse to within 9% there; between
  them it averages the two neighbours as complex numbers, and the band
  edges' slow light turns their phases up to 140° apart, so the passbands
  scallop: 0.36 against 0.99 at 1.25 Hz for a 6 s repeat. A single pulse
  sits at the start of every segment that holds it, where the taper weighs
  it less than the response after it, and reads 1.3 to 5 times high. A
  repeat as long as the segment keeps one pulse to a segment: at 16 s the
  readout is within 26% but 43% low on the 2.3 Hz peak; at 20 s within 5%
  across the gap and below it and within 15% above it; at 30 s within 18% at
  worst. What stays off at 20 s is the
  crystal's narrowest transmission peaks, about 0.1 Hz wide, finer than a
  20 s Hann window resolves: 14% and 25% low at 1.25 and 2.3 Hz. The scene
  repeats every 20 s and reads 20 s segments.

Claims at edge 0.08:

- Both arms empty, the transfer is within 1% of one (0.25%).
- One whole pulse, unwindowed, 30 s: under a tenth from 1.5 to 2.1 Hz
  (0.068 at most); under a third at 1.375 and 2.225 Hz (0.22, 0.23); over
  0.95 at 1 Hz (0.999) and over 0.7 at 2.5 Hz (0.825).
- The readout's own average, a pulse a segment for 80 s: within 12% of the
  whole pulse from 1.45 to 2.1 Hz (6.7%), and within 10% at 1 and 2.5 Hz
  (3.0% and 5.9% low).

### F4. Etalon

Interfaces and media, after the Brewster angle. A ceramic slab, `ε = 9`,
`d = 1/6`, from `x = −0.1` across the upper arm: `nd = 1/2`, so a round trip
inside is one second. Each face reflects `(n − 1)/(n + 1)`, half the field,
and the slab passes `1/√(cos²δ + ¼(n + 1/n)² sin²δ)`, `δ = 2πfnd/c`: one at
every whole hertz and 0.6 halfway between. (An index of 2 would swing only
from 1 to 0.8, too little in amplitude, which is what the readout shows.) A
sinc pulse flat from 0.5 to 3.5 Hz shows three fringes; "Behind the etalon"
reads its transfer from "Reference", linear to 4 Hz.

Explored on the CPU:

- **Mesh.** The slab's shortest wavelength is 0.095 at 3.5 Hz. At edge 0.08
  the 3 Hz fringe reads 0.55; at 0.05 the fringes are within 2% to 3 Hz and
  12% at 3.5 Hz; at 0.04 within 2.5% across the band. The application
  refines to six elements per wavelength by itself.
- **Ringing.** Behind the slab 7% of the energy comes more than 4 s after the
  pulse, 5e-4 after 6 s.
- **The readout.** A pulse every 8 s read in 8 s segments reads the troughs
  (0.61) but the peaks at 0.93: the slab's echoes, a second apart, read low
  by the segment's overlap with itself shifted by them. A pulse every 16 s in
  16 s segments reads within 2% everywhere, the peaks at 0.98.

Claims at edge 0.04:

- One whole pulse, 20 s, follows the slab's `|t(f)|` within 3% from 0.5 to
  3.5 Hz (2.5%).
- The readout, a pulse a segment for two segments, reads the whole pulse
  within 5% from 0.6 to 3.4 Hz (2.0%).

### F5. Cavity filter

Asked for by the user on 30 September, when the plan was agreed. Resonators,
before the ring resonator. Two ceramic plates, `ε = 9`, each a quarter wave
thick at 2 Hz (`1/24`) and half a wave apart (`1/4`), across the upper arm
from `x = −0.15`: a Fabry-Pérot cavity whose mirrors are the plates. A
quarter-wave plate of index `n` reflects `(n² − 1)/(n² + 1)`, 0.8, of the
field, and between two of them the cavity passes all of it at 2 Hz, over a
half-power band 0.22 Hz wide, and 0.29 at 1.5 and 2.5 Hz. A sinc pulse flat
from 1 to 3 Hz, every 24 s; "Behind the filter" reads its transfer from
"Reference", linear to 3.5 Hz.

Designed with the transfer matrices first. Mirrors of three quarter-wave
layers (ceramic, vacuum, ceramic) stop far more either side, 0.05 at
`ε = 9`, but narrow the passband to 0.02 Hz, which rings for 16 s and which
no segment the readout can take resolves; at `ε = 4` they pass 0.2 to 0.3
either side over 0.09 Hz, still too narrow, and bring the next orders in at
1 and 3 Hz. Single plates keep one passband in the band and ring for about
1.4 s.

Explored on the CPU: at edge 0.04 one whole pulse is within 0.5% of the
plates' `|t(f)|` from 1 to 3 Hz. Behind the plates 2.7% of the energy comes
more than 6 s after the pulse starts, 0.17% after 8 s. The readout reads
the peak at 0.93 over 16 s segments and 0.965 over 24 s, the window's main
lobe smoothing a band 0.22 Hz wide.

Claims at edge 0.04:

- One whole pulse, 20 s, follows the plates' `|t(f)|` within 2% from 1 to
  3 Hz (0.5%).
- The readout, a pulse a segment for two segments, reads the whole pulse
  within 5% from 1.1 to 2.9 Hz (3.5%, at the peak).

### F6. Plasma group delay

Interfaces and media, after the plasma skin depth. The Klein-Gordon plasma,
cutting off at 2 Hz, from `x = −0.7` to 0.6 across the upper arm; its
material is `tm_plasma`, shared with the plasma skin depth. A Gaussian pulse
0.5 s wide at 3 Hz every 8 s: its spectrum is down to 1% by 0.95 Hz either
side, all of it above the cutoff. There its energy runs at `v_g =
c√(1 − (f_c/f)²)` = 0.745c and its crests at `v_p = c²/v_g` = 1.342c, so
behind 1.3 of plasma it arrives `L(1/v_g − 1/c)` = 0.444 s after its twin
in the vacuum arm. "Behind the plasma" and "Reference" open on their fields
over 4 s, where the lag shows.

Explored on the CPU: the slab's faces reflect 15% of the field, and its own
group delay, `−dφ/dω` of its transfer against as much vacuum, ripples about
`L(1/v_g − 1/c)` by about a tenth of a second (0.63 s at 2.6 Hz against
0.73, 0.32 at 3.2 against 0.37). The transfer's group delay follows the
ripple: within 6% at edge 0.08 and 3.2% at 0.05. The energy centroids lag by
0.454 s at 0.08 and 0.467 s at 0.05. Along 0.9 of the plasma, `x` from −0.5
to 0.4, the 3 Hz phase of the whole record runs at 1.349c and 1.358c; a fit
over two wavelengths averages out the standing ripple the back face leaves.

Claims at edge 0.05, 12 s from one pulse:

- The energy behind the plasma trails its twin's by `L(1/v_g − 1/c)` within
  10% (0.467 s against 0.444).
- The transfer's group delay follows the slab's within 5% from 2.6 to 3.4 Hz
  (3.2%).
- The 3 Hz crests run at `c/√(1 − (f_c/f)²)` within 2% (1.2% fast).

### F7. Temporal slab

Time-varying media, after the parametric pump. Glass, `ε = 4`, wall to wall
from `x = −0.6` to 0.9, carrying the "Parametric pump" preset on its mass row
at 0 Hz, depth 0.75, phase π, under a flat-top gate 2 s long with a 1 ms edge,
shorter than a step: `1 − 0.75`, so the permittivity drops from 4 to 1 at
once and comes back 2 s later. The planned direction, 1 to 4, cannot be
authored: a pump's depth stays under one. A Gaussian pulse 0.2 s wide at
1.5 Hz from the launcher at −0.85 enters the glass at 1.15 s, runs at half
the wave speed, 0.8 long, and is centred at `x = 0.1` when the drop comes at
2.55 s. Pulse and drop repeat every 6 s. Probes "Upstream" at −0.45 and
"Downstream" at 0.65, either side of the pulse then, open on their fields
over 4 s.

Across a change in time the step keeps the canonical state, `Q = D` and
`b = B`, so the wavenumber stays and the frequency doubles. With `n₁ = 2` and
`n₂ = 1` the pulse splits into `E_f = ½(n₁/n₂)(1 + n₁/n₂)` = 3 of its field
running on and `E_b = ½(n₁/n₂)(n₁/n₂ − 1)` = 1 running back. Each discrete
mode splits the same way, so the mesh's dispersion does not enter a ratio of
`∫u² dt` at a probe: the reflected pulse carries `E_b² n₂/n₁` = 0.5 of the
incident's, as much field over half the time, and the pulse running on
`(E_f/E_b)²` = 9 times the reflected one. While the permittivity is down the
glass is vacuum, and its faces reflect nothing.

Claims at edge 0.08, 4.5 s from rest:

- Upstream the reflected pulse carries 0.5 of the incident's `∫u² dt` within
  3% (0.498).
- Downstream the pulse running on carries 9 times the reflected one's within
  3% (9.10).
- Both come at twice the incident's spectral centroid within 3% (3.03 and
  3.08 Hz against 2 × 1.546).
- With the permittivity held, under a thousandth comes back (3e-4).

As built the drive halved the step, 6.8e-3 s to 3.4e-3 s, the driven ceiling
lowering the whole domain by the weakest factor anywhere. Since the ceiling
is taken node by node (`docs/engineering-log.md`, 30 September) the glass,
never faster than the vacuum beside it, leaves the step to the vacuum. The
claims were measured again at the new step; before it they read 0.497, 9.14
and 3.03 and 3.09 Hz.

### F8. Ellipse flash

Lenses and imaging, before the Fresnel zone plate. A Mechanical room inside a
reflecting ellipse, `a = 0.95`, `b = 0.8`, everything outside cut away, with
a velocity loss of 0.2/s so one flash has faded to 37% before the next. A
point source at the left focus, `x = −0.512`, flashes a Gaussian 0.1 s wide
at 2 Hz every 10 s, a sine carrier, so it leaves the closed room no offset.
Every path from one focus to the wall and on to the other is `2a` long: the
echoes reach the far focus together `2a/c` = 1.9 s after the flash, where
the direct pulse took `2√(a² − b²)/c` = 1.02 s. Probes at the far focus and a
quarter beside it open on their fields over 4 s.

Explored first at `b = 0.6`, whose foci sit further out: the echoes peaked at
the far focus 1.903 s after the flash and at 5 times the direct pulse, but
the two arrive only `2(a − √(a² − b²))/c` = 0.43 s apart, and the direct
pulse ran into the echoes. At `b = 0.8` they are 0.88 s apart.

Claims at edge 0.05, 5 s from one flash, the largest swing within 0.3 s of
each arrival:

- At the far focus the echoes peak `2a/c` after the flash's centre within 1%
  (1.898 s), over six times the direct pulse there (8.8; 8.9 at edge 0.04).
- A quarter beside the focus they reach under a fifth of the focus's peak
  (0.14).

## Batch G: the rest of the pulses and gates

Proposed on 30 September 2026 after batch F, from its "Still open" list, and
agreed with the user: an echo comb, a pumped drum, and pulsed Doppler to be
explored first and built only if a clean claim holds. The chopper, plain
amplitude modulation, and the plasmonic guide, which needs a Drude law on
the complementary row, stay out.

### G1. Echo comb

Basics, after the obstacle over a mirror. The reference arm with a mirror
(`Builder::arms` takes one now): a reflecting wall across the upper arm at
`x = 0.95`, the face past it cut away. Probes at `x = 0.45` in both arms; a
sinc pulse flat from 0.5 to 3.5 Hz every 16 s. The upper probe hears the
pulse and then, `τ = 2d/c` = 1 s later, its echo, which a reflecting face
in the E_z skin returns in phase, so over the reference it reads `1 +
e^{−iωτ}`: `2|cos(πfτ)|`, teeth of two at every whole hertz and gaps of
nothing halfway between. It is the comb that made the user's first
transfers, read in front of walls, not flat.

Explored on the CPU: the whole pulse reads the teeth at 2.00 at edge 0.08,
but the gaps at 0.12 and 0.26, a phase error of about 0.08 rad per hertz on
a round trip a second long, the mesh's dispersion and the probe's nearest
node; at edge 0.05, 0.01 and 0.05. The readout over 16 s segments reads the
echo low by the segment's overlap with itself shifted a second, `R(1/16)` =
0.975, so its teeth stand at 1.975.

Claims at edge 0.05:

- One whole pulse, 20 s, is within 0.06 of `2|cos(πfτ)|` from 0.6 to
  3.4 Hz (0.048).
- The readout, a pulse a segment for two segments, puts its teeth at `1 +
  R(1/16)` within 2% (1.977 to 1.982 against 1.975) and its gaps under 0.1
  (0.03 and 0.05).

The device run at 1000 steps fails the long run's relative measure, and the
scene is not at fault: by 5.5 s both halves of the pulse have left the
domain, the reference's field falls about 200-fold from step 800 to 1000
(`|b|` 19 to 0.099), and the device's absolute error stays at 6.1 to 6.6e-5 in
`b` throughout, about 1e-6 of the peak. At 800 steps, with the field still
in the domain, Q 3.2e-6 and b 3.4e-6.

## Order, as built

Built from 25 to 26 September in batch order, A to E, groundwork first.
CPU exploration in the scratchpad came before the scene text where a number
had to be found: the fluxon emission (7), the pacemaker's entrainment (9),
the clad disk's resonance (10), the fiber's β and stable depth (11), the
crystal's gap (13), the ring's resonance (15), the dielectric disk's
resonance (17) and the Doppler grating's depth (27). Every other scene took
its claim tolerance from its own first run. The pacemaker failed its
exploration and shipped as the self-sustained emitter.

Straight after batch E, on 26 September: a scene change stops the old field
at once (`docs/plan.md`, "Maintenance").

After the planned scenes, at the user's request: the GRIN collimator (scene
3) ran in the Mechanical skin and moved to the E_z skin as glass, `μ = 1`,
on 27 September, as the other optics scenes are. Its length is unchanged and
its claims were measured again (see scene 3).

## View settings

After the last scene, at the user's request on 26 September: a pass over
every gallery scene's view settings, so each opens showing what it is about
(field or energy, the vector overlays, the material overlay, exposure, the
readouts a probe opens with where that becomes storable). The toy is FEM
and vector-valued, and the scenes should show it.

Done on 26 September, judged on a screenshot of every scene 12 s from its
own launch. The pass also moved probes that overlapped
(`no_gallery_probe_sits_on_another`), and turned up that auto exposure
carried one scene's loudest level into the next. Since 27 September the
flow arrows are low-passed at the default 0.5 Hz, so these scenes open
showing the mean flow rather than its ripple; the log has the measurement.

- **Power-flow arrows** where the power has somewhere to go, at gain 2 (at
  1 most arrows were a few pixels):
  - the material lens and the Luneburg lens, converging on the focus;
  - the GRIN collimator's straight beam, since it moved to E_z (27
    September);
  - the anisotropic crystal, walking off its wavefronts;
  - the phased array's steered beam;
  - the bent fiber's shed beam;
  - the crystal bend's corner;
  - the ring's circulation;
  - the FTIR beam leaving into the block;
  - the two skin depths, forward and fading in the lossy slab and swinging
    about nothing in the plasma.
- **Electric-field arrows** in Brewster's TE skin, since the in-plane field's
  direction is the point there.
- **Material overlays:** the GRIN rod, the Luneburg lens and the fisheye show
  density or permittivity instead of wave speed. That keeps the vacuum at the
  bottom of the palette rather than painting the domain over. The
  anisotropy overlay is lighter (0.22), so the arrows inside read.
- **Decluttered:** the three rod lattices hide control polygons and handles.
- **The drum shows its mesh.**
- The rest keep the plain field, which is what they are about.

Each probe's readout is kept with its scene too, since readouts became
storable (see the log's "Probe readouts are kept with the scene"). Every
gallery probe opens on what its claim reads, one or two plots
(`every_gallery_probe_opens_on_one_or_two_plots`):

- **Point probes:** the field alone. On the plasma and dielectric
  galleries' opposite rims and on the drum, it covers the whole ten-second
  history, so the build-up shows.
- **Line and boundary probes:** the average energy density along them, where
  the fringes, the focus, the decay, the rim lobes, the Talbot images and the
  beam's width are read. The disordered crystal's cut adds the average flux
  integrated across it, the power through.
- **Area probes:** the ring's total energy over the whole history, as it
  fills; the Luneburg lens's total and its focus's mean energy density; and
  the mean energy density of the acoustic gallery's two disks.
- **Far fields** (double slit, phased array): the polar patterns alone.

## Sections

Grouped on 26 September, for onboarding: the gallery shows the catalogue in
sections, and the catalogue runs through them in order, so stepping from one
example to the next walks the gallery as it is shown
(`the_catalog_runs_section_by_section`):

- **Basics:** obstacle over a mirror, double slit, obstacle array, phased
  array, Talbot carpet. Batch G adds the echo comb after the obstacle over a
  mirror.
- **Interfaces and media:** anisotropic crystal, Brewster angle, skin depth,
  plasma skin depth, plasma mirror. Batch F adds the etalon after the Brewster
  angle and the plasma group delay after the plasma skin depth.
- **Lenses and imaging:** material lens, GRIN collimator, Luneburg lens,
  Maxwell's fisheye, Fresnel zone plate. Batch F adds the ellipse flash
  before the zone plate.
- **Guides and crystals:** bent fiber, frustrated TIR, photonic crystal,
  crystal bend, disordered crystal.
- **Resonators:** drum modes, the acoustic, dielectric and plasma whispering
  galleries, ring resonator. Batch F adds the struck drum after the drum
  modes and the cavity filter before the ring resonator.
- **Time-varying media:** parametric pump, time crystal, travelling
  modulation, Doppler mirror, parametric fiber amplifier. Batch F adds the
  temporal slab after the parametric pump.
- **Nonlinear and self-organizing:** Kerr slab, spatial soliton, Josephson
  line, symmetry breaking, pinned domain wall, self-sustained emitter.

The starter obstacle became "Obstacle over a mirror" then, since it had long
stopped being where the app starts.

## Still open

- **Plasmonic guide.** Proposed by the user on 25 September, after the
  planned scenes. A surface plasmon on a flat interface needs a permittivity
  that turns negative acting on the in-plane electric field. In E_z the
  Klein-Gordon plasma's negative ε acts on the out-of-plane field, which has
  no surface mode. In H_z the in-plane field is the complementary row, and
  Gate O's restoring laws act on the primary field only, which there makes a
  magnetic plasma. So the guide most likely needs a Drude law on the
  complementary row first. The user agreed on 27 September and left the
  analysis for later.

- **Gated-drive scenes.** Proposed on 29 September, once material drives
  took gates. The temporal slab is built (F7), as a drop in the index, which
  a pump's depth under one allows where a rise to four does not. Still
  proposals: a pump burst, a gated pump at twice a mode's frequency that
  amplifies it for a set time, the gain following the gate's length; and a
  chopper, a gated loss across a guide cutting pulses out of a continuous
  wave, which is amplitude modulation and was not recommended in batch F.

- **Pulse scenes not built.** Of the five the pulsed signals opened, batch F
  built the time of flight and the group delay as one scene (F6) and the
  ellipse (F8). Echoes, a pulse and its echo off a wall, whose spectrum is a
  comb, and pulsed Doppler, whether the moving grating returns a pulse three
  times shorter, remain; the grating reflects along its whole length, so the
  second needs exploring first.

## Blocked

- **A true polariton.** Needs a Lorentz restoring law with a second auxiliary
  state, which Gate O did not add. Scene 10 uses the Drude plasma the
  catalogue has.
- **Saturable and polynomial loss.** Still gated (D2), so no optical limiter,
  saturable absorber or loss-saturated oscillator. Saturation in this plan
  comes from Kerr detuning and from wall leakage only.
