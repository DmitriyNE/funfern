# Rectangular radiation, PH-08

A one-off experiment, not a regression test: no nonlocal radiation operator
is planned, so nothing in the gate holds these numbers. The report is
`docs/spikes/funfern-rectangular-radiation.md`.

The solver exports, from two ignored tests in
`crates/funfern-app/src/topology_examples.rs`:

- `export_the_rectangles_wall_traces`: each wall's trace as production
  assembles it on the box `[-1, 1]²` of a unit medium, at edges 0.1, 0.07
  and 0.05: the trace nodes, the first-order damping, and the second-order
  wall's modes, each its decay and its row of `T`.
- `export_the_rectangles_harmonic_fields`: 168 harmonic runs of the walled
  box driven by Gaussian multipoles, every node's steady-state phasor with
  the wall's and the source's time-averaged power. `FUNFERN_PH08_ONLY` keeps
  the runs whose file name holds it.

Two scripts read them against exact fields:

- `boundary.py`: the wall's weak boundary flux on the exact outgoing
  multipole's boundary values against the exact flux, the radiated power
  against the exact pair's and that against `2/ω`.
- `harmonic.py`: the solver's field against the source's exact free-space
  field, the wall's power against the power the source radiates.

Both read Hankel functions from scipy, checked against mpmath in the run.

```sh
python3 -m venv venv && venv/bin/pip install numpy scipy mpmath
PH08_PYTHON=venv/bin/python bash experiments/rectangular-radiation/run.sh
```

It writes `boundary-results.json` and `harmonic-results.json` here and the
raw exports under `target/ph08`, about four minutes on an M1 Max.
