# funfern checks

The routine checks are in the README. This file covers the specialized
convergence, benchmark and GPU-versus-reference runs, and what each one
asserts.

```sh
cargo run -p funfern-core --release --example mesh_timing -- --slices
cargo run -p funfern-core --release --example mesh_timing -- --wave --slices
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
toolchain is re-fetched into the scratch one. Every binary it runs is
hard-linked into the run's log directory first and run from there, so a build
beside the suite cannot change what it runs, and the linked app is checked to
carry the fixture driver before the fixtures start. Nothing runs them in CI,
which has no GPU.

The examples drive the device request directly. The app's own end-to-end
fixtures (`src/ui/e2e.rs`) run the app as it ships through the
`funfern-fixtures` binary, the same app from the library with the driver
added, chosen by `FUNFERN_E2E=<fixture>`: a driver beside the frame acts only
through the app's handlers - it opens a document, places a pulse, presses Run -
and reads only what the device accepted, its accepted-step counter, the serial
of the last event it processed, and a full snapshot stamped with the step it
holds. The f64 reference steps on the generation the app accepted, so on the
same mesh. The one thing the driver adds to the app is where a run stops, since
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
on the old step fails at 1.1. `handoff-driven` is `handoff` with a disc of the
parametric pump, a medium driven from the run's start: the same edit hands one
driven generation to another, its candidate packed against the time the run
had reached, and the reference carries its state by the core's driven
handoff, the pump's runtime with it. Q reads `8.5e-7` and b `1.0e-6`, the run
without the edit `1.6` off. Until 2026-10-20 the app refused every driven
handoff once a run had advanced, and the fixture timed out waiting for it.
`handoff-driven-pins` makes the cavity's whole medium the parametric pump and
drives its top side, so the pinned nodes lie on a medium whose mass at the
handoff is not the authored one, and reads the handed state at the handoff's
own step, pins included: the core's driven handoff pins them at the mass in
force there. Q reads `4.3e-7` off the pins and `1.6e-7` on them at the
handoff, and `6.8e-7` after 256 steps; until 2026-10-22 the device pinned them
at the authored mass, `1.2e-1` off (GPU-3 of the 2026-10-20 review).
`remesh` sets the target edge finer at step 128,
nothing else changed: the field is interpolated from 11,121 nodes onto 15,349 of
the same geometry. Q reads `7.7e-7`, and a reference that drops the component
totals the runtime keeps reads 2.5e-5 and fails. The six `handoff-memory`
fixtures, `-rebuild`, `-remesh`, `-edit`, `-timestep`, `-flux` and `-running`, give
the cavity a second-order outgoing wall on its bottom side and a 0.8 Hz drive
on its top, run the pulse into the wall, and at step 192, once the wall holds
6.5e-4 of the field's energy, hand the running field over the way the name
says: Remesh pressed with nothing changed, the edge set finer, the medium's
mass density raised, the speed halved, which shortens the time step, or the
top side's law set to a 0.8 Hz harmonic Neumann flux, each held at the step
and run to 384; or, in `-running`, the top side's drive
switched on with the run not held, the device stepping on while the candidate
is prepared, as it does for a user, so the handoff lands where the device has
got to, step 202 on the M1 Max, read from the new generation's clock origin,
and the run goes to 768. The
reference carries the wall's memory the physical way, the pole currents'
densities along the trace interpolated onto the new trace and projected onto
the new basis, apart from
the normalized matrix the device uses, and runs on under the generation's own
forcing from a clock started where the run had got to. A snapshot at the
handoff, before a step, reads the carry on its own: Q over the unpinned
nodes, b, the pole states and the memory they represent agree to 4e-7 to
7e-7 on every case. At the endpoint Q and b read 7e-7 to 1.1e-6, the wall's
energy 1e-7 to 5e-7, the pole states 2e-6 to 1.9e-5 against their own 1e-4,
the device integrating them in f32 beside a field a thousand times their
size, and the clock within 1.4e-7 s of the reference's. The rebuild leaves
the device where the uninterrupted run is, 7e-7 off; the others stand 0.8 to
11 from it. The carry keeps the wall's energy: the remesh's moves it by
-2.4e-3 and leaves 4.8e-4 of the memory outside the new basis, the edit's
moves it by 1e-13, the rest by roundoff, held to 1e-2 and 2e-3; see the log
of 2026-10-13. A drive the new generation starts takes its phase moved on
by the time between the candidate's preparation and its arrival, which the
runtime transfer computes and a held run has none of: the mutation
`drive-phase-reset-on-handoff` zeroes it, and `-running` reads Q 1.4 and
fails where the held fixtures, whose drive is carried from the old
generation's own clock, do not see it. A harmonic flux or source the
handoff starts is integrated from rest there, the stage-6 rule, and the
reference runs it that way through `CanonicalForcing::handed_on`, which
also carries on unchanged what it ran. `-flux` reads Q `4.9e-7`. A device
that kept the rate anchor it was packed with, the flux's authored integral
at the candidate's preparation, read 0.62 there (fixed 2026-10-20). `rejection` makes the cavity's
medium defocusing Kerr with an amplitude bound of 0.2 and drives it with its
point source at the (1, 1) mode until the field reaches the bound. The device
refuses the step after 788 with the inverse-domain status, as the reference
refuses it; the app pauses on its own, the accepted state reads Q `7.3e-7`
against the reference's step 788, and Run - which clears the failure - retries
the step to the same refusal with the stored state the same bits. A reference a
step short, or a retry that moves one bit, fails it.

CI has no GPU, but it runs these fixtures on Mesa's software Vulkan, lavapipe,
under a virtual X server: the job `Device fixtures on software Vulkan` builds
the fixtures binary and runs `scripts/e2e-lavapipe.sh`, which takes the fixture
list from the run lines of `scripts/device-suite.sh`, not its driver probe, and
fails a run that did not report lavapipe's adapter. They read there as on the M1 Max - the cavity Q `6.4e-7` -
each its own bits across batchings, in 80 to 120 s a fixture on the four-core
runner, the rejection fixture the longest, under a driver deadline the script
raises to 600 s for the software device; the Pages deploy waits for the job. Lavapipe flushes subnormals to zero, which is how it
found the nonlinear inverse's relative convergence tests unreachable near zero.
The f64 inverse on the CPU had the same flaw for subnormal fluxes, which it
underflows to rather than flushes: a Kerr or saturable target below f64's
smallest normal could not meet its relative tests, and a Kerr medium started
from a Gaussian whose tail went subnormal at a node refused to build. Its tests
now have absolute floors of a few of the smallest subnormal's spacings
(`a_field_law_inverts_a_subnormal_flux`,
`a_kerr_state_starts_from_a_subnormal_flux`).

The same fixtures run in the browser from the fixtures binary's bundle,
`scripts/fixtures-page.sh && scripts/trunk build fixtures.html --release
--dist dist-e2e`: the page script writes `fixtures.html`, `index.html` with
its rust link pointed at `funfern-fixtures`, generated and never committed,
and Trunk writes the bundle's page as `index.html` as for the shipped one.
The Pages bundle is the shipped binary's, which carries no driver. `npm run test:e2e` serves it
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
`DISC_MESH`, `FINER_CAVITY_MESH` and `CHANNEL_MESH` in `src/ui/e2e.rs`). The
mesher decides through portable arithmetic, and the M1 Max natively, Chrome's
wasm and Linux build the meshes alike, the disc's and the channel's curves
included; CI's x86_64 runner holds them each run. A change to the mesher made on purpose records them
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
`scripts/trunk build fixtures.html --release --public-url /funfern/ --dist
dist-e2e-pages`, served under `/funfern/`
(`server.py --prefix`) without the headers. On the cold start the first
navigation registers the worker and the page reloads itself - two navigations
of the main frame, then the page is cross-origin isolated, a worker controls
it and the reload flag is gone - and the fixture passes; a warm reload starts
in one navigation, the worker already there, and passes again, the build
attribute checked on both. The cooperative builds run the
same way. `npm run test:e2e:plain` serves the single-threaded bundle, built by
plain `trunk` without the wrapper's shared memory
(`trunk build fixtures.html --release --public-url /funfern/ --dist
dist-e2e-plain`), on the Pages path, where such a bundle lives, and holds the
fixtures with no worker pool on the page (`FUNFERN_EXPECT_BROWSER_WORKER=0`);
`npm run test:e2e:nopool` serves the threaded bundle under the headers with
`?e2e-pool=off`, which makes the page's shared-memory growth check report what
it reports on iOS Safari, so the app runs its work on the main thread
(`data-funfern-preparation-worker` reading `unavailable`), and holds the
fixtures there. `npm run test:e2e:pages-blocked` serves the Pages bundle as
the site is served but with service workers refused, as a browser that blocks
the site's storage refuses them: the page cannot isolate itself, reloads once
marked `isolation=off`, gives the address back without the marker and runs
without its pool, and the fixtures must pass there, cold and warm. The threaded
bundle needs isolation only to share its memory with the pool; Chrome, Firefox
148 and Safari 26.3.1 all create the shared memory without it.
`scripts/browser-suite.sh` is the five in one: it
builds the three bundles, or runs against them as they are with `--no-build`,
runs the five hosting variants one after another, prints a verdict a run and
each fixture's line, and exits 1 if any failed. It is the browser gate for a
change to the web shell - `index.html`, the service worker, the Playwright
specs, `scripts/trunk`, the wasm entry points - as the device suite is for a
change to the shaders or the runtime.

A machine without a GPU runs the fixtures on Chromium's bundled software
Vulkan, SwiftShader. `npm run test:e2e:swiftshader` sets `FUNFERN_WEBGPU=swiftshader`,
which gives Chromium SwiftShader as the page's adapter and as its compositor's
Vulkan - with the compositor elsewhere the canvas has no shared image to draw
into and the app's device is lost as it starts - and each fixture's page must
then report SwiftShader's adapter once its verdict is in
(`FUNFERN_EXPECT_ADAPTER` names another; `FUNFERN_CHROMIUM_ARGS` adds flags).
It is slow: SwiftShader compiles the app's 77 compute pipelines before the
first step, in under a minute a page on x86_64 and about four on arm64, where
the two kicks alone take 47 and 36 s, and steps the cavity at a few frames a
second, so its fixture takes about three minutes on x86_64 and eight on arm64
where Chrome takes seconds. `FUNFERN_E2E_FIXTURES=cavity` runs only the fixtures named. It
runs on Linux; on a Mac, in Playwright's image under Colima, which shares only
the home directory, against a `dist-e2e` built from the checkout:

    docker run --rm --platform linux/amd64 --ipc=host -v "$PWD":/src -w /src \
      -e CI=1 -e FUNFERN_E2E_FIXTURES=cavity \
      -e FUNFERN_EXPECT_BUILD=$(git rev-parse --short=12 HEAD) \
      mcr.microsoft.com/playwright:v1.55.0-noble npm run test:e2e:swiftshader

CI runs it in the `browser` job: on every push and pull request the cavity
alone, isolated - a smoke check that the shipped bundle starts in a browser,
compiles its shaders through Tint and draws through the compositor - with the
deploy waiting on it; nightly at 03:17 UTC, and when the workflow is
dispatched with `browser: full`, every fixture, isolated and with service
workers refused, the two in parallel and the other jobs skipped.

A lost device or any other fatal console message fails a fixture at once
rather than at its deadline.

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
the main thread. The page says on the console why it refused: the trap, a
worker that failed, or one that did not answer within 20 s - a starved machine's
worker can take seconds to start, and five turned the pool off there. An iPhone is driven like desktop Safari, with
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
funfern-app --release --lib material_edits_in_any_order` runs a sweep. A
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
nothing uploads, at the step the device runs, and no live event in flight
while a handoff is pending; and from any state reached in six, a fair device
and preparation bring it to rest within nine rounds. The model's device, like
the real one, processes nothing once it has failed: an event still in flight
is stranded until clearing the failure sends it again. Its unit tests walk the
two sequences the 2026-10-20 review found the model could not reach, a pulse
sent while a packed candidate waits and a pulse stranded by a fault. It
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
frame asks for are held back, whether a packed candidate's upload may begin,
what the host does about a failure of the running generation, and what
becomes of a live event - a source patch, a pulse, a Switch press - the
running generation would not take.

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
the second-order wall 1.01 of its passive law, the mesh's own, held to 2%,
under the default trapezoid wall step
(`docs/spikes/funfern-outgoing-wall-step-spike.md`); and between electric
walls in TM, where the field vanishes at the corners, the second-order
wall's trace ends there and the mode reflects as it does between reflecting
sides, to 2%
(`an_outgoing_wall_returns_what_its_law_says_between_free_and_pinned_sides`).
PH-04 reads the complex reflection itself against the law at 0°, 45° and
75°, both orders: the channel's constant or second transverse mode driven
at the frequency that puts it at the angle, held to steady state, a graded
sponge in the medium's damping at the channel's far end, and a
Blackman-Harris lock-in at nine points beside the wall, the reflecting and
pinned runs giving the incident field and a unit reflection; the sponge's
own small reflection biases R by `C(1 − R²)`, so every reading is the mean
of two with the sponge a quarter wavelength apart, which turns `C` over.
75° lies 3.5% above its mode's cutoff and takes a minute's switch-on, two
minutes of lock-in and a gentle sponge; a pulse there never left the points.
In the gate, `planar_reflection_meets_its_law_at_0_and_45_degrees`, 25 s,
holds edge 0.1 to 1e-3 (measured 4.0e-4 to 6.0e-4). Nightly,
`planar_reflection_converges_to_its_law_at_three_angles`, 57 minutes on the
M1 Max, reads all three at edges 0.1, 0.07 and 0.05: each wall's error falls
at every level, 3.3 to 5.1 times over the halving, to 4.9e-5 to 1.8e-4 at
0.05, held under 2.5e-4, the points within 2.1e-5; at 45° on 0.07 at 1, ½
and ¼ of the step the successive differences fall 5.4 times or more. The law
is a closed form in the test; its distance from the exact half-space
relation, which reflects nothing, is `|R_law|`, reported. The mutation
`wall-memory-half-kick` puts 45°'s second-order wall 7.6e-2 off its law.
PH-06 and PH-07 read the same laws on a circle of radius one: an annulus
whose outer circle is the wall, the corners outside it cut off, and a hole
whose outer side is the wall in a square of side 5.2 whose own sides absorb.
One angular order a run, 0 to 8 where it propagates at kR 0.8, 2, 5 and 10,
driven from a source ring to steady state; each order's radial phasor
fitted to `A H + B H′`, the Hankel functions read from
`tests/physics/references/cylinder-hankel.json`; and `B/A` against the law
with the order's tangential wavenumber on the circle, extrapolated to a
step of nothing from the production step and its half, since the circle's
small boundary elements, meshed at a curve tolerance of 5e-5, set the step
whatever the edge. In the gate, `circular_walls_meet_their_law_on_the_coarse_mesh`,
32 s, reads order 2 at kR 5 on edge 0.1. Nightly, the jobs `outgoing-circle`
and `absorbing-hole` (30 and 46 minutes on the M1 Max) run every case under
both laws, kR 5 and 10 on edges 0.1, 0.07 and 0.05: within
`5e-5 + 2e-3 |R_law|` of the law on the finest mesh, measured 2e-6 to
4.6e-5 and 6e-5 to 2.4e-4 at the near-grazing orders whose shape error
weighs most; orders 0 to 2 at kR 10 falling 7.5 to 13.6 times over the
halving; every hole passive. The mutation `curve-wall-stiffness-doubled`
puts the hole's second-order order 2 5.4e-2 off its law.
PH-09 reads the same laws through a pulse, at every frequency it carries.
In PH-04's channel, the reflecting and pinned walls' spectra at the nine
points give the incident field and a unit reflection, and each wall's
spectrum is read against `U_inc + R_law U_unit`: an RMS over the band, the
bins above `1.02 nπ` and over 1e-3 of the peak, in units of the unit
reflection. On the circle, a disk whose circle is the wall, rung from a
ring at `r = 0.35`: the wall's records projected on `cos mθ` stand between
two walls in the ratio `(J′ − iY_r J) / (J′ − iY_t J)` at `kR = ω`, `J` from
`tests/physics/references/cylinder-hankel-dense.json`, the second-order
wall read against a `Y = 1` wall (its law) and a `Y = 2` wall against the
same (the method's own check). In the gate,
`separated_pulses_meet_the_planar_law_through_the_walls_memory`, 17 s: two
broadband pulses in mode 1, the second launched after the first has ended
and while its slow part still reaches the wall, held to 1e-3 on edge 0.1
(measured 5.7e-4 and 6.3e-4); and
`a_disk_rung_in_two_orders_at_once_meets_its_wall_law`, 75 s: orders 0 and 2
on 0.1, held to 3e-5 (measured 1.1e-5 and 1.2e-5). Nightly, the job
`transient-walls` (seventeen minutes on the M1 Max): the separated pulses on
edges 0.1, 0.07 and 0.05 falling at order 2.5 then 2.1 to 1.1e-4 and
1.3e-4, and 2.8 to 3.0 times at half the step on 0.07; a narrow pulse in
mode 2 at PH-04's 45° falling at order 2.7 and 3.1; the late tail, a pulse
just above mode 1's cutoff weighted 1, −2, 1 at 2 s spacing so its sum
vanishes doubly at the cutoff itself, which would ring past any record,
read over 60 s in a channel twice as long, falling at order 1.9 and 1.5 to
8.1e-5 and 7.0e-5, its records ending at 3.8e-4 of their peak. The circle in
order 0 broadband and low (`kR` from 1.5), order 2 narrow and the two orders
at once, on 0.1 and the two orders on 0.07 too: the law 3.7e-6 to 1.3e-5 and
the method 2.8e-5 to 6.4e-5, alike on both meshes, held to 3e-5 and 1.5e-4.
That floor is the drawn circle's: its wall nodes sit 9.5e-6 inside the unit
circle on average, so the circle cannot show convergence and the channel
does. The mutation `wall-memory-leaks`, the zero pole leaking a twentieth of
its mode's decay rate, puts the separated pulses' second-order wall 2.9e-3
off its law and the disk's order 2 2.7e-3.
PH-11 holds planar interfaces to Fresnel. A channel one unit high from
`x = −8` to 8, reflecting floor and ceiling, sponges of equal electric and
magnetic loss at both ends, which absorb without an impedance step, and
layers meeting at vertical interfaces: a channel mode meets them at an exact
angle, its tangential wavenumber `nπ` in every layer, and nothing converts it
to another. Held to steady state, the field at eight heights every 0.11, on
three units either side, is read by a lock-in and projected on the mode, and
each outer layer's projections are fitted to a forward and a backward wave of
the layer's exact wavenumber. The four amplitudes are held to the stack's
exact scattering matrix, `B = R A + T′ D` and `C = T A + R′ D`, `R′` and `T′`
the mirrored stack's, so that whatever the ends return is one more incident
wave; with them the power balance and, across one interface, the jumps of the
field and of its flux. Fresnel is written from the physical fields - TM's
`E_z` over `1/μ`, TE's `H_z` over `1/ε`, Mechanical's displacement over `k`
- through a transfer matrix in the test, with `ε ≠ μ` so that a slot mixed up
shows. In each skin: normal incidence, 30° and 60°, an impedance-matched
layer, contrast 25, total internal reflection at 45° and 60° from index 2,
and a slab at and off its resonance; in Mechanical a layer of axis ratio 2
along and across the channel (a frame turned otherwise couples the channel's
walls to the mode, which is then no longer one). A switch-on rings at every
layer's cutoff, where the mode stands still, so the drive's switch-on and the
lock-in are sized from its distance `Δω` to the nearest: with the lock-in
`30/Δω` long, total reflection at 60°, 7% above cutoff, read 1.7e-3 on every
mesh; at `90/Δω`, 2e-5. In the gate,
`planar_interfaces_scatter_as_fresnel_says` (106 s): normal incidence and 30°
in TM and TE, total reflection at 45°, the two anisotropic layers and the
resonant slab on edge 0.1, every reading under 8e-3 (measured 4.2e-3 at
worst). Nightly, the job `fresnel-interfaces` (42 minutes on the
M1 Max): every case on 0.1 under 1.5e-2 (7.5e-3, contrast 25 in TE on four
elements to its wavelength); on 0.07 all but the six at 60°, which read alike
on every mesh, under 3e-3 (1.9e-3); a case of each kind on 0.05 under 1e-3
(3.8e-4), falling about fourth order, twofold or more wherever above the
measurement's floor of 1e-4; and at half the step the readings do not move.
The mutation `frame-rotation-dropped`, a frame's angle never reaching its
tensor, turns the layer across the channel into one along it.
PH-12 holds cylinders to the partial-wave series. A cylinder of radius 0.5
in the box `[−2, 2]²` of a unit medium, second-order walls all round, is lit
by a band source; held to steady state, the field at 64 angles on five
circles from 0.6 to 1 about it is read by a lock-in, and each order's
coefficient fitted to `α_n J_n(kr) + c_n H_n(kr)`, so that whatever the walls
return is one more regular field. `c_n/α_n` is held to the ratio `S_n` of
`tests/physics/references/cylinder-scattering.json`, with the scattered
field's error on the inner circle, each measurable order's and its balance
`|1 + 2 S_n| = 1`, the optical theorem order by order; each skin's medium
maps through its physical fields to the nodal field's index and stiffness
ratio. The mesh draws the rim as a polygon inscribed in the circle, 128
vertices at the default curve tolerance and 256 at PH-06's (the topology's
own curve sampling caps it there), and the polygon scatters, to first order,
as the circle of its area, 2.0e-4 and 5.0e-5 short in radius as measured on
the mesh: that moved the readings by up to 8e-4, eight times the solver's
own error. So the solver is read against `S_n − k a δ S_n'`, the slope
tabulated beside the ratio, and the true circle's ratios read the geometry
with it. Cases: holes at `k a` 0.5, 2 and 5 and pinned cylinders at 2 and 5;
in TM and TE `ε = 4` at 0.5 and 2, `ε = 2` at 2 and 5, `ε = 10` at 0.5 and
`μ = 2` at 2; in Mechanical `ρ = 4` with `k` 1 and 2 and a hole; three off
centre, lit from below and at 30°. A switch-on rings the cylinder's
resonances: switched on over two periods, `ε = 4`'s third order, 0.8 rad/s
from the drive and inside the lock-in's main lobe, still read 5e-5 after the
settle, so the source switches on over 20 s; index 2 at `k a = 5` and `√10`
at 2 keep resonances that ring for 25 to 400 s and are left out. In the
gate, `cylinders_scatter_as_the_partial_waves_say` (89 s): a hole, a pinned
cylinder, `ε = 4` in TM and off centre in TE and Mechanical's stiffer
inclusion at `k a = 2` on edge 0.1, the solver's error under 2e-4 (measured
1.2e-4 at worst) and the circle's under 1.2e-3 (7.8e-4); and `ε = 4` in TE
at PH-06's tolerance, under 4e-4 off the circle (1.5e-4). Nightly, the job
`cylinder-scattering` (23 minutes on the M1 Max): every case on 0.1,
the solver's error under 2e-4 up to `k a = 2` and under 5e-3 (2.6e-3) at 5,
four to six elements to the wavelength; on 0.07 under 8e-5 (4.6e-5) and 2e-3
(1.1e-3); a subset on 0.05 under 2.5e-5 (1.2e-5) and 4e-4 (2.0e-4), falling
2.4 to 3 times from 0.07; at PH-06's tolerance the error against the circle
falls 3.3 to 4.9 times, to under 4e-4 (2.2e-4); half the step moves nothing.
The mutation `curve-sampling-coarsened`, the topology sampling curves to
four times its tolerance, leaves the rim 128 vertices at any tolerance and
puts `ε = 4` in TE 7.3e-4 off the circle.
PH-13 holds thin gaps to their spring law in PH-11's channel. A gap is a
divider whose two reflecting faces a spring couples: the flux across it is
the spring's tension, `K (u_right − u_left) = s_x ∂_x u`, `K` its ratio times
the geometric mean of the stiffness either side - `k`, `1/μ`, `1/ε` - so the
transfer matrix carries it as one more interface condition and PH-11's
scattering matrix holds `R` and `T`, the power and, across one gap, the
field's jump against the flux over the spring. The gap's own memory, the
jump of the field's time integral at each sample, is read by the lock-in
along the gap and held to the exact jump the measured waves drive, and the
springs' averaged energy to `K |J|²` over the mode. The transfer matrix
meets `|T|² = 1/(1 + (k_x/2r)²)`, the weld and the cut as limits and a
pair's resonance at `r = (k/2) tan(k d)`
(`the_gap_transfer_meets_its_closed_forms`). Cases: in TM a weak gap passing
1%, one passing half and a stiff one 99% at normal incidence, the half gap
at 30°, the cut (two independent reflecting faces, the vanishing spring's
limit), and a pair 0.15 apart at the ratio where it transmits everything
and 0.25 apart off it; between a unit medium and `ε = 4` in TM and TE and
`k = 4` in Mechanical, the last two drawn either way, which read alike. The
gap reaches the floor and the ceiling split, its springs totalling the ratio
times its length. Its memory is a drifted variable of the production step
and carries the step's bounded second-order offset
(`funfern-fourth-order-step.md`): the stored energy 7e-4 off at the step on
0.1 and 1.5e-4 at half of it, while `R` and `T` do not move. In the gate,
`thin_gaps_scatter_as_their_springs_say` (26 s): the half gap, the pair at
resonance and the two-sided TE gap drawn down on edge 0.1, every reading
under 5e-3 (2.3e-3 at worst, the TE gap's field jump); and the half gap
remeshed at 0.07, its memory right after the handoff 8.8e-4 of the jump
from the coarse run's harmonic at that instant, held to 3e-3. Nightly, the
job `thin-gap-interfaces` (12 minutes on the M1 Max): every case on 0.1
and 0.07 under 5e-3 and 2e-3 (2.4e-3 and 9.7e-4); a case of each kind on
0.05 under 5e-4 (2.4e-4), `R` and `T` falling 2 to 13 times from 0.07; at
half the step the stored energy's offset under 0.4 of the step's; and the
half gap at 30°, its jump varying along the gap, remeshed and read on both
meshes, its memory carried to 4e-4. The mutation
`gap-history-dropped-on-handoff`, a handoff carrying no gap history,
restarts the springs unstretched, the memory 0.69 of the jump off, its
value at that instant.
PH-14 holds temporal interfaces to the modal law. In a uniform medium every
mode evolves alone, `d/dt(m β̇) = −k² s β`, `m` and `s` the nodal mass and
stiffness through the physical fields - TM's `ε` over `1/μ`, TE's `μ` over
`1/ε`, Mechanical's `ρ` over `1/s₀`, the reciprocal stiffness its laws act
on - each times the law's factor, a Switch's smootherstep blend or a gate's
Hann edges. An instant change keeps `β` and `m β̇`, `B` and `D`; the test
integrates the ODE by RK4 split at every event, halving its own step moving
nothing, and it meets the temporal Fresnel ratio `(1 − r)/(1 + r)`,
`r = m₁ω₁/(m₂ω₂)`, and keeps `E/ω` where the impedance is kept and over a
forty-period ramp (`the_modal_ode_meets_the_temporal_closed_forms`). The unit
box, reflecting, starts in its mode `cos π(x + ½) cos π(y + ½)` and changes
after two periods, or a quarter period later when the field's energy has
moved to the other lane. The field projected on the mode is held to the ODE
over the run, and so are the stored energy, the modulation's energy - the
work lane, with the stored energy's jump at an instant throw - and the
frequency after; each pair a quarter period apart fixes the map
`c₂ = A c₁ + B c̄₁` of the mode's amplitude, and its time-reflection ratio
`|B/A|` is held to the ODE's. In each skin: the mass slot times 4 and a
quarter and the stiffness slot times 4 at once, both slots doubled (the
impedance kept, `|B/A| = 0`), the mass slot doubled with the stiffness slot
tripled, the mass slot times 4 over a tenth of a period, one and four
(`|B/A|` 0.33, 0.066 and 0.001, towards the adiabatic limit), slabs of it a
quarter and half the switched period long (0.6 and 0) and three
quarter-period slabs, and a gate's two-period slab. In the gate,
`temporal_interfaces_meet_the_modal_law` (31 s), every case at the
production step on edge 0.1: the trace under 1.2e-3 (5.9e-4), the energies
under 2.5e-3 (1.2e-3), the frequency under 3e-5 (1.1e-5), `|B/A|` within
2e-4 (9.6e-5). Every reading but the frequency, the lock-in's own, is the
step's and second order: half the step reads 1.6e-4, 3.0e-4 and 2.4e-5, a
quarter 6.2e-5, 7.5e-5 and 6.0e-6, the trace then on the mesh's 5.2e-5 - the
fourth-order step's bounded offset between its kicked and drifted variables
(`funfern-fourth-order-step.md`). Nightly, the job `temporal-interfaces`
(8 minutes on the M1 Max): half the step under a quarter of the bounds and
falling under 0.35 of the step's readings wherever those stand above the
step's floor, a quarter of the step held as at half, and edge 0.05 at its
own step. The mutation `switch-ramp-work-dropped`, a coefficient's rate
leaving out its Switch's ramp, leaves a ramp's work unbooked, 0.71 of the
energy, while the field follows the ramp.
PH-15 holds periodically modulated media to Floquet theory. PH-14's mode
equation with a periodic coefficient is a Hill equation, and its monodromy
over one modulation period gives the Floquet multipliers: whether the mode
grows, at what exponent, and the Floquet frequency of a stable one. The
oracle keeps area (`det = 1`), grows at `d ω/4` for a small pump at twice
the mode's frequency, keeps a pump of both slots - the impedance kept - on
the unit circle at any frequency, and near the mode's own frequency has the
second tongue closed for a slot that multiplies the mass or divides the
stiffness, the trace touching 2 and never passing it, and open for a divided
stiffness slot, Mathieu's form (`the_hill_equation_meets_floquet`). The box
mode runs with the pump on from the start. Read: the trace against the ODE,
relative to the field's largest so far; the growth exponent from the stored
energy at the start of every modulation period, against the ODE's by the
same fit, which meets Floquet's to 2%; the energy the work lane says the
modulation supplied; and for stable modes with their lines apart, the main
line against Floquet's frequency and the sidebands `ω_F + nΩ`, `n` from −2
to 2, against the ODE's. In each skin, depth 0.2 unless named: the mass
slot, the stiffness slot and a time crystal at 0.6 of the mode's frequency;
both slots at twice it, which never grows; the mass slot at twice it, inside
the tongue's edge by a tenth of its half-width and outside it; the stiffness
slot divided, depth 0.4, at the second tongue's centre; and a time crystal at
twice it. In the gate, `modulated_media_meet_floquet` (32 s), every case on
edge 0.1 at the production step: the trace under 1.5e-2 (7.0e-3), the energy
supplied under 3e-2 (1.4e-2), the growth exponent within `1.5e-4 ω` of the
ODE's (`6.7e-5 ω`; a mass-side pump at twice the frequency reads 0.22299
against Floquet's 0.22299), the main line within 5e-6 of
Floquet's (1.8e-6) and the sidebands within 4e-4 of the ODE's (1.9e-4). The
worst are stiffness-side pumps, TE's `ε`, whose second-order error the step
sets five times a mass-side one's; half the step reads 1.5e-3, 3.0e-3 and
1.3e-5. Nightly, the job `modulated-media` (8 minutes on the M1 Max):
half the step under a quarter of the bounds, the trace and the growth's
error falling under 0.35 where above the mesh's floor, a quarter of the step
and edge 0.05 held so. The mutation `pump-carrier-turned`, every drive's
carrier advanced a quarter turn, puts the mass pump at 0.6 0.47 off its
trace while the Floquet multipliers stand.
PH-16 holds the nonlinear laws to their own definitions. The constitutive
inverse meets an independent root - bisection on each law's definition to
adjacent floats, Cardano for focusing Kerr - for the linear map, focusing
Kerr, defocusing Kerr bounded at half its turning point and at 0.99 of it,
saturable laws of either sign and a junction, at targets from the smallest
subnormal to 100 and at a bound, its stored energy against a quadrature of
the definition; a target past a bound, and the laws the solver does not run,
are refused (`constitutive_inverses_meet_their_roots_and_refuse_past_their_bounds`).
A uniform field in a reflecting box makes every node one oscillator, held to
RK4 on its own law (`uniform_oscillators_follow_their_laws_at_second_order`,
4 s): sine-Gordon swung to 1e-3, 1, 2.5 and 2.9 rad and rotating, against
its elliptic periods; φ⁴ in a well, small and near the barrier, and over it,
against the period of its energy integral; a saturable mass, and Kerr with a
constant loss, each beside Klein-Gordon; van der Pol beside Klein-Gordon
from inside and outside its cycle over 40 s. Read at the production step:
the integrated field's and the field's largest difference from RK4 relative
to RK4's largest, the period, a conservative case's energy drift, and the
field's amplitude over its last three periods. Each case keeps within bounds
about twice its reading - the worst the swing at 2.9 rad, its period 2.6e-2
off where the period diverges at the separatrix, then φ⁴ near the barrier at
8.3e-3, everything else under 3.4e-3 - the energy under 1e-2 (4.5e-3), the
amplitude under 2e-3 (9.7e-4), the nodes together within 1e-6, and half the
step divides the trajectory's error by at least 3 (3.3 to 9.7). The
references are checked first (`the_oscillator_references_agree_with_each_other`):
RK4's periods meet the closed forms within 1e-7 (4e-8), and its van der Pol
cycle is one from inside and outside and within 1e-3 of the weak-gain
amplitude `2a/√3` (1.4e-4). Klein-Gordon moves a box mode's `ω²` by exactly
`ω₀²`, within 1e-6 (3e-8), and meets `√(k² + ω₀²)` within 2e-6 (9e-7) for
the modes (1, 0) and (1, 1) at edge 0.25
(`klein_gordon_adds_its_cutoff_to_a_modes_frequency`). A defocusing Kerr
mass bounded at 0.5 runs below its bound; past it the step is refused, the
one in which RK4 crosses the bound, and the state is untouched; φ⁴ past its
bound alike; van der Pol off the primary row, or driven, does not compile
(`a_uniform_field_driven_past_its_bound_is_refused_where_its_ode_crosses`).
The mutation `sine-gordon-force-linearized`, sine-Gordon's force its
small-angle limit, puts the 1 rad swing's period 6.2% off while the 1e-3 rad
swing still passes.
PH-18 holds what users read to analytic fields. Every probe number on screen
comes from a GPU recorder whose CPU twin is the canonical consumer:
`CanonicalPointStencil` for a point or each sample of a line, and
`sample_canonical_area` for an area. First the consumers, in every skin, on
a medium of mass slot 2 and stiffness slot 0.8, so that each skin's units
show: Mechanical's density and modulus, TM's `ε` and `1/μ`, and TE's `μ` and
`1/ε`, as `(ρ, A)` in `ρü = ∇·(A∇u)`. A standing mode of the reflecting box,
started at rest (`the_consumers_read_a_standing_mode_in_every_skin`), is held
at an edge of 0.125 and the production step to:
- the field within 3e-4 (1.5e-4);
- the rate, a backward difference, against `u̇` half a step before the
  sample, within 2e-3 (1.0e-3);
- the transverse field `c = orientation·A·R∇φ sin ωt/ω` within 3.2e-3
  (1.6e-3);
- the energy density within 1.7e-3 (8.3e-4) and the flow
  `orientation·u·Rc` within 1.1e-3 (5.5e-4).

Each falls at least 2.5 times from an edge of 0.25. A disk's mean and RMS
read within 1e-4 (4e-5, the drawn circle's floor) of polar quadratures of
the field, and its energy within 7e-3 (3.5e-3). That energy is the solver's
lumped energy with a clipped element counted by its covered fraction, as the
architecture says. The whole box reads `ρ∫φ²/2` within 1e-3 (4e-4).

A packet travelling down a channel, `u = −c₀P′(x − c₀t)` from an integrated
field `P`, a Gaussian times a sine
(`the_consumers_read_a_travelling_packet_in_every_skin`), is held at an edge
of 0.05 to:
- the field within 4e-4 (1.8e-4) and the transverse field within 4e-3
  (1.9e-3);
- the density within 2.2e-3 (1.1e-3) and the flow `c₀w` along its travel
  within 4e-3 (1.9e-3);
- the readout's trapezoid over 64 points across the channel within 1.1e-3
  (5.5e-4), negative under the segment's left normal against the travel;
- the energy that flux carries over the run within 1.2e-3 of the packet's
  own (5.1e-4).

Each falls at least 2.5 times from an edge of 0.1. The mutation
`consumer-flow-orientation-dropped`, the flow without its skin's
orientation, reverses Mechanical's flow (2.0 off).

Then the recorders themselves, through the app: the end-to-end fixture
`observables` (`src/ui/e2e/observables.rs`). It opens a TM channel whose left
side drives a 2 Hz burst into a first-order outgoing wall on its right, with:
- two points;
- a segment across the channel and one down the wall;
- a boundary probe on the inside trace of a transmitting circle of the same
  medium;
- an area probe of the circle's region, and a disk.

It runs 8 s at the app's own pacing. Every trace the readouts draw is then
held to the recorders' f64 twins, read off the reference after every step:
- **Coverage:** each kind's records a stride apart from the run's start to
  its end, none missing.
- **Values:** within 1e-4 of their scale. The M1 Max reads 2.2e-5 at worst,
  a point's; lines 8.6e-6, their readout integrals 7.9e-6, areas 8.7e-7.
- **Stamps:** within 2e-6 s of the f64 clock (3.9e-7).
- **Poynting's theorem round the loop:** what leaves it against what the
  region inside loses, within 1e-2 by a trapezoid over the records' stride
  (5.4e-3), and within 1e-4 of the twin's (6.4e-6).
- **The readouts' estimators on the device's records:** the upstream point's
  spectrum peaks within 1e-2 of the drive's amplitude (0.49787 of 0.5), and
  the Welch transfer to the downstream point, a pure delay, reads one within
  2e-3 (0.99951).
- **The rate:** read within 1e-2 of `u̇` half a step before its stamp
  (4.2e-3). At the stamp it reads 3.7e-2 off, the half step's `ωΔt/2`.

The same run with no probes then ends on the same bits. It takes 25 s on
Metal, 290 s on lavapipe and 27 s in Chrome on Metal, which reads what the
native app reads to every printed digit. In the nightly it runs in the
browser's recorders part, apart from the state part, which in the slower
hosting already takes up to 5 h 54 min of the runner's six hours. On
SwiftShader a page takes 55 to 57 minutes served as Pages, and past 100
isolated, beside the worker pool, where it ran out of time on 2026-10-19 and
2026-10-20; it has 150.

The twins are held to the wave on the CPU at the same mesh and step
(`the_twins_meet_the_wave_on_the_observables_channel`):
- the points' field within 2.5e-3 of an amplitude of 0.5 (1.2e-3), the
  segment across within 1.5e-3 (7.0e-4), Poynting within 4e-3 (1.8e-3);
- the wall's ledger booking `2∫g²` within 5e-4 (1.8e-4);
- the spectrum and the transfer as on the device.

The segment down the wall carries 1.5% more than the ledger books. The
transverse field on the wall's elements reads 1.5% high at the wall and 0.7%
low half an element in, where the field reads exact, falling four times as
the edge halves. It is held within 3% and listed as worth checking. The
mutation `curve-probe-flux-reversed`, the line recorder's normal flux with
its sign turned, reads the lines 2.1 off.

The far field last, with its twin `FarFieldTwin`
(`src/ui/e2e/far_field.rs`), the recorder in f64: contour frames centred on
their step, kept by the shader's bucket rule and projected by its retarded
interpolation.
- **The analytic pattern.** A line source's pattern is
  `F(θ) = e^{−ikρ cos ψ} + Σ (−i)ⁿ Sₙ H1ₙ(kρ) e^{inψ}`, with PH-12's ratios
  `Sₙ` and `H1ₙ(kρ)` from `cylinder-source.json`. The source sits 1.1 from a
  cylinder of size `k a = 2`, in a TM box of half-width 8 whose walls'
  first return reaches the contour only after the read: two periods of
  time-averaged intensity, relative to its peak.
  - **Gate:** `a_line_sources_far_field_reads_flat` holds the source alone
    flat within 5e-2 at an edge of 0.2 (2.3e-2).
  - **Nightly** (`far_fields_meet_a_line_sources_patterns`, in PH-12's
    job), at an edge of 0.15:

    | Case | Bound | Read |
    |---|---|---|
    | Alone | 2e-2 | 1.1e-2 |
    | Beside a hole | 1e-2 | 4.5e-3 |
    | Beside a cylinder of index √2 | 1e-2 | 4.6e-3 |

    The cylinder of index 2 rings in the resonance PH-12 switched on over
    20 s, and is left out.
- **The fixture `far-field`.** A TM box of outgoing walls with a hole and a
  source beside it, its far field on, run 6.5 s. Every record the readout
  draws stands a bucket apart from the first the delay window lets through,
  each direction's amplitude within 4e-5 of the twin's largest (8.1e-6), its
  far time within 2e-6 s (3.8e-7). The run with the far field off ends on
  the same bits.
- **Mutation.** `far-field-frame-at-step-end` puts the shader back as it
  was, and the fixture reads 6.1e-2 off its twin.

Last, the stamps.
- **Two-part stamps.** A point, line or area record carries its time as two
  f32 parts, as the vector overlay does: the epoch origin's coarse part, and
  its fine part with the epoch-local time. The host adds them in f64.
- **The test.** `probe_stamps_keep_their_precision_however_long_the_run`
  reproduces the device's arithmetic up to 36 hours in. Every stamp stands
  within 2e-5 s of the clock, where one f32 was 3.9e-3 s off.
  `probe_recorders_write_their_stamps_as_two_parts` holds each shader's slots
  to its decoder's.
- **Mutation.** `probe-stamps-summed-in-f32` sums the parts in f32 again; an
  hour in, it reads 3.2e-5 s off.
- **The far field's frames** still carry one f32.
- **The point rate's half step** is documented in the architecture: against
  `u̇` at its stamp it reads `ωΔt/2` off, as `observables` measures.

The far field's times, and the clock under them, late into a run.
- **Two-part far-field times.** The far field carries every time as a two-part
  stamp and differences two part by part. Its ring records a frame once a
  pass's stamp stands a period past the newest frame's, into the slot after
  it, keyed to its own count. Its projection finds each retarded time by
  bisection over the frames by age.
  - `far-field` reads within 5.3e-6 of the twin, which mirrors the rule.
  - The ring's tests hold its order across a handoff that changes the step.
  - The line source's patterns read 2.4e-2 (gate), 1.2e-2, 4.2e-3 and 3.7e-3
    (nightly).
- **Late-clock fixtures.** A start clock, which only the fixtures set, starts
  two of them 36 hours in, a few hundred steps before an epoch's rebase.
  - **`far-field-late`:** the clock within 2e-5 s of the f64 reference across
    the rebase (9.2e-6), the far field within 5e-4 of its twin (1.17e-4), its
    stamps within 2e-6 s.
  - **`handoff-memory-running-late`:** the handoff that starts a drive, the
    clock within 2e-5 s (4.4e-6), the state within 1e-4 (2.1e-5).
  - **State bounds.** The late runs' state bounds stand above the others'.
    Late in an epoch the local time is one f32 near 256 s, and drive phases
    carry 1e-4 rad from it.
- **An integrated drive across a rebase.** `canonical_gpu_integrated_rebase`
  drives a closed reflecting box in proportion to the nodal mass with an
  integrated harmonic whose integral has no mean, so the field stays uniform
  and the box's total flux integrates the rate. Its clock starts 64 steps
  before a rebase at 1,000 s and runs 8 s past it. The total reads 2.4e-5 of
  its swing from the reference, held to 1e-4; a rebase that formed the rate
  anchor apart from the carrier's reduced phase read 4.1e-4 (GPU-6 of the
  2026-10-20 review). `canonical_gpu_handoff --source --integrated` carries
  an integrated drive across a handoff, which no other run did.
- **A plan past the device's limits.** `canonical_gpu_binding_limit` makes
  the device bind half the largest buffer of the cavity's plan at edge 0.08,
  the 4.1 MiB table buffer, through Bevy's constrained limits. The request
  must admit against that limit, copied from the device as the app starts,
  refuse that plan naming the buffer, and install and step a coarser plan
  that fits. Installed, the larger plan's bind group failed validation and
  the app quit (GPU-7 of the 2026-10-20 review).
- **Two fixes they found.**
  - **The compensated sum.** Metal's fast math folded the rounding out of the
    clock's compensated sum at every rebase and handoff, 3.5e-3 s at 36 hours,
    and Chrome's transfer folded it even when guarded. The origin is now whole
    seconds and a fraction, moved on by exact sums
    (`the_clock_origin_moves_on_by_exact_sums`).
  - **The candidate's clock.** A handoff's candidate was packed against a
    clock at zero, so a drive it started moved on by the run's whole age in
    f32, 0.02 to 0.05 rad off at 36 hours; it is packed against the running
    time.
- **Mutations.** `clock-origin-summed-in-f32`,
  `handoff-candidate-packed-at-zero` and `far-field-stamps-summed-in-f32` each
  fail their late fixture.
The half kicks that step replaced read the second-order wall at 1.10 of its
law at the default step, an excess of their stepping, and are held to 15%
for comparison; at normal incidence the two meshes' difference hides the
wall's step, so there the trapezoid step's near run is held against its own
run at a quarter of the step, to a fifth of the half kicks' distance
(measured: a tenth). On the driven
path twelve wall tests - second-order balance under pumps and pins,
nonlinear walls' passivity, the strong Kerr pulse, a reversal, equality with
the fixed path on inert generations - run under both wall steps
(`under_each_wall_step`), and `measure_the_kick_forms_wall_against_its_law`
(ignored: seventeen minutes, a field law sending every node through its
inverse) reads the kick form's band at 1.0195 of its law against the half
kicks' 1.0532. The adaptation estimate's outgoing-history term reads
roundoff under the trapezoid step, which satisfies the trapezoid relation it
checks (`the_trapezoid_wall_step_leaves_no_outgoing_history_defect`;
`measure_what_the_wall_step_does_to_the_estimate`, ignored, finds the term at
most 0.04% of the estimate across the gallery), and a handoff every 25
steps beside a wall leaves 7% more behind under it than without one, against
the half kicks' 25%
(`a_handoff_beside_a_wall_leaves_less_behind_under_the_trapezoid_step`).
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
fourth-order run's semidiscrete frequency lands 7.6e-5 off leapfrog's.
PH-01 widens it to the three physics models (`box_modes_in_mechanical`,
`box_modes_in_tm`, `box_modes_in_te`, about 35 s each, in parallel): a
reflecting 2 × 1.4 box, so that no mode shares its frequency with another,
one medium whose slots give Mechanical half the electromagnetic speed, and
modes (1,0), (0,1), (1,2) and (2,3) started at a third of a period with Q
and b from the closed forms, b carrying each model's orientation as its law
writes it. The three read alike to the digit: the semidiscrete frequency
falls 139 to 674 times over two halvings to 1.9e-7 at worst, the phase
starts within 7.6e-4 of its closed form, the four modes together stand 2.0e-5
from it in L2 at edge 0.04 with the constant's total flux held to 3e-15,
and each integrator misses the semidiscrete frequency by its own law. The
mutation `tm-orientation-flipped` starts TM's modes at −θ. The
complementary flux, `B⊥` in TM and `D⊥` in TE, stands 9.0e-4, 1.1e-4 and
1.6e-5 from its closed form there, PH-02's field reconstruction in each
polarization. PH-02 sweeps the plane-wave dispersion on the same path
(`plane_waves_*`, five tests, about 35 s together): seven modes of a
2 × 1.06 box from 0° to 90° at five to twenty elements a wavelength, their
semidiscrete frequency fourth order, 121 to 282 times down over two
halvings, its coefficient `|ε|/(k h)⁴` within 25% in every direction;
the same modes in a medium of axis ratio 2 and 4, which is the isotropic
medium on the mesher's elements stretched twofold and fourfold, still
fourth order but ten and 175 times the dispersion along the stretch; each
integrator's law to 0.1% up to its stability limit; and the group velocity
from three wavenumbers along a direction, fourth order to 1.1e-6. The
mutation `lumping-midpoints-to-vertices` drops the dispersion to second
order. Edits the field
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

PH-17 holds the energy ledger to the physics rather than to itself. On
PH-01's box, started from three modes with `Q` and `b` both nonzero, each
channel's power is written in the test from the authored law - a flux-rate
loss `γ` takes `γ · flux · field`, a drive `m(t)` on a coefficient moves
the energy over it at `−(∂m/∂t) M Φ(u)`, `Φ = ∫ u g(u) du` under the row's
law `Q = m M g(u) u` - half of `Q u` on a linear row - a volume source puts
its nodal rate times the field in, a harmonic source's rate being the
integral of its signal - and integrated by the trapezoid over the run's own
states, never read from the solver's lanes or compiled rates. In each skin
fourteen configurations - electric, magnetic and spatially rising loss, a
driven loss, constant loss in a Kerr medium, a pump in a Kerr medium, a pump on either row, a time
crystal, a travelling modulation, a volume source, a thin gap, all of them
together, and van der Pol either side of its threshold - run three seconds
at the production step, its half and its quarter: the stored energy less
every channel's integral, and each lane less its channel, fall at second
order to under 2e-4 of the energy (measured to 6e-5), or stand at roundoff
where the solver books by the oracle's own quadrature. Van der Pol's gain
lane carries the law's sign, and what it leaves of the oracle settles to a
dissipation, the self-oscillating elements' short-wave viscosity, which the
oracle leaves out: 9e-3 to 1e-2 of the energy below the threshold, 1e-3
above. A gap's jumps are the time integral of the field's difference across
it, to second order, and its springs total the authored ratio of the wave
stiffness beside it - `k`, `1/μ`, `1/ε` - over its length, less the lumps at
its closed tips (`every_channel_books_what_its_law_puts_in_or_takes_out`,
27 s). With the electric and magnetic rates equal the loss is `−γ` times the
identity in energy-normalized coordinates, so every field decays as
`e^{−2γt}`: met to 6e-7 and 1e-6 at the quarter step
(`matched_loss_decays_every_field_as_its_closed_form`). A linear medium,
Kerr and Klein-Gordon keep their energy within an excursion that falls four
times over the halving under either integrator: the fourth-order step is
fourth order in phase, and its energy breathes at the leapfrog's second
order (`conservative_media_keep_their_energy_to_their_integrators_order`).
The composition with a gap, both losses and a source, remeshed halfway from
edge 0.25 to 0.18 through the maps the runtime prepares and handed over by
`CanonicalWaveState::handed_over`, books its own change of the stored energy
in the edit lane, 4e-4 to 7e-4 of it, and with that the whole run's ledger
closes at second order
(`a_remesh_books_its_own_energy_change_and_the_ledger_closes_across_it`). The
same on the driven path, by the time-driven handoff, and across handoffs that
change what a generation runs: a pump, Klein-Gordon's `r`, and a pump beside
a second-order wall and a gap, each remeshed (edits 3e-4 to 7e-4 of the
energy); a pump handed a Kerr law (an edit of 5e-2, the law's own energy); a
pump switched on, from the fixed path (−8e-2), and off, back to it (+9e-2).
Every run's ledger closes at second order, to 9e-6 at the quarter step, the
wall's loss read from the run's lane, which PH-05 holds to its law
(`driven_handoffs_book_their_edit_and_the_ledger_closes_across_them`).
The mutations `loss-lanes-crossed`, `modulation-work-omitted` and
`edit-exchange-dropped` each break one of them; `pump-rate-before-loss`
restores the defect PH-17 found, a pumped complementary row with its own
loss booking its pump work at first order.

PH-05 holds the walls to their law's own equations the same way. The right
side of PH-01's box is the wall, and the test assembles its trace from the
geometry alone - vertex, midpoint, vertex along each quadratic edge on
`x = 1` - with the trace damping `D = Z ℓ` lumped by Simpson's weights and
the tangential stiffness `k²/(2Z)` times each edge's line stiffness,
diagonalizes `D^{-½} A D^{-½}` by its own Jacobi rotations, and integrates
each mode's three pole states of `docs/spikes/funfern-boundary-auxiliary-spike.md`
exactly, the solver's trace field taken linear between states: the power
into the wall `Σ w j`, its store `Σ xᵀHx/2`, its dissipation
`Σ (w + √d ℓ·x)²`. In each skin, the first-order wall and the second-order
one beside a lossless medium, electric, magnetic and rising loss, Kerr and a
pumped density, three seconds at three steps: the wall's loss lane meets the
oracle's dissipation to 4.5e-6 of the energy at the quarter step and its
memory the oracle's poles to 7.9e-7, both at second order; the residual,
with the oracle's poles in place of the wall's store, falls at second order
to 4.3e-5 to 2.2e-4, held to 5e-4 for the trapezoid wall step's own
remainder. In TM and TE the second-order wall gives energy back at 12 to 16
of the production step's steps, its reactive part, which the law allows
(`walls_book_what_their_law_takes_beside_every_medium`). A slow source
beside the wall, varying along it as its third tangential mode at a tenth
of that mode's `d` in Mechanical and a twentieth in TM, for one 20 s period:
the wall gives back at a seventh of the steps in Mechanical and a third in
TM, and the ledger closes as anywhere else
(`a_slow_near_field_against_the_wall_keeps_its_ledger`; 7.5 s for both).
The mutation `wall-loss-without-memory`, the poles' share left out of the
wall's loss, puts the second-order wall's lane 8.3e-2 of the energy off. PH-05
found a source on a wall booking its work at first order; the mutation
`wall-source-work-before-its-half` restores it.

The handoff examples hold the device's edit lane, the energy a handoff
itself moves, to the reference: `canonical_gpu_handoff` in every suite
variant, `canonical_gpu_temporal_handoff` (pumped, gated and into a
nonlinear target) and `canonical_gpu_oscillator_handoff` (remesh, identity,
from and to a linear medium, an opened hole), each side valued at the handoff
by its own generation's law, restoring potential included. They agree to
1.9e-8 to 2.6e-7 of the energy, held to 1e-6; Klein-Gordon handed to a plain
medium books the 10.7, half its energy, that dropping `r` takes. The
mutation `handoff-store-authored` values the stores quadratically again, and
the pumped edit reads 0 for -3.0e-5. Nightly, `energy-long-runs` runs PH-17
and PH-05 long (`long_run_*`, half a minute): a linear medium in two skins
and Klein-Gordon a thousand seconds under either integrator, the energy's
excursion the same over the last quarter as the first (3.65e-4 and 4.3e-4);
weak loss, either wall and a wall beside a gap four hundred seconds in each
skin, the peak of each 20 s window never above the last; the slow near field
twenty periods, its residual where its switch-on left it; and Kerr's known
gain under the splitting held to 0.15 by t = 160 (measured 9.5e-2), which
by t = 1000 takes this strong a field past its declared bound.

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
pumped interface, down to 1.04 at its finest mesh, and the self-oscillating
medium at 0.86 to 0.90. An argument runs only the rows whose label contains
it.

`wave_boundary_reflection` sends finite Gaussian P2e packets at the outer box and
compares first- and second-order residual-energy reflection at two angles and two
wavelengths, alongside the ideal continuous plane-wave coefficients and a
long-time finite-state check.
Omit `--slices` for per-phase profiling.

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
stored bit, `r` included, unchanged. `FAILURE_FILTER=1` runs either law with the
grid filter on, as the app runs by default, the oracle filtering on the
device's cadence. The device runs up to 64 steps ahead of what the host has
seen, so the steps encoded after the failure cross a filter commit, and the
device must stay at the refused step. Until 2026-10-21 the filter took the
failed step's code for a filter not taken and cleared it, and the steps after
committed with nothing computed: on Kerr the device read its failure 24 steps
past the refusal (`failure-cleared-by-the-filter`). φ⁴'s status 6 is not a code
the filter takes that way, and it held either way.

`canonical_gpu_temporal_work` stamps a material Switch mid-run and requires
the runtime decoded from the state snapshot, and the bulk energy split and
pump power computed from it, to match an f64 oracle. The Switch origin is
stamped on the GPU, so this is the check that the accepted runtime reaches a
consumer rather than being guessed from the clock.

`canonical_gpu_complementary_mode` is PH-03: a state with Q zero and b in
the discrete kernel of the coupling, a curl-free flux that pushes no field,
the gradient of a potential vanishing on every wall, on the reflecting box
and, with `--baffle`, on a box a reflecting baffle divides from wall to wall,
drawn as the editor draws one, the flux on the baffle's left alone. Stepped
400 steps in f64 the mode is exact, Q at 1e-24 of its energy and the energy
kept to 1e-16; on the device b stays within `2.6e-8` of the reference, Q at
1e-24 and the energy within `2e-9`, where the bound is 1e-8, and the right
side of the baffle holds nothing, to the bit. The carry onto a mesh 1.4 times
finer, through the maps the runtime prepares for the edit, has its own
budget, the carried flux against the mode sampled on the new mesh, `4.0e-5`
on the box at edge 0.16 and `1.5e-4` on the baffled one, falling eightfold
at half the edge; within it the flux's first moment is kept to 1.6e-7 and
2.2e-6, the carried flux's compatible share in the new space is 3e-10 and
5e-9, what that share pushes into Q over 400 steps is 1.3e-10 and 2e-9, and
nothing crosses the baffle. With `--remesh` the device steps the carried
state and stays within `5.2e-7` of the reference. The end-to-end fixture the
plan named is not built: the driver acts through the app's handlers alone,
and the app has no tool that authors a complementary flux.

`canonical_gpu_cavity_modes` is PH-01's device half, `--physics mechanical`,
`tm` or `te`: the 2 × 1.4 box of `box_modes_in_*`, (1,2) and (2,3) under the
fourth-order step at edges 0.16, 0.08 and 0.04, and (2,3) at 0.08 under
leapfrog at 1, ½ and ¼ of its step, each from its closed form at a third of
a period, stepped on the device and in f64 and read every twelfth of a
period through a full readback at a known step. The device's series stands
within `5.7e-7` of the reference's, its frequency within `2.9e-8`, the f32
floor, held to 1e-7; that floor lies under the frequency's own error on
every mesh, so the device shows the element's fourth order in space to the
finest, (2,3) at 2.6e-5, 2.9e-6 and 1.8e-7, and leapfrog's second order in
time, successive differences falling 4.001 times. 45 to 90 s a model.

The walls step by the trapezoid rule
(`docs/spikes/funfern-outgoing-wall-step-spike.md`) on the reference and the
device alike; `--half-kicks` on `canonical_gpu_timing`,
`canonical_gpu_handoff` and `canonical_gpu_temporal_timing`, and
`LONG_RUN_WALL_STEP=half-kicks` on `canonical_gpu_long_run`, step them by the
half kicks it replaced instead, for comparison. The trapezoid rule's ledger
is not an exact identity: its split leaves a second-order remainder, `7.7e-4`
of the energy over the timing fixture's 144 steps, which the reference
leaves too. The timing example therefore holds the device's ledger to the
reference's own remainder, which under the half kicks is roundoff.
`--leapfrog` steps both at the leapfrog's own step instead of the production
integrator's. `--law-patch` is the check that a patch's commit keeps the
wall step; with `--loss` the plan is compiled from other rates than the
reference's, 0.3/s and 0.2/s, and the patch must bring every node and sample
to the reference's, which a patch that left the last samples as compiled
failed at b `3.1e-2` (GPU-5 of the 2026-10-20 review). On a driven generation `FORCED_WALL_STEP=half-kicks` on
`canonical_gpu_temporal_forced`, with `FORCED_WALLS=second` for a
second-order wall, `DRIVEN_WALL_STEP=half-kicks` on
`canonical_gpu_driven_document`, `TEMPORAL_AMR_WALL_STEP=half-kicks` on
`canonical_gpu_temporal_amr`, `NONLINEAR_WALL_STEP=half-kicks` on
`canonical_gpu_nonlinear` and `OSCILLATOR_WALL_STEP=half-kicks` on
`canonical_gpu_oscillator` do the same, the last two behind nonlinear walls
- the nonlinear example's medium in the kick form; the forced and nonlinear
examples also hold the device's wall-loss lane to the reference's.

`cargo test --release -p funfern-app --test wall_sweep -- --nocapture` is the
wall sweep of the testing campaign's stage A0.1
(`docs/spikes/funfern-testing-campaign-plan.md`): every wall law on every
side of every span and every outer side of every gallery scene, prepared
through the app's own path at the Coarse preset, each case classed before it
runs and the prepared ones' outgoing basis read back against its operator.
It runs in the gate without the five arrays of one obstacle, which the
nightly's `wall-sweep` job adds, at 0.16 and at 0.25, the resolution
slider's coarsest. `FUNFERN_WALL_SWEEP=<name>`
runs one scene with its report, `FUNFERN_WALL_SWEEP_ALL=1` every scene,
`FUNFERN_WALL_SWEEP_EDGE=<edge>` another edge, and
`FUNFERN_WALL_SWEEP_PLAN=1` prints the case counts without preparing.

`cargo test --release -p funfern-app --test regression_scenes -- --nocapture`
is stage A0.2: every scene under `tests/fixtures/scenes-regressions`, one per
reported bug that came with a scene, every frozen version 22 scene and every
gallery scene, opened, validated and prepared through the app's own path at
the document's own mesh edge, the prepared outgoing basis read back against
its operator and the regression scenes prepared twice, bit for bit; every
fixed generation's device plan is packed both ways, through the production
packer and the one the packer's own tests use, and the two must agree. It
runs in the gate, about a second on ten threads.

`scripts/mutations.sh` is stage A0.4: each folder under `tests/mutations/`
holds a deliberate defect as a patch and a manifest naming the test that must
catch it and the pattern its failure must match; the script applies each,
runs the test before and after, requires the failure the manifest names, and
reverts. A mutation that does not build, fails elsewhere, times out, or whose
test does not pass before the patch is "not detected". Not in the gate, since
each rebuilds the tree; the nightly runs all of them on Linux, the device
ones on lavapipe through `scripts/e2e-fixture.sh`, which runs one fixture on
this machine's GPU otherwise. `scripts/mutations.sh wall` runs the ones whose
names match. That every patch still applies to the tree, and every manifest
names its test, failure, needs and purpose, is held in the gate by
`crates/funfern-app/tests/mutations.rs`, so a patch the code has moved past
fails the day it moves rather than at the next nightly.

Which hardware each check has been run on, and when, is recorded in the
engineering log rather than here.
