# femfun

A browser geometry playground for a future 2D finite-element wave toy. Edit
periodic cubic spline obstacles inside a fixed square, with persistent invalid
drafts, geometric validation, undo/redo, and versioned scene files.

Milestone 1 is implemented. Native startup and automated checks pass. Browser
interaction and GPU/frame-time measurements still need a connected browser;
see [verification notes](docs/engineering-log.md). Meshing and wave simulation
are later milestones; simulation buttons are intentionally disabled.

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

Scenes allow 32 obstacles and 128 controls per obstacle. JSON files are capped at
2 MiB and require finite coordinates. The editor uses a fixed
world-space validation tolerance of `0.0002`, independent of zoom. This is an
approximate editor validator; robust meshing predicates arrive in milestone 2.
Load [examples/eight-obstacles.json](examples/eight-obstacles.json) for a
representative scene.

## Check

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build -p femfun-app --locked
trunk build --release
```

The tests cover spline evaluation/derivatives, seam insertion, geometry rejection,
validation budgets, draft/accepted history, scene files, and egui pointer/keyboard
interactions. Native GPU startup was exercised on Apple M1 Max / Metal. Browser
upload/download, visual rendering, unsupported-WebGPU behavior, and performance
must additionally be checked in a real browser; see the
[browser checklist](docs/browser-checks.md).

## Layout

```text
crates/femfun-core/       Dependency-free f64 splines, sampling, geometry validation
crates/femfun-app/        Bevy/egui UI, document/history model, JSON and file dialogs
examples/                Scene files for exercising the editor
docs/plan.md             Milestones and completion criteria
docs/architecture.md     Representation, draft model, and later solver design
docs/engineering-log.md  Verification results and remaining work
```

Bevy owns the wgpu device. The current viewport uses egui's painter through
`bevy_egui` on that device; there is no second renderer/device. Audio, 3D render
pipelines, and the WebGL fallback are disabled. All numerical geometry remains
independent of Bevy, egui, serde, and external numerical libraries.
