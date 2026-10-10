# funfern checks

The routine checks are in the README. This file covers the specialized
convergence, benchmark and GPU-versus-reference runs, and what each one
asserts.

```sh
cargo run -p funfern-core --release --example mesh_timing -- --slices
cargo run -p funfern-core --release --example mesh_timing -- --wave --slices
cargo run -p funfern-core --release --example mesh_edit_timing -- --paced
cargo run -p funfern-core --release --example wave_convergence
cargo run -p funfern-core --release --example wave_boundary_reflection
cargo run -p funfern-core --release --example temporal_amr_calibration
cargo run -p funfern-core --release --example canonical_temporal_timing
scripts/device-suite.sh
```

`scripts/device-suite.sh` runs every `funfern-app` GPU-versus-reference example,
in the configurations it lists, on this machine's GPU, and prints a verdict per
run; given a pattern it runs the matching ones. A run fails on a device fault, a
timeout, or an error outside its bound, a NaN included: the examples judge their
errors through `examples/support/check.rs`, since `error > tolerance` is false
for a NaN and `f64::max` drops one. The examples open a window and read the
autosave, so the script runs each with `HOME` pointed at a scratch directory,
after building them with the real `HOME`; run one by hand the same way, or the
toolchain is re-fetched into the scratch one. Nothing runs them in CI, which has
no GPU.

The examples drive the device request directly. The app's own end-to-end
fixtures (`src/ui/e2e.rs`) run the app as it ships, built with the `e2e` feature
and chosen by `FUNFERN_E2E=<fixture>`: a driver beside the frame acts only
through the app's handlers - it opens a document, places a pulse, presses Run -
and reads only what the device accepted, its accepted-step counter, the serial
of the last event it processed, and a full snapshot stamped with the step it
holds. The f64 reference steps on the generation the app accepted, so on the
same mesh. The one thing the feature adds to the app is where a run stops, since
the app asks for steps by the wall clock. The process exits 0 when the fixture
holds; the suite runs each. `cavity` strikes a closed reflecting box of the
default medium, grid filter off, with one pulse at step 0 and compares Q and b
after 256 steps, to 5e-6: it reads `6.2e-7` and `1.0e-6` at 11,121 degrees of
freedom. Every fixture reads 6e-7 to 1.2e-6 on the M1 Max; 5e-6 leaves four times
that, where the device examples' 3e-5 let a remesh that lost its component totals
through. Negating b, reading the lanes a step old, or stepping the reference one
step short each fails it. `cavity-batches` runs the cavity four times, its steps
asked for one at a time, seven, 128 and all 256 a frame, and requires the
accepted state at the endpoint to be the same bits each time, which it is: the
device keeps its clock and control on the GPU, takes no host write while
stepping and reduces in fixed-order trees. The 11,121 nodes leave a partial
workgroup at 128, 64 and 16. One flipped ulp in one run, which no tolerance on
Q and b sees, fails it. A fixture holds the run at the steps where it acts, so
an action lands at a step both sides know and the reference takes it there.
`switch` adds a disc of the switchable medium to the cavity and presses Switch's
hotkey at step 128 of 256, once the pulse's wave has reached the disc; the
reference throws the Switch the app records sending, with the material's ramp,
at the same step. It reads Q `7.4e-7` and b `9.4e-7`, and the run without the
Switch stands `8.8e-2` off, which the fixture requires to be a hundred
tolerances or more, so a Switch the device ignored cannot pass. A reference
that omits the Switch, throws it the other way, or throws it a step late fails.
`handoff` raises the cavity medium's permittivity by half at step 128, as the
material panel's Apply does: a whole new generation, handed the running field by
an admitted handoff. The reference carries its own state through the maps the
candidate was prepared with, taken from it before the runtime drops them, keeps
each component's total as the device does, and steps on at the step the app
uploaded. Q reads `9.9e-7`, the run without the edit `1.4` off; a reference left
on the old step fails at 1.1. `remesh` sets the target edge finer at step 128,
nothing else changed: the field is interpolated from 11,121 nodes onto 15,349 of
the same geometry. Q reads `7.7e-7`, and a reference that drops the component
totals the runtime keeps reads 2.5e-5 and fails. `rejection` makes the cavity's
medium defocusing Kerr with an amplitude bound of 0.2 and drives it with its
point source at the (1, 1) mode until the field reaches the bound. The device
refuses the step after 788 with the inverse-domain status, as the reference
refuses it; the app pauses on its own, the accepted state reads Q `7.3e-7`
against the reference's step 788, and Run - which clears the failure - retries
the step to the same refusal with the stored state the same bits. A reference a
step short, or a retry that moves one bit, fails it.

CI has no GPU, but it runs these fixtures on Mesa's software Vulkan, lavapipe,
under a virtual X server: the job `Device fixtures on software Vulkan` builds
the app with the feature and runs `scripts/e2e-lavapipe.sh`, which takes the
fixture list from `scripts/device-suite.sh` and fails a run that did not report
lavapipe's adapter. They read there as on the M1 Max - the cavity Q `6.4e-7` -
each its own bits across batchings, in about forty seconds a fixture; the Pages
deploy waits for the job. Lavapipe flushes subnormals to zero, which is how it
found the nonlinear inverse's relative convergence tests unreachable near zero.

The same fixtures run in the browser from a bundle built with the feature,
`TRUNK_BUILD_FEATURES=browser-threads,e2e scripts/trunk build --release --dist
dist-e2e`, which the Pages bundle is not. `npm run test:e2e` serves it
cross-origin isolated and opens `?e2e=<fixture>` for each fixture the device
suite names; the driver writes its verdict on the page's root element,
`data-funfern-e2e` reading `pass` or `fail` beside `data-funfern-e2e-summary`,
and its lines on the console, and `e2e-deadline=` raises its time limit.
Every browser run first holds the page to the build it meant to load:
`build.rs` stamps the application with `FUNFERN_BUILD_ID`, the commit CI
checked out or the checkout's own, the application writes it on the root
element as `data-funfern-build` (and shows it in the diagnostics window,
prints it for `--version`), and the startup, end-to-end and Safari runs
compare it with `FUNFERN_EXPECT_BUILD` or this checkout's commit, so a stale
service worker or a cached bundle serving yesterday's application fails the
run instead of passing on old code. In
Chrome on the M1 Max (`PLAYWRIGHT_CHANNEL=chrome`), whose WebGPU runs on
Metal, five fixtures read what the native app reads to every printed digit -
the cavity Q `6.242e-7` - and the refusal lands at step 788 as natively. A
reference a step short fails there as it does natively. A seventh fixture,
`device-loss`, destroys the application's own GPU device at step 128, right
after a change to the document the autosave had not yet written: within two
seconds rendering must have stopped and the app said so - `data-funfern-stopped`
on the page's root element, `device-lost` or `render-error`, and the startup
overlay back with "funfern stopped … The document is saved; the running
simulation is not." - and the autosave must open to the document as it was at
the loss, change included; natively the process then exits by the fixture's
verdict, and nothing stays pending. The browser spec, which fails a fixture at
the first "Caught DeviceLost" or "Caught rendering error" on the console, lets
this fixture have them and fails it on Bevy's "Quitting the application"
instead, which the app's handler replaces.

Each fixture also holds its mesh to the one recorded, to the bit: the
triangles' vertices and regions, and every vertex's coordinates (`CAVITY_MESH`,
`DISC_MESH` and `FINER_CAVITY_MESH` in `src/ui/e2e.rs`). The mesher decides
through portable arithmetic, and the M1 Max natively, Chrome's wasm and Linux on
arm64 build the three meshes alike, the disc's curve included; CI's x86_64
runner holds them each run. A change to the mesher made on purpose records them
again.

Safari runs them by hand, as no CI browser runs its WebGPU: with remote
automation on in Safari's developer settings, `npm run test:e2e:safari` serves
`dist-e2e` isolated and drives Safari through `safaridriver`, speaking WebDriver
directly, fixture by fixture, and exits 1 if one fails. On Safari 26.3.1 on the
M1 Max all six pass in about a minute, within a percent or two of Chrome's
readings - WebKit translates the shaders its own way onto the same Metal - the
refusal at step 788 as everywhere.
The site itself is served another way: GitHub Pages sends no isolation
headers, so the page installs `coi-serviceworker.js`, which adds them to every
response, and reloads once. `npm run test:e2e:pages` runs the fixtures as the
site runs them: a bundle built for the production path,
`TRUNK_BUILD_FEATURES=browser-threads,e2e scripts/trunk build --release
--public-url /funfern/ --dist dist-e2e-pages`, served under `/funfern/`
(`server.py --prefix`) without the headers. On the cold start the first
navigation registers the worker and the page reloads itself - two navigations
of the main frame, then the page is cross-origin isolated, a worker controls
it and the reload flag is gone - and the fixture passes; a warm reload starts
in one navigation, the worker already there, and passes again, the build
attribute checked on both. The cooperative builds run the
same way. `npm run test:e2e:plain` serves the single-threaded bundle, built by
plain `trunk` without the wrapper's shared memory
(`TRUNK_BUILD_FEATURES=e2e trunk build --release --public-url /funfern/ --dist
dist-e2e-plain`), on the Pages path, where such a bundle lives, and holds the
fixtures with no worker pool on the page (`FUNFERN_EXPECT_BROWSER_WORKER=0`);
`npm run test:e2e:nopool` serves the threaded bundle under the headers with
`?e2e-pool=off`, which makes the page's shared-memory growth check report what
it reports on iOS Safari, so the app runs its work on the main thread
(`data-funfern-preparation-worker` reading `unavailable`), and holds the
fixtures there. With service workers blocked the page shows its note that
funfern needs shared memory and never starts, for either bundle. `scripts/browser-suite.sh` is the four in one: it
builds the three bundles, or runs against them as they are with `--no-build`,
runs the four hosting variants one after another, prints a verdict a run and
each fixture's line, and exits 1 if any failed. It is the browser gate for a
change to the web shell - `index.html`, the service worker, the Playwright
specs, `scripts/trunk`, the wasm entry points - as the device suite is for a
change to the shaders or the runtime.

No CI job runs the browser fixtures yet. Chromium's software WebGPU, SwiftShader,
found its adapter in Playwright's image on arm64 Linux, but lost the app's device
as it started ("A valid external Instance reference no longer exists"), with the
config's Linux flags and with none; the attempt is in the log of 2026-10-09 and
the TODOs. For such a run the spec takes more Chromium flags from
`FUNFERN_CHROMIUM_ARGS` and requires the page's adapter to name
`FUNFERN_EXPECT_ADAPTER`, and a lost device or any other fatal console message
fails a fixture at once rather than at its deadline.

`examples/` is an export of the gallery's catalog, which stays the source.
`cargo test -p funfern-app --test examples` fails when a scene changes without
its file; regenerate them all with

```sh
FUNFERN_BLESS_EXAMPLES=1 cargo test -p funfern-app --test examples
```

Set `PLAYWRIGHT_CHANNEL=chrome` to run the smoke test with an installed Google
Chrome instead of Playwright's pinned Chromium.

Run `npm run test:shaders` (with `PLAYWRIGHT_CHANNEL=chrome` where the pinned
Chromium does not start) whenever a shader changes. It compiles every WGSL file
in Chrome's WebGPU, whose compiler applies WGSL's uniformity rules, as the
browser build meets them. The workspace tests validate the shaders with naga,
which the native build also compiles through, and naga accepts shaders the
browser rejects: twice now a shader has passed every native check and failed
the whole browser build, most recently a `workgroupBarrier` under a branch on a
flag read from read-write storage.

A browser is not given the WGSL as written. Bevy composes every shader into a
naga module, and wgpu writes it back out as WGSL for the browser. That writer
prints each `f32` constant in full digits, and Safari cannot read a digit-only
float literal past 2^63: `3.0e38` goes out as 39 digits and Safari rejects the
whole shader, which Chrome, and so the browser test, reads without complaint. The
workspace test `every_shader_reaches_a_browser_in_a_form_safari_reads` writes each
shader through naga's writer and fails on such a literal. Safari itself is checked
by hand, since no CI browser runs its WebGPU: with remote automation enabled in
Safari's developer settings, `safaridriver` can drive it against a local
`browser-tests/server.py --isolated` serving the built bundle.

iOS Safari 26.2 has a defect no desktop browser shows: its main thread does not
see a worker's growth of shared wasm memory in `memory.fill` and `memory.copy`,
so the threaded bundle's first bulk copy into memory its workers grew traps inside
the allocator, and the page freezes on its splash. `index.html` checks for exactly
this on a two-page memory before the app starts its background pool, and where
the check fails the app runs without the pool, its preparation and adaptation on
the main thread. An iPhone is driven like desktop Safari, with
`platformName: iOS` and Remote Automation on in its Safari settings; it reaches
a local bundle only over HTTPS, through a tunnel such as
`cloudflared tunnel --url`.

GitHub Actions runs two jobs in parallel for pull requests and pushes to `main`:
one checks formatting and Clippy, runs the workspace tests and makes the native
release build, and the other makes the threaded WASM release bundle. Each caches
its compiled dependencies under a key naming the toolchain and the exact lockfile
(for the bundle also the Trunk version, `scripts/trunk` and `Trunk.toml`), and
restores only an exact match, so a cache is one clean build and never grows; a
changed lockfile or toolchain costs one cold run. Caches are saved from `main`
only, after the build and before the tests run. The Playwright smoke test is
a local hardware-backed check because GitHub-hosted runners do not expose a usable
WebGPU adapter. It rejects browser rendering failures, requires the page's
shared-memory check to pass so that the check cannot switch off a working pool,
checks that the canvas
continues to change, and verifies that its backing render target follows a viewport
resize. A successful `main` run publishes the browser bundle to
<https://dmitriyne.github.io/funfern/>. The Pages source is configured as
**GitHub Actions** in the repository settings.

The Materials panel is also driven by generated sequences of its own actions
(`crates/funfern-app/src/ui/sequences.rs`): typing, presets, Apply, Revert,
selection moves, Undo and Redo, a physics switch and New, in orders nobody
wrote down, checked against a shadow of the history and a post-condition per
action. The gate runs a few dozen sequences; `PROPTEST_CASES=2000 cargo test -p
funfern-app --release --bins material_edits_in_any_order` runs a sweep. A
failure is shrunk to the shortest sequence that still fails, printed, and its
seed written under `crates/funfern-app/proptest-regressions`, which is checked
in so the case is replayed first everywhere. A seed names its sequence only
through the generator as it stands: a changed action, weight or text makes
every seed replay something else. So each finding also gets a focused test,
and the file keeps only the seeds of findings drawn by the current generator,
not those a sweep writes while the harness itself is being fixed.

A second machine (`ui/sequences/reopening.rs`,
`documents_survive_reopening_between_any_edits`) runs the same panel with the
document's other edits among its actions, drawing, dividing, baffles, control
drags, deletions, detached ends, probes, the source and the domain, and saves
and opens the document again between any two: by Save and Open, by a shared
link and by the browser's autosave, each through the handler the app has for
it. A reopened document must equal what a file keeps of the saved one, and
validation must bring it where the session would have come. After every
action, in every panel machine, the document must save, and a Valid
acceptance must mean the accepted scene is the draft. Its seeds have a file
of their own.

A third machine (`ui/sequences/unfinished.rs`,
`unfinished_edits_land_on_their_own_document_or_nowhere`) runs the panel with
edits overtaken before they are let go: text typed without Enter and let go
by a click on a Library row, on the toolbar's Undo or Redo or on nothing, the
click in one frame or its press and release in two, and a colour dragged in a
picker while Cmd+Z or Cmd+Shift+Z steps the history. What was begun must land
on the document it was begun in when nothing moved, and go when something did.
It starts with a parameter on the open material, so a name field is there to
type into. After every action, in all three panel machines, no parameter name
typed may outlive its field and no colour pick the pointer. Its seeds have a
file of their own.

A fourth machine (`ui/sequences/generations.rs`,
`the_host_and_the_device_agree_on_what_runs`) runs the runtime itself under
interleavings of host frames, device completions and user actions: edits, mesh
edge and speed changes, Remesh, source edits, Reset, Run and Step, pulses,
Switch, scenes opened, Undo and Redo, while the device delivers readbacks,
admits or refuses a handoff, takes or refuses a live event, or faults. The
device is the request's test transitions, preparation advances by the slices
each frame is granted and packing happens on the frame, so the order is the
sequence's alone. While nothing uploads, the host's active topology must be
the one the device runs, at the step it was packed for; a topology is only
prepared from a scene the editor has held since its last replacement; a commit
without a new generation must be one that needs none; and with the device fair
at the end, the runtime must come to rest on the accepted revision or with an
error naming it.

The protocol that machine exercises is also a small state machine of its own,
`crates/funfern-protocol`: requests, preparation, packing, uploads, handoffs,
faults, Reset, scene replacement and live events, with what the app computes
from geometry as inputs of each step. `cargo kani -p funfern-protocol` checks
it with [Kani](https://model-checking.github.io/kani/): every state reached in
nine steps of any kind keeps the host's active topology on the device while
nothing uploads, at the step the device runs, and from any state reached in
six, a fair device and preparation bring it to rest within nine rounds. It
takes about three and a half minutes; CI runs it in a job of its own, with
Kani 0.67.0, and Pages deploys only once it has passed. The runtime machine
runs the model in lockstep with the real runtime and compares the two after
every step, so the proofs speak about the app's runtime only while that
comparison holds. The protocol's decisions are functions in the crate's
`decisions.rs`, which the runtime calls where the model does, so for those the
proofs explore the code that decides in the app; what the runtime reads to
give them their inputs is still held only by the comparison. They are whether
the document as it stands takes a new request and whether that starts the
field from zero, how a ready candidate is carried - in place, by a live patch
or as a whole generation - how an upload in flight settles, when the steps a
frame asks for are held back, what the host does about a failure of the
running generation, and what becomes of a live event - a source patch, a
pulse, a Switch press - the running generation would not take.

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
`canonical_temporal_timing` records the actual-core incremental cost of a
time-driven medium, per boundary composition, with the same mesh, operator,
forcing and timestep on both sides. At `h=0.1` the driven bulk runs about `13x`
the fixed bulk, which is the floor: the cost of recomputing a stage's
coefficients in every helper that wants them. The second-order outgoing wall
used to sit at `35x`, well above that floor, because it rebuilt its trace
factorization every stage. It now sits at `8x`, below the floor, because that
system carries no nodal mass: one preparation serves every stage and the mass
arrives at the solve. The fixed column is unchanged - a generation whose mass
cannot move still inverts once and solves in a single pass.

Some tests hold the physics itself to references that owe nothing to the
solver. A spatially uniform field in a closed box feels no gradient, so every
node is one oscillator: a sine-Gordon medium's is the pendulum, checked swung
to 2 radians against its exact period `4K(sin 1)/ω₀` and against RK4
(`a_sine_gordon_uniform_mode_swings_as_the_pendulum`), and a Kerr medium with a
Klein-Gordon cutoff its own ODE, against RK4 with the flux inverted by Newton
from the law's definition (`a_kerr_uniform_mode_follows_its_ode`). Both
integrators converge at second order: the production step's fourth order is
the linear bulk's. A pendulum law 0.1% too strong or a Kerr coefficient 1% too
large fails them. A uniform medium switched at once to four times its
permittivity keeps `D` and `B`, its `E` falls to exactly a quarter with that lane's
energy, the other lane's kept, and from the switch it runs as the unswitched
medium at half the step - `(Q(t/2), b(t/2)/2)` in TM, `(Q(t/2)/2, b(t/2))` in TE -
which the discrete step inherits; the two runs agree to the bit
(`a_uniform_permittivity_switch_keeps_d_and_b_and_runs_as_the_slower_medium`). A
pulse down the two-arm channel at normal incidence on a planar interface, in TM
and in TE, splits as Fresnel says: medium 2 with `ε = 4` reflects 1/9 of the
energy to 3e-4 and transmits the rest to 1e-5, read against the reference arm
when the reflected, transmitted and reference pulses are clear of the
interface, and the transmitted pulse lags the reference by what the slower
medium adds, to 1e-4 s; `ε = μ = 2`, as slow and matched, reflects under 1e-5
(`a_pulse_at_an_interface_splits_as_fresnel_says_and_lags_by_the_slower_medium`).
An impedance 0.2% off at the right speed fails it. An outgoing wall
at the end of a channel is read against the same channel with the wall moved
far off, and at normal incidence a reflecting wall returns everything, either
outgoing order under 2e-5; met by the channel's second transverse mode at 27°
to 39°, read on one mesh against reflecting and zero-Dirichlet references, the
first-order wall returns `((1 − cos θ)/(1 + cos θ))²` of the energy to 1% and
the second-order wall 1.10 of its passive law at the default step, an excess
of the stepping listed under "Worth checking sometime" in `docs/plan.md`; and
between electric walls in TM, where the field vanishes at the corners, the
second-order wall's trace ends there and the mode reflects as it does between
reflecting sides, to 2%
(`an_outgoing_wall_returns_what_its_law_says_between_free_and_pinned_sides`).
With the pinned corners kept on the trace it reflected 6.6 times its law. The
box mode `cos(3π(x+1)/2) cos(3π(y+1)/2)` runs on the production path, the
application's mesh of the reflecting box under its fourth-order step at its
recommended size, with space and time told apart exactly: on a linear mode
each integrator's dispersion relation is exact, so the semidiscrete frequency
is read from a leapfrog run and from a fourth-order run without
extrapolation and the two agree to 1e-8 over three step sizes; the stepped
frequencies miss it by `ω_h³dt²/24` and `−ω_h⁵dt⁴/720` to 0.1% and 0.2%; the
semidiscrete frequency misses the exact one by 3.7e-5, 9.9e-7 and 1.1e-7 at
edges 0.16, 0.08 and 0.04, the element's fourth order; the amplitude holds
over ten periods and the energy wobbles by a quarter of `(ω dt)²` without
drifting
(`a_box_mode_on_the_production_path_keeps_its_frequency_in_space_and_time`).
A fourth-order correction of `dt²/13` instead of `dt²/12` fails it: the
fourth-order run's semidiscrete frequency lands 7.6e-5 off leapfrog's. Edits the field
cannot tell run on the application's own path: a disc of the box's own
medium drawn into the running box mode and erased three periods later, the
state carried each time through the maps the runtime prepares for the edit,
is off the untouched run by 7e-4 at edge 0.16 and 5e-5 at 0.08 right after
the disc, 9e-4 and 1e-4 three periods after it is erased; a remesh 1.4
times finer by 1e-3 and 2e-4 at once, 3e-3 and 4e-4 three periods on. A
curl-free flux, the gradient of `cos(πx/2) cos(πy/2)`, pushes no field -
sampled, its compatible part is 1e-13 of it by energy, and less that part
its field stays at 1e-14 over three periods - and carried across that remesh
keeps its
energy to 4e-7 and 5e-9, a compatible share of 3e-10 and 5e-12, and gives
the field 1e-10 of its energy
(`edits_the_field_cannot_tell_leave_it_as_the_mesh_allows_and_a_static_flux_static`).

`temporal_amr_calibration` measures the AMR estimator's efficiency index, its
estimate over the true error, across a refinement sequence on a smooth
reflecting-box problem. It crosses the driven constitutive row against the
spatial pattern in it - inert, each row pumped uniformly, each row carrying a
travelling pattern, one row at two wavenumbers, and both rows at once - because
a sweep that changes the row and the pattern together cannot say which one the
estimator is charging for. The index should be bounded and roughly constant;
where it climbs with refinement the estimate cannot be read as a percentage.
The rows after the oscillator media carry boundary data - a prescribed side, a
Neumann side, absorbing walls, and a channel with one absorbing end that a
pulse reaches during the run - and then loss: constant primary and
complementary loss, loss beside Klein-Gordon, and the self-sustained emitter's
medium, van der Pol gain with a complementary loss. Every row's estimate is
handed the walls its solver ran, as production's is, so the wall residual is
measured too. The last rows are self-similar: the box mode at two and three
times the wavenumber, on meshes and over a run that many times smaller, which
leaves the true relative error where it was, so an index that moves with them
is weighting by frequency. One estimator reads every generation, fixed and
driven; the "static path" rows run the fixed generation's supplement, and on
an inert medium they read the time-driven one's index to three digits.
`INDICATOR_CALIBRATION` is set so the smooth box reads 1.41, the index the 6
percent target was calibrated at; every row then reads 1.26 to 1.67, except the
pumped interface, down to 0.97 at its finest mesh, and the self-oscillating
medium at 0.86 to 0.90. An argument runs only the rows whose label contains
it.

`wave_boundary_reflection` sends finite Gaussian P2e packets at the outer box and
compares first- and second-order residual-energy reflection at two angles and two
wavelengths, alongside the ideal continuous plane-wave coefficients and a
long-time finite-state check.
Omit `--slices` for per-phase profiling.
`mesh_edit_timing --paced` applies edits with 2 ms mesh slices at a simulated
60 Hz schedule, excluding rendering.

The `--mesh-edit-benchmark`, `--wave-gpu-check`, `--wave-transfer-check` and
`--amr-check` application flags no longer exist; the unified-topology cutover
removed them and the hidden `canonical_gpu_*` examples took over what they
covered.
`canonical_gpu_driven_document` is the only end-to-end run: an authored
document with a material drive, meshed and assembled by the application's own
resumable jobs, compiled into a temporal plan and stepped on the device against
an f64 oracle. It reads `2.46e-7` over forty-eight steps. `DRIVEN_WALLS=outgoing`
puts the drive behind a second-order outgoing wall - the document's default, and
the combination that used to be refused - and reads `3.31e-7` over the same
forty-eight. `DRIVEN_STEPS` and `DRIVEN_DEPTH` override the step count and the
modulation depth. `DRIVEN_LAW=sine-gordon` or `van-der-pol` authors an
oscillator from the editor's own presets instead and also compares `r`.

`canonical_gpu_temporal_timing` is the throughput comparison that decides
whether a drive is affordable, run twice with and without `--fixed` on an
otherwise identical fixture. On an M1 Max at 15270 DOFs both read `517 us/step`:
per step the drive is free on the production core. What it does cost is the
timestep, because the CFL bound tightens with the coefficient trajectory, so a
driven medium runs about `1.23x` the wall clock per simulated second.
`--outgoing` swaps the first-order wall for a second-order one, the only
composition whose stage does non-local work. Both runs then read about
`1045 us/step` at a 328-node trace - still free per step - against `833 us/step`
before the trace solve became a sweep. That `1.26x` is what buys a driven medium
behind that wall at all, and it is confined to this composition. The sweep is
two dispatches per pass and each stays as wide as the boundary; running it in
one workgroup instead, so a pass could use a barrier rather than a dispatch
boundary, read `6975 us/step`.

`canonical_gpu_temporal_forced` runs a pumped medium with a volume source and
an absorbing wall - three stages that each divide by the nodal mass - against
the f64 reference, and requires both state lanes within `2e-4`. It exists
because the plan compiler used to refuse that combination, so no stage had ever
been exercised with a moving mass; the first run found the kick dividing by the
authored mass and missing by `1.4e-2`. It reads `2.42e-7`, down from `5.98e-6`
once a driven stage stopped pinning half a step in.

`canonical_gpu_temporal_amr` runs the error estimate against the f32 state a
device actually produces, on a travelling mass modulation - the medium that used
to take the efficiency index from 1.4 to 17.4 before the estimate moved onto the
solver's own flux. The instantaneous coefficients come from the material runtime
decoded out of the same buffer copy as the state. On the accepted generation it
requires every term within `1e-4` of the f64 oracle, except the cell residual at
`1e-3` and the worst element indicator at `1e-2`. It then refines where the
estimate asks, hands the generation over on the device, steps the new one, and
requires the same bounds again - the estimate is as good after a refinement
transfer as before one. In between it measures the transfer on its own, host
and device applying the same maps to the same input, and requires that the
device is performing the *conserving* primary transfer: the free one misses by
`2e-4`, the conserving one lands at `6e-8`, and the complementary transfer is
exact. The estimate is handed the walls the solver runs and production's rate.
`TEMPORAL_AMR_LOSSY_WALLS=1` adds a complementary loss and absorbing walls, the
two compositions whose terms the estimate reads from the direct state itself.
The field view tessellates every quadratic parent triangle into six display
triangles around its shared edge-midpoint and element bubble nodes.

`canonical_gpu_filter_boundary` runs a point probe whose sample stride equals
the resident grid filter's cadence, the worst case the speed control can
produce, and requires the recorded field and rate to match an f64 oracle. The
filter flips the accepted state lane without advancing time, so a recorder
that differenced the lanes at that boundary reported a filter correction
divided by the timestep instead of a rate.

`canonical_gpu_oscillator` steps an oscillator medium (Gate O) on the device
against the f64 reference and requires `Q`, `b` and the integrated field `r`
within the Stage 0 `3e-5` after 200 steps. `OSCILLATOR_MEDIUM` picks
Klein-Gordon, sine-Gordon, a moving kink, a φ⁴ wall, or sine-Gordon beside a
pump or a Kerr row; `OSCILLATOR_COMPOSE` adds a wall of either order, a gap,
pins, a source, loss or a material junction; `OSCILLATOR_FILTER=1` runs the
resident filter. `van-der-pol` adds the Bernoulli loss stage and compares the
active-gain and primary-loss lanes to `1e-3`; `OSCILLATOR_AMPLITUDE=0.05` puts it
below its threshold, where it grows. Across all 120 combinations the worst
state reads `1.4e-5` and the worst lane `2.8e-6`. A filter that left `r` where
it was reads `1.3e-4` on sine-Gordon.

`canonical_gpu_oscillator_handoff` hands `r` to the next generation on the
device against `transfer_integrated_field`: `remesh` (a moving kink onto a
non-nested finer mesh), `identity`, `from-linear` (starts at `r = 0`),
`to-linear` (drops it), and `reject`, where a φ⁴ target's bound refuses the
transferred field with status 6 and the source keeps stepping. Each accepted
handoff holds `Q`, `b` and `r` to `3e-5` over 60 steps after it.
`FAILURE_LAW=phi4` runs `canonical_gpu_nonlinear_failure` on a kicked φ⁴ wall:
the device refuses the reference's step with status 6 and a retry leaves every
stored bit, `r` included, unchanged.

`canonical_gpu_temporal_work` stamps a material Switch mid-run and requires
the runtime decoded from the state snapshot, and the bulk energy split and
pump power computed from it, to match an f64 oracle. The Switch origin is
stamped on the GPU, so this is the check that the accepted runtime reaches a
consumer rather than being guessed from the clock.

Which hardware each check has been run on, and when, is recorded in the
engineering log rather than here.
