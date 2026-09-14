# funfern

[![CI and Pages](https://github.com/DmitriyNE/funfern/actions/workflows/ci.yml/badge.svg)](https://github.com/DmitriyNE/funfern/actions/workflows/ci.yml)

A browser finite-element wave playground with mechanical and TE/TM electromagnetic
scalar skins, editable periodic and open cubic splines, material regions, holes and
reflecting baffles, constrained triangle meshes, persistent invalid drafts,
undo/redo, and versioned scene files.

The production wave solver uses seven-node enriched quadratic, mass-lumped
triangles on both an f64 CPU reference and an f32 WebGPU gather kernel. Geometry
edits commit transactionally: the previous mesh continues to run during candidate
construction, then displacement and velocity transfer with the quadratic basis
before both new time levels and mesh become visible. Native automated checks pass;
browser verification passes on Chromium 152 with an Apple Metal WebGPU adapter.
That pass exposed and fixed both a portable WebGPU binding-limit violation and a
shader entry-point collision in Bevy's browser shader translation path.

## Run

Tested toolchain: Rust 1.96.0, Trunk 0.21.14. Install the browser tools once:

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
```

From the repository root:

```sh
trunk serve
```

Open <http://127.0.0.1:8080/> in a WebGPU-capable browser with hardware
acceleration enabled. WebGPU requires HTTPS or localhost. The page displays a
startup diagnostic if initialization fails. Trunk watches sources and reloads
on rebuild; its first build downloads matching WASM helpers.

Native development:

```sh
cargo run -p funfern-app --locked
```

Native viewport recording also requires `ffmpeg` on `PATH`. Funfern selects an
available H.264, VP9, or VP8 encoder and writes MP4 or WebM accordingly. Browser
recording uses the browser's built-in `MediaRecorder` and needs no additional tool.

Release browser bundle (output: `dist/`):

```sh
trunk build --release
# Serve the optimized build locally:
trunk serve --release
```

If your shell sets `NO_COLOR=1`, Trunk 0.21.14 rejects that value. Use
`NO_COLOR=true trunk serve` or `NO_COLOR=true trunk build --release` instead.
The release build and serving commands were exercised with this environment
setting. Keep `Cargo.lock` for reproducibility. The pinned integration is
[Bevy 0.19.1](https://docs.rs/bevy/0.19.1/bevy/) with
[bevy_egui 0.42.0](https://docs.rs/crate/bevy_egui/0.42.0).
See the [maintained Trunk project](https://github.com/trunk-rs/trunk) for tooling.

Docker builds the release WASM bundle and serves it with nginx:

```sh
docker compose up --build
```

Open <http://localhost:8080/>. Set `FUNFERN_PORT` to use another host port,
for example `FUNFERN_PORT=9000 docker compose up --build`. Stop it with
`docker compose down`.

## Edit

- **UI shell:** the top bar keeps Undo/Redo, scene files, Fit View, five
  hideable inspector panels, Draw, and wave playback controls visible;
  the solver starts running once its initial mesh is ready. Edit contains
  selection, transforms, topology, and boundary tools; View contains visual
  overlays and field intensity; Simulation contains mesh and solver settings;
  Materials contains the material library; Probes contains receivers and recording
  controls. The lower-right status control shows
  FPS, solver steps per second, DOFs, mesh size, and solver dt; clicking it opens
  the full frame, topology, mesh, handoff, and solver diagnostics. As the window narrows, file
  actions collapse into File first, followed by the inspector switches collapsing
  into Panels. The essential editing and playback controls stay on one row.
- **Select:** clicking a control or junction selects one handle. Clicking a curve
  selects its stable span; Shift-click toggles spans, and Ctrl/Cmd-click selects the
  complete curve. Marquee direction follows CAD convention: left-to-right fully
  encloses spans, while right-to-left crosses them. Shift at release adds the hits.
  The outer rectangle participates in the same span selection model.
- **Delete and transform:** Delete or Backspace removes every completely selected
  curve. Drag one handle to reshape it, or drag a transformable span selection as a
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
  offer Polyline and Spline tools with an initial Separator/Baffle purpose. A
  separator must start and finish on valid boundaries of the same active face;
  baffles may have free ends. Attached endpoints share an authoritative junction,
  follow outer-domain resizing, and can be detached from the selected junction.
  While drawing an open curve, eligible outer edges, inner curves, existing
  junctions, loose ends, and vertex-less corners are highlighted; the hovered
  attachment uses a fixed screen-space snap radius, independent of zoom. Starting
  or finishing on a loose end welds the new curve into that curve.
  Removing a divider that merges several subdomains highlights the candidates in
  the scene and takes a click as the survivor.
- **Boundaries:** one Boundary inspector applies conditions to every compatible
  selected span and reports mixed assignments. A span excluded on both sides, one
  between two holes say, is marked Inactive, since nothing it carries reaches the
  simulation. Outer edges support reflecting,
  prescribed
  time-varying Neumann and Dirichlet data, and first- or second-order outgoing
  conditions. Hole and baffle faces support the same choices, with an adjustable
  impedance ratio for first-order outgoing behavior. The baffle face selector stays
  in the inspector because its left and right traces are geometrically coincident;
  viewport arrows show the start-to-end direction defining those sides.
  A baffle span instead can use one conservative thin-gap law coupling both traces;
  applying a face condition converts it back to independent faces, and increasing
  gap stiffness can reduce the solver time step.
  New scenes start with the second-order auxiliary condition on all four outer
  edges. View > Boundary conditions colors the assigned laws directly on the
  outer box, hole spans, and both baffle traces.
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
- **Geometry role:** Draw contains closed and open curve tools. Closed curves start
  as subdomains or holes. Open curves start as transmitting separators or
  two-sided baffles; separator endpoints must snap to boundaries of the same active
  face. Gold highlights and a 14-pixel screen-space query show the exact available
  attachments on the outer domain, inner curves, and junctions.
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
  loose ends the deletion leaves alone at a junction fuse into one curve. When the
  deletion merges several subdomains into one, the candidates light up in the
  scene and a click picks the survivor; a deletion that would merge subdomains in
  two separate places is refused.
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
  control beside the material-property note opens a compact syntax reference.
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
  non-default materials can be deleted. Selecting a region row highlights its
  complete derived boundary in the viewport and carries the same categorical
  swatch the Subdomains overlay uses. Material changes preserve the live field and reuse the committed
  mesh. Profile placement appears for the selected subdomain and can be adjusted
  numerically or with its viewport origin/rotation gizmo. View can overlay Materials (each face in
  its assigned material's colour), Subdomains (a categorical colour per stable
  region, so neighbouring faces sharing a material stay distinct), density,
  stiffness, damping, wave speed, impedance, anisotropy, or volume-source
  amplitude with linear/log and
  automatic/manual range controls plus a local-coordinate hover readout. The
  anisotropy overlay uses a logarithmic ratio scale and sparse fast-axis marks.
  Automatic
  solution AMR evaluates varying coefficients and their gradients directly.
- **Vector view:** View can overlay arrows derived from the synchronized quadratic
  field readback. A DC-rejecting inverse time derivative reconstructs the transverse
  field without retaining stationary startup or source-edit imprints. TM scenes show
  the in-plane magnetic field `H`, TE scenes show the in-plane electric field `E`,
  and either polarization can show the corresponding Poynting vector. Mechanical
  scenes offer energy flow only; the scalar displacement field has no
  complementary transverse vector, so that mode is not listed there. Arrow spacing
  and gain are screen-space presentation controls, with optional temporal
  smoothing. Each EM mode remains one scalar Maxwell polarization rather than a
  simultaneous six-component field solve.
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
  in the viewport. Markers can be selected, dragged, renamed, colored, disabled,
  cleared, or deleted; double-clicking one opens its floating readout. Each readout
  contains primary field, transverse-field magnitude, Poynting magnitude, and local
  energy-density traces for EM scenes. Mechanical scenes expose displacement,
  velocity, and energy density. Hide individual traces, drag right to inspect
  earlier samples, and scroll to
  change the time span. The widest view returns to Live automatically; Live can also
  be selected directly. Sampling follows
  solver time rather than browser frame rate. Definitions are saved and undoable;
  recorded traces are transient.
- **Line probes:** place an independent straight sampling segment with two clicks.
  Drag either endpoint to reshape it or drag its body as one rigid object. The arrow
  shows the positive left normal; Flip direction reverses both the sampling order
  and flux sign. Low, Medium, and High presets record 32/64/128 spatial samples at
  30/60/120 samples per simulated second. A compact Plots menu exposes the signed
  primary field, transverse-field magnitude, signed normal Poynting flux, and
  energy as a waterfall, versus arclength, or integrated versus time. Mechanical
  scenes retain field, normal energy flux, and energy. New readouts initially show field versus
  arclength, its waterfall, normal power, and integrated energy. All active views
  share one time window; drag waterfalls vertically and time traces horizontally.
  Portions outside the simulated domain or on a two-trace boundary appear as gaps;
  the remaining coverage still contributes to the readout.
- **Boundary probes:** select one contiguous run of outer, loop, or baffle spans and
  choose **+ From selected spans** in Probes. The probe follows spline edits and
  uses the same nine curve readouts. Choose the sampled trace for material
  interfaces, walls, and baffles. Positive normal flux always points out of that
  trace; Flip direction reverses only the arclength axis. Whole closed curves use
  periodic sampling and integration without duplicating the seam.
- **Area probes:** choose Disk for a two-click center/radius receiver, or Subdomain
  and click inside a material region. Drag a disk body to move it and its edge
  handle to resize it. In EM scenes the GPU recorder reports mean and RMS primary
  field, RMS transverse-field magnitude, mean energy density, total energy, and
  target coverage. Mechanical scenes retain their displacement and energy
  quantities; new readouts initially show RMS primary field and total energy. Area
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
- **Files:** Save scene downloads JSON in the browser or opens a native save
  dialog. Load scene uses file upload/native selection and validates before
  replacement. Successful loading clears history; malformed files leave the
  current document intact. Examples opens a thumbnail gallery; opening one is
  undoable. Export can write a geometry SVG, a PNG snapshot, or a silent 30 FPS
  recording of the current viewport at its physical pixel resolution. Captures
  follow the active View settings while omitting panels, floating readouts,
  selection emphasis, gizmos, marquees, and active-tool prompts. Recording leaves
  playback and viewport navigation live; its status-strip control shows elapsed
  time and stops/finalizes the file. Documents autosave after edits and restore on
  startup from browser local storage or the native per-user recovery file. Copy
  scene link embeds compressed, validated scene data in a `#scene=v1.…` URL
  fragment; while that fragment is active, later autosaves keep it current.
  The catalog includes ready-to-run GRIN rod and circular Luneburg profiles with
  their drivers, probes, and wave-speed view presets. View toggles, field intensity,
  and material-overlay settings travel through files, links, recovery, and examples.
  Camera, selection, open panels, and floating-window layout are not saved. With no
  shared scene or autosave to restore, startup opens the first
  bundled example.
- **Mesh:** enable Accepted triangle mesh under Display. The overlay shows the
  constrained mesh and its labeled outer, hole, interface, closed-wall, and baffle
  edges. Elements below 15° are
  amber; the panel reports counts, minimum angle, maximum edge, refinement
  progress, build/work time, longest mesh slice, and explicit construction
  failures. Meshing targets a soft 2 ms per frame. An invalid draft keeps the last
  accepted mesh.
- **Resolution:** parent maximum edge 0.08 is the default for P2e; choose 0.04 for
  a finer solve or 0.16 for a quick preview. Changing resolution rebuilds the
  accepted mesh without changing geometry/history. Resolution is not stored in scene files.
  Full fine builds can take tens of seconds when spread across frames. Small
  hole, material-interface, and open-baffle control-point edits reuse and repair a
  bounded region of the previous mesh. Baffle repair keeps its coincident left/right
  traces and shared free tips intact, and subdivides both faces together. A failed
  local repair retries from the unchanged committed mesh with up to two larger guard
  regions before using a full rebuild. The Performance panel reports attempts, retry
  causes, patch size, repaired baffles/trace segments, unchanged elements, and a
  session fallback-cause count. Creation, deletion, knot/topology changes, closed-wall
  motion, large edits, and exhausted repairs still rebuild.
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
  in bounded frame slices. Fast, Balanced, and Detailed presets set the error and
  wavelength targets; advanced controls bound the smallest and largest element.
  Refinement reacts immediately, while coarsening requires two quiet estimates and
  has its own transaction quota. The View panel can overlay the current target field.
- **Waves:** Run/Pause, Step, and Reset operate the GPU solver. Place pulse adds a
  Gaussian displacement with zero initial velocity. The optional point source is
  repositioned by dragging its viewport marker. Simulation speed is bounded to 16
  substeps per display frame. Field colors use an adjustable symmetric gain.
  Pulses and point sources act only in their containing wall-separated
  region. With open baffles their Gaussian stencil uses mesh-path distance, so it
  goes around a free endpoint instead of jumping through coincident faces.
  The panel reports DOFs, GPU buffer size, operator-derived timestep, simulated
  time, substeps, throughput, operator/map preparation time, and discrete energy.
- **Outer boundary:** select each box side independently and assign zero Neumann
  (reflecting), prescribed Neumann flux, prescribed Dirichlet displacement,
  first-order outgoing, or second-order auxiliary behavior. Prescribed data uses
  `offset + amplitude · sin(2π f t + phase)`; zero amplitude gives a constant.
  Adjacent Dirichlet sides must agree at their shared corner. The second-order
  Engquist-Majda condition adds tangential propagation and reduces oblique
  reflection. Hole and baffle spans support reflecting or local first-order
  impedance conditions; closed walls remain reflecting and material interfaces
  transmit. Changes reuse the mesh and transactionally transfer the live field.
- **Solution adaptation:** a resumable quadratic indicator combines recovered-flux
  defects, interior equation and flux-jump residuals, and the active physical
  boundary laws. It includes paired thin-gap traces and second-order radiation
  memory while treating prescribed Dirichlet mismatch as a diagnostic. The result
  drives bounded refinement and coarsening with wavelength and grading limits.
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
Load [examples/eight-obstacles.json](examples/eight-obstacles.json) for a
representative scene.

## Check

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build -p funfern-app --locked
trunk build --release
npm ci
npx playwright install chromium
npm run test:browser
cargo run -p funfern-core --release --example mesh_timing -- --slices
cargo run -p funfern-core --release --example mesh_timing -- --wave --slices
cargo run -p funfern-core --release --example mesh_edit_timing -- --paced
cargo run -p funfern-core --release --example wave_convergence
cargo run -p funfern-core --release --example wave_boundary_reflection
cargo run -p funfern-app --release --locked -- --mesh-edit-benchmark
cargo run -p funfern-app --release --locked -- --wave-gpu-check
cargo run -p funfern-app --release --locked -- --wave-transfer-check
cargo run -p funfern-app --release --locked -- --amr-check
```

Set `PLAYWRIGHT_CHANNEL=chrome` to run the smoke test with an installed Google
Chrome instead of Playwright's pinned Chromium.

GitHub Actions runs formatting, Clippy, the workspace tests, and native and WASM
release builds for pull requests and pushes to `main`. The Playwright smoke test is
a local hardware-backed check because GitHub-hosted runners do not expose a usable
WebGPU adapter. It rejects browser rendering failures, checks that the canvas
continues to change, and verifies that its backing render target follows a viewport
resize. A successful `main` run publishes the browser bundle to
<https://dmitriyne.github.io/funfern/>. The Pages source is configured as
**GitHub Actions** in the repository settings.

The automated tests cover spline evaluation/derivatives, seam insertion, exact predicates,
constrained topology, concave and multiple holes, driven and absorbing internal spans,
nested material inclusions,
shared interface traces, separated wall traces, refinement limits, geometry
rejection, draft/accepted history, scene files, and egui pointer/keyboard
interactions, local Delaunay legality, incremental work, slice-independent output,
P1 and piecewise enriched-quadratic assembly, positive mass lumping, degree-four stiffness
quadrature, stable centered stepping, energy/damping behavior, spatial mode
convergence, GPU upload parameters, and a 32-obstacle meshing regression. The timing example uses the app's mesh
settings and 2 ms scheduling policy. `--wave` tests maximum edges 0.04 and 0.02;
without it, the example tests preview resolution. It reports P1 spatial DOFs,
actual edge size, and resolution relative to a reference wavelength of 0.4.
These are meshing timings. `wave_convergence` separately measures the analytic
reflecting-box mode for P1 at h=0.04 and h=0.02 and enriched quadratic triangles
at parent h=0.08 and h=0.04, with independent temporal refinement over one and
five box-crossing times.
`wave_boundary_reflection` sends finite Gaussian P2e packets at the outer box and
compares first- and second-order residual-energy reflection at two angles and two
wavelengths, alongside the ideal continuous plane-wave coefficients and a
long-time finite-state check.
Omit `--slices` for per-phase profiling.
`mesh_edit_timing --paced` applies edits with 2 ms mesh slices at a simulated
60 Hz schedule, excluding rendering. `--mesh-edit-benchmark` opens the real native
app with eight obstacles and the production parent-h=0.08 overlay, applies three small control edits,
prints edit-to-ready and active/scheduling times plus exact element reuse, then
exits. Interactive editor input is disabled during this scripted run. It does not
read or overwrite scene files. The native benchmark includes
editor validation and rendering load; mesh-ready means the atomic simulation commit.
`--wave-gpu-check` runs mixed harmonic Dirichlet and Neumann data on the outer box
and both faces of a baffle, both outgoing orders, and a closed wall for 128 steps.
It reads both time levels and boundary memory back, compares them to f64, reports
solve-to-readback throughput, and exits. `--wave-transfer-check` injects a nonzero
field, performs a real control-point edit, verifies transfer of all three state
components, then verifies that a lower-order boundary transaction clears the
auxiliary state, and finally checks a same-mesh material-coefficient transaction
against f64.
`--amr-check` first requires the normal automatic controller to finish an aligned
GPU-to-host solution estimate, then evolves a nonzero field, moves a deterministic
spatial refinement target, requires both refinement and coarsening, transfers all
quadratic state with no exposed nodes, verifies mesh/operator revision agreement,
and exits.
The field view tessellates every quadratic parent triangle into six display
triangles around its shared edge-midpoint and element bubble nodes. Native GPU
startup and the wave kernel were exercised on Apple M1 Max / Metal. The user
reports completing the Milestone 1 browser interaction checks; browser metadata
and interactive checks for the quadratic solver have not been recorded.

## Layout

```text
crates/funfern-core/       Dependency-free f64 geometry, meshing, and CPU wave reference
crates/funfern-app/        Bevy/egui editor, persistence, GPU waves, and field display
examples/                Scene files for exercising the editor
docs/plan.md             Milestones and completion criteria
docs/architecture.md     Representation, draft model, and later solver design
docs/engineering-log.md  Verification results and remaining work
```

Bevy owns the wgpu device. The current viewport uses egui's painter through
`bevy_egui` on that device; there is no second renderer/device. Audio, 3D render
pipelines, and the WebGL fallback are disabled. All numerical geometry remains
independent of Bevy, egui, serde, and external numerical libraries.
