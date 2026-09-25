# Development plan

## Intent and defaults

Build a fun, responsive browser wave toy with custom numerical methods. Triangles
establish the implementation; IGA is an intended destination, not an optional
justification for spline geometry.

Initial defaults:

- Dimensionless scalar, EM-ish waves with adjustable coefficients, damping, and
  sources. This is not initially a full Maxwell solver.
- Fixed outer computational box and editable closed internal obstacle loops.
- Periodic cubic B-splines with equal weights and nonuniform knot intervals;
  rational weights remain a possible extension.
- Geometry editing with approximate preservation of the field. Physical moving
  boundary effects are a possible later experiment.
- Reject self-intersections, obstacle contact/mergers, and nested/outside loops
  from the accepted scene while keeping invalid drafts editable after release.
- WebGPU required, desktop browser first, with native builds useful for development.
- egui for panels and controls; direct manipulation in the Bevy viewport.
- Custom math without numerical library dependencies, apart from a possible solver
  exception. Bevy/wgpu/egui infrastructure is outside that restriction.

Energy drift during editing is acceptable. Runaway numerical growth, mixed
simulation revisions, and unbounded work per frame are not.

## 0. Repository setup and project initialization

- Initialize a Rust workspace and a dependency-free numerical core crate.
- Write the README, this plan, and initial architecture notes.
- Establish `docs/engineering-log.md` for low-effort dated notes, TODOs, plans,
  experiment results, and open issues.
- Add basic formatting/ignore configuration and reproducible dependency locking.

**Completion:** the workspace passes formatting and compiler/lint checks, and the
documentation captures the agreed scope and next steps. App initialization and
browser execution belong to milestone 1.

## 1. Browser application and spline editor

Implementation is present. Native startup and automated tests pass; real-browser
interaction and performance checks remain pending. See [browser checks](browser-checks.md).

- Add the Bevy application and `bevy_egui` panels; choose compatible versions and
  the browser build/serve workflow, and commit the dependency lockfile updates.
- Establish a minimal browser GPU path and useful native development path.
- Implement pan/zoom, selection, control-point dragging, point insertion/removal,
  and obstacle creation/deletion.
- Implement periodic cubic spline evaluation and display sampling in our core.
- Distinguish proposed curves from accepted simulation geometry. Before there is
  a mesher, geometric validation supplies the acceptance boundary.
- Keep editor pointer/keyboard input separate from egui panel interaction.
- Add scene save/load and space for simulation controls; unavailable actions must
  be visibly unavailable until implemented.

**Completion:** several spline loops can be edited smoothly in a WebGPU browser,
invalid geometry is detected, scenes round-trip, and egui interaction does not
accidentally manipulate geometry underneath.

## 2. Custom triangular mesher

Implemented with automated topology/quality coverage. The user has deferred a
fresh interactive browser pass for the mesh overlay.

- Implement robust geometric predicates, constrained triangulation, domain
  classification, and bounded quality refinement.
- Preserve boundary and region identities independently of temporary mesh indices.
- Start with complete mesh construction, including concave domains and holes.
- Add mesh overlays and invalid/poor-element diagnostics to the editor.
- Establish mesh-size and geometry-resolution limits and explicit failure results.

**Completion:** representative domains produce valid meshes within configured
limits; difficult or invalid inputs fail without disrupting the editor. Predicate
and topology tests include degeneracies and near-degeneracies.

**Performance follow-up implemented:** persistent adjacency, a local edge queue,
and an ordered quality queue replace global refinement sweeps. Topology searches
and verification are resumable, and the app targets 2 ms of meshing per frame.
The engineering log records native timings. The first geometry-only part of
milestone 5 has also been brought forward: bounded local repair for coordinate
edits, with full rebuilding as a fallback.

## 3. Static-domain GPU waves

The P1 baseline and enriched-quadratic comparison are recorded. The selected
seven-node mass-lumped quadratic is integrated with GPU evolution, field display,
and live-edit state transfer; its native Metal results agree with the f64 reference.
A fresh interactive browser performance and long-run pass remains deferred at the
user's request.

- Implement linear triangular FEM, lumped mass, second-order explicit stepping,
  and conservative timestep selection. Start with reflecting boundaries.
- Use GPU gather operations and keep evolving state resident on the GPU.
- Retain a small CPU reference path for numerical comparison.
- Render the field; add pulse placement, continuous sources, run/pause, reset,
  single-step, simulation speed, and basic diagnostics.
- Expose mesh size, timestep, substeps, simulation speed, and energy diagnostics.
- Establish a wavelength-based P1 baseline before claiming solver performance:
  use a reference wavelength of 0.4 in the width-2 box, with maximum edges 0.04
  and 0.02 (10 and 20 edge lengths per wavelength), then refine further if error
  requires it. These are starting resolutions, not accuracy guarantees. For
  broadband sources, document the frequency cutoff used for shortest wavelength.
- Measure spatial dispersion against analytic reflecting-box modes over one and
  five crossing times. Reduce the timestep independently to separate temporal
  error from spatial error. Report phase/amplitude error, DOFs, memory, stable
  timestep, and achieved simulated time per wall-clock second together.
- If P1 refinement cannot meet the measured accuracy/throughput target, compare
  modest-order triangular elements (p=2 or p=3) with an explicitly chosen mass
  treatment at equal error. Enriched mass-lumped triangles or an element-local
  DG mass inverse are candidates; a polynomial-degree knob alone is insufficient.
  The p=2 enriched mass-lumped comparison is complete and substantially improves
  phase error at lower DOF than the fine P1 baseline. GPU evolution, field display,
  and transaction transfer now use it at parent h=0.08 by default.

**Completion:** GPU and CPU results agree to an appropriate tolerance; propagation
and fixed-domain energy behavior are sensible; long runs stay bounded. Record
performance versus mesh size and substeps on named hardware/browser versions.
Treat display rate, mesh preparation, and simulated-time throughput as separate
measurements. Include a wave-resolution convergence study; the earlier editor
preview mesh benchmarks do not establish wave-solver performance.

## 4. Transactional live editing with full remeshing

Core geometry-edit transactions and GPU transfer are implemented. Candidate mesh
construction is resumable; operator and transfer-map preparation still form a
short synchronous tail and should be made resumable if larger discretizations make
it visible. Boundary-condition transactions reuse the current mesh and preserve
the live field. Material-coefficient transactions also reuse the current mesh and
preserve the live field.

- Prepare candidate geometry, mesh, operators, boundary state policy, transfer map,
  and timestep while continuing to simulate on the accepted revision.
- Apply the transfer to the latest state at commit, between complete timesteps.
- Coalesce pointer updates into the latest request; maintain at most one active
  candidate. Let useful intermediate builds complete rather than restarting on
  every pointer event.
- Bound preparation work across frames. A monolithic synchronous rebuild does not
  satisfy the responsiveness requirement; use resumable work or a worker.
- Validate candidates and preserve the accepted simulation when preparation fails.
- Define initialization for newly exposed regions and treatment of boundary data.
- Boundary-condition and material-coefficient transactions are complete.

**Completion:** sustained dragging and parameter edits remain responsive, commits
never mix revisions, rejected candidates recover cleanly, and the transfer uses
the state at commit time rather than the state when preparation began.

## 5. Bounded incremental adaptation

**Spatial-field foundation implemented:** control-coordinate edits reuse the
previous mesh, move a bounded region, and repair with constrained flips,
refinement, and conservative interior coarsening. A separate resumable adaptation
transaction now refines and coarsens bulk and constrained boundaries against an
immutable spatial edge-size field. It preserves vertex lineage, applies generation
cooldown, distinguishes mesh identity from geometry identity, and carries the live
quadratic field through the normal atomic GPU handoff. The internal native check
moves the target and requires refinement, coarsening, and zero exposed transfer
nodes. A resumable solution indicator supplies recovery, strong cell, flux-jump,
and physical-boundary residuals plus active-source wavelength limits. It samples
spatial material profiles, includes variable-stiffness divergence, and drives an
optional target-field overlay.

- Implement local mesh motion, retriangulation, refinement, and coarsening, keeping
  full remeshing as a fallback.
- Limit vertex/element counts, repair work per update, and substeps per frame.
- Enforce admissible element quality/scale, with explicit policies when geometry
  cannot be represented within the budget.
- Preserve unaffected regions. Add hysteresis and cooldown to avoid adaptation
  oscillation.
- Start with geometry/quality indicators. Add user wavelength targets and then
  investigate solution-driven indicators.

**Completion:** local edits normally preserve most of the mesh, adaptation advances
across frames without starvation, and challenging edits respect the budgets.

## 6. Higher-order boundary radiation

The first higher-order condition is implemented. Reflecting, first-order outgoing,
and second-order Engquist-Majda auxiliary modes are selectable at runtime. The
second-order mode retains explicit lumped stepping, couples the two incident sides
through shared corner nodes, and transactionally preserves its boundary memory
through geometry edits. CPU and GPU evolution, state handoff, reflection, and
long-time stability have automated checks.

- Extend the reflection benchmark across more angles and frequencies.
- Add higher auxiliary orders only after comparing their cost and stability with
  the implemented second-order condition; use a symmetric formulation compatible
  with explicit mass lumping.
- Keep codimension-1 absorption as the intended design. A damping layer or PML is
  only an optional intermediate experiment, not the target implementation.

**Completion achieved for second order:** it outperforms first order over the
documented angle/frequency cases, its GPU and f64 implementations agree, corner
coupling and transactions are covered, and long runs have no unexplained growing
modes.

## 7. Interior topology and material assignment

Implemented for closed spline loops, open material dividers, and open baffles. Stable regions
and materials distinguish holes and conforming transmitting interfaces. Closed
walls and open baffles have two finite-element traces; baffle traces reconnect at
their free tips so waves can diffract around them. Nested
ownership is validated, triangles carry regions, P2e assembly is piecewise, and
coefficient edits use a same-mesh field-transfer transaction. Version 4 scene
files introduced open-spline span laws; version 5 also preserves periodic hole-span
conditions. Older loops migrate to reflecting spans. Multi-region coordinate edits
took the full-mesh path until carving arrived on 2026-09-15; the carve relabels
kept elements from the compiled topology, so region changes need no special case.

Open dividers stay staged until both ends attach to an outer edge or another divider.
Curve attachment inserts a shape-preserving C0 breakpoint and junction automatically.
The graph stores per-span left/right regions and explicit junctions,
meshes all transmitting branches as one conforming trace, and supports T/crossing
sector relabeling, junction dragging, and region-merge removal by selected
junction-to-junction section. Version 21 persists the graph in draft and accepted
scenes. Since 2026-09-15 coordinate and graph edits repair the mesh by carving
(see the engineering log); only the outer rectangle and the resolution rebuild.

- Give regions, materials, interfaces, and internal walls stable semantic IDs
  independent of mesh vertices and element indices. Keep holes, transmitting
  material interfaces, and two-sided walls distinct.
- Let closed interior loops create material regions without removing their
  interiors. Replace blanket nested-loop rejection with topology-aware containment
  validation and explicit region ownership.
- Store per-material mass density, stiffness, and volume damping. Label mesh
  elements and interface edges from geometry rather than rediscovering regions
  from coordinates after every rebuild.
- Assemble piecewise P2e operators with conforming transmission across material
  interfaces. A true internal wall must separate the two traces; a constrained
  mesh edge alone does not create that separation.
- Represent a free-ended internal boundary as a clamped nonuniform cubic spline.
  Recover it as a constrained mesh chain, duplicate both faces away from shared
  tips, and use mesh-path source stencils so excitation cannot jump through it.
- Add viewport region selection, material creation/assignment, clear material
  colors and legends, and versioned scene persistence with migration from version 1.
- Treat coefficient-only edits as same-mesh simulation transactions. Topology
  changes use the normal remesh and field-transfer path.

**Completion achieved:** nested material inclusions and internal boundaries mesh with stable
labels, piecewise coefficients reach both CPU and GPU solvers, assignments survive
save/load and undo/redo, and coefficient edits preserve the running field without
remeshing.

### Unified curve and face topology refactor

The first open-divider implementation exposed a structural split between closed
`Obstacle` loops, open `InternalBoundary` baffles, and `MaterialInterface` graph
edges. It prevents an open curve from changing role, prevents a divider from
attaching to a legacy closed interface, duplicates curve editing and hit testing,
and leaves the editor overlay and mesher with different notions of a region. The
replacement is one planar arrangement of curves and topology vertices.

The target model has four independent layers:

- A curve is a stable `CurveId`, an open or closed cubic spline, stable logical
  spans, and optional topology-vertex references at spline breakpoints. Open and
  periodic splines retain their existing exact insertion, continuity, and editing
  operations; the surrounding object type no longer determines which operations
  are available.
- A topology vertex is free, interior, or constrained to an outer side and a
  normalized side fraction. Every incident curve breakpoint references the same
  vertex. The vertex position is authoritative, so moving a junction updates all
  incident splines and resizing the rectangle recomputes every attached endpoint.
- The topology compiler derives directed half-edges, ordered sectors, boundary
  cycles, and planar faces from the domain and curves. Each curve side points to a
  derived face. The two sides of a free-ended baffle may deliberately point to the
  same face while retaining distinct finite-element traces.
- Span behavior is separate from curve geometry. A span either transmits with one
  shared trace, separates its two traces with independent per-side boundary
  conditions, or separates them with an explicit coupling such as the thin-gap
  law. An active face owns a stable `RegionId`, material/frame assignment, and
  region source. The exterior and hole interiors are excluded faces.

`Hole`, `material region`, `divider`, and `baffle` remain useful drawing presets,
but are not stored geometry classes. The rectangular outer domain remains a
special editable primitive for this refactor, while its four sides participate in
the same compiled face adjacency and boundary-law representation. General outer
curves are a later slice.

#### Stage 1: topology kernel and immutable snapshots

- Introduce `CurveId`, `CurveSpanId`, `TopologyVertexId`, and `FaceId`, plus unified
  open/closed curve and span-behavior types in `funfern-core`. Keep the types free
  of Bevy, egui, and serde.
- Add exact spline operations needed by topology edits: cut a periodic spline at a
  breakpoint, join compatible open pieces, insert a breakpoint with the required
  knot multiplicity without changing shape, and propagate stable span identities
  through insertion, deletion, split, and merge.
- Build a revisioned, resumable arrangement compiler. It adaptively samples curves
  at the fixed validation tolerance, uses exact orientation predicates for
  intersections, orders incident arms around vertices, traces all face cycles,
  and exposes face lookup for a point and directed adjacency for every span.
- Treat a proper crossing of transmitting spans as an inserted junction. Reject
  tangencies, overlaps, near-contact ambiguity, and crossings involving separated
  or coupled traces until the user resolves them explicitly. Bound subdivision,
  intersection work, vertices, faces, and total compiler work.
- Define draft validity in topology terms. A transmitting open curve is incomplete
  until its endpoints participate in closed face cycles. A separated curve may
  have zero, one, or two free ends. Coupled spans require two active sides. Invalid
  or unfinished graphs remain editable and never replace the accepted snapshot.
- Return structured errors with curve/span/vertex IDs and locations so the UI can
  highlight the exact cause rather than report a generic invalid scene.

The kernel test matrix covers an empty rectangle, nested closed loops, a divider
between outer sides, a branch into a closed interface, T and X junctions, a
same-face free baffle, a one-ended baffle, mixed transmitting/separated sectors,
hole exclusion, crossings, tangencies, overlaps, exhausted work, and domain
resize with attached vertices. Face cycles, orientation, adjacency, and point
lookup are checked without a browser.

**Exit criterion:** the compiler deterministically produces the expected faces and
span adjacency for these fixtures, and stale jobs cannot publish their result.

#### Stage 2: mesher, solver, AMR, transfer, and probes

In progress. The topology snapshot now derives snapshot-local trace-vertex
equivalence from the ordered sectors at every arrangement vertex. Transmitting
spans union the sectors on their two sides, separated spans retain distinct
traces, and a free baffle tip reconnects because it has one surrounding sector.
`TopologyMeshPlan` validates total active/excluded face assignment and projects
compiled face cycles and oriented curve sides into region-labelled trace cycles.
The full-rebuild triangulator now consumes that plan directly, preserving stable
curve/span labels and snapshot trace identities through holes, transmitting
interfaces, free and attached baffles, outer dividers, and junctions. Quadratic
solution transfer now follows unified curve sides and junction-sector traces;
transmitting face split/merge handoffs use geometric overlap instead of requiring
the same region ID on both plans. The app, mesh-repair path, and probes still use
the legacy scene path. The quadratic operator now also assembles directly from the
plan: it evaluates each triangle's assigned region, resolves outer and curve-side
conditions from stable labels, supports paired thin-gap traces, and accepts
multi-region junction nodes.
Separated T-junctions are recovered only after all incident arms are installed, so
the mesher emits the same sector equivalence as the topology compiler; mixed
transmitting/separated junctions retain their conforming paths.
Law-only plan changes reuse the existing mesh. The application cutover waits for
the remaining numerical consumers so one transaction cannot mix label models.
GPU upload no longer caps a node at two incident regions: host-filtered source
weights plus a per-node source-membership flag support arbitrary junction degree
without increasing the portable WebGPU binding count. Unified separated traces
also participate in the source's mesh-path distance calculation. Volume sources
now compile from active plan regions and the shared material library, retain their
resumable snapshot semantics, and use sparse GPU channel/weight records so every
driven face at a multi-region junction contributes without a two-channel cap. The
solution-error indicator now samples topology-assigned materials, treats
transmitting curve labels as interior flux jumps, and resolves separated boundary
laws and thin-gap pairs directly from the plan. Its adaptive size field remains
keyed by the mesh's explicit `RegionId` labels.

Ordinary probe stencils now have topology entry points as well. Point and line
samples reject ambiguous curve traces, disk and region integrals use active face
assignments and topology material evaluation, and boundary samples validate their
curve/span/side/parameter/region target against the plan before selecting an
adjacent element. Application-level boundary-path metadata and far-field
upload remain for the atomic application cutover. The core far-field compiler now
derives one exterior face from the topology plan, checks compiled curve clearance,
and rejects spatial, anisotropic, lossy, or driven exterior media before producing
its rectangular Huygens stencils.

##### Topology-aware fixed-geometry AMR slice (implemented)

This slice migrates solution-driven refinement and coarsening on an unchanged
`TopologyMeshPlan`. It does not migrate coordinate-edit repair; that arrived
separately as carving (`TopologyMeshUpdateAction::Repair`), which takes the
adapted mesh as its input and keeps the refinement outside the carved band. Only
a changed outer rectangle still goes through `FullRebuild`. The AMR transaction
itself stays local: its input geometry, face graph, trace equivalence, and
boundary sampling are immutable for the lifetime of the job.

**Geometry contract and preflight**

- Give `MeshAdaptationJob` the same dual-input shape as the operator, source, and
  indicator jobs: retain `new` for legacy tests and add `new_topology`, which owns a
  cloned plan. Keep one refinement/coarsening engine and isolate differences behind
  geometry-contract helpers.
- Build a compact topology adaptation index from the plan: active regions; canonical
  physical chains; sampled parameter intervals; expected one- or two-element
  adjacency; planned endpoint trace IDs and points; and opposite-side keys for
  separated curves. A transmitting span has one physical chain keyed by
  `(curve, span)` even though the plan describes both adjacent faces. A separated
  span has independent left and right chains keyed by `(curve, span, side)`.
- Validate the source mesh against that index before changing it. Every triangle
  region must be active. Every constrained edge must lie inside exactly one planned
  interval with the expected label, parameter direction, geometry, adjacency, and
  incident region. Every planned interval must be covered once without gaps or
  overlaps. Every planned `TraceVertexId` must occur once at its exact plan point
  with the expected incident region sectors. Missing separated partners, duplicate
  traces, unknown spans, and a transmitting edge represented as two cracks are hard
  input errors.
- Preserve `MeshVertex::trace` while importing and compacting. It is independent of
  AMR vertex lineage: lineage tracks discretization history, while a trace ID tracks
  a topology sector at a sampled endpoint or junction.

**Legal refinement and coarsening**

- Treat each `PlannedBoundaryEdge` as an immutable straight geometric atom. Boundary
  refinement may insert a no-trace vertex only inside that atom, with its position
  obtained by parameter interpolation on the atom. This preserves the full mesher's
  sampled spline exactly instead of re-evaluating a mutable curve or introducing a
  second approximation.
- Refine outer and transmitting constraints once. Refine a separated constraint by
  locating its opposite side through stable curve/span and normalized parameter
  interval, then split both traces at the same parameter in one operation. Physical
  coordinate equality is an asserted consequence, never the lookup key.
- Pin every vertex carrying a `TraceVertexId`, every outer corner, and every planned
  interval endpoint. These include spline sampling vertices, span boundaries, free
  tips, attached endpoints, and all junction sectors. Only AMR-created boundary
  vertices inside one planned interval may be removed.
- Permit transmitting-boundary coarsening within one atom when the ordinary
  orientation, target-length, region, and quality tests pass. Coarsen separated
  sides as one paired operation and require both sides to choose the same retained
  parameter. Never merge across plan atoms or spans, change a trace equivalence
  class, remove a face's last element, or collapse an interior vertex into a
  different region or constrained sector.
- Classify constrained vertices from both boundary-edge incidence and trace IDs;
  topology junction endpoints often intentionally have `boundary == None`. Extend
  the existing relaxed angle test only to triangles touching a separated trace, not
  to ordinary transmitting material interfaces.
- Share the boundary-adjacency rule used by full topology meshing and AMR
  verification: outer and separated edges have one incident triangle; transmitting
  topology curves have two. Edge legalization remains unable to cross any
  constrained chain and must retain equal region IDs across every unconstrained
  edge it flips.

**Publication and failure behavior**

- Add a topology-aware final verification pass over triangles, constrained chains,
  separated pairs, trace vertices, and active regions. It must prove that refinement
  merely subdivided planned atoms and coarsening merely removed AMR-created
  subdivisions. Preserve the source geometry revision, publish a fresh mesh
  revision, and remap lineage, cooldown, boundary metadata, and trace IDs together.
- Keep construction and mesh scans resumable under the existing work budget. A
  topology-change or capacity limit may publish a verified partial adaptation as it
  does today; a work-limit or contract error publishes nothing and leaves the source
  mesh and adaptation state untouched.
- The job owns its plan snapshot. The later application cutover must still reject a
  completed job whose source mesh/settings token is stale before operator assembly
  or field handoff.

**Tests and cutover gates**

- Retain the legacy AMR suite and add slice-size determinism for the topology path.
  Compare legacy and topology results for a one-face rectangle, including repeated
  refine/coarsen generations and cooldown lineage.
- Exercise region-dependent targets on a transmitting divider and a closed material
  interface. Verify both incident regions remain connected and their shared chain
  retains two-element adjacency.
- Refine and coarsen a free baffle, an outer-attached baffle, a fully separated
  T-junction, a mixed transmitting/separated junction, and a paired thin gap. Assert
  stable trace-sector identity, coincident opposite chains, preserved free-tip
  reconnection, and pinned junction vertices.
- Cover malformed meshes explicitly: missing or misoriented separated partners,
  a lost/duplicated trace ID, a boundary edge spanning two planned atoms, an unknown
  region or span, and incorrect one-/two-sided adjacency. Each must fail before
  publication without mutating the source or state.
- Run an end-to-end core regression from topology indicator to AMR, topology
  operator assembly, quadratic transfer, and several finite wave steps. With an
  unchanged topology plan, transfer must report no exposed nodes and preserve a
  quadratic field to tolerance.
- Completion requires formatting, workspace tests, Clippy with warnings denied,
  native release compilation, and a release Trunk/WebGPU build. The application
  continues using legacy AMR until the atomic document cutover; this slice proves
  the topology numerical path directly without temporarily mixing label models.

- Make meshing consume a completed topology snapshot instead of independently
  rediscovering loop nesting and open-divider regions. Triangulate each active face
  and recover each logical span as a constrained chain with `CurveSpanId`, side,
  and adjacent `RegionId` labels.
- Derive degree-of-freedom equivalence from span behavior. Transmitting spans share
  nodes across material jumps; separated spans duplicate their traces; coupled
  spans use the duplicated traces plus the coupling operator. At a junction,
  sector connectivity determines which coincident nodes are shared.
- Replace the current assumption that a wave node belongs to at most two regions
  with topology-supplied trace/sector memberships. This is required for three or
  more regions meeting at a point.
- Update local-repair eligibility and application-level probe metadata to consume
  the new labels. Core point/line/boundary/area stencils already use the topology
  plan. Preserve the existing full-remesh transaction as the safe first path for
  graph edits; local graph repair is a later optimization.
- Compile far-field contours only when the inset contour lies wholly in one
  uniform exterior face. Disable it with a precise reason when interfaces or
  varying exterior material cross that face. This is implemented in the core
  topology path; application upload switches during the atomic cutover.
- Add numerical comparisons for old supported scenes, multi-region junction
  assembly, separated same-face traces, transfer through face split/merge, AMR
  commit continuity, and finite long runs with all supported boundary laws.
- Preserve expected active area, boundary chains, side orientation,
  material-at-point results, and region connectivity from representative current
  examples as fixtures. Do not retain a production legacy-geometry adapter solely
  for these comparisons.
- Keep exact mesh reuse for unchanged topology plans and skip remeshing for
  boundary-law-only changes. Coordinate motion and graph edits repair the mesh
  by carving the changed atoms' band (done 2026-09-15); no spline evaluation is
  needed because the plan's atoms are straight.

**Exit criterion:** the solver and probes use no coordinate-side guesses for
region or trace identity, existing examples retain their behavior, and junction
fixtures assemble and step without leaks or invalid node-membership errors.

#### Stage 3: document model, face assignments, and persistence

In progress. The dependency-free authored `TopologyScene`, explicit active or
excluded face dispositions, oriented boundary anchors, structured assignment
errors, same-face separator endpoint check, synchronous convenience compiler, and
resumable scene compiler are implemented in the core. The headless application
document now creates closed curves, free or attached baffles, and attached
separators; it remaps split-span anchors and boundary probes and resolves curve
removal ownership atomically. The strict version-22 codec and all eight built-in
examples now use this document directly, including sources, probes, materials,
frames, boundary laws, far-field settings, and presentation state. Loading reseeds
stable IDs and begins with empty history. Remaining spline topology edits and the
live atomic UI/runtime switch still await the cut.

The application cutover needs a stable authored face reference. `FaceId` is an
ordinal in one compiled snapshot, so persisting it would make material ownership
change when an unrelated edit changes face traversal order. A point seed is also
unsafe because a moving divider can pass over it. Persist an oriented boundary
anchor instead:

```text
TopologyScene
  geometry: TopologyGeometry
  physics: PhysicsModel
  materials: [Material]
  regions: [Region]
  face_assignments: [AuthoredFaceAssignment]
  volume_sources: [VolumeSource]
  outer_boundaries: OuterBoundaryConditions

AuthoredFaceAssignment
  anchor: Outer(side, fraction)
        | Curve(curve, span, side, parameter)
  region: RegionId | Excluded
```

The parameter is inside the referenced logical span. It disambiguates faces when
a crossing divides one span into several compiled atoms. `resolve_face_anchor`
maps the anchor to a snapshot-local `FaceId`; derived `FaceRegionAssignment`s then
feed `TopologyMeshPlan`. Anchors at an endpoint, on an ambiguous crossing, on a
deleted span, or on the wrong side of an excluded face fail with an ID-bearing
validation issue. Every active region resolves to exactly one bounded face, no two
assignments may resolve to the same face, and every valid bounded face has exactly
one active or excluded assignment. A face omitted from the assignment list is a
persistent invalid-draft state until the user chooses a material or hole. The
exterior is never assignable.

- Replace `Scene.obstacles`, `Scene.internal_boundaries`,
  `Scene.material_interfaces`, and `Scene.junctions` in the application document
  with `TopologyScene`. Materials remain a library; region material/frame state
  and volume sources keep their stable `RegionId` references.
- Store only authored geometry, anchors, laws, probes, sources, and presentation
  settings. Keep the compiled topology snapshot and mesh plan in a revision-keyed
  cache, never in serialization or undo history. Draft and accepted scenes each
  have independent cache entries.
- Centralize topology mutations as editor commands. Insertion, control removal,
  split/join, curve reversal, junction materialization, attachment, detachment, and
  deletion must return an ID-remap record. Apply that record to region anchors,
  selected spans, and boundary probes before the command is committed. Coordinate
  movement does not need a remap.
- Normalize a proper transmitting crossing into authored C0 breakpoints and one
  stable topology vertex before accepting or persisting it. A derived crossing
  must not become an unselectable, unstable junction in the editor.
- When a face splits, its existing anchor determines which daughter retains the
  old `RegionId`. A creation command either gives the other daughter a new region
  and anchor or leaves it visibly unassigned. When faces merge and assignments
  differ, the delete command must carry the surviving assignment; it must never
  select one by traversal order. Deleting a hole or separated baffle is
  unambiguous.
- Keep region probes and sources with the surviving `RegionId`. Deleting their
  region removes or retargets them as part of the same visible, undoable command;
  never leave a probe silently sampling a different face.
- Replace legacy boundary-probe targets with stable topology paths. An outer target
  stores ordered outer sides. A curve target stores `CurveId`, an ordered bounded
  list of `CurveSpanId`s, `CurveTraceSide`, direction, and sampling preset. Knot
  insertion expands the path, reversal reverses it and swaps orientation, and
  removal is rejected if the remaining path is no longer contiguous.
- [x] Add scene-file version 22 as a deliberate compatibility break with no
  versions 1 through 21 decoder. Rewrite the built-in examples directly in the
  version-22 model.
- [ ] Switch shared-link and recovery payloads to version 22 during the atomic live
  cut. Ignore obsolete local storage with one clear notice; loading an obsolete or
  malformed file must leave the current document untouched.
- [x] Keep one complete topology edit as one `DocumentModel` history entry. Loading
  clears history as it does now. Invalid drafts, including unresolved anchors and
  incomplete transmitting dividers, remain serializable and undoable.

Before changing the live application, add document-level tests for anchor
resolution across coordinate movement, curve sampling changes, crossings, knot
insertion/removal, split/join, and reversal. Cover face split/merge ownership,
dependent probe/source handling, exact version-22 round trips, invalid-draft round
trips, stale compilation rejection, and atomic failure of old or malformed files.

**Exit criterion:** version 22 round-trips exact semantic state, obsolete schemas
fail atomically with a useful message, and undo/redo restores geometry, face
assignments, probes, sources, and accepted/draft pairs together.

#### Stage 4: cooperative rebuild and atomic runtime contract

In progress. `TopologyMeshingJob` now cooperatively bridges cycles, clips faces,
legalizes, refines, recovers separated curves, and verifies output, with exact
slice-size determinism. The application library now carries one immutable accepted
topology bundle through meshing, assembly, transfer, volume-source compilation,
ordinary probes, and far-field compilation. A coordinator retains the active state
until the caller explicitly acknowledges successful GPU upload and rejects stale or
failed candidates without publication. Source/probe/far-field-only edits reuse the
exact topology, mesh, and operator; material and boundary-law changes reuse the mesh
and build an exact transfer; coordinate and graph changes take the typed cooperative
full rebuild. The visible application still launches its legacy transaction until
Stage 5 removes its object-specific editor dependencies.

The topology mesher was originally synchronous while the live legacy mesher
yielded after a small work slice. Switching it directly would have frozen the
browser during a full rebuild, so the topology full-rebuild path must remain a
deterministic cooperative job before the application document changes over.

- Split topology meshing into bounded phases: plan import, one-face triangulation
  and refinement, separated-trace recovery, junction/slit recovery, legalization,
  verification, and publication. Yield on both primitive work and elapsed frame
  budget. The final mesh must be identical for every slice size.
- Carry one immutable accepted bundle through the complete transaction:

  ```text
  AcceptedTopology
    document_revision
    topology_revision
    authored: Arc<TopologyScene>
    snapshot: Arc<TopologySnapshot>
    plan: Arc<TopologyMeshPlan>
  ```

  Mesh, assembly, volume-source compilation, transfer, GPU upload, probes, AMR,
  indicators, overlays, and far-field compilation must all reference this bundle
  or its token. No candidate may combine an old scene with a newly compiled plan.
- Keep the last accepted bundle and GPU state running while draft compilation,
  meshing, assembly, transfer, or upload is pending or fails. Publish the complete
  candidate atomically only after GPU resources are ready. Preserve requested
  run/pause state, simulation time, and compatible probe histories across a normal
  handoff; example load and explicit reset still request a fresh field.
- Use `topology_mesh_update_action` for transaction classification. Material,
  formula, source, boundary-law, and presentation changes reuse geometry. Curve
  coordinate and graph changes took the verified full topology rebuild initially,
  reported as a typed `CoordinateRepairDeferred` decision; that decision became
  `Repair` by carving on 2026-09-15 without a change of UI policy.
- Route production calls through `assemble_topology`, topology volume-source and
  probe compilers, `SolutionIndicatorJob::new_topology`,
  `MeshAdaptationJob::new_topology`, and topology far-field compilation. Material
  property overlays resolve the same active face assignments. Point-source region
  lookup uses the committed plan; a source exactly on an interface gets a precise
  ambiguous-placement status.
- Include document, topology, mesh, adaptation, and GPU generations in stale-result
  checks. Cancellation discards partial output without touching the active bundle.

Core tests compare topology mesh output across tiny and large work slices and
exercise cancellation at every phase. Application transaction tests cover
material-only reuse, law-only reuse, graph and coordinate rebuilds, failed draft
compilation, failed mesh or upload, AMR handoff, ordinary probes, far field, and a
stale completion arriving after a newer edit.

**Exit criterion:** a topology rebuild never monopolizes a browser frame, every
published numerical object names the same topology token, and failure at any stage
leaves the running simulation intact.

#### Stage 5: drawing, selection, attachment, and face UI

The UI stays geometry-first. **Closed curves** and **Open curves** are the two
object families; a compact purpose choice only supplies their initial face or span
configuration. Purpose does not create a separate stored object type and does not
restrict later editing.

**Draw popover**

- Keep the top-bar **+ Draw** entry. Divide the popover into two clearly labelled
  groups:
  - **Closed curve** contains Circle, Rectangle, Polygon, and Spline.
  - **Open curve** contains Polyline and Spline.
- Each group has one compact initial-purpose control next to or directly below its
  tools. Closed curves choose **Subdomain** or **Hole**. Open curves choose
  **Subdomain separator** or **BC baffle**. Present each pair as two mutually
  exclusive checkbox choices, remember the last choice per family, and do not make
  users choose it again for repeated drawing.
- A closed **Subdomain** starts transmitting and asks for its interior material. A
  **Hole** starts separated and asks for its domain-side condition. A **Subdomain
  separator** starts transmitting and asks for the new side's material. A **BC
  baffle** starts separated and asks for its initial condition. Thin gap remains a
  coupling in the contextual editor.
- A Subdomain separator can start only on the boundary of an active face. Once its
  first segment establishes which incident face it enters, only boundary points of
  that same face are valid completion targets. Outer sides, closed curves, existing
  open curves, and authored junctions participate when their relevant side bounds
  that face. At a multi-sector junction, the hovered sector and live path preview
  disambiguate the intended face.
- While drawing a separator, show valid start and completion targets in gold and
  dim invalid boundary targets. Enter or clicking the final point commits only when
  both endpoints are attached, the path lies inside the selected face, and the
  result creates two valid daughter faces without an illegal crossing or contact.
  Otherwise keep the construction gesture active with a short reason; Escape
  cancels it without changing the document. A later edit may still detach or
  invalidate an accepted separator, in which case the normal persistent-invalid-
  draft behavior applies.
- A completed separator keeps the old material on the daughter containing the old
  face anchor; the other daughter receives the selected material. This avoids
  asking the user to reason about left/right before the curve has a direction.
- Preserve the two-point spline-baffle shortcut that inserts equidistant internal
  controls for a straight cubic. Drawing stays active until explicitly dismissed
  where the current tool already has repeat placement.
- Use one attachment hit-testing path for all open curves: authored junctions,
  loose ends of other open curves, vertex-less breakpoints, then outer sides and
  any curve interior, in that precedence. Separators filter it through the
  same-face completion rule; BC baffles allow free, singly attached, or doubly
  attached endpoints. A curve-interior drop performs exact C0 breakpoint insertion
  and creates or reuses a topology vertex in one history action; a breakpoint drop
  makes that breakpoint the junction without inserting a span. A loose-end drop
  welds the curves into one — valence two is one curve, never a junction — and a
  loose end meeting its own curve's other end closes the loop. The same targets
  serve when a loose end is dragged: the weld happens on release and is one
  history action together with the drag. Render targets above geometry and law
  strokes. Keyboard-accessible **Attach endpoint** and **Detach endpoint** actions
  provide a precise fallback.

**Viewport selection and manipulation**

- Replace object-specific selection with `CurveId`/`CurveSpanId`. One ordinary
  control selects and moves only that control. An attached breakpoint selects its
  authoritative `TopologyVertexId`; dragging it moves all incident curves once.
  Dragging the control of a loose end is the weld gesture above.
- Deleting spans or curves that merge several subdomains into one highlights the
  candidates in the scene and takes a click as the survivor, independent of
  which panels are open; a deletion that would merge subdomains in more than one
  place is refused with a reason.
  Multiple selected spans remain a rigid transform selection, including the
  existing translation, scale, rotation gizmos, snapping, and marquee behavior.
- Keep handle priority over spans, Shift span toggling, command-click whole-curve
  selection, and right-drag/two-finger viewport navigation. Preserve the
  direction-dependent marquee: left-to-right fully encloses, right-to-left hits.
- A rigid transform changes nothing outside its selection. A selection holding
  only some arms of a shared junction, or one whose boundary knot is smooth, is
  therefore not rigidly movable, and the gizmo does not appear for it. If every
  incident arm is selected, transform the vertex once. Marquee selection and
  **Isolate at C0** are how the user widens a selection until it moves.
- Draw a small direction arrow on the focused curve and tint the selected trace
  side. `Left` and `Right` always mean relative to increasing curve parameter;
  bulk edits across curves apply that same coherent rule.
- Keep outer-edge/corner selection and extent editing in the same contextual area
  as curve editing. A topology vertex attached to an outer side follows a domain
  resize through its stored fraction.

**Edit panel**

- Keep one lean contextual inspector below the collapsible feature list. Do not
  reintroduce operation modes, a collapsible Transform group, or explanatory
  paragraphs. Geometry moves remain available immediately after selection.
- Order controls as: selection/coordinates, transform and gizmos, spline tools,
  attachment/topology actions, then span behavior and boundary laws. Show only
  controls that act on the current selection.
- Selected spans expose **Transmit** or **Boundary**. For a separated span with one
  active side, show a single **Domain side** law. With two active sides, show the
  Left/Right picker and highlight that side in the viewport; show coupling only
  when both traces support it. Mixed multi-selection remains editable through
  explicit common-value actions.
- A closed curve with one well-defined interior face gets a compact
  `Inside: Hole | <material>` shortcut. Full region frame, source, and library
  editing remains in **Materials**. Divider removal previews the merged face and,
  only if assignments differ, presents **Keep <material A>** and
  **Keep <material B>** before committing.
- Delete acts on the current semantic selection. Deleting a control, span path,
  curve, junction, or dependent probe is one history action and either includes a
  complete ownership choice or leaves the document unchanged with a specific
  reason. Escape restores the pre-drag document as today.

**Materials, overlays, probes, and status**

- Build Materials > **Subdomain assignment** from the current compiled draft.
  Rows show stable region/material names and swatches, never `FaceId`. Picking a
  face highlights its complete derived boundary. An unassigned face offers
  **Assign material** or **Make hole**; the latter is disabled when transmitting
  adjacency would make the mesh plan invalid.
- Keep **Library** unchanged. A region frame remains fixed unless the complete
  defining boundary undergoes one rigid/similarity transform; do not infer a new
  local frame orientation from a general junction graph.
- Derive both **Materials** and **Subdomains** overlays from the draft snapshot.
  Materials uses the assigned material colors; Subdomains uses categorical region
  colors so adjacent equal-material faces remain distinct. A compilable but
  unassigned face gets an amber hatch. Never preview draft topology using the last
  committed mesh.
- Preserve probe panel and floating readouts. Boundary probes pick the same span
  and side highlights as boundary editing. Area-region probes follow `RegionId`.
  Double-click behavior and view toggles stay unchanged.
- Use one status-bar sentence for the active phase, such as
  `Geometry invalid: divider endpoint is free`, `Topology rebuilding: tracing
  faces`, `Mesh rebuilding: recovering junctions`, or `Simulation ready`. Do not
  add a second permanent progress label to the Simulation panel.
- If geometry compilation fails, draw the authored draft in red over a subdued
  accepted reference. If geometry compiles but assignment fails, retain normal
  curves and highlight the relevant face/anchor in amber or red. The invalid draft
  remains fully selectable and editable in either case.

Editor tests use synthesized egui pointer events for inner and outer attachment,
both curve families and all four initial purposes, separator start and same-face
completion filtering, refusal of unattached separator completion, selection
priority, junction dragging and partial-arm blocking, detach, divider removal and
ownership choice, bulk span conditions, side orientation, overlay face picking,
Delete, Escape, and one-entry history.
A short manual native/browser pass checks visual layering, narrow layout, touch
targets, and gestures. Browser WebGPU execution remains a local smoke check rather
than a CI requirement.

**Exit criterion:** users can build, attach, reconfigure, and remove the same curve
without knowing its former object class, and the draft overlay agrees with the mesh
that will be committed.

#### Stage 6: atomic application switch and legacy removal

**Implemented 2026-09-14.** The production path now uses version 22 and unified
topology identities end to end. The legacy editor and codecs were removed on
2026-09-16, once the shared value codecs moved into `topology_persistence`.

- Land the new document/editor commands, cooperative mesh job, and UI structures
  behind non-production tests first. Switch document loading, validation,
  rendering, hit testing, simulation transactions, AMR, probes, far field, and
  overlays in one integration change so a frame cannot mix legacy and topology
  identities.
- Rewrite every bundled example in the new authored model and make the first
  example the startup document. Check its sources, probes, boundary laws, material
  frames, view settings, and readout state rather than comparing geometry alone.
- Delete production uses of `ObstacleId`, `InternalBoundaryId`,
  `MaterialInterfaceId`, `JunctionId`, object-specific `GeometrySpan` variants,
  legacy boundary-probe targets, and legacy mesher/operator/probe/AMR entry points.
  Keep legacy core algorithms temporarily only where they provide numerical
  equivalence fixtures; do not ship a compatibility adapter.
- Update architecture notes, file-format documentation, examples, help text, and
  the engineering log. Topology coordinate movement's initial full-rebuild
  behavior and its replacement by carving are both recorded there.
- Run formatting, Clippy with warnings denied, the complete core/app suite, native
  release compilation, release Trunk/WebGPU compilation, scene fixtures, and the
  local browser smoke suite where WebGPU is available. Record handoff times for a
  multi-region junction example and confirm that the cooperative topology job
  stays inside its frame budget.
- Use a source audit as a final gate: the application production path must contain
  no old geometry IDs and no calls to legacy scene meshing, assembly, indicator,
  adaptation, transfer, source, ordinary-probe, or far-field compilers.

**Completion:** one stored curve model, one compiled face map, and one span-law
model drive validation, rendering, meshing, simulation, probes, persistence, and
editing. Inner-loop attachment, outer-resize propagation, same-material subdomain
display, divider removal, and multi-region junction behavior all have direct
non-browser regressions.

## 8. Boundary spans and assigned conditions

Open-baffle knot spans are implemented. A selected span exposes its left and right
faces independently; each face can reflect, use driven Dirichlet/Neumann data, or
use either outgoing order. Alternatively, the pair can use one conservative
thin-gap spring. Assignments survive resampling,
knot insertion, undo/redo, and versioned persistence. Removal rejects a merge when
the two affected spans disagree. Outer sides, hole spans, and baffle spans now use
the same viewport selection and inspector workflow.

- Represent selectable logical spans independently of sampled mesh edges. Outer
  sides and spline knot spans retain stable assignments through resampling and
  ordinary control-point motion.
- Assign the existing reflecting, first-order outgoing, and second-order auxiliary conditions per span,
  with explicit defaults and visible viewport/panel labels. Add further fixed or
  prescribed conditions only with defined scalar-wave semantics.
- Keep independent baffle face laws and paired thin-gap coupling as exclusive span
  modes. Include coupling in the explicit timestep bound and treat
  frequency-dependent laws as auxiliary boundary state.
- Define how insertion, removal, seam changes, splitting, and merging inherit or
  combine span assignments. Ambiguous edits remain drafts until resolved.
- Assemble mixed boundary conditions and handle junction nodes where neighboring
  spans use different conditions.
- Generalize the completed higher-order outer-side condition to assigned exterior
  spans, preserving its endpoint/corner coupling and auxiliary-state transactions.

**Completion:** mixed conditions can be painted onto stable spans, survive all
document operations, assemble consistently on remeshed edges, and pass reflection,
junction, transaction, and long-time stability checks.

## 9. Product spline editing

The transform slice is implemented with exclusive single-control editing and
multi-span selection. Selected spans support bulk boundary assignment; complete
curves support rigid pivot-snapped dragging, numeric affine transforms, and a
viewport rotation gizmo. Duplication and exact baffle straightening retain span
assignments. Repeated knots now expose C1/C0 refinement on loops and baffles;
open baffles split and merge without losing span laws or oriented face semantics.
Contiguous and disjoint partial-span selections can be isolated at their exposed
C0 knots and transformed rigidly, including across a periodic seam. Smoothing
uses exact knot removal where possible and an explicit least-squares reshape with
a reported displacement bound otherwise. Selected loops can convert between
hole, material-interface, and closed-wall roles with explicit region ownership.

- Keep complete-curve translate/rotate/scale, duplication, snapping, and alignment
  predictable under span selection.
- Expose knot-interval and seam editing without conflating control positions with
  interpolation points. Preserve or clearly report changes to span assignments.
- Add direct span selection and the topology operations needed by interior regions,
  including explicit loop role changes and safe split/merge workflows.
- Keep every completed gesture as one history action, retain invalid drafts, and
  make dense scenes manageable through selection filtering and clearer overlays.
- Evaluate rational weights and additional continuity/corner controls after the
  product workflows identify a concrete need.

**Completion:** common geometry and assignment tasks do not require editing JSON,
all new actions round-trip and undo correctly, and viewport interactions remain
predictable on representative dense scenes.

## 10. Single-patch IGA

- Implement an untrimmed spline patch and spline solution basis, quadrature,
  operators, field evaluation, and state transfer.
- Investigate mass treatment and timestep restrictions rather than assuming the
  triangle solver's diagonal mass treatment transfers unchanged.
- Reuse the UI and transaction lifecycle, with discretization-specific numerical
  kernels where appropriate.
- Compare propagation, editing behavior, and cost against the triangular solver.
- Revisit second-order outgoing conditions on curved spans here, where the
  boundary curvature is available exactly. On the triangle solver the
  Engquist-Majda tangential term injects energy on curved spans (reproducer
  test ignored in `wave_quadratic.rs`, diagnosis in the engineering log for
  2026-09-15); until then curved faces stay reflecting or first order.

**Completion:** an interactive browser IGA wave example works on one patch, with
documented numerical and performance observations.

## Later experiments

Multipatch/trimmed IGA, higher-order triangles, and physical moving-boundary effects.
Time-domain FEM-BEM coupling is not planned for the initial implementation.

- [ ] Pulsed and gated signals. `TimeSignal` is harmonic only, so a document
  cannot hold a pulse, and the run-time pulse act is not a document feature.
  That keeps time of flight, group delay in a Klein-Gordon medium, echoes, an
  ellipse refocusing a flash and pulsed Doppler out of the gallery
  (`docs/spikes/funfern-gallery-plan.md`, "Blocked"). Add a windowed harmonic
  (start, duration, ramp) or a Gaussian burst as a signal variant, honoured by
  every consumer: point and volume sources, Dirichlet and Neumann walls and
  faces, and time drives. It is persisted, so it takes a serde default or a
  file version.

## Maintenance

- [ ] **Next after the gallery:** a scene change must stop the old field at
  once. Opening an example, New scene, loading a file or a link goes through
  `set_document` (`crates/funfern-app/src/ui/session.rs`), which replaces the
  document and requests a fresh generation, but the outgoing generation keeps
  stepping, and stays on screen, until the new one is meshed, compiled and
  admitted. On a large or law-carrying scene that is seconds of the previous
  scene still running under the new one's geometry and panels. Today this is
  deliberate: the comment there keeps the field scale because "the outgoing
  scene is still on display until its replacement is prepared". Questions to
  settle before wiring it:
  - What is shown in the gap: a cleared field over the new geometry, the
    still last frame, or nothing. Probes, the far field, the energy readouts
    and the recorder all read the active generation and need the same answer.
  - Whether the old generation is paused or dropped at once, on the CPU and
    on the device, and how that meets the handoff machinery that exists
    precisely to keep the accepted generation live while a candidate prepares
    (live edits must keep that behaviour; only a document replacement drops
    it).
  - Undo of a scene change, which also goes through `set_document`.
  - A test that a replacement leaves no step of the old generation after it,
    and a scratch-HOME app run clicking through the gallery.

- Enable fat LTO and `codegen-units = 1` for the native release build. The
  browser bundle already gets both through `scripts/trunk`, which exports
  `CARGO_PROFILE_RELEASE_LTO` and `CARGO_PROFILE_RELEASE_CODEGEN_UNITS`; that
  took 822 KB off its gzipped size. They were deliberately kept out of
  `[profile.release]` because the same profile builds the benchmarks, so turning
  them on there invalidates the timings recorded in the engineering log against
  the current codegen settings. Doing it therefore means re-baselining
  `mesh_timing`, `mesh_edit_timing`, `--mesh-edit-benchmark` and the
  `--wave-gpu-check` throughput figure in the same change, and accepting a
  slower native release build in CI.

- Narrow what each `ui` submodule touches. `crates/funfern-app/src/ui.rs` is now
  1,883 lines over 28 submodules, with every test beside the code it exercises,
  but the split relocated the coupling rather than reducing it: `Playground` in
  `ui/state.rs` carries 171 fields and nearly every method still takes `&mut
  self` over all of them. Group those fields into sub-structs — probes, AMR,
  recording, view — and narrow the methods of a module that only touches one
  group to `&mut self.probes` and the like. Do one group at a time, and only
  where the access pattern is already clean. Two smaller follow-ups belong with
  it: `crates/funfern-app/src/topology_editor.rs` is 10,282 lines and wants the
  same treatment, and the carving pass marked every moved method `pub(super)`
  without working out which are actually called from outside their module, so a
  visibility pass (make private, compile, promote only what fails) is cheap once
  the boundaries stop moving.

- Factor the built-in example scenes out of
  `crates/funfern-app/src/topology_examples.rs` into `examples/`. The gallery
  scenes are compiled in while `examples/` carries one standalone file, so the
  two can disagree with nothing to catch it.

- Decide what `experiments/material-laws-spike/` is for. It is 500 KB committed,
  160 KB of it results JSON, referenced from the spike reports in `docs/spikes/`.
  Keeping it as the record behind those reports is a fine answer; drifting into
  it is not.

## Working practice

Keep the engineering log brief and useful. Update it after meaningful work with
decisions, verification, remaining issues, and next actions. Move durable changes
to the plan or architecture rather than leaving conflicting policy in log entries.
Use focused numerical tests and browser interaction checks as features arrive;
avoid placeholder tests and a generic solver framework ahead of implementation.
