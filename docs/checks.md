# funfern checks

The routine checks are in the README. This file covers the specialized
convergence, benchmark and GPU-versus-reference runs, and what each one
asserts.

```sh
cargo run -p funfern-core --release --example mesh_timing -- --slices
cargo run -p funfern-core --release --example mesh_timing -- --wave --slices
cargo run -p funfern-core --release --example mesh_edit_timing -- --paced
cargo run -p funfern-core --release --example wave_convergence
cargo run -p funfern-core --release --example wave_boundary_reflection
cargo run -p funfern-core --release --example temporal_amr_calibration
cargo run -p funfern-core --release --example canonical_temporal_timing
cargo run -p funfern-app --release --locked --example canonical_gpu_filter_boundary
cargo run -p funfern-app --release --locked --example canonical_gpu_temporal_work
cargo run -p funfern-app --release --locked --example canonical_gpu_temporal_amr
cargo run -p funfern-app --release --locked --example canonical_gpu_temporal_forced
```

The `funfern-app` examples open a window and read the autosave, so run them with
`HOME` pointed at a scratch directory. Build with the real `HOME` first, or the
toolchain is re-fetched into the scratch one.

Set `PLAYWRIGHT_CHANNEL=chrome` to run the smoke test with an installed Google
Chrome instead of Playwright's pinned Chromium.

GitHub Actions runs formatting, Clippy, the workspace tests, and native and WASM
release builds for pull requests and pushes to `main`. The Playwright smoke test is
a local hardware-backed check because GitHub-hosted runners do not expose a usable
WebGPU adapter. It rejects browser rendering failures, checks that the canvas
continues to change, and verifies that its backing render target follows a viewport
resize. A successful `main` run publishes the browser bundle to
<https://dmitriyne.github.io/funfern/>. The Pages source is configured as
**GitHub Actions** in the repository settings.

The automated tests cover spline evaluation/derivatives, seam insertion, exact predicates,
constrained topology, concave and multiple holes, driven and absorbing internal spans,
nested material inclusions,
shared interface traces, separated wall traces, refinement limits, geometry
rejection, draft/accepted history, scene files, and egui pointer/keyboard
interactions, local Delaunay legality, incremental work, slice-independent output,
P1 and piecewise enriched-quadratic assembly, positive mass lumping, degree-four stiffness
quadrature, stable centered stepping, energy/damping behavior, spatial mode
convergence, GPU upload parameters, and a 32-obstacle meshing regression. The timing example uses the app's mesh
settings and 2 ms scheduling policy. `--wave` tests maximum edges 0.04 and 0.02;
without it, the example tests preview resolution. It reports P1 spatial DOFs,
actual edge size, and resolution relative to a reference wavelength of 0.4.
These are meshing timings. `wave_convergence` separately measures the analytic
reflecting-box mode for P1 at h=0.04 and h=0.02 and enriched quadratic triangles
at parent h=0.08 and h=0.04, with independent temporal refinement over one and
five box-crossing times.
`canonical_temporal_timing` records the actual-core incremental cost of a
time-driven medium, per boundary composition, with the same mesh, operator,
forcing and timestep on both sides. At `h=0.1` the driven bulk runs about `19x`
the fixed bulk and the second-order outgoing wall about `38x`. The bulk ratio is
the floor - the cost of recomputing a stage's coefficients in every helper that
wants them - and the wall's excess over it is the separate, asymptotic cost of
rebuilding the trace factorization every stage instead of once.

`temporal_amr_calibration` measures the AMR estimator's efficiency index, its
estimate over the true error, across a refinement sequence on a smooth
reflecting-box problem. It crosses the driven constitutive row against the
spatial pattern in it - inert, each row pumped uniformly, each row carrying a
travelling pattern, one row at two wavenumbers, and both rows at once - because
a sweep that changes the row and the pattern together cannot say which one the
estimator is charging for. The index should be bounded and roughly constant;
where it climbs with refinement the estimate cannot be read as a percentage.
The first row is the production static estimator on an inert medium, and it is
there because the driven estimate's calibration constant is the ratio of the two
geometric means; two indices only compare if one study produced both. With that
constant applied, every row reads 1.21 to 1.62 against the static row's 1.26 to
1.54, so the same accuracy target means the same true error on either path. This
sweep is where `DRIVEN_INDICATOR_CALIBRATION` comes from: if the driven rows stop
agreeing with the static one, the constant is stale.

`wave_boundary_reflection` sends finite Gaussian P2e packets at the outer box and
compares first- and second-order residual-energy reflection at two angles and two
wavelengths, alongside the ideal continuous plane-wave coefficients and a
long-time finite-state check.
Omit `--slices` for per-phase profiling.
`mesh_edit_timing --paced` applies edits with 2 ms mesh slices at a simulated
60 Hz schedule, excluding rendering.

The `--mesh-edit-benchmark`, `--wave-gpu-check`, `--wave-transfer-check` and
`--amr-check` application flags no longer exist; the unified-topology cutover
removed them and the hidden `canonical_gpu_*` examples took over what they
covered.
`canonical_gpu_temporal_forced` runs a pumped medium with a volume source and
an absorbing wall - three stages that each divide by the nodal mass - against
the f64 reference, and requires both state lanes within `2e-4`. It exists
because the plan compiler used to refuse that combination, so no stage had ever
been exercised with a moving mass; the first run found the kick dividing by the
authored mass and missing by `1.4e-2`.

`canonical_gpu_temporal_amr` runs the error estimate against the f32 state a
device actually produces, on a travelling mass modulation - the medium that used
to take the efficiency index from 1.4 to 17.4 before the estimate moved onto the
solver's own flux. The instantaneous coefficients come from the material runtime
decoded out of the same buffer copy as the state. On the accepted generation it
requires every term within `1e-4` of the f64 oracle, except the cell residual at
`1e-3` and the worst element indicator at `1e-2`. It then refines where the
estimate asks, hands the generation over on the device, steps the new one, and
requires the same bounds again - the estimate is as good after a refinement
transfer as before one. In between it measures the transfer on its own, host
and device applying the same maps to the same input, and requires that the
device is performing the *conserving* primary transfer: the free one misses by
`2e-4`, the conserving one lands at `6e-8`, and the complementary transfer is
exact.
The field view tessellates every quadratic parent triangle into six display
triangles around its shared edge-midpoint and element bubble nodes.

`canonical_gpu_filter_boundary` runs a point probe whose sample stride equals
the resident grid filter's cadence, the worst case the speed control can
produce, and requires the recorded field and rate to match an f64 oracle. The
filter flips the accepted state lane without advancing time, so a recorder
that differenced the lanes at that boundary reported a filter correction
divided by the timestep instead of a rate.

`canonical_gpu_temporal_work` stamps a material Switch mid-run and requires
the runtime decoded from the state snapshot, and the bulk energy split and
pump power computed from it, to match an f64 oracle. The Switch origin is
stamped on the GPU, so this is the check that the accepted runtime reaches a
consumer rather than being guessed from the clock.

Which hardware each check has been run on, and when, is recorded in the
engineering log rather than here.
