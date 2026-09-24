# funfern user guide

What the editor does and how it behaves. The README covers building and
running; this file covers using it.


- **UI shell:** the top bar keeps Undo/Redo, scene files, Fit View, five
  hideable inspector panels, Draw, and wave playback controls visible;
  the solver starts running once its initial mesh is ready. Edit contains
  selection, transforms, topology, and boundary tools; View contains visual
  overlays and field intensity; Simulation contains mesh and solver settings;
  Materials contains the material library; Probes contains receivers and recording
  controls. The lower-right status control shows
  FPS, solver steps per second, DOFs, mesh size, and solver dt; clicking it opens
  the full frame, topology, mesh, handoff, and solver diagnostics. Under the frame
  graph, above the sections that change height, is a Log of what those sections
  said before they were overwritten - status lines, preparation and adaptation
  errors, and the reason a repair became a rebuild - which can be cleared or
  copied out. A preparation or adaptation error lights a
  marker beside the status control and keeps it lit until the diagnostics are
  opened, rather than opening them. As the window narrows, file
  actions collapse into File first, followed by the inspector switches collapsing
  into Panels. The essential editing and playback controls stay on one row.
- **Select:** clicking a control or junction selects one handle. Clicking a curve
  selects its stable span; Shift-click toggles spans, and Ctrl/Cmd-click selects the
  complete curve. Marquee direction follows CAD convention: left-to-right fully
  encloses spans, while right-to-left crosses them. Shift at release adds the hits.
  The outer rectangle participates in the same span selection model.
- **Delete and transform:** Delete or Backspace removes the selection: every
  completely selected curve, and one curve selected in part cut down to that run. Drag one handle to reshape it, or drag a transformable span selection as a
  rigid piece. The Edit panel also applies numeric translation, rotation, and
  uniform scale around the selection centroid; the viewport ring and square grip
  provide direct rotation and scaling. A junction can move only with all
  incident arms, and a selection whose boundary knot is smooth shares control
  points with its neighbour, so neither offers the transform gizmo until the
  selection is widened or Isolate at C0 walls it off. The outer rectangle resizes
  by dragging a side or one of its corner grips, with Shift snapping to the grid;
  the Edit panel also carries its extents as numbers.
- **Continuity:** a selected end knot reads C0, C1, or C2 and can be set to any
  of them, except at a topology junction, which stays a corner. Smoothing a knot
  on a small loop refines the curve first so there are controls to spend; the
  refinement is exact, so nothing moves and one undo steps back over the whole
  change. Actions that cannot succeed are shown disabled with the reason.
- **Weld:** dragging a loose end of an open curve shows every target it can join
  in gold. Dropped on another loose end, the two curves become one — the curve
  that stayed put keeps its identity and direction, and the seam is an ordinary
  corner that Continuity can smooth. Dropped on its own other end, the curve
  closes into a loop. Dropped on a junction, a curve interior, or a corner that
  owns no junction yet, the end attaches there as a new arm, including on its own
  curve, where only the span the end itself sits on is out of reach. A drop the
  arrangement rejects is refused and only the drag remains. A curve too small to
  become a loop is refined first by knot insertion, which does not change its
  shape.
- **Topology:** drawing is geometry-first. Closed curves offer Circle, Rectangle,
  Polygon, and Spline tools with an initial Subdomain/Hole purpose. Open curves
  offer Polyline and Spline tools with an initial Separator/Baffle purpose.
  Either kind may have free ends: a separator that meets nothing divides
  nothing, which is how a wall is switched off without being deleted, or a
  divider drawn now and attached later. Both ends of an open curve must touch the
  same subdomain, and which subdomain a curve belongs to is read from where the
  curve is drawn, not from which side of a boundary a click landed on - clicking
  a boundary names the boundary.
  Attached endpoints share an authoritative junction, follow outer-domain
  resizing, and can be detached from the selected junction.
  While drawing an open curve, eligible outer edges, inner curves, existing
  junctions, loose ends, and vertex-less corners are highlighted; the hovered
  attachment uses a fixed screen-space snap radius, independent of zoom. Starting
  or finishing on a loose end welds the new curve into that curve.
  Removing a divider that merges several subdomains highlights the candidates in
  the scene and takes a click as the survivor, and so does a weld that merges
  them. A weld removes no edge, but it moves the end it welds, and a divider
  separates two faces only while its circuit closes: welding a divider's loose
  end onto a baffle that reaches no wall opens that circuit and folds the two
  subdomains into one face. The weld waits for the same click rather than being
  refused.
- **Boundaries:** one Boundary inspector applies conditions to every compatible
  selected span and reports mixed assignments. Transmit and Boundary are tick
  boxes reporting which state the selection is in: a separated span reads as a
  boundary whatever conditions its two faces carry, a selection holding both
  states ticks neither and is marked Mixed, and ticking a box puts every selected
  span in that state. A span excluded on both sides, one
  between two holes say, is marked Inactive, since nothing it carries reaches the
  simulation, and a transmitting span excluded on just one side is marked Walled,
  since it has nothing to transmit into and reflects until the far side carries a
  material again. Outer edges support reflecting,
  prescribed
  time-varying Neumann and Dirichlet data, and first- or second-order outgoing
  conditions. Hole and baffle faces support the same choices, with an adjustable
  impedance ratio for first-order outgoing behavior. The baffle face selector stays
  in the inspector because its left and right traces are geometrically coincident;
  the viewport bands the selected side of every selected span in white and marks
  the start-to-end direction those two names are measured from with a gold arrow.
  A baffle span instead can use one conservative thin-gap law coupling both traces;
  applying a face condition converts it back to independent faces, and increasing
  gap stiffness can reduce the solver time step.
  New scenes start with the second-order outgoing condition on all four outer
  edges. View > Boundary conditions colors the assigned laws directly on the
  outer box, hole spans, and both baffle traces, each law on its own side of the
  span.
- **Physics skins:** Simulation switches the shared scalar PDE between mechanical,
  electromagnetic TM (`E_z`), and electromagnetic TE (`H_z`) views. Mechanical
  materials expose density, stiffness, and damping. EM materials expose relative
  permittivity `ε`, relative permeability `μ`, and a loss rate `α`; both
  polarizations use `c = 1/sqrt(ε μ)` and `Z = sqrt(μ/ε)`. Perfect electric and
  magnetic walls resolve to the appropriate zero-value or zero-flux scalar law for
  the chosen polarization. Changing between Mechanical and EM converts every
  material with `ε = 1/K`, `μ = ρ`, and `α = d/ρ` (or the inverse mapping),
  preserving local wave speed, impedance, and normalized damping rate. Formula
  conversion simplifies exact reciprocal/product cancellations so repeated switches
  stay bounded. The change is undoable, reuses unchanged mesh geometry, and starts a
  fresh field so incompatible state is never transferred. TM/TE switches retain the
  same EM material law.
- **Geometry role:** Draw contains closed and open curve tools. Its palette is a
  floating window that can be dragged anywhere and stays up across draws, so one
  primitive follows another; it closes from its own button or the toolbar toggle.
  Holding Shift places each point on the same grid dragging snaps to, except
  where the point attaches to existing geometry, which outranks it. That grid is
  the one drawn in the viewport: it steps in 1/2/5 per decade from the zoom and
  divides each step into four or five, and Shift lands on those divisions, so
  what is drawn is what can be reached. Zooming in makes it finer.
  Closed curves start as subdomains or holes. Open curves start as transmitting separators or
  two-sided baffles, either of which may be left unattached. The material chosen
  for a separator applies only if it encloses a face as it is drawn; a face
  enclosed later, by attaching an end, inherits the material of the region it is
  cut out of. Gold highlights and a 14-pixel screen-space query show the exact
  available attachments on the outer domain, inner curves, and junctions, either
  side of each being a target.
- **Circle:** click to place eight controls on a radius `0.15` circle, then
  automatically return to selection. The spline lies inside its control polygon.
- **Rectangle:** two opposite-corner clicks create an axis-aligned rectangular
  loop from exact straight cubic spans and then return to selection.
- **Polygon and Polyline:** clicks place interpolation vertices joined by exact C0
  straight spans. Enter finishes either tool; clicking the first vertex also closes
  a polygon. Polygon is available for loops and Polyline for baffles.
- **Spline:** this is the control-point tool. A baffle can be finished with exactly
  two clicked controls to create an exact straight line with equidistant internal
  controls; three controls are incomplete, and four or more form the usual freeform
  spline. Four controls enable the live curve preview. Enter finishes an open
  baffle; Enter or clicking the first handle closes a loop. Its control polygon
  distinguishes it from the vertex-based tools. Backspace removes the latest staged
  point and Escape cancels construction.
- **Spline editing:** **Straighten spans** makes every selected logical span an
  exact line between its own endpoints, inserting exact C0 isolation where needed,
  so a multi-span selection becomes a polyline through the existing breakpoints.
  **Straighten selection** instead lays one contiguous run on a single chord
  between its outer breakpoints; it is available only while each touched curve
  contributes one contiguous run, and it refuses a run containing a junction.
  Both preserve stable span identities and their assigned laws and probes.
  **Isolate at C0** preserves the curve exactly while freeing a partial span
  selection for rigid motion. A single selected span exposes C2, C1, and C0 choices
  for its end knot; smoothing uses a bounded projection and reports its maximum
  displacement. Junction knots stay C0. Drag selected geometry to translate it.
  Movable selections show a draggable rotation/scale center, a rotation ring, and
  separate uniform, horizontal, and vertical scale grips. Hold Shift while dragging
  to snap world coordinates to `0.05`, angles to 15°, and scale to 0.1 increments.
  Numeric translation, rotation, and scale use the same topology-aware transform;
  each geometry gesture or command is one history action.
- **Remove:** Delete or the panel action removes the selected control and its
  associated knot. This can reshape the curve. At least four controls must remain.
  A removal that would merge different span conditions is rejected until the two
  assignments agree. Shape-preserving insertion copies the split span assignment.
  A complete selected loop or baffle can be deleted with Delete or Backspace.
  Delete also accepts one contiguous run of spans: the run goes and the rest of
  the curve stays as open baffles with the default wall, because an open curve
  cannot keep a transmitting end at a free tip. A curve whose junction sat inside
  the deleted run is promoted the same way and named in the status line, and two
  loose ends the deletion leaves alone at a junction fuse into one curve.
  One Delete is one deletion however much it names: any number of curves whole
  and at most one of them cut down to a run, worked out together and applied as
  one history entry, so one undo brings the whole gesture back. Two curves each
  selected in part are two questions about where the pieces land, and are refused
  rather than half answered. When the deletion merges several subdomains into
  one, the candidates light up in the scene and a click picks the survivor; the
  question covers the whole deletion, so subdomains merging across several of the
  deleted curves are offered together. A deletion that would merge subdomains in
  two separate places is refused before anything is removed.
- **Materials:** create and name materials in the Library, edit the three base
  properties named by the active physics skin, and assign them under
  Subdomain assignment, which lists either Faces or Regions. Faces shows one row
  per compiled subdomain, holes included, and its dropdown offers Hole alongside
  the materials, so a subdomain becomes a hole and back from the same place.
  Emptying a face walls its own boundary and nothing else: a span transmits
  exactly when the faces on both of its sides carry a material. The toggle also
  steers the viewport, where a click selects a face or a region to match. Each coefficient can be a constant or a formula. Formulas
  use local `x`, `y`, `r`, and `theta`, constants `pi` and `e`, named material
  parameters, arithmetic, powers, and `sqrt`, `abs`, `sin`, `cos`, `tan`, `exp`,
  `log`, `min`, `max`, `clamp`, and `smoothstep`. For example, a radial profile can
  define parameter `R = 0.35` and stiffness `2 - clamp(0, 1, r / R)^2`. The `?`
  beside Library, and the one beside an enabled volume source, open a compact
  syntax reference that stays up while a formula is typed.
  Coordinates are measured in world units in a rigid orthonormal frame: origin and
  angle set placement, with no hidden coordinate scaling. A new subdomain's frame starts at the
  centre of the face it owns, so a local profile is usable before touching the
  gizmo. A region frame can stay
  fixed in the world or follow whole-loop translation, rotation, and uniform scale;
  scaling moves the frame with the object but does not rescale the profile. Unused
  materials are isotropic by default. Directional materials add a constant or
  formula **Axis ratio** of at least one. The frame's local x axis is the fast
  principal axis; the local stiffness/flux tensor is `k diag(a, 1/a)`, preserving
  the base coefficient's geometric mean while giving a principal wave-speed ratio
  `a`. The ratio survives Mechanical/EM and TM/TE switches unchanged. Unused
  non-default materials can be deleted. A material's **Response** selector
  applies a law preset - a switchable medium, a pump, a time crystal, a
  travelling modulation, a Kerr or a saturable medium - and shows what the
  laws compose to. A switchable medium gets a **Switch ▸** button (hotkey
  **S**, for the material open in the editor or the scene's only Switch) that
  ramps it to its alternate law and back over its Switch ramp, on the running
  field; it is not an edit and leaves undo alone. A Kerr or saturable medium
  reads out how far the field has moved it, as the coefficient's peak change
  from its small-signal value, and the Solver section names what lowers the
  step ceiling. Selecting a region row highlights its
  complete derived boundary in the viewport and carries the same categorical
  swatch the Subdomains overlay uses. Material changes preserve the live field and reuse the committed
  mesh. Profile placement appears for the selected subdomain and can be adjusted
  numerically or with its viewport origin/rotation gizmo. View can overlay Materials (each face in
  its assigned material's colour), Subdomains (a categorical colour per stable
  region, so neighbouring faces sharing a material stay distinct), the adaptation
  target, density,
  stiffness, damping, wave speed, impedance, anisotropy, or volume-source
  amplitude with linear/log and
  automatic/manual range controls plus a local-coordinate hover readout. The
  anisotropy overlay uses a logarithmic ratio scale and sparse fast-axis marks.
  Automatic
  solution AMR evaluates varying coefficients and their gradients directly.
- **Vector view:** View can overlay the canonical complementary field or energy flow
  sampled directly on the GPU at display cadence. Arrow spacing and gain are
  screen-space presentation controls. Complementary-field arrows optionally subtract
  a slow, explicitly presentation-only baseline; energy-flow arrows do not, because
  their time average is meaningful. The canonical state, probes and energy always
  retain the full field. The arrow filter follows stable physical mesh samples while
  the view moves; a newly exposed sample starts silent until it has temporal history,
  so an unknown DC baseline is not flashed as a wave. A resident grid-filter event
  rebases that presentation filter instead of appearing as temporal field content.
  Once the overlay falls below one percent of its run peak it fades rather than
  magnifying f32 residue. Each EM mode remains one scalar
  Maxwell polarization rather than a simultaneous six-component field solve.
- **Field exposure:** the scalar field and the vector overlay each set their own
  scale from what is on screen, so a scene whose amplitude is a hundredth of
  another's reads the same. Auto exposure can be turned off, which puts the
  intensity slider back in charge of the whole scale. The scale is a high quantile of the current frame; it
  rises the instant the field does, so a placed pulse is never clipped, and falls
  back slowly — slower than a field drains away, so a domain that has emptied goes
  dark instead of being renormalized back to full brightness. A placed pulse
  therefore holds the scale for some seconds before the view returns to normal. It
  will not fall below a hundredth of the loudest level seen. Below that floor the
  drawing fades smoothly, so a field that has decayed into rounding noise is not
  magnified back into view; sparse arrow outliers cannot override that shared fade.
  Field
  intensity and arrow gain trim that automatic scale rather than replacing it, and
  View prints the level the colours are relative to so a decaying field can be
  told from a steady one. Each isolated subdomain is also drawn relative to its
  own rigid offset, because a uniform displacement carries no energy and no
  radiating wall can damp one, so left in it would become the scale and wash the
  domain flat. Performance diagnostics reports what was removed per subdomain and
  how fast it is moving, and the status marker asks for attention if one grows
  past what single precision can carry alongside the wave.
- **Region sources:** the selected subdomain can own one distributed source,
  independent of its reusable passive material. Its signed spatial profile uses the
  same region-local world-unit coordinates and its own named parameters, multiplied
  by a bias-plus-sinusoid signal with amplitude, frequency, and phase. Source-only
  edits preserve the mesh, operator, live field, solver clock, and probe traces;
  temporal-only edits also reuse the compiled spatial weights. Region frames appear
  when either the passive material or its source uses local coordinates.
- **Simulation input:** Place pulse remains active for repeated viewport clicks and
  can be toggled off with the same selected button, the viewport Done action, or
  Escape. Drag the visible point-source marker directly to reposition it.
  Pulse strength/width and point-source position, width, region, bias, amplitude,
  frequency, and phase are editable in Simulation. Point-source settings are included
  in scene files, shared links, examples, Undo/Redo, and autosave.
- **Point probes:** add persistent point receivers from Probes, then click repeatedly
  in the viewport, holding Shift to place each marker on the grid. Markers can be selected, dragged, renamed, colored, disabled,
  cleared, or deleted; double-clicking one opens its floating readout. Each readout
  contains primary field, transverse-field magnitude, Poynting magnitude, and local
  energy-density traces for EM scenes. Mechanical scenes expose displacement,
  velocity, and energy density. Hide individual traces, drag right to inspect
  earlier samples, and scroll to
  change the time span. The widest view returns to Live automatically; Live can also
  be selected directly. Sampling follows
  solver time rather than browser frame rate. Definitions are saved and undoable;
  recorded traces are transient. Every trace runs on the solver's own clock,
  which a remesh carries across, so adaptation leaves the recording continuous
  rather than stepping its time axis forward. A new mesh over the same recorders
  also takes over the GPU rings they were filling, so the samples written between
  the last readback and the handoff arrive rather than going with the buffers.
  Reset restarts that clock, and the traces and rings with it.
- **Line probes:** place an independent straight sampling segment with two clicks,
  which Shift snaps to the grid.
  Drag either endpoint to reshape it or drag its body as one rigid object. The arrow
  shows the positive left normal; **Swap ends** reverses the sampling order and
  the flux sign together. Low, Medium, and High presets record 32/64/128 spatial samples at
  30/60/120 samples per simulated second. A compact Plots menu exposes the signed
  primary field, transverse-field magnitude, signed normal Poynting flux, its
  average over a trailing window, and energy as a waterfall, versus arclength, or
  integrated versus time. Mechanical scenes retain field, normal energy flux, its
  average, and energy. New readouts initially show field versus arclength, its
  waterfall, the Average flux profile, normal power, and integrated energy. All
  active views share one time window; drag waterfalls vertically and time traces
  horizontally.
  Portions outside the simulated domain or on a two-trace boundary appear as gaps;
  the remaining coverage still contributes to the readout. Average flux reports,
  at every recorded instant, the mean of the seconds ending there, so a standing
  wave averages to nothing while a travelling one keeps the power it carries. Its
  window is a slider in the same menu, independent of the visible one so navigating
  never rewrites the numbers, and bounded by what the 512-frame trace holds at that
  preset. The readout says how much of the window is recorded until it is full.
- **Boundary probes:** select one contiguous run of outer, loop, or baffle spans and
  choose **+ From selected spans** in Probes. The probe follows spline edits and
  uses the same twelve curve readouts, sixteen in EM scenes. Choose the sampled
  trace for material interfaces, walls, and baffles. Positive normal flux always
  points out of that trace; **Reverse direction** turns the arclength axis alone,
  leaving the side and the sign of the flux where they are. Because
  a span's two traces lie on top of each other, the scene says which one is read with
  a stem standing on that side and running into the probe's marker, so the reading
  arrives from the side the stem sits on and travels the way positive flux points. A
  second arrow leaves the middle of that stem the way the arclength axis runs. Whole
  closed curves use periodic sampling and integration without duplicating the seam.
- **Area probes:** choose Disk for a two-click center/radius receiver - Shift puts
  the center on the grid and the radius on a multiple of it - or Subdomain
  and click inside a material region. Drag a disk body to move it and its edge
  handle to resize it. In EM scenes the GPU recorder reports mean and RMS primary
  field, RMS transverse-field magnitude, mean energy density, total energy, and
  target coverage. Mechanical scenes retain their displacement and energy
  quantities; new readouts initially show RMS primary field and total energy. A
  subdomain marker sits at the area-weighted centroid of the region it reads, taken
  from the geometry, so remeshing and adaptation leave it where it is; for a region
  shaped like a ring that point lands in the hole. Area
  markers and region outlines have an independent
  View toggle. Definitions are saved and undoable while recorded histories remain
  transient across files and continuous across ordinary solver handoffs.
- **Far field:** enable the outer-domain far field in Probes and adjust one inset.
  Funfern derives a square Huygens contour from the domain, samples it on the GPU,
  and projects the delayed field into 96 observation directions. Its floating
  readout combines a direction/time waterfall, matching 40 dB instantaneous and
  visible-window-averaged polar patterns, and radiated power over time. Double-click
  the contour or its `FF` badge to open it. The contour must enclose every modeled boundary
  and stay in the lossless background material; View can hide its overlay.
  Every direction reads the contour at its own retarded time, so nothing is
  plotted until the recorder holds a whole delay window; Probes and the readout
  report how much of it is recorded. The recording is of the exterior at fixed
  world points, so a remesh - an adaptation included - keeps it and reads on
  through the new mesh. Moving the contour, changing the exterior, and restarting
  the solver start it again.
- **Navigate:** right-drag or Space + left-drag pans with a mouse. On touch screens,
  drag an object to manipulate it, drag empty canvas with one finger to marquee
  spans, and use two fingers to pan and pinch around their shared center. Touch hit
  targets expand without making the drawn controls larger. Marquee feedback shows
  the live Replace/Add/Subtract operation and Enclosed/Crossing hit mode. Shift adds,
  Alt subtracts, and either modifier can change while dragging; left-to-right fully
  encloses spans while right-to-left selects crossings. Fit View frames the
  current domain. Panel scrolling and text editing do not manipulate the viewport.
- **Drafts:** green curves are accepted, amber curves are being checked, red
  curves are invalid. The last accepted scene stays as a subdued reference.
  Invalid edits remain after release. Escape during a drag restores its starting
  document; Undo can restore earlier accepted snapshots.
- **History:** Ctrl/Cmd+Z undoes, Ctrl/Cmd+Shift+Z redoes outside text editing.
  Each drag, coordinate edit, insertion, removal, creation, or deletion is one
  action. History keeps up to 100 actions, including invalid drafts.
- **Files:** New starts an empty scene: one background material, every wall
  second-order outgoing, and the point source on, so it runs as soon as it
  opens. Save scene downloads JSON in the browser or opens a native save
  dialog. Load scene uses file upload/native selection and validates before
  replacement. Successful loading clears history; malformed files leave the
  current document intact. Examples opens a thumbnail gallery with a
  description beside each scene; opening one is undoable and leaves the gallery
  up, so the catalog can be clicked through. A thumbnail draws the scene's own
  faces in their material colours, its walls in their boundary-condition
  colours, and, for an example whose view preset names a material property, that
  property across the face it varies over. Export can write a geometry SVG, a PNG snapshot, or a silent 60 FPS
  recording of the current viewport at its physical pixel resolution. Captures
  follow the active View settings while omitting panels, floating readouts,
  selection emphasis, gizmos, marquees, and active-tool prompts. Recording leaves
  playback and viewport navigation live; its status-strip control shows elapsed
  time and stops/finalizes the file. The file is paced by wall clock rather than
  by the app's frame rate, so it stays the right length on a machine that cannot
  feed every frame; a slot no frame arrived for holds the one before it, and the
  status strip counts frames the encoder could not keep up with. Documents autosave after edits and restore on
  startup from browser local storage or the native per-user recovery file. Copy
  scene link embeds compressed, validated scene data in a `#scene=v1.…` URL
  fragment; while that fragment is active, later autosaves keep it current.
  The catalog includes ready-to-run GRIN rod and circular Luneburg profiles with
  their drivers, probes, and wave-speed view presets. View toggles, field intensity,
  and material-overlay settings travel through files, links, recovery, and examples.
  Camera, selection, open panels, and floating-window layout are not saved. With no
  shared scene or autosave to restore, startup opens a bundled example at
  random.
- **Mesh:** enable Accepted triangle mesh under Display. The overlay shows the
  constrained mesh and its labeled outer, hole, interface, closed-wall, and baffle
  edges. Elements below 15° are
  amber; the panel reports counts, minimum angle, maximum edge, refinement
  progress, build/work time, longest mesh slice, and explicit construction
  failures. Meshing targets a soft 2 ms per frame. An invalid draft keeps the last
  accepted mesh.
- **Resolution:** the Simulation panel offers Coarse (0.16), Medium (0.08, the
  default) and Fine (0.04) presets and a Target edge slider. Changing resolution
  rebuilds the accepted mesh when the control is released, without touching
  geometry or history, and carries the running field across. Remesh rebuilds at
  the current resolution, which also leaves an adapted mesh. Resolution is not
  stored in scene files.
  A full Fine build can take a second when spread across frames, so geometry
  edits repair the active mesh instead: moving a control or a junction, adding
  or deleting a curve, and changing a span's behaviour carve out the band the
  changed boundary touched and refill it, however far the boundary moved, and
  everything else stays as it was, refinement included. The refill matches the
  density around the band, coarsening where adaptation asks for it but never
  refining finer than the mesh it joins, so dragging one curve back and forth
  over the same ground leaves the mesh where it was instead of grinding it
  finer with every pass. Assigning another
  material to a subdomain relabels its elements without carving. The running
  field crosses over with every node outside the band copied exactly. The
  Performance panel's handoff line reports the repair with its kept, removed and
  inserted element counts, or the reason a repair fell back to a full rebuild.
  Resizing the domain and changing the resolution still rebuild.
  Open-curve insertion reuses safe nearby bulk vertices at the exact curve position
  to avoid tiny CFL-limiting elements around baffles.
- **Adaptive mesh foundation:** the core can refine and coarsen an existing mesh
  against a bounded spatial edge-size field while preserving the accepted geometry
  revision. A separate mesh revision identifies each discretization. Outer edges,
  holes, material interfaces, closed walls, and both coincident baffle faces remain
  constrained; spline breakpoints, box corners, and open tips remain fixed. The app
  runs adaptation in 2 ms slices and transfers the live quadratic field at an atomic
  GPU handoff. Automatic adaptation is enabled by default. A recovery-plus-residual
  indicator reads synchronized displacement, velocity, and acceleration from spare
  lanes in the existing GPU state readback, then builds a graded spatial size field
  in bounded frame slices. Target accuracy is how much estimated error the whole
  field may carry, with Coarse, Medium and Fine presets at 24%, 12% and 6% over a
  slider that reaches everything between; the panel shows the estimate beside the
  target. Refinement the error estimate asks for stops once the field is inside
  the target, which is what keeps a mesh off its floor: the estimator's step down
  is clamped, so anything it cannot satisfy - a boundary the field disagrees with,
  the grid-scale leftovers of a wave that has passed - otherwise shrinks by that
  same step every cycle until it reaches the smallest element allowed, whatever
  the target says. Once the whole field has fallen below `10⁻⁴` of the run's
  peak canonical energy, relative error is reported as dormant and stops driving
  refinement; this prevents numerical tail divided by vanishing field energy from
  looking increasingly inaccurate. Error concentrated in a small part of a field
  that is comfortably inside the target is the trade one number for the whole
  field makes.
  Carrying a forced wavelength and staying under the largest element allowed are
  floors rather than judgements about error, so they refine regardless; Advanced
  settings holds elements per wavelength, at six - twelve nodes, quadratically -
  and the smallest and largest element, and the panel says when a forced
  wavelength wants elements under the smallest allowed, which holds the mesh at
  its floor on its own. Refinement reacts immediately, while coarsening requires
  two quiet estimates and has its own transaction quota. View's Overlay list carries the current target
  field alongside the material overlays, blue where the estimate wants the finest
  elements and orange where it wants the coarsest, and says which of adaptation
  being off, no estimate yet, or an estimate behind the mesh is leaving it empty.
- **Waves:** Run/Pause, Step, and Reset operate the GPU solver. Reset restarts the
  clock at zero, so it clears the probe traces and records the new run from its
  own start rather than appending to the previous one. Place pulse adds a
  Gaussian displacement with zero initial velocity. The optional point source is
  repositioned by dragging its viewport marker. Simulation contains a speed
  ceiling on simulated seconds per wall second, saved with the document; the
  solver is paced to it rather than to real time, and says what it is actually
  reaching when a scene costs more per step than a frame can afford. Substeps per
  display frame are bounded, so asking for more than a scene can deliver falls
  short rather than building a backlog. Slow speeds shorten the solver's own time
  step rather than skipping frames, so the motion stays smooth instead of
  advancing in visible jumps; the step never exceeds the stability limit the mesh
  sets. Field colors use an adjustable symmetric gain.
  Pulses and point sources act only in their containing wall-separated
  region. With open baffles their Gaussian stencil uses mesh-path distance, so it
  goes around a free endpoint instead of jumping through coincident faces.
  The panel reports DOFs, GPU buffer size, operator-derived timestep, simulated
  time, substeps, throughput, operator/map preparation time, and discrete energy.
  Advanced settings, folded away at the bottom, holds the adaptation limits -
  elements per wavelength and the smallest and largest element - and Damp
  unresolvable detail, on by default: the scheme dissipates at no wavelength and the fastest modes a
  mesh can hold barely travel, so a sharp event leaves a speckle that stays put
  for the rest of the run, and this removes it for well under a percent per half
  minute of a wave resolved as finely as the mesh indicator aims for. It preserves
  constants and stationary force-free complementary flux, so it is not a general
  DC-removal or terminal-silence control. It runs on every medium - fixed,
  time-driven, Kerr or saturable - beside any boundary; on a field-dependent
  medium it acts through the medium's local response, and a pass that would add
  energy is skipped. The default
  point source starts on a cosine, which carries no net impulse; any other phase
  hands the field a mean velocity, and a region sealed by reflecting walls has
  nowhere to put one, so its level then rises for as long as the run lasts.
- **Outer boundary:** select each box side independently and assign zero Neumann
  (reflecting), prescribed Neumann flux, prescribed Dirichlet displacement,
  first-order outgoing, or second-order outgoing behavior. Prescribed data uses
  `offset + amplitude · sin(2π f t + phase)`; zero amplitude gives a constant.
  Adjacent Dirichlet sides must agree at their shared corner. The second-order
  Engquist-Majda condition adds tangential propagation and reduces oblique
  reflection. Hole and baffle spans support reflecting or local first-order
  impedance conditions; closed walls remain reflecting and material interfaces
  transmit. Changes reuse the mesh and transactionally transfer the live field.
- **Solution adaptation:** a resumable quadratic indicator combines recovered-flux
  defects, interior equation and flux-jump residuals, and the active physical
  boundary laws. It includes paired thin-gap traces and second-order radiation
  memory while treating prescribed Dirichlet mismatch as a diagnostic. It reports
  the relative error of the whole field in the energy norm alongside the per-element
  indicators, and counts the refinements it asks for apart from the ones a limit
  asks for. The result drives bounded refinement and coarsening with wavelength and
  grading limits.
- **Interior media:** every triangle carries a stable region ID. The P2e operator
  assembles piecewise mass, stiffness, and damping. Material interfaces use a
  conforming shared trace. Open material dividers may terminate on the outer
  boundary or meet at explicit C0 junctions; T/crossing edits preserve per-span
  left/right region sectors. Closed walls and open baffles have separate solution
  DOFs on their two faces; baffle tips reconnect to the surrounding domain.
- **Mesh changes:** simulation continues on the displayed committed mesh while a
  replacement mesh, operator, timestep, and transfer map are prepared. The GPU
  reconstructs velocity from both old displacement levels, interpolates field and
  velocity, initializes the new staggered level for its new timestep, and commits
  after a finite tagged readback. Newly exposed domain starts at zero. A failed
  candidate retains the previous simulation. Baffle trace nodes prefer the same
  stable boundary ID and left/right face on the old mesh during transfer.

Scenes allow 64 unified curves, 32 materials, one volume source per region, and
128 controls per curve. Version 22 JSON stores one open/closed curve model, stable
span and topology-vertex identities, oriented region anchors, editable draft and
accepted scenes, boundary laws, constant/formula materials, directional axis ratios,
orthonormal region frames, sources, probes, far-field settings, and presentation
settings. Version 22 is a deliberate hard cut and older schemas are rejected; the
built-in examples are authored directly in the new model. JSON files are capped at
2 MiB and require finite coordinates. The editor
uses a world-space validation clearance of `1e-4` times the larger domain extent,
independent of zoom. The editor
validator is deliberately conservative; final mesh topology uses adaptive exact
orientation and incircle signs rather than geometric epsilons.
Load [examples/eight-obstacles.json](../examples/eight-obstacles.json) for a
representative scene, or open Obstacle array from the example gallery, which is
the same scene.
