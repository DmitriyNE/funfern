# Engineering log

Low-effort working notes: what changed, what was checked, TODOs, open issues, and
next steps. Short bullets are enough; no entry is required for every tiny edit.
Keep current actions near the top and dated entries newest first. Durable decisions
belong in [architecture.md](architecture.md) and milestone scope in [plan.md](plan.md).

## Current TODOs

- [ ] Turn local-remesh hardening into the foundation for required solution-driven
  adaptive mesh refinement. Preserve stable identities and field state through
  insertion, collapse, repair, and bounded per-frame adaptation work.
- [ ] Extend outer-boundary measurements across more angles/frequencies and assess
  whether higher auxiliary orders justify their state and compute cost.
- [ ] Decide whether the load-compatible closed-wall role still warrants assigned
  face conditions now that holes and open baffles cover the primary workflows.
- [ ] Evaluate a dissipative relative dashpot for thin gaps. Keeping centered time
  integration would require an off-diagonal damping solve; the implemented gap
  spring is conservative.
- [ ] Add standard primitive entries to the Draw catalog beyond Circle, Straight,
  and Custom. Keep creation role selection and primitive parameters in the
  transient popover.
- [ ] Profile the complete geometry-edit handoff on representative full-rebuild
  and local-repair cases. The user reports that the end-to-end handoff still feels
  slow even though the small scripted transfer case is much faster.
- [ ] Replace the sharp zero initialization at newly exposed domain with a localized
  transition/blur pass. A hard jump against the retained field produces artificial
  wideband excitation when an obstacle boundary moves inward. Measure added spectral
  energy and keep established regions outside the transition band unchanged.
- [ ] Run a longer browser soak with a representative multi-obstacle scene and
  record solver throughput and memory behavior over time.
- [ ] Make operator assembly and transfer-map construction resumable if their
  synchronous post-mesh tail becomes visible on larger discretizations.
- [ ] Decide whether paired-trace coarsening is useful for open baffles, and extend
  local repair to closed walls if that legacy role remains worth supporting.

## 2026-09-11 — Local mesh repair for open baffles

- Extended coordinate-only local repair to open baffles without changing their
  two-face topology. Imported trace edges are paired by stable baffle ID and exact
  parameter interval; left/right interior vertices remain distinct and coincident,
  while the two free tips remain shared. Malformed pairs and tip connectivity now
  produce separate typed fallback causes.
- Selected baffle patches with capsules around each old trace segment, new trace
  segment, and endpoint sweep. This keeps a long baffle edit local instead of using
  the bounding box of the whole open curve. Translation, rotation, uniform scale,
  straighten, and control-coordinate edits use this path when motion stays within
  `4h` and the active patch stays below one third of the mesh.
- Added paired curve refinement: if curvature or edge length requires a split, both
  faces receive distinct coincident vertices at the same spline parameter and both
  adjacent elements split together. Knot insertion/removal, continuity and topology
  edits, split/merge, creation/deletion, region reassignment, and closed walls retain
  the full-build path. Trace coarsening remains deferred.
- Update reports and Performance diagnostics now include repaired-baffle and paired
  trace-segment counts. The app regression also requires a local result and a valid
  face-aware field-transfer map with no newly exposed nodes after a baffle move.
- Added mesh tests for paired face orientation and coincidence, shared tips, distant
  element reuse, immutable source meshes, rigid transforms, straightening, forced
  paired subdivision, malformed traces, topology-change fallback, and mixed holes,
  material interfaces, and multiple baffles with stable unmoved topology.
- On Apple M1 Max in release mode, the representative eight-loop scene plus one
  baffle built in 326.0 ms at `h=0.04` and 3.56 s at `h=0.02`. Baffle edits completed
  locally on the first attempt in 49.1 ms and 246.3 ms, preserving 90.9% and 95.9%
  of triangles. Hole/interface edits stayed at 43.9–44.3 ms and 216.8–217.7 ms;
  the longest cooperative slice was 2.201 ms.
- Formatting, Clippy with warnings denied, all 159 workspace tests, native release
  compilation, and release Trunk/WASM packaging pass. Interactive browser
  verification remains deferred by prior agreement.

## 2026-09-11 — Retryable local repair for holes and material interfaces

- Reworked coordinate-edit adaptation as three isolated attempts sourced from the
  unchanged committed mesh. Attempts expand from one to three triangle guard rings,
  add `2h` of reach each time, raise the boundary-split budget through
  256/512/1024, and raise refinement through 512/1024/2048. Total local work
  remains capped at five million units; motion beyond `4h` and patches above
  `max(256, triangles/3)` fall back immediately.
- Added local motion for shared-trace material interfaces, including refinement of
  curved edges with triangles on both sides. Boundary cycles now follow scene loop
  order and stable IDs, so nested region ownership does not depend on numeric ID
  sorting. Per-loop old/new bounding boxes replace the previous moved-point scan
  while selecting nearby bulk vertices.
- Replaced free-form fallback strings in update reports with typed causes plus
  details. Reports retain retry history and expose repair attempts, patch vertices
  and triangles, reuse, moved/inserted/collapsed counts, and final fallback cause.
  Performance diagnostics show these values and a session fallback histogram; the
  status strip names an expanding retry while it is active.
- Added regression coverage for ordinary repeated hole edits, material-interface
  motion on both sides of a shared trace, nested material-interface motion,
  immutable retry restarts, retryable/terminal classification, scheduling
  determinism, large-motion fallback, topology changes, and mesh invariants.
- Open baffles and two-trace walls deliberately retain full rebuilding. Their local
  repair needs paired-side and endpoint-aware topology rather than treating them as
  closed cycles.
- The release timing harness mixed hole and interface edits across eight loops. At
  `h=0.04`, the initial build took 315.5 ms and edits took 41.8–43.1 ms while
  preserving 93.9–94.8% of triangles. At `h=0.02`, the initial build took 4.63 s
  and edits took 207–221 ms while preserving 97.3–98.2%. All six edits succeeded
  on the first local attempt; the longest measured cooperative slice was 3.53 ms.
  Interactive browser checking was skipped as previously agreed with the user.
- Formatting, Clippy with warnings denied, all 155 workspace tests, native release
  compilation, and release Trunk/WASM packaging pass. Trunk 0.21.14 required
  `NO_COLOR=false` because the surrounding CLI environment sets `NO_COLOR=1`, which
  that version parses as an invalid boolean.

## 2026-09-11 — Uniform scale gizmo and modifier-safe span dragging

- Added a square uniform-scale grip to the transform ring. It scales every eligible
  selected curve piece around the shared movable pivot from the drag-start control
  snapshot, uses a diagonal resize cursor, and takes hit priority over the rotation
  ring. Ordinary scaling is continuous; Shift snaps the positive factor to 0.1
  increments. Release creates one document transaction and Escape restores the
  exact pre-drag draft.
- Deferred removal of a Shift-clicked selected span until pointer release. Crossing
  the drag threshold retains the selection instead, so Shift-drag can translate it.
  Starting on an unselected span adds it before moving the resulting transformable
  selection. Shift also temporarily enables the configured grid for curve and
  individual-control translation even when persistent Snap is disabled.
- Invalid scaled geometry remains in the draft while the accepted scene stays
  active. Existing whole-curve and C0-isolated partial-span eligibility, numeric
  transforms, scene files, and transient pivot semantics remain unchanged.
- Automated egui coverage exercises continuous and snapped scaling, scale-handle
  hit priority, pivot invariance, one-entry undo, Escape cancellation, invalid-draft
  persistence, an isolated baffle span, Shift-click toggling, Shift-drag addition
  and retention, and temporary grid snapping for controls and curves.
- Formatting, Clippy with warnings denied, all 151 workspace tests, native release
  compilation, and release Trunk/WASM packaging pass. Interactive browser checking
  remains user-owned.

## 2026-09-11 — Toy-first product roadmap after the editor overhaul

- Keep funfern an exploratory time-domain wave toy. Judge product work by whether
  it creates an interesting, visible experiment that is easy to set up and share.
  Engineering-simulation workflows such as frequency-domain solves, eigenmodes,
  parameter sweeps, and heavy units infrastructure are outside the intended scope.
  Lightweight units may still be useful where they clarify time and wavelength.
- Finish the current geometry slice with a uniform scale handle alongside the
  rotation and pivot gizmos. A selected boundary group scales around the movable
  pivot as one object; snapping and the complete drag follow the existing transform
  and undo semantics.
- AMR remains a required goal from the initial specification. First make local
  insertion, collapse, constraint repair, and state transfer reliable enough to
  reduce full-remesh fallbacks. Then add coefficient-gradient and solution-error
  indicators, bounded work per frame, and a view that explains refinement activity.
- Add an example gallery with names, short descriptions, metadata, and thumbnails.
  Add crash-safe autosave through IndexedDB in the browser and an atomic recovery
  file in native builds. Scene-only screenshots come first, with optional UI and
  plots; browser video capture can follow.
- Build a probe system that shares time-series recording and plotting across point,
  curve, region, and selected-geometry probes. Curve probes should support waterfall
  and average-intensity views. A later far-field probe should remain time-domain,
  using a closed Huygens/Kirchhoff sampling contour to derive angle-time and
  integrated polar radiation views in a homogeneous exterior region.
- Represent spatial variation as a reusable profile plus an explicit placement
  frame. Profiles use local dimensionless coordinates such as `x`, `y`, `r`, and
  `theta`; frames can be world-fixed, object-attached, or independently transformed.
  Evaluate profiles at FEM quadrature points. This supports GRIN and Luneburg media
  without making non-rigid region edits silently warp a field definition.
- Keep distributed excitation separate from passive material properties. A volume
  source targets a region or material and combines its own spatial profile/frame
  with a time signal, enabling shaped and phased radiators. Geometry selections can
  also become reusable probe targets.
- Extend topology so several subdomains may meet at a validated junction, and make
  the rectangular outer extent editable before considering arbitrary outer shapes.
- Add derived vector-style overlays from the scalar solution, such as gradient,
  flux, and intensity/energy flow. A true vector PDE should be added only if it
  enables a compelling toy interaction that these derived fields cannot provide.
- Introduce symmetric positive-definite 2x2 tensor stiffness as the first anisotropic
  material model, initially constant with an orientation gizmo and later compatible
  with spatial profiles. Choose any nonlinear material model for clear visible
  behavior and stable explicit time stepping rather than general constitutive scope.
- PML remains explicitly out of scope. Investigate time-domain nonlocal radiation
  conditions instead: exact or modal DtN on simple enclosing shapes, rational
  auxiliary-state approximations, time-domain boundary integrals, and compressed
  boundary-history representations. Compare their reflection, cost, and visual
  payoff before committing one to the product.
- Proposed sequence: scale gizmo; local-remesh hardening and AMR foundations;
  examples, metadata, thumbnails, and autosave; probes; spatial material/source
  profiles with Luneburg and radiator examples; active solution AMR; far-field
  views; editable domain extent and junction topology; derived vector overlays,
  tensors, a focused nonlinear model; then nonlocal radiation experiments.

## 2026-09-11 — Staged interaction overhaul

- Replaced the overlapping tool and mode state with one explicit interaction mode.
  Draw, pulse placement, and source placement now remain visible over the viewport;
  inspector changes do not silently arm or cancel them. Circle/Straight placement
  is one-shot, while pulse and source tools support repeated clicks and toggle off
  from the same button.
- Made control handles exclusive and boundary spans multi-selectable. Edit now keeps
  selection filters and contextual transform/boundary/topology controls together.
  Whole-object role and delete actions require exactly one complete selected curve,
  eliminating the last-selected-object ambiguity in mixed selections.
- Added viewport feedback for drawing, pulse width, source position, transform pivot,
  rotation, topology endpoints, and assigned boundary laws. Baffle left/right traces
  are drawn coherently; thin gaps use one paired indication. Selection blue is now
  distinct from accepted-geometry teal.
- Simplified all four inspectors. Simulation has named resolution presets, pulse and
  continuous-source parameters, and energy. View owns field intensity, boundary-law
  visualization, a legend, and view reset. Materials presents subdomain assignments
  separately from its named library and supports deletion only when unused. Legacy
  closed-wall loops remain loadable but are hidden from the normal role picker.
- Made the top bar responsive, added close controls to inspectors, added contextual
  cursors/tooltips, and separated persistent errors from four-second success notices.
  Performance diagnostics is one continuous window with a 90-frame plot plus
  average, p95, and peak frame times. The status strip owns current validation,
  rebuild, and handoff stages.
- Automated app/editor tests cover the refactored interaction state and all prior
  geometry, history, persistence, mesh, boundary, and handoff behavior. Formatting,
  Clippy with warnings denied, all 146 workspace tests, native release compilation,
  and a release Trunk/WASM bundle pass. The native automated edit run initialized
  Apple M1 Max / Metal and completed three local mesh repairs. Interactive browser
  checking remains user-owned. Practical keyboard/focus/contrast support is in
  scope; a screen-reader representation of the custom numerical canvas remains
  future work.

## 2026-09-11 — Contextual UI shell and diagnostics

- Replaced the single long left panel with a top action bar, compact task rail,
  central viewport, right inspector, and persistent status strip.
- Moved document actions to the top bar and made task context visible in the
  inspector. Add geometry now uses a transient role/primitive popover; the existing
  Rounded and Custom workflows remain available there.
- Moved detailed frame, mesh, handoff, and solver measurements into one draggable
  Performance diagnostics window with collapsible sections. The lower-right status
  control now summarizes FPS, solver steps per second, DOFs, mesh size, and dt.
  Solver or mesh errors open the diagnostics window automatically; normal mesh
  rebuilds use the progress indicator and do not light the error badge. Metrics
  remain transient and are not serialized.
- Wave playback now starts in the running state, with Run/Pause, Step, and Reset
  grouped on the right side of the top bar while solver tuning remains in the
  Simulation inspector.
- Regrouped the inspector into hideable Edit, View, Simulation, and Materials
  panels. Panel switches and Add geometry now live in the top bar; removing all
  panel selection expands the viewport.
- Added egui coverage for tool-rail/popover discovery, diagnostics opening, and the
  compact metric summary. Formatting, Clippy, all 138 workspace tests, native
  release compilation, and the release Trunk/WASM build pass. Interactive browser
  testing remains user-owned as requested.

## 2026-09-10 — Dense-scene span selection

- Added screen-space box selection for boundary spans. Dragging empty viewport
  space replaces the selection, Shift-drag adds, and Alt-drag subtracts; Escape
  restores the pre-drag selection.
- Added outer-edge, loop, baffle, and all-span filters plus Select filtered,
  Invert, Clear, and Ctrl/Cmd+A. Filters affect span picking and bulk operations;
  control handles remain an explicit single-control selection path.
- The live marquee and filter operations are transient and create no geometry or
  history transaction. Baffle face choice remains coherent across every selected
  baffle span.
- Automated egui tests exercise filtered baffle selection, subtraction, select-all,
  Escape restoration, and unchanged history.
- Formatting, Clippy with warnings denied, all 134 workspace tests, native release
  compilation, and the release Trunk/WASM build pass. Interactive browser testing
  was skipped as requested.

## 2026-09-10 — Baffle timestep and geometry handoff repair

- Internal-constraint insertion now relocates a nearby unconstrained bulk vertex
  onto the exact curve sample when its complete triangle fan remains valid. This
  avoids accidental slivers without moving the baffle or changing its paired cut.
- On Apple M1 Max / Metal, the assigned-law `--wave-gpu-check` at parent h=0.08
  improved from `dt=5.4207e-4`, a `2.81°` minimum angle, and about `2.10` simulated
  seconds per wall second to `dt=2.3337e-3`, `17.25°`, and `6.58–9.77` across two
  runs. DOFs changed only from 9,720 to 9,690. The check still agrees with the f64
  solver to `1.64e-6` relative L2 error after 128 steps.
- Quadratic state transfer tags baffle vertices and edge nodes by stable boundary
  ID and face. Coincident points prefer an old element on the same face; moved
  trace points fall back to the containing bulk element. Geometry edits with a
  baffle now prepare the normal transactional GPU handoff instead of resetting
  the field.
- Added regressions for the former CFL-sliver scene, preservation of distinct
  coincident face values, and a real editor remesh with zero exposed target nodes.
- Formatting, Clippy with warnings denied, all 132 workspace tests, native release
  compilation, the native GPU check, and the release Trunk/WASM build pass.
  Interactive browser testing was skipped as requested.

## 2026-09-10 — CI and GitHub Pages

- Added a GitHub Actions pipeline for Rustfmt, Clippy with warnings denied, all
  workspace tests, the native release build, and the release Trunk/WASM bundle.
- Successful runs from `main` upload the `dist` artifact and deploy it through
  GitHub's Pages environment. Pull requests build the same browser target without
  deploying it.
- Pinned Rust 1.96.0 and Trunk 0.21.14. The Pages build takes its repository base
  path from `actions/configure-pages`, so hashed WASM and JavaScript assets load
  correctly below `/funfern/` and continue to work with a future custom domain.
- Enabled the repository Pages site with the workflow publishing source and HTTPS
  enforcement at `https://dmitriyne.github.io/funfern/`.

## 2026-09-10 — Safe smoothing and loop role conversion

- Added repeated-knot removal for open and periodic cubics, including the seam.
  It reconstructs the lower-multiplicity control space and reinserts the knot to
  test exactness. Edited incompatible corners now fall back to a least-squares
  reshape, so C0 can always be promoted back to C1 or C2. The UI reports a
  convex-hull upper bound on displacement and the edit remains one undo step.
- Replaced one-way continuity buttons with explicit C2/C1/C0 choices. Exact
  sharpening and exact-first smoothing share one inspector and retain span
  assignments.
- Added loop conversion between hole, material interface, and two-sided closed
  wall. Hole conversion allocates an owned region using the selected material;
  interface/wall conversion retains it. Converting to a hole removes an empty
  region and rejects child geometry atomically.
- Core tests cover round-trip removal at open and periodic knots, approximate
  smoothing after corner deformation, and the reported displacement bound.
  Editor tests cover exact smoothing, undoable reshaping, role ownership,
  persistence, validation, and nonempty-interior rejection.
- Verification passes formatting, Clippy with warnings denied, all **129 workspace
  tests**, native release compilation, and the optimized Trunk/WebGPU build.

## 2026-09-10 — Rigid partial-span transforms

- Added exact control-support queries for open and periodic cubic spans. A partial
  selection becomes transformable when every exposed end is C0 or a baffle tip;
  selected spans receive one rigid affine map while neighbors remain connected at
  the shared corner.
- Added **Isolate selection at C0**, which finds every selected/unselected
  transition and performs all required shape-preserving knot insertions as one
  history action. It supports multiple disjoint pieces, multiple curves, open
  endpoints, and loop selections crossing the periodic seam.
- Pivot and grid snapping now use only selected arc length rather than the full
  parent curves. Viewport dragging, the rotation gizmo, numeric transforms, and
  alignment all use the same isolated control groups.
- Direct UI tests cover atomic baffle isolation, exact partial-span translation,
  unchanged remote endpoints, undo, and a seam-wrapped loop selection.
- Verification passes formatting, Clippy with warnings denied, all **126 workspace
  tests**, native release compilation, and the optimized Trunk/WebGPU build.

## 2026-09-10 — Explicit corners and baffle topology

- Separated cubic knot multiplicity from positive knot spans. Periodic and open
  splines now support exact C2→C1→C0 refinement, including the periodic seam,
  without changing geometry or per-span boundary assignments.
- Added a topology inspector at the end knot of a selected span and distinct gold
  diamond markers for repeated knots. The editor avoids an implicit approximate
  smoothing operation after corner controls have moved; Undo retains exactness.
- Added shape-preserving baffle split and endpoint merge. Split retains the start
  half's stable ID, merge reconciles parameter direction and left/right laws, and
  coincident same-region tips validate and mesh as junctions.
- Scene JSON version 8 stores knot multiplicities. Versions 1–7 continue to load
  as smooth splines. Split, merge, continuity changes, and round trips have direct
  core/editor coverage.
- Verification passes formatting, Clippy with warnings denied, all **124 workspace
  tests**, native release compilation, and the optimized Trunk/WebGPU build. The
  split test also meshes the shared tip and assembles the quadratic wave operator.

## 2026-09-10 — Span selection, bulk boundary editing, and transform gizmo

- Replaced control multiselection with exclusive single-control editing and
  Shift-based span multiselection. Ctrl/Cmd-click selects a complete curve;
  Ctrl/Cmd+Shift-click adds or removes all of its spans.
- Unified bulk condition assignment across compatible outer, hole, and oriented
  baffle-face spans. Mixed values are explicit, all targets validate before one
  history action, and applying a face condition converts selected thin-gap spans
  to independent faces.
- Complete selected curves translate, rotate, scale, snap, and align without
  changing their relative controls. Grid snapping applies one displacement to the
  arc-length centroid. Partial-span transforms remain disabled until continuity
  boundaries can isolate their support.
- Added a viewport rotation ring, a draggable transient pivot, baffle direction
  arrows, and coherent Left/Right highlighting across every selected span.
- Verification passes formatting, Clippy with warnings denied, all **122 workspace
  tests**, native release compilation, and a release Trunk build. Playwright
  confirmed WebGPU startup and rendering with no console errors.

## 2026-09-10 — Product spline transforms, duplication, and straight baffles

- Added Shift-based control and whole-curve multi-selection. Dragging a selected
  handle moves the complete selected set, while dragging a curve translates all
  of its controls. Each gesture remains one document-history action.
- Added a transform inspector with translation, rotation, uniform scale, grid
  snapping, and axis alignment. It uses the selected controls' centroid as pivot
  and can transform controls from loops and baffles together.
- Added loop and baffle duplication with new stable IDs and copied intervals and
  span assignments. Duplicated material-interface and wall loops receive a new
  interior region carrying the source material.
- Added **Straighten baffle**, which distributes all controls along the segment
  between its endpoints and makes an exact straight cubic boundary. Explicit
  corners and continuity, repeated knots, and split/merge remain the next spline
  data-model stage.
- Verification passes formatting, Clippy with warnings denied, all **117 workspace
  tests**, native release compilation, and the release Trunk/WebGPU build. A
  Playwright smoke check confirmed startup and rendering without console errors.

## 2026-09-10 — Driven and absorbing internal faces

- New scenes now start with second-order auxiliary absorption on all four outer
  sides. Loading versions 1–5 still restores their historical implicit reflecting
  walls, while version 6 and later retain the explicitly stored assignments.
- Extended hole and baffle faces with harmonic prescribed Neumann flux and strong
  Dirichlet displacement, plus first- and second-order outgoing conditions. The
  second-order condition assembles boundary damping and the tangential auxiliary
  operator using the material adjacent to that face.
- Replaced the GPU's outer-side-only signal assumption with per-node Dirichlet
  signals and two bounded Neumann signal/weight slots. They are packed into the
  existing node storage buffer, keeping the portable eight-binding layout.
- Made baffle span modes explicit and exclusive: either independent left/right
  face conditions or one coupled thin-gap spring. Selecting thin gap clears both
  face laws. Core validation rejects parallel combinations.
- Scene JSON is version 7. Versions 1–6 remain readable; version-6 baffles that
  combined thin-gap coupling with face conditions migrate with the coupled law
  taking precedence.
- Verification passes formatting, Clippy with warnings denied, all **113 workspace
  tests**, native release compilation, and the release Trunk/WebGPU build. The
  Apple M1 Max / Metal GPU comparison exercised driven baffle faces at 9,720 DOFs;
  relative L2 errors were `5.84e-6` for current displacement, `5.82e-6` for the
  previous level, and `1.28e-5` for auxiliary state.
- The native transfer check also runs driven Dirichlet and Neumann data on hole
  spans. Geometry, boundary-law, and material transactions passed their f64
  comparisons; their largest reported relative L2 errors were `2.72e-8`,
  `3.09e-9`, and `3.09e-8`, respectively.
- A clean Compose rebuild serves the new release bundle from a healthy nginx
  container. Chromium initialized BrowserWebGPU and rendered a 2,000 × 1,300
  backing canvas at a 1,000 × 650 CSS viewport without shader or application
  errors; the existing optional favicon 404 and capability warning remain.

## 2026-09-10 — Unified boundary selection and inspector

- Replaced the four outer-side buttons and the separate hole/baffle controls with
  one boundary target selected in the viewport. Outer edges, hole knot spans, and
  baffle knot spans share one inspector; the selected geometry is highlighted.
- The inspector exposes conditions implemented for its target. The baffle
  left/right face choice remains explicit because both traces occupy the same
  screen curve.
- Boundary editing no longer depends on an initialized wave operator. Boundary
  selection remains transient and does not enter history or scene files.
- Added egui input coverage for outer-edge selection and assignment and migrated
  the existing hole/baffle selection tests to the unified interaction.
- Verification passes with formatting, Clippy warnings denied, all 110 workspace
  tests, native release compilation, and a release Trunk/WebGPU build. Chromium
  rendered the updated inspector and viewport without runtime console errors; the
  only browser error was the existing missing optional favicon.

## 2026-09-09 — Live browser loop restored

- Playwright reproduced the reported frozen first frame in Chromium 152 on the
  Apple Metal 3 WebGPU adapter. The browser created the shader module but rejected
  the compute pipeline because its requested `step` entry point did not exist.
- Bevy 0.19's browser shader path reparses WGSL with Naga and emits WGSL again.
  `step` is also a WGSL built-in, so Naga renamed the user function while Bevy's
  pipeline descriptor retained the original name. Renamed the compute entry point
  to `advance_wave`; the pipeline now initializes and the browser event loop remains
  live.
- Post-fix browser checks confirm WebGPU startup with 9,664 DOFs, Run/Pause input,
  and a resize from 1200×800 to 1000×650 with the canvas backing store updating
  from 2400×1600 to 2000×1300. The user independently confirmed that controls and
  resizing work. The observed display frame time was about 11.5 ms in the initial
  one-obstacle scene. A longer multi-obstacle soak remains outstanding.
- Final verification passes formatting, Clippy with warnings denied, all **107
  tests**, native release compilation, the Apple M1 Max / Metal GPU reference
  check, and a release Trunk build. A clean Compose image rebuild completes, its
  nginx service reports healthy, and the exact rebuilt image starts in Chromium
  with no console errors.

## 2026-09-09 — Assigned outer-side Dirichlet and Neumann data

- Replaced the global outer-boundary mode with four independently assigned box
  sides. Each side supports homogeneous Neumann/reflecting, prescribed Neumann
  flux, prescribed Dirichlet displacement, first-order outgoing, or second-order
  auxiliary behavior. The selected side is highlighted in the viewport.
- Prescribed data is constant along its side and uses the analytic time law
  `offset + amplitude sin(2π f t + phase)`. Neumann data is assembled as a
  quadratic boundary load. Dirichlet values are imposed strongly at initialization,
  every centered step, and field-transfer commits. Different Dirichlet signals on
  adjacent sides are rejected as a persistent invalid draft because their corner
  value would be contradictory.
- The GPU keeps the eight portable storage bindings by packing four signals into
  the forcing buffer and per-side Neumann weights plus a Dirichlet side index into
  each node record. Transfer gained a bounded preparation dispatch so target
  Dirichlet values participate in the reconstructed previous level. Auxiliary
  radiation memory is suppressed where an essential condition owns a mixed corner.
- Scene JSON is now version 6 and stores outer-side laws in both draft and accepted
  scenes. Versions 1–5 migrate to four reflecting sides. Boundary edits participate
  in document history and reuse the existing mesh through the operator transaction.
- The expanded CPU/editor suite has **109 tests**. The native mixed-condition GPU
  check exercises both prescribed laws and both outgoing orders at 9,720 DOFs; its
  current, previous, and auxiliary relative L2 errors against f64 are respectively
  `7.51e-6`, `7.49e-6`, and `1.37e-5`.
- Formatting, Clippy with warnings denied, native release compilation, the existing
  geometry/boundary/material GPU transfer check, and release Trunk packaging pass.
  A clean Compose image rebuild is healthy, and Chromium starts that exact image
  with no console errors.

## 2026-09-09 — Portable WebGPU wave bindings

- The first actual browser launch rendered the initial frame but then stopped
  processing controls and resize events. Pipeline startup was creating ten storage
  bindings in the wave compute stage, above WebGPU's portable per-stage limit of
  eight; the native Metal adapter had accepted that layout.
- Packed source and pulse parameters into one forcing buffer and paired their
  precomputed path-distance weights in one `vec2` buffer. The wave stage now has
  eight storage bindings. The transfer shaders read the same packed forcing buffer
  and remain at eight bindings. GPU-owned time and state stay in separate buffers,
  so source edits cannot overwrite the simulation clock.
- All 33 application/editor tests and Clippy with warnings denied pass. The native
  release GPU check passes at 9,720 DOFs with current/previous/auxiliary relative
  L2 errors of `7.85e-6`, `7.80e-6`, and `7.39e-6`. A clean release Trunk build
  passes and its WASM contains the eight-binding shader. Subsequent Playwright
  verification exposed the separate entry-point issue documented above.

## 2026-09-09 — Containerized browser serving

- Added a Docker Compose service that builds the locked release WASM application
  with Rust 1.96 and Trunk 0.21.14, then serves only the static bundle from nginx.
  The host port defaults to 8080 and can be changed with `FUNFERN_PORT`.
- The nginx configuration supplies the WASM MIME type through the standard MIME
  table, disables index caching, caches content-hashed JS/WASM assets, and exposes
  a container health check. Build context excludes Git and local build artifacts.
- `docker compose config` and a clean ARM64 `docker compose build` pass. The
  running service becomes healthy on port 8080; probes return 200 with `text/html`
  and `no-cache` for the index, and `application/wasm` with immutable caching for
  the hashed 24.7 MiB WASM asset. The verification container was removed afterward.

## 2026-09-09 — Renamed project to funfern

- Renamed the workspace packages and source directories to `funfern-core` and
  `funfern-app`, including Rust crate imports, documented commands, and Cargo lock
  entries. The native executable and Trunk artifacts now use the `funfern-app` name.
- Updated the browser title/canvas target, native window title, editor heading,
  startup message, scene-file filters, and default `funfern-scene.json` filename.
  The version 5 scene schema is unchanged and existing scene files remain compatible.
- Formatting, Clippy with warnings denied, all **107 tests**, native release
  compilation, Apple M1 Max / Metal solver and transfer checks, and a release
  WASM/Trunk build pass under the new package names. Interactive browser testing
  was skipped as previously requested.

## 2026-09-09 — Assigned hole-span conditions

- Periodic loops now carry one exterior-face condition per knot interval. Clicking
  a hole curve selects and highlights its logical span; the panel assigns reflecting
  or scaled matched impedance behavior against the exterior material.
- Hole impedance is assembled on every labeled P2e edge belonging to the selected
  spline span with positive lumped boundary weights. A condition-only edit is
  excluded from geometry equality, so it rebuilds the operator on the same mesh
  and preserves the live field through the existing transaction.
- Shape-preserving insertion copies the split span condition, including at the
  periodic seam. Removal refuses to merge unequal neighboring conditions. Both
  actions and condition assignment retain their existing one-entry history rules.
- Scene JSON is version 5 and requires hole-span conditions in new files. Versions
  2–4 migrate periodic loops to reflecting spans; version 1 retains its existing
  background-hole migration. Invalid lengths and impedance coefficients are
  rejected before document replacement.
- Final verification passes formatting, Clippy with warnings denied, all **107
  tests**, native release compilation, and a release WASM/Trunk build. The Apple
  M1 Max / Metal solver regression remains within `7.85e-6` current-state relative
  L2 error with an isolated-region peak of exactly `0.0`; the transfer regression
  reports `7.49e-16` geometry current-state error and zero boundary/material
  current-state error. Interactive browser testing was skipped as requested.

## 2026-09-09 — Assigned open-baffle span laws

- Each nonempty open-spline knot span now owns independent left/right face
  conditions and an optional paired-trace law. Clicking the curve selects its
  stable parameter span; the panel selects a face, highlights it with a visible
  offset, and assigns reflecting or scaled matched impedance behavior.
- Matched face impedance adds positive lumped damping using the adjacent medium's
  characteristic impedance. The thin-gap option adds a symmetric conservative
  spring between matching P2e trace nodes. Its stiffness is included in the
  spectral time-step bound, and a 200-step f64 regression conserves the discrete
  energy.
- Knot insertion copies the affected span law to both children. Removal rejects
  an ambiguous merge until neighboring laws agree. Law edits are single history
  actions, reuse the existing mesh, and use the same field-transfer transaction
  as material and outer-boundary changes.
- Scene JSON is version 4. It persists all face/coupling coefficients and migrates
  version 3's whole-baffle reflecting law. Invalid counts and nonpositive or
  nonfinite coefficients are rejected before replacing the document.
- Final verification passes formatting, Clippy with warnings denied, all **102
  tests**, native release compilation, and a release WASM/Trunk build. The native
  Apple M1 Max / Metal run exercises both assigned laws and agrees with f64 to
  `7.85e-6` relative L2 error after 128 steps; its wall-isolated peak remains
  exactly `0.0`. The existing transfer regression also passes with geometry current
  error `7.49e-16`. Interactive browser testing was skipped as previously requested.

## 2026-09-09 — Open reflecting baffles

- Replaced the closed-wall creation workflow with open reflecting baffles; legacy
  closed-wall scenes remain load-compatible. Preset and custom creation, selection,
  dragging, coordinates, shape-preserving insertion, reshaping removal, history,
  invalid drafts, and deletion use the normal editor lifecycle.
- Added dependency-free clamped nonuniform cubic B-splines with exact de Boor
  evaluation, two derivatives, adaptive sampling, closest-parameter refinement,
  and shape-preserving knot insertion. Version 3 JSON stores open boundaries and
  version 1/2 scenes continue to load.
- The mesher first refines the closed material domains, recovers each sampled open
  curve through deterministic edge flips, duplicates interior trace vertices, and
  rewires one triangle fan. Free tips remain shared, so the two reflecting faces
  are uncoupled locally while the surrounding domain remains reachable around
  either endpoint.
- Pulse and continuous-source stencils use truncated shortest-path distances over
  the P2e mesh when baffles are present. This prevents a Gaussian from appearing
  directly across the coincident faces while allowing support to go around a tip.
- Open-baffle movement currently performs a full rebuild and resets the wave field.
  The existing region-component transfer cannot distinguish two faces belonging
  to the same globally connected region; a side-aware transfer is required before
  live field preservation can be enabled safely.
- Final verification passes formatting, Clippy with warnings denied, all **95
  tests**, native release compilation, and a release WASM/Trunk build. Tests cover
  open-spline calculus and insertion, validation failures, multiple independent
  baffles, paired trace labels, shared free tips, path-aware forcing, editor history,
  and versioned scene round trips.
- The native Apple M1 Max / Metal GPU regression completes with current-state
  relative L2 error `1.48e-6`, an isolated-region peak of exactly `0.0`, and
  `14.61` simulated seconds per wall second. Geometry, boundary-condition, and
  material transfer regressions also pass; geometry-transfer current-state error
  is `7.49e-16`. Interactive browser testing was skipped at the user's request.

## 2026-09-09 — Wall-isolated excitation and transfer

- User testing found apparent energy leakage into retained subdomains enclosed by
  two-sided walls. The assembled wall operator was already block-disconnected;
  the leak came from pulse and continuous-source Gaussians being applied by
  Euclidean distance to every node on both sides.
- GPU nodes now carry up to two exact 64-bit region IDs. Pulse and continuous
  sources carry their containing region and excite matching nodes only. Shared
  interface nodes retain membership in both regions, so transmitting interfaces
  do not become artificial source barriers.
- Geometry transfer now computes connected components of regions joined by
  material interfaces and prevents interpolation across walls for every target
  DOF. Newly created or exposed wall-separated areas initialize independently.
- Added an f64 evolution regression with field one outside and zero inside a wall;
  both disconnected Neumann domains remain constant for 100 steps. The existing
  coincident-trace transfer test now checks every P2e node rather than trace nodes
  alone.
- Final verification passes formatting, Clippy with warnings denied, all **85
  tests**, native release compilation, and a release WASM/Trunk build. The native
  Apple M1 Max / Metal GPU regression excites only the exterior of a centered
  two-sided wall and measures an interior peak of exactly `0.0`; its GPU/CPU
  relative L2 error is `1.35e-6` after the scripted evolution.
- The native transfer regression also passes after the component restriction:
  geometry-transfer current-state relative L2 error is `7.49e-16`; boundary and
  material transactions report zero current-state error.

## 2026-09-09 — Interior regions and assigned materials

- Added stable region and material IDs plus explicit Hole, Material interface,
  and Closed wall loop roles. Validation now accepts consistent nested inclusions,
  rejects ownership that disagrees with containment, and retains invalid drafts.
- The mesher triangulates every retained region. Interface sides share constrained
  vertices and P2e trace DOFs; wall sides use coincident geometry with distinct
  vertices and DOFs. Triangles carry region IDs and final verification checks the
  expected one- or two-sided adjacency.
- P2e assembly reads density, stiffness, and volume damping per triangle region.
  Material edits reassemble the operator on the existing mesh and use the normal
  GPU field-transfer transaction. Geometry comparison excludes coefficients, so
  these edits never start a mesh job.
- The panel creates all three loop roles, assigns materials to the background or
  retained interiors, edits coefficients, selects regions from the viewport, and
  draws material fills. Multi-region geometry motion intentionally falls back to
  the full mesher; the current local repair algorithm is restricted to holes.
- Scene JSON is version 2 and stores materials, regions, roles, and both document
  scenes. Version 1 files migrate to default-medium background holes. Load remains
  atomic and permits structurally valid invalid draft geometry.
- Automated coverage includes nested ownership, shared and duplicated traces,
  piecewise mass/stiffness/damping, unknown-region rejection, material history,
  v1 migration/v2 round trips, and same-mesh material transactions. Interactive
  browser testing remains deferred by the user.
- The native Apple M1 Max / Metal transfer check now includes a density/stiffness/
  damping change after its geometry and boundary transactions. The material edit
  reused the mesh, matched the f64 expected current exactly and the previous level
  to relative mass-weighted L2 error `3.01e-8`, and completed in 15.4 ms including
  5.13 ms operator/map preparation. The existing Metal bindless and shutdown
  readback warnings remain unchanged.
- Formatting, Clippy with warnings denied, all 84 tests, native release compilation,
  release Trunk packaging, the 128-step GPU reference check, and the expanded
  native transfer check pass. Interactive browser testing remains deferred.

## 2026-09-09 — Second-order auxiliary radiation

- Added a selectable second-order Engquist-Majda condition alongside reflecting
  and first-order outgoing modes. It introduces `ψ_t = u` on the outer boundary
  and the symmetric positive-semidefinite tangential term `(k c/2) KΓ ψ` while
  retaining diagonal mass/damping and explicit centered stepping.
- Quadratic edge stiffness is assembled into the main CSR sparsity. Shared global
  corner nodes sum the two incident side contributions. CPU and WGSL advance the
  memory with the same trapezoidal update, include its force in velocity/time-level
  reconstruction, and include its quadratic term in the energy diagnostic.
- Geometry transactions preserve and interpolate memory only from second order to
  second order. Entering the mode initializes it to zero; leaving drops it. The
  native transfer check moved a nonzero memory field through a control edit with
  relative mass-weighted L2 error `2.95e-15`, then switched to first order with
  zero residual memory. Two final same-mesh boundary runs took 78–145 ms end to
  end, including 5.12–6.14 ms operator/map preparation; the remaining observed
  latency is GPU scheduling/readback and window-frame timing.
- The Apple M1 Max / Metal check ran 128 production-timestep steps and matched f64
  with relative errors `2.01e-6` (current), `2.00e-6` (previous), and `3.92e-7`
  (memory), at 19.2–19.9 simulated seconds per wall second for two measured
  dispatch and readback intervals.
- Finite-packet reflection results (`|R|` is the square root of residual-energy
  ratio against a reflecting run):

  | wavelength | parent h | angle | first order | second order | second-order ideal |
  | ---: | ---: | ---: | ---: | ---: | ---: |
  | .40 | .08 | 0° | .0257 | .0159 | 0 |
  | .40 | .08 | 30° | .0981 | .0684 | .0052 |
  | .20 | .04 | 0° | .0106 | .0068 | 0 |
  | .20 | .04 | 30° | .0978 | .0471 | .0052 |

  The wavelength-.4 normal packet stayed finite through t=10 and retained
  `4.794e-5` of its initial energy. A 10,000-step core regression also passes at
  the production recommended timestep.
- All 75 tests, formatting, Clippy with warnings denied, native release compilation,
  both native Metal GPU checks, and `NO_COLOR=true trunk build --release` pass.
  Interactive browser verification remains deferred at the user's request.

## 2026-09-09 — Product roadmap before IGA

- IGA now follows three product milestones: interior topology and material
  assignment, per-span boundary conditions, and expanded spline editing.
- Stable region/material/span identities are the shared foundation. Holes,
  transmitting material interfaces, and two-sided internal walls remain distinct
  semantics rather than modes inferred from loop winding.
- Higher-order radiation was placed before the three product milestones. Per-span
  assignment later generalizes it to selectable subspans and mixed junctions.

## 2026-09-09 — First-order outgoing outer boundary

- Added the selectable first-order condition `∂n u = -u_t/c` on outer-square
  edges. Its weak boundary impedance `sqrt(rho k)` is mass-lumped onto each P2e
  endpoint/midpoint/endpoint triplet with positive Simpson weights. Obstacle edges
  remain reflecting, and reflecting remains the startup default.
- Boundary changes assemble only a new operator, reuse the exact committed mesh,
  and pass through the existing GPU transaction. Displacement, reconstructed
  velocity, simulation clock, and run state are retained; no nodes are newly
  exposed and no mesh job starts.
- Added exact assembly checks for impedance, positive damping, unchanged mass and
  stiffness, malformed outer edges, and an editor regression for operator-only
  transactions. The native transfer check now continues through a reflecting to
  outgoing live-field handoff and compares both GPU levels with f64. A direct
  identity map avoids spatial point location for same-mesh changes. On Apple M1
  Max / Metal the final boundary transaction took 41.5 ms end to end, including
  4.6 ms operator/map preparation. Current and previous relative mass-weighted L2
  errors were 0 and 8.86e-10. The 128-step GPU check now runs the outgoing
  operator so its nonzero damping path is exercised; it matched f64 to 2.09e-6
  and 2.08e-6 for the two levels at 9.12 simulated seconds per wall second.
- `wave_boundary_reflection` measures finite Gaussian packets with a timestep of
  .225 of the conservative limit:

  | wavelength | parent h | angle | measured energy-equivalent `|R|` | ideal plane-wave `|R|` | outgoing energy retained |
  | ---: | ---: | ---: | ---: | ---: | ---: |
  | .40 | .08 | 0° | .0257 | 0 | 6.594e-4 |
  | .40 | .08 | 30° | .0981 | .0718 | 9.616e-3 |
  | .20 | .04 | 0° | .0106 | 0 | 1.133e-4 |
  | .20 | .04 | 30° | .0978 | .0718 | 9.562e-3 |

  Reflecting reference energy stayed at 1.000000 in all four measurements. The
  wavelength-.4 normal case stayed finite through t=10 and retained 4.799e-5 of
  initial energy. The measured packet values include finite bandwidth, diffraction,
  and discretization; they are not expected to equal the monochromatic plane-wave
  coefficient exactly.
- Higher-order auxiliary boundary dynamics and corner coupling remain the next
  radiation milestone. All 74 tests, formatting, Clippy with warnings denied,
  native release compilation, both native Metal GPU checks, and release Trunk WASM
  packaging pass. Interactive browser verification remains deferred at the user's
  request.

## 2026-09-09 — Production enriched-quadratic GPU solver

- Replaced the application's P1 operator with the seven-node mass-lumped enriched
  quadratic operator and changed the default parent mesh from h=.04 to h=.08. The
  empty-box convergence study gives the new default far smaller phase error at
  lower DOF than the former h=.02 P1 comparison.
- The existing CSR WGSL evolution kernel now consumes quadratic node positions,
  damping, and rows. Pulse and continuous-source profiles are sampled at every
  vertex, shared edge midpoint, and element centroid. The field view splits each
  parent triangle into six display triangles around those nodes.
- Added `QuadraticTransferMap`: every target solution node is located in the source
  parent mesh and receives the seven enriched basis weights. Tests reproduce full
  quadratic polynomials between meshes, arbitrary nodal state on a self-map, and
  reject mismatched operator/mesh revisions. Newly exposed solution nodes retain
  the existing explicit zero policy pending the localized smoothing experiment.
- A first direct seven-point transfer repeated source stiffness rows and took
  53.6 ms through tagged readback. The final three-dispatch path reconstructs and
  caches velocity once per old DOF in the old state's disposable scratch component,
  then gathers displacement/velocity and builds the new previous level. Final runs
  took 21.1–23.3 ms for this stage while retaining the portable eight-storage-buffer
  ceiling.
- Native Apple M1 Max / Metal checks:

  - 9,326 DOFs, dt=.00479230, 128 GPU steps: current and previous relative
    mass-weighted L2 errors `1.887e-6` and `1.874e-6`; solve through tagged readback
    was 36.86 ms, or 16.64 simulated seconds per wall second.
  - A real control edit and quadratic transfer produced current/previous relative
    errors `1.662e-8` and `2.965e-8`. Across final runs, mesh request through commit
    was 58–72 ms and synchronous operator/map preparation was 6.9–8.7 ms.
  - The eight-obstacle parent-h=.08 scripted run produced 2,921 parent triangles.
    Three local edits preserved 2,540–2,541 triangles; request-to-commit was
    125–215 ms, active meshing 25–29 ms, and largest slices 7.4–8.9 ms. Multi-frame
    scheduling gaps and over-budget mesh slices remain performance work.

- All 71 tests, formatting, Clippy, native release compilation, and native GPU
  checks pass. Interactive browser testing remains deferred at the user's request.

## 2026-09-09 — Enriched quadratic wave reference and P1 comparison

- Added a dependency-free seven-node `P2` plus cubic-bubble triangular operator.
  Edge midpoint DOFs are shared, bubble DOFs are element-local, the positive nodal
  mass weights are 1/20 at vertices, 2/15 at edge midpoints, and 9/20 at the
  centroid, and stiffness uses a symmetric degree-four-exact quadrature rule.
- Tests cover nodal cardinality, affine reproduction and exact affine energy,
  quadrature moments through degree four, shared-edge numbering, positive/exact
  total mass, matrix symmetry/nullspace, 2,000-step energy conservation, stationary
  constants, and malformed inputs.
- Extended `wave_convergence` to compare P1 and enriched quadratic (`P2e`) with
  temporal refinement. At `0.225 dt_max`, t≈10 on Apple M1 Max:

  | element | parent h | DOFs | dt max | buffers | phase | amplitude | L2 | CPU throughput |
  | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
  | P1 | .04 | 6,328 | .0100807 | .55 MiB | -.959 rad | -.00461 | .426 | 21.57 sim s/wall s |
  | P1 | .02 | 25,218 | .0050806 | 2.20 MiB | -.238 rad | -.000114 | .0284 | 3.61 sim s/wall s |
  | P2e | .08 | 9,215 | .0069839 | 1.15 MiB | -.00519 rad | -4.08e-6 | .00237 | 8.61 sim s/wall s |
  | P2e | .04 | 37,443 | .0034506 | 4.70 MiB | +.000505 rad | -2.44e-8 | .000188 | .99 sim s/wall s |

  At lower cost than the fine P1 case, parent-h=.08 P2e cuts long-time phase error
  by about 46× and L2 error by about 12×. At `0.1125 dt_max`, its phase error is
  -.00818 rad; the larger step partly cancels spatial phase error, so both timestep
  levels remain in the benchmark. The result supports P2e as the next GPU element,
  but these are f64 CPU timings and do not claim browser or GPU throughput.
- Interactive browser testing remains deferred at the user's request.

## 2026-09-09 — Transactional geometry edits and GPU state transfer

- Completed meshes now remain candidates until their operator, timestep, transfer
  map, GPU buffers, and transferred state are ready. The active mesh and field stay
  visible and continue evolving during meshing. Scheduling pauses only after the
  candidate is ready and prior GPU steps have been encoded; a finite tagged readback
  commits mesh, operator, timestep, and field together. Failed candidates retain the
  active simulation, and retained GPU buffers support rollback on shader failure.
- Added a dependency-free spatially indexed barycentric `TransferMap`. Target
  vertices outside the old triangulated domain initialize to zero displacement and
  velocity. Revision and size checks reject stale maps.
- The first transfer dispatch reconstructs velocity from old previous/current
  levels, stiffness, damping, forcing, and timestep, then maps current displacement
  and velocity. The second applies the new operator and initializes the new previous
  level consistently with the new timestep. GPU time is copied so continuous-source
  phase remains continuous.
- Fixed a pre-existing dynamic-buffer race found by the new checks: source/pulse
  updates now replace their asset buffer and advance a binding revision, and compute
  waits for a bind group with that exact revision. This prevents stepping once with
  stale pulse data.
- Native `--wave-transfer-check` on Apple M1 Max / Metal injected a nonzero field,
  performed a real control edit and remesh, then matched the independent f64 result:
  current relative mass-weighted L2 `4.68e-9`, previous `2.52e-8`. The small-edit
  mesh request through atomic commit took `116–117 ms`; synchronous operator/map
  prep took `4.4–4.6 ms`, and transfer submission through tagged readback took
  `21.6–25.4 ms` across two runs. The existing 128-step GPU check still matches f64
  at about `1.22e-6` for both levels.
- Operator assembly plus transfer-map construction is still a synchronous tail when
  a mesh job finishes. Browser interaction and long-run verification remain deferred
  at the user's request.
- User feedback after exercising the implementation: the full geometry-movement
  handoff is still somewhat slow. Zero initialization of newly exposed regions also
  creates a sharp field profile and artificial broadband excitation; add a localized
  smoothing policy in a future pass.

## 2026-09-09 — Static P1 CPU/GPU wave solver

- Added dependency-free f64 P1 assembly and reference evolution. The operator uses
  CSR stiffness, lumped mass/damping, reflecting natural Neumann boundaries, and
  a damped centered difference update. A Gershgorin bound on `M^-1 K` selects a
  conservative timestep. Validation rejects invalid coefficients, topology,
  masses, time steps, sizes, and non-finite states.
- Added a Bevy render-world f32 compute kernel on the existing wgpu device. Each
  DOF gathers its CSR row into uniquely owned output, followed by a level rotation;
  there are no floating-point scatter atomics. GPU state stays resident. Bevy's
  asynchronous buffer readback feeds interpolated egui triangle colors and CPU
  energy diagnostics. GPU-written step markers reject stale readbacks.
- Enabled Run/Pause, Step, Reset, Gaussian pulse placement, a movable continuous
  sinusoidal source, simulation-speed and color-gain controls. Work is capped at
  16 substeps per display frame. The panel reports DOFs, GPU memory, timestep,
  simulated time, substeps, throughput, operator assembly, and discrete energy.
  A newly published mesh resets state; transfer is milestone 4 work. The previous
  committed solver continues during mesh preparation and invalid drafts.
- Native `--wave-gpu-check`, Rust 1.96.0, Apple M1 Max / Metal: 6,240 DOFs,
  dt=0.00742465, 128 steps. GPU versus f64 relative mass-weighted L2 error was
  **1.223e-6** for current and **1.225e-6** for previous. Solve submission through
  tagged readback took **44.92 ms**, equivalent to **21.16 simulated s/wall s**.
  Startup/mesh preparation took 1.77 s and is excluded from that throughput.
- `wave_convergence` uses an actual empty-domain mesh and the wavelength-0.4 mode
  `cos(5π(x+1))`. At `0.225 dt_max`:

  | h | DOFs | triangles | dt max | phase at t≈2 | phase at t≈10 | amplitude at t≈10 | L2 at t≈10 | CPU throughput at t≈10 |
  | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
  | .04 | 6,328 | 12,394 | .0100807 | -.191 rad | -.959 rad | -.00461 | .426 | 24.5 sim s/wall s |
  | .02 | 25,218 | 49,892 | .0050806 | -.0476 rad | -.238 rad | -.000114 | .0284 | 3.05 sim s/wall s |

  Halving the temporal step changed long-time phase only from -.935 to -.959 rad
  at h=.04 and -.232 to -.238 rad at h=.02, identifying spatial dispersion as the
  dominant error. The h=.02 P1 mesh is a useful baseline, but still has roughly
  13.6° phase error after five crossings; higher-order mass treatment should be
  compared at equal error rather than adding a nominal degree knob.
- Core tests cover symmetry/nullspace, stationary constants, 2,000-step discrete
  energy conservation, damping, spatial convergence, and malformed inputs. The
  native GPU check exercises shader compilation, storage layouts, both time levels,
  and readback. All **57 tests**, formatting, Clippy with warnings denied, native
  release compilation, and release Trunk packaging pass. WASM packaging used
  `NO_COLOR=true trunk build --release --dist /private/tmp/funfern-wave-solver-dist`.
  Interactive browser verification remains deferred as requested.
- User feedback retained: local mesh repair still falls back often for small-ish
  moves and is fragile. This solver change does not alter that repair policy.

## 2026-09-09 — Bounded mesh reuse across control edits

- Brought the first geometry-only part of milestone 5 forward. `MeshUpdateJob`
  imports the displayed mesh and its committed geometry, moves affected boundary
  vertices using spline parameters, extends displacement through a fixed local
  region, and repairs using constrained flips, refinement, and conservative
  interior coarsening. Distant positions and elements remain exactly unchanged.
  The repair region cannot grow during flips/refinement: exceeding it falls back.
- Motion >2h, a large affected fraction, obstacle/knot changes, invalid motion,
  exhausted repair budgets, or failed quality checks trigger full construction.
  Resolution changes request a fresh mesh. Old meshes are immutable shared
  snapshots and stay visible during both local repair and fallback, including
  when the candidate build fails. Superseded jobs use the committed scene/mesh
  pair, not the previous in-flight scene. Geometry/history are unaffected.
- Reuse reporting compares original vertex identities and exact coordinates
  before compaction, separating unchanged geometry from connectivity alone.
  Interior edge collapse preserves the link condition and quality bounds; a
  0.35h collapse threshold is separated from the 1.05h refinement threshold.
  Tests exercise actual removal and compaction, not only eligibility checks.
- UI now distinguishes request-to-ready wall time, accumulated active meshing,
  time outside slices, and maximum mesh slice. Completed-edit fallback frequency,
  moved/inserted/collapsed vertices, and exact preserved-element fraction are
  displayed. Request time begins after editor acceptance; the native benchmark
  also logs from the edit itself, including editor validation.
- Added `cargo run -p funfern-app --release --locked -- --mesh-edit-benchmark`:
  the real native Bevy/egui app, eight obstacles, h≤0.02, visible fine overlay,
  three coordinate deltas (+.005,0), (0,+.005), (-.005,-.005), then automatic exit.
  It uses a temporary scene without file I/O and disables interactive editor
  input during the scripted run. A strict target-scene check prevents reporting
  stale completion statistics. An initial diagnostic run exposed that missing
  benchmark guard; its stale second-edit result was discarded.
- Corrected native release run: Rust 1.96.0, Apple M1 Max / Metal, 1280×800 window.
  Initial mesh: 46,147 triangles, **12,245.47 ms request-to-ready**, 2,970.93 ms
  active meshing, 9,274.54 ms outside mesh slices. This reproduces the user's
  roughly 12-second native observation, rather than attributing it to a browser.

  | Edit | Edit → ready | Request → ready | Active mesh | Outside slices | Max slice | Exactly unchanged |
  | --- | ---: | ---: | ---: | ---: | ---: | ---: |
  | 1 | 985.88 ms | 708.56 ms | 110.17 ms | 598.39 ms | 8.12 ms | 45,527 / 46,147 |
  | 2 | 995.04 ms | 715.45 ms | 109.49 ms | 605.96 ms | 7.36 ms | 45,524 / 46,147 |
  | 3 | 969.16 ms | 689.42 ms | 108.53 ms | 580.89 ms | 8.50 ms | 45,524 / 46,149 |

- All three edits used local repair, **0/3 fallbacks**, and preserved about 98.6%
  of previous elements exactly; connectivity-only preservation was above 99.97%.
  The overlay remained visible, with smoothed frame intervals around 13.5–13.7 ms
  at publication. These are three specific small edits, not a latency guarantee
  for arbitrary drags. Mesh-ready means publication to app state, not a GPU
  presentation fence. Final cleanup and quality-flag caching can exceed the soft
  2 ms slice target; the measured peak is reported rather than hidden.
- Added `mesh_edit_timing` CPU harness, with optional `--paced` (2 ms slices on
  a requested 60 Hz schedule). Continuous fine repair took ~84–86 ms in an early
  run. A separate paced run took 2.53–2.87 s wall / 284–328 ms active, with peaks
  up to 9.25 ms; coarse repairs took .43–.74 s wall. The cause of timing differences
  between harness and native app was not isolated. The harness excludes editor
  validation and rendering and must not substitute for application latency.
- Verification: **49 tests**, formatting, Clippy with warnings denied, native
  release compilation/run, and release WASM packaging pass. Tests cover repeated
  edits/reversal, exact preservation of remote triangles, independent manifold /
  Euler / area / Delaunay checks, slice-size independence, explicit fallback,
  frozen-patch coarsening guards, actual coarsening/compaction, superseded requests,
  invalid drafts/history, and retaining the displayed mesh after build failure.
  WASM command used isolated output:
  `NO_COLOR=true trunk build --release --dist /private/tmp/funfern-local-repair-dist`.
  Native run retains the existing Metal bindless fallback warning; normal exit
  also logged an unknown-window Destroyed-event warning. Browser testing remains
  deferred at the user's request.
- Remaining limits: import, connectivity indexing, final verification and
  compaction still make resumable whole-mesh passes; point location and
  encroachment scan globally. Only geometry/topology changes are bounded locally.
  Creation/deletion, knot changes and difficult motion still rebuild. Persistent
  connectivity caching, cooldown and wave-state transfer remain future work.

## 2026-09-09 — Wave-resolution meshes and honest performance baselines

- User feedback: the h≈0.16 preview mesh does not represent a useful P1 wave
  workload, and iterative reconstruction is still spatially non-local. Keep the
  local-update optimization, but do not infer wave throughput from its timings.
- Default app maximum edge is now 0.04, with 0.02 fine and 0.16 preview choices.
  Compensate for the refiner's 5% slack, scale curve tolerance as min(.0015,h/50),
  and raise application limits to 50k vertices / 100k triangles / 50k insertions.
  Resolution joins the mesh request identity, without entering document history
  or scene files. Cache per-triangle quality flags at mesh completion to avoid
  repeating trigonometric quality calculations in every overlay frame.
- Added `--wave` to the timing example. It tests h≤0.04 and h≤0.02 on 1/8/32
  obstacle scenes, prints spatial P1 DOFs, achieved edge/angle extrema, and a
  reference wavelength / h ratio, and reports failures with a failing exit code.
  Mesh construction is the only measured operation; there is no wave solve yet.
- Command: `cargo run -p funfern-core --release --example mesh_timing -- --wave --slices`.
  Rust 1.96.0, release native CPU, same previously identified Apple M1 Max host.
  Final run after our build commands completed:

  | Maximum h | Obstacles | P1 spatial DOFs | Triangles | Active meshing | Largest slice | Slices |
  | --- | ---: | ---: | ---: | ---: | ---: | ---: |
  | .04 | 1 | 6,240 | 12,150 | 236.22 ms | 2.045 ms | 118 |
  | .04 | 8 | 6,043 | 11,569 | 249.08 ms | 2.052 ms | 125 |
  | .04 | 32 | 6,295 | 11,371 | 495.68 ms | 2.056 ms | 247 |
  | .02 | 1 | 24,756 | 48,908 | 2,497.12 ms | 2.276 ms | 1,218 |
  | .02 | 8 | 23,602 | 46,179 | 2,534.28 ms | 2.267 ms | 1,239 |
  | .02 | 32 | 23,699 | 45,401 | 3,285.20 ms | 2.513 ms | 1,610 |

- The 2 ms budget implies about 2–4 seconds of completion latency for h=.04
  and 20–27 seconds for h=.02 at one slice per 60 Hz frame, before other work.
  These are inferred latencies, not measured browser frame rates. An earlier
  run while development/build activity was ongoing reached 4.89 s total and a
  61.653 ms maximum slice; no hard deadline or realtime claim is made. Overlay,
  operator assembly, GPU stepping, and render cost are excluded from this table.
- Reference wavelength .4 means five wavelengths across the box and 10/20
  maximum-edge lengths per wavelength. This is a starting convergence pair;
  shorter waves or longer propagation may need further refinement or another
  basis. Milestone 3 now explicitly requires analytic-box phase/amplitude error,
  independent temporal refinement, and throughput reported at measured error.
  Documented p=2/p=3 mass-lumped or DG alternatives if P1 proves too expensive,
  with a primary-source reference for appropriate higher-order mass treatment.
- Rebuilds still search/select globally and can propagate beyond a local region;
  every accepted edit starts a new construction. No mesh-reuse implementation
  was added in this change.
- Verification: 43 workspace tests passed, plus the expanded UI regression for
  the finer default, cached quality, resolution changes, and preservation of
  invalid drafts/history. Formatting, Clippy with warnings denied, native build,
  and release Trunk packaging to `/private/tmp/funfern-wave-resolution-dist` pass.
  All six wave-resolution benchmark cases meet their requested edge bounds.
  Interactive browser testing remains deferred by user request.

## 2026-09-09 — Local refinement and resumable mesh preparation

- Replaced whole-mesh adjacency reconstruction and quality scans during refinement
  with persistent adjacency, an ordered quality queue, and a deduplicated edge
  queue. A unit checks at most one edge or inserts one point; only changed
  triangles get new quality entries. Convexity guards constrain flips.
- Split validation, sampling, bridge visibility, ear clipping, legalization, and
  final verification into resumable phases. Bridge/ear scans yield between
  segment/vertex tests. Try the shortest bridge first, and use exact bounding-box
  rejection before expensive segment predicates. Preserve full visibility search
  as a fallback. Synchronous and sliced construction share one implementation.
- The app uses Bevy's portable clock for a soft 2 ms budget and 100,000-unit
  ceiling per frame. It displays phase, elapsed build time, accumulated work, and
  maximum meshing slice. A full job has a 50-million-unit ceiling; existing
  geometry, refinement, and mesh capacities remain enforced.
- Added a reproducible native timing example for 1, 8, and 32 obstacles using
  app settings (curve tolerance .0015, edge target .16, minimum angle 12°).
  Commands: `cargo run -p funfern-core --release --example mesh_timing` and the
  same command with `-- --slices`. Rust 1.96.0, release build, same macOS host
  previously identified as Apple M1 Max. These are CPU measurements, not browser
  frame times. Before/after single-unit profiling observations:

  | Obstacles | Before total | After total | Before longest unit | After longest unit |
  | --- | ---: | ---: | ---: | ---: |
  | 1 | 119.66 ms | 8.10 ms | 21.048 ms | 0.017 ms |
  | 8 | 818.58 ms | 41.28 ms | 495.714 ms | 0.137 ms |
  | 32 | 16,401.27 ms | 284.96 ms | 9,729.001 ms | 0.225 ms |

- Three subsequent runs of the app-style 2 ms scheduler used 6.92–8.54 ms total
  work for one obstacle, 30.87–32.90 ms for eight, and 201.81–286.29 ms for 32.
  Most maximum slices were 2.001–2.036 ms; one 32-obstacle run had a 16.734 ms
  outlier. The cause of that outlier was not established; the budget is explicitly
  soft. The 32-obstacle scene completed in 101–125 slices (about 1.7–2.1 seconds
  if served once per frame at 60 Hz, excluding other work). Profiling clocks and
  phase bookkeeping add overhead. The new flip order changes the particular
  triangulation: output counts were 734, 1,014, and 2,790 triangles, all meeting
  the same quality targets.
- All **43 tests** pass, including independent manifold/Euler/area and local
  Delaunay checks, per-unit local-work bounds, slice-size invariance, explicit
  limit completion, obsolete-job replacement, and the maximum-obstacle scene.
  Formatting, Clippy with warnings denied, native compilation, and release Trunk
  WASM build pass. No dependency or lockfile changes were needed.
- A final default-output Trunk rerun encountered truncated WASM in `dist/.stage`.
  Direct release WASM compilation passed, and packaging succeeded using
  `NO_COLOR=true trunk build --release --dist /private/tmp/funfern-mesh-verification-dist`.
  Isolating the staging output resolved the failure; a competing watched build
  was suspected but not confirmed.
- Interactive browser testing remains deferred by user request. Boundary assembly,
  point location, and domain classification retain linear capacity-bounded scans;
  a strict realtime guarantee is not claimed. Each accepted edit still constructs
  a fresh mesh; reusing unaffected regions is deferred. Next: milestone 3 waves.

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
  rendering, and WebGL fallback; `funfern-core` still has no dependencies.
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
- `cargo build -p funfern-app --locked` passes. `cargo run -p funfern-app --locked`
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
- Measure broader angle/frequency coverage before selecting further auxiliary
  radiation orders.
- IGA mass treatment and explicit timestep behavior are research tasks for the
  single-patch implementation.

## 2026-09-09 — Project setup and agreed direction

- Added milestone 0 for repository setup, documentation, and project initialization.
- Initialized a Rust workspace with an empty, dependency-free `funfern-core` library.
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
