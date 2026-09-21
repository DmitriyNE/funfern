# Single-boundary scattering follow-up

Purpose: separate absorber reflection from finite-box/corner/packet effects.
No production changes, GPU benchmark, or curved-boundary extension.

Run from the repository root:

```sh
bash experiments/material-laws-spike/run-scattering.sh
bash experiments/material-laws-spike/run-scattering.sh --original-split
```

The first command runs the corrected candidate (248 passing checks). The second
reproduces the rejected split (21 failed checks; expected exit status 1).
Both emit complete JSON on stdout and check progress on stderr. Capture stdout
to a file if desired. See `docs/spikes/funfern-boundary-scattering-spike.md` for the
derivation and interpretation.

Use an enriched-triangle strip with Bloch-periodic transverse boundaries. A
single transverse cell represents a plane wave at prescribed tangential
wavenumber. The right edge is the absorber; the left trace drives a harmonic
field. Decompose the interior solution into forward/backward **discrete FEM
propagating modes**, extrapolated to the absorbing edge. The backward/forward
ratio measures that edge's reflection, independent of the drive amplitude.
Transverse boundaries have no wall reflection; no corners are present.

The local triangles and quadrature are exported from the same pinned core as
the original spike. The strip is an exact Bloch reduction of that uniform 2D
FEM, not a replacement 1D spatial method. Test both semidiscrete harmonic
equations and harmonic equations of the actual explicit KDK + boundary-midpoint
split update. This frequency-response calculation avoids long grazing-packet
flight times/startup/windowing; it is **not a new transient packet test**.

Three boundary laws: first order, legacy second-order admittance, and the new
passive three-state candidate. All use the same spatial data and, for the
time-discrete comparison, the same splitting/midpoint boundary integration.
The legacy comparison is its law, not its old production timestep algorithm.

## Criteria recorded before running measurements

- Reconstructed local stiffness agrees with the exported assembled FEM operator
  to relative 1e-11; boundary line assembly agrees to 1e-11.
- Bloch-reduced matrices are Hermitian to 1e-11; fitted forward/backward mode
  residual <=1e-5 on the finest meshes, away from boundary-local evanescent modes.
- Angles from the normal: 0,30,60,75,85 degrees. Wavelengths .75,1,1.5 in a
  fixed nominal length-2 strip. Meshes at approximately 8,16,32 cells/wavelength.
- Finest-mesh complex reflection error versus each continuum prediction <=0.01
  absolute amplitude. Mesh refinement reduces error where it exceeds 1e-6;
  require at least a factor 2 from coarsest to finest (no fitted order assumed).
- Time refinement at fixed 16 cells/wavelength, wavelength 1, angles 30 and 75:
  dt/h=.12,.06,.03,.015. Error versus the semidiscrete result must decrease by
  >=3.5 on halving dt once above a 1e-8 numerical floor.
- Verify the time-discrete harmonic solution against its KDK/boundary stage
  equations with residual <=1e-9, rather than relying only on derived impedance.
- First-order normal reflection <=0.01 on the finest mesh. For 60,75,85 degrees,
  the passive candidate must measurably improve on first order; report legacy
  second order separately, with no promise the passive candidate beats it.
- Repeat selected measurements at a different strip length / fitting window;
  reflection change <=1e-4 absolute, to screen drive/evanescent contamination.

Use complex reflection error (including phase), not only magnitudes. Retain
failed cases and emit nonzero exit status. If a criterion cannot be exercised,
report that gap explicitly rather than silently declaring it passed.

## Outcome-driven timestep correction

The first run found 21 mesh-refinement failures with the original separate
boundary split: its reflection error approaches a fixed-CFL floor. Those
results are retained as `scattering-original-split-results.json` and can be
reproduced with the `--original-split` argument (expected exit status 1).
The corrected candidate couples the held interior force into each boundary-aware
midpoint kick, keeping the drift explicit. The same scattering thresholds apply,
without relaxation. Before testing that correction, add the following stability
regression: full coarse 2D state spectral excess <=1e-10 at CFL .2,.7,.95 for
lossless, both loss, the old complementary-loss counterexample, and spatial loss;
peak total energy to t=100 <=1.01 of its initial value for the existing fixture.
