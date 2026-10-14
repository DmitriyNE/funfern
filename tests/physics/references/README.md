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
