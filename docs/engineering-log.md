# Engineering log

Low-effort working notes: what changed, what was checked, TODOs, open issues, and
next steps. Short bullets are enough; no entry is required for every tiny edit.
Keep current actions near the top and dated entries newest first. Durable decisions
belong in [architecture.md](architecture.md) and milestone scope in [plan.md](plan.md).

## Current TODOs

- [ ] Milestone 3: static-domain CPU reference and GPU wave evolution.
- [ ] Record browser/GPU metadata and frame-time ranges when the mesh overlay is
  next exercised interactively; the user has deferred this pass.
- [ ] Consider time-slicing topology preparation if browser measurements show a
  visible pause on complex accepted scenes. Refinement is already cooperative.

## 2026-09-09 — Milestone 2 constrained triangular meshing

- Added adaptive exact-sign `orient2d` and `incircle`: common inputs use certified
  error bounds; ambiguous inputs use non-overlapping expansion arithmetic. Exact
  segment relations and polygon location build on the same orientation predicate.
- Added deterministic constrained triangulation of the fixed square with multiple
  spline holes, including concave obstacles. Visibility bridges and exact-sign ear
  clipping establish topology; unconstrained edge flips produce a locally Delaunay
  mesh without changing labeled boundary segments.
- Added circumcenter refinement, centroid fallback, encroached-boundary splitting,
  and explicit curve, vertex, triangle, and refinement-step limits. Split boundary
  edges preserve stable outer/obstacle labels and continuous parameter ranges.
- `MeshingJob` snapshots the accepted revision and advances at most one refinement
  insertion per work unit. The app advances two units per frame after an edit ends,
  discards obsolete jobs, and retains the previous mesh for invalid drafts.
- Added the accepted triangle overlay, labeled boundary emphasis, amber elements
  below 15°, counts, quality extrema, progress, and structured failure messages.
- Verification: all **40 tests** pass (3 predicate, 10 spline/validation, 10 mesh,
  9 document/persistence, 8 egui interaction). Mesh tests cover exact degeneracies,
  integer predicate agreement, empty and rounded domains, concave and multiple
  holes, Euler/manifold/area invariants, classification, deterministic output,
  cooperative work, limits, and an eight-obstacle scene.
- Known limit: topology preparation (sampling, bridge search, initial ear clipping)
  is one capacity-bounded work phase. Quality refinement is cooperative, but a
  complex accepted scene can still cause one longer frame during preparation.
- Interactive browser testing was skipped at the user's request. The user reports
  the previous Milestone 1 browser state looked good; exact browser/GPU and timing
  observations were not provided.
- Final checks pass: formatting, Clippy with warnings denied, the full workspace
  test suite, native compilation, and `NO_COLOR=true trunk build --release`.
  The current native app also ran on Apple M1 Max / Metal through initial mesh
  completion without a runtime error; the existing Metal bindless warning remains.

## 2026-09-09 — Milestone 1 implementation and verification

- Added the Bevy 0.19.1 / bevy_egui 0.42.0 application, explicit rendering/window/
  input/WebGPU features, full-window canvas, startup diagnostics, and Trunk setup.
  Bevy owns the only wgpu device. Active dependencies exclude audio, PBR/3D
  rendering, and WebGL fallback; `femfun-core` still has no dependencies.
- Implemented nonuniform periodic cubic de Boor evaluation and two derivatives,
  shape-preserving seam insertion, reshaping removal, and adaptive hull sampling.
- Implemented preset/custom creation, hit testing, coordinate editing, curve and
  control overlays, panning/zoom/Fit View, disabled simulation controls, and
  persistent invalid drafts with accepted references and structured diagnostics.
- Added incremental validation with per-frame and total work ceilings. Bounding
  boxes skip disjoint loops. A 32-loop scene completes within 128 slices of
  12,000 operations; this is a deterministic work check, not a frame-time claim.
- Added 100-entry snapshot history, Escape rollback, Revert Draft, monotonic IDs,
  versioned JSON, browser/native file-dialog paths, and atomic validated loads.
  Serde's float-roundtrip feature is required to keep exact f64 values on reload.
- Regression fixes: near-seam insertion must not create false self-contact from
  tiny adjacent sample spans; existing curve knots snap in screen space; plain
  handle selection must not create a history entry; custom previews use the
  actual committed control points; finite extreme drafts remain loadable.
- Verification: formatting, Clippy with `-D warnings`, and all **26 tests** pass
  (10 core, 9 document/persistence, 7 real egui input tests). Coverage includes
  repeated seam insertion/derivatives, affine/uniform agreement, geometry guards,
  validation limits and revisions, both creation workflows, handle dragging,
  Escape, one-entry history, numeric edits, typing capture, panel scrolling,
  insertion/removal, cursor zoom, both pan gestures, and viewport resize.
- `cargo build -p femfun-app --locked` passes. `cargo run -p femfun-app --locked`
  starts a native window and initializes **Apple M1 Max / Metal**. bevy_egui
  reports its known Metal bindless-texture fallback; no startup failure observed.
- `NO_COLOR=true trunk build --release` passes with Rust 1.96.0 and Trunk 0.21.14.
  Trunk fetched wasm-bindgen 0.2.128 and wasm-opt 123. Browser bundle is about
  **24 MiB WASM + 112 KiB JS**, uncompressed. `trunk serve --release` builds,
  watches/rebuilds, and serves at localhost:8080; an HTTP probe returns 200.
  The sandbox required permission for helper-cache writes and local serving.
- The user subsequently reported exercising this Milestone 1 browser state and
  finding it good. Exact browser/GPU metadata and timing were not supplied. Native
  file-dialog interaction remains unrecorded; JSON behavior is automated.
- Updated README, architecture, and milestone notes. Meshing, wave evolution,
  and solver transaction machinery remain deferred.

## Open issues and experiments

- Performance budgets need measurements on an actual browser/GPU; no fixed mesh
  capacity or realtime throughput is promised yet.
- Exact signs now cover meshing topology. The editor's approximate near-contact
  policy remains intentionally conservative until stronger feature-scale rules are
  developed alongside mesh adaptation.
- Newly exposed region initialization and time-staggered state transfer need concrete
  policies before live geometry commits.
- Select the higher-order boundary radiation formulation and corner treatment during
  the absorber milestone.
- IGA mass treatment and explicit timestep behavior are research tasks for the
  single-patch implementation.

## 2026-09-09 — Project setup and agreed direction

- Added milestone 0 for repository setup, documentation, and project initialization.
- Initialized a Rust workspace with an empty, dependency-free `femfun-core` library.
  The application and its dependencies start in milestone 1.
- Wrote the README, milestone plan, and architecture notes.
- Adopted egui for panels and this file for lightweight development notes.
- Recorded the main choices: custom math, triangles before IGA, boundary-only
  radiation, bounded adaptation, and transactional geometry/parameter edits.
- Preparation may span frames while the old system evolves. Build the transfer map
  against that system, then transfer its latest state at commit time.
- Energy drift during edits is acceptable. Physical moving-wall effects are deferred.
- Verification: `cargo fmt --all -- --check`, `cargo check --workspace --locked`,
  and `cargo clippy --workspace --all-targets --locked -- -D warnings` pass with
  Rust 1.96.0. Generated the lockfile offline. No numerical tests exist yet because
  the core contains no implementation; browser execution starts in milestone 1.
- Next: begin the browser shell and spline editor after this setup step.
