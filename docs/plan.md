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
conditions. Older loops migrate to reflecting spans. Multi-region coordinate edits currently take the full-mesh path; extending
bounded repair across region topology remains adaptation work.

Open dividers stay staged until both ends attach to an outer edge or another divider.
Curve attachment inserts a shape-preserving C0 breakpoint and junction automatically.
The graph stores per-span left/right regions and explicit junctions,
meshes all transmitting branches as one conforming trace, and supports T/crossing
sector relabeling, junction dragging, and region-merge removal by selected
junction-to-junction section. Version 21 persists the graph in draft and accepted
scenes. Coordinate-edit local repair currently falls back to a full rebuild for
these graph edges.

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
The existing triangulator and solver do not consume this plan yet.

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
- Update boundary assembly, higher-order auxiliary state, thin-gap assembly,
  timestep estimation, material/source lookup, AMR indicators, local-repair
  eligibility, solution transfer, and point/line/boundary/area probes to consume
  the new labels. Preserve the existing full-remesh transaction as the safe first
  path for graph edits; local graph repair is a later optimization.
- Compile far-field contours only when the inset contour lies wholly in one
  uniform exterior face. Disable it with a precise reason when interfaces or
  varying exterior material cross that face.
- Add numerical comparisons for old supported scenes, multi-region junction
  assembly, separated same-face traces, transfer through face split/merge, AMR
  commit continuity, and finite long runs with all supported boundary laws.
- Preserve expected active area, boundary chains, side orientation,
  material-at-point results, and region connectivity from representative current
  examples as fixtures. Do not retain a production legacy-geometry adapter solely
  for these comparisons.

**Exit criterion:** the solver and probes use no coordinate-side guesses for
region or trace identity, existing examples retain their behavior, and junction
fixtures assemble and step without leaks or invalid node-membership errors.

#### Stage 3: document model, face assignments, and persistence

- Replace `Scene.obstacles`, `Scene.internal_boundaries`,
  `Scene.material_interfaces`, and `Scene.junctions` with unified curves,
  topology vertices, span behaviors, and face assignments. Keep materials in the
  library and keep region frames/sources on active-face assignments.
- Store only user-authored geometry and semantic assignments in the document.
  Store compiled faces as a revision-keyed cache, never as a second editable source
  of truth. Draft and accepted scenes each receive their own compiled snapshot.
- Preserve semantic IDs across harmless edits. When a face splits, retain the old
  region on the side containing its stable anchor and allocate a region for the
  other face; when faces merge, prefer the background/root assignment and otherwise
  require the edit command to state which assignment survives. Retarget or remove
  region probes and sources in the same undoable transaction.
- Add scene-file version 22 as a deliberate compatibility break and remove the
  versions 1 through 21 decoders. Rewrite the built-in examples and regenerate the
  checked-in example JSON in version 22. Loading an obsolete or malformed file
  reports the unsupported version and leaves the current document untouched.
- Keep one complete topology edit as one `DocumentModel` history entry. Loading
  clears history as it does now; invalid drafts remain serializable.

**Exit criterion:** version 22 round-trips exact semantic state, obsolete schemas
fail atomically with a useful message, and undo/redo restores geometry, face
assignments, probes, sources, and accepted/draft pairs together.

#### Stage 4: drawing, selection, attachment, and face UI

- Replace separate geometry tools with `Open curve` and `Closed curve`, plus
  friendly initial-configuration controls:
  - Closed + **Material region** creates a transmitting loop and assigns the chosen
    material to the new interior face.
  - Closed + **Hole** excludes the new interior face and applies the chosen boundary
    condition to its active side.
  - Open + **Divider** creates transmitting spans and previews which directed side
    receives the chosen material. It remains an invalid draft until it completes a
    partition.
  - Open + **Baffle** creates separated spans and applies the chosen initial
    condition to both sides. Thin gap remains a separate coupling choice, not a
    boundary-condition option.
- Keep Circle, Rectangle, spline, and polygonal construction as geometric shape
  choices under those two tools. The two-point spline-baffle shortcut continues to
  insert the intermediate controls needed for a straight cubic.
- Make attachment primarily a snap operation. Dragging or drawing an endpoint near
  an outer side, existing topology vertex, or curve shows a strong gold target.
  Dropping on a curve performs shape-preserving breakpoint insertion and creates or
  reuses a junction. Endpoint-to-endpoint drops merge vertices. Dragging a branch
  endpoint away detaches it when its span behavior permits a free end.
- Paint targets after curves and boundary-condition overlays so they cannot be
  obscured. Make inner and outer attachment targets use the same hit-testing path.
  Add keyboard-accessible attach/detach actions as a fallback to precise dragging.
- Preserve the current selection model: one control or topology vertex for point
  editing, multiple spans for rigid transforms and bulk law assignment, and a
  whole-curve selection command. A topology vertex has one visible handle even
  when several splines meet there.
- Derive both **Materials** and **Subdomains** overlays from the current compiled
  draft. Materials colors by assigned material; Subdomains uses categorical
  `RegionId` colors so equal-material faces remain visibly distinct. Never use the
  last committed mesh to preview draft topology.
- Use one contextual inspector below the feature list. Curve geometry controls,
  span behavior/conditions, junction attachment, face material/frame/source, and
  divider removal appear according to selection. Removing a divider previews the
  face merge and asks which material survives only when the two assignments differ.

Editor tests use synthesized egui pointer events for both attachment paths,
creation presets, selection priority, junction dragging, detach, divider removal,
bulk span conditions, overlays, Delete, Escape, and one-entry history. A short
manual native/browser pass checks touch targets and visual layering; browser
WebGPU execution remains a local smoke check rather than a CI requirement.

**Exit criterion:** users can build, attach, reconfigure, and remove the same curve
without knowing its former object class, and the draft overlay agrees with the mesh
that will be committed.

#### Stage 5: remove the parallel models and document the result

- Delete the legacy object-specific topology, validation, region flood, mesher
  branches, editor selection variants, and persistence writers after all consumers
  use the unified snapshot. Remove the obsolete file decoders as part of the same
  hard cut.
- Update architecture notes, file-format documentation, examples, help text, and
  the engineering log. Record the initial full-remesh limitation and the later
  local-repair work explicitly.
- Run formatting, Clippy with warnings denied, core/app tests, native compilation,
  release WASM compilation, scene migration fixtures, and the local browser smoke
  suite where WebGPU is available.

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

**Completion:** an interactive browser IGA wave example works on one patch, with
documented numerical and performance observations.

## Later experiments

Multipatch/trimmed IGA, higher-order triangles, and physical moving-boundary effects.
Time-domain FEM-BEM coupling is not planned for the initial implementation.

## Working practice

Keep the engineering log brief and useful. Update it after meaningful work with
decisions, verification, remaining issues, and next actions. Move durable changes
to the plan or architecture rather than leaving conflicting policy in log entries.
Use focused numerical tests and browser interaction checks as features arrive;
avoid placeholder tests and a generic solver framework ahead of implementation.
