# Physics references

Reference values the physics tests read, for what is not a closed form: rule
7 of `docs/spikes/funfern-testing-campaign-plan.md`. A closed form, a few
lines of arithmetic in the test's own terms, lives in its test instead.

Each table sits beside the generator that wrote it, which checks the
identities that validate it before writing. Generators run by hand when a
table's cases change; CI runs none of them, and the tests read only the
tables.

- `cylinder-hankel.json`, from `cylinder_hankel.py` (mpmath): Hankel
  functions of the first and second kind for PH-06 and PH-07, the circular
  outgoing wall and the absorbing hole, at the radii each case fits at and at
  the wall, each checked by the Wronskian and against a second evaluation at
  twice the precision.
- `cylinder-hankel-dense.json`, from the same generator: `H1_m` and its
  derivative on the unit circle for orders 0 to 4 on a grid of kR by 0.01,
  each from `max(3/2, m)` to 14, for PH-09's pulses, which the test
  interpolates by cubic Hermite. Checked by the Wronskian, at twice the
  precision, and by that interpolation at every midpoint to 1e-9.
- `cylinder-scattering.json`, from `cylinder_scattering.py` (mpmath): PH-12's
  scattering ratios `S_n` for reflecting holes, pinned cylinders and
  penetrable ones, by size `k a`, index and the nodal field's stiffness
  ratio, with each ratio's slope in `k a` and `J_n`, `H1_n` at the radii the
  test fits at. Checked by the Wronskian, at twice the precision, every
  ratio by unitarity, `|1 + 2 S_n| = 1`, and every slope along it and by
  carrying its ratio to a circle 2e-4 smaller to 1e-5.
