# The outgoing wall under the production step

Purpose: find why the second-order outgoing wall reflects more than its law
at the app's step, and a wall update that does not. No production changes.

Run from the repository root:

```sh
bash experiments/material-laws-spike/run-wall-step.sh > wall-step-results.json
```

It compiles the same clean `beca47e` core as the scattering spike, exports the
enriched-triangle cell and the coarse 2D mesh, and runs `wall_step.py`: checks
and observations on stderr, complete JSON on stdout, exit status 1 if any
check fails. It takes about four minutes once the core has compiled. The
recorded run is `wall-step-results.json`. See
`docs/spikes/funfern-outgoing-wall-step-spike.md` for the derivation and the
reading.

`wall_step.py` steps the scattering spike's Bloch strip by an explicit
one-step map assembled from its stages - kick, drift, wall update - so a
candidate update is a few lines rather than a derived harmonic equation. A
harmonic solution of the map is fitted to the strip's discrete propagating
modes, as `scattering.py` fits its closed form. The 2D coarse mesh of
`boundary.py`, behind second-order walls, checks stability, energy and a
pulse in the time domain.

What it checks:

- The step map reproduces `scattering.py`'s closed-form leapfrog response to
  1e-11.
- On a strip as stiff as the fixture's mesh (cell 0.025, its bound 5.46e-3 s),
  at the fixture's steps, the production step reproduces the fixture's
  second-order excess and the leapfrog its deficit.
- Each scheme's temporal order against the strip's semidiscrete reflection,
  first- and second-order walls, 1.6 and 2.2 Hz of the fixture's band.
- The adopted update - plain kicks at the trace, one Cayley over the wall for
  the whole step, the wall's acceleration in the drift's corrected field - at
  normal incidence: z²/24 at small steps, as derived; the leapfrog with the
  full-step Cayley at z²/16.
- The adopted update stepped as square-root halves, as one wall solve after
  the kicks, and as the trapezoid split - the explicit half with the first
  kick, the implicit half after the second - respond identically.
- Under refinement at 1.2 of each mesh's bound the adopted update's error
  falls as h², production's as h.
- Over wavelengths 0.45, 0.625, 1 and angles 0 to 85 degrees, at the
  fixture's step and at 1.2 of the strip's bound, the adopted update's error
  is at most 0.4 of production's; the fixture's second-order band is within
  1% of its law.
- The kick form's linear limit: under production's half-step kicks it
  reflects as the drift form; under the trapezoid split it needs the wall's
  rate in the trace's `u̇`, without which it is worse than production, and
  with which it is at most 0.6 of production's error and `z²/24` at small
  steps.
- 2D: spectral excess at most 1e-10 from 0.2 to 1.22 of the bound with the
  four loss cases of the scattering spike, energy peak at most 1.01 to
  t = 100, and a pulse from the interior within 10% of its own small step,
  for production, the wall-after split and the trapezoid split; data that
  starts with a field at the wall, which the trapezoid split keeps less of
  in the wall's static memory than the wall-after split.

Thresholds were set from the first measurements, not before them: this is a
record of a diagnosis and its fix, kept as a regression check, not an
acceptance test written in advance.
