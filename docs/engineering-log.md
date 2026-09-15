# Engineering log

Low-effort working notes: what changed, what was checked, TODOs, open issues, and
next steps. Short bullets are enough; no entry is required for every tiny edit.
Keep current actions near the top and dated entries newest first. Durable decisions
belong in [architecture.md](architecture.md) and milestone scope in [plan.md](plan.md).

## Current TODOs

Open findings from the 2026-09-14 adversarial review. The bracketed score is how
many of the two verifier lenses upheld the claim; treat it as a filter, not a
ruling — the same detach bug scored 2/2 under one phrasing and 0/2 under another.


Follow-ups from the welding work:

- [ ] Offer the survivor picker for a weld that merges subdomains. Welding a
  detached separator's end onto a baffle folds its two regions into one face;
  the weld is refused with the compiler's `DuplicateFace` message rather than
  asking which region survives.
- [ ] `detach_endpoint` still leaves a two-arm vertex when the other two arms are
  open ends, so a seam a manual detach produces stays a locked C0 corner until
  something touches that junction. Fold it into the deferred unweld/split work.
- [ ] A curve at the 128-control ceiling cannot be sharpened to C0 and cannot
  accept a divider, because both spend controls and refinement only adds more.
  Reachable at about 41 polygon vertices. The message says so; a pre-emptive gate
  would need the cost of the whole attachment, not just of one knot.

Carried over from the cutover follow-up work, not from the review:

- [ ] A deletion that needs a survivor closes the entry before asking, so a
  mixed selection lands as two undo steps: the curves that needed no question,
  then the one that did. Gathering every choice before removing anything would
  make it one, and needs a picker that can ask more than once.
- [ ] Decide what to do with the pre-cutover `editor` module. Nothing in the app
  uses it: `files.rs` takes one constant from `persistence`, and the module's only
  other consumer is its own 54-test file. Either delete both, or keep them
  deliberately as the migration path's regression suite and say so.
- [ ] Decide what "Flip direction" means for a line probe. The README describes it
  as reversing both the sampling order and the flux sign; segments currently only
  offer Swap ends, and boundary targets carry a separate `reversed` flag.
- [ ] Measure representative browser frame timing for the cooperative topology
  job. The cutover recorded that it advances in fixed 256-work-unit slices but
  never measured what that costs in a real browser frame.

Longer-standing work:

- [ ] After the atomic topology application cutover, add sector-aware local mesh
  repair for unified curve coordinate edits. The first cutover deliberately takes
  a cooperative full rebuild for curve and junction movement while preserving
  exact mesh reuse for material, source, and boundary-law edits.
- [ ] Raise viewport video capture from the initial 30 FPS implementation to 60 FPS.
  Measure browser encoding and native GPU-readback pressure first, retain bounded
  native queues and wall-clock pacing, and report dropped frames rather than slowing
  the simulation when capture cannot sustain the requested rate.
- [ ] Replace the vector overlay's run-peak exposure heuristic with a robust
  automatic scale that can recover after a legitimate transient spike such as
  **Place pulse**, while still refusing to magnify late numerical noise. Avoid
  relying on manually identified reset points; investigate a noise-aware envelope,
  hysteresis, or a scale derived from the evolving field-energy distribution.

- [ ] Revisit the EM reconstruction's fixed `0.08 Hz` DC-rejection corner when
  editable domain extents or deliberately very-low-frequency sources arrive. It
  should eventually follow a scene time scale or become an advanced presentation
  control without making ordinary EM examples noisy.
- [ ] Add probe-data export for point, line, area, and far-field readouts. Preserve
  timestamps, spatial or angular coordinates, quantity names, and coverage metadata
  in a simple format suitable for plotting outside Funfern.
- [ ] Support longer probe recordings without unbounded host memory or unreadable
  plots. Investigate bounded multiresolution history or progressive decimation while
  keeping recent samples at full resolution and synchronized readout navigation.
- [ ] Decide whether far-field radiated power should remain explicitly relative and
  nondimensional or gain an optional physical calibration. Document the amplitude,
  distance, material, and dimensional conventions before presenting absolute units.
- [ ] Revisit generalized far-field sampling contours when editable outer-domain
  shapes arrive. Keep the automatic inset contour as the simple default and only
  expose custom contour geometry if non-rectangular domains require it.
- [ ] Diagnose and stabilize second-order outgoing conditions on curved hole or
  internal-boundary spans. They can inject energy and make the solution diverge;
  keep examples on reflecting or first-order curved faces until this is resolved.
- [ ] Extend outer-boundary measurements across more angles/frequencies and assess
  whether higher auxiliary orders justify their state and compute cost.
- [ ] Decide whether the load-compatible closed-wall role still warrants assigned
  face conditions now that holes and open baffles cover the primary workflows.
- [ ] Evaluate a dissipative relative dashpot for thin gaps. Keeping centered time
  integration would require an off-diagonal damping solve; the implemented gap
  spring is conservative.
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
- [ ] Extend coordinate-edit local repair to closed walls if that legacy role
  remains worth supporting.
- [ ] Extend region sources beyond the initial bias-plus-sinusoid time law. Candidate
  follow-ups include bounded time-envelope expressions, pulsed/chirped drives,
  vector source terms when a vector field exists, and nonlinear field-dependent
  source/material laws.
- [x] Implement topology-aware solution-driven AMR on immutable mesh plans as
  specified below.

## 2026-09-15 — A stale adaptation is dropped, not reported

"Adapted mesh does not match the active topology" was a race dressed as an
error. An adaptation job runs across many frames; while it runs, the user edits
geometry, `refresh_amr` waits for the new topology to be prepared, and then the
job finishes against a mesh that is no longer active. The runtime correctly
rejected the handoff, and the UI showed the rejection in red and raised the
warning flag, although nothing was wrong with either the mesh or the topology.

The job now records the revision of the mesh it started from, and every frame
drops it as soon as the active mesh differs, with a plain status line and no
error. The check keys on the mesh revision rather than the whole topology token
so that a document-only change, such as moving a probe, which re-prepares the
runtime but reuses the mesh, does not throw away a valid adaptation. The
handoff's own mismatch check stays as a hard error, which it now should never
reach. A `Playground` test starts an adaptation, replaces the active mesh
through a domain edit, and asserts the job is gone with no error.

## 2026-09-15 — Adaptation applies a pass at a time

"Mesh adaptation work limit reached" was structural. The refinement scan walked
every triangle to nominate a single edge, split it, legalised, and walked every
triangle again; coarsening did the same per collapse. The cost was changes times
mesh size, so 512 changes on a 20,000-triangle mesh needed ten million work
units against a budget of five million, and the job gave up before it had done
what the indicator asked for.

Both directions now gather every candidate in one sweep, sort them worst first,
and apply the whole list before sweeping again. A candidate whose edge or vertex
an earlier change of the same pass consumed, or whose vertex is cooling down, is
skipped at apply time; the plan builders reject the rest as before. A further
sweep runs only when the previous pass changed the mesh, so convergence keeps
its fixpoint meaning. On the initial scene, refining from `h = 0.23` to `0.12`
made 463 changes in 4 passes and 15,400 work units, and coarsening back made 259
collapses in 3 passes and 12,700 units. The deterministic-result tests still
pass, and a new one bounds the work at forty units per final triangle, which the
one-sweep-per-change loop exceeded by an order of magnitude. The report and the
Performance panel show the pass counts.

## 2026-09-15 — A constant field stays constant on the GPU

Enclosed subdomains with reflecting walls grew a uniform offset that reached
several percent of the field within tens of thousands of steps; Dirichlet walls
did not show it. The mechanism is floating point, not physics. The GPU applies
the operator as a CSR of `K_ij / M_i` in f32, and once each entry is divided and
rounded the rows no longer sum to zero. On a constant field that residual is a
permanent per-node acceleration. Homogeneous Neumann leaves the constant mode
free, so it integrates without bound; Dirichlet pins it, so the same residual
only produces a bounded static offset there.

Measured on a reflecting unit cavity at `h = 0.06`, stepping a field of `1.0`
exactly as `advance_wave` does: f64 row sums are `2.8e-16` relative, the f32 rows
are `1e-7` relative, and the mean drift was `1.2e-5` after 3,000 steps,
`5.3e-4` after 10,000 and `3.5e-2` after 30,000, almost entirely in the uniform
mode. The f64 reference stayed at `5.6e-10`.

Both stiffness products are now evaluated in difference form,
`Σ_j K_ij (u_j - u_i)`, on the CPU and in the shader. The forms agree whenever
the rows annihilate constants, which every assembly path does, and the
difference form is exactly zero on a constant field in any precision. It costs
nothing and the diagonal needs no special case, since its term is `K_ii · 0`.
Two tests pin the property: every assembled operator, across second-order outer
edges, curved absorbers, thin gaps, material interfaces and the topology path,
has zero row sums in both matrices; and an f32 emulation of the kernel holds a
constant field bit for bit while the old row product is shown to drift. The
edited shader was parsed and validated with the naga version the app links.

A separate effect remains and is physics: a source switched on abruptly leaves
a nonzero mean velocity, and in a Neumann cavity that gives a linear drift that
the f64 reference also shows. Quadratic versus linear growth tells the two
apart.

## 2026-09-15 — The example loads and the checklist describes this app

`examples/eight-obstacles.json` was still schema version 1 and could not load
after the version-22 break, while the README and the browser checklist both sent
readers straight to it. It also predated the in-app example gallery, which
already carries the same scene as Obstacle array, so it had quietly become a
stale duplicate rather than a separate artefact.

It is now an export of that catalog entry at the current version, and two tests
keep it honest: one parses every JSON file in `examples/`, the other asserts the
shipped file still matches the catalog document, so changing one without the
other fails here rather than in front of a reader.

`browser-checks.md` still described the pre-cutover UI, with a Draw menu split
into Hole and Interface, a role change on a mixed selection, and baffle endpoint
merging. Those are rewritten for the tools that exist, and the checklist gained
items for what this run built: welding by drag, the in-scene survivor picker,
inactive spans, the Faces and Regions toggle with hole conversion, dragging the
outer rectangle, hover cursors, and the mesh overlay drawing over the field while
the categorical overlays no longer imprint it.

One thing the swap turned up: the version-one file was doubling as the fixture
for a pre-cutover migration test, which is why it had never been regenerated. The
fixture moved to `crates/funfern-app/tests/fixtures/`, where a deliberately old
document belongs, leaving `examples/` for scenes a reader can open. That test
belongs to the `editor` module, which nothing in the app uses any more; whether
to keep it is now its own item above.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
-- -D warnings` clean, `cargo test --workspace --locked` 431 passing, and
`cargo build --release -p funfern-app --locked`.

## 2026-09-15 — One gesture, one undo step

Deleting several curves issued one editor command per curve, each closing its own
history entry, so undo walked back through them one at a time. The log carried
this as two separate items; they were the same thing.

`begin` was already idempotent, so the fix is a bracket rather than a new
multi-target planner. `apply_removal` split into a variant that stops after
`changed` and a wrapper that commits, `remove_curve_during_edit` exposes the
first, and the viewport's delete brackets the loop and commits once at the end.
A failure part-way cancels, taking back every removal the gesture had made
instead of leaving half a selection deleted.

The one case that still lands as two entries is a mixed selection where one curve
needs a survivor chosen. That commits what is already done before putting the
question, because answering it is a separate decision the user may cancel.
Folding it in would need a picker that can ask more than once, which is logged
rather than built.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
-- -D warnings` clean, `cargo test --workspace --locked` 429 passing, and
`cargo build --release -p funfern-app --locked`. The new test deletes three
baffles and fails with the per-curve commit restored.

## 2026-09-15 — Captures show the scene, not the tools

Capture suppression did not survive the cutover, so PNG exports and video frames
carried whatever was on screen. Restoring it turned out smaller than the log
feared, because the capture already crops to the central viewport: side panels
and the status bar are outside it by construction and needed nothing. What leaks
in is the chrome drawn inside the crop and the windows that float over it.

A `capturing()` predicate over the snapshot and recording states now gates the
weld targets, the domain grips, the transform and material frame gizmos, the
survivor highlight and its prompt, the marquee, the draw preview, the diagnostics
and probe readout windows, the Draw window, and the inspector on a narrow layout
where it floats rather than docks. Selection emphasis reads through the predicate
instead, since `span_selected`, the owned-control ring, the active handle and the
selected probe are consulted from several places; making the predicate answer
false is one change rather than a dozen.

What stays is what `browser-checks.md` asks for: the field, the active View
overlays including control polygons, handles and boundary badges, the geometry,
the probes, the source marker, and the logo.

The old implementation threaded a `clean_capture` bool through eighteen call
sites. Each draw consults the predicate itself now, which is why this is a
smaller diff than the one it replaces.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
-- -D warnings` clean, `cargo test --workspace --locked` 428 passing, and
`cargo build --release -p funfern-app --locked`. What a capture actually contains
needs the browser checklist; the test covers the predicate and the selection
emphasis that reads through it.

## 2026-09-15 — The measurement now includes its own tail

**A finished preparation dropped its last slice.** `advance` counted the slice
and updated `longest_slice_ms` only after `advance_slice` had already copied the
timing into the finished handoff, so every committed `PreparedTopology`
understated exactly the tail the diagnostics exist to show. The finished result
is stamped with the updated timing now. On the default scene that is 10821 slices
reported instead of 10820, and the missing one is the longest. Worth having
before an optimization sweep reads this instrument. The test drives a preparation
one slice at a time and compares its own count against the handoff, and it fails
with the fix reverted.

**`CompiledFace::centroid`'s doc** claimed the point lies inside the face. It does
for a simply connected face and not in general, since an annulus puts it in the
hole. That is the right answer for a radial profile in a ring, which is what it is
for, so the comment changed rather than the arithmetic.

**`set_face_disposition` spent a `RegionId` before its compile check**, which the
review had found in the command this one replaced and which followed the pattern
across. The id is provisional now and the allocator only moves once the candidate
has compiled, the same shape `plan_removal` already used for a cut curve.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
-- -D warnings` clean, `cargo test --workspace --locked` 427 passing, and
`cargo build --release -p funfern-app --locked`.

## 2026-09-15 — Three input defects from the review

**Shift-snapping a probe snapped the pointer.** The update computed
`snap_point(cursor) - grab` from the raw press point, so the probe landed off the
grid by however far inside itself it was picked up, and a disk radius jumped at
gesture start. What moves is snapped now, per hit kind: a point or an endpoint
lands on the grid, a segment body translates rigidly with its start on the grid,
and a radius snaps to the same step.

**Escape cancelled a gesture while a text field had focus.** The review suspected
this and asked for verification; egui 0.36.2 confirms it. `Focus::begin_pass`
sets `focused_widget = None` on Escape while processing the frame's input, and
`egui_wants_keyboard_input` is just `focused().is_some()`, so the guard reads
false on exactly the frame that matters. The viewport now also consults whether a
widget held focus when the previous frame ended, which makes the first Escape
leave the field and only a second one reach the viewport.

**The double-click probe lookup hand-rolled its own distance test**, so it
ignored the View visibility toggles and picked the bottom-most probe where a
single click picks the topmost. It calls `hit_probe` now, the same lookup a
single click uses.

The `enclosed_region` finding is closed as well, though not by this commit: the
Inside and Hole toggle it described no longer exists, having been replaced by the
per-face disposition in the Materials panel.

The Escape fix carries no automated coverage, since egui's pass order cannot be
driven from a unit test; it rests on reading egui's source and needs a look in the
app. The other two are covered.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
-- -D warnings` clean, `cargo test --workspace --locked` 426 passing, and
`cargo build --release -p funfern-app --locked`.

## 2026-09-15 — Categorical overlays stop printing the triangulation

With the wireframe fixed, the Materials and Subdomains overlays still carried the
mesh, and the material property overlay did not. The difference was how each is
painted. The property overlay builds one `egui::Mesh`; the categorical ones added
one `Shape::convex_polygon` per triangle, and a polygon carries its own
antialiased outline, so the outlines of neighbours leave a seam along every
shared edge. The mesh was imprinted on the overlay whether or not the user had
asked to see it, which is also why the Mesh checkbox looked inert before.

Both categorical overlays now build a single mesh. Vertices are duplicated per
triangle rather than shared, so each triangle keeps its flat colour and the
boundary between two regions stays a step instead of becoming a gradient, which
is the one thing a shared-vertex mesh would have got wrong.

The survivor highlight had the same defect and is fixed with it: it was printing
the triangulation across the very subdomain it was asking the user to look at.

The adaptation-target overlay keeps its per-triangle polygons. Its colour is a
per-element quantity, so there the element boundaries are the data rather than an
artefact of how it is drawn.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
-- -D warnings` clean, `cargo test --workspace --locked` 425 passing, and
`cargo build --release -p funfern-app --locked`.

## 2026-09-15 — The mesh is drawn over the field, not under it

The View panel's Mesh checkbox reached the document and the wireframe was drawn,
but underneath the field. The field covers the whole domain, and with no material
overlay beneath it `field_color` returns a fully opaque colour, so the wireframe
was painted and then completely buried. With an overlay on, the field's alpha
rises with the wave amplitude to 220, which washed out what remained.

The wireframe was also far too faint to compete even where it showed: half a
pixel at grey 75, against mesh boundaries drawn at 1.15 pixels in a light blue
grey. What the Materials and Subdomains overlays "show" is not the wireframe at
all. It is the antialiasing seams between adjacent flat-filled triangles, which
is why the mesh looks visible with either overlay, Regions being the default,
and why toggling the real checkbox seemed to change nothing.

Both the wireframe and the mesh boundaries now draw after the field and before
the vector overlay, and the wireframe reads at 0.7 pixels in a light grey at
moderate alpha: visible over a field, still subordinate to the boundary lines.
Mesh boundaries had the same burial and were fixed with it, though nothing had
reported them, presumably because they are mostly viewed with an overlay on.

Paint order carries no automated coverage, so this one is for the eye.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
-- -D warnings` clean, `cargo test --workspace --locked` 425 passing, and
`cargo build --release -p funfern-app --locked`.

## 2026-09-15 — Cursors that appear before the action, not after

The viewport's cursor block read its position from `interact_pointer_pos`, which
egui documents as `None` unless the widget is already being interacted with. So
every hover branch was dead and only the branch keyed off a running drag ever
fired, which is the opposite of what a cursor is for: it announced what you were
already doing. `hover_pos` fixes it in one call, and that alone revives the
transform gizmo and domain corner cursors that were already written.

The rule for what earns one, since most affordances had no mapping at all: a
cursor appears only where the drawing does not already announce the affordance,
or where direction matters. A drawn handle that moves itself is its own
announcement, so control points, junctions, loose ends, probe points and segment
endpoints, and the source marker stay bare, as does open space.

What is mapped now, in the order the press handler resolves grabs: the material
frame's origin and rotate, the transform gizmo's pivot, ring and three scale
axes, a probe's disk radius and its disk and segment bodies, the domain's
corners and sides, and a selected span. The last one is conditional on the
selection being able to move rigidly, which the gizmo already answers, so its
absence is what tells the user to widen the selection. That quietly restores the
feedback lost when the transform refusal messages were dropped.

Modal states carry a cursor because no handle can: crosshair while drawing,
placing a pulse, or placing a probe, and a pointing hand over a survivor
candidate, with nothing elsewhere since a click there does nothing. Resolution is
modal first, then the running gesture, then hover, so a gesture keeps whatever
appeared under the pointer when it started.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
-- -D warnings` clean, `cargo test --workspace --locked` 425 passing, and
`cargo build --release -p funfern-app --locked`.

## 2026-09-15 — The outer rectangle is draggable again

Dragging the domain's sides and corners did not survive the cutover in
`eddad77`. The old viewport carried a `DomainDrag` with Side and Corner
variants, a corner hit test, and a live resize; none of it came across, and the
replacement was the four numeric extents in the Edit panel, which appear only
once an outer side is selected. Nothing reported the loss: pressing an outer side
started an ordinary span transform whose curve-span set was empty, so the drag
moved nothing and said nothing. The rigid-transform refusal for outer selections
even points at the numeric fields by name.

Restored to what it was. A corner grab outranks the two sides that meet there, so
both gestures stay reachable; a side drag needs no modifier, leaving Shift and
Command for selection as before. Shift snaps to the grid, the corners carry their
diagonal cursors, and the whole drag is one history entry, since press begins the
transaction and release commits it. `set_domain_during_edit` was already on the
topology editor and unused, so nothing new was needed underneath.

The corners are now drawn as small grips, which the old viewport did not do. They
hide during drawing, a staged removal, and any gesture that is not about the
domain.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
-- -D warnings` clean, `cargo test --workspace --locked` 422 passing, and
`cargo build --release -p funfern-app --locked`.

## 2026-09-15 — Spans that bound nothing say so

The span inspector now reads "Inactive" when every selected span has an excluded
face on both sides, and "N of M inactive" for a mixed selection. Such a span
bounds nothing the simulation solves, so its boundary law, its side conditions
and its coupling all have no effect, and the panel offered them with no hint that
they were inert.

`span_context` already resolved both sides' activity for the per-side readout, so
this reads what was there and needed no new machinery. The state is easy to reach
now that each half of a split subdomain is emptied on its own: a separator with a
hole either side of it is the ordinary case.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
-- -D warnings` clean, `cargo test --workspace --locked` 420 passing, and
`cargo build --release -p funfern-app --locked`.

## 2026-09-15 — Holes belong to faces, not to curves

The Inside and Hole toggle sat in the span inspector, keyed by curve, and on the
autosave that prompted this it was broken twice over. It appeared for an open
curve, because its only condition was "does any face anchor belong to this
curve", with no closedness test. That curve owned two anchors, so the lookup took
whichever came first. Then Hole failed with "Only a closed curve encloses a face"
while the material button succeeded by short-circuiting into a plain material
reassignment, so one button always failed and the other quietly did something
else.

The operation moved to the face, where it belongs. `set_face_disposition` takes a
face assignment and re-walls only that face's own boundary, read off
`CompiledFace::boundaries`. A span dividing two faces transmits exactly when both
of its sides are active subdomains, and a slit inside the face, where both sides
are the same, keeps whatever the user gave it. That is well defined in both
directions and never touches a span outside the face. It also removes the old
refusal for a divided subdomain: each half is its own face and is emptied on its
own, leaving the other alive behind the new wall.

Worth recording that the boundary was never in doubt. An earlier note here
implied a face bounded by parts of an open curve had no span set, which was
wrong: the compiler gives every face closed cycles and every curve span borders
exactly two faces. What the old code lacked was not the boundary but any use of
it, since it walled a whole curve instead.

The Materials panel gained a Faces and Regions toggle. Faces lists one row per
assignment, holes included, each with a material dropdown that also offers Hole;
Regions is the old list. The toggle also steers viewport picking and the
selection outline, so clicking a hole selects its row the way clicking a
subdomain always has. `set_enclosed_disposition` and `enclosed_region` are gone.

One consequence to know about: a round trip normalises a face's boundary. A
subdomain whose edge was part wall and part opening comes back all open, because
nothing records which walls were deliberate.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
-- -D warnings` clean, `cargo test --workspace --locked` 419 passing, and
`cargo build --release -p funfern-app --locked`.

## 2026-09-14 — Errors that read as sentences

Six error types across the core rendered `Display` as `{self:?}`, so the status
bar greeted the user with `NearContact { first: Curve(CurveSpanId(57)), second:
Curve(CurveSpanId(65)) }` and the continuity buttons reported `NotRemovable`.
Each now writes one sentence, and the same line reads "span 57 and span 65 touch
with no junction between them".

Deliberately not a help system, as the intent was only to stop leaking `Debug`.
Each variant gets a single clause, naming the curve or span when that is what
tells the user where to look, and `CompiledEdgeSource` gained a `Display` so a
contact can name the outer side or the span on either end of it.

`Debug` keeps every field, and the diagnostics window's Topology section now
prints the compile issue through it, which is where that detail belongs. The
types covered are `TopologyIssue`, `SplineError`, `FaceAnchorIssue`,
`SeparatorAttachmentIssue`, `TopologySceneIssue`, and `TopologyMeshPlanError`.
`user_facing_errors_read_as_sentences` in the core geometry tests holds the line:
each message must differ from its debug form, carry no Rust punctuation, and run
to at least four words.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
-- -D warnings` clean, `cargo test --workspace --locked` 418 passing, and
`cargo build --release -p funfern-app --locked`.

## 2026-09-14 — A curve may attach to itself, and Delete is a button again

**Self-attachment.** A loose end could not be dropped on its own curve. The hit
test excluded the whole dragged curve from breakpoint and edge targets, and the
command refused a curve anchor on itself with "Attach to another curve". Neither
was load-bearing: the representation already holds a self-attached curve, and the
autosave that prompted this is one, a loop and a string that are a single open
curve sharing one vertex between node 0 and node 3.

The exclusion is now the dragged tip's own end span and nothing else, which is
the part that genuinely sits under the cursor for the whole gesture. Open-curve
endpoints were already skipped by the breakpoint test, so the tip cannot pick
itself. On the saved scene every node and span of the curve is now a target
except that one span. `attach_end_to_vertex` locates the tip again after
materialising the attachment, because splitting a span of the same curve inserts
a node ahead of it and the old index went stale.

**Delete.** The panel's only deletion affordance for spans was a "Delete curve N"
button, shown when a whole curve was selected, which called the removal with no
survivor and so failed with "Choose which adjacent material survives this
deletion" on any deletion that merges two subdomains. Partial span selections had
no button at all and could only be deleted from the keyboard. One "Delete" button
now runs the same path the Delete key does, which handles whole curves, partial
runs, probes, and the in-scene survivor picker.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
-- -D warnings` clean, `cargo test --workspace --locked` 417 passing, and
`cargo build --release -p funfern-app --locked`.

## 2026-09-14 — Drop the transform refusal messages

The rigid-transform gizmo used to explain itself when a selection could not
move, with `Junction also belongs to unselected spans` or `Selected span section
must end at a corner or include the whole curve`, and it offered a **Select
incident spans** button under the first. Both are gone, along with the second
**Select incident spans** button on a selected junction handle. A marquee selects
incident arms trivially, and what is holding a selection is visible in the scene,
so the gizmo now simply does not appear.

Worth recording what the rule actually is, because the old message stated it
badly and so did I when asked. Nothing tears: `synchronize_vertices` re-pins
every vertex-carrying node after any transform. Removing the check by hand and
translating one arm of a three-arm junction by 0.15 moved the free tip by the
full 0.15 and the junction end by zero, leaving the arm stretched rather than
moved. So the single rule behind both refusals is scope: **a transform changes
nothing outside its selection.** A partial junction breaks it in both directions,
since moving the vertex reshapes unselected arms and leaving it distorts the
selected one, and a smooth boundary knot breaks it through shared controls.
Dragging a junction handle is the legitimate form of the first, which is why that
has always worked.

`Isolate at C0` remains the remedy for the second and was measured as exact: it
moved a curve by 1.1e-16 and turned the refusal into a four-control plan.

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
-- -D warnings` clean, `cargo test --workspace --locked`, and `cargo build
--release -p funfern-app --locked`.

## 2026-09-14 — Actions that are offered are actions that work

Manual testing turned up a family of UI actions that stay live while the
operation behind them cannot succeed. Reproduced each against the native
autosave before changing anything.

**A closed curve's four-control floor.** A periodic cubic needs at least four
control points, and a closed curve's control count is the sum of its breakpoint
multiplicities. The saved scene held a two-span loop at `[3, 1]`, exactly on the
floor, so its C0 seam could not be promoted in either direction — and the buttons
offered it anyway, reporting the raw `NotRemovable` enum name. This is a direct
consequence of the weld: closing prepends a multiplicity-3 seam, so welding a
two-span baffle into a loop always lands on the floor.

The fix is refinement rather than a gate. Knot insertion is exact, so promotion
now buys the controls it needs first and the curve does not move; measured on the
saved curve, two insertions moved it by 4e-16. `promotion_control_deficit`
computes the shortfall, `refine_curve_to_spans` splits the widest span, and both
run on the command's candidate so the whole gesture stays one history entry. The
breakpoint is re-found by parameter afterwards, since inserting a knot shifts
node indices. Welding a single-span curve into a loop refines the same way
instead of refusing.

The one gate that stays is topological: a node carrying a junction remains a C0
corner, and no refinement changes that. The one case refinement cannot buy is the
opposite wall — the 128-control ceiling, where sharpening to C0 costs two
controls and insertion only makes it worse. That now says so in words.

**A closed curve's seam lost its junction.** `node_vertex_at` matched node
parameters without wrapping, so the span that ends at a closed curve's seam
arrives at the period and never matched node zero. Anything attached there — a
divider drawn onto the seam, a loose end welded to it — compiled as an accidental
`NearContact` instead of a junction. One periodic comparison fixes it, and the
editor's own node lookups wrap the same way. This is what made dividers fail "in
some cases" without an obvious pattern: the pattern was the seam.

**Two more offered-but-impossible actions.** Delete control now asks the command
itself whether it would succeed, through `control_removal_error`, which runs the
real removal on a copy so the prediction cannot drift. Transmit and Boundary are
offered only when applying them would change something; before, a selection that
already carried that law reported "Select existing curve spans".

Checked: `cargo fmt --all`, `cargo clippy --workspace --all-targets --locked
-- -D warnings` clean, `cargo test --workspace --locked` 416 passing, and
`cargo build --release -p funfern-app --locked`. Both seam tests were verified to
fail with the wrap reverted. Interactive verification of the buttons and the
refinement gesture is left to the browser and the native app.

## 2026-09-14 — Span soup: welding, and deletion that survives the figure-8

- The figure-8 autosave — a closed loop with a chord welded across it — could
  not lose any span. Four separate defects, all reproduced against the file
  before touching anything:
  - `remove_spans` rebased every face anchor on the cut curve into the piece's
    parameter domain, then handed `rebuild_face_assignments` the *un-rebased*
    snapshot. An anchor whose parameter had left its span failed to resolve, was
    dropped silently, and its face fell into the "no anchor, so take the
    survivor" branch — two faces claiming region 1, `DuplicateRegion`. It bit
    only when a surviving, untouched face was anchored on the cut curve: nine of
    sixteen spans.
  - A run through a junction node frees the chord and merges all three faces,
    which the "cap at two" rule refused.
  - `remove_curve` never called `promote_freed_curves`, so deleting the loop
    stranded the chord with two free transmitting ends.
  - The control-point highlight lit every control of a curve when any of its
    spans was selected.
- Removal now goes through one `RemovalPlan` for curves and spans: cut or
  remove on a copy, prune, promote, fuse loose ends left at a two-arm junction,
  compile, and read the merge off the result — the regions landing on one face.
  One group asks for a survivor from however many regions it holds; two groups
  is refused with a reason; a region whose anchor died but whose face persists
  keeps its region under a fresh anchor. `curve_removal_choices` and
  `span_removal_choices` are thin wrappers over the plan, so the question the UI
  asks is exactly the merge the command performs. Deleting a hole now reports no
  choices rather than the one region it borders; both mean "no question".
- Welding. Two loose ends meeting are one curve, never a junction: an authored
  vertex exists only at valence three or more and on the outer domain. New in
  core: `OpenCubicSpline::close` (the inverse of `open_at` at breakpoint 0 —
  drop the duplicated corner, seam multiplicity 3, period and span order kept;
  a single span has too few controls and is refused), `TopologyCurve::reversed`
  with `SpanBehavior::mirrored` and `CurveTraceSide::opposite`. `join` and
  `reversed` existed unused since the legacy editor; they now have error-path
  and involution tests. The editor's `join_curves` keeps the stationary curve's
  identity and direction, reverses the absorbed one when the ends demand it,
  pins the absorbed end onto the stationary tip first so `join`'s averaged seam
  control is exact, and carries every anchor and boundary probe across with
  `p' = offset + (reversed ? period − p : p)`, flipping sides and toggling the
  probes' direction. `weld_endpoint` routes a dropped loose end: another loose
  end joins, the same curve's other end closes, a junction, outer side, curve
  interior, or vertex-less breakpoint gains an arm. The arrangement must compile
  or the weld is refused and only the drag remains.
- A vertex-less breakpoint is a new attachment target, `Breakpoint`, because a
  `FaceAnchor::Curve` sitting exactly on a node resolves to `AtVertex`.
  `materialize_attachment` raises the node to a corner and binds a vertex
  without inserting a span — a seam produced by a weld used to fail or grow a
  sliver span when a third curve was attached there.
- Promotion follows the compiler's own rule now: an open curve is demoted only
  when a free tip's adjacent span transmits. The earlier "any transmitting span
  plus any free tip" would have walled off the chord after the arc auto-joined
  it on the figure-8, though it still separates the surviving lobe. A
  transmitting span that ends up with the same face on both sides is left as it
  is; the mesh plan drops such edges, so it is inert rather than wrong.
- UI. Dragging the control of a loose end is a new gesture: gold dots on every
  eligible target, a ring at the live snap, the weld on release, one history
  entry with the drag. The snap is recomputed at release rather than taken from
  the gesture, because a snap captured mid-drag can name a face of a snapshot
  that validation has since replaced. Drawing an open curve snaps to loose ends
  and breakpoints too. The survivor picker moved out of the Edit panel, where it
  only rendered with a span selection open, into the scene: candidates fill
  gold with their material named at the face centre, a prompt sits over the
  viewport, a click picks. Controls now ring blue only when they shape a
  selected span (`selected_span_controls`).
- Verified: the figure-8 autosave deletes every span, offers three survivors for
  the run through the junction, and drops the whole loop leaving the chord as a
  baffle; `cargo fmt`, Clippy with warnings denied, all workspace tests, native
  release build. Not verified from here: the drag gesture, the picker, and the
  highlight need a hand on the mouse.

## 2026-09-14 — Deleting part of a divider

- Delete only ever acted on a span selection that covered a complete curve, so a
  partial selection did nothing at all and said nothing. It now deletes one
  contiguous run and keeps the rest as open pieces.
- Added `PeriodicCubicSpline::open_at`, the crate's first periodic-to-open cut:
  it raises the breakpoint to C0, rotates the controls to start at that corner,
  and takes one extra control so the corner clamps both ends, the same shared
  seam control `OpenCubicSpline::split` produces. Everything else composes from
  `split`. Verified against every breakpoint of a rounded loop, a polygon's
  existing corners, and a C2 knot.
- Every surviving piece becomes a baffle with the default wall. That is forced,
  not cosmetic: `GraphBuilder::finish` rejects an open curve whose end span is
  transmitting at a free tip. For the same reason any curve the cut frees from a
  junction is promoted whole and reported, so the gesture cannot leave an invalid
  draft behind.
- Face merging reuses the whole-curve survivor rules, including the exterior
  identity override, and refuses when more than two subdomains would merge.
  `remove_curve`'s `by_new_face` rebuild is now a shared
  `rebuild_face_assignments` rather than a second copy.
- Boundary probes follow the piece holding more of their path and are dropped,
  and reported, when nothing contiguous survives — the rule the pre-topology
  `split_internal_boundary` used. Nothing had ever trimmed a probe's span list
  before, and `validate_probes` rejects a document naming a missing span, so an
  untrimmed probe would have made the scene unsaveable.

## 2026-09-14 — Nothing else touches the GPU during an upload

- Three writes to the GPU ran inside the upload window and each could wedge the
  handoff, which the new pacing gate then turns into a permanent stall because
  stepping is withheld while a handoff is pending.
  - `reset_requested` rebuilt the buffers against the *active* topology through
    `replace_with_volume_sources` -> `install`, which bumps the generation the
    pending commit is waiting for, so `display.generation == upload.generation`
    could never hold again. Reset now stays queued until the upload finishes.
  - A queued pulse would land in the new buffers through the old operator's
    stencil. It waits too.
  - `request_runtime()` ran unconditionally, so an edit mid-upload started a new
    preparation, and `TopologyRuntime::request` clears `ready`. It is now skipped
    while an upload is in flight; the edit is picked up on a later frame because
    the revision comparison runs every frame. `refresh_amr` already returned
    early in this window, so AMR could not supersede either.
- Reordered the commit so `finish_transfer` runs only after `commit_ready`
  succeeds, and a refusal rolls the transfer back. Finalising first freed the old
  buffers while `runtime.active()` still named the old topology, leaving the GPU
  on a discretization nothing in the document described. With the request gate
  above this should now be unreachable, so it is defence in depth rather than the
  fix.
- Not covered by tests: all three live in the Bevy frame loop and need
  `Assets<ShaderBuffer>` and `Commands`, which the headless suite cannot build.
  The runtime-side premise — a superseded `commit_ready` leaving the active state
  untouched — is covered by `failed_or_superseded_candidate_never_replaces_active_state`.
  The rest wants the browser pass.

## 2026-09-14 — The stranded point source was in three places, not one

- The review reported the point source being left on a deleted region against
  `drop_region_dependents`, and the first fix went in there. Checking the finding
  against the new partial-deletion work found the same defect in both
  `remove_curve` and `remove_spans`, neither of which touches `model.source` at
  all. A test proved all three.
- The merge cases want different behaviour from the hole case, so the shared
  `retarget_point_source` distinguishes them: a distributed source dies with its
  region because it is a profile over that face, but the point source has a
  position that is still meshed once the faces merge, so it follows the surviving
  identity and keeps driving. Only an exclusion — the subdomain becoming a hole —
  leaves it with nowhere to be, and there it falls back to the background and
  switches off.
- Worth noting for the remaining review items: a finding names one call site, not
  the defect's extent.

## 2026-09-14 — Review triage, second pass

Harvested the stopped review's journal: 23 findings across six dimensions. Six
were in work landed today. Fixed:

- `drop_region_dependents` dropped a region without retargeting
  `model.source.region`, so making a subdomain a hole left the point source
  naming a region that no longer existed. The scene still compiled `Valid` —
  nothing in the arrangement checks the point source — but preparation rejected
  every candidate with "references an inactive region" and the simulation stopped
  with no visible cause. The source now falls back to the background and is
  disabled, since its position is inside the new hole.
- `set_enclosed_disposition(curve, None)` cleared only the anchor the closed curve
  owns. With a separator already splitting the interior, the other sub-face
  stayed an active subdomain inside the hole and the command reported success.
  The guard counts the distinct faces on the side the curve's own anchor names
  and refuses only when there is more than one, so a junction whose curve
  attaches from outside the loop still converts.
- The new material colour button committed on every frame of a drag inside its
  popup, turning one colour edit into dozens of undo entries and dozens of full
  scene revalidations. The value is staged and committed when the pointer is
  released.
- The staged divider-survivor question outlived its gesture: nothing cleared it
  when the selection changed or undo ran, so the buttons could act on a curve the
  user was no longer looking at. It is now dropped as soon as the geometry it
  names is gone.
- `subdomain_color` indexed the accepted scene from the viewport and the draft
  from the panel, so the two disagreed mid-edit. Both take the draft now, and the
  doc comment says so instead of claiming `RegionId` keying it never did.

Correlating the 42 adversarial verdicts afterwards was worth doing: the two
verifier lenses reached opposite conclusions on the same detach-orphan bug
(confirmed on the high-severity phrasing, refuted on the low-severity one), which
a failing repro settled in favour of confirmed. Treat the verdicts as a filter,
not an oracle. The split-subdomain claim lost both its verifiers to the kill, so
it was never judged at all — the first guard written for it was broader than the
claim and has since been narrowed.

Not acted on, recorded for later: the runtime findings around `reset_requested`
and `request_runtime` running while an upload is in flight, the Escape key's
typing guard, probe shift-snapping the cursor rather than the probe, and the
double-click probe lookup still using its own hit test rather than `hit_probe`.

## 2026-09-14 — Three fixes found by adversarial review

- Detaching an outer-attached endpoint left an orphan vertex. Replacing
  `prune_unused_vertex` with `prune_dangling_junctions` earlier today narrowed
  pruning to interior vertices, so a zero-reference outer vertex survived, kept
  subdividing its domain side, and still drew a junction handle.
- The opposite error in the same helper: detaching one arm of a shared junction
  also cleared the other arm's breakpoint, so re-attaching materialised a second
  vertex instead of restoring the scene. The two policies are now explicit —
  `prune_unreferenced_vertices` for detach, `prune_dangling_junctions` for
  removal — over one `prune_vertices` core.
- Deleting several curves in one gesture removed only the first. Every command
  clears `compiled_draft`, and both the survivor query and the command need it,
  so the second iteration failed with `Resolve the invalid draft`. The gesture
  now revalidates between commands. Multi-curve deletion is still one history
  entry per curve rather than per gesture; that remains open.

## 2026-09-14 — Subdomain/hole switching and the missing survivor choice

- Nothing in the editor could change a face's disposition. `face_assignments`
  was only written by curve creation and removal, so turning a subdomain into a
  hole or back meant deleting the curve and redrawing it.
  `set_enclosed_disposition` now flips the face the closed curve owns: to a hole
  it clears the assignment, drops the region with its distributed source and area
  probes, and separates every span with the default wall; back to a subdomain it
  allocates a fresh region on the chosen material, seeds its frame at the face
  centroid, and makes every span transmit. The candidate compiles before it is
  committed, and the whole switch is one undo entry.
- `remove_curve` has always required an explicit survivor when a curve borders
  two assigned subdomains, but the viewport called it with `None` and printed
  `Choose which adjacent material survives divider removal` with no way to
  answer. That is also why a closed subdomain appeared undeletable while a hole
  or a baffle deleted fine: those border one active region, a subdomain borders
  two. `curve_removal_choices` now reports the candidates, Delete stages the
  question instead of failing, and the Edit panel offers **Keep &lt;material&gt;**
  per adjacent region plus Cancel.
- Still open: removing part of a divider and promoting the leftovers to baffles.
  That needs curve splitting with stable span identities and dependent remapping,
  so it is a slice of its own rather than a rider on this one.

## 2026-09-14 — New subdomains start with a frame inside themselves

- A new region took `MaterialFrame::world()`, so a region-local profile or volume
  source began writing its coordinates around the world origin however far away
  the region actually was. The first thing anyone had to do was drag the frame
  gizmo back onto the subdomain.
- Added `CompiledFace::centroid`: the area-weighted centroid over the face's
  cycles, so the counter-clockwise outer boundary and clockwise hole cycles
  subtract correctly and a face with an inclusion still centres on its material.
  A degenerate or zero-area face falls back to the mean of its outer cycle.
- `create_closed_curve` seeds a new subdomain's frame at the centroid of the face
  its anchor resolves to, and a separator's genuinely new daughter centres on the
  face it owns rather than on the parent it split. A daughter that inherits the
  old material still inherits the old frame, and the background keeps the frame
  it was authored with. Angle and attachment are untouched.

## 2026-09-14 — Material frame gizmo restored

- The region frame could only be aligned by typing: the viewport origin/rotation
  gizmo did not survive the cutover at all. `MaterialFrameDrag`,
  `MaterialFrameGizmoHit`, `selected_material_frame`, `hit_material_frame_gizmo`,
  and the drawing block were all dropped, while the README and the Profile
  placement controls still promised it.
- Restored with the pre-topology geometry: a 42 px teal ring with its angle bead,
  a red local x and teal local y axis, and a gold origin grip. Dragging the origin
  moves the frame, dragging the ring turns it, Shift snaps coordinates to 0.05 and
  angles to 15°, and the whole gesture is one undo entry through
  `set_region_frame_during_edit`.
- The gizmo now follows the same condition as the numeric controls: it appears
  while Materials is open and either the assigned material or an enabled,
  spatially varying volume source actually uses local coordinates. The pre-swap
  version only checked the material, so a constant material driving a varying
  source had numeric placement with no gizmo.
- Grip radii use the shared `hit_tolerance`, so they absorb the drag threshold
  like every other control and widen under touch.

## 2026-09-14 — Volume-source editors follow their checkbox

- The profile and signal editors appeared on the first tick of **Volume source**
  and then never went away: the block was gated on
  `existing.is_some() || source.enabled`, and unchecking only writes
  `enabled: false`, leaving the source in the document forever. The fields stayed
  on screen greyed out through `add_enabled_ui`.
- They now render only while the checkbox is ticked. The commit moved outside the
  block, otherwise unchecking would never be recorded and the box would spring
  back on the next frame. The profile, parameters, and signal stay in the
  document while disabled, so re-ticking restores what was there.
- **Profile placement** follows the same rule: a disabled source no longer keeps
  the region frame controls open on its own, though a material that uses local
  coordinates still does.

## 2026-09-14 — Paint every region, one swatch per row

- Reverted skipping the ambient medium: every region paints its assigned
  material again, background included. Instead of hiding the background, the
  default medium's colour moved from `[47, 73, 88]` to `[86, 116, 138]`, so it
  reads against the dark canvas at the overlay's default opacity while staying
  calmer than an assigned material. Saved scenes keep their stored colour.
- Dropped the second swatch. A region row showed both its categorical subdomain
  colour and its assigned material's colour, which read as two colours per
  material. Subdomain assignment rows now carry only the subdomain colour, and
  the Library carries only the material colour and its editor.

## 2026-09-14 — Materials overlay contrast and a reachable material colour

- The Materials overlay was drawing, but every pixel of it was the ambient
  medium. `Material::default_medium` is `[47, 73, 88]`, chosen to sit close to
  the canvas, and the bundled examples assign materials only a few steps away
  from it, so the whole domain painted one near-canvas wash at 39% alpha and read
  as nothing. It only looked broken beside the new Subdomains palette.
- Regions still carrying `DEFAULT_MATERIAL` are now left unpainted: the ambient
  medium reads as the canvas and an assigned material stands out against it. A
  scene that deliberately assigns a non-default material to the background still
  paints it.
- A material's colour had no editor anywhere in the UI — probes had one,
  materials did not — so the only key the overlay uses was invisible and
  unreachable. The Library rows gained a colour button, and each region row shows
  both its categorical subdomain swatch and its assigned material's colour, so
  the panel and either overlay agree.

## 2026-09-14 — Visible material colours and click-to-select subdomains

- The Materials overlay drew nothing visible because every material carried the
  same colour. `add_material` cloned `Material::default_medium()` wholesale,
  inheriting its `[47, 73, 88]` slate — deliberately close to the canvas for the
  ambient background, and therefore invisible once every new material shared it.
  The pre-swap editor cycled a six-colour palette per material id; restored, so
  the overlay separates materials again while the background stays subdued.
- Clicking inside a face now selects that subdomain. The click already cleared
  the geometry selection; it resolves the committed face under the pointer
  through `snapshot.face_at` and the plan's domains, and sets the region
  selection the Materials panel reads.
- The derived-boundary highlight no longer needs the Materials panel to be open:
  it also shows while either region overlay is active, so a click has visible
  feedback wherever region colour is on screen.
- Added a regression asserting that successive materials take distinct colours
  and none reuses the background's.

## 2026-09-14 — A subdomain overlay that shows subdomains

- The View combo listed the entry as **Subdomains** while the closed combo and
  every other caller read **Material regions**, because the item label was a
  hardcoded string beside `MaterialOverlay::label_for`. Labels now come from the
  enum in both places.
- The overlay also did not show subdomains: `MaterialOverlay::Regions` colours
  each triangle by its region's *material*, so every face using the default
  medium — including the background — painted the same wash and nothing read as
  a separate subdomain.
- Split the two intents the plan already called for. **Materials** keeps the
  assigned material colour; the new **Subdomains** takes a categorical colour
  keyed by the region's position in the authored list, so neighbouring faces that
  share a material stay distinct. The stored codec gained the variant; existing
  files decode unchanged.
- The Materials panel now carries the same categorical swatch on each region row,
  and picking a row outlines that subdomain's complete derived boundary in the
  viewport, which is what the plan asked for and nothing implemented.
- Formatting, 372 workspace tests, and workspace Clippy with warnings denied pass.

## 2026-09-14 — One chord per selection, and junctions that let go

- **Straighten selection** did the same thing as **Straighten spans** in practice.
  It refused any run whose ends were not already C0 (`Isolate the selection at C0
  before straightening it`), so the only way to reach it was to run the other
  command first, by which point every internal knot was already a corner and both
  produced the same polyline. It now isolates the run's ends itself, raises the
  interior knots to C0 exactly, and lays the whole run on one chord between its
  outer breakpoints. Measured against the per-span result on three spans of a
  circle, the two now differ by 2.5e-1 instead of 3.4e-2.
- Contiguity is now a precondition rather than an accident: `contiguous_run`
  finds the single run per curve (wrapping through a closed seam), a split
  selection is refused with a specific reason, and the button is disabled through
  `selection_is_contiguous`. A junction strictly inside the run is refused
  because straightening would drag a point another curve shares; the run's own
  end breakpoints may be junctions and stay put.
- Removing or detaching a separator left a ghost junction behind.
  `prune_unused_vertex` only dropped a vertex with no references at all, but the
  curve the separator had attached to still carried the C0 breakpoint that
  materialised the junction. That breakpoint kept `vertex: Some(..)` forever, and
  `set_curve_continuity` refuses any node with a vertex, so an ordinary corner
  was permanently locked out of a C1/C2 upgrade. `prune_dangling_junctions` now
  releases interior vertices with fewer than two incident breakpoints and clears
  the references pointing at them. Outer attachments and free tips are kept: they
  still constrain their breakpoint.
- Added regressions for collinearity across a straightened run, span identity
  survival, split and wrapping and whole-loop contiguity, and a separator removal
  that leaves its host corner smoothable again.

## 2026-09-14 — Vector overlay lists only drawable modes

- The View combo offered the complementary-field mode in mechanical scenes, where
  `vector_overlay_samples` has no arm for it and returns no arrows. Selecting it
  silently emptied the overlay. `VectorOverlay::choices` now drives the combo, so
  the mechanical skin lists Off and energy flow only.
- `VectorOverlay::resolved` maps a stored complementary-field mode onto energy
  flow for the mechanical skin, so a scene authored in EM and switched or
  reopened as mechanical draws energy flow rather than nothing. Both the combo
  and the draw path resolve before use, and the smoothing cache keys on the
  resolved mode so a switch resets its running average.

## 2026-09-14 — Grab from the press point, not the drag point

- Found why grabbing felt worse than before the swap even after the radii were
  restored: egui only reports `drag_started` once the pointer has travelled past
  `max_click_dist` (6 px), and the cutover's handler hit-tested at that already
  displaced position. A 10 px radius therefore left under 4 px of real margin,
  and none at all on a fast flick. The pre-swap viewport drove its own pointer
  state machine and tested on press. Every grab test now uses
  `pointer.press_origin()`, while the motion itself still starts from the live
  pointer so nothing jumps by the threshold distance.
- Grab radii also now exceed the drawn control rather than matching it:
  handles and probe grips 13 px, bodies and spans 9 px, region badges 15 px,
  the point source 13 px, all still floored at 18 px under touch. A direct test
  pins the invariant against the drag threshold.
- Delete, Backspace, Enter, and Escape no longer reach the viewport while a text
  field has focus, so renaming a probe no longer deletes it. The pre-swap
  `typing` guard had been dropped entirely.
- Added the missing **Boundaries** probe toggle to View — `boundary_probes`
  existed in the document and was the only probe class with no switch — plus a
  **Probe names** toggle behind a new `probe_labels` presentation field, stored
  with a serde default so existing version-22 files still load.
- Boundary-law strokes became a diagnostic layer drawn under the curves instead
  of over them, and they step outside a selected span's width. A selected span
  also carries a dark halo, so it stays readable over the law colours and a
  bright field.
- Formatting, 367 workspace tests, and workspace Clippy with warnings denied pass.

## 2026-09-14 — Probe manipulation and pre-topology handle sizing

- Restored probe hit testing as one `hit_probe` pass with the pre-swap
  tolerances (endpoint and radius grips 10 px, bodies 7 px, badges 11–12 px, all
  floored at 18 px under touch). A click selects the probe under the pointer
  before geometry sees the event, so a point probe no longer has to be dragged to
  be selected.
- Line probes have their endpoint grips back and gained a body drag; disks
  regained their radius grip. A drag re-applies its delta to the target captured
  at gesture start rather than accumulating, and Shift snaps as it does for
  geometry.
- Every probe kind now resolves an explicit badge point, so all of them carry a
  name label: point at its marker, line at its midpoint, boundary at the
  arclength midpoint of its drawn path, disk at its centre, region at its mesh
  anchor. Boundary probes regained the midpoint badge and region probes the "A"
  badge with the face outline. Failed probes draw red and non-recording probes
  grey, matching the pre-swap rule.
- Handle and marker sizes and strokes come from `2bfa853`: control handles are a
  4 px fill (6 px active) inside a 1.5 px ring, junctions 5.5/7 px in gold, point
  probes 4.5/6 px inside a white 7/9 px ring, boundary badges 5.5/7 px, region
  badges 7/9 px, and the point source is the gold 7 px crosshair again rather
  than a small filled dot. Geometry hit radii went from a flat 8/7 px to the
  pre-swap 10/7 px with the same touch floor.
- Added direct tests for arclength midpoints, label anchors across all probe
  kinds, hit classification including both endpoints and the radius grip, and
  endpoint versus body dragging.
- Formatting, 366 workspace tests, and workspace Clippy with warnings denied
  pass.

## 2026-09-14 — Probe readouts restored

- The GPU recorders never stopped producing the full payload; the cutover's host
  code discarded it. `ingest_probes` kept only `displacement` out of the five
  fields in `PointProbeRecord`, and the readouts drew one un-navigable trace per
  probe. Point readouts again plot the primary field, velocity or transverse
  magnitude by physics skin, Poynting magnitude in EM, and local energy density.
- Line and boundary readouts have their four quantities back (primary field,
  transverse magnitude, normal flux, energy density), each available versus
  arclength, as a waterfall, or integrated over the path versus time, behind the
  compact Plots grid with its waterfall gain. Boundary path length and closure now
  come from the committed stencil, so the arclength axis and the trapezoidal
  integral are correct for periodic curves.
- Area readouts expose mean and RMS primary field, RMS transverse magnitude, mean
  energy density, and total energy. The far field regained its instantaneous and
  visible-window time-averaged 40 dB polar patterns beside the waterfall and
  radiated power, plus its own Plots menu and gain.
- Every trace in a readout shares one pan/zoom time window with a Live button, and
  the Probes panel carries a per-probe **Plot** toggle, a rename field, color,
  sampling preset, boundary trace side and direction, segment end swap, disk
  radius, and the committed compilation status.
- Probes are labelled in the scene. Point, disk, and region probes label their
  marker, line probes label their midpoint, and boundary probes label their first
  sampled point; region anchors come from the committed mesh and are cached per
  topology token.
- Formatting, 362 workspace tests, and workspace Clippy with warnings denied pass.
  The readouts still need an interactive pass.

## 2026-09-14 — Topology-shaped performance diagnostics

- The performance window did not survive the cutover: the lower-right status
  summary was a plain label with nothing behind it, and every instrument it used
  to show belonged to the retired incremental repair path (local attempts, reuse
  percentage, fallback histograms). Rebuilt around what the unified engine
  actually has.
- The status summary is a button again, with a warning marker beside it. A
  preparation or adaptation error opens the window once; ordinary rebuilding does
  not. Sections are Frame, Topology, Mesh, Handoff, and Solver.
- Added `TopologyPreparationTiming` to the runtime. Each candidate accumulates
  wall-clock milliseconds per phase plus its slice count and longest slice, and
  carries them into `PreparedTopology`, so the cooperative mesh phase and the
  synchronous assembly/transfer/probe/far-field tail can be told apart. The live
  job's breakdown is readable while it runs.
- Handoff now records the three waits separately: CPU preparation, draining the
  solver's requested steps, and GPU upload. Each completed transaction also
  reports what it reused, whether the field was transferred or reset, and the
  rebuild reason behind a full remesh, using a new
  `TopologyFullRebuildReason::label`.
- Solver reports GPU status, dispatches, DOFs and estimated buffer size, dt,
  throughput in simulated seconds per wall second, and the outstanding step
  backlog against the per-frame ceiling, which is what the pacing fix above
  bounds.
- Formatting, 362 workspace tests, and workspace Clippy with warnings denied
  pass. The window itself still needs an interactive pass.

## 2026-09-14 — Bounded solver pacing during a handoff

- A prepared candidate could sit in **Ready for GPU upload** indefinitely. The
  upload waits for `WaveGpuRequest::caught_up`, an exact match between requested
  and completed steps, but the cutover dropped the host-side pacing that made
  that reachable: steps were requested every frame at wall-clock rate with a
  4096-step ceiling, an unclamped frame delta, and an unclamped accumulator,
  while the render node encodes at most 64 steps per frame. The backlog then
  grew monotonically whenever `1/dt` exceeded `64 x fps`, and the rebuild's own
  slow frames built a debt that took seconds to drain even when it did.
- Restored the withheld schedule: while a candidate is ready or uploading, both
  continuous and manual stepping pause without touching the user's Run/Pause
  preference or a pressed Step. Requests are now capped by the render node's own
  `MAX_STEPS_PER_FRAME`, the frame delta is clamped to 100 ms, and unspent
  wall-clock time beyond one frame of steps is dropped rather than queued.
- The commit gate now compares the readback length against the uploading
  candidate's own degree-of-freedom count. It previously read the runtime's
  pending slot, which a newer edit clears mid-upload, leaving `uploading` stuck.

## 2026-09-14 — Junction-safe control deletion

- A reshaping control deletion no longer tears an incident junction off its
  authoritative topology vertex. `remove_control` and the approximate continuity
  change now re-pin every vertex-bearing breakpoint through
  `synchronize_vertices`. Deleting a control next to a junction previously moved
  that C0 breakpoint by up to `5.3e-2` world units and left the document stuck in
  `VertexMismatch`.
- Removing the control attributed to a closed curve's first knot interval, or an
  open curve's leading interval, moves the curve's parameter origin. Face anchors
  on that curve now shift by the same amount before they are re-attributed to a
  span, and an anchor left without an interior parameter is recentred on its
  surviving span. Without that shift an anchor could drift backwards across a
  junction into a face another assignment already owned, which is the reported
  `DuplicateFace` after several deletions.
- The accepted-reference ghost drawn under an invalid draft no longer takes the
  current span selection. Selection emphasis now belongs to the interactive pass
  only, matching handles, control polygons, and boundary-law strokes.
- Added a direct regression over a subdomain split by an inner separator: seam
  control deletion keeps every junction exactly on its vertex, keeps all three
  compiled domains, and keeps the region set stable.
- Formatting, all 362 workspace tests, and workspace Clippy with warnings denied
  pass.

## 2026-09-14 — Selection-preserving geometry drag

- Starting a geometry drag on any span already in the current selection now keeps
  the complete span selection intact. Whole curves and multi-span C0 selections
  therefore translate as one rigid object; an ordinary click still reduces or
  modifies selection according to its modifiers.
- Added a direct regression covering selected spans, unselected spans, and control
  handles at drag start.
- Fixed control attribution during deletion on C0/C1 curves. A raw B-spline
  control is now mapped through knot multiplicities to its logical corner and
  span. That corner is smoothed automatically before the reshaping deletion;
  unrelated corners retain their continuity, and the complete operation remains
  one history action. Core bounds checks still prevent direct ambiguous indexing.

## 2026-09-14 — Open-curve attachment snapping repair

- Fixed initial attachment hits on inner curves: the cursor's screen-side position
  now chooses the authored curve's left or right face instead of always choosing
  left. Once a separator starts, its other endpoint remains filtered to that same
  active face.
- Open-curve drawing highlights every eligible outer edge, inner curve segment,
  and authored junction. A gold ring and **Attach** label show the exact projected
  point, and the live segment ends at that point before the click commits it.
- Preview and commit share one 14-pixel attachment query. Direct regression
  coverage checks an inner hole from its active side at two zoom levels and proves
  the measured snap distance remains three screen pixels.

## 2026-09-14 — Unified spline authoring tools restored

- Restored topology-native C2/C1/C0 editing at the end knot of a selected span.
  Sharpening is shape-preserving; smoothing first tries exact knot removal and then
  uses the spline's bounded least-squares projection. Authored junctions remain C0.
- Restored **Isolate at C0** for partial span selections and per-span
  **Straighten spans**. Both preserve stable span IDs and dependent boundary laws,
  probes, and face anchors, and each completes as one undoable document action.
- Completed the viewport transform gizmo with a draggable center of rotation and
  scale, a rotation ring with a grab cursor, and separate uniform, X, and Y scale
  grips with directional cursors. Geometry itself remains the translation target.
  Shift snaps coordinates to 0.05 world units, angles to 15 degrees, and scale to
  0.1 increments without conflicting with Shift span selection. The gizmo appears only when the
  topology transform planner accepts the current whole-curve or C0-isolated
  selection.
- Added direct command regressions for exact isolation across a closed seam, stable
  identities, undo atomicity, approximate continuity upgrades, junction protection,
  and independent chord straightening.

## 2026-09-14 — Marquee intent feedback restored

- Restored the live marquee label, operation-colored fill, solid blue enclosure
  border, and dashed teal crossing border. Left-to-right requires full enclosure;
  right-to-left accepts any crossed span.
- Replace, Shift-add, and Alt-subtract now update both the label and selection while
  the pointer remains down. Modifier changes are read again at release, and Escape
  restores the selection captured at drag start.
- Added direct tests for direction classification and baseline-relative replace,
  addition, and subtraction.

## 2026-09-14 — Atomic topology application cutover

- Replaced the production editor with the unified topology document and removed
  the legacy example/UI module. Version 22 is now the only schema used by file
  load/save, shared links, recovery, autosave, and all eight bundled examples;
  older schemas have no production adapter.
- One immutable topology token now drives rendering, cooperative meshing,
  quadratic assembly, solution transfer, GPU publication, volume and point
  sources, AMR, point/line/boundary/area probes, far field, and material overlays.
  Adapted meshes receive a distinct mesh generation and cannot publish until the
  matching GPU upload is acknowledged. Source/probe-only edits update in place;
  material and boundary-law edits reuse the mesh and transfer the field.
- The visible editor now creates unified open and closed curves, attaches
  separators only across one active face, edits shared junctions, assigns
  left/right span laws, removes dividers with explicit region ownership, edits the
  outer rectangle, and keeps invalid drafts over the accepted reference. Rendering
  restores the persisted grid, control-polygon, handle, boundary-law, mesh,
  mesh-boundary, field, material, AMR, vector, probe, and far-field overlays.
- Restored direct rotation and uniform-scale gizmos, topology-native
  shape-preserving double-click insertion, guarded reshaping control deletion,
  probe selection/deletion, material parameters, region profile frames, and
  region-owned volume-source editing without reintroducing legacy identities.
- Restored topology-backed capture, sharing, examples, responsive inspectors, and
  two-finger touch pan/zoom. The first catalog entry is the startup document.
- A production-source audit finds no legacy geometry IDs or calls to legacy scene
  meshing in the active UI/runtime path. Legacy editor and codec modules remain
  compiled only for their existing equivalence/regression suite and shared scalar
  serialization helpers.
- Verification passes: formatting; workspace Clippy with warnings denied; all 349
  workspace tests; native release compilation and startup on Apple M1 Max/Metal;
  and `NO_COLOR=false trunk build --release` with Trunk 0.21.14. Interactive
  browser testing was left to the user as requested. The UI advances topology
  preparation in fixed 256-work-unit slices and AMR/overlay jobs in bounded
  cooperative slices; representative browser frame timing remains to be measured.
- Topology coordinate and graph edits deliberately use the cooperative full
  rebuild. Sector-aware local mesh repair is the next performance follow-up.

## 2026-09-14 — Topology-native viewport interaction contract

- Added an egui/Bevy-independent viewport model keyed only by stable `CurveId`,
  `CurveSpanId`, and `TopologyVertexId`. Adaptive rendering samples carry their
  exact span identity; screen-space hit testing gives authoritative junctions and
  controls priority over curves.
- Selection now has one exclusive semantic shape: a single control/junction or a
  set of spans. Shift toggles spans, whole-curve selection expands by stable IDs,
  and marquee direction chooses full enclosure or crossing selection.
- Rigid multi-span planning uses spline support controls and requires partial
  sections to end at C0 breakpoints. A partially selected shared junction returns
  `Junction also belongs to unselected spans` plus the missing incident spans;
  expanding the selection transforms the authoritative vertex exactly once.
- Added topology-editor transform commands suitable for a live drag transaction.
  Repeated updates between `begin` and `commit` produce one undo entry, while
  cancellation restores the exact pre-drag draft. Outer attachments remain on
  their side and update their stored normalized fraction.
- Added face-filtered attachment hit testing for outer sides, curve interiors, and
  authored junction sectors, plus compiled span context for coherent parameter-
  relative Left/Right laws and active-side display.
- Direct tests cover hit priority, Shift/whole-curve selection, both marquee
  directions, partial-junction blocking and recovery, same-face attachment
  filtering, active trace-side context, one-entry dragging, undo, and cancellation.
  All 478 workspace tests, workspace Clippy with warnings denied, native release
  compilation, and the release Trunk build pass. Next: bind this contract to the
  visible egui viewport and contextual inspector during the production document
  cutover.

## 2026-09-14 — Atomic topology runtime preparation

- Added immutable accepted-topology tokens that bind one document revision to its
  authored scene, compiled arrangement, and mesh plan. CPU preparation now carries
  that token through cooperative meshing, topology operator assembly, transfer,
  volume sources, point/line/boundary/area probes, and far-field compilation.
- Added a publication coordinator with an explicit GPU-acknowledgement boundary.
  Ready CPU data cannot replace the active runtime on its own; failed, rejected,
  superseded, and stale candidates leave the committed state untouched.
- Source, probe, and far-field-only edits reuse the exact topology, mesh, operator,
  and compiled volume sources. Material or boundary-law changes reuse the mesh and
  prepare a transfer into the reassembled operator. Coordinate and graph changes
  report a typed cooperative full rebuild.
- Restored probe enabled/color semantics in the version-22 topology document and
  added atomic topology-editor commands for domain, physics, materials, regions,
  frames, sources, far field, and probes. Load now reseeds material and probe IDs in
  addition to geometry identities.
- Direct transaction tests cover publication acknowledgement, exact non-geometric
  reuse, material-only transfer, coordinate rebuild classification, topology probe
  compilation, far-field disable status, and active-state retention after failures
  and supersession.
- All 470 workspace tests, workspace Clippy with warnings denied, native release
  compilation, and the release Trunk build pass.

## 2026-09-14 — Topology version 22 persistence and examples

- Added a strict version-22 topology document codec. It stores independent draft
  and accepted authored scenes, stable face anchors, spline topology, span laws,
  regions and material frames, formulas and sources, topology probe targets,
  far-field settings, and presentation state. Compiled topology, mesh/cache state,
  selection, camera, transient readouts, and history stay out of the file.
- Version 22 is a hard schema break. Versions 1–21, unknown fields, malformed IDs,
  dangling references, excessive documents, and invalid accepted scenes fail before
  document replacement. Structurally valid invalid drafts remain loadable and
  editable beside their independently validated accepted scene.
- Re-authored all eight built-in examples directly in the unified topology model.
  Their full source, probe, material, field-overlay, EM polarization, and far-field
  semantics survive exact compact-JSON round trips; no legacy scene adapter is used.
- Added the topology editor load constructor. It compiles accepted state, clears
  undo/redo, starts revisioned draft validation, and reseeds curve, span, region,
  and topology-vertex allocators across both stored snapshots.
- All 462 workspace tests, workspace Clippy with warnings denied, native release
  compilation, and the release Trunk build pass.

## 2026-09-14 — Headless topology document editing

- Added the application-side topology document model with draft/accepted snapshots,
  stable ID allocation, revisioned cooperative validation, and complete snapshot
  undo/redo. Closed subdomain and hole creation, free baffle creation, coordinate
  edits, and bulk span-law edits use the same atomic command path.
- Added boundary-interior and junction-sector attachment targets. Open-curve
  commands now create free, singly attached, or doubly attached baffles and require
  transmitting separators to attach twice to the same active face. Inner-curve
  attachment performs exact C0 insertion and remaps face anchors and boundary-probe
  paths before accepting the candidate.
- Curve removal automatically keeps the only active neighboring region. Merging
  two active regions requires an explicit survivor; dropped region sources and
  area probes, plus probes attached to the removed curve, disappear in the same
  undoable command. When the chosen exterior material carries a non-background
  region ID, its dependents are retargeted to the stable background identity and
  the previous background dependents are removed. Endpoint detach remains an editable invalid draft for a
  separator, and reattachment to the original junction restores the exact scene.
- Changed excluded-face boundary semantics. An authored transmitting curve with one
  active neighbor now compiles to an effective homogeneous Neumann wall on that
  side; the authored transmission intent is retained and returns if the face is
  reactivated. Curves surrounded only by excluded faces are valid but inert.
- Direct tests cover invalid-draft retention and cancellation, one-entry creation
  and span edits, stable accepted/draft undo, the effective wall rule, and actual
  meshing of an excluded face bounded by an authored transmitting curve.
- All 456 workspace tests, workspace Clippy with warnings denied, native release
  compilation, and the release Trunk build pass.

## 2026-09-14 — Authored topology scenes and cooperative full meshing

- Added dependency-free `TopologyScene`, `FaceAnchor`, and explicit authored face
  dispositions. Stable outer/curve-side anchors resolve to snapshot-local faces;
  deliberate exclusion is distinct from a missing invalid-draft assignment.
- Structural validation covers material/region/source identity, outer corner laws,
  total face disposition, duplicate ownership, malformed anchors, and plan
  compatibility. A same-face endpoint query provides the contract needed to filter
  valid Subdomain separator completion targets.
- Added `TopologySceneJob` around the resumable arrangement compiler and replaced
  the synchronous topology triangulator internals with `TopologyMeshingJob`.
  Bridge search, ear clipping, legalization, refinement, separated-curve recovery,
  and verification now expose useful phases and publish once.
- Changed the current Draw popover to geometry-first **Closed curve** and **Open
  curve** groups. Each remembers its own initial-purpose choice. The live legacy
  separator already requires attached endpoints; same-face/sector filtering lands
  with the topology editor because the legacy mesh cannot express that contract
  reliably.
- Slice-size determinism is covered directly. All 443 workspace tests, workspace
  Clippy with warnings denied, native release compilation, and the release Trunk
  build pass.

## 2026-09-14 — Planned atomic topology application cutover

- Specified an authored `TopologyScene` with stable oriented face anchors. Compiled
  `FaceId`s remain snapshot-local cache data and will not enter persistence,
  history, selection, probes, or source ownership.
- Split the implementation into document/anchor contracts, a cooperative topology
  mesh job, headless topology editor commands, a version-22 persistence and example
  hard cut, one atomic UI/runtime switch, and legacy production-path removal.
- Defined a geometry-first Draw popover with Closed curve and Open curve groups.
  Closed curves choose an initial Subdomain/Hole purpose; open curves choose
  Subdomain separator/BC baffle. Separator drawing must start and finish on valid
  boundaries of the same active face; unattached completion is refused, while
  baffles retain free-end support. Also specified unified curve/span/junction
  selection, partial-junction transform handling, contextual span and face
  controls, explicit material choice on ambiguous divider removal, and
  draft-derived subdomain overlays. Invalid topology and unassigned faces remain
  editable and get localized viewport feedback.
- The live transaction will carry one immutable authored scene, topology snapshot,
  and mesh plan token through meshing, assembly, transfer, GPU upload, AMR, probes,
  overlays, and far field. A failed or stale candidate leaves the accepted running
  state untouched.
- Topology coordinate edits initially use the verified cooperative full rebuild.
  Local graph repair remains a measured follow-up rather than a cutover blocker.

## 2026-09-14 — Topology-aware far-field compilation

- Added a dependency-free `QuadraticFarFieldStencil` compiler for topology plans.
  It produces a bounded rectangular midpoint contour, outward normals, topology
  point stencils, sample spacing, wave speed, and the common retarded-time delay
  margin needed by the existing GPU projection.
- Exterior classification now comes from stable outer-boundary face ownership.
  More than one incident outer face is rejected even if both regions currently
  evaluate to the same constants, so an interface cannot silently cross the
  Huygens contour.
- Clearance uses the plan's sampled curve segments and a fixed world-space margin.
  Internal baffles and inclusions are allowed; curves entering the exterior shell
  receive a specific enclosure error.
- The compiler requires a uniform, isotropic, lossless exterior with no active
  region source. Enabled point sources must lie inside the contour. Spatial and
  driven materials in fully enclosed faces remain valid.
- Structured failures cover malformed revisions, inset and output bounds, exterior
  topology, material properties, source placement, mesh sampling, and a contour
  that resolves to the wrong face. Direct tests cover an internal baffle, a
  spatially varying driven inclusion, outer-face partitioning, shell intrusion,
  anisotropy, and both point and region sources.
- All **430 workspace tests**, workspace Clippy with warnings denied, native release
  compilation, and `NO_COLOR=true trunk build --release --locked` pass.

## 2026-09-14 — Topology-aware ordinary probe stencils

- Added topology entry points for quadratic point, boundary, disk, and region probe
  stencils. Line probes inherit the point behavior because their spatial profiles
  are compiled as repeated point samples.
- Unified material lookup behind a shared probe model. Topology probes evaluate
  anisotropic or spatial coefficients from explicit active face regions without
  reconstructing legacy obstacles or dividers.
- Point samples reject every unified curve constraint as ambiguous; this preserves
  gaps when a line crosses a separated trace and avoids choosing an arbitrary
  material at a transmitting interface.
- Added `BoundaryStencilTarget` for stable curve/span/side, parameter, period, and
  adjacent-region selection. Topology boundary probes validate that target against
  the plan before binding a mesh edge, and separated left/right samples retain
  distinct nodes and opposite outward normals.
- Direct tests cover active versus merely library-present regions, whole-face area
  integrals, line gaps across separated curves, both baffle traces, transmitting
  face assignments, and ambiguous material-interface points. Far-field and
  application-level boundary-path metadata remain separate follow-up slices.
- All **428 workspace tests**, workspace Clippy with warnings denied, native release
  compilation, and `NO_COLOR=true trunk build --release --locked` pass.

## 2026-09-14 — Topology-aware fixed-geometry AMR

- Added `MeshAdaptationJob::new_topology` while retaining one resumable adaptation
  engine and all legacy behavior. The job owns its plan contract, active regions,
  physical boundary atoms, paired sides, and trace points.
- Import now preserves `MeshVertex::trace`; trace vertices and sampled plan
  endpoints are pinned independently from AMR lineage. Refinement and coarsening
  operate only inside one immutable sampled segment, so adaptation cannot change
  the topology compiler's curve approximation or merge junction sectors.
- Transmitting curves remain shared two-element constraints. Separated curves with
  two active faces split and collapse both traces atomically using stable
  curve/span/parameter identity; a hole with one active face adapts as a one-sided
  constraint without requiring a nonexistent partner.
- Added preflight and publication checks for active regions, exact plan-chain
  coverage, endpoint geometry and trace identity, expected adjacency and incident
  regions, and identical subdivisions on paired traces. Contract and work-limit
  failures retain the source mesh and adaptation state.
- Normalized topology baffle sample metadata when the full mesher replaces its
  temporary legacy trace labels. This makes edge-authoritative curve-side metadata
  consistent for later AMR and transfer consumers.
- Direct tests cover deterministic rectangle adaptation, transmitting region
  dividers, paired free baffle refinement/coarsening plus quadratic transfer,
  one-sided hole refinement/coarsening, fully separated T-junction sectors, a mixed
  transmitting/separated junction, and malformed trace rejection.
- All **425 workspace tests**, workspace Clippy with warnings denied, native release
  compilation, and `NO_COLOR=true trunk build --release --locked` pass. Application
  wiring remains deferred until the document cutover can switch every numerical
  consumer to topology labels atomically.

## 2026-09-13 — Topology-aware AMR indicator

- Added `SolutionIndicatorJob::new_topology`, which owns the plan and material
  model for deterministic bounded work while retaining the existing estimator,
  grading, and target-field behavior.
- Topology outer edges and separated curve sides now resolve their laws by stable
  source label and parameter interval. Transmitting curves enter the ordinary
  two-element flux-jump path; paired thin gaps use curve/span identity rather than
  legacy baffle IDs.
- Material sampling and recovery use explicit triangle regions. Four material
  sectors may meet at one shared junction without cross-region gradient averaging,
  and the output field continues to require the requested region during lookup.
- Direct tests compare the scene and topology paths exactly on a one-face mesh,
  exercise all four regions of a crossing junction, and process both sides of a
  coupled topology baffle.
- All **418 workspace tests**, workspace Clippy with warnings denied, native
  release compilation, and release Trunk WebGPU packaging pass.

## 2026-09-13 — Topology-aware material and volume-source lookup

- `TopologyWaveModel` now exposes region/material lookup and scalar or directional
  material evaluation from the explicit plan libraries. Legacy scenes and topology
  consumers share the same evaluator, including material frames, anisotropy, and
  mechanical/EM coefficient conversion.
- Added a resumable topology volume-source compiler. It validates source regions
  against active face assignments, owns an immutable material/source snapshot, and
  accumulates forcing by the region labels already carried by mesh triangles.
- Replaced the two-volume-source-per-node representation with arbitrary sparse
  contributions. The WebGPU path packs a node header plus channel/weight pairs in
  the existing eighth storage binding, so a junction pays only for its actual
  incident sources and ordinary nodes stay compact.
- Direct tests cover region-frame evaluation, cooperative compilation, inactive
  face rejection, four independently driven faces meeting at one transmitting
  junction, source isolation by triangle region, and sparse GPU packing.
- All **415 workspace tests**, workspace Clippy with warnings denied, native
  release compilation and Metal shader startup, and release Trunk WebGPU packaging
  pass.

## 2026-09-13 — Separated and mixed junction meshing

- Separated curves are now divided into recovery runs whenever either side's
  compiled trace changes at a junction. All runs are cut before junction vertices
  are resolved, so every incident arm is present when the triangle fan is split.
- The final fan pass treats separated curve edges as barriers, follows transmitting
  edges across the junction, and assigns each resulting angular sector to the
  `TraceVertexId` supplied by the topology plan. Free tips still reconnect, while
  three reflecting arms receive three coincident finite-element vertices.
- Added direct meshing coverage for a fully separated T-junction and for a
  separated branch attached to a transmitting material divider. The former also
  assembles a quadratic operator and round-trips an arbitrary nodal state through
  the topology-aware transfer map without mixing sectors.
- All **411 workspace tests**, workspace Clippy with warnings denied, native
  release compilation, and release Trunk WebGPU packaging pass.

## 2026-09-13 — Topology-aware quadratic solution transfer

- Unified separated curves now retain their stable `CurveId` plus left/right side
  through quadratic handoff. Span IDs are deliberately omitted from transfer
  identity, so knot insertion and span splitting do not break side lineage.
- Junction vertices use snapshot trace IDs to narrow coincident source candidates
  within the matching curve sides. When an edit renumbers a snapshot trace or
  moves the trace outside its old adjacent element, transfer falls back first to
  the stable curve side and then to ordinary containing-element lookup.
- Topology meshes no longer require equal region IDs across a handoff. A direct
  full-mesh fixture splits one face with a transmitting divider and merges it back;
  a quadratic field transfers in both directions with zero exposed nodes. Legacy
  scene meshes retain their material-component restriction for closed walls.
- Added direct regressions for unified baffle-side isolation, three coincident
  junction sectors, and transmitting face split/merge. The focused **11 transfer
  tests** pass. All **409 workspace tests**, workspace Clippy with warnings denied,
  native release compilation, and release Trunk WebGPU packaging also pass.

## 2026-09-13 — Topology-capable GPU solver upload

- Removed the fixed two-region membership table from GPU node packing. Arbitrary
  region membership is collected on the host, while the packed node stores only
  whether it belongs to the active point-source region. Both transfer shaders use
  that boolean when reconstructing acceleration; ordinary wave stepping and pulse
  injection use their already region-filtered spatial weights.
- Changing a point source's region now transactionally regenerates both its
  spatial weights and source-membership node buffer. Signal-only updates retain
  both buffers. This preserves exact 64-bit `RegionId` semantics without adding a
  ninth WebGPU storage binding or imposing a new junction-degree limit.
- Unified separated-curve labels now activate mesh-path forcing distance just as
  legacy baffle labels do, so a Gaussian source cannot leak directly across a
  topology-plan baffle trace.
- Added host-side GPU upload coverage for a quadratic node shared by four material
  regions, source-region buffer refresh, signal-only buffer reuse, and equivalent
  forcing behavior for legacy and unified baffle labels.
- Verification passes: formatting, all **406 workspace tests**, Clippy across all
  targets with warnings denied, native and WASM release builds, and native Metal
  startup through live solver execution without a shader or pipeline error. The
  expected forced-shutdown readback warnings appeared after Ctrl-C. Interactive
  browser testing was not repeated.
- The GPU handoff entry points now accept a topology mesh/operator pair. Creating
  those pairs in the interactive editor still awaits the version-22 document
  model; volume-source compilation, transfer maps, AMR, and probes remain the next
  numerical consumers to migrate.

## 2026-09-13 — Topology-plan quadratic operator assembly

- Added direct enriched-quadratic assembly from `TopologyMeshPlan` plus the
  material/region library. Triangle coefficients, spatial material frames, and
  rectangular outer-boundary coefficients now follow the plan's assigned regions
  rather than legacy background or object lookup.
- Unified curve labels drive transmitting-adjacency checks and both separated face
  laws. Impedance, prescribed Neumann/Dirichlet, and second-order auxiliary terms
  share the existing numerical assembly. Matching left/right trace segments add
  the conservative thin-gap spring using both adjacent materials.
- A boundary-law-only plan revision rebuilds the operator against the same mesh.
  The operator retains that mesh's geometry/mesh revisions so GPU validation and
  identity transfer remain coherent. Four-region crossing junctions assemble
  without a two-region operator assumption; constant fields remain in the
  stiffness nullspace.
- Corrected free-slit trace lineage at logical knots to use the configured curve
  tolerance. Exact coordinate equality could miss a recovered collinear vertex by
  roundoff even though the correct curve-side trace was present.
- Direct tests cover numerical equivalence with the established coefficient path,
  a four-region X junction, mixed impedance/driven baffle sides, law-only mesh
  reuse, and paired thin-gap force conservation/timestep tightening.
- Verification passes: formatting, all **404 workspace tests**, Clippy across all
  targets with warnings denied, native release compilation, and the release Trunk
  WebGPU build. Interactive browser testing was not repeated for this numerical
  core slice.
- The application transaction, sources, AMR, transfer, and probes still use legacy
  scene labels. They will move in follow-up consumer slices; this CPU entry point
  alone does not change interactive behavior.

## 2026-09-13 — Topology-plan full meshing baseline

- Added a full constrained-mesh path that consumes `TopologyMeshPlan` directly.
  It triangulates assigned faces without recreating legacy obstacles or dividers,
  shares complete transmitting chains, duplicates separated traces, and retains
  `CurveId`, `CurveSpanId`, side, and junction `TraceVertexId` lineage in the mesh.
- Free and one-ended baffles use the proven recover-and-cut path after face
  triangulation. Attached tips are rewired to the exact topology sector rather
  than the nearest coincident coordinate, closing the leak-prone ambiguity that
  motivated the unified trace contract.
- Added direct meshing fixtures for the empty rectangle, transmitting and excluded
  closed loops, free and outer-attached baffles, an outer divider, and a
  three-region T junction. The fixtures check area, region count, constraint
  adjacency, two-sided labels, and trace lineage.
- Audited local repair rather than adapting it speculatively. Exact unchanged
  plans reuse the existing mesh, and boundary-law changes preserve the same
  discretization. Coordinate changes currently report
  `CoordinateRepairDeferred` and take the robust full rebuild. The legacy repair
  obtains curve geometry and trace pairing from object-specific `Scene` members
  and cannot safely rewire arbitrary junction sectors; migrating that algorithm is
  a separate slice after solver/AMR consumers use the topology plan.
- The topology full-build entry point is synchronous at this stage. The legacy app
  continues using its cooperative meshing transaction until the document and
  numerical consumer cutover provides a revisioned topology plan to schedule.

## 2026-09-13 — Unified curve topology kernel

- Extended the compiled arrangement with directed face-boundary edge references
  and snapshot-local trace vertices. Sector connectivity now determines whether
  coincident geometry shares a solver node: transmitting rays join sectors,
  separated rays divide them, and free separated tips reconnect without a special
  coordinate rule.
- Added `TopologyMeshPlan`, the checked input contract for the new triangulation
  path. It requires an explicit active or excluded assignment for every bounded
  face, resolves a transmitting span beside an excluded face as an effective
  reflecting wall, rejects coupled spans missing an active side, preserves
  arbitrary region membership at junctions, and emits oriented per-side curve
  constraints with stable curve/span IDs.
- The plan has direct tests for shared transmitting traces, one-sided hole
  boundaries, separated baffle interiors with reconnected tips, complete face
  assignments, and outer attachments. The legacy triangulator does not consume the
  plan yet; this is the remaining half of the mesher cutover.
- Added the dependency-free migration target for one open/closed curve model,
  stable logical spans, authoritative free/interior/outer topology vertices, and
  transmitting or separated trace behavior. Hole, region, divider, and baffle will
  become creation presets rather than stored geometry classes when the app moves
  to this model.
- Added a revisioned topology job that adaptively samples curves and checks segment
  pairs cooperatively. The compiler builds a directed planar arrangement, traces
  bounded faces, reports each span side's face, and retains same-face but distinct
  baffle sides.
- Added exact shape-preserving topology-breakpoint insertion with span-ID lineage
  and periodic C0 breakpoint movement. Outer-constrained vertices recompute their
  world position when the rectangular domain changes and update every incident
  spline breakpoint.
- Added direct core coverage for closed-loop faces, outer-to-outer dividers,
  attachment to a closed loop, T and X junctions, free and one-ended baffles,
  invalid free transmitting ends, incompatible crossings, overlap/near-contact,
  subdivision exhaustion, resize propagation, and revision tagging.
- The planned document cut deliberately drops scene schemas 1 through 21. Built-in
  examples and the checked-in example scene will be regenerated in the new schema;
  obsolete loads will fail atomically with an unsupported-version message.
- The new topology is not yet connected to the legacy `Scene`, mesher, solver, or
  editor. That consumer cutover is the next stage; current behavior is unchanged.

## 2026-09-13 — Open material dividers and junction topology

- Added stable open material-interface, breakpoint-node, and junction IDs. Interface
  spans own explicit left/right regions; interior attachments require C0 and outer
  attachments store an edge plus normalized coordinate.
- Added the staged **Divider** polyline/spline tool. The document changes only after
  both endpoints attach. Eligible outer edges and divider curves are highlighted;
  clicking a target to end the curve finishes the operation. A curve hit inserts a
  shape-preserving C0 breakpoint and junction automatically, and a drawn curve may
  cross existing C0 nodes while assigning the source region independently per span.
- Added junction diamonds and direct junction dragging. Interior moves update every
  attached curve node; outer moves remain constrained to their selected domain edge.
  A complete drag is one undoable edit and Escape restores its snapshot.
- Divider-span selection identifies its junction-to-junction section. Removal merges
  the two adjacent regions with the older ID surviving, splits retained curve pieces,
  collapses redundant two-arm junctions, and removes unused region state.
- The mesher recovers the complete interface graph before flooding faces, retains a
  single conforming trace at T junctions, and assembles the quadratic operator over
  all incident material regions. The incremental validator samples divider geometry
  and rejects free ends, incompatible contacts, invalid sectors, and ambiguous arms.
- Far-field compilation now examines the complete shell outside its inset contour.
  It requires one effective uniform, isotropic, lossless, source-free medium even if
  that shell contains several region IDs.
- Scene JSON is version 21; version 20 and older documents load with empty graph
  fields. Automated coverage includes T/crossing creation, removal, junction drag,
  persistence, three-region meshing/operator assembly, and exterior-shell rejection.

## 2026-09-13 — Directional material tensors

- Added a reusable dependency-free symmetric 2D tensor and a fourth material
  field, **Axis ratio**, with constant/formula input and `a >= 1` validation.
  Region frames orient `A = k R diag(a, 1/a) R^T`; the base coefficient remains
  its geometric mean and the x/y wave-speed ratio is exactly `a`.
- Updated enriched-quadratic volume assembly, the row-sum CFL guard, first- and
  second-order straight-edge radiation terms, thin-gap scaling, and solution AMR
  to use directional flux. The estimator now recovers `A grad(u)`, includes the
  reconstructed tensor divergence in its strong residual, and limits active
  wavelengths with the slow principal speed.
- Updated point, line, boundary, and area probe energy/flux calculations on CPU
  and WebGPU. EM complementary fields rotate `A grad(potential)` and Poynting flow
  uses the same tensor flux. Far-field projection now rejects an anisotropic
  background explicitly while allowing directional inclusions.
- Added the logarithmic **Material anisotropy** overlay with fast-axis marks and a
  local principal-coefficient/speed readout. The **Anisotropic crystal** example
  demonstrates a rotated inclusion, source, probes, and second-order outer edges.
- Scene JSON is version 20. Axis-ratio formulas, the selected overlay, files,
  links, recovery, examples, and history share the document representation;
  versions 1–19 migrate to isotropic ratio one.
- Verification: `cargo fmt --all`, 362 workspace tests, native and wasm32 Clippy
  with warnings denied, native release compilation, and `trunk build --release`
  pass. Interactive browser behavior was not re-exercised in this slice.

## 2026-09-13 — Unified viewport video recording

- Added a shared capture coordinator for PNG and video presentation state. Capture
  requests now cross a frame boundary before the clean presentation fence is
  enabled, so an Export popup painted during the click frame cannot appear in the
  PNG or the opening video frame.
- Added silent, fixed-size 30 FPS recording with a common Export action and a bottom
  status-strip recording indicator, elapsed timer, dropped-frame count, and Stop
  control. Simulation playback, pan, zoom, and persisted View overlays remain live;
  panels, floating readouts, selection emphasis, gizmos, marquees, and prompts stay
  out of the captured viewport.
- Browser builds copy the central WebGPU canvas region into a hidden recording
  canvas, select VP9/VP8 WebM or MP4 through `MediaRecorder`, and download the final
  Blob. Native builds choose a destination after probing FFmpeg, keep one Bevy GPU
  readback in flight, and feed a two-frame bounded worker queue. The worker streams
  RGBA to H.264, VP9, or VP8 and repeats the latest image across missed wall-clock
  slots. Both backends retain fixed even dimensions and letterbox after resize.
- Added crop, even-dimension, letterbox, clean-frame ordering, and capture-state
  coverage. All 354 workspace tests and warning-denied native/WASM Clippy pass. A
  native integration test encoded a real H.264 MP4 with the installed FFmpeg; the
  release native build and release Trunk package pass. The local Chrome/WebGPU suite
  starts and resizes the app, copies the real WebGPU canvas through `MediaRecorder`,
  and verifies a nonempty downloaded video. Full visual inspection remains on the
  local checklist because the GitHub runner cannot initialize WebGPU.

## 2026-09-13 — Physics-switch material conversion

- Corrected the Mechanical/EM skin transition. It previously relabeled `(rho, K)`
  directly as `(epsilon, mu)`, which made mechanical wave speed numerically become
  EM impedance and mechanical impedance become reciprocal EM wave speed. The switch
  now uses `epsilon = 1/K`, `mu = rho`, and `alpha = d/rho`, with the inverse mapping
  on return, preserving local speed, impedance, and normalized damping rate. TM/TE
  changes leave the shared EM law untouched.
- Refactored material parsing to build a private expression tree and then emit the
  existing bounded postfix program. Normal formula entry keeps source text verbatim;
  only physics conversion invokes structural simplification and canonical printing.
  Double reciprocals and exact damping product/quotient pairs cancel, so repeated
  switches do not accumulate wrappers. Zero products and a zero numerator over a
  structurally nonzero material coefficient reduce to constant zero, keeping a
  lossless material's converted `alpha` readable.
- Added a small `?` menu beside the Materials property note with coordinates,
  operators, parameter use, and every available formula function.
- Material conversion is prepared before the editor transaction and fails atomically
  if a generated expression exceeds existing limits. One successful switch remains
  one undo entry. A divergent formula text draft blocks switching rather than being
  silently discarded.
- All 348 workspace tests pass, including spatial invariance, 32 repeated conversion
  cycles, persistence, exact undo/redo, atomic failure, and pending-formula coverage.

## 2026-09-13 — Scene-only viewport PNG capture

- Added **Viewport PNG** between scene-link copying and SVG export. A request waits
  for the export menu to close, captures the current Bevy window through the existing
  wgpu screenshot path, converts the logical central viewport to physical image
  pixels, crops it, and encodes `funfern-snapshot.png`.
- The capture frame follows persisted View settings and retains the in-scene logo,
  field, geometry, source, probes, and requested overlays. It suppresses floating
  windows, selection emphasis, transform/material gizmos, staged construction,
  marquees, cursor previews, and active-tool prompts without mutating live editor
  state. Camera, playback, documents, presentation settings, and history are left
  untouched.
- Native output uses the existing save-dialog path. Browser output uses a Blob
  download after GPU readback, avoiding a delayed file picker that could lose its
  user-activation window. Pixel-mapping and PNG tests cover ordinary, fractional,
  clamped, empty, and orientation-sensitive crops.
- All 342 workspace tests, warning-denied Clippy, native and WASM checks, native
  release compilation, and the release Trunk build pass. Interactive snapshot
  inspection remains on the manual browser checklist.

## 2026-09-13 — Per-span and per-selection straightening

- Split straightening into two explicit scopes. **Straighten spans** turns every
  selected logical span into its own exact endpoint-to-endpoint line, automatically
  increasing all necessary boundary-knot multiplicities to C0 without first
  reshaping the curve. **Straighten selection** retains the previous behavior of
  replacing each contiguous, already isolated selected run with one chord.
- Per-span straightening prepares all loop and baffle spline replacements before
  mutation, keeps interval and logical-span indices stable, and preserves boundary
  assignments and geometry-attached probes. Isolation plus reshaping is one document
  revision and at most one undo entry.
- All 337 workspace tests, warning-denied Clippy, native release compilation, and
  the release Trunk build pass.

## 2026-09-13 — Touch marquee and two-point spline baffles

- Reserved viewport navigation on touch for the two-finger centroid/pinch gesture.
  An empty one-finger drag now starts the ordinary directional marquee in Select,
  while feature hits still directly manipulate handles, curves, probes, sources,
  domain edges, and gizmos. A second finger cancels the tentative marquee before
  navigation takes ownership.
- Removed the separate **Straight** baffle primitive. **Baffle → Spline** now has
  two valid completion forms: exactly two entered controls expand to the existing
  exact straight cubic representation with equidistant controls, while four or
  more controls create the freeform open spline. Three entered controls deliberately
  remain incomplete. Polyline continues to provide explicit C0 interpolation
  vertices.
- All 335 workspace tests, warning-denied Clippy, native release compilation, and
  the release Trunk build pass. Real-device touch interaction remains on the manual
  browser checklist.

## 2026-09-13 — Drawing primitives and generalized straightening

- Replaced the ambiguous **Custom** choice with explicit role-specific catalogs.
  Hole and Interface offer Circle, Rectangle, Polygon, and Spline; Baffle offers
  Straight, Polyline, and Spline. Viewport prompts and previews distinguish clicked
  interpolation vertices from freeform spline controls.
- Added dependency-free exact polyline and polygon constructors to `funfern-core`.
  Every edge is represented by a collinear cubic Bézier span and every entered
  vertex is a multiplicity-three C0 breakpoint. Completed geometry remains an
  ordinary open or periodic spline throughout validation, persistence, and meshing.
- Straight baffles now use two clicked endpoints. Rectangle uses two opposite
  corners; Polyline and Polygon accept actual vertices and finish with Enter, while
  clicking the first vertex also closes Polygon. Each completed primitive is one
  document-history action; Backspace and Escape retain their staging behavior.
- Moved straightening into the shared contextual transform controls. It now handles
  complete open baffles and partial loop or baffle pieces bounded by C0 breaks,
  including seam-wrapped loop selections. It keeps endpoints fixed and changes only
  active control positions, so span conditions and boundary-probe attachments stay
  intact. Complete loops are deliberately ineligible.
- All 334 workspace tests, warning-denied Clippy, native release compilation, and
  the release Trunk build pass. Interactive construction remains on the manual
  browser and touch-device checklist.

## 2026-09-13 — Revised drawing-primitives slice

- Keep creation choices local to the **Draw** popup rather than adding a persistent
  Poly/Spline mode. The first choice remains the target role. Hole and Interface
  then offer **Circle**, **Rectangle**, **Polygon**, and **Spline**; Baffle offers
  **Straight**, **Polyline**, and **Spline**. Rename the current **Custom** action to
  **Spline** without changing its control-point semantics.
- Make construction semantics visible in the active-tool overlay and preview.
  Polygon and Polyline clicks place interpolation vertices joined by exact straight
  pieces. Spline clicks place control points and continues to show its control
  polygon. All completed primitives become the existing periodic or open spline
  representation, so files, validation, meshing, selection, BC assignment, and
  transforms do not gain a second geometry model.
- Replace the fixed one-click baffle preset with **Straight** as a two-endpoint
  workflow. Its required cubic controls are inserted equidistantly along the chord.
  **Polyline** accepts two or more vertices and finishes with Enter; **Polygon**
  accepts three or more vertices and finishes with Enter or by clicking its first
  vertex. **Rectangle** takes two opposite corners and creates four exact C0 sides.
  Backspace removes the latest staged vertex and Escape cancels every staged tool.
- Generalize **Straighten baffle** into **Straighten selected piece** for a contiguous
  span run bounded by C0 breaks or open endpoints. Preserve the piece endpoints,
  distribute its controls along the chord, and retain span boundary conditions,
  baffle face coherence, and geometry-attached probe references. A complete open
  curve remains eligible; a complete periodic loop does not collapse into a line.
- Each completed primitive and each straighten operation is one undoable document
  action. Failed or invalid placement remains governed by the existing draft and
  validator behavior. Respect the 32-feature and 128-control limits, and keep all
  constructors independent of camera zoom.
- Cover exact straightness, C0 corners, vertex ordering, closure, two-point baffles,
  cancellation, limits, role/region inheritance, BC and probe preservation, history,
  and scene round-trips. Exercise mouse and touch construction in the manual browser
  checklist.
- Ellipses, arcs, rounded rectangles, capsules, freehand drawing, arrays, mirrors,
  offsets, fillets, trim/extend, and construction guides are explicitly deferred.

## 2026-09-13 — Phone interactions and directional marquees

- Preserved the desktop interaction contract. On touch, a tap uses the existing hit
  priority and selection behavior; a one-finger drag beginning on an editable
  handle, curve, source, probe, domain edge, or gizmo directly manipulates it. An
  empty one-finger drag pans after the normal drag threshold, while an empty tap
  keeps the existing clear-selection/region-selection behavior.
- Added a transient touch-gesture owner so the synthetic primary-pointer events from
  the browser cannot compete with multi-touch navigation. A second finger cancels
  any uncommitted one-finger document drag back to its pre-drag snapshot, suppresses
  the associated synthetic click, and gives the gesture to viewport navigation
  until all fingers lift.
- Used egui's multi-touch centroid, translation delta, and zoom delta for two-finger
  pan and cursor-centered pinch zoom. Gesture rotation is ignored. Scale remains
  clamped through
  the existing camera bounds; navigation never enters document history.
- Added a persistent-until-dismissed **Area select** interaction mode near the span
  filter. It supplies Replace, Add, and Subtract choices for touch users. A
  one-finger drag in that mode draws the marquee; Done or Escape returns to ordinary
  selection. Mouse modifiers continue to provide the existing shortcuts.
- Derived marquee containment from horizontal drag direction. Left-to-right uses a
  conservative full-enclosure test over each complete adaptively sampled logical
  span; right-to-left uses the current rectangle-crossing test. Render the two modes
  distinctly, including a dashed crossing border, while retaining operation color
  for replace/add/subtract.
- Centralized viewport hit tolerances and expanded their invisible radius only for
  an active touch. The current handle/curve/source/probe/domain/gizmo priority is
  unchanged, so larger targets do not change which overlapping feature wins. Touch,
  marquee-tool, and gesture state remain transient.
- Set `touch-action: none` and contained overscroll on the WebGPU canvas so the browser
  does not steal viewport gestures. Egui panels retain normal scrolling and text
  editing because geometry input remains gated by the viewport response.
- Added synthetic egui touch coverage for direct manipulation with one history
  entry, empty-drag pan, pinch-center invariance, two-finger translation, transition
  from one to two fingers, click suppression after pinch, panel capture, repeated
  placement tools, enlarged hit targets, and touch Area-select operations. Desktop
  regressions cover directional enclosure/crossing, filters, modifiers, panning, and
  Escape restoration. All 325 workspace tests, warning-denied Clippy, native release
  compilation, and the release Trunk build pass. Real-device touch behavior remains
  a short manual browser check.

## 2026-09-13 — Live marquee modifiers and selection deletion

- Marquee operations now follow Shift and Alt throughout a drag instead of capturing
  modifiers only at pointer-down. Releasing a modifier returns to Replace for the
  ordinary mouse marquee or to the chosen Area-select operation.
- Delete and Backspace now act on the current editor selection regardless of whether
  the pointer is over the viewport. They remove probes, individual spline controls,
  and all completely selected loops and baffles. Multi-feature geometry deletion is
  one history action and also removes dependent probes through the editor model.
  Partial-span and outer-boundary selections remain selected because those spans are
  not independently deletable topology.
- All 328 workspace tests, warning-denied Clippy, native release compilation, and
  the release Trunk build pass.

## 2026-09-13 — Editable rectangular outer domain

- Moved the axis-aligned domain rectangle into each core `Scene`, so draft and
  accepted extents participate in validation, history, meshing, handoff decisions,
  save/load, recovery, examples, and shared links.
- Added direct edge/corner resizing in the viewport and numeric left/right/bottom/top
  controls in Edit. Domain drags are one history action, Escape restores the
  pre-drag document, invalid bounds remain visible as a draft, and Fit View follows
  the current rectangle.
- Generalized outer meshing, AMR boundary projection, boundary probes, grid drawing,
  material-region display, thumbnails, and SVG projection to non-square and shifted
  rectangles. Outer BC side identities remain stable.
- Generalized the automatic far-field inset contour to a rectangular equal-arclength
  sampler and constrained its inset by the shorter domain extent. Arbitrary curved
  outer domains remain deferred while auxiliary outgoing conditions on curves are
  still unstable.
- Scene JSON is version 19. Versions through 18 recover their historical top-level
  domain; version 19 stores draft and accepted bounds independently.
- Verification passes: 312 workspace tests, warnings-denied Clippy, optimized native
  compilation, and `NO_COLOR=true trunk build --release`.

## 2026-09-13 — Stable vector-overlay exposure

- Replaced frame-local-only arrow normalization with a peak-held exposure. The
  current spatial 90th percentile still responds immediately to stronger fields,
  while the strongest meaningful value seen during the current run and overlay mode
  remains the denominator. Weaker fields therefore remain proportionally shorter.
- Suppressed the overlay when its current 90th percentile falls below an absolute
  numerical floor or 0.01% of the held peak. Once outgoing energy leaves, arrows
  now shrink and disappear instead of expanding residual noise to full length.
  Fresh fields, direct solver resets, source changes, same-mesh physics-setting
  changes, and quantity changes reset exposure; ordinary geometry handoffs and
  toggling the same overlay retain it.
- Added a regression for peak retention, bounded late-field scaling, and silence
  thresholds. The 305-test workspace suite, warning-denied Clippy, and release
  Trunk build pass.

## 2026-09-13 — DC-stable EM reconstruction

- Replaced the raw primary-field accumulator with a two-stage, critically damped
  inverse derivative. Its zero DC response removes the persistent transverse-field
  bias and stationary source silhouettes caused by startup transients or moving a
  live point source, while retaining the expected quadrature response above its
  low cutoff.
- Used the two spare lanes in the existing state record and the spare transfer-entry
  lane, so both filter stages survive ordinary remesh and AMR handoffs without a new
  WebGPU binding or another buffer. Legitimate outgoing waves remain continuous;
  only stationary reconstruction memory decays.
- Added numerical regressions for DC rejection and the retained 3 Hz quadrature
  response. The full workspace suite, warning-denied Clippy, optimized native Metal
  pipeline startup, and `trunk build --release` pass.

## 2026-09-13 — Reconstructed EM fields and physical probe observables

- Added one trapezoidally integrated primary-field value per GPU DOF. Ordinary
  remesh and AMR transfer carry it with displacement and velocity, and fresh scenes
  initialize it to zero.
- Replaced the temporary EM time-derivative arrows with reconstructed transverse
  fields: `H = (-A_y, A_x)/mu` for TM and `E = (A_y, -A_x)/epsilon` for TE. The EM
  flow overlay now shows the Poynting vector `-k u grad(A)`.
- Updated the EM energy diagnostic to `(m u² + k |grad A|²)/2`. Point probes expose
  signed `E_z`/`H_z`, transverse magnitude, Poynting magnitude, and energy; curve
  probes expose the same scalar observables with signed normal Poynting flux; area
  probes add RMS transverse magnitude. Mechanical probe behavior remains unchanged,
  and no Cartesian component or abstract complementary-field trace is exposed.
- Kept the wave pipeline within WebGPU's eight-storage-binding budget by extending
  the existing aligned state record. Added CPU formula, transfer, shader-source,
  readback-layout, and UI regression coverage.
- Verification passes: `cargo test --workspace --no-fail-fast`, warning-denied
  Clippy for all workspace targets, the optimized native build and Metal pipeline
  startup, and `trunk build --release`. Interactive browser checks remain local and
  were not repeated for this slice.

## 2026-09-13 — Mechanical and TE/TM electromagnetic skins

- Added an undoable scene physics model with Mechanical, EM/TM (`E_z`), and EM/TE
  (`H_z`) modes. The scalar assembly maps EM permittivity, permeability, and loss
  rate onto the existing mass, stiffness, and damping operator, so meshing,
  stepping, transfer infrastructure, probes, and AMR continue to share one solver.
- Added semantic PEC and PMC conditions for outer edges, holes, and both baffle
  faces. Assembly and solution-error estimation resolve them consistently for the
  selected polarization. Mechanical boundary choices retain their previous labels
  and behavior.
- Physics changes reuse identical mesh geometry but deliberately start a zero field;
  transient probe histories, indicator work, and vector smoothing are cleared. This
  avoids transferring a scalar state between incompatible physical meanings.
- Material controls, overlays, point/curve/area readout vocabulary, and source labels
  follow the active skin. Material property frames remain rigid and orthonormal in
  world units. This initial implementation directly reinterpreted formulas; the
  later physics-switch conversion entry above replaces that behavior.
- Added derived arrow overlays using the synchronized P2 displacement/velocity
  readback: complementary-field rate for TM/TE and reduced relative energy flow.
  Density and gain are screen-space controls; smoothing is presentation-only and no
  new GPU shader or binding was added.
- Scene JSON is version 18. New files store explicit physics and tagged mechanical
  or electromagnetic material laws with physical property names. Version 17 and
  older files migrate exactly to Mechanical; vector display settings round-trip as
  presentation data.
- Converted Material lens to a TM dielectric example and the radial Luneburg lens
  to TE. Both enable complementary-field arrows by default while retaining their
  existing scalar wave-speed behavior.
- Verification passes: `cargo test --workspace`, Clippy for all workspace targets
  with warnings denied, the optimized native build, and `trunk build --release`.
  The native app initialized on Apple M1 Max / Metal. Interactive browser checks
  remain a local manual step and were not repeated for this slice.

## 2026-09-13 — Browser smoke test moved out of CI

- Removed Playwright, Chromium installation, and the WebGPU smoke test from the
  GitHub Actions job. GitHub-hosted Linux runners can return no WebGPU adapter, which
  made an otherwise valid build fail before Pages deployment.
- Kept the smoke suite and `npm run test:browser` as a local hardware-backed check.
  CI still builds the release WASM bundle before publishing from `main`.

## 2026-09-13 — Unified time signals and point-source ownership

- Added a dependency-free `TimeSignal` to the numerical core and use it for point
  sources, region sources, and prescribed Dirichlet/Neumann data. The first tagged
  variant is harmonic with bias, amplitude, frequency, and phase; pulse placement
  remains a distinct initial-condition action.
- Moved the point-source definition into the core and replaced its dedicated
  amplitude/frequency controls with the same signal editor used by volume and
  boundary drives. The AMR wavelength guard now obtains active bandwidth through
  the shared signal interface.
- Unified the GPU representation into one fixed-size signal record with reserved
  parameter space and no additional storage binding. Temporal-only point and volume
  edits replace forcing data while retaining their spatial weights and the live
  solver state.
- Scene JSON version 17 tags signal variants and nests the point-source signal.
  Version 16 point, volume, and boundary encodings migrate through the same decoder.
- Verification passes: formatting, warnings-denied Clippy, all **293 workspace
  tests**, native release compilation, release Trunk/WASM packaging, the native
  Metal mixed-source reference check at 9,690 DOFs, and the Chromium/WebGPU startup,
  frame-advance, and resize smoke test.

## 2026-09-12 — Unified document ownership and persisted presentation

- Replaced the parallel example simulation preset with a complete `Document`.
  Sources, probes, far-field configuration, and accepted/draft scenes now live in
  one `DocumentModel`; examples no longer need a second structure that can drift
  from file persistence.
- Undo/Redo stores `DocumentModel` directly instead of maintaining a duplicate
  history snapshot shape. Presentation settings wrap that model in `Document`, are
  persisted across scene files, shared links, examples, and recovery, and remain
  outside model history so an edit does not unexpectedly rewind the current view.
- Moved View-panel toggles, field intensity, and material-overlay configuration into
  `PresentationSettings`. Scene JSON version 16 serializes them, while older files
  load with the established defaults. Camera, selection, panel/window layout, GPU
  state, and derived caches remain transient.
- Added round-trip, migration, validation, and model-history tests for the new
  ownership boundary.
- Verification passes: formatting, warnings-denied Clippy, all **288 workspace
  tests**, native release compilation, release Trunk/WASM packaging, and the
  Chromium/WebGPU startup, frame-advance, and resize smoke test.

## 2026-09-12 — Browser probe-shader compatibility fix

- Chromium WebGPU rejected the line-, area-, and far-field probe modules because
  their WGSL constructed NaN sentinels as constant expressions. Native wgpu/Naga
  accepted the same source, so native testing did not expose the regression.
- The shaders now derive the same NaN payload from their runtime invocation index.
  Readback filtering and gap semantics are unchanged. A local release WASM build
  starts in Chromium with every probe pipeline accepted.
- Added a small Playwright Chromium/WebGPU smoke suite. It watches all console
  levels for Bevy rendering failures, requires a WebGPU adapter, confirms that the
  canvas advances, and checks that resize updates the backing render target. Run it
  locally against the root release bundle on a machine with a WebGPU adapter.

## 2026-09-12 — Region-owned volume sources

- Added one optional distributed source per subdomain. Each source has a signed
  constant/formula profile over the region's rigid world-unit frame, its own named
  parameters, enabled state, and harmonic bias/amplitude/frequency/phase signal.
- Added a resumable mass-lumped compiler for the enriched quadratic basis. Source
  profiles are integrated per triangle and normalized by assembled nodal mass, so
  the GPU applies a body acceleration consistently across material density.
- Source-only edits now compile cooperatively and replace only the forcing buffers,
  preserving mesh, operator, solution levels, solver clock, playback, and probe
  histories. Fresh/full/AMR candidates compile sources before their ordinary
  transaction. Active source frequencies participate in the AMR wavelength guard.
- Kept the wave pipeline at WebGPU's portable eight-storage-buffer limit by packing
  two source channel/weight pairs into the existing point/pulse weight record and a
  fixed signal table into the existing forcing buffer. Current topology allows no
  more than two sourced regions at a DOF; a future point-junction topology extension
  must revisit this representation.
- Materials now configures sources for the selected subdomain. View adds a signed,
  symmetric volume-source overlay, and Profile placement appears when either the
  passive material or source uses local coordinates. Renamed the older continuous
  Gaussian driver to **Point source** in the UI.
- Scene JSON version 15 persists sources in both draft and accepted scenes through
  files, shared links, recovery, examples, and history. Added a **Phased array**
  example with five compact phase-ramped sources, source overlay, and far-field
  monitor.
- All **285 workspace tests**, formatting, warnings-denied Clippy, native release
  compilation, and release Trunk/WASM packaging pass. The native production GPU
  check exercised a nonzero spatial source on Apple M1 Max / Metal at 9,690 DOFs,
  matched the f64 reference within 1.67e-4 relative L2, and confirmed that a live
  source edit retained the solver generation, 1,024-step clock, and field. Interactive
  browser review remains with the normal user testing pass.

## 2026-09-12 — GRIN rod and Luneburg examples

- Added two ready-to-run spatial-material examples. **GRIN rod** uses the local
  transverse coordinate to define an impedance-matched index profile, an off-axis
  continuous source inside the guide, and a high-resolution output line probe.
  **Luneburg lens** uses a radial profile in a region-attached frame, a driven left
  Dirichlet edge as a plane-like incident wave, a focal line probe, and a focal
  energy disk.
- Both examples open with the wave-speed material overlay and a fresh zero field.
  Example view settings now travel with the existing source preset without entering
  scene persistence. Gallery thumbnails sample the profile itself and mark driven
  boundary edges as well as point sources.
- At the default AMR quality, the active-frequency wavelength ceiling already
  produces local target ranges of **0.0312–0.0500** for the 4 Hz GRIN rod and
  **0.0404–0.0571** for the 3.5 Hz Luneburg lens from an initially dormant field.
  Their coarse test meshes contain 3,115 and 3,121 triangles with recommended
  explicit steps of 4.26e-3 and 6.28e-3 respectively. This is sufficient to wake
  spatial resolution before wave energy arrives, so no separate permanent
  coefficient-gradient floor was added.
- Automated checks cover structural/persistence round trips, active drivers,
  impedance matching, center/edge wave speeds, probes, thumbnail data, initial
  overlay state, practical time steps, and dormant-field AMR targets.
- All 278 workspace tests, warnings-denied Clippy, native release compilation, and
  release Trunk/WASM packaging pass. The production native AMR check also completed
  on Apple M1 Max / Metal in 4.80 s with finite spatial-material estimates and two
  continuous field handoffs. Interactive visual review remains with the normal
  product testing pass.

## 2026-09-12 — Coefficient-aware solution AMR

- Enabled automatic solution adaptation for spatial material profiles. The
  resumable estimator caches density, stiffness, and damping at its element
  quadrature points, reconstructs the world-space stiffness gradient from vertex
  values, and adds `grad(k) · grad(u)` to the strong variable-coefficient residual.
- Recovered fluxes, interior and material-interface jumps, energy normalization,
  impedance/radiation faces, and thin-gap faces now use coefficients at their
  corresponding element or edge locations. The wavelength ceiling uses the
  slowest sampled wave speed in each element.
- Material sampling has its own bounded phase and reports the failing region and
  point without publishing a partial target. Existing work limits, grading,
  refinement hysteresis, confirmed coarsening, and transaction handoffs are
  unchanged.
- The native production AMR check now runs with a radial density profile. On Apple
  M1 Max / Metal it completed in 4.89 s, including a live spatial-material estimate,
  two handoffs, 371 second-pass insertions, 282 collapses, and a continuous probe
  trace; the longest indicator slice remained at 2.00 ms.
- All 277 workspace tests, Clippy with warnings denied, native release compilation,
  and release Trunk/WASM packaging pass.

## 2026-09-12 — Region-aware material-overlay ranges

- Fixed automatic property scaling when one subdomain occupies only a small part
  of the mesh. Robust outlier trimming now happens independently within every
  subdomain, and the resulting ranges are combined, so sample-count weighting can
  no longer discard an entire small region and clamp it to an endpoint color.
- Linear and logarithmic views use the same region-aware policy; manual ranges are
  unchanged.
- All 271 workspace tests, Clippy with warnings denied, native release compilation,
  and release Trunk/WASM packaging pass.

## 2026-09-12 — Persistent adaptation-target display

- Fixed the adaptation-target overlay disappearing at every automatic AMR commit.
  The last completed adaptive size field now survives its mesh handoff and is
  sampled on the replacement mesh, rather than requiring the old result's
  per-element array to have the new triangle count.
- Fresh scenes and geometry, material, boundary, reset, or AMR-setting changes
  continue to invalidate the display. A regression test evaluates an old indicator
  field on a replacement mesh with a different revision and element count.
- All 270 workspace tests, Clippy with warnings denied, native release compilation,
  and release Trunk/WASM packaging pass.

## 2026-09-12 — Material profile placement and visualization

- Moved region-owned material-frame controls under Profile placement for the
  selected subdomain. They appear only for a varying assigned material; the
  material Library remains concerned with reusable coefficient definitions.
- Added a viewport frame gizmo with an origin handle, oriented axes, and rotation
  ring. Origin and angle drags support grid/15-degree Shift snapping, Escape
  cancellation, and one history entry per gesture. It intentionally has no scale:
  local coordinates remain in world units.
- Replaced the material-fill toggle with one overlay selector for material regions,
  density, stiffness, damping, wave speed, or impedance. Property overlays provide
  opacity, robust automatic or manual ranges, linear/log mapping, a viewport legend,
  and local/world-coordinate hover readout.
- Property rendering uses the quadratic operator's seven display nodes and six
  subtriangles. Vertices are keyed by operator node and region, preventing values
  from blending across material interfaces. Sampling and topology preparation are
  cooperative and revision keyed; property switches reuse cached samples, stale
  jobs are discarded, and the previous complete overlay remains visible during an
  update. Runtime formula failures color only affected triangles red and do not
  change solver acceptance.
- All 269 workspace tests pass, including interface duplication, range and derived
  property behavior, cache reuse/stale replacement, and frame-gizmo history.
  Formatting, Clippy with warnings denied, native release compilation, and release
  Trunk/WASM packaging pass. Interactive visual testing remains for the normal
  product review pass.

## 2026-09-12 — Spatial scalar material profiles

- Added a dependency-free bounded expression compiler for density, stiffness, and
  damping. Expressions support local Cartesian/polar coordinates, shared named
  parameters, arithmetic and a small math-function set; source, operation, nesting,
  stack, and parameter counts are capped.
- Added rigid orthonormal material frames with numeric origin, angle, and world or
  follow-region attachment. Local `x`, `y`, and `r` remain world units. Whole-loop
  similarity edits carry attached frames, while scaling geometry does not scale the
  coordinate system and non-rigid control edits leave it fixed.
- The enriched quadratic operator now evaluates coefficients at mass nodes,
  stiffness quadrature points, and boundary quadrature points. Point and area probe
  energy use the same spatial values, and the maximum sampled wave speed enters the
  explicit timestep bound. A formula that becomes non-finite, nonpositive, or
  negative where disallowed rejects the candidate operator without replacing the
  running solver.
- Scene JSON is version 14 and persists formula source, material parameters, and
  region frames through files, links, examples, recovery, and history. Versions
  1–13 remain loadable; legacy coefficients become constants and interior frames
  are centered on their sampled owning loops.
- Automatic solution AMR pauses with `waiting for coefficient-aware AMR` whenever
  an assigned material is spatially varying. Far-field compilation continues to
  require a uniform lossless background.
- Formula drafts remain in the Materials inspector when parsing fails, with the
  last valid material retained. Numeric coefficient, parameter, and frame edits are
  normal undoable material transactions.
- All 264 workspace tests pass. Formatting, Clippy with warnings denied, native
  release compilation, and release Trunk/WASM packaging pass for this slice.

## 2026-09-12 — Automatic inset far-field monitor

- Corrected contour clearance to test adaptively subdivided closed and open spline
  traces rather than their global control hulls. The sampler's fixed world-space
  error margin keeps genuine contact ambiguous while allowing enclosed curves whose
  off-curve control points cross the contour.
- Implemented the scene-level far-field switch as a derived square Huygens contour:
  one inset controls 256 counterclockwise midpoint samples and no additional probe
  geometry or selection mode is exposed. The contour must enclose modeled geometry
  and remain entirely in the lossless background material.
- Added two GPU passes at 60 samples per simulated second. The first stores field,
  velocity, and outward normal derivative in a 512-frame contour ring; the second
  interpolates retarded samples and integrates 96 observation directions. Only the
  compact directional ring is read back, and an undersized delay window is reported
  instead of silently truncating slow-background results.
- Added a floating readout with a direction/time waterfall, synchronized
  radiated-power trace, and equal-width 40 dB instantaneous and
  visible-window-averaged polar patterns. A persistent three-item picker controls
  those views. Double-clicking the contour or `FF` badge opens the readout. The View
  inspector can hide the derived contour, while compatible mesh and AMR handoffs
  preserve host history.
- All 252 workspace tests pass. The native Metal check compiled and exercised all
  recorder shaders on an Apple M1 Max with 9,690 DOFs: 1,024 steps and readback took
  about 0.15 s after setup, producing finite 96-direction far-field records. Native
  release compilation, Clippy, and release Trunk/WASM are checked for this slice.

## 2026-09-12 — Interactive GPU area probes

- Added disk and subdomain creation to the Probes inspector. Disks use a two-click
  center/radius workflow, then support direct body motion and a visible radius
  handle. Subdomain targets are chosen by clicking inside a region and render their
  owning outline; all area receivers share a separate View toggle.
- Added area readouts for mean and RMS displacement, mean energy density, total
  energy, and coverage. A compact persistent Plots menu starts with mean field and
  total energy, and host traces remain continuous across ordinary solver and AMR
  generations.
- Added a two-stage GPU recorder. Clipped quadratic elements upload preintegrated
  field, mass, and stiffness matrices; one dispatch evaluates element contributions
  and another reduces each probe into a compact time-stamped ring. The path is
  capped at 16 probes, 200,000 element contributions, 2048 frames, and 120 samples
  per simulated second.
- Extended the native GPU check scene with a disk receiver so runtime verification
  covers WGSL compilation, reduction, physical values, and readback. Far-field
  recording from the derived inset contour remains the next part of the milestone.
- All 244 workspace tests pass. Formatting and Clippy with warnings denied pass,
  as do native and release Trunk/WASM builds. The native Metal check exercised a
  9,690-DOF mesh and all three recorder pipelines; 128 steps plus readback completed
  in about 1.92 s with valid area coverage and finite nonnegative energy.

## 2026-09-12 — Area-probe and far-field document foundation

- Added free-disk and stable-region area targets to the document model. Region
  attachments are removed atomically when their owning region disappears; disk and
  region creation, updates, deletion, and attachment lifecycle use normal snapshot
  history.
- Added a dependency-free enriched-quadratic area integration reference. It clips
  partial disks in world space, applies degree-six quadrature, and reports mean and
  RMS displacement, mean and total energy, covered area, and geometric coverage.
- Far field is represented as one scene-level enable switch and inset value with a
  `0.12` default, ready for the derived outer contour in the recorder/UI slice.
- Scene JSON is version 13. Area targets and far-field settings round-trip through
  every existing persistence path; versions 1–12 load with the disabled default.
- GPU area reduction, viewport creation/editing, readouts, and the automatic
  far-field contour remain the next parts of this milestone.
- All 238 workspace tests, formatting, Clippy with warnings denied, native release
  compilation, and the release Trunk/WASM build pass.

## 2026-09-12 — Geometry-attached boundary probes

- Added probes from one contiguous selected run on the outer boundary, a closed
  loop, or a baffle. Closed selections may cross the periodic seam or cover the
  complete curve. Material interfaces, walls, and baffles expose one explicit
  sampled trace.
- Boundary samples compile from labeled mesh-edge parameter intervals into
  side-aware quadratic stencils. Each point carries its own outward normal, so
  signed flux keeps its physical meaning when the arclength direction is flipped.
  Whole closed probes use periodic sampling and integration without a duplicate
  seam point.
- Attachments follow knot insertion/removal and baffle split/merge operations by
  deterministic best-overlap remapping. Deleted geometry removes its probes in
  the same history action. Runtime traces keep their IDs and time history through
  accepted geometry and AMR handoffs.
- View now controls point, line, and boundary probe visibility independently.
  Hidden probe markers and overlays also stop participating in viewport hit tests;
  recording and open readouts continue normally.
- Scene JSON is version 12. Boundary feature, span run, trace side, direction, and
  sampling preset now round-trip through files, links, examples, autosave, and
  Undo/Redo; versions 1–11 remain loadable.
- All 233 workspace tests, formatting, Clippy with warnings denied, native release
  compilation, and the release Trunk/WASM build pass.

## 2026-09-12 — Direct and persistent continuous source

- Removed the separate Move source placement mode. An enabled continuous source is
  now dragged directly by its viewport marker, with grid/Shift snapping and Escape
  restoring the pre-drag position. One complete drag is one Undo/Redo action.
- Moved continuous-source enablement, position, frequency, strength, width, and
  containing region into the document. Scene files are version 11; version 10 and
  older files load with the previous disabled default. Files, scene links, examples,
  Undo/Redo, and browser/native autosave now use the same source state.
- Added UI input coverage for direct drag and cancellation, plus persistence tests
  for round-trip, legacy defaults, invalid values, and missing region references.
- All 222 workspace tests, formatting, Clippy with warnings denied, and the release
  Trunk/WASM build pass.

## 2026-09-12 — Line-probe plot matrix

- Replaced the four exposed line-readout toggles with a compact 3×3 Plots menu.
  Field, signed normal flux, and energy can each be displayed versus arclength, as
  a waterfall, or as an arclength integral versus time. The default remains four
  plots: field profile and waterfall, normal power, and integrated energy. The
  picker stays open while toggling plots or adjusting gain, and closes on an
  outside click or Escape.
- All nine views use the same time-window state. Waterfall dragging now follows its
  vertical time axis; horizontal movement does not pan it. Time traces retain their
  horizontal grabbed-content interaction, and arclength profiles follow the shared
  selected time.
- Replaced the previous mean-energy trace with the requested energy line integral.
  Field and flux use the same gap-aware trapezoidal integration, so every integral
  excludes intervals adjacent to invalid spatial samples and retains coverage.
- Regression coverage checks the default four-of-nine selection, repeated toggles
  within one open picker, all three line integrals across a gap, and
  horizontal-versus-vertical waterfall dragging.
- All 220 workspace tests, formatting, Clippy with warnings denied, and the release
  Trunk/WASM build pass.

## 2026-09-12 — Straight line probes

- Added independent two-click line probes with endpoint editing, rigid body dragging,
  a visible positive-normal arrow, direction reversal, names/colors, and the same
  history, file, link, example, and autosave behavior as point probes.
- Added Low, Medium, and High sampling presets: 32 points at 30 Hz, 64 at 60 Hz,
  and 128 at 120 Hz. Documents remain limited to 16 total probes and 512 line
  sample points. Scene JSON is now version 10; version 9 point-probe files remain
  loadable.
- Added a separate bounded GPU line recorder with enriched-quadratic displacement,
  material-aware energy density, and signed normal energy flux. Its 64-frame ring
  is independent of the longer point-probe ring. Partial domain or ambiguous-trace
  coverage produces spatial gaps while valid intervals continue recording.
- The initial line readout provided field profile and waterfall, mean energy, and
  signed normal-power history. The plot-matrix follow-up above generalizes this to
  all quantity/representation combinations.
- Automated coverage includes two-click placement, rigid dragging as one undoable
  action, partial-coverage aggregation, format migration/round-trip, shader clock,
  flux/gap output, and independent preset strides. The native GPU check now also
  requires a finite 64-point line-probe readback.
- All 217 workspace tests and Clippy with warnings denied pass. The release native
  Metal check executed both recorder pipelines on an Apple M1 Max with 9,690 DOFs,
  `dt=0.0030078`, finite complete 64-point frames, expected NaN gaps on a partially
  outside line, and `1.30e-6` field relative L2 error. Native release compilation
  and the release Trunk/WASM build pass.

## 2026-09-12 — Probe foundation and point receivers

- Added Probes as a fifth hideable inspector with persistent point placement,
  direct marker dragging, naming/color, recording controls, and
  one closeable floating readout per probe.
- Point readouts show solver-time field, centered velocity, and material-aware
  local energy density. Their sections are independently hideable and share a
  draggable, wheel-zoomable time window with an explicit Live follow mode.
  Definitions participate in Undo/Redo, scene files, links, examples, and autosave;
  recorded samples remain transient.
- Added a dependency-free enriched-quadratic point stencil in core. Points outside
  the domain or directly on two-trace boundaries remain visible but inactive with
  a concrete status.
- Added a separate portable GPU probe pipeline and bounded ring. Sampling cadence
  follows solver steps rather than rendered frames, stale generations are rejected,
  and ordinary remesh/AMR handoffs retain host history while recompiling stencils.
- Fixed a handoff timestamp error where the probe recorder added the host handoff
  offset to an already continuous transferred GPU clock. Depending on elapsed time,
  this appeared as a gap after geometry remeshing or aged out the complete trace
  after AMR. Both paths now append to the same continuous solver-time history.
- Extended the native transfer and AMR checks to run with a live point recorder.
  The two-handoff AMR check retained 147 samples with a largest rebind interval of
  0.0273 simulation seconds and no clock discontinuity or history loss.
- Plot dragging now accumulates egui's per-frame pointer deltas instead of treating
  each delta as the displacement from the gesture origin. The historical window no
  longer springs back when the pointer pauses, and it stays fixed as new samples
  arrive. Dragging follows grabbed-content semantics: left moves toward the future
  and right moves into history. Zooming out to the full retained window returns to
  Live automatically.
  Regression checks cover multi-frame time-window dragging and moving the floating
  window.
- Probe markers open their floating readout on double-click. The starter example now
  includes a receiver, and clean startup loads that catalog entry and its simulation
  settings instead of maintaining a separate hardcoded initial scene.
- The top toolbar now uses progressive single-row compaction. File operations fold
  into File first; at narrower widths the five inspector switches fold into Panels.
  Undo/Redo, Fit view, Panels, Draw, and playback remain directly reachable without
  increasing the header height.
- All 210 workspace tests and Clippy with warnings denied pass. The release Metal
  GPU check on an Apple M1 Max exercised 9,690 DOFs at `dt=0.0030078`, produced
  finite nonnegative probe energy, matched the f64 field within `1.30e-6` relative
  L2 error, and advanced at 18.9 simulated seconds per wall second. Native release
  compilation and the release Trunk/WASM build pass.
- Region, selected-geometry, and far-field targets remain later probe slices;
  the panel and per-probe readout dispatch are structured to accept them.

## 2026-09-12 — Examples, recovery, export, and shareable scenes

- Added an Examples gallery with live vector thumbnails, names, and short
  descriptions. Starter obstacle, double-slit baffles, a material lens, and the
  eight-obstacle scattering scene are bundled. Every accepted example passes the
  normal bounded validator, and opening one is a single undoable document action.
- Made the bundled examples ready to run by pairing each scene with a tuned
  continuous source and an intentional outer-boundary configuration. Opening an
  example applies its source and reveals the boundary-law overlay. Gallery
  thumbnails show both the source location and colored outer-edge laws; rows use
  full-width separators instead of content-sized card frames.
- Example loading now requests a fresh zero-field solver candidate instead of
  building a transfer map from the previously open scene. This prevents unrelated
  fields and velocities from contaminating the next example while leaving normal
  edit and AMR handoffs unchanged.
- Repaired near-coincident open constraints in the mesher. Once a baffle segment
  is recovered, a dangerously close free bulk vertex moves away only when its
  complete incident triangle fan remains oriented; constraint geometry does not
  move. The double-slit screen is exactly vertical again, and its production
  `h=0.08` mesh has an explicit timestep above `1e-3` instead of the former
  roundoff-sliver value near `1e-17`.
- Added two independent safety nets: final mesh verification rejects
  scale-degenerate triangles, and quadratic operator assembly rejects a CFL bound
  that is absurd relative to the mesh's shortest edge and fastest material wave
  speed. Regressions cover the exact vertical double slit and a deliberately
  degenerate input mesh.
- Added debounced crash recovery for the complete draft/accepted document pair.
  Browser builds use local storage; native builds write and atomically rename a
  per-user recovery file. Startup restores recovery automatically unless a shared
  scene fragment is present. Invalid editable drafts are retained.
- Added compressed `#scene=v1.…` sharing. The fragment contains URL-safe,
  zlib-compressed versioned scene JSON, is bounded before and after decompression,
  and goes through the ordinary structural and geometry validation pipeline. Copy
  scene link updates the browser address and clipboard; native builds copy a link
  to the deployed Pages app. Once active, autosave keeps the fragment current with
  browser `replaceState` rather than adding navigation history entries.
- Added accepted-scene SVG export independent of camera, selection, field state,
  and editor overlays. This is also the reusable scene-thumbnail rendering basis;
  raster screenshots and video remain later work.
- Verification: formatting, workspace Clippy with warnings denied, all 193
  workspace tests, native release compilation, and warning-free release Trunk/WASM
  packaging pass. Coverage includes compressed-link corruption and round trips,
  native atomic recovery, complete example validation/SVG generation, gallery UI,
  and undoable example replacement. Interactive browser testing remains deferred
  by prior agreement.

## 2026-09-12 — Physical boundary residuals for solution AMR

- Follow-up: AMR handoff now gates step scheduling without changing the user's
  Run/Pause preference. This removes playback-button flicker, preserves a choice
  made while a candidate is pending, and avoids accumulating catch-up time during
  the transfer. The 79 app/editor tests, app Clippy, native release build, release
  Trunk/WASM package, and Metal `--amr-check` pass after the fix.
- Extended the resumable solution indicator across every physical boundary edge.
  Reflecting, prescribed Neumann, first-order impedance, and second-order radiation
  faces now contribute their active-law residual to the adjacent element. Closed
  walls use their solver-defined reflecting law, while transmitting material
  interfaces remain part of the interior flux-jump estimate.
- Included paired thin-gap spring terms on both baffle traces. Pair lookup uses the
  stable boundary ID and parameter interval, rejects missing, duplicate, crossed,
  or geometrically inconsistent faces, and preserves distinct left/right laws.
- Added the second-order auxiliary field to indicator snapshots. The app aligns the
  post-step GPU auxiliary readback to the existing centered displacement snapshot
  on the host, avoiding another GPU buffer or transfer. Malformed lengths and
  non-finite auxiliary values are rejected before estimation.
- Kept prescribed Dirichlet mismatch diagnostic-only because the solver strongly
  eliminates those nodal values. Performance diagnostics now show the relative
  recovery, cell, interior-jump, and boundary contributions, the number of physical
  edges evaluated, and the largest Dirichlet mismatch.
- Added manufactured boundary-law regressions, deterministic sliced execution,
  hole impedance coverage, two-sided thin-gap pairing/error coverage, Dirichlet
  policy coverage, malformed auxiliary snapshots, and host time-alignment coverage.
  The end-to-end `--amr-check` now requires finite physical-boundary results.
- Verification: formatting, Clippy with warnings denied, all 186 workspace tests,
  native release compilation, release Trunk/WASM packaging, and the Apple M1 Max /
  Metal `--amr-check` pass. The live estimate evaluated 101 boundary edges with a
  `4.984e-4` boundary contribution; total indicator work was 2.50 ms with a 1.30 ms
  longest slice. The complete check finished in 5.90 s and its final deterministic
  transaction produced 2,036 triangles / 6,224 DOFs with 371 insertions and 279
  collapses. Interactive browser testing remains deferred by prior agreement.

## 2026-09-12 — Bidirectional solution AMR tuning

- Fixed the one-way size policy. The estimator previously allowed targets to grow
  by only `1.4×`, while collapse required an edge below `0.35×` target; ordinary
  near-uniform elements therefore could not become collapse candidates. Automatic
  presets now use compatible `1.9–2.5×` quiet growth and a `0.65×` collapse ratio.
- Refinement and coarsening decisions are evaluated separately. Refinement remains
  immediate; coarsening requires two consecutive requests on the same committed
  mesh. Collapse work is capped at half the preset topology budget so it cannot
  consume the capacity reserved for urgent refinement.
- Kept graded targets local to each source triangle instead of spreading the lowest
  value through every triangle sharing a vertex. Shared-edge lookup remains
  conservative, and the active-source wavelength ceiling is unchanged.
- Rejected collapses now continue through the already sorted candidate set instead
  of restarting a full vertex scan for every rejection. Performance diagnostics
  report requested candidates, accepted/rejected collapses, and separate refinement
  and coarsening change counts.
- Added regressions proving that repeated quiet estimates shrink a fine mesh without
  inserting vertices, a zero coarsening quota still permits refinement, confirmation
  requires two estimates, and every preset can cross its collapse threshold.
- Verification: formatting, Clippy with warnings denied, all 181 workspace tests,
  native release compilation, release Trunk/WASM packaging, and the Apple M1 Max /
  Metal `--amr-check` pass. The live-field check completed in 5.00 s with 371
  insertions and 279 collapsed vertices in its final deterministic transaction,
  2.40 ms total indicator work, and a 1.24 ms longest indicator slice. Interactive
  browser testing remains deferred by prior agreement.

## 2026-09-12 — Automatic solution-driven AMR

- Added a dependency-free, resumable quadratic solution indicator. It combines
  material-aware recovered displacement/velocity flux defects, the strong interior
  wave-equation residual with continuous-source acceleration removed, and flux jumps
  across interior edges. Recovery keys include region identity, so material
  interfaces and duplicated wall/baffle traces do not smear into one another.
- Reused three spare lanes in the existing GPU state buffer and readback for
  time-aligned displacement, velocity, and acceleration. Normal stepping, pulse
  injection, prescribed Dirichlet nodes, and transferred states maintain these
  values without another GPU buffer or host transfer.
- Enabled automatic adaptation by default with Fast, Balanced, and Detailed presets,
  advanced minimum/maximum element sizes, a wavelength ceiling derived from every
  active continuous or driven-boundary source, neighbor grading, scheduling delay,
  candidate/severity hysteresis, and stale mesh/GPU/source/settings rejection.
  Geometry edits cancel pending solution work and retain priority.
- Added an optional adaptation-target overlay in View. Simulation reports the live
  AMR phase; Performance reports indicator range, target range, candidates, work,
  slice timing, and the last topology changes.
- Added core tests for deterministic slicing, affine recovery, amplitude
  normalization, volume-source subtraction, wavelength limiting, field lookup, and
  malformed/stale snapshots. UI coverage checks the default controls and scans all
  source kinds for the wavelength guard. `--amr-check` now requires the ordinary
  automatic controller to complete an aligned readback/estimate before its two
  deterministic transaction passes.
- Physical boundary residual terms are deliberately deferred as the next AMR slice;
  the present indicator uses interior cell and flux information at those edges.
- Follow-up fixes clamp the interpolated spatial field back to its configured bounds
  after tolerant barycentric lookup, preventing a nominal `0.02` target from becoming
  `0.019999…`. During the one frame where a candidate-generation readback precedes
  its application commit, the viewport now renders it with the candidate operator;
  the wave colors no longer disappear because of a temporary DOF-count mismatch.
- Automatic adaptation no longer inherits the fixed five-million-unit ceiling used
  by explicit transactions. Its work allowance now scales with the committed mesh
  and the selected topology-change preset. If that bounded allowance is still
  exhausted, the automatic controller records the report and retains the current
  valid mesh without raising a global mesh fault.
- Verification: formatting, Clippy with warnings denied, all 178 workspace tests,
  native release compilation, and release Trunk/WASM packaging pass. The Apple M1
  Max / Metal `--amr-check` processed a nonzero field through automatic adaptation
  without a target-bound failure, then completed both deterministic transactions in
  5.26 s. Indicator work totaled 3.05 ms with a 1.69 ms longest slice; the final
  mesh had 2,093 triangles / 6,398 DOFs, with 356 insertions and 275 collapses in
  the second deterministic pass. Interactive browser testing remains deferred by
  prior agreement.

## 2026-09-12 — Spatial size-field adaptive mesh transaction

- Added dependency-free `MeshSizeField` and resumable `MeshAdaptationJob` APIs.
  One transaction coarsens and refines an immutable committed mesh against a smooth
  target length, restores local legality, verifies topology, and publishes a valid
  result under explicit work, topology-change, and capacity limits.
- Added a separate mesh revision alongside the geometry revision. Wave operators,
  GPU preparation, and linear/quadratic transfer maps now reject a different
  discretization even when it represents the same accepted scene.
- Preserved persistent vertex lineage and modification generations across passes.
  The refine/collapse hysteresis is 1.05/0.35, and generation cooldown prevents
  newly changed vertices from immediately reversing topology.
- Kept box corners, spline seams and knot breakpoints, and baffle tips fixed.
  Ordinary hole/material-interface constraints coarsen when their merged exact
  cubic span stays within curve tolerance. Open baffles and closed walls refine and
  coarsen both coincident traces atomically. Testing exposed a closed-wall seam case
  where paired parameters differ by one period; pairing now follows coincident,
  opposite geometric edges while retaining each face's own parameter interval.
- Integrated AMR with the existing application transaction pipeline in soft 2 ms
  slices. A completed mesh receives a new operator and exact quadratic transfer;
  the old GPU solution continues until mesh, operator, state, timestep, and lineage
  commit together. There is no normal UI or persistence surface in this slice.
- Added deterministic slice-size, moving-target refine/coarsen, malformed input,
  immutable-source, lineage, hole/interface/wall/baffle topology, and exact
  quadratic-transfer tests. The opt-in native `--amr-check` evolves a nonzero field
  and performs two spatial-target handoffs with zero exposed nodes.
- Apple M1 Max / Metal release check: first pass inserted 359 vertices and produced
  1,516 triangles; moving the target inserted 372 and collapsed 207, producing
  1,844 triangles and 5,638 quadratic DOFs. The second pass used 766,693 work units,
  converged without a limit, preserved 624 source triangles, and the complete
  startup/two-handoff check took 1.17 s with solver dt 0.003947.
- Formatting, Clippy with warnings denied, workspace tests, native release build,
  and release WASM packaging pass. Interactive browser verification was skipped by
  prior agreement; the feature has no normal browser UI yet.

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
  frame. Profiles use local world-unit coordinates `x`, `y`, `r`, and `theta` in a
  rigid orthonormal frame; frames can be world-fixed, object-attached, or
  independently transformed.
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
