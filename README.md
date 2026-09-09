# femfun

A browser finite-element wave playground with editable periodic and open cubic
splines, material regions, holes and reflecting baffles, constrained triangle meshes,
persistent invalid drafts, undo/redo, and versioned scene files.

The production wave solver uses seven-node enriched quadratic, mass-lumped
triangles on both an f64 CPU reference and an f32 WebGPU gather kernel. Geometry
edits commit transactionally: the previous mesh continues to run during candidate
construction, then displacement and velocity transfer with the quadratic basis
before both new time levels and mesh become visible. Native automated checks pass;
interactive browser checks for this solver revision remain deferred.

## Run

Tested toolchain: Rust 1.96.0, Trunk 0.21.14. Install the browser tools once:

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
```

From the repository root:

```sh
trunk serve
```

Open <http://127.0.0.1:8080/> in a WebGPU-capable browser with hardware
acceleration enabled. WebGPU requires HTTPS or localhost. The page displays a
startup diagnostic if initialization fails. Trunk watches sources and reloads
on rebuild; its first build downloads matching WASM helpers.

Native development:

```sh
cargo run -p femfun-app --locked
```

Release browser bundle (output: `dist/`):

```sh
trunk build --release
# Serve the optimized build locally:
trunk serve --release
```

If your shell sets `NO_COLOR=1`, Trunk 0.21.14 rejects that value. Use
`NO_COLOR=true trunk serve` or `NO_COLOR=true trunk build --release` instead.
The release build and serving commands were exercised with this environment
setting. Keep `Cargo.lock` for reproducibility. The pinned integration is
[Bevy 0.19.1](https://docs.rs/bevy/0.19.1/bevy/) with
[bevy_egui 0.42.0](https://docs.rs/crate/bevy_egui/0.42.0).
See the [maintained Trunk project](https://github.com/trunk-rs/trunk) for tooling.

## Edit

- **Select:** left-click a handle or curve; drag handles to reshape. Double-click
  a curve to insert a knot without changing its shape. Clicking near an existing
  knot selects its associated control instead of adding a repeated knot.
- **Geometry role:** choose Hole, Material interface, or Reflecting baffle before
  creating. An interface retains its interior and shares its finite-element trace
  with the exterior. A baffle is an open curve with two independent coincident
  traces; waves reflect from its faces and diffract around its free endpoints.
  Nested loops and baffles inherit the region under the creation point. Closed
  two-sided walls remain load-compatible but are no longer a primary creation tool.
- **Rounded:** click to place eight controls on a radius `0.15` circle, then
  automatically return to selection. The spline lies inside its control polygon.
- **Custom:** click control points; four points enable the live preview. Enter
  finishes an open baffle; Enter or clicking the first handle closes a loop.
  Backspace removes the last point; Escape cancels construction. These are control
  points, not interpolation points.
- **Remove:** Delete or the panel action removes the selected control and its
  associated knot. This can reshape the curve. At least four controls must remain.
  Deleting an entire loop is a separate panel action.
- **Materials:** create materials under Regions and materials, edit positive mass
  density and stiffness plus nonnegative volume damping, and assign a material to
  the background or a retained loop interior. Clicking a filled region in the
  viewport selects and highlights it. Material changes preserve the live field
  and reuse the committed mesh.
- **Navigate:** middle-drag or Space + left-drag pans. Wheel zoom stays centered
  on the cursor. Fit View frames the fixed square. Panel scrolling and text
  editing do not manipulate the viewport.
- **Drafts:** green curves are accepted, amber curves are being checked, red
  curves are invalid. The last accepted scene stays as a subdued reference.
  Invalid edits remain after release. Escape during a drag restores its starting
  document; Revert Draft restores the accepted scene.
- **History:** Ctrl/Cmd+Z undoes, Ctrl/Cmd+Shift+Z redoes outside text editing.
  Each drag, coordinate edit, insertion, removal, creation, deletion, or revert
  is one action. History keeps up to 100 actions, including invalid drafts.
- **Files:** Save scene downloads JSON in the browser or opens a native save
  dialog. Load scene uses file upload/native selection and validates before
  replacement. Successful loading clears history; malformed files leave the
  current document intact. Camera and selection are not saved.
- **Mesh:** enable Accepted triangle mesh under Display. The overlay shows the
  constrained mesh and its labeled outer, hole, interface, closed-wall, and baffle
  edges. Elements below 15° are
  amber; the panel reports counts, minimum angle, maximum edge, refinement
  progress, build/work time, longest mesh slice, and explicit construction
  failures. Meshing targets a soft 2 ms per frame. An invalid draft keeps the last
  accepted mesh.
- **Resolution:** parent maximum edge 0.08 is the default for P2e; choose 0.04 for
  a finer solve or 0.16 for a quick preview. Changing resolution rebuilds the
  accepted mesh without changing geometry/history. Resolution is not stored in scene files.
  Full fine builds can take tens of seconds when spread across frames. Small
  hole control-point edits now reuse and repair a bounded region of the previous mesh.
  The panel reports unchanged elements and full-rebuild fallbacks; creation,
  deletion, knot changes, interface/baffle motion, and large/failed repairs still rebuild.
- **Waves:** Run/Pause, Step, and Reset operate the GPU solver. Place pulse adds a
  Gaussian displacement with zero initial velocity. Move source positions the
  optional continuous sinusoidal source. Simulation speed is bounded to 16
  substeps per display frame. Field colors use an adjustable symmetric gain.
  Pulses and continuous sources act only in their containing wall-separated
  region. With open baffles their Gaussian stencil uses mesh-path distance, so it
  goes around a free endpoint instead of jumping through coincident faces.
  The panel reports DOFs, GPU buffer size, operator-derived timestep, simulated
  time, substeps, throughput, operator/map preparation time, and discrete energy.
- **Outer boundary:** choose Reflecting, First-order outgoing, or Second-order
  auxiliary. The second-order Engquist-Majda condition adds tangential propagation
  along the fixed outer square and reduces oblique reflection. Hole, closed-wall,
  and baffle boundaries remain reflecting; material interfaces transmit. A change reuses the mesh and transactionally transfers the
  live field to the replacement operator. Boundary memory survives second-order
  geometry edits and is cleared when entering or leaving that mode.
- **Interior media:** every triangle carries a stable region ID. The P2e operator
  assembles piecewise mass, stiffness, and damping. Material interfaces use a
  conforming shared trace. Closed walls and open baffles have separate solution
  DOFs on their two faces; baffle tips reconnect to the surrounding domain.
- **Mesh changes:** simulation continues on the displayed committed mesh while a
  replacement mesh, operator, timestep, and transfer map are prepared. The GPU
  reconstructs velocity from both old displacement levels, interpolates field and
  velocity, initializes the new staggered level for its new timestep, and commits
  after a finite tagged readback. Newly exposed domain starts at zero. A failed
  candidate retains the previous simulation.

Scenes allow 32 geometric features, 32 materials, and 128 controls per curve.
Version 3 JSON stores open baffles plus draft and accepted material/region topology;
version 1 files migrate their
loops to background holes. JSON files are capped at 2 MiB and require finite
coordinates. The editor uses a fixed
world-space validation tolerance of `0.0002`, independent of zoom. The editor
validator is deliberately conservative; final mesh topology uses adaptive exact
orientation and incircle signs rather than geometric epsilons.
Load [examples/eight-obstacles.json](examples/eight-obstacles.json) for a
representative scene.

## Check

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build -p femfun-app --locked
trunk build --release
cargo run -p femfun-core --release --example mesh_timing -- --slices
cargo run -p femfun-core --release --example mesh_timing -- --wave --slices
cargo run -p femfun-core --release --example mesh_edit_timing -- --paced
cargo run -p femfun-core --release --example wave_convergence
cargo run -p femfun-core --release --example wave_boundary_reflection
cargo run -p femfun-app --release --locked -- --mesh-edit-benchmark
cargo run -p femfun-app --release --locked -- --wave-gpu-check
cargo run -p femfun-app --release --locked -- --wave-transfer-check
```

The automated tests cover spline evaluation/derivatives, seam insertion, exact predicates,
constrained topology, concave and multiple holes, nested material inclusions,
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
`--wave-gpu-check` runs the real second-order compute pipeline for 128 steps, reads
both time levels and boundary memory back, compares them to f64, reports
solve-to-readback throughput, and exits. `--wave-transfer-check` injects a nonzero
field, performs a real control-point edit, verifies transfer of all three state
components, then verifies that a lower-order boundary transaction clears the
auxiliary state, and finally checks a same-mesh material-coefficient transaction
against f64.
The field view tessellates every quadratic parent triangle into six display
triangles around its shared edge-midpoint and element bubble nodes. Native GPU
startup and the wave kernel were exercised on Apple M1 Max / Metal. The user
reports completing the Milestone 1 browser interaction checks; browser metadata
and interactive checks for the quadratic solver have not been recorded.

## Layout

```text
crates/femfun-core/       Dependency-free f64 geometry, meshing, and CPU wave reference
crates/femfun-app/        Bevy/egui editor, persistence, GPU waves, and field display
examples/                Scene files for exercising the editor
docs/plan.md             Milestones and completion criteria
docs/architecture.md     Representation, draft model, and later solver design
docs/engineering-log.md  Verification results and remaining work
```

Bevy owns the wgpu device. The current viewport uses egui's painter through
`bevy_egui` on that device; there is no second renderer/device. Audio, 3D render
pipelines, and the WebGL fallback are disabled. All numerical geometry remains
independent of Bevy, egui, serde, and external numerical libraries.
