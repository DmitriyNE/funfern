# The IGA scene — a design note, 3 October 2026

**Status: postponed on 4 October 2026. Nothing here is built, and the
document kind below will not be built.** The note follows the
[feasibility spikes](funfern-iga-feasibility-spike.md), whose verdict is that
single-patch isogeometric analysis (IGA) is feasible in this architecture
but only as its own kind of document: a rectangle deformed through a control
net, with material regions as parametric rectangles bounded by knot lines.
The note says what that document would be, what it reuses, what is new, what
it buys and costs, and proposes milestones. It is kept as the record of that
option; the decision is in the next section.

## The decision

The user set the bar any IGA in this product has to meet, and this design
fails both halves of it.

- **All or nothing.** IGA is incorporated only if it does everything the
  triangle solver does and most of it better. A second document kind whose
  scenes are boxes with rectangular regions, and into which today's gallery
  does not transfer, is a partial product.
- **Automatic.** Usage stays roughly as it is now: the user draws curves and
  regions, and the solver builds its own discretization, as the mesher does.
  Patch editing, the net and the knot lines, is exposed only in an advanced
  mode. In this design it is the main way of working.

An automatic IGA has to take today's documents and build the spline space
itself. The routes known so far, none of them measured to the bar:

- **Automatic multipatch.** Each region decomposed into four-sided patches
  with C⁰ joins. A robust quad layout of arbitrary curved regions with holes
  is a research problem; the layout's quality sets the stable step (a
  single patch over a disk lost 50 to 100 times, spike 3); and the layout
  jumps under a drag, so a live edit becomes a whole-scene handoff.
- **Immersed curves.** Curves cut through a background patch. Interfaces
  are second order (spike 4), below the conforming triangle, and cut cells
  shrink the step unless stabilized. It loses on the optics scenes.
- **Smooth splines on the triangle mesh**, such as Powell–Sabin quadratics
  on the mesher's own triangulation. It keeps the documents, the editor and
  live edits, so it meets the bar on paper; lumping, dispersion, stencils,
  walls and interface continuity are all unmeasured. Filed in plan.md under
  Later experiments as a spike series of its own.

And every route degrades from the flat box, which the spikes measured at
about twice the triangle's speed at equal accuracy. So IGA is postponed
until a route clears the bar. The spikes' largest transferable gain, the
fourth-order integrator, goes ahead on the triangle solver as M0 below; the
spike module and its examples stay as reference.

## The box-patch option in one paragraph

The spikes ruled out the two cheap routes. A single untrimmed patch cannot
wrap a smooth closed boundary without losing fifty to a hundred times in the
stable step, so the patch is the scene's own box. And a free curve inside the
box, the present product's whole vocabulary, maps onto the patch only as an
immersed interface, second order, about 1% of a reflected energy at six
spans per pulse width, which is below the conforming triangle. What is left
is a new scene kind whose geometry is a deformed box and whose regions are
parametric rectangles, curved through the control net. On smooth media it is
nine times more accurate than the triangle at equal dofs and about half its
time at equal accuracy; its edits rebuild 3% of the samples with no
remeshing and no artifact; its refinement is exact; and with the
fourth-order integrator it is spatial-error-limited at the stable step. The
cost is a second discretization through most of the stack: document,
persistence, editor, runtime, device layout, probes, transfer and display.
The choice is whether a second scene kind with that profile is worth that
stack. Two things fall out regardless and are worth doing first: the
fourth-order integrator for the triangle solver, and nothing else.

## What the spikes fixed about the numerics

These are settled by measurement and the note takes them as given.

| choice | what | why |
| --- | --- | --- |
| basis | quadratic tensor-product B-splines, C¹ across knot lines, unclamped (uniform knots running past the walls) | quadratics with two sweeps are 9× the triangle's accuracy at equal dofs; cubics pass on accuracy and lag on cost; the clamped corner function halves the stable step |
| C⁰ knot lines | an interior knot repeated to the degree, per direction, where a region boundary has a stiffness jump | a stiffness jump kinks the field; without the knot the interface is second order |
| quadrature | Gauss 2 × 2 per span, `b` at those samples | no spurious modes (one point has them), enough for two sweeps |
| mass | row-sum lumped diagonal plus two Jacobi sweeps toward the consistent mass through the sample tables | symmetric positive definite, matrix-free, each sweep two orders in the wavenumber; lumping alone is 280× worse than the triangle |
| time | the modified-equation Störmer step, `b` drifting on `P (Q − dt²/12 · K u)` | fourth order for one extra pass a step and a √3 larger step; 13% of the throughput |
| geometry | a bicubic B-spline surface with a control net over the parametric box, Jacobians folded into the sample weights and gradients | a deformed box keeps 0.82 to 0.87 of the flat box's step; a Coons patch over a smooth curve keeps 1 to 2% |
| regions | parametric rectangles in knot units, one `Region` each, material read at the samples | an interface on a knot line with the right continuity is high order |
| edits | a control-point move rebuilds the `4 × 4` geometry spans it touches; the field's coefficients move with the geometry; the flux keeps its covariant components | 3% of samples, 4% of dofs, no artifact (4e-10) |
| refinement | span halving; the field projected into the fine space; the flux carried as the gradient of its potential | exact: the flux stays a gradient to 6e-15 |
| handoff | carry the field and the flux at the field's own instant; the new stepper staggers it | the half-step flux cost 3% when the step halved |

Open numerical points, none of them blocking a first milestone:

- A field-dependent primary law (Kerr or saturable in the mass row) is a
  per-node Newton map `Q = P(u)` on the triangle solver, which assumes a
  diagonal mass. Under sweeps the map is not closed per node. First
  milestone: field laws and van der Pol on the lumped diagonal only, or
  refused on a patch. A time-driven mass is fine: the sweeps read the density
  at the samples where it stands.
- A handoff that changes the step under the fourth-order integrator carries
  an `O(dt²)` inconsistency, since the modified field depends on the step.
  Measured at 3.9e-3 against 6e-4 for no handoff at all. Acceptable; a
  re-staggering through the new step's modified field would remove it.
- Dirichlet and signal sides. The unclamped edge functions are not
  interpolatory, so a wall value is a constraint across two rows of
  coefficients, not a pin. The clamped basis makes the edge row
  interpolatory and reuses the present pin machinery, at the cost of the
  corner outlier on the clamped sides. Proposal: clamp only the sides that
  carry Dirichlet or signal walls; measure the step on a patch clamped on
  one or two sides before committing to it.
- Outgoing walls on patch edges. First order is a diagonal damping on the
  edge dofs' lumped edge weights, as today. Second order reuses the trace
  graph if the trace is a node subset, which the clamped edge row is; on an
  unclamped edge the trace would need a restriction operator through the
  prepare and finalize stages. Plan §10's wish to revisit second-order
  conditions on curved spans does not arise on a box patch.

## The document

A `PatchScene` beside `TopologyScene`, holding:

- the control net: degree three, `m × m` geometry spans, `(m+3)²` points,
  starting as the Greville points of the box (`GeometryMap::flat_surface`);
- the solution grid: `n × n` uniform spans, degree two, with a set of C⁰
  knot indices per direction;
- the material blocks: parametric rectangles in knot units, each a
  `Region { id, material, frame }` as today, so the materials list,
  `Material`, the laws and the catalogue are reused unchanged, and
  `set_region_material` and the region listing work as they are;
- the physics skin, `PhysicsModel`, reused unchanged, including the
  conversion of materials on a skin change;
- the four outer sides, `OuterBoundaryConditions`, reused unchanged on the
  four parametric sides;
- volume sources per block; the point source, `PointSource`, with its
  physical position located in the patch; probes of the point, segment and
  disk kinds as they are, with boundary and region-area targets later by
  knot line.

Persistence. Version 22 is the last break, and the policy allows an additive
key whose default means what files without it meant. The file gets a
top-level `patch: Option<StoredPatchScene>`, default `None`, skipped when
absent. A patch file still carries a `model` because that key is required:
the default topology scene, which compiles. The reader checks `patch`
first. Every existing file, fixture and frozen scene is untouched, and an
older build refuses a patch file with an error rather than opening a blank
scene, because the file struct denies unknown fields. The alternative, a
version 23 with a tagged scene enum and a wrapping migration, is cleaner
and the policy permits it, but it is a shape change the user has asked not
to make without need; the note recommends the additive key.

The gallery. The catalogue is a vector of `TopologyExample` with a
`TopologyDocument` inside, and a dozen tests loop over it assuming curves.
Patch examples go in a parallel catalogue with their own round-trip and
authoring tests, and the 45-count test stays as it is. The written rule
that every gallery scene is authorable in the UI applies to patch scenes
too: control net, knot grid and material blocks must all be editable.

## The editor and the viewport

`TopologyEditor` compiles a topology on construction and derives its ids
from curves; generalizing it is awkward. Proposal: a `PatchEditor` with the
same shape (draft and accepted, revision, undo and redo of a model snapshot,
`begin` / `changed` / `commit`), and `Playground.editor` becomes an enum over
the two with a small trait for what the chrome needs: revision, undo and
redo, the save and load surface. The runtime request takes a prepared
patch instead of a compiled topology.

Tools and handles:

- a control-net handle `(ix, iy)` as a new handle kind, dragged through the
  present handle-drag path, each move going to `with_moved_control` on the
  draft and committed as a handoff that carries the field, as the spike
  does; the net is drawn as a lattice with handles, toggled from the view
  panel;
- knot-grid commands: insert a knot line, remove one, toggle a line C⁰;
  insertion is exact refinement along one direction and reuses the
  field-and-potential transfer;
- material blocks: the materials panel lists blocks where it lists faces
  today, and a viewport click picks the block under the pointer. That needs
  a physical-to-parametric inversion, Newton on the geometry map, which the
  spike module does not have and which probes and the point source need too;
- the draw palette, curves, junctions and spans do not exist on a patch; the
  Edit tab shows the net and the grid instead.

Display. Control coefficients are not point values. The patch builds a
display lattice of parametric points, four by four per span or by zoom,
mapped through the geometry, with values `Σ N_i u_i` from the swept field,
and feeds `FieldPaintTopology` with positions, indices and values as it is;
the rasterizer is generic. Categorical overlays color by block. The mesh
toggles in the presentation settings are hidden for a patch and a net and
grid toggle appear.

## The runtime and the device

A `PreparedPatch` beside `PreparedTopology`, holding a `SplinePatch` with
its materials sampled through `Material::evaluate_static` at every sample
(the same evaluation the triangle operator uses per sample), the forcing,
the probe stencils and the transfer. The spike module is promoted from
spike grade to a core module with the application's error types, and its
box-only pieces (separable projection) stay as test helpers.

The transaction and commit machinery is generic and is reused as it is:
jobs, generations, install and handoff, receipts, the clock, the event
system, accounting and the failure codes. What is new is everything that
produces an operator, a transfer map or a stencil, which today reads the
triangle mesh directly; there is no interface boundary to plug into, so the
patch gets parallel types rather than generalized ones.

The device layout, from the canonical shader as it stands:

- A sample is 112 bytes with seven node indices and seven curls in fixed
  lanes, and `sample_node`, `sample_curl` and the drift loop are hard-coded
  to seven. A quadratic patch sample has nine nodes, and the mass pass also
  needs the nine basis values. Rather than a 192-byte sample, the tensor
  structure allows a compact encoding: a sample stores its span `(ix, iy)`,
  its Gauss point and its Jacobian, and nodes, curls and values are derived
  from two one-dimensional tables. The IGA path gets its own sample layout
  and its own drift, with the loop bound a constant of that shader.
- The force gather is node-parallel through table ranges and is stencil
  agnostic; it is reused. The per-sample constitutive, temporal, loss,
  secant and short-wave machinery is reused once its record pointer, which
  lives in a spare node-index lane today, has a lane of its own.
- The mass pass is new: a sample pass `u_s = Σ N_i u_i` scaled by `w_s ρ_s`,
  and a node pass `(M u)_i = Σ_s N_i u_s` followed by the Jacobi update
  `u ← u + D⁻¹ (Q − M u)`, two sweeps, so four dispatches per field
  evaluation. The node pass needs a per-node range of `(sample, N_i)`
  entries, which takes the node stride from 96 to 112. The swept field lands
  in the lane the nonlinear path already writes before the drift and the
  drift already reads, so the drift is unchanged. The eight storage
  bindings are a hard limit; the new tables share the existing table buffer.
- The fourth-order integrator is one more gather, scatter and field
  evaluation before the drift, the same passes again.
- Every per-node use of `Q / m` becomes a read of the swept field: the drift
  field, pins, the damping ratio, the energy lanes, the trace's `1/m`, the
  grid filter and the probes. These are small edits each, many places.
- Boundaries: Dirichlet and the trace graph on clamped sides as above;
  first-order damping on edge dofs.
- Transfer kernels: the primary transfer is `u` kept and `Q` rescaled by
  the lumped ratio, then re-swept; the flux transfer is the covariant map
  for a net edit and the potential's gradient for a refinement. Both are
  simpler than the triangle's point location and bins.
- Probes and the far field: patch stencils of nine nodes with value and
  gradient weights at a located point; the area probe integrates over
  samples by block.

## Milestones

Each ends at a gate the user can judge. M0 is independent of the decision.

| milestone | what | gate |
| --- | --- | --- |
| M0 | The fourth-order integrator for the triangle solver: a design check of the implicit boundary kicks, loss stages and nonlinear maps inside the production step, then the linear path on CPU and device, then the rest or a documented exclusion. | The app's phase error at its operating point drops from 0.064 rad to about 2e-4 at t = 10 for about 15% of the throughput; the device suite passes. |
| M1 | The CPU patch path end to end: `PatchScene`, persistence, `PreparedPatch`, materials through the catalogue, point source and probes, control-net and knot handoffs, a headless example. The spike module promoted. | The slab, box-mode and disk measurements reproduce through the application's types; a patch autosave round-trips. |
| M2 | The device: the IGA sample layout and drift, the mass pass, the integrator pass, the handoff kernels, probes. | The device matches the CPU oracle to f32 on the spike scenes; a 96 × 96 patch runs at the predicted fraction of the triangle's throughput. |
| M3 | The UI: `PatchEditor`, net and grid handles, block materials, the display lattice, New → spline patch, autosave and links, a gallery section of three or four knot-aligned scenes. | Plan §10's completion line: an interactive browser IGA example on one patch, with the numbers written down. |
| M4, later | Clamped Dirichlet sides measured and chosen; second-order outgoing on patch edges; field laws on a patch; multipatch for a curved outer wall; trimming. | Each its own spike. |

Relative size: M2 is the largest, M3 next, M1 about half of M3, M0 small.
M1 is the checkpoint: after it the CPU path on a real scene says whether the
document kind feels right, before the device and the editor are paid for.

## What it buys and what it does not

Buys: a smooth-medium solver nine times more accurate than the triangle at
equal dofs and about half its time at equal accuracy; a geometry the user
sculpts by dragging a net, with the wave carried through the edit and no
mesher in the loop; exact refinement; fourth order in time; a flat-box
baseline for every optics scene whose interfaces are knot lines (slabs,
gratings, layered media, Bragg stacks, waveguides along the axes).

Does not buy: free curves. A lens, a disk, a rod lattice or an arbitrary
baffle is immersed at second order or needs trimming, which is a later
experiment. The present gallery's scenes do not transfer; a patch gallery is
written for what a patch does well.

## Recommendation

As written on 3 October: do M0 now in either case, then build M1 and decide
after it with a real patch scene in hand.

**Outcome, 4 October.** M0 goes ahead. M1 to M4 are withdrawn with the
document kind, for the reasons in "The decision" above.
