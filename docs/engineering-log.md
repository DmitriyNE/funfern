# Engineering log

Low-effort working notes: what changed, what was checked, TODOs, open issues, and
next steps. Short bullets are enough; no entry is required for every tiny edit.
Keep current actions near the top and dated entries newest first. Durable decisions
belong in [architecture.md](architecture.md) and milestone scope in [plan.md](plan.md).

## Current TODOs

- [ ] Implement the higher-order auxiliary radiation condition on stable outer
  sides, including corner coupling, GPU evolution, transactional state, reflection
  measurements, and long-time checks.
- [ ] Add stable interior-region/material/interface/wall semantics, piecewise
  coefficients, assignment UI, persistence, meshing labels, and solver transactions.
- [ ] Add stable selectable boundary spans and per-span conditions, then generalize
  the completed outer-side auxiliary condition to mixed exterior spans and junctions.
- [ ] Expand product spline editing with multi-selection and transforms, direct
  span selection, knot/seam controls, duplication, snapping, and safe topology
  workflows. Rational weights remain conditional on a demonstrated workflow need.
- [ ] Profile the complete geometry-edit handoff on representative full-rebuild
  and local-repair cases. The user reports that the end-to-end handoff still feels
  slow even though the small scripted transfer case is much faster.
- [ ] Replace the sharp zero initialization at newly exposed domain with a localized
  transition/blur pass. A hard jump against the retained field produces artificial
  wideband excitation when an obstacle boundary moves inward. Measure added spectral
  energy and keep established regions outside the transition band unchanged.
- [ ] Exercise the static wave solver interactively in a WebGPU browser and record
  display frame time and long-run behavior; the user deferred browser testing.
- [ ] Record browser/GPU metadata and frame-time ranges when the mesh overlay is
  next exercised interactively; the user has deferred this pass.
- [ ] Make operator assembly and transfer-map construction resumable if their
  synchronous post-mesh tail becomes visible on larger discretizations.
- [ ] Extend fragile local repair/fallback behavior, persistent connectivity, and
  cooldown in a later adaptation pass.

## 2026-09-09 — Product roadmap before IGA

- IGA now follows three product milestones: interior topology and material
  assignment, per-span boundary conditions, and expanded spline editing.
- Stable region/material/span identities are the shared foundation. Holes,
  transmitting material interfaces, and two-sided internal walls remain distinct
  semantics rather than modes inferred from loop winding.
- Higher-order radiation remains the next milestone and initially attaches its
  auxiliary state to the four stable outer-side identities. Per-span assignment
  later generalizes that condition to selectable subspans and mixed junctions.

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
  `NO_COLOR=true trunk build --release --dist /private/tmp/femfun-wave-solver-dist`.
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
- Added `cargo run -p femfun-app --release --locked -- --mesh-edit-benchmark`:
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
  `NO_COLOR=true trunk build --release --dist /private/tmp/femfun-local-repair-dist`.
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
- Command: `cargo run -p femfun-core --release --example mesh_timing -- --wave --slices`.
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
  and release Trunk packaging to `/private/tmp/femfun-wave-resolution-dist` pass.
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
  Commands: `cargo run -p femfun-core --release --example mesh_timing` and the
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
  `NO_COLOR=true trunk build --release --dist /private/tmp/femfun-mesh-verification-dist`.
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
