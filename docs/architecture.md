# Architecture notes

The spline editor, constrained mesher, bounded coordinate-edit repair, enriched
quadratic GPU wave solver, and geometry-edit simulation transactions are
implemented. Outer sides, hole spans, and baffle faces support reflecting or
prescribed Neumann data, prescribed Dirichlet data, and first- or second-order
radiation. Stable material regions, transmitting interfaces, closed two-sided
walls, and open baffles with independent face laws or coupled thin-gap spans are
implemented. The closed-wall representation is retained for scene compatibility,
but the current editor does not offer it as a normal role. A first spatial
edge-size adaptation transaction is implemented; user wavelength and active
solution indicators remain work.

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

The application shell is canvas-first: a top action bar owns document, view,
inspector, Draw, and wave playback actions, while a hideable contextual right
inspector exposes one of four panels: Edit, View, Simulation, or Materials. The
solver starts in the running state after its initial operator is committed.
Selection remains contextual rather than introducing separate selection modes.
Draw opens a transient role/primitive popover: holes and interfaces offer Circle
and Custom, while baffles offer Straight and Custom. A single `InteractionMode`
owns draw, pulse-placement, and source-placement state. Placement modes are shown
over the viewport and survive inspector changes; preset geometry is deliberately
one-shot, while pulse and source placement remain active until toggled off, ended
with the overlay, or cancelled with Escape.

The lower-right status control carries a compact performance summary (FPS, solver
steps per second, DOFs, mesh size, and timestep). Detailed performance measurements
are UI-only diagnostics in one draggable, scrollable window with Frame, Mesh,
Handoff, and Solver sections. It includes a rolling frame-time plot and aggregate
frame statistics. Mesh/solver errors open diagnostics and light the warning marker;
ordinary rebuilding does neither. Validation, mesh, and handoff stages share the
middle of the status strip, while successful document and handoff actions appear as
short-lived notices.

Control handles are exclusive selections. Boundary spans support click, Shift
toggle, Command/Ctrl whole-curve selection, and marquee selection with optional
filters. Whole-object role and delete actions appear only when one complete curve
is selected, so a mixed selection cannot accidentally act on its last member.
Selected spans move as rigid pieces and expose the transform, boundary, and
topology controls together. A viewport rotation ring, uniform scale grip, and
movable pivot complement numeric transforms. Boundary laws may be visualized in
place for the outer box, hole spans, and both baffle traces; selection is drawn over
that diagnostic layer.
Closed two-sided loops remain load-compatible but are omitted from normal creation
and role controls because open baffles cover the useful two-trace workflow.

## Geometry and discretization

Geometry owns curves, loops, boundary labels, materials, and regions.
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
- An open baffle has two traces but does not define separate material regions.
  Its free endpoints reconnect the domain and admit diffraction around the tips.

The topology model has stable `RegionId` and `MaterialId`; logical span identities
arrive with assigned boundary conditions. A closed loop has
an explicit role; its winding or containment must not silently decide whether it
is a hole, a transmitting material interface, or a two-sided wall. Regions form a
validated containment graph. Mesh triangles carry their owning region/material ID,
while constrained edges carry their logical boundary/span ID and side information.
Those labels survive remeshing even though numerical indices do not.

Material interfaces use a conforming field with piecewise mass density, stiffness,
and damping, giving the scalar weak-form transmission condition. Internal walls
require separate traces on their two sides and therefore a topology/DOF operation,
not merely a coefficient label. Material-value changes reuse the current mesh and
enter the same transactional operator replacement path as boundary changes.

Boundary conditions attach to logical parameter spans rather than individual mesh
segments. Sampling copies a span assignment onto every resulting constrained edge.
Knot insertion can split an assignment exactly; removal, seam movement, and span
merging need explicit inheritance rules and must reject ambiguous drafts. Junction
nodes and higher-order auxiliary fields belong to this semantic boundary graph.
The editor represents an outer edge, hole span, or baffle span/face with one
transient boundary-selection type. Viewport hit testing creates that selection and
one inspector dispatches to the conditions supported by its target; selection is
excluded from scene files and document history.

Open splines store one law per nonempty knot span. A span either has independent
left and right face conditions or one paired thin-gap law. These modes are mutually
exclusive. Reflecting faces add no weak boundary term. Prescribed Neumann data adds
a quadratic boundary load, prescribed Dirichlet data owns the trace nodes strongly,
and both use the same harmonic time law as outer boundaries. A first-order outgoing
face multiplies the adjacent material's characteristic impedance
`sqrt(rho * kappa)` by its adjustable ratio and contributes positive lumped
boundary damping. Second-order outgoing faces also assemble the tangential
auxiliary operator along curved or open spans. The thin-gap law adds a symmetric
positive-semidefinite spring for the trace jump, scaled by material stiffness. It
enters the assembled spectral bound and can reduce the explicit time step. A
dissipative relative dashpot is deferred because preserving the centered scheme
would require an off-diagonal damping solve.

Periodic loops likewise store one exterior-face condition per knot interval. Hole
spans support reflecting, driven Neumann or Dirichlet, and first- or second-order
outgoing behavior against the material in the hole's exterior region. Periodic
knot insertion copies the split condition to both child spans, including across the
stored seam. Removal
merges the deleted interval into its predecessor and is rejected when those two
conditions differ. Boundary-law edits are excluded from geometry equality, so an
accepted condition change rebuilds the operator on the existing mesh.

The initial implementation handles obstacle loops and open baffles in a fixed
outer box. More involved topology remains a future extension.

## Milestone 1 spline and document model

`funfern-core` is dependency-free. It owns `Point2`, `PeriodicCubicSpline`,
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
the pre-drag document, and Undo restores earlier document snapshots.

History stores both scenes in document snapshots, grouping drags and numeric
edits into transactions. It retains 100 actions, clears redo on a new edit, and
excludes selection and navigation. Stable obstacle IDs increase independently of
undo, preventing reuse after undoing creation. Loading derives the next ID from
both scenes and clears history.

Selection is either one spline control or a transient set of topological spans.
Handles always select one control for local deformation; spans support bulk
boundary assignment, while a set containing every span of movable curves also
supports exact affine transforms. A screen-space marquee selects every curve or
outer span touched by its rectangle. Its geometry-kind filter can restrict the
operation to loops, baffles, or outer edges; replacement, additive, and subtractive
operations all produce the same transient span set used by click selection.
Repeated-knot multiplicity is stored separately from positive knot intervals, so
C2, C1, and C0 joins do not create empty boundary-condition spans. Raising
multiplicity uses exact knot insertion and
leaves geometry and span assignments unchanged, including at the periodic seam.
Complete curves are always transformable. A partial selection is transformable
when every exposed end is a C0 knot or an open-curve endpoint. The union of the
four controls active on each selected span receives one affine map, which moves
the selected subcurve rigidly; an adjacent unselected span can change only through
its shared corner control. An atomic isolation action inserts every missing C0
knot first. This also works for selection components wrapping across a periodic
seam. Rigid dragging and snapping use the selected arcs' length-weighted centroid.
Shift distinguishes a click from a drag before toggling an already selected span;
during translation it temporarily enables grid snapping, and during rotation or
uniform scaling it selects the transform's fixed increment. The transform pivot is
transient, as are selection, transform inputs, and snapping preferences. Bulk
boundary edits validate all outer, hole, and oriented
baffle-face targets before one revision and history transaction. Baffle left/right
always follows increasing spline parameter.

Open baffles split at an existing breakpoint after exact refinement to C0. The
start half retains its stable ID and the end half receives a new one. Exact shared
tips in the same region are valid topology junctions. Merging chooses the nearest
pair of tips; any parameter reversal also reverses span-law order and exchanges
left/right face assignments. Smoothing removes one repeated knot by solving for
the lower-multiplicity controls and reinserting the knot as a verification step.
An exact reconstruction is used when it matches within a scale-aware tolerance.
Otherwise the least-squares lower-multiplicity projection is committed as an
explicit reshape; the maximum refined-control residual provides a convex-hull
upper bound on curve displacement. Each continuity change is one undoable action.

Loop role conversion preserves the stable obstacle ID and span assignments.
Changing a hole to a material interface or closed wall allocates a new owned
region with the selected material. Interface/wall conversion retains the region.
Changing either to a hole removes an empty interior region; direct child loops or
baffles make the conversion fail atomically because they cannot remain inside a
void.

Duplication creates new stable geometry IDs, copies knot intervals,
multiplicities, and span laws, and gives duplicated material-interface or wall
loops their own interior region with the same material.

Version 8 JSON stores the fixed domain, loop roles, all assigned boundary laws,
materials, regions, controls, intervals, knot multiplicities, and both scenes.
Versions 2–7 remain
compatible; version 1 loads by assigning its loops the background hole role and
creating the default background material/region. Older loop records migrate to a
reflecting condition on every periodic span. Version-6 baffles that combined a
thin-gap law with independent face laws migrate with the thin-gap law taking
precedence.
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

Milestone 2 keeps meshing in `funfern-core` with no numerical dependencies. The
predicate layer first evaluates orientation and incircle determinants with a
certified floating-point error bound. Ambiguous results fall back to exact-sign
non-overlapping expansions. Tests compare thousands of integer-coordinate cases
against `i128` determinants and exercise nearly collinear and cocircular inputs.
The editor's proximity tolerance remains separate: exact predicate signs answer
topology questions, while the editor tolerance decides whether near-contact is
acceptable input.

Mesh construction samples the fixed outer square and every accepted spline. It
builds one polygonal domain per retained region. A material interface reuses one
vertex trace for its exterior and interior domains; a wall duplicates the trace
so the two sides have independent DOFs. A visibility bridge turns each domain and
its direct child holes into one weakly-simple polygon; exact-sign ear clipping
creates the initial constrained triangulation. Boundary segments are authoritative
constraints. Queued edge legalization flips only unconstrained convex diagonals,
using the robust incircle predicate, to produce a locally constrained-Delaunay
mesh. Triangle centroids are checked against the sampled domain before a result
is published.

Open baffles are inserted after the closed-region mesh reaches its requested bulk
resolution. Each sampled open curve is recovered as a constrained edge chain by
deterministic diagonal flips. Interior chain vertices are duplicated and the
triangle fan on one bank is rewired to the duplicate; the two free tips stay
shared. Both banks are labeled independently. Before insertion, a nearby free bulk
vertex may be relocated onto the exact curve sample when every triangle in its fan
remains positively oriented and in the assigned region. This prevents short edges
caused by nearly coincident free and constrained vertices. Minimum-angle refinement
is completed before the cut because a zero-thickness crack tip is a reentrant
singular point; the final mesh reports the resulting tip quality instead of
repeatedly refining the coincident faces.

Quality refinement selects the worst size or angle violation from an ordered
queue. Persistent edge adjacency and triangle quality entries are updated only
for changed triangles; removed/replaced entries leave no stale quality records.
An edge queue with duplicate suppression revisits the neighborhood of each split
or flip, rather than rebuilding connectivity and sweeping all edges. It inserts a
circumcenter when that point remains in the meshed domain and otherwise uses the
triangle centroid. A candidate that encroaches a constrained edge splits that
edge and carries its label and continuous parameter range to both children.
Interior insertions split their containing triangle or edge, followed by robust
legalization. Core defaults target an 18° minimum angle and 0.18 edge length.
The application uses 12° and offers maximum-edge settings 0.16 (preview), 0.04
(default), and 0.02 (fine). Its internal target is requested maximum / 1.05,
compensating for the refiner's 5% slack. Curve tolerance is min(0.0015, h/50),
so finer meshes also resolve the curved boundary more closely. App capacities
are 50,000 vertices, 100,000 triangles, and 50,000 insertions. These limits do
not promise either a particular wave accuracy or interactive rebuild latency.

`TriMesh` stores separate geometry and mesh revisions, f64 vertices,
counter-clockwise triangle indices with stable
region IDs, labeled outer/hole/interface/wall/baffle edges, stable curve IDs, continuous
boundary parameters, and aggregate quality. Boundary labels are independent of
temporary vertex and triangle indices. Verification requires positive triangle
orientation, manifold edge adjacency, two incident triangles for a transmitting
interface and one for each other boundary trace, region classification of every
centroid, and configured size/angle targets.

`MeshingJob` snapshots an accepted geometry revision. Validation, adaptive sampling,
bridge visibility, ear clipping, legalization, and output verification resume
across calls. Bridge and ear searches yield between segment/vertex tests. The
bridge search tries the shortest pair first and falls back to a complete visible
bridge search if it is blocked. Each legalization unit checks at most one edge;
one insertion changes at most four triangles. The synchronous `mesh_scene` drives
the same state machine, so slice sizes do not change results.

The application checks a portable monotonic clock between units, targeting 2 ms
of meshing per frame, with a ceiling of 100,000 units. This is a soft deadline:
boundary assembly, point location, insertion checks, and domain classification
still use linear capacity-bounded scans, and OS scheduling can delay a call.
There is also a 50-million-unit job ceiling, a per-legalization work ceiling, and
the existing refinement/vertex/triangle limits. Deterministic counters report
quality evaluations, edge tests/flips, and insertions. The UI reports elapsed
request-to-ready time, accumulated active work, time outside mesh slices, and the
longest mesh slice, separately from its existing display frame-time indicator.
Request-to-ready starts when accepted geometry requests a mesh; the opt-in native
benchmark additionally measures from the control edit, including editor validation.

Jobs run after an edit transaction ends and are replaced when a newer accepted
scene arrives. The previous accepted mesh remains available during preparation
and invalid drafts, and is retained if candidate preparation fails. The displayed
mesh has its own committed scene and resolution, separate from the latest request.
Superseding a job always starts from that committed pair, so a half-finished
candidate cannot become the source of a later edit.

`MeshUpdateJob` attempts reuse for hole, material-interface, and open-baffle
control-coordinate edits with unchanged IDs, roles, and knot intervals at the same
resolution. Closed-wall motion currently uses the full region-aware mesher. The
local path imports the accepted mesh and
connectivity in resumable linear passes. Boundary vertices whose spline parameter
positions changed move to the new spline; unchanged boundary vertices remain fixed.
Interior displacement uses 24 Jacobi averaging sweeps with fixed boundary values
and zero displacement outside a radius max(4h, 4×maximum boundary displacement).
An extra triangle ring forms a frozen repair boundary. Motion exceeding 2h or a
patch touching more than max(256, one third of the old elements) falls back.

Moved elements must remain positively oriented. Changed spline edges are checked
against cubic Bezier hulls and subdivided locally when needed. Constrained edge
flips and quality refinement are confined to the frozen patch; an operation that
needs to cross its boundary triggers full reconstruction. Conservative interior
edge collapse checks the link condition, orientation, angle, and edge-size bounds.
Collapse is attempted below 0.35h, separated from the 1.05h refinement threshold
to reduce oscillation. Coordinate-edit repair does not collapse boundary vertices.
Final compaction
removes unused interior vertices; indices may change, but distant geometry and
triangles are preserved.

Local work is limited to five million units, 256 curved-boundary subdivisions and
512 quality insertions, with the existing capacities and per-frame budget. Output
passes check orientation, adjacency, and quality globally, and domain membership
in the changed patch; unchanged regions inherit the source mesh's certificate.
Import, final verification and compaction still touch the whole mesh; candidate
point location and encroachment checks still scan globally. Geometry/topology
changes, unlike those bookkeeping passes, are spatially bounded. Preserved-element
statistics require both identical connectivity and exactly identical coordinates,
and are computed before compaction. Connectivity-only reuse is reported separately
by the core. Fallback frequency counts completed requests, excluding canceled jobs.

Creation, deletion, knot edits, resolution changes, excessive motion and failed
repair use the full resumable mesher. Resolution remains an application preference,
excluded from geometry files and undo history.

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

The implemented operator uses linear triangular FEM with row-wise CSR stiffness,
lumped mass, and lumped damping. Reflecting outer and obstacle boundaries are the
natural Neumann condition, so all mesh vertices remain DOFs. The centered update is

```text
(M + dt D/2) u[n+1] = (2M - dt² K) u[n] - (M - dt D/2) u[n-1] + dt² f[n]
```

It is second order for zero damping and treats diagonal damping symmetrically. A
Gershgorin bound on `M^-1 K` supplies `dt_max = 2/sqrt(bound)`; the app uses 90%
of that bound. The f64 CPU implementation is the reference and conserves the
scheme's discrete half-step energy to roundoff in the undamped test.

The f32 GPU kernel stores both committed time levels and a scratch level in one
storage buffer. Each solution-DOF invocation gathers its CSR row and writes only its
own scratch value; a second dispatch rotates levels. This avoids scatter atomics.
The operator, state, source, and controls use Bevy's render-world buffers and its
existing wgpu device. State remains GPU-resident; asynchronous readback supplies
the egui field colors and energy diagnostic. Each readback carries a GPU-written
step marker so stale asynchronous results cannot be mistaken for a newer level.

The previous h≈0.16 overlay was an editor preview. Wave benchmarks start with
h≤0.04 and h≤0.02, corresponding to 10 and 20 maximum-edge lengths per reference
wavelength 0.4 (five wavelengths across the box). P1 has one scalar spatial DOF
per vertex before essential boundary elimination; reflecting Neumann boundaries
retain all vertices. Resolution relative to wavelength is only a starting point:
phase error accumulates with propagation distance and must be measured. Use
analytic reflecting-box modes, e.g. cos(10π(x+1)/2) with the matching temporal
frequency at c=1, and compare over one and five box crossing times. Vary timestep
independently. Obstacle cases subsequently need a sufficiently refined reference
and a stated source bandwidth.

The measured P1 baseline confirms material dispersion. At 0.225 of the
conservative timestep, h=0.04 accumulates about 0.96 rad of phase error over five
box crossings; h=0.02 accumulates about 0.24 rad. Temporal halving changes these
figures only slightly, so spatial error dominates.

The selected higher-order candidate is the seven-node enriched quadratic triangle
`P2 + span(27 λ0 λ1 λ2)`. Vertex, edge-midpoint, and centroid quadrature weights are
respectively 1/20, 2/15, and 9/20 of element area, producing a positive diagonal
mass. Shared edge nodes remain conforming and each element owns its centroid node.
The formulation and positive nodal quadrature follow the [TUM analysis of
higher-order mass-lumped triangular elements](https://mediatum.ub.tum.de/doc/1452928/376222.pdf).
The stiffness uses a separate symmetric six-point rule exact through degree four,
which exactly integrates products of the enriched basis gradients.

At parent h=0.08, this reference has 9,215 DOFs and accumulates 0.00519 rad phase
error at t≈10, compared with 0.238 rad for 25,218-DOF P1 at h=0.02. Estimated GPU
buffers are 1.15 MiB versus 2.20 MiB; measured f64 CPU throughput is 8.61 versus
3.61 simulated seconds per wall second on Apple M1 Max. Halving the timestep moves
the quadratic phase error to 0.00818 rad, exposing some cancellation between
spatial and temporal dispersion at the larger step. Keep both refinements in
comparisons. The finer parent-h=0.04 quadratic case reaches roughly 0.0005 rad
phase error, but its 37,443 DOFs, 4.70 MiB estimate, smaller timestep, and dense
element rows reduce CPU throughput to about one simulated second per wall second.

The production application now uses this enriched quadratic operator at parent
h=0.08 by default. The CSR gather evolution is independent of element degree; GPU
node data includes the parent vertices, shared edge midpoints, and element bubble
centroids. Field display splits each parent triangle into six visual triangles so
all seven coefficients contribute. Transaction transfer locates every new node in
an old parent triangle and applies its seven enriched basis values to displacement
and velocity. Source velocity is reconstructed once per old DOF and cached before
the interpolation gather, avoiding repeated old-operator rows. Ordinary
higher-degree triangular Lagrange bases must not inherit P1 lumping. Report mesh
preparation, operator preparation, display time, and stepping throughput separately,
with DOFs, memory, timestep, and phase/amplitude error.

The application requests at most 16 substeps per display frame and reports achieved
simulation time per wall time. Pulse injection modifies both stored levels equally,
giving zero added velocity. The continuous source is a Gaussian nodal acceleration
with a sinusoidal time factor; its default frequency is 2.5 cycles per dimensionless
time, corresponding to wavelength 0.4 at wave speed one.

The old solver keeps running while a replacement mesh is prepared. Invalid drafts
do not alter the active solver. Reset is explicit; accepted geometry edits carry
the field forward through the transaction lifecycle below.

## Transaction lifecycle

The implementation maintains an accepted simulation revision, at most one active
candidate, and the latest requested geometry revision.

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

Candidate mesh construction remains resumable. Operator assembly and the spatial
transfer map are currently prepared synchronously when meshing finishes; their
measured cost is small at current capacities, but this is the remaining bounded-work
gap in the transaction. The old solver continues until all previously requested
steps have been encoded. The application then pauses scheduling briefly, dispatches
the transfer, and waits for a generation-tagged finite GPU readback before switching
mesh, operator, timestep, and displayed field together. A shader failure restores
the retained old buffers.

The old mesh/operators remain fixed during candidate preparation; only the old
state evolves. Transfer maps target the source discretization, not a captured
field snapshot. For enriched quadratic triangles, map every new solution node to an
old parent triangle on the CPU, then evaluate that triangle's seven basis functions.
The CPU lookup uses a uniform spatial bin index rather than testing every old
triangle for every new node. Keep other discretizations free to provide different
transfer operations.

Coalesce pointer events. Do not restart all preparation on every event: useful
intermediate shapes may commit while a newer request waits. Obsolete or invalid
candidates may be discarded. Show proposed and accepted geometry separately so
latency is visible and understandable.

## State transfer and stability

Transfer both the field and its time derivative (or equivalent complete integrator
state). Respect time staggering; changing the timestep must not reinterpret an old
half-step state as a new one. Boundary auxiliary fields need an explicit policy.

Enriched quadratic interpolation does not conserve energy. A target node maps only
through an old triangle that contains it, so transfer never crosses an excluded
obstacle. Nodes newly exposed by a shrinking or moved obstacle start with zero
displacement and velocity. Mild local smoothing is a possible response to
edit-induced bursts, not a substitute for stable stepping.

Transfer for closed regions respects topological connectivity. Regions joined
through material interfaces form one transferable component, while a two-sided
wall separates its components even where old and new domains overlap geometrically.
Closed-wall sources use the same region membership. Quadratic nodes on an open
baffle carry its stable boundary ID and face. At a coincident location, the locator
prefers a source element adjacent to the same face; when a moved trace no longer
overlaps an old face element, it falls back to the containing bulk element. Thus a
stationary jump across the baffle is not mixed during handoff, while newly moved
faces receive the field from their former physical location. Pulse and continuous
source stencils separately use shortest paths through the cut finite-element graph,
so they can reach the opposite bank only by travelling around a free tip.

The hard zero policy is intentionally temporary. When newly exposed nodes meet a
nonzero retained field, it creates a steep artificial front and injects broadband
wave content. A future commit pass should construct a narrow transition band around
the exposed region and smooth or taper both displacement and velocity there while
leaving established nodes outside that band unchanged. The pass needs a bounded
work budget and diagnostics for its energy and spectral effect; it must not silently
renormalize the whole field.

For the centered two-level scheme, the first GPU transfer dispatch reconstructs
current velocity once per old DOF from previous/current displacement, the old
operator, damping, forcing, and timestep. It stores velocity in the old state's
otherwise disposable scratch component. A second dispatch maps current displacement
and velocity with seven basis weights. A third applies the new operator and forcing
to initialize the new previous displacement consistently with the new timestep.
The exact GPU clock is copied at commit, so a continuous source retains its phase.
Every pipeline stays within WebGPU's portable eight-storage-buffer-per-stage limit.

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

`MeshAdaptationJob` is the first broader adaptation path. One immutable transaction
samples a spatial target length at triangle vertices, edge midpoints, and centroids;
coarsens short edges; restores local constrained-Delaunay legality; refines long
edges; verifies the result; and compacts it. Every phase resumes under the caller's
work budget. The app gives it the same soft 2 ms frame slice as ordinary meshing.
Refinement starts above `1.05 h_target`; collapse uses a configurable lower ratio.
Automatic solution adaptation uses `0.65 h_target`, with post-collapse size and
quality checks providing the final guard.
Persistent vertex lineage records the generation of each topology change, and a
configurable generation cooldown prevents an immediately changed vertex from
oscillating on the next pass.

Outer corners, spline seams and logical knot breakpoints, and open-curve tips are
anchors. Other constraint vertices may coarsen if the merged chord still represents
the exact cubic span within the fixed curve tolerance. Material interfaces keep a
shared trace. Closed walls and open baffles refine or collapse their two coincident,
oppositely oriented traces atomically; each face retains its own periodic parameter
interval. Malformed pair topology is an error, while a locally unsafe collapse is
skipped. Capacity or topology-change exhaustion publishes a valid partial result
with an explicit limit report; malformed input and total work exhaustion discard
the candidate.

Geometry revision identifies the accepted scene, while mesh revision identifies a
particular discretization of it. Wave operators and linear/quadratic transfer maps
validate both. A successful adaptation assembles a candidate operator, prepares a
quadratic transfer from the still-running source mesh, and commits mesh, operator,
state, timestep, and lineage together after the tagged GPU handoff. The target field
is transient and is not serialized. The application derives it automatically from
the live quadratic solution. The existing GPU state readback uses spare auxiliary
lanes to carry centered displacement, velocity, and acceleration at one explicit
time level; this avoids another buffer or readback. The core estimator combines
material-aware recovered flux defects, a strong interior equation residual with the
volume source removed, and interior flux jumps. It does not yet add
physical-boundary residuals. Recovery never averages across a material region or a
duplicated baffle/wall trace.

The relative indicator maps error to local edge length, caps that length by the
shortest wavelength of every active continuous or driven-boundary source, and grades
neighboring targets. Quiet-element growth factors and collapse thresholds are paired
so a low-error mesh can actually coarsen. Graded targets stay local to each source
triangle instead of taking the minimum over an entire vertex star; evaluation still
chooses the conservative side on a shared edge. Fast, Balanced, and Detailed presets
select tolerance, elements per wavelength, topology budget, and maximum coarsening
step. Advanced minimum/maximum limits keep capacity explicit and allow quiet regions
to become coarser than the initial mesh.

The app advances indicator and adaptation jobs in soft 2 ms slices, rejects stale
mesh/GPU/settings generations, and waits between estimates. Refinement can start
after one estimate. Coarsening requires two consecutive estimates on the same mesh,
and consumes at most half of one automatic transaction so refinement retains a
separate topology budget. Geometry editing cancels estimator/adaptation preparation
and takes priority. An optional heatmap shows the transient target.

Coordinate-only edits of closed holes, transmitting material interfaces, and open
baffles first try local mesh repair. Each attempt imports the immutable committed
mesh afresh; failed topology changes never become the input to the next attempt.
The first patch contains vertices within `max(4h, 4d)` of each moved loop's old/new
bounds plus one triangle guard ring. Retry attempts add `2h` and another guard ring,
up to three attempts total. Curved-boundary and refinement budgets also grow per
attempt. A motion over `4h`, a patch larger than `max(256, triangle_count/3)`, invalid input,
capacity exhaustion, or the aggregate local-work ceiling goes directly to a full
rebuild. Inversion, contact with the frozen patch boundary, curved subdivision
exhaustion, and refinement exhaustion retry first. Stable loop IDs associate mesh
vertices with splines, while closed boundary cycles are reconstructed in scene
order so nested material regions keep their explicit ownership. Open baffles are
imported as parameter-keyed pairs of oppositely oriented left/right edges. Their
interior vertices stay distinct and coincident, their free tips stay shared, and any
curvature or length subdivision splits both faces at the same parameter. Segment
capsules select a narrow repair strip instead of the bounding box of the whole open
curve. Trace coarsening, baffle topology changes, and two-trace closed-wall motion
remain on the full-build path.

## Radiation and IGA

The absorber lives on the one-dimensional outer boundary. The implemented
first-order condition is `∂n u = -u_t/c`. In the weak equation it contributes the
positive boundary damping `∫Γ sqrt(rho k) v u_t ds`. Each quadratic boundary edge
uses the diagonal endpoint/midpoint/endpoint Simpson weights `L/6, 2L/3, L/6`, so
the explicit solver retains a diagonal damping operation. Only edges labeled as
the fixed outer square receive this term; obstacle edges stay reflecting. At a
corner, the two incident edge integrals both contribute to the corner node.

Reflecting remains the startup default. Each fixed-box side is one logical span.
Its prescribed Neumann value is the outward flux `k ∂n u = q(t)` and contributes
`∫Γ N_i q(t) ds` with the same quadratic Simpson weights used for impedance.
Dirichlet data strongly sets every boundary time level to `g(t)`, including
initialization and operator-transfer commits. Both use
`offset + amplitude sin(2π f t + phase)`, so constants need no separate mode.
Adjacent Dirichlet sides must carry identical signals at their shared corner;
contradictory assignments remain an invalid editable draft. At a mixed
Dirichlet/second-order corner, the essential value wins and auxiliary memory is
disabled on that node.

Switching modes assembles a replacement
operator on the same mesh, constructs a direct identity quadratic transfer without
spatial point location, then uses the normal GPU transaction to carry displacement
and velocity into the new centered time levels. The operator and live field
therefore change together without invoking the mesher.

Finite Gaussian-packet measurements at parent h=.08 for wavelength .4 give
energy-equivalent amplitude reflections .0257 at normal incidence and .0981 at
30 degrees; the continuous first-order plane-wave values are 0 and .0718. At
parent h=.04 and wavelength .2 the measured values are .0106 and .0978. The
normal wavelength-.4 case remains finite through t=10 with 4.80e-5 of its initial
energy. These measurements include finite-beam bandwidth, diffraction, spatial
discretization, and time integration, so the plane-wave values are context rather
than exact expected outputs.

The implemented higher-order option is the second-order Engquist-Majda rational
condition

```text
∂n u = -u_t/c + (c/2) ∂t^-1 ∂ss u,    ψ_t = u.
```

With material stiffness `k`, its semidiscrete weak equation is

```text
M u_tt + K u + CΓ u_t + (k c/2) KΓ ψ = f.
```

`KΓ` is the one-dimensional tangential stiffness assembled on the quadratic outer
edges. In endpoint/endpoint/midpoint order its local matrix is
`[[7,1,-8],[1,7,-8],[-8,-8,16]]/(3L)`. It is symmetric, positive semidefinite, and
annihilates constants. The global outer-corner DOF is shared by both incident
sides, so their tangential terms couple at the corner instead of evolving two
unrelated endpoint states. The added stored energy is
`ψᵀ (k c/2 KΓ) ψ / 2`.

The centered displacement step reads `ψ[n]`; after predicting `u[n+1]`, a
trapezoidal update advances `ψ` by `dt (u[n] + u[n+1]) / 2` on DOFs assigned a
second-order condition. CPU and WGSL use the same update. The GPU packs interior and boundary
coefficients together, keeping a single CSR gather. At the production timestep,
a 10,000-step regression remains finite and decays.

A geometry transaction with an unchanged auxiliary boundary layout interpolates
`ψ` with the same quadratic map used for displacement and velocity. A boundary-law
change clears auxiliary memory conservatively; unchanged same-mesh settings still
bypass spatial point location. The tangential operator is assembled only on spans
assigned the second-order law, and essential values take precedence at mixed
junctions.

The rational form follows the [original Engquist-Majda absorbing-boundary
construction](https://doi.org/10.1090/S0025-5718-1977-0436612-4). Further
Hagstrom-Warburton orders remain candidates, but their finite-element realization
must retain symmetric operators suitable for the explicit solver; see the
[symmetric FEM formulation](https://doi.org/10.1016/j.cma.2024.117579) and the
[complete radiation-condition hierarchy](https://doi.org/10.1137/090745477).

IGA follows the interior-region, assigned-boundary, and expanded spline-editing
product work. Start with an untrimmed single patch. Boundary splines alone do not
supply an interior parameterization. Evaluate quadrature, mass treatment, spectral
behavior, timestep restrictions, and transfer on their own merits before extending
to multipatch or trimmed geometry.

## Reference starting points

These are implementation/research references, not numerical dependencies:

- [Bevy WebGPU examples](https://bevy.org/examples-webgpu/)
- [bevy_egui documentation](https://docs.rs/bevy_egui/)
- [DUNE wave equation with mass lumping](https://www.dune-project.org/sphinx/content/sphinx/dune-fem/wave_nb.html)
- [WGSL specification](https://www.w3.org/TR/WGSL/)
- [Hagstrom-Warburton complete radiation conditions](https://doi.org/10.1137/090745477)
- [Mass lumping and outlier removal for complex IGA geometry](https://arxiv.org/abs/2402.14956)
