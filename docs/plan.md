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
edits, with full rebuilding as a fallback. Static-domain waves remain next.

## 3. Static-domain GPU waves

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

**Completion:** GPU and CPU results agree to an appropriate tolerance; propagation
and fixed-domain energy behavior are sensible; long runs stay bounded. Record
performance versus mesh size and substeps on named hardware/browser versions.
Treat display rate, mesh preparation, and simulated-time throughput as separate
measurements. Include a wave-resolution convergence study; the earlier editor
preview mesh benchmarks do not establish wave-solver performance.

## 4. Transactional live editing with full remeshing

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
- Extend transactions to material coefficients and boundary-condition changes.

**Completion:** sustained dragging and parameter edits remain responsive, commits
never mix revisions, rejected candidates recover cleanly, and the transfer uses
the state at commit time rather than the state when preparation began.

## 5. Bounded incremental adaptation

**Partial implementation brought forward:** control-coordinate edits reuse the
previous mesh, move a bounded region, and repair with constrained flips,
refinement, and conservative interior coarsening. Remote elements remain fixed.
The application reports latency, exact element reuse, and fallback frequency.
Creation/deletion, knot edits, large motions and failed repair rebuild globally.
Wave-state transfer, persistent cooldown, and broader adaptation remain pending.

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

## 6. Boundary-only radiation conditions

- Add first-order outgoing conditions on the fixed outer box.
- Select and implement a higher-order auxiliary-boundary formulation in the
  Hagstrom-Warburton direction, including corner coupling and state lifecycle.
- Keep codimension-1 absorption as the intended design. A damping layer or PML is
  only an optional intermediate experiment, not the target implementation.

**Completion:** quantify pulse reflections versus angle and frequency and check
long-time behavior. Imperfect absorption is acceptable; unexplained growing modes
are not. A small first-order boundary prototype may be pulled forward if useful.

## 7. Single-patch IGA

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

Multipatch/trimmed IGA, rational weights and knot controls, material interfaces,
thin internal walls, higher-order triangles, and physical moving-boundary effects.
Time-domain FEM-BEM coupling is not planned for the initial implementation.

## Working practice

Keep the engineering log brief and useful. Update it after meaningful work with
decisions, verification, remaining issues, and next actions. Move durable changes
to the plan or architecture rather than leaving conflicting policy in log entries.
Use focused numerical tests and browser interaction checks as features arrive;
avoid placeholder tests and a generic solver framework ahead of implementation.
