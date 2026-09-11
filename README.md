# funfern

[![CI and Pages](https://github.com/DmitriyNE/funfern/actions/workflows/ci.yml/badge.svg)](https://github.com/DmitriyNE/funfern/actions/workflows/ci.yml)

A browser finite-element wave playground with editable periodic and open cubic
splines, material regions, holes and reflecting baffles, constrained triangle meshes,
persistent invalid drafts, undo/redo, and versioned scene files.

The production wave solver uses seven-node enriched quadratic, mass-lumped
triangles on both an f64 CPU reference and an f32 WebGPU gather kernel. Geometry
edits commit transactionally: the previous mesh continues to run during candidate
construction, then displacement and velocity transfer with the quadratic basis
before both new time levels and mesh become visible. Native automated checks pass;
browser verification passes on Chromium 152 with an Apple Metal WebGPU adapter.
That pass exposed and fixed both a portable WebGPU binding-limit violation and a
shader entry-point collision in Bevy's browser shader translation path.

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
cargo run -p funfern-app --locked
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

Docker builds the release WASM bundle and serves it with nginx:

```sh
docker compose up --build
```

Open <http://localhost:8080/>. Set `FUNFERN_PORT` to use another host port,
for example `FUNFERN_PORT=9000 docker compose up --build`. Stop it with
`docker compose down`.

## Edit

- **UI shell:** the top bar keeps Undo/Redo, scene files, Fit View, four
  hideable inspector panels, Draw, and wave playback controls visible;
  the solver starts running once its initial mesh is ready. Edit contains
  selection, transforms, topology, and boundary tools; View contains visual
  overlays and field intensity; Simulation contains mesh and solver settings;
  Materials contains the material library. The lower-right status control shows
  FPS, solver steps per second, DOFs, mesh size, and solver dt; clicking it opens
  the full frame, mesh, handoff, and solver diagnostics. Narrow windows collapse
  document and inspector actions into compact menus.
- **Select:** clicking a handle selects that one control for local reshaping.
  Clicking a curve selects its knot span; Shift-click toggles spans, and
  Ctrl/Cmd-click selects the complete curve. Shift-drag adds an unselected span
  before moving the resulting transformable selection and temporarily enables
  grid snapping. Double-click inserts a knot without changing the curve and
  selects its control. Drag from empty viewport space to
  box-select every touched span; Shift-drag adds and Alt-drag subtracts. The span
  filter limits box selection and Ctrl/Cmd+A to outer edges, loops, baffles, or all
  geometry. Select filtered, Invert, and Clear provide the same bulk operations in
  the panel. Selection and its filter do not enter document history.
- **Transform:** complete curves and partial selections bounded by C0 knots can
  be dragged, translated, rotated, uniformly scaled, snapped, or aligned as one
  undoable action. **Isolate selection at C0** inserts every missing boundary
  corner exactly. Each selected subcurve then moves rigidly; connected neighboring
  spans follow only through their shared corner point. Snapping moves the selected
  arc-length centroid. Drag the viewport ring to rotate, its square grip to scale,
  and its center marker to reposition the temporary pivot. Shift snaps rotation to
  15°, scale to 0.1 increments, and translation to the configured grid even when
  persistent snapping is off. Seam-wrapped loop selections are supported.
  A selected span exposes the continuity at its end knot. **C1 tangent** and
  **C0 corner** refine the cubic exactly; gold diamonds on the curve are
  repeated knots, separate from circular control handles. **C2 smooth** and
  **C1 tangent** first try exact knot removal, then use a least-squares reshape
  when the edited corner cannot be represented at the requested continuity.
  The status strip reports an upper bound on the curve displacement.
  Whole loops and baffles can also be duplicated with their span assignments;
  selected baffles can be straightened between their endpoints.
- **Baffle topology:** select one baffle span to split at its end knot. Select
  every span of two baffles to merge their nearest tips. Split preserves the curve
  exactly; merge snaps sufficiently close tips to their midpoint. Both retain
  per-span laws, undo history, and start-to-end face orientation; reversing a
  piece exchanges its left/right assignments. Coincident split tips remain a
  valid mesh junction.
- **Loop roles:** a completely selected loop can switch between a hole and material
  interface. Creating an interior region uses the chosen material. Converting to a
  hole removes that region and is allowed only when it contains no child loops or
  baffles. Existing two-sided closed walls remain load-compatible but are omitted
  from the normal role picker.
- **Boundaries:** one Boundary inspector applies conditions to every compatible
  selected span and reports mixed assignments. Outer edges support reflecting,
  prescribed
  time-varying Neumann and Dirichlet data, and first- or second-order outgoing
  conditions. Hole and baffle faces support the same choices, with an adjustable
  impedance ratio for first-order outgoing behavior. The baffle face selector stays
  in the inspector because its left and right traces are geometrically coincident;
  viewport arrows show the start-to-end direction defining those sides.
  A baffle span instead can use one conservative thin-gap law coupling both traces;
  applying a face condition converts it back to independent faces, and increasing
  gap stiffness can reduce the solver time step.
  New scenes start with the second-order auxiliary condition on all four outer
  edges. View > Boundary conditions colors the assigned laws directly on the
  outer box, hole spans, and both baffle traces.
- **Geometry role:** choose Hole, Interface, or Baffle from Draw before
  creating. An interface retains its interior and shares its finite-element trace
  with the exterior. A baffle is an open curve with two independent coincident
  traces; waves reflect from its faces and diffract around its free endpoints.
  Nested loops and baffles inherit the region under the creation point. Closed
  two-sided walls remain load-compatible but are no longer a primary creation tool.
- **Circle:** click to place eight controls on a radius `0.15` circle, then
  automatically return to selection. The spline lies inside its control polygon.
- **Straight:** click to place a four-control straight baffle, then return to
  selection.
- **Custom:** click control points; four points enable the live preview. Enter
  finishes an open baffle; Enter or clicking the first handle closes a loop.
  Backspace removes the last point; Escape cancels construction. These are control
  points, not interpolation points.
- **Remove:** Delete or the panel action removes the selected control and its
  associated knot. This can reshape the curve. At least four controls must remain.
  A removal that would merge different span conditions is rejected until the two
  assignments agree. Shape-preserving insertion copies the split span assignment.
  Deleting an entire loop is a separate panel action.
- **Materials:** create and name materials in the Library, edit positive mass
  density and stiffness plus nonnegative volume damping, and assign them under
  Subdomain assignment. Unused non-default materials can be deleted. Clicking a
  filled region in the viewport selects and highlights it. Material changes
  preserve the live field and reuse the committed mesh.
- **Simulation input:** Place pulse and Move source remain active for repeated
  viewport clicks and can be toggled off with the same selected button, the
  viewport Done action, or Escape. Pulse strength/width and continuous-source
  position, frequency, strength, width, and region are editable in Simulation.
- **Navigate:** right-drag or Space + left-drag pans. Wheel zoom stays centered
  on the cursor. Fit View frames the fixed square. Panel scrolling and text
  editing do not manipulate the viewport.
- **Drafts:** green curves are accepted, amber curves are being checked, red
  curves are invalid. The last accepted scene stays as a subdued reference.
  Invalid edits remain after release. Escape during a drag restores its starting
  document; Undo can restore earlier accepted snapshots.
- **History:** Ctrl/Cmd+Z undoes, Ctrl/Cmd+Shift+Z redoes outside text editing.
  Each drag, coordinate edit, insertion, removal, creation, or deletion is one
  action. History keeps up to 100 actions, including invalid drafts.
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
  hole, material-interface, and open-baffle control-point edits reuse and repair a
  bounded region of the previous mesh. Baffle repair keeps its coincident left/right
  traces and shared free tips intact, and subdivides both faces together. A failed
  local repair retries from the unchanged committed mesh with up to two larger guard
  regions before using a full rebuild. The Performance panel reports attempts, retry
  causes, patch size, repaired baffles/trace segments, unchanged elements, and a
  session fallback-cause count. Creation, deletion, knot/topology changes, closed-wall
  motion, large edits, and exhausted repairs still rebuild.
  Open-curve insertion reuses safe nearby bulk vertices at the exact curve position
  to avoid tiny CFL-limiting elements around baffles.
- **Adaptive mesh foundation:** the core can refine and coarsen an existing mesh
  against a bounded spatial edge-size field while preserving the accepted geometry
  revision. A separate mesh revision identifies each discretization. Outer edges,
  holes, material interfaces, closed walls, and both coincident baffle faces remain
  constrained; spline breakpoints, box corners, and open tips remain fixed. The app
  runs adaptation in 2 ms slices and transfers the live quadratic field at an atomic
  GPU handoff. Automatic adaptation is enabled by default. A recovery-plus-residual
  indicator reads synchronized displacement, velocity, and acceleration from spare
  lanes in the existing GPU state readback, then builds a graded spatial size field
  in bounded frame slices. Fast, Balanced, and Detailed presets set the error and
  wavelength targets; advanced controls bound the smallest and largest element.
  The View panel can overlay the current target field.
- **Waves:** Run/Pause, Step, and Reset operate the GPU solver. Place pulse adds a
  Gaussian displacement with zero initial velocity. Move source positions the
  optional continuous sinusoidal source. Simulation speed is bounded to 16
  substeps per display frame. Field colors use an adjustable symmetric gain.
  Pulses and continuous sources act only in their containing wall-separated
  region. With open baffles their Gaussian stencil uses mesh-path distance, so it
  goes around a free endpoint instead of jumping through coincident faces.
  The panel reports DOFs, GPU buffer size, operator-derived timestep, simulated
  time, substeps, throughput, operator/map preparation time, and discrete energy.
- **Outer boundary:** select each box side independently and assign zero Neumann
  (reflecting), prescribed Neumann flux, prescribed Dirichlet displacement,
  first-order outgoing, or second-order auxiliary behavior. Prescribed data uses
  `offset + amplitude · sin(2π f t + phase)`; zero amplitude gives a constant.
  Adjacent Dirichlet sides must agree at their shared corner. The second-order
  Engquist-Majda condition adds tangential propagation and reduces oblique
  reflection. Hole and baffle spans support reflecting or local first-order
  impedance conditions; closed walls remain reflecting and material interfaces
  transmit. Changes reuse the mesh and transactionally transfer the live field.
- **Interior media:** every triangle carries a stable region ID. The P2e operator
  assembles piecewise mass, stiffness, and damping. Material interfaces use a
  conforming shared trace. Closed walls and open baffles have separate solution
  DOFs on their two faces; baffle tips reconnect to the surrounding domain.
- **Mesh changes:** simulation continues on the displayed committed mesh while a
  replacement mesh, operator, timestep, and transfer map are prepared. The GPU
  reconstructs velocity from both old displacement levels, interpolates field and
  velocity, initializes the new staggered level for its new timestep, and commits
  after a finite tagged readback. Newly exposed domain starts at zero. A failed
  candidate retains the previous simulation. Baffle trace nodes prefer the same
  stable boundary ID and left/right face on the old mesh during transfer.

Scenes allow 32 geometric features, 32 materials, and 128 controls per curve.
Version 7 JSON stores the complete outer, hole-span, and open-baffle laws together
with draft and accepted material/region topology. Versions 2–6 remain compatible,
and version 1 files migrate their loops to background holes. Legacy baffles that
combined a thin-gap spring with face laws load with the thin-gap law taking
precedence. JSON files are capped at 2 MiB and require finite coordinates. The editor uses a fixed
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
cargo build -p funfern-app --locked
trunk build --release
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

GitHub Actions runs formatting, Clippy, the workspace tests, and native and WASM
release builds for pull requests and pushes to `main`. A successful `main` run
publishes the browser bundle to <https://dmitriyne.github.io/funfern/>. The Pages
source is configured as **GitHub Actions** in the repository settings.

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
triangles around its shared edge-midpoint and element bubble nodes. Native GPU
startup and the wave kernel were exercised on Apple M1 Max / Metal. The user
reports completing the Milestone 1 browser interaction checks; browser metadata
and interactive checks for the quadratic solver have not been recorded.

## Layout

```text
crates/funfern-core/       Dependency-free f64 geometry, meshing, and CPU wave reference
crates/funfern-app/        Bevy/egui editor, persistence, GPU waves, and field display
examples/                Scene files for exercising the editor
docs/plan.md             Milestones and completion criteria
docs/architecture.md     Representation, draft model, and later solver design
docs/engineering-log.md  Verification results and remaining work
```

Bevy owns the wgpu device. The current viewport uses egui's painter through
`bevy_egui` on that device; there is no second renderer/device. Audio, 3D render
pipelines, and the WebGL fallback are disabled. All numerical geometry remains
independent of Bevy, egui, serde, and external numerical libraries.
