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
cargo run -p funfern-app --release --locked -- --mesh-edit-benchmark
cargo run -p funfern-app --release --locked -- --wave-gpu-check
cargo run -p funfern-app --release --locked -- --wave-transfer-check
cargo run -p funfern-app --release --locked -- --amr-check
```

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
`wave_boundary_reflection` sends finite Gaussian P2e packets at the outer box and
compares first- and second-order residual-energy reflection at two angles and two
wavelengths, alongside the ideal continuous plane-wave coefficients and a
long-time finite-state check.
Omit `--slices` for per-phase profiling.
`mesh_edit_timing --paced` applies edits with 2 ms mesh slices at a simulated
60 Hz schedule, excluding rendering. `--mesh-edit-benchmark` opens the real native
app with eight obstacles and the production parent-h=0.08 overlay, applies three small control edits,
prints edit-to-ready and active/scheduling times plus exact element reuse, then
exits. Interactive editor input is disabled during this scripted run. It does not
read or overwrite scene files. The native benchmark includes
editor validation and rendering load; mesh-ready means the atomic simulation commit.
`--wave-gpu-check` runs mixed harmonic Dirichlet and Neumann data on the outer box
and both faces of a baffle, both outgoing orders, and a closed wall for 128 steps.
It reads both time levels and boundary memory back, compares them to f64, reports
solve-to-readback throughput, and exits. `--wave-transfer-check` injects a nonzero
field, performs a real control-point edit, verifies transfer of all three state
components, then verifies that a lower-order boundary transaction clears the
auxiliary state, and finally checks a same-mesh material-coefficient transaction
against f64.
`--amr-check` first requires the normal automatic controller to finish an aligned
GPU-to-host solution estimate, then evolves a nonzero field, moves a deterministic
spatial refinement target, requires both refinement and coarsening, transfers all
quadratic state with no exposed nodes, verifies mesh/operator revision agreement,
and exits.
The field view tessellates every quadratic parent triangle into six display
triangles around its shared edge-midpoint and element bubble nodes.

Which hardware each check has been run on, and when, is recorded in the
engineering log rather than here.
