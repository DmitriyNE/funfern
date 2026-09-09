# Architecture notes

Milestone 1 implements the spline core, geometric editor validation, and Bevy/egui
application described below. Meshing, simulation transactions, GPU evolution, and
state transfer in later sections remain design directions.

## Responsibilities and dependencies

| Part | Responsibility | Intended execution |
| --- | --- | --- |
| Numerical core | Splines, predicates, meshing, assembly, transfer preparation | Rust/WASM CPU |
| Application | Input, editor, egui panels, scene lifecycle, scheduling | Bevy |
| GPU evolution | Wave steps, commit-time transfer, later auxiliary states | WGSL through Bevy/wgpu |
| Visualization | Field, accepted mesh, proposed curves, handles | Bevy rendering |

Keep our geometry and numerical code free of external math/meshing libraries. A
solver dependency can be considered if it solves a concrete problem. Framework
transitive math dependencies do not imply using them in the numerical core.

The core must not depend on Bevy ECS, egui, or GPU handles. Do not make an ECS entity
for every numerical degree of freedom. Add an app crate in milestone 1; split GPU
code further only when the implementation benefits from it.

Use egui for panels, controls, and diagnostics. Render the field and directly
manipulated geometry in the viewport. Coordinate panel input capture and the usable
viewport rectangle with the camera and editor.

## Geometry and discretization

Geometry owns curves, loops, boundary labels, and eventually material regions.
Discretization owns basis coefficients, operators, evaluation, and state transfer.
Do not expose all solution coefficients as geometric vertex values: that works for
linear triangles but not generally for a spline basis.

Sample initial cubic spline loops into constrained mesh segments using geometric
tolerance and an element-size target. Stable geometry IDs must survive remeshing.

Keep these semantics distinct as they arrive:

- An obstacle removes its interior and supplies boundary conditions.
- A material interface retains both sides with appropriate coefficient/continuity
  treatment.
- A thin wall needs the correct separation of the two sides' degrees of freedom;
  a constrained edge alone is insufficient.

The initial implementation handles obstacle loops in a fixed outer box. Outer
editable boundaries and more involved topology are future extensions.

## Milestone 1 spline and document model

`femfun-core` is dependency-free. It owns `Point2`, `PeriodicCubicSpline`,
`ObstacleId`, `Scene`, parameter-bearing samples, and structured validation
results. Controls and one period of positive nonuniform knot intervals are f64.
There are 4–128 controls, each associated with its same-index knot. Knots and
controls extend periodically; de Boor evaluates positions and the differentiated
control/knot sequences. Intervals below 1e-12 of a period are rejected as
numerically ill-conditioned. Parameters wrap over one period.

Periodic knot insertion updates a whole period of the affected control sequence,
including controls crossing the seam, and preserves position and derivatives.
Existing knots select their associated handles. Removal drops a control and its
knot, merges the neighboring intervals, and is explicitly allowed to reshape.
The UI snaps a curve click within four logical pixels of an existing knot's
curve position to that knot, after giving control handles hit-test priority.

Adaptive sampling handles each knot span using its exact cubic Bezier control
hull and requires monotone projection onto the chord. This catches inflections
and small reversals that a midpoint-only flatness test can miss. Every sample
retains its parameter. Rendering uses a 0.6 logical-pixel tolerance, up to 2,048
points per obstacle at depth 14, cached by revision and scale. Validation uses
an independent fixed world-space sampling tolerance of 0.000025, depth 16, and
4,096 points per obstacle. Exhaustion is an unaccepted result.

`ValidationJob::advance` processes a caller-supplied work budget. Sampling,
segment construction, pair tests, and containment tests resume across frames;
the application gives each job 12,000 operations per frame. Disjoint loop bounds
skip segment blocks and containment tests. A 50-million-operation ceiling rejects
unreasonably expensive scenes. Contact clearance is 0.0002 (1e-4 of box width),
with sampling uncertainty added. Self-contact checks distinguish nearby samples
along one local arc from nonlocal contact, so very short spans introduced by knot
insertion do not falsely reject an unchanged curve. Crossings are still tested.
These approximate predicates are editor guards, not a meshing certificate.

The application owns a `Document { draft, accepted }`. Every edit advances a
monotonic revision and invalidates outstanding draft validation. At the end of
each frame, validation advances for the latest draft. Only a matching valid
revision promotes the complete draft. Invalid drafts remain editable and preserve
the accepted reference; release never silently rolls them back. Escape restores
the pre-drag document. Revert Draft is an explicit undoable action.

History stores both scenes in document snapshots, grouping drags and numeric
edits into transactions. It retains 100 actions, clears redo on a new edit, and
excludes selection and navigation. Stable obstacle IDs increase independently of
undo, preventing reuse after undoing creation. Loading derives the next ID from
both scenes and clears history.

Version 1 JSON stores the fixed domain, IDs, controls, intervals, and both scenes.
Serde and rfd live only in the app crate. Round-trip f64 parsing preserves exact
stored values. Files have a 2 MiB limit, strict fields/version/domain, finite
values, and scene/control-count checks. A prospective load validates its accepted
scene incrementally while the existing document stays intact. Invalid draft
geometry is allowed. Failed parsing or validation never replaces the document.

The viewport and panels share egui's event routing and logical-pixel coordinates.
Geometry is drawn with the egui painter on Bevy's wgpu device. Input gestures
start in the viewport; panel/text capture prevents accidental geometry edits.
Canvas resize/display scale comes from Bevy/egui, and Fit View frames the fixed
box. JSON excludes the viewport. There is no independent wgpu device, WebGL
fallback, audio subsystem, or 3D rendering pipeline. This editor acceptance model
is separate from the future solver transaction machinery below.

## Custom triangular meshing

Milestone 2 keeps meshing in `femfun-core` with no numerical dependencies. The
predicate layer first evaluates orientation and incircle determinants with a
certified floating-point error bound. Ambiguous results fall back to exact-sign
non-overlapping expansions. Tests compare thousands of integer-coordinate cases
against `i128` determinants and exercise nearly collinear and cocircular inputs.
The editor's proximity tolerance remains separate: exact predicate signs answer
topology questions, while the editor tolerance decides whether near-contact is
acceptable input.

Mesh construction samples the fixed outer square and every accepted spline. The
outer loop is counter-clockwise and obstacle loops are clockwise. A visibility
bridge turns each hole into one weakly-simple polygon; exact-sign ear clipping
creates the initial constrained triangulation. Boundary segments are authoritative
constraints. Global edge legalization flips only unconstrained convex diagonals,
using the robust incircle predicate, to produce a locally constrained-Delaunay
mesh. Triangle centroids are checked against the sampled domain before a result
is published.

Quality refinement selects the worst size or angle violation. It inserts a
circumcenter when that point remains in the meshed domain and otherwise uses the
triangle centroid. A candidate that encroaches a constrained edge splits that
edge and carries its label and continuous parameter range to both children.
Interior insertions split their containing triangle or edge, followed by robust
legalization. The defaults target an 18° minimum angle and 0.18 maximum edge;
the interactive overlay uses 12°, 0.16, and a coarser 0.0015 curve tolerance for
responsive diagnostics. Both are explicit choices rather than solver accuracy
claims.

`TriMesh` stores f64 vertices, counter-clockwise triangle indices, labeled outer
and obstacle edges, stable obstacle IDs, continuous boundary parameters, and
aggregate quality. Boundary labels are independent of temporary vertex and
triangle indices. Verification requires positive triangle orientation, manifold
edge adjacency, exactly one incident triangle for every boundary edge, domain
classification of every centroid, and configured size/angle targets.

`MeshingJob` snapshots an accepted geometry revision. Its topology-preparation
phase is bounded by sampling and mesh capacities; each later `advance` unit makes
at most one refinement insertion plus legalization. The application advances two
units per frame after an edit transaction ends and replaces obsolete jobs when a
new accepted scene arrives. It keeps the previous accepted mesh throughout an
invalid draft. Current topology preparation is a single phase rather than fully
time-sliced; a complex valid scene can therefore still cause one longer frame.
Later transaction machinery can split or move that phase if measurements require
it.

The overlay draws all accepted triangle edges, emphasizes boundary labels, colors
elements below 15° amber, and reports counts and extrema. A meshing failure is a
structured diagnostic and never changes the draft, accepted geometry, or history.
The current bridge search is deliberately simple and deterministic. Difficult
valid arrangements may return a topology or capacity error rather than attempting
unbounded recovery.

## Initial evolution model

Use a dimensionless scalar model

```text
m(x) u_tt + d(x) u_t - div(a(x) grad(u)) = f
m > 0, a > 0, d >= 0
```

Begin with linear triangular FEM, lumped mass, and second-order explicit time
integration. Pick the damping treatment deliberately. GPU work should gather
contributions into uniquely owned outputs, avoiding floating-point scatter atomics.

Choose a conservative timestep from the actual accepted operator, including
relevant restrictions introduced by damping and later radiation conditions. Mesh
quality, coefficient changes, and wave speed are part of timestep management.
Bound the number of substeps per frame and report achieved simulation speed.

Use a CPU reference to check GPU kernels. Prefer f64 for geometry/assembly on the
CPU and f32 GPU fields initially; normalize scales and validate uploaded values.
Predicate robustness needs its own treatment rather than a universal epsilon.

## Transaction lifecycle

Maintain an accepted simulation revision, at most one active candidate, and the
latest requested geometry/parameter revision.

1. Capture a candidate request and its accepted source-discretization revision.
2. Prepare geometry, mesh, connectivity, operators, boundary labels, transfer map,
   and timestep under a bounded work budget. Continue evolution on the old system.
3. Validate the candidate. Preparation errors discard the candidate, not the
   accepted simulation.
4. At a complete timestep boundary, verify the transfer's source revision is still
   current and apply it to the latest GPU state.
5. Initialize or transfer auxiliary state as specified, enforce constraints, and
   validate the resulting state before activating it. Preserve old buffers until
   the candidate can safely become active.
6. Switch all accepted resources together, then schedule the latest outstanding
   request if necessary.

GPU validation may itself take time; design its ordering/readback deliberately so
the old simulation remains usable while a commit is pending. A CPU-valid mesh
alone does not establish that a transferred GPU state is finite.

The old mesh/operators remain fixed during candidate preparation; only the old
state evolves. Transfer maps target the source discretization, not a captured
field snapshot. For triangles, map new samples to old triangle indices and
barycentric weights on the CPU, then apply that map to current state on the GPU.
Keep other discretizations free to provide different transfer operations.

Coalesce pointer events. Do not restart all preparation on every event: useful
intermediate shapes may commit while a newer request waits. Obsolete or invalid
candidates may be discarded. Show proposed and accepted geometry separately so
latency is visible and understandable.

## State transfer and stability

Transfer both the field and its time derivative (or equivalent complete integrator
state). Respect time staggering; changing the timestep must not reinterpret an old
half-step state as a new one. Boundary auxiliary fields need an explicit policy.

Barycentric interpolation is acceptable initially. It need not conserve energy.
Avoid interpolation across excluded obstacles or incorrectly across distinct
regions. Newly exposed domain needs a documented initialization policy before live
edits are enabled. Mild local smoothing is a possible response to edit-induced
bursts, not a substitute for stable stepping.

Validate finite values, positive areas/masses, operator consistency, and admissible
timesteps. State magnitude guards and recovery behavior can be added based on
observed failures. A sudden material edit may legitimately change energy; do not
globally renormalize it automatically.

Geometry editing initially means changing the domain and carrying the wave forward
approximately. It does not claim to model wall work or Doppler effects.

## Bounded adaptation

Budget mesh size, adaptation work, and simulation work separately. Use minimum
admissible scales and element quality checks to prevent tiny features from silently
forcing unlimited stepping work. Budget exhaustion can defer acceptance, limit the
accepted geometry, or reduce simulated-time throughput; keep the UI responsive and
report the chosen outcome.

Use resumable CPU work across frames first, or a worker if measurements warrant
one. Merely putting a non-yielding rebuild in an async function does not amortize
its CPU cost. Browser threading is not a prerequisite for the initial design.

Adapt to geometry/quality first, then user wavelength targets and potentially the
field. Use hysteresis and cooldown for refinement/coarsening. Retain full remeshing
as a fallback for large or troublesome edits.

## Radiation and IGA

The target absorber lives on the one-dimensional outer boundary. Start with a
first-order outgoing condition and investigate higher-order auxiliary-boundary
conditions next. Corner coupling and long-time stability are explicit work items.
Keep the outer box fixed while the interior evolves.

IGA is an intended experiment after triangles. Start with an untrimmed single
patch. Boundary splines alone do not supply an interior parameterization. Evaluate
quadrature, mass treatment, spectral behavior, timestep restrictions, and transfer
on their own merits before extending to multipatch or trimmed geometry.

## Reference starting points

These are implementation/research references, not numerical dependencies:

- [Bevy WebGPU examples](https://bevy.org/examples-webgpu/)
- [bevy_egui documentation](https://docs.rs/bevy_egui/)
- [DUNE wave equation with mass lumping](https://www.dune-project.org/sphinx/content/sphinx/dune-fem/wave_nb.html)
- [WGSL specification](https://www.w3.org/TR/WGSL/)
- [Hagstrom-Warburton complete radiation conditions](https://doi.org/10.1137/090745477)
- [Mass lumping and outlier removal for complex IGA geometry](https://arxiv.org/abs/2402.14956)
