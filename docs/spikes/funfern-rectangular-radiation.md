# Rectangular radiation, PH-08 — 17 October 2026

How far funfern's walls on its rectangular box are from exact radiation, for
sources at the centre, off it, beside an edge and beside a corner, of
several multipole orders and frequencies. The 11 October plan's PH-08 is a
one-off experiment here, since no nonlocal radiation operator is planned
(`funfern-testing-campaign-plan.md`, section 6). The scripts and how to run
them are in `experiments/rectangular-radiation/`.

## Summary

- **The walls' error is their law's.** On exact boundary data the wall's
  flux error agrees between edges 0.1, 0.07 and 0.05 to 0.8% for orders up
  to two 0.25 or more from a wall, and to 2.8% for order four; 0.05 from a
  wall the coarser meshes do not resolve the data. The solver's returned
  field changes by under 1% between meshes for monopoles and dipoles. What
  is left is the local law against the exact, nonlocal one.
- **The corners carry the second-order wall's error.** Its tangential
  operator runs round the box as one ring, straight through each corner. For
  orders up to two at the centre and off it, 46–97% of its boundary-flux
  error's square lies within 0.25 of a corner, a quarter of the boundary,
  and 78–97% for the monopole. A centred
  quadrupole whose standing field vanishes on the diagonals, at the corners,
  returns 1.0–1.6% of the field from the second-order wall; a centred
  monopole, which meets the corners at full strength, 6.5–18%.
- **Second order is not uniformly better.** It returns less beside a corner
  (10–21% against 17–27%), for quadrupoles everywhere (1.0–9.9% against
  4.2–24%), and at `k = 10` for every source but the centred monopole and
  dipole. The first-order wall returns less for a monopole at the centre
  (5.4–12% against 6.5–18%), for a centred dipole at `k ≥ 5`, and for a
  monopole off the centre at `k = 2` and 5. The boundary spike found as much
  on its finite box (`funfern-boundary-auxiliary-spike.md`, section 5: "no
  measured advantage over first order").
- **A local wall takes energy from the near field.** It reads a reactive
  near field as radiation and absorbs it. On exact boundary data the
  second-order wall would take 2.1–2.2 times the radiated power from a
  quadrupole 0.25 from an edge or 0.3 from a corner at `k = 2`, the
  first-order 7.3–7.7 times, and from an order-four source off the centre at
  `k = 2`, 19 and 102 times. In the solver, an order-two source 0.3 from an
  edge or 0.35 from a corner at `k = 2` sends 2.4 and 1.9 times its
  free-space power into the second-order wall, 11.5 and 11.9 times into the
  first-order one. At `k ≥ 5`, monopoles and dipoles anywhere send within 5%
  of their free-space power into either wall.
- **The ledger holds.** In every harmonic run the wall's time-averaged power
  equals the source's to 2e-3, the largest the order-two sources at `k = 2`
  on the coarsest mesh.

## Part 1: the boundary operator

For a source at `x₀`, `u = H_m(k|x − x₀|) e^{imθ}` is exact outside it, and
so is its normal derivative on the box's boundary Γ, which gives exact
Dirichlet-to-Neumann pairs without an exact operator. Production's walls are
read as assembled (`export_the_rectangles_wall_traces`): the trace nodes, the
first-order damping `D` (Simpson's lumps of `Z ℓ`, checked), and the
second-order wall's modes, `Tᵀ T = D`, each with its decay `d`. With `u` the
primary field in `e^{−iωt}`, `ω = k`, the boundary's weak flux is exactly

    F_exact,i = (i/ω) ∫_Γ φ_i ∂_n u        (six Gauss points on each quadratic edge)

and the wall's, on the exact field's nodal values `U`,

    F_law = −Tᵀ diag(Y(−iω, d)) T U,   Y = 1 + d (6/7 / s − 8/7 / (s + d) + 2/7 / (s + 2d)),

or `−D U` at first order. Read: the flux error in the `D⁻¹` norm relative to
the exact flux; the share of its square within 0.25 of a corner; and the
power the wall would take, `−½ Re Uᴴ F_law`, against the exact pair's,
which meets the analytic `2/ω` to 1e-5 wherever the mesh resolves the
boundary data. For orders one and up 0.05 from a wall it does not - the
pair's own power is off by 0.1% to 10⁷ - and those powers are not read.
Hankel functions from scipy, against mpmath to 9.2e-16.

Flux error on edge 0.05 (edges 0.1 and 0.07 agree to 0.8% on every row),
first and second order, and the corner share of the second-order error:

| Source | k = 2 | k = 5 | k = 10 |
| --- | --- | --- | --- |
| m = 0, centre | 0.253, 0.298 (0.81) | 0.179, 0.198 (0.93) | 0.164, 0.143 (0.97) |
| m = 2, centre | 0.806, 0.399 (0.60) | 0.288, 0.213 (0.87) | 0.194, 0.147 (0.95) |
| m = 4, centre | 1.182, 0.374 (0.21) | 0.652, 0.310 (0.61) | 0.279, 0.165 (0.87) |
| m = 0, off-centre | 0.323, 0.312 (0.78) | 0.254, 0.208 (0.89) | 0.241, 0.158 (0.90) |
| m = 0, 0.25 from an edge | 0.532, 0.327 (0.69) | 0.464, 0.246 (0.71) | 0.449, 0.221 (0.65) |
| m = 0, 0.3 from a corner | 0.594, 0.390 (0.70) | 0.529, 0.306 (0.63) | 0.513, 0.284 (0.48) |
| m = 2, 0.3 from a corner | 1.056, 0.290 (0.50) | 0.997, 0.413 (0.45) | 0.648, 0.336 (0.40) |
| m = 0, 0.05 from a corner | 0.927, 0.472 (0.90) | 0.918, 0.493 (0.72) | 0.931, 0.521 (0.51) |
| m = 0 and m = 2 at once | 0.856, 0.336 (0.47) | 0.352, 0.233 (0.77) | 0.273, 0.165 (0.79) |

The power the wall would take, against the radiated, on edge 0.05 where it
resolves the data: for monopoles and dipoles the second-order wall 1.05–1.17
at `k = 2`, 1.03–1.42 at `k = 5` and 1.02–1.39 at `k = 10`; the first-order
1.10–1.74, 1.12–3.16 and 1.12–2.41. For orders two and four, which meet the
near field, the second-order wall 1.11–19, 1.05–14 and 1.02–1.7; the
first-order 1.6–102, 1.2–68 and 1.1–2.8.

## Part 2: the full solver

`export_the_rectangles_harmonic_fields` drives the same box by a volume
source `(r/σ)ᵐ e^{−r²/σ²} cos mθ`, σ = 0.1, switched on over two periods,
held 15 s and two more, then reads every node's phasor by a Blackman-Harris
lock-in over six periods, with the wall's and the source's averaged power.
The source's exact free-space field, with `ψ̇ = u` and `S` its phasor, is

    U_free = −iω S (iπ/2) cos mθ [H_m(kr) ∫₀ʳ J_m g r′ dr′ + J_m(kr) ∫ᵣ^∞ H_m g r′ dr′],

`g = (r/σ)ᵐ e^{−r²/σ²}`, its integrals on a radial grid; their limit meets
its closed form `(σ²/2)(kσ/2)ᵐ e^{−(kσ)²/4}` to 5e-10. What the box returns
is `U − U_free`, read in the lumped-mass norm against `U_free`. A
least-squares fit of `U_free` to `U` gives 1.0004 at −0.001 rad where the
walls return least (the centred quadrupole at `k = 10`), so the source is
read as the solver applies it. The sources sit no nearer a wall than 0.3,
σ's reach; the two-source case scales its quadrupole by `(2/kσ)²` so that
both radiate alike.

Returned field on edge 0.07, first and second order. Edge 0.1 and, for the
monopoles at the centre and the corner, 0.05 agree to 1% for monopoles and
dipoles; the quadrupoles, whose return is smallest, move by up to 44%, the
solver's own dispersion showing beside so small a return:

| Source | k = 2 | k = 5 | k = 10 |
| --- | --- | --- | --- |
| m = 0, centre | 0.118, 0.182 | 0.066, 0.092 | 0.055, 0.065 |
| m = 0, off-centre | 0.147, 0.184 | 0.085, 0.100 | 0.081, 0.071 |
| m = 0, 0.3 from an edge | 0.178, 0.183 | 0.142, 0.107 | 0.123, 0.074 |
| m = 0, 0.35 from a corner | 0.266, 0.214 | 0.200, 0.145 | 0.169, 0.101 |
| m = 1, centre | 0.143, 0.143 | 0.068, 0.095 | 0.059, 0.071 |
| m = 1, 0.35 from a corner | 0.258, 0.180 | 0.204, 0.138 | 0.188, 0.116 |
| m = 2, centre | 0.065, 0.011 | 0.052, 0.014 | 0.042, 0.010 |
| m = 2, 0.3 from an edge | 0.227, 0.083 | 0.167, 0.099 | 0.121, 0.077 |
| m = 2, 0.35 from a corner | 0.194, 0.081 | 0.187, 0.092 | 0.171, 0.090 |
| two at once | 0.132, 0.101 | 0.087, 0.094 | 0.076, 0.069 |

The wall's power against the source's free-space radiation, second order:
0.83–1.18 for monopoles and dipoles at `k = 2`, 0.95–1.05 at `k = 5`,
0.96–1.04 at `k = 10`; the quadrupoles 1.08–2.37 at `k = 2` and 0.97–1.12
above. First order: 0.85–1.89 for monopoles and dipoles at `k = 2`, 0.95–1.02
above; quadrupoles 1.4–11.9 at `k = 2`, 0.93–1.44 above.

## What it means

The two parts agree on where the walls fail: at the corners, and wherever a
source's near field reaches a wall. A better local law on straight sides
would not touch the first. The corner is where the ring's tangential
operator is not the exterior's, and corner compatibility conditions, which
Hagstrom and coauthors construct for high-order local walls, or rounding
the box's corners, are where an improvement would pay. Neither is planned.
For scenes as they are: a monopole or a dipole 0.3 or more inside the box
sees 6.5–21% of its field come back from the second-order wall and 5–27%
from the first-order one, most of it from the corners, and more the nearer
a corner it sits. The second-order wall's advantage is for fields that meet
the sides obliquely away from the corners, not for a source at the centre.

## Limits

- Part 2's sources keep 0.3 from a wall, σ's reach; Part 1 goes to 0.05.
- Part 2 counts the solver's own dispersion with the walls' return; the
  meshes show it is small beside it except where the return is about 1%,
  the centred quadrupole, which falls from 1.6% to 1.1% at `k = 2` between
  edges 0.1 and 0.07.
- Part 1's norm and Part 2's measure different things - the boundary flux
  on exact data, and the field the solver settles to - so their numbers are
  not comparable one to one, only their pattern.
