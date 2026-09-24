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
cargo run -p funfern-app --release --locked --example canonical_gpu_temporal_timing
cargo run -p funfern-app --release --locked --example canonical_gpu_temporal_timing -- --fixed
cargo run -p funfern-app --release --locked --example canonical_gpu_temporal_timing -- --outgoing
cargo run -p funfern-app --release --locked --example canonical_gpu_driven_document
DRIVEN_WALLS=outgoing cargo run -p funfern-app --release --locked --example canonical_gpu_driven_document
OSCILLATOR_MEDIUM=kink OSCILLATOR_FILTER=1 cargo run -p funfern-app --release --locked --example canonical_gpu_oscillator
OSCILLATOR_HANDOFF=remesh cargo run -p funfern-app --release --locked --example canonical_gpu_oscillator_handoff
FAILURE_LAW=phi4 cargo run -p funfern-app --release --locked --example canonical_gpu_nonlinear_failure
cargo run -p funfern-app --release --locked --example canonical_gpu_temporal_timing -- --oscillator
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
forcing and timestep on both sides. At `h=0.1` the driven bulk runs about `13x`
the fixed bulk, which is the floor: the cost of recomputing a stage's
coefficients in every helper that wants them. The second-order outgoing wall
used to sit at `35x`, well above that floor, because it rebuilt its trace
factorization every stage. It now sits at `8x`, below the floor, because that
system carries no nodal mass: one preparation serves every stage and the mass
arrives at the solve. The fixed column is unchanged - a generation whose mass
cannot move still inverts once and solves in a single pass.

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
`canonical_gpu_driven_document` is the only end-to-end run: an authored
document with a material drive, meshed and assembled by the application's own
resumable jobs, compiled into a temporal plan and stepped on the device against
an f64 oracle. It reads `2.46e-7` over forty-eight steps. `DRIVEN_WALLS=outgoing`
puts the drive behind a second-order outgoing wall - the document's default, and
the combination that used to be refused - and reads `3.31e-7` over the same
forty-eight. `DRIVEN_STEPS` and `DRIVEN_DEPTH` override the step count and the
modulation depth.

`canonical_gpu_temporal_timing` is the throughput comparison that decides
whether a drive is affordable, run twice with and without `--fixed` on an
otherwise identical fixture. On an M1 Max at 15270 DOFs both read `517 us/step`:
per step the drive is free on the production core. What it does cost is the
timestep, because the CFL bound tightens with the coefficient trajectory, so a
driven medium runs about `1.23x` the wall clock per simulated second.
`--outgoing` swaps the first-order wall for a second-order one, the only
composition whose stage does non-local work. Both runs then read about
`1045 us/step` at a 328-node trace - still free per step - against `833 us/step`
before the trace solve became a sweep. That `1.26x` is what buys a driven medium
behind that wall at all, and it is confined to this composition. The sweep is
two dispatches per pass and each stays as wide as the boundary; running it in
one workgroup instead, so a pass could use a barrier rather than a dispatch
boundary, read `6975 us/step`.

`canonical_gpu_temporal_forced` runs a pumped medium with a volume source and
an absorbing wall - three stages that each divide by the nodal mass - against
the f64 reference, and requires both state lanes within `2e-4`. It exists
because the plan compiler used to refuse that combination, so no stage had ever
been exercised with a moving mass; the first run found the kick dividing by the
authored mass and missing by `1.4e-2`. It reads `2.42e-7`, down from `5.98e-6`
once a driven stage stopped pinning half a step in.

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

`canonical_gpu_oscillator` steps an oscillator medium (Gate O) on the device
against the f64 reference and requires `Q`, `b` and the integrated field `r`
within the Stage 0 `3e-5` after 200 steps. `OSCILLATOR_MEDIUM` picks
Klein-Gordon, sine-Gordon, a moving kink, a φ⁴ wall, or sine-Gordon beside a
pump or a Kerr row; `OSCILLATOR_COMPOSE` adds a wall of either order, a gap,
pins, a source, loss or a material junction; `OSCILLATOR_FILTER=1` runs the
resident filter. `van-der-pol` adds the Bernoulli loss stage and compares the
active-gain and primary-loss lanes to `1e-3`; `OSCILLATOR_AMPLITUDE=0.05` puts it
below its threshold, where it grows. Across all 120 combinations the worst
state reads `1.4e-5` and the worst lane `2.8e-6`. A filter that left `r` where
it was reads `1.3e-4` on sine-Gordon.

`canonical_gpu_oscillator_handoff` hands `r` to the next generation on the
device against `transfer_integrated_field`: `remesh` (a moving kink onto a
non-nested finer mesh), `identity`, `from-linear` (starts at `r = 0`),
`to-linear` (drops it), and `reject`, where a φ⁴ target's bound refuses the
transferred field with status 6 and the source keeps stepping. Each accepted
handoff holds `Q`, `b` and `r` to `3e-5` over 60 steps after it.
`FAILURE_LAW=phi4` runs `canonical_gpu_nonlinear_failure` on a kicked φ⁴ wall:
the device refuses the reference's step with status 6 and a retry leaves every
stored bit, `r` included, unchanged.

`canonical_gpu_temporal_work` stamps a material Switch mid-run and requires
the runtime decoded from the state snapshot, and the bulk energy split and
pump power computed from it, to match an f64 oracle. The Switch origin is
stamped on the GPU, so this is the check that the accepted runtime reaches a
consumer rather than being guessed from the clock.

Which hardware each check has been run on, and when, is recorded in the
engineering log rather than here.
