# Gallery plan: Stage 11.5 and the scenes after it

**Date:** 25 September 2026

**Status:** proposed, awaiting review. Nothing here is built yet.

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

### 2. Starter obstacle

The description says "above a reflecting floor" but both walls reflect. The
top wall becomes outgoing; the floor stays.

### 3. GRIN collimator

The rod becomes a quarter-pitch collimator: a point source on its entrance
base leaves the other base as a collimated beam. The profile changes from
`smoothstep` to parabolic, `n = 1 + dn·max(0, 1 − (y/H)²)`, because the
smoothstep profile is flat to fourth order on the axis and has no paraxial
pitch. With `H = 0.3` and `dn = 0.6` the paraxial pitch is
`2πH·√((1 + dn)/(2dn)) = 2.18`, so the rod is 0.55 long: x from −0.55 to 0,
y from −0.3 to 0.3, rectangular. The source sits inside the rod on the
entrance base at (−0.54, 0), 4 Hz. Mass and stiffness stay reciprocal so the
impedance is matched, as today.

Claims: the phase of the field at 4 Hz along x = 0.05 is flat to λ/8 over
the central 70% of the aperture; at x = 0.6 at least 70% of the RMS power
across the domain height lies within the aperture, against under 35% for the
bare source. Rays far from the axis have a shorter pitch, which is why the
claim is on the central 70%.

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
wavenumber `2β = 43.88`, phase `3π/4`, angle 0. A 2.5 Hz point source of 4 on
the axis at x = −0.85, a probe at x = 0.85, and a line probe along the axis
from x = −0.75, past the source's near field, to 0.9, whose energy density
shows the signal growing along the fiber. Walls outgoing.

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

Claims at edge 0.08, at the far end at 2.5 Hz over 2 s windows:

- The travelling pump amplifies the signal more than 4× against the pump
  off (4.85×), and steadily: at 14 s the gain is within 10% of 8 s (4.63×).
- Advanced by half a turn it squeezes the signal below half (0.13×).
- Uniform in space, the same pump makes the fiber oscillate: its far end
  grows more than 3× from 8 s to 12 s (5.7×).

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
line probe runs along the midline between two rows, and a point probe
reads the transmitted wave 0.25 behind the crystal.

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
a point probe on the image. The view shows the wave speed. Walls outgoing.

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
A point probe on the focus and a line probe along the axis behind the
screen. Walls outgoing.

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
the walls. Lit at 2.5 Hz, where the ordered crystal passes 80%. A point
probe behind the slab and a line probe across the channel.

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
stalled"), as the axis-aligned square rods did in scene 13. Reported, not
fixed.

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
along the channel through the slab, a point probe behind it.

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
same at 10. Walls second-order outgoing. Probes along the axis and across
the beam just behind the slab, at x = 0.65 (a probe on the face itself
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

## Order

Batch A first, including the groundwork and the todo. Then B, C, D, E. Before
their scene text is fixed, CPU exploration in the scratchpad settles: the
fluxon emission (7), the pacemaker's entrainment (9, which failed), the clad disk's
resonance (10), the fiber's β and stable depth (11), the crystal's gap (13),
the ring's resonance (15), the dielectric disk's resonance (17) and the
Doppler grating's depth (27). Every other scene gets its claim tolerance from
its own first run.

Straight after batch E: a scene change stops the old field at once (the
first item under "Maintenance" in `docs/plan.md`).

After the last scene, at the user's request on 26 September: a pass over
every gallery scene's view settings, so each opens showing what it is about
(field or energy, the vector overlays, the material overlay, exposure, the
readouts a probe opens with where that becomes storable). The toy is FEM
and vector-valued, and the scenes should show it.

Revisit after the planned scenes, at the user's request: the GRIN collimator
(scene 3) runs in the Mechanical skin and should run in an EM one, as the
other optics scenes do. In TM that means ε = μ = n for its impedance-matched
profile, not the Mechanical `1/n` stiffness (see scene 11); its quarter-pitch
length and claims are re-measured then.

Proposed by the user on 25 September, after the planned scenes: a plasmonic
guide. To scope first: a surface plasmon on a flat interface needs a
permittivity that turns negative acting on the in-plane electric field. In
TM (`E_z`) the Klein-Gordon plasma's negative ε acts on the out-of-plane
field, which has no surface mode; in TE the in-plane field is the
complementary row, and Gate O's restoring laws act on the primary field
only, which there makes a magnetic plasma. So the guide likely needs a Drude
law on the complementary row first.

Catalogue count after everything: 12 when the plan began, 36 with the
planned scenes and 25b; the catalogue test's count moves with each commit.

## Blocked

- **Pulses.** `TimeSignal` is harmonic only, so a document cannot hold a
  pulse. That blocks time of flight, group delay in the plasma, echoes, an
  ellipse refocusing a flash and pulsed Doppler. Filed in `docs/plan.md`
  under "Later experiments": a windowed harmonic or a burst envelope on every
  signal consumer, persisted.
- **A true polariton.** Needs a Lorentz restoring law with a second auxiliary
  state, which Gate O did not add. Scene 10 uses the Drude plasma the
  catalogue has.
- **Saturable and polynomial loss.** Still gated (D2), so no optical limiter,
  saturable absorber or loss-saturated oscillator. Saturation in this plan
  comes from Kerr detuning and from wall leakage only.
