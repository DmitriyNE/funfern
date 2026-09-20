# Architecture notes

The spline editor, constrained mesher, bounded coordinate-edit repair, enriched
quadratic GPU wave solver, and geometry-edit simulation transactions are
implemented. Outer sides, hole spans, and baffle faces support reflecting or
prescribed Neumann data, prescribed Dirichlet data, and first- or second-order
radiation. Stable material regions, closed and open transmitting interfaces,
explicit material junctions, closed two-sided walls, and open baffles with
independent face laws or coupled thin-gap spans are
implemented. The closed-wall representation is retained for scene compatibility,
but the current editor does not offer it as a normal role. A first spatial
edge-size adaptation transaction is implemented; user wavelength and active
solution indicators drive bounded bidirectional refinement and coarsening.
The same scalar weak form now has explicit Mechanical, electromagnetic TM (`E_z`),
and electromagnetic TE (`H_z`) scene semantics.

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
inspector exposes one of five panels: Edit, View, Simulation, Materials, or Probes. The
solver starts in the running state after its initial operator is committed.
Selection remains contextual rather than introducing separate selection modes.
Draw opens a transient role/primitive popover: holes and interfaces offer Circle,
Rectangle, Polygon, and Spline, while baffles offer Polyline and Spline.
Polygonal tools accept interpolation vertices; Spline accepts control points. A
single `InteractionMode` owns draw, pulse-placement, and probe placement state.
Placement modes are shown over the viewport and survive inspector changes;
constrained finite constructions are deliberately one-shot, while pulse placement
remains active until toggled off, ended with the overlay, or cancelled with Escape.
The point source is manipulated directly through its viewport marker.

The lower-right status control carries a compact performance summary (FPS, solver
steps per second, DOFs, mesh size, and timestep). Detailed performance measurements
are UI-only diagnostics in one draggable, scrollable window with Frame, Topology,
Mesh, Handoff, and Solver sections. It includes a rolling frame-time plot and
aggregate frame statistics. The sections follow one transaction end to end: the
accepted token and the live preparation breakdown, the committed mesh and its
adaptation, the handoff phases (CPU preparation, the short complete-step drain,
GPU upload/admission) with what each transaction reused, and the running
solver's throughput and outstanding step backlog. Preparation carries its own
wall-clock buckets so the cooperative mesh, assembly, and transfer phases and the
remaining synchronous probe and far-field tail are distinguishable. Preparation and
adaptation errors open diagnostics and light the warning marker;
ordinary rebuilding does neither. Validation, mesh, and handoff stages share the
middle of the status strip, while successful document and handoff actions appear as
short-lived notices.

Probe definitions are application-document data with stable IDs and participate in
the same bounded snapshot history as geometry. They are serialized in scene files,
shared links, and recovery data; sampled traces and floating-window layout remain
transient. The Probes inspector owns creation, configuration, and the receiver list,
while each receiver has one independent closeable readout window. The target model
covers points, independent straight line segments, geometry-attached boundary runs,
free disks, and stable material regions. The far-field monitor is singleton
document state rather than another selectable probe: it is enabled with one inset
distance and derives its contour from the outer domain. Point readouts share one
pan/zoom time window across their independently hideable observable sections; Live
mode follows the newest solver-clock sample. Probe timestamps come
directly from the solver's transferred absolute clock. The host handoff offset is
only used to map the continuous accepted-step total across timestep changes; a
fresh install establishes a new rate baseline instead of treating its absolute
counter value as progress.

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
is a hole, a transmitting material interface, or a two-sided wall. Closed regions
form a validated containment graph. Open transmitting dividers form a planar graph
with stable interface, breakpoint-node, and junction IDs. Each interface span stores
coherent left/right regions; attached breakpoints are C0, while outer junctions
retain their side and normalized side coordinate. Mesh triangles carry their owning region/material ID,
while constrained edges carry their logical boundary/span ID and side information.
Those labels survive remeshing even though numerical indices do not.

The replacement topology kernel removes those stored object roles. It stores one
open/closed curve type, optional shared topology vertices at spline breakpoints,
and a behavior on each stable span: transmitting or separated, with coupling
orthogonal to the two side conditions. A bounded compiler derives directed
half-edges and planar faces. `TopologyScene` keeps materials and stable regions
beside explicit authored face dispositions. Each disposition uses an oriented
outer-side or curve-span anchor to recover its snapshot-local `FaceId`; it assigns
either one `RegionId` or deliberate exclusion. A missing disposition is an invalid
draft rather than an implicit hole. Compiled face IDs never enter persistence or
history. Hole, material-region, divider, and baffle choices are initial curve
configurations rather than geometry classes.

Face exclusion also resolves the effective boundary law. An authored transmitting
span between two active faces remains conforming. If exactly one neighboring face
is excluded, the active side compiles as a homogeneous Neumann wall while the
authored span remains transmitting; reactivating the excluded face therefore
restores transmission without losing intent. A curve with no active neighboring
face is inert and omitted from the mesh. Coupled two-trace laws still require both
sides to be active.

At an arrangement vertex, angular sectors determine finite-element trace identity.
Crossing a transmitting ray joins the adjacent sectors; crossing a separated ray
does not. A free baffle tip has one surrounding sector and therefore reconnects
without a special coordinate test. The compiled snapshot exposes these trace IDs
on both endpoints and sides of every edge. `TopologyMeshPlan` then combines them
with explicit active/excluded face assignments, so meshing and assembly do not
infer trace connectivity or region identity from point coordinates. This kernel
and projection are implemented alongside the current pipeline while consumers are
being changed over.

During divider drawing, the outer boundary and existing divider curves are attachment
targets. Attaching to the interior of a divider inserts a shape-preserving knot,
raises that breakpoint to C0 without moving the curve, and creates the junction as
part of the same staged document edit.

The headless topology editor represents an attachment hit as either an oriented
boundary anchor or an authored junction plus its selected incident face. Creating
a separator validates both targets against one compiled draft, requires the same
active face, attaches both endpoints, then assigns the single new daughter face in
one document command. Span insertion remaps face anchors and expands boundary-probe
paths before validation. Removal is planned on a copy: the geometry is cut or
removed, dangling junctions pruned, curves that lost a junction promoted to
baffles, and the result compiled; the region merge is then read off that result
as the regions landing on one face rather than inferred from edge adjacency.
Several regions landing on one face require an explicit survivor and remove
probes or sources owned by the dropped regions atomically; regions landing on
several faces at once is refused. The outer domain retains the stable background
region identity; choosing the other side's material transfers that side's
material and dependents into the background identity and removes dependents
belonging to the discarded side.

Two open-curve ends that meet form one curve, not a junction: an authored
topology vertex exists only where three or more arms meet or where a curve
attaches to the outer domain. Welding a loose end onto another concatenates the
splines — the stationary curve keeps its identity and direction, the other is
reversed when the ends demand it, which swaps its separated laws and flips the
side of its face anchors and boundary probes while reversing the probes' paths —
and leaves a plain multiplicity-3 breakpoint that continuity edits may smooth. A
loose end dropped on a vertex-less breakpoint makes that breakpoint the junction
without inserting a span, and a removal that leaves exactly two open-curve ends
at a junction fuses them in the same command. The arrangement compiler rejects
coincident endpoints and T-contacts that carry no authored vertex, so an authored
vertex or a single joined curve is what makes a touch compile, not only what
keeps it stable under editing. Promotion to a baffle follows the compiler's own
rule: an open curve is demoted only when a free tip's adjacent span transmits, so
a curve that keeps a transmitting span between two faces keeps it. A closed
curve's seam is node zero, which the span that ends there reaches at the period
rather than at zero, so every parameter comparison against a closed curve's nodes
wraps; without that the seam's junction is invisible from one side and a contact
there reads as accidental.

A periodic cubic needs at least four control points, and a closed curve's control
count is the sum of its breakpoint multiplicities, so smoothing a knot on a small
loop can run out of room. That floor is bought past rather than enforced: an
operation that needs controls refines the curve by exact knot insertion first,
inside its own command, so the curve does not move and the gesture stays one
history entry. What refinement cannot buy is the opposite wall — sharpening to a
corner and attaching a junction both spend controls against a 128-control ceiling
— or a topological rule: a node carrying a junction is always a C0 corner.

Viewport interaction uses the authored identities directly. Its transient
selection is either one curve control or authoritative topology vertex, or a set
of stable outer/curve spans. Adaptive display samples retain their originating
span ID, and screen-space hit testing prefers junctions and controls over curve
strokes. Shift toggles spans, whole-curve selection expands through the authored
curve, and marquee direction selects by enclosure or crossing. Rigid span
transforms operate on spline support controls only when a partial section is
isolated by C0 breakpoints. If only some arms incident to a shared junction are
selected, the transform is rejected with the missing incident span set; a complete
selection moves the authoritative vertex once and synchronizes every attached
breakpoint. Attachment hit testing may use snapshot-local faces to filter the
gesture, but the stored command target remains an oriented stable anchor or
topology-vertex ID.

Material interfaces use a conforming scalar field with three base material
properties. Mechanical scenes interpret them as density `rho`, stiffness `k`, and
damping `d`. EM scenes store permittivity `epsilon`, permeability `mu`, and reduced
loss rate `alpha`, then compile TM to `(mass, stiffness, damping) =
(epsilon, 1/mu, epsilon*alpha)` and TE to `(mu, 1/epsilon, mu*alpha)`. Both give
`c = 1/sqrt(epsilon*mu)` and `Z = sqrt(mu/epsilon)`. An optional directional
axis-ratio field `a >= 1` resolves in the region's orthonormal material frame to
`A_local = k diag(a, 1/a)`. Its determinant remains `k^2`, its principal wave-speed
ratio is `a`, and `a = 1` is exactly the prior isotropic model. Mechanical, TM, and
TE compilation all use this same flux tensor, so changing the physics skin leaves
the directional law unchanged. Each scalar
coefficient is either constant or a bounded compiled expression over local `x`,
`y`, `r`, and `theta` plus material-level named parameters. The local coordinates
always have world units. Their frame contains only an origin, an angle, and a
world/follow-region attachment policy; it is rigid and orthonormal, with no scale.
A whole-loop similarity transform carries an attached origin and orientation, but
uniformly scaling geometry does not change the spatial wavelength of its material
profile. Non-rigid control edits leave the frame unchanged. Coefficients are sampled
at volume and boundary quadrature points and at probe positions, so assembly,
energy, directional boundary impedance, and timestep limits use the same material definition. Invalid
runtime values abort the candidate operator and leave the current solver running.
Material formulas parse through a private expression tree before compiling to the
bounded postfix evaluator. Ordinary edits retain their original source verbatim and
do not run algebraic simplification. A Mechanical/EM physics switch transforms the
tree using `epsilon = 1/k`, `mu = rho`, and `alpha = d/rho`, or the inverse mapping,
then applies only finite constant folding, neutral-element removal, double-reciprocal
elimination, and exact structural product/quotient cancellation. This preserves local
wave speed, characteristic impedance, and normalized damping rate without formula
growth under repeated switches. The complete material library converts before the
document transaction begins; a formula-limit failure leaves the document untouched.
Internal walls require separate traces on their two sides and therefore a
topology/DOF operation, not merely a coefficient label. Material-value changes
reuse the current mesh and enter the same transactional operator replacement path
as boundary changes. The View inspector can render density, stiffness, damping,
wave speed, impedance, or anisotropy on the quadratic display topology. The
anisotropy view colors `log(a)` and draws sparse marks along the fast principal
axis. Overlay vertices
include region identity so a shared interface retains a sharp coefficient jump. A
revision-keyed cooperative cache evaluates all properties once, allowing property
switches without resampling; invalid samples are localized in red and never affect
solver acceptance. Profile placement belongs to a region and has both numeric
controls and a rigid origin/rotation viewport gizmo. Solution AMR samples spatial
coefficients at element and boundary quadrature points. Its strong residual includes
the tensor-divergence term, recovered and interface fluxes use `A grad(u)`,
and each element's active-frequency ceiling uses its slowest sampled wave speed.
The element stiffness gradient is the piecewise-linear reconstruction from vertex
samples, keeping the estimator dependency-free and convergent under refinement.

A distributed source belongs to one region rather than to its reusable passive
material. It combines a signed scalar profile over that region's rigid local frame
and source-local parameters with `bias + amplitude sin(2 pi frequency t + phase)`.
Quadratic element mass weights assemble the body acceleration into mass-normalized
nodal weights. Point sources, distributed sources, and prescribed boundary data all
own the same dependency-free `TimeSignal`; the initial harmonic variant keeps offset,
amplitude, frequency, and phase together and leaves waveform extension independent
of each spatial carrier. Compilation is resumable, and temporal-only source edits
replace the forcing buffer while retaining compiled spatial weights, the mesh,
operator, solution levels, solver clock, and probe history. Conforming material
junctions share one scalar DOF while incident triangle regions retain their own
element coefficients.

Boundary conditions attach to logical parameter spans rather than individual mesh
segments. Sampling copies a span assignment onto every resulting constrained edge.
Knot insertion can split an assignment exactly; removal, seam movement, and span
merging need explicit inheritance rules and must reject ambiguous drafts. Junction
nodes and higher-order auxiliary fields belong to this semantic boundary graph.
The editor represents an outer edge, hole span, or baffle span/face with one
transient boundary-selection type. Viewport hit testing creates that selection and
one inspector dispatches to the conditions supported by its target; selection is
excluded from scene files and document history.

The replacement topology document has a strict scene JSON version 22. It is a
deliberate compatibility break: its decoder accepts only version 22, rejects unknown
fields and malformed or excessive data before replacement, and has no versions 1–21
adapter. Both draft and accepted authored topology are stored, so an invalid but
structurally sound draft survives a round trip while the accepted scene must compile.
Stable curve, span, topology-vertex, region, material, probe, and source references
are checked during decoding. Compiled faces, mesh plans, caches, history, camera,
selection, panel state, floating-window positions, and solver state are omitted.

A topology `Document` owns one undoable model plus `PresentationSettings`. The model
contains draft and accepted scenes, probes, point-source configuration, and far-field
settings; it is the exact snapshot type stored by Undo/Redo. Presentation contains
the View inspector's visibility, field-intensity, and material-overlay controls,
and the solver's speed ceiling, which travels with a document for the same reason
the view does. It
travels through scene files, examples, shared links, and recovery, but stays outside
history, so model edits never rewind the user's current view. Loading validates the
accepted scene, reseeds every stable-ID allocator from both snapshots, and starts
with empty history. The eight built-in examples are authored directly as topology
documents and exercise geometry, sources, probes, spatial materials, EM modes,
far-field settings, and presentation presets without a legacy conversion layer.

The production application reads and writes only version 22, and that is the only
decoder in the tree. The pre-cutover `editor` and `persistence` modules were
removed once the value codecs they held — physics, material coefficients, time
signals, wall and face conditions, splines, presentation, sampling presets —
moved into `topology_persistence`, beside the schema that is their only caller.
There is no compatibility adapter and no migration code: a file that is not
version 22 is turned away by its version header, and version 22's own decoder
accepts only the tagged value shapes it writes. Several presentation keys carry
serde defaults so that a version-22 file written by an earlier build still loads,
and the retired adaptation-target flag still migrates to the material overlay it
became; those are the only tolerances inside the version.

Versions 1 to 21 are recorded below because they explain the shape of the current
file, not because anything still reads them. Version 21 added open
material-interface splines, stable breakpoint nodes, per-span left/right regions,
and explicit interior/outer junctions. Version 20 added the optional material
axis-ratio field and anisotropy presentation overlay. Version 19 moved the
editable axis-aligned domain into both draft and accepted scenes. Version 18
added the scene physics model, polarization, explicit electric/magnetic wall
variants, and a tagged material law whose serialized property names follow the
active physics; it also persisted the derived vector-overlay mode, its then-current
smoothing flag, density, and gain. The compatible version-22 presentation record
now writes that retired flag as false and separately stores optional arrow AC
coupling. Version 17 introduced the tagged shared time-signal
representation and moved the point source into the numerical core alongside its
spatial carrier. Version 16 added presentation settings. Version 15 added
region-owned volume sources and their profiles, parameters, and harmonic signals
to both draft and accepted scenes. Version 14 stored constant/formula coefficient
variants, material parameters, and every region's material frame.

File loading, bundled examples,
crash recovery, and shared links all pass through the same structural decoder and
bounded accepted-scene validator before replacing the editor document. Examples
carry names and descriptions and render their thumbnails directly from the accepted
spline geometry. Spatial-material examples overlay a cached, bounded sampling of
their initial property view in the thumbnail. Each catalog entry supplies a complete
document and can include a point source or driven boundary, probes, and an initial
material-overlay preset. Opening one is a single model-history action, installs its
presentation preset, and starts a fresh zero field; it does not transfer the field
from the previously open scene. Ordinary edits and AMR retain their normal
field-preserving handoff.

Physics is part of the undoable `Scene` and operator equality, while geometry
equality deliberately ignores it. A physics or polarization change reassembles
the presentation-specific coefficient roles on the existing mesh and transfers
the live canonical state at the same accepted-step boundary as other generation
edits. Point and line histories are cleared because their labels and physical
meaning change; the evolving field is not reset. EM wall semantics are explicit
at the scene layer: PEC is zero `E_z` for TM and zero normal `H_z` flux for TE;
PMC is the dual. Assembly and the static-linear AMR adapter resolve those
semantic variants to canonical prescribed/free ownership before numerical work.

The production solver stores integrated nodal primary flux `Q` and an independent
two-component complementary flux `b` at six quadrature samples per element. A
consumer obtains `u=Q/M`, evaluates the physical complementary field `c=B^-1 b`,
and uses `orientation*u*R*c` for directed energy flow. Mechanical, TM and TE are
presentations of this same state. No skin reconstructs a transverse field from a
bulk potential or inverse derivative, so stationary complementary modes remain
part of the physical state across handoff and remain visible to direct probes and diagnostics.
The complementary-field arrow view can explicitly AC-couple its presentation at
the old 0.08 Hz corner; this subtracts only a slow per-arrow display baseline.
Its lazy state is keyed by physical mesh element, not screen bin, so panning does
not reinterpret a spatial jump as temporal signal. State is kept only for elements
that have actually been sampled in the current mesh discretization. A newly visible
element cold-starts at zero output until a second accepted sample supplies temporal
history. A solver-generation handoff on the same mesh and physics retains that
history. Across a same-physics remesh, new screen-lattice samples inherit the
nearest old sample's decayed AC output and rebase their raw input to the
transferred field, avoiding a zero-frame blink without interpreting remap error
as temporal signal. A physics-skin change discards the old identities and
meaning. Fresh zero-state installation also clears it explicitly. Energy-flow
arrows never use AC coupling because their temporal mean is meaningful. Quiet-tail
visibility is applied after each arrow saturates, so a sparse numerical outlier
cannot defeat the overlay-wide fade; sub-pixel residual arrows are not drawn. The
overlay's run peak provides the global quiet reference: below `10⁻²` of that
amplitude, a squared smoothstep suppresses normalized arrows. This prevents
arbitrary directions in f32 residue from being amplified back into view.

The accepted primary field is drawn directly, including a real free-component
constant. There is no display-only mean subtraction or hidden gauge correction.
The paired grid-scale filter is an explicit accepted-state event at its exact
cadence boundary; it is not a presentation transform. It preserves constants and
stationary force-free complementary flux and is not a terminal-silence control.
Because it flips the accepted lane without advancing time, its other lane is the
pre-filter state rather than the endpoint one `dt` earlier. Temporal consumers
must either consume explicit endpoint metadata or skip/rebase that maintenance
boundary. The production AMR adapter skips it; complementary-arrow AC presentation
advances through the explicitly sampled pre-filter endpoint, then rebases its
input to the post-filter state without presenting the zero-duration correction
as a wave.

Every source in a scene is eased in by one shared smooth envelope spanning
`SOURCE_RAMP_PERIODS` periods of the slowest oscillating source. A sine started
from rest at a phase whose cosine is not zero injects a net impulse, and no outer
condition can remove the uniform offset it becomes: a constant lies in the
stiffness operator's null space, and a radiating wall adds damping without a
restoring term, so it is transparent to a static field. Where a wall damps, the
drift settles into a permanent offset; where none does, it ramps without bound.
The envelope is shared rather than per source because a phased array steers on
the phases between its sources, and it is an envelope rather than a phase
convention because the phase is authored and persisted. It multiplies the point
source and the compiled volume sources; prescribed boundary data is not a force
and is left alone. `funfern_core::source_envelope` and `wave.wgsl` carry the same
smoothstep.

Both the scalar field and the vector overlay are drawn against a scale measured
from the field rather than a fixed gain. The shipped examples span a hundredfold
in amplitude, which is wider than the intensity slider's whole range, so no fixed
gain can serve them: at the default seven of the eight painted under a tenth of
full colour, and at the slider's maximum three still did. The scale is a high
quantile of the current frame — the 98th over a strided sample of the nodes for
the field, the 90th over the drawn arrows for the overlay. It rises to a louder
field at once, so nothing is clipped, and falls back by at most a fixed factor a
second. Falling by a factor rather than by a difference is what makes a spike of
any size take the same time to walk off. That factor has to be smaller than the
field's own decay, or the scale merely follows a draining domain down and the
wave looks like it never left; the rate trades the brightness a decayed field
settles at against how long a placed pulse holds the scale, and is set from a
recorded level series rather than by taste. The scale never falls below a
hundredth of the loudest level the run has reached. Below that floor a squared
smoothstep visibility factor takes both scalar colour and arrow length to zero;
flooring the denominator alone still left late f32 residue visible as a
full-domain static pattern.

The scale starts again only when a field replaced with zeros actually arrives —
the frame the upload commits — and never at the moment one is asked for. A reset
or a document load zeroes the field on the GPU, but the outgoing scene stays on
display until its replacement is ready, so a scale cleared at the request
measures that residue and paints it at full brightness for as long as the gap
lasts. Restarting keeps how loud the session has been, because the first frames
of a new field are numerical dust and an instant attack onto a scale with nothing
behind it latches onto that instead; the remembered peak holds the quiet floor
above it. A handoff that carries the field onto a new mesh changes nothing — it
is the same field, renumbered. Keying any of this on the solver's generation is
wrong, because adaptation bumps the generation every second or two and each bump
would drop the scale onto the instantaneous level, which a decaying field shows
as visible steps rather than a steady fall. Field intensity and arrow
gain trim the automatic scale rather than replacing it, and their persisted
ranges and defaults are unchanged; the intensity default of 2.0 places the
reference level at `tanh(1.0)`. `field_auto_exposure` turns the automatic scale
off, which puts the intensity slider back in charge of the whole scale exactly as
it was before; the subdomain centring above is a separate concern and stays on. The View inspector prints the level the colours
are relative to, because relative colours otherwise make a decaying field
indistinguishable from a steady one.

Autosave retains both the accepted scene and any invalid editable draft, writing to
browser local storage or an atomic per-user native recovery file after a short
debounce. On browser startup, a `#scene=v1.…` fragment takes precedence over local
recovery. Its URL-safe payload is zlib-compressed versioned scene JSON, capped before
and after decompression. Once sharing is active, autosave updates the fragment with
`history.replaceState`, avoiding a browser-history entry for every edit. Scene-only
SVG export samples the accepted curves independently of camera and transient UI.

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

The outer domain is an editable axis-aligned rectangle with stable bottom, right,
top, and left side identities. More involved outer-boundary topology remains a
future extension.

## Milestone 1 spline and document model

`funfern-core` is dependency-free. It owns `Point2`, `PeriodicCubicSpline`,
`ObstacleId`, `Scene`, parameter-bearing samples, and structured validation
results. Controls and one period of positive nonuniform knot intervals are f64.
There are 4–128 controls, each associated with its same-index knot. Knots and
controls extend periodically; de Boor evaluates positions and the differentiated
control/knot sequences. Intervals below 1e-12 of a period are rejected as
numerically ill-conditioned. Parameters wrap over one period.

Exact polyline constructors expand each vertex-to-vertex edge into one collinear
cubic Bézier span. Interior or periodic breakpoints have multiplicity three, making
the vertex an interpolated C0 corner while retaining the normal open or periodic
spline representation. A two-control baffle Spline expands its endpoints into one
such exact straight span, and the rectangular tool uses the periodic constructor;
no separate polygon geometry reaches validation or meshing.

The contextual straightening tools have two scopes. Per-span straightening raises
every selected span boundary to C0 exactly, then places that span's four Bézier
controls along its own endpoint chord. Selection straightening treats each already
isolated contiguous run as one chord. Both prepare all affected splines before an
atomic document update, retain logical span indices and assignments, and create at
most one history entry.

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
unreasonably expensive scenes. Contact clearance is `1e-4` of the larger domain extent,
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
supports exact affine transforms. A left-to-right screen-space marquee selects
logical spans fully enclosed by its rectangle; right-to-left selects spans that
cross or lie inside it. Its geometry-kind filter can restrict the operation to
loops, baffles, or outer edges. Replace, Add, and Subtract update live with keyboard
modifiers and all produce the same transient span set used by click selection.
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
transient, as are selection, transform inputs, and snapping preferences. An open
complete curve or a partial spline piece with C0-bounded support can be straightened
by distributing its active controls along the chord between its fixed endpoints.
This changes only control positions, preserving knot topology, span laws, and
geometry-attached probe spans. Complete periodic curves are excluded. Bulk boundary
edits validate all outer, hole, and oriented
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

Version 22 JSON stores unified open and closed curves, stable spans and shared
topology vertices, oriented region anchors, all assigned boundary laws, materials,
regions, controls, intervals, knot multiplicities, and both draft and accepted
scenes. It also stores sources, probes, far-field configuration, and presentation
settings. Versions 1–21 are intentionally rejected by this production decoder.
Serde and rfd live only in the app crate. Round-trip f64 parsing preserves exact
stored values. Files have a 2 MiB limit, strict fields/version/domain, finite
values, and scene/control-count checks. A prospective load validates its accepted
scene incrementally while the existing document stays intact. Invalid draft
geometry is allowed. Failed parsing or validation never replaces the document.

The viewport and panels share egui's event routing and logical-pixel coordinates.
Geometry is drawn with the egui painter on Bevy's wgpu device. Input gestures
start in the viewport; panel/text capture prevents accidental geometry edits.
Mouse and touch share the same feature-hit priority, with larger invisible touch
targets. Empty one-finger drags use the ordinary directional marquee, while the
explicit Area-select mode supplies persistent replace/add/subtract operations.
Egui's multi-touch centroid supplies cursor-centered pinch zoom and pan; once a
second finger joins, navigation owns the gesture until every finger lifts and any
tentative document drag or marquee is rolled back. Marquee direction distinguishes
fully enclosed logical spans from crossing spans.

Viewport capture has one coordinator and presentation fence for still images and
video. A request remains pending through the menu-click frame, enters preparation
at the beginning of the following UI frame, and only then suppresses transient
editor emphasis and floating windows. Persisted View overlays and the logo remain.
This deferred transition prevents the already-painted Export popup from entering a
PNG or the first video frame. Capture state, framing, and output never enter the
document or history.

PNG export uses Bevy's primary-window screenshot pipeline on the existing wgpu
device, maps the logical central-panel rectangle onto the returned physical image,
and crops before encoding. Video fixes its even-pixel output dimensions at start
and contains a resized viewport within that frame after a window resize. Browser
recording copies the WebGPU canvas viewport into a hidden 2D canvas and gives its
30 FPS stream to `MediaRecorder`; codec selection prefers VP9, VP8, WebM, then MP4.
Native recording keeps one Bevy screenshot readback in flight and passes completed
images through a two-frame bounded queue to a worker streaming raw RGBA into an
FFmpeg H.264/VP9/VP8 encoder. Missing native timing slots repeat the latest frame
rather than changing wall-clock duration. The common status UI, start/stop states,
crop calculation, presentation policy, and error recovery do not depend on the
backend.

Canvas resize/display scale comes from Bevy/egui, and Fit View frames the current
draft domain. JSON excludes the viewport. There is no independent wgpu device, WebGL
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

Mesh construction samples the accepted axis-aligned outer rectangle and every accepted spline. It
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
caused by nearly coincident free and constrained vertices. After each open segment
is recovered, a free opposite vertex that would form a narrow trace sliver is moved
away from the segment only if its complete fan remains positively oriented. The
constraint itself never moves. Minimum-angle refinement is completed before the cut
because a zero-thickness crack tip is a reentrant singular point; the final mesh
reports the resulting tip quality instead of repeatedly refining the coincident
faces. Final verification rejects area that is negligible relative to the
triangle's longest edge.

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

A transmitting curve may end in open space. Such an end divides nothing: the
same face lies on both sides of the span, which carries no boundary condition
and changes no material, so the curve is inert until an end is attached. It is
how a wall is switched off without being deleted, how a divider is drawn before
it is attached, and how a probe is carried on a curve that changes nothing. Such
a chain is topologically a baffle whose two sides are not separated: the face
walks out along it and back, so the plan carries an atom for each direction even
though both name the same face, which is what lets carving follow the rim round
the free tip and lets a probe read either side. Carving leaves any slit - either
kind - out of the cavity polygon and recovers it into the triangulation
afterwards, because a boundary that walks out and back encloses no area and so
has no orientation to be sorted by; only the tail differs, a separated slit
being cut apart and a transmitting one only labelled. Every reader of that list asks
whether some atom matches rather than summing over it, so the pair costs nothing
elsewhere. A separator only receives the material chosen while drawing it if it
encloses a face there and then; a face enclosed later inherits the region it was
cut out of.

An open curve belongs to one face, and the editor reads which one from the
curve itself: the faces sampled along the drawn path, falling back to the sides
the attachments named and then to the only face the two ends share. A click on a
boundary names the boundary, not a side of it - which side it reports follows
from the pixel the pointer landed on, and near a breakpoint that is the far side
often enough to refuse ordinary work. Attachments therefore offer every face
they could belong to, and the path settles which one they do.

The unified topology path has its own full-rebuild baseline. `TopologyMeshPlan`
supplies directed face cycles, stable curve/span labels, and snapshot-local sector
trace IDs; the triangulator no longer infers region ownership from object classes.
Transmitting sides share the same constrained vertex chain. Separated free or
attached baffles are recovered into the face mesh and cut into the exact trace
sectors, including distinct coincident vertices at an outer attachment. At a
branched separated junction, recovery first installs every incident run; a final
fan pass crosses transmitting edges, stops at separated edges, and duplicates the
junction vertex once per compiled sector trace. An
unchanged plan reuses its mesh, and changing only a boundary law does not remesh.
Reuse carries the mesh's trace ids forward, so it holds only while both plans
name the same traces. The arrangement issues one id per sector, so switching a
span between transmitting and separated renumbers every trace from there on even
when the atoms come out identical - which they do when the far face is excluded
and the span is walled either way. The atoms line the two numberings up, since
the same boundary in the same place ends at the same two topological points, and
the reused mesh is rewritten through that map or rebuilt when no such map exists.
A slit - a separated span whose two sides face the same face - is left out of
the polygon and cut back in after the face is triangulated, so removing it may
break the face cycle it sits in into several closed stretches: a slit bridging
two holes, joining a hole to the outer boundary, or meeting itself is walked
twice by a cycle that runs through both of the loops it joins. Each stretch
becomes its own polygon, and outer versus hole follows the orientation rather
than the order, counter-clockwise being the outer boundary, since such a split
can produce one of each from the face's first cycle. A stretch that leaves a
junction through one sector and returns through another closes on a different
trace at the same point; the two are one mesh vertex while the face is
triangulated, and the final fan pass separates the sectors again.
`TopologyMeshingJob` runs the full baseline cooperatively: bridge search, ear
clipping, legalization, refinement, separated-curve recovery, and verification
are explicit phases, and changing the caller work slice does not change the
published mesh. The two sides of a separated span between two active faces are
distinct chains that must subdivide identically; chain expansion computes their
pieces from the lower parameter upwards for both, and every midpoint split of a
boundary edge also splits the partner edge at the same parameter.

Every plan difference other than a changed outer rectangle, a resolution change
or a requested rebuild is a repair by carving (`TopologyCarveJob`). The plan is a
set of straight atoms, and only the atoms that differ matter to the mesh. Two
atoms are the same when their label, separated flag, parameter range and
endpoints agree; trace ids are reissued by every compile and faces and regions
belong to the triangles, so a face that only changes its material or its id
keeps every atom. Triangles incident to a changed old atom, triangles a changed
new atom passes through, one more ring, and tiny kept islands are removed. The
kept remainder is flood-filled across unconstrained edges; each component lies in
one face of the new topology, so one face lookup relabels it or, for an excluded
face, removes it. A separated curve that a removed triangle touches is rebuilt
whole, so slit recovery always sees complete runs, and a junction whose kept
atoms disagree about its sectors has every atom meeting there rebuilt; the
selection repeats until both sets are stable. Kept atoms' endpoints receive their
new trace ids through the kept boundary edges, changed atoms are expanded with
the mesher's chain expansion, and the rim edges of the removed triangles plus the
new chains are walked into cavity cycles, counter-clockwise ones being
components and clockwise ones holes. The meshing job then resumes at its bridge
state with the imported triangles frozen: legalization, refinement and splits
skip anything that would change one, so the kept part comes out exactly as it
went in, and an element whose circumcenter falls outside the cavity is left as
it is rather than split at its centroid. Refinement and chain subdivision in
the cavity work towards, per removed triangle, the coarser of the edge length
adaptation *requested* there and the edge length the triangle had, capped by the
meshing target. The request is a ceiling and the existing density the floor, for
two reasons. Reading measured sizes back as targets made repeated repairs of one
curve refine the mesh without bound: a refill has to meet frozen rim vertices and
the boundary's own chords with elements smaller than the target, and each repair
took the minimum again. And refining a frozen cavity finer than its rim is
unsafe: the cavity's initial triangulation spans nearly collinear rim vertices
with slivers that the fresh mesher would split away through its boundary edges,
but rim edges can be neither split nor flipped, so refinement pressure leaves
them in the mesh. The indicator's targets are steps from the current size, not
final sizes, so the floor also keeps a repair from out-refining what adaptation
has reached. Refill triangles inherit the request beneath them, ground a moved
hole uncovers has none, and a mesh no adaptation has touched refills at the
meshing target; the application passes its adaptation switch to the carve, and
with it off the band returns to the target regardless of stale requests. A carve
verifies its refill: an inserted element below half the mesher's minimum angle,
or half the worst angle the repaired mesh already had, is a defect, and the carve
fails with the count, the worst angle and its place rather than hand the solver
a collapsed timestep; the runtime falls back to the full rebuild and the panel
shows that message. The mesher's capacity caps bound what a repair adds, not what
it inherits, since an adapted mesh may already exceed them. There is no motion
cap and no patch cap; a
carve that fails falls back to the full rebuild with the reason attached to the
candidate. Because coarsening merges arrangement segments but never splits one,
an outer wall is a single atom per side and a wall attachment move rebuilds the
whole wall band.

Application preparation binds these objects with a `TopologyToken` containing the
document and topology revisions. `AcceptedTopology` owns the authored scene,
compiled arrangement, and mesh plan behind shared immutable pointers.
`TopologyPreparationJob` carries that bundle through cooperative meshing, quadratic
assembly, solution-transfer construction, cooperative volume-source compilation,
ordinary probe stencils, and far-field compilation. A ready candidate does not
replace the active runtime: the caller first uploads it and then acknowledges the
same token through `TopologyRuntime::commit_ready`. Upload rejection, preparation
failure, supersession, or a stale token leaves the previous runtime untouched.
Document-only changes to point sources, probes, and far-field settings preserve the
topology token and reuse the exact mesh and operator. Material and boundary-law
changes retain the mesh but assemble a new operator; coordinate and graph changes
carve the active mesh, adapted or not, in a Repairing phase, assemble a new
operator, and carry the field across a transfer that copies every node outside
the band exactly. The prepared topology carries the carve report, or the reason a
carve fell back to the full rebuild, for the Performance panel.

The enriched-quadratic CPU operator has a topology-plan assembly entry point.
Materials and local frames come from the region library keyed by each triangle's
`RegionId`; outer segments and curve traces resolve their owning region and law
through the plan's parameter intervals. Transmitting labels verify two-sided
adjacency and add no boundary term. Separated labels assemble the selected
left/right condition, while a thin gap pairs matching curve/span parameter ranges
and adds a symmetric spring between the two trace DOFs. The same face-condition
routine is shared with legacy assembly, so impedance, prescribed data, and the
second-order auxiliary term retain their numerical definitions. Plan revisions
that change only laws may rebuild the operator on the existing mesh; the operator
retains the mesh's revisions for upload and transfer compatibility.

GPU node packing does not encode a bounded list of incident regions. Point and
pulse stencils are filtered on the host, and each packed node carries one boolean
for membership in the current continuous-source region so transfer-time
acceleration can be reconstructed. Moving that source between regions replaces
the membership buffer together with its weights. This admits arbitrary-degree
material junctions without another storage binding. The upload/handoff boundary
therefore accepts topology-plan meshes and operators. Quadratic solution transfer
also consumes the unified mesh lineage: separated nodes prefer source elements on
the same stable curve side, and matching snapshot trace IDs narrow coincident
junction candidates to the same angular sector. Volume-source compilation also
consumes the topology plan directly. It validates carriers against active face
assignments and evaluates profiles and mass density in the assigned region's
orthonormal material frame. The resumable job owns its material/source snapshot,
so editor changes cannot alter an in-flight result. Shared nodes accumulate every
incident driven region; the GPU keeps the existing forcing-weight binding by
storing a fixed node header followed by sparse channel/weight pairs. This removes
the old two-source junction limit without making every node carry the maximum 33
source channels. Scene and topology paths use one material-library evaluator,
including the mechanical/EM coefficient conversion. Core point, line-sample,
boundary-side, disk, and region probe stencils also accept the topology plan and
material model directly. Production probe targets use stable topology span/side
references and the same transaction token as the displayed mesh.

The residual-based AMR estimator has a topology entry point with the same
cooperative work phases as the scene path. Its owned plan/model snapshot resolves
outer laws by side and parameter interval, skips transmitting curve traces as
interior edges, evaluates separated left/right laws, and pairs thin-gap traces by
stable curve/span plus parameter range. Gradient recovery remains keyed by mesh
vertex and `RegionId`, so a shared multi-region junction cannot average fluxes from
different material sectors. The resulting size field uses the triangle's explicit
region for interpolation. Fixed-geometry mesh adaptation also consumes an owned
topology plan. It validates the existing mesh against the plan before changing it,
pins every sampled endpoint and junction-sector trace, and restricts boundary
changes to AMR-created vertices inside one sampled plan segment. Transmitting
constraints remain shared, while two active sides of a separated constraint split
and collapse atomically by curve/span/parameter identity. Every pass stamps each
triangle with the size the field asks for where it lies, read from the finished
mesh. Refinement an adaptation added survives a repair outside the carved band,
the band itself is refilled no finer than it was and no finer than those
requests, and a refilled band is ordinary mesh to the next pass, which refines
or collapses it as the field asks; the application uses the unified topology
path end to end.

The static-linear AMR controller keeps a canonical-energy peak across ordinary
mesh and solver-generation handoffs and resets it only when a fresh zero field is
accepted. Below `10⁻⁴` of that peak energy, relative residual normalization is
dormant: element error indicators become coarsening targets and the reported
whole-field relative error is zero. Wavelength and configured maximum-element
limits are evaluated independently and remain binding. An aligned full snapshot
at a resident-filter cadence boundary is deferred until the next ordinary solver
endpoint because the filter's zero-time lane flip does not provide the previous
`dt` endpoint required by the temporal residual.

Every full canonical state snapshot carries a metadata word in the same GPU
buffer copy: accepted lane and absolute accepted step. AMR and energy decode the
state with that word rather than pairing it with the separately arriving
continuous control readback. The latter remains appropriate for live status, but
cannot identify the lane or time of an asynchronous state snapshot.

The overlay draws all accepted triangle edges, emphasizes boundary labels, colors
elements below 15° amber, and reports counts and extrema. A meshing failure is a
structured diagnostic and never changes the draft, accepted geometry, or history.
The current bridge search is deliberately simple and deterministic. Difficult
valid arrangements may return a topology or capacity error rather than attempting
unbounded recovery.

## Production canonical evolution model

The production linear core advances direct physical state

```text
Qdot = -orientation C^T W J b + integrated sources
bdot =  orientation C u,                 u = Q / M
```

with endpoint/midpoint kick-drift-kick composition. Fixed primary and
complementary loss, prescribed exchange, thin-gap jump history, first-order
outgoing impedance and the passive nonlocal three-state second-order boundary all
participate in the same accepted/candidate transaction. The paired grid filter,
pulses, maintenance and linear-law events use that global acceptance boundary as
well. Its fixed-cadence production filter is resident in the solver command
stream; authored pulses and edits remain host-queued events. A rejected step or
event leaves accepted physical state and its clock unchanged.

The GPU manifest stays within eight storage bindings and owns explicit accepted
and candidate lanes for `Q`, `b`, physical auxiliaries, force caches, energy
accounting and the bounded epoch clock. Every topology/material generation is
prepared while the current one evolves, maps the latest accepted direct state and
physical histories on GPU, validates the complete target, then switches all
resources together. Upload and asynchronous admission do not freeze the source:
the transfer records its exact absolute and epoch-local snapshot steps, subsequent
requests keep the source display live, and the admitted target consumes that short
step backlog from the transferred clock. A rejected target leaves the continuing
source authoritative. The [material-law Stage 4](funfern-material-laws-stage4-report.md),
[Stage 5](funfern-material-laws-stage5-report.md) and
[Stage 6](funfern-material-laws-stage6-report.md) reports contain the equations,
layouts, transfer contracts and measured acceptance results.

## Legacy scalar evolution model

The following centered scalar formulation is retained as a numerical comparison
path and as historical context for older fixtures. It is not selected by any
production skin after the canonical Stage 6 cutover.

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
of that bound. Quadratic assembly rejects a bound that is numerically absurd
relative to the mesh's shortest edge and fastest material wave speed, preventing
a near-degenerate element from silently reducing the timestep by many orders of
magnitude. The f64 CPU implementation is the reference and conserves the scheme's
discrete half-step energy to roundoff in the undamped test.

The f32 GPU kernel stores both committed time levels, a scratch level, and the two
inverse-derivative filter stages in one storage buffer. Each solution-DOF invocation
gathers its CSR row and writes only its own scratch value; a second dispatch advances
the reconstruction and rotates levels. This avoids scatter atomics.

Both stiffness operators are applied in difference form, `Σ_j K_ij (u_j - u_i)`, on
the CPU and the GPU. Every assembled row sums to zero in exact arithmetic, so the
two forms agree, but only the difference form is exactly zero on a constant field
once the rows have been divided by the lumped mass and rounded to f32. The plain
row product leaves a residual of about `1e-7` relative that acts as a permanent
force on the free constant mode of a reflecting cavity and grew a uniform offset
quadratically in time; Dirichlet walls pin that mode, which is why they seemed
immune. A row-sum test covers every assembly path, and an f32 emulation of the
kernel checks that a constant field is held bit for bit.
The operator, state, sources, and controls use Bevy's render-world buffers and its
existing wgpu device. State remains GPU-resident; asynchronous primary readback
supplies the egui field colors, while lower-cadence full snapshots supply the
canonical energy and AMR diagnostics. Vector arrows use a separate compact
display-rate consumer: the CPU chooses one stencil per cell of a quantized,
world-origin-anchored lattice covering the view plus a one-cell apron. Panning
therefore translates the existing interior arrows with the scene and rebuilds
only when a view edge crosses a world cell; zoom changes density only at the
lattice's scale thresholds. The GPU samples complementary field and energy flow
after each rendered solver batch, and only the arrow records cross back. Each readback
carries a GPU-written step marker and compensated two-f32 absolute time. Stale
asynchronous results cannot be mistaken for a newer level, and presentation
filters retain their physical decay across timestep-changing handoffs. Camera
changes coalesce behind at most one in-flight sampling-lattice replacement; the
last completed world-space lattice remains drawable and is reprojected during
pan/zoom or a mesh handoff until its replacement has actually arrived.
Continuous buffer readback is completion-paced rather than frame-paced: a shared
main/render-world token allows at most one staging copy per readback entity to be
in flight, and a completion event rearms it. Requested full-state snapshots use
the token once, while handoff admission polls its pending marker through the same
bounded gate until the transfer pipeline has run. Independently, the host request
clock and render-world encoded clock stay within one 64-step interactive batch of
the last GPU-completed boundary. A slow or background-throttled GPU therefore
loses obsolete wall-time catch-up and reports a speed shortfall instead of
accumulating unbounded command buffers and staging resources.
The wave layout already occupies WebGPU's portable eight-storage-binding budget.
Volume-source weights therefore share the existing point/pulse forcing-weight
buffer. Each DOF has a compact header containing point/pulse weights plus an
offset/count into packed pairs that follow the headers; only nodes touching a
driven region consume pair records. Point, volume, and boundary drives use one
fixed-size GPU signal record with reserved space for later waveform parameters;
the volume signal table shares the existing forcing buffer. No second wgpu device
or extra storage binding is needed.

Point probes compile seven-node primary interpolation plus the six independent
complementary samples of the containing element. A dedicated canonical compute
pipeline samples `u=Q/M`, its accepted endpoint rate, `c=B^-1 b`, canonical energy
density and `orientation*u*R*c` flow at uniform accepted-step intervals. Mechanical
and EM skins expose the same quantities with presentation-appropriate names. The
bounded time-stamped ring has its own portable layout. Host history survives an
ordinary remesh/AMR handoff only when probe identity, skin and canonical semantics
still match; skin changes, reset and fresh scene loads clear it. Locations on
duplicated or material-interface traces are inactive until moved away because
their pointwise physical side is ambiguous.

Straight line and geometry-attached boundary probes use their own bounded
five-binding compute recorder. Sampling
presets pair 32/64/128 uniformly spaced enriched-quadratic stencils with 30/60/120
samples per simulated second; all line probes share a 512-point document budget and
a 64-frame GPU ring. Each valid point records primary field/rate, complementary
magnitude, local canonical energy density and directed flow. Normal flow is
`orientation*u*(R c) dot n`, where `n` is the left normal of the ordered
start-to-end segment for free lines. Boundary probes compile directly from
mesh boundary labels and parameter intervals, select one explicit physical trace,
and carry an outward normal per sample. They store a contiguous spline-span run,
follow topology edits by best-overlap remapping, and retain their probe ID through
solver handoffs. Invalid points remain NaN gaps. Trapezoidal aggregates
integrate only adjacent valid samples and report their covered fraction. Line
definitions survive history, files, links, and recovery while trace data remains
transient like point-probe history.

Mechanical line readouts form a 3×3 quantity/representation matrix: field, signed
normal energy flux, and energy may each be shown versus arclength, as a
time/arclength waterfall, or as an arclength integral versus time. EM readouts add
transverse-field magnitude and relabel directed flux as normal Poynting flux. A
compact checkbox menu controls the combinations and starts with four views active.
Every active view shares the same time window.
Horizontal dragging pans time traces; vertical dragging pans waterfalls along their
vertical time axis.

Area probes compile either a free disk or one stable material region into compact
clipped element contributions. Region targets integrate complete mesh triangles;
disk targets clip each triangle against a bounded, world-space approximation of the
circle before applying a degree-six triangle rule. Each contribution uploads the
canonical primary and six-sample complementary interpolation required by the shared
physical quadrature. A first compute dispatch evaluates its primary integral/RMS,
complementary-field RMS, canonical energy and area; a second dispatch reduces the
probe's contiguous contributions to
one compact ring record. Readback size therefore depends on the 16-probe, 2048-frame
ring rather than mesh density. The recorder runs at most 120 samples per simulated
second and limits one compiled set to 200,000 element contributions. CPU and GPU
paths report mean and RMS primary field, mean energy density, total energy, covered
area, geometric coverage and RMS in-plane complementary-field magnitude.
Definitions and host histories survive ordinary
solver handoffs; stencils rebuild for each committed mesh.

The singleton far-field monitor derives a counterclockwise rectangular contour from
the outer domain and a document-level inset. It uses 256 equal-arclength midpoint
samples with outward normals and accepts the configuration only when the contour
encloses all modeled boundaries and every sample lies in the same lossless
exterior region. The topology compiler derives that exterior from outer-boundary
face ownership, requires exactly one exterior face, and checks every compiled curve
segment against the inset shell. The exterior material must be uniform, isotropic,
lossless, and free of region sources; a localized point source must lie inside the
contour. Spatial or driven materials remain valid in fully enclosed faces. The
legacy path checks adaptively subdivided periodic and open spline traces with the
same fixed world-space margin. Both paths deliberately ignore control polygons
because their hull may extend beyond a fully enclosed curve.
For exterior speed `c`, the canonical GPU records direct `u=Q/M`, its accepted
backward endpoint rate, and `grad(u) dot n` at 60
samples per simulated second. A second compute dispatch evaluates 96 directions
with temporal interpolation of the retarded contour data and the directional
Huygens integrand `grad(u) dot n - (n dot d) u_t / c`. The observation timestamp is
delayed by the largest contour projection, so every direction refers to the same
far time. The 512-frame raw ring must span the full propagation-delay range; an
explicit configuration error replaces truncated output when it cannot.

Only the compact 96-direction, 512-frame ring is read back. Its host history stays
continuous across remesh and AMR generations when the inset and background wave
speed remain compatible. The readout shares the other probes' time-navigation
semantics across a direction/time waterfall, normalized 40 dB instantaneous and
visible-window-averaged polar patterns, and radiated power versus time. The power
trace is the directional intensity integrated over observation angle, so it remains
a relative nondimensional quantity rather than an absolutely calibrated flux. The
contour is derived display state, not selectable geometry, and its visibility has an
independent View toggle.

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
all seven coefficients contribute. Canonical transaction transfer locates target
primary support in source parent triangles for bounded conservative `Q` mapping,
and reconstructs the six complementary samples in physical coordinates. Ordinary
higher-degree triangular Lagrange bases must not inherit P1 lumping. Report mesh
preparation, operator preparation, display time, and stepping throughput separately,
with DOFs, memory, timestep, and phase/amplitude error.

The solver is paced to a persisted ceiling on simulated seconds per wall second
rather than to real time: the wall-clock time a frame took is scaled by that
ceiling before being spent at one time step a substep, so half the speed asks for
half the substeps. Below a ceiling of one time step per frame budget that count
floors to zero on most frames and the picture judders, so beneath that point the
step itself shrinks to `PACING_FRAME_SECONDS` times the ceiling and every frame
gets one. Shrinking is always safe — the mesh's figure is a stability limit, an
upper bound — and the step never goes above it. A ceiling that wants a different
step republishes the scene unchanged, so the preparation reuses the plan, the
mesh and the operator and the field crosses on the identity transfer; that is the
same path an adaptation handoff takes, which already changes the step on nearly
every run. At most `MAX_STEPS_PER_FRAME` are requested in a frame and the
leftover is capped at one frame's worth, so unspent time is dropped rather than
queued into a backlog that never drains — which is also why asking for more than
a scene can afford falls short instead of running away. The reached rate is
reported beside the ceiling, read from a held best rather than the raw windowed
measurement, because that measurement dips to about three quarters whenever a
handoff withholds stepping inside its window; comparing it directly would report
a shortfall on scenes that are keeping up perfectly well. Pulse injection is an
accepted `Q` increment and remains an initial-condition action rather than a time
signal. Version-22 point and volume sources author acceleration waveforms; the
canonical forcing path uses their zero-initial-rate analytic antiderivative and
the immutable generation reference mass. The default frequency is 2.5 cycles per
dimensionless time, corresponding to wavelength 0.4 at wave speed one.

The accepted canonical generation keeps running while a replacement is prepared.
Invalid drafts do not alter it. Reset is explicit; accepted geometry edits carry
the direct field and physical histories through the transaction lifecycle below.

## Transaction lifecycle

The implementation maintains an accepted simulation revision, at most one active
candidate, and the latest requested geometry revision.

1. Capture a candidate request and its accepted source-discretization revision.
2. Prepare geometry, mesh, connectivity, scalar comparison and canonical operators,
   boundary labels, direct-state/history transfer maps, forcing and timestep under
   a bounded work budget. Continue evolution on the accepted generation.
3. Validate the candidate. Preparation errors discard the candidate, not the
   accepted simulation.
4. At a complete timestep boundary, verify the transfer's source generation is
   still current and apply it to the latest accepted GPU `Q,b` and physical history.
5. Initialize or transfer auxiliary state as specified, enforce constraints, and
   validate the resulting state before activating it. Preserve old buffers until
   the candidate can safely become active.
6. Switch all accepted resources together, then schedule the latest outstanding
   request if necessary.

Not every document revision is a solver-generation replacement. Dependency
classification admits two narrower transactions on the current generation:

- measurement-only changes compile/adopt probe and far-field metadata without
  transferring state, packing a solver plan, draining steps, or changing the GPU
  generation;
- source-drive-only changes use a staged GPU table event and publish the CPU
  candidate only after that serial is accepted. This path requires unchanged
  spatial weights, prescribed data, drive count and timestep; it preserves the
  instantaneous carrier phase and integrated-rate anchor.
- source-weight changes with identical explicit structural support stage
  candidate weights in unused table lanes, validate them, and atomically promote
  weights and drive runtime together. Point-source support is its selected
  physical trace side, not the exact nonzero pattern of its Gaussian values, so
  underflow, motion, width and enable/disable edits do not manufacture layout
  changes. A changed structural support mask is not eligible for this event.

Spatial source support/layout, prescribed data, operator, mesh or timestep changes retain
the full lifecycle above. A narrow transaction that fails its dependency check
must fall back to a full candidate rather than partially updating accepted data.

Candidate mesh construction, scalar/canonical operator assembly, and scalar/vector/
physical-history transfer maps are resumable and yield under the frame's work
budget. Changed probe and far-field stencils are compiled as measurement work;
unchanged stencils survive revisions. The
native application packs the immutable GPU plan and transfer tables on a worker;
the accepted generation keeps running while that result is pending. GPU asset
serialization adopts the encoder's owned byte vectors rather than duplicating a
whole generation on the UI thread. The
accepted canonical generation continues until all previously requested steps have
been encoded. The application then pauses scheduling, dispatches transfer and
validation, and waits for one global accepted-generation commit before switching
mesh, operators, timestep, recorders and displayed state together. A rejected
candidate leaves the retained accepted buffers active.

Canonical reassembly fingerprints the exact sparse input to the second-order
outgoing trace eigensystem: trace-node order, impedance weights, and tangential
operator entries. If that signature is unchanged, the candidate retains the
accepted immutable modal system by shared ownership and skips the dense
eigensolve. Any signature difference takes the ordinary cooperative compile;
reuse is never inferred merely from an unchanged boundary-condition label.

The source mesh/operators remain fixed during candidate preparation; only their
state evolves. Transfer maps target that discretization, not a captured field
snapshot. Primary `Q` uses support-aware conservative seven-node interpolation and
bounded component correction. Complementary `b` reconstructs six physical-coordinate
samples locally through a uniform source-element index rather than scanning the
source mesh per sample. Thin-gap and outgoing maps transfer physical jump/trace histories,
not modal indices. The CPU locator uses a uniform spatial bin index rather than
testing every source triangle for every target node.

Coalesce pointer events. Do not restart all preparation on every event: useful
intermediate shapes may commit while a newer request waits. Obsolete or invalid
candidates may be discarded. Show proposed and accepted geometry separately so
latency is visible and understandable.

## State transfer and stability

Transfer integrated primary flux, independent complementary flux, physical boundary
histories, forcing runtime anchors, accounting and the exact accepted clock. Respect
time staggering; changing the timestep cannot reinterpret endpoint data. A target
node maps only through source support that contains it, so transfer does not cross
an excluded obstacle. Bounded local extension supplies new connected support;
disconnected new islands start at zero. Transfer is deliberately local and bounded,
not a claim of exact global energy conservation.

Legacy transfer for closed regions respects topological connectivity. Regions
joined through material interfaces form one transferable component, while a
two-sided wall separates its components even where old and new domains overlap
geometrically. Closed-wall sources use the same region membership.

Unified-topology transfer does not require target region IDs to exist in the
source. This lets a transmitting divider split one face into several assigned
regions, or let several faces merge, while field values follow geometric overlap.
Quadratic nodes on a separated curve carry its stable curve side; authored and
derived junction nodes additionally carry their sector trace ID. At a coincident
location, the locator first intersects those two restrictions. Because trace IDs
are snapshot-local, a changed or absent ID falls back to the curve-side lineage;
if a moved trace no longer overlaps an old face element, location finally falls
back to the containing bulk element. Thus stationary jumps are not mixed during
handoff, harmless span splitting keeps the same side lineage, and new regions from
a transmitting face split inherit the old field. Pulse and continuous source
stencils separately use shortest paths through the cut finite-element graph, so
they can reach the opposite bank only by travelling around a free tip.

GPU transfer reads the latest accepted source lanes, maps `Q,b` and physical
auxiliaries, rebuilds target force caches and prescribed ownership, validates the
whole candidate, then commits one generation serial. Source and prescribed edits
install target authored parameters while preserving accepted runtime anchors where
the contract requires continuity. Every transfer/evolution pipeline stays within
WebGPU's portable eight-storage-buffer-per-stage limit.

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
work budget. The app gives it a soft 2 ms frame slice. Ordinary topology
preparation is advanced for 6 ms of wall time per frame rather than a fixed step
count: its cooperative steps are so fine that 256 steps per frame stretched a
60 ms single-loop rebuild across more than eight hundred frames, most visibly
as seconds spent in "Connecting holes".
Refinement starts above `1.05 h_target`; collapse uses a configurable lower ratio.
Automatic solution adaptation uses `0.65 h_target`, with post-collapse size and
quality checks providing the final guard. Both directions work in passes: one
sweep of the mesh gathers every candidate, the whole list is applied worst first
with stale entries skipped, and the mesh is swept again only when a pass changed
it. A pass therefore costs one sweep however many edges it splits or collapses;
the earlier one-sweep-per-change loop made a few hundred changes on a medium mesh
exhaust the work budget.
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

For a unified topology plan, each `PlannedBoundaryEdge` is the immutable geometric
atom. The compiled plan carries one atom per arrangement segment, sampled finely
enough to intersect curves robustly; the plan the mesh is built from merges runs
of those segments (`TopologyMeshPlan::coarsened`) while every joint stays within
the meshing curve tolerance of the chord and the chord stays under the target
edge. Runs never cross an authored vertex, a span boundary, an outer corner, a
change of the faces or behaviour beside them, or a trace shared with another
source, and both sides of a span merge over the same runs so paired traces stay
paired. Before this, every curved span was cut into segments about 7e-3 long and
the time step followed; the coarsened plan on a single-hole scene gave a time
step more than three times larger with the same interior mesh. Refinement
linearly subdivides an atom, which the chord tolerance keeps within the same
distance of the curve, and coarsening cannot cross
one of its endpoints. `TraceVertexId` and AMR lineage remain separate: trace identity
pins a face sector at a sampling point or junction, while lineage controls cooldown
and preserved-element accounting. Preflight and final publication both verify exact
plan-interval coverage, active region membership, endpoint trace identity,
one-versus-two-element adjacency, and matching subdivisions for paired separated
traces. A one-sided hole boundary has no artificial partner.

Geometry revision identifies the accepted scene, while mesh revision identifies a
particular discretization of it. Scalar comparison and canonical operators plus
their transfer maps validate both. A successful adaptation prepares a candidate
canonical generation from the still-running source mesh and commits mesh,
operators, direct state, physical history, timestep and lineage together after the
tagged GPU handoff. The target field is transient and is not serialized.

For a static-linear generation, continuous synchronized canonical readback carries
the primary state needed to render `u=Q/M`. A full primary/complementary/auxiliary
snapshot is requested at the slower diagnostic/AMR cadence; the vector overlay's
compact consumer is independent of it. AMR starts only from an aligned full
snapshot. Accepted endpoint rate and acceleration are derived at that cadence,
not on every painted frame. The established scalar
spatial estimator retains recovered-flux, strong interior, interface-jump and
ordinary boundary terms, with primary loss included in the acceleration adapter.
A canonical supplement adds direct primary/complementary energy normalization,
the complementary curl/half-loss endpoint defect, and thin-gap/outgoing endpoint
defects plus their stored boundary energy. Those residuals are distributed to the
adjacent elements; retired scalar gap and second-order-memory terms are disabled
when the supplement is present. Prescribed mismatch remains a diagnostic because
strong ownership already enforces it. Recovery never averages across a material
region or duplicated baffle/wall trace. Time-driven and nonlinear generations do
not use this adapter until their residual/work contracts are derived.

The relative indicator maps error to local edge length, caps that length by the
shortest wavelength of every active time-varying point, volume, or boundary drive, and grades
neighboring targets. Quiet-element growth factors and collapse thresholds are paired
so a low-error mesh can actually coarsen. Graded targets stay local to each source
triangle instead of taking the minimum over an entire vertex star; evaluation still
chooses the conservative side on a shared edge. Fast, Balanced, and Detailed presets
select tolerance, elements per wavelength, topology budget, and maximum coarsening
step. Advanced minimum/maximum limits keep capacity explicit and allow quiet regions
to become coarser than the initial mesh.

The app advances coefficient sampling, indicator, and adaptation jobs in soft 2 ms
slices, rejects stale mesh/GPU/settings generations, and waits between estimates.
Refinement can start
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
the four outer rectangle sides receive this term; obstacle edges stay reflecting. At a
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

The production higher-order option is the passive three-state-per-mode rational
condition

```text
d = sqrt(7/8) c |k_tau|
Y(s)/Y0 = 1 + (6d/7)/s - (8d/7)/(s+d) + (2d/7)/(s+2d).
```

For each normalized trace mode it stores energy-normalized forms of three states

```text
xdot = -d diag(0,1,2) x + sqrt(d) [1,1,1]^T w
j = w + sqrt(d) [6/7,-8/7,2/7] x.
```

The trace graph diagonalizes the normalized tangential stiffness, applies the
three-state response independently per mode and transforms the flux back to the
physical trace. Stored variables `z=H^(1/2)x` have energy `|z|^2/2`; the prepared
positive storage matrix gives an explicit passive bulk-plus-boundary balance.
Zero tangential modes reduce exactly to first order. The corrected implementation
uses force-coupled implicit midpoint boundary kicks around the explicit interior
drift, rather than a separate boundary-only split.

This is a genuinely nonlocal trace operation. Disconnected trace components are
prepared separately, while corners share the assembled trace graph. Generation
handoff maps physical trace histories and then composes the target modal basis;
it never interpolates modal indices. Prescribed trace intersections use a cached
constrained factor and essential ownership wins at mixed junctions.

The candidate retains the quadratic angular expansion of the old Engquist-Majda
condition but trades some planar reflection accuracy for passivity and robustness.
On curved/corner traces the graph realization is passive, not an exact exterior
DtN map. The [auxiliary derivation](funfern-boundary-auxiliary-spike.md) and
[corrected scattering study](funfern-boundary-scattering-spike.md) document the
energy proof, reflection tradeoff and rejected split. Higher-order CRBC or
shape-specific DtN variants remain future work behind this power-conjugate
boundary interface.

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
