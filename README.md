# femfun

A browser finite-element wave playground with editable periodic cubic spline
obstacles, constrained triangle meshes, persistent invalid drafts, undo/redo,
and versioned scene files.

The static-domain P1 wave solver is implemented on both an f64 CPU reference and
an f32 WebGPU gather kernel. Native startup and automated checks pass; the user
has exercised the earlier browser editor state. Interactive browser checks for
the solver remain deferred.

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
- **Rounded:** click to place eight controls on a radius `0.15` circle, then
  automatically return to selection. The spline lies inside its control polygon.
- **Custom:** click control points; four points enable the live preview. Enter
  or clicking the first handle closes the loop. Backspace removes the last point;
  Escape cancels construction. These are control points, not interpolation points.
- **Remove:** Delete or the panel action removes the selected control and its
  associated knot. This can reshape the curve. At least four controls must remain.
  Deleting an entire obstacle is a separate panel action.
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
  constrained mesh and its labeled outer/obstacle edges. Elements below 15° are
  amber; the panel reports counts, minimum angle, maximum edge, refinement
  progress, build/work time, longest mesh slice, and explicit construction
  failures. Meshing targets a soft 2 ms per frame. An invalid draft keeps the last
  accepted mesh.
- **Resolution:** maximum edge 0.04 is the default; choose 0.02 for a finer mesh
  or 0.16 for a quick preview. The finer settings produce roughly 12k and 46k
  triangles in the benchmark scenes. Changing resolution rebuilds the accepted
  mesh without changing geometry/history. Resolution is not stored in scene files.
  Full fine builds can take tens of seconds when spread across frames. Small
  control-point edits now reuse and repair a bounded region of the previous mesh.
  The panel reports unchanged elements and full-rebuild fallbacks; creation,
  deletion, knot changes and large/failed repairs still rebuild.
- **Waves:** Run/Pause, Step, and Reset operate the GPU solver. Place pulse adds a
  Gaussian displacement with zero initial velocity. Move source positions the
  optional continuous sinusoidal source. Simulation speed is bounded to 16
  substeps per display frame. Field colors use an adjustable symmetric gain.
  The panel reports DOFs, GPU buffer size, operator-derived timestep, simulated
  time, substeps, throughput, assembly time, and discrete energy.
- **Mesh changes:** simulation continues on the displayed committed mesh while a
  replacement is prepared. Publishing a new mesh resets the field in this static
  milestone. Transfer of both time levels belongs to the transactional editing
  milestone.

Scenes allow 32 obstacles and 128 controls per obstacle. JSON files are capped at
2 MiB and require finite coordinates. The editor uses a fixed
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
cargo run -p femfun-app --release --locked -- --mesh-edit-benchmark
cargo run -p femfun-app --release --locked -- --wave-gpu-check
```

The 57 tests cover spline evaluation/derivatives, seam insertion, exact predicates,
constrained topology, concave and multiple holes, refinement limits, geometry
rejection, draft/accepted history, scene files, and egui pointer/keyboard
interactions, local Delaunay legality, incremental work, slice-independent output,
P1 assembly, stable centered stepping, energy/damping behavior, spatial mode
convergence, GPU upload parameters, and a 32-obstacle meshing regression. The timing example uses the app's mesh
settings and 2 ms scheduling policy. `--wave` tests maximum edges 0.04 and 0.02;
without it, the example tests preview resolution. It reports P1 spatial DOFs,
actual edge size, and resolution relative to a reference wavelength of 0.4.
These are meshing timings. `wave_convergence` separately measures the analytic
reflecting-box mode at h=0.04 and h=0.02, with independent temporal refinement,
over one and five box-crossing times.
Omit `--slices` for per-phase profiling.
`mesh_edit_timing --paced` applies edits with 2 ms mesh slices at a simulated
60 Hz schedule, excluding rendering. `--mesh-edit-benchmark` opens the real native
app with eight obstacles and the fine overlay, applies three small control edits,
prints edit-to-ready and active/scheduling times plus exact element reuse, then
exits. Interactive editor input is disabled during this scripted run. It does not
read or overwrite scene files. The native benchmark includes
editor validation and rendering load; mesh-ready means publication to app state.
`--wave-gpu-check` runs the real compute pipeline for 128 steps, reads both time
levels back, compares them to f64, reports solve-to-readback throughput, and exits.
Native GPU startup and the wave kernel were exercised on Apple M1 Max / Metal. The user
reports completing the Milestone 1 browser interaction checks; browser metadata
and Milestone 2 overlay performance have not been recorded.

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
